// CPP §13（TS Gateway）：command lifecycle、acknowledged→uncertain、cancel 冪等、duplicate
// messageId、expired、generation／reconnect、adapter crash、bounded queue、payload size、
// 偽造 verified、emergency 搶占、system.text 退路、多實例去重、輸入正規化（§6）。

import { describe, expect, it } from "vitest";
import type { AdapterHost, AdapterInputEvent, CharacterAdapter, ReceiptSink } from "../character/adapter";
import { CharacterGateway, MAX_LIVE_GRANTS, type AuditEntry, type SystemTextMessage } from "../character/gateway";
import { validateCharacterManifest } from "../character/manifest";
import { ProtocolVersionError } from "../character/negotiate";
import {
  CHARACTER_INTENTS,
  CharacterInputEvent,
  CharacterIntent,
  CharacterManifest,
  CommandReceipt,
  Hello,
  IntentEnvelope,
  IntentResolution,
  LIMITS,
  Negotiate,
  PROTOCOL_VERSION,
} from "../character/protocol";

type Mode = "hold" | "complete" | "ack" | "silent" | "throw" | "accepted-only";

function fakeManifest(over: Record<string, unknown> = {}): CharacterManifest {
  const raw = {
    schemaVersion: "1.0",
    characterId: "fake",
    displayName: { en: "Fake" },
    version: "1.0.0",
    adapterKind: "in-process",
    entrypoint: { kind: "builtin", id: "text" },
    capabilities: {
      "visual.presence": { supported: true, interruptible: true, resumable: true },
      "visual.expression": { supported: true, interruptible: true, resumable: true, reducedMotionBehavior: "static" },
    },
    inputCapabilities: {
      "input.click": { supported: true },
      "input.hover": { supported: true },
      "input.drag": { supported: true },
      "input.drop": { supported: true },
      "input.text": { supported: true },
      "input.fileDrop": { supported: true },
    },
    channels: ["expression", "bubble", "com.example.character.wings", "weird"],
    intents: [...CHARACTER_INTENTS],
    ...over,
  };
  const v = validateCharacterManifest(raw);
  if (!v.ok) throw new Error(v.errors.join("; "));
  return v.manifest;
}

class FakeAdapter implements CharacterAdapter {
  manifest: CharacterManifest;
  mode: Mode = "hold";
  offerVersion: string = PROTOCOL_VERSION;
  sinks = new Map<string, ReceiptSink>();
  performed: IntentEnvelope[] = [];
  resolutions: Array<IntentResolution | undefined> = [];
  cancelled: string[] = [];
  disposed = 0;
  initialized = 0;
  host: AdapterHost | null = null;
  private listeners = new Set<(e: AdapterInputEvent) => void>();

  constructor(over: Record<string, unknown> = {}) {
    this.manifest = fakeManifest(over);
  }

  async initialize(host: AdapterHost) {
    this.host = host;
    this.initialized += 1;
  }

  negotiate(_hello: Hello): Negotiate {
    return {
      type: "negotiate",
      protocolVersion: this.offerVersion,
      characterId: this.manifest.characterId,
      manifestVersion: this.manifest.version,
      capabilities: this.manifest.capabilities,
      inputCapabilities: this.manifest.inputCapabilities,
      channels: this.manifest.channels,
      intents: this.manifest.intents,
      variants: [],
      generation: 0,
      fallbacks: this.manifest.fallbacks,
    };
  }

  show() {}
  hide() {}
  suspend() {}
  resume() {}
  reconfigure() {}

  perform(envelope: IntentEnvelope, sink: ReceiptSink, resolution?: IntentResolution) {
    this.performed.push(envelope);
    this.resolutions.push(resolution);
    this.sinks.set(envelope.messageId, sink);
    const id = envelope.messageId;
    switch (this.mode) {
      case "throw":
        throw new Error("boom");
      case "silent":
        return;
      case "accepted-only":
        sink({ messageId: id, status: "accepted" });
        return;
      case "ack":
        sink({ messageId: id, status: "accepted" });
        sink({ messageId: id, status: "acknowledged" });
        return;
      case "complete":
        sink({ messageId: id, status: "accepted" });
        sink({ messageId: id, status: "started" });
        sink({ messageId: id, status: "completed" });
        return;
      default:
        sink({ messageId: id, status: "accepted" });
        sink({ messageId: id, status: "started" });
    }
  }

  emit(messageId: string, receipt: Record<string, unknown>) {
    this.sinks.get(messageId)?.({ messageId, status: "completed", ...receipt } as never);
  }

  cancel(messageId: string) {
    this.cancelled.push(messageId);
    this.sinks.get(messageId)?.({ messageId, status: "cancelled", reason: "adapter" });
  }

  dispose() {
    this.disposed += 1;
  }

  onInput(cb: (e: AdapterInputEvent) => void) {
    this.listeners.add(cb);
    return () => {
      this.listeners.delete(cb);
    };
  }

  fire(e: AdapterInputEvent) {
    for (const cb of [...this.listeners]) cb(e);
  }
}

interface Harness {
  gw: CharacterGateway;
  receipts: CommandReceipt[];
  audits: AuditEntry[];
  systemTexts: SystemTextMessage[];
  inputs: CharacterInputEvent[];
  clock: { now: number };
  reducedMotion: { on: boolean };
}

function harness(opts: { pull?: boolean; reducedMotion?: boolean } = {}): Harness {
  const clock = { now: 1_700_000_000_000 };
  const reducedMotion = { on: opts.reducedMotion ?? false };
  const receipts: CommandReceipt[] = [];
  const audits: AuditEntry[] = [];
  const systemTexts: SystemTextMessage[] = [];
  const inputs: CharacterInputEvent[] = [];
  const gw = new CharacterGateway({
    now: () => clock.now,
    onSystemText: (m) => systemTexts.push(m),
    onReceipt: (r) => receipts.push(r),
    onAudit: (a) => audits.push(a),
    ...(opts.pull ? {} : { onInput: (e) => inputs.push(e) }),
    reducedMotion: () => reducedMotion.on,
    runtimeVersion: "0.5.0-test",
  });
  return { gw, receipts, audits, systemTexts, inputs, clock, reducedMotion };
}

