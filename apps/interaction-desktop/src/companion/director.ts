// Interaction Director（spec §6）：統一的行為導演。
//
//   事件 → Event Normalizer → Attention → Utility Scoring → Behavior Intent
//   → Action Scheduler →（machine transient / 表情通道）
//
// 誠實說明 Utility Scoring 這一段目前落在哪裡（對抗審查 director-pipeline-046）：
// 它**不在** Director 的決策裡。Director 自己的節流是 hazard 抽樣＋冷卻＋防重複
// （tick／reactDetailed）；behavior.ts 的 scoreEvent 只被 machine.ts 用在「同優先
// transient 的平手判定」。Director 上曾有一個 `score()` 純轉呼包裝，整個 repo
// 沒有任何呼叫端，已移除——不留一個假裝管線接上了的入口。
//
// 本模組純確定性（seeded RNG 注入），不呼叫 AI。它擁有「生活層」的
// 選擇權：ambient 變體、冷卻、防重複、中斷後恢復；真相狀態（成功/失敗/
// 阻擋/未知/緊急）永遠由 machine.ts 的 runtime 事件／CPP intent 驅動，
// Director 不可也不會排程 truthState 表情（schedule 端過濾＋測試釘死）。
//
// Engine-neutral（CPP）：Director 不認識任何角色的表情 id。哪些表情可播
// （isPlayable）、ambient 變體池、反應表、睡眠類集合、眨眼表情，全部由角色
// adapter 的 DirectorTables 注入（例如 character/adapters/shuTables.ts）；
// 沒注入時（文字角色）Director 永遠不出手。

import { BehaviorState } from "./behavior";
import { DEFAULT_TUNING, PersonalityTuning } from "./personality";

export interface DirectorContext {
  nowMs: number;
  /** machine 是否 ambient（idle 基態且無 transient）。 */
  ambient: boolean;
  /** quiet hours / 使用者要求安靜。 */
  quiet: boolean;
  reducedMotion: boolean;
  /** 表現度：quiet 0.5 / natural 1 / lively 1.5。 */
  expressiveness: number;
  msSinceInteraction: number;
  behavior: BehaviorState;
}

export interface DirectorAction {
  /** 表情 id（rig）／動畫名（sprite fallback 由 renderer 鏈處理）。 */
  expression: string;
  durationMs: number;
  /**
   * 這個動作是哪一層排出來的。`blink` 是安靜時段唯一允許的「就地眨眼」——
   * 呼叫端要靠這個標記認出它，不能去比對表情 id：眨眼的 id 由角色 adapter
   * 的 DirectorTables 注入，host 不該知道它叫什麼（對抗審查
   * director-pipeline-045；CLAUDE.md「頁面不得引用角色表情名」）。
   */
  source: "ambient" | "reaction" | "resume" | "blink";
}

/** ambient 變體池：全部非 truthState（由角色 tables 的 isPlayable 把關）。 */
export interface AmbientVariant {
  expression: string;
  durationMs: number;
  weight: number;
  /** 需要的最低放鬆度（0..1；越大越要真的沒事才出現）。 */
  minRelax: number;
  reducedMotionOk: boolean;
  cooldownMs: number;
}

/** 角色 adapter 注入給 Director 的表（純資料＋一個白名單函式）。 */
export interface DirectorTables {
  /** 真相狀態防線：只有回 true 的表情才可能被排程。 */
  isPlayable: (expression: string) => boolean;
  ambient: readonly AmbientVariant[];
  /**
   * 反應意圖 → 表情（玩家/事件反應層；仍非 truthState）。
   * 一個意圖可以有多個變體（spec §5.2：高頻反應 3～6 個變體＋防重複＋冷卻）：
   * Director 依冷卻與「上一次用的」挑一個不同的。
   */
  reactions: Readonly<Record<string, string | readonly string[]>>;
  /** 可以「假裝沒看到」的低優先意圖（安全/確認類永遠不會被忽略）。 */
  softIntents: readonly string[];
  /** 「假裝沒聽見」的表情（null＝這個角色沒有這種反應）。 */
  pretendNotHear: string | null;
  /** 睡眠類長 ambient：被互動打斷後不該原樣睡回去。 */
  sleepy: ReadonlySet<string>;
  /** 安靜時唯一允許的「就地眨眼」（null＝安靜時完全不動）。 */
  blink: { expression: string; durationMs: number } | null;
}

