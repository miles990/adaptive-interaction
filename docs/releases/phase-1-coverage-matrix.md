# 階段 1：互動可追溯性覆蓋矩陣

> 逐列核對 repo 現況（2026-09-15，分支 `phase-1/interaction-traceability`，核對 HEAD `9309028`）。
> 簡報點名的三個 findings（D9、D10、D16）與階段 0 的 D2／D3 全部已落地；S3（runtime 寫入點、
> 交易化、保存清理）與 S4（查詢 API／CLI／桌面 UI）也已全部落地。三個由獨立懷疑者發現的問題
> （TB-1、TB-4、TB-5）**也已在核對過程中修復並 commit**（`ba01c0a`／`711de77`），對應列標
> 「已落地」並附核對結果。未列入本輪清單的項目一律標「範圍外」並附理由，
> 不得誤稱已修。證據等級用字：「已驗證」只用在附測試名且來源 commit 有實跑數字；其餘一律
> 「未覆蓋」或「未確認」。真 Agent 端到端驗收（`scripts/tests/phase1/trace_e2e.py`）與完整
> 回歸尚未執行，數字留待整合者補（見 `docs/releases/phase-1-progress.md` §5／§8）。
>
> 欄位：領域／操作／程式入口／改善前紀錄／改善後紀錄（本輪）／儲存位置／跨重啟可查／
> 可串回原互動／敏感資料／保存清理／測試與證據層級／階段（＋範圍外理由）。

## 1. 事件入口

| 領域 | 操作 | 程式入口 | 改善前紀錄 | 改善後紀錄（本輪） | 儲存位置 | 跨重啟可查 | 可串回原互動 | 敏感資料 | 保存清理 | 測試與證據層級 | 階段（＋範圍外理由） |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 事件入口 | HTTP 401（缺 token／token 錯） | `crates/interaction-api/src/lib.rs` `auth_middleware` | 無任何紀錄 | 無（不變） | — | 否 | 否 | 無（未記錄） | n/a | 未覆蓋 | 範圍外——本輪聚焦 resume／狀態語意／stderr／runtime 寫入點與查詢，未授權請求的紀錄留後續階段 |
| 事件入口 | HTTP 403（scope 拒絕） | `lib.rs` `agent_request_allowed`／`session_request_allowed`／`adapter_request_allowed` | 無任何紀錄 | 無（不變） | — | 否 | 否 | 無 | n/a | 未覆蓋 | 範圍外，理由同上 |
| 事件入口 | HTTP 413（body 過大） | tower `RequestBodyLimitLayer`（`MAX_BODY_BYTES=256KiB`） | 無（tower 層直接拒絕，不進 handler） | 無（不變） | — | 否 | 否 | 無 | n/a | 未覆蓋 | 範圍外——由 tower 中介層擋下，不在 runtime 稽核路徑上 |
| 事件入口 | `mobile.unauthenticated-timeout`／`.rate-limited`／`.pair-failed`／`.pair-burned-by-peer`／`.auth-failed` | `crates/interaction-runtime/src/mobile.rs` | 已有 audit | 不變 | SQLite `audit`（class NULL→視為 audit） | 是 | 部分（無 `trace_id`，本輪未回填） | 連線來源資訊，`auth-failed` 刻意不記 token | 依現行 audit 保存（本輪起適用 90 天／100 000 筆） | 未覆蓋 | 範圍外——已有紀錄，非本輪清單 |
| 事件入口 | AIP dedupe：重複訊息被丟 | `crates/interaction-aip`／`interaction-session` 接收路徑（`DEDUPE_RING=256`） | 不 audit | 不變 | — | 否 | 否 | 無 | n/a | 未覆蓋 | 範圍外——既有設計（高頻去重不逐筆稽核） |
| 事件入口 | character HTTP 限流 | `crates/interaction-runtime/src/character.rs` | **刻意**只 `tracing`，不 audit | 不變 | log only | 否 | 否 | 無 | n/a | 未覆蓋 | 範圍外——既有設計決策，非缺陷 |
| 事件入口 | `character.wire-rejected` | `character.rs` | 已有 audit | 不變 | SQLite audit | 是 | 部分 | 已正規化（`safe_name`／`ErrorCode`） | 依 audit 保存 | 未覆蓋 | 範圍外——角色域稽核已完整 |
| 事件入口 | `aip.rejected`／`.fragment-dropped`／`.outbound-undeliverable`／`.outbound-refused` | `crates/interaction-adapter-declarative/src/declarative_session.rs` | 已有 audit | 不變 | SQLite audit | 是 | 部分 | 已正規化 | 依 audit 保存 | 未覆蓋 | 範圍外 |

## 2. 裝置

