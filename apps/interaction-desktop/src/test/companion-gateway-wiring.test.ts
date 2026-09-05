// CompanionApp ↔ CharacterGateway ↔ Runtime 的接線（純函式）：角色選擇（索引＋8 個舊 id）、
// 載入失敗退回文字角色、character.intent 的 targets 過濾、回執轉送（去掉 @instance 後綴、
// 帶 Runtime 世代、只送主角）、legacy／protocol feed 選擇、本機互動 → CPP 輸入事件、
// MixerRenderer 門面、machineEventForAnimation、settingsTransfer 的 characterId 別名、
// DEFAULT_LINES 的 `{name}` 樣板。全部不碰 React、不起 daemon。

import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import shuMaidRaw from "../../public/characters/shu-maid/manifest.json";
import type { AdapterHost, CharacterAdapter } from "../character/adapter";
import { TextCharacterAdapter, buildTextCharacterManifest } from "../character/adapters/text";
import { CharacterGateway } from "../character/gateway";
import { validateCharacterManifest } from "../character/manifest";
import { CommandReceipt, PROTOCOL_VERSION } from "../character/protocol";
import { builtinAdapterMeta } from "../character/adapterRegistry";
import type { CharacterIndex, CharacterIndexEntry } from "../character/registry";
import {
  CHARACTER_LOAD_FAILED_LINE,
  charNameFor,
  cssClassForEntrypoint,
  entrypointKindOf,
  envelopeForInstance,
  helloFor,
  inputEventFor,
  isLocalOnlyMessageId,
  personaIdFor,
  PRIMARY_INSTANCE_ID,
  receiptForRuntime,
  selectCharacterSource,
  selectRuntimeFeed,
  storyPackIdFor,
  systemTextFromEvent,
} from "../companion/gatewayWiring";
import { rigPaletteFor } from "../character/adapters/shu";
import { machineEventForAnimation, MachineEvent, MachineState, mapRuntimeEvent, reduce } from "../companion/machine";
import { MixerRenderer } from "../companion/mixerRenderer";
import type { RendererBackend } from "../companion/renderer";
import { applyLineVars, DEFAULT_LINES, resolveLine } from "../companion/packs";
import {
  exportCompanionSettings,
  isImportableCharacterId,
  LEGACY_CHARACTER_IDS,
  parseCompanionSettingsImport,
} from "../companion/settingsTransfer";
import type { DesktopPrefs } from "../desktop";

function shuManifest() {
  const v = validateCharacterManifest(shuMaidRaw);
  if (!v.ok) throw new Error(v.errors.join("; "));
  return v.manifest;
}

function entry(characterId: string, manifest = shuManifest(), extra: Partial<CharacterIndexEntry> = {}): CharacterIndexEntry {
  return {
    characterId,
    manifestPath: `/characters/${characterId}/manifest.json`,
    origin: "builtin",
    manifest: { ...manifest, characterId },
    report: { newerMinor: false, unknownCapabilities: [], customCapabilities: [], warnings: [], flags: { external: false, network: false, executable: false, unsigned: true } },
    ...extra,
  };
}

function index(chars: CharacterIndexEntry[], def = "shu-maid"): CharacterIndex {
  return { schemaVersion: "1.0", default: def, characters: chars, errors: [] };
}

const textEntry = entry("plain-text", buildTextCharacterManifest());

describe("feed 選擇：daemon 有 characterProtocol 才走 CPP", () => {
  it("status.characterProtocol 物件 → protocol；缺席／非物件／null → legacy", () => {
    expect(selectRuntimeFeed({ characterProtocol: { version: "1.0", instances: 1, activeCharacter: null } })).toBe("protocol");
    expect(selectRuntimeFeed({ emergencyStop: false })).toBe("legacy");
    expect(selectRuntimeFeed({ characterProtocol: "1.0" })).toBe("legacy");
    expect(selectRuntimeFeed(null)).toBe("legacy");
    expect(selectRuntimeFeed(undefined)).toBe("legacy");
  });
});

