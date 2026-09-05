#!/usr/bin/env bash
# 真 Tauri 視窗走查（可重跑、非互動）。
#
# 為什麼需要它：一般模式的必測任務裡有一整類（換角色、陪伴預設、勿擾、顯示／隱藏
# 桌面角色）**住在 Tauri host**，瀏覽器模式的 Playwright 只驗得到誠實降級那一面。
# 那些任務要嘛有人手動點一次然後把結果抄進文件（下一次改動之後沒有人知道還準不準），
# 要嘛就寫成這支腳本：實際 build 出 .app、用隔離的家啟動、用 macOS 輔助使用（AX）
# 驅動真的視窗，每一步都再用 HTTP／偏好檔讀回**權威狀態**——畫面說了不算。
#
# 誠實邊界（輸出的 JSON 與文件都必須照抄）：
#   * 證據等級＝「真 Tauri 視窗（debug build，AX 驅動，fixture agent，隔離 home）」。
#   * AI 幫手是 fixture 子程序（`crates/interaction-runtime/tests/fixtures/fake_*.sh`），
#     不是真的 Codex／Claude Code。
#   * 沒有任何 iPhone 真機參與；這支腳本完全不碰手機配對。
#   * 每一步的結果只會是 completed／failed／needs-environment 其中之一。
#     沒跑到就是 not-run，不得寫成通過。
#   * 沒有輔助使用權限（系統設定 → 隱私權與安全性 → 輔助使用）時，AX 那幾步一律
#     needs-environment——不是「通過」，也不是「失敗」。
#
# 安全：
#   * 只對這支腳本自己啟動的那個 App 程序操作，不點任何其他應用程式。
#   * `INTERACT_AI_HOME` 指向 mktemp 出來的隔離目錄；`INTERACT_AI_MOBILE_ADVERTISE=0`
#     （行動伺服器不對區網廣播）；API 綁 127.0.0.1 的自選埠號，不碰使用者的 8787。
#   * 截圖只截自己視窗的矩形（`screencapture -R`），輸出到 `--out` 指定的目錄，
#     預設在 repo 之外，不入版控。
#   * 結束一定殺掉 App 程序並確認埠號關閉。
#
# 用法：
#   scripts/tauri-ax-walkthrough.sh [--app <path/to/.app>] [--build] [--port N] [--out <dir>]
#
#   --app     用現成的 .app（預設找 src-tauri/target/debug/bundle/macos/…，沒有就 build）
#   --build   強制重新 `pnpm tauri build --debug --bundles app`
#   --port    隔離 daemon 的 API 埠號（預設 18922）
#   --out     結果 JSON 與截圖的輸出目錄（預設 ./tauri-ax-walkthrough）
#
# 隔離的家一律保留（路徑印在結果 JSON 的 `isolatedHome`）：它是這一次執行的實際狀態，
# 出事時要看得到。它在 /tmp 底下，由系統自行清理，不進 repo。

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DESKTOP_DIR="$REPO_ROOT/apps/interaction-desktop"
DEFAULT_APP="$DESKTOP_DIR/src-tauri/target/debug/bundle/macos/interaction-control-center.app"

APP_PATH=""
FORCE_BUILD=0
PORT=18922
OUT_DIR="$PWD/tauri-ax-walkthrough"

while [ $# -gt 0 ]; do
  case "$1" in
    --app) APP_PATH="${2:-}"; shift 2 ;;
    --build) FORCE_BUILD=1; shift ;;
    --port) PORT="${2:-}"; shift 2 ;;
    --out) OUT_DIR="${2:-}"; shift 2 ;;
    -h|--help) sed -n '1,40p' "$0"; exit 0 ;;
    *) echo "未知參數：$1" >&2; exit 2 ;;
  esac
done

if [ "$(uname -s)" != "Darwin" ]; then
  echo "這支腳本只在 macOS 上有意義（System Events／AX）。" >&2
  exit 2
fi

mkdir -p "$OUT_DIR"
STEPS_FILE="$OUT_DIR/steps.jsonl"
RESULT_FILE="$OUT_DIR/tauri-ax-walkthrough.json"
LOG_FILE="$OUT_DIR/walkthrough.log"
: >"$STEPS_FILE"
: >"$LOG_FILE"

RUN_STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
BUILD_SECONDS="null"
APP_PID=""
PROC_NAME=""
AX_OK=0
HOME_DIR=""
TOKEN=""
API="http://127.0.0.1:$PORT"

log() { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" | tee -a "$LOG_FILE"; }

# --- 結果紀錄 ---------------------------------------------------------------
# 每一步一行 JSON：id／title／status／evidence／detail。status 只能是
# completed｜failed｜needs-environment（not-run 由組裝時補）。
record() {
  local id="$1" title="$2" status="$3" evidence="$4" shot="${5:-}"
  python3 - "$STEPS_FILE" "$id" "$title" "$status" "$evidence" "$shot" <<'PY'
import json, sys
path, sid, title, status, evidence, shot = sys.argv[1:7]
row = {"id": sid, "title": title, "status": status, "evidence": evidence}
if shot:
    row["screenshot"] = shot
with open(path, "a", encoding="utf-8") as f:
    f.write(json.dumps(row, ensure_ascii=False) + "\n")
PY
  log "[$status] $id — $title :: $evidence"
}

# --- HTTP（權威狀態；畫面說了不算） ----------------------------------------
hget() {
  curl -sS --max-time 10 -H "Authorization: Bearer $TOKEN" "$API$1" 2>>"$LOG_FILE"
}
jq_path() { # jq_path <json> <python-expression on `d`>
  python3 -c '
import json,sys
try:
    d = json.loads(sys.argv[1] or "null")
except Exception:
    d = None
try:
    print(eval(sys.argv[2]))
except Exception:
    print("")
' "$1" "$2"
}
prefs_get() { # prefs_get <camelCaseKey>
  python3 - "$HOME_DIR/state/desktop.json" "$1" <<'PY'
import json, sys
try:
    with open(sys.argv[1], encoding="utf-8") as f:
        print(json.load(f).get(sys.argv[2], ""))
except Exception:
    print("")
PY
}

# --- AppleScript 驅動 -------------------------------------------------------
# 一個 osascript 檔負責所有 AX 操作：走訪 WebView 的 AX 樹靠 `entire contents`
# 一次抓平，再在 AppleScript 本地比對名字（每個元素都往回問一次會慢到不能用）。
AX_SCRIPT="$OUT_DIR/ax.applescript"
cat >"$AX_SCRIPT" <<'APPLESCRIPT'
-- AX 驅動工具。argv：<procName> <command> [args…]
--
-- 兩個踩過的坑，改動時不要退回去：
--   1. 主視窗一律用**名字**找（"Interaction Control Center"），不能用 window 1：
--      隱藏／顯示桌面角色之後視窗順序會變，window 1 可能是角色視窗，後面每一步
--      就會安靜地對錯的視窗操作（而且看起來只是「找不到按鈕」）。
--   2. `entire contents of X` 一定要先 `set flat to …` 存成變數再 repeat。直接
--      `repeat with e in (entire contents of X)` 在 System Events 上會拿到空清單。
--
-- 指令：
--   windows                          列出視窗標題（每行一個）
--   click <role> <text>              點第一個 role 且名字含 text 的元素（role=AXAny 表示不限）
--   exists <role> <text>             回 yes/no
--   value <role> <text>              回該元素的 value（aria-pressed 的按鈕在 macOS 上是 AXCheckBox）
--   navclick <index>                 點「主要導覽」的第 index 顆按鈕（1-based）
--   tray <text>                      點狀態列選單中名字含 text 的項目
--   bounds                           回 x,y,w,h（主視窗）
--   resize <w> <h>                   設定主視窗大小
--   hscroll                          回 yes/no：主視窗裡有沒有水平捲軸
on labelOf(e)
	tell application "System Events"
		set lbl to ""
		try
			set lbl to (name of e) as text
		end try
		if lbl is "missing value" or lbl is "" then
			try
				set lbl to (description of e) as text
			end try
		end if
		if lbl is "missing value" then set lbl to ""
		return lbl
	end tell
end labelOf

on run argv
	set procName to item 1 of argv
	set cmd to item 2 of argv
	tell application "System Events"
		if not (exists process procName) then error "process not running: " & procName
		tell process procName
			if cmd is "windows" then
				set out to ""
				repeat with w in windows
					set out to out & (name of w) & linefeed
				end repeat
				return out
			end if
			if cmd is "tray" then
				set wanted to item 3 of argv
				-- 狀態列：menu bar 2 是這個 App 自己的 status item。
				tell menu bar 2
					click menu bar item 1
					delay 0.6
					tell menu 1 of menu bar item 1
						repeat with mi in menu items
							set t to ""
							try
								set t to name of mi
							end try
							if t contains wanted then
								click mi
								return "clicked:" & t
							end if
						end repeat
						key code 53 -- Escape：不要把選單留在畫面上
					end tell
				end tell
				error "tray item not found: " & wanted
			end if
			-- 主視窗以名字定位（見檔頭第 1 點）。
			set target to missing value
			repeat with w in windows
				if (name of w) is "Interaction Control Center" then set target to w
			end repeat
			if target is missing value then
				if (count of windows) is 0 then error "no window"
				set target to window 1
			end if
			if cmd is "bounds" then
				set p to position of target
				set s to size of target
				return ((item 1 of p) as text) & "," & ((item 2 of p) as text) & "," & ((item 1 of s) as text) & "," & ((item 2 of s) as text)
			end if
			if cmd is "resize" then
				set w to (item 3 of argv) as integer
				set h to (item 4 of argv) as integer
				set size of target to {w, h}
				return "resized"
			end if
			-- 以下都要走 AX 樹（見檔頭第 2 點：先存成變數）。
			set flat to entire contents of target
			if cmd is "hscroll" then
				repeat with e in flat
					try
						if (role of e) is "AXScrollBar" then
							if (value of attribute "AXOrientation" of e) contains "Horizontal" then return "yes"
						end if
					end try
				end repeat
				return "no"
			end if
			if cmd is "navclick" then
				set wantIdx to (item 3 of argv) as integer
				set navIdx to 0
				set i to 0
				repeat with e in flat
					set i to i + 1
					try
						if (description of e) is "主要導覽" then
							set navIdx to i
							exit repeat
						end if
					end try
				end repeat
				if navIdx is 0 then error "nav not found"
				-- 導覽群組後面緊接著就是它的按鈕：往後掃到第一個非按鈕為止。
				set btns to {}
				set j to navIdx + 1
				repeat while j ≤ (count of flat)
					set r to ""
					try
						set r to (role of (item j of flat)) as text
					end try
					if r is not "AXButton" then exit repeat
					set end of btns to (item j of flat)
					set j to j + 1
				end repeat
				if (count of btns) < wantIdx then error "nav has only " & (count of btns) & " items"
				click item wantIdx of btns
				return "clicked nav " & wantIdx & " (" & my labelOf(item wantIdx of btns) & ")"
			end if
			set wantRole to item 3 of argv
			set wantText to item 4 of argv
			set hits to {}
			repeat with e in flat
				try
					set r to (role of e) as text
					if wantRole is "AXAny" or r is wantRole then
						set lbl to my labelOf(e)
						if lbl is not "" and lbl contains wantText then set end of hits to e
					end if
				end try
			end repeat
			if cmd is "exists" then
				if (count of hits) > 0 then
					return "yes"
				else
					return "no"
				end if
			end if
			if (count of hits) < 1 then error "not found (" & wantRole & "/" & wantText & ")"
			set target2 to item 1 of hits
			if cmd is "value" then
				set v to ""
				try
					set v to (value of target2) as text
				end try
				return v
			end if
			click target2
			return "clicked " & my labelOf(target2)
		end tell
	end tell
end run
APPLESCRIPT

ax() { # ax <command> [args…] → stdout；失敗時 stderr 有原因、回非 0
  osascript "$AX_SCRIPT" "$PROC_NAME" "$@" 2>>"$LOG_FILE"
}

# 畫面是非同步長出來的（角色庫要讀十份 manifest、對話框有掛載延遲）。固定 sleep
# 要嘛不夠、要嘛浪費；這個包裝在時限內重試，才不會把「還沒畫出來」誤判成
# 「這顆按鈕不存在」——後者會變成文件裡一筆假的 failed。
ax_click_wait() { # ax_click_wait <role> <text> [seconds]
  local role="$1" text="$2" secs="${3:-20}" deadline
  deadline=$(( $(date +%s) + secs ))
  while :; do
    if ax click "$role" "$text" >/dev/null 2>>"$LOG_FILE"; then return 0; fi
    if [ "$(date +%s)" -ge "$deadline" ]; then return 1; fi
    sleep 1
  done
}

# 視窗清單（排序過）：隱藏／顯示桌面角色之後 System Events 的順序會變，
# 直接比字串會把「回來了」誤判成「沒回來」。
win_set() { ax windows | sed '/^$/d' | sort | tr '\n' '/'; }

shot() { # shot <name> → 只截自己視窗的矩形
  local name="$1" b
  b="$(ax bounds 2>/dev/null)" || return 1
  [ -n "$b" ] || return 1
  local x y w h
  IFS=, read -r x y w h <<<"$b"
  screencapture -x -R "${x},${y},${w},${h}" "$OUT_DIR/$name.png" 2>>"$LOG_FILE" || return 1
  echo "$name.png"
}

cleanup() {
  if [ -n "$APP_PID" ] && kill -0 "$APP_PID" 2>/dev/null; then
    log "結束 App（pid ${APP_PID}）"
    kill "$APP_PID" 2>/dev/null
    for _ in $(seq 1 20); do
      kill -0 "$APP_PID" 2>/dev/null || break
      sleep 0.5
    done
    kill -9 "$APP_PID" 2>/dev/null
  fi
  # 埠號真的關了才算收乾淨。
  local still
  still="$(lsof -nP -iTCP:"$PORT" -sTCP:LISTEN 2>/dev/null | tail -n +2 | wc -l | tr -d ' ')"
  log "收尾：埠號 $PORT 仍在監聽的程序數＝$still"
  assemble "$still"
}

assemble() {
  local listeners="${1:-unknown}"
  python3 - "$STEPS_FILE" "$RESULT_FILE" "$RUN_STARTED_AT" "$BUILD_SECONDS" "$APP_PATH" "$HOME_DIR" "$API" "$listeners" "$AX_OK" <<'PY'
import json, sys, datetime
steps_file, out, started, build_s, app, home, api, listeners, ax_ok = sys.argv[1:10]
# 這一輪應該走到的每一步；沒有出現在 steps.jsonl 裡的就是 not-run（誠實：不是通過）。
PLAN = [
    ("launch", "啟動 .app 並等到 /ready"),
    ("onboarding", "首次設定精靈走到完成並顯示角色"),
    ("switch-character", "更換角色（讀回 packId／companionPack）"),
    ("companion-preset", "陪伴預設「安靜」（讀回偏好＋主動說話模式）"),
    ("pause-proactive", "暫停主動對話並恢復（/v1/pause）"),
    ("do-not-disturb", "勿擾開關（讀回偏好）"),
    ("companion-visibility", "顯示／隱藏桌面角色（視窗清單＋presentation.visible）"),
    ("emergency-stop", "緊急停止與安全解除（二段確認）"),
    ("narrow-390", "視窗縮到 390px 寬並檢查橫向捲動"),
]
rows = []
for line in open(steps_file, encoding="utf-8"):
    line = line.strip()
    if line:
        rows.append(json.loads(line))
by_id = {r["id"]: r for r in rows}
tasks = []
for sid, title in PLAN:
    r = by_id.get(sid)
    if r:
        tasks.append(r)
    else:
        tasks.append({"id": sid, "title": title, "status": "not-run", "evidence": "本輪沒有走到這一步"})
summary = {}
for t in tasks:
    summary[t["status"]] = summary.get(t["status"], 0) + 1
doc = {
    "startedAt": started,
    "finishedAt": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "evidenceLevel": "真 Tauri 視窗（debug build，AX 驅動，fixture agent，隔離 home）",
    "honesty": {
        "agent": "AI 幫手是 fixture 子程序（fake_codex.sh／fake_claude.sh），不是真的 Codex／Claude Code",
        "phone": "這一輪完全沒有手機參與（真機與模擬手機都沒有）",
        "statuses": "completed／failed／needs-environment／not-run；needs-environment 不等於通過",
    },
    "app": app,
    "appBuildSeconds": None if build_s in ("null", "") else float(build_s),
    "isolatedHome": home,
    "api": api,
    "accessibilityPermission": "granted" if ax_ok == "1" else "denied-or-unknown",
    "portListenersAfterCleanup": listeners,
    "summary": summary,
    "tasks": tasks,
}
with open(out, "w", encoding="utf-8") as f:
    json.dump(doc, f, ensure_ascii=False, indent=2)
    f.write("\n")
print(json.dumps(summary, ensure_ascii=False))
print(out)
PY
}
trap cleanup EXIT

# --- 1. .app -----------------------------------------------------------------
if [ -z "$APP_PATH" ]; then APP_PATH="$DEFAULT_APP"; fi
if [ "$FORCE_BUILD" = "1" ] || [ ! -d "$APP_PATH" ]; then
  log "build：pnpm tauri build --debug --bundles app"
  build_start=$(date +%s)
  ( cd "$DESKTOP_DIR" && CARGO_INCREMENTAL=0 pnpm tauri build --debug --bundles app ) >>"$LOG_FILE" 2>&1
  build_rc=$?
  BUILD_SECONDS=$(( $(date +%s) - build_start ))
  if [ $build_rc -ne 0 ] || [ ! -d "$DEFAULT_APP" ]; then
    record "launch" "啟動 .app 並等到 /ready" "needs-environment" "pnpm tauri build --debug --bundles app 失敗（見 walkthrough.log），沒有 .app 可以走查"
    exit 1
  fi
  APP_PATH="$DEFAULT_APP"
  log "build 完成，耗時 ${BUILD_SECONDS}s"
fi

BIN="$APP_PATH/Contents/MacOS/interaction-desktop"
if [ ! -x "$BIN" ]; then
  BIN="$(ls "$APP_PATH/Contents/MacOS/" 2>/dev/null | head -1)"
  BIN="$APP_PATH/Contents/MacOS/$BIN"
fi
if [ ! -x "$BIN" ]; then
  record "launch" "啟動 .app 並等到 /ready" "failed" ".app 裡找不到可執行檔：$APP_PATH"
  exit 1
fi

# --- 2. 隔離的家 -------------------------------------------------------------
HOME_DIR="$(mktemp -d /tmp/interaction-ax-home.XXXXXX)"
mkdir -p "$HOME_DIR/config"
printf 'apiHost: 127.0.0.1\napiPort: %s\n' "$PORT" >"$HOME_DIR/config/interaction.yaml"
log "隔離的家：${HOME_DIR}（API 埠號 ${PORT}）"

if lsof -nP -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; then
  record "launch" "啟動 .app 並等到 /ready" "failed" "埠號 $PORT 已被占用，換一個 --port 再跑"
  exit 1
fi

# --- 3. 啟動 -----------------------------------------------------------------
INTERACT_AI_HOME="$HOME_DIR" \
INTERACT_AI_MOBILE_ADVERTISE=0 \
INTERACT_AI_CODEX_BIN="$REPO_ROOT/crates/interaction-runtime/tests/fixtures/fake_codex.sh" \
INTERACT_AI_CLAUDE_BIN="$REPO_ROOT/crates/interaction-runtime/tests/fixtures/fake_claude.sh" \
  "$BIN" >>"$LOG_FILE" 2>&1 &
APP_PID=$!
log "App pid=$APP_PID"

ready=""
for _ in $(seq 1 60); do
  ready="$(curl -sS --max-time 2 "$API/ready" 2>/dev/null)"
  case "$ready" in *'"status"'*) break ;; esac
  kill -0 "$APP_PID" 2>/dev/null || { ready=""; break; }
  sleep 1
