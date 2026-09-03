// v0.5 Phase 7 對抗審查「第二輪」缺陷的 regression tests。
// 每個 describe 對應一個被獨立懷疑者確認的缺陷：
//   - SSE 訂閱游標：初次連線不重放整個 ring buffer；daemon 重啟後不沿用舊序號。
//   - 角色重播守門：時間戳早於 App 啟動的重播事件不驅動演出。
//   - Approval 裁決在介面上看得見：已裁決的請求不再顯示可按的核可／拒絕。
//   - Global Search 一般模式說人話：不外洩 layer id／治理狀態／完整 UUID。
//   - 已到期的記憶不假裝救得回來：不留一顆按下去只會靜默失敗的按鈕。

import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { api, AgentSessionRecord } from "../api";
import { AppStateProvider } from "../appstate";
import { GlobalSearch, shortId } from "../components/GlobalSearch";
import {
  initial,
  mapRuntimeEvent,
  isReplayedBeforeStart,
  pose,
  reduce,
  setAppStartedAt,
} from "../companion/machine";
import {
  AiPage,
  approvalResolutions,
  approvalResolutionText,
  APPROVAL_TTL_SECONDS,
} from "../pages/AiPage";
import { MemoryKnowledgePage } from "../pages/MemoryKnowledgePage";
import { INITIAL_STREAM_CURSOR, nextStreamCursor } from "../transport";
import TRANSPORT_SOURCE from "../transport.ts?raw";

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

// ---------------------------------------------------------------------------

describe("SSE 訂閱游標：初次連線只要新事件，重啟後不沿用舊序號", () => {
  const RUN_A = { startedAt: "2026-08-28T09:00:00Z", eventSequence: 412 };
  const RUN_B = { startedAt: "2026-08-28T10:30:00Z", eventSequence: 3 };

  it("初次連線從目前序號開始，不以 Last-Event-ID: 0 重放整個 buffer", () => {
    const { cursor, reset } = nextStreamCursor(INITIAL_STREAM_CURSOR, RUN_A);
    expect(cursor.lastId).toBe("412");
    expect(cursor.instance).toBe(RUN_A.startedAt);
    // 初次連線不是「重置」（沒有舊實例的緩衝要丟）。
    expect(reset).toBe(false);
  });

  it("同一個 daemon 重連：續用 lastId，不漏事件", () => {
    const connected = nextStreamCursor(INITIAL_STREAM_CURSOR, RUN_A).cursor;
    const advanced = { ...connected, lastId: "460" };
    const again = nextStreamCursor(advanced, RUN_A);
    expect(again.cursor.lastId).toBe("460");
    expect(again.reset).toBe(false);
  });

  it("daemon 重啟（序號從頭來）：重置游標並丟掉跨實例的重播", () => {
    const stale = { instance: RUN_A.startedAt, lastId: "460" };
    const restarted = nextStreamCursor(stale, RUN_B);
    // 舊序號 460 會把新 daemon 的 1..3 號事件全部吞掉；而反過來若新
    // daemon 跑久了，舊的 verified 事件會被當成新事件重播出綠勾。
    expect(restarted.cursor.instance).toBe(RUN_B.startedAt);
    expect(restarted.cursor.lastId).toBe("3");
    expect(restarted.reset).toBe(true);
  });

  it("runStream 真的走這條路：先問 /v1/status，再用游標訂閱 /v1/events", () => {
    // 純函式正確但沒接上去，缺陷依舊存在——所以連接線本身也要被守住。
    expect(TRANSPORT_SOURCE).not.toMatch(/let\s+lastId\s*=\s*"0"/);
    const statusAt = TRANSPORT_SOURCE.indexOf("/v1/status`");
    const eventsAt = TRANSPORT_SOURCE.indexOf("/v1/events`");
    expect(statusAt).toBeGreaterThan(-1);
    expect(eventsAt).toBeGreaterThan(statusAt);
    expect(TRANSPORT_SOURCE).toContain("nextStreamCursor(cursor,");
    expect(TRANSPORT_SOURCE).toContain('"Last-Event-ID": cursor.lastId');
  });

  it("讀不到 startedAt／eventSequence 就不猜：游標維持原狀", () => {
    const stale = { instance: RUN_A.startedAt, lastId: "460" };
    for (const bad of [null, {}, { startedAt: "x" }, { eventSequence: 5 }, "nope"]) {
      const out = nextStreamCursor(stale, bad);
      expect(out.cursor).toBe(stale);
      expect(out.reset).toBe(false);
    }
  });
});

