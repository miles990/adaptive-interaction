// CPP §12 host adapter registry：桌面 host 有哪些 in-process builtin adapter。
//
// v0.6.0 strangler（docs/aip/architecture-boundaries.md §4）：協定核心不認識任何具名角色。
// 「有哪些 builtin adapter」是 **host 的事**，所以：
//   - builtin entrypoint 白名單 ＝ 這個 registry 的 keys（manifest.ts 預設取這裡）；
//   - CompanionApp／gatewayWiring 只呼叫 createBuiltinAdapter(entrypoint, ctx)，
//     不再有「entrypoint 等於某個具名角色 id」這類分岔；
//   - 角色專屬的畫布 class、遊玩場、variant 別名鍵由 adapter meta 宣告。
//
// 這個模組**不** import 任何 adapter 實作（避免循環）：id 在這裡宣告，工廠由
// `character/adapters/index.ts` 註冊。宣告了卻沒註冊工廠 → createBuiltinAdapter 誠實失敗，
// 不會假裝角色載入成功。

import type { MixerPort } from "../companion/machine";
import type { RendererBackend } from "../companion/renderer";
import type { DirectorTables } from "../companion/director";
import type { LandingTable } from "../companion/gameFeel";
import type { LegacyEventArt } from "../companion/machine";
import type { VariantWeightTable } from "../companion/personality";
import type { StageRenderer } from "../companion/rig/stage";
import type { CharacterAdapter } from "./adapter";
import { coreMigrationRegistry, MigrationRegistry, type PackMigrator } from "./manifest";
import type { CharacterManifest, LocalizedText } from "./protocol";

/** host 宣告的 in-process builtin adapter id（＝ builtin entrypoint 白名單）。 */
export const BUILTIN_ADAPTER_IDS = ["shu-rig", "sprite", "text", "shape"] as const;
export type BuiltinEntrypointId = (typeof BUILTIN_ADAPTER_IDS)[number];

/**
 * 可信退路 adapter：其他角色停用／崩潰／載不到時一律退到它。
 * CPP §12 指定純文字角色擔任這個角色（安全訊息永遠看得到）。
 */
export const FALLBACK_ADAPTER_ID: BuiltinEntrypointId = "text";

/** registry 有界：host 不可能有無限多個 in-process adapter。 */
export const MAX_BUILTIN_ADAPTERS = 32;

/** 角色畫面掛在哪一種宿主元素上。 */
export type AdapterSurface = "canvas" | "dom";

/** 角色畫布的 CSS class（host 依 adapter 宣告套用，不看 pack id 或 entrypoint 字串）。 */
export type AdapterCssClass = "companion-stage" | "companion-canvas" | "companion-text";

/**
 * adapter 的 host 側中繼資料。角色專屬的呈現細節（舞台 class、遊玩場、variant 別名）
 * 在這裡宣告，host 只讀 meta。
 */
export interface BuiltinAdapterMeta {
  /** 畫布 CSS class。 */
  readonly cssClass: AdapterCssClass;
  /** 需要 canvas 還是純 DOM 宿主。 */
  readonly surface: AdapterSurface;
  /** 是否有遊玩場（玩具／使魔／roll call／角色表）。 */
  readonly hasPlayfield: boolean;
  /**
   * 選定 variant 時要一起送出的別名鍵（例如某個 rig 用 `palette` 收同一個值）。
   * 有界：最多 4 個鍵。
   */
  readonly variantAliasKeys?: readonly string[];
  /**
   * adapter 認得的 variant id。有宣告時，只有清單內的 variant 才會走 `variantAliasKeys`
   * （未知 variant 只原樣透傳，不猜）。有界：最多 32 個。
   */
  readonly variants?: readonly string[];
  /** 這個 adapter 需要舊 pack 版型（`x-legacy` character-pack）才能建出來。 */
  readonly requiresLegacyPackShape?: boolean;
  /** 這個 adapter 能接手的舊 pack `kind`（host 用它把舊 pack 導到對的 adapter）。 */
  readonly legacyPackKinds?: readonly string[];
}

