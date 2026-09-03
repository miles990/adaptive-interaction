// CPP：canonical intent ↔ 舊 sprite pack（character-pack 1.0／1.1）動畫名的對照。
//
// 同時供 migratePackToManifest（推導 intents／fallbacks）與 SpriteCharacterAdapter
// （執行期解析）使用，確保「manifest 宣告什麼」與「adapter 真的演什麼」一致。
// 誠實規則：
//   - claim-completed 只演 success 的點頭幀（frameSlice [0,1]）；完整綠勾只給
//     truthState === "verified" 的 verified-success。
//   - 安全 intent（emergency／offline／blocked／unknown／failed）的 fallback 鏈只往
//     更冷靜的動畫走，永遠不落到 success。
//   - presentationHints.variant 只能替換非安全 intent 的動畫。

import { CHARACTER_INTENTS, CharacterIntent, isSafetyIntent, TruthState } from "./protocol";

/** intent → 直接對應的動畫名（存在時視為原生支援）。 */
export const SPRITE_INTENT_ANIMATION: Readonly<Record<CharacterIntent, string>> = {
  idle: "idle",
  notice: "notice",
  acknowledge: "clicked",
  think: "thinking",
  work: "act",
  wait: "waiting",
  ask: "ask",
  "request-consent": "ask",
  blocked: "blocked",
  unknown: "unknown",
  "claim-completed": "success",
  "verified-success": "success",
  failed: "failed",
  cancelled: "paused",
  offline: "offline",
  emergency: "emergency",
  greet: "notice",
  play: "curious",
  rest: "quiet",
  sleep: "lie",
};

/**
 * 與 companion/renderer.ts 的 FALLBACKS 相同的安全退階鏈（該表未匯出，這裡鏡射；
 * 安全狀態只往 paused／offline／idle 退，絕不往 success）。
 */
export const SPRITE_FALLBACKS: Readonly<Record<string, readonly string[]>> = {
  emergency: ["paused", "offline", "idle"],
  offline: ["paused", "idle"],
  blocked: ["paused", "idle"],
  unknown: ["paused", "idle"],
  failed: ["blocked", "paused", "idle"],
  success: ["idle"],
  listening: ["notice", "idle"],
  curious: ["notice", "idle"],
  stretch: ["idle"],
  lie: ["quiet", "idle"],
  legswing: ["idle"],
  tailhug: ["quiet", "idle"],
  default: ["idle"],
};

/** 動畫名 → 代表的 intent（取 CHARACTER_INTENTS 順序第一個；success→claim-completed，永不升級成 verified）。 */
export function intentForAnimation(animation: string): CharacterIntent | null {
  for (const intent of CHARACTER_INTENTS) {
    if (SPRITE_INTENT_ANIMATION[intent] === animation) return intent;
  }
  return null;
}

export interface SpriteAnimationDef {
  frames: number[];
  fps: number;
  loop: boolean;
}

export interface SpriteResolution {
  animation: string;
  frameSlice?: [number, number];
  /** true = intent 的直接動畫存在（原生）；false = 走 fallback 鏈。 */
  direct: boolean;
  /** 第一輪播放長度（ms）。 */
  firstLoopMs: number;
}

/**
 * 解析 intent 應播的動畫。回 null 表示 pack 裡什麼都找不到（連 idle 都沒有）。
 */
export function resolveSpriteAnimation(
  animations: Record<string, SpriteAnimationDef>,
  intent: CharacterIntent,
  opts: { truthState?: TruthState; variant?: string } = {}
): SpriteResolution | null {
  // verified-success 沒有 verified 真相 → 視為 claim-completed（只點頭）。
  const effectiveIntent: CharacterIntent =
    intent === "verified-success" && opts.truthState !== "verified" ? "claim-completed" : intent;
  const safety = isSafetyIntent(effectiveIntent);
  const intendedBase = SPRITE_INTENT_ANIMATION[effectiveIntent];
  // 安全 intent 中「本來就不是 success」者（emergency／offline／blocked／unknown／failed／
  // cancelled／request-consent／wait／ask）絕不可落到 success；claim-completed 本身就是 success（點頭幀）。
  const neverSuccess = safety && intendedBase !== "success";

  let base = intendedBase;
  let direct = !!animations[base];
  if (!safety && opts.variant && animations[opts.variant] && opts.variant !== "success") {
    // 非安全 intent 可依提示換動畫（例如 notice + variant "listening"）。
    base = opts.variant;
    direct = true;
  }

  let chosen: string | null = animations[base] ? base : null;
  if (!chosen) {
    const chain = SPRITE_FALLBACKS[base] ?? SPRITE_FALLBACKS.default;
    for (const alt of chain) {
      if (animations[alt]) {
        chosen = alt;
        break;
      }
    }
  }
  if (!chosen && animations.idle) chosen = "idle";
  if (!chosen) return null;
  // 負面安全 intent 絕不可落到 success（鏈本身保證，這裡再守一次）。
  if (neverSuccess && chosen === "success" && animations.idle) chosen = "idle";

  const def = animations[chosen];
  let frameSlice: [number, number] | undefined;
  if (chosen === "success" && effectiveIntent === "claim-completed") {
    frameSlice = [0, Math.min(1, Math.max(0, def.frames.length - 1))];
  }
  const frameCount = frameSlice ? frameSlice[1] - frameSlice[0] + 1 : def.frames.length;
  const fps = def.fps > 0 ? def.fps : 1;
  return {
    animation: chosen,
    frameSlice,
    direct,
    firstLoopMs: Math.max(1, Math.round((frameCount / fps) * 1000)),
  };
}

/** 從動畫集合推導原生支援的 intent 清單（供 migration）。 */
export function nativeIntentsOf(animations: Record<string, unknown>): CharacterIntent[] {
  return CHARACTER_INTENTS.filter((i) => !!animations[SPRITE_INTENT_ANIMATION[i]]);
}

/**
 * 從舊 FALLBACKS 鏈推導 manifest.fallbacks.intents：對每個沒有直接動畫的 intent，
 * 沿鏈找第一個存在的動畫，再反查它代表的 intent。
 */
export function deriveIntentFallbacks(
  animations: Record<string, unknown>
): Partial<Record<CharacterIntent, CharacterIntent>> {
  const out: Partial<Record<CharacterIntent, CharacterIntent>> = {};
  for (const intent of CHARACTER_INTENTS) {
    const base = SPRITE_INTENT_ANIMATION[intent];
    if (animations[base]) continue;
    const chain = SPRITE_FALLBACKS[base] ?? SPRITE_FALLBACKS.default;
    for (const alt of chain) {
      if (!animations[alt]) continue;
      const target = intentForAnimation(alt);
      if (target && target !== intent) {
        out[intent] = target;
      }
      break;
    }
  }
  return out;
}
