# 維度 I — 測試責任矩陣與精簡處置

**範圍**：`/Users/user/Workspace/claude-lab/adaptive-interaction`，branch `phase-0/state-recovery-baseline`，HEAD `78dcda1`（= origin/main）。
**證據層級（本份報告全體）**：`static-inspection`。本輪**沒有執行任何測試**（依指示避開 cargo／pnpm）。下面所有數字都是「原始碼裡宣告的測試屬性／案例數」，**不是 pass 數**，也不證明它們曾在本機或 CI 綠過。凡是 `it.each`／`for` 迴圈裡的斷言，實際 runner 報出的案例數會比宣告數多。

---

## 0 方法

1. 先用 `grep -c '#\[test\]|#\[tokio::test\]'`（Rust）、`^\s*it(|^\s*test(`（vitest／Playwright）、`func test`（XCTest）盤點資產。
2. 再對每個「重要行為」從**正式入口**回溯：CLI `crates/interaction-cli/src/{main,commands}.rs` → HTTP `crates/interaction-api/src/{lib,routes}.rs` → runtime `crates/interaction-runtime/src/*.rs`，並找出哪一層有測試。
3. 執行觸發條件以 `.github/workflows/{ci,release}.yml`、`scripts/release-verify.sh`、`scripts/tests/architecture-checks.sh`、`scripts/tests/phase0/baseline.sh` 的**實際命令**為準，不採信文件宣稱。
4. 低價值判定只採「我讀過該段原始碼並能指出它在什麼情況下會誤綠／誤紅」的項目。

---

## 1 測試資產盤點

### 1.1 Rust（workspace）

| 位置 | 檔數 | 宣告測試函式 | 備註 |
|---|---:|---:|---|
| `crates/*/src`＋`adapters/*/src`（單元） | — | **404** | 依 crate：runtime 75、character 45、adapter-declarative 43、core 41、recipe 30、session 27、registry 23、agent-gateway 22、aip 19、storage 15、api 13、cli 13、policy 10、tool-schema 8、adapter-sdk 4、events 2、character-shu 0 |
| `crates/interaction-runtime/tests` | 23 | **449** | 最大宗；`agents_loop` 23、`gateway_loop` 28、`character_session_loop` 36、`sensors_loop` 32、`providers_loop` 29、`mobile_loop`（≈70）、`declarative_session_loop` 2696 行 |
| `crates/interaction-adapter-declarative/tests` | 8 | **124** | `protocol_honesty` 33、`esp32_sim_conformance` 23、`aip_fragment`/`aip_link` 各 17 |
| `crates/interaction-session/tests` | 10 | **121** | `session.rs` 49、`session_hardening` 17、`pure_functions` 15 |
| `crates/interaction-character/tests` | 6 | **85** | `gateway.rs` 37、`manifest` 18、`negotiation` 16 |
| `crates/interaction-api/tests` | 3 | **39** | `api_e2e` 32、`consent_e2e` 4、`providers_e2e` 2 |
| `crates/interaction-cli/tests` | 2 | **18** | `cli_e2e` 8、`release_provenance` 10 |
| `crates/interaction-aip/tests` | 2 | **18** | 跨語言 canonical/hash 向量 |
| `crates/interaction-character-shu/tests` | 2 | **7** | |
| `tests/e2e/tests` | 3 | **13** | `golden` 8、`dependency_boundaries` 3、`builtin_whitelist_consistency` 2 |
| `crates/interaction-recipe/tests` | 0 | **0** | 只有 src 內 30 個單元測試 |
| **workspace 合計** | | **≈1,278** | |
| `apps/interaction-desktop/src-tauri/src`（**不在 workspace**） | 10 | **78** | `lib.rs` 29、`character_store` 15、`preset_service` 11、`host_safety` 9、`supervisor` 6、`tray` 5、`settings_recovery_review` 2、`character_package_drill` 1 |

### 1.2 前端 vitest

- `apps/interaction-desktop/src/test/`：**94 個測試檔**，宣告 **≈1,629** 個 `it(`／`test(`（`it.each` 展開後更多）。
- `vitest.config.ts:include = ["src/**/*.test.tsx","src/**/*.test.ts"]`；`src/` 下沒有 `test/` 以外的測試檔，所以 `pnpm test` = 這 94 檔。
- 前十大：`regressions-run2-companion` 60、`regressions-phase7` 54、`connectPage` 52、`session-client` 51、`statusProjection-session` 48、`character-protocol` 43、`character-gateway` 42、`regressions-v05` 38、`rig` 38、`companion-gateway-wiring` 37。
- **13 個檔名以 `regressions-` 開頭**（≈330 案例，約占五分之一），依「對抗審查輪次」而非依行為分檔。

### 1.3 Playwright（真 daemon＋fixture agent）

`apps/interaction-desktop/e2e/`：14 個 spec、**92** 個 `test(`。

| spec | 案例 | 行數 | 主題 |
|---|---:|---:|---|
| `general-mode-tasks.spec.ts` | 20 | 1641 | 一般模式 14 項任務＋7 項可及性 |
| `evidence.spec.ts` | 15 | 751 | 截圖證據矩陣（13 次 `screenshot()`、107 個 `expect`） |
| `app.spec.ts` | 15 | 365 | 首次設定、五入口、進階模式、estop |
| `work-delegate.spec.ts` | 7 | 358 | 交辦／取消／誠實階梯 |
| `a11y.spec.ts` | 6 | 208 | 鍵盤、SR 名稱、reduce-motion、主題 |
| `narrow.spec.ts` | 5 | 125 | 390px |
| `character-session` / `home-state` / `iphone` | 4 each | | 角色同步／首頁狀態／模擬 iPhone |
| `estop` / `sensors` / `character` | 3 each | | |
| `agent-not-installed` | 2 | 76 | |
| `offline.spec.ts` | 1 | 12 | |

`playwright.config.ts`：`workers:1`、`fullyParallel:false`、`retries:0`、三段 project（`first-run`=app.spec → `main` → `estop-last`=estop.spec）。單一 daemon、狀態互相污染，所以順序被寫死。

### 1.4 iOS XCTest

