# AIP Character Session：iPhone Companion（手機端）

> 這份文件寫**手機端實作出來的形狀**：它在 session 裡是什麼角色、送什麼、收什麼、
> 怎麼呈現、什麼時候誠實降級。契約在 `docs/aip/README.md`（envelope／版本／outcome）與
> `docs/aip/character-session.md`（session 協定、state ownership、§11 人話文案）；
> 線上綁定在 `docs/aip/transport-bindings.md` §1（iPhone wss frame）。
>
> 權威實作：`apps/interaction-ios/InteractionCompanion/Services/SessionClient.swift`
> （決策全部是純函式）＋`Models/CharacterSemantic.swift`（語意狀態鏡射、RFC 7396 merge
> patch、canonical hash）＋`Views/CharacterView.swift`（呈現與本地動畫）。
> 測試：`InteractionCompanionTests/SessionClientTests.swift`、`ProtocolTests.swift`。

## 1. 角色與能力宣告

手機是 **`remote-renderer`**：只送語意事件、只收語意狀態與 Behavior Intent，
**不擁有**任何共享狀態（`character-session.md` §2）。

`auth-ok` 之後送出的 capability 宣告內容固定（golden test
`testCapabilityAnnouncementIsExactlyThisShape` 釘住）：

```jsonc
{
  "features":     { "haptic": false, "reducedMotion": <UIAccessibility.isReduceMotionEnabled> },
  "inputs":       ["character.interaction.touch", "character.interaction.dismiss"],
  "intents":      ["react-happily-to-touch", "celebrate", "settle", "idle"],
  "limits":       { "maxMessageBytes": 65536 },
  "profiles":     ["character-session"],
  "role":         "remote-renderer",
  "specVersions": ["aip/1.0"],
  "syncClasses":  ["semantic"]
}
```

* `haptic` **永遠是 `false`**：震動只走受 Policy Governor 管的 `haptic.pulse` 動器路徑。
  Behavior Intent 是呈現語意，不得自己讓手機震動——所以這裡誠實宣告「這條路上沒有 haptic」。
* `reducedMotion` 是使用者的系統設定，不是 App 的偏好；每次協商時重讀。
* 只宣告能**真的做到**的 intent。1.0 的四個全部支援，因此正常情況下
  `unsupportedIntents` 是空的；桌面若送來第五種，手機回
  `result{status:"rejected", code:"unsupported-capability"}`，**絕不**回 `observed`。

## 2. 生命週期

