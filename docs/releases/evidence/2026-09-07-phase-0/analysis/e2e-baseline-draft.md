# 階段 0：雙平台與多 Session 實際情境 E2E 基線

- **source commit**：`78dcda1a3733c97d266ca9b60ad4461c69ca2032`（= origin/main；v0.8.0 tag `1fa69b8` 之後只有 4 個 docs／test commit）
- **daemon binary**：`/Users/user/Workspace/claude-lab/adaptive-interaction/target/debug/interact-ai`，`interact-ai 0.8.0`，sha256 `1e066d84d28b9397c6c6843522ec6a0fbaf692d10ffbea59481afb79860c84c7`（2026-09-07 11:29 由 HEAD 建置；本輪未重建、未改任何 repo 檔案）
- **原生 App**：`apps/interaction-desktop` release bundle，binary sha256 `1103cda7f8491ece2adbaab478056d14a174da47ab8536a2d652c734a0765f19`（2026-09-06 13:53 建置，Info.plist `CFBundleShortVersionString=0.7.0`，provenance `appSourceRef=3b375bde` **不是** HEAD 的祖先）→ **所有 native-desktop 結果都綁定在這顆舊 App，不是 HEAD build**
- **fixture**：`target/debug/examples/fake_iphone` sha256 `64ed2d995093064ed5db070705e7830448a11ae991f8d1823d6799429709127e`（HEAD build）；`crates/interaction-runtime/tests/fixtures/fake_claude.sh`／`fake_codex.sh`
- **真 agent**：Claude Code CLI 2.1.263（claude.ai max）、Codex CLI 0.153.4（ChatGPT 登入）。**模型不可經 gateway 指定、也不可經 gateway 回報**（K-01/02/03 = absent）；所有 `actualModel` 都只從 provider 端本機 log 唯讀取得。
- **隔離**：每個 daemon 一個 `INTERACT_AI_HOME`（在各自 out 目錄下）、`INTERACT_AI_MOBILE_ADVERTISE=0`；所有回合的隔離檢查一致通過（`~/.adaptive-interaction` 的 9 個檔案 mtime／size 與 `user-home-before.txt` 逐項相同，全程未寫入 `~/.claude`、`~/.codex`，只唯讀 provider log）。

## 分類與判讀規則

只用六個值：`completed`／`correctly-blocked`／`product-failed`／`harness-failed`／`not-implemented`／`needs-environment`。
`completed` 只給「投影（API／SSE／UI）＋核心狀態（session record／status）＋實際效果（檔案／程序／provider 端紀錄）」三層齊全的案例。agent 說完成、UI 顯示成功、exit 0 都不算。`claimed-completed` 是**聲稱**，不是驗證：record 上要出現 `humanVerified` 只有在人類 `POST /verify` 之後（R4 實測），而 state 仍維持 `claimed-completed`。

**懷疑者改判（4 筆）**：
1. `E0-06 併發上限` 原判 `not-implemented` → **改判 `needs-environment`**。理由：`DelegationLimits`（`crates/interaction-core/src/agent.rs:124/133/146`）與 `agents.rs:490/496` 的強制檢查在原始碼裡確實存在，`not-implemented` 與自己引用的證據矛盾；真正的狀況是本輪沒跑超過 2 個並行 session。
2. `E0-04 SIGTERM 路徑` 原判 `needs-environment` → **改判 `harness-failed`**。理由：`restart_test.py --signal TERM` 在同一台機器、同一個 binary 上可用，沒有任何環境缺口；只是本輪沒跑（範圍／時間），寫成 needs-environment 會讓讀者以為缺裝置或憑證。
3. `E0-05 supersede` 結論維持 `completed`，但**缺陷敘述要收窄**：`crates/interaction-core/src/knowledge.rs` 與 `crates/interaction-runtime/src/knowledge.rs:1182-1225` 內有完整的 candidate→active→stale→disputed→superseded 狀態機，只是**沒有接到 user-memory／preference 這一層**。不能寫成「系統沒有任何衝突解析機制」。
4. `E0-05 已終止 session 仍可收新任務` 原判 medium product defect → **改判 by-design 的觀測性缺口（low）**：`crates/interaction-core/src/agent.rs:48-55` 與其回歸測試 `claimed_completed_is_open_not_terminal` 明文寫「agent 的聲稱讓 session 保持 open，驗證與關閉是人類／runtime 的獨立步驟」。同時**刪除**原文那組 `spentMessages=4 / spentCost=0.31141175` 的數字——沒有任何被擷取的 HTTP 回應含這個值，唯一擷取到的 GET 是 `spentMessages=3 / spentCost=0.135597`。

---

## E0-01 首次設定與偏好持久化

