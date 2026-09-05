// 角色生命週期（Character Presentation Protocol §7 `AdapterLifecycleState`；
// TS 鏡射見 `character/protocol.ts`，鏡射 Rust `interaction-character::lifecycle`）：
// 角色頁把 `instance.lifecycle`＋presence 轉成一句人話徽章的唯一投影入口。
//
// 對外一律經由 `../statusProjection.ts` 這個匯總檔（既有 import 路徑不變）。

import type { AdapterLifecycleState } from "../character/protocol";
import type { BadgeKind } from "./workState";

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
