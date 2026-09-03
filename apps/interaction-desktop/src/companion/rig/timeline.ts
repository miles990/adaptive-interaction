// ExpressionTimeline：表情四段式時間軸的純狀態機（無 canvas）。
// RigRenderer（單角色）與 StageRenderer（遊玩場）共用。
//
// 四段式（spec §6.1）：enter（一次）→ hold（基準）＋ loop（小循環）→ exit（離開）。
// 表情資料只手寫「有意義」的段落，缺的段由 resolveSegments() 派生（標記
// derived），所以「離開」段永遠會被播——切換表情時先播上一個表情的 exit
// （上限 EXIT_MAX_MS），再 crossfade 進新表情。
//
// 誠實/安全例外：
//   - 進入 emergency/offline/blocked/failed/unknown 立即搶佔，不播 exit
//     （安全狀態不能等一個離開動畫演完）。
//   - emergency/offline/paused 是凍結狀態：不播 enter/loop/exit、不做
//     crossfade，也不疊微動作——停住的系統不做任何表演。
//   - Reduced Motion：只保留 hold（無 enter/loop/exit）。
//
// 誠實映射（success+slice→claimed／無 slice→verified）與 fallback 鏈
// 在 resolveRigAnimation；truth-state 不可點播由 Director/展示白名單把關。

import { MicroMotionOverlay } from "../renderer";
import {
  blendArm,
  blendPose,
  clamp,
  clampParams,
  DEFAULT_PARAMS,
  lerpParams,
  RigParams,
} from "./params";
import {
  Expression,
  ExprKeyframe,
  ExprPhase,
  resolveExpression,
  RIG_FALLBACKS,
} from "./expressions";

const TRANSITION_MS = 180;
/** 姿勢（lie ↔ stand/sit）過場長度：比一般 crossfade 長一點、且線性。 */
export const POSE_TRANSITION_MS = 260;
/** 離開段最長播放時間：再長的 exit 也不能拖延下一個狀態的呈現。 */
export const EXIT_MAX_MS = 260;
/** 微動作分段（耳→視線→頭）每個通道自己的爬升時間。 */
const MICRO_RAMP_MS = 120;

/** 進入這些表情時立即搶佔：不播上一個表情的 exit。 */
export const PREEMPTING_EXPRESSIONS = new Set([
  "emergency",
  "offline",
  "blocked",
  "failed",
  "unknown",
]);

/** 凍結表情：完全不做動畫（安全/停止狀態）。 */
export const FROZEN_EXPRESSIONS = new Set(["emergency", "offline", "paused"]);

/** 輕微回彈的 ease（overshoot ~4%）：便宜的 game feel。 */
export function easeOutBackLite(t: number): number {
  const c = 1.2;
  const x = t - 1;
  return 1 + (c + 1) * x * x * x + c * x * x;
}

/** 在 phase 時間軸上取參數（keyframe 線性插值；base 為 hold 全參數）。 */
export function evalPhase(base: RigParams, ph: ExprPhase, t01: number): RigParams {
  const frames = ph.frames;
  if (frames.length === 0) return base;
  const t = clamp(t01, 0, 1);
  let prev = frames[0];
  let next = frames[frames.length - 1];
  for (let i = 0; i < frames.length; i++) {
    if (frames[i].t <= t) prev = frames[i];
    if (frames[i].t >= t) {
      next = frames[i];
      break;
    }
  }
  const a = clampParams({ ...base, ...prev.p });
  if (prev === next) return a;
  const b = clampParams({ ...base, ...next.p });
  const span = next.t - prev.t;
  const k = span <= 0 ? 1 : (t - prev.t) / span;
  return lerpParams(a, b, k);
}

// ---------------------------------------------------------------------------
// 段落派生（resolveSegments）：未手寫的段落用有意義的預設補齊。
// ---------------------------------------------------------------------------

const kf = (t: number, p: Partial<RigParams>): ExprKeyframe => ({ t, p });
const phase = (durationMs: number, ...frames: ExprKeyframe[]): ExprPhase => ({
  durationMs,
  frames,
});

