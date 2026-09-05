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
6. **WS/MQTT/Serial/BLE transport**：v0.3 時宣告式引擎只解析不實作（僅 HTTP/SSE）；
   **v0.5 起 Serial／MQTT／BLE 已實作**（見下方 v0.5 章節），WebSocket／HID／Home Assistant
   仍未實作。
7. **第三方 pack zip 安裝流程未做**：內建 packs 與驗證器齊備，但沒有 zip 匯入 UI；
   pack 驗證器已防 zip-slip 概念（sheet 僅限純檔名）。

---

# v0.4 驗收證據 — 2026-08-28

全部證據可重跑；命令列於各節。基準：main@18037e5（v0.3.0）。
本輪 closing 的二進位／畫面 SHA-256、真實 connector 與測試數字見
[`v04-final-machine-evidence.md`](v04-final-machine-evidence.md)。

## 回歸（v0.3 不變量全數保留；本輪 2026-08-28 實測）
- `cargo test --workspace` → **336 passed, 0 failed, 0 ignored**（v0.3 基線 201 → +135）
- `cargo fmt --check`＋`cargo clippy --workspace --all-targets -- -D warnings` → 0 errors
- `cargo test --manifest-path apps/interaction-desktop/src-tauri/Cargo.toml` → **4 passed**
- `cd apps/interaction-desktop && pnpm typecheck && pnpm test && pnpm build` →
  typecheck/build 成功、vitest **94/94**（v0.3 基線 40 → +54）
- `pnpm test:e2e` → Playwright **23/23**（v0.3 基線 11 → +12；真 daemon＋真 Chromium）
- `./scripts/v03-cli-e2e.sh` → CLI E2E **51/51、exit 0**（v0.3 基線 12 → +39；
  真 daemon＋限權 agent token＋mock device＋真 agent fixture 子程序）

## 對抗審查（v0.4；`.claude/workflows/adversarial-review-v04.js`，find→independent verify）
- 8 個維度（presentation-honesty／gateway-safety／memory-privacy／knowledge-integrity／
  proactive-limits／state-machine-v04／frontend-honesty／regression-v03）共 67 個 agent。
- **59 個 findings → 逐項獨立對抗驗證 → 38 確認／21 駁回**。
- **38/38 確認缺陷全數修復**，每項附 regression test（Rust +37、vitest +20）。代表性修復：
  - presentation_ack 先發事件後持久化（estop/expiry 競態下對 /v1/events 謊稱完成）→
    persist-成功才發事件（`presentation_loop` 新 regression）。
  - kill_tree 領頭寬限內退出就跳過 SIGKILL 升級／孤兒程序跨重啟存活／stdin 寫入佔住
    handle 鎖使 estop 無法終止卡死 agent → 鎖外 ProcessGroup terminate＋pgid 持久化
    （meta）＋啟動 reap＋send/interrupt/approval 有界逾時（`process.rs` 3 測試＋
    `gateway_loop` 崩潰模擬）。
  - GET messages 對 gateway session 蓋 delivered/acknowledged 戳記（觀看≠送達）→
    送達語意只屬 gateway_deliver 真實轉發。
  - estop/create TOCTOU → create 鎖序列化＋建立後複查回滾；close 無 terminal guard →
    Conflict 拒重複關閉。
  - 記憶：far-future reviewAfter 逃過 candidate 降權（改看 horizon）、secret 掃描漏
    tags/provenance、過期記憶 sweep 前仍被供應／可復活（讀取端即拒）、清除 1000 上限
    靜默截斷（清到空或誠實回報殘量）。
  - 知識：空 SourceRef 過證據門檻、approve 復活 superseded/archived、未審核 candidate
    邊可把 active 拉成 disputed、approve 不重驗證據（懸空 hash 拒升格）、`LIKE %hash%`
    改精確 json_extract、freshness/級聯 1000 上限改 keyset 掃全量。
  - 主動對話：dnd_defer 死碼 → 接上 policy quietHours 確定性生效；persist 失敗不再靜默。
  - 前端誠實：全域搜尋 estop 改二段確認＋IME（isComposing/229）防誤觸、指令失敗回報、
    匯出真的顯示在畫面、拖放/關閉/收件匣計數/倒數計時文案全部對齊真實狀態。
- 21 項駁回皆有具體反證（例：AiPage「已送達」實為 gateway 即時轉發、Codex
  provider_session_id 於 start_session 已就緒），詳見 workflow journal。

## 指定 closing 對抗審查（`.claude/workflows/adversarial-review-adaptive-interaction.js`）

- Workflow `wf_856f48f8-3e9`／session `911ad58c-d833-4e34-942d-390126c27d95` 完成，
  **41 agents、0 workflow errors、617 秒、36 findings → 12 confirmed／24 rejected**。
- 先前提到的「存活約 85 分鐘」只是舊嘗試的 OS elapsed time，不是完成證據；本次以
  workflow final output 與 journal 為準。
- 12/12 確認缺陷均修復並加回歸：Governor 用量／費用 reservation 原子性、同 plan
  single-flight、recipe direct-run policy bypass、late driver evidence、超大 duration、
  mock 界限、supersede schema、restricted SSE、nested transport error、rolling hourly
  window、equal-quality fusion、driver status-chain spoof。
- Final output：`/private/tmp/claude-501/-Users-user-Workspace-claude-lab-adaptive-interaction/911ad58c-d833-4e34-942d-390126c27d95/tasks/w0k3dcase.output`；
  journal：`~/.claude/projects/-Users-user-Workspace-claude-lab-adaptive-interaction/911ad58c-d833-4e34-942d-390126c27d95/subagents/workflows/wf_856f48f8-3e9/journal.jsonl`。

## Presentation Provider（真實迴路）
- 整合測試 `cargo test -p interaction-runtime --test presentation_loop`（8）：
  逐項 provider（7+7）、bubble dispatched→ack→completed（AcknowledgedOnly 證據）、
  無 ack→Uncertain、隱藏拒 ingest／actuator 誠實失敗、estop 清佇列拒遲到 ack、
  behaviorIntent 白名單全迴路、斷線 receptor Offline。
- CLI E2E：`presentation hello/status/ack` 七項檢查（含 HTTP 503 誠實拒絕）。

## 真實 Agent Connector（本機已登入 agent；無模型 API、無假接線）
- 協定鎖定：`codex app-server generate-json-schema`（最初鎖定於 codex-cli 0.149.1；
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
- **本輪重驗（2026-08-27）**：Codex 0.150.1 與 Claude Code 2.1.247 都由
  全新隔離 runtime home 建立只讀 Session。Codex app-server 產生的 request
  `item/commandExecution/requestApproval \"/bin/zsh -lc 'cat README.md'\"` 先停在
  `waiting-for-consent`，人類面只核可 request `0` 後才讀取；結果為
  `claimed-completed`，Mailbox 文字另以 `rg` 對 README 標題獨立比對。Claude
  stream-json 同樣回報 `claimed-completed`。兩者均有 Provider Session ID，
  均沒有被作為 `verified`。
- Codex 不支援 app-server 時的 `exec --json`／`exec resume` fallback 已實作；
  本機新版實際走 app-server，fallback 以真子程序 fixture 驗證 safe args、
  resume、malformed event 與 cancel。
- 子程序樹終止：`gateway_loop` 測試以 pid 檔證明 estop 後 fixture 程序組死亡。
- fake agent（`tests/fixtures/fake_claude.sh`）：CLI E2E 六項確定性檢查（絕不動用真額度）。

## 記憶／知識／決策器
- `memory_loop`（5）：actor 降權、secret 拒收、三態期限、到期清除、
  確定性 bundle（stale/敏感/denylist/candidate 排除並揭露）、handoff 落地。
- `knowledge_loop`（6）：CAS write-once、claim 要證據、agent 只能 Candidate、
  agent approve 降留言、人類 approve→active、類比≠因果、supersede 版本化、
  FTS＋lexical-vector 候選、刪素材→disputed＋級聯。
- `curator_loop`（7）＋curator 單元：決策表（repo-commit 不用 AI、外部研究必先問）、
  freshness→stale、衝突雙方→disputed、經驗升格閘門（無反例＋適用範圍不可 approve）、
  使用者糾正只建 UserMemory＋Knowledge Candidate、receipt 誠實欄位。

## 控制中心新 IA＋畫面證據
- Playwright：8 一級頁可達、AI 頁 Consent Sheet、記憶知識頁、全域搜尋（⌘K）導頁、
  390px 底部導覽＋鍵盤＋無水平捲動、離線誠實畫面。
- 畫面證據 `docs/assets/v04-evidence/`（**100 張，全部來自真 App＋真 daemon**）：
  9 頁 × desktop/390px × normal/empty、loading、error/unknown、waiting、emergency；
  offline 為 app-level shared state，另有 desktop/390px；再含 hardware scan、Global Search；
  實機（Tauri dev、隔離 home）：`live-companion-shu-v2.png`
  （v2 貓系小樞於真實桌面、story first-meeting 真實觸發）、
  `live-control-center.png`（初次設定精靈＝初次使用狀態）。
- 空白／初次使用狀態：每個案例使用全新隔離 home 的真實空資料；loading 是延遲真請求，
  error/unknown 是中斷一個真 transport request，waiting 建立真 Knowledge Candidate，
  emergency 觸發真 estop；不注入成功假資料。

## 已知限制（v0.4 初版歷史快照；已由下節取代）
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
10. agent 子程序孤兒回收為 best-effort：pgid 以 OS 快照歸因（候選不唯一時誠實
    放棄記錄並 warn，該 session 遇崩潰會漏 reap）；重啟 reap 驗證「存活＋group
    leader＋command line 相同」，pid 重用且 command 完全相同的極端情況可能誤殺；
    daemon 在 close 的 kill 寬限期內崩潰時已 forget 的 pgid 不再被找回。
    程式註解逐一標明（agents.rs／runtime.rs）。

## 已知限制（v0.4 最終 closing audit）

1. Human、restricted Agent、Session/Domain capability token 已分層；同一 OS 帳號的
   程序檔案可見性仍取決於 Codex／Claude Code 自身 sandbox。0600 token file 不被誤稱為
   完整 OS 程序隔離。
2. 本機向量是可重現的 sparse subword embedding，不宣稱為 neural embedding；介面保留
   可替換索引。
3. thumbnail 與 WAV features 有 production implementation；OCR、whisper、ffmpeg
   derivative 依賴使用者本機已安裝工具，缺少時產生 `unavailable`，不以假輸出冒充。
4. hardware discovery 只承諾「目前可見」；driver、權限、sandbox、未配對或裝置占用仍可
   使設備不可見。Windows 尚未在真機驗收，會回傳具體 unsupported reason。
5. Codex exec fallback 因本機 Codex 支援 app-server，無法強迫真 CLI 走舊版路徑；以真
   子程序 fixture 驗證 JSON stream、resume、malformed event、cancel 與 process tree。
6. Agent claim 一律停在 `claimed-completed`，本輪兩個真連線沒有被標為 verified；這是
   誠實階梯，不是未接線。
7. Offline 是整個 App 的 shared state；Runtime 離線時一級頁資料不可達，因此只提供一組
   desktop/390px offline 證據，不複製九張內容相同的假頁面。
8. 記憶 JSON 還原是逐筆、重新驗證、配置新 ID 的邏輯還原，不覆寫 SQLite；已成功項在後續
   項失敗時保留並誠實回報，不宣稱跨筆 transaction。
9. Unix orphan process 回收用持久化 pgid、leader 與 command identity；若不能唯一歸因就
   fail-safe 放棄並寫 warning，避免誤殺外部 daemon。
10. 本輪未 push、release、deploy、開 PR 或建立 commit；依 repo 規則需使用者明確授權，
    不影響本機工程與驗收結果。

---

## v0.5（Phase 7 收尾；2026-08-28，macOS 26.2／Apple M2 Pro／rustc 1.94.0／node 24.5.0／pnpm 10.27.0）

> 本節是 v0.5 的驗收證據總表。所有數字都是本 Session 在同一台機器實跑；模擬器／fixture／程序內 client 一律標示，
> 沒有任何真機（ESP32／iPhone）證據。逐條恢復矩陣見 `docs/v05-recovery-matrix.md`，收尾狀態見
> `docs/v05-capability-gap-matrix.md` §9。

### 自動化回歸（Phase 7 修復後，最後一次全套）

