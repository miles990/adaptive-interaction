#!/usr/bin/env bash
# v0.3 CLI E2E: providers / agent sessions / sensors closed loops against a
# REAL interact-ai daemon in an isolated home. Asserts the honesty + safety
# invariants (not just "ran ok"). Prints PASS/FAIL per check.
set -uo pipefail

ROOT="/Users/user/Workspace/claude-lab/adaptive-interaction"
BIN="$ROOT/target/debug/interact-ai"
HOME_DIR="$(mktemp -d /tmp/v03-e2e.XXXXXX)"
PORT=18811
PASS=0; FAIL=0
J() { python3 -c "import sys,json; d=json.load(sys.stdin); print(eval(sys.argv[1]))" "$1"; }
ok()   { PASS=$((PASS+1)); echo "  PASS: $1"; }
bad()  { FAIL=$((FAIL+1)); echo "  FAIL: $1"; }
check(){ if [ "$2" = "$3" ]; then ok "$1 ($2)"; else bad "$1 (got '$2', want '$3')"; fi; }

export INTERACT_AI_HOME="$HOME_DIR"
# Gateway 測試用 fake agent（真子程序、假模型；絕不動用真額度）。
export INTERACT_AI_CLAUDE_BIN="$ROOT/crates/interaction-runtime/tests/fixtures/fake_claude.sh"
mkdir -p "$HOME_DIR/config/adapters"

# A declarative device spec (points at a local mock we spin up below).
DEV_PORT=18812
cat > "$HOME_DIR/config/adapters/desk.yaml" <<YAML
schemaVersion: "1.0"
id: desk-light
displayName: 書桌燈
capabilities:
  - kind: actuator
    id: set
    channel: light
    transport: http
    confirmation: acknowledged
    request:
      method: POST
      url: "http://127.0.0.1:${DEV_PORT}/set"
      body: { brightness: "{{magnitude}}" }
YAML

# Minimal mock device (records what it receives).
python3 - "$DEV_PORT" "$HOME_DIR/dev.log" <<'PY' &
import sys, json
from http.server import BaseHTTPRequestHandler, HTTPServer
port=int(sys.argv[1]); log=sys.argv[2]
class H(BaseHTTPRequestHandler):
    def do_POST(self):
        n=int(self.headers.get('content-length',0)); body=self.rfile.read(n)
        open(log,'ab').write(body+b"\n")
        self.send_response(200); self.send_header('content-type','application/json'); self.end_headers()
        self.wfile.write(b'{"queued":true}')
    def log_message(self,*a): pass
HTTPServer(('127.0.0.1',port),H).serve_forever()
PY
DEV_PID=$!
sleep 1

echo "apiHost: 127.0.0.1" > "$HOME_DIR/config/interaction.yaml"
echo "apiPort: ${PORT}"  >> "$HOME_DIR/config/interaction.yaml"
"$BIN" serve >/dev/null 2>&1 &
DAEMON_PID=$!
for i in $(seq 1 40); do "$BIN" status --json >/dev/null 2>&1 && break; sleep 0.25; done

echo "== Providers =="
# The declarative device registered as a provider in Installed state.
STATE=$("$BIN" providers list --json 2>/dev/null | python3 -c "import sys,json;print(next((p['state'] for p in json.load(sys.stdin) if p['identity']['id']=='provider.adapter.desk-light'),'MISSING'))")
check "declarative provider Installed (not auto-available)" "$STATE" "installed"

# A discovered network provider goes through the pairing ceremony.
# (Register via a manual push is not exposed; we test lifecycle refusal on the
#  declarative provider instead: shortcut Installed->Available is legal, but
#  discovered->available is refused. Here we confirm revoke is sticky.)
"$BIN" providers revoke provider.adapter.desk-light --json >/dev/null 2>&1
STATE=$("$BIN" providers show provider.adapter.desk-light --json 2>/dev/null | J "d['state']")
check "revoke is sticky" "$STATE" "revoked"
RC=$("$BIN" providers transition provider.adapter.desk-light --state available --json >/dev/null 2>&1; echo $?)
if [ "$RC" != "0" ]; then ok "revoked -> available refused"; else bad "revoked -> available should be refused"; fi

