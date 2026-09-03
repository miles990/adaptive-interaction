#!/usr/bin/env bash
# v0.3 CLI E2E: providers / agent sessions / sensors closed loops against a
# REAL interact-ai daemon in an isolated home. Asserts the honesty + safety
# invariants (not just "ran ok"). Prints PASS/FAIL per check.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$ROOT/target/debug/interact-ai"
HOME_DIR="$(mktemp -d /tmp/v03-e2e.XXXXXX)"
# Agent 工作資料夾必須是獨立目錄：runtime 自 v0.5.0 起拒絕把自己的狀態資料夾
# （含 human api-token）當 workdir（agent-honesty-022 回歸）。
WORK_DIR="$(mktemp -d /tmp/v03-e2e-work.XXXXXX)"
PORT=18811
PASS=0; FAIL=0
J() { python3 -c "import sys,json; d=json.load(sys.stdin); print(eval(sys.argv[1]))" "$1"; }
ok()   { PASS=$((PASS+1)); echo "  PASS: $1"; }
bad()  { FAIL=$((FAIL+1)); echo "  FAIL: $1"; }
check(){ if [ "$2" = "$3" ]; then ok "$1 ($2)"; else bad "$1 (got '$2', want '$3')"; fi; }

export INTERACT_AI_HOME="$HOME_DIR"
# Always exercise the current source tree. A stale target/debug binary can
# otherwise make this acceptance script report yesterday's Runtime truth.
cargo build --manifest-path "$ROOT/Cargo.toml" -p interaction-cli >/dev/null || exit 1
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

# --- v0.5：ESP32 serial【模擬器】（pty；與參考韌體同一線協定）---
# 明確標示：模擬器驗收，非真機。配對碼走 secret://（環境變數），不落 YAML。
export INTERACT_AI_SECRET_SIM_PAIR="9927"
SIM_PTY_FILE="$HOME_DIR/sim-pty-path"
# 感測面控制通道（韌體上是真實感測器）：--facts-file 覆寫 button/distanceMm/
# lux/tempC，SIGUSR1 翻轉按鈕——閉環的「獨立觀察」半邊靠它在模擬器上驗。
SIM_FACTS="$HOME_DIR/sim-facts.json"
echo '{}' > "$SIM_FACTS"
python3 "$(dirname "$0")/esp32-serial-sim.py" \
  --device-id esp32-sim01 --pairing-code 9927 \
  --pty-path-file "$SIM_PTY_FILE" --log "$HOME_DIR/sim.log" \
  --facts-file "$SIM_FACTS" 2>/dev/null &
SIM_PID=$!
for i in $(seq 1 20); do [ -s "$SIM_PTY_FILE" ] && break; sleep 0.2; done
SIM_PTY=$(cat "$SIM_PTY_FILE" 2>/dev/null || echo "/dev/null")
cat > "$HOME_DIR/config/adapters/esp32-desk.yaml" <<YAML
schemaVersion: "1.0"
id: esp32-desk
displayName: ESP32 書桌裝置（模擬器）
capabilities:
  - kind: actuator
    id: vibe
    channel: haptic
    transport: serial
    timeoutMs: 4000
    command:
      name: "vibe.pulse"
      params: { strength: "{{magnitude}}", durationMs: "{{durationMs}}" }
    serial:
      port: "${SIM_PTY}"
      baud: 115200
      expectedDeviceId: "esp32-sim01"
      pairingCode: "secret://sim-pair"
  # 韌體硬限制展示用：params 是 spec 作者寫死的常數（不是 AI 可調的值），
  # 故意超界 → 裝置端 clamp 後以 ack.applied 誠實回報實際值。
  - kind: actuator
    id: servo
    channel: motion
    transport: serial
    timeoutMs: 4000
    command:
      name: "servo.move"
      params: { angle: 999 }
    serial:
      port: "${SIM_PTY}"
      baud: 115200
      expectedDeviceId: "esp32-sim01"
      pairingCode: "secret://sim-pair"
  - kind: actuator
    id: buzz
    channel: sound
    transport: serial
    timeoutMs: 4000
    command:
      name: "buzzer.beep"
      params: { freqHz: 99999, durationMs: 9999 }
    serial:
      port: "${SIM_PTY}"
      baud: 115200
      expectedDeviceId: "esp32-sim01"
      pairingCode: "secret://sim-pair"
  - kind: receptor
    id: env
    transport: serial
    timeoutMs: 4000
    facts:
      lux: "/facts/lux"
      distanceMm: "/facts/distanceMm"
      tempC: "/facts/tempC"
      button: "/facts/button"
      servoAngle: "/facts/servoAngle"
    serial:
      port: "${SIM_PTY}"
      baud: 115200
      expectedDeviceId: "esp32-sim01"
      pairingCode: "secret://sim-pair"
