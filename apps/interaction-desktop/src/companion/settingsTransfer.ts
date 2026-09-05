// 角色設定匯出／匯入（spec §5.1）：只搬「呈現偏好」，不含任何權限、
// token、位置或歷史。匯入逐欄白名單驗證——未知欄位丟棄、非法值拒絕。
//
// CPP：`companionPack` 就是 characterId。匯入時接受「符合 CHARACTER_ID_RE 且
// host 認得（索引裡有，或是 8 個舊 pack id）」的任何角色，不再是閉合清單；
// 匯出同時寫 `characterId`（別名，語意等同 companionPack）。schemaVersion 維持 1。
//
// M2 §3.4 驗證邊界綁定角色（v0.6.0 已知限制 #17）：說話風格與使魔配色以前是全域白名單
// （一份硬編的 persona 清單＋單一 rig 的配色表），同一份 JSON 說自己屬於哪個角色完全
// 不影響驗證——匯入一個純文字角色的設定檔照樣可以夾帶 rig 的配色，存成一個沒有人吃的
// 死值。現在先解出 characterId → 由呼叫端（角色目錄）告訴我們它的 entrypoint →
// 用**那個 adapter** 宣告的 `personas`／`variants`／`hasPlayfield`／`scenes` 驗證；問不出 adapter
// 就誠實拒絕角色專屬欄位，不拿別的角色的允許值頂替，也不靜默丟棄。

import { DesktopPrefs } from "../desktop";
// 角色專屬欄位（說話風格、使魔配色）是**那個角色**的 adapter 的知識：host 只讀 meta。
import { builtinAdapterMeta, type BuiltinAdapterMeta } from "../character/adapterRegistry";
import { CHARACTER_ID_RE } from "../character/protocol";

export interface CompanionSettingsExport {
  kind: "companion-settings";
  schemaVersion: 1;
  companionName: string;
  companionPack: string;
  /** CPP 別名：與 companionPack 相同（characterId）。 */
  characterId: string;
  companionPersona: string;
  companionExpressiveness: string;
  companionScene: string;
  companionPlay: boolean;
  companionCursorPlay: boolean;
  companionApproach: boolean;
  companionDeskMove: boolean;
  companionFamiliars: { id: string; name: string; palette: string }[];
}

/** v0.4／v0.5 出貨的 8 個 pack id：CPP §2.2 規定永遠可用（視為 characterId）。 */
export const LEGACY_CHARACTER_IDS: readonly string[] = [
  "shu-maid",
  "shu-maid-dusk",
  "shu-maid-sakura",
  "shu-agile",
  "shu-lazy",
  "shu-lively",
  "shu-standard",
  "shu-minimal",
];
/** 表達強度是**桌面共用**的偏好（任何角色都吃得下），不是角色專屬欄位。 */
const EXPRESSIVENESS = ["quiet", "natural", "lively"];

/** 匯出時同樣需要「這個角色是誰」才知道哪些欄位屬於它。 */
export interface ExportOptions {
  /** characterId → 這台電腦上該角色的 entrypoint id（不知道就回 null）。 */
  entrypointFor?: (characterId: string) => string | null;
}

export function exportCompanionSettings(prefs: DesktopPrefs, opts: ExportOptions = {}): CompanionSettingsExport {
  // 知道目標角色是誰時，只帶它的 adapter 宣告得出來的角色專屬欄位：桌面偏好是共用的，
  // 換角色不會清掉上一個角色的說話風格／使魔，直接整份寫出去會做出一個自己匯不回來的檔案。
  // 沒給對照表（或角色不認得）時維持完整快照，舊呼叫端行為不變。
  const meta = adapterMetaFor(prefs.companionPack, opts.entrypointFor);
  const keepPersona = meta === null || (meta.personas?.length ?? 0) > 0;
  const keepFamiliars = meta === null || meta.hasPlayfield;
  const keepScene = meta === null || (meta.scenes?.length ?? 0) > 0;
  return {
    kind: "companion-settings",
    schemaVersion: 1,
    // 沒取名字就留空：匯入端會用角色 manifest 的 displayName，不硬編任何名字。
    companionName: prefs.companionName ?? "",
    companionPack: prefs.companionPack,
    characterId: prefs.companionPack,
    companionPersona: keepPersona ? prefs.companionPersona : "",
    companionExpressiveness: String(prefs.companionExpressiveness ?? "natural"),
    companionScene: keepScene ? String(prefs.companionScene ?? "none") : "",
    companionPlay: prefs.companionPlay !== false,
    companionCursorPlay: prefs.companionCursorPlay !== false,
    companionApproach: prefs.companionApproach !== false,
    companionDeskMove: prefs.companionDeskMove !== false,
    companionFamiliars: keepFamiliars ? (prefs.companionFamiliars ?? []).slice(0, 3) : [],
  };
}

