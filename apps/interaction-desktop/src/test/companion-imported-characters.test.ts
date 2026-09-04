// CompanionApp 的匯入角色選擇與各角色偏好（純函式，無 React、無 Tauri、無 daemon）：
//   - prefs.companionPack 不在索引、也不是舊 id → 問 host 匯入清單（只有桌面版；瀏覽器模式跳過、永不擲例外）；
//   - 匯入 text／shu-rig／sprite 各自怎麼建 adapter（sprite 版型來自 manifest 的 x-legacy、sheet 經 host data URL）；
//   - 壞掉／不在白名單／找不到 → 文字角色＋failed＋原因（固定文案由 host 顯示，不是 adapter 說的）；
//   - prefs.companionPreferences[characterId] → adapter.reconfigure({...既有欄位, preferences, variant, palette})，
//     未知鍵原樣透傳、三個 reference adapter 都不擲例外；shu-rig 的 variant＝配色 → stage.setPalette；
//   - companion-reload：只動可就地套用的偏好就 reconfigure，動了角色／persona／尺寸才整頁重載。

import { describe, expect, it, vi } from "vitest";
import shuMaidRaw from "../../public/characters/shu-maid/manifest.json";
import shuStandard from "../../public/packs/shu-standard/manifest.json";
import type { CharacterAdapter } from "../character/adapter";
import { hostMigrationRegistry } from "../character/adapterRegistry";
import {
  importedRigPack,
  isShuRigPalette,
  rigPaletteForImported,
  SHU_RIG_PALETTES,
  SHU_RIG_VARIANTS,
  ShuCharacterAdapter,
} from "../character/adapters/shu";
import { SpriteCharacterAdapter } from "../character/adapters/sprite";
import { TextCharacterAdapter, buildTextCharacterManifest } from "../character/adapters/text";
import { CharacterGateway } from "../character/gateway";
import { migratePackToManifest, validateCharacterManifest } from "../character/manifest";
import type { CharacterManifest } from "../character/protocol";
import type { CharacterIndex, CharacterIndexEntry } from "../character/registry";
import {
  adapterReconfigureFor,
  characterPreferencesFor,
  companionReloadPlan,
  HOST_APPLIED_PREF_KEYS,
  importedCharacterSource,
  isImageDataUrl,
  LIVE_PREF_KEYS,
  needsImportedLookup,
  PRIMARY_INSTANCE_ID,
  resolveCharacterSource,
  selectCharacterSource,
  spritePackFromManifest,
  type ImportedCharacterListing,
} from "../companion/gatewayWiring";
import * as gatewayWiring from "../companion/gatewayWiring";
import { RIG_PALETTES } from "../companion/rig/params";
import { StageRenderer } from "../companion/rig/stage";
import { validateManifest, type PackManifest, type RendererBackend } from "../companion/renderer";
import type { DesktopPrefs } from "../desktop";

// ---------------------------------------------------------------------------
// fixtures
// ---------------------------------------------------------------------------

function shuManifest(): CharacterManifest {
  const v = validateCharacterManifest(shuMaidRaw);
  if (!v.ok) throw new Error(v.errors.join("; "));
  return v.manifest;
}

function indexEntry(characterId: string, manifest = shuManifest()): CharacterIndexEntry {
  return {
    characterId,
    manifestPath: `/characters/${characterId}/manifest.json`,
    origin: "builtin",
    manifest: { ...manifest, characterId },
    report: { newerMinor: false, unknownCapabilities: [], customCapabilities: [], warnings: [], flags: { external: false, network: false, executable: false, unsigned: true } },
  };
}

const textEntry = indexEntry("plain-text", buildTextCharacterManifest());
const idx: CharacterIndex = { schemaVersion: "1.0", default: "shu-maid", characters: [indexEntry("shu-maid"), textEntry], errors: [] };

function listing(over: Partial<ImportedCharacterListing> & { characterId: string }): ImportedCharacterListing {
  return {
    valid: true,
    displayName: { "zh-TW": "小狐", en: "Fox" },
    adapterKind: "in-process",
    entrypoint: "text",
    version: "1.0.0",
    executable: false,
    network: false,
    external: false,
    assets: [],
    origin: "imported",
    ...over,
  };
}

const pack = shuStandard as unknown as PackManifest;

/** 匯入 sprite 角色的 manifest：CPP 欄位由 TS 遷移器產生，版型放在 Rust 遷移器寫的 `x-legacy` 擴充。 */
function importedSpriteManifest(characterId = "fox-sprite", over: Record<string, unknown> = {}): Record<string, unknown> {
  const migrated = migratePackToManifest(pack);
  if (!migrated.ok) throw new Error(migrated.errors.join("; "));
  const { legacy: _legacy, ...base } = migrated.manifest as unknown as Record<string, unknown>;
  void _legacy;
  return {
    ...base,
    characterId,
    displayName: { "zh-TW": "小狐（圖格）", en: "Fox Sprite" },
    assets: [{ id: "sheet", path: pack.sheet, mediaType: "image/png" }],
    "x-legacy": {
      kind: "character-pack",
      schemaVersion: pack.schemaVersion,
      sheet: pack.sheet,
      frameSize: pack.frameSize,
      anchor: pack.anchor,
      columns: pack.columns,
      animations: pack.animations,
      hasAnchors: false,
    },
    ...over,
  };
}

