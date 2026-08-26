// 誠實性不變量：任何狀態文案都不得往上美化。
// queued ≠ completed；acknowledged ≠ delivered；delivered ≠ 使用者已看見。

import { describe, expect, it } from "vitest";
import { actionStatusLabel, confirmationLabel, triLabel } from "../appstate";

describe("actionStatusLabel", () => {
  it("never displays queued/accepted as completed", () => {
    expect(actionStatusLabel("accepted")).toContain("尚未執行");
    expect(actionStatusLabel("accepted")).not.toContain("完成");
    expect(actionStatusLabel("dispatched")).not.toContain("完成");
  });

  it("acknowledged is not shown as delivered or completed", () => {
    const label = actionStatusLabel("acknowledged");
    expect(label).toContain("效果未確認");
    expect(label).not.toContain("完成");
    expect(label).not.toContain("送達");
  });

  it("uncertain is honest", () => {
    expect(actionStatusLabel("uncertain")).toBe("結果未知");
  });

  it("blocked names the policy, not a failure of the device", () => {
    expect(actionStatusLabel("blocked")).toContain("安全規則");
  });
});

describe("confirmationLabel", () => {
  it("delivered admits the user may not have seen it", () => {
    const { cannot } = confirmationLabel("delivered");
    expect(cannot).toContain("看見");
  });

  it("queued admits execution is unconfirmed", () => {
    const { cannot } = confirmationLabel("queued");
    expect(cannot).toContain("無法確認");
  });

  it("unknown level degrades conservatively", () => {
    const { cannot } = confirmationLabel("unknown");
    expect(cannot).not.toBe("");
  });
});

describe("triLabel", () => {
  it("unknown never maps to the safe label", () => {
    expect(triLabel("unknown", "會", "不會", "未知")).toBe("未知");
    expect(triLabel(false, "會", "不會", "未知")).toBe("不會");
    expect(triLabel(true, "會", "不會", "未知")).toBe("會");
  });
});
