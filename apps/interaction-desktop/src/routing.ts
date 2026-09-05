// 控制中心的路由／導覽：有哪些入口、舊 id 折疊到哪、標題是什麼。
//
// 這裡只有**純資料與純函式**（零 React、零 API），所以任何入口
// （側邊欄、窄視窗底部導覽、⌘K 全域搜尋、通知中心「前往」、狀態列深連結）
// 都能共用同一份對照，深連結測試也能直接對它斷言。
//
// 導覽狀態（目前路由與重新掛載）在 `useNavigation.ts`；頁面內容的分派在
// `App.tsx` 的 `PageBody`。

import { characterNameFallback, NEUTRAL_CHARACTER_ICON } from "./characterName";

export type Tab = string;

export interface NavEntry {
  id: Tab;
  label: string;
  icon: string;
}

// v0.5 資訊架構：5 個一級入口（現在／角色／工作／連接與權限／更多）。
// 第二項的 label 與 icon 是目前角色（useCharacterName：prefs 名字＞manifest
// displayName＞「角色」），由 simpleNavFor 在執行期代入；這份靜態表只放中立值。
// 舊 tab id 全部保留可用（tray 深連結、Inbox route、書籤），由
// navAnchorFor 折疊到新家；內容走 PageBody 的相容路由。
export const SIMPLE_NAV: NavEntry[] = [
  { id: "home", label: "現在", icon: "house" },
  { id: "companion", label: characterNameFallback, icon: NEUTRAL_CHARACTER_ICON },
  { id: "work", label: "工作", icon: "bot" },
  { id: "connect", label: "連接與權限", icon: "plug" },
  { id: "more", label: "更多", icon: "menu" },
];

/** 一級導覽的執行期版本：第二項換成目前角色的名字與 icon（其餘不變、仍恰 5 項）。 */
export function simpleNavFor(character: { name: string; icon: string }): NavEntry[] {
  return SIMPLE_NAV.map((t) =>
    t.id === "companion" ? { ...t, label: character.name, icon: character.icon } : t
  );
}

/** 進階模式才出現的原始技術頁（側邊欄與窄視窗「更多」選單共用同一份）。 */
export const ADVANCED_NAV: { id: Tab; label: string }[] = [
  { id: "adv-overview", label: "總覽（原始）" },
  { id: "adv-receptors", label: "受器" },
  { id: "adv-actuators", label: "動器" },
  { id: "adv-tools", label: "工具" },
  { id: "adv-recipes", label: "配方 YAML" },
  { id: "adv-policy", label: "政策／同意" },
  { id: "adv-timeline", label: "時間軸" },
  { id: "adv-providers", label: "Provider Registry" },
  { id: "adv-knowledge", label: "Knowledge Graph" },
];

// 相容 tab id → 新一級入口的折疊表。key 是舊 id（tray 深連結、
// Runtime Inbox route、舊書籤、GlobalSearch），value 是導覽高亮／標題的新家。
export const LEGACY_ANCHORS: Record<string, string> = {
  ai: "work",
  automations: "work",
  capabilities: "connect",
  senses: "connect",
  responses: "connect",
  toolops: "connect",
  safety: "connect",
  memory: "more",
  activity: "more",
  settings: "more",
  // v0.5 一般模式「更多」的新分頁：備份與還原／進階模式。
  backup: "more",
  // 相容保留：角色與整合管理不再是「更多」的分頁按鈕，但舊書籤／深連結仍要到得了。
  manage: "more",
  "advanced-features": "more",
};

/** 導覽高亮／標題所對應的 nav id（相容 tab 折疊到新 5 入口）。 */
export function navAnchorFor(tab: string): string {
  return LEGACY_ANCHORS[tab] ?? tab;
}

/** topbar 標題：相容 tab 也必須有標題，不得渲染空字串。
 *  角色頁的標題是目前角色的名字（傳入 characterName）；沒傳就是中立的「角色」。 */
export function titleFor(tab: string, characterName?: string): string {
  const anchor = navAnchorFor(tab);
  if (anchor === "companion" && characterName) return characterName;
  return (
    SIMPLE_NAV.find((t) => t.id === anchor)?.label ??
    ADVANCED_NAV.find((t) => t.id === anchor)?.label ??
    "未知頁面"
  );
}

/** 窄視窗（<700px）底部導覽列直接放的 4 個一級入口；其餘全部收進「更多」選單
 *  （所有頁面仍都可抵達，見 components/NarrowNav.tsx）。 */
export const NARROW_PRIMARY: readonly string[] = ["home", "companion", "work", "connect"];

/** 窄視窗「更多」選單的細項（寬視窗時這些是 MorePage 的分頁）。
 *  與 MORE_TABS 同一組 id／文案；`manage` 是隱藏的相容路由，不列在這裡。 */
export const NARROW_MORE_ITEMS: NavEntry[] = [
  { id: "memory", label: "記憶與資料", icon: "book-open" },
  { id: "activity", label: "活動紀錄", icon: "history" },
  { id: "settings", label: "外觀與語言", icon: "settings" },
  { id: "backup", label: "備份與還原", icon: "cloud-download" },
  { id: "advanced-features", label: "進階模式", icon: "code2" },
];

/** 「更多」選單裡目前所在的細項 id。傳進來的是**未折疊**的路由（settings／memory…）；
 *  裸的 `more` 對應 PageBody 的預設分頁（記憶與資料），與寬視窗 MorePage 的高亮一致。 */
export function moreSheetCurrent(tab: Tab): Tab {
  return tab === "more" ? "memory" : tab;
}
