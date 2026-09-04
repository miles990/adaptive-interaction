// CPP §2：CharacterManifest 驗證（§2.1）、舊 pack 遷移（§2.2）與顯示用 helper。
//
// 驗證器是第三方角色的第一道防線：路徑穿越、可執行 entrypoint、巨大資產、
// 巢狀 schema 都在這裡擋下；錯誤訊息不回顯輸入內容（只講欄位與規則），
// 不含絕對路徑。entrypoint 的 process／url／module 只記錄，永不執行／連線／下載。

import { builtinEntrypointIds } from "./adapterRegistry";
import { validateManifest as validateLegacyPack, type PackManifest } from "../companion/renderer";
import {
  AdapterKind,
  AssetDecl,
  CANONICAL_CAPABILITY_PREFIXES,
  CapabilityDecl,
  CHARACTER_ID_RE,
  CharacterIntent,
  CharacterManifest,
  Compatibility,
  CUSTOM_CAPABILITY_ID_RE,
  Entrypoint,
  FallbackDecl,
  isCanonicalCapabilityId,
  isCharacterIntent,
  isSafetyIntent,
  LIMITS,
  LocalizedText,
  parseProtocolVersion,
  PreferencePropertySchema,
  PreferencesSchema,
  PROTOCOL_MINOR,
  ResourceLimits,
  SecurityRequirements,
  VariantDecl,
} from "./protocol";
import { deriveIntentFallbacks, nativeIntentsOf } from "./spriteIntents";

// ---------------------------------------------------------------------------
// 結果型別
// ---------------------------------------------------------------------------

export interface ManifestReport {
  /** schemaVersion minor 大於實作者（未知欄位保留、不崩潰）。 */
  newerMinor: boolean;
  /** canonical 前綴但未收錄的能力 id（已標 unknown: true）。 */
  unknownCapabilities: string[];
  /** namespaced custom 能力 id。 */
  customCapabilities: string[];
  warnings: string[];
  flags: {
    /** adapterKind ≠ in-process。 */
    external: boolean;
    network: boolean;
    executable: boolean;
    /** 本版沒有簽章機制：一律 true。 */
    unsigned: boolean;
  };
}

export type ManifestValidation =
  | { ok: true; manifest: CharacterManifest; report: ManifestReport }
  | { ok: false; errors: string[] };

export interface ValidateOptions {
  /** 若提供，檢查 JSON 文字大小 ≤ 256 KB。 */
  jsonText?: string;
  /** host 的 builtin entrypoint 白名單（預設取 adapter registry 的宣告清單）。 */
  builtinWhitelist?: readonly string[];
  /** 實作者的 schema minor（預設 PROTOCOL_MINOR）。 */
  implMinor?: number;
}

const SEMVER_RE = /^\d{1,6}\.\d{1,6}\.\d{1,6}(?:[-+][0-9A-Za-z.-]{1,64})?$/;
const LOCALE_KEY_RE = /^[A-Za-z]{2,3}(?:-[A-Za-z0-9]{2,8})*$/;
const VARIANT_ID_RE = /^[a-z0-9][a-z0-9._-]{0,63}$/;
const ASSET_ID_RE = /^[a-z0-9][a-z0-9._-]{0,63}$/;
const URL_SCHEME_RE = /^[a-zA-Z][a-zA-Z0-9+.-]*:/;
const DRIVE_RE = /^[a-zA-Z]:/;
const SHA256_RE = /^[a-f0-9]{64}$/i;
/** §2.1 media type：與 Rust `is_media_type()` 同一套（全小寫，不接受大寫）。 */
const MEDIA_TYPE_RE = /^[a-z0-9.+-]+\/[a-z0-9.+-]+$/;
const MAX_ERRORS = 64;

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function utf8Bytes(text: string): number {
  return new TextEncoder().encode(text).length;
}

/** 資產路徑規則（§2.1）：回傳違規原因或 null。永遠不回顯路徑本身。 */
export function assetPathIssue(path: unknown): string | null {
  if (typeof path !== "string" || path.length === 0) return "path must be a non-empty string";
  if (path.length > 512) return "path too long (max 512)";
  if (path.includes("\0")) return "path contains NUL";
  if (path.includes("..")) return "path must not contain '..'";
  if (path.startsWith("/") || path.startsWith("\\")) return "path must be relative";
  if (DRIVE_RE.test(path)) return "path must not start with a drive letter";
  if (path.includes("\\")) return "path must not contain backslashes";
  if (URL_SCHEME_RE.test(path)) return "path must not be a URL";
  if (path.startsWith("~")) return "path must not start with '~'";
  return null;
}

class Collector {
  errors: string[] = [];
  warnings: string[] = [];
  err(msg: string) {
    if (this.errors.length < MAX_ERRORS) this.errors.push(msg);
  }
  warn(msg: string) {
    if (this.warnings.length < MAX_ERRORS) this.warnings.push(msg);
  }
}

function checkLocalized(
  c: Collector,
  field: string,
  value: unknown,
  maxChars: number,
  required: boolean
): LocalizedText | undefined {
  if (value === undefined) {
    if (required) c.err(`${field} is required`);
    return undefined;
  }
  if (!isPlainObject(value)) {
    c.err(`${field} must be an object of locale → text`);
    return undefined;
  }
  const out: LocalizedText = {};
  const keys = Object.keys(value);
  if (keys.length === 0) {
    if (required) c.err(`${field} needs at least one locale`);
    return required ? undefined : out;
  }
  if (keys.length > 32) c.err(`${field} has too many locales (max 32)`);
  for (const key of keys) {
    if (!LOCALE_KEY_RE.test(key)) {
      c.err(`${field} has an invalid locale key`);
      continue;
    }
    const text = value[key];
    if (typeof text !== "string" || text.length === 0) {
      c.err(`${field}.${key} must be a non-empty string`);
      continue;
    }
    if (text.length > maxChars) {
      c.err(`${field}.${key} exceeds ${maxChars} chars`);
      continue;
    }
    out[key] = text;
  }
  return out;
}

