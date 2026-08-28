# InteractionCompanion — iPhone 陪伴 App(v0.5 Phase 6)

跨 AI 能力感知平台的 iPhone 端:與桌面 `interact-ai` daemon 以
**wss(TLS + 憑證指紋固定)** 連線,提供動作/電池/麥克風音量感測、
觸覺/通知/朗讀/閃光/手電筒動器、簡化陪伴角色,以及 BLE 閘道。

> **誠實聲明(先讀這段)**
> 本目錄是**完整的 SwiftUI 原始碼交付**。驗收等級(2026-08-28 更新):
> - ✅ **iOS 模擬器驗收**(iPhone 17 模擬器、iOS 26.2 runtime、Xcode 26.6 / iOS 26.5 SDK):
>   以 `swiftc` 直接編成 `.app`(無 xcodeproj)、裝進模擬器啟動,並與**真實
>   `interact-ai` daemon** 完成 wss+TLS 指紋固定+HMAC 配對、Keychain 重連(`auth → auth-ok`)、
>   `character.present` 動器閉環(收據 `acknowledged` + `deviceApplied`)、撤銷後重連
>   `auth-fail` 顯示;XCTest 兩個測試檔在模擬器內執行 **19/19 通過**。
>   **第二輪(2026-08-28 晚)復測**:撤銷**即時斷線**(socket 於 DELETE 後
>   ≤0.035s 消失、App 立刻顯示「配對已被撤銷或過期」)、桌面 `emergency-stop`
>   **停掉 iPhone 感測**(`activeSensors` 立刻清空、手機 0.499s 內回報
>   mic/BLE/battery 全部 false、App 顯示「因桌面緊急停止而停用」)、
>   Bonjour 以 `_interact-ai._tcp` 實際廣播成功(`dns-sd -B` 看得到)。
>   證據截圖:`docs/assets/v05-evidence/ios-sim-*.png`(檔名前綴即標示模擬器)。
> - ✅ 全部 12 個 `.swift` 通過 iOS 模擬器目標的完整 `swiftc -typecheck`(0 錯誤、0 警告)
> - ❌ **未經真機驗收**:haptic / torch / CoreMotion / 真實 BLE / 通知顯示 / QR 相機掃描
>   在模擬器上不可用或未觸發,行為仍未驗證。
> - ❌ 未經 `xcodebuild`(沒有 xcodeproj);Xcode 專案仍需依下方步驟自行建立。

## 目錄結構

```
apps/interaction-ios/
├── README.md                          本文件
├── InteractionCompanionTests/
│   ├── MotionClassifierTests.swift    純分類器行為測試(XCTest;等價案例已於本機驗證)
│   └── ProtocolTests.swift            Wire protocol 編解碼測試(XCTest;等價案例已於本機驗證)
└── InteractionCompanion/
    ├── InteractionCompanionApp.swift  App 進入點 + 元件接線(scenePhase → 前景觀察)
    ├── Info.plist.example             需要加入 target 的權限描述(見下)
    ├── Models/
    │   └── Protocol.swift             Wire protocol v1 訊息模型(Codable,經 33 項測試)
    ├── Services/
    │   ├── ConnectionManager.swift    WebSocket + TLS 指紋固定 + 配對/認證 + 重連 backoff
    │   ├── PairingStore.swift         Keychain(deviceId/token/host/port/指紋;不存配對碼)
    │   ├── MotionSemantics.swift      CoreMotion → 語意事件(純分類器核心可測)
    │   ├── SensorCenter.swift         電池/前景/麥克風音量/位置權限(全部預設關閉)
    │   ├── ActuatorCenter.swift       haptics(purr/heartbeat)/通知/朗讀/閃光/手電筒/stop-all
    │   └── BleGateway.swift           CoreBluetooth central(使用者開啟才存在)
    └── Views/
        ├── ContentView.swift          分頁:連線 / 感測 / 角色 + 閃光覆蓋層
        ├── PairingView.swift          QR 掃描(VisionKit)或手動貼上 + 立即中斷
        ├── SensorsView.swift          感測開關 + 權限誠實顯示 + 「感測中」橫幅
        └── CharacterView.swift        簡化角色(貓耳剪影)+ 觸控事件
```

