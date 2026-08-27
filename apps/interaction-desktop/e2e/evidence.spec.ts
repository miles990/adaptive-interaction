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
  { id: "activity", label: "活動與確認", marker: /待我決定/ },
  { id: "safety", label: "隱私與安全", marker: /緊急停止|同意/ },
  { id: "settings", label: "設定", marker: "顯示模式" },
];

async function openApp(page: Page) {
  await page.goto(appUrl());
  await expect(page.getByRole("navigation", { name: "主要導覽" })).toBeVisible({
    timeout: 15_000,
  });
}

test.describe.configure({ mode: "serial" });

test("擷取：每個一級頁（桌面 1200px）", async ({ page }) => {
  fs.mkdirSync(OUT, { recursive: true });
  await page.setViewportSize({ width: 1200, height: 800 });
  await openApp(page);
  for (const p of PAGES) {
    await page.getByRole("navigation", { name: "主要導覽" }).getByText(p.label, { exact: true }).click();
    await expect(page.getByText(p.marker).first()).toBeVisible({ timeout: 10_000 });
    await page.waitForTimeout(400);
    await page.screenshot({ path: path.join(OUT, `desktop-${p.id}.png`), fullPage: false });
  }
});

test("擷取：每個一級頁（390px 窄視窗）", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(appUrl());
  const bottomNav = page.getByRole("navigation", { name: "主要導覽（窄視窗）" });
  await expect(bottomNav).toBeVisible({ timeout: 15_000 });
  for (const p of PAGES) {
    // 主要四項直接點；其餘走「更多」。
    const direct = await bottomNav.getByText(p.label, { exact: true }).count();
    if (direct > 0) {
      await bottomNav.getByText(p.label, { exact: true }).click();
    } else {
      await bottomNav.getByRole("button", { name: "更多" }).click();
      const sheet = page.getByRole("dialog", { name: "更多功能" });
      await sheet.getByText(p.label, { exact: true }).click();
    }
    // 窄視窗以頁標題確認導覽成功（內容細節由桌面測試把關）。
    await expect(page.locator(".topbar-title")).toHaveText(p.label, { timeout: 10_000 });
    await page.waitForTimeout(300);
    await page.screenshot({ path: path.join(OUT, `narrow-${p.id}.png`) });
  }
});

test("擷取：全域搜尋與緊急停止狀態", async ({ page }) => {
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
  await page.waitForTimeout(300);
  await page.screenshot({ path: path.join(OUT, "desktop-emergency.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.waitForTimeout(300);
  await page.screenshot({ path: path.join(OUT, "narrow-emergency.png") });
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

test("擷取：Runtime 離線（誠實錯誤畫面）", async ({ page }) => {
  await page.setViewportSize({ width: 1200, height: 800 });
  await page.goto(`/?api=${encodeURIComponent("http://127.0.0.1:1")}&token=x`);
  await expect(page.getByText("系統無法啟動")).toBeVisible({ timeout: 20_000 });
  await page.screenshot({ path: path.join(OUT, "desktop-offline.png") });
});
