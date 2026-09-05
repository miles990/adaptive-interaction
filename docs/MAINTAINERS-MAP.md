# 能力歸屬對照表（MAINTAINERS MAP）

> 這份表回答一個問題：**「我要動這件事，該去哪裡、不該去哪裡、動完誰會紅？」**
>
> 每一列是一個能力，欄位固定七個：
>
> | 欄位 | 意思 |
> |---|---|
> | **owner** | 這件事的權威實作（crate／模組）。同一件事只有一個 owner；別處看到的是投影或鏡射 |
> | **入口** | 人與 AI 從哪裡碰到它（API／CLI／UI） |
> | **狀態來源** | 這件事的**真相**存在哪裡。畫面上的值一律是它的投影，不得反向寫回 |
> | **公開契約** | 改了會影響外部的那份文件。契約先改，實作再跟 |
> | **擴充點** | 加一個新的（新角色／新裝置／新 transport／新狀態）要動哪裡 |
> | **必要測試** | 動完至少要跑綠的那幾支（`file::name`）。整組跑法見 `scripts/tests/architecture-checks.sh --list` |
> | **已知限制** | 誠實的邊界。詳情在 `CHANGELOG.md` 與 `docs/releases/*-known-limitations.md` |
>
> 入口地圖在 `AGENTS.md`；分層與禁止依賴在 `docs/aip/architecture-boundaries.md` §1；
> 相容路徑的退場計畫在 `docs/aip/deprecation-ledger.md`。

---

## 1. 角色（manifest／adapter／registry）

| | |
|---|---|
| **owner** | `crates/interaction-character`（CPP 1.0 純函式：manifest 驗證、能力協商、intent、receipt、wire）；小樞專屬內容在 `crates/interaction-character-shu`；TS 鏡射 `apps/interaction-desktop/src/character/{manifest,protocol,negotiate,gateway}.ts` |
| **入口** | CLI `interact-ai character adapters add --name X --manifest m.json`／`character status｜instances`；HTTP `GET /v1/character/instances｜adapters`；UI 角色頁（`src/pages/character/`） |
| **狀態來源** | Runtime `crates/interaction-runtime/src/character.rs`（CharacterHub＋真相投影）。**truthState／verified 只由 Runtime 決定**，adapter 與 Character Pack 不能改寫安全文字、不能偽造 verified |
| **公開契約** | `docs/character-protocol/README.md`（唯一契約）＋`adapter-authoring.md`；`docs/aip/character-package.md`、`docs/aip/reference-character.md` |
| **擴充點** | **兩種，代價不同**（`docs/aip/adapter-development.md` §1a／§1b，實測 `docs/releases/v0.7.0-drills.md` §1）：**(a) 加角色（重用既有 adapter）**＝一份 manifest＋`public/characters/index.json` 一列＋一支角色專屬測試；**兩份 host 白名單都不動**——白名單的鍵是 entrypoint，不是角色 id（出貨數量只寫在 `character-manifests.test.ts` 的 `SHIPPED_CHARACTER_IDS`）。**(b) 加 adapter（新 entrypoint）**＝才要動兩份白名單：Rust `interaction_runtime::character::character_host_registry()` 的 `CHARACTER_BUILTIN_ENTRYPOINTS`、TS `src/character/adapterRegistry.ts` 的 `BUILTIN_ADAPTER_IDS`＋`adapters/index.ts` 的工廠，並在 `adapter-contract.test.ts` 加案例。**核心 crate 與頁面不得認得任何角色 id** |
| **必要測試** | `tests/e2e/tests/builtin_whitelist_consistency.rs::the_rust_and_typescript_builtin_whitelists_are_the_same_set`；`crates/interaction-runtime/tests/character_host_registry.rs`；`apps/interaction-desktop/src/test/architecture-no-entrypoint-switch.test.ts`、`character-ref-shape.test.ts`、`character-manifests.test.ts` |
| **已知限制** | `CharacterPreview.tsx`／`CharacterLibrary.tsx` 仍在守門測試的待收斂棘輪清單裡（`architecture-boundaries.md` §4） |

## 2. Renderer（adapter 生命週期）