## Xcode 專案建立步驟

1. Xcode → **File → New → Project → iOS → App**
   - Product Name:`InteractionCompanion`,Interface:**SwiftUI**,Language:**Swift**
   - Minimum Deployment:**iOS 17.0**(使用了 `AVAudioApplication`、`onChange` 雙參數等 iOS 17 API)
2. 刪除範本產生的 `ContentView.swift` 與 `<App>App.swift`,把本目錄
   `InteractionCompanion/` 下的 `.swift` 檔(含 `Models/`、`Services/`、`Views/`
   子目錄)拖入專案(勾選 *Copy items if needed* 與 target membership)。
3. Target → **Info**:依 `Info.plist.example` 加入以下 key(缺少任一個,對應功能
   在第一次要求權限時會直接 crash):
   - `NSMicrophoneUsageDescription`(麥克風音量感測)
   - `NSCameraUsageDescription`(配對 QR 掃描)
   - `NSLocationWhenInUseUsageDescription`(位置權限回報)
   - `NSBluetoothAlwaysUsageDescription`(BLE 閘道)
   - `NSLocalNetworkUsageDescription` + `NSBonjourServices` = `_interact-ai._tcp`
     (RFC 6763 §7.2:service name label 最長 15 bytes;舊名 `interact-ai-mobile`
     為 18 bytes 會被 mDNS 拒絕,daemon 端已改用 `_interact-ai._tcp`)
4. **Capabilities:預設不加任何 Background Modes**(刻意——背景長駐不在 v1 範圍)。
5. Signing 選你的 Team,Build & Run。
6. 新增 **Unit Testing Bundle** target(`InteractionCompanionTests`),
   把 `InteractionCompanionTests/` 下兩個測試檔加入——皆為純邏輯,不需 mock。
   這兩個 XCTest 檔已在 **iOS 模擬器內實際執行 19/19 通過**(以 swiftc 直接編成
   `.xctest` bundle、`xctest` agent 執行;見下方「本機驗證了什麼」),Xcode 專案
   建好後直接 ⌘U 即可。

### DEBUG 限定啟動參數(自動化驗收,僅供模擬器/CI;release 不編入)

`DebugLaunchOptions`(`InteractionCompanionApp.swift`,整段在 **`#if DEBUG`** 內)
讀取以下啟動參數 / 環境變數(用 `simctl` 傳環境變數時要加 `SIMCTL_CHILD_` 前綴)。
每個選項都只是**替使用者做一個他本來就能在 UI 上做的動作**,不繞過任何
配對、權限或政策檢查:

| 參數 | 環境變數 | 等同的使用者動作 |
|---|---|---|
| `--pairing-payload '<json>'` | `INTERACT_PAIRING_PAYLOAD` | 把 JSON 貼進「手動貼上」並按「開始配對」(`PairingView.applyPayloadText` 同一路徑) |
| `--auto-connect` | `INTERACT_AUTO_CONNECT=1` | 已配對時按「連線」(`ConnectionManager.connectIfPaired`,走 Keychain token 的 `auth → auth-ok`) |
| `--initial-tab pairing\|sensors\|character` | `INTERACT_INITIAL_TAB` | 啟動後點該分頁(截圖用;有 `--pairing-payload` 時一律停在「連線」) |

配對 JSON 即桌面端 `POST /v1/mobile/pairing-session` 回傳的 `payload`
(`{"v":1,"host":…,"port":…,"fp":…,"code":…}`);在模擬器上請把 `host` 改成
`127.0.0.1`(模擬器與主機共用網路堆疊)。每個選項每次啟動只套用一次。

```bash
# 首次配對
xcrun simctl launch booted dev.interact-ai.companion \
  --pairing-payload '{"v":1,"host":"127.0.0.1","port":18790,"fp":"<64-hex>","code":"123456"}'
# 之後:用已存的 Keychain 憑證重連,並直接開「角色」頁
xcrun simctl launch booted dev.interact-ai.companion --auto-connect --initial-tab character
```