let seq = 0;
function env(instanceId: string, intent: CharacterIntent, over: Partial<IntentEnvelope> = {}): IntentEnvelope {
  seq += 1;
  return {
    protocolVersion: PROTOCOL_VERSION,
    messageId: `m${seq}`,
    characterInstanceId: instanceId,
    correlationId: "corr-1",
    timestamp: "2026-09-02T00:00:00.000Z",
    intent,
    truthState: "none",
    priority: 10,
    interruptPolicy: "preempt",
    resumePolicy: "none",
    privacyClass: "internal",
    ...over,
  };
}

function statuses(receipts: CommandReceipt[], messageId: string) {
  return receipts.filter((r) => r.messageId === messageId).map((r) => r.status);
}

describe("Gateway 握手", () => {
  it("registerInstance：initialize → negotiate → ready；協商結果涵蓋 20 個 intent", async () => {
    const h = harness();
    const a = new FakeAdapter();
    const { instanceId, negotiated } = await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    expect(instanceId).toBe("a");
    expect(a.initialized).toBe(1);
    expect(Object.keys(negotiated.resolutions)).toHaveLength(20);
    expect(negotiated.generation).toBe(1);
    expect(negotiated.nonSafetyChannels).toEqual(["com.example.character.wings"]);
    expect(negotiated.ignoredChannels).toEqual(["weird"]);
    expect(h.gw.getInstance("a")?.state).toBe("ready");
  });

  it("協定 major 不同：拒絕註冊、adapter 被 dispose、記 audit", async () => {
    const h = harness();
    const a = new FakeAdapter();
    a.offerVersion = "2.0";
    await expect(h.gw.registerInstance(a, "primary-companion", { instanceId: "a" })).rejects.toBeInstanceOf(ProtocolVersionError);
    expect(a.disposed).toBe(1);
    expect(h.gw.getInstance("a")?.state).toBe("disposed");
    expect(h.audits.some((x) => x.kind === "handshake-rejected")).toBe(true);
  });

  it("reduced motion 影響協商；renegotiate 反映變更", async () => {
    const h = harness({ reducedMotion: true });
    const a = new FakeAdapter();
    const { negotiated } = await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    expect(negotiated.reducedMotion).toBe(true);
    expect(negotiated.resolutions.notice.resolution).toBe("reduced");
    h.reducedMotion.on = false;
    const again = h.gw.renegotiate("a");
    expect(again.resolutions.notice.resolution).toBe("exact");
    expect(h.gw.getInstance("a")?.generation).toBe(1);
  });
});

describe("Command lifecycle 與回執合法性（§7）", () => {
  it("accepted → started → completed；resolution 由 Gateway 決定且只能變差", async () => {
    const h = harness();
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    const e = env("a", "notice");
    const first = h.gw.dispatch(e);
    expect(first.status).toBe("accepted");
    expect(first.resolution).toBe("exact");
    expect(first.generation).toBe(1);
    expect(a.performed).toHaveLength(1);
    expect(a.resolutions[0]).toEqual({ resolution: "exact", via: "visual.expression" });
    a.emit(e.messageId, { status: "completed", resolution: "exact" });
    expect(statuses(h.receipts, e.messageId)).toEqual(["accepted", "started", "completed"]);
    // 試圖升級 resolution 無效；降級有效
    const e2 = env("a", "notice");
    h.gw.dispatch(e2);
    a.emit(e2.messageId, { status: "completed", resolution: "reduced" });
    expect(h.receipts.find((r) => r.messageId === e2.messageId && r.status === "completed")?.resolution).toBe("reduced");
    expect(h.gw.getInstance("a")?.pendingCount).toBe(0);
  });

  it("acknowledged 不猜 completed：sweep 後記成 uncertain", async () => {
    const h = harness();
    const a = new FakeAdapter();
    a.mode = "ack";
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    const e = env("a", "work", { durationHint: { ms: 1000 } });
    h.gw.dispatch(e);
    expect(statuses(h.receipts, e.messageId)).toEqual(["accepted", "acknowledged"]);
    h.clock.now += 1000 + LIMITS.acknowledgedGraceMs - 1;
    h.gw.sweep();
    expect(statuses(h.receipts, e.messageId)).toEqual(["accepted", "acknowledged"]);
    h.clock.now += 1;
    h.gw.sweep();
    expect(statuses(h.receipts, e.messageId)).toEqual(["accepted", "acknowledged", "uncertain"]);
    // 之後 adapter 才回 completed → 丟棄並記 audit
    a.emit(e.messageId, { status: "completed" });
    expect(statuses(h.receipts, e.messageId)).toEqual(["accepted", "acknowledged", "uncertain"]);
    expect(h.audits.some((x) => x.kind === "receipt-after-terminal")).toBe(true);
  });

  it("非法順序（accepted → completed 沒有 started）被丟棄並記 audit；看門狗最後記 uncertain", async () => {
    const h = harness();
    const a = new FakeAdapter();
    a.mode = "accepted-only";
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    const e = env("a", "notice");
    h.gw.dispatch(e);
    a.emit(e.messageId, { status: "completed" });
    expect(statuses(h.receipts, e.messageId)).toEqual(["accepted"]);
    expect(h.audits.some((x) => x.kind === "illegal-receipt-transition")).toBe(true);
    h.clock.now += LIMITS.durationMaxMs + LIMITS.startedWatchdogGraceMs;
    h.gw.sweep();
    expect(statuses(h.receipts, e.messageId)).toEqual(["accepted", "uncertain"]);
  });

  it("adapter 在 perform 內沒有任何回執 → failed（adapter-silent）；安全 intent 走 system.text", async () => {
    const h = harness();
    const a = new FakeAdapter();
    a.mode = "silent";
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    const e = env("a", "blocked", { truthState: "blocked" });
    h.gw.dispatch(e);
    expect(statuses(h.receipts, e.messageId)).toEqual(["accepted", "failed"]);
    expect(h.systemTexts).toHaveLength(1);
    expect(h.systemTexts[0].reason).toBe("adapter-silent");
    expect(h.systemTexts[0].text).toBe("這個動作超出目前允許範圍，所以我沒有執行。");
  });

  it("adapter 擲例外 → 該 command failed，Gateway 不擲出", async () => {
    const h = harness();
    const a = new FakeAdapter();
    a.mode = "throw";
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    const e = env("a", "notice");
    expect(() => h.gw.dispatch(e)).not.toThrow();
    expect(statuses(h.receipts, e.messageId)).toEqual(["accepted", "failed"]);
  });

  it("started 後看門狗：超過 durationHint＋寬限沒 completed → uncertain 並要求 adapter 釋放", async () => {
    const h = harness();
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    const e = env("a", "work", { durationHint: { ms: 2000, loop: true } });
    h.gw.dispatch(e);
    h.clock.now += 2000 + LIMITS.startedWatchdogGraceMs;
    h.gw.sweep();
    expect(statuses(h.receipts, e.messageId)).toEqual(["accepted", "started", "uncertain"]);
    expect(a.cancelled).toContain(e.messageId);
  });
});

