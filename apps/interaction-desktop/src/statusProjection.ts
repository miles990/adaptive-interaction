// 一般模式的狀態投影（Character Presentation Protocol §4.2 truthState／§11
// truth projection 在 UI 側的鏡射）。
//
// 所有頁面（收件匣徽章、AI 工作階段卡片、「現在」摘要、全域搜尋）共用
// 這一份「Runtime 原始 taxonomy 字串 → 人話」對照，而且在型別上窮舉：
// Runtime 多一個狀態而這裡沒有投影，`satisfies Record<WorkState, Projection>`
// 會讓 typecheck 失敗，不會靜默退化成把原始字串印到畫面上。
//
// 誠實階梯：
// - claimed ≠ verified：對方說做完了只是「它的說法」，等待你檢查。
// - unknown 既不是成功也不是失敗，只能說「結果不確定」。
// - 介面不認得的原始值一律投影成「結果不確定」並標 `known: false`；
//   一般模式絕不把原始字串當主要標籤，進階模式才在次要的 muted 行顯示原始值。
// - 這裡只做「翻譯」，不做升級：沒有任何路徑能把 claimed 翻成 verified。
//
// 本檔只是匯總殼：實作依子領域分在 `statusProjection/` 底下（工作狀態／收件匣／
// provider 與感測／角色生命週期／角色 Session 同步），所有既有 import 路徑
// （`from "./statusProjection"`）不變。新增投影請加到對應子領域檔，不要加在這裡。

export * from "./statusProjection/workState";
export * from "./statusProjection/inbox";
export * from "./statusProjection/provider";
export * from "./statusProjection/unresolvedStops";
export * from "./statusProjection/characterLifecycle";
export * from "./statusProjection/characterSync";
