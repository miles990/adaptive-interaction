// Typed bridge to the Tauri backend. All commands go through the same runtime
// application services as the CLI and HTTP API; nothing here can bypass the
// policy governor.

import { UnlistenFn } from "@tauri-apps/api/event";
import { call, onError, onEvent, onReady } from "./transport";
import type { Envelope as AipEnvelope } from "./aip/generated";
import type {
  CharacterInputEvent,
  CharacterManifest,
  CharacterRole,
  CommandReceipt,
  LocalizedText,
  Negotiate,
  Negotiated,
} from "./character/protocol";

const invoke = call;

export interface ComponentHealth {
  status: "healthy" | "degraded" | "unhealthy" | "offline" | "unknown";
  message?: string;
}

export interface ReceptorManifest {
  id: string;
  name: string;
  description: string;
  category: string;
  mode: string;
  sensitivity: string;
  requiresConsent: boolean;
  availability: string;
  health: ComponentHealth;
  driver: string;
}

export interface ActuatorManifest {
  id: string;
  name: string;
  description: string;
  channel: string;
  riskClass: string;
  externalSideEffect: boolean;
  requiresConsent: boolean;
  supportsCancel: boolean;
  availability: string;
  health: ComponentHealth;
  limits: Record<string, number | undefined>;
  driver: string;
}

export interface ToolManifest {
  name: string;
  description: string;
  roles: string[];
  risk: string;
  requiresApproval: boolean;
  externalSideEffect: boolean;
  inputSchema: unknown;
  outputSchema: unknown;
}

export interface CapabilitySnapshot {
  receptors: ReceptorManifest[];
  actuators: ActuatorManifest[];
  toolOperations: ToolManifest[];
  constraints: { kind: string; detail: string }[];
  sessionPolicy: Record<string, unknown>;
  generatedAt: string;
  version: number;
}

export interface RuntimeEvent {
  eventId: string;
  sequence: number;
  eventType: string;
  timestamp: string;
  sessionId?: string;
  correlationId?: string;
  payload: Record<string, unknown>;
}

export interface Receipt {
  actionId: string;
  planId: string;
  actuatorId: string;
  intent: string;
  currentStatus: string;
  timestamps: [string, string][];
  policyDecisions: Record<string, unknown>[];
  effectiveBoundedParameters: Record<string, unknown>;
  requestedParameters: Record<string, unknown>;
  verification?: { verdict: string; detail?: string };
  errors: { code: string; message: string }[];
}

export interface Session {
  sessionId: string;
  state: string;
  startedAt: string;
  consents: { scope: { kind: string; id: string }; revokedAt?: string; expiresAt?: string }[];
  label?: string;
}

export interface HardwareScanReport {
  platform: string;
  startedAt: string;
  completedAt: string;
  sensorActivationAttempted: boolean;
  devices: {
    class: string;
    displayName: string;
    stableId?: string;
    identityBasis: string;
    availability: string;
    permissionRequirements: string[];
    capabilities: {
      id: string;
      kind: string;
      scope: string;
      read: boolean;
      write: boolean;
      requiresConsent: boolean;
      leavesDevice: boolean;
    }[];
    sourceAdapter: string;
    detail: string;
  }[];
  limitations: string[];
}

