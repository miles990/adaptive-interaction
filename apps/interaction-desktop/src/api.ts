// Typed bridge to the Tauri backend. All commands go through the same runtime
// application services as the CLI and HTTP API; nothing here can bypass the
// policy governor.

import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

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
  createPlan: (input: Record<string, unknown>) =>
    invoke<Record<string, unknown>>("create_plan", { input }),
  simulatePlan: (planId: string) => invoke<Record<string, unknown>>("simulate_plan", { planId }),
  executePlan: (planId: string) => invoke<Receipt[]>("execute_plan", { planId }),
  cancelAction: (actionId: string) => invoke<Receipt>("cancel_action", { actionId }),
  verifyAction: (actionId: string) => invoke<Receipt>("verify_action", { actionId }),
  emergencyStop: (reason?: string) =>
    invoke<Record<string, unknown>>("emergency_stop", { reason: reason ?? null }),
  emergencyStopClear: () => invoke("emergency_stop_clear"),
};

export function onRuntimeEvent(handler: (event: RuntimeEvent) => void): Promise<UnlistenFn> {
  return listen<RuntimeEvent>("runtime-event", (e) => handler(e.payload));
}

export function onRuntimeReady(handler: () => void): Promise<UnlistenFn> {
  return listen("runtime-ready", () => handler());
}

export function onRuntimeError(handler: (message: string) => void): Promise<UnlistenFn> {
  return listen<string>("runtime-error", (e) => handler(e.payload));
}
