// v0.5 對抗審查 run-2（2e02284-20260902T140445Z）confirmed 缺陷的 regression（角色視窗／rig／遊玩場）：
//  rig-renderer-011／companion-gameplay-001  ask（需要確認）整體搶佔，遊玩場停
//  director-pipeline-021                     presence-set 經 host 真的顯示／隱藏；拒絕＝failed
//  character-protocol-027                    Runtime 端斷線／重啟後重新 hello（節流）
//  character-protocol-028                    file-dropped 一檔一事件、README §6 扁平鍵
//  companion-gameplay-002／003               使魔遵守 quiet／玩耍開關／真相狀態；凍結時不動、Roll Call 誠實
//  companion-gameplay-004                    跟游標走的玩具不算進互動框、不可抓
//  companion-gameplay-005                    點擊／hover／拖起 ≥3 變體＋冷卻＋防重複；點擊經 Director
//  rig-renderer-012／013／016                lie-flat enter 全程 poseBlend；exit 從當前參數起算；手臂姿勢混合
//  rig-renderer-014                          趴著／打盹＋工作／等待通道可疊（不被拉起來）
//  director-pipeline-022                     cancel 不清安全 transient／安全氣泡；clear-all 才 force
//  companion-gameplay-006                    看向游標只由 companionApproach 控制（勿擾也不看）
//  companion-gameplay-007                    凍結時不能生成／拖曳玩具
//  companion-gameplay-008                    拖放預覽：大小／類型（不知道就說不知道）與去向／可讀 Agent
//  companion-gameplay-009                    hover-left 真的送出
//  director-pipeline-025                     performing→performing 替換要通知 Director
//  director-pipeline-026                     react() 回 null 記原因；L1 thinking 有對應表情；連戳冷卻退回一般點擊
// 所有數字皆為 jsdom 假 canvas 的模擬器結果。

import fs from "node:fs";
import path from "node:path";
import { describe, expect, it, vi } from "vitest";
import { blendArm, clampParams, DEFAULT_PARAMS, lerpParams, RIG_PALETTES } from "../companion/rig/params";
import { armHandPoints, layoutFor } from "../companion/rig/draw";
import { ExpressionTimeline } from "../companion/rig/timeline";
import { EXPRESSIONS, OFFICIAL_36, resolveExpression } from "../companion/rig/expressions";
import {
  machineStageFlags,
  nextRestingExpression,
  playfieldActive,
  REST_EXPRESSIONS,
  StageRenderer,
  stageExpressionPlan,
  statusOverlay,
} from "../companion/rig/stage";
import {
  createWorld,
  Familiar,
  grabToyAt,
  rollCall,
  spawnToy,
  StepInputs,
  stepWorld,
  World,
} from "../companion/playfield";
import {
  MachineState,
  pose,
  reduce,
  wasPreempted,
  wasReplacedByPerforming,
} from "../companion/machine";
import { InteractionDirector, REACTION_COOLDOWN_MS } from "../companion/director";
import { DEFAULT_TUNING, personalityFor } from "../companion/personality";
import { HOVER_LINES, hoverBubblePolicy } from "../companion/attention";
import { planClickReaction } from "../companion/gameFeel";
import { applyPresence, cancelEffects, planPresentationCommand } from "../companion/presentationCommands";
import { LocalTemplateProvider } from "../companion/conversation";
import { initialBehavior } from "../companion/behavior";
import {
  adapterReconfigureFor,
  dropDestinationLines,
  dropItemLine,
  dropPreviewItems,
  INITIAL_HELLO_TRACKER,
  inputEventFor,
  REHELLO_MIN_INTERVAL_MS,
  rehelloOnInstanceEvent,
  rehelloOnStatus,
  summarizeForwardDecisions,
} from "../companion/gatewayWiring";
import { SHU_CLICK_COOLDOWN_MS, SHU_DIRECTOR_TABLES, SHU_REACTIONS } from "../character/adapters/shuTables";
import { ShuCharacterAdapter } from "../character/adapters/shu";
import type { AdapterHost } from "../character/adapter";

// ---------------------------------------------------------------------------
// 測試工具
// ---------------------------------------------------------------------------

function stubCanvas(w = 416, h = 216): HTMLCanvasElement {
  const store: Record<string | symbol, unknown> = {};
  const ctx: unknown = new Proxy(store, {
    get(target, prop) {
      if (prop in target) return target[prop];
      return () => ctx;
    },
    set(target, prop, value) {
      target[prop] = value;
      return true;
    },
  });
  return {
    clientWidth: w,
    clientHeight: h,
    width: w,
    height: h,
    style: {},
    getContext: () => ctx,
    getBoundingClientRect: () => ({ left: 0, top: 0, width: w, height: h }),
  } as unknown as HTMLCanvasElement;
}

function recordingCanvas(w = 416, h = 216): { canvas: HTMLCanvasElement; take: () => string[] } {
  let log: string[] = [];
  const store: Record<string | symbol, unknown> = {};
  const fmt = (v: unknown) => (typeof v === "number" ? v.toFixed(4) : String(v));
  const ctx: unknown = new Proxy(store, {
    get(target, prop) {
      if (prop in target) return target[prop];
      return (...args: unknown[]) => {
        log.push(`${String(prop)}(${args.map(fmt).join(",")})`);
        return ctx;
      };
    },
    set(target, prop, value) {
      log.push(`${String(prop)}=${fmt(value)}`);
      target[prop] = value;
      return true;
    },
  });
  const canvas = {
    clientWidth: w,
    clientHeight: h,
    width: w,
    height: h,
    style: {},
    getContext: () => ctx,
    getBoundingClientRect: () => ({ left: 0, top: 0, width: w, height: h }),
  } as unknown as HTMLCanvasElement;
  return {
    canvas,
    take: () => {
      const out = log;
      log = [];
      return out;
    },
  };
}

function scriptedRng(values: number[], tail = 0.99): () => number {
  let i = 0;
  return () => (i < values.length ? values[i++] : tail);
}

const PAL = RIG_PALETTES["maid-classic"];
const T0 = 1_000_000;

function familiar(id: string, x: number, over: Partial<Familiar> = {}): Familiar {
  return { id, name: id, palette: "maid-classic", x, vx: 0, facing: 1, state: "idle", stateUntil: 0, greetWith: null, ...over };
}

function stepInputs(over: Partial<StepInputs> = {}): StepInputs {
  return {
    nowMs: 10_000,
    dtMs: 16,
    ambient: true,
    frozen: false,
    quiet: false,
    reducedMotion: false,
    playEnabled: true,
    cursorPlayEnabled: true,
    deskMoveEnabled: false,
    pointer: null,
    ...over,
  };
}

function worldWith(familiars: Familiar[], charX = 160): World {
  const w = createWorld(320, 176);
  return { ...w, char: { ...w.char, x: charX }, familiars };
}

function makeStage(opts: { rng?: () => number; canvas?: HTMLCanvasElement } = {}) {
  const clock = { t: 1_000 };
  const stage = new StageRenderer(opts.canvas ?? stubCanvas(), "maid-classic", 1, {
    autoStart: false,
    rng: opts.rng ?? (() => 0.9),
    now: () => clock.t,
  });
  const frames = (n: number, stepMs = 33) => {
    for (let i = 0; i < n; i++) {
      clock.t += stepMs;
      stage.renderFrame(clock.t);
    }
  };
  return { stage, clock, frames };
}

const companionSrc = () => fs.readFileSync(path.resolve("src/companion/CompanionApp.tsx"), "utf8");