function checkPreferencesSchema(
  c: Collector,
  field: string,
  value: unknown
): PreferencesSchema | undefined {
  if (value === undefined) return undefined;
  if (!isPlainObject(value)) {
    c.err(`${field} must be an object`);
    return undefined;
  }
  if (value.type !== "object") {
    c.err(`${field}.type must be "object"`);
    return undefined;
  }
  for (const key of Object.keys(value)) {
    if (key !== "type" && key !== "properties" && key !== "title" && key !== "description") {
      c.err(`${field} has unsupported keyword "${key.slice(0, 40)}"`);
      return undefined;
    }
  }
  const props = value.properties;
  const out: PreferencesSchema = { type: "object", properties: {} };
  if (props === undefined) return out;
  if (!isPlainObject(props)) {
    c.err(`${field}.properties must be an object`);
    return undefined;
  }
  const names = Object.keys(props);
  if (names.length > LIMITS.preferencesMaxProperties) {
    c.err(`${field}.properties exceeds ${LIMITS.preferencesMaxProperties} entries`);
    return undefined;
  }
  for (const name of names) {
    if (!/^[A-Za-z][A-Za-z0-9_-]{0,63}$/.test(name)) {
      c.err(`${field}.properties has an invalid property name`);
      return undefined;
    }
    const p = props[name];
    if (!isPlainObject(p)) {
      c.err(`${field}.properties.${name} must be an object`);
      return undefined;
    }
    const allowed = new Set([
      "type",
      "minimum",
      "maximum",
      "maxLength",
      "enum",
      "default",
      "title",
      "description",
    ]);
    for (const key of Object.keys(p)) {
      if (!allowed.has(key)) {
        c.err(`${field}.properties.${name} has unsupported keyword "${key.slice(0, 40)}"`);
        return undefined;
      }
    }
    const type = p.type;
    if (type !== "boolean" && type !== "number" && type !== "integer" && type !== "string") {
      c.err(`${field}.properties.${name}.type must be boolean/number/integer/string`);
      return undefined;
    }
    const prop: PreferencePropertySchema = { type };
    if (p.minimum !== undefined) {
      if (typeof p.minimum !== "number" || !Number.isFinite(p.minimum) || type === "boolean" || type === "string") {
        c.err(`${field}.properties.${name}.minimum invalid`);
        return undefined;
      }
      prop.minimum = p.minimum;
    }
    if (p.maximum !== undefined) {
      if (typeof p.maximum !== "number" || !Number.isFinite(p.maximum) || type === "boolean" || type === "string") {
        c.err(`${field}.properties.${name}.maximum invalid`);
        return undefined;
      }
      prop.maximum = p.maximum;
    }
    if (p.maxLength !== undefined) {
      if (
        type !== "string" ||
        !Number.isInteger(p.maxLength) ||
        (p.maxLength as number) < 0 ||
        (p.maxLength as number) > LIMITS.preferencesStringMaxLength
      ) {
        c.err(`${field}.properties.${name}.maxLength must be an integer ≤ ${LIMITS.preferencesStringMaxLength}`);
        return undefined;
      }
      prop.maxLength = p.maxLength as number;
    }
    if (type === "string" && p.maxLength === undefined) {
      prop.maxLength = LIMITS.preferencesStringMaxLength;
    }
    if (p.enum !== undefined) {
      if (
        type !== "string" ||
        !Array.isArray(p.enum) ||
        p.enum.length === 0 ||
        p.enum.length > LIMITS.preferencesEnumMax ||
        p.enum.some((e) => typeof e !== "string" || e.length > LIMITS.preferencesStringMaxLength)
      ) {
        c.err(`${field}.properties.${name}.enum must be ≤ ${LIMITS.preferencesEnumMax} short strings`);
        return undefined;
      }
      prop.enum = [...(p.enum as string[])];
    }
    if (p.default !== undefined) {
      const d = p.default;
      const okDefault =
        (type === "boolean" && typeof d === "boolean") ||
        ((type === "number" || type === "integer") && typeof d === "number" && Number.isFinite(d)) ||
        (type === "string" && typeof d === "string" && d.length <= LIMITS.preferencesStringMaxLength);
      if (!okDefault) {
        c.err(`${field}.properties.${name}.default does not match type`);
        return undefined;
      }
      prop.default = d as boolean | number | string;
    }
    for (const textKey of ["title", "description"] as const) {
      const t = p[textKey];
      if (t !== undefined) {
        if (typeof t !== "string" || t.length > LIMITS.stringMaxChars) {
          c.err(`${field}.properties.${name}.${textKey} must be a string ≤ ${LIMITS.stringMaxChars}`);
          return undefined;
        }
        prop[textKey] = t;
      }
    }
    out.properties![name] = prop;
  }
  return out;
}

