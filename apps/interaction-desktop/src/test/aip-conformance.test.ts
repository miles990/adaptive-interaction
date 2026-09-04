// @vitest-environment node
//
// AIP 1.0 跨語言 conformance（TypeScript 端）。
//
// 讀的是 Rust crate 底下**同一份** fixture index（`crates/interaction-aip/tests/fixtures/manifest.json`）：
// Rust、TypeScript、Swift 三個實作對同一組訊息必須得到同一個結論。這份測試存在的理由是：
// 桌面端如果對某則訊息比 Runtime 寬鬆，攻擊面就從 Rust 的確定性檢查漏到 WebView。
//
// 契約：docs/aip/README.md §14；跑法：docs/aip/conformance.md。

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

import {
  DedupeRing,
  bindIdentity,
  canTransitionOutcome,
  encodeEnvelope,
  isOutcomeAllowedFor,
  isRuntimeOnlyName,
  negotiateCapabilities,
  offlinePolicy,
  parseEnvelope,
  validateEnvelope,
  type AipOutcome,
} from "../aip/envelope";
import { scanNumberLiterals } from "../aip/json-source";
import { AIP_MESSAGE_TYPES, type Envelope } from "../aip/generated";

// @types/node 不在這個 app 的依賴裡，所以不用 node:url 的 fileURLToPath，
// 直接從 import.meta.url 推出 repo 內的 fixture 目錄。
const FIXTURES = decodeURIComponent(
  new URL("../../../../crates/interaction-aip/tests/fixtures/", import.meta.url).pathname,
);

const read = (name: string) => readFileSync(`${FIXTURES}${name}`, "utf8");
const manifest = JSON.parse(read("manifest.json"));

/** 一則 wire 文字走完整條檢查：大小 → 解析 → profile／上限／版本驗證。 */
function evaluate(raw: string): AipOutcome<Envelope> {
  const parsed = parseEnvelope(raw);
  if (!parsed.ok) return parsed;
  const validated = validateEnvelope(parsed.value);
  if (!validated.ok) return validated;
  return parsed;
}

function assertExpectation(id: string, entry: Record<string, unknown>, raw: string) {
  const result = evaluate(raw);
  if (entry.expect === "ok") {
    expect(result.ok, `fixture ${id} should be accepted (got ${result.ok ? "" : result.error.code})`).toBe(
      true,
    );
    return result.ok ? result.value : null;
  }
  expect(result.ok, `fixture ${id} should be rejected but passed validation`).toBe(false);
  if (result.ok) return null;
  expect(result.error.code, `fixture ${id} produced the wrong ErrorCode`).toBe(entry.code);
  for (const token of (entry.mustNotEcho as string[] | undefined) ?? []) {
    expect(result.error.message, `fixture ${id}: error message echoes caller input`).not.toContain(
      token,
    );
  }
  for (const leak of ["/Users", "/private", "/home", ".json", ".ts", "\\", "://"]) {
    expect(result.error.message, `fixture ${id}: error message leaks a path-like fragment`).not.toContain(
      leak,
    );
  }
  expect([...result.error.message].length).toBeLessThanOrEqual(200);
  return null;
}

describe("AIP conformance — envelope fixtures", () => {
  it("reads the same index the Rust and Swift suites read", () => {
    expect(manifest.specVersion).toBe("aip/1.0");
    expect(manifest.envelopes.length).toBeGreaterThanOrEqual(20);
  });

  for (const entry of manifest.envelopes) {
    it(`${entry.expect === "ok" ? "accepts" : "rejects"} ${entry.id}`, () => {
      assertExpectation(entry.id, entry, read(entry.file));
    });
  }

  it("covers every message type with at least one accepted fixture", () => {
    const covered = new Set(
      manifest.envelopes
        .filter((e: Record<string, unknown>) => e.expect === "ok")
        .map((e: Record<string, string>) => JSON.parse(read(e.file)).messageType),
    );
    for (const type of AIP_MESSAGE_TYPES) {
      expect(covered.has(type), `no accepted fixture covers ${type}`).toBe(true);
    }
  });
});

describe("AIP conformance — generated (oversized and malformed)", () => {
  for (const entry of manifest.generated) {
    it(`rejects ${entry.id}`, () => {
      let raw: string;
      if (typeof entry.raw === "string") {
        raw = entry.raw;
      } else {
        const base = JSON.parse(read(entry.base));
        base.payload.blob = "x".repeat(entry.inflatePayloadChars);
        raw = JSON.stringify(base);
      }
      assertExpectation(entry.id, entry, raw);
    });
  }
});

