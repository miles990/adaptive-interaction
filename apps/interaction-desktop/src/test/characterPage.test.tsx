// 角色頁（v0.5 一般模式）：角色清單來自索引＋已匯入清單、adapter 旗標、匯入錯誤路徑、
// 非小樞角色的 preferencesSchema 表單（無 rig 字眼）、停用→純文字、崩潰→改用文字、
// 進階模式才有技術資料、390px 單欄 class。

import fs from "node:fs";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
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

/** 非小樞的內建文字角色：有 variants 與 preferencesSchema、沒有 pronouns。 */
const BUDDY = {
  schemaVersion: "1.0",
  characterId: "buddy",
  displayName: { "zh-TW": "阿寶" },
  version: "1.0.0",
  adapterKind: "in-process",
  entrypoint: { kind: "builtin", id: "text" },
  capabilities: { "visual.presence": { supported: true }, "visual.textBubble": { supported: true } },
  inputCapabilities: { "input.click": { supported: true } },
  channels: ["bubble"],
  states: ["line"],
  intents: ["idle", "notice", "claim-completed", "verified-success"],
  variants: [
    { id: "day", displayName: { "zh-TW": "白天" } },
    { id: "night", displayName: { "zh-TW": "夜晚" } },
  ],
  preferencesSchema: {
    type: "object",
    properties: {
      chatty: { type: "boolean", title: "愛講話", default: true },
      pace: { type: "integer", title: "步調", minimum: 1, maximum: 5, default: 3 },
      mood: { type: "string", title: "心情", enum: ["calm", "happy"] },
      motto: { type: "string", title: "座右銘", maxLength: 20 },
    },
  },
};

/** 外部程式角色：executable＋network 旗標。 */
const EXT = {
  schemaVersion: "1.0",
  characterId: "ext-bot",
  displayName: { "zh-TW": "外部機器人" },
  version: "0.1.0",
  adapterKind: "external-process",
  entrypoint: { kind: "process", command: ["node", "adapter.mjs"] },
  securityRequirements: {
    network: true,
    executable: true,
    fileAccess: "none",
    audioOutput: false,
    microphone: false,
    camera: false,
  },
  capabilities: { "visual.textBubble": { supported: true } },
  inputCapabilities: {},
  intents: ["idle"],
};

const INDEX = {
  schemaVersion: "1.0",
  default: "shu-maid",
  characters: [
    { characterId: "shu-maid", manifestPath: "/characters/shu-maid/manifest.json", origin: "builtin", persona: "persona-shu" },
    { characterId: "plain-text", manifestPath: "/characters/plain-text/manifest.json", origin: "builtin" },
    { characterId: "buddy", manifestPath: "/characters/buddy/manifest.json", origin: "builtin" },
    { characterId: "ext-bot", manifestPath: "/characters/ext-bot/manifest.json", origin: "imported" },
  ],
};

const FILES: Record<string, string> = {
  "/characters/index.json": JSON.stringify(INDEX),
  "/characters/shu-maid/manifest.json": bundled("shu-maid"),
  "/characters/plain-text/manifest.json": bundled("plain-text"),
  "/characters/buddy/manifest.json": JSON.stringify(BUDDY),
  "/characters/ext-bot/manifest.json": JSON.stringify(EXT),
};

