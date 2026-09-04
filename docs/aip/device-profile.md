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
未來若要接上第二種裝置（例如 ESP32），身分綁定的來源會是那個 Transport 自己的配對／認證機制，
AIP 層的規則不變——但**這只是設計上的可延伸性，1.0 沒有第二個裝置實作**（§5）。

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

## 5. iPhone 是目前唯一實作

`docs/aip/transport-bindings.md` §1 的 iPhone wss v1 綁定是唯一把 Device Profile
（`capability` 宣告→身分綁定→安全管線→resume）跑完整條路徑的 Transport，且有端到端測試：
`crates/interaction-runtime/tests/character_session_loop.rs`（14 個測試，走真 TLS wss、模擬
iPhone（fixture）——`connect`／`pair`／`hello` helper 在檔案內，非真機）與
`crates/interaction-runtime/tests/mobile_loop.rs::a_legacy_phone_that_never_negotiates_receives_no_aip_frames`
(:3759)。

## 6. ESP32／BLE／Serial：誠實標「未實作」

```bash
rg -n "aip|AIP" crates/interaction-adapter-declarative
```

本次核實**零命中**——`crates/interaction-adapter-declarative`（YAML→HTTP/SSE／Serial／MQTT／BLE
宣告式裝置 adapter）目前完全沒有任何 AIP 相關程式碼，也沒有 Device Profile 的 `capability` 宣告
邏輯。這些裝置目前透過既有的宣告式 adapter 協定（`protocol.rs` 裝置線協定 v1：hello 身分＋配對＋
cmd/ack＋dedupe）與 Runtime 溝通，那是一套獨立於 AIP 的既有系統，**不在 1.0 的 Device Profile
範圍內**。要讓 ESP32 之類的裝置成為 AIP session 成員，需要在 `interaction-adapter-declarative`
或對應的 Runtime 接線層新增一條 AIP frame 綁定（仿照 `mobile.rs` 的 `Some("aip")` 分支），
這是下一個 minor version 的設計項目，本文件不預先宣稱任何時程或形狀。