若 App 已有配對資料,`--pairing-payload` 仍會啟動新配對(配對成功即覆寫
Keychain;失敗則保留原配對)。Xcode 使用者可在 Scheme → Run → Arguments 加
同名參數。用 `swiftc` 直接編譯時需明確加 `-D DEBUG` 才會編入這些入口。

### 不用 Xcode 專案、直接以 `swiftc` 編成模擬器 .app(2026-08-28 實測可行)

`xcrun swiftc -sdk "$(xcrun --sdk iphonesimulator --show-sdk-path)" -target arm64-apple-ios17.0-simulator -parse-as-library -D DEBUG -module-name InteractionCompanion -emit-executable …`
即可產出可裝進模擬器的執行檔;兩個實測踩到的坑:

- **Keychain 需要 entitlements**:沒有 `application-identifier` /
  `keychain-access-groups` 時 `SecItemAdd` 回 `-34018`(errSecMissingEntitlement),
  配對會停在「配對成功但無法寫入 Keychain」。
- **模擬器不接受簽在 code signature 裡的 entitlements**(`codesign --entitlements`
  會讓 SpringBoard 拒絕啟動);要用 Xcode 對模擬器的同款做法:把 plist 與其 DER
  (`xcrun derq query -f xml -i ent.plist -o ent.der`)以
  `-Xlinker -sectcreate -Xlinker __TEXT -Xlinker __entitlements -Xlinker ent.plist`
  與 `… __ents_der … ent.der` 烘進執行檔,再 ad-hoc `codesign --sign -`。

## 配對流程

1. 桌面端顯示 QR(或給你 JSON):
   `{"v":1,"host":"192.168.x.x","port":18790,"fp":"<64-hex sha256(cert DER)>","code":"123456"}`
2. App 掃描或手動貼上 → 以 `wss://host:port` 連線,**只信任指紋等於 `fp` 的
   自簽憑證**(URLSession challenge 中比對 `SHA256(DER)`;不符直接斷線)。
3. 握手(每步一個 JSON text frame):
   - app → `pair-request`(裝置名稱 + `utsname.machine` 型號)
   - server → `pair-challenge`(nonce)
   - app → `pair-response`,`hmac = HMAC-SHA256(key: 配對碼 UTF-8, msg: nonce UTF-8)`
   - server → `paired`(`deviceId` + `deviceToken`)或 `pair-fail`
4. `deviceId / deviceToken / host / port / fp` 存入 **Keychain**
   (GenericPassword、ThisDeviceOnly)。**配對碼不儲存**。
5. 之後重連:`auth` → `auth-ok` / `auth-fail`。
   - `auth-fail` 時 UI 顯示「**配對已被撤銷或過期,請重新配對**」,
     停止自動重連;Keychain **只在使用者按「解除配對」時**清除。
6. 連線中斷:自動以 1s → 15s 指數 backoff 重連;
   **麥克風/位置/BLE 閘道立即自動停用,重連後不自動恢復**(需使用者重開)。

## 感測與動器:權限與限制表

### 感測(全部預設 OFF;開啟 = 使用者切換 + 系統權限同時成立)

| 感測 | receptor | 需要權限 | 上報內容與限制 |
|---|---|---|---|
| 動作 | `iphone.motion` | 無(CoreMotion) | 僅語意事件 `lifted/shaken/placed/rotated` + ISO8601 時間;原始樣本只存記憶體 3 秒滑動視窗;每種事件 debounce ≥ 1.5s;不支援 deviceMotion 的裝置顯示「不可用」 |
| 電池 | `iphone.battery` | 無 | `{level, charging, foreground}`,變更時上報;電量未知(模擬器)誠實回 `null` |
| 麥克風音量 | `iphone.mic-level` | 麥克風 | 僅 0.0–1.0 音量值,最多 2 次/秒;**絕不傳原始音訊**;權限被拒 → status 回報 `denied`,開關彈回 |
| 位置 | (無 observation) | 定位(使用期間) | **v1 協議未定義位置觀察,App 不送任何座標**;開關只影響 status 旗標與權限回報 |
| 觸控 | `iphone.touch` | 無 | 角色頁 tap / longpress;未連線時誠實提示「未送出(已丟棄)」 |

