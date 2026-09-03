// 第三輪對抗審查（0c845e0-20260903T185130Z）的回歸測試：遊玩場／Director／rig 渲染。
//
// 每一條都先在舊行為下紅燈，再由最小修復轉綠：
//   companion-gameplay-030 使魔身上的 pointerdown 被判成 none（死區）
//   companion-gameplay-031 / director-pipeline-019 quiet+Reduced Motion 逃出眨眼分支
//   companion-gameplay-033 stage 暫停後 Roll Call 仍報殘影活動
//   companion-gameplay-034 Reduced Motion 下光點／逗貓棒仍每幀追游標
//   companion-gameplay-035 playfield 死結構（restMs／carry／被丟棄的世界事件）
//   perf-claims-012        幀節奏基準線只會單調下修、永不回復
//   perf-claims-016        reportHitRect 每幀先建陣列才節流
//   director-pipeline-020  Utility Scoring 在生產路徑上是恆等式
//   rig-renderer-045       lie↔直立過場中頭與軀幹水平脫節
//   rig-renderer-046       startled-awake 沒有觸發路徑
//   rig-renderer-047       stand↔sit 裙擺／腿部單幀硬切
//   rig-renderer-049       組合式通道下胸前核心不呼吸

import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

import {
  StageRenderer,
  machineStageFlags,
  startleExpressionFor,
  statusChannelParams,
  worldEventExpression,
} from "../companion/rig/stage";
import { bodyAnchorX, drawRig, layoutFor, poseShiftX, skirtFlare } from "../companion/rig/draw";
import { ExpressionTimeline } from "../companion/rig/timeline";
import { RIG_PALETTES, clampParams } from "../companion/rig/params";
import {
  createWorld,
  spawnToy,
  stepWorld,
  type StepInputs,
  type World,
} from "../companion/playfield";
import {
  FRAME_WINDOW,
  framePacingPolicy,
  initialFramePacing,
} from "../companion/gameFeel";
import { InteractionDirector, type DirectorContext } from "../companion/director";
import { SHU_DIRECTOR_TABLES } from "../character/adapters/shuTables";
import { TRANSIENT_PRIORITY, transientCompetition, type TransientKind } from "../companion/machine";
import { seededRng } from "../companion/behavior";

const PAL = RIG_PALETTES["maid-classic"];

/** jsdom 沒有 canvas 2D：可鏈式 stub（stage 只呼叫繪圖指令，不讀像素）。 */
function stubCanvas(w = 416, h = 216): HTMLCanvasElement {
  const store: Record<string | symbol, unknown> = {};
  const ctx: unknown = new Proxy(store, {
    get(target, prop) {
      if (prop in target) return target[prop];
      return () => ctx;
    },
    set(target, prop, value) {
      target[prop] = value;
      return true;
    },
  });
  return {
    clientWidth: w,
    clientHeight: h,
    width: w,
    height: h,
    getContext: () => ctx,
    getBoundingClientRect: () => ({ left: 0, top: 0, width: w, height: h }),
  } as unknown as HTMLCanvasElement;
}

function makeStage(opts: { rng?: () => number } = {}) {
  const clock = { t: 1_000 };
  const stage = new StageRenderer(stubCanvas(), "maid-classic", 1, {
    autoStart: false,
    rng: opts.rng ?? (() => 0.9),
    now: () => clock.t,
  });
  stage.setAnimation("idle");
  stage.setMachineFlags(machineStageFlags("idle", null, "idle", true));
  const frames = (n: number, stepMs = 16) => {
    for (let i = 0; i < n; i++) {
      clock.t += stepMs;
      stage.renderFrame(clock.t);
    }
  };
  return { stage, clock, frames };
}

// ---------------------------------------------------------------------------
// companion-gameplay-030：使魔身上的點擊不是死區
// ---------------------------------------------------------------------------