// ---------------------------------------------------------------------------

describe("角色重播守門：早於本次啟動的事件不驅動演出", () => {
  const START = Date.parse("2026-08-28T12:00:00Z");
  afterEach(() => setAppStartedAt(0));

  it("重播的 action.observed 不會再演一次綠勾", () => {
    setAppStartedAt(START);
    const replayed = {
      eventType: "action.observed",
      payload: {},
      timestamp: "2026-08-20T08:00:00Z",
    };
    expect(isReplayedBeforeStart(replayed)).toBe(true);
    expect(mapRuntimeEvent(replayed)).toBeNull();
    // 走完整條路：重播事件對狀態機沒有任何影響 → 仍是待機。
    const state = reduce(initial, { type: "base", base: "idle" }, START);
    const mapped = mapRuntimeEvent(replayed);
    const after = mapped ? reduce(state, mapped, START) : state;
    expect(pose(after, START).animation).toBe("idle");
  });

  it("本次啟動之後的事件照常演出", () => {
    setAppStartedAt(START);
    const live = {
      eventType: "action.observed",
      payload: {},
      timestamp: "2026-08-28T12:00:05Z",
    };
    expect(isReplayedBeforeStart(live)).toBe(false);
    expect(mapRuntimeEvent(live)).toMatchObject({ kind: "succeeded", verified: true });
  });

  it("沒有時間戳就不猜：行為與過去完全相同", () => {
    setAppStartedAt(START);
    expect(isReplayedBeforeStart({ eventType: "action.failed", payload: {} })).toBe(false);
    expect(mapRuntimeEvent({ eventType: "action.failed", payload: {} })).toMatchObject({
      kind: "failed",
    });
  });
});

// ---------------------------------------------------------------------------

const CONSENT_SESSION: AgentSessionRecord = {
  sessionId: "sess-approval-1",
  providerId: "p-1",
  agentId: "codex",
  label: "等待核可的工作",
  state: "waiting-for-consent",
  lease: { issuedAt: "2026-01-01T00:00:00Z", expiresAt: "2126-01-01T01:00:00Z", renewable: true },
  dataScope: [],
  toolScope: [],
  consentScope: [],
  budget: { maxMessages: 10, spentMessages: 1, maxCost: 0, spentCost: 0 },
  createdAt: "2026-01-01T00:00:00Z",
};

const APPROVAL_REQUEST = {
  messageId: "msg-1",
  kind: "approval-request",
  createdAt: new Date().toISOString(),
  body: { requestId: "9001", summary: "codex 請求核可：刪除檔案" },
};

function renderAiPage() {
  vi.spyOn(api, "agentsDiscoveries").mockResolvedValue({ agents: [] });
  vi.spyOn(api, "agentSessionsList").mockResolvedValue([CONSENT_SESSION]);
  return render(
    <AppStateProvider ready={false} refreshKey={0}>
      <AiPage refreshKey={0} onNavigate={() => {}} />
    </AppStateProvider>
  );
}