**前置條件**：全新隔離 home（API 路徑 port 19050；native 路徑每案 `mktemp` 一個 home 並自選 ephemeral port），onboarding 未完成、UI 偏好為預設。
**使用者操作**：（API）GET onboarding／status → POST `/v1/onboarding/preview` → `/v1/onboarding/commit` → PATCH `/v1/ui/preferences` → SIGTERM daemon → 同 home 重啟 → 重讀。（native）在 App 內完成首次設定 → 切換「安靜」陪伴預設 → 在 loopback proxy 注入 10 種故障（連線被拒／回應遺失／回讀失敗／寫入前後崩潰／清理失敗／更新的選擇／無關偏好／未知格式標記／重連）→ SIGKILL App → 重啟。
**預期畫面**：重啟後不再回到首次設定精靈；偏好頁顯示 `locale=ja-JP`／`mode=advanced`；native 的「安靜」勾選在 9/10 故障案例後仍為勾選，`newer-choice` 以使用者較新的選擇為準，`future-marker` 顯示「恢復標記」而不是靜默丟棄。
**核心狀態**：`onboarding.completed=true` 且 `completedAt` 有值；`ui_prefs` meta 落地；native 的 `state/desktop.json.companionPendingPresetOp` 在恢復後清空（`future-marker` 例外，保留 `{"format":99,"unknownIntent":"keep-me"}`）。
**可觀察效果**：`GET /v1/status.onboardingCompleted=true`；重啟前後 export 逐欄位相同；native 恢復不會多送一次條件寫入（以 proxy 的 `conditionalWrites` 計數斷言）。
**失敗與恢復行為**：SIGTERM 不走 graceful shutdown（`crates/interaction-cli/src/commands.rs:1608` 只等 `ctrl_c()`），重啟時印出 `reclaiming stale instance lock pid=…` WARN；本輪未觀察到任何資料遺失。
**實際結果**：

| 分欄 | 結果 |
|---|---|
| Claude Code / Codex | n/a（本案不涉及 agent session） |
| API／CLI | **completed**（三層齊全） |
| fixture | n/a |
| native-desktop | **completed**：10/10 故障案例收斂，113.66 s；但 App 是 2026-09-06 的舊 bundle，此結果不代表 HEAD build |
| real-iphone | n/a |

**證據**：`runs/R5/E0-01-step1.log`、`step2.log`、`step3.log`、`runs/R5/homeA/daemon-run2.log:1`、`runs/R5/E0-11-restart-check.log`；`runs/R7/preset/result.json`（`$.results[*].status` 全 `completed`、`$.results[*].effectiveConfig`、`$.results[*].prefs`）、`runs/R7/preset.cmd.txt`（EXIT=0）、`runs/R7/preset/refused.log:1`。
**清理**：R5 兩個 daemon 已 SIGTERM，19050/19051 已釋放；R7 driver 自行刪除 10 個隔離 home（清單見 `runs/R7/preset-homes.txt`）。

---

## E0-02 派工給真 agent（含同 session 多輪與原生續接）

**前置條件**：隔離 daemon＋隔離 workdir（內含唯一暗號或 NOTES.md），`allowWrite=false`、`toolScope=[]`、`consentScope=[]`。
**使用者操作**：`POST /v1/agent-sessions` → `POST …/messages {kind:"task"}` → SSE／輪詢到終態 → 讀 `messages?direction=from-session` → 對照 ground truth；延伸：同一 session 再送第二個任務（K-04）、關閉後用 `resumeProviderSessionId` 續開。
**預期**：SSE 走 `fetched → working → claimed-completed`；record 的 `providerSessionId` 有值、`budget.spentMessages` 增加；回答內容與 workdir 內的真實資料一致；`humanVerified` 在未 verify 前不存在。
**核心狀態／效果**：provider 端本機 log（`~/.claude/projects/<slug>/<psid>.jsonl`、`~/.codex/sessions/…/rollout-*.jsonl`）必須出現對應的 turn；續接必須續寫**同一個檔案**。
**失敗與恢復行為**：codex 在 `approval_policy=untrusted`＋`sandbox=read-only` 下，連 `cat NOTES.md` 都會停在 `waiting-for-consent`；沒有人核可就永遠不會執行（`spentCost=0.0`），這是**正確阻擋**。
**實際結果**：

| 分欄 | 結果 |
|---|---|
| Claude Code | **completed**：答案 `「階段零煙霧測試」` 與 NOTES.md 第一行逐字相符，`claimed-completed`，`spentCost=0.14471075`；多輪（K-04）第二輪答對隨機暗號 `X55XY4F8`，**子程序 pid 33355 跨兩輪不變**（不重新 spawn）；續接走真 `--resume <psid>`（argv 直接證據），provider log 續寫同一檔，且續接輪 prompt 內**不含**暗號（排除洩題） |
| Codex | **completed（有核可時）**／**correctly-blocked（無核可時）**：`--approve approve` 走完 `waiting-for-consent → claimed-completed`，答案逐字正確；不核可則停在 `waiting-for-consent`、`spentCost=0.0`。多輪／續接同樣成立：pid 35908 跨兩輪不變、thread/resume 續寫同一個 rollout、`turn_context` 仍是 `read-only`／`untrusted`（授權是**重新上鎖**不是繼承） |
| fixture | n/a（native work driver 從未真的啟動 fixture agent） |
| native-desktop | **harness-failed**：composer 打字被截斷（見 E0-03），從未送出任何工作 |
| real-iphone | n/a |