export const api = {
  status: () => invoke<Record<string, unknown>>("status"),
  capabilities: (includeUnavailable = true) =>
    invoke<CapabilitySnapshot>("capabilities", { includeUnavailable }),
  observationsQuery: (query: Record<string, unknown>) =>
    invoke<Record<string, unknown>[]>("observations_query", { query }),
  actionsList: (limit = 50) => invoke<Receipt[]>("actions_list", { limit }),
  actionGet: (actionId: string) => invoke<Receipt>("action_get", { actionId }),
  policyGet: () => invoke<Record<string, unknown>>("policy_get"),
  policyPatch: (patch: Record<string, unknown>) =>
    invoke<Record<string, unknown>>("policy_patch", { patch }),
  sessionGet: () => invoke<Session | null>("session_get"),
  sessionStart: (label: string, consents: string[]) =>
    invoke<Session>("session_start", { label, consents }),
  sessionStop: () => invoke("session_stop"),
  // maxUses 是後端真正的「只這一次」：第一次成功派工就用掉（Rust 端強制，
  // 不是畫面上的約定）。不帶＝維持原本「只受有效期間約束、次數不限」。
  consentGrant: (scope: string, expiresMinutes?: number, maxUses?: number) =>
    invoke<Session>("consent_grant", {
      scope,
      expiresMinutes: expiresMinutes ?? null,
      maxUses: maxUses ?? null,
    }),
  consentRevoke: (scope: string) => invoke<Session>("consent_revoke", { scope }),
  recipesList: () =>
    invoke<{ recipe: Record<string, unknown>; state: Record<string, unknown> }[]>("recipes_list"),
  recipeUpsert: (text: string) => invoke("recipe_upsert", { text }),
  recipeValidate: (text: string) =>
    invoke<{ valid: boolean; issues?: { field: string; message: string }[] }>("recipe_validate", {
      text,
    }),
  recipeSetEnabled: (id: string, enabled: boolean) =>
    invoke("recipe_set_enabled", { id, enabled }),
  recipeDelete: (id: string) => invoke("recipe_delete", { id }),
  recipeSimulate: (id: string) => invoke<Record<string, unknown>>("recipe_simulate", { id }),
  recipeRun: (id: string) => invoke<Record<string, unknown>>("recipe_run", { id }),
  toolsList: () => invoke<ToolManifest[]>("tools_list"),
  toolsExport: (format: string) => invoke<Record<string, unknown>>("tools_export", { format }),
  outbox: (limit = 30) =>
    invoke<{ channel: string; intent: string; text?: string; at: string }[]>("outbox_recent", {
      limit,
    }),
  auditTail: (limit = 50) => invoke<Record<string, unknown>[]>("audit_tail", { limit }),
  eventsRecent: (limit = 100) => invoke<RuntimeEvent[]>("events_recent", { limit }),
  setReceptorEnabled: (id: string, enabled: boolean) =>
    invoke("set_receptor_enabled", { id, enabled }),
  setActuatorEnabled: (id: string, enabled: boolean) =>
    invoke("set_actuator_enabled", { id, enabled }),
  testReceptor: (id: string) => invoke<Record<string, unknown>>("test_receptor", { id }),
  testActuator: (id: string) => invoke<Receipt[]>("test_actuator", { id }),
  pushObservation: (receptorId: string, facts: Record<string, unknown>, confidence = 1.0) =>
    invoke("push_observation", { receptorId, facts, confidence }),
  providersList: () => invoke<Record<string, unknown>[]>("providers_list"),
  /** 「測試裝置」：唯讀測一次（只讀第一個可讀受器，不觸發任何動器）。 */
  providerTest: (id: string) => invoke<ProviderTestReport>("provider_test", { id }),
  hardwareScan: () => invoke<HardwareScanReport>("hardware_scan"),
  activityInbox: (filter: ActivityInboxFilter = {}) =>
    invoke<Record<string, unknown>>("activity_inbox", { filter }),
  agentsDiscoveries: () => invoke<Record<string, unknown>>("agents_discoveries"),
  agentsRefresh: () => invoke<Record<string, unknown>>("agents_refresh"),
  agentsRouting: (kind?: string) =>
    invoke<Record<string, unknown>>("agents_routing", { kind: kind ?? null }),
  agentSessionApprove: (id: string, requestId: string, approve: boolean) =>
    invoke<Record<string, unknown>>("agent_session_approve", { id, requestId, approve }),
  agentSessionInterrupt: (id: string) =>
    invoke<Record<string, unknown>>("agent_session_interrupt", { id }),
  memoryList: (layer?: string, limit = 200) =>
    invoke<Record<string, unknown>>("memory_list", { layer: layer ?? null, limit }),
  memoryCreate: (input: Record<string, unknown>) =>
    invoke<Record<string, unknown>>("memory_create", { input }),
  memoryPatch: (id: string, patch: Record<string, unknown>) =>
    invoke<Record<string, unknown>>("memory_patch", { id, patch }),
  memoryDelete: (id: string) => invoke<Record<string, unknown>>("memory_delete", { id }),
  memoryExport: () => invoke<Record<string, unknown>>("memory_export"),
  memoryClearSession: () => invoke<Record<string, unknown>>("memory_clear_session"),
  memoryBundle: (task: string, agentId: string, domains: string[]) =>
    invoke<Record<string, unknown>>("memory_bundle", { task, agentId, domains }),
  knowledgeList: (status?: string, limit = 100) =>
    invoke<Record<string, unknown>>("knowledge_list", { status: status ?? null, limit }),
  domainPacks: () => invoke<Record<string, unknown>>("domain_packs"),
  domainPackInstall: (id: string) =>
    invoke<Record<string, unknown>>("domain_pack_install", { id }),
  domainPackUninstall: (id: string) =>
    invoke<Record<string, unknown>>("domain_pack_uninstall", { id }),
  knowledgeSearch: (q: string, k = 10) =>
    invoke<Record<string, unknown>>("knowledge_search", { q, k }),
  knowledgeGet: (id: string) => invoke<Record<string, unknown>>("knowledge_get", { id }),
  knowledgeReview: (id: string, verdict: string, note?: string) =>
    invoke<Record<string, unknown>>("knowledge_review", { id, verdict, note: note ?? null }),
  knowledgeGraph: (id: string) => invoke<Record<string, unknown>>("knowledge_graph", { id }),
  knowledgeReceipts: () => invoke<Record<string, unknown>>("knowledge_receipts"),
  // 更新決策器（spec §13）：純檢查、零副作用——回傳的是決策，不是更新結果。
  knowledgeUpdateCheck: (trigger: string) =>
    invoke<UpdateDecision>("knowledge_update_check", { trigger }),
  knowledgeUserCorrection: (input: {
    originalAssumption?: string;
    correction: string;
    scope?: string;
  }) => invoke<Record<string, unknown>>("knowledge_user_correction", { input }),
  assetsList: () => invoke<Record<string, unknown>>("assets_list"),
  assetImport: (input: { path?: string; content?: string; description?: string }) =>
    invoke<Record<string, unknown>>("asset_import", input),
  assetDerivatives: (hash: string) =>
    invoke<Record<string, unknown>>("asset_derivatives", { hash }),
  assetDerive: (hash: string) => invoke<Record<string, unknown>>("asset_derive", { hash }),
  assetPreview: (hash: string) => invoke<Record<string, unknown>>("asset_preview", { hash }),
  assetImpact: (hash: string) => invoke<Record<string, unknown>>("asset_impact", { hash }),
  assetDelete: (hash: string) => invoke<Record<string, unknown>>("asset_delete", { hash }),
  proactiveDialogueGet: () => invoke<Record<string, unknown>>("proactive_dialogue_get"),
  proactiveDialoguePatch: (patch: Record<string, unknown>) =>
    invoke<Record<string, unknown>>("proactive_dialogue_patch", { patch }),
  proactiveDialogueQuiet: (minutes: number) =>
    invoke<Record<string, unknown>>("proactive_dialogue_quiet", { minutes }),
  presentationStatus: () => invoke<Record<string, unknown>>("presentation_status"),
  presentationHello: (
    visible: boolean,
    packId?: string,
    behaviorState?: Record<string, unknown>
  ) =>
    invoke<Record<string, unknown>>("presentation_hello", {
      visible,
      packId: packId ?? null,
      behaviorState: behaviorState ?? null,
    }),
  presentationAck: (actionId: string, outcome: string, detail?: string) =>
    invoke<Record<string, unknown>>("presentation_ack", {
      actionId,
      outcome,
      detail: detail ?? null,
    }),
  // ---- Character Presentation Protocol（docs/character-protocol/README.md §8.1） ----
  /** 桌面視窗（可信 host）向 Runtime 註冊／重新協商角色實例。同一 instanceId 再 hello ＝ 重協商（generation+1）。 */
  characterHello: (input: CharacterHelloInput) =>
    invoke<CharacterHelloResult>("character_hello", {
      instanceId: input.instanceId ?? "desktop-companion",
      role: input.role ?? "primary-companion",
      manifest: input.manifest,
      negotiate: input.negotiate,
      visible: input.visible,
      packId: input.packId ?? null,
      behaviorState: input.behaviorState ?? null,
      reducedMotion: input.reducedMotion ?? false,
    }),
  /** 角色演出回執（accepted≠started≠completed；completed 永遠只是「演完了」）。 */
  characterReceipt: (instanceId: string, receipt: CommandReceipt) =>
    invoke<CharacterReceiptResult>("character_receipt", { instanceId, receipt }),
  /** 正規化後的角色輸入事件（Runtime 轉成 receptor observation，仍經 policy／consent）。 */
  characterEvent: (instanceId: string, event: CharacterInputEvent) =>
    invoke<CharacterEventResult>("character_event", { instanceId, event }),
  // ---- AIP Character Session（docs/aip/transport-bindings.md §2；human token） ----
  /** 權威快照（一則 `state{kind:"snapshot"}` envelope）。讀不到就是讀不到，不得用上一次冒充。 */
  characterSessionSnapshot: () => invoke<CharacterSessionEnvelope>("character_session_snapshot"),
  /** 對齊：回補丁或完整快照（形狀見 transport-bindings §1.3）。 */
  characterSessionResume: (input: {
    lastRevision: number;
    lastSequence?: number;
    epoch?: number;
  }) =>
    invoke<Record<string, unknown>>("character_session_resume", {
      lastRevision: input.lastRevision,
      lastSequence: input.lastSequence ?? 0,
      epoch: input.epoch ?? 0,
    }),
  /** 可信 host surface（桌面視窗）送語意事件；身分由後端綁定，宣稱不符一律拒絕。 */
  characterSessionEvent: (envelope: CharacterSessionEnvelope) =>
    invoke<CharacterSessionEnvelope>("character_session_events", { envelope }),
  /** 進階模式的連接診斷（不含 token、路徑、原始內容）；一般模式不顯示這些。 */
  characterSessionDiagnostics: () =>
    invoke<CharacterSessionDiagnostics>("character_session_diagnostics"),
  characterInstances: () => invoke<{ instances: CharacterInstanceView[] }>("character_instances"),
  characterManifest: () => invoke<CharacterManifest>("character_manifest"),
  /** 已登記的外部角色 adapter 清單（永遠不含 token 或其 hash）。 */
  characterAdapters: () => invoke<{ adapters: CharacterAdapterView[] }>("character_adapters"),
  /** 撤銷外部角色 adapter：token 立即失效並斷線，且不會自己回來。 */
  characterAdapterRevoke: (adapterId: string) =>
    invoke<CharacterAdapterRevokeResult>("character_adapter_revoke", { adapterId }),
  createPlan: (input: Record<string, unknown>) =>
    invoke<Record<string, unknown>>("create_plan", { input }),
  simulatePlan: (planId: string) => invoke<Record<string, unknown>>("simulate_plan", { planId }),
  executePlan: (planId: string) => invoke<Receipt[]>("execute_plan", { planId }),
  cancelAction: (actionId: string) => invoke<Receipt>("cancel_action", { actionId }),
  verifyAction: (actionId: string) => invoke<Receipt>("verify_action", { actionId }),
  emergencyStop: (reason?: string) =>
    invoke<Record<string, unknown>>("emergency_stop", { reason: reason ?? null }),
  emergencyStopClear: () => invoke("emergency_stop_clear"),
  // ---- human layer ----
  catalogGet: () => invoke<Catalog>("catalog_get"),
  capabilitiesHuman: (locale?: string, includeUnavailable = true) =>
    invoke<HumanCapabilities>("capabilities_human", { locale: locale ?? null, includeUnavailable }),
  uiPrefsGet: () => invoke<UiPreferences>("ui_prefs_get"),
  uiPrefsPatch: (patch: Record<string, unknown>) =>
    invoke<UiPreferences>("ui_prefs_patch", { patch }),
  onboardingGet: () => invoke<OnboardingState>("onboarding_get"),
  onboardingDraft: (draft: Record<string, unknown>) => invoke("onboarding_draft", { draft }),
  /** 套用前試算：與 commit 同一套驗證，不會改任何設定。 */
  onboardingPreview: (commit: Record<string, unknown>) =>
    invoke<OnboardingPreview>("onboarding_preview", { commit }),
  onboardingCommit: (commit: Record<string, unknown>) =>
    invoke<Record<string, unknown>>("onboarding_commit", { commit }),
  pauseGet: () => invoke<PauseState>("pause_get"),
  pauseSet: (durationMinutes?: number, reason?: string) =>
    invoke<PauseState>("pause_set", {
      durationMinutes: durationMinutes ?? null,
      reason: reason ?? null,
    }),
  pauseClear: () => invoke<PauseState>("pause_clear"),
  aiAssistsList: () => invoke<PendingAssist[]>("ai_assists_list"),
  aiAssistResolve: (requestId: string, decision: "proceed" | "no-action", note?: string) =>
    invoke<Record<string, unknown>>("ai_assist_resolve", {
      requestId,
      decision,
      note: note ?? null,
    }),
  planGet: (planId: string) => invoke<Record<string, unknown>>("plan_get", { planId }),
  recipeSummary: (id: string, locale?: string) =>
    invoke<{ recipeId: string; summary: string }>("recipe_summary", {
      id,
      locale: locale ?? null,
    }),
  recipeSimulateScenario: (id: string, scenario: Record<string, unknown>) =>
    invoke<ScenarioReport>("recipe_simulate_scenario", { id, scenario }),
  recipeConvert: (text: string, to: "yaml" | "json") =>
    invoke<ConvertResult>("recipe_convert", { text, to }),
  recipeGet: (id: string) => invoke<Record<string, unknown>>("recipe_get", { id }),
  agentSessionsList: () => invoke<AgentSessionRecord[]>("agent_sessions_list"),
  agentSessionCreate: (input: Record<string, unknown>) =>
    invoke<AgentSessionRecord>("agent_session_create", { input }),
  agentSessionMessages: (id: string, direction: string) =>
    invoke<Record<string, unknown>[]>("agent_session_messages", { id, direction }),
  agentSessionSend: (id: string, kind: string, body: Record<string, unknown>) =>
    invoke<Record<string, unknown>>("agent_session_send", { id, kind, body }),
  agentSessionRenew: (id: string, extraMinutes = 30) =>
    invoke<AgentSessionRecord>("agent_session_renew", { id, extraMinutes }),
  agentSessionClose: (id: string, reason?: string) =>
    invoke<AgentSessionRecord>("agent_session_close", { id, reason: reason ?? null }),
  agentSessionVerify: (id: string, note?: string) =>
    invoke<AgentSessionRecord>("agent_session_verify", { id, note: note ?? null }),
  mobileStatus: () => invoke<Record<string, unknown>>("mobile_status"),
  mobilePairingBegin: () => invoke<Record<string, unknown>>("mobile_pairing_begin"),
  mobileRevoke: (id: string) => invoke<Record<string, unknown>>("mobile_revoke", { id }),
  /** 只對這一台手機要求停止感測。送出≠停止：`outcome` 是 stopped 才可以說「已停止」。 */
  mobileSensorsStop: (id: string) => invoke<MobileSensorsStopResult>("mobile_sensors_stop", { id }),
  /** 只測連線還在不在。有回應不代表手機 App 的功能可用；沒有回應一律「結果不確定」。 */
  mobileTest: (id: string) => invoke<MobileTestResult>("mobile_test", { id }),
  /** BLE 閘道代掃。`deviceId` 指名由哪一台手機掃；不指定時只有恰好一台
   *  手機連線才成立（多台連線時後端誠實回錯，不替你挑一台）。 */
  mobileBleScan: (durationMs = 4000, deviceId?: string) =>
    invoke<Record<string, unknown>>("mobile_ble_scan", {
      durationMs,
      deviceId: deviceId ?? null,
    }),
  sensorMicListen: (durationMs: number) =>
    invoke<Record<string, unknown>>("sensor_mic_listen", { durationMs }),
  sensorsStop: () => invoke<SensorStopReport>("sensors_stop"),
};

