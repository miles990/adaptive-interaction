// 使用者任務：在「工作」頁交代一件唯讀工作，然後看得懂它現在怎麼樣、停得掉、
// 分得清「Agent 說完成」與「我確認完成」、以及「結果不確定」。
//
// 每一條都用**真 daemon 的 API** 對照畫面（不只看有沒有那個字），
// agent 本體是 fixture 子程序（crates/interaction-runtime/tests/fixtures/fake_*.sh），
// 狀態機、mailbox、lease、人工 verify 全是真的。
//
// 曾經的缺陷（Runtime，已修）：中斷之後 Runtime 把工作取消了，卻在 receptor ingest
// 失敗時沒有發出 `agent.session.state` 事件，畫面在重新載入前停在舊狀態。
// Runtime 現在先發狀態事件再做 ingest（回歸測試在 crates/interaction-runtime/tests/
// gateway_loop.rs），所以下面「中斷之後畫面自己更新」是正常斷言，不再 test.fail()。

import { test, expect, Page } from "@playwright/test";
import * as fs from "node:fs";
import * as path from "node:path";
import {
  api,
  apiBase,
  appUrl,
  closeSessions,
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
const workRoot = makeWorkRoot();
/** 這支 spec 交代過的工作名稱；收尾時把還開著的關掉（fixture 子程序也跟著收）。 */
const createdLabels: string[] = [];

test.beforeEach(() => {
  test.skip(
    process.env.E2E_FAKE_AGENTS !== "1",
    "需要 fixture agent（global-setup 預設啟用；E2E_REAL_AGENTS=1 時略過）"
  );
});

test.afterAll(async () => {
  try {
    const res = await fetch(`${apiBase()}/v1/agent-sessions`, {
      headers: { Authorization: `Bearer ${process.env.E2E_TOKEN!}` },
    });
    if (!res.ok) return;
    const list = (await res.json()) as { sessionId: string; state: string; label?: string }[];
    await closeSessions(
      list
        .filter((s) => createdLabels.includes(String(s.label ?? "")))
        .filter((s) => !["closed", "cancelled", "expired"].includes(s.state))
        .map((s) => s.sessionId)
    );
  } catch {
    /* 收尾失敗不讓測試變紅 */
  }
});

/** GET /v1/agent-sessions 找出這個 label 的那一筆（找不到就讓測試失敗）。 */
async function sessionByLabel(
  request: import("@playwright/test").APIRequestContext,
  label: string
): Promise<Record<string, unknown>> {
  const deadline = Date.now() + 15_000;
  for (;;) {
    const list = (await api(request, "GET", "/v1/agent-sessions")) as Record<string, unknown>[];
    const found = list.find((s) => String(s.label ?? "") === label);
    if (found) return found;
    if (Date.now() > deadline) {
      throw new Error(
        `建立後 15 秒內找不到 label 為「${label}」的工作階段：${JSON.stringify(
          list.map((s) => s.label)
        )}`
      );
    }
    await new Promise((r) => setTimeout(r, 250));
  }
}

/** 使用者回到工作頁再看一次（重新打開控制中心；狀態一律重讀）。 */
async function reopenWork(page: Page) {
  await page.goto(appUrl());
  await expect(page.getByRole("navigation", { name: "主要導覽" })).toBeVisible({ timeout: 20_000 });
  await navigateTo(page, WORK, false);
}

/** 在 composer 裡交代一件工作並按「開始」；回傳 label。 */
async function delegate(
  page: Page,
  input: { task: string; workdir: string; kind: string }
): Promise<string> {
  await navigateTo(page, WORK, false);
  const task = page.getByLabel(/幫你做什麼/);
  await expect(task).toBeVisible({ timeout: 15_000 });
  await task.fill(input.task);
  await page.getByLabel("加入檔案或選擇資料夾").fill(input.workdir);
  await page.getByLabel("這是哪一種工作").selectOption({ label: input.kind });
  const start = page.getByRole("button", { name: "開始", exact: true });
  await expect(start).toBeEnabled({ timeout: 10_000 });
  await start.click();
  const label = input.task.split("\n")[0].trim();
  createdLabels.push(label);
  return label;
}

test("工作：在 composer 交代一件唯讀工作 → 真的建立成唯讀 session，卡片說「處理中」", async ({
  page,
  request,
}) => {
  test.setTimeout(120_000);
  const dir = makeWorkdir(workRoot, "delegate-readonly", "turns");
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  await navigateTo(page, WORK, false);

  // 開始前預覽照實回答三件事：讀哪裡、會不會改、上限多少。
  const label = "看一下這個資料夾的測試有沒有壞掉，只要告訴我結果";
  await page.getByLabel(/幫你做什麼/).fill(label);
  await page.getByLabel("加入檔案或選擇資料夾").fill(dir);
  await page.getByLabel("這是哪一種工作").selectOption({ label: "程式工作" });
  const preview = page.getByRole("group", { name: "開始前預覽" });
  await expect(
    preview.getByText(new RegExp(`你選擇的資料夾（${path.basename(dir)}）`))
  ).toBeVisible();
  await expect(preview.getByText(/不會修改：這次只看不改/)).toBeVisible();

  await page.getByRole("button", { name: "開始", exact: true }).click();
  createdLabels.push(label);
  await expect(page.getByText(/已交給 Codex：/)).toBeVisible({ timeout: 20_000 });

  // 後端事實：唯讀、資料範圍就是那個資料夾、沒有工具授權。
  const record = await sessionByLabel(request, label);
  expect(record.allowWrite).toBe(false);
  expect(record.dataScope).toEqual([`workspace:${dir}`]);
  expect(record.toolScope).toEqual([]);
  expect(record.consentScope).toEqual([]);
  expect(String(record.agentId)).toBe("codex");

  // 畫面事實：卡片說「處理中」（fake_codex `turns` 開了 turn 就不再說話）。
  const sessionId = String(record.sessionId);
  await waitSessionState(request, sessionId, ["active"]);
  await reopenWork(page);
  const card = page.locator(".provider-card", { hasText: label });
  await expect(card).toBeVisible({ timeout: 20_000 });
  await expect(card.getByText("處理中", { exact: true })).toBeVisible({ timeout: 20_000 });
  await expect(card.getByText(/只讀取，不修改/)).toBeVisible();

  // 390px：同一張卡、同一句話（不是桌面限定）。
  await page.setViewportSize(NARROW);
  await card.scrollIntoViewIfNeeded();
  await expect(card.getByText("處理中", { exact: true })).toBeVisible();
});

test("工作：暫停／中斷會真的停下來（後端 cancelled → 回到工作頁看到「已取消」，首頁不再算它進行中）", async ({
  page,
  request,
}) => {
  test.setTimeout(120_000);
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  const label = "先跑一下再讓我中斷它";
  await delegate(page, {
    task: label,
    workdir: makeWorkdir(workRoot, "cancel", "turns"),
    kind: "程式工作",
  });
  const record = await sessionByLabel(request, label);
  const sessionId = String(record.sessionId);
  await waitSessionState(request, sessionId, ["active"]);

  await reopenWork(page);
  const card = page.locator(".provider-card", { hasText: label });
  await expect(card).toBeVisible({ timeout: 20_000 });
  await card.getByRole("button", { name: "暫停／中斷目前工作" }).click();
  await expect(page.getByText("已送出中斷指令。")).toBeVisible({ timeout: 15_000 });

  // 後端事實：真的取消了（不是只顯示了一句話）。
  await waitSessionState(request, sessionId, ["cancelled"], 30_000);

  // 使用者回到工作頁：卡片說「已取消」，而且不再提供中斷／續租。
  await reopenWork(page);
  await expect(card.getByText("已取消", { exact: true })).toBeVisible({ timeout: 20_000 });
  await expect(card.getByRole("button", { name: "暫停／中斷目前工作" })).toHaveCount(0);

  // 首頁「進行中的工作」不再把它算進去（is_open 的定義只有一份）。
  await navigateTo(page, PAGES[0], false);
  const now = page.getByTestId("now-work");
  await expect(now).toBeVisible();
  await expect(now.getByText(label)).toHaveCount(0);
});

// regression（Runtime `report_agent_session`）：interrupt 之後 state 變成 cancelled
// 的同時，`agent.session.state` 事件也必須發得出去。舊版把 receptor ingest 排在事件
// 前面並用 `?` 提早返回，觀察管線一失敗事件就整個靜默（SSE 從中斷前的序號重放只停在
// working），畫面因此停在「處理中」直到重新載入。這一支盯的就是「不重新載入」。
test("工作：中斷之後畫面自己更新（不重新載入也要從「處理中」變成「已取消」）", async ({
  page,
  request,
}) => {
  test.setTimeout(120_000);
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  const label = "中斷之後畫面要自己更新";
  await delegate(page, {
    task: label,
    workdir: makeWorkdir(workRoot, "cancel-live", "turns"),
    kind: "程式工作",
  });
  const record = await sessionByLabel(request, label);
  const sessionId = String(record.sessionId);
  await waitSessionState(request, sessionId, ["active"]);
  await reopenWork(page);
  const card = page.locator(".provider-card", { hasText: label });
  await expect(card.getByText("處理中", { exact: true })).toBeVisible({ timeout: 20_000 });
  await card.getByRole("button", { name: "暫停／中斷目前工作" }).click();
  await waitSessionState(request, sessionId, ["cancelled"], 30_000);
  // 不重新載入：使用者盯著畫面，狀態必須自己更新。
  await expect(card.getByText("已取消", { exact: true })).toBeVisible({ timeout: 15_000 });
});

test("工作：關閉一個工作階段只敢說「已要求終止子程序」", async ({ page, request }) => {
  test.setTimeout(120_000);
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  const label = "這件事我改變主意了，直接關掉";
  await delegate(page, {
    task: label,
    workdir: makeWorkdir(workRoot, "close", "turns"),
    kind: "程式工作",
  });
  const record = await sessionByLabel(request, label);
  const sessionId = String(record.sessionId);
  await waitSessionState(request, sessionId, ["active"]);

  await reopenWork(page);
  const card = page.locator(".provider-card", { hasText: label });
  await card.getByRole("button", { name: "關閉", exact: true }).click();
  await expect(page.getByText("工作階段已關閉（已要求終止子程序）。")).toBeVisible({
    timeout: 15_000,
  });
  const after = (await api(request, "GET", `/v1/agent-sessions/${sessionId}`)) as {
    state: string;
  };
  expect(["closed", "cancelled"]).toContain(after.state);
});

test("工作：對方說完成 ≠ 已由你確認——綠勾只有按下「標記為已驗證」才會出現", async ({
  page,
  request,
}) => {
  test.setTimeout(120_000);
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  const label = "幫我把這份筆記歸納成三點";
  await delegate(page, {
    task: label,
    workdir: makeWorkdir(workRoot, "claimed"),
    kind: "一般對話與文件",
  });
  const record = await sessionByLabel(request, label);
  const sessionId = String(record.sessionId);
  await waitSessionState(request, sessionId, ["claimed-completed"]);

  await reopenWork(page);
  const card = page.locator(".provider-card", { hasText: label });
  await expect(card.getByText("對方說已完成", { exact: true })).toBeVisible({ timeout: 20_000 });
  await expect(card.getByText(/它的說法/)).toBeVisible();
  await expect(card.getByText(/✓/)).toHaveCount(0);

  // 人類按下按鈕（不是測試直接打 /verify）。
  await card.getByRole("button", { name: "標記為已驗證（我確認過結果）" }).click();
  await expect(page.getByText("已標記為已驗證（由你人工確認）。")).toBeVisible({ timeout: 15_000 });

  // 後端事實：state 仍然是 claimed-completed，多的是 humanVerified（誠實階梯）。
  const verified = (await api(request, "GET", `/v1/agent-sessions/${sessionId}`)) as {
    state: string;
    humanVerified?: unknown;
  };
  expect(verified.state).toBe("claimed-completed");
  expect(verified.humanVerified).toBeTruthy();

  await reopenWork(page);
  await expect(card.getByText(/✓ 已由你確認/)).toBeVisible({ timeout: 20_000 });
  await expect(card.getByText(/由你親自確認/)).toBeVisible();
  await expect(card.getByRole("button", { name: "標記為已驗證（我確認過結果）" })).toHaveCount(0);

  // 390px 也看得到同一組事實。
  await page.setViewportSize(NARROW);
  await card.scrollIntoViewIfNeeded();
  await expect(card.getByText(/✓ 已由你確認/)).toBeVisible();
});

test("工作：agent 沒說結果就結束＝結果不確定（不是成功也不是失敗，也沒有驗證鈕）", async ({
  page,
  request,
}) => {
  test.setTimeout(120_000);
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  const label = "這件事會沒有下文";
  await delegate(page, {
    task: label,
    workdir: makeWorkdir(workRoot, "unknown", "silent"),
    kind: "一般對話與文件",
  });
  const record = await sessionByLabel(request, label);
  const sessionId = String(record.sessionId);
  await waitSessionState(request, sessionId, ["unknown"], 45_000);

  await reopenWork(page);
  const card = page.locator(".provider-card", { hasText: label });
  await expect(card.getByText("結果不確定", { exact: true })).toBeVisible({ timeout: 20_000 });
  // 不確定不得被升級成「完成」，所以沒有驗證鈕；但仍然關得掉。
  await expect(card.getByRole("button", { name: "標記為已驗證（我確認過結果）" })).toHaveCount(0);
  await expect(card.getByRole("button", { name: "關閉", exact: true })).toBeVisible();

  await page.setViewportSize(NARROW);
  await card.scrollIntoViewIfNeeded();
  await expect(card.getByText("結果不確定", { exact: true })).toBeVisible();
});

test("工作：等你允許時按「拒絕」——畫面說「你已拒絕」，而且裁決真的送到 agent", async ({
  page,
  request,
}) => {
  test.setTimeout(120_000);
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  const dir = makeWorkdir(workRoot, "consent");
  const label = "這件事會問我要不要";
  await delegate(page, { task: label, workdir: dir, kind: "程式工作" });
  const record = await sessionByLabel(request, label);
  const sessionId = String(record.sessionId);
  await waitSessionState(request, sessionId, ["waiting-for-consent"]);

  await reopenWork(page);
  const card = page.locator(".provider-card", { hasText: label });
  await expect(card.getByText("等你允許", { exact: true })).toBeVisible({ timeout: 20_000 });
  await card.getByRole("button", { name: "查看結果／訊息" }).click();
  await expect(card.getByText("等待你核可")).toBeVisible({ timeout: 20_000 });
  await card.getByRole("button", { name: "拒絕", exact: true }).click();

  // 畫面：誰決定的要說清楚（逾時自動拒絕不是「你拒絕了」）。
  await expect(card.getByText(/你已拒絕/)).toBeVisible({ timeout: 20_000 });
  // 事實：裁決真的寫到 fixture agent 的 stdin（不是只記在畫面上）。
  const decisionFile = path.join(dir, "fake-approval-decision");
  const deadline = Date.now() + 15_000;
  while (!fs.existsSync(decisionFile)) {
    if (Date.now() > deadline) throw new Error("拒絕沒有送到 fixture agent（沒有 decision 檔）");
    await new Promise((r) => setTimeout(r, 200));
  }
  expect(fs.readFileSync(decisionFile, "utf8")).toContain("decision");
});