export interface ImportOptions {
  /** host 目前認得的 characterId（/characters/index.json）；舊 id 永遠接受。 */
  knownCharacterIds?: readonly string[];
  /**
   * characterId → 這台電腦上該角色的 entrypoint id（角色頁的 catalog 提供；不知道回 null）。
   * 角色專屬欄位只用**目標角色的** adapter 宣告驗證：沒有這個對照就沒有角色，
   * 也就沒有「這個欄位屬於誰」的答案，那些欄位一律拒絕。
   */
  entrypointFor?: (characterId: string) => string | null;
}

/** 目標角色的 adapter meta；問不出來（沒對照表／不是 builtin／未註冊）就是 null。 */
function adapterMetaFor(
  characterId: string | null | undefined,
  entrypointFor: ((characterId: string) => string | null) | undefined
): BuiltinAdapterMeta | null {
  if (!entrypointFor || typeof characterId !== "string" || characterId.length === 0) return null;
  return builtinAdapterMeta(entrypointFor(characterId));
}

/** 這個 characterId 是否可匯入：格式合法，且 host 認得或是舊 id。 */
export function isImportableCharacterId(id: string, known: readonly string[] = []): boolean {
  if (typeof id !== "string" || !CHARACTER_ID_RE.test(id)) return false;
  return LEGACY_CHARACTER_IDS.includes(id) || known.includes(id);
}

