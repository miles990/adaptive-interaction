// N4：React 只呈現 Tauri application use case 的結果。
// 真正兩儲存寫入/故障/restart測試在 src-tauri/src/preset_service.rs。
//
// 套用一個檔位＝兩段寫入（桌面偏好 → 後端主動說話模式）。中間任何一段失敗、
// 回應遺失、或程式被關掉，畫面都不得只留下一個「自訂」讓使用者自己猜：
//   - 第一段成功、第二段沒送到 → 說出來，並且可以只補送第二段；
//   - 回應遺失 → 先讀回，讀回等於目標就是完成（不重送、不謊報失敗）；
//   - 重開之後 marker 還在且使用者沒改過 → 自動補送一次；改過就清掉 marker，
//     絕不用過時的意圖覆蓋使用者剛選的設定；
//   - 讀不回有效值 → `unverified`：不高亮任何檔位、明說無法確認。
//
// 這些測試釘住語意與安全，不是逐字文案。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const MANIFEST_TEXTS = import.meta.glob("../../public/characters/*/manifest.json", {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;

function bundled(id: string): string {
  const key = Object.keys(MANIFEST_TEXTS).find((k) => k.endsWith(`/characters/${id}/manifest.json`));
  if (!key) throw new Error(`bundled manifest missing: ${id}`);
  return MANIFEST_TEXTS[key];
}

const INDEX = {
  schemaVersion: "1.0",
  default: "shu-maid",
  characters: [
    { characterId: "shu-maid", manifestPath: "/characters/shu-maid/manifest.json", origin: "builtin" },
    { characterId: "plain-text", manifestPath: "/characters/plain-text/manifest.json", origin: "builtin" },
  ],
};

const FILES: Record<string, string> = {
  "/characters/index.json": JSON.stringify(INDEX),
  "/characters/shu-maid/manifest.json": bundled("shu-maid"),
  "/characters/plain-text/manifest.json": bundled("plain-text"),
};

const BASE_PREFS: Record<string, unknown> = {
  closeBehavior: null,
  askOnClose: true,
  launchAtLogin: false,
  showCompanionOnStart: true,
  openControlCenterOnStart: false,
  companionVisible: true,
  companionPosition: null,
  companionSize: [200, 210],
  companionOpacity: 1,
  companionPack: "shu-maid",
  companionPersona: "persona-shu",
  companionExpressiveness: "natural",
  companionAlwaysOnTop: true,
  storyProgress: {},
  companionName: "",
  companionScene: "none",
  companionPlay: true,
  companionCursorPlay: true,
  companionApproach: true,
  companionDeskMove: true,
  companionFamiliars: [],
  companionDoNotDisturb: false,
  companionBubbles: true,
  companionSound: false,
  companionDragEnabled: true,
  companionProactiveQuietUntil: 0,
  companionPendingPresetOp: null,
  companionPresetRevision: "0",
  schemaVersion: 3,
};

const PROACTIVE_CONFIG = {
  mode: "natural",
  maxPerHour: 3,
  minIntervalMinutes: 12,
  dailyGenerativeSessions: 8,
  dailyGenerativeCostUsd: 1,
  generativeAgent: null as string | null,
  noFollowUp: true,
  dndDefer: true,
};

const mockApi = vi.hoisted(() => ({
  uiPrefsGet: vi.fn(async () => ({ mode: "simple", locale: "zh-TW", customNames: {}, schemaVersion: "1.0" })),
  uiPrefsPatch: vi.fn(async () => ({ mode: "simple", locale: "zh-TW", customNames: {}, schemaVersion: "1.0" })),
  pauseGet: vi.fn(async () => ({ paused: false })),
  capabilitiesHuman: vi.fn(async () => ({
    locale: "zh-TW",
    catalogVersion: 1,
    capabilityVersion: 1,
    generatedAt: "",
    constraints: [],
    receptors: [],
    actuators: [],
    toolOperations: [],
  })),
  presentationStatus: vi.fn(async () => ({ connected: false, visible: false, pendingCommands: 0 })),
  characterInstances: vi.fn(async () => ({ instances: [] as Record<string, unknown>[] })),
  proactiveDialogueGet: vi.fn(async () => ({ config: { ...PROACTIVE_CONFIG }, sentThisHour: 0 })),
  proactiveDialoguePatch: vi.fn(async (patch: Record<string, unknown>) => ({
    config: { ...PROACTIVE_CONFIG, ...patch },
    sentThisHour: 0,
  })),
  proactiveDialogueQuiet: vi.fn(async () => ({ config: { ...PROACTIVE_CONFIG }, sentThisHour: 0 })),
  agentsDiscoveries: vi.fn(async () => ({ agents: [] })),
  policyGet: vi.fn(async () => ({ initiative: "suggest", quietHours: [] })),
  policyPatch: vi.fn(async () => ({})),
}));

