# AIP Transport Bindings（v0.6.0 實際形狀）

> `docs/aip/README.md` §9 定義有哪些 Transport 綁定；這份文件寫**實作出來的實際形狀**：
> iPhone wss frame、HTTP 路由、SSE 事件、CLI 子指令、fixture op，以及每一條路徑上
> 誰是「已驗證身分」。權威實作：`crates/interaction-runtime/src/character_session.rs`
> （Session Host 接線）＋`crates/interaction-runtime/src/mobile.rs`（iPhone frame）＋
> `crates/interaction-api/src/{lib.rs,routes.rs,sse.rs}`。
>
> Transport 只是 AIP envelope 的載體：framing、重連、退避、速率窗屬各 Transport；
> 語意、錯誤碼、outcome 階梯、revision／sequence 規則一律由 AIP 決定。

## 0. 身分對照（誰是「已驗證身分」）

| Transport | 已驗證身分（bound identity） | `source` 必須宣稱 |
|---|---|---|
| iPhone wss v1 | 配對 token → `deviceId`（`identityStrength: paired-token`） | `{kind:"device", id:"<deviceId>"}` |
| 宣告式裝置線 v1.1（Serial／MQTT／BLE，§8） | `hello.deviceId`（裝置自報）＋裝置端配對碼（`identityStrength: transport-hello+device-side-pairing`，弱於 iPhone） | `{kind:"device", id:"<spec 的 deviceId>"}` |
| HTTP（human token） | 可信 host surface | `{kind:"human-surface", id:"desktop"}` |
| Runtime 內部 | Runtime 自己 | `{kind:"runtime", id:"runtime"}`（外部不得宣稱） |

宣稱與綁定身分不符 → `error{code:"identity-mismatch"}`＋稽核 `aip.identity-mismatch`，
**不執行**、也不「幫忙修正」後執行。

> **桌面視窗的成員身分是 `human-surface:desktop`，不是 `renderer:desktop`。**
> 桌面能證明的身分只有 human token，AIP §5 把它綁成 `human-surface`；用 `renderer`
> 會讓 `POST /v1/character-session/events` 永遠 `not-a-member`。CPP 投影不靠成員身分：
> host 端 renderer 一律收得到 `RendererIntent`（`character-session.md` §12.8），即使
> session 裡一個成員也沒有。

## 1. iPhone wss v1：`{"type":"aip","envelope":{…}}`

* 只在 `auth-ok` 之後接受；未認證連線送 `aip` 一律忽略（連線本身仍受未認證死線約束）。
* 每則 envelope ≤ 64 KiB（AIP `MAX_MESSAGE_BYTES`），並共用 v1 既有的 128 KiB frame 上限、
  **30 msg/s** 連線速率窗與 8 連線上限——`aip` frame 也計入那個窗（超過即關連線，見
  `mobile_loop.rs`）。Session 另有每個成員 30 msg/s 的 token bucket，超過回
  `result{status:"rejected", code:"rate-limited", retryable:true}`。
* 沒送過 `capability` 的舊 App **永遠不會**收到任何 `aip` frame；`character.present` 動器與
  `iphone.touch` observation 路徑完全不變（回歸測試：`mobile_loop.rs`
  `a_legacy_phone_that_never_negotiates_receives_no_aip_frames`）。

### 1.1 手機 → host

| envelope | host 的處理 | 回覆 |
|---|---|---|
| `capability{name:"character.session.capability"}` | 第一次＝加入；已是成員＝重新協商，走完整安全管線（速率上限＋去重），否則一台已配對的裝置能用 capability 洪水把 revision 與廣播打成無界成長 | **兩則** frame：`capability`（negotiated）＋`state{kind:"snapshot"}`（重新協商時另外回一則 `result{applied}`） |
| `event{name:"character.interaction.touch"｜".dismiss"}` | §8 安全管線 → Director | 一則 `result`（`applied`／`rejected`／`expired`／`accepted{duplicate:true}`） |
| `query{name:"character.session.resume"}` | 先過安全管線，再路由到 resume | `response`（§1.3） |
| `query{name:"character.session.snapshot"}` | 同上 | `response{kind:"snapshot"}` |
| `query{其他 name}` | 不猜、不執行 | `result{status:"rejected", code:"unknown-name"}` |
| `heartbeat` | presence online（`lastSeenAt` 走投影格線）。**不是唯一的存活證明**：任何一則過了身分綁定與 membership 的 frame 都算（`character-session.md` §7.1） | 無（`Submission.reply=false`：不回 result，避免 result 迴圈） |
| `result` | 記錄成員回報的進度；`causationId` 對得上 host 送出的 `command` 時依 status 結清該 pending intent（`character-session.md` §5） | 無 |
| `cancel` | 撤銷對應的 Behavior Intent（冪等） | `result{status:"cancel-confirmed"}` |
| `command`／`state` | host 的權力，成員不得送 | `result{status:"rejected", code:"scope-denied"}` |
| 未知 `messageType` | 不執行 | `error{code:"unsupported-message-type"}` |
| 壞 JSON／超大／深度或字串超限 | 不執行 | `error{code:"schema-invalid"｜"message-too-large"｜"payload-too-large"}` |
| 不同 major 版本 | 不猜 | `error{code:"unsupported-version"}` |