12 檔、**163** 個 `func test`（`SessionClientTests` 36、`LifecycleTests` 22、`ProtocolTests` 21、`ReconnectHintTests` 21、`AIPConformanceTests` 17、`ReceiveDecisionConformanceTests` 15、`ConnectionManagerGateTests` 12、`MotionClassifierTests` 8、`CanonicalVectorsTests` 5、`Semantic/StateHash Conformance` 3+3）。由 `scripts/tests/ios-simulator.sh` 在 task-owned simulator 跑，`--expected-count` 預設寫死 **163**。

### 1.5 腳本／走查／演練

| 位置 | 內容 |
|---|---|
| `scripts/v03-cli-e2e.sh` | 689 行、**≈76 個 `check`／`ok` 斷言**；真 daemon＋mock 裝置＋fixture agent |
| `scripts/tests/architecture-checks.sh` | **18 條具名檢查**、6 組（rust／ts／docs／swift／drills／drill-lint）；`CHECKS[]` 本身就是一份既有的「檢查什麼 → 可執行證據」責任表 |
| `scripts/tests/docs-claims.sh`、`release-scripts.sh` | 文件宣稱與發布腳本自測 |
| `scripts/tests/tauri-*.py`（4 支） | 原生 macOS AX 走查：mobile-sensor（510 行）／work-cancel（355）／preset-recovery（211）／settings（192）；agent 一律 fixture |
| `scripts/tests/ios-simulator.sh`＋`xctest-result.py`＋`test_xctest_result.py` | iOS 模擬器 runner 與其自測 |
| `scripts/tests/semantic-state-swift.sh` | native Swift 純模型 |
| `scripts/drills/` | `character-package.mjs`、`optional-state.mjs`、`restricted-device.py`、`provider-disable-reenable.sh`→`provider-lifecycle.py`、`remove-package-keep-user-data.sh` |
| `scripts/tests/phase0/`（**未追蹤，本日新增**） | `baseline.sh`、`agent_smoke.py`、`multi_session.py`、`restart_test.py`、`archive-evidence.py`、`run-step.sh` — 目前唯一的 real-agent harness |

### 1.6 執行觸發現況（誰真的會跑）

`.github/workflows/ci.yml` 只有 **4 個 job**：

| CI job | 命令 | 覆蓋 |
|---|---|---|
| `rust` | `cargo fmt --check` / `clippy` / `cargo test --workspace --no-fail-fast` / `build` | workspace 1,278 個測試函式（含 `tests/e2e`） |
| `desktop-backend` | `cargo test --manifest-path apps/interaction-desktop/src-tauri/Cargo.toml` | Tauri 78 |
| `frontend` | `pnpm aip:check` / `typecheck` / `pnpm test` / `build` | vitest 94 檔 |
| `e2e` | `pnpm test:e2e` | Playwright 92 |

`release.yml` 的 `ci-gate` 只要求「ci.yml 定義的每個 job 在該 commit 成功」。`release-verify.sh` 的本機關卡是版本一致／crate 版本無漂移／secret 掃描／`docs-claims.sh`／`release-scripts.sh`／CI check-runs；`--run-tests` 只多跑 fmt+clippy+`cargo test --workspace`+src-tauri+`aip:check`+typecheck+`pnpm test`+build。

**因此，下列全部不在任何 CI job、也不在任何發布關卡裡**：

- `architecture-checks.sh` 的 **6 組 18 條**（其中 rust／ts 兩組是既有 CI 的子集，**docs／swift／drills 三組才是 CI 完全沒有的**）。
- `scripts/v03-cli-e2e.sh`（CLI 驗收，≈76 斷言）。
- iOS XCTest 163 個。
- 4 支 Tauri 原生 AX 走查——**唯一**能碰到 Tauri IPC、可信 overlay、原生檔案選擇器、tray 的執行路徑。
- 5 支 drills、`firmware/esp32-companion/compile.sh`、`pnpm perf`。

---

## 2 專業測試責任矩陣

### 2.1 安全／權限不變量（最高風險）

