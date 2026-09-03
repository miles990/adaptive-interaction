// Character Presentation Protocol（CPP）v1.0 — TypeScript 型別鏡射。
//
// 本檔 1:1 對應 docs/character-protocol/README.md；Rust（interaction-character）
// 是權威實作，這裡只鏡射同一份契約給桌面視窗內的 in-process Gateway／Adapter。
// 原則：
//   - 名稱與數值（intent、truthState、priority floor、限制）在 1.x 內只增不改。
//   - 呈現層沒有權限主權；truthState 只由 Runtime 決定，Adapter 不得產生 verified。
//   - 這裡只有型別與常數，沒有任何執行邏輯依賴 DOM／React。

export const PROTOCOL_VERSION = "1.0" as const;
export const PROTOCOL_MAJOR = 1;
export const PROTOCOL_MINOR = 0;

// ---------------------------------------------------------------------------
// §3.1 Canonical capability ids
// ---------------------------------------------------------------------------

export const CANONICAL_CAPABILITY_IDS = [
  "visual.presence",
  "visual.pose",
  "visual.expression",
  "visual.gaze",
  "visual.locomotion",
  "visual.overlay",
  "visual.particles",
  "visual.prop",
  "visual.textBubble",
  "audio.speech",
  "audio.effect",
  "haptic.cue",
  "light.cue",
  "input.click",
  "input.hover",
  "input.drag",
  "input.drop",
  "input.pointerProximity",
  "input.text",
  "input.fileDrop",
  "multiCharacter",
  "scene",
  "rollCall",
  "gameplay.toys",
  "gameplay.autonomy",
  "system.text",
] as const;

export type CanonicalCapabilityId = (typeof CANONICAL_CAPABILITY_IDS)[number];

/** 自訂能力 id：至少三段、小寫、以字母開頭（例如 com.example.character.wings）。 */
export type CustomCapabilityId = `${string}.${string}.${string}`;

export type CapabilityId = CanonicalCapabilityId | CustomCapabilityId;

/** 已知 canonical 前綴：有此前綴但未收錄的 id 視為 custom 並標 unknown。 */
export const CANONICAL_CAPABILITY_PREFIXES = [
  "visual.",
  "audio.",
  "haptic.",
  "light.",
  "input.",
] as const;

export const CUSTOM_CAPABILITY_ID_RE = /^[a-z][a-z0-9]*(\.[a-z][a-z0-9]*){2,}$/;

export function isCanonicalCapabilityId(id: string): id is CanonicalCapabilityId {
  return (CANONICAL_CAPABILITY_IDS as readonly string[]).includes(id);
}

// ---------------------------------------------------------------------------
// §3.2 CapabilityDecl
// ---------------------------------------------------------------------------

export type QualityLevel = "full" | "reduced" | "minimal";
export type ReducedMotionBehavior = "static" | "reduced" | "unchanged" | "disabled";

export interface DurationRange {
  minMs: number;
  maxMs: number;
}

/** §2.1 preferencesSchema／parameterSchema 白名單子集。 */
export interface PreferencePropertySchema {
  type: "boolean" | "number" | "integer" | "string";
  minimum?: number;
  maximum?: number;
  maxLength?: number;
  enum?: string[];
  default?: boolean | number | string;
  title?: string;
  description?: string;
}

export interface PreferencesSchema {
  type: "object";
  properties?: Record<string, PreferencePropertySchema>;
}

export interface CapabilityDecl {
  supported: boolean;
  version?: string;
  variants?: string[];
  maxConcurrent?: number;
  interruptible?: boolean;
  resumable?: boolean;
  durationRange?: DurationRange;
  parameterSchema?: PreferencesSchema;
  qualityLevel?: QualityLevel;
  reducedMotionBehavior?: ReducedMotionBehavior;
  requiresForeground?: boolean;
  requiresAudio?: boolean;
  /** 驗證器標記：canonical 前綴但未收錄的 id。 */
  unknown?: boolean;
}

// ---------------------------------------------------------------------------
// §2 Manifest
// ---------------------------------------------------------------------------

export type LocalizedText = Record<string, string>;

export type AdapterKind = "in-process" | "web" | "external-process" | "remote-device";

export const BUILTIN_ENTRYPOINT_IDS = ["shu-rig", "sprite", "text"] as const;
export type BuiltinEntrypointId = (typeof BUILTIN_ENTRYPOINT_IDS)[number];

export type Entrypoint =
  | { kind: "builtin"; id: string }
  | { kind: "module"; path: string }
  | { kind: "process"; command: string[] }
  | { kind: "url"; url: string };

