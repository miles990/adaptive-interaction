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
# Rust：canonical JSON 向量的產生器兼驗證器（AIP_UPDATE_FIXTURES=1 重生）
cargo test -p interaction-aip --test canonical_vectors
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
pnpm vitest run src/test/canonical-vectors.test.ts   # canonical JSON 向量（鍵序／跳脫／數字字面）
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
| `canonicalVectors` | `id`、`note`、`doublePaths`、`input`、`canonical`、`sha256` | §6 canonical JSON 本身的向量：鍵序、跳脫、數字字面（Rust `canonical_vectors.rs`／TS `canonical-vectors.test.ts`／Swift `CanonicalVectorsTests`） |

`mustNotEcho` 列出這則 fixture 裡的呼叫端可控字串；三個實作都會斷言錯誤訊息**不含**它們（§5）。
三個實作另外都會斷言錯誤訊息不含路徑片段、長度 ≤ 200 字。

`receiveDecisions` 的每一筆是「本地長這樣（`local`）＋收到這則訊息（`incoming`）＝這個決策（`expect.decision`）」。
`local.hash`／`incoming.hash` 是真的 SHA-256（消費端可以自己重算）；`incoming.computedHash` 是**接收端自己算出來的**
hash（snapshot ＝對收到的 `state`；patch ＝merge 之後的結果），`null`／缺席代表「這個接收端沒有核對」，
不代表核對過了。`incomingBatch` 是一則 resume 回覆的逐則內容；`incomingBatchChain{kind, count}` 是「從
`local.revision` 起連續 `count` 則 patch」的縮寫（用來釘住 `maxResumePatches` 的邊界，不必把幾百則訊息寫進檔案）。
`expect` 一定帶 `revisionAfter`／`epochAfter`（套用後的本地副本；不採用的決策就是原值）與 `budgetAfter`／`budget`
（有界 realign 的計數與結論）；批次案例另有 `applied`／`skipped`／`stoppedAt`。`sessionIdAfter` 只在套用後記下的身分
與 `local.sessionId` **不同**時出現（規則 1 只在本地身分已知時比對，未知的那一格由套用補齊）；缺席就是「還是本地那一個」。
**超大訊息與壞 JSON 不在這一段**：那是 typed boundary 的事，由 `envelopes`／`generated` 段涵蓋；
boundary 擋下一則**權威回覆**時算一次 realign 失敗（案例 `boundary-rejected-authoritative-reply-costs-one-attempt`）。

`state` fixture 的 `hash` 是對 `state`／套用 patch 後狀態的 canonical JSON 取 SHA-256 的**真值**
（`state-snapshot.json` 的 state 是 host 真實形狀：`intensity` `0.0`、`members[].unsupportedIntents`；
`state-patch.json` 的 `hash` 與它鏈在一起，Rust `state_semantics.rs` 釘住這一對）。envelope conformance
只驗結構；hash 的**計算**由 `stateHashes` 段的三端測試驗證，hash 的**接收端決策**屬於 `interaction-session`。

**`stateHashes` 的兩條可執行規則**（`crates/interaction-session/tests/state_hash_fixtures.rs`）：

1. **`SemanticState` 的每一個欄位都必須出現在至少一份 `stateHashes` fixture 裡**
   （`every_semantic_state_field_appears_in_at_least_one_state_hash_fixture`，欄位清單由 schemars 推導）。
   理由：`SemanticState` 住在 `interaction-session`，不在 golden schema 裡，加欄位不會讓 golden 或 codegen 紅——
   fixture 是 TypeScript／Swift 唯一會被逼著看到新欄位的東西。缺席時**補 fixture**，不要放寬斷言。
2. **任何 fixture 的 `state` 與 `canonical` 文字都不得含 `null`**
   （`no_fixture_state_or_canonical_text_contains_null`）：值為「無」的選填鍵一律省略，因為 RFC 7396 的 `null`
   是刪除鍵，host 寫 `null` 而接收端刪鍵，兩邊的 canonical hash 就會分岔。

