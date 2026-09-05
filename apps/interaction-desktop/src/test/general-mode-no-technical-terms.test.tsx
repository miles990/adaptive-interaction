// X5：一般模式不外洩技術詞。
//
// 「一般模式看不到 revision／sequence／epoch」在文案表上很容易做到——真正會漏的是
// **從 Runtime 回應直接流進畫面的值**：裝置識別碼（UUID）、來源清單的 `provider.mobile.<id>`
// 前綴、後端寫入錯誤的英文原文。所以這一支不是檢查文案常數，而是把同步卡在
// 一般模式下**真的渲染出來的 DOM 文字**整段掃過去。
//
// 進階模式是另一回事：那裡本來就該看得到技術值（「連接診斷」收合區塊），
// 所以最後一個案例反過來要求它出現——否則這個測試會退化成「把診斷刪掉就會綠」。

import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

/**
 * 一般模式的 DOM 文字一個都不得命中：技術詞＋長得像識別碼的字串。
 *
 * `generation`／`sourceId` 是 v0.6.x 新增的兩個外洩面：感測來源的世代與內部 id
 * 會經「未解決停止」與角色實例流到畫面附近，兩者都只該拿去呼叫 API。
 */
const TECHNICAL = /revision|epoch|sequence|generation|sourceid|uuid|[0-9a-f]{8}-[0-9a-f]{4}-/i;
/** 來源清單的 id 前綴（Runtime `mobile.rs` 的 `provider.mobile.<id>`）。 */
const PROVIDER_PREFIX = "provider.mobile.";

/** 刻意長得像 UUID 的裝置識別碼：只要它漏進畫面，`TECHNICAL` 就會抓到。 */
const DEVICE_ID = "3f9a2b71-4c2d-4f0e-9a11-8de1f2b3c4d5";
const FIXTURE_PHONE = "模擬 iPhone（fixture）";

const mockApi = vi.hoisted(() => ({
  characterSessionSnapshot: vi.fn(),
  characterSessionResume: vi.fn(),
  characterSessionDiagnostics: vi.fn(),
  mobileStatus: vi.fn(),
  providersList: vi.fn(),
  sensorsUnresolved: vi.fn(),
  sensorsDismissUnresolved: vi.fn(),
}));

vi.mock("../api", async (importOriginal) => {
  const original = await importOriginal<Record<string, unknown>>();
  return { ...original, api: mockApi };
});

import { CharacterSyncCard } from "../components/CharacterSyncCard";
import { UnresolvedStopsSection } from "../pages/connect/UnresolvedStops";
import { stateHash } from "../aip/canonical";

afterEach(() => {
  vi.clearAllMocks();
});

function snapshot(members: Record<string, unknown>[]): Record<string, unknown> {
  // AIP 1.0 的 snapshot 必帶 hash（決策表規則 2），而且桌面端會自己重算來核對。
  const state = {
    characterId: "character",
    mood: { kind: "neutral", intensity: 0 },
    activity: "idle",
    truth: { state: "none" },
    members,
    reducedMotion: false,
    lastInteraction: {
      name: "character.interaction.touch",
      kind: "tap",
      source: `device:${DEVICE_ID}`,
    },
  };
  return {
    specVersion: "aip/1.0",
    messageId: `msg-${DEVICE_ID}`,
    messageType: "state",
    name: "character.session.snapshot",
    payload: { kind: "snapshot", revision: 12, sessionEpoch: 3, state, hash: stateHash(state) },
  };
}

const PHONE_MEMBER = {
  party: { kind: "device", id: DEVICE_ID },
  role: "remote-renderer",
  presence: "online",
  lastSeenAt: "2026-09-05T12:30:00.000Z",
  negotiated: { intents: { settle: "exact", idle: "exact" } },
};

