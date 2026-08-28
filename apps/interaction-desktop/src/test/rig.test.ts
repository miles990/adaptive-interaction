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
import {
  EXIT_MAX_MS,
  ExpressionTimeline,
  resolveSegments,
} from "../companion/rig/timeline";
import { playfieldActive, stageExpressionPlan, statusOverlay } from "../companion/rig/stage";
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

// ---------------------------------------------------------------------------
// v0.5 修復回歸：四段式「離開」段真的會播（曾經是死資料）。
// ---------------------------------------------------------------------------

describe("四段式：enter/hold/loop/exit（spec §6.1/§7）", () => {
  it("OFFICIAL_36 每個表情 resolveSegments 四段皆存在且 durationMs>0", () => {
    for (const id of OFFICIAL_36) {
      const expr = EXPRESSIONS[id];
      const seg = resolveSegments(expr);
      for (const key of ["enter", "loop", "exit"] as const) {
        expect(seg[key], `${id}.${key} 缺段`).toBeTruthy();
        expect(seg[key].durationMs, `${id}.${key} durationMs`).toBeGreaterThan(0);
        expect(seg[key].frames.length, `${id}.${key} frames`).toBeGreaterThan(0);
      }
    }
  });

  it("至少 8 個高頻表情有手寫（非派生）exit", () => {
    const handWritten = [
      "idle",
      "poked",
      "poked-rapid",
      "lifted",
      "wobbly-landing",
      "success-verified",
      "failed",
      "wait-codex",
    ];
    for (const id of handWritten) {
      const seg = resolveSegments(EXPRESSIONS[id]);
      expect(seg.derived.exit, `${id} 應有專屬 exit`).toBe(false);
    }
    expect(handWritten.length).toBeGreaterThanOrEqual(8);
  });

  it("派生段落標記 derived，且 ambient/狀態表情用不同的預設 loop", () => {
    const ambient = resolveSegments(EXPRESSIONS["await-player"]); // ambientOverlay
    expect(ambient.derived.exit).toBe(true);
    const notFound = resolveSegments(EXPRESSIONS["not-found"]);
    expect(notFound.derived.enter).toBe(true);
    // 派生 exit 是 settle 回 DEFAULT 的 follow-through（過衝後回中性）。
    const last = notFound.exit.frames[notFound.exit.frames.length - 1];
    expect(last.p.headNod).toBe(DEFAULT_PARAMS.headNod);
    expect(last.p.earPerk).toBe(DEFAULT_PARAMS.earPerk);
  });

  it("切換表情時真的輸出 exit 段參數（假時鐘逐幀）", () => {
    const tl = new ExpressionTimeline(() => 0.5, 0);
    tl.setAnimation("poked", 0);
    tl.paramsAt(0);
    tl.paramsAt(2_000); // 進入 hold/loop
    tl.setAnimation("idle", 2_000);
    const seg = resolveSegments(EXPRESSIONS["poked"]);
    const dur = Math.min(EXIT_MAX_MS, seg.exit.durationMs);
    expect(tl.isExiting(2_000 + 10)).toBe(true);
    const base = clampParams({ ...DEFAULT_PARAMS, ...EXPRESSIONS["poked"].hold });
    const expected = evalPhase(base, seg.exit, 0.5);
    const got = tl.paramsAt(2_000 + dur / 2);
    expect(got.headTilt).toBeCloseTo(expected.headTilt, 5);
    expect(got.blush).toBeCloseTo(expected.blush, 5);
    // exit 播完才換到新表情。
    tl.paramsAt(2_000 + dur + 1);
    expect(tl.isExiting(2_000 + dur + 1)).toBe(false);
    expect(tl.currentExpression()).toBe("idle");
  });

  it("exit 最長 260ms（再長的離開段也不拖延下一個狀態）", () => {
    const seg = resolveSegments(EXPRESSIONS["wobbly-landing"]);
    expect(seg.exit.durationMs).toBeGreaterThan(EXIT_MAX_MS);
    const tl = new ExpressionTimeline(() => 0.5, 0);
    tl.setAnimation("wobbly-landing", 0);
    tl.paramsAt(1_500);
    tl.setAnimation("idle", 1_500);
    expect(tl.isExiting(1_500 + EXIT_MAX_MS - 5)).toBe(true);
    tl.paramsAt(1_500 + EXIT_MAX_MS + 1);
    expect(tl.isExiting(1_500 + EXIT_MAX_MS + 1)).toBe(false);
  });

  it("truth-state（emergency/blocked/failed/unknown/offline）立即搶佔，不播 exit", () => {
    for (const id of ["emergency", "blocked", "failed", "unknown", "offline"]) {
      const tl = new ExpressionTimeline(() => 0.5, 0);
      tl.setAnimation("poked", 0);
      tl.paramsAt(1_000);
      tl.setAnimation(id, 1_000);
      expect(tl.isExiting(1_000), `${id} 不得等離開動畫`).toBe(false);
      expect(tl.currentExpression()).toBe(id);
    }
  });

  it("Reduced Motion 跳過 exit（直接呈現新表情的 hold）", () => {
    const tl = new ExpressionTimeline(() => 0.5, 0);
    tl.setReducedMotion(true);
    tl.setAnimation("poked", 0);
    tl.paramsAt(1_000);
    tl.setAnimation("idle", 1_000);
    expect(tl.isExiting(1_000)).toBe(false);
    const p = tl.paramsAt(1_010);
    expect(p).toEqual(clampParams({ ...DEFAULT_PARAMS, ...EXPRESSIONS["idle"].hold }));
  });

  it("emergency 凍結：時間過去參數完全不變", () => {
    const tl = new ExpressionTimeline(() => 0.5, 0);
    tl.setAnimation("emergency", 0);
    const a = tl.paramsAt(1_000);
    const b = tl.paramsAt(9_000);
    expect(b).toEqual(a);
    expect(a.dim).toBe(1);
  });
});

