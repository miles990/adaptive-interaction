# Adaptive Interaction Protocol（AIP）1.0 — Canonical Contract

> 這是 **唯一** 的跨裝置語意訊息契約。Rust crate `crates/interaction-aip` 是權威實作與 JSON Schema 來源
> （golden：`schemas/aip-1.0.schema.json`）；TypeScript（`apps/interaction-desktop/src/aip/generated.ts`）與
> Swift（`apps/interaction-ios/InteractionCompanion/Models/AIPGenerated.swift`）由 `scripts/aip-codegen.mjs`
> 從同一份 schema **產生**，CI 以 drift check 擋住手改。任何 Transport（iPhone wss、HTTP、SSE、Tauri IPC、
> in-process）都只是 AIP envelope 的**綁定**；framing、重連、退避細節屬各 Transport，AIP 不統一它們。
>
> AIP 與既有契約的關係：
> - **Character Presentation Protocol 1.0（CPP，`docs/character-protocol/README.md`）**：Renderer 契約，**不變**。
>   AIP 的 Behavior Intent 由 Runtime 端 `CppRendererAdapter` 投影成 CPP `IntentEnvelope`；CPP wire 訊息不改語意。
> - **iPhone 線協定 v1（`mobile.rs`／`Protocol.swift`）**：Transport 綁定，**保留全部既有訊息**；新增一種 frame
>   `{"type":"aip","envelope":{…}}` 承載 AIP（§9.1）。舊 App 遇到未知 type 只記錄不執行（既有行為）。
> - **Runtime 事件（`EventType`）**：Runtime 內部真相流，不變；Character Session 把其中的真相變化轉成 AIP `event`。

## 0. 設計原則

1. **語意優先**：AIP 同步「發生了什麼」「角色現在是什麼語意狀態」「角色想表達什麼」「哪些能力可用」，
   不同步 frame、粒子、座標、FPS、原始感測資料。
2. **只共用穩定語意**：envelope、版本、錯誤、身分參照、correlation、sequence／revision、能力協商、
   cancel／deadline、證據分類、package lifecycle 共用；Transport、identity 驗證法、payload schema、重試、
   驗證規則、discovery、framing、平台限制、安全強度各 Adapter 自留。
3. **誠實階梯**：`received ≠ accepted ≠ acknowledged ≠ applied ≠ observed ≠ claimed-completed ≠ verified`；
   單一 `success: true` 不存在。
4. **宣稱不是身分**：`source` 只是宣稱；可信身分由 Transport／配對綁定，宣稱不符一律拒絕或正規化並留稽核。
5. **未知不執行**：未知 message type／name／capability 回 `unsupported-*`，不猜、不執行；未知**選填**欄位保留並忽略。
6. **有界**：訊息大小、payload 大小、字串長度、巢狀深度、去重環、事件日誌、佇列全部有上限。

## 1. Envelope

```jsonc
{
  "specVersion": "aip/1.0",                 // 必填；major.minor（§4）
  "messageId": "msg_01J…",                  // 必填；每個 source 內唯一；去重鍵（≤128 字）
  "messageType": "event",                   // 必填；§2 十二種之一
  "name": "character.interaction.touch",    // 必填；點分小寫語意名（^[a-z][a-z0-9]*(\.[a-z][a-z0-9-]*)+$，≤128）
  "source": { "kind": "device", "id": "iphone-87b42264" },   // 必填；宣稱（§5）
  "target": { "kind": "session", "id": "session.home" },     // 選填；command／query／state 必填
  "sessionId": "session.home",              // 選填；Character Session 訊息必填
  "occurredAt": "2026-09-04T12:30:00.000Z", // 必填；RFC 3339；只作資訊，host 以自己時鐘為準
  "correlationId": "flow_…",                // 選填；同一個 flow（工作、互動）的所有訊息共用
  "causationId": "msg_…",                   // 選填；直接導致本訊息的那一則 messageId
  "sequence": 201,                          // 選填；host 送出的 session 訊息必填（§6）
  "baseRevision": 200,                      // 選填；state patch 必填（§6）
  "expiresAt": "2026-09-04T12:30:05.000Z",  // 選填；event／command 的 deadline（§7）
  "consentGrantId": "grant_…",              // 選填；需要授權的 command 帶
  "payload": { }                            // 必填（可為空物件）；依 messageType＋name 的 payload schema
}
```

