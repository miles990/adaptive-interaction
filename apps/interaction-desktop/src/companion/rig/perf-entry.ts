// 效能量測入口（僅供 scripts/shu/perf-rig.mjs 打包使用，不進 App bundle）。
//
// 在真 Chromium（headless）裡量測規格 §14 關心的數字，輸出 JSON 到
// `window.__perf` 並把 document.title 設成 "perf-ready"：
//   - drawRig：單角色一幀的純繪製成本（ms/幀，中位數與 p95；**含 raster**：
//     每批結束用 getImageData 強制 flush，不只是錄 canvas 指令）。
//   - stage：整個遊玩場（角色＋2 使魔＋3 玩具＋2D 物理＋表情時間軸）一幀成本
//     （同樣含 raster flush）。
//   - stageLoop：StageRenderer **自己的** requestAnimationFrame 主迴圈（loop()＋幀預算）
//     跑 360 個 rAF：ticks／drawn／skipEveryOther——舞台有沒有被自己的幀預算降到
//     30fps（對抗審查 perf-claims-017：以前餵 rAF 間隔，60Hz 螢幕上一秒後永久 30fps）。
//   - inputLatency：兩個**真的會改變狀態**的輸入路徑——
//       toyGrab：pointerDown 於玩具 → 下一幀該玩具 grabbed=player；
//       gaze：pointerMove 進入角色 hit-rect → 下一幀視線/耳朵參數改變。
//     （角色本體的拖曳走 Tauri 原生視窗拖曳，網頁端沒有狀態可量——量它只會
//     量到 rAF 間隔，不是輸入延遲。）
//   - memory：600 幀前後與 GC 後的 usedJSHeapSize（不可用時誠實回 null）。
//   - blinkDrift：時間軸跑「3 天」後參數仍有限且在 clamp 範圍（長時間執行數值行為）。
//   - boundedQueue：世界內玩具上限（規格「bounded queues」的可觀測面）。
//
// 誠實：這是 headless Chromium 的 CPU 數字，不是 Tauri WebView 的實機數字；
// 兩者同引擎（WebKit vs Blink 不同）——把它當「同機同碼可重現的相對基準」，
// 不要當作使用者機器的絕對值。

import { drawRig } from "./draw";
import { clampParams, DEFAULT_PARAMS, RIG_PALETTES } from "./params";
import { ExpressionTimeline } from "./timeline";
import { StageRenderer } from "./stage";
import { OFFICIAL_36 } from "./expressions";

type Stats = { n: number; medianMs: number; p95Ms: number; meanMs: number; maxMs: number };

function stats(samples: number[]): Stats {
  const s = [...samples].sort((a, b) => a - b);
  const q = (p: number) => s[Math.min(s.length - 1, Math.floor(p * s.length))] ?? 0;
  const mean = s.reduce((a, b) => a + b, 0) / Math.max(1, s.length);
  return { n: s.length, medianMs: q(0.5), p95Ms: q(0.95), meanMs: mean, maxMs: s[s.length - 1] ?? 0 };
}

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

/** 強制把錄下的 canvas 指令真的 raster 出來（否則只量到指令錄製成本）。 */
function flushRaster(ctx: CanvasRenderingContext2D): void {
  ctx.getImageData(0, 0, 1, 1);
}

/** 目前的 JS heap（非標準 API；取不到就誠實回 null，不猜）。 */
function heapBytes(): number | null {
  const mem = (performance as unknown as { memory?: { usedJSHeapSize?: number } }).memory;
  return typeof mem?.usedJSHeapSize === "number" ? mem.usedJSHeapSize : null;
}

