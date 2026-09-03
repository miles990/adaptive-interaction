# HTTP API reference

Base: `http://127.0.0.1:8787` (loopback only by default).
Auth: `Authorization: Bearer <token>` — token lives in
`<home>/state/api-agent-token` (0600) for AI hosts. The separate
`state/api-token` is reserved for the human/control-center plane: never read,
request, log, or forward it. `/health` & `/ready` are public.
Full machine-readable spec: `GET /v1/openapi.json`.

The restricted token can discover allowed runtime state, use canonical tools, plan/simulate/
execute through policy, and stop/cancel/revoke. It receives HTTP 403
`token_scope_forbidden` for human-only operations including consent grants,
Emergency Stop clear, policy/UI/provider mutation, Agent Session creation,
renewal, or interruption, Presentation acknowledgement, memory mutation/export,
source deletion, and knowledge approval. Direct memory, asset, knowledge, audit, outbox, and
Agent Session reads are also human-only; use the canonical tool surface where
available. Do not retry a refused operation through a direct URL.

## Core loop
```
GET  /v1/status
GET  /v1/capabilities?includeUnavailable=false
POST /v1/observations/query        {"receptorId": "...", "limit": 10, "maxAgeMs": 60000}
POST /v1/receptors/{id}/push       {"facts": {...}, "inferences": {...}, "confidence": 0.9}
POST /v1/receptors/{id}/read       (fresh read)
POST /v1/plans                     {"intent": "...", "candidates": [...], ...}
POST /v1/plans/{id}/simulate
POST /v1/plans/{id}/execute        (body {"dryRun": true} = simulate)
GET  /v1/actions ; GET /v1/actions/{id}
POST /v1/actions/{id}/verify ; POST /v1/actions/{id}/cancel
POST /v1/emergency-stop              # agent token allowed (safety direction)
POST /v1/emergency-stop/clear        # HUMAN TOKEN ONLY
GET  /v1/events                    (SSE; resume with Last-Event-ID header)
```

## Sessions, policy, recipes, tools
```
POST /v1/session/start   {"label": "...", "consents": ["channel:haptic"]} # HUMAN ONLY
GET  /v1/session ; POST /v1/session/consent {"scope": "...", "maxUses": 1|null} ;
POST /v1/session/revoke {"scope": "..."} ; POST /v1/session/stop
GET/PATCH /v1/policy               (PATCH = HUMAN ONLY JSON merge patch)
GET/POST /v1/recipes ; GET/PATCH/DELETE /v1/recipes/{id}
POST /v1/recipes/validate {"text": "<yaml>"} ; /v1/recipes/{id}/simulate ; /run
GET  /v1/tools ; GET /v1/tools/{name} ; POST /v1/tools/{name}/call
GET  /v1/tools/export/{format}
GET  /v1/outbox ; GET /v1/audit
```

## Human layer
```
GET  /v1/capabilities/human?locale=zh-TW&includeUnavailable=true
GET  /v1/catalog
GET/PATCH /v1/ui/preferences        (mode simple|advanced, locale, customNames)
GET  /v1/onboarding ; PUT /v1/onboarding/draft ; POST /v1/onboarding/commit
GET/POST /v1/pause {"durationMinutes": 60} ; POST /v1/pause/clear
PUT  /v1/capabilities/{kind}/{id}/ai-description
     {"locale": "...", "text": "...", "manifestHash": "..."}   (409 when stale)
GET  /v1/ai-assists ; POST /v1/ai-assists/{id}/resolve {"decision": "proceed"|"no-action"}
GET  /v1/recipes/{id}/summary?locale=zh-TW
POST /v1/recipes/{id}/simulate-scenario   {"quietHours":true, "event":{"receptor":"...","facts":{...}}}
POST /v1/recipes/convert {"text": "...", "to": "yaml"|"json"}  (lossless, keeps unknown fields)
```
Events added: `proactive.paused`, `proactive.resumed`, `ai.assist.requested`,
`ai.assist.resolved`. Subscribe to `/v1/events`; when you see
`ai.assist.requested` you may resolve it before its deadline — after the
deadline the recipe's deterministic `onUnavailable` behavior already applied.

