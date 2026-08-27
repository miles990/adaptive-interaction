# Capability Completion Matrix

每項能力的垂直閉環狀態。欄位值：`complete`／`partial`／`missing`／`n/a`（附理由）。
**任何必要欄位不是 complete，整體不得宣稱完成。** 機器證據欄指向可重跑命令或檔案。

圖例：Prov=Provider、Ad=真實 Adapter、RT=Runtime 接線、Pol=Consent/Policy、
CC=控制中心、Comp=Companion 呈現、Tray=Tray/Stop/Cancel、API、CLI、
Rcpt=Receipt/Verification、UT=Unit、IT=Integration、E2E、Ev=Machine Evidence。

## v0.3 既有能力（回歸基線 2026-08-27：Rust 201/201、vitest 42/42、PW 11/11、CLI-E2E 12/12）

| Capability | Prov | Ad | RT | Pol | CC | Comp | Tray | API | CLI | Rcpt | UT | IT | E2E | 已知限制 | Ev |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 內建受器動器（builtin） | complete | complete | complete | complete | complete | complete | complete | complete | complete | complete | complete | complete | complete | 單一扁平 API token | `cargo test --workspace`; `scripts/v03-cli-e2e.sh` |
| 麥克風 listen（30s 硬上限） | complete | complete | complete | complete | complete | complete | complete | complete | complete | complete | complete | complete | partial | 真實 cpal 擷取未實測（避免未同意錄音）；fake source 全測 | sensors_loop 測試; CLI-E2E #sensors |
| 攝影機 | n/a（誠實未實作） | missing | n/a | n/a | complete(顯示未支援) | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a | 誠實未實作 | docs/acceptance-evidence.md |
| 宣告式外部裝置（YAML→HTTP/SSE） | complete | complete | complete | complete | complete | complete | complete | complete | complete | complete | complete | complete | complete | 無裝置端逐請求 crypto | adapter-declarative 測試; CLI-E2E mock device |
| AI Agent Session（lease/mailbox/委派） | complete | n/a(v0.3 無外部接線) | complete | complete | complete | complete | complete | complete | complete | complete | complete | complete | complete | v0.3 僅 mailbox 模型，無真實 agent 子程序 | agents_loop 測試 |
| Emergency Stop 全鏈傳播 | complete | complete | complete | complete | complete | complete | complete | complete | complete | complete | complete | complete | complete | — | estop E2E; agents_loop estop |
| Runtime Supervisor（外部/內嵌） | complete | complete | complete | complete | complete | complete | complete | complete | complete | n/a | complete | complete | complete | 外部 Degraded 無健康輪詢（fail-closed） | supervisor 測試; offline.spec |
| 小樞角色視窗（v0.3 形態） | partial(未註冊 provider) | complete | partial | partial(無逐項 consent) | complete | complete | complete | n/a | n/a | missing(角色動作無 receipt) | complete | complete | complete | v0.4 主要重構對象 | companion.test; 實機截圖 |

## v0.4 新能力（本輪目標；初始全 missing，隨 Phase 更新）

