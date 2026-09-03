// Game Feel 與效能的純決策（spec §5.2 放下四種落地、§14 效能）。
//
// 這裡只有純函式：輸入是可量測的數字，輸出是「該演哪一段」或「該不該
// 少畫一幀」。沒有 canvas、沒有 I/O，可以逐幀測。

// ---------------------------------------------------------------------------
// 放下角色時的四種落地（§5.2）
// ---------------------------------------------------------------------------

export type LandingKind = "steady" | "light" | "wobbly" | "slip";

export interface LandingInput {
  /**
   * 放手瞬間的估算速度（px/s）。原生視窗拖曳期間沒有指標事件，
   * 呼叫端用「整段拖曳位移 ÷ 耗時」估算——這是平均速度，屬於下界估算，
   * 不是真正的瞬時速度（誠實標註，不宣稱精確）。
   */
  speedPxPerSec: number;
  /** 這段拖曳的垂直下降量（px；往上移動視為 0）。 */
  heightPx: number;
  /** 落點是否貼近螢幕/視窗邊緣。 */
  nearEdge: boolean;
}

/** 某種落地要演的美術（角色 adapter 提供；本模組不認識任何表情 id）。 */
export interface LandingArt {
  /** 表情 id（必須非 truthState——由角色 tables 保證）。 */
  expression: string;
  durationMs: number;
}

/** 落地種類 → 美術；沒有對應者＝站穩、不加演出。 */
export type LandingTable = Readonly<Partial<Record<Exclude<LandingKind, "steady">, LandingArt>>>;

export interface LandingPlan {
  landing: LandingKind;
  /** 要播的表情 id；null＝這個角色對這種落地沒有演出（呼叫端不送 transient）。 */
  expression: string | null;
  durationMs: number;
}

/**
 * 依速度、高度與位置選落地方式（純判定；美術由 `art` 注入）：
 *   - 快或落差大 → 踉蹌（wobbly）
 *   - 貼邊又有點速度 → 滑倒裝沒事（slip）
 *   - 慢又低 → 輕巧落地（light）
 *   - 其餘 → 站穩（不加演出）
 */
export function pickLanding(input: LandingInput, art: LandingTable = {}): LandingPlan {
  const speed = Number.isFinite(input.speedPxPerSec) ? Math.max(0, input.speedPxPerSec) : 0;
  const height = Number.isFinite(input.heightPx) ? Math.max(0, input.heightPx) : 0;
  const plan = (landing: Exclude<LandingKind, "steady">): LandingPlan => {
    const a = art[landing];
    return a
      ? { landing, expression: a.expression, durationMs: Math.max(0, a.durationMs) }
      : { landing, expression: null, durationMs: 0 };
  };
  if (speed > 900 || height > 260) return plan("wobbly");
  if (input.nearEdge && speed > 350) return plan("slip");
  if (speed < 120 && height < 60) return plan("light");
  return { landing: "steady", expression: null, durationMs: 0 };
}

// ---------------------------------------------------------------------------
// 幀預算（§14：60fps 目標，低效能裝置允許 30fps 降級）
//
// 輸入是「一幀真正花掉的繪製成本」（renderFrame 前後 performance.now() 的差），
// **不是** rAF 回呼之間的間隔：60Hz 螢幕的 rAF 間隔恆為 16.67ms，拿它當幀時間會
// 讓任何一台正常機器在一秒後永久降到 30fps 且永遠回不來（對抗審查 perf-claims-017）。
// ---------------------------------------------------------------------------

/** 平均繪製成本超過這個值就降到 30fps。 */
export const FRAME_DEGRADE_MS = 12;
/** 平均繪製成本低於這個值才回到 60fps（遲滯，避免抖動）。 */
export const FRAME_RECOVER_MS = 8;
/** 評估窗大小（最近 N 幀）。 */
export const FRAME_WINDOW = 60;

export interface FrameBudgetState {
  /** 目前窗內已累積的幀數。 */
  count: number;
  /** 目前窗內的繪製成本總和（ms）。 */
  sumMs: number;
  /** 上一個完整窗的平均繪製成本（ms；還沒有窗時為 0）。 */
  avgMs: number;
  /** true＝每兩幀才畫一次（30fps 降級）。 */
  skipEveryOther: boolean;
}

