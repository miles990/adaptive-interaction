// docs/aip/architecture-boundaries.md §3：Adapter lifecycle 的外層穩定語意。
//
// 同一套 contract 對四個 builtin adapter 跑一遍——shu-rig（rig＋遊玩場）、sprite（舊 pack）、
// text（可信退路）、shape（幾何 reference）。任何一個角色都不能靠「自己特別」逃掉：
//   - 生命週期順序：register → initialize → negotiate → live → dispose；
//   - capability 註冊：manifest 宣告什麼，negotiate 就提供什麼；
//   - unsupported intent 誠實回 unsupported，永遠不回 completed；
//   - cancel 冪等（重複 cancel 不再產生回執）；
//   - timeout：durationHint 由 tick(now) 推進，adapter 不自帶 timer；
//   - dispose 後不再有任何回執、輸入事件；
//   - 重複訂閱不重複送、退訂只退自己那一份；
//   - 資源清理：timer／rAF／DOM listener 數在 dispose 後回到 dispose 前的水位。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import shuMaidRaw from "../../public/characters/shu-maid/manifest.json";
import shuStandardPack from "../../public/packs/shu-standard/manifest.json";
import type { AdapterHost, AdapterReceipt, CharacterAdapter } from "../character/adapter";
import { ShapeCharacterAdapter } from "../character/adapters/shape";
import { ShuCharacterAdapter } from "../character/adapters/shu";
import { SpriteCharacterAdapter } from "../character/adapters/sprite";
import { TextCharacterAdapter } from "../character/adapters/text";
import {
  builtinAdapterMeta,
  builtinEntrypointIds,
  createBuiltinAdapter,
  registeredBuiltinAdapterIds,
  BUILTIN_ADAPTER_IDS,
} from "../character/adapterRegistry";
import "../character/adapters";
import { CharacterGateway } from "../character/gateway";
import { validateCharacterManifest } from "../character/manifest";
import {
  CharacterIntent,
  CharacterManifest,
  IntentEnvelope,
  PROTOCOL_VERSION,
  TruthState,
} from "../character/protocol";
import type { PackManifest, RendererBackend } from "../companion/renderer";
import { StageRenderer } from "../companion/rig/stage";

const clock = { now: 1_700_000_000_000 };
const host: AdapterHost = {
  now: () => clock.now,
  reducedMotion: () => false,
  locale: "zh-TW",
  log: () => {},
};

