// Browser-level E2E against a REAL runtime daemon (isolated home).
// Sequential: onboarding first, emergency stop last (it revokes consents).
// v0.5 IA：5 個一級入口（現在／小樞／工作／連接與權限／更多）。

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

test("首次設定精靈：3 步完成並套用", async ({ page }) => {
  await open(page);
  const wizard = page.getByRole("dialog", { name: "首次設定" });
  await expect(wizard).toBeVisible({ timeout: 15_000 });
  await expect(wizard.getByRole("heading", { name: "認識小樞" })).toBeVisible();
  await wizard.getByRole("button", { name: "下一步" }).click();
  // 第二步：AI 幫手——誠實 discovery（CI 上可能未安裝，狀態誠實即可）。
  await expect(wizard.getByRole("heading", { name: "要讓小樞幫忙工作嗎？" })).toBeVisible();
  await wizard.getByRole("button", { name: "下一步" }).click();
  // 第三步：安全預設——保證文字與資料流摘要。
  await expect(wizard.getByRole("heading", { name: "安全預設" })).toBeVisible();
  await expect(wizard.getByText(/麥克風、攝影機、定位/)).toBeVisible();
  await wizard.getByRole("button", { name: "完成設定" }).click();
  // Wizard closes into the home page.
  await expect(page.getByRole("navigation", { name: "主要導覽" })).toBeVisible({
    timeout: 15_000,
  });
});

test("現在：系統狀態與摘要條，不重複完整權限地圖", async ({ page }) => {
  await open(page);
  await expect(page.getByRole("navigation", { name: "主要導覽" })).toBeVisible({
    timeout: 15_000,
  });
  await expect(page.getByText("系統狀態", { exact: true })).toBeVisible();
  await expect(page.getByText("待我決定", { exact: true })).toBeVisible();
  await expect(page.getByText("進行中的工作", { exact: true })).toBeVisible();
  // 首頁瘦身：完整權限地圖已移到「連接與權限」。
  await expect(page.getByText("權限地圖 — AI 現在可以做什麼？")).toHaveCount(0);
});

test("暫停主動互動：暫停與恢復都反映後端真實狀態", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "暫停主動互動" }).click();
  await expect(page.getByText("主動互動已暫停").first()).toBeVisible();
  await page.getByRole("button", { name: "恢復主動互動" }).click();
  await expect(page.getByRole("button", { name: "暫停主動互動" })).toBeVisible();
});

test("連接與權限：裝置與能力誠實掃描；同意與安全含權限地圖", async ({ page }) => {
  await open(page);
  await page.getByRole("navigation", { name: "主要導覽" }).getByText("連接與權限").click();
  // 預設分頁＝裝置與能力 → 感知來源：builtin local receptors carry the local-only badge.
  await expect(page.getByText("僅限本機").first()).toBeVisible();
  await expect(page.getByText("系統時間")).toBeVisible();
  // 裝置與提供者：掃描文案誠實（已偵測≠全部）。
  await page.getByRole("tab", { name: "裝置與提供者" }).click();
  await expect(page.getByText("不代表找到了所有硬體")).toBeVisible();
  await expect(page.getByText("桌面角色小樞（Presentation）")).toBeVisible();
  await page.getByRole("button", { name: "重新掃描" }).click();
  await expect(page.getByText(/感測器啟動：否/)).toBeVisible();
  await expect(page.getByText("攝影機與影像來源").first()).toBeVisible();
  await expect(page.getByText(/不以路徑或假資料冒充裝置/).first()).toBeVisible();
  // 同意與安全分頁：完整權限地圖唯一的家＋緊急停止說明。
  await page.getByRole("tab", { name: "同意與安全" }).click();
  await expect(page.getByText("權限地圖 — AI 現在可以做什麼？")).toBeVisible();
  await expect(page.getByText("AI 可以知道", { exact: true })).toBeVisible();
  await expect(page.getByText("AI 可以做", { exact: true })).toBeVisible();
  await expect(page.getByText("AI 必須先問", { exact: true })).toBeVisible();
  await expect(page.getByText(/緊急停止未啟動/)).toBeVisible();
});

test("新 IA：5 個一級入口全部可達", async ({ page }) => {
  await open(page);
  const nav = page.getByRole("navigation", { name: "主要導覽" });
  await expect(nav).toBeVisible({ timeout: 15_000 });
  const pages: [string, RegExp | string][] = [
    ["小樞", /36 表情預覽/],
    ["工作", "本機 AI Agent"],
    ["連接與權限", "系統時間"],
    ["更多", "小樞記住了什麼"],
    ["現在", "快速操作"],
  ];
  for (const [label, marker] of pages) {
    await nav.getByText(label, { exact: true }).click();
    await expect(page.getByText(marker).first()).toBeVisible({ timeout: 10_000 });
  }
});

