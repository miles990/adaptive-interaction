// CPP §4–§7：桌面視窗內的 in-process Character Presentation Gateway。
//
// 職責：握手／協商、priority 下限、去重（環 256）、過期、pending 上限 64（安全 intent
// 不丟）、channel mixer 與搶占（§5）、回執合法性（世代、狀態機、resolution 只能變差、
// acknowledged→uncertain）、cancel 冪等、crash→uncertain＋世代 +1、輸入事件正規化
// （§6 節流、8 px 量化、佇列 64、角色過濾、file-drop 只給 metadata＋短效 grant）。
//
// 時間全部注入（opts.now）；Gateway 自己不開 timer——host 必須每 500 ms 呼叫
// sweep(now)（LIMITS.sweepIntervalMs），看門狗、acknowledged→uncertain、拖曳合併的
// 尾端事件與 adapter.tick 都在 sweep 裡推進。任何 adapter 例外都不會擲出 Gateway。

import type {
  AdapterHost,
  AdapterInputEvent,
  AdapterReceipt,
  CharacterAdapter,
  InputMeta,
  NormalizedInputSink,
} from "./adapter";
import { intentLine } from "./lines";
import { negotiate as runNegotiate, ProtocolVersionError } from "./negotiate";
import {
  AdapterLifecycleState,
  AI_PRIORITY_CAP,
  AI_REQUESTABLE_INTENTS,
  aiSafeSubstitute,
  CHARACTER_INTENTS,
  CharacterInputEvent,
  CharacterIntent,
  CharacterRole,
  CommandReceipt,
  FileDropGrant,
  HARD_PREEMPT_FLOOR,
  Hello,
  INPUT_SILENT_ROLES,
  InputEventKind,
  IntentEnvelope,
  IntentResolution,
  isCharacterIntent,
  isInputEventKind,
  isReceiptStatus,
  isResolution,
  isSafetyIntent,
  isSemanticChannel,
  isTruthState,
  LIMITS,
  Negotiated,
  parseProtocolVersion,
  priorityFloor,
  PRIORITY_MAX,
  PrivacyClass,
  PROTOCOL_VERSION,
  ReceiptStatus,
  Resolution,
  TruthState,
} from "./protocol";

// ---------------------------------------------------------------------------
// 對外型別
// ---------------------------------------------------------------------------

/** envelope 來源：runtime（真相投影、可帶 truthState）、ai（companion.state.present，強制 none／≤50）、resume（Gateway 自己恢復被搶占者）。 */
export type EnvelopeSource = "runtime" | "ai" | "resume";

export interface SystemTextMessage {
  instanceId: string | null;
  messageId: string;
  correlationId?: string;
  intent: CharacterIntent;
  truthState: TruthState;
  text: string;
  marker: "verified" | "none";
  reason:
    | "negotiated"
    | "no-instance"
    | "adapter-failed"
    | "adapter-crashed"
    | "adapter-silent"
    | "rejected"
    /** 終態既非 completed 也非 failed（unsupported／cancelled／expired／uncertain）：沒演到，一樣要補文字。 */
    | "not-presented";
}

export interface AuditEntry {
  at: string;
  kind: string;
  instanceId?: string;
  messageId?: string;
  detail?: string;
}

export interface GatewayOptions {
  now: () => number;
  onSystemText: (message: SystemTextMessage) => void;
  onInput?: NormalizedInputSink;
  onReceipt?: (receipt: CommandReceipt) => void;
  onAudit?: (entry: AuditEntry) => void;
  onLifecycle?: (instanceId: string, state: AdapterLifecycleState, detail?: string) => void;
  runtimeVersion?: string;
  locale?: string;
  reducedMotion?: () => boolean;
  idGen?: () => string;
  log?: AdapterHost["log"];
}

export interface RegisterOptions {
  instanceId?: string;
  requires?: CharacterIntent[];
  /** 需要 heartbeat 監看（外部／遠端 adapter 包裝時）；45 s 無訊息視為斷線。 */
  heartbeat?: boolean;
}

export interface InstanceInfo {
  instanceId: string;
  characterId: string;
  role: CharacterRole;
  state: AdapterLifecycleState;
  generation: number;
  pendingCount: number;
  negotiated: Negotiated | null;
}

export interface GrantRecord extends FileDropGrant {
  instanceId: string;
  revoked: boolean;
}

// ---------------------------------------------------------------------------
// 內部型別
// ---------------------------------------------------------------------------

interface Pending {
  envelope: IntentEnvelope;
  status: ReceiptStatus;
  resolution: IntentResolution;
  effective: Resolution;
  channels: string[];
  priority: number;
  floor: number;
  safety: boolean;
  interruptible: boolean;
  resumable: boolean;
  acceptedAt: number;
  startedAt: number | null;
  acknowledgedAt: number | null;
  expiresAt: number | null;
  adapterSpoke: boolean;
  resumeOf: IntentEnvelope | null;
}

interface InputState {
  lastHoverAt: number;
  lastDragAt: number;
  pendingDrag: Record<string, unknown> | null;
  lastProximityAt: number;
  rateWindowStart: number;
  rateCount: number;
  rateLimitedAudited: boolean;
}

interface Instance {
  instanceId: string;
  adapter: CharacterAdapter;
  role: CharacterRole;
  characterId: string;
  generation: number;
  state: AdapterLifecycleState;
  negotiated: Negotiated | null;
  hello: Hello | null;
  pending: Map<string, Pending>;
  /** 去重環：記住終結狀態**與當時協商出的 resolution**（重複／已終結的回執要照實帶，不能一律 exact）。 */
  seen: Map<string, { status: ReceiptStatus; resolution: Resolution }>;
  seenOrder: string[];
  channelOwners: Map<string, string>;
  unsubscribe: (() => void) | null;
  heartbeat: boolean;
  lastSeenAt: number;
  input: InputState;
  host: AdapterHost;
}

const RESOLUTION_RANK: Record<Resolution, number> = {
  exact: 0,
  substituted: 1,
  reduced: 2,
  unsupported: 3,
  failed: 4,
};

const PRIVACY_RANK: Record<PrivacyClass, number> = { public: 0, internal: 1, personal: 2, intimate: 3 };

const DROPPABLE_INPUT_KINDS: readonly InputEventKind[] = [
  "character.hover-entered",
  "character.hover-left",
  "character.dragged",
  "character.clicked",
  "character.double-clicked",
  "character.drag-started",
  "character.toy-thrown",
];

/**
 * §6 event kind → 協商後必須具備的能力 id（對抗審查 character-protocol-040）。
 *
 * 連接頁的「可以接收：…」直接由 `manifest.inputCapabilities` 產生，所以宣告就是契約：
 * 沒協商到對應能力的輸入種類一律丟棄並記 audit，不得變成 `companion.*` 觀察或 file-drop
 * grant。`dismissed`／`visibility-changed` 是 host 表面的生命週期通知（關閉視窗、隱藏），
 * 不是角色宣稱要接收的互動，因此不設閘。
 */
const INPUT_KIND_CAPABILITY: Partial<Record<InputEventKind, string>> = {
  "character.clicked": "input.click",
  "character.double-clicked": "input.click",
  "character.action-requested": "input.click",
  "character.hover-entered": "input.hover",
  "character.hover-left": "input.hover",
  "character.drag-started": "input.drag",
  "character.dragged": "input.drag",
  "character.dropped": "input.drop",
  "character.text-submitted": "input.text",
  "character.file-dropped": "input.fileDrop",
  "character.toy-thrown": "gameplay.toys",
};

/** file-drop grant 表上限（有界；超過先撤最舊）。 */
export const MAX_LIVE_GRANTS = 256;

