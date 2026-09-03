// 使用者任務：一鍵讓一切停下來——而且要看得見它真的停了。
//
// 這一支放在整個 suite 的最後（playwright.config.ts 的 estop-last project）：
// 緊急停止會撤銷同意、取消進行中的工作、要求所有感測停止，不該污染別人的狀態。
//
// 在場的東西全部是真的：真 daemon、真 agent session（fixture 子程序）、
// 【模擬 iPhone（fixture）】（crates/interaction-runtime/examples/fake_iphone.rs）。
// iPhone 真機驗收仍為零。

import { test, expect, Page } from "@playwright/test";
import {
  api,
  beginPairing,
  closeSessions,
  createFixtureSession,
  DESKTOP,
  makeWorkdir,
  makeWorkRoot,
  NARROW,
  navigateTo,
  openApp,
  PAGES,
  revokePairedPhones,
  spawnFakeIphone,
  waitActiveSensors,
  waitSessionState,
  type FakeIphone,
} from "./helpers";

test.describe.configure({ mode: "serial" });

const MIC = "iphone.mic-level";
const workRoot = makeWorkRoot("interaction-e2e-estop-");
let phone: FakeIphone | null = null;
const sessions: string[] = [];

test.beforeEach(() => {
  test.skip(
    process.env.E2E_FAKE_AGENTS !== "1",
    "需要 fixture agent（global-setup 預設啟用；E2E_REAL_AGENTS=1 時略過）"
  );
});

test.afterAll(async () => {
  phone?.kill();
  phone = null;
  await revokePairedPhones();
  await closeSessions(sessions);
  await fetch(`${process.env.E2E_API!}/v1/emergency-stop/clear`, {
    method: "POST",
    headers: { Authorization: `Bearer ${process.env.E2E_TOKEN!}` },
  }).catch(() => undefined);
});

/** 一件在跑的工作 ＋ 一台正在感測的模擬手機。 */
async function arrangeBusySystem(
  request: import("@playwright/test").APIRequestContext,
  label: string
): Promise<{ sessionId: string; fixture: FakeIphone }> {
  const sessionId = await createFixtureSession(request, {
    agentId: "codex",
    label,
    workdir: makeWorkdir(workRoot, label.replace(/[^a-z0-9]+/gi, "-").toLowerCase(), "turns"),
  });
  sessions.push(sessionId);
  await waitSessionState(request, sessionId, ["active"]);
  const pairing = await beginPairing(request);
  const fixture = await spawnFakeIphone({ ...pairing, autoAckStopAll: true });
  await api(request, "PATCH", `/v1/receptors/${MIC}`, { enabled: true });
  fixture.send({ op: "status", micLevel: true });
  await waitActiveSensors(request, (list) => list.some((s) => s.kind === MIC));
  return { sessionId, fixture };
}

/** 走完安全解除流程（沒有任何一鍵解除；解除只有人做得到）。 */
async function clearThroughSafetyFlow(page: Page) {
  await page.setViewportSize(DESKTOP);
  await page.locator(".topbar").getByRole("button", { name: /緊急停止中 — 前往解除/ }).click();
  await page.getByRole("button", { name: /開始安全解除流程/ }).click();
  const dialog = page.getByRole("dialog", { name: "解除緊急停止" });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "我了解，解除緊急停止" }).click();
  await dialog.getByRole("button", { name: "確定解除？" }).click();
  await expect(
    page.locator(".topbar").getByRole("button", { name: "緊急停止", exact: true })
  ).toBeVisible({ timeout: 20_000 });
}

