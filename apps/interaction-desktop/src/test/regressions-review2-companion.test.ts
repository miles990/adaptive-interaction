// v0.5 對抗審查（review2）— companion／rig／director／perf 面向的 regression。
//
// 每一則都對應一個 confirmed finding，且**在修正前會紅**：
//   rig-renderer-056      表情的 enter 中途離開 lie 時，crossfade 不再放手（單幀 14.14px → <5px）
//   rig-renderer-058      stand↔sit 也連續（單幀 10.00px → <1px）
//   rig-renderer-059      Reduced Motion 真靜態：連自動眨眼都停
//   director-pipeline-044 「在等你確認」不被連戳／cancel 的 clear-transient 抹掉
//   director-pipeline-045 安靜眨眼靠 Director 的 source 標記，不是硬寫的表情 id
//   director-pipeline-046 Director 上沒有死掉的 score() 轉呼包裝
//   companion-gameplay-032 互動框空白處是「一般視窗互動」，不是死區
//   companion-gameplay-033 Reduced Motion 讓使魔收斂到靜止，Roll Call 誠實
//   companion-gameplay-034 玩具已滿時丟光點：誠實拒絕，不假裝生成
//   companion-gameplay-035 主角會主動走過去跟使魔打招呼（互相注意是雙向的）
//   perf-claims-007       Reduced Motion 下主迴圈不再每幀重畫同一張圖
//   perf-claims-008       30fps 降級看得到「實際幀距 vs 螢幕基準」，不只 JS 成本
//   perf-claims-011       pause() 之後真的不再回報互動框

import { afterEach, describe, expect, it } from "vitest";
import { ExpressionTimeline } from "../companion/rig/timeline";
import { layoutFor } from "../companion/rig/draw";
import { RIG_PALETTES } from "../companion/rig/params";
import { REDUCED_TICK_MS, StageRenderer } from "../companion/rig/stage";
import {
  createWorld,
  rollCall,
  StepInputs,
  stepWorld,
  World,
} from "../companion/playfield";
import { pose, reduce, TransientKind } from "../companion/machine";
import {
  EMPTY_DIRECTOR_TABLES,
  InteractionDirector,
  DirectorTables,
} from "../companion/director";
import { initialBehavior } from "../companion/behavior";
import {
  frameBudgetPolicy,
  framePacingPolicy,
  FRAME_WINDOW,
  initialFrameBudget,
  initialFramePacing,
  shouldDrawFrame,
} from "../companion/gameFeel";

const PAL = RIG_PALETTES["maid-classic"];
const T0 = 1_000_000;

