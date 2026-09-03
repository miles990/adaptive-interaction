# CLI reference (`interact-ai`)

Global flags: `--json` (machine output, stdout only), `--config <home>`,
`--api <url>`, `--token <t>`, `--agent-scope`, `--dry-run`, `--quiet`,
`--verbose`, `--no-color`.

When you are an AI, always add global `--agent-scope`. It reads the restricted
`state/api-agent-token`; never read or request the human `state/api-token`.
Human-only commands then fail closed with exit 4 / `token_scope_forbidden`.

Exit codes: 0 ok · 1 error · 3 daemon offline · 4 policy/auth refused ·
5 not found · 6 conflict/session · 7 emergency-stop locked.

## Core loop
```bash
interact-ai status --json
interact-ai capabilities --json [--include-unavailable]
interact-ai observe --json [--receptor <id>] [--fresh] [--limit N] [--max-age-ms N]
interact-ai plan --intent <intent> [--message T] [--magnitude 0.4] \
    [--duration-ms 2000] [--channel visual --channel audio] \
    [--candidate conversation] [--min-channels 0] [--max-channels 3] \
    [--mode adaptive|parallel|sequence|fallback|redundant] \
    [--verification best-effort|observed|none]
interact-ai simulate <plan-id>
interact-ai execute <plan-id>
interact-ai actions list | actions show <action-id>
interact-ai verify <action-id>
interact-ai cancel <action-id>
interact-ai stop --all           # cancel all open actions (soft)
interact-ai emergency-stop [--reason R]   # hard stop; --clear re-arms
```

## Sessions & consent（human-only 參考）

以 `--agent-scope` 執行下列建立／授權／續租指令會固定失敗。AI 只能建議人類在
控制中心或 human CLI 執行，不可改讀 human token。
```bash
interact-ai session start [--label L] [--ttl-minutes N] [--consent channel:haptic]
interact-ai session show
interact-ai session consent channel:haptic [--expires-minutes 30]
interact-ai session revoke channel:haptic
interact-ai session stop
```
Scopes: `channel:<name>`, `actuator:<id>`, `receptor:<id>`, `tool:<name>`.

## Components
```bash
interact-ai receptors list|inspect <id>|enable <id>|disable <id>|test <id>
interact-ai receptors push <id> --fact key=value [--confidence 0.9]
interact-ai receptors add builtin.push --id my.lane [--category custom] [--sensitive]
interact-ai actuators list|inspect|enable|disable|test|remove <id>
interact-ai actuators add builtin.mock-actuator --id dev.mock --channel haptic
```

## Self management (install / update / remove)
```bash
interact-ai self version [--check]        # version; --check compares with latest release
interact-ai self update [--version vX.Y.Z]  # sha256-verified atomic self-update
interact-ai self install-skill [--dest D] # cross-AI: detects all agent homes (Claude/Codex/Gemini/Copilot/~/.agents), menu-selectable, installs to all by default
interact-ai self install-desktop          # download the desktop control center bundle
interact-ai self uninstall --yes [--purge]
```
If `interact-ai` is missing entirely, tell the human to install from
https://github.com/miles990/adaptive-interaction/releases (run `install.sh`),
or build from source with cargo. Never fake its output.

## Human layer (names, pause, AI assists)
```bash
interact-ai capabilities --human [--locale zh-TW]  # human cards: displayName, badges,
                                                   # data/impact semantics, manifestHash
interact-ai catalog                                # common-capability catalog (canonical names/aliases)
interact-ai pause [--for 2h] [--reason R]          # pause PROACTIVE interactions (NOT emergency stop;
interact-ai resume                                 #  explicit requests still execute)
interact-ai prefs show | prefs set '{"mode":"advanced"}'
interact-ai onboarding                             # first-run wizard state
interact-ai describe actuator <id> --text "..." --manifest-hash <hash> [--locale zh-TW]
    # write an AI-assisted description; hash must match `capabilities --human`.
    # Descriptions are presentation only — they can NEVER change risk/consent facts.
interact-ai assists list                           # pending ai.assist requests (see safety.md)
interact-ai assists resolve <request-id> proceed|no-action [--note "..."]
interact-ai recipes summary <id> [--locale zh-TW]  # deterministic natural-language summary
interact-ai recipes simulate <id> --scenario '{"quietHours":true,"aiUnavailable":true}'
```

## Recipes / tools / misc
```bash
interact-ai recipes list|show <id>|validate <path-or-id>|apply <path>
interact-ai recipes enable|disable|simulate|run|remove <id>
interact-ai tools list|describe <name>
interact-ai tools call interaction.plan --input '{"intent":"success"}'
interact-ai tools export --format openai|anthropic|gemini|openapi|json-schema [--out F]
interact-ai events [--seconds 30]     # SSE tail
interact-ai outbox                    # rendered conversation/web-ui messages
interact-ai policy show | policy set '<json-merge-patch>' | policy validate
interact-ai serve [--host 127.0.0.1] [--port 8787]
```

## Providers (external devices / services / AI sessions)

Discovered ≠ paired ≠ installed ≠ enabled ≠ authorized — each is a separate
step, and the runtime refuses shortcut transitions.

