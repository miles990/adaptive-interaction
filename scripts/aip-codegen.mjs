#!/usr/bin/env node
// AIP 1.0 跨語言型別產生器。
//
// 單一來源是 `schemas/aip-1.0.schema.json`（由 `crates/interaction-aip` 的 Rust 型別經
// `cargo test -p interaction-e2e --test golden` 產生）。這支腳本把同一份 schema 投影成
// TypeScript 與 Swift 型別，再把 conformance fixtures 內嵌成 Swift 字串（XCTest 讀不到 repo 檔）。
//
// 規則：
// - 純 Node，不依賴任何第三方套件。
// - 輸出必須是確定性的（同一份輸入 → 逐位元組相同的輸出），才能用 `--check` 當 CI drift gate。
// - 產生的檔案一律帶「GENERATED …do not edit」檔頭；手改會被 `--check` 擋下。
//
// 用法：
//   node scripts/aip-codegen.mjs           寫檔
//   node scripts/aip-codegen.mjs --check   只比對，漂移就 exit 1

import { readFileSync, readdirSync, writeFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SCHEMA_PATH = join(REPO_ROOT, "schemas/aip-1.0.schema.json");
const FIXTURES_DIR = join(REPO_ROOT, "crates/interaction-aip/tests/fixtures");
const TS_OUT = join(REPO_ROOT, "apps/interaction-desktop/src/aip/generated.ts");
const SWIFT_OUT = join(
  REPO_ROOT,
  "apps/interaction-ios/InteractionCompanion/Models/AIPGenerated.swift",
);
const SWIFT_FIXTURES_OUT = join(
  REPO_ROOT,
  "apps/interaction-ios/InteractionCompanionTests/AIPFixtures.swift",
);

/** Rust 端帶 `Unknown(String)` 變體的 enum：未知值必須能被表達，但永不執行（AIP §4.1）。 */
const OPEN_ENUMS = ["ErrorCode", "MessageType", "PartyKind"];
/** 已經有專屬頂層常數（AIP_MESSAGE_TYPES／AIP_ERROR_CODES）的開放 enum。 */
const DEDICATED_CONSTANT_ENUMS = ["MessageType", "ErrorCode"];
/** 需要保留未知頂層欄位的型別（AIP §1）。 */
const EXTRA_BEARING = ["Envelope"];
/** 產生順序固定，才有確定性輸出。 */
const STRUCT_ORDER = [
  "Party",
  "Envelope",
  "ErrorPayload",
  "CapabilityLimits",
  "CapabilityAnnouncement",
  "NegotiatedCapabilities",
];
const ENUM_ORDER = [
  "MessageType",
  "PartyKind",
  "ErrorCode",
  "Outcome",
  "OfflinePolicy",
  "EvidenceClass",
  "MemberRole",
  "SyncClass",
  "IntentSupport",
];

// ---------------------------------------------------------------- schema 讀取

/** 把 `$defs` 分類成 enum（開放／封閉）與 struct，並在分類不明時直接失敗（不猜）。 */
function classify(defs) {
  const enums = new Map();
  const structs = new Map();
  for (const [name, def] of Object.entries(defs)) {
    if (Array.isArray(def.enum)) {
      if (OPEN_ENUMS.includes(name)) {
        throw new Error(
          `${name} is listed as an open enum but the schema emits a closed \`enum\`; update OPEN_ENUMS`,
        );
      }
      enums.set(name, { values: [...def.enum], open: false, description: def.description ?? "" });
      continue;
    }
    if (Array.isArray(def.anyOf) && def.anyOf.every((b) => b.const !== undefined || Array.isArray(b.enum))) {
      if (!OPEN_ENUMS.includes(name)) {
        throw new Error(
          `${name} looks like an open enum but is not in OPEN_ENUMS; codegen refuses to guess`,
        );
      }
      const values = [];
      for (const branch of def.anyOf) {
        if (branch.const !== undefined) values.push(branch.const);
        else values.push(...branch.enum);
      }
      enums.set(name, { values, open: true, description: def.description ?? "" });
      continue;
    }
    if (def.type === "object" && def.properties) {
      structs.set(name, def);
      continue;
    }
    throw new Error(`cannot classify $defs/${name}; codegen refuses to guess`);
  }
  for (const name of [...ENUM_ORDER, ...STRUCT_ORDER]) {
    if (!enums.has(name) && !structs.has(name)) {
      throw new Error(`the generator expects $defs/${name} but the schema does not define it`);
    }
  }
  for (const name of enums.keys()) {
    if (!ENUM_ORDER.includes(name)) throw new Error(`unlisted enum ${name}; add it to ENUM_ORDER`);
  }
  for (const name of structs.keys()) {
    if (!STRUCT_ORDER.includes(name)) {
      throw new Error(`unlisted struct ${name}; add it to STRUCT_ORDER`);
    }
  }
  return { enums, structs };
}

/** 欄位順序：required 先（照 wire 順序），其餘按字典序。確定性且可讀。 */
function orderedFields(def) {
  const required = def.required ?? [];
  const rest = Object.keys(def.properties)
    .filter((k) => !required.includes(k))
    .sort();
  return [...required, ...rest].map((name) => ({
    name,
    schema: def.properties[name],
    required: required.includes(name),
  }));
}

function refName(schema) {
  if (typeof schema.$ref !== "string") return null;
  return schema.$ref.replace("#/$defs/", "");
}

/** anyOf 只有 `[X, null]` 一種形狀時，回傳 X；否則 null。 */
function nullableRef(schema) {
  if (!Array.isArray(schema.anyOf) || schema.anyOf.length !== 2) return null;
  const [a, b] = schema.anyOf;
  if (b.type !== "null") return null;
  return refName(a);
}

function typeSet(schema) {
  const t = schema.type;
  if (t === undefined) return [];
  return Array.isArray(t) ? t : [t];
}

// ------------------------------------------------------------- TypeScript 投影

function tsType(schema) {
  const ref = refName(schema);
  if (ref) return ref;
  const nullable = nullableRef(schema);
  if (nullable) return `${nullable} | null`;
  const types = typeSet(schema).filter((t) => t !== "null");
  const isNullable = typeSet(schema).includes("null");
  let base;
  if (types.length === 0) {
    base = "AipJsonValue";
  } else if (types[0] === "string") {
    base = "string";
  } else if (types[0] === "integer" || types[0] === "number") {
    base = "number";
  } else if (types[0] === "boolean") {
    base = "boolean";
  } else if (types[0] === "array") {
    base = `${tsType(schema.items ?? {})}[]`;
  } else if (types[0] === "object") {
    const additional = schema.additionalProperties;
    if (additional && typeof additional === "object" && additional.$ref) {
      base = `Record<string, ${refName(additional)}>`;
    } else {
      base = "Record<string, AipJsonValue>";
    }
  } else {
    throw new Error(`unsupported schema type ${JSON.stringify(schema.type)}`);
  }
  return isNullable ? `${base} | null` : base;
}

function tsDoc(text, indent = "") {
  if (!text) return "";
  const lines = String(text).split("\n");
  return `${indent}/**\n${lines.map((l) => `${indent} * ${l}`).join("\n")}\n${indent} */\n`;
}

function generateTypeScript(schema, { enums, structs }) {
  const doublePaths = readStateHashDoublePaths();
  const out = [];
  out.push("// GENERATED by scripts/aip-codegen.mjs — do not edit.");
  out.push("// Source of truth: schemas/aip-1.0.schema.json (generated from crates/interaction-aip).");
  out.push("// SemanticState double paths come from crates/interaction-aip/tests/fixtures/manifest.json.");
  out.push("// Regenerate with `pnpm aip:codegen`; CI verifies with `pnpm aip:check`.");
  out.push("//");
  out.push("// 契約：docs/aip/README.md。行為（驗證、去重、協商、離線政策）在 ./envelope.ts，不在這裡。");
  out.push("");
  out.push("/** 任意 JSON 值。未知欄位一律保留成這個型別，不猜、不丟。 */");
  out.push("export type AipJsonValue =");
  out.push("  | null");
  out.push("  | boolean");
  out.push("  | number");
  out.push("  | string");
  out.push("  | AipJsonValue[]");
  out.push("  | { [key: string]: AipJsonValue };");
  out.push("");

  for (const name of ENUM_ORDER) {
    const def = enums.get(name);
    out.push(tsDoc(def.description).trimEnd());
    const union = def.values.map((v) => JSON.stringify(v)).join("\n  | ");
    if (def.open) {
      out.push(`export type Known${name} =\n  | ${union};`);
      out.push("");
      out.push(
        `/** 未知值保留原字串供稽核，但永不執行（AIP §4.1）。 */\nexport type ${name} = Known${name} | (string & {});`,
      );
      // MessageType／ErrorCode 已經有 AIP_MESSAGE_TYPES／AIP_ERROR_CODES 這兩個專屬常數，
      // 不重複產生第二份清單（同源，但兩份名字會讓讀者不知道該用哪個）。
      if (!DEDICATED_CONSTANT_ENUMS.includes(name)) {
        out.push("");
        out.push(
          `export const AIP_KNOWN_${screamingSnake(name)}S: readonly Known${name}[] = [\n${def.values
            .map((v) => `  ${JSON.stringify(v)},`)
            .join("\n")}\n];`,
        );
      }
    } else {
      out.push(`export type ${name} =\n  | ${union};`);
      out.push("");
      out.push(
        `export const AIP_${screamingSnake(name)}S: readonly ${name}[] = [\n${def.values
          .map((v) => `  ${JSON.stringify(v)},`)
          .join("\n")}\n];`,
      );
    }
    out.push("");
  }

  for (const name of STRUCT_ORDER) {
    const def = structs.get(name);
    out.push(tsDoc(def.description).trimEnd());
    out.push(`export interface ${name} {`);
    for (const field of orderedFields(def)) {
      const doc = tsDoc(field.schema.description, "  ");
      if (doc) out.push(doc.trimEnd());
      out.push(`  ${field.name}${field.required ? "" : "?"}: ${tsType(field.schema)};`);
    }
    if (EXTRA_BEARING.includes(name)) {
      out.push("  /** 未知的頂層選填欄位：保留、忽略，round-trip 不遺失（AIP §1）。 */");
      out.push("  [key: string]: unknown;");
    }
    out.push("}");
    out.push("");
  }

  out.push("/** 本實作宣告的 spec 版本。major 不同一律拒絕（AIP §4.1）。 */");
  out.push(`export const AIP_SPEC_VERSION = ${JSON.stringify(schema.specVersion)} as const;`);
  out.push("");
  out.push("/** 十二種 message type（AIP §2.1）。 */");
  out.push(
    `export const AIP_MESSAGE_TYPES: readonly KnownMessageType[] = [\n${schema.messageTypes
      .map((v) => `  ${JSON.stringify(v)},`)
      .join("\n")}\n];`,
  );
  out.push("");
  out.push("/** 穩定錯誤碼（AIP §12）。 */");
  out.push(
    `export const AIP_ERROR_CODES: readonly KnownErrorCode[] = [\n${schema.errorCodes
      .map((v) => `  ${JSON.stringify(v)},`)
      .join("\n")}\n];`,
  );
  out.push("");
  out.push("/**");
  out.push(" * `SemanticState` 裡以 f64 序列化的欄位（RFC 6901 pointer）。");
  out.push(" *");
  out.push(" * canonical JSON 對 f64 一律寫成帶小數點的最短 round-trip 十進位（`0.0`、`1.0`、");
  out.push(" * `-0.0`），對整數型別則是純數字。JS 的 `number` 留不住這個區別（`JSON.parse`");
  out.push(" * 之後 Rust 的 `0.0` 就只是 `0`），所以重印 canonical 文字時得靠這份路徑清單。");
  out.push(" * 來源是跨語言 fixture manifest 的 `stateHashDoublePaths`——Rust 端每次測試從");
  out.push(" * `SemanticState` 的 schema 重新推導它，新增 f64 欄位卻沒重跑 codegen 會被");
  out.push(" * `pnpm aip:check` 擋下。");
  out.push(" */");
  out.push(
    `export const SEMANTIC_STATE_DOUBLE_PATHS: readonly string[] = [\n${doublePaths
      .map((v) => `  ${JSON.stringify(v)},`)
      .join("\n")}\n];`,
  );
  out.push("");
  out.push("/** 上限常數（AIP §11）。所有集合、訊息、字串都有界。 */");
  out.push("export const AIP_LIMITS = {");
  for (const key of Object.keys(schema.limits).sort()) {
    out.push(`  ${key}: ${schema.limits[key]},`);
  }
  out.push("} as const;");
  out.push("");
  return out.join("\n");
}

/**
 * 跨語言 fixture manifest 的 `stateHashDoublePaths`（`SemanticState` 的 f64 欄位）。
 *
 * Rust 端在測試裡從 `SemanticState` 的 schema 重新推導這份清單，所以它是漂移 gate：
 * 少一條、多一條、順序不同都會讓產生出來的 TypeScript 不同，`--check` 就會失敗。
 * 讀不到或形狀不對一律直接失敗——codegen 不猜。
 */
function readStateHashDoublePaths() {
  const path = join(FIXTURES_DIR, "manifest.json");
  const manifest = JSON.parse(readFileSync(path, "utf8"));
  const paths = manifest.stateHashDoublePaths;
  if (!Array.isArray(paths) || paths.some((p) => typeof p !== "string")) {
    throw new Error(
      `crates/interaction-aip/tests/fixtures/manifest.json is missing a string[] \`stateHashDoublePaths\``,
    );
  }
  return paths;
}

function screamingSnake(name) {
  return name.replace(/([a-z0-9])([A-Z])/g, "$1_$2").toUpperCase();
}

// ------------------------------------------------------------------ Swift 投影

function swiftCaseName(value) {
  const parts = String(value).split(/[-_.]/);
  const head = parts[0];
  const rest = parts.slice(1).map((p) => p.charAt(0).toUpperCase() + p.slice(1));
  const name = [head, ...rest].join("");
  const reserved = new Set(["default", "internal", "operator", "repeat", "return", "case", "error"]);
  return reserved.has(name) ? `\`${name}\`` : name;
}

function swiftType(schema) {
  const ref = refName(schema);
  if (ref) return ref === "Envelope" ? "AIPEnvelope" : `AIP${ref}`;
  const nullable = nullableRef(schema);
  if (nullable) return `${nullable === "Envelope" ? "AIPEnvelope" : `AIP${nullable}`}?`;
  const types = typeSet(schema).filter((t) => t !== "null");
  const isNullable = typeSet(schema).includes("null");
  let base;
  if (types.length === 0) {
    base = "JSONValue";
  } else if (types[0] === "string") {
    base = "String";
  } else if (types[0] === "integer") {
    base = schema.format === "uint64" ? "UInt64" : "Int";
  } else if (types[0] === "number") {
    base = "Double";
  } else if (types[0] === "boolean") {
    base = "Bool";
  } else if (types[0] === "array") {
    base = `[${swiftType(schema.items ?? {})}]`;
  } else if (types[0] === "object") {
    const additional = schema.additionalProperties;
    if (additional && typeof additional === "object" && additional.$ref) {
      base = `[String: AIP${refName(additional)}]`;
    } else {
      base = "[String: JSONValue]";
    }
  } else {
    throw new Error(`unsupported schema type ${JSON.stringify(schema.type)}`);
  }
  return isNullable ? `${base}?` : base;
}

function swiftDoc(text, indent = "") {
  if (!text) return [];
  return String(text)
    .split("\n")
    .map((line) => `${indent}/// ${line}`);
}

function generateSwift(schema, { enums, structs }) {
  const out = [];
  out.push("// GENERATED by scripts/aip-codegen.mjs — do not edit.");
  out.push("// Source of truth: schemas/aip-1.0.schema.json (generated from crates/interaction-aip).");
  out.push("// Regenerate with `pnpm aip:codegen` from apps/interaction-desktop.");
  out.push("//");
  out.push("// 契約：docs/aip/README.md。驗證行為在 AIPEnvelope.swift，不在這裡。");
  out.push("// 未知的 enum 值一律進 `.unknown(String)`：iPhone 端不得因為桌面版本較新而崩潰或誤判成功。");
  out.push("");
  out.push("import Foundation");
  out.push("");
  out.push("// MARK: - 任意鍵（保留未知頂層欄位用）");
  out.push("");
  out.push("struct AIPAnyCodingKey: CodingKey {");
  out.push("    var stringValue: String");
  out.push("    var intValue: Int? { nil }");
  out.push("    init(stringValue: String) { self.stringValue = stringValue }");
  out.push("    init?(intValue: Int) { nil }");
  out.push("}");
  out.push("");

  for (const name of ENUM_ORDER) {
    const def = enums.get(name);
    out.push(...swiftDoc(def.description));
    if (def.open) {
      out.push(`enum AIP${name}: Codable, Equatable, Hashable {`);
      for (const value of def.values) out.push(`    case ${swiftCaseName(value)}`);
      out.push("    /// 本版不認得的值：保留原字串供稽核，永不執行。");
      out.push("    case unknown(String)");
      out.push("");
      out.push(`    static let known: [AIP${name}] = [`);
      for (const value of def.values) out.push(`        .${swiftCaseName(value)},`);
      out.push("    ]");
      out.push("");
      out.push("    var rawValue: String {");
      out.push("        switch self {");
      for (const value of def.values) {
        out.push(`        case .${swiftCaseName(value)}: return ${JSON.stringify(value)}`);
      }
      out.push("        case .unknown(let raw): return raw");
      out.push("        }");
      out.push("    }");
      out.push("");
      out.push("    var isKnown: Bool {");
      out.push("        if case .unknown = self { return false }");
      out.push("        return true");
      out.push("    }");
      out.push("");
      out.push("    init(rawValue: String) {");
      out.push("        switch rawValue {");
      for (const value of def.values) {
        out.push(`        case ${JSON.stringify(value)}: self = .${swiftCaseName(value)}`);
      }
      out.push("        default: self = .unknown(rawValue)");
      out.push("        }");
      out.push("    }");
      out.push("");
      out.push("    init(from decoder: Decoder) throws {");
      out.push("        let container = try decoder.singleValueContainer()");
      out.push("        self.init(rawValue: try container.decode(String.self))");
      out.push("    }");
      out.push("");
      out.push("    func encode(to encoder: Encoder) throws {");
      out.push("        var container = encoder.singleValueContainer()");
      out.push("        try container.encode(rawValue)");
      out.push("    }");
      out.push("}");
    } else {
      out.push(`enum AIP${name}: String, Codable, Equatable, Hashable, CaseIterable {`);
      for (const value of def.values) {
        out.push(`    case ${swiftCaseName(value)} = ${JSON.stringify(value)}`);
      }
      out.push("}");
    }
    out.push("");
  }

  for (const name of STRUCT_ORDER) {
    const def = structs.get(name);
    const swiftName = name === "Envelope" ? "AIPEnvelope" : `AIP${name}`;
    const fields = orderedFields(def);
    const carriesExtra = EXTRA_BEARING.includes(name);
    out.push(...swiftDoc(def.description));
    out.push(`struct ${swiftName}: Codable, Equatable {`);
    for (const field of fields) {
      out.push(...swiftDoc(field.schema.description, "    "));
      const type = swiftType(field.schema);
      const optional = field.required ? type : type.endsWith("?") ? type : `${type}?`;
      out.push(`    var ${field.name}: ${optional}`);
    }
    if (carriesExtra) {
      out.push("    /// 未知的頂層選填欄位：保留、忽略，round-trip 不遺失（AIP §1）。");
      out.push("    var extra: [String: JSONValue] = [:]");
    }
    out.push("");
    out.push(
      carriesExtra
        ? "    private enum Keys: String, CodingKey, CaseIterable {"
        : "    private enum Keys: String, CodingKey {",
    );
    for (const field of fields) out.push(`        case ${field.name}`);
    out.push("    }");
    out.push("");
    out.push("    init(from decoder: Decoder) throws {");
    out.push("        let container = try decoder.container(keyedBy: Keys.self)");
    for (const field of fields) {
      const type = swiftType(field.schema).replace(/\?$/, "");
      if (field.required) {
        out.push(`        self.${field.name} = try container.decode(${type}.self, forKey: .${field.name})`);
      } else {
        out.push(
          `        self.${field.name} = try container.decodeIfPresent(${type}.self, forKey: .${field.name})`,
        );
      }
    }
    if (carriesExtra) {
      out.push("        let known = Set(Keys.allCases.map(\\.rawValue))");
      out.push("        let any = try decoder.container(keyedBy: AIPAnyCodingKey.self)");
      out.push("        var rest: [String: JSONValue] = [:]");
      out.push("        for key in any.allKeys where !known.contains(key.stringValue) {");
      out.push("            rest[key.stringValue] = try any.decode(JSONValue.self, forKey: key)");
      out.push("        }");
      out.push("        self.extra = rest");
    }
    out.push("    }");
    out.push("");
    out.push("    func encode(to encoder: Encoder) throws {");
    out.push("        var container = encoder.container(keyedBy: Keys.self)");
    for (const field of fields) {
      if (field.required) {
        out.push(`        try container.encode(${field.name}, forKey: .${field.name})`);
      } else {
        out.push(`        try container.encodeIfPresent(${field.name}, forKey: .${field.name})`);
      }
    }
    if (carriesExtra) {
      out.push("        var any = encoder.container(keyedBy: AIPAnyCodingKey.self)");
      out.push("        for key in extra.keys.sorted() {");
      out.push("            guard let value = extra[key] else { continue }");
      out.push("            try any.encode(value, forKey: AIPAnyCodingKey(stringValue: key))");
      out.push("        }");
    }
    out.push("    }");
    out.push("");
    out.push(`    init(`);
    const params = fields.map((field) => {
      const type = swiftType(field.schema);
      const optional = field.required ? type : type.endsWith("?") ? type : `${type}?`;
      return `        ${field.name}: ${optional}${field.required ? "" : " = nil"}`;
    });
    if (carriesExtra) params.push("        extra: [String: JSONValue] = [:]");
    out.push(params.join(",\n"));
    out.push("    ) {");
    for (const field of fields) out.push(`        self.${field.name} = ${field.name}`);
    if (carriesExtra) out.push("        self.extra = extra");
    out.push("    }");
    out.push("}");
    out.push("");
  }

  out.push("// MARK: - 常數（AIP §11／§12）");
  out.push("");
  out.push("enum AIPConstants {");
  out.push("    /// 本實作宣告的 spec 版本。major 不同一律拒絕。");
  out.push(`    static let specVersion = ${JSON.stringify(schema.specVersion)}`);
  out.push("    /// 十二種 message type（AIP §2.1）。");
  out.push(
    `    static let messageTypes: [String] = [\n${schema.messageTypes
      .map((v) => `        ${JSON.stringify(v)},`)
      .join("\n")}\n    ]`,
  );
  out.push("    /// 穩定錯誤碼（AIP §12）。");
  out.push(
    `    static let errorCodes: [String] = [\n${schema.errorCodes
      .map((v) => `        ${JSON.stringify(v)},`)
      .join("\n")}\n    ]`,
  );
  out.push("}");
  out.push("");
  out.push("/// 上限常數（AIP §11）。所有集合、訊息、字串都有界。");
  out.push("enum AIPLimits {");
  for (const key of Object.keys(schema.limits).sort()) {
    out.push(`    static let ${key} = ${schema.limits[key]}`);
  }
  out.push("}");
  out.push("");
  return out.join("\n");
}

// ------------------------------------------------- Swift fixtures（內嵌成字串）

/** Swift raw string literal。fixtures 內沒有 `"""#` 序列，用 `#"""` 界定最安全。 */
function swiftRawString(text) {
  if (text.includes('"""#')) {
    throw new Error("fixture content collides with the Swift raw-string delimiter");
  }
  return `#"""\n${text}\n"""#`;
}

function generateSwiftFixtures() {
  const names = readdirSync(FIXTURES_DIR)
    .filter((n) => n.endsWith(".json"))
    .sort();
  const out = [];
  out.push("// GENERATED by scripts/aip-codegen.mjs — do not edit.");
  out.push("// Source: crates/interaction-aip/tests/fixtures/ (the one conformance index all three languages read).");
  out.push("// XCTest 讀不到 repo 內的檔案，所以 fixtures 在這裡內嵌成字串。");
  out.push("//");
  out.push("// Regenerate with `pnpm aip:codegen` from apps/interaction-desktop.");
  out.push("");
  out.push("import Foundation");
  out.push("");
  out.push("enum AIPFixtures {");
  out.push("    /// 檔名 → 原始內容。`manifest.json` 是索引，其餘是 envelope fixture。");
  out.push("    static let files: [String: String] = [");
  for (const name of names) {
    const text = readFileSync(join(FIXTURES_DIR, name), "utf8").replace(/\n$/, "");
    out.push(`        ${JSON.stringify(name)}: ${swiftRawString(text)},`);
  }
  out.push("    ]");
  out.push("");
  out.push("    static var manifest: String { files[\"manifest.json\"] ?? \"\" }");
  out.push("}");
  out.push("");
  return out.join("\n");
}

// ------------------------------------------------------------------------ main

function main() {
  const check = process.argv.includes("--check");
  const schema = JSON.parse(readFileSync(SCHEMA_PATH, "utf8"));
  const classified = classify(schema.$defs);
  const artifacts = [
    [TS_OUT, generateTypeScript(schema, classified)],
    [SWIFT_OUT, generateSwift(schema, classified)],
    [SWIFT_FIXTURES_OUT, generateSwiftFixtures()],
  ];
  const drifted = [];
  for (const [path, content] of artifacts) {
    const relative = path.slice(REPO_ROOT.length + 1);
    if (check) {
      const current = existsSync(path) ? readFileSync(path, "utf8") : null;
      if (current !== content) drifted.push(relative);
      continue;
    }
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, content);
    console.log(`wrote ${relative}`);
  }
  if (check) {
    if (drifted.length > 0) {
      console.error(
      "AIP generated files drifted from schemas/aip-1.0.schema.json (or the fixture manifest):",
    );
      for (const path of drifted) console.error(`  ${path}`);
      console.error("Run `pnpm aip:codegen` (or `node scripts/aip-codegen.mjs`) and commit the result.");
      process.exit(1);
    }
    console.log("AIP generated files are in sync with schemas/aip-1.0.schema.json");
  }
}

main();
