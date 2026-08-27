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
  /** For `performing`: the whitelisted animation to play. */
  animation?: string;
  /** Bubble to show (already runtime-validated ≤200 chars). */
  bubble?: { text: string; ms: number } | null;
  /** presence-set target (needs the desktop bridge to apply). */
  presence?: boolean;
}

/** behaviorIntent → machine transient（v2 packs 有專屬美術；v1 靠 fallback）. */
const INTENT_TO_TRANSIENT: Record<string, { kind: TransientKind; animation?: string } | null> = {
  rest: null,
  notice: { kind: "performing", animation: "notice" },
  curious: { kind: "performing", animation: "curious" },
  listen: { kind: "listening" },
  think: { kind: "thinking" },
  work: { kind: "acting" },
  "wait-attention": { kind: "waiting-for-receipt" },
  "look-at-confirmation": { kind: "requesting-consent" },
  "acknowledge-briefly": { kind: "clicked" },
};

/** 可直接點播的動畫 → pack 內美術名（undefined = 本視窗不支援）。 */
const PLAYABLE_ART: Record<string, string | null | undefined> = {
  idle: null, // 回到待機（清除 transient）
  quiet: null,
  notice: "notice",
  curious: "curious",
  listening: "listening",
  thinking: "thinking",
  working: "act",
  waiting: "waiting",
  stretch: "stretch",
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
      const mapped = INTENT_TO_TRANSIENT[intent];
      const text = typeof params.message === "string" && params.message ? params.message : null;
      return {
        outcome: "displayed",
        transient: mapped === null ? null : mapped.kind,
        animation: mapped?.animation,
        bubble: text ? { text, ms: BUBBLE_MS } : null,
      };
    }
    case "animation-play": {
      const name = typeof params.animation === "string" ? params.animation : "";
      const art = PLAYABLE_ART[name];
      if (art === undefined) {
        return { outcome: "unsupported", detail: `animation ${name} has no art in this window` };
      }
      if (art === null) return { outcome: "displayed", transient: null };
      return { outcome: "displayed", transient: "performing", animation: art };
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
