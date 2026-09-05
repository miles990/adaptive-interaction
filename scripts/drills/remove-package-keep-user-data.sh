#!/usr/bin/env bash
# 演練 5（v0.7.0 候選）：移除一個第三方角色套件，使用者資料必須留著。
#
# 這支腳本不改任何程式碼。它做四件事：
#   1. 把一個第三方角色放進 `<home>/state/characters/<id>/`
#      （就是 `apps/interaction-desktop/src-tauri/src/character_store.rs` 的版型）；
#   2. 讓使用者資料指向它（後端 prefs 的 customNames ＋ 桌面 `state/desktop.json`）；
#   3. 移除套件——`character_store::remove` 做的就是對那個資料夾 `remove_dir_all`；
#   4. 重啟 daemon，逐項核對使用者資料還在、session 快照是**還原**而不是重建。
#
# 誠實邊界：Tauri 桌面沒有在這裡跑，所以 `state/desktop.json` 是照
# `apps/interaction-desktop/src/desktop.ts` 的 `DesktopPrefs` 形狀手寫的，
# 角色頁的降級由既有的桌面單元測試涵蓋（`companion-imported-characters.test.ts`
# 的「找不到／壞掉／不在白名單 → 文字角色＋原因」），不是這支腳本驗的。
#
# 用法：bash scripts/drills/remove-package-keep-user-data.sh [port]
set -euo pipefail

PORT="${1:-8875}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CLI="${INTERACT_AI_BIN:-$ROOT/target/debug/interact-ai}"
[ -x "$CLI" ] || { echo "先 build：cargo build -p interaction-cli（找不到 $CLI）" >&2; exit 1; }

WORK="$(mktemp -d)"
HOME_DIR="$WORK/home"
mkdir -p "$HOME_DIR/state/characters/party-lamp"
DAEMON_PID=""
cleanup() { [ -n "$DAEMON_PID" ] && kill "$DAEMON_PID" 2>/dev/null || true; rm -rf "$WORK"; }
trap cleanup EXIT

start_daemon() {
  INTERACT_AI_HOME="$HOME_DIR" INTERACT_AI_MOBILE_ADVERTISE=0 \
    "$CLI" serve --port "$PORT" >>"$WORK/daemon.log" 2>&1 &
  DAEMON_PID=$!
  disown "$DAEMON_PID" 2>/dev/null || true
  for _ in $(seq 1 120); do curl -sf -o /dev/null "http://127.0.0.1:$PORT/v1/health" && return 0; sleep 0.5; done
  echo "daemon did not become healthy" >&2; exit 1
}
stop_daemon() {
  kill "$DAEMON_PID" 2>/dev/null || true
  for _ in $(seq 1 60); do kill -0 "$DAEMON_PID" 2>/dev/null || break; sleep 0.25; done
  DAEMON_PID=""
}

export INTERACT_AI_HOME="$HOME_DIR" INTERACT_AI_API="http://127.0.0.1:$PORT"
start_daemon

echo "== 1) 匯入一個第三方角色（state/characters/party-lamp/manifest.json）"
python3 - "$ROOT" "$HOME_DIR" <<'PY'
import collections, json, os, sys
root, home = sys.argv[1], sys.argv[2]
src = json.load(
    open(os.path.join(root, "apps/interaction-desktop/public/characters/ref-shape/manifest.json")),
    object_pairs_hook=collections.OrderedDict,
)
src["characterId"] = "party-lamp"
src["author"] = "third-party.example"
src["displayName"] = {"zh-TW": "派對燈", "en": "Party Lamp"}
src["description"] = {"zh-TW": "第三方匯入角色（演練用）。", "en": "A third-party imported character (drill)."}
path = os.path.join(home, "state/characters/party-lamp/manifest.json")
json.dump(src, open(path, "w"), ensure_ascii=False, indent=2)
print("   寫入", path)
PY

echo
echo "== 2) 使用者資料指向它"
"$CLI" --json prefs set '{"customNames":{"party-lamp":"我的派對燈"}}' \
  | python3 -c 'import sys,json;print("   prefs.customNames =",json.dumps(json.load(sys.stdin)["customNames"],ensure_ascii=False))'
