# Device Profile 1.0：裝置成員的能力宣告與邊界

> 證據等級用字：本文件描述的是「裝置作為 AIP session 成員可以做什麼」，這件事目前**只有 iPhone
> 一個實作**。沒有測試支持的規則標「契約文字，未見專屬測試」；ESP32／BLE／Serial 明確標「未實作」，
> 不得寫成「規劃中即將支援」以外的任何更強說法。

## 1. 是什麼

Device Profile 不是一份獨立的新協定，是 AIP `capability` 訊息（`docs/aip/README.md` §4.2）在
`role` 為 `input-device` 或 `remote-renderer` 時的**使用方式**——裝置成員用同一份
`CapabilityAnnouncement`（`crates/interaction-aip/src/capability.rs`）宣告自己是什麼、能做什麼：

```jsonc
{
  "specVersions": ["aip/1.0"],
  "role": "remote-renderer",              // input-device | remote-renderer | host-renderer | observer
  "profiles": ["character-session"],
  "syncClasses": ["semantic"],
  "intents": ["react-happily-to-touch", "celebrate", "idle"],   // remote-renderer 才有意義
  "inputs": ["character.interaction.touch", "character.interaction.dismiss"],  // input-device 才有意義
  "features": { "haptic": false, "reducedMotion": true },
  "limits": { "maxMessageBytes": 65536 }
}
```

`MemberRole`（`crates/interaction-aip/src/capability.rs:11-16`）四個值：`HostRenderer`／
`RemoteRenderer`／`InputDevice`／`Observer`。iPhone 目前以單一連線同時宣告 `remote-renderer`
（它會呈現 Behavior Intent）與透過 `inputs` 宣告自己能送哪些事件（等於同時是 input-device）——
1.0 沒有把這兩個角色拆成兩個獨立連線，`capability` 的 `role` 欄位是單一值，`inputs`／`intents`
兩個陣列各自獨立生效（`docs/aip/character-session.md` §12.2：「`inputs` 只約束 `event`」）。

## 2. 裝置成員只能送什麼

依 `crates/interaction-session/src/session.rs::gate`（:706-714，第 9 步）：member 能送的
`messageType` 只有 `event`／`cancel`／`query`／`result`／`heartbeat`／`capability`；
`command`／`state` 一律 `rejected{scope-denied}`（host 的權力）。這正是任務書要求的
「只能送 event／query／heartbeat／cancel／result，不能送 command／state／task.*」——
`task.*`／`runtime.*` 額外被 `is_runtime_only_name`（`crates/interaction-aip/src/message.rs:164`）
擋在 `gate` 的第 6 步（:673-675）。

測試：`crates/interaction-session/tests/session.rs::devices_may_not_produce_runtime_truth_or_verified`
(:531)；member message type 白名單本身沒有找到單獨命名的測試（例如「member 送 `state` 被拒」），
`security_matrix.rs::the_pipeline_order_is_fixed_identity_before_membership_before_scope`(:217)
涵蓋的是管線順序而非這條特定規則的窮舉——**部分覆蓋，未見對六種訊息型別逐一送出並斷言結果的
窮舉測試**。

## 3. 身分綁定

Device Profile 的身分不是自己宣稱的，是 Transport 綁定出來的（`docs/aip/README.md` §5）：
iPhone 的 `source` 必須等於配對出來的 `{kind:"device", id:<deviceId>}`
（`docs/aip/transport-bindings.md` §0）。宣稱與綁定不符 → `identity-mismatch`，不「幫忙修正」。
第二種裝置（宣告式裝置線 v1.1，§6）的身分來源是那條線自己的 hello／配對機制：Runtime 依
`DeviceLink` 的期望身分綁定 `Party::device(<spec 的 deviceId>)`，AIP 層的規則不變。但兩條線的
**身分強度不同**，文件與稽核一律用這兩個字串、不寫「已驗證身分」：

| Transport | `identityStrength` | 意思 |
|---|---|---|
| iPhone wss v1 | `paired-token` | host 端逐次驗 sha256(token) |
| 宣告式裝置線 v1.1／v1.2（Serial／MQTT／BLE） | `transport-hello+device-side-pairing` | `hello.deviceId` 是**裝置自報的明文比對**；配對碼由**裝置端**比對（host 只送碼等 pair-ok）。`DeviceLink::pairing_unverified()` 為 true 時連配對碼是否被比對過都無法證明 |

