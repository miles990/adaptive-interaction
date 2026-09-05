// 角色頁首屏收斂（M3 §4.1）：
//   首屏只回答三件事——目前角色（名字＋預覽＋顯示／暫停）、陪伴方式（預設摘要＋調整）、
//   手機連接／同步；其餘（外觀與名字、細部行為、完整頻率、AI 生成設定、角色庫）按需展開。
//
// 這一組測試釘住的是**語意與安全**，不是逐字文案：
//   - 收合 ≠ 隱藏事實：費用／次數上限與指定的 AI 幫手在收合的摘要行仍看得到數值。
//   - 六組「安靜」語意分別列出各自的底層設定與有效狀態，不合併成一個布林。
//   - 安全提示永不安靜；感測使用中一定顯示。
//   - 套用陪伴預設不覆蓋其它自訂值、不改費用上限、不啟用權限、不換 AI 幫手。

import fs from "node:fs";
import path from "node:path";
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

function textCharacter(id: string, name: string) {
  return {
    schemaVersion: "1.0",
    characterId: id,
    displayName: { "zh-TW": name },
    version: "1.0.0",
    adapterKind: "in-process",
    entrypoint: { kind: "builtin", id: "text" },
    capabilities: { "visual.textBubble": { supported: true } },
    inputCapabilities: {},
    channels: ["bubble"],
    states: ["line"],
    intents: ["idle"],
  };
}

const EXTRA_IDS = ["c1", "c2", "c3", "c4"] as const;

const INDEX = {
  schemaVersion: "1.0",
  default: "shu-maid",
  characters: [
    { characterId: "shu-maid", manifestPath: "/characters/shu-maid/manifest.json", origin: "builtin" },
    { characterId: "plain-text", manifestPath: "/characters/plain-text/manifest.json", origin: "builtin" },
    ...EXTRA_IDS.map((id) => ({
      characterId: id,
      manifestPath: `/characters/${id}/manifest.json`,
      origin: "builtin" as const,
    })),
  ],
};

const FILES: Record<string, string> = {
  "/characters/index.json": JSON.stringify(INDEX),
  "/characters/shu-maid/manifest.json": bundled("shu-maid"),
  "/characters/plain-text/manifest.json": bundled("plain-text"),
  ...Object.fromEntries(
    EXTRA_IDS.map((id, i) => [`/characters/${id}/manifest.json`, JSON.stringify(textCharacter(id, `角色${i + 1}`))])
  ),
};

