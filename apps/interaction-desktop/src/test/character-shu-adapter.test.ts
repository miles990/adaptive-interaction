// CPP §12／§13：`shu-rig` Reference Adapter（小樞 v3 rig＋遊玩場）。
//
// manifest／協商（20 個 intent 全 exact）、intent → 表情表（claimed ≠ verified）、
// 安全搶占取消遊玩（cancelled{preempted}）、suspend 停迴圈、Reduced Motion 協商、
// cancel 冪等、回執誠實（accepted → started → completed 由時間軸驅動；沒上台就說沒上台）。
// 這裡的 StageRenderer 用 jsdom 假 canvas＋autoStart:false：所有數字都是模擬器結果。

import { describe, expect, it, vi } from "vitest";
import shuMaidRaw from "../../public/characters/shu-maid/manifest.json";
import legacyRig from "../../public/packs/shu-maid/manifest.json";
import type { AdapterHost, AdapterReceipt } from "../character/adapter";
import { ShuCharacterAdapter } from "../character/adapters/shu";
import {
  isShuPlayable,
  SHU_AMBIENT_VARIANTS,
  SHU_LANDING,
  SHU_MICRO_ACTIONS,
  SHU_REACTIONS,
  shuExpressionPlan,
  shuNaturalDurationMs,
} from "../character/adapters/shuTables";
import { CharacterGateway } from "../character/gateway";
import { shuRigCapabilities, validateCharacterManifest } from "../character/manifest";
import {
  CHARACTER_INTENTS,
  CharacterIntent,
  CharacterManifest,
  CommandReceipt,
  IntentEnvelope,
  PROTOCOL_VERSION,
  TruthState,
} from "../character/protocol";
import { initial, MachineEvent, MachineState, MixerPort, reduce } from "../companion/machine";
import { EXPRESSIONS, OFFICIAL_36, resolveExpression } from "../companion/rig/expressions";
import { StageRenderer } from "../companion/rig/stage";

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

const clock = { now: 1_700_000_000_000 };
const host: AdapterHost = {
  now: () => clock.now,
  reducedMotion: () => false,
  locale: "zh-TW",
  log: () => {},
};

function bundledManifest(): CharacterManifest {
  const v = validateCharacterManifest(shuMaidRaw);
  if (!v.ok) throw new Error(v.errors.join("; "));
  return v.manifest;
}

function makeStage(): StageRenderer {
  return new StageRenderer(stubCanvas(), "maid-classic", 1, { autoStart: false, rng: () => 0.9, now: () => clock.now });
}

let seq = 0;
function env(intent: CharacterIntent, truthState: TruthState = "none", over: Partial<IntentEnvelope> = {}): IntentEnvelope {
  seq += 1;
  return {
    protocolVersion: PROTOCOL_VERSION,
    messageId: `s${seq}`,
    characterInstanceId: "a",
    timestamp: new Date(clock.now).toISOString(),
    intent,
    truthState,
    priority: 10,
    interruptPolicy: "preempt",
    resumePolicy: "none",
    privacyClass: "internal",
    ...over,
  };
}

async function standalone(opts: { reducedMotion?: boolean } = {}) {
  const stage = makeStage();
  const a = new ShuCharacterAdapter({ manifest: bundledManifest(), stage });
  await a.initialize({ ...host, reducedMotion: () => opts.reducedMotion === true });
  return { a, stage };
}

// ---------------------------------------------------------------------------
// manifest／協商
// ---------------------------------------------------------------------------