```bash
interact-ai providers list
interact-ai providers scan                                      # metadata only; never starts sensors
interact-ai providers show <provider-id>
interact-ai providers pair <provider-id> --code <pairing-code>   # sha256 fingerprint; an IP is never identity
interact-ai providers transition <provider-id> --state installed|disabled|available
interact-ai providers revoke <provider-id>                       # capabilities disabled immediately; sticky
```

Declarative adapters live in `<home>/config/adapters/*.yaml` (File=Truth): a
validated YAML spec becomes a real HTTP/SSE receptor/actuator behind the SAME
governor. Secrets are `secret://name` references, never written in the spec.

## Agent sessions (leased, budgeted delegated work)

A session is NOT an identity — it is a lease with data/tool/consent scope and a
budget. An agent's report is a CLAIM (`claimed-completed`), never a receipt and
never verification.

```bash
interact-ai agents sessions                              # list (state, lease, budget)
interact-ai agents create --agent agent.coder [--ttl 120] [--max-messages 50]
interact-ai agents send <id> --kind task --body '{"task":"..."}'
interact-ai agents messages <id> [--direction to-session|from-session]  # fetching to-session marks delivery
interact-ai agents report <id> --event progress|claimed-completed|failed --payload '{}'
interact-ai agents renew <id> --extra-minutes 30         # only while open; expired = gone
interact-ai agents close <id> [--handoff '<bounded json>'] [--reason closed]
```

Delegation is limited deterministically by policy (max depth / cycle / session
count / message / cost). Emergency stop cancels every open session; open
sessions do not survive a runtime restart.

## Sensors (high-sensitivity; default OFF)

```bash
interact-ai sensors listen --ms 10000    # one bounded window; needs enable + explicit session consent
interact-ai sensors stop                 # stop all capture immediately
```

The microphone produces sound-level facts only (no raw audio, no STT, nothing
stored or transmitted); every window has a hard 30s ceiling. Capture is always
visible in `status.activeSensors` and `sensor.started/stopped` events — there is
no silent-capture path. The camera is deliberately unavailable in this build.

## v0.4: Local agent gateway (codex / claude-code)

```bash
interact-ai agents providers --json          # discovery: version/login/protocol (honest tri-state)
interact-ai agents providers --refresh --json
interact-ai agents route --kind code --json  # deterministic routing suggestion (advice only)
# Real subprocess sessions (read-only/plan mode; no credentials touched):
interact-ai agents create --agent claude-code --workdir /path --ttl 30 --max-cost 0.5 --json
# Human-only creation can add --allow-write after the UI/CLI shows the explicit workdir preview.
interact-ai agents send <sid> --kind task --body '{"task":"..."}'   # forwarded into the agent process
interact-ai agents show <sid> --json         # claimed-completed is a CLAIM, never verified
interact-ai agents messages <sid> --direction from-session --json   # results / approval-requests
interact-ai agents approve <sid> <requestId> --yes   # human resolves agent approvals (default deny)
interact-ai agents interrupt <sid>           # cancel the current turn
interact-ai agents close <sid>               # kills the whole process tree
```

## v0.4: Presentation surface / proactive dialogue

```bash
interact-ai presentation status --json       # companion window presence (honest offline/hidden)
interact-ai proactive status|mode <m>|quiet --minutes 60   # deterministic frequency limits
```

## v0.4: Memory / assets / knowledge

下列直接管理指令是 human/control-plane 參考；`--agent-scope` 會拒絕記憶、素材、
審核與發布端點。AI 讀取／提案應只使用 `interaction.knowledge_*` canonical
tools，寫入一律只會成為 Candidate。

```bash
interact-ai memory list --layer domain-know-how --json     # fact≠inference≠candidate labeled
interact-ai memory add --layer user-memory --kind preference --title t --content c
interact-ai memory bundle --task "..." --agent codex --domain rust --json  # what an agent would get
interact-ai memory export | delete <id> | clear-session    # no un-deletable memory exists
interact-ai assets import --path f | --text "..."          # content-addressed, write-once
interact-ai assets impact <hash> && interact-ai assets delete <hash>   # impact preview first
interact-ai knowledge search "q" --json                    # FTS+lexical candidates (not truth)
interact-ai knowledge propose-claim --title t --content c --evidence '[{"url":"..."}]'
interact-ai knowledge review <id> approve                  # ONLY humans activate; agents comment
interact-ai knowledge receipts | update-check <trigger>    # honest machine-readable trail
```

Hard rules for AIs: your writes are ALWAYS candidates; you cannot approve
your own proposals; analogies/conjecture can never claim causality; asset
sources are immutable; claimed-completed ≠ verified.

## v0.5: Character Presentation Protocol

```bash
interact-ai character status                      # protocol version, instances, active character
interact-ai character instances                   # desktop + external adapter instances (negotiated, tested, generation)
interact-ai character manifest                    # active desktop character manifest (404 before hello)
interact-ai character adapters list
interact-ai character adapters add --name "My engine" --manifest my.manifest.json   # prints adapter token ONCE
interact-ai character adapters revoke <adapterId> # token invalid + goodbye + disconnect
interact-ai character intent notice --message hi  # human manual test; safety intents (emergency/blocked/verified-success/...) are refused (403)
```

Truth states reach characters only through Runtime events (never through this CLI or an AI request). `completed` receipts from a character mean "the presentation played", never that work was verified.
