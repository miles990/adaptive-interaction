// CompanionApp ↔ CharacterGateway ↔ Runtime 的接線（純函式，無 React、無 I/O）。
//
// 這裡決定：daemon 有沒有 characterProtocol（走 CPP 還是舊路徑）、選哪個角色
// （索引＋prefs.companionPack；8 個舊 id 永遠有效；匯入角色由 host 清單認領）、
// 哪些 character.intent 是給這個視窗的、回執怎麼轉給 Runtime（去掉廣播後綴、
// 帶 Runtime 的世代）、本機互動事件怎麼變成 CPP 輸入事件、hello 怎麼建、
// 各角色偏好（prefs.companionPreferences[characterId]）怎麼變成 reconfigure 負載、
// companion-reload 時哪些偏好能就地套用。CompanionApp 只接線不判斷，
// 所以這些判斷可以在 vitest 裡逐條驗。

import type { AdapterInputEvent } from "../character/adapter";
import { baseMessageId } from "../character/gateway";
import { displayNameOf, validateCharacterManifest } from "../character/manifest";
import {
  CHARACTER_INTENTS,
  CharacterManifest,
  CharacterRole,
  CommandReceipt,
  Hello,
  IntentEnvelope,
  isCharacterIntent,
  LIMITS,
  PROTOCOL_VERSION,
} from "../character/protocol";
import type { CharacterIndex, CharacterIndexEntry } from "../character/registry";
import type { DesktopPrefs, ImportedCharacterEntry } from "../desktop";
import { sanitizeMemory, type InteractionMemory } from "./interactionMemory";
import { validateManifest as validateLegacyPack, type PackManifest } from "./renderer";
import { LEGACY_CHARACTER_IDS } from "./settingsTransfer";

/** 桌面視窗主角的固定 instanceId（Runtime 的 hello／targets 也用這個）。 */
export const PRIMARY_INSTANCE_ID = "desktop-companion";

/** 角色載入失敗時顯示在可信元素上的固定文案（不是 adapter 說的）。 */
export const CHARACTER_LOAD_FAILED_LINE = "角色載入失敗，改用文字顯示";

// ---------------------------------------------------------------------------
// Runtime feed：protocol（character.intent）或 legacy（mapRuntimeEvent）
// ---------------------------------------------------------------------------

export type RuntimeFeed = "protocol" | "legacy";

/** `/v1/status` 有 `characterProtocol` 物件 → 這個 daemon 會投影 character.intent。 */
export function selectRuntimeFeed(status: unknown): RuntimeFeed {
  if (!status || typeof status !== "object") return "legacy";
  const cp = (status as Record<string, unknown>)["characterProtocol"];
  return cp && typeof cp === "object" ? "protocol" : "legacy";
}

// ---------------------------------------------------------------------------
// 角色選擇
// ---------------------------------------------------------------------------

export type EntrypointKind = "shu-rig" | "sprite" | "text";

/**
 * host 匯入清單的一列（desktop.characterListImported）。`manifest` 是可選的完整 manifest：
 * 目前 `character_list_imported` 只回摘要（displayName／entrypoint／旗標／資產 id），
 * 沒有 manifest 本文；host 之後若夾帶就直接用（sprite 的 x-legacy 版型、
 * shu-rig 的 variants／preferencesSchema 都靠它）。
 */
export type ImportedCharacterListing = ImportedCharacterEntry & { manifest?: unknown };

/** 匯入 sprite 角色的版型（由 manifest 的 x-legacy 派生）＋ sheet 資產 id（經 host 讀成 data URL）。 */
export interface ImportedSpriteShape {
  pack: PackManifest;
  sheetAssetId: string;
}

export type CharacterSource =
  | { kind: "index"; entry: CharacterIndexEntry; characterId: string }
  | { kind: "legacy-pack"; characterId: string }
  | {
      kind: "imported";
      entry: ImportedCharacterListing;
      characterId: string;
      entrypoint: EntrypointKind;
      /** 清單有夾帶且驗證通過的 manifest；沒有就是 null（text／shu-rig 仍可由摘要建出）。 */
      manifest: CharacterManifest | null;
      /** 只有 sprite 才有。 */
      sprite?: ImportedSpriteShape;
    }
  | {
      kind: "text";
      characterId: string;
      reason: string;
      /** true＝這是「載入失敗」的退路（host 要顯示固定文案），不是使用者自己選的文字角色。 */
      failed?: boolean;
    };

/**
 * 從 /characters/index.json＋prefs.companionPack（＋host 匯入清單）選角色：
 *   1. 偏好的 id 在索引裡 → 用它；
 *   2. 索引載入失敗但偏好是 8 個舊 id 之一 → 由 /packs/<id> 遷移；
 *   3. 偏好的 id 在匯入清單裡 → 匯入角色（壞掉／不在白名單 → 文字角色＋原因）；
 *   4. 否則索引的 default；5. 什麼都沒有 → 文字角色。
 * `imported` 為 null＝沒問過 host（瀏覽器模式或不需要）。
 */
