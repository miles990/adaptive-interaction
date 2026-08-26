# Interaction recipes

Declarative YAML/JSON, validated by one shared validator
(schema: `schemas/recipe.schema.json`). Key blocks:

- `trigger`: `mode: single|all|any|quorum|weighted|sequence`, optional
  `within: 10m`, `steps[{receptor, condition, weight}]`. Conditions are either
  maps (`event: task.completed`) or expressions (`"count > 3"`); inference
  keys use the `inferred.` prefix and respect `context.minConfidence`.
- `context`: extra receptors, `maxAge`, `minConfidence`.
- `decision`: `objective`, `allowNoAction` (respect it!).
- `message`: `mode: fixed|random|adaptive|ai-generated|none`, `templates`,
  `allowSilence`, `language`, `tone`.
- `actuation`: `mode: single|parallel|sequence|fallback|adaptive|redundant`,
  `candidates` (preference order), `minChannels`/`maxChannels`,
  `chance` (0..1), `jitter`.
- `verification`: `strategy: best-effort|observed|none`, `timeout`.
- `limits`: `cooldown`, `expiresAfter`, `maxExecutionsPerSession`, `maxPerHour`.
- `consent.required`: e.g. `["channel:haptic"]`.

Lifecycle: `recipes validate <file>` → `recipes apply <file>` →
`recipes simulate <id>` (shows WHY it would/wouldn't fire and the policy
decisions) → enabled recipes fire autonomously on matching observations.
`recipes run <id>` bypasses the trigger but NEVER the policy.
