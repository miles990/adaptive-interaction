// 角色目錄：內建索引（/characters/index.json）＋已匯入角色（host 本機資料夾）合併成
// 角色頁用的卡片資料。只讀同源資料與 host 清單；不執行任何 entrypoint、不下載遠端資產。
//
// 每張卡片的文字都是 manifest／host 回報的轉述（registry.capabilitySummary），
// 不含角色專屬敘述；「已測試」只有內建（隨 App 自動化測試）或 Runtime 明確回報時才是「是」。

import React from "react";
import {
  capabilitySummaryParts,
  loadCharacterIndex,
  type CharacterIndex,
  type CharacterIndexEntry,
} from "../../character/registry";
import { displayNameOf, type ManifestReport } from "../../character/manifest";
import type { CharacterManifest, VariantDecl } from "../../character/protocol";
import { desktop, type ImportedCharacterEntry } from "../../desktop";

export const CHARACTER_LOCALE = "zh-TW";

/** 純文字 fallback 角色（停用其他角色時的去處；永遠可用）。 */
export const TEXT_FALLBACK_CHARACTER_ID = "plain-text";

export type CharacterOriginKind = "builtin" | "imported" | "external";
export type EntrypointId = "shu-rig" | "sprite" | "text" | "module" | "process" | "url" | "unknown";

export interface CharacterCard {
  characterId: string;
  name: string;
  origin: CharacterOriginKind;
  entrypoint: EntrypointId;
  /** 完整 manifest（內建索引一定有；匯入角色的清單只有摘要，沒有 manifest）。 */
  manifest: CharacterManifest | null;
  report: ManifestReport | null;
  flags: { external: boolean; network: boolean; executable: boolean };
  /** host 有沒有這個角色的自動化測試證據。 */
  tested: boolean;
  valid: boolean;
  error?: string;
  version?: string;
  assetBase?: string;
  persona?: string;
  story?: string;
  /** 一般模式的人話摘要（來源與版本、可以接收、需要的裝置、已測試）。 */
  summary: string[];
  /** 只在進階模式「技術資料」出現的宣告（執行方式、可執行程式、需要網路、檔案存取、簽章）。 */
  technical: string[];
}

export interface CharacterCatalog {
  cards: CharacterCard[];
  defaultId: string | null;
  /** 索引＋匯入的所有 characterId（角色設定匯入用）。 */
  knownIds: string[];
  /** 人話錯誤（不含路徑）。 */
  errors: string[];
  loaded: boolean;
}

export function entrypointIdOf(
  manifest: Pick<CharacterManifest, "entrypoint"> | null | undefined
): EntrypointId {
  const ep = manifest?.entrypoint;
  if (!ep) return "unknown";
  if (ep.kind === "builtin") {
    return ep.id === "shu-rig" || ep.id === "sprite" || ep.id === "text" ? ep.id : "unknown";
  }
  return ep.kind;
}

function entrypointFromImported(entry: ImportedCharacterEntry): EntrypointId {
  const ep = entry.entrypoint;
  if (ep === "shu-rig" || ep === "sprite" || ep === "text") return ep;
  return entry.external ? "process" : "unknown";
}

