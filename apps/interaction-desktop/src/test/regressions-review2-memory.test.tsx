// v0.5 對抗審查 review2（c3d1786-20260903T124638Z）memory-ui 維度確認缺陷的 regression tests：
// - memory-ui-001：控制中心「忘記這些」在角色視窗開著時沒有真的忘記——視窗手上的舊副本
//   會在下一次玩玩具時整包寫回。互動記憶必須走 live 路徑重新同步。
// - memory-ui-002：Context Bundle 撞到份量上限時靜默丟東西，一般模式卻說「擋下來的：沒有」。
// - memory-ui-003：「匯出全部」只匯出記憶、單次 1,000 條上限且靜默截斷。
// - memory-ui-004：「重新確認（再保留 90 天）」對 agent 建立的使用者記憶只延 30 天，
//   而且會把它降級成「等待確認」（從此不再提供給 AI）。
// - memory-ui-005：「清除短期記憶」「不採用」「素材」等按鈕沒有 try/catch，
//   後端刻意寫的誠實失敗訊息被吞掉（專案沒有全域 unhandledrejection／ErrorBoundary）。
// - memory-ui-006：一般模式暴露完整 10 層記憶 taxonomy，並把所有分層混列在
//   「關於我的記憶」標題下。

import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { api } from "../api";
import { primeCharacterNameForTests, resetCharacterNameForTests } from "../characterName";
import { BackupSection } from "../pages/BackupSection";
import {
  BundleHumanSummary,
  LAYER_LABEL,
  MemoryKnowledgePage,
  reconfirmDays,
  reconfirmOutcome,
} from "../pages/MemoryKnowledgePage";
import {
  companionReloadPlan,
  HOST_APPLIED_PREF_KEYS,
  interactionMemoryFromPrefs,
  LIVE_PREF_KEYS,
} from "../companion/gatewayWiring";
import { emptyMemory, notePlay, noteSession } from "../companion/interactionMemory";
import type { DesktopPrefs } from "../desktop";
import COMPANION_APP_SOURCE from "../companion/CompanionApp.tsx?raw";

afterEach(() => {
  vi.restoreAllMocks();
  resetCharacterNameForTests();
});

function stubMemoryApis(items: Record<string, unknown>[] = []) {
  vi.spyOn(api, "memoryList").mockResolvedValue({ items });
  vi.spyOn(api, "knowledgeList").mockResolvedValue({ nodes: [], count: 0 });
  vi.spyOn(api, "assetsList").mockResolvedValue({ assets: [], count: 0 });
  vi.spyOn(api, "knowledgeReceipts").mockResolvedValue({ receipts: [] });
  vi.spyOn(api, "domainPacks").mockResolvedValue({ packs: [] });
}

// ---------------------------------------------------------------------------
// memory-ui-001：「忘記這些」跨視窗真的忘記
// ---------------------------------------------------------------------------

