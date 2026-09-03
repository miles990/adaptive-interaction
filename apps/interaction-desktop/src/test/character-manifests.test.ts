// CPP：隨 App 出貨的角色 manifest（public/characters/**）全部通過 §2.1 驗證；index 預設角色存在；
// sprite 角色的 manifest 與 migratePackToManifest 由真實 pack 推導的結果一致；shu-rig 角色宣告 §12 完整能力集；
// registry 載入／匯入驗證／UI 摘要。

import { describe, expect, it } from "vitest";
import { migratePackToManifest, shuRigCapabilities, validateCharacterManifest } from "../character/manifest";
import { buildTextCharacterManifest } from "../character/adapters/text";
import { CHARACTER_INTENTS, LIMITS, SEMANTIC_CHANNELS } from "../character/protocol";
import {
  capabilitySummary,
  capabilitySummaryParts,
  loadCharacterIndex,
  validateImportedManifestText,
} from "../character/registry";

const MANIFESTS = import.meta.glob("../../public/characters/*/manifest.json", { eager: true, import: "default" }) as Record<
  string,
  Record<string, unknown>
>;
const MANIFEST_TEXTS = import.meta.glob("../../public/characters/*/manifest.json", { eager: true, query: "?raw", import: "default" }) as Record<
  string,
  string
>;
const PACKS = import.meta.glob("../../public/packs/*/manifest.json", { eager: true, import: "default" }) as Record<string, Record<string, unknown>>;
import indexJson from "../../public/characters/index.json";

interface IndexEntry {
  characterId: string;
  manifestPath: string;
  assetBase?: string;
  origin: string;
}

function manifestFor(id: string): Record<string, unknown> {
  const entry = Object.entries(MANIFESTS).find(([p]) => p.endsWith(`/characters/${id}/manifest.json`));
  if (!entry) throw new Error(`bundled manifest missing: ${id}`);
  return entry[1];
}

function packFor(id: string): Record<string, unknown> {
  const entry = Object.entries(PACKS).find(([p]) => p.endsWith(`/packs/${id}/manifest.json`));
  if (!entry) throw new Error(`legacy pack missing: ${id}`);
  return entry[1];
}