// ---------------------------------------------------------------------------
// rig-renderer-011 / companion-gameplay-001：ask 整體搶佔
// ---------------------------------------------------------------------------

describe("ask（需要確認）整體搶佔遊玩場", () => {
  it("statusOverlay(ask)=takeover；計畫不套遊玩表情、不疊通道；遊玩場停", () => {
    expect(statusOverlay("ask")).toBe("takeover");
    expect(stageExpressionPlan("ask", "chase")).toEqual({ expression: "ask", useMachineSlice: true, statusChannels: null });
    expect(stageExpressionPlan("ask", "stroll")).toEqual({ expression: "ask", useMachineSlice: true, statusChannels: null });
    expect(playfieldActive("ask", false, false)).toBe(false);
    expect(machineStageFlags("idle", { kind: "requesting-consent" }, "ask", false).ambient).toBe(false);
  });

  it("所有 truthState 表情都不可能只是 overlay", () => {
    for (const id of OFFICIAL_36) {
      if (EXPRESSIONS[id].truthState) expect(statusOverlay(id), id).not.toBe("overlay");
    }
  });

  it("舞台上 requesting-consent 真的舉手＋問號（不是頭飾亮一點）", () => {
    const { stage, frames } = makeStage();
    stage.setAnimation("idle");
    stage.setMachineFlags(machineStageFlags("idle", null, "idle", true));
    frames(10);
    stage.setAnimation("ask");
    stage.setMachineFlags(machineStageFlags("idle", { kind: "requesting-consent" }, "ask", false));
    frames(40);
    const p = stage.lastFrameParams()!;
    expect(p.armPose).toBe("raise");
    expect(p.overlay).toBe("question");
    stage.destroy();
  });
});

// ---------------------------------------------------------------------------
// director-pipeline-021：presence-set 經 host 真的顯示／隱藏
// ---------------------------------------------------------------------------

describe("presence-set 只有 host 確認後才是 completed", () => {
  it("host 拒絕 → failed（永不 completed）；host 確認 → completed 且以正確的 visible 呼叫", async () => {
    const rejecting = vi.fn(async () => {
      throw new Error("no such command");
    });
    const failed = await applyPresence(planPresentationCommand("presence-set", { visible: false }, true), rejecting);
    expect(rejecting).toHaveBeenCalledWith(false);
    expect(failed.outcome).toBe("failed");
    expect(failed.detail).toContain("not applied");

    const ok = vi.fn(async () => ({ visible: true }));
    const done = await applyPresence(planPresentationCommand("presence-set", { visible: true }, true), ok);
    expect(ok).toHaveBeenCalledWith(true);
    expect(done.outcome).toBe("completed");
  });

  it("CompanionApp 走 desktop.companionSetVisible，不再只寫 prefs", () => {
    const src = companionSrc();
    expect(src).toContain("desktop.companionSetVisible(");
    expect(src).not.toContain("prefsPatch({ companionVisible");
    // ack 在 applyPresence 之後（同一個 handler 內先套用、後 presentationAck）。
    expect(src.indexOf("applyPresence(plan")).toBeLessThan(src.indexOf("api.presentationAck(actionId, plan.outcome"));
  });
});

// ---------------------------------------------------------------------------
// character-protocol-027：重新 hello
// ---------------------------------------------------------------------------

describe("Runtime 端斷線／重啟後重新 hello（節流）", () => {
  const protocolStatus = (startedAt: string) => ({ characterProtocol: {}, startedAt });

  it("feed 出現 → hello；hello 成功且 startedAt 不變 → 不再 hello", () => {
    const first = rehelloOnStatus(INITIAL_HELLO_TRACKER, null, protocolStatus("a"), T0);
    expect(first.hello).toBe(true);
    expect(first.reason).toBe("feed-appeared");
    const sent = { ...first.tracker, sent: true };
    const steady = rehelloOnStatus(sent, "protocol", protocolStatus("a"), T0 + 5_000);
    expect(steady.hello).toBe(false);
    expect(steady.tracker.runtimeStartedAt).toBe("a");
  });

  it("上次 hello 沒成功 → 下一次輪詢再試", () => {
    const tracker = { sent: false, lastAttemptAt: T0, runtimeStartedAt: "a" };
    const d = rehelloOnStatus(tracker, "protocol", protocolStatus("a"), T0 + 5_000);
    expect(d.hello).toBe(true);
    expect(d.reason).toBe("hello-not-sent");
  });

  it("daemon 重啟（startedAt 變了）→ 即使上次 hello 成功也重新 hello", () => {
    const tracker = { sent: true, lastAttemptAt: T0, runtimeStartedAt: "a" };
    const d = rehelloOnStatus(tracker, "protocol", protocolStatus("b"), T0 + 5_000);
    expect(d.hello).toBe(true);
    expect(d.reason).toBe("runtime-restarted");
    expect(d.tracker.runtimeStartedAt).toBe("b");
  });

  it("legacy daemon（沒有 characterProtocol）永遠不 hello", () => {
    const d = rehelloOnStatus({ sent: true, lastAttemptAt: 0, runtimeStartedAt: "a" }, "protocol", { startedAt: "b" }, T0);
    expect(d.hello).toBe(false);
    expect(d.feed).toBe("legacy");
  });

  it("character.instance 把我們標成 connected:false → hello；別的實例／connected:true 不理；2 秒內節流", () => {
    const tracker = { sent: true, lastAttemptAt: 0, runtimeStartedAt: "a" };
    const mine = rehelloOnInstanceEvent(tracker, { instanceId: "desktop-companion", connected: false }, "desktop-companion", T0);
    expect(mine.hello).toBe(true);
    expect(mine.reason).toBe("instance-disconnected");
    expect(mine.tracker.sent).toBe(false);
    expect(rehelloOnInstanceEvent(tracker, { instanceId: "adapter:x", connected: false }, "desktop-companion", T0).hello).toBe(false);
    expect(rehelloOnInstanceEvent(tracker, { instanceId: "desktop-companion", connected: true }, "desktop-companion", T0).hello).toBe(false);
    // 連發：第二次被節流，但 sent 已是 false → 下一次 status 輪詢會補 hello。
    const again = rehelloOnInstanceEvent(mine.tracker, { instanceId: "desktop-companion", connected: false }, "desktop-companion", T0 + 500);
    expect(again.hello).toBe(false);
    expect(again.throttled).toBe(true);
    const later = rehelloOnInstanceEvent(again.tracker, { instanceId: "desktop-companion", connected: false }, "desktop-companion", T0 + REHELLO_MIN_INTERVAL_MS + 1);
    expect(later.hello).toBe(true);
  });

  it("CompanionApp 在通用的 character.* 忽略之前先處理 character.instance，並用 rehelloOnStatus", () => {
    const src = companionSrc();
    expect(src.indexOf('e.eventType === "character.instance"')).toBeGreaterThan(0);
    expect(src.indexOf('e.eventType === "character.instance"')).toBeLessThan(src.indexOf('e.eventType.startsWith("character.")'));
    expect(src).toContain("rehelloOnStatus(helloTrackerRef.current");
    expect(src).not.toContain("helloSentRef");
  });
});

// ---------------------------------------------------------------------------
// character-protocol-028：file-dropped 的原料與轉送彙總
// ---------------------------------------------------------------------------

