// Transport layer: the SAME typed api surface works over two backends.
//
// 1. Tauri IPC (embedded runtime; the desktop app owns the runtime).
// 2. HTTP against a local `interact-ai serve` daemon (external-daemon mode
//    and browser-level E2E tests). Same endpoints, same policy governor —
//    nothing here can bypass authorization, it only changes the wire.
//
// Human-confirmation surfaces stay honest across modes: the backend treats
// HTTP as the AI-host surface, so `ai_assist_resolve` over HTTP can never
// satisfy `requireHumanConfirmation` (the runtime enforces this; we do not
// pretend otherwise here).

import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

export const isTauri =
  typeof window !== "undefined" &&
  ("__TAURI_INTERNALS__" in window || "__TAURI__" in window);

// ---------------------------------------------------------------------------
// HTTP configuration (non-Tauri browser mode, or Tauri connected-to-external)
// ---------------------------------------------------------------------------

export interface HttpConfig {
  base: string;
  token: string;
}

let httpConfig: HttpConfig | null = null;
let httpMode = !isTauri;

/** Switch this window to HTTP mode against a daemon (used by the desktop app
 *  when a daemon already owns the runtime, and by browser E2E). */
export function configureHttp(base: string, token: string) {
  httpConfig = { base: base.replace(/\/+$/, ""), token };
  httpMode = true;
}

export function transportMode(): "tauri" | "http" {
  return httpMode ? "http" : "tauri";
}

function cfg(): HttpConfig {
  if (httpConfig) return httpConfig;
  // Browser mode bootstrap (local dev / E2E only): ?api=…&token=… or env.
  const params = new URLSearchParams(window.location.search);
  const env = (import.meta as unknown as { env?: Record<string, string> }).env ?? {};
  const base = params.get("api") ?? env.VITE_API_BASE ?? "http://127.0.0.1:8787";
  const token =
    params.get("token") ??
    env.VITE_API_TOKEN ??
    window.localStorage.getItem("interaction-api-token") ??
    "";
  httpConfig = { base: base.replace(/\/+$/, ""), token };
  return httpConfig;
}

