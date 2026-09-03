#!/usr/bin/env node
// 外部 Character Adapter 參考實作（純文字、無 expression、無 rig）。
//
// 用途：證明 Character Presentation Protocol（docs/character-protocol/README.md）的 wire protocol
// 不依賴小樞 rig，也不依賴 Tauri／React——任何能開 WebSocket 的程式（遊戲引擎、外部桌面程式、
// 遠端顯示器）都能用同一批訊息接上 Runtime。
//
// 需求：Node ≥ 22（內建 global WebSocket）。不需要任何 npm 依賴。
//
// 用法：
//   1. 用人類 token 註冊 adapter，拿到 adapter token：
//        interact-ai character adapters add --name "文字 adapter" --manifest examples/character-adapters/text-adapter.manifest.json
//   2. 執行：
//        INTERACT_AI_CHARACTER_TOKEN=<adapter token> node examples/character-adapters/text-adapter.mjs
//      環境變數：
//        INTERACT_AI_API            Runtime API base（預設 http://127.0.0.1:8787）
//        INTERACT_AI_CHARACTER_TOKEN adapter token（必填；**不是** human token，也拿不到 human token）
//        CHARACTER_FIXTURE_ONCE=1   收到第一個 intent 並回 completed 後結束（CLI E2E 用）
//        CHARACTER_FIXTURE_QUIET=1  不印 heartbeat
//
// 誠實原則：這個 adapter 只會回報它真的做了的事——把 intent 印成一行文字。它永遠不會自己送
// truthState、永遠不會宣稱 verified；completed 只代表「文字已印出」，不代表任何工作已驗證。

const PROTOCOL_VERSION = "1.0";
const CHARACTER_ID = "text-adapter-fixture";
const MANIFEST_VERSION = "1.0.0";

const SAFETY_INTENTS = [
  "emergency",
  "offline",
  "blocked",
  "failed",
  "request-consent",
  "unknown",
  "verified-success",
  "claim-completed",
  "wait",
  "ask",
  "cancelled",
];
const CASUAL_INTENTS = ["idle", "notice", "acknowledge", "think", "work", "greet", "play", "rest", "sleep"];

// 固定安全文案（與 Runtime 的 system.text／桌面 FIXED_SAFETY_LINES 語意一致；adapter 不得改寫成「成功」）。
const LINE = {
  emergency: "緊急停止中。",
  offline: "Runtime 離線。",
  blocked: "這個動作超出目前允許範圍，沒有執行。",
  failed: "執行失敗。",
  "request-consent": "需要你同意才能繼續。",
  unknown: "結果不確定，不會自動重試。",
  "verified-success": "做完了，也確認過結果。✔",
  "claim-completed": "Agent 說做完了，還沒檢查。",
  wait: "等待中。",
  ask: "等你補充。",
  cancelled: "已停止。",
  idle: "（閒置）",
  notice: "（注意到了）",
  acknowledge: "（收到）",
  think: "（思考中）",
  work: "（處理中）",
  greet: "（打招呼）",
  play: "（想玩）",
  rest: "（休息）",
  sleep: "（睡覺）",
};

const api = process.env.INTERACT_AI_API || "http://127.0.0.1:8787";
const token = process.env.INTERACT_AI_CHARACTER_TOKEN;
const once = process.env.CHARACTER_FIXTURE_ONCE === "1";
const quiet = process.env.CHARACTER_FIXTURE_QUIET === "1";
if (!token) {
  console.error("INTERACT_AI_CHARACTER_TOKEN is required (adapter token from `interact-ai character adapters add`)");
  process.exit(2);
}
if (typeof WebSocket !== "function") {
  console.error("Node >= 22 is required (global WebSocket)");
  process.exit(2);
}

const wsUrl = api.replace(/^http/, "ws").replace(/\/$/, "") + "/v1/character/ws?token=" + encodeURIComponent(token);

let generation = 0;
let backoffMs = 1000;
let instanceId = null;
let stopping = false;

function nowIso() {
  return new Date().toISOString();
}

function log(line) {
  process.stdout.write(line + "\n");
}

function send(ws, msg) {
  if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(msg));
}

function receipt(ws, envelope, status, resolution, detail) {
  send(ws, {
    type: "receipt",
    receipt: {
      messageId: envelope.messageId,
      characterInstanceId: instanceId ?? envelope.characterInstanceId,
      generation,
      status,
      resolution,
      detail,
      at: nowIso(),
    },
  });
}

function negotiateMessage() {
  generation += 1;
  return {
    type: "negotiate",
    protocolVersion: PROTOCOL_VERSION,
    characterId: CHARACTER_ID,
    manifestVersion: MANIFEST_VERSION,
    capabilities: {
      "visual.presence": { supported: true, version: "1", reducedMotionBehavior: "unchanged" },
      "visual.textBubble": { supported: true, version: "1", maxConcurrent: 1, interruptible: true },
    },
    inputCapabilities: {
      "input.text": { supported: true, version: "1" },
    },
    channels: ["bubble"],
    intents: [...SAFETY_INTENTS, ...CASUAL_INTENTS],
    variants: [],
    generation,
  };
}

