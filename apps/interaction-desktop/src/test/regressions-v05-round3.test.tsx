// v0.5 對抗審查第三輪（2e02284-20260902T080415Z）確認缺陷的 regression tests（desktop 頁面）：
// - ia-settings-035：pendingCount>0 但本頁沒裝到待決定時，通知中心不得說「目前沒有待決定事項」；
//   優先用後端 needsDecision 篩選，舊 daemon（deny_unknown_fields）退回不帶篩選的查詢。
// - ia-settings-036：「emergency-cleared」投影成「緊急停止已解除」；安全事件標題不印原始 event_type。
// - memory-ui-001：Bundle `excluded.needsReview` 是 id 陣列，要算成「N 條」而不是 NaN→「沒有」。
// - memory-ui-003：記憶與知識頁不寫死「小樞」，一律 useCharacterName。
// - memory-ui-005：角色互動記憶的說明只列真的會記的三類，不宣稱「相處距離」。
// - memory-ui-006：一般模式的預覽明說「不含工作階段授權的領域知識」並顯示被擋數量。
// - memory-ui-008：素材與來源依 advanced 分層：一般模式人話，原始 JSON／狀態碼只在進階。
// - agent-honesty-028／031：AiPage 綠勾只給「目前這一輪」的 claim；「進行中」與 Rust is_open 對齊。
// - ia-settings-039：工作頁「目前分工：」不冒充精靈選擇；四角色都 none → 「全部不交給 Agent」。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AgentSessionRecord, api } from "../api";
import { AppStateProvider } from "../appstate";
import { NotificationPanel } from "../App";
import { primeCharacterNameForTests, resetCharacterNameForTests } from "../characterName";
import { AiPage, verifiedForCurrentClaim } from "../pages/AiPage";
import { decisionPage, loadDecisionInbox, resetDecisionInboxProbeForTests } from "../pages/ConnectPage";
import {
  assetImpactSummary,
  BUNDLE_PREVIEW_DOMAINS_NOTE,
  BundleHumanSummary,
  excludedCount,
  MemoryKnowledgePage,
} from "../pages/MemoryKnowledgePage";
import { agentRouteSummary, WorkPage } from "../pages/WorkPage";
import {
  inboxItemTitle,
  INBOX_STATUSES,
  isOpenWorkState,
  projectInboxStatus,
} from "../statusProjection";
import { emptyMemory } from "../companion/interactionMemory";
import MEMORY_PAGE_SOURCE from "../pages/MemoryKnowledgePage.tsx?raw";

afterEach(() => {
  vi.restoreAllMocks();
  resetCharacterNameForTests();
  resetDecisionInboxProbeForTests();
});

/** 20 筆最近的、都不需要決定的項目（例如已完成的通知收據）。 */
function twentyDoneItems(): Record<string, unknown>[] {
  return Array.from({ length: 20 }, (_, i) => ({
    kind: "action-result",
    itemId: `a-${i}`,
    status: "completed",
    title: `送出桌面通知 ${i}`,
    route: "activity",
    needsDecision: false,
  }));
}

// ---------------------------------------------------------------------------
// ia-settings-035：通知中心與待決定清單
// ---------------------------------------------------------------------------

