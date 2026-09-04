import type { AdapterLifecycleState } from "./character/protocol";

// 一般模式的狀態投影（Character Presentation Protocol §4.2 truthState／§11
// truth projection 在 UI 側的鏡射）。
//
// 所有頁面（收件匣徽章、AI 工作階段卡片、「現在」摘要、全域搜尋）共用
// 這一份「Runtime 原始 taxonomy 字串 → 人話」對照，而且在型別上窮舉：
// Runtime 多一個狀態而這裡沒有投影，`satisfies Record<WorkState, Projection>`
// 會讓 typecheck 失敗，不會靜默退化成把原始字串印到畫面上。
//
// 誠實階梯：
// - claimed ≠ verified：對方說做完了只是「它的說法」，等待你檢查。
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
  | "sensor.stopped"
  | "sensor.stop-uncertain";

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
  // 誠實階梯：要求停止 ≠ 已停止。裝置沒回覆時它可能還在擷取，
  // 所以這一筆是「要你處理」的，不是純歷史。
  "sensor.stop-uncertain": {
    label: "停止結果不確定",
    kind: "unknown",
    badge: "warn",
    needsDecision: true,
    honesty: "裝置沒有回覆，可能仍在感測",
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

// ---------------------------------------------------------------------------
// 待決定計數的誠實度（activity.rs `pendingCountExact`）
// ---------------------------------------------------------------------------

/** 後端說 `pendingCount` 是不是完整總數。
 *  舊 daemon 不送這個欄位（undefined）＝ 精確；只有明確的 `false` 才是「至少」。
 *  不是布林的值一律當成不精確（寧可說「至少」，也不宣稱數字是全部）。 */
export function isPendingCountExact(inbox: unknown): boolean {
  if (!inbox || typeof inbox !== "object") return true;
  const raw = (inbox as Record<string, unknown>).pendingCountExact;
  return raw === undefined || raw === true;
}

/** 待決定數的人話：不精確時它只是下限，一定要說「至少」。 */
export function pendingCountLabel(count: number, exact: boolean): string {
  return exact ? `${count} 項` : `至少 ${count} 項`;
}

/** `pendingCountExact === false` 時的共用說明。
 *  這種情況下**絕不可以**說「目前沒有待決定事項」——後端只是沒把全部撈完。 */
export const PENDING_INCOMPLETE_NOTE = "還有未載入的待決定項，請到活動紀錄查看";

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
  "sensor.stop-uncertain": "感測停止結果不確定",
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

const KNOWLEDGE_TRIGGER_LABEL: Record<string, string> = {
  "user-correction": "你的更正",
  "review-overdue": "複審到期",
  "conflict-detected": "發現衝突",
  "task-experience": "工作經驗",
  "human-review": "人工複審",
};

/** 原始機器 id 的形狀（小寫英數與 `. _ -`）：`user-correction`／`emergency-stop`
 *  這種值不上一般模式的第一層文字；人類寫的描述（有空白或中文）照原樣顯示。 */
const MACHINE_ID = /^[a-z0-9][a-z0-9._-]*$/i;

/** 知識更新的來由（Runtime 的 `triggeredBy`，kebab-case 原始值）翻成人話。
 *  認不得的原始 id 不外洩，一律說「系統」——寧可少說，不假裝看得懂。 */
export function knowledgeTriggerLabel(raw: string): string {
  if (ownKey(KNOWLEDGE_TRIGGER_LABEL, raw)) return KNOWLEDGE_TRIGGER_LABEL[raw];
  return MACHINE_ID.test(raw) ? "系統" : raw;
}

const RECEIPT_INTENT_LABEL: Record<string, string> = {
  "emergency-stop": "緊急停止",
  "companion-test": "角色測試",
  notify: "通知",
  speak: "說話",
};

/** 動作意圖（互動結果的 `intent`）：Runtime 的原始 id（`emergency-stop`／
 *  `companion-test`／`presence`）不該出現在一般模式的第一層文字裡。認得的翻成
 *  人話，其餘只說「一個需要回應的訊號」；人類寫的描述（含空白或中文）照原樣顯示。
 *  活動紀錄、「現在」頁與全域搜尋共用這一份，不各自拼一套。 */
export function receiptIntentLabel(intent: string): string {
  if (ownKey(RECEIPT_INTENT_LABEL, intent)) return RECEIPT_INTENT_LABEL[intent];
  // 原始機器 id 不上畫面；人類寫的描述照原樣顯示。
  return MACHINE_ID.test(intent) ? "一個需要回應的訊號" : intent;
}

/**
 * 收件匣項目的「裝置」人話（`ActivityInboxItem.deviceId`）。
 *
 * 後端的 deviceId 是原始識別碼：互動結果是動器 id（`builtin.notification`），
 * 安全事件是感測來源或手機 id（`iphone-a1b2c3d4`／`iphone.mic-level`）。一般模式
 * 不得把它印出來（spec §15.5），但也不能因此連「哪一台」都不說：
 *  - 能力清單查得到名字 → 用那個名字；
 *  - `iphone-…`／`iphone.…` → 「你的 iPhone」；
 *  - 認得的感測種類 → 麥克風／攝影機／定位；
 *  - 其餘一律回 `null`：寧可不說，也不外洩原始識別碼（進階模式另有原始值那一行）。
 *
 * @param deviceId 後端給的原始值（形狀不可信）。
 * @param resolveName 呼叫端的能力名稱查詢；查不到請回 `null`／`undefined`
 *                    （回傳與 id 相同的字串一律視為沒查到）。
 */
export function inboxDeviceLabel(
  deviceId: unknown,
  resolveName?: (id: string) => string | null | undefined
): string | null {
  const raw = typeof deviceId === "string" ? deviceId.trim() : "";
  if (raw.length === 0) return null;
  const named = resolveName?.(raw);
  const name = typeof named === "string" ? named.trim() : "";
  if (name.length > 0 && name !== raw) return name;
  if (/^iphone([-.].*)?$/i.test(raw)) return "你的 iPhone";
  const kind = sensorKindLabel(raw);
  return kind === "其他感測器" ? null : kind;
}

// ---------------------------------------------------------------------------
// 感測：種類的人話，與「停止所有感測」的誠實回報
// ---------------------------------------------------------------------------

/** 感測來源種類的人話。認不得的種類（`iphone.motion` 這種原始 id）不猜、也不外洩原始
 *  字串——一般模式一律說「其他感測器」，使用者仍看得到「有東西在感測」這件事實。 */
export function sensorKindLabel(kind: string): string {
  const k = kind.toLowerCase();
  if (k === "microphone" || k.includes("mic")) return "麥克風";
  if (k.includes("camera") || k.includes("cam")) return "攝影機";
  if (k.includes("location") || k.includes("gps")) return "定位";
  return "其他感測器";
}

/**
 * 「是誰開始感測的」的人話（`SensorUse.startedBy`）。
 *
 * 原始值是內部身分字串（`iphone:iphone-87b4…` 這種裝置 id、`api`、`cli`…），一般模式
 * 直接印出來只是外洩實作細節、對使用者沒有意義。這裡只把**認得**的來源翻成人話；
 * 認不得的一律說「系統」——不猜、也絕不冒充成「你」（把系統自動啟動的感測說成使用者
 * 自己開的，是感測透明度的謊）。
 *
 * @param startedBy runtime 回報的原始值。
 * @param deviceName 若呼叫端知道那台裝置的名字就用它（目前 `SensorUse` 不帶，保留給
 *                   帶得出名字的呼叫端；空字串／未提供時退回通用標籤）。
 */
export function sensorStartedByLabel(
  startedBy: string | null | undefined,
  deviceName?: string | null
): string {
  const raw = typeof startedBy === "string" ? startedBy.trim() : "";
  if (raw.startsWith("iphone:")) {
    const name = typeof deviceName === "string" ? deviceName.trim() : "";
    return name.length > 0 ? name : "你的 iPhone";
  }
  if (raw === "user") return "你";
  if (raw === "desktop" || raw === "api" || raw === "cli") return "這台電腦";
  return "系統";
}

export interface SensorStopProjection {
  /** true 只代表「已確認全部停止」；任何不確定都必須是 false。 */
  ok: boolean;
  message: string;
}

function uniqueLabels(values: string[]): string[] {
  return values.filter((v, i) => v.length > 0 && values.indexOf(v) === i);
}

/**
 * 「停止所有感測」之後可以誠實說出口的一句話。
 *
 * 誠實階梯：送出請求 ≠ 已停止；裝置沒回覆是「結果不確定」，既不是成功也不是失敗。
 * 判斷主要看**重新讀取**到的 activeSensors（Runtime 的真實狀態），其次才看回報裡的
 * 每台裝置結果。舊 daemon 只回 `{stopped:true}`（沒有 devices／uncertain），這裡容忍
 * 缺欄位；但缺欄位不會被升級成「已確認停止」——只有重讀清單真的空了才敢這樣說。
 *
 * @param report `/v1/sensors/stop` 的回報（形狀不可信，任何值都要能吃）。
 * @param remaining 停止後重新讀到的 activeSensors；`null` ＝ 讀不到（查詢失敗）。
 */
export function projectSensorStop(
  report: unknown,
  remaining: readonly { kind?: unknown; state?: unknown }[] | null
): SensorStopProjection {
  const raw = (report && typeof report === "object" ? report : {}) as Record<string, unknown>;
  const devices = Array.isArray(raw.devices) ? (raw.devices as Record<string, unknown>[]) : [];
  const unsure = devices.filter((d) => {
    const outcome = d && typeof d === "object" ? d.outcome : undefined;
    return typeof outcome !== "string" || outcome !== "stopped";
  });
  const uncertain = raw.uncertain === true || raw.stopped === false || unsure.length > 0;

  if (remaining === null) {
    return {
      ok: false,
      message: "已要求停止，但目前無法確認感測狀態（系統查詢失敗）。請到「連接與權限」再確認一次。",
    };
  }
  const still = uniqueLabels(
    remaining.map((s) => sensorKindLabel(typeof s.kind === "string" ? s.kind : ""))
  );
  if (still.length > 0) {
    return {
      ok: false,
      message: `已要求停止，但仍在使用中：${still.join("、")}。手機上的感測要在手機上停止，或到「連接與權限」撤銷那台裝置。`,
    };
  }
  if (uncertain) {
    const who = uniqueLabels(
      unsure.map((d) => (typeof d.name === "string" && d.name.trim() ? d.name.trim() : "某台裝置"))
    );
    return {
      ok: false,
      message: `已要求停止，結果不確定（${who.length > 0 ? who.join("、") : "有來源"}未回覆）。`,
    };
  }
  return { ok: true, message: "已停止感測。" };
}

// ---------------------------------------------------------------------------
// 角色生命週期（Character Presentation Protocol §7 `AdapterLifecycleState`；
// TS 鏡射見 `character/protocol.ts`，鏡射 Rust `interaction-character::lifecycle`）：
// 角色頁把 `instance.lifecycle`＋presence 轉成一句人話徽章的唯一投影入口。
// ---------------------------------------------------------------------------

/** 語意桶（給程式判斷用，不是給人看的）。 */
export type CharacterLifecycleBucket = "crashed" | "hidden" | "ready" | "pending";

/** 14 個生命週期原始值 → 語意桶。`satisfies Record<AdapterLifecycleState, …>` 窮舉：
 *  Runtime 未來新增生命週期值而這裡沒有分類，typecheck 會失敗，不會靜默漏掉。 */
export const CHARACTER_LIFECYCLE_BUCKET = {
  discovered: "pending",
  loading: "pending",
  validated: "pending",
  initializing: "pending",
  negotiating: "pending",
  ready: "ready",
  shown: "ready",
  hidden: "hidden",
  suspended: "hidden",
  resumed: "ready",
  reconfiguring: "ready",
  disposed: "crashed",
  crashed: "crashed",
  reconnecting: "crashed",
} satisfies Record<AdapterLifecycleState, CharacterLifecycleBucket>;

/** 角色目前無法顯示（崩潰／失聯）時的固定文案；安全訊息改以此顯示，不外洩原始錯誤。 */
export const CHARACTER_UNAVAILABLE_TEXT = "角色目前無法顯示，改用文字";

export interface CharacterLifecycleProjection {
  label: string;
  kind: BadgeKind;
  detail: string;
}

/** 介面不認得的生命週期原始值不猜、不外洩——退回中立的「準備中」桶。 */
function characterLifecycleBucket(raw: string): CharacterLifecycleBucket | null {
  return Object.prototype.hasOwnProperty.call(CHARACTER_LIFECYCLE_BUCKET, raw)
    ? CHARACTER_LIFECYCLE_BUCKET[raw as AdapterLifecycleState]
    : null;
}

/** Runtime 角色實例（優先）＋ presence 推導出一句人話徽章；
 *  沒有任何回報就誠實說未連線，未知的原始生命週期值一律退回「準備中」。 */
export function projectCharacterLifecycle(
  instance: { lifecycle: string; connected: boolean } | null,
  presence: Record<string, unknown> | null
): CharacterLifecycleProjection {
  if (instance) {
    const bucket = characterLifecycleBucket(instance.lifecycle);
    if (!instance.connected || bucket === "crashed") {
      return {
        label: CHARACTER_UNAVAILABLE_TEXT,
        kind: "warn",
        detail: "角色的呈現程式已停止或失去連線；安全訊息會改以固定文字顯示，系統與進行中的工作不受影響。",
      };
    }
    if (bucket === "hidden" || presence?.visible === false) {
      return { label: "已隱藏", kind: "muted", detail: "角色視窗已連線但目前隱藏；打開「顯示桌面角色」就會出現。" };
    }
    if (bucket === "ready") {
      return { label: "角色視窗運作中", kind: "ok", detail: "角色視窗已連線並正在呈現。" };
    }
    return { label: "準備中", kind: "pending", detail: "角色視窗正在載入。" };
  }
  if (presence?.connected === true) {
    return presence.visible === true
      ? { label: "角色視窗運作中", kind: "ok", detail: "角色視窗已連線並正在呈現。" }
      : { label: "已隱藏", kind: "muted", detail: "角色視窗已連線但目前隱藏；打開「顯示桌面角色」就會出現。" };
  }
  return {
    label: "角色視窗未連線",
    kind: "bad",
    detail: "桌面角色視窗沒有連上（瀏覽器檢視沒有角色視窗）。安全訊息仍會以固定文字顯示在控制中心。",
  };
}

// ---------------------------------------------------------------------------
// 角色同步（AIP Character Session；契約 `docs/aip/character-session.md` §11）：
// 一般模式只看得到人話。權威狀態、revision／sequence／epoch、counters 全部留在
// 進階模式的「連接診斷」，這裡的輸出一個技術詞都不得出現。
//
// 誠實階梯：
// - 讀不到權威狀態 ≠ 已同步：一律「同步尚未完成」，不用上一次的樣子冒充現在。
// - 空狀態 ≠ 成功：一台裝置都沒有時是中性的「尚未連接 iPhone」（muted），不是綠勾。
// - 認不得的 presence 不猜：退回「同步尚未完成」並標 `known: false`。
// - 模擬 iPhone（fixture）的名稱本身已含標籤，投影原樣顯示、不再加工，也不會
//   把它寫成真機。
// ---------------------------------------------------------------------------

/** 十種同步狀態（`satisfies Record<CharacterSyncState, …>` 窮舉）。 */
export type CharacterSyncState =
  | "synced"
  | "reconnecting"
  | "offline"
  | "partial-capability"
  | "syncing"
  | "unrecoverable"
  | "needs-reconfirmation"
  | "no-device"
  | "disabled"
  | "store-reset";

export interface CharacterSyncProjection {
  /** 一般模式的主要句子（契約 §11 文案表，一字不改）。 */
  headline: string;
  /** 一句補充：說清楚「現在能相信什麼」，不是安慰話。 */
  detail: string;
  tone: BadgeKind;
}

/** §11 文案表。新增狀態而沒有文案，typecheck 會失敗。 */
export const CHARACTER_SYNC_PROJECTION = {
  synced: {
    headline: "iPhone 已連接，角色狀態已同步",
    detail: "手機上的角色和這台電腦看到的是同一個狀態。",
    tone: "ok",
  },
  reconnecting: {
    headline: "iPhone 正在重新連線",
    detail: "連線斷了一下，正在接回來；這段時間的互動不會補播。",
    tone: "pending",
  },
  offline: {
    headline: "iPhone 暫時離線",
    detail: "手機現在收不到角色狀態，也送不出互動；接回來之後才會重新對齊。",
    tone: "warn",
  },
  "partial-capability": {
    headline: "部分能力目前不可用",
    detail: "這台裝置接上了，但它做不到角色的部分表演；做不到的不會假裝做到。",
    tone: "warn",
  },
  syncing: {
    headline: "同步尚未完成",
    detail: "還在對齊角色狀態；在這之前不要把畫面上的樣子當成最新的。",
    tone: "pending",
  },
  unrecoverable: {
    headline: "無法恢復，請重新連接",
    detail: "連續好幾次都對不齊角色狀態，需要你重新連一次裝置。",
    tone: "bad",
  },
  "needs-reconfirmation": {
    headline: "需要重新確認裝置",
    detail: "這台裝置的授權已經撤銷；要再同步角色，必須重新確認一次。",
    tone: "warn",
  },
  "no-device": {
    headline: "尚未連接 iPhone",
    detail: "目前只有這台電腦在陪你；連上手機之後才會有東西可以同步。",
    tone: "muted",
  },
  disabled: {
    headline: "角色同步目前關閉",
    detail: "這台電腦沒有啟用角色同步；其他功能不受影響。",
    tone: "muted",
  },
  // 保存的同步紀錄壞掉、已被隔離並重新開始（Runtime 的 `storeNote`）。
  // 不靜默：這件事會讓已連接的裝置重新對齊一次，使用者有權知道；但它不是
  // 緊急狀況（不給紅色），也不是成功（不給綠色）。
  "store-reset": {
    headline: "角色同步紀錄曾損毀，已重新開始",
    detail: "已重新連接的裝置會重新同步；不影響角色本身。",
    tone: "warn",
  },
} satisfies Record<CharacterSyncState, CharacterSyncProjection>;

/** 判定順序也是宣告順序（測試釘住，避免有人偷偷把 synced 往後搬）。 */
export const CHARACTER_SYNC_STATES: readonly CharacterSyncState[] = Object.keys(
  CHARACTER_SYNC_PROJECTION
) as CharacterSyncState[];

/** 一個同步成員在畫面上的樣子（名稱由呼叫端從裝置清單補；查不到就中性稱呼）。 */
export interface CharacterSyncMember {
  /** 顯示名稱。模擬 iPhone（fixture）的名稱自帶標籤，原樣顯示。 */
  name: string;
  /** 遠端裝置才算「另一頭」；這台電腦自己的角色視窗不算。 */
  remote: boolean;
  /** Runtime 回報的 presence 原始值；不認得就是不認得（不猜）。 */
  presence: string;
  /** 這個成員演得出角色嗎（只有遠端呈現角色的裝置可以）。 */
  canPresent: boolean;
}

/** 投影需要、但不屬於權威狀態的訊號（全部由呼叫端從真實回應算出來）。 */
export interface CharacterSyncSignals {
  /** Runtime 有沒有啟用角色同步（關閉時要誠實說關閉，不說成沒有裝置）。 */
  enabled: boolean;
  /** 連續讀不到權威狀態的次數（達 3 次＝無法恢復，契約 §7.5）。 */
  failedReads: number;
  /** 有裝置的授權被撤銷過，還沒重新確認。 */
  revokedDevice: boolean;
  /** 有手機連著這台電腦，但還不是角色同步的成員（要重新確認才會同步）。 */
  connectedButNotSynced: boolean;
  /**
   * 保存的角色同步紀錄讀不回來、已被隔離並重新開始（Runtime diagnostics 的
   * `storeNote` 不是 null）。一般模式要翻成人話，不得靜默——但它講的是「紀錄」，
   * 不是角色本身，所以不能當成緊急狀況。
   */
  storeReset: boolean;
}

export interface ProjectedCharacterSync extends CharacterSyncProjection {
  state: CharacterSyncState;
  /** 介面認得目前每一台裝置回報的狀態嗎；false＝不猜，只說尚未完成。 */
  known: boolean;
}

/** 這台電腦自己（桌面角色視窗）在成員清單裡的稱呼——不印任何識別碼。 */
export const CHARACTER_SYNC_LOCAL_NAME = "這台電腦";
/** 名字查不到的裝置：中性稱呼，絕不退回裝置識別碼。 */
export const CHARACTER_SYNC_UNNAMED_DEVICE = "一台裝置";

const CHARACTER_SYNC_PRESENCE = {
  online: "已連接",
  reconnecting: "重新連線中",
  offline: "離線",
} satisfies Record<string, string>;

/** presence 原始值 → 人話；不認得就誠實說不確定。 */
export function characterSyncPresenceLabel(presence: string): string {
  return Object.prototype.hasOwnProperty.call(CHARACTER_SYNC_PRESENCE, presence)
    ? CHARACTER_SYNC_PRESENCE[presence as keyof typeof CHARACTER_SYNC_PRESENCE]
    : "狀態不確定";
}

function isKnownPresence(presence: string): boolean {
  return Object.prototype.hasOwnProperty.call(CHARACTER_SYNC_PRESENCE, presence);
}

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

/** snapshot envelope → 權威狀態（讀不到就是 null，不用空物件冒充）。 */
function sessionState(snapshot: unknown): Record<string, unknown> | null {
  const payload = record(record(snapshot)?.["payload"]);
  return record(payload?.["state"]);
}

/**
 * 權威狀態的成員清單 → 畫面上的成員。
 * `names` 是「裝置識別碼 → 這台電腦上的顯示名稱」（來自已配對裝置清單）；
 * 查不到就用中性稱呼，一般模式永遠看不到識別碼本身。
 */
export function characterSyncMembers(
  snapshot: unknown,
  names: Record<string, string>
): CharacterSyncMember[] {
  const raw = sessionState(snapshot)?.["members"];
  if (!Array.isArray(raw)) return [];
  const members: CharacterSyncMember[] = [];
  for (const entry of raw) {
    const item = record(entry);
    if (!item) continue;
    const party = record(item["party"]);
    const kind = String(party?.["kind"] ?? "");
    const id = String(party?.["id"] ?? "");
    const remote = kind === "device";
    members.push({
      name: remote ? (names[id] ?? CHARACTER_SYNC_UNNAMED_DEVICE) : CHARACTER_SYNC_LOCAL_NAME,
      remote,
      presence: String(item["presence"] ?? ""),
      // 只有「遠端呈現角色」的成員演得出角色；只送輸入或只旁觀的不算。
      canPresent: item["role"] === "remote-renderer" || item["role"] === "host-renderer",
    });
  }
  return members;
}

/** 互動種類 → 人話（未知種類不猜，用最中性的說法）。 */
const CHARACTER_TOUCH_VERB: Record<string, string> = {
  tap: "摸了摸角色",
  pat: "輕拍了角色",
  stroke: "撫摸了角色",
  longpress: "按著角色不放",
};

/** 誰做的（`"<kind>:<id>"`）→ 人話稱呼；裝置查不到名字就中性稱呼。 */
function interactionActor(source: string, names: Record<string, string>): string {
  const split = source.indexOf(":");
  const kind = split >= 0 ? source.slice(0, split) : source;
  const id = split >= 0 ? source.slice(split + 1) : "";
  if (kind === "device") return names[id] ?? CHARACTER_SYNC_UNNAMED_DEVICE;
  if (kind === "human-surface" || kind === "renderer") return "你在這台電腦上";
  return "有人";
}

/** 最近一次互動的一句人話；沒有互動過就是 `null`（不編一個出來）。 */
export function characterSyncLastInteraction(
  snapshot: unknown,
  names: Record<string, string>
): string | null {
  const last = record(sessionState(snapshot)?.["lastInteraction"]);
  if (!last) return null;
  const name = String(last["name"] ?? "");
  const kind = String(last["kind"] ?? "");
  const who = interactionActor(String(last["source"] ?? ""), names);
  if (name === "character.interaction.dismiss") return `${who}請角色休息一下`;
  if (name !== "character.interaction.touch") return null;
  return `${who}${CHARACTER_TOUCH_VERB[kind] ?? "和角色互動了一下"}`;
}

/** 緊急停止中的固定安全句（角色與 adapter 都不能覆寫）；其餘狀態沒有這一句。 */
export const CHARACTER_SYNC_EMERGENCY_TEXT =
  "緊急停止中：角色已停止表演，解除前不會接受任何互動。";

export function characterSyncSafetyNote(snapshot: unknown): string | null {
  const truth = record(sessionState(snapshot)?.["truth"]);
  return truth?.["state"] === "emergency" ? CHARACTER_SYNC_EMERGENCY_TEXT : null;
}

/**
 * 角色同步的一般模式投影。
 *
 * 判定順序（先擋住「不能相信」的情況，再談成功）：
 * 關閉 → 連續讀不到 → 讀不到這一次 → 認不得的回報 → 紀錄曾損毀 → online →
 * reconnecting → offline → 需要重新確認 → 沒有裝置。
 *
 * 「紀錄曾損毀」排在 online 之前：那一刻技術上也許真的同步著，但綠色徽章讀起來
 * 是「一切正常」，會把「你的裝置得重新對齊一次」這件事蓋掉。它仍然排在「讀不到」
 * 之後——連現在的狀態都讀不到時，先講讀不到。緊急停止的固定安全句由呼叫端
 * （可信 host 介面）另外顯示，永遠壓過這一句。
 */
export function projectCharacterSession(
  snapshot: unknown,
  members: readonly CharacterSyncMember[],
  signals: CharacterSyncSignals
): ProjectedCharacterSync {
  const project = (state: CharacterSyncState, known = true): ProjectedCharacterSync => ({
    state,
    known,
    ...CHARACTER_SYNC_PROJECTION[state],
  });
  if (!signals.enabled) return project("disabled");
  if (signals.failedReads >= 3) return project("unrecoverable");
  if (sessionState(snapshot) === null) return project("syncing");

  const remote = members.filter((m) => m.remote);
  // 認不得的回報一律不猜：不會被算成 online，也不會被寫成 offline。
  if (remote.some((m) => !isKnownPresence(m.presence))) {
    return {
      ...project("syncing", false),
      detail: "有裝置回報了這台電腦不認得的狀態；在弄清楚之前都當成尚未完成，不會當成已同步。",
    };
  }
  if (signals.storeReset) return project("store-reset");
  const online = remote.filter((m) => m.presence === "online");
  if (online.length > 0) {
    return online.every((m) => m.canPresent)
      ? project("synced")
      : project("partial-capability");
  }
  if (remote.some((m) => m.presence === "reconnecting")) return project("reconnecting");
  if (remote.length > 0) return project("offline");
  if (signals.revokedDevice || signals.connectedButNotSynced) {
    return project("needs-reconfirmation");
  }
  return project("no-device");
}

/**
 * 權威狀態裡「已經是同步成員」的裝置識別碼。
 *
 * 只給程式比對用（例如判斷某台連線中的手機還沒重新確認），**不得**印到畫面上：
 * 一般模式看得到的永遠是裝置名稱。
 */
export function characterSyncMemberDeviceIds(snapshot: unknown): string[] {
  const raw = sessionState(snapshot)?.["members"];
  if (!Array.isArray(raw)) return [];
  const ids: string[] = [];
  for (const entry of raw) {
    const party = record(record(entry)?.["party"]);
    if (party?.["kind"] !== "device") continue;
    const id = String(party["id"] ?? "");
    if (id.length > 0) ids.push(id);
  }
  return ids;
}

/**
 * 連接頁手機卡上的那一行同步狀態（一台手機一句人話）。
 *
 * 讀不到權威狀態就說讀不到；手機連著卻不在成員名單裡是「尚未同步」而不是「已同步」。
 */
export function characterSyncDeviceLine(snapshot: unknown, deviceId: string): string {
  const state = sessionState(snapshot);
  if (state === null) return "角色同步：目前讀不到狀態";
  const raw = state["members"];
  const members = Array.isArray(raw) ? raw : [];
  for (const entry of members) {
    const item = record(entry);
    const party = record(item?.["party"]);
    if (party?.["kind"] !== "device" || String(party["id"] ?? "") !== deviceId) continue;
    const presence = String(item?.["presence"] ?? "");
    if (presence === "online") return "角色同步：已同步";
    return `角色同步：${characterSyncPresenceLabel(presence)}`;
  }
  return "角色同步：尚未同步（需要在手機上重新確認）";
}
