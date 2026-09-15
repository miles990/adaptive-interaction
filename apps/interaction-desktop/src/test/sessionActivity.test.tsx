// 「這件工作的經過」（工作卡片展開區的區塊）。
//
// 這個區塊是誠實階梯在畫面上的最後一段：失敗要說得出原因、下一步要說得出
// 該做什麼，而技術層（原始紀錄、識別碼、stderr）一律只在進階模式。
//
// 一條紅線：**人話全部由後端決定**。前端不得拿 `kind`／`code` 自己造標籤，
// 所以這裡的斷言只比對後端給的 label，並且釘住一般模式看不到任何技術術語。

import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import fs from "node:fs";
import path from "node:path";
import { AgentSessionActivity, AgentSessionRecord, api } from "../api";
import { AiPage } from "../pages/AiPage";
import { AppStateProvider } from "../appstate";

const SESSION: AgentSessionRecord = {
  sessionId: "sess-9d3f1c00-1111-2222-3333-444455556666",
  providerId: "p-1",
  agentId: "claude-code",
  label: "整理測試報告",
  state: "failed",
  phase: "failed",
  lease: { issuedAt: "2026-01-01T00:00:00Z", expiresAt: "2026-01-01T01:00:00Z", renewable: true },
  dataScope: ["workspace:/tmp/repo"],
  toolScope: [],
  consentScope: [],
  budget: { maxMessages: 10, spentMessages: 1, maxCost: 0, spentCost: 0 },
  createdAt: "2026-01-01T00:00:00Z",
};

/** 一份「失敗 ＋ 有 stderr 診斷」的經過（後端已經投影成人話）。 */
function activity(overrides: Partial<AgentSessionActivity> = {}): AgentSessionActivity {
  return {
    sessionId: SESSION.sessionId,
    headline: "失敗：連接器沒有回應",
    stateLabel: "失敗",
    phase: "failed",
    recordState: "failed",
    lifecycle: "closed",
    failureReason: "連接器沒有回應",
    nextStep: "可重新交代一件工作；需要細節可展開技術詳情",
    timeline: [
      {
        at: "2026-01-01T00:05:00Z",
        id: 30,
        kind: "agent-session.outcome",
        label: "失敗：連接器沒有回應",
        outcome: "failed",
        code: "outcome.connector-error",
        detailAvailable: true,
      },
      {
        at: "2026-01-01T00:04:00Z",
        id: 20,
        kind: "agent-session.subprocess-stderr",
        label: "工作助手有診斷輸出（可展開）",
        detailAvailable: true,
      },
      {
        at: "2026-01-01T00:01:00Z",
        id: 10,
        kind: "agent-session.dispatched",
        label: "已交給 Claude Code",
        detailAvailable: true,
      },
    ],
    records: [
      {
        id: 20,
        at: "2026-01-01T00:04:00Z",
        class: "diagnostic",
        kind: "agent-session.subprocess-stderr",
        actor: "runtime",
        sessionId: SESSION.sessionId,
        detail: { tail: "warn: connector closed the pipe", truncated: false, linesDropped: 0 },
      },
    ],
    truncated: false,
    ...overrides,
  };
}

function renderPage(advanced = false) {
  return render(
    <AppStateProvider ready={false} refreshKey={0}>
      <AiPage refreshKey={0} advanced={advanced} onNavigate={() => {}} />
    </AppStateProvider>
  );
}

/** 展開工作卡片（既有的「查看結果／訊息」按鈕）。 */
async function expandCard() {
  await screen.findByText("整理測試報告");
  await userEvent.click(screen.getByRole("button", { name: "查看結果／訊息" }));
}