| 領域 | 操作 | 程式入口 | 改善前紀錄 | 改善後紀錄（本輪） | 儲存位置 | 跨重啟可查 | 可串回原互動 | 敏感資料 | 保存清理 | 測試與證據層級 | 階段（＋範圍外理由） |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 裝置 | `mobile.paired`／`.pair-not-persisted` | `mobile.rs` | 已有 audit | 不變 | SQLite audit | 是 | 部分 | 裝置 id（非 token） | 依 audit 保存 | 未覆蓋 | 範圍外 |
| 裝置 | 連線／斷線／重連 | `mobile.rs`／`declarative_lifecycle.rs`（僅 `provider.state-changed` event／`SensorStopped`） | **無 audit kind**，只有 event | 不變 | event bus（1024 筆 ring，重啟即失） | 否 | 否 | 無 | n/a（重啟丟失） | 未覆蓋 | 範圍外——既有已知限制，未列入本輪清單 |
| 裝置 | `mobile.device-revoked` | `mobile.rs`（`?` 傳播） | 已有 audit，但寫入路徑用 `?` 傳播——若上游提早出錯是否仍落地未逐一走查 | 不變 | SQLite audit | 是（成功寫入時） | 部分 | 無 | 同上 | 未確認 | 範圍外——未列入本輪清單 |
| 裝置 | declarative `provider.rebinding`／`.rebound`／`.rebind-*` | `crates/interaction-runtime/src/declarative_lifecycle.rs` | 已有 audit | 不變 | SQLite audit | 是 | 部分（帶世代） | 無 | 同上 | 未覆蓋（有行為測試，非稽核內容斷言） | 範圍外 |
| 裝置 | `provider.revoked`／`.paired`／`.tested` | `crates/interaction-runtime/src/providers.rs` | 已有 audit | 不變 | SQLite audit | 是 | 部分 | 無 | 同上 | 未覆蓋 | 範圍外 |
| 裝置 | `hardware.metadata-scan` | 未定位（`actor` 固定 `"unattributed-local-caller"`） | 已有 audit，但 `actor` 不符合契約的身分規則（§2） | 不變（非本輪必改） | SQLite audit | 是 | 否（actor 不可信） | 無 | 同上 | 未覆蓋 | 範圍外——actor 命名收斂留後續階段 |

## 3. 能力（capability／tool／action）

| 領域 | 操作 | 程式入口 | 改善前紀錄 | 改善後紀錄（本輪） | 儲存位置 | 跨重啟可查 | 可串回原互動 | 敏感資料 | 保存清理 | 測試與證據層級 | 階段（＋範圍外理由） |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 能力 | 能力註冊／可用性變化 | `crates/interaction-registry` | 只有 event，無 audit | 不變 | event bus（1024，重啟即失） | 否 | 否 | 無 | n/a | 未覆蓋 | 範圍外 |
| 能力 | tool／action 呼叫 | `crates/interaction-runtime/src/executor.rs`（receipts 表＋event） | 有 `receipts` 表（非 audit 表）＋event；**無 audit** | 不變 | SQLite `receipts` 表 | 是（receipts 本身持久） | 是（`action_id`／`plan_id`／`session_id` 可查） | 依 intent／channel 而定，未逐一核對 | 無獨立保存清理常數（未確認是否有 TTL） | 未覆蓋 | 範圍外——receipts 是既有平行機制，非 audit／trace 統一表 |
| 能力 | 逾時 → `ActionUncertain` | `executor.rs` | 反映在 receipt status，**無 audit** | 不變 | 同上（receipts） | 是 | 是 | 同上 | 同上 | 未覆蓋 | 範圍外 |
| 能力 | `action.cancelled` | `crates/interaction-runtime/src/runtime.rs` | 已有 audit | 不變 | SQLite audit | 是 | 部分 | 無 | 依 audit 保存 | 未覆蓋 | 範圍外 |
| 能力 | `action.blocked`（Policy Governor 攔截） | `executor.rs`（actor＝`governor`） | 已有 audit | 不變（`actor="governor"` 不符合本輪身分規則但非必改項） | SQLite audit | 是 | 部分 | 無 | 依 audit 保存 | 未覆蓋 | 範圍外 |

## 4. Agent（gateway／session／resume／終態）—— 本輪核心