/** 建 adapter 需要的 host 環境。欄位全部選填：每個 adapter 只讀自己要的。 */
export interface BuiltinAdapterContext {
  /** 索引／匯入清單裡的角色 id（沒有 manifest 本文時 adapter 用它組 manifest）。 */
  readonly characterId?: string | null;
  /** 已驗證的 CPP manifest（有就優先用）。 */
  readonly manifest?: CharacterManifest | null;
  /** 舊 pack JSON（character-pack／character-rig）。 */
  readonly legacyPack?: unknown;
  readonly displayName?: LocalizedText | null;
  readonly description?: LocalizedText | null;
  /** canvas 型 adapter 的畫布。 */
  readonly canvas?: HTMLCanvasElement | null;
  /** DOM 型 adapter 的宿主元素。 */
  readonly textHost?: HTMLElement | null;
  readonly scale?: number;
  readonly mixer?: MixerPort;
  readonly charName?: string | null;
  /** 初始 variant（rig 的配色、sprite 的版型…）。 */
  readonly variant?: string | null;
  /** sprite：同源資產根與 sheet URL（host 已解成同源路徑或 data URL）。 */
  readonly assetBase?: string | null;
  readonly sheetUrl?: string | null;
}

/**
 * host 需要從「有遊玩場的角色」拿到的東西。任何 adapter 都可以提供（或不提供）；
 * host 只看有沒有，不看是哪個角色。
 */
export interface AdapterToyEntry {
  /** adapter 自己的玩具 kind（host 只轉述，不認得任何特定角色的清單）。 */
  readonly kind: string;
  readonly label: string;
  readonly emoji: string;
}

export interface CompanionSurface {
  readonly directorTables: DirectorTables;
  readonly landingTable: LandingTable;
  readonly variantWeights: VariantWeightTable;
  readonly eventArt: LegacyEventArt;
  readonly toyCatalog: readonly AdapterToyEntry[];
  stageRenderer(): StageRenderer | null;
  rollCallNow(machineLabel: string | null): { name: string; activity: string }[];
}

/** 建好的 adapter ＋ host 要接的東西。 */
export interface BuiltinAdapterBuild {
  readonly adapter: CharacterAdapter;
  /** host 擁有並負責 destroy 的 renderer（沒有就是 null）。 */
  readonly renderer: RendererBackend | null;
  /** 有遊玩場的角色才有；host 用它拿角色表與舞台。 */
  readonly companion: CompanionSurface | null;
  /** 這個 adapter 的 meta（host 直接用，免得再查一次）。 */
  readonly meta: BuiltinAdapterMeta;
}

export type BuiltinAdapterFactory = (
  ctx: BuiltinAdapterContext
) => BuiltinAdapterBuild | Promise<BuiltinAdapterBuild>;

interface Registration {
  readonly factory: BuiltinAdapterFactory;
  readonly meta: BuiltinAdapterMeta;
}

const registry = new Map<string, Registration>();

const MAX_VARIANT_ALIAS_KEYS = 4;
const MAX_META_VARIANTS = 32;

/** id 是不是 host 宣告過的 builtin adapter（白名單檢查唯一入口）。 */
export function isBuiltinEntrypointId(id: unknown): id is BuiltinEntrypointId {
  return typeof id === "string" && (BUILTIN_ADAPTER_IDS as readonly string[]).includes(id);
}

/**
 * 註冊一個 builtin adapter 工廠。
 * - id 必須是宣告過的（未宣告的 id 不得偷偷變成白名單成員）；
 * - 同一個 id 不得註冊兩次（後者不得悄悄覆蓋前者）；
 * - 總數有界。
 */
export function registerBuiltinAdapter(id: string, factory: BuiltinAdapterFactory, meta: BuiltinAdapterMeta): void {
  if (!isBuiltinEntrypointId(id)) {
    throw new Error("builtin adapter id is not declared in the host registry");
  }
  if (registry.has(id)) {
    throw new Error(`builtin adapter '${id}' is already registered`);
  }
  if (registry.size >= MAX_BUILTIN_ADAPTERS) {
    throw new Error(`builtin adapter registry is full (max ${MAX_BUILTIN_ADAPTERS})`);
  }
  if ((meta.variantAliasKeys?.length ?? 0) > MAX_VARIANT_ALIAS_KEYS) {
    throw new Error(`builtin adapter '${id}' declares too many variant alias keys`);
  }
  if ((meta.variants?.length ?? 0) > MAX_META_VARIANTS) {
    throw new Error(`builtin adapter '${id}' declares too many variants`);
  }
  registry.set(id, { factory, meta });
}