const BASE_PREFS = {
  closeBehavior: null,
  askOnClose: true,
  launchAtLogin: false,
  showCompanionOnStart: true,
  openControlCenterOnStart: false,
  companionVisible: true,
  companionPosition: null,
  companionSize: [200, 210] as [number, number],
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
    const { companionPreferences: _dropped, ...rest } = patch;
    Object.assign(state.prefs, rest);
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
import { libraryDigest } from "../pages/companion/libraryDigest";

function renderPage(props: { advanced?: boolean; onNavigate?: (tab: string) => void } = {}) {
  return render(
    <AppStateProvider ready={true} refreshKey={0}>
      <CompanionPage refreshKey={0} {...props} />
    </AppStateProvider>
  );
}

/** 首屏＝第一個收合區塊之前、以 DOM 順序排在最前的區塊（不含收合中的內容）。 */
const INTERACTIVE = 'button, input, select, textarea, a[href], [role="button"]';

function inClosedDetails(el: Element): boolean {
  let n: Element | null = el.parentElement;
  while (n) {
    if (n.tagName === "DETAILS" && !(n as HTMLDetailsElement).open) return true;
    n = n.parentElement;
  }
  return false;
}

function firstScreenControls(container: HTMLElement): HTMLElement[] {
  const page = container.querySelector(".character-page")!;
  return Array.from(page.querySelectorAll<HTMLElement>(INTERACTIVE)).filter((el) => !inClosedDetails(el));
}

function disclosures(container: HTMLElement): HTMLDetailsElement[] {
  return Array.from(
    container.querySelectorAll<HTMLDetailsElement>(".character-page details[data-disclosure]")
  );
}

async function ready() {
  await screen.findByRole("group", { name: "陪伴方式" });
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  mockDesktop.state.prefs = { ...BASE_PREFS };
  mockName.current = { name: "小樞", pronoun: "她", characterId: "shu-maid", loaded: true, icon: "cat" };
  mockDesktop.prefsPatch.mockImplementation(mockDesktop.applyPrefsPatch);
  // 主動對話的兩個 mock 每一則測試都回到預設實作：`mockRejectedValue`／`mockImplementation`
  // 不會被 `clearAllMocks` 清掉，漏到下一則測試就會讓它以為「後端一直拒絕」。
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
// 1. 首屏
// ---------------------------------------------------------------------------

describe("角色頁：首屏只回答三件事", () => {
  it("首屏依序是目前角色／陪伴方式／同步，其餘一律預設收合", async () => {
    const { container } = renderPage();
    await ready();
    // 首屏＝第一個收合區塊之前的那幾個區塊（角色庫展開後裡面還有自己的標題，不算首屏）。
    const firstScreen = container.querySelector(".character-first-screen")!;
    const headings = Array.from(firstScreen.querySelectorAll("section.section > .section-head h2")).map(
      (h) => h.textContent
    );
    expect(headings).toEqual(["目前角色", "陪伴方式", "同步"]);

    const details = disclosures(container);
    expect(details.map((d) => d.dataset.disclosure)).toEqual([
      "behavior",
      "appearance",
      "quiet",
      "proactive",
      "library",
    ]);
    for (const d of details) {
      expect(d.open, `${d.dataset.disclosure} 必須預設收合`).toBe(false);
      const summary = d.querySelector("summary");
      // 鍵盤可達（原生 summary）＋有可及名稱。
      expect(summary).not.toBeNull();
      expect(summary!.textContent!.trim().length).toBeGreaterThan(0);
      expect(summary!.getAttribute("tabindex")).not.toBe("-1");
    }
  });

  it("首屏的可互動控制項收斂到個位數（收合前 40 個）", async () => {
    const { container } = renderPage();
    await ready();
    expect(firstScreenControls(container).length).toBeLessThanOrEqual(8);
  });

  it("目前角色：名字、狀態、預覽與顯示／暫停都留在首屏", async () => {
    const { container } = renderPage();
    await ready();
    const current = container.querySelector(".character-current")!;
    expect(within(current as HTMLElement).getByRole("heading", { name: "小樞", level: 3 })).toBeInTheDocument();
    expect(within(current as HTMLElement).getByRole("checkbox", { name: "顯示桌面角色" })).toBeInTheDocument();
    // 預覽留在首屏（不在任何收合區塊裡）。
    const preview = container.querySelector(".character-preview")!;
    expect(preview).not.toBeNull();
    expect(inClosedDetails(preview)).toBe(false);
    expect(screen.getByText(/隱藏不等於緊急停止/)).toBeInTheDocument();
  });

  it("同步卡收到並轉傳 onNavigate（第二入口的 route id 不變）", async () => {
    const source = fs.readFileSync(path.resolve("src/pages/CompanionPage.tsx"), "utf8");
    expect(source).toMatch(/onNavigate/);
    const onNavigate = vi.fn();
    renderPage({ onNavigate });
    await ready();
    // CompanionPage 必須真的把 prop 解構出來（不是宣告了卻沒用）。
    expect(source).toMatch(/onNavigate,/);
  });

  it("展開收合區塊後，原本的控制項全部還在（收合 ≠ 刪功能）", async () => {
    const { container } = renderPage();
    await ready();
    const before = firstScreenControls(container).length;
    for (const d of disclosures(container)) d.open = true;
    fireEvent.click(container.querySelector("summary")!);
    await waitFor(() => expect(firstScreenControls(container).length).toBeGreaterThan(before));
    expect(screen.getByRole("combobox", { name: /表現程度/ })).toBeInTheDocument();
    expect(screen.getByRole("spinbutton", { name: "每小時最多則數" })).toBeInTheDocument();
    expect(screen.getByRole("spinbutton", { name: "每日費用上限（USD）" })).toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: /指定 AI 幫手/ })).toBeInTheDocument();
  });
});

// ---------------------------------------------------------------------------
// 2. 陪伴預設
// ---------------------------------------------------------------------------

