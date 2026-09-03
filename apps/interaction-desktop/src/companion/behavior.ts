// Behavior Runtime（spec §5）— 純本機、確定性，絕不使用生成式 AI 逐幀控制。
//
// 三層：
//   生命底層：微動作排程（眨眼與各種閒置小動作）——**不在本模組**：執行期由
//     InteractionDirector.tick() 的 ambient 變體池（director.ts＋角色 tables 的
//     AmbientVariant）負責排程，眨眼由 rig 時間軸的 autoBlink 自己跑。本模組
//     曾經有一份同構的 scheduleMicroAction，但生產路徑從來沒有呼叫它，只有測試
//     在跑——已移除，不留一個執行期不存在的分層（對抗審查 companion-gameplay-036
//     ／rig-renderer-060）。
//   行為層：注意力與選擇（Utility AI 評分＋優先階梯＋反重複）——本模組
//   語意層：AI 只能經 runtime 驗證的 behaviorIntent 提出高層意圖（presentation.rs）
//
// 原則：
//   - BehaviorState 平滑變化（指數趨近），不可因單一事件 0→1。
//   - 隨機受情境控制且可注入（seeded RNG）；不得每固定 N 秒播同一動畫。
//   - 微動作只在 ambient（idle 基態且無 transient）時發生；Reduced Motion
//     只留眨眼；quiet/paused/emergency 完全停止玩鬧。
//   - 「眼先動、頭後轉」的反應鏈烘焙在 notice/curious 動畫時間軸內。
//   - familiarity 只影響呈現，永不影響權限。

export interface BehaviorState {
  /** 0..1 整體喚起度（事件推高、隨時間回落） */
  activation: number;
  /** 0..1 注意力集中度 */
  attention: number;
  /** 0..1 任務負載（有進行中的動作/等待時 > 0） */
  taskLoad: number;
  /** 0..1 使用者互動就緒度（最近互動越多越高） */
  interactionReadiness: number;
  /** 0..1 熟悉度（只影響呈現：微動作更放鬆多樣） */
  familiarity: number;
  /** 最近被打斷次數（衰減計數；高 → 降低主動表現） */
  recentInterruptions: number;
  /** 目前注意的語意焦點（不含座標） */
  currentFocus: string | null;
  /** 上次使用者互動時間（ms epoch） */
  lastInteractionAt: number;
}

export function initialBehavior(nowMs: number): BehaviorState {
  return {
    activation: 0.2,
    attention: 0.2,
    taskLoad: 0,
    interactionReadiness: 0.3,
    familiarity: 0,
    recentInterruptions: 0,
    currentFocus: null,
    lastInteractionAt: nowMs,
  };
}

/** 每步趨近目標的比例（500ms tick 下 ~2.5 秒收斂大半）。 */
const SMOOTH = 0.18;
const approach = (v: number, target: number, k = SMOOTH) => v + (target - v) * k;
const clamp01 = (v: number) => Math.max(0, Math.min(1, v));

export interface BehaviorInputs {
  /** 目前是否有進行中任務（transient acting/waiting 等） */
  busy: boolean;
  /** 是否有等待人類確認的事項 */
  waitingForHuman: boolean;
  /** 距上次使用者互動的毫秒數 */
  msSinceInteraction: number;
}

/** 平滑步進：每 tick 呼叫一次（500ms）。 */
export function stepBehavior(s: BehaviorState, inputs: BehaviorInputs): BehaviorState {
  const idleMin = inputs.msSinceInteraction / 60_000;
  return {
    ...s,
    // 喚起度朝「忙碌/等待=高、閒置越久越低」趨近。
    activation: clamp01(
      approach(s.activation, inputs.busy || inputs.waitingForHuman ? 0.85 : Math.max(0.1, 0.4 - idleMin * 0.05))
    ),
    attention: clamp01(approach(s.attention, inputs.busy ? 0.9 : inputs.waitingForHuman ? 0.7 : 0.25)),
    taskLoad: clamp01(approach(s.taskLoad, inputs.busy ? 1 : 0)),
    interactionReadiness: clamp01(
      approach(s.interactionReadiness, inputs.msSinceInteraction < 120_000 ? 0.8 : 0.3, 0.1)
    ),
    recentInterruptions: Math.max(0, s.recentInterruptions - 0.02),
  };
}

/** 事件登記：推高狀態並記焦點（單一事件推targets，不直接設滿）。 */
export function noteEvent(s: BehaviorState, focus: string, importance: number): BehaviorState {
  return {
    ...s,
    activation: clamp01(s.activation + importance * 0.35),
    attention: clamp01(s.attention + importance * 0.4),
    currentFocus: focus,
  };
}

export function noteUserInteraction(s: BehaviorState, nowMs: number): BehaviorState {
  return {
    ...s,
    lastInteractionAt: nowMs,
    interactionReadiness: clamp01(s.interactionReadiness + 0.2),
    familiarity: clamp01(s.familiarity + 0.01),
  };
}

export function noteInterruption(s: BehaviorState): BehaviorState {
  return { ...s, recentInterruptions: Math.min(5, s.recentInterruptions + 1) };
}

