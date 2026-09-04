// 第二個 Reference Character：`ref-shape`（builtin `shape`）。
//
// 目的是證明「加一個角色不用改協定核心」：一份 manifest ＋ 一個 adapter 模組 ＋ 一列索引。
// 這裡驗：manifest 合法且只宣告幾何角色做得到的事 → 20 個 CPP intent 逐一送（非安全 exact、
// 安全落 system.text）→ 協商／降級 → 切換 shu ↔ ref-shape → dispose 清乾淨。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import refShapeRaw from "../../public/characters/ref-shape/manifest.json";
import characterIndex from "../../public/characters/index.json";
import type { AdapterHost, AdapterReceipt } from "../character/adapter";
import { ShapeCharacterAdapter, buildShapeCharacterManifest } from "../character/adapters/shape";
import { TextCharacterAdapter } from "../character/adapters/text";
import { createBuiltinAdapter, builtinEntrypointIds } from "../character/adapterRegistry";
import "../character/adapters";
import { CharacterGateway, type SystemTextMessage } from "../character/gateway";
import { validateCharacterManifest } from "../character/manifest";
import {
  CHARACTER_INTENTS,
  CharacterIntent,
  CharacterManifest,
  IntentEnvelope,
  PROTOCOL_VERSION,
  TruthState,
} from "../character/protocol";

const SAFETY_INTENTS: readonly CharacterIntent[] = [
  "wait",
  "ask",
  "request-consent",
  "blocked",
  "unknown",
  "claim-completed",
  "verified-success",
  "failed",
  "cancelled",
  "offline",
  "emergency",
];

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
    messageId: `s${seq}`,
    characterInstanceId: "shape-1",
    timestamp: "2026-09-04T00:00:00.000Z",
    intent,
    truthState,
    priority: 10,
    interruptPolicy: "preempt",
    resumePolicy: "none",
    privacyClass: "internal",
    ...over,
  };
}

function container(): HTMLElement {
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
});

describe("ref-shape manifest", () => {
  it("bundled manifest 與 adapter 內建的定義一致，且只宣告幾何角色做得到的事", () => {
    const built = buildShapeCharacterManifest();
    const v = validateCharacterManifest(refShapeRaw);
    expect(v.ok).toBe(true);
    if (!v.ok) return;
    expect(v.manifest.characterId).toBe("ref-shape");
    expect(v.manifest.displayName).toEqual({ "zh-TW": "參考形狀", en: "Reference Shape" });
    expect(v.manifest.adapterKind).toBe("in-process");
    expect(v.manifest.entrypoint).toEqual({ kind: "builtin", id: "shape" });
    expect(v.manifest.assets).toEqual([]);
    expect(Object.keys(v.manifest.capabilities).sort()).toEqual(["visual.expression", "visual.presence"]);
    expect(Object.keys(v.manifest.inputCapabilities)).toEqual(["input.click"]);
    // 不支援 audio／gaze／particles。
    for (const missing of ["audio.speech", "audio.effect", "visual.gaze", "visual.particles", "gameplay.toys"]) {
      expect(v.manifest.capabilities[missing]).toBeUndefined();
    }
    expect(v.manifest.capabilities["visual.expression"].variants).toEqual([
      "idle",
      "notice",
      "play",
      "rest",
      "work",
      "think",
      "acknowledge",
      "wait",
      "greet",
      "sleep",
    ]);
    // 安全 intent 不宣告：CPP 規則會讓它們落到 system.text。
    for (const intent of SAFETY_INTENTS) expect(v.manifest.intents).not.toContain(intent);
    expect(built.characterId).toBe(v.manifest.characterId);
    expect(built.capabilities).toEqual(v.manifest.capabilities);
    expect(built.intents).toEqual(v.manifest.intents);
    expect(built.securityRequirements.audioOutput).toBe(false);
  });

  it("角色索引把 ref-shape 列為內建角色", () => {
    const entry = (characterIndex as { characters: { characterId: string; origin?: string; manifestPath: string }[] }).characters.find(
      (c) => c.characterId === "ref-shape"
    );
    expect(entry).toBeDefined();
    expect(entry?.origin).toBe("builtin");
    expect(entry?.manifestPath).toBe("/characters/ref-shape/manifest.json");
  });

  it("shape 在 builtin adapter registry 裡，角色頁能建出它", async () => {
    expect(builtinEntrypointIds()).toContain("shape");
    const built = await createBuiltinAdapter("shape", { textHost: container() });
    expect(built.adapter.manifest.characterId).toBe("ref-shape");
    expect(built.renderer).toBeNull();
    expect(built.companion).toBeNull();
    built.adapter.dispose();
  });
});

