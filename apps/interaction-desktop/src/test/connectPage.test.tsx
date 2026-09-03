// Phase 3 J：連接與權限 一般模式產品化。
// 第一層四區（可以看見／可以回應／使用的裝置／需要你確認）由 mock 的 human cards 產生；
// 角色 adapter 區顯示 內建／第三方、本機／外部、可執行、網路、可接收資料、已測試（只認 Runtime 旗標）；
// 撤銷真的呼叫 API；一般模式不外洩治理術語；角色名稱走共用 hook 而非寫死。

import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";

// 共用角色名稱 hook（G 提供）：這裡 mock 成可切換的名字，驗證頁面不寫死「小樞」。
const mockedName = vi.hoisted(() => ({ name: "小樞", pronoun: "她" }));
vi.mock("../characterName", () => ({
  useCharacterName: () => ({
    name: mockedName.name,
    pronoun: mockedName.pronoun,
    characterId: "shu-maid",
    loaded: true,
  }),
  characterNameFallback: "角色",
}));

import {
  api,
  CharacterAdapterView,
  CharacterInstanceView,
  HumanCapabilities,
  HumanCard,
} from "../api";
import { AppStateProvider } from "../appstate";
import { ConnectPage, resetDecisionInboxProbeForTests } from "../pages/ConnectPage";
import { SafetyPage } from "../pages/SafetyPage";
import { providerProgress } from "../pages/CapabilitiesHub";
import {
  adapterRows,
  CharacterAdaptersSection,
  localizedName,
} from "../pages/connect/CharacterAdaptersSection";
import { RISK_TIERS } from "../riskTier";

afterEach(() => {
  vi.restoreAllMocks();
  mockedName.name = "小樞";
  mockedName.pronoun = "她";
  resetDecisionInboxProbeForTests();
});

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

const HUMAN: HumanCapabilities = {
  locale: "zh-TW",
  catalogVersion: 1,
  capabilityVersion: 1,
  generatedAt: "2026-09-01T00:00:00Z",
  constraints: [],
  receptors: [
    card({ id: "time.now", kind: "receptor", displayName: "系統時間", icon: "clock", colorRole: "input" }),
    card({
      id: "mic.level",
      kind: "receptor",
      displayName: "麥克風音量",
      icon: "mic",
      colorRole: "input",
      availability: "disabled",
    }),
  ],
  actuators: [
    card({ id: "notify.desktop", displayName: "桌面通知" }),
    card({
      id: "light.desk",
      displayName: "桌燈",
      icon: "lightbulb",
      channel: "light",
      consent: { required: true, reason: "會改變房間亮度" },
      requiresConsent: true,
    }),
    card({ id: "speaker.beep", displayName: "提示音", availability: "disabled" }),
  ],
  toolOperations: [],
};

const INSTANCES: CharacterInstanceView[] = [
  {
    instanceId: "desktop-companion",
    characterId: "shu-maid",
    displayName: { "zh-TW": "小樞", en: "Shu" },
    role: "primary-companion",
    generation: 2,
    lifecycle: "shown",
    connected: true,
    negotiated: true,
    pending: 0,
    adapterKind: "in-process",
    origin: "builtin",
    executable: false,
    network: false,
    tested: true,
    adapterId: null,
  },
  {
    instanceId: "adapter:ad-1",
    characterId: "com.example.wings",
    displayName: { en: "Wings" },
    role: "familiar",
    generation: 1,
    lifecycle: "ready",
    connected: true,
    negotiated: true,
    pending: 0,
    adapterKind: "remote-device",
    origin: "external",
    executable: false,
    network: true,
    tested: false,
    adapterId: "ad-1",
  },
];

const ADAPTERS: CharacterAdapterView[] = [
  {
    adapterId: "ad-1",
    displayName: "Wings adapter",
    characterId: "com.example.wings",
    createdAt: "2026-08-30T00:00:00Z",
    revoked: false,
    connected: true,
  },
  {
    adapterId: "ad-2",
    displayName: "舊的測試角色",
    characterId: "com.example.old",
    createdAt: "2026-08-01T00:00:00Z",
    revoked: true,
    connected: false,
  },
];

