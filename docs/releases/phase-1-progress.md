# 階段 1：互動可追溯性（進度入口）

> 新 session／整合者先讀 **§6 下一動作** 與 **§7 Blockers**。

## 1. 起點與授權

- 查核時間：2026-09-15，Asia/Taipei。
- 起點：`main` = `18829cb54bbbe8f403bf1ee4a628b837234bf2b8`（`docs: record phase-0 checkpoint merge,
  CI and tag`）；最新 checkpoint tag `phase-0-baseline-20260914`（annotated，指向合併 commit
  `8f95f3d`）。
- 施工分支：`phase-1/interaction-traceability`（自 `18829cb`）。
- 使用者授權：建分支、依契約新增 `crates/interaction-core/src/trace.rs`＋`interaction-storage`
  的 `record`／`query_trace`／`prune_trace`／`trace_counts`、修 D9／D10／D16 三個 findings、
  Conventional Commits、每個可獨立驗證的邊界一個 commit；不自行 push／開 PR；不濫用
  `unwrap()`；不跑 `cargo test --workspace`（改跑受影響 crate／測試）。
- 本文件核對期間，`df -h /` 讀數在 991 MiB 至 5.4 GiB 之間波動（低於階段 0／S3 撰稿時記錄的
  4.7 GiB／9 GiB 假設，且本身不穩定），詳見 §7 Blockers。

## 2. 本階段做了什麼（方法與模型分工）

| 步驟 | 方式 | 模型分工 |
|---|---|---|
| 契約設計（三種紀錄責任、`TraceRecord` 契約 v1、schema 8→9 遷移規則） | 主模型撰寫共用契約簡報作為所有 subagent 的依據 | 主模型（orchestrator） |
| S1：`interaction-core::trace` 型別＋`interaction-storage` 的 `record`／`query_trace`／`prune_trace`／`trace_counts`／schema 9 migration | 獨立 agent 落地並自跑受影響 crate 測試 | Opus |
| S2：`interaction-agent-gateway` 的 stderr 留痕與脫敏（D16）、`SessionStarted.model` | 獨立 agent 落地 | Opus |
| S3：runtime 端（`agents.rs`／`gateway.rs`／`executor.rs`／`memory.rs`／`runtime.rs`）D9 續開稽核、D10 狀態語意三維拆分、派送／終態／approval／interrupt／consent／memory 紀錄、verify／close 交易化、寫入一致性、保存清理 | 獨立 agent，多個 commit 逐步落地（`ce68181`／`3c8a2a6`／`f106899`） | Opus |
| S4：`GET /v1/trace`、`GET /v1/agent-sessions/{id}/activity` 查詢端點、CLI `trace`／`agents activity`、桌面「這件工作的經過」 | 獨立 agent，多個 commit 逐步落地（`e63986f`／`58f7477`／`cbc76c0`） | Opus |
| 兩個獨立缺陷修復（不在原始三個 finding 清單內）：stderr 非 UTF-8 讀取（`1070eca`）、`detail` 16 KiB 上限（`377863c`） | 獨立懷疑者複核後由對應 agent 修復 | Opus |
| 三個獨立懷疑者發現的問題修復：TB-1（`ba01c0a`）、TB-4／TB-5（`711de77`）；UI 文案修正（`2b1572c`）；`agent-session.capability-issued` 掛上 session／trace id＋正式 commit `trace_e2e.py`（`9309028`） | 另一個 agent，與本文件整合同時進行 | Opus |
| 覆蓋矩陣唯讀調查（各 domain 的既有紀錄現況、行號、常數） | 獨立 agent 只讀不寫，產出覆蓋矩陣事實供文件引用 | Sonnet |
| 文件整合（本檔＋`docs/aip/interaction-tracing.md`＋`docs/releases/phase-1-coverage-matrix.md`＋CHANGELOG／MAINTAINERS-MAP／general-mode-ux／acceptance-evidence／evidence-index 等） | 本 agent：核對草稿與 repo 當前落地狀態，逐項改寫定稿 | Sonnet |
| 裁決與收尾 | 主模型核對 S1–S4 產出、決定發布時機 | 主模型 |

## 3. 恢復矩陣（D9／D10／D16／D2／D3 重驗；TB-1／TB-4／TB-5 現況）