**未觀察到**：300 s approval TTL 自動拒絕（`gateway.rs:29 APPROVAL_TTL_SECS=300`、`gateway.rs:1034 gateway_sweep`、`runtime.rs:2419` 每 tick 呼叫）——上一輪的 codex-smoke-1 在 deadline 前約 15 s 就被 harness 停掉，**只有靜態證據，沒有執行期證據**。
**證據**：`e2e/claude-smoke-1/result.json`＋`sse.jsonl`（112 行）、`e2e/codex-smoke-1/result.json`（`final.state=waiting-for-consent`、`wallSeconds=303.34`）、`e2e/codex-smoke-2/result.json`（3 則 mailbox：approval-request → approval-resolved 220 ms → result）、`runs/R8/claude/{run.log,raw.json,sse.jsonl}`、`runs/R8/argv-claude/run.log:5`（`--resume e63b51f8-…`）、`runs/R8/codex/{run.log,raw.json}`、`~/.claude/projects/…/e63b51f8-….jsonl`、`~/.codex/sessions/2026/09/07/rollout-…-01a07b22-….jsonl`。
**清理**：所有 session 已 close，daemon 皆自行 SIGTERM，19080–19086 已釋放，無殘留子程序。

---

## E0-03 取消／中斷／緊急停止

**前置條件**：隔離 daemon＋長任務（1500 字繁中短文，明示不碰檔案），等 record 進入 `active` 後再取消（R1 做到了 `--cancel-when-active`；上一輪三次 claude 取消都落在 `created/fetched`）。
**使用者操作**：`POST …/interrupt`（或 `POST …/close`）→ 輪詢終態 → 檢查子程序 → 檢查有無遲到的 from-session 訊息；另外 `POST /v1/emergency-stop` 與重啟後的 latch 驗證。
**預期**：人類取消 ⇒ `cancelled`（badge「已取消」）；子程序整組消失；沒有遲到結果；close 後仍保留終局狀態。
**核心狀態／效果**：`record.state`、SSE `agent.session.state`、角色 `character.system-text` 三者一致；`ps` 確認子程序死亡。
**失敗與恢復行為**：estop 後重啟不得自動恢復；AI 不得解除 estop。
**實際結果**：

| 分欄 | 結果 |
|---|---|
| Claude Code | **product-failed**：`interrupt` 一律變成 `failed`（誠實階梯：取消 ≠ 失敗），**5/5 重現**（claude-cancel-1、multi-C i=1、multi-E i=0、R1/claude-interrupt〔active 時取消〕、R1/b07-claude）。`grep -c TaskCancelled crates/interaction-agent-gateway/src/claude.rs == 0`——Claude 連接器在任何路徑都產不出 `cancelled`。機制本身有效（子程序死亡、無遲到訊息），錯的是分類與投影。`close`（從 active）→ `closed`，**completed** |
| Codex | **completed**：`interrupt` → `cancelled`（2/2 單 session＋多 session 各一次），provider rollout 獨立佐證 `turn_aborted reason="interrupted"`；`close` → `closed`，**completed** |
| 緊急停止（A-07，Claude） | **completed**：live session → `cancelled`＋`detail="cancelled (was Active)"`，子程序 1 s 內消失，`/v1/status.emergencyStop=true`，新 session 403 `policy_blocked`，受限 agent token 解除 403 `token_scope_forbidden`（但仍讀得到 `emergencyStop=true`），SIGTERM 重啟後仍鎖住、人類清除後恢復正常 |
| fixture | n/a |
| native-desktop | **harness-failed**：兩次執行都在第一步 composer 輸入就失敗（AX 讀回是 `Native fixture codex can` 與 `Native fixture code`，兩次截斷點不同、osascript 全部 exit 0），沒有任何 session 被建立 |
| real-iphone | n/a |

**額外發現**：`interrupt` 在**兩個 agent 上都是 session 級取消**，不是「停止這一輪」——之後對同一 session 送任務一律 409 `mailbox closed`（`agents.rs:900-904`），子程序已死也無法就地續跑 → **not-implemented（沒有 stop-generating 能力）**。Codex 的 provider 端其實只中止了 turn（`turn_aborted`），是 runtime 選擇連程序一起殺掉。
**證據**：`runs/R1/claude-interrupt/{result.json,sse.jsonl:109,116}`、`runs/R1/codex-interrupt/{result.json,sse.jsonl:103,109}`、`runs/R1/b07-{claude,codex}/result.json`（`task2Status=409`）、`runs/R1/a07-claude/{result.json,sse-d1.jsonl,sse-d2.jsonl}`、`runs/R1/close-children-check.txt`、`e2e/claude-cancel-1/home/state/interaction.db`（`inferences.report.error='error_during_execution'`）、`~/.claude/projects/…/5779447a-….jsonl:9`（`[Request interrupted by user]`）、`runs/R7/work/failed-ax.txt:40`、`runs/R7/work-diag1/failed-ax.txt:40`。
**清理**：R1 spawn 的 daemon（91044/91045/5661/5662/21898/21899/41215/51232）全部確認死亡、埠釋放、無孤兒子程序。