describe("cancel 冪等、去重、過期", () => {
  it("cancel：pending → cancelled{reason}；重複 cancel → alreadyTerminal 且不再發回執；未知 id 不報錯", async () => {
    const h = harness();
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    const e = env("a", "work");
    h.gw.dispatch(e);
    const r1 = h.gw.cancel(e.messageId);
    expect(r1.status).toBe("cancelled");
    expect(r1.reason).toBe("host");
    expect(a.cancelled).toEqual([e.messageId]);
    const before = h.receipts.length;
    const r2 = h.gw.cancel(e.messageId);
    expect(r2.status).toBe("cancelled");
    expect(r2.alreadyTerminal).toBe(true);
    expect(h.receipts.length).toBe(before);
    const r3 = h.gw.cancel("never-seen");
    expect(r3.alreadyTerminal).toBe(true);
    expect(statuses(h.receipts, e.messageId)).toEqual(["accepted", "started", "cancelled"]);
  });

  it("重複 messageId（環 256）→ accepted{duplicate:true}，adapter 只演一次", async () => {
    const h = harness();
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    const e = env("a", "notice");
    h.gw.dispatch(e);
    a.emit(e.messageId, { status: "completed" });
    const dup = h.gw.dispatch(e);
    expect(dup.status).toBe("accepted");
    expect(dup.duplicate).toBe(true);
    expect(a.performed).toHaveLength(1);
    // 超過 256 則之後最舊的才會被遺忘
    a.mode = "complete";
    for (let i = 0; i < LIMITS.dedupeRing; i += 1) h.gw.dispatch(env("a", "notice"));
    expect(h.gw.dispatch(e).duplicate).toBeUndefined();
  });

  it("duplicate／alreadyTerminal 回執帶原命令協商出的 resolution，不硬編 exact", async () => {
    const h = harness({ reducedMotion: true });
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    // fakeManifest 的 visual.expression 是 reducedMotionBehavior:"static" → 協商成 reduced。
    const e = env("a", "notice");
    const accepted = h.gw.dispatch(e);
    expect(accepted.resolution).toBe("reduced");
    const dup = h.gw.dispatch(e);
    expect(dup.duplicate).toBe(true);
    expect(dup.resolution).toBe("reduced");
    a.emit(e.messageId, { status: "completed" });
    const terminal = h.gw.cancel(e.messageId);
    expect(terminal.alreadyTerminal).toBe(true);
    expect(terminal.resolution).toBe("reduced");
    expect(terminal.detail).toBe("already completed");
    // 已終結之後再送同一個 messageId：一樣不會退回 exact。
    expect(h.gw.dispatch(e).resolution).toBe("reduced");
    // 未知 messageId：誠實 unsupported（沒有原命令可帶）。
    expect(h.gw.cancel("never-seen").resolution).toBe("unsupported");
  });

  it("expiresAt 已過 → expired，不派給 adapter；排隊中過期 → sweep 記 expired", async () => {
    const h = harness();
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    const past = new Date(h.clock.now - 1).toISOString();
    const r = h.gw.dispatch(env("a", "notice", { expiresAt: past }));
    expect(r.status).toBe("expired");
    expect(a.performed).toHaveLength(0);
    // 佇列中過期
    const hold = env("a", "work", { priority: 40 });
    h.gw.dispatch(hold);
    const queued = env("a", "notice", { priority: 40, interruptPolicy: "queue", expiresAt: new Date(h.clock.now + 500).toISOString() });
    h.gw.dispatch(queued);
    expect(statuses(h.receipts, queued.messageId)).toEqual(["accepted", "scheduled"]);
    h.clock.now += 1000;
    h.gw.sweep();
    expect(statuses(h.receipts, queued.messageId)).toEqual(["accepted", "scheduled", "expired"]);
  });
});

