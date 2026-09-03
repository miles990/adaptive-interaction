// v0.5.1 對抗審查（review3, 0c845e0）— 角色視窗（CompanionApp）面向的 regression。
//
// 每一則都對應一個 confirmed finding，且**在修正前會紅**：
//   ia-settings-005       角色視窗的感測標籤走狀態投影（不印 runtime 原始 id），
//                         且 iPhone 麥克風（iphone.mic-level）算「使用中的麥克風」
//   companion-gameplay-032 CompanionInput 的氣泡走同一個 showBubble 管道（同一個
//                         bubbleTimer 主人），不自排未追蹤的計時器抹掉安全氣泡
//   director-pipeline-018 點擊／拖曳中斷長 ambient 時，恢復計畫在 reactDetailed
//                         清掉 Director 的 interrupted 之前就先留一份（§6.1 可恢復）
//   perf-claims-017       角色視窗隱藏後，沒有觀眾的演出（micro-motion／姿勢刷新／
//                         互動框回報／Director ambient 排程）停下，狀態輪詢降頻

import fs from "node:fs";
import path from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { api } from "../api";
import { companionSensorLabel } from "../companion/sensorLabels";
import {
  companionPumpWork,
  CompanionInput,
  resumePlanFor,
  statusPollIntervalMs,
  takeResumePlan,
} from "../companion/CompanionApp";
import { EMPTY_DIRECTOR_TABLES, InteractionDirector } from "../companion/director";
import { initial, reduce } from "../companion/machine";

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

const companionSrc = () =>
  fs.readFileSync(path.resolve("src/companion/CompanionApp.tsx"), "utf8");

// ---------------------------------------------------------------------------
// ia-settings-005
// ---------------------------------------------------------------------------

describe("ia-settings-005：角色視窗的感測標籤走狀態投影", () => {
  it("iPhone 麥克風（iphone.mic-level）算「正在使用麥克風」", () => {
    expect(companionSensorLabel([{ kind: "iphone.mic-level" }])).toBe("🎙 正在使用麥克風");
    expect(companionSensorLabel([{ kind: "microphone" }])).toBe("🎙 正在使用麥克風");
    // 大小寫／變形也不能漏判（host_safety.rs 的 is_mic_kind 同一規則）。
    expect(companionSensorLabel([{ kind: "IPHONE.MIC-LEVEL" }])).toBe("🎙 正在使用麥克風");
  });

  it("認不得的種類投影成「其他感測器」，絕不把原始 id 印上畫面", () => {
    const label = companionSensorLabel([{ kind: "iphone.motion" }]);
    expect(label).toBe("使用中：其他感測器");
    expect(label).not.toContain("iphone.motion");
  });

  it("同種類的多筆來源只說一次（兩台 iPhone 同時串流不會變成重複字串）", () => {
    expect(
      companionSensorLabel([{ kind: "iphone.motion" }, { kind: "iphone.motion" }])
    ).toBe("使用中：其他感測器");
    expect(
      companionSensorLabel([{ kind: "camera" }, { kind: "iphone.motion" }])
    ).toBe("使用中：攝影機、其他感測器");
  });

  it("沒有感測器就沒有標籤；有麥克風時麥克風優先", () => {
    expect(companionSensorLabel([])).toBeNull();
    expect(
      companionSensorLabel([{ kind: "iphone.motion" }, { kind: "iphone.mic-level" }])
    ).toBe("🎙 正在使用麥克風");
  });

  it("CompanionApp 真的用它（不是自己手刻 kind 比對／原樣 join）", () => {
    const src = companionSrc();
    expect(src).toContain("companionSensorLabel(");
    expect(src).not.toContain('x.kind === "microphone"');
    expect(src).not.toMatch(/sensors\.map\(\(x\) => x\.kind\)\.join/);
  });
});

// ---------------------------------------------------------------------------
// companion-gameplay-032
// ---------------------------------------------------------------------------