describe("Approval 裁決在 AiPage 上看得見", () => {
  it("approvalResolutions／approvalResolutionText 分得出誰決定的", () => {
    const map = approvalResolutions([
      APPROVAL_REQUEST,
      {
        kind: "approval-resolved",
        body: { requestId: "9001", decision: "denied", by: "watchdog", deliveredToAgent: true },
      },
    ]);
    expect(map.get("9001")).toEqual({
      decision: "denied",
      by: "watchdog",
      deliveredToAgent: true,
    });
    expect(approvalResolutionText(map.get("9001")!)).toBe(
      `已由看門狗自動拒絕（${APPROVAL_TTL_SECONDS} 秒無人回應）`
    );
    expect(
      approvalResolutionText({ decision: "approved", by: "human", deliveredToAgent: true })
    ).toBe("你已核可");
    // 裁決成立 ≠ 裁決送到 agent。
    expect(
      approvalResolutionText({ decision: "denied", by: "human", deliveredToAgent: false })
    ).toContain("沒能送到 agent");
  });

  it("尚未裁決：核可／拒絕按鈕在", async () => {
    vi.spyOn(api, "agentSessionMessages").mockResolvedValue([APPROVAL_REQUEST]);
    renderAiPage();
    await userEvent.click(await screen.findByRole("button", { name: "查看結果／訊息" }));
    expect(await screen.findByRole("button", { name: "核可" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "拒絕" })).toBeInTheDocument();
  });

  it("看門狗逾時自動拒絕之後：按鈕消失，畫面說出是誰拒絕的", async () => {
    vi.spyOn(api, "agentSessionMessages").mockResolvedValue([
      APPROVAL_REQUEST,
      {
        messageId: "msg-2",
        kind: "approval-resolved",
        createdAt: new Date().toISOString(),
        body: {
          requestId: "9001",
          summary: "codex 請求核可：刪除檔案",
          decision: "denied",
          approved: false,
          by: "watchdog",
          deliveredToAgent: true,
        },
      },
    ]);
    const { container } = renderAiPage();
    await userEvent.click(await screen.findByRole("button", { name: "查看結果／訊息" }));
    await screen.findByText(/已由看門狗自動拒絕/);
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "核可" })).not.toBeInTheDocument()
    );
    expect(screen.queryByRole("button", { name: "拒絕" })).not.toBeInTheDocument();
    expect(container.textContent).toContain(`${APPROVAL_TTL_SECONDS} 秒無人回應`);
  });
});

// ---------------------------------------------------------------------------

