# CLI reference (`interact-ai`)

Global flags: `--json` (machine output, stdout only), `--config <home>`,
`--api <url>`, `--token <t>`, `--dry-run`, `--quiet`, `--verbose`, `--no-color`.

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

## Sessions & consent
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
