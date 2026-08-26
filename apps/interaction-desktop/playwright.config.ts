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
  use: {
    baseURL: "http://127.0.0.1:5199",
    trace: "retain-on-failure",
  },
  webServer: {
    // In CI, serve the prebuilt dist (fast, no esbuild optimizeDeps cold
    // start that can exceed the timeout on a shared runner); locally use the
    // dev server and reuse one if already running.
    command: process.env.CI
      ? "pnpm exec vite preview --port 5199 --strictPort"
      : "pnpm exec vite --port 5199 --strictPort",
    url: "http://127.0.0.1:5199",
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
    stdout: "pipe",
    stderr: "pipe",
  },
});