| 行為 | 風險與影響 | 要保護的不變量 | 層級 | 現有案例 | 缺口 | 環境選擇理由與診斷 | 觸發 |
|---|---|---|---|---|---|---|---|
| **Policy 有效值 = min** | 任一上限被略過 → AI 要到超過使用者偏好／裝置安全上限的強度或時長，實體傷害 | `effective = min(AI 請求, 偏好, session 限制, 裝置上限, 剩餘預算)`（`interaction-policy/src/lib.rs:5-8`） | unit（governor 是確定性純函式） | `interaction-policy/src/lib.rs::effective_limit_is_min_of_all_caps`(603)、`::session_budget_clamps_then_blocks`(779)、`::pattern_bounding_clamps_steps_and_magnitude`(830)、`::cooldown_and_hourly_limits`(733)、`::quiet_hours_block_audio_but_not_conversation`(706)、`::unlisted_actuator_is_blocked`(642)、`::autonomous_respects_initiative`(815) | 只有 **10** 個測試守 5 條上限的交叉組合；沒有 property/組合式測試；`min_opt_f64/u64/u32`(495-521) 的 None/NaN 邊界無獨立測試 | unit 最快最準，daemon 只會拖慢並掩蓋。診斷：斷言訊息直接印 `result.effective` 與五個輸入上限 | CI `rust` |
| **consent 不可由 AI 授予** | AI 自授同意 → 所有高風險動作的閘門失效 | agent／session token 不得 grant-consent／clear-estop／mutate-human-state | contract（授權住在 token scope，不是純函式） | `api_e2e.rs::agent_token_cannot_grant_consent_clear_estop_or_mutate_human_state`(167)、`::only_a_human_token_can_verify_a_claim_and_no_agent_can_self_upgrade`(1238)、`::mobile_routes_are_human_only_for_agent_and_session_tokens`(1058)、`::character_session_routes_are_human_only`(2705) | **CLI 端無對應測試**：`interaction-cli/tests` 18 案沒有一個驗「CLI 用 agent token 不能授予同意」。one-shot 耗用覆蓋良好（9+4 案），但「誰能授予」只有 HTTP 一端 | 必須真 HTTP＋真 token，mock 無法證明中介層。診斷：斷言 HTTP status＋error code，不看文案 | CI `rust` |
| **estop 不自動恢復、不謊稱全停** | 重啟自動解除 → 已停的東西又動了；謊稱全停 → 使用者停止求救 | 重啟不自動清除；未確認 actuator 不計入「全停」；高風險能力不自動恢復 | integration（跨 runtime／registry／journal／持久層） | `estop_parallel.rs::an_emergency_stop_with_unconfirmed_actuators_never_claims_every_output_halted`(166)、`::the_actuator_phase…bounded_and_parallel`(53)、`::the_sensor_phase…bounded_and_honest`(366)；`character_session_loop.rs::an_engaged_emergency_stop_is_replayed_into_the_session_on_startup`(1858)；`providers_loop.rs::operational_provider_does_not_auto_recover_after_restart`(297)、`::tested_evidence_survives_restart_but_never_re_arms_the_device`(490)、`::a_revoked_declarative_device_stays_off_across_a_restart`(730)；`mobile_loop.rs::phone_connecting_during_estop_receives_emergency`(2468)、`::estop_never_counts_mobile_actuators_when_no_phone_received_stop_all`(3663)；`agents_loop.rs::estop_cancels_all_open_sessions_and_blocks_new_ones`(428) | 沒有「estop 中直接重啟 daemon，斷言仍 engaged 且無 actuator 被重新 arm」的**單一端到端**測試（被拆成 session replay ＋ provider not-auto-recover 兩半）。`estop.spec.ts` 不重啟 daemon | 需真 runtime＋真 SQLite：持久性正是要驗的東西。診斷：比對重啟前後 `status.emergencyStop` 與每個 provider `state` | CI `rust`＋`e2e` |
| **agent 誠實階梯 claimed≠verified** | 把 agent 自述當事實 → 使用者以為工作完成 | queued≠completed；acknowledged≠completed；completed≠verified；未知一律 `uncertain` | integration(fixture 子程序)＋contract(HTTP)＋unit(前端投影) | `agents_loop.rs::delegation_honesty_ladder_dispatched_acknowledged_claimed`(49)、`::human_verify_is_the_only_path_from_claim_to_verified`(608)、`::a_claim_never_auto_upgrades_itself_to_verified`(1143)、`::agent_session_state_taxonomy_is_emitted_for_every_rung_of_the_ladder`(681)、`::human_verification_binds_to_one_claim…`(940)；`gateway_loop.rs::process_that_ends_without_a_claim_is_unknown_and_only_a_real_error_is_failed`(624)、`::working_never_precedes_the_task_actually_reaching_the_agent`(703)、`::a_second_turn_that_dies_is_not_covered_by_the_first_turns_claim`(1286)；`api_e2e.rs::an_unknown_outcome_is_reportable_and_can_never_be_verified`(1336)；`src/test/honesty.test.ts`(8)；`e2e/work-delegate.spec.ts:252/298` | **fixture 失敗模式是有限集**：`fake_claude.sh` 只有 silent／crash／deaf／hang／claim-then-crash／claim-then-silent；真 agent 的部分輸出、串流中斷、rate-limit、context 溢出無 fixture。`GatewayEvent::TaskWaitingForInput` 沒有任何 connector 產生 → **waiting-for-input 的真實產生路徑零覆蓋** | fixture 是對的預設（真 agent 花錢、慢、不確定）；缺一個低頻真跑（`phase0/agent_smoke.py`）。診斷：斷言 state 序列＋事件序列，不看文案 | CI `rust`＋`e2e`；real-agent **不在 gate** |
| **lease 到期即失效** | 過期 session 仍能用能力 → 授權無限期延長 | 到期 → 撤銷能力、拒絕續期、發 timed-out | integration | `agents_loop.rs::lease_expiry_kills_capabilities_and_refuses_renewal`(390)、`::lease_expiry_emits_timed_out_and_revokes_the_session_capability`(780)；`session_hardening.rs`(17) | 沒有「lease 到期時子程序也被殺」的測試（只驗到能力撤銷，程序樹留存與否未斷言） | fixture 子程序即可。診斷：`state==TimedOut`＋`renew` 回錯 | CI `rust` |
| **取消要殺整棵程序樹** | 只殺 leader → 子程序繼續改檔案／燒額度 | 取消／estop 後 process group 死亡，不因 leader 先結束而漏殺 | integration（真 fork/signal，不可 mock） | `agent-gateway/src/process.rs::kill_tree_escalates_to_group_even_when_leader_exits_in_grace`(281)、`::kill_tree_signals_group_even_after_leader_reaped`(301)、`::process_group_terminate_kills_sigterm_ignoring_members`(324)、`::delegated_process_never_inherits_runtime_capability_tokens`(229)；`gateway_loop.rs::estop_terminates_sessions_concurrently`(2095)、`::an_interrupted_codex_turn_is_cancelled_not_claimed_completed`(1772) | 只在 Unix；Windows 路徑零測試（`process.rs` 用 `libc`）。真 agent 自己 spawn 的子程序沒有實測 | 必須真程序。診斷：`pid_alive()` 輪詢＋逾時訊息 | CI `rust`(Linux)；real-agent 取消只在 `phase0/agent_smoke.py --cancel-when-active`，**不在 gate** |
| **重啟後未完成工作是 unknown** | 重啟把進行中講成完成／失敗 | 開著的 session 不跨重啟存活；未完成回報 unknown | integration | `agents_loop.rs::restart_reports_unknown_for_work_that_was_still_open`(812)、`::open_sessions_do_not_survive_restart`(491)；`sensors_loop.rs::restart_retains_unresolved_without_claiming_active_capture`(1602)、`::shutdown_persists_unknown_before_marking_the_process_clean`(1803) | 全是**優雅重啟**（drop 再 start）；SIGKILL／斷電只有 `phase0/restart_test.py`（不在 gate）。硬殺後的孤兒子程序零自動化覆蓋 | 真 SQLite＋真 home，in-memory 會讓測試失效 | CI `rust`（優雅）；硬殺**不在 gate** |
| **session 隔離** | 兩 session 串線 → A 的輸出／mailbox 進到 B，或取消 A 連帶殺 B | 各自獨立 workdir／mailbox／provider thread／token；取消一個不影響其他 | integration | 部分：`gateway_loop.rs::estop_terminates_sessions_concurrently`(2095)、`::workdir_never_exposes_the_runtime_state_dir`(2258)、`::a_symlink_cannot_smuggle_the_runtime_state_dir_in_as_a_workdir`(2592)、`::resume_cannot_widen_scope`(2157)；`api_e2e.rs::an_agent_session_can_fetch_its_own_mailbox…`(2361)、`::interrupt_requires_session_ownership_or_a_human_token`(2229) | **明確缺口**：全 repo 沒有測試同時開兩個獨立 session 並斷言 (a) A 的 mailbox 不會被 B fetch、(b) 取消 A 後 B 仍 active、(c) A 的輸出不寫進 B 的 record。`estop_terminates_sessions_concurrently` 開兩個，但斷言的是 estop 併發性 | fixture 即可（隔離是 runtime 責任）：各自寫獨一暗號到 `fake-input` 交叉比對。診斷：mailbox 集合互斥＋B 的 state 未變 | **無**（`phase0/multi_session.py` 是唯一 harness，且是 real-agent 版，不在 gate） |
| **mailbox 有界** | 無界 queue → 記憶體爆或 agent 被灌爆 | `maxMessages` 是硬上限；0≠無限 | integration | `agents_loop.rs::message_budget_is_a_hard_ceiling`(278)、`::max_messages_zero_does_not_mean_unlimited`(562)、`::delegation_limits_depth_cycle_and_count`(228)、`::delegation_tree_is_bounded_by_max_parallel…`(581)、`::closed_agent_session_history_is_bounded_and_pruned_from_storage`(1262)、`::retention_never_prunes_a_live_session`(1336) | 好；單則訊息的 byte 上限只在 HTTP 層驗（`api_e2e.rs::oversized_payload_is_rejected` 861） | fixture 即可 | CI `rust` |
| **Context Bundle 內容** | 塞進不該給 AI 的東西 → 隱私外洩；靜默截斷 → AI 拿到殘缺脈絡卻不知情 | session-scoped、決定性；截斷要說出來 | integration | `agents_loop.rs::every_task_receives_and_persists_the_exact_session_scoped_context_bundle`(309)；`memory_loop.rs::context_bundle_is_deterministic_and_honest`(299)、`::context_bundle_reports_capacity_truncation`(422)、`::clear_session_context_clears_beyond_storage_page_limit`(266)；`src/test/regressions-review2-memory.test.tsx` | 沒有「bundle 內不得出現 secret／其他 session 的記憶」的**否定式**測試（`memory_loop.rs::secrets_in_tags_and_provenance_are_rejected` 200 守的是寫入端） | 真 SQLite（決定性靠真排序）。診斷：斷言 bundle canonical 序列化逐位元組相同 | CI `rust` |