const IMPORTED_SPRITE = {
  characterId: "imp-sprite",
  valid: true,
  displayName: { "zh-TW": "匯入的角色" },
  adapterKind: "in-process",
  entrypoint: "sprite",
  version: "1.2.0",
  executable: false,
  network: false,
  external: false,
  assets: ["sheet"],
  origin: "imported" as const,
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

const mockApi = vi.hoisted(() => ({
  uiPrefsGet: vi.fn(async () => ({ mode: "simple", locale: "zh-TW", customNames: {}, schemaVersion: "1.0" })),
  uiPrefsPatch: vi.fn(async (patch: Record<string, unknown>) => ({ mode: "simple", locale: "zh-TW", customNames: {}, schemaVersion: "1.0", ...patch })),
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
  proactiveDialogueGet: vi.fn(async () => ({ config: { mode: "natural" }, sentThisHour: 0 })),
  proactiveDialoguePatch: vi.fn(async () => ({})),
  proactiveDialogueQuiet: vi.fn(async () => ({})),
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
  return {
    state,
    prefsGet: vi.fn(async () => ({ ...state.prefs })),
    // 模擬 Rust host：未知欄位（companionPreferences）被 serde 丟掉，不會回傳。
    prefsPatch: vi.fn(async (patch: Record<string, unknown>) => {
      const { companionPreferences: _dropped, ...rest } = patch;
      Object.assign(state.prefs, rest);
      return { ...state.prefs };
    }),
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
import { CompanionPage, characterLiveState, CHARACTER_UNAVAILABLE_TEXT } from "../pages/CompanionPage";
import { boundValue, boundValues } from "../pages/character/preferences";
import { sanitizeErrorText } from "../pages/character/catalog";
import { emptyMemory } from "../companion/interactionMemory";

function renderPage(props: { advanced?: boolean } = {}) {
  return render(
    <AppStateProvider ready={true} refreshKey={0}>
      <CompanionPage refreshKey={0} {...props} />
    </AppStateProvider>
  );
}

const RIG_TERMS = /使魔|玩具|調色盤|palette|骨架|通道|channel|rig\b|manifest|schemaVersion|adapterKind|entrypoint/i;

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  mockDesktop.state.prefs = { ...BASE_PREFS };
  mockName.current = { name: "小樞", pronoun: "她", characterId: "shu-maid", loaded: true, icon: "cat" };
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

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("角色頁：更換或加入角色", () => {
  it("列出索引角色與已匯入角色，並標示內建／第三方、本機／外部、可執行、網路、可接收、已測試", async () => {
    mockDesktop.characterListImported.mockResolvedValue([IMPORTED_SPRITE]);
    renderPage();
    const badge = { selector: ".badge" };
    const shu = await screen.findByRole("article", { name: "角色 小樞" });
    expect(within(shu).getByText("內建", badge)).toBeInTheDocument();
    expect(within(shu).getByText("使用中", badge)).toBeInTheDocument();
    expect(within(shu).getByText("本機")).toBeInTheDocument();
    expect(within(shu).getByText(/是（隨 App 自動化測試）/)).toBeInTheDocument();
    expect(within(shu).queryByRole("button", { name: "移除" })).not.toBeInTheDocument();
    expect(within(shu).queryByText("有可執行程式", badge)).not.toBeInTheDocument();
    // 內建文字角色是永遠可用的退路。
    expect(await screen.findByRole("article", { name: "角色 文字角色" })).toBeInTheDocument();

    const ext = await screen.findByRole("article", { name: "角色 外部機器人" });
    expect(within(ext).getByText("第三方", badge)).toBeInTheDocument();
    expect(within(ext).getByText("外部", badge)).toBeInTheDocument();
    expect(within(ext).getByText("有可執行程式", badge)).toBeInTheDocument();
    expect(within(ext).getByText("需要網路", badge)).toBeInTheDocument();
    expect(within(ext).getByText("不接收任何輸入")).toBeInTheDocument();
    expect(within(ext).getByText(/否（未經本機測試）/)).toBeInTheDocument();
    expect(within(ext).getByRole("button", { name: "移除" })).toBeInTheDocument();

    const imported = await screen.findByRole("article", { name: "角色 匯入的角色" });
    expect(within(imported).getByText("第三方", badge)).toBeInTheDocument();
    expect(within(imported).getByText("匯入", badge)).toBeInTheDocument();
    expect(within(imported).queryByText("外部", badge)).not.toBeInTheDocument();
    expect(within(imported).getByText(/不明（角色資料尚未載入）/)).toBeInTheDocument();
    expect(within(imported).getByRole("button", { name: "移除" })).toBeInTheDocument();
    expect(within(imported).getByRole("button", { name: "選用" })).toBeInTheDocument();
  });

  it("停用 → 純文字角色；選用 → prefs.companionPack ＋ companionApplyPrefs", async () => {
    renderPage();
    // 小樞（使用中）可以停用 → 改用純文字。
    const shu = await screen.findByRole("article", { name: "角色 小樞" });
    await userEvent.click(within(shu).getByRole("button", { name: "停用" }));
    await waitFor(() => expect(mockDesktop.prefsPatch).toHaveBeenCalledWith({ companionPack: "plain-text" }));
    expect(mockDesktop.companionApplyPrefs).toHaveBeenCalled();
    expect(await screen.findByText(/已停用目前角色，改用純文字角色/)).toBeInTheDocument();
    // 純文字角色本身沒有「停用」（它就是退路）。
    const text = await screen.findByRole("article", { name: "角色 文字角色" });
    expect(await within(text).findByText("使用中", { selector: ".badge" })).toBeInTheDocument();
    expect(within(text).queryByRole("button", { name: "停用" })).not.toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "36 表情預覽" })).not.toBeInTheDocument();

    // 選用另一個角色。
    const buddy = await screen.findByRole("article", { name: "角色 阿寶" });
    await userEvent.click(within(buddy).getByRole("button", { name: "選用" }));
    await waitFor(() => expect(mockDesktop.prefsPatch).toHaveBeenLastCalledWith({ companionPack: "buddy" }));
    expect(await screen.findByText(/已改用「阿寶」/)).toBeInTheDocument();
    const buddyActive = await screen.findByRole("article", { name: "角色 阿寶" });
    expect(await within(buddyActive).findByText("使用中", { selector: ".badge" })).toBeInTheDocument();
  });

  it("移除只給匯入角色；確認後呼叫 characterRemove", async () => {
    mockDesktop.characterListImported.mockResolvedValue([IMPORTED_SPRITE]);
    renderPage();
    const imported = await screen.findByRole("article", { name: "角色 匯入的角色" });
    await userEvent.click(within(imported).getByRole("button", { name: "移除" }));
    await userEvent.click(within(imported).getByRole("button", { name: "確定移除這個角色？" }));
    await waitFor(() => expect(mockDesktop.characterRemove).toHaveBeenCalledWith("imp-sprite"));
  });

  it("匯入：本機驗證錯誤與 host 錯誤都顯示，且不回顯路徑", async () => {
    renderPage();
    await screen.findByRole("article", { name: "角色 小樞" });
    await userEvent.click(screen.getByRole("button", { name: "匯入角色…" }));
    const dialog = await screen.findByRole("dialog", { name: "匯入角色" });
    const textarea = within(dialog).getByRole("textbox", { name: "角色描述檔內容" });
    fireEvent.change(textarea, { target: { value: "{not json" } });
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("角色描述檔不符合規格");
    expect(within(dialog).getByRole("alert")).toHaveTextContent("manifest is not valid JSON");
    expect(within(dialog).getByRole("button", { name: "匯入" })).toBeDisabled();

    // 缺 displayName 的 manifest：驗證器訊息（欄位＋規則，不含內容）。
    fireEvent.change(textarea, { target: { value: JSON.stringify({ ...BUDDY, characterId: "third", displayName: {} }) } });
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(/displayName/);

    // 合法 manifest → 預覽＋可匯入；host 拒絕的訊息會被過濾掉路徑。
    mockDesktop.characterImport.mockRejectedValueOnce(
      new Error("manifest invalid: /Users/someone/.adaptive-interaction/state/characters/third/manifest.json asset sheet not provided")
    );
    fireEvent.change(textarea, { target: { value: JSON.stringify({ ...BUDDY, characterId: "third", displayName: { "zh-TW": "第三個" } }) } });
    expect(await within(dialog).findByText("第三個")).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "匯入" }));
    const alert = await within(dialog).findByRole("alert");
    expect(alert).toHaveTextContent("匯入失敗：");
    expect(alert).toHaveTextContent("asset sheet not provided");
    expect(alert.textContent).not.toContain("/Users/");
    expect(alert.textContent).not.toContain(".adaptive-interaction");
    expect(mockDesktop.characterImport).toHaveBeenCalledWith({
      manifestText: expect.stringContaining('"third"'),
      assets: [],
    });
  });
});