function checkCapabilityDecl(c: Collector, field: string, value: unknown): CapabilityDecl | null {
  if (!isPlainObject(value)) {
    c.err(`${field} must be an object`);
    return null;
  }
  if (typeof value.supported !== "boolean") {
    c.err(`${field}.supported must be a boolean`);
    return null;
  }
  const out: CapabilityDecl = { supported: value.supported };
  let bad = false;
  const fail = (msg: string) => {
    c.err(`${field}.${msg}`);
    bad = true;
  };
  if (value.version !== undefined) {
    if (typeof value.version !== "string" || value.version.length > 32) fail("version invalid");
    else out.version = value.version;
  }
  if (value.variants !== undefined) {
    if (
      !Array.isArray(value.variants) ||
      value.variants.length > LIMITS.maxVariants ||
      value.variants.some((v) => typeof v !== "string" || !VARIANT_ID_RE.test(v))
    )
      fail(`variants must be ≤ ${LIMITS.maxVariants} ids`);
    else out.variants = [...(value.variants as string[])];
  }
  if (value.maxConcurrent !== undefined) {
    const n = value.maxConcurrent;
    if (!Number.isInteger(n) || (n as number) < 1 || (n as number) > LIMITS.maxConcurrentCommandsCap)
      fail(`maxConcurrent must be 1..${LIMITS.maxConcurrentCommandsCap}`);
    else out.maxConcurrent = n as number;
  }
  for (const b of ["interruptible", "resumable", "requiresForeground", "requiresAudio"] as const) {
    if (value[b] !== undefined) {
      if (typeof value[b] !== "boolean") fail(`${b} must be a boolean`);
      else out[b] = value[b] as boolean;
    }
  }
  if (value.durationRange !== undefined) {
    const d = value.durationRange;
    if (
      !isPlainObject(d) ||
      typeof d.minMs !== "number" ||
      typeof d.maxMs !== "number" ||
      !Number.isFinite(d.minMs) ||
      !Number.isFinite(d.maxMs) ||
      d.minMs < 0 ||
      d.maxMs > LIMITS.durationMaxMs ||
      d.minMs > d.maxMs
    )
      fail(`durationRange must satisfy 0 ≤ minMs ≤ maxMs ≤ ${LIMITS.durationMaxMs}`);
    else out.durationRange = { minMs: d.minMs, maxMs: d.maxMs };
  }
  if (value.parameterSchema !== undefined) {
    const before = c.errors.length;
    const schema = checkPreferencesSchema(c, `${field}.parameterSchema`, value.parameterSchema);
    if (!schema || c.errors.length !== before) bad = true;
    else out.parameterSchema = schema;
  }
  if (value.qualityLevel !== undefined) {
    if (value.qualityLevel !== "full" && value.qualityLevel !== "reduced" && value.qualityLevel !== "minimal")
      fail("qualityLevel invalid");
    else out.qualityLevel = value.qualityLevel;
  }
  if (value.reducedMotionBehavior !== undefined) {
    const r = value.reducedMotionBehavior;
    if (r !== "static" && r !== "reduced" && r !== "unchanged" && r !== "disabled")
      fail("reducedMotionBehavior invalid");
    else out.reducedMotionBehavior = r;
  }
  return bad ? null : out;
}

interface CapabilityMapResult {
  map: Record<string, CapabilityDecl>;
  unknown: string[];
  custom: string[];
}

function checkCapabilityMap(c: Collector, field: string, value: unknown): CapabilityMapResult {
  const result: CapabilityMapResult = { map: {}, unknown: [], custom: [] };
  if (value === undefined) return result;
  if (!isPlainObject(value)) {
    c.err(`${field} must be an object`);
    return result;
  }
  const keys = Object.keys(value);
  if (keys.length > 128) {
    c.err(`${field} has too many entries (max 128)`);
    return result;
  }
  for (const id of keys) {
    if (id.length > 128) {
      c.err(`${field} has an over-long capability id`);
      continue;
    }
    let unknownFlag = false;
    if (id === "system.text") {
      c.err(`${field} must not declare system.text (provided by the runtime)`);
      continue;
    }
    if (isCanonicalCapabilityId(id)) {
      // canonical
    } else if (CUSTOM_CAPABILITY_ID_RE.test(id)) {
      result.custom.push(id);
    } else if (CANONICAL_CAPABILITY_PREFIXES.some((p) => id.startsWith(p)) && /^[a-z][a-zA-Z0-9.]*$/.test(id)) {
      unknownFlag = true;
      result.unknown.push(id);
    } else {
      c.err(`${field} has an invalid capability id (canonical or namespaced custom required)`);
      continue;
    }
    const decl = checkCapabilityDecl(c, `${field}["${id.slice(0, 64)}"]`, value[id]);
    if (!decl) continue;
    if (unknownFlag) decl.unknown = true;
    result.map[id] = decl;
  }
  return result;
}

function checkEntrypoint(
  c: Collector,
  value: unknown,
  adapterKind: AdapterKind | null,
  whitelist: readonly string[]
): Entrypoint | null {
  if (!isPlainObject(value)) {
    c.err("entrypoint must be an object");
    return null;
  }
  const kind = value.kind;
  const expect: Record<AdapterKind, string> = {
    "in-process": "builtin",
    web: "module",
    "external-process": "process",
    "remote-device": "url",
  };
  if (adapterKind && expect[adapterKind] !== kind) {
    c.err(`entrypoint.kind must be "${expect[adapterKind]}" for adapterKind "${adapterKind}"`);
    return null;
  }
  switch (kind) {
    case "builtin": {
      if (typeof value.id !== "string" || !whitelist.includes(value.id)) {
        c.err("entrypoint.builtin id is not in the host whitelist");
        return null;
      }
      return { kind: "builtin", id: value.id };
    }
    case "module": {
      const issue = assetPathIssue(value.path);
      if (issue) {
        c.err(`entrypoint.module ${issue}`);
        return null;
      }
      return { kind: "module", path: value.path as string };
    }
    case "process": {
      const cmd = value.command;
      if (
        !Array.isArray(cmd) ||
        cmd.length === 0 ||
        cmd.length > 32 ||
        cmd.some((s) => typeof s !== "string" || s.length === 0 || s.length > 512)
      ) {
        c.err("entrypoint.process command must be a non-empty string array");
        return null;
      }
      return { kind: "process", command: [...(cmd as string[])] };
    }
    case "url": {
      const u = value.url;
      if (typeof u !== "string" || u.length > 512) {
        c.err("entrypoint.url must be a string ≤ 512");
        return null;
      }
      let parsed: URL;
      try {
        parsed = new URL(u);
      } catch {
        c.err("entrypoint.url is not a valid URL");
        return null;
      }
      if (parsed.protocol !== "ws:" && parsed.protocol !== "wss:") {
        c.err("entrypoint.url must use ws:// or wss://");
        return null;
      }
      return { kind: "url", url: u };
    }
    default:
      c.err("entrypoint.kind must be builtin/module/process/url");
      return null;
  }
}

