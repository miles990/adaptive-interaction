// CPP §12 `text`：最小文字角色 Reference Adapter，也是可信 fallback。
//
// 證明協定不依賴 rig：只宣告 visual.presence／visual.textBubble／input.click／input.text，
// 把每個 intent 演成一行文字。誠實規則：
//   - 安全 intent 用 FIXED_SAFETY_LINES（經 lines.ts），adapter 自己不能改寫。
//   - 綠勾（marker "verified"）只在 envelope.truthState === "verified" 時出現；
//     claimed 一律只寫「做完了。」。
//   - 回執 accepted → started → completed；有 durationHint.ms 時在 tick(now) 到期才 completed
//     （不自帶 timer；由 Gateway sweep 推進）。cancel 只對進行中的 messageId 有效。
//   - 可用 container（DOM）或 onRender callback（headless 測試）。

import type {
  AdapterHost,
  AdapterInputEvent,
  CharacterAdapter,
  ReceiptSink,
} from "../adapter";
import { intentLine, type IntentLine } from "../lines";
import { presentedIntent } from "../negotiate";
import { validateCharacterManifest } from "../manifest";
import {
  CHARACTER_INTENTS,
  CharacterManifest,
  Hello,
  IntentEnvelope,
  IntentResolution,
  Negotiate,
  PROTOCOL_VERSION,
} from "../protocol";

export interface RenderedLine extends IntentLine {
  messageId: string;
  intent: IntentEnvelope["intent"];
  truthState: IntentEnvelope["truthState"];
  /** 已由 Gateway 協商的 resolution（adapter 只轉述，不能升級）。 */
  resolution: IntentResolution["resolution"];
}

export interface TextAdapterOptions {
  container?: HTMLElement;
  onRender?: (line: RenderedLine | null) => void;
  characterId?: string;
  displayName?: Record<string, string>;
  description?: Record<string, string>;
}

/** 純資料建構 manifest（bundled plain-text manifest 也用同一份定義）。 */
export function buildTextCharacterManifest(opts: Pick<TextAdapterOptions, "characterId" | "displayName" | "description"> = {}): CharacterManifest {
  const raw = {
    schemaVersion: "1.0",
    characterId: opts.characterId ?? "plain-text",
    displayName: opts.displayName ?? { "zh-TW": "文字角色", en: "Plain Text" },
    author: "adaptive-interaction",
    description: opts.description ?? {
      "zh-TW": "最小文字角色：每個 intent 一行文字。沒有 rig、沒有動畫，也是其他角色停用或崩潰時的可信退路。",
      en: "Minimal text character: one line per intent. No rig, no animation; the trusted fallback when other characters are disabled or crash.",
    },
    version: "1.0.0",
    adapterKind: "in-process",
    entrypoint: { kind: "builtin", id: "text" },
    assets: [],
    capabilities: {
      "visual.presence": { supported: true, interruptible: true, resumable: true, reducedMotionBehavior: "unchanged", qualityLevel: "minimal" },
      "visual.textBubble": {
        supported: true,
        interruptible: true,
        resumable: true,
        maxConcurrent: 1,
        reducedMotionBehavior: "unchanged",
        qualityLevel: "minimal",
        durationRange: { minMs: 0, maxMs: 60000 },
      },
    },
    inputCapabilities: {
      "input.click": { supported: true },
      "input.text": { supported: true },
    },
    channels: ["bubble", "expression"],
    states: ["line"],
    intents: [...CHARACTER_INTENTS],
    variants: [],
    locales: ["zh-TW", "en"],
    securityRequirements: { network: false, executable: false, fileAccess: "none", audioOutput: false, microphone: false, camera: false },
    resourceLimits: { maxAssetBytes: 0, maxConcurrentCommands: 1, maxQueue: 32, maxFps: 1 },
    fallbacks: {},
    compatibility: { protocol: "1.x", runtime: ">=0.5.0" },
  };
  const v = validateCharacterManifest(raw);
  if (!v.ok) throw new Error(`text manifest invalid: ${v.errors.join("; ")}`);
  return v.manifest;
}

interface Active {
  messageId: string;
  completeAt: number | null;
  sink: ReceiptSink;
}

export class TextCharacterAdapter implements CharacterAdapter {
  readonly manifest: CharacterManifest;
  private host: AdapterHost | null = null;
  private readonly container: HTMLElement | null;
  private readonly onRender: ((line: RenderedLine | null) => void) | null;
  private lineEl: HTMLElement | null = null;
  private listeners = new Set<(e: AdapterInputEvent) => void>();
  private active: Active | null = null;
  private current: RenderedLine | null = null;
  private visible = true;
  private suspended = false;
  private disposed = false;
  private domCleanup: (() => void) | null = null;

  constructor(opts: TextAdapterOptions = {}) {
    this.manifest = buildTextCharacterManifest(opts);
    this.container = opts.container ?? null;
    this.onRender = opts.onRender ?? null;
  }

