# AIP Character Profile：Character Session 協定、State Ownership、Snapshot／Replay

> 權威實作：`crates/interaction-session`（純函式，無 I/O；時間由呼叫端注入）；Runtime 接線：
> `crates/interaction-runtime/src/character_session.rs`（Session Host）。Envelope 與版本規則見 `docs/aip/README.md`。

## 1. 單一權威 Session Host

- 1.0 只有**一個**權威 `Character Session Host`，預設由桌面 Runtime 擔任；介面 `SessionHost` trait 不把「桌面」寫死。
- Session 身分：`sessionId`（預設 `session.home`）＋ `sessionEpoch`（host 每次重建 session 時 +1，持久化在 store meta）。
  重啟後 revision 從持久化 snapshot 續接（不歸零）；找不到 snapshot 才 epoch+1、revision 從 1 起。
- Host 職責：驗證事件、排 sequence、去重、deadline、consent／scope、更新權威狀態、產生 Behavior Intent、
  發 State Patch、提供 Snapshot／delta replay、join／leave／presence、capability 協商、diagnostics（不洩密）。
- 多台電腦競爭 host、雲端同步、multi-master、CRDT：**不在 1.0**；`SessionHost` 介面保留擴充點。

## 2. State Ownership（每個共享欄位只有一個 canonical owner）

| 狀態 | Canonical owner | 其他元件能做什麼 |
|---|---|---|
| 角色情緒 `mood{kind,intensity}` | Character Session Host（Director） | 送 event；不得直接改 |
| 角色活動 `activity` | Character Session Host | 同上 |
| 注意目標 `attention` | Character Session Host | 同上 |
| 真相 `truth{state, correlationId}` | Runtime（經 `task.*`／`runtime.*` event 進 Session；Session 只轉錄，不推論） | renderer／device 只能讀 |
| 共享玩具語意狀態 | Character Session Host（1.0 保留欄位，未實作） | — |
| 長期記憶 | Memory Provider（`memory.rs`，MemoryActor 規則） | Session 不碰 |
| Agent 工作狀態 | Agent Runtime（`agents.rs`） | Session 只收 `task.*` |
| Consent／single-use grant | Consent Service（`policy.rs`／`executor.rs`） | Session 只帶 `consentGrantId` 參照 |
| iPhone 原始感測資料 | iPhone 本機 | 只送語意事件 |
| 裝置 capability | 對應 Device Adapter（iPhone：MobileBridge；外部 renderer：CPP adapter） | Session 只存協商結果 |
| 畫面位置與縮放 | 各裝置本機 | 不同步 |
| 動畫 frame／particle | 各 Renderer 本機 | 不同步 |
| 裝置音量與顯示偏好 | 各裝置本機 | 不同步 |
| Session membership／presence | Character Session Host | member 送 heartbeat／join／leave |

程式內約束：`SemanticState` 只有 `CharacterSession::apply` 能改（欄位私有、只回傳 patch）；Renderer／Device port
沒有任何 setter。

## 3. 語意狀態（`SemanticState`）

```jsonc
{
  "characterId": "shu-maid",
  "mood": { "kind": "neutral", "intensity": 0.0 },   // neutral|happy|playful|proud|tired|alert|down
  "activity": "idle",                                 // idle|reacting|working|waiting|celebrating|resting|frozen
  "attention": { "kind": "none" },                    // none | {kind:"member", id} | {kind:"task", correlationId}
  "truth": { "state": "none", "correlationId": null },// CPP TruthState 詞彙（none…verified…emergency…）
  "lastInteraction": { "name": "character.interaction.touch", "kind": "tap", "source": "device:iphone-…", "at": "…" },
  "members": [ { "party": {"kind":"device","id":"…"}, "role": "remote-renderer", "presence": "online",
                 "lastSeenAt": "…", "unsupportedIntents": [] } ],   // 協商為 unsupported 的 intent 名；沒有就是空陣列
  "reducedMotion": false
}
```

字串長度、成員數（≤ 16）、巢狀深度受 AIP limits 約束。

`members[].unsupportedIntents` 是**協商結果裡唯一被投影出去的欄位**：其餘 `NegotiatedCapabilities`
（inputs、limits、specVersion…）仍是 host 私有。投影它的理由是 §11 的「部分能力目前不可用」需要一個
真實來源——沒有它，一般模式只能顯示保守的「能力核對中」，或更糟：把「不知道」說成「已同步」。
規則：
- 永遠是**陣列**（沒有不支援的 intent 就是 `[]`），接收端因此不必區分「都支援」與「不知道」。
- 還沒重新協商的還原成員（§12.10）會列出**全部** host intent——它現在確實一個都演不了，
  空陣列會是一句 host 證明不了的話。
- 這個欄位加進 `SemanticState` 之後，**舊形狀的持久化快照會被 `restore` 判成 `HashMismatch`**
  （只有本實作寫得出來的 canonical state 才能成為權威狀態），host 隔離它、開新 session，
  diagnostics 的 `storeNote` 誠實標示（§11 的「角色同步紀錄曾損毀，已重新開始」）。

