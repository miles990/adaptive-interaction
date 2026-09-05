// 陪伴預設的兩段寫入（M4）：把「套用一個檔位」當成一筆可以被中斷、被恢復的交易。
//
// 為什麼需要這一層：套用檔位要寫兩個不同的地方——
//   1. 桌面偏好（`companionExpressiveness`＋`companionDoNotDisturb`；Tauri host 的檔案）
//   2. 後端主動說話的模式（`mode`；由 Rust 確定性強制）
// 兩段之間可能斷電、可能回應遺失、可能第二段被後端拒絕。以前的寫法只有「按鈕忙碌」
// 這一層保護：第二段失敗時，畫面只會退回「自訂」，使用者看不出**哪一段**沒生效、
// 也沒有辦法只補送缺的那一段。
//
// 這個模組把交易本身寫成純函式：
//   - `beginPresetOp` 產生計畫（含要與第一段**原子寫入**的 recovery marker）；
//   - `readPendingPresetOp` 把 host 回來的 marker 驗過（有界、只認得的檔位）才用；
//   - `shouldResumePendingOp` 決定重開之後還能不能安全補送——只有 marker 鎖定的
//     偏好欄位**仍等於**目前值才行；使用者事後改過就不補送、也不覆蓋；
//   - `projectPresetStatus` 把（有效值、marker、忙碌、恢復中、讀回失敗）投影成五種
//     使用者看得懂的狀態。五種裡沒有一種會在不確定時說「已完成」（誠實階梯）。
//
// 純函式模組：不 import api／desktop／React，也不認得任何角色。

import { presetDefinition, type CompanionPresetChoice, type CompanionPresetInputs } from "./presets";

/** marker 的 `opId` 上限（與 src-tauri 的驗證一致）。 */
export const PRESET_OP_ID_MAX_CHARS = 64;
/** marker 裡第二段 `mode` 的上限（與 src-tauri 的驗證一致）。 */
export const PRESET_OP_MODE_MAX_CHARS = 32;

/**
 * 存進桌面偏好的恢復標記：只帶「補送第二段」需要的東西。
 * 它**不是**新的設定層——沒有它，有效值也還是那三個既有欄位。
 */
export interface PresetOpMarker {
  opId: string;
  presetId: string;
  /** 第二段還沒確認送到的 patch（只有 mode）。 */
  proactivePatch: { mode: string };
  issuedAtMs: number;
}

/** 一次套用的完整計畫（第一段要寫的偏好、第二段要送的 patch，以及 marker 的內容）。 */
export interface PresetOpPlan {
  opId: string;
  presetId: string;
  prefs: { companionExpressiveness: string; companionDoNotDisturb: boolean };
  proactive: { mode: string };
  issuedAtMs: number;
}

/**
 * 使用者看到的狀態：
 *   - `applied`：三個欄位都等於某個檔位，而且沒有未完成的交易。
 *   - `partially-applied`：第一段寫進去了、第二段沒確認送到（要能補送）。
 *   - `recovering`：正在補送上一次沒完成的第二段。
 *   - `custom-effective`：不吻合任何檔位（逐項顯示有效值）。
 *   - `unverified`：讀不回有效值——不高亮任何檔位，也不假裝知道。
 */
export type CompanionPresetStatus =
  | "applied"
  | "partially-applied"
  | "recovering"
  | "custom-effective"
  | "unverified";

/** 有限、非負的時間戳；壞掉的輸入退回 0（不產生無界字串）。 */
function safeMillis(value: number): number {
  return Number.isFinite(value) && value >= 0 ? Math.floor(value) : 0;
}

/**
 * 開始一次套用。未知的檔位回 `null`（不猜）。
 *
 * `opId` 只用來分辨「這是我這次的交易」與「上一次留下來的」，所以由檔位＋發起時間
 * 決定就夠了：純函式、可重現、有界。
 */
export function beginPresetOp(id: string, nowMs: number): PresetOpPlan | null {
  const def = presetDefinition(id);
  if (!def) return null;
  const issuedAtMs = safeMillis(nowMs);
  return {
    opId: `${def.id}-${issuedAtMs.toString(36)}`,
    presetId: def.id,
    prefs: {
      companionExpressiveness: def.state.expressiveness,
      companionDoNotDisturb: def.state.doNotDisturb,
    },
    proactive: { mode: def.state.proactiveMode },
    issuedAtMs,
  };
}

