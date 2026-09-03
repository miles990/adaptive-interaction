// Browser-level E2E against a REAL runtime daemon (isolated home).
// Sequential: onboarding first, emergency stop last (it revokes consents).
// v0.5 IA：5 個一級入口（現在／角色／工作／連接與權限／更多）。
// 導覽第二項是目前角色的名字：瀏覽器 e2e 沒有桌面角色視窗，名字來自 bundled 索引的
// default（shu-maid → 小樞）；角色載入失敗時會是中立的「角色」。

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
  // 精靈套用後接「首次成功體驗」：可略過的一屏（不是第四個必要步驟）。
  // 安全文字固定、看過的旗標由 Runtime 保存（GET /v1/ui/preferences firstSuccessSeen）。
  const firstSuccess = page.getByRole("dialog", { name: "首次成功體驗" });
  await expect(firstSuccess).toBeVisible({ timeout: 15_000 });
  await expect(
    firstSuccess.getByRole("heading", { name: /準備好了。要不要先試一次？/ })
  ).toBeVisible();
  await expect(firstSuccess.getByText(/安全訊息永遠是固定文字/)).toBeVisible();
  for (const option of ["提醒我休息", "交代一件小工作", "先在桌面陪我", "更換角色"]) {
    await expect(firstSuccess.getByText(option, { exact: true })).toBeVisible();
  }
  await firstSuccess.getByRole("button", { name: "完成", exact: true }).click();
  // Wizard closes into the home page.
  await expect(page.getByRole("navigation", { name: "主要導覽" })).toBeVisible({
    timeout: 15_000,
  });
  // 看過的旗標真的落在 host（不是只在這個視窗記住）。
  const prefs = await page.request.get(`${process.env.E2E_API!}/v1/ui/preferences`, {
    headers: { Authorization: `Bearer ${process.env.E2E_TOKEN!}` },
  });
  expect(((await prefs.json()) as { firstSuccessSeen?: boolean }).firstSuccessSeen).toBe(true);
});

test("現在：第一屏只回答三件事＋快速操作；系統狀態收在詳細狀態", async ({ page }) => {
  await open(page);
  await expect(page.getByRole("navigation", { name: "主要導覽" })).toBeVisible({
    timeout: 15_000,
  });
  // 角色現在怎麼樣：瀏覽器 e2e 沒有角色視窗 → 可信的固定文字（不是角色文案）。
  await expect(page.getByTestId("now-character")).toBeVisible();
  await expect(page.getByText("角色離線，改用文字。")).toBeVisible();
  await expect(page.getByText("待我決定", { exact: true })).toBeVisible();
  await expect(page.getByText("進行中的工作", { exact: true })).toBeVisible();
  // 快速操作三件：交代一件事／暫停主動互動／加入裝置。
  await expect(page.getByRole("button", { name: "交代一件事" })).toBeVisible();
  await expect(page.getByRole("button", { name: "暫停主動互動" })).toBeVisible();
  await expect(page.getByRole("button", { name: "加入裝置" })).toBeVisible();
  // 數量與系統狀態不在第一屏；展開「詳細狀態」才出現。
  await expect(page.getByText("系統狀態", { exact: true })).toHaveCount(0);
  await page.getByText("詳細狀態", { exact: true }).click();
  await expect(page.getByText("系統狀態", { exact: true })).toBeVisible();
  await expect(page.getByText(/已載入 \d+ 個自動互動/)).toBeVisible();
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
  // provider 顯示名跟 Character Protocol 協商到的角色走：瀏覽器 e2e 沒有桌面
  // 角色視窗時是「桌面角色（尚未連線）」，hello 過就是「桌面角色：<名字>（Presentation）」。
  await expect(page.getByText(/^桌面角色(：[^（]+（Presentation）|（尚未連線）)$/)).toBeVisible();
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
    ["更多", "關於我的記憶"],
    ["現在", "快速操作"],
  ];
  for (const [label, marker] of pages) {
    await nav.getByText(label, { exact: true }).click();
    await expect(page.getByText(marker).first()).toBeVisible({ timeout: 10_000 });
  }
});