---

## E0-04 核心崩潰與重啟恢復

**前置條件**：隔離 daemon＋進行中的 agent session；`restart_test.py --signal KILL`。
**使用者操作**：SIGKILL daemon → 觀察子程序孤兒化 → 同 home 重啟 → GET session／list／messages、POST send／interrupt／renew。
**預期／核心狀態**：重啟時所有 `is_open()` 的 session 一律標成 `expired`（`agents.rs:1479-1520`），`detail` 誠實說明是否回收了孤兒 pgid；mailbox 409、gateway handle 404、renew 409。
**可觀察效果**：孤兒 `claude -p` 子程序在重啟時被 `reap_recorded_gateway_pgids`（`agents.rs:1615-1671`）依記錄的 pgid 清掉。
**實際結果**：

| 分欄 | 結果 |
|---|---|
| Claude Code | **completed**：`state=expired`、`detail="runtime restarted; orphan subprocess group reaped (pgid 15721)"`；SIGKILL 後子程序 ppid 變 1（真的孤兒），重啟後 `children-final=[]`；send 409／interrupt 404／renew 409 |
| Codex | **completed**：codex 隨 daemon 死亡（stdin 關閉），`+2 s`／`+5 s` 都查不到子程序；rollout 於 daemon 被殺後 5 ms 記下 `turn_aborted reason=interrupted`；`detail="runtime restarted"`（無 pgid，因為沒有孤兒可歸因）——**detail 文字的差異本身就是誠實訊號** |
| SIGTERM 路徑 | **harness-failed**（原判 `needs-environment` → 改判：`restart_test.py --signal TERM` 在同機同 binary 可用，只是本輪沒跑，不是環境缺口） |
| fixture / native-desktop / real-iphone | n/a |

**已知殘餘風險（非本輪新發現）**：孤兒回收是 best-effort，只在**下一次重啟**才做；若 daemon 從此不再啟動，孤兒 claude 會不受管理地跑到自然結束（`agents.rs:1618` 註解＋`CHANGELOG.md:1846/1859` v0.4 已記錄）。但 `docs/releases/v0.8.0-known-limitations.md` 沒有重申這條——文件延續性缺口。
**未解釋現象**：`restart-claude-kill` 的 session 送出任務後 21 s 仍是 `created`（同批 claude-smoke-1 只要約 5 s）；provider log 完全沒有 assistant 紀錄，daemon.log 沒有 debug 輸出。發生時間接近台北中午 Claude 額度重置窗口，是**合理但未證實**的假說，誠實記為 unknown，不歸咎程式碼。
**證據**：`e2e/restart-claude-kill/{result.json,daemon.log}`、`e2e/restart-codex-kill/result.json`、`~/.codex/sessions/2026/09/07/rollout-…-01a07a04-….jsonl`、`crates/interaction-runtime/src/agents.rs:1479-1520`／`:1615-1671`、`sensor_journal.rs:105`。

---

## E0-05 記憶、Context Bundle 與刪除的不可收回

**前置條件**：隔離 daemon（19020），記憶庫為空。
**使用者操作**：建記憶（明示 `agentVisibility` vs 不設）→ 產 Context Bundle（HTTP 與 CLI 各一份）→ 真 agent 讀 bundle → 加一筆矛盾記憶 → 刪除 → 新 session 再問 → 對舊 provider session 續接再問 → daemon 重啟 → 受限 agent token 直寫。
**預期／核心狀態**：預設可見性（`interaction-core/src/memory.rs:166-180`）讓 UserMemory 未明示 `agentVisibility` 時**對所有 agent 不可見**；bundle 排序是確定性的（layer_rank → updated_at desc → id）；刪除只影響「之後產生的 bundle」。
**實際結果**：