beforeEach(() => {
  vi.spyOn(api, "agentsDiscoveries").mockResolvedValue({ agents: [] });
  vi.spyOn(api, "agentSessionsList").mockResolvedValue([SESSION]);
  vi.spyOn(api, "agentSessionMessages").mockResolvedValue([]);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("這件工作的經過（一般模式）", () => {
  it("說得出發生了什麼、失敗原因與下一步", async () => {
    vi.spyOn(api, "agentSessionActivity").mockResolvedValue(activity());
    renderPage();
    await expandCard();

    const section = await screen.findByRole("region", { name: "這件工作的經過" });
    // headline（發生了什麼）與時間線第一步剛好是同一句話，所以指名那一段。
    expect(section.querySelector(".activity-headline")).toHaveTextContent(
      "失敗：連接器沒有回應"
    );
    expect(within(section).getByText("目前狀態：失敗")).toBeInTheDocument();
    expect(within(section).getByText("失敗原因：連接器沒有回應")).toBeInTheDocument();
    expect(
      within(section).getByText("下一步：可重新交代一件工作；需要細節可展開技術詳情")
    ).toBeInTheDocument();

    // 時間線是 <ol>，每一步都是後端給的人話 label。
    const timeline = within(section).getByRole("list");
    expect(timeline.tagName).toBe("OL");
    const steps = within(timeline).getAllByRole("listitem");
    expect(steps).toHaveLength(3);
    expect(steps[0]).toHaveTextContent("失敗：連接器沒有回應");
    expect(steps[1]).toHaveTextContent("工作助手有診斷輸出（可展開）");
    expect(steps[2]).toHaveTextContent("已交給 Claude Code");
  });

  it("不外洩任何技術術語、識別碼或 stderr 內容", async () => {
    vi.spyOn(api, "agentSessionActivity").mockResolvedValue(activity());
    const { container } = renderPage();
    await expandCard();
    await screen.findByRole("region", { name: "這件工作的經過" });

    const text = container.textContent ?? "";
    for (const banned of [/uuid/i, /trace/i, /stderr/i, /sse/i, /provider/i, /lease/i]) {
      expect(text, `一般模式不得出現 ${banned}`).not.toMatch(banned);
    }
    // 原始 id 與 kind／code 字串都不得出現。
    expect(text).not.toMatch(/[0-9a-f]{8}-[0-9a-f]{4}-/i);
    expect(text).not.toContain("agent-session.");
    expect(text).not.toContain("outcome.connector-error");
    expect(text).not.toContain("connector closed the pipe");
    // 技術詳情是進階模式的東西。
    expect(screen.queryByText("技術詳情")).not.toBeInTheDocument();
  });

  it("讀不到就說讀不到，不拿空清單假裝什麼都沒發生", async () => {
    vi.spyOn(api, "agentSessionActivity").mockRejectedValue(new Error("boom"));
    renderPage();
    await expandCard();
    const section = await screen.findByRole("region", { name: "這件工作的經過" });
    expect(await within(section).findByText("目前讀不到這件工作的經過。")).toBeInTheDocument();
    // 失敗不得被投影成「沒有紀錄」。
    expect(within(section).queryByText("這件工作還沒有留下任何紀錄。")).toBeNull();
  });
});

describe("這件工作的經過（進階模式）", () => {
  it("技術詳情裡才有原始紀錄與脫敏後的 stderr", async () => {
    vi.spyOn(api, "agentSessionActivity").mockResolvedValue(activity());
    renderPage(true);
    await expandCard();
    const section = await screen.findByRole("region", { name: "這件工作的經過" });

    expect(within(section).getByText("技術詳情")).toBeInTheDocument();
    const json = section.querySelector("pre")?.textContent ?? "";
    expect(json).toContain("connector closed the pipe");
    expect(json).toContain("agent-session.subprocess-stderr");
    // 一般模式看得到的人話在進階模式一樣在（進階是「多給」，不是「換掉」）。
    expect(within(section).getByText("失敗原因：連接器沒有回應")).toBeInTheDocument();
  });
});

describe("載入更早", () => {
  it("是真的 <button>，鍵盤到得了、Enter 會帶 before 呼叫 API", async () => {
    const first = activity({ truncated: true, nextCursor: 10 });
    const older = activity({
      headline: "已交給 Claude Code",
      timeline: [
        {
          at: "2025-12-31T23:59:00Z",
          id: 5,
          kind: "agent-session.task-delivered",
          label: "任務已送達",
          detailAvailable: true,
        },
      ],
      records: [],
      truncated: false,
      nextCursor: null,
    });
    const spy = vi
      .spyOn(api, "agentSessionActivity")
      .mockResolvedValueOnce(first)
      .mockResolvedValueOnce(older);

    renderPage();
    await expandCard();
    const section = await screen.findByRole("region", { name: "這件工作的經過" });
    const button = within(section).getByRole("button", { name: "載入更早" });
    expect(button.tagName).toBe("BUTTON");

    // 鍵盤：Tab 到得了，Enter 觸發（原生 button，不是 div + onClick）。
    button.focus();
    expect(button).toHaveFocus();
    await userEvent.keyboard("{Enter}");

    await waitFor(() => {
      expect(spy).toHaveBeenCalledWith(SESSION.sessionId, 10, 20);
    });
    // 更早的紀錄接在時間線後面（由新到舊），而且按鈕在到底之後消失。
    await waitFor(() => {
      expect(within(section).getByText("任務已送達")).toBeInTheDocument();
    });
    expect(within(section).queryByRole("button", { name: "載入更早" })).toBeNull();
  });

  it("沒有更早的紀錄時不畫按鈕（不給按不動的東西）", async () => {
    vi.spyOn(api, "agentSessionActivity").mockResolvedValue(activity());
    renderPage();
    await expandCard();
    const section = await screen.findByRole("region", { name: "這件工作的經過" });
    expect(within(section).queryByRole("button", { name: "載入更早" })).toBeNull();
  });
});

describe("窄螢幕（390px）", () => {
  it("不以 inline style 硬編超過視窗的寬度", async () => {
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 390 });
    window.dispatchEvent(new Event("resize"));
    vi.spyOn(api, "agentSessionActivity").mockResolvedValue(activity());
    const { container } = renderPage();
    await expandCard();
    await screen.findByRole("region", { name: "這件工作的經過" });

    const wide = Array.from(container.querySelectorAll<HTMLElement>("[style]")).filter((el) => {
      const w = parseFloat(el.style.width || "0");
      return Number.isFinite(w) && el.style.width.endsWith("px") && w > 390;
    });
    expect(wide).toEqual([]);
  });

  it("時間線可換行，區塊沒有固定寬度", () => {
    const css = fs.readFileSync(path.resolve("src/styles.css"), "utf8");
    const block = css.slice(
      css.indexOf("/* 「這件工作的經過」"),
      css.indexOf(".consent-sheet")
    );
    expect(block).toContain(".work-activity");
    expect(block).toContain(".activity-timeline");
    expect(block).toMatch(/flex-wrap:\s*wrap/);
    // 固定寬度會讓 390px 橫向捲動；只允許 min-width: 0（收窄用）。
    expect(block).not.toMatch(/(^|[^-])width:\s*\d+px/m);
  });
});