describe("crash／generation／reconnect（§7）", () => {
  it("crash：pending → uncertain、資源釋放、generation +1；舊世代回執與事件一律丟棄；安全 intent 走 system.text", async () => {
    const h = harness();
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    const emergency = env("a", "emergency", { truthState: "emergency" });
    const work = env("a", "work", { interruptPolicy: "queue" });
    h.gw.dispatch(emergency); // started（hold）
    h.gw.dispatch(work); // 排隊在 emergency 後面
    expect(statuses(h.receipts, work.messageId)).toEqual(["accepted", "scheduled"]);
    h.gw.onAdapterCrash("a", "test crash");
    expect(statuses(h.receipts, work.messageId)).toContain("uncertain");
    expect(statuses(h.receipts, emergency.messageId)).toContain("uncertain");
    expect(statuses(h.receipts, emergency.messageId)).not.toContain("completed");
    expect(a.disposed).toBe(1);
    expect(h.gw.getInstance("a")?.generation).toBe(2);
    expect(h.gw.getInstance("a")?.state).toBe("crashed");
    expect(h.systemTexts.map((s) => s.intent)).toEqual(["emergency"]);
    expect(h.systemTexts[0].text).toBe("緊急停止中");
    // 舊 sink 的回執被丟
    const before = h.receipts.length;
    a.emit(emergency.messageId, { status: "completed" });
    expect(h.receipts.length).toBe(before);
    expect(h.audits.some((x) => x.kind === "stale-generation-receipt")).toBe(true);
    // 舊世代事件被丟
    a.fire({ kind: "character.clicked", payload: { x: 1, y: 1 }, generation: 1 });
    expect(h.inputs).toHaveLength(0);
    // crash 後派送：安全走 system.text、非安全 unsupported
    const r = h.gw.dispatch(env("a", "notice"));
    expect(r.status).toBe("unsupported");
    const b = h.gw.dispatch(env("a", "blocked", { truthState: "blocked", messageId: "blocked-after-crash" }));
    expect(b.status).toBe("unsupported");
    expect(h.systemTexts.map((s) => s.intent)).toEqual(["emergency", "blocked"]);
  });

  it("reattach：重新 initialize＋hello，新世代可用；舊 sink 仍被拒", async () => {
    const h = harness();
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    const old = env("a", "work");
    h.gw.dispatch(old);
    h.gw.onAdapterCrash("a");
    const fresh = new FakeAdapter();
    const n = await h.gw.reattach("a", fresh);
    expect(n.generation).toBe(2);
    expect(fresh.initialized).toBe(1);
    expect(h.gw.getInstance("a")?.state).toBe("ready");
    const e = env("a", "notice");
    const r = h.gw.dispatch(e);
    expect(r.generation).toBe(2);
    a.emit(old.messageId, { status: "completed" });
    expect(statuses(h.receipts, old.messageId)).toEqual(["accepted", "started", "uncertain"]);
    fresh.emit(e.messageId, { status: "completed" });
    expect(statuses(h.receipts, e.messageId)).toEqual(["accepted", "started", "completed"]);
  });

  it("heartbeat：45 s 無訊息視為斷線（reconnecting）", async () => {
    const h = harness();
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a", heartbeat: true });
    h.clock.now += 30_000;
    h.gw.heartbeat("a");
    h.clock.now += 40_000;
    h.gw.sweep();
    expect(h.gw.getInstance("a")?.state).toBe("ready");
    h.clock.now += 6_000;
    h.gw.sweep();
    expect(h.gw.getInstance("a")?.state).toBe("reconnecting");
    expect(a.disposed).toBe(1);
  });

  it("disposeInstance（goodbye）：pending 記 uncertain、世代 +1", async () => {
    const h = harness();
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    const e = env("a", "work");
    h.gw.dispatch(e);
    h.gw.disposeInstance("a");
    expect(statuses(h.receipts, e.messageId)).toEqual(["accepted", "started", "uncertain"]);
    expect(h.gw.getInstance("a")?.state).toBe("disposed");
    expect(h.gw.getInstance("a")?.generation).toBe(2);
  });
});

describe("bounded queue 與 payload 限制（§4.4／§8）", () => {
  it("pending ≤ 64：滿了先丟最舊的非安全（cancelled{queue-full}）；安全 intent 不丟", async () => {
    const h = harness();
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    const ids: string[] = [];
    for (let i = 0; i < LIMITS.maxPending; i += 1) {
      const e = env("a", "work", { priority: 10, interruptPolicy: "queue" });
      ids.push(e.messageId);
      h.gw.dispatch(e);
    }
    expect(h.gw.getInstance("a")?.pendingCount).toBe(64);
    const extra = env("a", "notice", { interruptPolicy: "queue" });
    h.gw.dispatch(extra);
    expect(h.gw.getInstance("a")?.pendingCount).toBe(64);
    expect(h.receipts.find((r) => r.messageId === ids[0] && r.status === "cancelled")?.reason).toBe("queue-full");
    // 全部是安全 intent 時：新的非安全 intent 被拒（accepted → cancelled{queue-full}），安全 intent 仍可擠掉 floor 較低者
    const h2 = harness();
    const b = new FakeAdapter();
    await h2.gw.registerInstance(b, "primary-companion", { instanceId: "b" });
    for (let i = 0; i < LIMITS.maxPending; i += 1) h2.gw.dispatch(env("b", "blocked", { truthState: "blocked", interruptPolicy: "queue" }));
    const n = env("b", "notice", { interruptPolicy: "queue" });
    h2.gw.dispatch(n);
    expect(statuses(h2.receipts, n.messageId)).toEqual(["accepted", "cancelled"]);
    expect(h2.gw.getInstance("b")?.pendingCount).toBe(64);
    const em = env("b", "emergency", { truthState: "emergency" });
    h2.gw.dispatch(em);
    expect(statuses(h2.receipts, em.messageId)).toContain("started");
    expect(h2.gw.getInstance("b")?.pendingCount).toBe(64);
  });

  it("parameters > 4 KB／字串 > 200／envelope > 64 KB → failed；安全 intent 仍走 system.text", async () => {
    const h = harness();
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    const big = h.gw.dispatch(env("a", "notice", { parameters: { blob: "x".repeat(200).repeat(1) , many: Array.from({ length: 300 }, (_, i) => `v${i}`.padEnd(15, "0")) } }));
    expect(big.status).toBe("failed");
    const longStr = h.gw.dispatch(env("a", "notice", { parameters: { s: "y".repeat(201) } }));
    expect(longStr.status).toBe("failed");
    const huge = h.gw.dispatch(
      env("a", "blocked", { truthState: "blocked", presentationHints: { channels: { x: "z".repeat(70_000) } } })
    );
    expect(huge.status).toBe("failed");
    expect(h.systemTexts.map((s) => s.intent)).toEqual(["blocked"]);
    expect(a.performed).toHaveLength(0);
    // 合法大小照常
    expect(h.gw.dispatch(env("a", "notice", { parameters: { s: "ok" } })).status).toBe("accepted");
  });
});

describe("truthState／priority 只由 Runtime 決定", () => {
  it("runtime 來源：priority = max(requested, floor)；ai 來源：truthState 強制 none、priority ≤ 50、只允許 §11 子集", async () => {
    const h = harness();
    const a = new FakeAdapter();
    a.mode = "complete";
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    h.gw.dispatch(env("a", "emergency", { priority: 5, truthState: "emergency" }));
    expect(a.performed[0].priority).toBe(100);
    h.gw.dispatch(env("a", "work", { priority: 90, truthState: "verified" }), "ai");
    expect(a.performed[1].priority).toBe(50);
    expect(a.performed[1].truthState).toBe("none");
    expect(h.audits.some((x) => x.kind === "forged-truth-state")).toBe(true);
    const forged = h.gw.dispatch(env("a", "verified-success", { truthState: "verified" }), "ai");
    expect(forged.status).toBe("unsupported");
    expect(a.performed).toHaveLength(2);
    const estop = h.gw.dispatch(env("a", "emergency", { truthState: "emergency" }), "ai");
    expect(estop.status).toBe("unsupported");
  });

  it("adapter 回執夾帶 truthState／verified 一律忽略並記 audit", async () => {
    const h = harness();
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    const e = env("a", "claim-completed", { truthState: "claimed" });
    h.gw.dispatch(e);
    a.emit(e.messageId, { status: "completed", truthState: "verified", verified: true });
    const done = h.receipts.find((r) => r.messageId === e.messageId && r.status === "completed") as unknown as Record<string, unknown>;
    expect(done).toBeDefined();
    expect(done.truthState).toBeUndefined();
    expect(done.verified).toBeUndefined();
    expect(h.audits.some((x) => x.kind === "forged-receipt-field")).toBe(true);
  });
});