| 分欄 | 結果 |
|---|---|
| Claude Code | **completed**：`to-session` 的 `body.contextBundle.includes` 含目標記憶，from-session summary 逐字含「靛青色」與暗號；刪除後 `includes=[]` 且 agent 誠實說「不知道」；**E-09 成立**——用 `resumeProviderSessionId` 續開後，agent 完整覆誦已刪除的機密暗號（`剛才談到的顏色是靛青色，暗號是 phase0-mem-32b415。`），證明刪除**無法**回收已進入 provider transcript 的內容 |
| Codex | **not-run**：codex 的 E-09 等價行為（resume 後已刪記憶是否殘留）本輪完全未測 |
| API／CLI | **completed**：HTTP 與 CLI 的 bundle 除 `generatedAt` 外逐欄位相同；重啟後 6 筆記憶逐欄位不變；受限 agent token 對 `/v1/memory` 的 GET/POST 一律 403 `token_scope_forbidden`（**連 handler 都進不去**），只有「human token ＋ `asAgent`」才會走到降權邏輯（`fact → inference`） |
| fixture / native-desktop / real-iphone | n/a |

**收窄後的缺陷**：user-memory／preference 層**沒有**接上 supersede／conflict 機制——兩筆同標題、內容矛盾的偏好會同時進入每一次的 bundle，新舊判斷完全外包給下游 LLM 的文字推論。但 `crates/interaction-core/src/knowledge.rs` 的知識層**有**完整的 superseded 狀態機（`knowledge.rs:1182-1225`），所以這是「分層未接通」而不是「系統沒有這個能力」。
**降級為 by-design 的項目**：已 `claimed-completed` 的 session 仍可收新任務並真的執行（`agent.rs:48-55` 明文設計＋回歸測試），但 `GET /v1/agent-sessions/{id}` 的 `state` 全程不反映這次執行，只能靠輪詢 mailbox 才看得出來 → 觀測性缺口（low）。原文引用的 `spentCost=0.31141175` 沒有任何擷取到的回應支持，已刪除。
**證據**：`runs/R2/step1-*.json`、`step2-bundle-{http.json,cli.txt}`、`E0-05-delivery-claude/result.json`、`step4-bundle-after-mem2.json`、`step5-{delete-mem1,delete-mem2,bundle-after-delete,old-session-resend,old-session-final,resume-create,resume-final}.json`、`step6-memory-{before,after}-restart.json`、`step7-*.json`、`~/.claude/projects/…/1bf60429-….jsonl:20`。

---

## E0-06 同型多 Session 併發

**前置條件**：一個 daemon 下兩個同型 session，各自隔離 workdir、各自唯一暗號。
**使用者操作**：同時派工 → 取消其中一個 → 觀察另一個是否受影響 → 檢查記錄保留與 SSE 標記。
**預期／效果**：結果不得交叉洩漏；SSE 的 session 範圍事件必須帶正確 `sessionId`；取消一個不得干擾另一個；兩筆記錄都要留在 list（不得靜默丟工作）。
**實際結果**：

| 分欄 | 結果 |
|---|---|
| Claude Code ×2 | **completed**：i=0 summary 只含自己的 `TOKEN-0-a1fe3a`，SSE 134 行中 39 行 session 範圍事件、**0 筆混淆**；i=1 被取消後仍以 `failed`（見 E0-03 缺陷）留在 list |
| Codex ×2 | **completed**：核可只作用在 i=0（`approved=['0']`，i=1 為空），SSE 151 行、0 筆混淆；i=1 取消 → `cancelled` |
| 併發／保留上限 | **needs-environment**（原判 `not-implemented` → 改判：`max_sessions=8`／`max_parallel=4` 在 `interaction-core/src/agent.rs:124-146` 與 `interaction-runtime/src/agents.rs:490/496` 確實實作，只是本輪最多只跑 2 個並行 session，沒有執行期證據） |
| fixture / native-desktop / real-iphone | n/a |

**證據**：`e2e/multi-C-claude-x2/{result.json,sse.jsonl,work-0/NOTES.md,work-1/NOTES.md}`、`e2e/multi-D-codex-x2/{result.json,sse.jsonl}`、各自 `home/state/interaction.db`。
**觀察**：`status.agentSessions=1` 而 `list` 有 2 筆——那是「開啟中」計數器，不是資料遺失訊號。

---

## E0-07 跨型（Claude × Codex）並行

**前置條件／操作**：同 E0-06，但一個 claude-code、一個 codex，取消 claude 那一個。
**實際結果**：**completed**。codex 側完全不受 claude 取消影響，照常走完自己的核可循環並回報 `TOKEN-1-cd1568`；146 行 SSE 中 50 行 session 範圍事件、0 筆混淆；`providerSessionId` 格式各自正確（claude UUIDv4 / codex `01a07a0x-` thread id）；兩筆記錄都保留（`failed` / `claimed-completed`）。
**這一案同時鎖定了 D1 的性質**：claude interrupt→failed、codex interrupt→cancelled 在同型×2、同型×2、跨型×1 共 3/3 重現，是**連接器屬性**，不是配對造成的假象。
**證據**：`e2e/multi-E-claude-codex/{result.json,sse.jsonl}`、`~/.claude/projects/…/276c9883-….jsonl`、`~/.codex/sessions/…/rollout-…-01a07a03-….jsonl`。

---