test("小樞：Pack 詳情、Behavior State 與主動對話設定（單一主人）", async ({ page }) => {
  await open(page);
  await page.getByRole("navigation", { name: "主要導覽" }).getByText("小樞", { exact: true }).click();
  await expect(page.getByText("Character Pack 詳情")).toBeVisible();
  // v0.5 正式版是執行期參數化 rig：來源、形式與誠實對照都要可見。
  await expect(page.getByText(/App 內建程式碼（無外部素材、無遠端程式）/)).toBeVisible();
  await expect(page.getByText(/執行期參數化分層 rig/).first()).toBeVisible();
  await expect(page.getByText(/36 表情預覽/)).toBeVisible();
  await expect(page.getByText("只點頭，沒有綠勾")).toBeVisible();
  await expect(page.getByText("綠勾只在驗證後")).toBeVisible();
  await expect(page.getByText("現在的 Behavior State")).toBeVisible();
  // Roll Call（現在大家在做什麼）：瀏覽器模式沒有角色視窗，必須誠實說明。
  await expect(page.getByText("現在大家在做什麼")).toBeVisible();
  await expect(page.getByText(/尚未收到角色視窗的回報/)).toBeVisible();
  // Browser E2E has no native companion window. The UI must say so instead of
  // manufacturing idle percentages as though they were live telemetry.
  await expect(page.getByText(/尚未收到角色視窗的即時狀態/)).toBeVisible();
  // v0.5 單一主人：主動對話與主動程度／安靜時段住在小樞頁。
  await expect(page.getByText("主動式對話")).toBeVisible();
  await expect(page.getByText("主動程度與安靜時段")).toBeVisible();
});

test("工作：真實 agent 發現誠實顯示；建立面板有授權預覽", async ({ page }) => {
  await open(page);
  await page.getByRole("navigation", { name: "主要導覽" }).getByText("工作", { exact: true }).click();
  // 發現結果卡片（真 daemon 的真實偵測——CI 上可能未安裝，狀態誠實即可）。
  await expect(page.getByText(/Codex|Claude Code/).first()).toBeVisible({ timeout: 15_000 });
  await page.getByRole("button", { name: "建立工作階段…" }).click();
  const sheet = page.getByRole("dialog", { name: "建立 AI 工作階段" });
  await expect(sheet).toBeVisible();
  // Consent Sheet 語意：資料範圍／模式／外部傳送／取消方式。
  await expect(sheet.getByText("授權預覽")).toBeVisible();
  await expect(sheet.getByText(/唯讀／計畫/)).toBeVisible();
  await expect(sheet.getByText(/外部傳送/)).toBeVisible();
  await sheet.getByRole("button", { name: "取消" }).click();
});

test("更多：記憶與知識分層誠實標示；候選複審入口存在", async ({ page }) => {
  await open(page);
  await page.getByRole("navigation", { name: "主要導覽" }).getByText("更多", { exact: true }).click();
  await expect(page.getByText("沒有你不能刪除的記憶")).toBeVisible();
  await page.getByRole("tab", { name: "知識與候選" }).click();
  await expect(page.getByText(/AI（含各 agent）只能提出/)).toBeVisible();
  await page.getByRole("tab", { name: "提供給 AI 的內容" }).click();
  await expect(page.getByText(/實際會提供哪些/)).toBeVisible();
});

test("全域搜尋：Ctrl+K 開啟、能導頁、指令列出", async ({ page }) => {
  await open(page);
  await expect(page.getByRole("navigation", { name: "主要導覽" })).toBeVisible({ timeout: 15_000 });
  await page.keyboard.press("ControlOrMeta+k");
  const overlay = page.getByRole("dialog", { name: "全域搜尋" });
  await expect(overlay).toBeVisible();
  await expect(overlay.getByText("緊急停止").first()).toBeVisible();
  await overlay.getByPlaceholder(/搜尋設定/).fill("記憶與知識");
  await overlay.getByRole("option", { name: /記憶與知識/ }).first().click();
  await expect(page.getByText("小樞記住了什麼")).toBeVisible({ timeout: 10_000 });
});

test("工作 → 自動互動：句子式建立 → 摘要 → 模擬（零副作用）", async ({ page }) => {
  await open(page);
  await page.getByRole("navigation", { name: "主要導覽" }).getByText("工作", { exact: true }).click();
  await page.getByRole("tab", { name: "自動互動" }).click();
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

test("右上 Inbox：待決定入口與完整歷史", async ({ page }) => {
  await open(page);
  await expect(page.getByRole("navigation", { name: "主要導覽" })).toBeVisible({ timeout: 15_000 });
  await page.getByRole("button", { name: /通知中心/ }).click();
  const panel = page.getByRole("dialog", { name: "通知中心" });
  await expect(panel).toBeVisible();
  await expect(panel.getByText("待你決定")).toBeVisible();
  await panel.getByRole("button", { name: "查看完整活動歷史" }).click();
  // Activity 不再是一級頁：落在「更多 → 活動歷史」。
  await expect(page.locator(".topbar-title")).toHaveText("更多");
  await expect(page.getByText(/統一收件匣/)).toBeVisible({ timeout: 10_000 });
});

test("進階模式：更多 → 設定切換後顯示技術頁面", async ({ page }) => {
  await open(page);
  await page.getByRole("navigation", { name: "主要導覽" }).getByText("更多", { exact: true }).click();
  await page.getByRole("tab", { name: "設定" }).click();
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
  // The header deliberately has no clear button — it navigates to safety
  // (now the 同意與安全 tab inside 連接與權限).
  await page.getByRole("button", { name: /緊急停止中 — 前往解除/ }).click();
  await expect(page.locator(".topbar-title")).toHaveText("連接與權限");
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
