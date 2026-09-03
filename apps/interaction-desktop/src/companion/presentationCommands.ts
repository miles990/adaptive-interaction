// Presentation command handling (pure, testable).
//
// The runtime dispatches `presentation.command` events (already governor-
// bounded and whitelist-validated on the Rust side); this module maps each
// command to what the companion window should do, and what it should honestly
// report back (`displayed` / `completed` / `unsupported` / `failed`).
// Unsupported capabilities are ACKED as unsupported — never silently dropped,
// never faked as displayed.
//
// CPP（v0.5）：`state-present`／`animation-play` 在 daemon 提供 characterProtocol
// 時**不再走這裡**——Runtime 把它們投影成 `character.intent`，由 CharacterGateway
// 交給角色 adapter 演出並以 character.receipt 回執。下面的 INTENT_TO_TRANSIENT／
// PLAYABLE_ART 只服務舊 daemon（無 characterProtocol）的相容路徑；其中的名稱是
// Runtime 公布的 legacy 詞彙（Rust presentation.rs BEHAVIOR_INTENTS／
// PLAYABLE_ANIMATIONS：idle/notice/curious/listening/thinking/working/waiting/
// quiet/stretch）與 machine.ts pose() 的 canonical 動畫名，不是任何 rig 的部位或
// 表情 id；renderer 各自以 alias／fallback 鏈解析。

import type { TransientKind } from "./machine";

export type PresentationOutcome = "displayed" | "completed" | "unsupported" | "failed";

export interface CommandPlan {
  outcome: PresentationOutcome;
  detail?: string;
  /** Machine transient to inject (null = clear the current transient). */
  transient?: TransientKind | null;
  /** For `performing`: the whitelisted animation to play. */
  animation?: string;
  /** Bubble to show (already runtime-validated ≤200 chars). */
  bubble?: { text: string; ms: number } | null;
  /** presence-set target (needs the desktop bridge to apply). */
  presence?: boolean;
  /** Fixed, runtime-whitelisted sound id. */
  sound?: "chime" | "soft-pop" | "tick";
  /** Runtime-cleaned speech text (never arbitrary script/SSML). */
  speech?: string;
  /** Companion-window-only adjustment, already bounded by the Rust Runtime. */
  window?: {
    x?: number;
    y?: number;
    width?: number;
    height?: number;
    opacity?: number;
    alwaysOnTop?: boolean;
  };
}

/** legacy behaviorIntent → machine transient（canonical 動畫名；renderer 以 fallback 解析）. */
const INTENT_TO_TRANSIENT: Record<string, { kind: TransientKind; animation?: string } | null> = {
  rest: null,
  notice: { kind: "performing", animation: "notice" },
  curious: { kind: "performing", animation: "curious" },
  listen: { kind: "listening" },
  think: { kind: "thinking" },
  work: { kind: "acting" },
  "wait-attention": { kind: "waiting-for-receipt" },
  // 「看向確認」只是姿態，不是「runtime 正在等你授權」。requesting-consent
  // 會演出真相狀態 `ask`，AI 不得點播——這裡映射到非真相的 `question`。
  "look-at-confirmation": { kind: "performing", animation: "question" },
  "acknowledge-briefly": { kind: "clicked" },
};

/** legacy 可點播動畫（Rust PLAYABLE_ANIMATIONS 鏡射）→ canonical 動畫名（undefined = 本視窗不支援）。 */
const PLAYABLE_ART: Record<string, string | null | undefined> = {
  idle: null, // 回到待機（清除 transient）
  quiet: null,
  notice: "notice",
  curious: "curious",
  listening: "listening",
  thinking: "thinking",
  working: "act",
  waiting: "waiting",
  stretch: "stretch",
};

const BUBBLE_MS = 8000;

/** `cancel` vs `clear-all`：只有 estop 的 clear-all 可以清掉安全訊息（transient 與固定安全氣泡）。 */
export function cancelEffects(command: "cancel" | "clear-all"): { forceClear: boolean; clearSafetyBubble: boolean } {
  return command === "clear-all"
    ? { forceClear: true, clearSafetyBubble: true }
    : { forceClear: false, clearSafetyBubble: false };
}

