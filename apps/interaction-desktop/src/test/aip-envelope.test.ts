// @vitest-environment node
//
// AIP 驗證邏輯的單元測試：fixture index 沒辦法表達的邊界（多位元組長度、環的上限、
// 過期判斷的邊界、未知 minor 的處理）在這裡釘住。

import { describe, expect, it } from "vitest";

import { AIP_LIMITS, AIP_SPEC_VERSION, type Envelope } from "../aip/generated";
import {
  DedupeRing,
  bindIdentity,
  canTransitionOutcome,
  checkPayload,
  isExpired,
  isValidName,
  negotiateVersion,
  negotiateVersions,
  offlinePolicy,
  parseEnvelope,
  validateEnvelope,
} from "../aip/envelope";

function touch(overrides: Partial<Envelope> = {}): Envelope {
  return {
    specVersion: AIP_SPEC_VERSION,
    messageId: "msg_1",
    messageType: "event",
    name: "character.interaction.touch",
    source: { kind: "device", id: "iphone-1" },
    sessionId: "session.home",
    occurredAt: "2026-09-04T12:30:00Z",
    expiresAt: "2026-09-04T12:30:05Z",
    payload: { kind: "tap" },
    ...overrides,
  } as Envelope;
}

describe("parseEnvelope", () => {
  it("measures the size limit in UTF-8 bytes, not code points", () => {
    // 一個中日韓字元 3 bytes：字元數遠低於上限，位元組數卻超過。
    const padded = touch({ payload: { note: "界".repeat(30_000) } });
    const raw = JSON.stringify(padded);
    expect([...raw].length).toBeLessThan(AIP_LIMITS.maxMessageBytes);
    const result = parseEnvelope(raw);
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error.code).toBe("message-too-large");
  });

  it("refuses a JSON array, a bare string and a truncated body without echoing them", () => {
    for (const raw of ['[{"specVersion":"aip/1.0"}]', '"do-not-echo-me"', '{"a":']) {
      const result = parseEnvelope(raw);
      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.error.code).toBe("schema-invalid");
        expect(result.error.message).not.toContain("do-not-echo-me");
      }
    }
  });

  it("defaults a missing payload to null rather than undefined", () => {
    const { payload, ...rest } = touch();
    void payload;
    const result = parseEnvelope(JSON.stringify(rest));
    expect(result.ok).toBe(true);
    if (result.ok) expect(result.value.payload).toBeNull();
  });

  it("keeps unknown top-level fields verbatim", () => {
    const raw = JSON.stringify({ ...touch(), futureField: { keep: [1, null, "x"] } });
    const result = parseEnvelope(raw);
    expect(result.ok).toBe(true);
    if (result.ok) expect(result.value.futureField).toEqual({ keep: [1, null, "x"] });
  });
});

describe("validateEnvelope", () => {
  it("accepts a newer minor and refuses a different major", () => {
    expect(validateEnvelope(touch({ specVersion: "aip/1.9" })).ok).toBe(true);
    const major = validateEnvelope(touch({ specVersion: "aip/2.0" }));
    expect(major.ok).toBe(false);
    if (!major.ok) expect(major.error.code).toBe("unsupported-version");
  });

  it("never lets a result claim an unknown status", () => {
    const result = validateEnvelope(
      touch({
        messageType: "result",
        causationId: "msg_0",
        payload: { status: "definitely-done" },
      }),
    );
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.error.code).toBe("schema-invalid");
      expect(result.error.message).not.toContain("definitely-done");
    }
  });

  it("bounds identifiers and rejects whitespace inside them", () => {
    for (const messageId of ["", "m".repeat(AIP_LIMITS.maxIdChars + 1), "msg 1", "msg "]) {
      expect(validateEnvelope(touch({ messageId })).ok).toBe(false);
    }
  });
});