describe("file-dropped 原料與轉送彙總", () => {
  it("companion-dropped 原料只帶檔名；host 知道的大小／類型才帶，不補 0", () => {
    const ev = inputEventFor("companion-dropped", {
      attachments: ["/Users/x/a.pdf", "C:\\y\\b.png"],
      files: [{ bytes: 1234, mediaType: "application/pdf" }, {}],
    })!;
    expect(ev.kind).toBe("character.file-dropped");
    expect(ev.payload).toEqual({
      files: [{ name: "a.pdf", bytes: 1234, mediaType: "application/pdf" }, { name: "b.png" }],
    });
    expect(JSON.stringify(ev)).not.toContain("/Users/");
  });

  it("多則事件的決定 → 一個總結：任何一則沒送到＝null、任何一則被丟＝dropped、否則 queued；空批次＝null", () => {
    expect(summarizeForwardDecisions([])).toBeNull();
    expect(summarizeForwardDecisions([{ decision: "queued" }, null])).toBeNull();
    expect(summarizeForwardDecisions([{ decision: "queued" }, { decision: "dropped", reason: "invalid-payload" }])).toEqual({
      decision: "dropped",
      reason: "invalid-payload",
    });
    expect(summarizeForwardDecisions([{ decision: "queued" }, { decision: "merged" }])).toEqual({ decision: "queued", reason: undefined });
  });

  it("CompanionApp 的拖放確認等整批結果（awaitForwardBatch），不是只等最後一則", () => {
    const src = companionSrc();
    expect(src).toContain("awaitForwardBatch()");
    expect(src).not.toContain("lastForwardRef");
  });
});

// ---------------------------------------------------------------------------
// companion-gameplay-002 / 003：使魔的閘門與凍結
// ---------------------------------------------------------------------------

describe("使魔遵守 quiet／玩耍開關／真相狀態", () => {
  const cases: Array<[string, Partial<StepInputs>]> = [
    ["quiet", { quiet: true }],
    ["ambient:false（真相狀態在台上）", { ambient: false }],
    ["playEnabled:false", { playEnabled: false }],
  ];
  for (const [label, over] of cases) {
    it(`${label}：使魔不換狀態、不打招呼、主角不回看也不轉身`, () => {
      const w = worldWith([familiar("a", 60)]);
      // rng [0.65, 0.1] 在允許時會走 greet→主角 並回愛心；這裡必須完全不發生。
      const { world, events } = stepWorld(w, stepInputs(over), scriptedRng([0.65, 0.1]));
      expect(world.familiars[0].state).toBe("idle");
      expect(world.familiars[0].x).toBe(60);
      expect(events.filter((e) => e.type === "greeted-by")).toHaveLength(0);
      expect(world.char.attendTo).toBeNull();
      expect(world.char.greetBackUntil).toBe(0);
      expect(world.char.facing).toBe(1);
    });
  }

  it("正在散步的使魔在 quiet 下停在原地（不是繼續走到 stateUntil）", () => {
    const w = worldWith([familiar("a", 60, { state: "walk", vx: 40, stateUntil: 99_999 })]);
    let cur = w;
    for (let i = 0; i < 30; i++) {
      cur = stepWorld(cur, stepInputs({ quiet: true, nowMs: 10_000 + i * 16 }), () => 0.5).world;
    }
    expect(cur.familiars[0].state).toBe("idle");
    expect(cur.familiars[0].x).toBe(60);
    expect(cur.familiars[0].vx).toBe(0);
  });

  it("允許時仍會打招呼（閘門不是把互動整個關掉）", () => {
    const w = worldWith([familiar("a", 60)]);
    const { world, events } = stepWorld(w, stepInputs(), scriptedRng([0.65, 0.1]));
    expect(world.familiars[0].state).toBe("greet");
    expect(events).toContainEqual({ type: "greeted-by", id: "a" });
  });

  it("舞台：blocked／failed／unknown／success-claimed 期間不畫任何愛心、主角不轉身", () => {
    for (const [anim, kind] of [
      ["blocked", "blocked"],
      ["failed", "failed"],
      ["unknown", "unknown"],
      ["success", "succeeded"],
    ] as const) {
      const { stage, frames } = makeStage({ rng: scriptedRng([0.65, 0.2], 0.99) });
      stage.setToggles({ deskMove: false });
      stage.setFamiliars([{ id: "f1", name: "小白", palette: "maid-classic" }]);
      stage.setAnimation("idle");
      stage.setMachineFlags(machineStageFlags("idle", null, "idle", true));
      frames(1); // idle 下：使魔向主角打招呼＋回愛心（互動仍在）
      const heart = vi.spyOn(stage as unknown as { drawGreetHeart: () => void }, "drawGreetHeart");
      stage.setAnimation(anim, anim === "success" ? [0, 1] : undefined);
      stage.setMachineFlags(machineStageFlags("idle", { kind }, anim, false));
      const facingBefore = stage.charFacing();
      frames(30);
      expect(heart, anim).not.toHaveBeenCalled();
      expect(stage.charFacing(), anim).toBe(facingBefore);
      stage.destroy();
    }
  });
});

describe("凍結（緊急停止／離線／暫停）：使魔與玩具的裝飾動畫也停，Roll Call 誠實", () => {
  for (const base of ["emergency", "offline", "paused"] as const) {
    it(`${base}：相隔 60ms 與 10 秒的兩幀完全相同；使魔不再報「在散步」`, () => {
      const rec = recordingCanvas();
      const { stage, clock, frames } = makeStage({ canvas: rec.canvas, rng: scriptedRng([0.5, 0.5, 0.5, 0.5], 0.5) });
      stage.setFamiliars([{ id: "f1", name: "小白", palette: "maid-classic" }]);
      stage.setAnimation("idle");
      stage.setMachineFlags(machineStageFlags("idle", null, "idle", true));
      stage.spawnToy("wand");
      stage.spawnToy("trinket");
      stage.pointerMove(200, 60);
      frames(8); // rng 0.5 → 使魔進入 walk
      expect(stage.rollCallNow(null).some((r) => r.activity === "在散步")).toBe(true);

      stage.setAnimation(base);
      stage.setMachineFlags(machineStageFlags(base, null, base, false));
      stage.renderFrame(clock.t);
      rec.take();
      clock.t += 60;
      stage.renderFrame(clock.t);
      const a = rec.take();
      clock.t += 10_000;
      stage.renderFrame(clock.t);
      const b = rec.take();
      expect(a.length).toBeGreaterThan(50);
      expect(b).toEqual(a);
      const label = base === "emergency" ? "緊急停止中" : base === "offline" ? "離線" : "暫停中";
      for (const row of stage.rollCallNow(label).slice(1)) {
        expect(["在散步", "在追朋友", "在打招呼", "在跟大家打招呼"]).not.toContain(row.activity);
      }
      stage.destroy();
    });
  }

  it("rollCall(frozen)：走路中的使魔誠實回「停下來了」，睡著的照樣「在睡覺」", () => {
    const w = worldWith([familiar("a", 60, { state: "walk" }), familiar("b", 100, { state: "sleep" })]);
    const rows = rollCall(w, "小樞", "緊急停止中", 0, { frozen: true });
    expect(rows.map((r) => r.activity)).toEqual(["緊急停止中", "停下來了", "在睡覺"]);
  });
});

// ---------------------------------------------------------------------------
// companion-gameplay-004：跟游標走的玩具不算互動框、不可抓
// ---------------------------------------------------------------------------

