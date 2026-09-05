#!/usr/bin/env bash
# 架構檢查的單一入口：把散在 Rust／TypeScript／shell 三處的「架構邊界是可執行的」那些測試
# 收成一張清單，並且能分組實跑。
#
#   bash scripts/tests/architecture-checks.sh --list     # 只列出，零成本（不跑任何測試）
#   bash scripts/tests/architecture-checks.sh --docs     # 文件誠實度 lint ＋ 發布腳本自測
#   bash scripts/tests/architecture-checks.sh --ts       # 桌面守門測試（vitest，指定檔）
#   bash scripts/tests/architecture-checks.sh --rust     # 依賴邊界／schema 漂移／決策表／生命週期
#   bash scripts/tests/architecture-checks.sh            # 三組都跑
#
# 誠實：每一組印自己的 PASS／FAIL 與數字；**沒有跑到的組印 SKIP，不算通過**，
# 而且只要有任何一組 SKIP 或 FAIL，收尾就不會寫 "all checks passed"。
# `--rust` 需要編譯整個 workspace（磁碟／時間成本高），在磁碟吃緊的環境請單獨安排。
# swift 那一組在這裡永遠是 SKIP（XCTest 要 iOS 模擬器），但腳本仍會確認它的測試檔與
# 測試名還在——跑不到的東西被刪掉時，這張清單不得看起來一切如常。
#
# 契約：`docs/aip/architecture-boundaries.md`（§1 分層與依賴方向、§2 ports、§3 adapter lifecycle）。
# 能力歸屬與各領域的必要測試：`docs/MAINTAINERS-MAP.md`。
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

DESKTOP="apps/interaction-desktop"

# swift 那一組本腳本跑不到（XCTest 需要 iOS 模擬器）。跑不到不表示不用檢查：
# 至少要確認那份跨語言一致性測試還在，而且還在測我們宣稱它測的那件事。
SWIFT_TEST="apps/interaction-ios/InteractionCompanionTests/ReceiveDecisionConformanceTests.swift"
SWIFT_CASE="testEveryReceiveDecisionFixtureReachesTheDocumentedDecision"

RUN_RUST=0; RUN_TS=0; RUN_DOCS=0; LIST_ONLY=0
if [[ $# -eq 0 ]]; then
  RUN_RUST=1; RUN_TS=1; RUN_DOCS=1
else
  for arg in "$@"; do
    case "$arg" in
      --list) LIST_ONLY=1 ;;
      --rust) RUN_RUST=1 ;;
      --ts)   RUN_TS=1 ;;
      --docs) RUN_DOCS=1 ;;
      -h|--help)
        sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'
        exit 0 ;;
      *)
        echo "未知參數：${arg}（用 --list／--rust／--ts／--docs）" >&2
        exit 2 ;;
    esac
  done
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
"ts|entrypoint-switch|host 不依 entrypoint 字串分岔（小樞脫核心的可執行版本）|$DESKTOP/src/test/architecture-no-entrypoint-switch.test.ts"
"ts|adapter-contract|四個內建 adapter 共用同一套生命週期契約與資源清理|$DESKTOP/src/test/adapter-contract.test.ts"
"ts|receive-decisions|接收端決策表的 TypeScript 端讀同一份跨語言 fixture|$DESKTOP/src/test/receive-decision-fixtures.test.ts"
"ts|safety-honesty|一般模式的安全狀態誠實投影：五入口、不外洩技術詞、誠實階梯不鬆動|$DESKTOP/src/test/general-mode-no-technical-terms.test.tsx ／ regressions-v06-general-mode.test.tsx ／ overlay.test.tsx"
"docs|docs-claims|文件對程式碼現況的可驗證陳述必須與 repo 一致（含已發布版本的 canonical 事實）|scripts/tests/docs-claims.sh"
"docs|release-scripts|發布腳本／workflow 自測（語法、關卡誠實、CI 必需 check 清單）|scripts/tests/release-scripts.sh"
"swift|receive-decisions|接收端決策表的 Swift 端（**需要 iOS 模擬器，本腳本只確認測試還在**）|${SWIFT_TEST}::${SWIFT_CASE}（見 apps/interaction-ios/README.md）"
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
  echo "分組執行：--rust / --ts / --docs（swift 組需要 iOS 模擬器，不在本腳本內）"
}

if [[ "$LIST_ONLY" == "1" ]]; then
  print_list
  exit 0
fi

