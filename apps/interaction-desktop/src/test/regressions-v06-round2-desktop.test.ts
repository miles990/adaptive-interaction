// v0.6.0 第二輪對抗審查（desktop-and-tauri 組）確認缺陷的回歸測試。
//
// 每一條都對應一個 confirmed finding，並且在修復前必須是紅的：
//   character-package-017   匯入的 builtin:shape 角色被冒名成內建 ref-shape
//   character-package-018   host 接線層用小樞專屬 helper 決定 variant／legacy pack
//   renderer-lifecycle-028  sprite suspend() 對共享 machine 送 force clear-transient
//   renderer-lifecycle-029  MixerRenderer 不轉送 pause()／resume()
//   renderer-lifecycle-030  sprite 對沒上台的 intent 仍回 started→completed
//   renderer-lifecycle-033  Gateway.handshake 對 adapter.onInput() 沒有 try/catch
//   character-package-020   TS 鏡射把 assets[].mediaType 當選填（Rust／golden 必填）

import { describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import shuStandardPack from "../../public/packs/shu-standard/manifest.json";
import type { AdapterHost, AdapterInputEvent, AdapterReceipt, CharacterAdapter } from "../character/adapter";
import { buildShapeCharacterManifest } from "../character/adapters/shape";
import { SpriteCharacterAdapter } from "../character/adapters/sprite";
import { builtinAdapterMeta, createBuiltinAdapter } from "../character/adapterRegistry";
import "../character/adapters";
import { CharacterGateway } from "../character/gateway";
import { validateCharacterManifest } from "../character/manifest";
import {
  CharacterManifest,
  Hello,
  IntentEnvelope,
  Negotiate,
  PROTOCOL_VERSION,
} from "../character/protocol";
import { MixerRenderer } from "../companion/mixerRenderer";
import { MachineEvent, MachineState, reduce } from "../companion/machine";
import { SpriteRenderer, type PackManifest, type RendererBackend } from "../companion/renderer";

const NOW = 1_700_000_000_000;
const host: AdapterHost = { now: () => NOW, reducedMotion: () => false, locale: "zh-TW", log: () => {} };

function env(intent: IntentEnvelope["intent"], over: Partial<IntentEnvelope> = {}): IntentEnvelope {
  return {
    protocolVersion: PROTOCOL_VERSION,
    messageId: `m-${intent}`,
    kind: "play",
    intent,
    priority: 30,
    truthState: "none",
    interruptible: true,
    ...over,
  } as IntentEnvelope;
}

/** 一個真的由 `reduce` 驅動的 mixer（不是假的），讓 MixerRenderer 走正式路徑。 */
function liveMixer(initial: MachineState = { base: "idle", transient: null }) {
  let st = initial;
  const seen: MachineEvent[] = [];
  return {
    seen,
    apply(ev: MachineEvent): MachineState {
      seen.push(ev);
      st = reduce(st, ev, NOW);
      return st;
    },
    state(): MachineState {
      return st;
    },
  };
}

function stubCanvas(): HTMLCanvasElement {
  const canvas = document.createElement("canvas");
  canvas.getContext = (() => ({
    canvas,
    imageSmoothingEnabled: true,
    clearRect() {},
    drawImage() {},
    save() {},
    restore() {},
    translate() {},
    scale() {},
    fillRect() {},
    beginPath() {},
    arc() {},
    fill() {},
  })) as unknown as HTMLCanvasElement["getContext"];
  return canvas;
}

describe("character-package-017：builtin:shape 的匯入角色不得冒名 ref-shape", () => {
  function importedShapeManifest(characterId: string): CharacterManifest {
    const base = buildShapeCharacterManifest() as unknown as Record<string, unknown>;
    const raw = {
      ...base,
      characterId,
      displayName: { "zh-TW": "阿克米方塊", en: "Acme Blob" },
      author: "acme",
    };
    const v = validateCharacterManifest(raw);
    if (!v.ok) throw new Error(v.errors.join("; "));
    return v.manifest;
  }

  it("工廠收到 ctx.manifest／ctx.characterId 時，adapter 的身分是匯入角色而不是 ref-shape", async () => {
    const manifest = importedShapeManifest("acme-blob");
    const built = await createBuiltinAdapter("shape", {
      characterId: "acme-blob",
      manifest,
      displayName: manifest.displayName,
      textHost: document.createElement("div"),
    });
    expect(built.adapter.manifest.characterId).toBe("acme-blob");
    expect(built.adapter.manifest.displayName["zh-TW"]).not.toBe("參考形狀");
    const negotiate = built.adapter.negotiate({
      type: "hello",
      protocolVersion: PROTOCOL_VERSION,
      runtimeVersion: "0.6.0-test",
      characterInstanceId: "inst",
      role: "primary-companion",
      locale: "zh-TW",
      reducedMotion: false,
      requires: [],
      limits: { maxMessageBytes: 65536, maxMessagesPerSecond: 30, maxPending: 8 },
    } as Hello) as Negotiate;
    expect(negotiate.characterId).toBe("acme-blob");
  });
});

describe("renderer-lifecycle-028／029／030：sprite adapter 的暫停與搶佔誠實度", () => {
  const pack = shuStandardPack as unknown as PackManifest;

  it("028：suspend() 不得清掉受保護的安全 transient（blocked）", async () => {
    const mixer = liveMixer();
    const real: RendererBackend = {
      setAnimation: () => {},
      setReducedMotion: () => {},
      setMicroMotion: () => {},
      destroy: () => {},
      pause: () => {},
      resume: () => {},
    };
    const facade = new MixerRenderer(real, mixer);
    const adapter = new SpriteCharacterAdapter({ pack, assetBase: "/packs/shu-standard", renderer: facade });
    await adapter.initialize(host);
    facade.setAnimation("blocked");
    expect(mixer.state().transient?.kind).toBe("blocked");
    adapter.suspend();
    expect(mixer.state().transient?.kind, "suspend() 不得抹掉安全訊息").toBe("blocked");
  });

  it("029：MixerRenderer 要把 pause()／resume() 轉給真正的 renderer", () => {
    const calls: string[] = [];
    const real: RendererBackend = {
      setAnimation: () => {},
      setReducedMotion: () => {},
      setMicroMotion: () => {},
      destroy: () => {},
      pause: () => calls.push("pause"),
      resume: () => calls.push("resume"),
    };
    const facade = new MixerRenderer(real, liveMixer());
    facade.pause?.();
    facade.resume?.();
    expect(calls).toEqual(["pause", "resume"]);
  });

  it("029b：正式接線（MixerRenderer 包真 SpriteRenderer）下 suspend／hide 真的停掉 rAF", async () => {
    const raf = vi.fn((_cb: FrameRequestCallback) => 1);
    const caf = vi.fn(() => {});
    vi.stubGlobal("requestAnimationFrame", raf);
    vi.stubGlobal("cancelAnimationFrame", caf);
    try {
      const real = new SpriteRenderer(stubCanvas(), pack, "/packs/shu-standard/sheet.png", 1);
      const facade = new MixerRenderer(real, liveMixer());
      const adapter = new SpriteCharacterAdapter({ pack, assetBase: "/packs/shu-standard", renderer: facade });
      await adapter.initialize(host);
      adapter.suspend();
      expect((real as unknown as { isPaused(): boolean }).isPaused()).toBe(true);
      adapter.resume();
      expect((real as unknown as { isPaused(): boolean }).isPaused()).toBe(false);
      adapter.hide();
      expect((real as unknown as { isPaused(): boolean }).isPaused()).toBe(true);
      real.destroy();
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("030：混音器沒讓它上台時，sprite 要回 cancelled{preempted}，不得回 started／completed", async () => {
    const mixer = liveMixer();
    const real: RendererBackend = {
      setAnimation: () => {},
      setReducedMotion: () => {},
      setMicroMotion: () => {},
      destroy: () => {},
    };
    const facade = new MixerRenderer(real, mixer);
    const adapter = new SpriteCharacterAdapter({
      pack,
      assetBase: "/packs/shu-standard",
      renderer: facade,
      mixer,
    } as never);
    await adapter.initialize(host);
    // 先讓高優先的安全訊息（blocked=90）佔住舞台。
    facade.setAnimation("blocked");
    expect(mixer.state().transient?.kind).toBe("blocked");
    const receipts: AdapterReceipt[] = [];
    adapter.perform(env("notice", { messageId: "preempted-1" }), (r) => receipts.push(r));
    adapter.tick?.(NOW + 60_000);
    const statuses = receipts.map((r) => r.status);
    expect(statuses).not.toContain("completed");
    expect(statuses).toContain("cancelled");
    expect(mixer.state().transient?.kind, "畫面從未換過").toBe("blocked");
  });
});

describe("renderer-lifecycle-033：Gateway.handshake 對 adapter.onInput() 的例外要收好", () => {
  class ThrowingOnInputAdapter implements CharacterAdapter {
    readonly manifest: CharacterManifest = buildShapeCharacterManifest();
    async initialize(_host: AdapterHost): Promise<void> {
      void _host;
    }
    negotiate(_hello: Hello): Negotiate {
      void _hello;
      return {
        type: "negotiate",
        protocolVersion: PROTOCOL_VERSION,
        characterId: this.manifest.characterId,
        manifestVersion: this.manifest.version,
        capabilities: this.manifest.capabilities,
        inputCapabilities: this.manifest.inputCapabilities,
        channels: this.manifest.channels,
        intents: this.manifest.intents,
        variants: [],
        generation: 0,
        fallbacks: this.manifest.fallbacks,
      };
    }
    perform(): void {}
    cancel(): void {}
    dispose(): void {}
    show(): void {}
    hide(): void {}
    suspend(): void {}
    resume(): void {}
    reconfigure(): void {}
    onInput(_cb: (e: AdapterInputEvent) => void): () => void {
      void _cb;
      throw new Error("onInput exploded");
    }
  }

  it("onInput 擲例外：例外不穿出 Gateway，實例不會卡在 negotiating，同 id 可以重新註冊", async () => {
    const lifecycle: Array<[string, string]> = [];
    const gw = new CharacterGateway({
      now: () => NOW,
      onSystemText: () => {},
      onLifecycle: (id, state) => {
        lifecycle.push([id, state]);
      },
    });
    await expect(
      gw.registerInstance(new ThrowingOnInputAdapter(), "primary-companion", { instanceId: "inst-x" })
    ).resolves.toBeDefined();
    expect(gw.getInstance("inst-x")?.state).not.toBe("negotiating");
    // 收尾之後同一個 instanceId 必須可以再註冊（不得殘留殭屍）。
    const shape = await createBuiltinAdapter("shape", { textHost: document.createElement("div") });
    await expect(
      gw.registerInstance(shape.adapter, "primary-companion", { instanceId: "inst-x" })
    ).resolves.toBeDefined();
    expect(lifecycle.length).toBeGreaterThan(0);
  });
});

describe("character-package-018：初始 variant／舊 pack 由 adapter 宣告，host 不猜", () => {
  it("沒宣告 defaultVariant 的 adapter 不會拿到別人的預設配色", () => {
    // sprite 宣告 requiresLegacyPackShape 但沒有 variants／defaultVariant：
    // host 對它算出來的初始 variant 必須是 null，不是某個 rig 的 "maid-classic"。
    const sprite = builtinAdapterMeta("sprite");
    expect(sprite?.defaultVariant).toBeUndefined();
    expect(sprite?.legacyPackForEntry).toBeUndefined();
    const shape = builtinAdapterMeta("shape");
    expect(shape?.defaultVariant).toBeUndefined();
    expect(shape?.legacyPackForEntry).toBeUndefined();
  });

  it("rig 自己宣告 defaultVariant／legacyPackForEntry（角色知識住在 adapter）", () => {
    const rig = builtinAdapterMeta("shu-rig");
    expect(typeof rig?.defaultVariant).toBe("function");
    expect(rig?.defaultVariant?.(null)).toBe("maid-classic");
    const pack = rig?.legacyPackForEntry?.({ characterId: "acme", displayName: {}, version: "1.0.0" }, null) as
      | Record<string, unknown>
      | null
      | undefined;
    expect(pack?.["kind"]).toBe("character-rig");
    expect(pack?.["id"]).toBe("acme");
  });

  it("CompanionApp 只呼叫 meta hook，不再 import 任何 rig 專屬 helper", () => {
    const source = readFileSync(join(__dirname, "..", "companion", "CompanionApp.tsx"), "utf8");
    expect(source).toContain("meta?.defaultVariant?.(");
    expect(source).toContain("meta?.legacyPackForEntry?.(");
    for (const helper of ["rigPaletteForImported", "rigPaletteFor", "importedRigPack"]) {
      expect(source, `${helper} 是某個 rig 的知識，不該出現在接線層`).not.toContain(helper);
    }
  });
});

describe("character-package-020：TS 鏡射的 assets 驗證要和 Rust／golden schema 一致", () => {
  function withAssets(assets: unknown): unknown {
    const base = buildShapeCharacterManifest() as unknown as Record<string, unknown>;
    return { ...base, characterId: "asset-check", assets };
  }

  it("缺 mediaType 的 manifest 必須被 TS 擋下（golden schema 是必填）", () => {
    const r = validateCharacterManifest(withAssets([{ id: "sheet", path: "sheet.png" }]));
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.errors.join("; ")).toContain("mediaType");
  });

  it("大寫 mediaType 也擋下（Rust is_media_type 只收小寫）", () => {
    const r = validateCharacterManifest(withAssets([{ id: "sheet", path: "sheet.png", mediaType: "IMAGE/PNG" }]));
    expect(r.ok).toBe(false);
  });

  it("合法的小寫 mediaType 照樣通過", () => {
    const r = validateCharacterManifest(withAssets([{ id: "sheet", path: "sheet.png", mediaType: "image/png" }]));
    expect(r.ok, r.ok ? "" : r.errors.join("; ")).toBe(true);
    if (r.ok) expect(r.manifest.assets[0]?.mediaType).toBe("image/png");
  });

  it("golden schema 也把 mediaType 列為必填（兩邊同一套規則）", () => {
    const schema = JSON.parse(
      readFileSync(join(__dirname, "../../../../schemas/character-protocol.schema.json"), "utf8")
    ) as { definitions?: Record<string, { required?: string[] }>; $defs?: Record<string, { required?: string[] }> };
    const defs = schema.$defs ?? schema.definitions ?? {};
    expect(defs["AssetDecl"]?.required ?? []).toContain("mediaType");
  });
});
