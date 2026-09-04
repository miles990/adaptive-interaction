// 使用者任務：手機上的角色和這台電腦上的角色，是不是同一個角色？
//
// ⚠️ 這一支全部使用【模擬 iPhone（fixture）】——
// crates/interaction-runtime/examples/fake_iphone.rs，是程序外的假手機。
// **iPhone 真機驗收仍然是零**，本檔的斷言與截圖都不得被寫成真機驗收。
//
// 為什麼另外起一支真 daemon（而不是用共用的那一支）：這條 journey 會配對手機、
// 撤銷手機、還會按下緊急停止。緊急停止會撤銷同意、取消進行中的工作、停掉感測，
// 放在共用 daemon 上會污染別的 spec。隔離的家與埠號讓這一整條路可以走完整。
// daemon 帶 INTERACT_AI_MOBILE_ADVERTISE=0：不對區網廣播、只綁 127.0.0.1。
//
// 驗的是「畫面說的」與「後端真相」兩邊同時成立：
//   * UI 的每一句都對照 `docs/aip/character-session.md` §11 的文案表；
//   * 後端事實用 `GET /v1/character-session`（權威狀態）與 fixture 收到的 result 驗。
//   * 送出 ≠ 生效：撤銷後、緊急停止中的觸摸一律被拒，畫面不得顯示成已同步。

import { test, expect, Locator, Page, APIRequestContext } from "@playwright/test";
import * as path from "node:path";
import { CHARACTER_SYNC_PROJECTION } from "../src/statusProjection";
import {
  aipCapability,
  aipTouch,
  aipResume,
  api,
  appUrl,
  beginPairingFromUi,
  characterSessionSnapshot,
  DESKTOP,
  FAKE_IPHONE_LABEL,
  memberPresence,
  NARROW,
  navigateTo,
  openApp,
  PAGES,
  sessionState,
  spawnDaemon,
  spawnFakeIphone,
  waitCharacterSession,
  type FakeIphone,
  type SpawnedDaemon,
} from "./helpers";

test.describe.configure({ mode: "serial" });

/** v0.6 的證據目錄（不覆寫 v0.5 的截圖）。 */
const OUT = path.resolve(process.cwd(), "../../docs/assets/v06-evidence");
const PORT = 18794;
const COMPANION = PAGES[1];
/**
 * 有 online 遠端成員時同步卡可以說的那幾句——但**不含**綠色的「已同步」。
 *
 * 【模擬 iPhone（fixture）】只宣告三個 intent，host 有四個（`settle` 會被協商成
 * unsupported），所以這台手機無論如何都不該拿到唯一的綠勾：
 *   - Runtime 還沒把協商結果投影給桌面 → 桌面不知道 → `capability-unknown`（pending）；
 *   - Runtime 補上 `members[].unsupportedIntents` 之後 → `partial-capability`（warn）。
 * 兩種都是誠實的答案，綠色不是（對抗審查 capability-consent-052／general-mode-ux-022）。
 * 句子從投影表導出而不是手抄，文案改了這裡自動跟上（對抗審查 evidence-honesty-015）。
 */
const SYNC_HEADLINES_ONLINE_HONEST = [
  CHARACTER_SYNC_PROJECTION["capability-unknown"].headline,
  CHARACTER_SYNC_PROJECTION["partial-capability"].headline,
];

/** 同步卡目前顯示的那一句（badge 文字）。 */
async function syncHeadline(card: Locator): Promise<string> {
  return (await card.locator(".badge").first().innerText()).trim();
}
const CONNECT = PAGES[3];

let daemon: SpawnedDaemon | null = null;
const phones: FakeIphone[] = [];

test.beforeAll(async () => {
  daemon = await spawnDaemon({ port: PORT, label: "character-session" });
});

test.afterAll(async () => {
  for (const phone of phones) phone.kill();
  phones.length = 0;
  daemon?.kill();
  daemon = null;
});

/** 這一支自己的 daemon（所有 API 呼叫都要帶）。 */
function target(): { base: string; token: string } {
  return { base: daemon!.api, token: daemon!.token };
}

