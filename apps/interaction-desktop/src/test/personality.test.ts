// v0.5 修復回歸：個性要真的影響行為（spec §4.3），不是只存在於對話。
//
// personality.ts 是純函式；這裡同時釘死它有真的接到
// Director（冷卻/變體/假裝沒看到）、Playfield（速度/距離）與
// ExpressionTimeline（耳→視線→頭的分段）。

import { describe, expect, it } from "vitest";
import {
  dominantTrait,
  personalityFor,
  tuningFor,
  tuningForPreferences,
} from "../companion/personality";
import { InteractionDirector } from "../companion/director";
import { initialBehavior } from "../companion/behavior";
// CPP：變體權重表與 Director 表屬於角色（shu adapter tables）。
import { SHU_DIRECTOR_TABLES, SHU_VARIANT_WEIGHTS } from "../character/adapters/shuTables";
import { createWorld, spawnToy, StepInputs, stepWorld, World } from "../companion/playfield";
import { ExpressionTimeline } from "../companion/rig/timeline";

const inputs = (over?: Partial<StepInputs>): StepInputs => ({
  nowMs: 1_000_000,
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
});

function runStroll(world: World, over: Partial<StepInputs>, steps: number): number {
  let w: World = {
    ...world,
    char: { ...world.char, mode: "stroll", pounceX: world.w - 20, modeUntil: 9_999_999 },
  };
  const base = inputs(over);
  for (let i = 0; i < steps; i++) {
    w = stepWorld(w, { ...base, nowMs: base.nowMs + i * base.dtMs }, () => 0.5).world;
  }
  return w.char.x;
}

describe("個性派生（PersonalityProfile / tuning）", () => {
  it("三種表現度產生不同 tuning（quiet 慢、lively 快）", () => {
    const quiet = tuningForPreferences("quiet");
    const natural = tuningForPreferences("natural");
    const lively = tuningForPreferences("lively");
    expect(quiet.speedScale).toBeLessThan(natural.speedScale);
    expect(natural.speedScale).toBeLessThan(lively.speedScale);
    expect(quiet).not.toEqual(natural);
    expect(natural).not.toEqual(lively);
    // 未知表現度回落到自然，不會炸掉。
    expect(tuningForPreferences("nonsense")).toEqual(natural);
  });

  it("persona 會偏移個性（純函式、可預期）", () => {
    const shu = personalityFor("natural", "persona-shu");
    const navigator = personalityFor("natural", "persona-navigator");
    expect(shu.curious).toBeGreaterThan(navigator.curious);
    expect(navigator.smart).toBeGreaterThan(shu.smart);
    // 未知 persona = 不偏移。
    expect(personalityFor("natural", "persona-unknown")).toEqual(personalityFor("natural"));
  });

  it("慵懶：速度低於預設，且趴著/打哈欠的權重更高", () => {
    const lazy = tuningFor(
      {
        smart: 0.5,
        witty: 0.3,
        playful: 0.2,
        lazy: 1,
        proud: 0.3,
        curious: 0.3,
      },
      SHU_VARIANT_WEIGHTS
    );
    const neutral = tuningForPreferences("natural", undefined, SHU_VARIANT_WEIGHTS);
    expect(lazy.speedScale).toBeLessThan(neutral.speedScale);
    expect(lazy.chaseSpeedScale).toBeLessThan(neutral.chaseSpeedScale);
    expect(lazy.variantWeights["lie-flat"]).toBeGreaterThan(neutral.variantWeights["lie-flat"]);
    expect(lazy.variantWeights["yawn"]).toBeGreaterThan(neutral.variantWeights["yawn"]);
    // 慢半拍起身。
    expect(lazy.riseDelayMs).toBeGreaterThan(neutral.riseDelayMs);
    // 沒有角色權重表（engine-neutral）：variantWeights 為空，Director 一律視為 1。
    expect(tuningForPreferences("natural").variantWeights).toEqual({});
  });

  it("聰明：注意力順序永遠是耳→視線→頭，越聰明越緊湊", () => {
    const smart = tuningFor({
      smart: 1,
      witty: 0.5,
      playful: 0.5,
      lazy: 0.2,
      proud: 0.4,
      curious: 0.6,
    });
    const slow = tuningFor({
      smart: 0,
      witty: 0.5,
      playful: 0.5,
      lazy: 0.2,
      proud: 0.4,
      curious: 0.6,
    });
    for (const t of [smart, slow]) {
      expect(t.attentionStagger.earMs).toBeLessThan(t.attentionStagger.gazeMs);
      expect(t.attentionStagger.gazeMs).toBeLessThan(t.attentionStagger.headMs);
    }
    expect(smart.attentionStagger.headMs).toBeLessThan(slow.attentionStagger.headMs);
  });

  it("好奇會靠得更近；dominantTrait 取最突出的特質", () => {
    const curious = tuningFor({
      smart: 0.5,
      witty: 0.3,
      playful: 0.3,
      lazy: 0.2,
      proud: 0.3,
      curious: 1,
    });
    const incurious = tuningFor({
      smart: 0.5,
      witty: 0.3,
      playful: 0.3,
      lazy: 0.2,
      proud: 0.3,
      curious: 0,
    });
    expect(curious.approachDistance).toBeLessThan(incurious.approachDistance);
    expect(dominantTrait(personalityFor("lively"))).toBe("curious");
  });
});

