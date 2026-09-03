// v0.5.1 對抗審查第三輪（0c845e0）：iPhone 配對面板的兩條誠實底線。
//
// mobile-server-061：配對期是一次性快照。區網上任何未認證 peer 送一則錯的
// `pair-response` 就能把它燒掉（runtime 誠實地寫進 `status.pairingBurnedAt`／
// `pairingActive:false`），畫面卻繼續顯示配對碼與「有效至 …」——使用者看到
// 的是「還有效」，實際上已作廢。面板必須自己去問 runtime 並誠實改口。
//
// mobile-server-062：iOS 的手動備援只吃完整的配對 JSON（v/host/port/fp/code），
// 單獨輸入 6 位配對碼是不行的。Bonjour 不可用又不能掃 QR（相機被占用／未授權／
// 裝置不支援）時，桌面必須把 runtime 早就回傳的 `payload` 與主機位址顯示出來，
// 否則 UI 自己的指示做不到。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";

import { api } from "../api";
import { AppStateProvider } from "../appstate";
import { MobileSection } from "../pages/CapabilitiesHub";

const HOST = "192.168.7.23";
const PAYLOAD = JSON.stringify({
  v: 1,
  host: HOST,
  port: 18790,
  fp: "abc123def456",
  code: "123456",
});

function pairingSession(overrides: Record<string, unknown> = {}) {
  return {
    code: "123456",
    expiresAt: new Date(Date.now() + 5 * 60_000).toISOString(),
    payload: PAYLOAD,
    qrSvg: "<svg role='img'></svg>",
    port: 18790,
    fingerprint: "abc123def456789",
    ...overrides,
  };
}

function mobileStatus(overrides: Record<string, unknown> = {}) {
  return {
    started: true,
    port: 18790,
    fingerprint: "abc123def456789",
    pairingActive: true,
    pairingBurnedAt: null,
    bonjour: { advertised: false, error: "mDNS daemon unavailable" },
    devices: [],
    ...overrides,
  };
}

function renderSection(advanced = false) {
  return render(
    <AppStateProvider ready={false} refreshKey={0}>
      <MobileSection refreshKey={0} advanced={advanced} />
    </AppStateProvider>
  );
}

beforeEach(() => {
  vi.spyOn(api, "status").mockResolvedValue({ activeSensors: [] } as never);
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

async function startPairing() {
  const button = await screen.findByRole("button", { name: "開始配對（5 分鐘內有效）" });
  await act(async () => {
    fireEvent.click(button);
  });
  await screen.findByText(/輸入配對碼/);
}

describe("mobile-server-061：配對期被燒掉／過期，畫面必須改口", () => {
  it("區網 peer 燒掉配對期之後，不得再顯示配對碼與「有效至」", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const status = vi.spyOn(api, "mobileStatus").mockResolvedValue(mobileStatus());
    vi.spyOn(api, "mobilePairingBegin").mockResolvedValue(pairingSession());
    renderSection();
    await startPairing();
    expect(screen.getByText("123456")).toBeInTheDocument();

    // runtime：有別的裝置試過配對，這一段已經被燒掉。
    status.mockResolvedValue(
      mobileStatus({ pairingActive: false, pairingBurnedAt: new Date().toISOString() })
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3000);
    });

    await waitFor(() =>
      expect(screen.getByTestId("pairing-invalid")).toHaveTextContent(
        "有別的裝置試過配對"
      )
    );
    expect(screen.queryByText(/輸入配對碼/)).not.toBeInTheDocument();
    expect(screen.queryByText("123456")).not.toBeInTheDocument();
    expect(screen.queryByText(/有效至/)).not.toBeInTheDocument();
  });

  it("配對期過期之後（沒有人燒掉），一樣不得宣稱還有效", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.spyOn(api, "mobileStatus").mockResolvedValue(mobileStatus());
    vi.spyOn(api, "mobilePairingBegin").mockResolvedValue(
      pairingSession({ expiresAt: new Date(Date.now() + 2000).toISOString() })
    );
    renderSection();
    await startPairing();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(4000);
    });
    await waitFor(() =>
      expect(screen.getByTestId("pairing-invalid")).toHaveTextContent("已經過期")
    );
    expect(screen.queryByText("123456")).not.toBeInTheDocument();
  });

  it("配對期還有效時不得亂喊失效（誤報等於逼使用者重來）", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.spyOn(api, "mobileStatus").mockResolvedValue(mobileStatus());
    vi.spyOn(api, "mobilePairingBegin").mockResolvedValue(pairingSession());
    renderSection();
    await startPairing();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(6000);
    });
    expect(screen.queryByTestId("pairing-invalid")).not.toBeInTheDocument();
    expect(screen.getByText("123456")).toBeInTheDocument();
  });
});

describe("mobile-server-060：配對清單讀不到＝未知，不是「還沒有配對」", () => {
  it("runtime 說 devicesUnknown 時，畫面必須說「讀不到」而不是「還沒有配對的 iPhone」", async () => {
    vi.spyOn(api, "mobileStatus").mockResolvedValue(
      mobileStatus({
        devices: [],
        devicesUnknown: true,
        devicesError: "parse: EOF while parsing an object at line 9",
      })
    );
    renderSection();
    const box = await screen.findByTestId("mobile-devices-unknown");
    expect(box).toHaveTextContent("讀不到");
    expect(screen.queryByText("還沒有配對的 iPhone。")).not.toBeInTheDocument();
  });

  it("清單是可信的空（沒配對過）時，維持原本的說法", async () => {
    vi.spyOn(api, "mobileStatus").mockResolvedValue(mobileStatus({ devices: [] }));
    renderSection();
    expect(await screen.findByText("還沒有配對的 iPhone。")).toBeInTheDocument();
    expect(screen.queryByTestId("mobile-devices-unknown")).not.toBeInTheDocument();
  });
});

describe("mobile-server-062：沒有相機也要配得成", () => {
  it("配對面板顯示可複製的配對資料（iOS 手動貼上欄位吃的那一份）與電腦位址", async () => {
    vi.spyOn(api, "mobileStatus").mockResolvedValue(mobileStatus());
    vi.spyOn(api, "mobilePairingBegin").mockResolvedValue(pairingSession());
    renderSection();
    await startPairing();

    const payload = screen.getByTestId("pairing-payload");
    expect((payload as HTMLTextAreaElement).value).toBe(PAYLOAD);
    // 手動配對至少要看得到電腦位址（UI 的指示才做得到）。
    expect(screen.getByTestId("pairing-host")).toHaveTextContent(HOST);
    expect(screen.getByRole("button", { name: "複製配對資料" })).toBeInTheDocument();
  });

  it("Bonjour 不可用時的指示，要指向畫面上真的存在的東西", async () => {
    vi.spyOn(api, "mobileStatus").mockResolvedValue(mobileStatus());
    vi.spyOn(api, "mobilePairingBegin").mockResolvedValue(pairingSession());
    renderSection();
    const notice = await screen.findByTestId("mobile-bonjour");
    expect(notice).toHaveTextContent("配對資料");
    expect(notice.textContent ?? "").not.toContain("手動輸入電腦位址");
  });
});
