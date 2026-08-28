// 風險分級 L0–L4（spec §2）。純函式、無 I/O：把後端解析出來的能力事實
// （通道、風險類別、敏感度、外部／實體副作用、是否需同意）映射到「使用者
// 會遇到什麼」的一句人話。
//
// 兩條不可違反的規則：
//  1. 這裡只「呈現」分級，不決定實際權限 —— 真正的強制永遠在 Rust Policy
//     Governor。UI 的分級絕不能比後端事實寬鬆。
//  2. 未知一律保守往上升級（unknown 不是 false）。

import type { HumanCard, TriState } from "./api";

export type RiskTierLevel = 0 | 1 | 2 | 3 | 4;

export interface RiskTierInput {
  /** 能力通道，如 desktop-pet／notification／haptic／webhook。 */
  channel?: string;
  /** 後端 manifest 的 risk class：low／medium／high／critical。 */
  riskClass?: string;
  /** 資料敏感度：none／low／medium／high／unknown。 */
  sensitivity?: string;
  /** 是否影響外部服務（externalSideEffect）。 */
  external?: TriState;
  /** 是否有實體效果（physicalEffect）。 */
  physical?: TriState;
  /** 使用前是否需要明確同意。 */
  requiresConsent?: boolean;
  /** 是否含個人資料。 */
  personalData?: TriState;
  /** 資料是否離開這台電腦。 */
  leavesDevice?: TriState;
  /** 技術 id（只用來辨識攝影機／麥克風／定位／Agent 寫入這幾類）。 */
  id?: string;
}

export interface RiskTier {
  tier: RiskTierLevel;
  /** 「L2 個人資料」這種短標籤，可直接當徽章。 */
  label: string;
  /** 一句人話：這一級預設會怎麼處理。 */
  policy: string;
  /** L3 才有：硬限制摘要（強度／時間／頻率）。 */
  hardLimits?: string;
}

/** 攝影機、持續麥克風、定位、Agent 寫入 —— 一律 L4。 */
const L4_ID = /camera|microphone|(^|[.\-_])mic([.\-_]|$)|location|geo|gps|screen\.?(capture|record)|workspace\.write|agent-session:/;
const L4_CHANNELS = new Set(["camera", "microphone", "location", "agent-write"]);

/** 外部或實體效果的通道。 */
const L3_CHANNELS = new Set([
  "light",
  "haptic",
  "webhook",
  "device",
  "serial",
  "mqtt",
  "ble",
  "bluetooth",
]);

/** 個人資料／檔案／記憶／Context Bundle。 */
const L2_ID = /(^|[.\-_])(file|files|filesystem|memory|knowledge|context|bundle|clipboard|contacts|calendar)([.\-_]|$)/;

/** 本機低風險：通知、短音效、角色移動、對話與紀錄。 */
const L1_CHANNELS = new Set(["notification", "audio", "log", "conversation", "web-ui"]);

/** 純角色呈現。 */
const L0_CHANNELS = new Set(["desktop-pet", "visual"]);

const TIERS: Record<RiskTierLevel, RiskTier> = {
  0: {
    tier: 0,
    label: "L0 純角色呈現",
    policy: "小樞的表情、動作與氣泡：預設開啟，不會每次問你，也不會留下打擾你的待辦。",
  },
  1: {
    tier: 1,
    label: "L1 本機低風險",
    policy: "只在這台電腦上發生（通知、短音效、角色移動）：設定一次就好，隨時可以關掉。",
  },
  2: {
    tier: 2,
    label: "L2 個人資料",
    policy: "會用到你的檔案、偏好或記憶：第一次使用、或使用範圍改變時會先問你。",
  },
  3: {
    tier: 3,
    label: "L3 外部或實體效果",
    policy: "會送到外部或造成實體動作（燈光、震動、裝置命令）：需要你明確授權。",
    hardLimits: "強度、持續時間與頻率由裝置安全上限強制收斂，AI 不能調高；緊急停止會立刻中止。",
  },
  4: {
    tier: 4,
    label: "L4 高敏感",
    policy:
      "攝影機、持續麥克風、定位、Agent 寫入檔案：每次使用都要你同意（或只給短效授權），使用中會持續顯示指示。",
  },
};

/** 只讀查表：由分級取得標籤與說明。 */
export function riskTierInfo(tier: RiskTierLevel): RiskTier {
  return TIERS[tier];
}

/** L0–L4 全表，用於「同意與安全」的分級說明。 */
export const RISK_TIERS: RiskTier[] = [TIERS[0], TIERS[1], TIERS[2], TIERS[3], TIERS[4]];

/**
 * 由能力事實推導風險分級。由高往低判斷；`unknown` 視為「可能會」，
 * 一律往高的一級靠（誠實階梯：未知不得當成安全）。
 */
export function riskTierOf(input: RiskTierInput): RiskTier {
  const channel = (input.channel ?? "").toLowerCase();
  const id = (input.id ?? "").toLowerCase();
  const sensitivity = (input.sensitivity ?? "").toLowerCase();
  const riskClass = (input.riskClass ?? "").toLowerCase();

  if (
    L4_CHANNELS.has(channel) ||
    (id !== "" && L4_ID.test(id)) ||
    sensitivity === "high" ||
    riskClass === "critical"
  ) {
    return TIERS[4];
  }

  if (
    input.external === true ||
    input.physical === true ||
    input.external === "unknown" ||
    input.physical === "unknown" ||
    L3_CHANNELS.has(channel) ||
    riskClass === "high"
  ) {
    return TIERS[3];
  }

  if (
    input.personalData === true ||
    input.personalData === "unknown" ||
    input.leavesDevice === true ||
    input.leavesDevice === "unknown" ||
    sensitivity === "unknown" ||
    sensitivity === "medium" ||
    (id !== "" && L2_ID.test(id))
  ) {
    return TIERS[2];
  }

  // 需要明確同意的能力至少是 L2 —— 不可因為「沒宣告語意」就降到 L0／L1。
  if (input.requiresConsent === true) return TIERS[2];

  if (L0_CHANNELS.has(channel)) return TIERS[0];
  if (L1_CHANNELS.has(channel) || sensitivity === "none" || sensitivity === "low") return TIERS[1];

  // 完全沒有語意宣告：保守當成會碰到個人資料。
  return TIERS[2];
}

/** 由 human card 直接取分級（欄位對映集中在這裡，各頁不再自行拼湊）。 */
export function riskTierOfCard(card: HumanCard): RiskTier {
  return riskTierOf({
    id: card.id,
    channel: card.channel,
    riskClass: card.riskClass,
    sensitivity: card.data?.sensitivity,
    personalData: card.data?.personalData,
    leavesDevice: card.data?.leavesDevice,
    external: card.effect?.externalSideEffect,
    physical: card.effect?.physicalEffect,
    requiresConsent: card.requiresConsent || card.consent.required === true,
  });
}