function checkResourceLimits(c: Collector, value: unknown): ResourceLimits {
  const out: ResourceLimits = {
    maxAssetBytes: 8 * 1024 * 1024,
    maxConcurrentCommands: 4,
    maxQueue: 32,
    maxFps: 60,
  };
  if (value === undefined) return out;
  if (!isPlainObject(value)) {
    c.err("resourceLimits must be an object");
    return out;
  }
  const num = (k: keyof ResourceLimits, min: number, max: number) => {
    const v = value[k];
    if (v === undefined) return;
    if (!Number.isInteger(v) || (v as number) < min || (v as number) > max) {
      c.err(`resourceLimits.${k} must be an integer ${min}..${max}`);
      return;
    }
    out[k] = v as number;
  };
  num("maxAssetBytes", 0, LIMITS.maxAssetBytesCap);
  num("maxConcurrentCommands", 1, LIMITS.maxConcurrentCommandsCap);
  num("maxQueue", 1, LIMITS.maxQueueCap);
  num("maxFps", 1, LIMITS.maxFpsCap);
  return out;
}

function checkSecurity(c: Collector, value: unknown): SecurityRequirements {
  const out: SecurityRequirements = {
    network: false,
    executable: false,
    fileAccess: "none",
    audioOutput: false,
    microphone: false,
    camera: false,
  };
  if (value === undefined) return out;
  if (!isPlainObject(value)) {
    c.err("securityRequirements must be an object");
    return out;
  }
  for (const k of ["network", "executable", "audioOutput", "microphone", "camera"] as const) {
    if (value[k] === undefined) continue;
    if (typeof value[k] !== "boolean") c.err(`securityRequirements.${k} must be a boolean`);
    else out[k] = value[k] as boolean;
  }
  if (value.fileAccess !== undefined) {
    const f = value.fileAccess;
    if (f !== "none" && f !== "character-folder" && f !== "user-granted")
      c.err("securityRequirements.fileAccess must be none/character-folder/user-granted");
    else out.fileAccess = f;
  }
  return out;
}

function checkAssets(c: Collector, value: unknown, maxAssetBytes: number): AssetDecl[] {
  if (value === undefined) return [];
  if (!Array.isArray(value)) {
    c.err("assets must be an array");
    return [];
  }
  if (value.length > LIMITS.maxAssets) {
    c.err(`assets exceeds ${LIMITS.maxAssets} entries`);
    return [];
  }
  const out: AssetDecl[] = [];
  const ids = new Set<string>();
  value.forEach((a, i) => {
    if (!isPlainObject(a)) {
      c.err(`assets[${i}] must be an object`);
      return;
    }
    if (typeof a.id !== "string" || !ASSET_ID_RE.test(a.id)) {
      c.err(`assets[${i}].id invalid`);
      return;
    }
    if (ids.has(a.id)) {
      c.err(`assets[${i}].id duplicated`);
      return;
    }
    const issue = assetPathIssue(a.path);
    if (issue) {
      c.err(`assets[${i}].${issue}`);
      return;
    }
    // §2.1：mediaType 必填，且用與 Rust `is_media_type()` 同一套規則（全小寫的
    // `<kind>/<sub>`）。兩邊只要有一邊寬鬆，就會出現「這裡說合法、那裡說不合法」
    // 的跨語言分岔（對抗審查 character-package-020）。
    if (typeof a.mediaType !== "string" || !MEDIA_TYPE_RE.test(a.mediaType)) {
      c.err(`assets[${i}].mediaType invalid`);
      return;
    }
    const asset: AssetDecl = { id: a.id, path: a.path as string, mediaType: a.mediaType };
    if (a.bytes !== undefined) {
      if (!Number.isInteger(a.bytes) || (a.bytes as number) < 0) {
        c.err(`assets[${i}].bytes must be a non-negative integer`);
        return;
      }
      if ((a.bytes as number) > maxAssetBytes) {
        c.err(`assets[${i}].bytes exceeds resourceLimits.maxAssetBytes`);
        return;
      }
      asset.bytes = a.bytes as number;
    }
    if (a.sha256 !== undefined) {
      if (typeof a.sha256 !== "string" || !SHA256_RE.test(a.sha256)) {
        c.err(`assets[${i}].sha256 must be 64 hex chars`);
        return;
      }
      asset.sha256 = a.sha256.toLowerCase();
    }
    ids.add(a.id);
    out.push(asset);
  });
  return out;
}

function checkStringList(
  c: Collector,
  field: string,
  value: unknown,
  max: number,
  itemRe: RegExp,
  itemMax = 128
): string[] {
  if (value === undefined) return [];
  if (!Array.isArray(value)) {
    c.err(`${field} must be an array`);
    return [];
  }
  if (value.length > max) {
    c.err(`${field} exceeds ${max} entries`);
    return [];
  }
  const out: string[] = [];
  for (const v of value) {
    if (typeof v !== "string" || v.length === 0 || v.length > itemMax || !itemRe.test(v)) {
      c.err(`${field} contains an invalid entry`);
      continue;
    }
    if (!out.includes(v)) out.push(v);
  }
  return out;
}

function checkVariants(c: Collector, value: unknown): VariantDecl[] {
  if (value === undefined) return [];
  if (!Array.isArray(value)) {
    c.err("variants must be an array");
    return [];
  }
  if (value.length > LIMITS.maxVariants) {
    c.err(`variants exceeds ${LIMITS.maxVariants} entries`);
    return [];
  }
  const out: VariantDecl[] = [];
  const seen = new Set<string>();
  value.forEach((v, i) => {
    if (!isPlainObject(v) || typeof v.id !== "string" || !VARIANT_ID_RE.test(v.id)) {
      c.err(`variants[${i}].id invalid`);
      return;
    }
    if (seen.has(v.id)) {
      c.err(`variants[${i}].id duplicated`);
      return;
    }
    const decl: VariantDecl = { id: v.id };
    const dn = checkLocalized(c, `variants[${i}].displayName`, v.displayName, LIMITS.localizedDisplayNameMaxChars, false);
    if (dn && Object.keys(dn).length > 0) decl.displayName = dn;
    seen.add(v.id);
    out.push(decl);
  });
  return out;
}

