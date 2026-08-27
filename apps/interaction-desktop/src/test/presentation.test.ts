// Presentation command handling invariants:
// unsupported is ACKED as unsupported (never faked as displayed),
// behaviorIntent maps only to non-truth visual states,
// presence control requires the desktop bridge.

import { describe, expect, it } from "vitest";
import { planPresentationCommand } from "../companion/presentationCommands";

describe("planPresentationCommand", () => {
  it("bubble-show displays the runtime-validated message", () => {
    const plan = planPresentationCommand("bubble-show", { message: "嗨" }, true);
    expect(plan.outcome).toBe("displayed");
    expect(plan.bubble?.text).toBe("嗨");
  });

  it("bubble-show with empty message fails honestly", () => {
    expect(planPresentationCommand("bubble-show", {}, true).outcome).toBe("failed");
  });

  it("state-present maps intents to transients; rest clears", () => {
    const think = planPresentationCommand("state-present", { behaviorIntent: "think" }, true);
    expect(think.outcome).toBe("displayed");
    expect(think.transient).toBe("thinking");
    const rest = planPresentationCommand("state-present", { behaviorIntent: "rest" }, true);
    expect(rest.transient).toBeNull();
  });

  it("no behaviorIntent maps to a truth state (success/blocked/failed)", () => {
    for (const intent of [
      "rest",
      "notice",
      "curious",
      "listen",
      "think",
      "work",
      "wait-attention",
      "look-at-confirmation",
      "acknowledge-briefly",
    ]) {
      const plan = planPresentationCommand("state-present", { behaviorIntent: intent }, true);
      expect(["succeeded", "blocked", "failed", "unknown"]).not.toContain(plan.transient);
    }
  });

  it("plans bounded sound, speech and companion-window effects", () => {
    expect(planPresentationCommand("sound-play", { sound: "soft-pop" }, true)).toMatchObject({
      outcome: "completed",
      sound: "soft-pop",
    });
    expect(planPresentationCommand("speak", { text: "需要確認。" }, true)).toMatchObject({
      outcome: "completed",
      speech: "需要確認。",
    });
    expect(
      planPresentationCommand("window-adjust", { x: 20, width: 240, opacity: 0.8 }, true)
    ).toMatchObject({ outcome: "completed", window: { x: 20, width: 240, opacity: 0.8 } });
    expect(planPresentationCommand("window-adjust", { x: 20 }, false).outcome).toBe("unsupported");
  });

  it("animation without art is unsupported; known animations display", () => {
    expect(
      planPresentationCommand("animation-play", { animation: "moonwalk" }, true).outcome
    ).toBe("unsupported");
    const stretch = planPresentationCommand("animation-play", { animation: "stretch" }, true);
    expect(stretch.outcome).toBe("displayed");
    expect(stretch.transient).toBe("performing");
    expect(stretch.animation).toBe("stretch");
    expect(planPresentationCommand("animation-play", { animation: "thinking" }, true).outcome).toBe(
      "displayed"
    );
    // idle/quiet 是「回到待機」：清除 transient，不是播放特定美術。
    expect(planPresentationCommand("animation-play", { animation: "idle" }, true).transient).toBeNull();
  });

  it("presence-set needs the desktop bridge", () => {
    expect(planPresentationCommand("presence-set", { visible: true }, false).outcome).toBe(
      "unsupported"
    );
    const ok = planPresentationCommand("presence-set", { visible: true }, true);
    expect(ok.outcome).toBe("completed");
    expect(ok.presence).toBe(true);
  });

  it("unknown commands are unsupported (version skew stays honest)", () => {
    expect(planPresentationCommand("teleport", {}, true).outcome).toBe("unsupported");
  });
});
