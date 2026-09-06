import { describe, expect, it } from "vitest";
import { beginPresetOp } from "../companion/applyPresetPlan";

describe("preset operation identity", () => {
  it("two user operations in the same millisecond must not share an identity", () => {
    const a = beginPresetOp("natural", 1700000000000);
    const b = beginPresetOp("natural", 1700000000000);
    expect(a?.opId).not.toBe(b?.opId);
  });
});
