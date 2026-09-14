# 階段 0 之後：分階段計畫、驗收門檻與下一個實作切片

> 這份文件是階段 0 的裁決結果：依 [能力恢復矩陣](phase-0-capability-recovery-matrix.md)、
> [E2E 基線](phase-0-e2e-baseline.md)、[已知問題可重現性](phase-0-known-issues-reproducibility.md) 與
> [測試責任矩陣](phase-0-test-responsibility.md) 的實查結果排定後續階段。每個階段列出使用者可見成果、
> 前置依賴、owner／契約變更、不在範圍、E2E 驗收、安全與資料門檻、真 Agent／真機需求、發布條件與 blockers。
> 已完成的能力不重做；階段 0 已驗證的多 Session 基礎（同種×2、異種×1、隔離、單獨取消）由階段 5 直接引用。
> 缺陷編號 D1–D20 見 E2E 基線；能力列編號（A-xx…K-xx）見矩陣。

## 0. 階段 0 交付出去的起點（不重做）

| 已驗證（階段 0） | 證據 |
|---|---|
| 電腦單獨跑通：安裝／`interact-ai serve`／首次設定／偏好保留／重啟／備份匯出還原（API 路徑） | E0-01、E0-04、E0-11 completed |
| 真 Claude Code 與真 Codex：派工→接收→claimed→人類 verify；唯讀擋寫、授權寫入落地、撤銷、過期、續開不放寬、agent token 全部確定性阻擋 | E0-02、E0-10、K-04 completed／correctly-blocked |
| 多 Session：Claude×2、Codex×2、Claude＋Codex 不串線、單一取消不影響另一個 | E0-06、E0-07 completed |
| Context Bundle 建立→送達→被讀→刪除後新任務不再含 | E0-05 completed |
| 模擬 iPhone（fixture）配對／雙向／停止未知／AIP／斷線重連／撤銷 | E0-08 fixture completed；真機 needs-environment |

## 1. 階段 1：電腦完整文字工作流程（Claude Code 與 Codex 各自跑通）

**使用者可見成果**：在桌面（原生 Tauri，非 fixture）對真 Claude Code 與真 Codex 各派一件低風險任務，看得到「工作中→對方說已完成→我確認」；按「取消」看到「已取消」而不是「失敗」或「結果不確定」；拒絕 Codex 的核可請求後 Codex 得到的是語意上的拒絕；工作卡片顯示這次是哪個模型、花了多久、多少費用／token；失敗時看得到原因。

**前置依賴**：階段 0 基線（本文件所在 checkpoint）；磁碟清出空間以從 HEAD 重建原生 App（目前 native 證據綁在 2026-09-06 的 0.7.0 候選 bundle）。

**owner／契約變更**：
- `crates/interaction-agent-gateway/src/claude.rs`：interrupt 語意（D1）；record `detail` 寫入失敗原因（D3，`crates/interaction-runtime/src/agents.rs:1209-1218`）；close 的 SSE 投影保留終局狀態（D2，`agents.rs:1371-1379`）。契約：`docs/character-protocol/README.md` 與 `workState.ts` 的 cancelled／failed 文案不變，只修 runtime 產生的值。
- `crates/interaction-agent-gateway/src/codex.rs:643-644`：deny 的 wire 值改為 0.153.4 列舉內的 `decline`（D4）；stderr 導進 daemon log（D16）；`SessionSpec.model`／effort 從 create payload 接線並記錄實際模型（K-01／K-02／K-03：新增 `CreateAgentSession.model`、record `requestedModel`／`actualModel`——契約變更需同步 `schemas/`、`apps/interaction-desktop/src/api.ts`、CLI `agents create --model`）。
- Codex 連接器的 MCP／plugin 邊界（D5）與 `writable_roots`（D6）：先做產品決策「平台 session 授權 vs 使用者 `~/.codex/config.toml` 誰優先」，決策寫進 `docs/ARCHITECTURE.md` 的信任邊界一節；決策前先把現況寫進 known limitations（本 checkpoint 已寫）。
- 續開稽核（D9）：resume guard 接受／拒絕都寫 `store.audit`。
- Context Bundle 派送上限矛盾（D-06／E-04：`BUNDLE_MAX_BYTES` 48 KiB 對 `MAX_BODY_BYTES` 16 KiB）：這是「電腦完整文字工作流程」在記憶稍多時就會派送失敗的缺陷，放在階段 1 而不是階段 4。

