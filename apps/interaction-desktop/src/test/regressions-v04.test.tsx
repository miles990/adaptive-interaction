// v0.4 對抗審查（frontend cluster）確認缺陷的 regression tests：
// 緊急停止不可單鍵誤觸／IME 防護、指令失敗不得靜默、匯出必須真的呈現、
// 拖放「已記錄」要等實際結果、關閉文案誠實、計數上限與查詢失敗誠實顯示、
// 倒數要真的走、權限投影不可被事件流餓死、自動互動計數吃 refreshKey、
// 相容 tab 標題與導覽高亮。

import { afterEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { api, AgentSessionRecord } from "../api";
import { AppStateProvider } from "../appstate";
import { GlobalSearch } from "../components/GlobalSearch";
import { MemoryKnowledgePage, parseSourceSegment } from "../pages/MemoryKnowledgePage";
import { BackupSection } from "../pages/BackupSection";
import { MorePage } from "../pages/MorePage";
import { recordDroppedItems } from "../companion/CompanionApp";
import { AiPage } from "../pages/AiPage";
import { SettingsPage } from "../pages/SettingsPage";
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

  it("IME 組字的 Enter（isComposing）不執行、也不進入確認態", async () => {
    stubPaletteData();
    const { onEstop, onClose } = renderPalette();
    await act(async () => {});
    const input = screen.getByPlaceholderText(/搜尋設定/);
    fireEvent.keyDown(input, { key: "Enter", isComposing: true });
    expect(onEstop).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.queryByText("立即停止一切？")).not.toBeInTheDocument();
  });

  it("keyCode 229（WebKit IME commit）同樣被忽略", async () => {
    stubPaletteData();
    const { onEstop } = renderPalette();
    await act(async () => {});
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

  // v0.5：不得再寫死「已停止所有感測。」——送出 ≠ 已停止。
  it("停止所有感測：重讀後仍在使用中就不算成功", async () => {
    stubPaletteData();
    vi.spyOn(api, "sensorsStop").mockResolvedValue({ stopped: true });
    vi.spyOn(api, "status").mockResolvedValue({
      activeSensors: [{ kind: "microphone", startedAt: "", startedBy: "desktop", purpose: "" }],
    });
    const { onCommandFeedback } = renderPalette();
    const input = screen.getByPlaceholderText(/搜尋設定/);
    fireEvent.change(input, { target: { value: "停止所有感測" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(onCommandFeedback).toHaveBeenCalled());
    const [message, ok] = vi.mocked(onCommandFeedback).mock.calls[0];
    expect(ok).toBe(false);
    expect(String(message)).toContain("仍在使用中");
    expect(String(message)).not.toContain("已停止所有感測");
  });

  it("停止所有感測：裝置沒回覆是「結果不確定」（ok=false）", async () => {
    stubPaletteData();
    vi.spyOn(api, "sensorsStop").mockResolvedValue({
      stopped: true,
      uncertain: true,
      local: { microphone: "stopped" },
      devices: [{ deviceId: "d1", name: "iPhone", outcome: "unreachable", waitedMs: 3000 }],
    });
    vi.spyOn(api, "status").mockResolvedValue({ activeSensors: [] });
    const { onCommandFeedback } = renderPalette();
    const input = screen.getByPlaceholderText(/搜尋設定/);
    fireEvent.change(input, { target: { value: "停止所有感測" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(onCommandFeedback).toHaveBeenCalled());
    const [message, ok] = vi.mocked(onCommandFeedback).mock.calls[0];
    expect(ok).toBe(false);
    expect(String(message)).toContain("結果不確定");
    expect(String(message)).toContain("iPhone");
  });

  it("停止所有感測：真的都停了才回報成功", async () => {
    stubPaletteData();
    vi.spyOn(api, "sensorsStop").mockResolvedValue({ stopped: true, uncertain: false, devices: [] });
    vi.spyOn(api, "status").mockResolvedValue({ activeSensors: [] });
    const { onCommandFeedback } = renderPalette();
    const input = screen.getByPlaceholderText(/搜尋設定/);
    fireEvent.change(input, { target: { value: "停止所有感測" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(onCommandFeedback).toHaveBeenCalledWith("已停止感測。", true));
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

// v0.5 IA：匯出／還原搬到「更多 → 備份與還原」（BackupSection）；
// 「記憶與資料」只留一個指路按鈕，不再有第二份控制項。
describe("備份與還原（BackupSection）", () => {
  it("匯出結果真的呈現在畫面上（不是只寫 console）", async () => {
    vi.spyOn(api, "memoryExport").mockResolvedValue({
      count: 2,
      items: [{ title: "demo-item-1" }, { title: "demo-item-2" }],
    });
    render(<BackupSection />);
    await userEvent.click(await screen.findByRole("button", { name: "匯出全部" }));
    expect(await screen.findByText("匯出結果")).toBeInTheDocument();
    expect(screen.getByText(/demo-item-1/)).toBeInTheDocument();
    expect(screen.getByText(/已匯出 2 條/)).toBeInTheDocument();
  });

  it("匯出失敗時誠實回報，不顯示匯出結果", async () => {
    vi.spyOn(api, "memoryExport").mockRejectedValue(new Error("export boom"));
    render(<BackupSection />);
    await userEvent.click(await screen.findByRole("button", { name: "匯出全部" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/匯出失敗/);
    expect(screen.queryByText("匯出結果")).not.toBeInTheDocument();
  });

  it("從備份檔還原時逐筆經 human-only Runtime API 驗證", async () => {
    const create = vi.spyOn(api, "memoryCreate").mockResolvedValue({});
    render(<BackupSection />);
    const file = new File(
      [JSON.stringify({ count: 1, items: [{ layer: "user-memory", kind: "preference", title: "偏好", content: "簡短回答", retention: {} }] })],
      "memory-backup.json",
      { type: "application/json" }
    );
    await userEvent.upload(screen.getByLabelText("選擇記憶備份檔"), file);
    await waitFor(() => expect(create).toHaveBeenCalledTimes(1));
    expect(create).toHaveBeenCalledWith(
      expect.objectContaining({ title: "偏好", content: "簡短回答" })
    );
    expect(await screen.findByText(/已還原 1 條/)).toBeInTheDocument();
  });

  it("「記憶與資料」不再有第二份匯出／還原，只指路到備份與還原", async () => {
    vi.spyOn(api, "memoryList").mockResolvedValue({ items: [] });
    const onNavigate = vi.fn();
    render(<MemoryKnowledgePage refreshKey={0} onNavigate={onNavigate} />);
    await screen.findByText(/沒有你不能刪除的記憶/);
    expect(screen.queryByRole("button", { name: "匯出全部" })).not.toBeInTheDocument();
    expect(screen.queryByLabelText(/選擇記憶備份/)).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "前往備份與還原" }));
    expect(onNavigate).toHaveBeenCalledWith("backup");
  });
});

describe("Source Viewer 真實媒體預覽", () => {
  it("從 Runtime preview payload 顯示圖片，不使用硬編碼假資料", async () => {
    vi.spyOn(api, "memoryList").mockResolvedValue({ items: [] });
    vi.spyOn(api, "assetsList").mockResolvedValue({
      assets: [
        {
          hash: "a".repeat(64),
          mediaType: "image",
          sizeBytes: 68,
          source: "user-import",
          originalName: "pixel.png",
        },
      ],
      count: 1,
    });
    vi.spyOn(api, "assetPreview").mockResolvedValue({
      hash: "a".repeat(64),
      mediaType: "image",
      mime: "image/png",
      sizeBytes: 68,
      dataBase64: "iVBORw0KGgo=",
      note: "runtime payload",
    });
    render(<MemoryKnowledgePage refreshKey={0} />);
    await userEvent.click(screen.getByRole("tab", { name: "素材與來源" }));
    await userEvent.click(await screen.findByRole("button", { name: "開啟來源" }));
    const viewer = await screen.findByTestId("source-media-viewer");
    expect(within(viewer).getByRole("img")).toHaveAttribute(
      "src",
      "data:image/png;base64,iVBORw0KGgo="
    );
    expect(viewer).toHaveTextContent("runtime payload");
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
    render(
      <AppStateProvider ready={false} refreshKey={0}>
        <AiPage refreshKey={0} onNavigate={() => {}} />
      </AppStateProvider>
    );
    await userEvent.click(await screen.findByRole("button", { name: "關閉" }));
    expect(
      await screen.findByText("工作階段已關閉（已要求終止子程序）。")
    ).toBeInTheDocument();
    expect(screen.queryByText(/子程序已終止/)).not.toBeInTheDocument();
  });

  it("可從工作階段卡片明確續租，且只在後端成功後宣稱完成", async () => {
    vi.spyOn(api, "agentsDiscoveries").mockResolvedValue({ agents: [] });
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([record]);
    const renewed = {
      ...record,
      lease: { ...record.lease, expiresAt: "2026-01-01T01:30:00Z" },
    };
    const renew = vi.spyOn(api, "agentSessionRenew").mockResolvedValue(renewed);
    render(
      <AppStateProvider ready={false} refreshKey={0}>
        <AiPage refreshKey={0} onNavigate={() => {}} />
      </AppStateProvider>
    );
    await userEvent.click(await screen.findByRole("button", { name: "續租 30 分鐘" }));
    expect(renew).toHaveBeenCalledWith("s-1", 30);
    expect(await screen.findByText(/已續租至/)).toBeInTheDocument();
  });
});

describe("版本與資料管理的新家（v0.5 IA）", () => {
  it("Runtime 真實版本搬到「更多 → 進階模式」的第二層（只有進階模式看得到）", async () => {
    vi.spyOn(api, "status").mockResolvedValue({ version: "0.4.0", schemaVersion: "0.4" });
    vi.spyOn(api, "uiPrefsGet").mockResolvedValue({
      mode: "advanced",
      locale: "zh-TW",
      customNames: {},
      schemaVersion: "1.0",
    });
    vi.spyOn(api, "pauseGet").mockResolvedValue({ paused: false });
    render(
      <AppStateProvider ready={true} refreshKey={0}>
        <MorePage
          refreshKey={0}
          events={[]}
          advanced
          onNavigate={() => {}}
          onRerunOnboarding={() => {}}
          initial="advanced-features"
        />
      </AppStateProvider>
    );
    expect(await screen.findByText(/Runtime 0\.4\.0/)).toBeInTheDocument();
  });

  it("外觀與語言頁只指路到進階模式，不再自己放版本或匯出入口", async () => {
    const navigate = vi.fn();
    render(
      <AppStateProvider ready={false} refreshKey={0}>
        <SettingsPage onRerunOnboarding={() => {}} onNavigate={navigate} />
      </AppStateProvider>
    );
    await screen.findByText("語言、外觀與可及性");
    expect(screen.queryByText(/Runtime/)).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "開啟匯出、還原與刪除" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "前往進階模式" }));
    expect(navigate).toHaveBeenCalledWith("advanced-features");
  });
});

describe("AiPage 限權寫入工作階段", () => {
  it("必須有明確目錄與第二次確認，並傳送精確 scope", async () => {
    vi.spyOn(api, "agentsDiscoveries").mockResolvedValue({ agents: [] });
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([]);
    vi.spyOn(api, "agentsRouting").mockResolvedValue({ reason: "test" });
    const create = vi
      .spyOn(api, "agentSessionCreate")
      .mockResolvedValue({} as AgentSessionRecord);
    render(
      <AppStateProvider ready={false} refreshKey={0}>
        <AiPage refreshKey={0} onNavigate={() => {}} />
      </AppStateProvider>
    );
    await userEvent.click(await screen.findByRole("button", { name: "建立工作階段…" }));
    const dialog = within(screen.getByRole("dialog", { name: "建立 AI 工作階段" }));
    const submit = dialog.getByRole("button", { name: "同意並建立" });
    await userEvent.click(dialog.getByRole("checkbox", { name: /允許 Agent 修改/ }));
    expect(submit).toBeDisabled();
    await userEvent.type(dialog.getByPlaceholderText(/path\/to\/project/), "/tmp/repo");
    expect(submit).toBeDisabled();
    await userEvent.click(dialog.getByRole("checkbox", { name: /我已確認上方工作目錄/ }));
    expect(submit).toBeEnabled();
    await userEvent.click(submit);
    await waitFor(() =>
      expect(create).toHaveBeenCalledWith(
        expect.objectContaining({
          workdir: "/tmp/repo",
          allowWrite: true,
          dataScope: ["workspace:/tmp/repo"],
          toolScope: ["workspace.write"],
          consentScope: ["agent-session:workspace-write"],
        })
      )
    );
  });
});

describe("待我決定計數誠實（收件匣與 NowStrip）", () => {
  it("收件匣：Runtime 統一查詢失敗時誠實顯示錯誤", async () => {
    vi.spyOn(api, "activityInbox").mockRejectedValue(new Error("inbox down"));
    render(<InboxSection refreshKey={0} onNavigate={() => {}} />);
    expect(await screen.findByRole("alert")).toHaveTextContent("收件匣無法載入");
    expect(screen.queryByText("目前沒有符合篩選條件的活動。")).not.toBeInTheDocument();
  });

  it("收件匣：複合 Agent/裝置/Domain 篩選送到同一 application service", async () => {
    const inbox = vi.spyOn(api, "activityInbox").mockResolvedValue({
      items: [],
      count: 0,
      totalBeforeLimit: 0,
      pendingCount: 0,
    });
    render(<InboxSection refreshKey={0} onNavigate={() => {}} />);
    await screen.findByText("統一收件匣（待決定 0／共 0）");
    const user = userEvent.setup();
    await user.type(screen.getByLabelText("Agent"), "codex");
    await user.type(screen.getByLabelText("裝置"), "camera");
    await user.type(screen.getByLabelText("知識領域"), "rust");
    await waitFor(() =>
      expect(inbox).toHaveBeenLastCalledWith(
        expect.objectContaining({ agent: "codex", device: "camera", domain: "rust", limit: 200 })
      )
    );
  });

  it("NowStrip：查詢失敗顯示無法確認，不顯示綠色 0 項", async () => {
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([]);
    vi.spyOn(api, "activityInbox").mockRejectedValue(new Error("inbox down"));
    vi.spyOn(api, "knowledgeReceipts").mockResolvedValue({ receipts: [] });
    render(<NowStrip refreshKey={0} status={{}} onNavigate={() => {}} />);
    expect(await screen.findByText("無法確認（查詢失敗）")).toBeInTheDocument();
    expect(screen.queryByText("0 項")).not.toBeInTheDocument();
  });

  it("NowStrip：待我決定直接顯示後端 pendingCount（與右上角 Inbox 同一個數字）", async () => {
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([]);
    const inbox = vi.spyOn(api, "activityInbox").mockResolvedValue({
      items: [],
      count: 1,
      totalBeforeLimit: 42,
      pendingCount: 7,
    });
    vi.spyOn(api, "knowledgeReceipts").mockResolvedValue({ receipts: [] });
    render(<NowStrip refreshKey={0} status={{}} onNavigate={() => {}} />);
    expect(await screen.findByText("7 項")).toBeInTheDocument();
    expect(inbox).toHaveBeenCalled();
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

describe("相容 tab 的標題與導覽歸屬（v0.5 五入口）", () => {
  it("舊能力 tab 與 safety 歸屬連接與權限，標題不為空", () => {
    for (const legacy of ["senses", "responses", "toolops", "capabilities", "safety"]) {
      expect(navAnchorFor(legacy)).toBe("connect");
      expect(titleFor(legacy)).toBe("連接與權限");
    }
  });

  it("ai/automations 歸屬工作；memory/activity/settings 歸屬更多", () => {
    for (const legacy of ["ai", "automations"]) {
      expect(navAnchorFor(legacy)).toBe("work");
      expect(titleFor(legacy)).toBe("工作");
    }
    for (const legacy of ["memory", "activity", "settings"]) {
      expect(navAnchorFor(legacy)).toBe("more");
      expect(titleFor(legacy)).toBe("更多");
    }
  });

  it("一般與進階 tab 不受影響", () => {
    expect(navAnchorFor("home")).toBe("home");
    expect(titleFor("home")).toBe("現在");
    expect(titleFor("adv-recipes")).toBe("配方 YAML");
  });
});

describe("Source Viewer 精確區域與時碼", () => {
  it("解析圖像 region 與音視訊 time range，未知片段不猜測", () => {
    expect(parseSourceSegment("region=10,20,30,40")).toEqual({
      region: { x: 10, y: 20, width: 30, height: 40 },
    });
    expect(parseSourceSegment("t=12.5-30.2")).toEqual({ startSeconds: 12.5, endSeconds: 30.2 });
    expect(parseSourceSegment("region=full")).toEqual({});
    expect(parseSourceSegment("t=unknown")).toEqual({});
  });
});