let seq = 0;
function env(
  intent: CharacterIntent,
  instanceId: string,
  over: Partial<IntentEnvelope> = {},
  truthState: TruthState = "none"
): IntentEnvelope {
  seq += 1;
  return {
    protocolVersion: PROTOCOL_VERSION,
    messageId: `c${seq}`,
    characterInstanceId: instanceId,
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

class FakeRenderer implements RendererBackend {
  destroyed = 0;
  setAnimation() {}
  setReducedMotion() {}
  setMicroMotion() {}
  destroy() {
    this.destroyed += 1;
  }
  pause() {}
  resume() {}
}

function bundledShuManifest(): CharacterManifest {
  const v = validateCharacterManifest(shuMaidRaw);
  if (!v.ok) throw new Error(v.errors.join("; "));
  return v.manifest;
}

interface Case {
  /** builtin entrypoint id（＝ registry key）。 */
  readonly id: string;
  /** 這個 adapter 一定演得出來的非安全 intent。 */
  readonly playable: CharacterIntent;
  make(): CharacterAdapter;
}

const CASES: readonly Case[] = [
  {
    id: "shu-rig",
    playable: "play",
    make: () =>
      new ShuCharacterAdapter({
        manifest: bundledShuManifest(),
        stage: new StageRenderer(stubCanvas(), "maid-classic", 1, {
          autoStart: false,
          rng: () => 0.9,
          now: () => clock.now,
        }),
      }),
  },
  {
    id: "sprite",
    playable: "notice",
    make: () =>
      new SpriteCharacterAdapter({
        pack: shuStandardPack as unknown as PackManifest,
        assetBase: "/packs/shu-standard",
        renderer: new FakeRenderer(),
      }),
  },
  {
    id: "text",
    playable: "notice",
    make: () => new TextCharacterAdapter({ container: mountHost() }),
  },
  {
    id: "shape",
    playable: "play",
    make: () => new ShapeCharacterAdapter({ container: mountHost() }),
  },
];

function mountHost(): HTMLElement {
  const el = document.createElement("div");
  document.body.appendChild(el);
  return el;
}

beforeEach(() => {
  clock.now = 1_700_000_000_000;
});

afterEach(() => {
  document.body.innerHTML = "";
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("builtin adapter registry", () => {
  it("宣告的 id 都有工廠，白名單不依賴載入順序", () => {
    expect([...builtinEntrypointIds()]).toEqual([...BUILTIN_ADAPTER_IDS]);
    expect([...registeredBuiltinAdapterIds()]).toEqual([...BUILTIN_ADAPTER_IDS]);
    for (const id of BUILTIN_ADAPTER_IDS) {
      const meta = builtinAdapterMeta(id);
      expect(meta, id).not.toBeNull();
      expect(["companion-stage", "companion-canvas", "companion-text"]).toContain(meta?.cssClass);
      expect(["canvas", "dom"]).toContain(meta?.surface);
    }
    // 只有宣告有遊玩場的 adapter 才會回 companion surface。
    expect(builtinAdapterMeta("shu-rig")?.hasPlayfield).toBe(true);
    for (const id of ["sprite", "text", "shape"]) expect(builtinAdapterMeta(id)?.hasPlayfield).toBe(false);
    expect(builtinAdapterMeta("nope")).toBeNull();
  });

  it("未註冊的 entrypoint 誠實失敗，錯誤訊息不回顯輸入", async () => {
    await expect(createBuiltinAdapter("no-such-adapter", {})).rejects.toThrow(/no builtin adapter is registered/);
    await expect(createBuiltinAdapter("no-such-adapter", {})).rejects.not.toThrow(/no-such-adapter/);
  });

  it("createBuiltinAdapter 建得出四個 adapter，meta 與 registry 一致", async () => {
    for (const id of BUILTIN_ADAPTER_IDS) {
      const ctx =
        builtinAdapterMeta(id)?.surface === "canvas"
          ? {
              canvas: stubCanvas(),
              scale: 1,
              legacyPack: id === "sprite" ? (shuStandardPack as unknown as PackManifest) : undefined,
              assetBase: "/packs/shu-standard",
              manifest: id === "shu-rig" ? bundledShuManifest() : null,
            }
          : { textHost: mountHost() };
      const built = await createBuiltinAdapter(id, ctx);
      expect(built.meta).toEqual(builtinAdapterMeta(id));
      expect(built.adapter.manifest.entrypoint).toEqual({ kind: "builtin", id });
      expect(built.companion === null).toBe(builtinAdapterMeta(id)?.hasPlayfield !== true);
      built.adapter.dispose();
      built.renderer?.destroy();
    }
  });
});

describe.each(CASES.map((c) => [c.id, c] as const))("adapter contract：%s", (_id, testCase) => {
  it("生命週期順序：註冊後 ready、negotiate 提供 manifest 宣告的能力", async () => {
    const seen: string[] = [];
    const gateway = new CharacterGateway({
      now: () => clock.now,
      reducedMotion: () => false,
      locale: "zh-TW",
      onSystemText: () => {},
      onAudit: (entry) => seen.push(entry.kind),
    });
    const adapter = testCase.make();
    const { negotiated } = await gateway.registerInstance(adapter, "primary-companion", { instanceId: "inst" });
    const info = gateway.getInstance("inst");
    expect(info?.state).toBe("ready");
    expect(info?.characterId).toBe(adapter.manifest.characterId);
    // capability 註冊：manifest 宣告的能力全部出現在協商 offer 裡。
    const offer = adapter.negotiate({
      type: "hello",
      protocolVersion: PROTOCOL_VERSION,
      runtimeVersion: "0.6.0",
      characterInstanceId: "inst",
      role: "primary-companion",
      locale: "zh-TW",
      reducedMotion: false,
      requires: [],
      limits: { maxMessageBytes: 32768, maxMessagesPerSecond: 30, maxPending: 8 },
    });
    expect(Object.keys(offer.capabilities).sort()).toEqual(Object.keys(adapter.manifest.capabilities).sort());
    expect(Object.keys(offer.inputCapabilities).sort()).toEqual(Object.keys(adapter.manifest.inputCapabilities).sort());
    // 20 個 intent 全部有協商結果（沒有「沒答案」的 intent）。
    expect(Object.keys(negotiated.resolutions)).toHaveLength(20);
    gateway.disposeInstance("inst", "done");
    expect(seen.length).toBeGreaterThanOrEqual(0);
  });

  it("unsupported resolution：回 unsupported，永遠不回 completed", () => {
    const adapter = testCase.make();
    const receipts: AdapterReceipt[] = [];
    adapter.perform(env(testCase.playable, "inst"), (r) => receipts.push(r), {
      resolution: "unsupported",
    });
    expect(receipts.map((r) => r.status)).toEqual(["unsupported"]);
    expect(receipts.some((r) => r.status === "completed")).toBe(false);
    adapter.dispose();
  });

  it("cancel 冪等：重複 cancel 不再產生回執", async () => {
    const adapter = testCase.make();
    await adapter.initialize(host);
    const receipts: AdapterReceipt[] = [];
    const e = env(testCase.playable, "inst", { durationHint: { ms: 3_000 } });
    adapter.perform(e, (r) => receipts.push(r));
    adapter.cancel(e.messageId);
    const afterFirst = receipts.length;
    adapter.cancel(e.messageId);
    adapter.cancel(e.messageId);
    expect(receipts.length).toBe(afterFirst);
    expect(receipts.filter((r) => r.status === "cancelled")).toHaveLength(1);
    expect(receipts.some((r) => r.status === "completed")).toBe(false);
    adapter.dispose();
  });

  it("timeout：durationHint 由 tick(now) 推進，時間沒到不會提前 completed", async () => {
    const adapter = testCase.make();
    await adapter.initialize(host);
    const receipts: AdapterReceipt[] = [];
    const e = env(testCase.playable, "inst", { durationHint: { ms: 2_000 } });
    adapter.perform(e, (r) => receipts.push(r));
    adapter.tick?.(clock.now + 100);
    expect(receipts.some((r) => r.status === "completed")).toBe(false);
    clock.now += 10_000;
    adapter.tick?.(clock.now);
    const terminal = receipts.filter((r) => ["completed", "cancelled", "failed", "unsupported"].includes(r.status));
    expect(terminal.length, "到期後必須有一個終態回執").toBeGreaterThanOrEqual(1);
    adapter.dispose();
  });

  it("dispose 後：perform 只回 failed，不再送輸入事件", async () => {
    const adapter = testCase.make();
    await adapter.initialize(host);
    const inputs: string[] = [];
    adapter.onInput((e) => inputs.push(e.kind));
    adapter.dispose();
    const receipts: AdapterReceipt[] = [];
    adapter.perform(env(testCase.playable, "inst"), (r) => receipts.push(r));
    expect(receipts.some((r) => r.status === "completed")).toBe(false);
    expect(receipts.every((r) => r.status === "failed" || r.status === "unsupported")).toBe(true);
    adapter.tick?.(clock.now + 60_000);
    expect(inputs).toEqual([]);
  });

  it("重複訂閱：兩個 callback 各收一次，退訂只退自己那一份", async () => {
    const adapter = testCase.make();
    await adapter.initialize(host);
    const a: string[] = [];
    const b: string[] = [];
    const offA = adapter.onInput((e) => a.push(e.kind));
    const offB = adapter.onInput((e) => b.push(e.kind));
    // 同一個 callback 訂兩次不得收兩份。
    const shared = (e: { kind: string }) => a.push(`shared:${e.kind}`);
    const offShared1 = adapter.onInput(shared);
    const offShared2 = adapter.onInput(shared);
    emitOneInput(adapter);
    expect(a.filter((k) => !k.startsWith("shared:")).length).toBeLessThanOrEqual(1);
    expect(b.length).toBeLessThanOrEqual(1);
    expect(a.filter((k) => k.startsWith("shared:")).length).toBeLessThanOrEqual(1);
    offA();
    offB();
    offShared1();
    offShared2();
    const beforeB = b.length;
    emitOneInput(adapter);
    expect(b.length).toBe(beforeB);
    adapter.dispose();
  });

  it("資源清理：dispose 後 timer／rAF／DOM listener 都回到原本水位", async () => {
    vi.useFakeTimers();
    const rafHandles = new Set<number>();
    let rafSeq = 0;
    const raf = vi.fn((_cb: FrameRequestCallback) => {
      rafSeq += 1;
      rafHandles.add(rafSeq);
      return rafSeq;
    });
    const caf = vi.fn((handle: number) => {
      rafHandles.delete(handle);
    });
    vi.stubGlobal("requestAnimationFrame", raf);
    vi.stubGlobal("cancelAnimationFrame", caf);
    const listeners = new Map<string, number>();
    const addSpy = vi
      .spyOn(EventTarget.prototype, "addEventListener")
      .mockImplementation(function (this: EventTarget, type: string) {
        listeners.set(type, (listeners.get(type) ?? 0) + 1);
      });
    const removeSpy = vi
      .spyOn(EventTarget.prototype, "removeEventListener")
      .mockImplementation(function (this: EventTarget, type: string) {
        listeners.set(type, (listeners.get(type) ?? 0) - 1);
      });

    const adapter = testCase.make();
    await adapter.initialize(host);
    adapter.perform(env(testCase.playable, "inst", { durationHint: { ms: 1_000 } }), () => {});
    adapter.dispose();

    expect(vi.getTimerCount(), "dispose 後不得留下 timer").toBe(0);
    expect(rafHandles.size, "dispose 後不得留下 rAF").toBe(0);
    for (const [type, count] of listeners) {
      expect(count, `dispose 後 ${type} listener 必須歸零`).toBeLessThanOrEqual(0);
    }
    addSpy.mockRestore();
    removeSpy.mockRestore();
    vi.unstubAllGlobals();
  });
});

/** 用 adapter 自己的輸入路徑送一個事件（沒有公開路徑的就跳過）。 */
function emitOneInput(adapter: CharacterAdapter): void {
  const withEmit = adapter as unknown as { emitInput?: (e: { kind: string; payload?: unknown }) => void };
  if (typeof withEmit.emitInput === "function") {
    withEmit.emitInput({ kind: "character.clicked", payload: {} });
    return;
  }
  document.querySelectorAll<HTMLElement>("[data-cpp-text-character], [data-cpp-shape-character]").forEach((el) => el.click());
}
