// CPP §12／§13：text（最小文字角色）與 sprite（舊 pack 相容層）Reference Adapter。
// 文字角色誠實呈現所有安全 intent、綠勾只給 verified；sprite adapter claimed 與 verified 演法不同、
// 安全 intent 絕不落到 success；兩者回執順序合法、cancel／dispose 乾淨。

import { afterEach, describe, expect, it, vi } from "vitest";
import shuStandard from "../../public/packs/shu-standard/manifest.json";
import shuLively from "../../public/packs/shu-lively/manifest.json";
import { FIXED_SAFETY_LINES } from "../companion/packs";
import { SpriteRenderer, type PackManifest, type RendererBackend } from "../companion/renderer";
import type { AdapterHost, AdapterReceipt } from "../character/adapter";
import { SpriteCharacterAdapter } from "../character/adapters/sprite";
import { TextCharacterAdapter, buildTextCharacterManifest, type RenderedLine } from "../character/adapters/text";
import { CharacterGateway, type SystemTextMessage } from "../character/gateway";
import {
  CHARACTER_INTENTS,
  CharacterIntent,
  CommandReceipt,
  IntentEnvelope,
  PROTOCOL_VERSION,
  TruthState,
} from "../character/protocol";

const clock = { now: 1_700_000_000_000 };
const host: AdapterHost = {
  now: () => clock.now,
  reducedMotion: () => false,
  locale: "zh-TW",
  log: () => {},
};

let seq = 0;
function env(intent: CharacterIntent, truthState: TruthState = "none", over: Partial<IntentEnvelope> = {}): IntentEnvelope {
  seq += 1;
  return {
    protocolVersion: PROTOCOL_VERSION,
    messageId: `t${seq}`,
    characterInstanceId: "a",
    timestamp: "2026-09-02T00:00:00.000Z",
    intent,
    truthState,
    priority: 10,
    interruptPolicy: "preempt",
    resumePolicy: "none",
    privacyClass: "internal",
    ...over,
  };
}

class FakeRenderer implements RendererBackend {
  calls: Array<{ name: string; slice?: [number, number] }> = [];
  reduced = false;
  destroyed = 0;
  paused = 0;
  resumed = 0;
  setAnimation(name: string, frameSlice?: [number, number]) {
    this.calls.push({ name, slice: frameSlice });
  }
  setReducedMotion(on: boolean) {
    this.reduced = on;
  }
  setMicroMotion() {}
  destroy() {
    this.destroyed += 1;
  }
  pause() {
    this.paused += 1;
  }
  resume() {
    this.resumed += 1;
  }
}

