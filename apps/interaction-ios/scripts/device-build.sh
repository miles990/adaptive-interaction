#!/usr/bin/env bash
#
# device-build.sh — 把 InteractionCompanion 編到「真的 iPhone」上。
#
# 誠實原則(對應 repo CLAUDE.md):
#   - 每個前置條件都是**閘門**:不成立就印出「人要做什麼」並以非 0 結束,
#     絕不先跑 xcodebuild 再讓它噴一堆看不懂的簽章錯誤。
#   - 安裝成功 ≠ 功能已驗收;啟動成功 ≠ 已配對。本腳本只印 devicectl 實際回什麼。
#   - 不把 UDID / 配對碼 / token 寫進 repo:所有 --json-output 一律落在
#     mktemp 產生的暫存目錄,配對 JSON 只當成參數傳給 devicectl,印出時遮蔽配對碼。
#
# 用法:
#   apps/interaction-ios/scripts/device-build.sh [選項]
#
#   --check-only                 只跑前置閘門,不編譯(拿來看還缺什麼)
#   --configuration Debug|Release 預設 Debug(--pairing-payload 等啟動參數只有 Debug 編得進去)
#   --device <identifier>         指定裝置(預設:自動挑唯一一台已配對的 iOS 裝置)
#   --pairing-payload '<json>'    啟動時帶入配對 JSON(等同在 App 裡貼上並按「開始配對」)
#   --auto-connect                啟動時用已存的憑證重連
#   --initial-tab pairing|sensors|character
#   --no-install                  只編譯,不安裝
#   --no-launch                   安裝但不啟動
#
# 環境變數:
#   DEVELOPER_DIR            必須指向 Xcode.app(非 Command Line Tools)
#   IOS_DEVELOPMENT_TEAM     Team ID;沒設就從 Xcode 的 IDEProvisioningTeams 讀第一個
#   IOS_DEVICE_ID            同 --device
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IOS_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
PROJECT="$IOS_DIR/InteractionCompanion.xcodeproj"
SCHEME="InteractionCompanion"
BUNDLE_ID="dev.interact-ai.companion"

CONFIGURATION="Debug"
DEVICE_ID="${IOS_DEVICE_ID:-}"
PAIRING_PAYLOAD=""
AUTO_CONNECT=0
INITIAL_TAB=""
DO_INSTALL=1
DO_LAUNCH=1
CHECK_ONLY=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --check-only) CHECK_ONLY=1; shift ;;
    --configuration) CONFIGURATION="${2:?--configuration 需要值}"; shift 2 ;;
    --device) DEVICE_ID="${2:?--device 需要值}"; shift 2 ;;
    --pairing-payload) PAIRING_PAYLOAD="${2:?--pairing-payload 需要值}"; shift 2 ;;
    --auto-connect) AUTO_CONNECT=1; shift ;;
    --initial-tab) INITIAL_TAB="${2:?--initial-tab 需要值}"; shift 2 ;;
    --no-install) DO_INSTALL=0; DO_LAUNCH=0; shift ;;
    --no-launch) DO_LAUNCH=0; shift ;;
    -h|--help) sed -n '2,40p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "未知選項:$1(用 --help 看用法)" >&2; exit 2 ;;
  esac
done

WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/interact-ios-build.XXXXXX")"
cleanup() { rm -rf "$WORK_DIR"; }
trap cleanup EXIT