> **實作註記（`crates/interaction-session`）**：一則外部訊息只回**一則** `result`（`applied`／`rejected`／`expired`／`cancel-confirmed`）；
> 同一個 `messageId` 再送一次回 `accepted{duplicate:true}` 且**不重套用**。上面的 JSON 是示意：值為「無」的選填鍵
> （`truth.correlationId`、`lastInteraction`）實作上**省略**該鍵而不是寫 `null`，因為 RFC 7396 的 `null` 是刪除語意，
> host 寫 `null` 而接收端刪除鍵會讓兩邊 canonical hash 分歧。其餘落差見文末 §12。
>
> **數字字面**：`mood.intensity` 恆為 0..=1、四捨五入到 3 位小數，且**永遠是非負零**——canonical 字面是
> `0.0`（serde_json 的 f64：整數值也帶小數，`1.0`），不是 `0`、也不是 `-0.0`。host 的 `clamp_unit` 把 `-0.0`
> 收斂成 `+0.0`；不可信來源（snapshot 檔、patch 結果）送進 sign-negative 的零一律拒絕
> （`SessionError::InvalidState` → `schema-invalid`），不修正。三端對同一份 state 的 canonical 文字與 SHA-256
> 由 `crates/interaction-aip/tests/fixtures/manifest.json` 的 `stateHashes`（9 份 host 真實輸出）釘住；
> f64 欄位清單 `stateHashDoublePaths` 由 schemars 推導（目前只有 `/mood/intensity`）。

## 4. 語意事件目錄（1.0）

| name | 誰可送 | payload | 離線政策 | Director 效果 |
|---|---|---|---|---|
| `character.interaction.touch` | input-device／host-renderer（可信 surface） | `{kind: tap|longpress|pat|stroke, intensity?: 0..1}` | expire-by-deadline（5 s） | 非 emergency／blocked 時：mood→happy 或 playful（longpress／pat→playful），activity→reacting；發 `react-happily-to-touch` |
| `character.interaction.dismiss` | 同上 | `{}` | drop-if-offline | activity→resting；發 `settle` |
| `task.state` | runtime | `{truth, correlationId}` | state-reconcile | truth 轉錄；working→activity working；waiting→waiting；failed→mood down |
| `task.verified` | runtime（人類驗證） | `{correlationId}` | state-reconcile | truth→verified、mood→proud、activity→celebrating；發 `celebrate` |
| `runtime.emergency` | runtime | `{engaged: bool}` | state-reconcile | engaged：truth→emergency、activity→frozen、取消所有 pending intent、之後的互動事件 `rejected{scope-denied}`；解除：truth→none、activity→idle |
| `character.session.presence` | host | `{party, presence}` | state-reconcile | members 更新 |

未知 name → `rejected{unknown-name}`。Device／renderer 送 `task.*`／`runtime.*` → `rejected{scope-denied}`＋稽核。

## 5. Behavior Intent（`command{name:"character.behavior.request"}`）

```jsonc
{ "intent": "react-happily-to-touch", "intensity": 0.45, "interruptible": true,
  "origin": "interaction",        // interaction | truth | ambient
  "hints": { "haptic": "light" } }  // 建議；是否觸發 haptic 由 governor 管的動器路徑決定，不由 renderer 自作主張
```

1.0 intent 詞彙：`react-happily-to-touch`、`celebrate`、`settle`、`idle`。Renderer 不支援者協商為 `unsupported`，
本地降級（idle／文字），回 `result{status: rejected, code: unsupported-capability}` 或不回；不得回 `observed`。

**結清（誰把 pending intent 收掉）**：成員回的 `result` 只要 `causationId` 對得上 host 送出的
`command`（或 `correlationId` 對得上該 intent），就依 status 結清：

| status | host 的處理 |
|---|---|
| `observed`／`rejected`／`failed`／`cancel-confirmed` | 終態：把這個成員從該 intent 的待覆名單移除；名單空了就結清（不再等 TTL），計數 `intents.observed`／`intents.rejected`／`intents.failed` |
| `accepted`／`acknowledged` | 只更新狀態，intent 繼續掛著（誠實階梯：acknowledged ≠ completed） |
| 已結清的 intent 再收到 result | 忽略（不重複計數，重播灌不進計數器） |

只有**真的沒有人回覆**的 intent 才會在 TTL 到期時被稽核成 `character.session.intent-expired`。
`observed` 仍然**不是** `verified`：`verified` 只能由 Runtime 的人類驗證路徑產生，成員送
`result{status:"verified"}` 在 §8 第 8 關就被 `scope-denied` 擋掉。

投影到 CPP（桌面 renderer，`CppRendererAdapter`）：`react-happily-to-touch` → CPP `play`（variant 同名、
parameters.intensity、priority 40、truthState none）；`settle` → `rest`；`idle` → `idle`；`celebrate`（origin truth）
**不投影**——桌面已由既有 Runtime 真相投影送 `verified-success`（受保護行為，不雙播）。iPhone 沒有既有真相投影，
所以 `celebrate` 直接送給它。