describe("companion-gameplay-032：CompanionInput 的氣泡不繞過安全氣泡的 sticky 規則", () => {
  it("送出後把存活時間交給 onBubble，不自排未追蹤的計時器", async () => {
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([]);
    vi.useFakeTimers();
    const calls: Array<[string | null, number | undefined]> = [];
    render(
      <CompanionInput
        name="小樞"
        onClose={() => {}}
        onBubble={(text, ms) => calls.push([text, ms])}
        line={(k: string) => k}
        submit={async () => {}}
      />
    );
    const input = screen.getByLabelText("訊息內容");
    fireEvent.change(input, { target: { value: "你好" } });
    await act(async () => {
      fireEvent.keyDown(input, { key: "Enter" });
    });
    expect(calls.length).toBe(1);
    // 存活時間必須是參數（由 showBubble 的 bubbleTimer 統一持有），不是自己的 setTimeout。
    expect(calls[0][1]).toBe(3500);
    // 舊行為：3500ms 後自己 setBubble(null) —— 會把期間出現的安全氣泡抹掉。
    await act(async () => {
      vi.advanceTimersByTime(10_000);
    });
    expect(calls.filter(([text]) => text === null)).toHaveLength(0);
  });

  it("送出失敗時同樣只回報一次（帶存活時間），不自排計時器", async () => {
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([]);
    vi.useFakeTimers();
    const calls: Array<[string | null, number | undefined, { safety?: boolean } | undefined]> = [];
    render(
      <CompanionInput
        name="小樞"
        onClose={() => {}}
        onBubble={(text, ms, opts) => calls.push([text, ms, opts])}
        line={(k: string) => k}
        submit={async () => {
          throw new Error("boom");
        }}
      />
    );
    const input = screen.getByLabelText("訊息內容");
    fireEvent.change(input, { target: { value: "你好" } });
    await act(async () => {
      fireEvent.keyDown(input, { key: "Enter" });
    });
    expect(calls.length).toBe(1);
    expect(calls[0][0]).toContain("送出失敗");
    expect(calls[0][1]).toBe(4000);
    // 失敗結果不得被「關掉氣泡」的偏好靜默吞掉。
    expect(calls[0][2]).toEqual({ safety: true });
    await act(async () => {
      vi.advanceTimersByTime(10_000);
    });
    expect(calls.filter(([text]) => text === null)).toHaveLength(0);
  });

  it("CompanionApp 把 showBubble 接給 CompanionInput，且 send() 內沒有自排的 onBubble(null)", () => {
    const src = companionSrc();
    expect(src).not.toContain("onBubble={setBubble}");
    expect(src).toMatch(/onBubble=\{\([\s\S]{0,120}showBubble\(/);
    expect(src).not.toMatch(/setTimeout\(\(\) => onBubble\(null\)/);
  });
});

// ---------------------------------------------------------------------------
// director-pipeline-018
// ---------------------------------------------------------------------------

describe("director-pipeline-018：點擊／拖曳中斷長 ambient 後真的恢復得回去", () => {
  const T0 = 1_000_000;

  it("resumePlanFor 只對「夠長、剩得夠多」的動作留計畫（與 Director 同一門檻）", () => {
    expect(
      resumePlanFor({ animation: "tailhug", durationMs: 7000, startedAt: T0 }, T0 + 1000)
    ).toEqual({ animation: "tailhug", remainingMs: 6000, expiresAt: T0 + 1000 + 20_000 });
    // 短反應（點擊 700ms／抱起 1500ms）不值得恢復。
    expect(resumePlanFor({ animation: "poked", durationMs: 700, startedAt: T0 }, T0)).toBeNull();
    // 已經快播完（剩 <= 1500ms）也不恢復。
    expect(
      resumePlanFor({ animation: "tailhug", durationMs: 7000, startedAt: T0 }, T0 + 6000)
    ).toBeNull();
    expect(resumePlanFor(null, T0)).toBeNull();
  });

  it("takeResumePlan：過期就放棄，安靜／Reduced Motion 時先不演，其餘取用一次", () => {
    const plan = { animation: "tailhug", remainingMs: 6000, expiresAt: T0 + 20_000 };
    expect(
      takeResumePlan(plan, { nowMs: T0 + 21_000, quiet: false, reducedMotion: false })
    ).toEqual({ plan: null, action: null });
    expect(
      takeResumePlan(plan, { nowMs: T0 + 1000, quiet: true, reducedMotion: false })
    ).toEqual({ plan, action: null });
    expect(
      takeResumePlan(plan, { nowMs: T0 + 1000, quiet: false, reducedMotion: true })
    ).toEqual({ plan, action: null });
    const taken = takeResumePlan(plan, { nowMs: T0 + 1000, quiet: false, reducedMotion: false });
    expect(taken.action).toEqual(plan);
    expect(taken.plan).toBeNull();
    expect(takeResumePlan(null, { nowMs: T0, quiet: false, reducedMotion: false })).toEqual({
      plan: null,
      action: null,
    });
  });

  it("App 的實際點擊順序（先留計畫 → reactDetailed → apply）之後，長 ambient 回得去", () => {
    const tables = {
      ...EMPTY_DIRECTOR_TABLES,
      reactions: { poked: ["poked-flinch"] },
      isPlayable: (e: string) => e === "poked-flinch",
    };
    const d = new InteractionDirector(undefined, tables);
    // ambient 開演（App 端鏡像 Director 的 currentAction）。
    const ambient = { animation: "tailhug", durationMs: 7000, startedAt: T0 };
    let machine = reduce(initial, { type: "base", base: "idle" }, T0);
    machine = reduce(
      machine,
      { type: "transient", kind: "performing", animation: "tailhug", durationMs: 7000 },
      T0
    );
    // 點擊：App 先留一份恢復計畫，再走 Director 的反應。
    const plan = resumePlanFor(ambient, T0 + 1000);
    d.reactDetailed("poked", T0 + 1000, 700, () => 0.5);
    d.notePreempted(T0 + 1000); // apply() 內的 wasPreempted 分支
    machine = reduce(
      machine,
      { type: "transient", kind: "clicked", animation: "poked-flinch", durationMs: 700 },
      T0 + 1000
    );
    // Director 自己的恢復計畫已被 reactDetailed 清掉（根因仍在 director.ts）——
    // 30 秒的 tick 一個 resume 都不會出來。
    const resumes: string[] = [];
    for (let i = 1; i <= 60; i++) {
      const a = d.tick(
        {
          nowMs: T0 + 1000 + i * 500,
          ambient: true,
          quiet: false,
          reducedMotion: false,
          expressiveness: 1,
          msSinceInteraction: 60_000,
          behavior: { taskLoad: 0 } as never,
        },
        () => 0.999
      );
      if (a?.source === "resume") resumes.push(a.expression);
    }
    expect(resumes).toEqual([]);
    // App 留的計畫讓它回得去：短反應播完後取用。
    const taken = takeResumePlan(plan, { nowMs: T0 + 2000, quiet: false, reducedMotion: false });
    expect(taken.action?.animation).toBe("tailhug");
    expect(taken.action?.remainingMs).toBe(6000);
  });

  it("CompanionApp 在點擊／拖曳反應之前就留下恢復計畫", () => {
    const src = companionSrc();
    const click = src.indexOf("const plan = planClickReaction(");
    const drag = src.indexOf('reactDetailed("lifted"');
    expect(click).toBeGreaterThan(0);
    expect(drag).toBeGreaterThan(0);
    // 兩條路徑都要在 react 之前呼叫 resumePlanFor（同一個函式內、前面）。
    const beforeClick = src.slice(Math.max(0, click - 700), click);
    const beforeDrag = src.slice(Math.max(0, drag - 700), drag);
    expect(beforeClick).toContain("resumePlanFor(");
    expect(beforeDrag).toContain("resumePlanFor(");
    expect(src).toContain("takeResumePlan(");
  });
});

// ---------------------------------------------------------------------------
// perf-claims-017
// ---------------------------------------------------------------------------

describe("perf-claims-017：隱藏 = 沒有觀眾的演出停下", () => {
  it("companionPumpWork：隱藏時只留看門狗與行為記帳", () => {
    expect(companionPumpWork(false)).toEqual({ sweep: true, behavior: true, present: true });
    expect(companionPumpWork(true)).toEqual({ sweep: true, behavior: true, present: false });
  });

  it("statusPollIntervalMs：隱藏時降頻，但不是關掉（緊急停止仍要有輪詢後盾）", () => {
    expect(statusPollIntervalMs(false)).toBe(5000);
    expect(statusPollIntervalMs(true)).toBe(30_000);
  });

  it("pump 在 stepBehavior 之後、micro-motion 之前就以可見性短路", () => {
    const src = companionSrc();
    const step = src.indexOf("behaviorState.current = stepBehavior(");
    const micro = src.indexOf("rendererRef.current?.setMicroMotion(");
    expect(step).toBeGreaterThan(0);
    expect(micro).toBeGreaterThan(step);
    const between = src.slice(step, micro);
    expect(between).toMatch(/if \(!work\.present\) return;/);
    expect(src).toContain("companionPumpWork(document.hidden)");
  });

  it("狀態輪詢用 statusPollIntervalMs 排程，且回到可見時立刻補一次", () => {
    const src = companionSrc();
    expect(src).toContain("statusPollIntervalMs(document.hidden)");
    expect(src).not.toMatch(/setInterval\(poll, 5000\)/);
    expect(src).toMatch(/pollVisibility|onPollVisibility/);
  });
});
