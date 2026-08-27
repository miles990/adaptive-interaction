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
Emergency Stop clear, policy/UI/provider mutation, Agent Session creation or
renewal, Presentation acknowledgement, memory mutation/export, source deletion,
and knowledge approval. Direct memory, asset, knowledge, audit, outbox, and
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
GET  /v1/session ; POST /v1/session/consent {"scope": "..."} ;
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

The Agent Session management routes below are human/control-plane routes except
for `/interrupt`; the restricted token cannot enumerate or create sessions.

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

Events added: `provider.registered`, `provider.state-changed`,
`sensor.started`, `sensor.stopped`.

Honesty over HTTP: an agent's `report` becomes an observation whose payload is
an INFERENCE — never a receipt. A delegated task in the mailbox is `dispatched`
until the session actually fetches it (`acknowledged`). The HTTP surface is the
AI-host surface: it can never satisfy a recipe's `requireHumanConfirmation`
gate (only the desktop IPC can).

## v0.4 endpoints

- `GET/POST /v1/presentation{,/hello,/ack}` — companion surface presence + honest render acks.
- `GET/PATCH /v1/proactive-dialogue`, `POST /v1/proactive-dialogue/quiet` — deterministic
  proactive-speech limits (hourly cap / min interval / no-follow-up; safety class only dedups).
- `GET /v1/agents`, `POST /v1/agents/refresh`, `GET /v1/agents/routing?kind=` — local agent
  discovery (codex/claude-code) + deterministic routing advice.
- `POST /v1/hardware/scan` — metadata-only 17-class coverage report. It never opens camera,
  microphone, HID, BLE, or mDNS and returns `sensorActivationAttempted:false`.
- `POST /v1/agent-sessions` now spawns REAL subprocess sessions for agentId codex/claude-code
  (read-only by default; explicit human consent may set `allowWrite` for the bounded `workdir`;
  `maxCost` supported). `POST /v1/agent-sessions/{id}/approve`
  (human approval resolution; unanswered approvals auto-deny), `/interrupt`.
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