echo "architecture-checks @ $ROOT"
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
    echo "  · 沒有 pnpm，這一組沒有跑（SKIP，不是通過）"
    record ts SKIP "pnpm 不存在"
  elif [[ ! -d "$DESKTOP/node_modules" ]]; then
    echo "  · $DESKTOP/node_modules 不存在（先 pnpm install），這一組沒有跑（SKIP，不是通過）"
    record ts SKIP "node_modules 不存在"
  else
    OUT="$(cd "$DESKTOP" && pnpm exec vitest run "${TS_FILES[@]}" 2>&1)"; RC=$?
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
    echo "  · 沒有 cargo，這一組沒有跑（SKIP，不是通過）"
    record rust SKIP "cargo 不存在"
  else
    RUST_FAIL=0; RUST_NOTE=""
    run_cargo() {
      local label="$1"; shift
      OUT="$("$@" 2>&1)"; RC=$?
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
    run_cargo "snapshot-migration+adapter-lifecycle" cargo test -p interaction-runtime \
      --test character_session_loop --test declarative_session_loop
    run_cargo "stop-paths" cargo test -p interaction-runtime --test sensors_loop --test providers_loop
    if [[ "$RUST_FAIL" == "0" ]]; then record rust PASS "$RUST_NOTE"; else record rust FAIL "$RUST_NOTE"; fi
  fi
  echo
else
  record rust SKIP "未指定 --rust"
fi

# ------------------------------------------------------------------ swift
# 一律檢查（成本是兩次 grep），而且**不**併進 docs／ts／rust 的執行計數：
# 它在這台機器上永遠跑不到，混進 SKIPPED 只會讓每一次完整執行都看起來像有東西
# 沒跑完。它自己的失敗條件很窄，但很重要——測試檔或測試名不見了的話，一份跨
# 語言一致性保證就這樣消失了，而在這張清單上看起來會跟一直以來一模一樣。
echo "── swift ───────────────────────────────────────────────"
SWIFT_STATE="SKIP"; SWIFT_NOTE=""
if [[ ! -f "${SWIFT_TEST}" ]]; then
  echo "  ✘ 找不到 ${SWIFT_TEST}（Swift 端的決策表一致性測試不見了）"
  SWIFT_STATE="FAIL"; SWIFT_NOTE="missing:${SWIFT_TEST}"; FAILED=$((FAILED + 1))
elif ! grep -q "func ${SWIFT_CASE}" "${SWIFT_TEST}"; then
  echo "  ✘ ${SWIFT_TEST} 裡沒有 func ${SWIFT_CASE}（被改名或刪掉了）"
  SWIFT_STATE="FAIL"; SWIFT_NOTE="missing-case:${SWIFT_CASE}"; FAILED=$((FAILED + 1))
else
  SWIFT_LINE="$(grep -n "func ${SWIFT_CASE}" "${SWIFT_TEST}" | head -1 | cut -d: -f1)"
  echo "  · ${SWIFT_TEST}:${SWIFT_LINE} 的 ${SWIFT_CASE} 還在"
  echo "    需要 iOS 模擬器才跑得到，本腳本沒有跑它（存在 ≠ 通過；見 apps/interaction-ios/README.md）"
  SWIFT_NOTE="${SWIFT_TEST}:${SWIFT_LINE} 在；需要 iOS 模擬器，本腳本未執行"
fi
echo

# ----------------------------------------------------------------- 收尾
echo "── 摘要 ────────────────────────────────────────────────"
for row in "${GROUP_RESULT[@]}"; do
  IFS='|' read -r g s n <<< "$row"
  printf '  %-5s %-4s %s\n' "$g" "$s" "$n"
done
printf '  %-5s %-4s %s\n' swift "${SWIFT_STATE}" "${SWIFT_NOTE}"
echo
SWIFT_TAIL="swift 那一組本腳本跑不到（需要 iOS 模擬器）"
[[ "${SWIFT_STATE}" == "SKIP" ]] && SWIFT_TAIL="${SWIFT_TAIL}；測試檔與測試名已確認還在"
if [[ "$FAILED" -gt 0 ]]; then
  echo "architecture-checks: $FAILED 組 FAIL、$SKIPPED 組未執行；${SWIFT_TAIL}"
  exit 1
fi
if [[ "$SKIPPED" -gt 0 ]]; then
  echo "architecture-checks: 已跑的組全數通過；$SKIPPED 組未執行（未執行 ≠ 通過）；${SWIFT_TAIL}"
  exit 0
fi
echo "architecture-checks: docs／ts／rust 三組全數通過；${SWIFT_TAIL}"