### 2.2 狀態、持久化與相容性

| 行為 | 風險與影響 | 不變量 | 層級 | 現有案例 | 缺口 | 環境選擇理由與診斷 | 觸發 |
|---|---|---|---|---|---|---|---|
| **memory TTL／刪除** | 過期記憶仍提供給 AI；使用者刪不掉 | 過期即拒絕且被 sweep；agent 建立的 user-memory 不得超 30 天；刪除要真刪 | integration | `memory_loop.rs::expired_memories_are_pruned_by_sweep`(124)、`::expired_memories_are_refused_before_sweep`(234)、`::far_future_horizon_cannot_escape_candidate_demotion`(144)、`::agent_user_memory_cannot_be_extended_beyond_thirty_days`(544)、`::agent_writes_are_demoted_and_secrets_rejected`(89)、`::handoff_lands_in_memory_with_30d_retention`(593)；`knowledge_loop.rs::deleting_an_asset_disputes_knowledge_and_cascades_derivatives`(227)、`::deleting_source_previews_and_removes_derived_assets_without_silent_orphans`(709) | 沒有「刪除後 SQLite 真的無殘留（含 FTS index、向量表）」的測試，只驗 API 讀不到 | 真 SQLite（TTL 與 cascade 都是 SQL 行為）。診斷：直接查表計數 | CI `rust` |
| **snapshot 遷移** | 舊快照誤判毀損 → session 全丟；未來版被覆寫 → 降版即資料損毀 | 舊格式遷移、未來格式原樣保留、截斷者隔離換 epoch | contract（golden fixture） | `character_session_loop.rs::a_v0_6_0_snapshot_is_restored_and_migrated_to_the_current_format`(2163)、`::a_snapshot_from_before_unsupported_intents_is_migrated_instead_of_quarantined`(2187)、`::a_future_format_snapshot_is_kept_untouched`(2203)、`::a_truncated_snapshot_is_quarantined_with_a_new_epoch`(2223)；fixture `fixture_format0()`(2081)／`fixture_format99()`(2106) | fixture 是**內嵌字串**而非凍結檔案；每多一版就要多一個手寫 fixture，沒有機制強制新增。`semantic-state-contract.test.ts` 的 `releases/v0.7.0/` 凍結 corpus＋`sourceSha` 鎖定是更好的模式 | contract 最適：不需 daemon 但需真舊 bytes。診斷：斷言遷移後 canonical 與 epoch | CI `rust`（`architecture-checks.sh --rust` 重複列為 `snapshot-migration`） |
| **receive 決策表三端一致** | 三端對同封 envelope 做不同決策 → 桌面／手機／runtime 狀態不一致 | 同一份 fixture、同一個決策 | contract（唯一能證明三端一致的層級） | Rust `receive_decision_fixtures.rs::receive_decision_fixtures_match_the_decision_table`、`::the_decision_table_fixtures_cover_every_branch`、`receive_decisions_from_json.rs::every_receive_decision_fixture_reaches_the_documented_decision`、`receive_decisions.rs`(8)；TS `src/test/receive-decision-fixtures.test.ts`(4 個 `it.each`)；Swift `ReceiveDecisionConformanceTests.swift`(15) | **Swift 端不在 CI**（ios-simulator.sh 與 `--swift` 都不在 ci.yml），所以「三端一致」在 PR 上只驗兩端 | 必須跨語言 fixture。診斷：fixture id 逐筆列 diff | Rust/TS 在 CI；**Swift 不在** |
| **canonical JSON／state hash 三端一致** | hash 對不上 → 桌面無法核對 host 狀態，那條防線等於不存在 | 逐位元組相同的 canonical 文字與 SHA-256 | contract | `interaction-aip/tests/canonical_vectors.rs`(6)、`conformance.rs`(12)；`src/test/canonical-hash.test.ts`(16)、`canonical-vectors.test.ts`(7)；`CanonicalVectorsTests.swift`(5)、`StateHashConformanceTests.swift`(3)；凍結 corpus（鎖 `sourceSha 630b4291…`） | 同上，Swift 不在 CI | 全 repo 最好的一組：向量由 Rust 產生、TS/Swift 只准對答案，且有 `code-point-order-not-utf16` 這種「舊實作會綠、新向量才抓得到」的回歸向量 | Rust/TS 在 CI；**Swift 不在** |
| **SSE 重連／Last-Event-ID** | 游標錯 → 事件被吞或舊事件重播（畫面冒出過時綠勾） | 初次連線從目前序號起；同 daemon 續用 lastId；daemon 重啟則重置 | contract(HTTP)＋unit(游標純函式) | `api_e2e.rs::sse_stream_replays_with_last_event_id`(773)；`src/test/regressions-v05-round2.test.tsx:38-88`（`nextStreamCursor` 四案） | **接線本身只有原始碼字串比對**（低價值 I-04）：沒有任何測試用 stub fetch 斷言 `runStream` 真的送出 `Last-Event-ID: <cursor>`；`src/test/transport.test.ts` 已示範怎麼寫（僅 2 案、未涵蓋 SSE） | 純函式 unit；接線用 stub fetch 的 unit 就夠，不必上 E2E。診斷：斷言第一個 fetch 的 headers | CI `rust`＋`frontend` |
| **設定恢復（兩段寫入中斷）** | 套用檔位中途失敗 → 畫面只剩「自訂」讓使用者猜；或用過時意圖覆蓋剛改的設定 | 任一段失敗都要說出來、可只補送第二段；重開後 marker 還在且使用者沒改過才自動補送 | unit(前端投影)＋unit(Tauri 服務)＋native 走查 | `src-tauri/src/preset_service.rs`(11)；`src/test/companion-preset-recovery.test.tsx`(16)、`companion-preset-a11y.test.tsx`(9)、`apply-preset-plan.test.ts`(13)；`scripts/tests/tauri-preset-recovery.py`(211 行，真 App＋隔離 loopback proxy 注入故障) | native 走查**不在 CI**；它是唯一能證明真 Tauri IPC＋真 WebView 這條路走得通的東西 | 故障注入放 loopback proxy（不動 production flag）是對的設計。診斷：marker 檔＋兩端讀回值 | `preset_service` 在 `desktop-backend`；前端在 `frontend`；**native 走查不在** |
| **備份還原** | 還原寫一半卻宣稱成功；使用者以為「備份」＝完整備份 | 拒絕就是一筆都不寫；中途失敗要說已寫入幾筆；不得自稱完整備份 | contract（應該有）＋unit（現況） | `src/test/backupSection.test.tsx`(7 案，全對 `vi.spyOn(api,…)` 的 mock)；`e2e/app.spec.ts:231`（只驗按鈕存在、「記憶與資料」不放第二份） | **矩陣裡最大的結構缺口**。後端只有 `GET /v1/memory/export`（`interaction-api/src/lib.rs:193`、`routes.rs:2135`），**沒有 import／restore 路由**；還原是前端對每筆呼叫 `memoryCreate` 的迴圈。因此 (a) 零 round-trip 測試；(b) `memory_loop.rs:467/506/526` 只驗 scope 與截斷宣告，不驗可還原性；(c) 中途失敗的原子性只在 mock 上驗過 | 應加 contract：真 daemon 種資料 → export → 換 home → 逐筆 create → 斷言 `memory_status` 一致；`scripts/v03-cli-e2e.sh` 是自然的家 | CI `frontend`(mock)＋`e2e`(只驗按鈕) |