const LIVE_STATES: readonly AdapterLifecycleState[] = [
  "ready",
  "shown",
  "hidden",
  "suspended",
  "resumed",
  "reconfiguring",
];

function utf8Bytes(text: string): number {
  return new TextEncoder().encode(text).length;
}

function clampInt(v: unknown, min: number, max: number): number | null {
  if (typeof v !== "number" || !Number.isFinite(v)) return null;
  return Math.min(max, Math.max(min, Math.round(v)));
}

function quantize(v: unknown): number | null {
  const n = clampInt(v, -32768, 32767);
  if (n === null) return null;
  return Math.round(n / LIMITS.pointerGridPx) * LIMITS.pointerGridPx;
}

function worse(a: Resolution, b: Resolution): Resolution {
  return RESOLUTION_RANK[b] > RESOLUTION_RANK[a] ? b : a;
}

function parseIso(v: unknown): number | null {
  if (typeof v !== "string") return null;
  const t = Date.parse(v);
  return Number.isFinite(t) ? t : null;
}

/** 廣播用 messageId `<base>@<instance>` 的 base；system.text 去重以此為鍵。 */
export function baseMessageId(messageId: string): string {
  const at = messageId.indexOf("@");
  return at > 0 ? messageId.slice(0, at) : messageId;
}

/** 檢查 parameters：≤ 4 KB、字串 ≤ 200、深度 ≤ 8。回傳違規原因或 null。 */
export function parametersIssue(params: unknown): string | null {
  if (params === undefined) return null;
  if (typeof params !== "object" || params === null || Array.isArray(params)) return "parameters must be an object";
  let text: string;
  try {
    text = JSON.stringify(params);
  } catch {
    return "parameters not serializable";
  }
  if (utf8Bytes(text) > LIMITS.parametersMaxBytes) return `parameters exceed ${LIMITS.parametersMaxBytes} bytes`;
  const walk = (v: unknown, depth: number): string | null => {
    if (depth > 8) return "parameters nested too deep";
    if (typeof v === "string") return v.length > LIMITS.stringMaxChars ? `parameter string exceeds ${LIMITS.stringMaxChars} chars` : null;
    if (Array.isArray(v)) {
      for (const x of v) {
        const r = walk(x, depth + 1);
        if (r) return r;
      }
      return null;
    }
    if (typeof v === "object" && v !== null) {
      for (const x of Object.values(v)) {
        const r = walk(x, depth + 1);
        if (r) return r;
      }
    }
    return null;
  };
  return walk(params, 0);
}

// ---------------------------------------------------------------------------
// Gateway
// ---------------------------------------------------------------------------

export class CharacterGateway {
  private readonly instances = new Map<string, Instance>();
  private readonly grants = new Map<string, GrantRecord>();
  private readonly inputQueue: Array<{ event: CharacterInputEvent; meta: InputMeta }> = [];
  private readonly systemTextSeen: string[] = [];
  private idCounter = 0;
  private readonly opts: GatewayOptions;

  constructor(opts: GatewayOptions) {
    this.opts = opts;
  }

  // ---- 基礎 --------------------------------------------------------------

  private now(): number {
    return this.opts.now();
  }

  private iso(at = this.now()): string {
    return new Date(at).toISOString();
  }

  private nextId(prefix: string): string {
    if (this.opts.idGen) return this.opts.idGen();
    this.idCounter += 1;
    return `${prefix}-${this.idCounter}`;
  }

  private audit(kind: string, extra: Omit<AuditEntry, "at" | "kind"> = {}) {
    this.opts.onAudit?.({ at: this.iso(), kind, ...extra });
  }

  private lifecycle(inst: Instance, state: AdapterLifecycleState, detail?: string) {
    inst.state = state;
    this.opts.onLifecycle?.(inst.instanceId, state, detail);
  }

  private makeHost(instanceId: string): AdapterHost {
    const gateway = this;
    return {
      now: () => gateway.now(),
      reducedMotion: () => gateway.opts.reducedMotion?.() ?? false,
      locale: gateway.opts.locale ?? "zh-TW",
      log: (level, message, data) => {
        gateway.opts.log?.(level, `[${instanceId}] ${message}`, data);
      },
    };
  }

  private live(inst: Instance | undefined): inst is Instance {
    return !!inst && LIVE_STATES.includes(inst.state);
  }

  // ---- 註冊／握手 ---------------------------------------------------------

  /** 建立實例：initialize → hello → negotiate → ready。協定 major 不同擲 ProtocolVersionError。 */
  async registerInstance(
    adapter: CharacterAdapter,
    role: CharacterRole,
    opts: RegisterOptions = {}
  ): Promise<{ instanceId: string; negotiated: Negotiated }> {
    const instanceId = opts.instanceId ?? this.nextId("char");
    const existing = this.instances.get(instanceId);
    if (existing && existing.state !== "disposed") {
      throw new Error(`character instance already registered: ${instanceId}`);
    }
    const inst: Instance = {
      instanceId,
      adapter,
      role,
      characterId: adapter.manifest.characterId,
      generation: existing ? existing.generation + 1 : 1,
      state: "initializing",
      negotiated: null,
      hello: null,
      pending: new Map(),
      seen: new Map(),
      seenOrder: [],
      channelOwners: new Map(),
      unsubscribe: null,
      heartbeat: opts.heartbeat === true,
      lastSeenAt: this.now(),
      input: {
        lastHoverAt: Number.NEGATIVE_INFINITY,
        lastDragAt: Number.NEGATIVE_INFINITY,
        pendingDrag: null,
        lastProximityAt: Number.NEGATIVE_INFINITY,
        rateWindowStart: this.now(),
        rateCount: 0,
        rateLimitedAudited: false,
      },
      host: this.makeHost(instanceId),
    };
    this.instances.set(instanceId, inst);
    this.opts.onLifecycle?.(instanceId, "initializing");
    try {
      await adapter.initialize(inst.host);
    } catch (e) {
      this.instances.delete(instanceId);
      this.opts.onLifecycle?.(instanceId, "crashed", "initialize threw");
      this.audit("adapter-initialize-failed", { instanceId, detail: describe(e) });
      throw e instanceof Error ? e : new Error("adapter initialize failed");
    }
    const negotiated = this.handshake(inst, opts.requires);
    return { instanceId, negotiated };
  }

  private buildHello(inst: Instance, requires?: CharacterIntent[]): Hello {
    return {
      type: "hello",
      protocolVersion: PROTOCOL_VERSION,
      runtimeVersion: this.opts.runtimeVersion ?? "0.5.0-dev",
      characterInstanceId: inst.instanceId,
      role: inst.role,
      locale: this.opts.locale ?? "zh-TW",
      reducedMotion: this.opts.reducedMotion?.() ?? false,
      requires: requires ? [...requires] : [...CHARACTER_INTENTS],
      limits: {
        maxMessageBytes: LIMITS.maxMessageBytes,
        maxMessagesPerSecond: LIMITS.maxMessagesPerSecond,
        maxPending: LIMITS.maxPending,
      },
    };
  }

