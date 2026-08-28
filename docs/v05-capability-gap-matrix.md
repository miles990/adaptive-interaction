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
| 本機反射延遲 | click/drag 同幀反應(<16ms),已達標 | 16–100ms | **已有** |
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
Interaction Director→**已有**（`react()`/`noteFinished()`/`scoreEvent` 已接進 App；等優先事件用 utility 競爭；
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
