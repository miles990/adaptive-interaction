// 390px narrow-viewport navigation + keyboard accessibility.

import { test, expect } from "@playwright/test";

test.use({ viewport: { width: 390, height: 844 } });

function appUrl(): string {
  return `/?api=${encodeURIComponent(process.env.E2E_API!)}&token=${encodeURIComponent(
    process.env.E2E_TOKEN!
  )}`;
}

test("390px：底部導覽可抵達所有頁面，緊急停止保持可見", async ({ page }) => {
  await page.goto(appUrl());
  const bottomNav = page.getByRole("navigation", { name: "主要導覽（窄視窗）" });
  await expect(bottomNav).toBeVisible({ timeout: 15_000 });
  // Primary items carry text labels, not just icons.
  await expect(bottomNav.getByText("首頁")).toBeVisible();
  await expect(bottomNav.getByText("AI 與工作階段")).toBeVisible();
  // Emergency stop stays reachable in the top bar.
  await expect(page.getByRole("button", { name: "緊急停止", exact: true })).toBeVisible();
  // The "more" sheet reaches every remaining page.
  await bottomNav.getByRole("button", { name: "更多" }).click();
  const sheet = page.getByRole("dialog", { name: "更多功能" });
  await expect(sheet).toBeVisible();
  for (const label of ["小樞", "能力與裝置", "記憶與知識", "自動互動", "設定"]) {
    await expect(sheet.getByText(label)).toBeVisible();
  }
  await sheet.getByText("能力與裝置").click();
  await expect(sheet).not.toBeVisible();
  await expect(page.getByText("系統時間")).toBeVisible({ timeout: 10_000 });
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
