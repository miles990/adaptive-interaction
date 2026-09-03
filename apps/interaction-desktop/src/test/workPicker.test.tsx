// 桌面版的原生資料夾選擇器（工作頁）。
//
// `isTauri` 在 transport 模組載入時就固定下來，所以每個案例都先塞
// `window.__TAURI_INTERNALS__` 再 `vi.resetModules()` 重新載入整條相依鏈，
// 拿到的是「桌面分支」的 TaskComposer 與同一份重新建立的 invoke mock。
//
// 誠實重點：picked／cancelled／unsupported／error 四種結果分得清楚——
// 真的打不開（權限被擋、外掛沒註冊、作業系統失敗）必須顯示原因，
// 不得再退化成「這個版本沒有資料夾選擇器」。原生對話框本身無法在 jsdom
// 或 Playwright 裡驗收，這裡驗的是呼叫參數與四種結果的處理。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

vi.mock("../characterName", () => ({
  useCharacterName: () => ({
    name: "小樞",
    pronoun: "她",
    characterId: "shu-maid",
    loaded: true,
    icon: "cat",
  }),
  characterNameFallback: "角色",
}));

const DISCOVERIES = {
  agents: [
    { kind: "codex", found: true, loggedIn: true, detail: "codex 1.0" },
    { kind: "claude-code", found: true, loggedIn: true, detail: "claude 1.0" },
  ],
};

/** 以「桌面版」重新載入模組鏈，並回傳這一輪新建立的 invoke mock。 */
async function loadDesktop() {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  vi.resetModules();
  const core = await import("@tauri-apps/api/core");
  const invoke = vi.mocked(core.invoke);
  invoke.mockReset();
  const { api } = await import("../api");
  vi.spyOn(api, "agentsDiscoveries").mockResolvedValue(
    DISCOVERIES as unknown as Record<string, unknown>
  );
  const { AppStateProvider } = await import("../appstate");
  const composer = await import("../pages/work/TaskComposer");
  const renderComposer = () =>
    render(
      <AppStateProvider ready={false} refreshKey={0}>
        <composer.TaskComposer />
      </AppStateProvider>
    );
  return { invoke, composer, renderComposer };
}

beforeEach(() => {
  window.sessionStorage.clear();
});

afterEach(() => {
  vi.restoreAllMocks();
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
});

describe("桌面版原生資料夾選擇器", () => {
  it("選好資料夾：以 directory＝true 呼叫 host 對話框，路徑填回輸入框", async () => {
    const { invoke, renderComposer } = await loadDesktop();
    invoke.mockResolvedValue("/Users/me/proj");
    renderComposer();
    await userEvent.click(screen.getByRole("button", { name: "選擇資料夾…" }));
    expect(invoke).toHaveBeenCalledWith("plugin:dialog|open", {
      options: { directory: true, multiple: false, title: "選擇資料夾" },
    });
    expect(screen.getByLabelText("加入檔案或選擇資料夾")).toHaveValue("/Users/me/proj");
    // 桌面版不再顯示「瀏覽器版沒有原生資料夾選擇器」那句。
    expect(screen.queryByText(/瀏覽器版沒有原生資料夾選擇器/)).not.toBeInTheDocument();
  });

  it("使用者取消：輸入框不動，也不跳任何錯誤", async () => {
    const { invoke, renderComposer } = await loadDesktop();
    invoke.mockResolvedValue(null);
    renderComposer();
    const folder = screen.getByLabelText("加入檔案或選擇資料夾");
    await userEvent.type(folder, "/tmp/keep");
    await userEvent.click(screen.getByRole("button", { name: "選擇資料夾…" }));
    expect(folder).toHaveValue("/tmp/keep");
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("打不開就照實說原因，不冒充「沒有資料夾選擇器」", async () => {
    const { invoke, renderComposer } = await loadDesktop();
    invoke.mockRejectedValue(new Error("dialog.open not allowed"));
    renderComposer();
    await userEvent.click(screen.getByRole("button", { name: "選擇資料夾…" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("dialog.open not allowed");
    expect(alert.textContent).toContain("可以直接貼上路徑");
    expect(alert.textContent).not.toContain("沒有資料夾選擇器");
    expect(alert.textContent).not.toContain("這個版本沒有");
  });

  it("換資料夾會作廢先前的寫入確認（授權對象變了就要重新同意）", async () => {
    const { invoke, renderComposer } = await loadDesktop();
    invoke.mockResolvedValue("/Users/me/proj");
    renderComposer();
    await userEvent.type(screen.getByLabelText("想讓小樞幫你做什麼？"), "修掉失敗的測試");
    await userEvent.type(screen.getByLabelText("加入檔案或選擇資料夾"), "/tmp/old");
    await userEvent.click(screen.getByRole("checkbox", { name: /允許修改這個資料夾裡的檔案/ }));
    await userEvent.click(
      screen.getByRole("checkbox", { name: /我已確認：這次工作只可以在 \/tmp\/old 裡修改檔案/ })
    );
    expect(screen.getByRole("button", { name: "開始" })).toBeEnabled();

    await userEvent.click(screen.getByRole("button", { name: "選擇資料夾…" }));
    const reconfirm = screen.getByRole("checkbox", {
      name: /我已確認：這次工作只可以在 \/Users\/me\/proj 裡修改檔案/,
    });
    expect(reconfirm).not.toBeChecked();
    expect(screen.getByRole("button", { name: "開始" })).toBeDisabled();
  });

  it("pickDirectory 的四種結果：字串／陣列＝picked，null／空陣列＝cancelled，丟出＝error", async () => {
    const { invoke, composer } = await loadDesktop();
    invoke.mockResolvedValueOnce("/a/b");
    await expect(composer.pickDirectory()).resolves.toEqual({ kind: "picked", path: "/a/b" });
    invoke.mockResolvedValueOnce(["/a/c"]);
    await expect(composer.pickDirectory()).resolves.toEqual({ kind: "picked", path: "/a/c" });
    invoke.mockResolvedValueOnce(null);
    await expect(composer.pickDirectory()).resolves.toEqual({ kind: "cancelled" });
    invoke.mockResolvedValueOnce([]);
    await expect(composer.pickDirectory()).resolves.toEqual({ kind: "cancelled" });
    invoke.mockRejectedValueOnce(new Error("boom"));
    await expect(composer.pickDirectory()).resolves.toEqual({
      kind: "error",
      message: "Error: boom",
    });
  });
});