vi.mock("../api", async (importOriginal) => {
  const original = await importOriginal<Record<string, unknown>>();
  return { ...original, api: mockApi };
});

const mockDesktop = vi.hoisted(() => {
  const state: { prefs: Record<string, unknown> } = { prefs: {} };
  const applyPrefsPatch = async (patch: Record<string, unknown>) => {
    Object.assign(state.prefs, patch);
    return { ...state.prefs };
  };
  return {
    state,
    applyPrefsPatch,
    prefsGet: vi.fn(async () => ({ ...state.prefs })),
    prefsPatch: vi.fn(applyPrefsPatch),
    presetApply: vi.fn<(request?: unknown) => Promise<Record<string, unknown>>>(),
    companionApplyPrefs: vi.fn(async () => null),
    companionResetPosition: vi.fn(async () => null),
    characterListImported: vi.fn(async () => [] as Record<string, unknown>[]),
    characterImport: vi.fn(async () => ({ characterId: "x", displayName: {}, report: {}, assets: [] })),
    characterRemove: vi.fn(async (characterId: string) => ({ removed: characterId })),
    characterAsset: vi.fn(async () => ""),
  };
});

vi.mock("../desktop", async (importOriginal) => {
  const original = await importOriginal<Record<string, unknown>>();
  return { ...original, isTauri: true, desktop: mockDesktop };
});

const mockName = vi.hoisted(() => ({
  current: { name: "小樞", pronoun: "她", characterId: "shu-maid", loaded: true, icon: "cat" },
}));

vi.mock("../characterName", () => ({
  useCharacterName: () => mockName.current,
  refreshCharacterName: vi.fn(async () => mockName.current),
  characterNameFallback: "角色",
}));

import { AppStateProvider } from "../appstate";
import { CompanionPage } from "../pages/CompanionPage";


function renderPage() {
  return render(
    <AppStateProvider ready={true} refreshKey={0}>
      <CompanionPage refreshKey={0} />
    </AppStateProvider>
  );
}

async function ready() {
  await screen.findByRole("group", { name: "陪伴方式" });
}

function presetButtons() {
  return within(screen.getByRole("group", { name: "陪伴方式" })).getAllByRole("button");
}

/** 目前被高亮（aria-pressed）的檔位；沒有就是 null。 */
function highlighted(): string | null {
  const on = presetButtons().find((b) => b.getAttribute("aria-pressed") === "true");
  return on?.textContent ?? null;
}

function summaryText(): string {
  return screen.getByTestId("companion-preset-summary").textContent ?? "";
}


function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  mockDesktop.state.prefs = { ...BASE_PREFS };
  mockName.current = { name: "小樞", pronoun: "她", characterId: "shu-maid", loaded: true, icon: "cat" };
  mockDesktop.prefsPatch.mockImplementation(mockDesktop.applyPrefsPatch);
  mockDesktop.prefsGet.mockImplementation(async () => ({ ...mockDesktop.state.prefs }));
  mockDesktop.presetApply.mockImplementation(async () => hostResult("applied"));
  mockApi.proactiveDialogueGet.mockImplementation(async () => ({
    config: { ...PROACTIVE_CONFIG },
    sentThisHour: 0,
  }));
  mockApi.proactiveDialoguePatch.mockImplementation(async (patch: Record<string, unknown>) => ({
    config: { ...PROACTIVE_CONFIG, ...patch },
    sentThisHour: 0,
  }));
  mockDesktop.characterListImported.mockResolvedValue([]);
  mockApi.characterInstances.mockResolvedValue({ instances: [] });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string) => {
      const body = FILES[url];
      if (body === undefined) return { ok: false, status: 404, text: async () => "", json: async () => ({}) };
      return { ok: true, status: 200, text: async () => body, json: async () => JSON.parse(body) };
    })
  );
});

afterEach(() => vi.unstubAllGlobals());

