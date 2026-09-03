// 使用者任務：有東西在感測時我一定看得見，而且一個動作就能要求全部停下來——
// 「已停止」與「還不確定」必須分得清清楚楚。
//
// ⚠️ 感測來源是【模擬 iPhone（fixture）】（crates/interaction-runtime/examples/fake_iphone.rs）：
// E2E 的 daemon 沒有本機麥克風後端（CI 不編 mic-capture），iPhone fixture 是唯一
// 不需要真硬體就能讓 `status.activeSensors` 真的有東西的路徑。iPhone 真機驗收仍為零。
//
// 兩條路徑都要測：手機有回 ack（可以說已停止）與手機不回（只能說還不確定）。

import { test, expect, Page } from "@playwright/test";
import * as path from "node:path";
import {
  api,
  beginPairing,
  DESKTOP,
  FAKE_IPHONE_LABEL,
  NARROW,
  openApp,
  revokePairedPhones,
  spawnFakeIphone,
  waitActiveSensors,
  type FakeIphone,
} from "./helpers";

test.describe.configure({ mode: "serial" });

const OUT = path.resolve(process.cwd(), "../../docs/assets/v05-evidence");
const MIC = "iphone.mic-level";
let phone: FakeIphone | null = null;

test.afterEach(async () => {
  phone?.kill();
  phone = null;
  await revokePairedPhones();
});

/** 配一台模擬手機並讓它回報「麥克風音量串流中」。 */
async function startFixtureSensing(
  request: import("@playwright/test").APIRequestContext,
  options: { autoAckStopAll: boolean }
): Promise<FakeIphone> {
  const pairing = await beginPairing(request);
  const fixture = await spawnFakeIphone({ ...pairing, autoAckStopAll: options.autoAckStopAll });
  // 受器是人類開的：手機自己說在串流不算數（沒授權就不算感測）。
  await api(request, "PATCH", `/v1/receptors/${MIC}`, { enabled: true });
  fixture.send({ op: "status", micLevel: true });
  await waitActiveSensors(request, (list) => list.some((s) => s.kind === MIC));
  return fixture;
}

/** 感測使用中時，狀態列橫幅與首頁那一句都必須說出來（感測不靜默）。 */
async function assertSensingIsVisible(page: Page) {
  const banner = page.locator(".sensor-banner");
  await expect(banner.first()).toBeVisible({ timeout: 20_000 });
  // 給螢幕閱讀器唸得到：橫幅是 role=status（狀態變化要被播報，不是純裝飾）。
  await expect(banner.first()).toHaveAttribute("role", "status");
  await expect(banner.first()).toContainText("感測使用中");
  await expect(banner.first()).toContainText("麥克風");
  await expect(page.getByTestId("now-character")).toContainText("感測使用中（麥克風）");
}

test("感測：模擬 iPhone 在串流麥克風音量時，狀態列與首頁同時說得出來", async ({
  page,
  request,
}) => {
  test.setTimeout(180_000);
  phone = await startFixtureSensing(request, { autoAckStopAll: true });

  // 後端事實：activeSensors 真的有這一筆，而且來源是那台手機。
  const sensors = await waitActiveSensors(request, (list) => list.some((s) => s.kind === MIC));
  const entry = sensors.find((s) => s.kind === MIC)!;
  expect(entry.startedBy).toBe(`iphone:${phone.deviceId}`);
  expect(String(entry.purpose)).toContain("僅音量值");

  await page.setViewportSize(DESKTOP);
  await openApp(page);
  await assertSensingIsVisible(page);
  await page.screenshot({ path: path.join(OUT, "desktop-sensors-active-iphone-fixture.png") });
  await page.setViewportSize(NARROW);
  await assertSensingIsVisible(page);
});

test("感測：一個動作停止全部——手機回覆確認後才敢說「已停止感測」", async ({ page, request }) => {
  test.setTimeout(180_000);
  phone = await startFixtureSensing(request, { autoAckStopAll: true });
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  await assertSensingIsVisible(page);

  const seen = phone.events.length;
  await page.locator(".home").getByRole("button", { name: "停止所有感測" }).click();

  // 手機真的收到「連感測一起停」的指令，並回了確認。
  const index = await phone.waitForEvent(
    (e) => e.event === "stop-all" && e.sensors === true,
    20_000,
    seen
  );
  await phone.waitForEvent((e) => e.event === "ack-stop-all", 20_000, index);

  // 後端事實：沒有任何感測還在跑，而且高風險受器被強制停用（要再開必須人類重新啟用）。
  await waitActiveSensors(request, (list) => list.length === 0);
  const receptors = (await api(request, "GET", "/v1/receptors")) as
    | { receptors?: { id: string; enabled?: boolean }[] }
    | { id: string; enabled?: boolean }[];
  const list = Array.isArray(receptors) ? receptors : (receptors.receptors ?? []);
  const mic = list.find((r) => r.id === MIC);
  if (mic) expect(mic.enabled).not.toBe(true);

  // 畫面：確認之後才可以說「已停止感測」，而且橫幅要消失。
  await expect(page.locator(".home").getByText("已停止感測。")).toBeVisible({ timeout: 20_000 });
  await expect(page.locator(".sensor-banner")).toHaveCount(0, { timeout: 20_000 });
});

test("感測：手機沒回覆時，畫面絕不說「已停止」——只說還不確定，而且感測不會從畫面消失", async ({
  page,
  request,
}) => {
  test.setTimeout(180_000);
  phone = await startFixtureSensing(request, { autoAckStopAll: false });
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  await assertSensingIsVisible(page);

  const seen = phone.events.length;
  await page.locator(".home").getByRole("button", { name: "停止所有感測" }).click();
  await phone.waitForEvent((e) => e.event === "stop-all" && e.sensors === true, 20_000, seen);

  // 後端事實：逐台回報「unknown」，整份報告標 uncertain（不是成功）。
  const report = (await api(request, "POST", "/v1/sensors/stop")) as {
    stopped?: boolean;
    uncertain?: boolean;
    devices?: { outcome?: string; name?: string }[];
  };
  expect(report.stopped).toBe(false);
  expect(report.uncertain).toBe(true);
  expect(report.devices?.[0]?.outcome).toBe("unknown");
  expect(report.devices?.[0]?.name).toBe(FAKE_IPHONE_LABEL);

  // 而且沒回覆的來源不得從畫面上消失：仍然列在 activeSensors，狀態標成「停止結果未知」。
  const remaining = await waitActiveSensors(request, (list) => list.some((s) => s.kind === MIC));
  expect(remaining.find((s) => s.kind === MIC)?.state).toBe("stop-unknown");

  // 畫面：警示（不是綠色狀態），而且永遠不出現「已停止感測」。
  const notice = page.locator(".home").getByRole("alert");
  await expect(notice.first()).toBeVisible({ timeout: 20_000 });
  await expect(notice.first()).toContainText("已要求停止");
  await expect(notice.first()).toContainText(/仍在使用中|結果不確定/);
  await expect(page.locator(".home").getByText("已停止感測。")).toHaveCount(0);
  // 感測橫幅也不能消失（真相是「可能還在錄」）。
  await expect(page.locator(".sensor-banner").first()).toBeVisible();

  // 手機後來才確認停了：不確定不是永久標籤，畫面要跟著回到「沒有感測」。
  phone.send({ op: "ack-stop-all" });
  await waitActiveSensors(request, (list) => list.length === 0, 15_000);
  await page.reload();
  await expect(page.locator(".sensor-banner")).toHaveCount(0, { timeout: 20_000 });
});
