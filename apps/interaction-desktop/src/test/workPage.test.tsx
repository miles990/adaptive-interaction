// 工作頁 task-first（v0.5 Phase 3 I）：交代流程與預填、開始前預覽的六件事、
// 「開始」走既有 agentSessionCreate 路徑（payload 精確）、寫入的第二次確認、
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

import { api, AgentSessionRecord } from "../api";
import { AppStateProvider } from "../appstate";
import { WorkPage } from "../pages/WorkPage";
import {
  agentAvailability,
  agentForKind,
  buildSessionCreateInput,
  CANCEL_SENTENCE,
  DEFAULT_TTL_MINUTES,
  parseWorkPrefill,
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

  it("開始前預覽列出六件事；Agent 有用途說明、偵測狀態與分工依據", async () => {
    stubApis();
    renderWork();
    await userEvent.type(screen.getByLabelText("加入檔案或選擇資料夾"), "/tmp/repo");
    await userEvent.selectOptions(screen.getByLabelText("這是哪一種工作"), "programming");
    const preview = screen.getByRole("group", { name: "開始前預覽" });
    expect(within(preview).getAllByRole("term").map((t) => t.textContent)).toEqual([
      "使用哪個 Agent",
      "讀取範圍",
      "是否寫入",
      "工具",
      "時間、訊息與費用上限",
      "如何取消",
    ]);
    expect(await within(preview).findByText("可用")).toBeInTheDocument();
    const text = preview.textContent ?? "";
    expect(text).toContain("Codex");
    expect(text).toContain("擅長程式實作");
    expect(text).toContain("依你的分工設定（程式工作）");
    expect(text).toContain("資料夾 /tmp/repo");
    expect(text).toContain("不寫入：只讀取，不修改任何檔案");
    expect(text).toContain("只讀取檔案；不修改");
    expect(text).toContain(`時間最多 ${DEFAULT_TTL_MINUTES} 分鐘`);
    expect(text).toContain("依 Codex 的登入方案計費");
    expect(text).toContain(CANCEL_SENTENCE);
    // 費用上限只對非 Codex 顯示金額。
    await userEvent.selectOptions(screen.getByLabelText("這是哪一種工作"), "conversation");
    expect(preview.textContent).toContain("Claude Code");
    expect(preview.textContent).toContain("最多 $0.50");
  });

  it("「開始」走既有 agentSessionCreate 路徑，再把內容送給 Agent，只宣稱已送達", async () => {
    stubApis();
    const create = vi
      .spyOn(api, "agentSessionCreate")
      .mockResolvedValue(session({ sessionId: "s-new", label: "看一下這個 repo 的測試" }));
    const send = vi.spyOn(api, "agentSessionSend").mockResolvedValue({});
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
    const notice = await screen.findByText(/已交給 Codex/);
    expect(notice.textContent).toContain("尚未完成");
    expect(notice.textContent).not.toMatch(/已完成/);
    expect(textarea).toHaveValue("");
  });

  it("寫入要有資料夾＋第二次確認，payload 才帶精確的 scope", async () => {
    stubApis();
    const create = vi.spyOn(api, "agentSessionCreate").mockResolvedValue(session({ sessionId: "s-w" }));
    vi.spyOn(api, "agentSessionSend").mockResolvedValue({});
    renderWork();
    await userEvent.type(screen.getByLabelText("想讓小樞幫你做什麼？"), "修掉失敗的測試");
    await userEvent.selectOptions(screen.getByLabelText("這是哪一種工作"), "programming");
    await within(screen.getByRole("group", { name: "開始前預覽" })).findByText("可用");
    const start = screen.getByRole("button", { name: "開始" });
    await userEvent.click(screen.getByRole("checkbox", { name: /允許修改這個資料夾裡的檔案/ }));
    expect(start).toBeDisabled();
    expect(screen.getByText("要允許修改，必須先指定資料夾。")).toBeInTheDocument();
    await userEvent.type(screen.getByLabelText("加入檔案或選擇資料夾"), "/tmp/repo");
    expect(start).toBeDisabled();
    expect(screen.getByText(/還需要你再確認一次/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("checkbox", { name: /我已確認上面的資料夾/ }));
    expect(start).toBeEnabled();
    expect(screen.getByText(/可以修改上面資料夾裡的檔案（你已確認）/)).toBeInTheDocument();
    await userEvent.click(start);
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

  it("開始失敗與送出失敗都照實說，不清空使用者打的字", async () => {
    stubApis();
    vi.spyOn(api, "agentSessionCreate").mockResolvedValue(session({ sessionId: "s-x" }));
    vi.spyOn(api, "agentSessionSend").mockRejectedValue(new Error("mailbox closed"));
    renderWork();
    const textarea = screen.getByLabelText("想讓小樞幫你做什麼？");
    await userEvent.type(textarea, "整理報告");
    await within(screen.getByRole("group", { name: "開始前預覽" })).findByText("可用");
    await userEvent.click(screen.getByRole("button", { name: "開始" }));
    expect(await screen.findByText(/工作已建立，但內容沒能送出/)).toBeInTheDocument();
    expect(textarea).toHaveValue("整理報告");
  });
});

// ---------------------------------------------------------------------------

describe("進行中與最近的工作（誠實狀態階梯）", () => {
  it("claimed→驗證按鈕＋說明；verified→綠勾＋已確認完成；unknown／未知→結果不確定", async () => {
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
    expect(within(cardA).getByText("Agent 說已完成，等待檢查")).toBeInTheDocument();
    expect(within(cardA).getByText(/尚未經過檢查/)).toBeInTheDocument();
    expect(within(cardA).getByText(/小樞才會顯示綠色勾勾/)).toBeInTheDocument();
    expect(within(cardA).queryByText(/✓/)).not.toBeInTheDocument();

    const cardB = screen.getByText("B 工作").closest<HTMLElement>(".provider-card")!;
    expect(within(cardB).getByText("✓ 已確認完成")).toBeInTheDocument();
    expect(within(cardB).getByText(/由你親自確認/)).toBeInTheDocument();
    expect(within(cardB).getByText(/看過了/)).toBeInTheDocument();
    expect(
      within(cardB).queryByRole("button", { name: "標記為已驗證（我確認過結果）" })
    ).not.toBeInTheDocument();
    expect(within(cardB).queryByText("Agent 說已完成，等待檢查")).not.toBeInTheDocument();

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
    const files = ["src/pages/WorkPage.tsx", "src/pages/work/TaskComposer.tsx"];
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
