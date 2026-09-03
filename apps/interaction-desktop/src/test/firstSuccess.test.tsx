// 首次成功體驗（精靈 commit 之後、可略過）：五個選項、提醒走本機 plan 路徑（不建 AI 工作）、
// 誠實的回執狀態投影、交代小工作預填、桌面顯示、看過旗標（host 沒保存就退回本機）、390px。

import fs from "node:fs";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const mockApi = vi.hoisted(() => ({
  uiPrefsGet: vi.fn(async () => ({ mode: "simple", locale: "zh-TW", customNames: {}, schemaVersion: "1.0" })),
  // 模擬 Rust host：未知欄位 firstSuccessSeen 被丟掉，不會回傳。
  uiPrefsPatch: vi.fn(async () => ({ mode: "simple", locale: "zh-TW", customNames: {}, schemaVersion: "1.0" })),
  sessionGet: vi.fn(async () => null),
  sessionStart: vi.fn(async () => ({ sessionId: "s1", state: "active", startedAt: "", consents: [] })),
  createPlan: vi.fn(async () => ({ planId: "plan-1" })),
  executePlan: vi.fn(async () => [
    {
      actionId: "a1",
      planId: "plan-1",
      actuatorId: "notification",
      intent: "rest-reminder",
      currentStatus: "dispatched",
      timestamps: [],
      policyDecisions: [],
      effectiveBoundedParameters: {},
      requestedParameters: {},
      errors: [],
    },
  ]),
  agentSessionCreate: vi.fn(async () => ({})),
}));

vi.mock("../api", async (importOriginal) => {
  const original = await importOriginal<Record<string, unknown>>();
  return { ...original, api: mockApi };
});

const mockDesktop = vi.hoisted(() => ({
  prefsPatch: vi.fn(async (patch: Record<string, unknown>) => ({ companionVisible: true, ...patch })),
  companionApplyPrefs: vi.fn(async () => null),
}));

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

