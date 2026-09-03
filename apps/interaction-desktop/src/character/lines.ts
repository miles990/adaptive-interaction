// CPP：intent → 固定文字的最後退路（system.text）與最小文字角色共用的文案。
//
// 安全語句（emergency／blocked／unknown／failed／verified-success）直接引用
// companion/packs.ts 的 FIXED_SAFETY_LINES——任何角色 pack 與 adapter 都不能覆寫。
// 「做完了，也確認過結果。」只在 truthState === "verified" 時出現；claimed 一律
// 只說「做完了。」，不加勾。

import { FIXED_SAFETY_LINES } from "../companion/packs";
import { CharacterIntent, TruthState } from "./protocol";

/** 非安全 intent 的預設短句（可被角色以 presentationHints.message 補充，不能取代安全句）。 */
export const DEFAULT_INTENT_LINES: Readonly<Record<CharacterIntent, string>> = {
  idle: "",
  notice: "注意到了。",
  acknowledge: "收到。",
  think: "思考中…",
  work: "處理中…",
  wait: "等待中。",
  ask: "需要你的回覆。",
  "request-consent": "需要你的同意才能繼續。",
  blocked: FIXED_SAFETY_LINES.blocked,
  unknown: FIXED_SAFETY_LINES.unknown,
  "claim-completed": "做完了。",
  "verified-success": "做完了。",
  failed: FIXED_SAFETY_LINES.failed,
  cancelled: "已取消。",
  offline: "目前連不上系統。",
  emergency: FIXED_SAFETY_LINES.emergency,
  greet: "你好。",
  play: "來玩一下。",
  rest: "休息中。",
  sleep: "睡著了。",
};

export interface IntentLine {
  text: string;
  /** 只有 verified 才有綠勾；其餘一律 "none"。 */
  marker: "verified" | "none";
  /** 這句是否為不可覆寫的安全語句。 */
  fixed: boolean;
}

/**
 * 決定某 intent／truthState 要顯示的文字。安全 intent 的文字固定；
 * verified-success 只有在 truthState 真的是 verified 時才給綠勾與「確認過」句。
 */
export function intentLine(
  intent: CharacterIntent,
  truthState: TruthState,
  hintMessage?: string
): IntentLine {
  if (intent === "verified-success" || intent === "claim-completed") {
    if (truthState === "verified") {
      return { text: FIXED_SAFETY_LINES["succeeded-verified"], marker: "verified", fixed: true };
    }
    return { text: DEFAULT_INTENT_LINES["claim-completed"], marker: "none", fixed: true };
  }
  switch (intent) {
    case "emergency":
    case "blocked":
    case "unknown":
    case "failed":
      return { text: DEFAULT_INTENT_LINES[intent], marker: "none", fixed: true };
    case "offline":
    case "request-consent":
    case "wait":
    case "ask":
    case "cancelled":
      return { text: DEFAULT_INTENT_LINES[intent], marker: "none", fixed: true };
    default: {
      const base = DEFAULT_INTENT_LINES[intent];
      const hint = typeof hintMessage === "string" ? hintMessage.slice(0, 200).trim() : "";
      return { text: hint.length > 0 ? hint : base, marker: "none", fixed: false };
    }
  }
}