/** 沒有角色 tables 時的預設：什麼都不可播、什麼都不排。 */
export const EMPTY_DIRECTOR_TABLES: DirectorTables = {
  isPlayable: () => false,
  ambient: [],
  reactions: {},
  softIntents: [],
  pretendNotHear: null,
  sleepy: new Set<string>(),
  blink: null,
};

/** 剛被互動多久之內不恢復睡眠類 ambient（改回 idle 系）。 */
export const SLEEP_RESUME_BLOCK_MS = 20_000;

/**
 * 這一 tick 該不該讓 Director 出手，以及是不是「安靜模式」。
 *
 * quiet 基態的 pose 是 `{ animation: "quiet", ambient: false }`——呼叫端如果
 * 用 `if (!pose.ambient) return` 當閘門，Director 的 quiet 分支（偶爾眨眼）
 * 永遠到不了。安靜不等於完全靜止：仍然 tick，只是只允許眨眼類。
 */
export function directorTickGate(input: {
  poseAmbient: boolean;
  base: string;
  hasActiveTransient: boolean;
  /** 使用者要求的本機安靜期（「一小時內不要主動說話」）。 */
  localQuiet?: boolean;
}): { tick: boolean; quiet: boolean } {
  if (input.hasActiveTransient) return { tick: false, quiet: false };
  const quiet = input.base === "quiet" || input.localQuiet === true;
  if (input.poseAmbient) return { tick: true, quiet };
  if (input.base === "quiet") return { tick: true, quiet: true };
  return { tick: false, quiet: false };
}

interface InterruptedAction {
  action: DirectorAction;
  remainingMs: number;
  /** 過了這個時間就不再恢復（情境已變）。 */
  expiresAt: number;
}

/** react() 回 null 的原因：不是靜默失敗，呼叫端可據此退回別的反應（例如一般點擊）。 */
export type ReactReason =
  | "ok"
  | "pretend-not-hear"
  | "no-mapping"
  | "not-playable"
  | "cooldown";

export interface ReactDecision {
  action: DirectorAction | null;
  reason: ReactReason;
  intent: string;
  atMs: number;
}

/** 反應的預設冷卻（同一表情 8 秒內不重播）。 */
export const REACTION_COOLDOWN_MS = 8_000;
/** 最近幾筆決定留著給診斷／Roll Call（有界）。 */
const DECISION_LOG_LIMIT = 16;

const clamp01 = (v: number) => Math.max(0, Math.min(1, v));

export class InteractionDirector {
  private cooldownUntil = new Map<string, number>();
  private recent: string[] = [];
  private interrupted: InterruptedAction | null = null;
  private currentAction: { action: DirectorAction; startedAt: number } | null = null;
  private tuning: PersonalityTuning;
  private tables: DirectorTables;
  /** 每個意圖上一次用的變體（防重複）。 */
  private lastVariant = new Map<string, string>();
  private decisions: ReactDecision[] = [];

  constructor(tuning: PersonalityTuning = DEFAULT_TUNING, tables: DirectorTables = EMPTY_DIRECTOR_TABLES) {
    this.tuning = tuning;
    this.tables = tables;
  }

  /** 個性 tuning（冷卻倍率、變體權重、假裝沒看到的機率）。 */
  setTuning(tuning: PersonalityTuning): void {
    this.tuning = tuning;
  }

  /** 換角色時換表（冷卻與恢復計畫一併清掉——那是上一個角色的）。 */
  setTables(tables: DirectorTables): void {
    this.tables = tables;
    this.cooldownUntil.clear();
    this.recent = [];
    this.interrupted = null;
    this.currentAction = null;
    this.lastVariant.clear();
  }