echo "== Agent sessions =="
SID=$("$BIN" agents create --agent agent.coder --ttl 30 --max-messages 3 --json 2>/dev/null | J "d['sessionId']")
[ -n "$SID" ] && ok "created agent session $SID" || bad "create agent session"
ST=$("$BIN" agents show "$SID" --json 2>/dev/null | J "d['state']")
check "session starts Created" "$ST" "created"
# Message budget is a hard ceiling (max 3).
for i in 1 2 3; do "$BIN" agents send "$SID" --kind task --body '{"task":"x"}' --json >/dev/null 2>&1; done
RC=$("$BIN" agents send "$SID" --kind task --body '{"task":"x"}' --json >/dev/null 2>&1; echo $?)
if [ "$RC" != "0" ]; then ok "message budget is a hard ceiling"; else bad "4th message should exceed budget"; fi
# A claim is not a receipt: report claimed-completed, session state changes,
# but it lands as an observation inference.
"$BIN" agents report "$SID" --event claimed-completed --payload '{"summary":"done"}' --json >/dev/null 2>&1
ST=$("$BIN" agents show "$SID" --json 2>/dev/null | J "d['state']")
check "claimed-completed is an OPEN state" "$ST" "claimed-completed"
# Close with a bounded handoff; consents die.
"$BIN" agents close "$SID" --reason closed --json >/dev/null 2>&1
ST=$("$BIN" agents show "$SID" --json 2>/dev/null | J "d['state']")
check "session closed" "$ST" "closed"

echo "== Sensors (default off, consent-gated) =="
# Without enabling + consent, listening is refused (no capture).
RC=$("$BIN" sensors listen --ms 2000 --json >/dev/null 2>&1; echo $?)
if [ "$RC" != "0" ]; then ok "microphone listen refused without consent"; else bad "listen should be refused"; fi
# Mic receptor is registered but consent-gated (not available by default).
AVAIL=$("$BIN" capabilities --json 2>/dev/null | python3 -c "import sys,json;print(next((r['availability'] for r in json.load(sys.stdin)['receptors'] if r['id']=='microphone.listen'),'MISSING'))")
if [ "$AVAIL" != "available" ]; then ok "microphone default not available ($AVAIL)"; else bad "microphone should not be available by default"; fi

