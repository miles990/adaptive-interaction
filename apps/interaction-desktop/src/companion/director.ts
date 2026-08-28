// Interaction Director（spec §6）：統一的行為導演。
//
//   事件 → Event Normalizer → Attention → Utility Scoring → Behavior Intent
//   → Action Scheduler →（machine transient / 表情通道）
//
// 本模組純確定性（seeded RNG 注入），不呼叫 AI。它擁有「生活層」的
// 選擇權：ambient 變體、冷卻、防重複、中斷後恢復；真相狀態（成功/失敗/
// 阻擋/未知/緊急）永遠由 machine.ts 的 runtime 事件驅動，Director 不可
// 也不會排程 truthState 表情（schedule 端過濾＋測試釘死）。

import { BehaviorState, EventClass, EventScoreContext, scoreEvent } from "./behavior";
import { resolveExpression } from "./rig/expressions";

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
  source: "ambient" | "reaction" | "resume";
}

/** ambient 變體池：全部非 truthState。 */
export interface AmbientVariant {
  expression: string;
  durationMs: number;
  weight: number;
  /** 需要的最低放鬆度（0..1；越大越要真的沒事才出現）。 */
  minRelax: number;
  reducedMotionOk: boolean;
  cooldownMs: number;
}

export const AMBIENT_VARIANTS: AmbientVariant[] = [
  { expression: "blink", durationMs: 400, weight: 10, minRelax: 0, reducedMotionOk: true, cooldownMs: 2_000 },
  { expression: "look-around", durationMs: 1_900, weight: 5, minRelax: 0.15, reducedMotionOk: false, cooldownMs: 18_000 },
  { expression: "groom", durationMs: 1_700, weight: 4, minRelax: 0.3, reducedMotionOk: false, cooldownMs: 40_000 },
  { expression: "stretch", durationMs: 1_500, weight: 3, minRelax: 0.45, reducedMotionOk: false, cooldownMs: 50_000 },
  { expression: "yawn", durationMs: 1_700, weight: 2, minRelax: 0.55, reducedMotionOk: false, cooldownMs: 70_000 },
  { expression: "legswing", durationMs: 6_000, weight: 3, minRelax: 0.4, reducedMotionOk: false, cooldownMs: 80_000 },
  { expression: "spaced-out", durationMs: 5_000, weight: 2, minRelax: 0.5, reducedMotionOk: false, cooldownMs: 60_000 },
  { expression: "tailhug", durationMs: 7_000, weight: 2, minRelax: 0.7, reducedMotionOk: false, cooldownMs: 110_000 },
  { expression: "lie-flat", durationMs: 9_000, weight: 2, minRelax: 0.85, reducedMotionOk: false, cooldownMs: 150_000 },
  { expression: "doze", durationMs: 10_000, weight: 1.5, minRelax: 0.92, reducedMotionOk: false, cooldownMs: 200_000 },
];

/** 反應意圖 → 表情（玩家/事件反應層；仍非 truthState）。 */
export const REACTION_EXPRESSIONS: Record<string, string> = {
  notice: "notice",
  curious: "curious",
  peek: "peek",
  "lean-in": "lean-in",
  "player-back": "player-back",
  "await-player": "await-player",
  praised: "praised",
  "caught-slacking": "caught-slacking",
  question: "question",
  "block-cursor": "block-cursor",
  "poked-rapid": "poked-rapid",
};

interface InterruptedAction {
  action: DirectorAction;
  remainingMs: number;
  /** 過了這個時間就不再恢復（情境已變）。 */
  expiresAt: number;
}

const clamp01 = (v: number) => Math.max(0, Math.min(1, v));

export class InteractionDirector {
  private cooldownUntil = new Map<string, number>();
  private recent: string[] = [];
  private interrupted: InterruptedAction | null = null;
  private currentAction: { action: DirectorAction; startedAt: number } | null = null;

  /** 真相狀態防線：Director 永不排程 truthState 表情。 */
  private playable(expression: string): boolean {
    const expr = resolveExpression(expression);
    return expr !== null && expr.truthState !== true;
  }

  /** 事件效用評分（供上層決定是否值得反應；安全類不受壓制）。 */
  score(cls: EventClass, ctx: EventScoreContext): number {
    return scoreEvent(cls, ctx);
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

  /** 使用者要求的即時反應（點擊等由 machine 直接處理；這裡處理語意反應）。 */
  react(intent: string, nowMs: number, durationMs = 2_500): DirectorAction | null {
    const expression = REACTION_EXPRESSIONS[intent];
    if (!expression || !this.playable(expression)) return null;
    const action: DirectorAction = { expression, durationMs, source: "reaction" };
    this.currentAction = { action, startedAt: nowMs };
    this.interrupted = null; // 新反應取消舊的恢復計畫
    this.markUsed(expression, nowMs, 8_000);
    return action;
  }

  /** 每 tick 呼叫（~500ms）。回傳要演的動作或 null。 */
  tick(ctx: DirectorContext, rng: () => number): DirectorAction | null {
    if (!ctx.ambient) return null;
    if (ctx.behavior.taskLoad > 0.15) return null; // 有任務不玩鬧

    // 先處理恢復：被打斷的長動作在情境允許時繼續。
    if (this.interrupted) {
      if (ctx.nowMs > this.interrupted.expiresAt) {
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
      // 安靜時段：只剩偶爾眨眼。
      if (rng() < 0.03) {
        return { expression: "blink", durationMs: 400, source: "ambient" };
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

    const pool = AMBIENT_VARIANTS.filter((v) => {
      if (!this.playable(v.expression)) return false;
      if (ctx.reducedMotion && !v.reducedMotionOk) return false;
      if (relax < v.minRelax) return false;
      if ((this.cooldownUntil.get(v.expression) ?? 0) > ctx.nowMs) return false;
      if (this.recent.slice(-3).includes(v.expression)) return false;
      return true;
    });
    if (pool.length === 0) return null;
    const total = pool.reduce((sum, v) => sum + v.weight, 0);
    let pick = rng() * total;
    let chosen = pool[pool.length - 1];
    for (const v of pool) {
      pick -= v.weight;
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
    this.cooldownUntil.set(expression, nowMs + cooldownMs);
    this.recent = [...this.recent.slice(-4), expression];
  }
}