  /** 最近一次 react() 的決定（含回 null 的原因）；沒有就是 null。 */
  lastDecision(): ReactDecision | null {
    return this.decisions.length > 0 ? this.decisions[this.decisions.length - 1] : null;
  }

  /** 最近的 react() 決定（有界，最新在最後）。 */
  recentDecisions(): readonly ReactDecision[] {
    return this.decisions;
  }

  /** 真相狀態防線：Director 永不排程 truthState 表情（白名單由角色 tables 提供）。 */
  private playable(expression: string): boolean {
    try {
      return this.tables.isPlayable(expression) === true;
    } catch {
      return false;
    }
  }

  /** 目前動作被真實事件搶佔：記下來，之後可恢復。 */
  notePreempted(nowMs: number): void {
    if (!this.currentAction) return;
    const { action, startedAt } = this.currentAction;
    const elapsed = nowMs - startedAt;
    const remaining = action.durationMs - elapsed;
    // 只有夠長的動作才值得恢復（短反應直接放棄）。
    if (remaining > 1_500 && action.durationMs >= 4_000) {
      this.interrupted = {
        action: { ...action, source: "resume" },
        remainingMs: remaining,
        expiresAt: nowMs + 20_000,
      };
    }
    this.currentAction = null;
  }

  /** 動作自然結束（transient 到期）。 */
  noteFinished(): void {
    this.currentAction = null;
  }

  /**
   * 使用者/L1 意圖的即時反應。真相狀態永遠不可點播（playable 白名單），
   * 冷卻中回 null（不重播）。給了 rng 時，俏皮的個性偶爾會「假裝沒看到」
   * ——那也是一個誠實的反應，不是靜默失敗。
   *
   * 回 null 時原因記在 lastDecision()（no-mapping／not-playable／cooldown），
   * 呼叫端據此退回別的反應，而不是什麼都不做。
   */
  react(
    intent: string,
    nowMs: number,
    durationMs = 2_500,
    rng?: () => number,
    opts: { cooldownMs?: number } = {}
  ): DirectorAction | null {
    return this.reactDetailed(intent, nowMs, durationMs, rng, opts).action;
  }

  /** react() 的完整決定：action 與原因。 */
  reactDetailed(
    intent: string,
    nowMs: number,
    durationMs = 2_500,
    rng?: () => number,
    opts: { cooldownMs?: number } = {}
  ): ReactDecision {
    const decide = (action: DirectorAction | null, reason: ReactReason): ReactDecision => {
      const d: ReactDecision = { action, reason, intent, atMs: nowMs };
      this.decisions = [...this.decisions.slice(-(DECISION_LOG_LIMIT - 1)), d];
      return d;
    };
    const mapped = this.tables.reactions[intent];
    const variants = (Array.isArray(mapped) ? mapped : mapped ? [mapped] : []) as readonly string[];
    if (variants.length === 0) return decide(null, "no-mapping");
    const playable = variants.filter((v) => this.playable(v));
    if (playable.length === 0) return decide(null, "not-playable");
    const ready = playable.filter((v) => (this.cooldownUntil.get(v) ?? 0) <= nowMs);
    if (ready.length === 0) return decide(null, "cooldown");
    // 防重複：有別的變體可選時，不連續用同一個。
    const last = this.lastVariant.get(intent);
    const pool = ready.length > 1 && last ? ready.filter((v) => v !== last) : ready;
    const pick = rng ? Math.min(pool.length - 1, Math.floor(Math.max(0, Math.min(0.999999, rng())) * pool.length)) : 0;
    let expression = pool[pick];
    let reason: ReactReason = "ok";
    const pretend = this.tables.pretendNotHear;
    if (
      rng &&
      pretend &&
      this.tables.softIntents.includes(intent) &&
      rng() < this.tuning.pretendNotSeeChance
    ) {
      if (this.playable(pretend) && (this.cooldownUntil.get(pretend) ?? 0) <= nowMs) {
        expression = pretend;
        reason = "pretend-not-hear";
      }
    }
    const action: DirectorAction = { expression, durationMs, source: "reaction" };
    this.currentAction = { action, startedAt: nowMs };
    this.interrupted = null; // 新反應取消舊的恢復計畫
    this.lastVariant.set(intent, expression);
    this.markUsed(expression, nowMs, opts.cooldownMs ?? REACTION_COOLDOWN_MS);
    return decide(action, reason);
  }