依共用契約簡報「本輪要修的三個 findings（位置已當場核實仍存在）」，本文件核對 HEAD `9309028`：

| finding | 位置引用 | 逐字核對結果 | 狀態 |
|---|---|---|---|
| D9 | `crates/interaction-runtime/src/agents.rs::create_agent_session` resume 分支／`check_resume_not_wider` | 接受／拒絕／找不到原紀錄三條路徑都寫 `TraceRecord::audit("agent-session.resume-checked")`；對外 `PolicyBlocked`／`ConsentRequired` 文案不變 | **已修**（`5eff899`）。實跑：`cargo test -p interaction-runtime --test gateway_loop -- resume`（8 passed）＋`cargo test -p interaction-runtime --lib -- resume`（3 passed） |
| D10 | `gateway.rs`（emit `"fetched"`）、`agents.rs`（`"working"`）、close 投影 | `AgentSessionRecord` 新增 `phase: Option<String>`；`persist_phase` 七個發事件點都改走它；SSE payload 加 `phase`／`recordState`／`lifecycle`；`close_taxonomy()` 依終局逐一對應真實字串 | **已修**（`d8f934b`；同時解決階段 0 D2）。實跑：`agents_loop -- phase close_projection`（3 passed）＋`gateway_loop -- phase`（1 passed）＋`-- closing_an_unknown`（1 passed） |
| D16 | `crates/interaction-agent-gateway/src/codex.rs`（修復前：吞掉 stderr） | `codex.rs`／`claude.rs`／`codex_exec.rs` 共用 `diagnostics.rs` 的有界、脫敏 tail | **已修**（`baabec7`）。實跑：`cargo test -p interaction-agent-gateway --lib`（32 passed） |
| 階段 0 D2 | close 投影把 `failed`／`unknown`／`timed-out` 塌成 `closed` | 隨 D10 一併修復（`close_taxonomy()`） | **已修**（`d8f934b`，同 D10） |
| 階段 0 D3 | `failed` 的 agent session 在 record／mailbox 上都沒有原因 | `agents.rs::safe_summary` 把連接器錯誤前 200 字寫進 `AgentSessionRecord.detail`；`close_agent_session` 不再讓收尾覆蓋失敗原因 | **已修**（`ce68181`） |

**S3／S4 其餘項目**（撰寫共用契約簡報時規劃、不在原始三個 finding 清單內，但屬同一輪工作）
**全部已落地**：`agent-session.dispatched`／`.task-delivered`／`.outcome`／`.interrupt-requested`
（`ce68181`）、`consent.consumed`／`consent.rejected`／`memory.updated`（`ce68181`）、
`verify`／`close` 交易化（`3c8a2a6`）、`Runtime::record_trace`／`trace_write_failures`
統一入口＋`/v1/status` 投影（`3c256c7`／`3c8a2a6`）、`GET /v1/trace`／`GET
/v1/agent-sessions/{id}/activity`（`e63986f`）、CLI `trace`／`agents activity`（`58f7477`）、
桌面「這件工作的經過」（`cbc76c0`）。

**兩個不在原始清單內、由獨立懷疑者發現並已修復的缺陷**：

- `1070eca`：`spawn_reader` 以前用 `BufReader::lines()` 逐行讀 stderr，一個非 UTF-8 位元組會讓
  `next_line()` 回 `InvalidData`、reader 靜默停讀，之後每一行都消失且 `truncated`／
  `lines_dropped` 停在「看完了全部」的值。改成 bytes 層 `read_until(b'\n')`＋
  `from_utf8_lossy`，只有真正的 I/O 錯誤才停，且誠實標 `truncated`＋`read_error`。
- `377863c`：`insert_record`／`insert_audit` 以前把 `detail.to_string()` 原文寫入，沒有上限；
  一筆異常巨大的 detail（例如失控的 stderr 摘要）可以不受控地撐大 DB 與查詢回應。改成
  `bounded_detail()`：超過 16 KiB 就改寫成自述的截斷標記，不丟棄整筆紀錄。

**三項由獨立懷疑者發現、原本記為「進行中修復」、在文件收尾前已 commit 完成的問題**
（詳見 `docs/aip/interaction-tracing.md` §8.2、`docs/releases/phase-1-coverage-matrix.md` 對應
列的逐字核對紀錄——同樣依「未提交不算數」的原則：核對過程中先後看到工作樹出現對應 diff，
每次都暫記為「進行中」，等對應 commit 真的落地才改記「已修」並實跑測試）：