import {
  FirstSuccess,
  FIRST_SUCCESS_STORAGE_KEY,
  isFirstSuccessSeen,
  markFirstSuccessSeen,
  sendRestReminder,
  WORK_PREFILL_KEY,
} from "../pages/FirstSuccess";

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  sessionStorage.clear();
  mockName.current = { name: "小樞", pronoun: "她", characterId: "shu-maid", loaded: true, icon: "cat" };
  mockApi.sessionGet.mockResolvedValue(null);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("首次成功體驗", () => {
  it("標題用角色名，五個選項齊全，而且不是精靈的一步（沒有設定進度列）", () => {
    render(<FirstSuccess onDone={() => {}} />);
    expect(screen.getByRole("heading", { name: "小樞準備好了。要不要先試一次？" })).toBeInTheDocument();
    expect(screen.getByRole("dialog", { name: "首次成功體驗" })).toBeInTheDocument();
    expect(screen.queryByRole("navigation", { name: "設定進度" })).not.toBeInTheDocument();
    for (const label of [/提醒我休息/, /交代一件小工作/, /先在桌面陪我/, /更換角色/]) {
      expect(screen.getByRole("button", { name: label })).toBeInTheDocument();
    }
    expect(screen.getByRole("button", { name: "稍後再說" })).toBeInTheDocument();
  });

  it("角色未載入 → 可信文字「角色準備好了」，畫面照常運作", () => {
    mockName.current = { name: "角色", pronoun: "角色", characterId: null as unknown as string, loaded: false, icon: "sparkles" };
    render(<FirstSuccess onDone={() => {}} />);
    expect(screen.getByRole("heading", { name: "角色準備好了。要不要先試一次？" })).toBeInTheDocument();
    expect(screen.getByText(/角色資料尚未載入/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /提醒我休息/ })).toBeEnabled();
  });

  it("提醒我休息：走本機 plan 路徑（通知／角色氣泡），不建立 AI 工作，狀態誠實投影", async () => {
    render(<FirstSuccess onDone={() => {}} />);
    await userEvent.click(screen.getByRole("button", { name: /提醒我休息/ }));
    await waitFor(() => expect(mockApi.executePlan).toHaveBeenCalledWith("plan-1"));
    // 沒有工作階段就先開一個純本機、零同意範圍的；不會建立 AI 工作。
    expect(mockApi.sessionStart).toHaveBeenCalledWith("first-success", []);
    const input = (mockApi.createPlan.mock.calls as unknown[][])[0][0] as Record<string, unknown>;
    expect(input.candidates).toEqual(["notification", "companion.bubble.show"]);
    expect(input.preferredChannels).toContain("notification");
    expect(String(input.message)).toContain("小樞提醒");
    expect(mockApi.agentSessionCreate).not.toHaveBeenCalled();
    const result = await screen.findByRole("status");
    // dispatched → 共用狀態投影（不是「已完成」）。
    expect(result).toHaveTextContent("已送出（等待確認）");
    expect(result).toHaveTextContent("透過系統通知");
    expect(result).toHaveTextContent("已送出不等於你已經看見");
    expect(result.textContent).not.toContain("已完成");
  });

  it("提醒失敗 → 誠實顯示錯誤，不宣稱送出", async () => {
    mockApi.createPlan.mockRejectedValueOnce(new Error("policy blocked: notification disabled"));
    render(<FirstSuccess onDone={() => {}} />);
    await userEvent.click(screen.getByRole("button", { name: /提醒我休息/ }));
    expect(await screen.findByRole("alert")).toHaveTextContent("提醒沒有送出");
    expect(screen.queryByText(/已送出（等待確認）/)).not.toBeInTheDocument();
  });

  it("sendRestReminder：沒有回執 → 結果不確定；被阻擋 → 投影標籤", async () => {
    mockApi.executePlan.mockResolvedValueOnce([]);
    const none = await sendRestReminder("小樞");
    expect(none.status.label).toBe("結果不確定");
    mockApi.executePlan.mockResolvedValueOnce([
      { actionId: "a2", planId: "plan-1", actuatorId: "companion.bubble.show", intent: "x", currentStatus: "blocked", timestamps: [], policyDecisions: [], effectiveBoundedParameters: {}, requestedParameters: {}, errors: [] },
    ]);
    const blocked = await sendRestReminder("小樞");
    expect(blocked.status.raw).toBe("blocked");
    expect(blocked.status.label).not.toBe("blocked");
    expect(blocked.via).toBe("透過桌面角色的氣泡");
  });

  it("交代一件小工作：預填 work.prefill、前往工作頁、關閉並記住看過", async () => {
    const onDone = vi.fn();
    const onNavigate = vi.fn();
    render(<FirstSuccess onDone={onDone} onNavigate={onNavigate} />);
    await userEvent.click(screen.getByRole("button", { name: /交代一件小工作/ }));
    await waitFor(() => expect(onDone).toHaveBeenCalledTimes(1));
    expect(onNavigate).toHaveBeenCalledWith("work");
    expect(sessionStorage.getItem(WORK_PREFILL_KEY)).toMatch(/清單/);
    expect(mockApi.uiPrefsPatch).toHaveBeenCalledWith({ firstSuccessSeen: true });
    // host 沒保存旗標 → 退回本機旗標。
    expect(localStorage.getItem(FIRST_SUCCESS_STORAGE_KEY)).toBe("1");
  });

  it("先在桌面陪我：prefsPatch companionVisible ＋ companionApplyPrefs，文案誠實（無法顯示會改用文字）", async () => {
    render(<FirstSuccess onDone={() => {}} />);
    await userEvent.click(screen.getByRole("button", { name: /先在桌面陪我/ }));
    await waitFor(() => expect(mockDesktop.prefsPatch).toHaveBeenCalledWith({ companionVisible: true }));
    expect(mockDesktop.companionApplyPrefs).toHaveBeenCalled();
    expect(await screen.findByRole("status")).toHaveTextContent(/已請桌面角色視窗顯示小樞；若無法顯示會改用文字/);
  });

  it("更換角色 → 前往角色頁；稍後再說 → 關閉；兩者都記住看過", async () => {
    const onDone = vi.fn();
    const onNavigate = vi.fn();
    const first = render(<FirstSuccess onDone={onDone} onNavigate={onNavigate} />);
    await userEvent.click(screen.getByRole("button", { name: /更換角色/ }));
    await waitFor(() => expect(onDone).toHaveBeenCalledTimes(1));
    expect(onNavigate).toHaveBeenCalledWith("companion");
    first.unmount();
    localStorage.clear();

    const onDone2 = vi.fn();
    render(<FirstSuccess onDone={onDone2} />);
    await userEvent.click(screen.getByRole("button", { name: "稍後再說" }));
    await waitFor(() => expect(onDone2).toHaveBeenCalledTimes(1));
    expect(await isFirstSuccessSeen()).toBe(true);
  });

  it("看過旗標：host 回傳旗標就算 host 保存；否則退回本機", async () => {
    mockApi.uiPrefsPatch.mockResolvedValueOnce({ mode: "simple", locale: "zh-TW", customNames: {}, schemaVersion: "1.0", firstSuccessSeen: true } as never);
    expect(await markFirstSuccessSeen()).toBe("host");
    expect(localStorage.getItem(FIRST_SUCCESS_STORAGE_KEY)).toBeNull();
    expect(await markFirstSuccessSeen()).toBe("local");
    expect(localStorage.getItem(FIRST_SUCCESS_STORAGE_KEY)).toBe("1");
    mockApi.uiPrefsGet.mockResolvedValueOnce({ mode: "simple", locale: "zh-TW", customNames: {}, schemaVersion: "1.0", firstSuccessSeen: true } as never);
    localStorage.clear();
    expect(await isFirstSuccessSeen()).toBe(true);
  });

  it("390px：選項是整寬的直排按鈕（class），CSS 有窄視窗規則", () => {
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 390 });
    const { container } = render(<FirstSuccess onDone={() => {}} />);
    expect(container.querySelector(".first-success-options")).not.toBeNull();
    expect(container.querySelectorAll(".first-success-option")).toHaveLength(4);
    const css = fs.readFileSync(path.resolve("src/styles.css"), "utf8");
    expect(css).toMatch(/\.first-success-option \{[\s\S]*width: 100%;/);
    expect(css).toMatch(/\.first-success \.onboarding-panel \{ padding: 16px 12px 12px; \}/);
  });
});