test("小樞：一般模式角色頁（目前角色／外觀／陪伴／更換）與誠實對照（單一主人）", async ({ page }) => {
  await open(page);
  await page.getByRole("navigation", { name: "主要導覽" }).getByText("小樞", { exact: true }).click();
  // 一般模式的五個區塊（技術資料只在進階模式）。
  for (const heading of ["目前角色", "外觀與名字", "平常如何陪伴", "安靜與勿擾", "更換或加入角色"]) {
    await expect(page.getByRole("heading", { name: heading, exact: true })).toBeVisible();
  }
  // 目前角色：能力摘要來自 manifest／registry 的轉述；瀏覽器 e2e 沒有角色視窗，必須誠實說未連線。
  await expect(page.getByRole("list", { name: "角色能力摘要" })).toBeVisible();
  await expect(page.getByText("角色視窗未連線")).toBeVisible();
  await expect(page.getByText(/36 表情預覽/)).toBeVisible();
  await expect(page.getByText("只點頭，沒有綠勾")).toBeVisible();
  await expect(page.getByText("綠勾只在驗證後")).toBeVisible();
  // 玩耍設定與 Roll Call（現在大家在做什麼）只在桌面版（prefs 走 Tauri）；瀏覽器檢視必須誠實說明，
  // 不得用預設值冒充角色視窗的回報。
  await expect(page.getByText(/桌面角色設定需要桌面版控制中心/).first()).toBeVisible();
  await expect(page.getByText("現在大家在做什麼")).toHaveCount(0);
  // v0.5 單一主人：主動對話與主動程度／安靜時段住在小樞頁。
  await expect(page.getByText("主動式對話")).toBeVisible();
  await expect(page.getByText("主動程度與安靜時段")).toBeVisible();
  // 技術細節（Pack 詳情／Behavior State／rig 分層）不在一般模式外洩。
  await expect(page.getByText("Character Pack 詳情")).toHaveCount(0);
  await expect(page.getByText("現在的 Behavior State")).toHaveCount(0);
  await expect(page.getByText("技術資料", { exact: true })).toHaveCount(0);
});

test("工作：task-first 交代一件工作；開始前預覽有授權語意；agent 發現誠實顯示", async ({ page }) => {
  await open(page);
  await page.getByRole("navigation", { name: "主要導覽" }).getByText("工作", { exact: true }).click();
  // 一般模式沒有「建立工作階段」對話框：直接是 composer；空白時不能開始。
  await expect(page.getByLabel(/幫你做什麼/)).toBeVisible({ timeout: 15_000 });
  await expect(page.getByRole("button", { name: "建立工作階段…" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "開始", exact: true })).toBeDisabled();
  // 開始前預覽＝Consent Sheet 語意：Agent／讀取範圍／是否寫入／工具／上限／如何取消。
  const preview = page.getByRole("group", { name: "開始前預覽" });
  await expect(preview).toBeVisible();
  for (const term of ["使用哪個 Agent", "讀取範圍", "是否寫入", "工具", "時間、訊息與費用上限", "如何取消"]) {
    await expect(preview.getByText(term, { exact: true })).toBeVisible();
  }
  await expect(preview.getByText(/只讀取|不修改/).first()).toBeVisible();
  await expect(preview.getByText(/緊急停止會立刻終止/)).toBeVisible();
  // 發現結果卡片收在「工作設定」（真 daemon 的偵測；本機跑時可能是 fixture agent，
  // 狀態誠實即可——CI 上未安裝就顯示未安裝）。
  await page.getByText("工作設定：本機 AI Agent 與分工").click();
  await expect(page.getByText(/Codex|Claude Code/).first()).toBeVisible({ timeout: 15_000 });
  await expect(page.getByText(/系統不讀取、不保存它們的登入憑證/)).toBeVisible();
});

