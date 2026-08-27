# 端到端驗收證據（真 daemon＋真 CLI）— 2026-08-26 11:33

## 情境 A：單受器 → conversation → receipt completed
planId: plan-46710062-2cad-4e5e-850f-55b76fa059b4
simulate.wouldExecute: True
execute → status=completed verdict=acknowledged-only
timestamps: ['authorized', 'accepted', 'dispatched', 'acknowledged', 'completed']
outbox: 完成了，所有檢查都已通過。

## 情境 B：多受器＋自適應多動器（最小有效組合）
steps chosen: [('web-ui', 0.6), ('conversation', 0.5)]
rejected: [('local-log', 'maxChannels 2 reached'), ('local-notification', 'maxChannels 2 reached')]
receipts: [('web-ui', 'completed'), ('conversation', 'completed')]

## 情境 C：安靜時段降級（audio 被政策擋下、文字仍可）
receipts: [('local-notification', 'blocked'), ('conversation', 'completed')]
notification 決策: [{'outcome': 'blocked', 'reason': 'channel notification is silenced during quiet hours 00:00-23:59', 'rule': 'quiet-hours'}]

## 情境 G：Mock 實體裝置完整狀態機（observed 驗證）
狀態機路徑: ['authorized', 'accepted', 'dispatched', 'acknowledged', 'observed', 'completed']
magnitude 0.9 → effective: 0.8（裝置安全上限 0.8）
verdict: observed

## 情境 E：撤回同意 → 後續 haptic 被擋
revoke 後執行: [('blocked', {'outcome': 'blocked', 'reason': 'actuator mock.actuator requires session consent', 'rule': 'consent.required'})]

## 情境 F：工具閉環（讀→規劃→執行→重讀→驗證，全走 tools）
tools.capabilities: actuators=5
tool loop: plan=plan-2e1634c1-c245-4458-be1d-0a7d44ffcd15 → execute=completed → verify=completed
re-observe count: 3

## 情境 D：動器離線 → fallback（誠實記錄首選未執行）
（見 crates/interaction-runtime/tests/runtime_loop.rs::scenario_d — mock Offline → blocked + conversation completed）

## SSE 事件流（Last-Event-ID=0 重播）
  28 event: capability.changed
   9 event: receptor.registered
   8 event: plan.authorized
   8 event: action.completed
   8 event: action.acknowledged
   8 event: action.accepted
   7 event: plan.created
   6 event: actuator.registered
   3 event: receptor.observation
   3 event: policy.changed
   2 event: plan.blocked
   2 event: consent.changed

## 緊急停止（CLI 觸發，不依賴 UI）
emergency-stop: (0, 6)
e-stop 期間 execute exit code: 7（7=locked）
audit tail: ['emergency.clear', 'emergency.stop']

## 工具匯出（單一 canonical → 5 格式）
- openai: 7279 chars, warnings=0
- anthropic: 6898 chars, warnings=0
- gemini: 6456 chars, warnings=0
- openapi: 60412 chars, warnings=0
- json-schema: 57353 chars, warnings=0

---

# 部署驗證（v0.1.3，真 Release 資產、本機 macOS arm64）— 2026-08-26

## 升級鏈
v0.1.0 → self update（sha256 驗證＋原子替換）→ v0.1.1 → v0.1.2 → v0.1.3；
`self version --check` 每步正確偵測；跨 AI skill 同步安裝至 5 個 agent home
（Claude Code／Codex／~/.agents／Gemini／Copilot，偵測到的全裝、選單可選）。

## 能力矩陣：47/47 PASS
12 個 canonical tools 全部行為正確（含 cancel-已完成 → conflict、estop 期間
execute → exit 7）；受器啟停/推送/動態新增、動器測試（含真 macOS 通知）、
magnitude 0.95→0.8 夾制、quiet hours 降級、consent 撤回即擋、5 格式匯出
0 warnings、SSE 重播、穩定 exit codes。

