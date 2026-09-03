// 使用者任務：打開「現在」就看懂三件事——角色現在怎麼樣、正在做什麼、有什麼要我決定。
//
// 每一種後端狀態（沒有工作／處理中／等你允許／對方說已完成／已人工驗證／
// 結果不確定／已取消／緊急停止中）都用真 daemon 造出來，再對照畫面上的那三張卡，
// 桌面與 390px 都看一次。誠實重點：首頁永遠不會把「Agent 的說法」升級成綠勾，
// 終局狀態（結果不確定／已取消）不算「進行中」。

import { test, expect, Page } from "@playwright/test";
import {
  api,
  apiBase,
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

const workRoot = makeWorkRoot("interaction-e2e-home-");
const createdLabels: string[] = [];

test.beforeEach(() => {
  test.skip(
    process.env.E2E_FAKE_AGENTS !== "1",
    "需要 fixture agent（global-setup 預設啟用；E2E_REAL_AGENTS=1 時略過）"
  );
});

test.afterAll(async () => {
  await closeAllOpenSessions();
});

/** 收尾／前置：把所有還開著的工作階段關掉，讓「沒有進行中」是真的。 */
async function closeAllOpenSessions(): Promise<void> {
  try {
    const res = await fetch(`${apiBase()}/v1/agent-sessions`, {
      headers: { Authorization: `Bearer ${process.env.E2E_TOKEN!}` },
    });
    if (!res.ok) return;
    const list = (await res.json()) as { sessionId: string; state: string }[];
    await closeSessions(
      list
        .filter((s) => !["closed", "cancelled", "expired", "unknown", "failed"].includes(s.state))
        .map((s) => s.sessionId)
    );
  } catch {
    /* 收尾失敗不讓測試變紅 */
  }
}

/** 首頁三張卡都在，而且「待我決定」與後端 inbox 的 pendingCount 一致。 */
async function assertThreeAnswers(
  page: Page,
  request: import("@playwright/test").APIRequestContext
) {
  for (const id of ["now-character", "now-work", "now-decisions"]) {
    await expect(page.getByTestId(id)).toBeVisible();
  }
  const inbox = (await api(request, "GET", "/v1/activity/inbox?limit=5")) as {
    pendingCount?: number;
  };
  const pending = Number(inbox.pendingCount ?? 0);
  const decisions = page.getByTestId("now-decisions");
  await expect(decisions.getByText(pending > 0 ? `${pending} 項` : "0 項")).toBeVisible({
    timeout: 15_000,
  });
}

test("現在：沒有工作時，三個回答誠實說「沒有進行中」", async ({ page, request }) => {
  test.setTimeout(90_000);
  await closeAllOpenSessions();
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  const work = page.getByTestId("now-work");
  await expect(work.getByText("沒有進行中")).toBeVisible({ timeout: 20_000 });
  // 瀏覽器檢視沒有角色視窗：固定的可信文字，不是角色文案。
  await expect(page.getByTestId("now-character").getByText("角色離線，改用文字。")).toBeVisible();
  await assertThreeAnswers(page, request);
  // 系統狀態仍然收在「詳細狀態」裡（第一屏只回答三件事）。
  await expect(page.getByText("系統狀態", { exact: true })).toHaveCount(0);

  await page.setViewportSize(NARROW);
  await expect(work.getByText("沒有進行中")).toBeVisible();
  await assertThreeAnswers(page, request);
});

test("現在：處理中／等你允許／對方說已完成各自用人話出現，驗證過也不會在首頁變綠勾", async ({
  page,
  request,
}) => {
  test.setTimeout(180_000);
  await closeAllOpenSessions();
  const working = "首頁狀態一（fixture Codex 開了一個 turn）";
  const consent = "首頁狀態二（fixture Codex 等人裁決）";
  const claimed = "首頁狀態三（fixture Claude 聲稱做完）";
  createdLabels.push(working, consent, claimed);
  const idWorking = await createFixtureSession(request, {
    agentId: "codex",
    label: working,
    workdir: makeWorkdir(workRoot, "working", "turns"),
  });
  const idConsent = await createFixtureSession(request, {
    agentId: "codex",
    label: consent,
    workdir: makeWorkdir(workRoot, "consent"),
  });
  const idClaimed = await createFixtureSession(request, {
    agentId: "claude-code",
    label: claimed,
    workdir: makeWorkdir(workRoot, "claimed"),
  });
  await waitSessionState(request, idWorking, ["active"]);
  await waitSessionState(request, idConsent, ["waiting-for-consent"]);
  await waitSessionState(request, idClaimed, ["claimed-completed"]);

  await page.setViewportSize(DESKTOP);
  await openApp(page);
  const work = page.getByTestId("now-work");
  await expect(work.getByText("3 個工作階段")).toBeVisible({ timeout: 20_000 });
  for (const [label, badge] of [
    [working, "處理中"],
    [consent, "等你允許"],
    [claimed, "對方說已完成"],
  ] as const) {
    const row = work.locator("li", { hasText: label });
    await expect(row).toBeVisible();
    await expect(row.locator(".badge")).toHaveText(badge);
  }
  await assertThreeAnswers(page, request);

  // 人工驗證之後：後端多了 humanVerified，首頁仍然只說「對方說已完成」——
  // 綠勾是工作頁那張卡的事，首頁不搶著宣稱。
  const verified = (await api(request, "POST", `/v1/agent-sessions/${idClaimed}/verify`, {
    note: "首頁狀態驗收：人工確認",
  })) as { state: string; humanVerified?: unknown };
  expect(verified.state).toBe("claimed-completed");
  expect(verified.humanVerified).toBeTruthy();
  await page.reload();
  await expect(work.getByText("3 個工作階段")).toBeVisible({ timeout: 20_000 });
  await expect(work.getByText("✓")).toHaveCount(0);
  await expect(work.locator("li", { hasText: claimed }).locator(".badge")).toHaveText(
    "對方說已完成"
  );

  await page.setViewportSize(NARROW);
  await expect(work.getByText("3 個工作階段")).toBeVisible();
  await expect(work.locator("li", { hasText: working }).locator(".badge")).toHaveText("處理中");
});

test("現在：結果不確定與已取消都是終局——不算進行中，也不會被講成完成", async ({
  page,
  request,
}) => {
  test.setTimeout(180_000);
  const unknown = "首頁狀態四（fixture Claude 一聲不響地結束）";
  const cancelled = "首頁狀態五（被人中斷）";
  createdLabels.push(unknown, cancelled);
  const idUnknown = await createFixtureSession(request, {
    agentId: "claude-code",
    label: unknown,
    workdir: makeWorkdir(workRoot, "unknown", "silent"),
    // fixture 一啟動就結束（結果未知），mailbox 立刻關閉：不送任務。
    task: null,
  });
  const idCancelled = await createFixtureSession(request, {
    agentId: "codex",
    label: cancelled,
    workdir: makeWorkdir(workRoot, "cancelled", "turns"),
  });
  await waitSessionState(request, idUnknown, ["unknown"], 45_000);
  await waitSessionState(request, idCancelled, ["active"]);
  await api(request, "POST", `/v1/agent-sessions/${idCancelled}/interrupt`);
  await waitSessionState(request, idCancelled, ["cancelled"], 45_000);
  // 其餘進行中的關掉，剩下的兩筆都是終局。
  await closeAllOpenSessions();

  await page.setViewportSize(DESKTOP);
  await openApp(page);
  const work = page.getByTestId("now-work");
  await expect(work.getByText("沒有進行中")).toBeVisible({ timeout: 20_000 });
  await expect(work.getByText(unknown)).toHaveCount(0);
  await expect(work.getByText(cancelled)).toHaveCount(0);
  // 後端仍然保有這兩筆真實紀錄（首頁只是不把它們算成「在跑」）。
  const list = (await api(request, "GET", "/v1/agent-sessions")) as {
    sessionId: string;
    state: string;
  }[];
  expect(list.find((s) => s.sessionId === idUnknown)?.state).toBe("unknown");
  expect(list.find((s) => s.sessionId === idCancelled)?.state).toBe("cancelled");
  await assertThreeAnswers(page, request);

  await page.setViewportSize(NARROW);
  await expect(work.getByText("沒有進行中")).toBeVisible();
});

test("現在：緊急停止中，角色那一句換成固定的安全文字（解除要走安全流程）", async ({
  page,
  request,
}) => {
  test.setTimeout(120_000);
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  try {
    await api(request, "POST", "/v1/emergency-stop", { reason: "home-state e2e" });
    const status = (await api(request, "GET", "/v1/status")) as { emergencyStop?: boolean };
    expect(status.emergencyStop).toBe(true);
    await page.reload();
    await navigateTo(page, PAGES[0], false);
    await expect(page.getByTestId("now-character").getByText(/緊急停止中：.*已停止所有回應。/)).toBeVisible(
      { timeout: 20_000 }
    );
    // 首頁只給「前往解除」，沒有任何一鍵解除。
    await expect(
      page.locator(".home").getByRole("button", { name: /緊急停止中 — 前往解除/ })
    ).toBeVisible();
    await expect(
      page.locator(".home").getByRole("button", { name: "緊急停止", exact: true })
    ).toHaveCount(0);
    await assertThreeAnswers(page, request);

    await page.setViewportSize(NARROW);
    await expect(page.getByTestId("now-character").getByText(/緊急停止中/)).toBeVisible();
  } finally {
    await api(request, "POST", "/v1/emergency-stop/clear");
  }
  const cleared = (await api(request, "GET", "/v1/status")) as { emergencyStop?: boolean };
  expect(cleared.emergencyStop).toBe(false);
});
