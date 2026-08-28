// RigRenderer：執行期參數化渲染後端（RendererBackend 實作）。
//
// - 表情四段式：enter（一次）→ hold＋loop（反覆）；切換表情時從目前
//   參數快照 crossfade（~180ms、輕微 overshoot 的 game feel）。
// - 誠實映射：machine 的 success + frameSlice（未驗證）→ success-claimed
//   （只點頭）；無 slice（已驗證）→ success-verified（綠勾）。
// - Reduced Motion：靜態 hold 姿勢，狀態仍可辨識，不播 enter/loop。
// - 微動作（gaze/ear）只疊加在 ambientOverlay 允許的表情上，且有界。

import { RendererBackend, MicroMotionOverlay } from "../renderer";
import { drawRig } from "./draw";
import {
  clamp,
  clampParams,
  DEFAULT_PARAMS,
  lerpParams,
  RIG_PALETTES,
  RigParams,
} from "./params";
import {
  Expression,
  ExprPhase,
  resolveExpression,
  RIG_FALLBACKS,
} from "./expressions";

const TRANSITION_MS = 180;

/** 輕微回彈的 ease（overshoot ~4%）：便宜的 game feel。 */
function easeOutBackLite(t: number): number {
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

export class RigRenderer implements RendererBackend {
  private ctx: CanvasRenderingContext2D;
  private paletteName: string;
  private scale: number;
  private raf = 0;
  private reducedMotion = false;
  private micro: MicroMotionOverlay = { gazeX: 0, gazeY: 0, earBias: 0, intensity: 0 };

  private exprId = "idle";
  private expr: Expression;
  private switchAt = 0;
  private enterUntil = 0;
  private loopStart = 0;
  private prevSnapshot: RigParams = { ...DEFAULT_PARAMS };
  private lastParams: RigParams = { ...DEFAULT_PARAMS };

  private nextBlinkAt = 0;
  private blinkStartedAt = -1;
  private rng: () => number;
  private now: () => number;
  private destroyed = false;

  constructor(
    private canvas: HTMLCanvasElement,
    paletteName: string,
    scale = 1,
    opts?: { rng?: () => number; now?: () => number; autoStart?: boolean }
  ) {
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("no 2d context");
    this.ctx = ctx;
    this.paletteName = RIG_PALETTES[paletteName] ? paletteName : "maid-classic";
    this.scale = scale;
    this.rng = opts?.rng ?? Math.random;
    this.now = opts?.now ?? (() => performance.now());
    const start = this.now();
    this.expr = resolveExpression("idle")!;
    this.loopStart = start;
    this.scheduleBlink(start);
    if (opts?.autoStart !== false) {
      this.loop();
    }
  }

  setAnimation(name: string, frameSlice?: [number, number]): void {
    const { id, expr } = resolveRigAnimation(name, frameSlice);
    if (id === this.exprId) return;
    const now = this.now();
    // 快照目前參數作 crossfade 起點。
    this.prevSnapshot = this.lastParams;
    this.exprId = id;
    this.expr = expr;
    this.switchAt = now;
    this.enterUntil = expr.enter ? now + expr.enter.durationMs : now;
    this.loopStart = this.enterUntil;
  }

  /** 目前表情 id（測試/預覽用）。 */
  currentExpression(): string {
    return this.exprId;
  }

  setReducedMotion(on: boolean): void {
    this.reducedMotion = on;
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

  destroy(): void {
    this.destroyed = true;
    cancelAnimationFrame(this.raf);
  }

  private scheduleBlink(now: number) {
    this.nextBlinkAt = now + 2200 + this.rng() * 3200;
  }

  /** 計算當下應繪參數（pure-ish：只讀內部狀態）。 */
  paramsAt(now: number): RigParams {
    const base = clampParams({ ...DEFAULT_PARAMS, ...this.expr.hold });
    let target: RigParams;
    if (this.reducedMotion) {
      // 靜態姿勢：保留狀態辨識（overlay/裙光等 hold 內容），不播動作。
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

    // 表情切換 crossfade（+輕微回彈）。
    const k = clamp((now - this.switchAt) / TRANSITION_MS, 0, 1);
    let params =
      k >= 1 || this.reducedMotion ? target : lerpParams(this.prevSnapshot, target, easeOutBackLite(k));

    // 微動作疊加（只在允許的表情、非 reduced motion）。
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

    // 自動眨眼（ambient 表情；Reduced Motion 也保留——它是唯一允許的動作）。
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
    return params;
  }

  /** 立即渲染一幀（測試/截圖用亦可直接呼叫）。 */
  renderFrame(now = this.now()): void {
    const params = this.paramsAt(now);
    this.lastParams = params;
    const dpr = window.devicePixelRatio || 1;
    const w = 128 * this.scale;
    const h = 128 * this.scale;
    if (this.canvas.width !== w * dpr || this.canvas.height !== h * dpr) {
      this.canvas.width = w * dpr;
      this.canvas.height = h * dpr;
      this.canvas.style.width = `${w}px`;
      this.canvas.style.height = `${h}px`;
    }
    const ctx = this.ctx;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);
    ctx.scale(this.scale * dpr, this.scale * dpr);
    drawRig(ctx, params, RIG_PALETTES[this.paletteName]);
    ctx.setTransform(1, 0, 0, 1, 0, 0);
  }

  private loop = () => {
    if (this.destroyed) return;
    this.raf = requestAnimationFrame(this.loop);
    this.renderFrame();
  };
}

/** rig pack manifest（kind=character-rig）驗證。 */
export interface RigManifest {
  schemaVersion: string;
  kind: string;
  id: string;
  name: Record<string, string>;
  palette: string;
  version?: string;
  author?: string;
  license?: string;
  generator?: string;
  description?: Record<string, string>;
}

export function validateRigManifest(m: unknown): string[] {
  const issues: string[] = [];
  const man = m as Partial<RigManifest>;
  if (!man || typeof man !== "object") return ["manifest is not an object"];
  if (man.kind !== "character-rig") issues.push("kind must be character-rig");
  if (!man.schemaVersion) issues.push("schemaVersion missing");
  if (!man.id || !/^[a-z0-9][a-z0-9-]{0,63}$/.test(man.id)) issues.push("invalid id");
  if (typeof man.palette !== "string" || !RIG_PALETTES[man.palette])
    issues.push(`palette must be one of: ${Object.keys(RIG_PALETTES).join(", ")}`);
  if (!man.name || typeof man.name !== "object") issues.push("name missing");
  return issues;
}

/** 靜態預覽：把某個表情的 hold 姿勢畫到 canvas（控制中心預覽格）。 */
export function drawExpressionPreview(
  ctx: CanvasRenderingContext2D,
  exprName: string,
  paletteName: string,
  size: number
): boolean {
  const { expr } = resolveRigAnimation(exprName);
  const params = clampParams({
    ...DEFAULT_PARAMS,
    ...expr.hold,
    overlayPhase: 0.5,
    particlePhase: 0.3,
    corePulse: 0.25,
  });
  ctx.save();
  ctx.clearRect(0, 0, size, size);
  ctx.scale(size / 128, size / 128);
  drawRig(ctx, params, RIG_PALETTES[paletteName] ?? RIG_PALETTES["maid-classic"]);
  ctx.restore();
  return true;
}