### 2.3 裝置、感測與呈現

| 行為 | 風險與影響 | 不變量 | 層級 | 現有案例 | 缺口 | 環境選擇理由與診斷 | 觸發 |
|---|---|---|---|---|---|---|---|
| **iPhone 配對／撤銷即斷線** | 撤銷後手機仍連著 → 已收回的授權還在生效 | 配對碼錯即燒毀；撤銷立即斷線且再也認證不過；斷線＝能力不可用 | integration（真 WebSocket＋TLS） | `mobile_loop.rs::pairing_observation_act_and_estop_loop`(627)、`::wrong_pairing_code_is_refused_and_session_burned`(701)、`::token_reconnect_works_until_revoked`(735)、`::revoke_disconnects_live_connection_immediately`(773)、`::unauthenticated_peer_cannot_resolve_pending_act`(842)、`::idle_connection_times_out_and_capabilities_go_offline`(882)、`::high_risk_receptor_forced_off_on_disconnect_and_stays_off`(930)、`::a_peer_that_burns_the_pairing_window_is_visible_to_the_user`(1721)、`::a_revoke_that_cannot_be_persisted_fails_honestly`(1669)、`::autostart_is_skipped_after_the_last_device_is_revoked`(1175)；`e2e/iphone.spec.ts`(4)；`ConnectionManagerGateTests.swift`(12) | 真機證據只有 `docs/releases/v0.5.0-iphone-device-evidence.md` 逐列標示的那幾筆；BLE peripheral、motion 未涵蓋；iOS 端不在 CI | fixture（`examples/fake_iphone`）是對的預設。診斷：WebSocket 關閉碼＋`mobileStatus` device 狀態 | CI `rust`＋`e2e`；iOS **不在** |
| **感測停止五態誠實投影** | 說「已停止」但沒停 → 使用者以為麥克風關了 | 只有重讀 activeSensors 為空且無不確定才可說已停止；查詢失敗說查詢失敗；歷史未解決要留提醒；不外洩原始 id | unit(投影純函式)＋integration(停止路徑)＋browser | `src/test/sensorStop.test.ts`(18)、`unresolvedStops.test.tsx`(17)、`sensor-stop-history.test.tsx`(1，走真 App→Shell→SensorBanner)；`sensors_loop.rs`(32，含 `::every_stop_outcome_of_a_source_is_reported_and_evented_honestly` 658、`::orphan_ttl_moves_unknown_to_unresolved_not_to_normal` 1158、`::the_stop_sweep_is_bounded_by_the_deadline` 785、6 個 restart 案)；`sensor_journal_review.rs`(5)；`e2e/sensors.spec.ts`(3) | 全 repo 最紮實的一塊。唯一缺口：真麥克風（cpal）只有 `FakeSource`，`adapters/media` 真裝置行為零自動化（合理） | 三層分工正確。診斷：斷言投影的 `ok`／`message` 分支，不是文案全文 | CI `rust`＋`frontend`＋`e2e` |
| **裝置 rebind／撤銷不復活** | 停用後自己接回來 → 使用者關掉的裝置還在動 | 免重啟 rebind、世代拒絕遲到回呼、撤銷中不復活、逾時有界 | integration（真 pty／真 MQTT broker） | `declarative_session_loop.rs::reenable_rebinds_without_restart`(1550) 等四條；`mqtt_rebind_loop.rs::mqtt_reenable_rebinds_without_restart`(175)；`providers_loop.rs::re_enabling_a_declarative_device_rebinds_without_a_restart`(2053)、`::a_legacy_disabled_provider_without_the_off_marker_stays_locked_after_restart`(1042) | **`declarative_session_loop.rs` 在沒有 python3 時整檔靜默變綠**（I-10）。ESP32 真板驗收為零（已知限制） | pty 模擬器是對的：真板不可 CI，但線協定要真的跑過。診斷：模擬器 log＋`members()` | CI `rust`（Ubuntu 有 python3 會跑，但沒有東西保證它跑了） |
| **五入口／不外洩技術詞** | 一般模式漏出 UUID／`provider.mobile.<id>`／revision → 使用者看不懂也洩漏內部識別碼 | 五入口都可達；一般模式 DOM 不得命中技術詞正規式 | unit(渲染後 DOM 掃描)＋browser | `src/test/general-mode-no-technical-terms.test.tsx`(9，掃 `TECHNICAL` 正規式＋`provider.mobile.` 前綴，最後一案反向要求進階模式**要**出現診斷，避免退化成「刪掉就綠」)；`regressions-v06-general-mode`(10)、`regressions-v06-round2-general-mode`(12)、`general-mode-metrics`(15)；`e2e/app.spec.ts:122/245`；`e2e/general-mode-tasks.spec.ts`(14 任務) | 掃描只覆蓋「同步卡＋未解決停止」兩個元件；工作／記憶／活動頁沒有同樣的全文掃描 | unit 掃 DOM 比 E2E 便宜且更全面；E2E 負責「五入口真的到得了」。診斷：印出命中字串 | CI `frontend`＋`e2e` |
| **a11y** | 鍵盤到不了緊急停止／對話框關不掉 → 安全操作對部分使用者不可達 | 第一個 Tab 跳到主要內容；從頁首 Tab 得到緊急停止；Escape 收得掉且焦點不外逃；390px 有名字 | browser（焦點順序只有真瀏覽器算數）＋unit(焦點陷阱) | `e2e/a11y.spec.ts`(6)；`e2e/general-mode-tasks.spec.ts` 的 7 個「可及性：…」(1318–1590)；`e2e/narrow.spec.ts`(5)；`src/test/dialog.test.tsx`(7)、`global-search-a11y.test.tsx`(4)、`companion-preset-a11y.test.tsx`(9) | **沒有任何自動化 a11y 掃描器**（repo 內找不到 axe／jest-axe／@axe-core 的任何引用）。對比度只有一條手寫檢查（`regressions-review3-ia.test.tsx:43`，對 CSS 字面 hex 算 WCAG，只涵蓋 `.companion-sensor-label` 一個 class）。色彩對比、ARIA 正確性、label 關聯、landmark 結構皆無系統性覆蓋 | 焦點與鍵盤必須 browser；靜態規則應用 axe 對五入口各跑一次，成本極低 | CI `e2e`＋`frontend` |
| **角色呈現層沒有權限主權** | Character Pack 改寫安全文字／偽造 verified | 安全語句固定；truthState／verified 只由 Runtime 決定；不支援就誠實降級 | unit＋contract | `src/test/packs.test.ts`(9)；`security_matrix.rs::renderer_capability_spoofing_only_earns_unsupported`(131)、`::every_result_envelope_validates_and_never_claims_verified`(260)、`::the_pipeline_order_is_fixed_identity_before_membership_before_scope`(217)；`interaction-character/tests/negotiation.rs`(16)、`gateway.rs`(37)；`src/test/character-gateway.test.ts`(42)、`adapter-contract.test.ts`(11) | 覆蓋良好 | 純函式＋contract 足夠 | CI `rust`＋`frontend` |