/** 派生 enter：120ms 輕微 anticipation（先壓一下再彈回）。 */
const DERIVED_ENTER: ExprPhase = phase(
  120,
  kf(0, { squash: -0.06 }),
  kf(0.55, { squash: -0.02 }),
  kf(1, { squash: 0 })
);

/** 派生 loop（ambient）：BREATHE ＋ TAIL_SWAY 合成的生活感小循環。 */
const DERIVED_AMBIENT_LOOP: ExprPhase = phase(
  3400,
  kf(0, { bodyBob: 0, tailSway: -0.35, corePulse: 0 }),
  kf(0.5, { bodyBob: -1.5, tailSway: 0.35, corePulse: 0.5 }),
  kf(1, { bodyBob: 0, tailSway: -0.35, corePulse: 1 })
);

/** 派生 loop（結果/工作狀態）：只有低幅核心呼吸——不做任何肢體表演。 */
const DERIVED_STATUS_LOOP: ExprPhase = phase(
  2600,
  kf(0, { corePulse: 0.15 }),
  kf(0.5, { corePulse: 0.35 }),
  kf(1, { corePulse: 0.15 })
);

/** 派生 exit：140ms settle 回 DEFAULT，headNod/earPerk 過衝 4% 再回。 */
const DERIVED_EXIT: ExprPhase = phase(
  140,
  kf(0, {}),
  kf(0.55, {
    headNod: DEFAULT_PARAMS.headNod + 0.04,
    earPerk: DEFAULT_PARAMS.earPerk + 0.04,
  }),
  kf(1, { headNod: DEFAULT_PARAMS.headNod, earPerk: DEFAULT_PARAMS.earPerk })
);

export interface ResolvedSegments {
  enter: ExprPhase;
  loop: ExprPhase;
  exit: ExprPhase;
  /** 哪些段落是派生的（不是表情自己手寫的）。 */
  derived: { enter: boolean; loop: boolean; exit: boolean };
}

/** 表情 id → 已解析段落（表情表是靜態的，快取避免每幀配置）。 */
const SEGMENT_CACHE = new Map<string, ResolvedSegments>();

/**
 * 補齊四段式。未手寫的段落用有意義的預設派生：
 *   enter：輕微 anticipation。
 *   loop：ambient 表情用呼吸＋尾巴擺；其餘（工作/結果狀態）用低幅核心呼吸。
 *   exit：settle 回 DEFAULT 的 follow-through。
 * 純函式：同一個表情永遠得到同一份段落。
 */
export function resolveSegments(expr: Expression): ResolvedSegments {
  const cached = SEGMENT_CACHE.get(expr.id);
  if (cached) return cached;
  const resolved: ResolvedSegments = {
    enter: expr.enter ?? DERIVED_ENTER,
    loop: expr.loop ?? (expr.ambientOverlay ? DERIVED_AMBIENT_LOOP : DERIVED_STATUS_LOOP),
    exit: expr.exit ?? DERIVED_EXIT,
    derived: {
      enter: expr.enter === undefined,
      loop: expr.loop === undefined,
      exit: expr.exit === undefined,
    },
  };
  SEGMENT_CACHE.set(expr.id, resolved);
  return resolved;
}

/** 解析動畫名（誠實 success 區分＋fallback 鏈，永不落到成功）。 */
export function resolveRigAnimation(
  name: string,
  frameSlice?: [number, number]
): { id: string; expr: Expression } {
  let wanted = name;
  if (name === "success") {
    wanted = frameSlice ? "success-claimed" : "success-verified";
  }
  const direct = resolveExpression(wanted);
  if (direct) return { id: direct.id, expr: direct };
  const chain = RIG_FALLBACKS[wanted] ?? RIG_FALLBACKS.default;
  for (const alt of chain) {
    const e = resolveExpression(alt);
    if (e) return { id: e.id, expr: e };
  }
  const idle = resolveExpression("idle")!;
  return { id: "idle", expr: idle };
}

