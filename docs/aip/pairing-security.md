# 配對與連線安全：Bonjour、配對碼、Token、Revoke、Endpoint Migration

> 證據等級用字：本文件的每一條規則都標明對應的常數／函式 path:line（`git show HEAD:crates/interaction-runtime/src/mobile.rs`，
> `HEAD = edb168259980a03c470cabd26441aa2634a926dd`）與是否有測試函式名可查；只有列出測試函式名的規則
> 才算「有回歸測試鎖住」，其餘一律是「契約文字／程式碼存在，未見專屬測試」。本文件核對過程沒有執行
> `cargo test`，不宣稱任何測試「目前通過」。真機一律「未驗證」；`examples/fake_iphone` 與
> `mobile_loop.rs` 的模擬連線一律標「模擬 iPhone（fixture）」。

## 1. Bonjour 只是發現，不是信任

`MOBILE_ADVERTISE_ENV = "INTERACT_AI_MOBILE_ADVERTISE"`（`mobile.rs:75`）控制是否對外廣播 mDNS／綁
`0.0.0.0`；`mobile_advertise_enabled()`（`:100` 起）預設允許，只有明確設為 `0|false|off` 才降級成只綁
`127.0.0.1`（doc comment `:98-99`）。廣播內容只有服務型別（`MDNS_SERVICE_TYPE = "_interact-ai._tcp.local."`，
`:51`）與連線資訊，**不含任何驗證用的秘密**——Bonjour 廣播出去的東西讓攻擊者知道「這裡有一台
interact-ai 桌面」，僅此而已。真正的信任建立在後續三層：配對碼（§2）、TLS 指紋固定（§3）、每機
token（§4）。測試：`mobile_loop.rs::test_mode_never_advertises_bonjour_on_the_lan`、
`::serve_mode_can_disable_bonjour_via_env`。

## 2. 配對碼＋HMAC challenge

配對期是**單一、單次使用、記憶體內**的 `PairingSession { code, expires_at }`（struct 定義 `:615-618`），
持有於 `Mutex<Option<PairingSession>>`。`mobile_pairing_begin()`（`:2269-2306`）：

- 產生 6 位數字碼：`format!("{:06}", rand::thread_rng().next_u32() % 1_000_000)`。
- TTL：`PAIRING_TTL_SECS = 300`（`:54`，5 分鐘）。
- QR payload：`{"v":1,"host":host,"port":port,"fp":fp,"code":code}`（`:2288`）——host 是本機區網
  IP、`fp` 是伺服器 TLS 憑證指紋（§3）、`code` 是配對碼。
- 每開一段新配對期，上一次「被別人燒掉」的提示（`pairing_burned_at`）歸零（`:2278`）。

握手（WS loop，同一個連線內）：

1. `pair-request`（未認證）→ 若沒有存活的配對期，直接 `pair-fail`；否則產生 16-byte 隨機 nonce
   （`token_hex(16)`），回 `pair-challenge{nonce}`（`:3304-3306`）。
2. `pair-response{hmac}` → 先 `pairing.take()`（**取用即銷毀**，配對期只能用一次，`:3316-3330`）；
   若已過期回 `pair-fail{"pairing session expired"}` 且**配對期已經沒了**（因為 `take()` 已經拿走）。
3. HMAC 驗證：`HmacSha256::new_from_slice(code.as_bytes())`（以配對碼當 key）、`mac.update(nonce)`，
   十六進位比對 `v["hmac"]`（`:3331-3341`）。
4. **驗證失敗（配對碼錯）**：回 `pair-fail{"wrong pairing code"}`，且**整段配對期已經被銷毀**
   （步驟 2 的 `take()`），不允許暴力嘗試；額外設 `pairing_burned_at = now()`（`:3352`）並留兩筆稽核
   （`mobile.pair-failed`、`mobile.pair-burned-by-peer`，`:3354-3367`）——設計理由（程式碼註解
   `:3348-3351`）：區網上**任何** peer 都能用一次錯誤回應燒掉這段配對期，所以必須讓使用者在 UI 上看到
   「有別的裝置試過配對，請重新開始」，否則只會覺得配對莫名失敗。