---

## 3 低價值測試盤點

「低價值」＝**維護成本或誤紅風險高於它擋住的缺陷**，或**目標缺陷發生時它不會失敗**。

### 3.1 鎖死非契約的原始碼／CSS 文字

| id | file:line | 判定理由 | 處置 | 替代保護 |
|---|---|---|---|---|
| **I-01** | `src/test/characterPage.test.tsx:778-786` | 讀 `src/styles.css` 原文，用 `expect(block).toMatch(/\.character-cards \{ grid-template-columns: 1fr; \}/)` 鎖死**單行格式**（對應 `styles.css:743`）。jsdom 不套 CSS，證明不了版面行為；重排即誤紅，class 打錯／被蓋掉照樣綠 | **rewrite**：刪 CSS 文字斷言，改由 `e2e/narrow.spec.ts` 在 390px 量 `boundingBox()`／`getComputedStyle` | 同檔 class 存在性與「無 inline style > 390px」保留；`e2e/narrow.spec.ts::390px：頁面主體不產生水平捲動`(58) |
| **I-02** | `src/test/characterPage-first-screen.test.tsx:639-654` | 同一手法，且**用 styles.css 裡兩句中文註解當書籤**（`indexOf("角色頁（CompanionPage）")`／`indexOf("H 區塊結束")`），改註解即整段抓不到 | **rewrite**（同 I-01） | 「不得有 transition／animation」有價值，改在 browser 驗 `getComputedStyle().transitionDuration` |
| **I-03** | `src/test/firstSuccess.test.tsx:189-197` | `expect(css).toMatch(/\.first-success \.onboarding-panel \{ padding: 16px 12px 12px; \}/)` 把 padding 數值寫進測試（`styles.css:748`）。改設計就紅、版面壞掉不紅 | **delete** 該行 CSS 斷言（保留 class 與數量斷言） | `e2e/narrow.spec.ts`、`evidence.spec.ts::擷取：每個一級頁（390px）` |
| **I-04** | `src/test/regressions-v05-round2.test.tsx:70-79` | `import TRANSPORT_SOURCE from "../transport.ts?raw"` 後 `toContain('"Last-Event-ID": cursor.lastId')`＋比較 `indexOf` 順序。改名／引號／抽常數都誤紅；「呼叫了 `nextStreamCursor` 卻忽略 `reset`」這種真缺陷抓不到 | **rewrite**：`configureHttp`＋`vi.stubGlobal("fetch")`，斷言第一個請求打 `/v1/status`、第二個請求的 `headers["Last-Event-ID"]`，並模擬 daemon 重啟斷言游標重置 | 同檔 38-68 的純函式案例保留；`src/test/transport.test.ts` 已示範 stub fetch |
| **I-05** | `src/test/general-mode-metrics.test.tsx:219-246` | vitest `readFileSync("e2e/general-mode-tasks.spec.ts")`，切出某支測試的**原始碼文字**斷言 `} else {` 後含 `actual: "not-run"`。測試在測另一個測試的原始碼；e2e 無害重構即誤紅，分類真的寫錯不保證抓得到 | **rewrite**：把分類判定抽成 `e2e/taskMetrics.ts` 的具名函式，改用一般 unit 測那個函式 | `e2e/general-mode-tasks.spec.ts::任務 11` 仍驗後端真的 `cancelled` |
| **I-06** | `src/test/deep-links.test.tsx:245-249` | `expect(app).toContain("onNavigate((t) => goTo(t))")` 鎖死 `App.tsx:257` 的箭頭函式文字；`App.tsx` 另有 6 處等價寫法（`onNavigate={(t) => goTo(t)}`、`onNavigate={goTo}`），改成任一種即誤紅而行為不變 | **rewrite**：`sensor-stop-history.test.tsx:17` 已示範 mock `../desktop` 的 `onNavigate`；改成觸發 mock callback 後斷言 topbar 標題換頁 | 同檔的「Rust emit 的 tab 必須可路由」「⌘K PAGES 表全部可路由」是有價值的盤點式檢查，保留 |
| **I-07** | `src/test/companion-gateway-wiring.test.ts:263-283` | 用 `indexOf` 切三個檔（`CompanionApp.tsx`／`api.ts`／`src-tauri/character_bridge.rs`）的字串片段證明 reducedMotion 三層接線；任一層重構誤紅，中間任一層把值丟掉不會紅 | **rewrite**：改 contract——`api_e2e.rs::character_hello_route_negotiates…`(1879) 加一 case 送 `reducedMotion:true` 斷言協商降級；前端用 spy 斷言參數 | 同檔上半（`h.limits`／`h.locale`／`inputEventFor` 對照／file-drop 不含路徑）是紮實行為測試，保留 |

