// CPP：角色索引（/characters/index.json）載入、匯入 manifest 文字驗證，與 UI 用的
// 能力摘要（內建／第三方、本機／外部、可執行程式、網路、可接收資料、已測試）。
//
// 只讀同源資料；不執行任何 entrypoint、不下載遠端資產。匯入的 manifest 先量大小
// （≤ 256 KB）再 JSON.parse 再驗證；壞掉的項目列入 errors，不會讓整份索引失敗。
// 摘要文案不含任何角色專屬（小樞）敘述——只描述 manifest 宣告了什麼。

import { displayNameOf, validateCharacterManifest, type ManifestReport } from "./manifest";
import { CHARACTER_ID_RE, CharacterManifest, LIMITS } from "./protocol";

export type CharacterOrigin = "builtin" | "imported";

export interface CharacterIndexEntry {
  characterId: string;
  manifestPath: string;
  assetBase?: string;
  /** 索引提示（純資料）：這個角色預設搭配的 persona pack／story pack id；host 可用 prefs 覆寫。 */
  persona?: string;
  story?: string;
  origin: CharacterOrigin;
  manifest: CharacterManifest;
  report: ManifestReport;
}

export interface CharacterIndex {
  schemaVersion: string;
  default: string;
  characters: CharacterIndexEntry[];
  /** 載入失敗的項目（characterId 或路徑索引＋原因）。 */
  errors: string[];
}

export type LoadIndexResult = { ok: true; index: CharacterIndex } | { ok: false; error: string };

type FetchLike = (input: string) => Promise<{ ok: boolean; status: number; text(): Promise<string> }>;

const PATH_RE = /^\/[A-Za-z0-9_\-./]{1,256}$/;

/** persona／story pack id（純資料檔名，不含路徑）。 */
const PACK_HINT_RE = /^[a-z0-9][a-z0-9-]{0,63}$/;

function safePath(p: unknown): p is string {
  return typeof p === "string" && PATH_RE.test(p) && !p.includes("..") && !p.includes("//");
}

/** 匯入用：先量文字大小，再 parse、再驗證。錯誤訊息不回顯內容。 */
export function validateImportedManifestText(text: string, opts: { builtinWhitelist?: readonly string[] } = {}) {
  if (typeof text !== "string") return { ok: false as const, errors: ["manifest text missing"] };
  if (new TextEncoder().encode(text).length > LIMITS.manifestMaxBytes) {
    return { ok: false as const, errors: [`manifest exceeds ${LIMITS.manifestMaxBytes} bytes`] };
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return { ok: false as const, errors: ["manifest is not valid JSON"] };
  }
  return validateCharacterManifest(parsed, { jsonText: text, builtinWhitelist: opts.builtinWhitelist });
}

/**
 * 讀取 /characters/index.json 並逐一載入、驗證 manifest。
 * index 結構：{ schemaVersion, default, characters: [{ characterId, manifestPath, assetBase?, origin }] }。
 */
