// 交代一件工作之後「到底怎麼了」的唯一投影（known limitation #24）。
//
// 後端（agents.rs `mailbox_send` ／ gateway.rs `gateway_deliver`）只給得出這幾
// 種事實，介面就只能說這幾件事：
//
// - 呼叫成功、訊息帶 `deliveredAt` 戳記 → 真的寫進 agent 了（已送達）。
// - 呼叫成功、沒有戳記 → 訊息進了信箱，但沒有人取走（尚未送達）。
// - `conflict:`（上一輪還在跑）→ 沒送到，agent 正在忙（排隊中）。
// - `conflict:` 但信箱已關 ／ `expired:` → 這個工作收掉了（Agent 不可用）。
// - `unavailable:`（子程序已結束／無回應）→ Agent 不可用。
// - `not found:` ／ `validation failed:` ／ `policy blocked:` ／緊急停止 → 傳送失敗。
// - 內部錯誤、連不上、認不得的錯誤 → 沒有證據，只能說結果不確定。
//
// 誠實階梯：**沒有送達戳記就不得說「已送達」**。訊息未送達≠任務失敗，也≠任務
// 完成；六態各自帶一句人話與一句誠實註記，任何一態都不宣稱工作已完成。
//
// 一般模式不外洩後端術語：後端的原文只放在 `detail`，由進階模式自行決定要不要
// 顯示；`message` 與 `honesty` 永遠是人話。

import type { BadgeKind } from "../statusProjection";

/** 六種送達結果（產品規格用語）。 */
export type DeliveryOutcome =
  | "delivered"
  | "mailbox"
  | "queued"
  | "agent-unavailable"
  | "send-failed"
  | "uncertain";

/** 六態的固定標籤。 */
export const DELIVERY_LABEL = {
  delivered: "已送達",
  mailbox: "尚未送達（已放進信箱）",
  queued: "排隊中",
  "agent-unavailable": "Agent 不可用",
  "send-failed": "傳送失敗",
  uncertain: "結果不確定",
} satisfies Record<DeliveryOutcome, string>;

/** 送達不是完成，所以沒有任何一態用成功綠（`ok`）。 */
const DELIVERY_BADGE = {
  delivered: "info",
  mailbox: "pending",
  queued: "pending",
  "agent-unavailable": "bad",
  "send-failed": "bad",
  uncertain: "warn",
} satisfies Record<DeliveryOutcome, BadgeKind>;

/** 這一次交代走到哪一步：`create`＝工作還沒建立；`send`＝工作已建立、在送內容。 */
export type DeliveryStage = "create" | "send";

export interface DeliveryInput {
  /** 預設 `send`（工作已經建立了）。 */
  stage?: DeliveryStage;
  /** 呼叫成功時後端回傳的信箱訊息；有帶這個欄位就代表後端收下了。 */
  sent?: unknown;
  /** 呼叫失敗時丟出的原因。 */
  error?: unknown;
  /** 對象的顯示名稱（Codex／Claude Code…）；沒有就用「工作助手」。 */
  agentName?: string;
  /** 這次交代的工作名稱；沒有就用「這次的交代」。 */
  taskLabel?: string;
}

export interface DeliveryStatus {
  outcome: DeliveryOutcome;
  /** 六態固定標籤。 */
  label: string;
  /** 一句人話（一般模式直接顯示）。 */
  message: string;
  /** 誠實註記：這個結果**不能**推論出什麼、接下來能做什麼。 */
  honesty: string;
  badge: BadgeKind;
  /** 後端真的蓋了送達戳記——只有這一態可以說「已送達」。 */
  delivered: boolean;
  /** 後端收下了內容（進了信箱）：可以清空輸入框，不必重打。 */
  accepted: boolean;
  /** 要用錯誤樣式呈現（沒送到而且不會自己好）。 */
  problem: boolean;
  /** 後端原文，只給進階模式；一般模式不顯示。 */
  detail?: string;
}

