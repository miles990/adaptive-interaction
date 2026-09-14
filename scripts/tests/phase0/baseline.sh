#!/usr/bin/env bash
# 階段 0 測試基線（依風險順序：快檢 → 相關整合 → 完整基線）。每一步由 run-step.sh 紀錄，
# 第一次失敗保留在 summary.jsonl 與 <name>.stderr，不重跑掩蓋。
#
#   PHASE0_OUT=/path/to/evidence scripts/tests/phase0/baseline.sh [step ...]
#
# 不帶參數＝全部；帶參數只跑指定步驟（名稱見下表）。需要的環境：
#   rust-*／tauri-test／cli-e2e／arch-rust：Rust toolchain（rust-toolchain.toml）
#   fe-*／pw-e2e：pnpm（apps/interaction-desktop 已 pnpm install）；pw-e2e 自起真 daemon（E2E_API_PORT）
#   ios-simulator／arch-swift：Xcode（DEVELOPER_DIR）；缺環境時該步 exit≠0，照實記錄為 needs-environment
#   firmware-*：arduino-cli（見 firmware/esp32-companion/compile.sh）；只是編譯檢查，不是真板驗收
# 真 Agent（Claude Code／Codex）與多 Session 基線不在這裡：見 README.md 的 agent_smoke.py／multi_session.py／restart_test.py。
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && cd .. && pwd)"
S="$ROOT/scripts/tests/phase0/run-step.sh"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}" CARGO_TERM_COLOR=never INTERACT_AI_MOBILE_ADVERTISE=0
export PHASE0_OUT="${PHASE0_OUT:?set PHASE0_OUT}"
E2E_API_PORT="${E2E_API_PORT:-18890}"
DESK="$ROOT/apps/interaction-desktop"
SELECTED=("$@")
# macOS 內建 bash 3.2：空陣列展開要用 ${arr[@]+"${arr[@]}"} 才不會踩到 set -u。
want() { [[ ${#SELECTED[@]} -eq 0 ]] && return 0; for s in ${SELECTED[@]+"${SELECTED[@]}"}; do [[ "$s" == "$STEP" ]] && return 0; done; return 1; }
run() { STEP="$1"; want || return 0; shift; "$S" "$STEP" "$@" || true; }

run rust-fmt            "$ROOT" cargo fmt --all --check
run rust-clippy         "$ROOT" cargo clippy --workspace --all-targets -- -D warnings
run rust-test-workspace "$ROOT" cargo test --workspace --no-fail-fast
run rust-build-cli      "$ROOT" cargo build -p interaction-cli
run rust-build-fake-iphone "$ROOT" cargo build -p interaction-runtime --example fake_iphone
run tauri-test          "$ROOT" cargo test --manifest-path apps/interaction-desktop/src-tauri/Cargo.toml
run fe-typecheck        "$DESK" pnpm typecheck
run fe-test             "$DESK" pnpm test
run fe-build            "$DESK" pnpm build
run arch-docs           "$ROOT" bash scripts/tests/architecture-checks.sh --docs
run arch-drill-lint     "$ROOT" bash scripts/tests/architecture-checks.sh --drill-lint
run arch-ts             "$ROOT" bash scripts/tests/architecture-checks.sh --ts
run cli-e2e             "$ROOT" bash scripts/v03-cli-e2e.sh
run arch-rust           "$ROOT" bash scripts/tests/architecture-checks.sh --rust
run arch-swift          "$ROOT" bash scripts/tests/architecture-checks.sh --swift
STEP=pw-e2e; if want; then E2E_API_PORT="$E2E_API_PORT" "$S" pw-e2e "$DESK" pnpm test:e2e || true; fi
run ios-simulator       "$ROOT" bash scripts/tests/ios-simulator.sh --out "$PHASE0_OUT/ios-sim-out" --build-dir "$PHASE0_OUT/ios-sim-build"
run arch-drills         "$ROOT" bash scripts/tests/architecture-checks.sh --drills --evidence-dir "$PHASE0_OUT/arch-drills-evidence"
run firmware-default    "$ROOT" bash firmware/esp32-companion/compile.sh
run firmware-ble        "$ROOT" bash firmware/esp32-companion/compile.sh --ble
echo "PHASE0_BASELINE_DONE $(date -u +%FT%TZ) → $PHASE0_OUT/summary.jsonl"
