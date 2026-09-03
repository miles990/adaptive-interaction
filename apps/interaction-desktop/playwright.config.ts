// Browser-level E2E: the real UI (vite dev server, HTTP transport) against a
// REAL `interact-ai` daemon in an isolated home. No mock backend — the same
// policy governor and runtime the desktop app uses.

import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  globalSetup: "./e2e/global-setup.ts",
  globalTeardown: "./e2e/global-teardown.ts",
  // The suite mutates one shared runtime (onboarding → flows); keep it serial.
  fullyParallel: false,
  workers: 1,
  timeout: 30_000,
  retries: 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : [["list"]],
  use: {
    baseURL: "http://127.0.0.1:5199",
    trace: "retain-on-failure",
  },
  // 順序有意義（單一 worker、單一 daemon），而且不能只靠檔名字母序：
  //  1. first-run —— 首次設定精靈只在 onboardingCompleted=false 時出現，所以
  //     app.spec 必須跑在任何會完成精靈的 spec 之前。
  //  2. main —— 其餘全部（含 evidence 的截圖矩陣）。
  //  3. estop-last —— 緊急停止會撤銷同意、取消進行中的工作、停掉感測，
  //     放最後才不會污染別人的狀態。
  projects: [
    { name: "first-run", testMatch: /app\.spec\.ts$/ },
    {
      name: "main",
      testIgnore: [/app\.spec\.ts$/, /estop\.spec\.ts$/],
      dependencies: ["first-run"],
    },
    { name: "estop-last", testMatch: /estop\.spec\.ts$/, dependencies: ["main"] },
  ],
  webServer: {
    // In CI, serve the prebuilt dist (fast, no esbuild optimizeDeps cold
    // start that can exceed the timeout on a shared runner); locally use the
    // dev server and reuse one if already running.
    // Bind 127.0.0.1 explicitly: on Linux CI `localhost` can resolve to ::1
    // first, so a server on localhost would never answer the 127.0.0.1 probe.
    command: process.env.CI
      ? "pnpm exec vite preview --host 127.0.0.1 --port 5199 --strictPort"
      : "pnpm exec vite --host 127.0.0.1 --port 5199 --strictPort",
    url: "http://127.0.0.1:5199",
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
    stdout: "pipe",
    stderr: "pipe",
  },
});
