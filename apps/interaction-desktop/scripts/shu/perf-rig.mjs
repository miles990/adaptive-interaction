// 角色／遊玩場效能量測（規格 §14／§18-20）：打包 perf-entry → 無頭 Chromium 執行 → JSON。
// 用法：node scripts/shu/perf-rig.mjs [outJson]
//   環境變數 PERF_SOAK_MS：記憶體浸泡秒數（預設 60000；低於 60 秒只當除錯，不算證據）。
//
// 這是可重現的量測腳本；docs/ 內引用的效能數字必須由它產生（附 userAgent 與時間）。
// 誠實：headless Chromium（Blink）≠ Tauri WKWebView（WebKit）；同碼同機的相對基準。
// 量測範圍：全部在 WebView 內（合成 pointer 呼叫→下一幀）；Rust 端點擊穿透閘
// （游標輪詢＋hit-rect 回報）與 OS 派送不在量測內，端到端未量。

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

// 記憶體浸泡長度：證據等級是 ≥60 秒；PERF_SOAK_MS 可縮短除錯，但輸出會標明不算證據。
const SOAK_EVIDENCE_MIN_MS = 60_000;
const soakMs = Number.parseInt(process.env.PERF_SOAK_MS ?? "", 10) > 0
  ? Number.parseInt(process.env.PERF_SOAK_MS, 10)
  : SOAK_EVIDENCE_MIN_MS;

async function bundleToHtml(entryRel, fileName) {
  const bundle = await esbuild.build({
    entryPoints: [path.join(appRoot, entryRel)],
    bundle: true,
    format: "iife",
    write: false,
    target: "es2020",
  });
  const js = bundle.outputFiles[0].text;
  const html = `<!doctype html><meta charset="utf-8"><body style="margin:0"><script>${js}</script></body>`;
  const htmlPath = path.join(work, fileName);
  writeFileSync(htmlPath, html);
  return htmlPath;
}

const perfHtml = await bundleToHtml("src/companion/rig/perf-entry.ts", "perf.html");
const soakHtml = await bundleToHtml("src/companion/rig/perf-soak-entry.ts", "soak.html");

// 啟動旗標：
//   --js-flags=--expose-gc         讓量測可以在 GC 後再讀一次 heap（拿不到就誠實回報 gcAvailable:false）。
//   --enable-precise-memory-info   關掉 Chromium 對 performance.memory 的量化（否則
//                                  usedJSHeapSize 被壓到 10 MB 級距，三個讀數一模一樣，沒有判讀力）。
const LAUNCH_ARGS = ["--js-flags=--expose-gc", "--enable-precise-memory-info"];
const browser = await chromium.launch({ args: LAUNCH_ARGS });

// 1) 幀成本／輸入延遲／600 幀 heap（perf-entry）。
const page = await browser.newPage({ viewport: { width: 800, height: 600 }, deviceScaleFactor: 2 });
await page.goto(`file://${perfHtml}`);
await page.waitForFunction(() => document.title === "perf-ready", null, { timeout: 180_000 });
const result = (await page.evaluate(() => window.__perf)) ?? {};
await page.close();

// 2) 記憶體浸泡：全舞台真 rAF 跑 ≥60 秒＋週期性抓玩具，另加 Gateway／Director／
//    behavior／記憶／事件環的 500 ms pump（perf-claims-009），GC 後差值（perf-soak-entry）。
const soakPage = await browser.newPage({ viewport: { width: 800, height: 600 }, deviceScaleFactor: 2 });
await soakPage.addInitScript((durationMs) => {
  window.__soakConfig = { durationMs };
}, soakMs);
await soakPage.goto(`file://${soakHtml}`);
await soakPage.waitForFunction(() => document.title === "soak-ready", null, {
  timeout: soakMs + 120_000,
});
const soak = (await soakPage.evaluate(() => window.__soak)) ?? {};
await soakPage.close();
await browser.close();

result.launchArgs = LAUNCH_ARGS;
result.memorySoak = { ...soak, evidenceGrade: (soak.durationMs ?? 0) >= SOAK_EVIDENCE_MIN_MS };
writeFileSync(out, JSON.stringify(result, null, 2));

