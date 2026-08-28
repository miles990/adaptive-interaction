// 首次設定精靈（v0.5，3 步）：預設保守、AI 幫手只做誠實 discovery、
// 草稿保存、commit 仍走同一 onboardingCommit 原子契約。

import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const mockApi = vi.hoisted(() => ({
  uiPrefsGet: vi.fn(async () => ({
    mode: "simple",
    locale: "zh-TW",
    customNames: {},
    schemaVersion: "1.0",
  })),
  uiPrefsPatch: vi.fn(),
  pauseGet: vi.fn(async () => ({ paused: false })),
  capabilitiesHuman: vi.fn(async () => ({
    locale: "zh-TW",
    catalogVersion: 1,
    capabilityVersion: 1,
    generatedAt: "",
    constraints: [],
    receptors: [
      {
        id: "task.lifecycle",
        kind: "receptor",
        displayName: "任務狀態",
        nameSource: "catalog",
        shortDescription: "x",
        descriptionSource: "catalog",
        icon: "list-checks",
        colorRole: "input",
        category: "task",
        beginnerRecommended: true,
        badges: [],
        consent: { required: false },
        undescribed: false,
        availability: "available",
        requiresConsent: false,
        manifestHash: "h1",
        data: { personalData: false, sensitivity: "none", source: "local", leavesDevice: false, retention: "session" },
      },
      {
        id: "camera.main",
        kind: "receptor",
        displayName: "攝影機",
        nameSource: "catalog",
        shortDescription: "x",
        descriptionSource: "catalog",
        icon: "video",
        colorRole: "input",
        category: "sensor",
        beginnerRecommended: false,
        badges: [],
        consent: { required: true },
        undescribed: false,
        availability: "disabled",
        requiresConsent: true,
        manifestHash: "h2",
        data: { personalData: true, sensitivity: "high", source: "device", leavesDevice: "unknown", retention: "unknown" },
      },
    ],
    actuators: [
      {
        id: "conversation",
        kind: "actuator",
        displayName: "對話訊息",
        nameSource: "catalog",
        shortDescription: "x",
        descriptionSource: "catalog",
        icon: "message-square",
        colorRole: "output",
        category: "message",
        beginnerRecommended: true,
        badges: [],
        consent: { required: false },
        undescribed: false,
        availability: "available",
        requiresConsent: false,
        manifestHash: "h3",
        effect: { externalSideEffect: false, physicalEffect: false, interruptiveness: "low", reversible: true, confirmationLevel: "delivered" },
      },
      {
        id: "webhook.output",
        kind: "actuator",
        displayName: "Webhook 傳送",
        nameSource: "catalog",
        shortDescription: "x",
        descriptionSource: "catalog",
        icon: "cloud-upload",
        colorRole: "output",
        category: "integration",
        beginnerRecommended: false,
        badges: [],
        consent: { required: false },
        undescribed: false,
        availability: "available",
        requiresConsent: false,
        manifestHash: "h4",
        effect: { externalSideEffect: true, physicalEffect: false, interruptiveness: "none", reversible: false, confirmationLevel: "acknowledged" },
      },
    ],
    toolOperations: [],
  })),
  onboardingGet: vi.fn(async () => ({
    completed: false,
    draft: null,
    starterRecipes: [
      { id: "starter-task-complete", title: "任務完成時，用最低干擾方式回應" },
      { id: "starter-quiet-log", title: "安靜時段只記錄、不打擾" },
    ],
  })),
  onboardingDraft: vi.fn(async () => ({})),
  onboardingCommit: vi.fn(async () => ({ completed: true })),
  proactiveDialoguePatch: vi.fn(async () => ({})),
  agentsDiscoveries: vi.fn(async () => ({
    agents: [
      { kind: "codex", found: true, loggedIn: true },
      { kind: "claude-code", found: false, detail: "未偵測到" },
    ],
  })),
}));

vi.mock("../api", async (importOriginal) => {
  const original = await importOriginal<Record<string, unknown>>();
  return { ...original, api: mockApi };
});

import { AppStateProvider } from "../appstate";
import { Onboarding } from "../pages/Onboarding";