export function selectCharacterSource(
  index: CharacterIndex | null,
  preferred: string | null | undefined,
  imported: readonly ImportedCharacterListing[] | null = null
): CharacterSource {
  const want = typeof preferred === "string" && preferred.length > 0 ? preferred : null;
  const listed = want !== null && Array.isArray(imported) && imported.some((e) => e && e.characterId === want);
  if (index) {
    const hit = want ? index.characters.find((c) => c.characterId === want) : undefined;
    if (hit) return { kind: "index", entry: hit, characterId: hit.characterId };
    if (want && LEGACY_CHARACTER_IDS.includes(want)) return { kind: "legacy-pack", characterId: want };
    if (want && listed) return importedCharacterSource(imported, want);
    const def = index.characters.find((c) => c.characterId === index.default);
    if (def) return { kind: "index", entry: def, characterId: def.characterId };
  }
  if (want && LEGACY_CHARACTER_IDS.includes(want)) return { kind: "legacy-pack", characterId: want };
  if (want && listed) return importedCharacterSource(imported, want);
  return { kind: "text", characterId: "plain-text", reason: index ? "no usable character in index" : "character index unavailable" };
}

// ---------------------------------------------------------------------------
// 已匯入角色（host 本機角色資料夾；只認 in-process＋builtin 白名單，任何不符 → 文字角色）
// ---------------------------------------------------------------------------

export function isBuiltinEntrypointId(id: unknown): id is EntrypointKind {
  return id === "shu-rig" || id === "sprite" || id === "text";
}

/** 偏好的 id 不在索引、也不是 8 個舊 id → 才需要問 host 的匯入清單（Tauri 才有本機角色資料夾）。 */
export function needsImportedLookup(index: CharacterIndex | null, preferred: string | null | undefined): boolean {
  const want = typeof preferred === "string" && preferred.length > 0 ? preferred : null;
  if (!want) return false;
  if (LEGACY_CHARACTER_IDS.includes(want)) return false;
  if (index && index.characters.some((c) => c.characterId === want)) return false;
  return true;
}