describe("Mixer／搶占（§5）", () => {
  it("emergency（floor 100）搶占 interruptible=false 的 play：被搶占者 cancelled{preempted}", async () => {
    const h = harness();
    const a = new FakeAdapter({
      capabilities: {
        "visual.presence": { supported: true },
        "visual.expression": { supported: true, interruptible: false, resumable: true },
      },
    });
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    const play = env("a", "play", { priority: 30 });
    h.gw.dispatch(play);
    // wait（floor 60 < 75）無法搶占非可中斷演出 → 排隊
    const wait = env("a", "wait", { truthState: "queued" });
    h.gw.dispatch(wait);
    expect(statuses(h.receipts, wait.messageId)).toEqual(["accepted", "scheduled"]);
    // emergency 可以
    const em = env("a", "emergency", { truthState: "emergency" });
    h.gw.dispatch(em);
    const cancelled = h.receipts.find((r) => r.messageId === play.messageId && r.status === "cancelled");
    expect(cancelled?.reason).toBe("preempted");
    expect(a.cancelled).toContain(play.messageId);
    expect(statuses(h.receipts, em.messageId)).toEqual(["accepted", "started"]);
    // emergency 完成後，排隊的 wait 才開始
    a.emit(em.messageId, { status: "completed" });
    expect(statuses(h.receipts, wait.messageId)).toEqual(["accepted", "scheduled", "started"]);
  });

  it("custom channel 不影響安全搶占；同優先度不搶占而排隊", async () => {
    const h = harness();
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    const p1 = env("a", "work", { priority: 40, presentationHints: { channels: { "com.example.character.wings": {} } } });
    h.gw.dispatch(p1);
    const p2 = env("a", "notice", { priority: 40, presentationHints: { channels: { "com.example.character.wings": {} } } });
    h.gw.dispatch(p2);
    expect(statuses(h.receipts, p2.messageId)).toEqual(["accepted", "scheduled"]);
    const em = env("a", "emergency", { truthState: "emergency" });
    h.gw.dispatch(em);
    expect(statuses(h.receipts, em.messageId)).toEqual(["accepted", "started"]);
  });

  it("resume-previous：安全演出結束後恢復被搶占的演出（新 messageId、source resume）", async () => {
    const h = harness();
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    const play = env("a", "play", { priority: 30, durationHint: { ms: 5000, loop: true } });
    h.gw.dispatch(play);
    const em = env("a", "emergency", { truthState: "emergency", resumePolicy: "resume-previous" });
    h.gw.dispatch(em);
    expect(a.performed).toHaveLength(2);
    a.emit(em.messageId, { status: "completed" });
    expect(a.performed).toHaveLength(3);
    const resumed = a.performed[2];
    expect(resumed.intent).toBe("play");
    expect(resumed.messageId).not.toBe(play.messageId);
    expect(resumed.messageId.startsWith(`${play.messageId}~r`)).toBe(true);
    expect(resumed.priority).toBe(30);
  });

  it("return-idle：結束後派 idle；drop-if-busy → cancelled{busy}；merge 同 intent＋同 correlation → cancelled{merged}（沒演過就不能說 completed）", async () => {
    const h = harness();
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    const w = env("a", "work", { priority: 40 });
    h.gw.dispatch(w);
    const drop = env("a", "notice", { priority: 40, interruptPolicy: "drop-if-busy" });
    h.gw.dispatch(drop);
    expect(h.receipts.find((r) => r.messageId === drop.messageId && r.status === "cancelled")?.reason).toBe("busy");
    const merge = env("a", "work", { priority: 40, interruptPolicy: "merge" });
    h.gw.dispatch(merge);
    // 併入既有演出：adapter 從沒收到它 → 只能是 cancelled{merged}（與 Rust 權威端一致）。
    expect(statuses(h.receipts, merge.messageId)).toEqual(["accepted", "cancelled"]);
    const merged = h.receipts.find((r) => r.messageId === merge.messageId && r.status === "cancelled");
    expect(merged?.reason).toBe("merged");
    expect(merged?.detail).toBe(`merged into ${w.messageId}`);
    expect(h.receipts.some((r) => r.messageId === merge.messageId && r.status === "completed")).toBe(false);
    expect(a.performed).toHaveLength(1);
    // 同 intent 但不同 correlation：不是同一件事 → 排隊（scheduled），不是合併。
    const other = env("a", "work", { priority: 40, interruptPolicy: "merge", correlationId: "corr-2" });
    h.gw.dispatch(other);
    expect(statuses(h.receipts, other.messageId)).toEqual(["accepted", "scheduled"]);
    expect(a.performed).toHaveLength(1);
    const blocked = env("a", "blocked", { truthState: "blocked", resumePolicy: "return-idle" });
    h.gw.dispatch(blocked);
    a.emit(blocked.messageId, { status: "completed" });
    expect(a.performed.slice(-1)[0]?.intent).toBe("idle");
    expect(a.performed.slice(-1)[0]?.priority).toBe(0);
  });
});