/** 計畫 → 要與第一段一起原子寫入的 marker。 */
export function markerOf(plan: PresetOpPlan): PresetOpMarker {
  return {
    opId: plan.opId,
    presetId: plan.presetId,
    proactivePatch: { mode: plan.proactive.mode },
    issuedAtMs: plan.issuedAtMs,
  };
}

function boundedString(value: unknown, max: number): string | null {
  if (typeof value !== "string") return null;
  if (value.length < 1 || value.length > max) return null;
  return value;
}

/**
 * host 回來的 marker 一律驗過才用：偏好檔是使用者可以手改的檔案，
 * 壞掉的內容當作「沒有 marker」（呼叫端會把它清掉），不猜、也不半信半疑地補送。
 */
export function readPendingPresetOp(value: unknown): PresetOpMarker | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return null;
  const raw = value as Record<string, unknown>;
  const opId = boundedString(raw.opId, PRESET_OP_ID_MAX_CHARS);
  if (!opId) return null;
  const presetId = typeof raw.presetId === "string" ? presetDefinition(raw.presetId)?.id : undefined;
  if (!presetId) return null;
  const patch = raw.proactivePatch;
  if (typeof patch !== "object" || patch === null || Array.isArray(patch)) return null;
  const patchKeys = Object.keys(patch as Record<string, unknown>);
  if (patchKeys.length !== 1 || patchKeys[0] !== "mode") return null;
  const mode = boundedString((patch as Record<string, unknown>).mode, PRESET_OP_MODE_MAX_CHARS);
  if (!mode) return null;
  const issuedAtMs = raw.issuedAtMs;
  if (typeof issuedAtMs !== "number" || !Number.isFinite(issuedAtMs) || issuedAtMs < 0) return null;
  return { opId, presetId, proactivePatch: { mode }, issuedAtMs };
}

/**
 * 重開之後還能不能自動補送第二段？
 *
 * 只有一種情況可以：marker 鎖定的兩個偏好欄位**仍然等於**目前的值——也就是第一段
 * 確實寫進去了、而且使用者事後沒有再改過。使用者改過（或讀不到偏好）就不補送：
 * 補送會用一份過時的意圖覆蓋掉他剛剛親手選的設定。
 *
 * marker 與這一版的檔位定義不一致（設定檔被手改、或版本換過）時同樣不補送：
 * 我們只補送這一版說得出口的組合。
 */
export function shouldResumePendingOp(marker: unknown, currentInputs: CompanionPresetInputs): boolean {
  const pending = readPendingPresetOp(marker);
  if (!pending) return false;
  const def = presetDefinition(pending.presetId);
  if (!def) return false;
  if (pending.proactivePatch.mode !== def.state.proactiveMode) return false;
  return (
    currentInputs.expressiveness === def.state.expressiveness &&
    currentInputs.doNotDisturb === def.state.doNotDisturb
  );
}

/**
 * 投影成使用者看得懂的狀態。
 *
 * 順序就是誠實階梯：
 *   1. 讀不回有效值 → `unverified`（最高優先：不知道就不要顯示任何結論）。
 *   2. 正在補送 → `recovering`。
 *   3. 交易還在飛（`busy`）→ **先不下判決**，只說現在的有效值是什麼：第一段已經
 *      寫進去、第二段還在路上時說「半套用」是誣賴自己。
 *   4. 有 marker：有效值已經等於目標＝`applied`（回應遺失但事情做到了）；
 *      還不是目標＝`partially-applied`（要能補送）。
 *   5. 其餘照有效值：吻合檔位＝`applied`，不吻合＝`custom-effective`。
 */
export function projectPresetStatus(input: {
  presetChoice: CompanionPresetChoice;
  pendingOp: PresetOpMarker | null;
  busy: boolean;
  recovering: boolean;
  readbackFailed: boolean;
}): CompanionPresetStatus {
  if (input.readbackFailed) return "unverified";
  if (input.recovering) return "recovering";
  if (input.pendingOp && !input.busy) {
    return input.presetChoice === input.pendingOp.presetId ? "applied" : "partially-applied";
  }
  return input.presetChoice === "custom" ? "custom-effective" : "applied";
}