export function initialFrameBudget(): FrameBudgetState {
  return { count: 0, sumMs: 0, avgMs: 0, skipEveryOther: false };
}

/**
 * 每畫一幀呼叫一次，`frameMs` 是那一幀的繪製成本（不是 rAF 間隔）。
 * 滿一個窗（60 幀）才決策，且有遲滯：>12ms 平均 → 降級；降級後要 <8ms 才回到 60fps。
 */
export function frameBudgetPolicy(state: FrameBudgetState, frameMs: number): FrameBudgetState {
  const ms = Number.isFinite(frameMs) ? Math.max(0, Math.min(1_000, frameMs)) : 0;
  const count = state.count + 1;
  const sumMs = state.sumMs + ms;
  if (count < FRAME_WINDOW) {
    return { count, sumMs, avgMs: state.avgMs, skipEveryOther: state.skipEveryOther };
  }
  const avgMs = sumMs / count;
  const skipEveryOther = state.skipEveryOther
    ? avgMs >= FRAME_RECOVER_MS // 降級中：夠快了才回 60fps
    : avgMs > FRAME_DEGRADE_MS; // 全速中：太慢才降級
  return { count: 0, sumMs: 0, avgMs, skipEveryOther };
}

// ---------------------------------------------------------------------------
// 幀節奏（§14 的另一半）：真正掉幀的原因多半不在 JS
//
// frameBudgetPolicy 量的是 renderFrame 的 JS 成本，實測中位數 0.24ms、門檻 12ms
// ——約 50 倍餘裕，實務上不可能觸發。但使用者真正會掉幀的情境（透明常駐視窗的
// GPU 合成貴、系統節流 rAF、別的行程搶 GPU）完全不反映在 JS 成本上
// （對抗審查 perf-claims-008）。所以另外量「rAF 回呼之間的實際間隔」，並且跟
// **這台螢幕自己的基準**（觀察到的最快間隔：60Hz≈16.7ms、120Hz≈8.3ms）相比，
// 而不是拿絕對毫秒數當門檻——那正是 perf-claims-017 的錯法。
// ---------------------------------------------------------------------------

/** 窗內平均幀距超過螢幕基準的這個倍數就降級。 */
export const FRAME_PACING_DEGRADE_RATIO = 1.5;
/** 降級後要低於這個倍數才回全速（遲滯，避免抖動）。 */
export const FRAME_PACING_RECOVER_RATIO = 1.15;
/** 可以當「螢幕基準」的間隔範圍（ms）：太短是計時雜訊，太長不是刷新率。 */
export const FRAME_GAP_MIN_MS = 4;
export const FRAME_GAP_MAX_MS = 40;
/** 單一樣本上限：一次 GC／系統暫停不該把整窗平均拉爆。 */
export const FRAME_GAP_CAP_MS = 200;

export interface FramePacingState {
  /** 目前窗內已累積的樣本數。 */
  count: number;
  /** 目前窗內的間隔總和（ms）。 */
  sumMs: number;
  /** 這台螢幕的基準幀距（觀察到的最快間隔；0＝還沒有可信樣本）。 */
  baselineMs: number;
  /** 上一個完整窗的平均幀距（ms）。 */
  avgGapMs: number;
  /** true＝節奏跟不上螢幕（掉幀），該降到 30fps。 */
  missing: boolean;
}

export function initialFramePacing(): FramePacingState {
  return { count: 0, sumMs: 0, baselineMs: 0, avgGapMs: 0, missing: false };
}

/**
 * 每次 rAF 回呼呼叫一次，`gapMs` 是距離上一次回呼的實際間隔。
 * 滿一個窗（60 次）才決策；沒有可信基準前永不降級。
 */
