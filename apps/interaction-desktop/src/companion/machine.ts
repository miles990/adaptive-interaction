// Deterministic companion state machine (spec §11.3).
//
// Pure functions only: (state, event, nowMs) → state; (state, nowMs) → pose.
// Honesty invariants enforced here, not in the renderer:
//   - `queued`/`completed` without verification NEVER shows the green check.
//   - Unknown outcomes never play a success animation.
//   - EmergencyStopped freezes ordinary animation and outranks everything.
//   - Blocked shows the policy shield; the standard safety text stays in UIs.

import { EventClass, scoreEvent } from "./behavior";

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
  /** For `performing`: which whitelisted animation to play.
   *  For `clicked`／`dragged`: the reaction variant the Director picked（省略＝canonical 名，
   *  由 renderer 的 alias 解析成 poked／lifted）。 */
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

/** Which utility class each transient belongs to (spec §5.4 ladder).
 *  Only consulted when two transients have the SAME priority — the ladder
 *  above still decides everything else. */
const CLASS_OF: Record<TransientKind, EventClass> = {
  blocked: "sensor-safety",
  failed: "sensor-safety",
  "requesting-consent": "waiting-confirmation",
  succeeded: "task-state",
  unknown: "task-state",
  clicked: "direct-interaction",
  dragged: "direct-interaction",
  "waiting-for-receipt": "task-state",
  acting: "task-state",
  routing: "task-state",
  thinking: "task-state",
  performing: "ambient",
  listening: "world-event",
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
  /**
   * 清掉 transient。預設**不動**安全訊息（blocked／failed／unknown）：presentation
   * `cancel`（AI 用同一把 token 就能對自己的 companion.speak 送）不得把被擋下／失敗／
   * 未知提早抹掉。只有 estop 的 `clear-all` 帶 `force:true`——即使如此基態 emergency
   * 也不在這裡解除（基態由 runtime 狀態擁有）。
   */
  | { type: "clear-transient"; force?: boolean };

/**
 * Equal-priority competition (spec §6.1 「多事件注意力競爭」).
 *
 * `keep`    — the running display outranks the new event.
 * `refresh` — the SAME event reported again: extend the display, but do not
 *             restart the performance (utility scoring's repeat penalty).
 * `replace` — a genuinely new event wins the stage.
 */
export function transientCompetition(
  active: Transient | null,
  next: { kind: TransientKind; verified?: boolean; animation?: string }
): "keep" | "refresh" | "replace" {
  if (!active) return "replace";
  const pa = PRIORITY[active.kind];
  const pe = PRIORITY[next.kind];
  if (pa > pe) return "keep";
  if (pa < pe) return "replace";
  const repeat =
    active.kind === next.kind &&
    active.verified === next.verified &&
    active.animation === next.animation;
  if (repeat) return "refresh";
  const base = {
    recentSameClass: 0,
    alreadyResponded: false,
    interruptible: true,
    doNotDisturb: false,
    relevance: 1,
    novelty: 0,
  };
  const activeScore = scoreEvent(CLASS_OF[active.kind], base);
  const nextScore = scoreEvent(CLASS_OF[next.kind], { ...base, novelty: 1 });
  return nextScore > activeScore ? "replace" : "keep";
}

/** 緊急停止時仍可以留在台上的 transient（安全訊息本身）。 */
const SAFETY_TRANSIENTS = new Set<TransientKind>(["blocked", "failed", "unknown"]);

/**
 * 這次 transient 更替算不算「被搶佔」？
 *
 * 只有還在播（`untilMs > nowMs`）的表演被別的東西換掉才算。已經自然到期的
 * 表演被下一個事件覆蓋是正常收場——把它記成 interruption 會讓角色誤以為
 * 一直被打斷，主動表現越收越小，也會排一個假的「恢復計畫」。
 */
export function wasPreempted(
  before: Transient | null,
  after: Transient | null,
  nowMs: number
): boolean {
  if (!before || before.kind !== "performing") return false;
  if (before.untilMs <= nowMs) return false; // 自然到期，不是被搶
  return after !== null && after.kind !== "performing";
}

/**
 * 還在播的表演被**另一個表演**換掉（同優先 performing 25 vs 25，不同動畫）。
 *
 * 這不是 wasPreempted（那是被真實事件搶）——但 Director 排的 ambient 已經下台了：
 * 不通知的話 Director.currentAction 仍指著它，之後任何真實搶佔都會拿早已下台的
 * 動作的 startedAt 算剩餘時間、排一個假的恢復計畫（對抗審查 director-pipeline-025）。
 */
export function wasReplacedByPerforming(
  before: Transient | null,
  after: Transient | null,
  nowMs: number
): boolean {
  if (!before || before.kind !== "performing") return false;
  if (before.untilMs <= nowMs) return false;
  if (!after || after.kind !== "performing") return false;
  return after.animation !== before.animation || after.frameSlice !== before.frameSlice;
}

/** 拖曳期間的「持續」transient：TTL 只是安全網，放下前每 500ms 續期。 */
export const DRAG_HOLD_MS = 1500;
export const DRAG_RENEW_MS = 500;

/**
 * 混音器入口（CPP in-process adapter 與 host 共用）：adapter 把 intent 轉成
 * machine event 餵進來，host 決定狀態放在哪裡（CompanionApp 的 machineRef，
 * 或 adapter 自帶的 LocalMixer）。回傳套用後的狀態。
 */
export interface MixerPort {
  apply(event: MachineEvent): MachineState;
  state(): MachineState;
}

/**
 * canonical 動畫名 → machine event。這是 pose() 詞彙的反向映射（engine-neutral）：
 * 真相名稱進對應的 transient kind／基態，其餘名稱當成 performing 表演。
 * `success` 帶 frameSlice ＝ claimed（只點頭）、不帶 ＝ verified；呼叫端
 * （adapter）已依 truthState 決定是否給 slice，這裡不做升級。
 */
export function machineEventForAnimation(
  name: string,
  frameSlice?: [number, number],
  durationMs?: number
): MachineEvent {
  const t = (kind: TransientKind, extra: Partial<Extract<MachineEvent, { type: "transient" }>> = {}): MachineEvent => ({
    type: "transient",
    kind,
    ...(durationMs !== undefined ? { durationMs } : {}),
    ...extra,
  });
  switch (name) {
    case "emergency":
      return { type: "base", base: "emergency" };
    case "offline":
      return { type: "base", base: "offline" };
    case "idle":
    case "paused":
      // 回到基態；paused／quiet 基態由 runtime 狀態輪詢擁有，不在這裡改。
      // 這條路只有 Runtime 派送的 intent／host 對「正在盯的那則命令」的 cancel 會走到
      // （Gateway 依 priority 擋掉 AI 的低優先 idle），所以可以連安全訊息一起收——
      // AI 可達的 presentation `cancel` 不走這裡（CompanionApp 那邊不帶 force）。
      return { type: "clear-transient", force: true };
    case "listening":
      return t("listening");
    case "thinking":
      return t("thinking");
    case "routing":
      return t("routing");
    case "ask":
      return t("requesting-consent");
    case "act":
      return t("acting");
    case "waiting":
      return t("waiting-for-receipt");
    case "success":
      return t("succeeded", { verified: !frameSlice });
    case "blocked":
      return t("blocked");
    case "unknown":
      return t("unknown");
    case "failed":
      return t("failed");
    case "clicked":
      return t("clicked");
    case "dragged":
      return t("dragged");
    default:
      return t("performing", { animation: name, ...(frameSlice ? { frameSlice } : {}) });
  }
}

export function reduce(state: MachineState, event: MachineEvent, nowMs: number): MachineState {
  switch (event.type) {
    case "base": {
      if (event.base === "emergency" && state.base !== "emergency") {
        // 緊急停止：進行中的表演/互動/工作狀態一律下台，不得撐過 estop
        // （安全訊息本身留著）。停住的系統不繼續演任何東西。
        const t = state.transient;
        const keep = t && t.untilMs > nowMs && SAFETY_TRANSIENTS.has(t.kind) ? t : null;
        return { base: "emergency", transient: keep };
      }
      return { ...state, base: event.base };
    }
    case "clear-transient": {
      const t = state.transient;
      // 安全訊息（被擋下／失敗／未知）只能被 force（estop clear-all）清掉；一般 cancel 不動它。
      if (!event.force && t && t.untilMs > nowMs && SAFETY_TRANSIENTS.has(t.kind)) return state;
      return { ...state, transient: null };
    }
    case "transient": {
      // Emergency/offline suppress every ordinary transient (no dead poses
      // over a stopped system — the safe pose stays fixed).
      if (state.base === "emergency" || state.base === "offline") {
        return state;
      }
      const current = state.transient;
      const active = current && current.untilMs > nowMs ? current : null;
      const duration = event.durationMs ?? DURATION[event.kind];
      const outcome = transientCompetition(active, event);
      if (outcome === "keep") {
        return state; // higher-priority (or equally-scored) display keeps the stage
      }
      if (outcome === "refresh" && active) {
        // Same thing reported again: keep the running performance, extend it.
        return { ...state, transient: { ...active, untilMs: nowMs + duration } };
      }
      return {
        ...state,
        transient: {
          kind: event.kind,
          verified: event.verified,
          animation: event.animation,
          frameSlice: event.frameSlice,
          untilMs: nowMs + duration,
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
        // Director 挑的變體（poked／poked-flinch／…）；沒有就是 canonical 名（alias → poked）。
        return { animation: t.animation ?? "clicked", ambient: false };
      case "dragged":
        return { animation: t.animation ?? "dragged", ambient: false };
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
  /** RuntimeEvent.timestamp（事件實際發生的時間），用來擋掉重播。 */
  timestamp?: string;
}

// ---------------------------------------------------------------------------
// Replay guard
// ---------------------------------------------------------------------------
//
// SSE 有一個有界的 replay buffer。訂閱時若帶著舊的（或 0 的）Last-Event-ID，
// daemon 會把上一輪的事件重放進來——對角色而言那是一串「剛剛發生」的
// verified 綠勾、失敗與緊急停止，實際上都是好幾天前的事。時間戳早於這個
// App 啟動的事件一律不驅動演出。
//
// 安全狀態不靠這條路：emergency／paused 由 `/v1/status` 輪詢決定，所以丟掉
// 重播的 `emergency.stop` 不會讓畫面漏掉真正的停止狀態。

/** App 啟動時間 = 這個模組被載入的時間。 */
let appStartedAtMs = Date.now();

/** 測試用：重設「App 啟動時間」基準。 */
export function setAppStartedAt(ms: number): void {
  appStartedAtMs = ms;
}

/** 這則事件是不是「早於本次啟動」的重播？沒有時間戳就不猜（回 false）。 */
export function isReplayedBeforeStart(
  e: RuntimeEventLike,
  startedAtMs: number = appStartedAtMs
): boolean {
  if (typeof e.timestamp !== "string" || !e.timestamp) return false;
  const at = Date.parse(e.timestamp);
  return Number.isFinite(at) && at < startedAtMs;
}

/** 這個動作是不是「非 desktop-pet」的動器？（presentation 動器 id 以
 *  `companion.` 開頭；沒有 actuatorId 時保守地當成一般動作。） */
function isDeviceAction(e: RuntimeEventLike): boolean {
  const actuator = e.payload["actuatorId"];
  return typeof actuator === "string" && actuator !== "" && !actuator.startsWith("companion.");
}

/**
 * 舊路徑（daemon 沒有 characterProtocol、沒有 `character.intent` 事件時）
 * 事件 → 美術名的可注入表。預設值只用 pose() 的 canonical 詞彙（任何 renderer
 * 都能解析）；角色專屬表情（例如 shu-rig 的 device-hello／operate-tool）由
 * 該角色的 adapter tables 注入，machine 本身不認識任何角色部位或表情 id。
 */
export interface LegacyEventArt {
  /** provider 上線／配對。 */
  deviceOnline: string;
  /** provider 斷線／撤銷。 */
  deviceOffline: string;
  /** 非 desktop-pet 動器的 action.dispatched（硬體／外部工具）。 */
  operateExternal: string;
  /** action.acknowledged 的短點頭（acknowledged ≠ completed）。 */
  ackBrief: string;
  /** agent.session.state created：等待哪個 agent 取走任務。 */
  waitForAgent: (agentId: string) => string;
}

export const NEUTRAL_EVENT_ART: LegacyEventArt = {
  deviceOnline: "notice",
  deviceOffline: "notice",
  operateExternal: "act",
  ackBrief: "clicked",
  waitForAgent: () => "waiting",
};

/** Map one runtime event to a machine event (or null = no visual change). */
export function mapRuntimeEvent(
  e: RuntimeEventLike,
  art: LegacyEventArt = NEUTRAL_EVENT_ART
): MachineEvent | null {
  // 重播的舊事件不演：它們描述的是上一輪已經結束的結果。
  if (isReplayedBeforeStart(e)) return null;
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
      return { type: "transient", kind: "acting" };
    case "action.dispatched":
      // 非 desktop-pet 的動作（硬體、外部工具）：角色「在操作別的東西」。
      // 低優先 transient——安全狀態隨時搶佔。
      return isDeviceAction(e)
        ? { type: "transient", kind: "performing", animation: art.operateExternal, durationMs: 4000 }
        : { type: "transient", kind: "acting" };
    case "action.acknowledged":
      // acknowledged ≠ completed：裝置說「收到」就只短暫回應，不演成功。
      return isDeviceAction(e)
        ? { type: "transient", kind: "performing", animation: art.ackBrief, durationMs: 900 }
        : { type: "transient", kind: "waiting-for-receipt" };
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
      const waitExpr = art.waitForAgent(agent);
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
        case "unknown":
        // 租約到期／daemon 重啟：工作沒收尾、結果沒人知道（runtime 目前對租約
        // 到期發 timed-out，但 AgentSessionState 有 Expired，防禦性地一併映射）。
        case "expired":
          // 結果未知：既不是成功也不是失敗——誠實階梯要求演 unknown，
          // 不能停在上一個狀態（例如永遠的「工作中」）。
          return { type: "transient", kind: "unknown" };
        case "cancelled":
        case "closed":
          // 取消/收尾：誠實回到待機，不演成功也不演失敗。
          return { type: "clear-transient" };
        default:
          return null;
      }
    }
    case "provider.state-changed": {
      // 硬體/提供者上下線（spec §9）：「剛連上」與「連線沒了」各有一段短演出，
      // 兩者都只是「發生了」，不代表可用或成功。美術由角色 tables 注入。
      const state = String(e.payload["state"] ?? "").toLowerCase();
      if (state === "available" || state === "paired") {
        return { type: "transient", kind: "performing", animation: art.deviceOnline, durationMs: 1800 };
      }
      if (state === "disconnected" || state === "revoked") {
        return { type: "transient", kind: "performing", animation: art.deviceOffline, durationMs: 2200 };
      }
      return null;
    }
    case "consent.changed":
      return null;
    default:
      return null;
  }
}
