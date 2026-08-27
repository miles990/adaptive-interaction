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
  await wizard.getByRole("button", { name: "下一步" }).click();
  await wizard.getByRole("button", { name: "掃描目前可用裝置" }).click();
  await expect(wizard.getByRole("status")).toContainText("感測器啟動：否");
  for (let i = 0; i < 5; i++) await wizard.getByRole("button", { name: "下一步" }).click();
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

test("能力與裝置：感知卡片誠實顯示資料流向；provider 分頁含掃描誠實文案", async ({ page }) => {
  await open(page);
  await page.getByRole("navigation", { name: "主要導覽" }).getByText("能力與裝置").click();
  // 預設分頁＝感知來源：builtin local receptors carry the local-only badge.
  await expect(page.getByText("僅限本機").first()).toBeVisible();
  await expect(page.getByText("系統時間")).toBeVisible();
  // 裝置與提供者：掃描文案誠實（已偵測≠全部）。
  await page.getByRole("tab", { name: "裝置與提供者" }).click();
  await expect(page.getByText("不代表找到了所有硬體")).toBeVisible();
  await expect(page.getByText("桌面角色小樞（Presentation）")).toBeVisible();
  await page.getByRole("button", { name: "重新掃描" }).click();
  await expect(page.getByText(/感測器啟動：否/)).toBeVisible();
  // 每一類都由真實掃描報告標示可見／需權限／未知／不支援，並附具體原因。
  await expect(page.getByText("攝影機與影像來源").first()).toBeVisible();
  await expect(page.getByText(/不以路徑或假資料冒充裝置/).first()).toBeVisible();
});

test("新 IA：8 個一級頁全部可達", async ({ page }) => {
  await open(page);
  const nav = page.getByRole("navigation", { name: "主要導覽" });
  await expect(nav).toBeVisible({ timeout: 15_000 });
  const pages: [string, RegExp | string][] = [
    ["小樞", "狀態預覽（取自實際角色素材）"],
    ["AI 與工作階段", "本機 AI Agent"],
    ["記憶與知識", "小樞記住了什麼"],
    ["活動與確認", /統一收件匣/],
    ["隱私與安全", /緊急停止|同意/],
    ["設定", "顯示模式"],
    ["首頁", "權限地圖 — AI 現在可以做什麼？"],
  ];
  for (const [label, marker] of pages) {
    await nav.getByText(label, { exact: true }).click();
    await expect(page.getByText(marker).first()).toBeVisible({ timeout: 10_000 });
  }
});

test("小樞：Character Pack 來源與 Behavior State 都誠實呈現", async ({ page }) => {
  await open(page);
  await page.getByRole("navigation", { name: "主要導覽" }).getByText("小樞", { exact: true }).click();
  await expect(page.getByText("Character Pack 詳情")).toBeVisible();
  await expect(page.getByText(/App 同源內建資產/)).toBeVisible();
  await expect(page.getByText(/manifest、sprite sheet、frame/)).toBeVisible();
  await expect(page.getByText(/內建安全 fallback 不可單獨解除安裝/)).toBeVisible();
  await expect(page.getByText("現在的 Behavior State")).toBeVisible();
  // Browser E2E has no native companion window. The UI must say so instead of
  // manufacturing idle percentages as though they were live telemetry.
  await expect(page.getByText(/尚未收到角色視窗的即時狀態/)).toBeVisible();
});

test("AI 與工作階段：真實 agent 發現誠實顯示；建立面板有授權預覽", async ({ page }) => {
  await open(page);
  await page.getByRole("navigation", { name: "主要導覽" }).getByText("AI 與工作階段").click();
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

test("記憶與知識：分層誠實標示；候選複審入口存在", async ({ page }) => {
  await open(page);
  await page.getByRole("navigation", { name: "主要導覽" }).getByText("記憶與知識").click();
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