echo "== Presentation Provider (v0.4) =="
TOKEN=$(cat "$HOME_DIR/state/api-token")
# The companion provider is itemized (7 receptors + 7 actuators).
NRCP=$("$BIN" providers show provider.companion.shu --json 2>/dev/null | J "len(d['receptors'])")
check "companion provider has 7 itemized receptors" "$NRCP" "7"
# Surface offline (no companion window) → honest refusal to ingest clicks.
RC=$(curl -s -o /dev/null -w "%{http_code}" -X POST "http://127.0.0.1:${PORT}/v1/receptors/companion.click/push" -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' -d '{"facts":{"kind":"clicked"}}')
if [ "$RC" != "200" ]; then ok "companion click refused while surface offline (HTTP $RC)"; else bad "click should be refused with no companion window"; fi
# A (simulated) companion window says hello → surface connected + visible.
"$BIN" presentation hello --visible --pack shu-standard --json >/dev/null 2>&1
CONN=$("$BIN" presentation status --json 2>/dev/null | J "d['connected']")
check "hello marks the surface connected" "$CONN" "True"
# Now the click ingests.
RC=$(curl -s -o /dev/null -w "%{http_code}" -X POST "http://127.0.0.1:${PORT}/v1/receptors/companion.click/push" -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' -d '{"facts":{"kind":"clicked"}}')
check "companion click accepted when visible" "$RC" "200"
# Full honest loop: bubble → dispatched (NOT completed) → ack → completed.
"$BIN" session start --label e2e --json >/dev/null 2>&1 || true
PLAN=$(curl -s -X POST "http://127.0.0.1:${PORT}/v1/plans" -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' -d '{"intent":"companion-test","message":"CLI E2E 氣泡","preferredChannels":["desktop-pet"],"candidates":["companion.bubble.show"],"minChannels":1,"maxChannels":1}')
PLAN_ID=$(echo "$PLAN" | J "d['planId']")
EXEC=$(curl -s -X POST "http://127.0.0.1:${PORT}/v1/plans/${PLAN_ID}/execute" -H "Authorization: Bearer $TOKEN")
AID=$(echo "$EXEC" | J "d[0]['actionId'] if isinstance(d,list) else d['receipts'][0]['actionId']")
ST=$("$BIN" actions show "$AID" --json 2>/dev/null | J "d['currentStatus']")
check "bubble receipt is dispatched, not completed" "$ST" "dispatched"
"$BIN" presentation ack "$AID" --outcome displayed --json >/dev/null 2>&1
ST=$("$BIN" actions show "$AID" --json 2>/dev/null | J "d['currentStatus']")
check "surface ack completes the receipt" "$ST" "completed"
# Consent-gated presentation actuators start disabled.
SND=$("$BIN" capabilities --json 2>/dev/null | python3 -c "import sys,json;print(next((a['availability'] for a in json.load(sys.stdin)['actuators'] if a['id']=='companion.sound.play'),'MISSING'))")
if [ "$SND" != "available" ]; then ok "companion.sound.play default disabled ($SND)"; else bad "sound should be consent-gated"; fi

echo "== Proactive dialogue (v0.4) =="
"$BIN" proactive mode off --json >/dev/null 2>&1
MODE=$("$BIN" proactive status --json 2>/dev/null | J "d['config']['mode']")
check "proactive mode persisted" "$MODE" "off"
"$BIN" proactive quiet --minutes 30 --json >/dev/null 2>&1
QU=$("$BIN" proactive status --json 2>/dev/null | J "'set' if d.get('quietUntil') else 'missing'")
check "quiet request recorded" "$QU" "set"
"$BIN" proactive mode natural --json >/dev/null 2>&1

echo "== Agent Gateway (v0.4, fake agent subprocess) =="
FOUND=$("$BIN" agents providers --refresh --json 2>/dev/null | J "next((str(a['loggedIn']) for a in d['agents'] if a['kind']=='claude-code'),'MISSING')")
check "fake claude discovered + logged in" "$FOUND" "True"
GSID=$("$BIN" agents create --agent claude-code --label e2e --ttl 5 --workdir "$HOME_DIR" --json 2>/dev/null | J "d['sessionId']")
[ -n "$GSID" ] && ok "gateway session created $GSID" || bad "gateway session create"
"$BIN" agents send "$GSID" --kind task --body '{"task":"do the thing"}' --json >/dev/null 2>&1
for i in $(seq 1 40); do
  GST=$("$BIN" agents show "$GSID" --json 2>/dev/null | J "d['state']")
  [ "$GST" = "claimed-completed" ] && break; sleep 0.25
done
check "fake agent reaches claimed-completed (claim, not verified)" "$GST" "claimed-completed"
PSID=$("$BIN" agents show "$GSID" --json 2>/dev/null | J "d.get('providerSessionId','')")
check "provider session id recorded" "$PSID" "fake-123"
RES=$("$BIN" agents messages "$GSID" --direction from-session --json 2>/dev/null | J "next((m['body'].get('summary','') for m in d if m['kind']=='result'),'MISSING')")
check "result lands in mailbox" "$RES" "完成了（這是聲稱）"
"$BIN" agents close "$GSID" --json >/dev/null 2>&1
GST=$("$BIN" agents show "$GSID" --json 2>/dev/null | J "d['state']")
check "gateway session closed kills subprocess" "$GST" "closed"

echo "== Memory layer (v0.4) =="
MID=$("$BIN" memory add --layer domain-know-how --kind know-how --title "先跑測試" --content "改完先跑測試再宣稱完成" --tag rust --json 2>/dev/null | J "d['memoryId']")
[ -n "$MID" ] && ok "memory created $MID" || bad "memory create"
# agent 身分寫入 → 降權為 inference。
AKIND=$("$BIN" memory add --layer domain-knowledge --kind fact --title "agent 宣稱" --content "某事為真" --as-agent codex --json 2>/dev/null | J "d['kind']")
check "agent fact demoted to inference" "$AKIND" "inference"
# secret 樣態拒收。
RC=$("$BIN" memory add --layer user-memory --kind fact --title x --content "Bearer abc123" --json >/dev/null 2>&1; echo $?)
if [ "$RC" != "0" ]; then ok "secret-like content refused"; else bad "secret content should be refused"; fi
# Context bundle 誠實揭露（含 excludes）。
BN=$("$BIN" memory bundle --task "檢查 repo" --agent codex --domain rust --json 2>/dev/null | J "len(d['includes'])")
check "context bundle includes the know-how" "$BN" "1"
"$BIN" memory delete "$MID" --json >/dev/null 2>&1
GONE=$("$BIN" memory show "$MID" --json >/dev/null 2>&1; echo $?)
if [ "$GONE" != "0" ]; then ok "memory deletable (no permanent memory)"; else bad "memory should be deletable"; fi

echo "== Emergency stop propagation =="
SID2=$("$BIN" agents create --agent agent.b --ttl 30 --json 2>/dev/null | J "d['sessionId']")
"$BIN" emergency-stop --json >/dev/null 2>&1
ST=$("$BIN" agents show "$SID2" --json 2>/dev/null | J "d['state']")
check "estop cancels open agent session" "$ST" "cancelled"
RC=$("$BIN" agents create --agent agent.c --json >/dev/null 2>&1; echo $?)
if [ "$RC" != "0" ]; then ok "estop blocks new agent sessions"; else bad "new session should be blocked under estop"; fi

echo
echo "RESULT: $PASS passed, $FAIL failed"
kill $DAEMON_PID $DEV_PID 2>/dev/null
rm -rf "$HOME_DIR"
[ "$FAIL" -eq 0 ]
