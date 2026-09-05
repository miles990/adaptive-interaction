# v0.5 Capability Gap Matrix（產品重定位基線）

> 本輪目標:把產品主體拉回三核心 —— **角色生命感與遊戲互動 > 真實硬體閉環 > AI Agent 工作與對話閉環**。
> 本文件是 Phase 0 的誠實基線:不沿用 v0.4「25/25 complete」的完成度敘述;
> 那份矩陣衡量的是治理平台的完成度,不是角色遊戲性與真實硬體的完成度。

## 0. 基線

| 項目 | 值 |
|---|---|
| 基準 commit | `d200e2c` release: v0.4.1(= origin/main,worktree 乾淨) |
| 日期 | 2026-08-28 |
| 環境 | macOS 26.2(Darwin 25.2.0)、Apple M2 Pro(12 核)、rustc 1.94.0、node 24.5.0、pnpm 10.27.0 |

### Phase 0 回歸實測(重定位動工前)

| 套件 | 命令 | 結果 |
|---|---|---|
| Rust fmt | `cargo fmt --check` | 通過(exit 0) |
| Rust clippy | `cargo clippy --workspace --all-targets -- -D warnings` | 通過,0 warnings |
| Rust tests | `cargo test --workspace` | **336 passed / 0 failed / 0 ignored** |
| 前端 typecheck | `pnpm typecheck` | 通過 |
| 前端 unit | `pnpm test`(vitest) | **94 passed / 0 failed**(11 檔) |
| 前端 build | `pnpm build` | 成功 |
| Tauri tests | `cargo test`(src-tauri) | **4 passed / 0 failed** |
| CLI E2E | `./scripts/v03-cli-e2e.sh` | **51 passed / 0 failed** |
| Playwright | `pnpm test:e2e` | **23 passed / 0 failed**(52.7s) |

> **閱讀提示（2026-09-04 補註）**：§1–§6 各表的「狀態」欄是 2026-08-28 Phase 0 動工前的**基線快照**，刻意不回寫；
> 表中寫「**缺**」的項目（例如 §4 的五個一級入口、三步精靈、右上 Inbox、設定單一主人、風險分級）多數已在 Phase 1–9
> 落地，現況以 §9–§12（與 v0.5.1 的 `docs/releases/v0.5.1-*.md`）為準。

## 1. 核心一:角色生命感與遊戲互動

| 能力 | 現況(v0.4.1 實際程式碼) | 目標 | 狀態 |
|---|---|---|---|
| 渲染 | 單 canvas 整幀 sprite blit(`SpriteRenderer`,128×128、8 欄 sheet),僅 idle 有 gaze/ear 微動疊加 | 分層組合通道(body/head/gaze/eyes/ears/tail/hair/bubble/audio/position/particles) | **缺** |
| 動畫數 | v2 packs 24 個具名動畫(整幀) | 36 表情 × 進入/保持/小循環/離開 | **缺** |
| Interaction Director | 無;僅 transient 優先階梯(machine.ts)+ 微動作排程(behavior.ts) | Event Normalizer→Attention→Utility→Intent→Scheduler→Mixer | **缺** |
| 自主移動 | 角色從不自行移動(僅使用者拖曳或 `companion.window.adjust`) | 散步/奔跑/急停/跳躍/攀爬/邊緣探頭 | **缺** |
| 游標互動 | 設計上排除原始座標;只有 30s 節流的 pointer-approached + click/drag | 追游標/躲游標/擋游標(本機 16–100ms,不出 WebView) | **缺** |
| 玩具與物理 | 無任何物理/玩具程式碼 | 毛球/紙團/光點/逗貓棒/紙飛機 + 輕量 2D 物理 | **缺** |
| 多角色 | 單一 companion 視窗硬編碼、prefs 單槽 | 多角色/使魔、互相注意、追逐、Roll Call | **缺** |
| 命名/場景/匯入匯出 | 無命名、無場景、pack 固定 5 選項下拉 | 命名、場景切換、角色設定匯入匯出 | **缺** |
| 檔案接取 | 有(drag-drop 確認流程,誠實等待 push 結果) | 保留並加上角色演出 | 部分 |
| 本機反射延遲 | click/drag 同幀反應(<16ms)（Phase 0 基線快照的說法；後續實測只量到 WebView 內段 ~8 ms，**端到端未量、不得宣稱達標**，見 `docs/acceptance-evidence.md` 效能節） | 16–100ms | **已有**（WebView 段） |
| 誠實演出 | verified 才有綠勾;truth-state 動畫不在 AI 可播白名單 | 保留 | **已有** |

## 2. 核心二:真實硬體閉環

