// 畫面證據（spec §16-1.M）：來自實際 App＋真實 Runtime 資料的截圖。
// 每個一級頁：桌面＋390px；另擷取 estop 與 offline 狀態。
// 新安裝的空狀態即「空白／初次使用」證據（誠實：不硬編假資料）。

import { test, expect, Page } from "@playwright/test";
import * as fs from "node:fs";
import * as path from "node:path";

const OUT = path.resolve(process.cwd(), "../../docs/assets/v04-evidence");

function appUrl(): string {
  return `/?api=${encodeURIComponent(process.env.E2E_API!)}&token=${encodeURIComponent(
    process.env.E2E_TOKEN!
  )}`;
}

const PAGES: { id: string; label: string; marker: string | RegExp }[] = [
  { id: "home", label: "首頁", marker: "權限地圖 — AI 現在可以做什麼？" },
  { id: "companion", label: "小樞", marker: "狀態預覽（取自實際角色素材）" },
  { id: "ai", label: "AI 與工作階段", marker: "本機 AI Agent" },
  { id: "capabilities", label: "能力與裝置", marker: "系統時間" },
  { id: "memory", label: "記憶與知識", marker: "小樞記住了什麼" },
  { id: "automations", label: "自動互動", marker: /自動互動|建立自動互動/ },
  { id: "activity", label: "活動與確認", marker: /統一收件匣/ },
  { id: "safety", label: "隱私與安全", marker: /緊急停止|同意/ },
  { id: "settings", label: "設定", marker: "顯示模式" },
];

async function openApp(page: Page) {
  await page.goto(appUrl());
  const wizard = page.getByRole("dialog", { name: "首次設定" });
  const desktopNav = page.getByRole("navigation", { name: "主要導覽" });
  await Promise.race([
    wizard.waitFor({ state: "visible", timeout: 15_000 }),
    desktopNav.waitFor({ state: "visible", timeout: 15_000 }),
  ]);
  if (await wizard.isVisible().catch(() => false)) {
    for (let step = 0; step < 6; step += 1) {
      await wizard.getByRole("button", { name: "下一步" }).click();
    }
    await wizard.getByRole("button", { name: "完成設定" }).click();
  }
  await expect(desktopNav).toBeVisible({ timeout: 15_000 });
}

async function navigateTo(page: Page, target: (typeof PAGES)[number], narrow: boolean) {
  if (!narrow) {
    await page
      .getByRole("navigation", { name: "主要導覽" })
      .getByText(target.label, { exact: true })
      .click();
  } else {
    const bottomNav = page.getByRole("navigation", { name: "主要導覽（窄視窗）" });
    const direct = bottomNav.getByText(target.label, { exact: true });
    if ((await direct.count()) > 0) {
      await direct.click();
    } else {
      await bottomNav.getByRole("button", { name: "更多" }).click();
      await page
        .getByRole("dialog", { name: "更多功能" })
        .getByText(target.label, { exact: true })
        .click();
    }
  }
  await expect(page.locator(".topbar-title")).toHaveText(target.label, { timeout: 10_000 });
}

async function capturePageMatrix(page: Page, state: string) {
  for (const [width, height, viewport] of [
    [1200, 800, "desktop"],
    [390, 844, "narrow"],
  ] as const) {
    await page.setViewportSize({ width, height });
    for (const target of PAGES) {
      await navigateTo(page, target, viewport === "narrow");
      await page.waitForTimeout(120);
      await page.screenshot({
        path: path.join(OUT, `${viewport}-${state}-${target.id}.png`),
        fullPage: false,
      });
    }
  }
}

test.describe.configure({ mode: "serial" });

