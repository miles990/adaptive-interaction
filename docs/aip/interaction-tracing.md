# 互動可追溯性：稽核（audit）／追蹤（trace）／診斷（diagnostic）紀錄

> 證據等級用字沿用既有慣例：程式碼裡有這條防線寫「有」＋`path:line`；沒有測試斷言的一律
> 「未覆蓋」，不寫「已驗證」。本文件涵蓋階段 1「互動可追溯性」新增的追蹤紀錄契約 v1；
> 既有的麥克風／記憶保存規則不變，見 `docs/ARCHITECTURE.md`「感測隱私」與 `docs/aip/privacy.md`。
> 本文件核對時 HEAD 為 `9309028`：S1（型別＋儲存）、S2（stderr 留痕＋脫敏）、S3（runtime 寫入點
> ＋交易化＋保存清理）、S4（查詢 API／CLI／桌面 UI）**全部已落地**；三個原始簡報 finding
> （D9、D10、D16）與階段 0 的 D2／D3 已修復，逐項附實跑測試數字（見 §3、§4）。三個由獨立懷疑者
> 發現、原本記為「進行中修復」的問題（TB-1、TB-4、TB-5）在文件收尾前**也已修復並 commit**
> （`ba01c0a`／`711de77`），§8.2 逐項附核對結果；四個不在本輪範圍內的已知限制（TB-3、TB-7、
> TB-8、TB-12）在 §8.1 誠實記錄，不假裝已解決。

## 1. 目的與三種紀錄責任

階段 0 已確認的現況：舊版 `audit` 表只有 `at`／`kind`／`actor`／`detail` 四欄，全庫沒有任何
`DELETE FROM audit`——授權決定（例如 resume 續開）與短暫的執行進度混在同一張無保存政策的表裡，
既沒有分級承諾，也沒有清理機制。階段 1 把「一張表、一種承諾」拆成三種紀錄責任，**共用同一張
底層表**，用 `class` 欄位分流：

| class | 內容 | 完整性承諾 | 保存政策（預設） | 查詢權限 |
|---|---|---|---|---|
| `audit` | 授權與狀態變更：consent 核准／拒絕／撤銷／消耗、resume 判定、記憶修改刪除、工作終態轉移、emergency stop、verified | 不取樣、不漏；關鍵轉移與狀態同一 SQLite transaction 提交 | 90 天／100 000 筆 | `GET /v1/audit`／`GET /v1/trace`，**只有 Human**（`AuthPrincipal::LegacyAgent` 明確排除；`AgentSession`／`CharacterAdapter` 沒有白名單條目＝一律拒絕，見 §6） |
| `trace` | 互動的接收、路由、派送、進度、結果 | 高頻進度可彙整，但保留起點／轉折／終點／丟棄計數 | 30 天／50 000 筆 | 同上 |
| `diagnostic` | 協定解析、子程序 stderr、重連、內部錯誤 | 有界、脫敏、可截斷（標 `truncated`） | 7 天／10 000 筆 | 同上 |

三者不是三張表，是同一張 SQLite `audit` 表的三種 `class` 值；權威型別與 API 在
`crates/interaction-core/src/trace.rs`（`pub mod trace`）＋`crates/interaction-storage/src/lib.rs`。

## 2. 追蹤紀錄契約 v1

沿用既有 `audit` 表（`id INTEGER PRIMARY KEY AUTOINCREMENT, at, kind, actor, detail`），
schema 8→9（`CURRENT_SCHEMA = 9`，`crates/interaction-storage/src/lib.rs:21`）**只加欄位與
索引**，全部 nullable：

| 欄位 | 型別 | 說明 |
|---|---|---|
| `class` | `TEXT` | `'audit'｜'trace'｜'diagnostic'`；舊列 `NULL` 讀出時視為 `'audit'`（`COALESCE(class,'audit')`，最保守：保存最久、不取樣） |
| `schema` | `INTEGER` | 紀錄 schema 版本；本輪＝`TRACE_RECORD_SCHEMA=1`；舊列 `NULL` |
| `trace_id` | `TEXT` | 同一段互動的串接鍵：agent 工作＝agentSessionId；resume 檢查＝被接續的原 session id（找得到時） |
| `causation_id` | `TEXT` | 直接原因：上一個事件／messageId／requestId／resumeProviderSessionId…；不知道就 `NULL`，不補造 |
| `session_id` | `TEXT` | 關聯的 agent session id（可查詢） |
| `outcome` | `TEXT` | `accepted｜rejected｜ignored｜deferred｜completed｜claimed｜failed｜cancelled｜unknown｜verified｜expired｜pruned`；可 `NULL` |
| `code` | `TEXT` | 結構化原因碼，kebab-case 且以領域為前綴，例：`resume.scope-widened`、`consent.denied` |
| `source_at` | `TEXT` | 來源自報時間（裝置／provider）；不可信，只供參考 |

索引：`idx_audit_trace(trace_id, id)`、`idx_audit_session(session_id, id)`、`idx_audit_kind(kind, id)`、
`idx_audit_class_at(class, at)`（`crates/interaction-storage/src/lib.rs:434-463`）。

**排序規則**：一律依 `id`（核心接收序，SQLite 自增），不依裝置時鐘；`source_at` 只是附註，
`query_trace`／`prune_trace` 都不拿它排序或篩選視窗邊界之外的用途。

**身分規則**：`actor` 只寫**已驗證**的身分類別——`human`／`runtime`／`watchdog`／
`agent-session:<id>`／`device:<id>`／`api`（舊）；絕不把呼叫端自報的 id 當成可信身分。
既有 audit 呼叫點的 `actor` 值仍有歷史遺留字串（例：`agent-session.closed` 的 `"user"`、
`hardware.metadata-scan`、`asset.imported/deleted/derived` 固定寫 `"unattributed-*"`）——這是
既有行為，本輪只要求**本輪新寫**的 kind 遵守身分規則，不強制回頭改寫舊呼叫點。