say()  { printf '%s\n' "$*"; }
step() { printf '\n=== %s ===\n' "$*"; }
fail() { printf '\n[閘門未通過] %s\n' "$1" >&2; shift; while [[ $# -gt 0 ]]; do printf '  %s\n' "$1" >&2; shift; done; exit 1; }

# 遮蔽配對碼後再印(配對碼是短期祕密,不該進終端記錄/CI log)
redact_payload() {
  python3 - "$1" <<'PY' 2>/dev/null || echo '<無法解析的配對 JSON>'
import json, sys
try:
    obj = json.loads(sys.argv[1])
except Exception:
    print("<無法解析的配對 JSON>")
    sys.exit(0)
if isinstance(obj, dict) and "code" in obj:
    obj["code"] = "******"
print(json.dumps(obj, ensure_ascii=False, separators=(",", ":")))
PY
}

step "0/5 基本工具"
if ! command -v python3 >/dev/null 2>&1; then
  fail "找不到 python3(本腳本用它解析 devicectl 的 JSON)。" \
       "安裝 Xcode Command Line Tools:xcode-select --install"
fi
DEV_DIR="${DEVELOPER_DIR:-$(xcode-select -p 2>/dev/null || true)}"
if [[ -z "$DEV_DIR" || ! -d "$DEV_DIR/Platforms/iPhoneOS.platform" ]]; then
  fail "DEVELOPER_DIR 沒有指向完整的 Xcode(現在是:${DEV_DIR:-未設定})。" \
       "你要做的:" \
       "  export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer" \
       "(只有 Command Line Tools 沒辦法簽真機 App。)"
fi
export DEVELOPER_DIR="$DEV_DIR"
say "DEVELOPER_DIR = $DEVELOPER_DIR"
say "Xcode         = $(xcodebuild -version 2>/dev/null | head -1)"
[[ -d "$PROJECT" ]] || fail "找不到 $PROJECT" "這個腳本必須留在 apps/interaction-ios/scripts/ 底下執行。"

step "1/5 找到 iPhone"
DEVICES_JSON="$WORK_DIR/devices.json"
if ! xcrun devicectl list devices --json-output "$DEVICES_JSON" >/dev/null 2>&1; then
  fail "xcrun devicectl list devices 失敗。" \
       "你要做的:用 USB 線接上 iPhone,解鎖螢幕,若跳出「信任這台電腦?」按「信任」。"
fi
DEVICE_LINE="$(python3 - "$DEVICES_JSON" "$DEVICE_ID" <<'PY'
import json, sys
path, wanted = sys.argv[1], sys.argv[2]
devices = json.load(open(path)).get("result", {}).get("devices", [])
ios = [d for d in devices if d.get("hardwareProperties", {}).get("platform") == "iOS"]
if wanted:
    ios = [d for d in ios
           if wanted in (d.get("identifier"), d.get("hardwareProperties", {}).get("udid"))]
if len(ios) != 1:
    print("COUNT\t%d" % len(ios))
    for d in ios:
        print("CAND\t%s\t%s" % (d.get("identifier"), d.get("deviceProperties", {}).get("name")))
    sys.exit(0)
d = ios[0]
hw, dp = d.get("hardwareProperties", {}), d.get("deviceProperties", {})
print("OK\t%s\t%s\t%s\t%s\t%s" % (
    d.get("identifier", ""), dp.get("name", "?"), hw.get("marketingName", "?"),
    hw.get("productType", "?"), dp.get("osVersionNumber", "?")))
PY
)"
if [[ "$DEVICE_LINE" != OK* ]]; then
  fail "找不到唯一一台 iOS 裝置(devicectl 看到 $(printf '%s' "$DEVICE_LINE" | head -1 | cut -f2) 台符合條件)。" \
       "你要做的:只接一台 iPhone,或用 --device <identifier> / IOS_DEVICE_ID 指定。" \
       "可用 xcrun devicectl list devices 查看清單。"
fi
IFS=$'\t' read -r _ DEVICE_ID DEVICE_NAME DEVICE_MARKETING DEVICE_MODEL DEVICE_OS <<<"$DEVICE_LINE"
say "裝置:$DEVICE_NAME($DEVICE_MARKETING / $DEVICE_MODEL,iOS $DEVICE_OS)"
say "(identifier 只在本次執行的記憶體與暫存目錄中使用,不寫進 repo)"

step "2/5 Developer Mode"
DETAILS_JSON="$WORK_DIR/details.json"
if ! xcrun devicectl device info details --device "$DEVICE_ID" --json-output "$DETAILS_JSON" >/dev/null 2>&1; then
  fail "xcrun devicectl device info details 失敗(裝置可能已拔線或鎖定)。" \
       "你要做的:接上 USB、解鎖 iPhone,再跑一次。"
fi
DEV_MODE="$(python3 - "$DETAILS_JSON" <<'PY'
import json, sys
r = json.load(open(sys.argv[1])).get("result", {})
print(r.get("deviceProperties", {}).get("developerModeStatus", "unknown"))
PY
)"
if [[ "$DEV_MODE" != "enabled" ]]; then
  fail "iPhone 的 Developer Mode 目前是「${DEV_MODE}」,devicectl 無法安裝或啟動 App。" \
       "你要做的(只需一次,需要重開機):" \
       "  1. iPhone → 設定 → 隱私權與安全性 → 開發者模式 → 開啟" \
       "  2. 依提示重新啟動 iPhone" \
       "  3. 重開機後解鎖,點「開啟」並輸入密碼" \
       "  4. 重新執行本腳本(重開機後 devicectl 連線大約要等 30 秒才會就緒)"
fi
say "Developer Mode:enabled"

step "3/5 簽章 Team"
TEAM_ID="${IOS_DEVELOPMENT_TEAM:-}"
TEAM_SOURCE="環境變數 IOS_DEVELOPMENT_TEAM"
if [[ -z "$TEAM_ID" ]]; then
  TEAM_SOURCE="Xcode 帳號設定(IDEProvisioningTeams)"
  TEAM_PLIST="$WORK_DIR/teams.plist"
  if defaults export com.apple.dt.Xcode "$TEAM_PLIST" >/dev/null 2>&1; then
    TEAM_ID="$(python3 - "$TEAM_PLIST" <<'PY'
import plistlib, sys
try:
    data = plistlib.load(open(sys.argv[1], "rb"))
except Exception:
    sys.exit(0)
teams = data.get("IDEProvisioningTeams") or {}
for _account, entries in teams.items():
    for entry in entries or []:
        team = entry.get("teamID")
        if team:
            print(team)
            sys.exit(0)
PY
)"
  fi