/**
 * `presence-set`：透過 host 真的 show／hide，host 確認（invoke resolve）後才維持
 * `completed`；拒絕（舊 host 沒有這個命令、視窗不存在…）→ `failed`，永不假裝完成。
 * 回傳的 plan 是同一個物件（就地更新 outcome／detail）。
 */
export async function applyPresence(
  plan: CommandPlan,
  setVisible: (visible: boolean) => Promise<unknown>
): Promise<CommandPlan> {
  if (plan.presence === undefined) return plan;
  try {
    await setVisible(plan.presence);
  } catch (error) {
    plan.outcome = "failed";
    plan.detail = `presence not applied by the host: ${String(error)}`;
  }
  return plan;
}

export function planPresentationCommand(
  command: string,
  params: Record<string, unknown>,
  hasDesktopBridge: boolean
): CommandPlan {
  switch (command) {
    // Runtime 取消/清場：把氣泡與「表演中」的 transient 一起清掉。
    // （安全姿勢由 base state 驅動，不受這裡影響。）
    case "cancel":
    case "clear-all":
      return { outcome: "completed", transient: null, bubble: null };
    case "bubble-show": {
      const text = typeof params.message === "string" ? params.message : "";
      if (!text) return { outcome: "failed", detail: "empty message" };
      return { outcome: "displayed", bubble: { text, ms: BUBBLE_MS } };
    }
    case "state-present": {
      const intent = typeof params.behaviorIntent === "string" ? params.behaviorIntent : "";
      if (!(intent in INTENT_TO_TRANSIENT)) {
        // Runtime already whitelists; an unknown here means version skew.
        return { outcome: "unsupported", detail: `behaviorIntent ${intent} not known to this window` };
      }
      const mapped = INTENT_TO_TRANSIENT[intent];
      const text = typeof params.message === "string" && params.message ? params.message : null;
      return {
        outcome: "displayed",
        transient: mapped === null ? null : mapped.kind,
        animation: mapped?.animation,
        bubble: text ? { text, ms: BUBBLE_MS } : null,
      };
    }
    case "animation-play": {
      const name = typeof params.animation === "string" ? params.animation : "";
      const art = PLAYABLE_ART[name];
      if (art === undefined) {
        return { outcome: "unsupported", detail: `animation ${name} has no art in this window` };
      }
      if (art === null) return { outcome: "displayed", transient: null };
      return { outcome: "displayed", transient: "performing", animation: art };
    }
    case "presence-set": {
      if (!hasDesktopBridge) {
        return { outcome: "unsupported", detail: "presence control needs the desktop shell" };
      }
      return { outcome: "completed", presence: Boolean(params.visible) };
    }
    case "sound-play": {
      const sound = params.sound;
      if (sound !== "chime" && sound !== "soft-pop" && sound !== "tick") {
        return { outcome: "unsupported", detail: `sound ${String(sound)} is not registered` };
      }
      return { outcome: "completed", sound };
    }
    case "speak": {
      const text = typeof params.text === "string" ? params.text : "";
      if (!text) return { outcome: "failed", detail: "empty speech text" };
      return { outcome: "completed", speech: text };
    }
    case "window-adjust": {
      if (!hasDesktopBridge) {
        return { outcome: "unsupported", detail: "window adjustment needs the desktop shell" };
      }
      const window: NonNullable<CommandPlan["window"]> = {};
      for (const key of ["x", "y", "width", "height", "opacity"] as const) {
        if (typeof params[key] === "number" && Number.isFinite(params[key])) window[key] = params[key];
      }
      if (typeof params.alwaysOnTop === "boolean") window.alwaysOnTop = params.alwaysOnTop;
      if (Object.keys(window).length === 0) return { outcome: "failed", detail: "empty window adjustment" };
      return { outcome: "completed", window };
    }
    default:
      return { outcome: "unsupported", detail: `unknown command ${command}` };
  }
}
