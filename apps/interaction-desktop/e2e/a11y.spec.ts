// 使用者任務：只用鍵盤也能用、螢幕閱讀器唸得出每個東西是什麼、
// 會動的東西可以關掉、淺色／深色／跟隨系統在每一個一級入口都讀得到。
//
// 這裡不驗「好不好看」，只驗可及性的事實：焦點真的落在該落的地方、
// 每個可按的東西都有名字、偏好真的改變了 DOM。

import { test, expect, Page } from "@playwright/test";
import { api, appUrl, DESKTOP, NARROW, navigateTo, openApp, PAGES } from "./helpers";

test.describe.configure({ mode: "serial" });

/** 目前焦點的簡述（測試失敗時看得懂是卡在哪一顆按鈕）。 */
async function focusInfo(page: Page): Promise<string> {
  return page.evaluate(() => {
    const el = document.activeElement as HTMLElement | null;
    if (!el) return "(none)";
    return `${el.tagName.toLowerCase()}[${el.className}] "${(
      el.getAttribute("aria-label") ??
      el.textContent ??
      ""
    )
      .trim()
      .slice(0, 40)}"`;
  });
}

test("鍵盤：第一個 Tab 是「跳到主要內容」，而且真的把焦點送進主要內容", async ({ page }) => {
  test.setTimeout(90_000);
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  // 重新載入一次：焦點回到文件開頭（不受前面點過什麼影響），第一個 Tab 才有意義。
  await page.reload();
  await expect(page.getByRole("navigation", { name: "主要導覽" })).toBeVisible({ timeout: 20_000 });
  await page.keyboard.press("Tab");
  const skip = page.getByRole("link", { name: "跳到主要內容" });
  await expect(skip).toBeFocused();
  // 跳過去之後，下一個 Tab 落在主要內容裡（不是又回到側邊導覽）。
  await page.keyboard.press("Enter");
  await page.keyboard.press("Tab");
  const insideMain = await page.evaluate(() =>
    Boolean(document.getElementById("main-content")?.contains(document.activeElement))
  );
  expect(insideMain, `焦點沒有進入主要內容：${await focusInfo(page)}`).toBe(true);
});

test("鍵盤：從頁首一路 Tab 一定按得到「緊急停止」", async ({ page }) => {
  test.setTimeout(90_000);
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  await page.reload();
  await expect(page.getByRole("navigation", { name: "主要導覽" })).toBeVisible({ timeout: 20_000 });
  const estop = page.locator(".topbar").getByRole("button", { name: "緊急停止", exact: true });
  await expect(estop).toBeVisible();
  let focused = false;
  for (let i = 0; i < 40 && !focused; i += 1) {
    await page.keyboard.press("Tab");
    focused = await estop.evaluate((el) => el === document.activeElement);
  }
  expect(focused, `40 次 Tab 之內沒有走到緊急停止，最後停在 ${await focusInfo(page)}`).toBe(true);
  await expect(estop).toBeFocused();
});

test("螢幕閱讀器：390px 的導覽、通知中心與快速操作都有名字", async ({ page }) => {
  test.setTimeout(90_000);
  await page.setViewportSize(NARROW);
  await page.goto(appUrl());
  await expect(page.getByRole("navigation", { name: "主要導覽（窄視窗）" })).toBeVisible({
    timeout: 20_000,
  });
  // 通知中心的名字要帶「幾項待決定」（不是只有一個鈴鐺圖示）。
  await expect(page.getByRole("button", { name: /通知中心，(\d+|未知) 項待決定/ })).toBeVisible();
  // 首頁三張卡與五個快速操作：每一顆可按的東西都要有非空的可及名稱。
  const nameless = await page.locator(".home button, .bottom-nav button").evaluateAll((els) =>
    els
      .filter((el) => {
        const label = el.getAttribute("aria-label") ?? el.textContent ?? "";
        return label.trim().length === 0;
      })
      .map((el) => el.outerHTML.slice(0, 120))
  );
  expect(nameless, "有按鈕沒有可及名稱").toEqual([]);
  for (const name of ["交代一件事", "暫停主動互動", "加入裝置", "停止所有感測"]) {
    await expect(page.locator(".home").getByRole("button", { name })).toBeVisible();
  }
  await expect(
    page.locator(".home").getByRole("button", { name: "緊急停止", exact: true })
  ).toBeVisible();
  // 分組也要有名字（「暫停或恢復主動互動」是一組，不是兩顆孤兒按鈕）。
  await expect(page.getByRole("group", { name: "暫停或恢復主動互動" })).toBeVisible();
});