async function run() {
  const out: Record<string, unknown> = {
    userAgent: navigator.userAgent,
    devicePixelRatio: window.devicePixelRatio || 1,
    startedAt: new Date().toISOString(),
  };

  // 1) drawRig：160px 角色（桌面預設尺寸）× 36 表情 hold 參數輪流，共 720 幀。
  {
    const c = makeCanvas(160, 200);
    const ctx = c.getContext("2d")!;
    const pal = RIG_PALETTES["maid-classic"];
    const tl = new ExpressionTimeline(() => 0.5, 0);
    // headless Chromium 的 performance.now() 解析度約 0.1ms，所以每個樣本量
    // 10 幀再除以 10（每幀參數仍各自不同，不是重畫同一幀）。
    const BATCH = 10;
    const samples: number[] = [];
    let t = 0;
    for (let i = 0; i < 72; i++) {
      const id = OFFICIAL_36[i % OFFICIAL_36.length];
      if (i % 2 === 0) tl.setAnimation(id, t);
      const frames = [];
      for (let k = 0; k < BATCH; k++) {
        t += 16.67;
        frames.push(tl.paramsAt(t));
      }
      const t0 = performance.now();
      for (const params of frames) {
        ctx.setTransform(window.devicePixelRatio || 1, 0, 0, window.devicePixelRatio || 1, 0, 0);
        ctx.clearRect(0, 0, 160, 200);
        drawRig(ctx, params, pal);
      }
      flushRaster(ctx); // 含 raster：不讓 GPU 把整批指令延後到量測窗外
      samples.push((performance.now() - t0) / BATCH);
    }
    out.drawRig = {
      canvasCss: "160x200",
      framesPerSample: BATCH,
      framesTotal: 72 * BATCH,
      includesRaster: true,
      ...stats(samples),
    };
    c.remove();
  }

  // 2) 全舞台：角色＋2 使魔＋3 玩具，真 rAF 迴圈 600 幀（含物理與時間軸）。
  {
    const c = makeCanvas(416, 216);
    const sctx = c.getContext("2d")!;
    const stage = new StageRenderer(c, "maid-classic", 1, { autoStart: false, rng: () => 0.37 });
    stage.setMachineFlags({ ambient: true, frozen: false, quiet: false, playPerforming: false });
    stage.setAnimation("idle"); // 沒有真相狀態搶佔：遊玩與注視都算數
    stage.setFamiliars([
      { id: "f1", name: "小白", palette: "maid-dusk" },
      { id: "f2", name: "小黑", palette: "maid-sakura" },
    ]);
    stage.spawnToy("yarn");
    stage.spawnToy("paper");
    stage.spawnToy("plane");
    // 真 rAF 迴圈 600 幀量 rAF 間隔；每幀成本另以 5 幀一批（時間各自前進）
    // 量 120 批，避開 0.1ms 計時解析度。
    const frameGaps: number[] = [];
    const heapBeforeFrames = heapBytes();
    let prev = await nextFrame();
    for (let i = 0; i < 600; i++) {
      const now = await nextFrame();
      frameGaps.push(now - prev);
      prev = now;
      stage.renderFrame(now);
    }
    const heapAfterFrames = heapBytes();
    const gc = (globalThis as unknown as { gc?: () => void }).gc;
    if (typeof gc === "function") gc();
    const heapAfterGc = typeof gc === "function" ? heapBytes() : null;
    out.memory = {
      beforeFramesBytes: heapBeforeFrames,
      afterFramesBytes: heapAfterFrames,
      afterGcBytes: heapAfterGc,
      gcAvailable: typeof gc === "function",
      frames: 600,
      // 未 cross-origin-isolated 時 Chrome 會把 usedJSHeapSize 量化到很粗的
      // 級距（常常三個數字一模一樣）——那是隱私保護，不是「零配置」。
      crossOriginIsolated: Boolean(
        (globalThis as unknown as { crossOriginIsolated?: boolean }).crossOriginIsolated
      ),
      note:
        heapBeforeFrames === null
          ? "performance.memory 不可用：這個引擎沒有給數字（不猜）"
          : "usedJSHeapSize（Blink 非標準 API）；未 crossOriginIsolated 時數值被量化，只能看數量級",
    };
    const samples: number[] = [];
    let simNow = performance.now();
    for (let i = 0; i < 120; i++) {
      const t0 = performance.now();
      for (let k = 0; k < 5; k++) {
        simNow += 16.67;
        stage.renderFrame(simNow);
      }
      flushRaster(sctx); // 含 raster
      samples.push((performance.now() - t0) / 5);
      await nextFrame();
    }
    out.stage = {
      canvasCss: "416x216",
      familiars: 2,
      toys: stage.toyCount(),
      framesPerSample: 5,
      includesRaster: true,
      ...stats(samples),
      rafGap: { ...stats(frameGaps), note: "headless Chromium 的 rAF 節奏，非使用者螢幕更新率" },
    };

    // 3) 輸入延遲：只量「真的會改變狀態」的兩條路徑。
    //    (a) pointerDown 於玩具 → 下一幀該玩具 grabbed=player。
    const grabLat: number[] = [];
    let grabConfirmed = 0;
    for (let i = 0; i < 20; i++) {
      const toy = stage.toyPoints()[0];
      if (!toy) break;
      const t0 = performance.now();
      const hit = stage.pointerDown(toy.x, toy.y);
      const now = await nextFrame();
      stage.renderFrame(now);
      const grabbed = stage.playerGrabbedToys() > 0;
      grabLat.push(performance.now() - t0);
      if (grabbed && hit === "toy") grabConfirmed += 1;
      stage.pointerUp();
      stage.renderFrame(await nextFrame());
    }
    out.inputLatencyToyGrab = {
      ...stats(grabLat),
      confirmedFrames: grabConfirmed,
      attempts: grabLat.length,
      target: "16-100ms (spec §14)",
      measures: "pointerDown 於玩具 → 下一幀 world 中該玩具 grabbed=player",
    };

    //    (b) pointerMove 進入角色 hit-rect → 下一幀視線/耳朵參數改變。
    const gazeLat: number[] = [];
    let gazeConfirmed = 0;
    const rect = stage.charHitRect();
    for (let i = 0; i < 20; i++) {
      stage.pointerLeave();
      stage.renderFrame(await nextFrame());
      const before = stage.lastFrameParams();
      const t0 = performance.now();
      stage.pointerMove(rect.x + rect.w * 0.9, rect.y + rect.h * 0.3);
      stage.renderFrame(await nextFrame());
      const after = stage.lastFrameParams();
      gazeLat.push(performance.now() - t0);
      if (
        before &&
        after &&
        (before.pupilX !== after.pupilX ||
          before.headTurn !== after.headTurn ||
          before.earLTilt !== after.earLTilt)
      ) {
        gazeConfirmed += 1;
      }
    }
    stage.pointerLeave();
    out.inputLatencyGaze = {
      ...stats(gazeLat),
      confirmedFrames: gazeConfirmed,
      attempts: gazeLat.length,
      target: "16-100ms (spec §14)",
      measures: "pointerMove 進入角色 hit-rect → 下一幀 pupilX/headTurn/earLTilt 改變",
    };

    // 5) bounded：玩具上限（規格 bounded queue 的可觀測面）。
    for (let i = 0; i < 20; i++) stage.spawnToy("yarn");
    out.boundedQueue = { toyCap: stage.toyCount(), spawnedAttempts: 23 };
    stage.destroy();
    c.remove();
  }

  // 2b) 真 rAF 主迴圈（loop()＋幀預算）：上面 2) 直呼 renderFrame 繞過了幀預算，
  //     量不到「舞台有沒有被自己的預算降到 30fps」。這裡讓 StageRenderer 自己跑
  //     requestAnimationFrame 360 幀（6 個 60 幀窗），回報 ticks／drawn／skipEveryOther。
  //     零成本、任何 rAF 節奏下 skipEveryOther 都必須是 false（幀預算餵的是 renderFrame
  //     成本，不是 rAF 間隔）。
  {
    const c = makeCanvas(416, 216);
    const stage = new StageRenderer(c, "maid-classic", 1, { rng: () => 0.37 }); // autoStart 預設：真迴圈
    stage.setMachineFlags({ ambient: true, frozen: false, quiet: false, playPerforming: false });
    stage.setAnimation("idle");
    stage.setFamiliars([
      { id: "f1", name: "小白", palette: "maid-dusk" },
      { id: "f2", name: "小黑", palette: "maid-sakura" },
    ]);
    stage.spawnToy("yarn");
    stage.spawnToy("paper");
    stage.spawnToy("plane");
    const gaps: number[] = [];
    let prev = await nextFrame();
    for (let i = 0; i < 360; i++) {
      const now = await nextFrame();
      gaps.push(now - prev);
      prev = now;
    }
    stage.pause();
    const loop = stage.loopStats();
    const budget = stage.frameBudget();
    const pacing = stage.framePacing();
    const toys = stage.toyCount();
    stage.destroy();
    c.remove();
    out.stageLoop = {
      canvasCss: "416x216",
      familiars: 2,
      toys,
      rafFramesWaited: 360,
      ticks: loop.ticks,
      drawn: loop.drawn,
      skipEveryOther: budget.skipEveryOther,
      lastWindowAvgCostMs: budget.avgMs,
      pacingMissing: pacing.missing,
      pacingBaselineMs: pacing.baselineMs,
      pacingAvgGapMs: pacing.avgGapMs,
      rafGap: { ...stats(gaps), note: "headless Chromium 的 rAF 節奏，非使用者螢幕更新率" },
      note:
        "StageRenderer 自己的 requestAnimationFrame 迴圈。降級有兩條訊號：幀預算（renderFrame 的 JS 成本，不含 raster flush）與幀節奏（rAF 實際間隔 vs 這台螢幕的基準，抓得到合成／GPU／節流造成的掉幀，對抗審查 perf-claims-008）。skipEveryOther/pacingMissing 任一為 true 代表舞台自己降到 30fps",
    };
  }

  // 2c) Reduced Motion 的工作量（對抗審查 perf-claims-007）：畫面逐幀相同時，
  //     主迴圈只該畫第一幀＋每 500ms 一次世界維護，不是以螢幕更新率重畫同一張圖。
  {
    const c = makeCanvas(416, 216);
    const stage = new StageRenderer(c, "maid-classic", 1, { rng: () => 0.37 });
    stage.setMachineFlags({ ambient: true, frozen: false, quiet: false, playPerforming: false });
    stage.setAnimation("idle");
    stage.setFamiliars([{ id: "f1", name: "小白", palette: "maid-dusk" }]);
    stage.spawnToy("yarn");
    stage.setReducedMotion(true);
    const startedAt = performance.now();
    for (let i = 0; i < 360; i++) await nextFrame();
    const elapsedMs = performance.now() - startedAt;
    stage.pause();
    const loop = stage.loopStats();
    stage.destroy();
    c.remove();
    out.reducedMotionLoop = {
      canvasCss: "416x216",
      rafFramesWaited: 360,
      ticks: loop.ticks,
      drawn: loop.drawn,
      elapsedMs,
      drawnPerSecond: loop.drawn / Math.max(0.001, elapsedMs / 1000),
      note:
        "Reduced Motion 靜態短路：drawn 應該遠小於 ticks（第一幀＋每 500ms 一次維護），而不是每個 rAF 都重畫透明視窗",
    };
  }

  // 4) 長時間數值行為：時間軸跑 3 天（每 16.67ms 一步，取樣 20 萬點）。
  {
    const tl = new ExpressionTimeline(() => 0.5, 0);
    tl.setAnimation("idle", 0);
    let finite = true;
    let inRange = true;
    const step = 16.67;
    const threeDays = 3 * 24 * 3600 * 1000;
    const samplesN = 200_000;
    const stride = threeDays / samplesN;
    for (let i = 0; i < samplesN; i++) {
      const p = tl.paramsAt(i * stride + (i % 7) * step);
      const clamped = clampParams(p);
      for (const k of Object.keys(DEFAULT_PARAMS) as (keyof typeof DEFAULT_PARAMS)[]) {
        const v = p[k];
        if (typeof v === "number") {
          if (!Number.isFinite(v)) finite = false;
          if (v !== clamped[k]) inRange = false;
        }
      }
      if (!finite || !inRange) break;
    }
    out.longRun = { simulatedMs: threeDays, samples: samplesN, allFinite: finite, allWithinClamp: inRange };
  }

  out.finishedAt = new Date().toISOString();
  (window as unknown as { __perf: unknown }).__perf = out;
  document.title = "perf-ready";
}

run().catch((e) => {
  (window as unknown as { __perf: unknown }).__perf = { error: String(e && (e as Error).stack ? (e as Error).stack : e) };
  document.title = "perf-ready";
});