describe("memory-ui-001：控制中心清空互動記憶後，角色視窗不得把舊副本寫回", () => {
  const prefs = (memory: unknown): DesktopPrefs =>
    ({ companionName: "小樞", companionInteractionMemory: memory }) as unknown as DesktopPrefs;

  it("互動記憶是就地套用的鍵（不是 host 專屬），視窗必須做事", () => {
    expect(LIVE_PREF_KEYS).toContain("companionInteractionMemory");
    expect(HOST_APPLIED_PREF_KEYS).not.toContain("companionInteractionMemory");
    // 兩張表仍互斥。
    for (const k of LIVE_PREF_KEYS) expect(HOST_APPLIED_PREF_KEYS).not.toContain(k);
  });

  it("清空 → companion-reload → 再玩一次玩具：寫回的是清空後的新記憶，不是復活的舊記憶", () => {
    // 視窗開機時已經記了一些東西（相處天數＋玩過毛球兩次）。
    let windowMemory = noteSession(emptyMemory(), Date.UTC(2026, 8, 1, 9));
    windowMemory = notePlay(windowMemory, "yarn", Date.UTC(2026, 8, 1, 10));
    windowMemory = notePlay(windowMemory, "yarn", Date.UTC(2026, 8, 1, 11));
    expect(windowMemory.toys).toEqual([{ kind: "yarn", count: 2 }]);

    // 控制中心按「忘記這些」：host 的偏好只剩空記憶。
    const before = prefs(windowMemory);
    const after = prefs(emptyMemory());
    const plan = companionReloadPlan(before, after);
    expect(plan).toEqual({ action: "live", changed: ["companionInteractionMemory"] });

    // 視窗的 live 路徑：副本換成 host 的最新值（這是修復前缺的那一步）。
    windowMemory = interactionMemoryFromPrefs(after);
    expect(windowMemory).toEqual(emptyMemory());

    // 下一次玩玩具寫回的東西只有這一次的新事件。
    const written = notePlay(windowMemory, "plane", Date.UTC(2026, 8, 2, 10));
    expect(written.toys).toEqual([{ kind: "plane", count: 1 }]);
    expect(written.daysSeen).toBe(0);
    expect(written.events).toHaveLength(1);
  });

  it("CompanionApp 的 live 路徑真的重設 memoryRef（不是只有純函式會算）", () => {
    const live = COMPANION_APP_SOURCE.slice(
      COMPANION_APP_SOURCE.indexOf("const applyLivePrefs"),
      COMPANION_APP_SOURCE.indexOf("const onCompanionReload")
    );
    expect(live).toContain("memoryRef.current = interactionMemoryFromPrefs(next)");
  });
});

// ---------------------------------------------------------------------------
// memory-ui-002：份量上限造成的遺漏要說
// ---------------------------------------------------------------------------

describe("memory-ui-002：Context Bundle 被份量上限截斷時不得說「擋下來的：沒有」", () => {
  it("excluded.overCapacity 有值就列出來", () => {
    const { container } = render(
      <BundleHumanSummary
        bundle={{
          includes: [],
          excluded: { needsReview: [], sensitive: 0, notVisibleToAgent: 0, overCapacity: 6 },
          truncated: true,
        }}
      />
    );
    const text = container.textContent ?? "";
    expect(text).toContain("超過這次能提供的份量 6 條");
    expect(text).not.toContain("擋下來的：沒有");
  });

  it("掃描上限（記憶太多沒看完）也要說", () => {
    const { container } = render(
      <BundleHumanSummary
        bundle={{
          includes: [],
          excluded: {},
          truncated: true,
          limits: { scanLimit: 1000, scanLimitReached: true },
        }}
      />
    );
    expect(container.textContent).toContain("只看了最近更新的 1000 條");
  });

  it("一般模式預覽：後端回 overCapacity 時畫面明說這份不完整", async () => {
    stubMemoryApis();
    primeCharacterNameForTests({ name: "小樞", pronoun: "她", characterId: "shu-maid" });
    vi.spyOn(api, "memoryBundle").mockResolvedValue({
      includes: [],
      excluded: { needsReview: [], sensitive: 0, notVisibleToAgent: 0, overCapacity: 3 },
      truncated: true,
    });
    const { container } = render(<MemoryKnowledgePage refreshKey={0} advanced={false} />);
    await screen.findByText("本次會提供給 AI 的內容");
    await userEvent.type(screen.getByPlaceholderText("任務描述…"), "整理報告");
    await userEvent.click(screen.getByRole("button", { name: "預覽" }));
    await screen.findByText(/超過這次能提供的份量 3 條/);
    expect(container.textContent).toContain("這份不是完整的");
    expect(container.textContent).not.toContain("擋下來的：沒有");
  });
});

// ---------------------------------------------------------------------------
// memory-ui-003：匯出範圍與上限
// ---------------------------------------------------------------------------

