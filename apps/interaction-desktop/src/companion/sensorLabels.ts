// 角色視窗的感測標籤（對抗審查 ia-settings-005）。
//
// 感測不靜默，但「不靜默」不等於「把 runtime 的原始 id 印給使用者看」：
// 全 App 只有一份 kind → 人話的對照（statusProjection 的 `sensorKindLabel`），
// tray／首頁／host overlay 都吃它，角色視窗也必須吃同一份，否則同一台機器上
// 會出現「🎙 麥克風使用中」（overlay）與「使用中：iphone.mic-level」（角色視窗）
// 這種自相矛盾、又外洩原始識別的文案。
//
// 這裡只做投影，不做判斷升級：認不得的種類一律「其他感測器」——使用者仍看得到
// 「有東西在感測」這個事實，但畫面上永遠不會出現原始字串。

import { sensorKindLabel } from "../statusProjection";

/** 角色視窗要的最小形狀（runtime `status.activeSensors` 的一筆）。 */
export interface SensorKindLike {
  kind: string;
}

/**
 * 角色視窗的感測標籤。
 *
 * - 任何一個來源投影成「麥克風」（本機 `microphone` 與手機 `iphone.mic-level` 都是）
 *   → 麥克風專屬文案，與 host overlay／控制中心一致。
 * - 其餘：把種類投影成人話後去重列出（兩台手機同時上報同一種類不會說兩次）。
 * - 沒有感測器 → `null`（不顯示標籤）。
 */
export function companionSensorLabel(sensors: readonly SensorKindLike[]): string | null {
  if (sensors.length === 0) return null;
  const labels = sensors.map((s) => sensorKindLabel(s.kind));
  if (labels.some((l) => l === "麥克風")) return "🎙 正在使用麥克風";
  const unique = [...new Set(labels)];
  return `使用中：${unique.join("、")}`;
}
