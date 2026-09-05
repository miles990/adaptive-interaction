// 窄視窗（<700px）底部導覽。路由表（NARROW_PRIMARY／NARROW_MORE_ITEMS／
// ADVANCED_NAV）與折疊規則（navAnchorFor／moreSheetCurrent）都在 `../routing`，
// 與寬視窗側邊欄共用同一份真相。

import React from "react";
import { Icon } from "../icons";
import { Dialog } from "./Dialog";
import {
  ADVANCED_NAV,
  moreSheetCurrent,
  NARROW_MORE_ITEMS,
  NARROW_PRIMARY,
  navAnchorFor,
  type NavEntry,
  type Tab,
} from "../routing";

/** 窄視窗（<700px）底部導覽：4 個主要入口＋「更多」選單。
 *  所有頁面都可抵達、鍵盤可操作、永遠有文字標籤（不只靠 Icon）。 */
export function NarrowNav({
  tab,
  nav,
  onNavigate,
  advanced,
  statusBadge,
}: {
  /** 未折疊的目前路由。一級入口的高亮走 navAnchorFor（相容 id 也會亮對），
   *  「更多」選單的細項則要用原始路由比對，否則永遠沒有細項會亮。 */
  tab: Tab;
  /** 執行期一級導覽（第二項已換成目前角色）。 */
  nav: NavEntry[];
  onNavigate: (tab: Tab) => void;
  advanced: boolean;
  statusBadge: React.ReactNode;
}) {
  const [moreOpen, setMoreOpen] = React.useState(false);
  const primary = nav.filter((t) => NARROW_PRIMARY.includes(t.id));
  const secondary = NARROW_MORE_ITEMS;
  const anchor = navAnchorFor(tab);
  const current = moreSheetCurrent(tab);
  const moreActive = !NARROW_PRIMARY.includes(anchor);
  return (
    <>
      <nav className="bottom-nav" aria-label="主要導覽（窄視窗）">
        {primary.map((t) => (
          <button
            key={t.id}
            className={anchor === t.id ? "bottom-nav-item active" : "bottom-nav-item"}
            onClick={() => onNavigate(t.id)}
            aria-current={anchor === t.id ? "page" : undefined}
          >
            <Icon name={t.icon} size={18} />
            <span>{t.label}</span>
          </button>
        ))}
        <button
          className={moreActive ? "bottom-nav-item active" : "bottom-nav-item"}
          onClick={() => setMoreOpen(true)}
          aria-haspopup="dialog"
          aria-expanded={moreOpen}
        >
          <Icon name="menu" size={18} />
          <span>更多</span>
        </button>
      </nav>
      {moreOpen && (
        <Dialog title="更多功能" onClose={() => setMoreOpen(false)}>
          <div className="more-sheet">
            <div className="more-status">{statusBadge}</div>
            {secondary.map((t) => (
              <button
                key={t.id}
                className={current === t.id ? "more-item active" : "more-item"}
                aria-current={current === t.id ? "page" : undefined}
                onClick={() => {
                  onNavigate(t.id);
                  setMoreOpen(false);
                }}
              >
                <Icon name={t.icon} size={16} /> <span>{t.label}</span>
              </button>
            ))}
            {advanced && (
              <>
                <div className="nav-group-label">
                  <Icon name="code2" size={13} /> 進階
                </div>
                {ADVANCED_NAV.map((t) => (
                  <button
                    key={t.id}
                    className={current === t.id ? "more-item active" : "more-item"}
                    aria-current={current === t.id ? "page" : undefined}
                    onClick={() => {
                      onNavigate(t.id);
                      setMoreOpen(false);
                    }}
                  >
                    <span>{t.label}</span>
                  </button>
                ))}
              </>
            )}
          </div>
        </Dialog>
      )}
    </>
  );
}
