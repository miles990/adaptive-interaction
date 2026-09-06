# N5 乾淨 checkpoint 演練證據

來源：`3b375bde4188e4a039852881d48d82ea9b619ce1`；[來源 SHA](../architecture-clean-checkpoint/source-sha.txt)、[乾淨工作樹紀錄](../architecture-clean-checkpoint/source-worktree.txt)。本摘要只讀既有證據，未重跑測試或修改 repo。

四個 runner 全部 exit 0，覆蓋五項演練。角色新增／移除共用同一次 runner；測試數與時長只計一次。[wrapper 原始輸出](../clean-checkpoint-architecture.txt)；[機器可讀完整指令與事件](n5-final-drill-summary.json)。

| 演練 | 實際結果 | 時長與計數邊界 |
|---|---|---|
| 新增角色包 | Rust 1/0 + TS 2/0（24 filtered） | 與移除共用：wrapper 28s；子命令 27.798s |
| 受限裝置 | 5/0 子命令：Python 4/0 + Rust 4/0 | wrapper 5s；子命令合計 4.852s |
| Provider 停用／啟用 | 4/0 steps，generation 1→2，撤銷跨重啟保留 | runner 2.483s；wrapper 整秒 2s |
| Optional state | 21/0 steps＝15 正向＋6 預期負向 | wrapper 62s；子命令合計 61.123s |
| 移除角色包 | 與新增同一份 Rust 1/0 + TS 2/0 | 共享，不重加 |

架構 wrapper 的 Rust 466、TS／Swift 子集與 optional 重跑均不加成 workspace 全套數字；六支 lint 是靜態檢查，不是六次演練。

## 執行命令

工作目錄：`/Users/user/Workspace/claude-lab/adaptive-interaction`。繼承 `CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=4`。父程序提供的歷史 exact wrapper command：

```bash
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=4 python3 /tmp/adaptive-convergence-20260906/check.py clean-checkpoint-architecture /Users/user/Workspace/claude-lab/adaptive-interaction bash scripts/tests/architecture-checks.sh --docs --ts --rust --swift --drill-lint --drills --evidence-dir /tmp/adaptive-convergence-20260906/architecture-clean-checkpoint
```

正式 clean-checkout 重跑入口位於 repo，不依賴 `/tmp/check.py`；在上述工作目錄使用全新的證據目錄：

```bash
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=4 bash scripts/tests/architecture-checks.sh --docs --ts --rust --swift --drill-lint --drills --evidence-dir <fresh-evidence-directory>
```

## 新增角色包（既有 text adapter）

```bash
node scripts/drills/character-package.mjs
```

來源 SHA：`3b375bde4188e4a039852881d48d82ea9b619ce1`；exit `0`。

Production 路徑：Committed examples/characters/drill-text/manifest.json → production character_store::import -> Tauri host preferences -> Runtime Character Session hello → TS normal renderer factory + renderer safety projection。

負向／紅燈預期：No deliberate red command in this runner; native production-port and TS renderer/page assertions must all pass. Historical regression evidence is separate.

資料保留：Uses the native test own tempdir; existing user home is not loaded. Persists selection and verifies the added package through existing adapter paths; no new privileged adapter is introduced.

清理：Mutable package/preferences/session data belongs to the native test tempdir. This report relies on successful production-test assertions; there is no separate per-run filesystem inventory.

限制：Production Rust ports and TS model tests are separate boundaries, not one native UI E2E. TS host calls are mocked; canvas getContext warnings are present, so this is not pixel-render evidence. No hardware or human task completion claim.

原始證據：[drill-character-package.log](../architecture-clean-checkpoint/drill-character-package.txt)。

## 受限裝置能力與已套用回執

```bash
python3 scripts/drills/restricted-device.py --evidence-dir /tmp/adaptive-convergence-20260906/architecture-clean-checkpoint/restricted-device
```

來源 SHA：`3b375bde4188e4a039852881d48d82ea9b619ce1`；exit `0`。

Production 路徑：Runtime.register_declarative_spec -> DeviceBinding -> DeviceLink -> SerialLink -> pty receiver -> bound receipt -> CharacterSession diagnostics。

負向／紅燈預期：All commands must exit 0 while assertions reject missing/incorrect/stale receipts, keep legacy write-only delivery unconfirmed, degrade no-fragmentation to intent-only, and retain event-only membership.

資料保留：Each run uses only its own temporary Runtime and provider IDs; no user preference, consent or package directory is loaded or rewritten.