**接收嘗試 vs 業務事件**：`trace` 描述「收到了、正在處理」，重試可以產生多筆；`audit` 描述
「發生了有副作用的事」，同一件事只能有一筆。例如一次 resume 檢查若客戶端重送同樣的請求，
可以有多筆 `trace`，但實際核准／拒絕只留一筆 `audit`。

**未知不補造**：`causation_id`／`trace_id`／`source_at` 找不到就寫 `None`／`NULL`，不得用
猜測值填補；`TraceOutcome::Unknown` 是合法結果，代表「有結果但不知道是什麼」，跟「這筆紀錄
不描述結果」（欄位整個是 `None`）是兩回事。

**相容與 migration 規則**：

- schema 8→9 只用 `ALTER TABLE audit ADD COLUMN`（`crates/interaction-storage/src/lib.rs:434-460`），
  不改動、不刪除既有欄位；migration 前寫入的每一列，新欄位讀出時全部是 `NULL`／`None`。
- 舊列 `class IS NULL` 一律視為 `'audit'`（查詢、prune、計數都用 `COALESCE(class,'audit')`）。
- 反序列化容忍未知欄位：`TraceRecord`／`TraceQuery`／`TraceRow` 都是 `#[serde(default)]`，
  未來多出的欄位會被忽略，不會讓解析失敗（見 `trace.rs` 的
  `deserializing_tolerates_unknown_and_missing_fields` 測試）。
- rollback：舊版 binary 只讀寫 `at`／`kind`／`actor`／`detail` 四個原始欄位，`ALTER TABLE ADD
  COLUMN` 不影響既有欄位的讀寫路徑，因此舊版 binary 仍可正常運作（新欄位對它不存在，等同
  沒發生過這次遷移）。

**Store API**（`crates/interaction-storage/src/lib.rs`）：

| 方法 | 說明 |
|---|---|
| `Store::audit(kind, actor, detail)` / `StoreTxn::audit(...)` | 相容入口，不變；寫 `class='audit', schema=1` |
| `Store::record(&TraceRecord) -> DomainResult<i64>` / `StoreTxn::record(...)` | 寫一筆任意 class 的紀錄，回 row id |
| `StoreTxn::save_agent_session(id, body)` | 與 `Store::save_agent_session` 相同 upsert，可與 `record` 同一 transaction 提交 |
| `Store::query_trace(&TraceQuery) -> DomainResult<Vec<TraceRow>>` | `ORDER BY id DESC`；`limit` clamp `1..=500` |
| `Store::prune_trace(&TraceRetention, now) -> DomainResult<TracePruned>` | 逐 class 依天數＋筆數上限刪除，真的刪了才寫一筆 `trace.pruned` |
| `Store::trace_counts() -> DomainResult<BTreeMap<String,u64>>` | 各 class 筆數，供 status／效能量測 |
| `Store::audit_tail(limit)` | 相容；回傳形狀不變，v9 起多帶 `id`／`class`／`traceId`／`sessionId`／`outcome`／`code`，舊消費者忽略即可 |

`detail` 有界（`bounded_detail`，`crates/interaction-storage/src/lib.rs:78-98`）：序列化後超過
`TRACE_DETAIL_MAX_BYTES`（16 KiB，`crates/interaction-core/src/trace.rs:40`）時，儲存層**不丟棄
整筆紀錄**，改寫成 `{"_truncated": true, "_originalBytes": n, "preview": "<前 2000 字元>"}`；
`preview` 依字元切、JSON 轉義可能讓字元膨脹，超過就把 preview 折半重來，直到真的 ≤ 16 KiB 為止。
`Store::audit`（舊入口）與 `Store::record`／`StoreTxn::record` 共用同一道門。

## 3. 事件與狀態的 canonical owner 表

一個 `kind` 只能有一個權威寫入點；其他地方看到同名字串是投影或消費，不得重複寫入。