## E0-08 iPhone（配對、雙向、停止未知、AIP、撤銷、核心離線）

**前置條件**：隔離 daemon（19060）＋`fake_iphone`（**模擬器，不是真機**）；真機路徑另做唯讀環境檢查。
**使用者操作**：`interact-ai mobile pair` → fixture 完成 TOFU pin＋HMAC 配對 → 回報 micLevel／權限 → 啟用受器 → `mobile stop-sensors`（不 ack）→ ack／斷線 → AIP capability／touch → 斷線重連 resume → `mobile revoke` → SIGKILL daemon 後重啟。
**預期／核心狀態**：感測不靜默；停止未回覆一律 `unknown`；撤銷立即斷線且舊 token 不可再認證；重啟後舊 token 仍可用（狀態持久化）。
**實際結果**：

| 分欄 | 結果 |
|---|---|
| fixture（模擬 iPhone） | **completed**（六個 journey）：配對＋連線；裝置回報的 micLevel 立刻出現在 `mobile status`，但**要到受器被明確 `PATCH /v1/receptors/iphone.mic-level {enabled:true}` 之後才進 `/v1/status.activeSensors`**（bulk onboarding 對 consent-gated 元件**永遠**拒絕，`human.rs:729-742`，是 by-design 的預設關閉不變量）；`stop-sensors` 無 ack → `outcome:unknown`＋`activeSensors[0].state=stop-unknown`（不靜默消失），ack 後才清空；裝置在未 ack 時斷線 → `/v1/sensors/unresolved` 立刻出現 `confirmedStopped:false`，人類 dismiss 也明寫「no source stop confirmation」；AIP capability→touch→Behavior Intent→`status:applied`，revision 1→2→3；斷線期間 presence=`reconnecting`（不是 offline），resume 只回補缺的兩個 patch；revoke 1.5 s 內斷線、裝置從清單消失、舊 token 再連被拒 |
| 核心離線（F-06） | **completed（daemon 側）／harness-failed（fixture 側）**：daemon 側用一支自寫的最小 WS client 驗到「daemon 死時連線被拒、重啟後同一 token `auth-ok`」；但**出貨的 fixture 本身**在第一次連線被拒時就 `process::exit(2)`（`crates/interaction-runtime/examples/fake_iphone.rs:177` 的 `die()`，由 `:298` 的 `reconnect()` 呼叫），無法照題目描述的「同一 process 重連」走完 |
| native-desktop | **harness-failed**：兩次跑分別只完成 4/10 與 6/10 journey，之後 AX `dump` 超過 driver 的 55 s 上限（dump 耗時 12.8→46.5 s 單調上升）；**四個 sensor journey（停止結果未知的誠實性）從未跑到** |
| Claude Code / Codex | n/a |
| real-iphone | **needs-environment**：裝置有（iPhone 11「Alex」available (paired)、Developer Mode enabled），但 `device-build.sh --check-only` 在 step 3/5 卡住——Xcode 的 `IDEProvisioningTeams` 是空的（keychain 內雖有一張 `Apple Development` 憑證，但那不是 `xcodebuild -allowProvisioningUpdates` 讀的那個 store）。整輪**沒有任何真機 E2E 證據** |

**證據**：`runs/R6/logs/{pair-2.json,phone.log:1-27,phone2.log,phone2.stderr.log,phone3.log:4-6,daemon-responses.jsonl,daemon-restart.log,probe-while-dead.json,probe-after-restart.json,ws_auth_probe.py}`、`runs/R7/mobile/result.json`、`runs/R7/mobile-diag1/result.json`、`crates/interaction-runtime/src/mobile.rs:68/3080-3120`、`sensor_source.rs:36-44`、`apps/interaction-ios/scripts/device-build.sh:158-190`。

---

## E0-09 非同步問答（waiting-for-input）

**前置條件**：隔離 daemon（19030/19031），真 agent 各一。
**使用者操作**：請 agent 先問澄清問題 → 人類 30 s 後回答 → 同一 session 送第二個任務；另外人工 `POST …/report {event:"waiting-for-input"}`。
**實際結果**：

| 分欄 | 結果 |
|---|---|
| 自動偵測 | **not-implemented**：`GatewayEvent::TaskWaitingForInput` 只在 `crates/interaction-agent-gateway/src/lib.rs:99` 定義（註解自己寫明沒有 connector 會產生它），唯一消費點 `gateway.rs:413`；UI 有完整的 `NEEDS_INPUT` 投影（`workState.ts:91-101`）卻永遠不會被觸發 |
| Claude Code | **completed（多輪續談）**：澄清問句被包成 `claimed-completed`，第二個任務被同一 session 收下並延續脈絡；`claimId` 才是判斷「新一輪」的可靠欄位（只看 `state` 會誤判——首次腳本因此踩到 race bug，已保留診斷資料） |
| Codex | **completed（多輪續談）**：同型行為，且拍到 `staleClaim → active → 新 claimId` 的中間態 |
| 人工回報路徑 | **completed**：`POST /report` 無條件把任何 open session 標成 `waiting-for-input`（不驗證「是否真的有人在等」——這是人工補位設計），送真任務後會被連接器事件正常覆蓋 |
| fixture / native-desktop / real-iphone | n/a（UI 投影只讀 `workState.ts` 常數表，未跑 Tauri／Playwright） |

