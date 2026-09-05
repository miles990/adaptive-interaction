// 陪伴預設（M3 §4.1）：把「平常如何陪伴」收斂成四個可解釋的檔位——安靜／自然／活潑／自訂。
//
// 預設**不是**新的設定層，只是**既有欄位**的一組值：
//   - `companionExpressiveness`（桌面偏好；表演與說話頻率，無權限語意）
//   - `companionDoNotDisturb`（桌面偏好；不主動靠近、不主動說話）
//   - 主動式對話的 `mode`（後端；由 Rust 確定性強制）
//
// 硬規則（由 `src/test/companion-presets.test.ts` 逐一釘住）：
//   - 套用預設**只**寫上面那三個欄位：不覆蓋其它自訂值、不改費用／次數上限、
//     不啟用任何權限、不更換指定的 AI 幫手。
//   - 反推（`presetFor`）只要有一個欄位不吻合就是「自訂」，並改為逐項顯示有效值；
//     未知值誠實顯示「不明」，不硬塞進某個檔位。
//   - 「安靜」不等於關掉主動對話：必要訊息（等待確認、失敗、結果不確定、感測提示）
//     仍然送得出來，所以安靜檔位是 `necessary` 而不是 `off`。
//
// 純函式模組：不 import api／desktop，也不認得任何角色。

/** 反推用的輸入（呼叫端從 prefs／主動對話 config 取出這三個值）。 */
export interface CompanionPresetInputs {
  expressiveness?: string | null;
  doNotDisturb?: boolean | null;
  proactiveMode?: string | null;
}

/** 一個預設就是這三個既有欄位的一組值。 */
export interface CompanionPresetState {
  expressiveness: string;
  doNotDisturb: boolean;
  proactiveMode: string;
}

export type CompanionPresetId = "quiet" | "natural" | "lively";
/** 反推結果：吻合某個預設，或「自訂」。 */
export type CompanionPresetChoice = CompanionPresetId | "custom";

export interface CompanionPresetDefinition {
  id: CompanionPresetId;
  label: string;
  /** 一行說明（首屏摘要用）。 */
  summary: string;
  state: CompanionPresetState;
}

export const COMPANION_PRESETS: readonly CompanionPresetDefinition[] = [
  {
    id: "quiet",
    label: "安靜",
    summary: "少說話、不主動靠近；只在等待確認、失敗或結果不確定時提醒。",
    state: { expressiveness: "quiet", doNotDisturb: true, proactiveMode: "necessary" },
  },
  {
    id: "natural",
    label: "自然",
    summary: "一般的表現與說話頻率；重要的事會主動說，其餘安靜等你。",
    state: { expressiveness: "natural", doNotDisturb: false, proactiveMode: "natural" },
  },
  {
    id: "lively",
    label: "活潑",
    summary: "表現多一些，也會問候與輕量陪伴；頻率上限仍由系統強制。",
    state: { expressiveness: "lively", doNotDisturb: false, proactiveMode: "lively" },
  },
];

/** 套用預設時**唯一**會寫到桌面偏好的兩個鍵。 */
export const COMPANION_PRESET_PREFS_KEYS = ["companionExpressiveness", "companionDoNotDisturb"] as const;
/** 套用預設時**唯一**會寫到主動式對話的鍵。 */
export const COMPANION_PRESET_PROACTIVE_KEYS = ["mode"] as const;

export function presetDefinition(id: string): CompanionPresetDefinition | null {
  return COMPANION_PRESETS.find((p) => p.id === id) ?? null;
}

/** 反推目前的組合是哪一個預設；有任何一項不吻合（含缺值／未知值）就是「自訂」。 */
export function presetFor(inputs: CompanionPresetInputs): CompanionPresetChoice {
  const { expressiveness, doNotDisturb, proactiveMode } = inputs;
  if (typeof expressiveness !== "string" || typeof doNotDisturb !== "boolean" || typeof proactiveMode !== "string") {
    return "custom";
  }
  const hit = COMPANION_PRESETS.find(
    (p) =>
      p.state.expressiveness === expressiveness &&
      p.state.doNotDisturb === doNotDisturb &&
      p.state.proactiveMode === proactiveMode
  );
  return hit ? hit.id : "custom";
}

/**
 * 套用預設要寫的欄位。回傳的兩個物件就是**全部**會被寫入的東西——
 * 呼叫端不得再往裡面補欄位（測試會檢查鍵集合）。未知的 id 回 `null`（不猜）。
 */
export function applyCompanionPreset(id: string): {
  prefs: { companionExpressiveness: string; companionDoNotDisturb: boolean };
  proactive: { mode: string };
} | null {
  const def = presetDefinition(id);
  if (!def) return null;
  return {
    prefs: {
      companionExpressiveness: def.state.expressiveness,
      companionDoNotDisturb: def.state.doNotDisturb,
    },
    proactive: { mode: def.state.proactiveMode },
  };
}

const EXPRESSIVENESS_LABELS: Record<string, string> = {
  quiet: "安靜",
  natural: "自然",
  lively: "活潑",
};

const PROACTIVE_MODE_LABELS: Record<string, string> = {
  off: "關閉",
  necessary: "只有必要的事",
  natural: "自然",
  lively: "活潑",
  custom: "自訂",
};

/** 未知或缺值一律「不明」——不猜、也不冒充某個檔位。 */
function labelOf(table: Record<string, string>, value: string | null | undefined): string {
  if (typeof value !== "string") return "不明";
  return table[value] ?? "不明";
}

/**
 * 逐項可讀的有效值（「自訂」時顯示，讓使用者仍看得到現在到底是什麼）。
 * 三個語意分開列，**不**合併成一個布林。
 */
export function describeCompanionState(inputs: CompanionPresetInputs): string[] {
  return [
    `表現程度：${labelOf(EXPRESSIVENESS_LABELS, inputs.expressiveness)}`,
    `勿擾：${typeof inputs.doNotDisturb === "boolean" ? (inputs.doNotDisturb ? "開啟" : "關閉") : "不明"}`,
    `主動說話：${labelOf(PROACTIVE_MODE_LABELS, inputs.proactiveMode)}`,
  ];
}

/** 主動式對話模式的人話標籤（安靜區的「現在」欄共用）。 */
export function proactiveModeLabel(mode: string | null | undefined): string {
  return labelOf(PROACTIVE_MODE_LABELS, mode);
}

/** 表現程度的人話標籤。 */
export function expressivenessLabel(value: string | null | undefined): string {
  return labelOf(EXPRESSIVENESS_LABELS, value);
}