| 套件 | 命令 | 結果 |
|---|---|---|
| Rust fmt | `cargo fmt --check` | 通過（exit 0） |
| Rust clippy | `cargo clippy --workspace --all-targets -- -D warnings` | 通過，0 warning |
| Rust workspace tests | `cargo test --workspace` | **426 passed / 0 failed / 0 ignored**（Phase 0 基線 336；本輪 Phase 1–6 起點 349；Phase 7 +77；mobile_loop 25、protocol_honesty 21、agents_loop 16、api_e2e 20…） |
| Tauri | `cargo test --manifest-path apps/interaction-desktop/src-tauri/Cargo.toml`＋同路徑 clippy | **8 passed / 0 failed**；clippy 0 warning |
| 前端 typecheck | `pnpm typecheck` | 通過 |
| 前端 unit | `pnpm test`（vitest） | **319 passed / 0 failed**（21 檔；基線 94／11 檔） |
| 前端 build | `pnpm build` | 成功（index 491 kB／gzip 163 kB） |
| CLI／API E2E | `./scripts/v03-cli-e2e.sh`（真 daemon＋mock HTTP 裝置＋**Serial 模擬器**＋fake agent 子程序） | **63 passed / 0 failed**（基線 51；Serial(SIMULATOR) 段 8 check、estop 段 3） |
| Playwright | `pnpm test:e2e`（真 daemon＋Chromium，桌面 1200px 與 390px） | **24 passed / 0 failed**（含通知中心鍵盤真斷言、§11 一般模式三區契約） |
| Golden schema | `cargo test -p interaction-e2e --test golden` | 5 passed，golden **未漂移**（未重生） |
| iOS typecheck | `xcrun swiftc -typecheck -sdk iphonesimulator26.5 -target arm64-apple-ios17.0-simulator`（12 檔） | **0 error / 0 warning** |
| iOS XCTest | swiftc 編 `.xctest` bundle → `xctest` agent 於 iPhone 17 模擬器 | **19 passed / 0 failed**（MotionClassifier 8＋Protocol 11）——**模擬器** |
| ESP32 韌體 | `./firmware/esp32-companion/compile.sh [--ble]`（arduino-cli 1.5.1、esp32:esp32 3.3.11） | 兩組態 **0 error、本韌體 0 warning**（ESP32Servo 函式庫 4 個 unused warning）；938 KB／1188 KB——**只證明可編譯，未燒錄真板** |
| 模擬器協定對測 | pty 對測 `scripts/esp32-serial-sim.py`＋從 `.ino` 逐字抽出的參數／loop 邏輯在桌面編譯執行 | 44/44、30/30（一次性檢查腳本在 scratch，非回歸套件） |

### 角色效能量測（可重現：`cd apps/interaction-desktop && pnpm perf`；headless Chromium 151、DPR 2、含 raster flush；2026-09-02 14:34Z 重跑（Phase 8 修復後），啟動旗標 `--js-flags=--expose-gc --enable-precise-memory-info`）

> **量測範圍**：下表全部是 **WebView 內段**（合成 pointer 呼叫→下一幀），不含 Rust 點擊穿透閘與 OS 派送；
> **端到端（OS pointer 事件→WebView 收到→toy.grabbed）沒有量測**，因此不得把下面的 8.3 ms 讀成「達到規格 §14 的 16–100 ms」。

| 指標 | 結果 |
|---|---|
| drawRig 單角色一幀（160×200，36 表情輪流） | median **0.100 ms**／p95 0.140／max 2.02（n=72，每樣本 10 幀） |
| 全舞台一幀（角色＋2 使魔＋3 玩具＋物理＋時間軸，416×216） | median **0.240 ms**／p95 0.280／max 0.40（n=120） |
| rAF 間隔（headless 節奏，非使用者螢幕） | median 8.3 ms／p95 9.1／max 9.3（n=600） |
| 舞台自身 rAF 主迴圈（loop()＋幀預算） | ticks=362 drawn=362 skipEveryOther=false，視窗平均成本 0.188 ms（rAF gap median 8.3／p95 9.3／max 10.4，n=360） |
| **WebView 內段** 輸入→下一幀：抓玩具（合成 `stage.pointerDown`→toy.grabbed） | median **8.3 ms**／p95 9.3（20/20 幀確認狀態改變）——只是規格 §14「16–100 ms」路徑的 WebView 段，**不是端到端**；下界就是量測環境的幀距（headless 120 Hz ≈ 8.3 ms，60 Hz 螢幕上同碼約 16.7 ms），量到的是更新率不是處理成本（v0.5.1 對抗審查 perf-claims-014） |
| **WebView 內段** 輸入→下一幀：看向游標（合成 `stage.pointerMove` 進 hit-rect→gaze/耳朵參數改變） | median **8.3 ms**／p95 9.2（20/20）——同上，WebView 段 |
| Rust 點擊穿透閘（**未量測**；讀碼上限） | 游標移動：≤80 ms 輪詢（`CLICKTHROUGH_POLL_MS`）；角色／玩具移動：≤60 ms hit-rect 節流回報（`HIT_RECT_MAX_QUIET_MS`）＋**回報落地即重算**（Phase 8 修：`companion_hit_rect` 直接重評，不再等下一次輪詢；舊上限 60＋80≈140 ms）。主機端上限約 max(80, 60) ms＋IPC／OS 派送；沒有任何從 OS pointer 事件起算的量測或 e2e 測試，**不得宣稱端到端達標** |
| JS heap（600 幀前／後／GC 後；精確位元組） | 2.56 MB → 3.60 MB → 2.20 MB（raw 2 689 310／3 777 918／2 312 049 bytes；`--enable-precise-memory-info`，rig 自檢 quantized=no） |
| JS heap 浸泡（60.0 s 真 rAF、7205 幀、29 次抓放玩具、6 次 spawn、每 5 s 取樣） | GC 後 **1.86 MB → 2.08 MB（Δ +231 KB，+12.2%）**；GC 前峰值 3.38 MB；取樣 3.13/2.51/3.19/3.07/2.24/2.36/2.97/2.82/3.19/3.38/2.37/2.53 MB（分配→GC 鋸齒）。三次 60 s 重跑 GC 後終值差 <1 KB（2 182 281／2 182 641／2 182 125 bytes），像是固定保留而非隨時間累積，但 60 s 分不出快取暖身與洩漏——**未判定無洩漏**，分鐘級以上浸泡待做 |
| bounded：玩具上限 | 23 次 spawn → 場內 4 個（cap） |
| 長時間數值行為 | 時間軸模擬 3 天、20 萬取樣：全部有限且在 clamp 範圍 |

誠實：這是 Blink（Chromium）數字，不是 Tauri WKWebView；同機同碼相對基準。Phase 6 文件裡的「drawRig 0.452 ms（2.2x）」
沒有任何產生程式，已作廢。Phase 7 版本表格引用的「9.5 MB → 9.5 MB → 9.5 MB」是 Chromium 量化桶值（10 000 000 bytes／1 048 576），
不是量測值，已由精確位元組取代；rig 現在若讀到量化值會以非零結束，數字不得引用。

### 對抗審查（`.claude/workflows/adversarial-review-v05.js`；find → 獨立懷疑者 verify）

- 恢復矩陣：10 位審計 agent 逐條對照規格（463 列）＋完整性審查員裁決 12 組矛盾（`docs/v05-recovery-matrix.md`）。
- 對抗審查：11 個維度 finder＋33 個 seed 主張 → **136 項審查、73 confirmed、59 fixed-meanwhile（驗證時已被並行修復）、
  4 refuted、0 unverified**；blocker/high 由兩位不同視角懷疑者皆確認才算數。第一輪 151 個驗證因模型速率上限失敗，
  改以 sonnet 重跑驗證、opus 修復（主迴圈 fable-5）。
- 修復分 3 輪 8 組（mobile／硬體／角色／IA＋記憶 UI／Agent taxonomy／角色第二輪＋效能量測／Agent+SSE+IA 第二輪／
  provider 已測試／安全底線＋link 層／韌體），**每項修復皆附 regression test**（Rust +76、vitest +181、iOS XCTest 形狀更新）；
  4 個 refuted 為驗證時已修好的重複主張。
- 確認但**未修**（列為已知限制，見下）：跨視窗桌面漫遊／邊緣探頭、其他視窗事件語意反應、Fullscreen／OS 勿擾偵測、
  硬體 Observed/Verified 死路、BLE 真機、iPhone 真機、App 端 Bonjour 瀏覽、桌面端 BLE gateway 只有 scan、
  配對期可被區網 peer 燒掉（已加 audit 與 UI 欄位）、MQTT rumqttc 內部佇列 deadline 伸不進、waiting-input 無來源。

### iPhone Mobile Provider（**iOS 模擬器**，iPhone 17／iOS 26.2 runtime；非真機）

> v0.6.x 分支（2026-09-05）：XCTest **126/126**（對抗審查 Swift wave `df8e013` 後；同日稍早 120/120）（模擬器，iPhone 17／iOS 26.5 runtime；AIPConformance 17＋Lifecycle 22＋
> MotionClassifier 8＋Protocol 21＋ReconnectHint 21＋SessionClient 34＋StateHashConformance 3），修復者與獨立驗證者各跑一次；
> 真機仍為零執行。

- 配對閉環：`POST /v1/mobile/pairing-session` → App `--pairing-payload` → wss＋TLS 指紋固定＋HMAC → `GET /v1/mobile/status`
  `connected:true`；Keychain 重連 auth→auth-ok；`iphone.character` 動器收據 `acknowledged`＋`deviceApplied`；
  BLE scan 誠實回 `err ble-gateway-disabled`；motion 顯示「不可用」。截圖 `docs/assets/v05-evidence/ios-sim-01..07.png`。
- 第一次模擬器實測暴露兩個桌面端缺陷（撤銷不斷線、Bonjour 服務名 18 bytes 註冊失敗），Phase 7 修正；**第二輪模擬器復測**
  （新編 .app、真 daemon 18831）：`DELETE /v1/mobile/devices/{id}` 後 wss 連線於 **≤0.035 s** 消失（上一輪 +42 s 仍在）、App 立即顯示
  「配對已被撤銷或過期」（`ios-sim-08-revoke-immediate.png`）；`interact-ai emergency-stop` 後 `status.activeSensors` 於 0.064 s 清空、
  手機自報 micLevel/bleGateway 於 0.5 s 內轉 false、App 顯示「因桌面緊急停止而停用」（`ios-sim-09-mic-active.png`、
  `ios-sim-10-estop-sensors-off.png`）、audit `mobile.estop-stop-sensors`／`mobile.high-risk-receptor-disabled`；
  `status.bonjour = {advertised:true, service:"_interact-ai._tcp", instance:"interact-ai-18790"}` 且 `dns-sd -B` 真的看得到；
  XCTest 在模擬器 19/19。附帶發現：測試模式的 Runtime 曾把 Bonjour 記錄廣播到實體區網（`cargo test` 期間數十筆）——已改為
  測試模式不廣播（`status.bonjour.error = "disabled (test mode…)"`，回歸測試 `test_mode_never_advertises_bonjour_on_the_lan`）。
- **未驗證（真機專屬）**：haptic、torch、CoreMotion 語意事件、真實 BLE 掃描／GATT、通知顯示、TTS、QR 相機、
  真機型號、背景／前景 wss 行為、非 loopback 區網 TLS 釘選。

### 已知限制（v0.5，誠實聲明；修掉時同步更新 CHANGELOG）

1. **ESP32 真板未驗收**：韌體只經 arduino-cli 編譯、桌面等價執行與 pty 模擬器對測；接線、PWM、感測時序未在真板驗證。
2. **iPhone 真機未驗收**：只有模擬器；感測／haptic／BLE／推播在模擬器上不可用。App 端無 Bonjour 瀏覽（靠 QR／手動）。
3. **BLE adapter 無真機**：scan/connect/subscribe 只有 TaskSlot 回收與狀態機測試；Linux 誠實拒絕。
4. **硬體 Observed/Verified 死路**：LinkReceptor 的 state facts 沒有 actionId，硬體動作停在 acknowledged（附 deviceApplied）；
   顯式 `POST /v1/actions/{id}/verify`（observed 策略）會在 5 秒找不到 actionId 觀察時把收據轉 uncertain。