/** 錯誤文字：去掉 Error 前綴、隱藏任何像絕對路徑的片段、限長。 */
export function sanitizeErrorText(e: unknown): string {
  const raw = e instanceof Error ? e.message : String(e ?? "");
  return raw
    .replace(/^Error:\s*/, "")
    .replace(/[A-Za-z]:\\[^\s"']+/g, "（路徑已隱藏）")
    .replace(/(?:\/[^\s/"']+){2,}\/?/g, "（路徑已隱藏）")
    .slice(0, 300);
}

export function cardFromIndexEntry(entry: CharacterIndexEntry): CharacterCard {
  const manifest = entry.manifest;
  const external = entry.report.flags.external || manifest.adapterKind !== "in-process";
  const origin: CharacterOriginKind = external ? "external" : entry.origin;
  const tested = entry.origin === "builtin" && !external;
  const parts = capabilitySummaryParts(manifest, CHARACTER_LOCALE, { origin: entry.origin, tested });
  return {
    characterId: entry.characterId,
    name: displayNameOf(manifest, CHARACTER_LOCALE),
    origin,
    entrypoint: entrypointIdOf(manifest),
    manifest,
    report: entry.report,
    flags: {
      external,
      network: entry.report.flags.network,
      executable: entry.report.flags.executable,
    },
    tested,
    valid: true,
    version: manifest.version,
    ...(entry.assetBase ? { assetBase: entry.assetBase } : {}),
    ...(entry.persona ? { persona: entry.persona } : {}),
    ...(entry.story ? { story: entry.story } : {}),
    summary: parts.general,
    technical: parts.technical,
  };
}

/** 匯入角色清單沒有完整 manifest：用 host 回報的旗標組出同措辭的摘要。 */
export function cardFromImported(entry: ImportedCharacterEntry): CharacterCard {
  const name = displayNameOf({ displayName: entry.displayName ?? {} }, CHARACTER_LOCALE);
  const external = entry.external === true || (entry.adapterKind !== undefined && entry.adapterKind !== "in-process");
  const summary = [
    `${name}：第三方角色（${entry.version ? `版本 ${entry.version}` : "版本未標示"}）`,
    "已測試：否（未經本機測試；請先在受控環境試用）",
  ];
  const technical = [
    external ? "外部程式（永不自動啟動，需明確安裝與授權）" : "在本機視窗內執行（內建 adapter）",
    entry.executable ? "有可執行程式：是（只記錄，不會自動執行）" : "有可執行程式：否（純資料）",
    entry.network ? "需要網路：是" : "需要網路：否",
    "簽章：無（本版不支援簽章驗證）",
  ];
  return {
    characterId: entry.characterId,
    name,
    origin: external ? "external" : "imported",
    entrypoint: entrypointFromImported(entry),
    manifest: null,
    report: entry.report ?? null,
    flags: { external, network: entry.network === true, executable: entry.executable === true },
    tested: false,
    valid: entry.valid !== false,
    ...(entry.error ? { error: sanitizeErrorText(entry.error) } : {}),
    ...(entry.version ? { version: entry.version } : {}),
    summary,
    technical,
  };
}

/** 合併：索引在前（含 default），匯入在後；同 id 以 host 匯入清單為準（host 不允許覆蓋內建 id）。 */
export function mergeCatalog(index: CharacterIndex | null, imported: ImportedCharacterEntry[]): CharacterCard[] {
  const cards: CharacterCard[] = [];
  const seen = new Set<string>();
  for (const entry of index?.characters ?? []) {
    if (seen.has(entry.characterId)) continue;
    seen.add(entry.characterId);
    cards.push(cardFromIndexEntry(entry));
  }
  for (const entry of imported) {
    if (!entry || typeof entry.characterId !== "string") continue;
    const card = cardFromImported(entry);
    const at = cards.findIndex((c) => c.characterId === card.characterId);
    if (at >= 0) cards[at] = card;
    else cards.push(card);
    seen.add(card.characterId);
  }
  return cards;
}

export function originLabel(origin: CharacterOriginKind): string {
  switch (origin) {
    case "builtin":
      return "內建";
    case "imported":
      return "匯入";
    default:
      return "外部";
  }
}

/** 內建／第三方（匯入與外部都是第三方）。 */
export function partyLabel(origin: CharacterOriginKind): string {
  return origin === "builtin" ? "內建" : "第三方";
}

/** 本機／外部（執行位置）。 */
export function locationLabel(card: Pick<CharacterCard, "flags">): string {
  return card.flags.external ? "外部" : "本機";
}

/**
 * 一般模式唯一的「額外授權」提示：需要跑自己的程式或需要網路的角色不能被藏起來
 * （誠實不可協商），但一般模式只給一句人話；細節在進階模式的技術資料。
 * 不需要額外授權時回 null。
 */
export function extraPermissionLine(card: Pick<CharacterCard, "flags">): string | null {
  const needs: string[] = [];
  if (card.flags.executable) needs.push("在你的電腦上執行它自己的程式");
  if (card.flags.network) needs.push("連上網路");
  if (needs.length === 0) return null;
  return `這個角色需要額外授權：${needs.join("、")}。控制中心不會自動執行它的程式、也不會自動連線。`;
}

/** 「可以接收：…」那一行（沒有 manifest 時誠實說不明）。 */
export function receivesLine(card: Pick<CharacterCard, "summary" | "manifest">): string {
  const hit = card.summary.find((line) => line.startsWith("可以接收："));
  if (hit) return hit;
  return card.manifest ? "可以接收：不接收任何輸入" : "可以接收：不明（角色資料尚未載入）";
}

export function isShuRig(card: Pick<CharacterCard, "entrypoint"> | null | undefined): boolean {
  return card?.entrypoint === "shu-rig";
}

export function variantName(variant: VariantDecl): string {
  return variant.displayName ? displayNameOf({ displayName: variant.displayName }, CHARACTER_LOCALE) : variant.id;
}

/**
 * 外觀切換：同名、同 entrypoint、且「第一個 variant」就是目標的角色（小樞三種配色是
 * 三個 characterId）→ 切換 companionPack；找不到就回 null（改存角色偏好）。
 */
export function siblingForVariant(cards: CharacterCard[], card: CharacterCard, variantId: string): string | null {
  if (card.manifest?.variants[0]?.id === variantId) return card.characterId;
  const hit = cards.find(
    (c) =>
      c.characterId !== card.characterId &&
      c.valid &&
      c.manifest !== null &&
      c.entrypoint === card.entrypoint &&
      c.name === card.name &&
      c.manifest.variants[0]?.id === variantId
  );
  return hit?.characterId ?? null;
}

export function useCharacterCatalog(refreshKey: number): CharacterCatalog & { reload: () => void } {
  const [state, setState] = React.useState<CharacterCatalog>({
    cards: [],
    defaultId: null,
    knownIds: [],
    errors: [],
    loaded: false,
  });
  const [tick, setTick] = React.useState(0);
  React.useEffect(() => {
    let alive = true;
    (async () => {
      const errors: string[] = [];
      let index: CharacterIndex | null = null;
      try {
        if (typeof fetch !== "function") throw new Error("fetch unavailable");
        const r = await loadCharacterIndex((url) => fetch(url));
        if (r.ok) {
          index = r.index;
          for (const e of r.index.errors) errors.push(`內建角色有問題：${sanitizeErrorText(e)}`);
        } else {
          errors.push(`內建角色索引無法載入：${sanitizeErrorText(r.error)}`);
        }
      } catch (e) {
        errors.push(`內建角色索引無法載入：${sanitizeErrorText(e)}`);
      }
      let imported: ImportedCharacterEntry[] = [];
      try {
        const list = await desktop.characterListImported();
        imported = Array.isArray(list) ? list : [];
      } catch (e) {
        errors.push(`已匯入角色清單無法讀取：${sanitizeErrorText(e)}`);
      }
      if (!alive) return;
      const cards = mergeCatalog(index, imported);
      setState({
        cards,
        defaultId: index?.default ?? null,
        knownIds: cards.map((c) => c.characterId),
        errors,
        loaded: true,
      });
    })();
    return () => {
      alive = false;
    };
  }, [refreshKey, tick]);
  const reload = React.useCallback(() => setTick((t) => t + 1), []);
  return { ...state, reload };
}