/** 診斷的預設值：技術數字齊全（一般模式一個都不得出現）。 */
function diagnostics(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    sessionId: `session.${DEVICE_ID}`,
    sessionEpoch: 3,
    revision: 12,
    sequence: 27,
    members: [],
    counters: { accepted: 5 },
    eventLog: { len: 3, cap: 512 },
    storeNote: null,
    ...overrides,
  };
}

function setup(options: {
  members?: Record<string, unknown>[];
  devices?: Record<string, unknown>[];
  providers?: Record<string, unknown>[];
  diagnostics?: Record<string, unknown>;
} = {}) {
  const envelope = snapshot(options.members ?? []);
  mockApi.characterSessionSnapshot.mockResolvedValue(envelope);
  mockApi.characterSessionResume.mockResolvedValue(envelope["payload"]);
  mockApi.mobileStatus.mockResolvedValue({ devices: options.devices ?? [] });
  mockApi.providersList.mockResolvedValue(options.providers ?? []);
  mockApi.characterSessionDiagnostics.mockResolvedValue(options.diagnostics ?? diagnostics());
}

/** 渲染同步卡，等它離開「正在讀取」，回傳整張卡的文字。 */
async function generalModeText(headline: RegExp): Promise<string> {
  const card = await screen.findByTestId("character-sync");
  await waitFor(() =>
    expect(within(card).getAllByText(headline).length).toBeGreaterThan(0)
  );
  return card.textContent ?? "";
}

function expectNoTechnicalTerms(text: string): void {
  expect(text, "一般模式外洩技術詞").not.toMatch(TECHNICAL);
  expect(text, "一般模式外洩來源識別碼前綴").not.toContain(PROVIDER_PREFIX);
  expect(text, "一般模式外洩裝置識別碼").not.toContain(DEVICE_ID);
}

describe("X5：同步卡在一般模式的 DOM 文字沒有技術詞", () => {
  it("已同步（有名字的裝置＋最近互動）", async () => {
    setup({
      members: [PHONE_MEMBER],
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
    });
    render(<CharacterSyncCard refreshKey={0} advanced={false} />);
    const text = await generalModeText(/角色狀態已同步/);
    expect(text).toContain(FIXTURE_PHONE);
    expectNoTechnicalTerms(text);
  });

  it("查不到名字的裝置：用中性稱呼，不退回識別碼", async () => {
    setup({ members: [PHONE_MEMBER], devices: [] });
    render(<CharacterSyncCard refreshKey={0} advanced={false} />);
    const text = await generalModeText(/能力核對中|角色狀態已同步/);
    expect(text).toContain("一台裝置");
    expectNoTechnicalTerms(text);
  });

  it("需要重新確認：指出是哪一台時只給名字", async () => {
    setup({ devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }] });
    render(<CharacterSyncCard refreshKey={0} advanced={false} />);
    const text = await generalModeText(/需要重新確認裝置/);
    expect(text).toContain(FIXTURE_PHONE);
    expectNoTechnicalTerms(text);
  });

  it("只在這台電腦使用（來源清單裡的 revoked 條目不得外洩 id）", async () => {
    setup({
      providers: [
        { identity: { id: `${PROVIDER_PREFIX}${DEVICE_ID}`, kind: "mobile" }, state: "revoked" },
      ],
    });
    render(<CharacterSyncCard refreshKey={0} advanced={false} />);
    const text = await generalModeText(/目前只在這台電腦使用/);
    expectNoTechnicalTerms(text);
  });

  it("紀錄存不下來：後端錯誤原文與技術數字都不得外洩", async () => {
    setup({
      members: [PHONE_MEMBER],
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
      diagnostics: diagnostics({
        store: {
          format: 2,
          migratedFrom: 1,
          migrationNote: "migrated from format 1",
          lastPersistedRevision: 11,
          persistFailures: 3,
          skippedStale: 0,
          parked: false,
          lastPersistError: "failed to persist revision 12 (epoch 3)",
          note: null,
        },
      }),
    });
    render(<CharacterSyncCard refreshKey={0} advanced={false} />);
    const text = await generalModeText(/寫不進去|存不下來/);
    expect(text).not.toContain("failed to persist");
    // 遷移是歷史通知，只進進階模式：一般模式連提都不提。
    expect(text).not.toContain("migrated");
    expectNoTechnicalTerms(text);
  });

  it("歷史通知（曾經重建過）也只有人話", async () => {
    setup({
      members: [PHONE_MEMBER],
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
      diagnostics: diagnostics({
        storeNote: "stored character session state was unusable; it was quarantined",
        store: {
          format: 2,
          migratedFrom: null,
          migrationNote: null,
          lastPersistedRevision: 12,
          persistFailures: 0,
          skippedStale: 0,
          parked: false,
          lastPersistError: null,
          note: null,
        },
      }),
    });
    render(<CharacterSyncCard refreshKey={0} advanced={false} />);
    const text = await generalModeText(/角色狀態已同步/);
    expect(text).toMatch(/曾經重建過/);
    expect(text).not.toContain("quarantined");
    expectNoTechnicalTerms(text);
  });

  it("進階模式才准出現技術值（否則這個測試會退化成「刪掉診斷就會綠」）", async () => {
    setup({
      members: [PHONE_MEMBER],
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
    });
    render(<CharacterSyncCard refreshKey={0} advanced />);
    const card = await screen.findByTestId("character-sync");
    await userEvent.click(await within(card).findByText("連接診斷"));
    await waitFor(() => expect(card.textContent ?? "").toMatch(TECHNICAL));
  });
});