/** jsdom 沒有 canvas 2D：可鏈式 stub（stage 只發繪圖指令，不讀像素）。 */
function stubCanvas(w = 320, h = 170): HTMLCanvasElement {
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

/** 手動驅動的 requestAnimationFrame（測 loop 自己的排程行為）。 */
function fakeRaf() {
  const queue: FrameRequestCallback[] = [];
  const origRaf = globalThis.requestAnimationFrame;
  const origCancel = globalThis.cancelAnimationFrame;
  globalThis.requestAnimationFrame = ((cb: FrameRequestCallback) => {
    queue.push(cb);
    return queue.length;
  }) as typeof globalThis.requestAnimationFrame;
  globalThis.cancelAnimationFrame = (() => {}) as typeof globalThis.cancelAnimationFrame;
  return {
    /** 跑 n 個 rAF 回呼（每個回呼會自己排下一個）。 */
    step(n: number) {
      for (let i = 0; i < n; i++) {
        const cb = queue.shift();
        if (!cb) break;
        cb(0);
      }
    },
    restore() {
      globalThis.requestAnimationFrame = origRaf;
      globalThis.cancelAnimationFrame = origCancel;
    },
  };
}

let raf: ReturnType<typeof fakeRaf> | null = null;
afterEach(() => {
  raf?.restore();
  raf = null;
});

function stepInputs(over: Partial<StepInputs> = {}): StepInputs {
  return {
    nowMs: T0,
    dtMs: 100,
    ambient: true,
    frozen: false,
    quiet: false,
    reducedMotion: false,
    playEnabled: true,
    cursorPlayEnabled: true,
    deskMoveEnabled: false,
    pointer: null,
    ...over,
  };
}

/** 兩個表情之間切換，量「連續兩幀頭中心 y 的最大位移」。 */
function headJump(from: string, to: string, untilMs = 2_000): { max: number; atMs: number } {
  const tl = new ExpressionTimeline(() => 0.5, 0);
  tl.setAnimation(from, 0);
  tl.paramsAt(3_000);
  tl.setAnimation(to, 3_000);
  let prevY = layoutFor(tl.paramsAt(3_000), PAL).hy;
  let max = 0;
  let atMs = 0;
  for (let t = 3_000 + 16.7; t <= 3_000 + untilMs; t += 16.7) {
    const y = layoutFor(tl.paramsAt(t), PAL).hy;
    const d = Math.abs(y - prevY);
    if (d > max) {
      max = d;
      atMs = t - 3_000;
    }
    prevY = y;
  }
  return { max, atMs };
}

// ---------------------------------------------------------------------------
// rig-renderer-056 / 058：姿勢過場
// ---------------------------------------------------------------------------

describe("rig-renderer-056：表情的 enter 中途換姿勢時，crossfade 不放手", () => {
  // startled-awake 的 enter 是 lie → crouch → stand：crossfade 窗口還沒關，
  // 目標姿勢自己就離開了 lie。舊 blendPose 只在「有一端是 lie」時作用，那一幀
  // 直接放手，頭中心單幀跳 14.14px（poked 起手）／10.41px（idle 起手）。
  for (const from of ["poked", "idle", "lie-flat", "sleep", "sneak-closer", "sit"]) {
    it(`${from} → startled-awake：連續兩幀頭部位移 < 5px`, () => {
      const { max, atMs } = headJump(from, "startled-awake");
      expect(max, `worst at +${Math.round(atMs)}ms`).toBeLessThan(5);
    });
  }

  it("全表情對掃描：任何一對之間都 < 6px（舊實作最差 14.14px）", () => {
    const names = [
      "idle",
      "sit",
      "doze",
      "lie-flat",
      "sleep",
      "startled-awake",
      "stretch",
      "poked",
      "poked-grin",
      "sneak-closer",
      "pounce-miss",
      "quiet",
      "land-light",
      "success-verified",
      "emergency",
      "offline",
    ];
    let worst = { max: 0, pair: "" };
    for (const a of names) {
      for (const b of names) {
        if (a === b) continue;
        const { max } = headJump(a, b);
        if (max > worst.max) worst = { max, pair: `${a} → ${b}` };
      }
    }
    expect(worst.max, `worst pair ${worst.pair}`).toBeLessThan(6);
  });
});

describe("rig-renderer-058：stand↔sit 不再硬切", () => {
  // 8 個常駐 ambient 都是坐姿：舊實作在切換點單幀跳 10.00px（drop=10）。
  for (const to of [
    "sit",
    "doze",
    "legswing",
    "tailhug",
    "caught-slacking",
    "await-player",
    "player-back",
    "quiet",
  ]) {
    it(`idle → ${to}：連續兩幀頭部位移 < 1px`, () => {
      const { max, atMs } = headJump("idle", to);
      expect(max, `worst at +${Math.round(atMs)}ms`).toBeLessThan(1);
    });
  }

  it("坐姿真的到位（頭中心 56）而不是卡在中間", () => {
    const tl = new ExpressionTimeline(() => 0.5, 0);
    tl.setAnimation("idle", 0);
    tl.paramsAt(3_000);
    tl.setAnimation("sit", 3_000);
    const p = tl.paramsAt(5_000);
    expect(p.pose).toBe("sit");
    expect(layoutFor(p, PAL).hy).toBeCloseTo(56, 3);
  });
});

// ---------------------------------------------------------------------------
// rig-renderer-059：Reduced Motion 真靜態
// ---------------------------------------------------------------------------

describe("rig-renderer-059：Reduced Motion 連自動眨眼都停", () => {
  for (const expr of ["idle", "sit", "lie-flat", "spaced-out", "quiet"]) {
    it(`${expr}：20 秒內每一個通道都恆等於 hold`, () => {
      const tl = new ExpressionTimeline(() => 0.5, 0);
      tl.setReducedMotion(true);
      tl.setAnimation(expr, 0);
      const base = tl.paramsAt(0);
      for (let t = 0; t <= 20_000; t += 137) {
        const p = tl.paramsAt(t);
        expect(p.eyeOpen, `eyeOpen at ${t}ms`).toBe(base.eyeOpen);
        expect(p, `params at ${t}ms`).toEqual(base);
      }
    });
  }

  it("關掉 Reduced Motion 之後眨眼回來（不是被永久拿掉）", () => {
    const tl = new ExpressionTimeline(() => 0.5, 0);
    tl.setAnimation("idle", 0);
    let dipped = false;
    for (let t = 0; t <= 20_000; t += 33) {
      if (tl.paramsAt(t).eyeOpen < 0.5) dipped = true;
    }
    expect(dipped).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// director-pipeline-044：「在等你確認」是安全層
// ---------------------------------------------------------------------------

describe("director-pipeline-044：非 force 的 clear-transient 不得抹掉 requesting-consent", () => {
  const seeded = (kind: TransientKind) =>
    reduce({ base: "idle", transient: null }, { type: "transient", kind, animation: "x" }, T0);

  for (const kind of ["blocked", "failed", "unknown", "requesting-consent"] as TransientKind[]) {
    it(`${kind}：連戳／cancel 的 clear-transient 清不掉`, () => {
      const s = seeded(kind);
      expect(s.transient?.kind).toBe(kind);
      const after = reduce(s, { type: "clear-transient" }, T0 + 1);
      expect(after.transient?.kind).toBe(kind);
    });
  }

  it("force（estop clear-all）仍然清得掉", () => {
    const s = seeded("requesting-consent");
    expect(reduce(s, { type: "clear-transient", force: true }, T0 + 1).transient).toBeNull();
  });

  it("連戳（先 clear-transient 再套 performing）不會把 ask 換成玩鬧姿勢", () => {
    let s = seeded("requesting-consent");
    s = reduce(s, { type: "clear-transient" }, T0 + 1);
    s = reduce(s, { type: "transient", kind: "performing", animation: "poked-rapid" }, T0 + 2);
    expect(s.transient?.kind).toBe("requesting-consent");
    expect(pose(s, T0 + 3).animation).toBe("ask");
  });

  it("緊急停止仍然把「在等你確認」下台（收回同意不該還舉著手）", () => {
    const s = seeded("requesting-consent");
    expect(reduce(s, { type: "base", base: "emergency" }, T0 + 1).transient).toBeNull();
    // 安全訊息本身照樣留著。
    const blocked = seeded("blocked");
    expect(reduce(blocked, { type: "base", base: "emergency" }, T0 + 1).transient?.kind).toBe("blocked");
  });
});

// ---------------------------------------------------------------------------
// director-pipeline-045 / 046
// ---------------------------------------------------------------------------

describe("director-pipeline-045：安靜眨眼由 Director 的 source 標記識別", () => {
  it("眨眼 id 由角色 tables 注入，動作帶 source:\"blink\"", () => {
    const tables: DirectorTables = {
      ...EMPTY_DIRECTOR_TABLES,
      isPlayable: () => true,
      blink: { expression: "eyes-shut", durationMs: 300 },
    };
    const d = new InteractionDirector(undefined, tables);
    const action = d.tick(
      {
        nowMs: T0,
        ambient: true,
        quiet: true,
        reducedMotion: false,
        expressiveness: 1,
        msSinceInteraction: 300_000,
        behavior: initialBehavior(0),
      },
      () => 0.01
    );
    expect(action).toMatchObject({ expression: "eyes-shut", source: "blink" });
  });
});

describe("director-pipeline-046：Director 沒有死掉的 score() 轉呼包裝", () => {
  it("score 不再是 Director 的 API", () => {
    const d = new InteractionDirector(undefined, EMPTY_DIRECTOR_TABLES);
    expect((d as unknown as { score?: unknown }).score).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// companion-gameplay-032：互動框空白處不是死區
// ---------------------------------------------------------------------------

describe("companion-gameplay-032：互動框內的空白會回 \"stage\"，不是靜默的 none", () => {
  function stageWithFarToy() {
    let t = 1_000;
    const stage = new StageRenderer(stubCanvas(), "maid-classic", 1, {
      autoStart: false,
      rng: () => 0.9,
      now: () => t,
    });
    stage.setMachineFlags({ ambient: true, frozen: false, quiet: false, playPerforming: false });
    stage.spawnToy("yarn");
    const toy = stage.toyPoints()[0];
    expect(stage.pointerDown(toy.x, toy.y)).toBe("toy");
    t += 50;
    stage.pointerMove(300, 100); // 把毛球拖到遊玩場另一側
    return stage;
  }

  it("角色與遠處玩具之間的空白：回 \"stage\"（可拖視窗／開選單），不是 none", () => {
    const stage = stageWithFarToy();
    const b = stage.interactiveBounds();
    const char = stage.charHitRect();
    const x = char.x + char.w + 34; // 框內、角色外、離玩具很遠
    const y = char.y + 18;
    expect(x).toBeGreaterThan(char.x + char.w);
    expect(x).toBeLessThan(b.x + b.w);
    expect(stage.pointerDown(x, y)).toBe("stage");
    stage.destroy();
  });

  it("互動框外仍然是 none（Rust 端讓它穿透到桌面）", () => {
    const stage = stageWithFarToy();
    const b = stage.interactiveBounds();
    expect(stage.pointerDown(b.x + b.w + 20, b.y + 10)).toBe("none");
    stage.destroy();
  });
});

// ---------------------------------------------------------------------------
// companion-gameplay-033：Reduced Motion 不得凍結出「還在動」的假象
// ---------------------------------------------------------------------------

describe("companion-gameplay-033：Reduced Motion 讓使魔收斂到靜止，Roll Call 誠實", () => {
  function worldWithGreetingFamiliar(): World {
    const w = createWorld(320, 170);
    return {
      ...w,
      char: { ...w.char, attendTo: "f1", attendUntil: T0 + 2_000, greetBackUntil: T0 + 1_800 },
      familiars: [
        {
          id: "f1",
          name: "小白",
          palette: "maid-classic",
          x: 60,
          vx: 25,
          facing: 1,
          state: "greet",
          stateUntil: T0 + 2_500,
          greetWith: "char",
        },
        {
          id: "f2",
          name: "小黑",
          palette: "maid-classic",
          x: 240,
          vx: -30,
          facing: -1,
          state: "walk",
          stateUntil: T0 + 3_000,
          greetWith: null,
        },
      ],
    };
  }

  it("開啟 Reduced Motion：使魔收到 idle／vx=0，愛心不再永遠掛著", () => {
    let w = worldWithGreetingFamiliar();
    for (let i = 0; i < 40; i++) {
      w = stepWorld(w, stepInputs({ nowMs: T0 + i * 100, reducedMotion: true }), () => 0.5).world;
    }
    for (const f of w.familiars) {
      expect(f.state, f.id).toBe("idle");
      expect(f.vx, f.id).toBe(0);
      expect(f.greetWith, f.id).toBeNull();
    }
    expect(w.char.greetBackUntil).toBe(0);
    expect(w.char.attendTo).toBeNull();
  });

  it("Roll Call 不說「在散步」「在打招呼」——牠們真的停下來了", () => {
    const w = worldWithGreetingFamiliar();
    const honest = rollCall(w, "小樞", null, T0 + 1_000, { reducedMotion: true });
    expect(honest.map((r) => r.activity)).toEqual(["停下來了", "停下來了", "停下來了"]);
    // 一般情況（沒有 Reduced Motion）照舊回報實際狀態。
    const normal = rollCall(w, "小樞", null, T0 + 1_000, {});
    expect(normal[1].activity).toBe("在跟大家打招呼");
    expect(normal[2].activity).toBe("在散步");
  });
});

// ---------------------------------------------------------------------------
// companion-gameplay-034：玩具已滿時誠實拒絕
// ---------------------------------------------------------------------------

describe("companion-gameplay-034：玩具已滿時丟光點不得回報成功", () => {
  function fullStage() {
    const stage = new StageRenderer(stubCanvas(), "maid-classic", 1, {
      autoStart: false,
      rng: () => 0.9,
      now: () => 1_000,
    });
    stage.setMachineFlags({ ambient: true, frozen: false, quiet: false, playPerforming: false });
    for (const k of ["yarn", "paper", "plane", "yarn"] as const) {
      expect(stage.spawnToy(k), k).toBe(true);
    }
    expect(stage.toyCount()).toBe(4);
    return stage;
  }

  it("已滿＋場上沒有光點：回 false、數量不變（沒生成就不說生成了）", () => {
    const stage = fullStage();
    expect(stage.spawnToy("light")).toBe(false);
    expect(stage.spawnToy("wand")).toBe(false);
    expect(stage.toyCount()).toBe(4);
    stage.destroy();
  });

  it("光點已在場上：重生＝替換，仍然回 true", () => {
    const stage = new StageRenderer(stubCanvas(), "maid-classic", 1, {
      autoStart: false,
      rng: () => 0.9,
      now: () => 1_000,
    });
    stage.setMachineFlags({ ambient: true, frozen: false, quiet: false, playPerforming: false });
    expect(stage.spawnToy("light")).toBe(true);
    expect(stage.spawnToy("light")).toBe(true); // 替換
    expect(stage.toyCount()).toBe(1);
    stage.destroy();
  });

  it("凍結時照樣拒絕", () => {
    const stage = new StageRenderer(stubCanvas(), "maid-classic", 1, {
      autoStart: false,
      rng: () => 0.9,
      now: () => 1_000,
    });
    stage.setMachineFlags({ ambient: false, frozen: true, quiet: false, playPerforming: false });
    expect(stage.spawnToy("light")).toBe(false);
    stage.destroy();
  });
});

// ---------------------------------------------------------------------------
// companion-gameplay-035：互相注意是雙向的
// ---------------------------------------------------------------------------

describe("companion-gameplay-035：主角會主動走過去跟使魔打招呼", () => {
  function worldWithIdleFamiliar(): World {
    const w = createWorld(320, 170);
    return {
      ...w,
      char: { ...w.char, x: 160 },
      familiars: [
        {
          id: "f1",
          name: "小白",
          palette: "maid-classic",
          x: 60,
          vx: 0,
          facing: 1,
          state: "idle",
          stateUntil: T0 + 10_000_000, // 不再重抽，隔離主角這一側的行為
          greetWith: null,
        },
      ],
    };
  }

  it("free 時會抽到 greet-familiar，走過去、停下、冒愛心，對方也回過頭", () => {
    let w = worldWithIdleFamiliar();
    let greeted = false;
    for (let i = 0; i < 400 && !greeted; i++) {
      const r = stepWorld(w, stepInputs({ nowMs: T0 + i * 100 }), () => 0.001);
      w = r.world;
      greeted = r.events.some((e) => e.type === "greeted-familiar" && e.id === "f1");
      if (i === 0) expect(w.char.mode).toBe("greet-familiar");
    }
    expect(greeted).toBe(true);
    expect(Math.abs(w.char.x - 60)).toBeLessThanOrEqual(34); // 真的走到旁邊
    expect(w.char.greetBackUntil).toBeGreaterThan(T0); // 冒了一顆愛心
    expect(w.familiars[0].state).toBe("greet");
    expect(w.familiars[0].greetWith).toBe("char");
  });

  it("Roll Call 對「正要去打招呼」有誠實的說法", () => {
    let w = worldWithIdleFamiliar();
    w = stepWorld(w, stepInputs({ nowMs: T0 }), () => 0.001).world;
    expect(w.char.mode).toBe("greet-familiar");
    expect(rollCall(w, "小樞", null, T0, {})[0].activity).toBe("正要去跟朋友打招呼");
  });

  it("不玩耍／安靜時不會主動去打招呼", () => {
    let w = worldWithIdleFamiliar();
    for (let i = 0; i < 50; i++) {
      w = stepWorld(w, stepInputs({ nowMs: T0 + i * 100, quiet: true }), () => 0.001).world;
    }
    expect(w.char.mode).toBe("free");
    expect(w.char.targetFamiliar).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// perf-claims-007：Reduced Motion 的靜態短路
// ---------------------------------------------------------------------------

describe("perf-claims-007：Reduced Motion 下不再每幀重畫同一張圖", () => {
  it("畫面沒變時只畫第一幀＋每 500ms 一次世界維護", () => {
    raf = fakeRaf();
    let t = 1_000;
    const stage = new StageRenderer(stubCanvas(), "maid-classic", 1, {
      autoStart: false,
      rng: () => 0.9,
      now: () => t,
    });
    stage.setMachineFlags({ ambient: true, frozen: false, quiet: false, playPerforming: false });
    stage.setAnimation("idle");
    stage.setReducedMotion(true);
    stage.start();
    for (let i = 0; i < 60; i++) {
      t += 16.7; // 一秒的 60Hz
      raf.step(1);
    }
    const s = stage.loopStats();
    stage.destroy();
    expect(s.ticks).toBe(61); // start() 自己就跑了第一拍
    // 第一幀＋每 REDUCED_TICK_MS 一次；絕不是 60 幀。
    expect(s.drawn).toBeLessThanOrEqual(1 + Math.ceil((60 * 16.7) / REDUCED_TICK_MS));
    expect(s.drawn).toBeLessThan(6);
  });

  it("狀態真的變了就重畫（換表情／指標／玩具都算）", () => {
    raf = fakeRaf();
    let t = 1_000;
    const stage = new StageRenderer(stubCanvas(), "maid-classic", 1, {
      autoStart: false,
      rng: () => 0.9,
      now: () => t,
    });
    stage.setMachineFlags({ ambient: true, frozen: false, quiet: false, playPerforming: false });
    stage.setAnimation("idle");
    stage.setReducedMotion(true);
    stage.start();
    t += 16.7;
    raf.step(1);
    expect(stage.isStaticDrawn()).toBe(true);
    stage.setAnimation("blocked");
    expect(stage.isStaticDrawn()).toBe(false);
    t += 16.7;
    raf.step(1);
    expect(stage.isStaticDrawn()).toBe(true);
    stage.destroy();
  });

  it("沒有 Reduced Motion 時照樣每幀畫（沒有偷偷降到靜態）", () => {
    raf = fakeRaf();
    let t = 1_000;
    const stage = new StageRenderer(stubCanvas(), "maid-classic", 1, {
      autoStart: false,
      rng: () => 0.9,
      now: () => t,
    });
    stage.setMachineFlags({ ambient: true, frozen: false, quiet: false, playPerforming: false });
    stage.setAnimation("idle");
    stage.start();
    for (let i = 0; i < 30; i++) {
      t += 16.7;
      raf.step(1);
    }
    const s = stage.loopStats();
    stage.destroy();
    expect(s.drawn).toBe(31); // start() 自己就畫了第一幀
  });
});

// ---------------------------------------------------------------------------
// perf-claims-008：降級訊號要看得到真正的掉幀
// ---------------------------------------------------------------------------

describe("perf-claims-008：30fps 降級改看「實際幀距 vs 螢幕基準」", () => {
  const feedPacing = (gapMs: number, from = initialFramePacing(), n = FRAME_WINDOW) => {
    let s = from;
    for (let i = 0; i < n; i++) s = framePacingPolicy(s, gapMs);
    return s;
  };

  it("純 JS 成本的政策抓不到合成瓶頸（這就是缺陷本體）", () => {
    let b = initialFrameBudget();
    for (let i = 0; i < FRAME_WINDOW * 3; i++) b = frameBudgetPolicy(b, 0.24); // 實測中位數
    expect(b.skipEveryOther).toBe(false); // 12ms 門檻永遠碰不到
  });

  it("幀距掉到基準的兩倍 → 降級；回到基準附近 → 回全速（遲滯）", () => {
    let p = feedPacing(1000 / 60); // 60Hz 基準
    expect(p.baselineMs).toBeCloseTo(16.67, 1);
    expect(p.missing).toBe(false);
    p = feedPacing(1000 / 30, p); // 每一幀都掉一幀
    expect(p.missing).toBe(true);
    p = feedPacing(17.5, p); // 1.05× 基準
    expect(p.missing).toBe(false);
  });

  it("120Hz 螢幕的 8.3ms 不會被誤判（門檻是相對的，不是絕對毫秒）", () => {
    const p = feedPacing(1000 / 120);
    expect(p.baselineMs).toBeCloseTo(8.33, 1);
    expect(p.missing).toBe(false);
    // 舊的絕對門檻（12ms）會把 60Hz 的 16.67ms 判成太慢——perf-claims-017。
    expect(feedPacing(1000 / 60).missing).toBe(false);
  });

  it("兩條訊號取聯集：任一成立就每兩幀畫一次", () => {
    const fast = initialFrameBudget();
    const missing = { ...initialFramePacing(), missing: true };
    expect(shouldDrawFrame(fast, 1)).toBe(true);
    expect(shouldDrawFrame(fast, 1, missing)).toBe(false);
    expect(shouldDrawFrame(fast, 0, missing)).toBe(true);
  });

  it("StageRenderer 的主迴圈真的會因為掉幀而降級（不是只有純函式）", () => {
    raf = fakeRaf();
    let t = 1_000;
    const stage = new StageRenderer(stubCanvas(), "maid-classic", 1, {
      autoStart: false,
      rng: () => 0.9,
      now: () => t,
    });
    stage.setMachineFlags({ ambient: true, frozen: false, quiet: false, playPerforming: false });
    stage.setAnimation("idle");
    stage.start();
    for (let i = 0; i < FRAME_WINDOW + 1; i++) {
      t += 1000 / 60; // 先跑滿速，建立這台螢幕的基準
      raf.step(1);
    }
    expect(stage.framePacing().missing).toBe(false);
    const before = stage.loopStats().drawn;
    for (let i = 0; i < FRAME_WINDOW + 1; i++) {
      t += 1000 / 30; // 合成端變慢：每一幀都掉
      raf.step(1);
    }
    expect(stage.framePacing().missing).toBe(true);
    const midway = stage.loopStats().drawn;
    for (let i = 0; i < 20; i++) {
      t += 1000 / 30;
      raf.step(1);
    }
    const after = stage.loopStats();
    stage.destroy();
    expect(midway).toBeGreaterThan(before);
    // 降級後每兩幀才畫一次。
    expect(after.drawn - midway).toBeLessThanOrEqual(11);
  });
});

// ---------------------------------------------------------------------------
// perf-claims-011：pause() 的契約
// ---------------------------------------------------------------------------

describe("perf-claims-011：pause() 之後真的不再回報互動框", () => {
  it("暫停中連 force 心跳都不回報（隱藏／suspend 後不再打 IPC）", () => {
    const stage = new StageRenderer(stubCanvas(), "maid-classic", 1, {
      autoStart: false,
      rng: () => 0.9,
      now: () => 1_000,
    });
    const reports: unknown[] = [];
    stage.onHitRect((r) => reports.push(r));
    stage.reportHitRect(true);
    expect(reports).toHaveLength(1);
    stage.pause();
    stage.reportHitRect(true);
    stage.reportHitRect(true);
    expect(reports).toHaveLength(1);
    stage.resume();
    stage.reportHitRect(true);
    expect(reports).toHaveLength(2);
    stage.destroy();
  });

  it("destroy 之後也不回報", () => {
    const stage = new StageRenderer(stubCanvas(), "maid-classic", 1, {
      autoStart: false,
      rng: () => 0.9,
      now: () => 1_000,
    });
    const reports: unknown[] = [];
    stage.onHitRect((r) => reports.push(r));
    stage.destroy();
    stage.reportHitRect(true);
    expect(reports).toHaveLength(0);
  });
});