// ---- AIP Character Session types（與 crates/interaction-aip 的 wire 一致） ----

/** AIP 1.0 信封（型別由 `scripts/aip-codegen.mjs` 從 golden schema 產生）。 */
export type CharacterSessionEnvelope = AipEnvelope;

/** `GET /v1/character-session/diagnostics`（進階模式限定；不含 token、路徑、原始內容）。 */
export interface CharacterSessionDiagnostics {
  sessionId: string;
  sessionEpoch: number;
  revision: number;
  sequence: number;
  members: {
    party: { kind: string; id: string };
    role: string;
    presence: string;
    lastSeenAt: string;
  }[];
  counters: Record<string, number>;
  eventLog: { len: number; cap: number };
  /** 不是 null＝保存的角色狀態讀不回來、session 真的被重建了；一般模式要翻成人話，不得靜默。 */
  storeNote: string | null;
  /**
   * 持久化 store 的健康度（選填；舊 Runtime 沒有）。`storeNote` 只在 session 真的被重建時非 null；
   * 遷移（舊格式→現行格式、session 沒重建）放在這裡的 `migratedFrom`／`migrationNote`。
   * `parked`＝這一輪什麼都不會存（讀不到／未來格式／備份失敗），`note` 是固定文字的原因。
   */
  store?: {
    format: number | null;
    migratedFrom: number | null;
    migrationNote: string | null;
    lastPersistedRevision: number | null;
    persistFailures: number;
    skippedStale: number;
    parked: boolean;
    lastPersistError: string | null;
    note: string | null;
  };
}

