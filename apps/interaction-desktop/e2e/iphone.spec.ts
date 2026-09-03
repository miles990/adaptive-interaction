// 使用者任務：把手機接上來、看得懂它現在能做什麼、權限沒給時看得到、
// 斷線時不會被當成還能用、要移除時真的移得掉。
//
// ⚠️ 這一支全部使用【模擬 iPhone（fixture）】——
// crates/interaction-runtime/examples/fake_iphone.rs，是程序外的假手機，
// 只是把 mobile_loop.rs 那個程序內模擬手機搬成可執行檔。
// **iPhone 真機驗收仍然是零**，本檔的斷言與截圖都不得被寫成真機驗收。
//
// 配對走的是人類流程（UI 的「開始配對」→ 讀畫面上的配對碼），連線用的
// port／指紋從 GET /v1/mobile/status 讀（一般模式的畫面只給指紋前 6 碼）。
// daemon 帶 INTERACT_AI_MOBILE_ADVERTISE=0：不對區網廣播、只綁 127.0.0.1，
// 所以這場模擬沒有任何外部副作用。

import { test, expect, Page } from "@playwright/test";
import * as path from "node:path";
import {
  api,
  beginPairingFromUi,
  DESKTOP,
  FAKE_IPHONE_LABEL,
  NARROW,
  navigateTo,
  openApp,
  PAGES,
  revokePairedPhones,
  spawnFakeIphone,
  type FakeIphone,
} from "./helpers";

test.describe.configure({ mode: "serial" });

const OUT = path.resolve(process.cwd(), "../../docs/assets/v05-evidence");
const CONNECT = PAGES[3];
let phone: FakeIphone | null = null;

test.afterAll(async () => {
  phone?.kill();
  phone = null;
  await revokePairedPhones();
});

/** 回到「連接與權限」並展開第二層的「裝置與來源」。 */
async function openConnect(page: Page) {
  await navigateTo(page, CONNECT, false);
  await page.getByRole("tab", { name: "裝置與來源" }).click();
}

/** 讀後端真相：這台手機現在的裝置列。 */
async function deviceRecord(
  request: import("@playwright/test").APIRequestContext,
  deviceId: string
): Promise<Record<string, unknown> | undefined> {
  const status = (await api(request, "GET", "/v1/mobile/status")) as {
    devices?: Record<string, unknown>[];
  };
  return (status.devices ?? []).find((d) => String(d.deviceId) === deviceId);
}

/** 有界等待後端的裝置狀態（連線與否是非同步的）。 */
async function waitDevice(
  request: import("@playwright/test").APIRequestContext,
  deviceId: string,
  predicate: (device: Record<string, unknown> | undefined) => boolean,
  timeoutMs = 15_000
): Promise<Record<string, unknown> | undefined> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const device = await deviceRecord(request, deviceId);
    if (predicate(device)) return device;
    if (Date.now() > deadline) {
      throw new Error(`裝置狀態等不到預期值：${JSON.stringify(device)}`);
    }
    await new Promise((r) => setTimeout(r, 250));
  }
}

test("iPhone（模擬 fixture）：用畫面上的配對碼配對 → 卡片說「已連線」，並列出可以提供／可以執行", async ({
  page,
  request,
}) => {
  test.setTimeout(180_000);
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  await openConnect(page);

  // 還沒配對時要誠實說沒有。
  await expect(page.getByText("還沒有配對的 iPhone。")).toBeVisible({ timeout: 15_000 });
  const pairing = await beginPairingFromUi(page, request);
  expect(pairing.code).toMatch(/^\d{6}$/);

  // 模擬不得有區網副作用：這支 daemon 不廣播 Bonjour，也只綁 127.0.0.1。
  const mobile = (await api(request, "GET", "/v1/mobile/status")) as {
    bonjour?: Record<string, unknown>;
  };
  expect(mobile.bonjour?.advertised).toBe(false);
  expect(mobile.bonjour?.bindIp).toBe("127.0.0.1");
  await expect(page.getByText(/iPhone 無法自動找到這台電腦（自動尋找未啟用/)).toBeVisible();

  phone = await spawnFakeIphone({
    port: pairing.port,
    fingerprint: pairing.fingerprint,
    code: pairing.code,
  });
  const device = await waitDevice(request, phone.deviceId, (d) => d?.connected === true);
  expect(device?.name).toBe(FAKE_IPHONE_LABEL);

  // 畫面：第一層「已連接的裝置」就看得到這台手機（不用進第二層）。
  await page.reload();
  await navigateTo(page, CONNECT, false);
  const card = page.locator(`[data-testid="phone-card-${phone.deviceId}"]`).first();
  await expect(card).toBeVisible({ timeout: 20_000 });
  await expect(card.getByText(FAKE_IPHONE_LABEL)).toBeVisible();
  await expect(card.getByText("已連線", { exact: true })).toBeVisible();
  await expect(card.getByText(/可以提供：/)).toBeVisible();
  await expect(card.getByText(/可以執行：/)).toBeVisible();
  // 能力清單不是空話：後端的能力卡真的有 iPhone 的受器與動器。
  const human = (await api(
    request,
    "GET",
    "/v1/capabilities/human?includeUnavailable=true"
  )) as {
    receptors?: { id: string }[];
    actuators?: { id: string }[];
  };
  expect((human.receptors ?? []).some((r) => r.id.startsWith("iphone."))).toBe(true);
  expect((human.actuators ?? []).some((a) => a.id.startsWith("iphone."))).toBe(true);
  await page.screenshot({ path: path.join(OUT, "desktop-connect-iphone-fixture.png") });

  await page.setViewportSize(NARROW);
  await card.scrollIntoViewIfNeeded();
  await expect(card.getByText("已連線", { exact: true })).toBeVisible();
  await page.screenshot({ path: path.join(OUT, "narrow-connect-iphone-fixture.png") });
});