describe("Global Search 一般模式說人話", () => {
  const UPDATE_ID = "b1c2d3e4-5678-90ab-cdef-1234567890ab";

  function stubSearchApis() {
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([
      { ...CONSENT_SESSION, label: "報告工作階段", state: "claimed-completed" },
    ]);
    vi.spyOn(api, "providersList").mockResolvedValue([]);
    vi.spyOn(api, "memoryList").mockResolvedValue({
      items: [{ memoryId: "m-1", title: "早餐偏好", layer: "user-memory" }],
    });
    vi.spyOn(api, "knowledgeList").mockResolvedValue({
      nodes: [{ nodeId: "k-1", title: "咖啡沖煮", status: "candidate" }],
      count: 1,
    });
    vi.spyOn(api, "domainPacks").mockResolvedValue({ packs: [] });
    vi.spyOn(api, "actionsList").mockResolvedValue([
      {
        actionId: "a-1",
        planId: "p-1",
        actuatorId: "notify.desktop",
        intent: "送出桌面通知",
        currentStatus: "completed",
        timestamps: [],
        policyDecisions: [],
        effectiveBoundedParameters: {},
        requestedParameters: {},
        errors: [],
      },
    ]);
    vi.spyOn(api, "knowledgeReceipts").mockResolvedValue({
      receipts: [{ triggeredBy: "手動複審", updateId: UPDATE_ID }],
    });
  }

  async function openAndSearch(query: string) {
    const { container } = render(
      <AppStateProvider ready={false} refreshKey={0}>
        <GlobalSearch
          open
          onClose={() => {}}
          onNavigate={() => {}}
          estopped={false}
          onEstop={async () => {}}
          onCommandFeedback={() => {}}
        />
      </AppStateProvider>
    );
    const input = screen.getByPlaceholderText(/搜尋設定、能力、記憶、知識/);
    await userEvent.type(input, query);
    return container;
  }

  it("記憶分層／知識狀態／收據狀態全部走人話對照，不外洩原始 id", async () => {
    stubSearchApis();
    const container = await openAndSearch("早餐");
    await screen.findByText("記憶：早餐偏好");
    // user-memory 在一般模式走 memoryLayerLabel 的人話分組（「你告訴我的事」），
    // 不是進階模式專用的 LAYER_LABEL 技術對照表（「關於我的記憶」）——兩者在這個
    // key 上文字不同，之前用「關於我的記憶」斷言其實是巧合撞到 bug 的輸出。
    expect(container.textContent).toContain("你告訴我的事");
    expect(container.textContent).not.toContain("user-memory");
    expect(container.textContent).not.toContain("關於我的記憶");

    const input = screen.getByPlaceholderText(/搜尋設定、能力、記憶、知識/);
    await userEvent.clear(input);
    await userEvent.type(input, "咖啡沖煮");
    await screen.findByText("知識：咖啡沖煮");
    expect(container.textContent).toContain("等待確認");
    expect(container.textContent).not.toContain("candidate");

    await userEvent.clear(input);
    await userEvent.type(input, "送出桌面通知");
    // v0.5 一般模式：「收據」是治理術語，只在進階模式出現；一般模式叫「互動結果」。
    await screen.findByText("互動結果：送出桌面通知");
    expect(container.textContent).toContain("已完成");
    expect(container.textContent).not.toContain("completed");
    expect(container.textContent).not.toContain("收據");

    await userEvent.clear(input);
    await userEvent.type(input, "報告工作階段");
    await screen.findByText("工作階段：報告工作階段");
    // 文案以共用狀態投影（statusProjection.ts）的 spec 表格為準：claimed ≠ verified。
    expect(container.textContent).toContain("對方說已完成");
    expect(container.textContent).not.toContain("claimed-completed");
  });

  it("一般模式的 UUID 只留後 6 碼；進階模式才是完整識別碼", async () => {
    stubSearchApis();
    const container = await openAndSearch("手動複審");
    // v0.5 一般模式：「知識收據」→「知識更新」（進階模式仍叫收據）。
    await screen.findByText("知識更新：手動複審");
    expect(container.textContent).not.toContain(UPDATE_ID);
    expect(container.textContent).toContain("…7890ab");
    expect(container.textContent).not.toContain("收據");
    // 進階模式保留完整識別碼（零能力退化）。
    expect(shortId(UPDATE_ID, true)).toBe(UPDATE_ID);
    expect(shortId(UPDATE_ID, false)).toBe("…7890ab");
    expect(shortId("short", false)).toBe("short");
  });

  function stubTechnicalLayerMemories() {
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([]);
    vi.spyOn(api, "providersList").mockResolvedValue([]);
    vi.spyOn(api, "memoryList").mockResolvedValue({
      items: [
        { memoryId: "m-handoff", title: "接手的待辦", layer: "agent-handoff" },
        { memoryId: "m-skill", title: "學到的沖泡技巧", layer: "skill" },
      ],
    });
    vi.spyOn(api, "knowledgeList").mockResolvedValue({ nodes: [], count: 0 });
    vi.spyOn(api, "domainPacks").mockResolvedValue({ packs: [] });
    vi.spyOn(api, "actionsList").mockResolvedValue([]);
    vi.spyOn(api, "knowledgeReceipts").mockResolvedValue({ receipts: [] });
  }

  it("一般模式：memory-ui-006——agent-handoff／skill 這類技術分層不外洩，走人話分組", async () => {
    stubTechnicalLayerMemories();
    const container = await openAndSearch("接手");
    await screen.findByText("記憶：接手的待辦");
    // 技術分層字串（原始 layer id 與 LAYER_LABEL 的技術文案）一律不得出現。
    expect(container.textContent).not.toContain("agent-handoff");
    expect(container.textContent).not.toContain("skill");
    expect(container.textContent).not.toContain("Agent 交接");
    expect(container.textContent).not.toContain("Skill");
    // 一般模式該顯示的是人話分組。
    expect(container.textContent).toContain("工作與任務");

    const input = screen.getByPlaceholderText(/搜尋設定、能力、記憶、知識/);
    await userEvent.clear(input);
    await userEvent.type(input, "沖泡技巧");
    await screen.findByText("記憶：學到的沖泡技巧");
    expect(container.textContent).not.toContain("agent-handoff");
    expect(container.textContent).not.toContain("skill");
    expect(container.textContent).not.toContain("Agent 交接");
    expect(container.textContent).not.toContain("Skill");
    expect(container.textContent).toContain("學到的知識");
  });

  it("進階模式：同一批記憶改回顯示技術分層（零能力退化）", async () => {
    stubTechnicalLayerMemories();
    vi.spyOn(api, "uiPrefsGet").mockResolvedValue({
      mode: "advanced",
      locale: "zh-TW",
      customNames: {},
      schemaVersion: "1.0",
    });
    const { container } = render(
      <AppStateProvider ready={true} refreshKey={0}>
        <GlobalSearch
          open
          onClose={() => {}}
          onNavigate={() => {}}
          estopped={false}
          onEstop={async () => {}}
          onCommandFeedback={() => {}}
        />
      </AppStateProvider>
    );
    const input = await screen.findByPlaceholderText(/搜尋設定、能力、記憶、知識/);
    await userEvent.type(input, "接手");
    await screen.findByText("記憶：接手的待辦");
    expect(container.textContent).toContain("Agent 交接");

    await userEvent.clear(input);
    await userEvent.type(input, "沖泡技巧");
    await screen.findByText("記憶：學到的沖泡技巧");
    expect(container.textContent).toContain("Skill");
  });
});

