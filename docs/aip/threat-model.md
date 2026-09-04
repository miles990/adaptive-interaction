# AIP／Character Session 威脅模型：資產、信任邊界、攻擊面逐項

> 證據等級用字：每一列「防線」都附 path:line；「測試」欄只填**實際存在的測試函式名**（本文件核對
> 過程以 `rg`／`sed`／`cat`／`git show HEAD:<path>`（`HEAD = edb168259980a03c470cabd26441aa2634a926dd`）
> 逐一核對函式存在，**沒有執行 `cargo test`／`pnpm test`，不宣稱任何測試「目前通過」**）；沒有找到防線
> 或測試的項目誠實寫「未覆蓋」，不得回填成「已驗證」。真機一律「未驗證」；`examples/fake_iphone`／
> `mobile_loop.rs` 的模擬連線一律標「模擬 iPhone（fixture）」。

## 1. 資產（值得保護的東西）

| 資產 | 為什麼重要 |
|---|---|
| 語意狀態（`SemanticState`：mood／activity／attention／truth／members） | 唯一權威來源；被污染會讓所有 renderer 顯示錯誤的角色狀態 |
| `verified` 判定 | 只能來自人類驗證路徑；偽造會讓 AI 或裝置能自稱「工作已驗證完成」 |
| 每機配對 token | 洩漏＝任意程式可冒充該裝置操作角色、送假事件 |
| TLS 私鑰／憑證指紋 | 洩漏或指紋比對失效＝中間人可攔截整個 iPhone Session 流量 |
| Character Package（manifest＋assets） | 惡意 package 可能路徑穿越寫出資料夾外、偽造資產類型、耗盡資源 |
| Diagnostics 輸出 | 若外洩 token／路徑／原始 payload，等於把攻擊面直接印給任何讀得到 HTTP 回應的人 |
| Session 持久化檔（`character-session.json`） | 竄改後重啟即可偽造歷史 revision／state |

## 2. 信任邊界

```
Runtime（唯一權威：Character Session Host、Policy Governor、Consent Service）
   │  已驗證身分（Transport 綁定，非自稱）
   ├── 可信 host surface（桌面視窗，human token → human-surface:desktop）
   ├── 配對裝置（iPhone，配對 token → device:<deviceId>）
   ├── 外部 Character Adapter（adapter token，CPP WebSocket，不承載 AIP）
   └── AI Agent（agent token；不可授予 consent、不可解除 estop、不可產生 verified）
```

跨越這些邊界的任何訊息都要先過 `crates/interaction-session/src/session.rs::gate()`（§3 的 13 步固定管線）
或（外部 CPP adapter 路徑）CPP 自己的驗證管線；沒有任何邊界外的呼叫端能繞過去直接改
`SemanticState`——欄位私有，只有 `CharacterSession::apply` 能改（`docs/aip/character-session.md` §2）。

## 3. 核心防線：`gate()` 的 13 步固定管線

`crates/interaction-session/src/session.rs::gate()`（`:642-737`），逐步引用：

| 步 | 檢查 | 位置 | 失敗結果 |
|---|---|---|---|
| 1 | schema／profile／大小／深度／版本／name 語法 | `:648-651`（呼叫 `envelope.validate()`，`crates/interaction-aip/src/envelope.rs`） | 對應 `ErrorCode`（來自 `validate()`） |
| 2 | 身分綁定（宣稱 vs Transport 綁定身分） | `:652-659`（呼叫 `bind_identity`，`crates/interaction-aip/src/envelope.rs:330`） | `IdentityMismatch` |
| 3 | 外部訊息不得宣稱 `Runtime` 身分 | `:660-666` | `IdentityMismatch` |
| 4 | Session membership | `:668-671` | `NotAMember` |
| 5 | 跨 session 注入（`sessionId` 不符） | `:672-677` | `NotAMember` |
| 6 | `task.*`／`runtime.*` 只有 Runtime 可送 | `:678-681`（`is_runtime_only_name`，`crates/interaction-aip/src/message.rs:164`） | `ScopeDenied` |
| 7 | `event` 的 `name` 必須在協商過的 `inputs` 內 | `:682-692` | `ScopeDenied` |
| 8 | `result.payload.status:"verified"` 只有 Runtime 能送 | `:693-698` | `ScopeDenied` |
| 9 | member 只能送 `event／cancel／query／result／heartbeat／capability` | `:699-710` | `ScopeDenied` |
| 10 | Rate limit（per-member token bucket，時間注入） | `:711-717` | `RateLimited` |
| 11 | Deadline（`envelope.is_expired(now)`） | `:718-725` | `Expired` |
| 12 | 去重（256 筆 messageId 環，重複回 `accepted{duplicate:true}` 不重套用） | `:726-729` | `Gate::Duplicate`（非錯誤，是特殊路徑） |
| 13 | Emergency 中拒絕互動事件 | `:730-735` | `ScopeDenied` |