function stubApis() {
  vi.spyOn(api, "uiPrefsGet").mockResolvedValue({
    mode: "simple",
    locale: "zh-TW",
    customNames: {},
    schemaVersion: "1.0",
  });
  vi.spyOn(api, "pauseGet").mockResolvedValue({ paused: false });
  vi.spyOn(api, "capabilitiesHuman").mockResolvedValue(HUMAN);
  vi.spyOn(api, "providersList").mockResolvedValue([
    {
      identity: { id: "provider.esp32.desk", displayName: "書桌 ESP32", kind: "device", trustLevel: "paired" },
      state: "available",
      receptors: ["desk.temp"],
      actuators: [],
      detail: "",
    },
    {
      identity: {
        id: "provider.companion.desktop",
        displayName: "桌面角色：小樞（Presentation）",
        kind: "local",
        trustLevel: "builtin",
      },
      state: "available",
      receptors: [],
      actuators: [],
      detail: "",
    },
    {
      identity: { id: "provider.ai.codex", displayName: "Codex", kind: "ai-agent", trustLevel: "verified" },
      state: "available",
      receptors: [],
      actuators: [],
    },
  ]);
  vi.spyOn(api, "mobileStatus").mockResolvedValue({
    devices: [
      {
        deviceId: "d1",
        name: "Alex 的 iPhone",
        model: "iPhone 15",
        pairedAt: "2026-08-01T00:00:00Z",
        connected: true,
      },
    ],
    bonjour: { advertised: true, service: "_interact-ai._tcp", instance: "mac" },
  });
  vi.spyOn(api, "activityInbox").mockResolvedValue({
    pendingCount: 1,
    items: [
      {
        kind: "agent-session",
        itemId: "s-1",
        status: "waiting-for-consent",
        title: "整理報告需要你同意",
        route: "ai",
        needsDecision: true,
      },
      {
        kind: "action-result",
        itemId: "a-1",
        status: "completed",
        title: "已送出通知",
        route: "activity",
        needsDecision: false,
      },
    ],
  });
  vi.spyOn(api, "sessionGet").mockResolvedValue({
    sessionId: "sess-1",
    state: "active",
    startedAt: "2026-09-01T00:00:00Z",
    consents: [{ scope: { kind: "actuator", id: "light.desk" } }],
  });
  vi.spyOn(api, "status").mockResolvedValue({ emergencyStop: false });
  vi.spyOn(api, "auditTail").mockResolvedValue([]);
  vi.spyOn(api, "characterInstances").mockResolvedValue({ instances: INSTANCES });
  vi.spyOn(api, "characterAdapters").mockResolvedValue({ adapters: ADAPTERS });
  vi.spyOn(api, "characterManifest").mockRejectedValue(
    new Error("404 Not Found on /v1/character/manifest")
  );
}

function renderConnect(advanced = false) {
  const onNavigate = vi.fn();
  const utils = render(
    <AppStateProvider ready refreshKey={0}>
      <ConnectPage refreshKey={0} advanced={advanced} onNavigate={onNavigate} />
    </AppStateProvider>
  );
  return { ...utils, onNavigate };
}

// ---------------------------------------------------------------------------

