// v0.5 修復回歸：放下的四種落地、30fps 降級遲滯、hover 短氣泡、
// 勿擾／氣泡／音效開關（全部純函式）。

import { describe, expect, it } from "vitest";
import {
  FRAME_WINDOW,
  frameBudgetPolicy,
  initialFrameBudget,
  pickLanding,
  shouldDrawFrame,
} from "../companion/gameFeel";
import {
  approachAllowed,
  bubbleAllowed,
  bubbleOutcome,
  HOVER_BUBBLE_COOLDOWN_MS,
  HOVER_BUBBLE_MIN_MS,
  hoverBubblePolicy,
  quietBase,
  soundOutcome,
} from "../companion/attention";
import { personalityFor } from "../companion/personality";
import { EXPRESSIONS } from "../companion/rig/expressions";
import { InteractionDirector } from "../companion/director";
import { initialBehavior } from "../companion/behavior";
import { DEFAULT_TUNING } from "../companion/personality";
// CPP：落地美術與 Director 表屬於角色（shu adapter tables）；gameFeel／Director 本身 engine-neutral。
import { SHU_DIRECTOR_TABLES, SHU_LANDING } from "../character/adapters/shuTables";

describe("放下角色的四種落地（§5.2）", () => {
  it("快／落差大 → 踉蹌；貼邊有速度 → 滑倒；慢又低 → 輕巧；其餘站穩", () => {
    expect(pickLanding({ speedPxPerSec: 1_200, heightPx: 50, nearEdge: false }).landing).toBe(
      "wobbly"
    );
    expect(pickLanding({ speedPxPerSec: 200, heightPx: 400, nearEdge: false }).landing).toBe(
      "wobbly"
    );
    expect(pickLanding({ speedPxPerSec: 500, heightPx: 80, nearEdge: true }).landing).toBe("slip");
    expect(pickLanding({ speedPxPerSec: 60, heightPx: 20, nearEdge: false }).landing).toBe("light");
    expect(pickLanding({ speedPxPerSec: 300, heightPx: 120, nearEdge: false }).landing).toBe(
      "steady"
    );
  });

  it("每種落地都對應真實存在、且非 truthState 的表情（小樞表）", () => {
    for (const input of [
      { speedPxPerSec: 1_200, heightPx: 50, nearEdge: false },
      { speedPxPerSec: 500, heightPx: 80, nearEdge: true },
      { speedPxPerSec: 60, heightPx: 20, nearEdge: false },
    ]) {
      const plan = pickLanding(input, SHU_LANDING);
      expect(plan.expression).not.toBeNull();
      const expr = EXPRESSIONS[plan.expression!];
      expect(expr, String(plan.expression)).toBeTruthy();
      expect(expr.truthState ?? false, `${plan.expression} 不得是真相狀態`).toBe(false);
      expect(plan.durationMs).toBeGreaterThan(0);
    }
    // 站穩＝不加演出（expression null、durationMs 0，呼叫端不會送 transient）。
    const steady = pickLanding({ speedPxPerSec: 300, heightPx: 120, nearEdge: false }, SHU_LANDING);
    expect(steady).toEqual({ landing: "steady", expression: null, durationMs: 0 });
  });

  it("沒有角色落地表（文字角色）：判定照舊，但沒有任何表情可演", () => {
    const plan = pickLanding({ speedPxPerSec: 1_200, heightPx: 50, nearEdge: false });
    expect(plan.landing).toBe("wobbly");
    expect(plan.expression).toBeNull();
    expect(plan.durationMs).toBe(0);
  });

  it("NaN／負數輸入不會產生奇怪結果", () => {
    expect(pickLanding({ speedPxPerSec: Number.NaN, heightPx: -50, nearEdge: false }).landing).toBe(
      "light"
    );
  });
});

describe("幀預算：30fps 降級與遲滯（§14）", () => {
  const feed = (state: ReturnType<typeof initialFrameBudget>, ms: number, frames = FRAME_WINDOW) => {
    let s = state;
    for (let i = 0; i < frames; i++) s = frameBudgetPolicy(s, ms);
    return s;
  };

  it("平均 >12ms → 每兩幀畫一次；<8ms 才回到 60fps（遲滯）", () => {
    let s = initialFrameBudget();
    expect(s.skipEveryOther).toBe(false);
    s = feed(s, 20); // 明顯太慢
    expect(s.skipEveryOther).toBe(true);
    expect(s.avgMs).toBeCloseTo(20, 5);
    s = feed(s, 9); // 好一點，但還沒到 8ms → 維持降級
    expect(s.skipEveryOther).toBe(true);
    s = feed(s, 5); // 夠快了 → 回到 60fps
    expect(s.skipEveryOther).toBe(false);
    // 10ms（介於 8 與 12 之間）不該讓它又降級。
    s = feed(s, 10);
    expect(s.skipEveryOther).toBe(false);
  });

  it("窗未滿不做決策（避免單幀抖動就降級）", () => {
    let s = initialFrameBudget();
    s = feed(s, 40, FRAME_WINDOW - 1);
    expect(s.skipEveryOther).toBe(false);
    expect(s.count).toBe(FRAME_WINDOW - 1);
  });

  it("降級時每兩幀畫一次", () => {
    const slow = { count: 0, sumMs: 0, avgMs: 20, skipEveryOther: true };
    expect(shouldDrawFrame(slow, 0)).toBe(true);
    expect(shouldDrawFrame(slow, 1)).toBe(false);
    const fast = { count: 0, sumMs: 0, avgMs: 5, skipEveryOther: false };
    expect(shouldDrawFrame(fast, 1)).toBe(true);
  });

  it("輸入是繪製成本、不是 rAF 間隔：把 60Hz 的 16.67ms 餵進來會被誤判成太慢且永遠回不來", () => {
    // 這就是 stage.loop 不能拿 rAF 間隔當幀時間的原因（對抗審查 perf-claims-017）：
    // 60Hz 螢幕的間隔恆為 16.67ms > 12ms → 降級；又 ≥ 8ms → 永遠回不到 60fps。
    const asInterval = feed(initialFrameBudget(), 1000 / 60);
    expect(asInterval.skipEveryOther).toBe(true);
    expect(feed(asInterval, 1000 / 60).skipEveryOther).toBe(true);
    // 真正的繪製成本（60Hz 下零～幾 ms）不會降級。
    expect(feed(initialFrameBudget(), 0).skipEveryOther).toBe(false);
    expect(feed(initialFrameBudget(), 3).skipEveryOther).toBe(false);
  });
});