describe("角色頁：陪伴方式摘要與預設", () => {
  it("摘要是一句話（預設名稱＋一行說明），並附「調整」展開", async () => {
    const { container } = renderPage();
    await ready();
    const group = screen.getByRole("group", { name: "陪伴方式" });
    expect(within(group).getByText("自然")).toBeInTheDocument();
    expect(screen.getByTestId("companion-preset-summary").textContent).toContain(
      "一般的表現與說話頻率"
    );
    const behavior = container.querySelector('details[data-disclosure="behavior"]')!;
    expect(behavior.querySelector("summary")!.textContent).toContain("調整");
  });

  it("套用預設只寫既有的三個欄位：不改費用上限、不換 AI 幫手、不啟用權限", async () => {
    renderPage();
    await ready();
    await userEvent.click(screen.getByRole("button", { name: "安靜" }));
    // M4：第一段與「還有一段沒送到」的恢復 marker 是**同一次**原子寫入
    //（交易語意見 `companion-preset-recovery.test.tsx`）。守的仍是同一件事——
    // 這一次寫入只有那兩個既有的偏好欄位，其餘一律不得出現。
    await waitFor(() =>
      expect(mockDesktop.prefsPatch).toHaveBeenCalledWith({
        companionExpressiveness: "quiet",
        companionDoNotDisturb: true,
        companionPendingPresetOp: expect.objectContaining({
          presetId: "quiet",
          proactivePatch: { mode: "necessary" },
        }),
      })
    );
    await waitFor(() => expect(mockApi.proactiveDialoguePatch).toHaveBeenCalledWith({ mode: "necessary" }));
    for (const call of mockApi.proactiveDialoguePatch.mock.calls as unknown as Record<string, unknown>[][]) {
      expect(Object.keys(call[0])).toEqual(["mode"]);
    }
    // 其它自訂值原封不動。
    expect(mockDesktop.state.prefs.companionPersona).toBe("persona-shu");
    expect(mockDesktop.state.prefs.companionSound).toBe(false);
    expect(mockDesktop.state.prefs.companionPack).toBe("shu-maid");
  });

  // 送出 ≠ 完成：預設是從首屏按下去的，失敗訊息不得被收進任何收合區塊。
  it("後端拒絕主動說話的設定時，錯誤留在首屏（不是藏在收合區塊裡）", async () => {
    mockApi.proactiveDialoguePatch.mockRejectedValue(new Error("後端拒絕"));
    const { container } = renderPage();
    await ready();
    await userEvent.click(screen.getByRole("button", { name: "安靜" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/主動說話的設定沒有寫入成功/);
    expect(alert).toHaveTextContent(/後端拒絕/);
    expect(inClosedDetails(alert)).toBe(false);
    expect(container.querySelector(".character-first-screen")!.contains(alert)).toBe(true);
    // 半套用要誠實：桌面偏好寫成功了、後端沒有 → 顯示「自訂」與逐項有效值。
    const summary = screen.getByTestId("companion-preset-summary");
    await waitFor(() => expect(summary.textContent).toContain("自訂"));
  });

  it("內建角色索引載入失敗時，錯誤留在首屏（不是藏在收合的角色庫裡）", async () => {
    // 真的讓 /characters/index.json 失敗：畫面必須說出原因，而且不必展開任何區塊。
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) => {
        if (url === "/characters/index.json") throw new Error("connection refused");
        const body = FILES[url];
        if (body === undefined) return { ok: false, status: 404, text: async () => "", json: async () => ({}) };
        return { ok: true, status: 200, text: async () => body, json: async () => JSON.parse(body) };
      })
    );
    const { container } = renderPage();
    const alert = await screen.findByText(/內建角色索引無法載入/);
    expect(alert).toHaveAttribute("role", "alert");
    expect(inClosedDetails(alert)).toBe(false);
    expect(container.querySelector(".character-first-screen")!.contains(alert)).toBe(true);
    // 同一個錯誤不在收合的角色庫裡再出現一次（螢幕閱讀器不必聽兩遍）。
    expect(screen.getAllByText(/內建角色索引無法載入/)).toHaveLength(1);
  });

  // 對抗審查 general-mode-ux-013：套用預設是**兩段寫入**（桌面偏好 → 後端主動對話模式），
  // 但 busy 以前只鎖住第一段：第二段還在飛的時候按鈕就已經解鎖了，快速切換檔位會讓
  // 先送出的舊回應蓋掉後送出的新回應，畫面顯示的檔位與後端真正生效的不一致。
  function deferred<T>() {
    let resolve!: (value: T) => void;
    const promise = new Promise<T>((r) => {
      resolve = r;
    });
    return { promise, resolve };
  }

  /** 摘要行「目前：<strong>檔位</strong>」的檔位名。 */
  function presetLabel(): string {
    return screen.getByTestId("companion-preset-summary").querySelector("strong")!.textContent ?? "";
  }

  it("兩段寫入都在忙碌鎖內：後端那一段還沒回來時，檔位按鈕不得再按", async () => {
    const gate = deferred<{ config: typeof PROACTIVE_CONFIG; sentThisHour: number }>();
    mockApi.proactiveDialoguePatch.mockImplementationOnce(async () => await gate.promise);
    renderPage();
    await ready();
    await userEvent.click(screen.getByRole("button", { name: "安靜" }));
    // 第一段（桌面偏好）已經寫完，第二段（後端主動對話模式）還在飛。
    await waitFor(() => expect(mockApi.proactiveDialoguePatch).toHaveBeenCalledTimes(1));
    const group = screen.getByRole("group", { name: "陪伴方式" });
    for (const button of within(group).getAllByRole("button")) {
      expect(button, `第二段寫入期間「${button.textContent}」不得可按`).toBeDisabled();
    }
    await act(async () => {
      gate.resolve({ config: { ...PROACTIVE_CONFIG, mode: "necessary" }, sentThisHour: 0 });
      await Promise.resolve();
    });
    await waitFor(() => expect(within(group).getByRole("button", { name: "安靜" })).toBeEnabled());
    expect(presetLabel()).toBe("安靜");
  });

  it("較舊的讀取回應不得覆蓋剛套用的檔位（世代計數器）", async () => {
    // 進頁面時的 GET 還在飛，使用者已經按了「活潑」：GET 帶回來的是**按之前**的模式，
    // 套下去畫面就退回舊檔位，而後端其實已經是新的。
    const slowGet = deferred<{ config: typeof PROACTIVE_CONFIG; sentThisHour: number }>();
    mockApi.proactiveDialogueGet.mockImplementationOnce(async () => await slowGet.promise);
    renderPage();
    await ready();
    await userEvent.click(screen.getByRole("button", { name: "活潑" }));
    await waitFor(() => expect(presetLabel()).toBe("活潑"));
    await act(async () => {
      slowGet.resolve({ config: { ...PROACTIVE_CONFIG }, sentThisHour: 0 });
      await Promise.resolve();
    });
    expect(presetLabel()).toBe("活潑");
  });

  it("進階區逐項修改：先送出的舊回應不得蓋掉後送出的新設定", async () => {
    // 主動對話的 status 只有一個 owner（這個 hook），但回應不保證照送出順序回來。
    const older = deferred<{ config: typeof PROACTIVE_CONFIG; sentThisHour: number }>();
    const newer = deferred<{ config: typeof PROACTIVE_CONFIG; sentThisHour: number }>();
    mockApi.proactiveDialoguePatch
      .mockImplementationOnce(async () => await older.promise)
      .mockImplementationOnce(async () => await newer.promise);
    const { container } = renderPage({ advanced: true });
    await ready();
    const details = container.querySelector<HTMLDetailsElement>('details[data-disclosure="proactive"]')!;
    fireEvent.click(details.querySelector("summary")!);
    const modeSelect = details.querySelector<HTMLSelectElement>("select")!;
    fireEvent.change(modeSelect, { target: { value: "necessary" } });
    fireEvent.change(modeSelect, { target: { value: "lively" } });
    await waitFor(() => expect(mockApi.proactiveDialoguePatch).toHaveBeenCalledTimes(2));
    // 後送出的先回來，先送出的後回來：畫面必須停在**最後一次請求**的結果。
    await act(async () => {
      newer.resolve({ config: { ...PROACTIVE_CONFIG, mode: "lively" }, sentThisHour: 0 });
      await Promise.resolve();
    });
    await act(async () => {
      older.resolve({ config: { ...PROACTIVE_CONFIG, mode: "necessary" }, sentThisHour: 0 });
      await Promise.resolve();
    });
    expect(details.querySelector<HTMLSelectElement>("select")!.value).toBe("lively");
  });

  it("不吻合任何預設時顯示「自訂」並逐項列出有效值", async () => {
    mockDesktop.state.prefs = { ...BASE_PREFS, companionDoNotDisturb: true };
    renderPage();
    await ready();
    const summary = await screen.findByTestId("companion-preset-summary");
    await waitFor(() => expect(summary.textContent).toContain("自訂"));
    expect(summary.textContent).toContain("表現程度：自然");
    expect(summary.textContent).toContain("勿擾：開啟");
    expect(summary.textContent).toContain("主動說話：自然");
  });
});