清理：Each fixture owns its Runtime tempdir/provider ID/pty process; disable/rebind preserves user choices and uses a new transfer context. Evidence declares per-fixture ownership; no separate OS-wide process cleanup inventory is recorded.

限制：production-adapter+pty-simulator pty/Python simulator only; no ESP32/board acceptance. Fixture opts into aip.applied/1; this does not claim firmware implements it. wire v1.3 optional receipt extension; AIP 1.0 unchanged and legacy v1.x preserved.

原始證據：[restricted-device.json](../architecture-clean-checkpoint/restricted-device/restricted-device.json)、[1.log](../architecture-clean-checkpoint/restricted-device/1.txt)、[2.log](../architecture-clean-checkpoint/restricted-device/2.txt)、[3.log](../architecture-clean-checkpoint/restricted-device/3.txt)、[4.log](../architecture-clean-checkpoint/restricted-device/4.txt)、[5.log](../architecture-clean-checkpoint/restricted-device/5.txt)。

## 停用／重啟用與撤銷後重啟

```bash
env INTERACT_AI_BIN=/Users/user/Workspace/claude-lab/adaptive-interaction/target/debug/interact-ai bash scripts/drills/provider-disable-reenable.sh --output-dir /tmp/adaptive-convergence-20260906/architecture-clean-checkpoint/provider-lifecycle
```

來源 SHA：`3b375bde4188e4a039852881d48d82ea9b619ce1`；exit `0`。

Production 路徑：CLI -> HTTP -> Runtime::transition_provider/revoke_provider -> run_declarative_rebind -> production serial adapter。

負向／紅燈預期：After revoke, a re-enable CLI call is allowed to fail, but its numeric exit is not recorded or asserted. Required negative result is provider state remains revoked and stays revoked after daemon restart. Before rebind completion the immediate reply must be disconnected, not available.

資料保留：Manually disabled non-consent button-box.button stays disabled through two rebinds; independent system.time capability remains unchanged. Generations increase from 1 to 2; old transport closed and session dispatch drained. Provider revoke persists across a fresh daemon process; audit and provider-after-restart JSON are retained outside temporary home.

清理：TemporaryDirectory owns adapter config, home and PTY marker; ExitStack plus finally terminates/waits only tracked simulator/daemon processes. Temporary home is removed; output result, logs, audit and post-restart provider are retained. No independent all-descendant process inventory or final listener count is recorded in this runner.

限制：production CLI/HTTP/serial adapter + PTY simulator, not physical device evidence. No consent grant or emergency unlock; Removed remains experimental. The name human OFF choice means exercising the user-state API path by automation, not human performance evidence. Dynamic isolated port/tempdir arguments are generated internally and are not recorded as a full subprocess argv trace.

原始證據：[result.json](../architecture-clean-checkpoint/provider-lifecycle/result.json)、[provider-after-restart.json](../architecture-clean-checkpoint/provider-lifecycle/provider-after-restart.json)、[audit.json](../architecture-clean-checkpoint/provider-lifecycle/audit.json)、[drill-provider-lifecycle.log](../architecture-clean-checkpoint/drill-provider-lifecycle.txt)。

CLI 建置：`cargo build -p interaction-cli --message-format=json`，記錄 dev profile 0.21s。使用 Cargo 回報的 `/Users/user/Workspace/claude-lab/adaptive-interaction/target/debug/interact-ai`；binary SHA-256 `0b11005e9197c79ffe53da0e89aa4364e3fbcd21111b375fa56de40980a8b2f8`。撤銷後 re-enable 的數字 exit 未記錄；本報告只聲稱 `/state=revoked` 保留至重啟。

## 新增 optional SemanticState 跨語言契約

```bash
node scripts/drills/optional-state.mjs
```

來源 SHA：`3b375bde4188e4a039852881d48d82ea9b619ce1`；exit `0`。

Production 路徑：Pure interaction-session SemanticState -> canonical schema golden -> consumer handling ledger -> aip-codegen generated TypeScript/Swift DTOs → Current state-hash fixtures + frozen v0.7.0 corpus -> Rust contract tests -> TS validators/canonical hash -> native Swift actual pure models。

負向／紅燈預期：Expected nonzero statuses are successful negative controls; they are not six unresolved failures. Runner also checks failure-output anchors for schema/consumer/fixture/null/published corruption.

資料保留：Archive only the clean committed SHA; all optional field/serializer/fixture/generated DTO mutations stay in disposable checkout. Published v0.7.0 corpus is independently pinned; deliberate whitespace mutation fails even after current fixture regeneration. Omission-to-null mutation fails the independent contract after schema/current-fixture regeneration. Dependency node_modules is shared by symlink, so dependency/cache storage is not isolated; the runner does not install dependencies or load user prefs/consent/device state.

