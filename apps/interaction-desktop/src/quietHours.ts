// Canonical quiet-hours builder（ia-settings-012）。
//
// 後端（crates/interaction-policy/src/lib.rs 的 Governor::authorize）把
// `silencedChannels: []` 解讀成「沒有明確表態」，於是套用內建預設清單
// `DEFAULT_QUIET_SILENCED`，而那份預設清單含 `desktop-pet`——也就是說，
// 只要任何一個呼叫點偷懶送出空陣列，桌面角色就會在安靜時段被誤靜音，
// 即使呈現層本身完全支援「安靜待著、不出聲不通知」這種 L0 降級表現。
//
// 首次設定精靈（Onboarding.tsx）與角色頁（CompanionPage.tsx）都要建立
// 安靜時段，兩邊必須送出同一份『刻意排除桌面角色』的明確清單，不能各自
// 硬寫字面陣列各憑印象維護。這裡就是那個唯一共用的建構點。

/** 安靜時段預設要靜音的通道；刻意不含 `desktop-pet`——桌面角色在安靜時段
 * 仍應以 L0（安靜待著、不出聲不通知）呈現，而不是被完全消音。 */
export const QUIET_SILENCED_CHANNELS = ["audio", "haptic", "notification", "light"];

export interface QuietHoursWindow {
  start: string;
  end: string;
  silencedChannels: string[];
}

/** 建立一筆安靜時段 patch。一律送出明確的 `silencedChannels` 清單——
 * 絕不送空陣列，空陣列會被後端解讀成含 `desktop-pet` 的內建預設。 */
export function buildQuietHoursPatch(start: string, end: string): QuietHoursWindow {
  return { start, end, silencedChannels: [...QUIET_SILENCED_CHANNELS] };
}