5. 成功：產生 `device_id = "iphone-" + token_hex(8)[..8]`、`token = token_hex(32)`，寫入
   `PairedDevice { device_id, name, model, token_hash: sha256_hex(token), paired_at }`（`:3368-3385`），
   持久化失敗只留稽核 `mobile.pair-not-persisted`、不擋這次配對（誠實：重啟後這台手機要重新配對，
   `:3388-3398`）。

測試：`mobile_loop.rs::wrong_pairing_code_is_refused_and_session_burned`、
`::a_peer_that_burns_the_pairing_window_is_visible_to_the_user`。

## 3. TLS 指紋固定（Trust-On-First-Use）

伺服器端只產生自簽憑證與指紋（`mobile_cert()`）並嵌進 Bonjour TXT 記錄與配對 QR payload 的 `fp`；
**TOFU 的驗證邏輯在客戶端**，不在 `mobile.rs`：`apps/interaction-ios/InteractionCompanion/Services/ConnectionManager.swift`
的 `PinnedWebSocketDelegate`（class doc：「不看 CA、不看有效期以外的鏈——這就是 trust-on-first-use 的
明確語意」）：`didReceive challenge:` 從 `SecTrustCopyCertificateChain` 取出葉憑證、算 hash、與配對時
記下的 `fingerprint` 比對；不符 → `onFingerprintMismatch` 回呼＋`.cancelAuthenticationChallenge`，並把
`pinningRejected` 設真，後續連線失敗一律分類成 `.tlsMismatch`，**永不**被誤判成單純的位址變更
（`ConnectionFailureKind.classify(error:pinningRejected:)`）。

測試：iOS `ReconnectHintTests.swift::testTlsMismatchIsNeverReportedAsAddressChange`
（模擬器層，見 `docs/releases/v0.6.0-recovery-matrix.md` §3「證據層級對照」表；本文件未重新讀取
`ConnectionManager.swift` 的精確行號，此段行號留白，只引用類別與函式名）。

## 4. 每機 Token（sha256 儲存）

- `PairedDevice.token_hash`（`:610-612`）：doc comment 明講「SHA-256(token)——明文只在配對回覆傳給
  手機一次」。
- 明文只出現一次：`paired` 訊息 `{"type":"paired","deviceId":device_id,"deviceToken":token}`
  （`:3418-3421`）。
- 重連比對：`auth{deviceId, token}` → `d.token_hash == sha256_hex(token.as_bytes())`（`:3427-3431`）。
- **失敗一律回同一句籠統文案**：`{"type":"auth-fail","reason":"unknown device or bad token (possibly
  revoked)"}`（`:3453`），不區分「裝置不存在」與「token 錯」——但**稽核**（`mobile.auth-failed`）會
  記錄 `knownDevice: bool`（伺服器內部知道差異，只是不告訴連線對象，`:3438-3450`）；**稽核內容不含
  token 本身**（doc comment `:3436-3437`：「token 本身永遠不記」）。
- **公開身分指紋 ≠ 認證驗證值**：`mobile_identity_fingerprint(device)`（`:1671-1679`）＝
  `sha256_hex("mobile-identity:v1:{device_id}:{token_hash}")`——doc comment 明講理由：`token_hash`
  本身就是 auth 比對用的驗證值，若直接公開它等於把驗證值送給每個讀得到裝置清單的人；再雜湊一層並綁
  `device_id` 之後，這個公開值穩定、可重現，但**推不回**驗證值。

測試：`mobile_loop.rs::token_reconnect_works_until_revoked`、
`::the_public_device_fingerprint_is_not_the_token_verifier`。

## 5. Revoke 即斷線

`mobile_revoke(device_id)`（`:2366` 起）：

1. 先從 `devices` map 移除（`:2367`）；找不到回 `NotFound`。
2. **落地失敗回滾**：`mobile_persist_devices()` 失敗重試一次，仍失敗就把裝置**放回**記憶體表並回
   `Err`（`:2373-2388`）——doc comment：「落地失敗＝撤銷沒有真的發生（重啟後 token 會復活）。誠實回
   Err 並把裝置放回記憶體表，不留下『UI 說撤銷了、其實沒有』的假象」。
3. **立即斷線**：`conn.close.cancel()`（CancellationToken）。連線 handler 的 `select!` 在
   `close.cancelled()` 分支判斷「這個 device_id 現在還在 `devices` map 裡嗎」——**revoke 已經先移除**，
   所以 `still_paired = false` → `closed_by_server = Some("revoked")` 並送
   `{"type":"auth-fail","reason":"revoked"}`（`:3179-3188`）。