| | |
|---|---|
| **owner** | TS `apps/interaction-desktop/src/character/adapters/{shu,sprite,text,shape}.ts`（實際的 renderer）；Rust 端 `crates/interaction-session/src/ports.rs::RendererPort` — **port 存在但無 transport 使用**（零 `impl`，見 `deprecation-ledger.md` §1.3） |
| **入口** | 角色視窗（`src/companion/CompanionApp.tsx` 只呼叫 `createBuiltinAdapter(entrypoint)`）；外部 adapter 走 WebSocket `crates/interaction-api/src/character_ws.rs` |
| **狀態來源** | 生命週期外層語意 `Registered → Initializing → Starting → Ready ⇄ Degraded → Stopping → Disposed`（另 `Cancelled`／`Failed`／`Unavailable`／`Removed`），由 CPP 的 14 態經 `statusProjection.projectCharacterLifecycle` 映射 |
| **公開契約** | `docs/aip/renderer-adapter.md`、`docs/aip/architecture-boundaries.md` §3、`docs/character-protocol/adapter-authoring.md` |
| **擴充點** | 新 renderer＝在 `adapterRegistry.ts` 註冊一個工廠並宣告 adapter meta（`personas`／`variants`／`scenes`／`hasPlayfield`／`playfieldControls`）。角色專屬設定 UI 住在 adapter 自己的模組（例：`adapters/shuPlayControls.tsx`），頁面不得認得它 |
| **必要測試** | `apps/interaction-desktop/src/test/adapter-contract.test.ts`（四個 adapter 跑同一套：生命週期順序、unsupported 不冒充 completed、cancel 冪等、dispose 後 timer／rAF／DOM listener 回到原水位） |
| **已知限制** | `RendererPort`／`DevicePort` 是 experimental：介面存在**不代表**這條擴充點可用 |

## 3. Session（權威語意狀態）

| | |
|---|---|
| **owner** | `crates/interaction-session`（純函式）：state／revision／epoch／`src/receive.rs` 接收端決策表／`src/session.rs` 十三關安全管線／`src/patch.rs` RFC 7396＋hash／`src/ports.rs` |
| **入口** | HTTP `GET /v1/character-session`、`POST /v1/character-session/{resume,events}`、`GET /v1/character-session/diagnostics`；SSE `character.session.state`；CLI `interact-ai character session status｜diagnostics｜resume` |
| **狀態來源** | Runtime 是**唯一** Session Host：`crates/interaction-runtime/src/character_session.rs`（真相唯一入口 `submit_runtime`）。成員（桌面／iPhone／宣告式裝置）不擁有共享狀態 |
| **公開契約** | `docs/aip/character-session.md`（§7.2 接收端決策表）、`docs/aip/README.md`（AIP 1.0） |
| **擴充點** | 新的 state reason 值／新 message name 依 `docs/aip/compatibility.md` §2 的 minor 規則；新增決策表分支要同時加跨語言 fixture（`crates/interaction-aip/tests/fixtures/manifest.json` 的 `receiveDecisions`）。**新增任何 `SemanticState` 欄位，必須同時有一份帶著該欄位的 `stateHashes` fixture**——`SemanticState` 不在 golden schema 裡，fixture 是 TypeScript／Swift 唯一會被逼著看到新欄位的東西（`state_hash_fixtures.rs::every_semantic_state_field_appears_in_at_least_one_state_hash_fixture` 會擋） |
| **必要測試** | `crates/interaction-session/tests/receive_decision_fixtures.rs::receive_decision_fixtures_match_the_decision_table`、`::the_decision_table_fixtures_cover_every_branch`；`receive_decisions_from_json.rs::every_receive_decision_fixture_reaches_the_documented_decision`；`security_matrix.rs::the_pipeline_order_is_fixed_identity_before_membership_before_scope`；三端鏡射 `apps/interaction-desktop/src/test/receive-decision-fixtures.test.ts`、`apps/interaction-ios/InteractionCompanionTests/ReceiveDecisionConformanceTests.swift`（需模擬器） |
| **已知限制** | `ConsentVerifier` 刻意不接進 `gate`（fail-closed）；多裝置同時連線同一 session 未覆蓋；iPhone 真機閉環為零 |

## 4. 裝置（宣告式 adapter）

