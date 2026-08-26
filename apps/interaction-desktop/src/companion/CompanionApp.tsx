// 小樞 companion window: honest runtime-state presentation + quick actions.
//
// This window is presentation + input entry ONLY. It holds no authority:
// every action goes through the same backend commands/policy as everything
// else, and safety states (emergency/blocked/unknown) use fixed standard
// wording that packs cannot override.

import React from "react";
import { api, RuntimeEvent } from "../api";
import { bootstrapSupervisor, desktop, isTauri } from "../desktop";
import { onRuntimeEvent } from "../api";
import {
  initial,
  MachineState,
  mapRuntimeEvent,
  pose,
  reduce,
} from "./machine";
import { PackManifest, SpriteRenderer, validateManifest } from "./renderer";

/** Deterministic default lines (persona packs may restyle NON-safety lines
 *  in a later phase; safety lines are fixed). */
const LINES: Record<string, string> = {
  succeeded: "做完了。",
  "succeeded-verified": "做完了，也確認過結果。",
  blocked: "這個動作超出目前允許範圍，所以我沒有執行。",
  unknown: "要求已送出，但目前無法確認是否真的完成。",
  emergency: "緊急停止中",
  paused: "主動互動暫停中。",
  offline: "目前連不上系統。",
};

const BUBBLE_COOLDOWN_MS = 8000;