## 6. Revision／Sequence／Snapshot／Patch／Replay

見 AIP §6。Session 端實作：`EventLog`（有界環 512：`{sequence, revision, patch, hash, at}`）、`resume(lastRevision)`
→ `Replay::Patches(Vec<..>)` 或 `Replay::Snapshot`；`snapshot()` 含 `hash`；`apply_patch` 純函式供接收端使用；
`state_hash` 用 canonical JSON SHA-256。Host 每 N（預設 32）個 revision 或每 60 s 把 snapshot 持久化到 `SessionStore`。
接收端收到這些訊息之後要做什麼，見 §7 的決策表（`interaction_session::receive`）。

**host 送出的 `payload.reason`（snapshot 專用，兩個值）**：

| 值 | 什麼時候送 | epoch | 接收端 |
|---|---|---|---|
| `session-reset` | session 真的被重建過（restore 拿到成員超前的證據、開了新 session、檔案損毀重來） | **變了**（+1 或從 1 重新起跳） | 丟棄本地狀態、採用這一份（§7 規則 3） |
| `recovery` | 同一個 session，host 的權威狀態真的比對方記得的舊（從較舊快照還原，但沒有重建的理由） | **不變** | 採納並退回 host 的 revision（§7 規則 6），稽核 `aip.state-recovered` |

`recovery` 是 **AIP 1.0 接收端澄清（2026-09-05，v0.7.0）** 新增的 reason 值：訊息形狀、欄位、版本號一律不變。
沒有它的時候，host 只能回一則沒有 reason 的舊 snapshot（被 rollback 防護忽略）或謊稱 `session-reset`
（§7 的 reset 例外要求 epoch **不同**，同 epoch 一樣被忽略）——兩條路都是「兩邊永久分歧、畫面卻寫著已同步」。
只認得舊值的接收端把 `recovery` 當成沒有 reason 的 snapshot，行為與今天完全相同（不會更糟）。
host 送出 `recovery` 時**不得** epoch+1（成員的宣稱不是重建 session 的理由），並記在 `counters.snapshots.recovery`。

**持久化容器的格式版本（`Snapshot.format`，與 AIP wire version 分開）**：`state/character-session.json` 的
`Snapshot` 帶 `format: u32`（`interaction_session::SNAPSHOT_FORMAT`，目前 1；缺鍵讀成 0＝v0.6.0 無版本格式）。
`format` **不進** `hash`（`hash` 只涵蓋 `state`，三端同一定義），所以改檔案佈局不是跨語言 wire 變更；
`Snapshot` 也從不出現在任何 AIP envelope 的 payload 裡。載入是有界讀取（≤1 MiB）→ 先辨識 `format` → 五種結果：

| 讀到的 | 處理 | diagnostics |
|---|---|---|
| 舊版本（format < 現行，例如 v0.6.0 寫的、或成員缺 `unsupportedIntents` 的檔） | 依原始 hash 驗完整性 → 反序列化（`deny_unknown_fields`＋`attention` 允許鍵）→ 驗不變量 → 遷移到記憶體；原檔先備份成 `character-session.json.pre-format-<n>`（同一來源格式只留一份）再以現行格式落地。session **不**重建（epoch 不變、成員不掉） | `store.migratedFrom`／`store.migrationNote`；`storeNote` 維持 null |
| 現行格式 | 同上，不遷移 | — |
| 未來版本（format > 現行） | **不隔離、不覆寫**：檔案原封不動，這一輪用記憶體 session（epoch 往前跳，成員 resume 會拿到 `session-reset`），store 進入 parked | `storeNote`＝future-format 固定文字；`store.parked=true` |
| 暫時無法讀取（權限、EIO、目錄佔住路徑） | 檔案原封不動、parked（之後每一次 persist 都拒絕，不只跳過開機那一次）；重啟 daemon 才會再試 | `storeNote`＝unreadable 固定文字（**不**宣稱已隔離）；`store.parked=true` |
| 真正損毀（解不開、超過上限、session id 不符、hash 不符、違反不變量） | 隔離成 `.corrupt`，開新 session（epoch 往前跳） | `storeNote`＝unusable 固定文字 |

遷移中斷（備份寫不進去、落地失敗）：原檔一個位元組都不動、不隔離，`store.migrationNote` 降級成 migration-deferred。
Downgrade：v0.6.0 讀 format 1 檔案**可以**（`Snapshot` 沒有 `deny_unknown_fields`，多一個 `format` 鍵被忽略；測試
`a_format_1_snapshot_is_still_readable_by_the_v0_6_0_snapshot_shape`）；但若未來在 `SemanticState` 新增欄位，v0.6.0 讀那種
檔案會因 `deny_unknown_fields` 隔離——`format` 只描述檔案佈局，不描述 `state` 內容版本。