describe("system.text 退路與多實例", () => {
  it("零能力角色：安全 intent → system.text（accepted＋completed substituted）；非安全 → unsupported", async () => {
    const h = harness();
    const a = new FakeAdapter({ capabilities: {}, intents: [] });
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    const em = env("a", "emergency", { truthState: "emergency" });
    const r = h.gw.dispatch(em);
    expect(r.status).toBe("accepted");
    expect(r.resolution).toBe("substituted");
    expect(statuses(h.receipts, em.messageId)).toEqual(["accepted", "completed"]);
    expect(h.systemTexts).toHaveLength(1);
    expect(h.systemTexts[0]).toMatchObject({ intent: "emergency", reason: "negotiated", text: "緊急停止中", marker: "none" });
    expect(a.performed).toHaveLength(0);
    expect(h.gw.dispatch(env("a", "play")).status).toBe("unsupported");
    const v = env("a", "verified-success", { truthState: "verified" });
    h.gw.dispatch(v);
    expect(h.systemTexts.slice(-1)[0]).toMatchObject({ marker: "verified", text: "做完了，也確認過結果。" });
    const c = env("a", "claim-completed", { truthState: "claimed" });
    h.gw.dispatch(c);
    expect(h.systemTexts.slice(-1)[0]).toMatchObject({ marker: "none", text: "做完了。" });
  });

  it("adapter 回 failed 的安全 intent 自動改走 system.text", async () => {
    const h = harness();
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    const e = env("a", "offline", { truthState: "offline" });
    h.gw.dispatch(e);
    a.emit(e.messageId, { status: "failed", detail: "renderer died" });
    expect(h.systemTexts).toHaveLength(1);
    expect(h.systemTexts[0].reason).toBe("adapter-failed");
    expect(h.receipts.find((r) => r.messageId === e.messageId && r.status === "failed")?.resolution).toBe("failed");
  });

  // character-protocol-038：unsupported／cancelled／uncertain／expired 都是協定允許的終態，
  // adapter 不能用它們把安全訊息吞掉（呈現層對安全訊息沒有否決權）。
  it.each(["unsupported", "cancelled", "uncertain", "expired"] as const)(
    "adapter 用 %s 結束安全 intent 一樣改走 system.text",
    async (status) => {
      const h = harness();
      const a = new FakeAdapter();
      // unsupported／expired 只在 accepted／scheduled 之後合法（§7），所以停在 accepted。
      a.mode = "accepted-only";
      await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
      const e = env("a", "emergency", { truthState: "emergency" });
      h.gw.dispatch(e);
      expect(h.systemTexts).toHaveLength(0);
      a.emit(e.messageId, { status, detail: "adapter chose a terminal status" });
      expect(h.systemTexts).toHaveLength(1);
      expect(h.systemTexts[0]).toMatchObject({
        intent: "emergency",
        truthState: "emergency",
        reason: "not-presented",
      });
      expect(h.receipts.find((r) => r.messageId === e.messageId && r.status === status)).toBeTruthy();
    }
  );

  it("completed 的安全 intent 與非安全 intent 的終態都不補 system.text", async () => {
    const h = harness();
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    const safe = env("a", "emergency", { truthState: "emergency" });
    h.gw.dispatch(safe);
    a.emit(safe.messageId, { status: "started" });
    a.emit(safe.messageId, { status: "completed" });
    expect(h.systemTexts).toHaveLength(0);
    const plain = env("a", "notice");
    h.gw.dispatch(plain);
    a.emit(plain.messageId, { status: "unsupported" });
    expect(h.systemTexts).toHaveLength(0);
  });

  it("沒有任何實例時安全 intent 仍不遺失", () => {
    const h = harness();
    const r = h.gw.dispatch(env("ghost", "blocked", { truthState: "blocked" }));
    expect(r.status).toBe("unsupported");
    expect(h.systemTexts).toHaveLength(1);
    expect(h.systemTexts[0].reason).toBe("no-instance");
  });

  it("broadcast 到多個零能力實例：system.text 只出現一次；非安全只送 companion/familiar/worker", async () => {
    const h = harness();
    await h.gw.registerInstance(new FakeAdapter({ capabilities: {}, intents: [] }), "primary-companion", { instanceId: "a" });
    await h.gw.registerInstance(new FakeAdapter({ capabilities: {}, intents: [] }), "familiar", { instanceId: "b" });
    const obs = new FakeAdapter();
    obs.mode = "complete";
    await h.gw.registerInstance(obs, "observer", { instanceId: "c" });
    const rs = h.gw.broadcast(env("all", "emergency", { truthState: "emergency" }));
    expect(rs).toHaveLength(3);
    expect(h.systemTexts).toHaveLength(1);
    expect(h.audits.filter((x) => x.kind === "system-text-deduped")).toHaveLength(1);
    const rs2 = h.gw.broadcast(env("all", "notice"));
    expect(rs2).toHaveLength(2);
    expect(obs.performed.map((p) => p.intent)).toEqual(["emergency"]);
  });
});