這 13 步的順序本身是安全屬性（先確認身分再看內容，避免用「內容合法」掩護「身分不合法」）：測試
`crates/interaction-session/tests/security_matrix.rs::the_pipeline_order_is_fixed_identity_before_membership_before_scope`
（`:217`）。

## 4. 攻擊面逐項

### 4.1 身分與連線層

| 攻擊面 | 防線 | 測試 | 證據等級 |
|---|---|---|---|
| 偽造 `source.id` | `gate()` 步 2／3；`bind_identity`（`envelope.rs:330-360`附近，`IdentityDecision::Reject`） | `crates/interaction-session/tests/session.rs::identity_mismatch_is_rejected_and_audited`（`:481`） | unit（純函式，本次未重跑） |
| 未配對裝置 | Transport 層：`mobile.rs` 的 `auth`／`aip` 分派只在 `authed.is_some()` 之後才可能進 session；Session 層：`gate()` 步 4 | `crates/interaction-session/tests/security_matrix.rs::an_unpaired_device_is_never_a_member`（`:70`） | unit |
| 已撤銷裝置重連 | `mobile_revoke` 從 `devices` map 移除該裝置後，`auth` 比對 `token_hash` 永遠失敗（`mobile.rs:3427-3453`）；`character_session_leave` 立即移出 session（`:2437-2438`） | `crates/interaction-runtime/tests/mobile_loop.rs::token_reconnect_works_until_revoked`、`::revoke_disconnects_live_connection_immediately` | unit（fixture 連線，模擬 iPhone（fixture）） |
| Bonjour spoofing | Bonjour 只承載服務發現資訊，不含任何驗證秘密；真正信任在 TLS 指紋 TOFU＋配對碼 HMAC（`docs/aip/pairing-security.md` §1-§3） | 無專屬測試——這是架構設計取捨（廣播內容本來就公開），不是可用單一測試斷言的攻擊情境 | 未覆蓋（設計文件層級的緩解，非測試鎖住） |
| Endpoint migration hijack（把裝置導到假伺服器） | iOS `PinnedWebSocketDelegate` 的 TLS 指紋固定（TOFU）；指紋不符必歸類 `tlsMismatch`，不得被誤判為單純位址變更 | `ReconnectHintTests.swift::testTlsMismatchIsNeverReportedAsAddressChange` | 模擬器（iPhone 17 XCTest，未重跑） |
| 同名裝置（device name 碰撞） | `PairedDevice.name`（`mobile.rs:606`）只是使用者顯示用字串，截斷 24 字元、**不參與**任何身分或授權判斷；真正的身分是隨機產生的 `device_id`（`iphone-` + 8 hex）與 `token_hash` | 無測試（因為 `name` 從未被當成安全邊界，沒有對應的攻防情境需要測試） | 未覆蓋（非防線需求：`name` 不是信任依據，見 §4「每機 Token」） |

### 4.2 訊息完整性與重放

