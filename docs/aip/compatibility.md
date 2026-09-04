# AIP 相容矩陣、minor 演進規則與 deprecation 表

> 契約在 `docs/aip/README.md`（AIP 1.0）與 `docs/aip/character-session.md`（Character Profile）。
> 這份文件只記錄**版本之間**的關係：誰跟誰能對話、minor 可以怎麼長、什麼被標記為即將移除。
> 每個 minor 發布時都必須更新這裡，否則 deprecation 承諾（§3）就是空話。

## 1. 協定相容矩陣

| | AIP 1.0 | CPP 1.0（`docs/character-protocol/README.md`） | iPhone 線協定 v1（`mobile.rs`／`Protocol.swift`） |
|---|---|---|---|
| **角色** | 跨裝置語意訊息契約 | Renderer 呈現契約 | Transport 綁定 |
| **AIP 1.0 的關係** | — | Behavior Intent 由 Runtime 端 `CppRendererAdapter` **投影**成 CPP `IntentEnvelope`；CPP wire 訊息語意不變 | 新增一種 frame `{"type":"aip","envelope":{…}}`；v1 既有訊息全部保留 |
| **誰是權威** | `crates/interaction-aip` | `crates/interaction-character` | `crates/interaction-runtime/src/mobile.rs` |
| **golden schema** | `schemas/aip-1.0.schema.json` | `schemas/character-protocol.schema.json` | 無（wire 由 ProtocolTests 釘住） |
| **版本協商** | `capability`（§4.2） | CPP `hello`／`negotiate`／`negotiated` | `auth-ok` 之後才允許 `aip` frame |
| **未知值行為** | 未知 type／name 不執行；未知選填欄位保留 | 不支援即誠實降級（substituted／reduced／unsupported） | 舊 App 遇到未知 type 只記錄不執行 |

### 1.1 實作分佈

| 語言 | 型別 | 行為 | 測試 |
|---|---|---|---|
| Rust（權威） | `crates/interaction-aip/src/*.rs` | 同左 | `crates/interaction-aip/tests/conformance.rs` |
| TypeScript | `apps/interaction-desktop/src/aip/generated.ts`（產生） | `apps/interaction-desktop/src/aip/envelope.ts`（手寫） | `src/test/aip-conformance.test.ts`、`src/test/aip-envelope.test.ts` |
| Swift | `InteractionCompanion/Models/AIPGenerated.swift`（產生） | `InteractionCompanion/Models/AIPEnvelope.swift`（手寫） | `InteractionCompanionTests/AIPConformanceTests.swift` |

型別**只能**由 `scripts/aip-codegen.mjs` 產生；CI 的 `pnpm aip:check` 會擋下手改與忘記重生。
行為是手寫的，一致性靠三個語言讀**同一份** fixture index 保證（`docs/aip/conformance.md`）。

## 2. minor 演進規則（§4.1 的可操作版本）

一個 `aip/1.x` → `aip/1.(x+1)` 只能做這些事：

| 允許 | 不允許 |
|---|---|
| 新增**選填**頂層欄位（舊實作保留並忽略） | 新增必填欄位 |
| 新增 name（舊實作回 `rejected{unknown-name}`，不執行） | 改變既有 name 的語意或 payload 形狀 |
| 新增 capability／feature 鍵（舊實作協商成 `unsupported`） | 移除既有 capability（要先 deprecate，見 §3） |
| 新增 `Outcome` 值，且**只在新 profile 使用** | 讓既有 profile 突然回傳新的 Outcome |
| 新增 `ErrorCode`（舊實作看到未知碼保留原字串） | 改變既有 ErrorCode 的觸發條件 |
| 放寬上限並在 `capability.limits` 協商（取 min） | 直接調高上限而不協商 |

跨 major（`aip/2.0`）才可以改既有欄位語意；舊 major 一律 `unsupported-version`，不猜、不降級執行。

發一個新 minor 的動作清單：

1. 改 `crates/interaction-aip`（型別／規則）並更新 `SPEC_MINOR`。
2. `GOLDEN_UPDATE=1 cargo test -p interaction-e2e --test golden` 重生 `schemas/aip-1.0.schema.json`。
3. `pnpm aip:codegen` 重生 TS／Swift 型別與 `AIPFixtures.swift`。
4. 在 `crates/interaction-aip/tests/fixtures/` 加**新 minor 的 envelope fixture**與對應 expected，三個語言一起跑綠。
5. 更新本文件 §1 矩陣與 §3 表，並在 `CHANGELOG.md` 記錄新增了什麼。

## 3. Deprecation 表

目前**沒有**任何被標記為 deprecated 的欄位、name 或 capability。

| 項目 | 標記 deprecated 的版本 | 最早可移除的版本 | 相容 adapter | diagnostics 警告 |
|---|---|---|---|---|
| _（空）_ | — | — | — | — |

流程（AIP §4.1）：標 `deprecated` → 至少跨一個公開 minor → 提供 compatibility adapter 與
`aip.deprecated-used` diagnostics warning → 更新本表 → 才可移除。舊 Character Package 與舊 iPhone App
不得因為 deprecation 而無法啟動。

## 4. 實作註記（契約補充）

實作 AIP 1.0 時發現契約有三處缺漏或需要說明。這裡用最小方式補記，**不改變**
`docs/aip/README.md` 的既有語意；下次修訂契約時應把 4.1 併回 §12。

### 4.1 `session-disabled` 不在 §12 的錯誤碼清單裡

`docs/aip/README.md` §12 列了 18 個穩定錯誤碼，但 `docs/aip/architecture-boundaries.md` §5 要求
feature flag 關閉時回 `503 session-disabled`。權威實作 `interaction_aip::ErrorCode::KNOWN` 因此有
**19** 個值，多出 `session-disabled`。

`session-disabled` 的語意：Session Host 沒有啟動（`INTERACT_AI_CHARACTER_SESSION=0`），不是這則訊息有問題。
`retryable` 為 `false`（重送同一則不會變好，要先開啟 feature flag）。

### 4.2 每個 `character.interaction.*` 事件都必須帶 `expiresAt`

§7 寫「互動事件（`character.interaction.*`）必填」，§8 又把 `character.interaction.dismiss` 歸為
`drop-if-offline`（不排隊，deadline 看似無用）。實作依 §7 字面執行：**所有** `character.interaction.*`
事件缺 `expiresAt` 一律 `schema-invalid`，dismiss 也不例外。

理由：deadline 不只管排隊，也管「host 收到時已經太舊就不套用」。兩個機制互不取代，同時要求較安全。

### 4.3 錯誤訊息不得回顯未知的 `messageType`

§5 要求 `error.payload.message` 不含輸入回顯。未知 `messageType` 的原字串是呼叫端可控、長度不受
envelope 欄位限制的資料，因此三個實作的 `unsupported-message-type` 訊息一律是固定文字
（"messageType is not one of the 12 known AIP message types"），原字串只保留在
`MessageType::Unknown` 供本地稽核，不回到 wire 上。