function connect() {
  const ws = new WebSocket(wsUrl);
  ws.addEventListener("open", () => {
    backoffMs = 1000;
    log(`[connect] ${wsUrl.replace(/token=[^&]+/, "token=***")}`);
  });
  ws.addEventListener("message", (ev) => {
    let msg;
    try {
      msg = JSON.parse(String(ev.data));
    } catch {
      send(ws, { type: "error", code: "bad-json", message: "adapter could not parse message" });
      return;
    }
    switch (msg.type) {
      case "hello": {
        instanceId = msg.characterInstanceId ?? instanceId;
        log(`[hello] protocol=${msg.protocolVersion} instance=${instanceId} reducedMotion=${msg.reducedMotion}`);
        send(ws, negotiateMessage());
        break;
      }
      case "negotiated": {
        instanceId = msg.characterInstanceId ?? instanceId;
        if (typeof msg.generation === "number") generation = msg.generation;
        const res = msg.resolutions ?? {};
        const summary = Object.entries(res)
          .map(([intent, r]) => `${intent}=${r.resolution}${r.via ? "@" + r.via : ""}`)
          .join(" ");
        log(`[negotiated] generation=${generation} ${summary}`);
        send(ws, { type: "lifecycle", state: "ready", characterInstanceId: instanceId, generation });
        break;
      }
      case "intent": {
        const env = msg.envelope ?? msg;
        const intent = String(env.intent);
        const truth = String(env.truthState ?? "none");
        if (env.expiresAt && Date.parse(env.expiresAt) < Date.now()) {
          receipt(ws, env, "expired", "unsupported", "arrived after expiresAt");
          break;
        }
        receipt(ws, env, "accepted", undefined, undefined);
        if (!(intent in LINE)) {
          receipt(ws, env, "unsupported", "unsupported", `unknown intent ${intent.slice(0, 40)}`);
          break;
        }
        // 綠勾只在 truthState=verified（Runtime 決定）；即使 intent 是 verified-success，truth 不符也只印中性文字。
        const line = intent === "verified-success" && truth !== "verified" ? LINE["claim-completed"] : LINE[intent];
        receipt(ws, env, "started", "exact", undefined);
        const hint = env.presentationHints?.message ? `「${String(env.presentationHints.message).slice(0, 200)}」` : "";
        log(`[intent] ${intent} truth=${truth} priority=${env.priority} corr=${env.correlationId ?? "-"} → ${line}${hint}`);
        receipt(ws, env, "completed", "exact", "line printed");
        if (once) {
          stopping = true;
          send(ws, { type: "goodbye", reason: "fixture-once" });
          setTimeout(() => process.exit(0), 100);
        }
        break;
      }
      case "cancel": {
        // 冪等：文字已印出的 command 不會「收回」，誠實回 cancelled{alreadyTerminal}。
        send(ws, {
          type: "receipt",
          receipt: {
            messageId: msg.messageId,
            characterInstanceId: instanceId,
            generation,
            status: "cancelled",
            resolution: "exact",
            detail: "alreadyTerminal",
            at: nowIso(),
          },
        });
        break;
      }
      case "heartbeat": {
        if (!quiet) log(`[heartbeat] ${msg.at ?? ""}`);
        send(ws, { type: "heartbeat", at: nowIso(), generation });
        break;
      }
      case "error": {
        log(`[error] ${msg.code}: ${msg.message ?? msg.detail ?? ""}`);
        break;
      }
      case "goodbye": {
        log(`[goodbye] ${msg.reason ?? ""}`);
        stopping = true;
        ws.close();
        break;
      }
      default:
        // 未知訊息：忽略（同 major 相容），不當機。
        if (!quiet) log(`[ignored] ${String(msg.type)}`);
    }
  });
  ws.addEventListener("close", (ev) => {
    log(`[closed] code=${ev.code}`);
    if (stopping) return;
    setTimeout(connect, backoffMs);
    backoffMs = Math.min(backoffMs * 2, 15000);
  });
  ws.addEventListener("error", () => {
    // close 事件會接手重連。
  });

  // 從 stdin 讀一行 → character.text-submitted（示範受限 input event）。
  if (process.stdin.isTTY === false || process.env.CHARACTER_FIXTURE_STDIN === "1") {
    process.stdin.setEncoding("utf8");
    process.stdin.on("data", (chunk) => {
      const text = String(chunk).trim().slice(0, 2000);
      if (!text || !instanceId) return;
      send(ws, {
        type: "event",
        event: {
          protocolVersion: PROTOCOL_VERSION,
          eventId: `evt-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`,
          characterInstanceId: instanceId,
          generation,
          timestamp: nowIso(),
          kind: "character.text-submitted",
          payload: { text },
          privacyClass: "personal",
        },
      });
    });
  }
}

process.on("SIGINT", () => {
  stopping = true;
  process.exit(0);
});
process.on("SIGTERM", () => {
  stopping = true;
  process.exit(0);
});

connect();