### `canonicalVectors`：canonical JSON 自己的向量

`stateHashes` 那組 fixture 是**真的 `SemanticState`**，鍵全是 ASCII 欄位名（`mood`／
`intensity`／`members`…）。ASCII 鍵在 UTF-8 位元組序、Unicode 排序、UTF-16 code unit 序
底下長得一模一樣，所以那一組在「鍵序」這件事上其實什麼都沒證明——對抗審查
`hash-numeric-contract-017` 指出 TypeScript 端的鍵序曾經是 UTF-16 code unit 序
（補充平面鍵排到 U+F801..U+FFFF 的 BMP 鍵**之前**），修掉之後（363c3d7）也沒有任何一筆
fixture 抓得住它。

`canonicalVectors` 就是補上那張網。12 筆，每一筆針對一個具體的分歧點：

| id | 釘住什麼 |
|---|---|
| `ascii-keys-unsorted` | 基準線：輸入順序 ≠ 輸出順序，巢狀與陣列內物件都要遞迴排序 |
| `bmp-non-ascii-keys` | BMP 非 ASCII 鍵，含 U+FFFD 與 U+F801..U+FFFF（私用區、noncharacter） |
| `supplementary-plane-keys` | 補充平面鍵（U+10000、U+1D11E、emoji、U+10FFFF） |
| `code-point-order-not-utf16` | **code point 序 ≠ UTF-16 code unit 序**；三端各有一條測試斷言這筆向量真的分得開兩種排序 |
| `combining-marks-not-unicode-collation` | **位元組序 ≠ 正規化後的排序**：`e` + U+0301 在位元組序排在 `f`／`z` 之前，Swift 的 `String` `<` 先做 NFC（U+00E9）會排到最後 |
| `escaped-keys-and-values` | 需要跳脫的鍵與值：`"`、`\`、`\b \t \n \f \r` 短寫、`\u0000`／`\u001f`（**小寫**十六進位） |
| `unescaped-passthrough` | 看起來像要跳脫但 serde_json **不**跳脫的：U+2028／U+2029／U+007F／U+00A0／`/` |
| `nested-key-order-recursion` | 鍵序遞迴五層，含空字串鍵與非 ASCII 鍵混排 |
| `numbers-integers` | 整數字面沒有小數點（±2^53 邊界） |
| `numbers-doubles` | f64 整數值帶小數點（`1.0`／`-0.0`／`2.0`）；鍵 `~tilde/slash` 同時釘住 `doublePaths` 的 RFC 6901 跳脫 |
| `numbers-exponent-forms` | ryu 的固定小數 ↔ 科學記號分界：指數 k ∈ [-5, 16) 用固定小數，其餘用 `1e+16`／`1e-6` |
| `empty-containers` | `{}`／`[]` 不得消失、不得變成 `null`（含空字串鍵） |

`input` 一定是 JSON 物件（鍵序才有東西可證明）；`canonical` 是拿去做 SHA-256 的那一串文字；
`sha256` 是它的小寫十六進位摘要。`doublePaths` 是這筆向量裡所有 **f64** 值的 RFC 6901
pointer——TypeScript 端唯一需要的型別知識（JS 的 `number` 分不出 `1` 與 `1.0`），與
`stateHashDoublePaths` 是同一個機制，只是來源是向量本身而不是 schema。Swift 端不需要它
（`SemanticJSON` 逐字保留數字字面）。

重生：`AIP_UPDATE_FIXTURES=1 cargo test -p interaction-aip --test canonical_vectors`，
之後在 `apps/interaction-desktop` 跑 `pnpm aip:codegen`（Swift 端內嵌）。**向量不遷就實作**：
任何一端對不上，要修的是那一端的 canonical 實作。這一段第一次跑起來就抓到了一個真的分歧
（見下）。

**這一段抓到的東西**：TypeScript 的 `canonicalNumber` 原本是
`String(value).replace("e+", "e")`，而 serde_json（ryu）的分界與 JS 的 `String()` 不同——
`1e-6` JS 印 `0.000001`、ryu 印 `1e-6`；`1e16` JS 印 `10000000000000000`、ryu 印 `1e+16`；
`1e21` JS 印 `1e+21`、被那個 `replace` 改成 `1e21`。`mood.intensity` 是 f64 且到得了
`0.000001`，所以這是**線上到得了**的分歧：桌面端會算出與 host 不同的 hash，然後卡在
「hash 不符 → 要 snapshot」的迴圈。已改成照著 ryu 的規則排版（`formatDouble`）。

`combining-marks-not-unicode-collation` 則是**驗證這組向量真的咬得到人**時補上的：
把 `CharacterSemantic.swift` 的 `Array(key.utf8).lexicographicallyPrecedes` 換成
`map.keys.sorted()`，原本那 11 筆向量三端照樣全綠——Swift 的 `String` `<` 對它們剛好與
位元組序一致。加上這一筆之後，同樣的改動會讓 Swift 端紅（實測 2 個 assertion 失敗）。

**刻意不涵蓋的邊界**（不是疏漏，是三端解析層本來就對不齊的地方）：

* **整數字面只到 ±2^53**。TypeScript 端用 `JSON.parse` 讀 manifest，超過就在解析那一步
  失真了，canonical 層救不回來。
* **`-0`（整數負零）**。serde_json 讀成 `0`、寫成 `0`；JS 的 `JSON.parse("-0")` 是 `-0`。
  TS 端的 `canonicalNumber` 在整數路徑上主動抹平成 `0` 以對齊 Rust，但 fixture 不收這個
  字面（host 不會寫出它）。`-0.0`（f64）**有**涵蓋——那是 host 真的寫得出來的。
* **`1E3`／`0.10` 這類非正規字面**。Swift 端逐字保留原字面，Rust 會正規化成 `1000.0`／
  `0.1`。manifest 裡的字面一律由 serde_json 寫出，所以三端看到的永遠是正規形。
* manifest 檔案裡 U+2028／U+2029／U+007F／U+0301 是以 `\uXXXX` 寫的（同一個 JSON 值）：這份檔案會被
  `scripts/aip-codegen.mjs` 內嵌進 Swift 原始碼的 raw string，裸的 U+2028 會被 Swift 的
  lexer 當成換行、裸的組合附加符號會黏到前一個原始碼字元上。Swift 端另有一條測試斷言解析回來之後**必須**是那個字元本身。

## 4. 加一則 fixture

1. 把 envelope JSON 放進 `crates/interaction-aip/tests/fixtures/`（超過 32 KiB 的請改用 `generated` 區段）。
2. 在 `manifest.json` 的 `envelopes` 加一筆，寫清楚 `expect` 與（若是 error）`code`。
3. `cargo test -p interaction-aip` → 先看 Rust 的結論是否符合預期。
4. `pnpm aip:codegen`（重生 `AIPFixtures.swift`，把新檔內嵌進去）。
5. TypeScript 與 Swift 測試重跑；三邊都綠才算加完。

**加一筆 canonical 向量**（`canonicalVectors`）走的是另一條路——它沒有獨立檔案，
期望值也不手寫：

1. 在 `crates/interaction-aip/tests/canonical_vectors.rs` 的 `vectors()` 加一筆
   `Vector { id, note, input }`（`input` 必須是 JSON 物件）。
2. `AIP_UPDATE_FIXTURES=1 cargo test -p interaction-aip --test canonical_vectors` 重生
   manifest 的 `canonicalVectors` 段（`canonical`／`sha256`／`doublePaths` 都是推導出來的）。
3. `pnpm aip:codegen` 重生 `AIPFixtures.swift`。
4. TypeScript 與 Swift 測試重跑。**對不上就修那一端的實作，不要改向量**。

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