describe("光點／逗貓棒不罩住游標", () => {
  it("光點跟到游標處後，interactiveBounds 不含游標；點下去不會抓到它；毛球仍可抓", () => {
    const { stage, frames } = makeStage({ rng: () => 0.5 });
    stage.setAnimation("idle");
    stage.setMachineFlags(machineStageFlags("idle", null, "idle", true));
    stage.setToggles({ play: false, deskMove: false });
    expect(stage.spawnToy("light")).toBe(true);
    stage.pointerMove(30, 40);
    frames(30, 16);
    const charRect = stage.charHitRect();
    const b = stage.interactiveBounds();
    const contains = (r: { x: number; y: number; w: number; h: number }, x: number, y: number) =>
      x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h;
    expect(contains(b, 30, 40)).toBe(false);
    expect(b).toEqual(charRect);
    expect(stage.pointerDown(30, 40)).toBe("none");
    expect(stage.playerGrabbedToys()).toBe(0);
    // 地面玩具照舊：算進互動框、可抓。
    expect(stage.spawnToy("yarn")).toBe(true);
    frames(60, 16);
    const yarn = stage.toyPoints().find((t) => t.id !== 1)!;
    expect(contains(stage.interactiveBounds(), yarn.x, yarn.y)).toBe(true);
    expect(stage.pointerDown(yarn.x, yarn.y)).toBe("toy");
    expect(stage.playerGrabbedToys()).toBe(1);
    stage.destroy();
  });

  it("grabToyAt 對 light／wand 永遠不命中", () => {
    let w = createWorld(320, 170);
    w = spawnToy(w, "light", 0);
    w = spawnToy(w, "wand", 0);
    for (const t of w.toys) expect(grabToyAt(w, t.x, t.y).toyId).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// companion-gameplay-005：高頻反應變體
// ---------------------------------------------------------------------------

describe("高頻反應 ≥3 變體＋冷卻＋防重複", () => {
  it("小樞表：poked／lifted 各 ≥3 個不同且非 truthState 的表情；hover 每個性 ≥3 句", () => {
    for (const key of ["poked", "lifted"]) {
      const v = SHU_REACTIONS[key];
      expect(Array.isArray(v), key).toBe(true);
      const list = v as readonly string[];
      expect(new Set(list).size).toBeGreaterThanOrEqual(3);
      for (const id of list) expect(resolveExpression(id)?.truthState ?? false, id).toBe(false);
    }
    for (const [trait, lines] of Object.entries(HOVER_LINES)) {
      expect(lines.length, trait).toBeGreaterThanOrEqual(3);
      expect(new Set(lines).size, trait).toBe(lines.length);
    }
  });

  it("Director.react(poked)：短冷卻下連續 12 次 ≥3 種、相鄰不重複；全部冷卻中回 null 並記 cooldown", () => {
    const d = new InteractionDirector(DEFAULT_TUNING, SHU_DIRECTOR_TABLES);
    const seen: string[] = [];
    const rng = scriptedRng([0.1, 0.7, 0.4, 0.9, 0.2, 0.6, 0.3, 0.8, 0.5, 0.05, 0.95, 0.45], 0.5);
    for (let i = 0; i < 12; i++) {
      const a = d.react("poked", T0 + i * 2_000, 700, rng, { cooldownMs: SHU_CLICK_COOLDOWN_MS });
      expect(a, `click ${i}`).not.toBeNull();
      if (seen.length > 0) expect(a!.expression).not.toBe(seen[seen.length - 1]);
      seen.push(a!.expression);
    }
    expect(new Set(seen).size).toBeGreaterThanOrEqual(3);
    // 三個變體都在冷卻：誠實回 null＋原因。
    const fresh = new InteractionDirector(DEFAULT_TUNING, SHU_DIRECTOR_TABLES);
    for (let i = 0; i < 3; i++) fresh.react("poked", T0 + i, 700, () => 0.5, { cooldownMs: 5_000 });
    const d4 = fresh.reactDetailed("poked", T0 + 10, 700, () => 0.5, { cooldownMs: 5_000 });
    expect(d4.action).toBeNull();
    expect(d4.reason).toBe("cooldown");
    expect(fresh.lastDecision()).toEqual(d4);
  });

  it("Director.react(lifted)：三個變體輪流，不會每次都是 lifted", () => {
    const d = new InteractionDirector(DEFAULT_TUNING, SHU_DIRECTOR_TABLES);
    const seen = new Set<string>();
    const rng = scriptedRng([0.1, 0.9, 0.9, 0.1, 0.5, 0.5], 0.5);
    for (let i = 0; i < 6; i++) {
      const a = d.react("lifted", T0 + i * 3_000, 1_500, rng, { cooldownMs: 1_500 });
      if (a) seen.add(a.expression);
    }
    expect(seen.size).toBeGreaterThanOrEqual(3);
  });

  it("hover 短句：給 lastText 時不連說同一句", () => {
    const p = personalityFor("natural");
    const base = { hoverMs: 1_000, nowMs: T0, lastBubbleAt: 0, bubblesEnabled: true, approachEnabled: true, quiet: false, personality: p };
    const first = hoverBubblePolicy({ ...base, rand: 0 });
    expect(first.show).toBe(true);
    for (const rand of [0, 0.01, 0.2, 0.5, 0.9]) {
      const next = hoverBubblePolicy({ ...base, rand, lastText: first.text });
      expect(next.text).not.toBe(first.text);
    }
  });

  it("machine：clicked／dragged 帶變體動畫時 pose 就播那個變體；沒帶維持 canonical 名", () => {
    let s: MachineState = { base: "idle", transient: null };
    s = reduce(s, { type: "transient", kind: "clicked", animation: "poked-grin" }, T0);
    expect(pose(s, T0 + 10).animation).toBe("poked-grin");
    s = reduce({ base: "idle", transient: null }, { type: "transient", kind: "clicked" }, T0);
    expect(pose(s, T0 + 10).animation).toBe("clicked");
    s = reduce({ base: "idle", transient: null }, { type: "transient", kind: "dragged", animation: "lifted-wriggle" }, T0);
    expect(pose(s, T0 + 10).animation).toBe("lifted-wriggle");
    // 直接互動的優先階梯不變：clicked(55) 仍壓過 acting(40)。
    const busy = reduce({ base: "idle", transient: null }, { type: "transient", kind: "acting" }, T0);
    expect(pose(reduce(busy, { type: "transient", kind: "clicked", animation: "poked-flinch" }, T0 + 1), T0 + 2).animation).toBe("poked-flinch");
  });

  it("planClickReaction：單擊走 Director 變體（clicked 55）、連戳走 poked-rapid、連戳冷卻中退回單擊並開選單；文字角色退回 canonical clicked", () => {
    const d = new InteractionDirector(DEFAULT_TUNING, SHU_DIRECTOR_TABLES);
    const single = planClickReaction({ rapid: false, nowMs: T0, director: d, rng: () => 0.5, singleCooldownMs: SHU_CLICK_COOLDOWN_MS });
    expect(single.kind).toBe("single");
    expect(single.transientKind).toBe("clicked");
    expect((SHU_REACTIONS.poked as readonly string[]).includes(single.animation!)).toBe(true);
    expect(single.toggleMenu).toBe(true);
    const rapid = planClickReaction({ rapid: true, nowMs: T0 + 100, director: d, rng: () => 0.5 });
    expect(rapid.kind).toBe("rapid");
    expect(rapid.animation).toBe("poked-rapid");
    expect(rapid.toggleMenu).toBe(false);
    // poked-rapid 8 秒冷卻中的第 4、5 次點擊：不是靜默——退回單擊（有反應、開選單）。
    const during = planClickReaction({ rapid: true, nowMs: T0 + 3_000, director: d, rng: () => 0.5, singleCooldownMs: SHU_CLICK_COOLDOWN_MS });
    expect(during.kind).toBe("single");
    expect(during.toggleMenu).toBe(true);
    expect(d.recentDecisions().some((x) => x.intent === "poked-rapid" && x.reason === "cooldown")).toBe(true);
    const text = new InteractionDirector(DEFAULT_TUNING);
    const fb = planClickReaction({ rapid: false, nowMs: T0, director: text, rng: () => 0.5 });
    expect(fb).toMatchObject({ kind: "fallback", transientKind: "clicked", reason: "no-mapping", toggleMenu: true });
    expect(fb.animation).toBeUndefined();
  });

  it("CompanionApp 的點擊與拖起都經 Director（planClickReaction／reactDetailed(lifted)）", () => {
    const src = companionSrc();
    expect(src).toContain("planClickReaction({");
    expect(src).toContain('reactDetailed("lifted"');
    expect(src).toContain("lastText: lastHoverLineRef.current");
  });
});

// ---------------------------------------------------------------------------
// rig-renderer-012 / 013：時間軸連續性
// ---------------------------------------------------------------------------

describe("時間軸：exit 從當前參數起算；lie-flat enter 全程 poseBlend", () => {
  it("伸懶腰到一半被戳：切換後第一幀不瞬移（squash／eyeOpen／headNod 連續、手臂不立刻收回）", () => {
    const tl = new ExpressionTimeline(() => 0.5, 0);
    tl.setAnimation("stretch", 0);
    const before = tl.paramsAt(845);
    expect(before.armPose).toBe("stretch");
    expect(before.squash).toBeLessThan(-0.15);
    tl.setAnimation("poked", 845);
    let prev = before;
    let maxSquash = 0;
    let maxNod = 0;
    let maxEye = 0;
    // 離開段期間（stretch 的派生 exit 140ms）逐幀必須連續；之後是 poked 自己的 enter（設計上的 squash 打擊）。
    for (let t = 861; t <= 845 + 140; t += 16) {
      const p = tl.paramsAt(t);
      maxSquash = Math.max(maxSquash, Math.abs(p.squash - prev.squash));
      maxNod = Math.max(maxNod, Math.abs(p.headNod - prev.headNod));
      maxEye = Math.max(maxEye, Math.abs(p.eyeOpen - prev.eyeOpen));
      prev = p;
    }
    const first = tl.paramsAt(861);
    expect(Math.abs(first.squash - before.squash)).toBeLessThan(0.05);
    expect(first.armPose).toBe("stretch");
    expect(maxSquash).toBeLessThan(0.08);
    expect(maxNod).toBeLessThan(0.15);
    expect(maxEye).toBeLessThan(0.2);
    // 之後真的到了 poked。
    tl.paramsAt(845 + 1_000);
    expect(tl.currentExpression()).toBe("poked");
  });

  it("撲空中途回 idle：bodyBob／bodyLean 不單幀歸零", () => {
    const tl = new ExpressionTimeline(() => 0.5, 0);
    tl.setAnimation("pounce-miss", 0);
    const before = tl.paramsAt(330);
    tl.setAnimation("idle", 330);
    const after = tl.paramsAt(346);
    expect(Math.abs(after.bodyBob - before.bodyBob)).toBeLessThan(1);
    expect(Math.abs(after.bodyLean - before.bodyLean)).toBeLessThan(1.5);
  });

  for (const from of ["idle", "sit"]) {
    it(`${from} → lie-flat：連續兩幀頭部 y 位移 < 12px，最後真的趴平（hy≈92）`, () => {
      const tl = new ExpressionTimeline(() => 0.5, 0);
      tl.setAnimation(from, 0);
      tl.paramsAt(3_000);
      tl.setAnimation("lie-flat", 3_000);
      let prevY = layoutFor(tl.paramsAt(3_000), PAL).hy;
      let maxJump = 0;
      let at = 0;
      for (let t = 3_000 + 16.7; t <= 4_200; t += 16.7) {
        const y = layoutFor(tl.paramsAt(t), PAL).hy;
        if (Math.abs(y - prevY) > maxJump) {
          maxJump = Math.abs(y - prevY);
          at = t;
        }
        prevY = y;
      }
      expect(maxJump, `worst at +${Math.round(at - 3_000)}ms`).toBeLessThan(12);
      expect(prevY).toBeCloseTo(92, 0);
      expect(tl.paramsAt(4_200).pose).toBe("lie");
    });
  }

  it("lie-flat → idle 仍然平滑（沒有把既有修法弄壞）", () => {
    const tl = new ExpressionTimeline(() => 0.5, 0);
    tl.setAnimation("lie-flat", 0);
    let prevY = layoutFor(tl.paramsAt(3_000), PAL).hy;
    tl.setAnimation("idle", 3_000);
    let maxJump = 0;
    for (let t = 3_000 + 16.7; t <= 4_200; t += 16.7) {
      const y = layoutFor(tl.paramsAt(t), PAL).hy;
      maxJump = Math.max(maxJump, Math.abs(y - prevY));
      prevY = y;
    }
    expect(maxJump).toBeLessThan(12);
    expect(prevY).toBeCloseTo(46);
  });
});

// ---------------------------------------------------------------------------
// rig-renderer-014：趴著＋核心亮
// ---------------------------------------------------------------------------

describe("組合式通道也適用於休息姿勢（趴著／打盹＋工作中）", () => {
  it("nextRestingExpression：休息表情記住、工作/等待保留、其餘清掉", () => {
    expect(nextRestingExpression(null, "lie-flat")).toBe("lie-flat");
    expect(nextRestingExpression("lie-flat", "act")).toBe("lie-flat");
    expect(nextRestingExpression("lie-flat", "waiting")).toBe("lie-flat");
    expect(nextRestingExpression("lie-flat", "idle")).toBeNull();
    expect(nextRestingExpression("lie-flat", "clicked")).toBeNull();
    expect(nextRestingExpression("lie-flat", "blocked")).toBeNull();
    expect(nextRestingExpression("lie-flat", "ask")).toBeNull();
    expect(nextRestingExpression("doze", "wait-codex")).toBe("doze");
    for (const id of REST_EXPRESSIONS) expect(resolveExpression(id), id).toBeTruthy();
  });

  it("stageExpressionPlan(act, free, lie-flat)：身體趴著、只疊 working 的狀態通道；沒有休息姿勢時照舊整體", () => {
    const plan = stageExpressionPlan("act", "free", "lie-flat");
    expect(plan.expression).toBe("lie-flat");
    expect(plan.useMachineSlice).toBe(false);
    expect(plan.statusChannels?.coreGlow).toBe(EXPRESSIONS["working"].hold.coreGlow);
    expect(stageExpressionPlan("act", "free", null)).toEqual({ expression: "act", useMachineSlice: true, statusChannels: null });
    // 安全與結果狀態不管趴不趴，一律整體搶佔。
    expect(stageExpressionPlan("blocked", "free", "lie-flat").expression).toBe("blocked");
    expect(stageExpressionPlan("ask", "free", "lie-flat").expression).toBe("ask");
  });

  it("舞台：lie-flat 被 acting 取代 → pose 仍是 lie、coreGlow=1；回 idle 才站起來", () => {
    const { stage, frames } = makeStage({ rng: () => 0.95 });
    stage.setAnimation("idle");
    stage.setMachineFlags(machineStageFlags("idle", null, "idle", true));
    frames(5);
    stage.setAnimation("lie-flat");
    stage.setMachineFlags(machineStageFlags("idle", { kind: "performing", animation: "lie-flat" }, "lie-flat", false));
    frames(45);
    expect(stage.lastFrameParams()?.pose).toBe("lie");
    stage.setAnimation("act");
    stage.setMachineFlags(machineStageFlags("idle", { kind: "acting" }, "act", false));
    frames(40);
    const p = stage.lastFrameParams()!;
    expect(p.pose).toBe("lie");
    expect(p.coreGlow).toBe(1);
    expect(stage.restingExpression()).toBe("lie-flat");
    stage.setAnimation("idle");
    stage.setMachineFlags(machineStageFlags("idle", null, "idle", true));
    frames(40);
    expect(stage.lastFrameParams()?.pose).toBe("stand");
    expect(stage.restingExpression()).toBeNull();
    stage.destroy();
  });
});

// ---------------------------------------------------------------------------
// director-pipeline-022：cancel 不清安全訊息
// ---------------------------------------------------------------------------

describe("presentation cancel 不抹掉被擋下／失敗／未知", () => {
  it("machine：clear-transient 保留安全 transient，force 才清；performing 照樣被清", () => {
    for (const kind of ["blocked", "failed", "unknown"] as const) {
      const s = reduce({ base: "idle", transient: null }, { type: "transient", kind }, T0);
      expect(reduce(s, { type: "clear-transient" }, T0 + 10).transient?.kind, kind).toBe(kind);
      expect(reduce(s, { type: "clear-transient", force: true }, T0 + 10).transient).toBeNull();
    }
    const perf = reduce({ base: "idle", transient: null }, { type: "transient", kind: "performing", animation: "stretch" }, T0);
    expect(reduce(perf, { type: "clear-transient" }, T0 + 10).transient).toBeNull();
    // force 也不動基態（緊急停止仍由 runtime 擁有）。
    const estop = reduce({ base: "emergency", transient: null }, { type: "clear-transient", force: true }, T0);
    expect(estop.base).toBe("emergency");
  });

  it("cancelEffects：cancel 不 force、不清安全氣泡；clear-all 兩者皆清", () => {
    expect(cancelEffects("cancel")).toEqual({ forceClear: false, clearSafetyBubble: false });
    expect(cancelEffects("clear-all")).toEqual({ forceClear: true, clearSafetyBubble: true });
  });

  it("CompanionApp 的 cancel 分支依 cancelEffects 決定 force 與安全氣泡", () => {
    const src = companionSrc();
    const branch = src.slice(src.indexOf('if (command === "cancel"'), src.indexOf('if (!actionId) return;'));
    expect(branch).toContain("cancelEffects(command)");
    expect(branch).toContain('apply({ type: "clear-transient", force: effects.forceClear })');
    expect(branch).toContain("effects.clearSafetyBubble || !bubbleSafetyRef.current");
  });
});

// ---------------------------------------------------------------------------
// companion-gameplay-006：看向游標只由 companionApproach 控制
// ---------------------------------------------------------------------------

describe("「游標靠近時看過來」只有一個主人", () => {
  it("adapterReconfigureFor 帶 approach；shu adapter 轉給舞台", async () => {
    const ctx = { name: "小樞", characterId: "shu-maid", entrypoint: "shu-rig" as const, tuning: {} };
    expect(adapterReconfigureFor({ companionApproach: false }, ctx).approach).toBe(false);
    expect(adapterReconfigureFor({}, ctx).approach).toBe(true);
    const stage = new StageRenderer(stubCanvas(), "maid-classic", 1, { autoStart: false, now: () => 0 });
    const setToggles = vi.spyOn(stage, "setToggles");
    const adapter = new ShuCharacterAdapter({ stage });
    const host: AdapterHost = { now: () => 0, reducedMotion: () => false, locale: "zh-TW", log: () => {} };
    await adapter.initialize(host);
    adapter.reconfigure({ approach: false, cursorPlay: true });
    expect(setToggles).toHaveBeenCalledWith({ approach: false, cursorPlay: true });
    adapter.dispose();
  });

  it("approach:false（cursorPlay:true）時視線不跟游標；approach:true 才看；勿擾也不看", () => {
    const baseline = (toggles: { approach: boolean }, quiet = false) => {
      const { stage, frames } = makeStage();
      stage.setAnimation(quiet ? "quiet" : "idle");
      stage.setMachineFlags(machineStageFlags(quiet ? "quiet" : "idle", null, quiet ? "quiet" : "idle", !quiet));
      stage.setToggles({ play: false, deskMove: false, cursorPlay: true, ...toggles });
      frames(3, 16);
      const noPointer = stage.lastFrameParams()!;
      const r = stage.charHitRect();
      stage.pointerMove(r.x + r.w + 40, r.y + 40);
      frames(3, 16);
      const withPointer = stage.lastFrameParams()!;
      stage.destroy();
      return { noPointer, withPointer };
    };
    const off = baseline({ approach: false });
    expect(off.withPointer.pupilX).toBeCloseTo(off.noPointer.pupilX, 5);
    expect(off.withPointer.headTurn).toBeCloseTo(off.noPointer.headTurn, 5);
    const on = baseline({ approach: true });
    expect(on.withPointer.pupilX).not.toBeCloseTo(on.noPointer.pupilX, 3);
    const dnd = baseline({ approach: true }, true);
    expect(dnd.withPointer.pupilX).toBeCloseTo(dnd.noPointer.pupilX, 5);
  });
});

// ---------------------------------------------------------------------------
// companion-gameplay-007：凍結時不能生成／拖曳玩具
// ---------------------------------------------------------------------------

describe("凍結時玩具不生成、不可拖", () => {
  it("emergency：spawnToy 拒絕（toyCount 0）；既有玩具點不起來；拖到一半凍結就地放下且零速度", () => {
    const { stage, frames } = makeStage({ rng: () => 0.95 });
    stage.setAnimation("idle");
    stage.setMachineFlags(machineStageFlags("idle", null, "idle", true));
    stage.setToggles({ play: false, deskMove: false });
    expect(stage.spawnToy("yarn")).toBe(true);
    frames(90, 16); // 落地靜止
    const yarn = stage.toyPoints()[0];
    expect(stage.pointerDown(yarn.x, yarn.y)).toBe("toy");
    stage.pointerMove(yarn.x + 30, yarn.y - 40);
    // 拖曳中進入緊急停止。
    stage.setAnimation("emergency");
    stage.setMachineFlags(machineStageFlags("emergency", null, "emergency", false));
    stage.pointerMove(yarn.x + 60, yarn.y - 80);
    expect(stage.playerGrabbedToys()).toBe(0);
    expect(stage.isDraggingToy()).toBe(false);
    const dropped = stage.toyPoints()[0];
    expect(dropped.x).toBeCloseTo(yarn.x + 30);
    expect(stage.spawnToy("plane")).toBe(false);
    expect(stage.toyCount()).toBe(1);
    expect(stage.pointerDown(dropped.x, dropped.y)).not.toBe("toy");
    expect(stage.playerGrabbedToys()).toBe(0);
    // 解凍：玩具只受重力落回地面，沒有殘留的拋擲速度（x 不變）。
    stage.setAnimation("idle");
    stage.setMachineFlags(machineStageFlags("idle", null, "idle", true));
    frames(60, 16);
    expect(stage.toyPoints()[0].x).toBeCloseTo(dropped.x, 0);
    stage.destroy();
  });

  it("shu adapter 的 gameplay.spawnToy 在凍結時回 null（不發 toy-thrown）", async () => {
    const stage = new StageRenderer(stubCanvas(), "maid-classic", 1, { autoStart: false, now: () => 0 });
    const adapter = new ShuCharacterAdapter({ stage });
    const host: AdapterHost = { now: () => 0, reducedMotion: () => false, locale: "zh-TW", log: () => {} };
    await adapter.initialize(host);
    const inputs: string[] = [];
    adapter.onInput((e) => inputs.push(e.kind));
    stage.setMachineFlags(machineStageFlags("emergency", null, "emergency", false));
    expect(adapter.gameplay.spawnToy("yarn")).toBeNull();
    expect(inputs).toHaveLength(0);
    stage.setMachineFlags(machineStageFlags("idle", null, "idle", true));
    expect(adapter.gameplay.spawnToy("yarn")).toBe("yarn");
    expect(inputs).toEqual(["character.toy-thrown"]);
    adapter.dispose();
  });

  it("CompanionApp：凍結時不顯示玩具列、quickToy 直接返回", () => {
    const src = companionSrc();
    expect(src).toContain("toyCatalog.length > 0 && !frozen");
    expect(src).toContain('if (["emergency", "offline", "paused"].includes(machineRef.current.base)) return;');
  });
});

// ---------------------------------------------------------------------------
// companion-gameplay-008：拖放預覽
// ---------------------------------------------------------------------------

describe("拖放預覽：大小／類型與去向", () => {
  it("不知道大小／類型就明說「未知」；知道就顯示 KB 與類型", () => {
    const items = dropPreviewItems(["/tmp/a.txt", "/tmp/b.pdf"], [{}, { bytes: 1234, mediaType: "application/pdf" }]);
    expect(dropItemLine(items[0])).toBe("a.txt（大小／類型：未知）");
    expect(dropItemLine(items[1])).toBe("b.pdf（1.2 KB・application/pdf）");
    expect(dropItemLine({ name: "c.bin", bytes: 5, mediaType: null })).toBe("c.bin（5 B・類型未知）");
    expect(JSON.stringify(items)).not.toContain("/tmp");
  });

  it("去向：本機 Runtime；可讀 Agent 清單拿不到就說拿不到、沒有就說沒有、有就列 label 與可讀範圍", () => {
    expect(dropDestinationLines(null).join("\n")).toContain("清單暫時拿不到");
    expect(dropDestinationLines([]).join("\n")).toContain("目前沒有開啟中的工作階段");
    const lines = dropDestinationLines([
      { sessionId: "s1", label: "修測試", agentId: "codex", dataScope: ["repo", "docs"] },
      { sessionId: "s2", agentId: "claude-code", dataScope: [], closedAt: "2026-01-01T00:00:00Z" },
      { sessionId: "s3", agentId: "claude-code", dataScope: [] },
    ]);
    expect(lines[0]).toContain("本機 Runtime");
    expect(lines).toContain("可讀：修測試・可讀範圍：repo、docs");
    expect(lines).toContain("可讀：claude-code・可讀範圍：未設定");
    expect(lines.filter((l) => l.startsWith("可讀："))).toHaveLength(2); // 已關閉的不算
  });

  it("CompanionApp 的預覽用 dropItemLine／dropDestinationLines，並向 Runtime 問工作階段", () => {
    const src = companionSrc();
    expect(src).toContain("dropItemLine(item)");
    expect(src).toContain("dropDestinationLines(dropSessions)");
    expect(src).toContain("agentSessionsList()");
  });
});

// ---------------------------------------------------------------------------
// companion-gameplay-009：hover-left
// ---------------------------------------------------------------------------

describe("游標離開會送 character.hover-left", () => {
  it("pointer-left 映射存在且沒有座標", () => {
    expect(inputEventFor("pointer-left")).toEqual({ kind: "character.hover-left", payload: {} });
  });

  it("CompanionApp 在 onPointerLeaveCanvas 送 pointer-left（成對：只在送過 hover-entered 之後）", () => {
    const src = companionSrc();
    const fn = src.slice(src.indexOf("function onPointerLeaveCanvas"), src.indexOf("// ---- coarse activity summary"));
    expect(fn).toContain('pushInteraction("pointer-left")');
    expect(fn).toContain("hoverEnteredRef.current");
    expect(src).toContain('"pointer-left": "companion.pointer"');
  });
});

// ---------------------------------------------------------------------------
// director-pipeline-025：performing→performing 替換
// ---------------------------------------------------------------------------

describe("Director 的動作被另一個表演換掉時要知道", () => {
  it("wasReplacedByPerforming：還在播的表演被不同表演換掉才算；到期／同一動畫（refresh）／非表演不算", () => {
    const a = { kind: "performing" as const, animation: "legswing", untilMs: T0 + 6_000 };
    expect(wasReplacedByPerforming(a, { kind: "performing", animation: "device-hello", untilMs: T0 + 1_800 }, T0 + 500)).toBe(true);
    expect(wasReplacedByPerforming(a, { kind: "performing", animation: "legswing", untilMs: T0 + 9_000 }, T0 + 500)).toBe(false);
    expect(wasReplacedByPerforming({ ...a, untilMs: T0 }, { kind: "performing", animation: "x", untilMs: T0 + 1 }, T0 + 1)).toBe(false);
    expect(wasReplacedByPerforming(a, { kind: "clicked", untilMs: T0 + 700 }, T0 + 500)).toBe(false);
    expect(wasPreempted(a, { kind: "performing", animation: "device-hello", untilMs: T0 + 1_800 }, T0 + 500)).toBe(false);
  });

  it("A 被 B 換掉、B 再被點擊搶佔 → 之後不會恢復早已下台的 A", () => {
    const d = new InteractionDirector(DEFAULT_TUNING, SHU_DIRECTOR_TABLES);
    const ctx = (nowMs: number) => ({
      nowMs,
      ambient: true,
      quiet: false,
      reducedMotion: false,
      expressiveness: 1,
      msSinceInteraction: 600_000,
      behavior: { ...initialBehavior(0), activation: 0.05, taskLoad: 0 },
    });
    let action = null;
    let startedAt = 0;
    for (let i = 0; i < 10 && (!action || action.durationMs < 4_000); i++) {
      startedAt = T0 + i * 1_000;
      action = d.tick(ctx(startedAt), () => 0);
    }
    expect(action!.durationMs).toBeGreaterThanOrEqual(4_000);
    let s: MachineState = reduce({ base: "idle", transient: null }, { type: "transient", kind: "performing", animation: action!.expression, durationMs: action!.durationMs }, startedAt);
    const before = s.transient;
    s = reduce(s, { type: "transient", kind: "performing", animation: "device-hello", durationMs: 1_800 }, startedAt + 500);
    expect(wasReplacedByPerforming(before, s.transient, startedAt + 500)).toBe(true);
    d.noteFinished(); // CompanionApp.apply 在 wasReplacedByPerforming 時做的事
    s = reduce(s, { type: "transient", kind: "clicked" }, startedAt + 1_000);
    d.notePreempted(startedAt + 1_000);
    expect(d.tick(ctx(startedAt + 2_000), () => 0.99)).toBeNull();
  });

  it("CompanionApp.apply 在替換時通知 Director", () => {
    const src = companionSrc();
    expect(src).toContain("wasReplacedByPerforming(before, after, now)");
  });
});

// ---------------------------------------------------------------------------
// director-pipeline-026：react() 的原因與 L1 對應
// ---------------------------------------------------------------------------

describe("Director.react 回 null 不是靜默", () => {
  it("每個 L1 本機模板會回的 behaviorIntent 都在小樞表裡且能演", () => {
    const provider = new LocalTemplateProvider();
    const ctx = { openAgentSessions: 0, msSinceInteraction: 0, expressiveness: "natural" };
    const intents = new Set<string>();
    for (const text of ["嗨", "謝謝", "幫我修這個測試", "這是什麼？", "好", "hello", "辛苦了", "refactor the module"]) {
      const r = provider.considerReply(text, ctx);
      if (r.behaviorIntent) intents.add(r.behaviorIntent);
    }
    expect(intents.has("thinking")).toBe(true);
    for (const intent of intents) {
      const d = new InteractionDirector(DEFAULT_TUNING, SHU_DIRECTOR_TABLES).reactDetailed(intent, T0);
      expect(d.action, intent).not.toBeNull();
      expect(d.reason, intent).toBe("ok");
      expect(resolveExpression(d.action!.expression)?.truthState ?? false).toBe(false);
    }
  });

  it("no-mapping／not-playable／cooldown 三種原因都會記下，lastDecision 可讀", () => {
    const d = new InteractionDirector(DEFAULT_TUNING, SHU_DIRECTOR_TABLES);
    expect(d.reactDetailed("celebrate-success", T0).reason).toBe("no-mapping");
    const truth = new InteractionDirector(DEFAULT_TUNING, { ...SHU_DIRECTOR_TABLES, reactions: { bad: "blocked" } });
    expect(truth.reactDetailed("bad", T0).reason).toBe("not-playable");
    expect(d.reactDetailed("poked-rapid", T0).reason).toBe("ok");
    const cd = d.reactDetailed("poked-rapid", T0 + REACTION_COOLDOWN_MS - 1);
    expect(cd.reason).toBe("cooldown");
    expect(cd.action).toBeNull();
    expect(d.lastDecision()?.reason).toBe("cooldown");
    expect(d.recentDecisions().length).toBe(3);
  });
});

// ---------------------------------------------------------------------------
// 保底：既有的 evalPhase／clampParams 行為沒被新通道弄壞
// ---------------------------------------------------------------------------

describe("參數模型保底", () => {
  it("clampParams 對新表情的 hold 全部合法", () => {
    for (const id of ["poked-flinch", "poked-grin", "lifted-curious", "lifted-wriggle"]) {
      const e = EXPRESSIONS[id];
      expect(e, id).toBeTruthy();
      expect(e.truthState ?? false).toBe(false);
      const p = clampParams({ ...DEFAULT_PARAMS, ...e.hold });
      expect(p.armPose).toBeDefined();
    }
  });
});

// ---------------------------------------------------------------------------
// rig-renderer-016：armPose 字串通道不再硬切——armFrom／armBlend 混合兩套手臂幾何
// ---------------------------------------------------------------------------

describe("手臂姿勢切換不單幀瞬移", () => {
  const maxHandJump = (tl: ExpressionTimeline, from: number, to: number) => {
    let prev = armHandPoints(tl.paramsAt(from), PAL);
    let worst = 0;
    let at = 0;
    for (let t = from + 16.7; t <= to; t += 16.7) {
      const cur = armHandPoints(tl.paramsAt(t), PAL);
      for (let i = 0; i < 2; i++) {
        // 看不見的手（pocket：半徑 0）沒有「位置」可言，不量。
        if (cur[i].r < 0.3 || prev[i].r < 0.3) continue;
        const d = Math.hypot(cur[i].x - prev[i].x, cur[i].y - prev[i].y);
        if (d > worst) {
          worst = d;
          at = t;
        }
      }
      prev = cur;
    }
    return { worst, at };
  };

  it("lerpParams：armPose 切換時 armFrom／armBlend 從 0.5 兩側連續帶過；同姿勢不動", () => {
    const front = clampParams({ armPose: "front" });
    const raise = clampParams({ armPose: "raise" });
    const before = lerpParams(front, raise, 0.49);
    expect(before.armPose).toBe("front");
    expect(before.armFrom).toBe("raise");
    expect(before.armBlend).toBeCloseTo(0.51);
    const after = lerpParams(front, raise, 0.51);
    expect(after.armPose).toBe("raise");
    expect(after.armFrom).toBe("front");
    expect(after.armBlend).toBeCloseTo(0.51);
    expect(lerpParams(front, raise, 1)).toMatchObject({ armPose: "raise", armBlend: 1 });
    expect(lerpParams(front, front, 0.5)).toMatchObject({ armPose: "front", armBlend: 1 });
    expect(blendArm(front, raise, raise, 0.2)).toMatchObject({ armPose: "front", armFrom: "raise", armBlend: 0.8 });
    expect(blendArm(front, raise, raise, 0.7)).toMatchObject({ armPose: "raise", armFrom: "front", armBlend: 0.7 });
    expect(blendArm(front, front, raise, 0.2)).toBe(raise);
    // 兩側的手位在切換點連續：{front←raise, .5} 與 {raise←front, .5} 畫出同一個位置。
    const mid = lerpParams(front, raise, 0.5);
    expect(mid).toMatchObject({ armPose: "raise", armFrom: "front", armBlend: 0.5 });
    const l = armHandPoints(clampParams({ ...mid, armPose: "front", armFrom: "raise" }), PAL);
    const r = armHandPoints(mid, PAL);
    for (let i = 0; i < 2; i++) {
      expect(l[i].x).toBeCloseTo(r[i].x, 6);
      expect(l[i].y).toBeCloseTo(r[i].y, 6);
    }
  });

  it("armBlend=1 時手位與原本各姿勢一致（front 交疊、stretch 高舉、pocket 沒有手）", () => {
    const front = armHandPoints(clampParams({ armPose: "front" }), PAL);
    expect(front.map((h) => Math.round(h.y))).toEqual([86, 86]);
    const stretch = armHandPoints(clampParams({ armPose: "stretch", armPhase: 1 }), PAL);
    expect(stretch[0].y).toBeLessThan(45);
    const pocket = armHandPoints(clampParams({ armPose: "pocket" }), PAL);
    expect(pocket.every((h) => h.r === 0)).toBe(true);
  });

  it("stretch 的 enter（front→stretch→front）：連續兩幀手位位移 < 12px（原本 45px 單幀跳）", () => {
    const tl = new ExpressionTimeline(() => 0.5, 0);
    tl.setAnimation("idle", 0);
    tl.paramsAt(500);
    tl.setAnimation("stretch", 500);
    const { worst, at } = maxHandJump(tl, 500, 500 + 1_400);
    expect(worst, `worst at +${Math.round(at - 500)}ms`).toBeLessThan(12);
  });

  it("ask → idle 的 crossfade（raise→front）與 caught-slacking → idle：手位連續", () => {
    for (const from of ["ask", "caught-slacking", "not-found", "working"]) {
      const tl = new ExpressionTimeline(() => 0.5, 0);
      tl.setAnimation(from, 0);
      tl.paramsAt(2_000);
      tl.setAnimation("idle", 2_000);
      const { worst, at } = maxHandJump(tl, 2_000, 2_800);
      expect(worst, `${from}: worst at +${Math.round(at - 2_000)}ms`).toBeLessThan(12);
      expect(tl.paramsAt(3_500).armPose).toBe("front");
    }
  });

  it("Reduced Motion：直接就位，armBlend=1", () => {
    const tl = new ExpressionTimeline(() => 0.5, 0);
    tl.setReducedMotion(true);
    tl.setAnimation("ask", 0);
    expect(tl.paramsAt(10)).toMatchObject({ armPose: "raise", armBlend: 1 });
  });
});
