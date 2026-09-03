// Behavior Runtime 不變量：
// 狀態平滑（無 0→1 跳變）、優先階梯、反重複、任務中不玩鬧、
// Reduced Motion 只留眨眼、勿擾近乎靜止、seeded RNG 可重現、
// 打斷後主動表現收斂。

import { describe, expect, it } from "vitest";
import {
  initialBehavior,
  layeredMicroMotion,
  noteEvent,
  noteInterruption,
  scheduleMicroAction,
  scoreEvent,
  seededRng,
  stepBehavior,
} from "../companion/behavior";
// CPP：微動作清單屬於角色（shu adapter tables）；排程器本身 engine-neutral。
import { SHU_MICRO_ACTIONS } from "../character/adapters/shuTables";

const baseCtx = {
  ambient: true,
  reducedMotion: false,
  quiet: false,
  expressiveness: 1,
  msSinceInteraction: 300_000,
  recent: [] as string[],
};

const calmState = () => {
  let s = initialBehavior(0);
  // 收斂到低喚起（長時間無事）。
  for (let i = 0; i < 40; i++) {
    s = stepBehavior(s, { busy: false, waitingForHuman: false, msSinceInteraction: 300_000 });
  }
  return s;
};

describe("behavior state smoothing", () => {
  it("single events never jump activation to 1", () => {
    const s = initialBehavior(0);
    const after = noteEvent(s, "action.completed", 0.5);
    expect(after.activation).toBeLessThan(0.5);
    expect(after.activation).toBeGreaterThan(s.activation);
  });

  it("busy raises taskLoad gradually, idle decays it", () => {
    let s = initialBehavior(0);
    const one = stepBehavior(s, { busy: true, waitingForHuman: false, msSinceInteraction: 0 });
    expect(one.taskLoad).toBeGreaterThan(0);
    expect(one.taskLoad).toBeLessThan(0.5); // 一步不到位（平滑）
    for (let i = 0; i < 30; i++) {
      s = stepBehavior(s, { busy: true, waitingForHuman: false, msSinceInteraction: 0 });
    }
    expect(s.taskLoad).toBeGreaterThan(0.9);
  });
});

describe("layered procedural micro motion", () => {
  it("is deterministic, bounded, and freezes for reduced motion or safety", () => {
    const s = { ...calmState(), activation: 0.6, attention: 0.4 };
    const a = layeredMicroMotion(s, 12_345, false, false);
    expect(a).toEqual(layeredMicroMotion(s, 12_345, false, false));
    expect(Math.abs(a.gazeX)).toBeLessThanOrEqual(1);
    expect(Math.abs(a.gazeY)).toBeLessThanOrEqual(1);
    expect(Math.abs(a.earBias)).toBeLessThanOrEqual(1);
    expect(a.intensity).toBeGreaterThan(0);
    expect(layeredMicroMotion(s, 12_345, true, false)).toEqual({
      gazeX: 0,
      gazeY: 0,
      earBias: 0,
      intensity: 0,
    });
    expect(layeredMicroMotion(s, 12_345, false, true).intensity).toBe(0);
  });

  it("changes continuously without using pointer coordinates", () => {
    const s = calmState();
    const values = [0, 250, 500, 750, 1000].map((t) => layeredMicroMotion(s, t, false, false));
    expect(new Set(values.map((v) => v.gazeX.toFixed(4))).size).toBeGreaterThan(3);
    for (let i = 1; i < values.length; i++) {
      expect(Math.abs(values[i].gazeX - values[i - 1].gazeX)).toBeLessThan(0.3);
    }
  });
});

describe("event priority ladder", () => {
  const ctx = {
    recentSameClass: 0,
    alreadyResponded: false,
    interruptible: true,
    doNotDisturb: false,
    relevance: 0.5,
    novelty: 0.5,
  };
  it("orders emergency > sensor > waiting > interaction > task > suggestion > world > ambient", () => {
    const order = [
      "emergency",
      "sensor-safety",
      "waiting-confirmation",
      "direct-interaction",
      "task-state",
      "suggestion",
      "world-event",
      "ambient",
    ] as const;
    const scores = order.map((c) => scoreEvent(c, ctx));
    for (let i = 1; i < scores.length; i++) {
      expect(scores[i - 1]).toBeGreaterThan(scores[i]);
    }
  });

  it("do-not-disturb suppresses suggestions but never zeroes safety", () => {
    const dnd = { ...ctx, doNotDisturb: true, recentSameClass: 3, alreadyResponded: true };
    expect(scoreEvent("suggestion", dnd)).toBeLessThan(0);
    expect(scoreEvent("sensor-safety", dnd)).toBeGreaterThan(0);
    expect(scoreEvent("emergency", dnd)).toBeGreaterThan(0);
  });
});

