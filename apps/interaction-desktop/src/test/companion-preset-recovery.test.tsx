// 陪伴預設的兩段寫入：交易化與恢復（M4）。
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
import { beginPresetOp, markerOf } from "../companion/applyPresetPlan";

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

function markerInPrefs(): Record<string, unknown> | null {
  return (mockDesktop.state.prefs.companionPendingPresetOp as Record<string, unknown> | null) ?? null;
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

// ---------------------------------------------------------------------------
// 1. 第一段成功、第二段失敗
// ---------------------------------------------------------------------------

describe("兩段寫入：第二段沒送到", () => {
  it("marker 與第一段是同一次原子寫入（不是先寫偏好再另外記一筆）", async () => {
    renderPage();
    await ready();
    await userEvent.click(screen.getByRole("button", { name: "安靜" }));
    await waitFor(() => expect(mockDesktop.prefsPatch).toHaveBeenCalled());
    const first = mockDesktop.prefsPatch.mock.calls[0][0] as Record<string, unknown>;
    expect(Object.keys(first).sort()).toEqual([
      "companionDoNotDisturb",
      "companionExpressiveness",
      "companionPendingPresetOp",
    ]);
    expect(first.companionExpressiveness).toBe("quiet");
    expect(first.companionDoNotDisturb).toBe(true);
    expect(first.companionPendingPresetOp).toMatchObject({
      presetId: "quiet",
      proactivePatch: { mode: "necessary" },
    });
  });

  it("第二段被拒絕：狀態是半套用、marker 留著、不高亮任何檔位、可以補送", async () => {
    mockApi.proactiveDialoguePatch.mockRejectedValue(new Error("後端拒絕"));
    renderPage();
    await ready();
    await userEvent.click(screen.getByRole("button", { name: "安靜" }));

    const partial = await screen.findByTestId("companion-preset-partial");
    expect(partial.textContent).toContain("安靜");
    expect(partial.textContent).toContain("補送");
    // 半套用不得高亮任何檔位（第一段寫進去了，但整組還沒生效）。
    expect(highlighted()).toBeNull();
    // 有效值仍然逐項說得出來（收合 ≠ 隱藏事實）。
    expect(summaryText()).toContain("自訂");
    expect(summaryText()).toContain("主動說話：自然");
    // marker 留在偏好裡，重開之後還補得回來。
    expect(markerInPrefs()).toMatchObject({ presetId: "quiet" });
  });

  it("補送鈕重送同一段（冪等：只有 mode），成功後清掉 marker 並回到已套用", async () => {
    mockApi.proactiveDialoguePatch.mockRejectedValueOnce(new Error("後端拒絕"));
    renderPage();
    await ready();
    await userEvent.click(screen.getByRole("button", { name: "安靜" }));
    const partial = await screen.findByTestId("companion-preset-partial");

    await userEvent.click(within(partial).getByRole("button", { name: "補送" }));
    await waitFor(() => expect(markerInPrefs()).toBeNull());
    for (const call of mockApi.proactiveDialoguePatch.mock.calls as unknown as Record<string, unknown>[][]) {
      expect(Object.keys(call[0])).toEqual(["mode"]);
      expect(call[0].mode).toBe("necessary");
    }
    expect(mockApi.proactiveDialoguePatch).toHaveBeenCalledTimes(2);
    await waitFor(() => expect(highlighted()).toBe("安靜"));
    expect(screen.queryByTestId("companion-preset-partial")).toBeNull();
  });

  it("讀回沒有明說模式：不算完成（marker 留著，不用預設值頂替）", async () => {
    mockApi.proactiveDialoguePatch.mockRejectedValue(new Error("timeout"));
    // 後端回了，但沒說 mode——這是「不知道」，不是「已經是預設值」。
    mockApi.proactiveDialogueGet.mockImplementation(
      async () => ({ sentThisHour: 0 }) as unknown as { config: typeof PROACTIVE_CONFIG; sentThisHour: number }
    );
    renderPage();
    await ready();
    await userEvent.click(screen.getByRole("button", { name: "安靜" }));
    await screen.findByTestId("companion-preset-partial");
    expect(markerInPrefs()).toMatchObject({ presetId: "quiet" });
  });

  it("回應遺失但其實已經生效：讀回等於目標就視為完成，不重送、不謊報失敗", async () => {
    mockApi.proactiveDialoguePatch.mockRejectedValue(new Error("connection reset"));
    // 讀回顯示後端其實收到了（模式已經是 necessary）。
    mockApi.proactiveDialogueGet.mockImplementation(async () => ({
      config: { ...PROACTIVE_CONFIG, mode: "necessary" },
      sentThisHour: 0,
    }));
    renderPage();
    await ready();
    await userEvent.click(screen.getByRole("button", { name: "安靜" }));

    await waitFor(() => expect(markerInPrefs()).toBeNull());
    await waitFor(() => expect(highlighted()).toBe("安靜"));
    expect(screen.queryByTestId("companion-preset-partial")).toBeNull();
    // 只送過一次：讀回確認過就不再重送。
    expect(mockApi.proactiveDialoguePatch).toHaveBeenCalledTimes(1);
  });
});

// ---------------------------------------------------------------------------
// 2. 重開之後的恢復
// ---------------------------------------------------------------------------

describe("重開之後：marker 的恢復", () => {
  function prefsWithMarker(id: "quiet" | "lively", extra: Record<string, unknown> = {}) {
    const plan = beginPresetOp(id, 1_700_000_000_000)!;
    return {
      ...BASE_PREFS,
      ...plan.prefs,
      companionPendingPresetOp: markerOf(plan),
      ...extra,
    };
  }

  it("marker 還在且使用者沒改過：自動補送一次（只有一次），完成後清掉 marker", async () => {
    mockDesktop.state.prefs = prefsWithMarker("quiet");
    renderPage();
    await ready();
    await waitFor(() => expect(mockApi.proactiveDialoguePatch).toHaveBeenCalledWith({ mode: "necessary" }));
    await waitFor(() => expect(markerInPrefs()).toBeNull());
    await waitFor(() => expect(highlighted()).toBe("安靜"));
    // 有界：每次 mount 只補送一次，不會因為重新 render 或輪詢又送一次。
    await act(async () => {
      await Promise.resolve();
    });
    expect(mockApi.proactiveDialoguePatch).toHaveBeenCalledTimes(1);
  });

  it("使用者事後改過目標欄位：不補送，只把 marker 清掉（不覆蓋使用者的修改）", async () => {
    // marker 說要套「安靜」，但目前的表現程度已經被使用者改成活潑。
    mockDesktop.state.prefs = prefsWithMarker("quiet", { companionExpressiveness: "lively" });
    renderPage();
    await ready();
    await waitFor(() => expect(markerInPrefs()).toBeNull());
    expect(mockApi.proactiveDialoguePatch).not.toHaveBeenCalled();
    // 使用者的修改原封不動。
    expect(mockDesktop.state.prefs.companionExpressiveness).toBe("lively");
    expect(highlighted()).toBeNull();
    expect(summaryText()).toContain("自訂");
  });

  it("補送期間畫面說「正在補送」，而且不高亮任何檔位", async () => {
    const gate = deferred<{ config: typeof PROACTIVE_CONFIG; sentThisHour: number }>();
    mockApi.proactiveDialoguePatch.mockImplementationOnce(async () => await gate.promise);
    mockDesktop.state.prefs = prefsWithMarker("quiet");
    renderPage();
    await ready();
    const recovering = await screen.findByTestId("companion-preset-recovering");
    expect(recovering.textContent).toContain("補送");
    expect(highlighted()).toBeNull();
    for (const button of presetButtons()) expect(button).toBeDisabled();
    await act(async () => {
      gate.resolve({ config: { ...PROACTIVE_CONFIG, mode: "necessary" }, sentThisHour: 0 });
      await Promise.resolve();
    });
    await waitFor(() => expect(screen.queryByTestId("companion-preset-recovering")).toBeNull());
    await waitFor(() => expect(highlighted()).toBe("安靜"));
  });
});

// ---------------------------------------------------------------------------
// 3. 讀不回有效值／讀不到桌面偏好
// ---------------------------------------------------------------------------

describe("讀不回有效值時不假裝知道", () => {
  it("主動說話的設定讀不回來：狀態 unverified，不高亮任何檔位", async () => {
    mockApi.proactiveDialogueGet.mockRejectedValue(new Error("daemon unreachable"));
    renderPage();
    await ready();
    await waitFor(() => expect(summaryText()).toContain("無法確認目前生效值"));
    expect(highlighted()).toBeNull();
  });

  it("桌面版讀不到偏好：說出讀取失敗的原因，不得誤報成「瀏覽器檢視」", async () => {
    mockDesktop.prefsGet.mockRejectedValue(new Error("prefs file unreadable"));
    renderPage();
    // 陪伴方式區塊仍在，但裡面是錯誤而不是「這是瀏覽器檢視」。
    const box = await screen.findByTestId("companion-prefs-unavailable");
    expect(box.textContent).toContain("prefs file unreadable");
    expect(box.textContent).not.toContain("瀏覽器檢視");
    expect(screen.queryByText(/此為瀏覽器檢視/)).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// 4. 交易期間的併發
// ---------------------------------------------------------------------------

describe("交易期間的併發", () => {
  it("交易期間「調整陪伴方式」的表現程度不得同時被改（select 停用）", async () => {
    const gate = deferred<{ config: typeof PROACTIVE_CONFIG; sentThisHour: number }>();
    mockApi.proactiveDialoguePatch.mockImplementationOnce(async () => await gate.promise);
    const { container } = renderPage();
    await ready();
    const details = container.querySelector<HTMLDetailsElement>('details[data-disclosure="behavior"]')!;
    fireEvent.click(details.querySelector("summary")!);
    const select = screen.getByRole("combobox", { name: /表現程度/ });
    expect(select).toBeEnabled();

    await userEvent.click(screen.getByRole("button", { name: "安靜" }));
    await waitFor(() => expect(mockApi.proactiveDialoguePatch).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("combobox", { name: /表現程度/ })).toBeDisabled();

    await act(async () => {
      gate.resolve({ config: { ...PROACTIVE_CONFIG, mode: "necessary" }, sentThisHour: 0 });
      await Promise.resolve();
    });
    await waitFor(() => expect(screen.getByRole("combobox", { name: /表現程度/ })).toBeEnabled());
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