  private handshake(inst: Instance, requires?: CharacterIntent[]): Negotiated {
    this.lifecycle(inst, "negotiating");
    const hello = this.buildHello(inst, requires);
    inst.hello = hello;
    let negotiated: Negotiated;
    try {
      const offer = inst.adapter.negotiate(hello);
      negotiated = runNegotiate(hello, offer, offer.fallbacks ?? inst.adapter.manifest.fallbacks, inst.generation);
    } catch (e) {
      this.lifecycle(inst, "disposed", e instanceof ProtocolVersionError ? "protocol-version" : "negotiate threw");
      this.audit("handshake-rejected", { instanceId: inst.instanceId, detail: describe(e) });
      try {
        inst.adapter.dispose();
      } catch {
        // adapter 已壞；忽略
      }
      throw e instanceof Error ? e : new Error("negotiate failed");
    }
    inst.negotiated = negotiated;
    try {
      inst.adapter.negotiated?.(negotiated);
    } catch (e) {
      this.audit("adapter-negotiated-threw", { instanceId: inst.instanceId, detail: describe(e) });
    }
    inst.unsubscribe?.();
    inst.unsubscribe = inst.adapter.onInput((event) => this.ingestInput(inst.instanceId, event));
    inst.lastSeenAt = this.now();
    this.lifecycle(inst, "ready");
    return negotiated;
  }

  /**
   * 重新協商（例如 reduced motion 改變）；世代不變。
   *
   * README §7：pending 一律 `uncertain`，其中還在演的安全 intent 比照斷線補 `system.text`
   * ——adapter（或一次 Reduced Motion 切換）不能靠重送 negotiate 讓安全訊息無聲消失，
   * 也不能讓舊 pending 在新協商下以舊 resolution 回 `completed`。
   */
  renegotiate(instanceId: string, requires?: CharacterIntent[]): Negotiated {
    const inst = this.instances.get(instanceId);
    if (!this.live(inst)) throw new Error("instance not live");
    this.settlePendingForRenegotiate(inst, "re-negotiated");
    return this.handshake(inst, requires);
  }

  /** 重新協商前把所有 pending 以 `uncertain` 結清；安全 intent 補 `system.text`。 */
  private settlePendingForRenegotiate(inst: Instance, detail: string) {
    if (inst.pending.size === 0) return;
    const settled = inst.pending.size;
    let safety = 0;
    for (const p of [...inst.pending.values()]) {
      this.emit(inst, p.envelope.messageId, {
        status: "uncertain",
        resolution: p.effective,
        detail: detail.slice(0, LIMITS.stringMaxChars),
      });
      this.forget(inst, p, "uncertain");
      if (p.safety) {
        safety += 1;
        this.systemText(inst, p.envelope, "not-presented");
      }
    }
    inst.pending.clear();
    inst.channelOwners.clear();
    this.audit("renegotiate-pending-uncertain", {
      instanceId: inst.instanceId,
      detail: `${settled} pending → uncertain (${safety} safety → system.text)`,
    });
  }

  /** crash／斷線後重新接上（同一或新的 adapter 物件）；每次重連重新 hello。 */
  async reattach(instanceId: string, adapter?: CharacterAdapter): Promise<Negotiated> {
    const inst = this.instances.get(instanceId);
    if (!inst) throw new Error("unknown instance");
    if (inst.state !== "crashed" && inst.state !== "reconnecting" && inst.state !== "disposed") {
      throw new Error("instance is not disconnected");
    }
    if (adapter) {
      inst.adapter = adapter;
      inst.characterId = adapter.manifest.characterId;
    }
    this.lifecycle(inst, "reconnecting");
    try {
      await inst.adapter.initialize(inst.host);
    } catch (e) {
      this.lifecycle(inst, "crashed", "initialize threw");
      throw e instanceof Error ? e : new Error("adapter initialize failed");
    }
    return this.handshake(inst);
  }

  listInstances(): InstanceInfo[] {
    return [...this.instances.values()].map((i) => ({
      instanceId: i.instanceId,
      characterId: i.characterId,
      role: i.role,
      state: i.state,
      generation: i.generation,
      pendingCount: i.pending.size,
      negotiated: i.negotiated,
    }));
  }

  getInstance(instanceId: string): InstanceInfo | null {
    const inst = this.instances.get(instanceId);
    if (!inst) return null;
    return {
      instanceId: inst.instanceId,
      characterId: inst.characterId,
      role: inst.role,
      state: inst.state,
      generation: inst.generation,
      pendingCount: inst.pending.size,
      negotiated: inst.negotiated,
    };
  }

  getNegotiated(instanceId: string): Negotiated | null {
    return this.instances.get(instanceId)?.negotiated ?? null;
  }

  // ---- 生命週期 ------------------------------------------------------------

  private guarded(instanceId: string, state: AdapterLifecycleState, fn: (a: CharacterAdapter) => void): boolean {
    const inst = this.instances.get(instanceId);
    if (!this.live(inst)) return false;
    try {
      fn(inst.adapter);
    } catch (e) {
      this.onAdapterCrash(instanceId, `adapter threw during ${state}: ${describe(e)}`);
      return false;
    }
    this.lifecycle(inst, state);
    return true;
  }

  show(instanceId: string): boolean {
    return this.guarded(instanceId, "shown", (a) => a.show());
  }

  hide(instanceId: string): boolean {
    return this.guarded(instanceId, "hidden", (a) => a.hide());
  }

  suspend(instanceId: string): boolean {
    return this.guarded(instanceId, "suspended", (a) => a.suspend());
  }

  resume(instanceId: string): boolean {
    return this.guarded(instanceId, "resumed", (a) => a.resume());
  }

  reconfigure(instanceId: string, prefs: Record<string, unknown>): boolean {
    const inst = this.instances.get(instanceId);
    if (!this.live(inst)) return false;
    const before = inst.state;
    this.lifecycle(inst, "reconfiguring");
    try {
      inst.adapter.reconfigure(prefs);
    } catch (e) {
      this.onAdapterCrash(instanceId, `adapter threw during reconfigure: ${describe(e)}`);
      return false;
    }
    this.lifecycle(inst, before === "reconfiguring" ? "ready" : before);
    return true;
  }

  /** 有序關閉（goodbye）：pending 一律 uncertain、世代 +1、釋放 adapter。 */
  disposeInstance(instanceId: string, detail = "disposed by host"): void {
    const inst = this.instances.get(instanceId);
    if (!inst || inst.state === "disposed") return;
    this.tearDown(inst, "disposed", detail, "adapter-crashed");
  }

  /** adapter crash／斷線：pending → uncertain（安全 intent 另走 system.text）、釋放資源、generation += 1。 */
  onAdapterCrash(instanceId: string, detail = "adapter crashed"): void {
    const inst = this.instances.get(instanceId);
    if (!inst || inst.state === "disposed" || inst.state === "crashed") return;
    this.tearDown(inst, "crashed", detail, "adapter-crashed");
  }

  private tearDown(
    inst: Instance,
    state: "crashed" | "disposed",
    detail: string,
    reason: SystemTextMessage["reason"]
  ) {
    const staleGeneration = inst.generation;
    for (const p of [...inst.pending.values()]) {
      this.emit(inst, p.envelope.messageId, {
        status: "uncertain",
        resolution: p.effective,
        detail: detail.slice(0, LIMITS.stringMaxChars),
      });
      this.forget(inst, p, "uncertain");
      if (p.safety) this.systemText(inst, p.envelope, reason);
    }
    inst.pending.clear();
    inst.channelOwners.clear();
    inst.input.pendingDrag = null;
    inst.unsubscribe?.();
    inst.unsubscribe = null;
    inst.generation = staleGeneration + 1;
    try {
      inst.adapter.dispose();
    } catch (e) {
      this.audit("adapter-dispose-threw", { instanceId: inst.instanceId, detail: describe(e) });
    }
    for (const g of this.grants.values()) {
      if (g.instanceId === inst.instanceId) g.revoked = true;
    }
    this.lifecycle(inst, state, detail);
    this.audit(state === "crashed" ? "adapter-crash" : "instance-disposed", {
      instanceId: inst.instanceId,
      detail: `generation ${staleGeneration} → ${inst.generation}`,
    });
  }

