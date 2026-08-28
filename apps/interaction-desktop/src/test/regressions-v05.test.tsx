// v0.5 Phase 1 對抗審查（獨立稽核＋主 session grep 確認）缺陷的 regression tests：
// 精靈步驟二不再是空殼、音效／安靜時段文案與行為一致、Inbox 待決定計數、
// 淺色主題可讀、通知面板鍵盤可用、風險分級 L0–L4、一般模式不外洩治理術語、
// §11 記憶與知識 UI 分層、AiPage 訊息輪詢，以及 IA 守門測試。

import fs from "node:fs";
import path from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { api, AgentSessionRecord, HumanCard } from "../api";
import { AppStateProvider } from "../appstate";
import { inboxStatusLabel, LEGACY_ANCHORS, navAnchorFor, NotificationPanel, SIMPLE_NAV } from "../App";
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

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
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
    expect(screen.getByText("等待你的同意")).toBeInTheDocument();
    expect(screen.queryByText("waiting-for-consent")).not.toBeInTheDocument();
    expect(inboxStatusLabel("some-new-state")).toBe("some-new-state");
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

  it("Agent 訊息以人話標題與摘要呈現，原始 JSON 收在技術詳情", async () => {
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
    render(
      <AppStateProvider ready={false} refreshKey={0}>
        <AiPage refreshKey={0} onNavigate={() => {}} />
      </AppStateProvider>
    );
    await userEvent.click(await screen.findByRole("button", { name: "查看結果／訊息" }));
    expect(await screen.findByText("等待你核可")).toBeInTheDocument();
    expect(screen.getByText("要寫入 src/main.rs")).toBeInTheDocument();
    expect(screen.getByText("技術詳情")).toBeInTheDocument();
    expect(screen.queryByText("approval-request")).not.toBeInTheDocument();
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

  it("advanced=false：關於我的記憶指路小樞頁，並顯示本次會提供給 AI 的內容", async () => {
    stubMemoryApis();
    const navigate = vi.fn();
    render(<MemoryKnowledgePage refreshKey={0} advanced={false} onNavigate={navigate} />);
    expect(await screen.findByText(/小樞跟你玩耍、互動累積的角色記憶/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "前往小樞" }));
    expect(navigate).toHaveBeenCalledWith("companion");
    expect(screen.getByText("本次會提供給 AI 的內容")).toBeInTheDocument();
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
    expect(agentRouteSummary(undefined)).toBe("尚未選擇（稍後再說）");
  });

  it("工作頁顯示「精靈選擇：…」摘要與前往調整的入口", async () => {
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
    await waitFor(() => expect(screen.getByText(/精靈選擇：全部交給 Codex/)).toBeInTheDocument());
    expect(screen.getByRole("button", { name: "前往工作頁調整" })).toBeInTheDocument();
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