describe("連接與權限：第一層四區", () => {
  it("四區由 human cards 產生：只列啟用中的感知來源與回應方式", async () => {
    stubApis();
    renderConnect();
    const see = await screen.findByTestId("connect-area-see");
    await within(see).findByText("系統時間");
    expect(within(see).getByRole("heading", { name: /可以看見/ })).toBeInTheDocument();
    expect(within(see).queryByText("麥克風音量")).not.toBeInTheDocument();

    const respond = screen.getByTestId("connect-area-respond");
    expect(within(respond).getByRole("heading", { name: /可以回應/ })).toBeInTheDocument();
    expect(within(respond).getByText("桌面通知")).toBeInTheDocument();
    expect(within(respond).getByText("桌燈")).toBeInTheDocument();
    expect(within(respond).getByText("使用前會先問你")).toBeInTheDocument();
    expect(within(respond).queryByText("提示音")).not.toBeInTheDocument();
  });

  it("使用的裝置：iPhone、硬體、角色 adapter 都在；AI 幫手與桌面呈現層 provider 不重複列", async () => {
    stubApis();
    renderConnect();
    const devices = await screen.findByTestId("connect-area-devices");
    expect(within(devices).getByRole("heading", { name: /使用的裝置/ })).toBeInTheDocument();
    await within(devices).findByText("Alex 的 iPhone");
    expect(within(devices).getByText("書桌 ESP32")).toBeInTheDocument();
    // 狀態標籤走 statusProjection（available → 可用），不是原始 enum。
    expect(within(devices).getByText("可用")).toBeInTheDocument();
    expect(within(devices).queryByText(/Presentation/)).not.toBeInTheDocument();
    expect(within(devices).queryByText("Codex")).not.toBeInTheDocument();
    // 角色 adapter 出現在裝置區。
    await within(devices).findByTestId("adapter-row-instance:desktop-companion");
    expect(within(devices).getByTestId("adapter-row-instance:adapter:ad-1")).toBeInTheDocument();
  });

  it("需要你確認：待決定事項走人話狀態投影，同意摘要用能力名稱", async () => {
    stubApis();
    const { onNavigate } = renderConnect();
    const confirm = await screen.findByTestId("connect-area-confirm");
    expect(within(confirm).getByRole("heading", { name: /需要你確認/ })).toBeInTheDocument();
    await within(confirm).findByText("整理報告需要你同意");
    expect(within(confirm).getByText("等你同意")).toBeInTheDocument();
    expect(within(confirm).queryByText("waiting-for-consent")).not.toBeInTheDocument();
    expect(within(confirm).queryByText("已送出通知")).not.toBeInTheDocument();
    await waitFor(() => expect(within(confirm).getByText(/目前有 1 項額外授權：桌燈/)).toBeInTheDocument());
    fireEvent.click(within(confirm).getByRole("button", { name: "處理" }));
    expect(onNavigate).toHaveBeenCalledWith("ai");
  });

  it("需要你確認：pendingCount:3 但最近 20 筆都不需決定 → 不得說「現在沒有需要你決定的事」（ia-settings-035）", async () => {
    stubApis();
    vi.spyOn(api, "activityInbox").mockResolvedValue({
      pendingCount: 3,
      count: 20,
      totalBeforeLimit: 23,
      items: Array.from({ length: 20 }, (_, i) => ({
        kind: "action-result",
        itemId: `a-${i}`,
        status: "completed",
        title: `已送出通知 ${i}`,
        route: "activity",
        needsDecision: false,
      })),
    });
    const { onNavigate } = renderConnect();
    const confirm = await screen.findByTestId("connect-area-confirm");
    await within(confirm).findByText(/還有 3 項待決定不在這一頁/);
    expect(within(confirm).queryByText("現在沒有需要你決定的事。")).not.toBeInTheDocument();
    fireEvent.click(within(confirm).getByRole("button", { name: "前往活動歷史" }));
    expect(onNavigate).toHaveBeenCalledWith("activity");
  });

  it("需要你確認：待決定清單優先以 needsDecision 篩選查詢；舊 daemon 拒絕時退回不帶篩選", async () => {
    stubApis();
    const inbox = vi.spyOn(api, "activityInbox").mockImplementation(async (filter) => {
      if (filter?.needsDecision !== undefined) throw new Error("unknown field `needsDecision`");
      return { pendingCount: 0, items: [] };
    });
    renderConnect();
    const confirm = await screen.findByTestId("connect-area-confirm");
    await within(confirm).findByText("現在沒有需要你決定的事。");
    expect(inbox.mock.calls.map(([f]) => f)).toEqual([{ limit: 20, needsDecision: true }, { limit: 20 }]);
  });

  it("緊急停止中：固定安全文字＋前往解除，不被角色文案取代", async () => {
    stubApis();
    vi.spyOn(api, "status").mockResolvedValue({ emergencyStop: true });
    renderConnect();
    const confirm = await screen.findByTestId("connect-area-confirm");
    await within(confirm).findByText(/緊急停止中/);
    fireEvent.click(within(confirm).getByRole("button", { name: "前往解除" }));
    expect(screen.getByRole("tab", { name: "同意與安全" })).toHaveAttribute("aria-selected", "true");
    // 解除流程保留在同意與安全（等它的非同步載入完成，避免測試結束後才更新狀態）。
    await screen.findByRole("button", { name: /開始安全解除流程/ });
    await screen.findAllByRole("button", { name: "撤回" });
    expect(screen.getByText(/緊急停止已啟動/)).toBeInTheDocument();
  });

  it("第二層「全部能力與裝置」仍可達：既有分類分頁都在，管理按鈕會切到對應分類", async () => {
    stubApis();
    renderConnect();
    await screen.findByTestId("connect-area-see");
    expect(screen.getByText("全部能力與裝置")).toBeInTheDocument();
    for (const name of ["感知來源", "回應方式", "工具操作", "裝置與提供者"]) {
      expect(screen.getByRole("tab", { name })).toBeInTheDocument();
    }
    fireEvent.click(screen.getByRole("button", { name: "加入或掃描裝置" }));
    expect(screen.getByRole("tab", { name: "裝置與提供者" })).toHaveAttribute("aria-selected", "true");
    // 第二層的裝置分頁也列出角色 adapter（進階與一般都可達）。
    await waitFor(() =>
      expect(screen.getAllByTestId("adapter-row-instance:desktop-companion").length).toBeGreaterThanOrEqual(2)
    );
  });

  it("一般模式不外洩治理術語，也不寫死角色名稱", async () => {
    stubApis();
    mockedName.name = "阿樞";
    const { container } = renderConnect(false);
    const see = await screen.findByTestId("connect-area-see");
    await within(see).findByText("系統時間");
    await screen.findByTestId("adapter-row-instance:desktop-companion");
    await screen.findByText("Alex 的 iPhone");
    const banned = [
      "Provider",
      "Receptor",
      "Actuator",
      "受器",
      "動器",
      "Lease",
      "UUID",
      "Receipt",
      "app-server",
      "hello",
      "pair-ok",
      "Registry",
      "Agent Session",
      "YAML",
      "manifest",
      "in-process",
      "remote-device",
      "primary-companion",
      "trustLevel",
      "TLS",
      "mDNS 本機網路裝置",
    ];
    const assertClean = (where: string) => {
      const text = container.textContent ?? "";
      for (const word of banned) {
        expect(text, `${where}：一般模式不得出現「${word}」`).not.toContain(word);
      }
      return text;
    };
    const first = assertClean("第一層");
    // 角色名稱來自 hook：換名字後頁面跟著變，不留寫死的「小樞」（mock 資料裡的角色顯示名除外）。
    expect(first).toContain("阿樞和 AI 現在能接收的資訊");

    // 第二層「裝置與提供者」：iPhone 說明、已連接的裝置與來源、角色 adapter 也要乾淨。
    fireEvent.click(screen.getByRole("tab", { name: "裝置與提供者" }));
    await screen.findByText("已連接的裝置與來源");
    await waitFor(() => expect(screen.getAllByText("書桌 ESP32").length).toBeGreaterThanOrEqual(2));
    const second = assertClean("裝置與提供者");
    expect(second).toContain("阿樞可以知道");
    expect(second).toContain("外接裝置・信任：已配對・狀態：可用");
    expect(second).not.toContain("生命週期狀態");
    // 桌面呈現層 provider 的後端顯示名照實顯示，但補上人話說明。
    expect(second).toContain("這是桌面角色的呈現層：只負責演出，不持有任何權限。");
  });

  it("進階模式才顯示原始 id（零能力退化）", async () => {
    stubApis();
    renderConnect(true);
    const row = await screen.findAllByTestId("adapter-row-instance:adapter:ad-1");
    expect(row[0].textContent).toContain("remote-device");
    expect(row[0].textContent).toContain("adapter ad-1");
  });
});