| 領域 | 操作 | 程式入口 | 改善前紀錄 | 改善後紀錄（本輪） | 儲存位置 | 跨重啟可查 | 可串回原互動 | 敏感資料 | 保存清理 | 測試與證據層級 | 階段（＋範圍外理由） |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Agent | resume 授權檢查（放寬／拒絕／找不到原紀錄） | `crates/interaction-runtime/src/agents.rs::create_agent_session`／`resume_audit_record`／`check_resume_not_wider` | 完全不落任何稽核 | **已落地**（D9，`5eff899`）：`agent-session.resume-checked`（audit），accepted／rejected／ignored（純對話 session）；`code` 有 `resume.ok`／`resume.tools-widened`／`resume.workdir-changed`／`resume.no-record` 等十餘種；`detail` 不含 token；workdir 只放 `{digest, basename}`。**TB-1 已修**（`ba01c0a`）：`dataScope`／`toolScope`／`consentScope` 改用新的 `safe_scope_list`，`workspace:<路徑>` 改寫成 `{digest}/{basename}`，拒絕文案裡帶原始路徑的部分也另外過 `ResumeRejection::audit_reason`。**TB-5 已修**（`711de77`）：接受分支的稽核延到 `check_delegation` 等後續上限檢查都通過、session 確定會被建立才寫（見下方「委派上限」列） | SQLite `audit`（class=audit） | 是 | 是：`trace_id`=原 session id（找得到時）、`session`=本次嘗試 id、`causation_id`=呼叫端給的 provider thread id | 不含 token；workdir 只放 `{digest, basename}`；scope 標籤同樣過 `safe_scope_list`（TB-1 已修） | 90 天／100 000 筆（audit class） | **已驗證**（各自 commit 附帶）：`cargo test -p interaction-runtime --test gateway_loop -- resume`（8 passed）＋`--lib -- resume`（3 passed）；TB-1 附 `scope_labels_keep_paths_out_of_the_record`；TB-5 附 `a_resume_blocked_by_the_session_limit_leaves_no_accepted_row` | **已落地（`5eff899`／`ba01c0a`／`711de77`）** |
| Agent | resume 接受後的委派上限檢查（`check_delegation`：深度／循環／同 delegation tree 並行數） | `agents.rs::create_agent_session`（resume 稽核之後） | n/a（先前無稽核，此問題不存在） | **TB-5 已修**（`711de77`）：判定結果先收進區域變數 `accepted_resume`，等所有上限檢查都通過、session 確定會被建立才寫 `resume.ok`；拒絕／no-record／ignored 三條路徑不受影響 | SQLite `audit` | 是 | 是（與實際結果一致） | 無 | 同上 | **已驗證**（`711de77` 附帶）：`a_resume_blocked_by_the_session_limit_leaves_no_accepted_row`（`maxSessions=1` 逼出 403，`query_trace(outcome=accepted)` 必須是空的） | **已落地（`711de77`）** |
| Agent | mailbox 派送（`gateway_attach`） | `gateway.rs::gateway_attach` | 無 audit／trace（只有 event） | **已落地**（`ce68181`）：`agent-session.dispatched`（trace，無 outcome）；connector 版本、resume 與否、唯讀／可寫、toolsDisabled、workdir 摘要、三種 scope、ttl／maxCost／maxMessages；`requestedModel` 誠實寫 `"not-specifiable-via-gateway"`。**TB-1 已修**（`ba01c0a`）：三個 scope 欄位同樣改用 `safe_scope_list` | SQLite `audit`（class=trace） | 是 | 是（`session_id`／`trace_id`＝agentSessionId） | detail 只含結構化欄位，不含訊息全文；scope 標籤同樣過 `safe_scope_list` | 30 天／50 000 筆（trace class） | **已驗證**（`ce68181` 附帶）：`a_full_turn_leaves_a_dispatch_delivery_and_outcome_trail` | **已落地（`ce68181`／`ba01c0a`）** |
| Agent | 任務送達（`gateway_deliver`） | `gateway.rs::gateway_deliver` | 無 audit／trace | **已落地**（`ce68181`）：`agent-session.task-delivered`（trace，無 outcome：送達≠完成），`caused_by`＝messageId，帶 contextBundle 的 bundleId／contentHash／bytes／truncated | SQLite `audit`（class=trace） | 是 | 是 | detail 不含訊息全文 | 30 天／50 000 筆 | **已驗證**（`ce68181` 同一測試） | **已落地（`ce68181`）** |
| Agent | 執行進度（`task-started`／`progress`） | `agents.rs`（`persist_phase` 設 `"working"`） | 只有 event＋`observations`（72 小時後消失） | 高頻進度**不逐筆**寫入 `trace`；改在記憶體累計 `progressEvents` 計數，終態 `agent-session.outcome` 的 detail 帶 `progressEventsAggregated` | 記憶體（進行中）＋終態 audit（完成時） | 進行中的計數**不**跨重啟；終態本身可查 | 終態記錄可串回；進行中的逐筆進度不可（本來就不逐筆記） | 無 | 終態依 audit 保存；72 小時內可查 observations | **已驗證**（`ce68181`：「進度不逐筆寫」斷言在 `a_full_turn_leaves_a_dispatch_delivery_and_outcome_trail` 內） | **已落地（`ce68181`）** |
| Agent | `agent.approval`（人工核准／拒絕、watchdog 拒絕） | `gateway.rs::resolve_approval_as` | 已有 audit，但**摘要原文**寫入 detail | **已落地**（`ce68181`）：`TraceRecord::audit`，`store.record(...)?`（關鍵路徑，寫不進去回錯，與 resume 接受同級）；summary 截 200 字（`safe_summary`），`code`＝`approval.human-approved`／`approval.human-denied`／`approval.watchdog-denied` | SQLite audit | 是 | 是（`caused_by`＝requestId） | 摘要截斷至 200 字 | 依 audit 保存 | **已驗證**（`ce68181` 附帶，approval 相關斷言） | **已落地（`ce68181`）** |
| Agent | `interrupt`（`POST /v1/agent-sessions/{id}/interrupt`） | `gateway.rs::gateway_interrupt` | **完全無 audit、無 event** | **已落地**（`ce68181`）：`agent-session.interrupt-requested`（audit）：`interrupt.sent`（Accepted）／`interrupt.undeliverable`（Failed）；`actor` 依 principal（`human`／`agent-session:<id>`，由 `routes.rs`／Tauri 傳入）；requested ≠ confirmed，非關鍵路徑（`record_trace`） | SQLite `audit`（class=audit） | 是 | 是（`session_id`） | 無 | 90 天／100 000 筆 | **已驗證**（`ce68181` 附帶）：`an_interrupt_is_audited_as_requested_not_confirmed` | **已落地（`ce68181`）** |
| Agent | `agent-session.closed` | `agents.rs::close_agent_session` | 已有 audit（`actor="user"`），狀態寫入與 audit 分離提交 | **已落地並交易化**（`3c8a2a6`）：`store.transaction`，`tx.save_agent_session`＋`tx.record(closed audit)` 同一筆提交；`actor` 仍寫 `"human"`（收斂非本輪必改項） | SQLite audit | 是 | 部分 | 無 | 依 audit 保存 | **已驗證**（`3c8a2a6` 附帶）：`a_failed_commit_rolls_back_the_close` | **已落地（`3c8a2a6`）** |
| Agent | `agent-session.emergency-stop` | `agents.rs::estop_mailbox_note` | 已有 audit，但寫入用 `let _ =` 吞錯 | **已落地**（`ce68181`）：改走 `record_trace`（失敗計數＋`tracing::error!`）；先停（`close_agent_session`／`gateway_spawn_kill`）再記 | SQLite audit（成功時） | 是（成功時） | 部分 | 無 | 依 audit 保存 | 未覆蓋（無獨立測試名，行為隨附在 estop 相關整合測試裡） | **已落地（`ce68181`）** |
| Agent | `expire_if_needed`（session TTL 到期） | `agents.rs::report_agent_session` 終態路徑 | 只有 event，**無 audit** | **已落地**（`ce68181`）：`agent-session.outcome`（audit，outcome=Expired，code=`lease.expired`） | SQLite audit | 是 | 是 | 無 | 90 天／100 000 筆 | **已驗證**（`ce68181` 附帶） | **已落地（`ce68181`）** |
| Agent | `restore_agent_sessions`（重啟時把仍開啟的 session 標 Expired） | `agents.rs::restore_agent_sessions` | 只有 event，**無 audit** | **已落地**（`ce68181`）：`agent-session.outcome`（audit，outcome=Unknown，code=`runtime.restarted`）。**TB-4 已修**（`711de77`）：session 狀態寫回改走 `persist_agent_session`（失敗會 `note_storage_write_failure`，計進 `trace_write_failures`），不再是裸的 `let _ =`；`restore_agent_sessions` 改為 `#[doc(hidden)] pub` 讓這條不變量可被測試確定性驗證 | SQLite audit（outcome 紀錄與狀態寫入兩者現在都會計數） | 是 | 是 | 無（可能含 reaped pgid） | 90 天／100 000 筆 | **已驗證**（`ce68181` 附帶 outcome 紀錄；`711de77` 附帶 `a_failed_restore_write_is_counted_not_swallowed`：正常 restore `traceWriteFailures=0`，用 `force_next_agent_session_save_error` 逼出失敗時計數變 1） | **已落地（`ce68181`／`711de77`）** |
| Agent | SSE `agent.session.state` 語意（D10 核心） | `agents.rs::persist_phase`／`emit_agent_session_state_for`／`close_taxonomy`；`gateway.rs`（消費點） | SSE taxonomy（`fetched`／`working`）不存在於 `AgentSessionRecord.state`；close 時 `Failed`／`Unknown`／`TimedOut`／`Expired` 全部塌成 `"closed"` | **已落地**（`d8f934b`）：三維拆分——`AgentSessionRecord.phase: Option<String>`（持久化，七個發事件點都改走 `persist_phase`）；SSE payload 加 `phase`／`recordState`／`lifecycle`；`close_taxonomy()` 依終局逐一對應真實字串 | SQLite `agent_sessions`（`phase` 欄位）＋SSE payload | 是 | 是 | 無 | n/a（狀態欄位非紀錄表） | **已驗證**：`cargo test -p interaction-runtime --test agents_loop -- phase close_projection`（3 passed）；`-- gateway_loop -- phase`（1 passed）；`-- closing_an_unknown`（1 passed） | **已落地（`d8f934b`；解決階段 0 D2）** |
| Agent | `verify_agent_session`（human_verified） | `agents.rs::verify_agent_session` | 已有 audit `agent-session.verified`，狀態寫入與 audit 不是同一 transaction | **已落地並交易化**（`3c8a2a6`）：`store.transaction(|tx| { tx.save_agent_session(...); tx.record(verified audit) })`；失敗回 `Err`，記憶體 record 不變 | SQLite audit＋`agent_sessions`（同一 transaction） | 是 | 是 | 無（備註內容不進稽核，只記 `hasNote`） | 90 天／100 000 筆 | **已驗證**（`3c8a2a6` 附帶）：`a_failed_commit_rolls_back_the_whole_verification`（用 `force_next_transaction_error` 真的讓 commit 失敗） | **已落地（`3c8a2a6`）** |
| Agent | `agent-session.history-pruned` | `agents.rs`（重啟／關閉歷史裁剪路徑） | 已有 audit，`let _ =` 吞錯 | **維持現況**：核對 HEAD `9309028`，此呼叫點仍是 `let _ = self.store.audit(...)`，不計數；未列入 S1–S4 已知清單 | SQLite audit（成功時） | 是（成功時） | 部分 | 無 | 依 audit 保存 | 未覆蓋 | 範圍外——確認未觸及，留後續階段 |
| Agent | provider 實際模型回報 | `crates/interaction-agent-gateway`（`GatewayEvent::SessionStarted{model}`，`5ee242d`）；`gateway.rs::set_provider_session_id`（`3c256c7`） | `requestedModel`／`actualModel` 完全無法取得 | **已落地**（`3c256c7`）：`SessionStarted` 帶 `model`；消費點寫一筆 `trace("agent-session.provider-model")`，同時落到 `AgentSessionRecord.actual_model: Option<String>`（serde default）；讀不到就不寫；`requestedModel` 仍 `not-specifiable-via-gateway` | event bus＋SQLite `audit`（trace class）＋`agent_sessions`（`actual_model` 欄位） | 是 | 是（`session_id`） | 模型名稱本身不算敏感 | trace class 30 天／50 000 筆 | 未見獨立測試名（行為隨附在整合測試） | **已落地（`3c256c7`）** |
| Agent | `agent-session.subprocess-stderr` 診斷紀錄化 | `gateway.rs`（消費 `GatewayEvent::StderrCaptured`，`3c256c7`） | S2（`baabec7`）只落地事件與脫敏 tail 本身，runtime 未消費成紀錄 | **已落地**（`3c256c7`）：`TraceRecord::diagnostic("agent-session.subprocess-stderr")`，tail 再收一次至 600 字；刻意排在 `TaskOutcomeUnknown` 與 `SessionClosed` 之間，不觸發／不影響 failed／unknown 判定 | SQLite `audit`（diagnostic class） | 是 | 是（`session_id`） | 已脫敏（`diagnostics.rs`，含 `1070eca` 的非 UTF-8 修復） | 7 天／10 000 筆（diagnostic class） | **已驗證**（`ce68181` 附帶）：`subprocess_stderr_is_a_diagnostic_record_and_changes_no_outcome` | **已落地（`3c256c7`）** |
| Agent | `agent-session.capability-issued` | `agents.rs` | 已有 audit（`Store::audit`，不含 `trace_id`／`session_id`） | **已落地**（`9309028`）：改用 `TraceRecord::audit`＋`trace_id`／`session_id`（`outcome=Accepted`，`code=capability.issued`），一次工作的完整因果鏈能用同一個 `sessionId` 從簽發查到關閉；寫入仍是 `store.record(...)?`（fail-closed，寫不進去不簽發），token 本身不進紀錄 | SQLite audit | 是 | 是（`session_id`／`trace_id`＝agentSessionId，本輪起可查） | 無 | 依 audit 保存 | 未覆蓋（無獨立測試名，行為隨附在因果鏈整合測試裡） | **已落地（`9309028`）** |

