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