### 3.2 斷言太弱／鎖內部細節

| id | file:line | 判定理由 | 處置 | 替代保護 |
|---|---|---|---|---|
| **I-08** | `src/test/companion-presets.test.ts:53-58` | `expect(def?.label.length).toBeGreaterThan(0)`、`summary.length > 4`：任何 5 字元字串都會過（含 `"TODO!"`、原始 id）。宣稱守「人看得懂」，實際只守「非空」 | **merge-into** 同檔 `describeCompanionState` 的逐句斷言（29-51），或改成「三檔位 label 互異且不等於 preset id」 | 同檔 `::任何一個欄位不吻合就是「自訂」，且有效值逐項可讀` |
| **I-09** | `src/test/companion-presets.test.ts:76-79` | `expect(Object.keys(mod)).not.toContain("applyCompanionPreset")` 鎖死一個**已刪除的函式名**；此字串永遠不會再出現 → 從此不可能失敗，而改名的第二入口照樣綠 | **rewrite** 成來源盤點（grep `src/` 找 `prefsPatch({ companionExpressiveness`），或 **delete** | 同檔 81-140 的行為測試已完全取代它 |
| **I-15** | `e2e/helpers.ts:56-58` | `PAGES` 把角色頁 label 寫死 `"小樞"`、marker 寫死 `/36 表情預覽/`。換預設角色、或索引載入失敗（導覽改顯示中性的「角色」）會讓所有依賴 `navigateTo` 的 spec 一起失敗，且訊息指向導覽而非真因 | **rewrite**：label 從 `public/characters/index.json` 的 default manifest 讀（helpers 已有 `repoRoot()`），marker 改 `data-testid` | `e2e/character.spec.ts`、`src/test/characterName.test.tsx`(23) 已在守角色名解析 |

### 3.3 永久／靜默 skip（綠燈不代表跑過）

| id | file:line | 判定理由 | 處置 | 替代保護 |
|---|---|---|---|---|
| **I-10** | `crates/interaction-runtime/tests/declarative_session_loop.rs:66-70` | `Fixture::spawn()` 無 python3 時回 `None`，20+ 個測試以 `let Some(..) else { return; }`（342/503/540/582/629/701/776/842/1048/1135/1187/1246/1284/1551/1874/1942/1991/2049…）**直接 return 成功**。此檔 2696 行，涵蓋 `architecture-checks.sh:55` 明列的 adapter-lifecycle 四條關卡。缺 python3 時 `cargo test` 報「ok」，發布關卡看不出差別 | **rewrite** 成 `panic!`。repo 已有此紀律的前例：`interaction-session/tests/state_hash_fixtures.rs:943::a_schema_too_deep_to_walk_panics_instead_of_skipping_silently` | 無。**不得因耗時刪除**——裝置生命週期的唯一自動化證據 |
| **I-11** | `crates/interaction-adapter-declarative/tests/esp32_sim_conformance.rs:66-70` | 同一模式，23 個裝置線協定 v1 一致性測試在無 python3 時全部靜默變綠 | **rewrite**（同 I-10） | 無 |
| **I-17** | `scripts/drills/remove-package-keep-user-data.sh`（整支） | **沒有任何 runner 呼叫它**：`architecture-checks.sh --drills` 只跑 `character-package.mjs`(317)／`restricted-device.py`(318)／`provider-disable-reenable.sh`(337)／`optional-state.mjs`(345)；`--drill-lint` 只做 `bash -n`。卻被 `docs/acceptance-evidence.md:1063`、`v0.7.0-migration.md:21`、`v0.7.0-final-report.md:73` 當成現行驗收證據 | **keep**（內容有價值）但必須**接上 runner**（加進 `--drills`）；在那之前，文件不得列為現行證據 | `src-tauri/src/character_store.rs`(15) 覆蓋 store 層，但不覆蓋「移除後使用者偏好檔位元不變」 |