test("緊急停止：從「現在」的快速操作按下去——工作被取消、感測停止、手機也收到停止指令", async ({
  page,
  request,
}) => {
  test.setTimeout(240_000);
  const label = "緊急停止驗收：進行中的工作";
  const { sessionId, fixture } = await arrangeBusySystem(request, label);
  phone = fixture;

  await page.setViewportSize(DESKTOP);
  await openApp(page);
  const seen = fixture.events.length;
  const home = page.locator(".home");
  await home.getByRole("button", { name: "緊急停止", exact: true }).click();
  await home.getByRole("button", { name: "立即停止一切？" }).click();
  await expect(page.getByText("緊急停止已啟動").first()).toBeVisible({ timeout: 20_000 });

  // 後端事實 1：真的處於緊急停止。
  const status = (await api(request, "GET", "/v1/status")) as {
    emergencyStop?: boolean;
    activeSensors?: unknown[];
  };
  expect(status.emergencyStop).toBe(true);

  // 後端事實 2：進行中的工作被取消（不是只有畫面上停了）。
  await waitSessionState(request, sessionId, ["cancelled"], 30_000);

  // 後端事實 3：沒有任何感測還在跑。
  await waitActiveSensors(request, (list) => list.length === 0, 20_000);

  // 手機端事實：收到「連感測一起停」與「緊急停止」的角色投影。
  await fixture.waitForEvent((e) => e.event === "stop-all" && e.sensors === true, 20_000, seen);
  await fixture.waitForEvent(
    (e) =>
      e.event === "act" &&
      e.name === "character.present" &&
      (e.params as Record<string, unknown> | undefined)?.state === "emergency",
    20_000,
    seen
  );

  // 畫面事實：連接與權限那一頁用固定文字說「緊急停止中」，並指路到解除流程。
  await navigateTo(page, PAGES[3], false);
  const stopArea = page.getByTestId("connect-area-stop");
  await expect(stopArea.getByText("緊急停止中")).toBeVisible({ timeout: 20_000 });
  await expect(stopArea.getByRole("button", { name: "前往解除" })).toBeVisible();

  // 解除只有人做得到，而且要走安全流程。
  const cleared = fixture.events.length;
  await clearThroughSafetyFlow(page);
  const after = (await api(request, "GET", "/v1/status")) as {
    emergencyStop?: boolean;
    activeSensors?: unknown[];
  };
  expect(after.emergencyStop).toBe(false);

  // 解除之後：手機被告知已經沒事了（不會停在緊急畫面），但麥克風**不會**自動恢復。
  await fixture.waitForEvent(
    (e) =>
      e.event === "act" &&
      e.name === "character.present" &&
      (e.params as Record<string, unknown> | undefined)?.state === "idle",
    20_000,
    cleared
  );
  expect(after.activeSensors ?? []).toEqual([]);
  fixture.send({ op: "status", micLevel: true });
  await new Promise((r) => setTimeout(r, 1_500));
  const resumed = (await api(request, "GET", "/v1/status")) as { activeSensors?: unknown[] };
  expect(
    resumed.activeSensors ?? [],
    "受器在緊急停止後必須保持停用，要人重新啟用才會再感測"
  ).toEqual([]);
});

test("緊急停止：390px 的頂部列一樣按得到，狀態一樣誠實", async ({ page, request }) => {
  test.setTimeout(180_000);
  await page.setViewportSize(NARROW);
  await openApp(page);
  const topbar = page.locator(".topbar");
  await topbar.getByRole("button", { name: "緊急停止", exact: true }).click();
  await topbar.getByRole("button", { name: "立即停止一切？" }).click();
  await expect(page.getByText("緊急停止已啟動").first()).toBeVisible({ timeout: 20_000 });
  const status = (await api(request, "GET", "/v1/status")) as { emergencyStop?: boolean };
  expect(status.emergencyStop).toBe(true);
  // 窄視窗也看得到固定的安全文字與解除入口（不是只有桌面才有）。
  await navigateTo(page, PAGES[3], true);
  await expect(page.getByTestId("connect-area-stop").getByText("緊急停止中")).toBeVisible({
    timeout: 20_000,
  });
  await clearThroughSafetyFlow(page);
  const after = (await api(request, "GET", "/v1/status")) as { emergencyStop?: boolean };
  expect(after.emergencyStop).toBe(false);
});