`error` envelope 的 `causationId` 是出問題那一則的 `messageId`；解析不出 messageId（例如整包
壞 JSON）時**省略** `causationId`。`error.payload.message` 是固定人話，**不回顯輸入內容**、
不含路徑或 token。

### 1.2 host → 手機

| envelope | 何時 |
|---|---|
| `state{kind:"snapshot"}` | 協商完成、resume 需要完整對齊時 |
| `state{kind:"patch", baseRevision}` | 每次權威狀態改變（廣播給所有 online 裝置成員） |
| `command{name:"character.behavior.request"}` | Behavior Intent，點對點；只送給 presence `online` 且把該 intent 協商成 `exact` 的 `remote-renderer` |
| `command{name:"character.behavior.cancel"}` | 取消已送出的 intent |
| `capability` | 協商結果 |
| `result`／`response`／`error` | 對手機訊息的回覆 |

**sequence 跳號不是錯誤。** 點對點的 `command` 會消耗 session sequence，非目標成員因此
會看到 sequence 跳號。**成員只以 `revision` 判斷要不要 resume**：`state.revision` 必須是
`localRevision + 1`（patch 的 `baseRevision` 對得上）才套用；對不上才送 resume。用 sequence
判斷會在每一次點對點 intent 之後觸發一次 resume，形成無謂的 resume 迴圈。
（證據：`character_session_loop.rs`
`resume_falls_back_to_a_snapshot_when_the_log_no_longer_covers_it` 證明「revision 已對齊、
sequence 落後」時 resume 回的是**空的** patches——沒有東西要補，迴圈自然停。）

### 1.3 resume 的 `response` 形狀

```jsonc
// 日誌涵蓋得到（envelope: messageType "response", name "character.session.resume",
// causationId = query 的 messageId）
{ "kind": "patches",
  "patches": [ { "sequence": 12, "baseRevision": 5, "revision": 6,
                 "patch": { … RFC 7396 merge patch … },
                 "hash": "<sha256>", "sessionEpoch": 1 } ] }

// 日誌不足、或 patches 塞不進 payload 上限 → 完整 snapshot（**不是**錯誤）
{ "kind": "snapshot", "revision": 42, "sequence": 87, "state": { … }, "hash": "…",
  "sessionEpoch": 1 }

// epoch 不同（host 重建過 session）：丟棄本地狀態，改用這份 snapshot
{ "kind": "snapshot", "reason": "session-reset", "revision": 1, … , "sessionEpoch": 2 }
```

`patches[]` 的項目**不是**完整 envelope，`snapshot` 也是直接內嵌 `state` 訊息的 payload：
AIP §11 的 payload 巢狀深度上限是 8，多包一層 envelope 就會超過。

### 1.4 舊協定 frame 也是存活證明

v1 的 `status` 心跳（iOS App 每次狀態變化都送；定期心跳 v0.6.x 起為**前景每 15 秒**——30 秒對 45 秒逾時是零容錯，
15 秒容得下一次漏送；進背景就停止心跳，回前景立即補一則、socket 還活著且真的進過背景時另送一次
`query character.session.resume`；常數集中在 `Services/AppLifecycle.swift` 的 `PresenceHeartbeatPolicy`，測試釘住
「間隔 < 逾時/2」與 45 秒＝Rust `SessionConfig::presence_timeout_ms`）在 `mobile.rs` 收到時，
對**已經協商過**的手機呼叫 `Runtime::character_session_touch_presence`：
`lastSeenAt` 前進（走投影格線，不會每則心跳都生一個 revision）、presence 若不是 online 就轉回 online。

