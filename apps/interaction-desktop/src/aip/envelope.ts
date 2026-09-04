// AIP 1.0 的 TypeScript 驗證邏輯。
//
// 型別在 ./generated.ts（由 scripts/aip-codegen.mjs 從 schemas/aip-1.0.schema.json 產生）；
// **行為**在這裡，而且必須與 Rust 權威實作（crates/interaction-aip）逐條一致 —— 檢查順序、
// 錯誤碼、上限、profile 必填欄位都一樣。不一致就是漏洞：桌面端寬鬆一分，攻擊面就多一分。
// 一致性由 src/test/aip-conformance.test.ts 對同一份 fixture index 釘住。
//
// 契約：docs/aip/README.md。這個模組是純函式，沒有 I/O、沒有計時器、沒有全域狀態。

import {
  AIP_ERROR_CODES,
  AIP_KNOWN_PARTY_KINDS,
  AIP_LIMITS,
  AIP_MESSAGE_TYPES,
  AIP_OUTCOMES,
  AIP_SPEC_VERSION,
  type CapabilityAnnouncement,
  type Envelope,
  type ErrorCode,
  type IntentSupport,
  type MemberRole,
  type NegotiatedCapabilities,
  type OfflinePolicy,
  type Outcome,
  type Party,
  type SyncClass,
} from "./generated";
import { isUint64Literal, scanNumberLiterals, type NumberLiteral } from "./json-source";

// ------------------------------------------------------------------ 結果型別

/** AIP 層的處理失敗。`message` ≤ 200 字、不回顯輸入、不含路徑（AIP §5／§12）。 */
export interface AipFailure {
  code: ErrorCode;
  message: string;
  retryable: boolean;
}

export type AipOutcome<T> = { ok: true; value: T } | { ok: false; error: AipFailure };

/** 可用**同一個 messageId** 重送的錯誤（idempotent）。 */
const RETRYABLE: readonly ErrorCode[] = ["rate-limited", "internal"];

function fail(code: ErrorCode, message: string): { ok: false; error: AipFailure } {
  return {
    ok: false,
    error: {
      code,
      message: [...message].slice(0, 200).join(""),
      retryable: RETRYABLE.includes(code),
    },
  };
}

function ok<T>(value: T): { ok: true; value: T } {
  return { ok: true, value };
}

const OK_VOID = ok(undefined as void);

// -------------------------------------------------------------------- 版本

export interface NegotiatedVersion {
  specVersion: string;
  newerMinor: boolean;
}

/** u32 上限：權威 host 的 `parse_spec_version` 回 `Option<(u32, u32)>`，溢出就是看不懂。 */
const MAX_VERSION_PART = 4294967295;

/**
 * 解析 `aip/<major>.<minor>`；其他格式一律 null（不猜）。
 *
 * 與 Rust `interaction_aip::parse_spec_version` 逐字對齊：
 *   * **不** trim 前後空白——空白不是版本的一部分，而且「容忍多少空白」三個語言各有各的
 *     答案（Swift 的 `.whitespaces` 不含換行），寬鬆的那個就會接受權威端拒絕的訊息。
 *   * major／minor 溢出 u32 → null（→ `schema-invalid`）。看不懂的字串不叫「不支援的版本」，
 *     否則 `aip/5000000000.0` 在這裡是 `unsupported-version`、在 Rust 是 `schema-invalid`。
 */
function parseSpecVersion(value: string): [number, number] | null {
  if (!value.startsWith("aip/")) return null;
  const rest = value.slice("aip/".length);
  const dot = rest.indexOf(".");
  if (dot < 0) return null;
  const major = rest.slice(0, dot);
  const minor = rest.slice(dot + 1);
  if (!/^\d+$/.test(major) || !/^\d+$/.test(minor)) return null;
  const parts: [number, number] = [Number(major), Number(minor)];
  if (parts[0] > MAX_VERSION_PART || parts[1] > MAX_VERSION_PART) return null;
  return parts;
}

const LOCAL_VERSION = parseSpecVersion(AIP_SPEC_VERSION) ?? [1, 0];
const LOCAL_MAJOR = LOCAL_VERSION[0];
const LOCAL_MINOR = LOCAL_VERSION[1];

/**
 * §4.1：major 不同一律拒絕（不猜）；minor 較新時取 min 並標 `newerMinor`，
 * 未知的選填欄位靠 Envelope 的 index signature 保留。
 */