// ---------------------------------------------------------------------------
// 3. 安靜語意合併呈現（不合併成一個布林）
// ---------------------------------------------------------------------------

describe("角色頁：安靜與勿擾列出實際受影響的項目", () => {
  it("五個項目各自標示底層設定與現在的有效狀態；安全提示永不安靜", async () => {
    const { container } = renderPage();
    await ready();
    const list = screen.getByRole("list", { name: "安靜與勿擾的實際影響" });
    const items = Array.from(list.querySelectorAll<HTMLElement>("li[data-quiet-item]"));
    expect(items.map((li) => li.dataset.quietItem)).toEqual([
      "safety",
      "sensing",
      "companion",
      "proactive",
      "notifications",
    ]);
    for (const li of items) {
      expect(li.querySelector("[data-quiet-source]"), `${li.dataset.quietItem} 必須標示由哪個設定控制`).not.toBeNull();
      expect(li.querySelector("[data-quiet-state]"), `${li.dataset.quietItem} 必須標示現在的有效狀態`).not.toBeNull();
    }
    expect(items[0].textContent).toContain("永遠顯示");
    expect(items[0].querySelector("[data-quiet-source]")!.textContent).toContain("固定安全文字");
    expect(items[1].textContent).toContain("使用中");
    expect(items[2].querySelector("[data-quiet-source]")!.textContent).toContain("勿擾");
    expect(items[3].querySelector("[data-quiet-source]")!.textContent).toContain("主動式對話");
    expect(items[4].querySelector("[data-quiet-source]")!.textContent).toContain("安靜時段");
    // 收合區塊的摘要行也不得把不同語意合併成一個字。
    const quiet = container.querySelector('details[data-disclosure="quiet"]')!;
    const line = quiet.querySelector("summary")!.textContent!;
    expect(line).toContain("勿擾");
    expect(line).toContain("主動說話");
    expect(line).toContain("安靜時段");
  });

  it("桌寵設的本機安靜期唯讀顯示，並可以在這裡取消（只清 prefs 欄位）", async () => {
    const until = Date.now() + 30 * 60 * 1000;
    mockDesktop.state.prefs = { ...BASE_PREFS, companionProactiveQuietUntil: until };
    renderPage();
    await ready();
    expect(await screen.findByText(/本機安靜期至/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "取消本機安靜期" }));
    await waitFor(() =>
      expect(mockDesktop.prefsPatch).toHaveBeenCalledWith({ companionProactiveQuietUntil: 0 })
    );
  });

  it("安靜時段編輯保留在安靜區，且仍送出明確的靜音清單（不含桌面角色）", async () => {
    renderPage();
    await ready();
    const toggle = await screen.findByRole("checkbox", { name: /未啟用|已啟用/ });
    await userEvent.click(toggle);
    await waitFor(() => expect(mockApi.policyPatch).toHaveBeenCalled());
    const calls = mockApi.policyPatch.mock.calls as unknown as unknown[][];
    const sent = calls[calls.length - 1][0] as { quietHours: { silencedChannels: string[] }[] };
    expect(sent.quietHours[0].silencedChannels).toEqual(["audio", "haptic", "notification", "light"]);
  });
});