5. **provider 停用／撤銷後連線關閉不可逆**：重新啟用需重載 adapter spec（重啟 daemon 或重新匯入）。
6. **MQTT**：rumqttc 內部佇列的 deadline 伸不進；廣播前檢查與 publish 之間斷線的訊息仍可能在重連後送出。
   韌體 `g_mqtt.loop()` 已連線時最壞仍可能阻塞 1 s（PubSubClient socket timeout 粒度為秒）。
7. **waiting-input** taxonomy 狀態沒有任何 connector 能產生（Claude stream-json／Codex app-server 0.150 皆無對應事件）；
   API/CLI 回報路徑保留。
8. **角色空間**：跨螢幕桌面漫遊、坐視窗邊緣、從螢幕邊緣探頭、躲到其他視窗後未做（角色限於自身遊玩場視窗）；
   對其他視窗開關／移動、下載完成、測試失敗無感知來源。
9. **Fullscreen／OS 勿擾偵測未做**（Tauri 現有 API 只看得到自己視窗；不加新平台依賴）；只有使用者層級勿擾開關。
10. **姿勢過渡**：`poseBlend` 只插值頭部中心與身體高度（lie↔stand/sit/crouch）；身體形狀仍在中點切換（stand↔sit 約 10 px）。
11. **拖曳落地的速度**是整段拖曳位移÷時間的下界估算（原生視窗拖曳期間沒有指標事件）。
12. **配對期 DoS**：任何區網 peer 可用錯誤 pair-response 燒掉人類的 5 分鐘配對期（刻意的一次錯就作廢設計）；
    已加 audit `mobile.pair-burned-by-peer` 與 status `pairingBurnedAt`，UI 尚未顯示提示。
12a. **estop 也會關掉 iPhone 的低風險感測（電池）**：比不變量要求更嚴；解除 estop 後需在手機重新開啟；`emergency-stop --clear`
    後手機端保持關閉的路徑未在模擬器實測。
13. **桌面端 BLE gateway 只有 scan**；iOS 端 connect/gatt(read/write/subscribe) 已寫但桌面沒有送端；訂閱串流語意與 one-shot 不相容。
14. **Camera／Location／Live Activity／Audio SFX／區網裝置事件** receptor/actuator 未實作。
15. **WebSocket／HID／Home Assistant adapter**（規格 §9.1 第 4–6 項）未實作。
16. **效能數字**是 headless Chromium 的 CPU 數字，非 Tauri WKWebView 實機；輸入延遲只量 WebView 內段（Rust 點擊穿透閘＋OS 派送
    未量，**端到端未量**）；heap 已是精確位元組（`--enable-precise-memory-info`）但只有 60 s 浸泡，GC 後 +231 KB 未判定是快取暖身還是洩漏。
17. **磁碟**：本機 `target/` 約 30 GB，Phase 7 期間兩度寫滿導致 build 中斷（刪除 `target/debug/incremental` 恢復）。
18. 本輪未 push、release、deploy 或開 PR（依 repo 規則需使用者明確授權）；Phase 6＋7 已提交為 `2e02284`，
    Phase 8 為 `d03e0b9`／`521c232`。

---

## v0.5 Phase 8（Character Presentation Protocol＋小樞 Reference Adapter＋一般模式產品化；2026-09-02，macOS 26.2／Apple M2 Pro／rustc 1.94.0 (4a4ef493e 2026-03-02) (Homebrew)／node v24.5.0／pnpm 10.27.0）

> 每個數字都是本機實跑；模擬器／fixture／瀏覽器版控制中心一律標示。**沒有任何 Tauri 角色視窗、可信 overlay 視窗、匯入資料夾、
> ESP32 或 iPhone 真機的證據**——這些只有單元／模擬器／fixture 證據。本節已提交（`d03e0b9`、`521c232`），
> 未 push、未 release。

### 對抗審查（Phase 8；`.claude/workflows/adversarial-review-v05.js`，報告落盤於 `docs/reviews/adversarial/`）

| run | 範圍 | reviewed | confirmed | fixed-meanwhile | refuted | unverified（verifier 失敗） |
|---|---|---:|---:|---:|---:|---:|
| 1 `2e02284-20260902T080415Z` | 13 維度 find→verify | 110 | 32 | 0 | 3 | 75（模型額度上限） |
| 2 `2e02284-20260902T140445Z` | run 1 的 75 項 unverified 以 seeds 重驗 | 75 | 25 | 1 | 2 | 47（額度用盡） |
| 3 `2e02284-20260902T142608Z` | run 2 的 47 項再重驗 | 47 | 44 | 1 | 2 | 0 |

run 3 的 44 項 confirmed **尚未修復**（本次交付先如實記錄：清單在 run 3 報告，嚴重度分布見下；後續回合處理）。run 1 的 32 項 confirmed 全部修復（5 組：Rust 誠實階梯、頁面、角色視窗、線協定／韌體、效能與 host），每項附回歸測試並做過
「暫時把 bug 放回去 → 測試變紅」的檢查；run 2 的 25 項 confirmed 由角色視窗／rig 修復回合處理，未能修的以已知限制記錄
（見 CHANGELOG「已知限制（Phase 8）」）。

### 回歸實測（Phase 8 收尾）

| 套件 | 命令 | 結果 | 環境／限制 |
|---|---|---|---|
| Rust fmt | `cargo fmt --all --check` | 通過（exit 0） | rustc 1.94.0（`rust-toolchain.toml` 釘 1.94.0；本機 Homebrew rust 無 rustup） |
| Rust clippy | `cargo clippy --workspace --all-targets -- -D warnings` | 通過，0 warnings | 含新 crate `interaction-character`＋axum `ws` |
| Rust tests | `cargo test --workspace` | **595 passed / 0 failed / 0 ignored**（42 個 test target；含 interaction-character 101、character_loop 13、api_e2e 22（含 WS fixture）、golden 6、對抗審查回歸 11） | 真 runtime、fake agent 子程序（`fake_claude.sh`／`fake_codex.sh`／`fake_codex_exec.sh`）、in-process WS client（fixture） |
| Tauri backend | `cargo test --manifest-path apps/interaction-desktop/src-tauri/Cargo.toml` | **43 passed / 0 failed**（clippy `-D warnings` 乾淨） | 單元（host_safety／character_store／window adjust／prefs／companion_set_visible） |
| 前端 typecheck | `pnpm typecheck` | 通過 | — |
| 前端 unit | `pnpm test`（vitest） | **759 passed / 0 failed / 0 skipped（39 檔）** | jsdom＋stub canvas（模擬器） |
| 前端 build | `pnpm build` | 成功（vite 1.5 s） | — |
| CLI E2E | `./scripts/v03-cli-e2e.sh` | **82 passed / 0 failed**（含「Character Protocol」段 14 項：adapter token 分權、WS 握手、手動安全 intent 拒絕、receipt、撤銷；ESP32 模擬器段新增按鈕翻轉推播／感測 null／訊息上限） | 真 daemon 18811＋mock HTTP 裝置＋serial 模擬器（含控制通道）＋**模擬 adapter（fixture，Node 24 走 WebSocket）** |
| Playwright | `pnpm test:e2e` | **35 passed / 0 failed**（app 15、evidence 15、narrow 4、offline 1；57.8 s） | 真 daemon 18790（fake agent 子程序 fixture）＋Chromium；瀏覽器版控制中心，**非 Tauri 角色視窗** |
| 效能 | `pnpm perf` | 見下方「角色效能量測」表（drawRig／全舞台／stage loop／輸入延遲／heap soak／bounded／3 天數值） | headless Chromium，非 WKWebView |


### 證據跑（Playwright＋perf；瀏覽器版控制中心，非 Tauri 角色視窗）

`pnpm test:e2e`：global-setup 建真 daemon（18790）並以 `fake_claude.sh`／`fake_codex.sh` 作為 agent 子程序 fixture；
evidence spec 在 1200×800（`desktop-*`）與 390×844（`narrow-*`）各擷取一次，全部落在 `docs/assets/v05-evidence/`（本輪重產 56 張、新增 30 張）。
（v0.6.x 起 `e2e/evidence.spec.ts` 改寫到 `docs/assets/v06-evidence/`：M3 之後角色頁 IA 不同，v05-evidence 保留為 v0.5.x 當時的證據，不再被覆寫。）
**每一張都是 Playwright Chromium 對瀏覽器版控制中心的截圖**——沒有 Tauri 角色視窗、沒有 overlay 視窗、沒有真硬體、沒有真機。

| 截圖（desktop-／narrow-） | 內容 | 真實 vs fixture |
|---|---|---|
| `home`、`narrow-home` | 現在：三個回答＋快速操作 | 真 daemon 狀態；沒有角色視窗 → 誠實顯示「角色離線，改用文字」 |
| `companion`、`companion-appearance`、`companion-companionship`、`companion-capabilities`、`companion-library` | 角色頁五區、能力摘要、內建角色清單（預設小樞） | manifest／registry 真實轉述；玩耍設定為桌面版專屬（瀏覽器誠實說明） |
| `companion-import` | 匯入角色對話框 | UI；瀏覽器版無 Tauri 匯入命令 |
| `companion-fallback`、`narrow-companion-fallback` | 角色載入失敗 → 中立「角色」＋改用文字 | 真實中斷 `/characters/index.json` 請求 |
| `work`、`work-preview` | 工作空狀態（task-first composer）、開始前預覽六項 | UI |
| `work-working`、`work-consent`、`work-claimed`、`work-verified` | 處理中／等你同意／Agent 說已完成／已確認完成 | **fixture agent**（fake_claude／fake_codex 子程序，真 daemon）；verified 由人類 token `POST /verify` |
| `connect`、`connect-adapters`、`connect-adapters-hub` | 連接與權限四區＋角色 adapter 詳細資料 | 外部 adapter 是 `examples/character-adapters/text-adapter.mjs`（**模擬 adapter，fixture，真 WebSocket 連線**） |
| `waiting-*`、`inbox`、`narrow-inbox` | 各頁真實待確認狀態＋通知中心 | fixture agent 產生的 waiting-consent |
| `loading-*`、`error-unknown-*` | 載入中／傳輸錯誤 | Playwright route 延遲／中斷 |
| `theme-light-home`、`theme-dark-home` | 淺色／深色主題 | Runtime UI 偏好 |
| `first-success`、`narrow-first-success`、`narrow-first-success-browser-honest` | 首次成功體驗（精靈 → FirstSuccess） | 瀏覽器版「先在桌面陪我」誠實說明需桌面版 |
| `offline`、`narrow-offline` | Runtime 離線 | 真實離線畫面 |
| `emergency-*` | 緊急停止（放最後，真觸發→擷取→安全流程解除） | 真 daemon estop |
| `global-search`、`hardware-scan`、`more`、`narrow-more` | ⌘K、硬體掃描（metadata only）、更多 | 真 daemon |

### 角色效能量測（`pnpm perf`；headless Chromium，Apple M2 Pro；非 WKWebView）

| 量測（`pnpm perf` 原始輸出行） |
|---|
| `drawRig      : median 0.110 ms / p95 0.150 ms / max 1.670 ms (n=72)` |
| `stage frame  : median 0.260 ms / p95 0.560 ms / max 0.600 ms (n=120)` |
| `rAF gap      : median 8.300 ms / p95 9.100 ms / max 9.400 ms (n=600)` |
| `stage loop   : ticks=362 drawn=362 skipEveryOther=false lastWindowAvgCost=0.247 ms (rAF gap median 8.300 ms / p95 9.200 ms / max 9.400 ms (n=360))` |
| `toy grab lat : median 8.200 ms / p95 8.500 ms / max 8.500 ms (n=20) (confirmed 20/20; WebView-only segment, host click-through gate not included)` |
| `gaze latency : median 8.300 ms / p95 9.100 ms / max 9.100 ms (n=20) (confirmed 20/20; WebView-only segment)` |
| `heap (600 f) : 3.01 MB → 3.51 MB → 2.26 MB after gc (gc available; source usedJSHeapSize, --enable-precise-memory-info, quantized=no)` |
| `heap soak    : 60.0 s / 7199 frames / 29 toy grabs; after-gc 1.92 MB → 2.14 MB (Δ +225 KB, 11.4%); peak before gc 3.61 MB; samples every 5 s: 3.57, 2.80, 3.61, 2.52, 2.51, 3.11, 3.19, 3.48, 3.33, 3.59, 3.39, 3.55 MB; quantized=no; evidence-grade=yes (≥60 s)` |
| `bounded toys : cap=4 of 23 spawns` |
| `3-day run    : finite=true withinClamp=true` |

