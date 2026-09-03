// 一般模式狀態投影（CPP §4.2 truthState／§11 truth projection 的 UI 鏡射）：
// 每一個會到達 UI 的原始 taxonomy 字串都有固定人話；未知值不猜、不回原始字串；
// 需要人類裁決的只有 waiting-*／claimed-completed；各頁面（AiPage、通知面板、
// 收件匣、「現在」摘要、全域搜尋）都走同一份投影。

import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AgentSessionRecord, api } from "../api";
import { AppStateProvider } from "../appstate";
import { inboxStatusLabel, NotificationPanel } from "../App";
import { AiPage, SESSION_STATE_LABEL } from "../pages/AiPage";
import { InboxSection } from "../pages/ActivityPage";
import { NowStrip } from "../pages/HomePage";
import { GlobalSearch } from "../components/GlobalSearch";
import {
  agentDisplayLabel,
  capabilityKindLabel,
  inboxKindLabel,
  INBOX_STATUSES,
  isOpenWorkState,
  isWorkState,
  projectInboxStatus,
  projectProviderState,
  projectWorkState,
  WORK_STATE_PROJECTION,
  WORK_STATES,
  WorkState,
} from "../statusProjection";

afterEach(() => {
  vi.restoreAllMocks();
});

/** spec 表格：原始值 → 一般模式標籤（一字不改）。 */
const SPEC_TABLE: Record<WorkState, string> = {
  created: "準備中",
  queued: "準備中",
  fetched: "準備中",
  active: "處理中",
  working: "處理中",
  "waiting-for-input": "等你補充",
  "waiting-input": "等你補充",
  "waiting-for-consent": "等你同意",
  "waiting-consent": "等你同意",
  blocked: "無法繼續",
  "claimed-completed": "Agent 說已完成，等待檢查",
  verified: "已確認完成",
  failed: "執行失敗",
  "timed-out": "執行逾時",
  expired: "已到期",
  unknown: "結果不確定",
  cancelled: "已停止",
  closed: "已停止",
};

const SESSION: AgentSessionRecord = {
  sessionId: "sess-0001",
  providerId: "p-1",
  agentId: "claude-code",
  label: "整理測試報告",
  state: "active",
  lease: { issuedAt: "2026-01-01T00:00:00Z", expiresAt: "2026-01-01T01:00:00Z", renewable: true },
  dataScope: [],
  toolScope: [],
  consentScope: [],
  budget: { maxMessages: 10, spentMessages: 1, maxCost: 0, spentCost: 0 },
  createdAt: "2026-01-01T00:00:00Z",
};

describe("projectWorkState：窮舉對照表", () => {
  it("spec 表格的每一個原始值都對到指定文案，且被標為 known", () => {
    const specKeys = Object.keys(SPEC_TABLE).sort();
    expect([...WORK_STATES].sort()).toEqual(specKeys);
    for (const [raw, label] of Object.entries(SPEC_TABLE)) {
      const p = projectWorkState(raw);
      expect(p.label, raw).toBe(label);
      expect(p.known, raw).toBe(true);
      expect(p.raw).toBe(raw);
      expect(isWorkState(raw)).toBe(true);
    }
  });

  it("未知原始值 → 「結果不確定」＋ known:false，絕不回原始字串", () => {
    for (const raw of ["some-new-state", "", "CLAIMED-COMPLETED", "constructor", "__proto__"]) {
      const p = projectWorkState(raw);
      expect(p.label, raw).toBe("結果不確定");
      expect(p.known, raw).toBe(false);
      expect(p.kind).toBe("unknown");
      expect(p.needsDecision).toBe(false);
      expect(p.raw).toBe(raw);
      expect(p.label).not.toBe(raw);
      expect(isWorkState(raw)).toBe(false);
    }
  });

  it("needsDecision 只在 waiting-* 與 claimed-completed 為 true", () => {
    const expected = new Set<WorkState>([
      "waiting-for-input",
      "waiting-input",
      "waiting-for-consent",
      "waiting-consent",
      "claimed-completed",
    ]);
    for (const raw of WORK_STATES) {
      expect(projectWorkState(raw).needsDecision, raw).toBe(expected.has(raw));
    }
  });

  it("claimed-completed 帶誠實註記，verified 只有綠色；沒有任何路徑把 claimed 翻成 verified", () => {
    const claimed = projectWorkState("claimed-completed");
    expect(claimed.honesty).toBe("Agent 的說法，尚未檢查");
    expect(claimed.kind).toBe("claimed");
    expect(claimed.badge).not.toBe("ok");
    expect(projectWorkState("verified").badge).toBe("ok");
    const okStates = WORK_STATES.filter((s) => WORK_STATE_PROJECTION[s].badge === "ok");
    expect(okStates).toEqual(["verified"]);
  });

  it("isOpenWorkState：對應 Rust is_open（含 fetched／working／queued 別名），未知值不算進行中", () => {
    for (const raw of [
      "created",
      "queued",
      "fetched",
      "active",
      "working",
      "waiting-for-input",
      "waiting-input",
      "waiting-for-consent",
      "waiting-consent",
      "claimed-completed",
    ]) {
      expect(isOpenWorkState(raw), raw).toBe(true);
    }
    for (const raw of [
      "verified",
      "failed",
      "timed-out",
      "expired",
      "unknown",
      "cancelled",
      "closed",
      "blocked",
      "bogus",
    ]) {
      expect(isOpenWorkState(raw), raw).toBe(false);
    }
  });
});

