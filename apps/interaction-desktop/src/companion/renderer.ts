// Canvas sprite renderer for character packs.
//
// Renderer interface kept minimal so other backends (Rive, …) can implement
// it later; this v1 backend plays sprite-sheet animations deterministically.
// Missing animations fall back safely (never crash, never fake success).

export interface PackManifest {
  schemaVersion: string;
  kind: string;
  id: string;
  name: Record<string, string>;
  description?: Record<string, string>;
  author?: string;
  version?: string;
  license?: string;
  generator?: string;
  frameSize: [number, number];
  anchor: [number, number];
  sheet: string;
  columns: number;
  animations: Record<string, { frames: number[]; fps: number; loop: boolean }>;
  /** Optional per-frame landmarks used only for bounded procedural overlays. */
  anchors?: {
    idle?: Array<{
      eyeL: [number, number];
      eyeR: [number, number];
      pupilR: number;
      earL: [number, number];
      earR: [number, number];
    }>;
  };
  preview?: string;
}

export interface MicroMotionOverlay {
  gazeX: number;
  gazeY: number;
  earBias: number;
  intensity: number;
}

/** Safe fallback order when an animation is missing from a pack. */
const FALLBACKS: Record<string, string[]> = {
  // Safety-critical states fall back to *calmer* representations, never to
  // success/celebration.
  emergency: ["paused", "offline", "idle"],
  offline: ["paused", "idle"],
  blocked: ["paused", "idle"],
  unknown: ["paused", "idle"],
  // Failed has dedicated art in v2 packs; v1 packs fall back to blocked
  // (never to success) — the fixed wording keeps the states distinct.
  failed: ["blocked", "paused", "idle"],
  success: ["idle"],
  // v2 ambient/performance animations degrade gracefully on v1 packs.
  listening: ["notice", "idle"],
  curious: ["notice", "idle"],
  stretch: ["idle"],
  lie: ["quiet", "idle"],
  legswing: ["idle"],
  tailhug: ["quiet", "idle"],
  default: ["idle"],
};

export interface RendererBackend {
  setAnimation(name: string, frameSlice?: [number, number]): void;
  setReducedMotion(on: boolean): void;
  setMicroMotion(motion: MicroMotionOverlay): void;
  destroy(): void;
  /**
   * CPP §7 hide／suspend：停掉 rAF（不畫、不排程），狀態原地保留；resume 接續。
   * 可選——不是每個後端都有自己的迴圈（注入的假 renderer 可以省略）。
   */
  pause?(): void;
  resume?(): void;
}

export class SpriteRenderer implements RendererBackend {
  private ctx: CanvasRenderingContext2D;
  private sheet: HTMLImageElement | null = null;
  private manifest: PackManifest;
  private animation = "idle";
  private slice: [number, number] | null = null;
  private frameIndex = 0;
  private lastFrameAt = 0;
  private raf = 0;
  private reducedMotion = false;
  /** Reduced Motion 的靜態幀是否已畫過（畫面沒變就不重畫；換動畫／切片時重設）。 */
  private staticDrawn = false;
  /** destroy() 之後永遠不再排 rAF——包括圖片 onload 晚於 destroy 的情況。 */
  private destroyed = false;
  /** 暫停中（視窗隱藏／adapter suspend）：不排 rAF、不畫；resume 後接續。 */
  private paused = false;
  private scale: number;
  private micro: MicroMotionOverlay = { gazeX: 0, gazeY: 0, earBias: 0, intensity: 0 };

  constructor(
    private canvas: HTMLCanvasElement,
    manifest: PackManifest,
    sheetUrl: string,
    scale = 1
  ) {
    this.manifest = manifest;
    this.scale = scale;
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("no 2d context");
    this.ctx = ctx;
    const img = new Image();
    img.onload = () => {
      // 圖片載入可能晚於 destroy()（effect cleanup／StrictMode 雙跑／fallbackToText）：
      // 那時不能再啟動迴圈，否則這個 rAF 迴圈沒有任何人能取消。
      if (this.destroyed) return;
      this.sheet = img;
      if (!this.paused) this.loop(performance.now());
    };
    img.src = sheetUrl;
  }

  /** Resolve an animation name through the safe fallback chain. */
  resolveAnimation(name: string): string {
    if (this.manifest.animations[name]) return name;
    const chain = FALLBACKS[name] ?? FALLBACKS.default;
    for (const alt of chain) {
      if (this.manifest.animations[alt]) return alt;
    }
    return Object.keys(this.manifest.animations)[0] ?? name;
  }