fi
if [[ -z "$TEAM_ID" ]]; then
  fail "找不到任何簽章 Team ID(Xcode 裡還沒登入 Apple ID)。" \
       "你要做的(只需一次):" \
       "  1. 開啟 Xcode → Settings → Accounts → 左下角「+」→ Apple ID → 登入" \
       "     (免費 Apple ID 就會產生 Personal Team,足以裝到自己的手機上)" \
       "  2. 若被要求,到 developer.apple.com 接受最新的開發者條款" \
       "  3. 重新執行本腳本;或先查到 Team ID 後用 IOS_DEVELOPMENT_TEAM=XXXXXXXXXX 指定"
fi
say "Team ID:$TEAM_ID(來源:$TEAM_SOURCE)"

if [[ "$CHECK_ONLY" == "1" ]]; then
  step "只做前置檢查(--check-only):全部通過"
  say "接下來可直接執行不帶 --check-only 的同一個指令。"
  exit 0
fi

step "4/5 xcodebuild(自動簽章)"
DERIVED="$WORK_DIR/DerivedData"
if [[ "$CONFIGURATION" != "Debug" && ( -n "$PAIRING_PAYLOAD" || "$AUTO_CONNECT" == "1" || -n "$INITIAL_TAB" ) ]]; then
  say "注意:--pairing-payload / --auto-connect / --initial-tab 只編進 Debug 版本"
  say "      (整段在 #if DEBUG 內),現在是 $CONFIGURATION,這些參數會被 App 忽略。"
fi
say "xcodebuild -project $PROJECT -scheme $SCHEME -configuration $CONFIGURATION \\"
say "  -destination id=<裝置> -allowProvisioningUpdates -allowProvisioningDeviceRegistration \\"
say "  DEVELOPMENT_TEAM=$TEAM_ID CODE_SIGN_STYLE=Automatic build"
say "(第一次跑會跳鑰匙圈授權視窗,請按「總是允許」;必須在有登入的桌面工作階段執行,不能用 ssh。)"
BUILD_LOG="$WORK_DIR/xcodebuild.log"
APP_PATH="$DERIVED/Build/Products/$CONFIGURATION-iphoneos/InteractionCompanion.app"
if xcodebuild \
  -project "$PROJECT" \
  -scheme "$SCHEME" \
  -configuration "$CONFIGURATION" \
  -destination "id=$DEVICE_ID" \
  -derivedDataPath "$DERIVED" \
  -allowProvisioningUpdates \
  -allowProvisioningDeviceRegistration \
  DEVELOPMENT_TEAM="$TEAM_ID" \
  CODE_SIGN_STYLE=Automatic \
  build 2>&1 | tee "$BUILD_LOG"; then
  :
elif grep -qE "is not installed|Unable to find a destination" "$BUILD_LOG"; then
  # Xcode 沒有下載 iOS 平台元件時,-destination 解析不到任何 iOS 裝置(2026-09-03 本機實況:
  # 「iOS 26.5 is not installed」)。不需要為此下載 8 GB 模擬器 runtime:改走 -sdk iphoneos
  # 的 target 建置,簽章與裝置註冊同樣由 -allowProvisioningUpdates 完成,再交給 devicectl 安裝。
  step "4/5b xcodebuild(平台元件未安裝:改用 -sdk iphoneos 建置)"
  SYM="$WORK_DIR/sym"; OBJ="$WORK_DIR/obj"
  xcodebuild \
    -project "$PROJECT" \
    -target "$SCHEME" \
    -configuration "$CONFIGURATION" \
    -sdk iphoneos -arch arm64 \
    -allowProvisioningUpdates \
    -allowProvisioningDeviceRegistration \
    DEVELOPMENT_TEAM="$TEAM_ID" \
    CODE_SIGN_STYLE=Automatic \
    SYMROOT="$SYM" OBJROOT="$OBJ" \
    build
  APP_PATH="$SYM/$CONFIGURATION-iphoneos/InteractionCompanion.app"
