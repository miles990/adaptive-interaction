// v0.5 Phase 1 對抗審查（獨立稽核＋主 session grep 確認）缺陷的 regression tests：
// 精靈步驟二不再是空殼、音效／安靜時段文案與行為一致、Inbox 待決定計數、
// 淺色主題可讀、通知面板鍵盤可用、風險分級 L0–L4、一般模式不外洩治理術語、
// §11 記憶與知識 UI 分層、AiPage 訊息輪詢，以及 IA 守門測試。

import fs from "node:fs";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { api, AgentSessionRecord, HumanCard } from "../api";
import { AppStateProvider } from "../appstate";
import {
  inboxStatusLabel,
  LEGACY_ANCHORS,
  NARROW_MORE_ITEMS,
  navAnchorFor,
  NotificationPanel,
  PageBody,
  SIMPLE_NAV,
  simpleNavFor,
  titleFor,
} from "../App";
import APP_SOURCE from "../App.tsx?raw";
import GLOBAL_SEARCH_SOURCE from "../components/GlobalSearch.tsx?raw";
import { characterNameFallback, primeCharacterNameForTests, resetCharacterNameForTests } from "../characterName";
import { HomePage } from "../pages/HomePage";
import { MorePage, MORE_TABS } from "../pages/MorePage";
import { SettingsPage } from "../pages/SettingsPage";
import { CapabilityCard } from "../components/CapabilityCard";
import { AiPage, ApprovalCountdown, messageSummary } from "../pages/AiPage";
import { dataScopeLabel, toolScopeLabel } from "../pages/HomePage";
import { MemoryKnowledgePage } from "../pages/MemoryKnowledgePage";
import { WorkPage, agentRouteSummary } from "../pages/WorkPage";
import { RISK_TIERS, riskTierOf, riskTierOfCard } from "../riskTier";
const PAGE_SOURCES = import.meta.glob("../pages/*.tsx", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;
/** 角色頁的子模組（一般模式看得到的卡片、預覽、匯入對話框）。 */
const CHARACTER_PAGE_SOURCES = import.meta.glob("../pages/character/*.{ts,tsx}", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

const SESSION: AgentSessionRecord = {
  sessionId: "sess-9d3f1c00-1111-2222-3333-444455556666",
  providerId: "p-1",
  agentId: "claude-code",
  label: "整理測試報告",
  state: "active",
  providerSessionId: "9d3f1c00-aaaa-bbbb-cccc-ddddeeeeffff",
  lease: { issuedAt: "2026-01-01T00:00:00Z", expiresAt: "2026-01-01T01:00:00Z", renewable: true },
  dataScope: ["workspace:/tmp/repo", "domain:rust"],
  toolScope: ["workspace.write"],
  consentScope: [],
  budget: { maxMessages: 10, spentMessages: 1, maxCost: 0, spentCost: 0 },
  createdAt: "2026-01-01T00:00:00Z",
};

function card(overrides: Partial<HumanCard>): HumanCard {
  return {
    id: "test.cap",
    kind: "actuator",
    displayName: "測試能力",
    nameSource: "catalog",
    shortDescription: "一句說明。",
    descriptionSource: "catalog",
    icon: "bell",
    colorRole: "output",
    category: "notification",
    beginnerRecommended: false,
    badges: [],
    consent: { required: false },
    undescribed: false,
    availability: "available",
    requiresConsent: false,
    manifestHash: "0123456789abcdef",
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// 5. 淺色主題不可讀（未定義 --panel、硬編輸入框底色）
// ---------------------------------------------------------------------------

describe("主題變數（淺色主題必須可讀）", () => {
  // vitest 會把 CSS 的 ?raw 匯入清空，這裡直接讀檔（root = apps/interaction-desktop）。
  const css = fs.readFileSync(path.resolve("src/styles.css"), "utf8");

  it("輸入框不再硬編深色底，改用主題變數", () => {
    const block = css.slice(css.indexOf("input, select, textarea {"));
    const rule = block.slice(0, block.indexOf("}"));
    expect(rule).not.toContain("#10141a");
    expect(rule).toContain("var(--input-bg)");
    expect(rule).toContain("var(--input-text)");
  });

  it("--panel／--input-bg／--input-text 在深色與淺色兩套主題都有定義", () => {
    for (const token of ["--panel", "--input-bg", "--input-text"]) {
      const defined = css.split(`${token}:`).length - 1;
      expect(defined, `${token} 必須三個主題區塊都定義`).toBeGreaterThanOrEqual(3);
    }
    expect(css).not.toContain("var(--panel,");
  });
});

// ---------------------------------------------------------------------------
// 6. 通知面板鍵盤支援
// ---------------------------------------------------------------------------

describe("通知中心（鍵盤可用，不是只能點的浮層）", () => {
  const inbox = {
    pendingCount: 1,
    items: [
      {
        kind: "agent-session",
        itemId: "s-1",
        status: "waiting-for-consent",
        title: "等你核可",
        route: "ai",
        needsDecision: true,
      },
    ],
  };

  it("宣告為 modal 並在開啟時把焦點移進面板", () => {
    render(<NotificationPanel inbox={inbox} onClose={() => {}} onNavigate={() => {}} />);
    const panel = screen.getByRole("dialog", { name: "通知中心" });
    expect(panel).toHaveAttribute("aria-modal", "true");
    expect(document.activeElement).toBe(panel);
  });

  it("Escape 關閉，並把焦點還給原本的觸發元素", async () => {
    const trigger = document.createElement("button");
    document.body.appendChild(trigger);
    trigger.focus();
    const onClose = vi.fn();
    const { unmount } = render(
      <NotificationPanel inbox={inbox} onClose={onClose} onNavigate={() => {}} />
    );
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(1);
    unmount();
    expect(document.activeElement).toBe(trigger);
    trigger.remove();
  });

  it("Tab 在面板內循環，不會掉到面板後面的頁面", () => {
    render(<NotificationPanel inbox={inbox} onClose={() => {}} onNavigate={() => {}} />);
    const panel = screen.getByRole("dialog", { name: "通知中心" });
    const buttons = within(panel).getAllByRole("button");
    const last = buttons[buttons.length - 1];
    last.focus();
    fireEvent.keyDown(panel, { key: "Tab" });
    expect(document.activeElement).toBe(buttons[0]);
    fireEvent.keyDown(panel, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(last);
  });

  it("狀態顯示人話，不是原始 enum 字串", () => {
    render(<NotificationPanel inbox={inbox} onClose={() => {}} onNavigate={() => {}} />);
    // 文案以共用狀態投影（statusProjection.ts）的 spec 表格為準。
    expect(screen.getByText("等你允許")).toBeInTheDocument();
    expect(screen.queryByText("waiting-for-consent")).not.toBeInTheDocument();
    // 介面不認得的狀態：不猜、也不把原始字串當標籤——投影成「結果不確定」。
    expect(inboxStatusLabel("some-new-state")).toBe("結果不確定");
    expect(inboxStatusLabel("some-new-state")).not.toBe("some-new-state");
  });
});

// ---------------------------------------------------------------------------
// 8. 風險分級 L0–L4（spec §2）
// ---------------------------------------------------------------------------

describe("風險分級 L0–L4", () => {
  it("L0：純角色呈現（desktop-pet）預設開啟、不逐次詢問", () => {
    const tier = riskTierOf({ channel: "desktop-pet", external: false, physical: false });
    expect(tier.tier).toBe(0);
    expect(tier.policy).toContain("不會每次問你");
  });

  it("L1：本機通知／短音效一次設定即可", () => {
    expect(riskTierOf({ channel: "notification", external: false, physical: false }).tier).toBe(1);
    expect(riskTierOf({ channel: "audio", external: false, physical: false }).tier).toBe(1);
  });

  it("L2：個人資料／檔案／記憶第一次或範圍改變時詢問", () => {
    expect(riskTierOf({ id: "memory.read", channel: "log" }).tier).toBe(2);
    expect(
      riskTierOf({ channel: "conversation", personalData: true, external: false, physical: false })
        .tier
    ).toBe(2);
  });

  it("L3：外部或實體效果需要明確授權，且有硬限制摘要", () => {
    const light = riskTierOf({ channel: "light", physical: true });
    expect(light.tier).toBe(3);
    expect(light.hardLimits).toContain("強度");
    expect(riskTierOf({ channel: "webhook", external: true }).tier).toBe(3);
    expect(riskTierOf({ channel: "conversation", external: "unknown" }).tier).toBe(3);
  });

  it("L4：攝影機／持續麥克風／定位／Agent 寫入每次或短效授權", () => {
    expect(riskTierOf({ id: "camera.main", sensitivity: "high" }).tier).toBe(4);
    expect(riskTierOf({ id: "mic.listen", channel: "audio" }).tier).toBe(4);
    expect(riskTierOf({ id: "location.coarse" }).tier).toBe(4);
    expect(riskTierOf({ id: "workspace.write" }).tier).toBe(4);
  });

  it("能力卡片顯示分級標籤與一句人話；L3 另外顯示硬限制", () => {
    render(
      <CapabilityCard
        card={card({
          displayName: "房間燈光",
          channel: "light",
          effect: {
            externalSideEffect: false,
            physicalEffect: true,
            interruptiveness: "medium",
            reversible: true,
            confirmationLevel: "acknowledged",
          },
        })}
        advanced={false}
        onChanged={() => {}}
      />
    );
    expect(screen.getAllByText("L3 外部或實體效果").length).toBeGreaterThan(0);
    expect(screen.getByText(/強度、持續時間與頻率/)).toBeInTheDocument();
  });

  it("L0–L4 全表可供「同意與安全」說明使用", () => {
    expect(RISK_TIERS.map((t) => t.tier)).toEqual([0, 1, 2, 3, 4]);
    expect(RISK_TIERS.every((t) => t.policy.length > 0)).toBe(true);
  });

  it("由 human card 推導：需同意的能力至少是 L2", () => {
    const tier = riskTierOfCard(
      card({ id: "unknown.cap", requiresConsent: true, consent: { required: true } })
    );
    expect(tier.tier).toBeGreaterThanOrEqual(2);
  });
});

// ---------------------------------------------------------------------------
// 9. 一般模式術語外洩
// ---------------------------------------------------------------------------

describe("一般模式不外洩治理術語", () => {
  it("AiPage 工作階段卡片不出現 Lease／provider session／原始 uuid", async () => {
    vi.spyOn(api, "agentsDiscoveries").mockResolvedValue({ agents: [] });
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([SESSION]);
    const { container } = render(
      <AppStateProvider ready={false} refreshKey={0}>
        <AiPage refreshKey={0} onNavigate={() => {}} />
      </AppStateProvider>
    );
    await screen.findByText("整理測試報告");
    const text = container.textContent ?? "";
    expect(text).not.toContain("Lease");
    expect(text).not.toContain("provider session");
    expect(text).not.toContain(SESSION.providerSessionId!.slice(0, 8));
    expect(text).toContain("有效至");
    expect(text).toContain("沿用既有對話脈絡");
  });

  it("進階模式才顯示 provider session 技術識別碼", async () => {
    vi.spyOn(api, "agentsDiscoveries").mockResolvedValue({ agents: [] });
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([SESSION]);
    const { container } = render(
      <AppStateProvider ready={false} refreshKey={0}>
        <AiPage refreshKey={0} advanced onNavigate={() => {}} />
      </AppStateProvider>
    );
    await screen.findByText("整理測試報告");
    expect(container.textContent).toContain("provider session");
  });

  it("HomePage 的資料與工具範圍是人話，不是原始 scope 字串", () => {
    expect(dataScopeLabel("workspace:/tmp/repo")).toBe("資料夾 /tmp/repo");
    expect(dataScopeLabel("domain:rust")).toBe("知識領域「rust」");
    expect(toolScopeLabel("workspace.write")).toBe("可以修改這個資料夾裡的檔案");
    expect(toolScopeLabel("some.future.scope")).toBe("some.future.scope");
  });

  it("Agent 訊息以人話標題與摘要呈現，原始 JSON 只在進階模式的技術詳情", async () => {
    vi.spyOn(api, "agentsDiscoveries").mockResolvedValue({ agents: [] });
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([SESSION]);
    vi.spyOn(api, "agentSessionMessages").mockResolvedValue([
      {
        messageId: "m-1",
        kind: "approval-request",
        createdAt: new Date().toISOString(),
        body: { requestId: "r-1", summary: "要寫入 src/main.rs" },
      },
    ]);
    const simple = render(
      <AppStateProvider ready={false} refreshKey={0}>
        <AiPage refreshKey={0} onNavigate={() => {}} />
      </AppStateProvider>
    );
    await userEvent.click(await screen.findByRole("button", { name: "查看結果／訊息" }));
    expect(await screen.findByText("等待你核可")).toBeInTheDocument();
    expect(screen.getByText("要寫入 src/main.rs")).toBeInTheDocument();
    expect(screen.queryByText("approval-request")).not.toBeInTheDocument();
    // Phase 3：一般模式連「技術詳情」與原始 JSON 都不出現（brief I(3)：JSON 只在進階）。
    expect(screen.queryByText("技術詳情")).not.toBeInTheDocument();
    expect(simple.container.textContent).not.toContain("requestId");
    simple.unmount();

    const advanced = render(
      <AppStateProvider ready={false} refreshKey={0}>
        <AiPage refreshKey={0} advanced onNavigate={() => {}} />
      </AppStateProvider>
    );
    await userEvent.click(await screen.findByRole("button", { name: "查看結果／訊息" }));
    expect(await screen.findByText("等待你核可")).toBeInTheDocument();
    expect(screen.getByText("技術詳情")).toBeInTheDocument();
    expect(advanced.container.textContent).toContain("requestId");
  });

  it("摘要優先取 summary，沒有文字時誠實說沒有", () => {
    expect(messageSummary({ summary: "做完了" })).toBe("做完了");
    expect(messageSummary({ tool: { name: "grep", phase: "started" } })).toBe(
      "使用工具 grep（started）"
    );
    expect(messageSummary({})).toBe("（沒有文字內容）");
  });
});

// ---------------------------------------------------------------------------
// 11. AiPage 訊息輪詢與核可剩餘時間
// ---------------------------------------------------------------------------

describe("AiPage 訊息不再只抓一次", () => {
  it("展開後每 5 秒重新抓一次，收合即停止", async () => {
    vi.useFakeTimers();
    vi.spyOn(api, "agentsDiscoveries").mockResolvedValue({ agents: [] });
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([SESSION]);
    const messages = vi.spyOn(api, "agentSessionMessages").mockResolvedValue([]);
    render(
      <AppStateProvider ready={false} refreshKey={0}>
        <AiPage refreshKey={0} onNavigate={() => {}} />
      </AppStateProvider>
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    fireEvent.click(screen.getByRole("button", { name: "查看結果／訊息" }));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(messages).toHaveBeenCalledTimes(1);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5000);
    });
    expect(messages).toHaveBeenCalledTimes(2);
    fireEvent.click(screen.getByRole("button", { name: "收合" }));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(15_000);
    });
    expect(messages).toHaveBeenCalledTimes(2);
  });

  it("核可請求顯示剩餘決定時間並真的倒數", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-01-01T00:00:00Z"));
    const { container } = render(<ApprovalCountdown createdAt="2026-01-01T00:00:00Z" />);
    expect(container.textContent).toContain("還有 300 秒");
    act(() => {
      vi.advanceTimersByTime(10_000);
    });
    expect(container.textContent).toContain("還有 290 秒");
    act(() => {
      vi.advanceTimersByTime(400_000);
    });
    expect(container.textContent).toContain("已超過決定時間");
  });
});