## 註冊/權限鏈：20/20 PASS
動態註冊（一般/敏感受器＋mock 裝置）→ 能力快照即時反映（版本遞增＋
registered/capability.changed 事件）→ 四道閘門逐一攔截驗證
（預設停用 → enable → policy allowlist → session consent）→ 全開後
完整狀態機 completed(observed) → 移除（連配對受器一併移除）。

## 桌面控制中心（release dmg）：12/12 PASS
啟動→內嵌 runtime API 上線→經同一套服務執行閉環→UI 即時同步（總覽/受器/
動器頁截圖見 docs/assets/desktop-*.png）→緊急停止/解除→視窗關閉與
AppleScript quit 均優雅關閉（clean_shutdown=true）。
受器頁：camera.fake 顯示「需同意」＋disabled＋啟用鈕；動器頁：dev.device
顯示裝置上限（maxMag 0.8/maxDur 10s）、webhook.output 顯示「外部副作用」。

## 實測發現並修復的缺陷（各附回歸測試）
1. v0.1.2：緊急停止 clear 後閂死裝置無法重新武裝（新增 Actuator::emergency_clear）
2. v0.1.3：app 層級退出不觸發 runtime 優雅關閉（RunEvent::Exit handler）
3. v0.1.3：動態註冊 mock 裝置缺配對 status 受器，observed 驗證無法閉環
4. 發版流程：golden schemas 內嵌版本號、bump 必 drift（release.sh 自動再生）

---

## 人類理解層驗收（v0.2.0，2026-08-26，Apple Silicon macOS 實機）

### 自動化測試
| 套件 | 指令 | 結果 |
|---|---|---|
| Rust workspace（30 個測試套件） | `cargo test --workspace` | 全綠（含新增：human serde 相容、catalog alias/glob、resolver 優先鏈與保守 unknown、AI 說明 hash 綁定、pause≠estop、pause 重啟持久、AI gate（確定性不呼叫 AI／no-action／fallback／resolve 單次觸發）、onboarding draft/commit、scenario 模擬零副作用、recipe round-trip 未知欄位保留、summarize） |
| clippy | `cargo clippy --workspace --all-targets -- -D warnings` | 0 warnings |
| API E2E | `cargo test -p interaction-api`（9 tests，含 human-layer endpoints roundtrip） | 全綠 |
| 前端元件 | `pnpm test`（vitest，22 tests） | 全綠（誠實性不變量、卡片、權限地圖、對話框二段確認、精靈預選） |
| 前端 build | `pnpm typecheck && pnpm build` | 通過 |
| CLI 人類層 E2E | `scratchpad/human-e2e.sh`（真 daemon＋CLI） | **27/27 PASS**（human cards zh/en、catalog、onboarding commit、pause≠estop＋暫停中 recipe 不觸發＋明確請求照常、AI 說明錯 hash 409、scenario 模擬零副作用、summary、prefs、convert 保留未知欄位） |

### 桌面 UI 實機走查（debug .app、全新 home）
1. 首次啟動 → 精靈自動開啟；敏感來源不在可選清單、對外寫入不預選 ✔（`simple-onboarding.png`）
2. 精靈 commit → policy＋starter recipes＋completed 一次套用；draft 清空 ✔
3. 首頁：狀態／主動互動（含暫停控制）／三區權限地圖／最近互動故事 ✔（`simple-home.png`）
4. 回應方式卡片：誠實確認層級（「無法確認你是否已經看見」）、需同意徽章 ✔（`simple-responses.png`）
5. 自動互動：自然語言摘要與結構一致；模擬對話框 7 情境、分階段報告、「不會執行」判定、零副作用 ✔（`simple-automations.png`、`simple-simulate.png`）
6. 緊急停止：二段確認觸發（`/ready` 確認 `emergencyStop:true`）；頂欄變「前往解除」；解除走安全頁流程（原因＋時間＋會/不會恢復清單＋再確認），後端確認後才解除 ✔（`simple-estop-recovery.png`）
7. 進階模式切換：側欄多出 7 個原始技術頁、偏好持久化 ✔（`simple-advanced.png`）
8. 390px 窄視窗：側欄轉頂列、單欄卡片、可完成主要流程 ✔（`simple-narrow.png`）
9. 視窗關閉 → runtime 優雅停止（`clean_shutdown=true`）✔

