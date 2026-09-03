// 使用者任務：電腦上沒有裝 Agent 時，「工作」頁必須先擋下來並說清楚原因，
// 不是讓人按了「開始」才失敗。
//
// Agent discovery 只在 daemon 啟動／重新偵測時讀 INTERACT_AI_*_BIN，所以這一支
// 另外起一支**真** daemon（隔離的家與埠號），把兩個 Agent 都指向不存在的路徑。

import { test, expect } from "@playwright/test";
import { api, appUrl, DESKTOP, navigateTo, openApp, PAGES, spawnDaemon } from "./helpers";
import type { SpawnedDaemon } from "./helpers";

test.describe.configure({ mode: "serial" });

const PORT = 18793;
let daemon: SpawnedDaemon | null = null;

test.beforeAll(async () => {
  daemon = await spawnDaemon({
    port: PORT,
    label: "no-agents",
    env: {
      INTERACT_AI_CODEX_BIN: "/nonexistent/codex",
      INTERACT_AI_CLAUDE_BIN: "/nonexistent/claude",
    },
  });
});

test.afterAll(async () => {
  daemon?.kill();
  daemon = null;
});

test("工作：Agent 未安裝時「開始」按不下去，而且說得出原因", async ({ page, request }) => {
  test.setTimeout(120_000);
  const target = daemon!;
  // 後端事實：discovery 誠實回報找不到（不是介面自己猜的）。
  const discoveries = (await api(request, "GET", "/v1/agents", undefined, {
    base: target.api,
    token: target.token,
  })) as { agents?: { kind?: string; found?: boolean }[] };
  expect(discoveries.agents?.length ?? 0).toBeGreaterThan(0);
  for (const agent of discoveries.agents ?? []) {
    expect(agent.found, `${agent.kind} 應該是找不到`).toBe(false);
  }

  await page.setViewportSize(DESKTOP);
  await openApp(page, appUrl(target.api, target.token));
  await navigateTo(page, PAGES[2], false);

  const task = page.getByLabel(/幫你做什麼/);
  await expect(task).toBeVisible({ timeout: 15_000 });
  await task.fill("幫我整理這份會議記錄");
  // 有內容也不能開始：先擋下來並說原因。
  const start = page.getByRole("button", { name: "開始", exact: true });
  await expect(start).toBeDisabled();
  await expect(page.getByText(/電腦上找不到這個 Agent；請先安裝並登入/)).toBeVisible();

  // 技術細節裡的 Agent 狀態也是「未安裝」（同一份判斷，不是兩套說法）。
  const preview = page.getByRole("group", { name: "開始前預覽" });
  await preview.locator("details.tech-details").getByText("查看技術細節").click();
  await expect(preview.getByText("未安裝", { exact: true })).toBeVisible();

  // 「工作設定」裡的偵測結果同樣誠實。
  await page.getByText("工作設定：本機 AI Agent 與分工").click();
  const cards = page.locator(".provider-card", { hasText: "Claude Code" });
  await expect(cards.first().getByText("未安裝", { exact: true })).toBeVisible({ timeout: 15_000 });
});

test("工作：未安裝時後端也不會偷偷建立工作階段", async ({ request }) => {
  test.setTimeout(60_000);
  const target = daemon!;
  const sessions = (await api(request, "GET", "/v1/agent-sessions", undefined, {
    base: target.api,
    token: target.token,
  })) as unknown[];
  expect(sessions).toEqual([]);
});