// ---- Character Presentation Protocol types（與 crates/interaction-character 的 wire 一致） ----

export interface CharacterHelloInput {
  /** 預設 "desktop-companion"。 */
  instanceId?: string;
  /** 預設 "primary-companion"。 */
  role?: CharacterRole;
  manifest: CharacterManifest;
  negotiate: Negotiate;
  visible: boolean;
  /** 相容：presence 的 packId（= characterId）。 */
  packId?: string;
  behaviorState?: Record<string, unknown>;
  /** 視窗目前的 Reduced Motion；Runtime 以它協商（`reduced`），是這項設定的唯一來源。 */
  reducedMotion?: boolean;
}

export interface CharacterHelloResult {
  instanceId: string;
  /** Runtime 端的世代；之後轉送的回執要帶這個值。 */
  generation: number;
  negotiated: Negotiated;
}

export interface CharacterReceiptResult {
  accepted: boolean;
  status?: string;
}

export interface CharacterEventResult {
  decision: "queued" | "merged" | "throttled" | "dropped" | string;
  reason?: string;
}

export interface CharacterInstanceView {
  instanceId: string;
  characterId: string;
  displayName: LocalizedText;
  role: CharacterRole | string;
  generation: number;
  lifecycle: string;
  connected: boolean;
  negotiated: boolean;
  pending: number;
  adapterKind: string;
  origin: "builtin" | "imported" | "external" | string;
  executable: boolean;
  network: boolean;
  tested: boolean;
  /** 外部 adapter 實例才有；內建／匯入角色為 null。 */
  adapterId?: string | null;
  /** 這次協商採用的 Reduced Motion；尚未協商為 null（不假裝 false）。 */
  reducedMotion?: boolean | null;
  /** manifest 作者／版本／支援的 input capability id（Phase 8 F1 起由 Runtime 回報）。 */
  author?: string | null;
  version?: string;
  inputCapabilities?: string[];
}