數字由 `apps/interaction-desktop/scripts/shu/perf-rig.mjs` 產生（`--expose-gc --enable-precise-memory-info`）；「輸入→下一幀」只量 WebView 內段，端到端未量（見已知限制）。

---

## v0.5 Phase 9（發布硬化＋兩輪對抗審查修復；2026-09-03，macOS 26.2／Apple M2 Pro／rustc 1.94.0／node 24.5.0／pnpm 10.27.0）

> 每個數字都是本機實跑或工程 agent 的第一手記錄；模擬器／fixture／程序內 client／真機一律標示。本節已提交
> Phase 8（`d03e0b9`、`521c232`）之後的工作樹變更；**本輪未 push、未 release、未 deploy、未開 PR、未
> commit**（依 repo 規則需使用者明確授權）。完整測試矩陣見 `docs/releases/v0.5.0-test-matrix.md`；已知限制
> 完整清單見 `docs/releases/v0.5.0-known-limitations.md`；收尾狀態見 `docs/v05-capability-gap-matrix.md`
> §11（第一輪）／§12（第二輪）。

### 對抗審查（兩輪；`.claude/workflows/adversarial-review-v05.js` 與 `adversarial-review-adaptive-interaction.js`）

| run | 範圍 | reviewed | confirmed | fixed／partial | refuted |
|---|---|---:|---:|---:|---:|
| 第一輪 `2e02284-20260902T142608Z` | 13 維度 find→verify，run 2 的 47 項再重驗 | 47 | 44 | 43 fixed／1 partial（1 already-fixed：`docs-claims-026`） | 2 |
| 第二輪 `c3d1786-20260903T124638Z` | 對第一輪收尾 commit `521c232` 全面重跑；find＝opus、verify＝sonnet | 78 | 74 | 63 fixed／4 partially-fixed／7 docs-claims 修在文件本身 | 4 |

第二輪 4 項 partial（根因已定位、範圍已明確劃定，非未處理）：`safety-invariants-078`（SSE 半邊已對齊，
`interrupt` 端點的擁有權比對未修）、`companion-gameplay-032`（舞台死區已消除，Tauri 單一 hit-rect IPC 未
拆分）、`protocol-conformance-030`（host 端已誠實標示配對碼未比對，`providers.rs` 尚未依此降級 evidence
level）、`link-transports-054`（serial pty/file fallback 讀取執行緒洩漏已計數，根因未消除）。7 項
docs-claims（文件與程式碼不符，非程式碼缺陷）：三項第一輪已知限制其實在 HEAD 已修好（`F-043`、
`credential_warnings()`、`mobile_ble_scan` deviceId）、桌面/iOS stop-all reason 與 emergency 角色狀態誤留
限制、測試矩陣 TBD-FINAL 過期、iOS XCTest 計數（21→25）、Playwright／`fake_iphone.rs` 未記錄、README／
FEATURES 精靈與導覽文案綁死「小樞」、FEATURES 指向不存在的 CHANGELOG `[Unreleased]`。workflow 的 persist
步驟因 API 529 失敗，本節與 `docs/releases/v0.5.0-*.md` 的第二輪內容由 integrator 依 workflow 執行結果
人工落盤。

修復分 7 個 AREA（memory-ui／mobile-server＋provider 生命週期／agent-honesty＋SSE／ia-settings＋前端 IA／
角色 rig＋perf＋Director／Character Presentation Protocol／link-transports＋協定＋韌體），**每項修復皆附
regression test 與「把 bug 放回去→測試變紅→還原」的實測記錄**（各工程 agent 的 stage-3 報告
`testCommandsRun`／`findingsAddressed[].regressionTest`）。逐項 summary 見 `CHANGELOG.md`「對抗審查第二輪」
小節。

### 回歸實測（第二輪修復收尾，最終一次全套；2026-09-03，所有 commit 就緒後同一台機器實跑）

| 套件 | 命令 | 結果 | 環境／限制 |
|---|---|---|---|
| Rust fmt／clippy／workspace tests | `cargo fmt --check`／`cargo clippy --workspace --all-targets -- -D warnings`／`cargo test --workspace` | **fmt exit 0／clippy 0 warning／736 passed / 0 failed / 0 ignored（63 個 test target）**；`cargo build --workspace` 成功 | 真 runtime、fake agent 子程序、程序內【模擬 iPhone】、內嵌 rumqttd broker |
| Tauri backend | `cargo test --manifest-path apps/interaction-desktop/src-tauri/Cargo.toml`＋clippy | **46 passed / 0 failed**；clippy 0 warning | 單元測試 |
| 前端 typecheck | `pnpm typecheck` | **通過**（`tsc --noEmit` 乾淨） | — |
| 前端 unit | `pnpm test`（vitest） | **988 passed / 0 failed（49 檔）** | jsdom＋stub canvas（模擬器） |
| 前端 build | `pnpm build` | **成功** | — |
| CLI E2E | `./scripts/v03-cli-e2e.sh` | **82 passed / 0 failed** | 真 daemon＋mock HTTP／serial 裝置＋模擬 adapter fixture |
| Playwright | `pnpm test:e2e` | **65 passed / 0 failed（12 spec，1.9 分）** | 真 daemon＋Chromium，瀏覽器版控制中心；iPhone 相關 spec 對接【模擬 iPhone（fixture）】 |
| ESP32 韌體 | `./firmware/esp32-companion/compile.sh [--ble]` | **兩組態 exit 0（2026-09-03，最終覆核跑）** | arduino-cli 1.5.1、esp32:esp32 3.3.11（非真板） |
| iOS 模擬器 XCTest | simctl 注入 `.xctest` | **25 passed / 0 failed**（MotionClassifier 8＋ProtocolTests 17） | 模擬器（iPhone 17，`docs-claims-070`訂正） |
| iPhone 真機 | `device-build.sh`／`device-acceptance.sh --grant-consent` | **部分驗收，見下方「iPhone 真機（2026-09-03）」** | 真機（iPhone 11／iOS 26.3.1），非模擬器 |

### Playwright user-task 套件與模擬 iPhone fixture（`docs-claims-071`）

第二輪修復期間新增 8 個 Playwright user-task spec（`a11y`／`agent-not-installed`／`character`／`estop`／
`home-state`／`iphone`／`sensors`／`work-delegate`），加上既有 `app`／`evidence`／`narrow`／`offline`，
`apps/interaction-desktop/e2e/` 現有 **12 個 spec、65 個 `test(`**；`playwright.config.ts` 三個有序
project：`first-run`（`app.spec.ts`）→`main`（其餘）→`estop-last`（`estop.spec.ts`，破壞性列放最後）。
新增 `crates/interaction-runtime/examples/fake_iphone.rs`：程序外**【模擬 iPhone（fixture）】**，讓
Playwright 能在真 daemon 上重現手機連線／斷線／權限拒絕／停止感測未回應等狀態；`iphone.spec.ts` 的 4 個
測試標題全部明寫「iPhone（模擬 fixture）」——**這不是真機驗收**，真機證據只在
`v0.5.0-iphone-device-evidence.md`。

### 角色效能量測（可重現：`pnpm perf`；headless Chromium，Apple M2 Pro；2026-09-03 第二輪修復收尾最終跑，
非 Tauri WKWebView）

> `perf-claims-007/008/009` 修復後的量測：Reduced Motion 加入真靜態短路、30fps 降級改用顯示器自身節奏的
> pacing 基準線、soak 涵蓋範圍擴大到 `CharacterGateway`／`InteractionDirector`／behavior／記憶／事件環。

| 指標 | 結果 |
|---|---|
| drawRig 單角色一幀 | median **0.100 ms**／p95 0.130／max 1.270（n=72） |
| 全舞台一幀（stage frame） | median **0.220 ms**／p95 0.500／max 0.520（n=120） |
| rAF 間隔（headless 節奏） | median **8.3 ms**／p95 9.0／max 9.4（n=600） |
| WebView 內段：抓玩具（toy grab） | median **8.4 ms**／p95 9.3（n=20；只是 WebView 段，不含主機端點擊穿透閘） |
| WebView 內段：看向游標（gaze） | median **8.3 ms**／p95 8.5 |
| JS heap（600 幀前／後／GC 後） | 3.02 → 3.93 → 2.26 MB |
| heap 浸泡（60 s） | after-GC **1.92 → 2.14 MB（Δ +223 KB）** |
| **heap 浸泡（10 分鐘，`PERF_SOAK_MS=600000`）** | **600.0 s／72,008 幀／299 次抓玩具，after-GC 1.92 → 2.12 MB（Δ +210 KB，10.7%），GC 前峰值 8.37 MB** |

10 分鐘的 Δ（+210 KB）沒有比 60 秒的 Δ（+223 KB）大，判讀為固定保留集合，不是隨時間累積的洩漏；**30 分鐘
浸泡未執行**。本輪 soak 涵蓋範圍比 Phase 8（僅 `StageRenderer`）更廣，兩次數字不是直接可比的回歸對照。

### iPhone 真機（2026-09-03）

完整逐列輸出見 `docs/releases/v0.5.0-iphone-device-evidence.md`（本文件不重複轉錄每一列，只總結）。裝置：
iPhone 11（`iPhone12,1`），iOS 26.3.1；桌面：macOS 26.2、Xcode 26.6；通訊：區網 Wi-Fi TLS WebSocket（非
loopback），憑證指紋釘選＋HMAC 配對＋每機獨立 token。

**已驗證（真機）**：安裝與啟動、配對、首次連線權限誠實顯示未授權、haptic／notify／tts／torch／flash 動器
acknowledged（deviceApplied 各異，如 haptic 在 iPhone 11 走 UIImpact 降級路徑並誠實回報 engine）、角色六態
acknowledged、AI 偽造 `emergency`／`verified-success` 被 runtime 擋下（receipt failed、從未 dispatched）、
背景／鎖定行為（App 進背景後 daemon 偵測斷線並強制停用高風險受器 `iphone.mic-level`，reason
`disconnected`，不自動恢復）、桌面 IP 變更需重新配對（daemon 端 0 次連線嘗試，見下方限制）、撤銷離線裝置、
觀察 battery／touch／麥克風音量（`activeSensors` 反映一致；`POST /v1/observations/query` 對
`iphone.mic-level` 一律為空是設計——mic-level retention none、不入 SQLite，不是缺陷）、BLE 閘道 scan（≈8 秒
回傳 10+ 個周邊，多數 name 為 null，誠實不編造）、停止所有感測（使用者路徑，313 ms 內確認）、緊急停止
（178 ms 內確認停感測＋角色投影，`stoppedActuators:20`）、解除緊急停止不自動恢復（5 秒後感測仍全 false）。

**未涵蓋（沒有結果就是沒有）**：observe-motion（需使用者在開啟「動作」時搖手機）、BLE connect／GATT
read/write/subscribe（無測試用 peripheral）、系統終止 App 後的冷啟動恢復（實測需按「連線」或
`--auto-connect`）。

**真機測試發現的三個真機限定的限制**（見 `docs/releases/v0.5.0-known-limitations.md`）：桌面 Wi-Fi IP
變更後 App 沒有 Bonjour 探索、host 釘死在配對當下的位址，換 IP 必須重新配對（新配對後 4 秒內連上）；App
進背景會被 iOS 收回 WebSocket（平台限制，非缺陷，行為與不變量一致：斷線後高風險受器不自動恢復）；
`device-acceptance.sh` 原本會在「沒有 active session／iPhone 動器預設 disabled／policy allowlist 未含
`iphone.*`」三道前置關卡誠實回 `session_inactive`／`no-action`／`blocked(actuator.allowlist)`（Governor
正確運作，不是手機問題），新增 `--grant-consent` 後這三道關卡會依序打開，讓完整矩陣可以跑完。

**使用者親眼確認**：`acknowledged` 只代表手機回報已套用；震動、通知橫幅、語音、手電筒、閃屏、角色狀態的
使用者親身確認尚未回填（見 `v0.5.0-iphone-device-evidence.md`「使用者親眼／親身確認」一節）。

### 已知限制（v0.5 Phase 9，誠實聲明；修掉時同步更新 CHANGELOG 與 known-limitations.md）