  /** 每 tick 呼叫（~500ms）。回傳要演的動作或 null。 */
  tick(ctx: DirectorContext, rng: () => number): DirectorAction | null {
    if (!ctx.ambient) return null;
    if (ctx.behavior.taskLoad > 0.15) return null; // 有任務不玩鬧

    // 先處理恢復：被打斷的長動作在情境允許時繼續。
    if (this.interrupted) {
      if (ctx.nowMs > this.interrupted.expiresAt) {
        this.interrupted = null;
      } else if (
        this.tables.sleepy.has(this.interrupted.action.expression) &&
        ctx.msSinceInteraction < SLEEP_RESUME_BLOCK_MS
      ) {
        // 剛被戳醒就躺回去睡，看起來像沒注意到人。放棄這個恢復計畫，
        // 回 idle 系（放鬆度還很低，池子裡也只剩眨眼類）。
        this.interrupted = null;
      } else if (!ctx.quiet && !ctx.reducedMotion) {
        const resume = this.interrupted;
        this.interrupted = null;
        const action = { ...resume.action, durationMs: Math.max(1_500, resume.remainingMs) };
        this.currentAction = { action, startedAt: ctx.nowMs };
        return action;
      }
    }

    if (ctx.quiet && !ctx.reducedMotion) {
      // 安靜時段：只剩偶爾眨眼（角色有眨眼表情才會）。
      const blink = this.tables.blink;
      if (blink && this.playable(blink.expression) && rng() < 0.03) {
        return { expression: blink.expression, durationMs: blink.durationMs, source: "blink" };
      }
      return null;
    }

    // 放鬆度：閒置越久越放鬆；喚起度高則收斂。
    const relax = clamp01(ctx.msSinceInteraction / 180_000) * (1 - ctx.behavior.activation);
    // hazard 抽樣（幾何分布間隔——絕不固定週期）。
    const hazard =
      0.06 *
      ctx.expressiveness *
      (1 + ctx.behavior.familiarity * 0.4) *
      (1 - Math.min(0.6, ctx.behavior.recentInterruptions * 0.15));
    if (rng() > hazard) return null;

    const pool = this.tables.ambient.filter((v) => {
      if (!this.playable(v.expression)) return false;
      if (ctx.reducedMotion && !v.reducedMotionOk) return false;
      if (relax < v.minRelax) return false;
      if ((this.cooldownUntil.get(v.expression) ?? 0) > ctx.nowMs) return false;
      if (this.recent.slice(-3).includes(v.expression)) return false;
      return true;
    });
    if (pool.length === 0) return null;
    // 個性權重：慵懶更常趴著/打哈欠、好奇更常張望…（權重，不是硬規則）。
    const weightOf = (v: AmbientVariant) =>
      Math.max(0.01, v.weight * (this.tuning.variantWeights[v.expression] ?? 1));
    const total = pool.reduce((sum, v) => sum + weightOf(v), 0);
    let pick = rng() * total;
    let chosen = pool[pool.length - 1];
    for (const v of pool) {
      pick -= weightOf(v);
      if (pick <= 0) {
        chosen = v;
        break;
      }
    }
    this.markUsed(chosen.expression, ctx.nowMs, chosen.cooldownMs);
    const action: DirectorAction = {
      expression: chosen.expression,
      durationMs: chosen.durationMs,
      source: "ambient",
    };
    this.currentAction = { action, startedAt: ctx.nowMs };
    return action;
  }

  private markUsed(expression: string, nowMs: number, cooldownMs: number) {
    this.cooldownUntil.set(expression, nowMs + cooldownMs * this.tuning.cooldownScale);
    this.recent = [...this.recent.slice(-4), expression];
  }
}