// ---------------------------------------------------------------------------
// 10. §11 記憶與知識 UI 分層
// ---------------------------------------------------------------------------

describe("記憶與知識分層（一般模式只有三區）", () => {
  // 角色名稱走 useCharacterName（memory-ui-003）：這裡釘成「小樞」，斷言文案不變；
  // 不寫死的證明在 regressions-v05-round3（阿樂）與下方的原始碼掃描。
  beforeEach(() => {
    primeCharacterNameForTests({ name: "小樞", pronoun: "她", characterId: "shu-maid" });
  });
  afterEach(() => {
    resetCharacterNameForTests();
  });
  function stubMemoryApis() {
    vi.spyOn(api, "memoryList").mockResolvedValue({ items: [] });
    vi.spyOn(api, "knowledgeList").mockResolvedValue({ nodes: [], count: 0 });
    vi.spyOn(api, "assetsList").mockResolvedValue({ assets: [], count: 0 });
    vi.spyOn(api, "knowledgeReceipts").mockResolvedValue({ receipts: [] });
    vi.spyOn(api, "domainPacks").mockResolvedValue({ packs: [] });
  }

  it("advanced=false：只有三個分頁，且不出現候選／收據／Context Bundle 字樣", async () => {
    stubMemoryApis();
    const { container } = render(<MemoryKnowledgePage refreshKey={0} advanced={false} />);
    const tabs = await screen.findAllByRole("tab");
    expect(tabs.map((t) => t.textContent)).toEqual([
      "關於我的記憶",
      "小樞學會的知識",
      "素材與來源",
    ]);
    for (const name of ["小樞學會的知識", "素材與來源", "關於我的記憶"]) {
      await userEvent.click(screen.getByRole("tab", { name }));
      const text = container.textContent ?? "";
      expect(text, `${name} 不得出現「候選」`).not.toContain("候選");
      expect(text, `${name} 不得出現「收據」`).not.toContain("收據");
      expect(text, `${name} 不得出現 Context Bundle`).not.toContain("Context Bundle");
    }
  });

  it("advanced=false：知識狀態使用規格指定的人話文案", async () => {
    stubMemoryApis();
    render(<MemoryKnowledgePage refreshKey={0} advanced={false} />);
    await userEvent.click(await screen.findByRole("tab", { name: "小樞學會的知識" }));
    const options = within(screen.getByRole("combobox")).getAllByRole("option");
    expect(options.map((o) => o.textContent)).toEqual([
      "等待確認",
      "已採用",
      "可能過期",
      "有不同說法",
      "已被新版取代",
      "已封存",
      "全部",
    ]);
  });

  it("advanced=false：關於我的記憶指路小樞頁，並顯示本次會提供給 AI 的內容（明說不含工作階段授權的領域知識）", async () => {
    stubMemoryApis();
    const navigate = vi.fn();
    const { container } = render(
      <MemoryKnowledgePage refreshKey={0} advanced={false} onNavigate={navigate} />
    );
    expect(await screen.findByText(/小樞跟你玩耍、互動累積的角色記憶/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "前往小樞" }));
    expect(navigate).toHaveBeenCalledWith("companion");
    expect(screen.getByText("本次會提供給 AI 的內容")).toBeInTheDocument();
    // memory-ui-006：預覽固定以 domains=[] 呼叫，和真實工作階段的 bundle 不同——文案必須明說。
    expect(container.textContent).toContain("不含工作階段授權的領域知識");
    // memory-ui-005：只宣稱真的會記的類別。
    expect(container.textContent).not.toContain("相處距離");
  });

  it("advanced=true：完整五個分頁與技術面板都還在（零能力退化）", async () => {
    stubMemoryApis();
    render(<MemoryKnowledgePage refreshKey={0} advanced />);
    const tabs = await screen.findAllByRole("tab");
    expect(tabs.map((t) => t.textContent)).toEqual([
      "關於我的記憶",
      "小樞學會的知識",
      "素材與來源",
      "知識收據",
      "Context Bundle 預覽",
    ]);
    await userEvent.click(screen.getByRole("tab", { name: "小樞學會的知識" }));
    expect(await screen.findByText("知識何時更新、何時需要 AI")).toBeInTheDocument();
  });
});