- 未列出的**頂層選填欄位**：接收端必須保留（round-trip 不遺失）且忽略；不得因此拒絕。
- **必填欄位**依 message profile 決定（§2.2）；`Envelope::validate(profile)` 是唯一的必填檢查點。

## 2. Message types

### 2.1 十二種

| messageType | 方向 | 語意 | 回應 |
|---|---|---|---|
| `event` | 任意→host | 已發生的語意事實（互動、真相變化） | `result`（accepted／rejected／expired／duplicate） |
| `command` | host→renderer／device | 請對方做某件事（Behavior Intent、cancel 以外的指令） | `result`（accepted→…→observed／failed） |
| `query` | 任意→host | 讀取（snapshot、resume、diagnostics） | `response` |
| `response` | host→發問者 | query 的答案 | — |
| `result` | 執行方→發起方 | event／command 的處理結果（Outcome，§3） | — |
| `state` | host→members | 權威狀態 snapshot 或 patch（§6） | 選填 `result{applied}` |
| `cancel` | 任意 | 取消某個 command／flow（`correlationId` 或 payload.messageId） | `result{cancel-requested→cancel-confirmed}` |
| `approval-request` | host→human surface | 需要人類決定（不由 AI 或 adapter 回答） | `approval-result` |
| `approval-result` | human surface→host | 人類決定（approved／denied／expired） | — |
| `error` | 任意 | 無法處理某則訊息（`causationId` 指向它） | — |
| `heartbeat` | 任意 | 存活與 presence（可帶 lastSequence／lastRevision） | 選填 heartbeat |
| `capability` | 任意 | 能力宣告／協商結果（§4.2） | `capability`（negotiated） |

### 2.2 Message profiles（必填欄位）

| profile | 必填（除共同必填 specVersion／messageId／messageType／name／source／occurredAt／payload 外） |
|---|---|
| `event` | `sessionId`；建議 `expiresAt`（互動事件必填，見 §7） |
| `command` | `sessionId`、`target`、`correlationId`、`expiresAt` |
| `query` | `target` |
| `response` | `causationId`（＝query 的 messageId） |
| `result` | `causationId`（＝被處理訊息的 messageId）、`payload.status` |
| `state` | `sessionId`、`sequence`、`payload.revision`；patch 另需 `baseRevision` |
| `cancel` | `causationId` 或 `payload.messageId`，二擇一 |
| `approval-request` | `correlationId`、`expiresAt`、`target{kind:human}` |
| `approval-result` | `causationId` |
| `error` | `payload.code`；`causationId`（若可對應） |
| `heartbeat` | 無 |
| `capability` | 無（payload 見 §4.2） |

### 2.3 Name 命名空間（1.0 保留）

```
character.interaction.*   使用者對角色的互動事實（touch／dismiss／…）
character.behavior.*      Behavior Intent（request／cancel）
character.session.*       Session 生命週期（join／leave／presence／snapshot／resume／patch）
task.*                    工作真相（state／verified）——只有 Runtime 可送
runtime.*                 Runtime 真相（emergency）——只有 Runtime 可送
device.*                  裝置能力／狀態（保留給 Device Profile）
```

## 3. Outcome（成功狀態不是一個布林）

`result.payload.status` ∈

```
received  accepted  acknowledged  applied  observed  claimed-completed  verified
rejected  expired  cancel-requested  cancel-confirmed  failed
```

各 profile 只用自己適合的子集：

