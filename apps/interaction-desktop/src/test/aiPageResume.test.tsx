// v0.5 對抗審查（review2）確認缺陷的 regression tests（工作卡片）：
// - agent-honesty-022：「接續上次（唯讀）」不得放寬範圍——資料夾、修改權限、
//   時間上限與費用上限都要沿用上次的實際值，不得省略後落到後端預設
//   （120 分鐘、沒有金額上限），也不得因為漏帶資料夾而讓後端自己決定。
// - agent-honesty-025／known limitation #24：「已送達」不是固定文案——送出的
//   結果一律走共用的 work/delivery 六態投影，後端沒蓋送達戳記時只能說
//   「尚未送達（已放進信箱）」，送不到就說 Agent 不可用／傳送失敗／結果不確定。

import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AgentSessionRecord, api } from "../api";
import { AppStateProvider } from "../appstate";
import { AiPage, buildResumeInput, resumeLimits, resumeLimitsText } from "../pages/AiPage";
import { deliveredToAgent } from "../work/delivery";
import { resetCharacterNameForTests } from "../characterName";

afterEach(() => {
  vi.restoreAllMocks();
  resetCharacterNameForTests();
});

/** 一個「使用者選了 30 分鐘、US$0.5、一個資料夾」的已結束工作。 */
const FINISHED: AgentSessionRecord = {
  sessionId: "sess-resume-1",
  providerId: "provider.ai-agent.claude-code",
  agentId: "claude-code",
  label: "整理測試報告",
  state: "unknown",
  lease: { issuedAt: "2026-01-01T00:00:00Z", expiresAt: "2026-01-01T00:30:00Z", renewable: true },
  dataScope: ["workspace:/Users/me/project", "domain:project-source"],
  toolScope: [],
  consentScope: [],
  budget: { maxMessages: 10, spentMessages: 3, maxCost: 0.5, spentCost: 0.1 },
  providerSessionId: "thread-abc",
  createdAt: "2026-01-01T00:00:00Z",
};

function renderAiPage(record: AgentSessionRecord) {
  vi.spyOn(api, "agentsDiscoveries").mockResolvedValue({ agents: [] });
  vi.spyOn(api, "agentSessionsList").mockResolvedValue([record]);
  vi.spyOn(api, "agentSessionMessages").mockResolvedValue([]);
  return render(
    <AppStateProvider ready={false} refreshKey={0}>
      <AiPage refreshKey={0} advanced={false} onNavigate={() => {}} />
    </AppStateProvider>
  );
}

describe("agent-honesty-022：接續上次（唯讀）不得放寬範圍", () => {
  it("送出的內容帶著上次的資料夾、時間與費用上限，且修改權限為關", async () => {
    const created = vi
      .spyOn(api, "agentSessionCreate")
      .mockResolvedValue({ ...FINISHED, sessionId: "sess-resume-2" });
    renderAiPage(FINISHED);
    await screen.findByText("整理測試報告");
    await userEvent.click(screen.getByRole("button", { name: "接續上次（唯讀）" }));

    await waitFor(() => expect(created).toHaveBeenCalledTimes(1));
    const input = created.mock.calls[0][0];
    expect(input.workdir).toBe("/Users/me/project");
    expect(input.ttlMinutes).toBe(30);
    expect(input.maxCost).toBe(0.5);
    expect(input.maxMessages).toBe(10);
    expect(input.allowWrite).toBe(false);
    expect(input.toolScope).toEqual([]);
    expect(input.consentScope).toEqual([]);
    expect(input.resumeProviderSessionId).toBe("thread-abc");
    // 資料夾也要寫回 dataScope，下一次接續才找得回同一個資料夾；
    // 其他授權（domain:）不繼承。
    expect(input.dataScope).toEqual(["workspace:/Users/me/project"]);
  });

  it("接續產生的工作再接續一次，仍然帶著同一個資料夾與上限（不會逐次變寬）", () => {
    // 第一次接續送出的內容，就是第二筆 record 的授權內容。
    const first = buildResumeInput(FINISHED);
    const second: AgentSessionRecord = {
      ...FINISHED,
      sessionId: "sess-resume-2",
      label: String(first.label),
      dataScope: first.dataScope as string[],
      budget: {
        maxMessages: first.maxMessages as number,
        spentMessages: 0,
        maxCost: first.maxCost as number,
        spentCost: 0,
      },
      lease: {
        issuedAt: "2026-01-02T00:00:00Z",
        expiresAt: "2026-01-02T00:30:00Z",
        renewable: true,
      },
    };
    const again = buildResumeInput(second);
    expect(again.workdir).toBe("/Users/me/project");
    expect(again.ttlMinutes).toBe(30);
    expect(again.maxCost).toBe(0.5);
    expect(again.allowWrite).toBe(false);
  });

  it("後端有回報原始時間上限時以它為準；沒有時退回租期長度", () => {
    const withDuration = {
      ...FINISHED,
      lease: { issuedAt: "2026-01-01T00:00:00Z", expiresAt: "2026-01-01T09:00:00Z", renewable: true },
      budget: { ...FINISHED.budget, maxDurationMs: 45 * 60_000 },
    } as unknown as AgentSessionRecord;
    expect(resumeLimits(withDuration).ttlMinutes).toBe(45);
    expect(resumeLimits(FINISHED).ttlMinutes).toBe(30);
  });

  it("資料夾以後端記錄的 resolvedWorkdir 為準，dataScope 標籤只是備援", () => {
    // 後端記錄的是「上一次真的掛上子程序的資料夾」（正規化後的絕對路徑）。
    // dataScope 裡的 `workspace:` 只是呼叫端自己附加的人話標籤，兩者不一致時
    // 必須以後端的事實為準——否則送出的續開會因為換資料夾而被後端擋下。
    const relocated = {
      ...FINISHED,
      resolvedWorkdir: "/private/Users/me/project",
    } as unknown as AgentSessionRecord;
    expect(buildResumeInput(relocated).workdir).toBe("/private/Users/me/project");
    expect(resumeLimitsText(relocated)).toContain("/private/Users/me/project");
    // 沒有記錄時才退回標籤，而且要誠實說出「沒能確認」。
    expect(buildResumeInput(FINISHED).workdir).toBe("/Users/me/project");
    expect(resumeLimitsText(FINISHED)).toContain("未確認");
  });

  it("通知說出實際沿用的資料夾與時間，不只是「沿用先前的對話脈絡」", async () => {
    vi.spyOn(api, "agentSessionCreate").mockResolvedValue({
      ...FINISHED,
      sessionId: "sess-resume-2",
    });
    renderAiPage(FINISHED);
    await screen.findByText("整理測試報告");
    await userEvent.click(screen.getByRole("button", { name: "接續上次（唯讀）" }));
    const notice = await screen.findByText(/已接續上次的工作/);
    expect(notice.textContent).toContain("/Users/me/project");
    expect(notice.textContent).toContain("30 分鐘");
    expect(notice.textContent).toContain("US$0.5");
  });
});