沒送過 `capability` 的舊 App 不是 session 成員，這個呼叫對它完全沒有作用——它仍然
**收不到任何 `aip` frame**（回歸：`mobile_loop.rs` `a_legacy_phone_that_never_negotiates_receives_no_aip_frames`）。

沒有這條規則時：iOS App 只送 `status`＋ws ping，不送 AIP heartbeat，於是協商過的手機在
45 秒後被標 offline、再被 stale 清除踢出成員，之後所有互動都得到 `not-a-member`
（回歸：`character_session_loop.rs` `a_negotiated_phone_that_only_sends_legacy_status_stays_a_member`、
`a_phone_that_only_touches_is_never_timed_out_of_the_session`）。

> App 端仍然建議之後補上每 15 秒一則 AIP `heartbeat`：那是協定內的存活證明，
> 餘裕比依賴 transport 心跳大。目前**尚未**實作送出；v0.6.x 起 App **收到** AIP `heartbeat` 時不再靜默吞掉
> （計數、note、5 秒節流回一則 legacy `status`；§2.1 的回應是選填 heartbeat，所以不回 AIP heartbeat 合規）。

## 2. HTTP（human token；`127.0.0.1`）

| 路由 | 回應 |
|---|---|
| `GET /v1/character-session` | 一則 `state{kind:"snapshot"}` envelope（`target` = `human-surface:desktop`；消耗一個 sequence） |
| `POST /v1/character-session/resume` `{lastRevision, lastSequence, epoch}` | §1.3 的 **payload**（HTTP 自帶請求-回應對應，不需要 `causationId`／envelope 外殼） |
| `POST /v1/character-session/events` `{envelope}` | 該事件的 `result` envelope（桌面把 CPP 點擊轉成 `character.interaction.touch` 時用；也是 Playwright 的入口） |
| `GET /v1/character-session/diagnostics` | §10 diagnostics（見下） |

* `agent` token、agent session capability token、character adapter token 一律 **403**
  （回歸：`api_e2e.rs` `character_session_routes_are_human_only`）。
* `INTERACT_AI_CHARACTER_SESSION=0` → 四條路由都是 **503**
  `{"error":{"code":"session-disabled","message":…}}`。
* 桌面必須先 `POST /v1/character/hello`（CPP 協商）才會成為 session 成員；還沒 hello 就送
  event → `result{rejected, code:"not-a-member"}`。

diagnostics（不含 token、路徑、原始 payload）：

```jsonc
{ "sessionId": "session.home", "sessionEpoch": 1, "revision": 11, "sequence": 18,
  "members": [ {"party": {"kind":"device","id":"iphone-…"}, "role": "remote-renderer",
                "presence": "online", "lastSeenAt": "…"} ],
  "counters": {"accepted": 3, "applied": 3, "patches": 9, "snapshots": 4,
               "rejected.scope-denied": 1, "intents.emitted": 2, "…": 0},
  "eventLog": {"len": 9, "cap": 512},
  "storeNote": null }
```

`storeNote` 不是 `null` 時代表持久化檔案讀不到／壞掉（已改名 `.corrupt`、epoch 已 +1）——
**不靜默**，一般模式要翻譯成人話（`character-session.md` §11），不得顯示這些技術詞。

## 3. SSE（`GET /v1/events`，human token）

事件型別 `character.session.state`，payload 是**完整 AIP envelope**。送出時機：

* 每一則廣播（`state{kind:"patch"｜"snapshot"}`）；
* 每一則要送給桌面可信 host surface 的 envelope（`capability`／`command`／`result`）。

與既有 `character.*` 事件同界線：agent token、adapter token 一律看不到（`sse.rs`
`character_events_are_human_only_on_sse`）。

**桌面同步卡就靠這條事件對齊，不靠輪詢**（`CharacterSyncCard` → 純函式接收端
`apps/interaction-desktop/src/aip/sessionClient.ts`，鏡射 Rust `accept_state_with_epoch`）：
`state{kind:"snapshot"}` 只在 revision 比本地新、或 host 明確標 `reason:"session-reset"` 且
`sessionEpoch` 與本地不同時取代本地副本（較舊／相同 revision 一律忽略；epoch 不同但沒有
reset 宣告 → 重新對齊，不猜）；`state{kind:"patch"}` 在 epoch 相同且 `baseRevision` 等於本地
revision 時以 RFC 7396 merge patch 套上去。缺 `revision`／`sessionEpoch`、負數、小數、超過
2^53 的值一律 invalid（不會變成 revision 0）。`GET /v1/character-session` **會消耗一個 sequence**，
所以只在**沒有本地副本**時（首次載入、讀失敗之後、卸載重掛）呼叫；patch 接不上、使用者按
「重新檢查」、連線切換（`connectionKey`）都走 `POST /v1/character-session/resume`（不消耗
sequence）。慢的 GET／resume 回應若在請求發出後已有 SSE 狀態套用，就以請求世代判為過期忽略。
裝置清單／來源清單／診斷是另一組，節流成最小間隔 2 秒的 trailing 重取。

