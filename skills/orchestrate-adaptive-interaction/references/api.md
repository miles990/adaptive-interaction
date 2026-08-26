# HTTP API reference

Base: `http://127.0.0.1:8787` (loopback only by default).
Auth: `Authorization: Bearer <token>` — token lives in
`<home>/state/api-token` (0600). `/health` & `/ready` are public.
Full machine-readable spec: `GET /v1/openapi.json`.

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
POST /v1/emergency-stop ; POST /v1/emergency-stop/clear
GET  /v1/events                    (SSE; resume with Last-Event-ID header)
```

## Sessions, policy, recipes, tools
```
POST /v1/session/start   {"label": "...", "consents": ["channel:haptic"]}
GET  /v1/session ; POST /v1/session/consent {"scope": "..."} ;
POST /v1/session/revoke {"scope": "..."} ; POST /v1/session/stop
GET/PATCH /v1/policy               (PATCH = JSON merge patch)
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
