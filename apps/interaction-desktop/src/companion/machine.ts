// Deterministic companion state machine (spec §11.3).
//
// Pure functions only: (state, event, nowMs) → state; (state, nowMs) → pose.
// Honesty invariants enforced here, not in the renderer:
//   - `queued`/`completed` without verification NEVER shows the green check.
//   - Unknown outcomes never play a success animation.
//   - EmergencyStopped freezes ordinary animation and outranks everything.
//   - Blocked shows the policy shield; the standard safety text stays in UIs.

export type BaseState = "idle" | "quiet" | "paused" | "emergency" | "offline";

export type TransientKind =
  | "listening"
  | "thinking"
  | "routing"
  | "requesting-consent"
  | "acting"
  | "waiting-for-receipt"
  | "succeeded" // verified=false → nod only; verified=true → green check
  | "blocked"
  | "unknown"
  | "failed"
  | "clicked"
  | "dragged"
  // A directly-requested, whitelisted performance animation (presentation
  // command / behavior runtime ambient). Never carries success/safety art.
  | "performing";

export interface Transient {
  kind: TransientKind;
  untilMs: number;
  verified?: boolean;
  /** For `performing`: which whitelisted animation to play. */
  animation?: string;
  /** For `performing`: optional honest sub-range of that animation. */
  frameSlice?: [number, number];
}

export interface MachineState {
  base: BaseState;
  transient: Transient | null;
}

export const initial: MachineState = { base: "offline", transient: null };

/** Priority for transient replacement (higher wins; spec §11.3 order). */
const PRIORITY: Record<TransientKind, number> = {
  blocked: 90, // safety warning
  failed: 85,
  "requesting-consent": 80,
  succeeded: 60,
  unknown: 60,
  clicked: 55, // direct user input
  dragged: 55,
  "waiting-for-receipt": 40,
  acting: 40,
  routing: 35,
  thinking: 30,
  performing: 25,
  listening: 20,
};

/** Default display durations (ms). */
const DURATION: Record<TransientKind, number> = {
  listening: 1500,
  thinking: 6000,
  routing: 4000,
  "requesting-consent": 12000,
  acting: 4000,
  "waiting-for-receipt": 10000,
  succeeded: 2500,
  blocked: 4500,
  unknown: 5000,
  failed: 5000,
  clicked: 700,
  dragged: 600,
  performing: 3000,
};

export type MachineEvent =
  | { type: "base"; base: BaseState }
  | {
      type: "transient";
      kind: TransientKind;
      verified?: boolean;
      durationMs?: number;
      animation?: string;
      frameSlice?: [number, number];
    }
  | { type: "clear-transient" };

export function reduce(state: MachineState, event: MachineEvent, nowMs: number): MachineState {
  switch (event.type) {
    case "base":
      return { ...state, base: event.base };
    case "clear-transient":
      return { ...state, transient: null };
    case "transient": {
      // Emergency/offline suppress every ordinary transient (no dead poses
      // over a stopped system — the safe pose stays fixed).
      if (state.base === "emergency" || state.base === "offline") {
        return state;
      }
      const current = state.transient;
      const active = current && current.untilMs > nowMs ? current : null;
      if (active && PRIORITY[active.kind] > PRIORITY[event.kind]) {
        return state; // higher-priority display keeps the stage
      }
      return {
        ...state,
        transient: {
          kind: event.kind,
          verified: event.verified,
          animation: event.animation,
          frameSlice: event.frameSlice,
          untilMs: nowMs + (event.durationMs ?? DURATION[event.kind]),
        },
      };
    }
  }
}

/** The animation the renderer should play right now + honest sub-range. */
export interface Pose {
  animation: string;
  /** For `succeeded` without verification: play only the nod frames. */
  frameSlice?: [number, number];
  /** Ambient personality (blink/idle variation) allowed? */
  ambient: boolean;
}

