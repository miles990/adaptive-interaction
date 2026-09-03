// 工作頁 task-first（v0.5 Phase 3 I）：交代流程與預填、開始前預覽只回答三件事
// （這次會讀取什麼／會不會修改內容／最多使用多少時間與費用）＋其餘收進「查看技術細節」、
// 「開始」走既有 agentSessionCreate 路徑（payload 精確）、寫入的第二次確認要印出完整路徑
// 且換路徑就作廢、瀏覽器版誠實說沒有原生資料夾選擇器、
// claimed／verified／unknown 的誠實呈現、一般模式不外洩治理術語（畫面＋原始碼）、
// 自動互動分頁仍可達、Agent 管理收進折疊的工作設定、進階模式零退化。

import fs from "node:fs";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// 角色名稱走共用 hook；這裡固定成「小樞」，讓斷言不依賴 hook 的載入行為。
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

import { invoke } from "@tauri-apps/api/core";

import { api, AgentSessionRecord } from "../api";
import { AppStateProvider } from "../appstate";
import { WorkPage } from "../pages/WorkPage";
import {
  agentAvailability,
  agentForKind,
  basename,
  buildSessionCreateInput,
  CANCEL_SENTENCE,
  DEFAULT_TTL_MINUTES,
  parseWorkPrefill,
  pickDirectory,
  previewAnswers,
  readWorkPrefill,
  taskLabelFrom,
  WORK_PREFILL_KEY,
  peekWorkPrefill,
  clearWorkPrefill,
} from "../pages/work/TaskComposer";

const DISCOVERIES = {
  agents: [
    { kind: "codex", found: true, loggedIn: true, detail: "codex 1.0" },
    { kind: "claude-code", found: true, loggedIn: true, detail: "claude 1.0" },
  ],
};