## 5. 同意（consent）

| 領域 | 操作 | 程式入口 | 改善前紀錄 | 改善後紀錄（本輪） | 儲存位置 | 跨重啟可查 | 可串回原互動 | 敏感資料 | 保存清理 | 測試與證據層級 | 階段（＋範圍外理由） |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 同意 | `consent.granted`／`consent.revoked` | `crates/interaction-runtime/src/runtime.rs` | 已有 audit | 不變 | SQLite audit | 是 | 部分 | 無 | 依 audit 保存 | 未覆蓋 | 範圍外 |
| 同意 | 拒絕（`maxUses=0`） | `runtime.rs::grant_consent_with_uses` | 無 audit | **已落地**（`ce68181`）：`consent.rejected`（audit，`record_trace`，非關鍵路徑） | SQLite audit | 是 | 是 | 無 | 90 天／100 000 筆 | 未覆蓋（無獨立測試名） | **已落地（`ce68181`）** |
| 同意 | 過期（惰性比對） | `crates/interaction-core/src/session.rs`（`has_consent` 呼叫現場才判定過期） | 無紀錄（純函式層判定，不主動落地） | 不變；仍是惰性比對，無獨立事件 | — | 否 | 否 | 無 | n/a | 未覆蓋 | 範圍外——`interaction-core` 是純函式層，過期判定發生在 `gate()` 呼叫當下，非本輪範圍；既有已知限制 |
| 同意 | one-shot 消耗 | `crates/interaction-runtime/src/executor.rs::consume_one_shot_consent` | 無 audit | **已落地**（`ce68181`）：`consent.consumed`（audit，outcome=Completed，`caused_by`=actionId，detail `{scope, remaining, actionId}`）。**寫入順序見 interaction-tracing.md §8.1 TB-3**：session 一次性消耗先關鍵寫入（`?`），稽核紀錄用 `record_trace` 之後補寫，非同一 transaction | SQLite audit | 是 | 是 | 無 | 90 天／100 000 筆 | **已驗證**（`ce68181` 附帶）：`spending_a_one_shot_consent_is_audited` | **已落地（`ce68181`）；TB-3 為已知限制（非本輪修復範圍）** |