### 實測發現並修復的缺陷
- **estop 二段確認按鈕紅字紅底**（`button.danger` 色彩疊在 `.estop` 紅底上，
  確認文字不可見）→ CSS 修正 `.estop.danger` 白字，重測通過。

### 誠實聲明（未執行／限制）
- 瀏覽器級 E2E（Playwright 等）未導入：Tauri WebView 走查以 AppleScript AX 實測代替，
  涵蓋精靈、導覽、模擬、estop 全流程；元件層由 vitest 覆蓋。
- `interruptiveness`／`confirmationLevel` 對內建動器目前多顯示「未知」：內建 adapter
  尚未宣告 `human.effect` 正式語意（目錄 typical 依規範不得當成事實）。列入下一步。
- 走查期間使用者同時在操作這台電腦，兩張截圖曾拍到非 app 視窗，已刪除重拍；
  自動化點擊已改為「前景驗證＋AX 元素定位」以避免誤觸其他應用程式。


### 對抗性審查（30-agent workflow）
5 個維度（安全繞過／誠實性／併發／round-trip／前端邏輯）各自獨立審查，每個發現再由
獨立驗證者對照程式碼對抗確認：**39 個發現、23 個確認**，全部修復並加回歸測試：

**安全（major）**
- assist 等待期間停用配方／撤回配方級同意後，timeout fallback 與 `proceed` 仍會觸發
  → `fire_recipe_deterministic` 重新檢查 enabled＋recipe consent＋estop（測試鎖住）。
- `ai.requireHumanConfirmation` 是死旗標 → 現在強制執行：API（AI 面）resolve `proceed`
  一律 `approval_required`，只有桌面 IPC（人類面）能確認；timeout fallback 自動降級為
  no-action（測試鎖住）。
- onboarding commit 可批次啟用需同意（敏感）元件 → 現在拒絕並導向明確同意流程（測試鎖住）。
- 無效 decision 會把 assist 重插回 map 造成洩漏與過期可解 → 先驗證後認領＋過期拒絕。

**誠實性（major）**
- 影響預覽把「未知」顯示成「不會」→ 三值判斷＋「資訊不足、無法保證」區塊。
- 首頁互動故事從事件流猜 AI 介入（會誤掛到別的配方）→ 改讀該 plan 持久化的
  `metadata.aiGate` 真實決策資料。
- 精靈確認頁硬編「都在本機運作」→ 改由實際選擇的卡片語意計算。
- 精靈預選把「未知風險」當安全 → 只預選「確定安全」；同時讓內建 adapters 正式宣告
  data/effect 語意（也修掉卡片上的「可以確認：未知」）。
- estop 解除清單把 physicalEffect unknown 列為「會恢復」→ 對齊後端事實（以同意為準）。

**round-trip（major）**
- `message:`／`ai:` 區塊內的未知欄位會被丟棄 → serde flatten 保留（測試鎖住）。
- recipe 以 `.yml`/`.json`/異名檔案為底時，編輯會分叉、刪除會復活 → save/delete
  改為「依內容 id 掃描」（測試鎖住）；`remove_recipe` 不再吞檔案刪除錯誤。
- 運算式字串條件會被句子編輯器覆寫 → 視為進階結構原樣保留。

**其餘 minor**（併發順序、每日上限計數、estop 失敗無聲、各處錯誤處理、a11y 焦點圈養、
availability 文案、摘要涵蓋 chance/AI 生成文字…）共 14 項一併修復。
2 個發現經驗證為誤報（合成事件情境、無法達成的組合）。

---

# v0.3 驗收（狀態列常駐＋桌面角色＋外部裝置＋AI Session＋感測，2026-08-26，Apple Silicon macOS 實機）