**store 實作契約（`SessionStore::save`）**：回 `Result<SaveOutcome, PortError>`（`Written`／`SkippedStale`／`SkippedParked`）；
`(epoch, revision)` 不得倒退——檢查與寫入必須在同一把鎖內原子完成，否則兩個併發 persist 會同時通過檢查再亂序 rename，
讓舊快照最後落盤。persist 失敗不得吞掉：計數與固定文字進 diagnostics `store`。這不是網路 exactly-once：broadcast 排在
persist 之前、每 N 個 revision 才落地，重啟後由 `resume`／`session-reset` 補洞。

## 7. 重連流程（member 視角）

1. 重連 Transport（各 Transport 自己的退避）。
2. 送 `capability`（重新協商；host 可能已重啟）。
3. 送 `query character.session.resume{lastRevision, lastSequence, sessionEpoch}`。
4. 收 `response{kind: patches|snapshot}`；逐則走下面的決策表。
5. 之後每一則 `state` 也走同一張表；連續 3 次未能套用 → 顯示「無法恢復，請重新連接」（狀態是**未知**，不是「已同步」）。
6. 重連期間**不**重播互動事件與 intent；本地佇列中過期的 touch 直接丟（稽核計數）。

### 7.2 接收端決策表（AIP 1.0 接收端澄清（2026-09-05，v0.7.0）：wire 不變、新增 reason 值 `recovery`）

權威實作是 `crates/interaction-session/src/receive.rs::decide_receive`（純函式），跨語言 fixture 是
`crates/interaction-aip/tests/fixtures/manifest.json` 的 `receiveDecisions` 段（45 個具名案例）。
桌面（TypeScript）與 iPhone（Swift）讀同一段對答案：**同一則訊息，三端必須得到同一個決策**。

前提：訊息已經通過 typed boundary（envelope 合法、`messageType == state`、`revision` 與 `sessionEpoch` 是非負整數、
大小／深度合法）。錯誤格式與超大訊息由 boundary 擋下 → `reject-invalid`；如果那是一則**權威回覆**，算一次 realign 失敗。

依序評估，**第一個命中即決定**：

| # | 條件 | 決策 |
|---|---|---|
| 0 | 訊息來自已失效的連線世代／請求世代 | `ignore-stale-connection`（計數）。**先於一切 epoch 判斷**——舊連線遲到的 `session-reset` 宣告的 epoch 一定與本地不同，任何 epoch 規則都會被它騙過去，這是唯一防線 |
| 1 | incoming 有 `sessionId`、local 有狀態、local 的 `sessionId` **已知**、且兩者不同 | `reject-identity`（稽核 `aip.identity-mismatch`）。**不** realign——realign 只會再要一次別人的 session。local 有狀態但 `sessionId` 未知（例如由不帶 `sessionId` 的 resume snapshot payload bootstrap 出來的那一份）**不算不符**：繼續往下判，並在套用時記下 incoming 的 `sessionId`（`advance`），下一則就有身分可比。把「未知」當成不符是 fail-closed 的地雷——`reject-identity` 不 realign，那台裝置會被永久凍在舊狀態且沒有出路 |
| 2 | snapshot 缺 `hash` 或缺 `state`（patch 缺 `baseRevision`） | `reject-invalid`。AIP 1.0 的 snapshot 必帶 hash，**沒有 legacy profile** |
| 3 | snapshot、`reason == "session-reset"`、epoch 與 local 不同（或 local 無狀態） | `reset`：丟棄本地狀態、採用新的 epoch／revision、清 realign 計數 |
| 4 | snapshot、local 無狀態 | `apply`（bootstrap） |
| 5 | snapshot、epoch 不同、無 reset 宣告 | `realign(epoch-changed)`：不套用，送 `character.session.resume`。host 對 epoch 不同的 resume 一律回 `session-reset` snapshot，所以一次就收斂 |
| 6 | snapshot、同 epoch、`reason == "recovery"`、`revision < local` | `recover`：套用、退回 host 的 revision、稽核 `aip.state-recovered` |
| 7 | snapshot、同 epoch、`revision < local` | `ignore-stale`（rollback；稽核 `aip.state-rollback-ignored`） |
| 8 | snapshot、同 epoch、`revision == local` | `already-applied`（宣告的 hash 與本地算出來的不同 → `realign(hash-mismatch)`） |
| 9 | snapshot、同 epoch、`revision > local` | hash 核對通過 → `apply`；不符 → `realign(hash-mismatch)` |
| 10 | patch、local 無狀態 | `realign(no-local)` |
| 11 | patch、epoch 不同 | `realign(epoch-changed)` |
| 12 | patch、`revision <= local` | `ignore-stale`／`already-applied` |
| 13 | patch、`baseRevision != local.revision` | `realign(base-mismatch)` |
| 14 | merge 之後的 hash 與宣告的不同 | `realign(hash-mismatch)` |
| 15 | 其餘 | `apply` |

**resume 回覆**：逐則走上表；`already-applied`／`ignore-stale` 是良性的舊項（host 回放的範圍本來就可能與本地
重疊），跳過**不**中止；第一個帶 effect 的 realign 中止整批（後面的補丁都建立在沒套用的那一則之上）；
`patches` 數量超過 `maxResumePatches` → 整批不處理、直接 realign（**不**靜默截斷成「我以為我追上了」）。