### 3.1 成員同步模式（`syncProfile`；v0.7.0）

身分之外還有第二件不能含糊的事：**這個成員實際上拿得到多少共享狀態**。三種值
（`crates/interaction-runtime/src/character_session.rs::derive_sync_profile`）：

| `syncProfile` | 什麼時候 | UI 語意 |
|---|---|---|
| `full-state` | 出站通道沒有單則上限（iPhone wss），或它有上限但對端宣告 `aip.frag/1` 會重組 | **只有這個**可以顯示「已同步」 |
| `intent-only` | 有上限、不會重組，但成員宣告自己是 renderer（`role` 為 `remote-renderer`／`host-renderer`） | 只收得到放得進單則上限的意圖訊息；**不得**顯示「已同步」 |
| `event-source` | 有上限、不會重組，而且成員只送事件（`input-device`／`observer`） | 送得進來、收不回去；**不得**顯示「已同步」 |

它是 **Runtime 推導**出來的，不是新的 AIP 宣告欄位——`aip/1.0` 的 wire 沒有改。裝置不該（也不能）
自己宣稱「我拿得到完整狀態」：那是**那條線**的事實（有沒有上限、會不會重組），不是它的意見。
Runtime 只問通道兩件事（`DeviceOutbound::max_line_bytes`／`supports_fragmentation`）再配上已經協商好的
`role`。投影到 `GET /v1/character-session/diagnostics` 的 `members[].syncProfile`（查不到出站通道就
**省略**該欄位，不猜）與 `GET /v1/status` 的 `characterSessionSync`（非空才序列化），
並在協商完成時稽核 `aip.member-sync-profile{deviceId,transport,role,syncProfile,maxLineBytes,supportsFragmentation}`。

## 4. Presence／Heartbeat／Offline policy

- **Heartbeat**：`crates/interaction-session/src/session.rs::handle_heartbeat`（:958）更新
  `lastSeenAt`；投影到共享狀態有節流（`docs/aip/character-session.md` §12.7：只在距離上次投影
  超過 `presenceTimeout / 3` 時才更新，避免高頻 heartbeat 把 revision 打成無界成長）；
  `Submission.reply=false`——host **不**對 heartbeat 回 `result`（避免 result 迴圈）。
- **Presence 逾時**：`SessionConfig::presence_timeout_ms` 預設 45 000（
  `crates/interaction-session/src/types.rs:76`）；`tick(now)`（`session.rs`:570）掃描逾時成員標
  `Presence::Offline`。**這是 AIP Character Session 自己的 presence 逾時常數，與既有 CPP
  桌面 instance 的 `PRESENCE_STALE_SECS=20`（`crates/interaction-runtime/src/presentation.rs:39`）
  是兩個不同系統的不同常數**，不要混用——`docs/releases/v0.6.0-recovery-matrix.md` §2.4 記載過
  一個尚未解決的「45 秒註解對不上任何常數」的舊謎團（`lib.rs:1908`），那是 CPP 系統的問題，
  與這裡的 `presence_timeout_ms=45000` 是巧合的數字重複，不是同一件事，本文件明確拆開避免延續混淆。
- **Offline policy**：`character.behavior.*` 是 `drop-if-offline`（`crates/interaction-aip/src/offline.rs`）；
  只送給 presence 為 `online`、且把該 intent 協商成 `exact` 的 `remote-renderer`
  （`docs/aip/character-session.md` §12.8）。

## 5. 兩條實作：iPhone wss v1（完整）與宣告式裝置線 v1.1（Serial 經模擬器）

`docs/aip/transport-bindings.md` §1 的 iPhone wss v1 綁定是把 Device Profile
（`capability` 宣告→身分綁定→安全管線→resume）跑完整條路徑、且有最多端到端測試的 Transport：
`crates/interaction-runtime/tests/character_session_loop.rs`（14 個測試，走真 TLS wss、模擬
iPhone（fixture）——`connect`／`pair`／`hello` helper 在檔案內，非真機）與
`crates/interaction-runtime/tests/mobile_loop.rs::a_legacy_phone_that_never_negotiates_receives_no_aip_frames`
(:3759)。