/** 錯誤原因：限長、隱藏任何像絕對路徑的片段（host 的錯誤本來就不回顯路徑；這裡再保險一次）。 */
function shortReason(e: unknown): string {
  const raw = typeof e === "string" ? e : e instanceof Error ? e.message : e === undefined || e === null ? "" : String(e);
  const cleaned = raw
    .replace(/[A-Za-z]:\\[^\s"']+/g, "（路徑已隱藏）")
    .replace(/(?:\/[^\s/"']+){2,}\/?/g, "（路徑已隱藏）")
    .slice(0, 120);
  return cleaned.length > 0 ? cleaned : "manifest unreadable";
}

function loadFailure(characterId: string, reason: string): CharacterSource {
  return { kind: "text", characterId: "plain-text", reason: `${characterId}: ${reason}`, failed: true };
}

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return !!v && typeof v === "object" && !Array.isArray(v);
}

/** manifest 的 vendor 擴充（`x-legacy`；TS 遷移器寫的是 `legacy`）；驗證器保留未知頂層欄位。 */
function extensionOf(manifest: CharacterManifest, key: "x-legacy" | "legacy"): Record<string, unknown> | null {
  const v = (manifest as unknown as Record<string, unknown>)[key];
  return isPlainObject(v) ? v : null;
}

/**
 * 匯入清單裡找一個角色。只接受 valid、in-process、builtin 白名單 entrypoint、
 * 不需要可執行程式／網路的項目；清單若夾帶 manifest 就驗證並核對 id／entrypoint；
 * sprite 還要能從 x-legacy 派生版型並找到 sheet 資產。任何不符 → 文字角色（failed＋原因）。
 */
export function importedCharacterSource(
  entries: readonly ImportedCharacterListing[] | null | undefined,
  characterId: string
): CharacterSource {
  const entry = Array.isArray(entries) ? entries.find((e) => e && e.characterId === characterId) : undefined;
  if (!entry) return loadFailure(characterId, "imported character not found");
  if (entry.valid !== true) return loadFailure(characterId, `imported character invalid: ${shortReason(entry.error)}`);
  if (entry.external === true || (entry.adapterKind !== undefined && entry.adapterKind !== "in-process")) {
    return loadFailure(characterId, "imported character is not in-process");
  }
  if (entry.executable === true || entry.network === true) {
    return loadFailure(characterId, "imported character requires executable code or network (not granted to builtin adapters)");
  }
  if (!isBuiltinEntrypointId(entry.entrypoint)) {
    return loadFailure(characterId, "imported entrypoint is not builtin shu-rig/sprite/text");
  }
  let manifest: CharacterManifest | null = null;
  if (entry.manifest !== undefined && entry.manifest !== null) {
    const v = validateCharacterManifest(entry.manifest);
    if (!v.ok) return loadFailure(characterId, "imported manifest invalid");
    if (v.manifest.characterId !== characterId) return loadFailure(characterId, "imported manifest characterId mismatch");
    if (entrypointKindOf(v.manifest) !== entry.entrypoint) return loadFailure(characterId, "imported manifest entrypoint mismatch");
    manifest = v.manifest;
  }
  if (entry.entrypoint === "sprite") {
    if (!manifest) {
      return loadFailure(characterId, "imported sprite needs its manifest (x-legacy pack shape) but the host list carries none");
    }
    const sprite = spritePackFromManifest(manifest);
    if (!sprite) return loadFailure(characterId, "imported sprite manifest has no usable x-legacy pack shape or sheet asset");
    return { kind: "imported", entry, characterId, entrypoint: "sprite", manifest, sprite };
  }
  return { kind: "imported", entry, characterId, entrypoint: entry.entrypoint, manifest };
}

const MAX_SHEET_COLUMNS = 4096;

/**
 * CPP manifest（sprite）→ 舊 PackManifest 版型：欄位來自 `x-legacy`（Rust 遷移器）或 `legacy`
 * 擴充（frameSize／anchor／sheet／columns／animations／可選 anchors），id／name 來自 manifest；
 * 再用既有的 validateManifest 驗一次。sheet 必須是 manifest 宣告的資產（id "sheet" 或同路徑）。
 */
export function spritePackFromManifest(manifest: CharacterManifest): ImportedSpriteShape | null {
  if (entrypointKindOf(manifest) !== "sprite") return null;
  const ext = extensionOf(manifest, "x-legacy") ?? extensionOf(manifest, "legacy");
  if (!ext || ext.kind !== "character-pack") return null;
  const candidate: Record<string, unknown> = {
    schemaVersion: typeof ext.schemaVersion === "string" && ext.schemaVersion.length > 0 ? ext.schemaVersion : "1.0",
    kind: "character-pack",
    id: manifest.characterId,
    name: manifest.displayName,
    ...(manifest.description ? { description: manifest.description } : {}),
    ...(manifest.author ? { author: manifest.author } : {}),
    version: manifest.version,
    frameSize: ext.frameSize,
    anchor: ext.anchor,
    sheet: ext.sheet,
    columns: ext.columns,
    animations: ext.animations,
    ...(isPlainObject(ext.anchors) ? { anchors: ext.anchors } : {}),
  };
  if (validateLegacyPack(candidate).length > 0) return null;
  const columns = candidate.columns;
  if (typeof columns !== "number" || !Number.isInteger(columns) || columns <= 0 || columns > MAX_SHEET_COLUMNS) return null;
  const dims = [...(candidate.frameSize as unknown[]), ...(candidate.anchor as unknown[])];
  if (dims.some((n) => typeof n !== "number" || !Number.isFinite(n))) return null;
  const sheet = candidate.sheet as string;
  const asset = manifest.assets.find((a) => a.id === "sheet") ?? manifest.assets.find((a) => a.path === sheet);
  if (!asset) return null;
  if (typeof asset.mediaType === "string" && !asset.mediaType.startsWith("image/")) return null;
  return { pack: candidate as unknown as PackManifest, sheetAssetId: asset.id };
}

/** host 讀出的資產必須是影像 data URL（`data:image/<type>;base64,…`）才拿去當 sheet。 */
export function isImageDataUrl(value: unknown): value is string {
  // 只看開頭（payload 可到 8 MB，不整串跑正則）：影像 MIME＋base64＋至少一個 payload 字元。
  return typeof value === "string" && /^data:image\/[a-z0-9.+-]+;base64,[A-Za-z0-9+/]/i.test(value);
}

/** shu-rig 的三種配色（rig/params RIG_PALETTES 的鍵；manifest variants 用同一組 id）。 */
export const SHU_RIG_PALETTES = ["maid-classic", "maid-dusk", "maid-sakura"] as const;

export function isShuRigPalette(value: unknown): value is (typeof SHU_RIG_PALETTES)[number] {
  return typeof value === "string" && (SHU_RIG_PALETTES as readonly string[]).includes(value);
}

/**
 * 匯入 shu-rig 的初始配色：x-legacy／legacy.palette → preferencesSchema 的 variant／palette
 * 預設值 → variants[0] → maid-classic。只接受白名單裡的配色名（未知名稱不猜）。
 */
export function rigPaletteForImported(manifest: CharacterManifest | null): string {
  if (!manifest) return "maid-classic";
  for (const key of ["x-legacy", "legacy"] as const) {
    const ext = extensionOf(manifest, key);
    if (ext && isShuRigPalette(ext.palette)) return ext.palette;
  }
  const props = manifest.preferencesSchema?.properties ?? {};
  for (const key of ["variant", "palette"]) {
    const d = props[key]?.default;
    if (isShuRigPalette(d)) return d;
  }
  const first = manifest.variants[0]?.id;
  if (isShuRigPalette(first)) return first;
  return "maid-classic";
}

/**
 * 沒有 manifest 本文時，用清單摘要組一個 character-rig 2.0 pack 交給 ShuCharacterAdapter
 * 遷移（characterId＝匯入 id、名字＝清單 displayName）。純資料，不執行任何東西。
 */
export function importedRigPack(entry: Pick<ImportedCharacterListing, "characterId" | "displayName" | "version">, palette: string): Record<string, unknown> {
  const name = isPlainObject(entry.displayName) && Object.keys(entry.displayName).length > 0 ? entry.displayName : { "zh-TW": "角色" };
  return {
    schemaVersion: "2.0",
    kind: "character-rig",
    id: entry.characterId,
    name,
    palette: isShuRigPalette(palette) ? palette : "maid-classic",
    ...(typeof entry.version === "string" && entry.version.length > 0 ? { version: entry.version } : {}),
  };
}

export type ImportedLookup = "skipped" | "done" | "failed";

export interface ResolveCharacterSourceInput {
  index: CharacterIndex | null;
  preferred: string | null | undefined;
  /** 只有桌面版（Tauri）才有本機角色資料夾；瀏覽器模式一律跳過清單查詢。 */
  tauri: boolean;
  listImported: () => Promise<readonly ImportedCharacterListing[]>;
}

/**
 * 選角色（含 host 匯入清單）。永不擲例外：瀏覽器模式或不需要時跳過清單；host 查詢失敗
 * 而偏好正是匯入角色 → 文字角色＋failed（不默默換成預設角色，也不假裝載入成功）。
 */
export async function resolveCharacterSource(
  input: ResolveCharacterSourceInput
): Promise<{ source: CharacterSource; importedLookup: ImportedLookup; detail?: string }> {
  if (!input.tauri || !needsImportedLookup(input.index, input.preferred)) {
    return { source: selectCharacterSource(input.index, input.preferred, null), importedLookup: "skipped" };
  }
  let imported: readonly ImportedCharacterListing[];
  try {
    const list = await input.listImported();
    imported = Array.isArray(list) ? list : [];
  } catch (e) {
    const detail = shortReason(e);
    return {
      source: loadFailure(String(input.preferred), `imported character list unavailable: ${detail}`),
      importedLookup: "failed",
      detail,
    };
  }
  return { source: selectCharacterSource(input.index, input.preferred, imported), importedLookup: "done" };
}

export function entrypointKindOf(manifest: Pick<CharacterManifest, "entrypoint"> | null | undefined): EntrypointKind | null {
  const e = manifest?.entrypoint;
  if (!e || e.kind !== "builtin") return null;
  return e.id === "shu-rig" || e.id === "sprite" || e.id === "text" ? e.id : null;
}

/** canvas 的 CSS class 由 entrypoint 種類決定（不再看 pack id 前綴）。 */
export function cssClassForEntrypoint(kind: EntrypointKind | null): "companion-stage" | "companion-canvas" | "companion-text" {
  if (kind === "shu-rig") return "companion-stage";
  if (kind === "text") return "companion-text";
  return "companion-canvas";
}

/** 索引項目可帶 persona 提示；沒有就用 prefs（persona pack 是資料，不綁角色）。 */
const PACK_ID_RE = /^[a-z0-9][a-z0-9-]{0,63}$/;

function hintOf(entry: object | null | undefined, key: "persona" | "story"): string | null {
  if (!entry || typeof entry !== "object") return null;
  const v = (entry as Record<string, unknown>)[key];
  return typeof v === "string" && PACK_ID_RE.test(v) ? v : null;
}

export function personaIdFor(entry: object | null | undefined, prefsPersona: string | null | undefined): string | null {
  const hint = hintOf(entry, "persona");
  if (hint) return hint;
  return typeof prefsPersona === "string" && PACK_ID_RE.test(prefsPersona) ? prefsPersona : null;
}

/** 故事 pack：索引提示優先；否則由 persona id 派生（persona-shu → story-shu-intro）。 */
export function storyPackIdFor(entry: object | null | undefined, personaId: string | null): string | null {
  const hint = hintOf(entry, "story");
  if (hint) return hint;
  if (!personaId) return null;
  return `story-${personaId.replace(/^persona-/, "")}-intro`;
}

/** 顯示名：使用者取的名字優先，否則 manifest displayName（locale），最後才是中立的「角色」。 */
export function charNameFor(prefsName: string | null | undefined, manifest: Pick<CharacterManifest, "displayName"> | null, locale: string): string {
  const own = typeof prefsName === "string" ? prefsName.trim().slice(0, 24) : "";
  if (own.length > 0) return own;
  return manifest ? displayNameOf(manifest, locale) : "角色";
}

/** rig 配色：bundled manifest 的 legacy.palette，或第一個 variant。 */
export function rigPaletteFor(manifest: CharacterManifest): string {
  const legacy = (manifest as unknown as { legacy?: { palette?: unknown } }).legacy;
  if (legacy && typeof legacy.palette === "string") return legacy.palette;
  return manifest.variants[0]?.id ?? "maid-classic";
}

// ---------------------------------------------------------------------------
// 各角色偏好（prefs.companionPreferences[characterId] → adapter.reconfigure）
// ---------------------------------------------------------------------------

export type CharacterPreferenceValue = boolean | number | string;

/** reconfigure 負載裡的角色偏好片段：`preferences` 原樣透傳（adapter 自己認鍵），`variant` 保留鍵，
 *  shu-rig 的 variant 若是三種配色之一就同時給 `palette`（ShuCharacterAdapter → stage.setPalette）。 */
export interface CharacterPreferenceConfig {
  preferences: Record<string, CharacterPreferenceValue>;
  variant?: string;
  palette?: string;
}

const MAX_PREFERENCE_KEYS = 32;
const MAX_PREFERENCE_KEY_CHARS = 64;
const MAX_PREFERENCE_STRING_CHARS = 200;
const FORBIDDEN_PREFERENCE_KEYS = new Set(["__proto__", "constructor", "prototype"]);

/**
 * 讀 prefs.companionPreferences[characterId]（host 尚未保存這欄位時是 undefined）→ 有界、只含
 * boolean／number／string 的值表；未知鍵保留（adapter 不認就忽略，永不擲例外）。
 */
export function characterPreferencesFor(
  prefs: { companionPreferences?: unknown } | null | undefined,
  characterId: string,
  entrypoint: EntrypointKind | null
): CharacterPreferenceConfig {
  const all = prefs?.companionPreferences;
  const raw = isPlainObject(all) ? all[characterId] : undefined;
  const preferences: Record<string, CharacterPreferenceValue> = {};
  if (isPlainObject(raw)) {
    let count = 0;
    for (const [key, value] of Object.entries(raw)) {
      if (count >= MAX_PREFERENCE_KEYS) break;
      if (key.length === 0 || key.length > MAX_PREFERENCE_KEY_CHARS || FORBIDDEN_PREFERENCE_KEYS.has(key)) continue;
      if (typeof value === "boolean") preferences[key] = value;
      else if (typeof value === "number" && Number.isFinite(value)) preferences[key] = value;
      else if (typeof value === "string") preferences[key] = value.slice(0, MAX_PREFERENCE_STRING_CHARS);
      else continue;
      count += 1;
    }
  }
  const out: CharacterPreferenceConfig = { preferences };
  const variant = preferences["variant"];
  if (typeof variant === "string" && variant.length > 0) {
    out.variant = variant;
    if (entrypoint === "shu-rig" && isShuRigPalette(variant)) out.palette = variant;
  }
  return out;
}

export interface AdapterReconfigureContext {
  /** 已由 charNameFor 決定的顯示名。 */
  name: string;
  characterId: string;
  entrypoint: EntrypointKind | null;
  /** 個性 tuning（host 由 personality＋角色權重表算出）。 */
  tuning: unknown;
}

/**
 * host 偏好 → adapter.reconfigure 的完整負載（既有欄位＋角色偏好＋variant／palette）。
 * 開機與 companion-reload 都用這一份，確保兩條路徑一致。
 */
export function adapterReconfigureFor(
  prefs: Partial<DesktopPrefs> | null | undefined,
  ctx: AdapterReconfigureContext
): Record<string, unknown> {
  const familiars = prefs?.companionFamiliars;
  return {
    name: ctx.name,
    scene: prefs?.companionScene ?? "none",
    play: prefs?.companionPlay !== false,
    cursorPlay: prefs?.companionCursorPlay !== false,
    deskMove: prefs?.companionDeskMove !== false,
    // 「游標靠近時看過來」：舞台注視游標的唯一主人（不再借 cursorPlay 判斷）。
    approach: prefs?.companionApproach !== false,
    familiars: Array.isArray(familiars) ? familiars : [],
    tuning: ctx.tuning,
    ...characterPreferencesFor(prefs, ctx.characterId, ctx.entrypoint),
  };
}

// ---------------------------------------------------------------------------
// companion-reload：哪些偏好可以就地套用（不整頁重載）
// ---------------------------------------------------------------------------

/** 改了只需 reconfigure／更新 ref 的偏好。 */
export const LIVE_PREF_KEYS: readonly (keyof DesktopPrefs)[] = [
  // 角色視窗自己也是互動記憶的寫入者：控制中心按「忘記這些」之後，
  // 視窗手上的副本必須換成 host 的最新值，否則下一次玩玩具會用舊副本
  // 把使用者刪掉的記憶整包寫回去（「忘記」變成沒有真的忘記）。
  "companionInteractionMemory",
  "companionName",
  "companionScene",
  "companionPlay",
  "companionCursorPlay",
  "companionDeskMove",
  "companionFamiliars",
  "companionPreferences",
  "companionDoNotDisturb",
  "companionBubbles",
  "companionSound",
  "companionDragEnabled",
  "companionApproach",
  "companionProactiveQuietUntil",
  "storyProgress",
];

/** host（Rust）自己套用的偏好：變了不需要視窗做任何事。
 *  注意：只放「視窗不是寫入者」的鍵。視窗自己也會寫的鍵放這裡＝兩份副本各自
 *  read-modify-write，最後一個寫的人會蓋掉另一邊（互動記憶就出過這個包）。 */
export const HOST_APPLIED_PREF_KEYS: readonly (keyof DesktopPrefs)[] = [
  "closeBehavior",
  "askOnClose",
  "launchAtLogin",
  "showCompanionOnStart",
  "openControlCenterOnStart",
  "companionVisible",
  "companionPosition",
  "companionOpacity",
  "companionAlwaysOnTop",
  "schemaVersion",
];

export interface CompanionReloadPlan {
  action: "reload" | "live";
  /** 有變動的鍵（排序）。 */
  changed: string[];
}

function stableEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a !== typeof b || a === null || b === null || typeof a !== "object") return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  if (Array.isArray(a)) {
    const bb = b as unknown[];
    return a.length === bb.length && a.every((v, i) => stableEqual(v, bb[i]));
  }
  const ka = Object.keys(a as object).sort();
  const kb = Object.keys(b as object).sort();
  if (ka.length !== kb.length || ka.some((k, i) => k !== kb[i])) return false;
  return ka.every((k) => stableEqual((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k]));
}

/**
 * companion-reload 事件（host 每次 companion_apply_prefs 都會發）：比對開機／上次的快照與最新偏好。
 * 只動了可就地套用的鍵 → "live"；動了角色／persona／表現度／尺寸或任何不認得的鍵，
 * 或沒有快照可比 → "reload"（維持整頁重載的既有行為）。
 */
export function companionReloadPlan(
  prev: Partial<DesktopPrefs> | null | undefined,
  next: Partial<DesktopPrefs> | null | undefined
): CompanionReloadPlan {
  if (!isPlainObject(prev) || !isPlainObject(next)) return { action: "reload", changed: [] };
  const a = prev as Record<string, unknown>;
  const b = next as Record<string, unknown>;
  const keys = new Set([...Object.keys(a), ...Object.keys(b)]);
  const changed = [...keys].filter((k) => !stableEqual(a[k], b[k])).sort();
  const live = new Set<string>(LIVE_PREF_KEYS);
  const host = new Set<string>(HOST_APPLIED_PREF_KEYS);
  const needsReload = changed.some((k) => !live.has(k) && !host.has(k));
  return { action: needsReload ? "reload" : "live", changed };
}

/**
 * companion-reload 之後，這個視窗手上的互動記憶副本。
 *
 * 互動記憶有兩個寫入者（控制中心的「忘記這些」／關掉反應，以及角色視窗的玩玩具），
 * 而 host 的 prefs patch 是整個欄位覆蓋。所以每次 host 的偏好變動，視窗都必須把
 * 自己的副本換成 host 的最新值——否則使用者刪掉的記憶會在下一次寫回時復活。
 * host 那一份永遠是唯一真相；這裡只做有界淨化。
 */
export function interactionMemoryFromPrefs(
  prefs: Partial<DesktopPrefs> | null | undefined
): InteractionMemory {
  return sanitizeMemory(prefs?.companionInteractionMemory);
}

// ---------------------------------------------------------------------------
// character.intent 事件 → 這個視窗的 envelope
// ---------------------------------------------------------------------------

/**
 * `character.intent` payload = { envelope, targets }。只有 targets 含我們的
 * instanceId（或通用的 "desktop-companion"）才派送；派送時把 envelope 的
 * characterInstanceId 對齊本機 Gateway 的 instanceId。
 */
export function envelopeForInstance(payload: unknown, instanceId: string): IntentEnvelope | null {
  if (!payload || typeof payload !== "object") return null;
  const p = payload as Record<string, unknown>;
  const env = p["envelope"];
  if (!env || typeof env !== "object") return null;
  const targets = Array.isArray(p["targets"]) ? (p["targets"] as unknown[]).filter((t): t is string => typeof t === "string") : [];
  if (!targets.includes(instanceId) && !targets.includes(PRIMARY_INSTANCE_ID)) return null;
  const e = env as Record<string, unknown>;
  if (typeof e.messageId !== "string" || !isCharacterIntent(e.intent)) return null;
  return { ...(env as IntentEnvelope), characterInstanceId: instanceId };
}

// ---------------------------------------------------------------------------
// 回執 → Runtime
// ---------------------------------------------------------------------------

/** Gateway 自己派生的本機命令（resume-previous `~r`、return-idle `~idle`）：Runtime 沒有對應 pending，不轉送。 */
export function isLocalOnlyMessageId(messageId: string): boolean {
  return baseMessageId(messageId).includes("~");
}

/**
 * 只轉送主角實例的回執；去掉廣播後綴 `@instance`；世代改成 Runtime hello 回
 * 給我們的 generation（本機 Gateway 的世代與 Runtime 不同步，直接送會被當 stale）。
 */
export function receiptForRuntime(
  receipt: CommandReceipt,
  primaryInstanceId: string,
  runtimeGeneration: number | null
): CommandReceipt | null {
  if (receipt.characterInstanceId !== primaryInstanceId) return null;
  if (isLocalOnlyMessageId(receipt.messageId)) return null;
  return {
    ...receipt,
    messageId: baseMessageId(receipt.messageId),
    characterInstanceId: primaryInstanceId,
    generation: runtimeGeneration ?? receipt.generation,
  };
}

// ---------------------------------------------------------------------------
// hello（給 adapter.negotiate 與 POST /v1/character/hello 用的同一份）
// ---------------------------------------------------------------------------

export function helloFor(
  instanceId: string,
  role: CharacterRole,
  opts: { reducedMotion: boolean; locale?: string; runtimeVersion?: string }
): Hello {
  return {
    type: "hello",
    protocolVersion: PROTOCOL_VERSION,
    runtimeVersion: opts.runtimeVersion ?? "0.5.0-dev",
    characterInstanceId: instanceId,
    role,
    locale: opts.locale ?? "zh-TW",
    reducedMotion: opts.reducedMotion,
    requires: [...CHARACTER_INTENTS],
    limits: {
      maxMessageBytes: LIMITS.maxMessageBytes,
      maxMessagesPerSecond: LIMITS.maxMessagesPerSecond,
      maxPending: LIMITS.maxPending,
    },
  };
}

// ---------------------------------------------------------------------------
// 本機互動 kind → CPP 輸入事件
// ---------------------------------------------------------------------------

function basename(p: unknown): string {
  return typeof p === "string" ? (p.split(/[\\/]/).pop() ?? "") : "";
}

/**
 * CompanionApp 既有的互動 kind → CharacterInputEvent 原料（Gateway 再正規化）。
 * 回 null 的 kind 在 CPP 沒有對應（例如 bubble-shown 純遙測），protocol 模式下
 * 不送。file drop 只帶檔名（metadata only；不帶路徑）。
 */
export function inputEventFor(kind: string, extra: Record<string, unknown> = {}): AdapterInputEvent | null {
  switch (kind) {
    case "companion-clicked":
      return { kind: "character.clicked", payload: {} };
    case "companion-double-clicked":
      return { kind: "character.double-clicked", payload: {} };
    case "companion-dragged":
      return { kind: "character.drag-started", payload: {} };
    case "companion-dropped-at":
      return { kind: "character.dropped", payload: {} };
    case "pointer-approached":
      return { kind: "character.hover-entered", payload: {} };
    case "pointer-left":
      return { kind: "character.hover-left", payload: {} };
    case "action-selected": {
      const action = typeof extra.action === "string" ? extra.action : "";
      return action ? { kind: "character.action-requested", payload: { action } } : null;
    }
    case "text-submitted": {
      const text = typeof extra.text === "string" ? extra.text : "";
      return text ? { kind: "character.text-submitted", payload: { text }, privacyClass: "personal" } : null;
    }
    case "companion-dropped": {
      // 原料：檔名（不帶路徑）＋host 若知道就帶大小／類型（Tauri 拖放事件目前只給路徑；
      // 不知道就不補假值——Gateway 正規化時才依 README §6 形狀補預設）。
      const paths = Array.isArray(extra.attachments) ? extra.attachments : [];
      const meta = Array.isArray(extra.files) ? (extra.files as unknown[]) : [];
      const files = paths
        .map((p, i) => {
          const m = meta[i] && typeof meta[i] === "object" ? (meta[i] as Record<string, unknown>) : {};
          const out: Record<string, unknown> = { name: basename(p) };
          if (typeof m.bytes === "number" && Number.isFinite(m.bytes) && m.bytes >= 0) out.bytes = m.bytes;
          if (typeof m.mediaType === "string" && m.mediaType.length > 0) out.mediaType = m.mediaType;
          return out;
        })
        .filter((f) => (f.name as string).length > 0);
      return files.length > 0 ? { kind: "character.file-dropped", payload: { files }, privacyClass: "personal" } : null;
    }
    case "toy-thrown": {
      const toyId = typeof extra.toyId === "string" ? extra.toyId : undefined;
      return { kind: "character.toy-thrown", payload: toyId ? { toyId } : {} };
    }
    case "dismissed":
      return { kind: "character.dismissed", payload: {} };
    case "visibility-changed":
      return { kind: "character.visibility-changed", payload: { visible: extra.visible === true } };
    default:
      return null;
  }
}

// ---------------------------------------------------------------------------
// Runtime 的 character.system-text 事件 → 可信文字元素
// ---------------------------------------------------------------------------

export function systemTextFromEvent(payload: unknown): { text: string; marker: "verified" | "none"; instanceId: string | null } | null {
  if (!payload || typeof payload !== "object") return null;
  const p = payload as Record<string, unknown>;
  const message = typeof p.message === "string" ? p.message.trim().slice(0, 200) : "";
  if (!message) return null;
  const instanceId = typeof p.instanceId === "string" ? p.instanceId : null;
  // 綠勾只認 Runtime 給的 truthState verified；文字本身不能宣稱驗證。
  const marker = p.truthState === "verified" && p.intent === "verified-success" ? "verified" : "none";
  return { text: message, marker, instanceId };
}

// ---------------------------------------------------------------------------
// 轉送結果彙總（file-drop 一檔一事件 → 一個對使用者的答覆）
// ---------------------------------------------------------------------------

export interface ForwardDecision {
  decision: string;
  reason?: string;
}

/**
 * 多則輸入事件各自的 Runtime 決定 → 一個誠實的總結：任何一則沒送到（null）＝
 * runtime unreachable；任何一則被丟掉＝dropped（帶原因）；否則 queued。
 * 空清單＝沒有東西送出去（呼叫端當失敗處理）。
 */
export function summarizeForwardDecisions(results: readonly (ForwardDecision | null)[]): ForwardDecision | null {
  if (results.length === 0) return null;
  if (results.some((r) => r === null)) return null;
  const dropped = results.find((r) => r && r.decision === "dropped");
  if (dropped) return dropped;
  const first = results[0]!;
  return { decision: first.decision, reason: first.reason };
}

// ---------------------------------------------------------------------------
// 拖放預覽（§5.2：檔名、大小、類型、資料去向與可讀 Agent）
// ---------------------------------------------------------------------------

export interface DropPreviewItem {
  name: string;
  /** 不知道就是 null（Tauri 拖放事件只給路徑；不猜、不補 0）。 */
  bytes: number | null;
  mediaType: string | null;
}

export interface DropPreviewSession {
  sessionId: string;
  label?: string;
  agentId: string;
  dataScope: string[];
  closedAt?: string | null;
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "未知";
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

/** 一個項目的預覽行：`name（1.2 KB・text/plain）`；不知道就明說「大小／類型：未知」。 */
export function dropItemLine(item: DropPreviewItem): string {
  const size = item.bytes === null ? null : formatBytes(item.bytes);
  const type = item.mediaType;
  if (size === null && type === null) return `${item.name}（大小／類型：未知）`;
  return `${item.name}（${size ?? "大小未知"}・${type ?? "類型未知"}）`;
}

/** 拖放事件（Tauri `paths`；若 host 夾帶 files metadata 也吃）→ 預覽項目。 */
export function dropPreviewItems(paths: readonly string[], meta: readonly unknown[] = []): DropPreviewItem[] {
  return paths
    .map((p, i) => {
      const m = meta[i] && typeof meta[i] === "object" ? (meta[i] as Record<string, unknown>) : {};
      return {
        name: basename(p),
        bytes: typeof m.bytes === "number" && Number.isFinite(m.bytes) && m.bytes >= 0 ? m.bytes : null,
        mediaType: typeof m.mediaType === "string" && m.mediaType.length > 0 ? m.mediaType : null,
      };
    })
    .filter((f) => f.name.length > 0);
}

/**
 * 資料去向與可讀 Agent 的說明。`sessions` 為 null＝清單沒拿到（明說，不當成「沒有」）。
 * 拖放記錄只進本機 Runtime 的觀察紀錄；哪些 AI 工作階段之後讀得到，由各階段的可讀範圍決定。
 */
export function dropDestinationLines(sessions: readonly DropPreviewSession[] | null): string[] {
  const lines = ["去向：本機 Runtime（觀察紀錄）・只記錄檔案位置，不讀取內容、不上傳、不離開本機。"];
  if (sessions === null) {
    lines.push("可讀的 AI 工作階段：清單暫時拿不到（確認前不會交給任何 Agent）。");
    return lines;
  }
  const open = sessions.filter((s) => !s.closedAt);
  if (open.length === 0) {
    lines.push("可讀的 AI 工作階段：目前沒有開啟中的工作階段（只留在本機紀錄）。");
    return lines;
  }
  for (const s of open.slice(0, 4)) {
    const scope = s.dataScope.length > 0 ? s.dataScope.join("、") : "未設定";
    lines.push(`可讀：${s.label ?? s.agentId}・可讀範圍：${scope}`);
  }
  if (open.length > 4) lines.push(`…等 ${open.length} 個工作階段`);
  return lines;
}

// ---------------------------------------------------------------------------
// 重新 hello（Runtime 端斷線／重啟後角色不能永遠收不到 character.intent）
// ---------------------------------------------------------------------------

/** 兩次 hello 嘗試的最小間隔（節流：instance 事件可能連發）。 */
export const REHELLO_MIN_INTERVAL_MS = 2_000;

export interface HelloTracker {
  /** 上一次 hello 成功（Runtime 回了 generation）。 */
  sent: boolean;
  /** 上一次嘗試 hello 的時間（epoch ms；0＝從未）。 */
  lastAttemptAt: number;
  /** 上一次看到的 `/v1/status.startedAt`（daemon 實例身分）；null＝還沒看過。 */
  runtimeStartedAt: string | null;
}

export const INITIAL_HELLO_TRACKER: HelloTracker = { sent: false, lastAttemptAt: 0, runtimeStartedAt: null };

export type RehelloReason =
  | "feed-appeared"
  | "hello-not-sent"
  | "runtime-restarted"
  | "instance-disconnected";

export interface RehelloDecision {
  hello: boolean;
  reason: RehelloReason | null;
  tracker: HelloTracker;
  /** 被節流（想 hello 但距上次嘗試太近）；下一次 status 輪詢會再試。 */
  throttled: boolean;
}

function rehello(tracker: HelloTracker, reason: RehelloReason | null, nowMs: number, force = false): RehelloDecision {
  if (!reason) return { hello: false, reason: null, tracker, throttled: false };
  if (!force && nowMs - tracker.lastAttemptAt < REHELLO_MIN_INTERVAL_MS) {
    // 節流：把 sent 標成 false，讓下一次輪詢一定重試。
    return { hello: false, reason, tracker: { ...tracker, sent: false }, throttled: true };
  }
  return { hello: true, reason, tracker: { ...tracker, sent: false, lastAttemptAt: nowMs }, throttled: false };
}

/**
 * 每次 `/v1/status` 輪詢：要不要（重新）hello。
 *   - feed 由 legacy 變 protocol（daemon 換了／升級了）→ hello（不節流）。
 *   - 上次 hello 沒成功 → 再試。
 *   - `startedAt` 變了（外部 daemon 重啟：Runtime 端的 instance 表是空的）→ hello。
 * 回傳的 tracker 已記下 startedAt。
 */
export function rehelloOnStatus(
  tracker: HelloTracker,
  prevFeed: RuntimeFeed | null,
  status: unknown,
  nowMs: number
): RehelloDecision & { feed: RuntimeFeed } {
  const feed = selectRuntimeFeed(status);
  const startedAt =
    status && typeof status === "object" && typeof (status as Record<string, unknown>).startedAt === "string"
      ? ((status as Record<string, unknown>).startedAt as string)
      : null;
  const restarted = startedAt !== null && tracker.runtimeStartedAt !== null && startedAt !== tracker.runtimeStartedAt;
  const next: HelloTracker = { ...tracker, runtimeStartedAt: startedAt ?? tracker.runtimeStartedAt };
  if (feed !== "protocol") return { hello: false, reason: null, tracker: next, throttled: false, feed };
  if (prevFeed !== "protocol") return { ...rehello(next, "feed-appeared", nowMs, true), feed };
  if (restarted) return { ...rehello(next, "runtime-restarted", nowMs, true), feed };
  if (!next.sent) return { ...rehello(next, "hello-not-sent", nowMs), feed };
  return { hello: false, reason: null, tracker: next, throttled: false, feed };
}

/**
 * `character.instance` 事件：我們這個實例被 Runtime 標成 connected:false（presence 逾時
 * sweep、adapter 撤銷…）→ 重新 hello（節流 2 秒）。別的實例、或 connected:true 不理。
 */
export function rehelloOnInstanceEvent(
  tracker: HelloTracker,
  payload: unknown,
  instanceId: string,
  nowMs: number
): RehelloDecision {
  if (!payload || typeof payload !== "object") return { hello: false, reason: null, tracker, throttled: false };
  const p = payload as Record<string, unknown>;
  if (p.instanceId !== instanceId) return { hello: false, reason: null, tracker, throttled: false };
  if (p.connected !== false) return { hello: false, reason: null, tracker, throttled: false };
  return rehello(tracker, "instance-disconnected", nowMs);
}
