// 「未解決停止」：投影＋「連接與權限」逐筆與人為確認流程。
//
// 這一區回答的是「有哪些擷取，我們不知道它停了沒有」。守門的是語意，不是逐字：
//   - 不得說「已停止」（那是它**沒有**回答的問題）；
//   - 逐筆一定看得到是哪一種感測、多久以前的事；
//   - `sourceId`／`generation` 只能拿去呼叫 API，不得進畫面文字；
//   - 「我確認它已經停了」是二段確認，第二段一定要說出「系統沒有收到裝置的回覆」；
//   - 解除失敗不得靜默，也不得說成已經處理掉。

import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { api, type UnresolvedStop } from "../api";
import { configureHttp } from "../transport";
import {
  MAX_UNRESOLVED_LINES,
  projectUnresolvedStops,
  relativeSince,
  UNRESOLVED_DISMISS_CONFIRM,
  UNRESOLVED_DISMISS_LABEL,
} from "../statusProjection";
import { UnresolvedStopsSection } from "../pages/connect/UnresolvedStops";
import { UnresolvedStopsBanner } from "../components/UnresolvedStopsBanner";

afterEach(() => {
  vi.restoreAllMocks();
});

const NOW = Date.parse("2026-09-06T12:00:00.000Z");

function stop(overrides: Partial<UnresolvedStop> = {}): UnresolvedStop {
  return {
    sourceId: "declarative.desk-esp32",
    generation: 7,
    sensors: ["microphone"],
    since: "2026-09-06T11:57:00.000Z",
    lastKnown: [{ kind: "microphone", startedAt: "x", startedBy: "api", purpose: "p" }],
    ...overrides,
  };
}

describe("projectUnresolvedStops：不知道就說不知道", () => {
  it("欄位缺席／不是物件／空陣列都是「沒有未解決的事」", () => {
    for (const input of [null, undefined, 42, "x", {}, { unresolvedStops: [] }, { unresolvedStops: 3 }]) {
      const view = projectUnresolvedStops(input, NOW);
      expect(view.count).toBe(0);
      expect(view.summary).toBeNull();
      expect(view.items).toEqual([]);
    }
  });

  it("逐筆：人話名稱、感測種類、相對時間；摘要說出筆數", () => {
    const view = projectUnresolvedStops(
      { unresolvedStops: [stop({ sourceLabel: "書桌 ESP32" }), stop({ sensors: ["camera"] })] },
      NOW
    );
    expect(view.count).toBe(2);
    expect(view.summary).toBe("有 2 筆感測停止沒有人確認");
    expect(view.items[0].label).toBe("書桌 ESP32");
    expect(view.items[0].sensorsText).toBe("麥克風");
    expect(view.items[0].sinceText).toBe("3 分鐘前");
    expect(view.items[0].line).toContain("書桌 ESP32");
    expect(view.items[0].line).toContain("麥克風");
    // 名字查不到時用中性稱呼，不退回 sourceId。
    expect(view.items[1].label).toBe("某個裝置");
    expect(view.items[1].sensorsText).toBe("攝影機");
  });

  it("不外洩 sourceId／generation，也不外洩認不得的感測原始 id", () => {
    const view = projectUnresolvedStops(
      { unresolvedStops: [stop({ sensors: ["iphone.motion"], generation: 12 })] },
      NOW
    );
    const text = [view.summary, view.note, ...view.items.map((i) => i.line)].join(" ");
    expect(text).not.toContain("declarative.desk-esp32");
    expect(text).not.toContain("iphone.motion");
    expect(text).not.toMatch(/generation|sourceId/i);
    expect(view.items[0].sensorsText).toBe("其他感測器");
    // 呼叫 API 需要的識別仍然拿得到（只是不上畫面）。
    expect(view.items[0].sourceId).toBe("declarative.desk-esp32");
    expect(view.items[0].generation).toBe(12);
  });

  it("文案不得說「已停止」——它沒有回答那個問題", () => {
    const view = projectUnresolvedStops({ unresolvedStops: [stop(), stop()] }, NOW);
    const text = [view.summary, view.note, ...view.items.map((i) => i.line)].join(" ");
    expect(text).not.toContain("已停止");
  });

  it("逐筆有界：超過上限只列前幾筆，其餘誠實說「還有 N 筆」", () => {
    const many = Array.from({ length: MAX_UNRESOLVED_LINES + 5 }, (_, i) =>
      stop({ generation: i })
    );
    const view = projectUnresolvedStops({ unresolvedStops: many }, NOW);
    expect(view.count).toBe(MAX_UNRESOLVED_LINES + 5);
    expect(view.items).toHaveLength(MAX_UNRESOLVED_LINES);
    expect(view.notShown).toBe(5);
  });

  it("相對時間：讀不出來說「時間不明」，未來時間不會變成負數", () => {
    expect(relativeSince("not a date", NOW)).toBe("時間不明");
    expect(relativeSince(undefined, NOW)).toBe("時間不明");
    expect(relativeSince("2026-09-06T11:59:30.000Z", NOW)).toBe("剛剛");
    expect(relativeSince("2026-09-06T12:05:00.000Z", NOW)).toBe("剛剛");
    expect(relativeSince("2026-09-06T09:00:00.000Z", NOW)).toBe("3 小時前");
    expect(relativeSince("2026-09-01T12:00:00.000Z", NOW)).toBe("5 天前");
  });
});