describe("角色選擇：索引＋prefs.companionPack；8 個舊 id 永遠有效", () => {
  const idx = index([entry("shu-maid"), entry("shu-lazy"), textEntry]);

  it("偏好在索引裡 → 該項目；不在索引但是舊 id → legacy pack；未知 → 索引 default", () => {
    expect(selectCharacterSource(idx, "shu-lazy")).toMatchObject({ kind: "index", characterId: "shu-lazy" });
    expect(selectCharacterSource(idx, "shu-agile")).toEqual({ kind: "legacy-pack", characterId: "shu-agile" });
    expect(selectCharacterSource(idx, "totally-new")).toMatchObject({ kind: "index", characterId: "shu-maid" });
    expect(selectCharacterSource(idx, null)).toMatchObject({ kind: "index", characterId: "shu-maid" });
    expect(selectCharacterSource(idx, "plain-text")).toMatchObject({ kind: "index", characterId: "plain-text" });
  });

  it("偏好是 host 匯入清單裡的角色 → imported；清單沒問過（省略／null）時維持原本的規則", () => {
    const fox = {
      characterId: "fox-text",
      valid: true,
      displayName: { "zh-TW": "小狐" },
      adapterKind: "in-process",
      entrypoint: "text",
      executable: false,
      network: false,
      external: false,
      assets: [],
      origin: "imported" as const,
    };
    expect(selectCharacterSource(idx, "fox-text", [fox])).toMatchObject({ kind: "imported", characterId: "fox-text", entrypoint: "text" });
    expect(selectCharacterSource(idx, "fox-text")).toMatchObject({ kind: "index", characterId: "shu-maid" });
    expect(selectCharacterSource(idx, "fox-text", null)).toMatchObject({ kind: "index", characterId: "shu-maid" });
    // 索引命中與舊 id 永遠優先於同名的匯入項目（匯入不能冒充內建）。
    expect(selectCharacterSource(idx, "shu-maid", [{ ...fox, characterId: "shu-maid" }])).toMatchObject({ kind: "index", characterId: "shu-maid" });
    expect(selectCharacterSource(idx, "shu-agile", [{ ...fox, characterId: "shu-agile" }])).toEqual({ kind: "legacy-pack", characterId: "shu-agile" });
  });

  it("索引載入失敗：舊 id → legacy pack；其他 → 文字角色（並說明原因）", () => {
    for (const id of LEGACY_CHARACTER_IDS) {
      expect(selectCharacterSource(null, id)).toEqual({ kind: "legacy-pack", characterId: id });
    }
    expect(selectCharacterSource(null, "evil-pack")).toMatchObject({ kind: "text", characterId: "plain-text" });
    expect(selectCharacterSource(null, null)).toMatchObject({ kind: "text", reason: "character index unavailable" });
    // 索引存在但 default 壞掉、偏好也不認得 → 文字角色。
    expect(selectCharacterSource(index([textEntry], "missing"), "nope")).toMatchObject({ kind: "text" });
  });

  it("entrypoint 種類決定 CSS class（不再看 pack id 前綴）與 rig 配色", () => {
    expect(entrypointKindOf(shuManifest())).toBe("shu-rig");
    expect(entrypointKindOf(buildTextCharacterManifest())).toBe("text");
    expect(entrypointKindOf({ entrypoint: { kind: "process", command: ["x"] } })).toBeNull();
    expect(entrypointKindOf(null)).toBeNull();
    expect(cssClassForEntrypoint("shu-rig")).toBe("companion-stage");
    expect(cssClassForEntrypoint("sprite")).toBe("companion-canvas");
    expect(cssClassForEntrypoint("text")).toBe("companion-text");
    expect(cssClassForEntrypoint(null)).toBe("companion-canvas");
    expect(rigPaletteFor(shuManifest())).toBe("maid-classic");
    expect(rigPaletteFor({ ...shuManifest(), legacy: undefined, variants: [{ id: "maid-dusk" }] } as never)).toBe("maid-dusk");
  });

  it("persona／story：索引提示優先，否則 prefs 的 persona；故事由 persona id 派生；非法 id 不採用", () => {
    expect(personaIdFor(null, "persona-shu")).toBe("persona-shu");
    expect(personaIdFor({ persona: "persona-navigator" }, "persona-shu")).toBe("persona-navigator");
    expect(personaIdFor({ persona: "../evil" }, "persona-shu")).toBe("persona-shu");
    expect(personaIdFor(null, "../evil")).toBeNull();
    expect(personaIdFor(entry("shu-maid"), null)).toBeNull();
    expect(storyPackIdFor(null, "persona-shu")).toBe("story-shu-intro");
    expect(storyPackIdFor({ story: "story-custom" }, "persona-shu")).toBe("story-custom");
    expect(storyPackIdFor(null, "persona-navigator")).toBe("story-navigator-intro");
    expect(storyPackIdFor(null, null)).toBeNull();
  });

  it("顯示名：使用者取的名字優先，否則 manifest displayName（locale），沒有 manifest 才是中立的「角色」", () => {
    expect(charNameFor("阿樞 ", shuManifest(), "zh-TW")).toBe("阿樞");
    expect(charNameFor("", shuManifest(), "zh-TW")).toBe("小樞");
    expect(charNameFor(null, shuManifest(), "en")).toBe("Shu");
    expect(charNameFor(null, buildTextCharacterManifest(), "zh-TW")).toBe("文字角色");
    expect(charNameFor(null, null, "zh-TW")).toBe("角色");
    expect(charNameFor("x".repeat(40), null, "zh-TW")).toHaveLength(24);
  });
});