| 被處理訊息 | 合法 status |
|---|---|
| `event` | `accepted`（已驗證並排入 sequence）→ `applied`（權威狀態已改）；或 `rejected`／`expired`；重複 messageId 回 `accepted` 並帶 `payload.duplicate: true`（不重套用） |
| `command`（Behavior Intent） | `accepted` → `acknowledged`（renderer 收到但不會回報完成）或 `observed`（renderer 已呈現於使用者眼前）；`rejected`／`expired`／`failed`／`cancel-confirmed` |
| `state` | `applied`／`rejected{code: revision-mismatch}` |
| `cancel` | `cancel-requested` → `cancel-confirmed`；已終態者回 `cancel-confirmed{alreadyTerminal:true}` |

不變量：
- `verified` 只能由 Runtime 的人類驗證路徑產生；任何 adapter／device／renderer 送來的 `verified` 一律
  `rejected{code: scope-denied}` 並稽核。
- `claimed-completed` 永遠不等於 `verified`；`observed`（呈現完成）永遠不等於工作 `verified`。
- 通知送達（`acknowledged`）不等於動畫 `observed`。

## 4. 版本與協商

### 4.1 規則

- `specVersion` = `aip/<major>.<minor>`。major 不同 → 拒絕（`unsupported-version`），不猜。
- minor 只能**向後相容新增**（新選填欄位、新 name、新 capability、新 outcome 值只在新 profile 使用）；
  patch 不改 schema 語意。
- 既有欄位語意不得悄悄改變；要改就是新 major。
- 未知 message type → `error{code: unsupported-message-type}`，**不執行**。
- 未知 name（在已知 type 內）→ `result{status: rejected, code: unknown-name}`，不執行。
- 未知 capability → 協商結果標 `unsupported`。
- Deprecation：欄位／name／capability 刪除前先標 `deprecated`，至少跨一個公開 minor 版本，提供 compatibility
  adapter 與 diagnostics warning（`aip.deprecated-used`），更新相容矩陣；舊 Character Package／舊 iPhone App 不得因此無法啟動。
- Migration：`docs/aip/compatibility.md` 記錄每個 minor 的新增、每個 deprecated 的移除期。

### 4.2 Capability 協商

`capability` payload（宣告）：

```jsonc
{
  "specVersions": ["aip/1.0"],
  "role": "remote-renderer",              // host-renderer | remote-renderer | input-device | observer
  "profiles": ["character-session"],       // 支援的 profile
  "syncClasses": ["semantic"],             // semantic | timeline | realtime（§8）
  "intents": ["react-happily-to-touch", "celebrate", "idle"],   // 可呈現的 Behavior Intent
  "inputs": ["character.interaction.touch"],                     // 可產生的 event name
  "features": { "haptic": false, "reducedMotion": true },        // 選填、自由鍵；未知鍵保留
  "limits": { "maxMessageBytes": 65536 }
}
```

host 回 `capability`（negotiated）：`{ specVersion, role, syncClass, intents:{ "<intent>": "exact"|"unsupported" },
inputs:[…accepted…], limits }`。協商是確定性的：交集＋min。renderer 對 `unsupported` intent 的降級由自己決定
（不播、播 idle、或純文字），但不得偽稱 `observed`。

## 5. 身分與安全

- `source` 是宣稱。host 在 Transport 層取得**已驗證身分**（iPhone：配對 token→deviceId；桌面視窗：human token
  的可信 host surface；Runtime 內部：`runtime`），與 `source` 比對：
  - 相符 → 通過；
  - `source.kind` 相符但 `id` 不符、或 kind 不符 → `error{code: identity-mismatch}`＋稽核 `aip.identity-mismatch`，**不執行**；
    host 不得「幫忙修正」後執行（正規化只用於 host 自己產生的訊息）。
- 每則外部輸入都經：schema 驗證 → 大小上限 → deadline → 身分綁定 → session membership → scope／capability
  （這個 source 有沒有宣告能送這個 name）→ rate limit → replay／dedupe → 才進 session。