describe("memory-ui-003：匯出說清楚範圍，達到上限要明說", () => {
  it("按鈕叫「匯出記憶」，並先說清楚不含知識／素材／互動記憶", async () => {
    vi.spyOn(api, "memoryExport").mockResolvedValue({ count: 2, limitReached: false, items: [] });
    const { container } = render(<BackupSection />);
    expect(container.textContent).toContain("只含記憶");
    expect(container.textContent).toContain("互動記憶");
    await userEvent.click(screen.getByRole("button", { name: "匯出記憶" }));
    expect(await screen.findByText(/已匯出 2 條記憶/)).toBeInTheDocument();
  });

  it("後端說達到單次上限：畫面明說更舊的沒有匯出、不是完整備份", async () => {
    vi.spyOn(api, "memoryExport").mockResolvedValue({
      count: 1000,
      limit: 1000,
      limitReached: true,
      items: [],
    });
    render(<BackupSection />);
    await userEvent.click(screen.getByRole("button", { name: "匯出記憶" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/已達單次上限 1000 條/);
    expect(alert).toHaveTextContent(/不是完整備份/);
  });
});

// ---------------------------------------------------------------------------
// memory-ui-004：「重新確認」的文案要跟後端真的會做的事一致
// ---------------------------------------------------------------------------

describe("memory-ui-004：重新確認的天數與降級要誠實", () => {
  const staleItem = (overrides: Record<string, unknown>) => ({
    memoryId: "m-1",
    title: "早餐偏好",
    kind: "preference",
    layer: "user-memory",
    content: "內容",
    status: "stale",
    retention: { reviewAfter: "2020-01-01T00:00:00Z" },
    createdBy: { kind: "human" },
    ...overrides,
  });

  it("reconfirmDays：人建立的可延 90 天；agent 建立的使用者記憶只能延 30 天", () => {
    expect(reconfirmDays("user-memory", false)).toBe(90);
    expect(reconfirmDays("user-memory", true)).toBe(30);
    expect(reconfirmDays("persona-core", true)).toBe(30);
    expect(reconfirmDays("domain-knowledge", true)).toBe(90);
    expect(reconfirmDays("unknown-layer", true)).toBe(30);
  });

  it("reconfirmOutcome：被壓短或被降級都要說；照要求做就不硬湊訊息", () => {
    const requested = new Date(Date.UTC(2026, 11, 1)).toISOString();
    const shorter = new Date(Date.UTC(2026, 9, 1)).toISOString();
    expect(
      reconfirmOutcome({ kind: "preference", retention: { reviewAfter: shorter } }, requested, "preference")
    ).toContain("比要求的短");
    expect(
      reconfirmOutcome({ kind: "candidate", retention: { reviewAfter: requested } }, requested, "preference")
    ).toContain("等待確認");
    expect(
      reconfirmOutcome({ kind: "preference", retention: { reviewAfter: requested } }, requested, "preference")
    ).toBeNull();
    expect(reconfirmOutcome(null, requested, "preference")).toBeNull();
  });

  it("agent 建立的使用者記憶：按鈕只承諾 30 天", async () => {
    stubMemoryApis([staleItem({ createdBy: { kind: "agent", id: "claude-code" } })]);
    render(<MemoryKnowledgePage refreshKey={0} advanced />);
    const card = (await screen.findByText("早餐偏好")).closest(".provider-card") as HTMLElement;
    expect(within(card).getByRole("button", { name: "重新確認（再保留 30 天）" })).toBeInTheDocument();
    expect(within(card).queryByRole("button", { name: /90 天/ })).not.toBeInTheDocument();
  });

  it("後端壓短並降級時，畫面照實說，不只顯示成功", async () => {
    stubMemoryApis([staleItem({ createdBy: { kind: "agent", id: "claude-code" } })]);
    vi.spyOn(api, "memoryPatch").mockResolvedValue({
      kind: "candidate",
      retention: { reviewAfter: "2020-01-02T00:00:00Z" },
    });
    render(<MemoryKnowledgePage refreshKey={0} advanced />);
    const card = (await screen.findByText("早餐偏好")).closest(".provider-card") as HTMLElement;
    await userEvent.click(within(card).getByRole("button", { name: /重新確認/ }));
    expect(await within(card).findByText(/比要求的短/)).toBeInTheDocument();
    expect(card.textContent).toContain("不會再提供給 AI");
  });

  it("人建立的記憶照要求延 90 天：不硬湊警告", async () => {
    stubMemoryApis([staleItem({})]);
    const future = new Date(Date.now() + 90 * 24 * 3600 * 1000).toISOString();
    vi.spyOn(api, "memoryPatch").mockResolvedValue({
      kind: "preference",
      retention: { reviewAfter: future },
    });
    render(<MemoryKnowledgePage refreshKey={0} advanced />);
    const card = (await screen.findByText("早餐偏好")).closest(".provider-card") as HTMLElement;
    expect(within(card).getByRole("button", { name: "重新確認（再保留 90 天）" })).toBeInTheDocument();
    await userEvent.click(within(card).getByRole("button", { name: /重新確認/ }));
    await waitFor(() => expect(api.memoryPatch).toHaveBeenCalled());
    expect(card.textContent).not.toContain("比要求的短");
    expect(card.textContent).not.toContain("等待確認");
  });
});

// ---------------------------------------------------------------------------
// memory-ui-005：後端的誠實失敗訊息不得被吞掉
// ---------------------------------------------------------------------------

describe("memory-ui-005：按鈕失敗要看得見", () => {
  it("清除短期記憶：後端說清不乾淨，畫面就要說", async () => {
    stubMemoryApis();
    vi.spyOn(api, "memoryClearSession").mockRejectedValue(
      new Error("session-context 清除未完成：已刪 3 筆，仍殘留至少 2 筆無法清除")
    );
    render(<MemoryKnowledgePage refreshKey={0} advanced />);
    await userEvent.click(await screen.findByRole("button", { name: "清除短期記憶" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/清除短期記憶沒有完成/);
    expect(alert).toHaveTextContent(/仍殘留至少 2 筆/);
  });

  it("知識「不採用」失敗要看得見（和「採用」一致）", async () => {
    stubMemoryApis();
    vi.spyOn(api, "knowledgeList").mockResolvedValue({
      nodes: [{ nodeId: "k-1", title: "咖啡沖煮", status: "candidate", evidence: [], content: "x" }],
      count: 1,
    });
    vi.spyOn(api, "knowledgeReview").mockRejectedValue(new Error("reject boom"));
    render(<MemoryKnowledgePage refreshKey={0} advanced />);
    await userEvent.click(await screen.findByRole("tab", { name: /學會的知識/ }));
    await userEvent.click(await screen.findByRole("button", { name: "不採用" }));
    expect(await screen.findByText(/無法拒絕/)).toBeInTheDocument();
  });

  it("素材刪除失敗要看得見", async () => {
    stubMemoryApis();
    vi.spyOn(api, "assetsList").mockResolvedValue({
      assets: [
        {
          hash: "a".repeat(64),
          mediaType: "text",
          sizeBytes: 12,
          source: "user-import",
          originalName: "note.txt",
        },
      ],
      count: 1,
    });
    vi.spyOn(api, "assetDelete").mockRejectedValue(new Error("delete boom"));
    render(<MemoryKnowledgePage refreshKey={0} advanced />);
    await userEvent.click(await screen.findByRole("tab", { name: /素材/ }));
    await userEvent.click(await screen.findByRole("button", { name: "刪除" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/刪除素材失敗/);
  });
});

// ---------------------------------------------------------------------------
// memory-ui-006：一般模式不外洩 10 層 taxonomy
// ---------------------------------------------------------------------------

describe("memory-ui-006：一般模式只給人話分類", () => {
  const TECHNICAL = [
    "Skill",
    "Agent 交接",
    "對話暫存",
    "領域 Know-how",
    "角色核心",
    "角色經歷",
    "世界觀",
    "任務記憶",
    "領域知識",
  ];

  it("一般模式的記憶頁不出現任何技術分層名稱", async () => {
    primeCharacterNameForTests({ name: "小樞", pronoun: "她", characterId: "shu-maid" });
    stubMemoryApis([
      {
        memoryId: "m-1",
        title: "交接紀錄",
        kind: "fact",
        layer: "agent-handoff",
        content: "x",
        status: "active",
        createdBy: { kind: "runtime" },
        retention: {},
      },
    ]);
    const { container } = render(<MemoryKnowledgePage refreshKey={0} advanced={false} />);
    const card = (await screen.findByText("交接紀錄")).closest(".provider-card") as HTMLElement;
    const select = container.querySelector("select") as HTMLElement;
    for (const label of TECHNICAL) {
      expect(select.textContent, `分類選單不得出現「${label}」`).not.toContain(label);
      expect(card.textContent, `記憶卡片不得出現「${label}」`).not.toContain(label);
    }
    // 整頁也不得出現這些字（「領域知識」除外：預覽文案本來就要說「不含工作階段
    // 授權的領域知識」，那是誠實說明，不是 taxonomy 外洩）。
    const text = container.textContent ?? "";
    for (const label of TECHNICAL.filter((l) => l !== "領域知識")) {
      expect(text, `一般模式不得出現「${label}」`).not.toContain(label);
    }
    // 該筆仍看得到，只是貼人話分類（資料主權：不能為了乾淨就把記憶藏起來）。
    expect(text).toContain("工作與任務");
  });

  it("進階模式維持完整分層下拉（零能力退化）", async () => {
    stubMemoryApis();
    const { container } = render(<MemoryKnowledgePage refreshKey={0} advanced />);
    await screen.findByText(/沒有你不能刪除的記憶/);
    const options = within(container.querySelector("select") as HTMLElement).getAllByRole("option");
    expect(options.map((o) => o.textContent)).toEqual([
      "全部",
      ...Object.values(LAYER_LABEL),
    ]);
  });

  it("一般模式的分類可以真的篩選，而且選項是人話", async () => {
    primeCharacterNameForTests({ name: "小樞", pronoun: "她", characterId: "shu-maid" });
    stubMemoryApis([
      {
        memoryId: "m-1",
        title: "交接紀錄",
        kind: "fact",
        layer: "agent-handoff",
        content: "x",
        status: "active",
        createdBy: { kind: "runtime" },
        retention: {},
      },
      {
        memoryId: "m-2",
        title: "早餐偏好",
        kind: "preference",
        layer: "user-memory",
        content: "x",
        status: "active",
        createdBy: { kind: "human" },
        retention: {},
      },
    ]);
    const { container } = render(<MemoryKnowledgePage refreshKey={0} advanced={false} />);
    await screen.findByText("交接紀錄");
    const select = container.querySelector("select") as HTMLElement;
    const labels = within(select)
      .getAllByRole("option")
      .map((o) => o.textContent ?? "");
    // 「{角色}的設定」用目前角色名（不寫死小樞），其餘是固定人話。
    expect(labels[2]).toMatch(/的設定$/);
    expect(labels.filter((_, i) => i !== 2)).toEqual([
      "全部",
      "你告訴我的事",
      "學到的知識",
      "工作與任務",
      "這次對話的暫存",
      "其他",
    ]);
    await userEvent.selectOptions(select, "about-me");
    expect(screen.queryByText("交接紀錄")).not.toBeInTheDocument();
    expect(screen.getByText("早餐偏好")).toBeInTheDocument();
  });
});