| 能力 | 現況 | 目標 | 狀態 |
|---|---|---|---|
| HTTP/SSE adapter | 完整(declarative YAML、SSRF 防護、secret://、retry/timeout) | 保留 | **已有** |
| USB Serial | `Transport::Serial` 解析後誠實拒絕;無 serialport 依賴;macOS cu.* 無 stable_id | 可用 adapter(discovery/pairing/reconnect/cancel/idempotency) | **缺** |
| BLE | 同上誠實拒絕;無 btleplug;只有 system_profiler metadata | 可用 adapter(scan/connect/subscribe/restore) | **缺** |
| MQTT | 同上誠實拒絕;無 rumqttc | 可用 adapter(reconnect/QoS/重複訊息) | **缺** |
| 配對儀式 | 只有狀態機模型(ProviderState/TrustLevel),無實際 key exchange | Pairing/verification + nonce/replay 防護 | **缺** |
| 連線監督 | 無 connection supervisor;HTTP 單發 | 持久連線 + reconnect/backoff + 狀態流轉 | **缺** |
| ESP32 參考裝置 | 無韌體、無 BOM、無接線圖;僅 YAML 分類啟發 | 韌體 + BOM + Flash 步驟 + 真機閉環 | **缺** |
| 誠實階梯 | ActionStatus/VerificationVerdict 完整且測試覆蓋 | 新 transport 沿用 | **已有** |
| 裝置導向 UI | CapabilitiesHub 以 receptor/actuator 為中心 | 以裝置為中心、只發現≠已配對≠已測試≠已啟用 | **缺** |

## 3. 核心三:AI Agent 工作與對話閉環

| 能力 | 現況 | 目標 | 狀態 |
|---|---|---|---|
| 真實連接器 | Codex app-server/exec fallback + Claude stream-json,完整 lifecycle/estop/process-tree | 保留 | **已有** |
| Agent 事件→角色 | `mapRuntimeEvent` 只映射 action.*/plan.*/emergency/proactive;agent session 事件無映射 | queued/fetched/working/waiting/blocked/claimed/verified/failed/unknown/cancelled 全數映射為 Behavior Intent | **缺** |
| Session verified | AgentSessionState 止於 ClaimedCompleted;無 session 級驗證步驟 | 獨立驗證後才播 verified 演出 | **缺** |
| Resume | Runtime 支援但 API/CLI/UI 無入口 | 可達的 resume 通路 | **缺** |
| Conversation Provider | 只是 routing role + tool_scope 限縮,無獨立抽象 | 可插拔介面 + 無 Provider 時本機模板降級 | **缺** |
| Approval 對稱性 | codex app-server 可 approve;claude -p 無 approval 通道(誠實回報) | 保留誠實差異、UI 明示 | 部分 |

## 4. 一般人可理解的設定(IA)

| 能力 | 現況 | 目標 | 狀態 |
|---|---|---|---|
| 一級入口 | 一般模式 9 個(SIMPLE_NAV)+ 進階 9 頁 | 5 個:現在/小樞/工作/連接與權限/更多 | **缺** |
| 首次設定 | 7 步精靈 | 3 步 + 漸進式詢問 | **缺** |
| Activity | 側欄整頁 + 右上通知 popover 兩套並存 | 右上 Inbox 為主,不佔一級 | **缺** |
| 設定單一主人 | companion 外觀在 CompanionPage 與 SettingsPage 重複可編;initiative/quietHours 三處可改 | 每項設定唯一 canonical owner | **缺** |
| 風險分級 | Consent 以能力逐項,無 L0–L4 分級呈現 | L0 純呈現不逐次詢問…L4 短效授權 | **缺** |
| Emergency Stop | 頂欄/tray/搜尋/CLI 觸發,SafetyPage 解除 | 保留 | **已有** |

## 5. iPhone Mobile Provider

（Phase 0 快照）全部未開始:無 iOS App、無 Bonjour/QR 配對、無 TLS WebSocket provider 通道、無 motion 語意事件、無 BLE gateway。**缺**(Phase 6)。→ Phase 6 後的狀態見 §9。

## 6. 記憶與知識 UI 分層

後端 10 層記憶 + 知識圖譜完整(**已有**);一般 UI 仍暴露 Candidate/Receipt/Context Bundle 技術術語,未分「角色互動記憶/工作記憶/正式知識」三類(**缺**)。

## 7. 本輪不做(明確排除)

- 不新增治理概念、不擴 MCP(不變量)。
- 不引入 3D 遊戲引擎;物理為輕量 2D 自寫。
- iPhone 真機驗收與 ESP32 真機驗收受限於實體環境,能做多少誠實記多少;模擬器結果一律標示為模擬器。

## 8. 既有已知限制(沿承 v0.4,未修改)

CHANGELOG v0.4.0 的 10 項 closing-audit 已知限制全部仍然成立,本文件不重複列出;修掉時同步更新該處與 docs/acceptance-evidence.md。

---

## 9. 收尾狀態（Phase 7 對抗審查＋修復後，2026-08-28；§1–§6 各表的異動）

> 上方 §0–§8 是 Phase 0 的誠實基線快照，不回寫。以下記錄本輪實際變化；每一句「已有」都對應程式碼＋測試，
> 「部分」與「未做」明列缺口。逐條矩陣（463 列）在 `docs/v05-recovery-matrix.md`，測試命令與數字在
> `docs/acceptance-evidence.md` 的 v0.5 章節。

**核心一（角色）**：渲染→**已有**（執行期參數化分層 rig＋組合通道；`poseBlend` 通道讓 lie↔stand 頭部連續過渡）；
動畫數→**已有**（36 正式表情，每個都有**四段**——進入／保持／小循環／離開；Phase 2 交付時「離開」段是死資料、
0/36 四段齊全，Phase 7 讓 timeline 真的播 exit、未手寫的段落以有意義的派生段補齊並標 `derived`，測試逐幀釘死）；
Interaction Director→**已有**（`react()`/`noteFinished()` 已接進 App；等優先事件為確定性替換，Utility Scoring（`behavior.ts` `scoreEvent`）
目前沒有接進任何執行期決策——v0.5.1 對抗審查 director-pipeline-020 誠實移除了假的 utility 競爭；
quiet 分支可達；「一小時內不要主動說話」同時管住角色端氣泡與 ambient）；組合式通道→**已有**（工作／等待類真相狀態只覆蓋
核心／頭飾／裙光／耳朵通道，身體保留遊玩姿勢；安全與結果狀態整體搶佔）；個性→**已有**（`personality.ts`：安靜／自然／活潑
＋persona 派生速度、靠近距離、冷卻、變體權重、假裝沒看到、耳→視線→轉頭分段）；自主移動→**部分**（遊玩場內散步／追逐／翻面；
**跨螢幕桌面漫遊、坐視窗邊緣、從螢幕邊緣探頭、躲到視窗後未做**——需 OS 視窗層 API）；游標互動→**已有**（視窗內光點／逗貓棒；
指標進入 hit-rect 時真的看向游標；hover 短氣泡有冷卻；座標不出視窗）；玩具與物理→**已有**（**6 種**玩具含可拖曳小物件 trinket＋
輕量 2D 物理＋追逐／撲抓／帶回／拒還）；多角色→**部分**（同舞台最多 3 使魔，互相注意／打招呼／追逐，主角會回看與回愛心、
被追者會逃；多視窗多主角色未做）；命名／場景／匯入匯出→**已有**；Roll Call→**已有**；放下四種落地→**已有**（依速度／高度／落點；
速度為拖曳位移÷時間的下界估算）；氣泡／音效／拖曳／勿擾各自開關→**已有**（音效預設關）；硬體／提供者事件演出→**已有**
（device-hello／device-lost／operate-tool／ack-nod）；**對其他視窗開關／移動／下載完成／測試失敗的語意反應未做**（無感知來源）；
**Fullscreen 與 OS 勿擾偵測未做**（需新平台依賴；只交付使用者層級勿擾開關）；Reduced Motion→**已有**（執行中可切換、真靜態）；
30fps 降級→**已有**（遲滯政策）。

**核心二（硬體）**：Serial→**已有**（模擬器閉環驗收；ENOTTY 判斷收窄；佇列帶 deadline、重連即清）；MQTT→**已有**
（內嵌 broker 閉環；dedupe／重連／QoS1 有真斷言）；BLE→**部分**（adapter 完成含 disconnect／task 回收，僅 macOS/Windows 編譯；
**無真機驗證**）；配對儀式→**已有**（hello 身分＋proto＋pairing 宣告核對＋**明文**配對碼——不是 HMAC；HMAC 只在 iPhone 配對）；
連線監督→**已有**（退避重連＋重新握手；裝置端 not-paired 會觸發重握手，該次命令誠實 failed 不重送）；健康度→**已有**
（斷線 offline／未握手 degraded，不再硬編 healthy）；hello.caps 能力識別→**已有**；provider 停用／撤銷關閉連線→**已有**
（不可逆，重啟用需重載 spec）；ESP32 參考裝置→**部分**（韌體＋BOM＋接線＋Flash 文件；**已用 arduino-cli 實際編譯兩種組態**；
浮點參數規則、MQTT 非阻塞退避、BLE 有界佇列、nonce 環已修；**未經真板驗收**）；裝置導向 UI→**已有**（只發現／已配對／
已安裝設定尚未連線／已連線尚未測試／**已測試**／已啟用 六階人話＋「測試裝置」）；Observed／Verified→**設計限制**
（state facts 無 actionId，硬體動作停在 acknowledged＋deviceApplied；顯式 observed 驗證會轉 uncertain）。

**核心三（AI）**：Agent 事件→角色→**已有**（taxonomy 全映射；新增 unknown）；Session verified→**已有**（human-only 驗證路由＋
CLI＋UI，綠勾只認 verified；iPhone 綠勾也只走人類驗證路徑）；Resume→**已有**（API／UI／CLI `agents create --resume`／
`agents resume`；codex `thread/resume` 真的重新上鎖 cwd／approvalPolicy／sandbox，schema 已核）；Conversation Provider→**已有**
（介面＋本機模板降級）；誠實階梯→**已有**（程序結束無結果＝unknown 非 failed；lease 到期／重啟發 timed-out／unknown；working 不早於
fetched；fetched 只在寫入 stdin 後；approval 裁決回寫信箱、看門狗自動拒絕可見；人類讀信箱不冒充送達；SSE 初連不重播、daemon
重啟重置 cursor）；**waiting-input 無任何 connector 能產生**（Claude stream-json／Codex app-server 都沒有對應事件，保留 API 路徑）。

**IA**：5 入口→**已有**；3 步精靈→**已有**（步驟二選擇真的寫入 agent 路由；預設不靜默寫安靜時段／initiative；音效文案有程式支撐）；
右上 Inbox→**已有**（pendingCount 在截斷前計算；鍵盤／焦點陷阱）；設定單一主人→**已有**（靜態守門測試）；風險分級→**已有**
（`riskTier.ts` L0–L4 標籤＋人話＋L3 硬限制摘要；L0 純呈現不進待決定）；§11 記憶 UI→**已有**（一般模式只有「關於我的記憶／
小樞學會的知識／素材與來源」三區、規格人類文案；技術 tab 進階才顯示；角色互動記憶在小樞頁、有界、不推論人格、不進知識庫）；
淺色主題可讀→**已有**。

**iPhone**：桌面伺服器／配對／token／斷線語意→**已有**（**模擬器**驗收；撤銷即斷線、Bonjour 服務名 `_interact-ai._tcp`、
heartbeat／idle 逾時、ack 綁定裝置與 authed、facts 依 manifest 過濾、參數映射與 App 驗證一致、estop 去重且無 ack 誠實）；
安全底線→**已有**（estop 同時停 iPhone 感測；啟用中的手機麥克風出現在 activeSensors／tray／UI；mic-level 需 session consent；
extra 不能放寬 L3 硬限制；撤銷持久化失敗誠實）；iOS App→**部分**（SwiftUI 原始碼對 iOS 26.5 SDK typecheck 0 error 0 warning；
以 swiftc 編成模擬器 .app 並與真 daemon 完成配對／動器／撤銷閉環，XCTest 在模擬器 19/19；**無真機驗收**；App 端 Bonjour 瀏覽未做，
靠 QR／手動）；BLE gateway→**部分**（桌面只有 scan；App 端 connect/gatt 已寫但桌面無送端）；**Camera／Location／Live Activity／
Audio SFX／區網裝置事件未做**。

**效能實測**（可重現：`pnpm perf`＝`apps/interaction-desktop/scripts/shu/perf-rig.mjs`；headless Chromium，Apple M2 Pro，
含 raster flush）：見 `docs/acceptance-evidence.md` v0.5 章節的最新一次輸出。Phase 6 文件裡的「drawRig 0.452 ms/幀（2.2x）」
沒有產生程式，已作廢。

---

## 10. Phase 8 收尾狀態（Character Presentation Protocol＋一般模式產品化，2026-09-02）

> 延續 §9 的寫法：每句「已有」都對應程式碼＋測試；「部分」與「缺」明列缺口。證據等級一律標示——
> **單元／整合測試**（cargo test、vitest）、**CLI E2E**（真 daemon＋WebSocket fixture）、**瀏覽器**（jsdom／Playwright，
> Playwright 本輪未跑）、**模擬器**；ESP32 與 iPhone **真機驗收仍為零**。

**協定核心**：CPP v1.0 規格→**已有**（`docs/character-protocol/README.md` 唯一契約）；Rust crate→**已有**
（`crates/interaction-character`：capability／gateway／input／intent／lifecycle／manifest／receipt／schema／wire；
golden `schemas/character-protocol.schema.json` 由 Rust 產生）；TS 鏡射→**已有**（`apps/interaction-desktop/src/character/`
protocol／manifest／negotiate／gateway／registry；fixture 由 Rust 驗證器測試）；§13 測試矩陣→**已有**（manifest 驗證、版本協商、
能力協商與 fallback、命令生命週期、acknowledged→uncertain、cancel 冪等、重複 messageId、過期、世代、崩潰、有界佇列、
惡意 manifest／路徑穿越、偽造 verified、緊急優先、reduced-motion、legacy pack 遷移）。

**Runtime 接線**：`interaction-runtime::character`（CharacterHub）→**已有**（hello／receipt／event／instances／manifest／adapters／
manual intent；truth projection 表 §11 全映射，含 AI `companion.state.present` 只能落在無 floor 的 intent；節流：receptor
observation 2 s、dragged 1/s、hover 30 s；presence heartbeat 逾期→pending 全 uncertain、generation+1）；事件→**已有**
（`character.intent`／`character.receipt`／`character.instance`／`character.system-text`）；HTTP／CLI→**已有**
（`/v1/character/*`、`interact-ai character status|instances|manifest|adapters|intent`）；storage v8 adapter 紀錄→**已有**。

**Transports**：In-process TS adapter→**已有**；Runtime↔桌面視窗（SSE／Tauri IPC＋human token 回執）→**已有**；
WebSocket `GET /v1/character/ws?token=`＋adapter token 分權（403／401 有測試）→**已有**（CLI E2E 以
`examples/character-adapters/text-adapter.mjs` 真連線：register→connect→negotiated→intent→receipt→revoke 即斷線）；
**stdio JSON Lines→缺**（只有規格：host 不 spawn 子程序、無 stdio fixture）；README §8.1「新增 transport」一節
（Phase 9 已改寫）→**已有**：老文字要求實作不存在的 `CharacterTransport`（`interaction-character::transport`），
已改成如實描述唯一的具體 transport（`interaction-api::character_ws`＋`interaction-runtime::character::WsSession`），
不再宣稱有可插拔 trait。

**Reference adapters**：小樞 `shu-rig`→**已有**（`src/character/adapters/shu.ts`：manifest 協商、intent→36 表情計畫→mixer、
回執 accepted→started→completed／cancelled{preempted|replaced|cancel}、reduced-motion 協商、gameplay／familiars／scene／rollCall
保留）；`sprite`（v1／v2 pack 相容層）→**已有**；`text`（純文字最小角色＝可信 fallback）→**已有**；角色視窗載入失敗／崩潰→文字
＋「角色載入失敗，改用文字顯示」→**已有**（瀏覽器單元測試：`companion-gateway-wiring.test.ts` 釘住
`CHARACTER_LOAD_FAILED_LINE` 且不來自 adapter manifest；「現在」頁在 Runtime 離線時顯示的是另一句
「角色離線，改用文字。」——兩者是刻意不同的固定文案，見 DESKTOP-GUIDE.md）。

**匯入**：`character_import`／`character_list_imported`／`character_asset`／`character_remove`（Tauri host，無 runtime 亦可）→**已有**
（只收 in-process builtin entrypoint；內建 id 不可覆蓋；資產 magic bytes／bytes／sha256 核對；單檔 ≤ manifest 上限（≤32 MB）、
總量 32 MB；不安全 id 拒絕；錯誤不回顯路徑；原子替換）；角色頁「更換或加入角色」卡片＋匯入對話框→**已有**；
**匯入角色在角色視窗實際演出→已有（單元／模擬器證據）**（`CompanionApp` 經 `characterListImported` 取得 manifest 與資產 data URL，
文字／sprite／shu-rig 三種 builtin 皆可建 adapter；失敗退回文字＋固定文案；**尚未在真 Tauri 視窗以真實匯入資料夾驗收**）；
角色偏好／`variant` 轉給 `adapter.reconfigure`→**已有**；host 端 `companion_preferences`（每角色 ≤32 鍵、布林／數字／字串）／
runtime `first_success_seen` 持久化→**已有**（Rust 驗證＋測試；`human_layer.rs::first_success_seen_persists_through_ui_preferences`、
`api_e2e.rs` PATCH／GET `/v1/ui/preferences`，前端 `FirstSuccess.tsx` 先送 host、host 未回旗標才退 localStorage 並誠實回報「本機」）

**可信 host 覆蓋視窗**：Tauri 第三視窗「安全狀態」（label `overlay`，只允許 listen；`host-safety` 事件只發給它；340×200 右上角、
透明、不搶焦點、不擋滑鼠；緊急停止／Runtime 離線／麥克風／攝影機／其他感測；有事才建立、清除即關閉；不受關閉行為與角色
prefs 影響）→**已有**（Rust `host_safety::HostSafetyView::derive`＋TS `isOverlayActive` 鏡射有測試；**Tauri 視窗本身不在
Playwright 覆蓋範圍**，瀏覽器模式以 `?window=overlay` 驗證 DOM）；tray 與覆蓋視窗共用同一 `HostSafetyView`→**已有**。

**狀態投影**：`status().characterProtocol{version, instances, activeCharacter}`→**已有**；桌面 `useCharacterName()` 單一名稱來源
（prefs.companionName > manifest displayName > 角色；pronoun 缺省中性）→**已有**；角色生命週期人話（角色視窗運作中／已隱藏／準備中／
角色視窗未連線／角色目前無法顯示，改用文字）→**已有**（在 `CompanionPage.characterLiveState`，尚未併入 `statusProjection.ts`）；
uncertain／unknown 一律「結果不確定」→**已有**。

**五個入口（一般模式）**：現在＝三個答案（角色現在怎麼樣／正在做什麼／有什麼需要處理）＋快速操作（交代一件事→工作預填、暫停／恢復、
加入裝置）＋「詳細狀態」折疊→**已有**；角色（側欄顯示目前角色名）＝目前角色／外觀與名字／平常如何陪伴／安靜與勿擾／主動式對話／
主動程度與安靜時段／更換或加入角色，技術資料只在進階→**已有**；工作＝任務優先 composer（想讓〔名〕幫你做什麼？＋加入檔案或選擇
資料夾＋這是哪一種工作）＋開始前預覽六項＋工作設定折疊，一般模式無 JSON→**已有**；**資料夾選擇器→缺**（src-tauri 未註冊 dialog
plugin，按鈕誠實回報「這個版本沒有資料夾選擇器」，路徑欄可用）；連接與權限＝可以看見／可以回應／使用的裝置／需要你確認四區＋
展開的「全部能力與裝置」＋「角色如何接上系統」adapter 列（來源／位置／可執行／網路／可以接收／已測試／撤銷）→**已有**；
**外部 adapter 列的「可以接收」與 executable／network→已有**（`/v1/character/instances`／`adapters` 皆由 manifest 回報
`author`／`version`／`inputCapabilities`／`executable`／`network`，從未連線的 adapter 也顯示；前端只在連到不回報這些欄位
的舊 daemon 時才顯示「未回報」；釘住：`character_loop.rs` 的 `adapter_tokens_are_hashed_persisted_and_revocable`／
`external_adapter_transport_attaches_negotiates_and_times_out`、`api_e2e.rs` 對應 HTTP 斷言）；更多＝記憶與知識／活動歷史／設定／角色與整合管理／
進階功能（「顯示進階功能」唯一主人）→**已有**；右上 Inbox→**已有**（沿 §9）。

**首次成功體驗**：3 步精靈後「〔名〕準備好了。要不要先試一次？」五選項（提醒我休息＝純本機 plan 路徑、收據狀態投影不冒充送達；
交代一件小工作＝預填工作頁；先在桌面陪我；更換角色；先不用）→**已有**（瀏覽器單元測試）；只看一次的旗標持久化→**已有**（runtime `UiPreferences.first_success_seen`，同上；host 未回旗標時前端才退
localStorage，畫面誠實標示為估算）。

**殘留術語與相容妥協**（文件化，不算缺陷）：`companion_provider_display_name()` 含「（Presentation）」（e2e 釘死）；
providers.rs 握手 note 仍含「受器」；GlobalSearch「裝置／Provider：」、ActivityPage 篩選「Domain」、「續租 30 分鐘」／
「工作階段已關閉（已要求終止子程序）。」受既有測試釘住；「全部能力與裝置」預設展開而非獨立子頁（相容 compat-routes 與 e2e）。

**證據等級總結**：Rust／vitest 單元與整合、CLI E2E（WebSocket fixture）＝本輪實跑；Playwright（app／narrow／evidence spec 已改）
＝**未跑**；Tauri 覆蓋視窗＝Rust 單元＋瀏覽器 DOM，無真視窗自動化；ESP32／iPhone＝**無真機**；Phase 8 截圖尚未產生
（`docs/assets/v05-evidence/` 目前是 Phase 7 證據跑的畫面）。

**Phase 8 開放問題（Phase 9 更新：(3)(4)(6) 已解決，見 §11）**：(1) 匯入角色路徑只有單元／模擬器證據，未在真 Tauri
視窗驗收；(2) stdio transport 無 host spawn／fixture；(3) ~~README §8.1 `CharacterTransport` 與 crate 不一致~~ ——
Phase 9 已改寫成如實描述（見上）；(4) ~~instances／adapters API 缺 `inputCapabilities`／author／version~~ —— Phase 9
複查發現程式碼早已回報，只是本文件與 DESKTOP-GUIDE 沒跟上，已改；(5) 資料夾選擇器——Phase 9 已加
`tauri-plugin-dialog`（僅 main 視窗），見 §11；(6) ~~host 持久化 `companion_preferences`／`first_success_seen`~~ —— 兩者
在 Phase 8 當下就已實作，本文件 §10 自相矛盾（一處寫已有、一處寫部分），Phase 9 已訂正；(7) `characterLiveState`
移入 `statusProjection.ts`——仍未做；(8) Tauri 覆蓋視窗無 Playwright 覆蓋——仍未做；(9) 真機驗收（ESP32／iPhone）
仍為零。

---

## 11. Phase 9 收尾狀態（發布硬化，2026-09-03）

> 延續 §9／§10 的寫法：每句「已有」都對應程式碼＋測試；「部分」與「缺」明列缺口。本輪是對 v0.5 對抗審查
> 第三輪（`2e02284-20260902T142608Z`，13 維度、136 findings→44 findings 落盤）的獨立覆核與修復：44 筆 finding
> 逐一以獨立懷疑者重新驗證（against commit `521c232`）——**43 confirmed／1 already-fixed**（`docs-claims-026`，
> Phase 8 commit `d03e0b9` 已把 acceptance-evidence 的證據跑一節填實）。43 項 confirmed 全數在本分支修掉，
> 除下方明列的兩項刻意 partial。證據等級標示同 §10；測試數字見 `docs/releases/v0.5.0-test-matrix.md`。

**link-transports（9 項，全 fixed）**：MQTT 重連不再重送在飛的 QoS1 command（斷線就整個 teardown／重建
rumqttc client＋eventloop，舊 handle 上未 ack 的 publish 隨之作廢，不會遲到套用）；ack 逾時／連線重置途中失敗的
收據不再卡在 Dispatched 假裝仍在進行——`dispatched_at` 改在真正送出當下蓋章、executor 立刻結算為 `Uncertain`，
watchdog 新增 `sweep_receipts_at` 對帳掃描；`LinkError::Reset` 不再誤報 `Failed`（改 `Uncertain`，不再誘發重試）；
HTTP 宣告式動器逾時後**不再自動重送**、誠實回 uncertain（`NotSent` vs `OutcomeUnknown` 分流；同一 root cause 也
是 `safety-invariants-036`）；宣告式硬體（HTTP／Serial／MQTT／BLE）新增 `limits:` YAML 欄位，Policy Governor 的
`min()` 鏈終於拿得到裝置安全上限（`safety-invariants-037` 同修）；MQTT 健康度新增以裝置沉默時間判斷的
`LinkReadiness::Stale`（`livenessTimeoutMs` 預設 15 s）；BLE `connected()`／health 現在真的反映斷線事件；
`stop_all` 對未連線／連線中的裝置改快速失敗（不再整套 `ensure_open`），BLE 的 scan 現在是 cancel-safe（估停
被取消也真的 `stop_scan`），Runtime 的 emergency-stop 動器階段改成並行、有界 ~2 s；無 id 的裝置錯誤在多筆
命令同時在飛時不再被誤歸給某一筆（改列為無法歸屬，逾時結算為 `Uncertain` 而非 `device-refused`）；Serial
reader 的殘段在 read 逾時後保留（有 16 KiB 上限），不再整段丟棄；YAML 明文 `pairingCode`／MQTT password 現在
build 時會 `warn!` 並記錄在 `BuiltCapabilities.warnings`（不阻擋，向後相容）。

**safety-invariants（5 項，全 fixed）**：「停止所有感測」／緊急停止現在真的送達 iPhone 並等待（≤2 s）確認
（`ack{stopAll:true}` 或後續 `status{micLevel:false}`），本機麥克風＋每台手機逐一誠實回報 stopped／unknown／
unreachable，不再只看 Runtime 自己的本機狀態就宣稱「已停止所有感測」；iPhone 麥克風狀態改成**真的變化**才發
`sensor.started`／`sensor.stopped`（不再靠 30 s 心跳洗版，也不再讓 SSE 訂閱者永遠看不到）；HTTP 宣告式動器
逾時誠實化（同 link-transports）；裝置安全上限終於能宣告並到達 Policy `min()`（同 link-transports）；agent
token 不再能經 `/v1/providers` 讀到 iPhone 的公開身分指紋（改用 `sha256("mobile-identity:v1:…")` 衍生指紋，且
非人類 principal 的回應一律抹掉 `identity.fingerprint`）。

**mobile-server（9 項，全 fixed——`F-043` 已於第二輪對抗審查覆核確認修復，見 §12）**：`emergency` 與
`verified-success` 兩個真相狀態收回 Runtime 專屬
（`RUNTIME_ONLY_STATES`）——AI 再也不能經 `character.present` plan 讓 iPhone 顯示「緊急停止中」，而 Runtime
自己的 estop／解除會並行投影到每台已連線手機（≤1.5 s 有界等待，逐台 acknowledged／refused／unknown／
unreachable）；DoS 面全部補上並在 `mobile_status` 誠實顯示——連線數上限 8、TLS／WebSocket 交握 5 s 逾時、
未認證連線 10 s 死線、單則訊息 128 KiB 上限、每連線 30 msg/s 速率限制；accept loop 對暫時性錯誤改退避重試而
非直接關站，真的致命時誠實回報 `started:false` 並清掉 Bonjour 廣播（而不是假裝仍在運作）；多台 iPhone 同時
連線時動作必須指定 `deviceId`（不再挑 BTreeMap 第一台），收據記錄實際送達的裝置；TLS 私鑰改成原子式 0600
落地＋載入時權限檢查修復（不再有先 0644 再 chmod 的空窗）；`pending_acts` 在等待端 future 被丟棄或手機斷線
時保證清空（不再永久殘留）；伺服器端動作參數驗證現在對齊 iOS App 的驗證規則（haptic／notify／tts／torch／
flash／character.present）；`mobile_present_verified` 在手機回非 `ack` 時誠實回 `Err`（不再吞成 `Ok`）。
**`F-043`（多台 iPhone「已測試」證據歸屬）——第一輪誤留為 partial，第二輪對抗審查
（`F-c3d1786-20260903T124638Z-docs-claims-067`）核對程式碼後確認已在 HEAD 修好**：
`executor.rs::note_capability_tested` 取的是 driver 收據的 `deviceId` 呼叫 `note_capability_tested_on`，
不是「字典序第一個」provider；回歸測試 `mobile_loop.rs::tested_evidence_lands_on_the_phone_that_actually_ran_the_action`。

**character-protocol（4 項，全 fixed）**：Reduced Motion 改成每個 instance 各自協商（不再是 Rust Gateway 永遠
以 `false` 協商），回執 resolution 只能降級不能升級（adapter 可誠實降級為 `reduced`，不能自己升回 `exact`）；
TS Gateway 的 merge 分支不再謊報 `completed{merged}`（改誠實 `cancelled{reason:"merged"}`），duplicate／
alreadyTerminal 回執改帶原命令協商出的真實 resolution（不再硬編 `exact`）；50 則/s 限流改成每個 instance 一份
真正共用的預算（HTTP 回執／事件與 WS 畸形訊息都算在內，稽核有界：每 5 秒最多一列＋`suppressed` 計數）；純
聲音／燈光角色現在只要宣告了 `audio.*`／`haptic.*`，就能真的用聲音或燈表達工作／思考／等待／未知／取消
（之前 work/think 會被誤判 `unsupported`、wait/unknown 會被誤判走 `system.text`）。附帶新增第三方 manifest
conformance 測試套件（`conformance.rs`）與斷線時安全 intent 交接給 `system.text` 的修復。

**ia-settings（7 項，6 fixed／1 partial）**：首次設定精靈「進一步自訂」不再寫入沒有主人的
`channelLimits["*"]`／`requireApprovalAt`（一般模式移除這兩個孤兒控制項，只保留安靜時段）；重新執行精靈不再
靜默停用使用者已啟用、但非新手安全的能力（例如已配對 iPhone 的動作感測）；精靈新增**套用前確認 diff**——
新的 `POST /v1/onboarding/preview` 零副作用試算端點，完成設定前先列出真的會變動的項目；收件匣 `pendingCount`
改為直接依狀態查詢開放收據（不再只看最近 200 筆歷史視窗），新增 `pendingCountExact` 誠實旗標（撈滿上限時
明說「至少」而非「總共」）；窄視窗「更多」選單現在會正確高亮目前所在的子項；解除緊急停止對話框改三段式
（會恢復可用／不會自動恢復／你先前已停用因此仍為停用），不再把使用者自己停用的動器列為「解除後會恢復」；
風險分級修正本機音效／語音朗讀從 L2「會用到你的檔案、偏好或記憶」降為正確的 L1。**Partial（`ia-settings-012`）**：
角色頁的安靜時段編輯器已送出明確的靜音通道清單（不含桌面角色，L0 呈現不再被安靜時段誤判為「待你決定」）；
**首次設定精靈那一側未修**——`Onboarding.tsx` 仍寫 `silencedChannels: []`，從精靈建立的安靜時段預設仍會靜音
桌面角色；Rust 根因（`activity.rs` 對 L0 呈現動器的 blocked 收據一律回 `needs_decision:true`）也已在
`receipt_item()` 修正為排除純呈現動器，僅 wizard 那一條路徑殘留。**（v0.5.1 已修，commit `f3a9eea`：
抽出 `src/quietHours.ts` 作為唯一 canonical builder，精靈與角色頁共用；見 §13。）**

**agent-honesty（1 項，fixed）**：緊急停止不再把「cancel」包裝成新的 user turn 送進每個 gateway agent（不再
觸發模型呼叫、不再消耗 message 預算、不再誤發 `fetched` taxonomy 事件）；改為直接關閉 session 並在事後補一則
不送達 agent 的 runtime 稽核訊息；所有 session 改為並行終止（每個 ≤2 s 有界），不再序列化等到 5 s×N。

**IA／前端結構性改動（非對抗審查 finding，任務要求）**：工作頁「開始前預覽」從固定六項收斂為**三個回答**
（這次會讀取什麼／會不會修改內容／最多使用多少時間與費用）＋收合的「查看技術細節」；原生資料夾選擇器
（`tauri-plugin-dialog`，僅 main 視窗；瀏覽器版誠實顯示不支援，不再是假按鈕）；換資料夾會使先前的寫入確認
作廢；連接與權限第一層改為**裝置優先五區**（已連接的裝置／系統可以看見什麼／系統可以做什麼／目前需要
確認的權限／立即停止與撤銷），iPhone 裝置卡片的「停止感測」「測試連接」改用新的每機端點且用字遵守誠實
階梯；角色頁一般模式只顯示內建／第三方、可接收、已測試三個徽章，額外授權只給一句人話，外部／可執行程式／
需要網路與執行位置欄位移到進階模式；一般模式匯入角色改成只有選檔（無貼上原文的輸入框），驗證器原文收在
收合的問題明細裡。

**新增／變更的對外契約（節選；完整清單見 `docs/releases/v0.5.0-migration.md`）**：`POST /v1/onboarding/preview`；
`POST /v1/sensors/stop` 回應改為 `{stopped, uncertain, local:{microphone}, devices:[...]}`；每機
`POST /v1/mobile/devices/{id}/sensors/stop`／`/test`；`POST /v1/emergency-stop` 的 payload／事件新增
`sensors`（`StopAllSensorsReport`）與 `characterEmergency`（`[{deviceId, outcome}]`）；`activity_inbox` 新增
`pendingCountExact`；`GET /v1/providers`／`/v1/providers/{id}` 對非人類 principal 省略 `identity.fingerprint`；
declarative YAML 新增可選 `limits:`（`CapabilitySpec`）與 `mqtt.livenessTimeoutMs`；新環境變數
`INTERACT_AI_MOBILE_ADVERTISE`（`0`／`false`／`off`／`no` 關閉 Bonjour 廣播並只綁 `127.0.0.1`，供 E2E 使用）；
CharacterHelloInput／CharacterHelloBody 新增 `reducedMotion`。

**證據等級總結（第一輪撰寫時；已由第二輪覆核訂正，見 §12）**：Rust workspace、vitest、Tauri、CLI E2E 全部
本輪實跑；所有 MQTT／BLE 相關測試一律**模擬器或程序內 fixture**（嵌入式 rumqttd broker、fake BLE event
stream）；ESP32 韌體兩種組態的 `arduino-cli` 編譯本輪重跑通過（非真板，仍未改變）。**iPhone 真機狀態已由
第二輪修訂**：撰寫本節當下 iPhone 11 只完成 USB 配對，Developer Mode 未開啟、本機無 Apple 簽章身分；
2026-09-03 稍後完成 Developer Mode／簽章身分設定並跑完真機驗收矩陣的大多數列（配對、動器、緊急停止投影＋
停感測、撤銷等），**不再是「真機驗收仍為零」**——完整逐列證據與尚未涵蓋的列見
`docs/releases/v0.5.0-iphone-device-evidence.md` 與下方 §12。Playwright 全量本輪由整合階段最終跑一次，
數字見 `docs/releases/v0.5.0-test-matrix.md`。

**Phase 9 第一輪已知限制（撰寫時；已知限制的最新狀態見 `docs/releases/v0.5.0-known-limitations.md`）**：
`ia-settings-012` 精靈半邊未修（見上，第二輪覆核仍為真，保留）；MQTT 重連不重送只在內嵌 broker 驗證，無
真實 ESP32 board 上的重送測試；BLE 斷線偵測只有假事件流的單元測試，零真實周邊驗證；HTTP 逾時分類保守地把
「TLS 握手失敗」等未知錯誤也歸類成 `OutcomeUnknown`（方向安全，但可能把真的沒送出的請求也標成不確定）；
原生資料夾選擇器沒有任何自動化驗收（vitest mock、Playwright 走瀏覽器版，需桌面手動驗收）**（v0.5.1 已在
真 Tauri 視窗完成手動驗收四列——取消／選擇／唯讀不取得 write scope／寫入二次確認；**仍無自動化**，見
§13）**；iOS 新增 Xcode 專案＋裝置腳本。**以下三項第一輪誤留為限制、第二輪對抗審查核對程式碼後確認已修**（見
`known-limitations.md` §5）：`F-043` executor 已測試證據歸屬、`credential_warnings()` 未轉發進 provider
紀錄、`mobile_ble_scan` 沒有 `deviceId` 參數。

---

## 12. Phase 9 第二輪收尾狀態（對抗審查第二輪修復＋iPhone 真機部分驗收，2026-09-03）

> 延續 §9／§10／§11 的寫法：每句「已有」都對應程式碼＋測試；「部分」與「缺」明列缺口。本輪是對第一輪
> Phase 9 收尾 commit（`521c232`）的第二輪對抗審查（`c3d1786-20260903T124638Z`，find＝opus、verify＝
> sonnet）：**78 reviewed／74 confirmed／4 refuted**。74 項 confirmed 中 **63 fixed、4 partially-fixed**
> （根因已定位、範圍已明確劃定），另有 7 項 `docs-claims`（文件與程式碼不符）直接訂正在文件本身（本節與
> `CHANGELOG.md`／`docs/releases/v0.5.0-*.md`）。完整 finding 清單見
> `docs/reviews/adversarial/c3d1786-20260903T124638Z.{md,json}`；逐項 fix summary 與回歸測試見
> `CHANGELOG.md`「對抗審查第二輪」小節與 `docs/releases/v0.5.0-known-limitations.md`。

**修復範圍（依維度）**：memory-ui 6 項全 fixed（角色視窗記憶單一寫入者、context bundle 誠實回報截斷、
export 誠實聲明範圍、agent 記憶重新確認天數對齊 Governor、按鈕失敗可見、一般模式記憶分類人話化）；
mobile-server／provider 生命週期 7 項全 fixed（estop 中連線的手機真的被要求停止感測、宣告式裝置停用／撤銷
跨重啟持久、BLE scan 尊重 estop、其他串流手機不再無聲消失、撤銷立即結束在途動作、觀察 `at` 時間戳併入
facts、認證失敗留 audit）；agent-honesty／SSE 邊界 6 項（5 fixed／1 partial：SSE 已對齊，interrupt 擁有權
未修**（v0.5.1 已修，commit `af8d1f1`：legacy 共享 agent token 一律 403；見 §13）**）；ia-settings／前端 IA 10 項全 fixed（導覽解除流程自動對焦、收件匣裝置 id／人話標題／感測標題、
GlobalSearch 標籤、精靈套用失敗誠實回報、重新驗證與工作階段按鈕錯誤可見、L4 同意對話框依風險分級）；角色
rig／perf／Director 17 項全 fixed（跨姿勢交叉淡出連續化、快速連點不再清掉待確認狀態、舞台死區消除、
Reduced Motion 使魔真收斂與真靜態、玩具滿額誠實拒絕、真正的使魔互相打招呼、pacing 降級、soak 涵蓋範圍
擴大、死程式碼移除、連續戳弄變體池、host 端眨眼判斷去除硬編表情 id、Utility Scoring 誠實記錄範圍、
`reportHitRect` 暫停時不再心跳）；Character Presentation Protocol 8 項全 fixed（含 1 項 blocker：安全
intent 每種非 completed 終態都會 fallback 到 `system.text`；重新協商不再讓在飛安全提示消失；TS intent→
capability 表改 golden 雙邊斷言；resolution 不再被樂觀改寫；AI 呈現命令只由桌面 instance 結算；外部
adapter 不再能合成人類互動觀察；緊急停止期間連線／協商立即收到投影）；link-transports／
protocol-conformance 13 項（11 fixed／2 partial：宣告式配對驗證與 serial fallback 讀取執行緒留為已知限制
**（兩項皆 v0.5.1 已修，commit `ddbab07`／`4029316`；serial 的修復只涵蓋 unix，非 unix 仍是
bounded-join＋detach＋計數。見 §13）**）。

**iPhone Mobile Provider — 真機部分驗收（不再是「真機驗收仍為零」）**：iPhone 11（`iPhone12,1`，
iOS 26.3.1）2026-09-03 完成 Developer Mode 開啟、Xcode 簽章身分設定、USB 安裝與啟動，對真 daemon（區網
Wi-Fi TLS WebSocket，非 loopback）跑完 18 列驗收矩陣中的大多數列——**已驗證（真機）**：配對（QR／HMAC／
每機 token）、首次連線權限誠實顯示未授權、haptic／notify／tts／torch／flash 動器 acknowledged、角色六態
acknowledged、AI 偽造 `emergency`／`verified-success` 被 runtime 擋下（receipt failed，從未 dispatched）、
背景／鎖定行為（App 切背景後 daemon 偵測斷線並強制停用高風險受器）、撤銷離線裝置、觀察 battery／touch／
麥克風音量（activeSensors 反映、查詢為空是設計非缺陷）、BLE 閘道 scan（8 秒內回傳 10+ 個周邊）、停止所有
感測（使用者路徑，313 ms 內確認）、緊急停止（178 ms 內確認停感測＋角色投影）、解除緊急停止不自動恢復。
**尚未涵蓋**：observe-motion（需使用者搖手機）、BLE connect／GATT read/write/subscribe（無測試用
peripheral）、系統終止 App 後的冷啟動恢復（實測需按「連線」或 `--auto-connect`）**（v0.5.1 已在 App 端加上
冷啟動自動重連，commit `5111208`；驗證等級僅 **iPhone 17 模擬器 XCTest**，真機驗證 blocked，不得寫成
已在真機驗收。見 §13 與 `docs/releases/v0.5.1-iphone-device-evidence.md`）**。**真機測試額外發現
三個真機限定的限制**：桌面 Wi-Fi IP 變更後 App 沒有 Bonjour 探索、host 釘死在配對當下，換 IP 必須重新配對
（daemon 端 0 次連線嘗試，非 bug）；App 進背景會被 iOS 收回 WebSocket（平台限制，非缺陷）；
`device-acceptance.sh` 原本會卡在「沒有 active session／iPhone 動器預設 disabled／policy allowlist 未含
iphone.\*」三道前置關卡（Governor 正確運作），新增 `--grant-consent` 後會一併打開這三道關卡跑完整矩陣。
完整逐列輸出見 `docs/releases/v0.5.0-iphone-device-evidence.md`。

**iOS XCTest**：`docs-claims-070` 核對後訂正為 **25/25**（MotionClassifier 8＋ProtocolTests 17，其中 4 個是
先前未記錄執行過的 stop-all 緊急狀態誠實性 async 測試：`testEmergencyStopAllSetsTheCharacterStateEvenIfCharacterPresentIsLost`／
`testUserStopAllDoesNotFakeAnEmergencyCharacterState`／`testActuatorOnlyStopAllTouchesNeitherSensorsNorCharacterState`／
`testOnlyTheRuntimeClearsTheEmergencyCharacterState`），在 iPhone 17 模擬器透過 simctl 注入執行，仍是**模擬器**
（與 iPhone 真機驗收是兩件事）。

**測試套件**：`docs-claims-071` 核對後補上文件——Playwright user-task 套件 12 spec／65 test（8 個新增：
`a11y`／`agent-not-installed`／`character`／`estop`／`home-state`／`iphone`／`sensors`／`work-delegate`）＋
三個有序 project（`first-run`→`main`→`estop-last`）；`examples/fake_iphone.rs` 程序外**模擬 iPhone
（fixture）**，供 `iphone.spec.ts` 等測試重現手機連線狀態，不是真機證據。完整測試矩陣見
`docs/releases/v0.5.0-test-matrix.md`。

**前端結構性文案訂正（`docs-claims-072`）**：README／FEATURES 先前描述的精靈步驟名（「認識小樞／要讓小樞
幫忙工作嗎／安全預設」）與五入口第二項固定寫「小樞」，與程式碼（`Onboarding.tsx` 的 `STEPS =
["選擇角色與陪伴方式","選擇 AI 工作方式","確認安全與權限預設"]`；`routing.ts`（v0.6.x 前在 `App.tsx`）的 `SIMPLE_NAV` 第二項在執行期
換成目前角色名稱，預設小樞）不符，已訂正。

**證據等級總結（第二輪）**：Rust／vitest 單元與整合＝各工程 agent 局部實跑（詳見各自 stage-3 報告的
`testCommandsRun`；整合階段最終一次全量跑：Rust 736 passed / 0 failed / 0 ignored（63 targets）、Tauri 46/0、CLI E2E 82/0、Playwright 65/0，見 `docs/releases/v0.5.0-test-matrix.md`）；
前端 typecheck／vitest（988 passed / 0 failed，49 檔）／build＝本輪最終一次全量跑；ESP32 韌體編譯兩組態
2026-09-03 最終覆核跑 exit 0（非真板）；iPhone＝**真機部分驗收**（見上）；BLE／MQTT 仍為模擬器或程序內
fixture（無真實周邊／broker）。

**Phase 9 第二輪已知限制（完整清單見 `docs/releases/v0.5.0-known-limitations.md`；逐項的 v0.5.1 重新分類見
`docs/releases/v0.5.1-known-limitations.md` §1）**：4 項 partial
（`safety-invariants-078` interrupt 擁有權未修、`companion-gameplay-032` 單一 hit-rect IPC 未修、
`protocol-conformance-030` providers.rs 未依 pairing_unverified 降級、`link-transports-054` reader thread
洩漏已計數未消除）；`safety-invariants-074` 的 `provider-off` 標記在升級邊界有一次性缺口；
`rig-renderer-056` 最差單幀跳動降到 4.48 px（非 0）；`agent-honesty-022` resume workdir 未持久化到
`AgentSessionRecord`；`memory-ui-003` 匯出仍只涵蓋記憶項目；`ia-settings-018` 精靈 commit 仍非原子；
`safety-invariants-075`「只這一次」目前是 5 分鐘 TTL，非真正單次；`character-protocol-043`／
`safety-invariants-077` 外部 adapter 輸入已完全移除（比原本更嚴格，尚無安全的新管道）；
`interaction-api` 的 WebSocket 限流測試在機器負載高時會 flake（既有脆弱性，本輪未改動限流程式碼）。

---

## 13. v0.5.1 修補（產品完成度、一般模式易用性、誠實狀態與剩餘技術債，2026-09-04）

> 延續 §9–§12 的寫法：每句「已有」都對應程式碼＋測試；「部分」與「缺」明列缺口。基準是 v0.5.0
> （tag `v0.5.0` ＝ `8b713c7`），分支 `release/v0.5.1-product-hardening` 共 30 個修復 commit（20 個規格修復＋10 個對抗審查修復）。
> 每一項修復都附「舊行為下先紅燈」的回歸測試。測試數字見 `docs/releases/v0.5.1-test-matrix.md`；
> 已知限制見 `docs/releases/v0.5.1-known-limitations.md`；20 道發布關卡見
> `docs/releases/v0.5.1-release-readiness.md`。對抗審查見 §13.5。

### 13.1 §11／§12 五個刻意保留的 partial：現況

| finding | v0.5.0 狀態 | v0.5.1 現況 | 缺口 |
|---|---|---|---|
| `ia-settings-012` | 部分（精靈那一側未修） | **已有**（`f3a9eea`）：`src/quietHours.ts` 是唯一 canonical builder（`QUIET_SILENCED_CHANNELS`＋`buildQuietHoursPatch`），精靈與角色頁共用；一般安靜時段保留 L0 呈現，只有明確關閉角色顯示才隱藏 | 無 |
| `safety-invariants-078` | 部分（SSE 已對齊，interrupt 擁有權未修） | **已有**（`af8d1f1`）：`/interrupt` 對 legacy 共享 agent token 一律 `403 token_scope_forbidden`；session-scoped token 只能中斷自己的 session（middleware＋handler `interrupt_principal_allowed` 雙層）；human token 保留管理能力；runtime 緊急停止走內部呼叫不受影響 | **Breaking**：以 `state/api-agent-token` 中斷 session 的 connector 需改用 `INTERACT_AI_SESSION_TOKEN` 或人類平面 |
| `companion-gameplay-032` | 部分（死區已消除，單一 hit-rect IPC 未拆） | **已有**（`73f0694`）：`companion_hit_regions` IPC 收多個 bounded regions（角色本體、每隻使魔、每個可抓玩具、每個真正互動的 UI），游標只在落於某個 region 時才攔截；≤16 個、逐一 clamp、單框不得兩軸皆 ≥80%、總面積 ≤80%、空清單／非有限值整份拒絕並沿用上一份（fail-closed）、每 45 ms 最多接受一次；`companion_hit_rect` 保留為相容 shim。**真 Tauri 視窗**驗過空白穿透與角色本體攔截 | 快捷選單／玩具／使魔未在真視窗驗收 |
| `protocol-conformance-030` | 部分（host 已標示，runtime 證據等級未降級） | **已有**（`ddbab07`）：`ProviderTested.pairingUnverified`（serde default）從 driver 收據一路投影到 provider 證據、`GET /v1/providers/{id}`、CLI JSON 與桌面六階階梯（「已測試」「已啟用」降為警告色「（配對碼未驗證）」）；`tested_note` 改說「配對碼未經比對，身分證據僅為裝置自報的 deviceId」 | 受器讀取路徑拿不到旗標（傳 `None`，不會洗白也不會主動標示） |
| `link-transports-054` | 部分（已計數，根因未消除） | **已有（unix）**（`4029316`）：fallback reader 改為獨立 `O_RDONLY\|O_NONBLOCK` fd＋`poll(2)`＋per-session self-pipe；shutdown 在 `READER_JOIN_GRACE_MS` 內真的 join 回 reader；EAGAIN／EINTR 回到 poll 不 busy loop。50 圈 open/shutdown soak 零 detached、最差關閉 203 ms | **非 unix 平台**仍是 bounded-join＋detach＋計數；`detached_reader_threads()` 保留為警報器 |

### 13.2 一般模式與誠實狀態的缺口：現況

| 缺口（v0.5.0 已知限制編號） | v0.5.1 現況 | 缺口 |
|---|---|---|
| #14 角色生命週期人話未集中 | **已有**（`e11c0d9`）：`projectCharacterLifecycle()` 併入 `statusProjection.ts`，以 `satisfies Record<AdapterLifecycleState, …>` 窮舉表取代三個 Set，未知原始值退回中立「準備中」不外洩 enum | 無 |
| #18 resume workdir 未持久化 | **已有**（`c8e78c6`）：`AgentSessionRecord.resolvedWorkdir`（canonicalize，serde default）；續開比對資料夾，不相等／未帶／舊記錄一律 `PolicyBlocked`；CLI `agents resume --max-cost/--max-messages` 省略時帶入原 session 實際上限 | **Breaking**：升級前的 gateway session 不可續開 |
| #19 首次設定 commit 非原子 | **已有**（`69a8ca3`）：兩段式提交（Phase 1 純驗證／Phase 2a 檔案 temp+rename＋SQLite 單一交易並在失敗時回寫舊 bytes／Phase 2b 記憶體旗標只在 2a 後翻、失敗全翻回並寫 `onboarding.partial` audit）；五條故障注入路徑各有測試 | 檔案寫入與 SQLite 提交之間的程序崩潰**沒有 journal** |
| #20 「只這一次」其實是 TTL | **部分**（`0c845e0`）：`Consent.maxUses`／`remainingUses`（serde default），executor 在授權臨界區內原子消耗一次並在同一把鎖內持久化；並行兩個 plan 只有一個通過、dispatch 失敗不歸還、重啟後仍是用掉的狀態、`maxUses=0` 拒絕 | **受器與 tool-operation 的授權仍是短效 TTL** |
| #21 記憶匯出 `limitReached` 不精確 | **部分**（`0551b9f`）：`Store::count_memory(layer)`，以真實筆數判斷截斷（剛好 1 000 筆不再誤報）；回應新增 `total`／`included`／精確的 `notIncluded`；備份頁改為轉述後端回報的範圍，文案不再稱「備份」 | **匯出範圍仍只有記憶項目**（方案 B，刻意） |
| #23 一般模式技術分層外洩 | **已有**（`5ad1262`）：GlobalSearch 改走 `memoryLayerLabel(layer, advanced, name)`，角色改名後重投影 | 無。原本兩條「一般模式術語」斷言只在 vitest 全量跑失敗，**已查明與一般模式無關**（`characterName.ts` 刷新無世代概念造成的測試輔助 product-race），由 `a6e289e` 修好；vitest **1168/0（60 檔）**，全量連跑 6 次全綠 |
| #24 TaskComposer 印固定「已送達」 | **已有**（`5760f7d`／`8ba2a51`）：`src/work/delivery.ts` 六態投影，只有後端蓋 `deliveredAt` 才說「已送達」；TaskComposer 與工作卡片「再交代」共用；工作狀態標籤對齊產品用語且 claimed 仍無綠勾 | 無 |
| #25 限流測試 flake | **已有**（`af8d1f1`）：改以 `CharacterHub::set_clock` 注入假時鐘，精確斷言 50 接受／第 51 rate-limited／每 20 ms 補一則，HTTP 回執與 WS 共用同一份預算；連續 20 次 20/20 | 限流演算法本身未改 |
| #26 provider `is_operational()` 未接執行期 | **部分**（`c4ad753`）：`ProviderGate`（capability→provider 反向索引）接進 `observe_fresh`／push ingest／`run_step`／`simulate_plan`；共用能力只在**所有**擁有者都停下時才擋；能力清單投影為 `Availability::Disabled`（不新增 enum 值）；升級邊界改為第一次重啟即採安全預設並留 `provider.legacy-off-assumed` audit | 驗證仍是 fixture（無真 serial／MQTT／BLE）；`transition_provider` 不同步 registry 旗標（gate 在更外層，非安全漏洞） |
| iPhone 冷啟動／位址變更（真機發現） | **已有（App 端）**（`5111208`）：冷啟動依使用者上次連線意圖自動重連（沿用 1 s→15 s 退避，**感測絕不隨之恢復**）；連續 4 次連線層失敗或持續 60 秒後顯示固定文案並提供「重新配對」捷徑；TLS 指紋不符與撤銷維持各自文案 | **只有 iPhone 17 模擬器 XCTest（`ReconnectHintTests` 21 條）**；真機驗證 blocked |
| 一般模式易用性（非 finding） | **已有**（`6f5c7c6`）：「現在」頁待決定精確為 0 時顯示「目前沒有需要處理的事」（下限時維持「至少 N 項」）；iPhone 卡片新增「重新配對」與離線時的位址提醒；進階模式的原始欄位收進「連接診斷」區塊，一般模式完全不顯示 | 無 |
| 安全邊界（非 finding） | **已有**（`80fe2c8`）：`resolve_gateway_workdir` 改為雙向檢查，工作資料夾不得位於 runtime `state/` 之下；runtime 自己的主動式對話工作區搬到 `agent-workspaces/proactive` | 無 |
| `cancel_action` 誠實化（非 finding） | **已有**（`0c845e0`）：只有 driver 在 2 秒內確認才是 `Cancelled`，回錯／逾時／不可達一律 `Uncertain`；內建動器取消一律誠實回 `Uncertain` | **Breaking**（呼叫端不得把送出取消讀成已取消） |

### 13.3 流程與 CI

- `desktop-backend` CI job 從 `cargo check` 升為 `cargo clippy -D warnings`＋`cargo test`（`e70ef28`）——
  src-tauri 的單元測試（host safety overlay、character store、tray、click-through）與 clippy 此前從未在
  CI 上被強制。第一次 CI 就抓到一個本機 macOS 看不到的問題（ubuntu 上 macOS-only tray 欄位 dead_code），以 `8899e7c` 修好後 run 33807755834 全綠。
- `scripts/release.sh`：CHANGELOG 既沒有 `## [<version>]` 也沒有 `## [Unreleased]` 時直接失敗，不再靜默
  跳過（v0.5.0 就是被這個靜默跳過影響）。
- `RawLink::shutdown` 的 trait 文件改為對齊新的可中斷 fallback（`977731c`）。

### 13.4 證據等級總結（v0.5.1）

| 等級 | 範圍 |
|---|---|
| **真 Tauri 視窗**（本輪新增） | 主視窗、狀態列 11 項、首次設定精靈、原生資料夾選擇器四列、claimed≠verified、角色視窗顯示／隱藏、空白穿透與角色本體攔截、緊急停止＋可信覆蓋視窗、感測指示、停止所有感測、角色匯入資料夾、外部 daemon 與離線覆蓋、完全結束。**未驗**：Reduced Motion（需人類切 OS 設定）、快捷選單／玩具／使魔、adapter 崩潰 fallback |
| **模擬器** | iOS build＋XCTest 46/46（iPhone 17 模擬器）、serial pty 模擬器 |
| **fixture** | fake agent 子程序、`examples/fake_iphone`、內嵌 rumqttd broker、BLE 假事件流、arduino-cli 編譯 |
| **browser（Playwright）** | `pnpm test:e2e` 65/0（2.0 分）、`docs/assets/v05-evidence/` 的 39 張截圖重新產生（`desktop-companion`／`desktop-work`／`desktop-connect`／`desktop-more`.png 不在本輪拍攝清單內，仍是舊圖）、`pnpm perf` |
| **單元** | `cargo test --workspace` 827/0（66 target；基線 736／63）、Tauri backend 50/0（基線 46）、vitest **1168/0（60 檔；基線 988／49 檔）** |
| **真機** | **無**。iPhone 真機 blocked（鑰匙圈授權），ESP32 無實體板。v0.5.0 的 iPhone 真機證據仍然有效、未被推翻 |

### 13.5 v0.5.1 對抗審查

**執行**：`.claude/workflows/adversarial-review-v05.js`（find＝opus、independent verify＝sonnet）對 `0c845e0`
（20 個修復 commit 之後的分支狀態）跑一次，run id `0c845e0-20260903T185130Z`，完整報告在
`docs/reviews/adversarial/0c845e0-20260903T185130Z.md`（＋同名 `.json`）。**62 個 finding 送審、55 個 confirmed**
（high 7／medium 27／low 21）、5 個 refuted、2 個在審查期間已被其他 commit 修掉。

**處置**：55 個 confirmed 依檔案歸屬分 10 組（`.claude` 動態 workflow，每組先寫「舊行為下紅燈」的回歸測試再修），
整合者再補各組因檔案獨占清單而留下的清單外小修。最終：**已修 52／部分修 3／未修 0**；**7 個 high 全部已修**。
新增回歸測試（各組報告合計）：Rust 約 27 個（`interaction-runtime` 14、`interaction-api` 5、`interaction-character` 4、
`interaction-adapter-declarative` 3、`interaction-agent-gateway` 1；整合者另加 `interaction-character` 純 Gateway 閘門 1 個）＋前端 5 個新測試檔
（`regressions-review3-{companion,ia,memory,mobile,rig}`，約 43 個案例）＋既有測試檔的新增案例；新增測試在修復前全部實跑為紅燈（各組報告內有紅燈輸出）。

| 嚴重度 | finding | 處置 | 摘要 |
|---|---|---|---|
| high | agent-honesty-021 | 已修 | session token 現在只能讀自己 session 的 mailbox（`GET /v1/agent-sessions/{id}/messages` middleware＋handler 雙層擁有權），`MailboxReader::Agent` 在正式環境可達 |
| high | agent-honesty-022 | 已修 | `tools_disabled()` 成為唯一真相；codex connector 對 intent-only session 誠實拒絕（app-server／exec 都沒有等價 `--tools ""`）；主動式對話 `generativeAgent` 只接受 `claude-code` |
| high | character-protocol-036 | 已修 | 安全 intent 只能 fallback 到安全 intent：協商守衛（Rust＋TS）、manifest 驗證拒絕、三個 adapter 以 `envelope.intent` 為準、conformance 逐 intent 斷言；舊 pack 遷移不再產生 `emergency→sleep` 類映射 |
| high | ia-settings-005 | 已修 | 角色感測標籤改走 `statusProjection.sensorKindLabel`（與 tray／首頁／host overlay 同一份投影），iPhone 麥克風不再漏判 |
| high | link-transports-027 | 已修 | 緊急停止逐一 zip 動器結果；未確認的動器列進事件／audit／outbox（`totalActuators`／`unconfirmedActuators`），只有全部確認才說「所有輸出已中止」；`text.rs` 計畫罐頭文案不再預先宣稱 |
| high | mobile-server-059 | 已修 | stop-all 一則都沒送出時不再關去重窗，六個 mobile 動器不再被代簽「已停」 |
| high | safety-invariants-056 | 已修 | 停用高風險受器時 mobile provider 的 `receptor.offline` watcher 對仍在串流的手機送 stop，status／tray／overlay 不再無聲 |
| medium | agent-honesty-023 | 已修 | 續開比對**實際生效**的工具開關（零工具→有工具一律 `PolicyBlocked`）；桌面／CLI 續開 intent-only session 時原樣帶回 `["conversation.generate"]` |
| medium | agent-honesty-024 | 部分修 | 已關閉 session 保留 200 筆／30 天並真的呼叫 `Storage::delete_agent_session`；**殘留**：桌面每個 runtime 事件仍全量重取、`/v1/agent-sessions` 無分頁（§4 殘留 1） |
| medium | character-protocol-037 | 已修 | TS Gateway `renegotiate()` 先把 pending 結清為 uncertain、安全 intent 補 `system.text` |
| medium | character-protocol-039 | 已修 | 外部 adapter outbound：安全訊息等空位有配額 8／TTL 5 s，WS 寫入逾時 5 s 即斷線並把 pending 結清為 uncertain；**殘留**見 §4 殘留 4 |
| medium | character-protocol-040 | 已修 | 宣告即契約：Runtime 觀察邊界＋純 Gateway 進佇列前都擋沒宣告的輸入能力（`capability-not-declared`），TS 端同步 |
| medium | companion-gameplay-030 | 已修 | 使魔框以 `interactiveRegions()` 分類為 stage，不再吞掉點擊 |
| medium | companion-gameplay-031／director-pipeline-019 | 已修 | quiet 時永遠只走就地眨眼；單元素 ambient 池不再自我飢餓 |
| medium | companion-gameplay-032 | 已修 | 所有氣泡回到同一個 `bubbleTimer` 主人；sticky 安全文字不再被孤兒計時器抹掉 |
| medium | companion-gameplay-033 | 已修 | Roll Call 在暫停後一律回「停下來了」；`onVisibility` 先 suspend 再 beat |
| medium | companion-gameplay-034 | 已修 | Reduced Motion 下光點／逗貓棒不再跟游標；manifest 的 `gameplay.toys: disabled` 指自主遊玩（§4 殘留 6） |
| medium | director-pipeline-018 | 已修 | CompanionApp 保留中斷前 ambient 計畫；**殘留**：`director.ts` 根因（reactDetailed 無條件清 interrupted）未動（§4 殘留 7） |
| medium | director-pipeline-020 | 已修 | 誠實移除假的 utility 競爭：等優先事件為確定性替換，文件同步 |
| medium | docs-claims-050／052／054 | 已修 | CHANGELOG 不存在的 `asyncUtilTimeout` 條目移除；README／CLAUDE.md／FEATURES 改為「v0.5.0 已發布」；iOS README 46/46＋目錄樹 |
| medium | link-transports-028 | 已修 | cancel 只有裝置回 `not-found` 才是 `NotFound`，其他錯誤／逾時回 `Uncertain` |
| medium | memory-ui-001／002 | 已修 | `memory_list` 回 `total`／`limit`／`limitReached`；來源檢視器一般／進階分層 |
| medium | mobile-server-060／061 | 已修 | 已配對清單原子寫入＋載入錯誤不再吞；配對面板每 2 秒查 `pairingBurnedAt`／到期並顯示原因 |
| medium | perf-claims-012 | 已修 | 幀節奏基準線改為近 5 窗最短間隔中位數，可回升 |
| medium | perf-claims-013 | 已修 | v0.5.0 最終報告加效能／記憶體補記與正確交叉引用 |
| medium | protocol-conformance-042 | 部分修 | `pairing_ever_compared`／`pairing_not_recompared`：本連線曾比對過碼的通道不再被標 `pairingUnverified`；註記文案改為「這次握手無法證明」；**殘留**見 §4 殘留 2 |
| medium | rig-renderer-045 | 已修 | 過場水平錨點整體位移，頭與軀幹對齊；**觀察**：overlay／particles 不吃位移（§4 殘留 8） |
| medium | rig-renderer-046 | 部分修 | `startled-awake` 接上真實觸發（休息姿勢被戳）；**殘留**：`not-found`（36 之 1）仍只有預覽格（§4 殘留 3） |
| medium | safety-invariants-057 | 已修 | emergency-stop／stop-all／cancel 的 audit actor 依 token 種類歸因（`api`／`agent`／`agent:{id}@{session}`／`adapter:{id}`） |
| low | agent-honesty-025 | 已修 | 續開找不到紀錄、或舊紀錄沒有 `resolvedWorkdir` 一律拒絕（§4 殘留 9 為刻意的範圍限縮） |
| low | agent-honesty-026 | 已修 | 已關閉且經人工驗證的工作顯示「已由你確認並收尾」 |
| low | character-protocol-038 | 已修 | pending 佇列滿時安全 intent 補 `system.text`＋audit |
| low | companion-gameplay-035 | 已修 | 移除只寫不讀的 `restMs`；`carry` 成為真的中繼狀態；世界事件→表情用純函式 |
| low | ia-settings-007／008／009／010／011 | 已修 | 感測 banner key 含 `startedBy`；標籤對比 ~9.4:1；未知路由顯示「找不到這個頁面」；通知中心成為真 modal；狀態列「外觀與語言…」；**殘留**見 §4 殘留 10／11 |
| low | memory-ui-003／004 | 已修 | 貼上文字可命名；四處空清單文案真的會顯示 |
| low | mobile-server-062 | 已修 | 配對面板顯示配對資料全文＋主機位址＋複製按鈕（沒有相機也能配） |
| low | perf-claims-014／015／016／017 | 已修 | 「輸入→下一幀」改標為 WebView 段（下界＝量測環境幀距）；能力矩陣 <16ms 改為未達標；`reportHitRect` 先做時間閘；隱藏時只保留 CPP sweep 與記帳（狀態輪詢降頻 30 s，見 §4 殘留 12） |
| low | protocol-conformance-043／044 | 已修 | 韌體 README 對 pair-locked 的描述改正；模擬器浮點序列化鏡射 ArduinoJson |
| low | rig-renderer-047／049 | 已修 | stand↔sit 幾何連續形變（逐幀最大跳動 1.87 px）；組合式通道核心會呼吸 |
| low | safety-invariants-058 | 已修 | receptor／tool scope 帶 `maxUses` 直接拒絕（HTTP 400），不再回報假的 `maxUses` |

**refuted 5**（未修，報告內有反證）與**審查期間已修 2** 見報告。審查對象是 `0c845e0`，本輪修復之後**未再跑第二次
對抗審查**（時間預算）；修復本身由各組的紅燈→綠燈測試與最終全套回歸背書。

殘留 17 條見 `docs/releases/v0.5.1-known-limitations.md` §4.1。