function session(overrides: Partial<AgentSessionRecord> = {}): AgentSessionRecord {
  return {
    sessionId: "sess-9d3f1c00-1111-2222-3333-444455556666",
    providerId: "p-1",
    agentId: "codex",
    label: "整理測試報告",
    state: "active",
    lease: { issuedAt: "2026-01-01T00:00:00Z", expiresAt: "2026-01-01T01:00:00Z", renewable: true },
    dataScope: ["workspace:/tmp/repo"],
    toolScope: [],
    consentScope: [],
    budget: { maxMessages: 10, spentMessages: 1, maxCost: 0.5, spentCost: 0.01 },
    createdAt: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

function stubApis(sessions: AgentSessionRecord[] = [], discoveries: unknown = DISCOVERIES) {
  vi.spyOn(api, "agentsDiscoveries").mockResolvedValue(discoveries as Record<string, unknown>);
  vi.spyOn(api, "agentSessionsList").mockResolvedValue(sessions);
  vi.spyOn(api, "agentSessionMessages").mockResolvedValue([]);
  vi.spyOn(api, "recipesList").mockResolvedValue([]);
}

function renderWork(props: { advanced?: boolean; initial?: "sessions" | "automations" } = {}) {
  return render(
    <AppStateProvider ready={false} refreshKey={0}>
      <WorkPage
        refreshKey={0}
        advanced={props.advanced ?? false}
        onNavigate={() => {}}
        initial={props.initial}
      />
    </AppStateProvider>
  );
}

const BANNED_SIMPLE_MODE_TERMS = [
  "Agent Session",
  "Provider Registry",
  "Receptor",
  "Actuator",
  "Lease",
  "UUID",
  "Receipt",
  "app-server",
  "YAML",
  "JSON",
  "provider session",
  "工作階段",
  "建立 Session",
  "Session",
  "Patch",
];
const UUID_RE = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i;

beforeEach(() => {
  window.sessionStorage.clear();
});

afterEach(() => {
  vi.restoreAllMocks();
});

// ---------------------------------------------------------------------------

describe("交代一件工作（task-first 第一屏）", () => {
  it("以角色名稱提問、可加入資料夾、有「開始」；空白時不能開始", async () => {
    stubApis();
    renderWork();
    expect(screen.getByLabelText("想讓小樞幫你做什麼？")).toBeInTheDocument();
    expect(screen.getByLabelText("加入檔案或選擇資料夾")).toBeInTheDocument();
    expect(screen.getByLabelText("這是哪一種工作")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "開始" })).toBeDisabled();
    // 分頁名稱是人話：「工作」與「自動互動」。
    expect(screen.getAllByRole("tab").map((t) => t.textContent)).toEqual(["工作", "自動互動"]);
  });

  it("從 sessionStorage 的 work.prefill 預填（結構化或純文字），讀完即清除", async () => {
    window.sessionStorage.setItem(
      WORK_PREFILL_KEY,
      JSON.stringify({ task: "看一下測試有沒有壞掉", workdir: "/tmp/repo", kind: "programming" })
    );
    stubApis();
    renderWork();
    expect(screen.getByLabelText("想讓小樞幫你做什麼？")).toHaveValue("看一下測試有沒有壞掉");
    expect(screen.getByLabelText("加入檔案或選擇資料夾")).toHaveValue("/tmp/repo");
    expect(screen.getByLabelText("這是哪一種工作")).toHaveValue("programming");
    expect(window.sessionStorage.getItem(WORK_PREFILL_KEY)).toBeNull();

    // 純文字（首次成功體驗只丟一句話）也接受。
    expect(parseWorkPrefill("提醒我休息")).toEqual({ task: "提醒我休息", workdir: "" });
    expect(parseWorkPrefill(JSON.stringify({ workdir: "/x" }))).toEqual({
      task: "",
      workdir: "/x",
      kind: undefined,
    });
    expect(parseWorkPrefill("")).toBeNull();
    expect(parseWorkPrefill(null)).toBeNull();
    expect(readWorkPrefill(null)).toBeNull();
    const removed: string[] = [];
    const fake = {
      getItem: () => "整理資料夾",
      removeItem: (key: string) => {
        removed.push(key);
      },
    };
    expect(readWorkPrefill(fake)).toEqual({ task: "整理資料夾", workdir: "" });
    expect(removed).toEqual([WORK_PREFILL_KEY]);
  });

  it("開始前預覽只回答三件事；其餘（Agent／工具／沙箱／上限／取消／原始授權範圍）收進「查看技術細節」", async () => {
    stubApis();
    renderWork();
    await userEvent.type(screen.getByLabelText("加入檔案或選擇資料夾"), "/tmp/repo");
    await userEvent.selectOptions(screen.getByLabelText("這是哪一種工作"), "programming");
    const preview = screen.getByRole("group", { name: "開始前預覽" });
    const answers = preview.querySelector<HTMLElement>(".work-preview-answers")!;
    expect(within(answers).getAllByRole("term").map((t) => t.textContent)).toEqual([
      "這次會讀取什麼",
      "會不會修改內容",
      "最多使用多少時間與費用",
    ]);
    const answerText = answers.textContent ?? "";
    expect(answerText).toContain("你選擇的資料夾（repo）");
    // 誠實：後端只擋寫入，不擋讀取——不得宣稱「只讀取這個資料夾」。
    expect(answerText).not.toContain("只讀取這個資料夾");
    expect(answerText).toContain("不會修改");
    expect(answerText).toContain(`${DEFAULT_TTL_MINUTES} 分鐘`);
    expect(answerText).toContain("依 Codex 登入方案計費");

    // 技術細節預設收合：Agent／工具／如何取消都看不到。
    const details = preview.querySelector<HTMLElement>("details.tech-details")!;
    const summary = within(details).getByText("查看技術細節");
    expect(within(details).getByText("使用哪個 Agent")).not.toBeVisible();
    expect(within(details).getByText("工具")).not.toBeVisible();
    expect(within(details).getByText("如何取消")).not.toBeVisible();
    expect(within(details).getByText(CANCEL_SENTENCE)).not.toBeVisible();
    // 三個回答本身永遠看得見。
    expect(within(answers).getByText("這次會讀取什麼")).toBeVisible();

    await userEvent.click(summary);
    expect(details).toHaveAttribute("open");
    expect(within(details).getByText("使用哪個 Agent")).toBeVisible();
    expect(within(details).getByText(CANCEL_SENTENCE)).toBeVisible();
    expect(await within(details).findByText("可用")).toBeInTheDocument();
    const techText = details.textContent ?? "";
    expect(techText).toContain("Codex");
    expect(techText).toContain("擅長程式實作");
    expect(techText).toContain("依你的分工設定（程式工作）");
    expect(techText).toContain("唯讀沙箱");
    expect(techText).toContain("/tmp/repo");
    expect(techText).toContain("訊息上限");
    expect(techText).toContain("原始授權範圍");
    expect(techText).toContain("workspace:/tmp/repo");
    // 沒開寫入時，寫入相關的原始授權範圍不會出現在任何地方。
    expect(preview.textContent).not.toContain("agent-session:workspace-write");

    // 費用上限只對非 Codex 顯示金額。
    await userEvent.selectOptions(screen.getByLabelText("這是哪一種工作"), "conversation");
    expect(answers.textContent).toContain(`${DEFAULT_TTL_MINUTES} 分鐘，最多 US$0.50`);
    expect(details.textContent).toContain("Claude Code");
  });

  it("瀏覽器版（非桌面）沒有假的「選擇資料夾…」按鈕，改用一句誠實說明", async () => {
    stubApis();
    renderWork();
    expect(screen.queryByRole("button", { name: "選擇資料夾…" })).not.toBeInTheDocument();
    expect(screen.getByText("瀏覽器版沒有原生資料夾選擇器；請貼上資料夾路徑。")).toBeInTheDocument();
    // 這個環境沒有原生選擇器：誠實回 unsupported，而且不去呼叫任何 host 指令。
    vi.mocked(invoke).mockClear();
    await expect(pickDirectory()).resolves.toEqual({ kind: "unsupported" });
    expect(vi.mocked(invoke)).not.toHaveBeenCalled();
  });

  it("「開始」走既有 agentSessionCreate 路徑，再把內容送給 Agent，只宣稱已送達", async () => {
    stubApis();
    const create = vi
      .spyOn(api, "agentSessionCreate")
      .mockResolvedValue(session({ sessionId: "s-new", label: "看一下這個 repo 的測試" }));
    const send = vi
      .spyOn(api, "agentSessionSend")
      .mockResolvedValue({ messageId: "m-1", deliveredAt: "2026-01-01T00:00:01Z" });
    renderWork();
    const textarea = screen.getByLabelText("想讓小樞幫你做什麼？");
    await userEvent.type(textarea, "看一下這個 repo 的測試有沒有壞掉");
    await userEvent.type(screen.getByLabelText("加入檔案或選擇資料夾"), "/tmp/repo");
    await userEvent.selectOptions(screen.getByLabelText("這是哪一種工作"), "programming");
    await within(screen.getByRole("group", { name: "開始前預覽" })).findByText("可用");
    const start = screen.getByRole("button", { name: "開始" });
    expect(start).toBeEnabled();
    await userEvent.click(start);
    await waitFor(() =>
      expect(create).toHaveBeenCalledWith({
        agentId: "codex",
        label: "看一下這個 repo 的測試有沒有壞掉",
        ttlMinutes: DEFAULT_TTL_MINUTES,
        maxCost: null,
        workdir: "/tmp/repo",
        allowWrite: false,
        dataScope: ["workspace:/tmp/repo"],
        toolScope: [],
        consentScope: [],
      })
    );
    await waitFor(() =>
      expect(send).toHaveBeenCalledWith("s-new", "task", {
        task: "看一下這個 repo 的測試有沒有壞掉",
      })
    );
    const notice = await screen.findByText(/已送到 Codex 手上/);
    expect(notice.textContent).toContain("尚未完成");
    expect(notice.textContent).not.toMatch(/^.*已完成。/);
    expect(textarea).toHaveValue("");
  });

  it("寫入要有資料夾＋第二次確認（確認文字印出完整路徑）；換路徑就作廢，payload 才帶精確的 scope", async () => {
    stubApis();
    const create = vi.spyOn(api, "agentSessionCreate").mockResolvedValue(session({ sessionId: "s-w" }));
    vi.spyOn(api, "agentSessionSend").mockResolvedValue({});
    renderWork();
    await userEvent.type(screen.getByLabelText("想讓小樞幫你做什麼？"), "修掉失敗的測試");
    await userEvent.selectOptions(screen.getByLabelText("這是哪一種工作"), "programming");
    const preview = screen.getByRole("group", { name: "開始前預覽" });
    await within(preview).findByText("可用");
    const start = screen.getByRole("button", { name: "開始" });
    await userEvent.click(screen.getByRole("checkbox", { name: /允許修改這個資料夾裡的檔案/ }));
    expect(start).toBeDisabled();
    expect(screen.getByText("要允許修改，必須先指定資料夾。")).toBeInTheDocument();
    const folder = screen.getByLabelText("加入檔案或選擇資料夾");
    await userEvent.type(folder, "/tmp/repo");
    expect(start).toBeDisabled();
    expect(screen.getByText(/還需要你確認一次/)).toBeInTheDocument();
    // 確認文字必須印出這一次真正會被寫入的完整路徑，不能只說「上面的資料夾」。
    const confirmOld = screen.getByRole("checkbox", {
      name: /我已確認：這次工作只可以在 \/tmp\/repo 裡修改檔案/,
    });
    expect(confirmOld.closest("label")!.textContent).toContain(
      `${DEFAULT_TTL_MINUTES} 分鐘到期、關閉或緊急停止時立即失效`
    );
    await userEvent.click(confirmOld);
    expect(start).toBeEnabled();
    expect(preview.textContent).toContain("會修改：只限 /tmp/repo（你已確認）");
    // 沒有「允許所有」之類的萬用開關。
    expect(screen.queryByText(/允許所有/)).not.toBeInTheDocument();

    // 改了路徑＝換了授權對象：確認自動作廢，「開始」重新變灰。
    await userEvent.type(folder, "-2");
    const confirmNew = screen.getByRole("checkbox", {
      name: /我已確認：這次工作只可以在 \/tmp\/repo-2 裡修改檔案/,
    });
    expect(confirmNew).not.toBeChecked();
    expect(start).toBeDisabled();
    await userEvent.click(confirmNew);
    expect(start).toBeEnabled();
    await userEvent.click(start);
    await waitFor(() =>
      expect(create).toHaveBeenCalledWith(
        expect.objectContaining({
          workdir: "/tmp/repo-2",
          allowWrite: true,
          dataScope: ["workspace:/tmp/repo-2"],
          toolScope: ["workspace.write"],
          consentScope: ["agent-session:workspace-write"],
        })
      )
    );
  });

  it("Agent 未安裝／未登入／設定為不交給 Agent 時不能開始，並說明原因", async () => {
    stubApis([], {
      agents: [
        { kind: "codex", found: true, loggedIn: false },
        { kind: "claude-code", found: false },
      ],
    });
    renderWork();
    await userEvent.type(screen.getByLabelText("想讓小樞幫你做什麼？"), "幫我整理");
    expect(await screen.findByText(/Claude Code未安裝/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "開始" })).toBeDisabled();
    await userEvent.selectOptions(screen.getByLabelText("這是哪一種工作"), "programming");
    expect(await screen.findByText(/Codex未登入/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "開始" })).toBeDisabled();

    expect(agentForKind({ conversation: "none" }, "conversation")).toBe("none");
    expect(agentForKind(undefined, "programming")).toBe("codex");
    expect(agentForKind({ programming: "bogus" }, "programming")).toBe("codex");
    expect(agentAvailability(undefined, true).blocking).toBe(true);
    expect(agentAvailability({ found: true, loggedIn: true }, false)).toMatchObject({
      label: "可用",
      blocking: false,
    });
    expect(agentAvailability({ found: true }, false)).toMatchObject({ label: "登入狀態未知", blocking: false });
  });

  it("開始失敗照實說，不清空使用者打的字", async () => {
    stubApis();
    vi.spyOn(api, "agentSessionCreate").mockRejectedValue(
      new Error("503: unavailable: agent 子程序已結束")
    );
    renderWork();
    const textarea = screen.getByLabelText("想讓小樞幫你做什麼？");
    await userEvent.type(textarea, "整理報告");
    await within(screen.getByRole("group", { name: "開始前預覽" })).findByText("可用");
    await userEvent.click(screen.getByRole("button", { name: "開始" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("沒有開始");
    expect(alert.textContent).not.toContain("已送達");
    expect(alert.textContent).not.toContain("子程序");
    expect(textarea).toHaveValue("整理報告");
  });

  // known limitation #24：送出的結果一律照後端證據投影，不再固定印「已送達」。
  it("送出結果依後端證據分四種：已送達／尚未送達／Agent 不可用／傳送失敗", async () => {
    const cases = [
      {
        name: "尚未送達（沒有送達戳記）",
        send: () => vi.spyOn(api, "agentSessionSend").mockResolvedValue({ messageId: "m-1" }),
        expect: /已放進 Claude Code 的信箱/,
        forbid: "已送達",
        badge: "尚未送達（已放進信箱）",
        keepsText: false,
      },
      {
        name: "Agent 不可用",
        send: () =>
          vi
            .spyOn(api, "agentSessionSend")
            .mockRejectedValue(new Error("503: unavailable: agent 子程序已結束")),
        expect: /Claude Code 現在不能接工作/,
        forbid: "已送達",
        badge: "Agent 不可用",
        keepsText: true,
      },
      {
        name: "傳送失敗",
        send: () =>
          vi
            .spyOn(api, "agentSessionSend")
            .mockRejectedValue(new Error("404: not found: agent session s-x")),
        expect: /沒能送出去/,
        forbid: "已送達",
        badge: "傳送失敗",
        keepsText: true,
      },
      {
        name: "結果不確定",
        send: () =>
          vi.spyOn(api, "agentSessionSend").mockRejectedValue(new TypeError("Failed to fetch")),
        expect: /不確定「整理報告」有沒有送到/,
        forbid: "已送達",
        badge: "結果不確定",
        keepsText: true,
      },
    ];
    for (const c of cases) {
      stubApis();
      vi.spyOn(api, "agentSessionCreate").mockResolvedValue(session({ sessionId: "s-x" }));
      c.send();
      const view = renderWork();
      const textarea = screen.getByLabelText("想讓小樞幫你做什麼？");
      await userEvent.type(textarea, "整理報告");
      await within(screen.getByRole("group", { name: "開始前預覽" })).findByText("可用");
      await userEvent.click(screen.getByRole("button", { name: "開始" }));
      const notice = await screen.findByText(c.expect);
      expect(notice.textContent, c.name).not.toContain(c.forbid);
      // 六態標籤看得見，而且不是綠色成功樣式。
      const badge = within(notice).getByText(c.badge);
      expect(badge.className, c.name).not.toContain("badge-ok");
      expect(textarea, c.name).toHaveValue(c.keepsText ? "整理報告" : "");
      view.unmount();
      vi.restoreAllMocks();
    }
  });
});

// ---------------------------------------------------------------------------

describe("進行中與最近的工作（誠實狀態階梯）", () => {
  it("claimed→驗證按鈕＋說明；verified→綠勾＋已由你確認；unknown／未知→結果不確定", async () => {
    stubApis([
      session({ sessionId: "s-a", label: "A 工作", state: "claimed-completed" }),
      session({
        sessionId: "s-b",
        label: "B 工作",
        state: "claimed-completed",
        humanVerified: { at: "2026-01-01T00:30:00Z", note: "看過了" },
      }),
      session({ sessionId: "s-c", label: "C 工作", state: "unknown" }),
      session({ sessionId: "s-d", label: "D 工作", state: "totally-bogus-state" }),
    ]);
    const verify = vi.spyOn(api, "agentSessionVerify").mockResolvedValue(session());
    const { container } = renderWork();
    const cardA = (await screen.findByText("A 工作")).closest<HTMLElement>(".provider-card")!;
    expect(within(cardA).getByText("對方說已完成")).toBeInTheDocument();
    expect(within(cardA).getByText(/尚未經過檢查/)).toBeInTheDocument();
    expect(within(cardA).getByText(/小樞才會顯示綠色勾勾/)).toBeInTheDocument();
    expect(within(cardA).queryByText(/✓/)).not.toBeInTheDocument();

    const cardB = screen.getByText("B 工作").closest<HTMLElement>(".provider-card")!;
    expect(within(cardB).getByText("✓ 已由你確認")).toBeInTheDocument();
    expect(within(cardB).getByText(/由你親自確認/)).toBeInTheDocument();
    expect(within(cardB).getByText(/看過了/)).toBeInTheDocument();
    expect(
      within(cardB).queryByRole("button", { name: "標記為已驗證（我確認過結果）" })
    ).not.toBeInTheDocument();
    expect(within(cardB).queryByText("對方說已完成")).not.toBeInTheDocument();

    const cardC = screen.getByText("C 工作").closest<HTMLElement>(".provider-card")!;
    expect(within(cardC).getByText("結果不確定")).toBeInTheDocument();
    const cardD = screen.getByText("D 工作").closest<HTMLElement>(".provider-card")!;
    expect(within(cardD).getByText("結果不確定")).toBeInTheDocument();
    expect(container.textContent).not.toContain("totally-bogus-state");

    await userEvent.click(
      within(cardA).getByRole("button", { name: "標記為已驗證（我確認過結果）" })
    );
    expect(verify).toHaveBeenCalledWith("s-a");
    expect(await screen.findByText("已標記為已驗證（由你人工確認）。")).toBeInTheDocument();
  });

  it("可修改資料夾的工作：延長有效期要再確認一次（延長＝修改權限也跟著延長）", async () => {
    stubApis([
      session({ sessionId: "s-w", label: "會改檔案的工作", state: "active", allowWrite: true }),
    ]);
    const renew = vi.spyOn(api, "agentSessionRenew").mockResolvedValue(session());
    renderWork();
    const card = (await screen.findByText("會改檔案的工作")).closest<HTMLElement>(".provider-card")!;
    await userEvent.click(within(card).getByRole("button", { name: "續租 30 分鐘" }));
    expect(renew).not.toHaveBeenCalled();
    expect(
      within(card).getByText("延長 30 分鐘會連同「可修改 /tmp/repo 裡的檔案」一起延長。")
    ).toBeInTheDocument();
    // 可以反悔，且反悔不會延長。
    await userEvent.click(within(card).getByRole("button", { name: "不延長" }));
    expect(renew).not.toHaveBeenCalled();
    expect(within(card).getByRole("button", { name: "續租 30 分鐘" })).toBeInTheDocument();
    // 第二次確認才真的送出。
    await userEvent.click(within(card).getByRole("button", { name: "續租 30 分鐘" }));
    await userEvent.click(within(card).getByRole("button", { name: "確認延長（含修改權限）" }));
    await waitFor(() => expect(renew).toHaveBeenCalledWith("s-w", 30));
  });

  it("只讀取的工作：延長有效期不多問一次（沒有寫入權限可延長）", async () => {
    stubApis([session({ sessionId: "s-r", label: "只讀取的工作", state: "active" })]);
    const renew = vi.spyOn(api, "agentSessionRenew").mockResolvedValue(session());
    renderWork();
    const card = (await screen.findByText("只讀取的工作")).closest<HTMLElement>(".provider-card")!;
    await userEvent.click(within(card).getByRole("button", { name: "續租 30 分鐘" }));
    await waitFor(() => expect(renew).toHaveBeenCalledWith("s-r", 30));
    expect(within(card).queryByRole("button", { name: "不延長" })).not.toBeInTheDocument();
  });
});

// ---------------------------------------------------------------------------

describe("一般模式不外洩治理術語", () => {
  it("畫面（含展開的訊息與折疊的工作設定）沒有 Session／Lease／Receipt／UUID／JSON 等字樣", async () => {
    stubApis([
      session({
        providerSessionId: "9d3f1c00-aaaa-bbbb-cccc-ddddeeeeffff",
        toolScope: ["workspace.write"],
        allowWrite: true,
      }),
    ]);
    vi.spyOn(api, "agentSessionMessages").mockResolvedValue([
      {
        messageId: "m-1",
        kind: "approval-request",
        createdAt: new Date().toISOString(),
        body: { requestId: "r-1", summary: "要寫入 src/main.rs" },
      },
    ]);
    const { container } = renderWork();
    await screen.findByText("整理測試報告");
    await userEvent.click(screen.getByRole("button", { name: "查看結果／訊息" }));
    await screen.findByText("等待你核可");
    await screen.findByText("要寫入 src/main.rs");
    const text = container.textContent ?? "";
    for (const term of BANNED_SIMPLE_MODE_TERMS) {
      expect(text, `一般模式畫面不得出現「${term}」`).not.toContain(term);
    }
    expect(text).not.toMatch(UUID_RE);
    expect(text).not.toContain("9d3f1c00");
    expect(text).not.toContain("技術詳情");
    expect(text).not.toContain("requestId");
    expect(text).toContain("有效至");
    expect(text).toContain("沿用既有對話脈絡");
    expect(text).toContain("可修改資料夾裡的檔案");
  });

  it("WorkPage／TaskComposer 原始碼的字串與 JSX 文字不含治理術語（source scan）", () => {
    const files = [
      "src/pages/WorkPage.tsx",
      "src/pages/work/TaskComposer.tsx",
      "src/work/delivery.ts",
    ];
    const banned = /Agent Session|Provider Registry|Receptor|Actuator|Lease|UUID|Receipt|app-server|YAML|JSON|工作階段|建立 Session|Patch/;
    for (const file of files) {
      const source = fs
        .readFileSync(path.resolve(file), "utf8")
        .replace(/\/\*[\s\S]*?\*\//g, "")
        .replace(/^\s*\/\/.*$/gm, "");
      const literals = [
        ...source.matchAll(/"(?:[^"\\\n]|\\.)*"|'(?:[^'\\\n]|\\.)*'|`[^`]*`/g),
        ...source.matchAll(/>([^<>{}]+)</g),
      ].map((m) => m[0]);
      expect(literals.length).toBeGreaterThan(20);
      for (const literal of literals) {
        expect(banned.test(literal), `${file}: ${literal}`).toBe(false);
      }
    }
  });
});

// ---------------------------------------------------------------------------

describe("分頁與工作設定", () => {
  it("自動互動分頁仍可達（頁內分頁與相容路由 initial）", async () => {
    stubApis();
    const first = renderWork();
    await userEvent.click(screen.getByRole("tab", { name: "自動互動" }));
    expect(screen.getByRole("tab", { name: "自動互動" })).toHaveAttribute("aria-selected", "true");
    expect(await screen.findByText(/自動互動是「當…就…」的規則/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "開始" })).not.toBeInTheDocument();
    first.unmount();

    renderWork({ initial: "automations" });
    expect(screen.getByRole("tab", { name: "自動互動" })).toHaveAttribute("aria-selected", "true");
    expect(await screen.findByText(/自動互動是「當…就…」的規則/)).toBeInTheDocument();
  });

  it("一般模式：Agent 管理收進折疊的「工作設定」，預設收合；「調整分工」展開", async () => {
    stubApis();
    const { container } = renderWork();
    const details = container.querySelector("details.work-settings")!;
    expect(details).toBeTruthy();
    expect(details).not.toHaveAttribute("open");
    expect(details.querySelector("summary")?.textContent).toContain("本機 AI Agent");
    expect(screen.queryByRole("button", { name: "建立工作階段…" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "調整分工" }));
    expect(details).toHaveAttribute("open");
    // Agent 卡片（偵測結果）真的住在折疊區裡：用 detail 字串找，避免和分工下拉的選項撞名。
    expect(await within(details as HTMLElement).findByText("codex 1.0")).toBeInTheDocument();
    expect(within(details as HTMLElement).getAllByText("Codex").length).toBeGreaterThanOrEqual(1);
    expect(within(details as HTMLElement).getAllByRole("combobox").length).toBeGreaterThanOrEqual(4);
    expect(within(details as HTMLElement).getByRole("button", { name: "重新偵測" })).toBeInTheDocument();
  });

  it("進階模式：完整建立面板與技術資訊仍在（零能力退化），交代流程也在", async () => {
    stubApis([session({ providerSessionId: "9d3f1c00-aaaa-bbbb-cccc-ddddeeeeffff" })]);
    const { container } = renderWork({ advanced: true });
    expect(await screen.findByRole("button", { name: "建立工作階段…" })).toBeInTheDocument();
    await screen.findByText("整理測試報告");
    const text = container.textContent ?? "";
    expect(text).toContain("狀態碼 active");
    expect(text).toContain("provider session");
    expect(text).toContain("訊息 1/10");
    expect(container.querySelector("details.work-settings")).toBeNull();
    expect(screen.getByRole("button", { name: "開始" })).toBeInTheDocument();
  });
});