const r = result;
const fmt = (s) => (s ? `median ${s.medianMs.toFixed(3)} ms / p95 ${s.p95Ms.toFixed(3)} ms / max ${s.maxMs.toFixed(3)} ms (n=${s.n})` : "n/a");
console.log(`drawRig      : ${fmt(r.drawRig)}`);
console.log(`stage frame  : ${fmt(r.stage)}`);
console.log(`rAF gap      : ${fmt(r.stage?.rafGap)}`);
const num = (v) => (typeof v === "number" ? v.toFixed(3) : "n/a");
// 真 rAF 主迴圈＋幀預算：skipEveryOther=true 代表舞台自己降到 30fps（零成本下必須是 false）。
console.log(
  `stage loop   : ticks=${r.stageLoop?.ticks ?? "n/a"} drawn=${r.stageLoop?.drawn ?? "n/a"} skipEveryOther=${r.stageLoop?.skipEveryOther ?? "n/a"} lastWindowAvgCost=${num(r.stageLoop?.lastWindowAvgCostMs)} ms (rAF gap ${fmt(r.stageLoop?.rafGap)})`
);
console.log(
  `reduced loop : ticks=${r.reducedMotionLoop?.ticks ?? "n/a"} drawn=${r.reducedMotionLoop?.drawn ?? "n/a"} (${num(r.reducedMotionLoop?.drawnPerSecond)} draws/s over ${num((r.reducedMotionLoop?.elapsedMs ?? 0) / 1000)} s) — Reduced Motion 靜態短路`
);
console.log(
  `toy grab lat : ${fmt(r.inputLatencyToyGrab)} (confirmed ${r.inputLatencyToyGrab?.confirmedFrames}/${r.inputLatencyToyGrab?.attempts}; WebView-only segment, host click-through gate not included)`
);
console.log(
  `gaze latency : ${fmt(r.inputLatencyGaze)} (confirmed ${r.inputLatencyGaze?.confirmedFrames}/${r.inputLatencyGaze?.attempts}; WebView-only segment)`
);

// 量化偵測：精確位元組幾乎不可能每一個都是 100000 的整數倍；全部整除＝Chromium 量化值。
const isNum = (v) => typeof v === "number";
const looksQuantized = (vals) => {
  const nums = vals.filter(isNum);
  return nums.length > 0 && nums.every((v) => v % 100_000 === 0);
};
const mb = (v) => (isNum(v) ? `${(v / 1048576).toFixed(2)} MB` : "n/a");
const kbDelta = (v) => (isNum(v) ? `${v >= 0 ? "+" : "-"}${(Math.abs(v) / 1024).toFixed(0)} KB` : "n/a");
const m = r.memory ?? {};
const memQuantized = looksQuantized([m.beforeFramesBytes, m.afterFramesBytes, m.afterGcBytes]);
console.log(
  `heap (600 f) : ${mb(m.beforeFramesBytes)} → ${mb(m.afterFramesBytes)} → ${mb(m.afterGcBytes)} after gc (gc ${m.gcAvailable ? "available" : "unavailable"}; source usedJSHeapSize, --enable-precise-memory-info, quantized=${memQuantized ? "YES" : "no"})`
);
const s = r.memorySoak ?? {};
const pct =
  isNum(s.deltaAfterGcBytes) && isNum(s.baselineAfterGcBytes) && s.baselineAfterGcBytes > 0
    ? `${((s.deltaAfterGcBytes / s.baselineAfterGcBytes) * 100).toFixed(1)}%`
    : "n/a";
console.log(
  `heap soak    : ${((s.durationMs ?? 0) / 1000).toFixed(1)} s / ${s.frames ?? "n/a"} frames / ${s.interactions?.toyGrabs ?? "n/a"} toy grabs; after-gc ${mb(s.baselineAfterGcBytes)} → ${mb(s.endAfterGcBytes)} (Δ ${kbDelta(s.deltaAfterGcBytes)}, ${pct}); peak before gc ${mb(s.peakBytes)}; samples every ${((s.sampleEveryMs ?? 0) / 1000).toFixed(0)} s: ${(s.samples ?? []).map((x) => mb(x.bytes).replace(" MB", "")).join(", ")} MB; quantized=${s.looksQuantized ? "YES" : "no"}; evidence-grade=${s.evidenceGrade ? "yes (≥60 s)" : "NO (<60 s)"}`
);
const al = s.appLayer ?? {};
const bd = al.bounded ?? {};
console.log(
  `soak scope   : ${s.scope ?? "n/a"}`
);
console.log(
  `soak app tier: pumps=${al.pumps ?? "n/a"} intents=${al.intents ?? "n/a"} inputs=${al.inputEvents ?? "n/a"} receipts=${al.receipts ?? "n/a"} director-actions=${al.directorActions ?? "n/a"}; bounded: instances=${bd.gatewayInstances ?? "n/a"} inputQueue=${bd.gatewayInputQueue ?? "n/a"} grants=${bd.gatewayGrants ?? "n/a"} decisions=${bd.directorDecisions ?? "n/a"} eventRing=${bd.eventRing ?? "n/a"}/${bd.eventRingMax ?? "n/a"} memToys=${bd.memoryToys ?? "n/a"} memEvents=${bd.memoryEvents ?? "n/a"}`
);
console.log(`bounded toys : cap=${r.boundedQueue?.toyCap} of ${r.boundedQueue?.spawnedAttempts} spawns`);
console.log(`3-day run    : finite=${r.longRun?.allFinite} withinClamp=${r.longRun?.allWithinClamp}`);
if (r.error) console.error(r.error);
if (s.error) console.error(s.error);
console.log(`written ${out}`);

// 自我檢查：heap 讀數若仍是量化值，這份輸出不能拿去當記憶體證據——直接以非零結束，
// 免得 docs 又引用 10 MB 級距的桶值。
if (memQuantized || s.looksQuantized) {
  console.error(
    "perf-rig: usedJSHeapSize 讀數是量化值（--enable-precise-memory-info 沒生效？）；記憶體數字不可引用。"
  );
  process.exitCode = 1;
}
