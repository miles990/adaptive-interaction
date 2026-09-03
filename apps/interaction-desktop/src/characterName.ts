// 角色名稱的單一真相（一般模式所有「小樞」字樣的來源）。
//
// 優先序：使用者取的名字（desktop prefs.companionName）＞目前角色 manifest 的
// displayName（依 locale）＞中立的「角色」。代詞走 manifest.pronouns；沒宣告就用
// 中立文案（pronounOf）。角色是誰：Runtime `status.characterProtocol.activeCharacter`
// 有值就讀 `api.characterManifest()`（桌面角色視窗 hello 過的那一份）；否則用
// bundled 索引（prefs.companionPack 命中的項目，再不然索引 default）。任何一段失敗
// 都不猜——沒有 manifest 就是 `loaded:false`＋「角色」。
//
// 一個 module-level store＋useSyncExternalStore：導覽、標題、全域搜尋、「現在」頁
// 同時掛載也只打一輪 API；`refreshCharacterName()` 讓 Shell 在角色事件／換頁時更新。

import React from "react";
import { api } from "./api";
import { desktop, isTauri } from "./desktop";
import { displayNameOf, pronounOf } from "./character/manifest";
import type { CharacterManifest, LocalizedText } from "./character/protocol";
import { loadCharacterIndex, type CharacterIndex } from "./character/registry";
import { hasIcon } from "./icons";

/** 角色載入失敗／尚未載入時的中立名稱。 */
export const characterNameFallback = "角色";

/** manifest 沒有 icon 提示時，導覽用的中立 icon。 */
export const NEUTRAL_CHARACTER_ICON = "sparkles";

export const DEFAULT_CHARACTER_LOCALE = "zh-TW";

export interface CharacterNameState {
  /** 顯示名：prefs.companionName ＞ manifest displayName ＞「角色」。 */
  name: string;
  /** 代詞：manifest.pronouns（依 locale）＞中立文案。 */
  pronoun: string;
  characterId: string | null;
  /** 有沒有拿到 manifest（或 Runtime 回報的 activeCharacter）。false ＝ 用中立名稱。 */
  loaded: boolean;
  /** 導覽 icon：manifest 頂層 `icon` 提示命中目錄才用，否則中立 icon。 */
  icon: string;
}

/** 解析名稱只需要的 prefs 子集（桌面 prefs 或任何相容物件）。 */
export type CharacterNamePrefs =
  | { companionName?: string | null; companionPack?: string | null }
  | null
  | undefined;

/** 解析名稱只需要的 manifest 子集（Runtime 的 activeCharacter 也符合）。 */
export type CharacterManifestLike = Pick<CharacterManifest, "characterId" | "displayName"> &
  Partial<Pick<CharacterManifest, "pronouns">> & { icon?: unknown };

const MAX_NAME_CHARS = 24;

/** 中立代詞（與 pronounOf 的 fallback 一致）。 */
export function neutralPronoun(locale: string): string {
  return locale.toLowerCase().startsWith("zh") ? characterNameFallback : "they";
}

/**
 * 純函式：名稱／代詞／id／loaded／icon 的解析規則（給測試與非 React 程式碼）。
 * prefs 名字先 trim、限 24 字（與 companion 視窗 charNameFor 相同）。
 */
export function resolveCharacterName(
  prefs: CharacterNamePrefs,
  manifest: CharacterManifestLike | null | undefined,
  locale: string = DEFAULT_CHARACTER_LOCALE
): CharacterNameState {
  const own = typeof prefs?.companionName === "string" ? prefs.companionName.trim().slice(0, MAX_NAME_CHARS) : "";
  const fromManifest = manifest ? displayNameOf(manifest, locale) : characterNameFallback;
  const name = own.length > 0 ? own : fromManifest;
  const pronoun = manifest ? pronounOf(manifest, locale) : neutralPronoun(locale);
  const hint = manifest && typeof manifest.icon === "string" ? manifest.icon : "";
  const icon = hint && hasIcon(hint) ? hint : NEUTRAL_CHARACTER_ICON;
  return {
    name,
    pronoun,
    characterId: manifest?.characterId ?? null,
    loaded: Boolean(manifest),
    icon,
  };
}

