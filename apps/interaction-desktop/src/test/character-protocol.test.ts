// CPP §13（TS 側）：protocol 常數、manifest 驗證（含惡意輸入）、舊 pack 遷移、
// 協定版本協商、能力協商（exact／substituted／reduced／unsupported）、fallback 確定性。

import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import shuStandard from "../../public/packs/shu-standard/manifest.json";
import shuLively from "../../public/packs/shu-lively/manifest.json";
import shuMaid from "../../public/packs/shu-maid/manifest.json";
import { hostMigrationRegistry } from "../character/adapterRegistry";
// side effect：載入桌面 host 的 builtin adapter 與 migrator 註冊（rig 2.0 由 shu adapter 提供）。
import "../character/adapters";
import { rigPackMigrator, shuRigCapabilities } from "../character/adapters/shu";
import {
  coreMigrationRegistry,
  defaultMigrationRegistry,
  displayNameOf,
  MAX_MIGRATOR_VERSIONS,
  MAX_MIGRATORS,
  MigrationRegistry,
  migratePackToManifest,
  pronounOf,
  spritePackMigrator,
  validateCharacterManifest,
  type PackMigrator,
} from "../character/manifest";
import { classifyChannels, negotiate, ProtocolVersionError, resolveIntent } from "../character/negotiate";
import {
  ADAPTER_LIFECYCLE_STATES,
  CANONICAL_CAPABILITY_IDS,
  CHARACTER_INTENTS,
  CHARACTER_ROLES,
  Hello,
  INPUT_EVENT_KINDS,
  INTENT_CAPABILITIES,
  isSafetyIntent,
  LIMITS,
  Negotiate,
  PRIORITY_FLOOR,
  priorityFloor,
  PROTOCOL_VERSION,
  RECEIPT_STATUSES,
  SAFETY_INTENTS,
  SEMANTIC_CHANNELS,
  TRUTH_STATES,
} from "../character/protocol";
import { deriveIntentFallbacks, resolveSpriteAnimation } from "../character/spriteIntents";

function baseManifest(over: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    schemaVersion: "1.0",
    characterId: "demo-character",
    displayName: { "zh-TW": "示範", en: "Demo" },
    author: "tests",
    version: "1.2.3",
    adapterKind: "in-process",
    entrypoint: { kind: "builtin", id: "text" },
    assets: [{ id: "sheet", path: "art/sheet.png", mediaType: "image/png", bytes: 1024 }],
    capabilities: {
      "visual.presence": { supported: true },
      "visual.expression": { supported: true, variants: ["idle", "notice"], interruptible: true },
    },
    inputCapabilities: { "input.click": { supported: true } },
    channels: ["expression", "bubble"],
    intents: ["idle", "notice"],
    ...over,
  };
}

function hello(over: Partial<Hello> = {}): Hello {
  return {
    type: "hello",
    protocolVersion: PROTOCOL_VERSION,
    runtimeVersion: "0.5.0",
    characterInstanceId: "inst-1",
    role: "primary-companion",
    locale: "zh-TW",
    reducedMotion: false,
    requires: [...CHARACTER_INTENTS],
    limits: { maxMessageBytes: LIMITS.maxMessageBytes, maxMessagesPerSecond: 50, maxPending: 64 },
    ...over,
  };
}

function offer(over: Partial<Negotiate> = {}): Negotiate {
  return {
    type: "negotiate",
    protocolVersion: PROTOCOL_VERSION,
    characterId: "demo",
    manifestVersion: "1.0.0",
    capabilities: {
      "visual.presence": { supported: true },
      "visual.expression": { supported: true, reducedMotionBehavior: "static" },
    },
    inputCapabilities: {},
    channels: ["expression"],
    intents: [...CHARACTER_INTENTS],
    variants: [],
    generation: 0,
    ...over,
  };
}

describe("CPP protocol 常數（§3／§4／§6／§7）", () => {
  it("詞彙數量與文件一致", () => {
    expect(CHARACTER_INTENTS).toHaveLength(20);
    expect(TRUTH_STATES).toHaveLength(15);
    expect(INPUT_EVENT_KINDS).toHaveLength(13);
    expect(RECEIPT_STATUSES).toHaveLength(10);
    expect(CANONICAL_CAPABILITY_IDS).toHaveLength(26);
    expect(CHARACTER_ROLES).toHaveLength(5);
    // 文件 §7 列出 14 個狀態（含 crashed／reconnecting）。
    expect(ADAPTER_LIFECYCLE_STATES).toHaveLength(14);
  });

  it("priority floor 表與 §4.3 相同；安全 intent = 有 floor 者", () => {
    expect(PRIORITY_FLOOR.emergency).toBe(100);
    expect(PRIORITY_FLOOR.offline).toBe(95);
    expect(PRIORITY_FLOOR.blocked).toBe(90);
    expect(PRIORITY_FLOOR.failed).toBe(85);
    expect(PRIORITY_FLOOR["request-consent"]).toBe(80);
    expect(PRIORITY_FLOOR.unknown).toBe(75);
    expect(PRIORITY_FLOOR["verified-success"]).toBe(70);
    expect(PRIORITY_FLOOR["claim-completed"]).toBe(65);
    expect(PRIORITY_FLOOR.wait).toBe(60);
    expect(PRIORITY_FLOOR.ask).toBe(60);
    expect(PRIORITY_FLOOR.cancelled).toBe(55);
    for (const i of ["idle", "notice", "acknowledge", "think", "work", "greet", "play", "rest", "sleep"] as const) {
      expect(PRIORITY_FLOOR[i]).toBe(0);
      expect(isSafetyIntent(i)).toBe(false);
    }
    expect(SAFETY_INTENTS).toHaveLength(11);
    expect(isSafetyIntent("emergency")).toBe(true);
    expect(isSafetyIntent("not-an-intent")).toBe(false);
    expect(priorityFloor("blocked")).toBe(90);
    expect(priorityFloor("garbage")).toBe(0);
  });
});

