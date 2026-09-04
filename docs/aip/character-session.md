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
  "members": [ { "party": {"kind":"device","id":"…"}, "role": "remote-renderer", "presence": "online", "lastSeenAt": "…" } ],
  "reducedMotion": false
}
```

字串長度、成員數（≤ 16）、巢狀深度受 AIP limits 約束。

> **實作註記（`crates/interaction-session`）**：一則外部訊息只回**一則** `result`（`applied`／`rejected`／`expired`／`cancel-confirmed`）；
> 同一個 `messageId` 再送一次回 `accepted{duplicate:true}` 且**不重套用**。上面的 JSON 是示意：值為「無」的選填鍵
> （`truth.correlationId`、`lastInteraction`）實作上**省略**該鍵而不是寫 `null`，因為 RFC 7396 的 `null` 是刪除語意，
> host 寫 `null` 而接收端刪除鍵會讓兩邊 canonical hash 分歧。其餘落差見文末 §12。

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

## 7. 重連流程（member 視角）

1. 重連 Transport（各 Transport 自己的退避）。
2. 送 `capability`（重新協商；host 可能已重啟）。
3. 送 `query character.session.resume{lastRevision, lastSequence, sessionEpoch}`。
4. 收 `response{kind: patches|snapshot}`；epoch 不同 → 丟棄本地狀態、套用 snapshot。
5. 之後只接受 `state.revision > local`；缺 sequence（gap）→ 再 resume 一次；連續 3 次失敗 → 顯示「無法恢復，請重新連接」。
6. 重連期間**不**重播互動事件與 intent；本地佇列中過期的 touch 直接丟（稽核計數）。

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
rate limit → deadline → dedupe → apply。任一失敗：`result{rejected}`／`error`＋稽核（`aip.rejected{code}`），不執行。

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
duplicates, expired, resumes, snapshots, patches, intents.emitted, intents.expired, intents.dropped,
intents.observed, intents.rejected, intents.failed }, eventLog:{ len, cap } }`。不含 token、路徑、原始 payload。
一般模式不顯示這些；只顯示 §11 的人話。

## 11. 一般模式文案（人話；由 `statusProjection.ts` 投影，不外洩 revision／sequence）

| 狀態 | 文案 |
|---|---|
| 有 online 遠端成員 | 「iPhone 已連接，角色狀態已同步」 |
| 遠端成員 presence=reconnecting | 「iPhone 正在重新連線」 |
| 遠端成員 offline | 「iPhone 暫時離線」 |
| 協商後有 unsupported intent | 「部分能力目前不可用」 |
| resume 進行中 | 「同步尚未完成」 |
| 連續 resume 失敗 | 「無法恢復，請重新連接」 |
| 裝置被撤銷 | 「需要重新確認裝置」 |
| 持久化紀錄曾損毀（diagnostics `storeNote` 不是 null） | 「角色同步紀錄曾損毀，已重新開始」（補充：「已重新連接的裝置會重新同步；不影響角色本身。」） |
| 模擬 iPhone（fixture） | 一律附「模擬 iPhone（fixture）」 |

`storeNote` 那一列排在「有 online 成員」之前、「讀不到」之後：它講的是紀錄，不是角色，
所以不給綠色也不給紅色（警示色）；緊急停止的固定安全句永遠壓過它。

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
13. **待決 intent 的紀錄是 host 私有的**：`pending` 除了 intent 本身，還記著「送給了誰」與「送出去的
    command messageId」（成員的 `result{causationId}` 靠它對回來）。這些都不進 `SemanticState`，
    也不出現在任何 patch 裡。稽核 `character.session.intent-settled` 只寫 intent 名稱、status、
    對方的 `<kind>:<id>` 與是否已結清，不回顯 payload。
