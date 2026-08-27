// §17 知識收據 → 角色端六句固定文案（原已知限制⑨的 UI 面）。
//
// 純函式：依 knowledge.updated 事件的 receipt payload
// （published／verification／changes／triggeredBy／agentSessions／sources）
// 確定性選句。句子本體在 packs.FIXED_SAFETY_LINES——與安全語句同一機制，
// persona/world/story pack 不可覆寫。
//
// 誠實原則：每一句都是狀態宣稱，因此「沒有吻合的句子就沉默（null）」——
// 例如衝突收據（disputed）六句都不涵蓋，寧可不說也不硬湊；
// 收據語意在控制中心的「知識收據」頁永遠完整呈現。

import { FIXED_SAFETY_LINES } from "./packs";

function rec(v: unknown): Record<string, unknown> {
  return v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : {};
}

function num(v: unknown): number {
  return typeof v === "number" && Number.isFinite(v) ? v : 0;
}

function arrLen(v: unknown): number {
  return Array.isArray(v) ? v.length : 0;
}

export type KnowledgeLineKey =
  | "knowledge-new-material"
  | "knowledge-candidate-created"
  | "knowledge-review-completed"
  | "knowledge-published"
  | "knowledge-stale"
  | "knowledge-agent-unverified";

/** 決策表（由上而下，第一個吻合者生效——順序即優先序，全部確定性）：
 *  1. staleMarked > 0                       → 過期需確認（最需要使用者行動）
 *  2. published.claims === true             → 已正式發布（最終狀態優先於過程）
 *  3. verification.humanReviewed === true   → 候選已完成複審（含複審後拒絕）
 *  4. task-experience／agentSessions 非空   → Agent 回報完成，尚未驗證
 *     （agent 任務結束的回報：claimed ≠ verified）
 *  5. candidatesCreated > 0                 → 建立了知識候選
 *  6. sources／sourceHashes 非空            → 找到了新素材
 *  其餘（如 disputed-only 的衝突收據）→ null：六句沒有誠實對應者，不說。 */
export function knowledgeReceiptLineKey(
  payload: Record<string, unknown>
): KnowledgeLineKey | null {
  const changes = rec(payload["changes"]);
  const verification = rec(payload["verification"]);
  const published = rec(payload["published"]);
  const triggeredBy = String(payload["triggeredBy"] ?? "");

  if (num(changes["staleMarked"]) > 0) return "knowledge-stale";
  if (published["claims"] === true) return "knowledge-published";
  if (verification["humanReviewed"] === true) return "knowledge-review-completed";
  if (triggeredBy === "task-experience" || arrLen(payload["agentSessions"]) > 0) {
    return "knowledge-agent-unverified";
  }
  if (num(changes["candidatesCreated"]) > 0) return "knowledge-candidate-created";
  if (arrLen(payload["sources"]) > 0 || arrLen(payload["sourceHashes"]) > 0) {
    return "knowledge-new-material";
  }
  return null;
}

/** key → 固定文案（永遠取自 FIXED_SAFETY_LINES，pack 無從介入）。 */
export function knowledgeReceiptLine(payload: Record<string, unknown>): string | null {
  const key = knowledgeReceiptLineKey(payload);
  return key ? FIXED_SAFETY_LINES[key] : null;
}

/** 同一 receipt 只說一次的去重器。有界（FIFO 淘汰，預設 300 筆）——
 *  禁無界成長；SSE 重連會重放舊事件，靠 updateId 擋住重複發言。
 *  沒有 updateId 的 payload 一律回 false：無法去重就不宣稱。 */
export function createReceiptDedup(cap = 300): (updateId: string) => boolean {
  const seen = new Set<string>();
  const order: string[] = [];
  return (updateId: string) => {
    if (!updateId) return false;
    if (seen.has(updateId)) return false;
    seen.add(updateId);
    order.push(updateId);
    if (order.length > cap) {
      const oldest = order.shift();
      if (oldest !== undefined) seen.delete(oldest);
    }
    return true;
  };
}