/** `GET /v1/character/adapters` 的一筆（永遠不含 token）。 */
export interface CharacterAdapterView {
  adapterId: string;
  displayName: string;
  characterId: string;
  createdAt: string;
  revoked: boolean;
  connected: boolean;
  /** 登記時的 manifest 摘要（Phase 8 F1 起由 Runtime 回報；舊 daemon 可能沒有）。 */
  characterDisplayName?: LocalizedText;
  author?: string | null;
  version?: string;
  inputCapabilities?: string[];
  adapterKind?: string;
  executable?: boolean;
  network?: boolean;
}

export interface CharacterAdapterRevokeResult {
  adapterId: string;
  revoked: boolean;
  disconnected: boolean;
}

/** 知識更新決策（spec §13 決策表輸出；camelCase 由後端 serde 保證）。 */
export interface UpdateDecision {
  trigger: string;
  needsUpdate: boolean;
  needsAi: boolean;
  requiresUserAsk: boolean;
  deterministicSteps: string[];
  aiSteps: string[];
  reason: string;
}

export interface SensorUse {
  kind: string;
  startedAt: string;
  startedBy: string;
  purpose: string;
  autoStopAt?: string;
  /** 這個感測來源目前的狀態（runtime `sensors.rs`）：
   *  `active` ＝仍在擷取（已確認）；`stopping` ＝已要求停止、還在有界等待確認；
   *  `stop-unknown` ＝等不到確認（來源沒回報，可能仍在擷取）。
   *  後兩者**仍然是感測中**，介面不得因此隱藏。舊 daemon 不送這個欄位
   *  （undefined），呼叫端要把 undefined 視同 `active`；不認得的字串一律當成
   *  「可能仍在使用」，不得當作已停止。 */
  state?: "active" | "stopping" | "stop-unknown" | (string & {});
}