// ---------------------------------------------------------------------------

describe("角色 adapter 區", () => {
  function renderSection(advanced = false) {
    return render(<CharacterAdaptersSection refreshKey={0} advanced={advanced} standalone />);
  }

  it("每列顯示 內建／第三方、本機／外部、可執行、網路、可接收資料、已測試", async () => {
    stubApis();
    renderSection();
    const shu = await screen.findByTestId("adapter-row-instance:desktop-companion");
    expect(within(shu).getByText("小樞")).toBeInTheDocument();
    expect(within(shu).getByText("內建")).toBeInTheDocument();
    expect(within(shu).getByText("本機")).toBeInTheDocument();
    expect(within(shu).getByText(/有可執行程式：否/)).toBeInTheDocument();
    expect(within(shu).getByText(/需要網路：否/)).toBeInTheDocument();
    expect(within(shu).getByText(/可以接收：/)).toBeInTheDocument();
    expect(within(shu).getByText("已測試")).toBeInTheDocument();
    expect(within(shu).getByText("已連線")).toBeInTheDocument();
    expect(within(shu).getByText("主要角色")).toBeInTheDocument();
    expect(within(shu).getByText(/目前狀態：顯示中/)).toBeInTheDocument();
    expect(within(shu).queryByRole("button", { name: "撤銷" })).not.toBeInTheDocument();

    const wings = screen.getByTestId("adapter-row-instance:adapter:ad-1");
    expect(within(wings).getByText("Wings")).toBeInTheDocument();
    expect(within(wings).getByText("外部（第三方）")).toBeInTheDocument();
    expect(within(wings).getByText("外部")).toBeInTheDocument();
    expect(within(wings).getByText(/需要網路：是/)).toBeInTheDocument();
    expect(within(wings).getByText(/永遠不會自動連線/)).toBeInTheDocument();
    expect(within(wings).getByRole("button", { name: "撤銷" })).toBeInTheDocument();

    const old = screen.getByTestId("adapter-row-adapter:ad-2");
    expect(within(old).getByText("已撤銷")).toBeInTheDocument();
    expect(within(old).getByText(/有可執行程式：未回報/)).toBeInTheDocument();
    expect(within(old).queryByRole("button", { name: "撤銷" })).not.toBeInTheDocument();
  });

  it("已測試只認 Runtime 旗標：連上且協商完成但 tested=false 仍是「未測試」", async () => {
    stubApis();
    renderSection();
    const wings = await screen.findByTestId("adapter-row-instance:adapter:ad-1");
    expect(within(wings).getByText("已連線")).toBeInTheDocument();
    expect(within(wings).getByText("未測試")).toBeInTheDocument();
    expect(within(wings).queryByText("已測試")).not.toBeInTheDocument();
    expect(within(wings).getByText(/連上或協商完成都不等於測過/)).toBeInTheDocument();

    const rows = adapterRows(INSTANCES, ADAPTERS);
    expect(rows.find((r) => r.instanceId === "adapter:ad-1")?.tested).toBe(false);
    expect(rows.find((r) => r.instanceId === "desktop-companion")?.tested).toBe(true);
    // 只登記、沒連線的 adapter：可執行／網路一律未回報，不猜。
    const orphan = rows.find((r) => r.key === "adapter:ad-2");
    expect(orphan?.executable).toBe("unknown");
    expect(orphan?.network).toBe("unknown");
    expect(orphan?.tested).toBe(false);
    // 已撤銷排最後、主要角色排最前。
    expect(rows[0].instanceId).toBe("desktop-companion");
    expect(rows[rows.length - 1].revoked).toBe(true);
  });

  it("撤銷需要二次確認，確認後真的呼叫 DELETE 並重新讀取", async () => {
    stubApis();
    const revoke = vi
      .spyOn(api, "characterAdapterRevoke")
      .mockResolvedValue({ adapterId: "ad-1", revoked: true, disconnected: true });
    renderSection();
    const wings = await screen.findByTestId("adapter-row-instance:adapter:ad-1");
    fireEvent.click(within(wings).getByRole("button", { name: "撤銷" }));
    expect(revoke).not.toHaveBeenCalled();
    fireEvent.click(within(wings).getByRole("button", { name: /確定撤銷？/ }));
    await waitFor(() => expect(revoke).toHaveBeenCalledWith("ad-1"));
    await screen.findByText(/已撤銷並斷線/);
    expect(api.characterAdapters).toHaveBeenCalledTimes(2);
  });

  it("撤銷失敗時誠實回報，登記狀態未變", async () => {
    stubApis();
    vi.spyOn(api, "characterAdapterRevoke").mockRejectedValue(new Error("403 token_scope_forbidden"));
    renderSection();
    const wings = await screen.findByTestId("adapter-row-instance:adapter:ad-1");
    fireEvent.click(within(wings).getByRole("button", { name: "撤銷" }));
    fireEvent.click(within(wings).getByRole("button", { name: /確定撤銷？/ }));
    await screen.findByText(/撤銷失敗：.*登記狀態未變/);
  });

  it("桌面角色有 manifest 時，「可以接收」用角色模組的共用摘要", async () => {
    stubApis();
    vi.spyOn(api, "characterManifest").mockResolvedValue({
      schemaVersion: "1.0",
      characterId: "shu-maid",
      displayName: { "zh-TW": "小樞" },
      version: "3.0.0",
      adapterKind: "in-process",
      entrypoint: { kind: "builtin", id: "shu-rig" },
      assets: [],
      capabilities: {},
      inputCapabilities: {
        "input.click": { supported: true },
        "input.text": { supported: true },
        "input.hover": { supported: false },
      },
      channels: [],
      states: [],
      intents: [],
      variants: [],
      locales: ["zh-TW"],
      securityRequirements: {
        network: false,
        executable: false,
        fileAccess: "none",
        audioOutput: true,
        microphone: false,
        camera: false,
      },
      resourceLimits: { maxAssetBytes: 1, maxConcurrentCommands: 1, maxQueue: 1, maxFps: 60 },
      fallbacks: {},
      compatibility: { protocol: "1.x" },
    });
    renderSection();
    const shu = await screen.findByTestId("adapter-row-instance:desktop-companion");
    await within(shu).findByText("可以接收：點擊、文字輸入");
  });

  it("沒有任何角色接上時說清楚，不放假資料", async () => {
    stubApis();
    vi.spyOn(api, "characterInstances").mockResolvedValue({ instances: [] });
    vi.spyOn(api, "characterAdapters").mockResolvedValue({ adapters: [] });
    renderSection();
    await screen.findByText(/目前沒有任何角色接上系統/);
  });

  it("localizedName：zh-TW 優先，其次 zh、en、第一個；缺省「角色」", () => {
    expect(localizedName({ en: "Shu", "zh-TW": "小樞" })).toBe("小樞");
    expect(localizedName({ en: "Shu", "zh-CN": "小枢" })).toBe("小枢");
    expect(localizedName({ ja: "シュウ", en: "Shu" })).toBe("Shu");
    expect(localizedName({ ja: "シュウ" })).toBe("シュウ");
    expect(localizedName({})).toBe("角色");
    expect(localizedName(undefined)).toBe("角色");
    expect(localizedName("  ")).toBe("角色");
  });
});