describe("AIP conformance — round-trip", () => {
  for (const entry of manifest.envelopes.filter((e: Record<string, unknown>) => e.expect === "ok")) {
    it(`round-trips ${entry.id} without losing unknown top-level fields`, () => {
      const raw = read(entry.file);
      const original = JSON.parse(raw);
      const parsed = parseEnvelope(raw);
      expect(parsed.ok).toBe(true);
      if (!parsed.ok) return;
      const encoded = encodeEnvelope(parsed.value);
      const reparsed = parseEnvelope(encoded);
      expect(reparsed.ok).toBe(true);
      if (!reparsed.ok) return;
      expect(encodeEnvelope(reparsed.value)).toBe(encoded);

      // §1「round-trip 不遺失」是對**線上位元組**的保證，不是對「兩個都經過同一條 double
      // 管線的值」的保證：把 9007199254740993 讀成 9007199254740992 再寫回去，
      // parsed 與 reparsed 會一路相等，但收到的 Rust host 讀到的已經是另一個數字。
      // 所以這裡拿原始文字的數字字面值逐字比對。
      const before = scanNumberLiterals(raw);
      const after = scanNumberLiterals(encoded);
      for (const [pointer, literal] of before) {
        const emitted = after.get(pointer);
        expect(emitted, `fixture ${entry.id}: ${pointer} disappeared from the encoded message`)
          .toBeDefined();
        expect(
          emitted?.raw,
          `fixture ${entry.id}: number literal at ${pointer} changed across round-trip`,
        ).toBe(literal.raw);
      }

      if (entry.roundTrip === true) {
        const known = new Set([
          "specVersion",
          "messageId",
          "messageType",
          "name",
          "source",
          "target",
          "sessionId",
          "occurredAt",
          "correlationId",
          "causationId",
          "sequence",
          "baseRevision",
          "expiresAt",
          "consentGrantId",
          "payload",
        ]);
        const unknownKeys = Object.keys(original).filter((k) => !known.has(k));
        expect(unknownKeys.length).toBeGreaterThan(0);
        for (const key of unknownKeys) {
          expect(reparsed.value[key]).toEqual(original[key]);
        }
      }
    });
  }

  it("keeps an integer beyond 2^53 byte-identical on the way back out", () => {
    // JSON.stringify 會把它寫成 9007199254740992——差一個數字，而且是靜默的。
    const raw = read("roundtrip-big-integer-unknown-field.json");
    const parsed = parseEnvelope(raw);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    const encoded = encodeEnvelope(parsed.value);
    expect(encoded).toContain("9007199254740993");
    expect(encoded).toContain("1000000000000000001");
    expect(JSON.stringify(parsed.value)).not.toContain("9007199254740993");
  });

  it("does not resurrect a stale literal after the caller changes the field", () => {
    // 保留原文只對「沒被動過」的欄位有效：呼叫端改了值就以呼叫端為準，
    // 否則這層保留會變成默默改寫呼叫端的資料。
    const parsed = parseEnvelope(read("roundtrip-big-integer-unknown-field.json"));
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    const mutated = parsed.value as Record<string, unknown> & Envelope;
    mutated.futureTraceId = 7;
    const encoded = encodeEnvelope(mutated);
    expect(encoded).toContain('"futureTraceId":7');
    expect(encoded).not.toContain("9007199254740993");
    // 沒被動到的欄位仍然逐字保留。
    expect(encoded).toContain("1000000000000000001");
  });
});

describe("AIP conformance — negotiation", () => {
  for (const entry of manifest.negotiations) {
    it(`negotiates ${entry.id} deterministically`, () => {
      const result = negotiateCapabilities(entry.offer, entry.announcement);
      if (entry.expect === "ok") {
        expect(result.ok).toBe(true);
        if (!result.ok) return;
        expect(result.value).toEqual(entry.negotiated);
        const again = negotiateCapabilities(entry.offer, entry.announcement);
        expect(again.ok && again.value).toEqual(result.value);
      } else {
        expect(result.ok).toBe(false);
        if (result.ok) return;
        expect(result.error.code).toBe(entry.code);
      }
    });
  }
});

describe("AIP conformance — decision tables", () => {
  it("binds identity exactly as the Rust host does", () => {
    for (const entry of manifest.identity) {
      const decision = bindIdentity(entry.bound, entry.claimed);
      expect(decision.kind, `identity ${entry.id}`).toBe(entry.expect);
    }
  });

  it("classifies offline policy exactly as the Rust host does", () => {
    for (const entry of manifest.offlinePolicy) {
      expect(offlinePolicy(entry.name, entry.hasConsentGrant === true), entry.name).toBe(
        entry.expect,
      );
    }
  });

  it("keeps the honesty ladder: nothing walks itself up to verified", () => {
    for (const entry of manifest.outcomeTransitions) {
      expect(
        canTransitionOutcome(entry.from, entry.to),
        `${entry.from} -> ${entry.to}`,
      ).toBe(entry.allowed);
    }
    for (const entry of manifest.outcomeProfiles) {
      expect(
        isOutcomeAllowedFor(entry.profile, entry.status),
        `${entry.status} in ${entry.profile}`,
      ).toBe(entry.allowed);
    }
  });

  it("keeps task.* and runtime.* runtime-only", () => {
    for (const entry of manifest.nameScope) {
      expect(isRuntimeOnlyName(entry.name), entry.name).toBe(entry.runtimeOnly);
    }
  });
});

describe("AIP conformance — dedupe ring", () => {
  it("is bounded and evicts the oldest messageId", () => {
    const ring = new DedupeRing(2);
    expect(ring.note("a")).toBe(true);
    expect(ring.note("a")).toBe(false);
    expect(ring.note("b")).toBe(true);
    expect(ring.note("c")).toBe(true);
    expect(ring.has("a")).toBe(false);
    expect(ring.size).toBe(2);
  });
});