**有界 realign**：連續 `maxRealignAttempts`（3）次未能 apply → `unrecoverable`，照實說狀態未知、不再自動重試；
任一次 `apply`／`reset`／`recover` 清零。兩個常數的權威值在 `interaction_aip::limits`
（`MAX_RESUME_PATCHES` = 512 = `EVENT_LOG_RING`、`MAX_REALIGN_ATTEMPTS` = 3），發布在 golden schema 的 `limits` 表，
三端由 codegen 讀同一個數字。本地可以更嚴，但要走 realign，**不得靜默截斷**。

與 v0.6.0 的行為差異（三處，全部往「不猜」的方向）：

1. snapshot 的 epoch 與本地不同又沒有 reset 宣告時，Rust／Swift 以前直接套用並靜默改寫本地 epoch；現在 realign（規則 5）。
2. patch 以前完全不看 epoch，只靠 `baseRevision` 恰巧不符去擋；現在 realign（規則 11）。
3. 桌面端以前有 `allowRegression`／`hostRegressed`（「最新的 HTTP 回覆比本地舊就接受」）；現在取消——同一個
   incarnation 的回退要 host 明說 `recovery`（規則 6／7）。

規則 1 的「已知才比對」在 v0.7.0 一併釘死（fixture `identity-unknown-locally-adopts-incoming`／
`identity-known-mismatch-still-rejected`）：三端原本都是「local 有狀態＋incoming 有 `sessionId`＋不等於 local 的（可能是
未知的）身分 → reject」，一旦 resume snapshot payload 負責 bootstrap 而它不帶 `sessionId`，之後每一則帶 `sessionId`
的 SSE 都會被擋掉。放寬只發生在「本地不知道」那一格：本地知道就照樣 reject。

### 7.1 存活證明（`lastSeenAt` 從哪裡來）

**存活證明 ＝ 任何一則通過身分綁定與 membership 檢查的 inbound 訊息**，不論 `messageType`、
也不論它最後是 `applied`／`rejected`／`expired`／`accepted{duplicate}`。heartbeat 只是其中一種，
**不是唯一一種**：只送 `character.interaction.touch`、從不送 heartbeat 的裝置一樣是活著的。

Transport 層的舊協定 frame 也算：已經協商過（是成員）的 iPhone 送來 v1 的 `status` 心跳時，
host 呼叫 `Runtime::character_session_touch_presence` 記下存活（`transport-bindings.md` §1.4）。
沒協商過的舊 App 不是成員，這個呼叫對它沒有任何作用。

規則：
- `lastSeenAt` 走 §12.7 的投影格線（每則訊息都改共享狀態＝revision 無界成長）。
- presence 的變化即時反映：`offline`／`reconnecting` 的成員送來一則已驗證訊息就轉回 `online`，
  而且與這則訊息本身造成的狀態變更**合併成同一個 revision**（一則訊息只產生一則 patch）。
- 沒有這條規則的後果（實測）：只送 touch 的裝置在 `presenceTimeout` 後被標 offline，
  host 的 stale 清除接著把它 `leave`，之後每一則互動都得到 `not-a-member`。

## 8. 安全檢查（每則來自 device／renderer 的訊息）

順序固定：message bytes ≤ 64 KiB → JSON 解析 → schema／profile 驗證 → payload ≤ 32 KiB、深度、字串長度 →
`specVersion` major → 身分綁定（Transport 身分 vs `source`）→ session membership（未 join 的 device 不能送 event）→
`sessionId` 等於本 session（跨 session 注入 → `not-a-member`）→ name scope（capability 宣告過的 inputs 才能送）→
`consentGrantId` scope（第 8.1 關）→ message type 白名單 → rate limit → deadline（第 11 關，含夾制）→
dedupe（第 12 關，只查不記）→ emergency → apply。任一失敗：`result{rejected}`／`error`＋稽核
（`aip.rejected{code}`），不執行。

三個容易被誤讀的關卡，權威實作（`session.rs::gate`，十三個 `// 1.`…`// 13.` 註解錨點）補齊如下：

- **第 8.1 關 — inbound 的 `consentGrantId` 一律 `scope-denied`**。`consentGrantId` 只存在於
  host→裝置、需要授權的 `command` 上。成員送進來的訊息**沒有任何理由**帶 grant：AI／adapter／裝置
  不能授予 consent（CLAUDE.md 不變量）。1.0 直接拒絕，**不去問任何驗證器**——把一個偽造的 grant 拿去
  查詢，本身就已經把「誰能授權」這個決定交給了外部輸入。
- **第 11 關 — `expiresAt` 是宣稱，不是授權**。互動事件（`character.interaction.*`）的有效期一律夾成
  `min(成員自報的 expiresAt, occurredAt + touchTtlMs)`；沒帶 `expiresAt` 就用上界。沒有這個夾制，
  一台離線幾分鐘的手機只要把 `expiresAt` 寫成一小時後，重連時排隊的舊觸摸就會被當成新鮮互動套用。
  `occurredAt` 與 host 時鐘偏差超過 `MAX_CLOCK_SKEW_MS`（30 s）時**只稽核不拒絕**：留下
  `aip.clock-skew{skewMs, maxMs}`（只記偏差量，不回顯 payload），因為時鐘不準不是攻擊的證據，
  而夾制已經把它造成的傷害關掉了。