export async function loadCharacterIndex(
  fetchImpl: FetchLike,
  opts: { indexUrl?: string; builtinWhitelist?: readonly string[] } = {}
): Promise<LoadIndexResult> {
  const indexUrl = opts.indexUrl ?? "/characters/index.json";
  let raw: unknown;
  try {
    const res = await fetchImpl(indexUrl);
    if (!res.ok) return { ok: false, error: `character index fetch failed (${res.status})` };
    const text = await res.text();
    if (new TextEncoder().encode(text).length > LIMITS.manifestMaxBytes) {
      return { ok: false, error: "character index too large" };
    }
    raw = JSON.parse(text);
  } catch {
    return { ok: false, error: "character index unreadable" };
  }
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return { ok: false, error: "character index must be an object" };
  const idx = raw as Record<string, unknown>;
  if (typeof idx.schemaVersion !== "string" || !/^1\.\d{1,4}$/.test(idx.schemaVersion)) {
    return { ok: false, error: "character index schemaVersion must be 1.x" };
  }
  if (!Array.isArray(idx.characters) || idx.characters.length > 256) {
    return { ok: false, error: "character index characters must be an array (≤ 256)" };
  }
  const errors: string[] = [];
  const characters: CharacterIndexEntry[] = [];
  const seen = new Set<string>();
  for (const [i, entry] of idx.characters.entries()) {
    if (!entry || typeof entry !== "object") {
      errors.push(`characters[${i}]: not an object`);
      continue;
    }
    const e = entry as Record<string, unknown>;
    const characterId = typeof e.characterId === "string" && CHARACTER_ID_RE.test(e.characterId) ? e.characterId : null;
    if (!characterId) {
      errors.push(`characters[${i}]: invalid characterId`);
      continue;
    }
    if (seen.has(characterId)) {
      errors.push(`${characterId}: duplicated in index`);
      continue;
    }
    if (!safePath(e.manifestPath)) {
      errors.push(`${characterId}: invalid manifestPath`);
      continue;
    }
    if (e.assetBase !== undefined && !safePath(e.assetBase)) {
      errors.push(`${characterId}: invalid assetBase`);
      continue;
    }
    if (e.persona !== undefined && !PACK_HINT_RE.test(String(e.persona))) {
      errors.push(`${characterId}: invalid persona hint`);
      continue;
    }
    if (e.story !== undefined && !PACK_HINT_RE.test(String(e.story))) {
      errors.push(`${characterId}: invalid story hint`);
      continue;
    }
    const origin: CharacterOrigin = e.origin === "imported" ? "imported" : "builtin";
    let text: string;
    try {
      const res = await fetchImpl(e.manifestPath);
      if (!res.ok) {
        errors.push(`${characterId}: manifest fetch failed (${res.status})`);
        continue;
      }
      text = await res.text();
    } catch {
      errors.push(`${characterId}: manifest unreadable`);
      continue;
    }
    const v = validateImportedManifestText(text, { builtinWhitelist: opts.builtinWhitelist });
    if (!v.ok) {
      errors.push(`${characterId}: ${v.errors[0] ?? "invalid manifest"}`);
      continue;
    }
    if (v.manifest.characterId !== characterId) {
      errors.push(`${characterId}: manifest characterId does not match index`);
      continue;
    }
    seen.add(characterId);
    characters.push({
      characterId,
      manifestPath: e.manifestPath,
      ...(typeof e.assetBase === "string" ? { assetBase: e.assetBase } : {}),
      ...(typeof e.persona === "string" ? { persona: e.persona } : {}),
      ...(typeof e.story === "string" ? { story: e.story } : {}),
      origin,
      manifest: v.manifest,
      report: v.report,
    });
  }
  const def = typeof idx.default === "string" ? idx.default : "";
  if (!seen.has(def)) {
    return { ok: false, error: `character index default "${def.slice(0, 64)}" is not a loadable character` };
  }
  return { ok: true, index: { schemaVersion: idx.schemaVersion, default: def, characters, errors } };
}

// ---------------------------------------------------------------------------
// UI 摘要
// ---------------------------------------------------------------------------

export interface CapabilitySummaryOptions {
  origin?: CharacterOrigin;
  /** host 是否有這個角色的自動化測試證據（內建角色由 CI 覆蓋；匯入者預設無）。 */
  tested?: boolean;
}

const INPUT_LABELS_ZH: Record<string, string> = {
  "input.click": "點擊",
  "input.hover": "游標懸停（節流）",
  "input.drag": "拖曳（量化座標）",
  "input.drop": "放下",
  "input.pointerProximity": "游標靠近（低頻）",
  "input.text": "文字輸入",
  "input.fileDrop": "檔案拖放（只有檔名／類型／大小＋短效授權）",
};

const INPUT_LABELS_EN: Record<string, string> = {
  "input.click": "click",
  "input.hover": "hover (throttled)",
  "input.drag": "drag (quantized coordinates)",
  "input.drop": "drop",
  "input.pointerProximity": "pointer proximity (low rate)",
  "input.text": "text input",
  "input.fileDrop": "file drop (name/type/size + short-lived grant only)",
};

/**
 * 摘要分組：`general` 是一般模式看得到的人話事實（來源與版本、可以接收、需要的裝置、
 * 已測試）；`technical` 是執行方式與安全宣告（執行位置、可執行程式、需要網路、檔案存取、
 * 簽章），只在進階模式的技術資料出現。分組只影響「放在哪一層」，不隱藏任何事實。
 */
export interface CapabilitySummaryParts {
  general: string[];
  technical: string[];
}

/**
 * 給 UI 的人話摘要（一般＋進階兩組）：來源、執行位置、可執行程式、網路、可接收的資料、
 * 測試狀態、感測需求。每一行都只是 manifest 宣告的轉述，不評價角色本身。
 */