// ---------------------------------------------------------------------------
// 組合式通道（spec §6.2）：狀態不再一律整體覆蓋遊玩姿勢。
// ---------------------------------------------------------------------------

describe("組合式角色通道", () => {
  it("mode=stroll ＋ working → 核心亮起，但姿勢仍是遊玩姿勢", () => {
    const plan = stageExpressionPlan("act", "stroll"); // act 是 working 的別名
    expect(plan.expression).toBe("play-chase");
    expect(plan.useMachineSlice).toBe(false);
    expect(plan.statusChannels).toBeTruthy();
    expect(plan.statusChannels!.coreGlow).toBe(EXPRESSIONS["working"].hold.coreGlow);
    expect(plan.statusChannels!.earR).toBe(EXPRESSIONS["working"].hold.earR);

    const tl = new ExpressionTimeline(() => 0.5, 0);
    tl.setAnimation(plan.expression, 0);
    let params = tl.paramsAt(1_000);
    params = clampParams({ ...params, ...plan.statusChannels });
    expect(params.coreGlow).toBe(1); // 核心＝Agent 工作中
    expect(params.armPose).toBe("down"); // 遊玩姿勢保留（working 是 pocket）
  });

  it("等待/工作類狀態只覆蓋狀態通道；安全與結果狀態整體搶佔", () => {
    for (const anim of ["routing", "waiting", "wait-codex", "wait-claude", "ask", "listening"]) {
      expect(statusOverlay(anim), anim).toBe("overlay");
    }
    for (const anim of [
      "emergency",
      "blocked",
      "failed",
      "unknown",
      "offline",
      "paused",
      "success",
      "clicked",
    ]) {
      expect(statusOverlay(anim), anim).toBe("takeover");
    }
    expect(statusOverlay("idle")).toBe("none");
  });

  it("工作/等待狀態不會讓遊玩場停住；安全與結果狀態會", () => {
    // 只借通道的狀態：遊玩場繼續運轉（她可以一邊玩一邊顯示工作中）。
    for (const anim of ["act", "waiting", "wait-codex", "routing"]) {
      expect(playfieldActive(anim, false, false), anim).toBe(true);
    }
    // 安全與結果狀態：遊玩停止（除非本來就是 ambient/遊玩表演）。
    for (const anim of ["emergency", "blocked", "failed", "unknown", "offline", "success"]) {
      expect(playfieldActive(anim, false, false), anim).toBe(false);
    }
    expect(playfieldActive("idle", true, false)).toBe(true);
    expect(playfieldActive("hold-ball", false, true)).toBe(true);
  });

  it("emergency 期間完整覆蓋遊玩：不套遊玩表情、不疊通道", () => {
    const plan = stageExpressionPlan("emergency", "chase");
    expect(plan).toEqual({
      expression: "emergency",
      useMachineSlice: true,
      statusChannels: null,
    });
  });
});

// ---------------------------------------------------------------------------
// v0.5 修復回歸：Director 真的被接上（react / noteFinished / 白名單 / 冷卻）。
// ---------------------------------------------------------------------------

describe("Interaction Director 接線", () => {
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

  it("react()：truth-state 一律回 null（AI/L1 意圖不能點播真相狀態）", () => {
    const d = new InteractionDirector();
    for (const truth of [
      "success-verified",
      "success-claimed",
      "blocked",
      "failed",
      "unknown",
      "emergency",
      "offline",
      "paused",
    ]) {
      expect(d.react(truth, 0), truth).toBeNull();
    }
  });

  it("react()：冷卻內不重播同一個反應", () => {
    const d = new InteractionDirector();
    expect(d.react("poked-rapid", 0)?.expression).toBe("poked-rapid");
    expect(d.react("poked-rapid", 3_000)).toBeNull(); // 8s 冷卻內
    expect(d.react("poked-rapid", 30_000)?.expression).toBe("poked-rapid");
  });

  it("noteFinished()：自然播完後不再排恢復", () => {
    const d = new InteractionDirector();
    let action = null;
    let startedAt = 0;
    for (let i = 0; i < 10 && (!action || action.durationMs < 4_000); i++) {
      startedAt = 1_000_000 + i * 1_000;
      action = d.tick(ctx({ nowMs: startedAt }), () => 0);
    }
    expect(action).toBeTruthy();
    d.noteFinished(); // 表演自然結束
    d.notePreempted(startedAt + 1_000); // 已經沒有進行中的動作
    expect(d.tick(ctx({ nowMs: startedAt + 2_000 }), () => 0.99)).toBeNull();
  });
});