describe("Manifest 驗證（§2.1）", () => {
  it("合法 manifest 通過並補齊預設值、保留未知欄位", () => {
    const r = validateCharacterManifest(baseManifest({ futureField: { keep: true } }));
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.manifest.resourceLimits).toEqual({ maxAssetBytes: 8388608, maxConcurrentCommands: 4, maxQueue: 32, maxFps: 60 });
    expect(r.manifest.securityRequirements.executable).toBe(false);
    expect(r.manifest.compatibility.protocol).toBe("1.x");
    expect((r.manifest as unknown as Record<string, unknown>).futureField).toEqual({ keep: true });
    expect(r.report.flags).toEqual({ external: false, network: false, executable: false, unsigned: true });
  });

  it("schemaVersion：major ≠ 1 拒絕；minor 較新允許並標 newerMinor", () => {
    expect(validateCharacterManifest(baseManifest({ schemaVersion: "2.0" })).ok).toBe(false);
    expect(validateCharacterManifest(baseManifest({ schemaVersion: "x" })).ok).toBe(false);
    const r = validateCharacterManifest(baseManifest({ schemaVersion: "1.7" }));
    expect(r.ok && r.report.newerMinor).toBe(true);
  });

  it("檔案大小 > 256 KB 拒絕", () => {
    const text = JSON.stringify({ ...baseManifest(), pad: "x".repeat(LIMITS.manifestMaxBytes) });
    const r = validateCharacterManifest(JSON.parse(text), { jsonText: text });
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.errors[0]).toMatch(/exceeds/);
  });

  it("惡意資產路徑：../、絕對、磁碟代號、反斜線、URL、~ 全部拒絕", () => {
    const bad = ["../secret.png", "/etc/passwd", "C:\\Windows\\x.png", "art\\sheet.png", "https://evil.example/x.png", "file:///x", "~/x.png", "a/../b.png"];
    for (const path of bad) {
      const r = validateCharacterManifest(baseManifest({ assets: [{ id: "a", path }] }));
      expect(r.ok, path).toBe(false);
    }
    expect(validateCharacterManifest(baseManifest({ assets: [{ id: "a", path: "sub/dir/ok.png" }] })).ok).toBe(true);
  });

  it("assets > 64 項、單一資產超過 maxAssetBytes、maxAssetBytes > 32 MB 拒絕", () => {
    const many = Array.from({ length: 65 }, (_, i) => ({ id: `a${i}`, path: `a${i}.png` }));
    expect(validateCharacterManifest(baseManifest({ assets: many })).ok).toBe(false);
    expect(validateCharacterManifest(baseManifest({ assets: [{ id: "a", path: "a.png", bytes: 9_000_000 }] })).ok).toBe(false);
    expect(validateCharacterManifest(baseManifest({ resourceLimits: { maxAssetBytes: 64 * 1024 * 1024 } })).ok).toBe(false);
    expect(
      validateCharacterManifest(baseManifest({ resourceLimits: { maxAssetBytes: 16_000_000 }, assets: [{ id: "a", path: "a.png", bytes: 9_000_000 }] })).ok
    ).toBe(true);
  });

  it("characterId 規則與 LocalizedText 長度", () => {
    for (const id of ["Bad", "-x", "a b", "a".repeat(65), "", "x/y"]) {
      expect(validateCharacterManifest(baseManifest({ characterId: id })).ok, id).toBe(false);
    }
    expect(validateCharacterManifest(baseManifest({ displayName: {} })).ok).toBe(false);
    expect(validateCharacterManifest(baseManifest({ displayName: { en: "x".repeat(49) } })).ok).toBe(false);
    expect(validateCharacterManifest(baseManifest({ description: { en: "x".repeat(401) } })).ok).toBe(false);
    expect(validateCharacterManifest(baseManifest({ author: "x".repeat(121) })).ok).toBe(false);
    expect(validateCharacterManifest(baseManifest({ version: "not-semver" })).ok).toBe(false);
  });

  it("entrypoint：builtin 白名單；process／url／module 只記錄不執行；kind 需與 adapterKind 相符", () => {
    expect(validateCharacterManifest(baseManifest({ entrypoint: { kind: "builtin", id: "evil" } })).ok).toBe(false);
    expect(validateCharacterManifest(baseManifest({ entrypoint: { kind: "builtin", id: "shu-rig" } }), { builtinWhitelist: ["text"] }).ok).toBe(false);
    const proc = validateCharacterManifest(
      baseManifest({ adapterKind: "external-process", entrypoint: { kind: "process", command: ["/usr/bin/evil", "--rm-rf"] } })
    );
    expect(proc.ok).toBe(true);
    if (proc.ok) {
      expect(proc.manifest.entrypoint).toEqual({ kind: "process", command: ["/usr/bin/evil", "--rm-rf"] });
      expect(proc.report.flags.executable).toBe(true);
      expect(proc.report.flags.external).toBe(true);
    }
    const url = validateCharacterManifest(baseManifest({ adapterKind: "remote-device", entrypoint: { kind: "url", url: "ws://127.0.0.1:9000" } }));
    expect(url.ok && url.report.flags.network).toBe(true);
    expect(validateCharacterManifest(baseManifest({ adapterKind: "remote-device", entrypoint: { kind: "url", url: "http://evil" } })).ok).toBe(false);
    expect(validateCharacterManifest(baseManifest({ adapterKind: "web", entrypoint: { kind: "module", path: "../x.js" } })).ok).toBe(false);
    expect(validateCharacterManifest(baseManifest({ adapterKind: "web", entrypoint: { kind: "module", path: "adapter.js" } })).ok).toBe(true);
    // 不相符：in-process 卻給 process
    expect(validateCharacterManifest(baseManifest({ entrypoint: { kind: "process", command: ["x"] } })).ok).toBe(false);
  });

  it("capability id：canonical／namespaced custom 接受；未收錄 canonical 前綴標 unknown；其他拒絕；system.text 不可宣告", () => {
    const r = validateCharacterManifest(
      baseManifest({
        capabilities: {
          "visual.presence": { supported: true },
          "com.example.character.wings": { supported: true },
          "visual.wings": { supported: true },
        },
      })
    );
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.report.customCapabilities).toEqual(["com.example.character.wings"]);
      expect(r.report.unknownCapabilities).toEqual(["visual.wings"]);
      expect(r.manifest.capabilities["visual.wings"].unknown).toBe(true);
    }
    expect(validateCharacterManifest(baseManifest({ capabilities: { wings: { supported: true } } })).ok).toBe(false);
    expect(validateCharacterManifest(baseManifest({ capabilities: { "system.text": { supported: true } } })).ok).toBe(false);
    expect(validateCharacterManifest(baseManifest({ capabilities: { "visual.presence": { supported: "yes" } } })).ok).toBe(false);
    expect(validateCharacterManifest(baseManifest({ capabilities: { "visual.presence": { supported: true, durationRange: { minMs: 0, maxMs: 61_000 } } } })).ok).toBe(false);
    expect(validateCharacterManifest(baseManifest({ inputCapabilities: { "visual.presence": { supported: true } } })).ok).toBe(false);
  });

  it("preferencesSchema：只接受扁平 boolean/number/integer/string；$ref、pattern、巢狀、陣列拒絕；> 32 屬性拒絕", () => {
    const ok = validateCharacterManifest(
      baseManifest({
        preferencesSchema: {
          type: "object",
          properties: {
            volume: { type: "number", minimum: 0, maximum: 1 },
            name: { type: "string", maxLength: 20, enum: ["a", "b"] },
            on: { type: "boolean", default: true },
            count: { type: "integer" },
          },
        },
      })
    );
    expect(ok.ok).toBe(true);
    const cases: Record<string, unknown> = {
      ref: { type: "object", properties: { x: { $ref: "#/defs/x" } } },
      pattern: { type: "object", properties: { x: { type: "string", pattern: ".*" } } },
      nested: { type: "object", properties: { x: { type: "object", properties: {} } } },
      array: { type: "object", properties: { x: { type: "array" } } },
      notObject: { type: "string" },
      longString: { type: "object", properties: { x: { type: "string", maxLength: 201 } } },
      bigEnum: { type: "object", properties: { x: { type: "string", enum: Array.from({ length: 17 }, (_, i) => `e${i}`) } } },
      tooMany: {
        type: "object",
        properties: Object.fromEntries(Array.from({ length: 33 }, (_, i) => [`p${i}`, { type: "boolean" }])),
      },
    };
    for (const [name, schema] of Object.entries(cases)) {
      expect(validateCharacterManifest(baseManifest({ preferencesSchema: schema })).ok, name).toBe(false);
    }
  });

  it("錯誤訊息不回顯超長輸入、不含絕對路徑", () => {
    const evilId = "/Users/victim/secret/" + "z".repeat(300);
    const r = validateCharacterManifest(
      baseManifest({ characterId: evilId, assets: [{ id: "a", path: "/Users/victim/.ssh/id_rsa" }], entrypoint: { kind: "builtin", id: "/usr/local/bin/evil" } })
    );
    expect(r.ok).toBe(false);
    if (r.ok) return;
    for (const e of r.errors) {
      expect(e.length).toBeLessThanOrEqual(200);
      expect(e).not.toContain("/Users/");
      expect(e).not.toContain("zzzz");
    }
  });

  it("未知 intent 與 fallbacks 目標未知者忽略並記 warning", () => {
    const r = validateCharacterManifest(baseManifest({ intents: ["idle", "dance"], fallbacks: { intents: { play: "dance", sleep: "rest" } } }));
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.manifest.intents).toEqual(["idle"]);
    expect(r.manifest.fallbacks.intents).toEqual({ sleep: "rest" });
    expect(r.report.warnings.length).toBeGreaterThanOrEqual(2);
  });
});