status 訊息(`sensors` 五旗標 + `microphone/location/bluetooth` 權限)於
**每次變更 + 每 30 秒**送出;30 秒定時同時做 WebSocket ping watchdog。

### 動器(每個 act 必回 `ack`(含 applied)或 `err`(含原因))

| act | 需要權限 | 限制與誠實語意 |
|---|---|---|
| `haptic.pulse` | 無 | style `light/medium/heavy/purr/heartbeat`,count 1–5;間隔 < 500ms → `err "rate-limited"`;無 CoreHaptics 時降級 UIImpact 並在 `applied.engine` 註明 |
| `notify.show` | 通知 | 權限被拒 → `err "notification-permission-denied"`;applied 為 `scheduled: true`(**scheduled ≠ 已顯示**) |
| `tts.speak` | 無 | ≤ 200 字,zh-TW 語音;applied 為 `started: true`(**started ≠ 唸完**) |
| `screen.flash` | 無 | 僅前景,否則 `err "background"`;durationMs ≤ 1500 |
| `torch.set` | 無 | 無手電筒硬體 → `err "no-torch"`;開啟需 durationMs ≤ 5000,到時自動關 |
| `character.present` | 無 | 狀態 `idle/working/waiting/verified-success/failed/unknown/emergency`;**綠色勾號只在 verified-success**;emergency 固定顯示「緊急停止中」 |
| `stop-all` | 無 | 立即停止 haptics/tts/torch/flash → `{"type":"ack","stopAll":true}` |

## BLE 閘道

- 預設 OFF;**使用者開啟那一刻才建立 `CBCentralManager`**(即那時才觸發藍牙權限詢問)。
- `ble.scan`(durationMs ≤ 8000)→ `ble.result`(依 RSSI 排序;掃不到就回空清單,
  name 未知回 `null`,不編造)。
- `ble.connect` → ack/err(10 秒 watchdog → `err "connect-timeout"`)。
- `ble.gatt` read/write/subscribe:自動走 service → characteristic 探索鏈,
  10 秒 watchdog;read 與訂閱通知回 `ble.value`(訂閱通知沿用 subscribe 請求 id),
  write(withResponse)回 ack。
- 藍牙關閉/權限被拒/裝置斷線/閘道停用:**所有進行中請求與訂閱一律以 `err`
  收尾**(`bluetooth-off` / `bluetooth-denied` / `disconnected` / `gateway-disabled`),
  不留無主請求、不假裝有結果。
- 與桌面端斷線 → 閘道自動停用並斷開所有 BLE 連線;重連後不自動恢復。

## 本機驗證了什麼(可重現)

本機(macOS 26.2、Xcode 26.6 via `DEVELOPER_DIR`、iOS 26.5 SDK、Swift 6.3.3)實際執行並通過
(2026-08-28;證據截圖 `docs/assets/v05-evidence/ios-sim-*.png`,全部為**模擬器**):

1. 全部 12 個 App `.swift` 對 `arm64-apple-ios17.0-simulator` 完整 `swiftc -typecheck`:
   **0 error、0 warning**(初版 5 個 Swift 6 並行警告已修)。
2. 以 `swiftc` 直接編成模擬器 `.app`(無 xcodeproj;entitlements 以 `-sectcreate`
   烘進執行檔再 ad-hoc 簽),裝進 iPhone 17 模擬器啟動,與**真實 `interact-ai`
   daemon** 完成:wss+TLS 指紋固定+HMAC 配對 → `GET /v1/mobile/status` connected:true;
   Keychain 重連(auth→auth-ok);`character.present` 動器閉環收據 `acknowledged`
   + `deviceApplied`;撤銷後重連收到 `auth-fail` 並顯示「配對已被撤銷或過期」。
   模擬器實測也暴露了桌面端「撤銷不斷線」與 Bonjour 服務名過長兩個缺陷(已於桌面端修正)。