/** 角色頁的「同步」卡。 */
function syncCard(page: Page) {
  return page.getByTestId("character-sync");
}

/** 開到角色頁並回傳同步卡（等它真的讀到狀態）。 */
async function openSyncCard(page: Page, narrow: boolean) {
  await navigateTo(page, COMPANION, narrow);
  const card = syncCard(page);
  await card.scrollIntoViewIfNeeded();
  await expect(card).toBeVisible({ timeout: 20_000 });
  return card;
}

/** 配對一台模擬 iPhone（fixture）：走人類流程（畫面上的配對碼）。 */
async function pairFixturePhone(
  page: Page,
  request: APIRequestContext,
  narrow: boolean
): Promise<FakeIphone> {
  await navigateTo(page, CONNECT, narrow);
  await page.getByRole("tab", { name: "裝置與來源" }).click();
  const pairing = await beginPairingFromUi(page, request, target());
  expect(pairing.code).toMatch(/^\d{6}$/);
  const phone = await spawnFakeIphone({
    port: pairing.port,
    fingerprint: pairing.fingerprint,
    code: pairing.code,
  });
  phones.push(phone);
  return phone;
}

/**
 * 一輪完整 journey（桌面寬度與 390px 各跑一次）。
 *
 * 配對 → 協商 → 已同步 → 摸一下 → 離線 → 重連 → 回到已同步 → 撤銷 → 需要重新確認。
 */