export function negotiateVersion(remote: string): AipOutcome<NegotiatedVersion> {
  const parsed = parseSpecVersion(remote);
  if (parsed === null) {
    return fail("schema-invalid", "specVersion must look like aip/<major>.<minor>");
  }
  const [major, minor] = parsed;
  if (major !== LOCAL_MAJOR) {
    return fail("unsupported-version", `unsupported major ${major}; this build speaks aip/${LOCAL_MAJOR}.x`);
  }
  return ok({
    specVersion: `aip/${major}.${Math.min(minor, LOCAL_MINOR)}`,
    newerMinor: minor > LOCAL_MINOR,
  });
}

/** 從 `capability.specVersions` 挑第一個能協商的版本。 */
export function negotiateVersions(remote: readonly string[]): AipOutcome<NegotiatedVersion> {
  let last: AipOutcome<NegotiatedVersion> | null = null;
  for (const candidate of remote) {
    const result = negotiateVersion(candidate);
    if (result.ok) return result;
    last = result;
  }
  return last ?? fail("unsupported-version", "no specVersions offered");
}

// ---------------------------------------------------------------- name 語法

/** `^[a-z][a-z0-9]*(\.[a-z][a-z0-9-]*)+$`，≤ MAX_NAME_CHARS。 */
export function isValidName(name: string): boolean {
  if (name.length === 0 || [...name].length > AIP_LIMITS.maxNameChars) return false;
  return /^[a-z][a-z0-9]*(\.[a-z][a-z0-9-]*)+$/.test(name);
}

/** 只有 Runtime 可以送的 name 前綴；device／renderer 送來一律 `scope-denied`。 */
export function isRuntimeOnlyName(name: string): boolean {
  return name.startsWith("task.") || name.startsWith("runtime.");
}

// ---------------------------------------------------------------- 解析與上限

const UTF8 = new TextEncoder();

/**
 * 解析時保留下來的數字字面值原文（AIP §1 的 round-trip 保證＋§6 的 u64 欄位判斷）。
 *
 * 掛成**非列舉**的 symbol 屬性：不會被 `JSON.stringify` 寫出去、不會被 `{...envelope}`
 * 複製走、也不會讓深度比較看到它。模組本身仍然沒有全域狀態——這份原文跟著那一個信封走。
 */
export const AIP_NUMBER_SOURCE: unique symbol = Symbol("aip.numberSource");

function numberSourceOf(envelope: Envelope): Map<string, NumberLiteral> | undefined {
  const carrier = envelope as unknown as Record<symbol, unknown>;
  const literals = carrier[AIP_NUMBER_SOURCE];
  return literals instanceof Map ? (literals as Map<string, NumberLiteral>) : undefined;
}

/**
 * 某個 u64 欄位（§6 的 `sequence`／`baseRevision`／`payload.revision`）是否合法。
 *
 * 有原文就看原文（`1.0`、`1e30`、超過 2^64 都不是 u64，權威 host 也是這樣判的）；
 * 沒有原文（程式碼直接組出來的信封）就退回 JS 能誠實表達的範圍：安全整數且非負。
 * 原文與現值對不上表示呼叫端改過這個欄位，這時原文已經過期，一律以現值為準。
 */
function isUint64Field(envelope: Envelope, pointer: string, value: unknown): boolean {
  const literal = numberSourceOf(envelope)?.get(pointer);
  if (literal !== undefined && Number(literal.raw) === value) return isUint64Literal(literal.raw);
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

/** RFC 3339。chrono 端也只接受這個形狀，兩邊必須一樣嚴。 */
const RFC3339 = /^\d{4}-\d{2}-\d{2}[Tt ]\d{2}:\d{2}:\d{2}(\.\d+)?([Zz]|[+-]\d{2}:\d{2})$/;

function isTimestamp(value: unknown): value is string {
  return typeof value === "string" && RFC3339.test(value) && Number.isFinite(Date.parse(value));
}

function isParty(value: unknown): value is Party {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as Party).kind === "string" &&
    typeof (value as Party).id === "string"
  );
}

/**
 * 大小上限（§11）→ JSON 解析 → 結構檢查（等同 Rust 的 serde 反序列化）。
 * 未知的頂層欄位原封保留在物件上（§1），不執行、不拒絕。
 */