YAML

echo "apiHost: 127.0.0.1" > "$HOME_DIR/config/interaction.yaml"
echo "apiPort: ${PORT}"  >> "$HOME_DIR/config/interaction.yaml"
"$BIN" serve >/dev/null 2>&1 &
DAEMON_PID=$!
cleanup() {
  kill "$DAEMON_PID" "$DEV_PID" "$SIM_PID" 2>/dev/null || true
  wait "$DAEMON_PID" "$DEV_PID" "$SIM_PID" 2>/dev/null || true
  if [[ "$HOME_DIR" == /tmp/v03-e2e.* && -d "$HOME_DIR" ]]; then
    rm -rf -- "$HOME_DIR"
  fi
  if [[ "$WORK_DIR" == /tmp/v03-e2e-work.* && -d "$WORK_DIR" ]]; then
    rm -rf -- "$WORK_DIR"
  fi
}
trap cleanup EXIT INT TERM
for i in $(seq 1 40); do "$BIN" status --json >/dev/null 2>&1 && break; sleep 0.25; done

echo "== Scoped API token boundary =="
RC=$("$BIN" --agent-scope status --json >/dev/null 2>&1; echo $?)
check "restricted agent token can read status" "$RC" "0"
RC=$("$BIN" --agent-scope session start --label forbidden --json >/dev/null 2>&1; echo $?)
if [ "$RC" != "0" ]; then ok "restricted agent token cannot create/grant a human session"; else bad "agent token must not create session"; fi
RC=$("$BIN" --agent-scope policy set '{"requireApprovalAt":"critical"}' --json >/dev/null 2>&1; echo $?)
if [ "$RC" != "0" ]; then ok "restricted agent token cannot weaken policy"; else bad "agent token must not mutate policy"; fi

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
# v0.5: claim → verified 只能由人工驗證（human token）完成，且只能一次。
HV=$("$BIN" agents show "$SID" --json 2>/dev/null | J "d.get('humanVerified') is None")
check "claim starts unverified" "$HV" "True"
"$BIN" agents verify "$SID" --note "e2e checked output" --json >/dev/null 2>&1
HV=$("$BIN" agents show "$SID" --json 2>/dev/null | J "d['humanVerified']['note']")
check "human verify records the note" "$HV" "e2e checked output"
RC=$("$BIN" agents verify "$SID" --json >/dev/null 2>&1; echo $?)
if [ "$RC" != "0" ]; then ok "double verify refused"; else bad "double verify should be refused"; fi
# Close with a bounded handoff; consents die.
"$BIN" agents close "$SID" --reason closed --json >/dev/null 2>&1
ST=$("$BIN" agents show "$SID" --json 2>/dev/null | J "d['state']")
check "session closed" "$ST" "closed"

echo "== Sensors (default off, consent-gated) =="
# Without enabling + consent, listening is refused (no capture).
RC=$("$BIN" sensors listen --ms 2000 --json >/dev/null 2>&1; echo $?)
if [ "$RC" != "0" ]; then ok "microphone listen refused without consent"; else bad "listen should be refused"; fi
# Mic receptor is registered but consent-gated (not available by default).
AVAIL=$("$BIN" capabilities --include-unavailable --json 2>/dev/null | python3 -c "import sys,json;print(next((r['availability'] for r in json.load(sys.stdin)['receptors'] if r['id']=='microphone.listen'),'MISSING'))")
if [ "$AVAIL" = "disabled" ]; then ok "microphone default disabled ($AVAIL)"; else bad "microphone must be present and disabled (got $AVAIL)"; fi