## 6. 工作（task／observations 視角）

| 領域 | 操作 | 程式入口 | 改善前紀錄 | 改善後紀錄（本輪） | 儲存位置 | 跨重啟可查 | 可串回原互動 | 敏感資料 | 保存清理 | 測試與證據層級 | 階段（＋範圍外理由） |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 工作 | `claimed`／`failed`／`cancelled`／`unknown`／`timed-out` 的執行過程 | `agents.rs`（各終態轉移） | 只有 event＋`observations`（`observation_retention_hours=72`） | 終態本身走 §4 Agent 域的 `agent-session.outcome`（已落地）；過程中的細節仍依賴 72 小時 observations 視窗，本輪不延長此視窗 | event bus（1024 ring，重啟即失）＋`observations` 表（72h TTL） | 終態：是；過程細節：僅 72 小時內 | 終態：是；72 小時內的過程細節：是；之後：否 | 依 observation 內容而定，未逐一核對 | observations 72 小時；終態依 audit 保存政策 | 見 §4 對應列 | 見 §4——本項只補充「過程紀錄仍受 72 小時視窗限制」這件事，本輪不改變此視窗 |
| 工作 | 人工驗證閉環（`agent-session.verified`） | 同 §4 `verify_agent_session` | 見 §4 | 見 §4（已交易化） | 見 §4 | 見 §4 | 見 §4 | 見 §4 | 見 §4 | 見 §4 | 見 §4，此處不重複列出 |
| 工作 | 「這件工作的經過」桌面呈現 | `apps/interaction-desktop/src/pages/AiPage.tsx::WorkActivitySection` | 不存在（只有 mailbox 訊息） | **已落地**（`cbc76c0`）：`GET /v1/agent-sessions/{id}/activity` 投影成 headline／目前狀態／失敗原因／下一步／時間線；一般模式看不到任何識別碼／kind／code／stderr；進階模式才有「技術詳情」 | 無獨立儲存（純投影） | 是（依底層 trace 紀錄） | 是 | 一般模式零技術詞；進階模式的原始紀錄仍受 diagnostic 脫敏規則保護 | 依底層 trace／audit 保存政策 | **已驗證**（`cbc76c0` 附帶）：`apps/interaction-desktop/src/test/sessionActivity.test.tsx`（8 個）＋`regressions-v05.test.tsx` 擴充黑名單＋`e2e/work-activity.spec.ts`（真 daemon＋fixture）；`pnpm typecheck` ✓、`pnpm test` 93 files／1925 passed／0 failed、`pnpm build` ✓（`cbc76c0` commit message 附帶數字，本文件未重跑） | **已落地（`cbc76c0`）** |

