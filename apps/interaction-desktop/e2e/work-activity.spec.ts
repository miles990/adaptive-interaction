// 使用者任務：一件工作失敗了，我看得懂它**經過了什麼**、為什麼失敗、接下來
// 該做什麼——而且不必懂任何技術名詞。
//
// 全部打 global-setup 起的**真** daemon：session、子程序、狀態機、追蹤紀錄
// 都是真的，agent 本體是 fixture 子程序
// （crates/interaction-runtime/tests/fixtures/fake_claude.sh，`crash` 模式＝
// 往 stderr 寫一行再以非零狀態結束，所以結局誠實地是 failed，而且會留下一筆
// 診斷紀錄）。

import { test, expect, Page } from "@playwright/test";
import {
  api,
  apiBase,
  appUrl,
  closeSessions,
  createFixtureSession,
  DESKTOP,
  makeWorkdir,
  makeWorkRoot,
  NARROW,
  navigateTo,
  openApp,
  PAGES,
  waitSessionState,
} from "./helpers";

test.describe.configure({ mode: "serial" });

const WORK = PAGES[2];
const workRoot = makeWorkRoot("interaction-e2e-activity-");
const created: string[] = [];

test.beforeEach(() => {
  test.skip(
    process.env.E2E_FAKE_AGENTS !== "1",
    "需要 fixture agent（global-setup 預設啟用；E2E_REAL_AGENTS=1 時略過）"
  );
});

test.afterAll(async () => {
  await closeSessions(created);
});

/** 回到工作頁重新讀一次狀態（不靠上一個測試留下的畫面）。 */
async function reopenWork(page: Page, narrow = false) {
  await page.setViewportSize(narrow ? NARROW : DESKTOP);
  await page.goto(appUrl());
  await expect(page.getByRole("navigation", { name: "主要導覽" })).toBeVisible({ timeout: 20_000 });
  await navigateTo(page, WORK, narrow);
}

/** 展開某個 label 的工作卡片，回傳「這件工作的經過」區塊。 */
async function activitySection(page: Page, label: string) {
  const card = page.locator(".provider-card", { hasText: label });
  await expect(card).toBeVisible({ timeout: 20_000 });
  await card.scrollIntoViewIfNeeded();
  await card.getByRole("button", { name: "查看結果／訊息" }).click();
  const section = card.getByRole("region", { name: "這件工作的經過" });
  await expect(section).toBeVisible({ timeout: 20_000 });
  return section;
}

test("工作：失敗的工作說得出經過、原因與下一步（一般模式全是人話）", async ({ page, request }) => {
  test.setTimeout(120_000);
  const label = "這件工作會失敗";
  const sessionId = await createFixtureSession(request, {
    agentId: "claude-code",
    label,
    workdir: makeWorkdir(workRoot, "activity-failed", "crash"),
    // `crash` 在 spawn 當下就寫 stderr 並以非零狀態結束：mailbox 在 POST
    // 任務之前就關了（409）。這個 fixture 不需要任務就會走到 failed。
    task: null,
  });
  created.push(sessionId);
  await waitSessionState(request, sessionId, ["failed"], 45_000);

  await openApp(page);
  await reopenWork(page);
  const section = await activitySection(page, label);

  // ① 發生了什麼 ② 目前狀態 ③ 失敗原因 ④ 下一步。
  await expect(section.getByText(/^目前狀態：/)).toBeVisible();
  await expect(section.getByText(/^失敗原因：/)).toBeVisible();
  await expect(section.getByText(/^下一步：/)).toBeVisible();
  // ⑤ 時間線是有序清單，而且每一步都是後端投影好的人話。
  const timeline = section.getByRole("list");
  await expect(timeline).toBeVisible();
  expect(await timeline.getByRole("listitem").count()).toBeGreaterThan(0);

  // 後端確實留下了診斷紀錄（stderr），但一般模式只說「有診斷輸出」。
  const trace = (await api(
    request,
    "GET",
    `/v1/trace?sessionId=${sessionId}&class=diagnostic&limit=50`
  )) as { items: Record<string, unknown>[] };
  expect(trace.items.length, "crash 模式必須留下 stderr 診斷紀錄").toBeGreaterThan(0);

  // 一般模式不得出現任何紀錄層術語、原始識別碼或 stderr 內容。
  const text = (await section.textContent()) ?? "";
  for (const banned of ["trace", "Trace", "stderr", "SSE", "provider", "lease", "agent-session."]) {
    expect(text, `一般模式不得出現「${banned}」`).not.toContain(banned);
  }
  expect(text).not.toMatch(/[0-9a-f]{8}-[0-9a-f]{4}-/i);
  // 一般模式沒有可展開的技術詳情（`<details>` 只在進階模式畫出來）。
  // 比對元素而不是字串：畫面上本來就不該出現「技術詳情」這四個字，
  // 但這一條釘住的是「連可展開的東西都沒有」。
  await expect(section.locator("details")).toHaveCount(0);
});

test("工作：窄螢幕（390px）看得到經過；紀錄夠多時「載入更早」鍵盤到得了", async ({
  page,
  request,
}) => {
  test.setTimeout(180_000);
  const label = "這件工作留下很多紀錄";
  const sessionId = await createFixtureSession(request, {
    agentId: "claude-code",
    label,
    workdir: makeWorkdir(workRoot, "activity-paging"),
    task: "第 1 句",
  });
  created.push(sessionId);

  // 一頁 20 筆：多交代幾輪，讓這個 session 真的有超過一頁的紀錄可以往回翻。
  // 送出＝進 mailbox；fixture 每一輪都會回一個 result，所以每一輪都留下
  // 「任務已送達」與一筆終態紀錄。
  const traceCount = async () => {
    const page1 = (await api(request, "GET", `/v1/trace?sessionId=${sessionId}&limit=100`)) as {
      items: unknown[];
    };
    return page1.items.length;
  };
  for (let turn = 2; turn <= 16 && (await traceCount()) <= 20; turn += 1) {
    await api(request, "POST", `/v1/agent-sessions/${sessionId}/messages`, {
      kind: "task",
      body: { task: `第 ${turn} 句` },
    });
    await new Promise((r) => setTimeout(r, 250));
  }
  const total = await traceCount();

  await openApp(page);
  await reopenWork(page, true);
  const section = await activitySection(page, label);
  // 窄螢幕：區塊與時間線都看得到，而且整頁不橫向捲動。
  await expect(section.getByRole("list")).toBeVisible();
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth
  );
  expect(overflow, "390px 下不得出現橫向捲動").toBeLessThanOrEqual(1);

  // 前提要成立才驗得到分頁。不成立就直接失敗（而不是悄悄跳過這一段）：
  // 一個「有時候什麼都沒驗」的測試比沒有測試更糟。
  expect(
    total,
    "設定不成立：這個 session 必須留下超過一頁（20 筆）的紀錄，才看得到「載入更早」"
  ).toBeGreaterThan(20);

  // 鍵盤：真的是 <button>，focus 得了、Enter 按得動，而且真的載入更早的紀錄。
  const older = section.getByRole("button", { name: "載入更早" });
  await expect(older).toBeVisible();
  const before = await section.getByRole("listitem").count();
  await older.focus();
  await expect(older).toBeFocused();
  await page.keyboard.press("Enter");
  await expect
    .poll(async () => section.getByRole("listitem").count(), { timeout: 20_000 })
    .toBeGreaterThan(before);
});