  // ---- Dispatch ------------------------------------------------------------

  /**
   * 派送一則 intent envelope（同步）。回傳第一則回執（accepted／duplicate／expired／
   * unsupported／failed）；後續回執經 onReceipt。
   */
  dispatch(input: IntentEnvelope, source: EnvelopeSource = "runtime"): CommandReceipt {
    const at = this.now();
    const inst = this.instances.get(input?.characterInstanceId);
    const messageId = typeof input?.messageId === "string" && input.messageId.length > 0 ? input.messageId : "";
    const orphan = (status: ReceiptStatus, detail: string, resolution: Resolution = "unsupported"): CommandReceipt => {
      const r: CommandReceipt = {
        messageId,
        characterInstanceId: typeof input?.characterInstanceId === "string" ? input.characterInstanceId : "",
        generation: inst?.generation ?? 0,
        status,
        resolution,
        detail,
        at: this.iso(at),
      };
      this.opts.onReceipt?.(r);
      return r;
    };

    if (!messageId) return orphan("unsupported", "messageId missing");
    if (!isCharacterIntent(input.intent)) return orphan("unsupported", "unknown intent");
    const ver = parseProtocolVersion(input.protocolVersion);
    if (!ver || ver.major !== 1) return orphan("unsupported", "protocol-version");

    const envelope = this.normalizeEnvelope(input, source);
    if (!envelope) {
      this.audit("ai-intent-rejected", { messageId, detail: input.intent });
      return orphan("unsupported", "intent not requestable by AI");
    }
    const safety = isSafetyIntent(envelope.intent);

    // 大小限制（§4.4／§8）：安全訊息不因此遺失——固定文字走 system.text。
    const sizeIssue = this.sizeIssue(envelope);
    if (sizeIssue) {
      this.audit("envelope-rejected", { instanceId: inst?.instanceId, messageId, detail: sizeIssue });
      if (safety) this.systemText(inst ?? null, envelope, "rejected");
      return orphan("failed", sizeIssue, "failed");
    }

    if (!this.live(inst) || !inst.negotiated) {
      if (safety) this.systemText(inst ?? null, envelope, "no-instance");
      return orphan("unsupported", inst ? `instance ${inst.state}` : "no such instance");
    }
    inst.lastSeenAt = at;

    const resolution = inst.negotiated.resolutions[envelope.intent];
    const negotiatedResolution: Resolution = resolution?.resolution ?? "unsupported";

    // 去重（環 256）：回執帶原命令的 resolution（Rust 權威端同樣行為），不硬編 exact。
    if (inst.seen.has(messageId) || inst.pending.has(messageId)) {
      const known = inst.pending.get(messageId)?.effective ?? inst.seen.get(messageId)?.resolution ?? negotiatedResolution;
      const r = this.emit(inst, messageId, { status: "accepted", resolution: known, duplicate: true, detail: "duplicate" });
      return r;
    }
    this.remember(inst, messageId, "accepted", negotiatedResolution);

    // 過期
    const expiresAt = parseIso(envelope.expiresAt);
    if (expiresAt !== null && at > expiresAt) {
      this.audit("envelope-expired", { instanceId: inst.instanceId, messageId });
      const r = this.emit(inst, messageId, { status: "expired", resolution: "unsupported", detail: "expired before dispatch" });
      this.remember(inst, messageId, "expired", "unsupported");
      return r;
    }

    if (negotiatedResolution === "unsupported") {
      const r = this.emit(inst, messageId, { status: "unsupported", resolution: "unsupported", detail: "not negotiated" });
      this.remember(inst, messageId, "unsupported", "unsupported");
      return r;
    }
    if (resolution.via === "system.text") {
      const accepted = this.emit(inst, messageId, { status: "accepted", resolution: "substituted", detail: "system.text" });
      this.systemText(inst, envelope, "negotiated");
      this.emit(inst, messageId, { status: "completed", resolution: "substituted", detail: "system.text" });
      this.remember(inst, messageId, "completed", "substituted");
      return accepted;
    }

    const cap = inst.negotiated.capabilities[resolution.via ?? ""];
    const pending: Pending = {
      envelope,
      status: "accepted",
      resolution,
      effective: resolution.resolution,
      channels: this.channelsFor(inst, envelope),
      priority: envelope.priority,
      floor: priorityFloor(envelope.intent),
      safety,
      interruptible: cap?.interruptible ?? true,
      resumable: cap?.resumable ?? false,
      acceptedAt: at,
      startedAt: null,
      acknowledgedAt: null,
      expiresAt,
      adapterSpoke: false,
      resumeOf: null,
    };

    // pending 上限 64：先丟最舊的非安全；否則丟 floor 較低的最舊者；都不行則拒絕新者。
    if (inst.pending.size >= LIMITS.maxPending && !this.makeRoom(inst, pending)) {
      const accepted = this.emit(inst, messageId, { status: "accepted", resolution: pending.effective });
      this.emit(inst, messageId, { status: "cancelled", resolution: pending.effective, reason: "queue-full" });
      this.remember(inst, messageId, "cancelled", pending.effective);
      this.audit("pending-queue-full", { instanceId: inst.instanceId, messageId, detail: envelope.intent });
      // 佇列被同層／更高 floor 的安全 intent 塞滿時，新的安全訊息不能就這樣消失
      // （既沒演、也沒文字）：比照 Rust 權威端交給 system.text。
      if (safety) this.systemText(inst, envelope, "not-presented");
      return accepted;
    }

    inst.pending.set(messageId, pending);
    const accepted = this.emit(inst, messageId, { status: "accepted", resolution: pending.effective });
    this.arbitrate(inst, pending);
    return accepted;
  }

  /** 對所有活著的實例派送（廣播）；messageId 加上 `@<instanceId>`；安全 intent 的 system.text 只出一次。 */
  broadcast(envelope: IntentEnvelope, source: EnvelopeSource = "runtime", roles?: CharacterRole[]): CommandReceipt[] {
    const safety = isSafetyIntent(envelope.intent);
    const targets = [...this.instances.values()].filter((i) => {
      if (!this.live(i)) return false;
      if (roles) return roles.includes(i.role);
      if (safety) return true;
      return i.role === "primary-companion" || i.role === "familiar" || i.role === "worker";
    });
    if (targets.length === 0) {
      if (safety) this.systemText(null, envelope, "no-instance");
      return [];
    }
    return targets.map((i) =>
      this.dispatch(
        { ...envelope, messageId: `${envelope.messageId}@${i.instanceId}`, characterInstanceId: i.instanceId },
        source
      )
    );
  }

