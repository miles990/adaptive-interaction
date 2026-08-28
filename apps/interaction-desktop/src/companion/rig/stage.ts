// StageRenderer：遊玩場渲染後端（RendererBackend 實作）。
//
// 一個 canvas ＝ 一個小舞台：場景 → 玩具 → 使魔 → 小樞（可走動、翻面）。
// - 表情：machine 驅動的動畫永遠優先；只有 idle 時 playfield 的遊玩
//   模式才覆蓋（play-chase/sneak-closer/…）。真相狀態不受任何遊玩影響。
// - 指標座標只存在於本視窗 canvas 內，永不送 runtime/AI、不持久化。
// - Reduced Motion：無自主移動、無物理彈跳、無粒子；狀態辨識保留。

import { RendererBackend, MicroMotionOverlay } from "../renderer";
import { drawRig } from "./draw";
import { clampParams, mixColor, RIG_PALETTES, RigPalette, RigParams } from "./params";
import { AttentionStagger, ExpressionTimeline, resolveRigAnimation } from "./timeline";
import { EXPRESSION_ALIASES } from "./expressions";
import {
  CharPlayMode,
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
import {
  FrameBudgetState,
  frameBudgetPolicy,
  initialFrameBudget,
  shouldDrawFrame,
} from "../gameFeel";
import { DEFAULT_TUNING, PersonalityTuning } from "../personality";

/** playfield 會請求的表演表情（機器 performing 中仍算遊玩狀態）。 */
export const PLAYFIELD_EXPRESSIONS = new Set([
  "hold-ball",
  "keep-ball",
  "pounce-miss",
  "curious",
]);

// ---------------------------------------------------------------------------
// 組合式通道（spec §6.2）：狀態不一定要整體覆蓋遊玩姿勢。
// ---------------------------------------------------------------------------

/**
 * 工作/等待類的「非安全真相狀態」：只借用核心、頭飾、裙擺光與耳朵通道，
 * 身體姿勢仍然是遊玩中的姿勢（趴著＋核心顯示 Agent 工作中）。
 */
const OVERLAY_STATUS = new Set([
  "queued",
  "routing",
  "working",
  "thinking",
  "wait-codex",
  "wait-claude",
  "waiting",
  "ask",
  "listening",
]);

/** 只被狀態借用的通道（其餘通道留給遊玩姿勢）。 */
export const STATUS_CHANNELS = [
  "coreGlow",
  "corePulse",
  "headpieceGlow",
  "skirtGlow",
  "skirtTone",
  "earL",
  "earR",
] as const;

/**
 * 機器動畫要怎麼跟遊玩姿勢共存：
 *   none     ＝ 沒有狀態（idle），遊玩姿勢自己說了算
 *   overlay  ＝ 只覆蓋狀態通道，保留遊玩姿勢
 *   takeover ＝ 整體搶佔（安全與結果狀態：緊急、擋下、失敗、未知、離線、
 *              暫停、成功、需要確認，以及所有直接互動/表演）
 */
export function statusOverlay(machineAnim: string): "none" | "overlay" | "takeover" {
  if (machineAnim === "idle") return "none";
  const id = EXPRESSION_ALIASES[machineAnim] ?? machineAnim;
  return OVERLAY_STATUS.has(id) ? "overlay" : "takeover";
}

/** 遊玩模式 → 遊玩表情（沒有專屬表情就回 null）。 */
export function playExpressionFor(mode: CharPlayMode): string | null {
  switch (mode) {
    case "chase":
    case "stroll":
      return "play-chase";
    case "pounce":
      return "sneak-closer";
    case "return":
    case "refuse":
    case "carry":
      return "play-carry";
    case "sniff":
      return "curious";
    default:
      return null;
  }
}

/** 狀態表情裡屬於「狀態通道」的參數（沒有就回 null）。 */
export function statusChannelParams(machineAnim: string): Partial<RigParams> | null {
  const { expr } = resolveRigAnimation(machineAnim);
  const out: Partial<RigParams> = {};
  let found = false;
  for (const key of STATUS_CHANNELS) {
    const value = expr.hold[key];
    if (value !== undefined) {
      (out as Record<string, unknown>)[key] = value;
      found = true;
    }
  }
  return found ? out : null;
}

/**
 * 遊玩場要不要繼續運轉。
 *
 * 只借通道的工作/等待狀態（routing/working/waiting/…）不該讓整個遊玩場停住
 * ——她可以一邊玩、一邊用胸前核心顯示 Agent 在工作。安全與結果狀態
 * （emergency/blocked/failed/unknown/offline/paused/success/…）仍然整體搶佔，
 * 遊玩立刻停止。
 */
export function playfieldActive(
  machineAnim: string,
  poseAmbient: boolean,
  playPerforming: boolean
): boolean {
  return poseAmbient || playPerforming || statusOverlay(machineAnim) === "overlay";
}

// ---------------------------------------------------------------------------
// hit-rect 回報節流（點擊穿透的新鮮度）
// ---------------------------------------------------------------------------

export interface HitRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** 兩次回報之間的最小間隔（不得每幀 invoke）。 */
export const HIT_RECT_MIN_INTERVAL_MS = 50;
/** 沒有位移時的最長沉默：超過就補一次（Rust 端的框永遠不會太舊）。 */
export const HIT_RECT_MAX_QUIET_MS = 60;
/** 位移多少才值得立刻回報（px）。 */
export const HIT_RECT_MOVE_EPS = 4;

/**
 * 這一幀該不該把 hit-rect 回報給 Rust？
 *
 * 角色會走動、玩具會滾，互動框每幀都在變；只在 500ms pump 回報的話，Rust
 * 的點擊穿透輪詢會拿著最多半秒前的框判定——追逐／拖曳玩具時指標事件就掉了。
 * 這裡是有界節流：至少隔 50ms、位移 >4px 立刻報，否則 60ms 補一次。
 *
 * @param dtMs 距離上次回報的時間（首次回報傳 Infinity 或給 prev=null）。
 */
export function hitRectReportPolicy(
  prev: HitRect | null,
  next: HitRect,
  dtMs: number
): boolean {
  if (!prev) return true;
  if (!Number.isFinite(dtMs)) return true;
  if (dtMs < HIT_RECT_MIN_INTERVAL_MS) return false;
  const moved =
    Math.abs(next.x - prev.x) > HIT_RECT_MOVE_EPS ||
    Math.abs(next.y - prev.y) > HIT_RECT_MOVE_EPS ||
    Math.abs(next.w - prev.w) > HIT_RECT_MOVE_EPS ||
    Math.abs(next.h - prev.h) > HIT_RECT_MOVE_EPS;
  return moved || dtMs >= HIT_RECT_MAX_QUIET_MS;
}

// ---------------------------------------------------------------------------
// Reduced Motion：所有 sway/浮動取常數
// ---------------------------------------------------------------------------

/**
 * 週期性擺動。Reduced Motion 時回 0（不是「幅度小一點」——是真的不動）。
 * 逗貓棒羽毛、使魔尾巴、打招呼愛心、小物件轉動都走這裡。
 */
export function swayAt(nowMs: number, periodMs: number, amp: number, reduced: boolean): number {
  if (reduced) return 0;
  const t = Number.isFinite(nowMs) ? nowMs : 0;
  const p = periodMs > 0 ? periodMs : 1;
  return Math.sin(t / p) * amp;
}

/**
 * 注視偏移 → 參數（回看使魔／看向游標）。`dir` 為 -1..1 的**角色本地**方向
 * （已考慮翻面），正=角色的右手邊。只動視線/耳朵/微幅轉頭，不改姿勢，
 * 也永遠不套在真相狀態上（呼叫端把關）。
 */
export function gazeBiasParams(p: RigParams, dir: number, strength = 1): RigParams {
  const d = Math.max(-1, Math.min(1, Number.isFinite(dir) ? dir : 0)) *
    Math.max(0, Math.min(1, Number.isFinite(strength) ? strength : 0));
  if (d === 0) return p;
  return clampParams({
    ...p,
    headTurn: p.headTurn + d * 0.3,
    pupilX: p.pupilX + d * 2.2,
    earPerk: Math.max(p.earPerk, 0.72),
    earLTilt: p.earLTilt - d * 5,
    earRTilt: p.earRTilt + d * 5,
  });
}

/** 游標在角色附近時的注視方向（-1..1；不在場內或太遠回 0）。 */
export function pointerGazeDir(
  pointer: { x: number; y: number } | null,
  charX: number,
  rangePx = 110
): number {
  if (!pointer || !Number.isFinite(pointer.x) || rangePx <= 0) return 0;
  const dx = pointer.x - charX;
  if (Math.abs(dx) > rangePx) return Math.sign(dx);
  return Math.max(-1, Math.min(1, dx / rangePx));
}

export interface StagePlan {
  /** 要播的表情/動畫名。 */
  expression: string;
  /** 是否沿用 machine 的 frameSlice（遊玩姿勢時不沿用）。 */
  useMachineSlice: boolean;
  /** 疊在遊玩姿勢上的狀態通道（null＝不疊）。 */
  statusChannels: Partial<RigParams> | null;
}

/**
 * 遊玩姿勢 × 機器狀態的合成計畫（純函式，可單測）：
 *   - 安全/結果狀態一律整體搶佔（永遠不會被遊玩蓋掉）。
 *   - 工作/等待狀態只點亮核心/頭飾/裙擺/耳朵，身體照樣在玩。
 */
export function stageExpressionPlan(machineAnim: string, mode: CharPlayMode): StagePlan {
  const overlay = statusOverlay(machineAnim);
  const play = playExpressionFor(mode);
  if (overlay === "takeover" || !play) {
    return { expression: machineAnim, useMachineSlice: true, statusChannels: null };
  }
  if (overlay === "none") {
    return { expression: play, useMachineSlice: false, statusChannels: null };
  }
  return {
    expression: play,
    useMachineSlice: false,
    statusChannels: statusChannelParams(machineAnim),
  };
}

export type StageScene = "none" | "nest" | "desk" | "sill" | "night";
export const STAGE_SCENES: StageScene[] = ["none", "nest", "desk", "sill", "night"];

export interface StageToggles {
  play: boolean;
  cursorPlay: boolean;
  deskMove: boolean;
}

export interface MachineFlags {
  ambient: boolean;
  frozen: boolean;
  quiet: boolean;
  playPerforming: boolean;
}

/**
 * machine 真相狀態 → 舞台旗標（純函式）。
 *
 * 呼叫端必須在 machine 一變就套用（syncPose），不能等下一次 500ms pump：
 * 緊急停止後遊玩場多轉半秒＝角色在「已停止」的畫面上還在追球。
 */
export function machineStageFlags(
  base: string,
  transient: { kind: string; animation?: string } | null,
  poseAnimation: string,
  poseAmbient: boolean
): MachineFlags {
  const playPerforming =
    transient?.kind === "performing" && PLAYFIELD_EXPRESSIONS.has(transient.animation ?? "");
  return {
    // 工作/等待狀態只借通道：遊玩場繼續運轉（安全與結果狀態才整體停）。
    ambient: playfieldActive(poseAnimation, poseAmbient, playPerforming),
    frozen: ["emergency", "offline", "paused"].includes(base),
    quiet: base === "quiet",
    playPerforming,
  };
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
  private tuning: PersonalityTuning = DEFAULT_TUNING;
  private budget: FrameBudgetState = initialFrameBudget();
  private frameParity = 0;
  private lastLoopAt = 0;
  private hitRectCb: ((rect: HitRect) => void) | null = null;
  private lastReportedRect: HitRect | null = null;
  private lastReportAt = 0;
  /** 上一幀實際套用的 rig 參數（診斷／效能量測用）。 */
  private lastFrame: RigParams | null = null;

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

  /** 安靜時的「就地眨眼」：不換表情、不搶走安靜姿勢。 */
  blinkNow(): boolean {
    return this.timeline.blinkNow(this.now());
  }

  setMicroMotion(motion: MicroMotionOverlay): void {
    // 帶上此刻的時間戳：注意力分段（耳→視線→頭）要知道「什麼時候改變的」。
    this.timeline.setMicroMotion(motion, this.now());
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

  /** 個性 tuning：速度/距離/注意力分段（只影響呈現）。 */
  setTuning(tuning: PersonalityTuning): void {
    this.tuning = tuning;
    this.timeline.setAttentionStagger(tuning.attentionStagger as AttentionStagger);
  }

  /** 目前的幀預算狀態（30fps 降級診斷用）。 */
  frameBudget(): FrameBudgetState {
    return this.budget;
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

  /** 互動框回報：每幀依 hitRectReportPolicy 節流呼叫（不是每幀 invoke）。 */
  onHitRect(cb: (rect: HitRect) => void): void {
    this.hitRectCb = cb;
    this.lastReportedRect = null; // 換 callback：先報一次目前的框
  }

  /**
   * 依節流政策回報互動框。`force=true` 供 500ms 心跳使用（rAF 停擺時
   * ——視窗被隱藏、系統節流——仍要有一次回報）。
   */
  reportHitRect(force = false): void {
    if (!this.hitRectCb) return;
    const next = this.interactiveBounds();
    const now = this.now();
    const dt = this.lastReportedRect === null ? Number.POSITIVE_INFINITY : now - this.lastReportAt;
    if (!force && !hitRectReportPolicy(this.lastReportedRect, next, dt)) return;
    this.lastReportedRect = next;
    this.lastReportAt = now;
    this.hitRectCb(next);
  }

  /** 診斷用：上一幀實際套用的 rig 參數（無座標、無使用者資料）。 */
  lastFrameParams(): RigParams | null {
    return this.lastFrame;
  }

  /** 診斷用：目前被玩家抓著的玩具數。 */
  playerGrabbedToys(): number {
    return this.world.toys.filter((t) => t.grabbed === "player").length;
  }

  /** 診斷用：玩具在舞台上的位置（CSS px）。只活在本視窗，不外送。 */
  toyPoints(): { id: number; x: number; y: number }[] {
    return this.world.toys.map((t) => ({ id: t.id, x: t.x * this.scale, y: t.y * this.scale }));
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
    return rollCall(this.world, this.charName, machineLabel, this.now());
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

  /** 指標是否落在角色身上（hover 短氣泡用；座標只留在本視窗）。 */
  hitTestChar(cssX: number, cssY: number): boolean {
    const r = this.charHitRect();
    return cssX >= r.x && cssX <= r.x + r.w && cssY >= r.y && cssY <= r.y + r.h;
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
    const now = this.now();
    // 幀預算（§14）：最近 60 幀平均 >12ms 就每兩幀畫一次，<8ms 才回 60fps。
    if (this.lastLoopAt > 0) {
      this.budget = frameBudgetPolicy(this.budget, now - this.lastLoopAt);
    }
    this.lastLoopAt = now;
    this.frameParity = (this.frameParity + 1) % 2;
    if (!shouldDrawFrame(this.budget, this.frameParity)) return;
    this.renderFrame(now);
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
        speedScale: this.tuning.speedScale,
        chaseSpeedScale: this.tuning.chaseSpeedScale,
        approachDistance: this.tuning.approachDistance,
        riseDelayMs: this.tuning.riseDelayMs,
      },
      this.rng
    );
    this.world = world;
    for (const e of events) {
      if (e.type === "expression" && this.exprCb) this.exprCb(e.id, e.durationMs);
    }

    // 表情選擇（組合式通道，spec §6.2）：安全與結果狀態整體搶佔；
    // 工作/等待狀態只點亮核心/頭飾/裙擺/耳朵，身體維持遊玩姿勢。
    const plan = stageExpressionPlan(this.machineAnim, this.world.char.mode);
    this.timeline.setAnimation(plan.expression, now, plan.useMachineSlice ? this.machineSlice : undefined);
    let params = this.timeline.paramsAt(now);
    if (plan.statusChannels) params = clampParams({ ...params, ...plan.statusChannels });

    // 移動 secondary motion：步態、髮尾、頭飾微彈。
    //
    // 只有「遊玩姿勢真的在台上」時才覆蓋姿勢：機器狀態整體搶佔
    // （plan.useMachineSlice＝安全/結果狀態或沒有遊玩表情）時不得把 pose
    // 改成 stand——那會讓 doze/lie-flat 被步行姿勢蓋掉，也會讓緊急停止後
    // 殘留的速度繼續演走路。凍結狀態一律不動。
    const speed = Math.abs(this.world.char.vx);
    const walking = !plan.useMachineSlice && !this.flags.frozen && speed > 1 && !reduced;
    if (walking) {
      const cyc = now / (speed > 80 ? 90 : 140);
      params = clampParams({
        ...params,
        pose: "stand",
        poseBlend: 1,
        legPhase: Math.sin(cyc),
        bodyBob: params.bodyBob - Math.abs(Math.sin(cyc)) * 1.8,
        hairSway: Math.sin(cyc * 0.9) * 0.6,
        tailSway: Math.sin(cyc * 0.8) * 0.5,
      });
    }

    // 注視：有使魔向她打招呼就回看，否則游標靠近時看過來。
    // 只在「沒有整體搶佔」（idle 或只借通道的工作狀態）時疊，
    // 真相狀態與凍結狀態永遠不疊；Reduced Motion 不做這層。
    if (!reduced && !this.flags.frozen && statusOverlay(this.machineAnim) !== "takeover") {
      const char = this.world.char;
      const greeter =
        char.attendTo !== null && now <= char.attendUntil
          ? this.world.familiars.find((f) => f.id === char.attendTo)
          : undefined;
      let dirWorld = 0;
      if (greeter) {
        dirWorld = Math.max(-1, Math.min(1, (greeter.x - char.x) / 60));
      } else if (this.toggles.cursorPlay) {
        dirWorld = pointerGazeDir(this.pointer, char.x);
      }
      if (dirWorld !== 0) {
        params = gazeBiasParams(params, dirWorld * char.facing, greeter ? 1 : 0.8);
      }
    }

    // ---- 繪製 ----
    this.lastFrame = params;
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

    // 回一顆愛心（有使魔跟她打招呼時；純裝飾，非狀態符號）。
    if (this.world.char.greetBackUntil > now) {
      this.drawGreetHeart(ctx, this.world.char.x, this.world.ground - 132, pal, now, reduced);
    }

    // 逗貓棒畫在最上（有「線」從上方垂到玩具）。
    for (const t of this.world.toys) if (t.kind === "wand") this.drawToy(ctx, t, pal, now);

    ctx.setTransform(1, 0, 0, 1, 0, 0);

    // 互動框回報（節流；角色走動/玩具滾動時 Rust 的點擊穿透才不會用舊框）。
    this.reportHitRect();
  }

  /** 打招呼愛心（角色與使魔共用；Reduced Motion 時不浮動）。 */
  private drawGreetHeart(
    ctx: CanvasRenderingContext2D,
    x: number,
    y: number,
    pal: RigPalette,
    now: number,
    reduced: boolean
  ) {
    ctx.save();
    ctx.fillStyle = pal.pinkLilac;
    const hy = y - swayAt(now, 300, 1.5, reduced);
    ctx.beginPath();
    ctx.moveTo(x, hy + 2.4);
    ctx.bezierCurveTo(x - 4.2, hy - 1, x - 1.8, hy - 3.4, x, hy - 1);
    ctx.bezierCurveTo(x + 1.8, hy - 3.4, x + 4.2, hy - 1, x, hy + 2.4);
    ctx.fill();
    ctx.restore();
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
    const reduced = this.timeline.isReducedMotion();
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
      case "trinket": {
        // 小物件：一顆有稜角的小方塊＋掛環（看得出可以拖、不像食物）。
        const spin = swayAt(now + t.id * 900, 900, 0.12, reduced);
        ctx.translate(t.x, t.y);
        ctx.rotate(spin);
        ctx.fillStyle = mixColor(pal.toolViolet, "#ffffff", 0.25);
        ctx.strokeStyle = mixColor(pal.toolViolet, "#000000", 0.4);
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.rect(-6, -5, 12, 10);
        ctx.fill();
        ctx.stroke();
        ctx.beginPath();
        ctx.moveTo(-6, -1);
        ctx.lineTo(6, -1);
        ctx.stroke();
        ctx.beginPath();
        ctx.arc(0, -7.5, 2.4, Math.PI, Math.PI * 2);
        ctx.stroke();
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
        const sway = swayAt(now, 260, 0.35, reduced);
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
    const reduced = this.timeline.isReducedMotion();
    const bob =
      !reduced && (f.state === "walk" || f.state === "chase")
        ? Math.abs(Math.sin(now / 120)) * 2
        : 0;
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
    ctx.quadraticCurveTo(-14, -2 + swayAt(now, 400, 2, reduced), -13, -8);
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
      const hy = -16 - swayAt(now, 300, 1.5, reduced);
      ctx.beginPath();
      ctx.moveTo(0, hy + 2.4);
      ctx.bezierCurveTo(-4.2, hy - 1, -1.8, hy - 3.4, 0, hy - 1);
      ctx.bezierCurveTo(1.8, hy - 3.4, 4.2, hy - 1, 0, hy + 2.4);
      ctx.fill();
    }
    ctx.restore();
  }
}