/** 丟掉「安全 intent → 非安全 intent」的映射（§3.4 步驟 2 守衛；遷移與驗證共用規則）。 */
function safeIntentFallbacks(
  intents: Partial<Record<CharacterIntent, CharacterIntent>>
): Partial<Record<CharacterIntent, CharacterIntent>> {
  const out: Partial<Record<CharacterIntent, CharacterIntent>> = {};
  for (const [from, to] of Object.entries(intents) as Array<[CharacterIntent, CharacterIntent]>) {
    if (isSafetyIntent(from) && !isSafetyIntent(to)) continue;
    out[from] = to;
  }
  return out;
}

function checkFallbacks(c: Collector, value: unknown, caps: Record<string, CapabilityDecl>): FallbackDecl {
  const out: FallbackDecl = {};
  if (value === undefined) return out;
  if (!isPlainObject(value)) {
    c.err("fallbacks must be an object");
    return out;
  }
  if (value.capabilities !== undefined) {
    if (!isPlainObject(value.capabilities)) {
      c.err("fallbacks.capabilities must be an object");
    } else {
      const map: Record<string, string[]> = {};
      const entries = Object.entries(value.capabilities);
      if (entries.length > 64) c.err("fallbacks.capabilities has too many entries (max 64)");
      for (const [cap, chain] of entries.slice(0, 64)) {
        const validId = (id: string) =>
          id.length <= 128 && (isCanonicalCapabilityId(id) || CUSTOM_CAPABILITY_ID_RE.test(id) || id in caps);
        if (!validId(cap)) {
          c.err("fallbacks.capabilities has an invalid capability id");
          continue;
        }
        if (!Array.isArray(chain) || chain.length > 16 || chain.some((x) => typeof x !== "string" || !validId(x))) {
          c.err("fallbacks.capabilities chain must be ≤ 16 valid capability ids");
          continue;
        }
        map[cap] = (chain as string[]).filter((x) => x !== cap);
      }
      out.capabilities = map;
    }
  }
  if (value.intents !== undefined) {
    if (!isPlainObject(value.intents)) {
      c.err("fallbacks.intents must be an object");
    } else {
      const map: Partial<Record<CharacterIntent, CharacterIntent>> = {};
      for (const [from, to] of Object.entries(value.intents)) {
        if (!isCharacterIntent(from)) {
          c.warn("fallbacks.intents has an unknown source intent (ignored)");
          continue;
        }
        if (!isCharacterIntent(to)) {
          c.warn(`fallbacks.intents.${from} targets an unknown intent (ignored)`);
          continue;
        }
        if (to === from) continue;
        // 安全 intent 只能退到另一個安全 intent：呈現層不得用 fallbacks.intents
        // 把「需要同意／被阻擋／失敗／離線」換成 greet／play 之類的日常演出。
        if (isSafetyIntent(from) && !isSafetyIntent(to)) {
          c.err(`fallbacks.intents.${from} may only fall back to another safety intent`);
          continue;
        }
        map[from] = to;
      }
      out.intents = map;
    }
  }
  return out;
}

function checkCompatibility(c: Collector, value: unknown): Compatibility {
  const out: Compatibility = { protocol: "1.x" };
  if (value === undefined) return out;
  if (!isPlainObject(value)) {
    c.err("compatibility must be an object");
    return out;
  }
  if (value.protocol !== undefined) {
    const p = value.protocol;
    if (typeof p !== "string" || !/^1(\.(x|\d{1,4}))?$/.test(p)) {
      c.err("compatibility.protocol must be 1.x or 1.N");
    } else out.protocol = p;
  }
  if (value.runtime !== undefined) {
    if (typeof value.runtime !== "string" || value.runtime.length > 64)
      c.err("compatibility.runtime must be a short string");
    else out.runtime = value.runtime;
  }
  return out;
}

const RESERVED_TOP_LEVEL = new Set([
  "schemaVersion",
  "characterId",
  "displayName",
  "author",
  "description",
  "version",
  "adapterKind",
  "entrypoint",
  "assets",
  "capabilities",
  "inputCapabilities",
  "channels",
  "states",
  "intents",
  "variants",
  "locales",
  "pronouns",
  "preferencesSchema",
  "securityRequirements",
  "resourceLimits",
  "fallbacks",
  "compatibility",
]);

/**
 * §2.1 驗證。回傳正規化後的 manifest（預設值補齊、未知頂層欄位保留），
 * 或錯誤清單（不回顯輸入內容、不含絕對路徑）。
 */
