#!/usr/bin/env bash
# 架構檢查單一入口：可列出清單，也可逐組實跑。
#
#   --list        只列出；不執行測試
#   --docs        文件陳述與發布腳本自測
#   --ts          桌面架構、跨語言契約與安全投影
#   --rust        純核心依賴、schema、接收決策、migration、lifecycle
#   --swift       native Swift 模型與共用 fixture；不是 iOS simulator/真機
#   --drills      實跑角色新增/移除、受限裝置、停用/啟用、optional-state
#   --drill-lint  演練腳本的便宜靜態檢查；不代表演練通過
#   --evidence-dir DIR  保存各命令完整輸出及摘要；預設建立獨立暫存目錄
#   無參數        實跑 docs/ts/rust/swift/drills；成本包含隔離 optional-state 編譯
#
# selected 組缺工具/環境即 FAIL；未選組明示 SKIP，不混入已通過。
# --drills 的 optional-state 預設以 committed HEAD 建立隔離演練樹；整合提交後執行。
# 契約：docs/aip/architecture-boundaries.md；歸屬：docs/MAINTAINERS-MAP.md。
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
DESKTOP="apps/interaction-desktop"
RUN_RUST=0; RUN_TS=0; RUN_DOCS=0; RUN_DRILLS=0; RUN_SWIFT=0; RUN_DRILL_LINT=0; LIST_ONLY=0
EVIDENCE_DIR=""
if [[ $# -eq 0 ]]; then
  RUN_RUST=1; RUN_TS=1; RUN_DOCS=1; RUN_DRILLS=1; RUN_SWIFT=1
fi
while [[ $# -gt 0 ]]; do
  case "$1" in
    --list) LIST_ONLY=1 ;;
    --rust) RUN_RUST=1 ;;
    --ts) RUN_TS=1 ;;
    --docs) RUN_DOCS=1 ;;
    --drills) RUN_DRILLS=1 ;;
    --swift) RUN_SWIFT=1 ;;
    --drill-lint) RUN_DRILL_LINT=1 ;;
    --evidence-dir)
      shift
      if [[ $# -eq 0 || -z "$1" ]]; then echo "--evidence-dir 需要路徑" >&2; exit 2; fi
      EVIDENCE_DIR="$1" ;;
    -h|--help) sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "未知參數：$1（用 --help）" >&2; exit 2 ;;
  esac
  shift
done
if [[ "$LIST_ONLY" == "0" && $((RUN_RUST + RUN_TS + RUN_DOCS + RUN_DRILLS + RUN_SWIFT + RUN_DRILL_LINT)) -eq 0 ]]; then
  RUN_RUST=1; RUN_TS=1; RUN_DOCS=1; RUN_DRILLS=1; RUN_SWIFT=1
fi

# ---------------------------------------------------------------- 檢查清單
# 每一列：<組>|<代號>|<檢查的是什麼>|<可執行的證據（測試檔::測試名／命令）>
CHECKS=(
"rust|core-boundaries|純領域 crate 不得長出 transport／runtime 依賴（架構邊界 §1）|tests/e2e/tests/dependency_boundaries.rs::pure_crates_declare_no_transport_or_runtime_dependencies ／ ::pure_crates_do_not_pull_transport_or_runtime_crates_transitively ／ ::the_transitive_check_actually_detects_a_banned_crate"
"rust|schema-drift|golden schema 不漂移，且 schema 與 Rust 常數雙向一致|tests/e2e/tests/golden.rs::golden_aip_schema ／ crates/interaction-aip/src/schema.rs::every_limit_constant_is_published_in_the_schema ／ ::schema_has_all_roots_and_is_stable"
"rust|receive-decisions|三端共用的接收端決策表（Rust 端：產生器＋只讀 JSON 的獨立消費者＋行為）|crates/interaction-session/tests/receive_decision_fixtures.rs::receive_decision_fixtures_match_the_decision_table ／ ::the_decision_table_fixtures_cover_every_branch ／ receive_decisions_from_json.rs::every_receive_decision_fixture_reaches_the_documented_decision"
"rust|snapshot-migration|已發布快照格式的遷移／未來格式不覆寫（相容路徑）|crates/interaction-runtime/tests/character_session_loop.rs::a_v0_6_0_snapshot_is_restored_and_migrated_to_the_current_format ／ ::a_future_format_snapshot_is_kept_untouched ／ ::a_truncated_snapshot_is_quarantined_with_a_new_epoch"
"rust|adapter-lifecycle|宣告式裝置綁定的顯式生命週期：免重啟 rebind、世代、撤銷不復活、有界|crates/interaction-runtime/tests/declarative_session_loop.rs::reenable_rebinds_without_restart ／ ::rebind_generation_rejects_late_callbacks ／ ::revoke_during_rebind_does_not_resurrect ／ ::rebind_timeout_is_bounded_and_honest"
"rust|stop-paths|停用／撤銷／刪除受器都走同一條有界停止路徑，未確認一律 uncertain|crates/interaction-runtime/tests/sensors_loop.rs::emergency_stop_and_stop_all_sensors_agree_about_an_unstoppable_receptor ／ ::revoking_a_provider_stops_its_sensor_source_with_a_target ／ ::deleting_a_high_risk_receptor_asks_its_source_to_stop_first ／ providers_loop.rs::disabling_one_device_never_retracts_the_family_declaration ／ ::retracting_a_declaration_removes_its_capability_semantics"
"rust|semantic-contract|consumer schema、canonical hash、已發布樣本與 null 突變不變量|crates/interaction-session/tests/semantic_contract.rs ／ state_hash_fixtures.rs ／ state_semantics.rs"
"rust|state-applied|協商回執的有界等待、世代與 stale/replay 拒絕|crates/interaction-adapter-declarative/src/state_applied.rs；runtime character_session_loop/declarative_session_loop"
"ts|semantic-contract|SemanticState 接收與 canonical hash、同步/unknown 安全投影|src/test/semantic-state-contract.test.ts ／ canonical-hash.test.ts ／ state-applied-projection.test.ts ／ unresolvedStops.test.tsx"
"ts|entrypoint-switch|host 不依 entrypoint 字串分岔（小樞脫核心的可執行版本）|$DESKTOP/src/test/architecture-no-entrypoint-switch.test.ts"
"ts|adapter-contract|四個內建 adapter 共用同一套生命週期契約與資源清理|$DESKTOP/src/test/adapter-contract.test.ts"
"ts|receive-decisions|接收端決策表的 TypeScript 端讀同一份跨語言 fixture|$DESKTOP/src/test/receive-decision-fixtures.test.ts"
"ts|safety-honesty|一般模式的安全狀態誠實投影：五入口、不外洩技術詞、誠實階梯不鬆動|$DESKTOP/src/test/general-mode-no-technical-terms.test.tsx ／ regressions-v06-general-mode.test.tsx ／ overlay.test.tsx"
"docs|docs-claims|文件對程式碼現況的可驗證陳述必須與 repo 一致（含已發布版本的 canonical 事實）|scripts/tests/docs-claims.sh"
"docs|release-scripts|發布腳本／workflow 自測（語法、關卡誠實、CI 必需 check 清單）|scripts/tests/release-scripts.sh"
"drills|extensions|production 角色新增/移除、受限裝置、adapter 停用/啟用與 optional-state 演練|scripts/drills/character-package.mjs ／ restricted-device.py ／ provider-disable-reenable.sh ／ optional-state.mjs（committed HEAD 隔離樹）"
"drill-lint|syntax|腳本語法與 shell 引用檢查；不是演練執行證據|bash -n ／ node --check ／ Python ast.parse"
"swift|semantic-conformance|實跑 native Swift pure-model 共用 fixtures；不等於 iOS simulator/真機|scripts/tests/semantic-state-swift.sh"
)

print_list() {
  echo "架構檢查清單 @ $ROOT"
  echo
  local group code what how
  for row in "${CHECKS[@]}"; do
    IFS='|' read -r group code what how <<< "$row"
    printf '  [%-5s] %-18s %s\n' "$group" "$code" "$what"
    printf '           └─ %s\n' "$how"
  done
  echo
  echo "分組執行：--rust / --ts / --docs / --swift / --drills / --drill-lint；Swift 是 native 純模型證據"
}

if [[ "$LIST_ONLY" == "1" ]]; then
  print_list
  exit 0
fi

if [[ -z "$EVIDENCE_DIR" ]]; then EVIDENCE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/architecture-evidence.XXXXXX")"; fi
mkdir -p "$EVIDENCE_DIR" || exit 2
EVIDENCE_DIR="$(cd "$EVIDENCE_DIR" && pwd)"
if [[ -e "$EVIDENCE_DIR/summary.tsv" ]]; then echo "證據目錄已含摘要，請使用新目錄：$EVIDENCE_DIR" >&2; exit 2; fi
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
echo "architecture-checks @ $ROOT"
echo "完整輸出：$EVIDENCE_DIR"
git rev-parse HEAD > "$EVIDENCE_DIR/source-sha.txt"
git status --porcelain > "$EVIDENCE_DIR/source-worktree.txt"
echo

GROUP_RESULT=()   # "組名 狀態 說明"
FAILED=0
SKIPPED=0

record() { GROUP_RESULT+=("$1|$2|$3"); [[ "$2" == "FAIL" ]] && FAILED=$((FAILED + 1)); [[ "$2" == "SKIP" ]] && SKIPPED=$((SKIPPED + 1)); return 0; }

# ------------------------------------------------------------------- docs
if [[ "$RUN_DOCS" == "1" ]]; then
  echo "── docs ────────────────────────────────────────────────"
  DOCS_FAIL=0; DOCS_NOTE=""
  for s in scripts/tests/docs-claims.sh scripts/tests/release-scripts.sh; do
    if [[ ! -f "$s" ]]; then
      echo "  ✘ $s 不存在"; DOCS_FAIL=1; DOCS_NOTE="$DOCS_NOTE $s:missing"; continue
    fi
    OUT="$(/bin/bash "$s" 2>&1)"; RC=$?
    printf '%s\n' "$OUT" > "$EVIDENCE_DIR/${s##*/}.log"
    TAIL="$(printf '%s\n' "$OUT" | grep -Ei 'passed|failed' | tail -1)"
    if [[ "$RC" == "0" ]]; then
      echo "  ✔ $s — ${TAIL:-exit 0}"
    else
      echo "  ✘ $s — ${TAIL:-exit $RC}"
      printf '%s\n' "$OUT" | tail -20 | sed 's/^/      /'
      DOCS_FAIL=1
    fi
    DOCS_NOTE="$DOCS_NOTE ${s##*/}:${TAIL:-exit $RC}"
  done
  if [[ "$DOCS_FAIL" == "0" ]]; then record docs PASS "$DOCS_NOTE"; else record docs FAIL "$DOCS_NOTE"; fi
  echo
else
  record docs SKIP "未指定 --docs"
fi

# --------------------------------------------------------------------- ts
if [[ "$RUN_TS" == "1" ]]; then
  echo "── ts ──────────────────────────────────────────────────"
  TS_FILES=(
    "src/test/architecture-no-entrypoint-switch.test.ts"
    "src/test/adapter-contract.test.ts"
    "src/test/receive-decision-fixtures.test.ts"
    "src/test/semantic-state-contract.test.ts"
    "src/test/canonical-hash.test.ts"
    "src/test/state-applied-projection.test.ts"
    "src/test/unresolvedStops.test.tsx"
    "src/test/general-mode-no-technical-terms.test.tsx"
    "src/test/regressions-v06-general-mode.test.tsx"
    "src/test/overlay.test.tsx"
  )
  MISSING=""
  for f in "${TS_FILES[@]}"; do [[ -f "$DESKTOP/$f" ]] || MISSING="$MISSING $f"; done
  if [[ -n "$MISSING" ]]; then
    echo "  ✘ 找不到守門測試檔：$MISSING"
    record ts FAIL "missing:$MISSING"
  elif ! command -v pnpm >/dev/null 2>&1; then
    echo "  · 沒有 pnpm，所選組無法執行（needs-environment，FAIL）"
    record ts FAIL "needs-environment: pnpm 不存在"
  elif [[ ! -d "$DESKTOP/node_modules" ]]; then
    echo "  · $DESKTOP/node_modules 不存在（先 pnpm install），所選組無法執行（needs-environment，FAIL）"
    record ts FAIL "needs-environment: node_modules 不存在"
  else
    OUT="$(cd "$DESKTOP" && pnpm exec vitest run "${TS_FILES[@]}" 2>&1)"; RC=$?
    printf '%s\n' "$OUT" > "$EVIDENCE_DIR/typescript.log"
    TAIL="$(printf '%s\n' "$OUT" | grep -E '^ *(Test Files|Tests) ' | tr '\n' ' ')"
    if [[ "$RC" == "0" ]]; then
      echo "  ✔ vitest（${#TS_FILES[@]} 檔）— ${TAIL:-exit 0}"
      record ts PASS "${TAIL:-exit 0}"
    else
      echo "  ✘ vitest（${#TS_FILES[@]} 檔）— ${TAIL:-exit $RC}"
      printf '%s\n' "$OUT" | tail -30 | sed 's/^/      /'
      record ts FAIL "${TAIL:-exit $RC}"
    fi
  fi
  echo
else
  record ts SKIP "未指定 --ts"
fi

# ------------------------------------------------------------------- rust
if [[ "$RUN_RUST" == "1" ]]; then
  echo "── rust ────────────────────────────────────────────────"
  if ! command -v cargo >/dev/null 2>&1; then
    echo "  · 沒有 cargo，所選組無法執行（needs-environment，FAIL）"
    record rust FAIL "needs-environment: cargo 不存在"
  else
    RUST_FAIL=0; RUST_NOTE=""
    run_cargo() {
      local label="$1"; shift
      OUT="$("$@" 2>&1)"; RC=$?
      printf '%s\n' "$OUT" > "$EVIDENCE_DIR/rust-$label.log"
      TAIL="$(printf '%s\n' "$OUT" | grep -E '^test result:' | tr '\n' ' ')"
      if [[ "$RC" == "0" ]]; then
        echo "  ✔ $label — ${TAIL:-exit 0}"
      else
        echo "  ✘ $label — ${TAIL:-exit $RC}"
        printf '%s\n' "$OUT" | tail -25 | sed 's/^/      /'
        RUST_FAIL=1
      fi
      RUST_NOTE="$RUST_NOTE $label:${TAIL:-exit $RC}"
    }
    run_cargo "core-boundaries" cargo test -p interaction-e2e --test dependency_boundaries
    run_cargo "schema-drift(golden)" cargo test -p interaction-e2e --test golden
    run_cargo "schema-drift(aip)" cargo test -p interaction-aip
    run_cargo "receive-decisions" cargo test -p interaction-session \
      --test receive_decision_fixtures --test receive_decisions_from_json --test receive_decisions
    run_cargo "semantic-contract+published" cargo test -p interaction-session \
      --test semantic_contract --test state_hash_fixtures --test state_semantics
    run_cargo "state-applied-bounds" cargo test -p interaction-adapter-declarative state_applied --lib
    run_cargo "snapshot-migration+adapter-lifecycle" cargo test -p interaction-runtime \
      --test character_session_loop --test declarative_session_loop
    run_cargo "stop-paths" cargo test -p interaction-runtime --test sensors_loop --test providers_loop --test sensor_journal_review
    if [[ "$RUST_FAIL" == "0" ]]; then record rust PASS "$RUST_NOTE"; else record rust FAIL "$RUST_NOTE"; fi
  fi
  echo
else
  record rust SKIP "未指定 --rust"
fi

# ------------------------------------------------------------------ drills
# 演練腳本（`scripts/drills/*.sh`）不在 CI、也不會有人每天跑，最可能的死法是**安靜地腐爛**：
# 它引用的 YAML／模擬器／manifest 被改名，腳本卻還躺在那裡看起來很正常。這一組只做便宜的
# 靜態檢查（語法＋引用到的檔案與端點還在），**不實跑**——實跑要真 daemon 與 pty 模擬器。
# 靜態通過**不代表**演練還走得完。
if [[ "$RUN_DRILL_LINT" == "1" ]]; then
  echo "── drill-lint ──────────────────────────────────────────────"
  DRILL_FAIL=0; DRILL_NOTE=""
  DRILL_ERR="$(mktemp)"
  DRILLS=()
  while IFS= read -r f; do DRILLS+=("$f"); done < <(ls scripts/drills/*.sh 2>/dev/null | sort)
  if [[ "${#DRILLS[@]}" -eq 0 ]]; then
    echo "  ✘ scripts/drills/ 下沒有任何 .sh（演練腳本不見了）"
    record drill-lint FAIL "no-drill-scripts"
  else
    for d in "${DRILLS[@]}"; do
      if ! /bin/bash -n "$d" 2>"$DRILL_ERR"; then
        echo "  ✘ bash -n $d — $(head -1 "$DRILL_ERR")"; DRILL_FAIL=1
        DRILL_NOTE="$DRILL_NOTE ${d##*/}:syntax"; continue
      fi
      # 引用到的 repo 檔案：`$ROOT/<path>`（跳過 target/ 的建置產物）與腳本／註解裡寫死的 repo 路徑。
      MISS=""
      while IFS= read -r ref; do
        [[ -z "$ref" ]] && continue
        case "$ref" in target/*) continue ;; esac
        [[ -e "$ref" ]] || MISS="$MISS $ref"
      done < <({ grep -oE '\$ROOT/[A-Za-z0-9._/-]+' "$d" | sed 's#^\$ROOT/##'
                 grep -oE '(apps|crates|scripts|examples|docs|firmware|schemas)/[A-Za-z0-9._/-]+\.[A-Za-z0-9]+' "$d"; } | sort -u)
      # 引用到的 HTTP 端點必須still在 API 的路由表裡。
      while IFS= read -r ep; do
        [[ -z "$ep" ]] && continue
        grep -rq "\"$ep\"" crates/interaction-api/src || MISS="$MISS $ep(route)"
      done < <(grep -oE '/v1/[a-z0-9-]+' "$d" | sort -u)
      if [[ -n "$MISS" ]]; then
        echo "  ✘ $d 引用的東西不存在：$MISS"; DRILL_FAIL=1
        DRILL_NOTE="$DRILL_NOTE ${d##*/}:missing"
      else
        echo "  ✔ $d — bash -n 通過，引用的檔案與端點都在（未實跑）"
        DRILL_NOTE="$DRILL_NOTE ${d##*/}:ok"
      fi
    done
    LINT_COUNT=${#DRILLS[@]}
    for d in scripts/drills/*.mjs; do
      LINT_COUNT=$((LINT_COUNT + 1))
      if node --check "$d" >"$DRILL_ERR" 2>&1; then
        echo "  ✔ node --check ${d}（未實跑）"
      else
        echo "  ✘ node --check $d"; cat "$DRILL_ERR"; DRILL_FAIL=1
      fi
    done
    for d in scripts/drills/*.py; do
      LINT_COUNT=$((LINT_COUNT + 1))
      if python3 -c 'import ast, pathlib, sys; ast.parse(pathlib.Path(sys.argv[1]).read_text())' "$d" >"$DRILL_ERR" 2>&1; then
        echo "  ✔ Python syntax ${d}（未實跑）"
      else
        echo "  ✘ Python syntax $d"; cat "$DRILL_ERR"; DRILL_FAIL=1
      fi
    done
    if [[ "$DRILL_FAIL" == "0" ]]; then
      record drill-lint PASS "${LINT_COUNT} 支腳本靜態檢查通過（未實跑）:$DRILL_NOTE"
    else
      record drill-lint FAIL "$DRILL_NOTE"
    fi
  fi
  rm -f "$DRILL_ERR"
  echo
else
  record drill-lint SKIP "未指定 --drill-lint"
fi

# ------------------------------------------------------------------ execution
# Full child logs stay on disk; a missing runtime returns nonzero and never becomes a pass.
run_logged() {
  local label="$1"; shift
  local logfile="$EVIDENCE_DIR/$label.log"
  echo "  RUN $label — $logfile"
  local started="$SECONDS"
  "$@" >"$logfile" 2>&1
  local rc=$?
  echo "  $label: exit=$rc elapsed=$((SECONDS - started))s"
  if [[ "$rc" -ne 0 ]]; then tail -25 "$logfile" | sed 's/^/    /'; fi
  return "$rc"
}

if [[ "$RUN_SWIFT" == "1" ]]; then
  echo "── swift（native 純模型；不是 iOS simulator 或真機）──────"
  if run_logged swift-native bash scripts/tests/semantic-state-swift.sh; then
    SWIFT_NOTE="$(tail -1 "$EVIDENCE_DIR/swift-native.log")"
    echo "  $SWIFT_NOTE"
    record swift PASS "${SWIFT_NOTE}；native macOS 純模型，iOS XCTest 不在本組"
  else
    record swift FAIL "native Swift runner 未通過；環境與錯誤見 $EVIDENCE_DIR/swift-native.log"
  fi
else
  record swift SKIP "未指定 --swift；不以測試檔存在當成通過"
fi

if [[ "$RUN_DRILLS" == "1" ]]; then
  echo "── drills（production 路徑＋模擬器／隔離樹）───────────────"
  DRILL_FAIL=0
  run_logged drill-character-package node scripts/drills/character-package.mjs || DRILL_FAIL=1
  run_logged drill-restricted-device python3 scripts/drills/restricted-device.py --evidence-dir "$EVIDENCE_DIR/restricted-device" || DRILL_FAIL=1
  # Build the exact checkout before the CLI drill, rather than trusting an old binary.
  if run_logged drill-current-cli cargo build -p interaction-cli --message-format=json; then
    # Cargo's artifact path respects workspace/user target-dir settings.
    if DRILL_CLI="$(python3 - "$EVIDENCE_DIR/drill-current-cli.log" <<'PYCLI'
import json, pathlib, sys
paths = []
for line in pathlib.Path(sys.argv[1]).read_text().splitlines():
    try:
        item = json.loads(line)
    except json.JSONDecodeError:
        continue
    if item.get("reason") == "compiler-artifact" and item.get("target", {}).get("name") == "interact-ai" and item.get("executable"):
        paths.append(item["executable"])
if not paths:
    raise SystemExit("current CLI artifact path was not reported by Cargo")
print(paths[-1])
PYCLI
    )"; then
      run_logged drill-provider-lifecycle env INTERACT_AI_BIN="$DRILL_CLI" bash scripts/drills/provider-disable-reenable.sh --output-dir "$EVIDENCE_DIR/provider-lifecycle" || DRILL_FAIL=1
    else
      echo "  FAIL provider-lifecycle 未執行：找不到目前 CLI artifact"; DRILL_FAIL=1
    fi
  else
    echo "  FAIL provider-lifecycle 未執行：無法建置目前 CLI"; DRILL_FAIL=1
  fi
  # The extension/mutation exercise archives committed HEAD by default.
  run_logged drill-optional-state node scripts/drills/optional-state.mjs || DRILL_FAIL=1
  if [[ "$DRILL_FAIL" == "0" ]]; then
    record drills PASS "四個 runner 實跑，覆蓋五項 N5 演練；per-run 日誌與資料保留結果在 $EVIDENCE_DIR"
  else
    record drills FAIL "至少一個實際演練失敗；完整輸出在 $EVIDENCE_DIR"
  fi
else
  record drills SKIP "未指定 --drills；靜態 lint 不代表演練通過"
fi

# ----------------------------------------------------------------- summary
echo "── 摘要 ────────────────────────────────────────────────"
printf 'group\tstatus\tnote\n' > "$EVIDENCE_DIR/summary.tsv"
for row in "${GROUP_RESULT[@]}"; do
  IFS='|' read -r g s n <<< "$row"
  printf '  %-10s %-4s %s\n' "$g" "$s" "$n"
  printf '%s\t%s\t%s\n' "$g" "$s" "$n" >> "$EVIDENCE_DIR/summary.tsv"
done
if [[ "$FAILED" -gt 0 ]]; then
  echo "architecture-checks: $FAILED 組 FAIL、$SKIPPED 組未選取；缺環境與失敗均未計通過"
  exit 1
fi
echo "architecture-checks: 所選組全部通過；$SKIPPED 組未選取（不是通過）；證據 $EVIDENCE_DIR"