  private normalizeEnvelope(input: IntentEnvelope, source: EnvelopeSource): IntentEnvelope | null {
    const requested = typeof input.priority === "number" && Number.isFinite(input.priority) ? input.priority : 0;
    let priority: number;
    let truthState: TruthState = isTruthState(input.truthState) ? input.truthState : "none";
    if (source === "ai") {
      // wait／ask 有 priority floor：AI 請求一律換成非安全近似 intent（保留原意圖為 variant 提示）。
      const substituted = aiSafeSubstitute(input.intent);
      if (substituted !== input.intent) {
        input = {
          ...input,
          intent: substituted,
          presentationHints: { ...(input.presentationHints ?? {}), variant: input.presentationHints?.variant ?? input.intent },
        };
      }
      if (!AI_REQUESTABLE_INTENTS.includes(input.intent)) return null;
      priority = Math.max(0, Math.min(AI_PRIORITY_CAP, requested));
      if (truthState !== "none") this.audit("forged-truth-state", { messageId: input.messageId, detail: truthState });
      truthState = "none";
    } else if (source === "resume") {
      priority = Math.max(0, Math.min(PRIORITY_MAX, requested));
    } else {
      priority = Math.max(0, Math.min(PRIORITY_MAX, Math.max(requested, priorityFloor(input.intent))));
    }
    const hints = input.presentationHints;
    const presentationHints =
      hints && typeof hints === "object"
        ? {
            ...hints,
            ...(typeof hints.message === "string" ? { message: hints.message.slice(0, LIMITS.stringMaxChars) } : {}),
          }
        : undefined;
    const interruptPolicy =
      input.interruptPolicy === "queue" ||
      input.interruptPolicy === "drop-if-busy" ||
      input.interruptPolicy === "merge" ||
      input.interruptPolicy === "preempt"
        ? input.interruptPolicy
        : "preempt";
    const resumePolicy =
      input.resumePolicy === "resume-previous" || input.resumePolicy === "return-idle" || input.resumePolicy === "none"
        ? input.resumePolicy
        : "none";
    const privacyClass =
      input.privacyClass === "public" ||
      input.privacyClass === "internal" ||
      input.privacyClass === "personal" ||
      input.privacyClass === "intimate"
        ? input.privacyClass
        : "internal";
    return {
      ...input,
      truthState,
      priority,
      interruptPolicy,
      resumePolicy,
      privacyClass,
      ...(presentationHints ? { presentationHints } : {}),
      timestamp: typeof input.timestamp === "string" ? input.timestamp : this.iso(),
    };
  }

  private sizeIssue(envelope: IntentEnvelope): string | null {
    let text: string;
    try {
      text = JSON.stringify(envelope);
    } catch {
      return "envelope not serializable";
    }
    if (utf8Bytes(text) > LIMITS.maxMessageBytes) return `envelope exceeds ${LIMITS.maxMessageBytes} bytes`;
    return parametersIssue(envelope.parameters);
  }

  private channelsFor(inst: Instance, envelope: IntentEnvelope): string[] {
    const channels = new Set<string>(["expression"]);
    if (envelope.presentationHints?.message) channels.add("bubble");
    const hinted = envelope.presentationHints?.channels;
    if (hinted && typeof hinted === "object") {
      for (const ch of Object.keys(hinted)) {
        // custom（nonSafety）channel 不參與搶占判斷。
        if (isSemanticChannel(ch) && inst.negotiated?.acceptedChannels.includes(ch)) channels.add(ch);
      }
    }
    return [...channels];
  }

  private makeRoom(inst: Instance, incoming: Pending): boolean {
    const ordered = [...inst.pending.values()].sort((a, b) => a.acceptedAt - b.acceptedAt);
    const victim = ordered.find((p) => !p.safety) ?? ordered.find((p) => p.floor < incoming.floor);
    if (!victim) return false;
    this.cancelPending(inst, victim, "queue-full");
    return true;
  }

  // ---- Mixer／搶占（§5） ----------------------------------------------------

  private ownersOf(inst: Instance, channels: string[], exceptId: string): Pending[] {
    const owners = new Map<string, Pending>();
    for (const ch of channels) {
      const id = inst.channelOwners.get(ch);
      if (!id || id === exceptId) continue;
      const p = inst.pending.get(id);
      if (p) owners.set(id, p);
      else inst.channelOwners.delete(ch);
    }
    return [...owners.values()];
  }

  private canPreempt(incoming: Pending, owner: Pending): boolean {
    if (incoming.priority <= owner.priority) return false;
    return owner.interruptible || incoming.floor >= HARD_PREEMPT_FLOOR;
  }

  private arbitrate(inst: Instance, p: Pending) {
    const owners = this.ownersOf(inst, p.channels, p.envelope.messageId);
    if (owners.length === 0) {
      this.start(inst, p);
      return;
    }
    if (owners.every((o) => this.canPreempt(p, o))) {
      // 先佔住 channel，再取消被搶占者（其 cancelled 回執可能同步觸發排程）。
      for (const ch of p.channels) inst.channelOwners.set(ch, p.envelope.messageId);
      const best = owners.slice().sort((a, b) => b.priority - a.priority || a.acceptedAt - b.acceptedAt)[0];
      if (p.envelope.resumePolicy === "resume-previous" && best.resumable && best.status !== "scheduled") {
        p.resumeOf = best.envelope;
      }
      for (const o of owners) this.cancelPending(inst, o, "preempted");
      this.start(inst, p);
      return;
    }
    switch (p.envelope.interruptPolicy) {
      case "drop-if-busy":
        this.cancelPending(inst, p, "busy");
        return;
      case "merge": {
        // 合併鍵與 Rust 權威端一致：同 intent＋同 correlationId 才算同一件事。
        const same = owners.find(
          (o) => o.envelope.intent === p.envelope.intent && o.envelope.correlationId === p.envelope.correlationId
        );
        if (same) {
          // 併入既有演出的命令**沒有被演出過**：誠實回 cancelled{merged}，不得謊報 completed。
          this.emit(inst, p.envelope.messageId, {
            status: "cancelled",
            resolution: p.effective,
            reason: "merged",
            detail: `merged into ${same.envelope.messageId}`,
          });
          this.forget(inst, p, "cancelled");
          return;
        }
        this.schedule(inst, p);
        return;
      }
      default:
        this.schedule(inst, p);
    }
  }

  private schedule(inst: Instance, p: Pending) {
    p.status = "scheduled";
    this.emit(inst, p.envelope.messageId, { status: "scheduled", resolution: p.effective });
  }

  private start(inst: Instance, p: Pending) {
    for (const ch of p.channels) inst.channelOwners.set(ch, p.envelope.messageId);
    if (p.status === "scheduled") p.status = "accepted";
    const generation = inst.generation;
    const sink = (r: AdapterReceipt) => this.onAdapterReceipt(inst, generation, r);
    try {
      inst.adapter.perform(p.envelope, sink, p.resolution);
    } catch (e) {
      if (inst.pending.get(p.envelope.messageId) === p) {
        this.emit(inst, p.envelope.messageId, {
          status: "failed",
          resolution: "failed",
          detail: `adapter threw: ${describe(e)}`.slice(0, LIMITS.stringMaxChars),
        });
        this.finalize(inst, p, "failed");
      }
      return;
    }
    if (inst.pending.get(p.envelope.messageId) === p && !p.adapterSpoke) {
      this.emit(inst, p.envelope.messageId, { status: "failed", resolution: "failed", detail: "adapter-silent" });
      this.finalize(inst, p, "failed", "adapter-silent");
    }
  }

  private scheduleNext(inst: Instance) {
    if (!this.live(inst)) return;
    const waiting = [...inst.pending.values()]
      .filter((p) => p.status === "scheduled")
      .sort((a, b) => b.priority - a.priority || a.acceptedAt - b.acceptedAt);
    for (const p of waiting) {
      if (inst.pending.get(p.envelope.messageId) !== p || p.status !== "scheduled") continue;
      if (p.expiresAt !== null && this.now() > p.expiresAt) {
        this.emit(inst, p.envelope.messageId, { status: "expired", resolution: p.effective, detail: "expired while queued" });
        this.forget(inst, p, "expired");
        continue;
      }
      if (this.ownersOf(inst, p.channels, p.envelope.messageId).length === 0) {
        this.start(inst, p);
      }
    }
  }

  // ---- 回執 ----------------------------------------------------------------

