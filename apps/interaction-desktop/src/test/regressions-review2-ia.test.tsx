// 對抗審查（第二輪）ia-settings／safety-invariants 的回歸測試。
// 每一項都對應一個已確認的缺陷，沒有修復就會失敗：
//  * ia-settings-012：緊急停止中「前往解除」在已位於安全頁時是死點擊。
//  * ia-settings-013：收件匣次要行印出原始裝置／受器 id。
//  * ia-settings-015：待決定數忽略 pendingCountExact，把下限講成總數。
//  * ia-settings-017：活動紀錄與全域搜尋直接印 runtime 原始 intent。
//  * ia-settings-019：「重新驗證」是無回饋的 floating promise。
//  * ia-settings-020：「現在」頁工作階段開始／結束沒有失敗回報。
//  * ia-settings-021：全域搜尋印出知識更新的原始 triggeredBy。
//  * safety-invariants-075：L4 同意對話預設「整個工作階段」。

import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { api, HumanCapabilities, HumanCard, Receipt } from "../api";
import { AppStateProvider } from "../appstate";
import { PageBody, useNavigation } from "../App";
import { ActivityPage, InboxSection, verifyResultMessage } from "../pages/ActivityPage";
import { HomePage } from "../pages/HomePage";
import { SafetyPage, consentScopeOptions, defaultConsentScope } from "../pages/SafetyPage";
import { GlobalSearch } from "../components/GlobalSearch";
import { inboxDeviceLabel, knowledgeTriggerLabel, receiptIntentLabel } from "../statusProjection";

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
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
    card({
      id: "iphone.mic-level",
      kind: "receptor",
      displayName: "iPhone 環境音量",
      colorRole: "input",
      consent: { required: true },
      requiresConsent: true,
      availability: "disabled",
    }),
  ],
  actuators: [card({ id: "notify.desktop", displayName: "桌面通知" })],
  toolOperations: [],
};