describe("舊 pack 遷移（§2.2）", () => {
  it("character-pack 1.0（shu-standard）→ sprite adapter manifest", () => {
    const r = migratePackToManifest(shuStandard, { assetBase: "/packs/shu-standard" });
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.source).toBe("character-pack");
    expect(r.assetBase).toBe("/packs/shu-standard");
    const m = r.manifest;
    expect(m.characterId).toBe("shu-standard");
    expect(m.entrypoint).toEqual({ kind: "builtin", id: "sprite" });
    expect(m.assets[0]).toEqual({ id: "sheet", path: "sheet.png", mediaType: "image/png" });
    expect(Object.keys(m.capabilities)).toEqual(["visual.presence", "visual.expression"]);
    expect(m.capabilities["visual.expression"].variants).toEqual(Object.keys(shuStandard.animations));
    expect(Object.keys(m.inputCapabilities)).toEqual(["input.click", "input.drag", "input.drop", "input.text", "input.fileDrop"]);
    // v1 沒有 failed 美術：fallback 到 blocked（絕不是 success）。
    expect(m.intents).not.toContain("failed");
    expect(m.fallbacks.intents?.failed).toBe("blocked");
    expect(m.fallbacks.intents?.sleep).toBe("rest");
    expect(m.fallbacks.intents?.play).toBe("notice");
    expect(m.securityRequirements.executable).toBe(false);
  });

  it("character-pack 1.1（shu-lively，含 anchors）→ 多 visual.gaze，全部 intent 原生", () => {
    const r = migratePackToManifest(shuLively);
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(Object.keys(r.manifest.capabilities)).toContain("visual.gaze");
    expect(r.manifest.capabilities["visual.gaze"].reducedMotionBehavior).toBe("disabled");
    expect(r.manifest.channels).toContain("gaze");
    expect(r.manifest.intents).toHaveLength(20);
    expect(r.manifest.fallbacks.intents).toEqual({});
  });

  it("character-rig 2.0（shu-maid）→ shu-rig adapter、三個 palette variants、完整能力集、代名詞", () => {
    const r = migratePackToManifest(shuMaid, { registry: hostMigrationRegistry() });
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    const m = r.manifest;
    expect(r.source).toBe("character-rig");
    expect(m.entrypoint).toEqual({ kind: "builtin", id: "shu-rig" });
    expect(m.variants.map((v) => v.id)).toEqual(["maid-classic", "maid-dusk", "maid-sakura"]);
    expect(m.pronouns).toEqual({ "zh-TW": "她", en: "she" });
    expect(m.capabilities).toEqual(shuRigCapabilities().capabilities);
    expect(Object.keys(m.inputCapabilities)).toHaveLength(7);
    expect(m.intents).toHaveLength(20);
    expect(m.states.length).toBeGreaterThanOrEqual(36);
  });

  it("dusk rig 的第一個 variant 是自己的 palette", () => {
    const r = migratePackToManifest({ ...shuMaid, id: "shu-maid-dusk", palette: "maid-dusk" }, { registry: hostMigrationRegistry() });
    expect(r.ok && r.manifest.variants[0].id).toBe("maid-dusk");
  });

  it("壞的舊 pack 被既有驗證器擋下", () => {
    expect(migratePackToManifest({ kind: "character-pack", id: "x" }).ok).toBe(false);
    expect(migratePackToManifest({ kind: "character-rig", id: "x", palette: "evil", name: {} }).ok).toBe(false);
    // 有正確 schemaVersion 也一樣：分派到 shu 的 rig migrator，仍被舊 rig 驗證器擋下。
    expect(
      migratePackToManifest({ schemaVersion: "2.0", kind: "character-rig", id: "x", palette: "evil", name: {} }, { registry: hostMigrationRegistry() }).ok
    ).toBe(false);
    expect(migratePackToManifest({ kind: "persona-pack" }).ok).toBe(false);
    expect(migratePackToManifest(null).ok).toBe(false);
  });

  it("sprite intent 對照：claimed 只點頭、verified 才完整；安全 intent 不落到 success", () => {
    const anims = shuStandard.animations;
    expect(resolveSpriteAnimation(anims, "claim-completed", { truthState: "claimed" })).toMatchObject({ animation: "success", frameSlice: [0, 1] });
    expect(resolveSpriteAnimation(anims, "verified-success", { truthState: "verified" })).toMatchObject({ animation: "success", frameSlice: undefined });
    expect(resolveSpriteAnimation(anims, "verified-success", { truthState: "claimed" })).toMatchObject({ animation: "success", frameSlice: [0, 1] });
    expect(resolveSpriteAnimation(anims, "failed")).toMatchObject({ animation: "blocked", direct: false });
    expect(resolveSpriteAnimation(shuLively.animations, "failed")).toMatchObject({ animation: "failed", direct: true });
    for (const intent of ["emergency", "offline", "blocked", "unknown", "failed"] as const) {
      expect(resolveSpriteAnimation(anims, intent)?.animation).not.toBe("success");
      // 提示 variant 不能改寫安全 intent 的動畫
      expect(resolveSpriteAnimation(anims, intent, { variant: "success" })?.animation).not.toBe("success");
    }
    expect(resolveSpriteAnimation(shuLively.animations, "notice", { variant: "listening" })?.animation).toBe("listening");
    expect(deriveIntentFallbacks(anims)).toEqual({ failed: "blocked", play: "notice", sleep: "rest" });
  });

  it("displayNameOf／pronounOf 的中立 fallback", () => {
    const r = migratePackToManifest(shuMaid, { registry: hostMigrationRegistry() });
    if (!r.ok) throw new Error("migration failed");
    expect(displayNameOf(r.manifest, "zh-TW")).toBe(shuMaid.name["zh-TW"]);
    expect(displayNameOf(r.manifest, "ja")).toBe(shuMaid.name["zh-TW"]);
    expect(pronounOf(r.manifest, "zh-TW")).toBe("她");
    expect(pronounOf(r.manifest, "en-US")).toBe("she");
    expect(pronounOf(r.manifest, "ja")).toBe("they");
    expect(pronounOf({ pronouns: undefined }, "zh-TW")).toBe("角色");
    expect(pronounOf({ pronouns: undefined }, "en")).toBe("they");
    expect(displayNameOf({ displayName: {} }, "en")).toBe("角色");
  });
});