  setAnimation(name: string, frameSlice?: [number, number]) {
    const resolved = this.resolveAnimation(name);
    if (resolved === this.animation && this.eqSlice(frameSlice)) return;
    this.animation = resolved;
    this.slice = frameSlice ?? null;
    this.frameIndex = 0;
    this.lastFrameAt = 0;
    this.staticDrawn = false;
  }

  private eqSlice(s?: [number, number]) {
    if (!s && !this.slice) return true;
    return !!s && !!this.slice && s[0] === this.slice[0] && s[1] === this.slice[1];
  }

  setReducedMotion(on: boolean) {
    if (on !== this.reducedMotion) this.staticDrawn = false;
    this.reducedMotion = on;
  }

  /** CPP §7 hide／suspend：取消 rAF，不再畫；動畫狀態原地保留。 */
  pause() {
    if (this.paused) return;
    this.paused = true;
    if (typeof cancelAnimationFrame === "function") cancelAnimationFrame(this.raf);
    this.raf = 0;
  }

  /** 恢復迴圈（圖片還沒載好就等 onload 自己啟動）。destroy 後不可恢復。 */
  resume() {
    if (!this.paused || this.destroyed) return;
    this.paused = false;
    // 重新顯示時重畫一次靜態幀（Reduced Motion 的 dirty 旗標）。
    this.staticDrawn = false;
    if (this.sheet) this.loop(performance.now());
  }

  isPaused() {
    return this.paused;
  }

  setMicroMotion(motion: MicroMotionOverlay) {
    const clamp = (v: number) => Math.max(-1, Math.min(1, Number.isFinite(v) ? v : 0));
    this.micro = {
      gazeX: clamp(motion.gazeX),
      gazeY: clamp(motion.gazeY),
      earBias: clamp(motion.earBias),
      intensity: Math.max(0, Math.min(1, Number.isFinite(motion.intensity) ? motion.intensity : 0)),
    };
  }

  destroy() {
    this.destroyed = true;
    if (typeof cancelAnimationFrame === "function") cancelAnimationFrame(this.raf);
    this.raf = 0;
  }

  private loop = (now: number) => {
    if (this.destroyed || this.paused) return;
    this.raf = requestAnimationFrame(this.loop);
    const anim = this.manifest.animations[this.animation];
    if (!anim || !this.sheet) return;
    const localFrames = this.slice
      ? anim.frames.slice(this.slice[0], this.slice[1] + 1)
      : anim.frames;
    if (localFrames.length === 0) return;

    if (this.reducedMotion) {
      // Static representation: hold the first frame of the state — drawn once,
      // not on every rAF (the picture does not change; Reduced Motion must
      // reduce work, not redraw a transparent window at full refresh rate).
      if (!this.staticDrawn) {
        this.draw(localFrames[0]);
        this.staticDrawn = true;
      }
      return;
    }
    this.staticDrawn = false;
    const interval = 1000 / anim.fps;
    if (now - this.lastFrameAt >= interval) {
      this.lastFrameAt = now;
      this.draw(localFrames[this.frameIndex % localFrames.length]);
      if (this.frameIndex < localFrames.length - 1) {
        this.frameIndex += 1;
      } else if (anim.loop) {
        this.frameIndex = 0;
      }
    }
  };

