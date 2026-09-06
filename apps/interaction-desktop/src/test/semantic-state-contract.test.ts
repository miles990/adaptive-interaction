// @vitest-environment node
import { describe, expect, it } from "vitest";
import { stateHash } from "../aip/canonical";
import { initialSession, reduce } from "../aip/sessionClient";

const valid = () => ({ characterId: "ref-shape", mood: { kind: "neutral", intensity: 0 }, activity: "idle", attention: { kind: "none" }, truth: { state: "none" }, members: [], reducedMotion: false });
function snapshot(state: unknown) {
  return { messageType: "state", sessionId: "session.home", payload: { kind: "snapshot", revision: 1, sessionEpoch: 1, state, hash: stateHash(state) } };
}
describe("semantic state production receive boundary", () => {
  it.each(["attention", "members", "reducedMotion"])("rejects missing required %s before adoption", (key) => {
    const state: Record<string, unknown> = valid();
    delete state[key];
    const result = reduce(initialSession(), { kind: "sse", arrivedOn: 0, envelope: snapshot(state) });
    expect(result.next.lastDecision?.decision).toBe("reject-invalid");
    expect(result.next.local).toBeNull();
  });
  it("rejects null optional state values while permitting null in a merge patch", () => {
    const result = reduce(initialSession(), { kind: "sse", arrivedOn: 0, envelope: snapshot({ ...valid(), lastInteraction: null }) });
    expect(result.next.lastDecision?.decision).toBe("reject-invalid");
  });
  it("rejects a hash-consistent merge that deletes a required field atomically", () => {
    const initial = reduce(initialSession(), { kind: "sse", arrivedOn: 0, envelope: snapshot(valid()) }).next;
    const invalid: Record<string, unknown> = valid();
    delete invalid["mood"];
    const envelope = { messageType: "state", sessionId: "session.home", baseRevision: 1, payload: { kind: "patch", revision: 2, sessionEpoch: 1, patch: { mood: null }, hash: stateHash(invalid) } };
    const result = reduce(initial, { kind: "sse", arrivedOn: 0, envelope });
    expect(result.next.lastDecision?.decision).toBe("reject-invalid");
    expect(result.next.local).toBe(initial.local);
  });
});

import { readFileSync } from "node:fs";
import { canonicalJson, sha256Hex } from "../aip/canonical";
import { parseJsonWithNumberSources } from "../aip/json-source";
import { applyMergePatch } from "../aip/envelope";
import { validateSemanticState, projectSemanticState } from "../aip/semanticState";
import { SEMANTIC_STATE_DOUBLE_PATHS } from "../aip/generated";
const fixtureRoot = new URL("../../../../crates/interaction-aip/tests/fixtures/", import.meta.url);
const corpus = JSON.parse(readFileSync(decodeURIComponent(new URL("semantic-state-cases.json", fixtureRoot).pathname), "utf8")) as {
  states: Array<{ id: string; wire: string; accept: boolean; canonical: string; hash: string; projection?: Record<string, unknown> }>;
  patches: Array<{ id: string; baseWire: string; patchWire: string; accept: boolean; canonical: string; hash: string }>;
};
describe("shared semantic consumer corpus", () => {
  it.each(corpus.states)("$id: runtime validation, canonical/hash and projection", (entry) => {
    const raw = parseJsonWithNumberSources(entry.wire);
    const checked = validateSemanticState(raw);
    expect(checked !== null).toBe(entry.accept);
    expect(canonicalJson(raw, SEMANTIC_STATE_DOUBLE_PATHS)).toBe(entry.canonical);
    expect(stateHash(raw)).toBe(entry.hash);
    const adopted = reduce(initialSession(), { kind: "sse", arrivedOn: 0, envelope: snapshot(raw) }).next;
    expect(adopted.lastDecision?.decision).toBe(entry.accept ? "apply" : "reject-invalid");
    if (checked) {
      const projected = projectSemanticState(checked);
      for (const [key, value] of Object.entries(entry.projection ?? {})) expect((projected as Record<string, unknown>)[key] ?? null).toEqual(value);
      expect(adopted.local?.state).toBe(raw);
    }
  });
  it.each(corpus.patches)("$id: patch and resume validate the whole merged result", (entry) => {
    const base = parseJsonWithNumberSources(entry.baseWire);
    const patch = parseJsonWithNumberSources(entry.patchWire);
    const merged = applyMergePatch(base, patch);
    expect(canonicalJson(merged, SEMANTIC_STATE_DOUBLE_PATHS)).toBe(entry.canonical);
    expect(stateHash(merged)).toBe(entry.hash);
    const initial = reduce(initialSession(), { kind: "sse", arrivedOn: 0, envelope: snapshot(base) }).next;
    const payload = { kind: "patches", patches: [{ revision: 2, baseRevision: 1, sessionEpoch: 1, patch, hash: entry.hash }] };
    const requested = reduce(initial, { kind: "fetch-issued", requestId: 1 }).next;
    const result = reduce(requested, { kind: "resume-response", requestId: 1, arrivedOn: 0, payload }).next;
    expect(result.local?.revision).toBe(entry.accept ? 2 : 1);
    expect(stateHash(base)).toBe(stateHash(parseJsonWithNumberSources(entry.baseWire)));
  });
  it("published corpus stays byte-for-byte unchanged even when current fixtures are regenerated", () => {
    const release = new URL("releases/v0.7.0/", fixtureRoot);
    const index = JSON.parse(readFileSync(decodeURIComponent(new URL("manifest.json", release).pathname), "utf8")) as { sourceSha: string; files: Array<{file:string;sha256:string}> };
    expect(index.sourceSha).toBe("630b4291f6a59444cfb1d8185f757f9dcda9ecc4");
    const contents: Record<string,string> = {};
    for (const file of index.files) {
      const text = readFileSync(decodeURIComponent(new URL(file.file, release).pathname), "utf8");
      expect(sha256Hex(text)).toBe(file.sha256);
      contents[file.file] = text;
    }
    expect(sha256Hex(canonicalJson(contents))).toBe("f043a8c9fbbd4eb5441a4069d009cc2c8e31a1b22f0a4c3102b46f356d72e1b0");
    const envelope = parseJsonWithNumberSources(contents["state-snapshot.json"]!);
    const initial = reduce(initialSession(), { kind: "sse", arrivedOn: 0, envelope }).next;
    expect(initial.lastDecision?.decision).toBe("apply");
    const patched = reduce(initial, { kind: "sse", arrivedOn: 0, envelope: parseJsonWithNumberSources(contents["state-patch.json"]!) }).next;
    expect(patched.lastDecision?.decision).toBe("apply");
  });
});
