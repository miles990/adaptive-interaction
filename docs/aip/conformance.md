# AIP conformance：三個語言、一份 fixture、怎麼跑、怎麼重生

> 契約在 `docs/aip/README.md`（§14 列出必須存在的 conformance 測試）。
> 這份文件是操作手冊：怎麼跑、加 fixture 時要動哪些檔、golden 與 codegen 怎麼重生。

## 1. 為什麼要跨語言 conformance

AIP 的檢查是**安全邊界**，不是格式美化。同一則訊息，Rust Runtime 拒絕、桌面 WebView 接受，
攻擊面就從確定性的 Rust 檢查漏到 JS；iPhone 端寬鬆一分，未配對裝置就多一分機會。

所以三個實作讀**同一份** fixture index，對每一則訊息必須得到**同一個**結論（`ok`，或同一個 ErrorCode）：

| 語言 | 測試 | 怎麼拿到 fixtures |
|---|---|---|
| Rust（權威） | `crates/interaction-aip/tests/conformance.rs` | 直接讀 `tests/fixtures/` |
| TypeScript | `apps/interaction-desktop/src/test/aip-conformance.test.ts` | `node:fs` 讀同一個目錄（檔頭 `// @vitest-environment node`） |
| Swift | `apps/interaction-ios/InteractionCompanionTests/AIPConformanceTests.swift` | 讀 `AIPFixtures.swift`（XCTest 讀不到 repo 檔，所以由 codegen 內嵌成字串） |

## 2. 怎麼跑

```bash
# Rust：型別、規則與 fixture conformance（含 stateHashes 消費者測試）
cargo test -p interaction-aip
# Rust：state-hash fixtures 的產生器兼驗證器（AIP_UPDATE_FIXTURES=1 重生）＋數字字面語意
cargo test -p interaction-session --test state_hash_fixtures --test state_semantics
# Rust：接收端決策表（產生器 + 只讀 JSON 的獨立消費者 + 行為測試）
cargo test -p interaction-session --test receive_decision_fixtures --test receive_decisions_from_json --test receive_decisions

# Golden schema 不漂移（schemas/aip-1.0.schema.json）＋ 依賴邊界
cargo test -p interaction-e2e --test golden
cargo test -p interaction-e2e --test dependency_boundaries

# TypeScript：conformance ＋ 邊界單元測試
cd apps/interaction-desktop
pnpm vitest run src/test/aip-conformance.test.ts src/test/aip-envelope.test.ts
pnpm vitest run src/test/canonical-hash.test.ts src/test/session-client.test.ts   # state hash 三端一致＋接收端 reducer
pnpm aip:check      # generated.ts／AIPGenerated.swift／AIPFixtures.swift 不漂移
pnpm typecheck

# Swift（iOS 模擬器）：見 apps/interaction-ios/README.md 的 xcodebuild + simctl 流程
#   1) xcodebuild -target InteractionCompanion      -sdk iphonesimulator … build
#   2) xcodebuild -target InteractionCompanionTests -sdk iphonesimulator … build
#   3) simctl install + launch -XCTest All（注入 libXCTestBundleInject.dylib）
#      app bundle 的 Frameworks/ 需要 XCTest*.framework、Testing.framework、
#      _Testing_*.framework 與 lib_TestingInterop.dylib 都在位，否則啟動即失敗。
```

**證據分級**：Rust／TypeScript 是 `unit`＋`contract`；Swift 是 `simulator`（iOS 模擬器）。
三者都**不是** `real-device`。iPhone 真機證據只有 `docs/releases/v0.5.0-iphone-device-evidence.md`
逐列標示的那些，AIP conformance 不在其中。

## 3. Fixture index 的格式

`crates/interaction-aip/tests/fixtures/manifest.json` 是唯一索引。三個語言都只信它，不自行掃目錄。

| 區段 | 每筆欄位 | 意義 |
|---|---|---|
| `envelopes` | `id`、`file`、`expect`（`ok`／`error`）、`code`、`roundTrip?`、`mustNotEcho?`、`note?` | 存成檔案的 envelope fixture |
| `generated` | `id`、`expect`、`code`、以及 `raw`（原始文字）或 `base`＋`inflatePayloadChars` | 超大訊息與壞 JSON 在測試內生成，不存大檔 |
| `negotiations` | `id`、`offer`、`announcement`、`expect`、`negotiated`／`code` | §4.2 capability 協商（Rust／TypeScript 跑；iPhone 只宣告不協商） |
| `identity` | `id`、`bound`、`claimed`、`expect`（`accept`／`reject`） | §5 身分綁定決策表 |
| `offlinePolicy` | `name`、`hasConsentGrant`、`expect` | §8 離線政策表 |
| `outcomeTransitions` | `from`、`to`、`allowed` | §3 誠實階梯的合法遷移 |
| `outcomeProfiles` | `profile`、`status`、`allowed` | §3 各 profile 的合法 Outcome 子集 |
| `nameScope` | `name`、`runtimeOnly` | §2.3 只有 Runtime 能送的前綴 |
| `stateHashes` | `id`、`file`、`semanticValid`、`note`；檔案內含 `state`、`hash`、`canonical` | §6 state hash：host 真實寫出的 `SemanticState` 與其 canonical 文字／SHA-256（Rust `conformance.rs`＋`state_hash_fixtures.rs`／TS `canonical-hash.test.ts`／Swift `StateHashConformanceTests`） |
| `stateHashDoublePaths` | JSON pointer 陣列 | `SemanticState` 裡所有 f64 欄位（schemars 推導）；TS 的 `SEMANTIC_STATE_DOUBLE_PATHS` 由 codegen 從這裡產出 |
| `receiveDecisions` | `id`、`note`、`local`、`incoming`／`incomingBatch`／`incomingBatchChain`、`budgetBefore?`、`expect` | §6／`character-session.md` §7.2 的接收端決策表：一則 `state` 訊息對一份本地副本的決策（`apply`／`reset`／`recover`／`realign`／`ignore-stale`／`already-applied`／`reject-*`／`ignore-stale-connection`） |