cat > "$HOME_DIR/state/desktop.json" <<'JSON'
{
  "schemaVersion": 1,
  "companionPack": "party-lamp",
  "companionName": "我的派對燈",
  "companionVisible": true,
  "companionScene": "none",
  "storyProgress": {"party-lamp-intro": true},
  "companionPreferences": {"party-lamp": {"glow": 3}},
  "companionInteractionMemory": {"schemaVersion": 1, "items": [{"at": 1757000000000, "kind": "pat"}]}
}
JSON
echo "   state/desktop.json companionPack=party-lamp"

# 也讓稽核裡有一筆跟這個角色有關的紀錄（外部 adapter token 註冊）——移除套件之後
# 稽核不得跟著消失：使用者要查得到「這個角色曾經被授權過什麼」。
# （外部 adapter 只收 external-process／remote-device／web；in-process 的套件
#  manifest 會被正確拒絕，所以這裡另外寫一份 remote-device 的。）
cat > "$WORK/party-lamp-remote.json" <<'JSON'
{
  "schemaVersion": "1.0",
  "characterId": "party-lamp-remote",
  "displayName": {"zh-TW": "派對燈（外接）", "en": "Party Lamp (remote)"},
  "version": "0.1.0",
  "adapterKind": "remote-device",
  "entrypoint": {"kind": "url", "url": "ws://127.0.0.1:9999"},
  "capabilities": {"visual.presence": {"supported": true}},
  "securityRequirements": {"network": true, "executable": false, "fileAccess": "none",
                           "audioOutput": false, "microphone": false, "camera": false}
}
JSON
if "$CLI" --json character adapters add --name party-lamp-remote \
     --manifest "$WORK/party-lamp-remote.json" >/dev/null 2>&1; then
  echo "   已註冊一個外部 adapter token（產生稽核）"
else
  echo "   （外部 adapter 註冊失敗，稽核以其他事件為準）"
fi

fingerprint() {
  for f in state/desktop.json state/character-session.json; do
    printf '   %-32s sha256=%s  %s bytes\n' "$f" \
      "$(shasum -a 256 "$HOME_DIR/$f" | cut -c1-16)" "$(wc -c < "$HOME_DIR/$f" | tr -d ' ')"
  done
  printf '   %-32s %s\n' "prefs.customNames" \
    "$("$CLI" --json prefs show | python3 -c 'import sys,json;print(json.dumps(json.load(sys.stdin)["customNames"],ensure_ascii=False))')"
  printf '   %-32s %s\n' "audit rows" \
    "$("$CLI" --json audit --limit 200 | python3 -c 'import sys,json;print(len(json.load(sys.stdin)))')"
  printf '   %-32s %s\n' "character-session" \
    "$(python3 -c 'import json,os;d=json.load(open(os.environ["INTERACT_AI_HOME"]+"/state/character-session.json"));print("epoch=%s revision=%s sessionId=%s"%(d["epoch"],d["revision"],d["sessionId"]))')"
}

echo
echo "== 3) 移除前"
fingerprint

echo
echo "== 4) 移除套件（character_store::remove ＝ remove_dir_all 那個資料夾）"
rm -rf "$HOME_DIR/state/characters/party-lamp"
echo "   state/characters 現在有：$(ls -A "$HOME_DIR/state/characters" | tr '\n' ' ')(空)"

echo
echo "== 5) 重啟 daemon"
stop_daemon
start_daemon

echo
echo "== 6) 移除＋重啟之後"
fingerprint
echo
echo "   隔離／遷移檔："
ls -A "$HOME_DIR/state" | grep -E 'corrupt|pre-format' \
  && echo "   ↑ 有隔離或遷移備份" \
  || echo "   沒有 .corrupt、沒有 .pre-format-*：快照是還原，不是重建"

echo
echo "== 7) daemon 側的角色狀態（套件不在了就誠實回報，不假裝角色還在）"
"$CLI" --json character status | python3 -c 'import sys,json;print("   ",json.dumps(json.load(sys.stdin),ensure_ascii=False))'
