// v0.4 對抗審查（frontend cluster）確認缺陷的 regression tests：
// 緊急停止不可單鍵誤觸／IME 防護、指令失敗不得靜默、匯出必須真的呈現、
// 拖放「已記錄」要等實際結果、關閉文案誠實、計數上限與查詢失敗誠實顯示、
// 倒數要真的走、權限投影不可被事件流餓死、自動互動計數吃 refreshKey、
// 相容 tab 標題與導覽高亮。

import { afterEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { api, AgentSessionRecord } from "../api";
import { AppStateProvider } from "../appstate";
import { GlobalSearch } from "../components/GlobalSearch";
import { MemoryKnowledgePage } from "../pages/MemoryKnowledgePage";
import { recordDroppedItems } from "../companion/CompanionApp";
import { AiPage } from "../pages/AiPage";
import { InboxSection } from "../pages/ActivityPage";
import { NowStrip, ProactiveSummary } from "../pages/HomePage";
import { navAnchorFor, SensorCountdown, titleFor } from "../App";

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

// GlobalSearch 的動態載入（session／記憶／知識）全部給空結果。
function stubPaletteData() {
  vi.spyOn(api, "agentSessionsList").mockResolvedValue([]);
  vi.spyOn(api, "memoryList").mockResolvedValue({ items: [] });
  vi.spyOn(api, "knowledgeList").mockResolvedValue({ nodes: [], count: 0 });
}

function renderPalette(overrides?: {
  onEstop?: () => Promise<void>;
  onCommandFeedback?: (message: string, ok: boolean) => void;
  onClose?: () => void;
  onNavigate?: (tab: string) => void;
}) {
  const onEstop = overrides?.onEstop ?? vi.fn(async () => {});
  const onCommandFeedback = overrides?.onCommandFeedback ?? vi.fn();
  const onClose = overrides?.onClose ?? vi.fn();
  const onNavigate = overrides?.onNavigate ?? vi.fn();
  render(
    <AppStateProvider ready={false} refreshKey={0}>
      <GlobalSearch
        open
        onClose={onClose}
        onNavigate={onNavigate}
        estopped={false}
        onEstop={onEstop}
        onCommandFeedback={onCommandFeedback}
      />
    </AppStateProvider>
  );
  return { onEstop, onCommandFeedback, onClose, onNavigate };
}

describe("GlobalSearch 緊急停止二段確認（不可單鍵誤觸）", () => {
  it("開啟面板直接按 Enter 只進入確認態，不執行緊急停止", async () => {
    stubPaletteData();
    const { onEstop, onClose } = renderPalette();
    const input = screen.getByPlaceholderText(/搜尋設定/);
    fireEvent.keyDown(input, { key: "Enter" });
    // 第一下絕不觸發；面板保持開啟等待確認。
    expect(onEstop).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
    expect(await screen.findByText("立即停止一切？")).toBeInTheDocument();
  });

  it("確認態再按一次 Enter 才執行並導向安全頁", async () => {
    stubPaletteData();
    const { onEstop, onNavigate } = renderPalette();
    const input = screen.getByPlaceholderText(/搜尋設定/);
    fireEvent.keyDown(input, { key: "Enter" });
    await screen.findByText("立即停止一切？");
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(onEstop).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(onNavigate).toHaveBeenCalledWith("safety"));
  });

  it("IME 組字的 Enter（isComposing）不執行、也不進入確認態", () => {
    stubPaletteData();
    const { onEstop, onClose } = renderPalette();
    const input = screen.getByPlaceholderText(/搜尋設定/);
    fireEvent.keyDown(input, { key: "Enter", isComposing: true });
    expect(onEstop).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.queryByText("立即停止一切？")).not.toBeInTheDocument();
  });

  it("keyCode 229（WebKit IME commit）同樣被忽略", () => {
    stubPaletteData();
    const { onEstop } = renderPalette();
    const input = screen.getByPlaceholderText(/搜尋設定/);
    fireEvent.keyDown(input, { key: "Enter", keyCode: 229 });
    expect(onEstop).not.toHaveBeenCalled();
    expect(screen.queryByText("立即停止一切？")).not.toBeInTheDocument();
  });
});