done
if [ -z "$ready" ]; then
  record "launch" "啟動 .app 並等到 /ready" "failed" "60 秒內 $API/ready 沒有回應（App 可能已退出，見 walkthrough.log）"
  exit 1
fi
TOKEN="$(tr -d '\n' <"$HOME_DIR/state/api-token" 2>/dev/null)"
if [ -z "$TOKEN" ]; then
  record "launch" "啟動 .app 並等到 /ready" "failed" "/ready 有回應但讀不到 $HOME_DIR/state/api-token"
  exit 1
fi

# 程序名（System Events 用的名字，可能與產品名不同）。
PROC_NAME="$(osascript -e "tell application \"System Events\" to get name of (first process whose unix id is $APP_PID)" 2>>"$LOG_FILE")"
[ -n "$PROC_NAME" ] || PROC_NAME="interaction-desktop"
log "System Events 程序名：$PROC_NAME"

# --- 4. AX 權限探測 ----------------------------------------------------------
sleep 3
WINDOWS="$(ax windows)"
if [ -z "$WINDOWS" ]; then
  AX_OK=0
  record "launch" "啟動 .app 並等到 /ready" "completed" "/ready＝${ready}；token 讀得到；但 System Events 讀不到視窗清單"
  for step in onboarding switch-character companion-preset pause-proactive do-not-disturb companion-visibility emergency-stop narrow-390; do
    record "$step" "（AX 驅動）" "needs-environment" "沒有輔助使用權限（系統設定 → 隱私權與安全性 → 輔助使用），或 System Events 讀不到這個程序的視窗；腳本保留可重跑"
  done
  exit 0
