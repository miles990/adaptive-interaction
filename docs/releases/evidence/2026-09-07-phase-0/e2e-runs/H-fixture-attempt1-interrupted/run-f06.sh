#!/usr/bin/env bash
# E0-08-fixture-core-offline-rerun：核心離線→重啟→同一個 fake_iphone process
# 用同 token 重連（fixture 修好硬退出之後）。全部隔離在自己的 INTERACT_AI_HOME。
set -u
OUT=/private/tmp/claude-501/-Users-user-Workspace-claude-lab-adaptive-interaction/95db6375-b13e-4494-9180-4b15da263657/scratchpad/e2e2/runs/H-fixture
REPO=/Users/user/Workspace/claude-lab/adaptive-interaction
BIN="$REPO/target/debug/interact-ai"
FAKE="$REPO/target/debug/examples/fake_iphone"
HOMEDIR="$OUT/home"
LOGS="$OUT/logs"
PORT=19100
mkdir -p "$HOMEDIR" "$LOGS"
unset INTERACT_AI_CLAUDE_BIN INTERACT_AI_CODEX_BIN || true
export INTERACT_AI_HOME="$HOMEDIR"
export INTERACT_AI_MOBILE_ADVERTISE=0

ts() { date -u +%FT%TZ; }
say() { echo "[$(ts)] $*" | tee -a "$LOGS/run.log"; }

