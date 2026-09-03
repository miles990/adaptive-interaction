// 「現在」頁（一般模式）第一屏：只回答三件事——角色現在怎麼樣（角色狀態一句話＋
// 可信文字 fallback）、正在做什麼（進行中工作以 statusProjection 投影）、有什麼需要
// 處理（待決定）——加五個快速操作（交代一件事／暫停或恢復主動互動／加入裝置／
// 停止所有感測／緊急停止）；數量／系統狀態收在「詳細狀態」折疊區，展開才出現。
// 「停止所有感測」必須誠實：送出 ≠ 已停止，重讀狀態還有感測就不得說已停止。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AgentSessionRecord, api } from "../api";
import { AppStateProvider } from "../appstate";
import { resetCharacterNameForTests } from "../characterName";
import {
  CHARACTER_OFFLINE_LINE,
  characterSentence,
  HomePage,
  NowStrip,
  sensorLabel,
  WORK_PREFILL_KEY,
} from "../pages/HomePage";

const SHU = {
  characterId: "shu-maid",
  displayName: { "zh-TW": "小樞", en: "Shu" },
  pronouns: { "zh-TW": "她", en: "she" },
};

const SESSION: AgentSessionRecord = {
  sessionId: "sess-1",
  providerId: "p-1",
  agentId: "claude-code",
  label: "整理測試報告",
  state: "active",
  lease: { issuedAt: "2026-01-01T00:00:00Z", expiresAt: "2126-01-01T01:00:00Z", renewable: true },
  dataScope: ["workspace:/tmp/repo"],
  toolScope: [],
  consentScope: [],
  budget: { maxMessages: 10, spentMessages: 1, maxCost: 0, spentCost: 0 },
  createdAt: "2026-01-01T00:00:00Z",
};

function status(overrides: Record<string, unknown> = {}) {
  return {
    emergencyStop: false,
    presentation: { connected: true, visible: true },
    characterProtocol: {
      version: "1.0",
      instances: 1,
      activeCharacter: { characterId: "shu-maid", displayName: SHU.displayName },
    },
    recipes: { loaded: 3 },
    activeSensors: [],
    pendingAiAssists: 0,
    ...overrides,
  };
}

const INBOX = {
  pendingCount: 1,
  count: 1,
  totalBeforeLimit: 1,
  items: [
    {
      kind: "agent-session",
      itemId: "s-1",
      status: "waiting-for-consent",
      title: "等你核可寫入",
      route: "ai",
      needsDecision: true,
      occurredAt: "2026-01-01T00:00:00Z",
    },
  ],
};

function stubHome(opts: { status?: Record<string, unknown>; sessions?: AgentSessionRecord[] } = {}) {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => {
      throw new Error("offline");
    })
  );
  vi.spyOn(api, "status").mockResolvedValue(opts.status ?? status());
  vi.spyOn(api, "characterManifest").mockResolvedValue(SHU as never);
  vi.spyOn(api, "agentSessionsList").mockResolvedValue(
    opts.sessions ?? [SESSION, { ...SESSION, sessionId: "sess-2", label: "已結束", state: "closed", closedAt: "2026-01-01T00:10:00Z" }]
  );
  vi.spyOn(api, "activityInbox").mockResolvedValue(INBOX);
  // 詳細狀態展開後才會用到。
  vi.spyOn(api, "actionsList").mockResolvedValue([]);
  vi.spyOn(api, "sessionGet").mockResolvedValue(null);
  vi.spyOn(api, "providersList").mockResolvedValue([{ identity: { id: "dev-1" } }, { identity: { id: "dev-2" } }]);
  vi.spyOn(api, "knowledgeReceipts").mockResolvedValue({ receipts: [] });
  vi.spyOn(api, "recipesList").mockResolvedValue([]);
}

function renderHome(
  onNavigate = vi.fn(),
  extra: { estopped?: boolean; onEstop?: () => Promise<void> } = {}
) {
  const utils = render(
    <AppStateProvider ready={false} refreshKey={0}>
      <HomePage
        refreshKey={0}
        events={[]}
        onNavigate={onNavigate}
        estopped={extra.estopped}
        onEstop={extra.onEstop}
      />
    </AppStateProvider>
  );
  return { ...utils, onNavigate };
}