## 自動化測試（每組實際數字）
- `cargo test --workspace`：**201 passed / 0 failed**（含新整合套件 providers_loop 5、
  agents_loop 9、sensors_loop 6、declarative http_loop 5；桌面 tray/supervisor 單元 4）
- `cargo clippy --workspace --all-targets -- -D warnings`：**0**
- `cargo fmt --check`：通過
- 前端 `pnpm typecheck && pnpm test && pnpm build`：**vitest 40/40**、typecheck、build 全過
  （新增 packs 6、companion machine +2）
- 瀏覽器級 **Playwright E2E**（真 daemon＋真 Chromium）：**11/11**
  （onboarding／權限地圖／暫停／句子編輯器＋零副作用模擬／進階切換／緊急停止觸發＋
  安全解除／390px 導覽＋鍵盤＋無水平捲動／離線誠實）

## CLI E2E（真 daemon＋真 mock device，斷言不變量非「有跑就好」）：12/12 PASS
`scripts/v03-cli-e2e.sh`——providers（宣告式裝置為 Installed 非自動 available、revoke 黏性、
revoked→available 被拒）、agent sessions（Created 狀態、訊息預算硬上限、聲稱完成是 OPEN
狀態、關閉）、sensors（無同意拒絕聆聽、預設不可用）、estop 傳播（取消 open session＋阻擋
新建）。

## 桌面實機走查（debug .app、全新／既有 home）
- 啟動：`Interaction Control Center`＋`小樞` 兩視窗，`/ready` 正常
- **關閉控制中心 → 首次說明對話框**（含 v0.2→v0.3 行為改變告知）→「保持運作」→
  主視窗隱藏、`/ready` 仍回應、小樞留存、process 續跑（截圖 `v03-close-dialog.png`）
- **桌面角色小樞**：透明無邊框視窗實機渲染、idle 姿態（runtime online 才睜眼）、
  三變體與 sprite sheet（`v03-companion-shu.png`／`v03-shu-variants.png`／`v03-shu-spritesheet.png`）
- **首次見面劇情**：一次性氣泡觸發、進度落盤 `{meet:true}` 後不再重播
  （`v03-story-first-meeting.png`）
- **Persona 世界觀端到端**：設定頁切「導航員」→ 角色視窗重載 → CLI 觸發 success →
  氣泡顯示世界觀台詞「任務節點已完成。」（取代預設「做完了。」）；安全訊息不受影響
  （`v03-settings-companion.png`／`v03-persona-navigator.png`）
- 完全結束（Cmd+Q）→ 優雅關閉、`/ready` 無回應
- 走查後將機器上被改動的 persona 偏好還原為預設

## 對抗性安全審查（多 agent workflow）：48 raised → 31 confirmed → 27 修復＋4 記錄
5 維度獨立審查（runtime 生命週期／provider-device／agent session／sensor 隱私／
前端誠實）→ 每發現獨立對抗驗證。確認 31 項（18 major、13 minor）。

**已修復的 major（節選）**
- SSRF：redirect 未重驗、IPv4-mapped IPv6（`::ffff:169.254.x`）繞過 metadata 封鎖 →
  no-redirect policy＋IP 解析封鎖全編碼
- 裝置回應體被存進 receipt（可回射 secret 憑證）→ 只留 httpStatus
- 麥克風 start-timeout 孤兒執行緒（無限靜默擷取）→ 逾時即設 stop 旗標釋放裝置
- 撤回受器同意／結束 session 不停止擷取 → 立即停止；estop TOCTOU → 開窗後重檢＋
  watchdog 掃描；retention:none 未落實 → 麥克風衍生事實不入 SQLite
- agent 聲稱被當驗證證據（自 push actionId）→ ingest 改名 claimActionId；
  委派防循環信封全由呼叫端捏造 → 建立序列化＋max_parallel 依 rootTaskId＋信封遞增
- max_messages=0 = 無限 → 對政策取 min；狀態列麥克風指示是死碼 → 讀 activeSensors；
  內嵌 API bind 失敗被吞（顯示健康）→ 設 Degraded＋顯示錯誤