fi
AX_OK=1
log "視窗清單：$(echo "$WINDOWS" | tr '\n' '/')"
S1_SHOT="$(shot 01-launch || true)"
record "launch" "啟動 .app 並等到 /ready" "completed" \
  "/ready＝${ready}；視窗清單＝$(echo "$WINDOWS" | tr '\n' '/')" "$S1_SHOT"

# --- 5. 首次設定精靈 ---------------------------------------------------------
step_onboarding() {
  local before after
  before="$(jq_path "$(hget /v1/status)" 'd.get("onboardingCompleted")')"
  ax_click_wait AXButton "下一步" 30 || { record onboarding "首次設定精靈走到完成並顯示角色" failed "找不到「下一步」（精靈可能沒有出現；onboardingCompleted=${before}）"; return; }
  sleep 1
  ax_click_wait AXButton "下一步" 20
  sleep 1
  ax_click_wait AXButton "完成設定" 20 || { record onboarding "首次設定精靈走到完成並顯示角色" failed "找不到「完成設定」"; return; }
  sleep 1
  # 套用前確認：按「套用」之前後端什麼都沒改。
  local mid; mid="$(jq_path "$(hget /v1/status)" 'd.get("onboardingCompleted")')"
  ax_click_wait AXButton "套用" 20 || { record onboarding "首次設定精靈走到完成並顯示角色" failed "找不到套用前確認的「套用」"; return; }
  sleep 2
  ax click AXAny "完成" >/dev/null 2>&1   # 首次成功體驗（可略過）
  sleep 1
  after="$(jq_path "$(hget /v1/status)" 'd.get("onboardingCompleted")')"
  local wins; wins="$(ax windows | tr '\n' '/')"
  local s; s="$(shot 02-onboarding || true)"
  if [ "$after" = "True" ]; then
    record onboarding "首次設定精靈走到完成並顯示角色" completed \
      "onboardingCompleted：$before → 套用前 $mid → 套用後 ${after}；視窗清單＝$wins" "$s"
  else
    record onboarding "首次設定精靈走到完成並顯示角色" failed \
      "走完精靈之後 onboardingCompleted 仍是 ${after}；視窗清單＝$wins" "$s"
  fi
}
step_onboarding

