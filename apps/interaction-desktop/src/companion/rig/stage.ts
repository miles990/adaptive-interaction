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
  isCursorToy,
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
 * 身體姿勢仍然是遊玩中／休息中的姿勢（趴著＋核心顯示 Agent 工作中）。
 *
 * `ask`（requesting-consent，CPP floor 80）**不在**這裡：runtime 真的在等使用者
 * 確認時，舉手＋問號必須整個人演出來，遊玩場也要停——「在玩、頭飾亮一點」
 * 不是誠實的「在等你確認」（對抗審查 rig-renderer-011／companion-gameplay-001）。
 */
const OVERLAY_STATUS = new Set([
  "queued",
  "routing",
  "working",
  "thinking",
  "wait-codex",
  "wait-claude",
  "waiting",
  "listening",
]);

/**
 * 可以「一邊休息一邊亮核心」的身體姿勢（spec §6.2「趴著＋核心顯示 Agent 工作中」）：
 * 這些表情被工作/等待類狀態取代時，身體維持原姿勢，只疊狀態通道。
 */
export const REST_EXPRESSIONS = new Set(["lie-flat", "doze", "sleep", "sit", "quiet"]);

/**
 * 下一幀的「休息姿勢」：machine 動畫是休息表情 → 記住它；只借通道的工作/等待狀態 →
 * 保留上一個；其餘（idle、互動、安全與結果狀態）→ 清掉（她站起來了）。純函式。
 */
export function nextRestingExpression(prev: string | null, machineAnim: string): string | null {
  const id = EXPRESSION_ALIASES[machineAnim] ?? machineAnim;
  if (REST_EXPRESSIONS.has(id)) return id;
  if (statusOverlay(machineAnim) === "overlay") return prev;
  return null;
}

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
 *   - 安全/結果狀態（含 ask）一律整體搶佔（永遠不會被遊玩蓋掉）。
 *   - 工作/等待狀態只點亮核心/頭飾/裙擺/耳朵，身體照樣在玩；沒在玩但正趴著／
 *     打盹／端坐（`resting`）時，身體維持休息姿勢——「趴著＋核心亮」由此可達。
 */