- 實例鎖 TOCTOU（雙 runtime）→ O_EXCL 原子建立＋只刪自己 PID 的鎖
- 小樞緊急停止標籤在純輪詢偵測時不重繪 → base 鏡射進 React state；
  過期氣泡計時器抹掉安全氣泡 → 追蹤並清除；action.failed 與 unknown 混淆 → 獨立處理

**記錄為已知限制（未在本輪修復，附原因與替代檢查）**——見下節。

## 已知限制（v0.3，誠實聲明）
1. **單一扁平 API token（架構級）**：AI 面與人類面共用權限，AI 技術上可呼叫
   estop-clear／consent-grant／sensor-listen。已用 skill 安全條款明令禁止並全程審計，
   桌面 IPC 是唯一能滿足 `requireHumanConfirmation` 的面；但真正的分權需 **scoped
   token**（下一版）。這是 v0.2 起就記錄的限制，本輪未改。
2. **裝置身分逐請求強制未實作**：宣告式 HTTP/mock 裝置不出示金鑰，配對指紋已加隨機
   salt（不再是公開資料的確定性雜湊），但無法在每次請求時擋下同位址的冒充者——需裝置端
   crypto。已在程式碼與 ARCHITECTURE.md 誠實標註。
3. **外部 daemon 模式的 Degraded（token 不可讀）不啟動健康輪詢**：邊緣情況，backend()
   回 None 故一切 fail-closed，重啟即恢復。
4. **麥克風真擷取未在此機器實測**：會觸發 macOS 權限提示並錄環境音，未經你在場明確
   同意我不開麥；cpal 路徑已編譯，以確定性 fake source 完整測試（sensors_loop 6/6），
   需要一次你在場的手動驗證。
5. **攝影機誠實未實作**：不做假 driver；catalog 標為不可用。
6. **WS/MQTT/Serial/BLE transport**：宣告式引擎解析但誠實拒絕（僅 HTTP/SSE 實作）。
7. **第三方 pack zip 安裝流程未做**：內建 packs 與驗證器齊備，但沒有 zip 匯入 UI；
   pack 驗證器已防 zip-slip 概念（sheet 僅限純檔名）。

---

# v0.4 驗收證據 — 2026-08-27

全部證據可重跑；命令列於各節。基準：main@18037e5（v0.3.0）。

## 回歸（v0.3 不變量全數保留）
- `cargo test --workspace` → **257/257 passed, 0 failed**（v0.3 基線 201 → +56）
- `cargo clippy --workspace --all-targets -- -D warnings` → 0 errors
- `cd apps/interaction-desktop && pnpm typecheck && pnpm test` → typecheck 0 錯誤、vitest **61/61**（v0.3 基線 40 → +21）
- `pnpm test:e2e` → Playwright **19/19**（v0.3 基線 11 → +8；真 daemon＋真 Chromium）
- `./scripts/v03-cli-e2e.sh` → CLI E2E **42/42**（v0.3 基線 12 → +30；真 daemon＋mock device＋fake agent 子程序）

## Presentation Provider（真實迴路）
- 整合測試 `cargo test -p interaction-runtime --test presentation_loop`（8）：
  逐項 provider（7+7）、bubble dispatched→ack→completed（AcknowledgedOnly 證據）、
  無 ack→Uncertain、隱藏拒 ingest／actuator 誠實失敗、estop 清佇列拒遲到 ack、
  behaviorIntent 白名單全迴路、斷線 receptor Offline。
- CLI E2E：`presentation hello/status/ack` 七項檢查（含 HTTP 503 誠實拒絕）。

## 真實 Agent Connector（本機已登入 agent；無模型 API、無假接線）
- 協定鎖定：`codex app-server generate-json-schema`（codex-cli 0.149.1；
  ClientRequest 95 方法／ServerNotification 75）；Claude stream-json 事件實錄樣本。
