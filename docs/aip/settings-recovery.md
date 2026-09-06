# 陪伴設定：權威值、恢復與備份

## 1. Owner 與 production 路徑

`CompanionPage` 呼叫 `desktop.presetApply` → Tauri `companion_preset_apply` →
`src-tauri/src/preset_service.rs::PresetService`。服務協調 desktop.json 與 Runtime
`proactive.rs`，React 只呈現服務結果與有效值，不執行跨儲存恢復狀態機。
預設的三個受控值由 `src/companion/presetDefinitions.json` 單一來源定義，Rust 與 TS 共用；
它不包含費用限制、指定 AI、consent 或安全上限。

桌面偏好的 owner 是 `supervisor::{load_prefs,save_prefs}`；`desktop_prefs_patch` 是一般設定入口。
Runtime 主動對話 config 的 owner 是 `proactive::configure`；UI 的 `applyPresetPlan.ts` 只保留
預設規劃及投影用純函式，不能當成持久化交易的 authority。

## 2. 版本與有界操作

- `companionPresetRevision` 是 desktop.json 的十進位字串 revision，缺省 `"0"`。
- `configRevision` 是 Runtime 主動對話設定的獨立十進位字串 revision，與 config 在同一 SQLite meta
  記錄保存。它不是 AIP epoch/revision，也不改 Session snapshot format。
- 操作 ID 用 UUID。相同毫秒的兩次操作仍是兩個 ID；相同 ID、相同 patch、仍為當前 revision 的
  Runtime 重送是冪等。相同 ID 改參數、舊 revision 或已被新寫入超越均回 conflict。
- Host gate 對競爭操作立即回 busy，不排無界佇列；一般 prefs patch 與 preset 共用 gate。
  Emergency stop 不等待這個 gate。外部 Runtime 單次 request 最長 5 秒。

## 3. 持久化邊界與恢復

服務先讀 Runtime，再以原子檔案替換保存第一段偏好與 format 1 pending marker，marker 記錄操作 ID、
預設與原 Runtime revision。第二段以條件寫入調整 Runtime；即使寫入回應遺失，也讀回有效值再判斷。
只有三項有效值一致才回 applied 並清 marker。清 marker 失敗保留可重試狀態。

啟動後 backend available 或外部 Runtime 從離線恢復時，host 執行一次恢復。若值已符合目標，
只完成 marker 清理；若 Runtime revision 或使用者相關設定已較新，保留新選擇並轉 custom，不能補償覆寫。
無關偏好以目前記憶體/檔案值保存。未取得有效讀回是 unverified；部分值已寫入但另一段未完成是 partial。
UI 以 host 完成事件失效化舊讀取；開始與完成兩端均排除競態中的舊 prefs/config 回覆。

這是兩個持久化 owner 之間的可恢復操作，**不是跨程序 ACID 交易**。
原子 rename 保護 desktop.json 的單檔提交；不能用 busy 或 generation guard 宣稱跨儲存原子性。

## 4. 舊資料與備份

舊版未標 format 的 pending marker 仍可讀，但沒有 Runtime revision 時不自動重寫不同的有效值；
使用者明確重選預設可建立新操作。損壞／future marker 原始 JSON 保留、顯示無法確認，不重設其他偏好。
正常偏好備份不攜帶 pending marker、操作 revision、憑證或 consent。

設定匯出／匯入仍經 `settingsTransfer.ts` 與 `CharacterLibrarySection`：先驗證 schema/version、
所有已知欄位型別及 adapter meta，再提交偏好。缺少的 optional 欄位保留目前值；**明確的空角色名稱是
有效值，還原時會清除後來的自訂名稱**（v0.7.0 曾忽略此空值）。匯入時角色／adapter meta 缺席則拒絕；既有偏好指向已移除 package 才走既有 fallback，
不刪除 package 或使用者資料，不替使用者授權硬體能力。

必要回歸：`preset_service` 真 Runtime＋暫存偏好檔；`companion-preset-recovery.test.tsx` 的讀取競態；
`settings-transfer-validation.test.ts` 與 `companion-gateway-wiring.test.ts` 的匯入契約；
`scripts/tests/tauri-preset-recovery.py` 的真 Tauri＋隔離 fault proxy。最後一支測量原生視窗，
proxy 故障是模擬器，不是真網路故障或真人可用性研究。
