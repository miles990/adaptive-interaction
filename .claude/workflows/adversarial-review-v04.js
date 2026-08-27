export const meta = {
  name: 'adversarial-review-v04',
  description: 'Adversarial review of the v0.4 subsystems (presentation/gateway/memory/knowledge/curator/proactive/frontend)',
  phases: [
    { title: 'Review', detail: '8 dimension reviewers over the v0.4 code' },
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
    key: 'presentation-honesty',
    prompt: `You are a security/honesty reviewer. Repo: ${ROOT}. Read crates/interaction-runtime/src/presentation.rs, crates/interaction-api/src/routes.rs (presentation handlers), apps/interaction-desktop/src/companion/presentationCommands.ts, apps/interaction-desktop/src/companion/CompanionApp.tsx. Hunt ONLY for: ways a presentation receipt can reach completed without the surface actually rendering (forged/replayed acks, ack race with estop or watchdog Uncertain sweep, take_pending double-consume gaps), hidden-companion bypasses (ingest or actuator paths not gated), behaviorIntent/animation whitelist bypass (truth states playable by AI), pending-queue growth or TTL bugs, presence spoofing consequences. Concrete file+line findings only.`,
  },
  {
    key: 'gateway-safety',
    prompt: `You are a security reviewer. Repo: ${ROOT}. Read crates/interaction-agent-gateway/src/{lib.rs,claude.rs,codex.rs,process.rs} and crates/interaction-runtime/src/gateway.rs and the create/close/estop paths in crates/interaction-runtime/src/agents.rs. Hunt ONLY for: subprocess orphaning (paths where the child survives estop/close/restart/panic in the pump), approval flow holes (auto-approve paths, approvals resolvable by the wrong session, TTL deny races), credential leakage (env/token exposure to child or logs), command injection via workdir/prompt, budget bypass (cost accrued after ceiling, message budget races), stdin/stdout deadlocks (full pipes, waiting on RPC responses that never come), kill_tree failure on already-reaped children. Concrete file+line findings only.`,
  },
  {
    key: 'memory-privacy',
    prompt: `You are a privacy reviewer. Repo: ${ROOT}. Read crates/interaction-core/src/memory.rs, crates/interaction-runtime/src/memory.rs, the memory routes in crates/interaction-api/src/routes.rs, and the handoff/task-experience paths in crates/interaction-runtime/src/{agents.rs,curator.rs}. Hunt ONLY for: paths where agent-created content escapes the demotion rules (fact staying fact, long-term user memory without review), secrets slipping past looks_like_secret via update/patch paths, retention violations (expired items still served, delete_with_parent not honored, export leaking expired/sensitive), context-bundle leaks (stale/sensitive/denylisted/candidate items included, or agent_visibility confusion), memory_update allowing layer/creator changes that break invariants. Concrete file+line findings only.`,
  },
  {
    key: 'knowledge-integrity',
    prompt: `You are a data-integrity reviewer. Repo: ${ROOT}. Read crates/interaction-core/src/knowledge.rs, crates/interaction-runtime/src/knowledge.rs, crates/interaction-runtime/src/curator.rs, the knowledge/asset routes + tool dispatch arms in crates/interaction-api/src/routes.rs. Hunt ONLY for: candidate-only bypasses (any path where an agent/tool call yields an ACTIVE node or edge, incl. the human 'activate' flag reachable from AI surfaces), review gates skippable (approve without counterexamples via direct store writes exposed through APIs, agent approve becoming effective), CAS violations (asset overwrite/delete by AI paths, blob/metadata divergence), evidence checks bypassable (supersede/propose paths), FTS injection or panic, conflict-check marking wrong nodes, receipt fields lying (human_reviewed true without human). Concrete file+line findings only.`,
  },
  {
    key: 'proactive-limits',
    prompt: `You are a policy reviewer. Repo: ${ROOT}. Read crates/interaction-runtime/src/proactive.rs and the gate call site in crates/interaction-runtime/src/executor.rs. Hunt ONLY for: rate-limit bypasses (metadata omission making generative dialogue skip the gate entirely — check who sets proactiveClass, whether an AI-driven plan could just not declare it), state persistence races (two concurrent gates both passing the hourly cap), quiet/dnd not honored, safety-class abuse (everything declared safety to bypass limits — is that acceptable per design or a hole?), dedup key collisions suppressing safety messages. Concrete file+line findings only.`,
  },
  {
    key: 'state-machine-v04',
    prompt: `You are a correctness reviewer. Repo: ${ROOT}. Read crates/interaction-runtime/src/presentation.rs (ack/sweep), crates/interaction-runtime/src/gateway.rs (pump event ordering), crates/interaction-storage/src/lib.rs (v4-v6 tables), crates/interaction-runtime/src/agents.rs (close path changes). Hunt ONLY for: receipt/state transitions that violate the honesty ladder in the NEW code paths, sticky-terminal violations, mailbox delivered-stamp races between gateway_deliver and mailbox_fetch, close_agent_session prior_state capture bugs, storage timestamp format inconsistencies breaking pruning (memory_items/knowledge tables), migration idempotency issues v3→v6. Concrete file+line findings only.`,
  },
  {
    key: 'frontend-honesty',
    prompt: `You are a UI-honesty reviewer. Repo: ${ROOT}. Read apps/interaction-desktop/src/App.tsx, src/pages/{AiPage.tsx,MemoryKnowledgePage.tsx,CompanionPage.tsx,CapabilitiesHub.tsx,ActivityPage.tsx,HomePage.tsx}, src/components/GlobalSearch.tsx, src/companion/{behavior.ts,machine.ts,CompanionApp.tsx}. Hunt ONLY for: UI stating success/completion without receipt evidence, claimed-completed rendered as verified anywhere, localStorage/UI-only state pretending to be runtime truth, safety wording overridable or missing (estop reachable from every page? sensors visible?), stale closures/refresh bugs that could show outdated permission state after revoke, the NowStrip/Inbox miscounting pending items, GlobalSearch executing destructive commands without confirmation beyond estop (estop via search skips the two-step confirm — real issue?). Concrete file+line findings only.`,
  },
  {
    key: 'regression-v03',
    prompt: `You are a regression reviewer. Repo: ${ROOT}. Read crates/interaction-runtime/src/{runtime.rs,executor.rs} focusing on v0.4-added code (presentation gate in ingest, proactive gate in run_step, watchdog additions, orchestrator is_text_channel change), crates/interaction-core/src/policy.rs defaults, apps/interaction-desktop/src/App.tsx nav compat. Hunt ONLY for: v0.3 behaviors broken by v0.4 edits — recipes silenced unexpectedly, desktop-pet channel additions affecting existing planner scoring for conversation/notification, ingest gate misfiring on non-companion receptors, watchdog per-tick cost growth (gateway_sweep + sweep_presentation every tick — lock contention), legacy tab ids/tray navigation broken, policy default allowlist changes weakening prior restrictions. Concrete file+line findings only.`,
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
      `Repo: ${ROOT}. A reviewer claims this defect:\n\nTITLE: ${f.title}\nFILE: ${f.file}${f.line ? ':' + f.line : ''}\nSEVERITY: ${f.severity}\nDETAIL: ${f.detail}\n\nYour job: REFUTE it. Read the actual code (and its tests) carefully. Consider whether existing tests, callers, or invariants already prevent the claimed failure, whether the scenario is actually reachable, and whether the claim misreads the code. Only confirm isReal=true if you can trace a concrete reachable failure path. Default to isReal=false when uncertain. Include a suggestedFix only when confirming.`,
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