# --- 6. 更換角色 -------------------------------------------------------------
step_switch_character() {
  local pack_before pack_after id_before id_after wins_before wins_after
  pack_before="$(prefs_get companionPack)"
  id_before="$(jq_path "$(hget /v1/status)" 'd.get("presentation",{}).get("packId")')"
  wins_before="$(win_set)"
  ax navclick 2 >/dev/null || { record switch-character "更換角色（讀回 packId／companionPack）" failed "點不到導覽的第 2 項（角色頁）"; return; }
  sleep 2
  ax_click_wait AXDisclosureTriangle "更換或加入角色" 20 \
    || ax_click_wait AXAny "更換或加入角色" 10
  if ! ax_click_wait AXButton "選用" 30; then
    record switch-character "更換角色（讀回 packId／companionPack）" failed "展開角色庫之後找不到「選用」按鈕"
    return
  fi
  sleep 3
  pack_after="$(prefs_get companionPack)"
  id_after="$(jq_path "$(hget /v1/status)" 'd.get("presentation",{}).get("packId")')"
  wins_after="$(win_set)"
  local s; s="$(shot 03-switch-character || true)"
  if [ -n "$pack_after" ] && [ "$pack_after" != "$pack_before" ]; then
    record switch-character "更換角色（讀回 packId／companionPack）" completed \
      "companionPack：$pack_before → ${pack_after}；status.presentation.packId：$id_before → ${id_after}；視窗清單 $wins_before → $wins_after" "$s"
  else
    record switch-character "更換角色（讀回 packId／companionPack）" failed \
      "按了「選用」但 companionPack 沒變（$pack_before → ${pack_after}）；packId：$id_before → $id_after" "$s"
  fi
}
step_switch_character