- 不同身分不得沿用舊配對；同一可信身分的 endpoint 改變（IP／port）是 Transport 的事，AIP 只看綁定後的 Party。
- 原始平台錯誤不得暴露 token、路徑、憑證、裝置識別資訊；`error.payload.message` ≤ 200 字、不含輸入回顯。

## 6. Sequence、Revision、Snapshot、Patch、Replay

- **sequence**：host 對每個 session 送出的 `state`／`command`／`event`（轉發）遞增 u64，從 1 起；member 以
  `lastSequence` 偵測缺漏（gap）。
- **revision**：權威狀態的單調版本；只有 session host 遞增；每次成功 `applied` +1。
- **snapshot**：`state{payload:{kind:"snapshot", revision, sequence, state, hash}}`；`hash` = SHA-256（canonical JSON，
  鍵排序、無空白）of `state`。
- **patch**：`state{baseRevision, payload:{kind:"patch", revision, patch, hash}}`；`patch` 為 RFC 7396 JSON Merge Patch；
  接收端 `localRevision != baseRevision` → **不得套用**，改送 `query character.session.resume`。
- **resume**：`query{name:"character.session.resume", payload:{lastRevision, lastSequence}}` → host 若日誌內有
  `lastRevision+1..=current` 的 patch 就回 `response{payload:{kind:"patches", patches:[…]}}`，否則回 `kind:"snapshot"`。
  日誌是有界環（預設 512 筆），超出即 snapshot fallback，不是錯誤。
- **hash 驗證**：套用 patch 後本地 hash 必須等於 `hash`，否則丟棄本地狀態並要 snapshot。
- **持久化不是 wire**：host 本機快照檔的格式版本（`interaction_session::SNAPSHOT_FORMAT`）與這裡的 `specVersion`
  完全分開演進，`hash` 只涵蓋 `state`；改檔案佈局不是 AIP 版本變更（見 `character-session.md` §6）。
- **rollback 防護**：`state.revision` 小於或等於本地已套用 revision 的訊息一律忽略（稽核 `aip.state-rollback-ignored`），
  除非 host 明確說出理由（見下面的 `reason` 清單）。
- **`payload.reason`（1.0 的兩個值；未知值一律當成沒有 reason，不得給任何特權）**：

  | 值 | 意義 | epoch | 接收端 |
  |---|---|---|---|
  | `session-reset` | host 重建了 session | 與本地**不同**（重灌後可能從 1 重新起跳，所以是「不同」不是「大於」） | 丟棄本地狀態、套用這一份 |
  | `recovery` | 同一個 session，host 的權威狀態真的比對方記得的舊（從較舊快照還原） | **不變** | 套用並退回 host 的 revision，稽核 `aip.state-recovered` |

  `recovery` 是 **AIP 1.0 接收端澄清（2026-09-05，v0.7.0）** 新增的值：wire 形狀與 `specVersion` 都不變，
  只認得舊值的接收端把它當成沒有 reason 的 snapshot（行為與今天相同）。完整的接收端決策表（連線世代、身分、
  hash 核對、resume 逐則規則、有界 realign）在 `docs/aip/character-session.md` §7.2，跨語言 fixture 是
  `manifest.json` 的 `receiveDecisions` 段。

## 7. Deadline、Cancel、Idempotency

- `expiresAt`：host 在**收到時**與**套用時**各檢查一次；過期 → `result{status: expired}`，不執行。互動事件
  （`character.interaction.*`）必填，建議 5 s；Behavior Intent 必填，建議 ≤ 10 s。
- 重連後 host **不重播** 過期或 `drop-if-offline` 類的事件與 intent（§8 離線政策）；只 reconcile 狀態。
- `cancel`：冪等；重複 cancel 回同一結果；對已終態回 `cancel-confirmed{alreadyTerminal:true}`。
- 去重：host 對每個 (session, source) 保留 256 筆 messageId 環；重複回 `accepted{duplicate:true}` 不重套用；
  command 的重複永不重執行。超出環的重放靠 `expiresAt`＋`sequence` 上限擋（舊事件必然過期）。
