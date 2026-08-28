// 角色設定匯出／匯入（spec §5.1）：只搬「呈現偏好」，不含任何權限、
// token、位置或歷史。匯入逐欄白名單驗證——未知欄位丟棄、非法值拒絕。

import { DesktopPrefs } from "../desktop";

export interface CompanionSettingsExport {
  kind: "companion-settings";
  schemaVersion: 1;
  companionName: string;
  companionPack: string;
  companionPersona: string;
  companionExpressiveness: string;
  companionScene: string;
  companionPlay: boolean;
  companionCursorPlay: boolean;
  companionApproach: boolean;
  companionDeskMove: boolean;
  companionFamiliars: { id: string; name: string; palette: string }[];
}

const PACKS = [
  "shu-maid",
  "shu-maid-dusk",
  "shu-maid-sakura",
  "shu-agile",
  "shu-lazy",
  "shu-lively",
  "shu-standard",
  "shu-minimal",
];
const PERSONAS = ["persona-shu", "persona-navigator"];
const EXPRESSIVENESS = ["quiet", "natural", "lively"];
const SCENES = ["none", "nest", "desk", "sill", "night"];
const PALETTES = ["maid-classic", "maid-dusk", "maid-sakura"];

export function exportCompanionSettings(prefs: DesktopPrefs): CompanionSettingsExport {
  return {
    kind: "companion-settings",
    schemaVersion: 1,
    companionName: prefs.companionName ?? "小樞",
    companionPack: prefs.companionPack,
    companionPersona: prefs.companionPersona,
    companionExpressiveness: String(prefs.companionExpressiveness ?? "natural"),
    companionScene: String(prefs.companionScene ?? "none"),
    companionPlay: prefs.companionPlay !== false,
    companionCursorPlay: prefs.companionCursorPlay !== false,
    companionApproach: prefs.companionApproach !== false,
    companionDeskMove: prefs.companionDeskMove !== false,
    companionFamiliars: (prefs.companionFamiliars ?? []).slice(0, 3),
  };
}

/** 驗證匯入 JSON → 可直接 prefsPatch 的部分偏好。非法輸入 throw。 */
export function parseCompanionSettingsImport(raw: unknown): Partial<DesktopPrefs> {
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
  if (name !== null) out.companionName = name;
  const pack = str("companionPack", PACKS, 64);
  if (pack !== null) out.companionPack = pack;
  const persona = str("companionPersona", PERSONAS, 64);
  if (persona !== null) out.companionPersona = persona;
  const expr = str("companionExpressiveness", EXPRESSIVENESS, 16);
  if (expr !== null) out.companionExpressiveness = expr;
  const scene = str("companionScene", SCENES, 16);
  if (scene !== null) out.companionScene = scene;
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
    const familiars: { id: string; name: string; palette: string }[] = [];
    for (const f of obj.companionFamiliars) {
      const fo = f as Record<string, unknown>;
      const id = String(fo.id ?? "");
      const fname = String(fo.name ?? "");
      const palette = String(fo.palette ?? "");
      if (!/^[a-zA-Z0-9-]{1,32}$/.test(id)) throw new Error("使魔 id 非法");
      if (fname.length === 0 || fname.length > 24) throw new Error("使魔名字長度非法");
      if (!PALETTES.includes(palette)) throw new Error("使魔配色不在允許清單");
      familiars.push({ id, name: fname, palette });
    }
    out.companionFamiliars = familiars;
  }
  return out;
}