// ---------------------------------------------------------------------------

describe("純函式", () => {
  it("taskLabelFrom：取第一個非空行、收攏空白、限長", () => {
    expect(taskLabelFrom("\n\n  幫我  看一下   測試 \n第二行")).toBe("幫我 看一下 測試");
    const long = "一".repeat(60);
    expect(taskLabelFrom(long)).toBe(`${"一".repeat(40)}…`);
    expect(taskLabelFrom("   ")).toBe("");
  });

  it("buildSessionCreateInput：Codex 不送費用上限；0 費用＝不設上限；空資料夾＝空 scope", () => {
    expect(
      buildSessionCreateInput({
        agent: "codex",
        label: "x",
        workdir: "  /tmp/repo ",
        ttlMinutes: 15,
        maxCost: 0.5,
        allowWrite: false,
      })
    ).toEqual({
      agentId: "codex",
      label: "x",
      ttlMinutes: 15,
      maxCost: null,
      workdir: "/tmp/repo",
      allowWrite: false,
      dataScope: ["workspace:/tmp/repo"],
      toolScope: [],
      consentScope: [],
    });
    expect(
      buildSessionCreateInput({
        agent: "claude-code",
        label: "",
        workdir: "",
        ttlMinutes: 30,
        maxCost: 0,
        allowWrite: false,
      })
    ).toMatchObject({ label: null, maxCost: null, workdir: null, dataScope: [] });
  });

  it("basename：吃 / 與 \\，尾端斜線不影響，沒有分隔就回原字串", () => {
    expect(basename("/tmp/repo")).toBe("repo");
    expect(basename("/tmp/repo/")).toBe("repo");
    expect(basename("C:\\Users\\me\\proj")).toBe("proj");
    expect(basename("  /a/b/c  ")).toBe("c");
    expect(basename("repo")).toBe("repo");
    expect(basename("")).toBe("");
  });

  it("previewAnswers：三個回答的用字（讀取不宣稱硬邊界、寫入印路徑、Codex 沒有費用上限）", () => {
    const base = {
      agent: "codex" as const,
      workdir: "",
      allowWrite: false,
      writeConfirmed: false,
      ttlMinutes: 30,
      maxCost: 0.5,
    };
    // 沒選資料夾。
    expect(previewAnswers(base)).toEqual({
      reads: "沒有選資料夾：從系統資料夾開始工作，不會用到你自己的檔案。",
      writes: "不會修改：這次只看不改，任何檔案都不會被動到。",
      limits: "30 分鐘；費用依 Codex 登入方案計費，這裡無法設上限",
    });
    // 選了資料夾、不寫入：只說「從這個資料夾開始工作」，不宣稱只讀取。
    const readOnly = previewAnswers({ ...base, workdir: " /tmp/repo " });
    expect(readOnly.reads).toContain("你選擇的資料夾（repo）");
    expect(readOnly.reads).toContain("不保證它完全不看資料夾以外的內容");
    expect(readOnly.reads).not.toContain("只讀取這個資料夾");
    // 寫入未確認 vs 已確認：兩者都印出完整路徑。
    expect(previewAnswers({ ...base, workdir: "/tmp/repo", allowWrite: true }).writes).toBe(
      "會修改：只限 /tmp/repo——還需要你確認一次"
    );
    expect(
      previewAnswers({ ...base, workdir: "/tmp/repo", allowWrite: true, writeConfirmed: true })
        .writes
    ).toBe("會修改：只限 /tmp/repo（你已確認）");
    // 勾了寫入但還沒選資料夾：不得假裝已經有範圍。
    expect(previewAnswers({ ...base, allowWrite: true, writeConfirmed: true }).writes).toBe(
      "會修改：只限 （尚未選擇資料夾）——還需要你確認一次"
    );
    // 非 Codex 才有金額上限（與後端一致：Codex 會拒絕費用上限）。
    expect(previewAnswers({ ...base, agent: "claude-code" }).limits).toBe(
      "30 分鐘，最多 US$0.50"
    );
    // Windows 路徑的最後一段。
    expect(previewAnswers({ ...base, workdir: "C:\\Users\\me\\proj" }).reads).toContain(
      "你選擇的資料夾（proj）"
    );
  });
});


describe("預填與 StrictMode 雙重初始化相容", () => {
  it("peek 兩次都讀得到，clear 之後才消失", () => {
    const data = new Map<string, string>([["work.prefill", "整理下載資料夾"]]);
    const fake = {
      getItem: (k: string) => data.get(k) ?? null,
      removeItem: (k: string) => {
        data.delete(k);
      },
    };
    expect(peekWorkPrefill(fake)).toEqual({ task: "整理下載資料夾", workdir: "" });
    expect(peekWorkPrefill(fake)).toEqual({ task: "整理下載資料夾", workdir: "" });
    clearWorkPrefill(fake);
    expect(peekWorkPrefill(fake)).toBeNull();
    clearWorkPrefill(fake); // 冪等
  });
});
