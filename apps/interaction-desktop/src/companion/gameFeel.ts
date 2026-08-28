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

export interface LandingPlan {
  landing: LandingKind;
  /** 要播的表情 id（全部非 truthState）。 */
  expression: string;
  durationMs: number;
}

/**
 * 依速度、高度與位置選落地方式：
 *   - 快或落差大 → 踉蹌（wobbly-landing）
 *   - 貼邊又有點速度 → 滑倒裝沒事（slip-play-cool）
 *   - 慢又低 → 輕巧落地（land-light）
 *   - 其餘 → 站穩（不加演出）
 */
export function pickLanding(input: LandingInput): LandingPlan {
  const speed = Number.isFinite(input.speedPxPerSec) ? Math.max(0, input.speedPxPerSec) : 0;
  const height = Number.isFinite(input.heightPx) ? Math.max(0, input.heightPx) : 0;
  if (speed > 900 || height > 260) {
    return { landing: "wobbly", expression: "wobbly-landing", durationMs: 1600 };
  }
  if (input.nearEdge && speed > 350) {
    return { landing: "slip", expression: "slip-play-cool", durationMs: 1800 };
  }
  if (speed < 120 && height < 60) {
    return { landing: "light", expression: "land-light", durationMs: 900 };
  }
  return { landing: "steady", expression: "idle", durationMs: 0 };
}

// ---------------------------------------------------------------------------
// 幀預算（§14：60fps 目標，低效能裝置允許 30fps 降級）
// ---------------------------------------------------------------------------

/** 平均幀時間超過這個值就降到 30fps。 */
export const FRAME_DEGRADE_MS = 12;
/** 平均幀時間低於這個值才回到 60fps（遲滯，避免抖動）。 */
export const FRAME_RECOVER_MS = 8;
/** 評估窗大小（最近 N 幀）。 */
export const FRAME_WINDOW = 60;

export interface FrameBudgetState {
  /** 目前窗內已累積的幀數。 */
  count: number;
  /** 目前窗內的幀時間總和（ms）。 */
  sumMs: number;
  /** 上一個完整窗的平均幀時間（ms；還沒有窗時為 0）。 */
  avgMs: number;
  /** true＝每兩幀才畫一次（30fps 降級）。 */
  skipEveryOther: boolean;
}

export function initialFrameBudget(): FrameBudgetState {
  return { count: 0, sumMs: 0, avgMs: 0, skipEveryOther: false };
}

/**
 * 每幀呼叫一次。滿一個窗（60 幀）才決策，且有遲滯：
 * >12ms 平均 → 降級；降級後要 <8ms 才回到 60fps。
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

/** 這一幀該不該畫（降級時每兩幀畫一次）。 */
export function shouldDrawFrame(state: FrameBudgetState, frameParity: number): boolean {
  return !state.skipEveryOther || frameParity % 2 === 0;
}
