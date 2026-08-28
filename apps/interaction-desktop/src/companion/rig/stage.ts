// StageRenderer：遊玩場渲染後端（RendererBackend 實作）。
//
// 一個 canvas ＝ 一個小舞台：場景 → 玩具 → 使魔 → 小樞（可走動、翻面）。
// - 表情：machine 驅動的動畫永遠優先；只有 idle 時 playfield 的遊玩
//   模式才覆蓋（play-chase/sneak-closer/…）。真相狀態不受任何遊玩影響。
// - 指標座標只存在於本視窗 canvas 內，永不送 runtime/AI、不持久化。
// - Reduced Motion：無自主移動、無物理彈跳、無粒子；狀態辨識保留。

import { RendererBackend, MicroMotionOverlay } from "../renderer";
import { drawRig } from "./draw";
import { clampParams, mixColor, RIG_PALETTES, RigPalette } from "./params";
import { ExpressionTimeline } from "./timeline";
import {
  clearToys,
  createWorld,
  dragToy,
  Familiar,
  grabToyAt,
  releaseToy,
  rollCall,
  spawnToy,
  stepWorld,
  Toy,
  ToyKind,
  World,
} from "../playfield";

/** playfield 會請求的表演表情（機器 performing 中仍算遊玩狀態）。 */
export const PLAYFIELD_EXPRESSIONS = new Set(["hold-ball", "keep-ball", "pounce-miss"]);

export type StageScene = "none" | "nest" | "desk" | "sill" | "night";
export const STAGE_SCENES: StageScene[] = ["none", "nest", "desk", "sill", "night"];

export interface StageToggles {
  play: boolean;
  cursorPlay: boolean;
  deskMove: boolean;
}

interface MachineFlags {
  ambient: boolean;
  frozen: boolean;
  quiet: boolean;
  playPerforming: boolean;
}

export class StageRenderer implements RendererBackend {
  private ctx: CanvasRenderingContext2D;
  private paletteName: string;
  private scale: number;
  private raf = 0;
  private timeline: ExpressionTimeline;
  private now: () => number;
  private rng: () => number;
  private destroyed = false;