**靜態發現**：codex 0.153.4 的 v2 schema 確實有等價通知 `item/tool/requestUserInput`（`ToolRequestUserInputParams/Response`，EXPERIMENTAL），但 `codex.rs:250-263` 把**所有**帶 id+method 的 ServerRequest 一律當核可請求，`approval_summary`（`codex.rs:432-445`）只找 command/cmd/path/reason、**問題文字被丟掉**，`resolve_approval`（`codex.rs:623-660`）一律回 `{result:{decision}}`，與 `ToolRequestUserInputResponse` 要求的 `{answers:{…}}` 不符。本輪兩次真 codex turn 都沒自然觸發到，所以後果是**靜態推論**，不是觀察。
**證據**：`runs/R3/{claude-followup,codex-followup,manual-report}/result.json`、`runs/R3/claude-followup-attempt1-race-bug/result.json`、`runs/R3/codex-followup/daemon.log`、`crates/interaction-agent-gateway/src/{lib.rs:99,codex.rs:249-266/431-445/623-660}`、`crates/interaction-runtime/src/gateway.rs:412-417`。

---

## E0-10 權限：拒絕、唯讀、授權寫入、驗證、撤銷、過期、續開不放寬

**前置條件**：每個子案一個隔離 daemon（19040–19045）＋隔離 workdir。
**使用者操作**：唯讀 session 收到寫檔任務／人類 deny 核可請求／授權寫入後 verify／close 撤銷／TTL 1 分鐘到期／用受限 agent token 打人類層端點／各種「比上次更寬」的續開。
**實際結果**：

| 分欄 | 結果 |
|---|---|
| Claude Code（唯讀） | **correctly-blocked**：provider 端 `permissionMode=plan`，整輪只用 Glob／Read，**連 Write 工具都沒被提供**；agent 誠實說「沒有 Write 工具」，workdir 沒有 hello.txt |
| Claude Code（授權寫入） | **completed**：`permissionMode` 變 `acceptEdits`，`hello.txt` 內容 `hi\n`（od -c 驗過），record `allowWrite=true`／`toolScope=['workspace.write']`／**無 `humanVerified` 欄位**；`POST /verify` 後 `humanVerified={at,claimId,note}` 且綁定當次 `claimId`，重複 verify 409 |
| Codex（deny） | **completed（阻擋有效）＋ product-failed（wire 值錯）**：deny 之後指令確實沒執行、agent 誠實回報、狀態沒卡住；但 gateway 送的是 `"reject"`（`codex.rs:644`），而 codex 0.153.4 的 `CommandExecutionApprovalDecision` 只接受 accept／acceptForSession／acceptWithExecpolicyAmendment／applyNetworkPolicyAmendment／decline／cancel。實測 codex 端把它當成**核可管線錯誤**（`Rejected("approval request failed")`）而 fail-closed——安全結果對，但是靠 provider 的失敗處理，不是靠我們送對值；而且人類的「否決」被原文轉述成像系統故障 |
| Codex（授權寫入） | **completed**：`accept` 是合法值，`hello.txt` 落地（`hi`，2 bytes）；但 provider 自報的有效 sandbox 是 `workspace-write` **＋使用者全域 `~/.codex/config.toml` 的三個 writable_roots**，超出 session 授權的 resolvedWorkdir |
| 撤銷／過期 | **completed**：close 後 `state=closed`、`consentScope` 清空、`humanVerified` 保留；messages/renew 409、approve/interrupt 404；TTL 到期（懶惰式，讀取時翻轉）→ `expired`，之後 409/404/409；**重啟後所有 open session 一律 expired，授權不因重啟復活** |
| 續開不得放寬 | **correctly-blocked**：R4 六種＋R8 十七種（兩 agent 各一輪）形狀全部 4xx，訊息逐項指名（可用工具／使用授權／資料範圍／時間上限／訊息上限／工作目錄）；`../` 繞路被 canonicalize 擋下；**省略欄位＝落到 runtime 預設＝放寬，一樣被擋**；縮小（ttl 5／maxMessages 10／結尾斜線／claude 加 maxCost）正確放行；假造／空字串／全空白的 `resumeProviderSessionId` 一律 403 |
| AI 不可授權 | **correctly-blocked**：受限 agent token 對 approve／create／PATCH policy／estop clear／verify／renew／GET session／onboarding commit 全部 403 `token_scope_forbidden`；只有「安全遞減」的 `POST /v1/emergency-stop` 允許（by design） |
| fixture / native-desktop / real-iphone | n/a |

