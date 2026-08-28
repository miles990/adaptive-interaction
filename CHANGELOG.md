# Changelog

本專案採 [Keep a Changelog](https://keepachangelog.com/zh-TW/1.1.0/) 格式與
[語意化版本](https://semver.org/lang/zh-TW/)（`MAJOR.MINOR.PATCH`）。

版本一致性：workspace `Cargo.toml`、`apps/interaction-desktop/src-tauri/Cargo.toml`、
`apps/interaction-desktop/src-tauri/tauri.conf.json`、`apps/interaction-desktop/package.json`
四處版本必須相同——用 `scripts/release.sh <version>` 一次搞定。

## [Unreleased] — v0.5 產品重定位（角色・硬體・AI 三核心）

### Phase 7：整合、對抗審查、修復、全套回歸（2026-08-28）

> 這一段記錄的是「把 Phase 1–6 宣稱的東西變成真的」：新 Session 先不信任前一輪的進度敘述，
> 用 10 位審計 agent 逐條對照規格重建恢復矩陣（`docs/v05-recovery-matrix.md`），再跑
> `.claude/workflows/adversarial-review-v05.js`（11 維度 find → 獨立懷疑者 verify），
> 136 項審查 → 73 confirmed／59 fixed-meanwhile／4 refuted，只修 confirmed，每項附 regression test。
> 測試數字：Rust workspace **349 → 426**、vitest **138 → 319**、CLI E2E **59 → 63**、Playwright 24、Tauri 4 → 8、
> iOS XCTest 19（模擬器）。模擬器復測：撤銷 ≤0.035 s 斷線、estop 0.5 s 內停手機感測、Bonjour 真的廣播。

#### Fixed — 誠實階梯與安全底線
- Agent 程序結束而無結果 → 新 taxonomy 狀態 **`unknown`**（不再謊報 failed）；lease 到期／重啟發 timed-out／unknown；
  Claude `system/init` 不再等於 working（第一個 assistant/tool 事件才算）；codex／claude 的 `fetched` 只在 stdin 真的
  write+flush 後才發；approval 裁決（人類或 300s 看門狗）回寫信箱 `approval-resolved`，AiPage 顯示「已由看門狗自動拒絕」；
  人類 GET 信箱是「觀看」不是「送達」（不再冒充 delivered）；`PendingApproval.summary` 真的顯示；CLI `agents create --resume`／
  `agents resume`；codex `thread/resume` 真的重新上鎖 cwd／approvalPolicy／sandbox（以 codex-cli 0.150.1 產生的 schema 核對）。
- SSE：初次連線不再重播整個 ring buffer（改從 `status.eventSequence` 起）；daemon 重啟（`startedAt` 改變）重置 cursor；
  角色 machine 忽略早於 App 啟動的重播事件（safety 狀態仍以 `/v1/status` 為準）。
- iPhone：**撤銷即斷線**（CancellationToken；之前只移除表項）；ack/err/ble.* 需 authed 且綁定裝置；heartbeat 15s／idle 45s；
  斷線強制停用高風險 receptor 且重連不恢復；facts 依 manifest 過濾、mic-level 不持久化；**estop 同時停 iPhone 感測**
  （`stop-all{sensors:true}`，iOS 端同步）；啟用中的手機麥克風出現在 `status.activeSensors`（tray／首頁／角色視窗）；
  `iphone.mic-level` 需 session consent，撤銷即丟棄；**綠勾（verified-success）只走人類驗證路徑**，plan/agent 帶入一律拒絕；
  extra 參數不能放寬 policy 的 L3 硬限制；estop 500ms 內只送一則 stop-all、無連線誠實 Err；撤銷持久化失敗誠實回 Err；
  Bonjour 服務型別改 `_interact-ai._tcp`（舊名超過 15 bytes 註冊失敗）並在 status 回報 `bonjour`；`started` 只在 bind
  成功後設；devices 為空不開埠；`mobile ble-scan` CLI／Tauri／UI 可達；MobileSection 顯示手機 permissions 與 Bonjour 狀態；
  **測試模式不把 Bonjour 記錄廣播到實體區網**（模擬不得有外部副作用）。
- 硬體 link：health()/status() 依真實連線（斷線 offline、未握手 degraded，不再硬編 healthy）；serial ENOTTY 判斷收窄
  （實測 pty 錯誤字串）；MQTT dedupe／重連／QoS1 真斷言；provider 停用／撤銷關閉連線（serial/mqtt/ble `shutdown`）；
  `hello.caps` 能力識別（未宣告 → 不上線、failed）；hello `proto`／`pairing` 核對；state 核對 deviceId；cmd 帶 deadline、
  重連清佇列並以 `link-reset` 結束等待；無 id 的 err 以拒絕原因結束（不演逾時）；裝置 not-paired → 該次 failed、下一次前重握手
  （絕不自動重送實體命令）；BLE 途中失敗＝uncertain 不重試；stop-all 無 ack 誠實回 Err；握手逾時不再標 dispatched。
- L0 純呈現（`builtin.presentation`）的 uncertain 不進「待我決定」；外部裝置（含 iphone.character）仍進；Inbox pendingCount
  在截斷前計算；風險分級 L0–L4 標籤（`riskTier.ts`）；精靈預設不靜默寫入安靜時段／initiative，步驟二選擇真的寫入 agent 路由。

#### Fixed — 角色
- 四段式：timeline 真的播 **exit**（≤260ms；安全狀態立即搶佔不播）、缺段以派生段補齊並標 `derived`、8 個高頻表情手寫 exit；
  組合通道：工作／等待只覆蓋核心／頭飾／裙光／耳朵、身體保留遊玩姿勢；`personality.ts` 個性模型接進 director／playfield／timeline；
  `director.react()`／`noteFinished()`／`scoreEvent` 接進 App，quiet 分支可達，「一小時內不要主動說話」也管角色端；
  machine 旗標即時同步舞台（estop 不再多追 ≤480ms 球）；hit-rect 逐幀節流回報、Rust 點擊穿透輪詢 80ms；拖曳期間持續 hold；
  放下依速度／高度／落點四種落地；第 6 種玩具 trinket；多角色互看／回愛心／被追者逃跑；hover 短氣泡；硬體／提供者事件演出；
  氣泡／音效（預設關）／拖曳／勿擾開關；estop 停語音並清 transient；睡眠類 ambient 剛互動後不秒回；Reduced Motion 真靜態
  且執行中可切換；30fps 降級遲滯；`ask` 為真相狀態（AI 只能點播 `question`）；clampParams 嚴格型別；lerpParams 允許回彈；
  `poseBlend` 通道消除 lie↔stand 頭部瞬移；角色互動記憶（有界、不推論人格、不進知識庫）；Rust hit-rect clamp；隱藏視窗通知 runtime。

#### Fixed — 控制中心、記憶 UI、文件
- 通知中心鍵盤／焦點陷阱；淺色主題 `--panel`／`--input-bg`；一般模式術語外洩（Lease／provider session／raw JSON／UUID）改人話；
  §11 記憶與知識一般模式只有三區、規格人類文案、技術 tab 進階才顯示；到期記憶只能刪除（不再有必失敗的按鈕）；
  GlobalSearch 人話；AiPage 訊息 5 秒輪詢＋approval 倒數；WorkPage 顯示精靈選擇；守門測試（5 入口／單一主人／舊 tab 對照）。
- Provider「已測試」：`detail.tested` 證據（handshake／capability／human）、`POST /v1/providers/{id}/test`（human-only、唯讀）、
  CLI `providers test`、UI 六階人話＋「測試裝置」。
- 效能量測可重現（`pnpm perf`）：drawRig／全舞台／輸入延遲（真狀態改變）／heap／bounded／3 天數值；作廢無來源的舊數字。
- ESP32 韌體：arduino-cli 實際編譯兩組態（`compile.sh`＋Apple Silicon ctags shim）；浮點參數規則明確（兩端一致）；
  MQTT 非阻塞退避、效果進行中不連線；BLE 有界佇列由 loop 統一處理；nonce 環；模擬器鏡射 8 周邊與所有 err 形狀。
- iOS：對 iOS 26.5 SDK typecheck 0 error 0 warning；swiftc 直接編模擬器 .app＋DEBUG 啟動參數；與真 daemon 完成配對／動器／撤銷
  閉環；XCTest 在模擬器 19/19；`stop-all{sensors}` 停感測並顯示原因。
- 文件：README／CLAUDE.md／DESKTOP-GUIDE／FEATURES／acceptance-evidence／gap matrix 的過期與不實敘述全部改正
  （版本號、舊 IA、127.0.0.1 例外、「無 Xcode」、「撤銷立即斷線」、「36 表情皆三段」、HMAC 配對、看向游標、效能數字、59 checks）。

#### 已知限制（v0.5，完整清單見 `docs/acceptance-evidence.md` v0.5 章節）
ESP32 與 iPhone 皆無真機驗收；BLE 無真機；硬體 Observed/Verified 死路（acknowledged 為誠實上限）；provider 停用後連線關閉不可逆；
MQTT 內部佇列 deadline 伸不進；`waiting-input` 無 connector 來源；跨視窗桌面漫遊／邊緣探頭／其他視窗事件反應／Fullscreen／
OS 勿擾偵測未做；桌面 BLE gateway 只有 scan；Camera／Location／Live Activity／SFX／區網裝置事件未做；WebSocket／HID／
Home Assistant adapter 未做；配對期可被區網 peer 燒掉（已 audit）；效能數字為 headless Chromium 非 WKWebView。


### Added — Phase 2：正式小樞（Q 版貓娘女僕）與動畫核心

- **小樞 v3「女僕正式版」**：執行期參數化分層 rig（`companion/rig/`），
  非 sprite sheet——每幀由 `drawRig(params, palette)` 純函式即時繪製。
  約 2.5–2.6 頭身 Q 版：柔黑深灰紫短髮＋不對稱髮束＋呆毛、紫灰大眼、
  小虎牙；奶白×深灰紫女僕工作服（泡泡袖、工具圍裙＋口袋、蓬裙＋
  燈籠褲、圓頭軟靴、分體頭飾讓位貓耳）。3 個調色盤變體
  （classic／dusk／sakura），pack kind=`character-rig` schema 2.0。
- **服裝參與功能呈現**：左耳冷藍=感知、右耳暖橙=行動、胸前蝴蝶結
  結晶核心=Runtime/AI 工作（呼吸發光）、頭飾光=Agent 連線、
  裙擺細光=waiting（琥珀）/unknown（紫）/blocked（紅）、尾尖紫光=工具、
  policy 小盾保留。
- **組合式角色通道**：~40 個有界參數（body/head/gaze/eyes/brows/mouth/
  ears/hair/headpiece/core/arms/tail/legs/skirt-light/overlay/particles），
  `clampParams`/`lerpParams` 保證任何輸入都畫得出合法畫面。
- **36 個正式表情**（spec §7.5 全清單）：Phase 2 交付時只有 hold＋部分
  enter/loop（13/36 兩段齊全、0/36 四段齊全，「離開」段是死資料——Phase 7 對抗
  審查確認並修正為真四段式，見下方 Phase 7）；加基態/相容別名共 55 個表情；
  Game feel：anticipation、squash&stretch、hit-stop、粒子（灰塵/星光/
  愛心/zzz）、headpiece 歪掉扶正等 secondary motion。
- **RigRenderer**：表情 crossfade（~180ms 輕微回彈）、自動眨眼
  （幾何分布間隔）、gaze/ear 微動作疊加（僅 ambient 表情）、Reduced
  Motion 靜態姿勢仍保留狀態辨識；誠實映射：machine 的 success＋
  frameSlice（未驗證）→「聲稱完成」只點頭，無 slice（已驗證）→
  「驗證成功」才有綠勾與慶祝。truth-state 表情（成功/失敗/阻擋/未知/
  緊急/離線）不可被 AI 或 ambient 點播，fallback 鏈永不落到成功。
- **Interaction Director**（`companion/director.ts`）：統一行為導演——
  ambient 變體池（hazard 抽樣、每表情冷卻、防重複 3、放鬆度門檻）、
  被真實事件搶佔後可恢復（20 秒 TTL）、quiet 只剩眨眼、Reduced Motion
  只允許眨眼類、utility 評分沿用 EventClass 階梯；取代舊
  scheduleMicroAction 路徑。（Phase 2 交付時 `react()`/`noteFinished()`/
  `scoreEvent()` 尚未接進 App、quietHours 鍵在 runtime status 缺失——
  Phase 7 修正，見下方。）
- 控制中心小樞頁：36 表情即時預覽（與桌面同一套 rig 程式）；
  首次設定精靈的角色預覽同源。預設 pack 改為 `shu-maid`；
  v1/v2 sprite packs 保留為相容層（fallback 鏈不變）。
- 設計稿與驗收畫面：`docs/assets/v05-evidence/shu-maid-rig-sheet.png`
  （代表性姿勢×明暗底×3 配色＋36 表情 hold 網格）。

### Added — Phase 6：iPhone Mobile Provider

- **桌面端 Mobile 伺服器**（`interaction-runtime/src/mobile.rs`）：
  TLS WebSocket（自簽憑證、SHA-256 指紋由 QR 載荷釘選）＋Bonjour 廣播
  （服務型別 `_interact-ai._tcp`——Phase 6 交付時的 `_interact-ai-mobile`
  超過 RFC 6763 15-byte 上限、註冊靜默失敗，Phase 7 修正；TXT 帶指紋）＋
  配對儀式（一次性 6 位配對碼 5 分鐘、HMAC-SHA256 challenge-response、
  錯一次即作廢防暴力）＋每台 iPhone 獨立 device token（只存 SHA-256）。
  撤銷立即斷線與「已配對過才開網路埠」在 Phase 6 交付時**不成立**
  （撤銷只移除表項、連線仍活著；devices.json 存在即開埠），Phase 7 修正並以
  測試釘死。伺服器綁 0.0.0.0（區網），這是 iPhone 配對的刻意設計。
- **iPhone 能力接進既有管線**：4 個受器（motion 語意事件/battery/touch/
  mic-level——mic 需 consent）＋6 個動器（haptic/notify/tts/torch/flash/
  character，全部 consent-gated、健康度跟連線走）；act→ack 收據
  acknowledged＋deviceApplied；ack 逾時＝結果未知不重送；estop →
  stop-all 廣播到所有手機；斷線 → provider Disconnected、能力
  unavailable。「高風險感測不自動恢復」在 Phase 6 只由手機端強制，桌面端
  於 Phase 7 補上（斷線即停用 iphone.mic-level）。觀察 receptor 白名單
  ——Phase 6 只驗 receptor id，Phase 7 起依 manifest 欄位過濾 facts。
- **BLE Gateway 通道**：桌面端只實作 `ble.scan`（`POST /v1/mobile/ble/scan`、
  CLI `mobile ble-scan`）；iOS App 端有 connect/gatt(read/write/subscribe)
  實作但桌面端**沒有對應送端**——列為已知限制。
- **UI／CLI**：連接與權限→裝置與能力新增 iPhone 區（配對 QR SVG＋配對碼、
  裝置清單含連線狀態與手機自報感測、撤銷）；CLI `interact-ai mobile
  status|pair|revoke`；`/v1/mobile/*` human-only（agent token 連 GET 都拒）。
- **iOS Companion App 原始碼**（`apps/interaction-ios/`，SwiftUI）：
  配對（QR/手動＋憑證指紋釘選）、語意 motion 分類器（lifted/shaken/
  placed/rotated，3 秒視窗記憶體內、不存軌跡）、感測預設全關＋權限誠實
  顯示＋斷線自動停用高風險感測、haptic（含 purr/heartbeat）/通知/TTS/
  手電筒/螢幕閃示/角色狀態（綠勾只在 verified-success）、CoreBluetooth
  gateway。Phase 6 交付時只做了 `swiftc -parse`（當時誤以為本機無 Xcode）；
  Phase 7 以 Xcode 26.6／iOS 26.5 SDK 完整 typecheck、編成模擬器 .app 並與真
  daemon 完成配對閉環（見 Phase 7）。**真機未驗收；不宣稱 iPhone 可操作任意
  USB 配件。**
- 測試（【模擬 iPhone】程序內 client，明確標示非真機）：Phase 6 交付 3 個
  測試（TLS 指紋釘選配對閉環、錯誤配對碼拒絕＋配對期作廢、token 重連／撤銷後
  auth-fail）；「觀察 ingest＋白名單」「斷線→Disconnected」當時只有送出沒有
  斷言——Phase 7 補齊為 16 個測試（含 mutation 驗證）。

### Added — Phase 5：真實硬體（Serial／MQTT／BLE adapters＋ESP32 參考裝置）

- **裝置線協定 v1**（`interaction-adapter-declarative/src/protocol.rs`，
  三傳輸共用）：hello 身分驗證（**埠/IP/topic 永遠不是身分**——
  hello.deviceId 必須等於 spec.expectedDeviceId，不符即拒）、配對碼握手
  （pair/pair-ok/pair-fail，建議 secret://）、cmd 帶 action id＋nonce、
  裝置端 16 筆 id ring dedupe（重複 ack dup:true 不重放效果）、cancel、
  read/state、stop-all。**ack 逾時＝結果未知且絕不自動重送**（實體效果
  不得重複觸發）；斷線退避重連（1s→15s cap）後強制重新握手。
- **Serial 傳輸**（feature transport-serial；serialport crate，macOS pty
  模擬器 ENOTTY 時誠實退回純檔案 I/O）＋**MQTT 傳輸**（transport-mqtt；
  rumqttc，QoS1 `<prefix>/to-device|from-device`）＋**BLE 傳輸**
  （transport-ble；btleplug GATT command/state characteristics，僅
  macOS/Windows 編譯——Linux 誠實拒絕）。收據誠實：send 失敗=failed、
  裝置 ack=acknowledged（附 deviceApplied 揭露韌體 clamp）、ack 逾時=
  dispatched+ackTimeout→runtime watchdog 標 uncertain；estop 對每台
  裝置直送 stop-all。
- **YAML spec 擴充**（schema 追加欄位）：`request` 改為選填（http/sse
  專用）；link 傳輸用 `command:{name,params}`＋`serial:/mqtt:/ble:` 設定
  塊；同一 adapter 的 link capability 必須指向同一台裝置。
- **ESP32 官方參考裝置**（`firmware/esp32-companion/`）：Arduino 韌體
  （USB Serial＋Wi-Fi/MQTT；BLE 可選 NimBLE）、RGB LED/按鈕/HC-SR04/
  光敏/DHT22/震動馬達/伺服/蜂鳴器；**韌體硬限制**（vibe duty≤0.8、
  ≤3000ms、脈衝間隔≥500ms；buzzer 200–4000Hz、≤2000ms、duty≤50%；
  servo 10–170°、300ms 節流）主機不可解除，clamp 後以 ack.applied 誠實
  回報；BOM／接線圖／Flash／測試步驟／YAML 範例齊備（zh-TW README）。
- **模擬器與真機分離標示**：`scripts/esp32-serial-sim.py`（pty，與韌體
  同協定、模擬硬限制與 dedupe）；CLI E2E 新增「Serial hardware closed
  loop (SIMULATOR)」段——provider 註冊→三段式人類授權（enable→policy
  allowlist→consent）→受器經配對握手讀 facts→cmd ack acknowledged→
  deviceApplied 揭露 1.0→0.8 clamp→estop stop-all 送達裝置（Serial 段 4 個
  check；整支腳本當時共 59 checks，Phase 7 增為 63）。
  MQTT 閉環以內嵌 rumqttd broker＋模擬裝置測試（含身分不符拒絕）。
  **真實 ESP32 硬體驗收未執行**（本環境無實體裝置）——Phase 5 交付時韌體僅經
  程式碼審閱＋協定模擬器對測；Phase 7 已用 arduino-cli 對 esp32:esp32 3.3.11
  實際編譯兩種組態（`firmware/esp32-companion/compile.sh`），仍未在真板驗收。
- 裝置 UI（連接與權限→裝置與提供者）：provider 卡片顯示人話狀態說明
  （只發現≠已配對≠已啟用；「已測試」於 Phase 7 補上）＋「小樞可以知道／
  小樞可以做」裝置導向能力清單。

### Added — Phase 4：AI 角色閉環（taxonomy、人工驗證、對話層）

- **Agent Session taxonomy 事件**（`agent.session.state`）：runtime 對
  created（queued）/fetched（任務真的送進子程序）/working/waiting-input/
  waiting-consent/claimed-completed/verified/failed/timed-out/cancelled/
  closed 發出標準化事件；小樞逐一映射成演出——等 Codex/等 Claude 專屬
  等待動畫、fetched=翻找、working=努力工作、waiting=請求確認、
  claimed 只點頭、cancelled 誠實清場。
- **人工驗證（claim → verified 唯一路徑）**：
  `POST /v1/agent-sessions/{id}/verify`（human token 專屬；agent/session
  token 一律 403）、CLI `interact-ai agents verify <id> --note`、
  AiPage「標記為已驗證（我確認過結果）」按鈕。記錄
  `humanVerified{at,note}`（audit 留痕）；只有 verified 事件會讓小樞播
  綠勾與慶祝。不可重複驗證、active session 不可驗證、備註 ≤500 字。
- **Resume 通路打通**：`CreateAgentSession.resumeProviderSessionId` →
  gateway SessionSpec（claude --resume / codex thread resume；sandbox 與
  權限旗標由 connector 重新上鎖，不繼承不放寬）；AiPage 已關閉 session
  提供「接續上次（唯讀）」。
- **Conversation Provider 介面（L1）**＋本機降級：
  `companion/conversation.ts` 可插拔介面；預設 LocalTemplateProvider
  純確定性規則——決定是否回話、選語氣與 behaviorIntent、判斷是否建議
  建立 Codex/Claude 任務；問句誠實承認「本機沒有答案」，絕不為普通
  反應啟動昂貴工作 Agent。
- 誠實不變量回歸釘死：runtime 測試（verify 唯一路徑/不可重複/長度界線）、
  前端測試（claimed→success-claimed 無綠勾、verified→success-verified）、
  CLI E2E（+3 verify 檢查）。

### Added — Phase 3：遊戲互動（VS Code Pets 基準＋超越）

- **遊玩場（StageRenderer）**：小樞的透明視窗加寬為小舞台
  （視窗＝角色寬×2.6、高×1.35），角色可在場內散步、翻面、追逐；
  hit-rect 隨角色與玩具動態更新（Rust 每 500ms 收到新互動框），
  空白區維持點擊穿透。
- **第一批玩具＋輕量 2D 物理**（`companion/playfield.ts`，純函式可測）：
  毛球/紙團/紙飛機（滑翔）/光點/逗貓棒；資料模型含位置/速度/重力/
  碰撞/抓取狀態/擁有者/興趣值/冷卻/生命週期（150s TTL、上限 4）。
  拖曳投擲：方向與速度來自實際拖曳軌跡；牆壁與地面反彈、摩擦停下。
- **追逐/撲抓/帶回/拒絕歸還**：hazard 抽樣（dt 縮放，非固定週期）；
  撲抓 75% 成功（光點永遠撲空）；30% 機率想獨占（keep-ball 演出後
  才放下）；玩過的玩具進冷卻。machine 真實事件（點擊/工作/安全狀態）
  永遠搶占遊玩；緊急/離線/暫停凍結全場。
- **多角色架構（小型使魔）**：同一遊玩場最多 3 隻迷你貓精靈，
  自主散步/睡覺/互相注意/打招呼（愛心）/追逐；可個別命名、換配色、
  移除——與 VS Code Pets 同為單一面板多寵物模型。
- **場景**：透明桌面（預設）/桌面巢穴/工作桌/窗台/夜間——低調小道具，
  透明模式完整保留。**Roll Call**：「現在大家在做什麼」人話清單，
  由角色視窗經 presentationHello 誠實回報（Rust 端白名單＋長度驗證），
  控制中心小樞頁顯示；machine 真實狀態（等待確認/緊急停止…）優先。
- **拖曳體感**：拖起小樞＝lifted 懸空演出；放下＝wobbly-landing
  （灰塵粒子＋頭飾歪掉再扶正）。連戳 ≥3 次＝poked-rapid 抗議、
  不再狂開關選單。
- **玩耍偏好（單一主人＝小樞頁）**：名字、場景、玩耍/游標互動/靠近
  反應/自主散步各自可關；角色設定匯出/匯入（白名單驗證，不含權限/
  位置/token）。游標座標只活在角色視窗 canvas 內，永不送 runtime/AI、
  不持久化；Reduced Motion 自動停止玩耍與移動、玩具不彈跳。

### Changed — Phase 1：控制中心簡化

- **一級導覽 9 → 5**：現在／小樞／工作／連接與權限／更多。
  工作＝AI 工作階段＋自動互動；連接與權限＝裝置與能力＋同意與安全；
  更多＝記憶與知識＋活動歷史＋設定。舊 tab id（tray 深連結、Runtime Inbox
  route：`ai`/`automations`/`capabilities`/`safety`/`memory`/`activity`/`settings`/
  `senses`/`responses`/`toolops`）全部相容折疊到新入口，不破壞既有深連結。
- **Activity 改為右上 Inbox**：待決定事項在右上角「通知中心」逐項可前往；
  完整活動歷史移到「更多 → 活動歷史」，不再佔一級入口。
- **首頁瘦身（現在）**：移除完整權限地圖（唯一的家改為「連接與權限 → 同意與安全」）；
  保留系統狀態、感測／待決定／進行中工作摘要、最近互動故事與快速操作。
- **每項設定只有一個主人**：小樞外觀／Persona／劇情進度／主動式對話／
  AI 主動程度／安靜時段全部集中「小樞」頁；設定頁與同意與安全頁只留摘要與
  「前往小樞」，不再有第二份相同開關。
- **首次設定精靈 7 步 → 3 步**（認識小樞／AI 幫手／安全預設）：低風險本機能力
  自動保守挑選（未知不預選不變）；AI 幫手只做 discovery／登入檢查，不授權工作區；
  主動對話預設「必要時」；硬體掃描移出精靈；仍走同一 onboardingCommit 原子契約。
- 進階模式技術頁面全部保留（零能力退化）；緊急停止入口不變
  （頂欄／tray／全域搜尋／CLI，解除仍走同意與安全的安全流程）。

## [0.4.1] - 2026-08-28

### Fixed

- Keep macOS hardware-profiler helpers out of non-macOS production builds while retaining
  cross-platform fixture tests, and adopt Rust 1.98-compatible Clippy forms for activity
  ordering, knowledge pagination, and WAV sample parsing.

## [0.4.0] - 2026-08-28

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