test("擷取：每個一級頁（桌面 1200px）", async ({ page }) => {
  fs.mkdirSync(OUT, { recursive: true });
  await page.setViewportSize({ width: 1200, height: 800 });
  await openApp(page);
  for (const p of PAGES) {
    await navigateTo(page, p, false);
    await expect(page.getByText(p.marker).first()).toBeVisible({ timeout: 10_000 });
    await page.waitForTimeout(400);
    await page.screenshot({ path: path.join(OUT, `desktop-${p.id}.png`), fullPage: false });
    if (p.id === "capabilities") {
      await page.getByRole("tab", { name: "裝置與提供者" }).click();
      await page.getByRole("button", { name: "重新掃描" }).click();
      await expect(page.getByText(/感測器啟動：否/)).toBeVisible();
      await page.screenshot({ path: path.join(OUT, "desktop-hardware-scan.png"), fullPage: false });
    }
  }
});

test("擷取：每個一級頁（390px 窄視窗）", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(appUrl());
  await expect(page.getByRole("navigation", { name: "主要導覽（窄視窗）" })).toBeVisible({
    timeout: 15_000,
  });
  for (const p of PAGES) {
    // 主要四項直接點；其餘走「更多」。
    await navigateTo(page, p, true);
    // 窄視窗以頁標題確認導覽成功（內容細節由桌面測試把關）。
    await expect(page.locator(".topbar-title")).toHaveText(p.label, { timeout: 10_000 });
    await page.waitForTimeout(300);
    await page.screenshot({ path: path.join(OUT, `narrow-${p.id}.png`) });
    if (p.id === "capabilities") {
      await page.getByRole("tab", { name: "裝置與提供者" }).click();
      await page.getByRole("button", { name: "重新掃描" }).click();
      await expect(page.getByText(/感測器啟動：否/)).toBeVisible();
      await page.screenshot({ path: path.join(OUT, "narrow-hardware-scan.png") });
    }
  }
});

test("擷取：全域搜尋與緊急停止狀態", async ({ page }) => {
  test.setTimeout(90_000);
  await page.setViewportSize({ width: 1200, height: 800 });
  await openApp(page);
  // 全域搜尋。
  await page.keyboard.press("ControlOrMeta+k");
  await expect(page.getByRole("dialog", { name: "全域搜尋" })).toBeVisible();
  await page.screenshot({ path: path.join(OUT, "desktop-global-search.png") });
  await page.keyboard.press("Escape");
  // 緊急停止（真實觸發 → 擷取 → 走安全流程解除）。
  await page.getByRole("button", { name: "緊急停止", exact: true }).click();
  await page.getByRole("button", { name: "立即停止一切？" }).click();
  await expect(page.getByText("緊急停止已啟動").first()).toBeVisible();
  await capturePageMatrix(page, "emergency");
  await page.setViewportSize({ width: 1200, height: 800 });
  // 解除（讓 suite 保持乾淨收尾）。
  await page.getByRole("button", { name: /緊急停止中 — 前往解除/ }).click();
  await page.getByRole("button", { name: /開始安全解除流程/ }).click();
  const dialog = page.getByRole("dialog", { name: "解除緊急停止" });
  await dialog.getByRole("button", { name: "我了解，解除緊急停止" }).click();
  await dialog.getByRole("button", { name: "確定解除？" }).click();
  await expect(page.getByRole("button", { name: "緊急停止", exact: true })).toBeVisible({
    timeout: 10_000,
  });
});

test("擷取：每個一級頁的真實待確認狀態", async ({ page }) => {
  test.setTimeout(90_000);
  const response = await page.request.post(`${process.env.E2E_API!}/v1/knowledge/nodes`, {
    headers: { Authorization: `Bearer ${process.env.E2E_TOKEN!}` },
    data: {
      title: "畫面驗收候選",
      content: "這筆候選由實際 Runtime 保存，只用於驗證統一待辦入口。",
      domains: ["acceptance-evidence"],
      asAgent: "evidence-reviewer",
      evidence: [{ url: "https://example.invalid/acceptance", segment: "local-test" }],
    },
  });
  expect(response.ok()).toBeTruthy();

  await page.setViewportSize({ width: 1200, height: 800 });
  await openApp(page);
  await expect(page.getByRole("button", { name: /通知中心，[1-9][0-9]* 項待決定/ })).toBeVisible({
    timeout: 15_000,
  });
  await capturePageMatrix(page, "waiting");
});