export interface AssetDecl {
  id: string;
  path: string;
  mediaType?: string;
  bytes?: number;
  sha256?: string;
}

export interface VariantDecl {
  id: string;
  displayName?: LocalizedText;
}

export type FileAccess = "none" | "character-folder" | "user-granted";

export interface SecurityRequirements {
  network: boolean;
  executable: boolean;
  fileAccess: FileAccess;
  audioOutput: boolean;
  microphone: boolean;
  camera: boolean;
}

export interface ResourceLimits {
  maxAssetBytes: number;
  maxConcurrentCommands: number;
  maxQueue: number;
  maxFps: number;
}

export interface FallbackDecl {
  capabilities?: Record<string, string[]>;
  intents?: Partial<Record<CharacterIntent, CharacterIntent>>;
}

export interface Compatibility {
  protocol: string;
  runtime?: string;
}

export interface CharacterManifest {
  schemaVersion: string;
  characterId: string;
  displayName: LocalizedText;
  author?: string;
  description?: LocalizedText;
  version: string;
  adapterKind: AdapterKind;
  entrypoint: Entrypoint;
  assets: AssetDecl[];
  capabilities: Record<string, CapabilityDecl>;
  inputCapabilities: Record<string, CapabilityDecl>;
  channels: string[];
  states: string[];
  intents: CharacterIntent[];
  variants: VariantDecl[];
  locales: string[];
  pronouns?: LocalizedText;
  preferencesSchema?: PreferencesSchema;
  securityRequirements: SecurityRequirements;
  resourceLimits: ResourceLimits;
  fallbacks: FallbackDecl;
  compatibility: Compatibility;
}

export const CHARACTER_ID_RE = /^[a-z0-9][a-z0-9._-]{0,63}$/;

// ---------------------------------------------------------------------------
// §4 Intent / truthState / priority floor
// ---------------------------------------------------------------------------

export const CHARACTER_INTENTS = [
  "idle",
  "notice",
  "acknowledge",
  "think",
  "work",
  "wait",
  "ask",
  "request-consent",
  "blocked",
  "unknown",
  "claim-completed",
  "verified-success",
  "failed",
  "cancelled",
  "offline",
  "emergency",
  "greet",
  "play",
  "rest",
  "sleep",
] as const;

export type CharacterIntent = (typeof CHARACTER_INTENTS)[number];

export function isCharacterIntent(v: unknown): v is CharacterIntent {
  return typeof v === "string" && (CHARACTER_INTENTS as readonly string[]).includes(v);
}

export const TRUTH_STATES = [
  "none",
  "queued",
  "working",
  "waiting-input",
  "waiting-consent",
  "blocked",
  "claimed",
  "verified",
  "failed",
  "timed-out",
  "expired",
  "unknown",
  "cancelled",
  "emergency",
  "offline",
] as const;

export type TruthState = (typeof TRUTH_STATES)[number];

export function isTruthState(v: unknown): v is TruthState {
  return typeof v === "string" && (TRUTH_STATES as readonly string[]).includes(v);
}

/** §4.3 priority 下限（Runtime 固定）。非安全 intent 無下限（0），AI 請求上限 50。 */
export const PRIORITY_FLOOR: Readonly<Record<CharacterIntent, number>> = {
  emergency: 100,
  offline: 95,
  blocked: 90,
  failed: 85,
  "request-consent": 80,
  unknown: 75,
  "verified-success": 70,
  "claim-completed": 65,
  wait: 60,
  ask: 60,
  cancelled: 55,
  idle: 0,
  notice: 0,
  acknowledge: 0,
  think: 0,
  work: 0,
  greet: 0,
  play: 0,
  rest: 0,
  sleep: 0,
};

export const AI_PRIORITY_CAP = 50;
export const PRIORITY_MAX = 100;

/** floor ≥ 75 的 intent 可搶占 interruptible=false 的演出（§5）。 */
export const HARD_PREEMPT_FLOOR = 75;

/** §3.4 步驟 5：「安全 intent」= §4.3 有 floor 者。 */
export const SAFETY_INTENTS: readonly CharacterIntent[] = CHARACTER_INTENTS.filter(
  (i) => PRIORITY_FLOOR[i] > 0
);

export function isSafetyIntent(intent: string): boolean {
  return isCharacterIntent(intent) && PRIORITY_FLOOR[intent] > 0;
}

export function priorityFloor(intent: string): number {
  return isCharacterIntent(intent) ? PRIORITY_FLOOR[intent] : 0;
}

