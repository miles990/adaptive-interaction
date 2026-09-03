// Behavior Runtime 不變量：
// 狀態平滑（無 0→1 跳變）、優先階梯、反重複、任務中不玩鬧、
// Reduced Motion 只留眨眼、勿擾近乎靜止、seeded RNG 可重現、
// 打斷後主動表現收斂。

import { describe, expect, it } from "vitest";
import { InteractionDirector } from "../companion/director";
// CPP：ambient 變體清單屬於角色（shu adapter tables）；排程器本身 engine-neutral。
import { SHU_DIRECTOR_TABLES } from "../character/adapters/shuTables";
import {
  initialBehavior,
  layeredMicroMotion,
  noteEvent,
  noteInterruption,
  scoreEvent,
  seededRng,
  stepBehavior,
} from "../companion/behavior";
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

// ---------------------------------------------------------------------------
// 生命底層（微動作）在執行期就是 InteractionDirector 的 ambient 排程。
// behavior.scheduleMicroAction 曾經是一份同構但**沒有任何生產呼叫端**的排程器，
// 已移除（對抗審查 companion-gameplay-036／rig-renderer-060）；這裡把它原本
// 保證的不變量改釘在真的會跑的那一條路上。
// ---------------------------------------------------------------------------

describe("micro-action scheduling lives in InteractionDirector (no dead scheduler)", () => {
  const director = () => new InteractionDirector(undefined, SHU_DIRECTOR_TABLES);
  const ctx = (over: Partial<Parameters<InteractionDirector["tick"]>[0]> = {}) => ({
    nowMs: 0,
    ambient: true,
    quiet: false,
    reducedMotion: false,
    expressiveness: 1,
    msSinceInteraction: 300_000,
    behavior: calmState(),
    ...over,
  });

  it("behavior.ts 不再匯出死掉的排程器", async () => {
    const mod = (await import("../companion/behavior")) as Record<string, unknown>;
    expect(mod.scheduleMicroAction).toBeUndefined();
  });

  it("never acts outside ambient or under task load", () => {
    const rng = seededRng(1);
    const d = director();
    expect(d.tick(ctx({ ambient: false }), rng)).toBeNull();
    const busy = { ...calmState(), taskLoad: 0.8 };
    for (let i = 0; i < 200; i++) {
      expect(d.tick(ctx({ behavior: busy, nowMs: i * 500 }), rng)).toBeNull();
    }
  });

  it("reduced motion only allows reducedMotionOk variants", () => {
    const rng = seededRng(2);
    const d = director();
    const allowed = new Set(
      SHU_DIRECTOR_TABLES.ambient.filter((v) => v.reducedMotionOk).map((v) => v.expression)
    );
    for (let i = 0; i < 500; i++) {
      const a = d.tick(ctx({ reducedMotion: true, nowMs: i * 500 }), rng);
      if (a) expect(allowed).toContain(a.expression);
    }
  });

  it("interruptions dampen initiative", () => {
    let damped = calmState();
    for (let i = 0; i < 5; i++) damped = noteInterruption(damped);
    // hazard 恰好落在兩者之間：平靜時出手、被打斷五次後不出手（確定性，不靠取樣）。
    const rng = () => 0.05;
    expect(director().tick(ctx({ nowMs: 1_000 }), rng)).not.toBeNull();
    expect(director().tick(ctx({ nowMs: 1_000, behavior: damped }), rng)).toBeNull();
  });

  it("seeded rng makes scheduling reproducible", () => {
    const run = () => {
      const d = director();
      const rng = seededRng(42);
      const out: string[] = [];
      for (let i = 0; i < 500; i++) {
        const a = d.tick(ctx({ nowMs: i * 500 }), rng);
        if (a) out.push(a.expression);
      }
      return out;
    };
    const first = run();
    expect(first.length).toBeGreaterThan(3);
    expect(run()).toEqual(first);
  });
});
