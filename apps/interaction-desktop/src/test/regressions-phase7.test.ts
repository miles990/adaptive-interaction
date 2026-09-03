// v0.5 Phase 7 對抗審查（第二輪：每項都由獨立懷疑者對原始碼確認）的 regression。
//
// 1  hit-rect 每幀節流回報（Rust 不再用過期的框判定點擊穿透）
// 2  舞台旗標與 machine 同步生效；步行姿勢不覆蓋真相/表演姿勢
// 3  拖曳期間的持續 transient（不會在半空中回 idle）
// 4  多角色互相注意（主角回看、greet 取最近、被追者有反應）
// 5  Director 的 quiet 分支可達（安靜時仍眨眼）
// 6  「不要主動說話」同時關掉本機的隨口氣泡與 ambient
// 7  已過期的表演不算被搶佔
// 8  estop/cancel 停止語音、清掉活著的非安全 transient
// 9  剛被互動就不睡回去
// 10 Reduced Motion 真的靜態（sway 常數、粒子歸零、狀態符號保留）
// 11 `ask` 是真相狀態；look-at-confirmation 映射到非真相的 question
// 12 clampParams 不做 Number() 強制轉型
// 13 lerpParams 允許 ease 的回彈外推
// 15 lie↔stand 姿勢過場不再單幀瞬移

import fs from "node:fs";
import path from "node:path";
import { describe, expect, it, vi } from "vitest";
import {
  blendPose,
  clampParams,
  DEFAULT_PARAMS,
  lerpParams,
  LERP_T_MAX,
  RIG_PALETTES,
} from "../companion/rig/params";
import { layoutFor } from "../companion/rig/draw";
import { easeOutBackLite, ExpressionTimeline } from "../companion/rig/timeline";
import { resolveExpression } from "../companion/rig/expressions";
import {
  gazeBiasParams,
  hitRectReportPolicy,
  HIT_RECT_MAX_QUIET_MS,
  HIT_RECT_MIN_INTERVAL_MS,
  machineStageFlags,
  pointerGazeDir,
  StageRenderer,
  swayAt,
} from "../companion/rig/stage";
import {
  DRAG_HOLD_MS,
  DRAG_RENEW_MS,
  MachineState,
  pose,
  reduce,
  wasPreempted,
} from "../companion/machine";
import {
  directorTickGate,
  InteractionDirector,
  SLEEP_RESUME_BLOCK_MS,
} from "../companion/director";
import { initialBehavior } from "../companion/behavior";
import {
  hoverBubblePolicy,
  proactiveQuietActive,
  proactiveQuietUntil,
} from "../companion/attention";
import { DEFAULT_TUNING, personalityFor } from "../companion/personality";
// CPP：Director 表屬於角色（shu adapter tables）。
import { SHU_DIRECTOR_TABLES } from "../character/adapters/shuTables";
import { planPresentationCommand } from "../companion/presentationCommands";
import {
  createWorld,
  Familiar,
  nearestFamiliar,
  StepInputs,
  stepWorld,
  World,
} from "../companion/playfield";
import { stopSpeech } from "../companion/CompanionApp";

// ---------------------------------------------------------------------------
// 測試工具
// ---------------------------------------------------------------------------