describe("載入失敗退回文字角色", () => {
  const host: AdapterHost = { now: () => 0, reducedMotion: () => false, locale: "zh-TW", log: () => {} };

  it("adapter initialize 擲例外 → registerInstance 拒絕、實例釋放；同一 instanceId 可改註冊文字角色；固定文案由 host 顯示", async () => {
    const lifecycle: string[] = [];
    const gw = new CharacterGateway({ now: () => 0, onSystemText: () => {}, onLifecycle: (_id, s) => lifecycle.push(s) });
    const broken: CharacterAdapter = {
      manifest: shuManifest(),
      initialize: async () => {
        throw new Error("rig blew up");
      },
      negotiate: () => {
        throw new Error("unreachable");
      },
      show() {},
      hide() {},
      suspend() {},
      resume() {},
      reconfigure() {},
      perform() {},
      cancel() {},
      dispose() {},
      onInput: () => () => {},
    };
    await expect(gw.registerInstance(broken, "primary-companion", { instanceId: PRIMARY_INSTANCE_ID })).rejects.toThrow(/rig blew up/);
    expect(lifecycle).toContain("crashed");
    expect(gw.getInstance(PRIMARY_INSTANCE_ID)).toBeNull();
    const text = new TextCharacterAdapter();
    await text.initialize(host);
    const { negotiated } = await gw.registerInstance(text, "primary-companion", { instanceId: PRIMARY_INSTANCE_ID });
    expect(negotiated.resolutions.emergency.resolution).toBe("exact");
    expect(gw.getInstance(PRIMARY_INSTANCE_ID)?.characterId).toBe("plain-text");
    // 文案是 host 常數，不來自任何 adapter。
    expect(CHARACTER_LOAD_FAILED_LINE).toBe("角色載入失敗，改用文字顯示");
    expect(JSON.stringify(text.manifest)).not.toContain(CHARACTER_LOAD_FAILED_LINE);
  });
});

describe("character.intent 的 targets 過濾", () => {
  const envelope = {
    protocolVersion: PROTOCOL_VERSION,
    messageId: "m1",
    characterInstanceId: "runtime-side-id",
    timestamp: "2026-09-02T00:00:00Z",
    intent: "work",
    truthState: "working",
    priority: 40,
    interruptPolicy: "queue",
    resumePolicy: "none",
    privacyClass: "internal",
  };

  it("targets 含本機 instanceId 或通用 desktop-companion → 派送，並把 characterInstanceId 對齊本機", () => {
    const own = envelopeForInstance({ envelope, targets: ["desktop-companion"] }, "desktop-companion");
    expect(own?.characterInstanceId).toBe("desktop-companion");
    expect(own?.intent).toBe("work");
    const alias = envelopeForInstance({ envelope, targets: [PRIMARY_INSTANCE_ID] }, "win-2");
    expect(alias?.characterInstanceId).toBe("win-2");
    expect(envelopeForInstance({ envelope, targets: ["win-2", "other"] }, "win-2")).not.toBeNull();
  });

  it("targets 不含我們、缺 envelope、未知 intent、缺 messageId → 不派送", () => {
    expect(envelopeForInstance({ envelope, targets: ["other"] }, "desktop-companion")).toBeNull();
    expect(envelopeForInstance({ envelope, targets: [] }, "desktop-companion")).toBeNull();
    expect(envelopeForInstance({ targets: ["desktop-companion"] }, "desktop-companion")).toBeNull();
    expect(envelopeForInstance({ envelope: { ...envelope, intent: "teleport" }, targets: ["desktop-companion"] }, "desktop-companion")).toBeNull();
    expect(envelopeForInstance({ envelope: { ...envelope, messageId: 7 }, targets: ["desktop-companion"] }, "desktop-companion")).toBeNull();
    expect(envelopeForInstance(null, "desktop-companion")).toBeNull();
  });
});