function receipt(overrides: Partial<Receipt> = {}): Receipt {
  return {
    actionId: "a-1",
    planId: "p-1",
    actuatorId: "notify.desktop",
    intent: "emergency-stop",
    currentStatus: "uncertain",
    timestamps: [["accepted", "2026-09-01T00:00:00Z"]],
    policyDecisions: [],
    effectiveBoundedParameters: {},
    requestedParameters: {},
    errors: [],
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// ia-settings-012：導到「已經在的」路由仍然要有作用
// ---------------------------------------------------------------------------

describe("ia-settings-012 緊急停止「前往解除」在安全頁上仍然有作用", () => {
  function stubConnectApis() {
    vi.spyOn(api, "uiPrefsGet").mockResolvedValue({
      mode: "simple",
      locale: "zh-TW",
      customNames: {},
      schemaVersion: "1.0",
    });
    vi.spyOn(api, "pauseGet").mockResolvedValue({ paused: false });
    vi.spyOn(api, "capabilitiesHuman").mockResolvedValue(HUMAN);
    vi.spyOn(api, "providersList").mockResolvedValue([]);
    vi.spyOn(api, "mobileStatus").mockResolvedValue({ devices: [] });
    vi.spyOn(api, "activityInbox").mockResolvedValue({
      pendingCount: 0,
      items: [],
      count: 0,
      totalBeforeLimit: 0,
    });
    vi.spyOn(api, "sessionGet").mockResolvedValue(null);
    vi.spyOn(api, "status").mockResolvedValue({ emergencyStop: true });
    vi.spyOn(api, "auditTail").mockResolvedValue([]);
    vi.spyOn(api, "characterInstances").mockResolvedValue({ instances: [] });
    vi.spyOn(api, "characterAdapters").mockResolvedValue({ adapters: [] });
    vi.spyOn(api, "characterManifest").mockRejectedValue(new Error("404"));
    vi.spyOn(api, "actionsList").mockResolvedValue([]);
    vi.spyOn(api, "knowledgeReceipts").mockResolvedValue({ receipts: [] });
    vi.spyOn(api, "recipesList").mockResolvedValue([]);
  }

  /** Shell 的導覽骨架：同一個 useNavigation＋同一個 key 策略＋真的 PageBody。 */
  function Harness() {
    const { tab, mountKey, goTo } = useNavigation("home");
    return (
      <AppStateProvider ready refreshKey={0}>
        <button onClick={() => goTo("safety")}>緊急停止中 — 前往解除</button>
        <div key={mountKey}>
          <PageBody
            tab={tab}
            refreshKey={0}
            events={[]}
            advanced={false}
            onNavigate={goTo}
            onRerunOnboarding={() => {}}
          />
        </div>
      </AppStateProvider>
    );
  }

  it("已在安全頁、又把頁內分頁切到「裝置與能力」時，再按一次仍會回到解除流程", async () => {
    stubConnectApis();
    render(<Harness />);
    const goToSafety = screen.getByRole("button", { name: /前往解除/ });

    fireEvent.click(goToSafety);
    expect(await screen.findByRole("tab", { name: "同意與安全" })).toHaveAttribute(
      "aria-selected",
      "true"
    );
    await screen.findByRole("button", { name: /開始安全解除流程/ });

    // 使用者在頁內切到另一個分頁（App 的 route 沒變，仍是 safety）。
    fireEvent.click(screen.getByRole("tab", { name: "裝置與能力" }));
    expect(screen.getByRole("tab", { name: "裝置與能力" })).toHaveAttribute(
      "aria-selected",
      "true"
    );
    expect(screen.queryByRole("button", { name: /開始安全解除流程/ })).not.toBeInTheDocument();

    // 再按一次頂列的「前往解除」：修好之前這裡完全沒有反應。
    fireEvent.click(goToSafety);
    expect(await screen.findByRole("tab", { name: "同意與安全" })).toHaveAttribute(
      "aria-selected",
      "true"
    );
    expect(await screen.findByRole("button", { name: /開始安全解除流程/ })).toBeInTheDocument();
  });

  it("導覽序號：導到「已經在的」路由也會換一把新的掛載 key", () => {
    const seen: string[] = [];
    function Probe() {
      const nav = useNavigation("home");
      seen.push(nav.mountKey);
      return <button onClick={() => nav.goTo("safety")}>go</button>;
    }
    render(<Probe />);
    const go = screen.getByRole("button", { name: "go" });
    fireEvent.click(go);
    fireEvent.click(go);
    const unique = Array.from(new Set(seen));
    expect(unique.length).toBeGreaterThanOrEqual(3);
    expect(unique.filter((k) => k.startsWith("safety#")).length).toBeGreaterThanOrEqual(2);
  });

  it("解除區塊會自己取得焦點（人被導過來時不用自己找）", async () => {
    stubConnectApis();
    render(
      <AppStateProvider ready refreshKey={0}>
        <SafetyPage refreshKey={0} onNavigate={() => {}} />
      </AppStateProvider>
    );
    const recover = await screen.findByRole("button", { name: /開始安全解除流程/ });
    await waitFor(() => expect(document.activeElement).toBe(recover));
  });
});

// ---------------------------------------------------------------------------
// ia-settings-013／015／017／019：活動紀錄
// ---------------------------------------------------------------------------

describe("ia-settings-013 收件匣不印原始裝置識別碼", () => {
  const ITEMS = [
    {
      kind: "safety-event",
      itemId: "e-1",
      status: "sensor.started",
      title: "感測開始：iPhone",
      occurredAt: "2026-01-01T00:00:00Z",
      route: "safety",
      needsDecision: false,
      deviceId: "iphone-a1b2c3d4",
    },
    {
      kind: "action-result",
      itemId: "a-1",
      status: "completed",
      title: "已送出通知",
      occurredAt: "2026-01-01T00:00:00Z",
      route: "activity",
      needsDecision: false,
      deviceId: "notify.desktop",
    },
  ];

  it("一般模式：手機 id 不上畫面，能力 id 換成名字；進階模式才有原始值", async () => {
    vi.spyOn(api, "activityInbox").mockResolvedValue({
      items: ITEMS,
      count: 2,
      totalBeforeLimit: 2,
      pendingCount: 0,
    });
    const resolve = (id: string) =>
      [...HUMAN.receptors, ...HUMAN.actuators].find((c) => c.id === id)?.displayName ?? null;
    const { unmount } = render(
      <InboxSection refreshKey={0} onNavigate={() => {}} resolveDeviceName={resolve} />
    );
    const list = await screen.findByTestId("activity-inbox-results");
    expect(list.textContent ?? "").not.toContain("iphone-a1b2c3d4");
    expect(list.textContent ?? "").toContain("你的 iPhone");
    expect(list.textContent ?? "").toContain("桌面通知");
    expect(list.textContent ?? "").not.toContain("notify.desktop");
    unmount();

    render(
      <InboxSection refreshKey={0} advanced onNavigate={() => {}} resolveDeviceName={resolve} />
    );
    const advancedList = await screen.findByTestId("activity-inbox-results");
    expect(advancedList.textContent ?? "").toContain("iphone-a1b2c3d4");
  });

  it("inboxDeviceLabel：查不到名字的原始 id 一律不說（不外洩）", () => {
    expect(inboxDeviceLabel("iphone-a1b2c3d4")).toBe("你的 iPhone");
    expect(inboxDeviceLabel("iphone.mic-level")).toBe("你的 iPhone");
    expect(inboxDeviceLabel("builtin.microphone")).toBe("麥克風");
    expect(inboxDeviceLabel("weird.internal.id")).toBeNull();
    expect(inboxDeviceLabel("notify.desktop", () => "桌面通知")).toBe("桌面通知");
    expect(inboxDeviceLabel("notify.desktop", (id) => id)).toBeNull();
    expect(inboxDeviceLabel(undefined)).toBeNull();
  });
});

describe("ia-settings-015 待決定數不精確時說「至少」", () => {
  it("pendingCountExact=false：標題說「至少 N 項」並加上未掃完的說明", async () => {
    vi.spyOn(api, "activityInbox").mockResolvedValue({
      items: [],
      count: 0,
      totalBeforeLimit: 5,
      pendingCount: 5,
      pendingCountExact: false,
    });
    const { container } = render(<InboxSection refreshKey={0} onNavigate={() => {}} />);
    await screen.findByText(/統一收件匣（待決定 至少 5 項／共 5）/);
    await waitFor(() => expect(container.textContent ?? "").toContain("待決定數只是下限"));
  });

  it("pendingCountExact 缺席（舊 daemon）＝精確：不加「至少」", async () => {
    vi.spyOn(api, "activityInbox").mockResolvedValue({
      items: [],
      count: 0,
      totalBeforeLimit: 2,
      pendingCount: 2,
    });
    const { container } = render(<InboxSection refreshKey={0} onNavigate={() => {}} />);
    await screen.findByText("統一收件匣（待決定 2 項／共 2）");
    expect(container.textContent ?? "").not.toContain("至少");
    expect(container.textContent ?? "").not.toContain("只是下限");
  });
});

describe("ia-settings-017／019 活動紀錄的意圖與重新驗證", () => {
  function stubActivity(receipts: Receipt[]) {
    vi.spyOn(api, "actionsList").mockResolvedValue(receipts);
    vi.spyOn(api, "activityInbox").mockResolvedValue({
      items: [],
      count: 0,
      totalBeforeLimit: 0,
      pendingCount: 0,
    });
    vi.spyOn(api, "capabilitiesHuman").mockResolvedValue(HUMAN);
    vi.spyOn(api, "uiPrefsGet").mockResolvedValue({
      mode: "simple",
      locale: "zh-TW",
      customNames: {},
      schemaVersion: "1.0",
    });
    vi.spyOn(api, "pauseGet").mockResolvedValue({ paused: false });
  }

  it("一般模式的區塊標題是人話，不出現 runtime 原始 intent", async () => {
    stubActivity([receipt({ intent: "emergency-stop" })]);
    const { container } = render(
      <AppStateProvider ready refreshKey={0}>
        <ActivityPage refreshKey={0} events={[]} advanced={false} />
      </AppStateProvider>
    );
    await screen.findByText(/緊急停止/);
    expect(container.textContent ?? "").not.toContain("emergency-stop");
    expect(receiptIntentLabel("presence")).toBe("一個需要回應的訊號");
    expect(receiptIntentLabel("送出桌面通知")).toBe("送出桌面通知");
  });

  it("「重新驗證」失敗會顯示訊息，不是無聲的 floating promise", async () => {
    stubActivity([receipt({ currentStatus: "uncertain" })]);
    const verify = vi
      .spyOn(api, "verifyAction")
      .mockRejectedValue(new Error("runtime unreachable"));
    render(
      <AppStateProvider ready refreshKey={0}>
        <ActivityPage refreshKey={0} events={[]} advanced={false} />
      </AppStateProvider>
    );
    fireEvent.click(await screen.findByRole("button", { name: "重新驗證" }));
    await waitFor(() => expect(verify).toHaveBeenCalledWith("a-1"));
    expect(await screen.findByText(/重新查驗失敗/)).toBeInTheDocument();
  });

  it("查驗結果只說觀察得到的事實（送出 ≠ 已驗證）", () => {
    expect(verifyResultMessage(receipt({ verification: { verdict: "observed" } }))).toContain(
      "確認觀察到實際效果"
    );
    expect(
      verifyResultMessage(receipt({ verification: { verdict: "acknowledged-only" } }))
    ).toContain("仍未觀察到實際效果");
    expect(verifyResultMessage(receipt({ currentStatus: "uncertain" }))).toContain(
      "結果仍然不確定"
    );
  });
});

// ---------------------------------------------------------------------------
// ia-settings-020：工作階段開始／結束的失敗回報
// ---------------------------------------------------------------------------

describe("ia-settings-020 工作階段開始失敗會說出來", () => {
  it("sessionStart 失敗：畫面出現警示，不是只有未處理的 promise", async () => {
    vi.spyOn(api, "status").mockResolvedValue({
      emergencyStop: false,
      recipes: { loaded: 0 },
      activeSensors: [],
    });
    vi.spyOn(api, "actionsList").mockResolvedValue([]);
    vi.spyOn(api, "sessionGet").mockResolvedValue(null);
    vi.spyOn(api, "providersList").mockResolvedValue([]);
    vi.spyOn(api, "knowledgeReceipts").mockResolvedValue({ receipts: [] });
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([]);
    vi.spyOn(api, "activityInbox").mockResolvedValue({
      items: [],
      count: 0,
      totalBeforeLimit: 0,
      pendingCount: 0,
    });
    vi.spyOn(api, "memoryList").mockResolvedValue({ items: [] });
    vi.spyOn(api, "capabilitiesHuman").mockResolvedValue(HUMAN);
    vi.spyOn(api, "uiPrefsGet").mockResolvedValue({
      mode: "simple",
      locale: "zh-TW",
      customNames: {},
      schemaVersion: "1.0",
    });
    vi.spyOn(api, "pauseGet").mockResolvedValue({ paused: false });
    const start = vi.spyOn(api, "sessionStart").mockRejectedValue(new Error("daemon down"));

    render(
      <AppStateProvider ready refreshKey={0}>
        <HomePage refreshKey={0} events={[]} onNavigate={() => {}} />
      </AppStateProvider>
    );
    await userEvent.click(await screen.findByText("詳細狀態"));
    const button = await screen.findByRole("button", { name: "開始工作階段" });
    await userEvent.click(button);
    await waitFor(() => expect(start).toHaveBeenCalled());
    const alerts = await screen.findAllByRole("alert");
    expect(alerts.some((a) => (a.textContent ?? "").includes("開始工作階段失敗"))).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// ia-settings-021：全域搜尋不印原始 triggeredBy／包 id
// ---------------------------------------------------------------------------

describe("ia-settings-021 全域搜尋的知識更新說人話", () => {
  function stubSearch() {
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([]);
    vi.spyOn(api, "providersList").mockResolvedValue([]);
    vi.spyOn(api, "memoryList").mockResolvedValue({ items: [] });
    vi.spyOn(api, "knowledgeList").mockResolvedValue({ nodes: [], count: 0 });
    vi.spyOn(api, "domainPacks").mockResolvedValue({
      packs: [{ pack: { id: "pack.cooking", displayName: "料理" }, installed: true }],
    });
    vi.spyOn(api, "actionsList").mockResolvedValue([receipt({ intent: "emergency-stop" })]);
    vi.spyOn(api, "knowledgeReceipts").mockResolvedValue({
      receipts: [{ triggeredBy: "user-correction", updateId: "u-1234567890ab" }],
    });
    vi.spyOn(api, "uiPrefsGet").mockResolvedValue({
      mode: "simple",
      locale: "zh-TW",
      customNames: {},
      schemaVersion: "1.0",
    });
    vi.spyOn(api, "pauseGet").mockResolvedValue({ paused: false });
  }

  it("一般模式：triggeredBy／intent／知識包 id 的原始值都不上畫面", async () => {
    stubSearch();
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
    await userEvent.type(input, "你的更正");
    await screen.findByText("知識更新：你的更正");
    expect(container.textContent ?? "").not.toContain("user-correction");

    await userEvent.clear(input);
    await userEvent.type(input, "緊急停止");
    await screen.findByText("互動結果：緊急停止");
    expect(container.textContent ?? "").not.toContain("emergency-stop");

    await userEvent.clear(input);
    await userEvent.type(input, "料理");
    await screen.findByText("知識包：料理");
    expect(container.textContent ?? "").not.toContain("pack.cooking");
    expect(knowledgeTriggerLabel("brand-new-trigger")).toBe("系統");
  });
});

// ---------------------------------------------------------------------------
// safety-invariants-075：L4 不得預設整個工作階段
// ---------------------------------------------------------------------------

describe("safety-invariants-075 L4 同意只給短效授權", () => {
  const CAMERA = card({
    id: "iphone.camera",
    kind: "receptor",
    displayName: "iPhone 攝影機",
    colorRole: "input",
    consent: { required: true, reason: "會拍到你" },
    requiresConsent: true,
  });

  it("選項與預設：L4 沒有「整個工作階段」，預設是「只這一次」", () => {
    expect(consentScopeOptions(CAMERA).map((o) => o.value)).toEqual(["5", "30"]);
    expect(consentScopeOptions(CAMERA).some((o) => o.value === "session")).toBe(false);
    expect(defaultConsentScope(CAMERA)).toBe("5");
    // 一般能力維持原本的選擇。
    const notify = card({ id: "notify.desktop", consent: { required: true } });
    expect(consentScopeOptions(notify).map((o) => o.value)).toContain("session");
    expect(defaultConsentScope(notify)).toBe("session");
  });

  it("不動選單直接按同意：送出的是有到期時間的短效授權", async () => {
    vi.spyOn(api, "sessionGet").mockResolvedValue({
      sessionId: "s-1",
      state: "active",
      startedAt: "2026-09-01T00:00:00Z",
      consents: [],
    });
    vi.spyOn(api, "capabilitiesHuman").mockResolvedValue({
      ...HUMAN,
      receptors: [CAMERA],
      actuators: [],
    });
    vi.spyOn(api, "status").mockResolvedValue({ emergencyStop: false });
    vi.spyOn(api, "auditTail").mockResolvedValue([]);
    vi.spyOn(api, "uiPrefsGet").mockResolvedValue({
      mode: "simple",
      locale: "zh-TW",
      customNames: {},
      schemaVersion: "1.0",
    });
    vi.spyOn(api, "pauseGet").mockResolvedValue({ paused: false });
    const grant = vi.spyOn(api, "consentGrant").mockResolvedValue(undefined as never);

    render(
      <AppStateProvider ready refreshKey={0}>
        <SafetyPage refreshKey={0} onNavigate={() => {}} />
      </AppStateProvider>
    );
    fireEvent.click(await screen.findByRole("button", { name: /授予新權限/ }));
    const dialog = within(await screen.findByRole("dialog"));
    fireEvent.click(await dialog.findByText("iPhone 攝影機"));
    expect(dialog.queryByRole("option", { name: "整個工作階段" })).not.toBeInTheDocument();
    fireEvent.click(dialog.getByRole("button", { name: "同意" }));
    await waitFor(() => expect(grant).toHaveBeenCalledWith("receptor:iphone.camera", 5));
  });
});