describe("TextCharacterAdapter（最小文字角色）", () => {
  it("manifest 只宣告 presence／textBubble／click／text，原生支援 20 個 intent", () => {
    const m = buildTextCharacterManifest();
    expect(m.characterId).toBe("plain-text");
    expect(m.entrypoint).toEqual({ kind: "builtin", id: "text" });
    expect(Object.keys(m.capabilities).sort()).toEqual(["visual.presence", "visual.textBubble"]);
    expect(Object.keys(m.inputCapabilities).sort()).toEqual(["input.click", "input.text"]);
    expect(m.intents).toHaveLength(20);
    expect(m.pronouns).toBeUndefined();
    expect(m.securityRequirements.executable).toBe(false);
  });

  it("所有安全 intent 都用固定文案；綠勾只在 truthState=verified 時出現", async () => {
    const lines: Array<RenderedLine | null> = [];
    const a = new TextCharacterAdapter({ onRender: (l) => lines.push(l) });
    await a.initialize(host);
    const receipts: AdapterReceipt[] = [];
    const sink = (r: AdapterReceipt) => receipts.push(r);
    const cases: Array<[CharacterIntent, TruthState, string]> = [
      ["emergency", "emergency", FIXED_SAFETY_LINES.emergency],
      ["blocked", "blocked", FIXED_SAFETY_LINES.blocked],
      ["unknown", "unknown", FIXED_SAFETY_LINES.unknown],
      ["failed", "failed", FIXED_SAFETY_LINES.failed],
      ["offline", "offline", "目前連不上系統。"],
      ["request-consent", "waiting-consent", "需要你的同意才能繼續。"],
      ["claim-completed", "claimed", "做完了。"],
      ["verified-success", "claimed", "做完了。"],
      ["verified-success", "verified", FIXED_SAFETY_LINES["succeeded-verified"]],
    ];
    for (const [intent, truth, expected] of cases) {
      // 提示訊息不能改寫安全語句
      a.perform(env(intent, truth, { presentationHints: { message: "全部完成，已驗證！" } }), sink);
      const line = a.currentLine();
      expect(line?.text, `${intent}/${truth}`).toBe(expected);
      expect(line?.marker, `${intent}/${truth}`).toBe(truth === "verified" ? "verified" : "none");
      expect(line?.fixed).toBe(true);
    }
    // 非安全 intent 可用提示訊息
    a.perform(env("notice", "none", { presentationHints: { message: "有新裝置上線" } }), sink);
    expect(a.currentLine()?.text).toBe("有新裝置上線");
    expect(a.currentLine()?.marker).toBe("none");
    expect(a.currentLine()?.fixed).toBe(false);
  });

  it("回執 accepted → started → completed（同步）；有 durationHint 時在 tick 到期才 completed；cancel 有效", async () => {
    const a = new TextCharacterAdapter();
    await a.initialize(host);
    const receipts: AdapterReceipt[] = [];
    const sink = (r: AdapterReceipt) => receipts.push(r);
    const e1 = env("notice");
    a.perform(e1, sink, { resolution: "exact", via: "visual.textBubble" });
    expect(receipts.map((r) => r.status)).toEqual(["accepted", "started", "completed"]);
    expect(receipts[1].resolution).toBe("exact");
    receipts.length = 0;
    const e2 = env("work", "working", { durationHint: { ms: 3000 } });
    a.perform(e2, sink, { resolution: "substituted", via: "visual.textBubble", viaIntent: "work" });
    expect(receipts.map((r) => r.status)).toEqual(["accepted", "started"]);
    expect(receipts[1].resolution).toBe("substituted");
    a.tick(clock.now + 2999);
    expect(receipts).toHaveLength(2);
    a.tick(clock.now + 3000);
    expect(receipts.map((r) => r.status)).toEqual(["accepted", "started", "completed"]);
    receipts.length = 0;
    const e3 = env("think", "working", { durationHint: { ms: 3000 } });
    a.perform(e3, sink);
    a.cancel(e3.messageId);
    expect(receipts.map((r) => r.status)).toEqual(["accepted", "started", "cancelled"]);
    expect(a.currentLine()).toBeNull();
    a.cancel(e3.messageId); // 冪等：不再多發
    expect(receipts).toHaveLength(3);
  });

  it("negotiated 為 unsupported → unsupported 回執；dispose 後 failed", async () => {
    const a = new TextCharacterAdapter();
    await a.initialize(host);
    const receipts: AdapterReceipt[] = [];
    a.perform(env("play"), (r) => receipts.push(r), { resolution: "unsupported" });
    expect(receipts.map((r) => r.status)).toEqual(["unsupported"]);
    a.dispose();
    a.perform(env("notice"), (r) => receipts.push(r));
    expect(receipts.slice(-1)[0]?.status).toBe("failed");
  });

  it("DOM 模式：掛在 container、點擊發出 input.click、綠勾只給 verified、dispose 清乾淨", async () => {
    const container = document.createElement("div");
    const a = new TextCharacterAdapter({ container });
    await a.initialize(host);
    const events: string[] = [];
    const off = a.onInput((e) => events.push(e.kind));
    const el = container.querySelector("[data-cpp-text-character]") as HTMLElement;
    expect(el).not.toBeNull();
    el.click();
    expect(events).toEqual(["character.clicked"]);
    const sink = () => {};
    a.perform(env("claim-completed", "claimed"), sink);
    const line = () => container.querySelector("[data-cpp-line]") as HTMLElement;
    expect(line().textContent).toBe("做完了。");
    expect(line().getAttribute("data-marker")).toBe("none");
    a.perform(env("verified-success", "verified"), sink);
    expect(line().textContent).toBe("✓ 做完了，也確認過結果。");
    expect(line().getAttribute("data-marker")).toBe("verified");
    a.hide();
    expect(line().textContent).toBe("");
    a.show();
    expect(line().textContent).toContain("✓");
    a.submitText("hello");
    expect(events).toEqual(["character.clicked", "character.text-submitted"]);
    off();
    a.dispose();
    expect(container.querySelector("[data-cpp-text-character]")).toBeNull();
    a.submitText("after dispose");
    expect(events).toHaveLength(2);
  });

  it("透過 Gateway：19 個 intent exact 並完成、play 誠實 unsupported；forged verified 只能來自 Runtime", async () => {
    const lines: Array<RenderedLine | null> = [];
    const receipts: CommandReceipt[] = [];
    const systemTexts: SystemTextMessage[] = [];
    const gw = new CharacterGateway({ now: () => clock.now, onSystemText: (m) => systemTexts.push(m), onReceipt: (r) => receipts.push(r) });
    const a = new TextCharacterAdapter({ onRender: (l) => lines.push(l) });
    const { negotiated } = await gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    for (const intent of CHARACTER_INTENTS) {
      // §3.4：play 只能由 gameplay.toys／visual.locomotion／visual.pose／visual.expression 承載，
      // 純文字角色一個都沒有、也沒宣告 fallback → 誠實回 unsupported（Runtime 權威端同樣結果，
      // 兩邊必須一致），不假裝演得出來。play 不是安全 intent，所以沒有 system.text 兜底。
      const expected = intent === "play" ? "unsupported" : "exact";
      expect(negotiated.resolutions[intent].resolution, intent).toBe(expected);
      const e = env(intent, intent === "verified-success" ? "verified" : "none");
      gw.dispatch(e);
      expect(receipts.filter((r) => r.messageId === e.messageId).map((r) => r.status), intent).toEqual(
        intent === "play" ? ["unsupported"] : ["accepted", "started", "completed"]
      );
    }
    expect(systemTexts).toHaveLength(0);
    // AI 來源不能讓文字角色打綠勾
    const forged = env("verified-success", "verified");
    expect(gw.dispatch(forged, "ai").status).toBe("unsupported");
    expect(lines.filter((l) => l?.marker === "verified")).toHaveLength(1);
  });
});

