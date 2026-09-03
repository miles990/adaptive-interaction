#!/usr/bin/env bash
#
# device-acceptance.sh — 用「真的 daemon」對「真的 iPhone」跑一次驗收矩陣。
#
# 誠實原則(對應 repo CLAUDE.md,違反就是這支腳本壞掉):
#   - **本腳本不判定成敗、不編造結果**:每一列只把 daemon 真正回什麼原樣印出來
#     (Requested / Dispatched / Acknowledged / deviceApplied / outcome)。
#     沒回應就印「未知(uncertain)」,絕不寫成成功或失敗。
#   - acknowledged ≠ completed ≠ verified;notify 的 scheduled ≠ 已顯示;
#     tts 的 started ≠ 唸完。這些字眼在輸出裡原樣保留,不美化。
#   - **不代替使用者授權**:缺同意就印出「人要自己跑哪一行」,
#     不會偷偷幫你 session consent(除非你自己加 --grant-consent)。
#   - 破壞性動作(緊急停止 / 撤銷配對)預設**不執行**,要 --confirm-destructive。
#   - 全部輸出標示「真機 iPhone」,方便和模擬器證據區分。
#   - 不把 token / 配對碼寫進 repo,也不印出 token。
#
# 前置:daemon 已在跑(interact-ai serve),iPhone 已用
#      apps/interaction-ios/scripts/device-build.sh 裝好並完成配對。
#
# 用法:
#   apps/interaction-ios/scripts/device-acceptance.sh [選項]
#     --api <url>            預設 http://127.0.0.1:8787
#     --rows <a,b,c>         只跑指定列(預設除破壞性列外全跑)
#     --list-rows            印出所有列名後結束
#     --grant-consent        允許本腳本呼叫 /v1/session/consent 補齊同意
#     --confirm-destructive  允許執行 estop / estop-clear / revoke 三列
#     --dry-run              只印出將送出的請求,不真的送(不產生任何副作用)
#
# 環境變數:INTERACT_API_TOKEN(預設讀 ~/.adaptive-interaction/state/api-token)
#
set -euo pipefail

API="${INTERACT_API:-http://127.0.0.1:8787}"
TOKEN_FILE="${HOME}/.adaptive-interaction/state/api-token"
ROWS=""
GRANT_CONSENT=0
CONFIRM_DESTRUCTIVE=0
DRY_RUN=0

ALL_ROWS="pair status haptic notify tts torch flash character character-verified-rejected observe-motion observe-battery observe-touch observe-mic ble-scan sensors-stop estop estop-clear revoke"
DESTRUCTIVE_ROWS="estop estop-clear revoke"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --api) API="${2:?--api 需要值}"; shift 2 ;;
    --rows) ROWS="${2:?--rows 需要值}"; shift 2 ;;
    --list-rows) printf '%s\n' "$ALL_ROWS" | tr ' ' '\n'; exit 0 ;;
    --grant-consent) GRANT_CONSENT=1; shift ;;
    --confirm-destructive) CONFIRM_DESTRUCTIVE=1; shift ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) sed -n '2,32p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "未知選項:$1(用 --help 看用法)" >&2; exit 2 ;;
  esac
done

