# v0.5 恢復矩陣（新 Session 事實恢復，2026-08-28）

> 本文件由 10 位獨立審計 agent 逐條對照規格《adaptive-interaction-v05-core-experience-prompt.md》產生（每列附 file:line 證據），並經完整性審查員裁決矛盾。
> **不以「檔案存在」標示完成**：已接線＝真的從 CLI／HTTP／Tauri／UI 可達；測試＝真的有斷言覆蓋；真環境＝模擬器／fixture／程序內 client 一律標「僅模擬器」，沒真機就「未驗證」。
> 原始 JSON（含完整證據字串）在本次 Session scratchpad `matrix/*.json`；本文件為可讀版。

## 0. 基線（本 Session 親跑）

| 項目 | 值 |
|---|---|
| HEAD | `a898996` feat(hardware): v0.5 Phase 5（origin/main 仍在 `d200e2c` v0.4.1；Phase 0–5 為本機 6 個未 push commit） |
| worktree | Phase 6 全部未提交（mobile.rs、mobile_loop.rs、apps/interaction-ios/、CapabilitiesHub/api/transport/lib.rs 等 22 檔 M ＋ 4 個 untracked） |
| 殘留程序 | 無上一 Session 的 agent／背景程序在寫入（另一 claude session 只是 cwd 在此 repo；lsof 無寫入 handle） |
| Rust | `cargo fmt --check` 通過；`cargo clippy --workspace --all-targets -- -D warnings` 0 warning；`cargo test --workspace` **349 passed / 0 failed / 0 ignored** |
| 前端 | `pnpm typecheck` 通過；`pnpm test` **138 passed / 0 failed**（14 檔）；`pnpm build` 成功 |
| Tauri | `cargo test`（src-tauri）**4 passed / 0 failed** |
| CLI E2E | `./scripts/v03-cli-e2e.sh` **59 passed / 0 failed** |
| Playwright | `pnpm test:e2e` **24 passed / 0 failed**（36.5s） |
| iOS | 本機**有** Xcode 26.6（DEVELOPER_DIR）；12 檔對 iOS 26.5 SDK `swiftc -typecheck` 0 error（初始 5 warning，已修為 0） |
| ESP32 | arduino-cli 1.5.1 ＋ esp32:esp32 3.3.11：兩組態實際編譯 0 error（`firmware/esp32-companion/compile.sh`） |


## 1. 完整性審查員：頂層缺口排序

| # | 嚴重度 | 規模 | Phase 7 處理 | 缺口 |
|---|---|---|---|---|
| 1 | 🔴 blocker | small | 補齊 | iPhone 撤銷不斷線＋未認證 peer 可偽造 ack/err/ble.result＋配對期在 HMAC 驗證前被 take()（mobile.rs:587-605、:823-843、:703）；並行 session 自己的 ios-sim-06-revoked-plus40s.png 已證實撤銷 40s 後仍「已連線」；CHANGELOG:54／UI 文案／cli main.rs:440「立即斷線」不實 |
| 2 | 🔴 blocker | medium | 補齊 | 四段式「離開」段是死資料：timeline.ts:130-181 不讀 expr.exit，0/36 表情四段齊全（16 無 enter、7 無 loop、31 無 exit）；CHANGELOG:27-28、gap matrix:105、CompanionPage.tsx:848-850、pack manifest「四段式動畫」皆與事實不符。Phase 7 應：播放既有 5 個 exit 段＋rig.test 斷言改嚴＋文件改為誠實「三段＋crossfade」；補齊 36×4 屬大規模製作，列已知限制 |
| 3 | 🟠 high | medium | 補齊 | iphone.mic-level 感測靜默：ingest 不查 consent（mobile.rs:812-822）、不進 active_sensors/事件/tray（sensors.rs 對 iphone 0 命中）、manifest 未宣告 retention=none 致音量值持久化（runtime.rs:573-575）、estop 與桌面 stop_all_sensors 不停手機感測、撤銷/重連無 re-gate——直接違反 CLAUDE.md「感測不靜默」與 §14-8 |
| 4 | 🟠 high | medium | 補齊 | §17「Quiet Hours、Fullscreen 正常」不成立：runtime status() 從不輸出 quietHours（runtime.rs:409-450）→ CompanionApp.tsx:489 與 director.ts:150-156 quiet 路徑在生產不可達；Fullscreen 與 OS 勿擾偵測完全不存在（src-tauri、companion/ 0 命中）；gap matrix:105 仍標 Director「已有」。修法：status() 補 quietHours（小）＋Tauri 端全螢幕偵測（中）；OS 勿擾列已知限制 |
| 5 | 🟠 high | medium | 補齊 | Interaction Director 半接線：react()（director.ts:122）與 REACTION_EXPRESSIONS、scoreEvent/score()、noteFinished() 在 app 零呼叫；CompanionApp.tsx:1001-1003 直接 apply performing 繞過 playable() 防線；presentation cancel/clear-all 只 showBubble(null) 不清 performing（:349-354）；9–12 個表情執行期不可達（peek/lean-in/deadpan/startled-awake/pretend-not-hear/slip-play-cool/caught-slacking/await-player/not-found/block-cursor/sit/sleep）。Phase 7 可接線 react()＋修 cancel（小～中）；個性模型（§4.3）不存在屬大規模，列已知限制 |
| 6 | 🟠 high | medium | 補齊 | §11 記憶與知識 UI 分層整段未做：MemoryKnowledgePage.tsx 一般模式仍 5 個技術 tab（候選/知識收據/Context Bundle），無「角色互動記憶」資料類別（全 repo 0 命中）；§2 L0–L4 分級無標籤、L0 純呈現動作產生 uncertain receipt 計入 Inbox 待決定（activity.rs:148、presentation.rs:993-1014）。Phase 7：技術 tab 收進 advanced＋人類文案（中）、presentation receipt 不計 needs_decision（小）；角色互動記憶 store 屬新功能，列已知限制 |
| 7 | 🟠 high | medium | 補齊 | 硬體閉環誠實性缺口：link_caps.rs:83-85/:252-254 與 lib.rs:788-790 health()/status() 硬編 healthy（斷線仍健康）；LinkReceptor state 無 actionId → 硬體動作永遠停在 acknowledged/uncertain，Observed/Verified 死路（executor.rs:786-875）；serial.rs:41-43 ENOTTY 判斷把任何 io::Other 當 pty 退回檔案 I/O；SerialRawLink/MqttRawLink 在 build() 開埠且 provider disabled/revoke 不關（lib.rs:884-972）；mqtt_loop.rs:222-224 dedupe 只有註解無斷言；hello.caps 收下不用；無「已測試」狀態、掃到的埠無法一鍵配對 |
| 8 | 🟠 high | small | 補齊 | ESP32 韌體從未成功編譯：arduino-cli 與 esp32 core 現已就緒，但 `./firmware/esp32-companion/compile.sh` → exit 1、37 error（原型插入在 struct Link/JsonDocument 之前，ctags-shim 未解）；README:354 自承未編譯；真機驗收（§3.3「不能只停在 metadata scan」、§17「ESP32 能完成實際閉環」）在無實體裝置下不可能，必須明列 |
| 9 | 🟠 high | medium | 補齊 | Agent taxonomy 誠實性：waiting-input 無任何 connector 產生（agent-gateway lib.rs:89）；lease 到期只發 session.stopped 無 taxonomy 事件（agents.rs:455-480）；程序結束無結果報 failed 而非 unknown（gateway.rs:443-455）；Claude system/init 即 working/Active（claude.rs:359-369，工作先於 fetched）；codex app-server fetched 在 stdin 寫入前發出（codex.rs:433-452）；AiPage:224-230 approval 只抓一次、300s 自動拒絕不可見；無 codex app-server fixture、無 verify 403 負向測試 |
| 10 | 🟠 high | small | 補齊 | 精靈與小樞設定的空殼：Onboarding.tsx:317-389 步驟二 agentChoice 從未被 commit() 讀取（選任何選項結果相同）；:214「音效預設關閉」無任何音效偏好支撐（DesktopPrefs/supervisor.rs 無欄位、CompanionApp.tsx:375 收到 sound-play 即播）；§12.2/§5.2-10 氣泡、音效、拖曳三個開關不存在；:287-288 說安靜時段稍後再問但 :110-117 靜默寫入 |
| 11 | 🟠 high | small | 補齊 | 控制中心可用性缺陷（截圖已暴露但前一 session 未察覺）：styles.css:416/530/558 用未定義 --panel、:169 input 硬編深底 → 淺色主題面板/表單不可讀；通知面板 App.tsx:405-451 無 Escape/焦點移入/focus trap（evidence.spec.ts:172-174 以 .catch 遷就）；activity.rs:231-232 pendingCount 在 truncate(limit) 之後計算 → Inbox 計數上限 20 且與 HomePage.tsx:415-427 口徑不一；一般模式外洩 Lease/provider session id/raw JSON/raw status（AiPage.tsx:220,227,369,377-379；HomePage.tsx:343-370；App.tsx:428） |
| 12 | 🟡 medium | large | 列已知限制 | 規格明列但完全未實作且文件未列為已知限制：第 6 種玩具「可拖曳的小物件」（playfield.ts:11 只有 5 種）；WebSocket/HID/Home Assistant adapter（§9.1 第 4–6 項，CHANGELOG Phase 5 未提）；iOS App 端 Bonjour 瀏覽（無 NWBrowser）；Camera/Location receptor、Live Activity、Audio/SFX 動器、Local-network device events；「前往 iPhone」presentation surface；坐視窗邊緣/螢幕邊探頭/躲到視窗後；對視窗開關/下載/測試失敗的語意反應；依速度高度選 4 種落地（一律 wobbly-landing CompanionApp.tsx:780）。Phase 7 應補第 6 種玩具（小）並把其餘明列於 CHANGELOG/acceptance-evidence 已知限制 |
| 13 | 🟡 medium | medium | 補齊 | 文件與事實系統性漂移（docs-claims 審計交白卷）：CHANGELOG（未提交）「無 Xcode」「撤銷立即斷線」「36 表情皆有三段」「ble.scan/connect/gatt」「沒配對過不開網路埠」（mobile.rs:351-352 只要 json 檔存在即開）；gap matrix §9 vs §6/§7 行自相矛盾（Resume/Conversation Provider/iPhone）、「看向游標」「HMAC 配對」；acceptance-evidence.md:233 過時且無 v0.5 節；CLAUDE.md:6 v0.3.0、佈局 scripts/shu/ 路徑；ARCHITECTURE.md/README.md 零 v0.5；skills/ 停在 v0.4；CHANGELOG 無任何測試命令與數字（§15.1 明禁）；apps/interaction-ios/README.md:8-18 已被並行 session 改寫成「模擬器驗收」但其他文件未同步 |
| 14 | 🟡 medium | medium | 補齊 | 回歸與測試覆蓋空洞：無 Phase 6 後完整 `cargo test --workspace` 數字；draw.ts(1246 行)/timeline.ts/stage.ts/RigRenderer 零測試；CompanionApp 五種指標互動 handler 零測試；BLE（adapter 與 App）零測試、Serial/MQTT 真傳輸層 reconnect/timeout 零測試；mobile_loop.rs:6 標頭宣稱「斷線→Disconnected」但無斷言、:186-198 白名單無斷言；無 agent/session token 對 /verify 與 /v1/mobile 的 403 測試；無 estop_engaged 跨重啟測試；無 runtime 端 agent.session.state 發射測試；無 5 入口/單一主人/390px 主要流程守門測試；src-tauri lib.rs 0 測試 |
| 15 | 🟡 medium | small | 補齊 | 效能與誠實性宣稱無可重現量測：gap matrix §9「drawRig 0.452 ms/幀」找不到產生程式；§14 16–100ms 反應、60fps 量測、30fps 降級、記憶體、bounded queue 皆無腳本／測試；renderer.ts:85-89 透明視窗 rAF 每幀重繪無節流、CompanionApp.tsx:298 reduced 旗標只在 boot 設一次 |
| 16 | 🟡 medium | small | 補齊 | 介面可達性缺口：POST /v1/mobile/ble/scan 只在 HTTP（CLI main.rs:435-442、Tauri lib.rs:2122-2124、UI 皆無）；agents resume 無 CLI 旗標（main.rs:364-392）；CapabilitiesHub.tsx:292-299 不顯示手機 permissions（denied/revoked）；桌面對 iphone.* 受器一律以連線判可用（mobile.rs:443-458） |
| 17 | 🟡 medium | small | 補齊 | 資源與生命週期殘留：mobile.rs:866-884 ble scan 逾時不移除 pending_acts；ble.rs:157-167 每次 connect 新 notification task 不取消、無 disconnect；mqtt.rs:59-90 eventloop 無 shutdown；mobile.rs:365 started.swap(true) 在 bind 成功前設定致失敗後永不重試；WS 連線無 idle timeout/伺服器 ping/連線數上限（:639-863）；runtime.rs:823-832 estop 對 6 個 iphone.* actuator 一律計 stopped 且 broadcast 失敗被 let _ 吞（假成功） |
| 18 | 🟡 medium | small | 補齊 | 未提交與並行漂移：Phase 6 全部（mobile.rs、mobile_loop.rs、apps/interaction-ios/、CapabilitiesHub/api/transport/lib.rs 等 22 檔）與 15:08–15:26 並行 session 的 compile.sh、tools/ctags-shim、ios-sim-*.png、Swift 修改（ActuatorCenter/ConnectionManager/SensorCenter/PairingView DEBUG 自動配對入口）皆未 commit；本審計矩陣是 13:5x–15:1x 快照，Phase 7 開工前必須先 commit 並重跑 Find→Verify |

### 審計者矛盾裁決

- 【Xcode／iOS SDK 是否可用】p6-mobile-rs「xcrun iphonesimulator SDK 不存在，無法編譯 iOS App」與 safety-baseline「xcrun 找不到 iphonesimulator SDK（僅 CommandLineTools）」 vs p6-ios-swift「本機有 Xcode 26.6／iOS 26.5 SDK，宣稱不實」。裁決：p6-ios-swift 正確——`ls -d /Applications/Xcode.app` 存在；`xcode-select -p` = /Library/Developer/CommandLineTools（其他兩人因此失敗）；`DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer xcrun --sdk iphonesimulator --show-sdk-path` → iPhoneSimulator26.5.sdk。連帶：CHANGELOG.md（未提交 Phase 6 段）「本環境無 Xcode/iOS SDK——僅 swiftc -parse」與 docs/v05-capability-gap-matrix.md §9「無 Xcode 無法編譯」為不實；apps/interaction-ios/README.md:8-18（15:26 被並行 session 改寫）現宣稱「iPhone 17 模擬器驗收＋XCTest 19/19」——同一 worktree 三份文件互相矛盾，Phase 7 必須統一。
- 【Quiet Hours 對角色是否生效】p3-play §15.2-11「code=yes wired=yes tests=pass sev=low」 vs p2-director §6.1「wired=no sev=high：runtime 從不輸出 status.quietHours」。裁決：p2-director 正確——crates/interaction-runtime/src/runtime.rs:409-450 status() 鍵清單無 quietHours（policy.quiet_hours 只在 :458 內部迴圈使用；human.rs:1246 只在 what-if scenario 報告輸出），CompanionApp.tsx:489 `Boolean(s["quietHours"])` 永遠 false；p3-play 的 pass 是前端 unit test 餵手工 payload。§17「Quiet Hours 正常」不成立。
- 【組合式通道能否真正混合】p3-play §15.2-3「不同通道動畫混合 code=yes」 vs p2-rig §6.2「允許組合 code=partial sev=high」與 p2-director。裁決：p2-rig 正確——apps/interaction-desktop/src/companion/rig/stage.ts:313-324 只選一個 effective expression（machineAnim 非 idle 一律整體覆蓋），:327-338 僅在移動時把 legPhase/bodyBob/hairSway/tailSway 疊加到同一組 params；規格例「趴著＋核心顯示 Agent 工作中」做不到。p3-play 所謂「混合」只是參數層 overlay。
- 【帶回（return）分支是否有測試】p2-rig §7.3「追逐/撲毛球/撲空/抱住/帶回/不想歸還 tests=pass」 vs p3-play「帶回分支未測」。裁決：p3-play 正確——playfield.test.ts:151-162 只把 mode:"return" 當前置條件驗證「非 ambient 時放下玩具」，沒有任何斷言驗證 return 模式把玩具帶回；CharPlayMode "carry" 從未被賦值（playfield.ts:32）。
- 【撤銷是否立即斷線】p6-mobile-rs「blocker：撤銷後已連線手機仍可持續送 observation/status 並被 ingest」 vs p6-ios-swift「§15.4 配對、撤銷 code=yes wired=yes」與 CHANGELOG.md:54「撤銷立即斷線」、CapabilitiesHub.tsx:311 文案。裁決：p6-mobile-rs 正確——mobile.rs:587-605 mobile_revoke 只 `conns.remove(device_id)`；handler 迴圈 :812-843 的 `authed` 與自身 `out_tx` 不受影響，observation/status 持續 ingest 直到 TCP 自行關閉。並行 session 自己的證據截圖 docs/assets/v05-evidence/ios-sim-06-revoked-plus40s.png（15:26）顯示撤銷 40 秒後 iPhone 仍「已連線」，機器證據反證了 CHANGELOG 的宣稱；mobile.rs mtime 13:51 至今未改。
- 【韌體能否被檢查】p5-hw「本機無 arduino-cli/platformio，無法 typecheck 840 行 .ino」。裁決：審計時正確、現已過時——arduino-cli 1.5.1 於 15:08 安裝（symlink 時間戳），esp32 core 3.3.11 與 6 個函式庫齊備；但親跑 compile.sh → exit 1、37 error（原型插入在 struct Link/JsonDocument 宣告之前）。結論從「無法驗證」變成「驗證失敗」，firmware/README.md:354 的自述仍成立。
- 【Golden schema 是否過期】p5-hw「schemas/ golden 未含 expectedDeviceId/serial/mqtt/ble」與 p4-ai「openapi.json 無 /v1/agent-sessions、無 agent.session.state」以缺口口吻陳述。裁決：非回歸缺口——`cargo test -p interaction-e2e` golden.rs 5/5 通過，golden 依設計只涵蓋 AI tool export（tools.*.json、openapi 子集）與 recipe.schema.json；human-only 路由與 adapter YAML 本來就不在 golden 內。應記為設計決定而非待修項。
- 【L0 短氣泡】p2-director §8.1「眼睛、耳朵、尾巴、眨眼、姿勢及短氣泡 sev=none」 vs p3-play §5.1-3「Hover/靠近短氣泡 code=no sev=medium」。裁決：兩者各對一半——氣泡機制存在（runtime 命令與文字輸入會 showBubble），但 hover/靠近觸發的短氣泡完全沒有；p2-director 的 none 忽略了 §5.1-3，p3-play 缺口成立。
- 【Behavior Intent priority vs Attention/Utility】p3-play §15.2-1「tests=pass sev=none」 vs p2-director「Attention Manager + Utility Scoring 死碼 sev=high」。裁決：不矛盾——machine.ts:47-61 靜態優先度階梯有測試且生效；behavior.ts:182 scoreEvent 與 director.ts:95 score() 在 app 內零呼叫。規格 §6 管線的 Attention/Utility 環節確實未接線，兩列同時成立。
- 【安靜時段／每小時上限在精靈外有無主人】p1-ia §12.6「安靜時段、每小時上限、高風險核可門檻在精靈外沒有主人」 vs CHANGELOG Phase 1「安靜時段全部集中「小樞」頁」。裁決：p1-ia 錯——apps/interaction-desktop/src/pages/CompanionPage.tsx:637-706 有「主動程度與安靜時段」區段（:703-706 可編輯 quietHours），SettingsPage.tsx:98 只留摘要；maxPerHour/requireApprovalAt 同時出現在 CompanionPage.tsx、AutomationsPage.tsx 與 Onboarding.tsx（前兩者是否為第二份相同開關需 Phase 7 守門測試判定）。p1-ia 的「精靈是第二寫入點」屬初始設定流程，不違反 §12.6。
- 【CLI／HTTP／Tauri 共用同一服務 vs 各介面可達】safety-baseline「共用同一 application service gap=none」 vs p6-mobile-rs「POST /v1/mobile/ble/scan 只在 HTTP（CLI main.rs:435-442、Tauri lib.rs:2122-2124、UI 皆無）」與 p4-ai「resume 無 CLI 旗標（main.rs:364-392）」。裁決：各自成立——不變量（同一 runtime service）未違反；但規格「已接線＝從 CLI/HTTP/Tauri/UI 可達」對 ble/scan 與 resume 不成立，應列為小型缺口。
- 【docs-claims 審計者交白卷】無人系統性核對文件宣稱。抽查裁決：CLAUDE.md:6「目前版本 v0.3.0」 vs Cargo.toml:26 = 0.4.1；docs/ARCHITECTURE.md 與 README.md 對 v0.5 零提及；docs/acceptance-evidence.md:233 仍寫「WS/MQTT/Serial/BLE 誠實拒絕（僅 HTTP/SSE 實作）」；gap matrix §9「游標互動→已有（…看向游標…）」 vs behavior.ts:112-114 明言不讀游標、params.ts 只有 pupilX/pupilY 無 target（p2-rig/p3-play 一致）；gap matrix §9「配對儀式 HMAC/pair」 vs protocol.rs 明文比對（p5-hw）；CHANGELOG/gap matrix「36 表情皆有進入/保持/小循環」 vs 腳本統計 16 無 enter、7 無 loop、31 無 exit（p2-rig）。文件宣稱普遍高於程式碼事實。

### 審計未覆蓋而由審查員補查的需求

