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


## Human names vs technical ids

Humans see display names resolved by the runtime (user override → adapter →
central catalog → tokenized fallback), e.g. 桌面通知 = `local-notification`,
對話訊息 = `conversation`, 任務狀態 = `task.lifecycle`. YOU must keep using
stable technical ids in every command and API call; get the mapping from
`capabilities --human` when talking to the human about capabilities.

You may improve human descriptions with `interact-ai describe` (bind to the
current `manifestHash`). Descriptions are presentation only: they never change
risk, consent, data-flow or limits, and a manifest change invalidates them.

Also honor the proactive pause: when `status.proactivePause.paused` is true,
recipe-driven autonomy is off. Do not try to work around it by firing recipes
manually; explicit user requests remain fine.