test("iPhone（模擬 fixture）：手機上沒給麥克風權限，桌面照實顯示「已拒絕」並列進需要確認", async ({
  page,
  request,
}) => {
  test.setTimeout(120_000);
  const target = phone!;
  target.send({ op: "status", micLevel: false, permissions: { microphone: "denied" } });
  const device = await waitDevice(
    request,
    target.deviceId,
    (d) => (d?.permissions as Record<string, unknown> | undefined)?.microphone === "denied"
  );
  expect((device?.permissions as Record<string, unknown>).microphone).toBe("denied");

  await page.setViewportSize(DESKTOP);
  await openApp(page);
  await navigateTo(page, CONNECT, false);
  const card = page.locator(`[data-testid="phone-card-${target.deviceId}"]`).first();
  await expect(card.getByText(/麥克風：已拒絕/)).toBeVisible({ timeout: 20_000 });
  // 桌面的同意不能取代 iOS 系統權限：列進「目前需要確認的權限」。
  await expect(
    page
      .getByTestId("connect-area-confirm")
      .getByText(new RegExp(`在 ${FAKE_IPHONE_LABEL} 上尚未允許：麥克風（已拒絕）`))
  ).toBeVisible();
});

test("iPhone（模擬 fixture）：斷線＝能力不可用，重新連上才恢復", async ({ page, request }) => {
  test.setTimeout(120_000);
  const target = phone!;
  target.send({ op: "disconnect" });
  await target.waitForEvent((e) => e.event === "disconnected");
  await waitDevice(request, target.deviceId, (d) => d?.connected === false);

  await page.setViewportSize(DESKTOP);
  await openApp(page);
  await navigateTo(page, CONNECT, false);
  const card = page.locator(`[data-testid="phone-card-${target.deviceId}"]`).first();
  await expect(card.getByText("未連線（能力不可用）")).toBeVisible({ timeout: 20_000 });
  await expect(card.getByText("手機未連線時送不出任何指令。")).toBeVisible();
  await expect(card.getByRole("button", { name: "測試連接" })).toBeDisabled();

  // 重新連上（用配對時拿到的 token，不需要再配對一次）。
  const seen = target.events.length;
  target.send({ op: "reconnect" });
  await target.waitForEvent((e) => e.event === "connected", 15_000, seen);
  await waitDevice(request, target.deviceId, (d) => d?.connected === true);
  await page.reload();
  await navigateTo(page, CONNECT, false);
  await expect(card.getByText("已連線", { exact: true })).toBeVisible({ timeout: 20_000 });
});

test("iPhone（模擬 fixture）：移除這台手機＝立刻斷線，而且再也認證不過", async ({
  page,
  request,
}) => {
  test.setTimeout(120_000);
  const target = phone!;
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  await navigateTo(page, CONNECT, false);
  const card = page.locator(`[data-testid="phone-card-${target.deviceId}"]`).first();
  await expect(card).toBeVisible({ timeout: 20_000 });
  await card.getByRole("button", { name: "移除此手機" }).click();
  await card.getByRole("button", { name: /確定移除？/ }).click();
  // 清單自己重讀之後這張卡就不該還在（移除成功的提示是寫在卡片裡的，
  // 卡片一消失提示也跟著消失——所以這裡驗「卡片真的不見了」，不驗那句話）。
  await expect(page.locator(`[data-testid="phone-card-${target.deviceId}"]`)).toHaveCount(0, {
    timeout: 20_000,
  });

  // 後端事實：裝置真的不在了。
  await waitDevice(request, target.deviceId, (d) => d === undefined);
  // 手機端事實：拿舊 token 重連會被拒絕（撤銷不是只有畫面上的事）。
  const seen = target.events.length;
  target.send({ op: "reconnect" });
  await target.waitForEvent((e) => e.event === "auth-fail", 20_000, seen);

  await page.reload();
  await navigateTo(page, CONNECT, false);
  await expect(page.locator(`[data-testid="phone-card-${target.deviceId}"]`)).toHaveCount(0);
});