- 重試：只有 `retryable: true` 的 error 允許重送**同一 messageId**（idempotent）；其他情況產生新 messageId。

## 8. 離線事件政策（Offline policy）

每個 name 固定歸類（實作在 `interaction_aip::offline_policy(name)`）：

| class | 意義 | 1.0 歸類 |
|---|---|---|
| `drop-if-offline` | 對方不在線就丟，永不排隊 | `character.behavior.request`、`character.interaction.dismiss`、`heartbeat` |
| `expire-by-deadline` | 可短暫排隊，過 `expiresAt` 即丟 | `character.interaction.touch` |
| `queue-idempotent` | 可離線排隊，重送安全 | `character.preference.*`（保留） |
| `require-reconfirmation` | 離線後不得自動重送，需人類再確認 | `approval-request`、任何帶 `consentGrantId` 的 command |
| `state-reconcile` | 不重播事件，以最新 snapshot／patch 對齊 | `task.*`、`runtime.*`、`character.session.*`、`state` |

不得讓幾分鐘前的觸摸事件在重連後連續播放：touch 是 `expire-by-deadline`，intent 是 `drop-if-offline`。

**這張表與 `offline_policy()` 的真實關係（誠實敘述）**：Character Session **不呼叫** `offline_policy()`——
它把同一組語意直接寫進了自己的機制裡，因此不需要查表：

- 互動事件靠 **deadline**（§8 第 11 關把 `expiresAt` 夾成 `min(自報, occurredAt + touchTtlMs)`），
  過期就是 `expired`，這就是 `expire-by-deadline`。
- Behavior Intent **不排隊**：只送給 presence 為 `online`、且把該 intent 協商成 `exact` 的成員，
  沒有人符合就計 `intents.dropped`，這就是 `drop-if-offline`。
- 狀態靠 **snapshot／patch 對齊**（§6 的 revision／epoch 規則與 `character.session.resume`），
  從不重播事件，這就是 `state-reconcile`。
- `require-reconfirmation` 在 session 裡是更嚴格的形式：inbound 訊息只要帶 `consentGrantId` 就
  `scope-denied`（§8 第 8.1 關），連問驗證器都不問。

所以 `offline_policy()` 目前是一個**給 Transport／UI／文件用的分類函式**，
**沒有 production 呼叫者**（只有測試與跨語言 conformance fixture 用它）。它存在的價值是把
「哪些 name 屬於哪一類」寫成三個語言都對得起來的可執行事實；把它說成「session 用它決定重連行為」
會是一句程式碼背不起來的話。要接線的話，該接的地方是 Transport 層的離線佇列——1.0 沒有那個佇列。

## 9. Transport bindings（1.0）

| Transport | 綁定 | 身分 | 狀態 |
|---|---|---|---|
| iPhone wss v1 | `{"type":"aip","envelope":{…}}`；只在 `auth-ok` 之後；每則 ≤ 64 KiB；共用 v1 的 30 msg/s、128 KiB frame 上限與 8 連線上限 | 配對 token → `{kind:"device", id:<deviceId>}` | v0.6.0 實作 |
| HTTP（human token） | `GET /v1/character-session`（snapshot）、`POST /v1/character-session/resume`、`POST /v1/character-session/events`（可信 host surface 送 event）、`GET /v1/character-session/diagnostics` | human token → `{kind:"human-surface", id:"desktop"}` | v0.6.0 實作 |
| SSE | `character.session.state` 事件，payload 是完整 AIP `state` envelope | human token | v0.6.0 實作 |
| Tauri IPC | 內嵌模式同 HTTP 語意（`character_session_*` 指令） | 可信 host | v0.6.0 實作 |
| In-process（Rust） | `CharacterSessionHost` 方法呼叫 | `{kind:"runtime"}` | v0.6.0 實作 |
| CPP WebSocket adapter | **不**承載 AIP；外部 renderer 仍走 CPP wire（Behavior Intent 已投影成 CPP intent） | adapter token | 不變 |