Errors: `{"error": {"code": "...", "message": "..."}}` — codes include
`policy_blocked`, `approval_required`, `consent_required`, `session_inactive`,
`emergency_stop` (HTTP 423), `not_found`, `validation_failed`.

## Providers, agent sessions, sensors (v0.3)

The Agent Session management routes below are human/control-plane routes; the
restricted token cannot enumerate, create, or interrupt sessions.

`POST /v1/agent-sessions/{id}/interrupt` names one session, so the caller must
be able to prove ownership. Since v0.5.1 it accepts only:

* a **session-scoped capability token** — the token minted for that session
  (put it in `INTERACT_AI_SESSION_TOKEN`; `interact-ai --agent-scope` prefers it
  over `state/api-agent-token`). It can interrupt **only its own** session; any
  other id is `403 token_scope_forbidden`.
* the **human/control-center token** — retains management ability over any
  session.

The shared restricted token in `state/api-agent-token` carries no session
identity (it cannot create or list sessions either), so it now receives
`403 token_scope_forbidden` on `/interrupt` as well. This is a deliberate
security narrowing in v0.5.1: up to v0.5.0 that token could interrupt any
session id without any ownership check. Migration: run with the session's own
`INTERACT_AI_SESSION_TOKEN`, or hand the interrupt to the human plane.

```http
GET    /v1/providers
GET    /v1/providers/:id
POST   /v1/providers/:id/pair          {"pairingCode": "..."}
POST   /v1/providers/:id/transition    {"state": "installed|disabled|available|revoked"}
POST   /v1/providers/:id/revoke

GET    /v1/agent-sessions
POST   /v1/agent-sessions              {"agentId","ttlMinutes","dataScope","toolScope","maxMessages","delegation"}
GET    /v1/agent-sessions/:id
POST   /v1/agent-sessions/:id/report   {"event","payload"}
GET    /v1/agent-sessions/:id/messages ?direction=to-session|from-session
POST   /v1/agent-sessions/:id/messages {"kind","body"}
POST   /v1/agent-sessions/:id/renew    {"extraMinutes"}
POST   /v1/agent-sessions/:id/close    {"handoff","reason"}

POST   /v1/sensors/microphone/listen   {"durationMs"}   # needs enable + explicit session consent
POST   /v1/sensors/stop
```

`POST /v1/sensors/stop` (v0.5 Phase 9; available to human, agent, and agent-session tokens) returns
`{"stopped": bool, "uncertain": bool, "local": {"microphone": "stopped"|"idle"}, "devices": [{"deviceId",
"name", "outcome": "stopped"|"unknown"|"unreachable", "waitedMs", "via": "ack"|"status"}]}` — `stopped` is
only `true` when every local and remote source confirmed; `uncertain` means at least one source (typically
a connected iPhone) did not confirm within the ~2 s wait. Never read the old `{"stopped": true}` shape as a
guarantee of anything before Phase 9; check `stopped`/`uncertain`/`devices[].outcome`, not just presence of
a 200 response.

Events added: `provider.registered`, `provider.state-changed`,
`sensor.started`, `sensor.stopped`.

Honesty over HTTP: an agent's `report` becomes an observation whose payload is
an INFERENCE — never a receipt. A delegated task in the mailbox is `dispatched`
until the session actually fetches it (`acknowledged`). The HTTP surface is the
AI-host surface: it can never satisfy a recipe's `requireHumanConfirmation`
gate (only the desktop IPC can).

## v0.4 endpoints

- `GET/POST /v1/presentation{,/hello,/ack}` — companion surface presence + honest render acks.
- `POST /v1/character/{hello,receipts,events,intent}`, `GET /v1/character/{instances,manifest,adapters}`, `POST/DELETE /v1/character/adapters[/{id}]`, `GET /v1/character/ws?token=` — Character Presentation Protocol 1.0 (human token; the WS route accepts ONLY adapter tokens, which can never reach human routes). AI never constructs intent envelopes: it can only request `companion.state.present`/`companion.animation.play` (bounded to non-safety intents, priority <= 50).
- `GET/PATCH /v1/proactive-dialogue`, `POST /v1/proactive-dialogue/quiet` — deterministic
  proactive-speech limits (hourly cap / min interval / no-follow-up; safety class only dedups).
