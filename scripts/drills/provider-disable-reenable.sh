#!/usr/bin/env bash
# 演練 4（v0.7.0 候選）：停用 → 重新啟用 → 撤銷不復活，全部對**真的 daemon** 跑。
#
# 這支腳本不改任何程式碼：它把 `docs/aip/device-profile.md` §6.1 與
# `docs/aip/adapter-development.md` §5 描述的生命週期，用 CLI 逐步走一次，
# 並把每一步的可觀測面（ProviderState、detail 文案、稽核事件）印出來。
#
# 誠實邊界：裝置是 `scripts/esp32-serial-sim.py` 的 pty **模擬器**，不是 ESP32
# 真板。身分強度是 `transport-hello+device-side-pairing`（裝置自報明文比對）。
#
# 用法：bash scripts/drills/provider-disable-reenable.sh [port]
set -euo pipefail

PORT="${1:-8871}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CLI="${INTERACT_AI_BIN:-$ROOT/target/debug/interact-ai}"
[ -x "$CLI" ] || { echo "先 build：cargo build -p interaction-cli（找不到 $CLI）" >&2; exit 1; }

WORK="$(mktemp -d)"
HOME_DIR="$WORK/home"
mkdir -p "$HOME_DIR/config/adapters" "$WORK/sim"
SIM_PID=""; DAEMON_PID=""
cleanup() {
  [ -n "$DAEMON_PID" ] && kill "$DAEMON_PID" 2>/dev/null || true
  [ -n "$SIM_PID" ] && kill "$SIM_PID" 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT

# --- 1) pty 模擬器 ----------------------------------------------------------
python3 "$ROOT/scripts/esp32-serial-sim.py" \
  --device-id button-box-01 --pairing-code 9927 \
  --pty-path-file "$WORK/sim/pty" --log "$WORK/sim/sim.log" --no-frag \
  >"$WORK/sim/stdout.log" 2>&1 &
SIM_PID=$!
disown "$SIM_PID" 2>/dev/null || true
for _ in $(seq 1 100); do [ -s "$WORK/sim/pty" ] && break; sleep 0.2; done
PTY="$(cat "$WORK/sim/pty")"
echo "模擬器 pty：$PTY"

# --- 2) 宣告式 spec（就是 examples/ 裡那一份，只換掉埠） --------------------
sed -e "s#/dev/cu.usbmodem-CHANGE-ME#$PTY#" \
  "$ROOT/examples/adapters/event-source-button.yaml" \
  > "$HOME_DIR/config/adapters/button-box.yaml"

# --- 3) 隔離 home 的真 daemon ----------------------------------------------
INTERACT_AI_HOME="$HOME_DIR" INTERACT_AI_MOBILE_ADVERTISE=0 \
  "$CLI" serve --port "$PORT" >"$WORK/daemon.log" 2>&1 &
DAEMON_PID=$!
disown "$DAEMON_PID" 2>/dev/null || true
for _ in $(seq 1 120); do curl -sf -o /dev/null "http://127.0.0.1:$PORT/v1/health" && break; sleep 0.5; done

export INTERACT_AI_HOME="$HOME_DIR" INTERACT_AI_API="http://127.0.0.1:$PORT"
PID=provider.adapter.button-box
state() { "$CLI" --json providers show "$PID" | python3 -c 'import sys,json;print(json.load(sys.stdin)["state"])'; }
audit() {
  "$CLI" --json audit --limit 80 | python3 -c '
import sys, json
for r in json.load(sys.stdin):
    if str(r.get("kind","")).startswith("provider."):
        print("   ", json.dumps({"kind": r["kind"], "detail": r.get("detail")}, ensure_ascii=False)[:220])
'
}

echo
echo "== 0) 綁定：先讓它 available"
"$CLI" --json providers transition "$PID" --state available >/dev/null
for _ in $(seq 1 20); do [ "$(state)" = "available" ] && break; sleep 0.5; done
echo "   state=$(state)"

echo
echo "== 1) 停用（Disabled 是可以重新綁定的原因）"
"$CLI" --json providers transition "$PID" --state disabled >/dev/null
echo "   state=$(state)"

echo
echo "== 2) 重新啟用：ProviderState 不得先於實際連線"
# transition 的回應本身就是「翻旗標當下」的觀測點：握手還沒成功時它必須是
# disconnected＋一句人話，而不是先報 available。
"$CLI" --json providers transition "$PID" --state available | python3 -c '
import sys, json
d = json.load(sys.stdin)
print("   transition 回應：state=%s" % d["state"])
print("   detail：%s" % d.get("detail", "")[:240])
'
for i in $(seq 1 20); do s="$(state)"; echo "   t+${i}：state=${s}"; [ "${s}" = "available" ] && break; sleep 1; done

echo
echo "== 3) 稽核（provider.rebinding → provider.rebound）"
audit

echo
echo "== 4) 撤銷（sticky）"
"$CLI" --json providers revoke "$PID" >/dev/null
sleep 1
echo "   state=$(state)"

echo
echo "== 5) 撤銷之後再 available：不得復活"
set +e
"$CLI" --json providers transition "$PID" --state available
set -e
sleep 2
echo "   state=$(state)   ← 必須仍然是 revoked"

echo
echo "== 6) 撤銷後的稽核"
audit