/** 微動作分段延遲（ms）：耳朵先動、視線其次、頭最後（spec §4.3 聰明）。 */
export interface AttentionStagger {
  earMs: number;
  gazeMs: number;
  headMs: number;
}

export const DEFAULT_ATTENTION_STAGGER: AttentionStagger = {
  earMs: 0,
  gazeMs: 60,
  headMs: 160,
};

const ZERO_MICRO: MicroMotionOverlay = { gazeX: 0, gazeY: 0, earBias: 0, intensity: 0 };

interface ExitPlayback {
  phase: ExprPhase;
  base: RigParams;
  startAt: number;
  until: number;
}

export class ExpressionTimeline {
  private exprId = "idle";
  private expr: Expression;
  private segments: ResolvedSegments;
  private switchAt = 0;
  private enterUntil = 0;
  private loopStart = 0;
  private prevSnapshot: RigParams = { ...DEFAULT_PARAMS };
  private lastParams: RigParams = { ...DEFAULT_PARAMS };
  private reducedMotion = false;
  private micro: MicroMotionOverlay = { ...ZERO_MICRO };
  private prevMicro: MicroMotionOverlay = { ...ZERO_MICRO };
  private microAt = 0;
  private stagger: AttentionStagger = { ...DEFAULT_ATTENTION_STAGGER };
  private exiting: ExitPlayback | null = null;
  private lastNow = 0;
  private nextBlinkAt = 0;
  private blinkStartedAt = -1;
  private rng: () => number;

  constructor(rng?: () => number, startMs = 0) {
    this.rng = rng ?? Math.random;
    this.expr = resolveExpression("idle")!;
    this.segments = resolveSegments(this.expr);
    this.loopStart = startMs;
    this.lastNow = startMs;
    this.microAt = startMs;
    this.scheduleBlink(startMs);
  }

  setAnimation(name: string, nowMs: number, frameSlice?: [number, number]): void {
    const { id, expr } = resolveRigAnimation(name, frameSlice);
    if (id === this.exprId) return;
    this.lastNow = nowMs;

    // 目前正在播的表情（已在播 exit 就不再疊第二段離開）。
    const outgoing = this.exiting ? null : this.expr;
    const preempt = PREEMPTING_EXPRESSIONS.has(id) || FROZEN_EXPRESSIONS.has(id);
    if (preempt) {
      this.exiting = null;
    } else if (outgoing && !this.reducedMotion && !FROZEN_EXPRESSIONS.has(this.exprId)) {
      const seg = resolveSegments(outgoing);
      const durationMs = Math.min(EXIT_MAX_MS, seg.exit.durationMs);
      if (durationMs > 0) {
        // 離開段從「此刻實際輸出的參數」起算，不是從 hold：表情在 enter／loop 中途被
        // 打斷（伸懶腰到一半被戳）時，手臂／squash／眼睛不會先瞬移回 hold 再演離開
        // （對抗審查 rig-renderer-013）。exit 的關鍵幀只覆寫它有寫的通道，其餘沿用當下值。
        this.exiting = {
          phase: seg.exit,
          base: clampParams({ ...this.lastParams }),
          startAt: nowMs,
          until: nowMs + durationMs,
        };
      }
    }

    this.prevSnapshot = this.lastParams;
    this.exprId = id;
    this.expr = expr;
    this.segments = resolveSegments(expr);
    // exit 播完才起算新表情的 enter/loop（實際時間在 paramsAt 收尾時定案）。
    const startAt = this.exiting ? this.exiting.until : nowMs;
    this.switchAt = startAt;
    this.enterUntil = startAt + this.segments.enter.durationMs;
    this.loopStart = this.enterUntil;
  }

  currentExpression(): string {
    return this.exprId;
  }

  /** 是否正在播上一個表情的離開段。 */
  isExiting(nowMs = this.lastNow): boolean {
    return this.exiting !== null && nowMs < this.exiting.until;
  }

  currentIsAmbientOverlay(): boolean {
    return this.expr.ambientOverlay === true;
  }

