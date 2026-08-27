# Changelog

本專案採 [Keep a Changelog](https://keepachangelog.com/zh-TW/1.1.0/) 格式與
[語意化版本](https://semver.org/lang/zh-TW/)（`MAJOR.MINOR.PATCH`）。

版本一致性：workspace `Cargo.toml`、`apps/interaction-desktop/src-tauri/Cargo.toml`、
`apps/interaction-desktop/src-tauri/tauri.conf.json`、`apps/interaction-desktop/package.json`
四處版本必須相同——用 `scripts/release.sh <version>` 一次搞定。

## [Unreleased]

### Added — v0.4：Presentation Provider、真實 Agent Connector、記憶與知識系統、控制中心新 IA

- **Presentation Provider**（`provider.companion.shu`）：桌面角色正式成為一級 provider，
  能力**逐項**宣告——7 個語意 receptor（點擊／文字輸入／快捷／拖放／游標語意／動畫事件／
  氣泡事件）＋7 個 actuator（狀態呈現／播放動畫／氣泡／音效／語音／視窗調整／顯示隱藏）。
  誠實迴路：`presentation.command` SSE → 視窗渲染 → ack → receipt
  （Dispatched→Acknowledged→Completed，證據誠實標 AcknowledgedOnly；10 秒無 ack →
  Uncertain）。音效／語音／視窗調整／顯示隱藏 consent-gated 預設停用；隱藏角色時視窗內
  receptor 確定性拒絕 ingest（隱藏≠緊急停止）。behaviorIntent／動畫白名單 runtime 驗證，
  成功／阻擋／緊急等真相狀態 AI 不可點播。
- **小樞 v2 貓系重設計**：3 頭身 Q 版貓系數位小精靈原創 rig——眼尾上揚狡黠眼＋逐眼眉毛
  眼皮（「發現了」「真的假的」「讓我看看」）、貓耳=注意力顯示器（earPerk）、尾巴重量與捲曲、
  23 個動畫（察覺反應鏈：耳先動→眼亮→頭後轉；失敗專屬美術＝愣住→認真檢查＋✕；
  伸展／晃腳／抱尾巴／趴下慵懶反差）。三變體共用骨架：靈巧（預設）／慵懶／活潑。
  成功綠勾契約不變（幀 0-1 點頭、幀 2-3 才有勾）。v1 packs 保留相容。
- **Behavior Runtime**：純本機確定性三層生命系統（不用生成式 AI 逐幀控制）——
  BehaviorState 平滑量（activation／attention／taskLoad／readiness／familiarity）、
  Utility AI 優先階梯（緊急>感測安全>等待確認>直接互動>任務>建議>世界觀>待機）、
  hazard 抽樣微動作排程（幾何分布間隔、反重複、放鬆度門檻、Reduced Motion 只留眨眼、
  seeded RNG 可重現）＋程序化視線／耳朵疊加（安全／Reduced Motion 狀態凍結）。
- **主動式對話政策**：off／necessary／natural（預設）／lively／custom 五模式，Rust 確定性
  強制——每小時上限 3、最短間隔 12 分、30 秒合併、未回覆不追問、勿擾延後、安全類只去重
  永不被壓制；狀態跨重啟持續。氣泡快捷「一小時不要說話」「今天安靜一點」。生成式觸發
  會建立真實、有期限/訊息/費用範圍的本機 Agent Session，只呈現通過 schema 的候選。
- **Agent Gateway（無模型 API）**：直接連本機已登入的 **Codex**（`codex app-server`
  stdio JSON-RPC，協定 schema 由 `generate-json-schema` 鎖定；不支援時降級至
  `codex exec --json`／`exec resume`；sandbox=read-only、approval 預設拒絕、逾時
  自動 deny）與 **Claude Code**（`claude -p` stream-json；預設 plan-only，經明確
  workdir＋二次確認才開限權 `acceptEdits`，絕不用 skip-permissions）。claims 走 v0.3 誠實路徑
  （inference 0.5、claimActionId 防偽）；成本入 SessionBudget 硬上限；estop／close／
  lease 到期終止整棵子程序樹；子程序不跨重啟存活；不讀取任何 credential。
  確定性路由建議（程式→Codex、文件→Claude Code、模糊→列兩者）。
- **記憶分層（10 層）＋保存期限三態**：expiresAt（到期停用並刪）／reviewAfter（stale 需
  重新確認）／until-deleted（不存在不可刪的永久記憶）。agent 寫入降權（fact→inference、
  長期使用者記憶→candidate＋30 天複查）；secret 樣態拒收；到期 watchdog 清除；
  **確定性 Context Bundle**（stale／敏感／denylist／candidate 排除並揭露）；每個真 task
  自動附上 Session/Domain 授權的最小 bundle 並持久化實際提供證據。JSON 備份可逐筆經
  human-only Runtime 驗證還原，不 raw overwrite SQLite。
- **多模態素材庫（CAS）＋知識圖譜**：SHA-256 內容定址 write-once 素材（AI 不可覆寫／
  刪除來源；刪除有影響預覽，失去來源的 Active 知識標 disputed 不靜默消失）；
  Entity／Claim／Source 節點＋12 種關係型別＋認識論來源標記（research-supported／
  analogy／ai-conjecture／user-confirmed…）；claim 必附證據（素材 hash＋片段語法）；
  類比／推測不可標因果；candidate→active→stale→disputed→superseded→archived 狀態機，
  只有 active 參與回答；FTS5（bm25）＋本機 sparse subword embedding（可重現、
  可替換，不宣稱 neural embedding）。縮圖與 WAV features 有 production pipeline；
  OCR/whisper/ffmpeg 本機工具缺少時回報 unavailable。
  9 個 `interaction.knowledge_*` canonical tools：AI 讀受限、寫一律 Candidate、
  agent 裁決降留言，activate 只屬人類。
- **知識更新決策器＋經驗轉知識＋Knowledge Receipt**：「要不要更新」與「要不要 AI」分開
  確定性判斷；外部研究必先問；發布三級政策（auto metadata／candidate-only AI 內容／
  必須人類確認）；freshness sweep 標 stale、衝突雙方標 disputed；任務結束確定性收集
  TaskMemory，學習訊號才建 Reflection Candidate，升格必須補反例＋適用範圍；
  每次知識變化寫機器可讀 receipt（誠實 conflictCheck 三態＋humanReviewed）＋
  `knowledge.updated` 事件。控制中心加入 update-check 與「使用者糾正」入口；糾正只建立
  30 天複查 UserMemory＋Knowledge Candidate。小樞把 receipt 映射為六種固定安全文案。
- **metadata-only 硬體掃描**：跨平台 discovery adapter 介面＋17 類覆蓋報告；掃描不開
  攝影機／麥克風／HID／BLE／mDNS。Linux 只讀 `/dev/*/by-id` 穩定連結；macOS 用
  `system_profiler` 列舉 camera/mic/audio/display/USB HID/controller/serial/Bluetooth/MIDI，
  volatile path 不冒充永久身分；不可用／需權限／未支援皆附原因。首次設定、控制中心、
  API `POST /v1/hardware/scan` 與 CLI `providers scan` 共用同一 Runtime Truth。
- **API 身分分權**：human token（`state/api-token`）、restricted agent token
  （`state/api-agent-token`）分離、constant-time 驗證。agent token 可以讀狀態、呼叫
  canonical tools 與安全停止；Knowledge Tool 再綁 Session/Domain token；不能開／授權 session、改 policy、發佈知識、清除 estop
  或匯入／刪除素材。CLI 新增 `--agent-scope`；啟動 agent 子程序時移除所有 Runtime token env。
- **控制中心新 IA**：8 一級頁（首頁／小樞／AI 與工作階段／能力與裝置／記憶與知識／
  活動與確認／隱私與安全／設定）＋自動互動保留＋進階 Provider Registry／Knowledge Graph。
  小樞頁 13 個真實素材狀態預覽（未驗證 vs 已驗證對照可見）；AI 頁真實發現／session 卡片
  （claimed-completed 標「回報完成——尚未驗證」）／approval 裁決／Consent Sheet 授權預覽；
  能力頁掃描誠實文案＋未支援能力附具體原因；記憶知識頁含候選複審、素材影響預覽、
  收據、Bundle「本次提供了哪些」；活動頁統一「待我決定」收件匣；首頁「現在」摘要條；
  Global Search＋Command Palette（⌘K，指令只列可執行）。Activity 支援 Agent/裝置/
  Domain 複合篩選，Source Viewer 支援圖片區域、音視訊時間段與程式位置。畫面證據 100 張由 E2E 從
  真 App＋真 daemon 自動擷取（docs/assets/v04-evidence/）。
- API：`/v1/presentation*`、`/v1/proactive-dialogue*`、`/v1/agents*`、
  `/v1/agent-sessions/{id}/{approve,interrupt}`、`/v1/memory*`、`/v1/assets*`、
  `/v1/knowledge*`。CLI：`presentation`／`proactive`／`memory`／`assets`／`knowledge`
  ＋`agents providers|route|approve|interrupt`＋`agents create --workdir --max-cost --allow-write`
  ＋`providers scan`；全域 `--agent-scope` 使用 restricted token。
  Storage schema v3→v7。

### Fixed

- 出貨 persona pack（persona-shu／persona-navigator）因含 `succeeded-verified` 安全鍵
  被驗證器整包拒絕、persona 語句靜默失效（v0.3 對抗審查修正引入的回歸）。
  新增「出貨 pack 必須通過驗證」測試防再發。
- 佇列訊息文字可達 desktop-pet 頻道（is_text_channel 涵蓋）。
- **對抗審查（8 維度 67 agent、59 findings → 38 確認／21 駁回）確認缺陷 38/38 全修**，
  每項附 regression test（該輪 Rust 257→294、vitest 61→81；本次收斂後 336／94）。要點：presentation_ack 改
  persist-成功才發事件（estop 競態不再對事件流謊稱完成）；kill 路徑鎖外化＋pgid
  持久化＋重啟 reap 孤兒＋stdin 有界逾時（卡死 agent 不再擋 estop/close）；GET
  messages 不再對 gateway session 偽造送達；estop/create TOCTOU 與重複 close 修復；
  記憶 far-future horizon 降權漏洞、secret 掃描涵蓋 tags/provenance、過期即拒；
  知識空證據門檻、approve 復活終態、candidate 邊誤降 active、懸空 hash 拒升格、
  LIKE 子字串誤匹配、1000 上限靜默截斷全修；主動對話勿擾真確定性生效；前端
  estop 二段確認＋IME 防誤觸、匯出/拖放/計數/倒數文案誠實化。
- **指定 closing 對抗審查**（41 agents、36 findings → 12 confirmed／24 rejected、
  0 workflow errors）12/12 修復：Governor 原子 reservation、plan single-flight、recipe
  direct-run policy、late receipt evidence、duration overflow、bounded mock state、supersede
  schema、restricted SSE、nested transport error、rolling window、fusion tie-break、driver
  status-chain spoof。

### 已知限制（v0.4 初版快照；已由下方 closing audit 取代）

1. 單一扁平 API token 沿用（v0.3 已知限制①延續；presentation ack 同 token 可偽）。
2. Codex exec fallback 未實作（app-server 不可用時誠實拒絕）。
3. 向量檢索為誠實標示的 lexical-fallback，非語意 embedding（介面可替換）。
4. 影音素材衍生解析（縮圖/OCR/轉錄）未實作；資料模型與片段語法已就緒。
5. OS 層硬體列舉（HID/BLE/MIDI/mDNS/攝影機）誠實未實作，UI 附具體原因。
6. Claude `-p` 模式無互動核可管道；寫入型工作流程為下一階段。
7. 程序化眼球／耳朵疊加層未實作（錨點已輸出；反應鏈烘焙於動畫時間軸）。
8. 生成式主動對話的觸發端排程器為下一階段（閘門＋預算＋metadata 已就緒）。
9. 知識 UI 三末端未接線：update-check 觸發僅 API/CLI、「使用者糾正」專屬入口、
   角色端知識六句固定文案（§17）未接到氣泡（語意在控制中心完整呈現）。
10. agent 子程序孤兒回收 best-effort：pgid OS 快照歸因、重啟 reap 驗證存活＋
    leader＋command；極端 pid 重用可能誤殺、歸因不唯一時誠實放棄（warn）。

### 已知限制（v0.4 closing audit）

1. 同 OS 帳號的 Agent 程序隔離仍依 Codex／Claude Code sandbox；0600 token file
   不冒充完整程序隔離。
2. 本機向量為 sparse subword embedding，不宣稱 neural embedding。
3. OCR／whisper／ffmpeg derivative 依賴本機可選工具；未安裝時明確 unavailable。
4. Windows hardware discovery 尚未真機驗收；driver／權限／sandbox 仍可能讓裝置不可見。
5. 本機 Codex 支援 app-server，因此 exec fallback 以真子程序 fixture 而非真模型驗收。
6. Offline 是 App-level shared state，只保留 desktop／390px 各一張，不複製假頁面。
7. 記憶備份採逐筆驗證還原，不是跨筆 transaction 或 SQLite 覆寫。
8. agent orphan recovery 對無法唯一歸因的 pgid fail-safe 放棄並記錄 warning。

## [0.3.0] - 2026-08-26

### Added — 狀態列常駐、桌面角色、外部裝置、AI Session、感測層

- **狀態列常駐工具**：跨平台 tray／menu-bar（狀態文字永不只靠顏色）、暫停／暫停一小時／
  緊急停止直接走 Rust、「顯示/隱藏桌面角色」、「開啟控制中心」。關閉控制中心視窗改為
  **只隱藏**（首次顯示說明對話框，含 v0.2→v0.3 行為改變告知）；只有「完全結束」才
  優雅關閉內嵌 runtime。登入啟動預設關閉。single-instance 保護。
- **RuntimeSupervisor**：啟動時偵測既有 `interact-ai` daemon → 連線外部（前端切 HTTP
  transport，token 走 IPC）或啟動內嵌 runtime；外部模式下完全結束 app 不會關閉外部
  daemon；health loop 在斷線時誠實降級。狀態：Starting / EmbeddedOwned /
  ConnectedToExternal / Ready / Degraded / Disconnected / Stopping / Stopped。
- **桌面角色小樞**：第二個透明無邊框視窗，原創參數化 SVG rig → 確定性 sprite sheet
  （72 幀 × 3 變體 × 18 動畫）。確定性狀態機：completed=點頭、綠勾只給 verified、
  emergency 凍結一切；click-through（角色 hit-rect 外可穿透）、可拖曳並記憶位置。
- **多模態輸入**：`desktop.companion.interaction`（純語意事件，**永不記錄原始座標**）、
  `desktop.pointer.activity`（本 app 摘要，誠實限制）；點擊快捷操作（確定性）、文字輸入
  ＋資料去向預覽、拖放先預覽再確認。
- **Persona / World / Story packs**：純資料、有界、無可執行內容。**安全語句
  （緊急停止／被阻擋／結果未知／感測使用中）固定不可覆寫**——驗證器標記覆寫企圖，
  resolver 也直接無視。內建 persona-shu／persona-navigator／story-shu-intro。
  表現程度（安靜／自然／活潑）調整氣泡與眨眼節奏。
- **Capability Provider 統一模型**：device/service/application/ai-provider/ai-agent/
  ai-session/companion/human。生命週期禁走捷徑（配對≠安裝≠啟用≠同意），revoked 黏性；
  配對用 sha256 指紋（IP 不是身分）。
- **宣告式 adapter engine**（`interaction-adapter-declarative`）：YAML spec →
  真 HTTP/SSE receptor/actuator，無需寫 Rust。policy-bounded 模板替換（設備只看到限界後
  的值）、`secret://` 參照（env／secret store，不寫進 YAML）、SSRF 防護、
  retry/timeout/idempotency；不支援的 transport（WS/MQTT/Serial/BLE）誠實拒絕。
- **AI Agent Session**：Provider→Agent→Session 分離；有租約（明確續租、lazy 過期）、
  範圍（data/tool/consent）、預算（訊息/成本/時間硬上限）；mailbox 溝通（不互讀對話）；
  `agent.delegate` actuator 走同一 governor 管線；防循環信封（深度/循環/數量/預算）；
  estop 取消所有 session 並阻擋新建；open session 不跨重啟存活；bounded handoff（拒絕
  對話大小的 payload）。
- **感測層**（`adapters-media`）：麥克風 listen 視窗（30 秒硬上限、watchdog 掃描），
  原始音訊只在記憶體、只導出 level 事實，不存不傳、無 STT；預設關＋Intimate＋
  三重確定性閘門（no-estop＋enabled＋session consent）。**無靜默擷取路徑**（status／
  事件／狀態列 glyph／控制中心橫幅／小樞標籤全程反映）。攝影機誠實未實作。
- **新 API**：`/v1/providers*`、`/v1/agent-sessions*`、`/v1/sensors/microphone/listen`、
  `/v1/sensors/stop`；事件新增 `provider.registered/state-changed`、`sensor.started/stopped`。
- **新 CLI**：`interact-ai providers`、`agents`、`sensors`。
- **前端**：一般模式首頁多 Session 視圖、設定頁「桌面角色」區塊；HTTP transport 層
  （同一 typed API 走 Tauri IPC 或外部 daemon，含 fetch-SSE 重播）；Playwright 瀏覽器
  級 E2E（11 測試，真 daemon＋真瀏覽器）。

### Changed
- Storage schema v3：新增 `providers`、`agent_sessions` 表（自動遷移）。
- **誠實性修正**：best-effort 驗證不再把裝置 actuator 的 acknowledged 自動升級成
  completed，除非該 actuator 正式宣告 ack 即代表送達（本機通道）；設備需 observation 才
  completed。內建 canonical tool 現在攜帶正式 human meta（execute/recipe_run 保守標為
  Unknown，因下游影響取決於選中的 actuator）。
- CORS 改為 loopback-only 判定（任何 port：Tauri／vite dev／瀏覽器 E2E）；token 仍是
  實際守門。
- CI 新增瀏覽器 E2E job 與 alsa 系統相依；release 兩個 job 補 `libasound2-dev`。

### Security
- 30-agent 對抗審查（5 維度 find → 逐發現獨立 verify），確認缺陷全數修復（詳見
  `docs/acceptance-evidence.md`）。

## [0.2.0] - 2026-08-26

### Added — 人類理解層（Human Layer）
- **一般／進階雙模式桌面介面**：預設一般模式（首頁／感知來源／回應方式／工具操作／自動互動／同意與安全／活動紀錄／設定），進階模式完整保留原有技術頁面；模式偏好持久化於後端，CLI/API/UI 共用。
- **四層人類語意系統**：manifest 可選 `human` 欄位（presentation／data／effect／consent semantics，缺漏一律保守視為 unknown）→ 內建 44 條目的常見能力中央目錄（zh-TW/en、alias glob）→ 確定性 fallback（技術 ID 分詞＋schema 說明）→ AI 輔助說明（綁定 manifest hash、變更即失效、絕不覆蓋安全事實）。
- **首次設定精靈**：7 步 draft/commit，敏感與對外能力永不預選；套用走同一套 governor 驗證路徑。
- **句子式配方編輯器**：不寫 YAML 建立自動互動；自然語言摘要由結構化 recipe 確定性生成（Rust `summarize`）；YAML↔JSON 經單一模型無損轉換，未知欄位以 serde flatten 完整保留。
- **情境模擬**：安靜時段／缺同意／裝置離線／AI 不可用／低信心／資料過期／已提醒過／緊急停止，重用同一套 pure governor/planner，保證零副作用。
- **主動互動暫停（pause）**：與緊急停止語意分離的一般控制；暫停期間 recipe 不觸發、明確請求照常；持久化、重啟不消失、可設定期限自動恢復。
- **AI 介入決策閘門**：recipe 級 `ai` 策略（never／when-uncertain／generate-text…）；確定性事件絕不呼叫 AI；證據模糊時發布 `ai.assist.requested` 事件，逾時依 `onUnavailable` 確定性處理（fallback／no-action）；外部 AI 可在期限內以 `assists resolve` 回應。
- **緊急停止安全解除流程**：觸發（一鍵、二段確認）與解除（安全頁、顯示原因／恢復清單、高風險不自動恢復）分離。
- **新 API**：`/v1/capabilities/human`、`/v1/catalog`、`/v1/ui/preferences`、`/v1/onboarding/*`、`/v1/pause*`、`/v1/capabilities/{kind}/{id}/ai-description`、`/v1/ai-assists*`、`/v1/recipes/{id}/summary`、`/v1/recipes/{id}/simulate-scenario`、`/v1/recipes/convert`；事件新增 `proactive.paused/resumed`、`ai.assist.requested/resolved`。
- **新 CLI**：`capabilities --human`、`catalog`、`pause`／`resume`、`prefs`、`onboarding`、`describe`、`assists`、`recipes summary`、`recipes simulate --scenario`。
- 前端元件測試（vitest）：誠實性不變量（queued≠completed 等）、能力卡片、權限地圖、對話框、精靈。

### Changed
- Storage schema v2：新增 `ai_descriptions` 表（自動遷移）。
- Recipe JSON Schema 隨模型擴充（`ai`、未知欄位保留）。

## [0.1.3] - 2026-08-26

### Fixed
- 桌面 app：app 層級退出（Cmd+Q／AppleScript quit）現在也會優雅關閉內嵌
  runtime（RunEvent::Exit handler）；先前只有視窗關閉路徑會清理
- 動態註冊的 mock 裝置現在自動配對 `<id>.device-status` 受器，
  `observed` 驗證可完整閉環；`actuators remove` 一併移除配對受器
  （註冊/權限鏈實測發現的缺陷）

## [0.1.2] - 2026-08-26

### Fixed
- 緊急停止 clear 後，閂死（latched）的實體裝置 driver 現在會被重新武裝
  （新增 `Actuator::emergency_clear`，預設 no-op；動作仍不自動恢復）——
  全能力矩陣實測發現的缺陷

## [0.1.1] - 2026-08-26

### Changed
- `self install-skill` 跨 AI 化：自動偵測 Claude Code／Codex CLI／~/.agents／
  Gemini CLI／GitHub Copilot CLI 的 agent home，TTY 下提供選單（預設全選），
  非互動直接全裝；`--dest` 仍可指定任意位置
- 修復 CLI e2e 測試的埠競態（sequential port allocation）

## [0.1.0] - 2026-08-26

### Added
- 首版：12-crate Rust workspace（core／policy／recipe／storage／registry／events／
  runtime／tool-schema／api／cli／adapter-sdk／builtin adapters）
- `interact-ai` CLI（40+ 子指令、`--json` 潔淨輸出、穩定 exit codes、daemon 模式）
- HTTP API（axum、Bearer token、SSE + Last-Event-ID 重播、OpenAPI）
- Deterministic Policy Governor（min() 限界鏈、consent、quiet hours、預算、
  pre-dispatch gate、sticky terminal receipts）
- Recipe 引擎（六種觸發融合、六種編排模式、事件消耗語意、跨重啟狀態持久化）
- Canonical Tool Manifest → OpenAI／Anthropic／Gemini／OpenAPI／JSON-Schema 產生器
  （golden tests）
- 跨 AI Agent Skill（`orchestrate-adaptive-interaction`）
- Tauri 2 桌面控制中心（總覽／受器／動器／工具／配方／政策／時間軸＋緊急停止）
- `interact-ai self` 自我管理（update／uninstall／version／install-skill／install-desktop）
- 文件：ELI5 安裝、特點能力、人類使用手冊、桌面指南（mermaid＋插圖）
- 25-agent 對抗式審查，14 項確認缺陷全數修復；105 測試

[Unreleased]: https://github.com/miles990/adaptive-interaction/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/miles990/adaptive-interaction/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/miles990/adaptive-interaction/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/miles990/adaptive-interaction/releases/tag/v0.1.0