echo "== Presentation Provider (v0.4) =="
TOKEN=$(cat "$HOME_DIR/state/api-token")
# The companion provider is itemized (7 receptors + 7 actuators).
NRCP=$("$BIN" providers show provider.companion.desktop --json 2>/dev/null | J "len(d['receptors'])")
check "companion provider has 7 itemized receptors" "$NRCP" "7"
PNAME=$("$BIN" providers show provider.companion.desktop --json 2>/dev/null | J "d['identity']['displayName']")
check "companion provider name is honest before any character hello" "$PNAME" "桌面角色（尚未連線）"
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
SND=$("$BIN" capabilities --include-unavailable --json 2>/dev/null | python3 -c "import sys,json;print(next((a['availability'] for a in json.load(sys.stdin)['actuators'] if a['id']=='companion.sound.play'),'MISSING'))")
if [ "$SND" = "disabled" ]; then ok "companion.sound.play default disabled ($SND)"; else bad "sound must be present and disabled (got $SND)"; fi

echo "== Proactive dialogue (v0.4) =="
"$BIN" proactive mode off --json >/dev/null 2>&1
MODE=$("$BIN" proactive status --json 2>/dev/null | J "d['config']['mode']")
check "proactive mode persisted" "$MODE" "off"
"$BIN" proactive quiet --minutes 30 --json >/dev/null 2>&1
QU=$("$BIN" proactive status --json 2>/dev/null | J "'set' if d.get('quietUntil') else 'missing'")
check "quiet request recorded" "$QU" "set"
"$BIN" proactive mode natural --json >/dev/null 2>&1
"$BIN" proactive set --max-per-hour 4 --min-interval-minutes 11 --merge-window-seconds 25 --daily-sessions 2 --daily-cost-usd 0.5 --generative-agent claude-code --json >/dev/null 2>&1
PAGENT=$("$BIN" proactive status --json 2>/dev/null | J "d['config']['generativeAgent']")
check "proactive generative agent is explicit" "$PAGENT" "claude-code"

echo "== Agent Gateway (v0.4, fake agent subprocess) =="
FOUND=$("$BIN" agents providers --refresh --json 2>/dev/null | J "next((str(a['loggedIn']) for a in d['agents'] if a['kind']=='claude-code'),'MISSING')")
check "fake claude discovered + logged in" "$FOUND" "True"
GSID=$("$BIN" agents create --agent claude-code --label e2e --ttl 5 --workdir "$WORK_DIR" --json 2>/dev/null | J "d['sessionId']")
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
CBN=$("$BIN" agents show "$GSID" --json 2>/dev/null | J "len(d.get('contextBundles',[]))")
check "exact context bundle receipt persisted on real task" "$CBN" "1"
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
BN=$("$BIN" memory bundle --task "檢查 repo" --agent codex --domain rust --json 2>/dev/null | J "any(i['title']=='先跑測試' for i in d['includes'])")
check "context bundle includes the know-how" "$BN" "True"
"$BIN" memory delete "$MID" --json >/dev/null 2>&1
GONE=$("$BIN" memory show "$MID" --json >/dev/null 2>&1; echo $?)
if [ "$GONE" != "0" ]; then ok "memory deletable (no permanent memory)"; else bad "memory should be deletable"; fi

