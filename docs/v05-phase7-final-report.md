# v0.5 Phase 7 最終交付報告（2026-08-28）

> 依規格《adaptive-interaction-v05-core-experience-prompt.md》§18 的 25 項順序撰寫。
> 每個數字都是本機實跑；模擬器／fixture／程序內 client 一律標示；沒有任何 ESP32 或 iPhone 真機證據。
> 本報告不 commit、不 push、不 release（依 repo 規則需使用者明確授權）。

## 1. 基準 commit、環境與 worktree 狀態

| 項目 | 值 |
|---|---|
| HEAD | `a898996` feat(hardware): v0.5 Phase 5（本機 6 個未 push commit：Phase 0–5） |
| origin/main | `d200e2c` release: v0.4.1 |
| 工作樹 | **全部未提交**：Phase 6（前一 Session）＋Phase 7（本 Session）——146 個已追蹤檔修改（+11 466／−1 053）＋42 個新檔（約 12 700 行）＋7 張新截圖；`git diff --check` 乾淨 |
| 環境 | macOS 26.2（Darwin 25.2.0）、Apple M2 Pro、rustc 1.94.0、node 24.5.0、pnpm 10.27.0、Xcode 26.6（`DEVELOPER_DIR`）／iOS 26.5 SDK、arduino-cli 1.5.1＋esp32:esp32 3.3.11、Universal Ctags 6.2.1（Apple Silicon ctags shim） |
| 版本號 | 0.4.1（v0.5 未發布；`main` 開發中） |
| 上一 Session 殘留 | 無 agent／背景程序在寫入（另一 claude session 只是 cwd 在此 repo） |
| 模型分配 | 主迴圈 fable-5；審計／驗證 sonnet（fable-5 於 16:50 前觸發速率上限，151 個驗證重跑）；修復 opus |

## 2. 產品重構摘要

v0.5 把產品主體拉回三核心：**角色生命感與遊戲互動 > 真實硬體閉環 > AI Agent 工作與對話閉環**，安全留在底層。
Phase 1–6 交付了 5 入口 IA／3 步精靈／小樞 v3 rig／Director／遊玩場／AI taxonomy／Serial-MQTT-BLE adapter／ESP32 韌體／
iPhone provider；Phase 7（本 Session）先**不信任進度敘述**，用 10 位審計 agent 逐條對照規格重建恢復矩陣（463 列），
發現 Phase 2–6 多處「文件高於程式」（四段式離開段是死資料、撤銷不斷線、Bonjour 註冊失敗、health 硬編 healthy、
quietHours 鍵不存在、§11 記憶 UI 整段未做、效能數字無來源…），再以對抗審查（136 項審查、73 confirmed）驅動 3 輪 8 組修復，
每項附 regression test，最後全套回歸與文件校正。

## 3. 修改檔案完整清單

見 `git status`／`git diff --stat`（146 M＋49 untracked）。分類：

- **Rust runtime**：`crates/interaction-runtime/src/{mobile.rs（新）,runtime.rs,agents.rs,gateway.rs,presentation.rs,providers.rs,
  executor.rs,activity.rs,sensors.rs,hardware.rs,lib.rs}`、`tests/{mobile_loop（新）,agents_loop,gateway_loop,presentation_loop,
  providers_loop,runtime_loop,sensors_loop}.rs`、`tests/fixtures/{fake_claude.sh,fake_codex.sh（新）}`、`Cargo.toml`
- **硬體 adapter**：`crates/interaction-adapter-declarative/src/{protocol,serial,mqtt,ble,link_caps,lib}.rs`、
  `tests/{protocol_honesty,mqtt_loop,http_loop}.rs`
- **Agent gateway／core／api／cli**：`crates/interaction-agent-gateway/src/{claude,codex,lib}.rs`、`crates/interaction-core/src/agent.rs`、
  `crates/interaction-api/src/{lib,routes}.rs`＋`tests/api_e2e.rs`、`crates/interaction-cli/src/{main,commands}.rs`＋`tests/cli_e2e.rs`
