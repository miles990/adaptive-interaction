// v0.5 rig／表情／Director regression：
// 參數有界、36 表情齊全且非靜態圖、truth-state 不可點播、
// success claimed/verified 誠實區分、Director 冷卻/防重複/搶佔恢復。

import { describe, expect, it } from "vitest";
import {
  clampParams,
  DEFAULT_PARAMS,
  lerpParams,
  RIG_PALETTES,
} from "../companion/rig/params";
import {
  EXPRESSIONS,
  OFFICIAL_36,
  resolveExpression,
  RIG_FALLBACKS,
} from "../companion/rig/expressions";
import { evalPhase, resolveRigAnimation, validateRigManifest } from "../companion/rig/renderer";
import { AMBIENT_VARIANTS, InteractionDirector, REACTION_EXPRESSIONS } from "../companion/director";
import { initialBehavior } from "../companion/behavior";
import { pose } from "../companion/machine";

describe("rig 參數模型", () => {
  it("clampParams：數值進硬界線、未知字串回退預設、NaN 丟棄", () => {
    const p = clampParams({
      bodyBob: 999,
      eyeOpen: -5,
      headTilt: Number.NaN,
      mouth: "evil-grin" as never,
      pose: "sit",
    });
    expect(p.bodyBob).toBeLessThanOrEqual(10);
    expect(p.eyeOpen).toBe(0);
    expect(p.headTilt).toBe(DEFAULT_PARAMS.headTilt);
    expect(p.mouth).toBe(DEFAULT_PARAMS.mouth);
    expect(p.pose).toBe("sit");
  });

  it("lerpParams：中點數值插值、字串在 t>=0.5 切換、輸出仍合法", () => {
    const a = clampParams({ bodyBob: 0, mouth: "soft" });
    const b = clampParams({ bodyBob: 8, mouth: "smile" });
    const mid = lerpParams(a, b, 0.5);
    expect(mid.bodyBob).toBeCloseTo(4);
    expect(mid.mouth).toBe("smile");
    expect(lerpParams(a, b, 0.49).mouth).toBe("soft");
  });
});

describe("36 表情完整性（spec §7.5）", () => {
  it("36 個正式表情全部存在且非靜態圖片（有 enter 或 loop 時間軸）", () => {
    expect(OFFICIAL_36).toHaveLength(36);
    for (const id of OFFICIAL_36) {
      const expr = EXPRESSIONS[id];
      expect(expr, `missing expression: ${id}`).toBeTruthy();
      expect(expr.hold, `${id} needs hold`).toBeTruthy();
      expect(
        Boolean(expr.enter || expr.loop),
        `${id} 必須有 enter 或 loop（不得只是一張靜態圖片）`
      ).toBe(true);
    }
  });

  it("machine pose 會用到的動畫名全部可解析（含別名與 fallback）", () => {
    const names = [
      "idle",
      "listening",
      "thinking",
      "routing",
      "ask",
      "act",
      "waiting",
      "success",
      "blocked",
      "unknown",
      "failed",
      "clicked",
      "dragged",
      "quiet",
      "paused",
      "emergency",
      "offline",
      "notice",
      "curious",
      "stretch",
      "lie",
      "legswing",
      "tailhug",
      "blink",
      "move",
    ];
    for (const name of names) {
      const { expr } = resolveRigAnimation(name);
      expect(expr, `unresolvable: ${name}`).toBeTruthy();
    }
  });

  it("truth-state 表情正確標記，且 fallback 鏈永不落到成功", () => {
    for (const id of [
      "success-claimed",
      "success-verified",
      "blocked",
      "unknown",
      "failed",
      "emergency",
      "offline",
      "paused",
    ]) {
      expect(EXPRESSIONS[id]?.truthState, `${id} 應為 truthState`).toBe(true);
    }
    for (const chain of Object.values(RIG_FALLBACKS)) {
      expect(chain).not.toContain("success-claimed");
      expect(chain.filter((x) => x.startsWith("success-verified"))).toHaveLength(0);
    }
  });
});

describe("誠實映射（claimed ≠ verified）", () => {
  it("success + frameSlice（未驗證）→ 聲稱完成：無綠勾、無慶祝粒子", () => {
    const { id, expr } = resolveRigAnimation("success", [0, 1]);
    expect(id).toBe("success-claimed");
    expect(expr.hold.overlay ?? "none").not.toBe("check");
    expect(expr.hold.particles ?? "none").toBe("none");
    for (const f of expr.enter?.frames ?? []) {
      expect(f.p.overlay ?? "none").not.toBe("check");
      expect(f.p.particles ?? "none").toBe("none");
    }
  });

  it("success 無 slice（已驗證）→ 驗證成功：綠勾在 hold", () => {
    const { id, expr } = resolveRigAnimation("success");
    expect(id).toBe("success-verified");
    expect(expr.hold.overlay).toBe("check");
  });

  it("machine 的 pose 決定 slice：未驗證給 slice、已驗證不給", () => {
    const claimed = pose(
      { base: "idle", transient: { kind: "succeeded", verified: false, untilMs: 10 } },
      0
    );
    expect(claimed.frameSlice).toEqual([0, 1]);
    const verified = pose(
      { base: "idle", transient: { kind: "succeeded", verified: true, untilMs: 10 } },
      0
    );
    expect(verified.frameSlice).toBeUndefined();
  });

  it("emergency 表情凍結（無 loop）且 dim", () => {
    const e = EXPRESSIONS["emergency"];
    expect(e.loop).toBeUndefined();
    expect(e.hold.dim).toBe(1);
  });
});