export function pose(state: MachineState, nowMs: number): Pose {
  if (state.base === "emergency") return { animation: "emergency", ambient: false };
  if (state.base === "offline") return { animation: "offline", ambient: false };

  const t = state.transient && state.transient.untilMs > nowMs ? state.transient : null;
  if (t) {
    switch (t.kind) {
      case "listening":
        // v2 packs 有專屬 listening 美術；v1 packs 由 renderer fallback 到 notice。
        return { animation: "listening", ambient: false };
      case "thinking":
        return { animation: "thinking", ambient: false };
      case "routing":
        return { animation: "routing", ambient: false };
      case "requesting-consent":
        return { animation: "ask", ambient: false };
      case "acting":
        return { animation: "act", ambient: false };
      case "waiting-for-receipt":
        return { animation: "waiting", ambient: false };
      case "succeeded":
        // Honest success: nod for completed, green check ONLY when verified.
        return t.verified
          ? { animation: "success", ambient: false }
          : { animation: "success", frameSlice: [0, 1], ambient: false };
      case "blocked":
        return { animation: "blocked", ambient: false };
      case "unknown":
        return { animation: "unknown", ambient: false };
      case "failed":
        // v2 packs 有失敗專屬美術（愣住→認真檢查＋✕）；v1 packs 由 renderer
        // fallback 到 blocked。固定安全語句讓兩者永遠可分辨。
        return { animation: "failed", ambient: false };
      case "clicked":
        return { animation: "clicked", ambient: false };
      case "dragged":
        return { animation: "dragged", ambient: false };
      case "performing":
        return { animation: t.animation ?? "idle", frameSlice: t.frameSlice, ambient: false };
    }
  }
  if (state.base === "quiet") return { animation: "quiet", ambient: false };
  if (state.base === "paused") return { animation: "paused", ambient: false };
  return { animation: "idle", ambient: true };
}

// ---------------------------------------------------------------------------
// Runtime-event mapping (kept pure for testability).
// ---------------------------------------------------------------------------

export interface RuntimeEventLike {
  eventType: string;
  payload: Record<string, unknown>;
}

/** Map one runtime event to a machine event (or null = no visual change). */
export function mapRuntimeEvent(e: RuntimeEventLike): MachineEvent | null {
  switch (e.eventType) {
    case "emergency.stop":
      return e.payload["cleared"] === true
        ? { type: "base", base: "idle" }
        : { type: "base", base: "emergency" };
    case "proactive.paused":
      return { type: "base", base: "paused" };
    case "proactive.resumed":
      return { type: "base", base: "idle" };
    case "receptor.observation":
      return { type: "transient", kind: "listening" };
    case "plan.created":
      return { type: "transient", kind: "thinking" };
    case "plan.blocked":
      return { type: "transient", kind: "blocked" };
    case "action.accepted":
    case "action.dispatched":
      return { type: "transient", kind: "acting" };
    case "action.acknowledged":
      return { type: "transient", kind: "waiting-for-receipt" };
    case "action.completed":
      // Completed ≠ verified: nod only (frameSlice), never the green check.
      return { type: "transient", kind: "succeeded", verified: false };
    case "action.observed":
      // Independent verification saw the effect: the check is honest now.
      return { type: "transient", kind: "succeeded", verified: true };
    case "action.uncertain":
      return { type: "transient", kind: "unknown" };
    case "action.failed":
      return { type: "transient", kind: "failed" };
    case "ai.assist.requested":
      return { type: "transient", kind: "waiting-for-receipt", durationMs: 15000 };
    // v0.5：Agent Session taxonomy → 角色演出。全部由 runtime 真實事件驅動；
    // claimed-completed 只點頭（verified:false），綠勾只認 `verified`
    // （人工驗證，human-only 路由）。
    case "agent.session.state": {
      const state = String(e.payload["state"] ?? "");
      const agent = String(e.payload["agentId"] ?? "");
      const waitExpr =
        agent === "codex" ? "wait-codex" : agent === "claude-code" ? "wait-claude" : "waiting";
      switch (state) {
        case "created": // queued：等待任務被取走
          return { type: "transient", kind: "performing", animation: waitExpr, durationMs: 6000 };
        case "fetched": // 任務真的送進 agent 子程序
          return { type: "transient", kind: "routing", durationMs: 3000 };
        case "working":
          return { type: "transient", kind: "acting", durationMs: 8000 };
        case "waiting-input":
        case "waiting-consent":
          return { type: "transient", kind: "requesting-consent" };
        case "claimed-completed":
          return { type: "transient", kind: "succeeded", verified: false };
        case "verified":
          return { type: "transient", kind: "succeeded", verified: true };
        case "failed":
        case "timed-out":
          return { type: "transient", kind: "failed" };
        case "cancelled":
        case "closed":
          // 取消/收尾：誠實回到待機，不演成功也不演失敗。
          return { type: "clear-transient" };
        default:
          return null;
      }
    }
    case "consent.changed":
      return null;
    default:
      return null;
  }
}