describe("companion-gameplay-030：使魔身上的 pointerdown 不再被判成 none", () => {
  it("落在回報出去的使魔框內 → 不是 none（可拖視窗／開選單）", () => {
    const { stage, frames } = makeStage();
    stage.setToggles({ play: false, cursorPlay: false, deskMove: false, approach: false });
    stage.setFamiliars([{ id: "a", name: "小白", palette: "maid-classic" }]);
    frames(5, 16);
    const region = stage.interactiveRegions().find((g) => g.id === "familiar:a");
    expect(region).toBeDefined();
    const cx = region!.x + region!.w / 2;
    const cy = region!.y + region!.h / 2;
    // 角色框不可以蓋住這個點，否則測的就不是使魔了。
    const char = stage.charHitRect();
    expect(cx < char.x || cx > char.x + char.w).toBe(true);
    expect(stage.pointerDown(cx, cy)).not.toBe("none");
    stage.destroy();
  });
});

// ---------------------------------------------------------------------------
// companion-gameplay-031 / director-pipeline-019：quiet + Reduced Motion
// ---------------------------------------------------------------------------

describe("companion-gameplay-031／director-pipeline-019：安靜時段永遠走就地眨眼分支", () => {
  const ctx = (over: Partial<DirectorContext>): DirectorContext => ({
    nowMs: 0,
    ambient: true,
    quiet: false,
    reducedMotion: false,
    expressiveness: 1,
    msSinceInteraction: 300_000,
    behavior: {
      activation: 0,
      attention: 0.5,
      taskLoad: 0,
      interactionReadiness: 1,
      familiarity: 0.5,
      recentInterruptions: 0,
      currentFocus: null,
      lastInteractionAt: 0,
    },
    ...over,
  });

  it("quiet + Reduced Motion：任何動作都必須是 source=blink，永不落到一般 ambient", () => {
    const d = new InteractionDirector(undefined, SHU_DIRECTOR_TABLES);
    const rng = seededRng(7);
    const sources = new Set<string>();
    let count = 0;
    for (let i = 0; i < 4_000; i++) {
      const a = d.tick(ctx({ quiet: true, reducedMotion: true, nowMs: i * 500 }), rng);
      if (a) {
        sources.add(a.source);
        count += 1;
      }
    }
    expect(count).toBeGreaterThan(0);
    expect([...sources]).toEqual(["blink"]);
  });

  it("Reduced Motion（非安靜）下的 ambient 眨眼不會在第一次之後永久停止", () => {
    const d = new InteractionDirector(undefined, SHU_DIRECTOR_TABLES);
    const rng = seededRng(11);
    let count = 0;
    for (let i = 0; i < 4_000; i++) {
      if (d.tick(ctx({ reducedMotion: true, nowMs: i * 500 }), rng)) count += 1;
    }
    expect(count).toBeGreaterThan(1);
  });
});

// ---------------------------------------------------------------------------
// companion-gameplay-033：暫停後的 Roll Call
// ---------------------------------------------------------------------------

describe("companion-gameplay-033：stage 暫停後 Roll Call 不報殘影活動", () => {
  it("pause() 之後角色與使魔一律回「停下來了」", () => {
    const { stage, frames } = makeStage({ rng: () => 0 });
    stage.setFamiliars([{ id: "a", name: "小白", palette: "maid-classic" }]);
    expect(stage.spawnToy("yarn")).toBe(true);
    for (let i = 0; i < 40 && !stage.worldBusy(); i++) frames(1, 16);
    expect(stage.worldBusy()).toBe(true); // 真的在追／叼／散步
    const live = stage.rollCallNow(null);
    expect(live[0].activity).not.toBe("停下來了");
    stage.pause();
    const paused = stage.rollCallNow(null);
    expect(paused[0].activity).toBe("停下來了");
    expect(paused.slice(1).every((r) => r.activity === "停下來了" || r.activity === "在睡覺")).toBe(
      true
    );
    stage.destroy();
  });
});

// ---------------------------------------------------------------------------
// companion-gameplay-034：Reduced Motion 下的游標玩具
// ---------------------------------------------------------------------------