| kind | 權威寫入點 | 落地狀態 |
|---|---|---|
| `agent-session.resume-checked` | `crates/interaction-runtime/src/agents.rs::resume_audit_record`（`agents.rs:401`）；呼叫點在 `create_agent_session`（`agents.rs:787,803,819,861`） | **已落地**（D9，`5eff899`）：接受／拒絕／找不到原紀錄三條路徑都寫 audit，`outcome`＝accepted/rejected/ignored（純對話 session 無實際續接時），`code` 有 `resume.ok`／`resume.tools-widened`／`resume.workdir-changed`／`resume.no-record` 等十餘種；`detail` 不含 token，工作目錄只放 `{digest, basename}` |
| `agent-session.dispatched` | `crates/interaction-runtime/src/gateway.rs`（`gateway_attach` 內，`gateway.rs:385`） | **已落地**（`ce68181`）：`class=trace`，無 `outcome`（派送不描述結果）；connector 版本、resume 與否、唯讀／可寫、toolsDisabled、workdir 摘要、三種 scope、ttl／maxCost／maxMessages；`requestedModel` 誠實寫 `"not-specifiable-via-gateway"` |
| `agent-session.task-delivered` | `gateway.rs::gateway_deliver`（`gateway.rs:940`） | **已落地**（`ce68181`）：`class=trace`，無 `outcome`（送達≠完成），`caused_by`＝messageId，帶 contextBundle 的 bundleId／contentHash／bytes／truncated |
| `agent-session.provider-model` | `gateway.rs::set_provider_session_id`（`gateway.rs:740`） | **已落地**（`3c256c7`）：provider 自報的模型寫成一筆 `trace`，同時落到 `AgentSessionRecord.actual_model: Option<String>`（serde default，舊快照讀出 `None`）；讀不到就什麼都不寫 |
| `agent-session.outcome` | `agents.rs::outcome_audit_record`（`agents.rs:1155`），呼叫點在 `report_agent_session`（`agents.rs:1688`）、`estop_mailbox_note`（見下）、`restore_agent_sessions`（`agents.rs:2022`） | **已落地**（`ce68181`）：`outcome.claimed-completed`（`Claimed`，非 `Completed`，`f106899` 修正——claim≠completed）／`outcome.connector-error`（`Failed`）／`outcome.no-result`（`Unknown`）／`outcome.timed-out`（`Expired`）／`outcome.cancelled`／`lease.expired`／`runtime.restarted`（重啟時把還開著的 session 標成結果未知）。高頻 progress 不逐筆寫，只在終態帶 `progressEventsAggregated` |
| `agent-session.interrupt-requested` | `gateway.rs::gateway_interrupt`（`gateway.rs:1178`） | **已落地**（`ce68181`）：`class=audit`，`interrupt.sent`／`interrupt.undeliverable`；actor 依 principal（`human`／`agent-session:<id>`）。留的是 **requested**——「真的停了」由之後的 cancelled 結局來說 |
| `agent-session.subprocess-stderr` | `gateway.rs`（消費 `GatewayEvent::StderrCaptured`，`gateway.rs:666`） | **已落地**（`3c256c7`）：`class=diagnostic`，tail 再收一次至 600 字；刻意排在 `TaskOutcomeUnknown` 與 `SessionClosed` 之間，不觸發／不影響 failed／unknown 判定 |
| `agent-session.capability-issued` | `agents.rs`（`agents.rs:612`） | 既有紀錄；**`9309028` 補強**：改用 `TraceRecord::audit`＋`trace_id`／`session_id`（`outcome=Accepted`，`code=capability.issued`），讓一次工作的完整因果鏈能用同一個 `sessionId` 從簽發查到關閉；寫入仍是關鍵路徑（`store.record(...)?`，fail-closed，不簽發也不留痕跡的憑證）。token 本身不進紀錄 |
| `agent-session.closed` | `agents.rs::close_agent_session`（`agents.rs:1802`） | **已落地並交易化**（`3c8a2a6`）：與 `save_agent_session` 同一 transaction 提交；`actor` 仍寫 `"human"`（原為 `"user"`，未列入本輪身分規則必改項的其餘呼叫點維持現況） |
| `agent-session.emergency-stop` | `agents.rs::estop_mailbox_note`（`agents.rs:1956`） | **已落地**（`ce68181`）：從 `let _ = store.audit(...)` 改走 `record_trace`；先停（`close_agent_session`／`gateway_spawn_kill`）再記 |
| `agent-session.history-pruned` | `agents.rs`（重啟／關閉歷史裁剪路徑，`agents.rs:1151`） | **維持現況**：仍是 `let _ = self.store.audit(...)`（吞錯，不計數）。未列入本輪 S1–S4 清單，留後續階段 |
| `agent-session.verified` | `agents.rs::verify_agent_session`（`agents.rs:1553`） | **已落地並交易化**（`3c8a2a6`）：`store.transaction`，`tx.save_agent_session` 與 `tx.record(verified audit)` 同一筆提交；`outcome=Verified`，備註**內容**不進稽核、只記 `hasNote` |
| `agent.approval` | `gateway.rs::resolve_approval_as`（`gateway.rs:1098`） | **已落地**（`ce68181`）：`store.record(...)?`（關鍵路徑，寫不進去就回錯）；summary 截 200 字，`code`＝`approval.human-approved`／`approval.human-denied`／`approval.watchdog-denied` |
| `consent.granted` | `crates/interaction-runtime/src/runtime.rs`（既有） | 既有 |
| `consent.revoked` | `runtime.rs`（既有） | 既有 |
| `consent.consumed` | `crates/interaction-runtime/src/executor.rs::consume_one_shot_consent`（`executor.rs:432`） | **已落地**（`ce68181`）：`outcome=Completed`，`code=consent.one-shot-spent`，`caused_by`＝當下真的存在的計畫 id。**寫入順序見 §8.1 TB-3**：session 的一次性消耗先 `self.store.upsert_session(session)?`（關鍵、失敗即整個操作回錯），稽核紀錄是**之後**才用 `record_trace` 補寫（非同一 transaction） |
| `consent.rejected` | `runtime.rs::grant_consent_with_uses`（maxUses=0 分支，`runtime.rs:1447`） | **已落地**（`ce68181`）：`record_trace`（非關鍵，只計數） |
| `memory.created` / `memory.deleted` | `crates/interaction-runtime/src/memory.rs` | 既有 |
| `memory.updated` | `memory.rs`（`memory.rs:94`） | **已落地**（`ce68181`）：只記改了哪些**欄位名**（`fields`），不記內容。**寫入順序見 §8.1 TB-3**：`self.persist_memory(&item)?` 先落地（關鍵），稽核紀錄用 `record_trace` 之後補寫 |
| `domain-pack.installed` / `.uninstalled` | `crates/interaction-runtime/src/knowledge.rs` 或 `memory.rs` | 既有 |
| `asset.imported` / `.deleted` / `.derived` | 同上 | 既有；`actor` 固定 `"unattributed-api-caller"`，非本輪必改項 |
| `action.cancelled` / `action.blocked` | `runtime.rs` / `executor.rs` | 既有 |
| `character.*`（hello／adapter 生命週期／manual-intent／receipt／system-text／estop-resync／event-refused／input-capability-not-declared／wire-rejected） | `crates/interaction-runtime/src/character.rs` | 既有（角色域稽核已完整，不在本輪範圍） |
| `mobile.*` | `crates/interaction-runtime/src/mobile.rs` | 既有 |
| `provider.*` | `crates/interaction-runtime/src/{providers,declarative_lifecycle}.rs` | 既有 |
| `aip.*` | `crates/interaction-runtime/src/declarative_session.rs` | 既有 |
| `trace.pruned` | `crates/interaction-storage/src/lib.rs::prune_trace` | 本輪新增，S1 已落地 |

## 4. 三個維度：執行階段（phase）／持久狀態（recordState）／生命週期（lifecycle）／驗證（human_verified）

