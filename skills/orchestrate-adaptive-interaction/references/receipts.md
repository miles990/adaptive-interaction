# Action receipts & the state machine

States (forward path):
`planned → authorized → accepted → dispatched → acknowledged → observed → completed`
Terminal alternates: `blocked`, `failed`, `uncertain`, `cancelled`, `expired`, `stopped`.

Meanings you must not conflate:
- `accepted` — the RUNTIME queued it. Nothing has happened yet.
- `dispatched` — the driver sent the command. Still not done.
- `acknowledged` — the target confirmed receipt. Effect still unverified.
- `observed` — an observation corroborated the effect.
- `completed` — done to the configured verification standard. Check
  `verification.verdict`: `observed` (environment confirmed) vs
  `acknowledged-only` (driver-level only) vs `uncertain`/`refuted`.

Receipt fields: `requestedParameters` vs `effectiveBoundedParameters` (what
policy clamped), `policyDecisions` (why), `timestamps` (full history),
`errors`, `expiresAt` (watchdog TTL), `correlationId`.

Rules:
- Report queued/accepted as "queued", never as "done".
- `uncertain` means unknown — say so; re-`verify` before retrying.
- After a runtime restart, previously open actions become `uncertain` and are
  never re-dispatched automatically.