describe("個性接線：Director", () => {
  const ctx = (over?: Partial<Parameters<InteractionDirector["tick"]>[0]>) => ({
    nowMs: 1_000_000,
    ambient: true,
    quiet: false,
    reducedMotion: false,
    expressiveness: 1,
    msSinceInteraction: 600_000,
    behavior: { ...initialBehavior(0), activation: 0.05, taskLoad: 0 },
    ...over,
  });

  it("cooldownScale 拉長冷卻：慵懶的個性更久才會重播同一動作", () => {
    const lazyTuning = tuningFor({
      smart: 0.5,
      witty: 0.3,
      playful: 0,
      lazy: 1,
      proud: 0.3,
      curious: 0.3,
    });
    const lazy = new InteractionDirector(lazyTuning, SHU_DIRECTOR_TABLES);
    const brisk = new InteractionDirector(
      tuningFor({ smart: 0.5, witty: 0.5, playful: 1, lazy: 0, proud: 0.3, curious: 0.5 }),
      SHU_DIRECTOR_TABLES
    );
    // blink 冷卻 2s：俏皮 0.65 倍 → 1.5s 後可重播；慵懶 1.6 倍 → 還在冷卻。
    lazy.react("curious", 0);
    brisk.react("curious", 0);
    expect(brisk.react("curious", 6_000)).toBeTruthy(); // 8s * 0.65 = 5.2s
    expect(lazy.react("curious", 6_000)).toBeNull(); // 8s * 1.6 = 12.8s
  });

  it("俏皮：給了 rng 時偶爾假裝沒看到（仍是白名單內的表情）", () => {
    const playful = new InteractionDirector(
      tuningFor({ smart: 0.5, witty: 0.5, playful: 1, lazy: 0.4, proud: 0.3, curious: 0.5 }),
      SHU_DIRECTOR_TABLES
    );
    // rng 永遠命中 → 假裝沒聽見；不給 rng 則維持原意圖（確定性）。
    expect(playful.react("notice", 0, 2_500, () => 0)?.expression).toBe("pretend-not-hear");
    expect(playful.react("notice", 100_000)?.expression).toBe("notice");
    // rng 永遠不命中 → 原意圖。
    expect(playful.react("peek", 200_000, 2_500, () => 0.99)?.expression).toBe("peek");
  });

  it("變體權重：慵懶的角色更常挑到趴平/打哈欠等休息動作", () => {
    const lazyTuning = tuningFor(
      {
        smart: 0.5,
        witty: 0.3,
        playful: 0,
        lazy: 1,
        proud: 0.3,
        curious: 0.2,
      },
      SHU_VARIANT_WEIGHTS
    );
    // 掃過整個權重輪盤，統計「休息類」變體占多少比例（每次都用全新
    // Director，避免冷卻/防重複干擾這個純權重量測）。
    const restShare = (tuning?: ReturnType<typeof tuningFor>) => {
      const REST = ["lie-flat", "yawn", "doze", "spaced-out", "stretch"];
      const N = 200;
      let hits = 0;
      for (let i = 0; i < N; i++) {
        const d = new InteractionDirector(tuning ?? tuningForPreferences("natural", undefined, SHU_VARIANT_WEIGHTS), SHU_DIRECTOR_TABLES);
        let call = 0;
        // 第一次 rng＝hazard（必觸發），第二次＝輪盤位置。
        const rng = () => (call++ === 0 ? 0.001 : (i + 0.5) / N);
        const a = d.tick(ctx(), rng);
        if (a && REST.includes(a.expression)) hits += 1;
      }
      return hits / N;
    };
    expect(restShare(lazyTuning)).toBeGreaterThan(restShare());
  });
});

describe("個性接線：Playfield 與 Timeline", () => {
  it("speedScale 真的改變移動距離（經 StepInputs）", () => {
    const world = createWorld(320, 170);
    const slow = runStroll(world, { speedScale: 0.5 }, 60);
    const fast = runStroll(world, { speedScale: 1.4 }, 60);
    const neutral = runStroll(world, {}, 60);
    expect(slow).toBeLessThan(neutral);
    expect(neutral).toBeLessThan(fast);
  });

  it("approachDistance 改變「多近才撲」", () => {
    const build = (approach: number) => {
      let w = spawnToy(createWorld(320, 170), "yarn", 1_000_000);
      // 玩具放在角色右側 30px 的地面上。
      w = {
        ...w,
        toys: w.toys.map((t) => ({ ...t, x: w.char.x + 30, y: w.ground - 10 })),
        char: { ...w.char, mode: "chase", targetToy: w.toys[0].id, modeUntil: 0 },
      };
      const base = inputs({ approachDistance: approach });
      return stepWorld(w, base, () => 0.5).world.char.mode;
    };
    expect(build(20)).toBe("chase"); // 30px 還不夠近
    expect(build(40)).toBe("pounce"); // 放寬距離就會撲
  });

  it("attentionStagger：耳朵先動，頭最後才轉", () => {
    const tl = new ExpressionTimeline(() => 0.5, 0);
    tl.setAnimation("idle", 0);
    tl.setAttentionStagger({ earMs: 0, gazeMs: 200, headMs: 400 });
    tl.paramsAt(1_000);
    tl.setMicroMotion({ gazeX: 1, gazeY: 0, earBias: 1, intensity: 1 }, 1_000);
    const early = tl.paramsAt(1_100);
    expect(Math.abs(early.earRTilt)).toBeGreaterThan(0.5); // 耳朵已經動了
    expect(early.headTurn).toBeCloseTo(0, 6); // 頭還沒轉
    const late = tl.paramsAt(1_700);
    expect(Math.abs(late.headTurn)).toBeGreaterThan(0); // 最後才轉頭
  });
});