function renderWizard() {
  return render(
    <AppStateProvider ready={true} refreshKey={0}>
      <Onboarding onDone={() => {}} onSkip={() => {}} />
    </AppStateProvider>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("Onboarding (3 步)", () => {
  it("步驟一：認識小樞 — 顯示預設開啟、表現程度預設自然", async () => {
    renderWizard();
    await screen.findByRole("heading", { name: "認識小樞" });
    expect(screen.getByRole("checkbox", { name: /在桌面上顯示小樞/ })).toBeChecked();
    expect(screen.getByRole("radio", { name: /自然（建議）/ })).toBeChecked();
    // 3 步導覽列。
    const nav = screen.getByRole("navigation", { name: "設定進度" });
    expect(nav.textContent).toContain("認識小樞");
    expect(nav.textContent).toContain("AI 幫手");
    expect(nav.textContent).toContain("安全預設");
  });

  it("步驟二：AI 幫手 — 只做誠實 discovery，預設稍後再說", async () => {
    renderWizard();
    await screen.findByRole("heading", { name: "認識小樞" });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await screen.findByRole("heading", { name: "要讓小樞幫忙工作嗎？" });
    await waitFor(() => expect(mockApi.agentsDiscoveries).toHaveBeenCalled());
    expect(await screen.findByText(/codex：已安裝、已登入/)).toBeInTheDocument();
    expect(screen.getByText(/claude-code：未偵測到/)).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "稍後再說" })).toBeChecked();
    // 這一步不得授權任何工作區寫入。
    expect(screen.getByText(/實際建立工作/)).toBeInTheDocument();
  });

  it("草稿隨步驟前進保存", async () => {
    renderWizard();
    await screen.findByRole("heading", { name: "認識小樞" });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await waitFor(() => expect(mockApi.onboardingDraft).toHaveBeenCalled());
    const calls = (mockApi.onboardingDraft as ReturnType<typeof vi.fn>).mock.calls;
    const draft = calls[calls.length - 1][0] as { step: number };
    expect(draft.step).toBe(1);
  });

  it("步驟三：安全預設 — 保證文字齊全、攝影機不在自動啟用清單", async () => {
    renderWizard();
    await screen.findByRole("heading", { name: "認識小樞" });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await screen.findByRole("heading", { name: "安全預設" });
    expect(screen.getByText(/麥克風、攝影機、定位/)).toBeInTheDocument();
    expect(screen.getByText(/緊急停止/)).toBeInTheDocument();
    expect(screen.getByText(/能力存在不等於 AI 自動獲得權限/)).toBeInTheDocument();
    // 自動挑選摘要：低風險本機能力入選；攝影機（需同意）不在其中。
    expect(screen.getByText("任務狀態")).toBeInTheDocument();
    expect(screen.queryByText("攝影機")).not.toBeInTheDocument();
    expect(screen.queryByText(/Webhook 傳送/)).not.toBeInTheDocument();
  });

  it("commit 送出 enable 清單、保守 policy 與主動對話模式", async () => {
    renderWizard();
    await screen.findByRole("heading", { name: "認識小樞" });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await screen.findByRole("heading", { name: "安全預設" });
    await userEvent.click(screen.getByRole("button", { name: "完成設定" }));
    await waitFor(() => expect(mockApi.onboardingCommit).toHaveBeenCalled());
    const commit = (mockApi.onboardingCommit as ReturnType<typeof vi.fn>).mock
      .calls[0][0] as Record<string, unknown>;
    expect(commit["enableReceptors"]).toContain("task.lifecycle");
    expect(commit["enableReceptors"]).not.toContain("camera.main");
    // 對外寫入未被啟用。
    expect(commit["enableActuators"]).not.toContain("webhook.output");
    const policy = commit["policyPatch"] as Record<string, unknown>;
    // 精靈沒有「主動程度」欄位：使用者沒做過的決定就不能寫進 policy。
    expect(policy).not.toHaveProperty("initiative");
    // 主動對話預設「必要」。
    await waitFor(() =>
      expect(mockApi.proactiveDialoguePatch).toHaveBeenCalledWith({ mode: "necessary" })
    );
  });

  it("步驟一：音效與玩耍是唯讀說明，不是第二份開關", async () => {
    renderWizard();
    await screen.findByRole("heading", { name: "認識小樞" });
    expect(screen.getByText(/音效預設關閉，之後可在「小樞」頁開啟/)).toBeInTheDocument();
    expect(screen.getByText(/玩耍與游標互動：預設開啟/)).toBeInTheDocument();
    // 小樞頁才是這些設定的主人：精靈不放對應的核取方塊。
    expect(screen.queryByRole("checkbox", { name: /音效/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("checkbox", { name: /玩耍/ })).not.toBeInTheDocument();
  });

  it("預設 commit 不寫安靜時段、通道頻率與核准門檻（文案說「之後再問」就不能偷偷寫）", async () => {
    renderWizard();
    await screen.findByRole("heading", { name: "認識小樞" });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(await screen.findByRole("button", { name: "完成設定" }));
    await waitFor(() => expect(mockApi.onboardingCommit).toHaveBeenCalled());
    const commit = (mockApi.onboardingCommit as ReturnType<typeof vi.fn>).mock
      .calls[0][0] as Record<string, unknown>;
    const policy = commit["policyPatch"] as Record<string, unknown>;
    expect(policy).not.toHaveProperty("quietHours");
    expect(policy).not.toHaveProperty("channelLimits");
    expect(policy).not.toHaveProperty("requireApprovalAt");
    // 主動程度同理：UI 上沒有這個選項，就不得靜默覆蓋 runtime 預設。
    expect(policy).not.toHaveProperty("initiative");
    expect(policy).toEqual({});
  });

  it("打開「進一步自訂」勾安靜時段後，commit 才會寫 quietHours", async () => {
    renderWizard();
    await screen.findByRole("heading", { name: "認識小樞" });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await screen.findByRole("heading", { name: "安全預設" });
    await userEvent.click(screen.getByRole("checkbox", { name: /我要在這裡直接設定/ }));
    await userEvent.click(await screen.findByRole("checkbox", { name: "設定安靜時段" }));
    await userEvent.click(screen.getByRole("button", { name: "完成設定" }));
    await waitFor(() => expect(mockApi.onboardingCommit).toHaveBeenCalled());
    const commit = (mockApi.onboardingCommit as ReturnType<typeof vi.fn>).mock
      .calls[0][0] as Record<string, unknown>;
    const policy = commit["policyPatch"] as Record<string, unknown>;
    expect(policy["quietHours"]).toEqual([
      { start: "22:00", end: "08:00", silencedChannels: [] },
    ]);
    expect(policy["requireApprovalAt"]).toBe("high");
  });

  it("步驟二選 Codex → commit 寫入對應的 agent 路由偏好", async () => {
    renderWizard();
    await screen.findByRole("heading", { name: "認識小樞" });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await screen.findByRole("heading", { name: "要讓小樞幫忙工作嗎？" });
    await userEvent.click(screen.getByRole("radio", { name: "用 Codex 幫忙" }));
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(await screen.findByRole("button", { name: "完成設定" }));
    await waitFor(() => expect(mockApi.onboardingCommit).toHaveBeenCalled());
    const commit = (mockApi.onboardingCommit as ReturnType<typeof vi.fn>).mock
      .calls[0][0] as Record<string, unknown>;
    expect(commit["preferences"]).toEqual({
      locale: "zh-TW",
      agentRoutes: {
        conversation: "codex",
        programming: "codex",
        knowledge: "codex",
        review: "codex",
      },
    });
  });

  it("步驟二選「稍後再說」→ commit 完全不動路由偏好", async () => {
    renderWizard();
    await screen.findByRole("heading", { name: "認識小樞" });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(await screen.findByRole("button", { name: "完成設定" }));
    await waitFor(() => expect(mockApi.onboardingCommit).toHaveBeenCalled());
    const commit = (mockApi.onboardingCommit as ReturnType<typeof vi.fn>).mock
      .calls[0][0] as Record<string, unknown>;
    expect(commit["preferences"]).toEqual({ locale: "zh-TW" });
  });

  it("commit 失敗誠實顯示錯誤，不宣稱完成", async () => {
    mockApi.onboardingCommit.mockRejectedValueOnce(new Error("commit boom"));
    renderWizard();
    await screen.findByRole("heading", { name: "認識小樞" });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(await screen.findByRole("button", { name: "完成設定" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("套用失敗");
    expect(mockApi.proactiveDialoguePatch).not.toHaveBeenCalled();
  });
});