完整清單見 `docs/releases/v0.5.0-known-limitations.md`（第二輪整合版）。摘要：4 項第二輪 partial
（`safety-invariants-078`／`companion-gameplay-032`／`protocol-conformance-030`／`link-transports-054`）；
`ia-settings-012` 精靈半邊未修（第二輪覆核仍為真）；ESP32 真板未驗收（僅編譯＋模擬器對測）；iPhone
observe-motion／BLE connect-GATT／冷啟動恢復未涵蓋（見上）；`rig-renderer-056` 最差單幀跳動 4.48 px（非
0）；`memory-ui-003` 匯出仍只涵蓋記憶項目；`safety-invariants-075`「只這一次」是 5 分鐘 TTL 非真正單次；
`agent-honesty-022` resume workdir 未持久化；`ia-settings-018` 精靈 commit 非原子；外部角色 adapter 輸入
已完全移除（比原本更嚴格）；`interaction-api` WebSocket 限流測試在機器負載高時會 flake。上一輪（v0.5
Phase 7／8）已知限制清單見上方對應章節，第二輪已修復項目不再重複列出。

# v0.5.1（產品完成度、一般模式易用性、誠實狀態與剩餘技術債；2026-09-04，macOS 26.2／Apple M2 Pro／rustc 1.94.0／node 24.5.0／pnpm 10.27.0）

> 每個數字都是本機實跑。證據等級一律標示：真機／模擬器／fixture／browser（Playwright）／真 Tauri 視窗／單元。
> 基準是 v0.5.0（tag `v0.5.0` ＝ `8b713c7`），在同一台機器重跑一次當對照。完整矩陣見
> `docs/releases/v0.5.1-test-matrix.md`；已知限制見 `docs/releases/v0.5.1-known-limitations.md`；
> 20 道發布關卡見 `docs/releases/v0.5.1-release-readiness.md`。
> **已發布**：PR #2 → CI run 33807755834 全綠 → ff-merge → main CI run 33808297245 → `release.sh 0.5.1` → tag `v0.5.1` → Release run 33808841552（18 個資產，4 個 checksum 比對相符）。
> 對抗審查 `0c845e0-20260903T185130Z`：55 confirmed → 已修 52／部分修 3（見下方〈對抗審查〉）。

## 基線（v0.5.0 tag 於同一台機器重跑）

| 套件 | 結果 | 證據等級 |
|---|---|---|
| `cargo fmt --check`／`cargo clippy --workspace --all-targets -- -D warnings` | exit 0／0 warning | — |
| `cargo test --workspace` | **736 passed / 0 failed / 0 ignored（63 個 test target）** | 單元＋真 runtime＋fixture |
| Tauri backend `cargo test`＋clippy | **46 passed / 0 failed**；clippy exit 0 | 單元 |
| `pnpm test`（vitest） | **988 passed / 0 failed（49 檔）** | 單元（jsdom，模擬器） |
| `pnpm build` | 成功 | — |
| `./scripts/v03-cli-e2e.sh` | **82 passed / 0 failed** | 真 daemon＋mock 裝置＋fixture |
| `compile.sh`／`--ble` | 兩組態 exit 0 | fixture（arduino-cli，非真板） |
| Playwright | **未執行**（基線階段未跑） | — |

## 回歸實測（v0.5.1 分支，30 個修復 commit 全部就緒後的最後一次全套；對象 `957332e`，2026-09-04 05:05–05:14）

| 套件 | 命令 | 結果 | 證據等級 |
|---|---|---|---|
| Rust fmt／clippy | `cargo fmt --check`／`cargo clippy --workspace --all-targets -- -D warnings` | exit 0／**0 warning** | — |
| Rust workspace tests | `cargo test --workspace` | **827 passed / 0 failed / 0 ignored（66 個 test target）**；基線 736（63 target）→ **+91／+3 target**；`cargo build --workspace` 成功 | 單元＋真 runtime＋fixture |
| CLI E2E | `./scripts/v03-cli-e2e.sh` | **82 passed / 0 failed** | 真 daemon＋mock HTTP／serial 模擬器＋模擬 adapter fixture |
| Tauri backend | `cargo test --manifest-path apps/interaction-desktop/src-tauri/Cargo.toml` | **50 passed / 0 failed**（基線 46 → +4）；clippy 乾淨 | 單元 |
| 前端 typecheck | `pnpm typecheck` | 乾淨 | — |
| 前端 vitest | `pnpm test` | **1168 passed / 0 failed（60 檔）**；基線 988（49 檔）→ **+180 測試、+11 檔**。最初的全量跑有 2 個案例只在全量跑失敗、單獨跑通過（`regressions-v05.test.tsx` 的兩條一般模式術語斷言），根因是 `src/characterName.ts` 的刷新沒有世代概念——測試輔助 `resetCharacterNameForTests`／`primeCharacterNameForTests` 之後遲到的舊刷新會把已解析的「小樞」蓋回中立的「角色」並更新節流時間戳（**正式執行期沒有人換世代，行為不變**）；修法是刷新帶世代編號、reset／prime 換代、舊世代落地即作廢（commit `a6e289e`，新增 3 個 characterName regression test）。修後全量連跑 **6 次全綠**，typecheck 乾淨 | 單元（jsdom，模擬器） |
| 前端 build | `pnpm build` | 成功 | — |
| Playwright | `pnpm test:e2e` | **65 passed / 0 failed（2.0 分）** | browser（Chromium＋真 daemon；iPhone 相關 spec 對接**模擬 iPhone fixture**） |
| WS 限流穩定度 | 連續 20 次跑限流測試 | **pass=20 fail=0**（改以 `CharacterHub::set_clock` 注入假時鐘；限流演算法本身未改） | 單元 |
| ESP32 韌體 | `./firmware/esp32-companion/compile.sh`／`--ble` | 兩組態 **EXIT=0**；程式 939 379 bytes（71%）／1 190 215 bytes（90%），全域變數 49 924（15%）／61 064（18%） | fixture（arduino-cli，非真板） |
| iOS typecheck／build | `xcrun swiftc -typecheck`／`xcodebuild` | **0 error（EXIT=0）**／**BUILD SUCCEEDED（EXIT=0）** | 單元／模擬器 |
| iOS XCTest | simctl 注入（iPhone 17 模擬器） | **Executed 46 tests, 0 failures** ＝ MotionClassifier 8＋ProtocolTests 17＋**ReconnectHintTests 21（本輪新增）**；v0.5.0 為 25 → +21 | 模擬器 |

## 真 Tauri 視窗驗收（本輪新增的證據等級）

組態：`pnpm tauri build --debug --bundles app` 產出的 `.app`（debug；前端 dist 為分支 commit `8ba2a51`
時的內容；Rust host 含 `companion_hit_regions`），隔離 home（`INTERACT_AI_HOME` 指向暫存目錄），內嵌
Runtime 綁 `127.0.0.1:8787`；agent 二進位指向 `tests/fixtures/fake_claude.sh`／`fake_codex.sh`
（**fixture，非真 Claude Code／Codex**）；`INTERACT_AI_MOBILE_ADVERTISE=0`（只綁 127.0.0.1，`lsof`
確認）。驅動：macOS System Events（AX）＋ Core Graphics 真滑鼠事件（`CGEventPost`；AX 的 `click at`
會繞過 ignore-cursor-events，不能用來測穿透）＋ `screencapture -R` 只截自己的視窗區域。

| 項目 | 做法 | 結果 | 等級 |
|---|---|---|---|
| 主視窗啟動 | 執行 .app 內二進位 | `/ready` 10 秒內回 `{"emergencyStop":false,"status":"ok"}`；System Events 列出「Interaction Control Center」（1280×840）與「小樞」（520×284） | 真 Tauri 視窗 |
| Tray（狀態列選單） | System Events 讀 `menu bar 2` | 11 個項目：系統狀態：正常（內嵌 Runtime）／主動互動：進行中／AI 工作階段：0／開啟控制中心／隱藏桌面角色／暫停主動互動／暫停一小時／停止所有感測／緊急停止／設定…／完全結束 | 真 Tauri 視窗 |
| 首次設定精靈 | AX 點「下一步」×2 →「完成設定」→ 套用前確認對話框 →「套用」→ 首次成功畫面 →「稍後再說」 | 按鈕與文案如 DESKTOP-GUIDE 所述；`GET /v1/onboarding` 由 `completed:false` 變 `true` | 真 Tauri 視窗 |
| 原生資料夾選擇器（取消） | 工作頁點「選擇資料夾…」→ NSOpenPanel（AXSheet）→ Escape | sheet 消失；欄位仍為空；無「打不開資料夾選擇器」錯誤 | 真 Tauri 視窗＋原生對話框 |
| 原生資料夾選擇器（選擇） | 再開 → ⌘⇧G 輸入暫存目錄路徑 → Return ×2 | 欄位顯示實際路徑；預覽「你選擇的資料夾（pick-me）」「不會修改：這次只看不改」 | 真 Tauri 視窗＋原生對話框 |
| Read-only 不取得 write scope／Write 需額外確認 | 勾「允許修改這個資料夾裡的檔案」 | 預覽變「會修改：只限 &lt;完整路徑&gt;——還需要你確認一次」，出現第二個核取（工作結束、30 分鐘到期、關閉或緊急停止即失效）；不勾就無法以寫入模式開始 | 真 Tauri 視窗 |
| 唯讀工作建立與 scope | 取消寫入、填任務、點「開始」（fixture agent） | 畫面：「「…」已送到 Claude Code 手上，尚未完成；做完後會請你檢查結果。」；API `allowWrite:false`、`dataScope:["workspace:<選定資料夾>"]`、`toolScope:[]`、`resolvedWorkdir` 為正規化後同一路徑（未擴大） | 真 Tauri 視窗＋fixture agent |
| claimed ≠ verified | 讀工作卡片 | 顯示「對方說已完成，尚未經過檢查」＋「標記為已驗證（我確認過結果）」按鈕，**沒有綠勾** | 真 Tauri 視窗 |
| Session 結束後撤銷 | `POST /v1/agent-sessions/{id}/close` | `state:closed`、`lease.revokeOnSessionEnd:true`；卡片改顯示「已取消」 | 真 Tauri 視窗＋API |
| 角色視窗顯示／隱藏 | Tray「隱藏／顯示桌面角色」 | 視窗自 System Events 清單消失又出現；`status.presentation.visible` false→true；選單文字互換 | 真 Tauri 視窗 |
| 遮蔽觀察（設計限制記錄） | 角色視窗落在主視窗下方 | WebKit 因遮蔽暫停繪製，Runtime 每 ~21 秒收到重新 hello（generation +2）；移出遮蔽後恢復。屬正常 macOS 行為，非缺陷，但值得註明 | 觀察（真 Tauri 視窗） |
| Click-through（空白穿透） | 角色視窗疊在主視窗上使空白區蓋住側欄「更多」，CGEvent 真滑鼠點該點 | 底下主視窗切到「更多」分頁——**點擊穿透了空白區** | 真 Tauri 視窗＋真滑鼠事件 |
| Click-through（角色本體攔截） | CGEvent 點角色本體 | 角色換成被戳的表情；底下主視窗未被觸發 | 真 Tauri 視窗＋真滑鼠事件 |
| Trusted overlay＋Emergency Stop | Tray「緊急停止」 | `/ready` `emergencyStop:true`；「安全狀態」視窗出現（1376,45，340×200）顯示「緊急停止中」；角色視窗顯示固定文字；Tray 同步。`POST /v1/emergency-stop/clear` 後覆蓋視窗與角色文字消失 | 真 Tauri 視窗 |
| 感測指示 | 配對【模擬 iPhone（fixture）】`examples/fake_iphone`，啟用 `iphone.mic-level`＋session consent | `status.activeSensors` 出現 `iphone.mic-level active`；覆蓋視窗顯示「麥克風使用中 iphone:…」；Tray「系統狀態：正常（內嵌 Runtime）｜麥克風使用中」 | 真 Tauri 視窗＋fixture 手機 |
| 停止所有感測 | Tray「停止所有感測」 | `activeSensors` 清空、覆蓋視窗消失；fixture 收到 2 則 stop-all（含 auto-ack） | 真 Tauri 視窗＋fixture |
| 角色匯入資料夾 | 角色頁「匯入角色…」→ 原生 Open 對話框 → ⌘⇧G 選暫存目錄的 `manifest.json` →「匯入」 | 對話框列出「驗收文字角色／這個角色沒有宣告任何資產」；匯入後顯示「已匯入「驗收文字角色」」，`<home>/state/characters/accept-text/` 建立 | 真 Tauri 視窗＋原生對話框 |
| Reduced Motion | OS 設定無法自動切換（`defaults write com.apple.universalaccess reduceMotion` 被系統拒絕） | **未在真視窗驗收**：需人類切換「減少動態效果」後觀察角色重新協商。單元＋Playwright（模擬 media query）有覆蓋 | 未驗（需人類） |
| 多角色／玩具（快捷選單） | CGEvent 點角色本體 | 角色有反應（表情），但未捕捉到快捷選單畫面；**未以真視窗驗證丟玩具／使魔** | 未驗（jsdom＋vitest 覆蓋） |
| 完全結束 | Tray「完全結束」 | App 程序退出、127.0.0.1:8787 關閉、`state/runtime.lock` 釋放 | 真 Tauri 視窗 |
| 外部 daemon 模式 | 先起 `interact-ai serve`（隔離 home）再啟動 App | Tray「系統狀態：已連線外部 Runtime」 | 真 Tauri 視窗＋真 daemon |
| Runtime disconnect | kill 外部 daemon | 9 秒內覆蓋視窗出現「Runtime 離線」；Tray「系統狀態：離線（無法連線 Runtime）」；之後「完全結束」仍乾淨退出（0 個殘留程序） | 真 Tauri 視窗＋真 daemon |
| Adapter crash fallback | 無法在真視窗誘發 adapter 崩潰 | **未驗**（vitest `companion-gateway-wiring` 覆蓋固定文案） | 未驗 |
| Dialog 不得從 overlay／角色視窗開啟 | Tauri capability：`main-dialog.json` 只授 `main` 視窗 `dialog:allow-open` | 設定層核對；未在真視窗嘗試從角色視窗呼叫 | 設定核對 |