| 時機 | 手機做什麼 |
|---|---|
| `auth-ok`（首次配對或重連） | 送 `capability`。本地已有狀態（曾經對齊過）時，另外送 `query character.session.resume{lastRevision, lastSequence, sessionEpoch}` |
| 收到 `capability`（negotiated） | 記下 `intents` 裡被標 `unsupported` 的名字；`newerMinor` 只記錄，不改行為 |
| 收到 `state{kind:"snapshot"}` | 整份套用，記下 revision／sessionEpoch／sequence |
| 收到 `state{kind:"patch"}` | `baseRevision == 本地 revision` 才套用；套用後核對 `hash` |
| 收到 `response`（resume） | `kind:"patches"` 依序套用；`kind:"snapshot"` 整份取代。空的 patches＝已經對齊，不是錯誤 |
| 收到 `command character.behavior.request` | 見 §4 |
| 收到 `command character.behavior.cancel` | 取消對應 intent（冪等），回 `result{cancel-confirmed}` |
| 收到 `result{rejected, not-a-member}` | 逾時被清出成員：重新協商（重送 capability） |
| 收到 `error{session-disabled｜unsupported-capability}` | 這台桌面沒開角色同步：停用同步、顯示人話，不重試轟炸 |
| 斷線 | 協商狀態清空、待播佇列清空；**不重播**任何互動事件或 intent（AIP §8） |
| 進背景（`scenePhase == .background`） | 停 status 心跳、本地 presence 標成 background（連線頁診斷區顯示「背景（心跳已停；桌面最遲 45 秒後會把這台裝置標成離線）」）；**不**假設 socket 會活著、不在背景重連（App 刻意沒有 Background Mode）；**AIP 出站一律不送**（capability／resume／snapshot query／result／互動事件都算），擋下時計入 `droppedFrames` 並留一行說明——桌面把任何一則通過身分綁定的 inbound envelope 都當成存活證明（`session.rs` gate 4.1 → `note_alive` → `Presence::Online`），只擋 legacy `status` 而放行 AIP，桌面就會顯示 online、與手機自己畫面上的「背景」互相矛盾（純函式 `LifecycleDecision.shouldSendCharacterSync(phase:)`） |
| 回前景（`.active`） | 立刻補一則 `status`、重啟心跳；socket 還活著且背景 ≥ 1 秒 → 送一次 `query character.session.resume`（只 reconcile，不重播）；socket 已死且使用者仍想連線＋有配對 → 跳過退避立即重連（使用者按過中斷／已撤銷／無配對則不動）。決策是純函式 `LifecycleDecision.on`／`shouldReconnectImmediately`。已連線但**尚未協商**（`auth-ok` 是在背景才到的，capability 被上一列的閘門擋下）→ 回前景補送一次 capability，否則沒有人會再送第二次。回前景的 resume 在上一則仍等回覆的 10 s 寬限窗內**不重送**（`SessionDecisions.shouldResendResumeOnForeground`；桌面真的送來對不上的狀態仍會重問）；背景中既有的斷線重試與心跳也被 `LifecycleDecision.shouldScheduleReconnect`／`shouldSendPresenceHeartbeat` 閘住，進背景取消等待中的重試（`LifecycleTests` 22 支，模擬器；ConnectionManager 的接線只有 typecheck 背書） |
| `.inactive`（通知中心、切換器預覽、來電橫幅） | 完全不動（常直接回 `.active`） |
| 收到 AIP `heartbeat` | 計數＋note，5 秒節流回一則 legacy `status`（本版不送 AIP heartbeat） |

**重連不重播**：touch 是 `expire-by-deadline`（5 秒後自然過期）、intent 是
`drop-if-offline`。重連只 reconcile 狀態，幾分鐘前的觸碰不會突然連播。

## 3. 接收端的狀態規則（AIP §6）

`SessionDecisions.apply` 是純函式，與 Rust `interaction_session::accept_state_with_epoch`
同一組規則：

| 情況 | 結果 |
|---|---|
| `revision < 本地` | 忽略（rollback 防護） |
| `revision == 本地` | 忽略（已套用過） |
| patch 的 `baseRevision != 本地 revision` | **不套用**，送 `resume` |
| patch 套用後 hash 與 host 的 `hash` 不符 | 丟棄本地狀態，送 `query character.session.snapshot` |
| snapshot 自己的 hash 對不上自己的內容 | 不執行（再要一次同一份只會無限迴圈） |
| snapshot 帶 `reason:"session-reset"` 且 `sessionEpoch` 更新 | 接受，即使 revision 比本地小（host 重建了 session） |
| 連續 3 次送出 resume／snapshot 仍未對齊 | 顯示「無法恢復，請重新連接」 |

### 3.1 為什麼 hash 需要「逐字保留的 JSON」

host 的 `hash` 是對 **serde_json 寫出來的文字**取 SHA-256（canonical JSON：鍵以 UTF-8
位元組序排序、無空白）。`mood.intensity` 是 `f64`，值為 0 時 serde_json 寫的是 **`0.0`**，
而一般 JSON 解析器讀進 `Double` 之後再寫出來會變成 `0`——canonical 文字不同，hash 就不同，
接收端會永遠「hash 不符 → 要 snapshot」。

實測（本機模擬器對真 daemon，`GET /v1/character-session`）：