// ---------------------------------------------------------------------------
// 4. 費用／權限有效限制仍看得見
// ---------------------------------------------------------------------------

describe("角色頁：收起數字調校不等於藏起使用成本", () => {
  it("主動式對話收合摘要就看得到每小時上限、每日次數、費用上限與指定的 AI 幫手", async () => {
    const { container } = renderPage();
    await ready();
    const proactive = container.querySelector('details[data-disclosure="proactive"]')!;
    const line = proactive.querySelector("summary")!.textContent!;
    await waitFor(() => expect(proactive.querySelector("summary")!.textContent).toContain("每日"));
    const summaryText = proactive.querySelector("summary")!.textContent!;
    expect(summaryText).toContain("3");
    expect(summaryText).toContain("12");
    expect(summaryText).toContain("8");
    expect(summaryText).toContain("USD 1");
    expect(summaryText).toContain("AI 幫手：不使用");
    expect(line.length).toBeGreaterThan(0);
    // 展開才可以改。
    expect(proactive.hasAttribute("open")).toBe(false);
  });

  it("指定了 AI 幫手時摘要顯示的是實際那一家（不會靜默改送別家）", async () => {
    mockApi.proactiveDialogueGet.mockResolvedValue({
      config: { ...PROACTIVE_CONFIG, generativeAgent: "claude-code", dailyGenerativeCostUsd: 2.5 },
      sentThisHour: 0,
    });
    const { container } = renderPage();
    await ready();
    const proactive = container.querySelector('details[data-disclosure="proactive"]')!;
    await waitFor(() =>
      expect(proactive.querySelector("summary")!.textContent).toContain("AI 幫手：Claude Code")
    );
    expect(proactive.querySelector("summary")!.textContent).toContain("USD 2.5");
  });
});

