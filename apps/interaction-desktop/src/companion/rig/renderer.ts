// RigRenderer：執行期參數化渲染後端（RendererBackend 實作，單角色）。
// 表情時間軸邏輯在 timeline.ts（與 StageRenderer 共用）。

import { RendererBackend, MicroMotionOverlay } from "../renderer";
import { drawRig } from "./draw";
import { clampParams, DEFAULT_PARAMS, RIG_PALETTES } from "./params";
import { ExpressionTimeline } from "./timeline";

export { evalPhase, resolveRigAnimation } from "./timeline";

export class RigRenderer implements RendererBackend {
  private ctx: CanvasRenderingContext2D;
  private paletteName: string;
  private scale: number;
  private raf = 0;
  private timeline: ExpressionTimeline;
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
    this.now = opts?.now ?? (() => performance.now());
    this.timeline = new ExpressionTimeline(opts?.rng, this.now());
    if (opts?.autoStart !== false) {
      this.loop();
    }
  }

  setAnimation(name: string, frameSlice?: [number, number]): void {
    this.timeline.setAnimation(name, this.now(), frameSlice);
  }

  /** 目前表情 id（測試/預覽用）。 */
  currentExpression(): string {
    return this.timeline.currentExpression();
  }

  setReducedMotion(on: boolean): void {
    this.timeline.setReducedMotion(on);
  }

  setMicroMotion(motion: MicroMotionOverlay): void {
    this.timeline.setMicroMotion(motion, this.now());
  }

  destroy(): void {
    this.destroyed = true;
    cancelAnimationFrame(this.raf);
  }

  /** 計算當下應繪參數（測試用）。 */
  paramsAt(now: number) {
    return this.timeline.paramsAt(now);
  }

  /** 立即渲染一幀（測試/截圖用亦可直接呼叫）。 */
  renderFrame(now = this.now()): void {
    const params = this.timeline.paramsAt(now);
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
  const { expr } = resolveAnim(exprName);
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

import { resolveRigAnimation as resolveAnim } from "./timeline";