export function parseEnvelope(input: string | Uint8Array): AipOutcome<Envelope> {
  const bytes = typeof input === "string" ? UTF8.encode(input) : input;
  if (bytes.length > AIP_LIMITS.maxMessageBytes) {
    return fail("message-too-large", `message exceeds ${AIP_LIMITS.maxMessageBytes} bytes`);
  }
  const text = typeof input === "string" ? input : new TextDecoder().decode(input);
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    return fail("schema-invalid", "invalid envelope (syntax)");
  }
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
    return fail("schema-invalid", "invalid envelope (data)");
  }
  const source = raw as Record<string, unknown>;
  const requiredString = (key: string) => typeof source[key] === "string";
  for (const key of ["specVersion", "messageId", "messageType", "name"]) {
    if (!requiredString(key)) return fail("schema-invalid", "invalid envelope (data)");
  }
  if (!isParty(source.source)) return fail("schema-invalid", "invalid envelope (data)");
  if (!isTimestamp(source.occurredAt)) return fail("schema-invalid", "invalid envelope (data)");
  if (source.target !== undefined && source.target !== null && !isParty(source.target)) {
    return fail("schema-invalid", "invalid envelope (data)");
  }
  for (const key of ["sessionId", "correlationId", "causationId", "consentGrantId"]) {
    const value = source[key];
    if (value !== undefined && value !== null && typeof value !== "string") {
      return fail("schema-invalid", "invalid envelope (data)");
    }
  }
  // 數字字面值的原文：`sequence`／`baseRevision` 在 Rust 是 u64 欄位，`1.0`、`1e30`、
  // 超過 2^64 的值在那邊連反序列化都過不了。JSON.parse 之後看不出差別，所以比對原文。
  const literals = scanNumberLiterals(text);
  for (const key of ["sequence", "baseRevision"]) {
    const value = source[key];
    if (value === undefined || value === null) continue;
    if (typeof value !== "number" || !Number.isInteger(value) || value < 0) {
      return fail("schema-invalid", "invalid envelope (data)");
    }
    const literal = literals.get(`/${key}`);
    if (literal !== undefined && !isUint64Literal(literal.raw)) {
      return fail("schema-invalid", "invalid envelope (data)");
    }
  }
  if (source.expiresAt !== undefined && source.expiresAt !== null && !isTimestamp(source.expiresAt)) {
    return fail("schema-invalid", "invalid envelope (data)");
  }
  const envelope = { ...source } as Envelope;
  if (envelope.payload === undefined) envelope.payload = null;
  Object.defineProperty(envelope, AIP_NUMBER_SOURCE, {
    value: literals,
    enumerable: false,
    configurable: true,
  });
  return ok(envelope);
}

/**
 * 序列化一則信封，並把解析時看到的數字字面值原封寫回去（AIP §1「round-trip 不遺失」）。
 *
 * `JSON.stringify` 會把 9007199254740993 寫成 9007199254740992、把 1000000000000000001
 * 寫成 1e+18——後者連 Rust 的 u64 欄位都讀不回來。只有**沒被改過**的欄位才還原原文：
 * 呼叫端動過的值以呼叫端的為準，這個函式不會把新值蓋掉。
 */
export function encodeEnvelope(envelope: Envelope): string {
  const text = JSON.stringify(envelope);
  const preserved = numberSourceOf(envelope);
  if (preserved === undefined || preserved.size === 0) return text;
  const restorations: Array<{ start: number; end: number; raw: string }> = [];
  for (const [pointer, current] of scanNumberLiterals(text)) {
    const original = preserved.get(pointer);
    if (original === undefined || original.raw === current.raw) continue;
    if (Number(original.raw) !== Number(current.raw)) continue;
    restorations.push({ start: current.start, end: current.end, raw: original.raw });
  }
  if (restorations.length === 0) return text;
  // 由後往前替換，前面的位移才不會被動到。
  restorations.sort((a, b) => b.start - a.start);
  let out = text;
  for (const patch of restorations) out = out.slice(0, patch.start) + patch.raw + out.slice(patch.end);
  return out;
}