export function validateCharacterManifest(input: unknown, opts: ValidateOptions = {}): ManifestValidation {
  const c = new Collector();
  if (opts.jsonText !== undefined && utf8Bytes(opts.jsonText) > LIMITS.manifestMaxBytes) {
    return { ok: false, errors: [`manifest exceeds ${LIMITS.manifestMaxBytes} bytes`] };
  }
  if (!isPlainObject(input)) return { ok: false, errors: ["manifest must be a JSON object"] };
  const m = input;
  const whitelist = opts.builtinWhitelist ?? builtinEntrypointIds();
  const implMinor = opts.implMinor ?? PROTOCOL_MINOR;

  // schemaVersion
  let newerMinor = false;
  const ver = parseProtocolVersion(m.schemaVersion);
  if (!ver) c.err("schemaVersion must be major.minor");
  else if (ver.major !== 1) c.err("schemaVersion major must be 1");
  else if (ver.minor > implMinor) {
    newerMinor = true;
    c.warn("schemaVersion minor is newer than this implementation; unknown fields preserved");
  }

  // characterId
  if (typeof m.characterId !== "string" || !CHARACTER_ID_RE.test(m.characterId)) {
    c.err("characterId must match ^[a-z0-9][a-z0-9._-]{0,63}$");
  }

  const displayName = checkLocalized(c, "displayName", m.displayName, LIMITS.localizedDisplayNameMaxChars, true);
  const description = checkLocalized(c, "description", m.description, LIMITS.localizedDescriptionMaxChars, false);
  const pronouns = checkLocalized(c, "pronouns", m.pronouns, LIMITS.pronounMaxChars, false);

  if (m.author !== undefined && (typeof m.author !== "string" || m.author.length > LIMITS.authorMaxChars)) {
    c.err(`author must be a string ≤ ${LIMITS.authorMaxChars}`);
  }
  if (typeof m.version !== "string" || !SEMVER_RE.test(m.version)) {
    c.err("version must be a semver string");
  }

  const adapterKind: AdapterKind | null =
    m.adapterKind === "in-process" ||
    m.adapterKind === "web" ||
    m.adapterKind === "external-process" ||
    m.adapterKind === "remote-device"
      ? m.adapterKind
      : null;
  if (!adapterKind) c.err("adapterKind must be in-process/web/external-process/remote-device");
  const entrypoint = checkEntrypoint(c, m.entrypoint, adapterKind, whitelist);

  const resourceLimits = checkResourceLimits(c, m.resourceLimits);
  const assets = checkAssets(c, m.assets, resourceLimits.maxAssetBytes);
  const caps = checkCapabilityMap(c, "capabilities", m.capabilities);
  const inputCaps = checkCapabilityMap(c, "inputCapabilities", m.inputCapabilities);
  const channels = checkStringList(c, "channels", m.channels, LIMITS.maxChannels, /^[a-z][a-zA-Z0-9.]*$/);
  const states = checkStringList(c, "states", m.states, LIMITS.maxStates, /^[a-z0-9][a-zA-Z0-9._-]*$/, 64);

  const intents: CharacterIntent[] = [];
  if (m.intents !== undefined) {
    if (!Array.isArray(m.intents) || m.intents.length > 64) c.err("intents must be an array (≤ 64)");
    else {
      for (const i of m.intents) {
        if (isCharacterIntent(i)) {
          if (!intents.includes(i)) intents.push(i);
        } else c.warn("intents contains an unknown intent (ignored)");
      }
    }
  }
  const variants = checkVariants(c, m.variants);
  const locales = checkStringList(c, "locales", m.locales, 32, LOCALE_KEY_RE, 16);
  const preferencesSchema = checkPreferencesSchema(c, "preferencesSchema", m.preferencesSchema);
  const securityRequirements = checkSecurity(c, m.securityRequirements);
  const fallbacks = checkFallbacks(c, m.fallbacks, caps.map);
  const compatibility = checkCompatibility(c, m.compatibility);

  if (adapterKind === "external-process" && !securityRequirements.executable) {
    c.warn("external-process adapter implies executable content; flagged as executable");
    securityRequirements.executable = true;
  }
  if (adapterKind === "remote-device" && !securityRequirements.network) {
    c.warn("remote-device adapter implies network access; flagged as network");
    securityRequirements.network = true;
  }
  for (const id of Object.keys(inputCaps.map)) {
    if (!id.startsWith("input.") && !CUSTOM_CAPABILITY_ID_RE.test(id)) {
      c.err("inputCapabilities may only declare input.* or custom ids");
    }
  }

  if (c.errors.length > 0 || !entrypoint || !displayName || !adapterKind) {
    return { ok: false, errors: c.errors.length > 0 ? c.errors : ["manifest invalid"] };
  }

  // 未知頂層欄位保留（不含以 __ 開頭者）。
  const preserved: Record<string, unknown> = {};
  for (const key of Object.keys(m)) {
    if (!RESERVED_TOP_LEVEL.has(key) && !key.startsWith("__") && key !== "constructor" && key !== "prototype") {
      preserved[key] = m[key];
    }
  }

  const manifest: CharacterManifest = {
    ...(preserved as Partial<CharacterManifest>),
    schemaVersion: m.schemaVersion as string,
    characterId: m.characterId as string,
    displayName,
    ...(typeof m.author === "string" ? { author: m.author } : {}),
    ...(description && Object.keys(description).length > 0 ? { description } : {}),
    version: m.version as string,
    adapterKind,
    entrypoint,
    assets,
    capabilities: caps.map,
    inputCapabilities: inputCaps.map,
    channels,
    states,
    intents,
    variants,
    locales: locales.length > 0 ? locales : Object.keys(displayName),
    ...(pronouns && Object.keys(pronouns).length > 0 ? { pronouns } : {}),
    ...(preferencesSchema ? { preferencesSchema } : {}),
    securityRequirements,
    resourceLimits,
    fallbacks,
    compatibility,
  };

  return {
    ok: true,
    manifest,
    report: {
      newerMinor,
      unknownCapabilities: [...caps.unknown, ...inputCaps.unknown],
      customCapabilities: [...caps.custom, ...inputCaps.custom],
      warnings: c.warnings,
      flags: {
        external: adapterKind !== "in-process",
        network: securityRequirements.network,
        executable: securityRequirements.executable,
        unsigned: true,
      },
    },
  };
}

// ---------------------------------------------------------------------------
// §2.2 Migration：PackMigrator registry
//
// v0.6.0 strangler（docs/aip/architecture-boundaries.md §4）：協定核心只認識**通用**
// sprite pack（character-pack 1.0／1.1）。任何具名角色的舊格式（例如某個 rig）由它自己的
// adapter 模組實作 PackMigrator，再由 host 註冊（character/adapterRegistry.ts 的
// registerHostMigrator）。鏡射 Rust 的 interaction_character::{PackMigrator, MigrationRegistry}
// （docs/character-protocol/README.md §2.2）。
// ---------------------------------------------------------------------------

export type MigrationResult =
  | { ok: true; manifest: CharacterManifest; source: string; assetBase?: string }
  | { ok: false; errors: string[] };

/** migrator 拿得到的 host 脈絡：純資料，沒有 I/O、沒有權限、沒有時間來源。 */
export interface MigrationContext {
  /** 同源資產根（host 已解析）；原樣帶進 MigrationResult。 */
  readonly assetBase?: string;
}

/**
 * 一種舊 pack 格式的遷移器。純函式：不讀檔、不執行 entrypoint、不下載、不改寫使用者設定。
 * 錯誤一律走 `MigrationResult.ok = false`，訊息不回顯輸入內容。
 */
export interface PackMigrator {
  /** 負責的舊 pack `kind`（例如 `character-pack`）。 */
  readonly kind: string;
  /** 支援的 `schemaVersion`（1..=MAX_MIGRATOR_VERSIONS 個）。 */
  readonly schemaVersions: readonly string[];
  migrate(legacy: Record<string, unknown>, ctx: MigrationContext): MigrationResult;
}