- **第 12 關 — 去重「只查不記」**。這一關命中就回 `accepted{duplicate:true}` 且**不重套用**；
  沒命中則**先不佔位**，等訊息真的被 `apply` 之後才把 `messageId` 放進去重環。先佔位的話，
  一則後來被拒絕／過期的訊息會把自己的 id 燒進 256 筆的環裡，讓合法的重送永遠得不到處理。
  去重對**每一種** message type 都成立，`query`（resume／snapshot）不例外：重播不得再跑一次
  resume／snapshot，那會多消耗一個 sequence 並灌大 diagnostics 計數器。

membership 那一關通過之後**立刻記存活證明**（§7.1）：後面每一關都可能拒絕這則訊息，但拒絕的是訊息，
不是這個成員的存在。身分綁定或 membership 沒過的訊息**不算**存活證明（那才是「不認識的人」）。

必測（§15 安全要求）：偽造 source.id、未配對裝置、已撤銷裝置重連、duplicate、out-of-order、replay old、expired touch、
oversized、unknown type、unknown capability、invalid baseRevision、snapshot rollback、cross-session injection、
scope mismatch、renderer capability spoofing（宣告不存在的 intent 只會得到 unsupported，不影響他人）。

## 9. 同步等級（`syncClass`）

- `semantic`（1.0 完成目標）：同步 event／state／intent，各 renderer 自行呈現。
- `timeline`（1.0 只定介面）：`{performanceId, startAt, durationMs, seed, offsetMs, lateJoinPolicy: seek|restart|skip,
  fallback: semantic, disconnectPolicy: continue-local|stop}`；不傳逐幀畫面。無實作，協商時回 `unsupported`。
- `realtime`（1.0 只定邊界）：連續拖曳、傾斜、姿態、AR；**不得**成為角色狀態的核心依賴、不要求跨平台 deterministic lockstep。

## 10. Diagnostics（`GET /v1/character-session/diagnostics`，human token）

`{ sessionId, sessionEpoch, revision, sequence, members[], counters:{ accepted, applied, rejected:{<code>:n},
duplicates, expired, resumes, resumes.ahead, snapshots, patches, identity_mismatch, internal,
intents.emitted, intents.expired, intents.dropped, intents.observed, intents.rejected, intents.failed,
intents.unsolicited }, eventLog:{ len, cap }, storeNote, store? }`。不含 token、路徑、原始 payload。
一般模式不顯示這些；只顯示 §11 的人話。

`storeNote` **只在 session 真的被重建時**非 null（unusable／unreadable／future-format 三種固定文字）。選填的
`store` 物件是持久化 store 的健康度：`{ format, migratedFrom, migrationNote, lastPersistedRevision, persistFailures,
skippedStale, parked, lastPersistError, note }`——遷移寫在 `migratedFrom`／`migrationNote`（不寫 `storeNote`，因為
沒有重建）；`parked=true` 代表這一輪什麼都存不下來（`note` 是原因）；`lastPersistError` 是固定文字，不含路徑。

`members[]` 是 §3 的 `MemberView`（含 `unsupportedIntents`），不含 `negotiated` 的其餘細節，外加一個由 Runtime
決定的選填欄位 `identityStrength`：`paired-token`（已配對 iPhone，host 端逐次驗 token）、
`transport-hello+device-side-pairing`（宣告式 Serial／MQTT／BLE 裝置線：裝置自報 id＋裝置端配對碼，**弱於** paired-token）、
`host-surface`（桌面 human token 綁定）、`unknown`（通道回報了核心不認得的值）。它來自 Runtime 的裝置出站登記表
（`character_session::DeviceOutbound`），不是成員自報；查不到出站通道（例如從快照還原、尚未重連的裝置）就**省略**
欄位——不猜、不冒充已驗證。介面不得把 `transport-hello+device-side-pairing` 顯示成「已驗證身分」。
幾個計數器的語意值得寫明，因為它們是「誠實記錄，不動狀態」的地方：

| counter | 意思 |
|---|---|
| `resumes` | 收到 `character.session.resume` 的次數（重播被去重擋掉的不算） |
| `resumes.ahead` | 成員宣稱的進度**超前 host**。這是宣稱不是證據：host 只在自己確實從一份無法證明最新的快照還原過時才重建 session，其餘一律只留這個計數 |
| `intents.emitted` / `expired` | 送出的 Behavior Intent 數／真的沒有任何人回覆而逾時的數 |
| `intents.dropped` | 因為沒有任何 online 且協商為 `exact` 的 remote-renderer 而沒送出去的 intent |
| `intents.observed` / `rejected` / `failed` | 成員回報的終態（`observed` **不是** `verified`） |
| `intents.unsolicited` | 對不上任何待決 intent 的 `result`（重播或亂送）；只計數，不結清任何東西 |
| `identity_mismatch` | 身分綁定沒過的訊息數（`source` 與 Transport 身分不符） |

