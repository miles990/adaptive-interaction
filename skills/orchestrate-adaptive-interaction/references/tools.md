# Tool operations

Canonical tools (namespace `interaction.*`; platform hosts see `interaction_*`):

| Tool | Role | Risk | Notes |
|---|---|---|---|
| interaction.status | receptor | read-only | runtime reachable? e-stop? |
| interaction.capabilities | receptor | read-only | fresh snapshot before planning |
| interaction.observe | receptor | read-only | facts vs inferences |
| interaction.plan | actuator | low | semantic intent → plan |
| interaction.simulate | receptor | read-only | policy dry-run |
| interaction.execute | actuator | bounded-side-effect | accepted ≠ completed |
| interaction.action_status | receptor | read-only | receipt state machine |
| interaction.verify | both | low | re-check vs observations |
| interaction.cancel | actuator | low | cancel one action |
| interaction.stop | actuator | low | EMERGENCY STOP, never gated |
| interaction.recipe_run | actuator | bounded-side-effect | trigger bypassed, policy not |
| interaction.policy | receptor | read-only | effective limits |

Export host-specific definitions from the single canonical manifest:
`interact-ai tools export --format openai|anthropic|gemini|openapi|json-schema`.
Every export ships a `companionPolicy` carrying risk/approval metadata the host
format cannot express — hosts should honor it.
External tools (GitHub, browser, files) are classified per *operation*
(read = receptor, write = actuator, high-risk writes need approval); their
results must flow back in as observations to close the loop.