## 7. 記憶（memory／knowledge／asset）

| 領域 | 操作 | 程式入口 | 改善前紀錄 | 改善後紀錄（本輪） | 儲存位置 | 跨重啟可查 | 可串回原互動 | 敏感資料 | 保存清理 | 測試與證據層級 | 階段（＋範圍外理由） |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 記憶 | `memory.created`／`memory.deleted` | `crates/interaction-runtime/src/memory.rs` | 已有 audit | 不變 | SQLite audit | 是 | 部分 | 記憶內容不進 detail（未逐一核對是否 100% 排除） | 依 audit 保存 | 未覆蓋 | 範圍外 |
| 記憶 | `memory.updated` | `memory.rs`（update 路徑） | 撰稿初期未確認是否已有 audit | **已落地**（`ce68181`）：`memory.updated`（audit，`code=memory.patched`，只記改了哪些**欄位名**`fields`，不記內容）。**寫入順序見 interaction-tracing.md §8.1 TB-3**：`persist_memory` 先關鍵寫入，稽核紀錄用 `record_trace` 之後補寫 | SQLite audit | 是 | 是 | 只記欄位名 | 90 天／100 000 筆 | 未覆蓋（無獨立測試名） | **已落地（`ce68181`）；TB-3 為已知限制（非本輪修復範圍）** |
| 記憶 | context bundle（`POST /v1/memory/context-bundle`） | `memory.rs` | 純讀無紀錄；無模板／版本概念 | 不變；本輪明確排除捏造模板概念 | — | 否 | 否 | bundle 內容含記憶全文（context bundle receipt，見 `docs/aip/interaction-tracing.md` §5） | n/a | 未覆蓋 | 範圍外——既有已知限制，不在本輪清單 |
| 記憶 | 匯出（`EXPORT_MAX_ITEMS=1000`） | `memory.rs` | 無 audit | 不變 | — | 否 | 否 | 匯出內容本身即敏感 | n/a | 未覆蓋 | 範圍外 |
| 記憶 | `domain-pack.installed`／`.uninstalled` | `crates/interaction-runtime/src/knowledge.rs` 或 `memory.rs` | 已有 audit | 不變 | SQLite audit | 是 | 部分 | 無 | 依 audit 保存 | 未覆蓋 | 範圍外 |
| 記憶 | `asset.imported`／`.deleted`／`.derived` | 同上 | 已有 audit，`actor` 固定 `"unattributed-api-caller"` | 不變（actor 收斂非本輪必改項） | SQLite audit | 是 | 部分 | 檔案路徑等未逐一核對 | 依 audit 保存 | 未覆蓋 | 範圍外 |