/** `/v1/sensors/stop` 對單一裝置的回報。`outcome` 只有 `stopped` 才是「已停止」，
 *  `unknown`／`unreachable`（或任何不認得的值）都是結果不確定，不是失敗也不是成功。 */
export interface SensorStopDeviceReport {
  deviceId?: string;
  name?: string;
  outcome?: string;
  waitedMs?: number;
}

/** 非手機來源（runtime `SensorStopReport`；M2 §3.1 的 `SensorSource` port）對一次停止請求的逐筆回報：
 *  例如經宣告式 adapter 的 Serial／MQTT 裝置。`outcome` 只有 `stopped`／`already-stopped` 算「確認沒在擷取」；
 *  `unknown`（沒回覆）、`unreachable`（沒送到）、`refused`（明確拒絕）都可能還在擷取。
 *  `sourceId`／`declarationId` 是內部 id，一般模式不上畫面；人話名稱看 `sourceLabel`。 */
export interface SensorStopSourceReport {
  sourceId?: string;
  declarationId?: string;
  sourceLabel?: string;
  sensors?: string[];
  outcome?: "stopped" | "already-stopped" | "unknown" | "unreachable" | "refused" | (string & {});
  waitedMs?: number;
  confirmedVia?: string;
  detail?: string;
}

/** `/v1/sensors/stop` 的回報。舊 daemon 只回 `{stopped:true}`（沒有 uncertain／local／
 *  devices）——呼叫端必須容忍缺欄位，並以重新讀取 status 的 activeSensors 為準。 */