describe("bundled character manifests", () => {
  it("每一份都通過 validateCharacterManifest（含檔案大小），characterId 與資料夾一致", () => {
    const entries = Object.entries(MANIFESTS);
    expect(entries.length).toBe(9);
    for (const [path, manifest] of entries) {
      const text = MANIFEST_TEXTS[path];
      expect(typeof text, path).toBe("string");
      const r = validateCharacterManifest(manifest, { jsonText: text });
      expect(r.ok, `${path}: ${r.ok ? "" : r.errors.join("; ")}`).toBe(true);
      if (!r.ok) continue;
      const folder = path.split("/").slice(-2)[0];
      expect(r.manifest.characterId).toBe(folder);
      expect(r.manifest.adapterKind).toBe("in-process");
      expect(r.report.flags).toEqual({ external: false, network: false, executable: false, unsigned: true });
      expect(r.report.newerMinor).toBe(false);
      expect(r.report.unknownCapabilities).toEqual([]);
    }
  });

  it("index.json：schemaVersion 1.0、default 存在、每個項目都有對應 manifest 且 id 相符", () => {
    expect(indexJson.schemaVersion).toBe("1.0");
    const characters = indexJson.characters as IndexEntry[];
    const ids = characters.map((c) => c.characterId);
    expect(ids).toContain(indexJson.default);
    expect(indexJson.default).toBe("shu-maid");
    expect(new Set(ids).size).toBe(ids.length);
    for (const c of characters) {
      expect(c.manifestPath).toBe(`/characters/${c.characterId}/manifest.json`);
      expect(c.origin).toBe("builtin");
      expect(manifestFor(c.characterId).characterId).toBe(c.characterId);
      if (c.characterId.startsWith("shu-maid") || c.characterId === "plain-text") expect(c.assetBase).toBeUndefined();
      else expect(c.assetBase).toBe(`/packs/${c.characterId}`);
    }
    expect(ids.sort()).toEqual(
      ["plain-text", "shu-agile", "shu-lazy", "shu-lively", "shu-maid", "shu-maid-dusk", "shu-maid-sakura", "shu-minimal", "shu-standard"].sort()
    );
  });

  it("sprite 角色的 manifest 等於由真實 pack 遷移的結果（能力只來自 sheet 真的有的動畫）", () => {
    for (const id of ["shu-standard", "shu-minimal", "shu-lively", "shu-agile", "shu-lazy"]) {
      const migrated = migratePackToManifest(packFor(id), { assetBase: `/packs/${id}` });
      expect(migrated.ok, id).toBe(true);
      if (!migrated.ok) continue;
      const bundled = validateCharacterManifest(manifestFor(id));
      expect(bundled.ok).toBe(true);
      if (!bundled.ok) continue;
      expect(bundled.manifest).toEqual(migrated.manifest);
      expect(bundled.manifest.entrypoint).toEqual({ kind: "builtin", id: "sprite" });
      expect(bundled.manifest.assets.map((a) => a.path)).toEqual(["sheet.png", "preview.png"]);
      const animations = Object.keys((packFor(id).animations ?? {}) as Record<string, unknown>);
      expect(bundled.manifest.capabilities["visual.expression"].variants).toEqual(animations);
      expect(bundled.manifest.states).toEqual(animations);
    }
    // v1 沒 failed 美術 → fallback 到 blocked；v2 全部原生
    expect(manifestFor("shu-standard").intents).not.toContain("failed");
    expect((manifestFor("shu-standard").fallbacks as { intents: Record<string, string> }).intents.failed).toBe("blocked");
    expect(manifestFor("shu-lively").intents).toHaveLength(20);
  });

  it("shu-maid 三色：shu-rig entrypoint、§12 完整能力集、三個 variants（自己的 palette 在前）、代名詞 她、顯示名 小樞", () => {
    const expected = shuRigCapabilities();
    for (const [id, palette] of [
      ["shu-maid", "maid-classic"],
      ["shu-maid-dusk", "maid-dusk"],
      ["shu-maid-sakura", "maid-sakura"],
    ] as const) {
      const r = validateCharacterManifest(manifestFor(id));
      expect(r.ok, id).toBe(true);
      if (!r.ok) continue;
      const m = r.manifest;
      expect(m.entrypoint).toEqual({ kind: "builtin", id: "shu-rig" });
      expect(m.displayName).toEqual({ "zh-TW": "小樞", en: "Shu" });
      expect(m.description).toEqual(packFor(id).description);
      expect(m.pronouns).toEqual({ "zh-TW": "她", en: "she" });
      expect(m.capabilities).toEqual(expected.capabilities);
      expect(m.inputCapabilities).toEqual(expected.inputCapabilities);
      expect(m.variants[0].id).toBe(palette);
      expect(m.variants.map((v) => v.id).sort()).toEqual(["maid-classic", "maid-dusk", "maid-sakura"]);
      expect(m.channels).toEqual([...SEMANTIC_CHANNELS]);
      expect(m.intents).toEqual([...CHARACTER_INTENTS]);
      expect(m.states.length).toBeGreaterThanOrEqual(36);
      expect(m.securityRequirements.audioOutput).toBe(true);
      expect(m.securityRequirements.microphone).toBe(false);
      expect(m.securityRequirements.camera).toBe(false);
      // §12：全部 visual.*、audio.speech/effect、input.*、multiCharacter、scene、rollCall、gameplay.*
      const caps = Object.keys(m.capabilities);
      for (const c of ["visual.presence", "visual.pose", "visual.expression", "visual.gaze", "visual.locomotion", "visual.overlay", "visual.particles", "visual.prop", "visual.textBubble", "audio.speech", "audio.effect", "multiCharacter", "scene", "rollCall", "gameplay.toys", "gameplay.autonomy"]) {
        expect(caps, c).toContain(c);
      }
      expect(Object.keys(m.inputCapabilities)).toHaveLength(7);
    }
  });

  it("plain-text：text entrypoint、只有 presence/textBubble/click/text、沒有代名詞、與 adapter builder 一致", () => {
    const r = validateCharacterManifest(manifestFor("plain-text"));
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.manifest).toEqual(buildTextCharacterManifest());
    expect(r.manifest.displayName).toEqual({ "zh-TW": "文字角色", en: "Plain Text" });
    expect(r.manifest.pronouns).toBeUndefined();
    expect(r.manifest.assets).toEqual([]);
  });
});

