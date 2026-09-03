// CPP §12 `shu-rig` Reference Adapter 的**全部**角色專屬表（純資料＋純函式）。
//
// 這裡是唯一允許出現小樞表情 id 的「行為層」位置：Director／gameFeel／
// personality／behavior／machine 都是 engine-neutral，由本檔的表注入。
// 換一個角色＝換一份 tables，不動任何行為模組。
//
// 誠實不變量（與 rig/expressions.ts 的 truthState 標記一致）：
//   - claim-completed 只演 success-claimed；success-verified 只在 envelope.truthState
//     === "verified" 時出現（truthState 只由 Runtime 決定，本表不升級）。
//   - presentationHints.variant 只能替換非安全 intent 的表情；安全 intent
//     （emergency／offline／blocked／unknown／failed／request-consent／ask／
//     cancelled）的表情固定。wait 的 variant 只在「等哪個 agent」的三個等待表情
//     之間選，全都是非真相的等待姿勢。
//   - ambient／反應／落地／微動作表全部非 truthState（isShuPlayable 把關，
//     rig.test 釘死）。

import { EXPRESSIONS, resolveExpression } from "../../companion/rig/expressions";
import { resolveRigAnimation, resolveSegments } from "../../companion/rig/timeline";
import type { AmbientVariant, DirectorTables } from "../../companion/director";
import type { LandingTable } from "../../companion/gameFeel";
import type { VariantWeightTable } from "../../companion/personality";
import type { LegacyEventArt } from "../../companion/machine";
import type { ToyKind } from "../../companion/playfield";
import { CharacterIntent, isSafetyIntent, TruthState } from "../protocol";

// ---------------------------------------------------------------------------
// 真相狀態防線
// ---------------------------------------------------------------------------

/** 這個表情存在且不是 truthState（Director／反應／ambient 只能播這些）。 */
export function isShuPlayable(expression: string): boolean {
  const expr = resolveExpression(expression);
  return expr !== null && expr.truthState !== true;
}

// ---------------------------------------------------------------------------
// Director tables（ambient 變體、反應、睡眠類、眨眼）
// ---------------------------------------------------------------------------