describe("companion-gameplay-034：Reduced Motion 下光點／逗貓棒不追游標", () => {
  const inputs = (over: Partial<StepInputs> = {}): StepInputs => ({
    nowMs: 1_000,
    dtMs: 16,
    ambient: true,
    frozen: false,
    quiet: false,
    reducedMotion: true,
    playEnabled: true,
    cursorPlayEnabled: true,
    deskMoveEnabled: true,
    pointer: null,
    ...over,
  });

  it("light：一步之後位置不變（不做指數插值）", () => {
    let w: World = createWorld(320, 170);
    w = spawnToy(w, "light", 1_000);
    const before = { ...w.toys[0] };
    const { world } = stepWorld(
      w,
      inputs({ pointer: { x: before.x + 100, y: before.y + 50, active: true } }),
      () => 0.5
    );
    expect(world.toys[0].x).toBe(before.x);
  });

  it("wand：游標不在場內時也不會平滑垂下", () => {
    let w: World = createWorld(320, 170);
    w = spawnToy(w, "wand", 1_000);
    w = { ...w, toys: [{ ...w.toys[0], y: 40 }] };
    const before = { ...w.toys[0] };
    const { world } = stepWorld(w, inputs({ pointer: null }), () => 0.5);
    expect(world.toys[0].y).not.toBeCloseTo(before.y + (w.ground - 24 - before.y) * 16 * 0.006, 3);
    expect(world.toys[0].vx).toBe(0);
  });
});

// ---------------------------------------------------------------------------
// companion-gameplay-035：playfield 死結構
// ---------------------------------------------------------------------------

describe("companion-gameplay-035：playfield 沒有只有資料、沒有行為的結構", () => {
  it("restMs 這個沒有讀取端的欄位不再存在", () => {
    const w = spawnToy(createWorld(320, 170), "yarn", 0);
    expect(Object.keys(w.toys[0])).not.toContain("restMs");
  });

  it("carry 模式真的到得了（叼起來的那一拍）", () => {
    let w: World = createWorld(320, 170);
    w = spawnToy(w, "yarn", 1_000_000);
    w = { ...w, toys: [{ ...w.toys[0], x: 260, y: w.ground - 6, interest: 1 }] };
    const modes = new Set<string>();
    for (let i = 0; i < 900; i++) {
      const r = stepWorld(
        w,
        {
          nowMs: 1_000_000 + i * 16,
          dtMs: 16,
          ambient: true,
          frozen: false,
          quiet: false,
          reducedMotion: false,
          playEnabled: true,
          cursorPlayEnabled: true,
          deskMoveEnabled: true,
          pointer: null,
        },
        () => 0
      );
      w = r.world;
      modes.add(w.char.mode);
    }
    expect(modes.has("carry")).toBe(true);
  });

  it("叼回來／拒絕歸還／尾巴推一下都有對應的演出（不是被丟棄的通道）", () => {
    expect(worldEventExpression({ type: "toy-returned", id: 1 })).not.toBeNull();
    expect(worldEventExpression({ type: "toy-refused", id: 1 })).not.toBeNull();
    expect(worldEventExpression({ type: "toy-pushed", id: 1 })).not.toBeNull();
    expect(worldEventExpression({ type: "expression", id: "curious", durationMs: 100 })).toEqual({
      id: "curious",
      durationMs: 100,
    });
  });
});

// ---------------------------------------------------------------------------
// perf-claims-012：幀節奏基準線
// ---------------------------------------------------------------------------