describe("Hover 短氣泡（§5.1-3）", () => {
  const base = {
    hoverMs: HOVER_BUBBLE_MIN_MS + 10,
    nowMs: 1_000_000,
    lastBubbleAt: 0,
    bubblesEnabled: true,
    approachEnabled: true,
    quiet: false,
    personality: personalityFor("natural"),
    rand: 0.1,
  };

  it("停留超過 700ms 才說一句，且是本機模板短句", () => {
    expect(hoverBubblePolicy({ ...base, hoverMs: 300 })).toMatchObject({
      show: false,
      reason: "too-short",
    });
    const shown = hoverBubblePolicy(base);
    expect(shown.show).toBe(true);
    expect(typeof shown.text).toBe("string");
    expect(shown.text!.length).toBeGreaterThan(0);
  });

  it("45 秒內不重複打擾", () => {
    expect(
      hoverBubblePolicy({ ...base, lastBubbleAt: base.nowMs - HOVER_BUBBLE_COOLDOWN_MS + 1_000 })
    ).toMatchObject({ show: false, reason: "cooldown" });
    expect(
      hoverBubblePolicy({ ...base, lastBubbleAt: base.nowMs - HOVER_BUBBLE_COOLDOWN_MS - 1 }).show
    ).toBe(true);
  });

  it("關掉氣泡／關掉主動靠近／安靜時段都不說話", () => {
    expect(hoverBubblePolicy({ ...base, bubblesEnabled: false }).show).toBe(false);
    expect(hoverBubblePolicy({ ...base, approachEnabled: false }).show).toBe(false);
    expect(hoverBubblePolicy({ ...base, quiet: true })).toMatchObject({
      show: false,
      reason: "quiet",
    });
  });

  it("個性影響選句（慵懶與好奇說的不一樣）", () => {
    const lazy = hoverBubblePolicy({
      ...base,
      personality: { smart: 0.4, witty: 0.2, playful: 0.1, lazy: 1, proud: 0.2, curious: 0.1 },
    });
    const curious = hoverBubblePolicy({
      ...base,
      personality: { smart: 0.4, witty: 0.2, playful: 0.1, lazy: 0.1, proud: 0.2, curious: 1 },
    });
    expect(lazy.text).not.toBe(curious.text);
  });
});

describe("勿擾／氣泡／音效開關（§5.2 可分別關閉）", () => {
  it("勿擾＝quiet 基態；主動靠近同時被關掉", () => {
    expect(quietBase({ quietHours: false, doNotDisturb: false })).toBe(false);
    expect(quietBase({ quietHours: true, doNotDisturb: false })).toBe(true);
    expect(quietBase({ quietHours: false, doNotDisturb: true })).toBe(true);
    expect(
      approachAllowed({ quietHours: false, doNotDisturb: true, approachEnabled: true })
    ).toBe(false);
    expect(
      approachAllowed({ quietHours: false, doNotDisturb: false, approachEnabled: true })
    ).toBe(true);
  });

  it("quiet 基態時 Director 只回眨眼類", () => {
    const d = new InteractionDirector(DEFAULT_TUNING, SHU_DIRECTOR_TABLES);
    const ctx = {
      nowMs: 1_000_000,
      ambient: true,
      quiet: quietBase({ quietHours: false, doNotDisturb: true }),
      reducedMotion: false,
      expressiveness: 1,
      msSinceInteraction: 600_000,
      behavior: { ...initialBehavior(0), activation: 0.05, taskLoad: 0 },
    };
    for (let i = 0; i < 30; i++) {
      const a = d.tick({ ...ctx, nowMs: ctx.nowMs + i * 1_000 }, () => (i % 3 === 0 ? 0.001 : 0.6));
      if (a) expect(a.expression).toBe("blink");
    }
  });

  it("氣泡關掉後只剩安全文字", () => {
    expect(bubbleAllowed({ enabled: false, safety: false })).toBe(false);
    expect(bubbleAllowed({ enabled: false, safety: true })).toBe(true);
    expect(bubbleAllowed({ enabled: true, safety: false })).toBe(true);
  });

  it("氣泡關閉時，Runtime 要求的訊息誠實回報 failed（不假裝顯示過）", () => {
    const off = bubbleOutcome(false);
    expect(off.show).toBe(false);
    expect(off.outcome).toBe("failed");
    expect(off.detail).toContain("沒有顯示");
    expect(bubbleOutcome(true)).toEqual({ show: true });
  });

  it("音效關閉時不播，而且誠實回報 failed（不假裝播過）", () => {
    const off = soundOutcome(false);
    expect(off.play).toBe(false);
    expect(off.outcome).toBe("failed");
    expect(off.detail).toContain("關閉");
    expect(soundOutcome(true)).toEqual({ play: true });
  });
});