export const SHU_AMBIENT_VARIANTS: readonly AmbientVariant[] = [
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

/**
 * 反應意圖 → 表情（玩家/事件反應層；仍非 truthState）。
 * 高頻反應（單擊 `poked`、連戳 `poked-rapid`、拖起 `lifted`）各有 3 個變體（spec §5.2），
 * 由 Director 依冷卻與上一次用的挑一個不同的；`thinking` 是 L1 本機模板判斷「像任務」時
 * 的反應——非 truthState 的思考表情（machine 的 thinking transient 仍只由 runtime 事件驅動）。
 */
export const SHU_REACTIONS: Readonly<Record<string, string | readonly string[]>> = {
  notice: "notice",
  curious: "curious",
  peek: "peek",
  "lean-in": "lean-in",
  "player-back": "player-back",
  "await-player": "await-player",
  praised: "praised",
  "caught-slacking": "caught-slacking",
  question: "question",
  thinking: "thinking",
  "block-cursor": "block-cursor",
  poked: ["poked", "poked-flinch", "poked-grin"],
  lifted: ["lifted", "lifted-curious", "lifted-wriggle"],
  // 連戳也是高頻反應：以前只有一個變體，加上 8 秒預設冷卻，連戳 30 秒只會看到
  // 同一段演出（對抗審查 companion-gameplay-037）。
  "poked-rapid": ["poked-rapid", "deadpan", "pretend-not-hear"],
};

/** 單擊反應的冷卻：比一般反應短很多（連續點擊仍要有反應，只是換一個變體）。 */
export const SHU_CLICK_COOLDOWN_MS = 1_200;

/** 連戳反應的冷卻：比單擊長、比預設 8 秒短，讓三個變體真的輪得動。 */
export const SHU_RAPID_COOLDOWN_MS = 2_500;

/** 可以「假裝沒看到」的低優先意圖。 */
export const SHU_SOFT_INTENTS: readonly string[] = ["notice", "curious", "peek", "lean-in", "player-back"];

export const SHU_PRETEND_NOT_HEAR = "pretend-not-hear";

/** 睡眠類長 ambient：被互動打斷後不該原樣睡回去。 */
export const SHU_SLEEPY_AMBIENT: ReadonlySet<string> = new Set(["doze", "lie-flat", "sleep"]);

/** 安靜時唯一允許的就地眨眼。 */
export const SHU_BLINK = { expression: "blink", durationMs: 400 } as const;

export const SHU_DIRECTOR_TABLES: DirectorTables = {
  isPlayable: isShuPlayable,
  ambient: SHU_AMBIENT_VARIANTS,
  reactions: SHU_REACTIONS,
  softIntents: SHU_SOFT_INTENTS,
  pretendNotHear: SHU_PRETEND_NOT_HEAR,
  sleepy: SHU_SLEEPY_AMBIENT,
  blink: SHU_BLINK,
};

// ---------------------------------------------------------------------------
// gameFeel／personality／behavior tables
// ---------------------------------------------------------------------------

/** 放下角色的落地美術（§5.2）。 */
export const SHU_LANDING: LandingTable = {
  wobbly: { expression: "wobbly-landing", durationMs: 1600 },
  slip: { expression: "slip-play-cool", durationMs: 1800 },
  light: { expression: "land-light", durationMs: 900 },
};

/** 個性 → ambient 變體權重（慵懶更常趴著/打哈欠、好奇更常張望…）。 */
export const SHU_VARIANT_WEIGHTS: VariantWeightTable = {
  "lie-flat": [{ trait: "lazy", gain: 2 }],
  doze: [{ trait: "lazy", gain: 1.5 }],
  yawn: [{ trait: "lazy", gain: 1.6 }],
  "spaced-out": [{ trait: "lazy", gain: 0.8 }],
  stretch: [{ trait: "lazy", gain: 0.4 }],
  "look-around": [{ trait: "curious", gain: 1.2 }],
  groom: [{ trait: "proud", gain: 0.8 }],
  legswing: [{ trait: "playful", gain: 0.9 }],
  tailhug: [{ trait: "playful", gain: 0.5 }],
};

/** 舊 daemon 相容路徑（machine.mapRuntimeEvent）事件 → 小樞表情。 */
export const SHU_EVENT_ART: LegacyEventArt = {
  deviceOnline: "device-hello",
  deviceOffline: "device-lost",
  operateExternal: "operate-tool",
  ackBrief: "ack-nod",
  waitForAgent: (agentId) =>
    agentId === "codex" ? "wait-codex" : agentId === "claude-code" ? "wait-claude" : "waiting",
};

// ---------------------------------------------------------------------------
// 玩具目錄（gameplay.toys；文案屬於這個角色，不屬於 host）
// ---------------------------------------------------------------------------

export interface ToyCatalogEntry {
  kind: ToyKind;
  label: string;
  emoji: string;
}

export const SHU_TOYS: readonly ToyCatalogEntry[] = [
  { kind: "yarn", label: "丟毛球", emoji: "🧶" },
  { kind: "paper", label: "丟紙團", emoji: "🗞️" },
  { kind: "plane", label: "紙飛機", emoji: "✈️" },
  { kind: "light", label: "光點", emoji: "✨" },
  { kind: "wand", label: "逗貓棒", emoji: "🪶" },
  { kind: "trinket", label: "小物件（她只會好奇地看看）", emoji: "🧸" },
];

export function isShuToyKind(kind: string): kind is ToyKind {
  return SHU_TOYS.some((t) => t.kind === kind);
}

// ---------------------------------------------------------------------------
// intent → 表情計畫
// ---------------------------------------------------------------------------

export interface ShuExpressionPlan {
  /** 餵給 machineEventForAnimation 的名稱：canonical 動畫名或 rig 表情 id。 */
  animation: string;
  frameSlice?: [number, number];
  /** 實際上台的 rig 表情 id（別名與 frameSlice 解析後）。 */
  expression: string;
  /** base＝機器基態（emergency／offline）；clear＝回待機；transient＝其餘。 */
  mode: "base" | "transient" | "clear";
}

const PLAY_VARIANTS: Readonly<Record<string, string>> = {
  chase: "play-chase",
  "play-chase": "play-chase",
  carry: "play-carry",
  "play-carry": "play-carry",
  sneak: "sneak-closer",
  "sneak-closer": "sneak-closer",
  pounce: "pounce-miss",
  "pounce-miss": "pounce-miss",
  "hold-ball": "hold-ball",
  "keep-ball": "keep-ball",
};

const NOTICE_VARIANTS: Readonly<Record<string, string>> = {
  curious: "curious",
  listening: "listening",
  listen: "listening",
  "device-offline": "device-lost",
  "device-lost": "device-lost",
  "look-at-confirmation": "question",
  question: "question",
  "wait-attention": "waiting",
  peek: "peek",
  "lean-in": "lean-in",
};

const WAIT_VARIANTS: Readonly<Record<string, string>> = {
  codex: "wait-codex",
  "wait-codex": "wait-codex",
  "claude-code": "wait-claude",
  "wait-claude": "wait-claude",
};

const SLEEP_VARIANTS: Readonly<Record<string, string>> = {
  "lie-flat": "lie-flat",
  sleep: "sleep",
  doze: "doze",
};

function plan(animation: string, mode: ShuExpressionPlan["mode"], frameSlice?: [number, number]): ShuExpressionPlan {
  return {
    animation,
    ...(frameSlice ? { frameSlice } : {}),
    expression: resolveRigAnimation(animation, frameSlice).id,
    mode,
  };
}

/**
 * 20 個 canonical intent → 小樞表情。`animation` 用 canonical 名（act／waiting／
 * success／ask／blocked…）時由 machine 走對應的 transient kind（優先階梯照舊）；
 * 其餘直接是 rig 表情 id（performing）。
 */
export function shuExpressionPlan(
  intent: CharacterIntent,
  truthState: TruthState,
  variant?: string
): ShuExpressionPlan {
  const v = typeof variant === "string" ? variant : "";
  const safety = isSafetyIntent(intent);
  switch (intent) {
    case "idle":
      return plan("idle", "clear");
    case "cancelled":
      // 取消：誠實回到待機，不演成功也不演失敗。
      return plan("idle", "clear");
    case "emergency":
      return plan("emergency", "base");
    case "offline":
      return plan("offline", "base");
    case "blocked":
      return plan("blocked", "transient");
    case "unknown":
      return plan("unknown", "transient");
    case "failed":
      return plan("failed", "transient");
    case "claim-completed":
      return plan("success", "transient", [0, 1]);
    case "verified-success":
      // 綠勾只認 Runtime 的 verified；其餘一律只點頭（claimed）。
      return truthState === "verified" ? plan("success", "transient") : plan("success", "transient", [0, 1]);
    case "request-consent":
    case "ask":
      return plan("ask", "transient");
    case "wait":
      return plan(WAIT_VARIANTS[v] ?? "waiting", "transient");
    case "work":
      return plan(v === "operate-tool" || v === "operate-external" ? "operate-tool" : "act", "transient");
    case "think":
      return plan(v === "wait-attention" ? "waiting" : "thinking", "transient");
    case "notice":
      return plan(NOTICE_VARIANTS[v] ?? "notice", "transient");
    case "acknowledge":
      return plan(v === "acknowledge-briefly" ? "clicked" : "ack-nod", "transient");
    case "greet":
      return plan("device-hello", "transient");
    case "play":
      return plan(PLAY_VARIANTS[v] ?? "play-chase", "transient");
    case "rest":
      return plan("quiet", "transient");
    case "sleep":
      return plan(SLEEP_VARIANTS[v] ?? "doze", "transient");
    default: {
      // 不可能到這裡（20 個都列了）；保守回待機並標記非安全。
      void safety;
      return plan("idle", "clear");
    }
  }
}

/** 表情 enter 段長度（基態類命令的「演完」時間）。 */
export function shuEnterMs(expression: string): number {
  const expr = EXPRESSIONS[expression];
  if (!expr) return 120;
  return Math.max(0, resolveSegments(expr).enter.durationMs);
}

/** 沒有 durationHint 時的自然演出長度：enter＋一輪 loop（有界）。 */
export function shuNaturalDurationMs(expression: string): number {
  const expr = EXPRESSIONS[expression];
  if (!expr) return 3000;
  const seg = resolveSegments(expr);
  return Math.max(600, Math.min(12_000, seg.enter.durationMs + seg.loop.durationMs));
}