D10 的根因是把三個不同維度混成一個字串——**已修復（commit `d8f934b`）**。契約把它們拆開：

| 維度 | 定義 | 權威來源 |
|---|---|---|
| **recordState** | `AgentSessionRecord.state`（`AgentSessionState`）持久化的真相狀態，kebab 值：`created｜active｜waiting-for-input｜waiting-for-consent｜claimed-completed｜failed｜timed-out｜cancelled｜expired｜closed｜unknown`。`GET /v1/agent-sessions/{id}` 回的就是這個 | `crates/interaction-core/src/agent.rs::AgentSessionState` |
| **phase** | 「這個工作進行到哪裡」的 taxonomy 字串：`created｜fetched｜working｜verified｜cancelled｜closed｜failed｜unknown｜timed-out｜expired`（未來含 `waiting-input`／`waiting-consent`）。`AgentSessionRecord.phase: Option<String>`（serde default，舊快照讀出 `None`），與 `state` 是兩個不同維度：`state` 是授權狀態機，`phase` 是執行進度 | `crates/interaction-core/src/agent.rs::AgentSessionRecord.phase`；寫入點 `agents.rs::persist_phase`（`agents.rs:1495`，在持有 entry 的臨界區內先落地，再回傳快照給鎖外發事件） |
| **lifecycle** | `open｜closed`，衍生自 recordState 是否為終局 | `agents.rs::emit_agent_session_state_for` 的 SSE payload 欄位（衍生值，不獨立持久化） |
| **human_verified** | 只能經 `verify_agent_session`（人類路徑）產生的布林／狀態；`claimed ≠ verified` 誠實階梯的核心 | `agents.rs::verify_agent_session` |

SSE payload（`agent.session.state`）帶 `{agentSessionId, agentId, state /*相容，一個字沒變*/,
phase, recordState /*AgentSessionState 的 kebab 值*/, lifecycle: "open"|"closed"}`。七個發事件的點
（create／lease 到期／mailbox fetch／gateway 送達／report／verify／close／restore）全部改走
`persist_phase`，**先落地、後發事件**，所以 GET 讀到的 `phase` 與 SSE 送出的 `state` 不會漂開。

**「closed 不覆蓋終局」規則（已修復，同時解決階段 0 D2）**：`close_taxonomy()`
（`agents.rs:517`）依終局逐一對應：`Failed→"failed"`、`Unknown→"unknown"`、
`TimedOut→"timed-out"`、`Cancelled→"cancelled"`、`Expired→"expired"`，其餘才是
`"closed"`；`character::session_projection` 與桌面 `WorkState` union 本來就認得這些字串。

**階段 0 D3 已修復（`ce68181`）**：`safe_summary()`（`agents.rs:454`）把連接器錯誤前 200 字寫進
`AgentSessionRecord.detail`；`close_agent_session` 不再讓收尾覆蓋失敗原因——`detail` 在 prior state
是 Failed 且已有摘要時變成「`<收尾方式>`：`<失敗原因>`」（見 `agents.rs:1782-1786`）。

**已驗證**（各自 commit 附帶的實跑數字，未在本文件重跑）：

- D9：`cargo test -p interaction-runtime --test gateway_loop -- resume`（8 passed／0 failed）＋
  `cargo test -p interaction-runtime --lib -- resume`（3 passed／0 failed）。
- D10：`cargo test -p interaction-runtime --test agents_loop -- phase close_projection`
  （3 passed／0 failed）＋`cargo test -p interaction-runtime --test gateway_loop -- phase`
  （1 passed）＋`-- closing_an_unknown`（1 passed）。
- D16：`cargo test -p interaction-agent-gateway --lib`（32 passed／0 failed）。
- S3 一次交付（dispatched／delivered／outcome／interrupt／approval／consent／memory 等）：
  `ce68181` 附帶的新測試 `a_full_turn_leaves_a_dispatch_delivery_and_outcome_trail`／
  `subprocess_stderr_is_a_diagnostic_record_and_changes_no_outcome`／
  `an_interrupt_is_audited_as_requested_not_confirmed`／`spending_a_one_shot_consent_is_audited`。
- verify／close 交易化：`3c8a2a6` 附帶 `a_failed_commit_rolls_back_the_whole_verification`／
  `a_failed_commit_rolls_back_the_close`（用 `force_next_transaction_error` 真的讓 commit 失敗）、
  `trace_retention_bounds_the_diagnostic_class_and_records_the_prune`、
  `status_reports_trace_write_failures_and_counts`。

## 5. 保存與隱私

**保存期限**（`TraceRetention::default()`，`crates/interaction-core/src/trace.rs`）：

| class | 天數 | 最多筆數 |
|---|---|---|
| audit | 90 | 100 000 |
| trace | 30 | 50 000 |
| diagnostic | 7 | 10 000 |

`prune_trace` 逐 class 先刪超過天數的，再刪超過筆數上限的最舊列（留最新），整批刪除與
「真的刪了才寫」的一筆 `class=audit, kind=trace.pruned, outcome=pruned` 在同一個 transaction
裡提交（`crates/interaction-storage/src/lib.rs:1233-1289`）；空刪不寫，避免 prune 自己變成
無限增長的來源。

**清理時機**（`3c8a2a6`）：`Runtime::prune_trace_records()`（`runtime.rs:644`）在
`restore_agent_sessions` 之後跑一次，之後由看門狗每 600 tick 呼叫一次（`runtime.rs:2543`）；
公開（而不只是看門狗內部呼叫）是為了讓「紀錄有界」能被確定性測試驗證，不必等 wall-clock。
見 §8.1 TB-12：600 tick 之間的窗口內，理論上界仍可能短暫超過筆數上限。

**diagnostic 脫敏規則**（權威實作 `crates/interaction-agent-gateway/src/diagnostics.rs`，D16 已落地，
`1070eca` 補強 stderr 非 UTF-8 讀取）：