/**
 * host 宣告的 builtin entrypoint id（＝白名單）。**不**取決於工廠有沒有載入：
 * manifest 驗證在任何模組載入順序下都必須給同一個答案。
 */
export function builtinEntrypointIds(): readonly string[] {
  return BUILTIN_ADAPTER_IDS;
}

/** 真的有工廠可以建出來的 id（依宣告順序）。宣告了卻沒註冊＝載入時誠實失敗。 */
export function registeredBuiltinAdapterIds(): readonly string[] {
  return BUILTIN_ADAPTER_IDS.filter((id) => registry.has(id));
}

/** 舊 pack 的 `kind` → 接手的 builtin adapter id；沒有人接就是 null（不猜）。 */
export function entrypointForLegacyPackKind(kind: unknown): BuiltinEntrypointId | null {
  if (typeof kind !== "string" || kind.length === 0) return null;
  for (const [id, entry] of registry) {
    if (entry.meta.legacyPackKinds?.includes(kind) && isBuiltinEntrypointId(id)) return id;
  }
  return null;
}

/** adapter 的 host meta；沒註冊就是 null（不猜預設值）。 */
export function builtinAdapterMeta(id: unknown): BuiltinAdapterMeta | null {
  return typeof id === "string" ? (registry.get(id)?.meta ?? null) : null;
}

/** 依 entrypoint id 建 adapter。未註冊 → 擲例外（host 會退回文字角色並顯示原因）。 */
export async function createBuiltinAdapter(id: string, ctx: BuiltinAdapterContext): Promise<BuiltinAdapterBuild> {
  const entry = registry.get(id);
  if (!entry) throw new Error(`no builtin adapter is registered for this entrypoint`);
  return await entry.factory(ctx);
}

/** 測試用：清空註冊（正式路徑永遠只在模組載入時註冊一次）。 */
export function resetBuiltinAdaptersForTest(): void {
  registry.clear();
}

// ---------------------------------------------------------------------------
// §2.2 舊 pack 遷移：host 的 MigrationRegistry
//
// 協定核心只內建通用 sprite（manifest.ts 的 spritePackMigrator）；具名角色的舊格式由
// 它自己的 adapter 模組實作 PackMigrator，在 `character/adapters/index.ts` 跟工廠註冊
// 放在一起。host 只知道「有人接手這個 kind」，不知道那是誰。
// ---------------------------------------------------------------------------

const hostMigrators: PackMigrator[] = [];
let cachedHostRegistry: MigrationRegistry | null = null;

/**
 * 註冊一個 host 側的 pack migrator。上限與重複檢查由 MigrationRegistry 負責
 * （這裡先建一次以便重複／有界違規在註冊當下就擲例外，而不是等到有人遷移才發現）。
 */
export function registerHostMigrator(migrator: PackMigrator): void {
  const next = coreMigrationRegistry();
  for (const m of [...hostMigrators, migrator]) next.register(m);
  hostMigrators.push(migrator);
  cachedHostRegistry = next;
}

/** 桌面 host 的完整 registry：核心的通用 sprite ＋ 各 adapter 註冊的舊格式。 */
export function hostMigrationRegistry(): MigrationRegistry {
  if (!cachedHostRegistry) {
    const next = coreMigrationRegistry();
    for (const m of hostMigrators) next.register(m);
    cachedHostRegistry = next;
  }
  return cachedHostRegistry;
}

/** 測試用：清空 host migrator 註冊（正式路徑只在模組載入時註冊一次）。 */
export function resetHostMigratorsForTest(): void {
  hostMigrators.length = 0;
  cachedHostRegistry = null;
}
