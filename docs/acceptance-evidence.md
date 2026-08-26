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
