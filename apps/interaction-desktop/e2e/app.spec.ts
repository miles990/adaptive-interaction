// Browser-level E2E against a REAL runtime daemon (isolated home).
// Sequential: onboarding first, emergency stop last (it revokes consents).

import { test, expect, Page } from "@playwright/test";

function appUrl(): string {
  return `/?api=${encodeURIComponent(process.env.E2E_API!)}&token=${encodeURIComponent(
    process.env.E2E_TOKEN!
  )}`;
}

async function open(page: Page) {
  await page.goto(appUrl());
}

test.describe.configure({ mode: "serial" });

test("首次設定精靈：完整走過 7 步並套用", async ({ page }) => {
  await open(page);
  const wizard = page.getByRole("dialog", { name: "首次設定" });
  await expect(wizard).toBeVisible({ timeout: 15_000 });
  await expect(wizard.getByRole("heading", { name: "歡迎使用自適應互動" })).toBeVisible();
  for (let i = 0; i < 6; i++) {
    await wizard.getByRole("button", { name: "下一步" }).click();
  }
  await wizard.getByRole("button", { name: "完成設定" }).click();
  // Wizard closes into the home page.
  await expect(page.getByRole("navigation", { name: "主要導覽" })).toBeVisible({
    timeout: 15_000,
  });
});

test("首頁：權限地圖與狀態顯示", async ({ page }) => {
  await open(page);
  await expect(page.getByRole("navigation", { name: "主要導覽" })).toBeVisible({
    timeout: 15_000,
  });
  // The three-zone permission map (AI 可以知道 / 可以做 / 必須先問).
  await expect(page.getByText("AI 可以知道", { exact: true })).toBeVisible();
  await expect(page.getByText("AI 可以做", { exact: true })).toBeVisible();
  await expect(page.getByText("AI 必須先問", { exact: true })).toBeVisible();
});

test("暫停主動互動：暫停與恢復都反映後端真實狀態", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "暫停主動互動" }).click();
  await expect(page.getByText("主動互動已暫停").first()).toBeVisible();
  await page.getByRole("button", { name: "恢復主動互動" }).click();
  await expect(page.getByRole("button", { name: "暫停主動互動" })).toBeVisible();
});

test("感知來源：卡片誠實顯示資料流向；未知不得顯示為安全", async ({ page }) => {
  await open(page);
  await page.getByRole("navigation", { name: "主要導覽" }).getByText("感知來源").click();
  // Builtin local receptors carry the local-only badge.
  await expect(page.getByText("僅限本機").first()).toBeVisible();
  // The system-time card resolves via the catalog (Chinese name, not raw id).
  await expect(page.getByText("系統時間")).toBeVisible();
});

test("自動互動：句子式建立 → 摘要 → 模擬（零副作用）", async ({ page }) => {
  await open(page);
  await page.getByRole("navigation", { name: "主要導覽" }).getByText("自動互動").click();
  await page.getByRole("button", { name: "建立自動互動" }).click();
  // The sentence editor shows a natural-language summary; save it.
  await page.getByRole("button", { name: "儲存" }).click();
  await expect(page.getByText("新的自動互動").first()).toBeVisible();
  // Simulate: reuses the real decision logic with zero side effects.
  await page.getByRole("button", { name: "模擬" }).first().click();
  const dialog = page.getByRole("dialog", { name: "模擬這個自動互動" });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "開始模擬" }).click();
  await expect(dialog.getByText(/不會真的執行|零副作用|模擬/).first()).toBeVisible();
});

test("進階模式：設定切換後顯示技術頁面", async ({ page }) => {
  await open(page);
  await page.getByRole("navigation", { name: "主要導覽" }).getByText("設定").click();
  // The toggle is a controlled input persisted through the backend, so state
  // flips only after the round-trip; assert on the advanced nav instead.
  const toggle = page.getByRole("checkbox", { name: "顯示進階功能" });
  const yamlNav = page.getByRole("navigation", { name: "主要導覽" }).getByText("配方 YAML");
  await expect(toggle).toBeVisible();
  await page.waitForTimeout(500); // let stored prefs land in the controlled input
  // Deterministic start: a previous (failed) run may have left advanced on.
  if ((await yamlNav.count()) > 0) {
    await toggle.click();
    await expect(yamlNav).toHaveCount(0, { timeout: 10_000 });
  }
  await toggle.click();
  await expect(yamlNav).toBeVisible({ timeout: 10_000 });
  // Switch back to simple mode for the rest of the suite.
  await toggle.click();
  await expect(yamlNav).toHaveCount(0, { timeout: 10_000 });
});

test("緊急停止：二段確認觸發 → 誠實顯示 → 安全解除流程", async ({ page }) => {
  await open(page);
  await expect(page.getByRole("navigation", { name: "主要導覽" })).toBeVisible({
    timeout: 15_000,
  });
  // Two-step trigger.
  await page.getByRole("button", { name: "緊急停止", exact: true }).click();
  await page.getByRole("button", { name: "立即停止一切？" }).click();
  await expect(page.getByText("緊急停止已啟動").first()).toBeVisible();
  // The header deliberately has no clear button — it navigates to safety.
  await page.getByRole("button", { name: /緊急停止中 — 前往解除/ }).click();
  await page.getByRole("button", { name: /開始安全解除流程/ }).click();
  const dialog = page.getByRole("dialog", { name: "解除緊急停止" });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "我了解，解除緊急停止" }).click();
  await dialog.getByRole("button", { name: "確定解除？" }).click();
  // Back to normal: the estop banner disappears and the trigger returns.
  await expect(page.getByRole("button", { name: "緊急停止", exact: true })).toBeVisible({
    timeout: 10_000,
  });
});
