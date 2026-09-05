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
| 宣告式裝置線 v1.1（Serial／MQTT／BLE） | `transport-hello+device-side-pairing` | `hello.deviceId` 是**裝置自報的明文比對**；配對碼由**裝置端**比對（host 只送碼等 pair-ok）。`DeviceLink::pairing_unverified()` 為 true 時連配對碼是否被比對過都無法證明 |

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
  `RefusedTooLarge`；出站超限一個位元組都不寫）→ 傳輸自己（serial／參考韌體單行 639 bytes、BLE 512 bytes、
  `parse_device_msg` 整行 16 KiB）。
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
- **已知限制**：(1) 參考韌體單行上限 639 bytes：協商的第二則回覆（`state{kind:"snapshot"}`，實測 1019 bytes）
  與任何含 `members` 的 `state{kind:"patch"}`（成員 presence／lastSeenAt 變動會整段重送，實測 660–784 bytes）
  都在寫上線前被拒絕並稽核 `aip.outbound-undeliverable{bytes,reason}`——Serial 成員拿不到初始快照，也收不到
  成員／互動類 patch；只有不含 members 的小 patch（例如緊急停止的真相變更，實測 450 bytes）送得到。分段／
  per-member diff／縮減 profile 是協定層決定，未做；(2) 宣告式裝置沒有 recipe 相容的 touch observation id
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

