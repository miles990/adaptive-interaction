export const meta = {
  name: 'adversarial-review-adaptive-interaction',
  description: 'Multi-lens review of the adaptive-interaction platform, findings adversarially verified',
  phases: [
    { title: 'Review', detail: '5 dimension reviewers over the Rust workspace' },
    { title: 'Verify', detail: 'adversarial refutation of each finding' },
  ],
}

const ROOT = '/Users/user/Workspace/claude-lab/adaptive-interaction'
const FINDINGS_SCHEMA = {
  type: 'object',
  required: ['findings'],
  properties: {
    findings: {
      type: 'array',
      items: {
        type: 'object',
        required: ['title', 'file', 'severity', 'detail'],
        properties: {
          title: { type: 'string' },
          file: { type: 'string' },
          line: { type: 'integer' },
          severity: { enum: ['critical', 'high', 'medium', 'low'] },
          detail: { type: 'string' },
        },
      },
    },
  },
}
const VERDICT_SCHEMA = {
  type: 'object',
  required: ['isReal', 'reasoning'],
  properties: {
    isReal: { type: 'boolean' },
    reasoning: { type: 'string' },
    suggestedFix: { type: 'string' },
  },
}

const DIMENSIONS = [
  {
    key: 'safety-bypass',
    prompt: `You are a security reviewer. Repo: ${ROOT}. Read crates/interaction-policy/src/lib.rs, crates/interaction-runtime/src/executor.rs, crates/interaction-runtime/src/runtime.rs, crates/interaction-api/src/routes.rs, crates/interaction-api/src/lib.rs. Hunt ONLY for ways an AI, HTTP caller or Tauri frontend could cause a side effect WITHOUT the deterministic policy governor bounding it: public execute paths that skip Governor::authorize, test/simulate endpoints with real side effects, consent checks that can be raced or skipped, emergency-stop gaps (paths that still dispatch while estop engaged), token/auth weaknesses. Report only concrete, code-anchored findings (file+line+why). No style nits.`,
  },
  {
    key: 'state-machine',
    prompt: `You are a correctness reviewer. Repo: ${ROOT}. Read crates/interaction-core/src/receipt.rs, crates/interaction-runtime/src/executor.rs, crates/interaction-adapter-sdk/src/lib.rs, crates/interaction-storage/src/lib.rs. Hunt ONLY for action-receipt state machine bugs: any path where accepted/queued could be reported as completed without acknowledgement, illegal transitions that silently succeed, merge_driver_receipt trusting driver claims it shouldn't, receipts persisted with wrong channel (empty channel string breaking budget queries), watchdog TTL races, verification marking completed without evidence. Concrete findings only (file+line+why).`,
  },
  {
    key: 'async-hygiene',
    prompt: `You are a Rust async reviewer. Repo: ${ROOT}. Read crates/interaction-runtime/src/runtime.rs, crates/interaction-runtime/src/executor.rs, crates/interaction-events/src/lib.rs, crates/interaction-api/src/sse.rs, adapters/builtin/src/actuators.rs. Hunt ONLY for: deadlocks (RwLock held across await that re-locks, e.g. session write guard held while calling methods that lock session again), blocking calls in async context (std Mutex/rusqlite under load, block_on inside runtime), orphan tasks that outlive shutdown, unbounded growth (vectors/maps never pruned), panics reachable in production paths (expect/unwrap on poisoned locks is acceptable; look for worse). Concrete findings only (file+line+why).`,
  },
  {
    key: 'api-contract',
    prompt: `You are an API reviewer. Repo: ${ROOT}. Read crates/interaction-api/src/routes.rs, crates/interaction-api/src/lib.rs, crates/interaction-api/src/dto.rs, crates/interaction-cli/src/commands.rs, crates/interaction-cli/src/client.rs. Hunt ONLY for: route/handler mismatches, auth middleware gaps (routes reachable without token that shouldn't be), error mapping lies (policy block returning 200), CLI commands that hit non-existent endpoints or parse responses wrongly, tool dispatcher inconsistencies vs the canonical manifest in crates/interaction-tool-schema/src/lib.rs (tools advertised but not dispatchable or vice versa). Concrete findings only (file+line+why).`,
  },
  {
    key: 'recipe-engine',
    prompt: `You are a logic reviewer. Repo: ${ROOT}. Read crates/interaction-recipe/src/trigger.rs, crates/interaction-recipe/src/fusion.rs, crates/interaction-recipe/src/condition.rs, crates/interaction-runtime/src/runtime.rs (recipe evaluation part), crates/interaction-runtime/src/orchestrator.rs. Hunt ONLY for: trigger evaluation bugs (sequence ordering, window math, stale observation acceptance), recipe cooldown/limit bypasses (state not persisted so restart resets cooldowns — is that safe?), fusion bugs (explicit input NOT overriding inference, contradiction resolution wrong), orchestrator scoring bugs that could select a consent-gated/high-risk actuator when a safe one exists, infinite loops (recipe execution triggering observations that re-trigger recipes). Concrete findings only (file+line+why).`,
  },
]

phase('Review')
const results = await parallel(
  DIMENSIONS.map((d) => () =>
    agent(
      d.prompt +
        ' Return findings as structured output. If the code is genuinely sound in your dimension, return an empty findings array — do NOT invent findings.',
      { label: `review:${d.key}`, phase: 'Review', schema: FINDINGS_SCHEMA }
    )
  )
)
const all = results.filter(Boolean).flatMap((r, i) =>
  r.findings.map((f) => ({ ...f, dimension: DIMENSIONS[i].key }))
)
log(`${all.length} raw findings across ${DIMENSIONS.length} dimensions`)

// Dedup by file+title similarity (cheap key)
const seen = new Set()
const deduped = all.filter((f) => {
  const key = f.file + '|' + f.title.toLowerCase().slice(0, 40)
  if (seen.has(key)) return false
  seen.add(key)
  return true
})

phase('Verify')
const verified = await parallel(
  deduped.map((f) => () =>
    agent(
      `Repo: ${ROOT}. A reviewer claims this defect:\n\nTITLE: ${f.title}\nFILE: ${f.file}${f.line ? ':' + f.line : ''}\nSEVERITY: ${f.severity}\nDETAIL: ${f.detail}\n\nYour job: REFUTE it. Read the actual code (and its tests in the same crate) carefully. Consider whether existing tests, callers, or invariants already prevent the claimed failure, whether the scenario is actually reachable, and whether the claim misreads the code. Only confirm isReal=true if you can trace a concrete reachable failure path. Default to isReal=false when uncertain. Include a suggestedFix only when confirming.`,
      { label: `verify:${f.title.slice(0, 30)}`, phase: 'Verify', schema: VERDICT_SCHEMA }
    ).then((v) => ({ finding: f, verdict: v }))
  )
)

const confirmed = verified
  .filter(Boolean)
  .filter((v) => v.verdict && v.verdict.isReal)
  .map((v) => ({
    ...v.finding,
    reasoning: v.verdict.reasoning,
    suggestedFix: v.verdict.suggestedFix || null,
  }))
log(`${confirmed.length}/${deduped.length} findings confirmed after adversarial verification`)
return { confirmed, rejectedCount: deduped.length - confirmed.length }