## 6. 宣告式裝置線 v1.1（Serial／MQTT／BLE）：Serial 經 pty 模擬器驗證，真板為零

v0.6.0 時 `rg -n "aip|AIP" crates/interaction-adapter-declarative` 是零命中；v0.6.x（`e86afb9`）起
宣告式 adapter 有了正式的 AIP 綁定，**核心零變更**（`interaction-session`／`interaction-aip`／
`character_session.rs` 的 diff 為空，由獨立驗證者核對）：

- **線上訊息**：兩個方向都是一行 `{"type":"aip","envelope":{…}}`（`DeviceMsg::Aip`／`HostMsg::Aip`）。
  `proto` 仍為 1——`aip` 是 v1.1 的追加訊息：舊韌體不認得就忽略、舊 host 當未知訊息丟棄，兩端都不壞。
  參考韌體對入站 `aip` 明確忽略（放在 not-paired 閘門**之後**，未配對照舊回 `not-paired`）。
- **准入在傳輸層**：只有通過 hello 身分驗證＋配對握手、且連線世代未變的通道才收 `aip`
  （`DeviceLink::admit_aip`，比照 iPhone 的 auth-ok）。之前一律拒絕、計數並稽核
  `aip.rejected{stage:"transport-admission"}`，不靜默丟棄。
- **大小三層**：AIP profile 64 KiB → 這條線 `protocol::MAX_AIP_ENVELOPE_BYTES` 8 KiB（入站超限
  `RefusedTooLarge`；出站超限一個位元組都不寫）→ 傳輸自己（serial 單行 **639 bytes**＝參考韌體
  `g_serialBuf[640]`、MQTT 一則 **639 bytes**、BLE 一則 **480 bytes**＝`ble.rs::MAX_WRITE_BYTES`
  ——本文件先前寫 512 是錯的，512 是**韌體端**的入站門檻、host 端一直是 480；
  `parse_device_msg` 整行 16 KiB）。
- **分片（裝置線 v1.2，v0.7.0）**：見下面的 §6.3（§6.2 是位元組實測表，也就是分片的動機）。
- **Runtime 接線** `crates/interaction-runtime/src/declarative_session.rs`：Runtime 只認得型別抹除的
  `DeviceAipChannel`（沒有 serial／mqtt／ble 分支）。`register_declarative_spec` 同時宣告能力（受器；
  spec 標 `requiresConsent` 的＝高風險，id 只有 `receptor_consent_map` 一個產生點）、登記 `SensorSource`
  （stop-all＋等 ack；靜默＝`unknown`，不冒充已停；停止前先關本機輪詢旗標，高風險受器不自動恢復）、
  綁定收送迴圈（握手 500 ms→15 s 有上限退避、400 ms 輪詢窗、broadcast Lagged 稽核 `aip.inbound-lagged`）。
  撤銷／停用 provider → leave session＋retract 宣告＋unregister 來源＋abort 綁定 task；停用中的 spec
  不開綁定 task（重啟不得讓人類關掉的裝置在背景重新握手）。
- **綁定生命週期與免重啟重新綁定（AIP 1.0 澄清／v0.7.0）**：見下面的 §6.1。
- **身分強度**：`transport-hello+device-side-pairing`（§3）；稽核 `aip.device-channel-ready`／
  `aip.device-channel-lost`／`aip.device-retired` 帶 `identityStrength`／`transport`／`pairingUnverified`。
- **證據等級**：`crates/interaction-runtime/tests/declarative_session_loop.rs`（13 測；D1 的 7 支加上 D2 的「其他成員的廣播真的經序列線到達／放不進 639 bytes 的 patch 留痕」「身分不符與 session-binding 稽核記 transport=serial」「diagnostics identityStrength 三來源」「撤銷後出站表清空」「無通道成員的 no-channel 稽核」）走 **production
  `DeviceLink`＋serial adapter**，對端是 `scripts/esp32-serial-sim.py`（**pty 模擬器**；stdin 控制通道
  `aip-capability`／`aip-touch`／`aip-resume`／`aip-raw`，未配對拒絕送出）；`aip_link.rs` 6 測、
  `esp32_sim_conformance.rs` 的韌體／README／模擬器三方一致 2 測。韌體只有 `compile.sh` 編譯檢查。
  **ESP32 真板驗收為零**；MQTT／BLE 共用同一段 `AipChannel<L>` 程式碼，但沒有 AIP session 測試。