describe("輸入事件正規化（§6）", () => {
  it("hover ≤ 4/s；dragged 合併 ≤ 10/s 且只帶 8 px 量化座標；不保留原始軌跡／絕對座標", async () => {
    const h = harness();
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    for (let i = 0; i < 10; i += 1) a.fire({ kind: "character.hover-entered", payload: { x: 3, y: 4 } });
    expect(h.inputs.filter((e) => e.kind === "character.hover-entered")).toHaveLength(1);
    h.clock.now += 250;
    a.fire({ kind: "character.hover-left", payload: {} });
    expect(h.inputs.filter((e) => e.kind.startsWith("character.hover"))).toHaveLength(2);

    a.fire({ kind: "character.dragged", payload: { x: 13, y: 21, screenX: 1999, screenY: 888, path: [[1, 2], [3, 4]] } });
    a.fire({ kind: "character.dragged", payload: { x: 29, y: 30 } });
    a.fire({ kind: "character.dragged", payload: { x: 44, y: 45 } });
    const drags = () => h.inputs.filter((e) => e.kind === "character.dragged");
    expect(drags()).toHaveLength(1);
    expect(drags()[0].payload).toEqual({ x: 16, y: 24 });
    expect(JSON.stringify(drags()[0])).not.toContain("screenX");
    expect(JSON.stringify(drags()[0])).not.toContain("path");
    h.clock.now += 100;
    h.gw.sweep();
    expect(drags()).toHaveLength(2);
    expect(drags()[1].payload).toEqual({ x: 48, y: 48 }); // 44→48、45→48（8 px 網格四捨五入）
    const last = h.inputs.slice(-1)[0]!;
    expect(last.protocolVersion).toBe(PROTOCOL_VERSION);
    expect(last.characterInstanceId).toBe("a");
    expect(last.generation).toBe(1);
    expect(last.privacyClass).toBe("internal");
    expect(typeof last.eventId).toBe("string");
  });

  it("佇列上限 64（拉取模式）：滿了丟最舊的非安全事件", async () => {
    const h = harness({ pull: true });
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    a.fire({ kind: "character.dismissed" });
    for (let i = 0; i < 40; i += 1) {
      h.clock.now += 30; // 避開 50/s 速率限制
      a.fire({ kind: "character.clicked", payload: { x: i, y: i } });
    }
    for (let i = 0; i < 40; i += 1) {
      h.clock.now += 30;
      a.fire({ kind: "character.double-clicked", payload: { x: i, y: i } });
    }
    expect(h.gw.inputQueueSize()).toBe(LIMITS.inputQueue);
    expect(h.audits.filter((x) => x.kind === "input-queue-full").length).toBeGreaterThan(0);
    const drained = h.gw.drainInput();
    expect(drained).toHaveLength(64);
    expect(drained[0].event.kind).toBe("character.dismissed");
    expect(h.gw.inputQueueSize()).toBe(0);
  });

  it("file-drop 只帶 metadata＋短效 grant（≤ 10 分鐘、可撤銷、只授權該檔）", async () => {
    const h = harness();
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    a.fire({
      kind: "character.file-dropped",
      payload: {
        files: [
          { name: "/Users/victim/Documents/secret.pdf", mediaType: "application/pdf", bytes: 1234, path: "/Users/victim/Documents/secret.pdf" },
          { name: "C:\\x\\y.png", mediaType: "not a type", bytes: -5 },
        ],
        grantTtlMs: 60 * 60_000,
      },
    });
    // run-2 character-protocol-028：這裡曾釘住 `{ files:[…] }` 一則事件的形狀——那正是 Runtime
    // 正規化器（README §6 扁平鍵）會以 invalid-payload 丟掉的形狀。現在一檔一事件、payload 只有
    // { name, mediaType, bytes, readableScope, grantId, expiresAt }。
    expect(h.inputs).toHaveLength(2);
    for (const ev of h.inputs) {
      expect(ev.kind).toBe("character.file-dropped");
      expect(ev.privacyClass).toBe("personal");
      expect(Object.keys(ev.payload).sort()).toEqual(["bytes", "expiresAt", "grantId", "mediaType", "name", "readableScope"]);
    }
    const files = h.inputs.map((ev) => ev.payload);
    expect(files[0].name).toBe("secret.pdf");
    expect(files[0].readableScope).toBe("file");
    expect(files[0].path).toBeUndefined();
    expect(JSON.stringify(h.inputs)).not.toContain("/Users/");
    expect(files[1]).toMatchObject({ name: "y.png", mediaType: "application/octet-stream", bytes: 0 });
    const grantId = files[0].grantId as string;
    const expires = Date.parse(files[0].expiresAt as string);
    expect(expires - h.clock.now).toBeLessThanOrEqual(LIMITS.fileGrantMaxMs);
    expect(h.gw.isGrantActive(grantId)).toBe(true);
    expect(h.gw.isGrantActive(grantId, h.clock.now + LIMITS.fileGrantMaxMs + 1)).toBe(false);
    expect(h.gw.revokeGrant(grantId)).toBe(true);
    expect(h.gw.isGrantActive(grantId)).toBe(false);
    expect(h.gw.revokeGrant(grantId)).toBe(false);
    // grant 表有界：灌 40 × 16 個檔案，存活 grant 不超過 MAX_LIVE_GRANTS，最舊者被撤
    const firstSurvivor = h.gw.listGrants()[0]?.grantId;
    for (let i = 0; i < 40; i += 1) {
      h.clock.now += 30;
      a.fire({ kind: "character.file-dropped", payload: { files: Array.from({ length: 16 }, (_, k) => ({ name: `f${i}-${k}.txt`, mediaType: "text/plain", bytes: 1 })) } });
    }
    expect(h.gw.listGrants().length).toBeLessThanOrEqual(MAX_LIVE_GRANTS);
    expect(h.gw.listGrants().some((g) => g.grantId === firstSurvivor)).toBe(false);
    expect(h.audits.some((x) => x.kind === "grant-evicted")).toBe(true);
  });

  it("角色過濾：observer／notification-only 不送輸入；未知 kind 丟棄；action-requested 只轉送 action id", async () => {
    const h = harness();
    const obs = new FakeAdapter();
    await h.gw.registerInstance(obs, "observer", { instanceId: "o" });
    obs.fire({ kind: "character.clicked", payload: { x: 0, y: 0 } });
    expect(h.inputs).toHaveLength(0);
    expect(h.audits.some((x) => x.kind === "role-filtered-event")).toBe(true);
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    a.fire({ kind: "character.human-verified" as never, payload: { verified: true } });
    expect(h.inputs).toHaveLength(0);
    expect(h.audits.some((x) => x.kind === "unknown-event-kind")).toBe(true);
    a.fire({ kind: "character.action-requested", payload: { action: "pause-proactive", token: "steal-me" } });
    expect(h.inputs).toHaveLength(1);
    expect(h.inputs[0].payload).toEqual({ action: "pause-proactive" });
    a.fire({ kind: "character.action-requested", payload: { action: "rm -rf /" } });
    expect(h.inputs).toHaveLength(1);
    a.fire({ kind: "character.text-submitted", payload: { text: "x".repeat(2500) } });
    expect(h.inputs[1].payload).toEqual({ text: "x".repeat(2000), truncated: true });
    expect(h.inputs[1].privacyClass).toBe("personal");
    a.fire({ kind: "character.visibility-changed", payload: { visible: "yes" } });
    expect(h.inputs[2].payload).toEqual({ visible: false });
  });

  it("每 adapter ≤ 50 則/s：超過丟棄並記 rate-limited", async () => {
    const h = harness();
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    for (let i = 0; i < 80; i += 1) a.fire({ kind: "character.clicked", payload: { x: 0, y: 0 } });
    expect(h.inputs).toHaveLength(50);
    expect(h.audits.filter((x) => x.kind === "rate-limited")).toHaveLength(1);
    h.clock.now += 1000;
    a.fire({ kind: "character.clicked", payload: { x: 0, y: 0 } });
    expect(h.inputs).toHaveLength(51);
  });
});