// ---------------------------------------------------------------------------
// store
// ---------------------------------------------------------------------------

const INITIAL: CharacterNameState = resolveCharacterName(null, null, DEFAULT_CHARACTER_LOCALE);

let state: CharacterNameState = INITIAL;
const listeners = new Set<() => void>();
let inflight: Promise<CharacterNameState> | null = null;
let indexPromise: Promise<CharacterIndex | null> | null = null;
let lastRefreshAt = 0;
/** 測試釘住的角色：非 force 的刷新不會蓋掉它（reset 解除）。 */
let pinned = false;
/**
 * 世代編號：`resetCharacterNameForTests()`／`primeCharacterNameForTests()` 會換一代。
 * 它們把 `inflight` 清成 null，所以上一代還在飛的刷新不會被新的呼叫等到——那一輪回來時
 * 若世代已經換過，就只是遲到的舊答案，不得覆寫 state 或 `lastRefreshAt`（否則會把新
 * 世代已經解析好的角色名蓋回中立值）。正式執行期沒有人換世代，行為完全不變。
 */
let generation = 0;

/** 換頁觸發的刷新最短間隔（角色事件用 force 略過）。 */
export const CHARACTER_NAME_MIN_REFRESH_MS = 1500;

function setState(next: CharacterNameState) {
  const changed =
    next.name !== state.name ||
    next.pronoun !== state.pronoun ||
    next.characterId !== state.characterId ||
    next.loaded !== state.loaded ||
    next.icon !== state.icon;
  if (!changed) return;
  state = next;
  listeners.forEach((l) => l());
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getSnapshot(): CharacterNameState {
  return state;
}

/** 目前的解析結果（非 React 程式碼用；可能是尚未載入的中立值）。 */
export function currentCharacterName(): CharacterNameState {
  return state;
}

/** 測試用：清掉快取與 inflight，讓每個測試從中立狀態開始（前一輪還沒回來的刷新就此作廢）。 */
export function resetCharacterNameForTests(): void {
  generation += 1;
  state = INITIAL;
  inflight = null;
  indexPromise = null;
  lastRefreshAt = 0;
  pinned = false;
}

/**
 * 測試用：直接釘住角色（不必 mock status／characterManifest／fetch）。
 * 掛載時的一般刷新不會蓋掉它；`refreshCharacterName({ force: true })` 仍會重新解析。
 * 例：primeCharacterNameForTests({ name: "小樞", pronoun: "她", characterId: "shu-maid" })。
 */
export function primeCharacterNameForTests(partial: Partial<CharacterNameState>): CharacterNameState {
  const next: CharacterNameState = {
    ...INITIAL,
    loaded: true,
    ...partial,
  };
  generation += 1;
  pinned = true;
  inflight = null;
  setState(next);
  return state;
}

async function readPrefs(): Promise<CharacterNamePrefs> {
  if (!isTauri) return null;
  try {
    return await desktop.prefsGet();
  } catch {
    return null;
  }
}

interface ActiveCharacter {
  characterId: string;
  displayName: LocalizedText;
}

function parseActiveCharacter(status: Record<string, unknown> | null): ActiveCharacter | null {
  const cp = status?.["characterProtocol"];
  if (!cp || typeof cp !== "object") return null;
  const active = (cp as Record<string, unknown>)["activeCharacter"];
  if (!active || typeof active !== "object") return null;
  const a = active as Record<string, unknown>;
  const characterId = typeof a.characterId === "string" ? a.characterId : null;
  const displayName = a.displayName;
  if (!characterId || !displayName || typeof displayName !== "object") return null;
  return { characterId, displayName: displayName as LocalizedText };
}

/** Runtime 認得的目前角色：activeCharacter 有值 → 讀完整 manifest；讀不到就退回
 *  activeCharacter 本身（名字是 Runtime 回報的，仍是事實；代詞則中立）。 */
async function readLiveManifest(): Promise<CharacterManifestLike | null> {
  let status: Record<string, unknown> | null = null;
  try {
    status = await api.status();
  } catch {
    status = null;
  }
  const active = parseActiveCharacter(status);
  if (!active) return null;
  try {
    const manifest = await api.characterManifest();
    if (manifest && typeof manifest === "object" && typeof manifest.characterId === "string") return manifest;
  } catch {
    /* 尚未 hello／舊 daemon：退回 activeCharacter */
  }
  return active;
}

/** bundled 索引（只在成功時快取；失敗下次再試）。 */
function loadBundledIndex(): Promise<CharacterIndex | null> {
  if (!indexPromise) {
    indexPromise = (async () => {
      if (typeof fetch !== "function") return null;
      try {
        const r = await loadCharacterIndex((url) => fetch(url));
        return r.ok ? r.index : null;
      } catch {
        return null;
      }
    })().then((index) => {
      if (!index) indexPromise = null;
      return index;
    });
  }
  return indexPromise;
}

/** bundled 索引裡選角色：prefs.companionPack 命中就用它，否則索引 default（純函式，給測試）。 */
export function pickBundledManifest(index: CharacterIndex, preferred: string | null | undefined): CharacterManifestLike | null {
  const want = typeof preferred === "string" && preferred.length > 0 ? preferred : null;
  const hit = want ? index.characters.find((c) => c.characterId === want) : undefined;
  const entry = hit ?? index.characters.find((c) => c.characterId === index.default);
  return entry?.manifest ?? null;
}

/**
 * 重新解析角色名稱。同時多處呼叫共用同一輪 I/O；非 force 呼叫在
 * CHARACTER_NAME_MIN_REFRESH_MS 內直接回目前狀態（換頁不會打爆 API）。
 */
export function refreshCharacterName(
  opts: { locale?: string; force?: boolean } = {}
): Promise<CharacterNameState> {
  if (inflight) return inflight;
  const now = Date.now();
  if (!opts.force && (pinned || (lastRefreshAt > 0 && now - lastRefreshAt < CHARACTER_NAME_MIN_REFRESH_MS))) {
    return Promise.resolve(state);
  }
  if (opts.force) pinned = false;
  const locale = opts.locale ?? DEFAULT_CHARACTER_LOCALE;
  const startedGeneration = generation;
  /** 這一輪還算不算數：reset／prime 換過世代就是遲到的舊答案，不得落地。 */
  const current = () => generation === startedGeneration;
  inflight = (async () => {
    try {
      const [prefs, live] = await Promise.all([readPrefs(), readLiveManifest()]);
      let manifest: CharacterManifestLike | null = live;
      if (!manifest) {
        const index = await loadBundledIndex();
        if (index) manifest = pickBundledManifest(index, prefs?.companionPack);
      }
      if (current()) setState(resolveCharacterName(prefs, manifest, locale));
    } catch {
      if (current()) setState(resolveCharacterName(null, null, locale));
    } finally {
      // 世代換過時 `inflight` 已經是 null（或別人那一輪），不能亂清。
      if (current()) {
        lastRefreshAt = Date.now();
        inflight = null;
      }
    }
    return state;
  })();
  return inflight;
}

/**
 * 角色名稱 hook：{ name, pronoun, characterId, loaded, icon }。
 * 掛載時刷新一次（多個元件共用同一輪）；`refreshKey` 改變時再刷新（受最短間隔保護）。
 * 不依賴 AppState context，所以在 provider 外也能用。
 */
export function useCharacterName(
  opts: { locale?: string; refreshKey?: unknown } = {}
): CharacterNameState {
  const snapshot = React.useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  const locale = opts.locale;
  const refreshKey = opts.refreshKey;
  React.useEffect(() => {
    void refreshCharacterName({ locale });
  }, [locale, refreshKey]);
  return snapshot;
}