**不在範圍**：串流文字／進度投影（階段 2）、waiting-for-input（階段 2）、iPhone（階段 3）、記憶檢索與相關性（階段 4）、多 Agent 分工（階段 5）。

**E2E 驗收**（全部走正式路徑，Claude Code 與 Codex 各跑一次，原生桌面＋API 兩種入口）：
- E1-01 派工→claimed→verify（沿用 E0-02，加上 record 有 `actualModel`、`startedAt`／`claimedAt`、cost／token）。
- E1-02 active 中取消：終態 `cancelled`、SSE 與 GET 一致、子程序樹消失、無遲到結果（沿用 `scripts/tests/phase0/agent_smoke.py --cancel-when-active`）。
- E1-03 Codex deny：rollout 不再出現 `Rejected("approval request failed")`，而是 decline 語意；Claude 唯讀擋寫沿用 E0-10。
- E1-04 失敗原因：讓 provider 子程序異常結束（例如 fixture 送 error result），record.detail 與 from-session 出現原因；真 agent 路徑至少一次。
- E1-05 記憶 >16 KiB 時派送仍成功（bundle 有界截斷且回執標 truncated）。
- E1-06 原生桌面（HEAD 重建的 App）：`scripts/tests/tauri-work-cancel.py` 擴充為可選真 agent；至少各一次真 agent 派工＋取消。

**安全與資料門檻**：Policy Governor 不變；deny 必須 fail-closed（若 provider 不接受 decline 也不得變成 accept）；模型／effort 只是「請求值」，實際值仍以 provider 回報為準並分開記錄；不得為了顯示原因而把 provider 原始 stderr 全文送進 UI（有界、去除路徑／憑證）。

**真 Agent／真機需求**：Claude Code 2.1.x（claude.ai 登入）、Codex 0.153.x（ChatGPT 登入）；不需要 iPhone／ESP32。

**發布條件與 blockers**：四項 CI 全綠＋`scripts/tests/phase0/baseline.sh` 全套；真 agent E1-01～E1-06 各平台一次且證據歸檔；D1／D2／D3／D4 對應回歸測試存在。若做 K-01 契約變更則走 minor（0.9.0）；只修 D1–D4／D16 可走 patch（0.8.1）。Blockers：磁碟空間（原生重建）；D5／D6 需要產品決策。

### 1.1 下一個實作切片（最小、完整、可驗收）

**切片：取消與失敗的誠實終態（D1＋D2＋D3）**——三者同一條路徑、同一組測試就能驗完。

1. 紅燈先行：在 `crates/interaction-runtime/tests/gateway_loop.rs` 仿照現有的
   `an_interrupted_codex_turn_is_cancelled_not_claimed_completed`（:1772，斷言 :1817-1838）新增
   `an_interrupted_claude_turn_is_cancelled_not_failed`：用 `crates/interaction-runtime/tests/fixtures/fake_claude.sh`
   開一個長任務（fixture 內有 `sleep 3600` 的等待分支），`POST /interrupt` 後斷言 record.state 為 `Cancelled`、
   SSE `agent.session.state` 為 `cancelled`、close 後 `detail == "closed (was Cancelled)"`。現況會紅：`failed`。
2. 實作：`crates/interaction-agent-gateway/src/claude.rs`
   - `ClaudeHandle::interrupt`（:354-357）在送訊號前 `cancel_requested.store(true)`（比照 `codex_exec.rs:55-57、:120-122` 的 `Arc<AtomicBool>`）；`send_user_message` 開新 turn 時重置。
   - stdout task（:213-231）：收到 `TaskFailed` 或子程序被訊號結束且 `cancel_requested` 為真時，改送 `GatewayEvent::TaskCancelled`；`parse_claude_line` 維持純函式不變。
   - runtime 端 `crates/interaction-runtime/src/gateway.rs:579-588` 已能處理 `TaskCancelled → "cancelled"`，不需改。
3. 同步修 D2：`crates/interaction-runtime/src/agents.rs:1371-1379` 的 close 投影改為輸出 record 的真實終局狀態
   （`failed`／`timed-out`／`unknown` 不再壓成 `closed`），新增單元測試 `close_projection_preserves_terminal_state`；
   對照 `apps/interaction-desktop/src/statusProjection/workState.ts` 確認文案不需改。
4. 同步修 D3：`report_agent_session`（`agents.rs:1209-1218`）在 `failed` 時把連接器的錯誤摘要寫進 `record.detail`
   （有界 200 字、去除絕對路徑），新增測試 `report_agent_session_sets_detail_on_failure`。