describe("SpriteCharacterAdapter（舊 pack 相容層）", () => {
  const packV1 = shuStandard as unknown as PackManifest;
  const packV2 = shuLively as unknown as PackManifest;

  async function make(pack: PackManifest) {
    const renderer = new FakeRenderer();
    const a = new SpriteCharacterAdapter({ pack, assetBase: `/packs/${pack.id}`, renderer });
    await a.initialize(host);
    return { a, renderer };
  }

  it("manifest 只宣告 sheet 真的有的東西（v1 無 gaze、v1.1 有）", () => {
    const v1 = new SpriteCharacterAdapter({ pack: packV1, assetBase: "/packs/shu-standard", renderer: new FakeRenderer() });
    expect(v1.manifest.entrypoint).toEqual({ kind: "builtin", id: "sprite" });
    expect(Object.keys(v1.manifest.capabilities)).toEqual(["visual.presence", "visual.expression"]);
    expect(v1.manifest.capabilities["visual.expression"].variants).toEqual(Object.keys(packV1.animations));
    expect(v1.manifest.intents).not.toContain("failed");
    const v2 = new SpriteCharacterAdapter({ pack: packV2, assetBase: "/packs/shu-lively", renderer: new FakeRenderer() });
    expect(Object.keys(v2.manifest.capabilities)).toContain("visual.gaze");
    expect(v2.manifest.intents).toContain("failed");
    expect(() => new SpriteCharacterAdapter({ pack: { kind: "character-pack" } as PackManifest, assetBase: "/x" })).toThrow();
  });

  it("claimed 只點頭（frameSlice [0,1]）、verified 才完整 success；安全 intent 絕不 success", async () => {
    const { a, renderer } = await make(packV1);
    const receipts: AdapterReceipt[] = [];
    const sink = (r: AdapterReceipt) => receipts.push(r);
    a.perform(env("claim-completed", "claimed"), sink, { resolution: "exact", via: "visual.expression" });
    expect(renderer.calls.slice(-1)[0]).toEqual({ name: "success", slice: [0, 1] });
    a.perform(env("verified-success", "verified"), sink, { resolution: "exact", via: "visual.expression" });
    expect(renderer.calls.slice(-1)[0]).toEqual({ name: "success", slice: undefined });
    // 就算 envelope 說 verified-success，truthState 不是 verified 就只點頭
    a.perform(env("verified-success", "claimed"), sink, { resolution: "exact", via: "visual.expression" });
    expect(renderer.calls.slice(-1)[0]).toEqual({ name: "success", slice: [0, 1] });
    for (const intent of ["emergency", "offline", "blocked", "unknown", "failed"] as const) {
      a.perform(env(intent, intent === "failed" ? "failed" : "none", { presentationHints: { variant: "success" } }), sink);
      expect(renderer.calls.slice(-1)[0]?.name, intent).not.toBe("success");
    }
    expect(renderer.calls.slice(-1)[0]?.name).toBe("blocked"); // v1 failed → blocked
  });

  it("回執：accepted → started → completed（第一輪播完或 durationHint）；fallback 動畫回 substituted；cancel 回 idle", async () => {
    const { a, renderer } = await make(packV1);
    const receipts: AdapterReceipt[] = [];
    const sink = (r: AdapterReceipt) => receipts.push(r);
    const e = env("think", "working");
    a.perform(e, sink, { resolution: "exact", via: "visual.expression" });
    expect(receipts.map((r) => r.status)).toEqual(["accepted", "started"]);
    expect(receipts[1].resolution).toBe("exact");
    // thinking: 6 frames @ 5 fps = 1200 ms
    a.tick(clock.now + 1199);
    expect(receipts).toHaveLength(2);
    a.tick(clock.now + 1200);
    expect(receipts.map((r) => r.status)).toEqual(["accepted", "started", "completed"]);
    receipts.length = 0;
    const f = env("failed", "failed", { durationHint: { ms: 500 } });
    a.perform(f, sink, { resolution: "substituted", via: "visual.expression", viaIntent: "blocked" });
    expect(receipts[1].resolution).toBe("substituted");
    a.tick(clock.now + 500);
    expect(receipts.slice(-1)[0]?.status).toBe("completed");
    receipts.length = 0;
    const w = env("work", "working", { durationHint: { ms: 4000 } });
    a.perform(w, sink);
    a.cancel(w.messageId);
    expect(receipts.map((r) => r.status)).toEqual(["accepted", "started", "cancelled"]);
    expect(renderer.calls.slice(-1)[0]?.name).toBe("idle");
    expect(a.lastAnimation()?.animation).toBe("act");
    a.perform(env("play"), sink, { resolution: "unsupported" });
    expect(receipts.slice(-1)[0]?.status).toBe("unsupported");
    a.dispose();
    expect(renderer.destroyed).toBe(0); // 注入的 renderer 由 host 擁有
  });

  it("透過 Gateway：v1 pack 的 failed → substituted via blocked、sleep → rest；v2 全部 exact；reduced motion 轉給 renderer", async () => {
    const receipts: CommandReceipt[] = [];
    const gw = new CharacterGateway({ now: () => clock.now, onSystemText: () => {}, onReceipt: (r) => receipts.push(r), reducedMotion: () => true });
    const { a, renderer } = await make(packV1);
    const { negotiated } = await gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    expect(renderer.reduced).toBe(true);
    expect(negotiated.resolutions.failed).toEqual({ resolution: "reduced", via: "visual.expression", viaIntent: "blocked", variant: "blocked" });
    // rest 的動畫名是 quiet（≠ intent 名）→ 沒有 variant 標記
    expect(negotiated.resolutions.sleep).toEqual({ resolution: "reduced", via: "visual.expression", viaIntent: "rest" });
    const e = env("failed", "failed");
    gw.dispatch(e);
    expect(renderer.calls.slice(-1)[0]?.name).toBe("blocked");
    expect(receipts.find((r) => r.messageId === e.messageId && r.status === "started")?.resolution).toBe("reduced");
    const { a: b } = await make(packV2);
    const n2 = await gw.registerInstance(b, "familiar", { instanceId: "b" });
    for (const intent of CHARACTER_INTENTS) expect(n2.negotiated.resolutions[intent].resolution, intent).toBe("reduced");
    // 輸入接線由 host 負責：emitInput 進 Gateway 後正規化
    const inputs: string[] = [];
    const gw2 = new CharacterGateway({ now: () => clock.now, onSystemText: () => {}, onInput: (ev) => inputs.push(ev.kind) });
    const { a: c } = await make(packV2);
    await gw2.registerInstance(c, "primary-companion", { instanceId: "c" });
    c.emitInput({ kind: "character.clicked", payload: { x: 9, y: 9 } });
    expect(inputs).toEqual(["character.clicked"]);
  });
});