| | |
|---|---|
| **owner** | `crates/interaction-adapter-declarative`：`DeclarativeSpec`（YAML spec）、`src/protocol.rs`（`DeviceLink`、裝置線 v1.x、`admit_aip`、`DeviceAipChannel`）、`serial`／`mqtt`／`ble` 傳輸 |
| **入口** | 設定檔 `~/.adaptive-interaction/config/adapters/*.yaml`；CLI `interact-ai providers …`；UI「連接與權限」 |
| **狀態來源** | 綁定狀態：`crates/interaction-runtime/src/declarative_lifecycle.rs`（`DeclarativeLifecycle::{Bound, Rebinding, Unbound}`＋`UnboundReason`，帶世代）；AIP 接線：`declarative_session.rs`。ProviderState 不得先於實際連線（握手 Ready 之前一律 `Disconnected`） |
| **公開契約** | `docs/aip/device-profile.md`（§3 身分強度、§6 裝置線 v1.1）、`docs/aip/adapter-development.md`、`docs/aip/pairing-security.md` |
| **擴充點** | 新裝置＝一份 YAML spec（不改 Rust）。新傳輸＝實作 `RawLink` 並用同一個 `AipChannel<L>`——Runtime 只認得型別抹除的 `DeviceAipChannel`，**沒有** serial／mqtt／ble 分支 |
| **必要測試** | `crates/interaction-runtime/tests/declarative_session_loop.rs::reenable_rebinds_without_restart`、`::rebind_generation_rejects_late_callbacks`、`::revoke_during_rebind_does_not_resurrect`、`::rebind_timeout_is_bounded_and_honest`；`crates/interaction-adapter-declarative/tests/{aip_link,esp32_sim_conformance}.rs` |
| **已知限制** | ESP32 真板驗收為零（只有 `compile.sh` 編譯檢查與 pty 模擬器）；MQTT／BLE 共用程式碼但沒有 AIP session 測試；參考韌體 639 bytes 單行上限讓部分回覆送不出去（稽核 `aip.outbound-undeliverable`）。成員 `syncProfile`（`crates/interaction-runtime/src/character_session.rs:193`＝`derive_sync_profile`）與裝置線 v1.2 分片（`crates/interaction-adapter-declarative/src/fragment.rs`＋`protocol.rs:769`＝`supports_fragmentation`）已合併於 `9799b1e`，契約見 `docs/aip/device-profile.md` §3.1／§6.3 |

## 5. Transport（wss／HTTP／SSE／IPC／裝置線）

| | |
|---|---|
| **owner** | iPhone wss：`crates/interaction-runtime/src/mobile.rs`（TLS、配對、每機 token）＋iOS `Services/SocketTransport.swift`／`ConnectionManager.swift`；HTTP／SSE：`crates/interaction-api`；Tauri IPC：`apps/interaction-desktop/src-tauri/src/{lib,character_bridge}.rs`；裝置線 v1.x：`crates/interaction-adapter-declarative/src/protocol.rs` |
| **入口** | `interact-ai serve`（127.0.0.1:8787，Bearer token；token 在 `~/.adaptive-interaction/state/api-token`）；SSE 支援 Last-Event-ID；WebView 只經 IPC，**不直接控制裝置** |
| **狀態來源** | 每個 transport 只負責 framing／重連／退避；語意真相一律回到 §3 的 Session。出站通道由有界（64）的 `character_session::DeviceOutbound` 登記表管理，核心沒有裝置種類分支 |
| **公開契約** | `docs/aip/transport-bindings.md`（§0 身分綁定、§1 iPhone frame、§7 三端接線狀態）、`docs/aip/iphone-companion.md` |
| **擴充點** | 新 transport＝登記一條出站通道＋提供 `DeviceOrigin`（`transport`／`identityStrength`），不改 `character_session_send` |
| **必要測試** | `crates/interaction-runtime/tests/mobile_loop.rs::a_legacy_phone_that_never_negotiates_receives_no_aip_frames`；`declarative_session_loop.rs::diagnostics_report_where_each_member_identity_came_from`、`::a_member_without_an_outbound_channel_is_audited_when_a_broadcast_cannot_reach_it` |
| **已知限制** | 兩條線身分強度不同且不得混稱：iPhone `paired-token`、宣告式裝置 `transport-hello+device-side-pairing`（後者是裝置自報明文比對）。iPhone 真機證據只有 `docs/releases/v0.5.0-iphone-device-evidence.md` 逐列標示的那幾筆 |

## 6. SensorSource（感測停止的唯一介面）

