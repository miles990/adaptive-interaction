// Typed bridge to the Tauri backend. All commands go through the same runtime
// application services as the CLI and HTTP API; nothing here can bypass the
// policy governor.

import { UnlistenFn } from "@tauri-apps/api/event";
import { call, onError, onEvent, onReady } from "./transport";

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
  consentGrant: (scope: string, expiresMinutes?: number) =>
    invoke<Session>("consent_grant", { scope, expiresMinutes: expiresMinutes ?? null }),
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
  presentationStatus: () => invoke<Record<string, unknown>>("presentation_status"),
  presentationHello: (visible: boolean, packId?: string) =>
    invoke<Record<string, unknown>>("presentation_hello", { visible, packId: packId ?? null }),
  presentationAck: (actionId: string, outcome: string, detail?: string) =>
    invoke<Record<string, unknown>>("presentation_ack", {
      actionId,
      outcome,
      detail: detail ?? null,
    }),
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
  agentSessionSend: (id: string, kind: string, body: Record<string, unknown>) =>
    invoke<Record<string, unknown>>("agent_session_send", { id, kind, body }),
  agentSessionClose: (id: string, reason?: string) =>
    invoke<AgentSessionRecord>("agent_session_close", { id, reason: reason ?? null }),
  sensorMicListen: (durationMs: number) =>
    invoke<Record<string, unknown>>("sensor_mic_listen", { durationMs }),
  sensorsStop: () => invoke("sensors_stop"),
};

export interface SensorUse {
  kind: string;
  startedAt: string;
  startedBy: string;
  purpose: string;
  autoStopAt?: string;
}

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
  customNames: Record<string, string>;
  schemaVersion: string;
}

export interface OnboardingState {
  completed: boolean;
  completedAt?: string | null;
  draft?: Record<string, unknown> | null;
  starterRecipes: { id: string; title: string }[];
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
