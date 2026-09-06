import { afterEach, describe, expect, it, vi } from "vitest";
const native = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: native.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: native.listen }));
import { canonicalJson, stateHash } from "../aip/canonical";
import { parseJsonWithNumberSources } from "../aip/json-source";
import { applyMergePatch } from "../aip/envelope";
const wire = '{"payload":{"state":{"future":{"float":1.0,"large":9007199254740993}}}}';
afterEach(() => { vi.unstubAllGlobals(); vi.resetModules(); vi.clearAllMocks(); delete (window as unknown as Record<string, unknown>)["__TAURI_INTERNALS__"]; });
describe("production transport retains semantic numeric literals", () => {
  it("HTTP response text retains unknown float and exact integer", async () => {
    const transport = await import("../transport");
    const { canonicalJson: transportCanonical } = await import("../aip/canonical");
    transport.configureHttp("http://127.0.0.1:8787", "fixture-token");
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true, text: async () => wire }));
    const value = await transport.call("character_session_snapshot");
    expect(transportCanonical(value)).toBe(wire);
  });
  it("Tauri snapshot/resume/recent commands receive JSON text before WebView numeric decoding", async () => {
    (window as unknown as Record<string, unknown>)["__TAURI_INTERNALS__"] = {};
    const transport = await import("../transport");
    const { canonicalJson: transportCanonical } = await import("../aip/canonical");
    native.invoke.mockResolvedValue(wire);
    for (const command of ["character_session_snapshot", "character_session_resume", "events_recent"]) {
      expect(transportCanonical(await transport.call(command))).toBe(wire);
      expect(native.invoke).toHaveBeenLastCalledWith(`${command}_raw`, undefined);
    }
  });
  it("Tauri live events use the same lossless raw channel", async () => {
    (window as unknown as Record<string, unknown>)["__TAURI_INTERNALS__"] = {};
    const transport = await import("../transport");
    const { canonicalJson: transportCanonical } = await import("../aip/canonical");
    native.listen.mockImplementation(async (_channel, callback) => { callback({ payload: wire }); return () => {}; });
    const handler = vi.fn();
    await transport.onEvent(handler);
    expect(native.listen.mock.calls[0]?.[0]).toBe("runtime-event-raw");
    expect(transportCanonical(handler.mock.calls[0]?.[0])).toBe(wire);
  });
  it("merge carries untouched literals and replaces a same-valued new representation", () => {
    const state = parseJsonWithNumberSources('{"unknown":1.0,"nested":{"large":9007199254740993}}');
    const patch = parseJsonWithNumberSources('{"unknown":1,"nested":{"new":2.0}}');
    const merged = applyMergePatch(state, patch);
    expect(canonicalJson(merged)).toBe('{"nested":{"large":9007199254740993,"new":2.0},"unknown":1}');
    expect(canonicalJson(state)).toBe('{"nested":{"large":9007199254740993},"unknown":1.0}');
    expect(stateHash(merged)).not.toBe(stateHash(state));
  });
  it("a null patch deletes the key and its numeric metadata, including prototype-shaped keys", () => {
    const merged = applyMergePatch(parseJsonWithNumberSources('{"unknown":1.0}'), parseJsonWithNumberSources('{"unknown":null,"__proto__":{"x":1.0}}'));
    expect(canonicalJson(merged)).toBe('{"__proto__":{"x":1.0}}');
    expect(({} as Record<string,unknown>)["x"]).toBeUndefined();
  });
});