// ---------------------------------------------------------------------------
// SpriteRenderer 生命週期（對抗審查 perf-claims-019／020）：
// pause／destroy 真的停 rAF（含 destroy 早於圖片 onload）；Reduced Motion 靜態幀只畫一次。
// ---------------------------------------------------------------------------

/** SpriteRenderer 的最小宿主：假 rAF（手動 tick）、假 Image（手動 onload）、記錄 drawImage 的 2D ctx。 */
function spriteHost() {
  const queue = new Map<number, FrameRequestCallback>();
  let id = 0;
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
    id += 1;
    queue.set(id, cb);
    return id;
  });
  vi.stubGlobal("cancelAnimationFrame", (h: number) => {
    queue.delete(h);
  });
  const images: Array<{ onload: null | (() => void); src: string }> = [];
  vi.stubGlobal(
    "Image",
    class {
      onload: null | (() => void) = null;
      src = "";
      constructor() {
        images.push(this);
      }
    }
  );
  const draws: number[] = [];
  const noop = () => {};
  const ctx = {
    clearRect: noop,
    drawImage: () => {
      draws.push(1);
    },
    save: noop,
    restore: noop,
    beginPath: noop,
    arc: noop,
    fill: noop,
    moveTo: noop,
    lineTo: noop,
    stroke: noop,
    imageSmoothingEnabled: false,
    globalAlpha: 1,
    fillStyle: "",
    strokeStyle: "",
    lineWidth: 1,
  };
  const canvas = {
    width: 0,
    height: 0,
    style: {} as Record<string, string>,
    getContext: () => ctx,
  } as unknown as HTMLCanvasElement;
  return {
    canvas,
    draws,
    loadSheet: () => {
      for (const img of images) img.onload?.();
    },
    tick: (now: number) => {
      const cbs = [...queue.values()];
      queue.clear();
      for (const cb of cbs) cb(now);
    },
    pending: () => queue.size,
  };
}

