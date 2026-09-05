// 其他會在同一批畫面出現、同樣不該外洩原始值的識別字：Provider 生命週期、
// 能力卡種類、agent id、知識來由、動作意圖、收件匣裝置名，以及感測種類與
// 「停止所有感測」的誠實回報。
//
// 對外一律經由 `../statusProjection.ts` 這個匯總檔（既有 import 路徑不變）。

import { ownKey, type BadgeKind } from "./workState";

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
