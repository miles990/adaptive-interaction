// v0.5 Phase 4：Agent Session taxonomy → 角色演出映射、claim≠verified、
// Conversation Provider（本機降級）規則。

import { describe, expect, it } from "vitest";
import { mapRuntimeEvent, pose, reduce, initial } from "../companion/machine";
import { resolveRigAnimation } from "../companion/rig/renderer";
import { LocalTemplateProvider } from "../companion/conversation";

const ev = (state: string, agentId = "codex") => ({
  eventType: "agent.session.state",
  payload: { agentSessionId: "s-1", agentId, state },
});

describe("agent.session.state → 角色演出", () => {
  it("created → 等待表演（依 agent 選 wait-codex/wait-claude）", () => {
    const codex = mapRuntimeEvent(ev("created", "codex"));
    expect(codex).toMatchObject({ kind: "performing", animation: "wait-codex" });
    const claude = mapRuntimeEvent(ev("created", "claude-code"));
    expect(claude).toMatchObject({ kind: "performing", animation: "wait-claude" });
    const other = mapRuntimeEvent(ev("created", "agent.coder"));
    expect(other).toMatchObject({ kind: "performing", animation: "waiting" });
  });

  it("fetched → routing；working → acting；waiting-* → 需要確認", () => {
    expect(mapRuntimeEvent(ev("fetched"))).toMatchObject({ kind: "routing" });
    expect(mapRuntimeEvent(ev("working"))).toMatchObject({ kind: "acting" });
    expect(mapRuntimeEvent(ev("waiting-input"))).toMatchObject({ kind: "requesting-consent" });
    expect(mapRuntimeEvent(ev("waiting-consent"))).toMatchObject({ kind: "requesting-consent" });
  });

  it("claimed-completed → 點頭（verified:false）；verified → 綠勾（verified:true）", () => {
    const claimed = mapRuntimeEvent(ev("claimed-completed"));
    expect(claimed).toMatchObject({ kind: "succeeded", verified: false });
    const verified = mapRuntimeEvent(ev("verified"));
    expect(verified).toMatchObject({ kind: "succeeded", verified: true });
    // 端到端：pose 的誠實 frameSlice → rig 的 claimed/verified 表情。
    let m = reduce({ ...initial, base: "idle" }, claimed!, 0);
    let p = pose(m, 10);
    expect(resolveRigAnimation(p.animation, p.frameSlice).id).toBe("success-claimed");
    m = reduce({ ...initial, base: "idle" }, verified!, 0);
    p = pose(m, 10);
    expect(resolveRigAnimation(p.animation, p.frameSlice).id).toBe("success-verified");
  });

  it("failed/timed-out → 失敗；cancelled/closed → 誠實清場（不演成功）", () => {
    expect(mapRuntimeEvent(ev("failed"))).toMatchObject({ kind: "failed" });
    expect(mapRuntimeEvent(ev("timed-out"))).toMatchObject({ kind: "failed" });
    expect(mapRuntimeEvent(ev("cancelled"))).toEqual({ type: "clear-transient" });
    expect(mapRuntimeEvent(ev("closed"))).toEqual({ type: "clear-transient" });
    expect(mapRuntimeEvent(ev("someday-new-state"))).toBeNull();
  });
});

describe("Conversation Provider（本機模板降級）", () => {
  const provider = new LocalTemplateProvider();
  const ctx = (over?: Record<string, unknown>) => ({
    openAgentSessions: 0,
    msSinceInteraction: 1_000,
    expressiveness: "natural",
    ...over,
  });

  it("打招呼會回話；久未互動 → 歡迎回來", () => {
    const hi = provider.considerReply("嗨嗨", ctx());
    expect(hi.reply).toBeTruthy();
    expect(hi.behaviorIntent).toBe("notice");
    const back = provider.considerReply("hi", ctx({ msSinceInteraction: 60 * 60_000 }));
    expect(back.behaviorIntent).toBe("player-back");
  });

  it("任務語句 → 建議委派（不自行啟動任何 Agent）", () => {
    const r = provider.considerReply("幫我修這個測試", ctx());
    expect(r.suggestDelegate).toBe(true);
    expect(r.reply).toContain("工作");
  });

  it("問句 → 誠實承認本機沒有答案", () => {
    const r = provider.considerReply("宇宙的意義是什麼？", ctx());
    expect(r.suggestDelegate).toBe(false);
    expect(r.reply).toContain("不太確定");
  });

  it("quiet 表現度 → 傾向不回話（決定是否回話是 Provider 的職責）", () => {
    const r = provider.considerReply("好喔", ctx({ expressiveness: "quiet" }));
    expect(r.reply).toBeNull();
    const thanks = provider.considerReply("謝謝", ctx({ expressiveness: "quiet" }));
    expect(thanks.reply).toBeNull();
    expect(thanks.behaviorIntent).toBe("praised"); // 表情仍可有，話不用多
  });

  it("空輸入 → 完全不反應", () => {
    const r = provider.considerReply("   ", ctx());
    expect(r.reply).toBeNull();
    expect(r.behaviorIntent).toBeNull();
  });
});