- 有界：記憶體只留最後 `TAIL_MAX_BYTES = 4096` bytes 的行，逐行再受 `LINE_MAX_CHARS = 500`
  字元限制；超過即截斷並標 `truncated`。事件用的 tail 另外限 `EVENT_TAIL_MAX_CHARS = 600` 字。
  前 `WARN_LINES = 20` 行以 `warn` 記錄，之後降為 `debug`。收攤等待 reader 收乾有界於
  `READER_DRAIN_TIMEOUT = 1s`，收不乾就用當下快照並照實標 `truncated`。
- **`1070eca`（非本輪 finding，獨立修復）**：讀取一律在 **bytes 層**逐行切（`read_bounded_line`，
  `read_until(b'\n')`＋`LINE_READ_MAX_BYTES = 1 MiB` 的硬上限），轉字串用 `from_utf8_lossy`
  （壞位元組 → U+FFFD）。以前用 `BufReader::lines()`，一個 0xFF 就讓 `next_line()` 回
  `InvalidData`，reader 就此停讀、之後每一行靜默消失，`truncated`／`lines_dropped` 卻停在
  「看完了全部」的值——現在只有真正的 I/O 錯誤才停，停的時候標 `truncated` 並留下
  `StderrSnapshot::read_error`（脫敏過），`lines_dropped` 刻意**不**在這裡加一——I/O 錯誤丟失的
  行數未知，「編一個 1」比「不知道」更像謊言。
- 脫敏在 **push 當下**執行（`redact()`），記憶體裡不存在未脫敏的原文：
  - 去除 ANSI escape（CSI／OSC／單字元）。
  - `Bearer <token>` → `Bearer [redacted]`（保留大小寫拼法）。
  - `Authorization:`／`token=`／`api_key=` 之後的值 → `[redacted]`。
  - 依 token 形狀遮蔽：`iat-session-<hex>`、`iat-<words><hex≥16>`、`sk-*`、`ghp_*`、
    `xox[abp]-*` → `[redacted-token]`。
  - `/Users/<name>/`、`/home/<name>/` → `~`（本機使用者名稱不進診斷紀錄）。
- `TraceRecord.detail` 的一般規則：不得含 token／secret／原始 prompt 全文；工作目錄只放
  `{"digest": sha256 前 12 hex, "basename": ...}`，且序列化後有界於 `TRACE_DETAIL_MAX_BYTES`
  （見 §2；`377863c` 補上這道上限，此前一筆異常巨大的 detail 可以不受控地撐大 DB 與查詢回應）。

**刪除記憶時，舊內容是否真的消失——誠實說明做不到同步刪除**：

- `memory.deleted` 只刪 `memory_items` 表當下那一列；`AgentContextBundleReceipt`
  （`crates/interaction-core/src/agent.rs::AgentContextBundleReceipt`，bounded 32 筆、
  `bundle` 欄位是完整 JSON 快照）一旦生成就是已發出的回執，**不會**因為原 memory 之後被刪除
  而回溯清除——它是那一刻的快照，不是對記憶表的即時視圖。
- agent transcript／provider 私有目錄的紀錄（例如 codex rollout）完全在 repo 控制範圍之外；
  刪除本地 memory 不會、也不能觸及這些外部紀錄（既有已知限制，性質與
  `docs/releases/phase-0-known-issues-reproducibility.md` D8／D9 一帶「依歸檔規則不入 repo」
  的說明相同：provider 私有資料本身就不受這條刪除路徑管轄）。
- 因此：刪除一筆記憶**不保證**它從未出現過的所有 context bundle receipt、audit detail 摘要、
  或 provider 私有 transcript 中被一併抹除。這是誠實的已知限制，不是本輪要解決的範圍。

## 6. 查詢與匯出

**權限**（`crates/interaction-api/src/lib.rs`）：`GET /v1/trace`、`GET
/v1/agent-sessions/{id}/activity` 與 `GET /v1/audit` 一樣**只有 Human**。
`agent_request_allowed`（`lib.rs:495-518`）明確排除 `path != "/v1/trace"` 且
`/v1/agent-sessions` 整個前綴排除；`session_request_allowed`／`adapter_request_allowed` 沒有
對應白名單條目＝一律拒絕。理由：追蹤紀錄是「誰被擋在哪一項、為什麼」的完整授權史，AI 讀得到
就等於讀得到一份現成的規避指南。

**`GET /v1/trace`**（`crates/interaction-api/src/routes.rs::trace_query`）：

```
GET /v1/trace?traceId=&sessionId=&kind=&class=&actor=&outcome=&since=&until=&before=&limit=
```

- 解析規則在 `crates/interaction-runtime/src/activity_trace.rs::TraceQueryInput::into_query`：
  HTTP、Tauri IPC（`trace_query` command）與 CLI（經 HTTP）**共用同一份**驗證，只寫一次。
- 認不得的 `class`／`outcome`／時間字串一律回 `400 Validation`，**不**悄悄忽略——忽略一個篩選
  條件會讓查詢者以為「沒有這種紀錄」。
- `since` 含下界（`at >= since`）、`until` 不含上界；`before` 是分頁 cursor（只回 `id < before`，
  對應 `TraceQuery::before_id`）；`limit` 由儲存層 `effective_limit()` clamp 到 `1..=500`。
- 回應：`{"items": TraceRow[], "nextCursor": number|null, "limit": number}`。`nextCursor` 一律
  出現（`None` 時序列化成明確的 `null`，因為 handler 用 `json!({...})` 手工組裝，不經
  `TracePage` 的 `skip_serializing_if`）。

**`GET /v1/agent-sessions/{id}/activity?before=&limit=`**（`routes.rs::agent_session_activity`）：
回 `AgentSessionActivity`（`activity_trace.rs`）——「這件工作的經過」的人話投影：