- **桌面前端**：`apps/interaction-desktop/src/{App.tsx,styles.css,desktop.ts,api.ts,transport.ts,riskTier.ts（新）}`、
  `src/companion/{CompanionApp.tsx,machine.ts,director.ts,playfield.ts,presentationCommands.ts,attention.ts（新）,gameFeel.ts（新）,
  personality.ts（新）,interactionMemory.ts（新）}`、`src/companion/rig/{expressions,timeline,stage,renderer,params,draw,perf-entry（新）}.ts`、
  `src/pages/{Onboarding,HomePage,AiPage,SettingsPage,SafetyPage,ConnectPage,MorePage,MemoryKnowledgePage,WorkPage,CompanionPage,
  CapabilitiesHub}.tsx`、`src/components/{Dialog,GlobalSearch,CapabilityCard}.tsx`、`src/test/*`（+7 檔）、`e2e/*.spec.ts`、
  `scripts/shu/{perf-rig（新）,preview-rig}.mjs`、`package.json`、`src-tauri/src/{lib,supervisor}.rs`
- **iOS**：`apps/interaction-ios/**`（16 檔，Phase 6 新增；Phase 7 修 Swift 6 警告、DEBUG 啟動參數、`stop-all{sensors}`、Bonjour 名）
- **韌體／模擬器**：`firmware/esp32-companion/{esp32-companion.ino,README.md,config.h.example,compile.sh（新）,tools/ctags-shim/ctags（新）}`、
  `scripts/{esp32-serial-sim.py,v03-cli-e2e.sh}`
- **文件**：`CHANGELOG.md`、`README.md`、`CLAUDE.md`、`.gitignore`、`docs/{acceptance-evidence,FEATURES,DESKTOP-GUIDE,
  capability-completion-matrix,v05-capability-gap-matrix,v05-recovery-matrix（新）,v05-phase7-final-report（新）}.md`、
  `docs/assets/v05-evidence/*.png`（Playwright 重產＋`ios-sim-*.png`）
- **工作流**：`.claude/workflows/adversarial-review-v05.js`（新；seed＋12 維度＋雙人驗證＋模型覆寫）

## 4. 5 頁控制中心 IA 與設定歸屬

| 入口 | 內容 | 單一主人 |
|---|---|---|
| 現在 | 系統狀態、感測／待決定／進行中工作摘要條（與 Inbox 同一 `pendingCount`）、最近互動、快速操作 | — |
| 小樞 | 外觀／配色／名字、安靜／自然／活潑、玩耍／游標／靠近／散步／**氣泡／音效／拖曳／勿擾**、主動對話與頻率、安靜時段、36 表情預覽、匯入匯出、角色互動記憶 | 小樞如何表現 |
| 工作 | Codex／Claude Code provider、工作階段（含 approval 裁決狀態、resume）、自動互動、精靈選擇摘要 | AI 做什麼、何時觸發 |
| 連接與權限 | iPhone（配對／撤銷／權限／Bonjour）、裝置與能力（六階狀態＋測試裝置）、同意與安全（**L0–L4 風險分級**＋三區權限地圖唯一的家） | 可以讀取／操作什麼 |
| 更多 | 記憶與知識（一般模式三區）、活動歷史、設定（語言／外觀／視窗／啟動／備份／版本／進階模式） | 資料如何保存、一般設定 |

守門：`regressions-v05.test.tsx` 斷言 `SIMPLE_NAV` 恰 5 項、initiative／quietHours／proactiveDialogue 編輯器只在 CompanionPage、
舊 tab id 對照表完整；Playwright「新 IA：5 個一級入口全部可達」與 390px 底部導覽通過。

## 5. 首次設定三步流程