describe("協商（§3.3／§3.4）", () => {
  it("協定 major 不同一律拒絕（typed error），不猜", () => {
    expect(() => negotiate(hello(), offer({ protocolVersion: "2.0" }))).toThrow(ProtocolVersionError);
    expect(() => negotiate(hello(), offer({ protocolVersion: "garbage" }))).toThrow(ProtocolVersionError);
    try {
      negotiate(hello(), offer({ protocolVersion: "2.3" }));
    } catch (e) {
      expect((e as ProtocolVersionError).code).toBe("protocol-version");
      expect((e as ProtocolVersionError).offered).toBe("2.3");
    }
    // 同 major、較新 minor 相容
    expect(negotiate(hello(), offer({ protocolVersion: "1.9" })).resolutions.idle.resolution).toBe("exact");
  });

  it("覆蓋全部 20 個 intent；全能力角色一律 exact", () => {
    const n = negotiate(hello(), offer());
    expect(Object.keys(n.resolutions)).toHaveLength(20);
    for (const intent of CHARACTER_INTENTS) {
      expect(n.resolutions[intent].resolution, intent).toBe("exact");
    }
    expect(n.resolutions.idle.via).toBe("visual.presence");
    expect(n.resolutions.notice.via).toBe("visual.expression");
    expect(n.capabilities["system.text"].supported).toBe(true);
    expect(n.generation).toBe(0);
    expect(n.characterInstanceId).toBe("inst-1");
  });

  it("沒有 expression 的角色（presence＋textBubble）→ 原生 intent 走 textBubble exact", () => {
    const n = negotiate(
      hello(),
      offer({ capabilities: { "visual.presence": { supported: true }, "visual.textBubble": { supported: true } } })
    );
    expect(n.resolutions.notice).toEqual({ resolution: "exact", via: "visual.textBubble" });
    expect(n.resolutions.emergency.via).toBe("visual.textBubble");
  });

  it("純聲音角色：原生 intent 走 audio.effect；非原生走能力鏈 substituted；鏈外安全 intent → system.text", () => {
    const n = negotiate(
      hello(),
      offer({
        capabilities: { "audio.effect": { supported: true } },
        intents: ["notice", "emergency"],
        fallbacks: { capabilities: { "visual.expression": ["audio.effect"] } },
      })
    );
    expect(n.resolutions.notice).toEqual({ resolution: "exact", via: "audio.effect" });
    expect(n.resolutions.emergency).toEqual({ resolution: "exact", via: "audio.effect" });
    expect(n.resolutions.think).toEqual({ resolution: "substituted", via: "audio.effect" });
    // idle 的主要能力是 visual.presence，沒有鏈 → 非安全 → unsupported
    expect(n.resolutions.idle).toEqual({ resolution: "unsupported" });
    const noChain = negotiate(hello(), offer({ capabilities: { "audio.effect": { supported: true } }, intents: [] }));
    expect(noChain.resolutions.blocked).toEqual({ resolution: "substituted", via: "system.text" });
    expect(noChain.resolutions.play).toEqual({ resolution: "unsupported" });
  });

  it("零能力角色：所有安全 intent → system.text substituted；其餘 unsupported", () => {
    const n = negotiate(hello(), offer({ capabilities: {}, intents: [] }));
    for (const intent of CHARACTER_INTENTS) {
      if (isSafetyIntent(intent)) expect(n.resolutions[intent]).toEqual({ resolution: "substituted", via: "system.text" });
      else expect(n.resolutions[intent]).toEqual({ resolution: "unsupported" });
    }
  });

  it("fallbacks.intents 只換一次，via 記錄實際 intent；variant 只在能力宣告時出現", () => {
    const n = negotiate(
      hello(),
      offer({
        capabilities: { "visual.expression": { supported: true, variants: ["notice", "blocked"] } },
        intents: ["notice", "blocked"],
        fallbacks: { intents: { failed: "blocked", play: "notice", sleep: "play" } },
      })
    );
    expect(n.resolutions.failed).toEqual({ resolution: "substituted", via: "visual.expression", viaIntent: "blocked", variant: "blocked" });
    expect(n.resolutions.play).toEqual({ resolution: "substituted", via: "visual.expression", viaIntent: "notice", variant: "notice" });
    // sleep → play（非原生）不能再鏈到 notice
    expect(n.resolutions.sleep).toEqual({ resolution: "unsupported" });
    expect(n.resolutions.notice.variant).toBe("notice");
    expect(n.resolutions.emergency).toEqual({ resolution: "substituted", via: "system.text" });
  });

  it("reduced motion：static/reduced → reduced；disabled → 往下一個 fallback；unchanged 不變", () => {
    const rm = hello({ reducedMotion: true });
    const n1 = negotiate(rm, offer());
    expect(n1.reducedMotion).toBe(true);
    expect(n1.resolutions.notice).toEqual({ resolution: "reduced", via: "visual.expression" });
    expect(n1.resolutions.idle).toEqual({ resolution: "exact", via: "visual.presence" }); // 無宣告 = unchanged
    const n2 = negotiate(
      rm,
      offer({
        capabilities: {
          "visual.expression": { supported: true, reducedMotionBehavior: "disabled" },
          "visual.textBubble": { supported: true, reducedMotionBehavior: "unchanged" },
        },
        intents: ["notice"],
        fallbacks: { capabilities: { "visual.expression": ["visual.textBubble"] } },
      })
    );
    // 原生 notice 的候選能力中 expression 停用，textBubble 仍在候選清單 → exact via textBubble
    expect(n2.resolutions.notice).toEqual({ resolution: "exact", via: "visual.textBubble" });
    expect(n2.capabilities["visual.expression"]).toBeUndefined();
    const n3 = negotiate(rm, offer({ capabilities: { "visual.expression": { supported: true, reducedMotionBehavior: "disabled" } }, intents: ["notice"] }));
    expect(n3.resolutions.notice).toEqual({ resolution: "unsupported" });
    expect(n3.resolutions.emergency).toEqual({ resolution: "substituted", via: "system.text" });
    expect(negotiate(hello({ reducedMotion: false }), offer()).capabilities["visual.expression"].qualityLevel).toBeUndefined();
    expect(n1.capabilities["visual.expression"].qualityLevel).toBe("minimal");
  });

  it("unknown custom channel：namespaced 進 accepted 並標 nonSafety；非 namespaced 進 ignored", () => {
    const c = classifyChannels(["expression", "com.example.character.wings", "weird", "expression", "gaze"]);
    expect(c.accepted).toEqual(["expression", "com.example.character.wings", "gaze"]);
    expect(c.nonSafety).toEqual(["com.example.character.wings"]);
    expect(c.ignored).toEqual(["weird"]);
    const n = negotiate(hello(), offer({ channels: ["expression", "com.example.character.wings", "weird"] }));
    expect(n.acceptedChannels).toContain("com.example.character.wings");
    expect(n.nonSafetyChannels).toEqual(["com.example.character.wings"]);
    expect(n.ignoredChannels).toEqual(["weird"]);
    // custom channel 不改變任何安全 intent 的解析
    expect(n.resolutions.emergency.resolution).toBe("exact");
  });

  it("未知能力視為 custom：可作 fallback 鏈目標", () => {
    const n = negotiate(
      hello(),
      offer({
        capabilities: { "com.example.character.wings": { supported: true } },
        intents: [],
        fallbacks: { capabilities: { "visual.expression": ["com.example.character.wings"] } },
      })
    );
    expect(n.resolutions.notice).toEqual({ resolution: "substituted", via: "com.example.character.wings" });
  });

  it("確定性：同輸入多次結果完全相同，且與 resolveIntent 逐一結果一致", () => {
    const o = offer({
      capabilities: { "visual.expression": { supported: true, reducedMotionBehavior: "reduced" }, "audio.effect": { supported: true } },
      intents: ["idle", "notice", "work"],
      fallbacks: { intents: { think: "work", greet: "notice" }, capabilities: { "visual.expression": ["audio.effect"] } },
    });
    const a = negotiate(hello({ reducedMotion: true }), o);
    const b = negotiate(hello({ reducedMotion: true }), o);
    expect(a).toEqual(b);
    for (const intent of CHARACTER_INTENTS) {
      expect(resolveIntent(intent, o, o.fallbacks!, true)).toEqual(a.resolutions[intent]);
    }
    expect(a.resolutions.think).toEqual({ resolution: "reduced", via: "visual.expression", viaIntent: "work" });
  });
});