describe("AI 請求的 wait／ask 會被換成非安全 intent（與 Rust ai_safe_substitute 一致）", () => {
  it("wait→think、ask→notice，priority 上限 50，原意圖留在 variant 提示", async () => {
    const h = harness();
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    const first = env("a", "wait", { priority: 90, truthState: "none" });
    h.gw.dispatch(first, "ai");
    // 第一個演出佔住 expression channel；先讓它完成，第二個才不會被 mixer 排隊。
    a.emit(first.messageId, { status: "completed", resolution: "exact" });
    h.gw.dispatch(env("a", "ask", { priority: 10, truthState: "none" }), "ai");
    const seen = a.performed.map((e) => `${e.intent}:${e.priority}:${e.presentationHints?.variant ?? ""}`);
    expect(seen).toEqual(["think:50:wait", "notice:10:ask"]);
  });
});

// ---------------------------------------------------------------------------
// 對抗審查 character-protocol-037／038／040：TS 鏡射與 Rust 權威端的誠實階梯對齊。
// ---------------------------------------------------------------------------

describe("重新協商的誠實結清（README §7、character-protocol-037）", () => {
  it("renegotiate：pending 一律 uncertain，還在演的安全 intent 比照斷線補 system.text", async () => {
    const h = harness();
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    const em = env("a", "emergency", { truthState: "emergency" });
    h.gw.dispatch(em);
    const work = env("a", "work", { interruptPolicy: "queue" });
    h.gw.dispatch(work);
    expect(h.gw.getInstance("a")?.pendingCount).toBe(2);
    expect(h.systemTexts).toHaveLength(0);

    h.reducedMotion.on = true;
    h.gw.renegotiate("a");

    expect(h.gw.getInstance("a")?.pendingCount).toBe(0);
    expect(statuses(h.receipts, em.messageId)).toContain("uncertain");
    expect(statuses(h.receipts, work.messageId)).toContain("uncertain");
    // 安全 intent 沒演完 → 一定要有 system.text；非安全的不補。
    expect(h.systemTexts.map((m) => m.messageId)).toEqual([em.messageId]);
    expect(h.systemTexts[0].intent).toBe("emergency");
    expect(h.systemTexts[0].truthState).toBe("emergency");
    expect(h.audits.some((x) => x.kind === "renegotiate-pending-uncertain")).toBe(true);
    // 世代不變（TS 端的既有約定），但舊 pending 不得留在新協商下以舊 resolution 回 completed。
    expect(h.gw.getInstance("a")?.generation).toBe(1);
    a.emit(em.messageId, { status: "completed", resolution: "exact" });
    expect(statuses(h.receipts, em.messageId).filter((s) => s === "completed")).toHaveLength(0);
  });
});

describe("pending 佇列滿時安全訊息不得消失（character-protocol-038）", () => {
  it("佇列被安全 intent 塞滿：新的安全 intent 仍補 system.text，不被 cancelled{queue-full} 吞掉", async () => {
    const h = harness();
    const a = new FakeAdapter();
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    for (let i = 0; i < LIMITS.maxPending; i += 1) {
      h.gw.dispatch(env("a", "emergency", { truthState: "emergency", interruptPolicy: "queue" }));
    }
    expect(h.gw.getInstance("a")?.pendingCount).toBe(LIMITS.maxPending);
    const before = h.systemTexts.length;

    const blocked = env("a", "blocked", { truthState: "blocked", interruptPolicy: "queue" });
    h.gw.dispatch(blocked);

    expect(statuses(h.receipts, blocked.messageId)).toEqual(["accepted", "cancelled"]);
    expect(h.receipts.find((r) => r.messageId === blocked.messageId && r.status === "cancelled")?.reason).toBe("queue-full");
    expect(h.gw.getInstance("a")?.pendingCount).toBe(LIMITS.maxPending);
    expect(h.systemTexts).toHaveLength(before + 1);
    const fallback = h.systemTexts[h.systemTexts.length - 1];
    expect(fallback.messageId).toBe(blocked.messageId);
    expect(fallback.intent).toBe("blocked");
    expect(fallback.reason).toBe("not-presented");
    // 非安全 intent 撞滿佇列時維持原本的誠實拒絕（不無故製造 system.text）。
    const notice = env("a", "notice", { interruptPolicy: "queue" });
    h.gw.dispatch(notice);
    expect(statuses(h.receipts, notice.messageId)).toEqual(["accepted", "cancelled"]);
    expect(h.systemTexts).toHaveLength(before + 1);
  });
});

describe("輸入能力宣告即契約（character-protocol-040）", () => {
  it("宣告「不接收任何輸入」的角色：所有輸入事件被丟棄並記 audit，不鑄出 file-drop grant", async () => {
    const h = harness();
    const a = new FakeAdapter({ inputCapabilities: {} });
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    a.fire({ kind: "character.text-submitted", payload: { text: "hi" } });
    a.fire({ kind: "character.file-dropped", payload: { files: [{ name: "a.png", mediaType: "image/png", bytes: 10 }] } });
    a.fire({ kind: "character.action-requested", payload: { action: "pause-proactive" } });
    a.fire({ kind: "character.clicked", payload: { x: 1, y: 2 } });
    a.fire({ kind: "character.dragged", payload: { x: 1, y: 2 } });
    expect(h.inputs).toHaveLength(0);
    expect(h.gw.listGrants()).toHaveLength(0);
    expect(h.audits.filter((x) => x.kind === "input-capability-not-declared")).toHaveLength(5);
  });

  it("只宣告 input.text：文字進得來，點擊／hover 被擋", async () => {
    const h = harness();
    const a = new FakeAdapter({ inputCapabilities: { "input.text": { supported: true } } });
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    a.fire({ kind: "character.text-submitted", payload: { text: "hi" } });
    a.fire({ kind: "character.clicked", payload: {} });
    a.fire({ kind: "character.hover-entered", payload: {} });
    expect(h.inputs.map((e) => e.kind)).toEqual(["character.text-submitted"]);
    expect(h.audits.filter((x) => x.kind === "input-capability-not-declared").map((x) => x.detail)).toEqual([
      "character.clicked needs input.click",
      "character.hover-entered needs input.hover",
    ]);
  });

  it("dismissed／visibility-changed 是 host 表面的生命週期事件，不需要宣告輸入能力", async () => {
    const h = harness();
    const a = new FakeAdapter({ inputCapabilities: {} });
    await h.gw.registerInstance(a, "primary-companion", { instanceId: "a" });
    a.fire({ kind: "character.dismissed" });
    a.fire({ kind: "character.visibility-changed", payload: { visible: true } });
    expect(h.inputs.map((e) => e.kind)).toEqual(["character.dismissed", "character.visibility-changed"]);
  });
});