  private emit(inst: Instance, messageId: string, partial: Partial<CommandReceipt> & { status: ReceiptStatus; resolution: Resolution }): CommandReceipt {
    const r: CommandReceipt = {
      messageId,
      characterInstanceId: inst.instanceId,
      generation: inst.generation,
      at: this.iso(),
      ...partial,
      ...(partial.detail !== undefined ? { detail: String(partial.detail).slice(0, LIMITS.stringMaxChars) } : {}),
    };
    this.opts.onReceipt?.(r);
    return r;
  }

  private remember(inst: Instance, messageId: string, status: ReceiptStatus, resolution: Resolution) {
    if (!inst.seen.has(messageId)) {
      inst.seenOrder.push(messageId);
      while (inst.seenOrder.length > LIMITS.dedupeRing) {
        const old = inst.seenOrder.shift();
        if (old !== undefined) inst.seen.delete(old);
      }
    }
    inst.seen.set(messageId, { status, resolution });
  }

  private forget(inst: Instance, p: Pending, status: ReceiptStatus) {
    const id = p.envelope.messageId;
    if (inst.pending.get(id) === p) inst.pending.delete(id);
    for (const [ch, owner] of [...inst.channelOwners.entries()]) {
      if (owner === id) inst.channelOwners.delete(ch);
    }
    this.remember(inst, id, status, status === "failed" ? "failed" : p.effective);
  }

  private finalize(inst: Instance, p: Pending, status: ReceiptStatus, reason?: SystemTextMessage["reason"]) {
    this.forget(inst, p, status);
    // 安全 intent 只有 completed 算演到使用者眼前。其餘任何終態（failed／unsupported／
    // cancelled／expired／uncertain）都代表訊息沒送到，一律補 system.text——否則 adapter
    // 只要挑一個合法終態就能吞掉 emergency／blocked／request-consent／offline。
    if (p.safety && status !== "completed") {
      this.systemText(inst, p.envelope, reason ?? (status === "failed" ? "adapter-failed" : "not-presented"));
    }
    if (status === "completed" || status === "expired") {
      if (p.resumeOf && p.envelope.resumePolicy === "resume-previous") {
        const prev = p.resumeOf;
        const prevExpiry = parseIso(prev.expiresAt);
        if (prevExpiry === null || this.now() <= prevExpiry) {
          this.dispatch(
            {
              ...prev,
              messageId: `${prev.messageId}~r${inst.generation}-${p.envelope.messageId}`,
              timestamp: this.iso(),
            },
            "resume"
          );
        }
      } else if (p.envelope.resumePolicy === "return-idle" && this.live(inst)) {
        this.dispatch(
          {
            protocolVersion: PROTOCOL_VERSION,
            messageId: `${p.envelope.messageId}~idle`,
            characterInstanceId: inst.instanceId,
            correlationId: p.envelope.correlationId,
            timestamp: this.iso(),
            intent: "idle",
            truthState: "none",
            priority: 0,
            interruptPolicy: "queue",
            resumePolicy: "none",
            privacyClass: "internal",
          },
          "resume"
        );
      }
    }
    this.scheduleNext(inst);
  }

  private cancelPending(inst: Instance, p: Pending, reason: string): CommandReceipt {
    const id = p.envelope.messageId;
    const wasStarted = p.status === "started" || p.status === "accepted" || p.status === "acknowledged";
    // 先記成終結，再通知 adapter（其同步回的 cancelled 會被視為 after-terminal 而丟棄）。
    const receipt = this.emit(inst, id, { status: "cancelled", resolution: p.effective, reason });
    this.forget(inst, p, "cancelled");
    if (wasStarted) {
      try {
        inst.adapter.cancel(id);
      } catch (e) {
        this.audit("adapter-cancel-threw", { instanceId: inst.instanceId, messageId: id, detail: describe(e) });
      }
    }
    return receipt;
  }

  /** cancel 冪等：pending → cancelled{reason}；已終結／未知 → cancelled{alreadyTerminal:true}（不報錯、不重複發回執）。 */
  cancel(messageId: string, reason = "host"): CommandReceipt {
    for (const inst of this.instances.values()) {
      const p = inst.pending.get(messageId);
      if (p) {
        const r = this.cancelPending(inst, p, reason);
        this.scheduleNext(inst);
        return r;
      }
    }
    for (const inst of this.instances.values()) {
      const last = inst.seen.get(messageId);
      if (last) {
        return {
          messageId,
          characterInstanceId: inst.instanceId,
          generation: inst.generation,
          status: "cancelled",
          resolution: last.resolution,
          alreadyTerminal: true,
          detail: `already ${last.status}`,
          at: this.iso(),
        };
      }
    }
    return {
      messageId,
      characterInstanceId: "",
      generation: 0,
      status: "cancelled",
      resolution: "unsupported",
      alreadyTerminal: true,
      detail: "unknown messageId",
      at: this.iso(),
    };
  }

  private onAdapterReceipt(inst: Instance, generation: number, raw: AdapterReceipt) {
    if (!raw || typeof raw !== "object") return;
    const messageId = typeof raw.messageId === "string" ? raw.messageId : "";
    if (generation !== inst.generation || (raw.generation !== undefined && raw.generation !== inst.generation)) {
      this.audit("stale-generation-receipt", { instanceId: inst.instanceId, messageId, detail: `sink generation ${generation}` });
      return;
    }
    if (!isReceiptStatus(raw.status)) {
      this.audit("invalid-receipt", { instanceId: inst.instanceId, messageId, detail: "status" });
      return;
    }
    const extra = raw as unknown as Record<string, unknown>;
    if ("truthState" in extra || "verified" in extra) {
      this.audit("forged-receipt-field", { instanceId: inst.instanceId, messageId, detail: "truthState/verified ignored" });
    }
    inst.lastSeenAt = this.now();
    const p = inst.pending.get(messageId);
    if (!p) {
      this.audit(inst.seen.has(messageId) ? "receipt-after-terminal" : "receipt-unknown-message", {
        instanceId: inst.instanceId,
        messageId,
        detail: raw.status,
      });
      return;
    }
    p.adapterSpoke = true;
    const status = raw.status;
    const from = p.status;
    const legal = (): boolean => {
      switch (status) {
        case "accepted":
          return false; // Gateway 已發 accepted；adapter 的 accepted 只算活著
        case "scheduled":
          return from === "accepted";
        case "started":
          return from === "accepted" || from === "scheduled";
        case "acknowledged":
          return from === "accepted" || from === "scheduled";
        case "completed":
          return from === "started";
        case "cancelled":
        case "failed":
        case "uncertain":
          return true;
        case "expired":
        case "unsupported":
          return from === "accepted" || from === "scheduled";
        default:
          return false;
      }
    };
    if (status === "accepted" || (status === "started" && from === "started")) return;
    if (!legal()) {
      this.audit("illegal-receipt-transition", { instanceId: inst.instanceId, messageId, detail: `${from} → ${status}` });
      return;
    }
    if (isResolution(raw.resolution)) p.effective = worse(p.effective, raw.resolution);
    if (status === "failed") p.effective = "failed";
    if (status === "unsupported") p.effective = "unsupported";
    const detail = typeof raw.detail === "string" ? raw.detail.slice(0, LIMITS.stringMaxChars) : undefined;
    const reason = typeof raw.reason === "string" ? raw.reason.slice(0, 64) : undefined;

    switch (status) {
      case "scheduled":
        p.status = "scheduled";
        this.emit(inst, messageId, { status, resolution: p.effective, detail });
        return;
      case "started":
        p.status = "started";
        p.startedAt = this.now();
        this.emit(inst, messageId, { status, resolution: p.effective, detail });
        return;
      case "acknowledged":
        p.status = "acknowledged";
        p.acknowledgedAt = this.now();
        this.emit(inst, messageId, { status, resolution: p.effective, detail });
        return;
      default: {
        this.emit(inst, messageId, {
          status,
          resolution: p.effective,
          detail,
          ...(status === "cancelled" ? { reason: reason ?? "adapter" } : {}),
        });
        this.finalize(inst, p, status);
      }
    }
  }