// character-protocol-041：Rust 是權威、TS 是鏡射。兩邊各自維護 intent→能力表時會靜默漂移，
// 同一份 manifest 在 Runtime gateway 與視窗 gateway 得到不同的 resolution／via。
// golden 由 Rust 產生（UPDATE_CPP_GOLDEN=1 cargo test -p interaction-character
// --test intent_capabilities_golden），這裡對它逐項斷言。
describe("§3.4 intent→能力表：TS 鏡射必須與 Rust 權威逐字相同", () => {
  const goldenPath = path.resolve("../../crates/interaction-character/tests/golden/intent-capabilities.json");
  const golden = JSON.parse(fs.readFileSync(goldenPath, "utf8")) as Record<string, string[]>;

  it("20 個 intent 全部在 golden 裡", () => {
    expect(Object.keys(golden).sort()).toEqual([...CHARACTER_INTENTS].sort());
  });

  it("每個 intent 的候選能力清單與順序都相同", () => {
    for (const intent of CHARACTER_INTENTS) {
      expect([...INTENT_CAPABILITIES[intent]], `intent ${intent} 的能力清單與 Rust 權威不符`).toEqual(
        golden[intent]
      );
    }
  });

  it("同一份「只有 LED／particle／overlay／玩具能力」的 manifest 解析結果與 Rust 相同", () => {
    // 這正是 Rust 與 TS 曾經分歧的 6 個 intent（verified-success／offline／emergency／play／rest／sleep）。
    const o = offer({
      capabilities: {
        "visual.presence": { supported: true },
        "visual.particles": { supported: true },
        "visual.overlay": { supported: true },
        "gameplay.toys": { supported: true },
        "visual.locomotion": { supported: true },
      },
      intents: [...CHARACTER_INTENTS],
      fallbacks: {},
    });
    const n = negotiate(hello(), o);
    expect(n.resolutions["verified-success"]).toEqual({ resolution: "exact", via: "visual.particles" });
    expect(n.resolutions.offline).toEqual({ resolution: "exact", via: "visual.presence" });
    expect(n.resolutions.emergency).toEqual({ resolution: "exact", via: "visual.overlay" });
    expect(n.resolutions.play).toEqual({ resolution: "exact", via: "gameplay.toys" });
    expect(n.resolutions.rest).toEqual({ resolution: "exact", via: "visual.presence" });
    expect(n.resolutions.sleep).toEqual({ resolution: "exact", via: "visual.presence" });
  });
});