// ---------------------------------------------------------------------------

describe("已到期的記憶不假裝救得回來", () => {
  const PAST = "2020-01-01T00:00:00Z";

  function memory(overrides: Record<string, unknown>) {
    return {
      memoryId: "m-x",
      title: "某條記憶",
      kind: "preference",
      layer: "user-memory",
      content: "內容",
      createdBy: { kind: "human" },
      retention: {},
      ...overrides,
    };
  }

  function stubMemoryApis(items: Record<string, unknown>[]) {
    vi.spyOn(api, "memoryList").mockResolvedValue({ items });
    vi.spyOn(api, "knowledgeList").mockResolvedValue({ nodes: [], count: 0 });
    vi.spyOn(api, "assetsList").mockResolvedValue({ assets: [], count: 0 });
    vi.spyOn(api, "knowledgeReceipts").mockResolvedValue({ receipts: [] });
    vi.spyOn(api, "domainPacks").mockResolvedValue({ packs: [] });
  }

  async function cardFor(title: string): Promise<HTMLElement> {
    const heading = await screen.findByText(title);
    return heading.closest(".provider-card") as HTMLElement;
  }

  it("到期項只給「刪除」，並說明已停止使用（後端 PATCH 一律 NotFound）", async () => {
    stubMemoryApis([
      memory({
        memoryId: "m-expired",
        title: "過期的偏好",
        status: "expired",
        retention: { expiresAt: PAST },
      }),
    ]);
    render(<MemoryKnowledgePage refreshKey={0} advanced />);
    const card = await cardFor("過期的偏好");
    expect(within(card).getByRole("button", { name: "刪除" })).toBeInTheDocument();
    expect(within(card).queryByRole("button", { name: /重新確認/ })).not.toBeInTheDocument();
    expect(card.textContent).toContain("已過保存期限");
  });

  it("已過複查期（stale）仍可重新確認，而且失敗要看得見，不得靜默", async () => {
    stubMemoryApis([
      memory({
        memoryId: "m-stale",
        title: "待複查的偏好",
        status: "stale",
        retention: { reviewAfter: PAST },
      }),
    ]);
    const patch = vi
      .spyOn(api, "memoryPatch")
      .mockRejectedValue(new Error("404: memory m-stale (expired)"));
    render(<MemoryKnowledgePage refreshKey={0} advanced />);
    const card = await cardFor("待複查的偏好");
    const button = within(card).getByRole("button", { name: /重新確認/ });
    await userEvent.click(button);
    await waitFor(() => expect(patch).toHaveBeenCalled());
    expect(await within(card).findByText(/重新確認失敗/)).toBeInTheDocument();
    expect(card.textContent).toContain("保存期限沒有變更");
  });
});