## iPhone 真機（2026-09-04）：blocked

`xcodebuild` 停在 macOS 鑰匙圈授權對話框（`codesign` 等待私鑰存取授權，約 9 分鐘 build log 零增長），
**App 從未裝上手機、冷啟動測試 0 次執行**、未建立任何配對（`devices=0` 全程未變）。AI 不得代按該對話框
（等同代替使用者授予同意）。Swift 端對真機 `arm64-apple-ios` 目標編譯與連結成功、0 error——**只證明
「編得過、簽不了」，不證明任何執行期行為**。v0.5.1 的冷啟動自動重連與位址變更提示，驗證等級僅
**iPhone 17 模擬器 XCTest（`ReconnectHintTests` 21 條）**，**不得寫成真機已驗收**。
v0.5.0 的 iPhone 真機證據（`docs/releases/v0.5.0-iphone-device-evidence.md`）不受影響、仍然有效。
需要的人工步驟與待補的列見 `docs/releases/v0.5.1-iphone-device-evidence.md`。

## ESP32／BLE 邊界句（固定用字）

> ESP32 firmware compiled and simulator-tested; not validated on a physical ESP32 board.

> BLE implementation compiled and fixture-tested; not validated against a physical BLE peripheral.

## 角色效能量測（可重現：`cd apps/interaction-desktop && pnpm perf`；headless Chromium，Apple M2 Pro；**非 Tauri WKWebView**）

| 指標 | 第一次（60 s） | 第二次（60 s） | 10 分鐘浸泡（`PERF_SOAK_MS=600000`） |
|---|---|---|---|
| drawRig 單角色一幀 | median **0.100 ms**／p95 0.210／max 2.420（n=72） | median 0.100／p95 0.120／max 1.050 | median 0.100／p95 0.130／max 1.340 |
| 全舞台一幀 | median **0.340 ms**／p95 0.520／max 0.540（n=120） | median **0.220 ms**／p95 0.240／max 0.260 | median 0.340／p95 0.620／max 0.660 |
| rAF 間隔 | median 8.300 ms／p95 10.100 | median 8.300／p95 9.600 | median 8.300／p95 9.800 |
| 抓玩具延遲（WebView 段，不含主機端點擊穿透閘） | median **8.3 ms**／p95 9.9（20/20 confirmed） | median 8.3／p95 9.2 | median 8.3／p95 9.3 |
| 看向游標延遲（WebView 段） | median 8.4 ms／p95 9.2 | median 8.3／p95 8.7 | median 8.7／p95 9.8 |
| Reduced Motion 靜態短路 | 361 ticks 只畫 7 幀 | 同上 | 361 ticks 畫 7 幀 |
| heap 浸泡 after-GC Δ | 2.36 → 2.86 MB（**+510 KB**，21.1%），7 200 幀／29 次抓玩具，峰值 5.27 MB | 2.36 → 2.80 MB（**+446 KB**，18.5%），峰值 4.93 MB | 2.41 → 2.92 MB（**+523 KB**，21.2%），71 992 幀／299 次抓玩具，峰值 9.11 MB |

**判讀（觀察項，不是結論）**：10 分鐘的 Δ（+523 KB）沒有比 60 秒（+510／+446 KB）大，方向上像固定
保留集合而非隨時間累積的洩漏；但比 v0.5.0 的同一量測（60 s +223 KB、10 分鐘 +210 KB）高出一倍以上，
**來源未逐項定位**（尚未做 heap snapshot 比對）。不宣稱無害，也不宣稱是洩漏。30 分鐘浸泡未執行。
浸泡涵蓋範圍與 v0.5.0 相同（`StageRenderer`＋`CharacterGateway` 真 shu adapter＋`InteractionDirector`
＋behavior／記憶＋500 筆事件環；不含 React 樹、Tauri IPC、真實 SSE）。有界性：`instances=1`、
`inputQueue=0`、`grants=0`、`decisions=16`、`eventRing=500/500`、玩具 cap=4 of 23 spawns、
3-day run `finite=true withinClamp=true`。

## 對抗審查（`.claude/workflows/adversarial-review-v05.js`）

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

## 已知限制（v0.5.1，誠實聲明；修掉時同步更新 CHANGELOG 與 v0.5.1-known-limitations.md）

完整清單見 `docs/releases/v0.5.1-known-limitations.md`。摘要：v0.5.0 的 28 項重新分類為**已修 13／
部分修 7／保留 9**（其中第 13 項單獨列在「保留」的補充表），加上本輪新增的 **14 項窄限制**——受器與 tool-operation 的
consent 仍是 TTL；首次設定在「檔案已寫、SQLite 未提交」之間崩潰沒有 journal；受器讀取路徑拿不到
`pairingUnverified`；`simulate` 已擋但 `transition_provider` 不同步 registry 旗標（**v0.6.x 分支已修**：停用時翻旗標、重新啟用不自動恢復）；Reduced Motion 未在
真視窗驗收；快捷選單／玩具／使魔與 adapter 崩潰 fallback 未在真視窗驗收；iPhone 冷啟動與位址提示只有
模擬器證據；heap 保留集合 +523 KB 為未定位的觀察項；角色視窗被主視窗遮蔽時 WebKit 暫停繪製造成週期性
re-hello（macOS 行為，非缺陷）；故障注入接縫編進正式碼（inert、只會讓寫入失敗）；legacy agent token
不能再 interrupt（刻意收斂，Breaking）；升級前的 gateway session 不可續開（Breaking）；外部 adapter
輸入維持拒絕；stdio transport 仍未實作。

## v0.6.0 Foundation（開發中；2026-09-05 記錄，HEAD `6683403`，分支 `feature/v0.6.0-foundation`）

> 本節是**純文件任務**的產物：只讀 `rg`／`sed`／`cat` 與
> `scratchpad/{baseline,wave1,wave2,wave3,hardening}/` 下 2026-09-04～09-05 的實跑 log 核實，
> 未另外執行任何 `cargo`／`pnpm`／daemon／Playwright 指令。完整數字與逐項核實見
> [`docs/releases/v0.6.0-test-matrix.md`](releases/v0.6.0-test-matrix.md)；修改前基線見
> [`docs/releases/v0.6.0-baseline.md`](releases/v0.6.0-baseline.md)；九個子系統的 Phase 0 恢復矩陣見
> [`docs/releases/v0.6.0-recovery-matrix.md`](releases/v0.6.0-recovery-matrix.md)。
>
> 下面依任務書 §21 定義的五個完成定義類別（架構／協定／跨裝置同步／保護既有能力／證據）逐條列
> 「證據＝測試名／文件／截圖」或「未達／未驗」。任務書原文不在本 repo 版本控制內，以下條目由
> 實際已提交並經回歸的工作回填五個類別，不引用任務書之外或尚未落地的項目。

### 1. 架構——AIP 成為唯一跨裝置語意契約、小樞脫離協定核心

| 完成定義 | 狀態 | 證據 |
|---|---|---|
| 新增純函式 `interaction-aip` crate（無 tokio／I/O），作為跨裝置語意契約的單一來源 | 達成 | `crates/interaction-aip`；14 個 lib 單元測試＋`tests/conformance.rs` 10 個；`tests/e2e/tests/dependency_boundaries.rs` 釘住不含 tokio／axum／tauri／transport 依賴（`dependency_boundaries` 2 passed，`rust-test.log`） |
| Schema 單一來源：Rust 型別產生 golden schema，TS／Swift 由同一份 schema 產生（禁止手改、CI 擋漂移） | 達成 | `schemas/aip-1.0.schema.json`；`scripts/aip-codegen.mjs`；`pnpm aip:check` 在 wave1／wave2／wave3／hardening 四輪全部 exit 0（`aip-check.log`） |
| `interaction-character`（CPP 核心）不再含任何小樞字串；小樞相關型別／遷移搬到新 crate `interaction-character-shu` | 達成 | `docs/aip/reference-character.md` §5 驗收清單；`rg -n -i 'shu|maid' crates/interaction-character/src` = 0 命中（文件內記錄的核實方式）；`interaction-character-shu` 7 個測試（conformance 1＋rig_pack 6） |
| 桌面 TS 移除 entrypoint if-chain，改用 adapter 註冊表 | 達成 | `apps/interaction-desktop/src/character/adapterRegistry.ts`；`src/test/architecture-no-entrypoint-switch.test.ts`（4 個，讀原始碼鎖住不再有字面分岔）；`src/test/adapter-contract.test.ts`（31 個，四個內建 adapter 共同契約） |
| 第二個 Reference Character（`ref-shape`）加入時，核心三個檔案（`interaction-character/src`、`character.rs`、`CompanionApp.tsx`）不新增任何分岔 | 達成 | `docs/aip/reference-character.md` §4；`src/test/character-ref-shape.test.ts`（9 個） |
| 發布流程拆分為 prepare／verify／tag 三步，verify 是唯讀關卡 | 達成 | `scripts/release-prepare.sh`／`release-verify.sh`／`release-tag.sh`（CHANGELOG wave1 段記錄）；**本節未重新驗證這三支腳本本身的邏輯**（`v0.6.0-recovery-matrix.md` §2.9 已指出 release 相關腳本無專屬自動化測試，這是既有缺口非本輪新增） |
| Runtime 掛載權威 Character Session 並綁定四種 transport（iPhone wire／HTTP／SSE／CLI） | 達成 | `interaction-runtime/tests/character_session_loop.rs`（17 passed）；`docs/aip/transport-bindings.md`；CLI `interact-ai character session status／diagnostics／resume` 子指令（CLI E2E Character Session 段 14 個斷言） |

### 2. 協定——AIP 1.0 十二種 message type、誠實階梯、版本協商、離線政策

