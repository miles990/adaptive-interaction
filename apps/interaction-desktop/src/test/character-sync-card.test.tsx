// 角色頁的「同步」卡（AIP Character Session 的一般模式呈現）。
//
// 這一支釘住：
//   * 卡片說的每一句都來自 Runtime 的真實回應（沒有示範資料、沒有樂觀預設）；
//   * 空狀態不像成功：沒有裝置時是中性的「尚未連接 iPhone」，不是綠色徽章；
//   * 一般模式看不到 revision／sequence／counters；進階模式才有「連接診斷」；
//   * 模擬 iPhone（fixture）的名稱原樣顯示，裝置識別碼永遠不出現在畫面上；
//   * 緊急停止中固定安全句一定看得到（角色不能覆寫）。

import { afterEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const FIXTURE_PHONE = "模擬 iPhone（fixture）";
const DEVICE_ID = "iphone-87b42264";

/** 一般模式一個字都不能出現的技術詞。 */
const FORBIDDEN = /revision|sequence|epoch|schema|token|provider|lease|transport|uuid|payload|envelope/i;

const mockApi = vi.hoisted(() => ({
  characterSessionSnapshot: vi.fn(),
  characterSessionResume: vi.fn(),
  characterSessionDiagnostics: vi.fn(),
  mobileStatus: vi.fn(),
  providersList: vi.fn(),
}));

vi.mock("../api", async (importOriginal) => {
  const original = await importOriginal<Record<string, unknown>>();
  return { ...original, api: mockApi };
});

import { CharacterSyncCard } from "../components/CharacterSyncCard";
import type { RuntimeEvent } from "../api";

function snapshot(
  state: Record<string, unknown> = {},
  revision = 11
): Record<string, unknown> {
  return {
    specVersion: "aip/1.0",
    messageId: `msg-snapshot-${revision}`,
    messageType: "state",
    name: "character.session.snapshot",
    payload: {
      kind: "snapshot",
      revision,
      sessionEpoch: 1,
      state: {
        characterId: "character",
        mood: { kind: "neutral", intensity: 0 },
        activity: "idle",
        truth: { state: "none" },
        members: [],
        reducedMotion: false,
        ...state,
      },
    },
  };
}

/**
 * 一台「協商結果齊全」的手機：每個 host intent 都演得出來。
 *
 * `negotiated` 是契約 §11 判定「已同步 vs 部分能力目前不可用」唯一正當的訊號
 * （成員自報的 role 不是）。沒有這個欄位時卡片只會說「已連接，能力核對中」——
 * 那才是誠實的，綠勾只給真的（對抗審查 capability-consent-052／general-mode-ux-022）。
 */
const NEGOTIATED_FULL = {
  intents: {
    "react-happily-to-touch": "exact",
    celebrate: "exact",
    settle: "exact",
    idle: "exact",
  },
};

const PHONE_MEMBER = {
  party: { kind: "device", id: DEVICE_ID },
  role: "remote-renderer",
  presence: "online",
  lastSeenAt: "2026-09-04T12:30:00.000Z",
  negotiated: NEGOTIATED_FULL,
};

function setup(options: {
  snapshot?: Record<string, unknown> | Error;
  devices?: Record<string, unknown>[];
  providers?: Record<string, unknown>[];
  diagnostics?: Record<string, unknown>;
} = {}) {
  mockApi.characterSessionSnapshot.mockImplementation(async () => {
    if (options.snapshot instanceof Error) throw options.snapshot;
    return options.snapshot ?? snapshot();
  });
  // 有本地副本時對齊走 `POST /v1/character-session/resume`，它回的是 **payload**
  // （transport-bindings §1.3：少一層 envelope），host 完全對齊時是空的 patches。
  mockApi.characterSessionResume.mockImplementation(async () => {
    if (options.snapshot instanceof Error) throw options.snapshot;
    const envelope = (options.snapshot ?? snapshot()) as Record<string, unknown>;
    return envelope["payload"] as Record<string, unknown>;
  });
  mockApi.mobileStatus.mockResolvedValue({ devices: options.devices ?? [] });
  mockApi.providersList.mockResolvedValue(options.providers ?? []);
  mockApi.characterSessionDiagnostics.mockResolvedValue(
    options.diagnostics ?? {
      sessionId: "session.home",
      sessionEpoch: 1,
      revision: 11,
      sequence: 18,
      members: [],
      counters: { accepted: 3, applied: 3 },
      eventLog: { len: 9, cap: 512 },
      storeNote: null,
    }
  );
}

/** SSE `character.session.state` 事件（payload 是完整 AIP envelope）。 */
function stateEvent(
  sequence: number,
  payload: Record<string, unknown>,
  baseRevision?: number
): RuntimeEvent {
  return {
    eventId: `e${sequence}`,
    sequence,
    eventType: "character.session.state",
    timestamp: "2026-09-04T12:30:00.000Z",
    payload: {
      specVersion: "aip/1.0",
      messageType: "state",
      name: "character.session.patch",
      ...(baseRevision === undefined ? {} : { baseRevision }),
      payload,
    },
  };
}

afterEach(() => {
  vi.clearAllMocks();
});

describe("角色頁「同步」卡：一般模式", () => {
  it("沒有裝置時是中性的空狀態，不是成功（不得出現綠色徽章）", async () => {
    setup();
    render(<CharacterSyncCard refreshKey={0} advanced={false} />);
    const card = await screen.findByTestId("character-sync");
    await waitFor(() => expect(within(card).getByText("尚未連接 iPhone")).toBeInTheDocument());
    expect(within(card).getByText("尚未連接 iPhone").className).toContain("badge-muted");
    expect(card.querySelector(".badge-ok")).toBeNull();
    expect(within(card).queryByText(/已同步/)).not.toBeInTheDocument();
  });

  it("手機在線＝「iPhone 已連接，角色狀態已同步」，成員清單用手機的名字", async () => {
    setup({
      snapshot: snapshot({ members: [PHONE_MEMBER] }),
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
    });
    render(<CharacterSyncCard refreshKey={0} advanced={false} />);
    const card = await screen.findByTestId("character-sync");
    await waitFor(() =>
      expect(within(card).getByText("iPhone 已連接，角色狀態已同步")).toBeInTheDocument()
    );
    const members = within(card).getByRole("list", { name: "同步中的裝置" });
    expect(within(members).getByText(FIXTURE_PHONE)).toBeInTheDocument();
    expect(within(members).getByText("已連接")).toBeInTheDocument();
    // 裝置識別碼永遠不進畫面。
    expect(card.textContent ?? "").not.toContain(DEVICE_ID);
  });

  it("離線與重新連線都照實說（不把離線寫成沒有裝置）", async () => {
    setup({
      snapshot: snapshot({ members: [{ ...PHONE_MEMBER, presence: "offline" }] }),
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: false }],
    });
    const view = render(<CharacterSyncCard refreshKey={0} advanced={false} />);
    await waitFor(() => expect(screen.getByText("iPhone 暫時離線")).toBeInTheDocument());
    expect(screen.getByText("離線")).toBeInTheDocument();

    // host 前進了一版：對齊回來的 revision 必須比本地新，落後的一律被 rollback 防護擋掉。
    setup({
      snapshot: snapshot({ members: [{ ...PHONE_MEMBER, presence: "reconnecting" }] }, 12),
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
    });
    view.rerender(<CharacterSyncCard refreshKey={1} advanced={false} />);
    await waitFor(() => expect(screen.getByText("iPhone 正在重新連線")).toBeInTheDocument());
    expect(screen.getByText("重新連線中")).toBeInTheDocument();
  });

  it("最近互動是人話，不是事件代號", async () => {
    setup({
      snapshot: snapshot({
        members: [PHONE_MEMBER],
        lastInteraction: {
          name: "character.interaction.touch",
          kind: "tap",
          source: `device:${DEVICE_ID}`,
          at: "2026-09-04T12:30:00.000Z",
        },
      }),
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
    });
    render(<CharacterSyncCard refreshKey={0} advanced={false} />);
    const card = await screen.findByTestId("character-sync");
    await waitFor(() =>
      expect(within(card).getByText(`最近互動：${FIXTURE_PHONE}摸了摸角色`)).toBeInTheDocument()
    );
    expect(card.textContent ?? "").not.toContain("character.interaction");
  });

  it("一般模式沒有任何技術數字；進階模式才出現「連接診斷」", async () => {
    setup({
      snapshot: snapshot({ members: [PHONE_MEMBER] }),
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
    });
    const view = render(<CharacterSyncCard refreshKey={0} advanced={false} />);
    const card = await screen.findByTestId("character-sync");
    await waitFor(() =>
      expect(within(card).getByText("iPhone 已連接，角色狀態已同步")).toBeInTheDocument()
    );
    expect(card.textContent ?? "").not.toMatch(FORBIDDEN);
    expect(within(card).queryByText("連接診斷")).not.toBeInTheDocument();
    // 診斷**會**被讀（`storeNote` 是「紀錄曾損毀」唯一的來源，不得靜默），
    // 但一般模式一個數字都不得顯示——上面兩行才是紅線。
    expect(card.querySelector(".tech-details")).toBeNull();

    view.rerender(<CharacterSyncCard refreshKey={1} advanced />);
    await waitFor(() => expect(screen.getByText("連接診斷")).toBeInTheDocument());
    await userEvent.click(screen.getByText("連接診斷"));
    await waitFor(() => expect(screen.getByText(/revision/)).toBeInTheDocument());
  });

  it("進階模式的連接診斷印出每個成員的 identityStrength 原始值；查不到就是「—」，永遠不翻成「已驗證身分」", async () => {
    setup({
      snapshot: snapshot({ members: [PHONE_MEMBER] }),
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
      diagnostics: {
        sessionId: "session.home",
        sessionEpoch: 1,
        revision: 11,
        sequence: 18,
        members: [
          {
            party: { kind: "device", id: "esp32-desk" },
            role: "remote-renderer",
            presence: "online",
            lastSeenAt: "2026-09-05T00:00:00Z",
            identityStrength: "transport-hello+device-side-pairing",
          },
          {
            party: { kind: "device", id: "restored-from-snapshot" },
            role: "remote-renderer",
            presence: "online",
            lastSeenAt: "2026-09-05T00:00:00Z",
          },
        ],
        counters: { accepted: 3, applied: 3 },
        eventLog: { len: 9, cap: 512 },
        storeNote: null,
      },
    });
    const view = render(<CharacterSyncCard refreshKey={0} advanced={false} />);
    const card = await screen.findByTestId("character-sync");
    await waitFor(() => expect(mockApi.characterSessionDiagnostics).toHaveBeenCalled());
    expect(card.textContent ?? "").not.toContain("identityStrength");
    expect(card.textContent ?? "").not.toContain("esp32-desk");

    view.rerender(<CharacterSyncCard refreshKey={1} advanced />);
    await waitFor(() => expect(screen.getByText("連接診斷")).toBeInTheDocument());
    await userEvent.click(screen.getByText("連接診斷"));
    await waitFor(() =>
      expect(
        screen.getByText(/member device:esp32-desk online identityStrength transport-hello\+device-side-pairing/)
      ).toBeInTheDocument()
    );
    expect(
      screen.getByText(/member device:restored-from-snapshot online identityStrength —/)
    ).toBeInTheDocument();
    expect(document.body.textContent ?? "").not.toContain("已驗證身分");
  });

  it("讀不到權威狀態就說「同步尚未完成」，連續失敗才升級成「無法恢復」", async () => {
    setup({ snapshot: new Error("500: boom") });
    const view = render(<CharacterSyncCard refreshKey={0} advanced={false} />);
    await waitFor(() => expect(screen.getByText("同步尚未完成")).toBeInTheDocument());
    view.rerender(<CharacterSyncCard refreshKey={1} advanced={false} />);
    view.rerender(<CharacterSyncCard refreshKey={2} advanced={false} />);
    await waitFor(() => expect(screen.getByText("無法恢復，請重新連接")).toBeInTheDocument());
    // 錯誤訊息不回顯後端原文。
    expect(document.body.textContent ?? "").not.toContain("boom");
  });

  it("Runtime 關閉角色同步時誠實說關閉，不說成沒有裝置", async () => {
    setup({ snapshot: new Error('503: {"code":"session-disabled"}') });
    render(<CharacterSyncCard refreshKey={0} advanced={false} />);
    await waitFor(() => expect(screen.getByText("角色同步目前關閉")).toBeInTheDocument());
    expect(screen.queryByText("尚未連接 iPhone")).not.toBeInTheDocument();
  });

  // M3 §4.3：撤銷／移除是終態。Runtime 的來源清單永遠留著 revoked 條目，舊行為
  // 因此在零裝置時永遠亮著「需要重新確認裝置」——一個使用者做完該做的事之後
  // 仍然亮著、而且按不動的警告。
  it("手機被移除之後是「目前只在這台電腦使用」，不是永遠亮著的「需要重新確認」", async () => {
    setup({
      providers: [
        { identity: { id: `provider.mobile.${DEVICE_ID}`, kind: "mobile" }, state: "revoked" },
      ],
    });
    render(<CharacterSyncCard refreshKey={0} advanced={false} />);
    const card = await screen.findByTestId("character-sync");
    await waitFor(() =>
      expect(within(card).getByText("目前只在這台電腦使用")).toBeInTheDocument()
    );
    expect(within(card).queryByText("需要重新確認裝置")).not.toBeInTheDocument();
    // 安全效果不變：被移除的手機不會自動回來，要用得重新配對。
    expect(card.textContent ?? "").toMatch(/不會自動回來/);
    expect(card.textContent ?? "").toMatch(/重新配對/);
  });

  it("手機連著但還不是同步成員也要說需要重新確認，並指出是哪一台", async () => {
    setup({ devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }] });
    render(<CharacterSyncCard refreshKey={0} advanced={false} />);
    const card = await screen.findByTestId("character-sync");
    await waitFor(() =>
      expect(within(card).getByText("需要重新確認裝置")).toBeInTheDocument()
    );
    await waitFor(() => expect(card.textContent ?? "").toContain(FIXTURE_PHONE));
    expect(card.textContent ?? "").not.toContain(DEVICE_ID);
  });

  it("緊急停止中固定安全句一定看得到（角色不能覆寫）", async () => {
    setup({
      snapshot: snapshot({ members: [PHONE_MEMBER], truth: { state: "emergency" } }),
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
    });
    render(<CharacterSyncCard refreshKey={0} advanced={false} />);
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("緊急停止中：角色已停止表演，解除前不會接受任何互動。");
    // 安全狀態壓過同步狀態：緊急停止中不得出現綠色徽章（會和安全句互相矛盾）。
    const card = screen.getByTestId("character-sync");
    expect(card.querySelector(".badge-ok")).toBeNull();
    // 句子本身照實不改：已同步就是已同步，只是不給綠色。
    expect(within(card).getByText("iPhone 已連接，角色狀態已同步")).toBeInTheDocument();
  });

  // M3 §4.3b：保存層的兩類訊號要分開。
  it("紀錄現在存不下來＝active issue，壓過綠色並給一句人話", async () => {
    setup({
      snapshot: snapshot({ members: [PHONE_MEMBER] }),
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
      diagnostics: {
        sessionId: "session.home",
        sessionEpoch: 2,
        revision: 1,
        sequence: 0,
        members: [],
        counters: {},
        eventLog: { len: 0, cap: 512 },
        storeNote: null,
        store: {
          format: 2,
          migratedFrom: null,
          migrationNote: null,
          lastPersistedRevision: null,
          persistFailures: 0,
          skippedStale: 0,
          parked: true,
          lastPersistError: null,
          note: "backup failed",
        },
      },
    });
    render(<CharacterSyncCard refreshKey={0} advanced={false} />);
    const card = await screen.findByTestId("character-sync");
    await waitFor(() =>
      expect(within(card).getByText("同步紀錄暫時存不下來")).toBeInTheDocument()
    );
    expect(card.querySelector(".badge-ok")).toBeNull();
    // 一般模式仍然只有人話：後端的英文技術原文不得外洩。
    expect(card.textContent ?? "").not.toMatch(FORBIDDEN);
    expect(card.textContent ?? "").not.toContain("backup failed");
  });

  it("曾經重建過但現在存得下來＝歷史通知，不再壓過「已同步」", async () => {
    setup({
      snapshot: snapshot({ members: [PHONE_MEMBER] }),
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
      diagnostics: {
        sessionId: "session.home",
        sessionEpoch: 2,
        revision: 9,
        sequence: 3,
        members: [],
        counters: {},
        eventLog: { len: 0, cap: 512 },
        storeNote: "stored character session state was unusable; it was quarantined",
        store: {
          format: 2,
          migratedFrom: null,
          migrationNote: null,
          lastPersistedRevision: 9,
          persistFailures: 0,
          skippedStale: 0,
          parked: false,
          lastPersistError: null,
          note: null,
        },
      },
    });
    render(<CharacterSyncCard refreshKey={0} advanced={false} />);
    const card = await screen.findByTestId("character-sync");
    await waitFor(() =>
      expect(within(card).getByText("iPhone 已連接，角色狀態已同步")).toBeInTheDocument()
    );
    // 通知照說（不靜默），但它是 muted 的附註，不是一個永遠亮著的警告。
    await waitFor(() => expect(card.textContent ?? "").toMatch(/曾經重建過/));
    expect(card.textContent ?? "").not.toMatch(FORBIDDEN);
    expect(card.textContent ?? "").not.toContain("quarantined");
  });
});