  setReducedMotion(on: boolean): void {
    this.reducedMotion = on;
    if (on) this.exiting = null; // Reduced Motion 不播離開段
  }

  isReducedMotion(): boolean {
    return this.reducedMotion;
  }

  /** 注意力分段（耳→視線→頭）；由 personality tuning 提供。 */
  setAttentionStagger(stagger: AttentionStagger): void {
    const s = (v: number) => clamp(Number.isFinite(v) ? v : 0, 0, 2_000);
    this.stagger = { earMs: s(stagger.earMs), gazeMs: s(stagger.gazeMs), headMs: s(stagger.headMs) };
  }

  setMicroMotion(motion: MicroMotionOverlay, nowMs = this.lastNow): void {
    const c = (v: number) => clamp(Number.isFinite(v) ? v : 0, -1, 1);
    const next: MicroMotionOverlay = {
      gazeX: c(motion.gazeX),
      gazeY: c(motion.gazeY),
      earBias: c(motion.earBias),
      intensity: clamp(Number.isFinite(motion.intensity) ? motion.intensity : 0, 0, 1),
    };
    // 分段起點：從「目前已生效的疊加值」出發，避免跳變。
    this.prevMicro = this.effectiveMicro(nowMs);
    this.micro = next;
    this.microAt = nowMs;
  }

  /** 依每個通道自己的延遲，取得此刻實際生效的微動作疊加量。 */
  private effectiveMicro(now: number): MicroMotionOverlay {
    const w = (delayMs: number) =>
      clamp((now - this.microAt - delayMs) / MICRO_RAMP_MS, 0, 1);
    const mix = (a: number, b: number, k: number) => a + (b - a) * k;
    const earW = w(this.stagger.earMs);
    const gazeW = w(this.stagger.gazeMs);
    return {
      earBias: mix(this.prevMicro.earBias, this.micro.earBias, earW),
      gazeX: mix(this.prevMicro.gazeX, this.micro.gazeX, gazeW),
      gazeY: mix(this.prevMicro.gazeY, this.micro.gazeY, gazeW),
      // 強度跟著最早的通道（耳朵）：注意力一開始就成立，
      // 只是視線與頭部還沒跟上。
      intensity: mix(this.prevMicro.intensity, this.micro.intensity, earW),
    };
  }

  /** 頭部跟隨視線的延遲量（最後才轉頭）。 */
  private headFollow(now: number): number {
    const k = clamp((now - this.microAt - this.stagger.headMs) / MICRO_RAMP_MS, 0, 1);
    return this.prevMicro.gazeX + (this.micro.gazeX - this.prevMicro.gazeX) * k;
  }

  private scheduleBlink(now: number) {
    this.nextBlinkAt = now + 2200 + this.rng() * 3200;
  }

  /**
   * 立刻眨一下眼，**不換表情**。
   *
   * 安靜時 Director 只允許眨眼類；如果把它當成一般表演套上去，她會從「安靜
   * 陪伴」的坐姿彈回中性站姿 0.4 秒——比原本的缺陷更糟。會自動眨眼的表情
   * 才收這個提示（其餘表情本來就沒有眨眼通道）。
   */
  blinkNow(nowMs: number): boolean {
    if (!this.expr.autoBlink) return false;
    this.blinkStartedAt = nowMs;
    this.scheduleBlink(nowMs);
    return true;
  }