4. 同一批收斂：`stop_sensors.mark_disconnected()`、在途 act 標記結果未知
   （`fail_pending_for_device`）、若正在串流麥克風補一則 `SensorStopped{reason:"revoked"}`
   （`:2394-2410`）、provider 轉 `ProviderState::Revoked`（`:2426-2432`）、
   **立即** `character_session_leave`（不是 offline——`:2437-2438`，doc comment：「撤銷＝立刻退出
   Character Session（不是 offline：這台裝置不再是成員）」）、關閉高風險受器
   （`mobile_disable_high_risk_receptors`）、並請其他仍在串流的手機也停止感測
   （`mobile_stop_other_streaming_phones`）。

測試：`mobile_loop.rs::a_revoke_that_cannot_be_persisted_fails_honestly`、
`::revoke_disconnects_live_connection_immediately`（斷言 2 秒內收到 `auth-fail{reason:"revoked"}`）。

## 6. conn_id 守衛與 "superseded"

`ConnState.conn_id: u64`（`:622`，doc comment：「每條連線唯一序號：收尾時只移除自己的表項（重連後的
新連線不受影響）」）。同一 `device_id` 的新連線取代舊連線時，`close.cancelled()` 分支同樣被觸發，但
`still_paired` 這時是 `true`（撤銷才會先移除 map），所以走的是**不同分支**：
`closed_by_server = Some("superseded")`（`:3185`），**不送** `auth-fail`（靜默關閉）——doc comment
`:3178`：「撤銷 → 明確告知（iOS 收到 auth-fail 會停止自動重連）；被同一台手機的新連線取代 →
靜默關閉（不能誤報成撤銷）」。收尾時 `Some("superseded") => {}`（`:3694`）——不執行撤銷／斷線那一套
收斂（不停感測、不發 revoked 稽核），因為裝置仍然是被連著的（只是換了一條連線）。

**無測試**：`docs/releases/v0.6.0-recovery-matrix.md` §2.4 第 11 點已記載
`rg -n "superseded" crates/interaction-runtime/tests/` 零命中，本文件核對過程獨立重跑同一個 `rg`
確認仍是零命中——**這條分支目前沒有專屬回歸測試**，是已知的測試覆蓋缺口，不是隱藏的。

## 7. Endpoint migration

- **同身分（同一台已配對的 iPhone，伺服器 TLS 指紋不變）**：`PairedDevice` 記錄本身**不含**host／port
  （struct 只有 `device_id/name/model/token_hash/paired_at`，`:605-613`）——伺服器不記手機的網路位址
  （TCP 是手機主動連過來），因此 token 的有效性與伺服器目前綁在哪個 IP／port **無關**：只要手機能連上
  正確的新位址並完成 TLS 指紋比對（§3）與 `auth{deviceId, token}`，`token_hash` 比對照樣成立，**不需要
  重新配對**。手機端要怎麼找到新位址，本文件核實到兩條路：(a) Bonjour 重新發現（若廣播仍開著，
  同一個 `fp` 對應新的 host/port，iOS 端拿到新位址即可重連）；(b) 使用者手動重新掃描／輸入桌面新產生
  的配對 QR payload——**但這條路實際上會觸發一次全新的 `pair-request`／`pair-response` 握手**
  （§2），依現有程式碼會產生**新的** `device_id`／`token`，不是「沿用舊 token、只換位址」的專用流程。
  桌面 `PhoneDeviceCard` 的「重新配對」按鈕（`docs/releases/v0.6.0-recovery-matrix.md` §2.4 第 5 點）
  對應的正是這條全新配對路徑——本文件誠實指出：**契約意圖與目前程式碼的落差**：任務書描述的
  「host/port 改變靠重新發現或重新輸入 payload，token 不變」在 (a) Bonjour 重新發現時成立（token
  確實不變），但在 (b) 重新輸入 payload 時，目前實作沒有「只更新位址、保留舊 token」的專用訊息，
  走的是完整重新配對（新 token）。這不是安全漏洞（重新配對本身是安全的），只是「token 不變」這句話
  只在 Bonjour 自動重新發現時精確成立。
