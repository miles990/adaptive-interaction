// 送達判定（known limitation #24）：交代一件工作之後，畫面只能說出後端真的
// 給了證據的那一件事。六態各有一句人話與一句誠實註記：
// 已送達／尚未送達（已放進信箱）／排隊中／Agent 不可用／傳送失敗／結果不確定。
//
// 不變量：只有後端蓋了 `deliveredAt` 戳記才可以說「已送達」。其餘任何情況
// （空回應、409 忙碌、503 子程序不在、網路斷線）都不得宣稱送達。

import { describe, expect, it } from "vitest";
import {
  backendErrorKind,
  classifyDelivery,
  DELIVERY_LABEL,
  deliveredToAgent,
  deliveryNoticeText,
  type DeliveryOutcome,
} from "../work/delivery";

const CTX = { agentName: "Codex", taskLabel: "跑一次測試" };

/** 一般模式不得出現的技術術語（含後端錯誤字串裡的實作名詞）。 */
const JARGON = /lease|provider session|uuid|session|stdin|子程序|mailbox|resume|http|\d{3}:/i;

describe("deliveredToAgent：只認真實的送達戳記", () => {
  it("有非空的 deliveredAt 字串才算送達", () => {
    expect(deliveredToAgent({ messageId: "m-1", deliveredAt: "2026-01-01T00:00:01Z" })).toBe(true);
    expect(deliveredToAgent({ messageId: "m-1" })).toBe(false);
    expect(deliveredToAgent({ messageId: "m-1", deliveredAt: "" })).toBe(false);
    expect(deliveredToAgent({ messageId: "m-1", deliveredAt: null })).toBe(false);
    expect(deliveredToAgent({ messageId: "m-1", deliveredAt: 12345 })).toBe(false);
    expect(deliveredToAgent({ deliveredAt: true })).toBe(false);
    expect(deliveredToAgent(undefined)).toBe(false);
    expect(deliveredToAgent(null)).toBe(false);
    expect(deliveredToAgent("delivered")).toBe(false);
  });
});

describe("backendErrorKind：兩種傳輸的錯誤字串都對得上 DomainError", () => {
  it("Tauri（純 Display）與 HTTP（狀態碼前綴）視為同一種", () => {
    expect(backendErrorKind("unavailable: agent 子程序已結束")).toBe("unavailable");
    expect(backendErrorKind(new Error("503: unavailable: agent 子程序已結束"))).toBe("unavailable");
    expect(backendErrorKind("conflict: 上一輪還在跑，這則訊息未送達")).toBe("busy");
    expect(backendErrorKind("409: conflict: agent session s-1 is Closed; mailbox closed")).toBe(
      "inactive"
    );
    expect(backendErrorKind("404: not found: agent session s-1")).toBe("not-found");
    expect(backendErrorKind("400: validation failed: mailbox body too large")).toBe("validation");
    expect(backendErrorKind("403: policy blocked: session 成本預算已用盡")).toBe("policy");
    expect(backendErrorKind("423: emergency stop engaged")).toBe("policy");
    expect(backendErrorKind("410: expired: agent session s-1")).toBe("expired");
    expect(backendErrorKind("500: internal error: boom")).toBe("internal");
    expect(backendErrorKind(new TypeError("Failed to fetch"))).toBe("unrecognized");
    expect(backendErrorKind(undefined)).toBe("none");
  });
});