| 完成定義 | 狀態 | 證據 |
|---|---|---|
| Envelope＋十二種 message type＋各自必填 profile | 達成 | `docs/aip/README.md` §1–2；`interaction_aip` 14 單元測試 |
| 十二值 Outcome 誠實階梯（`received≠accepted≠applied≠observed≠claimed-completed≠verified`），`verified` 只能由 Runtime 產生 | 達成 | `docs/aip/README.md` §3；`interaction-session/tests/security_matrix.rs`（7 個）；iOS `SessionClientTests::testTheAppCanNeverBuildAVerifiedResult` |
| 確定性版本協商（同 major、min minor）與確定性能力協商（交集＋min） | 達成 | `interaction_aip` 單元測試（版本協商／能力協商子集）；`docs/aip/README.md` §4 |
| 身分綁定：宣稱不符一律拒絕，不「修正後執行」 | 達成 | `docs/aip/README.md` §5；`security_matrix.rs` 的 identity-mismatch 案例 |
| 19 個穩定錯誤碼、有界去重環（256）、有界事件日誌（512）、訊息／payload／字串／深度上限 | 達成 | `docs/aip/README.md` §11–12；`interaction_aip` 單元測試逐一覆蓋上限 |
| 離線事件政策表（drop-if-offline／expire-by-deadline／queue-idempotent／require-reconfirmation／state-reconcile） | 達成 | `docs/aip/README.md` §8；`interaction_aip::offline_policy(name)`；conformance fixture 覆蓋 |
| 三方 conformance（Rust／TS／Swift）共用同一組 golden fixture | 達成 | Rust `conformance.rs` 10；TS `aip-conformance.test.ts` 73（實跑，非靜態 grep 的 11）；Swift `AIPConformanceTests` 14 |
| 未知 message type／name／capability 誠實拒絕，不猜、不執行 | 達成 | CLI E2E「未知 message type 回 error{unsupported-message-type}，不執行」（Character Session 段最後一個斷言） |
| CPP 既有契約不變（AIP 只投影，不改 CPP wire 語意） | 達成 | `docs/aip/README.md` §13 相容對照表；既有 `interaction-character` 測試套件（`gateway.rs` 37、`negotiation.rs` 16、`manifest.rs` 18）維持全綠 |

### 3. 跨裝置同步——iPhone（模擬 fixture／模擬器）⇄ Desktop 的語意事件閉環

| 完成定義 | 狀態 | 證據 |
|---|---|---|
| iPhone 送語意事件（touch）→ Desktop 權威狀態前進、Behavior Intent 回送 | 達成（**模擬 iPhone fixture＋模擬器**，非真機） | CLI E2E Character Session 段（14 斷言）；`character-session.spec.ts` 第一個 test＋`desktop-character-sync-synced.png` |
| Desktop 真相變化（`task.verified`）→ iPhone 收到 celebrate Behavior Intent | 達成（單元／integration 層） | `interaction-session/tests/pure_functions.rs`（Director `task.verified→proud+celebrate` 案例）；`docs/aip/README.md` §13。**沒有端到端 UI 截圖直接誘發並驗證這條路徑**（見 `v0.6.0-test-matrix.md` §6） |
| 斷線→重連→resume（delta patch 優先，超出日誌環才 snapshot fallback） | 達成 | CLI E2E（斷線／重連／resume 兩層）；`perf-session-after.json` 的 `reconnectResumeKind:"patches"`、`reconnectToResumedMs` 5.7 ms；`docs/assets/v06-evidence/desktop-character-sync-reconnecting.png` |
| 撤銷裝置後不自動恢復同步；主動移除全部手機是正常終態 | 達成（v0.6.x 語意） | `character-session.spec.ts` 撤銷段落＋`e2e/general-mode-tasks.spec.ts` 任務 6／7：移除後零裝置＝`local-only`（`desktop-character-sync-local-only.png`／`narrow-character-sync-local-only.png`），撤銷過的裝置再連上來才 `needs-reconfirmation` 並指名是哪一台；v0.6.0 當時的 `*-needs-reconfirmation.png` 是舊語意的證據 |
| 緊急停止中，觸摸被拒且畫面誠實顯示緊急狀態 | 達成 | `character-session.spec.ts` 第三個 test＋`desktop-character-sync-emergency.png`（僅桌面寬度；390px 版本未產出，見已知限制） |
| iOS 原生 App（非 Web）能作為 `remote-renderer` 加入 session | 達成（**iOS 模擬器**） | `apps/interaction-ios/InteractionCompanion/Services/SessionClient.swift`；`SessionClientTests` 28 個（v0.6.0 當時；HEAD 上是 34 個，另有 `ReceiveDecisionConformanceTests`／`ConnectionManagerGateTests`）；`ios-sim-character-session-synced.png`／`ios-sim-character-session-advanced-diagnostics.png` |
| iPhone **真機**上的完整閉環 | **未達（implemented-unverified）** | 零執行；`docs/releases/v0.5.0-iphone-device-evidence.md`／`v0.5.1-iphone-device-evidence.md` 是舊協定路徑的真機證據，**不涵蓋 AIP／Character Session** |
| 第二種裝置（宣告式 Serial）經正式 AIP binding 成為 session 成員 | 達成（**pty 模擬器經 production `DeviceLink`＋serial adapter**，非真板；v0.6.x `e86afb9`＋D2） | `crates/interaction-runtime/tests/declarative_session_loop.rs` **26 測**（v0.6.x 當時是 13：加入／touch 讓 revision 前進／declare＋SensorSource／stop-all 真 ack／靜默＝unknown／撤銷不影響其他成員／拔線 reconnecting→offline／**其他成員的廣播真的經序列線到達**／身分不符與 session-binding 稽核記 transport=serial／diagnostics identityStrength 三來源／撤銷後出站表清空／無通道成員的 no-channel 稽核；v0.7.0 候選再加分片四支、免重啟 rebind、`syncProfile` 推導與 event-source 成員）＋`aip_link.rs` **14 測**＋`esp32_sim_conformance.rs` **24 測**（其中三方一致 2 支）＋`mobile_loop.rs` iPhone 出站登記 1；D1 核心零變更由獨立驗證者以 `git diff --stat` 核對；D1 測試在預設並行下偶發失敗的根因（行程層 `PROVIDER_LINKS` 共用 provider id）已修機具，預設並行 ×5＋×3 全綠。已知：snapshot（1019 bytes）與含 members 的 patch（660–784 bytes）放不進 639 bytes 單行（稽核 `aip.outbound-undeliverable`；v0.7.0 候選的裝置線 v1.2 分片已對**宣告 `aip.frag/1` 的裝置**解除，參考韌體不宣告，照舊送不到）；韌體只編譯檢查，**ESP32 真板為零**；MQTT 有 rebind 閉環（`mqtt_rebind_loop.rs`，程序內 broker），**BLE 未測** |
| 多裝置同時連線同一 session | 部分（**桌面＋Serial 模擬器＋程序內 device fixture**） | `declarative_session_loop.rs::revoking_the_spec_leaves_the_session_without_touching_other_members`：撤銷 Serial spec 後桌面成員與另一個 device party 不受影響；**未**與真 fake_iphone 子程序並存測試 |

### 4. 保護既有能力——v0.5.1 的不變量在重構後仍然成立

| 完成定義 | 狀態 | 證據 |
|---|---|---|
| 全部既有 Rust／Tauri／vitest／CLI E2E／Playwright／iOS 套件通過數只增不減 | 達成 | `v0.6.0-test-matrix.md` §2：Rust 827→985、Tauri 50→54、vitest 1168→1366、CLI E2E 82→96、Playwright 65→71、iOS 46→92，四個 wave 逐輪 0 failed |
| 一般模式主入口維持五個，角色同步不是第六個入口 | 達成 | `src/test/regressions-v06-general-mode.test.tsx`（10 個，`SIMPLE_NAV` 長度與 id 鎖死）；`docs/aip/general-mode-ux.md` §1 |
| 小樞 rig／sprite／text 三個 adapter 對 20 個 CPP intent 的協商結果不變 | 達成 | `intent_capabilities_golden.rs`（1，golden 測試，未變則 diff 為空） |
| iPhone 線協定 v1 訊息對舊 App 保持相容（未知 `type` 只記錄不執行） | 達成 | `docs/aip/README.md` §9.1；`mobile_loop.rs` 68 passed（基線 64→+4，新增案例而非改動既有案例） |
| 配對指紋、每機 token、revoke 即斷線、estop 傳播 stop-all、感測不靜默 | 達成 | 沿用 v0.5.1 既有測試套件（`mobile_loop.rs`／`estop_parallel.rs` 等），本輪未修改這些不變量的實作 |
| `claimed-completed ≠ verified`；綠勾只在 verified；emergency 文案固定 | 達成 | `security_matrix.rs`；既有 `agents_loop.rs`／`gateway_loop.rs` 全數維持通過 |
| `git diff --check`（尾隨空白／檔尾格式）全程乾淨 | 達成（wave1 一次性例外已修） | wave1 曾因新文件檔尾多一行空白 exit 2，wave2 起全部 exit 0（`v0.6.0-test-matrix.md` §2 wave1 備註） |

### 5. 證據——誠實分級、不誇大、可重現

| 完成定義 | 狀態 | 證據 |
|---|---|---|
| 每個測試數字可追溯到實跑 log，而非靜態 grep 估計 | 達成 | `v0.6.0-test-matrix.md` 全篇引用 `scratchpad/{wave1,wave2,wave3,hardening}/*.log`；文件內明確指出 `aip-conformance.test.ts` 靜態 grep（11）與實跑（73）的落差，採實跑數字 |
| fixture／模擬器與真機分開標示，fixture 一律標「模擬 iPhone（fixture）」 | 達成 | 本節與 `v0.6.0-test-matrix.md` 全篇對 iPhone 相關證據逐項標註「模擬 iPhone（fixture）」或「iOS 模擬器」；未發現任何一處把 fixture／模擬器結果寫成真機 |
| 效能量測前後對照，觀察項不宣稱結論 | 達成 | `v0.6.0-test-matrix.md` §5：daemon／Session／角色渲染三段前後數字；`resumeMs` 5000.4 ms 的量測缺陷誠實記錄為「不採用」而非隱藏 |
| Playwright／CLI E2E 意外中止（記憶體不足）誠實記錄，不隱瞞重跑過程 | 達成 | `v0.6.0-test-matrix.md` §2「Playwright 重跑備註」：第一輪 `regress.sh` 在 CLI E2E／Playwright 附近因系統記憶體不足中止，log 被重跑覆寫，改用 `regress-tail.sh` 以 `--workers=1` 重跑成功 |
| 對抗審查（`.claude/workflows/adversarial-review-v06.js`）執行並處置 | **本節未涵蓋** | workflow 已建立（`3483abf`），執行與 find→verify 結果由並行 agent 群負責，其結果不在本次純文件任務範圍內；若已產出報告，應在本節之後由後續 commit 補上（比照 v0.5.1 節「對抗審查」小節的格式） |
| 已知限制與 CHANGELOG 同步更新 | **部分達成** | CHANGELOG `[Unreleased]` 目前只記錄到 wave1 的落地事實（`336a6b6`），wave2／wave3／hardening 尚未補寫；本節與 `v0.6.0-test-matrix.md` 是本輪能提供的最新事實來源 |

### 已知限制（v0.6.0 Foundation，2026-09-05 誠實聲明；HEAD `6683403`）

沿用並疊加 `v0.6.0-recovery-matrix.md` 盤點出的既有缺口（`superseded` 分支無測試、CLI
`mobile pair/revoke/status` 子指令無自動化測試、`release.sh`／CI／Release workflow 無自動化測試、
Memory 子系統缺 browser／真視窗覆蓋等，詳見該文件 §2／§4），本輪新增的窄限制：

iPhone 真機上的 AIP／Character Session 完整閉環零執行（implemented-unverified，與舊協定路徑的真機
證據不是同一層）；Desktop→iPhone `task.verified→celebrate` 缺端到端 UI 截圖；緊急停止場景只有桌面寬度
截圖、390px 版本缺；多裝置同時連線同一 session 未覆蓋；真 Tauri 視窗（而非 Playwright／jsdom）尚未針對
同步卡與十種狀態文案重新走查一次；角色渲染效能量測仍只在 headless Chromium，非 Tauri WKWebView 或真機；
`perf-session-after.json` 的 `resumeMs` 欄位（5000.4 ms）是量測腳本缺陷，不代表真實 resume 延遲；
真 `codex`／`claude` 二進位對接 AIP／Character Session 仍未跑（沿用既有 fixture agent）；對抗審查
（`adversarial-review-v06.js`）執行結果不在本節範圍內；CHANGELOG 的 v0.6.0 段落只記到 wave1，wave2／
wave3／hardening 的落地事實尚未回填。

#### 對抗審查修復後的已知限制（2026-09-05 收尾；完整清單見 `CHANGELOG.md` `[Unreleased]`）

