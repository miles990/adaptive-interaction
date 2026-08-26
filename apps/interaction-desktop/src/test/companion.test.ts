// Companion state machine honesty invariants (spec §11.3 + hard rules):
// completed ≠ verified, unknown never plays success, emergency freezes all,
// pack fallback never upgrades a safety state into a celebration.

import { describe, expect, it } from "vitest";
import {
  initial,
  MachineState,
  mapRuntimeEvent,
  pose,
  reduce,
} from "../companion/machine";
import { validateManifest } from "../companion/renderer";

const T0 = 1_000_000;

function feed(state: MachineState, events: Parameters<typeof reduce>[1][], now = T0) {
  return events.reduce((s, e) => reduce(s, e, now), state);
}

describe("companion machine honesty", () => {
  it("completed without verification shows nod only — never the green check", () => {
    const s = feed({ base: "idle", transient: null }, [
      mapRuntimeEvent({ eventType: "action.completed", payload: {} })!,
    ]);
    const p = pose(s, T0 + 100);
    expect(p.animation).toBe("success");
    expect(p.frameSlice).toEqual([0, 1]); // nod frames only
  });

  it("observed (verified) unlocks the full success animation", () => {
    const s = feed({ base: "idle", transient: null }, [
      mapRuntimeEvent({ eventType: "action.observed", payload: {} })!,
    ]);
    const p = pose(s, T0 + 100);
    expect(p.animation).toBe("success");
    expect(p.frameSlice).toBeUndefined();
  });

  it("uncertain outcomes play unknown, not success", () => {
    const s = feed({ base: "idle", transient: null }, [
      mapRuntimeEvent({ eventType: "action.uncertain", payload: {} })!,
    ]);
    expect(pose(s, T0 + 100).animation).toBe("unknown");
  });

  it("definitive failure is distinct from unknown and never plays success", () => {
    const s = feed({ base: "idle", transient: null }, [
      mapRuntimeEvent({ eventType: "action.failed", payload: {} })!,
    ]);
    const p = pose(s, T0 + 100);
    expect(p.animation).not.toBe("success");
    expect(p.animation).not.toBe("unknown"); // its own pose, not "result unknown"
  });

  it("emergency stop freezes ordinary animation and outranks everything", () => {
    let s: MachineState = { base: "idle", transient: null };
    s = reduce(s, mapRuntimeEvent({ eventType: "emergency.stop", payload: {} })!, T0);
    expect(pose(s, T0).animation).toBe("emergency");
    // Ordinary transients are suppressed while stopped.
    s = reduce(s, mapRuntimeEvent({ eventType: "action.completed", payload: {} })!, T0 + 10);
    expect(pose(s, T0 + 20).animation).toBe("emergency");
    // Clearing the estop returns to idle.
    s = reduce(
      s,
      mapRuntimeEvent({ eventType: "emergency.stop", payload: { cleared: true } })!,
      T0 + 30
    );
    expect(pose(s, T0 + 40).animation).toBe("idle");
  });

  it("blocked (safety warning) outranks lower-priority displays", () => {
    let s: MachineState = { base: "idle", transient: null };
    s = reduce(s, mapRuntimeEvent({ eventType: "plan.blocked", payload: {} })!, T0);
    s = reduce(s, mapRuntimeEvent({ eventType: "action.dispatched", payload: {} })!, T0 + 10);
    expect(pose(s, T0 + 20).animation).toBe("blocked");
  });

  it("paused and quiet bases render their own low-activity states", () => {
    const paused = reduce(initial, { type: "base", base: "paused" }, T0);
    expect(pose(paused, T0).animation).toBe("paused");
    const quiet = reduce(initial, { type: "base", base: "quiet" }, T0);
    expect(pose(quiet, T0).animation).toBe("quiet");
  });

  it("transients expire back to the base state", () => {
    let s: MachineState = { base: "idle", transient: null };
    s = reduce(s, mapRuntimeEvent({ eventType: "action.completed", payload: {} })!, T0);
    expect(pose(s, T0 + 100).animation).toBe("success");
    expect(pose(s, T0 + 60_000).animation).toBe("idle");
  });
});

describe("character pack validation", () => {
  const valid = {
    schemaVersion: "1.0",
    kind: "character-pack",
    id: "shu-standard",
    name: { "zh-TW": "小樞" },
    frameSize: [128, 128],
    anchor: [64, 120],
    sheet: "sheet.png",
    columns: 8,
    animations: { idle: { frames: [0, 1], fps: 3, loop: true } },
  };

  it("accepts a well-formed manifest", () => {
    expect(validateManifest(valid)).toEqual([]);
  });

  it("rejects path traversal in the sheet reference", () => {
    expect(
      validateManifest({ ...valid, sheet: "../../etc/passwd" }).some((i) =>
        i.includes("plain filename")
      )
    ).toBe(true);
    expect(
      validateManifest({ ...valid, sheet: "sub/dir.png" }).some((i) =>
        i.includes("plain filename")
      )
    ).toBe(true);
  });

  it("requires an idle animation and sane fps/frames", () => {
    expect(
      validateManifest({ ...valid, animations: {} }).some((i) => i.includes("idle"))
    ).toBe(true);
    expect(
      validateManifest({
        ...valid,
        animations: { idle: { frames: [], fps: 3, loop: true } },
      }).some((i) => i.includes("empty frames"))
    ).toBe(true);
    expect(
      validateManifest({
        ...valid,
        animations: { idle: { frames: [0], fps: 500, loop: true } },
      }).some((i) => i.includes("fps"))
    ).toBe(true);
  });
});