| 攻擊面 | 防線 | 測試 | 證據等級 |
|---|---|---|---|
| Duplicate messageId | `gate()` 步 12；每成員 256 筆去重環 | `crates/interaction-session/tests/session.rs::duplicate_message_id_is_accepted_once_and_never_reapplied`（`:391`） | unit |
| Out-of-order delivery | host 以自己時鐘為準套用（不依賴到達順序），舊 messageId 重放走去重路徑 | `crates/interaction-session/tests/session.rs::out_of_order_delivery_cannot_replay_an_old_touch`（`:409`，同一測試內也涵蓋 replay 與 expired，見下） | unit |
| Replay（重放舊事件） | 去重環（步 12）擋「同一 messageId 再送」；deadline（步 11）擋「真正的舊事件」 | 同上（`:409`，函式內同時斷言 duplicate 與 expired 兩種情況） | unit |
| Expired（過期互動事件） | `gate()` 步 11：`envelope.is_expired(now)` | `session.rs::out_of_order_delivery_cannot_replay_an_old_touch`（`:409`，末段 stale touch 案例）；意圖層另有 `session.rs::presence_times_out_and_expired_intents_are_dropped`（`:830`，過期 Behavior Intent 被 `tick` 清除） | unit |
| Oversized（訊息／payload／深度／字串超限） | `envelope.validate()`（`crates/interaction-aip/src/envelope.rs`），`gate()` 步 1 | `crates/interaction-session/tests/security_matrix.rs::oversized_payloads_are_rejected_before_anything_is_applied`（`:80`）；Transport 層另有 `mobile_loop.rs::an_oversized_message_closes_the_connection` | unit＋fixture |
| Unknown message type／version | `envelope.validate()`；`gate()` 步 1 | `security_matrix.rs::unknown_message_types_and_versions_are_not_executed`（`:107`） | unit |
| Unknown capability | 協商只回應 host 自己宣告過的 intent／inputs，對方發明的能力直接不出現在協商結果裡 | `security_matrix.rs::renderer_capability_spoofing_only_earns_unsupported`（`:131`，同時是 §4.4「renderer capability spoofing」的測試） | unit |
| Invalid `baseRevision` | member 端套用 patch 前檢查 `baseRevision` 是否等於本地 revision，不連續就不套用（純函式 `apply_patch`） | `crates/interaction-session/tests/pure_functions.rs::accept_state_applies_only_contiguous_patches`（`:123`） | unit |
| Snapshot rollback（竄改或倒退的持久化 snapshot） | `CharacterSession::restore()` 核對 `state_hash`（canonical JSON SHA-256，`patch.rs:58`）與 `session_id`；不符即拒絕還原，不悄悄接受 | `crates/interaction-session/tests/session.rs::restore_rejects_tampered_snapshots`（`:1028`，涵蓋 `HashMismatch`／`SessionMismatch`／`InvalidState` 三種竄改） | unit |
| Cross-session injection | `gate()` 步 5：`envelope.sessionId` 不等於本 session 一律 `NotAMember` | `crates/interaction-session/tests/session.rs::cross_session_injection_is_not_a_member`（`:512`） | unit |

> **AIP README §6 描述的「member 收到 `state.revision` ≤ 本地已套用 revision 一律忽略」的接收端
> rollback 防護**：這是**接收 `state` 廣播那一方**（renderer／member）的責任，不是 `interaction-session`
> （它是 host，只會遞增自己的 revision，不存在「host 自己倒退」的情境）。本文件核對範圍
> （`crates/interaction-aip/src/*.rs`、`crates/interaction-session/src/*.rs`）沒有找到接收端這條規則的
> 共用純函式或測試——如果它是由 TS（`apps/interaction-desktop/src/aip/envelope.ts`）或 Swift
> （`AIPEnvelope.swift`）各自實作，不在本文件核對的檔案清單內，**誠實標記「未核實」**，不是「不存在」。

### 4.3 授權與同意

| 攻擊面 | 防線 | 測試 | 證據等級 |
|---|---|---|---|
| Consent scope mismatch | `ConsentVerifier` port 已定義（`crates/interaction-session/src/ports.rs`）但**1.0 沒有任何呼叫端使用它**——AIP 1.0 的 session 訊息裡不會出現帶 `consentGrantId` 的 `command`（member 不能送 `command`，`docs/aip/transport-bindings.md` §7 第一點） | 無 | **未覆蓋**：這條路目前走不到，不是「已驗證安全」，是「還沒有訊息形狀能觸發它」 |
| Single-use grant 重複消耗 | 不在 AIP／Character Session 範圍——這是 `interaction-core::policy::Consent.maxUses` 的既有機制（`docs/releases/v0.5.1-known-limitations.md` §3 第 1 點記載：只在**動器**派工路徑原子消耗，受器與 tool-operation 路徑不計次） | 見 v0.5.1 既有文件，本文件未重新核對 `interaction-policy` 原始碼（`docs/releases/v0.6.0-recovery-matrix.md` §4 明確列為「未盤點」） | 未覆蓋（本輪任務範圍外，沿用既有文件結論） |
| Crash 後 grant 復活 | 同上，屬 `interaction-core::policy`／`interaction-runtime::executor` 既有機制，非 AIP 新增範圍 | 同上 | 未覆蓋（本輪任務範圍外） |