## 11. 一般模式文案（人話；由 `statusProjection/characterSync.ts` 投影，不外洩 revision／sequence）

> **這是語意契約，不是逐字契約**（v0.6.x）。實作端測試保護的是：緊急停止句逐字不變；綠色只給 `synced`；
> `needs-reconfirmation` 必須提到「重新確認」；`local-only` 必須說「不會自動回來」＋「重新配對」；空狀態／關閉不像故障；
> `partial-capability` 不是故障。其餘文案允許改寫與本地化。每一態另有**穩定的下一步 action id**（machine semantics）：
> `connect-phone`（`no-device`／`local-only` → connect/providers）、`reconfirm-device`（`needs-reconfirmation` → connect/providers，
> 帶上是哪一台）、`view-capabilities`（`partial-capability`／`capability-unknown` → connect/devices）、`safe-reconnect`
> （`unrecoverable` → connect/providers）、`open-devices`（`offline`／`reconnecting`，不催促）、`storage-help`（`store-issue`，
> 只有說明、沒有落點）、`null`（`synced`／`syncing`／`disabled`）。按鈕文案可改，id 與落點由測試釘住。

| 狀態 | 文案 |
|---|---|
| 有 online 遠端成員、協商結果說它全部演得出來 | 「iPhone 已連接，角色狀態已同步」 |
| 有 online 遠端成員，但**拿不到**協商結果（`members[].unsupportedIntents` 讀不到／形狀不認得） | 「iPhone 已連接，能力核對中」（`capability-unknown`；補充：「狀態對齊了，但還沒確認這台裝置演得出哪些表演；在確認之前不要當成完全同步。」） |
| 遠端成員 presence=reconnecting | 「iPhone 正在重新連線」 |
| 遠端成員 offline | 「iPhone 暫時離線」 |
| 協商後有 unsupported intent | 「部分能力目前不可用」 |
| resume 進行中 | 「同步尚未完成」 |
| 連續 resume 失敗 | 「無法恢復，請重新連接」 |
| **有裝置現在連著這台電腦、卻不是 session 成員**（含撤銷後重連） | 「需要重新確認裝置」（`needs-reconfirmation`；detail 指出是哪一台，只用顯示名稱） |
| **零裝置、只剩歷史撤銷**（使用者主動移除了全部手機） | 「目前只在這台電腦使用」（`local-only`；中性終態，detail 說之前移除的手機不會自動回來、要再用就重新配對；撤銷的安全效果不變） |
| 持久化紀錄**現在**存不下來（diagnostics `store.parked`，或 `persistFailures>0` 且有 `lastPersistError`） | 「同步紀錄暫時存不下來」（`store-issue`；active issue，排在 online 判定之前） |
| 紀錄**曾經**重建過（`storeNote` 不是 null）但現在存得下來 | 不是狀態：卡片下方一句 muted 附註，不壓過 `synced`；`store.migratedFrom` 連附註都不是，只在進階模式 |
| 模擬 iPhone（fixture） | 一律附「模擬 iPhone（fixture）」 |

三條排序規則（`statusProjection.ts` 的宣告順序就是判定順序，vitest 釘住）：

1. `store-issue`（現在存不下來）排在「有 online 成員」之前、「讀不到」之後：它講的是紀錄，不是角色，
   所以不給綠色也不給紅色（警示色）；緊急停止的固定安全句永遠壓過它。`storeNote`（曾經重建過）只是附註。
2. 「需要重新確認裝置」看的是**當下**的事實（有裝置連著卻不是成員），**不是**「曾經有裝置被撤銷過」
   的歷史。provider 列會永遠留著 revoked：零裝置時它投影成中性的 `local-only`（主動移除手機是正常終態，
   不永久要求重配），只有那台裝置又連上來（連著但不是成員）才回到 `needs-reconfirmation`。
3. 「能力核對中」排在「部分能力目前不可用」與「已同步」之間：不知道就不給綠勾（綠勾只給真的），
   也不誣賴裝置做不到。Runtime 投影 `members[].unsupportedIntents`（§3）之後，正式路徑上這一列
   只會在讀不到 diagnostics／形狀不認得時出現。

## 12. 實作註記（`crates/interaction-session`，v0.6.0）

這些是本文件沒有寫死、由權威實作補齊的細節。語意不變，只是把留白補上；改動它們等同改契約。

1. **revision／sequence 起點**：全新 session 的 `revision` 從 1 起、`sequence` 從 0 起（第一則送出的訊息 sequence = 1）。
   每次成功 `applied` 使 `revision` +1；host 每送一則 `state`／`command` 消耗一個 sequence。點對點的 `command`
   會讓其他成員看到 sequence 跳號，這是預期的（gap 偵測屬 Transport，不是狀態錯誤）。