> **桌面端會做接收端 hash 核對**（AIP §6）。JS 的 number 留不住數字字面，但 canonical 規則
> 是可重印的：`apps/interaction-desktop/src/aip/canonical.ts` 依 codegen 從 fixture manifest
> `stateHashDoublePaths` 產出的 f64 路徑把整數值的 double 印回 serde_json 的 `x.0` 形式，
> 鍵以 code point 序排序；三端共用的 `stateHashes` fixtures（9 份 host 真實輸出）逐位元組
> 核對（`src/test/canonical-hash.test.ts`）。hash 對不上就**不套用**並重新對齊；連續對齊
> 失敗達 3 次升級成「無法恢復，請重新連接」（狀態未知，不是無限重試）。

## 4. CLI（薄殼，human token）

```bash
interact-ai character session status                       # GET /v1/character-session
interact-ai character session diagnostics                  # GET …/diagnostics
interact-ai character session resume --last-revision N [--last-sequence M] [--epoch E]
```

CLI **不能**送語意事件：`character.interaction.*` 必須來自綁定身分的 surface（手機或桌面
視窗），安全 intent 只能由 Runtime 真相投影產生。

## 5. In-process（Rust）

`Runtime::character_session_{join,leave,presence,submit,submit_runtime,resume,snapshot_envelope,
diagnostics_value,peek,tick_at}`。`submit_runtime` 是 Runtime 真相的唯一入口
（`task.state`／`task.verified`／`runtime.emergency`／reduced motion）；`verified` 只會從
`verify_agent_session`（human-only）那條路徑進來。

Runtime 接線點：

| Runtime 事件 | Session |
|---|---|
| `agent.session.state`（`character_project_session` 旁） | `TaskState{truth}`；`verified` → `TaskVerified` |
| `emergency.stop` engage／clear（`character_project_emergency`） | `Emergency{engaged}` |
| `/v1/character/hello` 協商成功 | `join(human-surface:desktop, role host-renderer)`＋`ReducedMotion` |
| watchdog sweep | `tick`：reacting 逾時、presence 逾時、過期 intent、離線逾時的成員清除、到期的持久化 |
| iPhone 斷線 | `presence(device, offline)`（成員保留，逾時才清） |
| iPhone 撤銷 | `leave(device)`（立即，不是 offline） |

## 6. Fixture op（`cargo run -p interaction-runtime --example fake_iphone`）

**模擬 iPhone（fixture）**，不是真機驗收。stdin 一行一則 JSON：

```jsonc
{"op":"aip-capability"}                       // role remote-renderer；intents react-happily-to-touch/celebrate/idle
{"op":"aip-touch","kind":"tap","expiresInMs":5000,"messageId":"…","source":{…}}  // source 可覆寫以測偽造身分
{"op":"aip-resume","lastRevision":5,"lastSequence":8,"epoch":1}
{"op":"aip-raw","frame":{…}}                  // 任意 frame：未知 type／超大／壞 JSON
```

stdout（JSON Lines）：收到的每則 aip frame 印 `{"event":"aip","envelope":{…}}`，送出的印
`{"event":"aip-sent","messageType":…,"messageId":…}`；既有的 `status`／`disconnect`／
`reconnect`／`ack-stop-all`／`quit` op 與 `{"event":"connected"｜"disconnected"｜"act"…}`
輸出不變。驗收腳本：`scripts/v03-cli-e2e.sh` 的「Character Session（模擬 iPhone（fixture））」段。

**Serial 模擬器**（`scripts/esp32-serial-sim.py`，pty，不是 ESP32 真板）從 v0.6.x 起也讀 stdin 的
JSON Lines 控制指令：`aip-capability`／`aip-touch`／`aip-resume`／`aip-raw`（同名語意），未配對一律
拒絕送出並在 log 留痕；收到的每則 aip 行印成 `>> aip …`／`<< aip …`。EOF 後不再監看 stdin，
既有以 `Stdio::null()` 啟動它的呼叫端不受影響。

