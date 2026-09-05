// ⌘K 指令面板的可及性契約（M3c 發現、v0.6.x 修）：
// 面板寫著「Esc 關閉」，但 Escape 以前只掛在搜尋框上——按一次 Tab 焦點落到第一個
// 選項（正好是「緊急停止」）之後 Escape 就關不掉，而且 overlay 沒有焦點陷阱，
// 再 Tab 幾下焦點會逃到面板後面的頁面上。這裡把契約釘住：
// 1. 焦點在任何選項上，Escape 一樣收得掉；
// 2. Tab 在面板內循環，不會逃出去；
// 3. 面板是 aria-modal 對話框；關掉之後焦點回到開啟前的元素。
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";
import React from "react";

import { api } from "../api";
import { AppStateProvider } from "../appstate";
import { GlobalSearch } from "../components/GlobalSearch";

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

function stubSearch() {
  vi.spyOn(api, "agentSessionsList").mockResolvedValue([]);
  vi.spyOn(api, "providersList").mockResolvedValue([]);
  vi.spyOn(api, "memoryList").mockResolvedValue({ items: [] });
  vi.spyOn(api, "knowledgeList").mockResolvedValue({ nodes: [], count: 0 });
  vi.spyOn(api, "domainPacks").mockResolvedValue({ packs: [] });
  vi.spyOn(api, "actionsList").mockResolvedValue([]);
  vi.spyOn(api, "knowledgeReceipts").mockResolvedValue({ receipts: [] });
  vi.spyOn(api, "uiPrefsGet").mockResolvedValue({
    mode: "simple",
    locale: "zh-TW",
    customNames: {},
    schemaVersion: "1.0",
  });
  vi.spyOn(api, "pauseGet").mockResolvedValue({ paused: false });
}

function Harness({ onClose }: { onClose: () => void }) {
  const [open, setOpen] = React.useState(false);
  return (
    <AppStateProvider ready={false} refreshKey={0}>
      <button type="button" onClick={() => setOpen(true)}>
        開啟搜尋
      </button>
      <GlobalSearch
        open={open}
        onClose={() => {
          setOpen(false);
          onClose();
        }}
        onNavigate={() => {}}
        estopped={false}
        onEstop={async () => {}}
        onCommandFeedback={() => {}}
      />
    </AppStateProvider>
  );
}

describe("⌘K 指令面板：Escape 與焦點陷阱", () => {
  it("焦點在選項上（不在搜尋框）時，Escape 一樣收得掉", async () => {
    stubSearch();
    const onClose = vi.fn();
    render(<Harness onClose={onClose} />);
    fireEvent.click(screen.getByRole("button", { name: "開啟搜尋" }));
    const dialog = await screen.findByRole("dialog", { name: "全域搜尋" });
    const options = await screen.findAllByRole("option");
    expect(options.length).toBeGreaterThan(0);
    act(() => options[0].focus());
    expect(document.activeElement).toBe(options[0]);
    fireEvent.keyDown(options[0], { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("dialog", { name: "全域搜尋" })).toBeNull();
    expect(dialog).not.toBeInTheDocument();
  });

  it("Tab 在面板內循環：最後一個選項再 Tab 回到搜尋框，Shift+Tab 從搜尋框回到最後一個", async () => {
    stubSearch();
    render(<Harness onClose={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "開啟搜尋" }));
    await screen.findByRole("dialog", { name: "全域搜尋" });
    const input = screen.getByPlaceholderText(/搜尋設定、能力、記憶、知識/);
    const options = await screen.findAllByRole("option");
    const last = options[options.length - 1];
    act(() => last.focus());
    fireEvent.keyDown(last, { key: "Tab" });
    expect(document.activeElement).toBe(input);
    fireEvent.keyDown(input, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(last);
  });

  it("面板是 aria-modal 對話框；關掉之後焦點回到開啟它的按鈕", async () => {
    stubSearch();
    render(<Harness onClose={() => {}} />);
    const opener = screen.getByRole("button", { name: "開啟搜尋" });
    act(() => opener.focus());
    fireEvent.click(opener);
    const dialog = await screen.findByRole("dialog", { name: "全域搜尋" });
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    // 一開就有人接 Escape：容器本身可聚焦，不必等 30 ms 後搜尋框拿到焦點。
    expect(dialog.contains(document.activeElement)).toBe(true);
    fireEvent.keyDown(document.activeElement ?? dialog, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "全域搜尋" })).toBeNull();
    expect(document.activeElement).toBe(opener);
  });

  it("IME 組字中的 Escape 不關面板（那是取消選字）", async () => {
    stubSearch();
    const onClose = vi.fn();
    render(<Harness onClose={onClose} />);
    fireEvent.click(screen.getByRole("button", { name: "開啟搜尋" }));
    await screen.findByRole("dialog", { name: "全域搜尋" });
    const input = screen.getByPlaceholderText(/搜尋設定、能力、記憶、知識/);
    act(() => input.focus());
    fireEvent.keyDown(input, { key: "Escape", isComposing: true });
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.keyDown(input, { key: "Escape", keyCode: 229 });
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.keyDown(input, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