// ---------------------------------------------------------------------------
// 5. 角色庫收斂
// ---------------------------------------------------------------------------

describe("角色頁：角色庫預設只顯示使用中＋最近／常用", () => {
  it("libraryDigest：使用中第一，接著最近使用，其餘依目錄順序遞補", () => {
    const cards = ["a", "b", "c", "d", "e", "f"].map((characterId) => ({ characterId }));
    const digest = libraryDigest(cards, "c", { usedIds: ["f", "a"] });
    expect(digest.shown.map((c) => c.characterId)).toEqual(["c", "f", "a", "b"]);
    expect(digest.hidden).toBe(2);
    // 使用中的角色不在清單裡時也不會消失（誠實：先列出使用中）。
    expect(libraryDigest(cards, "zzz", {}).shown.map((c) => c.characterId)).toEqual(["a", "b", "c", "d"]);
    // 少於上限就全列，沒有「顯示全部」的必要。
    expect(libraryDigest(cards.slice(0, 3), "a", {}).hidden).toBe(0);
  });

  it("預設 4 張卡；按「顯示全部角色」才列出其餘", async () => {
    renderPage();
    await ready();
    await screen.findByRole("article", { name: "角色 小樞" });
    await waitFor(() => expect(screen.getAllByRole("article").length).toBe(4));
    expect(screen.queryByRole("article", { name: "角色 角色4" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /顯示全部角色/ }));
    await waitFor(() => expect(screen.getAllByRole("article").length).toBe(6));
    expect(screen.getByRole("article", { name: "角色 角色4" })).toBeInTheDocument();
  });
});

// ---------------------------------------------------------------------------
// 6. 邊界：新模組不認得任何特定角色；390px
// ---------------------------------------------------------------------------

describe("角色頁：新模組的邊界與 390px", () => {
  it("pages/companion/* 與 companion/presets.ts 不寫死參考角色", () => {
    const files = [
      "src/companion/presets.ts",
      ...fs.readdirSync(path.resolve("src/pages/companion")).map((f) => `src/pages/companion/${f}`),
    ];
    expect(files.length).toBeGreaterThan(2);
    for (const rel of files) {
      const source = fs
        .readFileSync(path.resolve(rel), "utf8")
        .replace(/\/\*[\s\S]*?\*\//g, "")
        .replace(/^\s*\/\/.*$/gm, "");
      expect(source, `${rel} 不得寫死參考角色`).not.toContain("小樞");
      for (const literal of ["shu-maid", "maid-classic", "persona-shu"]) {
        expect(source, `${rel} 不得寫死「${literal}」`).not.toContain(literal);
      }
    }
  });

  it("收合區塊有頁面專屬 class 與 390px 規則，且不用動畫", () => {
    const css = fs.readFileSync(path.resolve("src/styles.css"), "utf8");
    const block = css.slice(css.indexOf("角色頁（CompanionPage）"), css.indexOf("H 區塊結束"));
    expect(block).toContain(".character-disclosure");
    expect(block).toContain(".character-quiet-list");
    expect(block).toContain(".character-preset-row");
    // 沒有 transition／animation（Reduced Motion 下也不會動）。
    const disclosureRules = block
      .split("\n")
      .filter((l) => l.includes(".character-disclosure") || l.includes(".character-preset"));
    expect(disclosureRules.join("\n")).not.toMatch(/transition|animation/);
    // 頁面專屬前綴（既有 390px 測試的同一條規則）。
    for (const selector of block.match(/^\s*\.[a-zA-Z-]+/gm) ?? []) {
      expect(selector.trim()).toMatch(/^\.(character-|first-success)/);
    }
  });

  it("390px：新區塊不以 inline style 硬編超過視窗的寬度", async () => {
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 390 });
    window.dispatchEvent(new Event("resize"));
    const { container } = renderPage();
    await ready();
    const wide = Array.from(container.querySelectorAll<HTMLElement>("[style]")).filter((el) => {
      const w = parseFloat(el.style.width || "0");
      return Number.isFinite(w) && el.style.width.endsWith("px") && w > 390;
    });
    expect(wide).toEqual([]);
    expect(container.querySelector(".character-first-screen")).not.toBeNull();
  });
});
