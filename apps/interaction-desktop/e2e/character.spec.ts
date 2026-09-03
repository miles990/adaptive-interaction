// 使用者任務：換角色或把角色藏起來。
//
// 瀏覽器版控制中心**沒有**桌面角色視窗，角色偏好住在 Tauri host（desktop_prefs_*），
// 所以這裡驗的是「誠實降級」：不假裝換成功、不顯示一個按了沒用的開關，
// 而且無論如何都不得改到後端的角色狀態。真正的換角色／隱藏屬於桌面版（Tauri）驗收，
// 這一支不冒充它。
//
import { test, expect } from "@playwright/test";
import { api, DESKTOP, NARROW, navigateTo, openApp, PAGES } from "./helpers";

test.describe.configure({ mode: "serial" });

const COMPANION = PAGES[1];

/** 後端目前的角色實例狀態（換角色成功與否的唯一事實來源）。 */
async function characterState(
  request: import("@playwright/test").APIRequestContext
): Promise<string> {
  const status = (await api(request, "GET", "/v1/status")) as {
    characterProtocol?: Record<string, unknown>;
  };
  return JSON.stringify(status.characterProtocol ?? null);
}

test("角色：瀏覽器檢視不提供假的開關，也不會真的改到後端角色狀態", async ({ page, request }) => {
  test.setTimeout(120_000);
  const before = await characterState(request);
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  await navigateTo(page, COMPANION, false);

  // 桌面角色偏好需要桌面版：誠實說明，而不是給一個按了沒反應的開關。
  await expect(
    page.getByText("桌面角色設定需要桌面版控制中心（此為瀏覽器檢視）。").first()
  ).toBeVisible({ timeout: 20_000 });
  await expect(page.getByRole("checkbox", { name: "顯示桌面角色" })).toHaveCount(0);
  // 「隱藏」的語意也要講清楚：隱藏不等於緊急停止。
  await expect(page.getByText(/隱藏不等於緊急停止/)).toBeVisible();
  // 角色視窗沒接上就要說沒接上（不得用預設值冒充回報）。
  await expect(page.getByText("角色視窗未連線")).toBeVisible();

  // 一般模式不外洩角色描述檔原文（貼上 manifest 是進階模式的事）。
  await expect(page.getByLabel("角色描述檔內容")).toHaveCount(0);
  await expect(page.locator(".character-page textarea")).toHaveCount(0);

  // 按「停用」：畫面必須顯示錯誤，而且後端角色狀態不變、使用中的角色不變。
  const active = page.locator("article.character-card.active");
  await expect(active).toHaveCount(1);
  const activeName = await active.innerText();
  await active.getByRole("button", { name: "停用" }).click();
  await expect(page.locator(".character-page").getByRole("alert").first()).toBeVisible({
    timeout: 20_000,
  });
  await expect(active).toHaveCount(1);
  expect(await active.innerText()).toBe(activeName);
  expect(await characterState(request)).toBe(before);
});

// regression（CompanionPage.tsx 的 `patch()`）：舊版把錯誤吞成 error 狀態後仍 resolve，
// 停用失敗時畫面同時說「已停用目前角色，改用純文字角色」。送出 ≠ 完成：失敗只留錯誤。
test("角色：停用失敗時不得出現成功文案（只留誠實錯誤）", async ({ page }) => {
  test.setTimeout(120_000);
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  await navigateTo(page, COMPANION, false);
  const active = page.locator("article.character-card.active");
  await active.getByRole("button", { name: "停用" }).click();
  await expect(page.locator(".character-page").getByRole("alert").first()).toBeVisible({
    timeout: 20_000,
  });
  await expect(page.getByText("已停用目前角色")).toHaveCount(0);
});

test("角色：按其他角色的「選用」同樣不會偷偷換掉使用中的角色", async ({ page, request }) => {
  test.setTimeout(120_000);
  const before = await characterState(request);
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  await navigateTo(page, COMPANION, false);
  const candidate = page.locator("article.character-card:not(.active)").first();
  await expect(candidate).toBeVisible({ timeout: 20_000 });
  const pick = candidate.getByRole("button", { name: "選用" });
  await expect(pick).toBeVisible();
  await pick.click();
  await expect(page.locator(".character-page").getByRole("alert").first()).toBeVisible({
    timeout: 20_000,
  });
  await expect(page.locator("article.character-card.active")).toHaveCount(1);
  expect(await characterState(request)).toBe(before);

  // 390px 也是同一套誠實文案（不是桌面限定的說明）。
  await page.setViewportSize(NARROW);
  await expect(
    page.getByText("桌面角色設定需要桌面版控制中心（此為瀏覽器檢視）。").first()
  ).toBeVisible();
});
