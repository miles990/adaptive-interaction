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