describe("ShapeCharacterAdapter 呈現", () => {
  async function connect(): Promise<{ adapter: ShapeCharacterAdapter; gateway: CharacterGateway; texts: SystemTextMessage[] }> {
    const texts: SystemTextMessage[] = [];
    const gateway = new CharacterGateway({
      now: () => clock.now,
      reducedMotion: () => false,
      locale: "zh-TW",
      onSystemText: (m) => texts.push(m),
    });
    const adapter = new ShapeCharacterAdapter({ container: container() });
    await gateway.registerInstance(adapter, "primary-companion", { instanceId: "shape-1" });
    return { adapter, gateway, texts };
  }

  it("20 個 CPP intent：非安全 exact 演出、安全落可信 system.text", async () => {
    const { adapter, gateway, texts } = await connect();
    const performed: CharacterIntent[] = [];
    for (const intent of CHARACTER_INTENTS) {
      const before = texts.length;
      gateway.dispatch(env(intent, intent === "verified-success" ? "verified" : "none"));
      clock.now += 5_000;
      gateway.sweep(clock.now);
      const state = adapter.currentState();
      if (SAFETY_INTENTS.includes(intent)) {
        expect(texts.length, `${intent} 必須落 system.text`).toBeGreaterThan(before);
      } else {
        expect(texts.length, `${intent} 不該落 system.text`).toBe(before);
        expect(state?.intent, `${intent} 必須由角色演出`).toBe(intent);
        performed.push(intent);
      }
    }
    expect(performed.sort()).toEqual(
      ["acknowledge", "greet", "idle", "notice", "play", "rest", "sleep", "think", "work"].sort()
    );
    gateway.disposeInstance("shape-1", "test done");
  });

  it("顏色隨 intent 家族改變；play 脈衝一次、notice 位移、rest／idle 靜止", async () => {
    const { adapter } = await connect();
    adapter.perform(env("play"), () => {});
    const play = adapter.currentState();
    expect(play?.motion).toBe("pulse");
    adapter.perform(env("notice"), () => {});
    expect(adapter.currentState()?.motion).toBe("nudge");
    adapter.perform(env("rest"), () => {});
    expect(adapter.currentState()?.motion).toBe("still");
    adapter.perform(env("idle"), () => {});
    expect(adapter.currentState()?.motion).toBe("still");
    // 顏色只有幾個家族值；play 與 idle 不同色。
    adapter.perform(env("play"), () => {});
    const playColor = adapter.currentState()?.color;
    adapter.perform(env("idle"), () => {});
    expect(adapter.currentState()?.color).not.toBe(playColor);
  });

  it("Reduced Motion：不動，只變色", async () => {
    const reduced: AdapterHost = { ...host, reducedMotion: () => true };
    const adapter = new ShapeCharacterAdapter({ container: container() });
    await adapter.initialize(reduced);
    adapter.perform(env("play"), () => {});
    expect(adapter.currentState()?.motion).toBe("still");
    expect(adapter.currentState()?.color).toBeTruthy();
    const offer = adapter.negotiate({
      type: "hello",
      protocolVersion: PROTOCOL_VERSION,
      runtimeVersion: "0.6.0",
      characterInstanceId: "shape-1",
      role: "primary-companion",
      locale: "zh-TW",
      reducedMotion: true,
      requires: [...CHARACTER_INTENTS],
      limits: { maxMessageBytes: 32768, maxMessagesPerSecond: 30, maxPending: 8 },
    });
    expect(offer.capabilities["visual.expression"].reducedMotionBehavior).toBe("static");
    adapter.dispose();
  });

  it("input.click 送出 character.clicked；不宣告的輸入不送", async () => {
    const el = container();
    const adapter = new ShapeCharacterAdapter({ container: el });
    await adapter.initialize(host);
    const seen: string[] = [];
    const off = adapter.onInput((e) => seen.push(e.kind));
    el.querySelector<HTMLElement>("[data-cpp-shape-character]")?.click();
    expect(seen).toEqual(["character.clicked"]);
    off();
    el.querySelector<HTMLElement>("[data-cpp-shape-character]")?.click();
    expect(seen).toEqual(["character.clicked"]);
    adapter.dispose();
  });
});

describe("切換角色", () => {
  it("shu ↔ ref-shape 互換：每次只有一個 adapter 在線，舊的被 dispose", async () => {
    const gateway = new CharacterGateway({
      now: () => clock.now,
      reducedMotion: () => false,
      locale: "zh-TW",
      onSystemText: () => {},
    });
    const shape = new ShapeCharacterAdapter({ container: container() });
    await gateway.registerInstance(shape, "primary-companion", { instanceId: "primary" });
    expect(gateway.getInstance("primary")?.characterId).toBe("ref-shape");

    const text = new TextCharacterAdapter({ container: container() });
    gateway.disposeInstance("primary", "switch");
    await gateway.reattach("primary", text);
    expect(gateway.getInstance("primary")?.characterId).toBe("plain-text");
    const receipts: AdapterReceipt[] = [];
    shape.perform(env("play"), (r) => receipts.push(r));
    expect(receipts.map((r) => r.status)).toEqual(["failed"]);

    const back = new ShapeCharacterAdapter({ container: container() });
    gateway.disposeInstance("primary", "switch back");
    await gateway.reattach("primary", back);
    expect(gateway.getInstance("primary")?.characterId).toBe("ref-shape");
    gateway.disposeInstance("primary", "done");
  });
});

describe("manifest 型別", () => {
  it("bundled manifest 是合法的 CharacterManifest 物件", () => {
    const m = refShapeRaw as unknown as CharacterManifest;
    expect(m.schemaVersion).toBe("1.0");
    expect(m.compatibility?.protocol).toBe("1.x");
  });
});