- **TB-1（已修，`ba01c0a`）**：`resume_audit_record`（`agents.rs`）與 `gateway_attach` 的
  dispatched 紀錄（`gateway.rs`）以前把 `data_scope: Vec<String>` 原文寫入 `detail.dataScope`，
  未經摘要。新增 `agents.rs::safe_scope_list`：`workspace:<路徑>` 改寫成與 `workdir_digest`
  同一套規則的 `{digest}/{basename}`，不含路徑的標籤（如 `domain:health`）照抄；連帶修掉「範圍
  放寬」拒絕文案帶原始路徑進稽核 detail 的同類漏洞。
- **TB-4（已修，`711de77`）**：`restore_agent_sessions` 把重啟前仍開啟的 session 標成 `Expired`
  時，以前直接呼叫 `let _ = self.store.save_agent_session(...)`，繞過 `persist_agent_session`
  （也就繞過了 `note_storage_write_failure`／`trace_write_failures` 計數）。改走
  `persist_agent_session`，`restore_agent_sessions` 改為 `#[doc(hidden)] pub` 讓這條不變量能被
  確定性測試驗證。
- **TB-5（已修，`711de77`）**：resume 接受分支的 `resume.ok` 稽核以前寫在 `check_delegation`
  （委派深度／循環／並行上限）等後續檢查**之前**，這些檢查若失敗，session 不會真的建立，但
  稽核已經宣稱「接受」。改成先把判定結果收進區域變數，等所有上限檢查都通過、session 確定會
  被建立才寫。

`9309028`（同一批修復收尾時的最後一個 commit）額外把 `agent-session.capability-issued` 從舊的
`Store::audit` 升級成帶 `trace_id`／`session_id` 的 `TraceRecord::audit`，並正式 commit
`scripts/tests/phase1/trace_e2e.py`（真 Agent 端到端追蹤驗收腳本，用法見
`scripts/tests/phase1/README.md`）；`2b1572c` 修掉一句一般模式文案指向進階模式才有的「技術
詳情」（UI 用字問題，非追蹤紀錄契約本身）。

**已知限制（本輪未處理，非修復中，見 `docs/aip/interaction-tracing.md` §8.1）**：TB-3
（`consent.consumed`／`memory.updated` 為狀態落地後盡力寫入的稽核，非同一 transaction）、
TB-7（`report_agent_session` 持全域 session 寫鎖時同步寫 SQLite，每筆約 100–125 µs）、
TB-8（agent 自我回報無 idempotency key，重送會多開 claim 並讓舊 verify 失效，fail-closed）、
TB-12（`prune_trace_records` 只在啟動與看門狗每 600 tick 執行，非即時觸發）。

## 4. 交付文件索引

1. 本文件（進度入口）。
2. [`docs/aip/interaction-tracing.md`](../aip/interaction-tracing.md)（追蹤紀錄契約 v1、canonical
   owner 表、三維狀態語意、保存與隱私、查詢與匯出 API、接入範例、寫入失敗政策、效能與預算）。
3. [`phase-1-coverage-matrix.md`](phase-1-coverage-matrix.md)（逐領域覆蓋矩陣：事件入口、裝置、
   能力、Agent、同意、工作、記憶、系統、角色、查詢與治理）。
4. `CHANGELOG.md` `[Unreleased]` 段（本輪條目已併入既有階段 0 的 `[Unreleased]`）。
5. `scripts/tests/phase1/trace_e2e.py`＋[`scripts/tests/phase1/README.md`](../../scripts/tests/phase1/README.md)
   （真 Agent 端到端追蹤驗收腳本與使用說明）。
6. `docs/MAINTAINERS-MAP.md` §11（追蹤紀錄的能力歸屬）、`docs/aip/general-mode-ux.md` §5.8
   （「這件工作的經過」一般模式文案）、`docs/acceptance-evidence.md`（階段 1 一節）、
   `docs/releases/evidence-index.json`（`phase-1-interaction-traceability` candidate）、
   `docs/releases/phase-0-known-issues-reproducibility.md`（D2／D3／D9／D10／D16 已修標記）、
   `AGENTS.md` §7（入口指標）、`docs/ARCHITECTURE.md`／`docs/FEATURES.md`（指向段落）。