test("更多：五個入口；角色與整合管理指路到角色頁與連接與權限", async ({ page }) => {
  await open(page);
  await page.getByRole("navigation", { name: "主要導覽" }).getByText("更多", { exact: true }).click();
  const tabs = page.getByRole("tablist", { name: "更多分類" });
  for (const label of ["記憶與知識", "活動歷史", "設定", "角色與整合管理", "進階功能"]) {
    await expect(tabs.getByRole("tab", { name: label })).toBeVisible();
  }
  await tabs.getByRole("tab", { name: "角色與整合管理" }).click();
  await expect(page.getByText(/目前角色：/)).toBeVisible();
  await page.getByRole("button", { name: /管理裝置與整合/ }).click();
  await expect(page.locator(".topbar-title")).toHaveText("連接與權限");
});

test("更多：記憶與知識一般模式只有三區；技術細節不外洩", async ({ page }) => {
  await open(page);
  await page.getByRole("navigation", { name: "主要導覽" }).getByText("更多", { exact: true }).click();
  await expect(page.getByText("沒有你不能刪除的記憶")).toBeVisible();
  // spec §11：一般模式只顯示「關於我的記憶／小樞學會的知識／素材與來源」。
  const group = page.getByRole("tablist", { name: "記憶與知識分類" });
  await expect(group.getByRole("tab")).toHaveCount(3);
  await group.getByRole("tab", { name: "小樞學會的知識" }).click();
  await expect(page.getByText(/要你確認過才會被採用/)).toBeVisible();
  // 知識收據／Context Bundle 預覽／候選複審屬技術細節，只在進階模式出現。
  await expect(page.getByRole("tab", { name: "提供給 AI 的內容" })).toHaveCount(0);
  await expect(page.getByRole("tab", { name: "知識收據" })).toHaveCount(0);
  await expect(page.getByText("Context Bundle")).toHaveCount(0);
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
  await expect(page.getByText("關於我的記憶").first()).toBeVisible({ timeout: 10_000 });
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

test("進階模式：更多 → 進階功能切換後顯示技術頁面", async ({ page }) => {
  await open(page);
  await page.getByRole("navigation", { name: "主要導覽" }).getByText("更多", { exact: true }).click();
  // v0.5：顯示模式切換唯一的家是「進階功能」分頁（設定頁只指路）。
  await page.getByRole("tab", { name: "進階功能" }).click();
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

// 放在緊急停止之後（serial 群組裡一個失敗會讓其後全部 skip；這一項目前在 vite dev
// server 下會失敗——見測試內註解——不該連帶讓其餘回歸被跳過。它只導頁、不改後端狀態。）
test("現在：交代一件事會把描述帶到工作頁", async ({ page }) => {
  // 已知缺陷（產品端，非測試端）：src/pages/work/TaskComposer.tsx:279 在 useState 初始化器
  // 裡讀取並移除 sessionStorage 的 work.prefill；React StrictMode（開發模式）會把初始化器
  // 呼叫兩次並採用第二次結果，所以本機 `pnpm test:e2e`（vite dev）下描述會被消費掉卻沒落在
  // composer；CI（vite preview＝production build）與 Tauri build 不受影響。這裡不放寬斷言。
  await open(page);
  await expect(page.getByRole("button", { name: "交代一件事" })).toBeVisible({ timeout: 15_000 });
  await page.getByLabel(/幫你做什麼/).fill("整理下載資料夾");
  await page.getByRole("button", { name: "交代一件事" }).click();
  await expect(page.locator(".topbar-title")).toHaveText("工作");
  // task-first 工作頁掛載時讀取並清除 work.prefill：描述必須真的落在 composer 裡，
  // 且鍵已被消費（不會下次再莫名預填）。
  await expect(page.getByLabel(/幫你做什麼/)).toHaveValue("整理下載資料夾");
  const prefill = await page.evaluate(() => sessionStorage.getItem("work.prefill"));
  expect(prefill).toBeNull();
});
