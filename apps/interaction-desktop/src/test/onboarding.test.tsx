// 首次設定精靈（v0.5，3 步）：預設保守、AI 幫手只做誠實 discovery、
// 草稿保存、commit 仍走同一 onboardingCommit 原子契約。
// 角色名稱／代詞來自 useCharacterName()（這裡 mock 成小樞／她；另有非小樞角色的中立文案測試）；
// commit 之後接可略過的「首次成功體驗」。

import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const mockName = vi.hoisted(() => ({
  current: { name: "小樞", pronoun: "她", characterId: "shu-maid", loaded: true, icon: "cat" },
}));

vi.mock("../characterName", () => ({
  useCharacterName: () => mockName.current,
  refreshCharacterName: vi.fn(async () => mockName.current),
  characterNameFallback: "角色",
}));

const mockApi = vi.hoisted(() => {
  const receptors = [
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
    {
      // 已配對 iPhone 的受器：不需同意、已啟用，但資料流向未知 → 不是新手預選項。
      // 重跑精靈時它一定不能被靜默關掉（F-007）。
      id: "iphone.motion",
      kind: "receptor",
      displayName: "iPhone 動作",
      nameSource: "catalog",
      shortDescription: "x",
      descriptionSource: "catalog",
      icon: "smartphone",
      colorRole: "input",
      category: "device",
      beginnerRecommended: false,
      badges: [],
      consent: { required: false },
      undescribed: false,
      availability: "available",
      requiresConsent: false,
      manifestHash: "h5",
      data: { personalData: true, sensitivity: "medium", source: "device", leavesDevice: "unknown", retention: "session" },
    },
  ];
  const actuators = [
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
  ];
  // 後端試算的替身：用同一份快照算「現在 → 之後」，與 Runtime 的規則一致。
  const on = (id: string) =>
    [...receptors, ...actuators].some((c) => c.id === id && c.availability !== "disabled");
  const changes = (enable: string[], disable: string[]) => [
    ...enable.map((id) => ({ id, from: on(id) ? "on" : "off", to: "on", changed: !on(id) })),
    ...disable.map((id) => ({ id, from: on(id) ? "on" : "off", to: "off", changed: on(id) })),
  ];
  return {
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
      receptors,
      actuators,
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
    onboardingPreview: vi.fn(async (commit: Record<string, unknown>) => ({
      receptors: changes(
        (commit.enableReceptors as string[]) ?? [],
        (commit.disableReceptors as string[]) ?? []
      ),
      actuators: changes(
        (commit.enableActuators as string[]) ?? [],
        (commit.disableActuators as string[]) ?? []
      ),
      starterRecipes: ((commit.starterRecipes as string[]) ?? []).map((id) => ({
        id,
        exists: false,
      })),
      policyPatch: (commit.policyPatch as Record<string, unknown>) ?? null,
      preferences: (commit.preferences as Record<string, unknown>) ?? null,
      changed: true,
    })),
    onboardingCommit: vi.fn(async () => ({ completed: true })),
    proactiveDialogueGet: vi.fn(async () => ({ mode: "necessary" })),
    proactiveDialoguePatch: vi.fn(async () => ({})),
    agentsDiscoveries: vi.fn(async () => ({
      agents: [
        { kind: "codex", found: true, loggedIn: true },
        { kind: "claude-code", found: false, detail: "未偵測到" },
      ],
    })),
  };
});

vi.mock("../api", async (importOriginal) => {
  const original = await importOriginal<Record<string, unknown>>();
  return { ...original, api: mockApi };
});

import { AppStateProvider } from "../appstate";
import { introCopy, isShuFamily, localCapabilityRows, Onboarding } from "../pages/Onboarding";
import { FIRST_SUCCESS_STORAGE_KEY } from "../pages/FirstSuccess";

function renderWizard(handlers: { onDone?: () => void; onNavigate?: (tab: string) => void } = {}) {
  return render(
    <AppStateProvider ready={true} refreshKey={0}>
      <Onboarding onDone={handlers.onDone ?? (() => {})} onSkip={() => {}} onNavigate={handlers.onNavigate} />
    </AppStateProvider>
  );
}

/** 開啟「套用前確認」對話框（不按套用）。 */
async function openConfirm() {
  await userEvent.click(await screen.findByRole("button", { name: "完成設定" }));
  return await screen.findByRole("dialog", { name: "套用前確認" });
}

/** 走完確認：完成設定 → 套用。 */
async function finish() {
  const dialog = await openConfirm();
  await userEvent.click(within(dialog).getByRole("button", { name: "套用" }));
}

