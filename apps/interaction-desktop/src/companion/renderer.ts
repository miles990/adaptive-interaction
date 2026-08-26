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
  frameSize: [number, number];
  anchor: [number, number];
  sheet: string;
  columns: number;
  animations: Record<string, { frames: number[]; fps: number; loop: boolean }>;
  preview?: string;
}

/** Safe fallback order when an animation is missing from a pack. */
const FALLBACKS: Record<string, string[]> = {
  // Safety-critical states fall back to *calmer* representations, never to
  // success/celebration.
  emergency: ["paused", "offline", "idle"],
  offline: ["paused", "idle"],
  blocked: ["paused", "idle"],
  unknown: ["paused", "idle"],
  success: ["idle"],
  default: ["idle"],
};

export interface RendererBackend {
  setAnimation(name: string, frameSlice?: [number, number]): void;
  setReducedMotion(on: boolean): void;
  destroy(): void;
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
  private scale: number;

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
      this.sheet = img;
      this.loop(performance.now());
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
  }

  private eqSlice(s?: [number, number]) {
    if (!s && !this.slice) return true;
    return !!s && !!this.slice && s[0] === this.slice[0] && s[1] === this.slice[1];
  }

  setReducedMotion(on: boolean) {
    this.reducedMotion = on;
  }

  destroy() {
    cancelAnimationFrame(this.raf);
  }

  private loop = (now: number) => {
    this.raf = requestAnimationFrame(this.loop);
    const anim = this.manifest.animations[this.animation];
    if (!anim || !this.sheet) return;
    const localFrames = this.slice
      ? anim.frames.slice(this.slice[0], this.slice[1] + 1)
      : anim.frames;
    if (localFrames.length === 0) return;

    if (this.reducedMotion) {
      // Static representation: hold the first frame of the state.
      this.draw(localFrames[0]);
      return;
    }
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
  return issues;
}