## 8. 系統（daemon／instance lock／寫入失敗）

| 領域 | 操作 | 程式入口 | 改善前紀錄 | 改善後紀錄（本輪） | 儲存位置 | 跨重啟可查 | 可串回原互動 | 敏感資料 | 保存清理 | 測試與證據層級 | 階段（＋範圍外理由） |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 系統 | daemon 啟動／關閉 | `crates/interaction-cli/src/commands.rs` | 無 audit（`meta.clean_shutdown` 旗標） | 不變 | `meta` 表（旗標，非紀錄） | 是（旗標本身） | 否 | 無 | n/a | 未覆蓋 | 範圍外 |
| 系統 | `instance lock`（重啟搶鎖／回收 stale lock） | `crates/interaction-runtime/src/lock.rs` | 只有 `tracing`（log），不進 audit | 不變 | log only | 否 | 否 | 無 | n/a | 未覆蓋 | 範圍外 |
| 系統 | 紀錄寫入失敗（既有 `let _ = store.audit(...)`／`?` 傳播／transaction 三種既有模式） | 全 repo（`sensor_journal.rs:374` 是唯一既有的 transaction 化案例） | 大量靜默吞錯、部分 `?` 傳播、1 處已 transaction 化 | **統一入口已落地並大幅擴大覆蓋**：`Runtime::record_trace(&self, record: TraceRecord)`（`pub(crate)`，`runtime.rs:610`）＋`Runtime::note_storage_write_failure`（`runtime.rs:630`，session 狀態快照寫入失敗）＋`Runtime::trace_write_failures() -> u64`（`pub`，進 `/v1/status`，三者共用同一個計數器）。resume 拒絕、dispatched／delivered／provider-model／subprocess-stderr／interrupt-requested／emergency-stop／consent.rejected 等消費點已改走 `record_trace`；resume 接受／`agent.approval`／`agent-session.capability-issued` 走 `store.record(...)?`（關鍵，回錯）；verify／close 走 `store.transaction`（關鍵，回滾）；`restore_agent_sessions` 的狀態寫回改走 `persist_agent_session`（`711de77`，TB-4 已修）。**唯一已知例外**：`agent-session.history-pruned` 仍是裸的 `let _ =`（未列入本輪清單，維持現況） | `record_trace`／`note_storage_write_failure` 失敗時：`tracing::error!`（記憶體 log）＋`trace_write_failures`（記憶體計數＋status 投影） | 計數器本身不持久化（重啟歸零） | n/a | 無 | n/a | **已驗證**（`3c8a2a6` 附帶）：`status_reports_trace_write_failures_and_counts`；`711de77` 附帶：`a_failed_restore_write_is_counted_not_swallowed` | **已落地（`3c256c7`／`ce68181`／`3c8a2a6`／`711de77`／`9309028`）；`agent-session.history-pruned` 為唯一已知例外** |

## 9. 角色（character）

| 領域 | 操作 | 程式入口 | 改善前紀錄 | 改善後紀錄（本輪） | 儲存位置 | 跨重啟可查 | 可串回原互動 | 敏感資料 | 保存清理 | 測試與證據層級 | 階段（＋範圍外理由） |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 角色 | `character.hello`／adapter added／revoked／connected／disconnected | `crates/interaction-runtime/src/character.rs` | 已有 audit | 不變 | SQLite audit | 是 | 部分 | 無 | 依 audit 保存 | 未覆蓋（本文件未重新核對） | 範圍外——角色域稽核基礎設施已完整，非本輪範圍 |
| 角色 | `character.manual-intent`（message ≤200 字入 audit） | `character.rs` | 已有 audit | 不變 | SQLite audit | 是 | 部分 | 已截斷至 200 字 | 依 audit 保存 | 未覆蓋 | 範圍外 |
| 角色 | 外部 event 正常路徑 | `character.rs` | 不 audit（→`observations`） | 不變 | `observations`（72h） | 72 小時內 | 是（72h 內） | 依內容而定 | 72 小時 | 未覆蓋 | 範圍外——既有設計 |
| 角色 | `character.input-capability-not-declared`／`.receipt`／`.system-text`／`.estop-resync`／`.event-refused`／`.wire-rejected` | `character.rs` | 已有 audit | 不變 | SQLite audit | 是 | 部分 | 已正規化 | 依 audit 保存 | 未覆蓋 | 範圍外 |

## 10. 查詢與治理（誰能讀什麼）

