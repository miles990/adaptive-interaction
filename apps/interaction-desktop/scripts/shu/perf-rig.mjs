// 角色／遊玩場效能量測（規格 §14／§18-20）：打包 perf-entry → 無頭 Chromium 執行 → JSON。
// 用法：node scripts/shu/perf-rig.mjs [outJson]
//
// 這是可重現的量測腳本；docs/ 內引用的效能數字必須由它產生（附 userAgent 與時間）。
// 誠實：headless Chromium（Blink）≠ Tauri WKWebView（WebKit）；同碼同機的相對基準。

import { createRequire } from "node:module";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const appRoot = path.resolve(here, "..", "..");
const require_ = createRequire(path.join(appRoot, "package.json"));
// esbuild 是 vite 的傳遞依賴：先照正常解析找，找不到才退回 pnpm store 的硬編路徑
// （硬編版本一升級就會壞，只當備援）。
const esbuild = loadEsbuild();
const { chromium } = require_("@playwright/test");

function loadEsbuild() {
  try {
    return require_(require_.resolve("esbuild"));
  } catch {
    return require_(
      path.join(appRoot, "node_modules/.pnpm/esbuild@0.21.5/node_modules/esbuild")
    );
  }
}

// 預設輸出到暫存目錄，不在 repo 根目錄留檔（要留就自己給路徑）。
const out = process.argv[2] ?? path.join(tmpdir(), "rig-perf.json");
const work = mkdtempSync(path.join(tmpdir(), "rig-perf-"));

const bundle = await esbuild.build({
  entryPoints: [path.join(appRoot, "src/companion/rig/perf-entry.ts")],
  bundle: true,
  format: "iife",
  write: false,
  target: "es2020",
});
const js = bundle.outputFiles[0].text;
const html = `<!doctype html><meta charset="utf-8"><body style="margin:0"><script>${js}</script></body>`;
const htmlPath = path.join(work, "perf.html");
writeFileSync(htmlPath, html);

// --expose-gc 讓量測可以在 GC 後再讀一次 heap；拿不到就誠實回報 gcAvailable:false。
const browser = await chromium.launch({ args: ["--js-flags=--expose-gc"] });
const page = await browser.newPage({ viewport: { width: 800, height: 600 }, deviceScaleFactor: 2 });
await page.goto(`file://${htmlPath}`);
await page.waitForFunction(() => document.title === "perf-ready", null, { timeout: 180_000 });
const result = await page.evaluate(() => window.__perf);
await browser.close();
writeFileSync(out, JSON.stringify(result, null, 2));
const r = result ?? {};
const fmt = (s) => (s ? `median ${s.medianMs.toFixed(3)} ms / p95 ${s.p95Ms.toFixed(3)} ms / max ${s.maxMs.toFixed(3)} ms (n=${s.n})` : "n/a");
console.log(`drawRig      : ${fmt(r.drawRig)}`);
console.log(`stage frame  : ${fmt(r.stage)}`);
console.log(`rAF gap      : ${fmt(r.stage?.rafGap)}`);
console.log(
  `toy grab lat : ${fmt(r.inputLatencyToyGrab)} (confirmed ${r.inputLatencyToyGrab?.confirmedFrames}/${r.inputLatencyToyGrab?.attempts})`
);
console.log(
  `gaze latency : ${fmt(r.inputLatencyGaze)} (confirmed ${r.inputLatencyGaze?.confirmedFrames}/${r.inputLatencyGaze?.attempts})`
);
const mb = (v) => (typeof v === "number" ? `${(v / 1048576).toFixed(1)} MB` : "n/a");
console.log(
  `heap         : ${mb(r.memory?.beforeFramesBytes)} → ${mb(r.memory?.afterFramesBytes)} → ${mb(r.memory?.afterGcBytes)} after gc (gc ${r.memory?.gcAvailable ? "available" : "unavailable"})`
);
console.log(`bounded toys : cap=${r.boundedQueue?.toyCap} of ${r.boundedQueue?.spawnedAttempts} spawns`);
console.log(`3-day run    : finite=${r.longRun?.allFinite} withinClamp=${r.longRun?.allWithinClamp}`);
if (r.error) console.error(r.error);
console.log(`written ${out}`);