test("擷取：每個一級頁的實際載入狀態", async ({ page }) => {
  test.setTimeout(120_000);
  fs.mkdirSync(OUT, { recursive: true });
  await page.setViewportSize({ width: 1200, height: 800 });
  await openApp(page);

  for (const [width, height, viewport] of [
    [1200, 800, "desktop"],
    [390, 844, "narrow"],
  ] as const) {
    await page.setViewportSize({ width, height });
    for (const target of PAGES) {
      const away = target.id === "home" ? PAGES[PAGES.length - 1] : PAGES[0];
      await navigateTo(page, away, viewport === "narrow");
      const delayed = async (route: import("@playwright/test").Route) => {
        if (route.request().url().includes("/events/stream")) return route.continue();
        await new Promise((resolve) => setTimeout(resolve, 700));
        // React can unmount a page while one of its delayed requests is still
        // pending. In that case Chromium has already cancelled the route; it
        // is not an acceptance failure and must not fail the screenshot run.
        try {
          await route.continue();
        } catch (error) {
          if (!String(error).includes("already handled")) throw error;
        }
      };
      await page.route("**/v1/**", delayed);
      await navigateTo(page, target, viewport === "narrow");
      await page.waitForTimeout(80);
      await page.screenshot({
        path: path.join(OUT, `${viewport}-loading-${target.id}.png`),
        fullPage: false,
      });
      await page.waitForTimeout(750);
      await page.unroute("**/v1/**", delayed);
    }
  }
});

test("擷取：每個一級頁的傳輸錯誤／未知狀態", async ({ page }) => {
  test.setTimeout(120_000);
  fs.mkdirSync(OUT, { recursive: true });
  for (const [width, height, viewport] of [
    [1200, 800, "desktop"],
    [390, 844, "narrow"],
  ] as const) {
    await page.setViewportSize({ width, height });
    if (viewport === "desktop") await openApp(page);
    else await page.goto(appUrl());
    await expect(
      viewport === "narrow"
        ? page.getByRole("navigation", { name: "主要導覽（窄視窗）" })
        : page.getByRole("navigation", { name: "主要導覽" })
    ).toBeVisible({ timeout: 15_000 });

    // Inject a real transport failure (no fake response payload), run an
    // actual permission-aware command, and preserve the resulting honest
    // global error banner while visiting every page.
    await page.route("**/v1/sensors/stop", (route) => route.abort("connectionrefused"));
    await page.keyboard.press("ControlOrMeta+k");
    const search = page.getByRole("dialog", { name: "全域搜尋" });
    await search.getByPlaceholder(/搜尋設定/).fill("停止所有感測");
    await search.getByRole("option", { name: /停止所有感測/ }).first().click();
    await expect(page.getByRole("alert")).toContainText("停止所有感測", { timeout: 10_000 });
    await page.unroute("**/v1/sensors/stop");

    for (const target of PAGES) {
      await navigateTo(page, target, viewport === "narrow");
      await expect(page.getByRole("alert")).toBeVisible();
      await page.screenshot({
        path: path.join(OUT, `${viewport}-error-unknown-${target.id}.png`),
        fullPage: false,
      });
    }
  }
});

test("擷取：Runtime 離線（誠實錯誤畫面）", async ({ page }) => {
  await page.setViewportSize({ width: 1200, height: 800 });
  await page.goto(`/?api=${encodeURIComponent("http://127.0.0.1:1")}&token=x`);
  await expect(page.getByText("系統無法啟動")).toBeVisible({ timeout: 20_000 });
  await page.screenshot({ path: path.join(OUT, "desktop-offline.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: path.join(OUT, "narrow-offline.png") });
});