function checkId(value: string): AipOutcome<void> {
  const chars = [...value];
  if (chars.length === 0 || chars.length > AIP_LIMITS.maxIdChars) {
    return fail("schema-invalid", `identifiers must be 1..=${AIP_LIMITS.maxIdChars} chars`);
  }
  if (/[\p{White_Space}\p{Cc}]/u.test(value)) {
    return fail("schema-invalid", "an identifier contains whitespace or control characters");
  }
  return OK_VOID;
}

/** payload：大小、巢狀深度、字串長度（§11）。順序與 Rust 相同：先量大小，再走樹。 */
export function checkPayload(payload: unknown): AipOutcome<void> {
  const serialized = payload === undefined ? "null" : JSON.stringify(payload) ?? "null";
  if (UTF8.encode(serialized).length > AIP_LIMITS.maxPayloadBytes) {
    return fail("payload-too-large", `payload exceeds ${AIP_LIMITS.maxPayloadBytes} bytes`);
  }
  const walk = (value: unknown, depth: number): AipOutcome<void> => {
    if (depth > AIP_LIMITS.maxJsonDepth) {
      return fail("schema-invalid", "payload nesting too deep");
    }
    if (typeof value === "string") {
      if ([...value].length > AIP_LIMITS.maxStringChars) {
        return fail("schema-invalid", `payload string exceeds ${AIP_LIMITS.maxStringChars} chars`);
      }
      return OK_VOID;
    }
    if (Array.isArray(value)) {
      for (const item of value) {
        const result = walk(item, depth + 1);
        if (!result.ok) return result;
      }
      return OK_VOID;
    }
    if (typeof value === "object" && value !== null) {
      for (const item of Object.values(value)) {
        const result = walk(item, depth + 1);
        if (!result.ok) return result;
      }
      return OK_VOID;
    }
    return OK_VOID;
  };
  return walk(payload ?? null, 1);
}

// ------------------------------------------------------------ state patch

/**
 * RFC 7396 JSON Merge Patch（AIP §6 的 `state{kind:"patch"}` 就是這個形狀）。
 *
 * 規則只有三條：patch 不是物件就整份取代；patch 的值是 `null` 就刪掉那個鍵；
 * 兩邊都是物件就遞迴合併。**陣列整個換掉**（成員清單因此永遠是完整的一份，
 * 不會半新半舊）。純函式：不改動傳進來的任何值，回傳新的物件。
 *
 * 桌面端只用它把權威狀態的變更套到本地副本上；**不做**接收端 hash 核對，
 * 理由見 `CharacterSyncCard`（JS 的 number 留不住 `0.0` 這種字面，重算出來的
 * canonical JSON 與 Rust 端不會逐位元組相同）。對不上的時候以重新 GET snapshot
 * 對齊，不是靠 hash 判定。
 */
export function applyMergePatch(target: unknown, patch: unknown): unknown {
  if (typeof patch !== "object" || patch === null || Array.isArray(patch)) return patch;
  const base: Record<string, unknown> =
    typeof target === "object" && target !== null && !Array.isArray(target)
      ? { ...(target as Record<string, unknown>) }
      : {};
  for (const [key, value] of Object.entries(patch as Record<string, unknown>)) {
    if (value === null) {
      delete base[key];
      continue;
    }
    base[key] = applyMergePatch(base[key], value);
  }
  return base;
}

function need(condition: boolean, messageType: string, what: string): AipOutcome<void> {
  return condition ? OK_VOID : fail("schema-invalid", `${messageType} requires ${what}`);
}

function payloadObject(envelope: Envelope): Record<string, unknown> {
  const payload = envelope.payload;
  return typeof payload === "object" && payload !== null && !Array.isArray(payload)
    ? (payload as Record<string, unknown>)
    : {};
}

/**
 * §2.2 profile 必填 ＋ §11 上限 ＋ §4 版本 ＋ name 語法。
 * 順序固定並與 Rust 一致；第一個失敗即回，未知的一律不執行。
 */