/**
 * 送出的訊息是否**真的**送到 agent 了。
 *
 * 誠實階梯（dispatched ≠ acknowledged）：後端只有在訊息真的寫進 agent 子程序時
 * 才蓋 `deliveredAt`。輪詢型 agent（尚未來取）、子程序已經不再接收的情況都會回
 * 一則沒有戳記的訊息——那是「已放進信箱」，不是「已送達」。
 */
export function deliveredToAgent(message: unknown): boolean {
  if (!message || typeof message !== "object") return false;
  const at = (message as { deliveredAt?: unknown }).deliveredAt;
  return typeof at === "string" && at.length > 0;
}

/** 後端錯誤的種類（對應 `DomainError`；`busy`／`inactive` 是 conflict 的兩支）。 */
export type BackendErrorKind =
  | "none"
  | "not-found"
  | "busy"
  | "inactive"
  | "validation"
  | "policy"
  | "expired"
  | "unavailable"
  | "internal"
  | "unrecognized";

/** `Error: ` 包裝與 HTTP 模式的狀態碼前綴都剝掉，只留 `DomainError` 的 Display。 */
function normalizeErrorText(error: unknown): string {
  let text = typeof error === "string" ? error : String(error ?? "");
  text = text.replace(/^(?:[A-Za-z]*Error:\s*)+/, "");
  text = text.replace(/^\d{3}:\s*/, "");
  return text.trim();
}

/**
 * 後端錯誤字串 → 種類。兩種傳輸的字串形狀不同（Tauri 直接是 `DomainError` 的
 * Display；HTTP 前面多一個狀態碼），但前綴一樣，所以只認前綴，不猜訊息內容。
 */
export function backendErrorKind(error: unknown): BackendErrorKind {
  if (error === undefined || error === null) return "none";
  const text = normalizeErrorText(error);
  const lower = text.toLowerCase();
  if (lower.startsWith("not found:")) return "not-found";
  if (lower.startsWith("conflict:")) {
    // 信箱已關＝這個工作結束了；其餘 conflict 是「上一輪還在跑」。
    return lower.includes("mailbox closed") ? "inactive" : "busy";
  }
  if (lower.startsWith("session inactive:")) return "inactive";
  if (lower.startsWith("validation failed:")) return "validation";
  if (
    lower.startsWith("policy blocked:") ||
    lower.startsWith("approval required:") ||
    lower.startsWith("consent required:") ||
    lower.startsWith("emergency stop")
  ) {
    return "policy";
  }
  if (lower.startsWith("expired:")) return "expired";
  if (lower.startsWith("unavailable:")) return "unavailable";
  if (lower.startsWith("internal error:") || lower.startsWith("storage error:")) return "internal";
  return "unrecognized";
}

const ERROR_OUTCOME = {
  "not-found": "send-failed",
  busy: "queued",
  inactive: "agent-unavailable",
  validation: "send-failed",
  policy: "send-failed",
  expired: "agent-unavailable",
  unavailable: "agent-unavailable",
  // 後端自己壞了／連不上：請求可能已經被處理，也可能沒有——不猜。
  internal: "uncertain",
  unrecognized: "uncertain",
  none: "uncertain",
} satisfies Record<BackendErrorKind, DeliveryOutcome>;

/** 產品名多半是英數字，夾在中文裡要留空白才讀得順；純中文名稱不加。 */
function spaced(name: string): string {
  return /[A-Za-z0-9]/.test(name) ? ` ${name} ` : name;
}

/** 收掉組句留下的多餘空白。 */
function line(text: string): string {
  return text.replace(/\s+/g, " ").trim();
}