describe("checkPayload", () => {
  it("reports size before depth so a huge blob is not mislabelled", () => {
    const big = checkPayload({ blob: "x".repeat(AIP_LIMITS.maxPayloadBytes) });
    expect(big.ok).toBe(false);
    if (!big.ok) expect(big.error.code).toBe("payload-too-large");
  });

  it("rejects nesting deeper than the limit", () => {
    let value: unknown = 1;
    for (let i = 0; i < AIP_LIMITS.maxJsonDepth; i += 1) value = { nested: value };
    const result = checkPayload(value);
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error.code).toBe("schema-invalid");
  });

  it("accepts a payload right at the depth limit", () => {
    let value: unknown = 1;
    for (let i = 0; i < AIP_LIMITS.maxJsonDepth - 1; i += 1) value = { nested: value };
    expect(checkPayload(value).ok).toBe(true);
  });
});

describe("deadlines", () => {
  const now = Date.parse("2026-09-04T12:30:05Z");

  it("treats the deadline itself as expired and a missing deadline as never expiring", () => {
    expect(isExpired(touch(), now - 1)).toBe(false);
    expect(isExpired(touch(), now)).toBe(true);
    expect(isExpired(touch({ expiresAt: undefined }), now)).toBe(false);
  });
});

describe("version negotiation", () => {
  it("refuses to guess at a malformed version string", () => {
    for (const bad of ["1.0", "aip/1", "aip/a.b", "", "aip/"]) {
      const result = negotiateVersion(bad);
      expect(result.ok).toBe(false);
      if (!result.ok) expect(result.error.code).toBe("schema-invalid");
    }
  });

  it("picks the first workable candidate and reports an empty list honestly", () => {
    const picked = negotiateVersions(["aip/2.0", "aip/1.4"]);
    expect(picked.ok).toBe(true);
    if (picked.ok) {
      expect(picked.value.specVersion).toBe("aip/1.0");
      expect(picked.value.newerMinor).toBe(true);
    }
    const empty = negotiateVersions([]);
    expect(empty.ok).toBe(false);
    if (!empty.ok) expect(empty.error.code).toBe("unsupported-version");
  });
});

describe("bounded state", () => {
  it("never grows the dedupe ring past the protocol limit", () => {
    const ring = new DedupeRing(1_000_000);
    for (let i = 0; i < AIP_LIMITS.dedupeRing + 50; i += 1) ring.note(`msg_${i}`);
    expect(ring.size).toBe(AIP_LIMITS.dedupeRing);
    expect(ring.has("msg_0")).toBe(false);
    expect(ring.has(`msg_${AIP_LIMITS.dedupeRing + 49}`)).toBe(true);
  });
});

describe("names, identity and offline policy", () => {
  it("holds the name grammar", () => {
    expect(isValidName("character.interaction.touch")).toBe(true);
    expect(isValidName("character.session.resume-now")).toBe(true);
    expect(isValidName("touch")).toBe(false);
    expect(isValidName("a..b")).toBe(false);
    expect(isValidName("a.-b")).toBe(false);
    expect(isValidName(`a.${"b".repeat(AIP_LIMITS.maxNameChars)}`)).toBe(false);
  });

  it("never normalises a mismatched identity into an accepted one", () => {
    const bound = { kind: "device", id: "iphone-1" } as const;
    expect(bindIdentity(bound, { kind: "device", id: "iphone-1" }).kind).toBe("accept");
    expect(bindIdentity(bound, { kind: "runtime", id: "runtime" }).kind).toBe("reject");
  });

  it("falls back to the most conservative offline class for unknown names", () => {
    expect(offlinePolicy("totally.unknown")).toBe("drop-if-offline");
    expect(offlinePolicy("character.interaction.touch", true)).toBe("require-reconfirmation");
  });

  it("keeps observed and acknowledged away from verified", () => {
    expect(canTransitionOutcome("observed", "verified")).toBe(false);
    expect(canTransitionOutcome("acknowledged", "verified")).toBe(false);
    expect(canTransitionOutcome("claimed-completed", "verified")).toBe(true);
  });
});