# --- daemon 啟停 -------------------------------------------------------------
start_daemon() { # $1 = log suffix
  "$BIN" serve --port "$PORT" > "$LOGS/daemon-$1.log" 2>&1 &
  DAEMON_PID=$!
  echo "$DAEMON_PID" > "$LOGS/daemon-$1.pid"
  say "daemon($1) pid=$DAEMON_PID"
  for _ in $(seq 1 60); do
    if [ -f "$HOMEDIR/state/api-token" ]; then
      TOKEN=$(cat "$HOMEDIR/state/api-token")
      code=$(curl -s -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $TOKEN" \
        "http://127.0.0.1:$PORT/v1/status" || true)
      [ "$code" = "200" ] && { say "daemon($1) ready http=200"; return 0; }
    fi
    sleep 0.3
  done
  say "daemon($1) NOT ready"; return 1
}

api() { # $1 method $2 path $3 outfile
  curl -s -X "$1" -H "Authorization: Bearer $TOKEN" \
    "http://127.0.0.1:$PORT$2" -o "$3" -w '%{http_code}'
}

wait_line() { # $1 file $2 grep-pattern $3 timeout-sec [$4 want-count，預設 1]
  local f="$1" pat="$2" limit="$3" want="${4:-1}" i=0 n=0
  while [ "$i" -lt "$((limit * 10))" ]; do
    n=$(grep -c -- "$pat" "$f" 2>/dev/null || echo 0)
    [ "$n" -ge "$want" ] && return 0
    i=$((i + 1)); sleep 0.1
  done
  return 1
}

alive() { ps -p "$1" > /dev/null 2>&1 && echo alive || echo dead; }

phone_send() { printf '%s\n' "$1" >&3; printf '%s\n' "$1" >> "$LOGS/phone.cmds.txt"; say "-> phone: $1"; }

say "=== PHASE A: 起 daemon、配對、連線 ==="
start_daemon 1 || exit 1
api GET /v1/mobile/status "$LOGS/mobile-status-0.json" > "$LOGS/http-0.code"
api POST /v1/mobile/pairing-session "$LOGS/pairing.json" > "$LOGS/http-pair.code"
CODE=$(python3 -c "import json;print(json.load(open('$LOGS/pairing.json'))['code'])")
api GET /v1/mobile/status "$LOGS/mobile-status-1.json" > "$LOGS/http-1.code"
MPORT=$(python3 -c "import json;print(json.load(open('$LOGS/mobile-status-1.json'))['port'])")
FP=$(python3 -c "import json;print(json.load(open('$LOGS/mobile-status-1.json'))['fingerprint'])")
say "mobile port=$MPORT fingerprint=${FP:0:16}… code=${#CODE}chars"

FIFO="$LOGS/phone.in"
rm -f "$FIFO"; mkfifo "$FIFO"
"$FAKE" --port "$MPORT" --fingerprint "$FP" --code "$CODE" \
  < "$FIFO" > "$LOGS/phone.log" 2> "$LOGS/phone.stderr.log" &
PHONE_PID=$!
echo "$PHONE_PID" > "$LOGS/phone.pid"
exec 3> "$FIFO"
say "fake_iphone pid=$PHONE_PID"
wait_line "$LOGS/phone.log" '"deviceId"' 20 || { say "FAIL: 沒拿到 deviceId"; }
DEVICE_ID=$(python3 -c "
import json
for line in open('$LOGS/phone.log'):
    d=json.loads(line)
    if 'deviceId' in d and 'deviceToken' in d: print(d['deviceId']); break
")
say "deviceId=$DEVICE_ID"
wait_line "$LOGS/phone.log" '"event":"connected"' 20 || say "FAIL: 沒有 connected"
phone_send '{"op":"status","micLevel":true}'
sleep 1
api GET /v1/mobile/status "$LOGS/mobile-status-2.json" > "$LOGS/http-2.code"
say "A done: $(python3 -c "
import json;d=json.load(open('$LOGS/mobile-status-2.json'))
print([{k:v for k,v in x.items() if k in ('id','connected','name')} for x in d['devices']])")"

say "=== PHASE B: SIGKILL daemon，送 reconnect，期望 reconnect-failed 且 process 還活著 ==="
DPID=$(cat "$LOGS/daemon-1.pid")
kill -KILL "$DPID"; say "kill -KILL $DPID -> $(alive "$DPID")"
sleep 1
say "daemon after kill: $(alive "$DPID")"
curl -s -m 3 -o "$LOGS/http-while-dead.txt" -w 'curl_exit=%{exitcode} http=%{http_code}\n' \
  -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$PORT/v1/mobile/status" \
  > "$LOGS/http-while-dead.code" 2>&1
say "http while dead: $(cat "$LOGS/http-while-dead.code")"
wait_line "$LOGS/phone.log" '"event":"disconnected"' 10 || say "note: 沒看到 disconnected"
BEFORE_RC=$(wc -l < "$LOGS/phone.log")
phone_send '{"op":"reconnect"}'
if wait_line "$LOGS/phone.log" '"event":"reconnect-failed"' 15; then
  say "PASS B1: reconnect-failed 出現"
else
  say "FAIL B1: 沒有 reconnect-failed"
fi
sleep 1
say "PASS/FAIL B2 (process 存活): fake_iphone pid=$PHONE_PID $(alive "$PHONE_PID")"
say "phone.log lines before=$BEFORE_RC after=$(wc -l < "$LOGS/phone.log")"

say "=== PHASE C: 同 home 重啟 daemon，再送 reconnect，期望恢復 ==="
start_daemon 2 || say "FAIL: daemon 重啟失敗"
sleep 1
api GET /v1/mobile/status "$LOGS/mobile-status-3.json" > "$LOGS/http-3.code"
MPORT2=$(python3 -c "import json;print(json.load(open('$LOGS/mobile-status-3.json'))['port'])")
say "restarted mobile port=$MPORT2 (原本 $MPORT)"
phone_send '{"op":"reconnect"}'
# PHASE A 已經有一次 connected，這裡要等「第 2 次」
if wait_line "$LOGS/phone.log" '"event":"connected"' 20 2; then say "PASS C1: fixture 回報 connected（第 2 次）"; else say "FAIL C1"; fi
sleep 1
api GET /v1/mobile/status "$LOGS/mobile-status-4.json" > "$LOGS/http-4.code"
say "C done: $(python3 -c "
import json;d=json.load(open('$LOGS/mobile-status-4.json'))
print([{k:v for k,v in x.items() if k in ('id','connected')} for x in d['devices']])")"
phone_send '{"op":"status","micLevel":true}'
sleep 1
api GET /v1/mobile/status "$LOGS/mobile-status-5.json" > "$LOGS/http-5.code"

say "=== PHASE D: 撤銷裝置後再 reconnect，期望 auth-fail ==="
api DELETE "/v1/mobile/devices/$DEVICE_ID" "$LOGS/revoke.json" > "$LOGS/http-revoke.code"
say "revoke http=$(cat "$LOGS/http-revoke.code") body=$(cat "$LOGS/revoke.json")"
sleep 1
phone_send '{"op":"reconnect"}'
if wait_line "$LOGS/phone.log" '"event":"auth-fail"' 20; then say "PASS D1: auth-fail"; else say "FAIL D1"; fi
sleep 1
say "D2 (process 存活): $(alive "$PHONE_PID")"
api GET /v1/mobile/status "$LOGS/mobile-status-6.json" > "$LOGS/http-6.code"

say "=== PHASE E: 收工 ==="
phone_send '{"op":"quit"}'
sleep 2
exec 3>&-
say "fake_iphone: $(alive "$PHONE_PID")"
if ps -p "$PHONE_PID" > /dev/null 2>&1; then kill "$PHONE_PID" 2>/dev/null; sleep 1; fi
say "fake_iphone final: $(alive "$PHONE_PID")"
DPID2=$(cat "$LOGS/daemon-2.pid")
kill "$DPID2" 2>/dev/null; sleep 2
say "daemon2 final: $(alive "$DPID2")"
rm -f "$FIFO"
say "=== 完成 ==="