```json
{
  "sessionId": "…",
  "headline": "已交給 Codex",
  "stateLabel": "處理中",
  "phase": "working",
  "recordState": "active",
  "lifecycle": "open",
  "failureReason": null,
  "nextStep": null,
  "timeline": [{"at": "…", "id": 42, "kind": "agent-session.dispatched",
                "label": "已交給 Codex", "outcome": null, "code": null,
                "detailAvailable": true}],
  "records": ["…narrowed TraceRow…"],
  "truncated": true,
  "nextCursor": 17
}
```

- session 不存在就是 `404 NotFound`，不回一份空殼假裝有這件工作；被拒絕的續開嘗試沒有 session
  紀錄，它的稽核走 `GET /v1/trace?sessionId=…`。
- `headline`／`stateLabel`／`failureReason`／`nextStep` **不受分頁影響**：各走一次有界的專用
  查詢（最新一筆、最新一筆 `agent-session.outcome`），往回翻頁不會讓「這件工作現在怎麼了」
  跟著變。
- `kind → 人話` 是一張寫死在 `activity_trace.rs::step_label` 的表；認不得的 kind 一律「其他
  紀錄」，絕不把原始字串丟到使用者臉上。`claimed` 投影成「對方說已完成（尚未檢查）」，`verified`
  只認**目前這一輪**的 `claim_id`（`verified_for_current_claim`）。
- diagnostic 類（stderr）在 `records` 裡經 `narrow_diagnostic` 只留脫敏後的 `tail`／
  `truncated`／`linesDropped`，其餘欄位（bytesSeen 等）不外流。
- **已知的序列化不一致（cosmetic，非本輪修復範圍）**：`TracePage.next_cursor` 有
  `#[serde(skip_serializing_if = "Option::is_none")]`，但 `/v1/trace` 的 handler 用 `json!()`
  手工組裝，繞過了這個屬性，所以永遠出現 `"nextCursor"`（`null` 或數字）；
  `AgentSessionActivity` 走 `serde_json::to_value`，`skip_serializing_if` 生效，`None` 時**整個
  鍵被省略**而不是給 `null`。兩個端點對「沒有下一頁」的表示方式不同，消費端需個別處理
  （不得假設兩者一致）。

**CLI**（`crates/interaction-cli/src/main.rs`／`commands.rs`）：

```bash
interact-ai trace --session <id> --trace <id> --kind <kind> --class audit|trace|diagnostic \
  --actor <actor> --outcome <outcome> --since <rfc3339> --until <rfc3339> --before <id> --limit <n>
interact-ai agents activity <id> [--before <id>] [--limit <n>]
```

`trace` 打 `/v1/trace`，`agents activity` 打 `/v1/agent-sessions/{id}/activity`；沒給的條件不送
（送空字串會被後端當成「篩選空字串」，那是另一件事）。兩支都沿用既有 `emit()`（`--json` 單行、
預設 pretty）。人類層 token 才能用；`--agent-scope` 兩支都會被拒絕（`scripts/v03-cli-e2e.sh`
「Trace records」一節，見 `scripts/tests/phase1/README.md` 對照）。

現況可用（不變）：`GET /v1/audit` 回傳形狀是既有 `audit_tail` 的陣列，v9 起多帶
`id`／`class`／`traceId`／`sessionId`／`outcome`／`code`，舊消費者可忽略新欄位。

## 7. 新 Adapter／連接器接入範例

寫紀錄不必手工拼 SQL 或 JSON，直接用 builder。以下兩個範例是 `interaction-runtime` crate 內部
的**真實**呼叫（分別取材自 `gateway.rs::gateway_attach` 與 `agents.rs::verify_agent_session`，
簡化過只保留紀錄相關部分）：

```rust
use interaction_core::{TraceOutcome, TraceRecord};

// trace：接收／路由／派送／進度，允許多筆（例如重試），非關鍵路徑。
self.record_trace(
    TraceRecord::trace("agent-session.dispatched")
        .actor("runtime")
        .trace_id(session)
        .session(session)
        .detail(serde_json::json!({
            "agentId": record.agent_id,
            "providerKind": kind.agent_id(),
            "resume": resume_of.is_some(),
        })),
);

// audit：授權／終態變更，副作用只寫一筆，且與它描述的狀態同一個 transaction；
// commit 失敗時整個操作回 Err，記憶體不會留下一個其實沒落地的「已驗證」。
self.store.transaction(|tx| {
    tx.save_agent_session(id, &body)?;
    tx.record(
        &TraceRecord::audit("agent-session.verified")
            .actor("human")
            .outcome(TraceOutcome::Verified)
            .code("verify.human-confirmed")
            .trace_id(id)
            .session(id)
            .detail(serde_json::json!({ "agentSessionId": id, "hasNote": true })),
    )?;
    Ok(())
})?;
```

builder 只補你給的欄位，其餘一律 `None`／不補造（見 `trace.rs` 的
`builder_fills_only_what_was_given` 測試）。`trace_id` 的 builder 方法叫 `.trace_id(...)`，
不是 `.trace()`——後者是 `TraceClass::Trace` 的建構子（`TraceRecord::trace(kind)`），同一個
inherent impl 不能有兩個同名函式。

**`Runtime::record_trace`**（`runtime.rs:610`，`pub(crate)`）是「非關鍵路徑、失敗只記錄不阻擋」
的便捷入口——寫不進去就記一次 `tracing::error!` 並把 `trace_write_failures`
（`Runtime::trace_write_failures() -> u64`，`pub`，`/v1/status` 讀得到）加一，絕不再往同一儲存
層補寫一筆「剛剛寫失敗了」。同一個計數器也被 `note_storage_write_failure`（session 狀態快照
寫入失敗）與 `prune_trace_records` 的失敗共用——它其實是「非關鍵儲存寫入失敗總數」，不是嚴格
意義上只算 trace 記錄。