  private world: World;
  private machineAnim = "offline";
  private machineSlice: [number, number] | undefined;
  private flags: MachineFlags = { ambient: false, frozen: true, quiet: false, playPerforming: false };
  private toggles: StageToggles = { play: true, cursorPlay: true, deskMove: true };
  private scene: StageScene = "none";
  private charName = "小樞";
  private pointer: { x: number; y: number; active: boolean } | null = null;
  private draggingToy: number | null = null;
  private lastDrag: { x: number; y: number; at: number; vx: number; vy: number } | null = null;
  private lastStep = 0;
  private exprCb: ((id: string, durationMs: number) => void) | null = null;

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
    this.rng = opts?.rng ?? Math.random;
    this.timeline = new ExpressionTimeline(this.rng, this.now());
    // 邏輯舞台大小由 canvas CSS 尺寸/scale 決定；先建立再於 render 校正。
    this.world = createWorld(320, 170);
    this.lastStep = this.now();
    if (opts?.autoStart !== false) this.loop();
  }

  // ---- RendererBackend ----
  setAnimation(name: string, frameSlice?: [number, number]): void {
    this.machineAnim = name;
    this.machineSlice = frameSlice;
  }

  setReducedMotion(on: boolean): void {
    this.timeline.setReducedMotion(on);
  }

  setMicroMotion(motion: MicroMotionOverlay): void {
    this.timeline.setMicroMotion(motion);
  }

  destroy(): void {
    this.destroyed = true;
    cancelAnimationFrame(this.raf);
  }

  // ---- stage 控制 ----
  setMachineFlags(flags: MachineFlags): void {
    this.flags = flags;
  }

  setToggles(t: Partial<StageToggles>): void {
    this.toggles = { ...this.toggles, ...t };
  }

  setScene(scene: string): void {
    this.scene = (STAGE_SCENES as string[]).includes(scene) ? (scene as StageScene) : "none";
  }

  setCharName(name: string): void {
    this.charName = name.slice(0, 24) || "小樞";
  }

  setFamiliars(configs: { id: string; name: string; palette: string }[]): void {
    const existing = new Map(this.world.familiars.map((f) => [f.id, f]));
    const familiars: Familiar[] = configs.slice(0, 3).map((c, i) => {
      const old = existing.get(c.id);
      return {
        id: c.id,
        name: c.name.slice(0, 24) || `使魔${i + 1}`,
        palette: RIG_PALETTES[c.palette] ? c.palette : "maid-classic",
        x: old?.x ?? 40 + i * 60,
        vx: old?.vx ?? 0,
        facing: old?.facing ?? 1,
        state: old?.state ?? "idle",
        stateUntil: old?.stateUntil ?? 0,
        greetWith: old?.greetWith ?? null,
      };
    });
    this.world = { ...this.world, familiars };
  }

  onExpressionEvent(cb: (id: string, durationMs: number) => void): void {
    this.exprCb = cb;
  }

  spawnToy(kind: ToyKind): void {
    this.world = spawnToy(this.world, kind, this.now());
  }

  clearAllToys(): void {
    this.world = clearToys(this.world);
  }

  toyCount(): number {
    return this.world.toys.length;
  }

  worldBusy(): boolean {
    return this.world.char.mode !== "free";
  }

  rollCallNow(machineLabel: string | null): { name: string; activity: string }[] {
    return rollCall(this.world, this.charName, machineLabel);
  }

  /** 角色目前的 hit-rect（CSS px，供 companion_hit_rect）。 */
  charHitRect(): { x: number; y: number; w: number; h: number } {
    const s = this.scale;
    const cx = this.world.char.x;
    return {
      x: (cx - 26) * s,
      y: (this.world.ground - 122) * s,
      w: 52 * s,
      h: 124 * s,
    };
  }

  /** 互動範圍（角色＋玩具的聯集，CSS px）：有玩具時游標不可穿透它們。 */
  interactiveBounds(): { x: number; y: number; w: number; h: number } {
    let r = this.charHitRect();
    if (this.world.toys.length === 0) return r;
    const s = this.scale;
    let x0 = r.x;
    let y0 = r.y;
    let x1 = r.x + r.w;
    let y1 = r.y + r.h;
    for (const t of this.world.toys) {
      x0 = Math.min(x0, (t.x - 14) * s);
      y0 = Math.min(y0, (t.y - 14) * s);
      x1 = Math.max(x1, (t.x + 14) * s);
      y1 = Math.max(y1, (t.y + 14) * s);
    }
    return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
  }

  // ---- 指標（canvas CSS px；回傳命中類型供呼叫端決定行為） ----
  pointerDown(cssX: number, cssY: number): "toy" | "char" | "none" {
    const x = cssX / this.scale;
    const y = cssY / this.scale;
    const { world, toyId } = grabToyAt(this.world, x, y);
    if (toyId != null) {
      this.world = world;
      this.draggingToy = toyId;
      this.lastDrag = { x, y, at: this.now(), vx: 0, vy: 0 };
      return "toy";
    }
    const r = this.charHitRect();
    if (cssX >= r.x && cssX <= r.x + r.w && cssY >= r.y && cssY <= r.y + r.h) return "char";
    return "none";
  }

  pointerMove(cssX: number, cssY: number): void {
    const x = cssX / this.scale;
    const y = cssY / this.scale;
    this.pointer = { x, y, active: true };
    if (this.draggingToy != null && this.lastDrag) {
      const now = this.now();
      const dt = Math.max(8, now - this.lastDrag.at);
      const vx = ((x - this.lastDrag.x) / dt) * 1000;
      const vy = ((y - this.lastDrag.y) / dt) * 1000;
      // 平滑速度（丟出手感）。
      this.lastDrag = {
        x,
        y,
        at: now,
        vx: this.lastDrag.vx * 0.5 + vx * 0.5,
        vy: this.lastDrag.vy * 0.5 + vy * 0.5,
      };
      this.world = dragToy(this.world, this.draggingToy, x, y, this.lastDrag.vx, this.lastDrag.vy);
    }
  }

  pointerUp(): void {
    if (this.draggingToy != null && this.lastDrag) {
      this.world = releaseToy(
        this.world,
        this.draggingToy,
        this.lastDrag.vx,
        this.lastDrag.vy,
        this.now()
      );
    }
    this.draggingToy = null;
    this.lastDrag = null;
  }

  pointerLeave(): void {
    this.pointer = null;
    this.pointerUp();
  }

  isDraggingToy(): boolean {
    return this.draggingToy != null;
  }

  // ---- 主迴圈 ----
  private loop = () => {
    if (this.destroyed) return;
    this.raf = requestAnimationFrame(this.loop);
    this.renderFrame();
  };

  renderFrame(now = this.now()): void {
    const dpr = window.devicePixelRatio || 1;
    // 舞台 CSS 尺寸即 canvas 版面；邏輯尺寸 = CSS / scale。
    const cssW = this.canvas.clientWidth || 320 * this.scale;
    const cssH = this.canvas.clientHeight || 170 * this.scale;
    const logicalW = cssW / this.scale;
    const logicalH = cssH / this.scale;
    if (this.world.w !== logicalW || this.world.h !== logicalH) {
      const old = this.world;
      this.world = {
        ...old,
        w: logicalW,
        h: logicalH,
        ground: logicalH - 6,
        char: { ...old.char, x: Math.min(old.char.x, logicalW - 30) },
      };
    }
    if (this.canvas.width !== Math.round(cssW * dpr) || this.canvas.height !== Math.round(cssH * dpr)) {
      this.canvas.width = Math.round(cssW * dpr);
      this.canvas.height = Math.round(cssH * dpr);
    }

    // 物理與決策步進。
    const dtMs = Math.min(100, now - this.lastStep);
    this.lastStep = now;
    const reduced = this.timeline.isReducedMotion();
    const { world, events } = stepWorld(
      this.world,
      {
        nowMs: now,
        dtMs,
        ambient: this.flags.ambient || this.flags.playPerforming,
        frozen: this.flags.frozen,
        quiet: this.flags.quiet,
        reducedMotion: reduced,
        playEnabled: this.toggles.play,
        cursorPlayEnabled: this.toggles.cursorPlay,
        deskMoveEnabled: this.toggles.deskMove,
        pointer: this.pointer,
      },
      this.rng
    );
    this.world = world;
    for (const e of events) {
      if (e.type === "expression" && this.exprCb) this.exprCb(e.id, e.durationMs);
    }

    // 表情選擇：machine 非 idle 一律優先；idle 時允許遊玩覆蓋。
    const mode = this.world.char.mode;
    let effective = this.machineAnim;
    let slice = this.machineSlice;
    if (this.machineAnim === "idle") {
      if (mode === "chase") effective = "play-chase";
      else if (mode === "pounce") effective = "sneak-closer";
      else if (mode === "return" || mode === "refuse") effective = "play-carry";
      else if (mode === "stroll") effective = "play-chase";
      if (effective !== this.machineAnim) slice = undefined;
    }
    this.timeline.setAnimation(effective, now, slice);
    let params = this.timeline.paramsAt(now);

    // 移動 secondary motion：步態、髮尾、頭飾微彈。
    const speed = Math.abs(this.world.char.vx);
    if (speed > 1 && !reduced) {
      const cyc = now / (speed > 80 ? 90 : 140);
      params = clampParams({
        ...params,
        pose: "stand",
        legPhase: Math.sin(cyc),
        bodyBob: params.bodyBob - Math.abs(Math.sin(cyc)) * 1.8,
        hairSway: Math.sin(cyc * 0.9) * 0.6,
        tailSway: Math.sin(cyc * 0.8) * 0.5,
      });
    }

    // ---- 繪製 ----
    const ctx = this.ctx;
    const pal = RIG_PALETTES[this.paletteName];
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);
    ctx.scale(this.scale * dpr, this.scale * dpr);

    this.drawScene(ctx, logicalW, logicalH, pal);
    for (const f of this.world.familiars) this.drawFamiliar(ctx, f, now);
    for (const t of this.world.toys) if (t.kind !== "wand") this.drawToy(ctx, t, pal, now);

    // 角色（平移＋翻面）。
    ctx.save();
    ctx.translate(this.world.char.x - 64, this.world.ground - 124);
    if (this.world.char.facing === -1) {
      ctx.translate(64, 0);
      ctx.scale(-1, 1);
      ctx.translate(-64, 0);
    }
    drawRig(ctx, params, pal);
    ctx.restore();

    // 逗貓棒畫在最上（有「線」從上方垂到玩具）。
    for (const t of this.world.toys) if (t.kind === "wand") this.drawToy(ctx, t, pal, now);

    ctx.setTransform(1, 0, 0, 1, 0, 0);
  }

  // ---- 場景（低調的桌面情境，透明模式仍以透明為主） ----
  private drawScene(ctx: CanvasRenderingContext2D, w: number, h: number, pal: RigPalette) {
    const g = this.world.ground;
    ctx.save();
    switch (this.scene) {
      case "nest": {
        // 小巢穴：角色腳下一個軟墊。
        ctx.fillStyle = "rgba(216,167,196,0.35)";
        ctx.beginPath();
        ctx.ellipse(this.world.char.x, g - 2, 46, 8, 0, 0, Math.PI * 2);
        ctx.fill();
        ctx.strokeStyle = "rgba(216,167,196,0.5)";
        ctx.stroke();
        break;
      }
      case "desk": {
        // 工作桌：底部一條桌面線＋馬克杯剪影。
        ctx.strokeStyle = "rgba(160,150,140,0.5)";
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.moveTo(4, g + 3);
        ctx.lineTo(w - 4, g + 3);
        ctx.stroke();
        ctx.fillStyle = "rgba(160,150,140,0.35)";
        ctx.fillRect(w - 30, g - 16, 14, 16);
        ctx.strokeRect(w - 30, g - 16, 14, 16);
        break;
      }
      case "sill": {
        // 窗台：底線＋小盆栽剪影。
        ctx.strokeStyle = "rgba(140,160,150,0.5)";
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.moveTo(4, g + 3);
        ctx.lineTo(w - 4, g + 3);
        ctx.stroke();
        ctx.fillStyle = "rgba(120,170,130,0.4)";
        ctx.beginPath();
        ctx.ellipse(20, g - 16, 8, 10, 0, 0, Math.PI * 2);
        ctx.fill();
        ctx.fillStyle = "rgba(150,120,100,0.4)";
        ctx.fillRect(14, g - 8, 12, 8);
        break;
      }
      case "night": {
        // 夜間：微暗底暈＋兩顆小星。
        ctx.fillStyle = "rgba(20,24,40,0.18)";
        ctx.fillRect(0, 0, w, h);
        ctx.fillStyle = "rgba(230,235,255,0.6)";
        for (const [sx, sy] of [
          [w * 0.15, 18],
          [w * 0.8, 12],
        ] as const) {
          ctx.beginPath();
          ctx.arc(sx, sy, 1.4, 0, Math.PI * 2);
          ctx.fill();
        }
        break;
      }
      case "none":
      default:
        break;
    }
    void pal;
    ctx.restore();
  }

  // ---- 玩具 ----
  private drawToy(ctx: CanvasRenderingContext2D, t: Toy, pal: RigPalette, now: number) {
    ctx.save();
    switch (t.kind) {
      case "yarn": {
        ctx.fillStyle = pal.pinkLilac;
        ctx.strokeStyle = mixColor(pal.pinkLilac, "#000000", 0.35);
        ctx.beginPath();
        ctx.arc(t.x, t.y, 8, 0, Math.PI * 2);
        ctx.fill();
        ctx.stroke();
        ctx.beginPath();
        ctx.arc(t.x, t.y, 8, 0.4, 2.2);
        ctx.arc(t.x, t.y, 5, 2.8, 4.6);
        ctx.stroke();
        break;
      }
      case "paper": {
        ctx.fillStyle = "#efe9dc";
        ctx.strokeStyle = "#b9b2a0";
        ctx.beginPath();
        const r = 7;
        for (let i = 0; i < 7; i++) {
          const a = (i / 7) * Math.PI * 2 + (t.id % 5);
          const rr = r * (0.75 + ((i * 37 + t.id) % 10) / 28);
          const x = t.x + rr * Math.cos(a);
          const y = t.y + rr * Math.sin(a);
          if (i === 0) ctx.moveTo(x, y);
          else ctx.lineTo(x, y);
        }
        ctx.closePath();
        ctx.fill();
        ctx.stroke();
        break;
      }
      case "plane": {
        ctx.fillStyle = "#f2f4f8";
        ctx.strokeStyle = "#a9b2c4";
        const dir = t.vx >= 0 ? 1 : -1;
        ctx.translate(t.x, t.y);
        ctx.rotate(Math.atan2(t.vy, t.vx * dir) * 0.3 * dir);
        ctx.beginPath();
        ctx.moveTo(10 * dir, 0);
        ctx.lineTo(-8 * dir, -5);
        ctx.lineTo(-4 * dir, 0);
        ctx.lineTo(-8 * dir, 5);
        ctx.closePath();
        ctx.fill();
        ctx.stroke();
        break;
      }
      case "light": {
        const active = this.pointer !== null && this.toggles.cursorPlay;
        const a = active ? 0.85 : Math.max(0, t.interest) * 0.5;
        if (a > 0.02) {
          const grad = ctx.createRadialGradient(t.x, t.y, 0, t.x, t.y, 10);
          grad.addColorStop(0, `rgba(255,240,180,${a})`);
          grad.addColorStop(1, "rgba(255,240,180,0)");
          ctx.fillStyle = grad;
          ctx.beginPath();
          ctx.arc(t.x, t.y, 10, 0, Math.PI * 2);
          ctx.fill();
        }
        break;
      }
      case "wand": {
        // 線從舞台上方垂到羽毛玩具。
        ctx.strokeStyle = "rgba(150,150,160,0.7)";
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(t.x + 6, 0);
        ctx.quadraticCurveTo(t.x + 4, t.y * 0.5, t.x, t.y - 6);
        ctx.stroke();
        // 羽毛。
        const sway = Math.sin(now / 260) * 0.35;
        ctx.translate(t.x, t.y);
        ctx.rotate(sway);
        ctx.fillStyle = pal.warmOrange;
        ctx.strokeStyle = mixColor(pal.warmOrange, "#000000", 0.3);
        ctx.beginPath();
        ctx.ellipse(0, 0, 3.4, 8, 0.3, 0, Math.PI * 2);
        ctx.fill();
        ctx.stroke();
        ctx.beginPath();
        ctx.ellipse(2.5, 2, 2.6, 6, -0.2, 0, Math.PI * 2);
        ctx.fill();
        break;
      }
    }
    ctx.restore();
  }

  // ---- 使魔（小型使魔：迷你貓精靈） ----
  private drawFamiliar(ctx: CanvasRenderingContext2D, f: Familiar, now: number) {
    const pal = RIG_PALETTES[f.palette] ?? RIG_PALETTES["maid-classic"];
    const g = this.world.ground;
    const bob = f.state === "walk" || f.state === "chase" ? Math.abs(Math.sin(now / 120)) * 2 : 0;
    const y = g - 10 - bob;
    ctx.save();
    ctx.translate(f.x, y);
    if (f.facing === -1) ctx.scale(-1, 1);
    // 身體。
    ctx.fillStyle = pal.hair;
    ctx.strokeStyle = pal.hairEdge;
    ctx.beginPath();
    ctx.ellipse(0, 0, 9, 8, 0, 0, Math.PI * 2);
    ctx.fill();
    ctx.stroke();
    // 耳。
    for (const s of [-1, 1] as const) {
      ctx.beginPath();
      ctx.moveTo(s * 6.5, -4);
      ctx.lineTo(s * 4, -11);
      ctx.lineTo(s * 1.5, -5.5);
      ctx.closePath();
      ctx.fill();
    }
    // 尾巴。
    ctx.strokeStyle = pal.hair;
    ctx.lineWidth = 2.6;
    ctx.lineCap = "round";
    ctx.beginPath();
    ctx.moveTo(-8, 2);
    ctx.quadraticCurveTo(-14, -2 + Math.sin(now / 400) * 2, -13, -8);
    ctx.stroke();
    // 臉。
    if (f.state === "sleep") {
      ctx.strokeStyle = "#e8e4f0";
      ctx.lineWidth = 1;
      for (const s of [-1, 1] as const) {
        ctx.beginPath();
        ctx.moveTo(s * 4 - 1.5, -1);
        ctx.quadraticCurveTo(s * 4, 0.5, s * 4 + 1.5, -1);
        ctx.stroke();
      }
      ctx.fillStyle = "#9db2c8";
      ctx.font = "bold 6px Arial, sans-serif";
      ctx.fillText("z", 8, -10);
    } else {
      ctx.fillStyle = "#f4efff";
      for (const s of [-1, 1] as const) {
        ctx.beginPath();
        ctx.arc(s * 3.6, -1, 1.5, 0, Math.PI * 2);
        ctx.fill();
      }
      ctx.fillStyle = pal.pupil;
      for (const s of [-1, 1] as const) {
        ctx.beginPath();
        ctx.arc(s * 3.6 + 0.5, -1, 0.7, 0, Math.PI * 2);
        ctx.fill();
      }
    }
    // 打招呼：愛心。
    if (f.state === "greet") {
      ctx.fillStyle = pal.pinkLilac;
      const hy = -16 - Math.sin(now / 300) * 1.5;
      ctx.beginPath();
      ctx.moveTo(0, hy + 2.4);
      ctx.bezierCurveTo(-4.2, hy - 1, -1.8, hy - 3.4, 0, hy - 1);
      ctx.bezierCurveTo(1.8, hy - 3.4, 4.2, hy - 1, 0, hy + 2.4);
      ctx.fill();
    }
    ctx.restore();
  }
}