- **前言（題詞開頭）** 保留使用者現有修改；不得以 reset、checkout 或覆蓋方式破壞未提交內容 → 成立：`git reflog \| head -15` HEAD@{0..6} 全為 commit（65d3e30 Phase 0 → a898996 Phase 5），無 reset/checkout 條目；Phase 6 工作（mobile.rs、mobile_loop.rs、apps/interaction-ios/、CapabilitiesHub/api/transport 等 22 檔 +1248/-79）仍在 worktree 未提交，且另一並行 session 於 15:08–15:26 持續新增 firmware/esp32-companion/compile.sh、tools/ctags-shim、docs/assets/v05-evidence/ios-sim-*.png 並改寫 apps/interaction-ios/README.md——審計矩陣是 13:5x–15:1x 的快照，已與現況漂移
- **§1 產品重新定位** 本輪不得繼續擴張新的治理概念；安全功能不得成為一般 UI 主角、不得讓低風險角色動畫因 Consent/Receipt 失去即時性 → 部分成立：CHANGELOG.md [Unreleased] 新增概念只有裝置/iPhone 配對碼＋device token（§9.1/§10.1 要求的連線安全）與 §3.2 要求的 human-only verify 路由，無新 consent 層級或治理物件；但 L0 純呈現動作仍走 receipt→watchdog uncertain→activity.rs:148 `needs_decision: matches!(status, "uncertain"\|"blocked")` 進右上 Inbox（p1-ia 已列），一般 UI 仍有 5 個記憶/知識技術 tab（safety-baseline 已列）
- **§15.1 全部既有回歸 — Rust fmt** cargo fmt --check → 親跑 `cargo fmt --check` → exit 0
- **§15.1 全部既有回歸 — Clippy -D warnings** cargo clippy --workspace --all-targets -- -D warnings → 親跑 → exit 0、0 warning（scratchpad/clippy.log）。注意 Cargo.toml:21 `exclude = ["apps/interaction-desktop/src-tauri"]`，Tauri crate（含新增 mobile 命令 lib.rs:2122）不在 workspace clippy 範圍，本輪無人對它跑 clippy
- **§15.1 全部既有回歸 — Workspace tests** cargo test --workspace 並列出 passed/failed/skipped → 無人跑完整 `cargo test --workspace`（各審計者只跑 mobile_loop 3、hardware_loop 1、presentation 6/12/5、adapter-declarative 19、events 2）。我補跑 `cargo test -p interaction-runtime --test memory_loop --test knowledge_loop --test curator_loop` → 10/14/7 passed 0 failed；`cargo test -p interaction-e2e` → golden.rs 5 passed。Phase 6 後整個 workspace 的總數字仍不存在（gap matrix §0 的 336 是 Phase 0 基線 d200e2c）；CHANGELOG [Unreleased] 無任何數字
- **§15.1 全部既有回歸 — Desktop Tauri tests** src-tauri crate 測試 → 親跑 `cargo test --manifest-path apps/interaction-desktop/src-tauri/Cargo.toml` → 4 passed / 0 failed / 0 ignored（tray.rs 1、supervisor.rs 3）；lib.rs（含 toggle_companion_window:1433、companion_hit_rect:1565、mobile 命令:2122）與 main.rs 各 0 個 #[test]
- **§15.1 全部既有回歸 — Frontend build** pnpm build → 親跑 `pnpm build` → exit 0，✓ built in 1.31s（index-nUPZeFKC.js 450.15 kB / gzip 146.94 kB）
- **§15.1 全部既有回歸 — Golden schema／tool export** golden schemas 與 tool export 不得漂移 → `cargo test -p interaction-e2e` → tests/golden.rs 5 passed（golden_tool_exports、golden_recipe_schema、scenario_j_cross_platform_consistency、scenario_k_no_mcp_dependency、versions_are_in_sync）。schemas/ 最後異動 d200e2c（v0.4.1）且測試通過＝生成器輸出未變；p4-ai「openapi.json 無 /v1/agent-sessions」與 p5-hw「golden 無 serial/mqtt/ble 欄位」屬設計（golden 只涵蓋 AI tool export 與 recipe schema，不含 human control-plane 路由與 adapter YAML），不是回歸缺口
- **§15.1 全部既有回歸 — Character Pack 與 malicious archive tests** Character Pack 驗證與惡意封存測試回歸 → 存在且通過（含於 vitest 138/138）：apps/interaction-desktop/src/test/companion.test.ts:111 `rejects path traversal in the sheet reference`；packs.test.ts:29-43 惡意 persona pack 不得覆寫固定安全文案。沒有 zip/archive 匯入功能（docs/acceptance-evidence.md 已知限制 #7「第三方 pack zip 安裝流程未做」），故「malicious archive」只有驗證器層級測試；新 rig pack（schema 2.0，renderer.ts:93-117 只載 palette）無對應惡意輸入測試
- **§11 記憶與知識重新分層（首句）** 保留現有 10 層記憶與知識圖譜後端 → 成立：memory_loop 10/10、knowledge_loop 14/14、curator_loop 7/7 通過，後端未被 v0.5 破壞（UI 簡化未做由 safety-baseline 涵蓋）
- **CLAUDE.md 工作規則／§8.3** Skill 更新後用 interact-ai self install-skill 重裝；跨 AI Skill 應反映新能力（agent verify、mobile、serial/mqtt/ble 硬體） → 缺：skills/ 最後 commit 7cc2abd（v0.4，`git log -1 -- skills/`）；references/api.md:84-88 只有 v0.4 agent-sessions 端點，無 /verify、無 /v1/mobile/*、無 link 傳輸 adapter；api.md:113 仍寫 `/v1/hardware/scan — metadata-only`。SKILL.md 對 serial/mobile/iphone/mqtt 0 命中
- **§18-19／§6.4** Migration 與向後相容 → 存在：crates/interaction-storage/src/lib.rs:58-62 `store.migrate()`；apps/interaction-desktop/src/App.tsx:64-79 `LEGACY_ANCHORS` 把舊 9 頁 tab id 折疊到新 5 入口（CHANGELOG Phase 1 宣稱相容）；v1/v2 sprite pack fallback 有測試（p2-rig）。未見 LEGACY_ANCHORS 對應表的單元測試；HomePage.tsx:134 仍導向孤立 legacy tab `responses`（p1-ia 已列）
- **§16 每個 Phase 都必須提供真實畫面或機器證據** 逐 Phase 證據 → 不均：docs/assets/v05-evidence/ 有 Phase 1 desktop-*/narrow-*.png（Chromium 對真 daemon）、Phase 2 shu-maid-rig-sheet.png（headless）、Phase 6 ios-sim-01..07.png（15:26 新增、未追蹤、iOS 模擬器）；Phase 3（玩具/遊玩場）、Phase 4（真 Codex/Claude session）、Phase 5（硬體）沒有任何畫面或機器輸出檔，只有 CHANGELOG 文字；docs/acceptance-evidence.md 最後標題仍是 :373 v0.4 closing audit，無 v0.5 節
- **§2 安全底線 — Secret 不寫入 log** 配對碼／token／密碼不得進 log → 成立：`grep info!/warn!/debug!/error!` 於 crates/interaction-runtime/src/mobile.rs 與 crates/interaction-adapter-declarative/src/*.rs 對 code/token/secret/pair/password 0 命中
- **§9.2 官方 ESP32 參考裝置（可實際製作的韌體）** 韌體至少可編譯（compile check） → p5-hw 審計時 arduino-cli 不存在；現在 /opt/homebrew/bin/arduino-cli 1.5.1（15:08 安裝）＋esp32:esp32 3.3.11＋ArduinoJson/PubSubClient/DHT/ESP32Servo/NimBLE 皆已裝，repo 新增未追蹤 compile.sh 與 tools/ctags-shim。親跑 `./firmware/esp32-companion/compile.sh` → exit 1、37 個 error（sketch_merged.cpp:96 'Link' was not declared、:377 'JsonDocument' was not declared——Arduino 原型自動插入位於 struct/typedef 之前，經 ctags-shim 仍失敗）；README.md:354 仍寫「尚未在真實 ESP32 硬體上編譯與驗證」。目前「無法檢查」與「已編譯」皆不成立
- **§18-20 效能量測（記憶體）** 事件延遲、FPS、記憶體與 bounded queue 量測 → repo 內無任何記憶體量測；gap matrix §9 只有 drawRig 0.452 ms/幀（safety-baseline 確認 scripts/、e2e/、src/test/ 找不到產生該數字的程式）；無 16–100ms、60/30fps、memory 的可重跑腳本
- **§16 Phase 7／CLAUDE.md 對抗審查** 以 workflow 跑 find→independent verify 對抗審查 → 工具存在但未追蹤、未執行：.claude/workflows/adversarial-review-v05.js（meta: 8 維度 Find → Verify）；本輪 10 位審計者矩陣即其 Find 階段的替代，Verify 階段尚未發生

## 2. 逐條矩陣


### Phase 1 控制中心 IA／三步精靈／§2 風險分級（21 列）

| Phase | 需求 | 程式存在 | 已接線 | 測試通過 | 真環境驗證 | 缺口 | 嚴重度 |
|---|---|---|---|---|---|---|---|
| 1 | §12 控制中心 9 個一級入口縮為 5 個（現在／小樞／工作／連接與權限／更多），保留進階技術入口 | 是 | 是 | 部分 | 僅模擬器 | none（唯一證據為瀏覽器 Playwright 對真 daemon 的截圖 docs/assets/v05-evidence/desktop-*.png 2026-08-28 15:03，非 Tauri 實機） | 無 |
| 1 | §12.1 現在：系統是否正常／感測器運作／需要決定／小樞・Agent・硬體正在做什麼／最近一次已驗證結果／失敗・未知・離線例外；不重複權限地圖、全部 Session、歷史、設定 | 部分 | 是 | 部分 | 僅模擬器 | 首頁無「硬體現在正在做什麼」（NowStrip 無硬體卡）；無「失敗／未知／離線例外」清單（只有 estop/disconnected 全域 banner App.tsx:461-488）；小樞卡只顯示視窗狀態非「正在做什麼」；「最近一次互動」顯示最近 receipt 而非最近『已驗證』結果 | 🟡 medium |
| 1 | §12.2 小樞：外觀／女僕裝／顏色／名字；安靜・自然・活潑；玩耍・游標・靠近・氣泡・音效・拖曳・桌面移動；主動對話模式與頻率；安靜時段；表情與動作預覽；角色匯入／匯出 | 部分 | 部分 | 部分 | 未驗證 | 氣泡、音效、拖曳三個開關不存在（無偏好欄位、無 UI）；動作預覽只有靜態表情格；匯入/匯出是設定 JSON 非角色；Tauri 專屬設定路徑無任何自動測試覆蓋 | 🟡 medium |
| 1 | §12.3 工作：Codex／Claude Code Provider；工作階段與任務；自動互動；waiting／approval／cancel／resume；成本、時間與資料範圍 | 是 | 是 | 部分 | 僅模擬器 | 一般模式暴露技術術語：AiPage.tsx:220「provider session xxxxxxxx…」、:227「Lease 至」、:369 raw m.kind、:377-379 訊息 body 原始 JSON 直接顯示；HomePage.tsx:343-344,370「租約」、:367-368 raw dataScope/toolScope 字串 | 🟡 medium |
| 1 | §12.4 連接與權限：iPhone；ESP32／Arduino；BLE／Serial／MQTT／網路裝置；感測器與動作能力；Consent、資料去向、測試與立即停止 | 部分 | 是 | 部分 | 僅模擬器 | 無 ESP32/Arduino/Serial/MQTT 的裝置導向設定流程（配對碼、測試、立即停止皆無專屬 UI，僅通用掃描列表）；MQTT 在 UI 完全不存在；Provider 卡在一般模式顯示 lifecycle 狀態（discovered/unpaired/paired/installed :50-63）、trustLevel 原始值 :204、受器/動器/工具計數 :233-235、capability.id 原始 id :169 | 🟡 medium |
| 1 | §12.5 更多：記憶與知識／Activity 歷史／一般設定／備份與更新／進階功能 | 是 | 是 | 部分 | 僅模擬器 | 「備份與更新」實質只有版本字串＋跳轉；SettingsPage.tsx:84 文案仍引用舊頁名「能力與裝置」「隱私與安全」（新 IA 已無此頁名） | 🟢 low |
| 1 | §12 Activity／Confirm 預設為右上角 Inbox 不佔一級頁；Emergency Stop 固定保留在頂部、Tray、角色快捷選單與 CLI | 是 | 是 | 部分 | 僅模擬器 | 通知面板無鍵盤支援（App.tsx:405-451 無 Escape/焦點移入/focus trap；evidence.spec.ts:172-174 以 .catch 容忍 Escape 無效）；淺色主題下面板文字不可讀（styles.css:558 使用未定義的 --panel 變數退回 #1c2331 深底，文字沿用淺色主題深字；證據 desktop-inbox.png 標題與項目名不可見）；:428 顯示 raw status 字串（截圖見「candidate」） | 🟠 high |
| 1 | §12.6 每項設定只有一個主人：小樞表現→小樞；AI 工作→工作；觸發流程→工作/自動互動；可讀取操作→連接與權限；資料保存→記憶與知識；語言視窗啟動備份版本→設定；其他頁只留摘要＋前往，不可有第二份相同開關 | 部分 | 是 | 無 | 僅模擬器 | 精靈是第二個寫入點且部分設定（安靜時段、每小時上限、高風險核可門檻）在精靈外沒有主人；Agent 對話歸屬分散在工作頁與小樞頁；無回歸測試守住單一主人 | 🟡 medium |
| 1 | §13 首次設定精靈由七步縮成三個主要步驟（認識小樞／要讓小樞幫忙工作嗎／安全預設），其他採漸進式詢問 | 是 | 是 | 通過 | 僅模擬器 | 步數符合；但「漸進式詢問」無機制：硬體/iPhone/安靜時段沒有『第一次需要時再問』的觸發邏輯，安靜時段反而在 commit 靜默套用（:112-114） | 🟢 low |
| 1 | §13 步驟一 認識小樞：顯示／不顯示；預覽正式 Q 版貓娘女僕；安靜／自然／活潑預設自然；玩耍與游標互動預設開啟；音效預設關閉或低音量 | 部分 | 部分 | 部分 | 僅模擬器 | 「音效預設關閉」是無程式碼支撐的文案宣稱（沒有音效開關、沒有預設值）；玩耍/游標預設開啟只是隱含預設，精靈未呈現；預覽只驗證存在不驗證成功繪製 | 🟠 high |
| 1 | §13 步驟二 要讓小樞幫忙工作嗎：連接 Codex／Claude Code／兩者／稍後；只做 Discovery／登入狀態檢查，不自動授權工作區寫入 | 部分 | 部分 | 部分 | 僅模擬器 | 四選一是空殼控制項（placeholder）：使用者選「用 Codex 幫忙」與「稍後再說」結果完全相同；不授權寫入這點成立（正因為什麼都沒做） | 🟠 high |
| 1 | §13 步驟三 安全預設：麥克風／攝影機／位置預設關閉；外部裝置與實體動作首次使用詢問；Agent 寫入每個 workdir 明確確認；主動對話預設「必要時」；顯示如何暫停與 Emergency Stop；提供「進一步自訂」但不強迫 | 部分 | 是 | 通過 | 僅模擬器 | 未顯示「如何暫停」；無「進一步自訂」入口；『外部裝置首次使用詢問』僅文字，由 consent.required 機制間接成立；同一步驟靜默套用安靜時段與每小時上限（未顯示給使用者） | 🟡 medium |
| 1 | §15.5 UI 測試：5 個主要入口 | 是 | 是 | 部分 | 僅模擬器 | 無元件層測試渲染 Shell 並斷言恰好 5 個 nav-item | 🟢 low |
| 1 | §15.5 390px 所有主要流程可達 | 是 | 是 | 部分 | 僅模擬器 | 測試只覆蓋導覽與捲動，未在 390px 走完精靈、建立工作階段對話框、授權對話框、estop 解除等『主要流程』；本人未執行 | 🟢 low |
| 1 | §15.5 鍵盤與 focus trap | 部分 | 是 | 部分 | 僅模擬器 | Tab focus trap 無測試；全域搜尋與通知面板無 focus trap；通知面板 Escape 無效 | 🟡 medium |
| 1 | §15.5 Inbox 待確認數量 | 部分 | 是 | 部分 | 僅模擬器 | pendingCount 受 limit 截斷（後端計算順序缺陷）；同畫面兩個「待決定」計數口徑不同；無測試覆蓋 >20 筆情境 | 🟠 high |
| 1 | §15.5 每項設定只有單一 canonical owner（測試） | 部分 | 是 | 無 | 僅模擬器 | 缺守門測試；精靈與工作頁/小樞頁仍有重疊寫入點 | 🟡 medium |
| 1 | §15.5 三步首次設定（測試） | 是 | 是 | 通過 | 僅模擬器 | none（api 全 mock；真 daemon 路徑只在未親跑的 e2e） | 無 |
| 1 | §15.5 裝置導向的人類語言 | 部分 | 是 | 部分 | 僅模擬器 | 部分能力卡為英文目錄字串；provider 卡混雜技術欄位 | 🟢 low |
| 1 | §15.5／§2 一般模式不暴露不必要的 UUID、YAML、Token、Lease、Provider lifecycle；底層保留 Audit 但不淹沒一般使用者 | 部分 | 是 | 無 | 僅模擬器 | 多處 Lease／Provider lifecycle／UUID／原始狀態字串在一般模式可見；缺守門測試 | 🟡 medium |
| cross | §2 安全 UI 改成風險分級 L0–L4（L0 純呈現不逐次詢問不產生干擾性 Receipt UI／L1 一次設定／L2 首次或範圍改變詢問／L3 明確授權＋強度時間硬限制／L4 每次或短效授權＋持續指示） | 否 | 否 | 無 | n/a | 無 L0–L4 分級呈現；L0 純呈現動作會產生 uncertain receipt 並計入待決定，直接違反「不產生干擾性 Receipt UI」；L3 硬限制未在一般 UI 呈現 | 🟠 high |

**診斷（TODO／placeholder／mock 冒充／hard-coded success／空函式／跳過測試／警告）：**

- [placeholder 控制項] Onboarding.tsx:317-389 步驟二的四選一 agentChoice 只存 draft（:29,:46,:220），commit() :91-150 從未讀取，選任何選項結果相同。
- [無程式碼支撐的宣稱] Onboarding.tsx:214「音效預設關閉」——DesktopPrefs（desktop.ts:30-46、src-tauri/src/supervisor.rs:100-150）無音效偏好；CompanionApp.tsx:375 收到 runtime sound-play 即播放，無使用者開關（§12.2 氣泡/音效/拖曳三個開關皆不存在）。
- [文案與行為矛盾] Onboarding.tsx:287-288 說「安靜時段…會在第一次需要時再問你」，但 EMPTY_DRAFT :37-39 quietEnabled:true 且 commit :110-117 靜默寫入 quietHours 22:00-08:00、channelLimits maxPerHour 6、requireApprovalAt；requireApprovalAt 在精靈外沒有任何頁面可調整。
- [後端計數缺陷] crates/interaction-runtime/src/activity.rs:231-232 pendingCount 在 items.truncate(limit) 之後計算；App.tsx:281 以 limit:20 查詢 → 右上「通知 N」上限 20 且會漏算較舊的待決項。HomePage.tsx:415-427 NowStrip 另用不同口徑計算「待我決定」，同畫面兩個數字可不一致。
- [淺色主題不可讀，證據截圖已暴露但前一 Session 未察覺] styles.css:169 `input, select, textarea { background:#10141a; color:var(--text) }` 硬編深底，淺色主題文字 #18212c 落在深底上；styles.css:416,530,558 `.bottom-nav/.search-panel/.notification-panel` 使用從未定義的 `--panel` 變數退回 #1c2331。docs/assets/v05-evidence/desktop-inbox.png、desktop-global-search.png、desktop-work.png、desktop-companion.png、desktop-more.png 均可見 select/input/面板文字不可讀。
- [L0 違反] presentation 動作在角色視窗未 ack 時由 watchdog 標 Uncertain（presentation.rs:993-1014），activity.rs:148 將 uncertain 視為 needs_decision → 純呈現動作（氣泡/姿勢）會出現在右上 Inbox 待決定與活動歷史（ActivityPage.tsx:21 未過濾）。
- [鍵盤/focus] App.tsx:405-451 通知面板 role=dialog 但無 Escape、無焦點移入、無 trap；GlobalSearch.tsx:287 無 aria-modal 與 Tab trap；Dialog.tsx:33-51 Tab 循環邏輯零測試；e2e/evidence.spec.ts:172-174 以 `.catch(() => {})` 容忍 Escape 無效（測試遷就缺陷）。
- [一般模式術語外洩] AiPage.tsx:220,227,369,377-379（provider session id、Lease、raw JSON）；HomePage.tsx:343,367-370（租約、raw scope）；ActivityPage.tsx:272-280 與 App.tsx:428（raw status/kind 字串，截圖見「candidate」）；GlobalSearch.tsx:174 updateId UUID；CapabilitiesHub.tsx:185-236 provider lifecycle/trustLevel；無任何測試守門。
- [測試警告] 親跑 `pnpm test`：14 files / 138 passed / 0 failed / 0 skipped（15.11s）；onboarding.test.tsx 6 次 stderr『Not implemented: HTMLCanvasElement getContext』——PackPeek 預覽在 jsdom 一律走失敗分支，角色預覽繪製從未被測試。`pnpm typecheck` exit 0。未跑 e2e/Playwright（依規則禁跑）。
- [未提交變更] CapabilitiesHub.tsx:255-352 MobileSection（iPhone 配對 UI）、api.ts、transport.ts 為工作樹 M 狀態，不在任何 commit（最近 commit a898996 只到 Phase 5）。
- [文件缺口] docs/acceptance-evidence.md 無 v0.5 章節（最後為 v0.4 closing audit）；CHANGELOG.md [Unreleased] v0.5 段落沒有任何測試命令與 passed/failed 數字（§15.1 要求），僅 v0.4 段有數字。
- [過時文案] SettingsPage.tsx:84 仍引用舊頁名「能力與裝置」「隱私與安全」；HomePage.tsx:134 快速操作「測試回應方式」導向 legacy tab responses → App.tsx:761 渲染無 hub 分頁的孤立能力列表。
- [覆蓋缺口] 無測試斷言 Shell 恰有 5 個一級入口、無測試斷言其他頁沒有第二份開關、無 390px 主要流程（精靈/建立工作階段/授權/estop 解除）測試；390px 與 5 入口唯一證據為他人跑出的瀏覽器截圖（Chromium 對真 daemon，非 Tauri 實機）。

### Phase 2 小樞角色、36 表情、組合通道、Game Feel（118 列）

| Phase | 需求 | 程式存在 | 已接線 | 測試通過 | 真環境驗證 | 缺口 | 嚴重度 |
|---|---|---|---|---|---|---|---|
| 2 | §4.1 視覺方向：約 2.5～2.7 頭身、大頭小身體低重心圓潤 | 是 | 是 | 無 | 未驗證 | 比例只在註解與 layout 常數；無量測；真 Tauri 視窗未親驗（只有 docs/assets/v05-evidence/shu-maid-rig-sheet.png 由 headless Chromium 產出） | 🟢 low |
| 2 | §4.1 女性化但無成熟成人感，不強調胸腰臀 | 是 | 是 | 無 | 未驗證 | none（設計層面，僅目視可驗） | 無 |
| 2 | §4.1 聰明/俏皮/機靈/可愛/慵懶氣質 | 部分 | 部分 | 部分 | 僅模擬器 | 氣質只烘焙在個別表情關鍵幀；無個性參數影響行為（見 §4.3 各列） | 🟡 medium |
| 2 | §4.1 柔黑或深灰紫短髮，帶不對稱髮束 | 是 | 是 | 無 | 未驗證 | none | 無 |
| 2 | §4.1 大而機靈、眼尾微揚的紫灰色眼睛 | 是 | 是 | 無 | 未驗證 | none | 無 |
| 2 | §4.1 得意時偶爾露出一顆小虎牙 | 是 | 是 | 無 | 僅模擬器 | none | 無 |
| 2 | §4.1 一對真實參與表演的貓耳；不另加人類耳朵 | 是 | 是 | 部分 | 僅模擬器 | none | 無 |
| 2 | §4.1 左耳冷藍「感知耳」、右耳暖橘「行動耳」 | 是 | 是 | 無 | 僅模擬器 | 只有 listening 亮左耳、working 亮右耳；receptor.observation 以外的感知（麥克風/裝置）不驅動 earL | 🟢 low |
| 2 | §4.1 長而柔軟、高度表意並參與操作的貓尾 | 部分 | 是 | 無 | 僅模擬器 | 「參與操作」（指向/拖取物件）未實作：tailTip 只是發光，尾巴不拖任何物件、不指向目標 | 🟡 medium |
| 2 | §4.1 胸前呼吸般發光的小型結晶／蝴蝶結核心 | 是 | 是 | 無 | 僅模擬器 | none | 無 |
| 2 | §4.2 奶白泡泡袖、完整圓領或小立領 | 是 | 是 | 無 | 未驗證 | none | 無 |
| 2 | §4.2 深灰紫短外衣或小披肩 | 部分 | 是 | 無 | 未驗證 | 沒有短外衣/小披肩這一層，僅上身洋裝為深灰紫 | 🟢 low |
| 2 | §4.2 小型分體女僕頭飾，中央為貓耳保留活動空間 | 是 | 是 | 無 | 未驗證 | none | 無 |
| 2 | §4.2 胸前蝴蝶結與發光核心整合 | 是 | 是 | 無 | 未驗證 | none | 無 |
| 2 | §4.2 蓬鬆多層裙擺，內搭不透明燈籠安全短褲 | 是 | 是 | 無 | 未驗證 | none | 無 |
| 2 | §4.2 工具圍裙與兩側口袋 | 是 | 是 | 無 | 未驗證 | none（口袋功能見表格列） | 無 |
| 2 | §4.2 稍大的袖口、手套與圓頭軟底短靴 | 是 | 是 | 無 | 未驗證 | none | 無 |
| 2 | §4.2 奶白/深灰紫主色，冷藍/暖橘能力訊號，低飽和粉紫肉球 | 是 | 是 | 通過 | 僅模擬器 | none | 無 |
| 2 | §4.2 不使用胸口開洞、吊帶襪、貼身曲線、過短裙擺或成熟誘惑姿勢 | 是 | 是 | 無 | 未驗證 | none | 無 |
| 2 | §4.2 表：左耳＝受器／感知活動 | 部分 | 部分 | 無 | 僅模擬器 | 只在 listening transient（1.5s）亮；啟用中的感測器（麥克風/攝影機/硬體 receptor）長期活動不反映在左耳 | 🟡 medium |
| 2 | §4.2 表：右耳＝動器／行動活動 | 部分 | 部分 | 無 | 僅模擬器 | 只在 working 表情亮；硬體動器實際執行（Phase 5 adapter）不另驅動 earR | 🟡 medium |
| 2 | §4.2 表：胸前核心＝Runtime、AI 思考與工作狀態 | 是 | 是 | 部分 | 僅模擬器 | none | 無 |
| 2 | §4.2 表：頭飾＝網路與 Agent 連線狀態；奔跑後可歪掉再扶正 | 部分 | 部分 | 無 | 僅模擬器 | (1) headpieceGlow 只是表情內固定值，不讀真實網路/Agent 連線狀態；(2)「奔跑後歪掉再扶正」未實作：stage.ts:327 註解寫「頭飾微彈」但程式碼 331-338 沒有 headpieceTilt；(3) 扶正靠 exit 段，而 exit 段從未被播放（timeline.ts 無 exit 讀取） | 🟠 high |
| 2 | §4.2 表：圍裙口袋＝取出玩具、檔案、知識卡片與工具 | 否 | 否 | 無 | n/a | 沒有從口袋取出任何物件的動作/通道 | 🟡 medium |
| 2 | §4.2 表：袖口＝展開小型工作面板 | 否 | 否 | 無 | n/a | 完全未實作 | 🟡 medium |
| 2 | §4.2 表：尾巴＝指向、拖取物件、表達情緒 | 部分 | 部分 | 無 | 僅模擬器 | 尾巴不指向目標、不拖取玩具/物件 | 🟡 medium |
| 2 | §4.2 表：裙擺細光＝waiting、unknown、blocked 等輔助狀態 | 是 | 是 | 無 | 僅模擬器 | none | 🟢 low |
| 2 | §4.3 個性必須影響 Attention、Utility score、動作選擇、速度、距離、表情與恢復方式 | 否 | 否 | 無 | n/a | 不存在個性模型；三個 rig 變體只差調色盤。Attention/Utility/速度/距離皆與個性無關 | 🟠 high |
| 2 | §4.3 聰明：先動耳朵、再移動視線、最後才轉頭 | 是 | 是 | 無 | 僅模擬器 | 只烘焙在 2 個表情；非通用反應鏈 | 🟢 low |
| 2 | §4.3 機靈：事件剛出現便提前接住或避開 | 否 | 否 | 無 | n/a | 未實作 | 🟡 medium |
| 2 | §4.3 俏皮：偶爾假裝沒看到、藏起物件、從另一側探頭 | 部分 | 部分 | 部分 | 僅模擬器 | 假裝沒看到、探頭表情存在但執行期不可達；「從另一側探頭」無邊緣/側邊行為 | 🟡 medium |
| 2 | §4.3 慵懶：趴著操作、用尾巴拖工作、慢半拍起身 | 部分 | 部分 | 無 | 僅模擬器 | 趴著操作/尾巴拖工作/慢半拍起身皆未實作 | 🟡 medium |
| 2 | §4.3 得意：半瞇眼、抬下巴、尾巴豎起，等待稱讚卻嘴硬 | 是 | 是 | 部分 | 僅模擬器 | 「嘴硬」文案：conversation.ts:77「嘿嘿，小事一件」勉強算 | 🟢 low |
| 2 | §4.3 好奇：歪頭、靠近、瞳孔放大、耳朵朝向目標 | 是 | 部分 | 部分 | 僅模擬器 | 耳朵不會朝向真實目標（無目標座標）；lean-in 不可達 | 🟢 low |
| 2 | §4.3 被抓包：瞬間定格、移開視線、假裝整理袖口或工具 | 是 | 否 | 部分 | 僅模擬器 | 執行期永遠不會播放；無「偷懶被玩家看到」觸發條件 | 🟡 medium |
| 2 | §4.3 失敗：不崩潰、不責怪玩家，先確認現場再提出下一步 | 部分 | 是 | 通過 | 僅模擬器 | 「提出下一步」未實作：失敗後只顯示固定文案，無建議 | 🟢 low |
| 2 | §4.3 不是服從型女僕：不持續鞠躬、敬禮或稱呼主人 | 是 | 是 | 無 | 僅模擬器 | none | 無 |
| 2 | §6.2 通道：Body pose／locomotion | 是 | 是 | 部分 | 僅模擬器 | none | 無 |
| 2 | §6.2 通道：Head pose | 是 | 是 | 無 | 僅模擬器 | none | 無 |
| 2 | §6.2 通道：Gaze target | 部分 | 是 | 通過 | 僅模擬器 | 沒有「target」概念——只有正弦掃視，不看向玩具/游標/玩家 | 🟡 medium |
| 2 | §6.2 通道：Eyes／brows／mouth expression | 是 | 是 | 無 | 僅模擬器 | none | 無 |
| 2 | §6.2 通道：Cat ears | 是 | 是 | 無 | 僅模擬器 | none | 無 |
| 2 | §6.2 通道：Tail | 是 | 是 | 無 | 僅模擬器 | none（功能性見 §4.2 尾巴列） | 無 |
| 2 | §6.2 通道：Hair／sleeves／headpiece secondary motion | 部分 | 部分 | 無 | 僅模擬器 | 袖口無 secondary motion；頭飾不隨移動晃動（stage.ts:327 註解與程式不符） | 🟢 low |
| 2 | §6.2 通道：Hand／held prop | 部分 | 是 | 部分 | 僅模擬器 | 道具只有玩具；無檔案/知識卡/工具道具 | 🟢 low |
| 2 | §6.2 通道：Bubble／text | 是 | 是 | 通過 | 僅模擬器 | 氣泡是獨立 DOM，不在 rig 參數/Director 混音層；非「組合通道」的一員 | 🟢 low |
| 2 | §6.2 通道：Audio／voice／purr／SFX | 部分 | 部分 | 通過 | 僅模擬器 | 無 purr、無動作 SFX、無音效變體；音效通道未進 Director 混音 | 🟡 medium |
| 2 | §6.2 通道：Position／desktop anchor | 部分 | 是 | 部分 | 僅模擬器 | 角色只在自家 canvas 內移動；無桌面錨點（視窗邊/螢幕邊）概念 | 🟢 low |
| 2 | §6.2 通道：Particles／status effect | 是 | 是 | 通過 | 僅模擬器 | none | 無 |
| 2 | §6.2 允許組合（例：趴著＋看向玩家＋左耳注意＋尾尖輕擺＋核心顯示工作中） | 部分 | 部分 | 無 | 僅模擬器 | 沒有多通道獨立混音：不能「趴著（ambient）」同時「核心顯示 Agent 工作中（truth）」——machine 非 idle 就整個覆蓋（stage.ts:317） | 🟠 high |
| 2 | §6.3 Game Feel：Anticipation | 是 | 是 | 無 | 僅模擬器 | 只在 3 個表情有預備段 | 🟢 low |
| 2 | §6.3 Game Feel：Squash and Stretch | 是 | 是 | 無 | 僅模擬器 | none | 🟢 low |
| 2 | §6.3 Game Feel：Overshoot | 是 | 是 | 無 | 僅模擬器 | none | 🟢 low |
| 2 | §6.3 Game Feel：Follow-through | 部分 | 否 | 無 | n/a | exit/離開段是死資料，follow-through 實際不存在；只剩 180ms 線性交叉淡入 | 🟠 high |
| 2 | §6.3 Game Feel：Secondary motion | 是 | 是 | 無 | 僅模擬器 | none | 無 |
| 2 | §6.3 Game Feel：1～2 幀適量 hit-stop | 部分 | 是 | 無 | 僅模擬器 | 沒有真正的時間凍結機制（只是關鍵幀間 100–150ms 不變）；無測試 | 🟢 low |
| 2 | §6.3 Game Feel：細小粒子、灰塵、睏意與撞擊效果 | 是 | 是 | 無 | 僅模擬器 | 玩具落地/牆壁反彈無粒子 | 🟢 low |
| 2 | §6.3 Game Feel：音效變體 | 否 | 否 | 無 | n/a | 未實作 | 🟡 medium |
| 2 | §6.3 Game Feel：動作取消與恢復 | 是 | 是 | 通過 | 僅模擬器 | 恢復從頭重播表情（timeline 無 seek），非「從斷點恢復」 | 🟢 low |
| 2 | §6.3 不要過度晃動 UI；Game Feel 主要作用在角色與玩具 | 是 | 是 | 無 | 僅模擬器 | none | 無 |
| 2 | §6.4 分層 2D 骨架／mesh deformation 處理身體、頭、耳、尾、頭髮、袖口與配件 | 部分 | 是 | 無 | 僅模擬器 | 是參數化程序繪圖，不是骨架/mesh 系統；袖口無獨立變形 | 🟡 medium |
| 2 | §6.4 臉部使用可替換表情圖層與參數化眼睛／眉毛／嘴形 | 是 | 是 | 無 | 僅模擬器 | none（以參數化取代圖層，功能等價） | 無 |
| 2 | §6.4 撲抓、滑倒、壓扁、驚訝等誇張動作使用逐幀手繪或變形 Sprite | 否 | 否 | 無 | n/a | 未實作；誇張度受限於參數界線（squash ±0.5） | 🟡 medium |
| 2 | §6.4 渲染層可採 Canvas／WebGL；需評估 Tauri WebView、包體、效能、跨平台及授權 | 部分 | 是 | 無 | 未驗證 | 無評估文件；效能無量測數據 | 🟢 low |
| 2 | §6.4 不得為了技術方便犧牲 390px、Reduced Motion 或透明視窗效能 | 部分 | 是 | 通過 | 未驗證 | 透明視窗 rAF 每幀重繪 1246 行向量（renderer.ts:85-89 無節流）無效能量測；390px 為控制中心議題 | 🟢 low |
| 2 | §6.4 保留舊 Character Pack 相容層與 fallback | 是 | 是 | 通過 | 僅模擬器 | none | 無 |
| 2 | §6.4 新版格式須支援分層骨架、表情、通道、變體與行為 metadata | 否 | 部分 | 通過 | 僅模擬器 | pack 格式不承載任何動畫/表情/通道/行為資料；manifest description 自稱「四段式動畫」與實際不符 | 🟡 medium |
| 2 | §7.1 站立呼吸、坐下、趴下、打瞌睡、熟睡、驚醒 | 是 | 部分 | 部分 | 僅模擬器 | 缺：sit、sleep、startled-awake 執行期不可達；doze→sleep→驚醒無狀態鏈 | 🟡 medium |
| 2 | §7.1 伸懶腰、整理貓耳／頭髮／頭飾／裙擺 | 部分 | 是 | 部分 | 僅模擬器 | 缺：整理貓耳、整理裙擺 無專屬動作；頭髮/頭飾合併成一個 groom | 🟢 low |
| 2 | §7.1 左右張望、放空、偷懶被發現、從邊緣探頭 | 部分 | 部分 | 部分 | 僅模擬器 | 缺：偷懶被發現 無觸發；從邊緣探頭（螢幕/視窗邊緣進出）未實作 | 🟡 medium |
| 3 | §7.2 走路、小跑、奔跑、急停、轉身、跳起、落地、滑倒、攀爬 | 部分 | 部分 | 部分 | 僅模擬器 | 缺：急停（mode 切換 vx=0 瞬停 playfield.ts:361,387）、轉身、跳起、攀爬；滑倒不可達；走/小跑/奔跑共用同一步態僅速度不同 | 🟡 medium |
| 3 | §7.2 被拖曳、懸空、放下、重新站穩 | 是 | 是 | 部分 | 僅模擬器 | dragged 固定 1500ms 後即使仍懸空也會回 idle（startDragging 期間無持續狀態） | 🟢 low |
| 3 | §7.3 被點擊、連點、看向游標、靠近、躲開、伸手擋住游標 | 部分 | 部分 | 部分 | 僅模擬器 | 缺：看向游標（behavior.ts:112-114 明言不用游標；只靠光點玩具間接）、躲開、伸手擋游標（定義但不可達） | 🟡 medium |
| 3 | §7.3 追逐光點、撲毛球、撲空、抱住、帶回、不想歸還 | 是 | 是 | 通過 | 僅模擬器 | none | 無 |
| 3 | §7.3 接住拖入檔案、拒絕未確認附件 | 部分 | 部分 | 無 | 僅模擬器 | 缺：接住檔案動畫、拒絕附件動畫（drop overlay 是孤兒參數） | 🟡 medium |
| 4 | §7.4 閱讀、思考、快速書寫、操作工具、翻找資料 | 部分 | 是 | 部分 | 僅模擬器 | 缺：閱讀、快速書寫；操作工具僅以 working 代替 | 🟡 medium |
| 4 | §7.4 等待 Codex、等待 Claude、等待玩家確認 | 是 | 是 | 通過 | 僅模擬器 | wait-codex 與 wait-claude 僅 headTilt ±4 之差，辨識度低 | 🟢 low |
| 4 | §7.4 queued、fetched、working、waiting、blocked、claimed-completed、verified、failed、unknown、cancelled | 部分 | 是 | 通過 | 僅模擬器 | 缺：session-level blocked/unknown 映射；cancelled 無專屬演出（直接回 idle）；waiting 與 queued 共用 wait 表情 | 🟡 medium |
| 4 | §7.4 claimed-completed 只呈現「對方說完成了」；verified 才有綠勾與正式成功演出 | 是 | 是 | 通過 | 僅模擬器 | none | 無 |
| 2 | §7.5 每個表情定義進入、保持、小循環、離開（四段式），不得只做靜態圖片 | 部分 | 部分 | 部分 | 僅模擬器 | 「離開」段完全未實作（5 個有 exit 的表情也不會播）；CHANGELOG.md:27-28、docs/v05-capability-gap-matrix.md:105、CompanionPage.tsx:848-850 宣稱「皆有進入/保持/小循環」與事實不符 | 🔴 blocker |
| 2 | §7.5 疑問 question | 部分 | 是 | 部分 | 僅模擬器 | 缺 enter、exit | 🟢 low |
| 2 | §7.5 偷看 peek | 部分 | 否 | 部分 | 僅模擬器 | 缺 exit；執行期不可達 | 🟡 medium |
| 2 | §7.5 歪頭 curious | 部分 | 是 | 部分 | 僅模擬器 | 缺 exit | 🟢 low |
| 2 | §7.5 探頭 lean-in | 部分 | 否 | 部分 | 僅模擬器 | 缺 exit；不可達 | 🟡 medium |
| 2 | §7.5 無語 deadpan | 部分 | 否 | 部分 | 僅模擬器 | 缺 enter/exit；不可達 | 🟡 medium |
| 2 | §7.5 放空 spaced-out | 部分 | 是 | 部分 | 僅模擬器 | 缺 enter/exit | 🟢 low |
| 2 | §7.5 哈欠 yawn | 部分 | 是 | 部分 | 僅模擬器 | 缺 exit | 🟢 low |
| 2 | §7.5 趴平 lie-flat | 部分 | 是 | 部分 | 僅模擬器 | 缺 exit（起身無動畫） | 🟢 low |
| 2 | §7.5 伸懶腰 stretch | 部分 | 是 | 部分 | 僅模擬器 | 缺 exit | 🟢 low |
| 2 | §7.5 被吵醒 startled-awake | 部分 | 否 | 部分 | 僅模擬器 | 缺 loop；exit 不會播；不可達 | 🟡 medium |
| 2 | §7.5 假裝沒聽見 pretend-not-hear | 部分 | 否 | 部分 | 僅模擬器 | 缺 enter/exit；不可達 | 🟡 medium |
| 2 | §7.5 悄悄靠近 sneak-closer | 部分 | 是 | 部分 | 僅模擬器 | 缺 enter/exit | 🟢 low |
| 2 | §7.5 被點 poked | 部分 | 是 | 部分 | 僅模擬器 | 缺 exit | 🟢 low |
| 2 | §7.5 被連戳 poked-rapid | 部分 | 是 | 部分 | 僅模擬器 | 缺 exit | 🟢 low |
| 2 | §7.5 被拖起 lifted | 部分 | 是 | 部分 | 僅模擬器 | 缺 exit | 🟢 low |
| 2 | §7.5 落地站不穩 wobbly-landing | 部分 | 是 | 部分 | 僅模擬器 | 缺 loop；exit（頭飾扶正、汗消）不會播 | 🟡 medium |
| 2 | §7.5 抱球 hold-ball | 部分 | 是 | 通過 | 僅模擬器 | 缺 enter/exit | 🟢 low |
| 2 | §7.5 不還球 keep-ball | 部分 | 是 | 通過 | 僅模擬器 | 缺 enter/exit | 🟢 low |
| 2 | §7.5 撲空 pounce-miss | 部分 | 是 | 通過 | 僅模擬器 | 缺 loop；exit 不會播 | 🟢 low |
| 2 | §7.5 滑倒裝沒事 slip-play-cool | 部分 | 否 | 部分 | 僅模擬器 | 缺 loop；exit 不播；不可達 | 🟡 medium |
| 2 | §7.5 被稱讚 praised | 部分 | 是 | 部分 | 僅模擬器 | 缺 exit | 🟢 low |
| 2 | §7.5 偷懶被抓 caught-slacking | 部分 | 否 | 部分 | 僅模擬器 | 缺 loop；exit 不播；不可達 | 🟡 medium |
| 2 | §7.5 等玩家 await-player | 部分 | 否 | 部分 | 僅模擬器 | 缺 enter/exit；不可達（玩家離開無偵測→無此狀態） | 🟡 medium |
| 2 | §7.5 玩家回來 player-back | 部分 | 是 | 部分 | 僅模擬器 | 缺 exit；只在玩家主動打字時觸發，不會自動偵測回來 | 🟢 low |
| 2 | §7.5 思考 thinking | 部分 | 是 | 通過 | 僅模擬器 | 缺 enter/exit | 🟢 low |
| 2 | §7.5 找資料 routing | 部分 | 是 | 通過 | 僅模擬器 | 缺 enter/exit | 🟢 low |
| 2 | §7.5 努力工作 working | 部分 | 是 | 通過 | 僅模擬器 | 缺 enter/exit | 🟢 low |
| 2 | §7.5 等 Codex wait-codex | 部分 | 是 | 通過 | 僅模擬器 | 缺 enter/exit | 🟢 low |
| 2 | §7.5 等 Claude wait-claude | 部分 | 是 | 通過 | 僅模擬器 | 缺 enter/exit | 🟢 low |
| 2 | §7.5 需要確認 ask | 部分 | 是 | 通過 | 僅模擬器 | 缺 exit | 🟢 low |
| 2 | §7.5 權限不足 blocked | 部分 | 是 | 通過 | 僅模擬器 | 缺 enter/exit | 🟢 low |
| 2 | §7.5 找不到 not-found | 部分 | 否 | 部分 | 僅模擬器 | 缺 enter/exit；不可達 | 🟡 medium |
| 2 | §7.5 結果未知 unknown | 部分 | 是 | 通過 | 僅模擬器 | 缺 enter/exit | 🟢 low |
| 2 | §7.5 聲稱完成 success-claimed | 部分 | 是 | 通過 | 僅模擬器 | 缺 loop/exit | 🟢 low |
| 2 | §7.5 驗證成功 success-verified | 部分 | 是 | 通過 | 僅模擬器 | 缺 exit | 🟢 low |
| 2 | §7.5 工作失敗 failed | 部分 | 是 | 通過 | 僅模擬器 | 缺 loop/exit | 🟢 low |
| cross | scripts/shu/ 產生器（CLAUDE.md 佈局所述） | 部分 | n/a | 無 | 僅模擬器 | v3 rig 無產生器（純執行期程式），CLAUDE.md 路徑過期 | 🟢 low |
| cross | 我負責範圍的測試實跑數字 | 是 | n/a | 通過 | 僅模擬器 | draw.ts(1246行)、timeline.ts、stage.ts、RigRenderer 零測試覆蓋（grep drawRig/StageRenderer/ExpressionTimeline/paramsAt src/test → 0） | 🟡 medium |

**診斷（TODO／placeholder／mock 冒充／hard-coded success／空函式／跳過測試／警告）：**

- [blocker] 「離開」段死資料：expressions.ts:37 宣告 exit，5 個表情定義 exit（:202,:528,:648,:663,:714），但 timeline.ts:130-181 / stage.ts / renderer.ts 完全不讀 expr.exit → 四段式中的「離開」從未播放；表情切換只靠 180ms crossfade（timeline.ts:17,146-150）。
- [high] 36 表情四段統計（腳本實測）：16 無 enter、7 無 loop、31 無 exit、0 個四段齊全。rig.test.ts:57 只斷言 `enter \|\| loop`，測試名稱「非靜態圖片」把規格降級。
- [high] 文件/UI 宣稱與事實不符：CHANGELOG.md:27-28「每個都有進入/保持/小循環時間軸」、docs/v05-capability-gap-matrix.md:105「皆有進入/保持/小循環」、CompanionPage.tsx:848-850、public/packs/shu-maid*/manifest.json description「四段式動畫」。
- [high] 無個性模型：params.ts:314-315 明寫「變體只改配色…不改任何行為」；behavior.ts:182 scoreEvent、director.ts:133 tick、playfield.ts:100-101 速度常數皆無個性輸入。§4.3「個性影響 Attention/Utility/速度/距離」未實作。
- [high] 通道無法真正組合：stage.ts:313-324 machine 非 idle 就整個覆蓋遊玩表情；timeline 一次只播一個 Expression；規格例「趴著＋核心顯示 Agent 工作中」做不到。
- [medium] 死碼：director.ts:122 react() 與 :59-71 REACTION_EXPRESSIONS 在 app 內從未被呼叫（grep `.react(` 非測試 0 hits）；L1 意圖走 CompanionApp.tsx:1001-1003 直接 apply performing，繞過 Director 的 playable() 防線。
- [medium] 執行期不可達的表情（只在控制中心靜態預覽格可見）：peek, lean-in, deadpan, startled-awake, pretend-not-hear, slip-play-cool, caught-slacking, await-player, not-found（36 之 9）＋ block-cursor、sit、sleep。
- [medium] 註解與程式不符：stage.ts:327「移動 secondary motion：步態、髮尾、頭飾微彈」但 :331-338 未動 headpieceTilt；§4.2「頭飾奔跑後歪掉再扶正」未實作。
- [medium] params.ts:40 overlay 'drop' 與 draw.ts:1129-1138 繪製存在，但無任何表情使用（grep expressions.ts "drop" → 0）；§7.3 接住檔案/拒絕附件無動畫。
- [medium] Audio 通道只有 3 個 AI 白名單提示音（CompanionApp.tsx:69-102），無 purr/動作 SFX/音效變體；不在 Director 混音層。
- [medium] Rig pack 格式（renderer.ts:93-117）只承載 palette；§6.4 要求的骨架/表情/通道/變體/行為 metadata 全硬編碼於 TS；三份 manifest 僅 palette 不同。
- [medium] §6.4 逐幀手繪/變形 Sprite 未實作；無 Canvas/WebGL 評估文件（grep -i webgl docs/ CHANGELOG.md → 0）。
- [medium] draw.ts（1246 行）、timeline.ts、stage.ts 渲染、RigRenderer 零單元測試；squash/hit-stop/anticipation/overshoot/headpieceTilt 無任何斷言（grep src/test → 0）。
- [medium] machine.ts:224-252 agent.session.state 沒有 blocked/unknown 分支（§7.4 列為 session 狀態）；cancelled 只 clear-transient 無專屬演出。
- [low] CLAUDE.md 佈局寫 `scripts/shu/`，實際在 apps/interaction-desktop/scripts/shu/；generate/design/animations.mjs 仍是 v2 sprite 產生器，v3 rig 無產生器。
- [low] groom 一個動作合併「整理貓耳／頭髮／頭飾／裙擺」四項；wait-codex 與 wait-claude 僅 headTilt ±4 之差。
- [info] 實跑：vitest 6 檔 74/74 通過；pnpm typecheck 無錯誤。所有驗證皆 jsdom/程序內，無真 Tauri 視窗親驗；docs/assets/v05-evidence/shu-maid-rig-sheet.png 由 preview-rig.mjs headless Chromium 產出（simulator-only）。