- **出站與 diagnostics（D2）**：Runtime 的 AIP 出站是型別抹除的登記表（`character_session::DeviceOutbound`，
  有界 64；iPhone 與宣告式裝置各自在認證／握手後登記、斷線／撤銷時移除），`character_session_send` 只問
  「這台裝置現在有沒有一條送得出去的線」——所以其他成員造成的 shared state 廣播**真的**會走序列線
  （測試以模擬器 log 的 `>>` 行證明；沒有通道時稽核 `aip.outbound-undeliverable{reason:"no-channel"}`，
  表滿時 `aip.outbound-rejected`）。入站稽核（`aip.rejected`／`aip.identity-mismatch`）的 `transport` 與
  `identityStrength` 由來源（`DeviceOrigin`）提供，不再寫死 `iphone`。diagnostics `members[]` 新增選填
  `identityStrength`（`paired-token`／`transport-hello+device-side-pairing`／`host-surface`；查不到出站通道
  就**省略**該欄位，不猜）。
- **已知限制**：(1) ~~參考韌體單行上限 639 bytes 讓 snapshot 與含 members 的 patch 完全送不到~~
  **v0.7.0 已修（裝置線 v1.2 分片，見 §6.3）**——但只對**宣告了 `aip.frag/1` 的裝置**。參考韌體
  本身仍然不宣告（沒有重組緩衝），所以它的行為完全不變：那些訊息仍然在寫上線前被拒絕並稽核
  `aip.outbound-undeliverable{envelopeBytes,reason:"over-line-limit-no-fragmentation"}`，
  而它的成員模式誠實降級成 `intent-only`（§3.1），介面不得說「已同步」；
  (2) 宣告式裝置沒有 recipe 相容的 touch observation id
  （iPhone 有 `iphone.touch`），沒有憑空發明一個；(3) 隔離測試的第二個成員是程序內 device fixture，
  不是 fake_iphone 子程序；(4) `crates/interaction-adapter-declarative` 的 `PROVIDER_LINKS` 是行程層 static
  （鍵＝provider id）：同一行程多個 Runtime 的測試必須用不同 provider id，否則會互相關掉對方的裝置線
  （這正是 D1 測試在預設並行下偶發失敗的根因，已修機具、產品時間預算未動）。

### 6.1 綁定生命週期（AIP 1.0 澄清／v0.7.0）

wire 版本不變（`proto` 仍為 1，`aip` 訊息形狀不變）：這一節說的是 **host 這一側**的狀態，
裝置端不需要任何改動。

每一台宣告式 provider 在 runtime 有一個**顯式**的綁定狀態
（`crates/interaction-runtime/src/declarative_lifecycle.rs`）：

| 狀態 | 意思 | `ProviderState` |
| --- | --- | --- |
| `Bound` | 連線＋能力宣告＋`SensorSource` 都在 | `Installed`／`Available`／… |
| `Rebinding{generation}` | 正在重新綁定；帶世代 | `Disconnected`（誠實：尚未連上） |
| `Unbound{disabled\|disconnected\|revoked\|removed}` | 綁定已拆掉，並說得出原因 | 由那個決定自己決定 |

> **`Bound` 不等於「連上了」**：`DeclarativeLifecycle::Bound` 在**綁定 task 啟動、通道登記進表**的當下就成立
> （`note_declarative_bound`），它說的是「這台裝置的 spec 與通道在登記表裡」，**不代表握手成功、也不代表對面有東西**。
> 唯一的例外是重新綁定進行中：那時狀態刻意留在 `Rebinding` 直到第 8 步握手 Ready。要判斷連線是不是真的活著，
> 看**裝置端的證據**（裝置線 `who` 的 `hello` 回覆與 `read` 讀回來的實際值、`character-session/diagnostics` 的成員與 `presence`、稽核的
> `handshake`），不要拿 `Bound` 當連線證明。這三個狀態本身也**沒有**經由 HTTP／CLI 直接曝光：人看得到的是
> `ProviderState`＋`detail` 的人話＋稽核事件，要斷言 `Unbound{Disabled}` 只能靠 Rust 測試
> （`crates/interaction-runtime/tests/declarative_session_loop.rs`）。