| | |
|---|---|
| **owner** | `crates/interaction-runtime/src/sensor_source.rs`：`SensorSource` port（`source_id`／`declaration_id`／`active_captures`／`request_stop(target, deadline, reason)`／`release`）＋有界登記表（`MAX_SENSOR_SOURCES = 32`）＋未解決停止摘要（`UnresolvedStop`）。唯一的停止協調器 `Runtime::stop_all_sensor_sources` 在 `crates/interaction-runtime/src/sensors.rs` |
| **入口** | `POST /v1/sensors/stop`；緊急停止；UI 停止按鈕；`interact-ai` 對應子指令。四條路徑走**同一個**協調器 |
| **狀態來源** | 即時擷取 → `status.activeSensors`（tray／首頁／角色視窗都吃它）；未確認的停止 → `UnresolvedStop` 摘要（不隨 TTL 過期，`MAX_UNRESOLVED_STOPS = 32`，只能被明確確認或人為解除清掉） |
| **公開契約** | `docs/aip/architecture-boundaries.md` §4.1 實作註記 2；`docs/aip/privacy.md` |
| **擴充點** | 新的會擷取的來源＝實作 `SensorSource` 並登記（本機麥克風 `LocalMicSensorSource`、iPhone `MobileSensorSource`、宣告式裝置各是一般來源，核心沒有裝置特例分支） |
| **必要測試** | `crates/interaction-runtime/tests/sensors_loop.rs::emergency_stop_and_stop_all_sensors_agree_about_an_unstoppable_receptor`、`::revoking_a_provider_stops_its_sensor_source_with_a_target`、`::deleting_a_high_risk_receptor_asks_its_source_to_stop_first`、`::orphan_ttl_moves_unknown_to_unresolved_not_to_normal`、`::the_sensor_source_registry_is_bounded`；桌面投影 `apps/interaction-desktop/src/test/sensorStop.test.ts` |
| **已知限制** | 結果五態（`stopped`／`already-stopped`／`unknown`／`unreachable`／`refused`）只有前兩者算確認；「來源被移除但還在擷取」只能以有界可見的 stop-unknown 呈現——**過了孤兒窗不等於已經停了** |

## 7. Agent（gateway／session lease／預算）

| | |
|---|---|
| **owner** | `crates/interaction-agent-gateway`（真 Agent 行程接線：claude／codex）＋`crates/interaction-runtime/src/{agents,gateway}.rs`（session、lease、mailbox、delegation） |
| **入口** | CLI `interact-ai agent …`；HTTP `/v1/agents/*`；UI「工作」入口 |
| **狀態來源** | `crates/interaction-storage`（SQLite 的 `agent_sessions`）＋`interaction-core` 的 delegation 型別；限界由 `crates/interaction-policy` 確定性強制 |
| **公開契約** | `docs/ARCHITECTURE.md`「Agent Session」段；`docs/USER-GUIDE.md` |
| **擴充點** | 新 agent 種類＝在 `interaction-agent-gateway` 加一個 process adapter；**不得**新增繞過 Policy Governor 的路徑 |
| **必要測試** | `crates/interaction-runtime/tests/agents_loop.rs::delegation_honesty_ladder_dispatched_acknowledged_claimed`、`::human_verify_is_the_only_path_from_claim_to_verified`、`::lease_expiry_kills_capabilities_and_refuses_renewal`、`::estop_cancels_all_open_sessions_and_blocks_new_ones`、`::restart_reports_unknown_for_work_that_was_still_open`；`gateway_loop.rs::working_never_precedes_the_task_actually_reaching_the_agent` |
| **已知限制** | 重啟後仍在進行中的工作一律報 `unknown`（不猜結果）；AI 不可授予 consent、不可解除 emergency stop |

## 8. 設定（桌面偏好／陪伴預設／匯入匯出）

| | |
|---|---|
| **owner** | `apps/interaction-desktop/src/desktop.ts`（`DesktopPrefs` 型別）＋`src-tauri/src/lib.rs`（`desktop_prefs_get`／`desktop_prefs_patch`／`prefs_candidate`／`commit_prefs_patch`，檔案是真相）；陪伴預設交易 `src/companion/applyPresetPlan.ts`；匯入匯出 `src/companion/settingsTransfer.ts` |
| **入口** | UI 設定頁與角色頁；Tauri IPC；主動說話模式的第二段寫到後端（由 Rust `proactive.rs` 確定性強制） |
| **狀態來源** | 偏好檔（Tauri host）＋後端 `mode`。兩段之間可能斷，所以「套用一個檔位」是一筆**可恢復的交易**：recovery marker 與第一段偏好原子寫入，重開後只有 marker 鎖定的欄位仍等於目前值才補送 |
| **公開契約** | `docs/DESKTOP-GUIDE.md`、`docs/aip/general-mode-ux.md` |
| **擴充點** | 角色專屬設定值（配色／說話風格／場景／使魔）**只**由 adapter meta 宣告——`settingsTransfer.ts` 與頁面不得認得任何角色 id。新增一個檔位＝改 `companion/presets.ts` 的定義，交易層不動 |
| **必要測試** | `apps/interaction-desktop/src/test/apply-preset-plan.test.ts`（四個 describe：計畫／marker 驗證／只有沒改過才補送／五種狀態都不冒充「已完成」）；`companion-preset-recovery.test.tsx`；`playfield.test.ts` 的「角色設定匯出／匯入」describe |
| **已知限制** | 舊小樞家族 8 個 id 的匯入寬容路徑仍在（`deprecation-ledger.md` §2.3）；真 Tauri 視窗走查為 needs-environment |