### 9.1 iPhone frame 細節

- App 在 `auth-ok` 後送 `capability`；host 回 `capability`（negotiated）＋`state{kind:snapshot}`。
- 沒送 `capability` 的 App（舊版）永遠不會收到任何 `aip` frame；`character.present` 動器路徑照舊。
- 已協商的 App 仍會收到 `character.present`（那是受 governor 管的動器），但以 session `state` 為權威；兩者衝突以
  `state` 為準（`character.present` 只作 legacy hint）。

## 10. 證據分類（Evidence classification）

`EvidenceClass` ∈ `unit | contract | fixture | simulator | integration | browser | real-agent | real-device |
real-hardware | unverified`。**1.0 只定義詞彙，尚未接進 diagnostics（implemented-unverified）**：這個列舉
目前只存在於 Rust 型別、golden schema 與 codegen 產物中，Runtime／API／CLI／桌面 TS／iOS App 都沒有
生產端或消費端，`GET /v1/character-session/diagnostics` 的回傳裡也沒有證據等級欄位。

因此 **fixture／simulator 永遠不得標成 real-device** 這條不變量，目前是**由人工文件紀律維持的**——
`docs/releases/*-test-matrix.md`、`acceptance-evidence.md` 與 CHANGELOG 逐列標註，沒有任何程式在執行期
強制它。要把它變成機制，得由 host 依 transport／配對事實決定每個成員的來源等級並帶進 diagnostics
（裝置自報不算數），再補一則「fixture 來源不得被標成 real-device」的測試；在那之前，讀到
`EvidenceClass` 的人請把它當成**待接線的詞彙表**，不是既有的執行期保證。

Runtime 的 `ProviderTested`／`pairingUnverified` 語意不變，AIP 只提供統一詞彙。

## 11. Limits（1.0 常數，`interaction_aip::limits`）

| 常數 | 值 |
|---|---|
| `MAX_MESSAGE_BYTES` | 65 536 |
| `MAX_PAYLOAD_BYTES` | 32 768 |
| `MAX_ID_CHARS`／`MAX_NAME_CHARS` | 128 |
| `MAX_STRING_CHARS`（payload 內任一字串） | 2 000 |
| `MAX_JSON_DEPTH` | 8 |
| `DEDUPE_RING` | 256 |
| `EVENT_LOG_RING` | 512 |
| `MAX_CLOCK_SKEW_MS`（occurredAt 與 host 時鐘） | 30 000（超出只稽核，不拒絕） |
| `DEFAULT_INTERACTION_TTL_MS` | 5 000 |
| `DEFAULT_INTENT_TTL_MS` | 10 000 |
| `MAX_MEMBERS` | 16 |
| `MAX_UNSUPPORTED_INPUTS` | 32（協商結果 `unsupportedInputs` 的有界性要求：對方宣告的 `inputs` 本身無界，host 的協商回覆是一則要送上線的訊息，不截斷會超過 `MAX_PAYLOAD_BYTES`；它不是 wire 欄位長度上限，但仍發布進 golden schema 的 `limits` 表，讓三端的 `negotiate` 有同一個截斷點） |
| `MAX_RESUME_PATCHES` | 512（＝`EVENT_LOG_RING`：一則 resume 回覆最多幾則 patch。超過**不得**靜默截斷，改走 realign） |
| `MAX_REALIGN_ATTEMPTS` | 3（連續幾次未能套用就是 unrecoverable；任一次 apply／reset／recover 清零） |

golden schema `schemas/aip-1.0.schema.json` 的 `limits` 表由 `interaction_aip::limits` 的**每一個** `pub const`
產生（`schema.rs::every_limit_constant_is_published_in_the_schema` 雙向比對），TS／Swift 的 `AIP_LIMITS` 由
codegen 產出，不再有手寫的同值字面量。

## 12. 穩定錯誤碼（`ErrorCode`）