在此之前這只是一個布林集合（`declarative_rebind_pending`）：它說得出「綁定不在」，說不出
「為什麼不在」，也說不出「正在回來的路上」。於是把 provider 轉回 `Available` 只能誠實地印一句
`needs-restart-to-rebind`，使用者得重開 daemon 才拿得回一台自己剛剛停用又啟用的裝置。
那句話與它的稽核（`provider.needs-restart-to-rebind`）**已經移除**——它不再是事實。

**重新綁定的八個步驟**（`transition_provider` 只「允許再試一次」並啟動一條有界的背景任務；
它自己不做這八步，人類按下「啟用」的回應不該被一條要等握手的連線拖住）：

1. 停新請求：狀態進 `Rebinding{generation}`；舊裝置的出站通道從登記表移除。
   同時 `DeviceBinding` 對**入站** `aip` 加一道確定性閘門——綁定不是 `Bound` 就拒收並稽核
   `aip.rejected{stage:"provider-binding"}`。（只靠 `tasks.abort_all()` 不夠：abort 只在下一個
   await 點生效，一則已經在處理中的 frame 會照樣跑完，實測過的後果是裝置在
   `character.session.leave` 之後又 join 回來。）
2. 收斂進行中：有界等待舊連線的 in-flight 請求歸零（`DeviceAipChannel::in_flight`，預算 3 秒）；
   等不到就誠實記 `drained:false`，不假裝那些請求有結果。
3. 請來源停止並記錄結果：走**同一條** `SensorSource::request_stop`（reason `provider-rebinding`），
   結果原封不動進稽核。沒有登記來源就誠實地什麼都不報，不冒充「已停」。
4. 清 reader／task／subscription／`PROVIDER_LINKS` 舊項（`shutdown_provider_links`）。
5. 失效舊 connection generation：`DeviceLink::shutdown()` 把握手世代歸零，晚到的 ack／frame
   一律因世代不符被拒（`same_generation`）；稽核記下每條舊通道的 `readiness`／`handshakeInvalidated`。
6. 重新驗證設定與授權：`provider_off_reason` 仍是 `revoked`／provider 又回到停下來的狀態／
   spec 不再通過 `validate_spec` → 一律中止。**撤銷與移除永遠不重新綁定**
   （`UnboundReason::rebindable()` 為 false）。這一步與第 7 步共用 provider 的序列化鎖
   （`ProviderRegistry::lock_provider`），驗證通過與新連線登記之間插不進另一個決定。
7. 建新連線並握手協商：重新註冊整份 spec（能力回到**剛啟動時**的預設——需要 consent 的受器
   仍然是關的，高風險能力不會因為重新綁定而自己打開），然後在鎖**外面**有界等待握手
   （預算 20 秒；人類的停用不該被一台不回應的裝置擋住）。
8. 握手 Ready 之後才把 `ProviderState` 收斂為 `Available`，並把「重新連線中」那一句從 detail 拿掉。

整段有總預算（40 秒 watchdog）。任何一次未收斂的 rebind 被下一個決定接手時（又停用、又按一次
啟用、被撤銷），它的結果一律丟棄並留 `provider.rebind-superseded`——世代守衛讓晚到的完成回呼
改不動任何狀態。

**稽核**：`provider.rebinding`（開始，帶 generation 與舊連線描述）／`provider.rebound`（成功，帶
`drained`／`closedLinks`／`staleChannels`／`sensorStop`／`handshake`）／`provider.rebind-failed{reason}`
（失敗；狀態留在 `Disconnected`，detail 換成 `rebind-failed: …`）／`provider.rebind-superseded`。
純 HTTP 宣告式 adapter 沒有裝置線可握手，`handshake` 誠實記 `none`，不冒充「握手成功」。

**順序修正**：停用／撤銷現在是「先問來源停止 → 再關連線 → 最後翻能力旗標」。先關連線等於親手
拆掉唯一能問「你停了嗎」的那條線，然後永遠只能回答 `unknown`。