## 7. 1.0 的邊界（誠實記錄）

* **`consentGrantId` 不會出現在 1.0 的 session 訊息裡**：帶 grant 的訊息只可能是 `command`，
  而 1.0 的成員不得送 `command`（`scope-denied`）。因此 `ConsentVerifier` port 已定義但
  session 尚未使用它——不是「已驗過」，是「這條路目前走不到」。
* **同一瞬間多則 Runtime 真相事實的送達順序不保證**：狀態變更是同步、有序的（revision 依序
  遞增），但派送在背景任務裡；接收端以 revision 判定，落後的一律忽略（rollback 防護），
  必要時 resume。這是 AIP §6 設計內的行為，不是錯誤。
* **`characterId` 目前固定是 `"character"`**：session 在 Runtime 啟動時就建立，那時還沒有
  任何角色 hello。角色顯示名走 CPP／`/v1/character/manifest`，不從 session state 取。
* 多台電腦競爭 host、雲端同步、multi-master、CRDT：**不在 1.0**。
* **三端對「snapshot 的 epoch 不同但 reason 不是 session-reset」的結論不一致**（對抗審查 427c806 指出）：
  契約層已裁決，答案是桌面那一邊（realign，不猜）——決策表在 `docs/aip/character-session.md` §7.2，跨語言 fixture 是
  `manifest.json` 的 `receiveDecisions` 段。**Rust（`interaction_session::receive`）已依表實作**；
  TypeScript `alignState` 與 Swift `SessionDecisions.apply` 尚未改讀那張表（`allowRegression`／patch 的 epoch 規則
  仍是各自的舊版本），所以差異目前只縮到一端，還沒消失。
* `maxResumePatches`（512 ＝ host 事件日誌環）與 `maxRealignAttempts`（3）已進 golden schema 的 `limits` 表，
  三端由 codegen 讀同一個數字。桌面端 `sessionClient.ts` 仍寫著自己的 `MAX_RESUME_PATCHES = 1024`（較寬鬆），
  改讀生成常數同屬上面那一批未完成的接線。

## 8. 宣告式裝置線 v1.1（Serial／MQTT／BLE）：同一行 `{"type":"aip","envelope":{…}}`

`crates/interaction-adapter-declarative/src/protocol.rs` 裝置線協定的 v1.1 追加訊息（`proto` 仍為 1）；
規則與 §1 對稱，差異只在 Transport 自己的事：

* **准入**：只在 hello 身分驗證＋配對握手完成、且連線世代未變之後接受（`DeviceLink::admit_aip`）；
  之前一律拒絕、計數並稽核 `aip.rejected{stage:"transport-admission"}`。重連（世代更替）後舊准入立即失效。
* **大小**：AIP 64 KiB → `MAX_AIP_ENVELOPE_BYTES` 8 KiB（入站 `RefusedTooLarge`；出站超限一個位元組都不寫）
  → 傳輸自己（serial／參考韌體單行 639 bytes、BLE 512 bytes、整行解析 16 KiB）。送不出去的回覆稽核
  `aip.outbound-undeliverable{bytes,reason}`。**目前協商的 snapshot 回覆放不進 639 bytes**（§7 之外的
  v0.6.x 已知限制，見 `device-profile.md` §6）。
* **身分**：`Party::device(<spec 的 deviceId>)` 由 Runtime 依 `DeviceLink` 的期望身分綁定；
  強度 `transport-hello+device-side-pairing`（§0）。
* **存活**：既有 who／pair／ack／event 行也是存活證明（同 §1.4）；斷線＝`Presence::Reconnecting`，
  由既有 tick 轉 Offline／leave。
* **速率**：與 §1 相同的 session 端 token bucket；線上另有各 Transport 自己的窗。
* **出站**：與 iPhone 走同一張型別抹除的出站登記表（`character_session::DeviceOutbound`）；握手成立登記、
  撤銷／斷線移除。送不到（超過單行上限、線已關）→ `aip.outbound-undeliverable`；沒有通道 → 同一稽核
  `reason:"no-channel"`；表滿（64）→ `aip.outbound-rejected`。
* **證據**：`declarative_session_loop.rs` 13 測（pty 模擬器經 production serial adapter；含「廣播真的走序列線」
  與「放不進 639 bytes 的 patch 留痕」兩半）；MQTT／BLE 共用程式碼但未測；真板零。