/** registry 有界：host 不可能有無限多種舊格式。 */
export const MAX_MIGRATORS = 32;
/** 單一 migrator 能宣告的 schemaVersion 數上限。 */
export const MAX_MIGRATOR_VERSIONS = 8;

/** 只有「看起來就是識別字」的 kind／schemaVersion 才會出現在錯誤訊息裡（不回顯任意輸入）。 */
const SAFE_IDENTIFIER_RE = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;

function identifierEcho(value: unknown): string {
  return typeof value === "string" && SAFE_IDENTIFIER_RE.test(value) ? value : "(unreadable)";
}

/**
 * 依 (kind, schemaVersion) 分派的遷移器登錄表。**有界**、不允許重複註冊同一組
 * (kind, version)：後註冊者不得悄悄覆蓋前者（否則第三方角色可以劫持別人的舊格式）。
 */
export class MigrationRegistry {
  private readonly migrators: PackMigrator[] = [];

  /** 註冊一個 migrator；超過上限、版本數不合法或 (kind, version) 重複一律擲例外。 */
  register(migrator: PackMigrator): this {
    if (typeof migrator.kind !== "string" || migrator.kind.length === 0) {
      throw new Error("a pack migrator must declare a kind");
    }
    const versions = migrator.schemaVersions;
    if (!versions || versions.length === 0 || versions.length > MAX_MIGRATOR_VERSIONS) {
      throw new Error(`a pack migrator must declare 1..=${MAX_MIGRATOR_VERSIONS} schema versions`);
    }
    if (this.migrators.length >= MAX_MIGRATORS) {
      throw new Error(`migration registry is full (max ${MAX_MIGRATORS})`);
    }
    for (const version of versions) {
      if (this.find(migrator.kind, version)) {
        throw new Error(`a migrator for '${identifierEcho(migrator.kind)}' ${identifierEcho(version)} is already registered`);
      }
    }
    this.migrators.push(migrator);
    return this;
  }

  /** 依 (kind, schemaVersion) 找 migrator；找不到就是 null（不猜、不退而求其次）。 */
  find(kind: unknown, schemaVersion: unknown): PackMigrator | null {
    if (typeof kind !== "string" || typeof schemaVersion !== "string") return null;
    return this.migrators.find((m) => m.kind === kind && m.schemaVersions.includes(schemaVersion)) ?? null;
  }

  /** 已註冊的 migrator 數量。 */
  get size(): number {
    return this.migrators.length;
  }

  /** 支援的 (kind, schemaVersion)，依註冊順序。 */
  supportedKinds(): { kind: string; schemaVersion: string }[] {
    return this.migrators.flatMap((m) => m.schemaVersions.map((schemaVersion) => ({ kind: m.kind, schemaVersion })));
  }
}

/** 核心內建的 registry：只有通用 sprite，沒有任何具名角色。 */
export function coreMigrationRegistry(): MigrationRegistry {
  return new MigrationRegistry().register(spritePackMigrator);
}

let injectedDefaultRegistry: MigrationRegistry | null = null;

/**
 * host 注入預設 registry（`character/adapters/index.ts` 在模組載入時呼叫一次）。
 * 呼叫端能明確帶 `opts.registry` 時一律明確帶；這條路只服務拿不到 host registry 的呼叫端。
 */
export function setDefaultMigrationRegistry(registry: MigrationRegistry | null): void {
  injectedDefaultRegistry = registry;
}

/** 沒帶 `opts.registry` 時用的 registry：host 注入過就用它，否則只有核心的 sprite。 */
export function defaultMigrationRegistry(): MigrationRegistry {
  return injectedDefaultRegistry ?? coreMigrationRegistry();
}

/**
 * migrator 的收尾：候選 manifest 以 validateCharacterManifest 二次驗證後才算數。
 * 外部（角色 crate／adapter 模組）的 migrator 也用這一條，驗證規則不會分叉。
 */
export function finalizeMigratedManifest(
  candidate: Record<string, unknown>,
  source: string,
  ctx: MigrationContext = {}
): MigrationResult {
  const validated = validateCharacterManifest(candidate);
  if (!validated.ok) return { ok: false, errors: validated.errors.map((e) => `migrated manifest: ${e}`) };
  return { ok: true, manifest: validated.manifest, source, ...(ctx.assetBase ? { assetBase: ctx.assetBase } : {}) };
}

const SPRITE_INPUT_CAPABILITIES = [
  "input.click",
  "input.drag",
  "input.drop",
  "input.text",
  "input.fileDrop",
] as const;

/** `CapabilityDecl` 的 supported 捷徑；外部 migrator 也用得到（避免各自長出不同預設值）。 */
export function supportedCapability(extra: Partial<CapabilityDecl> = {}): CapabilityDecl {
  return { supported: true, ...extra };
}

/** 舊 pack 的 version 欄位 → 合法 semver（外部 migrator 共用同一條規則）。 */
export function normalizeLegacyVersion(v: unknown): string {
  return typeof v === "string" && SEMVER_RE.test(v) ? v : "0.0.0";
}

/**
 * 通用 sprite pack（`character-pack` 1.0／1.1）遷移器。與任何具名角色無關：
 * 能力只來自 sheet 真的有的動畫，安全 intent 的舊 fallback 鏈一律丟掉（§3.4）。
 */