**證據等級**：`declarative_session_loop.rs` 的 `reenable_rebinds_without_restart` 走 **pty 模擬器**
（不是真板）：停用 → 離開 session → 啟用 → 同一行程重新握手 → 重新成為成員；
`rebind_generation_rejects_late_callbacks`／`revoke_during_rebind_does_not_resurrect`／
`rebind_timeout_is_bounded_and_honest` 用一份指向不存在序列埠的 spec。
`providers_loop.rs` 的 `re_enabling_a_declarative_device_rebinds_without_a_restart`／
`disable_a_does_not_affect_b`／`reentrant_transitions_do_not_double_retire` 走純 HTTP spec。
**ESP32 真板驗收仍為零。**

### 6.2 這條線上實際的位元組數（實測）

先有數字才有討論。下表全部由
`crates/interaction-runtime/tests/declarative_session_loop.rs::the_measured_wire_sizes_of_the_session_replies_are_over_the_line_limit`
**實跑量出來**（pty 模擬器經 production serial adapter；`cargo test … -- --nocapture` 會把
`MEASURED wire line bytes:` 逐行印出來），量的是**整行**
`{"type":"aip","envelope":…}` 編碼後的 UTF-8 bytes（不含換行）——不是 envelope 本身，也不是估算。

| 訊息 | 實測整行 bytes | 放得進 639 bytes？ |
|---|---|---|
| `capability` 回覆（協商第一則） | 318 | ✅ |
| `state{kind:"snapshot"}` 回覆（協商第二則，成員少時） | **814** | ❌ |
| `state{kind:"snapshot"}`（成員較多／resume 時） | **1019** | ❌ |
| `state{kind:"patch"}`**含** `members`（成員 presence／lastSeenAt 一動就整段重送） | **686 / 810** | ❌ |
| `state{kind:"patch"}` 不含 `members`（例如緊急停止的真相變更） | 515 | ✅ |
| `character.behavior.request`（行為意圖） | 442 / 548 | ✅ |

行上限本身：**serial 639**、**MQTT 639**（兩者都對齊參考韌體的 `g_serialBuf[640]`）、
**BLE 480**（`ble.rs::MAX_WRITE_BYTES`）。本文件先前把 BLE 寫成 512 是錯的——512 是**韌體端**
BLE 入站的門檻，host 端一直是 480。

### 6.3 分片（裝置線 v1.2，v0.7.0）

上表就是動機：一台「已加入」的 serial 裝置，在此之前連初始快照都拿不到。

wire 形狀（`proto` 仍為 1；`aip-frag` 與 `aip` 一樣是**追加**的訊息型別，舊韌體不認得就忽略、
舊 host 當未知訊息丟棄）：

```jsonc
{"type":"aip-frag","xfer":42,"seq":0,"total":3,"bytes":1019,"crc":"a1b2c3d4","data":"{\"specVersion\":…"}
```

* **能力宣告驅動**：只有 `hello.caps` 含 `"aip.frag/1"`（`fragment.rs:31`＝`FRAG_CAP`）才使用
  （出站閘門 `protocol.rs:769`＝`DeviceLink::supports_fragmentation` 用 `== Some(true)`；入站對稱地用
  `!= Some(true)`，所以**沒宣告**——包含完全沒有宣告 caps 的舊韌體——卻送分片進來的裝置在
  `protocol.rs:664`＝`accept_fragment` 就被拒絕，丟棄原因 `not-advertised`）。**參考韌體不宣告它**——真板沒有重組緩衝，替它
  宣稱就會讓 host 把一則 snapshot 切成好幾片送出去、全部被丟掉，而收據寫著「已送出」。
  模擬器有做（`scripts/esp32-serial-sim.py:140` 的 `--no-frag` 可關掉，用來驗降級路徑）。
* **不放寬任何上限**：每一片編碼後的整行仍然 ≤ 行上限（切片 `fragment.rs:160`＝
  `fragment_envelope_line`）；重組後仍受 8 KiB（`protocol.rs:39`＝`MAX_AIP_ENVELOPE_BYTES`，
  `fragment.rs:35`＝`MAX_REASSEMBLED_BYTES` 直接引用它）限制。切點只落在 UTF-8 字元邊界。