**副作用（誠實記錄）**：R4 的 agent-token estop 那一步把當時 `claimed-completed` 的續開 session 轉成 `cancelled`，導致之後 verify 回 409——**是測試操作造成的**，不是重啟造成的；但它也暴露一個設計問題：AI 按下 estop 就能讓一個等待人類驗證的聲稱永遠無法被驗證。
**證據**：`runs/R4/{a-codex-deny,b-claude,b-codex,c-claude-write,c-codex-write,d-revoke,d-resume,d-resume2,e-ttl,f-agent-token}/*`、`runs/R8/{guard-claude,guard-codex}/run.log`、`~/.codex/config.toml:418-419`、`crates/interaction-runtime/src/agents.rs:105-237/453-468/1120-1150`、`crates/interaction-api/src/lib.rs:454-505`。

---

## E0-11 備份匯出、還原與重啟保留

**前置條件**：home A 已完成 onboarding／偏好／4 筆跨 layer 記憶；home B 全新。
**使用者操作**：`GET /v1/memory/export`（HTTP 與 CLI 各一）→ SIGTERM＋重啟 → 逐欄位比對 → 在全新 home 逐條還原（欄位集合與 `BackupSection.tsx:72-83` 相同）→ 再重啟 → 匯入一份中間有壞資料（缺 title）的備份；native：真 WebView 下載 ＋ 真 NSOpenPanel。
**實際結果**：

| 分欄 | 結果 |
|---|---|
| API／CLI | **completed**：export `count=4/total=4/limitReached=false`，`included=["memory-items"]`、`notIncluded=["knowledge-nodes","assets-and-derivatives","knowledge-receipts","character-interaction-memory"]`（後端硬編與前端 `SCOPE_LABEL` 語意一致——這是**明示的功能邊界**：換機重灌無法完整復原，只能復原記憶分層）；重啟後 6 筆逐欄位不變；全新 home 還原保留 layer/kind/title/content/tags/confidence/agentVisibility/agentDenylist/retention，重新賦予 memoryId/createdAt/updatedAt，且 `createdBy` 一律變成 human（`restoreBackup()` 不送 `asAgent`）——刻意「不信任備份中的身分」，但代價是 agent 出處遺失 |
| 壞資料 | **correctly-blocked**：第 3 筆（缺 title）400 `validation_failed: title 必須為 1..120 字`，第 4 筆完全沒被送出（for-loop 在第一個錯誤 throw），已寫入的 2 筆保留，總數 4→6 |
| native-desktop | **harness-failed**：匯出這一半**真的驗到了**（~/Downloads 出現 404 bytes、sha256 `f4a3171…` 的檔案並在收尾刪除）；但**匯入與四個 correctly-blocked 案例全部未驗證**——AX 的 `choosefile`（`scripts/lib/tauri-ax.applescript:51-63`）從不驗證是否真的導航，Open panel 停在 2026-09-06 的記憶目錄並開了字母序第一個檔。probe5 用 `kind:"not-a-companion-settings"` 的檔案（解析器必須拒絕）仍看到「已匯入角色設定並套用。」→ 證明 App 拿到的不是我們指定的檔。**這同時代表 2026-09-06 那次綠燈可能是假陽性** |
| Claude Code / Codex / fixture / real-iphone | n/a |

**證據**：`runs/R5/{E0-11-create.log,E0-11-export-api.json,E0-11-export-cli.json,E0-11-export-api-vs-cli-diff.log,E0-11-export-after-restart.json,restore.py,E0-11-restore-fresh-field-diff.log,E0-11-bad-backup.json,E0-11-restore-bad-item.log}`、`runs/R7/settings/{result.json,failed-ax.txt}`、`runs/R7/probe{1,4,5}/*`、`~/Library/Preferences/dev.adaptive.interaction.desktop.plist`（`NSOSPLastRootDirectory`）。

---

## 本階段「沒有驗到」的清單（不得寫成已驗收）

1. 真 iPhone 的任何 E2E（簽章卡住）；ESP32 真板依舊為零。
2. 原生桌面的：工作派送與取消（E0-03）、設定匯入與四個阻擋案例（E0-11）、四個 sensor journey（E0-08）。
3. codex 的 300 s approval TTL 自動拒絕在執行期的行為。
4. codex 的 `item/tool/requestUserInput` 真實協定往返。
5. `restart_test.py --signal TERM`（乾淨關閉）路徑。
6. 併發上限 `max_sessions=8` / `max_parallel=4` 的執行期驗證。
7. codex 的 E-09（resume 後已刪記憶是否殘留）。
8. codex `writable_roots` 是否真的可寫入 workdir 之外（只有 provider 自報的策略，沒有實際寫入證據——刻意不做）。
9. 原生 App 從 HEAD 重建（磁碟不足），所有 native 結果綁在 2026-09-06 的 0.7.0 bundle。
