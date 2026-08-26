# Safety model (deterministic, Rust-enforced)

Everything you execute passes the policy governor. Prompts cannot override it.

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
  never fire recipes to bypass a pause.
- AI decision gate: recipes with `ai.mode: when-uncertain` publish
  `ai.assist.requested` when evidence is ambiguous (low confidence or
  contradictory observations). You may answer via
  `assists resolve <id> proceed|no-action` before the deadline. If nobody
  answers, the deterministic `onUnavailable` behavior (fallback / no-action)
  applies — do not retro-resolve or double-fire. Deterministic events never
  generate assist requests; do not inject yourself into unambiguous flows.
- AI-assisted descriptions are presentation only, hash-bound, and can never
  alter risk/consent/data-flow facts. Never write a description that claims a
  capability is safer than its formal manifest says.