async function runJourney(
  page: Page,
  request: APIRequestContext,
  options: { narrow: boolean; shot: string; freshDaemon: boolean }
) {
  const { narrow } = options;
  await page.setViewportSize(narrow ? NARROW : DESKTOP);
  await openApp(page, appUrl(daemon!.api, daemon!.token));

  // 1. 開始之前一定不是「已同步」。第一輪的 daemon 還沒配對過任何裝置，
  //    所以空狀態要中性；第二輪跑在同一支 daemon 上（第一輪撤銷過一台手機），
  //    那時的正確狀態是「需要重新確認裝置」——兩種都不是成功。
  let card = await openSyncCard(page, narrow);
  if (options.freshDaemon) {
    await expect(card.getByText("尚未連接 iPhone")).toBeVisible({ timeout: 20_000 });
  } else {
    await expect(card.getByText("需要重新確認裝置")).toBeVisible({ timeout: 20_000 });
  }
  await expect(card.locator(".badge-ok")).toHaveCount(0);

  // 2. 配對模擬 iPhone（fixture）並送 capability（＝加入 session）。
  const phone = await pairFixturePhone(page, request, narrow);
  const negotiated = await aipCapability(phone);
  expect(negotiated.kind).toBe("snapshot");
  const joined = await waitCharacterSession(
    request,
    (payload) => memberPresence(payload, phone.deviceId) === "online",
    20_000,
    target()
  );

  // 3. 角色頁：手機成了 session 成員，成員清單用手機的名字。
  //    句子是「iPhone 已連接，能力核對中」而不是綠色的「角色狀態已同步」——Runtime 還沒把
  //    協商結果（哪些 intent 是 unsupported）投影到 /v1/character-session，桌面拿不到就不猜。
  //    這台 fixture 手機只宣告三個 intent，host 有四個（settle 會被協商成 unsupported），
  //    所以舊的綠勾本來就是假的（對抗審查 capability-consent-052／general-mode-ux-022）。
  card = await openSyncCard(page, narrow);
  await expect
    .poll(async () => syncHeadline(card), { timeout: 20_000 })
    .not.toBe(CHARACTER_SYNC_PROJECTION.synced.headline);
  expect(SYNC_HEADLINES_ONLINE_HONEST, "協商不完整的裝置不得拿到綠色「已同步」").toContain(
    await syncHeadline(card)
  );
  await expect(card.locator(".badge-ok"), "協商不完整就不得給綠勾").toHaveCount(0);
  const members = card.getByRole("list", { name: "同步中的裝置" });
  await expect(members.getByText(FAKE_IPHONE_LABEL)).toBeVisible();
  await expect(members.getByText("已連接")).toBeVisible();
  // 一般模式不外洩技術詞。
  const generalText = (await card.innerText()).toLowerCase();
  expect(generalText).not.toMatch(/revision|sequence|epoch|schema|token/);
  await page.screenshot({ path: path.join(OUT, `${options.shot}-synced.png`) });

  // 4. 摸一下角色：後端 revision 前進，畫面出現人話的「最近互動」。
  const beforeRevision = Number(joined.revision ?? 0);
  const result = await aipTouch(phone, "tap");
  expect(result.status).toBe("applied");
  const touched = await waitCharacterSession(
    request,
    (payload) => Number(payload.revision ?? 0) > beforeRevision,
    20_000,
    target()
  );
  expect(Number(touched.revision)).toBeGreaterThan(beforeRevision);
  expect((sessionState(touched).lastInteraction as Record<string, unknown>)?.name).toBe(
    "character.interaction.touch"
  );
  // SSE（character.session.state）會把卡片推到最新；不必重新整理頁面。
  await expect(card.getByText(/摸了摸角色/)).toBeVisible({ timeout: 30_000 });

  // 5. 斷線：Transport 在重連退避窗內先誠實說「iPhone 正在重新連線」（presence reconnecting，
  //    成員保留），session 逾時（45 s）後才轉 offline／「iPhone 暫時離線」——契約 character-session.md
  //    §7 與 §12.15。這裡只等得到 reconnecting；不是「沒有裝置」也不是綠色的「已同步」。
  const seen = phone.events.length;
  phone.send({ op: "disconnect" });
  await phone.waitForEvent((e) => e.event === "disconnected", 20_000, seen);
  await waitCharacterSession(
    request,
    (payload) => memberPresence(payload, phone.deviceId) === "reconnecting",
    20_000,
    target()
  );
  await expect(card.getByText("iPhone 正在重新連線")).toBeVisible({ timeout: 30_000 });
  await expect(card.locator(".badge-ok")).toHaveCount(0);
  await page.screenshot({ path: path.join(OUT, `${options.shot}-reconnecting.png`) });

  // 6. 重連：先重新協商、再對齊（契約 §7 的重連流程），回到已同步。
  const reconnected = phone.events.length;
  phone.send({ op: "reconnect" });
  await phone.waitForEvent((e) => e.event === "connected", 20_000, reconnected);
  const snapshot = await aipCapability(phone);
  const replay = await aipResume(phone, {
    lastRevision: Number(snapshot.revision ?? 0),
    lastSequence: Number(snapshot.sequence ?? 0),
    epoch: Number(snapshot.sessionEpoch ?? 0),
  });
  expect(["patches", "snapshot"]).toContain(String(replay.kind));
  await waitCharacterSession(
    request,
    (payload) => memberPresence(payload, phone.deviceId) === "online",
    20_000,
    target()
  );
  await expect
    .poll(async () => syncHeadline(card), { timeout: 30_000 })
    .not.toBe(CHARACTER_SYNC_PROJECTION["no-device"].headline);
  expect(SYNC_HEADLINES_ONLINE_HONEST, "重連之後仍然不得謊稱完全同步").toContain(
    await syncHeadline(card)
  );

  // 7. 撤銷這台手機（既有 UI 流程）→ 連線關閉、不再是成員。
  await navigateTo(page, CONNECT, narrow);
  const phoneCard = page.locator(`[data-testid="phone-card-${phone.deviceId}"]`).first();
  await expect(phoneCard).toBeVisible({ timeout: 20_000 });
  // 連接頁的手機卡上有一行同步狀態（連上 ≠ 已同步，兩件事分開說）。
  await expect(phoneCard.getByText(/角色同步：/)).toBeVisible();
  await phoneCard.getByRole("button", { name: "移除此手機" }).click();
  await phoneCard.getByRole("button", { name: /確定移除？/ }).click();
  await expect(page.locator(`[data-testid="phone-card-${phone.deviceId}"]`)).toHaveCount(0, {
    timeout: 20_000,
  });

  // 8. 撤銷之後的觸摸送不出去（連線已關），後端也不再有這個成員。
  phone.send({ op: "aip-touch", kind: "tap", expiresInMs: 5000 });
  const afterRevoke = await waitCharacterSession(
    request,
    (payload) => memberPresence(payload, phone.deviceId) === null,
    20_000,
    target()
  );
  expect(memberPresence(afterRevoke, phone.deviceId)).toBeNull();

  // 9. 角色頁：「需要重新確認裝置」——不是回到空狀態，也不是已同步。
  card = await openSyncCard(page, narrow);
  await expect(card.getByText("需要重新確認裝置")).toBeVisible({ timeout: 30_000 });
  await expect(card.locator(".badge-ok")).toHaveCount(0);
  await page.screenshot({ path: path.join(OUT, `${options.shot}-needs-reconfirmation.png`) });
}