describe("角色頁：互動記憶可以忘記（memory-ui-002）", () => {
  it("「小樞記得」列出摘要；按「忘記這些」以空記憶寫回偏好，摘要清空", async () => {
    mockDesktop.state.prefs = {
      ...BASE_PREFS,
      companionInteractionMemory: {
        toys: [{ kind: "yarn", count: 3 }],
        disabledReactions: [],
        events: [{ at: 1, kind: "play", detail: "yarn" }],
        daysSeen: 5,
        lastDay: 20000,
        lastSeenAt: 1,
      },
    };
    renderPage();
    expect(await screen.findByRole("heading", { name: "小樞記得" })).toBeInTheDocument();
    expect(screen.getByText(/小樞記得：最喜歡的玩具是/)).toBeInTheDocument();
    expect(screen.getByText(/小樞記得：我們一起待過 5 天/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "忘記這些" }));
    await waitFor(() =>
      expect(mockDesktop.prefsPatch).toHaveBeenCalledWith({ companionInteractionMemory: emptyMemory() })
    );
    expect(await screen.findByText(/還沒有互動記憶/)).toBeInTheDocument();
    expect(screen.queryByText(/小樞記得：/)).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "忘記這些" })).not.toBeInTheDocument();
  });
});

describe("角色頁：目前角色與陪伴設定", () => {
  it("小樞：能力摘要來自 manifest、36 表情預覽只給 shu-rig、一般模式沒有技術字眼", async () => {
    renderPage();
    expect(await screen.findByRole("heading", { name: "36 表情預覽" })).toBeInTheDocument();
    const summary = await screen.findByRole("list", { name: "角色能力摘要" });
    expect(summary).toHaveTextContent("內建角色");
    expect(summary).toHaveTextContent("需要網路：否");
    expect(summary).toHaveTextContent("已測試：是");
    expect(screen.getByText("只點頭，沒有綠勾")).toBeInTheDocument();
    expect(screen.getByText("綠勾只在驗證後")).toBeInTheDocument();
    // 小樞才有玩耍設定與使魔。
    expect(screen.getByRole("checkbox", { name: /玩耍（玩具、追逐、撲抓）/ })).toBeInTheDocument();
    expect(screen.getByText(/現在大家在做什麼/)).toBeInTheDocument();
    // 一般模式：沒有技術資料。
    expect(screen.queryByText("技術資料")).not.toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/schemaVersion|adapterKind|manifest JSON|Behavior State/);
    // 分區順序。
    const headings = screen.getAllByRole("heading", { level: 2 }).map((h) => h.textContent);
    const order = ["目前角色", "外觀與名字", "平常如何陪伴", "安靜與勿擾", "主動式對話", "主動程度與安靜時段", "更換或加入角色"];
    const idx = order.map((t) => headings.indexOf(t));
    expect(idx.every((i) => i >= 0)).toBe(true);
    expect([...idx].sort((a, b) => a - b)).toEqual(idx);
  });

  it("非小樞角色：由 preferencesSchema 產生 bounded 表單、文字範例，且沒有 rig 字眼", async () => {
    mockDesktop.state.prefs = { ...BASE_PREFS, companionPack: "buddy" };
    mockName.current = { name: "阿寶", pronoun: "角色", characterId: "buddy", loaded: true, icon: "sparkles" };
    renderPage();
    const form = await screen.findByRole("group", { name: "角色偏好" });
    expect(within(form).getByRole("checkbox", { name: "愛講話" })).toBeChecked();
    const pace = within(form).getByRole("slider", { name: "步調" }) as HTMLInputElement;
    expect(pace.min).toBe("1");
    expect(pace.max).toBe("5");
    expect(pace.value).toBe("3");
    const mood = within(form).getByRole("combobox", { name: "心情" });
    expect(within(mood).getAllByRole("option").map((o) => o.textContent)).toEqual(["calm", "happy"]);
    expect((within(form).getByRole("textbox", { name: "座右銘" }) as HTMLInputElement).maxLength).toBe(20);
    // 文字角色：文字範例（綠勾只在驗證後）。
    expect(screen.getByRole("heading", { name: "文字範例" })).toBeInTheDocument();
    expect(screen.getByText("✓ 做完了，也確認過結果。")).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "36 表情預覽" })).not.toBeInTheDocument();
    // 外觀來自 manifest.variants。
    const variant = screen.getByRole("combobox", { name: "外觀" });
    expect(within(variant).getAllByRole("option").map((o) => o.textContent)).toEqual(["白天", "夜晚"]);
    // 沒有 rig／玩耍字眼，也沒有小樞專屬設定。
    expect(document.body.textContent).not.toMatch(RIG_TERMS);
    expect(screen.queryByRole("checkbox", { name: /玩耍/ })).not.toBeInTheDocument();
    expect(screen.queryByText(/使魔/)).not.toBeInTheDocument();
    // 名字與代詞來自 hook（中立）。
    expect(screen.getByRole("heading", { name: "阿寶", level: 3 })).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "可以用滑鼠把角色拖到別的位置" })).toBeInTheDocument();

    // 改值 → prefsPatch companionPreferences[buddy]（限縮後）；host 沒保存 → 誠實提示＋本機暫存。
    await userEvent.click(within(form).getByRole("checkbox", { name: "愛講話" }));
    await waitFor(() =>
      expect(mockDesktop.prefsPatch).toHaveBeenCalledWith({
        companionPreferences: { buddy: { chatty: false, pace: 3, mood: "calm", motto: "" } },
      })
    );
    expect(await screen.findByText(/尚未保存角色偏好/)).toBeInTheDocument();
    expect(JSON.parse(localStorage.getItem("adaptive-interaction.characterPreferences") ?? "{}")).toEqual({
      buddy: { chatty: false, pace: 3, mood: "calm", motto: "" },
    });
    // 超出範圍的滑桿值被 clamp。
    fireEvent.change(pace, { target: { value: "9" } });
    await waitFor(() =>
      expect(mockDesktop.prefsPatch).toHaveBeenLastCalledWith({
        companionPreferences: { buddy: { chatty: false, pace: 5, mood: "calm", motto: "" } },
      })
    );
  });

  it("角色崩潰／失聯 → 「角色目前無法顯示，改用文字」；Runtime 回報的已測試只在同一角色時採信", async () => {
    mockDesktop.state.prefs = { ...BASE_PREFS, companionPack: "ext-bot" };
    mockName.current = { name: "外部機器人", pronoun: "角色", characterId: "ext-bot", loaded: true, icon: "sparkles" };
    const instance = {
      instanceId: "desktop-companion",
      characterId: "shu-maid",
      displayName: { "zh-TW": "小樞" },
      role: "primary-companion",
      generation: 2,
      lifecycle: "crashed",
      connected: false,
      negotiated: true,
      pending: 0,
      adapterKind: "in-process",
      origin: "builtin",
      executable: false,
      network: false,
      tested: true,
    };
    mockApi.characterInstances.mockResolvedValue({ instances: [instance] });
    const first = renderPage();
    expect(await screen.findByText(CHARACTER_UNAVAILABLE_TEXT)).toBeInTheDocument();
    // 實例是另一個角色（小樞）：外部機器人的「已測試」維持否。
    const summary = await screen.findByRole("list", { name: "角色能力摘要" });
    expect(summary).toHaveTextContent("已測試：否");
    expect(summary).toHaveTextContent("需要網路：是");
    first.unmount();

    mockApi.characterInstances.mockResolvedValue({ instances: [{ ...instance, characterId: "ext-bot" }] });
    renderPage();
    expect(await screen.findByText(/已測試：是（角色視窗完成過一次完整演出並回報）/)).toBeInTheDocument();
    expect(screen.getByText(CHARACTER_UNAVAILABLE_TEXT)).toBeInTheDocument();
    // 外部角色：控制中心不預覽、不啟動。
    expect(screen.getByText(/由外部程式或裝置呈現；控制中心不會啟動它/)).toBeInTheDocument();
  });

  it("characterLiveState：崩潰／未連線／隱藏／運作中／瀏覽器沒有角色視窗", () => {
    expect(characterLiveState({ lifecycle: "crashed", connected: true }, null).label).toBe(CHARACTER_UNAVAILABLE_TEXT);
    expect(characterLiveState({ lifecycle: "ready", connected: false }, null).label).toBe(CHARACTER_UNAVAILABLE_TEXT);
    expect(characterLiveState({ lifecycle: "hidden", connected: true }, null).label).toBe("已隱藏");
    expect(characterLiveState({ lifecycle: "shown", connected: true }, { visible: true }).label).toBe("角色視窗運作中");
    expect(characterLiveState({ lifecycle: "loading", connected: true }, null).label).toBe("準備中");
    expect(characterLiveState(null, { connected: true, visible: false }).label).toBe("已隱藏");
    expect(characterLiveState(null, { connected: false }).label).toBe("角色視窗未連線");
  });

  it("進階模式才有收合的「技術資料」", async () => {
    renderPage({ advanced: true });
    await screen.findByRole("article", { name: "角色 小樞" });
    const details = screen.getByText("技術資料").closest("details");
    expect(details).not.toBeNull();
    expect(details).not.toHaveAttribute("open");
    expect(details).toHaveTextContent("schemaVersion");
    expect(details).toHaveTextContent("builtin:shu-rig");
    expect(details).toHaveTextContent("Behavior State");
  });

  it("主動對話／主動程度／安靜時段仍住在角色頁（單一主人），且文案用角色名", async () => {
    mockName.current = { name: "阿寶", pronoun: "角色", characterId: "buddy", loaded: true, icon: "sparkles" };
    mockDesktop.state.prefs = { ...BASE_PREFS, companionPack: "buddy" };
    renderPage();
    expect(await screen.findByText(/阿寶什麼情況下可以主動說話/)).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "主動程度與安靜時段" })).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: /勿擾（安靜陪伴/ })).toBeInTheDocument();
    // 一般模式不外洩 Session／Agent Session 字眼。
    expect(document.body.textContent).not.toMatch(/Session|Receptor|Actuator|Provider Registry|UUID/i);
  });
});

