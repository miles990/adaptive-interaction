// Presentation command handling (pure, testable).
//
// The runtime dispatches `presentation.command` events (already governor-
// bounded and whitelist-validated on the Rust side); this module maps each
// command to what the companion window should do, and what it should honestly
// report back (`displayed` / `completed` / `unsupported` / `failed`).
// Unsupported capabilities are ACKED as unsupported — never silently dropped,
// never faked as displayed.

import type { TransientKind } from "./machine";

export type PresentationOutcome = "displayed" | "completed" | "unsupported" | "failed";

export interface CommandPlan {
  outcome: PresentationOutcome;
  detail?: string;
  /** Machine transient to inject (null = clear the current transient). */
  transient?: TransientKind | null;
  /** Bubble to show (already runtime-validated ≤200 chars). */
  bubble?: { text: string; ms: number } | null;
  /** presence-set target (needs the desktop bridge to apply). */
  presence?: boolean;
}

/** behaviorIntent → machine transient (v1: reuse existing visual states). */
const INTENT_TO_TRANSIENT: Record<string, TransientKind | null> = {
  rest: null,
  notice: "listening",
  curious: "listening",
  listen: "listening",
  think: "thinking",
  work: "acting",
  "wait-attention": "waiting-for-receipt",
  "look-at-confirmation": "requesting-consent",
  "acknowledge-briefly": "clicked",
};

/** Directly-playable animations → machine transient (undefined = no art yet). */
const ANIMATION_TO_TRANSIENT: Record<string, TransientKind | null | undefined> = {
  idle: null,
  quiet: null,
  notice: "listening",
  curious: "listening",
  listening: "listening",
  thinking: "thinking",
  working: "acting",
  waiting: "waiting-for-receipt",
  stretch: undefined,
};

const BUBBLE_MS = 8000;

export function planPresentationCommand(
  command: string,
  params: Record<string, unknown>,
  hasDesktopBridge: boolean
): CommandPlan {
  switch (command) {
    case "bubble-show": {
      const text = typeof params.message === "string" ? params.message : "";
      if (!text) return { outcome: "failed", detail: "empty message" };
      return { outcome: "displayed", bubble: { text, ms: BUBBLE_MS } };
    }
    case "state-present": {
      const intent = typeof params.behaviorIntent === "string" ? params.behaviorIntent : "";
      if (!(intent in INTENT_TO_TRANSIENT)) {
        // Runtime already whitelists; an unknown here means version skew.
        return { outcome: "unsupported", detail: `behaviorIntent ${intent} not known to this window` };
      }
      const text = typeof params.message === "string" && params.message ? params.message : null;
      return {
        outcome: "displayed",
        transient: INTENT_TO_TRANSIENT[intent],
        bubble: text ? { text, ms: BUBBLE_MS } : null,
      };
    }
    case "animation-play": {
      const name = typeof params.animation === "string" ? params.animation : "";
      const transient = ANIMATION_TO_TRANSIENT[name];
      if (transient === undefined) {
        return { outcome: "unsupported", detail: `animation ${name} has no art in this pack yet` };
      }
      return { outcome: "displayed", transient };
    }
    case "presence-set": {
      if (!hasDesktopBridge) {
        return { outcome: "unsupported", detail: "presence control needs the desktop shell" };
      }
      return { outcome: "completed", presence: Boolean(params.visible) };
    }
    case "sound-play":
      return { outcome: "unsupported", detail: "sound playback not implemented in this window yet" };
    case "speak":
      return { outcome: "unsupported", detail: "speech synthesis not implemented in this window yet" };
    case "window-adjust":
      return { outcome: "unsupported", detail: "window adjust not implemented in this window yet" };
    default:
      return { outcome: "unsupported", detail: `unknown command ${command}` };
  }
}