async function http(method: string, path: string, body?: unknown): Promise<unknown> {
  const { base, token } = cfg();
  const res = await fetch(`${base}${path}`, {
    method,
    headers: {
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
      ...(body !== undefined ? { "Content-Type": "application/json" } : {}),
    },
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  const text = await res.text();
  let json: unknown = null;
  try {
    json = text ? JSON.parse(text) : null;
  } catch {
    json = text;
  }
  if (!res.ok) {
    const record = json && typeof json === "object" ? (json as Record<string, unknown>) : null;
    const nested =
      record?.error && typeof record.error === "object"
        ? (record.error as Record<string, unknown>)
        : null;
    const detail =
      (nested && typeof nested.message === "string" && nested.message) ||
      (record && typeof record.message === "string" && record.message) ||
      text ||
      res.statusText;
    throw new Error(`${res.status}: ${detail}`);
  }
  return json;
}

// ---------------------------------------------------------------------------
// Command → HTTP route mapping (one entry per Tauri command)
// ---------------------------------------------------------------------------

type Args = Record<string, unknown>;
type Route = (a: Args) => Promise<unknown>;

const q = (v: unknown) => encodeURIComponent(String(v));

const ROUTES: Record<string, Route> = {
  status: () => http("GET", "/v1/status"),
  capabilities: (a) =>
    http("GET", `/v1/capabilities?includeUnavailable=${a.includeUnavailable ? "true" : "false"}`),
  observations_query: (a) => http("POST", "/v1/observations/query", a.query),
  actions_list: (a) => http("GET", `/v1/actions?limit=${q(a.limit ?? 50)}`),
  action_get: (a) => http("GET", `/v1/actions/${q(a.actionId)}`),
  policy_get: () => http("GET", "/v1/policy"),
  policy_patch: (a) => http("PATCH", "/v1/policy", a.patch),
  session_get: () => http("GET", "/v1/session"),
  session_start: (a) => http("POST", "/v1/session/start", { label: a.label, consents: a.consents }),
  session_stop: () => http("POST", "/v1/session/stop"),
  consent_grant: (a) =>
    http("POST", "/v1/session/consent", {
      scope: a.scope,
      expiresMinutes: a.expiresMinutes,
      // 「只這一次」＝maxUses:1；漏掉這個欄位會把單次授權悄悄退化成 TTL。
      maxUses: a.maxUses,
    }),
  consent_revoke: (a) => http("POST", "/v1/session/revoke", { scope: a.scope }),
  recipes_list: () => http("GET", "/v1/recipes"),
  recipe_upsert: (a) => http("POST", "/v1/recipes", { text: a.text }),
  recipe_validate: (a) => http("POST", "/v1/recipes/validate", { text: a.text }),
  recipe_set_enabled: (a) => http("PATCH", `/v1/recipes/${q(a.id)}`, { enabled: a.enabled }),
  recipe_delete: (a) => http("DELETE", `/v1/recipes/${q(a.id)}`),
  recipe_simulate: (a) => http("POST", `/v1/recipes/${q(a.id)}/simulate`),
  recipe_run: (a) => http("POST", `/v1/recipes/${q(a.id)}/run`),
  tools_list: () => http("GET", "/v1/tools"),
  tools_export: async (a) => {
    const r = (await http("GET", `/v1/tools/export/${q(a.format)}`)) as Record<string, unknown>;
    return r?.export ?? r;
  },
  outbox_recent: (a) => http("GET", `/v1/outbox?limit=${q(a.limit ?? 30)}`),
  audit_tail: (a) => http("GET", `/v1/audit?limit=${q(a.limit ?? 50)}`),
  events_recent: async (a) => eventsRecentHttp(Number(a.limit ?? 100)),
  set_receptor_enabled: (a) => http("PATCH", `/v1/receptors/${q(a.id)}`, { enabled: a.enabled }),
  set_actuator_enabled: (a) => http("PATCH", `/v1/actuators/${q(a.id)}`, { enabled: a.enabled }),
  test_receptor: async (a) => {
    const r = (await http("POST", `/v1/receptors/${q(a.id)}/test`)) as Record<string, unknown>;
    return r?.observation ?? r;
  },
  test_actuator: (a) => http("POST", `/v1/actuators/${q(a.id)}/test`),
  push_observation: (a) =>
    http("POST", `/v1/receptors/${q(a.receptorId)}/push`, {
      facts: a.facts,
      confidence: a.confidence,
    }),
  providers_list: () => http("GET", "/v1/providers"),
  provider_test: (a) => http("POST", `/v1/providers/${q(a.id)}/test`),
  hardware_scan: () => http("POST", "/v1/hardware/scan"),
  activity_inbox: (a) => {
    const filter = (a.filter ?? {}) as Record<string, unknown>;
    const params = new URLSearchParams();
    Object.entries(filter).forEach(([key, value]) => {
      if (value !== undefined && value !== null && String(value).trim()) params.set(key, String(value));
    });
    return http("GET", `/v1/activity/inbox${params.size ? `?${params}` : ""}`);
  },
  agents_discoveries: () => http("GET", "/v1/agents"),
  agents_refresh: () => http("POST", "/v1/agents/refresh"),
  agents_routing: (a) => http("GET", `/v1/agents/routing${a.kind ? `?kind=${q(a.kind)}` : ""}`),
  agent_session_create: (a) => http("POST", "/v1/agent-sessions", a.input),
  agent_session_messages: (a) =>
    http("GET", `/v1/agent-sessions/${q(a.id)}/messages?direction=${q(a.direction)}`),
  agent_session_approve: (a) =>
    http("POST", `/v1/agent-sessions/${q(a.id)}/approve`, {
      requestId: a.requestId,
      approve: a.approve,
    }),
  agent_session_interrupt: (a) => http("POST", `/v1/agent-sessions/${q(a.id)}/interrupt`),
  memory_list: (a) =>
    http("GET", `/v1/memory?limit=${q(a.limit ?? 200)}${a.layer ? `&layer=${q(a.layer)}` : ""}`),
  memory_create: (a) => http("POST", "/v1/memory", a.input),
  memory_patch: (a) => http("PATCH", `/v1/memory/${q(a.id)}`, a.patch),
  memory_delete: (a) => http("DELETE", `/v1/memory/${q(a.id)}`),
  memory_export: () => http("GET", "/v1/memory/export"),
  memory_clear_session: () => http("POST", "/v1/memory/clear-session-context"),
  memory_bundle: (a) =>
    http("POST", "/v1/memory/context-bundle", {
      task: a.task,
      domains: a.domains,
      agentId: a.agentId,
    }),
  knowledge_list: (a) =>
    http(
      "GET",
      `/v1/knowledge/nodes?limit=${q(a.limit ?? 100)}${a.status ? `&status=${q(a.status)}` : ""}`
    ),
  domain_packs: () => http("GET", "/v1/knowledge/domain-packs"),
  domain_pack_install: (a) => http("POST", `/v1/knowledge/domain-packs/${q(a.id)}/install`),
  domain_pack_uninstall: (a) => http("DELETE", `/v1/knowledge/domain-packs/${q(a.id)}`),
  knowledge_search: (a) => http("GET", `/v1/knowledge/search?q=${q(a.q)}&k=${q(a.k ?? 10)}`),
  knowledge_get: (a) => http("GET", `/v1/knowledge/nodes/${q(a.id)}`),
  knowledge_review: (a) =>
    http("POST", `/v1/knowledge/nodes/${q(a.id)}/review`, { verdict: a.verdict, note: a.note }),
  knowledge_graph: (a) => http("GET", `/v1/knowledge/nodes/${q(a.id)}/graph`),
  knowledge_receipts: () => http("GET", "/v1/knowledge/receipts"),
  knowledge_update_check: (a) =>
    http("POST", "/v1/knowledge/update-check", { trigger: a.trigger }),
  knowledge_user_correction: (a) =>
    http("POST", "/v1/knowledge/user-corrections", a.input),
  assets_list: () => http("GET", "/v1/assets"),
  asset_import: (a) =>
    http("POST", "/v1/assets/import", {
      path: a.path,
      content: a.content,
      description: a.description,
    }),
  asset_derivatives: (a) => http("GET", `/v1/assets/${q(a.hash)}/derivatives`),
  asset_derive: (a) => http("POST", `/v1/assets/${q(a.hash)}/derive`),
  asset_preview: (a) => http("GET", `/v1/assets/${q(a.hash)}/preview`),
  asset_impact: (a) => http("GET", `/v1/assets/${q(a.hash)}/impact`),
  asset_delete: (a) => http("DELETE", `/v1/assets/${q(a.hash)}`),
  proactive_dialogue_get: () => http("GET", "/v1/proactive-dialogue"),
  proactive_dialogue_patch: (a) => http("PATCH", "/v1/proactive-dialogue", a.patch),
  proactive_dialogue_quiet: (a) =>
    http("POST", "/v1/proactive-dialogue/quiet", { minutes: a.minutes }),
  presentation_status: () => http("GET", "/v1/presentation"),
  presentation_hello: (a) =>
    http("POST", "/v1/presentation/hello", {
      visible: a.visible,
      packId: a.packId,
      behaviorState: a.behaviorState,
    }),
  presentation_ack: (a) =>
    http("POST", "/v1/presentation/ack", {
      actionId: a.actionId,
      outcome: a.outcome,
      detail: a.detail,
    }),
  // Character Presentation Protocol（human token；桌面視窗是可信 host）。
  character_hello: (a) =>
    http("POST", "/v1/character/hello", {
      instanceId: a.instanceId,
      role: a.role,
      manifest: a.manifest,
      negotiate: a.negotiate,
      visible: a.visible,
      packId: a.packId,
      behaviorState: a.behaviorState,
    }),
  character_receipt: (a) =>
    http("POST", "/v1/character/receipts", { instanceId: a.instanceId, receipt: a.receipt }),
  character_event: (a) =>
    http("POST", "/v1/character/events", { instanceId: a.instanceId, event: a.event }),
  character_instances: () => http("GET", "/v1/character/instances"),
  character_manifest: () => http("GET", "/v1/character/manifest"),
  character_adapters: () => http("GET", "/v1/character/adapters"),
  character_adapter_revoke: (a) => http("DELETE", `/v1/character/adapters/${q(a.adapterId)}`),
  create_plan: (a) => http("POST", "/v1/plans", a.input),
  simulate_plan: (a) => http("POST", `/v1/plans/${q(a.planId)}/simulate`),
  execute_plan: (a) => http("POST", `/v1/plans/${q(a.planId)}/execute`),
  cancel_action: (a) => http("POST", `/v1/actions/${q(a.actionId)}/cancel`),
  verify_action: (a) => http("POST", `/v1/actions/${q(a.actionId)}/verify`),
  emergency_stop: (a) => http("POST", "/v1/emergency-stop", { reason: a.reason ?? null }),
  emergency_stop_clear: () => http("POST", "/v1/emergency-stop/clear"),
  catalog_get: () => http("GET", "/v1/catalog"),
  capabilities_human: (a) =>
    http(
      "GET",
      `/v1/capabilities/human?locale=${q(a.locale ?? "")}&includeUnavailable=${
        a.includeUnavailable ? "true" : "false"
      }`
    ),
  ui_prefs_get: () => http("GET", "/v1/ui/preferences"),
  ui_prefs_patch: (a) => http("PATCH", "/v1/ui/preferences", a.patch),
  onboarding_get: () => http("GET", "/v1/onboarding"),
  onboarding_draft: (a) => http("PUT", "/v1/onboarding/draft", a.draft),
  onboarding_preview: (a) => http("POST", "/v1/onboarding/preview", a.commit),
  onboarding_commit: (a) => http("POST", "/v1/onboarding/commit", a.commit),
  pause_get: () => http("GET", "/v1/pause"),
  pause_set: (a) =>
    http("POST", "/v1/pause", { durationMinutes: a.durationMinutes, reason: a.reason }),
  pause_clear: () => http("POST", "/v1/pause/clear"),
  ai_assists_list: () => http("GET", "/v1/ai-assists"),
  ai_assist_resolve: (a) =>
    http("POST", `/v1/ai-assists/${q(a.requestId)}/resolve`, {
      decision: a.decision,
      note: a.note,
    }),
  plan_get: (a) => http("GET", `/v1/plans/${q(a.planId)}`),
  recipe_summary: (a) =>
    http("GET", `/v1/recipes/${q(a.id)}/summary?locale=${q(a.locale ?? "")}`),
  recipe_simulate_scenario: (a) =>
    http("POST", `/v1/recipes/${q(a.id)}/simulate-scenario`, a.scenario),
  recipe_convert: (a) => http("POST", "/v1/recipes/convert", { text: a.text, to: a.to }),
  recipe_get: (a) => http("GET", `/v1/recipes/${q(a.id)}`),
  agent_sessions_list: () => http("GET", "/v1/agent-sessions"),
  agent_session_send: (a) =>
    http("POST", `/v1/agent-sessions/${q(a.id)}/messages`, { kind: a.kind, body: a.body }),
  agent_session_close: (a) =>
    http("POST", `/v1/agent-sessions/${q(a.id)}/close`, { reason: a.reason ?? "closed" }),
  agent_session_verify: (a) =>
    http("POST", `/v1/agent-sessions/${q(a.id)}/verify`, { note: a.note ?? null }),
  mobile_status: () => http("GET", "/v1/mobile/status"),
  mobile_pairing_begin: () => http("POST", "/v1/mobile/pairing-session", {}),
  mobile_revoke: (a) => http("DELETE", `/v1/mobile/devices/${q(a.id)}`),
  mobile_sensors_stop: (a) => http("POST", `/v1/mobile/devices/${q(a.id)}/sensors/stop`, {}),
  mobile_test: (a) => http("POST", `/v1/mobile/devices/${q(a.id)}/test`, {}),
  mobile_ble_scan: (a) =>
    http("POST", "/v1/mobile/ble/scan", {
      durationMs: a.durationMs ?? 4000,
      deviceId: a.deviceId ?? null,
    }),
  sensor_mic_listen: (a) =>
    http("POST", "/v1/sensors/microphone/listen", { durationMs: a.durationMs }),
  sensors_stop: () => http("POST", "/v1/sensors/stop"),
};

/** Invoke a backend command through whichever transport is active. */
export async function call<T>(cmd: string, args?: Args): Promise<T> {
  if (!httpMode) {
    return invoke<T>(cmd, args);
  }
  const route = ROUTES[cmd];
  if (!route) throw new Error(`no HTTP route for command ${cmd}`);
  return (await route(args ?? {})) as T;
}

// ---------------------------------------------------------------------------
// Event stream (HTTP mode): fetch-based SSE with Last-Event-ID replay.
// EventSource cannot send Authorization headers, so we parse the stream.
// ---------------------------------------------------------------------------

interface StreamState {
  buffer: unknown[];
  handlers: Set<(e: unknown) => void>;
  readyHandlers: Set<() => void>;
  errorHandlers: Set<(msg: string) => void>;
  started: boolean;
  opened: boolean;
  abort?: AbortController;
}

const stream: StreamState = {
  buffer: [],
  handlers: new Set(),
  readyHandlers: new Set(),
  errorHandlers: new Set(),
  started: false,
  opened: false,
};

const EVENT_BUFFER_MAX = 500;

/** Where this window is in the daemon's event sequence, and WHICH daemon that
 *  sequence belongs to. Sequence numbers restart at 1 in a fresh process, so a
 *  cursor without an instance identity is meaningless across a restart. */
export interface StreamCursor {
  /** Daemon instance identity (`/v1/status.startedAt`); null = never synced. */
  instance: string | null;
  lastId: string;
}

export const INITIAL_STREAM_CURSOR: StreamCursor = { instance: null, lastId: "0" };

/**
 * Decide the `Last-Event-ID` for a (re)connection from the daemon's status.
 *
 * - First connect: start at the daemon's CURRENT sequence, i.e. new events
 *   only. Replaying the whole ring buffer would push a previous run's
 *   `action.observed` / `emergency.stop` into a freshly started UI, so the
 *   companion re-performs results that finished long ago.
 * - Reconnect to the same instance: resume from `lastId` (no gap).
 * - Different instance (daemon restarted): the old `lastId` would swallow the
 *   new daemon's first events as if already seen — or, worse, its low
 *   sequence numbers replay as "new". Reset, and drop cross-instance buffer.
 * - Unreadable status: keep the cursor untouched and let the caller retry
 *   rather than guessing.
 */
export function nextStreamCursor(
  prev: StreamCursor,
  status: unknown
): { cursor: StreamCursor; reset: boolean } {
  const record = status && typeof status === "object" ? (status as Record<string, unknown>) : null;
  const startedAt = record?.["startedAt"];
  const sequence = Number(record?.["eventSequence"]);
  if (typeof startedAt !== "string" || !startedAt || !Number.isFinite(sequence)) {
    return { cursor: prev, reset: false };
  }
  if (prev.instance === startedAt) {
    return { cursor: { instance: startedAt, lastId: prev.lastId }, reset: false };
  }
  return {
    cursor: { instance: startedAt, lastId: String(Math.max(0, Math.trunc(sequence))) },
    reset: prev.instance !== null,
  };
}

async function runStream() {
  const { base, token } = cfg();
  let cursor: StreamCursor = { ...INITIAL_STREAM_CURSOR };
  let everOpened = false;
  let failures = 0;
  // Reconnect loop with modest backoff; caller UIs surface offline state.
  for (;;) {
    const abort = new AbortController();
    stream.abort = abort;
    try {
      // Identify the daemon instance and its current sequence BEFORE
      // subscribing. Without this the first connection asks for
      // `Last-Event-ID: 0` (the entire replay buffer) and a reconnect after a
      // restart keeps a cursor that belongs to a different process.
      const statusRes = await fetch(`${base}/v1/status`, {
        headers: { ...(token ? { Authorization: `Bearer ${token}` } : {}) },
        signal: abort.signal,
      });
      if (!statusRes.ok) throw new Error(`events stream status ${statusRes.status}`);
      const advanced = nextStreamCursor(cursor, await statusRes.json());
      // A restart invalidates the buffered events too: they came from a
      // process that no longer exists.
      if (advanced.reset) stream.buffer.length = 0;
      cursor = advanced.cursor;

      const res = await fetch(`${base}/v1/events`, {
        headers: {
          ...(token ? { Authorization: `Bearer ${token}` } : {}),
          "Last-Event-ID": cursor.lastId,
          Accept: "text/event-stream",
        },
        signal: abort.signal,
      });
      if (!res.ok || !res.body) throw new Error(`events stream ${res.status}`);
      stream.opened = true;
      everOpened = true;
      failures = 0;
      stream.readyHandlers.forEach((h) => h());
      const reader = res.body.getReader();
      const decoder = new TextDecoder();
      let pending = "";
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        pending += decoder.decode(value, { stream: true });
        let sep: number;
        while ((sep = pending.indexOf("\n\n")) >= 0) {
          const chunk = pending.slice(0, sep);
          pending = pending.slice(sep + 2);
          let data = "";
          for (const line of chunk.split("\n")) {
            if (line.startsWith("data:")) data += line.slice(5).trim();
            else if (line.startsWith("id:")) cursor.lastId = line.slice(3).trim();
          }
          if (!data || data === "keep-alive") continue;
          try {
            const event = JSON.parse(data);
            stream.buffer.push(event);
            if (stream.buffer.length > EVENT_BUFFER_MAX) stream.buffer.shift();
            stream.handlers.forEach((h) => h(event));
          } catch {
            /* non-JSON keep-alive lines are ignored */
          }
        }
      }
    } catch (e) {
      if (abort.signal.aborted) return;
      stream.opened = false;
      failures += 1;
      // Only surface an error when the daemon has never been reachable.
      // Transient drops after a successful connection retry silently; the
      // UI keeps its last honest state and recovers on reconnect.
      if (!everOpened && failures >= 2) {
        stream.errorHandlers.forEach((h) => h(String(e)));
      }
    }
    await new Promise((r) => setTimeout(r, 1000));
  }
}

function ensureStream() {
  if (!stream.started) {
    stream.started = true;
    void runStream();
  }
}

async function eventsRecentHttp(limit: number): Promise<unknown[]> {
  ensureStream();
  // Give the replay a moment to land on first call.
  if (stream.buffer.length === 0) {
    await new Promise((r) => setTimeout(r, 400));
  }
  return stream.buffer.slice(-limit);
}

// ---------------------------------------------------------------------------
// Unified event subscription API (mirrors the Tauri listen() helpers)
// ---------------------------------------------------------------------------

export function onEvent<T>(handler: (event: T) => void): Promise<UnlistenFn> {
  if (!httpMode) {
    return listen<T>("runtime-event", (e) => handler(e.payload));
  }
  ensureStream();
  const h = handler as (e: unknown) => void;
  stream.handlers.add(h);
  return Promise.resolve(() => stream.handlers.delete(h));
}

export function onReady(handler: () => void): Promise<UnlistenFn> {
  if (!httpMode) {
    return listen("runtime-ready", () => handler());
  }
  ensureStream();
  if (stream.opened) handler();
  stream.readyHandlers.add(handler);
  return Promise.resolve(() => stream.readyHandlers.delete(handler));
}

export function onError(handler: (message: string) => void): Promise<UnlistenFn> {
  if (!httpMode) {
    return listen<string>("runtime-error", (e) => handler(e.payload));
  }
  ensureStream();
  stream.errorHandlers.add(handler);
  return Promise.resolve(() => stream.errorHandlers.delete(handler));
}