export default function CompanionApp() {
  const canvasRef = React.useRef<HTMLCanvasElement>(null);
  const rendererRef = React.useRef<SpriteRenderer | null>(null);
  const machineRef = React.useRef<MachineState>(initial);
  const lastBubbleAt = React.useRef(0);
  const [bubble, setBubble] = React.useState<string | null>(null);
  const [menuOpen, setMenuOpen] = React.useState(false);
  const [packName, setPackName] = React.useState("shu-standard");
  const [ready, setReady] = React.useState(false);
  const [inputOpen, setInputOpen] = React.useState(false);

  // ---- boot: transport, pack, renderer ----
  React.useEffect(() => {
    let disposed = false;
    (async () => {
      await bootstrapSupervisor();
      let pack = "shu-standard";
      try {
        const prefs = await desktop.prefsGet();
        pack = (prefs as unknown as { companionPack?: string }).companionPack ?? pack;
      } catch {
        /* browser mode */
      }
      if (disposed) return;
      setPackName(pack);
      const manifest = (await fetch(`/packs/${pack}/manifest.json`).then((r) =>
        r.json()
      )) as PackManifest;
      const issues = validateManifest(manifest);
      if (issues.length > 0) {
        console.error("invalid character pack", issues);
        return;
      }
      if (disposed || !canvasRef.current) return;
      const renderer = new SpriteRenderer(
        canvasRef.current,
        manifest,
        `/packs/${pack}/sheet.png`,
        1.1
      );
      renderer.setReducedMotion(window.matchMedia("(prefers-reduced-motion: reduce)").matches);
      rendererRef.current = renderer;
      setReady(true);
    })();
    return () => {
      disposed = true;
      rendererRef.current?.destroy();
    };
  }, []);

  // ---- machine driving ----
  const apply = React.useCallback((ev: Parameters<typeof reduce>[1]) => {
    machineRef.current = reduce(machineRef.current, ev, Date.now());
    syncPose();
  }, []);

  const syncPose = React.useCallback(() => {
    const p = pose(machineRef.current, Date.now());
    rendererRef.current?.setAnimation(p.animation, p.frameSlice);
  }, []);

  // Runtime events → transients; status poll → base state.
  React.useEffect(() => {
    if (!ready) return;
    let stopped = false;
    const un = onRuntimeEvent((e: RuntimeEvent) => {
      const mapped = mapRuntimeEvent(e);
      if (mapped) {
        apply(mapped);
        maybeBubble(e);
      }
    });
    const poll = async () => {
      try {
        const s = await api.status();
        if (stopped) return;
        const estop = Boolean(s["emergencyStop"]);
        const paused = Boolean(
          (s["proactivePause"] as Record<string, unknown> | undefined)?.["paused"]
        );
        const quiet = Boolean(s["quietHours"]);
        apply({
          type: "base",
          base: estop ? "emergency" : paused ? "paused" : quiet ? "quiet" : "idle",
        });
      } catch {
        if (!stopped) apply({ type: "base", base: "offline" });
      }
    };
    void poll();
    const t = setInterval(poll, 5000);
    // Pose re-evaluation for transient expiry + ambient blink.
    const pump = setInterval(syncPose, 500);
    const blink = setInterval(() => {
      const p = pose(machineRef.current, Date.now());
      if (p.ambient && rendererRef.current) {
        rendererRef.current.setAnimation("blink");
        setTimeout(syncPose, 400);
      }
    }, 4500 + Math.floor(Math.random() * 2500));
    return () => {
      stopped = true;
      clearInterval(t);
      clearInterval(pump);
      clearInterval(blink);
      un.then((f) => f()).catch(() => {});
    };
  }, [ready, apply, syncPose]);

  function maybeBubble(e: RuntimeEvent) {
    const now = Date.now();
    const base = machineRef.current.base;
    // Quiet / paused: no ordinary bubbles (spec). Safety states show fixed text.
    if (e.eventType === "emergency.stop" && e.payload["cleared"] !== true) {
      setBubble(LINES.emergency);
      return;
    }
    if (base === "quiet" || base === "paused" || base === "emergency") return;
    if (now - lastBubbleAt.current < BUBBLE_COOLDOWN_MS) return;
    let text: string | null = null;
    if (e.eventType === "action.observed") text = LINES["succeeded-verified"];
    else if (e.eventType === "action.completed") text = LINES.succeeded;
    else if (e.eventType === "plan.blocked") text = LINES.blocked;
    else if (e.eventType === "action.uncertain") text = LINES.unknown;
    if (text) {
      lastBubbleAt.current = now;
      setBubble(text);
      setTimeout(() => setBubble(null), 4000);
    }
  }

  // ---- semantic interaction events (NEVER raw coordinates) ----
  const lastApproachAt = React.useRef(0);
  const pushInteraction = React.useCallback((kind: string, extra?: Record<string, unknown>) => {
    void api
      .pushObservation("desktop.companion.interaction", { kind, ...extra }, 1.0)
      .catch(() => {
        /* receptor disabled or runtime offline: interaction stays local */
      });
  }, []);

  function onPointerEnterCanvas() {
    const now = Date.now();
    if (now - lastApproachAt.current > 30_000) {
      lastApproachAt.current = now;
      pushInteraction("pointer-approached");
      // 游標靠近時看過來（不追蹤、不記錄座標）。
      apply({ type: "transient", kind: "listening", durationMs: 1200 });
    }
  }

  // ---- coarse activity summary (this app's windows only) ----
  const lastActiveRef = React.useRef(Date.now());
  React.useEffect(() => {
    if (!ready) return;
    const mark = () => {
      lastActiveRef.current = Date.now();
    };
    window.addEventListener("pointermove", mark, { passive: true });
    let wasActive = true;
    const t = setInterval(() => {
      const idle = Date.now() - lastActiveRef.current;
      const active = idle < 60_000;
      if (active !== wasActive) {
        wasActive = active;
        void api
          .pushObservation(
            "desktop.pointer.activity",
            { activeRecently: active, idleForMs: idle },
            1.0
          )
          .catch(() => {});
      }
    }, 15_000);
    return () => {
      window.removeEventListener("pointermove", mark);
      clearInterval(t);
    };
  }, [ready]);

  // ---- file drop: preview first, never auto-ingest ----
  const [dropPreview, setDropPreview] = React.useState<string[] | null>(null);
  React.useEffect(() => {
    if (!isTauri || !ready) return;
    let un: (() => void) | null = null;
    void (async () => {
      const { getCurrentWebview } = await import("@tauri-apps/api/webview");
      un = await getCurrentWebview().onDragDropEvent((event) => {
        if (event.payload.type === "drop") {
          const paths = (event.payload as { paths?: string[] }).paths ?? [];
          if (paths.length > 0) setDropPreview(paths);
        }
      });
    })();
    return () => {
      if (un) un();
    };
  }, [ready]);

  // ---- pointer: click vs drag ----
  const dragState = React.useRef<{ x: number; y: number; dragging: boolean } | null>(null);

  async function onPointerDown(e: React.PointerEvent) {
    dragState.current = { x: e.clientX, y: e.clientY, dragging: false };
  }

  async function onPointerMove(e: React.PointerEvent) {
    const d = dragState.current;
    if (!d || d.dragging) return;
    if (Math.abs(e.clientX - d.x) + Math.abs(e.clientY - d.y) > 5) {
      d.dragging = true;
      apply({ type: "transient", kind: "dragged" });
      pushInteraction("companion-dragged");
      if (isTauri) {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        const win = getCurrentWindow();
        await win.startDragging().catch(() => {});
        // startDragging blocks until drop: persist the new position.
        try {
          const pos = await win.outerPosition();
          await desktop.prefsPatch({
            companionPosition: [pos.x, pos.y],
          } as never);
        } catch {
          /* best effort */
        }
      }
      dragState.current = null;
    }
  }

  function onPointerUp() {
    const d = dragState.current;
    dragState.current = null;
    if (d && !d.dragging) {
      apply({ type: "transient", kind: "clicked" });
      pushInteraction("companion-clicked");
      setMenuOpen((v) => !v);
      setInputOpen(false);
    }
  }

  async function quick(action: string) {
    setMenuOpen(false);
    pushInteraction("action-selected", { action });
    try {
      switch (action) {
        case "talk":
          setInputOpen(true);
          break;
        case "open-cc":
          if (isTauri) {
            const { invoke } = await import("@tauri-apps/api/core");
            await invoke("companion_open_control_center", { tab: null });
          }
          break;
        case "tasks":
          if (isTauri) {
            const { invoke } = await import("@tauri-apps/api/core");
            await invoke("companion_open_control_center", { tab: "activity" });
          }
          break;
        case "pause-1h":
          await api.pauseSet(60, "companion quick action");
          setBubble("好的，接下來一小時我不會主動打擾。");
          setTimeout(() => setBubble(null), 3500);
          break;
        case "estop":
          await api.emergencyStop("companion quick action");
          break;
      }
    } catch (e) {
      setBubble(`失敗：${e}`);
      setTimeout(() => setBubble(null), 4000);
    }
  }

  // Click-through coordination: while a menu/input/bubble is open the whole
  // window must accept the cursor; otherwise only the character hit-rect does.
  React.useEffect(() => {
    if (!isTauri) return;
    void (async () => {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("companion_set_interactive", {
        interactive: menuOpen || inputOpen || bubble !== null || dropPreview !== null,
      }).catch(() => {});
    })();
  }, [menuOpen, inputOpen, bubble, dropPreview]);

  React.useEffect(() => {
    if (!isTauri || !ready || !canvasRef.current) return;
    const rect = canvasRef.current.getBoundingClientRect();
    void (async () => {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("companion_hit_rect", {
        x: rect.left,
        y: rect.top,
        w: rect.width,
        h: rect.height,
      }).catch(() => {});
    })();
  }, [ready]);

  const estop = machineRef.current.base === "emergency";

  return (
    <div className="companion-root">
      {bubble && (
        <div className={estop ? "companion-bubble danger" : "companion-bubble"} role="status">
          {bubble}
        </div>
      )}
      {estop && <div className="companion-estop-label">緊急停止中</div>}
      <canvas
        ref={canvasRef}
        className="companion-canvas"
        aria-label={`桌面角色小樞（${packName}）`}
        role="img"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerEnter={onPointerEnterCanvas}
      />
      {dropPreview && (
        <div className="companion-menu" role="dialog" aria-label="拖放預覽">
          <div style={{ fontSize: 12, padding: "4px 8px" }}>
            <strong>收到 {dropPreview.length} 個項目</strong>
            <ul style={{ margin: "4px 0", paddingLeft: 16, maxHeight: 80, overflow: "auto" }}>
              {dropPreview.slice(0, 5).map((p) => (
                <li key={p} style={{ wordBreak: "break-all" }}>
                  {p.split("/").pop()}
                </li>
              ))}
              {dropPreview.length > 5 && <li>…等 {dropPreview.length} 個</li>}
            </ul>
            <div className="muted" style={{ fontSize: 11, opacity: 0.8 }}>
              只記錄檔案位置，不讀取內容、不上傳、不離開本機。確認前不會做任何事。
            </div>
          </div>
          <button
            onClick={() => {
              pushInteraction("companion-dropped", {
                modality: "file-drop",
                attachments: dropPreview,
                mayLeaveDevice: false,
              });
              setDropPreview(null);
              setBubble("記下這些檔案了。");
              setTimeout(() => setBubble(null), 3000);
            }}
          >
            記錄這些項目
          </button>
          <button onClick={() => setDropPreview(null)}>取消（不做任何事）</button>
        </div>
      )}
      {menuOpen && (
        <div className="companion-menu" role="menu" aria-label="快捷操作">
          <button role="menuitem" onClick={() => quick("talk")}>
            對小樞說話…
          </button>
          <button role="menuitem" onClick={() => quick("tasks")}>
            查看目前工作
          </button>
          <button role="menuitem" onClick={() => quick("pause-1h")}>
            暫停一小時
          </button>
          <button role="menuitem" onClick={() => quick("open-cc")}>
            開啟控制中心
          </button>
          <button role="menuitem" className="danger" onClick={() => quick("estop")}>
            緊急停止
          </button>
        </div>
      )}
      {inputOpen && <CompanionInput onClose={() => setInputOpen(false)} onBubble={setBubble} />}
    </div>
  );
}