describe("SpriteRenderer 生命週期（CPP §7：pause／destroy 真的釋放 rAF；Reduced Motion 只畫一次）", () => {
  const packV1 = shuStandard as unknown as PackManifest;
  afterEach(() => vi.unstubAllGlobals());

  it("destroy() 早於圖片 onload：之後不啟動迴圈、不排 rAF、不畫（沒有人能再取消的迴圈不得存在）", () => {
    const h = spriteHost();
    const r = new SpriteRenderer(h.canvas, packV1, "/packs/shu-standard/sheet.png");
    r.destroy();
    h.loadSheet();
    expect(h.pending()).toBe(0);
    h.tick(16);
    h.tick(32);
    expect(h.draws).toHaveLength(0);
  });

  it("pause() 取消 rAF、tick 不再畫；resume() 接續；destroy 後 resume 無效", () => {
    const h = spriteHost();
    const r = new SpriteRenderer(h.canvas, packV1, "/x/sheet.png");
    h.loadSheet();
    expect(h.pending()).toBe(1);
    expect(h.draws).toHaveLength(1); // onload 立刻畫第一幀
    r.pause();
    expect(r.isPaused()).toBe(true);
    expect(h.pending()).toBe(0);
    h.tick(performance.now() + 1000);
    h.tick(performance.now() + 2000);
    expect(h.draws).toHaveLength(1);
    r.resume();
    expect(h.pending()).toBe(1);
    h.tick(performance.now() + 10_000);
    expect(h.draws).toHaveLength(2);
    r.destroy();
    expect(h.pending()).toBe(0);
    r.resume();
    expect(h.pending()).toBe(0);
  });

  it("圖片載入時正暫停：onload 不偷跑；resume 才啟動", () => {
    const h = spriteHost();
    const r = new SpriteRenderer(h.canvas, packV1, "/x/sheet.png");
    r.pause();
    h.loadSheet();
    expect(h.pending()).toBe(0);
    expect(h.draws).toHaveLength(0);
    r.resume();
    expect(h.pending()).toBe(1);
    r.destroy();
    expect(h.pending()).toBe(0);
  });

  it("Reduced Motion：60 個 rAF 只畫一次靜態幀；換動畫才再畫一次；關掉後恢復依 fps 節流", () => {
    const h = spriteHost();
    const r = new SpriteRenderer(h.canvas, packV1, "/x/sheet.png");
    r.setReducedMotion(true);
    h.loadSheet();
    for (let i = 1; i <= 60; i++) h.tick(i * (1000 / 60));
    expect(h.pending()).toBe(1); // 迴圈還在（等狀態改變），只是不重畫
    expect(h.draws).toHaveLength(1);
    r.setAnimation("thinking");
    h.tick(1100);
    h.tick(1120);
    expect(h.draws).toHaveLength(2);
    r.setReducedMotion(false);
    const before = h.draws.length;
    for (let i = 1; i <= 60; i++) h.tick(2000 + i * (1000 / 60));
    const drawn = h.draws.length - before;
    expect(drawn).toBeGreaterThan(1);
    expect(drawn).toBeLessThanOrEqual(packV1.animations.thinking.fps + 1);
    r.destroy();
  });

  it("SpriteCharacterAdapter（自有 renderer）：suspend／hide 停 rAF，resume／show 才恢復；隱藏中 resume 不偷跑", async () => {
    const h = spriteHost();
    const a = new SpriteCharacterAdapter({ pack: packV1, assetBase: "/packs/shu-standard", canvas: h.canvas });
    await a.initialize(host);
    h.loadSheet();
    expect(h.pending()).toBe(1);
    a.suspend();
    expect(h.pending()).toBe(0);
    const n = h.draws.length;
    h.tick(performance.now() + 5000);
    expect(h.draws).toHaveLength(n);
    a.resume();
    expect(h.pending()).toBe(1);
    a.hide();
    expect(h.pending()).toBe(0);
    expect((h.canvas.style as unknown as Record<string, string>).visibility).toBe("hidden");
    a.show();
    expect(h.pending()).toBe(1);
    a.hide();
    a.suspend();
    a.resume(); // 仍隱藏：不恢復
    expect(h.pending()).toBe(0);
    a.show();
    expect(h.pending()).toBe(1);
    a.dispose();
    expect(h.pending()).toBe(0);
  });

  it("注入的 renderer：suspend／hide → pause，resume／show → resume（可選方法）", async () => {
    const renderer = new FakeRenderer();
    const a = new SpriteCharacterAdapter({ pack: packV1, assetBase: "/packs/shu-standard", renderer });
    await a.initialize(host);
    a.suspend();
    expect(renderer.paused).toBe(1);
    a.resume();
    expect(renderer.resumed).toBe(1);
    a.hide();
    expect(renderer.paused).toBe(2);
    a.show();
    expect(renderer.resumed).toBe(2);
    a.dispose();
    expect(renderer.destroyed).toBe(0); // 注入的 renderer 由 host 擁有
  });
});