echo "== Knowledge system (v0.4) =="
DP=$("$BIN" knowledge domain-packs --json 2>/dev/null | J "d['count']")
check "ten built-in Domain Packs are listed" "$DP" "10"
"$BIN" knowledge uninstall-pack task-planning --json >/dev/null 2>&1
DPI=$("$BIN" knowledge domain-packs --json 2>/dev/null | J "next(e['installed'] for e in d['packs'] if e['pack']['id']=='task-planning')")
check "Domain Pack uninstall persists in Runtime Truth" "$DPI" "False"
"$BIN" knowledge install-pack task-planning --json >/dev/null 2>&1
DPI=$("$BIN" knowledge domain-packs --json 2>/dev/null | J "next(e['installed'] for e in d['packs'] if e['pack']['id']=='task-planning')")
check "Domain Pack can be restored" "$DPI" "True"
AH=$("$BIN" assets import --text "會議紀錄：決定採用方案B" --json 2>/dev/null | J "d['hash']")
[ -n "$AH" ] && ok "asset imported (CAS) $AH" || bad "asset import"
AH2=$("$BIN" assets import --text "會議紀錄：決定採用方案B" --json 2>/dev/null | J "d['hash']")
check "same content = same hash (write-once)" "$AH2" "$AH"
# agent 提案 → candidate。
KID=$("$BIN" knowledge propose-claim --title "方案B已定案" --content "依會議紀錄" --evidence "[{\"assetHash\":\"$AH\"}]" --as-agent claude-code --json 2>/dev/null | J "d['nodeId']")
KST=$("$BIN" knowledge show "$KID" --json 2>/dev/null | J "d['status']")
check "agent proposal is candidate" "$KST" "candidate"
INBOX=$("$BIN" inbox --status candidate --agent claude-code --json 2>/dev/null | J "d['pendingCount'] >= 1")
check "unified CLI inbox filters candidate by agent" "$INBOX" "True"
# agent approve → 降留言。
"$BIN" knowledge review "$KID" approve --as-agent codex --json >/dev/null 2>&1
KST=$("$BIN" knowledge show "$KID" --json 2>/dev/null | J "d['status']")
check "agent cannot self-approve" "$KST" "candidate"
# 人類 approve → active。
"$BIN" knowledge review "$KID" approve --json >/dev/null 2>&1
KST=$("$BIN" knowledge show "$KID" --json 2>/dev/null | J "d['status']")
check "human approval activates" "$KST" "active"
# 檢索找得到＋誠實 retrieval note。
FOUND=$("$BIN" knowledge search "方案B" --json 2>/dev/null | J "any(r['nodeId']=='$KID' for r in d['results'])")
check "FTS/vector search finds it" "$FOUND" "True"
# 類比不可標因果。
K2=$("$BIN" knowledge propose-claim --title "另一主張" --content "x" --evidence "[{\"url\":\"https://example.com\"}]" --activate --json 2>/dev/null | J "d['nodeId']")
RC=$("$BIN" knowledge link "$KID" "$K2" --relation causes --origin ai-conjecture --json >/dev/null 2>&1; echo $?)
if [ "$RC" != "0" ]; then ok "analogy/conjecture cannot claim causality"; else bad "causal edge should be refused"; fi

echo "== Curator + receipts (v0.4) =="
DEC=$("$BIN" knowledge update-check repo-commit --json 2>/dev/null | J "d['needsAi']")
check "repo-commit needs no AI" "$DEC" "False"
DEC=$("$BIN" knowledge update-check low-confidence-answer --json 2>/dev/null | J "d['requiresUserAsk']")
check "external research requires asking" "$DEC" "True"
NR=$("$BIN" knowledge receipts --json 2>/dev/null | J "d['count'] >= 1")
check "knowledge receipts recorded" "$NR" "True"

echo "== Serial hardware closed loop (SIMULATOR) =="
# 誠實標示：pty 模擬器（與參考韌體同協定），非真機。
STATE=$("$BIN" providers list --json 2>/dev/null | python3 -c "import sys,json;print(next((p['state'] for p in json.load(sys.stdin) if p['identity']['id']=='provider.adapter.esp32-desk'),'MISSING'))")
check "serial adapter provider registered (installed)" "$STATE" "installed"
"$BIN" providers transition provider.adapter.esp32-desk --state available --json >/dev/null 2>&1
# 實體動器的明確授權是三段式（全部人類動作）：enable → 加進 policy
# allowlist → session consent。缺一即 blocked——這正是安全設計。
for CAP in vibe servo buzz; do
  curl -s -X PATCH "http://127.0.0.1:${PORT}/v1/actuators/esp32-desk.${CAP}" -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' -d '{"enabled":true}' >/dev/null