// ---------------------------------------------------------------------------
// 1. 精靈步驟二的選擇在工作頁看得到
// ---------------------------------------------------------------------------

describe("WorkPage 顯示精靈的 agent 選擇", () => {
  it("由實際路由偏好產生人話摘要", () => {
    expect(
      agentRouteSummary({
        conversation: "codex",
        programming: "codex",
        knowledge: "codex",
        review: "codex",
      })
    ).toBe("全部交給 Codex");
    expect(
      agentRouteSummary({
        conversation: "claude-code",
        programming: "codex",
        knowledge: "claude-code",
        review: "claude-code",
      })
    ).toContain("程式工作交給 Codex");
    // ia-settings-039：沒有路由時不冒充「精靈選了稍後再說」，只說用預設分工。
    expect(agentRouteSummary(undefined)).toBe("尚未設定（使用預設分工）");
  });

  it("工作頁顯示「目前分工：…」摘要與前往調整的入口", async () => {
    vi.spyOn(api, "uiPrefsGet").mockResolvedValue({
      mode: "simple",
      locale: "zh-TW",
      customNames: {},
      schemaVersion: "1.0",
      agentRoutes: {
        conversation: "codex",
        programming: "codex",
        knowledge: "codex",
        review: "codex",
      },
    });
    vi.spyOn(api, "pauseGet").mockResolvedValue({ paused: false });
    vi.spyOn(api, "agentsDiscoveries").mockResolvedValue({ agents: [] });
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([]);
    vi.spyOn(api, "recipesList").mockResolvedValue([]);
    render(
      <AppStateProvider ready refreshKey={0}>
        <WorkPage refreshKey={0} advanced={false} onNavigate={() => {}} />
      </AppStateProvider>
    );
    // ia-settings-039：後端預設永遠帶四筆路由，畫面只能說「目前分工」，不能說是精靈的選擇。
    await waitFor(() => expect(screen.getByText(/目前分工：全部交給 Codex/)).toBeInTheDocument());
    expect(screen.queryByText(/精靈選擇/)).not.toBeInTheDocument();
    // Phase 3 task-first：分工設定住在同一頁的「工作設定」折疊區，入口改叫「調整分工」
    //（原「前往工作頁調整」在工作頁上是自己指自己）。
    expect(screen.getByRole("button", { name: "調整分工" })).toBeInTheDocument();
  });
});