export function validateEnvelope(envelope: Envelope): AipOutcome<void> {
  const version = negotiateVersion(envelope.specVersion);
  if (!version.ok) return version;
  if (!(AIP_MESSAGE_TYPES as readonly string[]).includes(envelope.messageType)) {
    // §5：未知 type 的原字串是呼叫端可控的資料，只留在本地稽核，不進錯誤訊息。
    return fail("unsupported-message-type", "messageType is not one of the 12 known AIP message types");
  }
  const messageId = checkId(envelope.messageId);
  if (!messageId.ok) return messageId;
  if (!isValidName(envelope.name)) return fail("schema-invalid", "name violates grammar");
  const sourceId = checkId(envelope.source.id);
  if (!sourceId.ok) return sourceId;
  if (!(AIP_KNOWN_PARTY_KINDS as readonly string[]).includes(envelope.source.kind)) {
    return fail("schema-invalid", "source.kind unknown");
  }
  if (envelope.target) {
    const targetId = checkId(envelope.target.id);
    if (!targetId.ok) return targetId;
  }
  for (const value of [
    envelope.sessionId,
    envelope.correlationId,
    envelope.causationId,
    envelope.consentGrantId,
  ]) {
    if (typeof value === "string") {
      const result = checkId(value);
      if (!result.ok) return result;
    }
  }
  const payloadCheck = checkPayload(envelope.payload);
  if (!payloadCheck.ok) return payloadCheck;

  const type = envelope.messageType;
  const payload = payloadObject(envelope);
  switch (type) {
    case "event": {
      const session = need(typeof envelope.sessionId === "string", type, "sessionId");
      if (!session.ok) return session;
      if (envelope.name.startsWith("character.interaction.")) {
        return need(typeof envelope.expiresAt === "string", type, "expiresAt for interaction events");
      }
      return OK_VOID;
    }
    case "command": {
      for (const [condition, what] of [
        [typeof envelope.sessionId === "string", "sessionId"],
        [!!envelope.target, "target"],
        [typeof envelope.correlationId === "string", "correlationId"],
        [typeof envelope.expiresAt === "string", "expiresAt"],
      ] as const) {
        const result = need(condition, type, what);
        if (!result.ok) return result;
      }
      return OK_VOID;
    }
    case "query":
      return need(!!envelope.target, type, "target");
    case "response":
      return need(typeof envelope.causationId === "string", type, "causationId");
    case "result": {
      const causation = need(typeof envelope.causationId === "string", type, "causationId");
      if (!causation.ok) return causation;
      const status = payload.status;
      const known = typeof status === "string" && (AIP_OUTCOMES as readonly string[]).includes(status);
      return need(known, type, "a known payload.status");
    }
    case "state": {
      for (const [condition, what] of [
        [typeof envelope.sessionId === "string", "sessionId"],
        [typeof envelope.sequence === "number", "sequence"],
        [
          typeof payload.revision === "number" &&
            Number.isInteger(payload.revision) &&
            (payload.revision as number) >= 0 &&
            isUint64Field(envelope, "/payload/revision", payload.revision),
          "payload.revision",
        ],
      ] as const) {
        const result = need(condition, type, what);
        if (!result.ok) return result;
      }
      if (payload.kind === "patch") {
        return need(typeof envelope.baseRevision === "number", type, "baseRevision for patches");
      }
      return OK_VOID;
    }
    case "cancel":
      return need(
        typeof envelope.causationId === "string" || typeof payload.messageId === "string",
        type,
        "causationId or payload.messageId",
      );
    case "approval-request": {
      for (const [condition, what] of [
        [typeof envelope.correlationId === "string", "correlationId"],
        [typeof envelope.expiresAt === "string", "expiresAt"],
        [envelope.target?.kind === "human", "target{kind:human}"],
      ] as const) {
        const result = need(condition, type, what);
        if (!result.ok) return result;
      }
      return OK_VOID;
    }
    case "approval-result":
      return need(typeof envelope.causationId === "string", type, "causationId");
    case "error":
      return need(typeof payload.code === "string", type, "payload.code");
    default:
      // heartbeat／capability 沒有額外必填欄位。
      return OK_VOID;
  }
}

/** §7：`expiresAt` 已過（含等於）→ 過期。沒有 expiresAt → 不過期。 */
export function isExpired(envelope: Envelope, nowMs: number): boolean {
  if (typeof envelope.expiresAt !== "string") return false;
  const at = Date.parse(envelope.expiresAt);
  return Number.isFinite(at) && at <= nowMs;
}

// -------------------------------------------------------------------- 身分

export type IdentityDecision =
  | { kind: "accept" }
  | { kind: "reject"; bound: Party; claimed: Party };

/**
 * §5：`source` 只是宣稱。不符一律拒絕並稽核；host **不得**「幫忙修正」後執行。
 */