describe("回執轉送：只送主角、去掉 @instance 後綴、帶 Runtime 世代、本機派生命令不送", () => {
  const receipt = (over: Partial<CommandReceipt>): CommandReceipt => ({
    messageId: "m1@desktop-companion",
    characterInstanceId: "desktop-companion",
    generation: 1,
    status: "completed",
    resolution: "exact",
    at: "2026-09-02T00:00:00Z",
    ...over,
  });

  it("廣播後綴被去掉、generation 換成 Runtime 的；沒有 Runtime 世代就保留本機的", () => {
    expect(receiptForRuntime(receipt({}), "desktop-companion", 3)).toMatchObject({ messageId: "m1", generation: 3, status: "completed" });
    expect(receiptForRuntime(receipt({ messageId: "m1" }), "desktop-companion", null)?.generation).toBe(1);
  });

  it("其他實例的回執不送；resume／return-idle 派生命令不送", () => {
    expect(receiptForRuntime(receipt({ characterInstanceId: "familiar-1" }), "desktop-companion", 3)).toBeNull();
    expect(receiptForRuntime(receipt({ messageId: "m1~idle" }), "desktop-companion", 3)).toBeNull();
    expect(receiptForRuntime(receipt({ messageId: "m1~r2-m9@desktop-companion" }), "desktop-companion", 3)).toBeNull();
    expect(isLocalOnlyMessageId("m1")).toBe(false);
    expect(isLocalOnlyMessageId("m1~idle")).toBe(true);
  });

  it("hello：20 個 intent、協定 1.0、限制同 LIMITS、reducedMotion 由 host 決定", () => {
    const h = helloFor("desktop-companion", "primary-companion", { reducedMotion: true });
    expect(h.type).toBe("hello");
    expect(h.protocolVersion).toBe("1.0");
    expect(h.requires).toHaveLength(20);
    expect(h.reducedMotion).toBe(true);
    expect(h.limits).toEqual({ maxMessageBytes: 65536, maxMessagesPerSecond: 50, maxPending: 64 });
    expect(h.locale).toBe("zh-TW");
  });

  it("Reduced Motion 只有一個主人：視窗的值一路送進 /v1/character/hello（TS 與 Runtime 協商同一個值）", () => {
    // 視窗 → api → Tauri bridge → Runtime：任何一段掉了，Runtime 就會永遠以 false 協商，
    // 把實際 reduced 的演出記成 exact。
    const app = fs.readFileSync(path.resolve("src/companion/CompanionApp.tsx"), "utf8");
    const call = app.slice(app.indexOf("api.characterHello({"));
    const body = call.slice(0, call.indexOf("});"));
    expect(body).toContain("reducedMotion: reducedMotionRef.current");
    // 同一個值也給本機 adapter 協商（helloFor），兩邊不會各算各的。
    const sendHello = app.slice(app.indexOf("const sendHello = React.useCallback"));
    expect(sendHello.slice(0, sendHello.indexOf("api.characterHello("))).toContain(
      "reducedMotion: reducedMotionRef.current"
    );

    const api = fs.readFileSync(path.resolve("src/api.ts"), "utf8");
    const invokeHello = api.slice(api.indexOf('invoke<CharacterHelloResult>("character_hello"'));
    expect(invokeHello.slice(0, invokeHello.indexOf("}),"))).toContain("reducedMotion: input.reducedMotion ?? false");

    const bridge = fs.readFileSync(path.resolve("src-tauri/src/character_bridge.rs"), "utf8");
    expect(bridge).toContain('.get("reducedMotion")');
  });
});

describe("本機互動 → CPP 輸入事件（Gateway 再正規化）", () => {
  it("點擊／拖曳／靠近／快捷／文字／檔案各自對應；file drop 只有檔名沒有路徑", () => {
    expect(inputEventFor("companion-clicked")).toEqual({ kind: "character.clicked", payload: {} });
    expect(inputEventFor("companion-dragged")).toEqual({ kind: "character.drag-started", payload: {} });
    expect(inputEventFor("pointer-approached")).toEqual({ kind: "character.hover-entered", payload: {} });
    expect(inputEventFor("action-selected", { action: "quiet-1h" })).toEqual({ kind: "character.action-requested", payload: { action: "quiet-1h" } });
    expect(inputEventFor("action-selected", {})).toBeNull();
    expect(inputEventFor("text-submitted", { text: "hi" })).toMatchObject({ kind: "character.text-submitted", payload: { text: "hi" }, privacyClass: "personal" });
    expect(inputEventFor("text-submitted", { text: "" })).toBeNull();
    const drop = inputEventFor("companion-dropped", { attachments: ["/Users/me/secret/report.pdf", "C:\\\\docs\\\\a.txt"] });
    expect(drop?.kind).toBe("character.file-dropped");
    expect(drop?.privacyClass).toBe("personal");
    expect(drop?.payload).toEqual({ files: [{ name: "report.pdf" }, { name: "a.txt" }] });
    expect(JSON.stringify(drop)).not.toContain("/Users/");
    expect(inputEventFor("visibility-changed", { visible: false })).toEqual({ kind: "character.visibility-changed", payload: { visible: false } });
    expect(inputEventFor("toy-thrown", { toyId: "yarn" })).toEqual({ kind: "character.toy-thrown", payload: { toyId: "yarn" } });
  });

  it("CPP 沒有對應的遙測 kind（bubble-shown 等）回 null：protocol 模式不送", () => {
    expect(inputEventFor("bubble-shown", { source: "presentation-command" })).toBeNull();
    expect(inputEventFor("animation-completed")).toBeNull();
    expect(inputEventFor("nonsense")).toBeNull();
  });

  it("Runtime 的 character.system-text：綠勾只在 verified-success＋truthState verified；訊息截到 200 字", () => {
    expect(systemTextFromEvent({ messageId: "m", intent: "emergency", truthState: "emergency", message: "緊急停止中" })).toEqual({
      text: "緊急停止中",
      marker: "none",
      instanceId: null,
    });
    expect(systemTextFromEvent({ instanceId: "desktop-companion", intent: "verified-success", truthState: "verified", message: "做完了，也確認過結果。" })).toMatchObject({ marker: "verified", instanceId: "desktop-companion" });
    expect(systemTextFromEvent({ intent: "verified-success", truthState: "claimed", message: "做完了。" })?.marker).toBe("none");
    expect(systemTextFromEvent({ intent: "notice", truthState: "verified", message: "x" })?.marker).toBe("none");
    expect(systemTextFromEvent({ intent: "notice", message: "" })).toBeNull();
    expect(systemTextFromEvent({ intent: "notice", message: "y".repeat(500) })?.text).toHaveLength(200);
    expect(systemTextFromEvent(null)).toBeNull();
  });
});