export const spritePackMigrator: PackMigrator = {
  kind: "character-pack",
  schemaVersions: ["1.0", "1.1"],
  migrate(legacy, ctx) {
    const issues = validateLegacyPack(legacy);
    if (issues.length > 0) return { ok: false, errors: issues.map((i) => `legacy pack: ${i}`) };
    const pack = legacy as unknown as PackManifest;
    const animations = Object.keys(pack.animations);
    const hasAnchors = Array.isArray(pack.anchors?.idle) && pack.anchors!.idle!.length > 0;
    const maxLoopMs = Math.max(
      ...Object.values(pack.animations).map((a) => Math.round((a.frames.length / Math.max(a.fps, 1)) * 1000)),
      1
    );
    const capabilities: Record<string, CapabilityDecl> = {
      "visual.presence": supportedCapability({ interruptible: true, resumable: true, reducedMotionBehavior: "static", qualityLevel: "full" }),
      "visual.expression": supportedCapability({
        interruptible: true,
        resumable: true,
        maxConcurrent: 1,
        variants: animations,
        durationRange: { minMs: 0, maxMs: Math.min(LIMITS.durationMaxMs, Math.max(maxLoopMs, 1000)) },
        reducedMotionBehavior: "static",
        qualityLevel: "full",
      }),
    };
    if (hasAnchors) {
      capabilities["visual.gaze"] = supportedCapability({
        interruptible: true,
        resumable: true,
        reducedMotionBehavior: "disabled",
        qualityLevel: "reduced",
      });
    }
    const inputCapabilities: Record<string, CapabilityDecl> = {};
    for (const id of SPRITE_INPUT_CAPABILITIES) inputCapabilities[id] = supportedCapability();
    const assets: AssetDecl[] = [{ id: "sheet", path: pack.sheet, mediaType: "image/png" }];
    if (typeof pack.preview === "string" && !assetPathIssue(pack.preview)) {
      assets.push({ id: "preview", path: pack.preview, mediaType: "image/png" });
    }
    const candidate: Record<string, unknown> = {
      schemaVersion: "1.0",
      characterId: pack.id,
      displayName: pack.name,
      ...(pack.author ? { author: pack.author } : {}),
      ...(pack.description ? { description: pack.description } : {}),
      version: normalizeLegacyVersion(pack.version),
      adapterKind: "in-process",
      entrypoint: { kind: "builtin", id: "sprite" },
      assets,
      capabilities,
      inputCapabilities,
      channels: ["transform", "expression", ...(hasAnchors ? ["gaze"] : [])],
      states: animations,
      intents: nativeIntentsOf(pack.animations),
      variants: [],
      locales: Object.keys(pack.name),
      securityRequirements: {
        network: false,
        executable: false,
        fileAccess: "none",
        audioOutput: false,
        microphone: false,
        camera: false,
      },
      resourceLimits: { maxAssetBytes: 8 * 1024 * 1024, maxConcurrentCommands: 1, maxQueue: 32, maxFps: 30 },
      fallbacks: {
        capabilities: { "visual.expression": ["visual.presence"] },
        // 舊 renderer 的 emergency → paused 這類鏈會改寫安全語意：遷移時丟掉，
        // 讓那些安全 intent 改走能力鏈／system.text。
        intents: safeIntentFallbacks(deriveIntentFallbacks(pack.animations)),
      },
      compatibility: { protocol: "1.x", runtime: ">=0.5.0" },
      legacy: { kind: "character-pack", schemaVersion: pack.schemaVersion },
    };
    return finalizeMigratedManifest(candidate, "character-pack", ctx);
  },
};

/**
 * 舊 Character Pack → CharacterManifest（§2.2）。依 registry 註冊的 (kind, schemaVersion)
 * 分派；沒有 migrator 的格式一律誠實拒絕（不猜、不執行），不改寫使用者設定。
 */
export function migratePackToManifest(
  legacy: unknown,
  opts: { assetBase?: string; registry?: MigrationRegistry } = {}
): MigrationResult {
  if (!isPlainObject(legacy)) return { ok: false, errors: ["legacy pack must be an object"] };
  const registry = opts.registry ?? defaultMigrationRegistry();
  const migrator = registry.find(legacy.kind, legacy.schemaVersion);
  if (!migrator) {
    return {
      ok: false,
      errors: [
        `no migrator is registered for legacy pack kind ${identifierEcho(legacy.kind)} schemaVersion ${identifierEcho(legacy.schemaVersion)}`,
      ],
    };
  }
  return migrator.migrate(legacy, opts.assetBase !== undefined ? { assetBase: opts.assetBase } : {});
}

/** 資產 URL：assetBase（同源、host 提供）＋相對路徑。路徑已由驗證器保證相對且無穿越。 */
export function resolveAssetUrl(assetBase: string, asset: AssetDecl): string {
  const base = assetBase.endsWith("/") ? assetBase.slice(0, -1) : assetBase;
  return `${base}/${asset.path}`;
}

// ---------------------------------------------------------------------------
// 顯示 helper
// ---------------------------------------------------------------------------

function pickLocalized(text: LocalizedText | undefined, locale: string): string | null {
  if (!text) return null;
  if (text[locale]) return text[locale];
  const lang = locale.split("-")[0].toLowerCase();
  for (const [k, v] of Object.entries(text)) {
    if (k.toLowerCase() === lang || k.toLowerCase().startsWith(`${lang}-`)) return v;
  }
  if (text.en) return text.en;
  const first = Object.values(text)[0];
  return first ?? null;
}

/** 顯示名稱；沒有任何可用值時回中立文案「角色」。 */
export function displayNameOf(manifest: Pick<CharacterManifest, "displayName">, locale: string): string {
  return pickLocalized(manifest.displayName, locale) ?? "角色";
}

/** 代名詞；manifest 沒宣告時用中立文案（zh → 「角色」、其他 → "they"）。 */
export function pronounOf(manifest: Pick<CharacterManifest, "pronouns">, locale: string): string {
  const explicit = pickLocalizedStrict(manifest.pronouns, locale);
  if (explicit) return explicit;
  return locale.toLowerCase().startsWith("zh") ? "角色" : "they";
}

/** 代名詞不跨語言 fallback（英文的 she 不能拿去當中文代名詞）。 */
function pickLocalizedStrict(text: LocalizedText | undefined, locale: string): string | null {
  if (!text) return null;
  if (text[locale]) return text[locale];
  const lang = locale.split("-")[0].toLowerCase();
  for (const [k, v] of Object.entries(text)) {
    if (k.toLowerCase() === lang || k.toLowerCase().startsWith(`${lang}-`)) return v;
  }
  return null;
}
