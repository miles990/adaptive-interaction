// 統一收件匣（activity.rs `activity_inbox`）：除了工作狀態之外還會出現
// 知識候選、動作收據狀態（ActionStatus）與安全事件型別。
//
// 對外一律經由 `../statusProjection.ts` 這個匯總檔（既有 import 路徑不變）。

import {
  isWorkState,
  ownKey,
  projectWorkState,
  UNRECOGNIZED,
  WORK_STATES,
  type ProjectedStatus,
  type Projection,
  type WorkState,
} from "./workState";

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
