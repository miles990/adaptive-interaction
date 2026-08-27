// Persona / world / story packs: data-only JSON that restyles NON-safety
// expression. Hard rules enforced here, not by pack authors:
//   - Safety lines (emergency / blocked / unknown / sensor-in-use) are FIXED
//     standard wording; packs cannot override or hide them.
//   - Packs carry no executable content; only known fields are read, length-
//     capped, and everything else is ignored.
//   - Story chapters are skippable, low-frequency, fire once, and never guilt
//     the user for leaving or closing the companion.

export interface PersonaPack {
  schemaVersion: string;
  kind: "persona-pack";
  id: string;
  name: Record<string, string>;
  author?: string;
  version?: string;
  license?: string;
  /** intent key → candidate lines (non-safety keys only). */
  lines: Record<string, string[]>;
}

export interface StoryChapter {
  id: string;
  trigger: "first-meeting" | "first-verified-success";
  line: string;
  skippable?: boolean;
}

export interface StoryPack {
  schemaVersion: string;
  kind: "story-pack";
  id: string;
  name: Record<string, string>;
  chapters: StoryChapter[];
}

/** §17 知識收據六句固定文案的 key（角色端知識進度語句）。
 *  每一句都是對知識狀態（發布／複審／驗證）的誠實宣稱，因此與安全語句
 *  走同一不可覆寫機制——pack 不得把「候選」說成「已發布」、把「未驗證」
 *  說成「已驗證」。 */
export const KNOWLEDGE_RECEIPT_KEYS = [
  "knowledge-new-material",
  "knowledge-candidate-created",
  "knowledge-review-completed",
  "knowledge-published",
  "knowledge-stale",
  "knowledge-agent-unverified",
] as const;

/** Keys whose wording is safety-critical and therefore immutable. */
export const SAFETY_KEYS = [
  "emergency",
  "blocked",
  "unknown",
  "failed",
  "sensor-microphone",
  "sensor-camera",
  // The VERIFIED-success line makes a verification claim, so a pack must not be
  // able to restyle it (or move it onto an unverified state). The plain
  // `succeeded` (completed) line stays pack-restylable — that is the feature.
  "succeeded-verified",
  // §17 知識六句：同樣是狀態宣稱，不可被 persona/world/story 覆寫。
  ...KNOWLEDGE_RECEIPT_KEYS,
] as const;

/** Fixed standard wording (never overridable by any pack). */
export const FIXED_SAFETY_LINES: Record<string, string> = {
  emergency: "緊急停止中",
  blocked: "這個動作超出目前允許範圍，所以我沒有執行。",
  unknown: "要求已送出，但目前無法確認是否真的完成。",
  // A definitive failure is distinct from "unknown" — never conflate them.
  failed: "這個動作失敗了。",
  "sensor-microphone": "🎙 正在使用麥克風",
  "sensor-camera": "📷 正在使用攝影機",
  // Verified success — its wording is fixed so no pack can claim verification
  // it didn't earn (shown only on action.observed).
  "succeeded-verified": "做完了，也確認過結果。",
  // §17 知識收據六句（spec 指定的固定文案；選句邏輯見
  // companion/knowledgeReceipts.ts——依 receipt payload 確定性選句）。
  "knowledge-new-material": "我找到了新素材。",
  "knowledge-candidate-created": "我建立了知識候選。",
  "knowledge-review-completed": "候選已完成複審。",
  "knowledge-published": "知識已正式發布。",
  "knowledge-stale": "這項知識已過期，需要確認。",
  "knowledge-agent-unverified": "Agent 回報完成，但尚未驗證。",
};

/** Default (小樞) non-safety lines. */
export const DEFAULT_LINES: Record<string, string[]> = {
  succeeded: ["做完了。", "這一段收尾了。"],
  "succeeded-verified": ["做完了，也確認過結果。"],
  paused: ["主動互動暫停中。"],
  offline: ["目前連不上系統。"],
  "pause-ack": ["好的，接下來一小時我不會主動打擾。"],
  "text-received": ["收到，我記下了。"],
  "drop-received": ["記下這些檔案了。"],
  delegated: ["我把這件事交給工作階段了。它收到後才算送達。"],
  "first-meeting": ["你好，我是小樞。我只會在你允許的範圍內幫忙留意事情。"],
};

const MAX_LINE_CHARS = 200;
const MAX_LINES_PER_KEY = 12;
const MAX_KEYS = 64;