### Phase 2 Interaction Director／§6.1／§8.1／§14（28 列）

| Phase | 需求 | 程式存在 | 已接線 | 測試通過 | 真環境驗證 | 缺口 | 嚴重度 |
|---|---|---|---|---|---|---|---|
| Phase 2 | §6 管線 Event Normalizer（Receptors／玩家／硬體／Agent 事件 → 統一事件） | 部分 | 是 | 部分 | 僅模擬器 | 只有 runtime 事件有 normalizer；玩家/指標/playfield 事件各自直接 apply transient；硬體裝置事件只以 receptor.observation→listening（machine.ts:198-199）一律化，無 device.* 專屬映射 | 🟢 low |
| Phase 2 | §6 管線 Attention Manager + Utility Scoring（多事件注意力競爭） | 部分 | 否 | 部分 | 僅模擬器 | Utility 評分是死碼：生產路徑沒有 Attention Manager，只有固定優先度替換（低優先直接丟棄，非競爭/排隊） | 🟠 high |
| Phase 2 | §6 管線 Character Context | 是 | 是 | 通過 | 僅模擬器 | Context 缺 Fullscreen／OS 勿擾／實際 quiet-hours 輸入（見對應列） | 🟢 low |
| Phase 2 | §6 管線 Behavior Intent → Action Scheduler | 是 | 部分 | 部分 | 僅模擬器 | react() 只在測試存在；Director 只掌管 ambient 變體，反應與 presentation intent 都繞過它（單一「不准事件直接切換動畫」的門並不存在） | 🟡 medium |
| Phase 2 | §6 管線 Animation／Gaze／Ear／Tail／Bubble／Audio Mixer | 是 | 是 | 部分 | 未驗證 | 混合層無專屬測試；音效只有 3 個合成音、無變體 | 🟢 low |
| Phase 2 | §6 管線 Presentation Ack（誠實回報 displayed/completed/unsupported/failed） | 是 | 是 | 通過 | 僅模擬器 | none（程序內 runtime＋fixture，無真視窗） | 無 |
| Phase 2 | §6.1 多事件注意力競爭與優先度 | 部分 | 部分 | 部分 | 僅模擬器 | 只有固定優先度替換，無「競爭」（無 relevance/novelty/重複懲罰進入實路徑）；等優先度後到者直接覆蓋（machine.ts:106 用 >） | 🟠 high |
| Phase 2 | §6.1 高風險安全事件可搶占任何表演 | 是 | 是 | 通過 | 僅模擬器 | none | 無 |
| Phase 2 | §6.1 動作可中斷、可恢復、可取消 | 部分 | 是 | 部分 | 僅模擬器 | presentation `cancel`/`clear-all` 不清除 performing transient（CompanionApp.tsx:349-354 註解宣稱 drop non-safety visual 但只 showBubble(null)）；director.noteFinished() 從未被呼叫（director.ts:117）；playfield 表情事件替換 ambient 時不經 notePreempted | 🟡 medium |
| Phase 2 | §6.1 進入、保持、小循環、離開四段式狀態 | 部分 | 部分 | 部分 | 未驗證 | 「離開」段未實作：切換表情時以 180ms crossfade 取代（timeline.ts:17,146-150），exit keyframes 為死資料；CHANGELOG.md:37-41 與 gap-matrix:105 未標此限制 | 🟡 medium |
| Phase 2 | §6.1 前一動作到下一動作的自然 Transition | 是 | 是 | 無 | 未驗證 | 缺 transition 連續性斷言（例如切換瞬間參數不跳變） | 🟢 low |
| Phase 2 | §6.1 同一意圖的變體選擇與近期防重複 | 部分 | 是 | 通過 | 僅模擬器 | 「同一意圖的變體」不成立：反應意圖 1:1 映射無變體；ambient 的變體是不同表情而非同意圖多版本 | 🟢 low |
| Phase 2 | §6.1 動作冷卻、頻率及最大連續時間 | 部分 | 是 | 通過 | 僅模擬器 | react() 只寫 cooldown 不檢查（director.ts:122-130 無 cooldownUntil 判斷）；無跨動作「最大連續表演時間」預算，僅靠單一動作時長與冷卻 | 🟢 low |
| Phase 4 | §6.1 AI 延遲時的自然等待與降級表現 | 部分 | 是 | 通過 | 僅模擬器 | 等待表現只維持 transient 時長：working 8s 後無再觸發即回 idle/ambient（grep runtime 無週期性 AgentSessionState 重發），長工作時角色看起來沒事；無「AI 逾時降級」專屬表現（timed-out 直接演 failed） | 🟡 medium |
| Phase 2 | §6.1 Reduced Motion | 是 | 是 | 通過 | 未驗證 | renderer/timeline 的 reduced 旗標只在 boot 設一次（CompanionApp.tsx:298），OS 執行中切換不會更新（Director/micro-motion 會）；hold 內粒子（praised heart expressions.ts:683、success-verified sparkle :1006）在 reduced 下仍靜態繪出（draw.ts:1175-1179 不看 reduced） | 🟢 low |
| Phase 2 | §6.1 Quiet Hours | 部分 | 否 | 部分 | 僅模擬器 | Quiet Hours 對角色是死路徑：runtime 的 quiet_hours 永遠不會讓小樞進 quiet 基態；快捷「一小時內不要主動說話」只調 proactiveDialogueQuiet（CompanionApp.tsx:843-851），也不影響 Director | 🟠 high |
| Phase 2 | §6.1 Fullscreen 偵測（全螢幕時收斂） | 否 | 否 | 無 | n/a | 完全未實作；docs/v05-capability-gap-matrix.md:105 標 Director「已有」、CHANGELOG 無已知限制註記 | 🟠 high |
| Phase 2 | §6.1 勿擾模式（OS Do Not Disturb／Focus） | 部分 | 否 | 部分 | n/a | 沒有系統勿擾偵測，也沒有使用者層級「勿擾」旗標接進 Director；唯一相關參數在死碼裡 | 🟠 high |
| Phase 2 | §6.1 角色被隱藏時停止 Presentation receptors，但 Runtime、Tray、Agent 保持正確狀態 | 是 | 是 | 通過 | 僅模擬器 | Tauri toggle_companion_window（lib.rs:1433-1446）只 w.hide()＋emit companion-visibility，不通知 runtime；CompanionApp 無 companion-visibility 監聽（grep 0），僅靠 WebView document.hidden／心跳逾時 20s，隱藏後最長 10–20s receptors 仍接受輸入；WKWebView 隱藏視窗是否觸發 visibilitychange 未真機驗證 | 🟡 medium |
| Phase 3 | §8.1 L0：游標、點擊、拖曳、玩具與物理（16–100ms、不呼叫 AI） | 是 | 是 | 通過 | 未驗證 | 無任何延遲量測（無 performance.mark／frame time 記錄）；16–100ms 目標僅由架構推定（同步 handler＋rAF），未實測 | 🟢 low |
| Phase 2 | §8.1 L0：眼睛、耳朵、尾巴、眨眼、姿勢及短氣泡 | 是 | 是 | 通過 | 未驗證 | none | 無 |
| Phase 4 | §8.1 L0：Runtime、裝置與 Agent 狀態映射 | 部分 | 是 | 通過 | 僅模擬器 | 硬體裝置（Phase 5 serial/MQTT/BLE）狀態無專屬映射：所有 receptor.observation 一律 listening 1.5s（machine.ts:198-199），裝置離線/錯誤不會反映在角色；quietHours 映射失效（見 Quiet Hours 列） | 🟢 low |
| Phase 2 | §8.1 L0：動作變體、冷卻、注意力與玩耍（本機、不呼叫 AI） | 是 | 是 | 通過 | 僅模擬器 | none | 無 |
| cross | §14 一般動畫不得因 HTTP、SQLite、Agent 或 AI 阻塞 | 是 | 是 | 無 | 未驗證 | pump 每 500ms 動態 import + invoke companion_hit_rect（CompanionApp.tsx:555）為固定開銷；無測試/量測證明不阻塞 | 🟢 low |
| cross | §14 Interaction Director 與 renderer 使用 bounded queues | 部分 | 是 | 部分 | 僅模擬器 | 「有界」靠丟棄而非佇列（Director/renderer 根本沒有 queue，符合上限但沒有排程語意）；PENDING_CAP 與前端 SSE buffer 無測試 | 🟢 low |
| cross | §14 動畫與事件在 60fps 目標下量測；低效能裝置允許 30fps 降級 | 否 | 否 | 無 | n/a | 無 frame-time 量測、無 30fps 模式、無低效能偵測；文件亦未列為已知限制 | 🟡 medium |
| cross | §14 Reduced Motion 下保留狀態辨識但減少位移、彈跳與粒子 | 部分 | 是 | 部分 | 未驗證 | 粒子未明確減少（hold 粒子照畫）；reduced 旗標執行中不更新 renderer | 🟢 low |
| cross | §14 原始游標軌跡不持久化、不傳 AI | 是 | 是 | 部分 | 僅模擬器 | 缺「送 runtime 的載荷不含座標」回歸測試；desktop.pointer.activity 送 idleForMs（粗粒度，可接受） | 🟢 low |