describe("perf-claims-012：單一短樣本不得造成永久降級", () => {
  const feed = (state: ReturnType<typeof initialFramePacing>, gapMs: number, n: number) => {
    let s = state;
    for (let i = 0; i < n; i++) s = framePacingPolicy(s, gapMs);
    return s;
  };

  it("59×16.67ms + 1×9ms 之後，乾淨的 60Hz 窗要能讓基準線回復", () => {
    let p = feed(initialFramePacing(), 1000 / 60, FRAME_WINDOW - 1);
    p = framePacingPolicy(p, 9); // 一次 rAF 抖動
    // 之後全部是乾淨的 60Hz。
    for (let win = 0; win < 4; win++) p = feed(p, 1000 / 60, FRAME_WINDOW);
    expect(p.baselineMs).toBeCloseTo(16.67, 1);
    expect(p.missing).toBe(false);
  });

  it("真的變慢（穩定 30fps）在基準線還記得 60Hz 時仍然降級", () => {
    let p = feed(initialFramePacing(), 1000 / 60, FRAME_WINDOW);
    expect(p.baselineMs).toBeCloseTo(16.67, 1);
    p = feed(p, 1000 / 30, FRAME_WINDOW);
    expect(p.missing).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// perf-claims-016：先判節流再建陣列
// ---------------------------------------------------------------------------

describe("perf-claims-016：reportHitRect 節流之前不建互動框", () => {
  it("50ms 內連續 5 次只算一次互動框與 regions", () => {
    const { stage, clock } = makeStage();
    let bounds = 0;
    let regions = 0;
    const realBounds = stage.interactiveBounds.bind(stage);
    const realRegions = stage.interactiveRegions.bind(stage);
    stage.interactiveBounds = () => {
      bounds += 1;
      return realBounds();
    };
    stage.interactiveRegions = () => {
      regions += 1;
      return realRegions();
    };
    stage.onHitRect(() => {});
    stage.onHitRegions(() => {});
    for (let i = 0; i < 5; i++) {
      stage.reportHitRect();
      clock.t += 10; // 遠小於 50ms 的節流窗
    }
    expect(bounds).toBe(1);
    expect(regions).toBe(1);
    stage.destroy();
  });
});

// ---------------------------------------------------------------------------
// director-pipeline-020：Utility Scoring 不留假裝接上的通道
// ---------------------------------------------------------------------------

describe("director-pipeline-020：同優先平手的判定誠實", () => {
  it("machine.ts 不再拿寫死常數呼叫 scoreEvent", () => {
    const src = fs.readFileSync(path.resolve("src/companion/machine.ts"), "utf8");
    expect(src).not.toMatch(/scoreEvent\(/); // 只剩註解說明，沒有呼叫
    expect(src).not.toMatch(/from "\.\/behavior"/);
  });

  it("同優先、非重複 → 永遠 replace（確定性，不是被評分翻轉）", () => {
    const kinds = Object.keys(TRANSIENT_PRIORITY) as TransientKind[];
    const results = new Set<string>();
    for (const a of kinds) {
      for (const b of kinds) {
        if (TRANSIENT_PRIORITY[a] !== TRANSIENT_PRIORITY[b]) continue;
        for (const verified of [undefined, true, false]) {
          for (const animation of [undefined, "x", "y"]) {
            const active = { kind: a, untilMs: 10_000, verified, animation };
            const next = { kind: b, verified: true, animation: "z" };
            const repeat =
              active.kind === next.kind &&
              active.verified === next.verified &&
              active.animation === next.animation;
            if (repeat) continue;
            results.add(transientCompetition(active, next));
          }
        }
      }
    }
    expect([...results]).toEqual(["replace"]);
  });
});

// ---------------------------------------------------------------------------
// rig-renderer-045：lie↔直立過場的水平錨點
// ---------------------------------------------------------------------------

describe("rig-renderer-045：過場中頭與軀幹不脫節", () => {
  it("layoutFor 的頭中心永遠貼著目標姿勢的軀幹錨點", () => {
    for (const [pose, from] of [
      ["lie", "stand"],
      ["lie", "crouch"],
      ["stand", "lie"],
      ["sit", "lie"],
    ] as const) {
      for (let b = 0; b <= 1.0001; b += 0.05) {
        const p = clampParams({ pose, poseFrom: from, poseBlend: Math.min(1, b) });
        const L = layoutFor(p, PAL);
        expect(Math.abs(L.hx - bodyAnchorX(pose))).toBeLessThanOrEqual(1);
      }
    }
  });

  it("整個角色的水平位移連續帶過（不是頭自己漂）", () => {
    const shiftAt = (b: number) =>
      poseShiftX(clampParams({ pose: "lie", poseFrom: "stand", poseBlend: b }));
    expect(shiftAt(1)).toBeCloseTo(0, 6);
    expect(shiftAt(0)).toBeCloseTo(bodyAnchorX("stand") - bodyAnchorX("lie"), 6);
    let prev = shiftAt(0);
    let maxJump = 0;
    for (let b = 0.02; b <= 1.0001; b += 0.02) {
      const s = shiftAt(Math.min(1, b));
      maxJump = Math.max(maxJump, Math.abs(s - prev));
      prev = s;
    }
    expect(maxJump).toBeLessThan(1);
  });

  it("真實時間軸：lie-flat → success-verified 過場中頭-軀幹水平偏移永遠 0", () => {
    const tl = new ExpressionTimeline(() => 0.5, 0);
    tl.setAnimation("lie-flat", 0);
    tl.paramsAt(3_000);
    tl.setAnimation("success-verified", 3_000);
    let worst = 0;
    for (let t = 3_000; t <= 4_200; t += 16.7) {
      const p = tl.paramsAt(t);
      const L = layoutFor(p, PAL);
      worst = Math.max(worst, Math.abs(L.hx - bodyAnchorX(p.pose)));
    }
    expect(worst).toBeLessThanOrEqual(1);
  });
});

// ---------------------------------------------------------------------------
// rig-renderer-046：startled-awake 有觸發路徑
// ---------------------------------------------------------------------------

describe("rig-renderer-046：睡著時被戳會驚醒", () => {
  it("純函式：休息姿勢是睡眠類 + 戳 → startled-awake", () => {
    expect(startleExpressionFor("sleep", "clicked")).toBe("startled-awake");
    expect(startleExpressionFor("doze", "poked")).toBe("startled-awake");
    expect(startleExpressionFor("lie-flat", "poked-rapid")).toBe("startled-awake");
    // 沒在睡、或不是互動 → 不改寫。
    expect(startleExpressionFor("sit", "clicked")).toBeNull();
    expect(startleExpressionFor("sleep", "blocked")).toBeNull();
    expect(startleExpressionFor(null, "clicked")).toBeNull();
  });

  it("stage：睡著中收到 clicked → 真的播 startled-awake", () => {
    const { stage, frames } = makeStage();
    stage.setToggles({ play: false, cursorPlay: false, deskMove: false, approach: false });
    stage.setAnimation("sleep");
    frames(3, 16);
    expect(stage.restingExpression()).toBe("sleep");
    stage.setAnimation("clicked");
    frames(1, 16);
    expect(stage.currentAnimation()).toBe("startled-awake");
    // host 每 500ms 會再送一次同樣的 clicked：驚醒不能只演半秒就被蓋掉。
    stage.setAnimation("clicked");
    frames(1, 16);
    expect(stage.currentAnimation()).toBe("startled-awake");
    // 換成別的動畫就結束改寫（她已經醒了）。
    stage.setAnimation("idle");
    frames(1, 16);
    expect(stage.currentAnimation()).toBe("idle");
    stage.destroy();
  });
});

// ---------------------------------------------------------------------------
// rig-renderer-047：stand↔sit 的裙擺與腿部
// ---------------------------------------------------------------------------

describe("rig-renderer-047：stand↔sit 的裙擺寬度連續", () => {
  const sweep = (from: string, to: string) => {
    const tl = new ExpressionTimeline(() => 0.5, 0);
    tl.setAnimation(from, 0);
    tl.paramsAt(3_000);
    tl.setAnimation(to, 3_000);
    let prevNaive = 0;
    let prev = 0;
    let naiveJump = 0;
    let jump = 0;
    for (let t = 3_000; t <= 4_400; t += 16.7) {
      const p = tl.paramsAt(t);
      const naive = p.pose === "sit" ? 27 : 23;
      const flare = skirtFlare(p);
      if (t > 3_000) {
        naiveJump = Math.max(naiveJump, Math.abs(naive - prevNaive));
        jump = Math.max(jump, Math.abs(flare - prev));
      }
      prevNaive = naive;
      prev = flare;
    }
    return { naiveJump, jump };
  };

  it("sit → idle：舊的字串硬切跳 4px，混合後每幀 ≤ 1px", () => {
    const { naiveJump, jump } = sweep("sit", "idle");
    expect(naiveJump).toBeCloseTo(4, 5);
    expect(jump).toBeLessThanOrEqual(1);
  });

  it("idle → doze（坐姿）：同樣連續", () => {
    const { naiveJump, jump } = sweep("idle", "doze");
    expect(naiveJump).toBeCloseTo(4, 5);
    expect(jump).toBeLessThanOrEqual(1);
  });

  it("stand↔sit 過場中腿部輪廓不換形狀，逐幀座標跳動 < 4px", () => {
    // 錄下 drawRig 對 canvas 下的每一道指令：舊實作在 pose 字串翻面那一幀把
    // 「小腿梯形 lineTo」整組換成「橢圓 ellipse」（指令序列改變＝單幀硬切）。
    const recording = () => {
      const ops: string[] = [];
      const nums: number[] = [];
      const grad = { addColorStop: () => {} };
      const target: Record<string | symbol, unknown> = {};
      const ctx = new Proxy(target, {
        get(t, prop) {
          if (prop in t) return t[prop];
          return (...args: unknown[]) => {
            ops.push(String(prop));
            for (const a of args) if (typeof a === "number") nums.push(a);
            if (prop === "createLinearGradient" || prop === "createRadialGradient") return grad;
            return undefined;
          };
        },
        set(t, prop, value) {
          t[prop] = value;
          return true;
        },
      }) as unknown as CanvasRenderingContext2D;
      return { ctx, ops, nums };
    };
    const tl = new ExpressionTimeline(() => 0.5, 0);
    tl.setAnimation("sit", 0);
    tl.paramsAt(3_000);
    tl.setAnimation("idle", 3_000);
    let prevOps: string[] | null = null;
    let prevNums: number[] | null = null;
    let shapeChanges = 0;
    let maxJump = 0;
    for (let t = 3_000; t <= 4_600; t += 16.7) {
      const p = tl.paramsAt(t);
      const r = recording();
      drawRig(r.ctx, p, PAL);
      if (prevOps) {
        if (prevOps.join(",") !== r.ops.join(",") || prevNums!.length !== r.nums.length) {
          shapeChanges += 1;
        } else {
          for (let i = 0; i < r.nums.length; i++) {
            maxJump = Math.max(maxJump, Math.abs(r.nums[i] - prevNums![i]));
          }
        }
      }
      prevOps = r.ops;
      prevNums = r.nums;
    }
    expect(shapeChanges).toBe(0); // 舊實作：1（腿型整組替換的那一幀）
    expect(maxJump).toBeLessThan(4);
  });

  it("純姿勢的裙擺半寬與原本一致（端點不變）", () => {
    expect(skirtFlare(clampParams({ pose: "sit" }))).toBeCloseTo(27, 6);
    expect(skirtFlare(clampParams({ pose: "stand" }))).toBeCloseTo(23, 6);
    expect(skirtFlare(clampParams({ pose: "crouch" }))).toBeCloseTo(23, 6);
  });
});

// ---------------------------------------------------------------------------
// rig-renderer-049：組合式通道下核心會呼吸
// ---------------------------------------------------------------------------

describe("rig-renderer-049：趴著＋核心顯示工作中時，核心真的在呼吸", () => {
  it("statusChannelParams(working, now) 的 corePulse 會隨時間變化", () => {
    const values = new Set<number>();
    for (let t = 0; t < 2_600; t += 130) {
      const ch = statusChannelParams("working", t);
      expect(ch).not.toBeNull();
      values.add(ch!.corePulse as number);
    }
    expect(values.size).toBeGreaterThan(1);
    expect([...values].every((v) => typeof v === "number" && v >= 0 && v <= 1)).toBe(true);
  });

  it("不給時間戳時仍是原本的靜態通道（Reduced Motion 用）", () => {
    const ch = statusChannelParams("working");
    expect(ch).not.toBeNull();
    expect(ch!.coreGlow).toBe(1);
    expect(ch!.corePulse).toBeUndefined();
  });
});
