// Offline honesty: when the runtime is unreachable the UI must say so —
// never render a fake-working control center.

import { test, expect } from "@playwright/test";

test("Runtime 離線：誠實顯示無法啟動，不假裝正常", async ({ page }) => {
  // Point at a dead port; no daemon listens there.
  await page.goto("/?api=http%3A%2F%2F127.0.0.1%3A19999&token=none");
  await expect(page.getByText("系統無法啟動")).toBeVisible({ timeout: 20_000 });
  // No permission map is shown while offline.
  await expect(page.getByText("AI 可以知道", { exact: true })).toHaveCount(0);
});