describe("呈現層沒有權限主權：viaIntent 換不掉安全語意", () => {
  const packV1 = shuStandard as unknown as PackManifest;

  it("text adapter：安全 intent 的固定文案以 envelope.intent 為準，非安全 viaIntent 一律忽略", async () => {
    const a = new TextCharacterAdapter();
    await a.initialize(host);
    const receipts: AdapterReceipt[] = [];
    const sink = (r: AdapterReceipt) => receipts.push(r);
    const cases: Array<[CharacterIntent, TruthState, CharacterIntent, string]> = [
      ["request-consent", "waiting-consent", "greet", "需要你的同意才能繼續。"],
      ["blocked", "blocked", "play", FIXED_SAFETY_LINES.blocked],
      ["emergency", "emergency", "idle", FIXED_SAFETY_LINES.emergency],
      ["failed", "failed", "notice", FIXED_SAFETY_LINES.failed],
    ];
    for (const [intent, truth, viaIntent, expected] of cases) {
      a.perform(env(intent, truth), sink, { resolution: "substituted", via: "visual.textBubble", viaIntent });
      const line = a.currentLine();
      expect(line?.text, `${intent} via ${viaIntent}`).toBe(expected);
      expect(line?.fixed, `${intent} via ${viaIntent}`).toBe(true);
      expect(line?.intent, `${intent} via ${viaIntent}`).toBe(intent);
    }
    // 安全 → 安全的合法替換仍照 viaIntent 演出（failed → blocked）。
    a.perform(env("failed", "failed"), sink, { resolution: "substituted", via: "visual.textBubble", viaIntent: "blocked" });
    expect(a.currentLine()?.text).toBe(FIXED_SAFETY_LINES.blocked);
    // 非安全 intent 不受影響。
    a.perform(env("think", "working"), sink, { resolution: "substituted", via: "visual.textBubble", viaIntent: "work" });
    expect(a.currentLine()?.intent).toBe("work");
  });

  it("sprite adapter：安全 intent 不會因為 viaIntent 而播出玩耍／打招呼動畫", async () => {
    const renderer = new FakeRenderer();
    const a = new SpriteCharacterAdapter({ pack: packV1, assetBase: "/packs/shu-standard", renderer });
    await a.initialize(host);
    const sink = () => {};
    a.perform(env("blocked", "blocked"), sink, { resolution: "substituted", via: "visual.expression", viaIntent: "play" });
    expect(renderer.calls.slice(-1)[0]?.name).toBe("blocked");
    a.perform(env("emergency", "emergency"), sink, { resolution: "substituted", via: "visual.expression", viaIntent: "greet" });
    expect(renderer.calls.slice(-1)[0]?.name).toBe("emergency");
    a.perform(env("request-consent", "waiting-consent"), sink, { resolution: "substituted", via: "visual.expression", viaIntent: "play" });
    expect(renderer.calls.slice(-1)[0]?.name).toBe("ask");
    // 安全 → 安全仍照 viaIntent（v1 沒有 failed 美術 → blocked）。
    a.perform(env("failed", "failed"), sink, { resolution: "substituted", via: "visual.expression", viaIntent: "blocked" });
    expect(renderer.calls.slice(-1)[0]?.name).toBe("blocked");
    a.dispose();
  });
});