export interface SensorStopReport {
  stopped?: boolean;
  /** 有任何來源沒能確認停止（手機未回覆…）。舊 daemon 不送，缺席不代表確定停了。 */
  uncertain?: boolean;
  /** 本機擷取的結果（runtime `LocalStopReport`）：`stopped` ＝本來在擷取、已停；
   *  `idle` ＝本來就沒有在擷取。**不是布林值**——舊程式碼曾誤宣告成 boolean。 */
  local?: { microphone?: "stopped" | "idle" | (string & {}) };
  devices?: SensorStopDeviceReport[];
  /** 非手機來源的逐筆結果（本機擷取投影在 `local`，不重複列）。舊 daemon 不送。 */
  sources?: SensorStopSourceReport[];
}

/** 單台手機的「停止感測」結果（runtime `mobile_sensors_stop`）。
 *  `requested` 只代表指令送出去了；手機那邊的事實看 `outcome`：
 *  `stopped`＝手機回報已停止；`unknown`＝送出了但還沒回報（結果不確定）；
 *  `unreachable`＝根本沒送到（手機未連線）。UI 不得把 requested 說成已停止。 */
export type MobileSensorsStopResult = {
  deviceId: string;
  requested: boolean;
  connected: boolean;
  outcome: "stopped" | "unknown" | "unreachable" | string;
  waitedMs?: number;
};

/** 單台手機的「測試連接」結果（runtime `mobile_test`）。
 *  `ok`＝這一次來回有回應——只證明連線還在，不證明手機 App 的功能可用；
 *  逾時／沒有回應時 `uncertain` 為 true，一律標「結果不確定」，不得說成失敗或成功。 */
export type MobileTestResult = {
  deviceId: string;
  ok: boolean;
  connected: boolean;
  latencyMs?: number;
  uncertain?: boolean;
  reason?: string;
};

/** 統一收件匣查詢（activity.rs `ActivityInboxFilter`）。後端是 deny_unknown_fields：
 *  只能送它認得的鍵。`needsDecision` 是 v0.5 新增的「只要待你決定的項目」篩選——
 *  舊 daemon 會整筆拒絕，呼叫端（ConnectPage `loadDecisionInbox`）必須退回不帶它的查詢，
 *  再用 `pendingCount` 對照本頁，不得把「這一頁沒有」說成「沒有待決定」。 */
export interface ActivityInboxFilter {
  status?: string;
  agent?: string;
  device?: string;
  task?: string;
  domain?: string;
  since?: string;
  limit?: number;
  needsDecision?: boolean;
}

/** safety-event 項目的 status（activity.rs）：緊急停止的「解除」與「啟動」是兩個不同的 status；
 *  title 已是人話（原始 event_type 在 `detail.eventType`）。 */
export type SafetyEventStatus = "emergency" | "emergency-cleared" | "sensor.started" | "sensor.stopped";

export interface AgentSessionRecord {
  sessionId: string;
  providerId: string;
  agentId: string;
  label?: string;
  state: string;
  lease: { issuedAt: string; expiresAt: string; renewable: boolean };
  dataScope: string[];
  toolScope: string[];
  consentScope: string[];
  budget: { maxMessages: number; spentMessages: number; maxCost: number; spentCost: number };
  /** 允許修改工作區檔案。後端欄位由 gateway cluster 提供；缺席（舊後端）
   *  一律視為唯讀——徽章依此欄位呈現，絕不依建立時的請求值宣稱可寫。 */
  allowWrite?: boolean;
  providerSessionId?: string;
  /** 上一次**真的**掛上子程序的工作目錄（後端正規化後的絕對路徑）。
   *  這是續開時唯一可信的資料夾來源：`dataScope` 裡的 `workspace:` 只是
   *  呼叫端自己附加的人話標籤，不代表子程序實際被掛在哪裡。舊記錄／純對話
   *  session 沒有這個欄位。 */
  resolvedWorkdir?: string;
  /** 最新一次 claimed-completed 的識別；每個新的聲稱都拿到新 id。 */
  claimId?: string;
  /** 人工驗證（human-only）：存在＝人類確認過**目前這個** claim；缺席＝仍只是聲稱。
   *  後端在新任務送達、新一輪工作或新的聲稱到來時會清掉它；`claimId` 指向被驗證的
   *  那個 claim，介面只在 `humanVerified.claimId === record.claimId`（或兩者皆缺席的
   *  舊資料）時顯示綠勾。 */
  humanVerified?: { at: string; note?: string; claimId?: string };
  createdAt: string;
  closedAt?: string;
  detail?: string;
}

// ---- human layer types ----

export type TriState = boolean | "unknown";

export interface HumanBadge {
  key: string;
  label: string;
  tone: "info" | "ok" | "warn" | "danger" | string;
}