### 3.4 重複的昂貴 E2E／固定 sleep

| id | file:line | 判定理由 | 處置 | 替代保護 |
|---|---|---|---|---|
| **I-12** | `e2e/app.spec.ts:324-348` | 完整 estop 流程，與 `e2e/estop.spec.ts:37/117/192` 三案重疊。更嚴重的是排程：`playwright.config.ts:25-26` 明講「estop 會撤銷同意、取消工作、停掉感測，放最後才不會污染別人」，於是 estop.spec 排在 `estop-last`；但 app.spec 屬於 **`first-run`（最先跑）**，等於在所有 main 之前就觸發並解除一次 estop。單 worker 序列下這時間直接加在每次 PR | **merge-into** `e2e/estop.spec.ts`（該檔三案涵蓋更多） | `estop.spec.ts` 三案；`dialog.test.tsx::ConfirmButton`；`api_e2e.rs::emergency_stop_via_api`；`cli_e2e.rs::emergency_stop_from_cli_and_stable_exit_codes` |
| **I-13** | `e2e/offline.spec.ts:6-13`（整檔 12 行 1 案） | 與 `e2e/evidence.spec.ts:706::擷取：Runtime 離線（誠實錯誤畫面）` 同一條路徑（指向死埠、斷言誠實錯誤畫面），後者還多截圖。多一個 spec 檔在單 worker 序列裡就多一次 `goto`＋20 秒 timeout 預算 | **merge-into** `evidence.spec.ts:706`，刪 `offline.spec.ts` | `evidence.spec.ts:706`；`statusProjection*.test.ts` 的離線投影案例 |
| **I-14** | `e2e/app.spec.ts:311` | `await page.waitForTimeout(500); // let stored prefs land in the controlled input` — 固定睡眠等後端往返；慢 runner 不夠、快機器浪費，失敗時看不出是逾時還是行為錯 | **rewrite** 成 `expect.poll(() => toggle.isChecked())` 或 `toBeEnabled()` | `evidence.spec.ts` 的 120–750 ms 是截圖前的穩定等待，屬合理用途，不列為缺陷 |

### 3.5 中等信心／需要人決定

| 候選 | 觀察 | 建議 |
|---|---|---|
| **I-18** `e2e/evidence.spec.ts`（15 案、751 行、最高 180 s、13 次 screenshot） | 有 107 個 `expect`，**不是**純截圖；但主要產出是 `docs/assets` 的證據圖，卻與功能回歸一起阻擋每個 PR，是 e2e job 最大的時間項 | 拆成獨立／release-only job，把其中真正的回歸斷言（載入失敗降級、傳輸錯誤、離線）搬回 `app.spec`／`home-state.spec` |
| **I-16** `scripts/tests/ios-simulator.sh:7`（`TASK_EXPECTED_COUNT=163`） | 防靜默 skip 的好意圖，但把測試數量變成契約：新增一個 XCTest 就會失敗直到有人改常數，誤紅方向安全但鼓勵「順手改大」 | 改成下限（`-ge`）＋版本化的棘輪值 |
| 13 個 `regressions-*` 檔（≈330 案） | 每檔檔頭明列對應 finding id，品質高、**不是**低價值；但依審查輪次分檔，使同一行為的測試散在 4 個檔 | **keep**；只做重新歸檔（依行為／頁面），不刪任何案例 |
| 角色 rig／遊玩場純函式測試（≈145 案） | 成本近乎零，不建議刪；只是相對於「角色呈現層沒有權限主權」這個低風險區域投入比例偏高。`rig.test.ts:64::expect(OFFICIAL_36).toHaveLength(36)` 鎖的是產品規格 §7.5 的數字，不是內部細節 | **keep** |

---

## 4 不得因耗時刪除的清單

- **安全**：`interaction-policy/src/lib.rs` 全部 10 案；`api_e2e.rs:167/1238/1058/2705`；`consent_one_shot_loop.rs` 全部；`estop_parallel.rs` 全部；`process.rs` 4 個 kill-tree 案。
- **資料遷移**：`character_session_loop.rs:2163/2187/2203/2223`；`interaction-character/tests/migration_registry.rs`(10)。
- **舊版相容**：`semantic-state-contract.test.ts` 的 `releases/v0.7.0/` 凍結 corpus（鎖 `sourceSha 630b4291…`）；`interaction-aip/tests/fixtures/manifest.json` 全部向量；`providers_loop.rs::a_legacy_disabled_provider_without_the_off_marker_stays_locked_after_restart`。
- **誠實階梯**：`agents_loop.rs` 與 `gateway_loop.rs` 的全部狀態機案例。

---

## 5 最小行動（依投報比）

1. **把靜默 skip 變成硬失敗**（I-10、I-11）：兩處 `python3_available()` 分支改 panic。一行改動，換回 ≈48 個測試的可信度。
2. **補 session 隔離測試**：兩個 fixture session、各自暗號、交叉斷言 mailbox 與取消。矩陣裡唯一「零覆蓋」的安全不變量。
3. **補備份還原 round-trip**（或誠實記錄「沒有 import 路由、還原是前端迴圈」為已知限制）。
4. **把 7 條原始碼／CSS 文字比對改寫成行為測試**（I-01…I-07），每條都有現成的替代寫法在同一個 repo 裡。
5. **合併重複的 E2E**（I-12、I-13）並移除固定 sleep（I-14），順手解掉 app.spec 在 `first-run` 觸發 estop 的排序矛盾。
6. **把 architecture-checks 的 docs／swift／drills 三組接進 CI**；`remove-package-keep-user-data.sh` 接進 `--drills`（I-17）。
7. **加 axe 掃描**到 `a11y.spec.ts` 的五個入口，補上目前完全沒有的靜態 a11y 覆蓋。