認識小樞（顯示／預覽 rig／安靜–自然–活潑預設自然／玩耍與游標唯讀說明「預設開啟」／音效「預設關閉，可在小樞頁開啟」）→
要讓小樞幫忙工作嗎（Codex／Claude／兩者／稍後；只做 discovery＋登入狀態；選擇**真的寫入** `agentRoutes`；不授權 workdir）→
安全預設（麥克風／攝影機／位置預設關；外部裝置首次詢問；Agent 寫入逐 workdir；主動對話「必要時」；顯示暫停與 Emergency Stop）。
**預設不再靜默寫入**安靜時段／每小時上限／initiative（「進一步自訂」勾選才寫）。測試：onboarding.test.tsx 11、e2e app.spec 首測。

## 6. 小樞正式角色設定、配色、輪廓、服裝與資產格式

執行期參數化分層 rig（`companion/rig/`）：約 2.5–2.6 頭身 Q 版貓娘女僕；奶白×深灰紫工作服（泡泡袖、工具圍裙＋口袋、蓬裙＋燈籠褲、
圓頭軟靴、分體頭飾讓位貓耳）；3 調色盤 classic／dusk／sakura（pack kind `character-rig` schema 2.0，manifest 只承載 palette；
骨架／表情／通道／變體在 TS）。服裝表現：左耳冷藍=感知、右耳暖橙=行動、胸前結晶核心=Runtime／AI 工作、頭飾光=Agent 連線、
裙擺細光=waiting／unknown／blocked、尾尖紫光=工具。設計稿：`docs/assets/v05-evidence/shu-maid-rig-sheet.png`。
**未做**：口袋取物／袖口面板的功能呈現；逐幀手繪誇張 sprite；WebGL 評估。

## 7. 36 表情與全部動畫狀態清單

`OFFICIAL_36`（`rig/expressions.ts`）：疑問、偷看、歪頭、探頭、無語、放空、哈欠、趴平、伸懶腰、被吵醒、假裝沒聽見、悄悄靠近、被點、
被連戳、被拖起、落地站不穩、抱球、不還球、撲空、滑倒裝沒事、被稱讚、偷懶被抓、等玩家、玩家回來、思考、找資料、努力工作、等 Codex、
等 Claude、需要確認、權限不足、找不到、結果未知、聲稱完成、驗證成功、工作失敗。**每個都四段**（進入／保持／小循環／離開；未手寫的段落
由 `resolveSegments` 派生並標 `derived`，8 個高頻表情手寫 exit；rig.test 逐幀驗證 exit 真的播放）。另有基態／別名／Phase 3–7 追加
（play-chase、play-carry、hold-ball、keep-ball、pounce-miss、await-player、land-light、device-hello、device-lost、operate-tool、ack-nod、
question…）共 60 餘個。真相狀態（成功／失敗／阻擋／未知／緊急／離線／**需要確認**）不可被 AI 或 ambient 點播。

## 8. Interaction Director、Attention、Utility、Scheduler 與 Mixer

`director.ts`（ambient 變體池、冷卻、防重複、被搶佔後恢復＋剛互動不回睡眠、quiet／勿擾／Reduced Motion 降級、`react()` 意圖白名單）；
`machine.ts`（真相優先階梯＋**等優先事件以 `scoreEvent` 競爭**、搶佔 vs 自然到期區分、estop 清 transient）；`attention.ts`
（勿擾／quietUntil／hover 氣泡政策／音效與氣泡誠實結果）；`personality.ts`（個性→速度／距離／冷卻／變體／注意順序）；
`rig/timeline.ts`（四段式＋crossfade＋回彈＋`poseBlend`＋micro overlay）；`rig/stage.ts`（`stageExpressionPlan`：狀態通道覆蓋 vs 整體搶佔、
遊玩場、hit-rect 逐幀節流回報、30fps 遲滯降級、Reduced Motion 真靜態）。Presentation Ack 走 runtime 的 presentation receipts。
**未做**：Fullscreen／OS 勿擾偵測（無零依賴方案）。