| Capability | Prov | Ad | RT | Pol | CC | Comp | Tray | API | CLI | Rcpt | UT | IT | E2E | 已知限制 | Ev |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| Presentation Provider（角色逐項能力） | complete | complete | complete | complete | partial(經既有能力頁；新 IA 見 Phase 7) | complete | complete(estop 清佇列) | complete | complete | complete | complete | complete | partial(CLI E2E 7 檢查；瀏覽器 E2E 見 Phase 7) | 表面 ack=AcknowledgedOnly 證據（無獨立觀察者）；flat token 可偽 ack（沿用已知限制①） | `cargo test -p interaction-runtime --test presentation_loop`(8); `scripts/v03-cli-e2e.sh`(19) |
| 角色能力預設矩陣＋隱藏即停用 | complete | complete | complete | complete | partial(既有能力頁) | complete | n/a(隱藏≠estop) | complete | complete | n/a | complete | complete | partial | 音效/語音/視窗調整/顯示隱藏 4 項 consent-gated 預設停用；隱藏時 ingest 確定性拒絕 | presentation_loop::hidden_companion_stops…; consent_gated… |
| 小樞貓系重設計（3 變體） | n/a(美術) | n/a | n/a | n/a | partial(設定頁可選；預覽頁見 Phase 7) | complete | n/a | n/a | n/a | n/a | complete(出貨 pack 驗證) | n/a | partial(實機截圖見 Phase 7 驗收) | 3.0 頭身（spec 3–3.5 下緣）；v1 packs 保留相容 | scripts/shu/generate.mjs; montage 截圖; vitest packs |
| Behavior Runtime（三層＋Utility AI） | n/a(本機確定性) | n/a | n/a | complete(RM 只留眨眼/quiet 停玩鬧/estop 凍結) | missing(狀態說明頁見 Phase 7) | complete | n/a | n/a | n/a | n/a | complete(11 tests) | partial | partial | 程序化眼球/耳朵疊加層未做（錨點已輸出到 manifest，反應鏈烘焙於 notice/curious 時間軸）；familiarity 僅存在記憶體 | vitest behavior.test.ts(11) |
| 主動式對話模式＋頻率限制 | n/a | n/a | complete | complete(Rust 確定性 gate＋跨重啟) | partial(設定區塊；完整 AI 設定頁見 Phase 7) | complete(快捷安靜選項) | missing(tray 快捷入口見 Phase 7) | complete | complete | complete(Silenced 決策入 receipt) | complete(6) | complete(4) | partial(CLI E2E 2 檢查) | 生成式每日預算欄位已定義、實際計量在 Phase 3 connector 接線 | proactive_loop tests; `interact-ai proactive` |
| behaviorIntent Schema 驗證 | n/a | n/a | complete | complete(白名單＋長度＋控制字元) | n/a | complete | n/a | complete(經 plan payload) | n/a | complete(拒絕=Failed receipt) | complete | complete | partial | Schema 隨 release.sh golden 重生輸出 | presentation_loop::behavior_intent_whitelist… |
| Codex Connector（app-server＋exec fallback） | complete(provider.ai-agent.codex) | complete(app-server JSON-RPC，schema 鎖定) | complete | complete(sandbox=read-only＋approval 預設拒絕＋逾時 deny) | partial(Phase 7 IA) | partial(狀態經事件) | complete(estop 殺子程序樹) | complete | complete | complete(claim=inference 0.5) | complete(2) | complete(真實驗收) | partial | exec fallback 未實作（app-server 不可用時誠實拒絕）；成本無 USD 報告 | 真實驗收：thread 01a04194… claimed-completed |
| Claude Code Connector（stream-json） | complete(provider.ai-agent.claude-code) | complete(stream-json＋--permission-mode plan) | complete | complete(唯讀 plan＋不用 skip-permissions) | partial(Phase 7 IA) | partial | complete | complete | complete | complete | complete(4) | complete(fake＋真實驗收) | complete(CLI E2E 6 檢查) | 互動核可管道在 -p 模式不存在（plan 模式寫入直接拒）；成本較高（真實驗收 $0.42/turn） | 真實驗收：session f200e65c… claimed-completed、cost 入預算；gateway_loop 測試 |
| Agent Gateway 正規化事件 | n/a | n/a | complete | n/a | partial | partial | n/a | complete(/v1/agents{,/refresh,/routing}) | complete(agents providers/route/approve/interrupt) | complete(claim 走 report 誠實路徑) | complete | complete | complete | — | gateway_loop; CLI E2E gateway 段 |
| AI 交互設定 UI（§9 五區） | n/a | n/a | n/a | complete | complete(AI 頁：連接/路由/session/授權預覽；對話=設定頁主動對話區；記憶=Bundle 預覽頁) | n/a | n/a | complete | n/a | n/a | complete | n/a | complete(PW ai-page 測試) | 任務範圍細項（Read/Write/Test/Network 逐項）以唯讀模式統一承載；寫入模式 UX 為下一階段 | PW app.spec AI 頁測試; 截圖 desktop-ai.png |
| 記憶分層＋保存期限 | n/a | n/a | complete(storage v4＋watchdog 清除) | complete(actor 降權/secret 拒收/三態期限) | missing(記憶頁見 Phase 7) | n/a | n/a | complete | complete | complete(audit) | complete(5) | complete(5) | partial(CLI E2E 5 檢查) | 匯出上限 1000 條；樣態偵測非完美（誠實記載） | memory_loop; CLI E2E memory 段 |
| Context Bundle | n/a | n/a | complete(確定性選擇) | complete(stale/敏感/denylist/candidate 排除) | missing(「本次提供了什麼」UI 見 Phase 7) | n/a | n/a | complete | complete | n/a | complete | complete | partial | bundle 上限 24 條/48KB | memory_loop::context_bundle… |
| 多模態素材庫（CAS） | n/a | complete(File-CAS sha256 write-once) | complete | complete(AI 不可覆寫/刪除來源；刪除有影響預覽) | missing(素材頁見 Phase 7) | n/a | n/a | complete(/v1/assets*) | complete | complete(audit) | complete | complete(2) | partial(CLI E2E 2 檢查) | 匯入走本機路徑/行內文字（無 multipart 上傳）；影像/音訊衍生解析（縮圖/OCR/轉錄）未實作——衍生資料模型與片段引用已就緒 | knowledge_loop::assets…; CLI E2E |
| Knowledge Graph＋FTS＋向量介面 | n/a | n/a | complete(storage v5＋FTS5 bm25) | complete(claim 必附證據/類比≠因果/superseded 不參與回答) | missing(圖譜頁見 Phase 7) | n/a | n/a | complete | complete | n/a | complete(4) | complete(6) | partial(CLI E2E 7 檢查) | 向量=lexical-fallback（誠實標示，非語意 embedding，可替換介面）；圖展開深度 1 | knowledge_loop; `interact-ai knowledge` |
| Knowledge Tools（Candidate-only 寫入） | n/a | n/a | complete(9 個 canonical tools) | complete(AI 寫入一律 Candidate；approve 只屬人類，agent 裁決降留言) | missing(候選複審頁見 Phase 7) | n/a | n/a | complete(tools/call) | complete | complete | complete | complete | partial | golden schemas 已重生 | tool-schema tests; knowledge_loop::agent_proposals… |
| 知識更新決策器 | n/a | n/a | complete(純函式決策表＋freshness/conflict sweep) | complete(外部研究必先問；健檢只做低成本) | missing(Phase 7) | n/a | n/a | complete(/update-check) | complete | complete | complete(3) | complete(2) | partial(CLI E2E 2 檢查) | AI 步驟本身由 host 執行（決策器只裁定要不要/能不能） | curator tests; `interact-ai knowledge update-check` |
| 經驗轉知識流程 | n/a | n/a | complete(close 時確定性收集＋學習訊號→Reflection Candidate) | complete(升格需反例＋適用範圍＋證據；單次偶發結構性無法普遍化) | missing(Phase 7) | n/a | n/a | complete(經 knowledge API) | complete | complete | complete | complete(1) | n/a | 使用者糾正訊號需 UI 入口（Phase 7）；AI 回顧內容由 host 生成後仍走 Candidate | curator_loop::experience… |
| Knowledge Receipt | n/a | n/a | complete(storage v6＋knowledge.updated 事件) | n/a | missing(Receipt Viewer 見 Phase 7) | missing(六句固定文案接線見 Phase 7) | n/a | complete(/receipts) | complete | complete(誠實三態 conflictCheck/humanReviewed) | complete | complete | partial(CLI E2E 1 檢查) | — | curator_loop; `interact-ai knowledge receipts` |
| 硬體能力掃描（發現模型） | partial(builtin/宣告式/agent/presentation providers) | partial | complete(生命週期 discovered→…→enabled 已有) | complete(掃描不啟動感測) | complete(掃描 UI＋誠實文案＋未支援清單附原因) | n/a | n/a | complete | complete | n/a | complete | complete | complete(PW provider 分頁) | OS 層 HID/BLE/MIDI/mDNS 列舉 adapter 誠實未實作（UI 明列原因） | CapabilitiesHub; desktop-capabilities.png |
| 控制中心新 IA（8 一級頁） | n/a | n/a | n/a | n/a | complete(9 項一級導覽＝8 必要頁＋自動互動；進階 +Provider Registry/Knowledge Graph) | n/a | complete(tray 深連結沿用) | n/a | n/a | n/a | complete(vitest 61) | n/a | complete(PW 19：8 頁可達＋390px＋鍵盤) | 進階 Agent/Session 原始頁沿用 AI 頁詳情；一般↔進階跳轉=模式切換＋既有技術頁 | PW app.spec 新 IA 測試; 22 張截圖 docs/assets/v04-evidence |
| Global Search／Command Palette | n/a | n/a | n/a | complete(指令只列可執行；estop 永在) | complete(⌘K/Ctrl+K＋topbar 按鈕) | n/a | n/a | n/a | n/a | n/a | complete | n/a | complete(PW 搜尋導頁測試) | 搜尋與指令合一面板（功能覆蓋兩者） | PW 全域搜尋測試; desktop-global-search.png |
| Activity Inbox（統一待辦） | n/a | n/a | complete(彙整 assists＋waiting sessions＋知識候選) | n/a | complete(活動頁頂部＋首頁摘要卡) | n/a | n/a | complete(既有 API 彙整) | n/a | n/a | complete | n/a | complete(PW 待我決定) | 篩選器（依 agent/裝置/domain）為基本版 | PW activity 測試; desktop-activity.png |
| Consent Sheet／Receipt Viewer／Source Viewer | n/a | n/a | n/a | n/a | complete(建立 session 授權預覽；知識收據檢視；素材影響預覽) | n/a | n/a | n/a | n/a | n/a | complete | n/a | complete(PW consent sheet 斷言) | Source Viewer 的片段預覽（畫圖區域/時間軸播放）為 JSON 級 | PW AI 頁測試; MemoryKnowledgePage |

n/a 理由備註：
- 「n/a(本機確定性)」：Behavior Runtime 是純本機演算法，無外部 provider/adapter 層。
- 「n/a(美術)」：角色美術是資料資產，不經 policy/API 層；其載入驗證在 UT/E2E 欄。
- 「隱藏≠estop」：隱藏角色刻意不提供 Tray 停止語意（spec §3）。
- 攝影機維持 v0.3 誠實未實作立場（spec §一 允許）。