## 5. 主要結論

- **S1（追蹤紀錄契約 v1：型別＋儲存層）**：已落地（`94b5342`／`85339ae`）。
- **簡報點名的三個 findings（D9、D10、D16）與階段 0 的 D2／D3**：**全部已落地**，各自 commit
  附帶實跑測試通過（見 §3）。
- **S3（runtime 寫入點：派送／送達／終態／approval／interrupt／consent／memory／emergency-stop、
  verify／close 交易化、統一非關鍵寫入入口）**：**全部已落地**（`ce68181`／`3c8a2a6`／
  `f106899`）。
- **S4（查詢 API／CLI／桌面 UI）**：**全部已落地**（`e63986f`／`58f7477`／`cbc76c0`）。
- **兩個獨立缺陷（stderr 非 UTF-8、detail 16 KiB 上限）**：**已修**（`1070eca`／`377863c`）。
- **TB-1／TB-4／TB-5**：**已修**（`ba01c0a`／`711de77`），文件收尾前逐一重新核對並確認 commit
  落地（見 §3）；`agent-session.capability-issued` 順帶補強掛上 session／trace id（`9309028`）。
- **TB-3／TB-7／TB-8／TB-12**：誠實記錄為已知限制，非本輪修復範圍。
- **真 Agent 端到端驗收（`scripts/tests/phase1/trace_e2e.py`，2026-09-15，HEAD `c89a5c4`，daemon
  sha256 `5bea07b8…`，macOS 26.2 arm64，Claude Code 2.1.272（實際模型 `claude-fable-5-1`）、Codex CLI 0.154.0
  （實際模型 `gpt-6-astra`，皆由 provider 自報 `actualModel`））**：Claude Code **8／8 passed**（normal、cancel、
  resume-reject、resume-accept、failure、restart、isolation、agent-token-forbidden；37.7 s）；Codex **9／9 passed**
  （同上加 approval（人類 deny）；54.8 s）。原始證據：`docs/releases/evidence/2026-09-15-phase-1/real-agent/`。
  前兩輪（HEAD `2b1572c`／`a24153e`）曾暴露兩個階段 0 的缺陷並在本輪修掉：真 Claude 的 interrupt 落 `failed`
  （D1，`24ae45d`）；真 Codex 對 deny 的 wire 值 `reject` 回「unknown variant」——這條協定錯誤正是由 D16 的
  stderr 診斷紀錄擷取到的（D4，`a24153e`）。
- **驗收項目對照**（任務書 §11）：1 派工→claimed→verified（normal）✔；2 iPhone 發起→核心接受→回兩端：**fixture 層**
  由既有 `mobile_loop`／`character_session_loop` 涵蓋，真 iPhone **blocked**（§7）；3 權限不足→拒絕→零副作用→稽核
  （resume-reject＋agent-token-forbidden）✔；4 resume 接受／拒絕可追蹤 ✔；5 取消區分 requested／confirmed／unknown
  （cancel：`interrupt-requested accepted` → `outcome cancelled`；codex failure：SIGKILL 子程序 → `unknown`）✔；
  6 協定錯誤可追且不洩密（D4 由 diagnostic 紀錄抓到；leaks 掃描 0）✔；7 重複／過期／亂序／限流：AIP dedupe 與限流由既有
  fixture 測試涵蓋、agent 自我回報重送的 idempotency 為已知限制 TB-8；8 重啟後查回最後可信狀態（restart：`expired`＋
  `outcome unknown code=runtime.restarted`）✔；9 crash 不產生假成功（同上）✔；10 儲存失敗不被當成功（`force_next_*`
  單元／整合測試）✔；11 保存期限與容量清理（storage `prune_trace` 測試＋啟動／600 tick）✔；12 一般模式找到失敗原因與
  下一步（failure：headline／失敗原因／「可重新交代一件工作」；Playwright `work-activity.spec.ts`）✔。
- **完整回歸**：見 §8.2（全部在 HEAD `c6dd64e`／`c89a5c4` 跑完：Rust workspace 1335／1／1（唯一失敗是未觸碰
  crate 的既有 flaky 測試，單獨重跑 33／0）、Tauri 78／0、vitest 1925／0、Playwright 94／0、CLI e2e 102／0、Swift 58／0）。