- **真實連線驗收（2026-08-27，隔離 home，真 daemon）**：
  - `interact-ai agents providers --json` → claude-code 2.1.247（loggedIn:true）＋
    codex 0.149.1（loggedIn:true，app-server 可用）
  - Claude Code session `asession-37236be7…`：任務「Reply with exactly OK」→
    state=claimed-completed、providerSessionId=f200e65c-92f7-4cd1-965b-150fa8693216、
    spentCost=$0.42094 入預算、mailbox result={"summary":"OK","costUsd":0.42094}
  - Codex session `asession-0fcb0a8d…`：app-server 握手→thread/start（read-only）→
    turn/start → state=claimed-completed、providerSessionId=01a04194-074f-7e92…
  - claims 落為 observation inferences（confidence 0.5）——實測輸出見上方 CLI 段
- 子程序樹終止：`gateway_loop` 測試以 pid 檔證明 estop 後 fixture 程序組死亡。
- fake agent（`tests/fixtures/fake_claude.sh`）：CLI E2E 六項確定性檢查（絕不動用真額度）。

## 記憶／知識／決策器
- `memory_loop`（5）：actor 降權、secret 拒收、三態期限、到期清除、
  確定性 bundle（stale/敏感/denylist/candidate 排除並揭露）、handoff 落地。
- `knowledge_loop`（6）：CAS write-once、claim 要證據、agent 只能 Candidate、
  agent approve 降留言、人類 approve→active、類比≠因果、supersede 版本化、
  FTS＋lexical-vector 候選、刪素材→disputed＋級聯。
- `curator_loop`（4）＋curator 單元（3）：決策表（repo-commit 不用 AI、外部研究必先問）、
  freshness→stale、衝突雙方→disputed、經驗升格閘門（無反例＋適用範圍不可 approve）、
  receipt 誠實欄位。

## 控制中心新 IA＋畫面證據
- Playwright：8 一級頁可達、AI 頁 Consent Sheet、記憶知識頁、全域搜尋（⌘K）導頁、
  390px 底部導覽＋鍵盤＋無水平捲動、離線誠實畫面。
- 畫面證據 `docs/assets/v04-evidence/`（**24 張，全部來自真 App＋真 daemon**）：
  9 頁 × 桌面＋390px、全域搜尋、緊急停止（桌面＋390px）、離線；
  實機（Tauri dev、隔離 home）：`live-companion-shu-v2.png`
  （v2 貓系小樞於真實桌面、story first-meeting 真實觸發）、
  `live-control-center.png`（初次設定精靈＝初次使用狀態）。
- 空白／初次使用狀態：以全新 home 的真實空狀態擷取（不硬編假資料）。

## 已知限制（v0.4 誠實記錄；另見 capability-completion-matrix.md）
1. 單一扁平 API token 沿用（presentation ack／human review 面與 AI 面同 token；
   需 scoped token 才能完全分離——v0.3 已知限制①延續）。
2. Codex exec fallback 未實作：app-server 不可用的舊版 codex 誠實拒絕建立 session。
3. 向量檢索為誠實標示的 lexical-fallback（詞彙雜湊袋餘弦），非語意 embedding；
   介面可替換。
4. 影音素材的衍生解析（縮圖/OCR/轉錄）未實作；資料模型與片段引用語法已就緒。
5. OS 層硬體列舉（HID/BLE/MIDI/mDNS/攝影機）誠實未實作，UI 明列具體原因。
6. Claude -p 模式無互動核可管道（plan 模式寫入直接拒）；寫入型工作流程為下一階段。
7. 程序化眼球／耳朵疊加層未實作（錨點已輸出到 manifest；反應鏈烘焙於動畫時間軸）。
8. 生成式主動對話的實際內容生成（§6.1 呼叫 agent 產生候選訊息）機制已就緒
   （閘門＋預算＋metadata），觸發端排程器為下一階段。
9. 知識系統的三個 UI 末端未接線：update-check 觸發僅 API/CLI（決策結果仍經
   候選/收據頁呈現）、「使用者糾正小樞」專屬入口未做（糾正仍可經候選複審）、
   角色端知識六句固定文案（§17）未接到氣泡（收據語意在控制中心完整呈現）。
