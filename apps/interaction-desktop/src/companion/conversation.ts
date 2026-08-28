// L1 短語意互動層（spec §8.2）：可插拔 Conversation Provider。
//
// 本輪不接模型 API：預設 LocalTemplateProvider 是純確定性規則＋有限模板。
// 職責：簡短輸入理解、決定是否回話、選語氣與 behaviorIntent、判斷是否
// 建議建立 Codex/Claude 任務。沒有合適 Provider 時就是這個本機降級——
// 絕不為普通反應啟動昂貴的工作 Agent。
//
// 誠實：模板回覆不冒充理解；不確定就承認（「我不太確定你的意思」），
// 不假裝有答案。安全語句仍由 packs.ts 固定文案負責，Provider 碰不到。

export interface ConversationContext {
  /** 有沒有已開啟的 agent 工作階段可以委派。 */
  openAgentSessions: number;
  /** 最近一次互動距今 ms（判斷「回來了」）。 */
  msSinceInteraction: number;
  /** 表現度（quiet 時傾向不回話）。 */
  expressiveness: "quiet" | "natural" | "lively" | string;
}

export interface ConversationResult {
  /** 要說的話（null=決定不回話）。 */
  reply: string | null;
  /** 建議的角色表情（非 truth-state；由呼叫端經 machine performing 播放）。 */
  behaviorIntent: string | null;
  /** 是否建議把這段輸入交給工作 Agent。 */
  suggestDelegate: boolean;
}

export interface ConversationProvider {
  readonly id: string;
  /** 決定怎麼回應一段短輸入。永不丟例外；不確定回 null reply。 */
  considerReply(text: string, ctx: ConversationContext): ConversationResult;
}

const GREETINGS = ["嗨", "hi", "hello", "哈囉", "早安", "午安", "晚安", "你好", "在嗎"];
const THANKS = ["謝謝", "感謝", "thanks", "thank you", "辛苦了", "做得好", "好棒", "讚"];
const TASK_HINTS = [
  "幫我",
  "請你",
  "可以幫",
  "修",
  "寫",
  "改",
  "查",
  "整理",
  "分析",
  "產生",
  "建立",
  "refactor",
  "fix",
  "implement",
  "review",
];

/** 本機規則＋有限模板（無 Provider 時的自然降級）。 */
export class LocalTemplateProvider implements ConversationProvider {
  readonly id = "local-template";

  considerReply(text: string, ctx: ConversationContext): ConversationResult {
    const t = text.trim().toLowerCase();
    if (t.length === 0) {
      return { reply: null, behaviorIntent: null, suggestDelegate: false };
    }
    // 安靜表現度：只在明確被打招呼時簡短回應。
    const quiet = ctx.expressiveness === "quiet";

    if (GREETINGS.some((g) => t.startsWith(g))) {
      const back = ctx.msSinceInteraction > 30 * 60_000;
      return {
        reply: quiet ? "嗨。" : back ? "歡迎回來！我有乖乖看家。" : "嗨，我在。",
        behaviorIntent: back ? "player-back" : "notice",
        suggestDelegate: false,
      };
    }
    if (THANKS.some((g) => t.includes(g))) {
      return {
        reply: quiet ? null : "嘿嘿，小事一件。",
        behaviorIntent: "praised",
        suggestDelegate: false,
      };
    }
    const taskLike = TASK_HINTS.some((h) => t.includes(h)) || t.length > 40;
    if (taskLike) {
      return {
        reply:
          ctx.openAgentSessions > 0
            ? "這聽起來像個任務——上面可以選擇交給哪個 AI 工作階段。"
            : "這聽起來像個任務。到「工作」頁建立 AI 工作階段，我就能把它交出去。",
        behaviorIntent: "thinking",
        suggestDelegate: true,
      };
    }
    if (t.endsWith("?") || t.endsWith("？")) {
      // 誠實：本機模板沒有知識，不假裝有答案。
      return {
        reply: quiet ? null : "我不太確定——這已經超過我本機能回答的範圍了。",
        behaviorIntent: "question",
        suggestDelegate: false,
      };
    }
    // 一般短句：已記錄（真實行為——呼叫端已 push observation），簡短承認。
    return {
      reply: quiet ? null : "收到，我記下來了。",
      behaviorIntent: null,
      suggestDelegate: false,
    };
  }
}

/** 目前啟用的 Provider（本輪固定為本機模板；介面保留可插拔）。 */
export function activeConversationProvider(): ConversationProvider {
  return new LocalTemplateProvider();
}