export function capabilitySummaryParts(
  manifest: CharacterManifest,
  locale: string,
  opts: CapabilitySummaryOptions = {}
): CapabilitySummaryParts {
  const zh = locale.toLowerCase().startsWith("zh");
  const name = displayNameOf(manifest, locale);
  const origin = opts.origin ?? "imported";
  const sec = manifest.securityRequirements;
  const general: string[] = [];
  const technical: string[] = [];

  general.push(
    zh
      ? `${name}：${origin === "builtin" ? "內建角色" : "第三方角色"}（${manifest.author ? `作者 ${manifest.author}` : "作者未標示"}，版本 ${manifest.version}）`
      : `${name}: ${origin === "builtin" ? "built-in character" : "third-party character"} (${manifest.author ? `author ${manifest.author}` : "author not stated"}, version ${manifest.version})`
  );

  const kindZh: Record<CharacterManifest["adapterKind"], string> = {
    "in-process": "在本機視窗內執行（內建 adapter）",
    web: "本機模組（啟用後才由 host 載入）",
    "external-process": "外部程式（永不自動啟動，需明確安裝與授權）",
    "remote-device": "遠端裝置（永不自動連線，需配對）",
  };
  const kindEn: Record<CharacterManifest["adapterKind"], string> = {
    "in-process": "runs inside the local window (built-in adapter)",
    web: "local module (loaded by the host only after you enable it)",
    "external-process": "external program (never auto-started; requires explicit install and approval)",
    "remote-device": "remote device (never auto-connects; requires pairing)",
  };
  technical.push(zh ? kindZh[manifest.adapterKind] : kindEn[manifest.adapterKind]);

  const executable = sec.executable || manifest.entrypoint.kind === "process";
  technical.push(
    zh
      ? executable
        ? "有可執行程式：是（只記錄，不會自動執行）"
        : "有可執行程式：否（純資料）"
      : executable
        ? "Executable content: yes (recorded only, never auto-run)"
        : "Executable content: no (data only)"
  );
  technical.push(
    zh
      ? sec.network
        ? "需要網路：是"
        : "需要網路：否"
      : sec.network
        ? "Needs network: yes"
        : "Needs network: no"
  );

  const inputs = Object.keys(manifest.inputCapabilities).filter((id) => manifest.inputCapabilities[id]?.supported);
  // 不認得的 input id 一律用中性詞，一般模式不會看到原始 id。
  const labels = Array.from(
    new Set(inputs.map((id) => (zh ? INPUT_LABELS_ZH[id] : INPUT_LABELS_EN[id]) ?? (zh ? "其他互動" : "other interaction")))
  );
  general.push(
    zh
      ? labels.length > 0
        ? `可以接收：${labels.join("、")}`
        : "可以接收：不接收任何輸入"
      : labels.length > 0
        ? `Can receive: ${labels.join(", ")}`
        : "Can receive: no input"
  );

  const fileAccessZh: Record<typeof sec.fileAccess, string> = {
    none: "不讀取檔案",
    "character-folder": "只讀角色資料夾",
    "user-granted": "只讀你明確拖放並授權的檔案（短效）",
  };
  const fileAccessEn: Record<typeof sec.fileAccess, string> = {
    none: "reads no files",
    "character-folder": "reads only its own character folder",
    "user-granted": "reads only files you explicitly drop and grant (short-lived)",
  };
  technical.push(zh ? `檔案存取：${fileAccessZh[sec.fileAccess]}` : `File access: ${fileAccessEn[sec.fileAccess]}`);

  const sensors: string[] = [];
  if (sec.microphone) sensors.push(zh ? "麥克風" : "microphone");
  if (sec.camera) sensors.push(zh ? "攝影機" : "camera");
  if (sec.audioOutput) sensors.push(zh ? "音訊輸出" : "audio output");
  general.push(
    zh
      ? sensors.length > 0
        ? `需要的裝置：${sensors.join("、")}（感測器預設關閉，啟用時會顯示指示）`
        : "需要的裝置：無"
      : sensors.length > 0
        ? `Devices requested: ${sensors.join(", ")} (sensors are off by default; an indicator shows when active)`
        : "Devices requested: none"
  );

  const tested = opts.tested ?? origin === "builtin";
  general.push(
    zh
      ? tested
        ? "已測試：是（隨 App 自動化測試）"
        : "已測試：否（未經本機測試；請先在受控環境試用）"
      : tested
        ? "Tested: yes (covered by the app's automated tests)"
        : "Tested: no (not tested on this machine; try it in a controlled setting first)"
  );

  technical.push(zh ? "簽章：無（本版不支援簽章驗證）" : "Signature: none (this version does not verify signatures)");
  return { general, technical };
}

/** 完整摘要（一般＋進階）。既有呼叫端（「可以接收：…」那一行）維持不變。 */
export function capabilitySummary(
  manifest: CharacterManifest,
  locale: string,
  opts: CapabilitySummaryOptions = {}
): string[] {
  const parts = capabilitySummaryParts(manifest, locale, opts);
  return [...parts.general, ...parts.technical];
}