- **不同身分（TLS 指紋不符）**：§3 的 TOFU 直接拒絕連線（`pinningRejected`），分類為 `.tlsMismatch`，
  **不得**被診斷成單純的位址變更，也就不會被導向「沿用舊 token」的任何路徑——iOS 端
  `ReconnectDiagnosis` 只對連續 4 次 `.connectivity`（非 TLS）失敗或持續 ≥60 秒才建議重新配對
  （`docs/releases/v0.6.0-recovery-matrix.md` §2.4 第 4 點；測試
  `ReconnectHintTests.swift::testFourConsecutiveConnectivityFailuresSuggestRepair`／
  `::testSustainedConnectivityFailuresOverSixtySecondsSuggestRepair`，模擬器層）。

## 8. Rate limit／連線上限／auth timeout／payload 上限

| 常數 | 值 | 位置 |
|---|---|---|
| 同時連線上限 | `MOBILE_MAX_CONNS = 8` | `:80` |
| TLS 交握逾時 | `MOBILE_TLS_HANDSHAKE_TIMEOUT = 5s` | `:82` |
| WS 交握逾時 | `MOBILE_WS_HANDSHAKE_TIMEOUT = 5s` | `:84` |
| 未完成配對／認證逾時（連了但沒認證就關） | `MOBILE_AUTH_TIMEOUT_DEFAULT_MS = 10_000` | `:87`；doc comment：「Ping、Pong 與未知訊息**不能續命**——否則未認證 peer 只要每 10 秒送一個 Ping 就能永遠佔著連線」 |
| 單一訊息／frame 上限 | `MOBILE_WS_MAX_MESSAGE_BYTES = 128 * 1024` | `:90` |
| 每連線每秒入站訊息數 | `MOBILE_MAX_INBOUND_PER_SEC = 30` | `:92` |
| accept 連續錯誤放棄門檻 | `MOBILE_ACCEPT_MAX_CONSECUTIVE_ERRORS = 20`，退避 25ms→50→…→800ms（上限 1s，`:3163`） | `:94` |

速率窗實作（`:3239-3260`）：以 1 秒為窗口滾動計數（`rate_window.elapsed() >= 1s` 就歸零），
`rate_count > MOBILE_MAX_INBOUND_PER_SEC` 就關閉連線並稽核 `mobile.rate-limited`（不含訊息內容）。
**這是連線層級的單一速率窗，`aip` frame 與 `pair-response`／`auth`／其他訊息共用同一個計數器**——
沒有另外針對 `aip` frame 的第二層 Transport 速率限制（AIP／Session 層另有自己的每成員速率限制，
屬於 `interaction-session`，不在 `mobile.rs`）。

測試：`mobile_loop.rs::a_flooding_connection_is_rate_limited_and_audited`、
`::an_oversized_message_closes_the_connection`。

## 9. Replay／Nonce

- **配對握手**：每次 `pair-request` 產生新的隨機 nonce（`token_hex(16)`），HMAC 綁定 nonce＋配對碼，
  且配對期**一次性**（`pairing.take()`）——舊的 `pair-response` 無法重放，因為配對期已經被銷毀
  （不論成功或失敗）。
- **`aip` frame 層**：`mobile.rs` 本身**沒有**為 `aip` frame 另外做 replay／dedupe——`Some("aip")`
  分支只是把 payload 轉呼叫 `character_session_device_frame`（`crates/interaction-runtime/src/character_session.rs`），
  真正的 messageId 去重環（256 筆，per session per source）與 deadline 過期檢查在
  `interaction-session` 裡（見 `docs/aip/threat-model.md` 對應列）。

## 10. Session membership 與 `aip` frame 的關係

配對成功、`auth` 成功只代表**Transport 層**認得這台裝置（`mobile.rs` 的 `devices` map）；要成為
**Character Session** 的成員，裝置還要另外送 `capability`（AIP §4.2）完成協商——這是兩層不同的信任：
Transport 信任（token）決定「這條連線允許存在」，Session membership（`join`）決定「這個身分可以參與
語意狀態同步」。沒送過 `capability` 的已配對裝置（例如舊版 App）永遠不會收到任何 `aip` frame
（`mobile_loop.rs::a_legacy_phone_that_never_negotiates_receives_no_aip_frames`），也不是 session 成員，
送 event 一律 `not-a-member`（`docs/aip/README.md` §8 安全管線順序）。細節見
`docs/aip/device-profile.md` §3、`docs/aip/threat-model.md`。