export function validatePersonaPack(raw: unknown): string[] {
  const issues: string[] = [];
  const p = raw as Partial<PersonaPack>;
  if (!p || typeof p !== "object") return ["pack is not an object"];
  if (p.kind !== "persona-pack") issues.push("kind must be persona-pack");
  if (!p.schemaVersion) issues.push("schemaVersion missing");
  if (!p.id || !/^[a-z0-9][a-z0-9-]{0,63}$/.test(p.id)) issues.push("invalid id");
  const lines = p.lines ?? {};
  if (typeof lines !== "object" || Array.isArray(lines)) {
    issues.push("lines must be an object");
    return issues;
  }
  const keys = Object.keys(lines);
  if (keys.length > MAX_KEYS) issues.push(`too many line keys (max ${MAX_KEYS})`);
  for (const key of keys) {
    if ((SAFETY_KEYS as readonly string[]).includes(key)) {
      issues.push(`line key "${key}" is safety-critical and cannot be overridden`);
    }
    const variants = lines[key];
    if (!Array.isArray(variants) || variants.length === 0) {
      issues.push(`lines["${key}"] must be a non-empty array`);
      continue;
    }
    if (variants.length > MAX_LINES_PER_KEY)
      issues.push(`lines["${key}"] has too many variants (max ${MAX_LINES_PER_KEY})`);
    for (const v of variants) {
      if (typeof v !== "string") issues.push(`lines["${key}"] contains a non-string`);
      else if (v.length > MAX_LINE_CHARS)
        issues.push(`lines["${key}"] line too long (max ${MAX_LINE_CHARS} chars)`);
    }
  }
  return issues;
}

export function validateStoryPack(raw: unknown): string[] {
  const issues: string[] = [];
  const p = raw as Partial<StoryPack>;
  if (!p || typeof p !== "object") return ["pack is not an object"];
  if (p.kind !== "story-pack") issues.push("kind must be story-pack");
  if (!p.schemaVersion) issues.push("schemaVersion missing");
  if (!p.id || !/^[a-z0-9][a-z0-9-]{0,63}$/.test(p.id)) issues.push("invalid id");
  const chapters = p.chapters ?? [];
  if (!Array.isArray(chapters)) return [...issues, "chapters must be an array"];
  if (chapters.length > 32) issues.push("too many chapters (max 32)");
  for (const ch of chapters) {
    if (!ch.id) issues.push("chapter missing id");
    if (!["first-meeting", "first-verified-success"].includes(ch.trigger))
      issues.push(`chapter ${ch.id}: unknown trigger ${String(ch.trigger)}`);
    if (typeof ch.line !== "string" || ch.line.length === 0 || ch.line.length > MAX_LINE_CHARS)
      issues.push(`chapter ${ch.id}: line must be 1..${MAX_LINE_CHARS} chars`);
  }
  return issues;
}

/** Resolve one line. Safety keys ALWAYS return the fixed wording — even if a
 *  (buggy or malicious) pack carries an entry for them. */
export function resolveLine(
  key: string,
  persona: PersonaPack | null,
  pick: (n: number) => number = (n) => Math.floor(Math.random() * n)
): string | null {
  if (key in FIXED_SAFETY_LINES) {
    return FIXED_SAFETY_LINES[key];
  }
  const fromPack = Array.isArray(persona?.lines?.[key])
    ? (persona!.lines[key] as string[]).filter(
        (v) => typeof v === "string" && v.length > 0 && v.length <= MAX_LINE_CHARS
      )
    : [];
  const candidates = fromPack.length > 0 ? fromPack : DEFAULT_LINES[key];
  if (!candidates || candidates.length === 0) return null;
  return candidates[pick(candidates.length) % candidates.length];
}

/** Expressiveness → behavior tuning (behavior-pack defaults built in). */
export interface BehaviorTuning {
  blinkIntervalMs: number;
  bubbleCooldownMs: number;
  /** quiet mode suppresses all non-safety bubbles. */
  allowCasualBubbles: boolean;
}

export function behaviorFor(expressiveness: string): BehaviorTuning {
  switch (expressiveness) {
    case "quiet":
      return { blinkIntervalMs: 9000, bubbleCooldownMs: 60_000, allowCasualBubbles: false };
    case "lively":
      return { blinkIntervalMs: 3500, bubbleCooldownMs: 5_000, allowCasualBubbles: true };
    default:
      return { blinkIntervalMs: 5500, bubbleCooldownMs: 8_000, allowCasualBubbles: true };
  }
}

/** Story progression: which chapter (if any) fires for an event, given
 *  already-seen progress. Chapters fire ONCE; nothing repeats or guilts. */
export function nextChapter(
  story: StoryPack | null,
  trigger: StoryChapter["trigger"],
  seen: Record<string, boolean>
): StoryChapter | null {
  if (!story) return null;
  for (const ch of story.chapters) {
    if (ch.trigger === trigger && !seen[ch.id]) {
      return ch;
    }
  }
  return null;
}
