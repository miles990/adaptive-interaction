# Changelog

本專案採 [Keep a Changelog](https://keepachangelog.com/zh-TW/1.1.0/) 格式與
[語意化版本](https://semver.org/lang/zh-TW/)（`MAJOR.MINOR.PATCH`）。

版本一致性：workspace `Cargo.toml`、`apps/interaction-desktop/src-tauri/Cargo.toml`、
`apps/interaction-desktop/src-tauri/tauri.conf.json`、`apps/interaction-desktop/package.json`
四處版本必須相同——用 `scripts/release.sh <version>` 一次搞定。

## [Unreleased]

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