| 領域 | 操作 | 程式入口 | 改善前紀錄 | 改善後紀錄（本輪） | 儲存位置 | 跨重啟可查 | 可串回原互動 | 敏感資料 | 保存清理 | 測試與證據層級 | 階段（＋範圍外理由） |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 查詢與治理 | `AuthPrincipal` 分權（Human／LegacyAgent／AgentSession／CharacterAdapter） | `crates/interaction-api/src/lib.rs` | 既有機制 | 不變 | — | n/a | n/a | n/a | n/a | 未覆蓋（有分權測試，非稽核內容測試） | 範圍外——既有機制，本輪只是消費它 |
| 查詢與治理 | `GET /v1/audit` 讀取權限 | `lib.rs`（`agent_request_allowed` 排除；`session_request_allowed`／`adapter_request_allowed` 皆無對應白名單） | 只有 Human 能讀 | 不變（本輪未擴大或縮小既有讀取權限） | n/a | n/a | n/a | n/a | n/a | 未覆蓋 | 範圍外——查詢端授權模型沿用既有設計 |
| 查詢與治理 | `GET /v1/trace` | 規劃中 | 不存在 | **已落地**（`e63986f`）：`traceId`／`sessionId`／`kind`／`class`／`actor`／`outcome`／`since`／`until`／`before`／`limit`，回 `{items, nextCursor, limit}`；打錯字的 `class`／`outcome`／時間回 400；human-only（`agent_request_allowed` 明確排除）；`cbc76c0` 再把解析規則搬進 `activity_trace::TraceQueryInput`，HTTP／Tauri IPC／CLI 共用同一條規則 | SQLite `audit` 表投影，無獨立儲存 | 是 | 是 | 依底層紀錄的既有規則 | 依底層紀錄的既有保存政策 | **已驗證**（`e63986f` 附帶）：`trace_and_activity_are_human_only`（legacy agent／session-scoped 皆 403）、`trace_query_clamps_pages_and_never_crosses_sessions`（limit 9999→500、cursor 分頁不重不漏、sessionId 不跨 session、三種打錯字回 400） | **已落地（`e63986f`／`cbc76c0`）** |
| 查詢與治理 | `GET /v1/agent-sessions/{id}/activity` | 規劃中 | 不存在 | **已落地**（`e63986f`，`cbc76c0` 擴充桌面消費）：投影成 headline／state_label／phase／record_state／lifecycle／failure_reason／next_step／timeline／records／truncated／next_cursor；session 不存在回 404；diagnostic 類只留脫敏 tail；human-only | 同上 | 是 | 是 | 一般模式看不到技術詞（見 `docs/aip/general-mode-ux.md` §5.8） | 依底層紀錄的既有保存政策 | **已驗證**（`e63986f` 附帶）：`activity_projects_records_into_plain_language`（標籤是人話且不含 kind 字串、stderr 只剩 tail、不存在的 session 回 404） | **已落地（`e63986f`／`cbc76c0`）** |
| 查詢與治理 | CLI `interact-ai trace`／`interact-ai agents activity` | 規劃中 | 不存在 | **已落地**（`58f7477`）：打 `/v1/trace`／`/v1/agent-sessions/{id}/activity`；`--agent-scope` 對兩支都失敗（人類層） | n/a | n/a | n/a | n/a | n/a | **已驗證**（`58f7477` commit message 附帶）：`./scripts/v03-cli-e2e.sh` → 102 passed／0 failed（新增前 96，六個新 case 全過） | **已落地（`58f7477`）** |
| 查詢與治理 | 桌面「這件工作的經過」 | `apps/interaction-desktop/src/pages/AiPage.tsx` | 不存在 | 見 §6 | — | — | — | — | — | 見 §6 | **已落地（`cbc76c0`）**，詳見 §6 |

## 附註：驗證方式

以下 kind／型別／函式，撰寫本文件過程中逐一以 `grep`／`Read`／`codegraph_explore` 核對對應檔案的
**當前內容**（HEAD `9309028`），並引用各自 commit message 附帶的實跑測試數字——不是抄錄
commit 訊息就當作已完成：`agent-session.resume-checked`、`agent-session.dispatched`、
`agent-session.task-delivered`、`agent-session.outcome`、`agent-session.interrupt-requested`、
`agent-session.subprocess-stderr`、`agent-session.provider-model`、`agent-session.closed`
（transaction）、`agent-session.verified`（transaction）、`agent-session.emergency-stop`、
`agent-session.capability-issued`、`agent.approval`、`consent.consumed`、`consent.rejected`、
`memory.updated`、`AgentSessionRecord.phase`／`actual_model`、
`Runtime::record_trace`／`note_storage_write_failure`／`trace_write_failures`、`GET /v1/trace`、
`GET /v1/agent-sessions/{id}/activity`、CLI `trace`／`agents activity`、桌面 `WorkActivitySection`、
`agents.rs::safe_scope_list`。

三個由獨立懷疑者發現的問題（TB-1、TB-4、TB-5）在本文件核對過程中觀察到工作樹先後出現、又
在收尾前 commit 完成（`ba01c0a`／`711de77`）——同樣依「未提交不算數」的原則：先記「進行中」，
等對應 commit 真的落地才改記「已落地」並實跑對應測試（見上方各列與
`docs/aip/interaction-tracing.md` §8.2 的詳細核對紀錄）。

真 Agent 端到端驗收腳本 `scripts/tests/phase1/trace_e2e.py`（`9309028` 已 commit，用法見
`scripts/tests/phase1/README.md`）本文件撰寫時**尚未執行**；六個情境（normal／cancel／
resume／failure／approval／restart）逐一對照本矩陣各列期望的 kind／outcome，執行後的
`result.json` 應回填本文件與 `docs/releases/phase-1-progress.md` §5／§8。