* **核心零變更**：組裝／重組完全在 `crates/interaction-adapter-declarative` 的
  `AipChannel`／`DeviceLink` 內部（`fragment.rs`＋`protocol.rs:132`＝`DeviceMsg::AipFrag`、
  `protocol.rs:172`＝`HostMsg::AipFrag`）。對呼叫端仍然是**一次** `send_aip`（`protocol.rs:786`）、
  **一則**完整的入站 envelope；`character_session.rs` 不認得 serial／mqtt／ble，也不認得「被分片」。
* **有界**：每台裝置同時 1 筆進行中（`fragment.rs:248`＝`Reassembler`；新 `xfer` 到達＝取消前一筆
  並稽核）、片數 ≤ 64（`fragment.rs:39`＝`MAX_FRAGMENTS`）、自最後一片起 2 秒逾時
  （`fragment.rs:42`＝`FRAGMENT_TIMEOUT`，由 `protocol.rs:728`＝`expire_fragments` 收走）；
  hello／斷線／revoke／stop-all／rebind 一律取消。待回報的丟棄稽核排成先進先出的有界佇列
  （`protocol.rs:505`＝`PENDING_FRAGMENT_AUDITS`＝8；滿了丟最舊的一筆並計數
  `DeviceLink::fragment_audit_overflow`——有界要付的代價必須數得出來）。
* **出站也是「每裝置 1 筆進行中」**：`send_aip` 的分片迴圈由每條 link 一把
  `tokio::sync::Mutex`（`protocol.rs:500`＝`DeviceLink::outbound`）序列化，兩則併發的大 envelope
  不會在線上交錯成 `A0 B0 A1 B1`（那會讓對端的重組器把兩筆都丟掉，而兩個呼叫端都拿到 `Ok`）。
  等待有界：`timeout` 就是這一則的有效期，排不到就誠實回「沒送出」。中途寫失敗且**已經寫出過片**
  時回 `LinkError::Uncertain`（不是 `Refused`）——線上已經有位元組，「什麼都沒送」是假話；
  呼叫端據此稽核 `aip.outbound-undeliverable`。
* **整筆丟棄**：缺片／重片／亂序／截斷／惡意 `total`／`bytes`／crc32 不符／組回來不是 JSON →
  整筆丟掉並稽核 `aip.fragment-dropped{xfer,reason,received,total}`。半份 envelope 絕不交給上層——
  那會把「傳輸壞了」演成「裝置說了一句沒有意義的話」。
* **不支援時的行為完全不變**：對端沒宣告 `aip.frag/1` 而 envelope 又放不進行上限 → 一個位元組都不寫，
  稽核原因 `over-line-limit-no-fragmentation`（`protocol.rs:43`＝
  `REASON_OVER_LINE_LIMIT_NO_FRAGMENTATION`），成員模式降級成 `intent-only`／`event-source`（§3.1）。

**證據等級**：`aip_fragment.rs` 17 測（切片邊界、UTF-8 不切壞、缺片／重片／亂序／截斷／惡意表頭／
crc／逾時／取消、crc32 標準向量）、`aip_link.rs` 14 測（`MockRawLink`：切片、降級拒絕、入站重組、
未握手拒絕、重連取消）、`esp32_sim_conformance.rs`（韌體忽略 `aip-frag` 且**不**宣告 `aip.frag/1`、
模擬器宣告且 `--no-frag` 可關、host 切的每一片模擬器都組得回來、亂序整筆丟棄）、
`crates/interaction-runtime/tests/declarative_session_loop.rs` 的四支（走 production serial adapter
＋pty 模擬器）：`::a_fragmenting_device_receives_the_snapshot_and_is_a_full_state_member`（snapshot 與含
`members` 的 patch 經分片**真的到達**並成為 `full-state` 成員）、
`::a_device_without_fragmentation_degrades_to_intent_only`（`--no-frag` 降級成 `intent-only`）、
`::a_reconnected_device_resumes_and_gets_the_state_it_missed`（重連 resume）、
`::an_interrupted_inbound_transfer_is_audited_not_silently_dropped`（被取消的傳輸留稽核）。
**ESP32 真板驗收仍為零**；MQTT／BLE 共用同一段 `AipChannel<L>` 程式碼，但沒有 AIP session 測試。

行號對應 `9799b1e`（`fragment.rs` 443 行）。行號會漂，`＝` 後面的符號名才是錨點——對不上時以符號名為準。