## 9. VS Code Pets 基準功能逐項驗收

| 基準 | 狀態 | 證據 |
|---|---|---|
| 自主散步／奔跑／停下／坐下／趴下／休息／睡覺 | 已有（遊玩場內） | playfield.test、rig.test |
| 游標靠近／進入／離開／點擊／連點反應 | 已有（看向游標、hover 氣泡、被點／連戳） | regressions-phase7.test（gaze latency 20/20） |
| Hover 短氣泡不過度打擾 | 已有（>700ms、冷卻 45s、可關） | gameFeel/attention 測試 |
| 投擲玩具：預備／追逐／撲抓／撿回／拒絕歸還 | 已有 | playfield.test |
| 滑鼠決定方向／速度／落點 | 已有（拖曳軌跡速度） | playfield.test |
| 多角色／使魔架構 | 已有（最多 3） | playfield.test |
| 互相注意／打招呼／愛心／追逐／玩耍 | 已有（雙向：主角回看回愛心、被追者逃） | regressions-phase7.test |
| 外觀／顏色／名稱 | 已有 | CompanionPage、e2e |
| 個別顯示／隱藏／移除 | 已有 | CompanionPage |
| 匯入／匯出 | 已有（設定 JSON 白名單） | settingsTransfer 測試 |
| 場景（巢穴／工作桌／窗台／夜間），透明模式正常 | 已有 | stage.ts |
| Roll Call 人話 | 已有 | presentation.rs 白名單＋e2e |

超越基準：拖曳懸空／四種落地／檔案接取（檔名＋去向；**大小／類型／可讀 Agent 未做**）／Agent 工作演出／硬體上線離線演出／
長時間無操作休息／變體與冷卻／各項可分別關閉——已有；**坐視窗邊緣／螢幕邊緣探頭／躲視窗後、其他視窗事件反應未做**。

## 10. 玩具與 2D 物理

`playfield.ts`：6 種玩具（毛球／紙團／紙飛機／光點／逗貓棒／trinket），資料模型含位置、速度、重力、碰撞、抓取狀態、擁有者、興趣值、
冷卻、生命週期（TTL 150s、上限 4）；牆／地反彈、摩擦；hazard 抽樣決策。測試 playfield.test 19＋regressions-phase7。

## 11. 玩家點擊、拖曳、投擲、多角色互動證據

單元測試（machine／playfield／stage 逐幀）與 `pnpm perf` 的真狀態改變延遲（抓玩具 8.3 ms、看向游標 8.7 ms）；控制中心 36 表情預覽
截圖（`desktop-companion.png`）；**沒有角色視窗的動畫影片／截圖**（角色視窗只在 Tauri 實機，本輪 Playwright 只覆蓋控制中心）。

## 12. Codex／Claude Code 真 Session 驗收

v0.5 所有 Agent 測試用 fake 子程序 fixture（`fake_claude.sh`／`fake_codex.sh`——真子程序、假模型，絕不動用真額度）：
agents_loop 16、gateway_loop 10、agent-gateway 18、CLI E2E gateway 段。本機真 binaries（claude 2.1.250、codex-cli 0.150.1）
只用於 discovery／schema 產生（`codex app-server generate-json-schema` 核對 `thread/resume` 參數）；**本輪未建立真模型 session**
（v0.4 的真 session 證據見 acceptance-evidence v0.4 節）。

## 13. Agent 狀態到小樞演出的映射

`machine.ts` `agent.session.state`：created→queued；fetched→routing（翻找）；working→working（努力工作，等 Codex／等 Claude 專屬）；
waiting-input／waiting-consent→ask（真相狀態）；claimed-completed→success-claimed（只點頭，無綠勾）；verified→success-verified（綠勾＋慶祝）；
failed→failed；timed-out／expired／**unknown**→unknown；cancelled→誠實清場；closed→idle。iPhone 端 `character.present` 同一階梯，
verified-success 只由人類驗證路徑推送。測試 agent-intent 9＋companion 21。