  async initialize(host: AdapterHost): Promise<void> {
    this.host = host;
    if (this.container && typeof document !== "undefined") {
      const el = document.createElement("div");
      el.setAttribute("data-cpp-text-character", this.manifest.characterId);
      el.setAttribute("role", "status");
      const line = document.createElement("span");
      line.setAttribute("data-cpp-line", "");
      el.appendChild(line);
      const onClick = () => this.emitInput({ kind: "character.clicked", payload: {} });
      el.addEventListener("click", onClick);
      this.container.appendChild(el);
      this.lineEl = line;
      this.domCleanup = () => {
        el.removeEventListener("click", onClick);
        el.remove();
      };
    }
  }

  negotiate(hello: Hello): Negotiate {
    void hello;
    return {
      type: "negotiate",
      protocolVersion: PROTOCOL_VERSION,
      characterId: this.manifest.characterId,
      manifestVersion: this.manifest.version,
      capabilities: this.manifest.capabilities,
      inputCapabilities: this.manifest.inputCapabilities,
      channels: this.manifest.channels,
      intents: this.manifest.intents,
      variants: this.manifest.variants.map((v) => v.id),
      generation: 0,
      fallbacks: this.manifest.fallbacks,
    };
  }

  show(): void {
    this.visible = true;
    this.paint();
  }

  hide(): void {
    this.visible = false;
    this.paint();
  }

  suspend(): void {
    this.suspended = true;
  }

  resume(): void {
    this.suspended = false;
    this.paint();
  }

  reconfigure(prefs: Record<string, unknown>): void {
    void prefs; // 文字角色沒有偏好；保留呼叫以符合契約
  }

  perform(envelope: IntentEnvelope, sink: ReceiptSink, resolution?: IntentResolution): void {
    if (this.disposed) {
      sink({ messageId: envelope.messageId, status: "failed", resolution: "failed", detail: "adapter disposed" });
      return;
    }
    if (resolution?.resolution === "unsupported") {
      sink({ messageId: envelope.messageId, status: "unsupported", resolution: "unsupported" });
      return;
    }
    // 被新的一則取代：舊的（若還在等 durationHint）算 cancelled。
    if (this.active && this.active.messageId !== envelope.messageId) {
      const old = this.active;
      this.active = null;
      old.sink({ messageId: old.messageId, status: "cancelled", resolution: "exact", reason: "replaced" });
    }
    sink({ messageId: envelope.messageId, status: "accepted" });
    // 安全 intent 的固定文案／安全動畫一律以 envelope.intent 為準：
    // 非安全的 viaIntent 不能改寫安全語意（呈現層沒有權限主權）。
    const intent = presentedIntent(envelope.intent, resolution?.viaIntent);
    const line = intentLine(intent, envelope.truthState, envelope.presentationHints?.message);
    this.current = {
      ...line,
      messageId: envelope.messageId,
      intent,
      truthState: envelope.truthState,
      resolution: resolution?.resolution ?? "exact",
    };
    this.paint();
    sink({ messageId: envelope.messageId, status: "started", resolution: resolution?.resolution ?? "exact" });
    const ms = envelope.durationHint?.ms;
    if (typeof ms === "number" && Number.isFinite(ms) && ms > 0 && this.host) {
      this.active = { messageId: envelope.messageId, completeAt: this.host.now() + Math.min(ms, 60_000), sink };
      return;
    }
    this.active = null;
    sink({ messageId: envelope.messageId, status: "completed", resolution: resolution?.resolution ?? "exact" });
  }

  tick(now: number): void {
    if (this.active && this.active.completeAt !== null && now >= this.active.completeAt) {
      const done = this.active;
      this.active = null;
      done.sink({ messageId: done.messageId, status: "completed" });
    }
  }

  cancel(messageId: string): void {
    if (this.active && this.active.messageId === messageId) {
      const old = this.active;
      this.active = null;
      old.sink({ messageId, status: "cancelled", reason: "cancel" });
    }
    if (this.current?.messageId === messageId) {
      this.current = null;
      this.paint();
    }
  }

  dispose(): void {
    this.disposed = true;
    this.active = null;
    this.current = null;
    this.listeners.clear();
    this.domCleanup?.();
    this.domCleanup = null;
    this.lineEl = null;
    this.onRender?.(null);
  }

  onInput(cb: (e: AdapterInputEvent) => void): () => void {
    this.listeners.add(cb);
    return () => {
      this.listeners.delete(cb);
    };
  }

  /** host 把使用者文字送進來（input.text）；adapter 只轉發，不解讀。 */
  submitText(text: string): void {
    this.emitInput({ kind: "character.text-submitted", payload: { text }, privacyClass: "personal" });
  }

  /** 測試／host 檢視目前那一行。 */
  currentLine(): RenderedLine | null {
    return this.current;
  }

  private emitInput(e: AdapterInputEvent) {
    if (this.disposed) return;
    for (const cb of [...this.listeners]) cb(e);
  }

  private paint() {
    const shown = this.visible && !this.suspended ? this.current : null;
    if (this.lineEl) {
      const text = shown ? (shown.marker === "verified" ? `✓ ${shown.text}` : shown.text) : "";
      this.lineEl.textContent = text;
      this.lineEl.setAttribute("data-marker", shown?.marker ?? "none");
      this.lineEl.setAttribute("data-intent", shown?.intent ?? "");
    }
    this.onRender?.(shown);
  }
}