**`record_trace` 只是 runtime 內部的私有封裝，不是公開 API**：`interaction-runtime` crate 外的
新 adapter／連接器要走 `Store::record`／`StoreTxn::record`（透過 `Runtime` 對外暴露的方法，或
直接持有 `Store` 的呼叫端）。crate 內部（gateway／agents／executor／memory 等既有消費點）才能
直接呼叫 `self.record_trace(...)`。

## 8. 寫入失敗政策表

| 場景 | 政策 | 依據 |
|---|---|---|
| resume **接受**（關鍵轉移） | `store.record(...)?`——稽核寫不進去就不放行，操作回 `Err`（「記不下來的授權不算授權」） | **已落地**（`5eff899`） |
| resume **拒絕／忽略** | `record_trace`——只計數，絕不讓一次寫入失敗把拒絕變成放行 | **已落地**（`5eff899`／`ce68181`） |
| `verify_agent_session`／`close_agent_session` | `store.transaction`：狀態＋稽核同一 transaction，commit 失敗整個回滾、操作回 `Err`，記憶體不寫回新 record | **已落地**（`3c8a2a6`） |
| `agent.approval`（人工核可／拒絕、watchdog 拒絕） | `store.record(...)?`——與 resume 接受同級：這是授權變更，不取樣、不漏 | **已落地**（`ce68181`） |
| `agent-session.capability-issued`（憑證簽發） | `store.record(...)?`——寫不進去就不簽發（fail-closed），token 本身不進紀錄 | **已落地**（`9309028`） |
| `restore_agent_sessions` 的狀態寫入（重啟時把仍開啟的 session 標成 Expired／unknown） | 走 `persist_agent_session`（非關鍵，失敗計數＋log），與其他所有狀態寫入同一條路徑 | **已落地**（`711de77`，見 §8.2 TB-4） |
| `agent-session.dispatched`／`.task-delivered`／`.provider-model`／`.subprocess-stderr`（diagnostic）／`.interrupt-requested`／`.outcome`（含 restore 的 `runtime.restarted`）／`.emergency-stop`／`consent.rejected` | `record_trace`——非關鍵路徑，寫不進去只記一次 `tracing::error!`＋`trace_write_failures` 計數，**不**遞迴補寫；操作本身照常繼續 | **已落地**（`ce68181`／`3c256c7`） |
| Emergency stop | 一律先停（`close_agent_session`／`gateway_spawn_kill`）再記（`estop_mailbox_note` 走 `record_trace`）；記錄失敗**不阻擋**停止本身 | 契約規則 4；`ce68181` |
| `consent.consumed`／`memory.updated` | **見 §8.1 TB-3**：狀態本身（session 一次性消耗、memory 內容）先關鍵寫入（`?`），稽核紀錄是**之後**才用 `record_trace` 補寫——不是同一個 transaction | **已知限制**，非本輪修復範圍 |
| `sensor_journal` 停止提醒的增刪與 audit | 同一 SQLite transaction（`sensor_journal.rs:374`），失敗回滾解除、保留記憶體提醒、回應失敗而非假成功 | 既有實作，非本輪新增 |
| `agent-session.history-pruned` 與其餘未列入本輪清單的既有 `let _ = store.audit(...)` 調用點 | 維持現況（吞錯，不計數） | 範圍界定；未列入本輪清單的部分不得誤稱已修 |

### 8.1 已知限制（本輪未處理，維持現況）

- **TB-3**：`consent.consumed`（`executor.rs::consume_one_shot_consent`）與 `memory.updated`
  （`memory.rs`）的稽核紀錄是「狀態已經落地之後」才盡力補寫的 `record_trace` 呼叫，不是與狀態
  同一個 transaction。極端情況下（狀態寫入成功、稽核寫入恰好失敗）會出現「消耗／修改確實發生，
  但稽核列缺席」——`trace_write_failures` 計數器會反映這一格增加，但查不到那一筆具體紀錄長什麼
  樣子。
- **TB-7**：`report_agent_session`（`agents.rs`）在持有全域 `agent_sessions` 寫鎖（`RwLock`）的
  臨界區內同步呼叫 `persist_agent_session`（SQLite 寫入）。debug build 量測單筆 autocommit 約
  100–125 µs（見 §9），高並發下這段鎖持有時間會隨磁碟 I/O 波動放大，尚未做成批次或鎖外提交。
- **TB-8**：agent 自我回報（`report_agent_session`）沒有 idempotency key；同一個 provider 事件
  重送會產生新的 `claim_id`（新一輪「已完成」聲稱），舊的 `human_verified` 因為
  `claim_id` 不匹配而失效（`verified_for_current_claim` 判定）。這是刻意的 fail-closed（寧可要求
  重新驗證，不可能讓舊的綠勾誤套在新的聲稱上），但代價是重送本身不可觀測為「這是重複事件」。
- **TB-12**：`prune_trace_records()` 只在啟動（`restore_agent_sessions` 之後）與看門狗每 600
  tick 執行一次，不是即時觸發。兩次清理之間，任一 class 的實際筆數理論上界可能短暫超過
  `TraceRetention` 設定的上限（見 §5）。

### 8.2 已修復（由獨立懷疑者發現，另一個 agent 修復）

以下三項最初由獨立懷疑者複核發現，撰稿過程中持續追蹤，**在文件收尾前由另一個 agent 修復並
commit**——與 D9／D10／D16 一樣，逐項核對程式碼現況與測試，不是抄錄 commit 訊息：