/** 一般使用者看得到的文字：排除折疊中的 details 內容（summary 保留）。 */
function visibleText(root: HTMLElement): string {
  const clone = root.cloneNode(true) as HTMLElement;
  clone.querySelectorAll("details:not([open])").forEach((d) => {
    Array.from(d.children).forEach((c) => {
      if (c.tagName !== "SUMMARY") c.remove();
    });
  });
  return clone.textContent ?? "";
}

function openDetails(summaryText: string) {
  const summary = screen.getByText(summaryText, { selector: "summary" });
  const details = summary.closest("details") as HTMLDetailsElement;
  // jsdom 不一定實作 summary 的 activation；直接切 open 並送 toggle（與瀏覽器行為相同）。
  details.open = true;
  fireEvent(details, new Event("toggle"));
}

beforeEach(() => {
  resetCharacterNameForTests();
  try {
    sessionStorage.clear();
  } catch {
    /* jsdom 一定有 sessionStorage */
  }
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

// ---------------------------------------------------------------------------

describe("characterSentence：角色現在怎麼樣（一句話，安全文字固定）", () => {
  const base = { name: "小樞", estop: false, paused: false, connected: true, visible: true, sensors: [] as string[] };

  it("正常在桌面上", () => {
    expect(characterSentence(base)).toBe("小樞在桌面上，一切正常。");
  });
  it("主動互動暫停", () => {
    expect(characterSentence({ ...base, paused: true })).toBe("小樞在桌面上，主動互動已暫停。");
  });
  it("感測使用中是固定文字，且優先於暫停文案", () => {
    const s = characterSentence({ ...base, paused: true, sensors: ["microphone"] });
    expect(s).toContain("感測使用中（麥克風）");
    expect(s).not.toContain("暫停");
  });
  it("角色離線：可信的固定文案（不由角色包決定），感測仍照常標示", () => {
    expect(characterSentence({ ...base, connected: false })).toBe(CHARACTER_OFFLINE_LINE);
    expect(characterSentence({ ...base, connected: false, visible: false, sensors: ["camera"] })).toBe(
      `${CHARACTER_OFFLINE_LINE}感測使用中（攝影機）。`
    );
  });
  it("已連線但隱藏", () => {
    expect(characterSentence({ ...base, visible: false })).toBe("小樞已連線，但目前隱藏中。");
  });
  it("緊急停止中永遠是固定文字開頭，蓋過其他狀態", () => {
    const s = characterSentence({ ...base, estop: true, paused: true, sensors: ["microphone"] });
    expect(s.startsWith("緊急停止中")).toBe(true);
    expect(s).toContain("小樞");
  });
  it("名字空白時退回「角色」；感測種類的人話不猜未知種類", () => {
    expect(characterSentence({ ...base, name: "" })).toBe("角色在桌面上，一切正常。");
    expect(sensorLabel("microphone")).toBe("麥克風");
    expect(sensorLabel("iphone.camera")).toBe("攝影機");
    // 認不得的種類不外洩原始 id，也不假裝知道是什麼感測器。
    expect(sensorLabel("weird.sensor")).toBe("其他感測器");
    expect(sensorLabel("iphone.motion")).toBe("其他感測器");
  });
});

// ---------------------------------------------------------------------------

describe("「現在」第一屏只回答三件事", () => {
  it("角色怎麼樣／正在做什麼／有什麼需要處理，狀態標籤走投影", async () => {
    stubHome();
    renderHome();
    // 1. 角色現在怎麼樣：名字來自 manifest，一句話。
    expect(await screen.findByText("小樞在桌面上，一切正常。")).toBeInTheDocument();
    const character = screen.getByTestId("now-character");
    expect(within(character).getByRole("button", { name: "前往小樞" })).toBeInTheDocument();
    // 2. 正在做什麼：只算 open 的工作，用投影的人話標籤，不印原始 state。
    const work = screen.getByTestId("now-work");
    expect(await within(work).findByText("1 個工作階段")).toBeInTheDocument();
    expect(within(work).getByText("整理測試報告")).toBeInTheDocument();
    expect(within(work).getByText("處理中")).toBeInTheDocument();
    expect(within(work).queryByText("已結束")).not.toBeInTheDocument();
    expect(work.textContent).not.toContain("active");
    // 3. 有什麼需要處理：後端 pendingCount＋待決定項目與投影標籤。
    const decisions = screen.getByTestId("now-decisions");
    expect(await within(decisions).findByText("1 項")).toBeInTheDocument();
    expect(within(decisions).getByText("等你核可寫入")).toBeInTheDocument();
    expect(within(decisions).getByText("等你同意")).toBeInTheDocument();
    expect(decisions.textContent).not.toContain("waiting-for-consent");
  });

  it("待我決定：精確且為 0 時明確說「目前沒有需要處理的事」，不是只留一個空白的 0 項", async () => {
    stubHome({ status: status(), sessions: [] });
    vi.spyOn(api, "activityInbox").mockResolvedValue({
      pendingCount: 0,
      count: 0,
      totalBeforeLimit: 0,
      items: [],
    });
    renderHome();
    await screen.findByText("小樞在桌面上，一切正常。");
    const decisions = screen.getByTestId("now-decisions");
    expect(await within(decisions).findByText("0 項")).toBeInTheDocument();
    expect(within(decisions).getByText("目前沒有需要處理的事。")).toBeInTheDocument();
  });

  it("待我決定：後端說 pendingCount 只是下限時，維持「至少 N 項」，不得說沒事", async () => {
    stubHome({ status: status(), sessions: [] });
    vi.spyOn(api, "activityInbox").mockResolvedValue({
      pendingCount: 0,
      count: 0,
      totalBeforeLimit: 0,
      items: [],
      pendingCountExact: false,
    });
    renderHome();
    await screen.findByText("小樞在桌面上，一切正常。");
    const decisions = screen.getByTestId("now-decisions");
    expect(await within(decisions).findByText("至少 0 項")).toBeInTheDocument();
    expect(within(decisions).queryByText("目前沒有需要處理的事。")).not.toBeInTheDocument();
  });

  it("五個快速操作都在第一屏（含停止所有感測與二段確認的緊急停止）", async () => {
    stubHome();
    const { container } = renderHome();
    await screen.findByText("小樞在桌面上，一切正常。");
    expect(screen.getByRole("button", { name: "交代一件事" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "加入裝置" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "停止所有感測" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "緊急停止" })).toBeInTheDocument();
    // 暫停／恢復是同一組（一個動作、兩個狀態），有可讀的群組名稱。
    const group = screen.getByRole("group", { name: "暫停或恢復主動互動" });
    expect(within(group).getByRole("button", { name: "暫停主動互動" })).toBeInTheDocument();
    expect(within(group).getByRole("button", { name: "暫停一段時間…" })).toBeInTheDocument();
    // 全部都在折疊區之外（第一屏就看得到）。
    expect(visibleText(container)).toContain("停止所有感測");
    expect(visibleText(container)).toContain("緊急停止");
  });

  it("數量與系統狀態收在「詳細狀態」，展開前不在畫面上，展開後才查詢並顯示", async () => {
    stubHome();
    const providers = vi.spyOn(api, "providersList");
    const { container } = renderHome();
    await screen.findByText("小樞在桌面上，一切正常。");
    const before = visibleText(container);
    expect(before).not.toContain("系統狀態");
    expect(before).not.toContain("已載入 3 個自動互動");
    expect(before).not.toContain("角色視窗");
    expect(before).not.toContain("裝置與整合來源");
    expect(screen.queryByText("系統狀態")).not.toBeInTheDocument();
    expect(providers).not.toHaveBeenCalled();

    openDetails("詳細狀態");
    expect(await screen.findByText("系統狀態")).toBeInTheDocument();
    expect(screen.getByText("已載入 3 個自動互動")).toBeInTheDocument();
    expect(screen.getByText("角色視窗：1 個連線中")).toBeInTheDocument();
    expect(await screen.findByText("裝置與整合來源：2 個")).toBeInTheDocument();
    expect(screen.getByText("系統運作正常。")).toBeInTheDocument();
    expect(providers).toHaveBeenCalledTimes(1);
  });

  it("角色離線：第一屏用可信的固定文字，不假裝角色在", async () => {
    stubHome({ status: status({ presentation: { connected: false, visible: false } }) });
    renderHome();
    expect(await screen.findByText(CHARACTER_OFFLINE_LINE)).toBeInTheDocument();
  });

  it("緊急停止中：固定安全文字出現在角色那一句", async () => {
    stubHome({ status: status({ emergencyStop: true }) });
    renderHome();
    expect(await screen.findByText(/^緊急停止中：小樞已停止所有回應。$/)).toBeInTheDocument();
  });

  it("感測使用中：固定安全文字，不被角色文案取代", async () => {
    stubHome({ status: status({ activeSensors: [{ kind: "microphone" }] }) });
    renderHome();
    expect(await screen.findByText("小樞在桌面上，感測使用中（麥克風）。")).toBeInTheDocument();
  });

  it("Runtime 回報連線的是另一個角色時誠實說明", async () => {
    stubHome({
      status: status({
        characterProtocol: {
          version: "1.0",
          instances: 1,
          activeCharacter: { characterId: "plain-text", displayName: { "zh-TW": "文字角色" } },
        },
      }),
    });
    // 名稱 hook 讀到的 manifest 是小樞（prefs／manifest），但 Runtime 說連線的是文字角色。
    renderHome();
    await screen.findByText(/小樞/, { selector: ".now-title" });
    expect(await screen.findByText("目前連線的是另一個角色：文字角色。")).toBeInTheDocument();
  });
});

// ---------------------------------------------------------------------------

describe("快速操作", () => {
  it("交代一件事：描述先放進 sessionStorage（work.prefill），再前往工作頁", async () => {
    stubHome();
    const { onNavigate } = renderHome();
    await screen.findByText("小樞在桌面上，一切正常。");
    await userEvent.type(screen.getByLabelText(/想讓小樞幫你做什麼/), "整理下載資料夾");
    await userEvent.click(screen.getByRole("button", { name: "交代一件事" }));
    expect(sessionStorage.getItem(WORK_PREFILL_KEY)).toBe("整理下載資料夾");
    expect(onNavigate).toHaveBeenCalledWith("work");
    expect(WORK_PREFILL_KEY).toBe("work.prefill");
  });

  it("交代一件事（沒寫描述）：不留舊的預填，仍前往工作頁", async () => {
    stubHome();
    sessionStorage.setItem(WORK_PREFILL_KEY, "上次殘留");
    const { onNavigate } = renderHome();
    await screen.findByText("小樞在桌面上，一切正常。");
    await userEvent.click(screen.getByRole("button", { name: "交代一件事" }));
    expect(sessionStorage.getItem(WORK_PREFILL_KEY)).toBeNull();
    expect(onNavigate).toHaveBeenCalledWith("work");
  });

  it("加入裝置 → 連接與權限；待決定的「前往」走該項目的 route", async () => {
    stubHome();
    const { onNavigate } = renderHome();
    await screen.findByText("小樞在桌面上，一切正常。");
    await userEvent.click(screen.getByRole("button", { name: "加入裝置" }));
    expect(onNavigate).toHaveBeenCalledWith("connect");
    const decisions = screen.getByTestId("now-decisions");
    await userEvent.click(await within(decisions).findByRole("button", { name: "前往" }));
    expect(onNavigate).toHaveBeenCalledWith("ai");
  });

  it("暫停主動互動反映後端狀態；失敗要看得見", async () => {
    stubHome();
    const pauseSet = vi.spyOn(api, "pauseSet").mockRejectedValue(new Error("policy down"));
    renderHome();
    await screen.findByText("小樞在桌面上，一切正常。");
    await userEvent.click(screen.getByRole("button", { name: "暫停主動互動" }));
    await waitFor(() => expect(pauseSet).toHaveBeenCalled());
    expect(await screen.findByRole("alert")).toHaveTextContent("操作失敗");
  });
});

// ---------------------------------------------------------------------------

describe("快速操作：緊急停止（只能觸發、不能解除）", () => {
  it("二段確認後才走 Shell 的緊急停止流程；頁面上沒有任何解除路徑", async () => {
    stubHome();
    const clear = vi.spyOn(api, "emergencyStopClear");
    const onEstop = vi.fn().mockResolvedValue(undefined);
    renderHome(vi.fn(), { onEstop });
    await screen.findByText("小樞在桌面上，一切正常。");
    await userEvent.click(screen.getByRole("button", { name: "緊急停止" }));
    // 第一下只進入確認態，不得直接觸發。
    expect(onEstop).not.toHaveBeenCalled();
    const confirm = await screen.findByRole("button", { name: "立即停止一切？" });
    await userEvent.click(confirm);
    await waitFor(() => expect(onEstop).toHaveBeenCalledTimes(1));
    expect(clear).not.toHaveBeenCalled();
  });

  it("沒有 onEstop 時直接呼叫後端並導到安全頁（仍是二段確認）", async () => {
    stubHome();
    const estop = vi.spyOn(api, "emergencyStop").mockResolvedValue({});
    const { onNavigate } = renderHome();
    await screen.findByText("小樞在桌面上，一切正常。");
    await userEvent.click(screen.getByRole("button", { name: "緊急停止" }));
    await userEvent.click(await screen.findByRole("button", { name: "立即停止一切？" }));
    await waitFor(() => expect(estop).toHaveBeenCalledWith("home quick action"));
    await waitFor(() => expect(onNavigate).toHaveBeenCalledWith("safety"));
  });

  it("已在緊急停止中：顯示「前往解除」而不是第二顆觸發鈕", async () => {
    stubHome({ status: status({ emergencyStop: true }) });
    const clear = vi.spyOn(api, "emergencyStopClear");
    const { onNavigate } = renderHome(vi.fn(), { estopped: true });
    await screen.findByText(/^緊急停止中：小樞已停止所有回應。$/);
    expect(screen.queryByRole("button", { name: "緊急停止" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /緊急停止中 — 前往解除/ }));
    expect(onNavigate).toHaveBeenCalledWith("safety");
    expect(clear).not.toHaveBeenCalled();
  });
});

describe("快速操作：停止所有感測（誠實階梯）", () => {
  it("重讀後仍有感測在用：說仍在使用中，不得說已停止", async () => {
    stubHome();
    // 停止之後手機仍在感測：用可變的後端狀態，避免依賴呼叫順序。
    let active: { kind: string }[] = [];
    vi.spyOn(api, "status").mockImplementation(async () => status({ activeSensors: active }));
    const stop = vi.spyOn(api, "sensorsStop").mockImplementation(async () => {
      active = [{ kind: "iphone.mic-level" }];
      return {
        stopped: true,
        uncertain: true,
        local: { microphone: "stopped" },
        devices: [{ deviceId: "d1", name: "iPhone", outcome: "unknown", waitedMs: 3000 }],
      };
    });
    renderHome();
    await screen.findByText("小樞在桌面上，一切正常。");
    await userEvent.click(screen.getByRole("button", { name: "停止所有感測" }));
    await waitFor(() => expect(stop).toHaveBeenCalled());
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("仍在使用中");
    expect(alert).toHaveTextContent("麥克風");
    expect(alert.textContent).not.toContain("已停止感測");
    expect(alert.textContent).not.toContain("iphone.mic-level");
  });

  it("裝置沒回覆：結果不確定，不算成功", async () => {
    stubHome();
    vi.spyOn(api, "sensorsStop").mockResolvedValue({
      stopped: true,
      uncertain: true,
      local: { microphone: "stopped" },
      devices: [{ deviceId: "d1", name: "iPhone", outcome: "unreachable", waitedMs: 3000 }],
    });
    vi.spyOn(api, "status").mockResolvedValue(status());
    renderHome();
    await screen.findByText("小樞在桌面上，一切正常。");
    await userEvent.click(screen.getByRole("button", { name: "停止所有感測" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("結果不確定");
    expect(alert).toHaveTextContent("iPhone");
    expect(alert.textContent).not.toContain("已停止感測");
  });

  it("舊 daemon 的 {stopped:true}＋重讀沒有感測：才敢說已停止感測", async () => {
    stubHome();
    vi.spyOn(api, "sensorsStop").mockResolvedValue({ stopped: true });
    vi.spyOn(api, "status").mockResolvedValue(status({ activeSensors: [] }));
    renderHome();
    await screen.findByText("小樞在桌面上，一切正常。");
    await userEvent.click(screen.getByRole("button", { name: "停止所有感測" }));
    expect(await screen.findByText("已停止感測。")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("重讀狀態失敗：說無法確認，不猜成功也不猜失敗", async () => {
    stubHome();
    let down = false;
    vi.spyOn(api, "status").mockImplementation(async () => {
      if (down) throw new Error("status down");
      return status();
    });
    vi.spyOn(api, "sensorsStop").mockImplementation(async () => {
      down = true;
      return { stopped: true };
    });
    renderHome();
    await screen.findByText("小樞在桌面上，一切正常。");
    await userEvent.click(screen.getByRole("button", { name: "停止所有感測" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("無法確認感測狀態");
    expect(alert.textContent).not.toContain("已停止感測");
  });

  it("停止請求本身失敗：不得靜默", async () => {
    stubHome();
    vi.spyOn(api, "sensorsStop").mockRejectedValue(new Error("daemon offline"));
    renderHome();
    await screen.findByText("小樞在桌面上，一切正常。");
    await userEvent.click(screen.getByRole("button", { name: "停止所有感測" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("停止所有感測失敗");
  });
});

describe("第一屏與詳細狀態不外洩機器字串", () => {
  it("待決定標題、知識更新來由、動作意圖與未知感測種類都翻成人話", async () => {
    stubHome({
      status: status({ activeSensors: [{ kind: "iphone.motion" }] }),
      sessions: [],
    });
    vi.spyOn(api, "activityInbox").mockResolvedValue({
      pendingCount: 1,
      count: 1,
      totalBeforeLimit: 1,
      items: [
        {
          kind: "safety-event",
          itemId: "e-1",
          status: "emergency",
          title: "emergency.stop",
          route: "safety",
          needsDecision: true,
          occurredAt: "2026-01-01T00:00:00Z",
        },
      ],
    });
    vi.spyOn(api, "knowledgeReceipts").mockResolvedValue({
      receipts: [{ updateId: "u-1", triggeredBy: "task-experience", verification: {} }],
    });
    vi.spyOn(api, "actionsList").mockResolvedValue([
      {
        actionId: "a-1",
        planId: "p-1",
        actuatorId: "notify.desktop",
        intent: "emergency-stop",
        currentStatus: "completed",
        timestamps: [],
        policyDecisions: [],
        effectiveBoundedParameters: {},
        requestedParameters: {},
        errors: [],
      },
    ]);
    vi.spyOn(api, "planGet").mockResolvedValue({ metadata: {} });
    const { container } = renderHome();
    await screen.findByText(/小樞在桌面上/);
    openDetails("詳細狀態");
    await screen.findByText("系統狀態");
    await screen.findByText(/最近更新：工作經驗/);
    const all = container.textContent ?? "";
    for (const raw of ["emergency.stop", "task-experience", "emergency-stop", "iphone.motion"]) {
      expect(all, `不得外洩原始字串「${raw}」`).not.toContain(raw);
    }
    expect(all).toContain("其他感測器");
  });

  it("詳細狀態不再列出每個工作階段，只給一行摘要與前往工作", async () => {
    stubHome();
    const close = vi.spyOn(api, "agentSessionClose");
    const { onNavigate, container } = renderHome();
    await screen.findByText("小樞在桌面上，一切正常。");
    openDetails("詳細狀態");
    await screen.findByText("系統狀態");
    expect(await screen.findByText(/目前有 1 件交代中的工作/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "取消這個工作階段" })).not.toBeInTheDocument();
    expect(container.textContent).not.toContain("權限：");
    expect(container.textContent).not.toContain("可用到");
    await userEvent.click(screen.getByRole("button", { name: "前往工作" }));
    expect(onNavigate).toHaveBeenCalledWith("work");
    expect(close).not.toHaveBeenCalled();
  });
});

// ---------------------------------------------------------------------------

describe("NowStrip 誠實計數（沿用既有契約）", () => {
  it("工作階段查詢失敗：不顯示綠色「沒有進行中」", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        throw new Error("offline");
      })
    );
    vi.spyOn(api, "status").mockRejectedValue(new Error("offline"));
    vi.spyOn(api, "agentSessionsList").mockRejectedValue(new Error("sessions down"));
    vi.spyOn(api, "activityInbox").mockResolvedValue({ items: [], count: 0, totalBeforeLimit: 0, pendingCount: 0 });
    render(<NowStrip refreshKey={0} status={{}} onNavigate={() => {}} />);
    expect(await screen.findByText("無法確認進行中的工作")).toBeInTheDocument();
    expect(screen.queryByText("沒有進行中")).not.toBeInTheDocument();
    // 角色載入失敗：名字是中立的「角色」，一句話是可信 fallback。
    expect(await screen.findByText("角色", { selector: ".now-title" })).toBeInTheDocument();
    expect(screen.getByText(CHARACTER_OFFLINE_LINE)).toBeInTheDocument();
  });

  it("系統查詢失敗：角色那一句誠實說無法確認，不假裝離線或在線", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        throw new Error("offline");
      })
    );
    vi.spyOn(api, "status").mockRejectedValue(new Error("offline"));
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([]);
    vi.spyOn(api, "activityInbox").mockResolvedValue({ items: [], count: 0, totalBeforeLimit: 0, pendingCount: 0 });
    render(<NowStrip refreshKey={0} statusError="boom" onNavigate={() => {}} />);
    expect(await screen.findByText("無法確認角色狀態（系統查詢失敗）。")).toBeInTheDocument();
  });
});