describe("classifyDelivery：六態", () => {
  it("已送達：後端蓋了送達戳記", () => {
    const d = classifyDelivery({
      ...CTX,
      sent: { messageId: "m-1", deliveredAt: "2026-01-01T00:00:01Z" },
    });
    expect(d.outcome).toBe("delivered");
    expect(d.label).toBe("已送達");
    expect(d.delivered).toBe(true);
    expect(d.problem).toBe(false);
    expect(d.accepted).toBe(true);
    expect(d.message).toContain("「跑一次測試」");
    expect(d.message).toContain("Codex");
    expect(d.message).toContain("尚未完成");
    expect(d.message).not.toContain("已完成。");
    expect(d.honesty.length).toBeGreaterThan(0);
    // 送達不是慶祝：不得用成功綠。
    expect(d.badge).not.toBe("ok");
  });

  it("尚未送達（已放進信箱）：送出成功但沒有戳記", () => {
    for (const sent of [{ messageId: "m-1" }, {}, { deliveredAt: null }, null]) {
      const d = classifyDelivery({ ...CTX, sent });
      expect(d.outcome, JSON.stringify(sent)).toBe("mailbox");
      expect(d.label).toBe("尚未送達（已放進信箱）");
      expect(d.delivered).toBe(false);
      expect(d.accepted).toBe(true);
      expect(d.problem).toBe(false);
      expect(d.message).not.toContain("已送達");
      expect(d.message).toContain("信箱");
    }
  });

  it("排隊中：上一輪還在跑（409 conflict）", () => {
    const d = classifyDelivery({
      ...CTX,
      error: new Error("409: conflict: 上一輪還在跑，這則訊息未送達；稍後再送或先中斷：busy"),
    });
    expect(d.outcome).toBe("queued");
    expect(d.label).toBe("排隊中");
    expect(d.delivered).toBe(false);
    expect(d.accepted).toBe(false);
    expect(d.message).not.toContain("已送達");
    expect(d.message).toContain("還沒送到");
    expect(d.honesty).toContain("再送一次");
  });

  it("Agent 不可用：子程序已結束／無回應／工作已關閉／已到期", () => {
    const errors = [
      "unavailable: agent 子程序已結束，這則訊息未送達；請續開（resume）或建立新的 session",
      "503: unavailable: agent 子程序無回應（stdin 阻塞），這則訊息未送達",
      "409: conflict: agent session s-1 is Closed; mailbox closed",
      "410: expired: agent session s-1",
    ];
    for (const error of errors) {
      const d = classifyDelivery({ ...CTX, error });
      expect(d.outcome, error).toBe("agent-unavailable");
      expect(d.label).toBe("Agent 不可用");
      expect(d.delivered).toBe(false);
      expect(d.problem).toBe(true);
      expect(d.message).not.toContain("已送達");
      // 一般模式不外洩後端術語；原文只留在 detail 給進階模式。
      expect(d.message).not.toMatch(JARGON);
      expect(d.honesty).not.toMatch(JARGON);
      expect(d.detail).toBe(error);
    }
  });

  it("傳送失敗：找不到、內容不合法、被安全規則擋下", () => {
    const errors = [
      "404: not found: agent session s-1",
      "400: validation failed: mailbox body too large (max 65536 bytes)",
      "403: policy blocked: session 成本預算已用盡，不再開新 turn；這則訊息未送達",
      "423: emergency stop engaged",
    ];
    for (const error of errors) {
      const d = classifyDelivery({ ...CTX, error });
      expect(d.outcome, error).toBe("send-failed");
      expect(d.label).toBe("傳送失敗");
      expect(d.delivered).toBe(false);
      expect(d.problem).toBe(true);
      expect(d.message).not.toMatch(JARGON);
    }
  });

  it("結果不確定：連不上、後端內部錯、完全沒有證據", () => {
    for (const error of [new TypeError("Failed to fetch"), "500: internal error: boom", "boom"]) {
      const d = classifyDelivery({ ...CTX, error });
      expect(d.outcome, String(error)).toBe("uncertain");
      expect(d.label).toBe("結果不確定");
      expect(d.delivered).toBe(false);
      expect(d.problem).toBe(true);
      expect(d.message).toContain("不確定");
      expect(d.message).not.toContain("已送達");
    }
    // 什麼證據都沒有（既沒有回傳也沒有錯誤）：不猜。
    const none = classifyDelivery({ ...CTX });
    expect(none.outcome).toBe("uncertain");
  });

  it("六態的標籤互不重複，也沒有任何一態把未送達說成已送達", () => {
    const labels = Object.values(DELIVERY_LABEL);
    expect(new Set(labels).size).toBe(labels.length);
    expect(labels).toHaveLength(6);
    const notDelivered: DeliveryOutcome[] = [
      "mailbox",
      "queued",
      "agent-unavailable",
      "send-failed",
      "uncertain",
    ];
    for (const outcome of notDelivered) {
      expect(DELIVERY_LABEL[outcome]).not.toBe("已送達");
    }
  });

  it("每一態的人話與誠實註記都沒有技術術語", () => {
    const inputs = [
      { sent: { deliveredAt: "2026-01-01T00:00:01Z" } },
      { sent: {} },
      { error: "conflict: 上一輪還在跑" },
      { error: "unavailable: agent 子程序已結束" },
      { error: "not found: agent session s-1" },
      { error: "boom" },
    ];
    for (const input of inputs) {
      const d = classifyDelivery({ ...CTX, ...input });
      expect(d.message, d.outcome).not.toMatch(JARGON);
      expect(d.honesty, d.outcome).not.toMatch(JARGON);
      expect(d.honesty.length, d.outcome).toBeGreaterThan(0);
    }
  });

  it("沒有工作名稱／對象名稱時句子仍然完整", () => {
    const d = classifyDelivery({ sent: { deliveredAt: "2026-01-01T00:00:01Z" } });
    expect(d.message).toContain("這次的交代");
    expect(d.message).toContain("工作助手");
    expect(d.message).not.toContain("undefined");
  });

  it("建立階段（工作根本沒開始）用不同的句子，不說「沒能送出去」", () => {
    const d = classifyDelivery({ ...CTX, stage: "create", error: "unavailable: agent 子程序已結束" });
    expect(d.outcome).toBe("agent-unavailable");
    expect(d.message).toContain("沒有開始");
    expect(d.message).not.toContain("送出去");
    expect(d.honesty).not.toContain("工作卡片");
  });

  it("送出階段的誠實註記會指向工作卡片（工作已經建立了）", () => {
    const d = classifyDelivery({ ...CTX, error: "unavailable: agent 子程序已結束" });
    expect(d.honesty).toContain("工作卡片");
  });
});

describe("deliveryNoticeText：一句話的通知文字", () => {
  it("一般模式＝人話＋誠實註記；不含後端原文", () => {
    const d = classifyDelivery({ ...CTX, error: "unavailable: agent 子程序已結束" });
    const text = deliveryNoticeText(d);
    expect(text).toContain(d.message);
    expect(text).toContain(d.honesty);
    expect(text).not.toContain("子程序");
  });

  it("進階模式才附上後端原文", () => {
    const d = classifyDelivery({ ...CTX, error: "unavailable: agent 子程序已結束" });
    const text = deliveryNoticeText(d, { advanced: true });
    expect(text).toContain("unavailable: agent 子程序已結束");
  });

  it("沒有 detail 時進階模式不會多出空括號", () => {
    const d = classifyDelivery({ ...CTX, sent: { deliveredAt: "2026-01-01T00:00:01Z" } });
    expect(deliveryNoticeText(d, { advanced: true })).toBe(`${d.message} ${d.honesty}`);
  });
});
