# Capability snapshots

`GET /v1/capabilities` / `interact-ai capabilities --json` returns:

- `receptors[]` — sources of observations. Fields to respect:
  `sensitivity` + `requiresConsent` (camera/mic-class receptors start
  disabled), `mode` (poll/event/stream), `availability`, `health`.
- `actuators[]` — output channels. Respect `riskClass`, `externalSideEffect`,
  `requiresConsent`, `limits` (device-safe ceilings), `availability`.
- `toolOperations[]` — the callable tool surface with input/output schemas,
  risk and approval metadata.
- `constraints[]` — live restrictions (quiet hours active, emergency stop
  engaged, no session). Read them BEFORE planning.
- `sessionPolicy` — allowlists, channel limits, initiative level.
- `version` / `generatedAt` — snapshots older than ~60s should be refreshed.

Never assume a device exists: plan only against actuators present and
`availability == "available"`. If a preferred channel is missing, fall back or
choose no action.