  // ---- system.text ---------------------------------------------------------

  private systemText(inst: Instance | null, envelope: IntentEnvelope, reason: SystemTextMessage["reason"]) {
    const key = `${baseMessageId(envelope.messageId)}|${envelope.intent}`;
    if (this.systemTextSeen.includes(key)) {
      this.audit("system-text-deduped", { instanceId: inst?.instanceId, messageId: envelope.messageId });
      return;
    }
    this.systemTextSeen.push(key);
    while (this.systemTextSeen.length > LIMITS.dedupeRing) this.systemTextSeen.shift();
    const line = intentLine(envelope.intent, envelope.truthState, envelope.presentationHints?.message);
    this.opts.onSystemText({
      instanceId: inst?.instanceId ?? null,
      messageId: envelope.messageId,
      correlationId: envelope.correlationId,
      intent: envelope.intent,
      truthState: envelope.truthState,
      text: line.text,
      marker: line.marker,
      reason,
    });
  }

  // ---- Sweep（host 每 500 ms 呼叫） -------------------------------------------

  heartbeat(instanceId: string): void {
    const inst = this.instances.get(instanceId);
    if (inst) inst.lastSeenAt = this.now();
  }

  sweep(now: number = this.now()): void {
    for (const inst of [...this.instances.values()]) {
      if (!this.live(inst)) continue;
      if (inst.heartbeat && now - inst.lastSeenAt > LIMITS.heartbeatTimeoutMs) {
        this.onAdapterCrash(inst.instanceId, "heartbeat-timeout");
        this.lifecycle(inst, "reconnecting", "heartbeat-timeout");
        continue;
      }
      this.flushDrag(inst, now);
      for (const p of [...inst.pending.values()]) {
        if (inst.pending.get(p.envelope.messageId) !== p) continue;
        const id = p.envelope.messageId;
        const duration = Math.min(LIMITS.durationMaxMs, Math.max(0, p.envelope.durationHint?.ms ?? 0));
        if (p.status === "scheduled" && p.expiresAt !== null && now > p.expiresAt) {
          this.emit(inst, id, { status: "expired", resolution: p.effective, detail: "expired while queued" });
          this.finalize(inst, p, "expired");
          continue;
        }
        if (p.status === "acknowledged" && p.acknowledgedAt !== null && now >= p.acknowledgedAt + duration + LIMITS.acknowledgedGraceMs) {
          this.emit(inst, id, { status: "uncertain", resolution: p.effective, detail: "acknowledged without completion" });
          this.finalize(inst, p, "uncertain");
          continue;
        }
        const watchdogMs = (duration > 0 ? duration : LIMITS.durationMaxMs) + LIMITS.startedWatchdogGraceMs;
        if (p.status === "started" && p.startedAt !== null && now >= p.startedAt + watchdogMs) {
          this.emit(inst, id, { status: "uncertain", resolution: p.effective, detail: "watchdog: no completion" });
          this.finalize(inst, p, "uncertain");
          try {
            inst.adapter.cancel(id);
          } catch {
            // 已記 uncertain；adapter 例外不再影響
          }
          continue;
        }
        if (p.status === "accepted" && now >= p.acceptedAt + LIMITS.durationMaxMs + LIMITS.startedWatchdogGraceMs) {
          this.emit(inst, id, { status: "uncertain", resolution: p.effective, detail: "watchdog: never started" });
          this.finalize(inst, p, "uncertain");
        }
      }
      if (!this.live(inst)) continue;
      try {
        inst.adapter.tick?.(now);
      } catch (e) {
        this.onAdapterCrash(inst.instanceId, `adapter threw during tick: ${describe(e)}`);
        continue;
      }
      this.scheduleNext(inst);
    }
    for (const [id, g] of [...this.grants.entries()]) {
      if (g.revoked || Date.parse(g.expiresAt) < now - LIMITS.fileGrantMaxMs) this.grants.delete(id);
    }
  }

  // ---- 輸入事件（§6） --------------------------------------------------------

  /** 接收 adapter（或 host 代為接線的 pointer）原始事件；正規化後送 onInput 或進佇列。 */
  ingestInput(instanceId: string, raw: AdapterInputEvent): boolean {
    const inst = this.instances.get(instanceId);
    if (!this.live(inst)) return false;
    if (!raw || typeof raw !== "object") return false;
    if (raw.generation !== undefined && raw.generation !== inst.generation) {
      this.audit("stale-generation-event", { instanceId });
      return false;
    }
    if (!isInputEventKind(raw.kind)) {
      this.audit("unknown-event-kind", { instanceId, detail: String(raw.kind).slice(0, 64) });
      return false;
    }
    if (INPUT_SILENT_ROLES.includes(inst.role)) {
      this.audit("role-filtered-event", { instanceId, detail: raw.kind });
      return false;
    }
    // 宣告即契約（§6）：協商後沒有對應輸入能力的種類一律丟棄——連接頁對使用者說
    // 「可以接收：…」就必須是真的，不能私下收下並鑄出 file-drop grant。與 role 過濾同層，
    // 在扣速率預算之前。
    const requiredCapability = INPUT_KIND_CAPABILITY[raw.kind];
    if (requiredCapability !== undefined && !inst.negotiated?.capabilities[requiredCapability]?.supported) {
      this.audit("input-capability-not-declared", {
        instanceId,
        detail: `${raw.kind} needs ${requiredCapability}`,
      });
      return false;
    }
    const now = this.now();
    inst.lastSeenAt = now;
    if (!this.rateAllows(inst, now)) return false;
    const payloadIn = raw.payload && typeof raw.payload === "object" ? raw.payload : {};
    const st = inst.input;

    let payload: Record<string, unknown> | null;
    switch (raw.kind) {
      case "character.hover-entered":
      case "character.hover-left": {
        if (now - st.lastHoverAt < 1000 / LIMITS.hoverPerSecond) return false;
        st.lastHoverAt = now;
        payload = this.pointPayload(payloadIn);
        break;
      }
      case "character.dragged": {
        const point = this.pointPayload(payloadIn);
        if (now - st.lastDragAt < 1000 / LIMITS.draggedPerSecond) {
          st.pendingDrag = point; // 合併：只留最新一筆，sweep／下一事件時送出
          return true;
        }
        st.lastDragAt = now;
        st.pendingDrag = null;
        payload = point;
        break;
      }
      case "character.dropped":
        st.pendingDrag = null;
        payload = this.pointPayload(payloadIn);
        break;
      case "character.clicked":
      case "character.double-clicked":
      case "character.drag-started":
        payload = this.pointPayload(payloadIn);
        break;
      case "character.text-submitted": {
        const text = typeof payloadIn.text === "string" ? payloadIn.text : "";
        if (text.length === 0) return false;
        payload =
          text.length > LIMITS.textSubmittedMaxChars
            ? { text: text.slice(0, LIMITS.textSubmittedMaxChars), truncated: true }
            : { text };
        break;
      }
      case "character.file-dropped": {
        // README §6：`file-dropped` 的 payload 是**一個檔案**的扁平形狀
        // { name, mediaType, bytes, readableScope, grantId, expiresAt }——多檔＝多則事件。
        // 舊形狀 `{ files:[…] }` 會被 Runtime 的正規化器以 invalid-payload 丟掉
        // （對抗審查 character-protocol-028）。grant 表仍記所有檔案（listGrants）。
        const files = this.fileGrants(inst, payloadIn, now);
        if (files.length === 0) return false;
        this.flushDrag(inst, now, false);
        for (const grant of files) {
          this.deliver(inst, raw.kind, { ...grant }, raw.privacyClass, now);
        }
        return true;
      }
      case "character.toy-thrown": {
        payload = {};
        if (typeof payloadIn.toyId === "string") payload.toyId = payloadIn.toyId.slice(0, 64);
        const vx = clampInt(payloadIn.vx, -1000, 1000);
        const vy = clampInt(payloadIn.vy, -1000, 1000);
        if (vx !== null) payload.vx = vx;
        if (vy !== null) payload.vy = vy;
        break;
      }
      case "character.action-requested": {
        const action = typeof payloadIn.action === "string" ? payloadIn.action : "";
        if (!/^[a-z][a-z0-9-]{0,63}$/.test(action)) {
          this.audit("invalid-action-request", { instanceId });
          return false;
        }
        payload = { action };
        break;
      }
      case "character.dismissed":
        payload = {};
        break;
      case "character.visibility-changed":
        payload = { visible: payloadIn.visible === true };
        break;
      default:
        return false;
    }
    if (!payload) return false;
    this.flushDrag(inst, now, raw.kind === "character.dragged");
    this.deliver(inst, raw.kind, payload, raw.privacyClass, now);
    return true;
  }