test("角色同步（模擬 iPhone（fixture））：配對 → 已同步 → 摸一下 → 離線 → 重連 → 撤銷（桌面寬度）", async ({
  page,
  request,
}) => {
  test.setTimeout(300_000);
  await runJourney(page, request, {
    narrow: false,
    shot: "desktop-character-sync",
    freshDaemon: true,
  });
});

test("角色同步（模擬 iPhone（fixture））：同一條路在 390px 上也走得完", async ({ page, request }) => {
  test.setTimeout(300_000);
  await runJourney(page, request, {
    narrow: true,
    shot: "narrow-character-sync",
    freshDaemon: false,
  });
  // 390px 不得產生水平捲動。
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth
  );
  expect(overflow).toBeLessThanOrEqual(1);
});

test("角色同步：緊急停止中，模擬 iPhone（fixture）的觸摸被拒，畫面顯示緊急狀態", async ({
  page,
  request,
}) => {
  test.setTimeout(300_000);
  await page.setViewportSize(DESKTOP);
  await openApp(page, appUrl(daemon!.api, daemon!.token));
  const phone = await pairFixturePhone(page, request, false);
  await aipCapability(phone);
  await waitCharacterSession(
    request,
    (payload) => memberPresence(payload, phone.deviceId) === "online",
    20_000,
    target()
  );

  // 人類按下緊急停止（快速操作 → 二次確認）。
  await navigateTo(page, PAGES[0], false);
  const home = page.locator(".home");
  await home.getByRole("button", { name: "緊急停止", exact: true }).click();
  await home.getByRole("button", { name: "立即停止一切？" }).click();
  await expect(page.getByText("緊急停止已啟動").first()).toBeVisible({ timeout: 20_000 });
  const status = (await api(request, "GET", "/v1/status", undefined, target())) as {
    emergencyStop?: boolean;
  };
  expect(status.emergencyStop).toBe(true);

  // 權威狀態：角色被凍住。Runtime 的真相是同步改的、派送在背景任務，
  // 所以這裡等它真的落到權威狀態，再測互動會不會被擋（不用 sleep 賭時間）。
  const frozen = await waitCharacterSession(
    request,
    (payload) => (sessionState(payload).truth as Record<string, unknown>)?.state === "emergency",
    20_000,
    target()
  );
  expect((sessionState(frozen).truth as Record<string, unknown>)?.state).toBe("emergency");

  // 後端事實：緊急停止中的互動一律被拒（不是「排隊等一下」）。
  const rejected = await aipTouch(phone, "tap");
  expect(rejected.status).toBe("rejected");
  expect(String(rejected.code)).toBe("scope-denied");

  // 畫面事實：同步卡用固定安全句說緊急停止中（角色不能覆寫這一句）。
  const card = await openSyncCard(page, false);
  await expect(
    card.getByText("緊急停止中：角色已停止表演，解除前不會接受任何互動。")
  ).toBeVisible({ timeout: 30_000 });
  await expect(card.locator(".badge-ok")).toHaveCount(0);
  await page.screenshot({ path: path.join(OUT, "desktop-character-sync-emergency.png") });

  // 解除只有人做得到，而且要走安全流程（不會自動恢復）。
  await page.locator(".topbar").getByRole("button", { name: /緊急停止中 — 前往解除/ }).click();
  await page.getByRole("button", { name: /開始安全解除流程/ }).click();
  const dialog = page.getByRole("dialog", { name: "解除緊急停止" });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "我了解，解除緊急停止" }).click();
  await dialog.getByRole("button", { name: "確定解除？" }).click();
  await expect(
    page.locator(".topbar").getByRole("button", { name: "緊急停止", exact: true })
  ).toBeVisible({ timeout: 20_000 });
});

