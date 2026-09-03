// 一般模式的狀態投影（Character Presentation Protocol §4.2 truthState／§11
// truth projection 在 UI 側的鏡射）。
//
// 所有頁面（收件匣徽章、AI 工作階段卡片、「現在」摘要、全域搜尋）共用
// 這一份「Runtime 原始 taxonomy 字串 → 人話」對照，而且在型別上窮舉：
// Runtime 多一個狀態而這裡沒有投影，`satisfies Record<WorkState, Projection>`
// 會讓 typecheck 失敗，不會靜默退化成把原始字串印到畫面上。
//
// 誠實階梯：
// - claimed ≠ verified：Agent 說做完了只是「它的說法」，等待你檢查。
// - unknown 既不是成功也不是失敗，只能說「結果不確定」。
// - 介面不認得的原始值一律投影成「結果不確定」並標 `known: false`；
//   一般模式絕不把原始字串當主要標籤，進階模式才在次要的 muted 行顯示原始值。
// - 這裡只做「翻譯」，不做升級：沒有任何路徑能把 claimed 翻成 verified。

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
  /** 需要人類裁決（等你補充／等你同意／等待檢查）。 */
  needsDecision: boolean;
  /** 誠實註記，例如「Agent 的說法，尚未檢查」；沒有就省略。 */
  honesty?: string;
}

/** 投影結果：多帶原始值與「介面是否認得這個值」。 */
export interface ProjectedStatus extends Projection {
  raw: string;
  known: boolean;
}

const PREPARING: Projection = {
  label: "準備中",
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
  label: "等你補充",
  kind: "needs-input",
  badge: "warn",
  needsDecision: true,
};
const NEEDS_CONSENT: Projection = {
  label: "等你同意",
  kind: "needs-consent",
  badge: "warn",
  needsDecision: true,
};
const STOPPED: Projection = {
  label: "已停止",
  kind: "stopped",
  badge: "muted",
  needsDecision: false,
};