# --- 7. 陪伴預設「安靜」 -----------------------------------------------------
step_preset() {
  ax navclick 2 >/dev/null
  sleep 1
  # `<button aria-pressed>` 在 macOS AX 上是 AXCheckBox（不是 AXButton）——
  # 這也正好讓 `value` 讀得到「有沒有被按下」，不必靠截圖判斷高亮。
  if ! ax_click_wait AXCheckBox "安靜" 20; then
    record companion-preset "陪伴預設「安靜」（讀回偏好＋主動說話模式）" failed "角色頁上找不到「安靜」檔位按鈕"
    return
  fi
  sleep 3
  local expr dnd mode pressed
  expr="$(prefs_get companionExpressiveness)"
  dnd="$(prefs_get companionDoNotDisturb)"
  mode="$(jq_path "$(hget /v1/proactive-dialogue)" 'd.get("config",{}).get("mode")')"
  pressed="$(ax value AXCheckBox "安靜" 2>/dev/null)"
  local s; s="$(shot 04-companion-preset || true)"
  if [ "$expr" = "quiet" ] && [ "$dnd" = "True" ] && [ "$mode" = "necessary" ]; then
    record companion-preset "陪伴預設「安靜」（讀回偏好＋主動說話模式）" completed \
      "prefs.companionExpressiveness=${expr}、companionDoNotDisturb=${dnd}、GET /v1/proactive-dialogue mode=${mode}；AX 讀回「安靜」按鈕 AXValue=${pressed:-（AX 未提供）}（1＝aria-pressed，畫面確實高亮在「安靜」）" "$s"
  else
    record companion-preset "陪伴預設「安靜」（讀回偏好＋主動說話模式）" failed \
      "按了「安靜」但讀回是 expressiveness=${expr}、doNotDisturb=${dnd}、mode=${mode}（期望 quiet／True／necessary）" "$s"
  fi
}
step_preset

