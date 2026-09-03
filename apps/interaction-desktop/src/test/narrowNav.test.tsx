// 窄視窗底部導覽與「更多」選單（regression: ia-settings-008）。
//
// 缺陷：Shell 過去傳給 NarrowNav 的是 navAnchorFor 折疊後的 anchor，
// 而選單細項用 `tab === t.id` 比對未折疊的 id（memory／activity／settings…），
// 於是五個細項永遠不會 active、也沒有 aria-current，只有 adv-* 會亮
//（同一張選單自相矛盾）。修法是傳未折疊的路由，一級入口才用 anchor 比對。

import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { NarrowNav, NARROW_MORE_ITEMS, moreSheetCurrent, simpleNavFor } from "../App";

afterEach(() => {
  vi.restoreAllMocks();
});

function renderNav(tab: string, advanced = false) {
  const onNavigate = vi.fn();
  const utils = render(
    <NarrowNav
      tab={tab}
      nav={simpleNavFor({ name: "角色", icon: "sparkles" })}
      onNavigate={onNavigate}
      advanced={advanced}
      statusBadge={null}
    />
  );
  return { ...utils, onNavigate };
}

async function openSheet() {
  const bottomNav = screen.getByRole("navigation", { name: "主要導覽（窄視窗）" });
  await userEvent.click(within(bottomNav).getByRole("button", { name: "更多" }));
  return screen.getByRole("dialog", { name: "更多功能" });
}

describe("窄視窗「更多」選單：高亮目前所在的細項", () => {
  for (const current of ["memory", "activity", "settings", "backup", "advanced-features"]) {
    it(`目前在 ${current} 時，只有該細項是 active 且帶 aria-current`, async () => {
      renderNav(current);
      const sheet = await openSheet();
      for (const item of NARROW_MORE_ITEMS) {
        const button = within(sheet).getByRole("button", { name: item.label });
        if (item.id === current) {
          expect(button.className, `${item.label} 應該高亮`).toContain("active");
          expect(button).toHaveAttribute("aria-current", "page");
        } else {
          expect(button.className, `${item.label} 不該高亮`).not.toContain("active");
          expect(button).not.toHaveAttribute("aria-current");
        }
      }
      // 底部的「更多」本身仍然是 active（細項都折疊在它底下）。
      const bottomNav = screen.getByRole("navigation", { name: "主要導覽（窄視窗）" });
      expect(
        within(bottomNav).getByRole("button", { name: /更多/ }).className
      ).toContain("active");
    });
  }

  it("裸的 more 路由對應預設分頁「記憶與資料」（與寬視窗 MorePage 一致）", async () => {
    expect(moreSheetCurrent("more")).toBe("memory");
    expect(moreSheetCurrent("settings")).toBe("settings");
    renderNav("more");
    const sheet = await openSheet();
    expect(within(sheet).getByRole("button", { name: "記憶與資料" })).toHaveAttribute(
      "aria-current",
      "page"
    );
  });

  it("進階細項也高亮（同一張選單的兩半必須一致）", async () => {
    renderNav("adv-recipes", true);
    const sheet = await openSheet();
    expect(within(sheet).getByRole("button", { name: "配方 YAML" })).toHaveAttribute(
      "aria-current",
      "page"
    );
    expect(
      within(sheet).getByRole("button", { name: "記憶與資料" })
    ).not.toHaveAttribute("aria-current");
  });

  it("一級入口用折疊後的 anchor 高亮：相容 id（ai／safety）也亮對", () => {
    renderNav("ai");
    const bottomNav = screen.getByRole("navigation", { name: "主要導覽（窄視窗）" });
    expect(within(bottomNav).getByRole("button", { name: /工作/ })).toHaveAttribute(
      "aria-current",
      "page"
    );
    expect(within(bottomNav).getByRole("button", { name: /更多/ })).not.toHaveAttribute(
      "aria-current"
    );
  });

  it("在一級頁時「更多」不高亮", () => {
    renderNav("home");
    const bottomNav = screen.getByRole("navigation", { name: "主要導覽（窄視窗）" });
    expect(within(bottomNav).getByRole("button", { name: /更多/ }).className).not.toContain(
      "active"
    );
  });
});