export function framePacingPolicy(state: FramePacingState, gapMs: number): FramePacingState {
  const raw = Number.isFinite(gapMs) ? Math.max(0, Math.min(FRAME_GAP_CAP_MS, gapMs)) : 0;
  const baselineMs =
    raw >= FRAME_GAP_MIN_MS && raw <= FRAME_GAP_MAX_MS
      ? state.baselineMs === 0
        ? raw
        : Math.min(state.baselineMs, raw)
      : state.baselineMs;
  const count = state.count + 1;
  const sumMs = state.sumMs + raw;
  if (count < FRAME_WINDOW) {
    return { ...state, count, sumMs, baselineMs };
  }
  const avgGapMs = sumMs / count;
  const missing =
    baselineMs === 0
      ? false
      : state.missing
        ? avgGapMs > baselineMs * FRAME_PACING_RECOVER_RATIO
        : avgGapMs > baselineMs * FRAME_PACING_DEGRADE_RATIO;
  return { count: 0, sumMs: 0, baselineMs, avgGapMs, missing };
}

/**
 * 這一幀該不該畫（降級時每兩幀畫一次）。
 *
 * 兩條降級訊號取聯集：JS 繪製成本（frameBudgetPolicy）與實際幀節奏
 * （framePacingPolicy）。後者才抓得到合成／GPU／系統節流造成的掉幀。
 */
export function shouldDrawFrame(
  state: FrameBudgetState,
  frameParity: number,
  pacing?: FramePacingState
): boolean {
  const degraded = state.skipEveryOther || pacing?.missing === true;
  return !degraded || frameParity % 2 === 0;
}

// ---------------------------------------------------------------------------
// 點擊反應（§5.2 高頻反應：變體＋冷卻，走 Director）
// ---------------------------------------------------------------------------

export interface ClickReactionPlan {
  /** rapid＝連戳反應；single＝Director 挑的單擊變體；fallback＝沒有角色表／全在冷卻：canonical clicked。 */
  kind: "rapid" | "single" | "fallback";
  /** 要套的 machine transient：連戳是 performing（先清場），單擊是 clicked（優先 55，帶變體動畫）。 */
  transientKind: "performing" | "clicked";
  animation?: string;
  durationMs?: number;
  /** Director 為什麼沒給反應（fallback 時）。 */
  reason?: string;
  /** 這次點擊是否切換快捷選單（連戳反應不開選單）。 */
  toggleMenu: boolean;
}

/** Director 的最小介面（避免 gameFeel 依賴整個 Director 類別）。 */
export interface ReactionSource {
  reactDetailed(
    intent: string,
    nowMs: number,
    durationMs?: number,
    rng?: () => number,
    opts?: { cooldownMs?: number }
  ): { action: { expression: string; durationMs: number } | null; reason: string };
}

/**
 * 單擊／連戳都經 Director：
 *   - 1.4 秒內第 3 次以上 → `poked-rapid` 的變體池（≥3，防重複，短冷卻）；全在冷卻或
 *     沒有角色表 → **退回一般單擊**（不是什麼都不做：連戳冷卻期的點擊仍要有反應、
 *     仍要開選單）。
 *   - 單擊 → `poked` 的變體池（≥3，防重複，短冷卻）；全在冷卻／文字角色 → canonical
 *     `clicked`（renderer alias → poked），保留直接互動的優先階梯（clicked 55）。
 */
export function planClickReaction(input: {
  rapid: boolean;
  nowMs: number;
  director: ReactionSource;
  rng: () => number;
  singleCooldownMs?: number;
  /** 連戳變體的冷卻（角色表注入；省略＝2.5 秒，讓變體池真的輪得動）。 */
  rapidCooldownMs?: number;
}): ClickReactionPlan {
  if (input.rapid) {
    const d = input.director.reactDetailed("poked-rapid", input.nowMs, 2_200, input.rng, {
      cooldownMs: input.rapidCooldownMs ?? 2_500,
    });
    if (d.action) {
      return {
        kind: "rapid",
        transientKind: "performing",
        animation: d.action.expression,
        durationMs: d.action.durationMs,
        toggleMenu: false,
      };
    }
  }
  const d = input.director.reactDetailed("poked", input.nowMs, 700, input.rng, {
    cooldownMs: input.singleCooldownMs ?? 1_200,
  });
  if (d.action) {
    return {
      kind: "single",
      transientKind: "clicked",
      animation: d.action.expression,
      durationMs: d.action.durationMs,
      toggleMenu: true,
    };
  }
  return { kind: "fallback", transientKind: "clicked", reason: d.reason, toggleMenu: true };
}