describe("projectInboxStatus／inboxKindLabel", () => {
  it("工作狀態沿用同一份投影；收件匣專屬狀態有自己的人話", () => {
    expect(projectInboxStatus("waiting-for-consent").label).toBe("等你同意");
    expect(projectInboxStatus("claimed-completed").needsDecision).toBe(true);
    expect(projectInboxStatus("candidate")).toMatchObject({
      label: "等待確認",
      needsDecision: true,
      known: true,
    });
    expect(projectInboxStatus("uncertain")).toMatchObject({
      label: "結果不確定",
      kind: "unknown",
      known: true,
    });
    expect(projectInboxStatus("emergency")).toMatchObject({ label: "緊急停止", badge: "bad" });
    expect(projectInboxStatus("sensor.started")).toMatchObject({
      label: "感測使用中",
      badge: "warn",
    });
    expect(projectInboxStatus("sensor.stopped").label).toBe("感測已停止");
    // 動作收據文案與 actionStatusLabel 同句；completed 依 §11 仍是 claimed，不是 verified。
    expect(projectInboxStatus("dispatched").label).toBe("已送出（等待確認）");
    expect(projectInboxStatus("acknowledged").label).toBe("已收到（效果未確認）");
    expect(projectInboxStatus("completed").kind).toBe("claimed");
    expect(projectInboxStatus("observed").kind).toBe("verified");
  });

  it("每一個收件匣狀態都 known，且沒有任何標籤等於原始值", () => {
    for (const raw of INBOX_STATUSES) {
      const p = projectInboxStatus(raw);
      expect(p.known, raw).toBe(true);
      expect(p.label, raw).not.toBe(raw);
      expect(p.label.length, raw).toBeGreaterThan(0);
    }
  });

  it("未知收件匣狀態 → 結果不確定＋known:false", () => {
    const p = projectInboxStatus("some-new-state");
    expect(p.label).toBe("結果不確定");
    expect(p.known).toBe(false);
  });

  it("收件匣種類是人話，沒有 Agent Session 這種原始字", () => {
    expect(inboxKindLabel("agent-session")).toBe("AI 工作階段");
    expect(inboxKindLabel("action-result")).toBe("互動結果");
    expect(inboxKindLabel("safety-event")).toBe("安全事件");
    expect(inboxKindLabel("knowledge-review")).toBe("知識審核");
    expect(inboxKindLabel("ai-assist")).toBe("AI 協助判斷");
    expect(inboxKindLabel("something-else")).toBe("其他活動");
    expect(inboxKindLabel("constructor")).toBe("其他活動");
  });

  it("provider 狀態與能力種類也不外洩原始字", () => {
    expect(projectProviderState("disconnected")).toMatchObject({ label: "未連線", known: true });
    expect(projectProviderState("weird")).toMatchObject({ label: "狀態不確定", known: false });
    expect(capabilityKindLabel("receptor")).toBe("感知來源");
    expect(capabilityKindLabel("tool-operation")).toBe("工具操作");
    expect(capabilityKindLabel("mystery")).toBe("能力");
    expect(agentDisplayLabel("claude-code")).toBe("Claude Code");
    expect(agentDisplayLabel("my-agent")).toBe("my-agent");
  });
});