describe("registry", () => {
  function fakeFetch(overrides: Record<string, string | null> = {}) {
    const files: Record<string, string> = { "/characters/index.json": JSON.stringify(indexJson) };
    for (const [path, text] of Object.entries(MANIFEST_TEXTS)) {
      const id = path.split("/").slice(-2)[0];
      files[`/characters/${id}/manifest.json`] = text;
    }
    for (const [k, v] of Object.entries(overrides)) {
      if (v === null) delete files[k];
      else files[k] = v;
    }
    const requested: string[] = [];
    const fetchImpl = async (url: string) => {
      requested.push(url);
      const body = files[url];
      return { ok: body !== undefined, status: body !== undefined ? 200 : 404, text: async () => body ?? "" };
    };
    return { fetchImpl, requested };
  }

  it("loadCharacterIndex 載入 9 個內建角色，預設 shu-maid，只讀同源路徑", async () => {
    const { fetchImpl, requested } = fakeFetch();
    const r = await loadCharacterIndex(fetchImpl);
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.index.default).toBe("shu-maid");
    expect(r.index.characters).toHaveLength(9);
    expect(r.index.errors).toEqual([]);
    expect(r.index.characters.every((c) => c.origin === "builtin")).toBe(true);
    expect(r.index.characters.find((c) => c.characterId === "shu-standard")?.assetBase).toBe("/packs/shu-standard");
    expect(requested.every((u) => u.startsWith("/characters/"))).toBe(true);
  });

  it("壞掉的項目進 errors 不拖垮索引；default 載不到才整份失敗；壞 index 安全失敗", async () => {
    const broken = fakeFetch({ "/characters/shu-lazy/manifest.json": JSON.stringify({ schemaVersion: "1.0", characterId: "shu-lazy" }) });
    const r = await loadCharacterIndex(broken.fetchImpl);
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.index.characters).toHaveLength(8);
      expect(r.index.errors).toHaveLength(1);
      expect(r.index.errors[0]).toMatch(/^shu-lazy: /);
    }
    const mismatch = fakeFetch({ "/characters/shu-lazy/manifest.json": MANIFEST_TEXTS[Object.keys(MANIFEST_TEXTS).find((p) => p.includes("shu-agile"))!] });
    const r2 = await loadCharacterIndex(mismatch.fetchImpl);
    expect(r2.ok && r2.index.errors[0]).toMatch(/does not match/);
    const noDefault = fakeFetch({ "/characters/shu-maid/manifest.json": null });
    const r3 = await loadCharacterIndex(noDefault.fetchImpl);
    expect(r3.ok).toBe(false);
    const badIndex = fakeFetch({ "/characters/index.json": "{not json" });
    expect((await loadCharacterIndex(badIndex.fetchImpl)).ok).toBe(false);
    const evilIndex = fakeFetch({
      "/characters/index.json": JSON.stringify({
        schemaVersion: "1.0",
        default: "shu-maid",
        characters: [
          { characterId: "shu-maid", manifestPath: "/characters/shu-maid/manifest.json", origin: "builtin" },
          { characterId: "evil", manifestPath: "https://evil.example/m.json", origin: "imported" },
          { characterId: "evil2", manifestPath: "/characters/../../etc/passwd", origin: "imported" },
        ],
      }),
    });
    const r4 = await loadCharacterIndex(evilIndex.fetchImpl);
    expect(r4.ok).toBe(true);
    if (r4.ok) {
      expect(r4.index.characters.map((c) => c.characterId)).toEqual(["shu-maid"]);
      expect(r4.index.errors).toHaveLength(2);
      expect(evilIndex.requested).not.toContain("https://evil.example/m.json");
    }
  });

  it("validateImportedManifestText：大小上限 256 KB、非 JSON、正常匯入", () => {
    const big = "x".repeat(LIMITS.manifestMaxBytes + 1);
    expect(validateImportedManifestText(big).ok).toBe(false);
    expect(validateImportedManifestText("{oops").ok).toBe(false);
    const text = MANIFEST_TEXTS[Object.keys(MANIFEST_TEXTS).find((p) => p.includes("plain-text"))!];
    const r = validateImportedManifestText(text);
    expect(r.ok && r.manifest.characterId).toBe("plain-text");
  });

  it("capabilitySummary：內建／第三方、本機／外部、可執行、網路、可接收資料、已測試（不含小樞專屬文案）", () => {
    const text = validateCharacterManifest(manifestFor("plain-text"));
    if (!text.ok) throw new Error("plain-text invalid");
    const zh = capabilitySummary(text.manifest, "zh-TW", { origin: "builtin" });
    expect(zh.join("\n")).toContain("內建角色");
    expect(zh.join("\n")).toContain("有可執行程式：否");
    expect(zh.join("\n")).toContain("需要網路：否");
    expect(zh.join("\n")).toContain("可以接收：點擊、文字輸入");
    expect(zh.join("\n")).toContain("已測試：是");
    expect(zh.join("\n")).not.toContain("小樞");
    const remote = validateCharacterManifest({
      schemaVersion: "1.0",
      characterId: "led-strip",
      displayName: { en: "LED strip" },
      version: "0.1.0",
      adapterKind: "remote-device",
      entrypoint: { kind: "url", url: "ws://127.0.0.1:9999" },
      capabilities: { "light.cue": { supported: true } },
      securityRequirements: { network: true, executable: false, fileAccess: "none", audioOutput: false, microphone: true, camera: false },
    });
    if (!remote.ok) throw new Error(remote.errors.join("; "));
    const en = capabilitySummary(remote.manifest, "en", { origin: "imported" });
    const joined = en.join("\n");
    expect(joined).toContain("third-party character");
    expect(joined).toContain("remote device (never auto-connects");
    expect(joined).toContain("Needs network: yes");
    expect(joined).toContain("Can receive: no input");
    expect(joined).toContain("microphone");
    expect(joined).toContain("Tested: no");
    expect(joined).toContain("Signature: none");
    const proc = validateCharacterManifest({
      schemaVersion: "1.0",
      characterId: "ext",
      displayName: { "zh-TW": "外部" },
      version: "0.1.0",
      adapterKind: "external-process",
      entrypoint: { kind: "process", command: ["ext-adapter"] },
    });
    if (!proc.ok) throw new Error(proc.errors.join("; "));
    expect(capabilitySummary(proc.manifest, "zh-TW").join("\n")).toContain("有可執行程式：是（只記錄，不會自動執行）");
  });

  it("capabilitySummaryParts：一般／進階分層，且不認得的互動 id 不外洩原始字串", () => {
    const r = validateCharacterManifest({
      schemaVersion: "1.0",
      characterId: "gesture-bot",
      displayName: { "zh-TW": "手勢機器人" },
      version: "0.1.0",
      adapterKind: "external-process",
      entrypoint: { kind: "process", command: ["gesture-adapter"] },
      capabilities: { "visual.textBubble": { supported: true } },
      inputCapabilities: { "input.gesture": { supported: true }, "input.mind": { supported: true } },
      securityRequirements: { network: true, executable: true, fileAccess: "none", audioOutput: false, microphone: false, camera: false },
    });
    if (!r.ok) throw new Error(r.errors.join("; "));
    const parts = capabilitySummaryParts(r.manifest, "zh-TW", { origin: "imported" });
    const general = parts.general.join("\n");
    const technical = parts.technical.join("\n");
    // 一般模式：來源與版本、可以接收、需要的裝置、已測試。
    expect(general).toContain("第三方角色");
    expect(general).toContain("可以接收：其他互動");
    expect(general).toContain("需要的裝置：無");
    expect(general).toContain("已測試：否");
    // 原始 id 永遠不進一般模式（重複的未知 id 也只出現一次）。
    expect(general).not.toContain("input.");
    expect(general.match(/其他互動/g)).toHaveLength(1);
    // 進階模式：執行方式、可執行程式、需要網路、檔案存取、簽章。
    expect(technical).toContain("外部程式（永不自動啟動，需明確安裝與授權）");
    expect(technical).toContain("有可執行程式：是");
    expect(technical).toContain("需要網路：是");
    expect(technical).toContain("檔案存取：不讀取檔案");
    expect(technical).toContain("簽章：無（本版不支援簽章驗證）");
    // 一般模式不含任何進階行；完整摘要仍是兩者的聯集（既有呼叫端不會少資訊）。
    for (const line of parts.technical) expect(parts.general).not.toContain(line);
    expect(capabilitySummary(r.manifest, "zh-TW", { origin: "imported" })).toEqual([
      ...parts.general,
      ...parts.technical,
    ]);
    // 「可以接收：」前綴保持不變（CharacterAdaptersSection.receiveLine 依賴它）。
    expect(capabilitySummary(r.manifest, "zh-TW").some((l) => l.startsWith("可以接收："))).toBe(true);
    expect(capabilitySummaryParts(r.manifest, "en", { origin: "imported" }).general.join("\n")).toContain(
      "Can receive: other interaction"
    );
  });
});