describe("通知中心：pendingCount 與本頁不一致時照實說「還有 N 項不在這一頁」", () => {
  it("pendingCount:3、本頁 20 筆都不需決定 → 不得說「目前沒有待決定事項」", () => {
    render(
      <NotificationPanel
        inbox={{ pendingCount: 3, count: 20, totalBeforeLimit: 23, items: twentyDoneItems() }}
        onClose={() => {}}
        onNavigate={() => {}}
      />
    );
    expect(screen.queryByText("目前沒有待決定事項。")).not.toBeInTheDocument();
    expect(screen.getByRole("status").textContent).toContain("還有 3 項待決定不在這一頁");
    expect(screen.getByRole("status").textContent).toContain("前往活動歷史");
  });

  it("本頁裝不下（12 筆待決定只列 10 筆）→ 列 10 筆＋「還有 2 項」", () => {
    const items = Array.from({ length: 12 }, (_, i) => ({
      kind: "agent-session",
      itemId: `s-${i}`,
      status: "waiting-for-consent",
      title: `工作 ${i}`,
      route: "ai",
      needsDecision: true,
    }));
    render(
      <NotificationPanel inbox={{ pendingCount: 12, items }} onClose={() => {}} onNavigate={() => {}} />
    );
    expect(screen.getAllByRole("button", { name: "前往" })).toHaveLength(10);
    expect(screen.getByRole("status").textContent).toContain("還有 2 項待決定不在這一頁");
  });

  it("真的沒有待決定（pendingCount:0 且本頁沒有）才說「目前沒有待決定事項」", () => {
    render(
      <NotificationPanel
        inbox={{ pendingCount: 0, items: twentyDoneItems() }}
        onClose={() => {}}
        onNavigate={() => {}}
      />
    );
    expect(screen.getByText("目前沒有待決定事項。")).toBeInTheDocument();
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("decisionPage：pendingCount 缺席／不合法時退回本頁計數，不會變 NaN", () => {
    const items = [{ needsDecision: true, itemId: "x" }, { needsDecision: false, itemId: "y" }];
    expect(decisionPage({ items }, 10)).toMatchObject({ notShown: 0, pendingCount: 1 });
    expect(decisionPage({ items, pendingCount: "bogus" }, 10)).toMatchObject({ notShown: 0, pendingCount: 1 });
    expect(decisionPage({ items, pendingCount: -4 }, 10)).toMatchObject({ notShown: 0, pendingCount: 1 });
    expect(decisionPage({ items, pendingCount: 5 }, 10)).toMatchObject({ notShown: 4, pendingCount: 5 });
    expect(decisionPage(null, 10)).toMatchObject({ shown: [], notShown: 0, pendingCount: 0 });
  });
});

describe("loadDecisionInbox：優先 needsDecision 篩選，舊 daemon 退回不帶篩選", () => {
  beforeEach(() => {
    resetDecisionInboxProbeForTests();
  });

  it("新 daemon：以 { limit, needsDecision: true } 查詢並直接回傳", async () => {
    const payload = { pendingCount: 1, items: [{ needsDecision: true, itemId: "s" }] };
    const spy = vi.spyOn(api, "activityInbox").mockResolvedValue(payload);
    await expect(loadDecisionInbox(20)).resolves.toBe(payload);
    expect(spy).toHaveBeenCalledTimes(1);
    expect(spy).toHaveBeenCalledWith({ limit: 20, needsDecision: true });
  });

  it("舊 daemon（deny_unknown_fields 拒絕）：退回 { limit }，之後不再送 needsDecision", async () => {
    const plain = { pendingCount: 3, items: twentyDoneItems() };
    const spy = vi.spyOn(api, "activityInbox").mockImplementation(async (filter) => {
      if (filter?.needsDecision !== undefined) {
        throw new Error("unknown field `needsDecision`, expected one of `status`, `agent`, `device`, `task`, `domain`, `since`, `limit`");
      }
      return plain;
    });
    await expect(loadDecisionInbox(20)).resolves.toBe(plain);
    expect(spy.mock.calls.map(([f]) => f)).toEqual([{ limit: 20, needsDecision: true }, { limit: 20 }]);
    spy.mockClear();
    await expect(loadDecisionInbox(20)).resolves.toBe(plain);
    expect(spy.mock.calls.map(([f]) => f)).toEqual([{ limit: 20 }]);
  });

  it("兩次都失敗（後端失聯）照實往上丟，不編造空清單", async () => {
    vi.spyOn(api, "activityInbox").mockRejectedValue(new Error("offline"));
    await expect(loadDecisionInbox(20)).rejects.toThrow("offline");
  });
});

// ---------------------------------------------------------------------------
// ia-settings-036：安全事件投影與標題
// ---------------------------------------------------------------------------

describe("安全事件：解除緊急停止不是再一次「緊急停止」；標題不印原始 event_type", () => {
  it("emergency-cleared 是已知狀態，標籤「緊急停止已解除」、badge ok", () => {
    expect(INBOX_STATUSES).toContain("emergency-cleared");
    expect(projectInboxStatus("emergency-cleared")).toMatchObject({
      label: "緊急停止已解除",
      badge: "ok",
      known: true,
      needsDecision: false,
    });
    expect(projectInboxStatus("emergency")).toMatchObject({ label: "緊急停止", badge: "bad" });
  });

  it("inboxItemTitle：舊 daemon 的原始標題換成人話；後端已給人話就照用；其它種類不動", () => {
    expect(inboxItemTitle({ kind: "safety-event", status: "emergency", title: "emergency.stop" })).toBe(
      "緊急停止已啟動"
    );
    expect(
      inboxItemTitle({ kind: "safety-event", status: "emergency-cleared", title: "emergency.stop" })
    ).toBe("緊急停止已解除");
    expect(
      inboxItemTitle({
        kind: "safety-event",
        status: "sensor.started",
        title: "sensor.started",
        detail: { payload: { sensor: "microphone" } },
      })
    ).toBe("感測開始：麥克風");
    expect(
      inboxItemTitle({ kind: "safety-event", status: "sensor.stopped", title: "sensor.stopped" })
    ).toBe("感測結束");
    expect(
      inboxItemTitle({ kind: "safety-event", status: "emergency", title: "緊急停止被觸發（桌面按鈕）" })
    ).toBe("緊急停止被觸發（桌面按鈕）");
    expect(inboxItemTitle({ kind: "safety-event", status: "weird.event", title: "weird.event" })).toBe(
      "安全事件"
    );
    expect(inboxItemTitle({ kind: "agent-session", status: "active", title: "整理報告" })).toBe("整理報告");
  });

  it("通知面板列出安全事件時用人話標題", () => {
    const { container } = render(
      <NotificationPanel
        inbox={{
          pendingCount: 1,
          items: [
            {
              kind: "safety-event",
              itemId: "e-1",
              status: "emergency-cleared",
              title: "emergency.stop",
              route: "safety",
              needsDecision: true,
            },
          ],
        }}
        onClose={() => {}}
        onNavigate={() => {}}
      />
    );
    expect(container.textContent).toContain("緊急停止已解除");
    expect(container.textContent).not.toContain("emergency.stop");
  });
});

// ---------------------------------------------------------------------------
// memory-ui-001／003／005／006／008：記憶與知識頁
// ---------------------------------------------------------------------------

function stubMemoryApis() {
  vi.spyOn(api, "memoryList").mockResolvedValue({ items: [] });
  vi.spyOn(api, "knowledgeList").mockResolvedValue({ nodes: [], count: 0 });
  vi.spyOn(api, "assetsList").mockResolvedValue({ assets: [], count: 0 });
  vi.spyOn(api, "knowledgeReceipts").mockResolvedValue({ receipts: [] });
  vi.spyOn(api, "domainPacks").mockResolvedValue({ packs: [] });
}

describe("Context Bundle 摘要：needsReview 是 id 陣列，要算成條數", () => {
  it("excludedCount：陣列取長度、數字取整、其它為 0", () => {
    expect(excludedCount(["mem-a", "mem-b"])).toBe(2);
    expect(excludedCount([])).toBe(0);
    expect(excludedCount(2)).toBe(2);
    expect(excludedCount("3")).toBe(3);
    expect(excludedCount(undefined)).toBe(0);
    expect(excludedCount(NaN)).toBe(0);
    expect(excludedCount(-1)).toBe(0);
  });

  it("BundleHumanSummary：excluded.needsReview:['mem-a'] → 「需要你重新確認 1 條」而不是「沒有」", () => {
    const { container } = render(
      <BundleHumanSummary
        bundle={{ includes: [], excluded: { needsReview: ["mem-a"], sensitive: 0, notVisibleToAgent: 0 } }}
      />
    );
    const text = container.textContent ?? "";
    expect(text).toContain("需要你重新確認 1 條");
    expect(text).not.toContain("擋下來的：沒有");
    expect(text).not.toContain("NaN");
  });

  it("一般模式預覽：mock memoryBundle 回 needsReview:['m1']，畫面出現「需要你重新確認 1 條」與領域知識註記", async () => {
    stubMemoryApis();
    primeCharacterNameForTests({ name: "小樞", pronoun: "她", characterId: "shu-maid" });
    const bundle = vi.spyOn(api, "memoryBundle").mockResolvedValue({
      includes: [],
      excluded: { needsReview: ["m1"], notVisibleToAgent: 0, sensitive: 0 },
    });
    const { container } = render(<MemoryKnowledgePage refreshKey={0} advanced={false} />);
    expect(await screen.findByText("本次會提供給 AI 的內容")).toBeInTheDocument();
    expect(container.textContent).toContain(BUNDLE_PREVIEW_DOMAINS_NOTE);
    await userEvent.type(screen.getByPlaceholderText("任務描述…"), "整理報告");
    await userEvent.click(screen.getByRole("button", { name: "預覽" }));
    await waitFor(() => expect(bundle).toHaveBeenCalledWith("整理報告", "claude-code", []));
    await screen.findByText(/需要你重新確認 1 條/);
    expect(container.textContent).not.toContain("擋下來的：沒有");
    // 一般模式仍不外洩技術字樣。
    expect(container.textContent).not.toContain("Context Bundle");
    expect(container.textContent).not.toContain("候選");
  });
});

describe("記憶與知識頁不寫死「小樞」", () => {
  it("角色叫阿樂：分頁、前往按鈕、知識區標題、糾正面板都用阿樂，畫面沒有「小樞」", async () => {
    stubMemoryApis();
    primeCharacterNameForTests({ name: "阿樂", pronoun: "角色", characterId: "text" });
    const navigate = vi.fn();
    const { container } = render(
      <MemoryKnowledgePage refreshKey={0} advanced={false} onNavigate={navigate} />
    );
    const tabs = await screen.findAllByRole("tab");
    expect(tabs.map((t) => t.textContent)).toEqual(["關於我的記憶", "阿樂學會的知識", "素材與來源"]);
    await screen.findByText(/阿樂跟你玩耍、互動累積的角色記憶/);
    expect(screen.getByRole("button", { name: "前往阿樂" })).toBeInTheDocument();
    expect(container.textContent).not.toContain("小樞");

    await userEvent.click(screen.getByRole("tab", { name: "阿樂學會的知識" }));
    expect(await screen.findByText("糾正阿樂的記憶或說法")).toBeInTheDocument();
    const details = screen.getByText("糾正阿樂的記憶或說法").closest("details") as HTMLDetailsElement;
    details.open = true;
    expect(container.textContent).toContain("不會馬上變成阿樂的通用說法");
    expect(container.textContent).not.toContain("小樞");

    await userEvent.click(screen.getByRole("tab", { name: "素材與來源" }));
    await screen.findByRole("button", { name: "加入素材" });
    expect(screen.getByRole("heading", { name: "素材與來源" })).toBeInTheDocument();
    expect(container.textContent).not.toContain("小樞");
  });

  it("原始碼（註解除外）不含「小樞」，並使用 useCharacterName", () => {
    const stripped = MEMORY_PAGE_SOURCE.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");
    expect(stripped).not.toContain("小樞");
    expect(MEMORY_PAGE_SOURCE).toContain("useCharacterName");
  });
});

describe("角色互動記憶的說明只列真的會記的三類", () => {
  it("一般模式：文案列出玩過的玩具／常關掉的反應／相處天數，不含「距離」；與 emptyMemory 的類別一致", async () => {
    stubMemoryApis();
    primeCharacterNameForTests({ name: "小樞", pronoun: "她", characterId: "shu-maid" });
    const { container } = render(<MemoryKnowledgePage refreshKey={0} advanced={false} />);
    const copy = await screen.findByText(/小樞跟你玩耍、互動累積的角色記憶/);
    expect(copy.textContent).toContain("玩過的玩具");
    expect(copy.textContent).toContain("常關掉的反應");
    expect(copy.textContent).toContain("相處天數");
    expect(container.textContent).not.toContain("距離");
    // 記憶資料類別：toys／disabledReactions／daysSeen（events／lastDay／lastSeenAt 是內部欄位）。
    const keys = Object.keys(emptyMemory());
    expect(keys).toEqual(expect.arrayContaining(["toys", "disabledReactions", "daysSeen"]));
    expect(keys).not.toContain("preferredDistance");
  });
});

describe("素材與來源依 advanced 分層", () => {
  const ASSET = {
    hash: "b".repeat(64),
    mediaType: "image",
    sizeBytes: 1234,
    source: "user-import",
    originalName: "receipt.png",
  };
  const IMPACT = {
    hash: "b".repeat(64),
    referencingKnowledgeNodes: ["kn-11111111-2222-3333-4444-555555555555"],
    memoriesDeletedWithParent: [],
    derivativesRemoved: 2,
    derivedAssetsRemoved: ["c".repeat(64)],
    derivedAssetsRetainedShared: [],
    note: "引用中的 Active 知識不會被靜默刪除——會標記 disputed（失去來源），需人工處理。",
  };
  const DERIVATIVES = {
    assetHash: "b".repeat(64),
    derivatives: [
      {
        derivativeId: "d-1",
        parentHash: "b".repeat(64),
        kind: "ocr-text",
        status: "failed",
        processor: "tesseract",
        processorVersion: "5.3.0",
        detail: "tesseract exited with code 1",
        source: { segment: "region=0,0,10,10" },
      },
    ],
  };

  function stubAssets() {
    vi.spyOn(api, "memoryList").mockResolvedValue({ items: [] });
    vi.spyOn(api, "assetsList").mockResolvedValue({ assets: [ASSET], count: 1 });
    vi.spyOn(api, "assetImpact").mockResolvedValue(IMPACT);
    vi.spyOn(api, "assetDerivatives").mockResolvedValue(DERIVATIVES);
  }

  it("assetImpactSummary 只講數量與後果，不倒 id", () => {
    const summary = assetImpactSummary(IMPACT);
    expect(summary).toContain("會影響 1 條已採用的知識");
    expect(summary).toContain("會一併移除 2 筆衍生資料");
    expect(summary).not.toContain("kn-");
    expect(summary).not.toContain("disputed");
    expect(assetImpactSummary({})).toBe("沒有知識引用這筆素材；沒有衍生資料要移除。");
  });

  it("一般模式：人話標題、人話刪除影響、人話衍生狀態；沒有原始 JSON、狀態碼、processor", async () => {
    stubAssets();
    primeCharacterNameForTests({ name: "小樞", pronoun: "她", characterId: "shu-maid" });
    const { container } = render(<MemoryKnowledgePage refreshKey={0} advanced={false} />);
    await userEvent.click(await screen.findByRole("tab", { name: "素材與來源" }));
    await screen.findByText("receipt.png");
    await userEvent.click(screen.getByRole("button", { name: "刪除影響預覽" }));
    await screen.findByTestId("asset-impact-preview");
    await userEvent.click(screen.getByRole("button", { name: "查看衍生資料" }));
    await screen.findByTestId("asset-derivative-viewer");
    const text = container.textContent ?? "";
    expect(text).toContain("會影響 1 條已採用的知識");
    expect(text).toContain("圖片文字辨識");
    expect(text).toContain("解析失敗");
    expect(text).toContain("你加入的");
    for (const leak of [
      "referencingKnowledgeNodes",
      "kn-",
      "disputed",
      "failed",
      "內容定址",
      "tesseract",
      "user-import",
      "ocr-text",
      "狀態碼",
    ]) {
      expect(text, `一般模式不得出現「${leak}」`).not.toContain(leak);
    }
    expect(container.querySelector("[data-testid=asset-impact-preview] pre")).toBeNull();
  });

  it("進階模式：原始 JSON、狀態碼與 processor 仍在（零能力退化）", async () => {
    stubAssets();
    primeCharacterNameForTests({ name: "小樞", pronoun: "她", characterId: "shu-maid" });
    const { container } = render(<MemoryKnowledgePage refreshKey={0} advanced />);
    await userEvent.click(await screen.findByRole("tab", { name: "素材與來源" }));
    await screen.findByText("receipt.png");
    await userEvent.click(screen.getByRole("button", { name: "刪除影響預覽" }));
    await screen.findByTestId("asset-impact-preview");
    await userEvent.click(screen.getByRole("button", { name: "查看衍生資料" }));
    await screen.findByTestId("asset-derivative-viewer");
    const text = container.textContent ?? "";
    expect(text).toContain("referencingKnowledgeNodes");
    expect(text).toContain("狀態碼 failed");
    expect(text).toContain("tesseract 5.3.0");
    expect(text).toContain("內容定址");
    expect(container.querySelector("[data-testid=asset-impact-preview] pre")).not.toBeNull();
  });
});

// ---------------------------------------------------------------------------
// agent-honesty-028／031：AiPage
// ---------------------------------------------------------------------------

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

function renderAiPage(record: Partial<AgentSessionRecord> & Record<string, unknown>) {
  vi.spyOn(api, "agentsDiscoveries").mockResolvedValue({ agents: [] });
  vi.spyOn(api, "agentSessionsList").mockResolvedValue([{ ...SESSION, ...record } as AgentSessionRecord]);
  vi.spyOn(api, "agentSessionMessages").mockResolvedValue([]);
  return render(
    <AppStateProvider ready={false} refreshKey={0}>
      <AiPage refreshKey={0} advanced={false} onNavigate={() => {}} />
    </AppStateProvider>
  );
}

describe("AiPage：「進行中」與 Rust is_open 對齊", () => {
  it("isOpenWorkState：failed／unknown／timed-out／closed 不算進行中；created／active／waiting／claimed 算", () => {
    for (const s of ["failed", "unknown", "timed-out", "closed", "cancelled", "expired", "bogus"]) {
      expect(isOpenWorkState(s), s).toBe(false);
    }
    for (const s of ["created", "active", "waiting-for-input", "waiting-for-consent", "claimed-completed"]) {
      expect(isOpenWorkState(s), s).toBe(true);
    }
  });

  it("state=unknown＋providerSessionId：沒有續租／中斷／再交代，有「接續上次（唯讀）」與「關閉」", async () => {
    renderAiPage({ state: "unknown", providerSessionId: "9d3f1c00-aaaa-bbbb-cccc-ddddeeeeffff" });
    await screen.findByText("整理測試報告");
    expect(screen.queryByRole("button", { name: "續租 30 分鐘" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "暫停／中斷目前工作" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "接續上次（唯讀）" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "關閉" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "查看結果／訊息" }));
    expect(screen.queryByPlaceholderText("再交代一句給這個 Agent…")).not.toBeInTheDocument();
  });

  it("state=failed 同樣是終局：沒有續租／中斷，可關閉，可接續", async () => {
    renderAiPage({ state: "failed", providerSessionId: "9d3f1c00-aaaa-bbbb-cccc-ddddeeeeffff" });
    await screen.findByText("整理測試報告");
    expect(screen.queryByRole("button", { name: "續租 30 分鐘" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "暫停／中斷目前工作" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "接續上次（唯讀）" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "關閉" })).toBeInTheDocument();
  });

  it("state=active：有續租／中斷／關閉／再交代，沒有「接續上次」", async () => {
    renderAiPage({ state: "active", providerSessionId: "9d3f1c00-aaaa-bbbb-cccc-ddddeeeeffff" });
    await screen.findByText("整理測試報告");
    expect(screen.getByRole("button", { name: "續租 30 分鐘" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "暫停／中斷目前工作" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "關閉" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "接續上次（唯讀）" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "查看結果／訊息" }));
    expect(screen.getByPlaceholderText("再交代一句給這個 Agent…")).toBeInTheDocument();
  });

  it("state=closed：只有「接續上次」，沒有關閉", async () => {
    renderAiPage({ state: "closed", providerSessionId: "9d3f1c00-aaaa-bbbb-cccc-ddddeeeeffff" });
    await screen.findByText("整理測試報告");
    expect(screen.getByRole("button", { name: "接續上次（唯讀）" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "關閉" })).not.toBeInTheDocument();
  });
});

describe("AiPage：綠勾只給目前這一輪的 claim", () => {
  const VERIFIED = { at: "2026-01-01T00:30:00Z", note: "看過了" };

  it("verifiedForCurrentClaim：state 不是 claimed-completed 一律 false；claim id 不同 false；沒有 id 退回 humanVerified", () => {
    expect(verifiedForCurrentClaim({ ...SESSION, state: "active", humanVerified: VERIFIED })).toBe(false);
    expect(verifiedForCurrentClaim({ ...SESSION, state: "claimed-completed" })).toBe(false);
    expect(verifiedForCurrentClaim({ ...SESSION, state: "claimed-completed", humanVerified: VERIFIED })).toBe(true);
    const scoped = {
      ...SESSION,
      state: "claimed-completed",
      humanVerified: VERIFIED,
      claimId: "claim-2",
      humanVerifiedClaimId: "claim-1",
    } as AgentSessionRecord;
    expect(verifiedForCurrentClaim(scoped)).toBe(false);
    expect(verifiedForCurrentClaim({ ...scoped, humanVerifiedClaimId: "claim-2" } as AgentSessionRecord)).toBe(true);
    expect(
      verifiedForCurrentClaim({
        ...SESSION,
        state: "claimed-completed",
        humanVerified: { ...VERIFIED, claimId: "claim-1" },
        claimId: "claim-2",
      } as AgentSessionRecord)
    ).toBe(false);
  });

  it("第二輪進行中（state=active）帶著舊的 humanVerified：不顯示「✓ 已由你確認」", async () => {
    renderAiPage({ state: "active", humanVerified: VERIFIED });
    await screen.findByText("整理測試報告");
    expect(screen.queryByText("✓ 已由你確認")).not.toBeInTheDocument();
    expect(screen.getByText("處理中")).toBeInTheDocument();
    expect(screen.queryByText(/由你親自確認/)).not.toBeInTheDocument();
    expect(screen.getByText(/先前一輪的結果你在/)).toBeInTheDocument();
  });

  it("第二輪 claimed-completed 但驗證對應的是上一輪 claim：不顯示綠勾，驗證按鈕仍在", async () => {
    renderAiPage({
      state: "claimed-completed",
      humanVerified: VERIFIED,
      claimId: "claim-2",
      humanVerifiedClaimId: "claim-1",
    });
    await screen.findByText("整理測試報告");
    expect(screen.queryByText("✓ 已由你確認")).not.toBeInTheDocument();
    expect(screen.getByText("對方說已完成")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "標記為已驗證（我確認過結果）" })).toBeInTheDocument();
  });

  it("目前這一輪的 claim 已由人確認：綠勾＋「由你親自確認」（舊後端沒有 claim id 也成立）", async () => {
    renderAiPage({ state: "claimed-completed", humanVerified: VERIFIED });
    await screen.findByText("整理測試報告");
    expect(screen.getByText("✓ 已由你確認")).toBeInTheDocument();
    expect(screen.getByText(/由你親自確認/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "標記為已驗證（我確認過結果）" })).not.toBeInTheDocument();
  });
});