describe("MixerRenderer 門面與 canonical 動畫名 → machine event", () => {
  it("machineEventForAnimation：真相名進對應 kind／基態，success 依 slice 分 claimed／verified，其餘是 performing", () => {
    expect(machineEventForAnimation("emergency")).toEqual({ type: "base", base: "emergency" });
    expect(machineEventForAnimation("offline")).toEqual({ type: "base", base: "offline" });
    // run-2 director-pipeline-022：adapter 層的回待機是 Runtime 派送／host cancel 走的路，可連安全訊息一起收（force）；
    // AI 可達的 presentation `cancel` 在 CompanionApp 不帶 force。
    expect(machineEventForAnimation("idle")).toEqual({ type: "clear-transient", force: true });
    expect(machineEventForAnimation("paused")).toEqual({ type: "clear-transient", force: true });
    expect(machineEventForAnimation("success", [0, 1])).toMatchObject({ type: "transient", kind: "succeeded", verified: false });
    expect(machineEventForAnimation("success")).toMatchObject({ kind: "succeeded", verified: true });
    expect(machineEventForAnimation("blocked")).toMatchObject({ kind: "blocked" });
    expect(machineEventForAnimation("ask")).toMatchObject({ kind: "requesting-consent" });
    expect(machineEventForAnimation("act", undefined, 800)).toMatchObject({ kind: "acting", durationMs: 800 });
    expect(machineEventForAnimation("waiting")).toMatchObject({ kind: "waiting-for-receipt" });
    expect(machineEventForAnimation("curious", [0, 1], 500)).toEqual({
      type: "transient",
      kind: "performing",
      animation: "curious",
      frameSlice: [0, 1],
      durationMs: 500,
    });
  });

  it("sprite adapter 的 setAnimation 進同一台 machine；reducedMotion／micro／destroy 轉給真正的 renderer", () => {
    let st: MachineState = { base: "idle", transient: null };
    const events: MachineEvent[] = [];
    const calls: string[] = [];
    const real: RendererBackend = {
      setAnimation: (n) => calls.push(`anim:${n}`),
      setReducedMotion: (on) => calls.push(`reduced:${on}`),
      setMicroMotion: () => calls.push("micro"),
      destroy: () => calls.push("destroy"),
    };
    const facade = new MixerRenderer(real, {
      apply: (ev) => {
        events.push(ev);
        st = reduce(st, ev, 1_000);
        return st;
      },
    });
    facade.setAnimation("success", [0, 1]);
    expect(st.transient).toMatchObject({ kind: "succeeded", verified: false });
    facade.setAnimation("blocked");
    expect(st.transient?.kind).toBe("blocked"); // 90 > 60：安全訊息贏
    // adapter 送進來的 idle 已被門面降權成非 force 的 clear-transient：安全訊息留在台上
    // （force 是 estop clear-all 的權力，呈現層沒有；對抗審查 renderer-lifecycle-028）。
    facade.setAnimation("idle");
    expect(st.transient?.kind).toBe("blocked");
    // 沒有安全訊息時 idle 照樣清得掉（一般表演不受影響）。
    st = { base: "idle", transient: { kind: "performing", untilMs: 9_000, animation: "greet" } };
    facade.setAnimation("idle");
    expect(st.transient).toBeNull();
    facade.setReducedMotion(true);
    facade.setMicroMotion({ gazeX: 0, gazeY: 0, earBias: 0, intensity: 0 });
    facade.destroy();
    // 真正的 renderer 從未直接收到 setAnimation（畫面由 host 的 pose() 驅動）。
    expect(calls).toEqual(["reduced:true", "micro", "destroy"]);
    expect(events).toHaveLength(4);
  });

  it("mapRuntimeEvent 預設 engine-neutral：不含任何 rig 表情 id", () => {
    const src = JSON.stringify([
      mapRuntimeEvent({ eventType: "provider.state-changed", payload: { state: "available" } }),
      mapRuntimeEvent({ eventType: "provider.state-changed", payload: { state: "revoked" } }),
      mapRuntimeEvent({ eventType: "action.dispatched", payload: { actuatorId: "device.x" } }),
      mapRuntimeEvent({ eventType: "action.acknowledged", payload: { actuatorId: "device.x" } }),
      mapRuntimeEvent({ eventType: "agent.session.state", payload: { state: "created", agentId: "codex" } }),
    ]);
    for (const id of ["device-hello", "device-lost", "operate-tool", "ack-nod", "wait-codex", "wait-claude"]) {
      expect(src).not.toContain(id);
    }
  });
});