describe("角色頁：偏好限縮與錯誤文字", () => {
  it("boundValue／boundValues 依 schema 限縮，未宣告的鍵丟棄", () => {
    expect(boundValue({ type: "integer", minimum: 1, maximum: 5 }, "9")).toBe(5);
    expect(boundValue({ type: "integer", minimum: 1, maximum: 5 }, 2.6)).toBe(3);
    expect(boundValue({ type: "number", minimum: 0, maximum: 1 }, "abc")).toBe(0);
    expect(boundValue({ type: "boolean" }, "true")).toBe(false);
    expect(boundValue({ type: "string", enum: ["a", "b"] }, "zzz")).toBe("a");
    expect(boundValue({ type: "string", maxLength: 3 }, "hello\u0007")).toBe("hel");
    const values = boundValues(BUDDY.preferencesSchema as never, { chatty: "yes", pace: 99, mood: "happy", motto: "x", extra: 1 }, {
      variantIds: ["day", "night"],
    });
    expect(values).toEqual({ chatty: false, pace: 5, mood: "happy", motto: "x" });
    expect(boundValues(BUDDY.preferencesSchema as never, { variant: "night" }, { variantIds: ["day", "night"] }).variant).toBe("night");
    expect(boundValues(BUDDY.preferencesSchema as never, { variant: "evil" }, { variantIds: ["day", "night"] })).not.toHaveProperty("variant");
  });

  it("sanitizeErrorText 隱藏絕對路徑並限長", () => {
    expect(sanitizeErrorText(new Error("Error: failed at /Users/x/.adaptive-interaction/state/characters/a"))).not.toContain("/Users/");
    expect(sanitizeErrorText("bad C:\\Users\\x\\file.json")).not.toContain("C:\\");
    expect(sanitizeErrorText("x".repeat(500)).length).toBeLessThanOrEqual(300);
  });
});