2. **`inputs` 只約束 `event`**：§4.2 的 `inputs` 定義是「可產生的 event name」，因此 §8 的 name scope 只套用在
   `messageType: event`；`heartbeat`／`capability`／`query`／`cancel`／`result` 的 name 不受 `inputs` 限制
   （它們仍受 message type 白名單、身分、membership、rate limit 管）。
3. **member 可送的 message type**：`event`／`cancel`／`query`／`result`／`heartbeat`／`capability`。
   `command`／`state` 是 host 的權力，成員送來一律 `rejected{scope-denied}`。
4. **`task.state{truth:"verified"}` 只轉錄真相**，不產生 `celebrate`；慶祝只由 `task.verified` 產生（避免雙播）。
5. **`attention` 的擁有者是 Director**：touch → `{kind:"member"}`、`task.*` 帶 correlation → `{kind:"task"}`、
   dismiss 與 emergency → `{kind:"none"}`。
6. **`Party` 的兩種書寫**：`attention.id` 與 `lastInteraction.source` 是 `"<kind>:<id>"` 字串（§3 範例的形狀），
   `members[].party` 是物件。
7. **`lastSeenAt` 投影格線**：任何一則存活證明（§7.1）只在距離上次投影超過 `presenceTimeout / 3` 時才更新
   共享狀態裡的 `lastSeenAt`，否則一個高頻送訊息的成員就能把 revision 與廣播打成無界成長。
   presence 本身的變化一律即時反映。時鐘倒退時 `lastSeenAt` 不往回拉（不憑空製造一次逾時）。
8. **`character.behavior.*` 是 drop-if-offline**：只送給 presence 為 `online`、且把該 intent 協商成 `exact` 的
   `remote-renderer`。host 端 renderer 一律拿得到 `RendererIntent`（含 CPP 投影），即使沒有任何遠端成員。
9. **host 送出的 `messageId`** 形如 `aip-<epoch>-<epochMillis>-<n>`；`n` 在 restore 時以持久化的 `sequence` 為起點。
   host→member 的去重靠 revision／sequence（§6），不靠 messageId。
10. **restore 之後的成員沒有協商結果**：他們留在 `members` 投影裡，但必須重送 `capability`（§7 第 2 步）才能再送
    event，否則得到 `rejected{scope-denied}`。
11. **`activity: reacting` 的計時器不持久化**：restore 後以 `now` 重新起算，`reactionMs` 後回到 `idle`。
12. **`persist`**：實作以 `Output::Persist` 建議 host 存檔（預設每 32 個 revision 或每 60 s，且 revision 有變動才建議）。
13. **`role` 被 Transport 身分夾住**：`capability` 裡的 `role` 也是**宣稱**。`host-renderer`
    （可信桌面 surface、拿得到安全 overlay 的那一個）只能由 `human-surface` 身分擔任；device／renderer
    自報 `host-renderer` 會被夾回 `remote-renderer` 並稽核 `character.session.role-corrected`
    （`{party, claimed, effective}`），**不拒絕連線**。不夾的話會得到一個「共享狀態說它能演、
    但它永遠不在派送名單上」的假象——一般模式會據此顯示綠色「已同步」。
14. **restore 的保守跳號 vs `resumes.ahead` 的取捨**：`restore` 把 revision 直接加上一個持久化間隔
    （`persist_every_revisions`），因為那份快照**無法證明**自己就是當機前最後廣播出去的那一版
    （§6 持久化是有間隔的）。代價是重啟後 revision 會跳號（成員看得到，屬預期）；換來的是不會
    倒退。反過來，成員送 `resume{lastRevision}` 宣稱自己領先 host 時：那是**宣稱**，不是證據——
    只計數 `resumes.ahead` 並回一份普通 snapshot。**唯一**的例外是本 session 確實是從那種
    無法自證最新的快照還原出來的：那時成員的領先就是「host 真的倒退過」的證據，host 才重建
    session（epoch+1）並發 `session-reset`。沒有這個不對稱，任何成員送一則
    `resume{lastRevision: u64::MAX}` 就能讓所有人丟掉本地狀態。
15. **`Presence::Reconnecting` 由 Transport 產生，不由 session 推論**：socket 斷了、而這台裝置仍在
    配對狀態、對方仍在退避窗口內重連——這是 Transport 才知道的事實。`mobile.rs` 的斷線收尾對
    **已協商過的成員**送 `Reconnecting`（沒協商過的舊 App 不是成員，什麼都不做），逾時之後由
    session `tick` 轉成 `Offline`，再久才清除成員。撤銷仍然是 `leave`（那台裝置不再是成員）。
    session 自己**不能**從「45 秒沒聽到聲音」推論出「對方正在重連」——那是推論，不是真相。
16. **待決 intent 的紀錄是 host 私有的**：`pending` 除了 intent 本身，還記著「送給了誰」與「送出去的
    command messageId」（成員的 `result{causationId}` 靠它對回來）。這些都不進 `SemanticState`，
    也不出現在任何 patch 裡。稽核 `character.session.intent-settled` 只寫 intent 名稱、status、
    對方的 `<kind>:<id>` 與是否已結清，不回顯 payload。