describe("連接與權限：沒有人確認的感測停止", () => {
  it("沒有紀錄時只說「目前沒有這一類紀錄」，不假裝已經全部停了", async () => {
    vi.spyOn(api, "sensorsUnresolved").mockResolvedValue({ unresolvedStops: [] });
    render(<UnresolvedStopsSection refreshKey={0} />);
    expect(await screen.findByText("目前沒有這一類紀錄。")).toBeInTheDocument();
    const section = screen.getByTestId("unresolved-stops");
    expect(section.textContent ?? "").not.toContain("已停止");
  });

  it("逐筆列出，確認按鈕是二段的，第二段說出「系統沒有收到裝置的回覆」", async () => {
    vi.spyOn(api, "sensorsUnresolved").mockResolvedValue({
      unresolvedStops: [stop({ sourceLabel: "書桌 ESP32" })],
    });
    const dismiss = vi
      .spyOn(api, "sensorsDismissUnresolved")
      .mockResolvedValue({ dismissed: true, confirmedStopped: false });
    render(<UnresolvedStopsSection refreshKey={0} />);
    const row = await screen.findByTestId("unresolved-stop-0");
    expect(within(row).getByText(/書桌 ESP32/)).toBeInTheDocument();

    // 第一段：還沒送出任何東西。
    await userEvent.click(within(row).getByRole("button", { name: UNRESOLVED_DISMISS_LABEL }));
    expect(dismiss).not.toHaveBeenCalled();

    // 第二段：文案必須說清楚這是誰的確認。
    const confirm = await screen.findByRole("button", { name: UNRESOLVED_DISMISS_CONFIRM });
    expect(confirm.textContent ?? "").toContain("系統沒有收到裝置的回覆");
    await userEvent.click(confirm);
    await waitFor(() => expect(dismiss).toHaveBeenCalledWith("declarative.desk-esp32", 7));
    expect(await screen.findByText(/系統沒有收到裝置的回覆/)).toBeInTheDocument();
  });

  it("解除失敗不得靜默，也不得說成已經處理掉", async () => {
    vi.spyOn(api, "sensorsUnresolved").mockResolvedValue({ unresolvedStops: [stop()] });
    vi.spyOn(api, "sensorsDismissUnresolved").mockRejectedValue(new Error("404 not found"));
    render(<UnresolvedStopsSection refreshKey={0} />);
    const row = await screen.findByTestId("unresolved-stop-0");
    await userEvent.click(within(row).getByRole("button", { name: UNRESOLVED_DISMISS_LABEL }));
    await userEvent.click(await screen.findByRole("button", { name: UNRESOLVED_DISMISS_CONFIRM }));
    expect(await screen.findByText(/沒有記下你的確認/)).toBeInTheDocument();
    expect(screen.getByText(/這一筆還在/)).toBeInTheDocument();
  });

  it("讀不到就說讀不到（不把「讀取失敗」說成「沒有」）", async () => {
    vi.spyOn(api, "sensorsUnresolved").mockRejectedValue(new Error("500"));
    render(<UnresolvedStopsSection refreshKey={0} />);
    expect(await screen.findByText(/讀不到「沒有人確認的感測停止」/)).toBeInTheDocument();
    expect(screen.queryByText("目前沒有這一類紀錄。")).not.toBeInTheDocument();
  });
});

describe("狀態列摘要那一行", () => {
  it("有未確認的停止時說出筆數，並給得出去「連接與權限」的路", async () => {
    const onOpen = vi.fn();
    const summary = projectUnresolvedStops({ unresolvedStops: [stop(), stop()] }, NOW).summary;
    const { container } = render(<UnresolvedStopsBanner summary={summary} onOpen={onOpen} />);
    expect(container.textContent).toContain("有 2 筆感測停止沒有人確認");
    expect(container.textContent).toContain("連接與權限");
    // 誠實：它不說「已停止」，也不說「還在感測」。
    expect(container.textContent).not.toContain("已停止");
    expect(container.textContent).not.toContain("感測使用中");
    await userEvent.click(screen.getByRole("button", { name: "前往查看" }));
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it("沒有這一類紀錄時不留空殼", () => {
    const { container } = render(<UnresolvedStopsBanner summary={null} onOpen={() => {}} />);
    expect(container.querySelector(".sensor-banner")).toBeNull();
  });
});

// 瀏覽器（外部 daemon）模式：同一組型別走 HTTP，路徑與 body 必須對得上後端。
describe("HTTP 模式的兩條路由", () => {
  it("讀取走 GET /v1/sensors/unresolved；解除走 POST …/{sourceId}/dismiss 並帶世代", async () => {
    configureHttp("http://127.0.0.1:8787", "test-token");
    const calls: { method: string; url: string; body?: string }[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string, init: RequestInit) => {
        calls.push({
          method: String(init?.method ?? "GET"),
          url: String(url),
          ...(typeof init?.body === "string" ? { body: init.body } : {}),
        });
        return new Response(JSON.stringify({ unresolvedStops: [] }), { status: 200 });
      })
    );
    await api.sensorsUnresolved();
    await api.sensorsDismissUnresolved("declarative.desk esp32", 9);
    vi.unstubAllGlobals();

    expect(calls[0]).toMatchObject({
      method: "GET",
      url: "http://127.0.0.1:8787/v1/sensors/unresolved",
    });
    expect(calls[1].method).toBe("POST");
    // sourceId 一定要 encode（來源 id 不保證是路徑安全字元）。
    expect(calls[1].url).toBe(
      "http://127.0.0.1:8787/v1/sensors/unresolved/declarative.desk%20esp32/dismiss"
    );
    expect(JSON.parse(calls[1].body ?? "{}")).toEqual({ generation: 9 });
  });
});