## 9. 儲存（快照 format／migration／backup／parked）

| | |
|---|---|
| **owner** | Session 快照：`crates/interaction-session/src/types.rs`（`SNAPSHOT_FORMAT`）＋`src/ports.rs`（`SessionStore`／`SaveOutcome`／`PortError`）；production store 與遷移：`crates/interaction-runtime/src/character_session.rs`（`JsonSessionStore`、`SESSION_BACKUP_SUFFIX`）。其餘持久化（receipts／plans／sessions／audit）：`crates/interaction-storage`（SQLite） |
| **入口** | 無直接使用者入口；可觀測面是 `GET /v1/character-session/diagnostics` 的選填 `store` 物件（format／migratedFrom／migrationNote／lastPersistedRevision／persistFailures／skippedStale／parked／lastPersistError／note） |
| **狀態來源** | 檔案是真相，但**寫入不等於被要求寫入**：`save` 以 `(epoch, revision)` 字典序在同一把鎖內守門，回 `Written`／`SkippedStale`／`SkippedParked` |
| **公開契約** | `docs/aip/character-session.md` §6；`docs/aip/deprecation-ledger.md` §2.1／§2.2 |
| **擴充點** | 改快照佈局＝`SNAPSHOT_FORMAT + 1`＋加一份**真實舊版本寫出來的** fixture（`crates/interaction-runtime/tests/fixtures/character-session/`）＋遷移路徑。未來格式一律不隔離、不覆寫 |
| **必要測試** | `crates/interaction-runtime/tests/character_session_loop.rs::a_v0_6_0_snapshot_is_restored_and_migrated_to_the_current_format`、`::a_snapshot_from_before_unsupported_intents_is_migrated_instead_of_quarantined`、`::a_future_format_snapshot_is_kept_untouched`、`::a_truncated_snapshot_is_quarantined_with_a_new_epoch` |
| **已知限制** | format 0 的檔案在使用者機器上，repo 無法證明「已經沒有人有」——遷移路徑只能加不能減 |

## 10. 發布（release scripts／evidence-index）

| | |
|---|---|
| **owner** | `scripts/release-prepare.sh`（版本＋CHANGELOG＋golden／codegen，不 commit）→ `scripts/release-verify.sh`（關卡）→ `scripts/release-tag.sh`（從已驗證 commit 打 annotated tag）。`scripts/release.sh` 只印流程 |
| **入口** | 上述三支腳本；CI `.github/workflows/release.yml`（draft → build → `ci-gate` → `finalize`）＋`scripts/ci-required-checks.sh` |
| **狀態來源** | 已發布版本的 canonical 事實：`docs/releases/evidence-index.json`（tag／commit／發布時間／資產數／CI 與 Release run／證據等級／文件指標，含誠實記錄的失敗 job） |
| **公開契約** | `CHANGELOG.md`、`docs/INSTALL.md`（誠實寫明未簽章、無 SBOM／provenance、平台覆蓋） |
| **擴充點** | 新關卡＝加進 `release-verify.sh` 並在 `scripts/tests/release-scripts.sh` 加對應自測；新文件宣稱＝加進 `scripts/tests/docs-claims.sh` |
| **必要測試** | `bash scripts/tests/release-scripts.sh`；`bash scripts/tests/docs-claims.sh`；`crates/interaction-cli/tests/release_provenance.rs::every_crate_version_follows_the_workspace`（兩者也是 `architecture-checks.sh --docs` 的內容） |
| **已知限制** | 桌面安裝包未簽章、無 SBOM／provenance；Linux aarch64 需從原始碼編譯 |

---

## 加一列的時機

新增一個**跨層**的能力（有自己的狀態來源、自己的擴充點、自己的契約文件）時加一列。
只是在既有能力底下多一個實作（多一個角色、多一份 YAML spec、多一個 agent 種類）不加列——
那是既有列的「擴充點」欄該回答的事；如果那一欄答不出來，該修的是那一欄，不是加新列。