```
{"activity":"idle",…,"mood":{"intensity":0.0,"kind":"neutral"},…}
sha256(以 0.0 計算) = host 回報的 hash        ← 一致
sha256(把 0.0 寫成 0) ≠ host 回報的 hash      ← 不一致
```

所以 `SemanticJSON` 把**數字一律以原始字面保存**（`case number(raw: String)`），
canonical 輸出時逐字寫回；字串與鍵則解碼後再用 serde_json 相容的規則重新跳脫
（只跳脫 `"`、`\`、控制字元；非 ASCII 與 `/` 原樣輸出）。

> **給其他語言的接收端**：任何用 `JSON.parse` / `JSONSerialization` / `json.loads`
> 讀進來再重新序列化的實作都有同一個坑。桌面（TypeScript）端也要保留數字字面，
> 或改成不在接收端驗 hash（那就失去「內容真的一致」的保證）。

## 4. Behavior Intent 的本地播放

1. 通過 `expiresAt`（過期 → `result{expired}`，不播）。
2. 通過去重（同一個 `messageId` 再來一次 → 不重播、不回第二次結果）。
3. 認得的 intent → 播本地動畫；認不得 → `result{rejected, unsupported-capability}`。
4. **動畫真的播完**之後才回 `result{status:"observed"}`。被新的可中斷 intent 蓋掉、
   或使用者離開頁面而沒播完，就**不回** `observed`（誠實階梯：呈現完成才是 observed）。

| intent | 本地呈現 | Reduced Motion 開啟時 |
|---|---|---|
| `react-happily-to-touch` | 一次縮放脈衝（幅度隨 `intensity`） | 不縮放，只換顏色 |
| `celebrate` | 色彩閃一次 | 同樣只換顏色（換色不是位移） |
| `settle`／`idle` | 回到靜止 | 同 |

待播佇列上限 **8**：滿了淘汰最舊的一個並誠實計數（drop-if-offline 語意，不會補播）。
去重環 256（AIP `DEDUPE_RING`）。

## 5. 觸摸與離開

| 情況 | 行為 |
|---|---|
| 已協商 | 送 AIP `event{name:"character.interaction.touch", payload:{kind:"tap"｜"longpress"}}`，`expiresAt = now + 5s`，`sessionId` 來自快照，`source = {kind:"device", id:<配對出來的 deviceId>}` |
| 未協商（舊桌面） | 走既有 wire protocol v1 的 `observation{receptor:"iphone.touch"}` |
| 未連線 | 丟棄並顯示「未連線，觸控事件未送出（已丟棄）」 |

兩條路**互斥**：已協商時只送 AIP 事件。host 會把 applied 的 AIP touch 另外落成
**恰好一筆** `iphone.touch` observation（recipe 相容），兩邊都送會變成算兩次。

離開角色頁時（已協商）送 `character.interaction.dismiss`——那是「使用者不再看著角色」
的語意事實，對應 §4 的 `activity → resting`。舊路徑沒有對應訊息，不硬造。

## 6. 呈現規則（誠實階梯）

`CharacterPresentation.resolve(session:negotiated:legacy:)` 是純函式，View 只負責畫：

* 已協商且有語意狀態 → 以語意狀態為準；舊的 `character.present` 只是 hint（AIP §9.1）。
* 未協商 → 完全退回既有的 `character.present` 路徑，不假裝有語意狀態。
* **緊急停止取兩條路徑的聯集**：語意狀態的 `truth = emergency` 或舊路徑的 `emergency`，
  任一成立就顯示固定文案「緊急停止中」。安全訊息只能加嚴，不能因為另一條路徑還沒更新
  就被淡化（`stop-all{reason:"emergency"}` 可能先到、`state` 後到）。
* **綠色勾號只在 `truth = verified`**。`claimed` 顯示「宣稱完成（尚未驗證）」。
* 未知的 mood／activity／truth 保留原字串、顯示「未知」，不猜、不美化。
* 成員數超過 AIP `MAX_MEMBERS` 的狀態整份拒絕，不截斷後假裝正常。

## 7. 同步狀態文案（`character-session.md` §11 的手機視角）

一般模式**只**顯示這一行人話，不得出現 revision／sequence／epoch／token 之類的技術詞
（`testSyncStatusCopyIsTheHumanWordingWithNoTechnicalTerms` 釘住）：

| 狀態 | 文案 |
|---|---|
| 未連線 | 「未連線，角色狀態可能不是最新的」 |
| 已連線但桌面沒有角色同步 | 「這台桌面尚未提供角色同步」 |
| 正常 | 「已連接桌面，角色狀態已同步」 |
| 協商後有 unsupported intent | 「部分能力目前不可用」 |
| 補齊中 | 「同步尚未完成」 |
| 連續補齊失敗 | 「無法恢復，請重新連接」 |

進階細節（revision／sequence／sessionEpoch、各種計數、同步記錄）只出現在
**「連線」頁 → 診斷 → 角色同步（進階）** 這個預設收合的折疊區。角色頁不顯示。

## 8. 有界

| 集合 | 上限 |
|---|---|
| 待播 Behavior Intent | 8（滿了淘汰最舊並計數） |
| 去重環 | 256（AIP `DEDUPE_RING`） |
| 同步記錄 | 50 行（只在本機） |
| resume 連敗計數 | 3 次即 `unrecoverable` |
| 單則 envelope | 64 KiB（AIP `MAX_MESSAGE_BYTES`）；送出前檢查，超過即丟棄並計數 |
| 送出佇列 | 沿用 `ConnectionManager` 既有的 64 則有界佇列，不另開旁路 |
| JSON 解析深度／輸入長度 | 32 層／128 KiB（`SemanticJSON`） |

## 9. 安全

* `source` 只是宣稱，一律填配對綁定出來的 `deviceId`；桌面會比對，不符即 `identity-mismatch`。
* 只接受 `source.kind` 是 `runtime`／`session` 的 host 訊息；`target` 指名別台裝置的一律忽略。
* 不合規（schema／profile／版本／上限）的訊息**不執行**，只計數與記錄。
* App **永遠不會**產生 `verified`：`SessionDecisions.resultEnvelope` 對 `verified` 直接回
  `nil`（host 端 gate 也會 `scope-denied`，這是自律的第一道）。
* 錯誤與記錄不回顯輸入內容、不含路徑或 token。

## 10. 驗證等級（誠實）

| 項目 | 等級 |
|---|---|
| 純決策（狀態規則、intent、capability、envelope 形狀、呈現投影、canonical hash） | **unit**：`SessionClientTests` 28 個測試，在 iPhone 17 **模擬器**內執行 |
| canonical hash 與 Rust 一致 | **contract**：直接對 `crates/interaction-aip/tests/fixtures/state-{snapshot,patch}.json` 的 hash 驗證，並確認 snapshot → patch 串接後仍等於 host 算出的 hash |
| 對真 daemon 的閉環（配對 → capability → snapshot → touch → intent → observed） | **simulator**：iPhone 17 模擬器 ＋ 真 `interact-ai` daemon（隔離 home、loopback、`INTERACT_AI_MOBILE_ADVERTISE=0`）。步驟與實際數字見 `apps/interaction-ios/README.md`「2026-09-04（v0.6.0 wave 2）」；截圖 `docs/assets/v06-evidence/ios-sim-character-session-*.png` |
| 真機（haptic／Reduced Motion 實機行為／背景重連） | **implemented-unverified**：真機安裝需要人類完成鑰匙圈與信任步驟，本輪未做 |

模擬器不是真機。凡是寫「模擬 iPhone（fixture）」的地方指的是
`cargo run -p interaction-runtime --example fake_iphone`，與這份文件講的**真 App 跑在模擬器上**
又是兩件事——後者跑的是同一份會裝進真機的 Swift 程式碼，只是硬體是模擬的。