/** 匯入 shu-rig 角色的 manifest（bundled shu-maid 換 id／名字、去掉 legacy 提示）。 */
function importedRigManifest(characterId = "fox-rig", over: Record<string, unknown> = {}): Record<string, unknown> {
  const { legacy: _legacy, ...base } = shuMaidRaw as unknown as Record<string, unknown>;
  void _legacy;
  return { ...base, characterId, displayName: { "zh-TW": "小狐（rig）", en: "Fox Rig" }, ...over };
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

function makeStage(palette = "maid-classic"): StageRenderer {
  return new StageRenderer(stubCanvas(), palette, 1, { autoStart: false, rng: () => 0.9, now: () => 0 });
}

const host = { now: () => 0, reducedMotion: () => false, locale: "zh-TW", log: () => {} };

class FakeRenderer implements RendererBackend {
  calls: string[] = [];
  setAnimation(name: string) {
    this.calls.push(name);
  }
  setReducedMotion() {}
  setMicroMotion() {}
  destroy() {}
}

/** 記錄每次 reconfigure 負載的假 adapter（其餘行為同文字角色）。 */
class RecordingAdapter extends TextCharacterAdapter {
  calls: Record<string, unknown>[] = [];
  override reconfigure(prefs: Record<string, unknown>): void {
    this.calls.push(prefs);
  }
}

function prefsWith(over: Partial<DesktopPrefs>): DesktopPrefs {
  return {
    closeBehavior: null,
    askOnClose: true,
    launchAtLogin: false,
    showCompanionOnStart: true,
    openControlCenterOnStart: false,
    companionVisible: true,
    companionPosition: null,
    companionSize: [200, 200],
    companionOpacity: 1,
    companionPack: "fox-rig",
    companionPersona: "persona-shu",
    companionExpressiveness: "natural",
    companionAlwaysOnTop: true,
    storyProgress: {},
    companionName: "",
    companionScene: "none",
    companionPlay: true,
    companionCursorPlay: true,
    companionApproach: true,
    companionDeskMove: true,
    companionFamiliars: [],
    companionDoNotDisturb: false,
    companionBubbles: true,
    companionSound: false,
    companionDragEnabled: true,
    companionProactiveQuietUntil: 0,
    schemaVersion: 1,
    ...over,
  };
}

// ---------------------------------------------------------------------------
// 匯入角色選擇
// ---------------------------------------------------------------------------

describe("匯入角色選擇：text／shu-rig／sprite", () => {
  it("text：清單摘要就夠（displayName 進 TextCharacterAdapter，characterId＝匯入 id）", async () => {
    const entry = listing({ characterId: "fox-text", entrypoint: "text" });
    const src = importedCharacterSource([entry], "fox-text");
    expect(src).toMatchObject({ kind: "imported", characterId: "fox-text", entrypoint: "text", manifest: null });
    const adapter = new TextCharacterAdapter({ characterId: entry.characterId, displayName: entry.displayName });
    expect(adapter.manifest.characterId).toBe("fox-text");
    expect(adapter.manifest.displayName["zh-TW"]).toBe("小狐");
    await adapter.initialize(host);
    expect(adapter.negotiate({ reducedMotion: false } as never).characterId).toBe("fox-text");
  });

  it("shu-rig 沒有 manifest 本文：由清單摘要組 character-rig 2.0 pack 遷移，配色可指定且進 stage", async () => {
    const entry = listing({ characterId: "fox-rig", entrypoint: "shu-rig", version: "2.1.0" });
    const src = importedCharacterSource([entry], "fox-rig");
    expect(src).toMatchObject({ kind: "imported", entrypoint: "shu-rig", manifest: null });
    const rig = importedRigPack(entry, "maid-dusk");
    expect(rig).toMatchObject({ kind: "character-rig", id: "fox-rig", palette: "maid-dusk", version: "2.1.0", name: { "zh-TW": "小狐" } });
    const migrated = migratePackToManifest(rig, { registry: hostMigrationRegistry() });
    expect(migrated.ok).toBe(true);
    if (!migrated.ok) return;
    expect(migrated.manifest.characterId).toBe("fox-rig");
    expect(migrated.manifest.entrypoint).toEqual({ kind: "builtin", id: "shu-rig" });
    const stage = makeStage();
    const shu = new ShuCharacterAdapter({ legacyRig: rig, palette: "maid-dusk", stage });
    await shu.initialize(host);
    expect(shu.manifest.characterId).toBe("fox-rig");
    expect(stage.currentPalette()).toBe("maid-dusk");
    // 未知配色名不猜：退回 maid-classic；沒有 displayName 用中立文案。
    expect(importedRigPack({ characterId: "x", displayName: {}, version: "" }, "neon")).toMatchObject({ palette: "maid-classic", name: { "zh-TW": "角色" } });
  });

  it("shu-rig 有 manifest 本文：驗證後直接用；初始配色 x-legacy → preferencesSchema 預設 → variants[0] → maid-classic", () => {
    const manifest = importedRigManifest("fox-rig");
    const entry = listing({ characterId: "fox-rig", entrypoint: "shu-rig", manifest });
    const src = importedCharacterSource([entry], "fox-rig");
    expect(src.kind).toBe("imported");
    if (src.kind !== "imported") return;
    expect(src.manifest?.characterId).toBe("fox-rig");
    expect(src.manifest?.displayName["zh-TW"]).toBe("小狐（rig）");
    // bundled shu-maid 的 variants[0] 是 maid-classic。
    expect(rigPaletteForImported(src.manifest)).toBe("maid-classic");
    const sakuraFirst = validateCharacterManifest(importedRigManifest("fox-rig", { variants: [{ id: "maid-sakura" }, { id: "maid-classic" }] }));
    expect(sakuraFirst.ok && rigPaletteForImported(sakuraFirst.manifest)).toBe("maid-sakura");
    const schemaDefault = validateCharacterManifest(
      importedRigManifest("fox-rig", { preferencesSchema: { type: "object", properties: { variant: { type: "string", enum: ["maid-dusk", "maid-classic"], default: "maid-dusk" } } } })
    );
    expect(schemaDefault.ok && rigPaletteForImported(schemaDefault.manifest)).toBe("maid-dusk");
    const xLegacy = validateCharacterManifest(importedRigManifest("fox-rig", { "x-legacy": { kind: "character-rig", schemaVersion: "2.0", palette: "maid-sakura" } }));
    expect(xLegacy.ok && rigPaletteForImported(xLegacy.manifest)).toBe("maid-sakura");
    // 白名單外的名字一律不採用。
    const bogus = validateCharacterManifest(importedRigManifest("fox-rig", { variants: [{ id: "neon" }], "x-legacy": { palette: "../x" } }));
    expect(bogus.ok && rigPaletteForImported(bogus.manifest)).toBe("maid-classic");
    expect(rigPaletteForImported(null)).toBe("maid-classic");
    expect(SHU_RIG_PALETTES.every((p) => p in RIG_PALETTES)).toBe(true);
    const migratedRig = migratePackToManifest(
      { schemaVersion: "2.0", kind: "character-rig", id: "a", name: { en: "A" }, palette: "maid-classic" },
      { registry: hostMigrationRegistry() }
    );
    expect(migratedRig.ok && migratedRig.manifest.variants.map((v) => v.id).sort()).toEqual([...SHU_RIG_PALETTES].sort());
  });

  it("配色 helper 只住在 shu adapter：接線層不再 re-export 任何一個（strangler 收尾）", () => {
    // 對抗審查 character-package-018：接線層曾經 re-export 這些 rig 專屬 helper，
    // 讓「某個角色的預設配色」有機會被當成所有 adapter 的預設值。現在只剩 adapter 自己認得。
    for (const name of ["importedRigPack", "isShuRigPalette", "rigPaletteFor", "rigPaletteForImported", "SHU_RIG_PALETTES"]) {
      expect(name in (gatewayWiring as unknown as Record<string, unknown>), `gatewayWiring 不該再 re-export ${name}`).toBe(false);
    }
    // variants 帶雙語顯示名（遷移產生的 manifest.variants 就是它）。
    expect(SHU_RIG_VARIANTS.map((v) => v.id)).toEqual([...SHU_RIG_PALETTES]);
    for (const v of SHU_RIG_VARIANTS) {
      expect(Object.keys(v.displayName).sort()).toEqual(["en", "zh-TW"]);
      expect(v.displayName["zh-TW"]?.length).toBeGreaterThan(0);
    }
    expect(isShuRigPalette("neon")).toBe(false);
    expect(isShuRigPalette("maid-dusk")).toBe(true);
  });

  it("sprite：版型由 manifest 的 x-legacy 派生（既有 validateManifest 驗過）、sheet 是宣告的資產、adapter 建得出來", async () => {
    const manifest = importedSpriteManifest("fox-sprite");
    const entry = listing({ characterId: "fox-sprite", entrypoint: "sprite", assets: ["sheet"], manifest });
    const src = importedCharacterSource([entry], "fox-sprite");
    expect(src.kind).toBe("imported");
    if (src.kind !== "imported") return;
    expect(src.entrypoint).toBe("sprite");
    expect(src.sprite?.sheetAssetId).toBe("sheet");
    const shape = src.sprite!.pack;
    expect(validateManifest(shape)).toEqual([]);
    expect(shape).toMatchObject({ kind: "character-pack", id: "fox-sprite", sheet: pack.sheet, columns: pack.columns, frameSize: pack.frameSize });
    expect(Object.keys(shape.animations)).toEqual(Object.keys(pack.animations));
    const renderer = new FakeRenderer();
    const adapter = new SpriteCharacterAdapter({ pack: shape, assetBase: "imported:fox-sprite", renderer });
    await adapter.initialize(host);
    expect(adapter.manifest.characterId).toBe("fox-sprite");
    expect(adapter.manifest.entrypoint).toEqual({ kind: "builtin", id: "sprite" });
    expect(renderer.calls).toEqual(["idle"]);
    // 版型 helper 也接受 TS 遷移器的 `legacy` 鍵名；sheet 以同路徑資產對上 id。
    const viaLegacyKey = validateCharacterManifest(
      importedSpriteManifest("fox-sprite", {
        "x-legacy": undefined,
        legacy: { kind: "character-pack", sheet: pack.sheet, frameSize: pack.frameSize, anchor: pack.anchor, columns: pack.columns, animations: pack.animations },
        assets: [{ id: "img-1", path: pack.sheet, mediaType: "image/png" }],
      })
    );
    expect(viaLegacyKey.ok && spritePackFromManifest(viaLegacyKey.manifest)?.sheetAssetId).toBe("img-1");
  });

  it("sprite 沒有版型就退文字（failed＋原因）：清單沒夾 manifest、manifest 沒有 x-legacy、sheet 不是宣告資產、columns 非法", () => {
    const noManifest = importedCharacterSource([listing({ characterId: "fox-sprite", entrypoint: "sprite", assets: ["sheet"] })], "fox-sprite");
    expect(noManifest).toMatchObject({ kind: "text", characterId: "plain-text", failed: true });
    expect((noManifest as { reason: string }).reason).toContain("x-legacy");

    const noShape = importedCharacterSource([listing({ characterId: "fox-sprite", entrypoint: "sprite", manifest: importedSpriteManifest("fox-sprite", { "x-legacy": undefined }) })], "fox-sprite");
    expect(noShape).toMatchObject({ kind: "text", failed: true });

    const noSheet = validateCharacterManifest(importedSpriteManifest("fox-sprite", { assets: [{ id: "preview", path: "preview.png", mediaType: "image/png" }] }));
    expect(noSheet.ok && spritePackFromManifest(noSheet.manifest)).toBeNull();

    const badColumns = validateCharacterManifest(importedSpriteManifest("fox-sprite", { "x-legacy": { ...(importedSpriteManifest()["x-legacy"] as object), columns: 0 } }));
    expect(badColumns.ok && spritePackFromManifest(badColumns.manifest)).toBeNull();

    const audioSheet = validateCharacterManifest(importedSpriteManifest("fox-sprite", { assets: [{ id: "sheet", path: pack.sheet, mediaType: "audio/mpeg" }] }));
    expect(audioSheet.ok && spritePackFromManifest(audioSheet.manifest)).toBeNull();

    // 不是 sprite entrypoint 的 manifest 不會被當版型。
    expect(spritePackFromManifest(shuManifest())).toBeNull();
  });

  it("host 讀出的 sheet 必須是影像 data URL", () => {
    expect(isImageDataUrl("data:image/png;base64,iVBORw0KGgo=")).toBe(true);
    expect(isImageDataUrl("data:image/svg+xml;base64,PHN2Zz4=")).toBe(true);
    expect(isImageDataUrl("data:text/html;base64,PGh0bWw+")).toBe(false);
    expect(isImageDataUrl("data:image/png;base64,")).toBe(false);
    expect(isImageDataUrl("https://example.com/sheet.png")).toBe(false);
    expect(isImageDataUrl("/packs/x/sheet.png")).toBe(false);
    expect(isImageDataUrl(null)).toBe(false);
  });
});

describe("匯入角色選擇：找不到／壞掉／不在白名單 → 文字角色＋原因", () => {
  it("找不到：清單查詢回文字＋failed；經 selectCharacterSource 時維持既有規則（索引 default／索引不可用 → 文字）", () => {
    expect(importedCharacterSource([], "ghost")).toMatchObject({ kind: "text", characterId: "plain-text", failed: true, reason: "ghost: imported character not found" });
    expect(importedCharacterSource(null, "ghost")).toMatchObject({ kind: "text", failed: true });
    expect(selectCharacterSource(idx, "ghost", [])).toMatchObject({ kind: "index", characterId: "shu-maid" });
    expect(selectCharacterSource(null, "ghost", [])).toMatchObject({ kind: "text", reason: "character index unavailable" });
    expect((selectCharacterSource(null, "ghost", []) as { failed?: boolean }).failed).toBeUndefined();
  });

  it("valid:false（壞資料夾）→ 文字＋原因含 host 的錯誤；錯誤裡像路徑的片段被隱藏", () => {
    const broken = importedCharacterSource([listing({ characterId: "bad", valid: false, error: "manifest invalid: schemaVersion major must be 1" })], "bad");
    expect(broken).toMatchObject({ kind: "text", failed: true });
    expect((broken as { reason: string }).reason).toContain("schemaVersion major must be 1");
    const leaky = importedCharacterSource([listing({ characterId: "bad", valid: false, error: "read manifest: /Users/me/.adaptive-interaction/state/characters/bad/manifest.json missing" })], "bad");
    const reason = (leaky as { reason: string }).reason;
    expect(reason).not.toContain("/Users/");
    expect(reason).toContain("（路徑已隱藏）");
    expect(importedCharacterSource([listing({ characterId: "bad", valid: false })], "bad")).toMatchObject({ reason: expect.stringContaining("manifest unreadable") });
  });

  it("不是 in-process／要可執行程式或網路／entrypoint 不在白名單 → 文字（角色視窗永不執行、永不連線）", () => {
    for (const over of [
      { external: true },
      { adapterKind: "web" },
      { adapterKind: "external-process" },
      { executable: true },
      { network: true },
      { entrypoint: "module" },
      { entrypoint: "process" },
      { entrypoint: undefined },
    ] as Partial<ImportedCharacterListing>[]) {
      const src = importedCharacterSource([listing({ characterId: "fox", ...over })], "fox");
      expect(src, JSON.stringify(over)).toMatchObject({ kind: "text", characterId: "plain-text", failed: true });
      expect((src as { reason: string }).reason).toContain("fox: ");
    }
  });

  it("清單夾帶的 manifest 要驗證、id 與 entrypoint 要對得上；不符就退文字", () => {
    const garbage = importedCharacterSource([listing({ characterId: "fox", manifest: { schemaVersion: "9.0" } })], "fox");
    expect(garbage).toMatchObject({ kind: "text", failed: true, reason: expect.stringContaining("manifest invalid") });
    const wrongId = importedCharacterSource([listing({ characterId: "fox", entrypoint: "shu-rig", manifest: importedRigManifest("other-fox") })], "fox");
    expect(wrongId).toMatchObject({ kind: "text", failed: true, reason: expect.stringContaining("characterId mismatch") });
    const wrongEntrypoint = importedCharacterSource([listing({ characterId: "fox-rig", entrypoint: "text", manifest: importedRigManifest("fox-rig") })], "fox-rig");
    expect(wrongEntrypoint).toMatchObject({ kind: "text", failed: true, reason: expect.stringContaining("entrypoint mismatch") });
    // 清單摘要合法時，同一列不帶 manifest 也能用（摘要與 manifest 的 entrypoint 一致）。
    expect(importedCharacterSource([listing({ characterId: "fox-rig", entrypoint: "shu-rig", manifest: importedRigManifest("fox-rig") })], "fox-rig").kind).toBe("imported");
  });

  it("優先序：索引命中 ＞ 舊 id ＞ 匯入清單 ＞ 索引 default；沒問過清單（null）就照舊", () => {
    const imported = [listing({ characterId: "shu-maid", entrypoint: "text" }), listing({ characterId: "shu-lazy", entrypoint: "text" }), listing({ characterId: "fox-text" })];
    expect(selectCharacterSource(idx, "shu-maid", imported)).toMatchObject({ kind: "index", characterId: "shu-maid" });
    expect(selectCharacterSource(idx, "shu-lazy", imported)).toEqual({ kind: "legacy-pack", characterId: "shu-lazy" });
    expect(selectCharacterSource(idx, "fox-text", imported)).toMatchObject({ kind: "imported", characterId: "fox-text" });
    expect(selectCharacterSource(idx, "fox-text", null)).toMatchObject({ kind: "index", characterId: "shu-maid" });
    expect(selectCharacterSource(idx, "fox-text")).toMatchObject({ kind: "index", characterId: "shu-maid" });
    expect(selectCharacterSource(null, "fox-text", imported)).toMatchObject({ kind: "imported", characterId: "fox-text" });
    // 清單裡有但壞掉：使用者明確選了它 → 文字＋failed，不默默換成預設角色。
    expect(selectCharacterSource(idx, "bad", [listing({ characterId: "bad", valid: false })])).toMatchObject({ kind: "text", failed: true });
  });

  it("needsImportedLookup：只有「偏好不在索引、也不是舊 id」才問 host", () => {
    expect(needsImportedLookup(idx, null)).toBe(false);
    expect(needsImportedLookup(idx, "")).toBe(false);
    expect(needsImportedLookup(idx, "shu-maid")).toBe(false);
    expect(needsImportedLookup(idx, "shu-lazy")).toBe(false);
    expect(needsImportedLookup(null, "shu-agile")).toBe(false);
    expect(needsImportedLookup(idx, "fox-text")).toBe(true);
    expect(needsImportedLookup(null, "fox-text")).toBe(true);
  });
});

describe("resolveCharacterSource：瀏覽器模式跳過清單、host 失敗不擲例外", () => {
  it("非 Tauri：完全不呼叫 listImported（即使它會擲例外），照索引選", async () => {
    const listImported = vi.fn(async () => {
      throw new Error("角色匯入與管理需要桌面版控制中心");
    });
    const r = await resolveCharacterSource({ index: idx, preferred: "fox-text", tauri: false, listImported });
    expect(listImported).not.toHaveBeenCalled();
    expect(r.importedLookup).toBe("skipped");
    expect(r.source).toMatchObject({ kind: "index", characterId: "shu-maid" });
    const none = await resolveCharacterSource({ index: null, preferred: "fox-text", tauri: false, listImported });
    expect(none.source).toMatchObject({ kind: "text", reason: "character index unavailable" });
  });

  it("Tauri 但偏好在索引／是舊 id：不問清單", async () => {
    const listImported = vi.fn(async () => [listing({ characterId: "fox-text" })]);
    expect((await resolveCharacterSource({ index: idx, preferred: "shu-maid", tauri: true, listImported })).importedLookup).toBe("skipped");
    expect((await resolveCharacterSource({ index: idx, preferred: "shu-agile", tauri: true, listImported })).source).toEqual({ kind: "legacy-pack", characterId: "shu-agile" });
    expect(listImported).not.toHaveBeenCalled();
  });

  it("Tauri＋偏好是匯入角色：清單回來就用；host 失敗 → 文字＋failed（不默默換預設、不假裝成功）", async () => {
    const ok = await resolveCharacterSource({ index: idx, preferred: "fox-text", tauri: true, listImported: async () => [listing({ characterId: "fox-text" })] });
    expect(ok.importedLookup).toBe("done");
    expect(ok.source).toMatchObject({ kind: "imported", characterId: "fox-text", entrypoint: "text" });
    const failed = await resolveCharacterSource({
      index: idx,
      preferred: "fox-text",
      tauri: true,
      listImported: async () => {
        throw new Error("invoke failed: /Users/me/state/characters unreadable");
      },
    });
    expect(failed.importedLookup).toBe("failed");
    expect(failed.source).toMatchObject({ kind: "text", characterId: "plain-text", failed: true });
    expect((failed.source as { reason: string }).reason).toContain("imported character list unavailable");
    expect(JSON.stringify(failed)).not.toContain("/Users/");
    // host 回了不是陣列的東西：當空清單 → 索引 default。
    const weird = await resolveCharacterSource({ index: idx, preferred: "fox-text", tauri: true, listImported: async () => ({} as never) });
    expect(weird.importedLookup).toBe("done");
    expect(weird.source).toMatchObject({ kind: "index", characterId: "shu-maid" });
  });
});

// ---------------------------------------------------------------------------
// 各角色偏好 → reconfigure
// ---------------------------------------------------------------------------

describe("各角色偏好：prefs.companionPreferences[characterId] → reconfigure 負載", () => {
  it("host 還沒保存這欄位 → 空表；有值時只收 boolean／number／string，其餘丟棄、字串限長、鍵數有界", () => {
    expect(characterPreferencesFor(null, "fox", "text")).toEqual({ preferences: {} });
    expect(characterPreferencesFor({}, "fox", "text")).toEqual({ preferences: {} });
    expect(characterPreferencesFor({ companionPreferences: { other: { a: 1 } } }, "fox", "text")).toEqual({ preferences: {} });
    expect(characterPreferencesFor({ companionPreferences: "nope" }, "fox", "text")).toEqual({ preferences: {} });
    const many: Record<string, unknown> = {};
    for (let i = 0; i < 40; i++) many[`k${i}`] = i;
    const out = characterPreferencesFor(
      {
        companionPreferences: {
          fox: {
            glow: true,
            speed: 2.5,
            mood: "calm",
            long: "x".repeat(500),
            nested: { a: 1 },
            list: [1, 2],
            nothing: null,
            nan: Number.NaN,
            fn: () => 1,
            __proto__: { hacked: true },
            ...many,
          } as unknown as Record<string, boolean | number | string>,
        },
      },
      "fox",
      "text"
    );
    expect(out.preferences.glow).toBe(true);
    expect(out.preferences.speed).toBe(2.5);
    expect(out.preferences.mood).toBe("calm");
    expect((out.preferences.long as string).length).toBe(200);
    expect(out.preferences).not.toHaveProperty("nested");
    expect(out.preferences).not.toHaveProperty("list");
    expect(out.preferences).not.toHaveProperty("nothing");
    expect(out.preferences).not.toHaveProperty("nan");
    expect(out.preferences).not.toHaveProperty("fn");
    expect(Object.prototype.hasOwnProperty.call(out.preferences, "__proto__")).toBe(false);
    expect((out.preferences as { hacked?: unknown }).hacked).toBeUndefined();
    expect(Object.keys(out.preferences).length).toBeLessThanOrEqual(32);
    expect(out.variant).toBeUndefined();
    expect(out.palette).toBeUndefined();
    // JSON 來的 own-property "__proto__"／"constructor" 也不收。
    const hostile = characterPreferencesFor({ companionPreferences: { fox: JSON.parse('{"__proto__":{"hacked":true},"constructor":"x","ok":1}') } }, "fox", "text");
    expect(hostile).toEqual({ preferences: { ok: 1 } });
    expect(Object.prototype.hasOwnProperty.call(hostile.preferences, "__proto__")).toBe(false);
  });

  it("variant 保留鍵：shu-rig 的三種配色 → palette；其他 entrypoint／未知 variant 只透傳 variant", () => {
    const prefs = { companionPreferences: { fox: { variant: "maid-dusk", glow: true } } };
    expect(characterPreferencesFor(prefs, "fox", "shu-rig")).toEqual({ preferences: { variant: "maid-dusk", glow: true }, variant: "maid-dusk", palette: "maid-dusk" });
    expect(characterPreferencesFor(prefs, "fox", "sprite")).toEqual({ preferences: { variant: "maid-dusk", glow: true }, variant: "maid-dusk" });
    expect(characterPreferencesFor(prefs, "fox", null)).toEqual({ preferences: { variant: "maid-dusk", glow: true }, variant: "maid-dusk" });
    const unknown = characterPreferencesFor({ companionPreferences: { fox: { variant: "neon" } } }, "fox", "shu-rig");
    expect(unknown).toEqual({ preferences: { variant: "neon" }, variant: "neon" });
    expect(characterPreferencesFor({ companionPreferences: { fox: { variant: "" } } }, "fox", "shu-rig")).toEqual({ preferences: { variant: "" } });
    expect(characterPreferencesFor({ companionPreferences: { fox: { variant: 3 } } }, "fox", "shu-rig")).toEqual({ preferences: { variant: 3 } });
  });

  it("adapterReconfigureFor：既有欄位（name／scene／play／cursorPlay／deskMove／familiars／tuning）＋角色偏好一起帶", () => {
    const prefs = prefsWith({
      companionScene: "desk",
      companionPlay: false,
      companionFamiliars: [{ id: "f1", name: "小一", palette: "maid-dusk" }],
      companionPreferences: { "fox-rig": { variant: "maid-sakura", chatty: false } },
    });
    const payload = adapterReconfigureFor(prefs, { name: "阿狐", characterId: "fox-rig", entrypoint: "shu-rig", tuning: { speed: 1 } });
    expect(payload).toEqual({
      name: "阿狐",
      scene: "desk",
      play: false,
      cursorPlay: true,
      approach: true,
      deskMove: true,
      familiars: [{ id: "f1", name: "小一", palette: "maid-dusk" }],
      tuning: { speed: 1 },
      preferences: { variant: "maid-sakura", chatty: false },
      variant: "maid-sakura",
      palette: "maid-sakura",
    });
    // 偏好裡的 name／scene 鍵只在 preferences 底下，不會蓋掉 host 的欄位。
    const shadow = adapterReconfigureFor(prefsWith({ companionPreferences: { fox: { name: "駭", scene: "night" } } }), { name: "阿狐", characterId: "fox", entrypoint: "text", tuning: null });
    expect(shadow.name).toBe("阿狐");
    expect(shadow.scene).toBe("none");
    expect(shadow.preferences).toEqual({ name: "駭", scene: "night" });
    expect(adapterReconfigureFor(null, { name: "角色", characterId: "x", entrypoint: null, tuning: null })).toMatchObject({ familiars: [], preferences: {} });
    expect(adapterReconfigureFor({ companionFamiliars: "nope" } as never, { name: "角色", characterId: "x", entrypoint: null, tuning: null }).familiars).toEqual([]);
  });

  it("假 adapter 經 Gateway 收到 reconfigure：開機一次、companion-reload 後帶新 variant 再一次；未知鍵透傳、實例仍 ready", async () => {
    const gw = new CharacterGateway({ now: () => 0, onSystemText: () => {} });
    const fake = new RecordingAdapter({ characterId: "fox-text", displayName: { "zh-TW": "小狐" } });
    await fake.initialize(host);
    await gw.registerInstance(fake, "primary-companion", { instanceId: PRIMARY_INSTANCE_ID });
    const ctx = { name: "小狐", characterId: "fox-text", entrypoint: "text" as const, tuning: { speed: 1 } };
    const boot = prefsWith({ companionPack: "fox-text", companionPreferences: { "fox-text": { variant: "night", whatever: 7 } } });
    expect(gw.reconfigure(PRIMARY_INSTANCE_ID, adapterReconfigureFor(boot, ctx))).toBe(true);
    expect(fake.calls).toHaveLength(1);
    expect(fake.calls[0]).toMatchObject({ name: "小狐", preferences: { variant: "night", whatever: 7 }, variant: "night" });
    expect(fake.calls[0]).not.toHaveProperty("palette");

    const next = prefsWith({ companionPack: "fox-text", companionPreferences: { "fox-text": { variant: "day", whatever: 7, extra: "x" } } });
    const plan = companionReloadPlan(boot, next);
    expect(plan).toEqual({ action: "live", changed: ["companionPreferences"] });
    expect(gw.reconfigure(PRIMARY_INSTANCE_ID, adapterReconfigureFor(next, ctx))).toBe(true);
    expect(fake.calls).toHaveLength(2);
    expect(fake.calls[1]).toMatchObject({ preferences: { variant: "day", whatever: 7, extra: "x" }, variant: "day" });
    expect(gw.getInstance(PRIMARY_INSTANCE_ID)?.state).toBe("ready");
  });

  it("三個 reference adapter 對未知鍵都不擲例外；shu-rig 的 palette 真的換配色、未知配色維持原狀", async () => {
    const weird = { preferences: { variant: "neon", nested: { a: 1 } }, variant: "neon", palette: 42, whatever: () => 1, familiars: "nope" };
    const text = new TextCharacterAdapter();
    await text.initialize(host);
    expect(() => text.reconfigure(weird)).not.toThrow();
    const sprite = new SpriteCharacterAdapter({ pack, assetBase: "/packs/shu-standard", renderer: new FakeRenderer() });
    await sprite.initialize(host);
    expect(() => sprite.reconfigure(weird)).not.toThrow();
    const stage = makeStage();
    const shu = new ShuCharacterAdapter({ manifest: shuManifest(), stage });
    await shu.initialize(host);
    expect(() => shu.reconfigure(weird)).not.toThrow();
    expect(stage.currentPalette()).toBe("maid-classic");
    shu.reconfigure(adapterReconfigureFor(prefsWith({ companionPreferences: { "shu-maid": { variant: "maid-dusk" } } }), { name: "小樞", characterId: "shu-maid", entrypoint: "shu-rig", tuning: null }));
    expect(stage.currentPalette()).toBe("maid-dusk");
    shu.reconfigure({ palette: "neon", preferences: { variant: "neon" }, variant: "neon" });
    expect(stage.currentPalette()).toBe("maid-dusk");
    shu.reconfigure(adapterReconfigureFor(prefsWith({ companionPreferences: { "shu-maid": { variant: "maid-sakura" } } }), { name: "小樞", characterId: "shu-maid", entrypoint: "shu-rig", tuning: null }));
    expect(stage.currentPalette()).toBe("maid-sakura");
  });

  it("Gateway 把 adapter 在 reconfigure 擲出的例外當 crash（退回文字的既有路徑），所以偏好負載本身必須先淨化", async () => {
    const gw = new CharacterGateway({ now: () => 0, onSystemText: () => {} });
    const throwing: CharacterAdapter = Object.assign(new TextCharacterAdapter({ characterId: "boom" }), {
      reconfigure() {
        throw new Error("bad prefs");
      },
    });
    await throwing.initialize(host);
    await gw.registerInstance(throwing, "primary-companion", { instanceId: PRIMARY_INSTANCE_ID });
    expect(gw.reconfigure(PRIMARY_INSTANCE_ID, { preferences: {} })).toBe(false);
    expect(gw.getInstance(PRIMARY_INSTANCE_ID)?.state).toBe("crashed");
  });
});

// ---------------------------------------------------------------------------
// companion-reload 就地套用計畫
// ---------------------------------------------------------------------------

describe("companion-reload：可就地套用的偏好 reconfigure，其餘整頁重載", () => {
  const base = prefsWith({});

  it("只動角色偏好／名字／場景／玩耍開關／使魔／安靜類 → live", () => {
    expect(companionReloadPlan(base, prefsWith({ companionPreferences: { "fox-rig": { variant: "maid-dusk" } } }))).toEqual({ action: "live", changed: ["companionPreferences"] });
    expect(companionReloadPlan(base, prefsWith({ companionName: "阿狐", companionScene: "night", companionPlay: false }))).toEqual({
      action: "live",
      changed: ["companionName", "companionPlay", "companionScene"],
    });
    expect(companionReloadPlan(base, prefsWith({ companionFamiliars: [{ id: "f", name: "n", palette: "maid-dusk" }], companionDoNotDisturb: true, companionProactiveQuietUntil: 99 })).action).toBe("live");
    expect(companionReloadPlan(base, prefsWith({ storyProgress: { intro: true }, companionBubbles: false, companionSound: true, companionDragEnabled: false, companionApproach: false })).action).toBe("live");
  });

  it("動了角色／persona／表現度／尺寸或不認得的鍵 → reload；沒有快照可比 → reload", () => {
    expect(companionReloadPlan(base, prefsWith({ companionPack: "shu-maid" }))).toEqual({ action: "reload", changed: ["companionPack"] });
    expect(companionReloadPlan(base, prefsWith({ companionPersona: "persona-navigator" })).action).toBe("reload");
    expect(companionReloadPlan(base, prefsWith({ companionExpressiveness: "lively" })).action).toBe("reload");
    expect(companionReloadPlan(base, prefsWith({ companionSize: [260, 260] })).action).toBe("reload");
    expect(companionReloadPlan(base, { ...prefsWith({ companionName: "x" }), futureKnob: 1 } as never)).toEqual({ action: "reload", changed: ["companionName", "futureKnob"] });
    expect(companionReloadPlan(null, base)).toEqual({ action: "reload", changed: [] });
    expect(companionReloadPlan(base, undefined)).toEqual({ action: "reload", changed: [] });
  });

  it("host 自己套用的欄位（可見／位置／透明度／置頂）變了不需要視窗做事；沒變就沒變（鍵序不同也算相等）", () => {
    expect(companionReloadPlan(base, prefsWith({ companionVisible: false, companionOpacity: 0.5, companionPosition: [1, 2], companionAlwaysOnTop: false }))).toEqual({
      action: "live",
      changed: ["companionAlwaysOnTop", "companionOpacity", "companionPosition", "companionVisible"],
    });
    expect(companionReloadPlan(base, { ...base })).toEqual({ action: "live", changed: [] });
    const a = prefsWith({ companionPreferences: { fox: { a: 1, b: "x" } } });
    const b = prefsWith({ companionPreferences: { fox: { b: "x", a: 1 } } });
    expect(companionReloadPlan(a, b)).toEqual({ action: "live", changed: [] });
    expect(companionReloadPlan(a, prefsWith({ companionPreferences: { fox: { a: 1, b: "y" } } })).changed).toEqual(["companionPreferences"]);
    // 兩張表互斥，而且都不含 companionPack／companionSize（那些必須重載）。
    for (const k of LIVE_PREF_KEYS) expect(HOST_APPLIED_PREF_KEYS).not.toContain(k);
    for (const k of ["companionPack", "companionPersona", "companionExpressiveness", "companionSize"]) {
      expect(LIVE_PREF_KEYS).not.toContain(k);
      expect(HOST_APPLIED_PREF_KEYS).not.toContain(k);
    }
  });
});