describe("角色頁：390px", () => {
  it("關鍵容器有頁面專屬 class，且 CSS 在 700px 以下改單欄", async () => {
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 390 });
    window.dispatchEvent(new Event("resize"));
    const { container } = renderPage();
    await screen.findByRole("article", { name: "角色 小樞" });
    expect(container.querySelector(".character-page")).not.toBeNull();
    expect(container.querySelector(".character-cards")).not.toBeNull();
    expect(container.querySelector(".character-current-head")).not.toBeNull();
    expect(container.querySelector(".preview-grid")).not.toBeNull();
    // 沒有任何元素以 inline style 硬編超過 390px 的寬度。
    const wide = Array.from(container.querySelectorAll<HTMLElement>("[style]")).filter((el) => {
      const w = parseFloat(el.style.width || "0");
      return Number.isFinite(w) && el.style.width.endsWith("px") && w > 390;
    });
    expect(wide).toEqual([]);
    const css = fs.readFileSync(path.resolve("src/styles.css"), "utf8");
    const block = css.slice(css.indexOf("角色頁（CompanionPage）"), css.indexOf("H 區塊結束"));
    expect(block).toContain("@media (max-width: 700px)");
    expect(block).toMatch(/\.character-cards \{ grid-template-columns: 1fr; \}/);
    expect(block).toMatch(/\.character-pref-form \{ grid-template-columns: 1fr; \}/);
    // 頁面專屬前綴：不動其他頁的選擇器。
    for (const selector of block.match(/^\s*\.[a-zA-Z-]+/gm) ?? []) {
      expect(selector.trim()).toMatch(/^\.(character-|first-success)/);
    }
  });
});