export function stageExpressionPlan(
  machineAnim: string,
  mode: CharPlayMode,
  resting: string | null = null
): StagePlan {
  const overlay = statusOverlay(machineAnim);
  const play = playExpressionFor(mode);
  if (overlay === "takeover") {
    return { expression: machineAnim, useMachineSlice: true, statusChannels: null };
  }
  if (overlay === "none") {
    return play
      ? { expression: play, useMachineSlice: false, statusChannels: null }
      : { expression: machineAnim, useMachineSlice: true, statusChannels: null };
  }
  const body = play ?? (resting && REST_EXPRESSIONS.has(resting) ? resting : null);
  if (!body) return { expression: machineAnim, useMachineSlice: true, statusChannels: null };
  return {
    expression: body,
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
  /** 「游標靠近時看過來」（DesktopPrefs.companionApproach）：注視游標的唯一主人。 */
  approach: boolean;
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
  /** 暫停中（視窗隱藏／adapter suspend）：不排 rAF、不步進物理、不畫。 */
  private paused = false;
  /** loop 是否曾啟動（autoStart=false 的測試實例 resume 時不會偷跑）。 */
  private looping = false;

  private world: World;
  private machineAnim = "offline";
  private machineSlice: [number, number] | undefined;
  private flags: MachineFlags = { ambient: false, frozen: true, quiet: false, playPerforming: false };
  private toggles: StageToggles = { play: true, cursorPlay: true, deskMove: true, approach: true };
  /** 目前的休息姿勢（趴平／打盹／端坐…）：工作/等待狀態只疊通道、不把她拉起來。 */
  private resting: string | null = null;
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
  /** 主迴圈統計（診斷／效能量測）：rAF 回呼次數與真正畫出的幀數。 */
  private loopTicks = 0;
  private drawnFrames = 0;
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
    if (opts?.autoStart !== false) {
      this.looping = true;
      this.loop();
    }
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
    if (typeof cancelAnimationFrame === "function") cancelAnimationFrame(this.raf);
  }

  /**
   * 暫停主迴圈（CPP §7 hide／suspend）：取消 rAF、物理與粒子都停下，
   * 也不再回報互動框。狀態（世界、玩具、表情）原地保留，resume 後接續。
   */
  pause(): void {
    if (this.paused) return;
    this.paused = true;
    if (typeof cancelAnimationFrame === "function") cancelAnimationFrame(this.raf);
    this.raf = 0;
  }

  /** 恢復主迴圈；dt 從現在起算，不把暫停期間當成一大步物理。 */
  resume(): void {
    if (!this.paused || this.destroyed) return;
    this.paused = false;
    this.lastStep = this.now();
    if (this.looping) this.loop();
  }

  isPaused(): boolean {
    return this.paused;
  }

  /**
   * 啟動主迴圈（給 autoStart:false 建立的實例：效能量測要走真 rAF 迴圈＋幀預算，
   * 不能只直呼 renderFrame）。已啟動／已銷毀時 no-op；暫停中只記下要跑，resume 才排 rAF。
   */
  start(): void {
    if (this.destroyed || this.looping) return;
    this.looping = true;
    if (!this.paused) this.loop();
  }

  /** 主迴圈統計：rAF 回呼了幾次、真正畫了幾幀（降到 30fps 時 drawn ≈ ticks/2）。 */
  loopStats(): { ticks: number; drawn: number } {
    return { ticks: this.loopTicks, drawn: this.drawnFrames };
  }

  /** 換配色（角色 variant）；未知名稱維持原配色。 */
  setPalette(name: string): void {
    if (RIG_PALETTES[name]) this.paletteName = name;
  }

  currentPalette(): string {
    return this.paletteName;
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

  /** 目前的幀預算狀態（30fps 降級診斷用；avgMs＝上一窗的平均 renderFrame 成本）。 */
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

  /** 丟玩具進場。凍結（緊急停止／離線／暫停）時拒絕：停住的系統不生成懸空的玩具。 */
  spawnToy(kind: ToyKind): boolean {
    if (this.flags.frozen) return false;
    const before = this.world.toys.length;
    this.world = spawnToy(this.world, kind, this.now());
    return this.world.toys.length > before || isCursorToy(kind);
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
    return rollCall(this.world, this.charName, machineLabel, this.now(), { frozen: this.flags.frozen });
  }

  /** 診斷用：目前記住的休息姿勢（null＝站著／在玩）。 */
  restingExpression(): string | null {
    return this.resting;
  }

  /** 診斷用：角色目前朝向（1＝右、-1＝左）。 */
  charFacing(): 1 | -1 {
    return this.world.char.facing;
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

  /**
   * 互動範圍（角色＋可抓玩具的聯集，CSS px）：有玩具時游標不可穿透它們。
   *
   * 跟著游標走的光點／逗貓棒**不算**：它們永遠在游標底下，算進來的話整個
   * 角色↔游標的聯集矩形都吃掉點擊，桌面的空白區就不再穿透（對抗審查
   * companion-gameplay-004）。
   */
  interactiveBounds(): { x: number; y: number; w: number; h: number } {
    const r = this.charHitRect();
    const solid = this.world.toys.filter((t) => !isCursorToy(t.kind) || t.grabbed === "player");
    if (solid.length === 0) return r;
    const s = this.scale;
    let x0 = r.x;
    let y0 = r.y;
    let x1 = r.x + r.w;
    let y1 = r.y + r.h;
    for (const t of solid) {
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
    // 凍結時不抓玩具（角色本體仍可點：緊急停止的快捷選單要開得了）。
    const { world, toyId } = this.flags.frozen ? { world: this.world, toyId: null } : grabToyAt(this.world, x, y);
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
    if (this.draggingToy != null && this.flags.frozen) {
      // 拖到一半世界凍結了：玩具就地放下（零速度），不跟著游標懸在半空。
      this.dropDraggedToyInPlace();
      return;
    }
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
    if (this.draggingToy != null && this.flags.frozen) {
      this.dropDraggedToyInPlace();
      return;
    }
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

  /** 凍結時放開手上的玩具：零速度、不重設興趣／冷卻（解凍後不會突然飛出去被追）。 */
  private dropDraggedToyInPlace(): void {
    const id = this.draggingToy;
    if (id != null) {
      this.world = {
        ...this.world,
        toys: this.world.toys.map((t) => (t.id === id && t.grabbed === "player" ? { ...t, grabbed: null, vx: 0, vy: 0 } : t)),
      };
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
    if (this.destroyed || this.paused) return;
    if (typeof requestAnimationFrame !== "function") return;
    this.raf = requestAnimationFrame(this.loop);
    this.loopTicks += 1;
    this.frameParity = (this.frameParity + 1) % 2;
    if (!shouldDrawFrame(this.budget, this.frameParity)) return;
    // 幀預算（§14）：餵的是「這一幀真正花掉的繪製成本」（renderFrame 前後的時間差），
    // 不是 rAF 間隔——60Hz 螢幕的 rAF 間隔恆為 16.67ms > 12ms，拿它當幀時間會讓舞台
    // 在一秒後永久降到 30fps 且回不來（對抗審查 perf-claims-017）。最近 60 幀平均
    // 成本 >12ms 才每兩幀畫一次，<8ms 才回 60fps；跳過的幀不計入（沒有成本）。
    const start = this.now();
    this.renderFrame(start);
    this.budget = frameBudgetPolicy(this.budget, this.now() - start);
    this.drawnFrames += 1;
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
    // 工作/等待狀態只點亮核心/頭飾/裙擺/耳朵，身體維持遊玩或休息姿勢。
    // 一起身去玩（mode≠free）休息姿勢就作廢。
    this.resting = this.world.char.mode === "free" ? nextRestingExpression(this.resting, this.machineAnim) : null;
    const plan = stageExpressionPlan(this.machineAnim, this.world.char.mode, this.resting);
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

    // 注視：有使魔向她打招呼就回看，否則「游標靠近時看過來」（toggles.approach
    // 是這個行為唯一的主人；勿擾／安靜時段也不看）。
    // 只在「沒有整體搶佔」（idle 或只借通道的工作狀態）時疊，
    // 真相狀態與凍結狀態永遠不疊；Reduced Motion 不做這層。
    const takeover = statusOverlay(this.machineAnim) === "takeover";
    if (!reduced && !this.flags.frozen && !takeover) {
      const char = this.world.char;
      const greeter =
        char.attendTo !== null && now <= char.attendUntil
          ? this.world.familiars.find((f) => f.id === char.attendTo)
          : undefined;
      let dirWorld = 0;
      if (greeter) {
        dirWorld = Math.max(-1, Math.min(1, (greeter.x - char.x) / 60));
      } else if (this.toggles.approach && !this.flags.quiet) {
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
    // 真相狀態在台上（被擋下／失敗／未知…）或凍結時不畫：那不是賣萌的畫面。
    if (this.world.char.greetBackUntil > now && !takeover && !this.flags.frozen) {
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
    // 凍結（緊急停止／離線／暫停）與 Reduced Motion 一樣：羽毛不擺、小物件不轉。
    const reduced = this.timeline.isReducedMotion() || this.flags.frozen;
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
    // 凍結時使魔也停：不上下抖、尾巴不擺、愛心不浮（停住的系統不做任何表演）。
    const reduced = this.timeline.isReducedMotion() || this.flags.frozen;
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
    // 打招呼：愛心。真相狀態在台上或凍結時不畫（被擋下／緊急停止的畫面上沒有愛心）。
    if (f.state === "greet" && !this.flags.frozen && statusOverlay(this.machineAnim) !== "takeover") {
      this.drawGreetHeart(ctx, 0, -16, pal, now, reduced);
    }
    ctx.restore();
  }
}