- `GET /v1/agents`, `POST /v1/agents/refresh`, `GET /v1/agents/routing?kind=` — local agent
  discovery (codex/claude-code) + deterministic routing advice.
- `POST /v1/hardware/scan` — metadata-only 17-class coverage report. It never opens camera,
  microphone, HID, BLE, or mDNS and returns `sensorActivationAttempted:false`.
- `POST /v1/agent-sessions` now spawns REAL subprocess sessions for agentId codex/claude-code
  (read-only by default; explicit human consent may set `allowWrite` for the bounded `workdir`;
  `maxCost` supported). `POST /v1/agent-sessions/{id}/approve`
  (human approval resolution; unanswered approvals auto-deny), `/interrupt`
  (v0.5.1: session-scoped capability token for its own session, or human token;
  the shared restricted token is refused).
- `GET/POST /v1/memory`, `/v1/memory/{id}`, `/v1/memory/export`, `/v1/memory/clear-session-context`,
  `POST /v1/memory/context-bundle` — layered memory with retention tri-state; deterministic bundles.
- `GET /v1/assets`, `POST /v1/assets/import`, `GET /v1/assets/{hash}{,/impact,/content}`,
  `DELETE /v1/assets/{hash}` — content-addressed, write-once sources with delete-impact preview.
- `GET /v1/knowledge/search|nodes|nodes/{id}|nodes/{id}/graph`, `POST /v1/knowledge/nodes|edges`,
  `POST /v1/knowledge/nodes/{id}/review`, `GET /v1/knowledge/receipts`,
  `POST /v1/knowledge/update-check` — candidate-only AI writes; human-only activation;
  machine-readable knowledge receipts.
- `POST /v1/knowledge/user-corrections` — human-only; creates a 30-day-review UserMemory plus
  a Knowledge Candidate, never an immediately active universal rule.
- Tool surface adds `interaction.knowledge_*` (search/get/get_source/expand_graph/
  propose_entity/propose_claim/propose_relation/propose_supersede/submit_review).

## v0.5 Phase 9 additions (release hardening)

- `POST /v1/onboarding/preview` — HUMAN TOKEN ONLY (same `!path.starts_with("/v1/onboarding")` rule as
  `/v1/onboarding/commit`). Same request body as `/v1/onboarding/commit`; zero side effects. Returns
  `{"receptors":[{"id","from":"on"|"off","to","changed"}],"actuators":[same shape],
  "starterRecipes":[{"id","exists"}],"policyPatch","preferences","changed"}` — a dry-run diff the desktop
  wizard shows before committing. Errors mirror commit exactly (404 unknown id, `consent_required` for a
  consent-gated capability, validation error for a bad policy patch).
- `POST /v1/mobile/devices/{id}/sensors/stop` and `POST /v1/mobile/devices/{id}/test` — HUMAN TOKEN ONLY
  (agent/session tokens get 403 `token_scope_forbidden`; unknown `id` is 404). Per-device counterparts to
  `POST /v1/sensors/stop`: `sensors/stop` returns `{"deviceId","requested":bool,"connected":bool,
  "outcome":"stopped"|"unknown"|"unreachable","waitedMs","via"}`; `test` sends a WebSocket ping and returns
  `{"deviceId","ok":bool,"connected":bool,"latencyMs"?,"uncertain"?,"reason"?}` — `ok:true` only means the
  socket answered, never that the phone's app functionality works.
- `POST /v1/emergency-stop` response/event/audit payload gained `"sensors"` (the same
  `StopAllSensorsReport` shape as `POST /v1/sensors/stop`) and `"characterEmergency"`
  (`[{"deviceId","outcome":"acknowledged"|"refused"|"unknown"|"unreachable"}]` — every connected iPhone's
  presentation-emergency projection outcome). An AI token can no longer make an iPhone display
  "emergency-stop" state through `character.present` — that truth state is runtime-owned now.
- `GET /v1/providers` / `GET /v1/providers/:id` omit `identity.fingerprint` for any non-human principal
  (agent token, agent-session token, character adapter token). Only the human/control-plane caller sees a
  paired iPhone's public identity fingerprint.
