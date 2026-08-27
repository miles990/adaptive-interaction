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

  it("unsupported capabilities are acked as unsupported, never displayed", () => {
    for (const cmd of ["sound-play", "speak", "window-adjust"]) {
      const plan = planPresentationCommand(cmd, {}, true);
      expect(plan.outcome).toBe("unsupported");
      expect(plan.detail).toBeTruthy();
    }
  });

  it("animation without art is unsupported; known animations display", () => {
    expect(planPresentationCommand("animation-play", { animation: "stretch" }, true).outcome).toBe(
      "unsupported"
    );
    expect(planPresentationCommand("animation-play", { animation: "thinking" }, true).outcome).toBe(
      "displayed"
    );
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