describe("GlobalSearch 指令結果回報（失敗不得靜默）", () => {
  it("停止所有感測失敗時回報錯誤訊息", async () => {
    stubPaletteData();
    vi.spyOn(api, "sensorsStop").mockRejectedValue(new Error("daemon offline"));
    const { onCommandFeedback } = renderPalette();
    const input = screen.getByPlaceholderText(/搜尋設定/);
    fireEvent.change(input, { target: { value: "停止所有感測" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() =>
      expect(onCommandFeedback).toHaveBeenCalledWith(
        expect.stringContaining("停止所有感測"),
        false
      )
    );
    expect(String(vi.mocked(onCommandFeedback).mock.calls[0][0])).toContain("失敗");
  });

  it("暫停主動互動成功時回報成功訊息", async () => {
    stubPaletteData();
    vi.spyOn(api, "pauseSet").mockResolvedValue({ paused: true });
    const { onCommandFeedback } = renderPalette();
    const input = screen.getByPlaceholderText(/搜尋設定/);
    fireEvent.change(input, { target: { value: "暫停主動互動" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() =>
      expect(onCommandFeedback).toHaveBeenCalledWith("已暫停主動互動一小時。", true)
    );
  });
});

describe("MemoryKnowledgePage 匯出", () => {
  it("匯出結果真的呈現在畫面上（不是只寫 console）", async () => {
    vi.spyOn(api, "memoryList").mockResolvedValue({ items: [] });
    vi.spyOn(api, "memoryExport").mockResolvedValue({
      count: 2,
      items: [{ title: "demo-item-1" }, { title: "demo-item-2" }],
    });
    render(<MemoryKnowledgePage refreshKey={0} />);
    await userEvent.click(await screen.findByRole("button", { name: "匯出全部" }));
    expect(await screen.findByText("匯出結果")).toBeInTheDocument();
    expect(screen.getByText(/demo-item-1/)).toBeInTheDocument();
    expect(screen.getByText(/已匯出 2 條/)).toBeInTheDocument();
  });

  it("匯出失敗時誠實回報，不顯示匯出結果", async () => {
    vi.spyOn(api, "memoryList").mockResolvedValue({ items: [] });
    vi.spyOn(api, "memoryExport").mockRejectedValue(new Error("export boom"));
    render(<MemoryKnowledgePage refreshKey={0} />);
    await userEvent.click(await screen.findByRole("button", { name: "匯出全部" }));
    expect(await screen.findByText(/匯出失敗/)).toBeInTheDocument();
    expect(screen.queryByText("匯出結果")).not.toBeInTheDocument();
  });
});

describe("小樞拖放記錄（等待實際結果）", () => {
  const lineStub = (key: string) => (key === "drop-received" ? "記下這些檔案了。" : null);

  it("push 成功才顯示成功語，並送到 companion.drag-drop", async () => {
    const push = vi.fn().mockResolvedValue({});
    const showBubble = vi.fn();
    const ok = await recordDroppedItems(["/tmp/a.txt"], push, showBubble, lineStub);
    expect(ok).toBe(true);
    expect(push).toHaveBeenCalledWith(
      "companion.drag-drop",
      expect.objectContaining({ kind: "companion-dropped", attachments: ["/tmp/a.txt"] }),
      1.0
    );
    expect(showBubble).toHaveBeenCalledWith("記下這些檔案了。", 3000);
  });

  it("push 失敗顯示失敗語，絕不顯示「記下了」", async () => {
    const push = vi.fn().mockRejectedValue(new Error("receptor disabled"));
    const showBubble = vi.fn();
    const ok = await recordDroppedItems(["/tmp/a.txt"], push, showBubble, lineStub);
    expect(ok).toBe(false);
    const text = String(showBubble.mock.calls[0][0]);
    expect(text).toContain("記錄失敗");
    expect(text).not.toContain("記下這些檔案了");
  });
});

describe("AiPage 關閉工作階段文案", () => {
  const record: AgentSessionRecord = {
    sessionId: "s-1",
    providerId: "p-1",
    agentId: "claude-code",
    state: "active",
    lease: { issuedAt: "2026-01-01T00:00:00Z", expiresAt: "2026-01-01T01:00:00Z", renewable: true },
    dataScope: [],
    toolScope: [],
    consentScope: [],
    budget: { maxMessages: 10, spentMessages: 0, maxCost: 0, spentCost: 0 },
    createdAt: "2026-01-01T00:00:00Z",
  };

  it("只宣稱「已要求終止子程序」，不得宣稱子程序已終止", async () => {
    vi.spyOn(api, "agentsDiscoveries").mockResolvedValue({ agents: [] });
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([record]);
    vi.spyOn(api, "agentSessionClose").mockResolvedValue(record);
    render(<AiPage refreshKey={0} onNavigate={() => {}} />);
    await userEvent.click(await screen.findByRole("button", { name: "關閉" }));
    expect(
      await screen.findByText("工作階段已關閉（已要求終止子程序）。")
    ).toBeInTheDocument();
    expect(screen.queryByText(/子程序已終止/)).not.toBeInTheDocument();
  });
});

describe("待我決定計數誠實（收件匣與 NowStrip）", () => {
  it("收件匣：查詢失敗顯示「無法確認」，不顯示綠色的沒有事項", async () => {
    vi.spyOn(api, "aiAssistsList").mockRejectedValue(new Error("assists down"));
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([]);
    vi.spyOn(api, "knowledgeList").mockResolvedValue({ nodes: [], count: 0 });
    render(<InboxSection refreshKey={0} onNavigate={() => {}} />);
    expect(await screen.findByText("待我決定（無法確認）")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("部分狀態查詢失敗");
    expect(screen.queryByText("目前沒有等待你決定的事項。")).not.toBeInTheDocument();
  });

  it("收件匣：知識候選達查詢上限顯示 50+，不假裝精確", async () => {
    vi.spyOn(api, "aiAssistsList").mockResolvedValue([]);
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([]);
    vi.spyOn(api, "knowledgeList").mockResolvedValue({ nodes: [], count: 50 });
    render(<InboxSection refreshKey={0} onNavigate={() => {}} />);
    expect(await screen.findByText("待我決定（50+）")).toBeInTheDocument();
    expect(screen.getByText("50+ 項等待複審")).toBeInTheDocument();
  });

  it("NowStrip：查詢失敗顯示無法確認，不顯示綠色 0 項", async () => {
    vi.spyOn(api, "agentSessionsList").mockRejectedValue(new Error("sessions down"));
    vi.spyOn(api, "aiAssistsList").mockResolvedValue([]);
    vi.spyOn(api, "knowledgeList").mockResolvedValue({ nodes: [], count: 0 });
    vi.spyOn(api, "knowledgeReceipts").mockResolvedValue({ receipts: [] });
    render(<NowStrip refreshKey={0} status={{}} onNavigate={() => {}} />);
    expect(await screen.findByText("無法確認（查詢失敗）")).toBeInTheDocument();
    expect(screen.queryByText("0 項")).not.toBeInTheDocument();
  });

  it("NowStrip：候選達上限顯示 N+ 項", async () => {
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([]);
    vi.spyOn(api, "aiAssistsList").mockResolvedValue([]);
    vi.spyOn(api, "knowledgeList").mockResolvedValue({ nodes: [], count: 50 });
    vi.spyOn(api, "knowledgeReceipts").mockResolvedValue({ receipts: [] });
    render(<NowStrip refreshKey={0} status={{}} onNavigate={() => {}} />);
    expect(await screen.findByText("50+ 項")).toBeInTheDocument();
  });
});

describe("感測倒數真的走", () => {
  it("每秒遞減顯示剩餘秒數", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-01-01T00:00:00Z"));
    const autoStopAt = new Date(Date.now() + 10_000).toISOString();
    const { container } = render(<SensorCountdown autoStopAt={autoStopAt} />);
    expect(container.textContent).toBe("・10 秒後自動停止");
    act(() => {
      vi.advanceTimersByTime(3000);
    });
    expect(container.textContent).toBe("・7 秒後自動停止");
    act(() => {
      vi.advanceTimersByTime(60_000);
    });
    // 過期後停在 0，不出現負數。
    expect(container.textContent).toBe("・0 秒後自動停止");
  });
});

describe("AppState 權限投影不可被事件流餓死", () => {
  it("持續事件流下仍至少每秒重投影一次", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-01-01T00:00:00Z"));
    vi.spyOn(api, "uiPrefsGet").mockResolvedValue({
      mode: "simple",
      locale: "zh-TW",
      customNames: {},
      schemaVersion: "1.0",
    });
    vi.spyOn(api, "pauseGet").mockResolvedValue({ paused: false });
    const humanSpy = vi
      .spyOn(api, "capabilitiesHuman")
      .mockResolvedValue({ receptors: [], actuators: [], toolOperations: [] } as never);

    const ui = (key: number) => (
      <AppStateProvider ready refreshKey={key}>
        <div />
      </AppStateProvider>
    );
    const { rerender } = render(ui(0));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    // 模擬每 100ms 一個 runtime 事件，持續 3 秒（純 trailing debounce 會
    // 不斷重置而永不觸發；修正後最多落後約 1 秒）。
    for (let i = 1; i <= 30; i++) {
      rerender(ui(i));
      await act(async () => {
        await vi.advanceTimersByTimeAsync(100);
      });
    }
    expect(humanSpy.mock.calls.length).toBeGreaterThanOrEqual(3);
  });
});

describe("ProactiveSummary 吃 refreshKey", () => {
  it("refreshKey 變更會重新抓配方，啟用計數不滯留", async () => {
    const list = vi
      .spyOn(api, "recipesList")
      .mockResolvedValue([{ recipe: { enabled: true }, state: {} }]);
    const { rerender } = render(<ProactiveSummary refreshKey={0} />);
    const p = await screen.findByText(/目前啟用/);
    await waitFor(() => expect(p.textContent).toBe("目前啟用 1 個自動互動。"));
    list.mockResolvedValue([]);
    rerender(<ProactiveSummary refreshKey={1} />);
    await waitFor(() =>
      expect(screen.getByText(/目前啟用/).textContent).toBe("目前啟用 0 個自動互動。")
    );
    expect(list).toHaveBeenCalledTimes(2);
  });
});

describe("v0.3 相容 tab 的標題與導覽歸屬", () => {
  it("senses/responses/toolops 歸屬能力與裝置，標題不為空", () => {
    for (const legacy of ["senses", "responses", "toolops"]) {
      expect(navAnchorFor(legacy)).toBe("capabilities");
      expect(titleFor(legacy)).toBe("能力與裝置");
    }
  });

  it("一般與進階 tab 不受影響", () => {
    expect(navAnchorFor("home")).toBe("home");
    expect(titleFor("home")).toBe("首頁");
    expect(titleFor("adv-recipes")).toBe("配方 YAML");
  });
});