`mustNotEcho` 列出這則 fixture 裡的呼叫端可控字串；三個實作都會斷言錯誤訊息**不含**它們（§5）。
三個實作另外都會斷言錯誤訊息不含路徑片段、長度 ≤ 200 字。

`receiveDecisions` 的每一筆是「本地長這樣（`local`）＋收到這則訊息（`incoming`）＝這個決策（`expect.decision`）」。
`local.hash`／`incoming.hash` 是真的 SHA-256（消費端可以自己重算）；`incoming.computedHash` 是**接收端自己算出來的**
hash（snapshot ＝對收到的 `state`；patch ＝merge 之後的結果），`null`／缺席代表「這個接收端沒有核對」，
不代表核對過了。`incomingBatch` 是一則 resume 回覆的逐則內容；`incomingBatchChain{kind, count}` 是「從
`local.revision` 起連續 `count` 則 patch」的縮寫（用來釘住 `maxResumePatches` 的邊界，不必把幾百則訊息寫進檔案）。
`expect` 一定帶 `revisionAfter`／`epochAfter`（套用後的本地副本；不採用的決策就是原值）與 `budgetAfter`／`budget`
（有界 realign 的計數與結論）；批次案例另有 `applied`／`skipped`／`stoppedAt`。
**超大訊息與壞 JSON 不在這一段**：那是 typed boundary 的事，由 `envelopes`／`generated` 段涵蓋；
boundary 擋下一則**權威回覆**時算一次 realign 失敗（案例 `boundary-rejected-authoritative-reply-costs-one-attempt`）。

`state` fixture 的 `hash` 是對 `state`／套用 patch 後狀態的 canonical JSON 取 SHA-256 的**真值**
（`state-snapshot.json` 的 state 是 host 真實形狀：`intensity` `0.0`、`members[].unsupportedIntents`；
`state-patch.json` 的 `hash` 與它鏈在一起，Rust `state_semantics.rs` 釘住這一對）。envelope conformance
只驗結構；hash 的**計算**由 `stateHashes` 段的三端測試驗證，hash 的**接收端決策**屬於 `interaction-session`。

## 4. 加一則 fixture

1. 把 envelope JSON 放進 `crates/interaction-aip/tests/fixtures/`（超過 32 KiB 的請改用 `generated` 區段）。
2. 在 `manifest.json` 的 `envelopes` 加一筆，寫清楚 `expect` 與（若是 error）`code`。
3. `cargo test -p interaction-aip` → 先看 Rust 的結論是否符合預期。
4. `pnpm aip:codegen`（重生 `AIPFixtures.swift`，把新檔內嵌進去）。
5. TypeScript 與 Swift 測試重跑；三邊都綠才算加完。

## 5. 重生 golden 與 generated 檔

```bash
# 1. Rust 型別改了 → 重生 golden schema，再正常跑一次確認不漂移
GOLDEN_UPDATE=1 cargo test -p interaction-e2e --test golden
cargo test -p interaction-e2e --test golden

# 2. golden schema 或 fixtures 改了 → 重生 TS／Swift 型別與內嵌 fixtures
cd apps/interaction-desktop && pnpm aip:codegen
pnpm aip:check      # 應該印 "in sync"
```

順序不能反：`scripts/aip-codegen.mjs` 讀的是 `schemas/aip-1.0.schema.json`，不是 Rust 原始碼。
schema 沒重生就跑 codegen，只會把舊型別再寫一次。

CI 的 frontend job 會跑 `pnpm aip:check`：手改 `generated.ts`／`AIPGenerated.swift`／`AIPFixtures.swift`，
或改了 schema／fixtures 卻忘記重生，都會 exit 1。

## 6. 依賴邊界

`tests/e2e/tests/dependency_boundaries.rs` 釘住 `docs/aip/architecture-boundaries.md` §1：
`interaction-aip` 與 `interaction-session` 的**直接**依賴不得含
tokio／axum／tauri／tungstenite／rumqttc／serialport／btleplug／reqwest／hyper，
**遞移**依賴（走 `cargo metadata` 的 normal 依賴圖）也不得含 tokio。
找不到 `cargo` 時遞移那一項會 skip 並印出原因，直接依賴那一項仍然會跑。