describe("角色頁「同步」卡：下一步（M3 §4.2）", () => {
  it("沒有 onNavigate 就不渲染主要動作按鈕（不給按不動的按鈕）", async () => {
    setup();
    render(<CharacterSyncCard refreshKey={0} advanced={false} />);
    const card = await screen.findByTestId("character-sync");
    await waitFor(() => expect(within(card).getByText("尚未連接 iPhone")).toBeInTheDocument());
    expect(within(card).queryByTestId("character-sync-action")).not.toBeInTheDocument();
    // 「重新檢查」不是那顆主要動作按鈕，它一直都在。
    expect(within(card).getByRole("button", { name: "重新檢查" })).toBeInTheDocument();
  });

  it("零裝置：一鍵到配對區（connect / providers）", async () => {
    setup();
    const onNavigate = vi.fn();
    render(<CharacterSyncCard refreshKey={0} advanced={false} onNavigate={onNavigate} />);
    const card = await screen.findByTestId("character-sync");
    const action = await within(card).findByTestId("character-sync-action");
    expect(action).toHaveAttribute("data-action", "connect-phone");
    await userEvent.click(action);
    expect(onNavigate).toHaveBeenCalledWith("connect", { hub: "providers" });
  });

  it("已同步時不催促：沒有主要動作按鈕", async () => {
    setup({
      snapshot: snapshot({ members: [PHONE_MEMBER] }),
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
    });
    const onNavigate = vi.fn();
    render(<CharacterSyncCard refreshKey={0} advanced={false} onNavigate={onNavigate} />);
    const card = await screen.findByTestId("character-sync");
    await waitFor(() =>
      expect(within(card).getByText("iPhone 已連接，角色狀態已同步")).toBeInTheDocument()
    );
    expect(within(card).queryByTestId("character-sync-action")).not.toBeInTheDocument();
  });

  it("需要重新確認：一鍵到配對區，並帶上是哪一台", async () => {
    setup({ devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }] });
    const onNavigate = vi.fn();
    render(<CharacterSyncCard refreshKey={0} advanced={false} onNavigate={onNavigate} />);
    const card = await screen.findByTestId("character-sync");
    const action = await within(card).findByTestId("character-sync-action");
    expect(action).toHaveAttribute("data-action", "reconfirm-device");
    await userEvent.click(action);
    expect(onNavigate).toHaveBeenCalledWith("connect", { hub: "providers", deviceId: DEVICE_ID });
  });

  it("storage-help 是說明不是導覽：不給導覽按鈕，但話要說清楚", async () => {
    setup({
      snapshot: snapshot({ members: [PHONE_MEMBER] }),
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
      diagnostics: {
        sessionId: "session.home",
        sessionEpoch: 2,
        revision: 1,
        sequence: 0,
        members: [],
        counters: {},
        eventLog: { len: 0, cap: 512 },
        storeNote: null,
        store: {
          format: 2,
          migratedFrom: null,
          migrationNote: null,
          lastPersistedRevision: 3,
          persistFailures: 5,
          skippedStale: 0,
          parked: false,
          lastPersistError: "disk full",
          note: null,
        },
      },
    });
    const onNavigate = vi.fn();
    render(<CharacterSyncCard refreshKey={0} advanced={false} onNavigate={onNavigate} />);
    const card = await screen.findByTestId("character-sync");
    await waitFor(() => expect(card.textContent ?? "").toMatch(/寫不進去/));
    expect(within(card).queryByTestId("character-sync-action")).not.toBeInTheDocument();
    expect(card.textContent ?? "").not.toContain("disk full");
  });
});