const PENDING = { opId: "saved-op", presetId: "quiet", proactivePatch: { mode: "necessary" }, issuedAtMs: 1 };
function hostResult(status: string, extra: Record<string, unknown> = {}) {
  const pending = status === "partially-applied" || status === "unverified";
  return {
    status,
    prefs: { ...BASE_PREFS, companionExpressiveness: "quiet", companionDoNotDisturb: true, companionPendingPresetOp: pending ? PENDING : null, companionPresetRevision: "1" },
    proactive: { config: { ...PROACTIVE_CONFIG, mode: pending ? "natural" : "necessary" }, sentThisHour: 0 },
    error: pending ? "桌面設定已保存，主動對話尚未完成套用。" : null,
    cleanupPending: pending,
    ...extra,
  };
}

describe("Tauri use case 的狀態投影", () => {
  it("只送預設ID、唯一operation ID及已讀取的偏好版本；React不再寫兩份store", async () => {
    renderPage(); await ready();
    await userEvent.click(screen.getByRole("button", { name: "安靜" }));
    await waitFor(() => expect(mockDesktop.presetApply).toHaveBeenCalledTimes(1));
    expect(mockDesktop.presetApply.mock.calls[0][0]).toEqual({ presetId: "quiet", operationId: expect.any(String), expectedPrefsRevision: "0" });
    expect(mockDesktop.prefsPatch).not.toHaveBeenCalled();
    expect(mockApi.proactiveDialoguePatch).not.toHaveBeenCalled();
    await waitFor(() => expect(highlighted()).toBe("安靜"));
  });

  it("第二段失敗顯示半套用與有效值，不高亮；補送交同一host use case", async () => {
    mockDesktop.presetApply.mockResolvedValueOnce(hostResult("partially-applied"));
    renderPage(); await ready();
    await userEvent.click(screen.getByRole("button", { name: "安靜" }));
    const partial = await screen.findByTestId("companion-preset-partial");
    expect(highlighted()).toBeNull();
    expect(summaryText()).toContain("主動說話：自然");
    await userEvent.click(within(partial).getByRole("button", { name: "補送" }));
    await waitFor(() => expect(highlighted()).toBe("安靜"));
    expect(mockDesktop.presetApply.mock.calls[1][0]).toBeNull();
    expect(mockApi.proactiveDialoguePatch).not.toHaveBeenCalled();
  });

  it("讀回失敗即使舊畫面吻合也不高亮", async () => {
    mockDesktop.presetApply.mockResolvedValueOnce(hostResult("unverified", { proactive: null, error: "無法確認生效值" }));
    renderPage(); await ready();
    await userEvent.click(screen.getByRole("button", { name: "安靜" }));
    await waitFor(() => expect(summaryText()).toContain("無法確認"));
    expect(highlighted()).toBeNull();
  });

  it("清marker失敗但有效值已確認：完整套用與清理提示同時保留", async () => {
    mockDesktop.presetApply.mockResolvedValueOnce(hostResult("applied", { error: "有效值已確認，恢復標記將在下次啟動時再清理。", cleanupPending: true }));
    renderPage(); await ready();
    await userEvent.click(screen.getByRole("button", { name: "安靜" }));
    await waitFor(() => expect(highlighted()).toBe("安靜"));
    expect(screen.getByText(/下次啟動時再清理/)).toBeInTheDocument();
  });

  it("舊marker在mount只要求host恢復一次，UI不自行重建patch", async () => {
    mockDesktop.state.prefs = { ...BASE_PREFS, companionPendingPresetOp: PENDING };
    renderPage(); await ready();
    await waitFor(() => expect(mockDesktop.presetApply).toHaveBeenCalledWith(null));
    await waitFor(() => expect(highlighted()).toBe("安靜"));
    expect(mockDesktop.presetApply).toHaveBeenCalledTimes(1);
    expect(mockApi.proactiveDialoguePatch).not.toHaveBeenCalled();
  });

  it("舊/損壞marker無法安全恢復時，呈現unverified而不猜模式", async () => {
    mockDesktop.state.prefs = { ...BASE_PREFS, companionPendingPresetOp: PENDING };
    mockDesktop.presetApply.mockResolvedValueOnce(hostResult("unverified", { error: "舊版恢復標記無法確認設定是否曾變更，請重新選擇陪伴方式。" }));
    renderPage(); await ready();
    await screen.findByText(/請重新選擇陪伴方式/);
    expect(highlighted()).toBeNull();
    expect(mockDesktop.presetApply).toHaveBeenCalledTimes(1);
  });

  it("恢復期間禁止競爭修改且顯示進度", async () => {
    const gate = deferred<Record<string, unknown>>();
    mockDesktop.state.prefs = { ...BASE_PREFS, companionPendingPresetOp: PENDING };
    mockDesktop.presetApply.mockReturnValueOnce(gate.promise);
    renderPage(); await ready();
    await screen.findByTestId("companion-preset-recovering");
    expect(highlighted()).toBeNull();
    for (const button of presetButtons()) expect(button).toBeDisabled();
    await act(async () => gate.resolve(hostResult("applied")));
    await waitFor(() => expect(highlighted()).toBe("安靜"));
  });

  it("host忙碌或連線錯誤時不把舊值當本次完成", async () => {
    mockDesktop.presetApply.mockRejectedValueOnce(new Error("陪伴設定正在套用，請稍後再試。"));
    renderPage(); await ready();
    await userEvent.click(screen.getByRole("button", { name: "安靜" }));
    await screen.findByText(/請稍後再試/);
    expect(highlighted()).toBeNull();
  });

  it("保留較新自訂設定時，畫面呈現host返回的有效值", async () => {
    mockDesktop.presetApply.mockResolvedValueOnce(hostResult("custom-effective", { prefs: { ...BASE_PREFS, companionExpressiveness: "lively" }, proactive: { config: { ...PROACTIVE_CONFIG, mode: "off" } } }));
    renderPage(); await ready();
    await userEvent.click(screen.getByRole("button", { name: "安靜" }));
    await waitFor(() => expect(summaryText()).toContain("主動說話：關閉"));
    expect(summaryText()).toContain("表現程度：活潑");
    expect(highlighted()).toBeNull();
  });

  it("讀不到桌面偏好會顯示真實錯誤", async () => {
    mockDesktop.prefsGet.mockRejectedValue(new Error("prefs file unreadable"));
    renderPage();
    const box = await screen.findByTestId("companion-prefs-unavailable");
    expect(box.textContent).toContain("prefs file unreadable");
    expect(box.textContent).not.toContain("瀏覽器檢視");
  });

  it("主動對話初次讀取失敗不高亮", async () => {
    mockApi.proactiveDialogueGet.mockRejectedValue(new Error("daemon unreachable"));
    renderPage(); await ready();
    await waitFor(() => expect(summaryText()).toContain("無法確認目前生效值"));
    expect(highlighted()).toBeNull();
  });

  it("套用期間主動對話和表現程度控制項停用", async () => {
    const gate = deferred<Record<string, unknown>>();
    mockDesktop.presetApply.mockReturnValueOnce(gate.promise);
    const { container } = renderPage(); await ready();
    for (const key of ["proactive", "behavior"]) {
      const details = container.querySelector<HTMLDetailsElement>(`details[data-disclosure="${key}"]`)!;
      fireEvent.click(details.querySelector("summary")!);
    }
    await userEvent.click(screen.getByRole("button", { name: "安靜" }));
    await waitFor(() => expect(mockDesktop.presetApply).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("combobox", { name: /表現程度/ })).toBeDisabled();
    expect(screen.getByRole("combobox", { name: /^模式$/ })).toBeDisabled();
    expect(screen.getByRole("spinbutton", { name: /每日費用上限/ })).toBeDisabled();
    await act(async () => gate.resolve(hostResult("applied")));
    await waitFor(() => expect(screen.getByRole("combobox", { name: /^模式$/ })).toBeEnabled());
  });

  it("兩次偏好寫入反序回來：先送出的舊回應不得蓋掉後送出的新設定", async () => {
    const older = deferred<Record<string, unknown>>();
    const newer = deferred<Record<string, unknown>>();
    mockDesktop.prefsPatch
      .mockImplementationOnce(async () => await older.promise)
      .mockImplementationOnce(async () => await newer.promise);
    const { container } = renderPage();
    await ready();
    const details = container.querySelector<HTMLDetailsElement>('details[data-disclosure="behavior"]')!;
    fireEvent.click(details.querySelector("summary")!);
    const select = screen.getByRole("combobox", { name: /表現程度/ });
    fireEvent.change(select, { target: { value: "lively" } });
    fireEvent.change(select, { target: { value: "quiet" } });
    await waitFor(() => expect(mockDesktop.prefsPatch).toHaveBeenCalledTimes(2));

    // 後送出的先回來，先送出的後回來：畫面必須停在**最後一次請求**的結果。
    await act(async () => {
      newer.resolve({ ...BASE_PREFS, companionExpressiveness: "quiet" });
      await Promise.resolve();
    });
    await act(async () => {
      older.resolve({ ...BASE_PREFS, companionExpressiveness: "lively" });
      await Promise.resolve();
    });
    await waitFor(() =>
      expect(screen.getByRole("combobox", { name: /表現程度/ })).toHaveValue("quiet")
    );
  });
});

