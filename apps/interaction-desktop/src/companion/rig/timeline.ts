// ExpressionTimeline：表情四段式時間軸的純狀態機（無 canvas）。
// RigRenderer（單角色）與 StageRenderer（遊玩場）共用。
//
// 誠實映射（success+slice→claimed／無 slice→verified）與 fallback 鏈
// 在 resolveRigAnimation；truth-state 不可點播由 Director/展示白名單把關。

import { MicroMotionOverlay } from "../renderer";
import {
  clamp,
  clampParams,
  DEFAULT_PARAMS,
  lerpParams,
  RigParams,
} from "./params";
import { Expression, ExprPhase, resolveExpression, RIG_FALLBACKS } from "./expressions";

const TRANSITION_MS = 180;

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

export class ExpressionTimeline {
  private exprId = "idle";
  private expr: Expression;
  private switchAt = 0;
  private enterUntil = 0;
  private loopStart = 0;
  private prevSnapshot: RigParams = { ...DEFAULT_PARAMS };
  private lastParams: RigParams = { ...DEFAULT_PARAMS };
  private reducedMotion = false;
  private micro: MicroMotionOverlay = { gazeX: 0, gazeY: 0, earBias: 0, intensity: 0 };
  private nextBlinkAt = 0;
  private blinkStartedAt = -1;
  private rng: () => number;

  constructor(rng?: () => number, startMs = 0) {
    this.rng = rng ?? Math.random;
    this.expr = resolveExpression("idle")!;
    this.loopStart = startMs;
    this.scheduleBlink(startMs);
  }

  setAnimation(name: string, nowMs: number, frameSlice?: [number, number]): void {
    const { id, expr } = resolveRigAnimation(name, frameSlice);
    if (id === this.exprId) return;
    this.prevSnapshot = this.lastParams;
    this.exprId = id;
    this.expr = expr;
    this.switchAt = nowMs;
    this.enterUntil = expr.enter ? nowMs + expr.enter.durationMs : nowMs;
    this.loopStart = this.enterUntil;
  }

  currentExpression(): string {
    return this.exprId;
  }

  currentIsAmbientOverlay(): boolean {
    return this.expr.ambientOverlay === true;
  }

  setReducedMotion(on: boolean): void {
    this.reducedMotion = on;
  }

  isReducedMotion(): boolean {
    return this.reducedMotion;
  }

  setMicroMotion(motion: MicroMotionOverlay): void {
    const c = (v: number) => clamp(Number.isFinite(v) ? v : 0, -1, 1);
    this.micro = {
      gazeX: c(motion.gazeX),
      gazeY: c(motion.gazeY),
      earBias: c(motion.earBias),
      intensity: clamp(Number.isFinite(motion.intensity) ? motion.intensity : 0, 0, 1),
    };
  }

  private scheduleBlink(now: number) {
    this.nextBlinkAt = now + 2200 + this.rng() * 3200;
  }

  paramsAt(now: number): RigParams {
    const base = clampParams({ ...DEFAULT_PARAMS, ...this.expr.hold });
    let target: RigParams;
    if (this.reducedMotion) {
      target = base;
    } else if (this.expr.enter && now < this.enterUntil) {
      const t = 1 - (this.enterUntil - now) / this.expr.enter.durationMs;
      target = evalPhase(base, this.expr.enter, t);
    } else if (this.expr.loop) {
      const dur = this.expr.loop.durationMs;
      const t = ((now - this.loopStart) % dur) / dur;
      target = evalPhase(base, this.expr.loop, t);
    } else {
      target = base;
    }

    const k = clamp((now - this.switchAt) / TRANSITION_MS, 0, 1);
    let params =
      k >= 1 || this.reducedMotion
        ? target
        : lerpParams(this.prevSnapshot, target, easeOutBackLite(k));

    if (!this.reducedMotion && this.expr.ambientOverlay && this.micro.intensity > 0.01) {
      const m = this.micro;
      params = clampParams({
        ...params,
        pupilX: params.pupilX + m.gazeX * 2.2 * m.intensity,
        pupilY: params.pupilY + m.gazeY * 1.4 * m.intensity,
        earLTilt: params.earLTilt - m.earBias * 6 * m.intensity,
        earRTilt: params.earRTilt + m.earBias * 6 * m.intensity,
        headTurn: params.headTurn + m.gazeX * 0.12 * m.intensity,
      });
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
