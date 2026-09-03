// 記憶體浸泡量測入口（僅供 scripts/shu/perf-rig.mjs 打包使用，不進 App bundle）。
//
// perf-entry.ts 的 600 幀 heap 讀數只有幾秒，看不出「長時間跑會不會長」；這裡把
// 全舞台（角色＋2 使魔＋3 玩具＋2D 物理＋表情時間軸）用真 rAF 迴圈跑 ≥60 秒，
// 期間每隔幾秒抓一次玩具拖一下再放開（真的走 pointer 狀態路徑，不只是靜態重繪），
// 每 5 秒取樣一次 usedJSHeapSize；開頭（暖身後）與結尾各強制 GC 一次，報
// **GC 後差值**——那才是留下來的物件，GC 前的峰值只是分配節奏。
//
// 誠實：
//   - usedJSHeapSize 是 Blink 非標準 API；rig 以 --enable-precise-memory-info 啟動
//     才是精確位元組，否則 Chromium 會量化到 10 MB 級距（三個數字一模一樣）。
//     本檔會回報看起來像不像量化值，由 rig 決定要不要當證據。
//   - 這是 headless Chromium（Blink）數字，不是 Tauri WKWebView；同機同碼相對基準。
//   - 秒數由 rig 透過 window.__soakConfig.durationMs 注入（預設 60 000）。
//
// 輸出 JSON 到 window.__soak，並把 document.title 設成 "soak-ready"。

import { StageRenderer } from "./stage";

type Sample = { tMs: number; bytes: number | null };

function makeCanvas(w: number, h: number): HTMLCanvasElement {
  const c = document.createElement("canvas");
  c.style.width = `${w}px`;
  c.style.height = `${h}px`;
  c.width = w * (window.devicePixelRatio || 1);
  c.height = h * (window.devicePixelRatio || 1);
  document.body.appendChild(c);
  return c;
}

async function nextFrame(): Promise<number> {
  return new Promise((r) => requestAnimationFrame((t) => r(t)));
}

/** 目前的 JS heap（非標準 API；取不到就誠實回 null，不猜）。 */
function heapBytes(): number | null {
  const mem = (performance as unknown as { memory?: { usedJSHeapSize?: number } }).memory;
  return typeof mem?.usedJSHeapSize === "number" ? mem.usedJSHeapSize : null;
}

/**
 * 量化值的特徵：Chromium 未給精確資訊時，usedJSHeapSize 落在很粗的級距，
 * 每個讀數都是 100 000 的整數倍（實務上是 10 MB 級距）。精確位元組幾乎不可能
 * 每一個都剛好整除。只回報特徵，不下結論。
 */
function looksQuantized(values: Array<number | null>): boolean {
  const nums = values.filter((v): v is number => typeof v === "number");
  if (nums.length === 0) return false;
  return nums.every((v) => v % 100_000 === 0);
}