### 4.4 Renderer／Adapter 層

| 攻擊面 | 防線 | 測試 | 證據等級 |
|---|---|---|---|
| Duplicate subscription（同一輸入被送兩次） | 內建 adapter 的訂閱管理：兩個 callback 各收一次，退訂只退自己那一份 | `apps/interaction-desktop/src/test/adapter-contract.test.ts`「重複訂閱：兩個 callback 各收一次，退訂只退自己那一份」（見 `docs/aip/renderer-adapter.md` §5／§6 對應列） | unit（vitest，本次以既有文件記載核對，未重新讀取行號） |
| Renderer capability spoofing（宣告不存在的能力騙取協商） | 協商演算法只會回應 host 自己認得的 intent／inputs 交集；對方多宣告的東西直接被忽略，不影響其他誠實成員 | `crates/interaction-session/tests/security_matrix.rs::renderer_capability_spoofing_only_earns_unsupported`（`:131-176`，同時驗證說謊者事後仍不能送 `task.verified`） | unit |

### 4.5 Character Package／匯入層

| 攻擊面 | 防線 | 測試 | 證據等級 |
|---|---|---|---|
| Malicious Character Package（惡意 manifest／entrypoint／資產） | CPP §2.1 驗證規則（大小上限、路徑規則、`entrypoint` 只記錄不執行、白名單由 host 注入）；`character_store.rs` 的 `validate_import` | `crates/interaction-character/tests/manifest.rs`（`docs/character-protocol/README.md` §13 記載 18 個測試，本文件沿用既有記載未逐一重新核對）；`apps/interaction-desktop/src-tauri/src/character_store.rs::rejects_spoofed_magic_bytes_and_wrong_hashes`、`::rejects_external_kinds_non_whitelisted_builtins_and_bundled_ids`（見 `docs/aip/character-package.md` §5） | unit |
| Asset path traversal | `is_safe_segment`（`character_store.rs:151`）＋`resolve_inside`（`:162`，逐段檢查＋`Component::Normal`＋`starts_with(base)`斷言） | `character_store.rs::rejects_traversal_in_asset_ids_and_paths`（`:662`，本文件以 `git show HEAD:` 直接核對函式存在）、`::resolve_inside_never_escapes`（`:688`） | unit |
| Symlink traversal | `import()` 內對暫存目錄與 root 各自 `canonicalize()` 後斷言 `tmp_real.starts_with(&root_real)`（`character_store.rs` 約 `:390-403`，「符號連結防線」註解） | **無測試**：本文件以 `rg -n "symlink" apps/interaction-desktop/src-tauri/src/character_store.rs`（透過 `git show HEAD:`）核對，只命中程式碼註解，沒有命中任何測試函式名（`fn ...symlink...`）——防線程式碼存在，**沒有專屬回歸測試建立真實 symlink 驗證它** | 未覆蓋（程式碼存在，未見專屬測試） |

### 4.6 Diagnostics 與誠實階梯