```
schema-invalid  unsupported-version  unsupported-message-type  unknown-name  unsupported-capability
payload-too-large  message-too-large  expired  duplicate  revision-mismatch  sequence-gap  identity-mismatch
not-a-member  scope-denied  rate-limited  session-not-found  session-disabled  cancelled  internal
```

`session-disabled`（19 個之一）：`INTERACT_AI_CHARACTER_SESSION=0` 時所有 Session 入口回它（HTTP 503），
`retryable: false`；見 `docs/aip/architecture-boundaries.md` §5。

`error.payload`：`{ code, message(≤200), retryable: bool, details?: object }`。`details` 不得含 secret／路徑／token。

## 13. 與既有訊息的相容對照（資訊性，不改既有語意）

| 既有 | AIP 對應 | 說明 |
|---|---|---|
| CPP `hello`／`negotiate`／`negotiated` | `capability` | CPP 保留自己的握手；概念等價 |
| CPP `intent{envelope}` | `command{name:"character.behavior.request"}` | Session intent → CPP 投影（`CppRendererAdapter`） |
| CPP `receipt{status}` | `result{status}` | `accepted→accepted`、`started→acknowledged`、`completed→observed`、`cancelled→cancel-confirmed`、`expired→expired`、`unsupported→rejected{unsupported-capability}`、`failed→failed`、`uncertain→failed{code:internal, retryable:false}`（uncertain 是誠實的未知，不升級） |
| CPP `event{kind:character.clicked}` | `event{name:"character.interaction.touch", payload.kind:"tap"}` | 桌面可信 surface 的點擊可進 session（v0.6.0 以 `/v1/character-session/events` 送） |
| iPhone v1 `observation{receptor:"iphone.touch"}` | `event{name:"character.interaction.touch"}` | 舊 App 仍送 observation；host 為已協商 App 把 AIP touch 也落成同一個 receptor observation（recipe 相容） |
| iPhone v1 `act{name:"character.present"}` | `state`（truth 部分） | 舊路徑保留；新路徑以 state 為權威 |
| Runtime `EventType::AgentSessionState{verified}` | `event{name:"task.verified"}`（source runtime） | 只有 Runtime 可產生 |
| Runtime `EventType::EmergencyStop` | `event{name:"runtime.emergency"}` | 只有 Runtime 可產生 |

## 14. Conformance tests（必須存在）

Rust（`crates/interaction-aip/tests/conformance.rs`）＋ TS（`src/test/aip-conformance.test.ts`）＋ Swift
（`AIPConformanceTests.swift`）對**同一組 fixture**（`crates/interaction-aip/tests/fixtures/*.json`）：

valid envelope（每種 type）／invalid（缺必填、壞 name、壞時間）／version negotiation（同 minor、舊 minor、新 minor、
不同 major）／unknown optional field round-trip／unknown message type／oversized（message、payload、深度、字串）／
round-trip 穩定（canonical JSON）／golden schema 不漂移／generated type 不漂移／stable error codes／deadline
（過期＝expired）／cancel correlation／duplicate messageId／identity-mismatch 決策表／offline policy 表。

manifest 另有三段不是 envelope、但同樣必須三端一致（`docs/aip/conformance.md` §3 有每一段的欄位表）：

| 段 | 內容 | 誰讀它 |
|---|---|---|
| `stateHashes` | host 真實寫出的 `SemanticState` 與其 canonical 文字／SHA-256 | Rust `conformance.rs`＋`state_hash_fixtures.rs`／TS `canonical-hash.test.ts`／Swift `StateHashConformanceTests` |
| `stateHashDoublePaths` | `SemanticState` 的 f64 欄位（schemars 推導） | TS `SEMANTIC_STATE_DOUBLE_PATHS`（codegen 產出） |
| `receiveDecisions` | §6／`character-session.md` §7.2 的接收端決策表案例 | Rust `receive_decision_fixtures.rs`（產生器）＋`receive_decisions_from_json.rs`（獨立消費者）＋`conformance.rs`（形狀） |