// ---------------------------------------------------------------------------
// ia-settings-039：工作頁分工摘要
// ---------------------------------------------------------------------------

describe("工作頁：「目前分工」不冒充精靈的選擇", () => {
  it("agentRouteSummary：四角色都 none → 全部不交給 Agent；沒有路由 → 尚未設定（使用預設分工）", () => {
    expect(
      agentRouteSummary({ conversation: "none", programming: "none", knowledge: "none", review: "none" })
    ).toBe("全部不交給 Agent");
    expect(agentRouteSummary({ conversation: "none", programming: "none" })).not.toContain("交給 不交給");
    expect(agentRouteSummary(undefined)).toBe("尚未設定（使用預設分工）");
    expect(agentRouteSummary({})).toBe("尚未設定（使用預設分工）");
  });

  it("精靈選「稍後再說」後的真實偏好（後端預設四筆路由）：顯示「目前分工：…」，不出現「精靈選擇」", async () => {
    vi.spyOn(api, "uiPrefsGet").mockResolvedValue({
      mode: "simple",
      locale: "zh-TW",
      customNames: {},
      schemaVersion: "1.0",
      agentRoutes: {
        conversation: "claude-code",
        programming: "codex",
        knowledge: "claude-code",
        review: "claude-code",
      },
    });
    vi.spyOn(api, "pauseGet").mockResolvedValue({ paused: false });
    vi.spyOn(api, "agentsDiscoveries").mockResolvedValue({ agents: [] });
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([]);
    vi.spyOn(api, "recipesList").mockResolvedValue([]);
    const { container } = render(
      <AppStateProvider ready refreshKey={0}>
        <WorkPage refreshKey={0} advanced={false} onNavigate={() => {}} />
      </AppStateProvider>
    );
    await waitFor(() =>
      expect(screen.getByText(/目前分工：程式工作交給 Codex；一般對話、知識整理與結果複審交給 Claude Code/)).toBeInTheDocument()
    );
    expect(container.textContent).not.toContain("精靈選擇");
    expect(within(container as HTMLElement).getByRole("button", { name: "調整分工" })).toBeInTheDocument();
  });
});
