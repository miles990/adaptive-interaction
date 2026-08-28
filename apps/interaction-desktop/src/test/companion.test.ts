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
import { transientCompetition } from "../companion/machine";
import { validateManifest } from "../companion/renderer";
import { EXPRESSIONS } from "../companion/rig/expressions";
import { planPresentationCommand } from "../companion/presentationCommands";

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

// ---------------------------------------------------------------------------
// v0.5：硬體／提供者事件映射、等優先競爭、presentation cancel 清場。
// ---------------------------------------------------------------------------

describe("硬體／提供者事件 → 角色演出", () => {
  const provider = (state: string) => ({
    eventType: "provider.state-changed",
    payload: { providerId: "device.esp32", state },
  });

  it("上線／配對 → device-hello（右耳亮＋看向）", () => {
    for (const state of ["available", "paired"]) {
      expect(mapRuntimeEvent(provider(state))).toMatchObject({
        type: "transient",
        kind: "performing",
        animation: "device-hello",
      });
    }
  });

  it("斷線／撤銷 → device-lost（耳朵下垂），其他狀態不亂演", () => {
    for (const state of ["disconnected", "revoked"]) {
      expect(mapRuntimeEvent(provider(state))).toMatchObject({
        kind: "performing",
        animation: "device-lost",
      });
    }
    expect(mapRuntimeEvent(provider("degraded"))).toBeNull();
    expect(mapRuntimeEvent(provider(""))).toBeNull();
  });

  it("非 desktop-pet 的 action.dispatched → operate-tool；acknowledged → 短點頭", () => {
    const device = (eventType: string) => ({
      eventType,
      payload: { actuatorId: "device.esp32.led", actionId: "a-1" },
    });
    expect(mapRuntimeEvent(device("action.dispatched"))).toMatchObject({
      kind: "performing",
      animation: "operate-tool",
    });
    expect(mapRuntimeEvent(device("action.acknowledged"))).toMatchObject({
      kind: "performing",
      animation: "ack-nod",
    });
    // desktop-pet 自己的動作維持原本的工作/等待語意。
    const pet = (eventType: string) => ({
      eventType,
      payload: { actuatorId: "companion.state.present" },
    });
    expect(mapRuntimeEvent(pet("action.dispatched"))).toMatchObject({ kind: "acting" });
    expect(mapRuntimeEvent(pet("action.acknowledged"))).toMatchObject({
      kind: "waiting-for-receipt",
    });
  });

  it("acknowledged 的短點頭沒有綠勾也沒有慶祝粒子（acknowledged ≠ completed）", () => {
    const nod = EXPRESSIONS["ack-nod"];
    expect(nod).toBeTruthy();
    expect(nod.truthState ?? false).toBe(false);
    expect(nod.hold.overlay ?? "none").toBe("none");
    expect(nod.hold.particles ?? "none").toBe("none");
    for (const f of [...(nod.enter?.frames ?? []), ...(nod.loop?.frames ?? [])]) {
      expect(f.p.overlay ?? "none").not.toBe("check");
      expect(f.p.particles ?? "none").toBe("none");
    }
  });

  it("硬體演出是低優先：安全狀態立刻搶佔", () => {
    let s = reduce(initial, { type: "base", base: "idle" }, T0);
    s = reduce(s, mapRuntimeEvent({
      eventType: "action.dispatched",
      payload: { actuatorId: "device.esp32.led" },
    })!, T0);
    expect(pose(s, T0 + 10).animation).toBe("operate-tool");
    s = reduce(s, mapRuntimeEvent({ eventType: "plan.blocked", payload: {} })!, T0 + 20);
    expect(pose(s, T0 + 30).animation).toBe("blocked");
  });
});

describe("等優先事件競爭（Utility scoring）", () => {
  const t = (kind: Parameters<typeof transientCompetition>[1]["kind"], extra = {}) => ({
    kind,
    untilMs: T0 + 1_000,
    ...extra,
  });

  it("高優先留在舞台上；低優先進來就被擋", () => {
    expect(transientCompetition(t("blocked"), { kind: "succeeded" })).toBe("keep");
    expect(transientCompetition(t("performing"), { kind: "failed" })).toBe("replace");
    expect(transientCompetition(null, { kind: "listening" })).toBe("replace");
  });

  it("等優先的重複事件輸給新事件（重複只續期，不重新搶舞台）", () => {
    expect(
      transientCompetition(t("succeeded", { verified: false }), {
        kind: "succeeded",
        verified: false,
      })
    ).toBe("refresh");
    expect(transientCompetition(t("succeeded", { verified: false }), { kind: "unknown" })).toBe(
      "replace"
    );
    expect(transientCompetition(t("clicked"), { kind: "dragged" })).toBe("replace");
  });

  it("重複回報只延長顯示時間，不會重播演出", () => {
    let s = reduce(initial, { type: "base", base: "idle" }, T0);
    s = reduce(s, { type: "transient", kind: "acting" }, T0);
    const first = s.transient!.untilMs;
    s = reduce(s, { type: "transient", kind: "acting" }, T0 + 500);
    expect(s.transient!.untilMs).toBeGreaterThan(first);
    expect(s.transient!.kind).toBe("acting");
  });
});

describe("presentation cancel / clear-all", () => {
  it("取消時清掉 performing（不是只清氣泡）", () => {
    const plan = planPresentationCommand("cancel", {}, true);
    expect(plan.transient).toBeNull();
    expect(plan.bubble).toBeNull();
    let s = reduce(initial, { type: "base", base: "idle" }, T0);
    s = reduce(s, { type: "transient", kind: "performing", animation: "stretch" }, T0);
    expect(pose(s, T0 + 10).animation).toBe("stretch");
    s = reduce(s, { type: "clear-transient" }, T0 + 20);
    expect(s.transient).toBeNull();
    expect(pose(s, T0 + 30).animation).toBe("idle");
  });

  it("clear-all 也一樣清場", () => {
    expect(planPresentationCommand("clear-all", {}, true).transient).toBeNull();
  });
});
