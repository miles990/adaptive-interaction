// 相容路由（v0.5 五入口）：舊 tab id（ai／automations／capabilities／safety／memory／activity／settings／manage）
// 由 PageBody 導到同一個複合頁的不同分頁。因為 work↔automations 渲染的是同一個元件型別，
// React 會沿用已掛載的實例；這裡直接對「已掛載的 PageBody 收到新 route」斷言內容真的切換，
// 而不是只測 route→anchor 對照表。

import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

// 子頁面各自需要一堆 API；本測試只驗證「切換」本身，把重頁面換成可辨識的 stub。
vi.mock("../pages/AiPage", () => ({ AiPage: () => <div data-testid="stub-ai">STUB AiPage</div> }));
vi.mock("../pages/AutomationsPage", () => ({
  AutomationsPage: () => <div data-testid="stub-automations">STUB AutomationsPage</div>,
}));
vi.mock("../pages/CapabilitiesHub", () => ({
  CapabilitiesHub: () => <div data-testid="stub-capabilities">STUB CapabilitiesHub</div>,
}));
vi.mock("../pages/SafetyPage", () => ({ SafetyPage: () => <div data-testid="stub-safety">STUB SafetyPage</div> }));
vi.mock("../pages/MemoryKnowledgePage", () => ({
  MemoryKnowledgePage: () => <div data-testid="stub-memory">STUB MemoryKnowledgePage</div>,
}));
vi.mock("../pages/ActivityPage", () => ({
  ActivityPage: () => <div data-testid="stub-activity">STUB ActivityPage</div>,
}));
vi.mock("../pages/SettingsPage", () => ({
  SettingsPage: () => <div data-testid="stub-settings">STUB SettingsPage</div>,
}));
vi.mock("../pages/BackupSection", () => ({
  BackupSection: () => <div data-testid="stub-backup">STUB BackupSection</div>,
}));
vi.mock("../pages/HomePage", async (importOriginal) => {
  const mod = (await importOriginal()) as Record<string, unknown>;
  return { ...mod, HomePage: () => <div data-testid="stub-home">STUB HomePage</div>, PermissionMap: () => <div /> };
});

import { api } from "../api";
import { AppStateProvider } from "../appstate";
import { PageBody, type Tab } from "../App";

afterEach(() => {
  vi.restoreAllMocks();
});

function mountBody(tab: Tab) {
  vi.spyOn(api, "uiPrefsGet").mockResolvedValue({
    mode: "simple",
    locale: "zh-TW",
    customNames: {},
    schemaVersion: "1.0",
  });
  vi.spyOn(api, "pauseGet").mockResolvedValue({ paused: false });
  const body = (t: Tab) => (
    <AppStateProvider ready={false} refreshKey={0}>
      <PageBody
        tab={t}
        refreshKey={0}
        events={[]}
        advanced={false}
        onNavigate={() => {}}
        onRerunOnboarding={() => {}}
      />
    </AppStateProvider>
  );
  const utils = render(body(tab));
  return { ...utils, go: (t: Tab) => utils.rerender(body(t)) };
}

describe("相容路由：已掛載元件收到新 route 後真的切換內容", () => {
  it("work ↔ automations（同一個 WorkPage 實例）", () => {
    const { go } = mountBody("work");
    expect(screen.getByTestId("stub-ai")).toBeInTheDocument();
    expect(screen.queryByTestId("stub-automations")).not.toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "工作" })).toHaveAttribute("aria-selected", "true");

    go("automations");
    expect(screen.getByTestId("stub-automations")).toBeInTheDocument();
    expect(screen.queryByTestId("stub-ai")).not.toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "自動互動" })).toHaveAttribute("aria-selected", "true");

    // 再切回去（含舊 id「ai」）。
    go("ai");
    expect(screen.getByTestId("stub-ai")).toBeInTheDocument();
    expect(screen.queryByTestId("stub-automations")).not.toBeInTheDocument();
  });

  it("connect ↔ safety（同一個 ConnectPage 實例）", () => {
    const { go } = mountBody("connect");
    expect(screen.getByTestId("stub-capabilities")).toBeInTheDocument();
    expect(screen.queryByTestId("stub-safety")).not.toBeInTheDocument();

    go("safety");
    expect(screen.getByTestId("stub-safety")).toBeInTheDocument();
    expect(screen.queryByTestId("stub-capabilities")).not.toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "同意與安全" })).toHaveAttribute("aria-selected", "true");

    go("capabilities");
    expect(screen.getByTestId("stub-capabilities")).toBeInTheDocument();
    expect(screen.queryByTestId("stub-safety")).not.toBeInTheDocument();
  });

  it("memory ↔ activity ↔ settings ↔ backup（同一個 MorePage 實例）", () => {
    const { go } = mountBody("more");
    expect(screen.getByTestId("stub-memory")).toBeInTheDocument();

    go("activity");
    expect(screen.getByTestId("stub-activity")).toBeInTheDocument();
    expect(screen.queryByTestId("stub-memory")).not.toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "活動紀錄" })).toHaveAttribute("aria-selected", "true");

    go("settings");
    expect(screen.getByTestId("stub-settings")).toBeInTheDocument();
    expect(screen.queryByTestId("stub-activity")).not.toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "外觀與語言" })).toHaveAttribute("aria-selected", "true");

    go("backup");
    expect(screen.getByTestId("stub-backup")).toBeInTheDocument();
    expect(screen.queryByTestId("stub-settings")).not.toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "備份與還原" })).toHaveAttribute("aria-selected", "true");

    go("memory");
    expect(screen.getByTestId("stub-memory")).toBeInTheDocument();
    expect(screen.queryByTestId("stub-backup")).not.toBeInTheDocument();
  });

  it("manage 是隱藏的相容路由：內容到得了，但沒有分頁按鈕", () => {
    const { go } = mountBody("more");
    go("manage");
    // 五個分頁按鈕不含「角色與整合管理」。
    expect(
      screen
        .getAllByRole("tab")
        .map((t) => t.textContent)
    ).toEqual(["記憶與資料", "活動紀錄", "外觀與語言", "備份與還原", "進階模式"]);
    expect(screen.queryByRole("tab", { name: "角色與整合管理" })).not.toBeInTheDocument();
    // 舊書籤／深連結仍看得到內容。
    expect(screen.getByRole("button", { name: /管理角色/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /管理裝置與整合/ })).toBeInTheDocument();
  });

  it("使用者在頁內點分頁後，route 再次改變仍以 route 為準", () => {
    const { go } = mountBody("work");
    fireEvent.click(screen.getByRole("tab", { name: "自動互動" }));
    expect(screen.getByTestId("stub-automations")).toBeInTheDocument();
    go("work");
    // route 沒變（仍是 work）時不強制彈回：initial 未改變。
    expect(screen.getByTestId("stub-automations")).toBeInTheDocument();
    go("safety");
    go("ai");
    expect(screen.getByTestId("stub-ai")).toBeInTheDocument();
  });
});