清理：finally removes the disposable checkout and its dedicated CARGO_TARGET_DIR; archive removed after extraction. Wrapper exit 0 confirms runner including cleanup returned successfully; no post-cleanup directory listing was recorded.

限制：Rust/TS machine tests and macOS native Swift pure models only; not iOS XCTest, iPhone, board or native UI evidence. Successful child stdout is not retained in this log, only argv, exit and duration; do not infer/add individual Rust/TS/Swift test counts for these 21 steps. Regeneration env overrides come from the byte-matching checkpoint script, not from the event JSON.

原始證據：[drill-optional-state.log](../architecture-clean-checkpoint/drill-optional-state.txt)。

| 負向控制 | 實際 exit | 秒 | 預期 |
|---|---:|---:|---|
| schema-drift-detected | 101 | 13.924 | 非零且 runner guard 符合，PASS |
| missing-consumer-detected | 1 | 0.047 | 非零且 runner guard 符合，PASS |
| missing-fixture-detected | 101 | 7.857 | 非零且 runner guard 符合，PASS |
| null-mutation-independent-contract | 101 | 0.654 | 非零且 runner guard 符合，PASS |
| published-corpus-mutation-detected | 101 | 0.461 | 非零且 runner guard 符合，PASS |
| deleted-dto-field-detected | 1 | 0.041 | 非零且 runner guard 符合，PASS |

所有 21 個實際 argv、環境 override、exit 與毫秒時長完整保留在 JSON；成功子命令原始 stdout 未留存，所以不反推其測試案例數。

## 移除使用中角色包並保留資料

```bash
node scripts/drills/character-package.mjs
```

來源 SHA：`3b375bde4188e4a039852881d48d82ea9b619ce1`；exit `0`。

Production 路徑：production character_store::remove -> fallback pack selection -> host preference persistence → Runtime Character Session state save/restore and fresh runtime restart → TS page ConfirmButton/remove/fallback/remount with modeled host。

負向／紅燈預期：Expected protective outcome: removal goes through production remove, fallback survives restart, unrelated preferences/audit/session remain; this is not the older rm-folder-only script.

資料保留：Production regression named package_import_remove_fallback_preserves_preferences_audit_and_session_after_restart passes. Existing preferences, retained audit and session data are checked across removal/restart. No fabricated package-removal audit event is claimed; preservation of existing audit is the checked invariant.

清理：Only the fixture package/tempdir is modified; saved user data is checked before test teardown. No destructive operation is executed on the user actual package directory.

限制：Uses the same Rust 1/0 plus TS 2/0 run and duration as N5-01; do not double-count. TS page boundary is modeled; parent native UI evidence is outside this summary. The old remove-package-keep-user-data.sh received syntax lint only and is not the production-removal evidence.

原始證據：[drill-character-package.log](../architecture-clean-checkpoint/drill-character-package.txt)。

## 子命令（角色 runner 共用）

```bash
cargo test --manifest-path apps/interaction-desktop/src-tauri/Cargo.toml --lib character_package_drill
pnpm --dir apps/interaction-desktop exec vitest run src/test/character-extension-drill.test.ts src/test/characterPage.test.tsx -t N5
```

## 受限裝置子命令

```bash
python3 scripts/tests/aip-applied-receiver.py
cargo test -p interaction-runtime --test declarative_session_loop negotiated_state_applied_tracks_snapshot_patch_loss_rebind_and_stale_receipts -- --exact
cargo test -p interaction-runtime --test declarative_session_loop legacy_fragmenting_devices_remain_unconfirmed_after_all_writes_succeed -- --exact
cargo test -p interaction-runtime --test declarative_session_loop a_device_without_fragmentation_degrades_to_intent_only -- --exact
cargo test -p interaction-runtime --test declarative_session_loop an_event_only_device_is_an_event_source_member -- --exact
```

## 歷史 findings 與本輪邊界

[既有 findings](/tmp/n1-n4-n6-findings-handoff.json) 中 N1-CONTRACT-01、N1-RESTORE-02、N1-REVIEW-03、N1-REVIEW-04 有已修正的 required-field、null、Attention、日期案例；先前 working-tree 演練留下的「整合提交後必跑 clean HEAD」條件，現在由此 clean-checkout 證據補齊。歷史紅綠數字不加進本輪統計。

本摘要不代表原生 UI、真 AI、真機、人類評估、緊急停止解除或發布完成；native 與 perf 是整合者的另外驗收。