describe("App.inboxStatusLabel 與 AiPage.SESSION_STATE_LABEL 由投影導出", () => {
  it("inboxStatusLabel 走投影：未知值不回原始字串", () => {
    expect(inboxStatusLabel("waiting-for-consent")).toBe("等你同意");
    expect(inboxStatusLabel("claimed-completed")).toBe("Agent 說已完成，等待檢查");
    expect(inboxStatusLabel("candidate")).toBe("等待確認");
    expect(inboxStatusLabel("some-new-state")).toBe("結果不確定");
  });

  it("SESSION_STATE_LABEL 覆蓋每一個工作狀態且文案與投影一致", () => {
    for (const raw of WORK_STATES) {
      expect(SESSION_STATE_LABEL[raw].text, raw).toBe(SPEC_TABLE[raw]);
      expect(SESSION_STATE_LABEL[raw].kind, raw).toBe(WORK_STATE_PROJECTION[raw].badge);
    }
  });
});

describe("AiPage 工作階段卡片用投影", () => {
  function renderAiPage(state: string, advanced = false) {
    vi.spyOn(api, "agentsDiscoveries").mockResolvedValue({ agents: [] });
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([{ ...SESSION, state }]);
    return render(
      <AppStateProvider ready={false} refreshKey={0}>
        <AiPage refreshKey={0} advanced={advanced} onNavigate={() => {}} />
      </AppStateProvider>
    );
  }

  it("state=fetched（角色 taxonomy 事件別名）顯示「準備中」，不是原始字串", async () => {
    const { container } = renderAiPage("fetched");
    await screen.findByText("整理測試報告");
    expect(screen.getByText("準備中")).toBeInTheDocument();
    expect(container.textContent).not.toContain("fetched");
  });

  it("未知 state 顯示「結果不確定」；一般模式不外洩原始值，進階模式才在次要行顯示", async () => {
    const { container, unmount } = renderAiPage("totally-bogus-state");
    await screen.findByText("整理測試報告");
    expect(screen.getByText("結果不確定")).toBeInTheDocument();
    expect(container.textContent).not.toContain("totally-bogus-state");
    unmount();

    const adv = renderAiPage("totally-bogus-state", true);
    await screen.findByText("整理測試報告");
    expect(screen.getByText("結果不確定")).toBeInTheDocument();
    expect(adv.container.textContent).toContain("totally-bogus-state");
  });

  it("claimed-completed 顯示 spec 文案並保留人工驗證按鈕", async () => {
    renderAiPage("claimed-completed");
    await screen.findByText("整理測試報告");
    expect(screen.getByText("Agent 說已完成，等待檢查")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "標記為已驗證（我確認過結果）" })
    ).toBeInTheDocument();
  });
});

describe("NotificationPanel 徽章用投影", () => {
  it("waiting-for-consent → 等你同意；未知狀態 → 結果不確定，不印原始 enum", () => {
    const inbox = {
      items: [
        {
          kind: "agent-session",
          itemId: "s-1",
          status: "waiting-for-consent",
          title: "等你核可",
          route: "ai",
          needsDecision: true,
        },
        {
          kind: "agent-session",
          itemId: "s-2",
          status: "brand-new-state",
          title: "神秘工作",
          route: "ai",
          needsDecision: true,
        },
      ],
    };
    const { container } = render(
      <NotificationPanel inbox={inbox} onClose={() => {}} onNavigate={() => {}} />
    );
    expect(screen.getByText("等你同意")).toBeInTheDocument();
    expect(screen.getByText("結果不確定")).toBeInTheDocument();
    expect(container.textContent).not.toContain("waiting-for-consent");
    expect(container.textContent).not.toContain("brand-new-state");
  });
});

