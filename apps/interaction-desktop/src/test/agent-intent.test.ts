// v0.5 Phase 4：Agent Session taxonomy → 角色演出映射、claim≠verified、
// Conversation Provider（本機降級）規則。

import { describe, expect, it } from "vitest";
import { mapRuntimeEvent, pose, reduce, initial } from "../companion/machine";
import { resolveRigAnimation } from "../companion/rig/renderer";
import { LocalTemplateProvider } from "../companion/conversation";
// CPP：等哪個 agent 的表情屬於角色表（shu adapter）；machine 預設只給 canonical 的 waiting。
import { SHU_EVENT_ART } from "../character/adapters/shuTables";

const ev = (state: string, agentId = "codex") => ({
  eventType: "agent.session.state",
  payload: { agentSessionId: "s-1", agentId, state },
});

describe("agent.session.state → 角色演出", () => {
  it("created → 等待表演（依 agent 選 wait-codex/wait-claude）", () => {
    const codex = mapRuntimeEvent(ev("created", "codex"), SHU_EVENT_ART);
    expect(codex).toMatchObject({ kind: "performing", animation: "wait-codex" });
    const claude = mapRuntimeEvent(ev("created", "claude-code"), SHU_EVENT_ART);
    expect(claude).toMatchObject({ kind: "performing", animation: "wait-claude" });
    const other = mapRuntimeEvent(ev("created", "agent.coder"), SHU_EVENT_ART);
    expect(other).toMatchObject({ kind: "performing", animation: "waiting" });
    // 沒有角色表：engine-neutral 的 machine 只認 canonical 的 waiting。
    expect(mapRuntimeEvent(ev("created", "codex"))).toMatchObject({ kind: "performing", animation: "waiting" });
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

  it("unknown／expired → 演 unknown：結果沒人知道時不能停在上一個狀態（例如永遠的「工作中」）", () => {
    expect(mapRuntimeEvent(ev("unknown"))).toMatchObject({ type: "transient", kind: "unknown" });
    expect(mapRuntimeEvent(ev("expired"))).toMatchObject({ type: "transient", kind: "unknown" });
    // 端到端（舊路徑 reduce/pose）：working（acting，8 秒）之後收到 unknown，
    // 姿勢要立刻變 unknown，而不是繼續演 act 到 transient 自然到期。
    let m = reduce({ ...initial, base: "idle" }, mapRuntimeEvent(ev("working"))!, 0);
    expect(pose(m, 100).animation).toBe("act");
    m = reduce(m, mapRuntimeEvent(ev("unknown"))!, 100);
    const p = pose(m, 110);
    expect(p.animation).toBe("unknown");
    expect(p.animation).not.toBe("success");
    expect(pose(m, 2_000).animation).toBe("unknown"); // 不會過早回到 act／idle
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