5. 綠燈後的真 agent 迴歸（不可省）：`python3 scripts/tests/phase0/agent_smoke.py --agent claude-code --port 19010 --out <dir> --task "<1500 字長文>" --cancel-after 3 --cancel-when-active --cancel-mode interrupt --close-after-cancel 5`，期望 final `cancelled`；Codex 同命令期望仍 `cancelled`（不退步）。
6. 文件：CHANGELOG `[Unreleased]` Fixed；`docs/releases/phase-0-known-issues-reproducibility.md` 的 D1／D2／D3 標「已修（commit）」；`docs/acceptance-evidence.md` 補一列。

**不做**：不改 codex 路徑、不加 model 欄位、不動 UI 文案、不處理「停止這一輪但保留 session」（D8，階段 2）。

## 2. 階段 2：流式、非同步、追問回答、修改、取消與恢復

**使用者可見成果**：看得到 agent 正在寫什麼（逐訊息或有界逐段）、正在用哪個工具；agent 提問時卡片變成「等你回答」，人類可以稍後回答並接續；執行中可補充需求；「停止這一輪」與「結束工作」是兩個動作；daemon 重啟後能看到「你離開時發生了什麼」並選擇續開。

**前置依賴**：階段 1（誠實終態、模型記錄）；B-02／B-03 的投影管道設計（SSE 事件或 mailbox progress）。

**owner／契約變更**：`GatewayEvent::TaskProgress` 進入 SSE／mailbox（B-02：目前只進觀察儲存；截斷 2000 字）；`TaskWaitingForInput` 由連接器產生（D11／D7：Codex `item/tool/requestUserInput` 以 `ToolRequestUserInputResponse` `{answers}` 回覆；Claude `-p` 無提問管道→誠實標 unsupported 或以「claimed 但內容是問題」的偵測降級）；interrupt 分成「停止本輪」與「結束 session」（D8：`agents.rs:900-904`、`gateway.rs:579-588`）；SSE 階段名與 record 狀態統一（D10）；SIGTERM 優雅關閉（D15）；create 回應即含 `providerSessionId`／保存 `resumeProviderSessionId`（D20）；核可請求持久化跨重啟（A-06）；resume 驗 agentId／擋雙重續開（C-10）；mid-turn 補充訊息的實際行為（B-06）由真 agent 驗證後定契約；重啟後的接續 UX（B-05）。

**不在範圍**：語音、iPhone 端回答（階段 3）、記憶檢索（階段 4）。

**E2E 驗收**：E2-01 串流可見（SSE 事件含文字片段，UI 卡片即時更新）；E2-02 提問→稍後回答→接續（Codex 真 requestUserInput；Claude 記錄不支援）；E2-03 停止本輪後同 session 再派任務成功；E2-04 執行中補充被排隊或合併並可觀察；E2-05 daemon SIGTERM／SIGKILL 後重啟：期間結果摘要與核可紀錄不遺失；E2-06 重複／遲到訊息去重可觀察（B-09）。

**安全與資料門檻**：串流內容仍是資料不是指令；mailbox／SSE 有界；waiting-for-input 的回答只能由人類 token 送；提問內容不得含憑證外洩。

**真 Agent／真機需求**：兩個真 agent；不需真機。

**發布條件與 blockers**：minor 版本；blocker：Claude `-p` 模式提問管道能力（需驗證 `AskUserQuestion`／`--input-format stream-json` 在 -p 下是否可用）。

## 3. 階段 3：真 iPhone 跨裝置接續與控制

**使用者可見成果**：手機看得到目前工作的身分與最後一則訊息／問題內容，能回答提問、核准／拒絕（F-03／F-04）；離開電腦後在手機接續；前背景切換與斷線重連不丟狀態；核心離線時手機標示「舊狀態」。

**前置依賴**：人類在 Xcode → Settings → Accounts 選 Team（簽章；階段 0 的唯一真機 blocker）；階段 2 的 waiting-for-input 與訊息投影；AIP 1.1 定義 approval／answer 訊息（目前 `approval-*` 只有型別）。

**owner／契約變更**：`docs/aip/README.md` 新增 approval／answer profile 與「已配對手機是否為可信人類介面」的裁決；`crates/interaction-runtime/src/mobile.rs`＋iOS `Services`；`crates/interaction-runtime/examples/fake_iphone.rs` 同步（本 checkpoint 已修 reconnect 不再硬退出）。動畫維持本地執行，跨裝置只同步語意事件與權威狀態。

**不在範圍**：ESP32／BLE peripheral／Serial 實體硬體驗收（維持 compile check 與 fixture）；多主機／雲端同步；推播服務。