// ---------------------------------------------------------------------------
// 12. 守門測試：IA 與「每項設定只有一個主人」
// ---------------------------------------------------------------------------

describe("資訊架構守門", () => {
  it("一級入口恰好 5 個，id 固定", () => {
    expect(SIMPLE_NAV).toHaveLength(5);
    expect(SIMPLE_NAV.map((t) => t.id)).toEqual(["home", "companion", "work", "connect", "more"]);
  });

  it("舊 tab id → 新 anchor 對照表完整，且都落在 5 個一級入口內", () => {
    const expected: Record<string, string> = {
      ai: "work",
      automations: "work",
      capabilities: "connect",
      senses: "connect",
      responses: "connect",
      toolops: "connect",
      safety: "connect",
      memory: "more",
      activity: "more",
      settings: "more",
      // v0.5 一般模式「更多」的新分頁（只新增、不移除）。
      backup: "more",
      manage: "more",
      "advanced-features": "more",
    };
    expect(LEGACY_ANCHORS).toEqual(expected);
    const primary = new Set(SIMPLE_NAV.map((t) => t.id));
    for (const [legacy, anchor] of Object.entries(expected)) {
      expect(navAnchorFor(legacy)).toBe(anchor);
      expect(primary.has(anchor)).toBe(true);
    }
    for (const route of ["automations", "ai", "memory", "activity", "safety"]) {
      expect(primary.has(navAnchorFor(route))).toBe(true);
    }
  });

  it("initiative／安靜時段／主動對話只有「小樞」頁一個主人", () => {
    const patterns: [RegExp, string][] = [
      [/patch\(\s*\{\s*initiative/, "主動程度"],
      [/patch\(\s*\{\s*quietHours/, "安靜時段"],
      [/proactiveDialoguePatch\(/, "主動對話模式"],
    ];
    // 例外只有兩個，且都有理由：首次設定精靈是一次性 commit（不是常駐設定
    // 控制項），進階模式的 Policy 頁是原始 JSON 編輯器。
    const exempt = new Set(["Onboarding.tsx", "Policy.tsx"]);
    const byFile = new Map(
      Object.entries(PAGE_SOURCES).map(([key, source]) => [key.split("/").pop()!, source])
    );
    expect(byFile.size).toBeGreaterThan(5);
    const owner = byFile.get("CompanionPage.tsx");
    expect(owner, "CompanionPage.tsx 必須被掃到").toBeTruthy();
    for (const [pattern, what] of patterns) {
      expect(pattern.test(owner!), `CompanionPage 必須是「${what}」的主人`).toBe(true);
    }
    for (const [file, source] of byFile) {
      if (file === "CompanionPage.tsx" || exempt.has(file)) continue;
      for (const [pattern, what] of patterns) {
        expect(pattern.test(source), `${file} 不得成為「${what}」的第二個主人`).toBe(false);
      }
    }
  });
});

// ---------------------------------------------------------------------------
// 13. 一般模式產品化（G）：導覽第二項跟著角色名稱、「更多」五個入口、
//     第一屏與設定頁不外洩治理術語、程式碼裡不再寫死「小樞」
// ---------------------------------------------------------------------------

const BANNED_SIMPLE_TERMS = [
  "Runtime",
  "daemon",
  "token",
  "Token",
  "CLI",
  "HTTP",
  "Provider",
  "Adapter",
  "Manifest",
  "GATT",
  "受器",
  "動器",
  "Lease",
  "租約",
  "Receipt",
  "收據",
  "JSON",
  "YAML",
  "Agent Session",
  "UUID",
  "app-server",
  "Receptor",
  "Actuator",
];

/** 一般使用者看得到的文字：排除折疊中的 details 內容（summary 保留）。 */
function visibleText(root: HTMLElement): string {
  const clone = root.cloneNode(true) as HTMLElement;
  clone.querySelectorAll("details:not([open])").forEach((d) => {
    Array.from(d.children).forEach((c) => {
      if (c.tagName !== "SUMMARY") c.remove();
    });
  });
  return clone.textContent ?? "";
}

function expectNoLeak(text: string, where: string) {
  for (const term of BANNED_SIMPLE_TERMS) {
    expect(text, `${where} 不得出現「${term}」`).not.toContain(term);
  }
}

// 去掉行註解與區塊註解後的原始碼（只掃真正會進畫面的字串）。
function stripComments(source: string): string {
  return source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");
}

describe("一般模式產品化（G）：導覽、更多、術語", () => {
  const OFFLINE_FETCH = () =>
    vi.fn(async () => {
      throw new Error("offline");
    });

  function stubShellApis() {
    vi.stubGlobal("fetch", OFFLINE_FETCH());
    vi.spyOn(api, "status").mockResolvedValue({
      version: "0.5.0",
      schemaVersion: "0.5",
      emergencyStop: false,
      presentation: { connected: true, visible: true },
      characterProtocol: {
        version: "1.0",
        instances: 1,
        activeCharacter: { characterId: "shu-maid", displayName: { "zh-TW": "小樞", en: "Shu" } },
      },
      recipes: { loaded: 2 },
      activeSensors: [],
    });
    vi.spyOn(api, "characterManifest").mockResolvedValue({
      characterId: "shu-maid",
      displayName: { "zh-TW": "小樞", en: "Shu" },
      pronouns: { "zh-TW": "她", en: "she" },
    } as never);
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([SESSION]);
    vi.spyOn(api, "activityInbox").mockResolvedValue({ items: [], count: 0, totalBeforeLimit: 0, pendingCount: 0 });
    vi.spyOn(api, "actionsList").mockResolvedValue([]);
    vi.spyOn(api, "sessionGet").mockResolvedValue(null);
    vi.spyOn(api, "providersList").mockResolvedValue([]);
    vi.spyOn(api, "knowledgeReceipts").mockResolvedValue({ receipts: [] });
    vi.spyOn(api, "recipesList").mockResolvedValue([]);
  }

  it("導覽第二項是目前角色的名字（載入成功＝小樞；載入失敗＝角色），其餘四項不變", () => {
    resetCharacterNameForTests();
    const loaded = simpleNavFor({ name: "小樞", icon: "sparkles" });
    expect(loaded.map((t) => t.label)).toEqual(["現在", "小樞", "工作", "連接與權限", "更多"]);
    const failed = simpleNavFor({ name: characterNameFallback, icon: "sparkles" });
    expect(failed.map((t) => t.label)).toEqual(["現在", "角色", "工作", "連接與權限", "更多"]);
    expect(titleFor("companion", "小樞")).toBe("小樞");
    expect(titleFor("companion", characterNameFallback)).toBe("角色");
  });

  it("「更多」有五個入口：記憶與資料／活動紀錄／外觀與語言／備份與還原／進階模式；窄視窗更多選單一致", () => {
    expect(MORE_TABS.map(([, label]) => label)).toEqual([
      "記憶與資料",
      "活動紀錄",
      "外觀與語言",
      "備份與還原",
      "進階模式",
    ]);
    // 窄視窗選單與寬視窗分頁是同一組 id／文案（順序也一樣）；`manage` 只是隱藏的相容路由。
    expect(NARROW_MORE_ITEMS.map((t) => t.id)).toEqual([
      "memory",
      "activity",
      "settings",
      "backup",
      "advanced-features",
    ]);
    expect(NARROW_MORE_ITEMS.map((t) => t.label)).toEqual(MORE_TABS.map(([, label]) => label));
    expect(NARROW_MORE_ITEMS.some((t) => t.id === "manage")).toBe(false);
    for (const item of NARROW_MORE_ITEMS) expect(navAnchorFor(item.id)).toBe("more");
    // 相容路由仍然到得了「更多」。
    expect(navAnchorFor("manage")).toBe("more");
  });

  it("backup／manage／advanced-features 路由落在 MorePage 對應分頁；進階模式是顯示模式唯一的主人", async () => {
    stubShellApis();
    vi.spyOn(api, "uiPrefsGet").mockResolvedValue({ mode: "simple", locale: "zh-TW", customNames: {}, schemaVersion: "1.0" });
    vi.spyOn(api, "pauseGet").mockResolvedValue({ paused: false });
    const body = (tab: string) => (
      <AppStateProvider ready={false} refreshKey={0}>
        <PageBody tab={tab} refreshKey={0} events={[]} advanced={false} onNavigate={() => {}} onRerunOnboarding={() => {}} />
      </AppStateProvider>
    );
    // manage 是隱藏的相容路由：內容仍在，但不再有分頁按鈕。
    const { rerender } = render(body("manage"));
    expect(screen.queryByRole("tab", { name: "角色與整合管理" })).not.toBeInTheDocument();
    expect(await screen.findByRole("button", { name: /管理角色/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /管理裝置與整合/ })).toBeInTheDocument();
    rerender(body("backup"));
    expect(screen.getByRole("tab", { name: "備份與還原" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("button", { name: "匯出記憶" })).toBeInTheDocument();
    rerender(body("advanced-features"));
    expect(screen.getByRole("tab", { name: "進階模式" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("checkbox", { name: "顯示進階功能" })).toBeInTheDocument();
    // 一般模式下第二層（版本與技術入口）完全不渲染。
    expect(screen.queryByText(/Runtime 0\.5\.0/)).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Provider 診斷" })).not.toBeInTheDocument();
    // 外觀與語言只指路，不再放第二個切換、也不再有版本區。
    rerender(body("settings"));
    expect(screen.queryByRole("checkbox", { name: "顯示進階功能" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "前往進階模式" })).toBeInTheDocument();
  });

  it("進階模式開啟後，第二層才出現版本、診斷與開發者工具", async () => {
    stubShellApis();
    vi.spyOn(api, "uiPrefsGet").mockResolvedValue({ mode: "advanced", locale: "zh-TW", customNames: {}, schemaVersion: "1.0" });
    vi.spyOn(api, "pauseGet").mockResolvedValue({ paused: false });
    render(
      <AppStateProvider ready={true} refreshKey={0}>
        <MorePage refreshKey={0} events={[]} advanced onNavigate={() => {}} onRerunOnboarding={() => {}} initial="advanced-features" />
      </AppStateProvider>
    );
    expect(await screen.findByText(/Runtime 0\.5\.0・Schema 0\.5/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Provider 診斷" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "配方 YAML" })).toBeInTheDocument();
    expect(screen.getByText(/api-token/)).toBeInTheDocument();
  });

  it("「現在」第一屏（含展開的詳細狀態）不外洩治理術語", async () => {
    stubShellApis();
    const { container } = render(
      <AppStateProvider ready={false} refreshKey={0}>
        <HomePage refreshKey={0} events={[]} onNavigate={() => {}} />
      </AppStateProvider>
    );
    await screen.findByText("小樞在桌面上，一切正常。");
    await screen.findByText("1 個工作階段");
    expectNoLeak(visibleText(container), "現在第一屏");
    const details = screen.getByText("詳細狀態", { selector: "summary" }).closest("details") as HTMLDetailsElement;
    details.open = true;
    fireEvent(details, new Event("toggle"));
    await screen.findByText("系統狀態");
    // 首頁只留一行摘要：完整清單、權限範圍、期限與取消的主人是工作頁。
    await screen.findByText(/目前有 1 件交代中的工作/);
    expectNoLeak(container.textContent ?? "", "現在詳細狀態");
    expect(container.textContent).not.toContain("取消這個工作階段");
    expect(container.textContent).not.toContain("權限：");
  });

  it("外觀與語言（一般模式）：沒有版本區，也沒有 Runtime／Schema／自訂名稱數字", async () => {
    stubShellApis();
    const { container } = render(
      <AppStateProvider ready={false} refreshKey={0}>
        <SettingsPage onRerunOnboarding={() => {}} onNavigate={() => {}} />
      </AppStateProvider>
    );
    await screen.findByText("語言、外觀與可及性");
    expectNoLeak(visibleText(container), "外觀與語言第一層");
    // 版本與技術資訊整段搬到「更多 → 進階模式」的第二層，這一頁連折疊區都沒有。
    const all = container.textContent ?? "";
    for (const gone of ["Runtime", "Schema", "系統版本", "自訂名稱"]) {
      expect(all, `外觀與語言不得再出現「${gone}」`).not.toContain(gone);
    }
    // 角色名稱來自 hook，不寫死。
    expect(await screen.findByRole("button", { name: "前往小樞" })).toBeInTheDocument();
    expect(screen.getByText("小樞的設定")).toBeInTheDocument();
  });

  it("「更多」的備份與還原／隱藏的角色與整合管理／進階模式與通知面板不外洩治理術語", async () => {
    stubShellApis();
    const { container } = render(
      <AppStateProvider ready={false} refreshKey={0}>
        <MorePage refreshKey={0} events={[]} advanced={false} onNavigate={() => {}} onRerunOnboarding={() => {}} initial="manage" />
      </AppStateProvider>
    );
    await screen.findByText(/目前角色：/);
    expectNoLeak(container.textContent ?? "", "角色與整合管理");
    await userEvent.click(screen.getByRole("tab", { name: "備份與還原" }));
    expectNoLeak(container.textContent ?? "", "備份與還原");
    await userEvent.click(screen.getByRole("tab", { name: "進階模式" }));
    expectNoLeak(container.textContent ?? "", "進階模式");
    const panel = render(
      <NotificationPanel
        inbox={{ pendingCount: 1, items: [{ kind: "agent-session", itemId: "s", status: "claimed-completed", title: "報告", route: "ai", needsDecision: true }] }}
        onClose={() => {}}
        onNavigate={() => {}}
      />
    );
    expectNoLeak(panel.container.textContent ?? "", "通知面板");
  });

  it("G 的檔案裡不再寫死「小樞」（註解除外），名稱一律走 useCharacterName", () => {
    const byFile = new Map(
      Object.entries(PAGE_SOURCES).map(([key, source]) => [key.split("/").pop()!, source])
    );
    const mine: [string, string][] = [
      ["App.tsx", APP_SOURCE],
      ["GlobalSearch.tsx", GLOBAL_SEARCH_SOURCE],
      ...(
        [
          "HomePage.tsx",
          "MorePage.tsx",
          "SettingsPage.tsx",
          "ActivityPage.tsx",
          "MemoryKnowledgePage.tsx",
          // 角色頁與首次設定精靈：文案一律跟著目前角色的名字，不寫死參考角色。
          "CompanionPage.tsx",
          "Onboarding.tsx",
        ] as const
      ).map((f) => [f, byFile.get(f) ?? ""] as [string, string]),
      // 角色頁的子模組（卡片／預覽／匯入對話框／目錄）。
      ...Object.entries(CHARACTER_PAGE_SOURCES).map(
        ([key, source]) => [`character/${key.split("/").pop()!}`, source] as [string, string]
      ),
    ];
    expect(Object.keys(CHARACTER_PAGE_SOURCES).length, "pages/character/* 必須被掃到").toBeGreaterThan(3);
    for (const [file, source] of mine) {
      expect(source.length, `${file} 必須被掃到`).toBeGreaterThan(0);
      expect(stripComments(source), `${file} 不得寫死「小樞」`).not.toContain("小樞");
    }
    // 參考角色的 pack id、配色 id 與說話風格 id 都不得出現在角色頁與其子模組
    //（退路一律用純文字角色；選項一律由 Reference Adapter 匯出）。
    for (const [file, source] of mine) {
      if (file !== "CompanionPage.tsx" && !file.startsWith("character/")) continue;
      const code = stripComments(source);
      for (const literal of ["shu-maid", "maid-classic", "maid-dusk", "maid-sakura", "persona-shu"]) {
        expect(code, `${file} 不得寫死參考角色的「${literal}」`).not.toContain(literal);
      }
      expect(code, `${file} 不得直接 import rig 內部的表情表`).not.toContain("companion/rig/expressions");
    }
    for (const file of [
      "App.tsx",
      "GlobalSearch.tsx",
      "HomePage.tsx",
      "MorePage.tsx",
      "SettingsPage.tsx",
      "MemoryKnowledgePage.tsx",
    ]) {
      const source = mine.find(([f]) => f === file)![1];
      expect(source, `${file} 必須使用 useCharacterName`).toContain("useCharacterName");
    }
  });
});