/** AI 透過 companion.state.present 能請求的 intent 子集（§11 最後一列）。 */
export const AI_REQUESTABLE_INTENTS: readonly CharacterIntent[] = [
  "rest",
  "notice",
  "think",
  "work",
  "acknowledge",
];

/**
 * AI 請求的 behaviorIntent 若對應到有 priority floor 的 intent（wait／ask 的 floor 是 60），
 * 一律換成非安全的近似 intent（與 Rust `CharacterIntent::ai_safe_substitute` 一致）：
 * AI 永遠不能點播會搶占安全演出的 intent；原始意圖以 presentationHints.variant 保留給 adapter 參考。
 */
export const AI_INTENT_SUBSTITUTE: Readonly<Partial<Record<CharacterIntent, CharacterIntent>>> = {
  wait: "think",
  ask: "notice",
};

export function aiSafeSubstitute(intent: CharacterIntent): CharacterIntent {
  return AI_INTENT_SUBSTITUTE[intent] ?? intent;
}

/**
 * 每個 intent 可由哪些能力承載（依偏好排序；[0] 為主要能力，也是
 * `fallbacks.capabilities` 鏈的鍵）。協商 §3.4 步驟 1「對應能力 supported」
 * 即檢查此清單中第一個 supported 者。
 */
const EXPRESSIVE: CanonicalCapabilityId[] = [
  "visual.expression",
  "visual.pose",
  "visual.textBubble",
  "audio.speech",
  "audio.effect",
  "light.cue",
  "haptic.cue",
];

export const INTENT_CAPABILITIES: Readonly<Record<CharacterIntent, readonly CanonicalCapabilityId[]>> =
  {
    idle: ["visual.presence", ...EXPRESSIVE],
    notice: EXPRESSIVE,
    acknowledge: EXPRESSIVE,
    think: EXPRESSIVE,
    work: EXPRESSIVE,
    wait: EXPRESSIVE,
    ask: EXPRESSIVE,
    "request-consent": EXPRESSIVE,
    blocked: EXPRESSIVE,
    unknown: EXPRESSIVE,
    "claim-completed": EXPRESSIVE,
    "verified-success": EXPRESSIVE,
    failed: EXPRESSIVE,
    cancelled: EXPRESSIVE,
    offline: EXPRESSIVE,
    emergency: EXPRESSIVE,
    greet: EXPRESSIVE,
    play: EXPRESSIVE,
    rest: EXPRESSIVE,
    sleep: EXPRESSIVE,
  };

// ---------------------------------------------------------------------------
// §4.4 Envelope
// ---------------------------------------------------------------------------

export type InterruptPolicy = "preempt" | "queue" | "drop-if-busy" | "merge";
export type ResumePolicy = "resume-previous" | "return-idle" | "none";
export type PrivacyClass = "public" | "internal" | "personal" | "intimate";

export interface DurationHint {
  ms: number;
  loop?: boolean;
}

export interface PresentationHints {
  tone?: string;
  message?: string;
  variant?: string;
  channels?: Record<string, unknown>;
}

export interface IntentEnvelope {
  protocolVersion: string;
  messageId: string;
  characterInstanceId: string;
  correlationId?: string;
  timestamp: string;
  intent: CharacterIntent;
  truthState: TruthState;
  priority: number;
  interruptPolicy: InterruptPolicy;
  resumePolicy: ResumePolicy;
  durationHint?: DurationHint;
  parameters?: Record<string, unknown>;
  presentationHints?: PresentationHints;
  privacyClass: PrivacyClass;
  expiresAt?: string;
}

// ---------------------------------------------------------------------------
// §5 Semantic channels
// ---------------------------------------------------------------------------

export const SEMANTIC_CHANNELS = [
  "transform",
  "locomotion",
  "pose",
  "expression",
  "gaze",
  "speech",
  "bubble",
  "audio",
  "prop",
  "overlay",
  "particle",
  "scene",
] as const;

export type SemanticChannel = (typeof SEMANTIC_CHANNELS)[number];

export function isSemanticChannel(id: string): id is SemanticChannel {
  return (SEMANTIC_CHANNELS as readonly string[]).includes(id);
}

// ---------------------------------------------------------------------------
// §6 Input events
// ---------------------------------------------------------------------------

export const INPUT_EVENT_KINDS = [
  "character.clicked",
  "character.double-clicked",
  "character.hover-entered",
  "character.hover-left",
  "character.drag-started",
  "character.dragged",
  "character.dropped",
  "character.text-submitted",
  "character.file-dropped",
  "character.toy-thrown",
  "character.action-requested",
  "character.dismissed",
  "character.visibility-changed",
] as const;