function messageFor(
  outcome: DeliveryOutcome,
  stage: DeliveryStage,
  subject: string,
  target: string
): string {
  const who = spaced(target);
  if (stage === "create") {
    switch (outcome) {
      case "queued":
        return line(`${who}正在忙，${subject}還沒有開始。`);
      case "agent-unavailable":
        return line(`${who}現在不能接工作，${subject}沒有開始。`);
      case "send-failed":
        return line(`${subject}沒能開始。`);
      case "uncertain":
        return line(`不確定${subject}有沒有開始。`);
      default:
        break;
    }
  }
  switch (outcome) {
    case "delivered":
      return line(`${subject}已送到${who}手上，尚未完成；做完後會請你檢查結果。`);
    case "mailbox":
      return line(`${subject}已放進${who}的信箱，還沒送到它手上；它來取走之後才會開始。`);
    case "queued":
      return line(`${who}上一輪還在跑，${subject}排在後面，還沒送到。`);
    case "agent-unavailable":
      return line(`${who}現在不能接工作，${subject}沒有送出去。`);
    case "send-failed":
      return line(`${subject}沒能送出去。`);
    case "uncertain":
      return line(`不確定${subject}有沒有送到${who}手上。`);
  }
}

function honestyFor(outcome: DeliveryOutcome, stage: DeliveryStage): string {
  // 工作已經建立的情況才可以叫人「回工作卡片再送一次」。
  const retry = stage === "send" ? "可以在下面的工作卡片再送一次。" : "可以重新交代一次。";
  switch (outcome) {
    case "delivered":
      return "送到不等於做完，也不等於做對；結果仍然要你檢查。";
    case "mailbox":
      return `沒有送達回條就不算送到；一直沒有動靜，${retry}`;
    case "queued":
      return "還沒送到，也不會自己接上；等它做完，或先按「暫停／中斷」，再送一次。";
    case "agent-unavailable":
      return `沒有送到，也沒有開始。${retry}`;
    case "send-failed":
      return `沒有送到，也沒有開始。${retry}`;
    case "uncertain":
      return "系統沒有拿到回覆，既不能說送到、也不能說沒送到；先看工作有沒有動靜，再決定要不要重送。";
  }
}

/**
 * 建立／送出的實際結果 → 六態之一。
 *
 * 判斷順序就是證據的強弱：有錯誤先看錯誤，其次看後端有沒有收下訊息，
 * 什麼證據都沒有就是「結果不確定」——不猜、不預設成功。
 */
export function classifyDelivery(input: DeliveryInput): DeliveryStatus {
  const stage: DeliveryStage = input.stage ?? "send";
  const hasError = input.error !== undefined && input.error !== null;
  const outcome: DeliveryOutcome = hasError
    ? ERROR_OUTCOME[backendErrorKind(input.error)]
    : "sent" in input
      ? deliveredToAgent(input.sent)
        ? "delivered"
        : // 呼叫成功＝後端已經把訊息放進信箱（送達與否是另一回事）。
          "mailbox"
      : "uncertain";
  const label = input.taskLabel?.trim();
  const subject = label ? `「${label}」` : "這次的交代";
  const target = input.agentName?.trim() || "工作助手";
  return {
    outcome,
    label: DELIVERY_LABEL[outcome],
    message: messageFor(outcome, stage, subject, target),
    honesty: honestyFor(outcome, stage),
    badge: DELIVERY_BADGE[outcome],
    delivered: outcome === "delivered",
    accepted: outcome === "delivered" || outcome === "mailbox",
    problem: outcome === "agent-unavailable" || outcome === "send-failed" || outcome === "uncertain",
    // 原文一字不改（含傳輸層的狀態碼），進階模式要拿去對後端日誌。
    detail: hasError ? String(input.error) : undefined,
  };
}

/** 只有一行位置可用時的通知文字（人話＋誠實註記；進階模式才附後端原文）。 */
export function deliveryNoticeText(
  status: DeliveryStatus,
  opts: { advanced?: boolean } = {}
): string {
  const base = `${status.message} ${status.honesty}`.trim();
  return opts.advanced && status.detail ? `${base}（${status.detail}）` : base;
}