| 攻擊面 | 防線 | 測試 | 證據等級 |
|---|---|---|---|
| Diagnostics secret leakage | `Diagnostics`（`session.rs` 的 `diagnostics()`，約 `:626-637`）只回 `session_id／epoch／revision／sequence／members／counters／eventLog{len,cap}`，不含 token／路徑／原始 payload；`character_session.rs::character_session_diagnostics_value`同樣只組出這些欄位；`storeNote` 自 `b5cdfa2` 起是固定常數（`STORE_NOTE_UNUSABLE`／`STORE_NOTE_UNREADABLE`），錯誤細節只進 log | `character_session::tests::store_note_never_carries_error_details_or_paths`（壞檔含路徑樣式與內容，斷言 note 不含）＋`crates/interaction-session/tests/session.rs::diagnostics_counts_without_leaking`（`:1105`，斷言 counters 鍵不含 `"iphone"` 字樣、members 序列化後不含 `"negotiated"`） | unit |
| Fixture 冒充真機 | `EvidenceClass`（`crates/interaction-aip/src/evidence.rs`）型別上把 `Fixture`／`Simulator` 與 `RealDevice`／`RealAgent`／`RealHardware` 分開，`is_real()`（`:34-39`）明確排除前兩者；`examples/fake_iphone` 與相關測試檔頭註解自陳「模擬 iPhone（fixture）」；CLAUDE.md 專案規則要求所有模擬器／fixture 結果標「模擬器」 | 沒有「測試」能檢查文件用字是否誠實——這是型別系統（`EvidenceClass`）＋專案慣例＋人工審閱三層防線的組合，不是單一自動化測試能鎖住的東西 | 未覆蓋（結構性緩解，非測試鎖住）——本文件與其餘 8 份 v0.6.0 文件在撰寫時已逐項標示模擬 iPhone（fixture）／未驗證，即是這道防線的實踐 |
| Claimed-completed 冒充 verified | `Outcome::can_transition_to`（`crates/interaction-aip/src/outcome.rs:109-136`）只允許 `ClaimedCompleted → Verified`（不允許 `Acknowledged`／`Observed` 直接變 `Verified`）；`Outcome::Verified.is_runtime_only()`；`gate()` 步 8（member 送 `result{status:"verified"}` 直接 `ScopeDenied`） | `outcome.rs::tests::ladder_is_honest`（同檔內嵌測試，斷言 `!Acknowledged.can_transition_to(Verified)`、`!Observed.can_transition_to(Verified)`、`ClaimedCompleted.can_transition_to(Verified)`、`Verified.is_runtime_only()`）；`crates/interaction-session/tests/session.rs::devices_may_not_produce_runtime_truth_or_verified`（`:531`）；`crates/interaction-session/tests/security_matrix.rs::every_result_envelope_validates_and_never_claims_verified`（`:260`） | unit |

## 5. 錯誤碼與訊息不回顯輸入

`crates/interaction-aip/src/error.rs`：`ErrorCode::KNOWN`（`:35-56`）恰為 19 個值（18 個具名＋
`SessionDisabled`，對應 `docs/aip/compatibility.md` §4.1 的說明）；`retryable()`（`:82-84`）只有
`RateLimited`／`Internal` 允許用同一 `messageId` 重送。`error.payload.message ≤ 200 字、不回顯輸入`
是型別上的文件約束（`error.rs:87` doc comment），本文件核對過程未在 `interaction-aip` 原始碼中找到
對「訊息內容是否真的不含輸入回顯」做斷言的測試函式名（例如檢查某個構造出的錯誤字串不包含使用者
提供的原始字串）——`docs/aip/README.md` §5 與 `docs/aip/compatibility.md` §4.3 描述了這條規則的設計
意圖（`unsupported-message-type` 一律回固定文字，原字串只留在本地稽核），但本文件核對範圍內沒有找到
可引用的測試函式名，誠實標記**未覆蓋**（規則存在於程式碼結構，未見專屬回歸測試）。

## 6. 已知未覆蓋清單（彙總）

以下項目在本文件核對過程中確認「防線程式碼存在但無專屬測試」或「完全未覆蓋」，供 v0.6.0 後續補測試：

1. Bonjour spoofing（架構緩解，非測試對象）。
2. 同名裝置碰撞（非防線需求，`name` 不是信任依據）。
3. Consent scope mismatch（1.0 訊息形狀走不到這條路）。
4. Single-use grant 重複消耗／crash 後復活（`interaction-policy`，本輪任務範圍外，未盤點）。
5. Symlink traversal（程式碼防線存在，無專屬測試）。
6. 接收端（member）的 `state.revision` rollback 忽略規則（README §6）——未在本文件核對的 Rust
   檔案內找到對應純函式或測試，可能存在於 TS／Swift，未核實。
7. `error.payload.message` 不回顯輸入的斷言型測試——規則存在，未見專屬測試函式名。