done
POLICY_PATCH=$("$BIN" policy show --json 2>/dev/null | python3 -c "
import sys,json
p=json.load(sys.stdin)
al=p.get('actuatorAllowlist',[]); ch=p.get('allowedChannels',[])
for a in ('esp32-desk.vibe','esp32-desk.servo','esp32-desk.buzz'):
    if a not in al: al.append(a)
for c in ('haptic','motion','sound'):
    if c not in ch: ch.append(c)
print(json.dumps({'actuatorAllowlist':al,'allowedChannels':ch}))")
"$BIN" policy set "$POLICY_PATCH" --json >/dev/null 2>&1
for CAP in vibe servo buzz; do
  "$BIN" session consent "actuator:esp32-desk.${CAP}" --json >/dev/null 2>&1
done
# 受器：真的向模擬裝置要一次 state（配對碼握手 + read）。
LUX=$("$BIN" observe --receptor esp32-desk.env --fresh --json 2>/dev/null | J "d.get('facts',{}).get('lux','MISSING')")
check "serial receptor reads live facts through pairing handshake" "$LUX" "133"
# 動器：magnitude 1.0 → 模擬韌體硬限制 clamp 0.8，收據誠實記 deviceApplied。
PLAN=$(curl -s -X POST "http://127.0.0.1:${PORT}/v1/plans" -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' -d '{"intent":"vibe-test","magnitude":1.0,"durationMs":800,"preferredChannels":["haptic"],"candidates":["esp32-desk.vibe"],"minChannels":1,"maxChannels":1}')
PLAN_ID=$(echo "$PLAN" | J "d['planId']")
EXEC=$(curl -s -X POST "http://127.0.0.1:${PORT}/v1/plans/${PLAN_ID}/execute" -H "Authorization: Bearer $TOKEN")
AID=$(echo "$EXEC" | J "d[0]['actionId'] if isinstance(d,list) else d['receipts'][0]['actionId']")
ST=$("$BIN" actions show "$AID" --json 2>/dev/null | J "d['currentStatus']")
check "serial cmd acked by device (acknowledged, not completed)" "$ST" "acknowledged"
APPLIED=$("$BIN" actions show "$AID" --json 2>/dev/null | J "d['driverResponse']['deviceApplied']['strength']")
check "firmware hard-limit clamp is honestly recorded (1.0 -> 0.8)" "$APPLIED" "0.8"

# --- 韌體硬限制與節流（模擬器鏡射 esp32-companion.ino 的常數）------------
# servo：角度硬限制 10..170（spec 寫死 999）＋每 300ms 只能動一次。
# 兩個 plan 先建好，再用同一個 curl（--next）連續送出兩次 execute——
# 兩次請求間隔遠小於 300ms 節流窗，所以第二次必定被裝置拒絕。
mkplan() {
  curl -s -X POST "http://127.0.0.1:${PORT}/v1/plans" -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
    -d "{\"intent\":\"$1\",\"magnitude\":0.5,\"durationMs\":500,\"preferredChannels\":[\"$2\"],\"candidates\":[\"$3\"],\"minChannels\":1,\"maxChannels\":1}" | J "d['planId']"
}
PLAN_S1=$(mkplan servo-clamp motion esp32-desk.servo)
PLAN_S2=$(mkplan servo-throttle motion esp32-desk.servo)
SERVO_OUT=$(curl -s -X POST "http://127.0.0.1:${PORT}/v1/plans/${PLAN_S1}/execute" -H "Authorization: Bearer $TOKEN" \
  --next -s -X POST "http://127.0.0.1:${PORT}/v1/plans/${PLAN_S2}/execute" -H "Authorization: Bearer $TOKEN")
read -r SERVO_A1 SERVO_A2 <<EOF
$(printf '%s' "$SERVO_OUT" | python3 -c "
import sys, json
raw = sys.stdin.read(); dec = json.JSONDecoder(); ids = []; i = 0
while i < len(raw):
    while i < len(raw) and raw[i] in ' \t\r\n': i += 1
    if i >= len(raw): break
    obj, i = dec.raw_decode(raw, i)
    receipts = obj if isinstance(obj, list) else obj.get('receipts', [])
    ids.append(receipts[0]['actionId'] if receipts else 'MISSING')
print(' '.join(ids[:2]) if len(ids) >= 2 else 'MISSING MISSING')")
EOF
SERVO_ANGLE=$("$BIN" actions show "$SERVO_A1" --json 2>/dev/null | J "d['driverResponse']['deviceApplied']['angle']")
check "servo angle clamped by the device (999 -> 170)" "$SERVO_ANGLE" "170"
THROTTLED=$("$BIN" actions show "$SERVO_A2" --json 2>/dev/null | python3 -c "
import sys,json
d=json.load(sys.stdin)
print(d['currentStatus']=='failed' and 'rate-limited' in json.dumps(d))" 2>/dev/null)
check "device rate-limit is an honest failed receipt (no fake success)" "$THROTTLED" "True"
# buzzer：freqHz 200..4000、durationMs ≤ 2000（spec 寫死 99999 / 9999）。
PLAN_B=$(mkplan buzz-clamp sound esp32-desk.buzz)
BUZZ_EXEC=$(curl -s -X POST "http://127.0.0.1:${PORT}/v1/plans/${PLAN_B}/execute" -H "Authorization: Bearer $TOKEN")
BUZZ_AID=$(echo "$BUZZ_EXEC" | J "d[0]['actionId'] if isinstance(d,list) else d['receipts'][0]['actionId']")
BUZZ_APPLIED=$("$BIN" actions show "$BUZZ_AID" --json 2>/dev/null | J "'%s/%s' % (d['driverResponse']['deviceApplied']['freqHz'], d['driverResponse']['deviceApplied']['durationMs'])")
check "buzzer clamped by the device (99999Hz/9999ms -> 4000/2000)" "$BUZZ_APPLIED" "4000/2000"
# 受器：servo 的實際角度也出現在觀察面（動作 ack ≠ 觀察，兩者分開驗）。
OBS_ANGLE=$("$BIN" observe --receptor esp32-desk.env --fresh --json 2>/dev/null | J "d.get('facts',{}).get('servoAngle','MISSING')")
check "device state reflects the clamped servo angle" "$OBS_ANGLE" "170"

# --- 感測面（模擬器控制通道；韌體上是真實感測器，真機驗收仍為零）-----------
# 距離改變 → observe 反映新值（感測值不是常數；ack ≠ 觀察）。
echo '{"distanceMm": 150}' > "$SIM_FACTS"
sleep 0.5
DIST=$("$BIN" observe --receptor esp32-desk.env --fresh --json 2>/dev/null | J "d.get('facts',{}).get('distanceMm','MISSING')")
check "sensor change on the device is reflected by observe (842 -> 150)" "$DIST" "150"
# 感測器缺席：韌體對讀不到的 HC-SR04／DHT22 回 -1／null——必須原樣穿透到
# Observation facts（tempC 不被當成數字、也不被吞掉）。
echo '{"distanceMm": -1, "tempC": null}' > "$SIM_FACTS"
sleep 0.5
ABSENT=$("$BIN" observe --receptor esp32-desk.env --fresh --json 2>/dev/null | J "'tempC' in d.get('facts',{}) and d['facts']['tempC'] is None and d['facts'].get('distanceMm')==-1")
check "absent sensors pass through as tempC=null / distanceMm=-1 (not a number, not dropped)" "$ABSENT" "True"
# 按鈕邊緣 → 裝置「主動」推播 state（線上沒有 read 就送出），observe 隨後反映 button=true。
# host 端目前只在 read 時消費 state（沒有推播快取），所以「未請求的推播」在
# 線層（sim.log）驗：SIGUSR1 之後、host 送 read 之前，已出現 button:true 的 state。
LOG_MARK=$(wc -l < "$HOME_DIR/sim.log" | tr -d ' ')
kill -USR1 "$SIM_PID"
sleep 0.5
PUSHED=$(python3 - "$HOME_DIR/sim.log" "$LOG_MARK" <<'PY'
import sys, json
lines = open(sys.argv[1], encoding="utf-8").read().splitlines()[int(sys.argv[2]):]
result = "no-state"
for line in lines:
    if line.startswith(">>") and '"read"' in line:
        result = "read-before-push"; break
    if line.startswith("<< "):
        try: msg = json.loads(line[3:])
        except Exception: continue
        if msg.get("type") == "state":
            result = "pushed" if msg.get("facts", {}).get("button") is True else "state-without-button"
            break
print(result)
PY
)
check "button edge pushes state unsolicited (no read on the wire)" "$PUSHED" "pushed"
BTN=$("$BIN" observe --receptor esp32-desk.env --fresh --json 2>/dev/null | J "d.get('facts',{}).get('button','MISSING')")
check "observe reflects the toggled button" "$BTN" "True"

echo "== Character Protocol =="
# 外部 adapter 的 WebSocket transport：以 examples/character-adapters/text-adapter.mjs
# 當「模擬 adapter（fixture）」——程序內／本機 node 程式，不是真外部裝置。
# token 只印一次、只存 sha256；撤銷即斷線。
CP_VER=$("$BIN" character status --json 2>/dev/null | J "d['version']")
check "character protocol version reported" "$CP_VER" "1.0"
NODE_MAJOR=$(node -p 'Number(process.versions.node.split(".")[0])' 2>/dev/null || echo 0)
if [ "${NODE_MAJOR:-0}" -lt 22 ]; then
  echo "  SKIP: Character Protocol WS fixture needs node >= 22 (global WebSocket); found node major '${NODE_MAJOR}'"
else
  ADD=$("$BIN" character adapters add --name "文字 adapter（fixture）" --manifest "$ROOT/examples/character-adapters/text-adapter.manifest.json" --json 2>/dev/null)
  ADAPTER_ID=$(echo "$ADD" | J "d.get('adapterId','')")
  ADAPTER_TOKEN=$(echo "$ADD" | J "d.get('token','')")
  if [ -n "$ADAPTER_ID" ] && [ "${#ADAPTER_TOKEN}" = "64" ]; then ok "adapter registered; token issued once (64 hex)"; else bad "adapter registration ($ADD)"; fi
  LISTED=$("$BIN" character adapters list --json 2>/dev/null | J "'token' not in json.dumps(d) and d['adapters'][0]['revoked']==False")
  check "adapter list never contains the token" "$LISTED" "True"
  # 模擬 adapter（fixture）：收到第一個 intent、回 completed 後自行結束。
  INTERACT_AI_API="http://127.0.0.1:${PORT}" INTERACT_AI_CHARACTER_TOKEN="$ADAPTER_TOKEN" \
    CHARACTER_FIXTURE_ONCE=1 CHARACTER_FIXTURE_QUIET=1 \
    node "$ROOT/examples/character-adapters/text-adapter.mjs" > "$HOME_DIR/fixture.log" 2>&1 &
  FIX_PID=$!
  CONN=False
  for i in $(seq 1 40); do
    CONN=$("$BIN" character instances --json 2>/dev/null | J "any(i.get('connected') and i.get('negotiated') and i.get('origin')=='external' for i in d['instances'])" 2>/dev/null || echo False)
    [ "$CONN" = "True" ] && break; sleep 0.25
  done
  check "模擬 adapter（fixture）negotiated over /v1/character/ws" "$CONN" "True"
  # human token 不能上 adapter 的 WebSocket（401；只收 adapter token）。
  WS_RC=$(curl -s -o /dev/null -w "%{http_code}" -H "Connection: Upgrade" -H "Upgrade: websocket" -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" "http://127.0.0.1:${PORT}/v1/character/ws?token=${TOKEN}")
  check "human token refused on /v1/character/ws" "$WS_RC" "401"
  # adapter token 打人類路由 → 403。
  AD_RC=$(curl -s -o /dev/null -w "%{http_code}" "http://127.0.0.1:${PORT}/v1/status" -H "Authorization: Bearer $ADAPTER_TOKEN")
  check "adapter token cannot read /v1/status" "$AD_RC" "403"
  AD_RC=$(curl -s -o /dev/null -w "%{http_code}" -X POST "http://127.0.0.1:${PORT}/v1/emergency-stop" -H "Authorization: Bearer $ADAPTER_TOKEN")
  check "adapter token cannot trigger emergency stop" "$AD_RC" "403"
  # 人類手動 intent（非安全）→ fixture 印一行文字並回 accepted→started→completed。
  INTENT=$("$BIN" character intent notice --message "CLI E2E" --json 2>/dev/null)
  MID=$(echo "$INTENT" | J "d['messageId']")
  TGT=$(echo "$INTENT" | J "len(d['targets'])")
  check "manual notice intent reached the fixture instance" "$TGT" "1"
  RC=$("$BIN" character intent emergency --json >/dev/null 2>&1; echo $?)
  if [ "$RC" != "0" ]; then ok "manual safety intent refused (runtime-only)"; else bad "emergency must not be manually playable"; fi
  DONE=False
  for i in $(seq 1 20); do
    DONE=$("$BIN" events --seconds 1 --json 2>/dev/null | python3 -c "
import sys,json
mid='$MID'; hit=False
for line in sys.stdin:
    line=line.strip()
    if not line: continue
    try: e=json.loads(line)
    except Exception: continue
    if e.get('eventType')=='character.receipt' and e.get('payload',{}).get('receipt',{}).get('messageId')==mid and e['payload']['receipt'].get('status')=='completed': hit=True
print(hit)")
    [ "$DONE" = "True" ] && break; sleep 0.25
  done
  check "character.receipt completed from the fixture (text printed, not verified)" "$DONE" "True"
  if grep -q "\[intent\] notice" "$HOME_DIR/fixture.log" 2>/dev/null; then ok "fixture printed the intent line"; else bad "fixture log lacks the intent line"; fi
  # 撤銷 → token 失效、instance 移除、adapters 標 revoked。
  "$BIN" character adapters revoke "$ADAPTER_ID" --json >/dev/null 2>&1
  REV=$("$BIN" character adapters list --json 2>/dev/null | J "d['adapters'][0]['revoked'] and not d['adapters'][0]['connected']")
  check "revoke marks the adapter revoked and disconnected" "$REV" "True"
  GONE=$("$BIN" character instances --json 2>/dev/null | J "all(i.get('origin')!='external' for i in d['instances'])")
  check "revoked adapter instance no longer listed" "$GONE" "True"
  AD_RC=$(curl -s -o /dev/null -w "%{http_code}" -X POST "http://127.0.0.1:${PORT}/v1/character/receipts" -H "Authorization: Bearer $ADAPTER_TOKEN" -H 'content-type: application/json' -d '{"instanceId":"x","receipt":{}}')
  check "revoked adapter token is unauthorized" "$AD_RC" "401"
  kill "$FIX_PID" 2>/dev/null || true
  wait "$FIX_PID" 2>/dev/null || true
fi

echo "== Emergency stop propagation =="
SID2=$("$BIN" agents create --agent agent.b --ttl 30 --json 2>/dev/null | J "d['sessionId']")
"$BIN" emergency-stop --json >/dev/null 2>&1
ST=$("$BIN" agents show "$SID2" --json 2>/dev/null | J "d['state']")
check "estop cancels open agent session" "$ST" "cancelled"
RC=$("$BIN" agents create --agent agent.c --json >/dev/null 2>&1; echo $?)
if [ "$RC" != "0" ]; then ok "estop blocks new agent sessions"; else bad "new session should be blocked under estop"; fi
# estop 也要打到硬體：serial 模擬器要收到 stop-all。
sleep 1
if grep -qE '"type": ?"stop-all"|"stopAll"' "$HOME_DIR/sim.log" 2>/dev/null; then
  ok "estop propagated stop-all to the serial device (simulator)"
else
  bad "serial device did not receive stop-all on estop"
fi

echo
echo "RESULT: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