**診斷（TODO／placeholder／mock 冒充／hard-coded success／空函式／跳過測試／警告）：**

- CompanionApp.tsx:489 讀取 status["quietHours"]，但 crates/interaction-runtime/src/runtime.rs:409-443 status() 從未輸出該鍵（grep crates/ + src-tauri 0 命中；git log -S 顯示 v0.3 commit 9ee4076 起即如此）→ 角色 quiet 基態與 Director quiet 路徑（director.ts:150-156）在生產環境永遠不可達。
- Attention/Utility 死碼：behavior.ts:182 scoreEvent 與 director.ts:95 score() 在 src/（排除 test）無任何呼叫點；實際只用 machine.ts:47-61 靜態優先度。
- director.ts:117 noteFinished() 從未被呼叫；director.ts:122 react() 從未被 app 呼叫（CompanionApp.tsx:1001-1003 直接 apply performing，繞過 truthState 過濾與冷卻）；react() 寫 cooldown 但不檢查 cooldownUntil。
- behavior.ts:212-275 MICRO_ACTIONS/scheduleMicroAction 已被 Director 取代（CHANGELOG.md:40-41 自述），仍輸出並有 6 個測試在測死碼（behavior.test.ts:119-207）。
- 四段式「離開」未實作：expressions.ts:37 exit 欄位在 5 個表情（:202,528,648,663,714）有資料，timeline.ts:130-144 從不評估 exit；rig.test.ts:50-61 只斷言 enter\|\|loop。
- CompanionApp.tsx:349-354 presentation `cancel`/`clear-all` 註解宣稱 drop any non-safety visual，實際只 showBubble(null)，performing transient 不清除。
- Fullscreen／OS 勿擾偵測完全不存在（grep src-tauri/src、companion/、runtime/src：0 命中）；docs/v05-capability-gap-matrix.md:105 將 Interaction Director 標為「已有」，CHANGELOG 已知限制未列 Fullscreen／勿擾／30fps。
- 無 60fps 量測與 30fps 降級程式碼；stage.ts:289 dtMs≤100 只是物理防爆。
- CompanionApp.tsx:298 renderer.setReducedMotion 只在 boot 呼叫一次，無 matchMedia change 監聽；pump 每 tick 重新讀 matchMedia 只餵 Director/micro-motion，timeline/playfield 的 reduced 旗標執行中不更新。
- lib.rs:1433-1446 toggle_companion_window 隱藏視窗後不通知 runtime（無 presentation_hello(false)），CompanionApp 也未監聽 companion-visibility（grep 0）；receptor 停止依賴 WebView document.hidden 心跳（10s）或 PRESENCE_STALE_SECS=20s 逾時。
- 硬體裝置事件無專屬角色映射：machine.ts:198-199 所有 receptor.observation → listening 1.5s。
- 無 TODO/FIXME/skip/only（grep companion/ 與 6 個測試檔 0 命中，僅 CompanionApp.tsx:1111 input placeholder 屬性）；pnpm typecheck exit 0；cargo test 輸出無 warning 行。
- 親跑數字：pnpm vitest run behavior/companion/rig/agent-intent/playfield/presentation = 6 files 74/74 pass；cargo test -p interaction-runtime --lib presentation 6/6；--test presentation_loop 12/12；--test planner_presentation 5/5；cargo test -p interaction-events 2/2。全部為程序內 fixture，無真機/真視窗。

### Phase 3 遊戲互動（VS Code Pets 基準＋超越）（45 列）

| Phase | 需求 | 程式存在 | 已接線 | 測試通過 | 真環境驗證 | 缺口 | 嚴重度 |
|---|---|---|---|---|---|---|---|
| Phase 3 | §3.1 玩家與角色閉環：靠近／點擊／拖曳／投擲玩具／輸入文字 → 本機感知 → Director 選反應 → 眼耳尾身表情氣泡音效組合呈現 → 必要時才叫 AI → 自然回前狀態 | 部分 | 是 | 部分 | 未驗證 | Director.react() 從未被 App 呼叫（只有 rig.test.ts:282 呼叫），語意反應層 peek/lean-in/await-player/block-cursor/caught-slacking 在真 App 不可達；hover 無氣泡；音效只在 runtime sound-play 命令時才有，本機閉環無音效；沒有角色視窗的 e2e（app.spec.ts:109-114 只驗證「尚未收到回報」） | 🟡 medium |
| Phase 3 | §5.1-1 自主散步、奔跑、停下、坐下、趴下、休息與睡覺 | 部分 | 是 | 部分 | 僅模擬器 | 無自主奔跑（只有追玩具時加速）；sit/sleep 表情存在但永遠不會被排程；「停下」只是 stroll 到點 | 🟡 medium |
| Phase 3 | §5.1-2 對滑鼠游標靠近、進入、離開、點擊、連點產生反應 | 部分 | 是 | 無 | 未驗證 | 「離開」無反應；「靠近」與「進入」是同一事件（僅 canvas enter）；handler 零測試 | 🟡 medium |
| Phase 3 | §5.1-3 Hover／靠近時可顯示短氣泡，但不能每次都打擾 | 否 | n/a | 無 | n/a | 完全沒有 hover 氣泡機制；唯一相關節流是 30s pointer-approached 遙測 | 🟡 medium |
| Phase 3 | §5.1-4 可投擲玩具；角色會預備、追逐、撲抓、撿回或拒絕歸還 | 是 | 是 | 部分 | 僅模擬器 | 帶回分支未測；CharPlayMode "carry" 是死值（playfield.ts:32，從未賦值） | 🟢 low |
| Phase 3 | §5.1-5 可用滑鼠決定投擲方向、速度與落點 | 是 | 是 | 通過 | 僅模擬器 | 落點由物理決定而非玩家瞄準；玩具初生位置固定 playfield.ts:136 | 無 |
| Phase 3 | §5.1-6 支援多角色／小型使魔的架構 | 是 | 是 | 部分 | 僅模擬器 | 使魔是 5 態小精靈非完整 rig；單一視窗內多角色（多視窗未做，gap matrix:105 已自述） | 🟢 low |
| Phase 3 | §5.1-7 多角色能互相注意、打招呼、出現愛心、追逐及玩耍 | 部分 | 是 | 部分 | 僅模擬器 | 是真的有行為（非只有資料結構）但很薄：主角單向被看、追逐無目標追蹤、無「玩耍」互動 | 🟡 medium |
| Phase 3 | §5.1-8 可選角色外觀、顏色與名稱 | 是 | 是 | 通過 | 僅模擬器 | 顏色僅 3 固定配色，無自訂色 | 無 |
| Phase 3 | §5.1-9 可個別顯示、隱藏或移除角色 | 部分 | 是 | 部分 | 未驗證 | 使魔不可個別隱藏（只能移除） | 🟢 low |
| Phase 3 | §5.1-10 可匯入／匯出角色設定與互動偏好 | 是 | 是 | 通過 | 未驗證 | 匯出用 <a download> data: URI（CompanionPage.tsx:909-912），lib.rs 無 on_download handler（grep → 0）；Tauri/WKWebView 下 download 屬性是否真的存檔未驗證 | 🟢 low |
| Phase 3 | §5.1-11 可切換桌面巢穴、工作桌、窗台、夜間等場景；透明桌面模式仍須正常 | 是 | 是 | 部分 | 僅模擬器 | 場景只是低調小道具（軟墊/桌線/盆栽/星星），可接受；無視覺測試 | 🟢 low |
| Phase 3 | §5.1-12 Roll Call「現在大家在做什麼」用人類語言 | 是 | 是 | 部分 | 未驗證 | Rust 端 rollCall 清洗無單元測試；CompanionPage.tsx:1069 key={r.name} 同名會撞 key | 🟢 low |
| Phase 3 | §5.2-1 拖曳小樞時有被抱起、懸空、掙扎或好奇反應 | 部分 | 是 | 無 | 未驗證 | 只有單一 lifted 變體，無掙扎/好奇分支 | 🟢 low |
| Phase 3 | §5.2-2 放下時依速度、高度與位置選擇站穩、踉蹌、滑倒或輕巧落地 | 否 | 部分 | 無 | 未驗證 | 無任何依速度/高度/位置的選擇邏輯；4 種落地只做 1 種 | 🟡 medium |
| Phase 3 | §5.2-3 小樞可以坐在視窗邊緣、從螢幕邊緣探頭、躲到視窗後再出現 | 否 | n/a | 無 | n/a | 整條 bullet 未實作（peek 表情 expressions.ts:385 存在但無觸發） | 🟠 high |
| Phase 3 | §5.2-4 對視窗開啟、關閉、移動、下載完成、測試失敗、任務完成做語意反應 | 部分 | 部分 | 部分 | 僅模擬器 | 6 項只有「任務完成」有；其餘 5 項無感知來源、無映射 | 🟠 high |
| Phase 3 | §5.2-5 能接住拖入檔案，先顯示檔名、大小、類型、資料去向與可讀 Agent，再確認 | 部分 | 是 | 部分 | 未驗證 | 缺大小、類型、可讀 Agent 三項；資料去向是固定句非動態 | 🟡 medium |
| Phase 3 | §5.2-6 Agent 工作時可抱著檔案、閱讀、書寫、翻找、等待、戳進度條 | 部分 | 是 | 通過 | 僅模擬器 | 6 種工作演出只有等待/翻找/工作 3 類；無檔案相關與進度條互動 | 🟢 low |
| Phase 3/5 | §5.2-7 硬體上線、離線、執行動作時有對應表演 | 部分 | 部分 | 無 | 未驗證 | 上線/離線完全無表演；執行動作只靠通用 action.* 映射 | 🟡 medium |
| Phase 3 | §5.2-8 長時間無操作時自然進入休息；玩家回來時注意到，但不必每次說話 | 部分 | 是 | 部分 | 僅模擬器 | 「回來注意到」不是由游標回來驅動，只有文字；休息中被喚醒無專屬演出 | 🟢 low |
| Phase 3 | §5.2-9 每個高頻反應至少 3～6 個變體，具防重複與冷卻 | 部分 | 是 | 部分 | 僅模擬器 | 高頻「反應」無變體池；防重複/冷卻只存在於 ambient 層 | 🟡 medium |
| Phase 3 | §5.2-10 玩耍、主動靠近、氣泡、音效、追逐游標、桌面移動均可分別關閉 | 部分 | 是 | 部分 | 僅模擬器 | 6 項可分別關閉只做 4；氣泡耦合表現度、音效無開關 | 🟡 medium |
| Phase 3 | §5.3 毛球 | 是 | 是 | 通過 | 僅模擬器 | none | 無 |
| Phase 3 | §5.3 紙團 | 是 | 是 | 通過 | 僅模擬器 | none | 無 |
| Phase 3 | §5.3 光點 | 是 | 是 | 通過 | 僅模擬器 | none | 無 |
| Phase 3 | §5.3 逗貓棒 | 是 | 是 | 部分 | 僅模擬器 | 拍打分支未測 | 🟢 low |
| Phase 3 | §5.3 小紙飛機 | 是 | 是 | 部分 | 僅模擬器 | 滑翔物理未測 | 🟢 low |
| Phase 3 | §5.3 可拖曳的小物件（第 6 種玩具） | 否 | n/a | 無 | n/a | 規格第 6 種玩具完全不存在（文件自述 5 種但未標為已知限制） | 🟠 high |
| Phase 3 | §5.3 玩具資料模型 9 欄位：位置、速度、重力、碰撞、抓取狀態、擁有者、角色興趣值、冷卻、生命週期；輕量 2D 物理 | 部分 | 是 | 通過 | 僅模擬器 | 重力與碰撞非資料模型欄位（實作可用，欄位語意不完整） | 🟢 low |
| Phase 2/3 | §15.2-1 Behavior Intent priority | 是 | 是 | 通過 | 僅模擬器 | none | 無 |
| Phase 2/3 | §15.2-2 Interaction interruption／resume | 是 | 是 | 通過 | 僅模擬器 | none | 無 |
| Phase 2/3 | §15.2-3 不同通道動畫混合 | 是 | 是 | 部分 | 僅模擬器 | 通道混合本身無斷言 | 🟢 low |
| Phase 2/3 | §15.2-4 高頻事件 bounded | 是 | 是 | 部分 | 僅模擬器 | UI 層節流無測試 | 🟢 low |
| Phase 2/3 | §15.2-5 同一動畫防重複 | 是 | 是 | 通過 | 僅模擬器 | none | 無 |
| Phase 3 | §15.2-6 點擊、連點、hover、拖曳、放下 | 部分 | 是 | 無 | 未驗證 | 五種互動 handler 完全無測試 | 🟡 medium |
| Phase 3 | §15.2-7 投擲軌跡、碰撞、追逐、抓取、帶回 | 是 | 是 | 部分 | 僅模擬器 | 帶回分支未測 | 🟢 low |
| Phase 3 | §15.2-8 多角色互相注意與追逐 | 部分 | 是 | 部分 | 僅模擬器 | 追逐未測且實作僅固定方向 | 🟡 medium |
| Phase 3 | §15.2-9 角色隱藏／恢復 | 是 | 是 | 部分 | 未驗證 | 隱藏後遊玩狀態是否恢復無測試 | 🟢 low |
| Phase 2/3 | §15.2-10 Reduced Motion | 是 | 是 | 通過 | 僅模擬器 | none | 無 |
| Phase 2/3 | §15.2-11 Quiet Hours | 是 | 是 | 通過 | 僅模擬器 | playfield quiet 路徑無測試 | 🟢 low |
| Phase 3 | §15.2-12 Fullscreen | 否 | n/a | 無 | n/a | 無全螢幕偵測／降級／隱藏邏輯，亦無測試 | 🟡 medium |
| Phase 3 | §15.2-13 低效能降級 | 否 | n/a | 無 | n/a | 無幀時間監測、無降級路徑、無測試 | 🟡 medium |
| Phase 4 | §15.2-14 claimed-completed 不播放 verified | 是 | 是 | 通過 | 僅模擬器 | none | 無 |
| Phase 3 | §15.2-15 emergency 搶占所有遊戲動畫 | 是 | 是 | 通過 | 僅模擬器 | 接線層無整合測試 | 🟢 low |

**診斷（TODO／placeholder／mock 冒充／hard-coded success／空函式／跳過測試／警告）：**

- 【缺項】第 6 種玩具「可拖曳的小物件」不存在：playfield.ts:11 ToyKind 只有 5 種；CompanionApp.tsx:959-966 玩具列 5 顆；CHANGELOG.md:151／gap matrix:105 只寫 5 種且未列為已知限制。
- 【死碼】CharPlayMode "carry"（playfield.ts:32）從未被賦值，rollCall :549 的 carry 分支不可達。
- 【未接線】InteractionDirector.react()（director.ts:122-130）在 App 零呼叫（僅 rig.test.ts:282），REACTION_EXPRESSIONS 的 peek/lean-in/await-player/caught-slacking/block-cursor 真 App 不可達；slip-play-cool/startled-awake/pretend-not-hear/deadpan/sit/sleep 表情已定義（expressions.ts）但 grep 全 src 零觸發，只出現在 36 表情預覽格——「36 表情存在」≠「36 表情會播」。
- 【硬編碼】放下小樞一律 wobbly-landing（CompanionApp.tsx:780），規格要求依速度/高度/位置選 4 種落地。
- 【規格縮水】拖入檔案預覽只顯示檔名（CompanionApp.tsx:932）＋固定去向句（:938），無大小/類型/可讀 Agent。
- 【無驗證】Rust companion_hit_rect（lib.rs:1565-1574）接受任意 f64，不 clamp 到視窗尺寸；WebView 可把整個視窗變成不可穿透。
- 【React key】CompanionPage.tsx:1069 RollCall 用 key={r.name}，使魔同名會撞 key。
- 【未驗證】匯出走 <a download> data: URI（CompanionPage.tsx:909-912），lib.rs 無 on_download handler；WKWebView 是否真的存檔未驗證。
- 【測試空洞】無 StageRenderer 測試；CompanionApp pointer handlers（click/連點/hover/拖曳/放下）零測試；playfield 帶回(return)/逗貓棒拍打/紙飛機滑翔/使魔追逐/quiet 路徑無斷言；四個 toggle 關閉行為無斷言；Rust behavior_telemetry 測試（presentation.rs:1068-1106）未含 rollCall 欄位。
- 【實作薄】使魔 chase 方向在開始時固定不追蹤（playfield.ts:522-529）；greet 目標取 others[0] 非最近（:504-508，註解說「最近」）；主角 stepChar 完全不引用 familiars，多角色互動為單向。
- 【缺開關】音效無任何偏好（grep companionSound → 0）；氣泡關閉耦合 expressiveness=quiet（packs.ts:190），非獨立開關。
- 【缺感知】machine.ts:188-258 無 provider.*／hardware.* 事件映射；runtime 有 provider.state-changed/paired/revoked 但角色不反應。
- 【e2e 覆蓋】app.spec.ts:109-114 Roll Call 只驗「尚未收到角色視窗的回報」誠實路徑；無角色視窗 e2e。
- 【實跑數字】pnpm vitest run 全套：14 files / 138 tests 全通過；6 個角色相關檔 74/74；cargo test -p interaction-runtime --lib presentation：6 passed / 34 filtered；pnpm typecheck 乾淨；cargo build -p interaction-runtime 0 warnings。未啟 daemon、未跑 e2e、無真機。