/** 生命底層的程序化疊加量。只表達局部視線／耳注意力，不含游標座標。 */
export interface LayeredMicroMotion {
  gazeX: number;
  gazeY: number;
  earBias: number;
  intensity: number;
}

/**
 * 非生成式、連續且有界的視線／耳朵微動。相位只取本機時間與 Behavior
 * State；不讀取、不保存完整系統游標軌跡。Reduced Motion 與安全凍結狀態
 * 回到零位。多個非整數週期疊加，避免固定 N 秒重播同一姿態。
 */
export function layeredMicroMotion(
  s: BehaviorState,
  nowMs: number,
  reducedMotion: boolean,
  frozen: boolean
): LayeredMicroMotion {
  if (reducedMotion || frozen) {
    return { gazeX: 0, gazeY: 0, earBias: 0, intensity: 0 };
  }
  const idleFreedom = clamp01(1 - s.taskLoad) * clamp01(1 - s.recentInterruptions * 0.12);
  const focus = clamp01(s.attention);
  // 專注時幅度縮小但不完全僵住；閒置時才有較寬的掃視。
  const gazeAmplitude = (0.18 + (1 - focus) * 0.62) * idleFreedom;
  const gazeX = Math.sin(nowMs / 1730) * 0.72 + Math.sin(nowMs / 4190) * 0.28;
  const gazeY = Math.sin(nowMs / 2710 + 0.9) * 0.65 + Math.sin(nowMs / 6130) * 0.2;
  // 耳朵比視線慢，形成重量與延遲；只呈現注意力，不指向精準座標。
  const earBias = Math.sin(nowMs / 3370 + 0.45) * (0.25 + s.activation * 0.55) * idleFreedom;
  return {
    gazeX: Math.max(-1, Math.min(1, gazeX * gazeAmplitude)),
    gazeY: Math.max(-1, Math.min(1, gazeY * gazeAmplitude)),
    earBias: Math.max(-1, Math.min(1, earBias)),
    intensity: clamp01(0.25 + s.activation * 0.45 + s.interactionReadiness * 0.2),
  };
}

// ---------------------------------------------------------------------------
// 事件優先階梯（spec §5.4）：分數高者先。Emergency 不經此路（機器基態處理）。
// ---------------------------------------------------------------------------

export type EventClass =
  | "emergency"
  | "sensor-safety"
  | "waiting-confirmation"
  | "direct-interaction"
  | "task-state"
  | "suggestion"
  | "world-event"
  | "ambient";

const CLASS_BASE: Record<EventClass, number> = {
  emergency: 100,
  "sensor-safety": 90,
  "waiting-confirmation": 80,
  "direct-interaction": 70,
  "task-state": 55,
  suggestion: 35,
  "world-event": 20,
  ambient: 5,
};

export interface EventScoreContext {
  /** 同類事件最近 10 分鐘出現次數（重複懲罰） */
  recentSameClass: number;
  /** 是否已回應過同一事件 */
  alreadyResponded: boolean;
  /** 目前動作是否可中斷（高優先動畫進行中=false） */
  interruptible: boolean;
  /** 勿擾（quiet hours / 使用者要求安靜） */
  doNotDisturb: boolean;
  /** 與目前任務的相關性 0..1 */
  relevance: number;
  /** 新鮮度 0..1（第一次見=1） */
  novelty: number;
}

/**
 * Utility 評分：<=0 代表不值得回應。安全類不受勿擾與重複懲罰壓到 0 以下。
 *
 * 誠實標註（對抗審查 director-pipeline-020）：這個函式目前**沒有**接進任何執行期
 * 決策。machine.ts 的同優先平手判定曾經呼叫它，但四個懲罰維度都被硬編成停用值，
 * 結果恆為 "replace"——那是一條假裝接上的管線，已經移除。要真的用起來，呼叫端
 * 必須餵進真實的 recentSameClass／alreadyResponded／interruptible／doNotDisturb。
 */
export function scoreEvent(cls: EventClass, ctx: EventScoreContext): number {
  let score = CLASS_BASE[cls];
  score += ctx.relevance * 10 + ctx.novelty * 8;
  score -= Math.min(3, ctx.recentSameClass) * 8; // 重複懲罰
  if (ctx.alreadyResponded) score -= 40;
  if (!ctx.interruptible) score -= 25;
  if (ctx.doNotDisturb) score -= 30; // 打擾成本
  const isSafety = cls === "emergency" || cls === "sensor-safety" || cls === "waiting-confirmation";
  if (isSafety) return Math.max(CLASS_BASE[cls] * 0.5, score); // 安全事件永不歸零
  return score;
}

/** 可注入的確定性 RNG（mulberry32）——測試與重現用。 */
export function seededRng(seed: number): () => number {
  let t = seed >>> 0;
  return () => {
    t += 0x6d2b79f5;
    let r = Math.imul(t ^ (t >>> 15), 1 | t);
    r = (r + Math.imul(r ^ (r >>> 7), 61 | r)) ^ r;
    return ((r ^ (r >>> 14)) >>> 0) / 4294967296;
  };
}