// Independent Verify of N4-UI-01/02: exercise the real page, not its presenter.
describe("N4 independent review: completed host results remain current", () => {
  it("a pre-operation prefs read cannot overwrite a successfully applied host preset", async () => {
    const page = renderPage();
    await ready();
    await waitFor(() => expect(highlighted()).toBe("自然"));
    const prior = { ...BASE_PREFS };
    const delayed = deferred<Record<string, unknown>>();
    const calls = mockDesktop.prefsGet.mock.calls.length;
    mockDesktop.prefsGet.mockImplementationOnce(() => delayed.promise);
    page.rerender(<AppStateProvider ready={true} refreshKey={1}><CompanionPage refreshKey={1} /></AppStateProvider>);
    await waitFor(() => expect(mockDesktop.prefsGet.mock.calls.length).toBeGreaterThan(calls));
    await userEvent.click(screen.getByRole("button", { name: "安靜" }));
    await waitFor(() => expect(highlighted()).toBe("安靜"));
    await act(async () => { delayed.resolve(prior); await Promise.resolve(); });
    expect(highlighted()).toBe("安靜");
    await userEvent.click(screen.getByRole("button", { name: "活潑" }));
    expect(mockDesktop.presetApply.mock.calls[mockDesktop.presetApply.mock.calls.length - 1]?.[0]).toMatchObject({ expectedPrefsRevision: "1" });
  });

  it("reconnection reconciles a host recovery completed after an unverified response", async () => {
    mockDesktop.presetApply.mockResolvedValueOnce(hostResult("unverified"));
    const page = renderPage();
    await ready();
    await waitFor(() => expect(highlighted()).toBe("自然"));
    await userEvent.click(screen.getByRole("button", { name: "安靜" }));
    await waitFor(() => expect(highlighted()).toBeNull());
    expect(summaryText()).toContain("無法確認");
    // Daemon recovery has now finished both stores and cleared its marker.
    mockDesktop.state.prefs = hostResult("applied").prefs;
    mockApi.proactiveDialogueGet.mockResolvedValue({ config: { ...PROACTIVE_CONFIG, mode: "necessary" }, sentThisHour: 0 });
    const calls = mockDesktop.prefsGet.mock.calls.length;
    page.rerender(<AppStateProvider ready={true} refreshKey={1}><CompanionPage refreshKey={1} connectionKey={1} /></AppStateProvider>);
    await waitFor(() => expect(mockDesktop.prefsGet.mock.calls.length).toBeGreaterThan(calls));
    await waitFor(() => expect(highlighted()).toBe("安靜"));
    expect(summaryText()).not.toContain("無法確認");
  });
});

it("a prefs read issued during a host operation cannot overwrite its later completion", async () => {
  const page = renderPage();
  await ready();
  await waitFor(() => expect(highlighted()).toBe("自然"));
  const operation = deferred<Record<string, unknown>>();
  mockDesktop.presetApply.mockReturnValueOnce(operation.promise);
  await userEvent.click(screen.getByRole("button", { name: "安靜" }));
  await waitFor(() => expect(mockDesktop.presetApply).toHaveBeenCalledTimes(1));
  const delayed = deferred<Record<string, unknown>>();
  const calls = mockDesktop.prefsGet.mock.calls.length;
  mockDesktop.prefsGet.mockImplementationOnce(() => delayed.promise);
  page.rerender(<AppStateProvider ready={true} refreshKey={1}><CompanionPage refreshKey={1} /></AppStateProvider>);
  await waitFor(() => expect(mockDesktop.prefsGet.mock.calls.length).toBeGreaterThan(calls));
  await act(async () => { operation.resolve(hostResult("applied")); await Promise.resolve(); });
  await waitFor(() => expect(highlighted()).toBe("安靜"));
  await act(async () => { delayed.resolve({ ...BASE_PREFS }); await Promise.resolve(); });
  expect(highlighted()).toBe("安靜");
});