describe("安全 intent 不得被 fallbacks.intents 換成非安全 intent（§3.4 步驟 2 守衛）", () => {
  it("manifest 驗證擋下安全 → 非安全的映射，安全 → 安全與非安全映射仍合法", () => {
    for (const [from, to] of [
      ["request-consent", "greet"],
      ["blocked", "play"],
      ["emergency", "sleep"],
      ["offline", "idle"],
      ["wait", "think"],
    ]) {
      const r = validateCharacterManifest(baseManifest({ fallbacks: { intents: { [from]: to } } }));
      expect(r.ok, `${from} → ${to} 必須被拒`).toBe(false);
      if (r.ok) continue;
      expect(r.errors.join(" ")).toContain(from);
    }
    for (const [from, to] of [
      ["failed", "blocked"],
      ["request-consent", "ask"],
      ["play", "notice"],
      ["sleep", "rest"],
    ]) {
      const r = validateCharacterManifest(baseManifest({ fallbacks: { intents: { [from]: to } } }));
      expect(r.ok, `${from} → ${to} 應該合法`).toBe(true);
      if (!r.ok) continue;
      expect(r.manifest.fallbacks.intents?.[from as never]).toBe(to);
    }
  });

  it("協商：adapter 自帶的惡意 fallback 也換不掉安全語意，誠實落到 system.text", () => {
    const n = negotiate(
      hello(),
      offer({
        capabilities: { "visual.expression": { supported: true, variants: ["greet", "play", "blocked"] } },
        intents: ["idle", "greet", "play", "blocked"],
        fallbacks: {
          intents: {
            "request-consent": "greet",
            offline: "play",
            unknown: "greet",
            emergency: "play",
            failed: "blocked",
          },
        },
      })
    );
    for (const intent of SAFETY_INTENTS) {
      const via = n.resolutions[intent].viaIntent;
      if (via) expect(isSafetyIntent(via), `${intent} → ${via}`).toBe(true);
    }
    for (const intent of ["request-consent", "offline", "unknown", "emergency"] as const) {
      expect(n.resolutions[intent]).toEqual({ resolution: "substituted", via: "system.text" });
    }
    // 安全 → 安全（failed → blocked）不受影響。
    expect(n.resolutions.failed).toEqual({ resolution: "substituted", via: "visual.expression", viaIntent: "blocked", variant: "blocked" });
  });
});