describe("evalPhase 插值", () => {
  it("keyframe 中點正確插值且 clamp", () => {
    const base = clampParams({});
    const ph = {
      durationMs: 1000,
      frames: [
        { t: 0, p: { bodyBob: 0 } },
        { t: 1, p: { bodyBob: 8 } },
      ],
    };
    expect(evalPhase(base, ph, 0.5).bodyBob).toBeCloseTo(4);
    expect(evalPhase(base, ph, 0).bodyBob).toBe(0);
    expect(evalPhase(base, ph, 1).bodyBob).toBe(8);
  });
});

describe("rig manifest 驗證", () => {
  it("所有 shipped rig packs 通過驗證", () => {
    const files = import.meta.glob("../../public/packs/shu-maid*/manifest.json", {
      eager: true,
    }) as Record<string, Record<string, unknown>>;
    const entries = Object.entries(files);
    expect(entries.length).toBeGreaterThanOrEqual(3);
    for (const [path, manifest] of entries) {
      expect(validateRigManifest(manifest), path).toHaveLength(0);
    }
  });

  it("合法 manifest 通過；壞 kind／palette 拒絕", () => {
    const ok = {
      schemaVersion: "2.0",
      kind: "character-rig",
      id: "shu-maid",
      name: { "zh-TW": "小樞" },
      palette: "maid-classic",
    };
    expect(validateRigManifest(ok)).toHaveLength(0);
    expect(validateRigManifest({ ...ok, kind: "character-pack" })).not.toHaveLength(0);
    expect(validateRigManifest({ ...ok, palette: "evil" })).not.toHaveLength(0);
    expect(Object.keys(RIG_PALETTES)).toContain("maid-classic");
  });
});

describe("Interaction Director", () => {
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
  const always = () => 0; // rng=0：hazard 必觸發、選第一個可用變體

  it("ambient 變體全部非 truth-state；反應表也是", () => {
    for (const v of AMBIENT_VARIANTS) {
      const expr = resolveExpression(v.expression);
      expect(expr, v.expression).toBeTruthy();
      expect(expr!.truthState ?? false, `${v.expression} 不得是 truthState`).toBe(false);
    }
    for (const exprId of Object.values(REACTION_EXPRESSIONS)) {
      expect(resolveExpression(exprId)?.truthState ?? false).toBe(false);
    }
  });

  it("非 ambient／有任務時不排程", () => {
    const d = new InteractionDirector();
    expect(d.tick(ctx({ ambient: false }), always)).toBeNull();
    expect(
      d.tick(ctx({ behavior: { ...initialBehavior(0), taskLoad: 0.5 } }), always)
    ).toBeNull();
  });

  it("quiet 只剩偶爾眨眼", () => {
    const d = new InteractionDirector();
    const a = d.tick(ctx({ quiet: true }), () => 0.001);
    expect(a?.expression).toBe("blink");
    expect(d.tick(ctx({ quiet: true }), () => 0.5)).toBeNull();
  });

  it("Reduced Motion 只允許眨眼類", () => {
    const d = new InteractionDirector();
    for (let i = 0; i < 20; i++) {
      const a = d.tick(ctx({ reducedMotion: true, nowMs: 1_000_000 + i * 60_000 }), always);
      if (a) expect(a.expression).toBe("blink");
    }
  });

  it("防重複＋冷卻：連續排程不會馬上重播同一動作", () => {
    const d = new InteractionDirector();
    const seen: string[] = [];
    for (let i = 0; i < 6; i++) {
      const a = d.tick(ctx({ nowMs: 1_000_000 + i * 1_000 }), always);
      if (a) {
        // 最近 3 個內不得重複。
        expect(seen.slice(-3)).not.toContain(a.expression);
        seen.push(a.expression);
      }
    }
    expect(seen.length).toBeGreaterThan(2);
  });

  it("長動作被搶佔後可恢復；逾時則放棄", () => {
    const d = new InteractionDirector();
    // 用 rng 挑出一個長動作（≥4s）：先冷卻掉前面的短動作。
    let action = null;
    let startedAt = 0;
    for (let i = 0; i < 10 && (!action || action.durationMs < 4000); i++) {
      startedAt = 1_000_000 + i * 1_000;
      action = d.tick(ctx({ nowMs: startedAt }), always);
    }
    expect(action).toBeTruthy();
    expect(action!.durationMs).toBeGreaterThanOrEqual(4000);
    // 播 1 秒後被搶佔。
    d.notePreempted(startedAt + 1_000);
    // 下一個 ambient tick：恢復同一動作（source=resume），rng 再高也要恢復。
    const resumed = d.tick(ctx({ nowMs: startedAt + 3_000 }), () => 0.99);
    expect(resumed?.source).toBe("resume");
    expect(resumed?.expression).toBe(action!.expression);
    // 再次搶佔但等超過 20 秒：已逾時，不恢復（也不再排程新動作）。
    d.notePreempted(startedAt + 3_500);
    const later = d.tick(ctx({ nowMs: startedAt + 30_000 }), () => 0.99);
    expect(later).toBeNull();
  });

  it("react()：未知意圖回 null；已知意圖回非 truth-state 表情", () => {
    const d = new InteractionDirector();
    expect(d.react("celebrate-success", 0)).toBeNull();
    const a = d.react("curious", 0);
    expect(a?.expression).toBe("curious");
  });
});
