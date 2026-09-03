// 角色偏好（manifest.preferencesSchema 宣告的 bounded 值）：型別限縮、clamp、持久化。
//
// 值永遠先經過 schema 限縮（boolean／number／integer 含 min-max／string enum／string maxLength），
// 不存 schema 沒宣告的鍵。持久化走 desktop.prefsPatch({ companionPreferences })；host 版本
// 還沒保存這個欄位時 patch 會被丟掉——我們用回傳值偵測，退回 localStorage 並誠實告知
//（不宣稱已保存）。這些都是呈現偏好，沒有任何權限語意。

import type { PreferencePropertySchema, PreferencesSchema } from "../../character/protocol";
import { desktop, type DesktopPrefs } from "../../desktop";

export type PreferenceValue = boolean | number | string;
export type PreferenceValues = Record<string, PreferenceValue>;

export const CHARACTER_PREFS_STORAGE_KEY = "adaptive-interaction.characterPreferences";
export const MAX_PREFERENCE_PROPERTIES = 32;
const DEFAULT_MAX_LENGTH = 200;
const NUMBER_LIMIT = 1_000_000;

function clamp(n: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, n));
}

function stripControl(s: string): string {
  return Array.from(s)
    .filter((ch) => {
      const code = ch.codePointAt(0) ?? 0;
      return code >= 0x20 && code !== 0x7f;
    })
    .join("");
}

/** 一個屬性的預設值（schema.default 也要先過限縮）。 */
export function propertyDefault(prop: PreferencePropertySchema): PreferenceValue {
  switch (prop.type) {
    case "boolean":
      return prop.default === true;
    case "number":
    case "integer":
      return boundValue(prop, typeof prop.default === "number" ? prop.default : prop.minimum ?? 0);
    default:
      return boundValue(prop, typeof prop.default === "string" ? prop.default : "");
  }
}

/** 把任意輸入限縮成 schema 允許的值；不合法就退回預設。 */
export function boundValue(prop: PreferencePropertySchema, raw: unknown): PreferenceValue {
  switch (prop.type) {
    case "boolean":
      return raw === true;
    case "number":
    case "integer": {
      const n = typeof raw === "number" ? raw : Number(raw);
      const min = typeof prop.minimum === "number" ? prop.minimum : -NUMBER_LIMIT;
      const max = typeof prop.maximum === "number" ? prop.maximum : NUMBER_LIMIT;
      const base = Number.isFinite(n) ? n : typeof prop.default === "number" ? prop.default : min > 0 ? min : 0;
      const bounded = clamp(base, Math.min(min, max), Math.max(min, max));
      return prop.type === "integer" ? Math.round(bounded) : bounded;
    }
    default: {
      const text = typeof raw === "string" ? raw : typeof prop.default === "string" ? prop.default : "";
      if (Array.isArray(prop.enum) && prop.enum.length > 0) {
        return prop.enum.includes(text) ? text : prop.enum[0];
      }
      const maxLength = typeof prop.maxLength === "number" ? Math.min(prop.maxLength, DEFAULT_MAX_LENGTH) : DEFAULT_MAX_LENGTH;
      return stripControl(text).slice(0, maxLength);
    }
  }
}

/** 外觀（manifest.variants）選擇的保留鍵：只接受 manifest 宣告過的 variant id。 */
export const VARIANT_PREFERENCE_KEY = "variant";

export interface BoundOptions {
  /** 允許保留鍵 `variant` 的合法值（manifest.variants[].id）。 */
  variantIds?: readonly string[];
}

/** schema 宣告的每個屬性都有值（有輸入就限縮，沒有就預設）；未宣告的鍵丟棄。 */
export function boundValues(schema: PreferencesSchema | undefined, raw: unknown, opts: BoundOptions = {}): PreferenceValues {
  const out: PreferenceValues = {};
  const props = Object.entries(schema?.properties ?? {}).slice(0, MAX_PREFERENCE_PROPERTIES);
  const source = raw && typeof raw === "object" && !Array.isArray(raw) ? (raw as Record<string, unknown>) : {};
  for (const [key, prop] of props) {
    out[key] = Object.prototype.hasOwnProperty.call(source, key) ? boundValue(prop, source[key]) : propertyDefault(prop);
  }
  const variant = source[VARIANT_PREFERENCE_KEY];
  if (opts.variantIds && typeof variant === "string" && opts.variantIds.includes(variant)) {
    out[VARIANT_PREFERENCE_KEY] = variant;
  }
  return out;
}

export function readLocalPreferences(): Record<string, PreferenceValues> {
  try {
    const raw = globalThis.localStorage?.getItem(CHARACTER_PREFS_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as unknown;
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? (parsed as Record<string, PreferenceValues>)
      : {};
  } catch {
    return {};
  }
}

export function writeLocalPreferences(all: Record<string, PreferenceValues>): void {
  try {
    globalThis.localStorage?.setItem(CHARACTER_PREFS_STORAGE_KEY, JSON.stringify(all));
  } catch {
    /* 私密模式／配額：不保存也不假裝保存 */
  }
}

export type PreferenceSource = "host" | "local" | "default";

/** 目前生效的值：host prefs ＞ 本視窗暫存 ＞ schema 預設。 */
export function effectivePreferences(
  schema: PreferencesSchema | undefined,
  characterId: string,
  prefs: Pick<DesktopPrefs, "companionPreferences"> | null,
  opts: BoundOptions = {}
): { values: PreferenceValues; source: PreferenceSource } {
  const fromHost = prefs?.companionPreferences?.[characterId];
  if (fromHost && typeof fromHost === "object") return { values: boundValues(schema, fromHost, opts), source: "host" };
  const fromLocal = readLocalPreferences()[characterId];
  if (fromLocal && typeof fromLocal === "object") return { values: boundValues(schema, fromLocal, opts), source: "local" };
  return { values: boundValues(schema, {}, opts), source: "default" };
}

/**
 * 保存：先送 host；回傳的 prefs 沒帶回 companionPreferences[characterId] 就表示這個版本的
 * host 沒有保存它——退回本視窗 localStorage，並回報 "local" 讓 UI 誠實顯示。
 */
export async function persistCharacterPreferences(
  characterId: string,
  values: PreferenceValues,
  prefs: DesktopPrefs
): Promise<{ prefs: DesktopPrefs; persisted: "host" | "local" }> {
  const next = { ...(prefs.companionPreferences ?? {}), [characterId]: values };
  const updated = await desktop.prefsPatch({ companionPreferences: next });
  const echoed = updated?.companionPreferences?.[characterId];
  if (echoed && typeof echoed === "object") {
    await desktop.companionApplyPrefs();
    return { prefs: updated, persisted: "host" };
  }
  const all = readLocalPreferences();
  all[characterId] = values;
  writeLocalPreferences(all);
  return { prefs: updated ?? prefs, persisted: "local" };
}