describe("設定匯出／匯入：characterId 別名與可匯入判定", () => {
  const prefs = {
    companionPack: "shu-lazy",
    companionPersona: "persona-shu",
    companionExpressiveness: "natural",
    companionScene: "nest",
    companionFamiliars: [],
  } as unknown as DesktopPrefs;

  it("匯出同時寫 companionPack 與 characterId；沒取名字就留空，不硬編任何名字", () => {
    const out = exportCompanionSettings(prefs);
    expect(out.schemaVersion).toBe(1);
    expect(out.characterId).toBe("shu-lazy");
    expect(out.companionPack).toBe("shu-lazy");
    expect(out.companionName).toBe("");
    expect(JSON.stringify(out)).not.toContain("小樞");
  });

  it("匯入接受索引裡的任何 characterId 或 8 個舊 id；格式合法但不認得的仍拒絕；characterId 別名可用", () => {
    expect(isImportableCharacterId("shu-maid")).toBe(true);
    expect(isImportableCharacterId("fox-9", ["fox-9"])).toBe(true);
    expect(isImportableCharacterId("fox-9")).toBe(false);
    expect(isImportableCharacterId("../evil", ["../evil"])).toBe(false);
    expect(() => parseCompanionSettingsImport({ kind: "companion-settings", schemaVersion: 1, companionPack: "fox-9" })).toThrow();
    expect(parseCompanionSettingsImport({ kind: "companion-settings", schemaVersion: 1, companionPack: "fox-9" }, { knownCharacterIds: ["fox-9"] }).companionPack).toBe("fox-9");
    expect(parseCompanionSettingsImport({ kind: "companion-settings", schemaVersion: 1, characterId: "shu-agile" }).companionPack).toBe("shu-agile");
    expect(() => parseCompanionSettingsImport({ kind: "companion-settings", schemaVersion: 1, characterId: "nope" })).toThrow();
    // 空名字不覆蓋使用者已有的名字。
    expect("companionName" in parseCompanionSettingsImport({ kind: "companion-settings", schemaVersion: 1, companionName: "" })).toBe(false);
  });
});

describe("DEFAULT_LINES 的 `{name}` 樣板", () => {
  it("first-meeting 不硬編角色名；resolveLine 以 vars 代入；沒給名字時是中立文案", () => {
    expect(DEFAULT_LINES["first-meeting"][0]).toContain("{name}");
    expect(DEFAULT_LINES["first-meeting"][0]).not.toContain("小樞");
    expect(resolveLine("first-meeting", null, () => 0, { name: "小狐" })).toBe("你好，我是小狐。我只會在你允許的範圍內幫忙留意事情。");
    expect(resolveLine("first-meeting", null, () => 0)).toContain("我是角色");
    expect(applyLineVars("{name}{name}", { name: "A" })).toBe("AA");
    // 安全語句不受樣板影響。
    expect(resolveLine("emergency", null, () => 0, { name: "{name}" })).toBe("緊急停止中");
  });
});

// ---------------------------------------------------------------------------
// M2 §3.4：設定匯入／匯出的驗證邊界綁定「目標角色的 adapter」
//
// v0.6.0 已知限制 #17：使魔配色以前用單一 rig 的全域白名單驗證，說話風格同樣是硬編的
// 兩個 id——同一份 JSON 說自己屬於哪個角色完全不影響驗證。現在先解出 characterId →
// 問它的 adapter meta（variants／personas／hasPlayfield），問不出來就誠實拒絕。
// ---------------------------------------------------------------------------