/** Text input with deterministic routing preview: default = local
 *  observation; open agent sessions can be selected as the destination
 *  (mailbox task). The preview always states where the data goes. */
function CompanionInput({
  onClose,
  onBubble,
}: {
  onClose: () => void;
  onBubble: (t: string | null) => void;
}) {
  const [text, setText] = React.useState("");
  const [sessions, setSessions] = React.useState<
    { sessionId: string; label?: string; agentId: string; dataScope: string[] }[]
  >([]);
  const [target, setTarget] = React.useState("local");
  const inputRef = React.useRef<HTMLInputElement>(null);
  React.useEffect(() => {
    inputRef.current?.focus();
    api
      .agentSessionsList()
      .then((list) => setSessions(list.filter((s) => !s.closedAt)))
      .catch(() => {});
  }, []);

  async function send() {
    const t = text.trim();
    if (!t) return;
    onClose();
    try {
      if (target === "local") {
        await api.pushObservation("session.input", { text: t, source: "desktop-companion" }, 1.0);
      } else {
        await api.agentSessionSend(target, "task", { task: t, source: "desktop-companion" });
      }
      await api
        .pushObservation("desktop.companion.interaction", { kind: "text-submitted", modality: "text" }, 1.0)
        .catch(() => {});
      onBubble(target === "local" ? "收到，我記下了。" : "已交給該工作階段（它收到後才算送達）。");
      setTimeout(() => onBubble(null), 3500);
    } catch (e) {
      onBubble(`送出失敗：${e}`);
      setTimeout(() => onBubble(null), 4000);
    }
  }

  const selected = sessions.find((s) => s.sessionId === target);
  return (
    <div className="companion-input" role="dialog" aria-label="對小樞說話">
      {sessions.length > 0 && (
        <select
          aria-label="交給誰"
          value={target}
          onChange={(e) => setTarget(e.target.value)}
          style={{ width: "100%", fontSize: 11 }}
        >
          <option value="local">本機 Runtime（觀察紀錄）</option>
          {sessions.map((s) => (
            <option key={s.sessionId} value={s.sessionId}>
              AI 工作階段：{s.label ?? s.agentId}
            </option>
          ))}
        </select>
      )}
      <div className="companion-route-note">
        {target === "local"
          ? "交給：本機 Runtime（觀察紀錄）・資料不離開本機"
          : `交給：${selected?.label ?? selected?.agentId}・可讀範圍：${
              selected?.dataScope.join("、") || "未設定"
            }`}
      </div>
      <input
        ref={inputRef}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") void send();
          if (e.key === "Escape") onClose();
        }}
        placeholder="輸入文字…（Enter 送出，Esc 取消）"
        aria-label="訊息內容"
      />
      <button onClick={() => void send()}>送出</button>
      <button onClick={onClose}>取消</button>
    </div>
  );
}