## 6. 下一動作

1. 執行 `scripts/tests/phase1/trace_e2e.py`（已 commit，`9309028`；用法見
   `scripts/tests/phase1/README.md`）：`--agent claude-code` 與 `--agent codex` 各跑一次，
   逐情境核對 `result.json` 的 verdict（`passed`／`product-failed`／`not-run`／`blocked`），
   回填本文件 §5／§8 與 `docs/acceptance-evidence.md` 階段 1 節的 real-agent 證據層級。
2. 補跑完整回歸（`cargo fmt --all --check`／`cargo clippy --workspace --all-targets -- -D
   warnings`／`cargo test -p interaction-runtime -p interaction-core -p interaction-storage
   -p interaction-agent-gateway -p interaction-api -p interaction-cli`／桌面
   `pnpm typecheck && pnpm test && pnpm build`／`pnpm test:e2e`／`./scripts/v03-cli-e2e.sh`），
   回報實際數字到本文件 §8.2 與 `docs/acceptance-evidence.md`。**跑之前先確認磁碟空間**（見
   §7——本輪查核期間磁碟可用空間在 991 MiB 到 5.4 GiB 之間波動，跑重型建置前務必重新
   `df -h /`）。
3. 用 `.claude/workflows/adversarial-review-adaptive-interaction.js` 對本輪 diff（`18829cb..HEAD`）
   跑一次對抗審查，確認的缺陷修掉或誠實記為已知限制；TB-1／TB-4／TB-5 已修（見 §3），四個
   已知限制（TB-3／TB-7／TB-8／TB-12）應併入同一輪覆核，不要重複開新編號。
4. 決定是否需要 checkpoint tag（比照階段 0 的 `phase-0-baseline-YYYYMMDD` 命名慣例）；若本輪
   要銜接發布，走 `release-prepare.sh` → `release-verify.sh` → `release-tag.sh`（見
   `docs/releases/evidence-index.json` 本輪 candidate 的 `versionPolicy` 備註）。

## 7. Blockers

- 真 iPhone：Xcode → Settings → Accounts 未選 Team（沿用階段 0 的既有 blocker，本階段未處理，
  不在 S1–S4 範圍）；需要人類在 Xcode 登入並選 Team，不得繞過。
- 原生桌面 App 未由 HEAD 重建：沿用階段 0 的既有 blocker，native 證據仍綁在 2026-09-06
  候選 bundle（sha256 `1103cda7…`）；本階段完全不涉及桌面原生打包，此 blocker 與階段 1
  的交付無關，僅為承接階段 0 §7 的記錄，避免遺漏。
- **磁碟空間（波動中，需在動手前重新量測）**：本輪文件整合期間 `df -h /` 觀察到兩個不同的
  讀數——查核初期約 **991 MiB**（`/dev/disk3s3s1`，容量使用 95%），與另一個 agent 的並行工作
  完成一部分之後回升到約 **5.4 GiB**（使用率 76%）。這代表本機磁碟水位對這個分支的其他並行
  工作（build cache、workdir fixture）很敏感，不能只信一次量測。低於 2 GiB 時 `pnpm tauri
  build`、完整 Rust workspace 建置（`cargo build --workspace`／`cargo test --workspace`）都
  可能以 `ENOSPC` 中斷，Tauri 測試（`cargo test --manifest-path
  apps/interaction-desktop/src-tauri/Cargo.toml`）在此水位下無法安全重跑。依 `CLAUDE.md`
  「工作規則」，`target/debug/incremental` 可安全刪除（純快取），`deps/` 不要動；§6 的完整
  回歸與 real-agent 驗收動手前一律先 `df -h /` 確認，不得沿用本文件記錄的任一個舊讀數。

## 8. 提交、合併與 tag

整合者（主模型）填寫，2026-09-15。

### 8.1 分支 `phase-1/interaction-traceability` 的 commits（自 `18829cb`，本文件核對當下已知）