/** jsdom 沒有 canvas 2D：用可鏈式的 stub（stage 只呼叫繪圖指令，不讀像素）。 */
function stubCanvas(w = 416, h = 216): HTMLCanvasElement {
  const store: Record<string | symbol, unknown> = {};
  const ctx: unknown = new Proxy(store, {
    get(target, prop) {
      if (prop in target) return target[prop];
      return () => ctx; // 任何方法都回自己（createRadialGradient().addColorStop 可鏈）
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

/** 會記下每一個繪圖指令的 canvas：用來驗「兩幀畫出來一模一樣」。 */
function recordingCanvas(w = 416, h = 216): {
  canvas: HTMLCanvasElement;
  take: () => string[];
} {
  let log: string[] = [];
  const store: Record<string | symbol, unknown> = {};
  const fmt = (v: unknown) => (typeof v === "number" ? v.toFixed(4) : String(v));
  const ctx: unknown = new Proxy(store, {
    get(target, prop) {
      if (prop in target) return target[prop];
      return (...args: unknown[]) => {
        log.push(`${String(prop)}(${args.map(fmt).join(",")})`);
        return ctx;
      };
    },
    set(target, prop, value) {
      log.push(`${String(prop)}=${fmt(value)}`);
      target[prop] = value;
      return true;
    },
  });
  const canvas = {
    clientWidth: w,
    clientHeight: h,
    width: w,
    height: h,
    getContext: () => ctx,
    getBoundingClientRect: () => ({ left: 0, top: 0, width: w, height: h }),
  } as unknown as HTMLCanvasElement;
  return {
    canvas,
    take: () => {
      const out = log;
      log = [];
      return out;
    },
  };
}

/** 腳本化 rng：用完清單後固定回 `tail`。 */
function scriptedRng(values: number[], tail = 0.99): () => number {
  let i = 0;
  return () => (i < values.length ? values[i++] : tail);
}

const PAL = RIG_PALETTES["maid-classic"];

function familiar(id: string, x: number, over: Partial<Familiar> = {}): Familiar {
  return {
    id,
    name: id,
    palette: "maid-classic",
    x,
    vx: 0,
    facing: 1,
    state: "idle",
    stateUntil: 0,
    greetWith: null,
    ...over,
  };
}

function stepInputs(over: Partial<StepInputs> = {}): StepInputs {
  return {
    nowMs: 10_000,
    dtMs: 16,
    ambient: true,
    frozen: false,
    quiet: false,
    reducedMotion: false,
    playEnabled: true,
    cursorPlayEnabled: true,
    deskMoveEnabled: true,
    pointer: null,
    ...over,
  };
}

const T0 = 1_000_000;

// ---------------------------------------------------------------------------
// 1. hit-rect 節流回報
// ---------------------------------------------------------------------------

describe("#1 hit-rect 回報政策（點擊穿透用的框不能是半秒前的）", () => {
  const r = (x: number) => ({ x, y: 10, w: 52, h: 124 });

  it("第一次一定回報", () => {
    expect(hitRectReportPolicy(null, r(0), 0)).toBe(true);
    expect(hitRectReportPolicy(null, r(0), Number.POSITIVE_INFINITY)).toBe(true);
  });

  it("節流：50ms 內不回報（不得每幀 invoke）", () => {
    expect(hitRectReportPolicy(r(0), r(400), HIT_RECT_MIN_INTERVAL_MS - 1)).toBe(false);
    expect(hitRectReportPolicy(r(0), r(400), 16)).toBe(false);
  });

  it("位移 >4px 且過了節流窗就立刻回報", () => {
    expect(hitRectReportPolicy(r(0), r(4.5), HIT_RECT_MIN_INTERVAL_MS)).toBe(true);
    expect(hitRectReportPolicy(r(0), r(3), HIT_RECT_MIN_INTERVAL_MS)).toBe(false);
  });

  it("沒有位移也要在 60ms 內補一次（Rust 端的框永遠不會太舊）", () => {
    expect(hitRectReportPolicy(r(0), r(0), HIT_RECT_MAX_QUIET_MS)).toBe(true);
    expect(hitRectReportPolicy(r(0), r(0), HIT_RECT_MAX_QUIET_MS - 1)).toBe(false);
  });

  it("StageRenderer：角色走動時每幾幀回報一次——不是每幀，也不是每 500ms", () => {
    let t = 1_000;
    const stage = new StageRenderer(stubCanvas(), "maid-classic", 1, {
      autoStart: false,
      rng: () => 0.001, // 觸發散步：角色會一直移動
      now: () => t,
    });
    const reports: { x: number }[] = [];
    stage.onHitRect((rect) => reports.push({ x: rect.x }));
    stage.setAnimation("idle");
    stage.setMachineFlags(machineStageFlags("idle", null, "idle", true));
    for (let i = 0; i < 60; i++) {
      t += 16.7; // ~1 秒
      stage.renderFrame(t);
    }
    stage.destroy();
    expect(reports.length).toBeLessThan(60); // 有界：沒有每幀 invoke
    expect(reports.length).toBeGreaterThanOrEqual(10); // 遠比 500ms pump 的 2 次密
  });

  it("StageRenderer：force 心跳一定回報（rAF 停擺時仍有一次）", () => {
    let t = 5_000;
    const stage = new StageRenderer(stubCanvas(), "maid-classic", 1, {
      autoStart: false,
      rng: () => 0.9,
      now: () => t,
    });
    const reports: unknown[] = [];
    stage.onHitRect((rect) => reports.push(rect));
    stage.reportHitRect(true);
    stage.reportHitRect(true); // 節流窗內，但 force 仍然回報
    stage.destroy();
    expect(reports).toHaveLength(2);
  });
});

// ---------------------------------------------------------------------------
// 2. 舞台旗標即時生效 + 步行姿勢不覆蓋
// ---------------------------------------------------------------------------

describe("#2 machine 真相狀態要立刻凍結舞台，步行不得覆蓋姿勢", () => {
  it("machineStageFlags：emergency/offline/paused 凍結、quiet 標記、ambient 只在遊玩允許時", () => {
    expect(machineStageFlags("emergency", null, "emergency", false)).toEqual({
      ambient: false,
      frozen: true,
      quiet: false,
      playPerforming: false,
    });
    expect(machineStageFlags("quiet", null, "quiet", false).quiet).toBe(true);
    expect(machineStageFlags("idle", null, "idle", true).ambient).toBe(true);
    // 工作/等待只借通道：遊玩場繼續運轉；安全與結果狀態整體停。
    expect(machineStageFlags("idle", { kind: "acting" }, "act", false).ambient).toBe(true);
    expect(machineStageFlags("idle", { kind: "routing" }, "routing", false).ambient).toBe(true);
    expect(machineStageFlags("idle", { kind: "succeeded" }, "success", false).ambient).toBe(false);
    expect(machineStageFlags("idle", { kind: "blocked" }, "blocked", false).ambient).toBe(false);
    expect(
      machineStageFlags("idle", { kind: "performing", animation: "hold-ball" }, "hold-ball", false)
    ).toMatchObject({ ambient: true, playPerforming: true });
  });

  it("CompanionApp 的 syncPose 會同步 stage 旗標（不是等 500ms pump）", () => {
    const src = fs.readFileSync(path.resolve("src/companion/CompanionApp.tsx"), "utf8");
    const syncPose = src.slice(src.indexOf("const syncPose = React.useCallback"));
    const body = syncPose.slice(0, syncPose.indexOf("}, []);"));
    expect(body).toContain("setMachineFlags");
    expect(body).toContain("machineStageFlags");
  });

  it("進入 emergency 的下一幀：世界凍結（角色不再前進）", () => {
    let t = 1_000;
    const stage = new StageRenderer(stubCanvas(), "maid-classic", 1, {
      autoStart: false,
      rng: () => 0.001,
      now: () => t,
    });
    stage.setAnimation("idle");
    stage.setMachineFlags(machineStageFlags("idle", null, "idle", true));
    for (let i = 0; i < 20; i++) {
      t += 100;
      stage.renderFrame(t);
    }
    const moving = stage.charHitRect().x;
    t += 100;
    stage.renderFrame(t);
    expect(stage.charHitRect().x).not.toBe(moving); // 先確認她真的在走

    stage.setAnimation("emergency");
    stage.setMachineFlags(machineStageFlags("emergency", null, "emergency", false));
    const frozenAt = stage.charHitRect().x;
    t += 100;
    stage.renderFrame(t);
    expect(stage.charHitRect().x).toBe(frozenAt);
    stage.destroy();
  });

  it("步行 secondary motion 不得把 lie-flat 的姿勢改成 stand（旗標還沒更新時也不行）", () => {
    let t = 1_000;
    const stage = new StageRenderer(stubCanvas(), "maid-classic", 1, {
      autoStart: false,
      rng: () => 0.001,
      now: () => t,
    });
    stage.setAnimation("idle");
    stage.setMachineFlags(machineStageFlags("idle", null, "idle", true));
    for (let i = 0; i < 20; i++) {
      t += 100;
      stage.renderFrame(t);
    }
    // 表演開始，但舞台旗標故意維持「還在遊玩」（模擬未同步的那半秒）。
    stage.setAnimation("lie-flat");
    for (let i = 0; i < 30; i++) {
      t += 33;
      stage.renderFrame(t);
    }
    expect(stage.lastFrameParams()?.pose).toBe("lie");
    stage.destroy();
  });
});

// ---------------------------------------------------------------------------
// 3. 拖曳期間的持續 transient
// ---------------------------------------------------------------------------

describe("#3 被抱起來的期間不能自己過期", () => {
  it("續期間隔短於 TTL：拖 3 秒仍然是 dragged", () => {
    expect(DRAG_RENEW_MS).toBeLessThan(DRAG_HOLD_MS);
    let s: MachineState = { base: "idle", transient: null };
    s = reduce(s, { type: "transient", kind: "dragged", durationMs: DRAG_HOLD_MS }, T0);
    for (let t = T0 + DRAG_RENEW_MS; t <= T0 + 3_000; t += DRAG_RENEW_MS) {
      s = reduce(s, { type: "transient", kind: "dragged", durationMs: DRAG_HOLD_MS }, t);
    }
    expect(pose(s, T0 + 3_000).animation).toBe("dragged");
  });

  it("沒有續期的話 1.5 秒後就掉回 idle（這正是被修掉的行為）", () => {
    let s: MachineState = { base: "idle", transient: null };
    s = reduce(s, { type: "transient", kind: "dragged", durationMs: DRAG_HOLD_MS }, T0);
    expect(pose(s, T0 + 1_600).animation).toBe("idle");
  });

  it("放下時清掉 dragged，落地演出才播得出來", () => {
    let s: MachineState = { base: "idle", transient: null };
    s = reduce(s, { type: "transient", kind: "dragged", durationMs: DRAG_HOLD_MS }, T0);
    // 沒清掉的話 dragged(55) 會壓過 performing(25)。
    const kept = reduce(
      s,
      { type: "transient", kind: "performing", animation: "wobbly-landing", durationMs: 1600 },
      T0 + 100
    );
    expect(pose(kept, T0 + 200).animation).toBe("dragged");
    const cleared = reduce(
      reduce(s, { type: "clear-transient" }, T0 + 100),
      { type: "transient", kind: "performing", animation: "wobbly-landing", durationMs: 1600 },
      T0 + 100
    );
    expect(pose(cleared, T0 + 200).animation).toBe("wobbly-landing");
  });

  it("CompanionApp 在拖曳開始/結束時管理續期，不是丟一個 1500ms 就走", () => {
    const src = fs.readFileSync(path.resolve("src/companion/CompanionApp.tsx"), "utf8");
    expect(src).toContain("beginDragHold()");
    expect(src).toContain("endDragHold()");
    expect(src).not.toContain('kind: "dragged", durationMs: 1500');
  });
});

// ---------------------------------------------------------------------------
// 4. 多角色互相注意
// ---------------------------------------------------------------------------

describe("#4 多角色互相注意（不只是使魔↔使魔）", () => {
  function worldWith(familiars: Familiar[], charX = 160): World {
    const w = createWorld(320, 176);
    return { ...w, char: { ...w.char, x: charX }, familiars };
  }

  it("greet 目標取最近的一隻，不是清單第一隻", () => {
    const w = worldWith([familiar("f1", 100), familiar("f2", 300), familiar("f3", 110)]);
    // roll=0.65 → greet 分支；0.1 <0.6 → 選使魔（最近的）。
    // deskMove 關掉：否則 stepChar 的散步抽樣會先吃掉一個 rng。
    const { world } = stepWorld(w, stepInputs({ deskMoveEnabled: false }), scriptedRng([0.65, 0.1]));
    expect(world.familiars[0].greetWith).toBe("f3");
  });

  it("使魔向主角打招呼 → 主角回看，並可能回一顆愛心", () => {
    const w = worldWith([familiar("f1", 40)]);
    // 只有一隻：nearest 為 null → 目標必為主角；0.2 <0.35 → 回愛心。
    const { world, events } = stepWorld(
      w,
      stepInputs({ deskMoveEnabled: false }),
      scriptedRng([0.65, 0.2])
    );
    expect(world.char.attendTo).toBe("f1");
    expect(world.char.attendUntil).toBeGreaterThan(10_000);
    expect(world.char.greetBackUntil).toBeGreaterThan(10_000);
    expect(events).toContainEqual({ type: "greeted-by", id: "f1" });
  });

  it("回看期間主角真的轉向那一側（且到期會收回）", () => {
    const w = worldWith([familiar("f1", 40)], 200);
    const attending: World = {
      ...w,
      char: { ...w.char, attendTo: "f1", attendUntil: 12_000, facing: 1 },
    };
    const { world } = stepWorld(attending, stepInputs({ nowMs: 10_000 }), () => 0.99);
    expect(world.char.facing).toBe(-1);

    const { world: expired } = stepWorld(attending, stepInputs({ nowMs: 13_000 }), () => 0.99);
    expect(expired.char.attendTo).toBeNull();
  });

  it("被追的使魔會有反應：逃跑或回頭，不是毫無反應地站著", () => {
    const w = worldWith([familiar("f1", 100), familiar("f2", 150)]);
    // roll=0.75 → chase；0.5 → stateUntil；0.1 <0.7 → 逃跑。
    const fled = stepWorld(w, stepInputs({ deskMoveEnabled: false }), scriptedRng([0.75, 0.5, 0.1]));
    const chased = fled.world.familiars.find((f) => f.id === "f2")!;
    expect(chased.state).toBe("walk");
    expect(chased.vx).toBeGreaterThan(0); // f2 在 f1 右邊 → 往右逃
    expect(fled.events).toContainEqual({ type: "familiar-fled", id: "f2", by: "f1" });

    // 0.9 >0.7 → 停下來回頭看。
    const looked = stepWorld(
      w,
      stepInputs({ deskMoveEnabled: false }),
      scriptedRng([0.75, 0.5, 0.9])
    );
    const back = looked.world.familiars.find((f) => f.id === "f2")!;
    expect(back.state).toBe("idle");
    expect(back.facing).toBe(-1); // 回頭看左邊的 f1
    expect(looked.events).toContainEqual({ type: "familiar-looked-back", id: "f2", by: "f1" });
  });

  it("nearestFamiliar：空清單回 null、否則回距離最近的", () => {
    expect(nearestFamiliar([], 0)).toBeNull();
    expect(nearestFamiliar([{ x: 10 }, { x: -3 }, { x: 40 }], 0)).toEqual({ x: -3 });
  });

  it("回看只動視線/耳朵，不動姿勢，也不是真相狀態", () => {
    const before = clampParams({});
    const after = gazeBiasParams(before, 1);
    expect(after.pose).toBe(before.pose);
    expect(after.pupilX).toBeGreaterThan(before.pupilX);
    expect(after.earPerk).toBeGreaterThan(before.earPerk);
    expect(gazeBiasParams(before, 0)).toEqual(before);
    expect(pointerGazeDir(null, 100)).toBe(0);
    expect(pointerGazeDir({ x: 210, y: 0 }, 100)).toBeCloseTo(1);
    expect(pointerGazeDir({ x: 45, y: 0 }, 100)).toBeCloseTo(-0.5);
  });
});

// ---------------------------------------------------------------------------
// 5. Director quiet 分支可達
// ---------------------------------------------------------------------------

describe("#5 安靜時 Director 仍然 tick（只剩眨眼）", () => {
  it("directorTickGate：quiet 基態要 tick，並標記 quiet", () => {
    expect(
      directorTickGate({ poseAmbient: false, base: "quiet", hasActiveTransient: false })
    ).toEqual({ tick: true, quiet: true });
    expect(
      directorTickGate({ poseAmbient: true, base: "idle", hasActiveTransient: false })
    ).toEqual({ tick: true, quiet: false });
    expect(
      directorTickGate({ poseAmbient: true, base: "idle", hasActiveTransient: true }).tick
    ).toBe(false);
    expect(
      directorTickGate({ poseAmbient: false, base: "emergency", hasActiveTransient: false }).tick
    ).toBe(false);
    // 使用者要求安靜：仍 tick，但只允許眨眼。
    expect(
      directorTickGate({
        poseAmbient: true,
        base: "idle",
        hasActiveTransient: false,
        localQuiet: true,
      })
    ).toEqual({ tick: true, quiet: true });
  });

  it("quiet 的 tick 只產出眨眼類，不會冒出伸懶腰/趴平", () => {
    const d = new InteractionDirector(DEFAULT_TUNING, SHU_DIRECTOR_TABLES);
    const seen = new Set<string>();
    for (let i = 0; i < 400; i++) {
      const a = d.tick(
        {
          nowMs: 1_000_000 + i * 500,
          ambient: true,
          quiet: true,
          reducedMotion: false,
          expressiveness: 1.5,
          msSinceInteraction: 600_000,
          behavior: { ...initialBehavior(0), activation: 0, taskLoad: 0 },
        },
        () => (i % 37) / 37
      );
      if (a) seen.add(a.expression);
    }
    expect(seen.size).toBeGreaterThan(0); // 分支真的可達
    expect([...seen]).toEqual(["blink"]);
  });

  it("安靜時的眨眼是「就地眨」：不換表情、不把安靜姿勢彈成中性站姿", () => {
    const tl = new ExpressionTimeline(() => 0.99, 0);
    tl.setAnimation("quiet", 0);
    const before = tl.paramsAt(1_000);
    expect(before.pose).toBe("sit"); // 安靜陪伴的坐姿
    expect(tl.blinkNow(1_000)).toBe(true);
    const during = tl.paramsAt(1_075);
    expect(during.eyeOpen).toBeLessThan(before.eyeOpen * 0.5);
    expect(during.pose).toBe("sit");
    expect(tl.currentExpression()).toBe("quiet");
  });

  it("不會自動眨眼的表情不收這個提示（回 false，呼叫端才知道要退回一般演出）", () => {
    const tl = new ExpressionTimeline(() => 0.99, 0);
    tl.setAnimation("blocked", 0);
    expect(tl.blinkNow(1_000)).toBe(false);
  });

  it("CompanionApp 在安靜時走就地眨眼，不套成一般 performing", () => {
    const src = fs.readFileSync(path.resolve("src/companion/CompanionApp.tsx"), "utf8");
    expect(src).toContain("blinkedInPlace");
    expect(src).toContain('gate.quiet && action.expression === "blink"');
  });
});

// ---------------------------------------------------------------------------
// 6. 「一小時內不要主動說話」
// ---------------------------------------------------------------------------

describe("#6 安靜期要真的關掉角色自己的主動行為", () => {
  it("proactiveQuietUntil / proactiveQuietActive：有界、到期即失效", () => {
    const now = T0;
    expect(proactiveQuietUntil(60, now)).toBe(now + 3_600_000);
    expect(proactiveQuietUntil(-5, now)).toBe(now); // 負值不倒轉時間
    expect(proactiveQuietUntil(99_999, now)).toBe(now + 24 * 60 * 60_000); // 上限一天
    expect(proactiveQuietActive(now + 1, now)).toBe(true);
    expect(proactiveQuietActive(now, now)).toBe(false);
    expect(proactiveQuietActive(0, now)).toBe(false);
    expect(proactiveQuietActive(Number.NaN, now)).toBe(false);
  });

  it("安靜期內 hover 短氣泡直接關閉", () => {
    const base = {
      hoverMs: 5_000,
      nowMs: T0,
      lastBubbleAt: 0,
      bubblesEnabled: true,
      approachEnabled: true,
      personality: personalityFor("natural"),
      rand: 0.1,
    };
    expect(hoverBubblePolicy({ ...base, quiet: false }).show).toBe(true);
    expect(hoverBubblePolicy({ ...base, quiet: true })).toEqual({
      show: false,
      reason: "quiet",
    });
  });

  it("快捷選單的兩個安靜選項都會設定本機安靜期（不只叫 runtime 閉嘴）", () => {
    const src = fs.readFileSync(path.resolve("src/companion/CompanionApp.tsx"), "utf8");
    const quiet1h = src.slice(src.indexOf('case "quiet-1h":'), src.indexOf('case "estop":'));
    expect(quiet1h).toContain("setLocalQuiet(60)");
    expect(quiet1h).toContain("setLocalQuiet(12 * 60)");
    // 隨口氣泡與知識收據都要被安靜期擋下；安全文字不受影響。
    expect(src).toContain("proactiveQuietActive(quietUntilRef.current, now)");
  });
});

// ---------------------------------------------------------------------------
// 7. 自然到期 ≠ 被搶佔
// ---------------------------------------------------------------------------

describe("#7 已經播完的表演不算被搶佔", () => {
  const performing = (untilMs: number) => ({ kind: "performing" as const, untilMs });

  it("到期後才被換掉 → 不記 interruption", () => {
    expect(wasPreempted(performing(T0), { kind: "clicked", untilMs: T0 + 700 }, T0 + 1)).toBe(
      false
    );
  });

  it("還在播就被換掉 → 記 interruption", () => {
    expect(
      wasPreempted(performing(T0 + 2_000), { kind: "blocked", untilMs: T0 + 4_500 }, T0)
    ).toBe(true);
  });

  it("換成另一個表演、或本來就不是表演 → 不記", () => {
    expect(
      wasPreempted(performing(T0 + 2_000), { kind: "performing", untilMs: T0 + 3_000 }, T0)
    ).toBe(false);
    expect(wasPreempted(null, { kind: "clicked", untilMs: T0 }, T0)).toBe(false);
    expect(
      wasPreempted({ kind: "clicked", untilMs: T0 + 500 }, { kind: "blocked", untilMs: T0 }, T0)
    ).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// 8. estop/cancel 真的停下來
// ---------------------------------------------------------------------------

describe("#8 緊急停止要停掉語音與活著的 transient", () => {
  it("進入 emergency 清掉非安全 transient", () => {
    for (const kind of ["dragged", "performing", "acting", "clicked", "succeeded"] as const) {
      const alive = reduce(
        { base: "idle", transient: null },
        { type: "transient", kind, animation: "stretch" },
        T0
      );
      const stopped = reduce(alive, { type: "base", base: "emergency" }, T0 + 10);
      expect(stopped.transient, kind).toBeNull();
    }
  });

  it("安全訊息本身留著（被擋下/失敗/未知不是要被藏起來的東西）", () => {
    const blocked = reduce(
      { base: "idle", transient: null },
      { type: "transient", kind: "blocked" },
      T0
    );
    const stopped = reduce(blocked, { type: "base", base: "emergency" }, T0 + 10);
    expect(stopped.transient?.kind).toBe("blocked");
    // 畫面仍然是緊急停止（安全狀態最大）。
    expect(pose(stopped, T0 + 20).animation).toBe("emergency");
  });

  it("stopSpeech 會取消進行中的語音（沒有語音服務時是 no-op）", () => {
    const cancel = vi.fn();
    Object.defineProperty(window, "speechSynthesis", {
      value: { cancel },
      configurable: true,
    });
    stopSpeech();
    expect(cancel).toHaveBeenCalledTimes(1);
    Object.defineProperty(window, "speechSynthesis", { value: undefined, configurable: true });
    expect(() => stopSpeech()).not.toThrow();
  });

  it("estop 與 presentation cancel 都呼叫 stopSpeech", () => {
    const src = fs.readFileSync(path.resolve("src/companion/CompanionApp.tsx"), "utf8");
    const cancelBranch = src.slice(src.indexOf('if (command === "cancel"'));
    expect(cancelBranch.slice(0, 600)).toContain("stopSpeech()");
    expect(src).toContain('machineRef.current.base === "emergency" && beforeBase !== "emergency"');
  });
});

// ---------------------------------------------------------------------------
// 9. 被打斷的睡眠不原樣恢復
// ---------------------------------------------------------------------------

describe("#9 剛被戳醒不會馬上躺回去睡", () => {
  const sleepyCtx = (over: Record<string, unknown> = {}) => ({
    nowMs: T0,
    ambient: true,
    quiet: false,
    reducedMotion: false,
    expressiveness: 1,
    msSinceInteraction: 600_000,
    behavior: { ...initialBehavior(0), activation: 0, taskLoad: 0 },
    ...over,
  });

  function directorPlayingDoze() {
    const d = new InteractionDirector(DEFAULT_TUNING, SHU_DIRECTOR_TABLES);
    // rng：hazard 觸發 → 權重抽樣落在最後一個（doze）。
    const a = d.tick(sleepyCtx(), scriptedRng([0.01, 0.999], 0.999));
    expect(a?.expression).toBe("doze");
    d.notePreempted(T0 + 1_000); // 被戳：表演還剩 9 秒
    return d;
  }

  it("互動後 20 秒內不恢復睡眠類 ambient", () => {
    const d = directorPlayingDoze();
    const resumed = d.tick(
      sleepyCtx({ nowMs: T0 + 2_000, msSinceInteraction: 1_000 }),
      () => 0.99 // hazard 不觸發：只可能是「恢復」
    );
    expect(resumed).toBeNull();
  });

  it("過了冷卻期才恢復原本的長 ambient", () => {
    const d = directorPlayingDoze();
    const resumed = d.tick(
      sleepyCtx({
        nowMs: T0 + 2_000,
        msSinceInteraction: SLEEP_RESUME_BLOCK_MS + 1_000,
      }),
      () => 0.99
    );
    expect(resumed?.expression).toBe("doze");
    expect(resumed?.source).toBe("resume");
  });
});

// ---------------------------------------------------------------------------
// 10. Reduced Motion 真的靜態
// ---------------------------------------------------------------------------

describe("#10 Reduced Motion：不是「動小一點」，是不動", () => {
  it("swayAt：reduced 時任何時間都回 0", () => {
    expect(swayAt(1234, 260, 0.35, false)).not.toBe(0);
    for (const t of [0, 137, 4_211, 1e9]) {
      expect(swayAt(t, 260, 0.35, true)).toBe(0);
    }
  });

  it("Reduced Motion + 凍結：不同時間的兩幀畫出完全一樣的東西", () => {
    const rec = recordingCanvas();
    let t = 1_000;
    const stage = new StageRenderer(rec.canvas, "maid-classic", 1, {
      autoStart: false,
      rng: () => 0.5,
      now: () => t,
    });
    stage.setReducedMotion(true);
    stage.setAnimation("emergency");
    stage.setMachineFlags(machineStageFlags("emergency", null, "emergency", false));
    stage.setFamiliars([{ id: "f1", name: "小白", palette: "maid-classic" }]);
    stage.spawnToy("wand"); // 羽毛 sway
    stage.spawnToy("trinket"); // 小物件轉動
    stage.renderFrame(t);
    rec.take();
    stage.renderFrame(t);
    const a = rec.take();
    t = 987_654; // 過了很久
    stage.renderFrame(t);
    const b = rec.take();
    stage.destroy();
    expect(a.length).toBeGreaterThan(50); // 真的有畫東西
    expect(b).toEqual(a);
  });

  it("關掉 Reduced Motion 後同樣兩幀就會不同（證明上面不是因為沒在畫）", () => {
    // run-2 companion-gameplay-003：這裡原本以 emergency（凍結）當「兩幀會不同」的基準，
    // 等於把「凍結時使魔仍抖動、羽毛仍擺」釘成預期。凍結必須完全靜止；改用 idle 當基準。
    const rec = recordingCanvas();
    let t = 1_000;
    const stage = new StageRenderer(rec.canvas, "maid-classic", 1, {
      autoStart: false,
      rng: () => 0.5,
      now: () => t,
    });
    stage.setAnimation("idle");
    stage.setMachineFlags(machineStageFlags("idle", null, "idle", true));
    stage.setFamiliars([{ id: "f1", name: "小白", palette: "maid-classic" }]);
    stage.spawnToy("wand");
    stage.renderFrame(t);
    rec.take();
    stage.renderFrame(t);
    const a = rec.take();
    t = 987_654;
    stage.renderFrame(t);
    const b = rec.take();
    stage.destroy();
    expect(b).not.toEqual(a);
  });

  it("hold 內的慶祝/情緒粒子歸零，但狀態符號（綠勾）保留", () => {
    const tl = new ExpressionTimeline(() => 0.5, 0);
    tl.setReducedMotion(true);
    tl.setAnimation("success-verified", 0);
    const p = tl.paramsAt(1_000);
    expect(p.particles).toBe("none");
    expect(p.particlePhase).toBe(0);
    expect(p.overlay).toBe("check"); // 狀態辨識不能被無障礙設定犧牲

    tl.setAnimation("praised", 2_000);
    expect(tl.paramsAt(3_000).particles).toBe("none");
  });

  it("關掉 Reduced Motion 後粒子回來（不是永久拿掉）", () => {
    const tl = new ExpressionTimeline(() => 0.5, 0);
    tl.setAnimation("success-verified", 0);
    expect(tl.paramsAt(1_000).particles).toBe("sparkle");
  });
});

// ---------------------------------------------------------------------------
// 11. ask 是真相狀態
// ---------------------------------------------------------------------------

describe("#11 「需要確認」只能由 runtime 驅動", () => {
  it("ask 標記為 truthState", () => {
    expect(resolveExpression("ask")?.truthState).toBe(true);
  });

  it("look-at-confirmation 映射到非真相的 question，不再是 requesting-consent", () => {
    const plan = planPresentationCommand(
      "state-present",
      { behaviorIntent: "look-at-confirmation" },
      true
    );
    expect(plan.transient).toBe("performing");
    expect(plan.animation).toBe("question");
    expect(resolveExpression("question")?.truthState ?? false).toBe(false);
  });

  it("presentation 白名單裡沒有任何指令能演出 ask", () => {
    const src = fs.readFileSync(path.resolve("src/companion/presentationCommands.ts"), "utf8");
    expect(src).not.toContain('"requesting-consent"');
  });
});

// ---------------------------------------------------------------------------
// 12/13. 參數強制轉型與外推
// ---------------------------------------------------------------------------

describe("#12 clampParams 不做 Number() 強制轉型", () => {
  it("null/空字串/陣列/布林都回退預設，不是變成 0 或 1", () => {
    const p = clampParams({
      bodyBob: null as never,
      eyeOpen: "" as never,
      squash: [] as never,
      dim: true as never,
      headTilt: "8" as never,
      blush: undefined as never,
    });
    expect(p.bodyBob).toBe(DEFAULT_PARAMS.bodyBob);
    expect(p.eyeOpen).toBe(DEFAULT_PARAMS.eyeOpen);
    expect(p.squash).toBe(DEFAULT_PARAMS.squash);
    expect(p.dim).toBe(DEFAULT_PARAMS.dim);
    expect(p.headTilt).toBe(DEFAULT_PARAMS.headTilt); // 數字字串也不接受
    expect(p.blush).toBe(DEFAULT_PARAMS.blush);
  });

  it("真正的數字照樣進硬界線", () => {
    expect(clampParams({ bodyBob: 999 }).bodyBob).toBe(10);
    expect(clampParams({ eyeOpen: 0.25 }).eyeOpen).toBe(0.25);
    expect(clampParams({ headTilt: Number.NaN }).headTilt).toBe(DEFAULT_PARAMS.headTilt);
  });
});

describe("#13 過場的回彈不能被 lerp 夾掉", () => {
  it("easeOutBackLite 真的會過衝", () => {
    expect(easeOutBackLite(0.7)).toBeGreaterThan(1);
  });

  it("lerpParams 允許有界外推，輸出仍在硬界線內", () => {
    const a = clampParams({ tailAngle: 24 });
    const b = clampParams({ tailAngle: 62 });
    expect(lerpParams(a, b, 1.05).tailAngle).toBeGreaterThan(62);
    expect(lerpParams(a, b, 99).tailAngle).toBeCloseTo(24 + 38 * LERP_T_MAX);
    expect(lerpParams(a, b, 99).tailAngle).toBeLessThanOrEqual(70); // clampParams 收尾
    expect(lerpParams(a, b, Number.NaN).tailAngle).toBe(24);
  });

  it("時間軸過場中某幀真的超過目標，之後回到目標", () => {
    const tl = new ExpressionTimeline(() => 0.5, 0);
    tl.setAnimation("idle", 0);
    tl.paramsAt(1_000);
    tl.setAnimation("praised", 1_000);
    let max = -Infinity;
    for (let t = 1_000; t <= 1_700; t += 8) max = Math.max(max, tl.paramsAt(t).tailAngle);
    expect(max).toBeGreaterThan(62);
    expect(tl.paramsAt(2_500).tailAngle).toBeCloseTo(62, 1);
  });
});

// ---------------------------------------------------------------------------
// 15. 姿勢過場
// ---------------------------------------------------------------------------

describe("#15 lie ↔ stand 不再單幀瞬移", () => {
  it("blendPose：只處理 lie 相關的切換，切換點與權重一致", () => {
    const lie = clampParams({ pose: "lie" });
    const stand = clampParams({ pose: "stand" });
    const sit = clampParams({ pose: "sit" });
    expect(blendPose(lie, stand, stand, 0).pose).toBe("lie");
    expect(blendPose(lie, stand, stand, 0).poseBlend).toBe(1);
    expect(blendPose(lie, stand, stand, 0.49).poseBlend).toBeCloseTo(0.51);
    expect(blendPose(lie, stand, stand, 0.51).pose).toBe("stand");
    expect(blendPose(lie, stand, stand, 0.51).poseBlend).toBeCloseTo(0.51);
    expect(blendPose(lie, stand, stand, 1)).toMatchObject({ pose: "stand", poseBlend: 1 });
    // stand↔sit 只差 10px：不插值（也不該平白多一個通道在動）。
    expect(blendPose(stand, sit, sit, 0.2).pose).toBe(sit.pose);
  });

  it("layoutFor：poseBlend 讓頭中心在兩個姿勢之間連續移動", () => {
    const at = (blend: number) => layoutFor(clampParams({ pose: "lie", poseBlend: blend }), PAL).hy;
    expect(at(1)).toBeCloseTo(92);
    expect(at(0)).toBeCloseTo(46);
    expect(at(0.5)).toBeCloseTo(69);
  });

  it("lie-flat → idle 的過場中，連續兩幀頭部 y 位移 < 12px", () => {
    const tl = new ExpressionTimeline(() => 0.5, 0);
    tl.setAnimation("lie-flat", 0);
    let prevY = layoutFor(tl.paramsAt(3_000), PAL).hy;
    expect(tl.paramsAt(3_000).pose).toBe("lie");
    tl.setAnimation("idle", 3_000);
    let maxJump = 0;
    for (let t = 3_000 + 16.7; t <= 4_200; t += 16.7) {
      const y = layoutFor(tl.paramsAt(t), PAL).hy;
      maxJump = Math.max(maxJump, Math.abs(y - prevY));
      prevY = y;
    }
    expect(maxJump).toBeLessThan(12);
    expect(prevY).toBeCloseTo(46); // 真的站起來了，不是卡在中間
  });

  it("Reduced Motion 不做姿勢過場（直接就位，不是慢動作）", () => {
    const tl = new ExpressionTimeline(() => 0.5, 0);
    tl.setReducedMotion(true);
    tl.setAnimation("lie-flat", 0);
    expect(tl.paramsAt(100).pose).toBe("lie");
    expect(tl.paramsAt(100).poseBlend).toBe(1);
  });
});