# --- 8. 暫停主動對話 ---------------------------------------------------------
step_pause() {
  ax navclick 1 >/dev/null
  sleep 1
  local before mid after quiet_before quiet_after
  before="$(jq_path "$(hget /v1/pause)" 'd.get("paused")')"
  quiet_before="$(jq_path "$(hget /v1/policy)" 'json.dumps(d.get("quietHours"),ensure_ascii=False)')"
  if ! ax_click_wait AXButton "暫停主動互動" 20; then
    record pause-proactive "暫停主動對話並恢復（/v1/pause）" failed "「現在」頁找不到「暫停主動互動」"
    return
  fi
  sleep 2
  mid="$(jq_path "$(hget /v1/pause)" 'd.get("paused")')"
  local s; s="$(shot 05-pause || true)"
  if ! ax_click_wait AXButton "恢復主動互動" 20; then
    record pause-proactive "暫停主動對話並恢復（/v1/pause）" failed "暫停後（paused=${mid}）找不到「恢復主動互動」" "$s"
    return
  fi
  sleep 2
  after="$(jq_path "$(hget /v1/pause)" 'd.get("paused")')"
  quiet_after="$(jq_path "$(hget /v1/policy)" 'json.dumps(d.get("quietHours"),ensure_ascii=False)')"
  if [ "$mid" = "True" ] && [ "$after" = "False" ] && [ "$quiet_before" = "$quiet_after" ]; then
    record pause-proactive "暫停主動對話並恢復（/v1/pause）" completed \
      "/v1/pause paused：$before → $mid → ${after}；安靜時段沒有被順手改掉（${quiet_before}）" "$s"
  else
    record pause-proactive "暫停主動對話並恢復（/v1/pause）" failed \
      "paused：$before → $mid → ${after}（期望 False→True→False）；安靜時段 $quiet_before → $quiet_after" "$s"
  fi
}
step_pause

# --- 9. 勿擾 -----------------------------------------------------------------
step_dnd() {
  ax navclick 2 >/dev/null
  sleep 1
  local before after
  before="$(prefs_get companionDoNotDisturb)"
  ax_click_wait AXDisclosureTriangle "安靜與勿擾" 20 \
    || ax_click_wait AXAny "安靜與勿擾" 10
  if ! ax_click_wait AXCheckBox "勿擾" 20; then
    record do-not-disturb "勿擾開關（讀回偏好）" failed "展開「安靜與勿擾」之後找不到勿擾開關（before=${before}）"
    return
  fi
  sleep 2
  after="$(prefs_get companionDoNotDisturb)"
  local s; s="$(shot 06-dnd || true)"
  if [ -n "$after" ] && [ "$after" != "$before" ]; then
    record do-not-disturb "勿擾開關（讀回偏好）" completed \
      "prefs.companionDoNotDisturb：$before → ${after}（開關真的寫進偏好檔）" "$s"
  else
    record do-not-disturb "勿擾開關（讀回偏好）" failed \
      "按了勿擾開關但偏好沒變（$before → ${after}）" "$s"
  fi
}
step_dnd

# --- 10. 顯示／隱藏桌面角色 --------------------------------------------------
step_visibility() {
  local w0 w1 w2 v0 v1 v2
  w0="$(win_set)"
  v0="$(jq_path "$(hget /v1/status)" 'd.get("presentation",{}).get("visible")')"
  if ! ax tray "隱藏桌面角色" >/dev/null; then
    record companion-visibility "顯示／隱藏桌面角色（視窗清單＋presentation.visible）" failed "狀態列選單裡找不到「隱藏桌面角色」（目前 visible=${v0}）"
    return
  fi
  sleep 2
  w1="$(win_set)"
  v1="$(jq_path "$(hget /v1/status)" 'd.get("presentation",{}).get("visible")')"
  local s; s="$(shot 07-companion-hidden || true)"
  if ! ax tray "顯示桌面角色" >/dev/null; then
    record companion-visibility "顯示／隱藏桌面角色（視窗清單＋presentation.visible）" failed \
      "隱藏後（視窗 $w0 → ${w1}，visible $v0 → ${v1}）找不到「顯示桌面角色」" "$s"
    return
  fi
  sleep 2
  w2="$(win_set)"
  v2="$(jq_path "$(hget /v1/status)" 'd.get("presentation",{}).get("visible")')"
  local prefs_vis; prefs_vis="$(prefs_get companionVisible)"
  if [ "$w1" != "$w0" ] && [ "$w2" = "$w0" ]; then
    record companion-visibility "顯示／隱藏桌面角色（視窗清單＋presentation.visible）" completed \
      "視窗清單：$w0 → $w1 → ${w2}；status.presentation.visible：$v0 → $v1 → ${v2}；prefs.companionVisible=$prefs_vis" "$s"
  else
    record companion-visibility "顯示／隱藏桌面角色（視窗清單＋presentation.visible）" failed \
      "視窗清單沒有如預期消失又出現：$w0 → $w1 → ${w2}；visible：$v0 → $v1 → $v2" "$s"
  fi
}
step_visibility

