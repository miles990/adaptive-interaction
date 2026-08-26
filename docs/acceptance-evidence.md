# 端到端驗收證據（真 daemon＋真 CLI）— 2026-08-26 11:07

## 情境 A：單受器 → conversation → receipt completed
planId: plan-6de4b473-b427-4d95-802d-79615cf71f6b
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
tool loop: plan=plan-877ccb75-2123-4acf-9edd-708db566809e → execute=completed → verify=completed
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
   2 event: session.started
   2 event: plan.blocked

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