對抗審查 `6683403-20260904T161327Z`：**80 送審／73 confirmed／已修 68／部分修 5／deferred 0**
（逐條處置在 `docs/releases/v0.6.0-known-limitations.md` §2.1）。修完之後仍然成立的限制，
簡短列在這裡，細節與 finding id 對應以 `CHANGELOG.md` `[Unreleased]` 那一段為準：

- `runtime-boundaries-065`：高風險受器停止路徑**只做到誠實回報**（`stopped=false`＋
  `SensorStopUncertain{no-stop-path}`），沒有做 `SensorSource` port；非 mobile provider 的高風險受器
  仍收不到真正的停止請求。
- `crates/interaction-adapter-declarative`（`0.2.0`）與 `adapters/media`（`0.2.0`）版本仍脫離
  workspace，`release-verify.sh` 以 `⚠ 已知版本漂移` 明列，不當成通過。**v0.6.x 已修**：兩者
  `version.workspace = true`，白名單移除並加回歸（release_provenance.rs＋release-scripts.sh）。
- Linux aarch64 已從支援宣告移除（不再給 404），需從原始碼建置。
- 沒有程式碼簽章／公證／SBOM／build provenance；Release 只有 `.sha256`。
- `release.yml` 的 `ci-gate`／`finalize` 兩個 job **未經真實 tag push 驗證**（證據等級：unit）。
- `apps/desktop` 的 `settingsTransfer.ts` 曾以 `SHU_RIG_PALETTES` 驗證使魔配色——「Runtime／頁面
  不得再引用小樞」這條不變量在前端的最後一個例外；**v0.6.x 分支已修**（驗證綁目標角色的 adapter meta；守門測試
  `architecture-no-entrypoint-switch.test.ts` 擴大到頁面層，`CharacterPreview.tsx`／`CharacterLibrary.tsx` 暫列待收斂棘輪）。
- iPhone 真機仍是 **implemented-unverified**（本輪 iOS 證據全部是 iOS Simulator 與程序內 fixture）。
- AIP frame 共用 iPhone 線協定 v1 的速率窗：burst > 30 msg/s 會觸發**既有 v1 的連線關閉**
  （不是只丟那一則）；session 端每成員 30/s 的 token bucket 在那之前會先回 `rejected{rate-limited}`。
- 桌面同步卡的「iPhone 已連接，能力核對中」（`capability-unknown`）在本輪之後**只剩讀不到／
  形狀不認得時的保守回退**：Runtime 已把協商結果投影成 `members[].unsupportedIntents`
  （`docs/aip/character-session.md` §3），正式路徑上會直接落到「已同步」或「部分能力目前不可用」。

# v0.7.0（候選）本輪證據等級（第二輪；分支 `feature/v0.6.x-maintainability`，起點 `f5b8e2f`，28 個 commit）

> 本節只回答一件事：**每一項落地的東西，證據是哪一級、由哪個測試持有。**
> 數字不在這裡重述——整合全套尚未執行，各步驟當時的實跑數字在
> [`docs/releases/v0.7.0-progress.md`](releases/v0.7.0-progress.md) §4，整合後的數字回填
> [`docs/releases/v0.7.0-final-report.md`](releases/v0.7.0-final-report.md) §7。
>
> **等級用字**（由弱到強，逐列標明，不合併、不美化）：
> `unit`＝cargo test／vitest／XCTest／src-tauri 單元；
> `contract`＝三端共用 fixtures 逐筆對答案（`crates/interaction-aip/tests/fixtures/manifest.json`）；
> `fixture`＝程序內替身（fake_iphone、fixture agent、in-process sensor source、替身 socket）；
> `pty 模擬器`＝`scripts/esp32-serial-sim.py` 走 production `DeviceLink`＋serial adapter；
> `broker 模擬器`＝in-process rumqttd broker＋rumqttc fake device；
> `iOS 模擬器`＝iPhone 17／iOS 26.2 runtime，simctl 注入；
> `browser`＝Playwright（Chromium）對真 daemon；
> `真 daemon`＝真的 `interact-ai serve`（隔離 home、自選埠）但沒有 UI；
> `真 Tauri 視窗`＝實際啟動的 `.app`（debug build）以 macOS AX API 走查；
> **`真機／真板`＝零**（iPhone 真機 0 次、ESP32 真板 0 次，本輪沒有任何一列是這一級）。

| # | 落地的東西 | 最高證據等級 | 持有它的測試／腳本 |
|---|---|---|---|
| 1 | AIP 接收端決策表（規則 0–15、resume 逐則、有界 realign 預算） | contract | `crates/interaction-session/tests/receive_decisions_from_json.rs`（只讀 JSON 的獨立消費者）、`receive_decision_fixtures.rs`（產生器兼驗證器）、`crates/interaction-session/tests/pure_functions.rs` |
| 2 | 同一張表的桌面端 | contract | `apps/interaction-desktop/src/test/receive-decision-fixtures.test.ts`（45 案例逐筆，零跳過）、`session-client.test.ts`（三端零差異棘輪） |
| 3 | 同一張表的 iPhone 端 | contract＋iOS 模擬器 | `apps/interaction-ios/InteractionCompanionTests/ReceiveDecisionConformanceTests.swift` |
| 4 | host 的 `reason:"recovery"` 契約 | unit | `crates/interaction-session/tests/session_hardening.rs`、`crates/interaction-runtime/tests/character_session_loop.rs` |
| 5 | 上限常數（`maxResumePatches` 512／`maxRealignAttempts` 3）不漂移 | contract | `crates/interaction-aip` 的 schema 雙向 gate（`every_limit_constant_is_published_in_the_schema`）＋`pnpm aip:check` |
| 6 | 身分規則「local 已知才比對」 | contract | fixtures `identity-unknown-locally-adopts-incoming`／`identity-known-mismatch-still-rejected`（三端各自消費） |
| 7 | 裝置線 v1.2 分片（切片／重組／crc32／有界／失敗整筆丟棄） | pty 模擬器 | `crates/interaction-adapter-declarative/tests/aip_fragment.rs`、`aip_link.rs`、`crates/interaction-runtime/tests/declarative_session_loop.rs` |
| 8 | 實測 wire bytes（capability／snapshot／patch；serial 639・MQTT 639・BLE 480） | pty 模擬器 | `declarative_session_loop.rs::the_measured_wire_sizes_of_the_session_replies_are_over_the_line_limit`（`-- --nocapture` 逐行印 `MEASURED wire line bytes:`） |
| 9 | 參考韌體兩種組態可編譯（**不是**真板驗收） | 編譯檢查 | `./firmware/esp32-companion/compile.sh`／`--ble` |
| 10 | 成員 `syncProfile` 推導與投影 | pty 模擬器 | `declarative_session_loop.rs`（event-source 案例由 `d391f38` 新增）＋稽核 `aip.member-sync-profile` |
| 11 | 桌面對非 full-state 成員不顯示「已同步」（`partial-sync`） | unit | `apps/interaction-desktop/src/test/statusProjection-session.test.ts`、`character-sync-card.test.tsx`、`regressions-v06-round2-general-mode.test.tsx` |
| 12 | 宣告式裝置免重啟 rebind（八步、有界、握手才收斂） | pty 模擬器 | `declarative_session_loop.rs::reenable_rebinds_without_restart`／`rebind_timeout_is_bounded_and_honest`／`a_removed_declarative_binding_is_never_rebound` |
| 13 | rebind 不是 serial 專屬路徑 | broker 模擬器 | `crates/interaction-runtime/tests/mqtt_rebind_loop.rs::mqtt_reenable_rebinds_without_restart`（**BLE 沒有對應測試**） |
| 14 | rebind 保留人類手動關掉的受器 | pty 模擬器 | `declarative_session_loop.rs::rebind_keeps_human_disabled_receptors_off` |
| 15 | 停用後裝置不得再 join（入站 `aip` 綁定閘門） | pty 模擬器 | `declarative_session_loop.rs`（稽核序列斷言；先紅後綠） |
| 16 | 「未解決停止」不因 TTL 消失、同 id 新來源不誤清、人為解除留痕 | unit | `crates/interaction-runtime/tests/sensors_loop.rs::same_id_new_source_does_not_clear_old_generation_unknown`／`confirmed_stop_from_new_source_clears_unresolved`／`dismiss_unresolved_is_explicit_and_audited` |
| 17 | 兩份未解決停止清單（status 與專屬端點）欄位逐欄相同、人話 `sourceLabel` | unit | `crates/interaction-api/tests/api_e2e.rs`（清單相等斷言）＋`sensors_loop.rs`（label 兩支） |
| 18 | 桌面三處呈現＋二段人為確認、文案不得出現「已停止」 | unit | `apps/interaction-desktop/src/test/unresolvedStops.test.tsx`、tray（`src-tauri` `host_safety` 單元） |
| 19 | 裝置「重新連線中／沒有重新連上」與「重新啟用」按鈕 | unit | `apps/interaction-desktop/src/test/connectPage.test.tsx`、`statusProjection` 的 provider 測試 |
| 20 | 陪伴預設兩段寫入的交易與恢復（含 crash 後補送） | unit | `apps/interaction-desktop/src/test/apply-preset-plan.test.ts`（純函式）、`companion-preset-recovery.test.tsx`（mock Tauri host） |
| 21 | `desktop_prefs_patch` persist 失敗回滾 | unit | `apps/interaction-desktop/src-tauri` 的 `commit_prefs_patch` 測試（可注入 persist 回 `Err`；**不是**真的寫滿磁碟） |
| 22 | 陪伴預設按鈕群可及性、Reduced Motion 下文案不變 | unit | `apps/interaction-desktop/src/test/companion-preset-a11y.test.tsx` |
| 23 | 一般模式 14 個任務、分類漂移防線、量測三欄位 | browser | `apps/interaction-desktop/e2e/general-mode-tasks.spec.ts`＋`e2e/taskMetrics.ts`（結果摘要 `test-results/general-mode-task-outcomes.json`） |
| 24 | 對話框開啟時停止路徑可達、⌘K／通知中心焦點 | browser＋unit | `e2e/a11y.spec.ts`＋對應 vitest |
| 25 | Tauri-only 正向路徑（換角色、陪伴檔位、勿擾、顯示／隱藏） | **真 Tauri 視窗** | `scripts/tauri-ax-walkthrough.sh`（9 步全 completed，連跑兩次；debug `.app`；**不在 CI**） |
| 26 | 停用／重新啟用／撤銷的八步走查 | 真 daemon＋pty 模擬器 | `scripts/drills/provider-disable-reenable.sh` |
| 27 | 移除角色套件後使用者資料仍在（`desktop.json` sha256 不變、稽核不刪、epoch 不跳） | 真 daemon | `scripts/drills/remove-package-keep-user-data.sh` |
| 28 | 只回報事件的受限裝置走 production serial adapter | pty 模擬器 | `examples/adapters/event-source-button.yaml`＋`declarative_session_loop.rs` 的 event-source 測試 |
| 29 | iOS 背景閘門五個接線點 | fixture（替身 socket／替身排程）＋iOS 模擬器 | `apps/interaction-ios/InteractionCompanionTests/ConnectionManagerGateTests.swift`（`URLSessionSocket` 本身**未測**） |
| 30 | 架構邊界可執行檢查（依賴邊界、schema 漂移、entrypoint 分岔、決策表、快照遷移、adapter 生命週期、停止路徑） | unit＋腳本 | `scripts/tests/architecture-checks.sh`（`--docs`／`--ts`／`--rust`；swift 列驗存在但需模擬器故維持 SKIP，**未跑 ≠ 通過**） |
| 31 | 文件誠實度與發布腳本 | 腳本 | `scripts/tests/docs-claims.sh`、`scripts/tests/release-scripts.sh`、`cargo test -p interaction-cli --test release_provenance` |

**本輪沒有拿到的證據（逐項寫明，不留白）**：iPhone 真機 0 次；ESP32 真板 0 次（分片路徑在真板上 0 次，
因為參考韌體刻意不宣告 `aip.frag/1`）；BLE 的 rebind 與 AIP session 端到端 0 次；非開發者受測者 0 人；
陪伴預設的「套用→關掉→重開→補送」沒有在真桌面程式跑過；`pnpm perf` 本輪未重跑；整合全套與對抗審查
尚未執行。詳見 [`docs/releases/v0.7.0-known-limitations.md`](releases/v0.7.0-known-limitations.md)。
