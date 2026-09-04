// 角色頁的「同步」卡（AIP Character Session 的一般模式呈現）。
//
// 這一支釘住：
//   * 卡片說的每一句都來自 Runtime 的真實回應（沒有示範資料、沒有樂觀預設）；
//   * 空狀態不像成功：沒有裝置時是中性的「尚未連接 iPhone」，不是綠色徽章；
//   * 一般模式看不到 revision／sequence／counters；進階模式才有「連接診斷」；
//   * 模擬 iPhone（fixture）的名稱原樣顯示，裝置識別碼永遠不出現在畫面上；
//   * 緊急停止中固定安全句一定看得到（角色不能覆寫）。

import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const FIXTURE_PHONE = "模擬 iPhone（fixture）";
const DEVICE_ID = "iphone-87b42264";

/** 一般模式一個字都不能出現的技術詞。 */
const FORBIDDEN = /revision|sequence|epoch|schema|token|provider|lease|transport|uuid|payload|envelope/i;

const mockApi = vi.hoisted(() => ({
  characterSessionSnapshot: vi.fn(),
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

function snapshot(state: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    specVersion: "aip/1.0",
    messageType: "state",
    name: "character.session.snapshot",
    payload: {
      kind: "snapshot",
      revision: 11,
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

    setup({
      snapshot: snapshot({ members: [{ ...PHONE_MEMBER, presence: "reconnecting" }] }),
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

  it("裝置被撤銷之後要「需要重新確認裝置」，不是回到空狀態", async () => {
    setup({
      providers: [
        { identity: { id: `provider.mobile.${DEVICE_ID}`, kind: "mobile" }, state: "revoked" },
      ],
    });
    render(<CharacterSyncCard refreshKey={0} advanced={false} />);
    await waitFor(() => expect(screen.getByText("需要重新確認裝置")).toBeInTheDocument());
  });

  it("手機連著但還不是同步成員也要說需要重新確認（不算已同步）", async () => {
    setup({ devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }] });
    render(<CharacterSyncCard refreshKey={0} advanced={false} />);
    await waitFor(() => expect(screen.getByText("需要重新確認裝置")).toBeInTheDocument());
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

  it("保存的同步紀錄曾損毀時，一般模式也要說出來（不靜默、不冒充已同步）", async () => {
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
        storeNote: "stored character session state was unusable; it was quarantined",
      },
    });
    render(<CharacterSyncCard refreshKey={0} advanced={false} />);
    const card = await screen.findByTestId("character-sync");
    await waitFor(() =>
      expect(within(card).getByText("角色同步紀錄曾損毀，已重新開始")).toBeInTheDocument()
    );
    expect(card.querySelector(".badge-ok")).toBeNull();
    // 一般模式仍然只有人話：後端的英文技術原文不得外洩。
    expect(card.textContent ?? "").not.toMatch(FORBIDDEN);
    expect(card.textContent ?? "").not.toContain("quarantined");
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

  it("baseRevision 對不上就重新取一次權威狀態（不硬套、不猜）", async () => {
    setup({
      snapshot: snapshot({ members: [PHONE_MEMBER] }),
      devices: [{ deviceId: DEVICE_ID, name: FIXTURE_PHONE, connected: true }],
    });
    const view = render(<CharacterSyncCard refreshKey={0} advanced={false} sessionEvents={[]} />);
    await waitFor(() =>
      expect(screen.getByText("iPhone 已連接，角色狀態已同步")).toBeInTheDocument()
    );

    // 之後改成「重新取到的是離線」，再送一則接不上的 patch。
    setup({
      snapshot: snapshot({ members: [{ ...PHONE_MEMBER, presence: "offline" }] }),
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
    await waitFor(() => expect(mockApi.characterSessionSnapshot).toHaveBeenCalledTimes(2));
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
    await waitFor(() => expect(mockApi.characterSessionSnapshot).toHaveBeenCalledTimes(2));
  });
});