// ---------------------------------------------------------------------------

describe("同意與安全／術語：不寫死角色名稱，分級文案通用", () => {
  it("SafetyPage 的指路文案與按鈕用 hook 的名字", async () => {
    stubApis();
    mockedName.name = "阿樞";
    const onNavigate = vi.fn();
    render(
      <AppStateProvider ready refreshKey={0}>
        <SafetyPage refreshKey={0} onNavigate={onNavigate} />
      </AppStateProvider>
    );
    const button = await screen.findByRole("button", { name: "前往阿樞" });
    expect(screen.getByText(/阿樞的表現設定/)).toBeInTheDocument();
    expect(screen.queryByText(/小樞/)).not.toBeInTheDocument();
    fireEvent.click(button);
    expect(onNavigate).toHaveBeenCalledWith("companion");
  });

  it("風險分級與 provider 階梯的文案不含角色名稱，名稱由呼叫端帶入", () => {
    for (const tier of RISK_TIERS) {
      expect(tier.policy).not.toContain("小樞");
      expect(tier.hardLimits ?? "").not.toContain("小樞");
    }
    const generic = providerProgress({ state: "discovered", enabledCapabilities: 0 });
    expect(generic.hint).toContain("角色還不能用它做任何事");
    const named = providerProgress({ state: "disabled", enabledCapabilities: 0 }, "阿樞");
    expect(named.hint).toContain("阿樞不會用它做任何事");
    expect(named.hint).not.toContain("小樞");
    // 介面不認得的狀態不把原始字串當標籤。
    const bogus = providerProgress({ state: "some-new-state", enabledCapabilities: 0 });
    expect(bogus.label).toBe("狀態不確定");
  });
});