  private draw(globalFrame: number) {
    const [fw, fh] = this.manifest.frameSize;
    const col = globalFrame % this.manifest.columns;
    const row = Math.floor(globalFrame / this.manifest.columns);
    const dpr = window.devicePixelRatio || 1;
    const w = fw * this.scale;
    const h = fh * this.scale;
    if (this.canvas.width !== w * dpr || this.canvas.height !== h * dpr) {
      this.canvas.width = w * dpr;
      this.canvas.height = h * dpr;
      this.canvas.style.width = `${w}px`;
      this.canvas.style.height = `${h}px`;
    }
    this.ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);
    this.ctx.imageSmoothingEnabled = true;
    this.ctx.drawImage(
      this.sheet!,
      col * fw,
      row * fh,
      fw,
      fh,
      0,
      0,
      w * dpr,
      h * dpr
    );
    this.drawMicroMotion(globalFrame, dpr);
  }

  /**
   * 程序化疊加只在有正式 landmarks 的 idle 幀啟用。它不改真實狀態圖示、
   * 不接收任意程式碼，也不追蹤游標；舊 pack 沒 anchors 就安全 no-op。
   */
  private drawMicroMotion(globalFrame: number, dpr: number) {
    if (this.reducedMotion || this.micro.intensity <= 0) return;
    const idleIndex = this.manifest.animations.idle?.frames.indexOf(globalFrame) ?? -1;
    const anchor = idleIndex >= 0 ? this.manifest.anchors?.idle?.[idleIndex] : undefined;
    if (!anchor) return;
    const unit = this.scale * dpr;
    const gx = this.micro.gazeX * 1.7;
    const gy = this.micro.gazeY * 1.15;
    this.ctx.save();
    this.ctx.globalAlpha = 0.22 + this.micro.intensity * 0.28;
    this.ctx.fillStyle = "#d7fbff";
    for (const [x, y] of [anchor.eyeL, anchor.eyeR]) {
      this.ctx.beginPath();
      this.ctx.arc((x + gx) * unit, (y + gy) * unit, 1.15 * unit, 0, Math.PI * 2);
      this.ctx.fill();
    }
    // 耳尖的注意力線：左右相反偏轉，幅度小且延遲於視線。
    this.ctx.strokeStyle = "#8ce7f2";
    this.ctx.lineWidth = Math.max(1, 0.75 * unit);
    for (const [anchorPoint, side] of [
      [anchor.earL, -1],
      [anchor.earR, 1],
    ] as const) {
      const [x, y] = anchorPoint;
      const bend = this.micro.earBias * side * 1.6;
      this.ctx.beginPath();
      this.ctx.moveTo(x * unit, (y - 1) * unit);
      this.ctx.lineTo((x + bend) * unit, (y - 4.2) * unit);
      this.ctx.stroke();
    }
    this.ctx.restore();
  }
}

/** Validate a pack manifest (shared by tests and the loader). */
export function validateManifest(m: unknown): string[] {
  const issues: string[] = [];
  const man = m as Partial<PackManifest>;
  if (!man || typeof man !== "object") return ["manifest is not an object"];
  if (man.kind !== "character-pack") issues.push("kind must be character-pack");
  if (!man.schemaVersion) issues.push("schemaVersion missing");
  if (!man.id || !/^[a-z0-9][a-z0-9-]{0,63}$/.test(man.id)) issues.push("invalid id");
  if (!Array.isArray(man.frameSize) || man.frameSize.length !== 2)
    issues.push("frameSize must be [w,h]");
  if (!Array.isArray(man.anchor) || man.anchor.length !== 2) issues.push("anchor must be [x,y]");
  if (typeof man.sheet !== "string" || man.sheet.includes("..") || man.sheet.includes("/"))
    issues.push("sheet must be a plain filename (no paths)");
  if (!man.animations || typeof man.animations !== "object" || !("idle" in man.animations))
    issues.push("animations must include idle");
  for (const [name, a] of Object.entries(man.animations ?? {})) {
    if (!Array.isArray(a.frames) || a.frames.length === 0)
      issues.push(`animation ${name}: empty frames`);
    if (!(a.fps > 0 && a.fps <= 30)) issues.push(`animation ${name}: fps out of range`);
    if ((a.frames ?? []).some((f) => !Number.isInteger(f) || f < 0 || f > 4096))
      issues.push(`animation ${name}: frame index out of range`);
  }
  const idleAnchors = man.anchors?.idle;
  if (idleAnchors !== undefined) {
    const idleFrames = man.animations?.idle?.frames.length ?? 0;
    if (!Array.isArray(idleAnchors) || idleAnchors.length !== idleFrames) {
      issues.push("anchors.idle must match idle frame count");
    } else {
      for (const [i, a] of idleAnchors.entries()) {
        const points = [a.eyeL, a.eyeR, a.earL, a.earR];
        if (
          points.some(
            (p) =>
              !Array.isArray(p) ||
              p.length !== 2 ||
              p.some((n) => typeof n !== "number" || !Number.isFinite(n))
          ) ||
          typeof a.pupilR !== "number" ||
          !Number.isFinite(a.pupilR)
        ) {
          issues.push(`anchors.idle[${i}] invalid`);
        }
      }
    }
  }
  return issues;
}
