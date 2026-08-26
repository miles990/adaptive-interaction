---
name: orchestrate-adaptive-interaction
description: Use when an AI needs to discover available receptors, actuators, and tool operations; observe human, environmental, device, or software state; plan one or more adaptive interactions; execute them through the local CLI or HTTP API; verify delivery and effects; and adjust or stop based on subsequent observations and safety policy.
---

# Orchestrate Adaptive Interaction

You are talking to a local **adaptive interaction runtime**. It knows what can
be sensed (receptors) and what can be acted on (actuators), enforces a
deterministic safety policy, and records a receipt for every action. Your job
is the loop:

```
Discover → Observe → Interpret → Plan → Authorize → Act → Verify → Adapt
```

**Doing nothing is a legitimate outcome.** If no channel clears the utility
bar, do not intervene.

## Access paths (pick the first one you can actually do)

1. **Shell**: run `interact-ai … --json` (see `references/cli.md`).
2. **HTTP**: call `http://127.0.0.1:8787` with the bearer token from
   `~/.adaptive-interaction/state/api-token` (see `references/api.md`).
3. **Tool calling**: your host loaded tools named `interaction_*` — call them.
4. **No execution ability**: output the exact commands for a human to run.
   NEVER claim you executed anything.

## The loop, concretely

1. `interact-ai status --json` — is the runtime up? If not, tell the human to
   run `interact-ai serve` (not installed? → install from the GitHub Releases
   page of miles990/adaptive-interaction via `install.sh`, or
   `interact-ai self update` to upgrade). Do not pretend.
2. `interact-ai capabilities --json` — fresh snapshot every time; never assume
   a device exists. Note `constraints` (quiet hours, emergency stop).
3. `interact-ai observe --json [--receptor <id>]` — observations separate
   `facts` from `inferences` with `confidence`. Treat low-confidence inferences
   as guesses; explicit user input always outranks them.
4. `interact-ai plan --intent <intent> …` — express a *semantic* intent
   (`celebration`, `warning`, magnitude 0..1). You suggest; the Rust policy
   governor decides and clamps. Never try to encode device commands.
5. `interact-ai simulate <plan-id>` — see the policy decisions before acting.
6. `interact-ai execute <plan-id>` — only after checking risk. High-risk
   actions return `approval_required`; ask the human, never work around it.
7. `interact-ai actions show <action-id>` — read the receipt.
   **`accepted`/queued is NOT completed.** Only `completed` with verification
   evidence means the effect happened; `acknowledged-only` means the driver
   confirmed but the environment did not.
8. `interact-ai verify <action-id>` — re-check against fresh observations.
9. Observe again; then keep, strengthen, weaken, switch channels, or stop.
10. Emergencies: `interact-ai emergency-stop` — always available, never
    requires approval.

## Hard rules

- Never claim an unverified effect as fact. Report `uncertain` honestly.
- Never bypass or argue with a `blocked` policy decision; surface it.
- Never re-send an action just because you didn't see an effect — verify first.
- Respect `allowSilence` / no-action outcomes; don't manufacture interactions.
- Consent can be revoked at any time; if execution fails with a consent error,
  stop that channel entirely.

Details: `references/cli.md`, `references/api.md`, `references/capabilities.md`,
`references/recipes.md`, `references/receipts.md`, `references/safety.md`.