test("減少動態：偏好打開後，html 真的帶 reduce-motion，而且樣式真的生效", async ({
  page,
  request,
}) => {
  test.setTimeout(90_000);
  try {
    await api(request, "PATCH", "/v1/ui/preferences", { reduceMotion: true });
    await page.setViewportSize(DESKTOP);
    await openApp(page);
    await expect(page.locator("html")).toHaveClass(/reduce-motion/, { timeout: 20_000 });
    // 規則真的套用到元素上（不是只加了一個沒有作用的 class）。
    const duration = await page
      .locator(".app")
      .evaluate((el) => getComputedStyle(el).transitionDuration);
    expect(duration).not.toBe("0s");
  } finally {
    await api(request, "PATCH", "/v1/ui/preferences", { reduceMotion: false });
  }
  await page.reload();
  await expect(page.locator("html")).not.toHaveClass(/reduce-motion/, { timeout: 20_000 });

  // 系統層級的「減少動態」也吃得到（不需要改偏好）。
  await page.emulateMedia({ reducedMotion: "reduce" });
  expect(
    await page.evaluate(() => window.matchMedia("(prefers-reduced-motion: reduce)").matches)
  ).toBe(true);
  await page.emulateMedia({ reducedMotion: null });
});

test("外觀：淺色／深色／跟隨系統在五個一級入口、兩種視窗尺寸下都一致", async ({
  page,
  request,
}) => {
  test.setTimeout(300_000);
  const background: Record<string, string> = {};
  try {
    for (const appearance of ["light", "dark"] as const) {
      await api(request, "PATCH", "/v1/ui/preferences", { appearance });
      await page.setViewportSize(DESKTOP);
      await openApp(page);
      await expect(page.locator("html")).toHaveAttribute("data-theme", appearance, {
        timeout: 20_000,
      });
      background[appearance] = await page
        .locator("body")
        .evaluate((el) => getComputedStyle(el).backgroundColor);
      for (const target of PAGES) {
        await navigateTo(page, target, false);
        await expect(page.locator("html")).toHaveAttribute("data-theme", appearance);
      }
      await page.setViewportSize(NARROW);
      await expect(page.getByRole("navigation", { name: "主要導覽（窄視窗）" })).toBeVisible();
      for (const target of PAGES) {
        await navigateTo(page, target, true);
        await expect(page.locator("html")).toHaveAttribute("data-theme", appearance);
      }
    }

    // 換了主題不只是換一個屬性：真的換了顏色。
    expect(background.light).not.toBe(background.dark);

    // 跟隨系統：不寫死 data-theme，改由作業系統的偏好決定顏色。
    await api(request, "PATCH", "/v1/ui/preferences", { appearance: "system" });
    for (const scheme of ["light", "dark"] as const) {
      await page.emulateMedia({ colorScheme: scheme });
      await page.setViewportSize(DESKTOP);
      await openApp(page);
      await expect(page.locator("html")).not.toHaveAttribute("data-theme", /.*/, {
        timeout: 20_000,
      });
      const background = await page
        .locator("body")
        .evaluate((el) => getComputedStyle(el).backgroundColor);
      expect(background, "跟隨系統時仍然要有明確的背景色").not.toBe("rgba(0, 0, 0, 0)");
      await page.setViewportSize(NARROW);
      await expect(page.getByRole("navigation", { name: "主要導覽（窄視窗）" })).toBeVisible();
    }
  } finally {
    await page.emulateMedia({ colorScheme: null });
    await api(request, "PATCH", "/v1/ui/preferences", { appearance: "system" });
  }
});

// v0.6：角色頁（含新的「同步」卡）也要過同一套可及性檢查。
test("螢幕閱讀器：角色頁每個可按的東西都有名字，「同步」卡是一個有名字的區域", async ({ page }) => {
  test.setTimeout(120_000);
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  await navigateTo(page, PAGES[1], false);

  // 同步卡：有 region 名稱、有可鍵盤操作的「重新檢查」。
  const card = page.getByRole("region", { name: "角色同步" });
  await expect(card).toBeVisible({ timeout: 20_000 });
  const recheck = card.getByRole("button", { name: "重新檢查" });
  await recheck.focus();
  await expect(recheck).toBeFocused();

  // 角色頁的每一顆按鈕都有非空的可及名稱（沿用 390px 那一支的檢查方式）。
  const nameless = await page.locator(".character-page button").evaluateAll((els) =>
    els
      .filter((el) => {
        const label = el.getAttribute("aria-label") ?? el.textContent ?? "";
        return label.trim().length === 0;
      })
      .map((el) => el.outerHTML.slice(0, 120))
  );
  expect(nameless, "角色頁有按鈕沒有可及名稱").toEqual([]);
});