  private rateAllows(inst: Instance, now: number): boolean {
    const st = inst.input;
    if (now - st.rateWindowStart >= 1000) {
      st.rateWindowStart = now;
      st.rateCount = 0;
      st.rateLimitedAudited = false;
    }
    st.rateCount += 1;
    if (st.rateCount > LIMITS.maxMessagesPerSecond) {
      if (!st.rateLimitedAudited) {
        st.rateLimitedAudited = true;
        this.audit("rate-limited", { instanceId: inst.instanceId, detail: "input events > 50/s dropped" });
      }
      return false;
    }
    return true;
  }

  /** 只保留視窗相對、8 px 量化的座標；任何其他欄位（含絕對螢幕座標、軌跡）一律丟棄。 */
  private pointPayload(input: Record<string, unknown>): Record<string, unknown> {
    const out: Record<string, unknown> = {};
    const x = quantize(input.x);
    const y = quantize(input.y);
    if (x !== null) out.x = x;
    if (y !== null) out.y = y;
    if (typeof input.button === "number" && Number.isInteger(input.button) && input.button >= 0 && input.button <= 4) {
      out.button = input.button;
    }
    return out;
  }

  private flushDrag(inst: Instance, now: number, skipIfDragging = false) {
    const st = inst.input;
    if (!st.pendingDrag || skipIfDragging) return;
    if (now - st.lastDragAt < 1000 / LIMITS.draggedPerSecond) return;
    const payload = st.pendingDrag;
    st.pendingDrag = null;
    st.lastDragAt = now;
    this.deliver(inst, "character.dragged", payload, undefined, now);
  }

  private fileGrants(inst: Instance, input: Record<string, unknown>, now: number): FileDropGrant[] {
    // 接受 `{ files:[…] }`（本機 adapter 的原料）或單一檔案的扁平形狀（README §6）。
    const list = Array.isArray(input.files)
      ? input.files.slice(0, LIMITS.fileDropMaxFiles)
      : typeof input.name === "string"
        ? [input]
        : [];
    const ttlRequested = typeof input.grantTtlMs === "number" && Number.isFinite(input.grantTtlMs) ? input.grantTtlMs : LIMITS.fileGrantMaxMs;
    const ttl = Math.max(1000, Math.min(LIMITS.fileGrantMaxMs, ttlRequested));
    const out: FileDropGrant[] = [];
    for (const f of list) {
      if (!f || typeof f !== "object") continue;
      const rec = f as Record<string, unknown>;
      const rawName = typeof rec.name === "string" ? rec.name : "";
      const base = rawName.split(/[\\/]/).pop() ?? "";
      if (base.length === 0) continue;
      const name = base.slice(0, LIMITS.fileNameMaxChars);
      const mediaType =
        typeof rec.mediaType === "string" && /^[a-z]+\/[a-z0-9.+-]{1,64}$/i.test(rec.mediaType)
          ? rec.mediaType.toLowerCase()
          : "application/octet-stream";
      const bytes = clampInt(rec.bytes, 0, Number.MAX_SAFE_INTEGER) ?? 0;
      const grantId = this.nextId("grant");
      const expiresAt = this.iso(now + ttl);
      const grant: FileDropGrant = { name, mediaType, bytes, readableScope: "file", grantId, expiresAt };
      // grant 表有界（MAX_LIVE_GRANTS）：滿了先撤銷最舊的一筆，不讓惡意 adapter 灌爆記憶體。
      while (this.grants.size >= MAX_LIVE_GRANTS) {
        const oldest = this.grants.keys().next().value;
        if (oldest === undefined) break;
        this.grants.delete(oldest);
        this.audit("grant-evicted", { instanceId: inst.instanceId, detail: "grant table full" });
      }
      this.grants.set(grantId, { ...grant, instanceId: inst.instanceId, revoked: false });
      out.push(grant);
    }
    return out;
  }

  private deliver(inst: Instance, kind: InputEventKind, payload: Record<string, unknown>, privacy: PrivacyClass | undefined, now: number) {
    const defaultPrivacy: PrivacyClass =
      kind === "character.text-submitted" || kind === "character.file-dropped" ? "personal" : "internal";
    const privacyClass =
      privacy && PRIVACY_RANK[privacy] !== undefined && PRIVACY_RANK[privacy] > PRIVACY_RANK[defaultPrivacy]
        ? privacy
        : defaultPrivacy;
    const event: CharacterInputEvent = {
      protocolVersion: PROTOCOL_VERSION,
      eventId: this.nextId("evt"),
      characterInstanceId: inst.instanceId,
      generation: inst.generation,
      timestamp: this.iso(now),
      kind,
      payload,
      privacyClass,
    };
    const meta: InputMeta = { instanceId: inst.instanceId, characterId: inst.characterId, role: inst.role };
    if (this.opts.onInput) {
      this.opts.onInput(event, meta);
      return;
    }
    if (this.inputQueue.length >= LIMITS.inputQueue) {
      const idx = this.inputQueue.findIndex((q) => DROPPABLE_INPUT_KINDS.includes(q.event.kind));
      const dropped = idx >= 0 ? this.inputQueue.splice(idx, 1)[0] : this.inputQueue.shift();
      this.audit("input-queue-full", { instanceId: inst.instanceId, detail: dropped?.event.kind });
    }
    this.inputQueue.push({ event, meta });
  }

  /** 拉取模式：取出並清空正規化事件佇列（推送模式下永遠為空）。 */
  drainInput(): Array<{ event: CharacterInputEvent; meta: InputMeta }> {
    return this.inputQueue.splice(0, this.inputQueue.length);
  }

  inputQueueSize(): number {
    return this.inputQueue.length;
  }

  // ---- file-drop grants ----------------------------------------------------

  isGrantActive(grantId: string, now: number = this.now()): boolean {
    const g = this.grants.get(grantId);
    if (!g || g.revoked) return false;
    return Date.parse(g.expiresAt) > now;
  }

  revokeGrant(grantId: string): boolean {
    const g = this.grants.get(grantId);
    if (!g || g.revoked) return false;
    g.revoked = true;
    return true;
  }

  listGrants(): GrantRecord[] {
    return [...this.grants.values()].map((g) => ({ ...g }));
  }
}

function describe(e: unknown): string {
  if (e instanceof Error) return e.message.slice(0, 120);
  return String(e).slice(0, 120);
}