**E2E 驗收**：E3-01 真機配對→工作真相投影（session 身分＋最後訊息）；E3-02 真機回答提問／核准，核心 audit 記錄 actor=human-mobile；E3-03 前背景／系統終止／Wi-Fi 切換後重連（含 AIP resume revision 接續）；E3-04 核心離線→手機標示舊狀態→重啟後恢復；fixture 版本先在 CI 走一遍再上真機。

**安全與資料門檻**：每機 token 只能做該裝置被授權的動作；手機端核准不得提高後端安全上限；撤銷即斷線；敏感感測（麥克風）預設關閉且三處同步顯示。

**真 Agent／真機需求**：真 iPhone（已在手邊：iPhone 11、iOS 26.3.1）＋一個真 agent。

**發布條件與 blockers**：真機證據逐列標示（沿用 `v0.5.0-iphone-device-evidence.md` 格式）；blocker：Xcode Team 簽章（人類操作）。

## 4. 階段 4：記憶查詢、來源關聯、上下文策略、Agent 原生 Session 利用

**使用者可見成果**：新偏好取代舊偏好而不是兩筆都塞給 agent（D13／E-07）；bundle 依任務相關性挑選而不是固定最新 N 筆（D-04）；桌面看得到「這次派送實際附了哪些記憶」與來源（G-05／D-03）；查詢、排序、分頁（E-04）；知識節點可刪除（E-03）；刪除記憶後能看到「哪些已派送 session 仍持有舊內容」（E-09）；agent 原生 session／專案記憶的利用方式有明確政策（E-08）。

**前置依賴**：階段 1 的 bundle 上限修正；階段 2 的投影管道。

**owner／契約變更**：`crates/interaction-runtime/src/memory.rs`（bundle 選取、supersede）、`crates/interaction-storage`（分頁／索引；語意檢索仍為離線稀疏特徵，E-05）、題詞模板版本化（D-02）、bundle 結構化管道（D-03：不只塞進任務文字）、信任邊界（D-05）。

**不在範圍**：神經向量檢索服務、雲端記憶、多主機。

**E2E 驗收**：E4-01 新偏好取代舊偏好→agent 只拿到新值；E4-02 任務相關性（同 agent 不同任務拿到不同 bundle）；E4-03 派送收據在桌面可查；E4-04 刪除後的持有清單正確；E4-05 分頁／排序 API 契約測試；沿用 E0-05 為回歸。

**安全與資料門檻**：agent 可見範圍與 denylist 不放寬；候選不得未複審即進 bundle；provenance 不得偽造。

**真 Agent／真機需求**：一個真 agent 即可。

**發布條件與 blockers**：minor；blocker：無（設計決策在本階段內完成）。

## 5. 階段 5：進階同種／異種協作、一般模式、視覺化、安裝與產品交付

**使用者可見成果**：分工、依賴與成果整合（不重做階段 0 的多 Session 基線）；執行中 turn 的並行上限與排程（C-05）；每 session 唯一工作目錄／worktree（C-07）；一般模式的工作時間線與待回答事項（G-01）；Windows 安裝路徑（A-01：`get.sh` 無 Windows 分支）；簽章／公證；整體備份還原（A-08）；動作收據 completed 視覺語法修正（G-06）。

**前置依賴**：階段 1–4。

**不在範圍**：ESP32 真板驗收（仍為 compile check）；新的公開 SDK；MCP（永遠不做）。

**E2E 驗收**：E5-01 三個以上 session 的分工與成果整合（真 agent）；E5-02 達上限明確拒絕（沿用 fixture 上限測試＋真 agent 一次）；E5-03 一般模式五入口真人受測；E5-04 各平台安裝 smoke（Windows／Linux 需環境）。

**發布條件與 blockers**：完整 release gates；blocker：Windows／Linux 驗收環境、簽章憑證。

## 6. 本 checkpoint 之後立即可做、不屬任何階段的維護

- 用有斷言的 AX helper 重驗 2026-09-06 的 settings-clean／settings-legacy 綠燈（D18：舊 helper 無條件回報成功，可能是假陽性）。
- 磁碟清理後從 HEAD 重建原生 App，重跑 `scripts/tests/tauri-*.py` 四支，讓 native 基線綁定 HEAD。
- `docs/releases/v0.8.0-known-limitations.md`：補「agent 子程序孤兒回收為 best-effort、只在下次重啟才 reap」與 Codex MCP／writable_roots 邊界。