  paramsAt(now: number): RigParams {
    this.lastNow = now;
    let base = clampParams({ ...DEFAULT_PARAMS, ...this.expr.hold });
    // Reduced Motion：hold 內的慶祝/情緒粒子（praised 的愛心、
    // success-verified 的火花）也是持續動畫——強度歸零。狀態辨識保留：
    // overlay（綠勾/✕/盾）是靜態符號，照樣顯示。
    if (this.reducedMotion) {
      base = clampParams({ ...base, particles: "none", particlePhase: 0 });
    }

    // ---- 離開段：上一個表情先演完，再 crossfade 進新表情 ----
    if (this.exiting) {
      if (!this.reducedMotion && now < this.exiting.until) {
        const e = this.exiting;
        const span = Math.max(1, e.until - e.startAt);
        const params = evalPhase(e.base, e.phase, (now - e.startAt) / span);
        this.lastParams = params;
        return params;
      }
      // exit 結束：從離開姿勢起算新表情的時間軸。
      const startAt = this.reducedMotion ? now : this.exiting.until;
      this.prevSnapshot = this.lastParams;
      this.switchAt = startAt;
      this.enterUntil = startAt + this.segments.enter.durationMs;
      this.loopStart = this.enterUntil;
      this.exiting = null;
    }

    // ---- 凍結狀態：不做任何演出（含 crossfade 與微動作） ----
    if (FROZEN_EXPRESSIONS.has(this.exprId)) {
      this.lastParams = base;
      return base;
    }

    let target: RigParams;
    if (this.reducedMotion) {
      target = base;
    } else if (now < this.enterUntil) {
      const dur = Math.max(1, this.segments.enter.durationMs);
      const t = 1 - (this.enterUntil - now) / dur;
      target = evalPhase(base, this.segments.enter, t);
    } else {
      const dur = Math.max(1, this.segments.loop.durationMs);
      const t = (((now - this.loopStart) % dur) + dur) % dur;
      target = evalPhase(base, this.segments.loop, t / dur);
    }

    const k = clamp((now - this.switchAt) / TRANSITION_MS, 0, 1);
    let params =
      k >= 1 || this.reducedMotion
        ? target
        : lerpParams(this.prevSnapshot, target, easeOutBackLite(k));

    // 姿勢過場不跟隨回彈 ease：ease 在第一幀就前進三成多，頭部照樣會跳
    // ~16px。用自己的線性進度，pose 的切換點與 poseBlend 永遠一致。
    // 只在過場窗口內覆寫：窗口過了就交還給 enter 段自己算的 poseBlend——
    // lie-flat 的 enter 在 450ms 才由 crouch 換成 lie，若此時仍硬寫 poseBlend=1，
    // 頭中心會單幀跳 21px（對抗審查 rig-renderer-012）。
    if (!this.reducedMotion) {
      const kp = clamp((now - this.switchAt) / POSE_TRANSITION_MS, 0, 1);
      if (kp < 1) params = blendPose(this.prevSnapshot, target, params, kp);
      // 手臂姿勢同理（線性、跟 crossfade 等長）：raise→front 之類的切換不再單幀跳手位。
      const ka = clamp((now - this.switchAt) / TRANSITION_MS, 0, 1);
      if (ka < 1) params = blendArm(this.prevSnapshot, target, params, ka);
    }

    if (!this.reducedMotion && this.expr.ambientOverlay) {
      const m = this.effectiveMicro(now);
      if (m.intensity > 0.01) {
        // 耳朵先動、視線其次、頭最後（每個通道有自己的延遲）。
        const head = this.headFollow(now);
        params = clampParams({
          ...params,
          pupilX: params.pupilX + m.gazeX * 2.2 * m.intensity,
          pupilY: params.pupilY + m.gazeY * 1.4 * m.intensity,
          earLTilt: params.earLTilt - m.earBias * 6 * m.intensity,
          earRTilt: params.earRTilt + m.earBias * 6 * m.intensity,
          headTurn: params.headTurn + head * 0.12 * m.intensity,
        });
      }
    }

    if (this.expr.autoBlink) {
      if (now >= this.nextBlinkAt) {
        this.blinkStartedAt = now;
        this.scheduleBlink(now);
      }
      if (this.blinkStartedAt >= 0) {
        const bt = (now - this.blinkStartedAt) / 150;
        if (bt < 1) {
          const dip = bt < 0.5 ? bt * 2 : (1 - bt) * 2;
          params = clampParams({ ...params, eyeOpen: params.eyeOpen * (1 - dip * 0.95) });
        } else {
          this.blinkStartedAt = -1;
        }
      }
    }
    this.lastParams = params;
    return params;
  }
}