## 14. Serial、BLE、MQTT Adapter

`crates/interaction-adapter-declarative`：線協定 v1（`protocol.rs`）、Serial（serialport；pty fallback 只在 ENOTTY）、MQTT（rumqttc QoS1）、
BLE（btleplug，macOS/Windows）。每 adapter 十項：Discovery（hardware scan metadata；宣告式 YAML 註冊）、Stable identity（hello.deviceId；
埠／IP／topic 不是身分）、Pairing（配對碼）、Manifest／Schema（YAML command＋params）、Timeout／cancel／reconnect／backoff（含 deadline、
link-reset、not-paired 重握手）、Idempotency（id 環＋nonce 環）、硬限制（韌體＋policy）、Acknowledged／Observed／Verified 誠實區分
（停在 acknowledged）、模擬器分離。測試：adapter crate 42（protocol_honesty 21、mqtt_loop 4、http_loop 6、unit 11）。BLE 無真機。

## 15. ESP32 BOM、接線、韌體與真機證據

`firmware/esp32-companion/README.md`：BOM（ESP32 DevKitC、RGB LED、按鈕、HC-SR04、光敏、DHT22、震動馬達、SG90、蜂鳴器）、接線圖、
Flash 步驟、協定表、硬限制表、數值參數型別規則、測試表 #1–#19。韌體 `compile.sh`／`--ble` 兩組態 **0 error**（938 KB／1188 KB）。
**真機證據：無**（本環境無實體裝置）。

## 16. iPhone App、配對、感測、動器與 BLE Gateway

桌面 `mobile.rs`（TLS wss、指紋、配對 HMAC、每機 token、heartbeat、撤銷即斷線、facts 過濾、consent、estop 停感測、感測不靜默、
綠勾人類專屬）；iOS SwiftUI app（配對／感測／動器／角色／BLE gateway）。**模擬器**驗收：配對閉環、Keychain 重連、character 動器收據、
撤銷、XCTest 19/19；第二輪復測：撤銷後 wss 於 **≤0.035 s** 斷線並即時顯示、estop 後桌面 activeSensors 0.064 s 清空且手機感測 0.5 s 內
關閉並顯示原因、`status.bonjour.advertised=true` 且 `dns-sd -B` 可見（`ios-sim-08..10.png`；`apps/interaction-ios/README.md`）。
**真機：無**。桌面 BLE gateway 只有 scan。附帶修正：測試模式 Runtime 不再把 Bonjour 記錄廣播到實體區網。

## 17. 權限、資料範圍、風險分級與 Emergency Stop

L0–L4（`riskTier.ts`）：L0 純呈現不逐次詢問、不進待決定；L1 一次設定；L2 首次或範圍改變詢問；L3 明確授權＋硬限制（policy clamp，
extra 不能放寬）；L4 短效授權＋持續指示（activeSensors／tray／角色視窗，含 iPhone 麥克風）。Emergency Stop：頂欄／tray／角色選單／CLI
觸發，解除走同意與安全；estop 停本機感測＋iPhone 感測＋角色語音／transient＋硬體 stop-all（無 ack 誠實 Err）；重啟不自動恢復高風險。
Token：human／agent／session 分離；`/v1/mobile/*`、verify、providers test human-only（api_e2e 403 測試）。

## 18. 記憶與知識 UI 簡化及資料相容性

一般模式三區（關於我的記憶／小樞學會的知識／素材與來源）＋規格人類文案；技術 tab 進階才顯示；角色互動記憶在小樞頁（有界、
不推論人格、不進知識庫）；後端 10 層／知識圖譜未動（memory_loop 10、knowledge_loop 14、curator_loop 7）。到期記憶只能刪除。

## 19. Migration 與向後相容