else
  fail "xcodebuild 失敗(簽章或編譯錯誤)。" "請看上面的輸出;若提到 profile/team,先到 Xcode 的 Signing & Capabilities 選一次 Team。"
fi

[[ -d "$APP_PATH" ]] || fail "編譯完成卻找不到 ${APP_PATH}。" "請把上面的 xcodebuild 輸出貼出來。"
say "已產出:$APP_PATH"

PROFILE="$APP_PATH/embedded.mobileprovision"
if [[ -f "$PROFILE" ]]; then
  EXPIRY="$(security cms -D -i "$PROFILE" 2>/dev/null | plutil -extract ExpirationDate raw -o - - 2>/dev/null || true)"
  if [[ -n "$EXPIRY" ]]; then
    say "佈建描述檔到期時間:$EXPIRY"
    say "(免費 Personal Team 只有 7 天:過期後 App 會拒絕啟動,得重跑本腳本重新簽章。)"
  fi
fi

if [[ "$DO_INSTALL" != "1" ]]; then
  step "5/5 略過安裝(--no-install)"
  exit 0
fi

step "5/5 devicectl 安裝 / 啟動"
INSTALL_JSON="$WORK_DIR/install.json"
xcrun devicectl device install app --device "$DEVICE_ID" "$APP_PATH" --json-output "$INSTALL_JSON"
python3 - "$INSTALL_JSON" <<'PY'
import json, sys
r = json.load(open(sys.argv[1]))
outcome = r.get("info", {}).get("outcome", "unknown")
apps = r.get("result", {}).get("installedApplications", [])
print("安裝結果(devicectl 原文):outcome=%s" % outcome)
for a in apps:
    print("  bundleID=%s installationURL=%s" % (a.get("bundleID"), a.get("installationURL")))
print("誠實提醒:安裝成功 ≠ 已可啟動。第一次安裝後必須在 iPhone 上")
print("  設定 → 一般 → VPN 與裝置管理 → 開發者 App → 信任你的 Apple ID,")
print("  否則啟動會被系統擋下(顯示「不受信任的開發者」)。")
PY

if [[ "$DO_LAUNCH" != "1" ]]; then
  say "已略過啟動(--no-launch)。"
  exit 0
fi

LAUNCH_ARGS=()
if [[ -n "$PAIRING_PAYLOAD" ]]; then
  LAUNCH_ARGS+=(--pairing-payload "$PAIRING_PAYLOAD")
  say "帶入配對 JSON(配對碼已遮蔽):$(redact_payload "$PAIRING_PAYLOAD")"
fi
[[ "$AUTO_CONNECT" == "1" ]] && LAUNCH_ARGS+=(--auto-connect)
[[ -n "$INITIAL_TAB" ]] && LAUNCH_ARGS+=(--initial-tab "$INITIAL_TAB")

LAUNCH_JSON="$WORK_DIR/launch.json"
set +e
xcrun devicectl device process launch \
  --device "$DEVICE_ID" \
  --terminate-existing \
  --json-output "$LAUNCH_JSON" \
  "$BUNDLE_ID" ${LAUNCH_ARGS[@]+"${LAUNCH_ARGS[@]}"}   # bash 3.2 + set -u:空陣列不能直接展開
LAUNCH_RC=$?
set -e
if [[ $LAUNCH_RC -ne 0 ]]; then
  say ""
  say "啟動失敗(devicectl 結束碼 $LAUNCH_RC)。最常見的兩個原因:"
  say "  1. 還沒信任開發者:iPhone → 設定 → 一般 → VPN 與裝置管理 → 開發者 App → 信任"
  say "  2. 免費 Team 的描述檔過期(7 天):重跑本腳本重新簽章"
  say "結果標示為 uncertain:App 是否真的跑起來,以 iPhone 螢幕為準。"
  exit $LAUNCH_RC
fi
python3 - "$LAUNCH_JSON" <<'PY'
import json, sys
r = json.load(open(sys.argv[1]))
proc = r.get("result", {}).get("process", {})
print("啟動結果(devicectl 原文):outcome=%s pid=%s" % (
    r.get("info", {}).get("outcome", "unknown"), proc.get("processIdentifier")))
print("誠實提醒:process 起來 ≠ 已配對 ≠ 功能已驗收。")
print("  第一次連線時 iPhone 會問「本地網路」權限,必須按「允許」;")
print("  麥克風 / 藍牙 / 通知 / 相機 / 定位也都是第一次用到才會問。")
print("  真機驗收請接著跑 apps/interaction-ios/scripts/device-acceptance.sh。")
PY
