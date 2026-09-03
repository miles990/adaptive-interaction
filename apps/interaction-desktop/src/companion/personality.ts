// 個性（spec §4.3）：個性不只存在於對話。
//
// PersonalityProfile 由「表現度（quiet/natural/lively）＋ persona pack id」
// 純函式派生；PersonalityTuning 再由 profile 派生，餵給：
//   - Interaction Director：冷卻倍率、ambient 變體權重、假裝沒看到的機率
//   - Playfield：移動/追逐速度、靠近距離（經 StepInputs）
//   - ExpressionTimeline：注意力分段（耳→視線→頭）
//
// 全部確定性、無 I/O、無 AI：同樣的輸入永遠得到同樣的 tuning。
// 個性只影響「怎麼演」，永遠不影響權限、安全上限或誠實階梯。
//
// 本模組不認識任何角色的表情 id：ambient 變體權重由角色 adapter 的 tables
// （例如 character/adapters/shuTables.ts 的 SHU_VARIANT_WEIGHTS）以
// VariantWeightTable 注入；沒注入時 variantWeights 為空（Director 視為權重 1）。

export type PersonalityTrait =
  | "smart"
  | "witty"
  | "playful"
  | "lazy"
  | "proud"
  | "curious";

/** 六個特質，各 0..1。 */
export type PersonalityProfile = Record<PersonalityTrait, number>;

export interface PersonalityTuning {
  /** 散步速度倍率。 */
  speedScale: number;
  /** 追逐速度倍率。 */
  chaseSpeedScale: number;
  /** 靠近目標時停下的距離（邏輯 px；好奇會靠更近）。 */
  approachDistance: number;
  /** 冷卻倍率（慵懶＝更久才想再動）。 */
  cooldownScale: number;
  /** ambient 變體權重倍率（未列出的＝1）。 */
  variantWeights: Record<string, number>;
  /** 俏皮：偶爾假裝沒看到的機率 0..1。 */
  pretendNotSeeChance: number;
  /** 注意力分段延遲：耳朵→視線→轉頭（ms）。 */
  attentionStagger: { earMs: number; gazeMs: number; headMs: number };
  /** 慵懶：起身慢半拍（ms）。 */
  riseDelayMs: number;
}

const clamp01 = (v: number) => Math.max(0, Math.min(1, Number.isFinite(v) ? v : 0));
const clampN = (v: number, a: number, b: number) => Math.max(a, Math.min(b, v));

/** 一條變體權重規則：weight = 1 + Σ trait × gain（gain 可為負）。 */
export interface VariantWeightRule {
  trait: PersonalityTrait;
  gain: number;
}

/** 表情 id → 權重規則（角色 adapter 提供；engine-neutral 的本模組只做乘加）。 */
export type VariantWeightTable = Readonly<Record<string, readonly VariantWeightRule[]>>;

/** 依個性算出每個變體的權重倍率（未列出的變體＝1）。 */
export function variantWeightsFor(p: PersonalityProfile, table: VariantWeightTable): Record<string, number> {
  const out: Record<string, number> = {};
  for (const [expression, rules] of Object.entries(table)) {
    let w = 1;
    for (const r of rules) w += clamp01(p[r.trait]) * (Number.isFinite(r.gain) ? r.gain : 0);
    out[expression] = Math.max(0.01, w);
  }
  return out;
}

/** 表現度基準（quiet 收斂、lively 外放）。 */
const BY_EXPRESSIVENESS: Record<string, PersonalityProfile> = {
  quiet: { smart: 0.6, witty: 0.35, playful: 0.25, lazy: 0.6, proud: 0.4, curious: 0.45 },
  natural: { smart: 0.65, witty: 0.5, playful: 0.5, lazy: 0.4, proud: 0.45, curious: 0.6 },
  lively: { smart: 0.7, witty: 0.7, playful: 0.8, lazy: 0.2, proud: 0.55, curious: 0.85 },
};

/** persona pack 的個性偏移（只有內建 persona；未知 persona = 不偏移）。 */
const BY_PERSONA: Record<string, Partial<PersonalityProfile>> = {
  "persona-shu": { curious: 0.1, proud: 0.1, playful: 0.05 },
  "persona-navigator": { smart: 0.15, playful: -0.15, lazy: -0.1, witty: 0.05 },
};

/** 表現度＋persona → 個性（純函式）。 */
export function personalityFor(
  expressiveness: string | null | undefined,
  personaId?: string | null
): PersonalityProfile {
  const base = BY_EXPRESSIVENESS[String(expressiveness ?? "natural")] ?? BY_EXPRESSIVENESS.natural;
  const shift = BY_PERSONA[String(personaId ?? "")] ?? {};
  const out = { ...base };
  for (const key of Object.keys(out) as PersonalityTrait[]) {
    out[key] = clamp01(out[key] + (shift[key] ?? 0));
  }
  return out;
}

/** 個性 → 行為 tuning（純函式）。變體權重表由角色 adapter 注入。 */
export function tuningFor(p: PersonalityProfile, variantWeightTable: VariantWeightTable = {}): PersonalityTuning {
  const { smart, playful, lazy, curious, witty } = p;
  return {
    // 慵懶明顯拖慢、俏皮稍微加快。
    speedScale: clampN(1 + playful * 0.25 - lazy * 0.45, 0.5, 1.4),
    chaseSpeedScale: clampN(1 + playful * 0.35 + witty * 0.1 - lazy * 0.3, 0.5, 1.5),
    // 好奇會靠得更近，慵懶懶得走完最後一段。
    approachDistance: clampN(24 - curious * 10 + lazy * 8, 8, 40),
    cooldownScale: clampN(1 + lazy * 0.6 - playful * 0.35, 0.5, 2),
    variantWeights: variantWeightsFor(p, variantWeightTable),
    // 俏皮才會假裝沒看到；慵懶偶爾也懶得理。
    pretendNotSeeChance: clamp01(playful * 0.25 + lazy * 0.1),
    // 聰明＝反應鏈更緊湊，但順序永遠是耳→眼→頭。
    attentionStagger: {
      earMs: 0,
      gazeMs: Math.round(40 + (1 - smart) * 80),
      headMs: Math.round(40 + (1 - smart) * 80 + 60 + (1 - smart) * 140),
    },
    riseDelayMs: Math.round(lazy * 600),
  };
}

/** 便利組合：偏好 → tuning。 */
export function tuningForPreferences(
  expressiveness: string | null | undefined,
  personaId?: string | null,
  variantWeightTable: VariantWeightTable = {}
): PersonalityTuning {
  return tuningFor(personalityFor(expressiveness, personaId), variantWeightTable);
}

export const DEFAULT_PERSONALITY = personalityFor("natural");
export const DEFAULT_TUNING = tuningFor(DEFAULT_PERSONALITY);

/** 目前最突出的特質（供本機模板短句選句；不呼叫 AI）。 */
export function dominantTrait(p: PersonalityProfile): PersonalityTrait {
  const order: PersonalityTrait[] = ["curious", "playful", "lazy", "proud", "witty", "smart"];
  let best: PersonalityTrait = order[0];
  for (const t of order) {
    if (p[t] > p[best]) best = t;
  }
  return best;
}