export interface HumanCard {
  id: string;
  kind: "receptor" | "actuator" | "tool-operation";
  displayName: string;
  nameSource: string;
  shortDescription?: string;
  descriptionSource: string;
  longDescription?: string;
  examples?: string[];
  setupInstructions?: string;
  icon: string;
  colorRole: string;
  category: string;
  beginnerRecommended: boolean;
  canonicalId?: string;
  badges: HumanBadge[];
  data?: {
    personalData: TriState;
    sensitivity: string;
    source: string;
    leavesDevice: TriState;
    retention: string;
    categories?: string[];
    factFields?: string[];
    inferenceFields?: string[];
  };
  effect?: {
    externalSideEffect: TriState;
    physicalEffect: TriState;
    interruptiveness: string;
    reversible: TriState;
    confirmationLevel: string;
    affects?: string[];
  };
  consent: { required: TriState; reason?: string; suggestedScope?: string };
  typical?: Record<string, unknown>;
  riskNote?: string;
  aiDescription?: string;
  undescribed: boolean;
  conservativeNotice?: string;
  availability: string;
  requiresConsent: boolean;
  riskClass?: string;
  channel?: string;
  driver?: string;
  manifestHash: string;
}

/** 「已測試」證據（spec §9.3）：掃描到 metadata／設定檔存在都不算測過。 */
export interface ProviderTested {
  at: string;
  /** handshake＝裝置連線握手成功；capability＝受器讀到／動器回 ack；human＝使用者按了測試。 */
  how: string;
  ok: boolean;
  note?: string;
  /**
   * 這次握手的配對碼從未被任何一方比對過（裝置在 hello 說它不需要配對，
   * 韌體對任何碼都回 pair-ok）：身分證據只剩裝置自報的 deviceId。
   * 缺席＝Runtime 沒有標記（舊記錄或真的比對過），不得當成「未驗證」。
   */
  pairingUnverified?: boolean;
}

export interface ProviderTestReport {
  providerId: string;
  ok: boolean;
  receptorId: string | null;
  reason?: string;
  observation?: Record<string, unknown>;
  tested?: ProviderTested;
}

export interface HumanCapabilities {
  locale: string;
  catalogVersion: number;
  capabilityVersion: number;
  generatedAt: string;
  constraints: { kind: string; detail: string }[];
  receptors: HumanCard[];
  actuators: HumanCard[];
  toolOperations: HumanCard[];
}

export interface Catalog {
  schemaVersion: string;
  version: number;
  entries: Record<string, unknown>[];
}

export interface UiPreferences {
  mode: "simple" | "advanced";
  locale: string;
  appearance?: "system" | "dark" | "light";
  scalePercent?: number;
  reduceMotion?: boolean;
  disabledAgents?: string[];
  agentRoutes?: Record<string, "codex" | "claude-code" | "none">;
  customNames: Record<string, string>;
  /** 首次成功體驗已看過（host 保存；純 UI 旗標，不影響任何權限）。 */
  firstSuccessSeen?: boolean;
  schemaVersion: string;
}

export interface OnboardingState {
  completed: boolean;
  completedAt?: string | null;
  draft?: Record<string, unknown> | null;
  starterRecipes: { id: string; title: string }[];
}

/** 一項能力在套用前後的狀態（from/to 由後端以目前真實狀態計算）。 */
export interface OnboardingChange {
  id: string;
  from: "on" | "off";
  to: "on" | "off";
  changed: boolean;
}

/** 套用前試算的結果；沒有任何副作用。 */
export interface OnboardingPreview {
  receptors: OnboardingChange[];
  actuators: OnboardingChange[];
  starterRecipes: { id: string; exists: boolean }[];
  policyPatch: Record<string, unknown> | null;
  preferences: Record<string, unknown> | null;
  changed: boolean;
}

export interface PauseState {
  paused: boolean;
  until?: string;
  reason?: string;
  pausedAt?: string;
}

export interface PendingAssist {
  requestId: string;
  recipeId: string;
  reason: string;
  createdAt: string;
  deadline: string;
  onUnavailable: string;
  dataScope: string[];
}

export interface ScenarioReport {
  recipeId: string;
  scenario: Record<string, unknown>;
  stages: Record<string, unknown>[];
  wouldExecute: boolean;
  sideEffects: string;
}

export interface ConvertResult {
  valid: boolean;
  issues?: { field: string; message: string }[];
  recipe?: Record<string, unknown>;
  text?: string;
}

export function onRuntimeEvent(handler: (event: RuntimeEvent) => void): Promise<UnlistenFn> {
  return onEvent<RuntimeEvent>(handler);
}

export function onRuntimeReady(handler: () => void): Promise<UnlistenFn> {
  return onReady(handler);
}

export function onRuntimeError(handler: (message: string) => void): Promise<UnlistenFn> {
  return onError(handler);
}