3. `MotionClassifier` 純核心(抽出 CoreMotion 包裝後在 macOS 編譯執行):
   11 項行為測試全過——lifted/shaken/placed/rotated 觸發、shaken 期間不誤報
   lifted、純靜止零事件、yaw 跨 ±π wrap、debounce ≥ 1.5s、滑動視窗 ≤ 3s。
4. Wire protocol:33 項編解碼測試全過——status 五旗標/三權限鍵名精確、
   motion 帶 `at` 而 battery 不帶、`{"type":"ack","stopAll":true}` 形狀、
   count 序列化為整數、`ble.result` 未知 name 為 `null`、規格中每一種
   server→app 訊息的解碼、配對 payload 驗證(拒 v≠1/壞指紋)、
   HMAC-SHA256(key=配對碼, msg=nonce) 與 `openssl dgst -sha256 -hmac` 參考值一致。

### 第二輪模擬器復測(2026-08-28 晚)

驗上一輪記為缺陷的兩件事是否真的修好。獨立 daemon(API `127.0.0.1:18831`、
mobile wss `18790`),`.app` 依修改後的 Swift 原始碼重編、`Info.plist` 的
`NSBonjourServices` 改為 `_interact-ai._tcp`。證據 `ios-sim-08..10.png`,
全部為 **iPhone 17 模擬器**(UDID `66067313-…`,iOS 26.2 runtime),**非真機**:
- **撤銷即斷線(已修)**:`DELETE /v1/mobile/devices/iphone-87b42264` 回
  `{"revoked":…,"wasConnected":true}`;`lsof` 對 daemon PID 輪詢,**第一個取樣點
  (DELETE 發出後 0.035s)該 ESTABLISHED 連線已消失**(上一輪為 +42s 仍在);
  `GET /v1/mobile/status` `devices:[]`;App 立即顯示紅點
  「配對已被撤銷或過期,請重新配對」(`ios-sim-08-revoke-immediate.png`)。
- **緊急停止停掉 iPhone 感測(已修)**:重新配對後在 App 感測頁打開
  電池 + 麥克風音量(以 `simctl privacy … grant microphone` 預先授權)+ BLE 閘道
  → `PATCH /v1/receptors/iphone.mic-level {"enabled":true}` +
  `session start --consent receptor:iphone.mic-level` →
  `GET /v1/status.activeSensors` 含 `iphone.mic-level`
  (`ios-sim-09-mic-active.png`);`interact-ai emergency-stop` 後
  **`activeSensors` 在 0.064s 清空、手機自報 `micLevel/bleGateway/battery`
  於 0.499s 全部轉 false**,App 顯示「因桌面緊急停止而停用(麥克風/位置/BLE 閘道)」
  (`ios-sim-10-estop-sensors-off.png`);audit 有 `mobile.estop-stop-sensors`
  `{"delivered":true,"sensors":true}` 與 `mobile.high-risk-receptor-disabled`。
- **Bonjour(已修)**:`GET /v1/mobile/status.bonjour` =
  `{"advertised":true,"error":null,"instance":"interact-ai-18790","service":"_interact-ai._tcp"}`;
  `dns-sd -B _interact-ai._tcp local` 確實看得到該 instance;daemon log **無 `mdns_sd ERROR`**。
- XCTest 依新原始碼重編後在模擬器內再跑一次:**19/19 通過**(exit 0)。

## 誠實已知限制

- **僅模擬器驗收、未經真機驗收**(見頂部誠實聲明)。模擬器上實測到的事實:
  `utsname.machine` 回 `arm64`(非 `iPhone17,x`);CoreMotion 顯示「不可用:此裝置
  不支援 deviceMotion」;藍牙權限顯示「已授權」但 BLE 閘道預設關閉,桌面端
  `POST /v1/mobile/ble/scan` 得到誠實的 `{"type":"err","reason":"ble-gateway-disabled"}`。