test("角色同步：鍵盤到得了五個主入口與同步卡；減少動態時卡片仍讀得到", async ({ page }) => {
  test.setTimeout(180_000);
  await page.setViewportSize(DESKTOP);
  await openApp(page, appUrl(daemon!.api, daemon!.token));
  await navigateTo(page, COMPANION, false);

  // 五個主入口都在鍵盤路徑上——真的按 Tab 走一遍並斷言焦點依序落在每一個入口，
  // 而不是只看五段文字可見（那種斷言在鍵盤根本到不了時也會通過；
  // 對抗審查 general-mode-ux-027）。
  const nav = page.getByRole("navigation", { name: "主要導覽" });
  const entries = nav.getByRole("button");
  await expect(entries.nth(PAGES.length - 1)).toBeVisible({ timeout: 20_000 });
  // 焦點回到文件開頭，再一路 Tab 走過去。
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  for (let index = 0; index < PAGES.length; index += 1) {
    const entry = entries.nth(index);
    const label = (await entry.innerText()).trim();
    let focused = false;
    for (let i = 0; i < 40 && !focused; i += 1) {
      await page.keyboard.press("Tab");
      focused = await entry.evaluate((el) => el === document.activeElement);
    }
    expect(focused, `鍵盤走不到第 ${index + 1} 個主入口「${label}」`).toBe(true);
  }
  // 可見 ≠ 可用：焦點停在最後一個入口時按 Enter 真的會換到那一頁。
  await page.keyboard.press("Enter");
  await expect(entries.nth(PAGES.length - 1)).toHaveAttribute("aria-current", "page");

  // 同步卡：從頁面開頭一路 Tab 一定按得到「重新檢查」。
  await page.reload();
  await navigateTo(page, COMPANION, false);
  const recheck = syncCard(page).getByRole("button", { name: "重新檢查" });
  await expect(recheck).toBeVisible({ timeout: 20_000 });
  let focused = false;
  for (let i = 0; i < 60 && !focused; i += 1) {
    await page.keyboard.press("Tab");
    focused = await recheck.evaluate((el) => el === document.activeElement);
  }
  expect(focused, "60 次 Tab 之內沒有走到同步卡的「重新檢查」").toBe(true);

  // 螢幕閱讀器：卡片是一個有名字的區域。
  await expect(page.getByRole("region", { name: "角色同步" })).toBeVisible();

  // 減少動態：卡片的每一句話照樣讀得到（不是靠動畫才看得見）。
  //
  // 舊版比對 /尚未連接 iPhone|iPhone|角色同步目前關閉/ ——中間那個空泛的 `iPhone`
  // 讓「iPhone 已連接，角色狀態已同步」這種**錯的**句子也照樣通過，等於什麼都沒驗
  //（對抗審查 general-mode-ux-027）。改成：先取非 reduced-motion 下的那一句，
  // 再要求 reduced-motion 下**完全相同**，而且必須是投影表裡的固定文案之一。
  const allowed = Object.values(CHARACTER_SYNC_PROJECTION).map((p) => p.headline);
  const before = (await (await openSyncCard(page, false)).locator(".badge").first().innerText()).trim();
  expect(allowed, "同步卡的句子必須是契約固定文案").toContain(before);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.reload();
  const card = await openSyncCard(page, false);
  const after = (await card.locator(".badge").first().innerText()).trim();
  expect(after, "減少動態不得改變同步卡說的事實").toBe(before);
  await expect(card.getByText(before, { exact: true }).first()).toBeVisible({ timeout: 20_000 });
  expect((await card.innerText()).trim().length).toBeGreaterThan(0);
  await page.emulateMedia({ reducedMotion: null });
});