- Character Presentation Protocol: `POST /v1/character/hello` accepts an optional `reducedMotion: bool`
  (defaults `false` if omitted) reflecting the caller's `prefers-reduced-motion` — negotiation and every
  subsequent receipt's `resolution` for that instance depend on it (a receipt's resolution can only degrade
  from what was negotiated, never upgrade). `GET /v1/character/instances` and `GET /v1/character/adapters`
  report `author`, `version`, `inputCapabilities`, `executable`, `network` (and adapters:
  `characterDisplayName`, `adapterKind`) for every registered instance/adapter, including ones that have
  never connected — these were already present before Phase 9, this note only corrects prior
  documentation drift.

## v0.5.1 additions (patch release; in development)

- `POST /v1/session/consent` accepts an optional `maxUses` (number or null). `maxUses: 1` is a REAL
  one-shot grant: the first authorized dispatch spends it inside the authorization critical section,
  two concurrent plans racing one grant let exactly one through, a failed dispatch does NOT refund it,
  and the spent state survives a restart. `maxUses: 0` is refused. Omitting it (or null) keeps the
  historical unlimited-within-TTL semantics. `Consent` gained `maxUses`/`remainingUses`
  (`serde(default)`), so old session blobs load unchanged. **`maxUses` is only accepted for
  `actuator:` / `channel:` scopes** — nothing spends a use on a `receptor:` or `tool:` scope, so a
  grant with `maxUses` on those scopes is refused with HTTP 400 (use `expiresMinutes`); the API never
  reports a `maxUses` it does not enforce. CLI: `interact-ai session consent <scope> --max-uses 1`.
- `GET /v1/memory/export` gained `total` (the real row count from `SELECT COUNT(*)`, so exactly 1000
  stored items no longer reports truncation) and `included` (always `["memory-items"]`); `notIncluded`
  is now the precise list `["knowledge-nodes","assets-and-derivatives","knowledge-receipts",
  "character-interaction-memory"]` (it used to be the vaguer `["knowledge","assets",
  "character-interaction-memory"]`). Export scope itself is unchanged: memory items only.
- `GET /v1/providers/{id}` carries `detail.tested.pairingUnverified` (`serde(default)`, absent when
  false, old records do not grow one): the device reported `hello.pairing=false`, i.e. the pairing code
  was never compared, so the only identity evidence is the deviceId the device claims for itself.
  `tested_note` says so instead of claiming the pairing completed. Sources that cannot know (a human
  test, a receptor read) pass none and never wash an earlier unverified mark clean. "A device that says
  it needs no pairing" is never a verified pairing.
- Agent session records carry `resolvedWorkdir` (`serde(default)`): the canonicalized absolute path the
  gateway subprocess was actually mounted in. A resume must keep the same folder — a different folder,
  a missing one, or a gateway session recorded before this field existed is `PolicyBlocked`.
  **Breaking**: gateway sessions created before v0.5.1 cannot be resumed and must be recreated.
  CLI `interact-ai agents resume` gained `--max-cost`/`--max-messages`; omitted flags reuse the previous
  session's actual limits (not the wider runtime defaults), and an omitted `--workdir` uses the
  recorded folder.
- Provider lifecycle is now enforced at execution time: when every provider owning a capability is
  stopped (Disabled/Expired/Revoked/Closed), `observe` returns `Unavailable` and `execute`/`simulate`
  return `Blocked{rule:"provider.not-operational"}`; the capability projects as
  `Availability::Disabled` (no new enum variant). Installed/Paired/Disconnected are connection facts,
  not decisions, and do not gate. Shared capabilities (several paired iPhones, several agent sessions)
  are gated only when ALL owners are stopped.
- `POST /v1/actions/{id}/cancel` is honest: only a driver confirmation within 2 s is `Cancelled`;
  errors, timeouts and unreachable actuators become `Uncertain` with a `cancel_unconfirmed` error.
  Built-in actuators (conversation, notification, …) cannot be recalled, so cancelling one always
  returns `Uncertain`. Do not read "cancel accepted" as "cancelled".
