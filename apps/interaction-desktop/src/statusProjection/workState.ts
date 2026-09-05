// 工作狀態的投影（`AgentSessionState` → 人話）。
//
// 本檔是 `statusProjection` 這一組模組的基底：`Projection`／`ProjectedStatus`／
// `BadgeKind` 型別、介面不認得時的 `UNRECOGNIZED` 退路，以及 `ownKey` 這個
// 「這張表真的有這個 key 嗎」的型別守衛，都由這裡提供給其餘子領域共用。
//
// 對外一律經由 `../statusProjection.ts` 這個匯總檔（既有 import 路徑不變）。

/** 會到達 UI 的每一個工作狀態原始值：
 *  - `AgentSessionState` enum（kebab-case：created／active／waiting-for-input／
 *    waiting-for-consent／claimed-completed／failed／timed-out／cancelled／expired／
 *    closed／unknown）——`/v1/agents/sessions` 與統一收件匣 `agent-session` 項目；
 *  - `agent.session.state` 角色 taxonomy 事件（created／fetched／working／
 *    waiting-input／waiting-consent／claimed-completed／verified／failed／unknown／
 *    timed-out／cancelled／closed）；
 *  - CPP §4.2 truthState（queued／working／blocked／…）。 */
export type WorkState =
  | "created"
  | "queued"
  | "fetched"
  | "active"
  | "working"
  | "waiting-for-input"
  | "waiting-input"
  | "waiting-for-consent"
  | "waiting-consent"
  | "blocked"
  | "claimed-completed"
  | "verified"
  | "failed"
  | "timed-out"
  | "expired"
  | "unknown"
  | "cancelled"
  | "closed";

/** 語意分類（給程式判斷用，不是給人看的）。 */
export type ProjectionKind =
  | "preparing"
  | "working"
  | "needs-input"
  | "needs-consent"
  | "blocked"
  | "claimed"
  | "verified"
  | "failed"
  | "timed-out"
  | "expired"
  | "unknown"
  | "stopped";

/** `ui.tsx` 的 `Badge` 樣式代號（`badge-<kind>`）。 */
export type BadgeKind = "ok" | "pending" | "info" | "bad" | "warn" | "muted";

export interface Projection {
  /** 一般模式的主要標籤（人話，固定文案）。 */
  label: string;
  kind: ProjectionKind;
  badge: BadgeKind;
  /** 需要人類裁決（等你回答／等你允許／對方說已完成）。 */
  needsDecision: boolean;
  /** 誠實註記，例如「對方的說法，尚未檢查」；沒有就省略。 */
  honesty?: string;
}

/** 投影結果：多帶原始值與「介面是否認得這個值」。 */
export interface ProjectedStatus extends Projection {
  raw: string;
  known: boolean;
}

const PREPARING: Projection = {
  label: "正在準備",
  kind: "preparing",
  badge: "pending",
  needsDecision: false,
};
/** 任務真的被取走了（gateway 送進子程序／輪詢型 agent fetch）——但還沒有結果。 */
const FETCHED: Projection = {
  label: "已交給工作助手",
  kind: "preparing",
  badge: "pending",
  needsDecision: false,
};
const WORKING: Projection = {
  label: "處理中",
  kind: "working",
  badge: "info",
  needsDecision: false,
};
const NEEDS_INPUT: Projection = {
  label: "等你回答",
  kind: "needs-input",
  badge: "warn",
  needsDecision: true,
};
const NEEDS_CONSENT: Projection = {
  label: "等你允許",
  kind: "needs-consent",
  badge: "warn",
  needsDecision: true,
};
/** 取消與關閉在一般模式是同一件事：這個工作不再進行了。 */
const CANCELLED: Projection = {
  label: "已取消",
  kind: "stopped",
  badge: "muted",
  needsDecision: false,
};

/** 工作狀態對照表（spec 表格文案一字不改）。 */
export const WORK_STATE_PROJECTION = {
  created: PREPARING,
  queued: PREPARING,
  fetched: FETCHED,
  active: WORKING,
  working: WORKING,
  "waiting-for-input": NEEDS_INPUT,
  "waiting-input": NEEDS_INPUT,
  "waiting-for-consent": NEEDS_CONSENT,
  "waiting-consent": NEEDS_CONSENT,
  blocked: {
    label: "無法繼續",
    kind: "blocked",
    badge: "bad",
    needsDecision: false,
  },
  // 誠實階梯：claimed 只是對方的說法，沒有綠勾、沒有慶祝，而且要你裁決。
  "claimed-completed": {
    label: "對方說已完成",
    kind: "claimed",
    badge: "warn",
    needsDecision: true,
    honesty: "對方的說法，尚未檢查",
  },
  verified: {
    label: "已由你確認",
    kind: "verified",
    badge: "ok",
    needsDecision: false,
    honesty: "由你親自確認",
  },
  failed: {
    label: "失敗",
    kind: "failed",
    badge: "bad",
    needsDecision: false,
  },
  "timed-out": {
    label: "逾時失敗",
    kind: "timed-out",
    badge: "bad",
    needsDecision: false,
  },
  expired: {
    label: "已到期",
    kind: "expired",
    badge: "muted",
    needsDecision: false,
  },
  unknown: {
    label: "結果不確定",
    kind: "unknown",
    badge: "warn",
    needsDecision: false,
    honesty: "既不是成功也不是失敗",
  },
  cancelled: CANCELLED,
  closed: CANCELLED,
} satisfies Record<WorkState, Projection>;

/** 全部工作狀態（由對照表導出，保證與型別一致）。 */
export const WORK_STATES: readonly WorkState[] = Object.keys(
  WORK_STATE_PROJECTION
) as WorkState[];

/** 介面不認得的原始值：不猜，照實說「結果不確定」。
 *  收件匣（`inbox.ts`）共用同一條退路，所以由本檔匯出。 */
export const UNRECOGNIZED: Projection = {
  ...WORK_STATE_PROJECTION.unknown,
  honesty: "介面不認得這個狀態，不猜測結果",
};

/** 「這張對照表真的有這個 key 嗎」的型別守衛（`hasOwnProperty`，不看原型鏈）。
 *  `statusProjection/` 的每個子領域都拿它把原始字串收窄成表上的 key。 */
export function ownKey<K extends string>(table: Record<K, unknown>, raw: string): raw is K {
  return Object.prototype.hasOwnProperty.call(table, raw);
}

export function isWorkState(raw: string): raw is WorkState {
  return ownKey(WORK_STATE_PROJECTION, raw);
}

/** 工作狀態 → 人話。未知原始值 → 「結果不確定」＋ `known: false`，
 *  永遠不會把原始字串當標籤回傳。 */
export function projectWorkState(raw: string): ProjectedStatus {
  if (isWorkState(raw)) return { ...WORK_STATE_PROJECTION[raw], raw, known: true };
  return { ...UNRECOGNIZED, raw, known: false };
}

/** 「進行中」的語意分類：對應 Rust `AgentSessionState::is_open`
 *  （created／active／waiting-*／claimed-completed）。 */
export const OPEN_WORK_KINDS: readonly ProjectionKind[] = [
  "preparing",
  "working",
  "needs-input",
  "needs-consent",
  "claimed",
];

/** 這個工作狀態算不算「進行中」（介面不認得的值不算，不假裝在跑）。 */
export function isOpenWorkState(raw: string): boolean {
  const p = projectWorkState(raw);
  return p.known && OPEN_WORK_KINDS.includes(p.kind);
}