export type InputEventKind = (typeof INPUT_EVENT_KINDS)[number];

export function isInputEventKind(v: unknown): v is InputEventKind {
  return typeof v === "string" && (INPUT_EVENT_KINDS as readonly string[]).includes(v);
}

export interface CharacterInputEvent {
  protocolVersion: string;
  eventId: string;
  characterInstanceId: string;
  generation: number;
  timestamp: string;
  kind: InputEventKind;
  payload: Record<string, unknown>;
  privacyClass: PrivacyClass;
}

/** file-dropped 的 payload 項目：只有 metadata 與短效 grant，沒有路徑、沒有內容。 */
export interface FileDropGrant {
  name: string;
  mediaType: string;
  bytes: number;
  readableScope: "file";
  grantId: string;
  expiresAt: string;
}

// ---------------------------------------------------------------------------
// §7 Lifecycle / receipts
// ---------------------------------------------------------------------------

export const RECEIPT_STATUSES = [
  "accepted",
  "acknowledged",
  "scheduled",
  "started",
  "completed",
  "cancelled",
  "expired",
  "unsupported",
  "failed",
  "uncertain",
] as const;

export type ReceiptStatus = (typeof RECEIPT_STATUSES)[number];

export function isReceiptStatus(v: unknown): v is ReceiptStatus {
  return typeof v === "string" && (RECEIPT_STATUSES as readonly string[]).includes(v);
}

export const RESOLUTIONS = ["exact", "substituted", "reduced", "unsupported", "failed"] as const;
export type Resolution = (typeof RESOLUTIONS)[number];

export function isResolution(v: unknown): v is Resolution {
  return typeof v === "string" && (RESOLUTIONS as readonly string[]).includes(v);
}

/** 終結狀態：之後不得再有任何回執。 */
export const TERMINAL_RECEIPT_STATUSES: readonly ReceiptStatus[] = [
  "completed",
  "cancelled",
  "expired",
  "unsupported",
  "failed",
  "uncertain",
];

export function isTerminalReceiptStatus(s: ReceiptStatus): boolean {
  return TERMINAL_RECEIPT_STATUSES.includes(s);
}

export interface CommandReceipt {
  messageId: string;
  characterInstanceId: string;
  generation: number;
  status: ReceiptStatus;
  resolution: Resolution;
  detail?: string;
  at: string;
  /** accepted{duplicate:true}（§4.4 去重）。 */
  duplicate?: boolean;
  /** cancelled{reason:"preempted"|"queue-full"|"busy"|"host"|…}（§5／§8）。 */
  reason?: string;
  /** cancelled{alreadyTerminal:true}（§7 cancel 冪等）。 */
  alreadyTerminal?: boolean;
}

/** 生命週期（§7）：文件列出 14 個狀態（含 crashed／reconnecting）。 */
export const ADAPTER_LIFECYCLE_STATES = [
  "discovered",
  "loading",
  "validated",
  "initializing",
  "negotiating",
  "ready",
  "shown",
  "hidden",
  "suspended",
  "resumed",
  "reconfiguring",
  "disposed",
  "crashed",
  "reconnecting",
] as const;

export type AdapterLifecycleState = (typeof ADAPTER_LIFECYCLE_STATES)[number];

export const CHARACTER_ROLES = [
  "primary-companion",
  "familiar",
  "worker",
  "observer",
  "notification-only",
] as const;

export type CharacterRole = (typeof CHARACTER_ROLES)[number];

/** 不接收輸入的角色（§6 多角色過濾）。 */
export const INPUT_SILENT_ROLES: readonly CharacterRole[] = ["observer", "notification-only"];

// ---------------------------------------------------------------------------
// §3.3 握手
// ---------------------------------------------------------------------------

export interface HelloLimits {
  maxMessageBytes: number;
  maxMessagesPerSecond: number;
  maxPending: number;
}

export interface Hello {
  type: "hello";
  protocolVersion: string;
  runtimeVersion: string;
  characterInstanceId: string;
  role: CharacterRole;
  locale: string;
  reducedMotion: boolean;
  requires: CharacterIntent[];
  limits: HelloLimits;
}

export interface Negotiate {
  type: "negotiate";
  protocolVersion: string;
  characterId: string;
  manifestVersion: string;
  capabilities: Record<string, CapabilityDecl>;
  inputCapabilities: Record<string, CapabilityDecl>;
  channels: string[];
  intents: CharacterIntent[];
  variants: string[];
  generation: number;
  /** 協商時 adapter 可附上 fallback 宣告（等同 manifest.fallbacks）。 */
  fallbacks?: FallbackDecl;
}