describe("角色頁「同步」卡：不靠輪詢對齊", () => {
  it("SSE 的 state patch 直接套用，不重取權威狀態", async () => {
    setup({
      snapshot: snapshot({ members: [PHONE_MEMBER] }),
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
    });
    const view = render(<CharacterSyncCard refreshKey={0} advanced={false} sessionEvents={[]} />);
    const card = await screen.findByTestId("character-sync");
    await waitFor(() =>
      expect(within(card).getByText("iPhone 已連接，角色狀態已同步")).toBeInTheDocument()
    );
    expect(mockApi.characterSessionSnapshot).toHaveBeenCalledTimes(1);

    // 手機離線的 patch：baseRevision 對得上 → 直接套在本地副本上。
    view.rerender(
      <CharacterSyncCard
        refreshKey={0}
        advanced={false}
        sessionEvents={[
          stateEvent(
            1,
            {
              kind: "patch",
              revision: 12,
              sessionEpoch: 1,
              patch: { members: [{ ...PHONE_MEMBER, presence: "offline" }] },
            },
            11
          ),
        ]}
      />
    );
    await waitFor(() => expect(screen.getByText("iPhone 暫時離線")).toBeInTheDocument());
    expect(mockApi.characterSessionSnapshot).toHaveBeenCalledTimes(1);
  });

  it("baseRevision 對不上就重新對齊（走 resume，不硬套、不猜）", async () => {
    setup({
      snapshot: snapshot({ members: [PHONE_MEMBER] }),
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
    });
    const view = render(<CharacterSyncCard refreshKey={0} advanced={false} sessionEvents={[]} />);
    await waitFor(() =>
      expect(screen.getByText("iPhone 已連接，角色狀態已同步")).toBeInTheDocument()
    );

    // 之後改成「對齊回來的是離線」，再送一則接不上的 patch。
    setup({
      snapshot: snapshot({ members: [{ ...PHONE_MEMBER, presence: "offline" }] }, 12),
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
    });
    view.rerender(
      <CharacterSyncCard
        refreshKey={0}
        advanced={false}
        sessionEvents={[
          stateEvent(
            2,
            { kind: "patch", revision: 99, sessionEpoch: 1, patch: { activity: "resting" } },
            98
          ),
        ]}
      />
    );
    // 已經有本地副本時對齊走 resume：GET 會**消耗**一個權威 session sequence，
    // 一個唯讀畫面不該推著它前進（`docs/aip/transport-bindings.md` §2）。
    await waitFor(() => expect(mockApi.characterSessionResume).toHaveBeenCalledTimes(1));
    expect(mockApi.characterSessionSnapshot).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(screen.getByText("iPhone 暫時離線")).toBeInTheDocument());
  });

  it("慢的初次讀取被新的 SSE 超車之後，不得把畫面倒回舊狀態", async () => {
    setup({
      snapshot: snapshot({ members: [PHONE_MEMBER] }),
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
    });
    let release: (value: unknown) => void = () => {};
    mockApi.characterSessionSnapshot.mockImplementation(
      () =>
        new Promise((resolve) => {
          release = resolve;
        })
    );
    const view = render(<CharacterSyncCard refreshKey={0} advanced={false} sessionEvents={[]} />);
    // 初次 GET 還在飛，SSE 先把 revision 40 的權威快照送到。
    view.rerender(
      <CharacterSyncCard
        refreshKey={0}
        advanced={false}
        sessionEvents={[
          stateEvent(1, {
            kind: "snapshot",
            revision: 40,
            sessionEpoch: 1,
            state: {
              characterId: "character",
              truth: { state: "none" },
              members: [{ ...PHONE_MEMBER, presence: "reconnecting" }],
            },
          }),
        ]}
      />
    );
    await waitFor(() => expect(screen.getByText("iPhone 正在重新連線")).toBeInTheDocument());

    // 現在那個慢的 GET 才回來，帶的是 revision 11 的舊狀態。
    await act(async () => {
      release(snapshot({ members: [PHONE_MEMBER] }));
    });
    expect(screen.getByText("iPhone 正在重新連線")).toBeInTheDocument();
    expect(screen.queryByText("iPhone 已連接，角色狀態已同步")).not.toBeInTheDocument();
  });

  it("卸載重掛之後重新 GET 一次權威狀態（本地副本不跨掛載）", async () => {
    setup({
      snapshot: snapshot({ members: [PHONE_MEMBER] }),
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
    });
    const view = render(<CharacterSyncCard refreshKey={0} advanced={false} sessionEvents={[]} />);
    await waitFor(() =>
      expect(screen.getByText("iPhone 已連接，角色狀態已同步")).toBeInTheDocument()
    );
    view.unmount();
    render(<CharacterSyncCard refreshKey={0} advanced={false} sessionEvents={[]} />);
    await waitFor(() => expect(mockApi.characterSessionSnapshot).toHaveBeenCalledTimes(2));
    expect(mockApi.characterSessionResume).not.toHaveBeenCalled();
  });

  it("連線狀態變化（connectionKey）會重新對齊一次，而不是每則事件都重問", async () => {
    setup({
      snapshot: snapshot({ members: [PHONE_MEMBER] }),
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
    });
    const view = render(
      <CharacterSyncCard refreshKey={0} advanced={false} sessionEvents={[]} connectionKey={0} />
    );
    await waitFor(() =>
      expect(screen.getByText("iPhone 已連接，角色狀態已同步")).toBeInTheDocument()
    );
    expect(mockApi.characterSessionResume).not.toHaveBeenCalled();

    setup({
      snapshot: snapshot({ members: [{ ...PHONE_MEMBER, presence: "offline" }] }, 12),
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
    });
    view.rerender(
      <CharacterSyncCard refreshKey={0} advanced={false} sessionEvents={[]} connectionKey={1} />
    );
    await waitFor(() => expect(mockApi.characterSessionResume).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(screen.getByText("iPhone 暫時離線")).toBeInTheDocument());
  });

  it("SSE 的 snapshot 事件直接對齊，不必再問一次", async () => {
    setup({
      snapshot: snapshot({ members: [PHONE_MEMBER] }),
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
    });
    const view = render(<CharacterSyncCard refreshKey={0} advanced={false} sessionEvents={[]} />);
    await waitFor(() =>
      expect(screen.getByText("iPhone 已連接，角色狀態已同步")).toBeInTheDocument()
    );
    view.rerender(
      <CharacterSyncCard
        refreshKey={0}
        advanced={false}
        sessionEvents={[
          stateEvent(3, {
            kind: "snapshot",
            revision: 40,
            sessionEpoch: 1,
            state: {
              characterId: "character",
              truth: { state: "none" },
              members: [{ ...PHONE_MEMBER, presence: "reconnecting" }],
            },
          }),
        ]}
      />
    );
    await waitFor(() => expect(screen.getByText("iPhone 正在重新連線")).toBeInTheDocument());
    expect(mockApi.characterSessionSnapshot).toHaveBeenCalledTimes(1);
  });

  it("hash 對不上的補丁不套用，而且在進階診斷裡留下痕跡（不靜默）", async () => {
    setup({
      snapshot: snapshot({ members: [PHONE_MEMBER] }),
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
    });
    const view = render(<CharacterSyncCard refreshKey={0} advanced sessionEvents={[]} />);
    await waitFor(() =>
      expect(screen.getByText("iPhone 已連接，角色狀態已同步")).toBeInTheDocument()
    );

    // 這則補丁自己宣告的 hash 與套用後的本地狀態對不上：不得套用。
    view.rerender(
      <CharacterSyncCard
        refreshKey={0}
        advanced
        sessionEvents={[
          stateEvent(
            1,
            {
              kind: "patch",
              revision: 12,
              sessionEpoch: 1,
              patch: { members: [{ ...PHONE_MEMBER, presence: "offline" }] },
              hash: "0".repeat(64),
            },
            11
          ),
        ]}
      />
    );
    await userEvent.click(screen.getByText("連接診斷"));
    await waitFor(() =>
      expect(screen.getByText("alignment.hashMismatch 1")).toBeInTheDocument()
    );
    // 沒有套用：畫面仍然是 patch 之前的樣子。
    expect(screen.queryByText("iPhone 暫時離線")).not.toBeInTheDocument();
  });

  it("每一則 runtime 事件都重畫時，裝置清單的重取要被節流", async () => {
    setup({
      snapshot: snapshot({ members: [PHONE_MEMBER] }),
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
    });
    const view = render(<CharacterSyncCard refreshKey={0} advanced={false} sessionEvents={[]} />);
    await waitFor(() => expect(mockApi.mobileStatus).toHaveBeenCalledTimes(1));
    for (let i = 1; i <= 8; i += 1) {
      view.rerender(<CharacterSyncCard refreshKey={i} advanced={false} sessionEvents={[]} />);
    }
    await waitFor(() =>
      expect(screen.getByText("iPhone 已連接，角色狀態已同步")).toBeInTheDocument()
    );
    // 八則事件在 2 秒的節流窗內：最多再多一次（trailing），不是八次。
    expect(mockApi.mobileStatus.mock.calls.length).toBeLessThanOrEqual(2);
    expect(mockApi.providersList.mock.calls.length).toBeLessThanOrEqual(2);
  });
});