舊 tab id → 新入口對照（守門測試）；v1/v2 sprite pack 相容層；DesktopPrefs 新欄位皆有預設＋bounded；ProviderDescriptor.detail 以 JSON 字串
承載 tested 證據（純文字舊值照舊）；`AgentSessionState::Unknown` 新增（golden 未漂移）；Bonjour 服務名改變（舊 App 需更新 Info.plist）。

## 20. 效能量測、事件延遲、FPS、記憶體與 bounded queue

見 §acceptance-evidence 表：drawRig 0.100 ms、全舞台 0.240 ms、輸入→下一幀 8.3／8.7 ms、heap 9.5 MB 量化、玩具 cap 4、3 天數值穩定；
30fps 降級遲滯政策；OUTBOUND_QUEUE 32／PENDING 有界。**未量**：Tauri WKWebView 實機 FPS。

## 21. 所有測試命令及 passed／failed／skipped

見 `docs/acceptance-evidence.md` v0.5 表：Rust 426/0/0、Tauri 8/0、vitest 319/0（21 檔）、build ✓、CLI E2E 63/0、Playwright 24/0、
golden 5、iOS typecheck 0/0、iOS XCTest 19/0（模擬器）、韌體 0 error×2、pty 對測 44/44＋30/30、fmt／clippy 乾淨（workspace＋Tauri）。

## 22. Desktop、390px、角色動畫、硬體與 iPhone 真機畫面

`docs/assets/v05-evidence/`：desktop-*.png 29、narrow-*.png 28（Playwright 重產）、shu-maid-rig-sheet.png、ios-sim-01..10.png（模擬器）。
**無**：角色視窗動畫影片、硬體照片、iPhone 真機畫面。

## 23. 無法執行或驗證項目與具體原因

ESP32 真板／iPhone 真機／BLE 真裝置（本環境沒有實體）；Fullscreen／OS 勿擾偵測（需新平台依賴）；跨視窗桌面漫遊（需 OS 視窗層 API）；
Tauri WKWebView 實機 FPS（無自動化路徑）；真模型 Agent session（不動用真額度）；Rosetta（無法跑 arduino-cli 內建 x86 ctags → 以 shim 取代）。

## 24. 替代檢查、仍存在風險與完整環境重跑命令

替代檢查：pty 模擬器＋從 .ino 逐字抽出的邏輯在桌面編譯執行；iOS 模擬器閉環；fake agent 子程序。
風險：MQTT rumqttc 內部佇列 deadline；provider 停用後不可逆；配對期 DoS；磁碟（target/ 30 GB）。
重跑：`cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`；
`cd apps/interaction-desktop && pnpm typecheck && pnpm test && pnpm build && pnpm test:e2e && pnpm perf`；
`cargo test --manifest-path apps/interaction-desktop/src-tauri/Cargo.toml`；`./scripts/v03-cli-e2e.sh`；
`./firmware/esp32-companion/compile.sh [--ble]`；iOS：`DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer xcrun swiftc -typecheck …`
（見 `apps/interaction-ios/README.md`）；對抗審查：`Workflow adversarial-review-v05`。

## 25. 下一階段建議

1. 取得一片 ESP32 DevKitC 依 BOM 接線，跑 README 測試表 #1–#19，補真機證據；BLE 真裝置驗證 adapter。
2. iPhone 真機：haptic／CoreMotion／BLE gateway／背景行為；App 端加 NWBrowser Bonjour 瀏覽；桌面補 BLE connect/gatt 送端。
3. 角色空間：多視窗／桌面錨點、螢幕邊緣探頭；Fullscreen 偵測（評估 objc2-app-kit 依賴成本）。
4. 硬體 Observed：讓裝置 state 回帶 last actionId 或 host 端關聯（協定 v2），讓硬體動作能走到 verified。
5. 真模型 session 驗收（在使用者授權額度下）與 codex `thread/resume` 真實行為確認。
6. 提交：Phase 6＋7 為一次或分段 commit（不含 scratch 產物），`release.sh v0.5.0` 前重生 golden。