// ---------------------------------------------------------------------------
// 未解決停止：世代與來源識別碼只准拿去呼叫 API，不准上畫面
// ---------------------------------------------------------------------------

/** 刻意長得像內部識別碼的來源 id：漏進畫面就會被 `TECHNICAL` 或字面比對抓到。 */
const SOURCE_ID = `declarative.serial.${DEVICE_ID}`;

describe("X5：未解決停止在一般模式的 DOM 文字沒有技術詞", () => {
  it("逐筆只有人話：沒有 sourceId、沒有 generation、沒有原始感測種類 id", async () => {
    mockApi.sensorsUnresolved.mockResolvedValue({
      unresolvedStops: [
        {
          sourceId: SOURCE_ID,
          generation: 42,
          sensors: ["iphone.motion"],
          since: new Date(Date.now() - 5 * 60_000).toISOString(),
          lastKnown: [{ kind: "iphone.motion", startedAt: "", startedBy: "api", purpose: "p" }],
        },
      ],
    });
    render(<UnresolvedStopsSection refreshKey={0} />);
    const section = await screen.findByTestId("unresolved-stops");
    await waitFor(() => expect(section.textContent ?? "").toContain("沒有人確認"));
    const text = section.textContent ?? "";
    expect(text).not.toContain(SOURCE_ID);
    expect(text).not.toContain("iphone.motion");
    expect(text, "一般模式外洩技術詞").not.toMatch(TECHNICAL);
    // 誠實：它沒有回答「停了沒有」，所以不得出現「已停止」。
    expect(text).not.toContain("已停止");
  });

  it("有人話名稱時用名字，沒有時用中性稱呼（都不退回識別碼）", async () => {
    mockApi.sensorsUnresolved.mockResolvedValue({
      unresolvedStops: [
        { sourceId: SOURCE_ID, generation: 1, sensors: ["microphone"], since: "", sourceLabel: FIXTURE_PHONE },
        { sourceId: SOURCE_ID, generation: 2, sensors: ["microphone"], since: "" },
      ],
    });
    render(<UnresolvedStopsSection refreshKey={0} />);
    const section = await screen.findByTestId("unresolved-stops");
    await waitFor(() => expect(section.textContent ?? "").toContain("某個裝置"));
    const text = section.textContent ?? "";
    expect(text).toContain(FIXTURE_PHONE);
    expect(text).not.toContain(SOURCE_ID);
    expect(text).not.toMatch(TECHNICAL);
  });
});