export interface IntentResolution {
  resolution: Resolution;
  /** 實際承載的能力 id，或 "system.text"；unsupported 時省略。 */
  via?: string;
  /** 步驟 2 替換後的實際 intent（substituted 時）。 */
  viaIntent?: CharacterIntent;
  variant?: string;
}

export interface Negotiated {
  type: "negotiated";
  characterInstanceId: string;
  generation: number;
  reducedMotion: boolean;
  resolutions: Record<CharacterIntent, IntentResolution>;
  acceptedChannels: string[];
  ignoredChannels: string[];
  /** acceptedChannels 中的 namespaced custom channel（§3.4：nonSafety，不得影響搶占）。 */
  nonSafetyChannels: string[];
  capabilities: Record<string, CapabilityDecl>;
}

// ---------------------------------------------------------------------------
// §8 Wire messages
// ---------------------------------------------------------------------------

export interface WireError {
  type: "error";
  code: "protocol-version" | "rate-limited" | "invalid-message" | "unsupported" | "internal";
  detail?: string;
}

export type WireMessage =
  | Hello
  | Negotiated
  | { type: "intent"; envelope: IntentEnvelope }
  | { type: "cancel"; messageId: string; reason?: string }
  | { type: "heartbeat"; at?: string }
  | WireError
  | { type: "goodbye"; reason?: string }
  | Negotiate
  | { type: "receipt"; receipt: CommandReceipt }
  | { type: "event"; event: CharacterInputEvent }
  | { type: "lifecycle"; state: AdapterLifecycleState; detail?: string };

// ---------------------------------------------------------------------------
// 限制（§2.1／§4.4／§6／§8）
// ---------------------------------------------------------------------------

export const LIMITS = {
  /** manifest 檔案大小上限（bytes）。 */
  manifestMaxBytes: 256 * 1024,
  maxAssets: 64,
  /** 單一資產大小絕對上限（resourceLimits.maxAssetBytes 不得超過）。 */
  maxAssetBytesCap: 32 * 1024 * 1024,
  localizedDisplayNameMaxChars: 48,
  localizedDescriptionMaxChars: 400,
  authorMaxChars: 120,
  pronounMaxChars: 16,
  preferencesMaxProperties: 32,
  preferencesStringMaxLength: 200,
  preferencesEnumMax: 16,
  durationMaxMs: 60_000,
  maxConcurrentCommandsCap: 16,
  maxQueueCap: 32,
  maxFpsCap: 120,
  maxVariants: 64,
  maxChannels: 64,
  maxStates: 256,
  /** §4.4：parameters ≤ 4 KB、字串 ≤ 200 字。 */
  parametersMaxBytes: 4096,
  stringMaxChars: 200,
  /** §8：單則訊息 ≤ 64 KB、每 adapter ≤ 50 則/s、pending ≤ 64、outbound ≤ 32。 */
  maxMessageBytes: 64 * 1024,
  maxMessagesPerSecond: 50,
  maxPending: 64,
  outboundQueue: 32,
  dedupeRing: 256,
  /** §6 輸入正規化。 */
  hoverPerSecond: 4,
  draggedPerSecond: 10,
  /** pointerProximity ≤ 1/30 s；文件語意取最嚴格解讀：30 秒一次。 */
  pointerProximityMinIntervalMs: 30_000,
  pointerGridPx: 8,
  inputQueue: 64,
  textSubmittedMaxChars: 2000,
  fileDropMaxFiles: 16,
  fileNameMaxChars: 255,
  fileGrantMaxMs: 10 * 60_000,
  actionIdMaxChars: 64,
  /** §7 外部 adapter heartbeat。 */
  heartbeatIntervalMs: 15_000,
  heartbeatTimeoutMs: 45_000,
  reconnectBackoffMinMs: 1_000,
  reconnectBackoffMaxMs: 15_000,
  /** acknowledged→uncertain 與 started 看門狗的寬限（本實作選定值）。 */
  acknowledgedGraceMs: 5_000,
  startedWatchdogGraceMs: 5_000,
  /** host 必須以此週期呼叫 gateway.sweep(now)。 */
  sweepIntervalMs: 500,
} as const;

/** 解析 "major.minor"；非法回 null。 */
export function parseProtocolVersion(v: unknown): { major: number; minor: number } | null {
  if (typeof v !== "string") return null;
  const m = /^(\d{1,4})\.(\d{1,4})$/.exec(v.trim());
  if (!m) return null;
  return { major: Number(m[1]), minor: Number(m[2]) };
}