// ---------------------------------------------------------------------------
// §2.2 遷移器 registry（v0.6.0 strangler）：核心只內建通用 sprite，具名角色的舊格式
// 由它自己的 adapter 模組實作 PackMigrator、由 host 註冊。鏡射 Rust 的
// interaction_character::{PackMigrator, MigrationRegistry}。
// ---------------------------------------------------------------------------

describe("遷移器 registry（§2.2）", () => {
  function fakeMigrator(kind: string, versions: readonly string[]): PackMigrator {
    return {
      kind,
      schemaVersions: versions,
      migrate: () => ({ ok: false as const, errors: ["not implemented"] }),
    };
  }

  it("核心 registry 只有通用 sprite（character-pack 1.0／1.1），沒有任何具名角色", () => {
    const core = coreMigrationRegistry();
    expect(core.supportedKinds()).toEqual([
      { kind: "character-pack", schemaVersion: "1.0" },
      { kind: "character-pack", schemaVersion: "1.1" },
    ]);
    expect(core.size).toBe(1);
    expect(core.find("character-pack", "1.0")).toBe(spritePackMigrator);
    expect(core.find("character-rig", "2.0")).toBeNull();
  });

  it("核心 registry 遷移 sprite pack 成功、遇到 rig pack 誠實失敗（不猜、不落到別的 adapter）", () => {
    const core = coreMigrationRegistry();
    expect(migratePackToManifest(shuStandard, { registry: core }).ok).toBe(true);
    const rig = migratePackToManifest(shuMaid, { registry: core });
    expect(rig.ok).toBe(false);
    if (rig.ok) return;
    expect(rig.errors.join(" ")).toMatch(/no migrator/i);
  });

  it("同一組 (kind, schemaVersion) 不得註冊兩次（後者不得悄悄覆蓋前者）", () => {
    const reg = new MigrationRegistry().register(fakeMigrator("demo-pack", ["1.0", "1.1"]));
    expect(() => reg.register(fakeMigrator("demo-pack", ["1.1"]))).toThrow(/already registered/);
    // 不同版本可以另外註冊
    expect(() => reg.register(fakeMigrator("demo-pack", ["2.0"]))).not.toThrow();
    expect(reg.size).toBe(2);
  });

  it("registry 有界：migrator 數量與每個 migrator 的版本數都有上限", () => {
    expect(() => new MigrationRegistry().register(fakeMigrator("demo", []))).toThrow(/schema versions/);
    const tooMany = Array.from({ length: MAX_MIGRATOR_VERSIONS + 1 }, (_, i) => `1.${i}`);
    expect(() => new MigrationRegistry().register(fakeMigrator("demo", tooMany))).toThrow(/schema versions/);
    const reg = new MigrationRegistry();
    for (let i = 0; i < MAX_MIGRATORS; i += 1) reg.register(fakeMigrator(`demo-${i}`, ["1.0"]));
    expect(reg.size).toBe(MAX_MIGRATORS);
    expect(() => reg.register(fakeMigrator("overflow", ["1.0"]))).toThrow(/full/);
  });

  it("沒有 migrator 的 kind 誠實回錯，訊息不回顯輸入內容", () => {
    const evil = "x".repeat(500);
    const r = migratePackToManifest({ kind: evil, schemaVersion: evil, secret: "/Users/someone/private" });
    expect(r.ok).toBe(false);
    if (r.ok) return;
    const message = r.errors.join(" ");
    expect(message).not.toContain(evil);
    expect(message).not.toContain("/Users/");
    expect(message.length).toBeLessThanOrEqual(200);
  });

  it("host registry ＝ 核心 sprite ＋ shu adapter 註冊的 character-rig 2.0", () => {
    const host = hostMigrationRegistry();
    expect(host.supportedKinds()).toEqual([
      { kind: "character-pack", schemaVersion: "1.0" },
      { kind: "character-pack", schemaVersion: "1.1" },
      { kind: "character-rig", schemaVersion: "2.0" },
    ]);
    expect(host.find("character-rig", "2.0")).toBe(rigPackMigrator);
  });

  it("rig 走 shu migrator 產生與 v0.5.1 相同的 manifest（golden）", () => {
    const viaRegistry = migratePackToManifest(shuMaid, { registry: hostMigrationRegistry() });
    const viaMigrator = rigPackMigrator.migrate(shuMaid as unknown as Record<string, unknown>, {});
    expect(viaRegistry.ok).toBe(true);
    expect(viaMigrator).toEqual(viaRegistry);
    if (!viaRegistry.ok) return;
    const m = viaRegistry.manifest;
    expect(viaRegistry.source).toBe("character-rig");
    expect(m.entrypoint).toEqual({ kind: "builtin", id: "shu-rig" });
    expect(m.variants).toEqual([
      { id: "maid-classic", displayName: { "zh-TW": "經典", en: "Classic" } },
      { id: "maid-dusk", displayName: { "zh-TW": "暮色", en: "Dusk" } },
      { id: "maid-sakura", displayName: { "zh-TW": "櫻花", en: "Sakura" } },
    ]);
    expect(Object.keys(m.capabilities)).toEqual([
      "visual.presence",
      "visual.pose",
      "visual.expression",
      "visual.gaze",
      "visual.locomotion",
      "visual.overlay",
      "visual.particles",
      "visual.prop",
      "visual.textBubble",
      "audio.speech",
      "audio.effect",
      "multiCharacter",
      "scene",
      "rollCall",
      "gameplay.toys",
      "gameplay.autonomy",
    ]);
    expect(Object.keys(m.inputCapabilities)).toEqual([
      "input.click",
      "input.hover",
      "input.drag",
      "input.drop",
      "input.pointerProximity",
      "input.text",
      "input.fileDrop",
    ]);
    expect(m.channels).toEqual([...SEMANTIC_CHANNELS]);
    expect(m.intents).toEqual([...CHARACTER_INTENTS]);
    expect(m.fallbacks).toEqual({});
    expect((m as unknown as { legacy?: unknown }).legacy).toEqual({
      kind: "character-rig",
      schemaVersion: "2.0",
      palette: "maid-classic",
    });
    expect(m.securityRequirements).toEqual({
      network: false,
      executable: false,
      fileAccess: "none",
      audioOutput: true,
      microphone: false,
      camera: false,
    });
    expect(m.resourceLimits).toEqual({ maxAssetBytes: 8 * 1024 * 1024, maxConcurrentCommands: 4, maxQueue: 32, maxFps: 60 });
    expect(m.pronouns).toEqual({ "zh-TW": "她", en: "she" });
    expect(m.states.length).toBeGreaterThanOrEqual(36);
  });

  it("host 注入的預設 registry：不帶 registry 的呼叫端也能遷移 rig（adapters/index.ts 的載入副作用）", () => {
    expect(defaultMigrationRegistry().find("character-rig", "2.0")).toBe(rigPackMigrator);
    expect(migratePackToManifest(shuMaid).ok).toBe(true);
  });
});