describe("角色頁「同步」卡：可及性", () => {
  it("卡片有可及名稱，而且鍵盤到得了（「重新檢查」按鈕）", async () => {
    setup();
    render(<CharacterSyncCard refreshKey={0} advanced={false} />);
    const card = await screen.findByRole("region", { name: "角色同步" });
    const recheck = within(card).getByRole("button", { name: "重新檢查" });
    recheck.focus();
    expect(recheck).toHaveFocus();
    await userEvent.click(recheck);
    // 沒有本地副本（這個 setup 的成員是空的，但快照讀得到）時仍然有一次對齊請求。
    await waitFor(() =>
      expect(
        mockApi.characterSessionResume.mock.calls.length + mockApi.characterSessionSnapshot.mock.calls.length
      ).toBeGreaterThan(1)
    );
  });

  it("有本地副本時「重新檢查」走 resume，不再消耗一個權威快照的 sequence", async () => {
    setup({
      snapshot: snapshot({ members: [PHONE_MEMBER] }),
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
    });
    render(<CharacterSyncCard refreshKey={0} advanced={false} sessionEvents={[]} />);
    await waitFor(() =>
      expect(screen.getByText("iPhone 已連接，角色狀態已同步")).toBeInTheDocument()
    );
    expect(mockApi.characterSessionSnapshot).toHaveBeenCalledTimes(1);

    mockApi.characterSessionResume.mockResolvedValueOnce(
      (snapshot({ members: [{ ...PHONE_MEMBER, presence: "offline" }] }, 12) as Record<string, unknown>)[
        "payload"
      ]
    );
    await userEvent.click(screen.getByRole("button", { name: "重新檢查" }));
    await waitFor(() => expect(screen.getByText("iPhone 暫時離線")).toBeInTheDocument());
    expect(mockApi.characterSessionResume).toHaveBeenCalledWith({
      lastRevision: 11,
      lastSequence: 0,
      epoch: 1,
    });
    expect(mockApi.characterSessionSnapshot).toHaveBeenCalledTimes(1);
  });
});
