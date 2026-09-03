// 打擾政策（spec §5.1「Hover 短氣泡但不能每次都打擾」、§6.1 Quiet/勿擾）。
//
// 全部純函式：什麼時候可以說話、可以出聲、可以主動靠近，由這裡決定，
// CompanionApp 只負責接線。安全文字（緊急停止等）永遠不受這些開關影響。

import { dominantTrait, PersonalityProfile, PersonalityTrait } from "./personality";

// ---------------------------------------------------------------------------
// Quiet / 勿擾
// ---------------------------------------------------------------------------

export interface QuietInputs {
  /** runtime status 的 quietHours。 */
  quietHours: boolean;
  /** 使用者的勿擾開關（DesktopPrefs.companionDoNotDisturb）。 */
  doNotDisturb: boolean;
}

/** 是否進入 quiet 基態（安靜陪伴：不主動靠近、不主動說話）。 */
export function quietBase(inputs: QuietInputs): boolean {
  return inputs.quietHours || inputs.doNotDisturb;
}

/**
 * 「一小時內不要主動說話」／「今天安靜一點」的本機安靜期。
 *
 * 這個開關同時要關掉角色自己的隨口氣泡、hover 短氣泡與 Director 的 ambient
 * 表演——只叫 runtime 停掉主動對話是不夠的（角色照樣會自己冒話）。
 * **安全文字（緊急停止／被擋下／未知／失敗）不受影響。**
 *
 * @param quietUntilMs epoch ms；0／非有限值＝沒有設定。
 */
export function proactiveQuietActive(quietUntilMs: number, nowMs: number): boolean {
  if (!Number.isFinite(quietUntilMs) || !Number.isFinite(nowMs)) return false;
  return quietUntilMs > nowMs;
}

/** 快捷選單的安靜時長（分鐘）→ 到期時間（epoch ms）。 */
export function proactiveQuietUntil(minutes: number, nowMs: number): number {
  const m = Number.isFinite(minutes) ? Math.max(0, Math.min(24 * 60, minutes)) : 0;
  return nowMs + m * 60_000;
}

/** 角色可否主動靠近／看向游標。 */
export function approachAllowed(inputs: QuietInputs & { approachEnabled: boolean }): boolean {
  return inputs.approachEnabled && !quietBase(inputs);
}

// ---------------------------------------------------------------------------
// 氣泡與音效開關
// ---------------------------------------------------------------------------

/** 氣泡是否顯示：關掉氣泡後只剩安全文字（緊急停止、被擋下、未知）。 */
export function bubbleAllowed(opts: { enabled: boolean; safety: boolean }): boolean {
  return opts.safety || opts.enabled;
}

/**
 * 氣泡偏好。Runtime 要求顯示訊息、但使用者關掉了氣泡時，不能回報
 * `displayed`——誠實回 failed＋原因（訊息真的沒有顯示）。
 */
export function bubbleOutcome(enabled: boolean): {
  show: boolean;
  outcome?: "failed";
  detail?: string;
} {
  if (enabled) return { show: true };
  return {
    show: false,
    outcome: "failed",
    detail: "使用者已關閉角色說話氣泡：這則訊息沒有顯示",
  };
}

/**
 * 音效偏好（預設關閉）。關閉時不得偷偷不播卻回報 completed——
 * 誠實回 failed＋原因，讓 Runtime 知道這次沒有真的出聲。
 */
export function soundOutcome(enabled: boolean): {
  play: boolean;
  outcome?: "failed";
  detail?: string;
} {
  if (enabled) return { play: true };
  return {
    play: false,
    outcome: "failed",
    detail: "使用者已關閉角色音效：這次沒有播放任何聲音",
  };
}

// ---------------------------------------------------------------------------
// Hover 短氣泡（§5.1-3）
// ---------------------------------------------------------------------------

/** 停留多久才算 hover。 */
export const HOVER_BUBBLE_MIN_MS = 700;
/** 兩次 hover 氣泡之間的最小間隔。 */
export const HOVER_BUBBLE_COOLDOWN_MS = 45_000;

/** 本機模板短句（不呼叫 AI、不含使用者資料、不持久化游標）。每個性 ≥ 3 句（spec §5.2 變體）。 */
export const HOVER_LINES: Record<PersonalityTrait, string[]> = {
  curious: ["在看什麼？", "有新東西嗎？", "你那邊有什麼好玩的？", "欸，剛剛那是什麼？"],
  playful: ["要玩嗎？", "戳我也可以。", "丟個毛球來嘛。", "抓不到我～"],
  lazy: ["……嗯？", "我在，只是不太想動。", "再五分鐘。", "有事再叫我。"],
  proud: ["我一直都在，不用確認。", "哼，看夠了嗎？", "看什麼，我當然沒偷懶。", "有問題儘管問，我都會。"],
  witty: ["來得正好。", "需要幫忙就說。", "盯著我不會讓工作變快喔。", "又見面了。"],
  smart: ["需要我看一下什麼嗎？", "在旁邊待命。", "目前沒有新狀況。", "要我查什麼嗎？"],
};

export interface HoverBubbleInput {
  /** 游標停在角色上的累計時間。 */
  hoverMs: number;
  nowMs: number;
  /** 上一次任何氣泡的時間（共用冷卻，不搶安全訊息的版面）。 */
  lastBubbleAt: number;
  /** DesktopPrefs.companionBubbles。 */
  bubblesEnabled: boolean;
  /** DesktopPrefs.companionApproach。 */
  approachEnabled: boolean;
  /** quiet hours 或勿擾。 */
  quiet: boolean;
  personality: PersonalityProfile;
  /** 0..1 決定選哪一句（seeded RNG 注入）。 */
  rand: number;
  /** 上一句 hover 短句（防重複：有別句可選就不連說同一句）。 */
  lastText?: string | null;
}

export interface HoverBubbleDecision {
  show: boolean;
  text?: string;
  reason?: "too-short" | "cooldown" | "disabled" | "quiet";
}

/** Hover 短氣泡：停留夠久、冷卻過了、沒被關掉也不在安靜時段才說一句。 */
export function hoverBubblePolicy(input: HoverBubbleInput): HoverBubbleDecision {
  if (!input.bubblesEnabled || !input.approachEnabled) return { show: false, reason: "disabled" };
  if (input.quiet) return { show: false, reason: "quiet" };
  if (input.hoverMs < HOVER_BUBBLE_MIN_MS) return { show: false, reason: "too-short" };
  if (input.nowMs - input.lastBubbleAt < HOVER_BUBBLE_COOLDOWN_MS) {
    return { show: false, reason: "cooldown" };
  }
  const all = HOVER_LINES[dominantTrait(input.personality)];
  const lines = all.length > 1 && input.lastText ? all.filter((l) => l !== input.lastText) : all;
  const rand = Number.isFinite(input.rand) ? Math.max(0, Math.min(0.999999, input.rand)) : 0;
  return { show: true, text: lines[Math.floor(rand * lines.length)] };
}