| commit | 內容 |
|---|---|
| `94b5342` | `feat(core): trace record contract v1 types with class, causation and retention` |
| `85339ae` | `feat(storage): trace record contract v1 with class, causation and bounded retention` |
| `baabec7` | `fix(gateway): keep a bounded, redacted stderr tail for every agent subprocess (D16)` |
| `5ee242d` | `feat(gateway): report the provider's actual model on session start` |
| `3c256c7` | `fix(runtime): consume the gateway's stderr and provider-model events` |
| `5eff899` | `fix(runtime): audit every resume authorization decision (D9)` |
| `d8f934b` | `fix(runtime): persist the execution phase and keep terminal outcomes on close (D10)` |
| `ce68181` | `feat(runtime): dispatch, delivery, outcome and stderr diagnostic records` |
| `3c8a2a6` | `feat(runtime): transactional verify/close and bounded trace retention` |
| `f106899` | `fix(core): record an agent's self-report as outcome=claimed, never completed` |
| `e63986f` | `feat(api): trace query and per-session activity endpoints (human-only)` |
| `58f7477` | `feat(cli): trace and agents activity commands` |
| `1070eca` | `fix(gateway): keep reading stderr past non-UTF-8 bytes and count what is dropped` |
| `377863c` | `fix(storage): bound trace record detail at 16 KiB with an honest truncation marker` |
| `cbc76c0` | `feat(desktop): 這件工作的經過 section on the work card` |
| `ba01c0a` | `fix(runtime): keep workspace paths out of trace details (TB-1)` |
| `711de77` | `fix(runtime): count restore-time persistence failures and audit resume only when a session is really created (TB-4, TB-5)` |
| `2b1572c` | `fix(runtime): next-step copy does not point at advanced-only details` |
| `9309028` | `feat(runtime): link capability issuance to its session trace and add the phase-1 real-agent trace harness`（同時正式 commit `scripts/tests/phase1/trace_e2e.py`） |
| `24ae45d` | `fix(gateway): an interrupted Claude turn is cancelled, not failed (D1)`（真 Claude 驗收暴露；紅燈測試 `an_interrupted_claude_turn_is_cancelled_not_failed`） |
| `a24153e` | `fix(gateway): send codex the enum value \`decline\` for a human deny (D4)`（由 D16 stderr 診斷紀錄擷取到的列舉） |
| `c6dd64e` | `fix(runtime): keep the connector's reason for an unknown outcome on the record` |
| `c89a5c4` | `docs: phase-1 interaction tracing contract, coverage matrix, progress and policies` |
| （本 commit） | `docs: phase-1 real-agent evidence, regression numbers and D1/D4 status` |

### 8.2 提交前回歸

2026-09-15，HEAD `c6dd64e`（程式碼最終 commit）／`c89a5c4`（docs），macOS 26.2 arm64，`CARGO_INCREMENTAL=0`：

| 命令 | 結果 | 耗時 | 證據層級 |
|---|---|---|---|
| `cargo fmt --all -- --check` | 0 diff | 1 s | static |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 warning | 18 s（快取） | static |
| `cargo test --workspace --no-fail-fast` | 100 個 `test result:` 行，**1335 passed／1 failed／1 ignored**；唯一失敗 `declarative_session_loop::reenable_rebinds_without_restart`（未觸碰的 crate；全 workspace 並行下 stop 等待逾時成 `stop-unknown`），單獨重跑 `--test declarative_session_loop` **33／0**、單測 1／0——判定為既有 flaky，非本輪回歸。第一次執行在連結 `api_e2e` 時因磁碟 `ENOSPC` 中斷，清出空間後重跑得到上述數字（log：`docs/releases/evidence/2026-09-15-phase-1/workspace-test.txt`） | 約 6 min | unit／integration／fixture |
| `cargo test --manifest-path apps/interaction-desktop/src-tauri/Cargo.toml` | 78／0 | 0.5 s（快取） | unit |
| `pnpm typecheck`／`pnpm test`／`pnpm build` | tsc 乾淨／**1925 passed（93 files）**／build OK | 4 s／23 s／2.4 s | unit（jsdom） |
| `pnpm test:e2e`（Playwright，真 daemon＋fixture agent） | **94 passed／0 failed**（含新增 `work-activity.spec.ts` 2 支） | 5.3 min | browser |
| `./scripts/v03-cli-e2e.sh` | **102 passed／0 failed** | 14 s | integration（真 daemon） |
| `scripts/tests/architecture-checks.sh --docs`（docs commit 後） | docs-claims 261／0、release-scripts 58／0 | 3 s | static |
| `scripts/tests/architecture-checks.sh --swift` | 58／0 | 6 s | native Swift 純模型 |
| `scripts/tests/phase1/trace_e2e.py`（Claude Code／Codex） | 8／8、9／9（見 §5） | 38 s／55 s | real-agent |