describe("agent-honesty-025／#24：再交代也走同一份六態送達投影", () => {
  it("deliveredToAgent：只認真實的 deliveredAt 戳記", () => {
    expect(deliveredToAgent({ messageId: "m-1", deliveredAt: "2026-01-01T00:00:01Z" })).toBe(true);
    expect(deliveredToAgent({ messageId: "m-1" })).toBe(false);
    expect(deliveredToAgent({ messageId: "m-1", deliveredAt: null })).toBe(false);
    expect(deliveredToAgent(undefined)).toBe(false);
  });

  async function sendAgain() {
    renderAiPage({ ...FINISHED, state: "active" });
    await screen.findByText("整理測試報告");
    await userEvent.click(screen.getByRole("button", { name: "查看結果／訊息" }));
    await userEvent.type(screen.getByPlaceholderText("再交代一句給這個 Agent…"), "再看一次");
    await userEvent.click(screen.getByRole("button", { name: "送出" }));
  }

  it("回傳沒有戳記的訊息 → 說「已放進…信箱」，不得說已送達", async () => {
    vi.spyOn(api, "agentSessionSend").mockResolvedValue({ messageId: "m-1" });
    await sendAgain();
    const notice = await screen.findByText(/已放進 Claude Code 的信箱/);
    expect(notice.textContent).not.toContain("已送達");
  });

  it("回傳帶戳記的訊息 → 才說已送到它手上、尚未完成", async () => {
    vi.spyOn(api, "agentSessionSend").mockResolvedValue({
      messageId: "m-1",
      deliveredAt: "2026-01-01T00:00:01Z",
    });
    await sendAgain();
    const notice = await screen.findByText(/已送到 Claude Code 手上/);
    expect(notice.textContent).toContain("尚未完成");
  });

  it("上一輪還在跑（409）→ 排隊中，不說失敗也不說送達", async () => {
    vi.spyOn(api, "agentSessionSend").mockRejectedValue(
      new Error("409: conflict: 上一輪還在跑，這則訊息未送達；稍後再送或先中斷：busy")
    );
    await sendAgain();
    const notice = await screen.findByText(/上一輪還在跑/);
    expect(notice.textContent).not.toContain("已送達");
    expect(notice.textContent).not.toContain("送出失敗");
  });

  it("子程序不在（503）→ Agent 不可用；一般模式不外洩後端術語", async () => {
    vi.spyOn(api, "agentSessionSend").mockRejectedValue(
      new Error("503: unavailable: agent 子程序已結束，這則訊息未送達；請續開（resume）")
    );
    await sendAgain();
    const notice = await screen.findByText(/Claude Code 現在不能接工作/);
    expect(notice.textContent).not.toContain("子程序");
    expect(notice.textContent).not.toContain("已送達");
  });

  it("連不上 → 結果不確定，不謊稱送到也不謊稱失敗", async () => {
    vi.spyOn(api, "agentSessionSend").mockRejectedValue(new TypeError("Failed to fetch"));
    await sendAgain();
    const notice = await screen.findByText(/不確定/);
    expect(notice.textContent).not.toContain("已送達");
  });
});