# --- 11. 緊急停止與安全解除 --------------------------------------------------
step_estop() {
  ax navclick 1 >/dev/null
  sleep 1
  local e0 e1 e2
  e0="$(jq_path "$(hget /v1/status)" 'd.get("emergencyStop")')"
  if ! ax_click_wait AXButton "緊急停止" 20; then
    record emergency-stop "緊急停止與安全解除（二段確認）" failed "找不到「緊急停止」按鈕（emergencyStop=${e0}）"
    return
  fi
  sleep 1
  # 二段確認：第一下只是 arm，第二下才真的停。
  local armed; armed="$(jq_path "$(hget /v1/status)" 'd.get("emergencyStop")')"
  if ! ax_click_wait AXAny "立即停止一切" 20; then
    record emergency-stop "緊急停止與安全解除（二段確認）" failed "按了緊急停止但找不到二段確認「立即停止一切？」（此時 emergencyStop=${armed}）"
    return
  fi
  sleep 2
  e1="$(jq_path "$(hget /v1/status)" 'd.get("emergencyStop")')"
  local s; s="$(shot 08-estop || true)"
  # 解除：不是一顆按鈕，要走安全流程（前往解除 → 開始安全解除流程 → 兩段確認）。
  ax_click_wait AXAny "前往解除" 15
  sleep 1
  ax_click_wait AXAny "開始安全解除流程" 15
  sleep 1
  ax_click_wait AXAny "我了解，解除緊急停止" 15
  sleep 1
  ax_click_wait AXAny "確定解除" 15
  sleep 2
  e2="$(jq_path "$(hget /v1/status)" 'd.get("emergencyStop")')"
  if [ "$armed" = "False" ] && [ "$e1" = "True" ] && [ "$e2" = "False" ]; then
    record emergency-stop "緊急停止與安全解除（二段確認）" completed \
      "emergencyStop：$e0 →（按第一下之後仍是 ${armed}，二段確認不可略過）→ $e1 → 走完安全解除流程後 $e2" "$s"
  elif [ "$e1" = "True" ]; then
    record emergency-stop "緊急停止與安全解除（二段確認）" failed \
      "停得下來但解除流程沒走完：emergencyStop $e0 → 第一下 $armed → $e1 → ${e2}（系統可能仍在緊急停止中）" "$s"
  else
    record emergency-stop "緊急停止與安全解除（二段確認）" failed \
      "emergencyStop：$e0 → 第一下 $armed → $e1 → ${e2}（期望 False→False→True→False）" "$s"
  fi
}
step_estop

# --- 12. 390px ---------------------------------------------------------------
step_narrow() {
  ax navclick 1 >/dev/null
  sleep 1
  if ! ax resize 390 844 >/dev/null; then
    record narrow-390 "視窗縮到 390px 寬並檢查橫向捲動" failed "AX 設定視窗大小失敗"
    return
  fi
  sleep 2
  local b w hs
  b="$(ax bounds)"
  w="$(echo "$b" | cut -d, -f3)"
  hs="$(ax hscroll 2>/dev/null)"
  local s; s="$(shot 09-narrow-390 || true)"
  if [ "$w" -gt 420 ] 2>/dev/null; then
    record narrow-390 "視窗縮到 390px 寬並檢查橫向捲動" failed \
      "視窗縮不到 390px（實際寬度 ${w}；可能有最小寬度限制）；AX 水平捲軸＝${hs:-未知}" "$s"
    return
  fi
  if [ "$hs" = "no" ]; then
    record narrow-390 "視窗縮到 390px 寬並檢查橫向捲動" completed \
      "視窗實際大小＝${b}；AX 樹裡沒有水平捲軸（AXScrollBar/AXHorizontalOrientation）。限制：AX 讀不到 documentElement.scrollWidth，這一項是「沒有水平捲軸」而不是「像素級沒有溢出」；像素級由 Playwright 的 390px 任務覆蓋" "$s"
  else
    record narrow-390 "視窗縮到 390px 寬並檢查橫向捲動" failed \
      "視窗實際大小＝${b}；AX 樹裡偵測到水平捲軸（hscroll=${hs}）" "$s"
  fi
}
step_narrow

log "走查結束"
exit 0