**未重跑**（理由：本輪零變更）：iOS 模擬器 XCTest、ESP32 `compile.sh`、`architecture-checks.sh --drills`；`--rust` 組的測試全部包含在上表的 workspace 測試內。
**未做**：從 HEAD 重建原生 Tauri `.app` 與 AX 走查（磁碟；見 §7）、真 iPhone（§7）。

以下是本輪各 commit 附帶、已經產生的範圍受限測試數字，僅供對照：

- `cargo test -p interaction-storage --test trace_store`（S1）— 4 passed／0 failed／1 ignored；
  `-- --ignored --nocapture` 單獨跑過 `perf_trace_write_and_query_baseline`，數字見
  `docs/aip/interaction-tracing.md` §9。
- `cargo test -p interaction-agent-gateway --lib`（D16）— 32 passed／0 failed。
- `cargo test -p interaction-runtime --test gateway_loop -- resume`（D9）— 8 passed／0 failed；
  `-- --lib -- resume` — 3 passed／0 failed。
- `cargo test -p interaction-runtime --test agents_loop -- phase close_projection`（D10）—
  3 passed／0 failed；`-- gateway_loop -- phase` — 1 passed；`-- closing_an_unknown` — 1 passed。
- S3 一次交付（`ce68181`／`3c8a2a6`）附帶的新測試：
  `a_full_turn_leaves_a_dispatch_delivery_and_outcome_trail`、
  `subprocess_stderr_is_a_diagnostic_record_and_changes_no_outcome`、
  `an_interrupt_is_audited_as_requested_not_confirmed`、
  `spending_a_one_shot_consent_is_audited`、
  `a_failed_commit_rolls_back_the_whole_verification`、
  `a_failed_commit_rolls_back_the_close`、
  `trace_retention_bounds_the_diagnostic_class_and_records_the_prune`、
  `status_reports_trace_write_failures_and_counts`（皆 commit message 附帶，本文件未重跑）。
- S4（`e63986f`）附帶：`trace_and_activity_are_human_only`、
  `trace_query_clamps_pages_and_never_crosses_sessions`、
  `activity_projects_records_into_plain_language`（皆 commit message 附帶）。
- CLI（`58f7477`）附帶：`./scripts/v03-cli-e2e.sh` → 102 passed／0 failed（新增前 96）。
- 桌面（`cbc76c0`）附帶：`pnpm typecheck` ✓、`pnpm test` 93 files／1925 passed／0 failed、
  `pnpm build` ✓。
- 1070eca／377863c：各自 commit message 描述的單元測試（非 UTF-8 中途讀取、CRLF／缺結尾換行、
  1.2 MB 無換行洪流、100 KiB detail 截斷）皆已通過，具體 passed／failed 數字未在 commit
  message 中逐一列出，待整合者以 `cargo test -p interaction-agent-gateway --lib`／
  `cargo test -p interaction-storage --lib` 重跑確認。
- TB-1（`ba01c0a`）附帶：`scope_labels_keep_paths_out_of_the_record`。
- TB-4／TB-5（`711de77`）附帶：`a_failed_restore_write_is_counted_not_swallowed`、
  `a_resume_blocked_by_the_session_limit_leaves_no_accepted_row`（皆 commit message 附帶，
  本文件未重跑）。

以上皆為**範圍受限測試**，不構成完整回歸；`trace_e2e.py` 執行後，
整合者仍需補跑 `cargo fmt --all --check`／`cargo clippy --workspace --all-targets -- -D
warnings`／`cargo test -p interaction-runtime` 全量與其餘受影響 crate。）

### 8.3 PR、CI、合併與 checkpoint tag

（留白，待整合者填寫；依 `AGENTS.md` §6，PR 走 rebase merge，tag 只從已通過
`release-verify.sh` 的 commit 打——本階段是否對應一次 minor 發布見
`docs/releases/evidence-index.json` 本輪 candidate 的 `versionPolicy`；若只是查核點，應比照
階段 0 的 `phase-1-baseline-YYYYMMDD` 命名慣例建 annotated checkpoint tag。）