export function bindIdentity(bound: Party, claimed: Party): IdentityDecision {
  if (bound.kind === claimed.kind && bound.id === claimed.id) return { kind: "accept" };
  return { kind: "reject", bound, claimed };
}

// ---------------------------------------------------------------- 去重（有界）

/** §7 有界去重環（每個 (session, source) 一份）。滿了淘汰最舊，永遠不會無界成長。 */
export class DedupeRing {
  private readonly cap: number;
  private readonly order: string[] = [];
  private readonly seen = new Set<string>();

  constructor(cap: number = AIP_LIMITS.dedupeRing) {
    this.cap = Math.min(Math.max(Math.trunc(cap), 1), AIP_LIMITS.dedupeRing);
  }

  /** `true` = 第一次看到（已記下）；`false` = 重複，不得重新套用。 */
  note(messageId: string): boolean {
    if (this.seen.has(messageId)) return false;
    if (this.order.length >= this.cap) {
      const oldest = this.order.shift();
      if (oldest !== undefined) this.seen.delete(oldest);
    }
    this.order.push(messageId);
    this.seen.add(messageId);
    return true;
  }

  has(messageId: string): boolean {
    return this.seen.has(messageId);
  }

  get size(): number {
    return this.order.length;
  }
}

// ---------------------------------------------------------------- 離線政策

/**
 * 需要人類再確認的 name（§8）：`approval-request` 的線上名字。
 *
 * `character.session.approval` 同時符合 `character.session.` 前綴，所以這條**必須**先判斷；
 * 反過來排的話，唯一真正存在的 approval name 會被歸成 `state-reconcile`，等於離線後可以自動
 * 對齊——那是人類決定，不得自動重送。與 Rust `interaction_aip::offline_policy` 逐條一致。
 */
function isApprovalName(name: string): boolean {
  return name.startsWith("approval.") || name.endsWith(".approval");
}

/** §8 的固定歸類表。未知 name → `drop-if-offline`（最保守：不排隊、不重播）。 */
export function offlinePolicy(name: string, hasConsentGrant = false): OfflinePolicy {
  if (hasConsentGrant) return "require-reconfirmation";
  if (isApprovalName(name)) return "require-reconfirmation";
  if (name.startsWith("character.interaction.touch")) return "expire-by-deadline";
  if (name.startsWith("character.interaction.")) return "drop-if-offline";
  if (name.startsWith("character.behavior.")) return "drop-if-offline";
  if (name.startsWith("character.preference.")) return "queue-idempotent";
  if (name.startsWith("character.session.")) return "state-reconcile";
  if (name.startsWith("task.") || name.startsWith("runtime.")) return "state-reconcile";
  return "drop-if-offline";
}

// ------------------------------------------------------------ Outcome 誠實階梯

const TRANSITIONS: ReadonlyArray<readonly [Outcome, Outcome]> = [
  ["received", "accepted"],
  ["received", "rejected"],
  ["received", "expired"],
  ["accepted", "acknowledged"],
  ["accepted", "applied"],
  ["accepted", "observed"],
  ["accepted", "claimed-completed"],
  ["accepted", "expired"],
  ["accepted", "failed"],
  ["accepted", "cancel-requested"],
  ["accepted", "cancel-confirmed"],
  ["acknowledged", "observed"],
  ["acknowledged", "failed"],
  ["acknowledged", "expired"],
  ["acknowledged", "cancel-requested"],
  ["acknowledged", "cancel-confirmed"],
  ["claimed-completed", "verified"],
  ["claimed-completed", "failed"],
  ["cancel-requested", "cancel-confirmed"],
  ["cancel-requested", "failed"],
];

const TERMINAL: readonly Outcome[] = [
  "applied",
  "observed",
  "verified",
  "rejected",
  "expired",
  "cancel-confirmed",
  "failed",
];

export function isTerminalOutcome(status: Outcome): boolean {
  return TERMINAL.includes(status);
}

/** `verified` 只能由 Runtime 的人類驗證路徑產生；adapter／device／renderer 一律不得。 */
export function isRuntimeOnlyOutcome(status: Outcome): boolean {
  return status === "verified";
}

/** 合法遷移：只能往前，終態黏住，`observed`／`acknowledged` 永遠走不到 `verified`。 */
export function canTransitionOutcome(from: Outcome, to: Outcome): boolean {
  if (isTerminalOutcome(from)) return false;
  return TRANSITIONS.some(([a, b]) => a === from && b === to);
}

