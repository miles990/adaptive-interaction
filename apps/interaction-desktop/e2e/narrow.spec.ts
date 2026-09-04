// 390px narrow-viewport navigation + keyboard accessibility（v0.5 五入口）。

import { test, expect } from "@playwright/test";
import { appUrl } from "./helpers";
import { CHARACTER_SYNC_PROJECTION } from "../src/statusProjection";

test.use({ viewport: { width: 390, height: 844 } });

test("390px：底部導覽可抵達所有頁面，緊急停止保持可見", async ({ page }) => {
  await page.goto(appUrl());
  const bottomNav = page.getByRole("navigation", { name: "主要導覽（窄視窗）" });
  await expect(bottomNav).toBeVisible({ timeout: 15_000 });
  // Primary items carry text labels, not just icons.
  await expect(bottomNav.getByText("現在")).toBeVisible();
  // 第二項是目前角色的名字（瀏覽器 e2e：bundled 索引 default → 小樞；載入失敗 → 角色）。
  await expect(bottomNav.getByText(/^(小樞|角色)$/)).toBeVisible();
  await expect(bottomNav.getByText("工作")).toBeVisible();
  await expect(bottomNav.getByText("連接與權限")).toBeVisible();
  // Emergency stop stays reachable in the top bar（首頁快速操作也有一顆，需限定範圍）。
  await expect(
    page.locator(".topbar").getByRole("button", { name: "緊急停止", exact: true })
  ).toBeVisible();
  // The "more" sheet reaches every remaining page.
  await bottomNav.getByRole("button", { name: "更多" }).click();
  const sheet = page.getByRole("dialog", { name: "更多功能" });
  await expect(sheet).toBeVisible();
  for (const label of ["記憶與資料", "活動紀錄", "外觀與語言", "備份與還原", "進階模式"]) {
    await expect(sheet.getByText(label)).toBeVisible();
  }
  await sheet.getByText("記憶與資料").click();
  await expect(sheet).not.toBeVisible();
  await expect(page.getByText("關於我的記憶").first()).toBeVisible({ timeout: 10_000 });
  // 再打開選單時，目前所在的細項要看得出來（regression: 細項永遠不高亮）。
  await bottomNav.getByRole("button", { name: "更多" }).click();
  await expect(sheet.getByRole("button", { name: "記憶與資料" })).toHaveAttribute(
    "aria-current",
    "page"
  );
  await expect(sheet.getByRole("button", { name: "活動紀錄" })).not.toHaveAttribute("aria-current");
  await page.keyboard.press("Escape");
});

test("390px：鍵盤可操作底部導覽與更多選單", async ({ page }) => {
  await page.goto(appUrl());
  const bottomNav = page.getByRole("navigation", { name: "主要導覽（窄視窗）" });
  await expect(bottomNav).toBeVisible({ timeout: 15_000 });
  // Focus the 更多 button via keyboard and open it with Enter.
  await bottomNav.getByRole("button", { name: "更多" }).focus();
  await page.keyboard.press("Enter");
  const sheet = page.getByRole("dialog", { name: "更多功能" });
  await expect(sheet).toBeVisible();
  // Escape closes; focus does not fall off-screen (returns to the page).
  await page.keyboard.press("Escape");
  await expect(sheet).not.toBeVisible();
});

test("390px：頁面主體不產生水平捲動", async ({ page }) => {
  await page.goto(appUrl());
  await expect(page.getByRole("navigation", { name: "主要導覽（窄視窗）" })).toBeVisible({
    timeout: 15_000,
  });
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth
  );
  expect(overflow).toBeLessThanOrEqual(1);
});

test("390px：「現在」第一屏的三個回答與快速操作都看得到、按得到", async ({ page }) => {
  await page.goto(appUrl());
  await expect(page.getByRole("navigation", { name: "主要導覽（窄視窗）" })).toBeVisible({
    timeout: 15_000,
  });
  await expect(page.getByTestId("now-character")).toBeVisible();
  await expect(page.getByTestId("now-work")).toBeVisible();
  await expect(page.getByTestId("now-decisions")).toBeVisible();
  const homeEstop = page.locator(".home").getByRole("button", { name: "緊急停止", exact: true });
  await homeEstop.scrollIntoViewIfNeeded();
  await expect(homeEstop).toBeVisible();
  for (const name of ["交代一件事", "暫停主動互動", "加入裝置", "停止所有感測"]) {
    const button = page.getByRole("button", { name });
    await button.scrollIntoViewIfNeeded();
    await expect(button).toBeVisible();
    await expect(button).toBeInViewport();
  }
  // 快速操作在單欄下不溢出視窗寬度。
  const box = await page.getByRole("button", { name: "交代一件事" }).boundingBox();
  expect(box).not.toBeNull();
  expect(box!.x + box!.width).toBeLessThanOrEqual(390);
  await page.getByRole("button", { name: "加入裝置" }).click();
  await expect(page.locator(".topbar-title")).toHaveText("連接與權限");
});

// v0.6：角色頁的「同步」卡在 390px 上要讀得完（不是桌面限定的資訊）。
test("390px：角色頁的「同步」卡看得到、不溢出，而且空狀態是中性的", async ({ page }) => {
  await page.goto(appUrl());
  const bottomNav = page.getByRole("navigation", { name: "主要導覽（窄視窗）" });
  await expect(bottomNav).toBeVisible({ timeout: 15_000 });
  await bottomNav.getByText(/^(小樞|角色)$/).click();
  const card = page.getByTestId("character-sync");
  await card.scrollIntoViewIfNeeded();
  await expect(card).toBeVisible({ timeout: 20_000 });
  // 這一支跑在共用 daemon 上，前面的 spec 可能配對過或撤銷過手機——所以不假設
  // 是哪一種狀態，只驗「一定是投影表裡的其中一句人話」，而且綠勾只給真的已同步。
  //
  // 允許清單直接從 CHARACTER_SYNC_PROJECTION 導出，不再手抄：手抄的那一份漏掉了
  // 「角色同步紀錄曾損毀，已重新開始」，於是那一態真的出現時 e2e 會誤判為失敗，
  // 而不是驗證它（對抗審查 evidence-honesty-015）。
  const headline = await card.locator(".badge").first().innerText();
  const allowed = Object.values(CHARACTER_SYNC_PROJECTION).map((p) => p.headline);
  expect(allowed).toContain(headline.trim());
  if (headline.trim() !== "iPhone 已連接，角色狀態已同步") {
    await expect(card.locator(".badge-ok")).toHaveCount(0);
  }
  // 一般模式看不到技術數字。
  expect((await card.innerText()).toLowerCase()).not.toMatch(/revision|sequence|epoch|schema|token/);
  // 卡片不超出 390px。
  const box = await card.boundingBox();
  expect(box).not.toBeNull();
  expect(box!.x + box!.width).toBeLessThanOrEqual(390);
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth
  );
  expect(overflow).toBeLessThanOrEqual(1);
});