function lastCommit(): Record<string, unknown> {
  const calls = (mockApi.onboardingCommit as ReturnType<typeof vi.fn>).mock.calls;
  return calls[calls.length - 1][0] as Record<string, unknown>;
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  sessionStorage.clear();
  mockName.current = { name: "小樞", pronoun: "她", characterId: "shu-maid", loaded: true, icon: "cat" };
  mockApi.onboardingGet.mockImplementation(async () => ({
    completed: false,
    draft: null,
    starterRecipes: [
      { id: "starter-task-complete", title: "任務完成時，用最低干擾方式回應" },
      { id: "starter-quiet-log", title: "安靜時段只記錄、不打擾" },
    ],
  }));
  mockApi.proactiveDialogueGet.mockImplementation(async () => ({ mode: "necessary" }));
});

describe("Onboarding (3 步)", () => {
  it("步驟一：選擇角色與陪伴方式 — 顯示預設開啟、表現程度預設自然", async () => {
    renderWizard();
    await screen.findByRole("heading", { name: "選擇角色與陪伴方式" });
    expect(screen.getByRole("checkbox", { name: /在桌面上顯示小樞/ })).toBeChecked();
    expect(screen.getByRole("radio", { name: /自然（建議）/ })).toBeChecked();
    // 3 步導覽列，名稱固定。
    const nav = screen.getByRole("navigation", { name: "設定進度" });
    expect(nav.textContent).toContain("選擇角色與陪伴方式");
    expect(nav.textContent).toContain("選擇 AI 工作方式");
    expect(nav.textContent).toContain("確認安全與權限預設");
  });

  it("步驟二：AI 幫手 — 只做誠實 discovery，預設稍後再說", async () => {
    renderWizard();
    await screen.findByRole("heading", { name: "選擇角色與陪伴方式" });
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
    await screen.findByRole("heading", { name: "選擇角色與陪伴方式" });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await waitFor(() => expect(mockApi.onboardingDraft).toHaveBeenCalled());
    const calls = (mockApi.onboardingDraft as ReturnType<typeof vi.fn>).mock.calls;
    const draft = calls[calls.length - 1][0] as { step: number };
    expect(draft.step).toBe(1);
  });

  it("步驟三：安全預設 — 保證文字齊全、攝影機不在自動啟用清單", async () => {
    renderWizard();
    await screen.findByRole("heading", { name: "選擇角色與陪伴方式" });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await screen.findByRole("heading", { name: "確認安全與權限預設" });
    expect(screen.getByText(/麥克風、攝影機、定位/)).toBeInTheDocument();
    expect(screen.getByText(/緊急停止/)).toBeInTheDocument();
    expect(screen.getByText(/能力存在不等於 AI 自動獲得權限/)).toBeInTheDocument();
    // 自動挑選摘要：低風險本機能力入選；攝影機（需同意）不在其中。
    expect(screen.getByText("任務狀態")).toBeInTheDocument();
    expect(screen.queryByText("攝影機")).not.toBeInTheDocument();
    expect(screen.queryByText(/Webhook 傳送/)).not.toBeInTheDocument();
  });

  it("commit 送出 enable 清單、保守 policy；沒改過的主動對話模式不覆寫", async () => {
    renderWizard();
    await screen.findByRole("heading", { name: "選擇角色與陪伴方式" });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await screen.findByRole("heading", { name: "確認安全與權限預設" });
    await finish();
    await waitFor(() => expect(mockApi.onboardingCommit).toHaveBeenCalled());
    const commit = lastCommit();
    expect(commit["enableReceptors"]).toContain("task.lifecycle");
    expect(commit["enableReceptors"]).not.toContain("camera.main");
    // 對外寫入未被啟用。
    expect(commit["enableActuators"]).not.toContain("webhook.output");
    const policy = commit["policyPatch"] as Record<string, unknown>;
    // 精靈沒有「主動程度」欄位：使用者沒做過的決定就不能寫進 policy。
    expect(policy).not.toHaveProperty("initiative");
    // 目前就是「必要時」，使用者也沒動它 → 不得重送覆寫。
    expect(mockApi.proactiveDialoguePatch).not.toHaveBeenCalled();
  });

  it("套用前確認：不按「套用」就不會 commit，取消也不會", async () => {
    renderWizard();
    await screen.findByRole("heading", { name: "選擇角色與陪伴方式" });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await screen.findByRole("heading", { name: "確認安全與權限預設" });
    const dialog = await openConfirm();
    // 試算是唯讀的，commit 還沒發生。
    await waitFor(() => expect(mockApi.onboardingPreview).toHaveBeenCalled());
    expect(mockApi.onboardingCommit).not.toHaveBeenCalled();
    // 第一次設定會關掉「Webhook 傳送」——必須明講，不能靜默進行。
    expect(dialog.textContent).toContain("Webhook 傳送：開啟 → 關閉");
    expect(dialog.textContent).not.toContain("以本機快照估算");
    await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: "套用前確認" })).not.toBeInTheDocument()
    );
    expect(mockApi.onboardingCommit).not.toHaveBeenCalled();
  });

  it("後端試算失敗 → 退回本機估算，畫面明說那是估算", async () => {
    mockApi.onboardingPreview.mockRejectedValueOnce(new Error("preview boom"));
    renderWizard();
    await screen.findByRole("heading", { name: "選擇角色與陪伴方式" });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await screen.findByRole("heading", { name: "確認安全與權限預設" });
    const dialog = await openConfirm();
    expect(dialog.textContent).toContain("以本機快照估算");
    expect(dialog.textContent).toContain("preview boom");
    expect(dialog.textContent).toContain("Webhook 傳送：開啟 → 關閉");
    expect(mockApi.onboardingCommit).not.toHaveBeenCalled();
  });

  it("步驟一：音效與玩耍是唯讀說明，不是第二份開關", async () => {
    renderWizard();
    await screen.findByRole("heading", { name: "選擇角色與陪伴方式" });
    expect(screen.getByText(/音效預設關閉，之後可在「小樞」頁開啟/)).toBeInTheDocument();
    expect(screen.getByText(/玩耍與游標互動：預設開啟/)).toBeInTheDocument();
    // 小樞頁才是這些設定的主人：精靈不放對應的核取方塊。
    expect(screen.queryByRole("checkbox", { name: /音效/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("checkbox", { name: /玩耍/ })).not.toBeInTheDocument();
  });

  it("預設 commit 不寫安靜時段、通道頻率與核准門檻（文案說「之後再問」就不能偷偷寫）", async () => {
    renderWizard();
    await screen.findByRole("heading", { name: "選擇角色與陪伴方式" });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await finish();
    await waitFor(() => expect(mockApi.onboardingCommit).toHaveBeenCalled());
    const policy = lastCommit()["policyPatch"] as Record<string, unknown>;
    expect(policy).not.toHaveProperty("quietHours");
    expect(policy).not.toHaveProperty("channelLimits");
    expect(policy).not.toHaveProperty("requireApprovalAt");
    // 主動程度同理：UI 上沒有這個選項，就不得靜默覆蓋 runtime 預設。
    expect(policy).not.toHaveProperty("initiative");
    expect(policy).toEqual({});
  });

  it("打開「進一步自訂」勾安靜時段後，commit 只寫 quietHours（沒有頻率上限與核准門檻）", async () => {
    renderWizard();
    await screen.findByRole("heading", { name: "選擇角色與陪伴方式" });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await screen.findByRole("heading", { name: "確認安全與權限預設" });
    await userEvent.click(screen.getByRole("checkbox", { name: /我要在這裡直接設定/ }));
    await userEvent.click(await screen.findByRole("checkbox", { name: "設定安靜時段" }));
    // F-006：一般模式沒有主人、標籤又對不上效果的兩個欄位已經從精靈拿掉。
    expect(screen.queryByRole("spinbutton", { name: /每小時/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("checkbox", { name: /高風險操作每次都先詢問/ })).not.toBeInTheDocument();
    await finish();
    await waitFor(() => expect(mockApi.onboardingCommit).toHaveBeenCalled());
    const policy = lastCommit()["policyPatch"] as Record<string, unknown>;
    const quietHours = policy["quietHours"] as { start: string; end: string; silencedChannels: string[] }[];
    expect(quietHours).toEqual([
      { start: "22:00", end: "08:00", silencedChannels: ["audio", "haptic", "notification", "light"] },
    ]);
    // 空陣列會被後端解讀成內建預設（含 desktop-pet），精靈必須送出明確清單，
    // 不能讓桌面角色在精靈建立的安靜時段裡被誤靜音（ia-settings-012）。
    expect(quietHours[0].silencedChannels).not.toContain("desktop-pet");
    // 這兩項精靈永遠不寫：一般模式看不到也改不回來。
    expect(policy).not.toHaveProperty("channelLimits");
    expect(policy).not.toHaveProperty("requireApprovalAt");
  });

  it("步驟二選 Codex → commit 寫入對應的 agent 路由偏好", async () => {
    renderWizard();
    await screen.findByRole("heading", { name: "選擇角色與陪伴方式" });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await screen.findByRole("heading", { name: "要讓小樞幫忙工作嗎？" });
    await userEvent.click(screen.getByRole("radio", { name: "用 Codex 幫忙" }));
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await finish();
    await waitFor(() => expect(mockApi.onboardingCommit).toHaveBeenCalled());
    const commit = lastCommit();
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
    await screen.findByRole("heading", { name: "選擇角色與陪伴方式" });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await finish();
    await waitFor(() => expect(mockApi.onboardingCommit).toHaveBeenCalled());
    expect(lastCommit()["preferences"]).toEqual({ locale: "zh-TW" });
  });

  it("commit 失敗誠實顯示錯誤，不宣稱完成", async () => {
    mockApi.onboardingCommit.mockRejectedValueOnce(new Error("commit boom"));
    renderWizard();
    await screen.findByRole("heading", { name: "選擇角色與陪伴方式" });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await finish();
    expect(await screen.findByRole("alert")).toHaveTextContent("套用失敗");
    expect(mockApi.proactiveDialoguePatch).not.toHaveBeenCalled();
    // 失敗就不會出現首次成功體驗。
    expect(screen.queryByRole("dialog", { name: "首次成功體驗" })).not.toBeInTheDocument();
  });

  it("第一次設定：保守建議值（必要時）仍然套用，但差異必須先列在確認畫面上", async () => {
    mockApi.proactiveDialogueGet.mockImplementation(async () => ({ mode: "natural" }));
    renderWizard();
    await screen.findByRole("heading", { name: "選擇角色與陪伴方式" });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await screen.findByRole("heading", { name: "確認安全與權限預設" });
    expect(screen.getByRole("radio", { name: /必要時（建議）/ })).toBeChecked();
    const dialog = await openConfirm();
    expect(dialog.textContent).toContain("小樞主動說話：自然 → 必要時");
    expect(mockApi.proactiveDialoguePatch).not.toHaveBeenCalled();
    await userEvent.click(within(dialog).getByRole("button", { name: "套用" }));
    await waitFor(() =>
      expect(mockApi.proactiveDialoguePatch).toHaveBeenCalledWith({ mode: "necessary" })
    );
  });
});

// ---------------------------------------------------------------------------
// 重新執行首次設定（F-007）：不得靜默停用既有能力，也不得覆寫後來的調整
// ---------------------------------------------------------------------------

describe("Onboarding：重新執行", () => {
  beforeEach(() => {
    mockApi.onboardingGet.mockImplementation(async () => ({
      completed: true,
      draft: null,
      starterRecipes: [
        { id: "starter-task-complete", title: "任務完成時，用最低干擾方式回應" },
        { id: "starter-quiet-log", title: "安靜時段只記錄、不打擾" },
      ],
    }));
  });

  async function walkToConfirm() {
    await screen.findByRole("heading", { name: "選擇角色與陪伴方式" });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await screen.findByRole("heading", { name: "確認安全與權限預設" });
    return await openConfirm();
  }

  it("已啟用但非新手預選的能力（已配對 iPhone 的動作感測）不會被靜默關掉", async () => {
    renderWizard();
    const dialog = await walkToConfirm();
    // 確認畫面沒有任何要關掉的東西。
    expect(dialog.textContent).not.toContain("iPhone 動作");
    expect(dialog.textContent).not.toContain("Webhook 傳送");
    expect(dialog.textContent).toContain("沒有任何變更");
    await userEvent.click(within(dialog).getByRole("button", { name: "套用" }));
    await waitFor(() => expect(mockApi.onboardingCommit).toHaveBeenCalled());
    const commit = lastCommit();
    expect(commit["disableReceptors"]).toEqual([]);
    expect(commit["disableActuators"]).toEqual([]);
    // 已啟用的能力仍在 enable 清單裡（後端會判定為「沒變」而跳過）。
    expect(commit["enableReceptors"]).toEqual(["task.lifecycle", "iphone.motion"]);
  });

  it("重跑不重裝起步範本、不重寫顯示語言，也不重送沒改過的主動說話模式", async () => {
    renderWizard();
    const dialog = await walkToConfirm();
    await userEvent.click(within(dialog).getByRole("button", { name: "套用" }));
    await waitFor(() => expect(mockApi.onboardingCommit).toHaveBeenCalled());
    const commit = lastCommit();
    expect(commit["starterRecipes"]).toEqual([]);
    expect(commit).not.toHaveProperty("preferences");
    expect(mockApi.proactiveDialoguePatch).not.toHaveBeenCalled();
  });

  it("重跑：主動說話顯示目前真值；不動就不送，改了才列出來並在套用後送出", async () => {
    mockApi.proactiveDialogueGet.mockImplementation(async () => ({ mode: "natural" }));
    renderWizard();
    await screen.findByRole("heading", { name: "選擇角色與陪伴方式" });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await screen.findByRole("heading", { name: "確認安全與權限預設" });
    // 顯示目前真值（自然），不是草稿預設值（必要時）。
    await waitFor(() => expect(screen.getByRole("radio", { name: /^自然——/ })).toBeChecked());
    // 不動它 → 沒有這一列。
    let dialog = await openConfirm();
    expect(dialog.textContent).not.toContain("主動說話");
    await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    // 改成「必要時」→ 列出來，套用後才送。
    await userEvent.click(screen.getByRole("radio", { name: /必要時（建議）/ }));
    dialog = await openConfirm();
    expect(dialog.textContent).toContain("小樞主動說話：自然 → 必要時");
    expect(mockApi.proactiveDialoguePatch).not.toHaveBeenCalled();
    await userEvent.click(within(dialog).getByRole("button", { name: "套用" }));
    await waitFor(() =>
      expect(mockApi.proactiveDialoguePatch).toHaveBeenCalledWith({ mode: "necessary" })
    );
  });

  it("重跑的步驟三文案說明「不會自動關掉任何一項」", async () => {
    renderWizard();
    await screen.findByRole("heading", { name: "選擇角色與陪伴方式" });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await screen.findByRole("heading", { name: "確認安全與權限預設" });
    expect(screen.getByText(/重新執行不會自動關掉任何一項/)).toBeInTheDocument();
  });

  it("localCapabilityRows：只列真的會變的項目，需要同意的能力永遠不列", () => {
    const human = {
      receptors: [
        { id: "a", displayName: "甲", availability: "available", requiresConsent: false },
        { id: "b", displayName: "乙", availability: "disabled", requiresConsent: false },
        { id: "c", displayName: "丙", availability: "disabled", requiresConsent: true },
      ],
      actuators: [],
    } as unknown as Parameters<typeof localCapabilityRows>[0];
    // 甲維持開、乙要開、丙（需同意）即使被選也不列。
    const rows = localCapabilityRows(human, ["a", "b", "c"], []);
    expect(rows).toEqual([{ key: "receptor:b", label: "乙", from: "關閉", to: "開啟" }]);
    // 甲被取消勾選 → 列出「開啟 → 關閉」。
    expect(localCapabilityRows(human, [], [])).toEqual([
      { key: "receptor:a", label: "甲", from: "開啟", to: "關閉" },
    ]);
  });
});

describe("Onboarding：角色名稱與代詞", () => {
  it("小樞：文案用 manifest 代詞「她」與貓系數位精靈介紹，步驟列與第二步標題都用角色名", async () => {
    renderWizard();
    await screen.findByRole("heading", { name: "選擇角色與陪伴方式" });
    expect(screen.getByText(/小樞是住在你桌面上的貓系數位精靈。她會眨眼/)).toBeInTheDocument();
    // 小樞家族走 rig 預覽（jsdom 沒有 canvas 只會顯示載入失敗），不是中立文字說明。
    expect(screen.queryByRole("note")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await screen.findByRole("heading", { name: "要讓小樞幫忙工作嗎？" });
    expect(screen.getByText(/小樞可以把任務交給本機的 AI 幫手/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await screen.findByRole("heading", { name: "確認安全與權限預設" });
    expect(screen.getByText("小樞主動說話")).toBeInTheDocument();
  });

  it("非小樞角色：用角色名與中立代詞，沒有物種／服裝文案、沒有小樞 rig 預覽", async () => {
    mockName.current = { name: "阿寶", pronoun: "角色", characterId: "buddy", loaded: true, icon: "sparkles" };
    renderWizard();
    await screen.findByRole("heading", { name: "選擇角色與陪伴方式" });
    const nav = screen.getByRole("navigation", { name: "設定進度" });
    expect(nav.textContent).toContain("選擇角色與陪伴方式");
    expect(nav.textContent).not.toContain("小樞");
    expect(screen.getByText(/阿寶是住在你桌面上的角色/)).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: /在桌面上顯示阿寶/ })).toBeChecked();
    expect(screen.getByRole("note")).toHaveTextContent(/阿寶會依角色自己宣告的方式出現/);
    expect(screen.queryByRole("img", { name: /預覽/ })).not.toBeInTheDocument();
    const body = document.body.textContent ?? "";
    expect(body).not.toMatch(/貓系|女僕|精靈|小樞/);
    expect(body).not.toContain("她");
    // 玩耍是小樞的功能，不對其他角色宣稱。
    expect(screen.queryByText(/玩耍與游標互動/)).not.toBeInTheDocument();
    expect(screen.getByText(/音效預設關閉，之後可在「阿寶」頁開啟/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await screen.findByRole("heading", { name: "要讓阿寶幫忙工作嗎？" });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await screen.findByRole("heading", { name: "確認安全與權限預設" });
    expect(screen.getByText("阿寶主動說話")).toBeInTheDocument();
  });

  it("角色載入失敗 → 中立的「角色」", async () => {
    mockName.current = { name: "角色", pronoun: "角色", characterId: null as unknown as string, loaded: false, icon: "sparkles" };
    renderWizard();
    await screen.findByRole("heading", { name: "選擇角色與陪伴方式" });
    expect(screen.getByRole("checkbox", { name: /在桌面上顯示角色/ })).toBeInTheDocument();
    expect(document.body.textContent).not.toContain("小樞");
  });

  it("introCopy／isShuFamily：只有 shu-* 家族用物種文案", () => {
    expect(isShuFamily("shu-maid")).toBe(true);
    expect(isShuFamily("shu-standard")).toBe(true);
    expect(isShuFamily("buddy")).toBe(false);
    expect(isShuFamily(null)).toBe(false);
    expect(introCopy("小樞", "她", true)).toContain("貓系數位精靈");
    const neutral = introCopy("阿寶", "角色", false);
    expect(neutral).not.toMatch(/貓|女僕|精靈|她/);
    expect(neutral).toContain("阿寶");
  });
});

describe("Onboarding：首次成功體驗（commit 之後、可略過）", () => {
  async function completeWizard() {
    await screen.findByRole("heading", { name: "選擇角色與陪伴方式" });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await finish();
    await waitFor(() => expect(mockApi.onboardingCommit).toHaveBeenCalled());
  }

  it("完成設定後出現「準備好了。要不要先試一次？」，不是第四步；稍後再說才 onDone", async () => {
    const onDone = vi.fn();
    renderWizard({ onDone });
    await completeWizard();
    const dialog = await screen.findByRole("dialog", { name: "首次成功體驗" });
    expect(dialog).toHaveTextContent("小樞準備好了。要不要先試一次？");
    // 精靈的三步進度列已經不在；這一屏不是精靈的一步。
    expect(screen.queryByRole("navigation", { name: "設定進度" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "完成設定" })).not.toBeInTheDocument();
    expect(onDone).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "稍後再說" }));
    await waitFor(() => expect(onDone).toHaveBeenCalledTimes(1));
    // 看過旗標：host 沒回傳就退回本機。
    expect(mockApi.uiPrefsPatch).toHaveBeenCalledWith({ firstSuccessSeen: true });
    expect(localStorage.getItem(FIRST_SUCCESS_STORAGE_KEY)).toBe("1");
  });

  it("交代一件小工作 → 預填並前往工作頁", async () => {
    const onDone = vi.fn();
    const onNavigate = vi.fn();
    renderWizard({ onDone, onNavigate });
    await completeWizard();
    await screen.findByRole("dialog", { name: "首次成功體驗" });
    await userEvent.click(screen.getByRole("button", { name: /交代一件小工作/ }));
    await waitFor(() => expect(onDone).toHaveBeenCalledTimes(1));
    expect(onNavigate).toHaveBeenCalledWith("work");
    expect(sessionStorage.getItem("work.prefill")).toBeTruthy();
  });

  it("已經看過 → 完成設定直接 onDone，不再打擾", async () => {
    localStorage.setItem(FIRST_SUCCESS_STORAGE_KEY, "1");
    const onDone = vi.fn();
    renderWizard({ onDone });
    await completeWizard();
    await waitFor(() => expect(onDone).toHaveBeenCalledTimes(1));
    expect(screen.queryByRole("dialog", { name: "首次成功體驗" })).not.toBeInTheDocument();
  });
});