const EVENT_OUTCOMES: readonly Outcome[] = ["received", "accepted", "applied", "rejected", "expired"];
const COMMAND_OUTCOMES: readonly Outcome[] = [
  "received",
  "accepted",
  "acknowledged",
  "observed",
  "rejected",
  "expired",
  "failed",
  "cancel-requested",
  "cancel-confirmed",
];
const STATE_OUTCOMES: readonly Outcome[] = ["applied", "rejected"];

export function isOutcomeAllowedFor(profile: "event" | "command" | "state", status: Outcome): boolean {
  if (profile === "event") return EVENT_OUTCOMES.includes(status);
  if (profile === "command") return COMMAND_OUTCOMES.includes(status);
  return STATE_OUTCOMES.includes(status);
}

// ------------------------------------------------------------ Capability 協商

export interface HostOffer {
  intents: string[];
  inputs: string[];
  syncClasses: SyncClass[];
}

const SYNC_CLASS_ORDER: readonly SyncClass[] = ["semantic", "timeline", "realtime"];

/**
 * §4.2 確定性協商：版本 → role（缺省 observer）→ sync class（交集裡最保守）→
 * intents（host 需要的每個 intent：對方宣告有→exact，否則 unsupported）→
 * inputs（對方宣告 ∩ host 接受）→ limits（min）。
 *
 * renderer 不能靠宣告憑空得到 host 沒提供的 intent，也不能把訊息上限抬高。
 */
/**
 * `unsupportedInputs` 的上限。權威值在 Rust（`crates/interaction-aip/src/limits.rs` 的
 * `MAX_UNSUPPORTED_INPUTS`）；它刻意**不進** `schemas/aip-1.0.schema.json` 的 limits 表，
 * 因為它不是 wire 契約的一部分，只是 host 端協商結果的有界性要求。
 */
const MAX_UNSUPPORTED_INPUTS = 32;

export function negotiateCapabilities(
  offer: HostOffer,
  announcement: CapabilityAnnouncement,
): AipOutcome<NegotiatedCapabilities> {
  const version = negotiateVersions(announcement.specVersions ?? []);
  if (!version.ok) return version;
  const role: MemberRole = announcement.role ?? "observer";
  const announced = announcement.syncClasses ?? [];
  const shared = (offer.syncClasses ?? []).filter((c) => announced.includes(c));
  const syncClass: SyncClass =
    shared.length === 0
      ? "semantic"
      : shared.reduce((best, current) =>
          SYNC_CLASS_ORDER.indexOf(current) < SYNC_CLASS_ORDER.indexOf(best) ? current : best,
        );
  const intents: Record<string, IntentSupport> = {};
  for (const intent of [...(offer.intents ?? [])].sort()) {
    intents[intent] = (announcement.intents ?? []).includes(intent) ? "exact" : "unsupported";
  }
  const inputs: string[] = [];
  const unsupportedInputs: string[] = [];
  for (const input of announcement.inputs ?? []) {
    if ((offer.inputs ?? []).includes(input)) {
      if (!inputs.includes(input)) inputs.push(input);
    } else if (!unsupportedInputs.includes(input)) {
      // 對方宣告的 inputs 是外部輸入、本身無界；協商結果會被序列化成一則要送上線的
      // `capability` 回覆，所以這份清單必須有界（session-integrity-060）。截斷是確定性的
      // （取前 N 個），與 Rust 權威實作 `interaction_aip::limits::MAX_UNSUPPORTED_INPUTS` 同值。
      if (unsupportedInputs.length >= MAX_UNSUPPORTED_INPUTS) continue;
      unsupportedInputs.push(input);
    }
  }
  const remoteMax = announcement.limits?.maxMessageBytes ?? AIP_LIMITS.maxMessageBytes;
  return ok({
    specVersion: version.value.specVersion,
    newerMinor: version.value.newerMinor,
    role,
    syncClass,
    intents,
    inputs,
    unsupportedInputs,
    limits: {
      maxMessageBytes: Math.min(Math.max(remoteMax, 1024), AIP_LIMITS.maxMessageBytes),
    },
  });
}

export { AIP_ERROR_CODES, AIP_LIMITS, AIP_MESSAGE_TYPES, AIP_SPEC_VERSION };