describe("ActivityPage 收件匣用投影", () => {
  const ITEMS = [
    {
      kind: "agent-session",
      itemId: "s-1",
      status: "waiting-for-consent",
      title: "等你核可的工作",
      occurredAt: "2026-01-01T00:00:00Z",
      route: "ai",
      needsDecision: true,
      agentId: "claude-code",
    },
    {
      kind: "action-result",
      itemId: "a-1",
      status: "uncertain",
      title: "送出桌面通知",
      occurredAt: "2026-01-01T00:00:00Z",
      route: "activity",
      needsDecision: true,
      deviceId: "notify.desktop",
    },
    {
      kind: "safety-event",
      itemId: "e-1",
      status: "sensor.started",
      title: "sensor.started",
      occurredAt: "2026-01-01T00:00:00Z",
      route: "safety",
      needsDecision: false,
    },
  ];

  it("一般模式：狀態與種類是人話，不出現 agent-session／waiting-for-consent", async () => {
    vi.spyOn(api, "activityInbox").mockResolvedValue({
      items: ITEMS,
      count: ITEMS.length,
      totalBeforeLimit: ITEMS.length,
      pendingCount: 2,
    });
    render(<InboxSection refreshKey={0} onNavigate={() => {}} />);
    await screen.findByText("等你核可的工作");
    const list = screen.getByTestId("activity-inbox-results");
    const text = list.textContent ?? "";
    expect(text).toContain("等你同意");
    expect(text).toContain("AI 工作階段");
    expect(text).toContain("結果不確定");
    expect(text).toContain("互動結果");
    expect(text).toContain("感測使用中");
    expect(text).toContain("安全事件");
    expect(text).toContain("Claude Code");
    expect(text).not.toContain("agent-session");
    expect(text).not.toContain("waiting-for-consent");
    expect(text).not.toContain("action-result");
    expect(text).not.toContain("safety-event");
    expect(text).not.toContain("uncertain");
  });

  it("進階模式才在次要行顯示原始狀態碼", async () => {
    vi.spyOn(api, "activityInbox").mockResolvedValue({
      items: ITEMS.slice(0, 1),
      count: 1,
      totalBeforeLimit: 1,
      pendingCount: 1,
    });
    render(<InboxSection refreshKey={0} advanced onNavigate={() => {}} />);
    await screen.findByText("等你核可的工作");
    const text = screen.getByTestId("activity-inbox-results").textContent ?? "";
    expect(text).toContain("等你同意");
    expect(text).toContain("waiting-for-consent");
    expect(text).toContain("agent-session");
  });
});

describe("HomePage NowStrip 進行中計數用投影", () => {
  it("fetched／working 別名算進行中；未知狀態不算", async () => {
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([
      { ...SESSION, sessionId: "a", state: "fetched" },
      { ...SESSION, sessionId: "b", state: "working" },
      { ...SESSION, sessionId: "c", state: "closed" },
      { ...SESSION, sessionId: "d", state: "mystery" },
    ]);
    vi.spyOn(api, "activityInbox").mockResolvedValue({
      items: [],
      count: 0,
      totalBeforeLimit: 0,
      pendingCount: 0,
    });
    vi.spyOn(api, "knowledgeReceipts").mockResolvedValue({ receipts: [] });
    render(<NowStrip refreshKey={0} status={{}} onNavigate={() => {}} />);
    expect(await screen.findByText("2 個工作階段")).toBeInTheDocument();
  });
});

describe("GlobalSearch 動態項目用投影", () => {
  it("工作階段／裝置／能力的細節是人話；未知 session 狀態不外洩", async () => {
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([
      { ...SESSION, sessionId: "x", label: "神秘工作階段", state: "never-seen-before" },
    ]);
    vi.spyOn(api, "providersList").mockResolvedValue([
      {
        identity: { id: "dev-1", displayName: "客廳燈", kind: "hardware" },
        state: "disconnected",
      } as never,
    ]);
    vi.spyOn(api, "memoryList").mockResolvedValue({ items: [] });
    vi.spyOn(api, "knowledgeList").mockResolvedValue({ nodes: [], count: 0 });
    vi.spyOn(api, "domainPacks").mockResolvedValue({ packs: [] });
    vi.spyOn(api, "actionsList").mockResolvedValue([]);
    vi.spyOn(api, "knowledgeReceipts").mockResolvedValue({ receipts: [] });
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
    await userEvent.type(input, "神秘工作階段");
    await screen.findByText("工作階段：神秘工作階段");
    expect(container.textContent).toContain("結果不確定");
    expect(container.textContent).not.toContain("never-seen-before");

    await userEvent.clear(input);
    await userEvent.type(input, "客廳燈");
    await screen.findByText("裝置：客廳燈");
    expect(container.textContent).toContain("未連線");
    expect(container.textContent).not.toContain("disconnected");
  });
});