say()  { printf '%s\n' "$*"; }
step() { printf '\n=== %s ===\n' "$*"; }
fail() { printf '\n[閘門未通過] %s\n' "$1" >&2; shift; while [[ $# -gt 0 ]]; do printf '  %s\n' "$1" >&2; shift; done; exit 1; }

command -v python3 >/dev/null 2>&1 || fail "找不到 python3(本腳本用它解析 daemon 的 JSON)。" "xcode-select --install"
command -v curl >/dev/null 2>&1 || fail "找不到 curl。"

TOKEN="${INTERACT_API_TOKEN:-}"
if [[ -z "$TOKEN" ]]; then
  [[ -r "$TOKEN_FILE" ]] || fail "讀不到 API token:$TOKEN_FILE" \
    "你要做的:先啟動 daemon(interact-ai serve),或用 INTERACT_API_TOKEN=… 指定。"
  TOKEN="$(tr -d '\r\n' < "$TOKEN_FILE")"
fi
[[ -n "$TOKEN" ]] || fail "API token 是空的。"   # 從不印出 token 本身

# --- 極薄的 HTTP 包裝:回傳 body;非 2xx 也照樣回 body(誠實顯示 daemon 說什麼) ---
api() {  # api <METHOD> <PATH> [JSON body]
  local method="$1" path="$2" body="${3:-}"
  if [[ "$DRY_RUN" == "1" ]]; then
    printf '{"dryRun":true,"method":"%s","path":"%s","body":%s}\n' \
      "$method" "$path" "${body:-null}"
    return 0
  fi
  if [[ -n "$body" ]]; then
    curl -sS -X "$method" "${API}${path}" \
      -H "Authorization: Bearer ${TOKEN}" -H 'content-type: application/json' -d "$body"
  else
    curl -sS -X "$method" "${API}${path}" -H "Authorization: Bearer ${TOKEN}"
  fi
}

# python 取值:jget '<json>' '<python expr on d>'
jget() { python3 -c '
import sys, json
raw = sys.argv[1]
try:
    d = json.loads(raw)
except Exception:
    print("PARSE-FAIL"); sys.exit(0)
try:
    v = eval(sys.argv[2], {"d": d, "json": json})
except Exception:
    print("MISSING"); sys.exit(0)
print(v if isinstance(v, str) else json.dumps(v, ensure_ascii=False))
' "$1" "$2"; }

want_row() {
  local row="$1"
  if [[ -n "$ROWS" ]]; then
    [[ ",$ROWS," == *",$row,"* ]] || return 1
    if [[ " $DESTRUCTIVE_ROWS " == *" $row "* && "$CONFIRM_DESTRUCTIVE" != "1" ]]; then
      say "[真機 iPhone] row=$row  → 跳過(破壞性動作需要 --confirm-destructive)"
      return 1
    fi
    return 0
  fi
  if [[ " $DESTRUCTIVE_ROWS " == *" $row "* && "$CONFIRM_DESTRUCTIVE" != "1" ]]; then
    say "[真機 iPhone] row=$row  → 跳過(破壞性動作需要 --confirm-destructive)"
    return 1
  fi
  return 0
}

step "0 daemon 與已連線的 iPhone"
[[ "$DRY_RUN" == "1" ]] && say "*** --dry-run:以下只印請求,不送出、不產生任何副作用 ***"
if [[ "$DRY_RUN" != "1" ]]; then
  if ! curl -sS -o /dev/null --max-time 5 "${API}/v1/status" -H "Authorization: Bearer ${TOKEN}" 2>/dev/null; then
    fail "連不上 daemon:${API}" \
         "你要做的:" \
         "  1. 另開一個終端機執行:interact-ai serve" \
         "  2. 或用 --api <url> 指定實際位址(預設 http://127.0.0.1:8787)" \
         "沒有 daemon 就沒有真實結果——本腳本不會用假資料頂替。"
  fi
fi
MOBILE_STATUS="$(api GET /v1/mobile/status)"
if [[ "$DRY_RUN" != "1" ]]; then
  DEVICE_ID="$(jget "$MOBILE_STATUS" 'next((x["deviceId"] for x in d.get("devices", []) if x.get("connected")), "")')"
  if [[ -z "$DEVICE_ID" || "$DEVICE_ID" == "MISSING" || "$DEVICE_ID" == "PARSE-FAIL" ]]; then
    say "daemon 回的 mobile status 原文:"
    say "$MOBILE_STATUS"
    fail "沒有任何**已連線**的 iPhone,驗收矩陣無法執行(不會用假資料代替真機)。" \
         "你要做的:" \
         "  1. 確認 daemon 在跑:interact-ai serve" \
         "  2. 用 apps/interaction-ios/scripts/device-build.sh 把 App 裝上 iPhone 並啟動" \
         "  3. 在桌面產生配對碼(POST /v1/mobile/pairing-session),用 App 掃 QR 或貼上 JSON" \
         "  4. iPhone 與 Mac 必須在同一個 Wi-Fi;第一次連線要按「允許本地網路」"
  fi
  DEVICE_MODEL="$(jget "$MOBILE_STATUS" 'next((x.get("model","?") for x in d.get("devices", []) if x.get("connected")), "?")')"
  say "已連線裝置:$DEVICE_ID(機型 $DEVICE_MODEL —— 真機應為 iPhoneXX,Y;模擬器會是 arm64)"
else
  DEVICE_ID="<device-id>"
  DEVICE_MODEL="<model>"
fi
say "本次所有結果標示:真機 iPhone($DEVICE_MODEL)"

# --- 啟用檢查:iPhone 的動器／受器是 consent-gated,daemon 預設一律 disabled ------
# 「啟用」與「同意」是兩件事(registry 的 enabled 閘 → session consent 閘)。沒啟用的
# 動器連候選都進不了 plan(plan 會誠實回 no-action),所以這裡先把它列出來;只有
# --grant-consent 才代你 PATCH enabled:true,否則印出你要自己跑的命令。
IPHONE_ACTUATORS="iphone.haptic iphone.notify iphone.tts iphone.torch iphone.flash iphone.character"
IPHONE_RECEPTORS="iphone.battery iphone.motion iphone.touch iphone.mic-level"
step "0a 啟用 iPhone 動器／受器 ＋ policy allowlist"
if [[ "$DRY_RUN" != "1" ]]; then
  ACTS_JSON="$(api GET /v1/actuators)"
  DISABLED_ACTS="$(jget "$ACTS_JSON" '" ".join(a["id"] for a in (d if isinstance(d, list) else (d.get("actuators") or d.get("items") or [])) if str(a.get("id","")).startswith("iphone.") and a.get("availability") == "disabled")')"
  [[ "$DISABLED_ACTS" == "MISSING" || "$DISABLED_ACTS" == "PARSE-FAIL" ]] && DISABLED_ACTS=""
  if [[ -n "$DISABLED_ACTS" ]]; then
    if [[ "$GRANT_CONSENT" == "1" ]]; then
      for id in $DISABLED_ACTS; do
        say "啟用動器(你用 --grant-consent 明示):$id"
        api PATCH "/v1/actuators/$id" '{"enabled":true}' >/dev/null
      done
      for id in $IPHONE_RECEPTORS; do
        api PATCH "/v1/receptors/$id" '{"enabled":true}' >/dev/null 2>&1 || true
      done
    else
      say "以下 iPhone 動器目前是 disabled,plan 會回 no-action:$DISABLED_ACTS"
      say "請自己啟用(或加 --grant-consent):"
      for id in $DISABLED_ACTS; do say "  interact-ai actuators enable $id"; done
    fi
  else
    say "iPhone 動器皆已啟用"
  fi
  # Policy Governor 的 allowlist 是第三道閘(enabled → allowlist → consent):不在
  # actuatorAllowlist / allowedChannels 裡的動器會被誠實記成 blocked(rule
  # actuator.allowlist)。這是人類層設定,同樣只有 --grant-consent 才代你合併。
  POLICY_JSON="$(api GET /v1/policy)"
  MISSING_POLICY="$(jget "$POLICY_JSON" 'json.dumps({"actuatorAllowlist": sorted(set(d.get("actuatorAllowlist") or []) | set("'"$IPHONE_ACTUATORS"'".split())), "receptorAllowlist": sorted(set(d.get("receptorAllowlist") or []) | set("'"$IPHONE_RECEPTORS"'".split())), "allowedChannels": sorted(set(d.get("allowedChannels") or []) | {"haptic","audio","light","display","notification","desktop-pet"})}) if (set("'"$IPHONE_ACTUATORS"'".split()) - set(d.get("actuatorAllowlist") or [])) or ({"haptic","audio","light","display"} - set(d.get("allowedChannels") or [])) else ""')"
  if [[ -n "$MISSING_POLICY" && "$MISSING_POLICY" != "MISSING" && "$MISSING_POLICY" != "PARSE-FAIL" ]]; then
    if [[ "$GRANT_CONSENT" == "1" ]]; then
      say "policy allowlist 缺少 iPhone 動器／通道:以 --grant-consent 代你合併(只新增,不移除既有項目)"
      api PATCH /v1/policy "$MISSING_POLICY" >/dev/null
    else
      say "policy allowlist 缺少 iPhone 動器／通道,動作會被記成 blocked(actuator.allowlist)。請自己跑:"
      say "  interact-ai policy set '$MISSING_POLICY'"
    fi
  fi
fi

# --- 同意檢查:缺什麼就講清楚,不代替使用者授權 --------------------------
NEEDED_CONSENTS="channel:haptic channel:notification channel:audio channel:light channel:display channel:desktop-pet receptor:iphone.mic-level"
step "0b Session 與同意"
SESSION="$(api GET /v1/session)"
say "session(原文節錄):$(jget "$SESSION" 'json.dumps({k: d.get(k) for k in ("sessionId","label","consents","expiresAt") if k in d}, ensure_ascii=False)')"
# 沒有 active session 時,所有 plan 都會被 daemon 以 session_inactive 拒絕(誠實但整份矩陣
# 會空跑)。--grant-consent 代表你明示要本腳本代你做「你本來就會自己做」的授權動作,
# 因此在同一個旗標下也代你 start 一個有 TTL 的 session;否則只印出你要自己跑的命令。
SESSION_ID="$(jget "$SESSION" 'print(d.get("sessionId") or d.get("id") or "")' 2>/dev/null || true)"
if [[ "$DRY_RUN" != "1" && -z "$SESSION_ID" ]]; then
  if [[ "$GRANT_CONSENT" == "1" ]]; then
    say "沒有 active session:以 --grant-consent 代你建立一個(TTL 120 分鐘,標籤「真機 iPhone 驗收」)"
    api POST /v1/session/start '{"label":"真機 iPhone 驗收","ttlMinutes":120}' >/dev/null
    SESSION="$(api GET /v1/session)"
    say "session(建立後原文節錄):$(jget "$SESSION" 'json.dumps({k: d.get(k) for k in ("sessionId","label","expiresAt") if k in d}, ensure_ascii=False)')"
  else
    say "沒有 active session:每一列 plan 都會被 daemon 拒絕(session_inactive)。請自己跑:"
    say "  interact-ai session start --label \"真機 iPhone 驗收\" --ttl-minutes 120"
    say "或加 --grant-consent 讓本腳本代你建立。"
  fi
fi
MISSING_CONSENTS=""
if [[ "$DRY_RUN" != "1" ]]; then
  for scope in $NEEDED_CONSENTS; do
    HAS="$(jget "$SESSION" "json.dumps(any((c if isinstance(c,str) else c.get('scope')) == '$scope' for c in (d.get('consents') or [])))")"
    [[ "$HAS" == "true" ]] || MISSING_CONSENTS="$MISSING_CONSENTS $scope"
  done
fi
if [[ -n "$MISSING_CONSENTS" ]]; then
  if [[ "$GRANT_CONSENT" == "1" ]]; then
    for scope in $MISSING_CONSENTS; do
      say "授予同意(你用 --grant-consent 明示同意):$scope"
      api POST /v1/session/consent "{\"scope\":\"$scope\"}" >/dev/null
    done
  else
    say "缺少同意:$MISSING_CONSENTS"
    say "腳本不代替你授權。請自己跑(或加 --grant-consent 由你明示授權):"
    for scope in $MISSING_CONSENTS; do
      say "  interact-ai session consent $scope"
    done
    say "缺同意的列會被 policy 擋下,收據會誠實顯示 blocked —— 那是正確行為,不是腳本壞了。"
  fi
fi

# --- 一列動器的完整流程:plan → execute → 讀收據 --------------------------
actuator_row() {  # actuator_row <row> <actuator-id> <channel> <intent> <payload-json> [message]
  local row="$1" actuator="$2" channel="$3" intent="$4" payload="$5" message="${6:-}"
  want_row "$row" || return 0
  local msg_field=""
  [[ -n "$message" ]] && msg_field="\"message\":$(python3 -c 'import json,sys; print(json.dumps(sys.argv[1]))' "$message"),"
  local plan_body
  plan_body="{\"intent\":\"$intent\",${msg_field}\"magnitude\":0.6,\"durationMs\":800,\"preferredChannels\":[\"$channel\"],\"candidates\":[\"$actuator\"],\"minChannels\":1,\"maxChannels\":1,\"payload\":$payload}"

  printf '\n[真機 iPhone] row=%s actuator=%s channel=%s\n' "$row" "$actuator" "$channel"
  say "  POST /v1/plans  $plan_body"
  local plan_resp plan_id exec_resp action_id receipt
  plan_resp="$(api POST /v1/plans "$plan_body")"
  if [[ "$DRY_RUN" == "1" ]]; then
    say "  (dry-run:不送出 execute)"
    return 0
  fi
  plan_id="$(jget "$plan_resp" 'd.get("planId","")')"
  if [[ -z "$plan_id" || "$plan_id" == "MISSING" || "$plan_id" == "PARSE-FAIL" ]]; then
    say "  plan 未建立,daemon 原文:$plan_resp"
    say "  outcome     : 未建立 plan(uncertain — 本列沒有可讀的收據)"
    return 0
  fi
  exec_resp="$(api POST "/v1/plans/${plan_id}/execute" '{}')"
  action_id="$(jget "$exec_resp" 'json.dumps((d if isinstance(d, list) else d.get("receipts", []))[0].get("actionId")) if (d if isinstance(d, list) else d.get("receipts", [])) else ""')"
  action_id="${action_id//\"/}"
  if [[ -z "$action_id" || "$action_id" == "MISSING" || "$action_id" == "PARSE-FAIL" ]]; then
    say "  execute 沒有回收據,daemon 原文:$exec_resp"
    say "  outcome     : 未知(uncertain)"
    return 0
  fi
  # ack 逾時是 4s(mobile.rs ACT_TIMEOUT):給它一點時間再讀最終收據。
  local waited=0
  while [[ $waited -lt 12 ]]; do
    receipt="$(api GET "/v1/actions/${action_id}")"
    local st; st="$(jget "$receipt" 'd.get("currentStatus","")')"
    case "$st" in
      acknowledged|completed|failed|blocked|uncertain|cancelled|expired|stopped|observed) break ;;
    esac
    sleep 1; waited=$((waited + 1))
  done
  print_receipt "$receipt"
}

print_receipt() {
  local receipt="$1"
  python3 - "$receipt" <<'PY'
import json, sys
try:
    d = json.loads(sys.argv[1])
except Exception:
    print("  收據無法解析,daemon 原文:", sys.argv[1]); sys.exit(0)
def ts(name):
    for entry in d.get("timestamps", []) or []:
        if isinstance(entry, (list, tuple)) and len(entry) == 2 and entry[0] == name:
            return entry[1]
    return "—(從未進入這個狀態)"
dumps = lambda v: json.dumps(v, ensure_ascii=False)
print("  Requested   :", dumps(d.get("requestedParameters")))
print("  Effective   :", dumps(d.get("effectiveBoundedParameters")))
print("  Dispatched  :", ts("dispatched"))
print("  Acknowledged:", ts("acknowledged"))
dr = d.get("driverResponse") or {}
print("  deviceApplied:", dumps(dr.get("deviceApplied")) if "deviceApplied" in dr else "—(手機沒回 applied)")
if dr and "deviceApplied" not in dr:
    print("  driverResponse:", dumps(dr))
print("  outcome     :", d.get("currentStatus", "unknown"))
for e in d.get("errors", []) or []:
    print("  error       :", e.get("code"), e.get("message"))
print("  誠實提醒    : acknowledged ≠ completed ≠ verified;此列只證明手機回了什麼。")
PY
}

step "1 配對 / 連線狀態(row=pair, status)"
if want_row pair; then
  printf '\n[真機 iPhone] row=pair\n'
  say "  已連線裝置:$DEVICE_ID"
  say "  mobile status 原文:$MOBILE_STATUS"
  say "  誠實提醒:connected=true 只代表 socket 活著,不代表任何功能已驗收。"
fi
if want_row status; then
  printf '\n[真機 iPhone] row=status\n'
  FRESH="$(api GET /v1/mobile/status)"
  say "  sensors/permissions(daemon 原文):$(jget "$FRESH" 'json.dumps([{ "deviceId": x.get("deviceId"), "sensors": x.get("sensors"), "permissions": x.get("permissions")} for x in d.get("devices", [])], ensure_ascii=False)')"
  say "  activeSensors(/v1/status):$(jget "$(api GET /v1/status)" 'json.dumps(d.get("activeSensors"), ensure_ascii=False)')"
  say "  誠實提醒:感測不靜默——這裡看到的旗標必須和手機畫面、tray 一致。"
fi

step "2 動器矩陣(每列都是真的送到手機)"
actuator_row haptic  iphone.haptic    haptic      "真機驗收-觸覺" '{"style":"medium","count":2}'
actuator_row notify  iphone.notify    notification "真機驗收-通知" '{"title":"真機驗收","body":"這是一則真機通知"}'
actuator_row tts     iphone.tts       audio        "真機驗收-朗讀" '{"text":"真機驗收測試"}'
actuator_row torch   iphone.torch     light        "真機驗收-手電筒開" '{"on":true,"durationMs":1500}'
actuator_row torch   iphone.torch     light        "真機驗收-手電筒關" '{"on":false}'
actuator_row flash   iphone.flash     display      "真機驗收-螢幕閃示" '{"color":"#FFB347","durationMs":600}'
for st in idle working waiting failed unknown emergency; do
  actuator_row character iphone.character desktop-pet "真機驗收-角色-$st" "{\"state\":\"$st\"}"
done
if want_row character-verified-rejected; then
  printf '\n[真機 iPhone] row=character-verified-rejected\n'
  say "  刻意送 state=verified-success:runtime 必須擋下(綠勾只能由人工驗證路徑產生)。"
  RESP="$(api POST /v1/plans '{"intent":"真機驗收-角色-verified","magnitude":0.6,"durationMs":800,"preferredChannels":["desktop-pet"],"candidates":["iphone.character"],"minChannels":1,"maxChannels":1,"payload":{"state":"verified-success"}}')"
  say "  daemon 回應原文:$RESP"
  # plan 只是草稿;真正的守門在 execute:map_wire_params 對 verified-success 一律拒絕,
  # 收據必須停在 failed 且**沒有** dispatched 時間戳(從未送到手機)。
  VPLAN="$(jget "$RESP" 'd.get("planId") or ""')"
  if [[ -n "$VPLAN" && "$VPLAN" != "MISSING" && "$VPLAN" != "PARSE-FAIL" && "$DRY_RUN" != "1" ]]; then
    VEXEC="$(api POST "/v1/plans/$VPLAN/execute" '{}')"
    say "  execute 收據原文:$(jget "$VEXEC" 'json.dumps([{k: r.get(k) for k in ("actuatorId","currentStatus","timestamps","errors")} for r in (d if isinstance(d, list) else d.get("receipts") or d.get("actions") or [d])], ensure_ascii=False)')"
    say "  期望:currentStatus=failed、timestamps 沒有 dispatched、error 提到 human-verification only。"
  fi
  say "  期望:plan 可以是草稿,但 execute 必須被 runtime 擋下;若收據出現 dispatched/acknowledged,就是安全缺陷,請記為 finding。"
fi

step "3 觀察(需要人實際搖手機 / 點角色)"
observe_row() {  # observe_row <row> <receptor>
  local row="$1" receptor="$2"
  want_row "$row" || return 0
  printf '\n[真機 iPhone] row=%s receptor=%s\n' "$row" "$receptor"
  local resp
  resp="$(api POST /v1/observations/query "{\"receptorId\":\"$receptor\",\"limit\":5}")"
  say "  最近 5 筆(daemon 原文):$resp"
  say "  誠實提醒:沒有資料就是沒有——請人先在手機上做出對應動作,不要用假資料補。"
}
observe_row observe-motion  iphone.motion
observe_row observe-battery iphone.battery
observe_row observe-touch   iphone.touch
observe_row observe-mic     iphone.mic-level

step "4 BLE 掃描(需要在 App 內先開啟閘道)"
if want_row ble-scan; then
  printf '\n[真機 iPhone] row=ble-scan\n'
  RESP="$(api POST /v1/mobile/ble/scan '{"durationMs":4000}')"
  say "  daemon 原文:$RESP"
  say "  誠實提醒:閘道關著會回 ble-gateway-disabled;掃不到裝置回空清單也是誠實結果,不是失敗。"
fi

step "5 停止所有感測(使用者路徑)"
if want_row sensors-stop; then
  printf '\n[真機 iPhone] row=sensors-stop\n'
  RESP="$(api POST "/v1/mobile/devices/${DEVICE_ID}/sensors/stop" '{}')"
  say "  daemon 原文:$RESP"
  [[ "$DRY_RUN" == "1" ]] || sleep 1
  say "  停止後 mobile status:$(jget "$(api GET /v1/mobile/status)" 'json.dumps([x.get("sensors") for x in d.get("devices", [])], ensure_ascii=False)')"
  say "  誠實提醒:手機沒在時限內確認 = 結果未知(uncertain),不得寫成已停止。"
  say "  已知落差:桌面目前送的 stop-all 不帶 reason,App 會顯示「因桌面緊急停止而停用」;"
  say "            App 端已支援 reason=user(見 apps/interaction-ios README「停止全部感測的兩種原因」)。"
fi

step "6 緊急停止 / 解除(破壞性)"
if want_row estop; then
  printf '\n[真機 iPhone] row=estop\n'
  RESP="$(api POST /v1/emergency-stop '{"reason":"真機驗收"}')"
  say "  daemon 原文:$RESP"
  [[ "$DRY_RUN" == "1" ]] || sleep 2
  say "  /v1/status.activeSensors:$(jget "$(api GET /v1/status)" 'json.dumps(d.get("activeSensors"), ensure_ascii=False)')"
  say "  手機自報 sensors:$(jget "$(api GET /v1/mobile/status)" 'json.dumps([x.get("sensors") for x in d.get("devices", [])], ensure_ascii=False)')"
  say "  audit(最近 10 筆,找 mobile.estop-stop-sensors):$(jget "$(api GET '/v1/audit?limit=10')" 'json.dumps([e for e in (d if isinstance(d, list) else d.get("entries", [])) if "estop" in json.dumps(e)], ensure_ascii=False)')"
fi
if want_row estop-clear; then
  printf '\n[真機 iPhone] row=estop-clear\n'
  say "  誠實提醒:AI 不得解除緊急停止——這一列是「人」在跑腳本時明示要求的。"
  RESP="$(api POST /v1/emergency-stop/clear)"
  say "  daemon 原文:$RESP"
  [[ "$DRY_RUN" == "1" ]] || sleep 5
  say "  解除 5 秒後手機自報 sensors(期望仍全 false ＝ 不自動恢復):$(jget "$(api GET /v1/mobile/status)" 'json.dumps([x.get("sensors") for x in d.get("devices", [])], ensure_ascii=False)')"
fi

step "7 撤銷配對(破壞性:之後必須重新配對)"
if want_row revoke; then
  printf '\n[真機 iPhone] row=revoke\n'
  RESP="$(api DELETE "/v1/mobile/devices/${DEVICE_ID}")"
  say "  daemon 原文:$RESP"
  [[ "$DRY_RUN" == "1" ]] || sleep 1
  say "  撤銷後 devices:$(jget "$(api GET /v1/mobile/status)" 'json.dumps(d.get("devices"), ensure_ascii=False)')"
  say "  期望:App 立即顯示「配對已被撤銷或過期,請重新配對」(請以手機畫面為準並截圖)。"
fi

step "完成"
say "本腳本只負責把 daemon 的原始回覆攤開來,**沒有**做任何通過/不通過的判定。"
say "要把結果寫進 docs/acceptance-evidence.md 時:"
say "  - 每一列標明「真機 iPhone(機型 / iOS 版本)」,和模擬器證據分開放;"
say "  - 只有你親眼在手機上看到/摸到的效果才可以寫成已驗證,ack 本身不算;"
say "  - 沒跑到或結果未知的列,誠實寫 uncertain / 未涵蓋。"