async function run() {
  const cfg = (window as unknown as { __soakConfig?: { durationMs?: number } }).__soakConfig ?? {};
  const durationMs = Number(cfg.durationMs) > 0 ? Number(cfg.durationMs) : 60_000;
  const SAMPLE_EVERY_MS = 5_000;
  const WARMUP_MS = 2_000;
  const gc = (globalThis as unknown as { gc?: () => void }).gc;
  const gcAvailable = typeof gc === "function";

  const c = makeCanvas(416, 216);
  const stage = new StageRenderer(c, "maid-classic", 1, { autoStart: false, rng: () => 0.37 });
  stage.setMachineFlags({ ambient: true, frozen: false, quiet: false, playPerforming: false });
  stage.setAnimation("idle");
  stage.setFamiliars([
    { id: "f1", name: "小白", palette: "maid-dusk" },
    { id: "f2", name: "小黑", palette: "maid-sakura" },
  ]);
  stage.spawnToy("yarn");
  stage.spawnToy("paper");
  stage.spawnToy("plane");

  // 暖身：第一批幀會建快取（sprite、路徑），不算成長。
  const warmStart = performance.now();
  while (performance.now() - warmStart < WARMUP_MS) {
    stage.renderFrame(await nextFrame());
  }
  if (gcAvailable) gc!();
  const baselineAfterGcBytes = heapBytes();

  const samples: Sample[] = [];
  const t0 = performance.now();
  let nextSampleAt = t0 + SAMPLE_EVERY_MS;
  let frames = 0;
  let peakBytes: number | null = null;
  // 互動節奏：每 2 秒抓最近的玩具、拖 ~300ms、放開；每 9 秒再生一個玩具（有上限）。
  let nextGrabAt = t0 + 2_000;
  let releaseAt: number | null = null;
  let nextSpawnAt = t0 + 9_000;
  let grabs = 0;
  let spawnAttempts = 0;

  for (;;) {
    const now = await nextFrame();
    frames += 1;
    stage.renderFrame(now);
    const wall = performance.now();
    if (releaseAt !== null) {
      if (wall >= releaseAt) {
        stage.pointerUp();
        stage.pointerLeave();
        releaseAt = null;
      } else {
        const toy = stage.toyPoints()[0];
        if (toy) stage.pointerMove(toy.x + 6, toy.y - 4);
      }
    } else if (wall >= nextGrabAt) {
      const toy = stage.toyPoints()[0];
      if (toy && stage.pointerDown(toy.x, toy.y) === "toy") {
        grabs += 1;
        releaseAt = wall + 300;
      }
      nextGrabAt = wall + 2_000;
    }
    if (wall >= nextSpawnAt) {
      stage.spawnToy(spawnAttempts % 2 === 0 ? "yarn" : "paper");
      spawnAttempts += 1;
      nextSpawnAt = wall + 9_000;
    }
    if (wall >= nextSampleAt) {
      const bytes = heapBytes();
      samples.push({ tMs: Math.round(wall - t0), bytes });
      if (typeof bytes === "number") peakBytes = peakBytes === null ? bytes : Math.max(peakBytes, bytes);
      nextSampleAt += SAMPLE_EVERY_MS;
    }
    if (wall - t0 >= durationMs) break;
  }
  const endBeforeGcBytes = heapBytes();
  if (gcAvailable) gc!();
  const endAfterGcBytes = heapBytes();
  const elapsedMs = performance.now() - t0;
  stage.destroy();
  c.remove();

  const deltaAfterGcBytes =
    typeof baselineAfterGcBytes === "number" && typeof endAfterGcBytes === "number"
      ? endAfterGcBytes - baselineAfterGcBytes
      : null;
  const quantized = looksQuantized([
    baselineAfterGcBytes,
    endBeforeGcBytes,
    endAfterGcBytes,
    ...samples.map((s) => s.bytes),
  ]);
  const out = {
    userAgent: navigator.userAgent,
    devicePixelRatio: window.devicePixelRatio || 1,
    durationMs: Math.round(elapsedMs),
    requestedDurationMs: durationMs,
    warmupMs: WARMUP_MS,
    frames,
    sampleEveryMs: SAMPLE_EVERY_MS,
    interactions: { toyGrabs: grabs, spawnAttempts, toysInWorld: stage.toyCount() },
    gcAvailable,
    baselineAfterGcBytes,
    endBeforeGcBytes,
    endAfterGcBytes,
    deltaAfterGcBytes,
    peakBytes,
    samples,
    source: "performance.memory.usedJSHeapSize（Blink 非標準 API）",
    looksQuantized: quantized,
    crossOriginIsolated: Boolean(
      (globalThis as unknown as { crossOriginIsolated?: boolean }).crossOriginIsolated
    ),
    note:
      baselineAfterGcBytes === null
        ? "performance.memory 不可用：這個引擎沒有給數字（不猜）"
        : quantized
          ? "讀數全是 100000 的整數倍：看起來是 Chromium 量化值，不是精確位元組——不能當記憶體證據"
          : "精確位元組（rig 以 --enable-precise-memory-info 啟動）；GC 後差值＝浸泡期間留下的物件",
  };
  (window as unknown as { __soak: unknown }).__soak = out;
  document.title = "soak-ready";
}

run().catch((e) => {
  (window as unknown as { __soak: unknown }).__soak = {
    error: String(e && (e as Error).stack ? (e as Error).stack : e),
  };
  document.title = "soak-ready";
});
