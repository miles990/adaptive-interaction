# Safety model (deterministic, Rust-enforced)

Everything you execute passes the policy governor. Prompts cannot override it.

- AI hosts use `state/api-agent-token` / CLI `--agent-scope`. Rust HTTP
  middleware refuses consent grants, session creation/renewal, policy or UI
  mutation, human review, Presentation acks, and Emergency Stop clear. Never
  access the human `state/api-token` or retry a 403 through another endpoint.

- Effective output = min(your suggestion, user preference, session limit,
  device safe limit, remaining budget). Expect clamping; read
  `policyDecisions` to see what happened.
- Allowlists: actuators, channels, tools. Off-list = blocked.
- Consent: consent-gated actuators need an active session consent
  (`session consent channel:haptic`). Revocation cancels in-flight actions.
- Approval: `riskClass >= high` returns `approval_required`. A human grants a
  scoped consent to approve. Never try to defeat this.
- Quiet hours silence intrusive channels (audio, haptic, notification, light).
  Degrade to conversation/web-ui or choose no action.
- Budgets: per-channel duration budgets and monetary budgets per session;
  frequency caps and cooldowns per actuator.
- Patterns are TTL-leased: `repeat: forever` becomes a bounded lease with a
  watchdog. Nothing runs unsupervised forever.
- Emergency stop (`interact-ai emergency-stop`, `POST /v1/emergency-stop`,
  desktop button) cancels everything, halts drivers, revokes consents, and
  does NOT auto-resume. Clearing requires an explicit human action.
- Crash recovery: the runtime never resumes high-risk output after a restart;
  open actions become `uncertain`.
- Proactive pause is a USER control, distinct from emergency stop: it silences
  recipe-driven autonomy while explicit requests keep working. Respect it;
  never fire recipes to bypass a pause, and NEVER call `interact-ai resume` /
  `POST /v1/pause/clear` (or `POST /v1/onboarding/commit`) on your own
  initiative — only when the human explicitly asked you to.
- AI decision gate: recipes with `ai.mode: when-uncertain` publish
  `ai.assist.requested` when evidence is ambiguous (low confidence or
  contradictory observations). You may answer via
  `assists resolve <id> proceed|no-action` before the deadline. If nobody
  answers, the deterministic `onUnavailable` behavior (fallback / no-action)
  applies — do not retro-resolve or double-fire. Deterministic events never
  generate assist requests; do not inject yourself into unambiguous flows.
  An assist marked `requireHumanConfirmation` cannot be resolved `proceed`
  over the API at all (only the human's desktop surface can); expect
  `approval_required` and surface it to the human instead of retrying.
- AI-assisted descriptions are presentation only, hash-bound, and can never
  alter risk/consent/data-flow facts. Never write a description that claims a
  capability is safer than its formal manifest says.

## Providers, agent sessions, sensors (v0.3)

- **Providers**: discovered ≠ paired ≠ installed ≠ enabled ≠ authorized. Never
  assume a device exists or is usable; check `providers list` state. Pairing
  uses a fingerprint, not an IP. Revoked is sticky.
- **Declarative adapters** only reach the network at human-owned URLs in
  `config/adapters/*.yaml`; secrets are `secret://` references. A device's HTTP
  2xx is `acknowledged`, not `completed` — completion needs an observation.
- **Agent sessions** are leased and budgeted. An agent's `claimed-completed` is
  an inference, NOT a receipt or verification — never report it as done.
  Delegation is depth/cycle/count/budget-limited; emergency stop cancels every
  session. Never delegate to bypass a `blocked` decision.
- **Sensors** are OFF by default and consent-gated. Never assume the microphone
  is available; `sensors listen` needs enable + explicit session consent and is
  hard-capped at 30s. Capture is always visible; the camera is unavailable.
  Never claim to have "heard" or "seen" anything beyond the derived facts the
  receptor actually produced.
- **Companion**: safety wording (emergency / blocked / unknown / sensor-in-use)
  is fixed and immutable — persona/world/story packs restyle only non-safety
  lines. The desktop character holds no authority; everything still goes through
  the governor.

## v0.4 additions

- Presentation: a companion render ack is DRIVER-level evidence (AcknowledgedOnly); no ack
  within 10s ⇒ Uncertain. Truth states (success/blocked/emergency) are never AI-playable.
- Agent gateway: sessions run read-only/plan; approvals default to DENY and only a human can
  approve; emergency stop kills the whole subprocess tree; subprocesses never survive restart;
  agent claims stay inferences (confidence 0.5).
- Knowledge: AI writes are always candidates; agents cannot approve; analogy/conjecture can
  never be causal; sources are content-addressed and immutable; superseded knowledge leaves
  general answering.
- Memory: no un-deletable memory; secrets are refused; expired memory is pruned; stale memory
  needs re-confirmation and never enters context bundles.

## v0.5 additions: Character Presentation Protocol
- The presentation layer has NO authority: adapters (built-in 小樞 rig, sprite packs, text, external WebSocket programs) receive semantic intents with a Runtime-decided `truthState` and can only send bounded receipts/input events. They cannot grant consent, clear emergency stop, change policy, or produce `verified`; a green check is drawn only when `truthState == "verified"` (human verification path).
- Honest degradation: `exact` / `substituted` / `reduced` / `unsupported` / `failed`; safety intents (emergency, offline, blocked, failed, request-consent, unknown, verified-success, claim-completed, wait, ask, cancelled) always resolve at least to `system.text`; adapter crash or disconnect marks pending presentations `uncertain`, never `completed`.
- AI-requested presentation is capped at priority 50 and non-safety intents (wait/ask requests are substituted with think/notice); emergency (100) preempts everything, including non-interruptible play.
- Emergency stop and sensor-in-use indicators are guaranteed by the trusted host (tray + overlay window), independent of any character renderer.