- **桌面端撤銷不斷線:桌面端已修復(本輪)**。原症狀:`mobile_revoke` 只移除
  conn 表項與 provider,連線本身仍 ESTABLISHED(模擬器實測撤銷後 +42s App 仍顯示
  「已連線」)。現在 daemon 會對該連線送 `{"type":"auth-fail","reason":"revoked"}`
  並主動關閉 socket,App 端**不必等下一次重連**就會顯示「配對已被撤銷或過期」。
  daemon 另加 15s ping / 45s idle 心跳,半開連線也會被斷開。
  **已於模擬器復測通過(2026-08-28 第二輪)**:socket 在 DELETE 後 ≤0.035s 消失、
  App 立刻顯示撤銷提示(`ios-sim-08-revoke-immediate.png`);另有
  `crates/interaction-runtime` 的 regression test
  (`revoke_disconnects_live_connection_immediately`)涵蓋。
- **daemon 的 Bonjour 廣播:桌面端已修復(本輪)**。原症狀:服務名
  `_interact-ai-mobile._tcp` 的 label 為 18 bytes,超過 RFC 6763 §7.2 的 15 bytes
  上限,`mdns_sd` 註冊失敗。現在 daemon 用 `_interact-ai._tcp`(11 bytes),
  註冊結果誠實顯示在 `mobile_status().bonjour`;失敗時仍可用 QR / 手動輸入
  host:port 配對(本 App 本來就不依賴 Bonjour 探索)。
  **已復測(2026-08-28 第二輪)**:`mobile_status().bonjour.advertised = true`、
  service `_interact-ai._tcp`、instance `interact-ai-18790`,並以 `dns-sd -B`
  在區網實際看到該 instance,daemon log 無 `mdns_sd ERROR`;
  App 的 `Info.plist` `NSBonjourServices` 也已跟著改成 `_interact-ai._tcp`。
  **仍未驗證的是「App 端主動用 Bonjour 探索 daemon」**——本 App 沒有這條路徑
  (只走 QR / 手動 host:port),`NSBonjourServices` 純粹是本機網路權限宣告。
- **桌面緊急停止會連低風險感測一起關掉**:`stop-all {sensors:true}` 走
  `SensorCenter.stopAllSensors(reason:)`,除了麥克風/位置/BLE 閘道,**電池也會被關**
  (模擬器實測:estop 後手機自報 `battery` 由 `true` → `false`)。這比平台不變量要求的
  更嚴格,代價是解除 estop 後使用者要手動把電池感測重新打開。
  **尚未驗證**:解除 estop(`emergency-stop --clear`)後手機端是否確實不自動恢復
  (依 `SensorCenter` 程式碼應為不恢復,但本輪沒有實測解除路徑)。
- **背景執行受 iOS 限制**:未申請任何 Background Mode。App 進入背景後
  WebSocket 會被系統暫停/收回,感測與動器停止;回前景後走重連流程。
  依平台不變量,**斷線後麥克風/位置/BLE 閘道不自動恢復**,需使用者重新開啟。
- **External Accessory / USB 不支援**(不在 v1 範圍)。
- QR 掃描使用 VisionKit `DataScannerViewController`,需 A12 以上晶片;
  不支援或相機被拒時 UI 誠實顯示並提供手動貼上備援。
- 手電筒、觸覺引擎、deviceMotion 依機型而異——App 對每項能力個別檢查,
  不可用時回 `err` / 顯示「不可用」,**不得假設所有 iPhone 相同**。
- 位置感測在 wire protocol v1 沒有 observation 定義:App 只回報權限/開關狀態,
  不送座標(協議擴充後再實作)。
- 通知的 `applied.scheduled: true` 表示已交給系統排程,不保證已顯示;
  TTS 的 `started: true` 不代表唸完——這是刻意的誠實階梯用詞。
- 連線訊息佇列有界(64 則):未連線或壅塞時觀察訊息會被丟棄並計數
  (「連線」頁的「丟棄的訊息」),不無界堆積、不假裝已送達。