### Phase 4 AI 角色閉環（Codex／Claude Code）（31 列）

| Phase | 需求 | 程式存在 | 已接線 | 測試通過 | 真環境驗證 | 缺口 | 嚴重度 |
|---|---|---|---|---|---|---|---|
| 4 | §3.2 step1 玩家向小樞提出任務（角色入口） | 是 | 部分 | 無 | 未驗證 | 小樞 input cannot create an Agent Session; when no session is open the provider reply just says 「到「工作」頁建立 AI 工作階段」(conversation.ts:87-88). suggestDelegate flag is computed but never consumed (CompanionApp.tsx:1062-1068 uses only reply/behaviorIntent). | 🟡 medium |
| 4 | §3.2 step2 顯示交給 Codex 或 Claude Code、資料範圍、workdir、工具與取消方式（預覽） | 部分 | 部分 | 無 | 未驗證 | Preview is static UI copy, not computed from runtime/policy (no preview API). Tool scope shown only as prose (唯讀／計畫；不能寫入檔案), actual --tools list (Read,Glob,Grep[,Edit,Write], claude.rs:87-104) not displayed. 小樞 route note lacks workdir/tools/cancel. Codex maxC | 🟡 medium |
| 4 | §3.2 step3 建立真 Agent Session（真子程序） | 是 | 是 | 通過 | 僅模擬器 | All runtime-level coverage uses tests/fixtures/fake_claude.sh / inline fake-codex.sh. Real claude 2.1.250 and codex-cli 0.150.1 are installed on this machine but no real session was exercised in this session; no real codex app-server (JSON-RPC) fixture exists  | 🟡 medium |
| 4 | §3.2 step4 小樞依真實事件呈現 queued／fetched／working／waiting-for-consent／claimed-completed／verified／failed／unknown | 部分 | 是 | 部分 | 僅模擬器 | (1) waiting-input never emitted in real flows: GatewayEvent::TaskWaitingForInput is produced by no connector (only defined lib.rs:89, consumed gateway.rs:280). (2) timed-out only reachable via manual POST report; lease expiry (agents.rs:455-480 expire_if_neede | 🟠 high |
| 4 | §3.2 step5 只有獨立驗證後才播放 verified 成功演出 | 是 | 是 | 部分 | 僅模擬器 | Scope guard for /verify is correct by code reading but unasserted; add explicit 403 tests for agent token + session token and a rejection test for report(event=verified). | 🟢 low |
| 4 | §3.2 step6 結果以角色可理解的方式交付，同時保留技術詳情入口 | 部分 | 部分 | 無 | 僅模擬器 | No character-level delivery of the result (no bubble text on agent.session.state claimed/verified/failed); user must open AiPage → expand session to see summary. | 🟡 medium |
| 4 | §7.4 閱讀、思考、快速書寫、操作工具、翻找資料 | 部分 | 部分 | 部分 | 僅模擬器 | Tool phases (read/write/tool/search) are collapsed into 'working'; 閱讀/快速書寫/操作工具 have no distinct performance. | 🟢 low |
| 4 | §7.4 等待 Codex、等待 Claude、等待玩家確認 | 是 | 是 | 通過 | 僅模擬器 | wait-* plays on 'created' for 6 s only (durationMs 6000); a long-idle queued session does not keep the waiting pose. waiting-input never actually fires (see step4). | 🟢 low |
| 4 | §7.4 狀態清單 queued、fetched、working、waiting、blocked、claimed-completed、verified、failed、unknown、cancelled | 部分 | 部分 | 部分 | 僅模擬器 | 'blocked' and 'unknown' are never emitted for agent sessions (policy refusals at create return an error without event; unknown outcomes are labelled failed). | 🟡 medium |
| 4 | §7.4 claimed-completed 只能呈現「對方說完成了」；verified 才能出現綠色勾勾與正式成功演出 | 是 | 是 | 通過 | 僅模擬器 | none | 無 |
| 4 | §8.2 建立可插拔 Conversation Provider 介面（本輪不接模型 API） | 部分 | 是 | 通過 | n/a | '可插拔' is nominal: no registry/selection/preference; no runtime-side (Rust) provider abstraction — logic lives in frontend JS (CLAUDE.md: 核心邏輯不進前端 JS). | 🟢 low |
| 4 | §8.2 簡短輸入理解 | 是 | 是 | 通過 | n/a | keyword/length heuristics only (by design for this round) | 無 |
| 4 | §8.2 決定是否回話 | 是 | 是 | 通過 | n/a | When reply is null the caller still shows fallback line('text-received') (CompanionApp.tsx:1066) — provider's 'do not reply' decision is overridden. | 🟢 low |
| 4 | §8.2 主動問候與一句短回應 | 部分 | 部分 | 部分 | n/a | Provider has no proactive (unprompted) greeting path; 主動 greeting not routed through the Provider. | 🟢 low |
| 4 | §8.2 根據近期情境選語氣與 behaviorIntent | 是 | 是 | 通過 | n/a | none | 無 |
| 4 | §8.2 判斷是否建議建立 Codex／Claude 任務；無 Provider 時降級為本機規則，不為普通反應啟動昂貴工作 Agent | 部分 | 部分 | 通過 | n/a | Suggestion is text-only; no affordance (deep-link/prefill) to create the session. Degradation invariant (no agent auto-start) holds by construction. | 🟢 low |
| 4 | §8.3 Discovery、版本、登入狀態 | 是 | 是 | 通過 | 未驗證 | Real binaries present (claude 2.1.250, codex-cli 0.150.1 shim) but discovery against them not exercised here; logged-in detection for codex is substring parsing of CLI text (codex.rs:80-91). | 🟢 low |
| 4 | §8.3 建立、續租、取消、interrupt、close、resume | 是 | 部分 | 部分 | 僅模擬器 | Resume not reachable from CLI; runtime→connector resume path and interrupt path untested. | 🟡 medium |
| 4 | §8.3 workdir、read/write scope、tool scope、資料範圍、成本、時間與取消方式預覽 | 部分 | 部分 | 部分 | 僅模擬器 | Preview is hard-coded prose, not derived from the effective policy/spec; tool list and data-scope bundling (memory_context_bundle at dispatch, agents.rs:530-546) not shown before consent. | 🟡 medium |
| 4 | §8.3 Mailbox 真實 fetched／working 狀態 | 部分 | 是 | 部分 | 僅模擬器 | 'fetched' honesty differs per connector: claude = pipe write_all Ok (claude.rs:276-285), codex_exec = spawn with argv (real), but codex app-server send_user_message returns Ok on mpsc enqueue to the writer task (codex.rs:433-452) BEFORE stdin write → fetched c | 🟡 medium |
| 4 | §8.3 Approval 對應人類 UI | 是 | 是 | 無 | 未驗證 | Only codex app-server can actually receive a decision; no fake app-server fixture so the human-approval loop is never executed in tests. AiPage fetches messages once on expand (no live refresh, AiPage:224-230) so a request can silently auto-deny at 300 s. Pend | 🟠 high |
| 4 | §8.3 將 Agent 事件標準化成 Character Behavior Intent | 是 | 是 | 通過 | 僅模擬器 | see §3.2 step4 for never-emitted states | 🟢 low |
| 4 | §8.3 Agent claim 永不冒充 Verified Receipt | 是 | 是 | 通過 | 僅模擬器 | none in logic; missing negative test that report(event='verified') and non-human tokens on /verify are refused | 🟢 low |
| 4 | §8.3 角色可利用 Agent 做事，但角色本身不持有不受限權限 | 是 | 是 | 通過 | 僅模擬器 | none | 無 |
| cross | §15.1 Agent connector 回歸 | 是 | n/a | 通過 | 僅模擬器 | no codex app-server protocol fixture; claude sample is a recorded shape (claude.rs:409) | 🟢 low |
| cross | §15.1 cancel 回歸 | 是 | 是 | 部分 | 僅模擬器 | interrupt (turn cancel without close) has no runtime/API test; TaskCancelled path of codex_exec untested | 🟢 low |
| cross | §15.1 process-tree 回歸 | 是 | 是 | 通過 | 僅模擬器 | pgid attribution is best-effort via ps snapshot (agents.rs:1055-1085); documented residual risk | 🟢 low |
| cross | §15.1 auth 回歸 | 是 | 是 | 通過 | 僅模擬器 | /verify and /report not explicitly asserted 403 for agent/session tokens | 🟢 low |
| cross | §15.1 receipt 回歸 | 是 | 是 | 通過 | 僅模擬器 | none | 無 |
| cross | §15.1 estop 回歸 | 是 | 是 | 通過 | 僅模擬器 | none | 無 |
| cross | §15.1 命令與數字（本 session 親跑） | 是 | n/a | 通過 | 僅模擬器 | CLI E2E and Playwright numbers not reproduced in this session | 🟢 low |

**診斷（TODO／placeholder／mock 冒充／hard-coded success／空函式／跳過測試／警告）：**

- crates/interaction-runtime/src/gateway.rs:55 `#[allow(dead_code)] summary` on PendingApproval with comment 「進階詳情面板將顯示；先保留」 — placeholder field never displayed.
- crates/interaction-agent-gateway/src/lib.rs:89 GatewayEvent::TaskWaitingForInput is produced by NO connector (claude.rs/codex.rs/codex_exec.rs never emit it); taxonomy state 'waiting-input' (gateway.rs:280) is dead in real flows.
- crates/interaction-runtime/src/agents.rs:455-480 expire_if_needed and :1000-1014 restore emit only session.stopped — no agent.session.state event on lease expiry; 'timed-out' taxonomy reachable only via manual POST /report.
- crates/interaction-agent-gateway/src/claude.rs:359-369 system/init → [SessionStarted, TaskAccepted]: 'working'/Active is reported at process start before any task is delivered (fake_claude.sh prints init immediately), so state order is working→fetched and an idle session displays 工作中.
- crates/interaction-agent-gateway/src/codex.rs:433-452 send_user_message returns Ok when the JSON-RPC line is enqueued into the writer mpsc, not when written to stdin; runtime then emits 'fetched' + delivered_at (gateway.rs:546-570) — fetched can be claimed for an unwritten message.
- crates/interaction-runtime/src/gateway.rs:443-455 SessionClosed without any claim is reported as 'failed' (「agent 程序已結束而未回報結果」); spec honesty ladder would label this unknown; no 'unknown'/'blocked' taxonomy states exist for agent sessions.
- apps/interaction-desktop/src/companion/CompanionApp.tsx:1062-1068 ConversationResult.suggestDelegate is computed but never consumed; provider's reply=null is overridden by line('text-received') at :1066.
- apps/interaction-desktop/src/companion/conversation.ts:111-113 activeConversationProvider() hard-returns LocalTemplateProvider — 'pluggable' has no selection mechanism; conversation logic lives in frontend JS.
- crates/interaction-cli/src/main.rs:364-392 `agents create` has no --resume / resume-provider-session flag; resume reachable only via API JSON and AiPage.
- apps/interaction-desktop/src/pages/AiPage.tsx:224-230 messages (incl. approval-request) are fetched once on expand with no SSE/poll refresh; approvals can auto-deny at 300 s (gateway.rs:28) unseen.
- No test asserts agent.session.state events are emitted by the runtime (grep AgentSessionState in crates/*/tests → 0 hits); mapping is tested only on the frontend side from hand-built payloads.
- No test covers runtime resume_provider_session_id path (all fixtures pass None), gateway_interrupt, gateway_resolve_approval, or the approval auto-deny sweep; no codex app-server fixture exists.
- No test that an agent/session token gets 403 on POST /v1/agent-sessions/{id}/verify or that report(event='verified') is rejected (api_e2e.rs:167-231 covers create/GET only).
- schemas/openapi.json golden contains no /v1/agent-sessions paths and no agent.session.state (grep → 0) — AI tool export only; the human control-plane routes/events are not part of any golden schema.
- docs/v05-capability-gap-matrix.md:66-67 lists Resume / Conversation Provider as 缺 while :109 says 已有 for the same items (baseline vs. post-Phase rows mixed in one table).
- All Phase 4 runtime tests run against tests/fixtures/fake_claude.sh (真子程序、假模型); no real Codex/Claude session exercised. Real binaries on this machine: claude 2.1.250, codex-cli 0.150.1 (codex is a shell shim).
- scripts/v03-cli-e2e.sh (verify section :154-161, gateway :217-236, estop :327-333) and Playwright not executed in this session (daemon start prohibited).
- cargo clippy -D warnings (core/agent-gateway/runtime/api/cli, all targets) and tsc --noEmit: no warnings/errors.

### Phase 5 真硬體（Serial／MQTT／BLE／ESP32）（39 列）

| Phase | 需求 | 程式存在 | 已接線 | 測試通過 | 真環境驗證 | 缺口 | 嚴重度 |
|---|---|---|---|---|---|---|---|
| 5 | §3.3 角色與硬體閉環：發現→配對與能力識別→人類選擇允許→小樞/玩家觸發→Adapter 執行→裝置回報／獨立 Observation→小樞依真實結果呈現成功/失敗/未知 | 部分 | 部分 | 部分 | 僅模擬器 | 閉環只在 pty 模擬器與內嵌 rumqttd 上驗過；「發現→配對」階段被跳過（宣告式裝置直接 Installed，hardware scan 列出的 /dev/cu.* 無法一鍵變成 adapter）；「獨立 Observation」未接上驗證引擎（state facts 無 actionId 關聯）；README:354-358 自承韌體未在真 ESP32 編譯/驗收。規格「至少交付一套可重現的 ESP32 參考硬體閉環，不能只停在 metadata scan」未達成真機 | 🟠 high |
| 5 | §9.1-1 USB Serial adapter | 是 | 是 | 部分 | 僅模擬器 | Serial 傳輸層本身零自動化測試；reconnect/timeout/cancel 只在 MockRawLink 層驗（protocol_honesty.rs）；真機 USB 未驗；serial.rs:42-43 的 ENOTTY 判斷把所有 io::Other 都當 pty 退回純檔案，任何普通檔案路徑都會被當成「serial 埠」開啟 | 🟠 high |
| 5 | §9.1-2 Bluetooth LE adapter | 部分 | 部分 | 無 | 未驗證 | 規格 §15.3 要求 BLE scan/connect/subscribe/disconnect/restore 測試——一條都沒有；ble.rs 無 disconnect/shutdown 方法（revoke 後連線不會斷）；每次 connect 都 spawn 新 notification task 未回收；Linux 不支援（誠實拒絕） | 🟠 high |
| 5 | §9.1-3 MQTT adapter | 是 | 是 | 部分 | 僅模擬器 | §15.3 要求的 MQTT reconnect、QoS、重複訊息測試不存在：mqtt_loop.rs:222-224 只有一段「同一 action id 重送→裝置 dedupe」註解、無程式碼無斷言；無 broker 斷線重連測試；QoS 只在程式碼指定未被斷言；LinkActuator 收到 dup:true 的 deduplicated 註記（link_caps.rs:212-214）無測試 | 🟠 high |
| 5 | §9.1-4 WebSocket adapter | 否 | 否 | 部分 | n/a | 未實作，僅誠實拒絕。CHANGELOG:79-116 Phase 5 未提及 WebSocket；gap matrix 亦無此列 | 🟡 medium |
| 5 | §9.1-5 HID adapter（平台允許範圍） | 否 | 否 | 部分 | n/a | HID 只有發現（metadata），無任何 adapter/capability 可執行；規格列為第 5 優先 | 🟡 medium |
| 5 | §9.1-6 Home Assistant bridge | 否 | 否 | 無 | n/a | 完全未實作、文件未提及、gap matrix 未列 | 🟡 medium |
| 5 | §9.1-7 ESP32／Arduino Reference Adapter | 部分 | 部分 | 部分 | 未驗證 | README:354-358 自承「尚未在真實 ESP32 硬體上編譯與驗證」；本機無 arduino-cli/platformio（which → not found），無法 typecheck 韌體；hardware.rs:852-854 Esp32Declaration 說明文字仍寫「可透過宣告式 HTTP/SSE adapter 加入」（過時） | 🟠 high |
| 5 | §9.1 每 Adapter 必備-1 Discovery | 部分 | 部分 | 部分 | 未驗證 | 發現結果與 adapter 建立完全脫鉤：掃到的 serial 埠無法配對/安裝（UI:160 明說），需手寫 YAML；MQTT 零 discovery；BLE 無主動掃描 API | 🟡 medium |
| 5 | §9.1 每 Adapter 必備-2 Stable identity 或誠實回報無法穩定識別 | 是 | 是 | 通過 | 僅模擬器 | 身分為裝置自報字串＋配對碼，非密碼學身分（providers.rs:211-218 註解自承）；hardware.rs:284-285 宣告列以 volatile serial 埠路徑當 fingerprint 來源，與「埠不是身分」原則相悖 | 🟢 low |
| 5 | §9.1 每 Adapter 必備-3 Pairing／verification | 是 | 是 | 部分 | 僅模擬器 | 配對碼明文寫在 YAML 或 env（README:251 範例明文）、線上明文（README:365）；gap matrix:107 宣稱「配對碼 HMAC/pair」但程式碼無 HMAC；Serial 通道配對永不重置（README:369） | 🟡 medium |
| 5 | §9.1 每 Adapter 必備-4 Capability Manifest | 部分 | 是 | 部分 | 僅模擬器 | 裝置 hello.caps 收到後完全未使用（無比對 YAML 宣告 vs 裝置實際能力），Manifest 純由人手寫 YAML；無「能力識別」自動化 | 🟡 medium |
| 5 | §9.1 每 Adapter 必備-5 Read／write schema | 部分 | 是 | 部分 | 僅模擬器 | 無正式 read/write JSON Schema（型別/範圍）；schemas/ golden 未更新（release.sh 才重生） | 🟢 low |
| 5 | §9.1 每 Adapter 必備-6 Timeout、cancel、reconnect、backoff | 是 | 是 | 部分 | 僅模擬器 | Serial/MQTT/BLE 三個傳輸的實際重連/退避路徑零測試；BLE 無退避、無 disconnect；serial supervisor 在 build() 時就開埠並永久重試（即使 provider disabled/revoked 也不關，lib.rs/link_caps.rs 無 shutdown 呼叫） | 🟠 high |
| 5 | §9.1 每 Adapter 必備-7 Idempotency／replay protection，適用時加入 nonce | 部分 | 是 | 部分 | 僅模擬器 | nonce 產生但裝置端不驗（README:370-371 自承）；dup 路徑無測試；裝置→host 訊息（state/ack）無 nonce/序號，舊 ack 同 id 可被誤配 | 🟡 medium |
| 5 | §9.1 每 Adapter 必備-8 硬體硬限制與 Runtime 限制 | 是 | 是 | 部分 | 僅模擬器 | 韌體硬限制只存在於未編譯/未燒錄的 .ino；所有 clamp 測試都是模擬器自己 clamp 自己 | 🟡 medium |
| 5 | §9.1 每 Adapter 必備-9 Acknowledged、Observed、Verified 的誠實區分 | 部分 | 是 | 部分 | 僅模擬器 | 硬體動作永遠停在 acknowledged→（超時）uncertain：LinkReceptor state 無 actionId，獨立觀察無法對應到動作，Observed/Verified 對硬體是死路；link_caps.rs:83-85,252-254 health()/status() 硬編 healthy，斷線時仍回報健康 | 🟡 medium |
| 5 | §9.1 每 Adapter 必備-10 模擬器與真硬體測試分開標示 | 是 | n/a | 通過 | 僅模擬器 | 標示誠實，但「真硬體」欄全空：無任何真機測試紀錄；docs/acceptance-evidence.md:235 仍寫「WS/MQTT/Serial/BLE 誠實拒絕（僅 HTTP/SSE 實作）」與現況矛盾 | 🟢 low |
| 5 | §9.2 ESP32 參考裝置-1 RGB LED | 是 | 部分 | 無 | 未驗證 | 未編譯（無 arduino-cli）、未燒錄 | 🟡 medium |
| 5 | §9.2 ESP32 參考裝置-2 按鈕 | 是 | 部分 | 無 | 未驗證 | 同上；模擬器不模擬按鈕事件 | 🟡 medium |
| 5 | §9.2 ESP32 參考裝置-3 距離感測器 | 是 | 部分 | 無 | 未驗證 | pulseIn 阻塞 30ms（README:374 自承）；未真機 | 🟡 medium |
| 5 | §9.2 ESP32 參考裝置-4 環境光 | 是 | 部分 | 無 | 未驗證 | 欄位名 lux 但語意為相對亮度；未真機 | 🟢 low |
| 5 | §9.2 ESP32 參考裝置-5 溫度感測器 | 是 | 部分 | 無 | 未驗證 | 未真機 | 🟢 low |
| 5 | §9.2 ESP32 參考裝置-6 震動馬達 | 是 | 部分 | 部分 | 僅模擬器 | 韌體本身未驗 | 🟡 medium |
| 5 | §9.2 ESP32 參考裝置-7 小型伺服馬達 | 是 | 部分 | 無 | 未驗證 | 模擬器無 servo；未真機 | 🟡 medium |
| 5 | §9.2 ESP32 參考裝置-8 蜂鳴器／小型揚聲器 | 是 | 部分 | 無 | 未驗證 | 模擬器無 buzzer；未真機 | 🟡 medium |
| 5 | §9.2 至少支援 BLE 與 Wi-Fi/MQTT 其中兩種連線 | 是 | 部分 | 無 | 未驗證 | Serial+MQTT 兩種為預設；BLE 需另裝庫且 runtime 僅 macOS/Windows；全部未真機；MQTT 無 TLS（README:365） | 🟡 medium |
| 5 | §9.2 韌體強制限制震動、伺服與聲音的強度、持續時間及頻率 | 是 | n/a | 無 | 未驗證 | 限制寫在未編譯的 .ino；伺服無「強度/持續時間」限制（只有角度與節流，物理上合理） | 🟡 medium |
| 5 | §9.2 可實際製作的參考韌體、接線圖、BOM、Flash 步驟與測試 | 是 | n/a | 無 | 未驗證 | README:354-358 明言未編譯未驗證；本環境無 arduino-cli 無法做最基本 typecheck | 🟡 medium |
| 5 | §9.3 以裝置為中心的 UI（小樞可以知道／小樞可以做，不先顯示 receptor/actuator） | 部分 | 是 | 無 | n/a | 無規格範例的 ✓/○ 逐能力授權狀態（如「震動（每次先詢問）」）；只列名稱不列 consent 狀態；硬體掃描列與 provider 卡片分開、非同一裝置視圖 | 🟡 medium |
| 5 | §9.3 UI 必須清楚顯示「只發現」「已配對」「已測試」「已啟用」的差異；掃描到 metadata≠連線完成 | 部分 | 部分 | 無 | n/a | 「已測試」狀態不存在於狀態機與 UI；serial/mqtt/ble 裝置實際只會經歷 installed→available 兩態；無「測試裝置」動作 | 🟠 high |
| 5 | §15.3-1 Serial reconnect／timeout／cancel | 部分 | 是 | 部分 | 僅模擬器 | 沒有任何以真實 serial 傳輸（含 pty）做的 reconnect/timeout/cancel 測試 | 🟠 high |
| 5 | §15.3-2 BLE scan／connect／subscribe／disconnect／restore | 部分 | 部分 | 無 | 未驗證 | 五項一條測試都沒有；disconnect 未實作 | 🟠 high |
| 5 | §15.3-3 MQTT reconnect、QoS 與重複訊息 | 部分 | 是 | 部分 | 僅模擬器 | 三項規格要求均無斷言 | 🟠 high |
| 5 | §15.3-4 Stable identity／無 stable ID | 是 | 是 | 通過 | 僅模擬器 | none（測試層面）；macOS /dev 列舉分支未被單元測試直接覆蓋 | 🟢 low |
| 5 | §15.3-5 Pairing、verification、nonce、replay | 部分 | 是 | 部分 | 僅模擬器 | nonce 與 replay 未被測試；韌體不驗 nonce | 🟡 medium |
| 5 | §15.3-6 Firmware hard limit＋Runtime limit | 是 | 是 | 部分 | 僅模擬器 | 韌體側 clamp 由模擬器代演，非韌體 | 🟡 medium |
| 5 | §15.3-7 acknowledged-only／independent observation | 部分 | 是 | 部分 | 僅模擬器 | 硬體「獨立觀察→observed」無法發生（無 actionId 關聯） | 🟡 medium |
| 5 | §15.3-8 ESP32 真硬體驗收與模擬器驗收分開 | 是 | n/a | 部分 | 僅模擬器 | 真硬體驗收欄空白；docs/acceptance-evidence.md 未記錄 59-check 執行結果，且 :235 內容過時 | 🟡 medium |

**診斷（TODO／placeholder／mock 冒充／hard-coded success／空函式／跳過測試／警告）：**

- tests/mqtt_loop.rs:222-224：「同一 action id 重送→裝置 dedupe」只有註解、沒有程式碼與斷言——dedupe/重複訊息實際未測試，但 CHANGELOG:110 與 gap matrix 把 MQTT 閉環列為已有
- firmware/esp32-companion/esp32-companion.ino:614 與 README:370-371：cmd.nonce「僅收下不驗」；replay 防護只靠 16 筆 id 環形緩衝；host 側 new_nonce（protocol.rs:99-103）產生後無任何一方驗證
- firmware README:354-358 自承韌體從未在真 ESP32 編譯/驗證；本機 which arduino-cli/platformio → not found，無法 typecheck 840 行 .ino
- crates/interaction-adapter-declarative/src/link_caps.rs:83-85 與 :252-254：LinkReceptor.health()/LinkActuator.status() 硬編 ComponentHealth::healthy()，與 serial connected flag／mqtt connected／ble session 完全無關（斷線仍回報健康）
- crates/interaction-adapter-declarative/src/serial.rs:41-43：ENOTTY 判斷為 `contains("typewriter") \|\| ErrorKind::Io(Other)`，任何 io::Other 錯誤都會退回 std::fs::OpenOptions 開檔——普通檔案路徑會被當 serial 埠成功開啟，與註解「只在 ENOTTY 時啟用」不符
- lib.rs:884-920 / :921-972：SerialRawLink::spawn / MqttRawLink::spawn 在 build() 即開埠/連 broker 並永久退避重試；provider disabled/revoke（providers.rs:276-300）不會關閉連線；lib.rs/link_caps.rs 無任何 shutdown 呼叫
- ble.rs：無 disconnect/shutdown；每次 connect（:157-167）spawn 新 notification task 未取消；無退避（ensure_open 直接重掃 6s）；零測試
- crates/interaction-runtime/src/providers.rs:191：宣告式（serial/mqtt/ble）裝置直接 state=Installed，跳過 Discovered/Unpaired/Paired；UI 的「只發現／已配對」人話（CapabilitiesHub.tsx:94-101）對這類裝置永遠不會顯示；ProviderState 無 Tested，規格四態缺「已測試」
- crates/interaction-runtime/src/hardware.rs:852-854：Esp32Declaration 說明文字「可透過宣告式 HTTP/SSE adapter 加入」已過時（現支援 serial/mqtt/ble）；hardware.rs:284-285 以 volatile serial 埠路徑當 declaration fingerprint 來源
- docs/acceptance-evidence.md:235 仍寫「WS/MQTT/Serial/BLE transport：宣告式引擎解析但誠實拒絕（僅 HTTP/SSE 實作）」，與程式碼現況矛盾；59-check CLI E2E 執行結果未記錄於 acceptance-evidence
- docs/v05-capability-gap-matrix.md:107 宣稱「配對儀式→已有（hello 身分＋配對碼 HMAC/pair）」——程式碼中無 HMAC（protocol.rs:216-231 明文 code 比對；firmware 常數時間比對），配對碼明文傳輸（README:365）
- protocol.rs:35-36 DeviceMsg::Hello.caps 解析後未使用：無「能力識別」比對 YAML 宣告 vs 裝置實際 caps
- executor.rs:786-875 observed 驗證只認 facts.actionId；LinkReceptor state facts（button/distanceMm/lux/tempC/vibeActive/servoAngle/led）無 actionId → 硬體動作永遠無法 Observed/Verified，最終超時標 uncertain
- scripts/esp32-serial-sim.py 只模擬 led.set/vibe.pulse（:79-90），不模擬 buzzer/servo/button 事件/rate-limit/自動推播；CLI E2E 因此未覆蓋 4/8 周邊
- Home Assistant bridge：整個 repo（crates/apps/docs/CHANGELOG）grep 零結果；WebSocket 只有 Transport enum＋誠實拒絕（lib.rs:139,323-331）；HID 只有 metadata 列舉無 adapter——§9.1 七項中三項完全未實作且 Phase 5 CHANGELOG 未提及
- schemas/ golden schema 未含 expectedDeviceId/serial/mqtt/ble 欄位（grep 0 結果），release.sh 才會重生
- 編譯／lint：親跑 cargo test -p interaction-adapter-declarative → 19 passed 0 failed 0 ignored、無 warning；cargo test -p interaction-runtime --lib hardware → 4 passed；cargo clippy -p interaction-adapter-declarative --all-targets -D warnings 無輸出；python3 -m py_compile sim OK；bash -n e2e OK。CLI E2E 59 checks 逐行計數確認存在（59 = 41 check + 14 if/then ok + 4 && ok；硬體相關 5 條），但依規則未親跑（會起 daemon）

### Phase 6 iPhone Mobile Provider（桌面端）（50 列）

| Phase | 需求 | 程式存在 | 已接線 | 測試通過 | 真環境驗證 | 缺口 | 嚴重度 |
|---|---|---|---|---|---|---|---|
| 6 | §10.1-1 Swift／SwiftUI 原生 App（可共用 Rust schema，不犧牲 iOS 權限與生命週期正確性） | 部分 | n/a | 無 | 未驗證 | 未經 xcodebuild 編譯、未經模擬器/真機；UIKit/CoreMotion/CoreBluetooth 相關型別正確性未知（README 自承） | 🟠 high |
| 6 | §10.1-2 Bonjour 自動發現 | 部分 | 部分 | 無 | 未驗證 | 廣播失敗 `if let Ok` 靜默吞掉（398/400/408），UI/CLI 看不出 Bonjour 是否在廣播；instance name 固定 "interact-ai"（402）兩台桌機同網會撞名；無測試 | 🟡 medium |
| 6 | §10.1-3 QR Code 或配對碼 | 是 | 是 | 部分 | 僅模擬器 | 無 UI vitest 覆蓋 MobileSection；QR 內 host 取自 local_lan_ip()（888-899），多網卡時可能給錯介面；payload 無電腦顯示名稱（537） | 🟢 low |
| 6 | §10.1-4 每台 iPhone 獨立金鑰與 challenge-response | 是 | 是 | 部分 | 僅模擬器 | 【暴力防護】6 位碼、一段配對期只允許一次嘗試、錯即作廢（703 take() + 725-739）✓。但 703 在 HMAC 驗證前就 take()：任何未認證 LAN 端點送 pair-request+亂 hmac 即可燒掉使用者的配對期（配對 DoS / 與真手機競速）。金鑰無過期（PairedDevice 62-70 無 expires_at） | 🟡 medium |
| 6 | §10.1-5 TLS WebSocket | 是 | 是 | 部分 | 僅模擬器 | 無 client auth；bind 0.0.0.0 所有介面；無 idle timeout/伺服器端 ping、無連線數上限——未認證端可無限持連；無「指紋不符即拒」負向測試 | 🟡 medium |
| 6 | §10.1-6 iPhone 清楚顯示連接的電腦、能力、活動中感測器與立即中斷 | 部分 | n/a | 無 | 未驗證 | 桌面不提供電腦顯示名稱，手機只能以 IP:port 辨識；手機無「桌面能力清單」顯示（grep capabilit 只有感測）；未編譯/未真機 | 🟢 low |
| 6 | §10.1-7 斷線後能力自動 unavailable；重連不得自動恢復高風險能力 | 部分 | 部分 | 無 | 僅模擬器 | 【高風險不自動恢復＝只靠手機】桌面重連時無任何 re-gate：iphone.mic-level 一旦被人類 enable（registry/lib.rs:104-110）就跨斷線/重連/estop 持續 enabled；stop_all_sensors（sensors.rs:138-144）只停內建 mic，不動 iphone.mic-level；手機端 stop-all（ActuatorCenter.swift:442-459）也不停感測。僅 iOS 端斷線自停（ConnectionManager.swift: | 🟠 high |
| 6 | §10.1-8 桌面 Consent 不能取代 iOS 系統權限 | 部分 | 部分 | 無 | 未驗證 | 桌面 UI 未顯示 iOS 權限狀態；桌面 capabilities 對 4 個 iphone.* receptor 一律以「連線」判可用（443-458），不依手機回報的 sensors/permissions 標 unavailable | 🟡 medium |
| 6 | §10.2-1 Touch／gesture | 是 | 是 | 無 | 僅模擬器 | 僅 tap/longpress，無 gesture（swipe/drag）語意；桌面無任何 recipe/director 消費 iphone.touch（全 repo grep 無 iphone.* 引用） | 🟢 low |
| 6 | §10.2-2 Accelerometer | 部分 | 是 | 部分 | 僅模擬器 | 無獨立 accelerometer receptor（依規格語意化併入 motion 屬合理），但桌面測試未驗證 ingest 結果 | 🟢 low |
| 6 | §10.2-3 Gyroscope／device motion／orientation | 部分 | 是 | 無 | 未驗證 | 無 orientation（portrait/landscape/faceUp）事件 | 🟢 low |
| 6 | §10.2-4 Battery／charging／foreground state | 是 | 是 | 無 | 僅模擬器 | 無測試 | 🟢 low |
| 6 | §10.2-5 Microphone level／audio，需權限 | 部分 | 部分 | 無 | 僅模擬器 | 【consent 追蹤】ingest 全程不查 ConsentScope（runtime.rs:531-590 無 has_consent），閘門只是 receptor enable 旗標；enable 後跨斷線/重連/estop 不重置（見 §10.1-7）；被丟棄的觀察無事件/audit | 🟠 high |
| 6 | §10.2-6 Camera／QR／capture，需權限 | 否 | 否 | 無 | n/a | 無 camera receptor／capture 動器；CHANGELOG 未宣稱（誠實缺） | 🟡 medium |
| 6 | §10.2-7 Location／geofence，需權限 | 否 | 否 | 無 | n/a | 無 location observation／geofence；誠實未冒充 | 🟡 medium |
| 6 | §10.2-8 BLE device discovery／state | 部分 | 部分 | 無 | 未驗證 | 無 BLE discovery/state 事件型 receptor（周邊出現/消失/連線狀態不進 observation） | 🟡 medium |
| 6 | §10.2-9 Local-network device events | 否 | 否 | 無 | n/a | 完全未實作 | 🟡 medium |
| 6 | §10.2-尾 不可用感測器依機型／系統 API 誠實標示 unavailable | 部分 | 部分 | 無 | 未驗證 | 桌面層對無 motion/無 torch 機型仍顯示 Available；只有手機自報字串在 UI 另行顯示（連線中才顯示） | 🟡 medium |
| 6 | §10.3-1 Character presentation | 部分 | 部分 | 無 | 僅模擬器 | 只是裸 actuator；未接到小樞/Director | 🟡 medium |
| 6 | §10.3-2 Custom haptic | 是 | 是 | 部分 | 僅模擬器 | 桌面 manifest 未設 ActuatorLimits（173-192 無 .limits()）→ 無 max_per_hour；channel cooldown 依 policy 設定；頻率限制只在手機端 | 🟢 low |
| 6 | §10.3-3 Notification | 是 | 是 | 無 | 僅模擬器 | 無測試 | 🟢 low |
| 6 | §10.3-4 Audio／SFX | 否 | 否 | 無 | n/a | 無短音效動器 | 🟢 low |
| 6 | §10.3-5 TTS | 是 | 是 | 無 | 僅模擬器 | 無測試 | 🟢 low |
| 6 | §10.3-6 Screen color／flash effect | 是 | 是 | 無 | 僅模擬器 | 桌面 manifest 未宣告參數 schema（AI 不知道要給 color） | 🟢 low |
| 6 | §10.3-7 Torch，需明確用途與限制 | 部分 | 是 | 無 | 僅模擬器 | 桌面 manifest 無 max_duration_ms/用途說明（description 通用 174）；限制只在手機端硬編 | 🟢 low |
| 6 | §10.3-8 Live Activity／鎖定畫面狀態 | 否 | 否 | 無 | n/a | 未實作 | 🟢 low |
| 6 | §10.4-1 BLE GATT 掃描、連線、Service／Characteristic 探索、read、write、subscribe（第一優先） | 部分 | 部分 | 無 | 未驗證 | 桌面無 connect/discover/read/write/subscribe 發送方法；CHANGELOG:63-64「ble.scan/connect/gatt 協定訊息」在桌面端只有 scan；ble.result 處理無 authed 守門 | 🟠 high |
| 6 | §10.4-2 Bonjour／HTTP／WebSocket／MQTT 區域網路裝置（經 iPhone） | 否 | 否 | 無 | n/a | 未實作 | 🟡 medium |
| 6 | §10.4-3 External Accessory 僅用於明確支援的 MFi／廠商配件 | 否 | n/a | 無 | n/a | 未實作亦未宣稱（可接受） | 無 |
| 6 | §10.4-4 不得宣稱 iPhone 可任意操作所有 USB／Lightning／USB-C 裝置 | 是 | n/a | 無 | n/a | none | 無 |
| 6 | §10.4-5 ESP32 的 iPhone 連線優先 BLE 或 Wi-Fi，不以 USB Serial 為第一版 | 部分 | 否 | 無 | 未驗證 | 通用 BLE 通道存在但無 ESP32 經手機閘道的閉環 | 🟡 medium |
| 6 | §10.5-1 小樞可從桌面「前往 iPhone」，須對應真實 connected presentation surface | 否 | 否 | 無 | n/a | 未實作；iphone.character actuator 存在但無 surface 概念；多機時 send_to_any（113-122）取 BTreeMap 第一台，無法指定哪支手機 | 🟠 high |
| 6 | §10.5-2 iPhone 被拿起時可觸發桌面小樞注意（授權與設定允許下） | 否 | 否 | 無 | n/a | 未實作 | 🟡 medium |
| 6 | §10.5-3 桌面任務進行時 iPhone 顯示簡化角色狀態與必要確認 | 否 | 否 | 無 | n/a | 未實作（無確認流程回傳） | 🟡 medium |
| 6 | §10.5-4 iPhone haptic 可表現輕敲、呼嚕、心跳、提醒，可分別關閉並限制頻率 | 部分 | 部分 | 無 | 未驗證 | 「分別關閉」桌面端無；頻率限制只在手機端且未測 | 🟡 medium |
| 6 | §10.5-5 不保存原始 motion 軌跡；輸出 lifted/shaken/placed/rotated 語意事件 | 是 | 是 | 部分 | 僅模擬器 | 白名單只擋 receptor id，不擋 facts 內容（iphone.motion facts 可塞任意 JSON 進 store） | 🟢 low |
| 6 | 特別核對：撤銷是否讓現有連線立刻斷 | 部分 | 是 | 部分 | 僅模擬器 | 【blocker】撤銷後已連線手機仍可持續送 observation（812-822）與 status（830-836）並被 ingest；UI 文案「立即斷線」與 mobile.rs:9,593 註解不實；之後真斷線時 Revoked→Disconnected 為非法轉移（provider.rs:101-128）被 `let _` 吞掉 | 🔴 blocker |
| 6 | 特別核對：ack/err/ble.result 是否有 authed 守門；未認證連線能做什麼 | 部分 | 是 | 無 | 僅模擬器 | 未認證端可：(a) 以已知/猜中的 id 冒充 ack 解除任何 pending act（ble-scan id 僅 4 bytes hex 867）；(b) 燒掉配對期（703）；(c) 無限持連（無 idle timeout/上限）。ack 也不綁定送出裝置——多機時 B 可替 A 回 ack | 🟠 high |
| 6 | 特別核對：多裝置 send_to_any 語意 | 部分 | 是 | 無 | 僅模擬器 | 無法指定目標手機；receipt 未記錄實際送達哪台（note 只有 transport）；健康度 any_connected 讓「至少一台連線」即全部 Available | 🟡 medium |
| 6 | 特別核對：estop 廣播是否需握手 | 部分 | 是 | 部分 | 僅模擬器 | 無握手：queue 滿或已斷線時 stop-all 靜默失敗仍計為已停止（違反「結果未知不得顯示成功」）；estop 也不停手機感測（見 §10.1-7） | 🟡 medium |
| 6 | 特別核對：agent／session token 摸不到 /v1/mobile（含 session-scoped） | 是 | 是 | 無 | 僅模擬器 | 邏輯正確但零測試覆蓋；CHANGELOG:67 宣稱「agent token 連 GET 都拒」無回歸保護 | 🟢 low |
| 6 | §15.4-1 配對、撤銷、錯誤電腦、過期金鑰 | 部分 | 部分 | 部分 | 僅模擬器 | 4 項中 1.5 項有測試；過期金鑰未實作 | 🟠 high |
| 6 | §15.4-2 Bonjour discovery、TLS WebSocket reconnect | 部分 | 部分 | 部分 | 僅模擬器 | Bonjour 零測試；手機端 backoff 重連（ConnectionManager.swift:8）未驗 | 🟡 medium |
| 6 | §15.4-3 前景／背景／被系統終止後誠實狀態 | 部分 | 部分 | 無 | 未驗證 | 桌面無 server-side keepalive，非正常終止時 connected 誠實度依賴 TCP 逾時；零測試 | 🟡 medium |
| 6 | §15.4-4 Motion 語意事件 | 是 | 是 | 部分 | 僅模擬器 | 桌面測試名不符實；README 稱「等價案例在 macOS 跑過」但 repo 內無可重跑的證據腳本 | 🟡 medium |
| 6 | §15.4-5 Haptic frequency limit | 部分 | 部分 | 無 | 未驗證 | 桌面端無頻率硬限制、無測試 | 🟡 medium |
| 6 | §15.4-6 Camera／microphone／location permission denied／revoked | 部分 | 部分 | 無 | 未驗證 | 桌面對 denied/revoked 無反應（不停用 receptor、不顯示）；零測試 | 🟡 medium |
| 6 | §15.4-7 BLE gateway | 部分 | 部分 | 無 | 未驗證 | 零測試 | 🟠 high |
| 6 | §15.4-8 iPhone 斷線後 capability unavailable | 是 | 是 | 無 | 僅模擬器 | 零斷言；測試環境 spawn_watchdog=false 故 availability 永不刷新，即使加斷言也需直接呼叫 refresh_health | 🟡 medium |
| 6 | §15.4-9 iOS 真機驗收；Simulator 不冒充 sensor／BLE 真機證據 | 否 | n/a | 無 | 未驗證 | 未真機驗收；文件誠實標示，但 docs/v05-capability-gap-matrix.md:81-83 仍寫「全部未開始」與 113 行「已有」自相矛盾 | 🟠 high |

**診斷（TODO／placeholder／mock 冒充／hard-coded success／空函式／跳過測試／警告）：**

- [不實註解] mobile.rs:9「撤銷立即生效（連線關閉）」與 :593「立即斷線（drop outbound → send loop 結束 → 連線關閉）」：實際只 drop ConnState 的 Sender clone，handler 自身 out_tx（651）活到 861，連線不會關；UI CapabilitiesHub.tsx:311 文案「撤銷配對（立即斷線）」、CHANGELOG.md:54「撤銷立即斷線」、cli main.rs:440「disconnects immediately」同樣不實
- [測試名不符實] tests/mobile_loop.rs:6 檔頭宣稱覆蓋「斷線 → provider Disconnected」，三個測試皆無此斷言；:186-198 motion/白名單觀察零斷言（只 sleep）
- [缺守門] mobile.rs:823-829 ack/err 與 :837-843 ble.result/ble.value 無 authed 檢查；ack 不綁定發送裝置
- [hard-coded success] runtime.rs:823-832 emergency_stop 對 MobileActuator::emergency_stop（mobile.rs:267-272 永遠 Ok(())、broadcast 錯誤 let _ 吞）一律計 stoppedActuators+=1；6 個 iphone.* actuator 造成 6 次重複 stop-all
- [靜默丟棄] mobile.rs:821 `let _ = self.ingest(...)`：receptor disabled（mic-level 預設）或其他錯誤時無事件/audit
- [狀態卡死] mobile.rs:365 started.swap(true) 在 cert/bind 成功前設定；失敗後 started 永為 true、port None、不重試
- [與 CHANGELOG 矛盾] mobile.rs:351-352 只要 mobile-devices.json 存在（即使 devices 為空，撤銷全部後仍存在）就開 0.0.0.0 網路埠；CHANGELOG.md:54-55「沒配對過不開網路埠」
- [CHANGELOG 過度宣稱] CHANGELOG.md:63「ble.scan/connect/gatt 協定訊息」桌面端只有 scan 發送（mobile.rs:866-884）；:60-61「高風險感測不自動恢復」桌面端無強制，只在 iOS 端
- [文件自相矛盾] docs/v05-capability-gap-matrix.md:81-83「iPhone 全部未開始」 vs :113「已有（模擬 iPhone 驗收）」
- [未接線] POST /v1/mobile/ble/scan 只在 HTTP（api lib.rs:199）；CLI（main.rs:435-442）、Tauri（lib.rs:2122-2124）、transport.ts:268-270、api.ts:303-305、UI 皆無
- [UI 缺誠實顯示] CapabilitiesHub.tsx:292-299 只渲染 status.sensors，不顯示手機回報的 permissions（denied/revoked）
- [無界/無逾時] mobile.rs:639-863 WS 連線無 idle timeout、無伺服器端 ping、無連線數上限；:833 conn.status 直接存手機任意 JSON
- [配對 DoS] mobile.rs:703 在 HMAC 驗證前 take() 配對期：任何未認證 LAN 端點可燒掉使用者的配對碼
- [無測試覆蓋] api_e2e.rs:167-227 agent-token 403 清單無 /v1/mobile；無 session-token 測試；desktop vitest 14 檔無 mobile/iPhone；scripts/*.sh 無 mobile
- [親跑數字] cargo test -p interaction-runtime --test mobile_loop → 3 passed / 0 failed / 0 ignored, 1.36s；cargo clippy -p interaction-runtime --all-targets → 0 warning；swiftc -parse 14/14 swift 0 error；swiftc -typecheck Protocol.swift+PairingStore.swift exit 0（macOS SDK，非 iOS）；xcrun iphonesimulator SDK 不存在，無法編譯 iOS App
- [iOS 端 stop-all 不停感測] ActuatorCenter.swift:442-459 只停 haptics/tts/torch/flash；桌面 estop 亦不停 iphone.mic-level（sensors.rs:138-144 只停內建 mic）

### Phase 6 iPhone Mobile Provider（桌面端，第二位審計者）（45 列）

| Phase | 需求 | 程式存在 | 已接線 | 測試通過 | 真環境驗證 | 缺口 | 嚴重度 |
|---|---|---|---|---|---|---|---|
| 6 | §10.1 Swift/SwiftUI 原生 App；可共用 Rust schema 但不犧牲 iOS 權限/生命週期 | 部分 | n/a | 未知 | 未驗證 | iOS App 未編譯、未真機；桌面/手機協定無共用 schema（手寫字串對照） | 🟡 medium |
| 6 | §10.1 Bonjour 自動發現 | 是 | 部分 | 無 | 未驗證 | 服務實例名與主機名固定 'interact-ai'/'interact-ai.local.'(mobile.rs:400-404)→兩台電腦在同一 LAN 無法區分且 mDNS 主機名衝突；註冊失敗僅 let _ 不回報 | 🟡 medium |
| 6 | §10.1 QR Code 或配對碼 | 是 | 是 | 通過 | 僅模擬器 | local_lan_ip 找不到時 host 留白(536)；配對碼 rand::thread_rng %1e6 均勻性可接受 | 無 |
| 6 | §10.1 每台 iPhone 獨立金鑰與 challenge-response | 是 | 是 | 通過 | 僅模擬器 | ack/err/ble.result 無 authed 守門(823-843)；未認證 peer 可無限次 pair-request 取 nonce(676-691)且一次垃圾 pair-response 就燒掉人類的配對期(703)→LAN 任何人可 DoS 配對；無 auth deadline/idle timeout/連線數上限 | 🟡 medium |
| 6 | §10.1 TLS WebSocket | 是 | 是 | 通過 | 僅模擬器 | 無 ping/pong/heartbeat/read timeout（grep Ping\|Pong\|idle 於 mobile.rs → 0 命中）；key 先 std::fs::write 再 chmod(339,343) 短暫 0644 視窗 | 🟡 medium |
| 6 | §10.1 iPhone 清楚顯示連接的電腦、能力、活動中感測器與立即中斷 | 部分 | 部分 | 無 | 未驗證 | 桌面協定未提供電腦身分/能力清單給手機；Bonjour 名固定無法辨別電腦 | 🟡 medium |
| 6 | §10.1 斷線後能力自動 unavailable；重連不得自動恢復高風險能力 | 部分 | 是 | 部分 | 僅模擬器 | (a)只對乾淨關閉有效：無 heartbeat→半開 TCP 時 conns 不清、健康度續報 healthy、act 4 秒後 UNKNOWN；(b)重連不恢復高風險僅手機端強制（InteractionCompanionApp.swift:74-77）：桌面斷線時不重設 iphone.mic-level 的 registry enabled 旗標，重連即照收 | 🟠 high |
| 6 | §10.1 桌面 Consent 不能取代 iOS 系統權限 | 部分 | 部分 | 無 | 未驗證 | 權限拒絕/撤銷未反映到 receptor availability 與 UI | 🟡 medium |
| 6 | §10.2 Touch／gesture | 是 | 是 | 無 | 未驗證 | none（僅 kind 一欄，無 gesture 細分） | 🟢 low |
| 6 | §10.2 Accelerometer | 部分 | 是 | 部分 | 僅模擬器 | 無獨立 accelerometer receptor（設計上改語意事件，符合 §10.5）；facts 未做欄位白名單(817-821) | 🟢 low |
| 6 | §10.2 Gyroscope／device motion／orientation | 部分 | 是 | 無 | 未驗證 | 無 orientation 狀態 receptor | 🟢 low |
| 6 | §10.2 Battery／charging／foreground state | 是 | 是 | 無 | 未驗證 | none | 🟢 low |
| 6 | §10.2 Microphone level／audio，需權限 | 是 | 部分 | 無 | 未驗證 | 桌面 consent 閘門＝registry enable 旗標而非 session consent；啟用中的手機 mic 不出現在 activeSensors/tray（runtime.rs:100-101,441 僅本機 sensors）→違反「感測不靜默」；斷線不重設旗標 | 🟡 medium |
| 6 | §10.2 Camera／QR／capture，需權限 | 否 | 否 | 無 | n/a | 桌面端無 camera receptor（QR 只用於配對，手機端） | 🟡 medium |
| 6 | §10.2 Location／geofence，需權限 | 否 | 否 | 無 | n/a | 桌面端無 location receptor；手機推送會被白名單丟棄(814-816) | 🟡 medium |
| 6 | §10.2 BLE device discovery／state | 部分 | 部分 | 無 | 未驗證 | 無 BLE state 事件 receptor；掃描結果不進 observation 管線 | 🟡 medium |
| 6 | §10.2 Local-network device events | 否 | 否 | 無 | n/a | 未實作 | 🟡 medium |
| 6 | §10.2 不可用感測器依機型/系統 API 誠實標示 unavailable | 部分 | 部分 | 無 | 未驗證 | registry availability 不反映機型/權限差異，只反映連線 | 🟡 medium |
| 6 | §10.3 Character presentation | 是 | 是 | 無 | 未驗證 | 無 Interaction Director/presentation 掛鉤（grep iphone 於 presentation.rs → 0） | 🟡 medium |
| 6 | §10.3 Custom haptic | 是 | 是 | 通過 | 僅模擬器 | 桌面無 pattern 分類/頻率限制（見 §10.5 haptic 列）；測試繞過 executor 直呼 actuator.execute(238)，未覆蓋 policy/consent 管線 | 🟢 low |
| 6 | §10.3 Notification | 是 | 是 | 無 | 未驗證 | none | 🟢 low |
| 6 | §10.3 Audio／SFX | 否 | 否 | 無 | n/a | 未實作 | 🟢 low |
| 6 | §10.3 TTS | 是 | 是 | 無 | 未驗證 | none | 🟢 low |
| 6 | §10.3 Screen color／flash effect | 是 | 是 | 無 | 未驗證 | none | 🟢 low |
| 6 | §10.3 Torch，需明確用途與限制 | 部分 | 是 | 無 | 未驗證 | 無明確用途宣告/專屬上限（durationMs 上限、無 no-torch 機型的 unavailable 標示只靠手機 err no-torch → 受據 device-refused 236-242） | 🟢 low |
| 6 | §10.3 Live Activity／鎖定畫面狀態 | 否 | 否 | 無 | n/a | 未實作 | 🟢 low |
| 6 | §10.4 BLE GATT 掃描、連線、Service/Characteristic 探索、read、write、subscribe（第一優先） | 部分 | 部分 | 無 | 未驗證 | connect/discover/read/write/subscribe 桌面端缺；scan 無 CLI/UI；ble.result 無 authed 守門；scan id 32-bit(867) | 🟠 high |
| 6 | §10.4 Bonjour、HTTP、WebSocket、MQTT 區域網路裝置（第二優先，經 iPhone） | 否 | 否 | 無 | n/a | 未實作（桌面自身的 MQTT/Serial adapter 屬 Phase 5，非經 iPhone） | 🟢 low |
| 6 | §10.4 External Accessory 僅用於明確支援的 MFi 配件 | 否 | n/a | 無 | n/a | 未實作亦未宣稱；符合「僅明確支援」的保守面 | 無 |
| 6 | §10.4 不得宣稱 iPhone 可任意操作 USB／Lightning／USB-C | 是 | n/a | n/a | n/a | none | 無 |
| 6 | §10.4 ESP32 的 iPhone 連線優先 BLE/Wi-Fi | 否 | 否 | 無 | n/a | 未實作 | 🟢 low |
| 6 | §10.5 小樞「前往 iPhone」須對應真實 connected presentation surface | 部分 | 部分 | 無 | 未驗證 | 多機時 send_to_any 固定送 BTreeMap 最小 device_id 那台(115)，act 無 device 參數(135) | 🟡 medium |
| 6 | §10.5 iPhone 被拿起可觸發桌面小樞注意（授權與設定允許下） | 部分 | 否 | 無 | 未驗證 | 需使用者自寫 recipe；無開關設定 | 🟡 medium |
| 6 | §10.5 桌面任務進行時 iPhone 顯示簡化角色狀態與必要確認 | 否 | 否 | 無 | n/a | 未實作（character.present 可帶狀態文字但無確認回流） | 🟡 medium |
| 6 | §10.5 haptic 輕敲/呼嚕/心跳/提醒可分別關閉並限制頻率 | 部分 | 部分 | 無 | 未驗證 | 桌面無 pattern 分別關閉、無頻率上限 | 🟡 medium |
| 6 | §10.5 不保存原始 motion 軌跡；輸出 lifted/shaken/placed/rotated 語意事件 | 部分 | 是 | 部分 | 僅模擬器 | 白名單只管 receptor id，不管 facts 欄位→手機若在 iphone.motion 塞 x/y/z 桌面照存；語意分類與不存軌跡皆手機端保證 | 🟡 medium |
| 6 | §15.4 配對、撤銷、錯誤電腦、過期金鑰 | 部分 | 是 | 部分 | 僅模擬器 | revoke 不會斷開現有連線：587-606 只 remove conns（drop 的是 767-773/802-808 的 clone），handler 自有 out_tx(651) 至 861 才 drop→writer(652-659)不結束、read loop(670)續讀、authed 仍 Some→被撤銷手機仍可推 observation(812-822)/status(830-836)；593 註解與 CHANGELOG.md:54「撤銷立即斷線」不實；mobile_revoke 也不像 prov | 🟠 high |
| 6 | §15.4 Bonjour discovery、TLS WebSocket reconnect | 是 | 是 | 部分 | 僅模擬器 | Bonjour 無測試；無 heartbeat 故「掉線偵測」只靠 TCP FIN | 🟡 medium |
| 6 | §15.4 前景／背景／被系統終止後誠實狀態 | 部分 | 部分 | 無 | 未驗證 | 無 heartbeat→被系統終止但未 FIN 時桌面續報連線中 | 🟡 medium |
| 6 | §15.4 Motion 語意事件 | 是 | 是 | 部分 | 僅模擬器 | 桌面測試無 ingest 驗證 | 🟡 medium |
| 6 | §15.4 Haptic frequency limit | 否 | 否 | 無 | 未驗證 | 僅手機端 ActuatorCenter.swift:84,128 且未編譯/未驗 | 🟡 medium |
| 6 | §15.4 Camera／microphone／location permission denied／revoked | 部分 | 部分 | 無 | 未驗證 | 權限拒絕不改變 receptor availability；camera/location 桌面端不存在 | 🟡 medium |
| 6 | §15.4 BLE gateway | 部分 | 部分 | 無 | 未驗證 | 無測試；GATT 操作缺 | 🟠 high |
| 6 | §15.4 iPhone 斷線後 capability unavailable | 是 | 是 | 無 | 僅模擬器 | 無斷線→offline 斷言；半開連線不偵測 | 🟡 medium |
| 6 | §15.4 iOS 真機驗收；Simulator 不冒充 sensor／BLE 真機證據 | n/a | n/a | 無 | 未驗證 | 誠實標示到位；真機驗收 0；docs/v05-capability-gap-matrix.md:83 仍寫「全部未開始」與 :113「已有」自相矛盾 | 🟡 medium |

**診斷（TODO／placeholder／mock 冒充／hard-coded success／空函式／跳過測試／警告）：**

- cargo test -p interaction-runtime --test mobile_loop：3 passed / 0 failed（1.00s）——3 個測試皆為程序內模擬 iPhone，real_env=simulator-only
- [1 revoke 不斷線] mobile.rs:587-606 mobile_revoke 只 remove devices+conns；ConnState.outbound 是 out_tx.clone()（767-773,802-808），handler 原始 out_tx(651) 直到 861 才 drop→writer 652-659 的 out_rx.recv() 不回 None、sink 不 close；read loop 670 續跑、authed(647) 仍 Some→被撤銷手機可續推 observation(812-822)、status(830-836；get_mut 為 None 無害)。斷線後 851-858 轉 Disconnected 會因 Revoked→Disconnected 不合法（interaction-core/src/provider.rs:83-125）而失敗、狀態留 Revoked（僅此點正確）。測試 mobile_loop.rs:293 先 drop(ws) 再 313 revoke，未覆蓋此缺陷。CHANGELOG.md:54、mobile.rs:9,593 宣稱「撤銷立即斷線」不實。severity high
- [2 ack 無守門] mobile.rs:823-829 (ack/err) 與 837-843 (ble.result/ble.value) 無 `if authed.is_some()`、pending_acts 不綁 device→未認證 TLS peer 或另一台已配對手機可憑 id 解除 pending act 並偽造 deviceApplied(231-233)。act id=ActionId::generate() uuid v4（interaction-core/src/ids.rs:17-19）難猜；ble-scan id 32-bit(867)。severity medium（縱深防禦缺）
- [3 未認證能做什麼] 有效配對期內可無限 pair-request 取新 nonce(676-691，pending_pair 覆蓋)；任一 pair-response 先 pairing.take()(703) 再驗 HMAC→每期只准 1 猜（1e-6）防暴力 OK，但任何 LAN peer 可用垃圾 pair-response 燒掉人類的配對期（DoS）；pair-fail 後 continue(686,699,711,739) 連線不關；auth-fail 才 break(796)；無 auth deadline/idle timeout/連線數上限；再加 [2] 的 ack 注入。severity medium
- [4 mic-level consent] runtime.rs:531-590 ingest 無 session ConsentScope 檢查；閘門＝registry enabled（iphone.mic-level requires_consent=true mobile.rs:481-487,495 → interaction-registry/src/lib.rs:76 預設 disabled → lib.rs:122-131 receptor() Err Unavailable）；mobile.rs:821 `let _ = self.ingest` 吞錯、手機無回饋。啟用路徑皆人類：human.rs:501-503、routes.rs:105（PATCH /v1/receptors/{id}）、src-tauri lib.rs:305-313。手機 mic 啟用不進 runtime.rs:100-101/441 activeSensors→tray 不顯示（違「感測不靜默」）。severity medium
- [5 send_to_any] mobile.rs:113-122 取 BTreeMap iter().next()＝device_id 字典序最小那台（隨機 hex，固定但任意）；act()(135) 與 MobileActuator::execute(195-223) 無 device 目標參數、extra 中的 deviceId 不解讀；mobile_ble_scan(874-877) 同。多機時全部動作送同一台。severity medium
- [6 重連高風險] 桌面斷線收尾 847-859 只清 conn＋provider Disconnected，不動 registry enabled 旗標；auth 重連 784-811→608-637 直接 Available。不自動恢復全靠手機（InteractionCompanionApp.swift:74-77 disableHighRiskSensors、ConnectionManager.swift:277-279）。桌面端無強制。severity medium
- [7 健康度] 443-458 閉包 try_read conns；斷線 849 write().remove 後→offline，正確。但 mobile.rs 無 Ping/Pong/heartbeat/read timeout（grep 0 命中）→半開 TCP 時 conns 不清、receptor/actuator 續報 healthy、act 4s ACT_TIMEOUT(41) 後 UNKNOWN(148-153,243-246)。severity medium
- [8 estop] MobileActuator::emergency_stop 267-272 → broadcast 124-132：send_timeout 300ms、錯誤 `let _` 丟棄、不等 ack、永遠 Ok→runtime.rs:822-830 stoppedActuators 計入即使無手機或訊息被丟；6 個 actuator 各 broadcast 一次→每機 6 則 stop-all；pending_acts 在 estop 不清（runtime.rs:801-863 無觸及）→在途 act 等滿 4s 才 UNKNOWN。測試 mobile_loop.rs:249-252 只斷言看到 stop-all。severity low
- [9 token 隔離] api lib.rs:372-405 agent_request_allowed：GET 排除 /v1/mobile(384-385)，POST/DELETE 不在 allowlist→403；session token session_request_allowed 347-367 僅 tools/estop/interrupt→403；/v1/hardware/scan(402) 不呼叫 mobile（hardware.rs grep 0）。無 API 測試覆蓋 /v1/mobile 拒絕（api_e2e.rs:180-210 清單無 mobile）。附帶：GET /v1/providers 對 agent 開放，provider.fingerprint=token_hash（mobile.rs:617）→agent 可讀 SHA-256(token)（不可逆，low）
- [10 檔案權限/明文] mobile-key.der 先 std::fs::write(339) 再 chmod 0600(343)（短暫 0644）；mobile-cert.der 預設權限（公開資料，可）；mobile-devices.json std::fs::write(294-302) 無 set_permissions→umask 預設 0644，內容為 token_hash+名稱（非明文）。token 明文只在 `paired` WSS 訊息(780)；audit 只記 deviceId(775-777)、port(545-546)；配對碼回傳給人類 HTTP/Tauri(547-554) 與 CLI stdout(commands.rs:813-815) 屬預期；無 tracing 記錄 token。severity low
- [11 autostart] runtime.rs:369 無條件呼叫 mobile_autostart_if_paired（不看 spawn_watchdog/測試模式）；mobile.rs:351-361 只以 <home>/state/mobile-devices.json 存在與否決定；bind 0.0.0.0:18790(384)。測試用 tempdir home(mobile_loop.rs:21-32)不會開埠；但 home=None 的 runtime（含任何用真實 home 的測試）若曾配對即開 LAN 埠；mobile_pairing_begin 亦在測試中真開 0.0.0.0 埠。severity low
- [12 佇列滿] OUTBOUND_QUEUE=32(42)：send_to_any 500ms 逾時→Err→act() 143-144 清 pending 回 Err→受據 failed iphone-unreachable(247-249) 誠實；broadcast 300ms 逾時靜默丟(127-131)→stop-all 可能遺失且 estop 仍報 Ok；handler send 閉包 661-668 500ms 靜默丟→paired(含 token)/auth-ok 理論上可遺失。severity low
- [13 測試映射] 見 §15.4 各列；額外：mobile_loop.rs:186-198 觀察送出後零斷言、293-294 斷線後零斷言，與 mobile_loop.rs:4-6 檔頭及 CHANGELOG.md:75-77 宣稱不符
- [facts 白名單] mobile.rs:814-821 只驗 receptor id，facts 原樣進 ingest 並落 DB（runtime.rs:571-573；iphone.motion 未宣告 retention None 490-496）→「不保存原始軌跡」桌面端無強制
- [Bonjour 身分] mobile.rs:400-404 服務實例名 'interact-ai'、主機名 'interact-ai.local.' 固定；paired/auth-ok(780,810) 不含電腦名→手機無法「清楚顯示連接的電腦」，兩台桌面在同 LAN 互相衝突
- [BLE 桌面缺口] 只有 ble.scan 送端(866-884)；iOS BleGateway.swift:213-336 已實作 ble.connect/ble.gatt(read/write/subscribe) 但桌面無對應送端與 API；scan 無 CLI（main.rs:435-443）、無 Tauri（lib.rs:2122-2124）、無 UI
- [docs 矛盾] docs/v05-capability-gap-matrix.md:83「全部未開始」vs :113「已有（模擬 iPhone 驗收）」vs CHANGELOG.md:48-77

### Phase 6 iOS App 端＋協定一致性（48 列）

| Phase | 需求 | 程式存在 | 已接線 | 測試通過 | 真環境驗證 | 缺口 | 嚴重度 |
|---|---|---|---|---|---|---|---|
| 6 | §10.1 Swift／SwiftUI 原生 App（可共用 Rust schema，但不犧牲 iOS 權限與生命週期正確性） | 是 | 部分 | 部分 | 僅模擬器 | 沒有專案檔與 CI，App 無法一鍵編譯；只有純邏輯（協定編解碼、動作分類器）有測試；UI/感測/動器/BLE 全未驗收。 | 🟠 high |
| 6 | §10.1 Bonjour 自動發現 | 部分 | 否 | 無 | 未驗證 | App 端完全沒有 Bonjour 瀏覽實作（NWBrowser），規格要求的自動發現只做了伺服器端廣播；README 也未列此為限制。 | 🟠 high |
| 6 | §10.1 QR Code 或配對碼 | 是 | 是 | 部分 | 僅模擬器 | QR 掃描（VisionKit）未在真機驗證；payload 解析與配對碼路徑已驗。 | 🟢 low |
| 6 | §10.1 每台 iPhone 獨立金鑰與 challenge-response | 是 | 是 | 通過 | 僅模擬器 | device token 永不過期／不輪替（mobile.rs:741-757 無 TTL）；「過期金鑰」只涵蓋 5 分鐘配對碼。 | 🟢 low |
| 6 | §10.1 TLS WebSocket（含憑證指紋釘選正確性） | 是 | 是 | 部分 | 僅模擬器 | 指紋不符（錯誤電腦／MITM）未測；:69-74 非 ServerTrust challenge 直接 cancel（無害）。 | 🟢 low |
| 6 | §10.1 iPhone 清楚顯示連接的電腦、能力、活動中感測器與立即中斷 | 部分 | 部分 | 無 | 未驗證 | 「能力」與「動器活動」未呈現（actionLog 死資料）；手電筒開啟中無持續指示。 | 🟡 medium |
| 6 | §10.1 斷線後能力自動 unavailable；重連不得自動恢復高風險能力 | 是 | 是 | 部分 | 僅模擬器 | 斷線→Disconnected 未被斷言；App 重連後不恢復高風險感測未被測試。 | 🟡 medium |
| 6 | §10.1 桌面 Consent 不能取代 iOS 系統權限 | 是 | 部分 | 部分 | 未驗證 | 桌面未顯示 iOS 權限狀態，與 mobile.rs:14 宣稱不符。 | 🟢 low |
| 6 | §10.1 架構：iPhone receptors/actuators＋BLE 裝置 → Desktop Runtime → 小樞／AI Agent | 是 | 是 | 通過 | 僅模擬器 | 見 cross 列：act 參數映射與 App 驗證不一致，6 動器中 5 個以預設參數會被 App 以 bad-params 拒絕。 | 🟠 high |
| 6 | §10.2 Touch／gesture receptor | 是 | 是 | 無 | 未驗證 | 僅 tap/longpress，無 swipe/drag/pinch 等 gesture。 | 🟢 low |
| 6 | §10.2 Accelerometer receptor（預設關／unavailable 誠實） | 部分 | 是 | 通過 | 僅模擬器 | 以語意事件 iphone.motion 涵蓋，無獨立 accelerometer 觀察；CMMotionManager 真機行為未驗。 | 🟢 low |
| 6 | §10.2 Gyroscope／device motion／orientation receptor | 部分 | 是 | 通過 | 僅模擬器 | 無裝置方向（orientation）事件；只有 rotated 語意。 | 🟢 low |
| 6 | §10.2 Battery／charging／foreground state | 是 | 是 | 通過 | 僅模擬器 | foreground 只在電池開啟時隨 battery facts 送出；status 訊息無 foreground 欄位。 | 🟢 low |
| 6 | §10.2 Microphone level／audio（需權限；預設關；denied 誠實） | 是 | 是 | 無 | 未驗證 | 未在真機驗證權限流程與音量計算；「audio」原始音訊刻意不提供（README:97）。 | 🟡 medium |
| 6 | §10.2 Camera／QR／capture（需權限） | 部分 | 否 | 無 | 未驗證 | 規格列為 receptor，但只做配對掃描；README 已知限制未列「無相機 receptor」。 | 🟡 medium |
| 6 | §10.2 Location／geofence（需權限） | 部分 | 部分 | 無 | 未驗證 | 位置／geofence 觀察完全未實作，僅權限回報；App 端誠實標示。 | 🟡 medium |
| 6 | §10.2 BLE device discovery／state | 部分 | 部分 | 部分 | 未驗證 | BLE 狀態不作為 receptor 事件推送；桌面除 HTTP 外無入口。 | 🟡 medium |
| 6 | §10.2 Local-network device events | 否 | 否 | 無 | n/a | 完全未實作，README 已知限制亦未提及。 | 🟠 high |
| 6 | §10.2 不可用的感測器依機型／系統 API 誠實標示 unavailable | 是 | 是 | 無 | 未驗證 | 無測試；未在真機驗證各機型差異。 | 🟢 low |
| 6 | §10.3 Character presentation actuator | 是 | 部分 | 無 | 未驗證 | 桌面端無人驅動角色狀態到 iPhone；預設參數必失敗。 | 🟡 medium |
| 6 | §10.3 Custom haptic actuator | 是 | 部分 | 部分 | 未驗證 | 桌面 pipeline 預設參數在真 App 會被拒；magnitude→intensity 無映射。 | 🟠 high |
| 6 | §10.3 Notification actuator | 是 | 部分 | 無 | 未驗證 | 欄位名不一致（text vs title/body），預設路徑必失敗。 | 🟠 high |
| 6 | §10.3 Audio／SFX actuator | 否 | 否 | 無 | n/a | 短音效／SFX 未實作，README 未列限制。 | 🟡 medium |
| 6 | §10.3 TTS actuator | 是 | 是 | 無 | 未驗證 | 未真機驗證。 | 🟢 low |
| 6 | §10.3 Screen color／flash effect | 是 | 部分 | 無 | 未驗證 | 欄位不一致；前景判斷依賴 SensorCenter.isForeground。 | 🟡 medium |
| 6 | §10.3 Torch（需明確用途與限制） | 是 | 部分 | 無 | 未驗證 | 欄位不一致；torchOn 狀態未在任何 View 顯示（持續指示缺）。 | 🟡 medium |
| 6 | §10.3 Live Activity／鎖定畫面狀態（平台允許時） | 否 | 否 | 無 | n/a | 未實作、未文件化。 | 🟢 low |
| 6 | §10.4 第一優先：BLE GATT 掃描、連線、Service／Characteristic 探索、read、write、subscribe | 是 | 部分 | 無 | 未驗證 | 閘道只有 scan 閉環到桌面；connect/read/write/subscribe 在桌面端無入口；訂閱串流語意與 Rust one-shot 不相容。 | 🟠 high |
| 6 | §10.4 第二優先：Bonjour、HTTP、WebSocket、MQTT 區域網路裝置 | 否 | 否 | 無 | n/a | 未實作、未文件化。 | 🟡 medium |
| 6 | §10.4 External Accessory 僅用於明確支援的 MFi／廠商配件 | 否 | n/a | 無 | n/a | 誠實未做；符合「不宣稱」要求。 | 無 |
| 6 | §10.4 不得宣稱 iPhone 可任意操作所有 USB／Lightning／USB-C 裝置 | 是 | n/a | 無 | n/a | none | 無 |
| 6 | §10.4 ESP32 的 iPhone 連線優先 BLE 或 Wi-Fi，不以通用 USB Serial 為第一版 | 部分 | 否 | 無 | 未驗證 | ESP32 經 iPhone 的閉環不存在；方向（BLE 優先）與規格相符。 | 🟢 low |
| 6 | §10.5 小樞可從桌面「前往 iPhone」，必須對應真實 connected presentation surface | 否 | 否 | 無 | n/a | 未實作。 | 🟡 medium |
| 6 | §10.5 iPhone 被拿起時可（授權下）觸發桌面小樞注意 | 部分 | 部分 | 部分 | 僅模擬器 | 缺桌面端「注意」反應與授權開關。 | 🟡 medium |
| 6 | §10.5 桌面任務進行時，iPhone 可顯示簡化角色狀態與必要確認 | 部分 | 否 | 無 | 未驗證 | 僅有被動狀態顯示；「必要確認」完全未實作。 | 🟡 medium |
| 6 | §10.5 iPhone haptic 可表現輕敲、呼嚕、心跳或提醒，可分別關閉並限制頻率 | 部分 | 部分 | 無 | 未驗證 | 「可分別關閉」未實作；頻率限制未測。 | 🟡 medium |
| 6 | §10.5／§14 不保存原始 motion 軌跡；優先輸出 lifted／shaken／placed／rotated 語意事件 | 是 | 是 | 通過 | 僅模擬器 | Rust 白名單丟棄未被斷言。 | 🟢 low |
| 6 | §15.4 配對、撤銷、錯誤電腦、過期金鑰 | 是 | 是 | 部分 | 僅模擬器 | 錯誤電腦與過期金鑰兩端皆無測試。 | 🟡 medium |
| 6 | §15.4 Bonjour discovery、TLS WebSocket reconnect | 部分 | 是 | 部分 | 僅模擬器 | Bonjour 未測且 App 未實作；App 重連未測。 | 🟡 medium |
| 6 | §15.4 前景／背景／被系統終止後誠實狀態 | 部分 | 部分 | 無 | 未驗證 | 背景進入時桌面無即時誠實狀態；無測試；未在模擬器／真機驗證背景→回前景重連。 | 🟡 medium |
| 6 | §15.4 Motion 語意事件 | 是 | 是 | 通過 | 僅模擬器 | 純分類器驗證；CoreMotion 真機門檻調校未驗。 | 🟢 low |
| 6 | §15.4 Haptic frequency limit | 是 | 是 | 無 | 未驗證 | 無測試。 | 🟡 medium |
| 6 | §15.4 Camera／microphone／location permission denied／revoked | 是 | 是 | 無 | 未驗證 | 全部無測試、未真機驗證。 | 🟡 medium |
| 6 | §15.4 BLE gateway | 是 | 部分 | 無 | 未驗證 | 零測試、零真機。 | 🟠 high |
| 6 | §15.4 iPhone 斷線後 capability unavailable | 是 | 是 | 無 | 僅模擬器 | 宣稱有測、實際無斷言。 | 🟡 medium |
| 6 | §15.4 iOS 真機驗收；Simulator 不冒充 sensor／BLE 真機證據 | 否 | n/a | 部分 | 未驗證 | 真機驗收完全缺席；Phase 6「真機驗收」與 §17 完成定義未達；環境不是原因。 | 🔴 blocker |
| cross | Wire protocol App(Protocol.swift) ↔ Rust(mobile.rs) 逐欄比對 | 部分 | 部分 | 部分 | 僅模擬器 | 訊息型別／欄位名層級一致，但 act 參數語意層級不一致，真 App 接上桌面 pipeline 時多數動器會失敗。 | 🟠 high |
| cross | Swift 編譯／typecheck 與警告（iOS 26.5 sim SDK，-target arm64-apple-ios17.0-simulator） | 是 | n/a | 通過 | 僅模擬器 | README.md:11-18 宣稱僅 swiftc -parse，現已可完整 typecheck 但文件未更新。 | 🟢 low |

**診斷（TODO／placeholder／mock 冒充／hard-coded success／空函式／跳過測試／警告）：**

- [文件錯誤] apps/interaction-ios/README.md:8「本開發機沒有 Xcode」、CHANGELOG.md:73「本環境無 Xcode/iOS SDK」、docs/v05-capability-gap-matrix.md:113「無 Xcode 無法編譯」——本機有 Xcode 26.6／iOS 26.5 SDK／模擬器，宣稱不實；上一 session 未編譯是選擇而非環境限制。
- [編譯警告—初版 15:08] ActuatorCenter.swift:446 `engine.stop(completionHandler: nil)` warning: consider using asynchronous alternative；ConnectionManager.swift:387、:388、:412 與 SensorCenter.swift:248 warning: reference to captured var 'self' in concurrently-executing code（Swift 6 language mode 為 error）。→ 15:09-15:10 被另一並行 session 修正（改為 Task { @MainActor [weak self] } 與 try? await engine.stop()），現 App 12 檔 0 warning。
- [編譯警告—殘留] MotionClassifierTests.swift:13、ProtocolTests.swift:12 `@testable import InteractionCompanion` ignoring import——僅在把測試檔與 App 同模組編譯時出現（我的 typecheck 方式），非真實問題。
- [測試宣稱不符] crates/interaction-runtime/tests/mobile_loop.rs:6 標頭宣稱覆蓋「斷線 → provider Disconnected」，檔內無任何 Disconnected／offline 斷言（grep 僅命中標頭）。
- [測試不斷言] mobile_loop.rs:193-199 送出白名單外 receptor `iphone.raw-trajectory` 但未斷言其被丟棄。
- [假 iPhone 遮蔽缺陷] mobile_loop.rs:219-223 模擬 iPhone 以硬編碼 `{"style":"medium","count":1}` 回 ack，不經 App 的參數驗證；因此 Rust execute（mobile.rs:200-212）只送 magnitude/durationMs/text、而 App 要求 style/title+body/color/on/state 的協定不一致從未被測到。
- [未接線] mobile.rs:865 註解「connect/gatt 走同協定」但 Rust 無任何送出 ble.connect／ble.gatt 的程式；`POST /v1/mobile/ble/scan` 未接到 CLI（MobileAction 只有 Status/Pair/Revoke，main.rs:435-442）、Tauri、UI。
- [語意不相容] mobile.rs:837-843 `ble.value` 以 pending_acts one-shot 處理；App BleGateway.swift:548-553 訂閱通知沿用 subscribe id 連續送出 → 第二筆起被靜默丟棄。
- [死資料] ActuatorCenter.swift:69 actionLog 與 :67 torchOn 未被任何 View 讀取（ContentView 只用 actuators.flash）→ 動器活動與手電筒開啟中對使用者不可見（違反「持續可見指示」精神）。
- [空函式] PairingView.swift:302-304 `Coordinator.reportStartFailure()` 為空實作；註解說「誠實」但掃描啟動失敗不會顯示任何訊息。
- [死程式] ConnectionManager.swift:289 `_ = payload` 無作用陳述。
- [未實作且未文件化] Live Activity（無 ActivityKit）、Audio/SFX 動器、Camera/capture receptor、Local-network device events、Bonjour App 端瀏覽（無 NWBrowser）——README 已知限制（:147-166）皆未列出。
- [缺專案檔] apps/interaction-ios 無 .xcodeproj／Package.swift／CI；App 從未被打包或在模擬器啟動；「SwiftUI App」交付停留在原始碼。
- [並行修改] 本次審查期間（15:09-15:10）另一 session 修改了 ConnectionManager.swift、SensorCenter.swift、ActuatorCenter.swift、PairingView.swift（新增 DEBUG-only `--pairing-payload`／`INTERACT_PAIRING_PAYLOAD` 自動配對入口 PairingView.swift:25-28,42-46,166-193）；該入口 DEBUG 限定並重用 applyPayloadText，但無測試、README 未提及。
- [金鑰無過期] mobile.rs:741-757 device token 無 TTL／輪替；PairingStore 亦無過期概念；§15.4「過期金鑰」只涵蓋 5 分鐘配對碼。
- [多裝置] mobile.rs:113-122 send_to_any 把 act 送給 BTreeMap 第一台連線 iPhone，無 per-device 目標。
- [桌面顯示] CapabilitiesHub.tsx:290-297 只顯示手機 status.sensors，未顯示 status.permissions，與 mobile.rs:14「手機回報的 permissions 誠實顯示」不符。
- [無測試區] haptic 頻率限制、權限 denied/revoked、重連 backoff、指紋不符、BLE gateway（App 與 Rust）、前景/背景狀態——全部零測試。
- [production expect] mobile.rs:716 `.expect("hmac accepts any key length")`（HMAC 接受任意長度，實際安全）。

### §2 安全底線／§14 效能誠實／§11 記憶分層／CLAUDE.md 不變量（38 列）

| Phase | 需求 | 程式存在 | 已接線 | 測試通過 | 真環境驗證 | 缺口 | 嚴重度 |
|---|---|---|---|---|---|---|---|
| cross | §2-1 Emergency Stop（保留並回歸驗證） | 是 | 是 | 通過 | 僅模擬器 | 沒有任何測試斷言 estop_engaged 跨重啟保留（runtime.rs:197/314 有程式碼，tests 只在同一程序內 assert is_estopped；human_layer.rs:123 只測 pause 跨重啟）。iPhone/serial 端 stop-all 只在模擬器驗證。 | 🟢 low |
| cross | §2-2 麥克風、攝影機使用中的持續可見指示與立即停止 | 部分 | 部分 | 部分 | 真環境已驗 | iPhone `iphone.mic-level`（Sensitivity::Personal, consent=true, mobile.rs:481-487）串流進來時 (mobile.rs:812-822 → runtime.ingest) 完全不經 sensor_state_changed：status.activeSensors、tray、SensorStarted 事件都不會反映，只有 CapabilitiesHub.tsx:292-298 一行「手機自報感測」文字。且該 receptor manife | 🟡 medium |
| cross | §2-3 Human Token、Agent Token、Session Scope 分離 | 是 | 是 | 通過 | 真環境已驗 | lib.rs:384-385 新增的 `/v1/mobile` agent-token 拒讀無測試覆蓋（api_e2e:178-206 清單未含 mobile 路徑）。 | 🟢 low |
| cross | §2-4 Agent 不得自行授權、解除 Emergency Stop 或擴大資料範圍 | 是 | 是 | 通過 | 真環境已驗 | none | 無 |
| cross | §2-5 指定工作區寫入、外部資料傳送與實體效果的明確授權 | 是 | 是 | 通過 | 僅模擬器 | none（實體效果只在模擬器/mock 驗證） | 無 |
| cross | §2-6 Agent claimed-completed 不等於 verified | 是 | 是 | 通過 | 真環境已驗 | none | 無 |
| cross | §2-7 外部動作結果未知不得顯示成功，也不得自動重試可能造成重複副作用的動作 | 是 | 是 | 通過 | 僅模擬器 | (a) MQTT 用 QoS1 at-least-once（mqtt.rs:113）→ 傳輸層可能重送，去重完全依賴裝置端 cmdIdSeen（firmware/esp32-companion.ino:222-234），只與 Python 模擬器對測。(b) mobile 路徑無 ack-timeout→unknown 的測試（mobile_loop.rs 只測 ack 成功）。(c) link_caps.rs:252-254 與 lib.rs:788-790 `status()` 硬編碼 healthy，裝置拔線 | 🟢 low |
| cross | §2-8 硬體強度、時間、頻率與韌體硬限制 | 是 | 是 | 部分 | 僅模擬器 | 韌體硬限制只經程式碼審閱＋pty/MQTT 模擬器（CHANGELOG:111-112 自承無真 ESP32）；BLE 無真機。 | 🟢 low |
| cross | §2-9 Secret 使用 Keychain／Credential Reference，不寫入 YAML 或 log | 部分 | 是 | 部分 | 真環境已驗 | 桌面端無 Keychain 整合；YAML 明文 pairingCode/password 不會被拒；secrets.json 是明文 JSON 檔（僅靠檔案權限）。 | 🟡 medium |
| cross | §2-10 Session 可取消、到期、撤銷；子程序樹可終止 | 是 | 是 | 通過 | 真環境已驗 | none | 無 |
| cross | §2-11 安全 UI 改成風險分級 L0–L4（L0 預設開啟不逐次詢問、L1 一次設定、L2 首次詢問、L3 明確授權+硬限制、L4 每次/短效授權+持續指示），一般使用者不被 Provider lifecycle/Lease/Token/Candidate/Receipt 淹沒 | 部分 | 部分 | 部分 | 真環境已驗 | 沒有任何 L0–L4 等級標示或依等級的預設處理 UI；L4「每次授權」沒有 per-use 選項（最短 30 分）；SafetyPage 標題仍用「使用授權（Consent）」；MemoryKnowledgePage 仍暴露 Candidate/Receipt/Context Bundle（見 §11）。 | 🟡 medium |
| cross | §11-1 分開三種資料：角色互動記憶（最喜歡的玩具、偏好距離、常關掉的反應、近期玩耍、熟悉度）／工作與個人記憶／正式知識；角色互動記憶不得一次行為推論人格、不得自動升級為正式知識 | 部分 | 否 | 無 | n/a | 第 1 類「角色互動記憶」完全未實作為資料類別（無 store、無 API、無 UI）；「不得推論人格／不得自動升級」規則因此無對應程式碼與測試。gap matrix §6 標「缺」、§9 收尾狀態完全未提，證實整段未做。 | 🟠 high |
| cross | §11-2 一般 UI 只顯示三項：關於我的記憶、小樞學會的知識、素材與來源 | 否 | 是 | 無 | 真環境已驗 | 一般 UI 仍顯示五個 tab（含知識收據與 Context Bundle 預覽）；標籤名稱與規格三項不符；沒有依 advanced 收斂。 | 🟠 high |
| cross | §11-3 Candidate/Active/Stale/Disputed/Superseded/Knowledge Receipt/Context Bundle 移至技術詳情／進階模式，並用人類文案（等待確認、已採用、可能過期、有不同說法、已被新版取代） | 部分 | 是 | 無 | 真環境已驗 | 技術詞（候選、知識收據、Context Bundle）仍在一般模式；人類文案未依規格（等待確認/已採用/可能過期/有不同說法）；只有 KnowledgeAdvanced 是進階專屬，其他未移動。 | 🟠 high |
| cross | §14-1 本機游標／玩具反應目標 16～100ms | 是 | 是 | 無 | 未驗證 | 無任何量測（時間戳/perf 測試/腳本）證明 16–100ms；結構上是同步路徑但未實測。 | 🟢 low |
| cross | §14-2 一般動畫不得因 HTTP、SQLite、Agent 或 AI 阻塞 | 是 | 是 | 部分 | 未驗證 | 無阻塞量測；架構上分離但未實測。 | 🟢 low |
| cross | §14-3 Interaction Director 與 renderer 使用 bounded queues | 部分 | 是 | 部分 | 真環境已驗 | Director/renderer 沒有「queue」這個結構（tick-based 單動作設計），因此無界問題不存在，但規格字面上的 bounded queue 未以 queue 形式實作；Runtime 側 presentation pending 有界。 | 🟢 low |
| cross | §14-4 動畫與事件在 60fps 目標下量測；低效能裝置允許 30fps 降級 | 部分 | 是 | 無 | 未驗證 | 沒有任何 fps 量測程式碼/測試/腳本；gap matrix 的效能數字不可重現；無顯式 30fps 降級模式（只靠 rAF 自然掉幀＋dt clamp）。 | 🟢 low |
| cross | §14-5 Reduced Motion 下保留狀態辨識但減少位移、彈跳與粒子 | 是 | 是 | 通過 | 真環境已驗 | none | 無 |
| cross | §14-6 原始游標軌跡不持久化、不傳 AI | 是 | 是 | 通過 | 真環境已驗 | none | 無 |
| cross | §14-7 原始 iPhone motion 軌跡預設不持久化，轉為語意事件 | 是 | 是 | 部分 | 僅模擬器 | 白名單丟棄行為無斷言；iOS 端分類器測試未執行；手機真機未驗。 | 🟢 low |
| cross | §14-8 麥克風、攝影機與定位不得靜默啟用 | 部分 | 部分 | 部分 | 僅模擬器 | 桌面端對 `iphone.mic-level` 只有 enabled 旗標：人類 PATCH 啟用後即持續 ingest＋持久化，無 consent 紀錄、無 activeSensors/tray 指示（與桌面 mic 的 begin_mic_listen 三重門檻不對等）；mobile.rs:812-822 ingest 亦不看 consent。定位：桌面白名單無 iphone.location（iOS 端有 locationEnabled 但無對應 receptor）。 | 🟡 medium |
| cross | §14-9 所有 unavailable、unsupported、claimed、acknowledged、unknown 狀態使用誠實文字 | 是 | 是 | 通過 | 真環境已驗 | none | 無 |
| cross | §14-10 不可為了畫面漂亮，把 Unknown、Blocked、Emergency 演成成功或賣萌慶祝 | 是 | 是 | 通過 | 真環境已驗 | none | 無 |
| cross | CLAUDE.md 嚴禁 MCP | 是 | n/a | 通過 | 真環境已驗 | none | 無 |
| cross | CLAUDE.md Policy Governor 確定性 min(AI 請求, 使用者偏好, session 限制, 裝置安全上限, 剩餘預算) | 是 | 是 | 通過 | 真環境已驗 | none | 無 |
| cross | CLAUDE.md 誠實階梯 queued≠completed、acknowledged≠completed、completed≠verified、inference≠fact、未知標 uncertain | 是 | 是 | 通過 | 真環境已驗 | none | 無 |
| cross | CLAUDE.md AI 不可授予 consent、不可解除 emergency stop、不可提高後端安全上限 | 是 | 是 | 通過 | 真環境已驗 | none | 無 |
| cross | CLAUDE.md 實體／外部副作用動器與敏感受器預設關閉 | 是 | 是 | 通過 | 真環境已驗 | none | 無 |
| cross | CLAUDE.md emergency stop 與高風險能力在重啟後不得自動恢復 | 是 | 是 | 部分 | 真環境已驗 | estop_engaged 跨重啟無自動測試；mobile 伺服器（0.0.0.0:18790 TLS listener, mobile.rs:384）在曾配對後會於重啟自動開啟——非高風險感測，但屬對外網路服務自動恢復，需明文列為設計決定。 | 🟢 low |
| cross | CLAUDE.md 模擬／dry-run 不得產生外部副作用；不用假資料冒充真實 agent／裝置／執行結果 | 是 | 是 | 通過 | 真環境已驗 | none | 無 |
| cross | CLAUDE.md 長時工作必須有 TTL／lease／watchdog／cancel | 是 | 是 | 通過 | 真環境已驗 | mobile.rs:866-884 mobile_ble_scan 逾時未從 pending_acts 移除（:869 insert、:880 逾時分支無 remove）→ 每次逾時殘留一筆；ble.rs:157-167 每次 connect 起的 notification task 無取消（重連累積）；mqtt.rs:59-90 eventloop task 無 shutdown 旗標（adapter 移除後仍存活）。 | 🟢 low |
| cross | CLAUDE.md 禁止無界 queue 與 blocking sleep；production code 不濫用 unwrap() | 是 | n/a | 未知 | 真環境已驗 | none（mobile.rs:716 expect 屬風格瑕疵） | 無 |
| cross | CLAUDE.md CLI／HTTP API／Tauri 共用同一 application service；核心邏輯不進前端 JS；WebView 不直接控制裝置 | 是 | 是 | 通過 | 真環境已驗 | none | 無 |
| cross | CLAUDE.md 感測不靜默：啟用中的感測器必須同時反映在 status、事件、tray 與 UI | 部分 | 部分 | 通過 | 真環境已驗 | iPhone `iphone.mic-level` 持續串流不進 active_sensors/事件/tray（sensors.rs grep iphone/mobile 0 hits；mobile.rs:812-822 直接 ingest），只在 CapabilitiesHub.tsx:292-298 顯示手機自報文字。 | 🟡 medium |
| 6 | 特別核對：mobile.rs 是否違反不變量（unwrap／無界／blocking sleep／自動恢復高風險／假成功） | 是 | 是 | 通過 | 僅模擬器 | (1) mobile.rs:823-829 `ack`/`err` 與 :837-843 `ble.result`/`ble.value` 分支沒有 `authed.is_some()` 守門：任何完成 TLS 握手但未配對/未 auth 的 peer 若猜中 action id 可偽造 ack→acknowledged 收據；(2) :384 綁 0.0.0.0（LAN 必要，但非 loopback 預設應在文件明列）；(3) :113-122 send_to_any 多台 iPhone 時任選第一台，無目標裝置 | 🟡 medium |
| 5 | 特別核對：serial.rs／mqtt.rs／ble.rs 是否違反不變量 | 是 | 是 | 通過 | 僅模擬器 | serial.rs:42-43 ENOTTY 判斷把任何 io::ErrorKind::Other 視為 pty 而退回檔案 I/O（真硬體開埠錯誤可能被誤判）；ble.rs 只 macOS/Windows 編譯且無真機；mqtt.rs task 無 shutdown；link_caps.rs:252-254 status() 硬編碼 healthy。 | 🟢 low |
| 3 | 特別核對：stage.ts／playfield.ts／director.ts 是否違反不變量 | 是 | 是 | 通過 | 真環境已驗 | none | 無 |

**診斷（TODO／placeholder／mock 冒充／hard-coded success／空函式／跳過測試／警告）：**

- §11 整段未做：apps/interaction-desktop/src/pages/MemoryKnowledgePage.tsx:9,36-61 一般 UI 仍有 5 個 tab（記憶／知識與候選／原始素材／知識收據／提供給 AI 的內容），元件無 advanced prop（:32）、MorePage.tsx:49 只傳 refreshKey；「角色互動記憶」資料類別 grep 全 repo 0 hits；docs/v05-capability-gap-matrix.md:§6 標「缺」而 §9 收尾狀態完全未提。
- Hard-coded success：crates/interaction-adapter-declarative/src/link_caps.rs:252-254 與 lib.rs:788-790 `status()` 永遠回 ComponentHealth::healthy()，不看 link connected/裝置拔線。
- 安全漏洞候選：crates/interaction-runtime/src/mobile.rs:823-829 (`ack`/`err`) 與 :837-843 (`ble.result`/`ble.value`) 未檢查 authed，未配對 peer 可對 pending_acts 偽造 ack。
- 資源殘留：mobile.rs:866-884 mobile_ble_scan 逾時未移除 pending_acts；ble.rs:157-167 notification task 重連不取消；mqtt.rs:59-90 eventloop task 無 shutdown 旗標。
- mobile.rs:716 `.expect("hmac accepts any key length")` 在 production path（不可失敗但違反不用 unwrap/expect 風格）。
- 感測不靜默缺口：`iphone.mic-level`（Personal, consent）串流不進 sensors.rs active_sensors/tray/SensorStarted；routes.rs:97-108 receptor_patch 啟用 consent-required receptor 不需 consent；manifest 未宣告 retention=none → 手機音量值被持久化（runtime.rs:573-575）。
- Secret：adapter-declarative lib.rs:478-508 resolve_secret 只支援 env var 與 state/secrets.json（明文 JSON, 0600），非 Keychain/Credential Reference；lib.rs:183 pairingCode 明文可用（僅「建議」secret://）。
- 未驗證效能宣稱：docs/v05-capability-gap-matrix.md:115「drawRig 0.452 ms/幀」在 scripts/、e2e/、src/test/ 找不到任何量測程式或測試（grep 0.452/performance.now 0 hits）；§14 60fps/30fps 降級、16–100ms 反應無量測。
- 測試缺口：無 estop_engaged 跨重啟測試；mobile_loop.rs:186-197 送 raw-trajectory 但無斷言被丟棄；mobile 路徑無 act 逾時→unknown 測試；無 agent token 拒 /v1/mobile 測試；無 §11 UI 分層測試；無 L0–L4 分級測試。
- iOS 全部原始碼（apps/interaction-ios）無法在本機驗證：xcrun 找不到 iphonesimulator SDK（僅 CommandLineTools），MotionClassifierTests/ProtocolTests 未執行；iPhone 真機未驗。
- serial.rs:42-43 ENOTTY 判斷過寬：任何 io::ErrorKind::Other 都退回純檔案 I/O，可能掩蓋真硬體開埠錯誤。
- SafetyPage.tsx 無 L0–L4 分級標籤；GrantDialog:286-290 最短授權 30 分鐘，L4「每次授權」無 per-use 選項；標題仍用技術詞「使用授權（Consent）」。
- mobile.rs:384 mobile 伺服器綁 0.0.0.0（非 loopback），且 mobile.rs:351-361 曾配對即於重啟自動開啟 listener——需在文件明列為設計決定。
- 親跑結果：cargo test -p interaction-runtime --test mobile_loop → 3 passed (2.76s)；--test hardware_loop → 1 passed (8.18s)；pnpm test (vitest) → 14 files / 138 tests passed；cargo build -p interaction-runtime -p interaction-adapter-declarative -p interaction-api → 0 warnings。未跑 e2e/playwright/CLI e2e（依指示）。
- 無 TODO/FIXME/todo!/unimplemented!/skipped tests 於 mobile.rs、hardware.rs、adapter-declarative、companion/、interaction-ios（grep 0 hits）。
- §14 規格實際為 10 條（非 11 條）；已逐條列出。