describe("micro-action scheduler", () => {
  it("never acts outside ambient or under task load", () => {
    const rng = seededRng(1);
    const s = calmState();
    expect(scheduleMicroAction(s, { ...baseCtx, ambient: false }, rng, SHU_MICRO_ACTIONS)).toBeNull();
    const busy = { ...s, taskLoad: 0.8 };
    for (let i = 0; i < 200; i++) {
      expect(scheduleMicroAction(busy, baseCtx, rng, SHU_MICRO_ACTIONS)).toBeNull();
    }
  });

  it("reduced motion only allows blink-class actions", () => {
    const rng = seededRng(2);
    const s = calmState();
    for (let i = 0; i < 500; i++) {
      const a = scheduleMicroAction(s, { ...baseCtx, reducedMotion: true }, rng, SHU_MICRO_ACTIONS);
      if (a) expect(a.reducedMotionOk).toBe(true);
    }
  });

  it("avoids repeating the last two actions and is not fixed-period", () => {
    const rng = seededRng(3);
    const s = calmState();
    const gaps: number[] = [];
    let last = 0;
    const played: string[] = [];
    for (let i = 0; i < 3000; i++) {
      const a = scheduleMicroAction(s, { ...baseCtx, recent: played.slice(-3) }, rng, SHU_MICRO_ACTIONS);
      if (a) {
        expect(played.slice(-2)).not.toContain(a.id);
        played.push(a.id);
        gaps.push(i - last);
        last = i;
      }
    }
    expect(played.length).toBeGreaterThan(5);
    // 間隔非固定：至少出現三種不同間隔。
    expect(new Set(gaps).size).toBeGreaterThan(2);
  });

  it("lazy poses need real relaxation (recent interaction blocks lie-down)", () => {
    const rng = seededRng(4);
    const s = calmState();
    for (let i = 0; i < 1000; i++) {
      const a = scheduleMicroAction(s, { ...baseCtx, msSinceInteraction: 5_000 }, rng, SHU_MICRO_ACTIONS);
      if (a) expect(["lie-down", "tail-hug"]).not.toContain(a.id);
    }
  });

  it("interruptions dampen initiative", () => {
    let s = calmState();
    for (let i = 0; i < 5; i++) s = noteInterruption(s);
    const rng1 = seededRng(5);
    const rng2 = seededRng(5);
    let calmCount = 0;
    let dampedCount = 0;
    const calm = calmState();
    for (let i = 0; i < 2000; i++) {
      if (scheduleMicroAction(calm, baseCtx, rng1, SHU_MICRO_ACTIONS)) calmCount++;
      if (scheduleMicroAction(s, baseCtx, rng2, SHU_MICRO_ACTIONS)) dampedCount++;
    }
    expect(dampedCount).toBeLessThan(calmCount);
  });

  it("seeded rng makes scheduling reproducible", () => {
    const s = calmState();
    const run = () => {
      const rng = seededRng(42);
      const out: string[] = [];
      const recent: string[] = [];
      for (let i = 0; i < 500; i++) {
        const a = scheduleMicroAction(s, { ...baseCtx, recent: recent.slice(-3) }, rng, SHU_MICRO_ACTIONS);
        if (a) {
          out.push(a.id);
          recent.push(a.id);
        }
      }
      return out;
    };
    expect(run()).toEqual(run());
  });

  it("every micro action uses only non-truth art", () => {
    for (const a of SHU_MICRO_ACTIONS) {
      expect(["success", "blocked", "unknown", "failed", "emergency", "offline"]).not.toContain(
        a.animation
      );
    }
  });
});