describe("ShuCharacterAdapter：manifest 與協商", () => {
  it("bundled manifest：entrypoint shu-rig、§12 完整能力集、20 個 intent、3 個 palette variants", () => {
    const a = new ShuCharacterAdapter({ manifest: bundledManifest(), stage: makeStage() });
    expect(a.manifest.characterId).toBe("shu-maid");
    expect(a.manifest.entrypoint).toEqual({ kind: "builtin", id: "shu-rig" });
    expect(a.manifest.capabilities).toEqual(shuRigCapabilities().capabilities);
    expect(a.manifest.inputCapabilities).toEqual(shuRigCapabilities().inputCapabilities);
    expect(a.manifest.intents).toHaveLength(20);
    expect(a.manifest.variants.map((v) => v.id)).toEqual(["maid-classic", "maid-dusk", "maid-sakura"]);
    // 36 個正式表情全部宣告在 states 裡（表情庫沒有退化）。
    for (const id of OFFICIAL_36) expect(a.manifest.states, id).toContain(id);
  });

  it("沒有 bundled manifest：由 legacy character-rig 2.0 pack 遷移（migratePackToManifest）", () => {
    const a = new ShuCharacterAdapter({ legacyRig: legacyRig, stage: makeStage() });
    expect(a.manifest.characterId).toBe("shu-maid");
    expect(a.manifest.entrypoint).toEqual({ kind: "builtin", id: "shu-rig" });
    expect(a.manifest.displayName["zh-TW"]).toBeTruthy();
    // 完全沒給也能建（內建預設 legacy rig）；非 shu-rig 的 manifest 拒絕。
    expect(new ShuCharacterAdapter({ stage: makeStage() }).manifest.characterId).toBe("shu-maid");
    const text = { ...bundledManifest(), entrypoint: { kind: "builtin" as const, id: "text" } };
    expect(() => new ShuCharacterAdapter({ manifest: text, stage: makeStage() })).toThrow(/shu-rig/);
  });

  it("透過 Gateway 協商：20 個 intent 全部 exact via visual.expression，12 個 semantic channel 全接受", async () => {
    const gw = new CharacterGateway({ now: () => clock.now, onSystemText: () => {} });
    const { a } = await standalone();
    const { negotiated } = await gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    for (const intent of CHARACTER_INTENTS) {
      expect(negotiated.resolutions[intent].resolution, intent).toBe("exact");
      expect(negotiated.resolutions[intent].via, intent).toBe(intent === "idle" ? "visual.presence" : "visual.expression");
    }
    expect(negotiated.acceptedChannels).toHaveLength(12);
    expect(negotiated.ignoredChannels).toEqual([]);
    expect(negotiated.reducedMotion).toBe(false);
    expect(Object.keys(negotiated.capabilities)).toContain("gameplay.toys");
  });

  it("Reduced Motion 由 hello 協商：visual.expression（static）→ reduced；adapter 與舞台同步；重新協商可回 exact", async () => {
    let reduced = true;
    const gw = new CharacterGateway({ now: () => clock.now, onSystemText: () => {}, reducedMotion: () => reduced });
    const { a } = await standalone({ reducedMotion: true });
    const { negotiated } = await gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    expect(negotiated.reducedMotion).toBe(true);
    expect(a.isReducedMotion()).toBe(true);
    for (const intent of CHARACTER_INTENTS) {
      expect(negotiated.resolutions[intent].resolution, intent).toBe("reduced");
    }
    // 移動類能力在 reduced motion 下被停用（不假裝還能走動）。
    expect(negotiated.capabilities["visual.locomotion"]).toBeUndefined();
    expect(negotiated.capabilities["gameplay.toys"]).toBeUndefined();
    reduced = false;
    const again = gw.renegotiate("a");
    expect(again.reducedMotion).toBe(false);
    expect(again.resolutions.work.resolution).toBe("exact");
    expect(a.isReducedMotion()).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// intent → 表情表（shuTables）
// ---------------------------------------------------------------------------

describe("shuTables：20 個 intent → 表情（claimed ≠ verified、安全表情固定）", () => {
  const expected: Array<[CharacterIntent, TruthState, string | undefined, string]> = [
    ["idle", "none", undefined, "idle"],
    ["notice", "none", undefined, "notice"],
    ["notice", "none", "curious", "curious"],
    ["notice", "none", "listening", "listening"],
    ["notice", "none", "device-offline", "device-lost"],
    ["notice", "none", "look-at-confirmation", "question"],
    ["notice", "none", "wait-attention", "waiting"],
    ["acknowledge", "working", undefined, "ack-nod"],
    ["think", "working", undefined, "thinking"],
    ["think", "none", "wait-attention", "waiting"],
    ["work", "working", undefined, "working"],
    ["work", "working", "operate-tool", "operate-tool"],
    ["wait", "queued", undefined, "waiting"],
    ["wait", "queued", "codex", "wait-codex"],
    ["wait", "queued", "claude-code", "wait-claude"],
    ["ask", "waiting-input", undefined, "ask"],
    ["request-consent", "waiting-consent", undefined, "ask"],
    ["blocked", "blocked", undefined, "blocked"],
    ["unknown", "unknown", undefined, "unknown"],
    ["claim-completed", "claimed", undefined, "success-claimed"],
    ["verified-success", "verified", undefined, "success-verified"],
    ["verified-success", "claimed", undefined, "success-claimed"],
    ["failed", "failed", undefined, "failed"],
    ["cancelled", "cancelled", undefined, "idle"],
    ["offline", "offline", undefined, "offline"],
    ["emergency", "emergency", undefined, "emergency"],
    ["greet", "none", undefined, "device-hello"],
    ["play", "none", undefined, "play-chase"],
    ["play", "none", "carry", "play-carry"],
    ["play", "none", "sneak", "sneak-closer"],
    ["rest", "none", undefined, "quiet"],
    ["sleep", "none", undefined, "doze"],
    ["sleep", "none", "lie-flat", "lie-flat"],
  ];

  it.each(expected)("%s／%s／variant=%s → %s", (intent, truth, variant, expression) => {
    const p = shuExpressionPlan(intent, truth, variant);
    expect(p.expression).toBe(expression);
    expect(EXPRESSIONS[p.expression], p.expression).toBeTruthy();
  });

  it("truthState 表情只由 Runtime 的 truthState 決定；variant 不能把安全表情換掉、也不能升級成綠勾", () => {
    expect(shuExpressionPlan("verified-success", "claimed", "success-verified").expression).toBe("success-claimed");
    expect(shuExpressionPlan("claim-completed", "claimed", "success-verified").expression).toBe("success-claimed");
    for (const intent of ["blocked", "unknown", "failed", "emergency", "offline", "ask", "request-consent"] as const) {
      expect(shuExpressionPlan(intent, "none", "praised").expression, intent).toBe(shuExpressionPlan(intent, "none").expression);
      expect(resolveExpression(shuExpressionPlan(intent, "none").expression)?.truthState, intent).toBe(true);
    }
    // 非安全 intent 永遠不會落到 truthState 表情。
    for (const intent of ["idle", "notice", "acknowledge", "think", "work", "greet", "play", "rest", "sleep"] as const) {
      const p = shuExpressionPlan(intent, "none", "success-verified");
      expect(resolveExpression(p.expression)?.truthState ?? false, intent).toBe(false);
    }
  });

  it("emergency／offline 是機器基態、idle／cancelled 是清場，其餘是 transient", () => {
    expect(shuExpressionPlan("emergency", "emergency").mode).toBe("base");
    expect(shuExpressionPlan("offline", "offline").mode).toBe("base");
    expect(shuExpressionPlan("idle", "none").mode).toBe("clear");
    expect(shuExpressionPlan("cancelled", "cancelled").mode).toBe("clear");
    expect(shuExpressionPlan("work", "working").mode).toBe("transient");
  });

  it("ambient／反應／落地／微動作表全部非 truthState 且存在", () => {
    for (const v of SHU_AMBIENT_VARIANTS) expect(isShuPlayable(v.expression), v.expression).toBe(true);
    for (const e of Object.values(SHU_REACTIONS).flat()) expect(isShuPlayable(e), e).toBe(true);
    for (const l of Object.values(SHU_LANDING)) expect(isShuPlayable(l.expression), l.expression).toBe(true);
    for (const m of SHU_MICRO_ACTIONS) expect(resolveExpression(m.animation), m.animation).toBeTruthy();
    for (const truth of ["success-verified", "success-claimed", "blocked", "failed", "unknown", "emergency", "offline", "ask"]) {
      expect(isShuPlayable(truth), truth).toBe(false);
    }
    expect(isShuPlayable("moonwalk")).toBe(false);
    expect(shuNaturalDurationMs("doze")).toBeGreaterThanOrEqual(600);
    expect(shuNaturalDurationMs("nope")).toBe(3000);
  });
});

// ---------------------------------------------------------------------------
// 回執（時間軸驅動）
// ---------------------------------------------------------------------------

describe("ShuCharacterAdapter：回執誠實", () => {
  it("claim-completed 只點頭（success [0,1]），verified-success+verified 才完整成功；舞台真的收到", async () => {
    const { a, stage } = await standalone();
    const spy = vi.spyOn(stage, "setAnimation");
    const receipts: AdapterReceipt[] = [];
    const sink = (r: AdapterReceipt) => receipts.push(r);
    a.perform(env("claim-completed", "claimed"), sink, { resolution: "exact", via: "visual.expression" });
    expect(spy.mock.calls.slice(-1)[0]).toEqual(["success", [0, 1]]);
    expect(a.lastExpressionPlan()?.expression).toBe("success-claimed");
    a.perform(env("verified-success", "verified"), sink, { resolution: "exact", via: "visual.expression" });
    expect(spy.mock.calls.slice(-1)[0]).toEqual(["success", undefined]);
    expect(a.lastExpressionPlan()?.expression).toBe("success-verified");
    a.perform(env("verified-success", "claimed"), sink, { resolution: "exact", via: "visual.expression" });
    expect(spy.mock.calls.slice(-1)[0]).toEqual(["success", [0, 1]]);
  });

  it("accepted → started → completed：transient 到期才 completed；durationHint 優先", async () => {
    const { a } = await standalone();
    const receipts: AdapterReceipt[] = [];
    const sink = (r: AdapterReceipt) => receipts.push(r);
    const e = env("work", "working");
    a.perform(e, sink, { resolution: "exact", via: "visual.expression" });
    expect(receipts.map((r) => r.status)).toEqual(["accepted", "started"]);
    expect(receipts[1].resolution).toBe("exact");
    const st = a.localMachineState()!;
    expect(st.transient?.kind).toBe("acting");
    const until = st.transient!.untilMs;
    a.tick(until - 1);
    expect(receipts).toHaveLength(2);
    a.tick(until);
    expect(receipts.map((r) => r.status)).toEqual(["accepted", "started", "completed"]);
    // 上一則 acting（40）到期後才能換 thinking（30）：時間往前走，讓舞台真的空了。
    const base = clock.now;
    clock.now = until + 1;
    receipts.length = 0;
    const hinted = env("think", "working", { durationHint: { ms: 1000 } });
    a.perform(hinted, sink);
    expect(a.localMachineState()!.transient!.untilMs).toBe(clock.now + 1000);
    a.tick(clock.now + 999);
    expect(receipts.map((r) => r.status)).toEqual(["accepted", "started"]);
    a.tick(clock.now + 1000);
    expect(receipts.slice(-1)[0]?.status).toBe("completed");
    clock.now = base;
  });

  it("emergency／offline 是基態：started 後 enter 段播完即 completed，但基態持續（cancel 也不解除）", async () => {
    const { a } = await standalone();
    const receipts: AdapterReceipt[] = [];
    const sink = (r: AdapterReceipt) => receipts.push(r);
    const e = env("emergency", "emergency", { priority: 100 });
    a.perform(e, sink);
    expect(receipts.map((r) => r.status)).toEqual(["accepted", "started"]);
    expect(a.localMachineState()!.base).toBe("emergency");
    a.tick(clock.now + 5_000);
    expect(receipts.slice(-1)[0]?.status).toBe("completed");
    expect(a.localMachineState()!.base).toBe("emergency");
    // 緊急中送非安全 intent：machine 不讓它上台 → 誠實 cancelled{preempted}，沒有 started。
    receipts.length = 0;
    a.perform(env("play"), sink);
    expect(receipts.map((r) => r.status)).toEqual(["accepted", "cancelled"]);
    expect(receipts[1].reason).toBe("preempted");
  });

  it("本機混音器留住更高優先的互動（clicked 55 > performing 25）→ 沒上台就說 cancelled{preempted}", async () => {
    let st: MachineState = { ...initial, base: "idle" };
    const mixer: MixerPort = {
      apply: (ev: MachineEvent) => {
        st = reduce(st, ev, clock.now);
        return st;
      },
      state: () => st,
    };
    const a = new ShuCharacterAdapter({ manifest: bundledManifest(), stage: makeStage(), mixer });
    await a.initialize(host);
    mixer.apply({ type: "transient", kind: "clicked" });
    const receipts: AdapterReceipt[] = [];
    a.perform(env("notice"), (r) => receipts.push(r));
    expect(receipts.map((r) => r.status)).toEqual(["accepted", "cancelled"]);
    expect(receipts[1].reason).toBe("preempted");
    // 一旦 clicked 過期，同樣的 notice 就能上台。
    clock.now += 1_000;
    receipts.length = 0;
    a.perform(env("notice"), (r) => receipts.push(r));
    expect(receipts.map((r) => r.status)).toEqual(["accepted", "started"]);
    expect(st.transient).toMatchObject({ kind: "performing", animation: "notice" });
    clock.now -= 1_000;
  });

  it("進行中被本機事件擠掉 → tick 時回 cancelled{preempted}，不會假裝 completed", async () => {
    let st: MachineState = { ...initial, base: "idle" };
    const mixer: MixerPort = {
      apply: (ev: MachineEvent) => {
        st = reduce(st, ev, clock.now);
        return st;
      },
      state: () => st,
    };
    const a = new ShuCharacterAdapter({ manifest: bundledManifest(), stage: makeStage(), mixer });
    await a.initialize(host);
    const receipts: AdapterReceipt[] = [];
    a.perform(env("play"), (r) => receipts.push(r));
    expect(receipts.map((r) => r.status)).toEqual(["accepted", "started"]);
    expect(st.transient).toMatchObject({ kind: "performing", animation: "play-chase" });
    // host 的 machine 收到安全事件（例如舊路徑的 plan.blocked）：遊玩被 blocked（90）擠掉。
    mixer.apply({ type: "transient", kind: "blocked" });
    expect(st.transient?.kind).toBe("blocked");
    a.tick(clock.now + 100);
    expect(receipts.slice(-1)[0]).toMatchObject({ status: "cancelled", reason: "preempted" });
    // 之後 tick 不再重複發回執，也不會把它補成 completed。
    a.tick(clock.now + 60_000);
    expect(receipts).toHaveLength(3);
  });

  it("cancel 冪等：只對進行中的 messageId 有效、回待機、第二次不再發回執；dispose 後 failed", async () => {
    const { a } = await standalone();
    const receipts: AdapterReceipt[] = [];
    const e = env("work", "working");
    a.perform(e, (r) => receipts.push(r));
    a.cancel("someone-else");
    expect(receipts).toHaveLength(2);
    a.cancel(e.messageId);
    expect(receipts.map((r) => r.status)).toEqual(["accepted", "started", "cancelled"]);
    expect(receipts[2].reason).toBe("cancel");
    expect(a.localMachineState()!.transient).toBeNull();
    a.cancel(e.messageId);
    expect(receipts).toHaveLength(3);
    a.dispose();
    a.perform(env("notice"), (r) => receipts.push(r));
    expect(receipts.slice(-1)[0]?.status).toBe("failed");
  });

  it("negotiated unsupported → unsupported；新命令取代舊命令 → 舊的 cancelled{replaced}", async () => {
    const { a } = await standalone();
    const receipts: Array<AdapterReceipt & { id: string }> = [];
    const sink = (r: AdapterReceipt) => receipts.push({ ...r, id: r.messageId });
    a.perform(env("play"), sink, { resolution: "unsupported" });
    expect(receipts.map((r) => r.status)).toEqual(["unsupported"]);
    receipts.length = 0;
    const first = env("work", "working");
    a.perform(first, sink);
    a.perform(env("think", "working"), sink);
    expect(receipts.find((r) => r.id === first.messageId && r.status === "cancelled")?.reason).toBe("replaced");
  });
});

// ---------------------------------------------------------------------------
// 透過 Gateway：安全搶占、suspend、reconfigure、gameplay
// ---------------------------------------------------------------------------

describe("ShuCharacterAdapter：透過 Gateway", () => {
  async function viaGateway(opts: { reducedMotion?: boolean } = {}) {
    const receipts: CommandReceipt[] = [];
    const gw = new CharacterGateway({
      now: () => clock.now,
      onSystemText: () => {},
      onReceipt: (r) => receipts.push(r),
      reducedMotion: () => opts.reducedMotion === true,
    });
    const { a, stage } = await standalone(opts);
    await gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    return { gw, a, stage, receipts, statuses: (id: string) => receipts.filter((r) => r.messageId === id).map((r) => r.status) };
  }

  it("emergency（floor 100）搶占進行中的 play：play cancelled{preempted}、emergency started、機器基態 emergency", async () => {
    const { gw, a, statuses, receipts } = await viaGateway();
    const play = env("play");
    gw.dispatch(play);
    expect(statuses(play.messageId)).toEqual(["accepted", "started"]);
    const em = env("emergency", "emergency");
    gw.dispatch(em);
    expect(statuses(play.messageId)).toEqual(["accepted", "started", "cancelled"]);
    expect(receipts.find((r) => r.messageId === play.messageId && r.status === "cancelled")?.reason).toBe("preempted");
    expect(statuses(em.messageId)).toEqual(["accepted", "started"]);
    expect(receipts.find((r) => r.messageId === em.messageId && r.status === "accepted")?.resolution).toBe("exact");
    expect(a.localMachineState()!.base).toBe("emergency");
    // 緊急後 sweep：emergency 演完 completed；不會被記成 uncertain。
    clock.now += 2_000;
    gw.sweep(clock.now);
    expect(statuses(em.messageId)).toEqual(["accepted", "started", "completed"]);
    clock.now -= 2_000;
  });

  it("blocked／unknown／failed／request-consent 都能搶占 play；AI 來源不能點播它們", async () => {
    const { gw, statuses, receipts } = await viaGateway();
    for (const intent of ["blocked", "unknown", "failed", "request-consent"] as const) {
      const play = env("play");
      gw.dispatch(play);
      const safety = env(intent, intent === "request-consent" ? "waiting-consent" : intent);
      gw.dispatch(safety);
      expect(statuses(play.messageId), intent).toContain("cancelled");
      expect(receipts.find((r) => r.messageId === play.messageId && r.status === "cancelled")?.reason, intent).toBe("preempted");
      expect(statuses(safety.messageId), intent).toEqual(["accepted", "started"]);
      gw.cancel(safety.messageId);
    }
    expect(gw.dispatch(env("blocked", "blocked"), "ai").status).toBe("unsupported");
    expect(gw.dispatch(env("verified-success", "verified"), "ai").status).toBe("unsupported");
  });

  it("suspend／hide 停掉舞台迴圈（rAF／物理），resume／show 恢復；dispose 後注入的舞台只暫停不銷毀", async () => {
    const { gw, stage } = await viaGateway();
    expect(stage.isPaused()).toBe(false);
    expect(gw.suspend("a")).toBe(true);
    expect(stage.isPaused()).toBe(true);
    expect(gw.resume("a")).toBe(true);
    expect(stage.isPaused()).toBe(false);
    expect(gw.hide("a")).toBe(true);
    expect(stage.isPaused()).toBe(true);
    expect(gw.show("a")).toBe(true);
    expect(stage.isPaused()).toBe(false);
    // 隱藏中 resume 不會偷跑（仍看不見）。
    gw.hide("a");
    gw.suspend("a");
    gw.resume("a");
    expect(stage.isPaused()).toBe(true);
    gw.show("a");
    expect(stage.isPaused()).toBe(false);
    const destroy = vi.spyOn(stage, "destroy");
    gw.disposeInstance("a");
    expect(destroy).not.toHaveBeenCalled();
    expect(stage.isPaused()).toBe(true);
  });

  it("cancel 經 Gateway 冪等：pending → cancelled{host}；已終結 → alreadyTerminal、adapter 不再收到 cancel", async () => {
    const { gw, a, statuses } = await viaGateway();
    const cancelSpy = vi.spyOn(a, "cancel");
    const e = env("work", "working");
    gw.dispatch(e);
    const first = gw.cancel(e.messageId);
    expect(first).toMatchObject({ status: "cancelled", reason: "host" });
    expect(cancelSpy).toHaveBeenCalledTimes(1);
    const second = gw.cancel(e.messageId);
    expect(second.alreadyTerminal).toBe(true);
    expect(cancelSpy).toHaveBeenCalledTimes(1);
    expect(statuses(e.messageId)).toEqual(["accepted", "started", "cancelled"]);
    expect(a.localMachineState()!.transient).toBeNull();
  });

  it("reconfigure 套用名字／場景／開關／tuning／使魔／配色；壞值不會讓 adapter 崩潰", async () => {
    const { gw, a, stage } = await viaGateway();
    const name = vi.spyOn(stage, "setCharName");
    const scene = vi.spyOn(stage, "setScene");
    const toggles = vi.spyOn(stage, "setToggles");
    const familiars = vi.spyOn(stage, "setFamiliars");
    expect(
      gw.reconfigure("a", {
        name: "阿樞",
        scene: "night",
        play: false,
        cursorPlay: true,
        familiars: [{ id: "f1", name: "小白", palette: "maid-dusk" }, { bogus: true }],
        palette: "maid-sakura",
        tuning: { speedScale: 0.7 },
        reducedMotion: true,
      })
    ).toBe(true);
    expect(name).toHaveBeenCalledWith("阿樞");
    expect(scene).toHaveBeenCalledWith("night");
    expect(toggles).toHaveBeenCalledWith({ play: false, cursorPlay: true });
    expect(familiars).toHaveBeenCalledWith([{ id: "f1", name: "小白", palette: "maid-dusk" }]);
    expect(stage.currentPalette()).toBe("maid-sakura");
    expect(a.isReducedMotion()).toBe(true);
    expect(a.gameplay.scene.current()).toBe("night");
    expect(a.gameplay.familiars.list()).toEqual(["f1"]);
    expect(gw.reconfigure("a", { palette: "neon", scene: "moon" })).toBe(true);
    expect(stage.currentPalette()).toBe("maid-sakura");
    expect(a.gameplay.scene.current()).toBe("none");
    expect(gw.getInstance("a")?.state).not.toBe("crashed");
  });

  it("gameplay 擴充：丟玩具會發 toy-thrown 輸入事件（經 Gateway 正規化）、使魔召喚有上限、場景白名單、指標路由", async () => {
    const inputs: string[] = [];
    const gw = new CharacterGateway({ now: () => clock.now, onSystemText: () => {}, onInput: (e) => inputs.push(e.kind) });
    const { a, stage } = await standalone();
    await gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    expect(a.gameplay.spawnToy("yarn")).toBe("yarn");
    expect(a.gameplay.spawnToy("laser")).toBeNull();
    expect(stage.toyCount()).toBe(1);
    expect(inputs).toEqual(["character.toy-thrown"]);
    a.gameplay.clearToys();
    expect(stage.toyCount()).toBe(0);
    expect(a.gameplay.familiars.summon("f1")).toBe(true);
    expect(a.gameplay.familiars.summon("f2")).toBe(true);
    expect(a.gameplay.familiars.summon("f3")).toBe(true);
    expect(a.gameplay.familiars.summon("f4")).toBe(false);
    expect(a.gameplay.familiars.dismiss("f2")).toBe(true);
    expect(a.gameplay.familiars.list()).toEqual(["f1", "f3"]);
    expect(a.gameplay.scene.set("desk")).toBe(true);
    expect(a.gameplay.scene.set("../evil")).toBe(false);
    expect(a.gameplay.scene.current()).toBe("desk");
    expect(a.gameplay.rollCall()).toBe(true);
    expect(a.rollCallNow(null)[0].name).toBe("小樞");
    // 指標路由：空白處 down 回 false（沒抓到玩具）；cancel 安全。
    expect(a.gameplay.routePointer({ type: "down", x: 1, y: 1 })).toBe(false);
    expect(a.gameplay.routePointer({ type: "move", x: 2, y: 2 })).toBe(false);
    expect(a.gameplay.routePointer({ type: "cancel", x: 0, y: 0 })).toBe(false);
  });
});