- **TB-1（dataScope 完整路徑外洩風險）——已修（`ba01c0a`）**：`resume_audit_record`
  （`agents.rs::resume_audit_record`）與 `gateway_attach` 的 dispatched 紀錄
  （`gateway.rs::gateway_attach`）以前直接寫入 `record.data_scope: Vec<String>` 原文；真實用法
  是 `workspace:<完整絕對路徑>`（桌面端 `AiPage` 一直這樣組），於是使用者的目錄結構被逐字留在
  SQLite 裡。新增純函式 `agents.rs::safe_scope_list`：`workspace:` 前綴（與任何帶 `/` 的值）
  改寫成與 `workdir_digest` 同一套規則的 `{digest 12 hex}/{basename}`，`domain:health` 這種不含
  路徑的標籤照抄；套用在 `resume_audit_record` 的 `requested.{dataScope,toolScope,consentScope}`、
  `ResumeComparison::to_detail` 的 `*ScopeAdded`、`gateway_attach` 的 dispatched 三個 scope 欄位。
  另外補一個同類漏洞：「範圍放寬」拒絕的對外錯誤文案帶著呼叫端原樣送來的 scope 標籤，會被當成
  `reason` 寫進稽核 detail——`ResumeRejection` 新增 `audit_reason` 欄位，對外文案一字不變，進
  紀錄的那一份過 `safe_scope_list`。單元測試：`scope_labels_keep_paths_out_of_the_record`
  （`workspace:/Users/x/proj` 脫敏後不含 `/Users`、仍看得出 basename `proj`、digest 穩定且不同
  路徑不撞號；`domain:health` 不變）。
- **TB-4（restore 繞過失敗計數）——已修（`711de77`）**：`restore_agent_sessions` 把重啟前仍開啟
  的 session 標成 `Expired`／`phase=unknown` 時，以前用 `let _ = store.save_agent_session(...)`
  落地——寫不進去（磁碟滿、DB 鎖住）完全靜默。改走既有的 `persist_agent_session`（失敗會呼叫
  `note_storage_write_failure`，計進 `trace_write_failures`），與其他所有狀態寫入同一條路徑；
  `restore_agent_sessions` 改為 `#[doc(hidden)] pub`（純測試接縫，HTTP／CLI 介面碰不到）讓這條
  不變量能被確定性驗證。測試：`a_failed_restore_write_is_counted_not_swallowed`（正常 restore
  的 `traceWriteFailures` 仍是 0、結果真的落地；用 `Store::force_next_agent_session_save_error`
  真的讓寫入失敗時計數變 1）。
- **TB-5（`resume.ok` 早於後續上限檢查）——已修（`711de77`）**：`create_agent_session` 以前在
  resume 接受分支的比對通過後**立刻**寫下 `resume.ok` 的稽核，但後面還有委派與 open session
  上限檢查（`check_delegation` 等）可能把這次建立擋掉——寫早了會在 DB 裡留下一筆「接續已核准」
  卻從來沒有對應 session 的紀錄。改成先把判定結果收進區域變數 `accepted_resume`，等所有上限
  檢查都通過、session 確定會被建立才寫；拒絕／no-record／ignored 三條路徑不受影響（當場
  `return`，寫入位置不動）。接受仍是關鍵轉移：稽核寫不進去就不放行（`store.record(...)?`）。
  測試：`a_resume_blocked_by_the_session_limit_leaves_no_accepted_row`（`maxSessions=1`、對還
  開著的 session 做完全誠實的續開比對 → 因上限被 403 擋下 → `query_trace(kind=resume-checked,
  outcome=accepted)` 必須是空的）。

## 9. 效能與預算

以下數字取自 `crates/interaction-storage/tests/trace_store.rs::perf_trace_write_and_query_baseline`
（**debug build**，本機 macOS arm64，`cargo test -p interaction-storage --test trace_store --
--ignored --nocapture`；由撰寫 S1 的 agent 實跑，本文件未重跑，逐筆數字取自該次量測）：

```json
{
  "writeAutocommit": { "avgUs": 100.79, "rows": 20000, "totalMs": 2015.75 },
  "writeTransaction": { "avgUs": 22.71, "rows": 1000, "totalMs": 22.71 },
  "queryAvgMs": {
    "byKind": 0.286, "bySessionId": 0.292, "byTraceId": 0.285,
    "tail50": 0.536, "iterations": 100
  },
  "queryRows": { "byKind": 50, "bySessionId": 50, "byTraceId": 50, "tail50": 50 },
  "prune": {
    "countsBefore": { "audit": 7000, "trace": 7000, "diagnostic": 7000 },
    "countsAfter": { "audit": 1001, "trace": 1000, "diagnostic": 1000 },
    "removed": { "audit": 6000, "trace": 6000, "diagnostic": 6000 },
    "ms": 129.74
  },
  "dbBytes": { "main": 5898240, "wal": 5965792, "total": 11864032 }
}
```

換算：逐筆（autocommit）寫入約 101 µs／筆；同一 transaction 內的批次寫入約 23 µs／筆；
`query_trace` 依 `kind`／`sessionId`／`traceId`／`tail(limit=50)` 四種查詢平均 0.29–0.54 ms；
`prune_trace` 一次刪掉 18 000 列（三個 class 各 6000）耗時約 130 ms；21 000 列（prune 前）時
主 DB 檔約 5.9 MB、WAL 約 6.0 MB。

**效能預算（整合者核定，debug build 基線之上留的安全邊際）**：

| 項目 | 預算 | 依據 |
|---|---|---|
| 單筆寫入（autocommit） | < 1 ms | 量測值約 0.1 ms，留 10 倍邊際給 release build 與磁碟壓力波動 |
| 查詢（500 筆內） | < 5 ms | 量測值 0.29–0.54 ms（50 筆），留邊際給 500 筆上限與較冷的頁快取 |
| 每 session 的 trace 列數 | < 60 | 一次典型的短任務（resume 檢查、dispatched、task-delivered、provider-model、若干 approval、outcome、closed）不應超過這個量級；持續超過代表某個高頻事件被錯誤地逐筆寫進 `trace` 而非彙整 |
| audit 表上限（依 class） | 依 `TraceRetention::default()`（audit 100 000／trace 50 000／diagnostic 10 000） | 見 §5；`prune_trace_records()` 的清理時機見 §5 與 §8.1 TB-12 |

以上是 **debug build**、單機本地 SQLite 的量測，不代表 release build 或正式負載下的表現；
release build 量測留給後續階段，本文件不據此下任何「達標／未達標」的結論。
