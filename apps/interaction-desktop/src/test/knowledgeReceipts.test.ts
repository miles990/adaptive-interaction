import { describe, expect, it } from "vitest";
import {
  createReceiptDedup,
  knowledgeReceiptLine,
  knowledgeReceiptLineKey,
} from "../companion/knowledgeReceipts";

describe("knowledge receipt companion wording", () => {
  it("uses the deterministic honesty priority", () => {
    expect(
      knowledgeReceiptLineKey({
        changes: { staleMarked: 1, candidatesCreated: 2 },
        published: { claims: true },
      })
    ).toBe("knowledge-stale");
    expect(knowledgeReceiptLineKey({ published: { claims: true } })).toBe("knowledge-published");
    expect(knowledgeReceiptLineKey({ verification: { humanReviewed: true } })).toBe(
      "knowledge-review-completed"
    );
    expect(knowledgeReceiptLineKey({ agentSessions: ["session-1"] })).toBe(
      "knowledge-agent-unverified"
    );
    expect(knowledgeReceiptLineKey({ changes: { candidatesCreated: 1 } })).toBe(
      "knowledge-candidate-created"
    );
    expect(knowledgeReceiptLineKey({ sourceHashes: ["sha256:x"] })).toBe(
      "knowledge-new-material"
    );
    expect(knowledgeReceiptLineKey({ changes: { disputedMarked: 1 } })).toBeNull();
  });

  it("never treats an agent task receipt as verified", () => {
    expect(knowledgeReceiptLine({ triggeredBy: "task-experience" })).toContain("尚未驗證");
    expect(knowledgeReceiptLine({ triggeredBy: "task-experience" })).not.toContain("已正式發布");
  });

  it("deduplicates with a bounded FIFO and rejects receipts without identity", () => {
    const accept = createReceiptDedup(2);
    expect(accept("")).toBe(false);
    expect(accept("a")).toBe(true);
    expect(accept("a")).toBe(false);
    expect(accept("b")).toBe(true);
    expect(accept("c")).toBe(true);
    expect(accept("a")).toBe(true); // a 已依容量淘汰
  });
});