/** 驗證匯入 JSON → 可直接 prefsPatch 的部分偏好。非法輸入 throw。 */
export function parseCompanionSettingsImport(raw: unknown, opts: ImportOptions = {}): Partial<DesktopPrefs> {
  const obj = raw as Record<string, unknown>;
  if (!obj || typeof obj !== "object") throw new Error("不是有效的設定檔");
  if (obj.kind !== "companion-settings") throw new Error("不是角色設定檔（kind 不符）");
  if (obj.schemaVersion !== 1) throw new Error(`不支援的版本：${String(obj.schemaVersion)}`);
  const out: Partial<DesktopPrefs> = {};
  const str = (key: string, allowed: string[] | null, maxLen: number): string | null => {
    const v = obj[key];
    if (typeof v !== "string") return null;
    if (v.length > maxLen) throw new Error(`${key} 過長`);
    if (allowed && !allowed.includes(v)) throw new Error(`${key} 的值不在允許清單`);
    return v;
  };
  const name = str("companionName", null, 24);
  if (name !== null && name.length > 0) out.companionName = name;
  // companionPack 優先；沒有時接受 CPP 別名 characterId。
  const packKey = typeof obj.companionPack === "string" ? "companionPack" : "characterId";
  const pack = str(packKey, null, 64);
  if (pack !== null) {
    if (!isImportableCharacterId(pack, opts.knownCharacterIds ?? [])) {
      throw new Error(`${packKey} 不是這台電腦認得的角色`);
    }
    out.companionPack = pack;
  }
  // 角色專屬欄位的驗證邊界：先確定目標角色是誰，再問它的 adapter 宣告了什麼。
  const meta = adapterMetaFor(out.companionPack, opts.entrypointFor);
  const target = typeof out.companionPack === "string" ? out.companionPack : null;
  /** 這個欄位問不出主人（沒指定角色／不認得那個角色／那個角色沒有這項設定）。 */
  const unattributable = (field: string): Error =>
    new Error(
      target === null
        ? `這份設定檔沒有指定角色，無法確認「${field}」屬於誰`
        : `這份設定檔裡的「${field}」不屬於「${target}」：這台電腦上的這個角色沒有這項設定`
    );
  // 舊小樞家族（v0.4／v0.5 出貨的 8 個 id）匯出的舊檔會夾帶當時全域共用的說話風格與使魔（那時
  // 這些偏好不分角色）。目標角色的 adapter 沒宣告那一項時，**誠實忽略**該欄位而不是拒絕整份檔：
  // 拒絕會讓 v0.5.x 使用者自己的匯出檔匯不回來；非舊 id 仍然拒絕（不知道就不猜）。
  // 寬容只適用於「問得出 adapter、但它沒宣告那一項」；問不出 adapter（沒對照表）仍一律拒絕——不猜。
  const legacyTolerant = meta !== null && target !== null && LEGACY_CHARACTER_IDS.includes(target);
  const persona = str("companionPersona", null, 64);
  if (persona !== null && persona.length > 0) {
    const personas = meta?.personas ?? [];
    if (personas.length === 0) {
      if (!legacyTolerant) throw unattributable("說話風格");
    } else if (!personas.some((p) => p.id === persona)) {
      throw new Error(`說話風格「${persona}」不在「${target}」提供的清單裡`);
    } else {
      out.companionPersona = persona;
    }
  }
  const expr = str("companionExpressiveness", EXPRESSIVENESS, 16);
  if (expr !== null) out.companionExpressiveness = expr;
  // 場景是**遊玩場**的東西：以前是這裡自帶的五個 id（某一個 rig 的場景），純文字／幾何角色
  // 照樣收得下，存成一個沒有人吃的死值（對抗審查 character-settings-binding-001）。
  // 現在比照說話風格／使魔：由目標角色的 adapter 宣告，問不出來就誠實拒絕。
  const scene = str("companionScene", null, 16);
  if (scene !== null && scene.length > 0) {
    const scenes = meta?.scenes ?? [];
    if (scenes.length === 0) {
      if (!legacyTolerant) throw unattributable("場景");
    } else if (!scenes.includes(scene)) {
      throw new Error(`場景「${scene}」不在「${target}」提供的清單裡`);
    } else {
      out.companionScene = scene;
    }
  }
  for (const key of [
    "companionPlay",
    "companionCursorPlay",
    "companionApproach",
    "companionDeskMove",
  ] as const) {
    if (typeof obj[key] === "boolean") out[key] = obj[key] as boolean;
  }
  if (Array.isArray(obj.companionFamiliars)) {
    if (obj.companionFamiliars.length > 3) throw new Error("使魔最多 3 隻");
    // 空清單不含任何角色專屬的值，任何角色都收得下（換角色後匯出的檔案才匯得回來）。
    if (obj.companionFamiliars.length > 0 && !meta?.hasPlayfield) {
      // 舊小樞家族：目標角色沒有遊玩場就忽略使魔清單（見上面的說明）；其他角色一律拒絕。
      if (!legacyTolerant) throw unattributable("使魔");
    } else {
      const palettes = meta?.variants ?? [];
      const familiars: { id: string; name: string; palette: string }[] = [];
      for (const f of obj.companionFamiliars) {
        const fo = f as Record<string, unknown>;
        const id = String(fo.id ?? "");
        const fname = String(fo.name ?? "");
        const palette = String(fo.palette ?? "");
        if (!/^[a-zA-Z0-9-]{1,32}$/.test(id)) throw new Error("使魔 id 非法");
        if (fname.length === 0 || fname.length > 24) throw new Error("使魔名字長度非法");
        if (!palettes.includes(palette)) {
          throw new Error(`使魔配色「${palette}」不在「${target}」提供的配色清單裡`);
        }
        familiars.push({ id, name: fname, palette });
      }
      out.companionFamiliars = familiars;
    }
  }
  return out;
}