describe("設定匯入：角色專屬欄位只用目標角色的 adapter 宣告驗證", () => {
  /** 角色 → entrypoint（正式路徑由角色頁的 catalog 提供，測試直接給對照表）。 */
  const ENTRYPOINTS: Record<string, string> = {
    "shu-maid": "shu-rig",
    "plain-text": "text",
    "ref-shape": "shape",
    "shu-lazy": "sprite",
  };
  const entrypointFor = (id: string): string | null => ENTRYPOINTS[id] ?? null;
  const knownCharacterIds = ["plain-text", "ref-shape"];
  const file = (extra: Record<string, unknown>) => ({ kind: "companion-settings", schemaVersion: 1, ...extra });

  it("使魔配色不再是全域白名單：ref-shape 帶著 rig 的配色會被拒絕", () => {
    expect(() =>
      parseCompanionSettingsImport(
        file({
          companionPack: "ref-shape",
          companionFamiliars: [{ id: "fam-1", name: "小雪", palette: "maid-classic" }],
        }),
        { knownCharacterIds, entrypointFor }
      )
    ).toThrowError(/使魔/);
  });

  it("說話風格不再是全域白名單：純文字角色帶著 rig 的 persona 會被拒絕", () => {
    expect(() =>
      parseCompanionSettingsImport(file({ companionPack: "plain-text", companionPersona: "persona-shu" }), {
        knownCharacterIds,
        entrypointFor,
      })
    ).toThrowError(/說話風格/);
  });

  it("舊小樞匯出檔（shu-maid＋使魔＋說話風格）照舊通過", () => {
    const out = parseCompanionSettingsImport(
      file({
        companionPack: "shu-maid",
        companionPersona: "persona-shu",
        companionScene: "nest",
        companionFamiliars: [{ id: "fam-1", name: "小雪", palette: "maid-dusk" }],
      }),
      { entrypointFor }
    );
    expect(out.companionPack).toBe("shu-maid");
    expect(out.companionPersona).toBe("persona-shu");
    expect(out.companionScene).toBe("nest");
    expect(out.companionFamiliars).toEqual([{ id: "fam-1", name: "小雪", palette: "maid-dusk" }]);
  });

  it("v0.5.x 由 sprite 小樞（shu-lazy）匯出的舊檔夾帶說話風格與使魔：誠實忽略，不拒絕整份檔", () => {
    // 那時說話風格／使魔是全域偏好、不分角色，5 個 sprite 小樞匯出的檔案都可能帶著它們。
    // 目標角色的 adapter 沒宣告那一項就忽略那個欄位（不寫進 prefs），其餘欄位照常匯入。
    const out = parseCompanionSettingsImport(
      file({
        companionPack: "shu-lazy",
        companionPersona: "persona-shu",
        companionScene: "desk",
        companionPlay: false,
        companionFamiliars: [{ id: "fam-1", name: "小雪", palette: "maid-dusk" }],
      }),
      { entrypointFor }
    );
    expect(out.companionPack).toBe("shu-lazy");
    expect(out.companionPlay).toBe(false);
    expect(out).not.toHaveProperty("companionPersona");
    expect(out).not.toHaveProperty("companionFamiliars");
    // 場景同樣是遊玩場（rig）的東西：sprite 小樞的 adapter 沒宣告，舊 id 走同一條寬容路徑
    //（誠實忽略，不拒絕整份檔）。
    expect(out).not.toHaveProperty("companionScene");
  });

  it("舊小樞家族的寬容只到「忽略未宣告的欄位」：宣告了的清單仍是白名單，非舊 id 一律不寬容", () => {
    // shu-maid（rig）宣告了 personas：值不在清單仍拒絕。
    expect(() =>
      parseCompanionSettingsImport(file({ companionPack: "shu-maid", companionPersona: "persona-nobody" }), {
        entrypointFor,
      })
    ).toThrowError(/說話風格/);
    // ref-shape 不是舊 id：未宣告就拒絕（不猜、不頂替）。
    expect(() =>
      parseCompanionSettingsImport(file({ companionPack: "ref-shape", companionPersona: "persona-shu" }), {
        knownCharacterIds,
        entrypointFor,
      })
    ).toThrowError(/說話風格/);
  });

  it("場景不再是全域白名單：問得出 adapter 但它沒宣告場景時，非舊 id 一律拒絕", () => {
    // 對抗審查 character-settings-binding-001：companionScene 以前是 settingsTransfer 自帶的
    // 五個 id（那是**某一個 rig** 的遊玩場場景），純文字／幾何角色照樣收得下，
    // 存成一個沒有人吃的死值。現在由目標角色的 adapter meta 宣告。
    expect(() =>
      parseCompanionSettingsImport(file({ companionPack: "plain-text", companionScene: "nest" }), {
        knownCharacterIds,
        entrypointFor,
      })
    ).toThrowError(/場景/);
    expect(() =>
      parseCompanionSettingsImport(file({ companionPack: "ref-shape", companionScene: "night" }), {
        knownCharacterIds,
        entrypointFor,
      })
    ).toThrowError(/場景/);
  });

  it("場景：問不出 adapter（沒有對照表）時誠實拒絕，不拿別的角色的允許值頂替", () => {
    expect(() => parseCompanionSettingsImport(file({ companionPack: "shu-maid", companionScene: "nest" }))).toThrow();
    expect(() => parseCompanionSettingsImport(file({ companionScene: "nest" }))).toThrow();
  });

  it("場景：宣告了清單就以那份清單驗證（rig 角色的合法值通過、非法值拒絕）", () => {
    expect(
      parseCompanionSettingsImport(file({ companionPack: "shu-maid", companionScene: "sill" }), { entrypointFor })
        .companionScene
    ).toBe("sill");
    expect(() =>
      parseCompanionSettingsImport(file({ companionPack: "shu-maid", companionScene: "volcano" }), { entrypointFor })
    ).toThrowError(/場景/);
  });

  it("匯出：目標角色沒有場景設定時不寫出場景（做不出一個自己匯不回來的檔案）", () => {
    const textPrefs = {
      companionPack: "plain-text",
      companionPersona: "",
      companionExpressiveness: "natural",
      companionScene: "nest",
      companionFamiliars: [],
    } as unknown as DesktopPrefs;
    expect(exportCompanionSettings(textPrefs, { entrypointFor }).companionScene).toBe("");
    const rigPrefs = { ...textPrefs, companionPack: "shu-maid" } as unknown as DesktopPrefs;
    expect(exportCompanionSettings(rigPrefs, { entrypointFor }).companionScene).toBe("nest");
  });

  it("adapter 認得角色時，值仍必須在該 adapter 宣告的清單裡", () => {
    expect(() =>
      parseCompanionSettingsImport(
        file({ companionPack: "shu-maid", companionFamiliars: [{ id: "a", name: "x", palette: "neon" }] }),
        { entrypointFor }
      )
    ).toThrow();
    expect(() =>
      parseCompanionSettingsImport(file({ companionPack: "shu-maid", companionPersona: "persona-nobody" }), {
        entrypointFor,
      })
    ).toThrow();
  });

  it("問不出目標角色的 adapter 時誠實拒絕角色專屬欄位（不拿別的角色的允許值頂替）", () => {
    expect(() =>
      parseCompanionSettingsImport(file({ companionPack: "shu-maid", companionPersona: "persona-shu" }))
    ).toThrow();
    expect(() =>
      parseCompanionSettingsImport(
        file({ companionPack: "shu-maid", companionFamiliars: [{ id: "a", name: "x", palette: "maid-classic" }] })
      )
    ).toThrow();
  });

  it("空的角色專屬欄位不算宣告：空使魔清單、空說話風格、空場景照樣通過", () => {
    // 純文字角色的匯出檔就長這樣（說話風格與場景都是空字串）：它必須匯得回來。
    const out = parseCompanionSettingsImport(
      file({ companionPack: "plain-text", companionPersona: "", companionFamiliars: [], companionScene: "" }),
      { knownCharacterIds, entrypointFor }
    );
    expect(out.companionFamiliars).toEqual([]);
    expect("companionPersona" in out).toBe(false);
    expect("companionScene" in out).toBe(false);
    // 但「不是空的、只是別人的場景 id」仍然拒絕（"none" 也是 rig 清單裡的一個 id）。
    expect(() =>
      parseCompanionSettingsImport(file({ companionPack: "plain-text", companionScene: "none" }), {
        knownCharacterIds,
        entrypointFor,
      })
    ).toThrowError(/場景/);
  });

  it("說話風格清單只有一份：由 adapter meta 宣告，settingsTransfer 不再自帶", () => {
    expect(builtinAdapterMeta("shu-rig")?.personas?.map((p) => p.id)).toEqual(["persona-shu", "persona-navigator"]);
    for (const id of ["sprite", "text", "shape"]) {
      expect(builtinAdapterMeta(id)?.personas ?? [], id).toEqual([]);
    }
  });

  it("匯出：知道目標角色時只帶該 adapter 宣告得出的角色專屬欄位（匯出→匯入不自打嘴巴）", () => {
    const prefs = {
      companionPack: "plain-text",
      companionPersona: "persona-shu",
      companionExpressiveness: "natural",
      companionScene: "none",
      companionFamiliars: [{ id: "fam-1", name: "小雪", palette: "maid-classic" }],
    } as unknown as DesktopPrefs;
    const out = exportCompanionSettings(prefs, { entrypointFor });
    expect(out.companionPersona).toBe("");
    expect(out.companionFamiliars).toEqual([]);
    expect(() =>
      parseCompanionSettingsImport(JSON.parse(JSON.stringify(out)), { knownCharacterIds, entrypointFor })
    ).not.toThrow();
    // 不給對照表時維持原樣（完整快照；舊呼叫端行為不變）。
    expect(exportCompanionSettings(prefs).companionPersona).toBe("persona-shu");
  });
});