/** 工作狀態對照表（spec 表格文案一字不改）。 */
export const WORK_STATE_PROJECTION = {
  created: PREPARING,
  queued: PREPARING,
  fetched: PREPARING,
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
  "claimed-completed": {
    label: "Agent 說已完成，等待檢查",
    kind: "claimed",
    badge: "warn",
    needsDecision: true,
    honesty: "Agent 的說法，尚未檢查",
  },
  verified: {
    label: "已確認完成",
    kind: "verified",
    badge: "ok",
    needsDecision: false,
    honesty: "由你親自確認",
  },
  failed: {
    label: "執行失敗",
    kind: "failed",
    badge: "bad",
    needsDecision: false,
  },
  "timed-out": {
    label: "執行逾時",
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
  cancelled: STOPPED,
  closed: STOPPED,
} satisfies Record<WorkState, Projection>;

/** 全部工作狀態（由對照表導出，保證與型別一致）。 */
export const WORK_STATES: readonly WorkState[] = Object.keys(
  WORK_STATE_PROJECTION
) as WorkState[];

/** 介面不認得的原始值：不猜，照實說「結果不確定」。 */
const UNRECOGNIZED: Projection = {
  ...WORK_STATE_PROJECTION.unknown,
  honesty: "介面不認得這個狀態，不猜測結果",
};

function ownKey<K extends string>(table: Record<K, unknown>, raw: string): raw is K {
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

// ---------------------------------------------------------------------------
// 統一收件匣（activity.rs `activity_inbox`）：除了工作狀態之外還會出現
// 知識候選、動作收據狀態（ActionStatus）與安全事件型別。
// ---------------------------------------------------------------------------

/** 收件匣才會出現、不是工作狀態的原始值。 */
export type InboxOnlyStatus =
  // knowledge-review
  | "candidate"
  // action-result（ActionStatus；blocked／failed／cancelled／expired 與工作狀態同名，走上表）
  | "planned"
  | "authorized"
  | "accepted"
  | "dispatched"
  | "acknowledged"
  | "observed"
  | "completed"
  | "uncertain"
  | "stopped"
  // safety-event（activity.rs：emergency.stop 事件依 payload.cleared 分成觸發／解除）
  | "emergency"
  | "emergency-cleared"
  | "sensor.started"
  | "sensor.stopped";

export type InboxStatus = WorkState | InboxOnlyStatus;

/** 動作收據的文案沿用 `appstate.actionStatusLabel`（同一句話不說兩種）；
 *  分類依 CPP §11：dispatched／acknowledged → working、completed → claimed、
 *  observed → verified、uncertain → unknown。 */
export const INBOX_ONLY_PROJECTION = {
  candidate: {
    label: "等待確認",
    kind: "needs-input",
    badge: "warn",
    needsDecision: true,
  },
  planned: {
    label: "已規劃",
    kind: "preparing",
    badge: "pending",
    needsDecision: false,
  },
  authorized: {
    label: "已授權",
    kind: "preparing",
    badge: "pending",
    needsDecision: false,
  },
  accepted: {
    label: "已排入（尚未執行）",
    kind: "preparing",
    badge: "pending",
    needsDecision: false,
  },
  dispatched: {
    label: "已送出（等待確認）",
    kind: "working",
    badge: "info",
    needsDecision: false,
  },
  acknowledged: {
    label: "已收到（效果未確認）",
    kind: "working",
    badge: "info",
    needsDecision: false,
  },
  observed: {
    label: "已觀察到效果",
    kind: "verified",
    badge: "ok",
    needsDecision: false,
  },
  completed: {
    label: "已完成",
    kind: "claimed",
    badge: "ok",
    needsDecision: false,
  },
  uncertain: {
    label: "結果不確定",
    kind: "unknown",
    badge: "warn",
    needsDecision: false,
    honesty: "既不是成功也不是失敗",
  },
  stopped: {
    label: "已停止",
    kind: "stopped",
    badge: "muted",
    needsDecision: false,
  },
  emergency: {
    label: "緊急停止",
    kind: "stopped",
    badge: "bad",
    needsDecision: false,
  },
  // 解除是另一個事件，不是再一次「緊急停止」：剛解除的人看到的必須是「已解除」。
  "emergency-cleared": {
    label: "緊急停止已解除",
    kind: "stopped",
    badge: "ok",
    needsDecision: false,
  },
  "sensor.started": {
    label: "感測使用中",
    kind: "working",
    badge: "warn",
    needsDecision: false,
  },
  "sensor.stopped": {
    label: "感測已停止",
    kind: "stopped",
    badge: "muted",
    needsDecision: false,
  },
} satisfies Record<InboxOnlyStatus, Projection>;

export const INBOX_STATUSES: readonly InboxStatus[] = [
  ...WORK_STATES,
  ...(Object.keys(INBOX_ONLY_PROJECTION) as InboxOnlyStatus[]),
];

/** 收件匣狀態 → 人話。先查工作狀態，再查收件匣專屬狀態；
 *  都不認得 → 「結果不確定」＋ `known: false`。 */
export function projectInboxStatus(raw: string): ProjectedStatus {
  if (isWorkState(raw)) return projectWorkState(raw);
  if (ownKey(INBOX_ONLY_PROJECTION, raw)) {
    return { ...INBOX_ONLY_PROJECTION[raw], raw, known: true };
  }
  return { ...UNRECOGNIZED, raw, known: false };
}

/** 收件匣項目種類（activity.rs `ActivityInboxItem.kind`）。 */
export type InboxKind =
  | "agent-session"
  | "action-result"
  | "safety-event"
  | "knowledge-review"
  | "ai-assist";

const INBOX_KIND_LABEL = {
  "agent-session": "AI 工作階段",
  "action-result": "互動結果",
  "safety-event": "安全事件",
  "knowledge-review": "知識審核",
  "ai-assist": "AI 協助判斷",
} satisfies Record<InboxKind, string>;

/** 收件匣種類的人話；不認得的種類說「其他活動」，不回原始字串。 */
export function inboxKindLabel(kind: string): string {
  return ownKey(INBOX_KIND_LABEL, kind) ? INBOX_KIND_LABEL[kind] : "其他活動";
}

/** 安全事件的人話標題（依投影後的狀態）。 */
const SAFETY_EVENT_TITLE = {
  emergency: "緊急停止已啟動",
  "emergency-cleared": "緊急停止已解除",
  "sensor.started": "感測開始",
  "sensor.stopped": "感測結束",
} satisfies Partial<Record<InboxOnlyStatus, string>>;

/** 原始事件型別字串（`emergency.stop`／`sensor.started`…）：小寫英文加點或連字號。 */
const RAW_EVENT_TYPE = /^[a-z][a-z0-9-]*(\.[a-z][a-z0-9-]*)+$/;

/** 收件匣項目的顯示標題。
 *  安全事件的 `title` 在舊 daemon 是原始 event_type（`emergency.stop`）——
 *  一般模式不得把它當標題印出來：改用狀態對應的人話（若 detail 帶受器名稱就一併說），
 *  其餘種類照後端給的人話標題。認不得的原始事件型別退回「安全事件」。 */
export function inboxItemTitle(item: {
  kind?: unknown;
  status?: unknown;
  title?: unknown;
  detail?: unknown;
}): string {
  const title = typeof item.title === "string" ? item.title : "";
  if (item.kind !== "safety-event") return title;
  const status = typeof item.status === "string" ? item.status : "";
  if (title && !RAW_EVENT_TYPE.test(title) && title !== status) return title;
  const detail = (item.detail && typeof item.detail === "object" ? item.detail : {}) as Record<
    string,
    unknown
  >;
  const payload = (detail.payload && typeof detail.payload === "object" ? detail.payload : detail) as Record<
    string,
    unknown
  >;
  // sensors.rs 的 payload 是 `{"sensor": kind}`；只翻譯認得的種類，不外洩原始 id。
  const sensor = typeof payload.sensor === "string" ? payload.sensor.toLowerCase() : "";
  const subject = sensor
    ? sensor === "microphone" || sensor.includes("mic")
      ? "麥克風"
      : sensor === "camera" || sensor.includes("cam")
        ? "攝影機"
        : "感測器"
    : "";
  if (ownKey(SAFETY_EVENT_TITLE, status)) {
    const base = SAFETY_EVENT_TITLE[status];
    return subject && status.startsWith("sensor.") ? `${base}：${subject}` : base;
  }
  return inboxKindLabel("safety-event");
}

// ---------------------------------------------------------------------------
// 其他會在同一批畫面出現、同樣不該外洩原始值的識別字
// ---------------------------------------------------------------------------

/** Provider 生命週期（`ProviderState`，kebab-case）。文案沿用 CapabilitiesHub。 */
export type ProviderState =
  | "discovered"
  | "unpaired"
  | "paired"
  | "installed"
  | "disabled"
  | "available"
  | "busy"
  | "degraded"
  | "disconnected"
  | "expired"
  | "revoked"
  | "closed";

export interface ProviderProjection {
  label: string;
  badge: BadgeKind;
}

export const PROVIDER_STATE_PROJECTION = {
  available: { label: "可用", badge: "ok" },
  busy: { label: "忙碌中", badge: "ok" },
  degraded: { label: "部分可用", badge: "warn" },
  discovered: { label: "已發現（未配對）", badge: "pending" },
  unpaired: { label: "未配對", badge: "pending" },
  paired: { label: "已配對（未安裝）", badge: "pending" },
  installed: { label: "已安裝（未啟用）", badge: "pending" },
  disabled: { label: "已停用", badge: "warn" },
  disconnected: { label: "未連線", badge: "bad" },
  expired: { label: "已過期", badge: "bad" },
  revoked: { label: "已撤銷", badge: "bad" },
  closed: { label: "已關閉", badge: "bad" },
} satisfies Record<ProviderState, ProviderProjection>;

const PROVIDER_UNRECOGNIZED: ProviderProjection = { label: "狀態不確定", badge: "warn" };

export function projectProviderState(
  raw: string
): ProviderProjection & { raw: string; known: boolean } {
  if (ownKey(PROVIDER_STATE_PROJECTION, raw)) {
    return { ...PROVIDER_STATE_PROJECTION[raw], raw, known: true };
  }
  return { ...PROVIDER_UNRECOGNIZED, raw, known: false };
}

/** 能力卡種類（`HumanCard.kind`）。文案沿用 CapabilityCard。 */
export type CapabilityKind = "receptor" | "actuator" | "tool-operation";

const CAPABILITY_KIND_LABEL = {
  receptor: "感知來源",
  actuator: "回應方式",
  "tool-operation": "工具操作",
} satisfies Record<CapabilityKind, string>;

export function capabilityKindLabel(kind: string): string {
  return ownKey(CAPABILITY_KIND_LABEL, kind) ? CAPABILITY_KIND_LABEL[kind] : "能力";
}

/** 本機 agent id 的顯示名稱。agent id 是身分不是狀態，不認得的照原樣顯示。 */
export function agentDisplayLabel(agentId: string): string {
  return agentId === "codex" ? "Codex" : agentId === "claude-code" ? "Claude Code" : agentId;
}
