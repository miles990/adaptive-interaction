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
>   `auth-fail` 顯示;XCTest 兩個測試檔在模擬器內執行 **19/19 通過**
>   (當時的測試方法數;之後補了 stop-all 的測試,現為 21,見下方 2026-09-03 更新)。
>   **第二輪(2026-08-28 晚)復測**:撤銷**即時斷線**(socket 於 DELETE 後
>   ≤0.035s 消失、App 立刻顯示「配對已被撤銷或過期」)、桌面 `emergency-stop`
>   **停掉 iPhone 感測**(`activeSensors` 立刻清空、手機 0.499s 內回報
>   mic/BLE/battery 全部 false、App 顯示「因桌面緊急停止而停用」)、
>   Bonjour 以 `_interact-ai._tcp` 實際廣播成功(`dns-sd -B` 看得到)。
>   證據截圖:`docs/assets/v05-evidence/ios-sim-*.png`(檔名前綴即標示模擬器)。
> - ✅ 全部 12 個 `.swift` 通過 iOS 模擬器目標的完整 `swiftc -typecheck`(0 錯誤、0 警告)
> - ❌ **未經真機驗收**:haptic / torch / CoreMotion / 真實 BLE / 通知顯示 / QR 相機掃描
>   在模擬器上不可用或未觸發,行為仍未驗證。
>
> **2026-09-03 更新(xcodeproj 與真機路徑)**
> - ✅ 本目錄現在有**手寫的 `InteractionCompanion.xcodeproj`**(objectVersion 77、
>   同步資料夾 group、app + 單元測試兩個 target、共用 scheme),`xcodebuild -list`
>   看得到 scheme `InteractionCompanion`。
> - ✅ **模擬器 SDK 建置通過**:`xcodebuild -target InteractionCompanion -sdk iphonesimulator
>   -arch arm64 CODE_SIGNING_ALLOWED=NO build` → `** BUILD SUCCEEDED **`,
>   產出的 `Info.plist` 六個隱私 key 齊全、`CFBundleIdentifier=dev.interact-ai.companion`、
>   `UIDeviceFamily=[1]`、`MinimumOSVersion=17.0`。
> - ✅ **裝置 SDK 建置通過(未簽章)**:`-sdk iphoneos -arch arm64 -configuration Release
>   CODE_SIGNING_ALLOWED=NO` → `** BUILD SUCCEEDED **`;12 個 `.swift` 對
>   `arm64-apple-ios17.0` + iphoneos26.5 SDK 的 `swiftc -typecheck` 也是 0 error / 0 warning。
> - ✅ **XCTest 25/25 通過**(MotionClassifier 8 + Protocol 17,其中 4 個是驗證 stop-all 緊急狀態
>   誠實性的 async 測試——之前的 21/21 只算到 13 個 Protocol 測試,`repo` 內其實一直有 17 個,見
>   下方「2026-09-03」章節)——用 xcodebuild 產出的 app-hosted `.xctest`,注入 iPhone 17
>   **模擬器**(iOS 26.2)以 `simctl` 執行(見上方「跑 XCTest」指令)。**這是模擬器測試,與下面的
>   真機驗收是兩件事**。
> - ⚠️ **`xcodebuild -destination` 在本機無法解析任何 iOS destination**:Xcode 26.6 回報
>   「iOS 26.5 is not installed」(平台元件未下載,只有 iOS 26.2 模擬器 runtime),
>   連純 SwiftPM 專案也一樣,**不是本 xcodeproj 的問題**。要用 `-scheme … -destination …`
>   (含 `xcodebuild test`)的人請先在 Xcode → Settings → Components 下載 iOS 平台。
>   `scripts/device-build.sh` 偵測到這個情況時會自動改走 `-sdk iphoneos -arch arm64` 建置後再用
>   `devicectl` 安裝(不需要下載 8 GB 模擬器 runtime),見下方「真機」章節。
>
> **2026-09-03 更新(真機部分驗收)**
> - ✅ **iPhone 11(`iPhone12,1`,iOS 26.3.1)已完成真機安裝與部分驗收**:Developer Mode 開啟、
>   Xcode 登入 Apple ID(Personal Team)完成後,以 `device-build.sh`(平台元件未裝時自動走
>   `-sdk iphoneos` 建置 fallback)裝上手機並啟動,對真 daemon(區網 TLS,非 loopback)跑過
>   `device-acceptance.sh --grant-consent` 的大多數列——配對、haptic/notify/tts/torch/flash、
>   角色六態、AI 偽造 emergency/verified-success 被 runtime 擋下、停止所有感測、緊急停止投影＋
>   停感測、解除不自動恢復、撤銷、觀察 battery/touch/mic-level、BLE scan。**尚未涵蓋**：
>   observe-motion(需人搖手機)、BLE connect/GATT(無測試用 peripheral)、系統終止 App 後的冷啟動
>   恢復。完整逐列證據見
>   [`docs/releases/v0.5.0-iphone-device-evidence.md`](../../docs/releases/v0.5.0-iphone-device-evidence.md)。
>   **不得**把 iPhone 寫成「真機驗收仍為零」；也不得把上述尚未涵蓋的列寫成「已驗收」。
> - ✅ **`device-acceptance.sh` 的三道 `--grant-consent` 閘門**都已在真機上實際觸發並代為打開：
>   (1) 啟用 iPhone 動器／受器(原本 disabled,plan 會回 `no-action`);(2) 合併 policy allowlist
>   缺少的 `iphone.*` 動器／通道(否則會被記成 `blocked(actuator.allowlist)`);(3) 建立 active
>   session 並授予同意(否則每一列 plan 都被 daemon 以 `session_inactive` 拒絕)。**三關都是
>   Governor 正確運作,不是手機或腳本的缺陷**——`--grant-consent` 讓腳本代你做「你本來就會自己做」
>   的授權動作,不會偷偷幫你同意任何原本沒問過的事。

## 目錄結構

```
apps/interaction-ios/
├── README.md                          本文件
├── Info.plist                         真正編進 app 的隱私用途描述(刻意放在同步資料夾外)
├── InteractionCompanion.xcodeproj/    手寫專案(objectVersion 77 + 共用 scheme)
├── scripts/
│   ├── device-build.sh                真機:前置閘門 → xcodebuild → devicectl 安裝/啟動
│   └── device-acceptance.sh           真機:對真 daemon 跑驗收矩陣(只印 daemon 原文)
├── InteractionCompanionTests/
│   ├── MotionClassifierTests.swift    純分類器行為測試(XCTest:8 個 test 方法)
│   └── ProtocolTests.swift            Wire protocol 編解碼測試(XCTest:17 個 test 方法,含 4 個
│                                       stop-all 緊急狀態誠實性 async 測試)
└── InteractionCompanion/
    ├── InteractionCompanionApp.swift  App 進入點 + 元件接線(scenePhase → 前景觀察)
    ├── Info.plist.example             隱私描述的來源範本(內容已複製到上面的 Info.plist)
    ├── Models/
    │   └── Protocol.swift             Wire protocol v1 訊息模型(Codable)
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

## Xcode 專案(已在 repo 內,不用再手動建立)

`InteractionCompanion.xcodeproj` 是**手寫**的(objectVersion 77),沒有用
xcodegen / tuist,也不需要任何產生步驟:

- `PBXFileSystemSynchronizedRootGroup` 直接同步 `InteractionCompanion/` 與
  `InteractionCompanionTests/` 兩個資料夾——**新增 `.swift` 檔不用改 pbxproj**。
  唯一的例外清單是 `Info.plist.example`(排除於 target 之外,免得被當成資源複製進 bundle)。
- Target `InteractionCompanion`:app,bundle id `dev.interact-ai.companion`,
  iOS 17.0(用了 `AVAudioApplication`、雙參數 `onChange` 等 iOS 17 API),
  `TARGETED_DEVICE_FAMILY=1`(只有 iPhone),Debug / Release 兩個 configuration,
  `CODE_SIGN_STYLE=Automatic`、`DEVELOPMENT_TEAM` **刻意留空**(由指令列覆寫,
  repo 裡不寫死任何人的 Team ID)。Debug 有 `SWIFT_ACTIVE_COMPILATION_CONDITIONS=DEBUG`
  ——下方的 DEBUG 啟動參數只有 Debug 版本編得進去。
- Target `InteractionCompanionTests`:單元測試 bundle,`TEST_HOST` 指向 app,
  依賴 app target;`ENABLE_TESTABILITY=YES` 讓 `@testable import` 成立。
- `Info.plist`(在 `apps/interaction-ios/Info.plist`,**同步資料夾之外**)提供六個
  隱私 key,其餘(`CFBundleIdentifier` / `UILaunchScreen` / `MinimumOSVersion` …)
  由 `GENERATE_INFOPLIST_FILE=YES` 產生後合併:
  - `NSMicrophoneUsageDescription`(麥克風音量感測)
  - `NSCameraUsageDescription`(配對 QR 掃描)
  - `NSLocationWhenInUseUsageDescription`(位置權限回報)
  - `NSBluetoothAlwaysUsageDescription`(BLE 閘道)
  - `NSLocalNetworkUsageDescription` + `NSBonjourServices` = `_interact-ai._tcp`
    (RFC 6763 §7.2:service name label 最長 15 bytes;舊名 `interact-ai-mobile`
    為 18 bytes 會被 mDNS 拒絕,daemon 端已改用 `_interact-ai._tcp`)
- **Capabilities:預設不加任何 Background Modes**(刻意——背景長駐不在 v1 範圍),
  也沒有 `.entitlements`:自動簽章會注入 `application-identifier` /
  `keychain-access-groups`,`PairingStore` 用預設 keychain group 就夠。
- 共用 scheme 在 `xcshareddata/xcschemes/InteractionCompanion.xcscheme`,
  所以 `xcodebuild -scheme InteractionCompanion` 在 CI 上也看得到。

```bash
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
cd <repo root>
xcodebuild -list -project apps/interaction-ios/InteractionCompanion.xcodeproj

# 模擬器建置(不需要任何 Apple 帳號)
xcodebuild -project apps/interaction-ios/InteractionCompanion.xcodeproj \
  -scheme InteractionCompanion -destination 'generic/platform=iOS Simulator' \
  CODE_SIGNING_ALLOWED=NO build

# 跑 XCTest(iPhone 17 模擬器)
xcodebuild test -project apps/interaction-ios/InteractionCompanion.xcodeproj \
  -scheme InteractionCompanion -destination 'platform=iOS Simulator,name=iPhone 17' \
  CODE_SIGNING_ALLOWED=NO
```

> ⚠️ **本機的已知環境限制**:上面兩條帶 `-destination` 的指令在這台機器上會失敗,
> 錯誤是 `iOS 26.5 is not installed. Please download and install the platform from
> Xcode > Settings > Components`——Xcode 26.6 的 iOS 平台元件沒下載(只有 iOS 26.2
> 模擬器 runtime),`xcodebuild` 因此列不出任何 iOS destination。**這與本 pbxproj 無關**
> (拿一個空的 SwiftPM iOS package 測也一樣)。在下載平台元件之前,可用不經 destination
> 解析的等價指令驗證專案(2026-09-03 實測皆 `** BUILD SUCCEEDED **`):
>
> ```bash
> xcodebuild -project apps/interaction-ios/InteractionCompanion.xcodeproj \
>   -target InteractionCompanionTests -configuration Debug \
>   -sdk iphonesimulator -arch arm64 CODE_SIGNING_ALLOWED=NO \
>   CONFIGURATION_BUILD_DIR=/tmp/ios-out OBJROOT=/tmp/ios-obj SYMROOT=/tmp/ios-sym build
> ```
>
> 產出的 `InteractionCompanion.app/PlugIns/InteractionCompanionTests.xctest` 可用
> `simctl` 注入模擬器執行(見下方「本機驗證了什麼」),2026-09-03 實測 **25/25 通過**
>（MotionClassifier 8＋ProtocolTests 17）。

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

### 不用 Xcode 專案、直接以 `swiftc` 編成模擬器 .app(2026-08-28 實測可行,**僅限模擬器**)

> 這條路只對**模擬器**成立。真機不吃 `-sectcreate` 的 entitlements 與 ad-hoc 簽章,
> 一定要走佈建描述檔 + Apple Development 憑證,也就是下面的「真機」章節。


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

## 真機(iPhone 實機安裝與驗收)

真機路徑只有兩支腳本,但**有四件事只有人做得到**,腳本會在做不到時
誠實停下來、印出你要做什麼,而不是硬跑一段最後噴簽章錯誤。

### 人要做的(一次性,依序)

| # | 你要做的事 | 為什麼腳本做不到 |
|---|---|---|
| H1 | Xcode → Settings → Accounts → 「+」→ Apple ID → 登入(免費 Apple ID 即可,會產生 **Personal Team**)。若被要求,到 developer.apple.com 接受最新開發者條款。 | 憑證與描述檔只能由 Apple 帳號簽發;免費 Team 沒有 App Store Connect API,無法用金鑰自動化。 |
| H2 | iPhone → 設定 → 隱私權與安全性 → **開發者模式** → 開啟 → **重新啟動** → 解鎖後點「開啟」並輸入密碼。 | 這是裝置端的安全開關,只能在手機上按;沒開的話 `devicectl` 任何安裝/啟動都會回 `CoreDeviceError 10005`。 |
| H3 | 第一次 `xcodebuild -allowProvisioningUpdates` 時,對跳出的**鑰匙圈視窗按「總是允許」**(必須在有登入的桌面工作階段執行,不能用 ssh)。 | 私鑰存取需要使用者授權。 |
| H4 | App 裝上去之後,iPhone → 設定 → 一般 → **VPN 與裝置管理** → 開發者 App → **信任**你的 Apple ID。 | 未信任的開發者憑證,系統直接擋下啟動。 |
| H5 | 第一次連線時按**「允許」本地網路**;之後用到麥克風 / 藍牙 / 通知 / 相機 / 定位時各按一次允許。手機與 Mac 要在**同一個 Wi-Fi**。 | iOS 權限對話框只能人按;AI 不得代替使用者同意。 |
| H6 | 驗收時親手**搖 / 拿起 / 旋轉手機**(`iphone.motion`)、**點角色**(`iphone.touch`),並用眼睛/手確認觸覺、手電筒、閃示、朗讀真的發生;截圖存 `docs/assets/v05-evidence/ios-device-*.png`。 | ack 只代表 App 回了訊息,**不代表效果真的發生**——這是誠實階梯的底線。 |

### 自動化的部分

```bash
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer

# 1) 只檢查前置條件(不編譯),看還缺什麼
apps/interaction-ios/scripts/device-build.sh --check-only

# 2) 編譯 + 簽章 + 安裝 + 啟動(可順便帶配對 JSON,等同在 App 裡貼上並按「開始配對」)
apps/interaction-ios/scripts/device-build.sh \
  --pairing-payload "$(curl -s -X POST http://127.0.0.1:8787/v1/mobile/pairing-session \
      -H "Authorization: Bearer $(cat ~/.adaptive-interaction/state/api-token)" \
      -H 'content-type: application/json' -d '{}' | python3 -c 'import sys,json;print(json.load(sys.stdin)["payload"])')"

# 3) 對真 daemon 跑驗收矩陣(預設不含破壞性列)
apps/interaction-ios/scripts/device-acceptance.sh --dry-run       # 先看要送什麼,不產生副作用
apps/interaction-ios/scripts/device-acceptance.sh
apps/interaction-ios/scripts/device-acceptance.sh --rows estop,estop-clear,revoke --confirm-destructive
```

`device-build.sh` 的閘門(任何一關不過就非 0 結束,**不會**先跑 xcodebuild):

1. `DEVELOPER_DIR` 指向完整 Xcode(不是 Command Line Tools)
2. `devicectl` 看得到**唯一一台** iOS 裝置(可用 `--device` / `IOS_DEVICE_ID` 指定)
3. 該裝置 `developerModeStatus == enabled`
4. 有 Team ID(`IOS_DEVELOPMENT_TEAM`,或 Xcode 的 `IDEProvisioningTeams` 第一筆)

通過之後才 `xcodebuild -destination id=<裝置> -allowProvisioningUpdates
-allowProvisioningDeviceRegistration DEVELOPMENT_TEAM=… build`,再
`xcrun devicectl device install app` 與 `device process launch`。
**UDID / Team ID / 配對碼一律不寫進 repo**:所有 `--json-output` 落在 `mktemp` 產生的
暫存目錄(結束即刪),印出配對 JSON 時 `code` 會被遮成 `******`。

`device-acceptance.sh` 涵蓋的列(`--list-rows` 可印出):
`pair`、`status`、`haptic`、`notify`、`tts`、`torch`(開+關)、`flash`、
`character`(idle/working/waiting/failed/unknown/emergency)、
`character-verified-rejected`(刻意送 `verified-success`,**期望被 runtime 擋下**)、
`observe-motion|battery|touch|mic`、`ble-scan`、`sensors-stop`、
`estop`、`estop-clear`、`revoke`。每一列印
`Requested / Effective / Dispatched / Acknowledged / deviceApplied / outcome`,
全部標示「真機 iPhone」。**腳本不做通過/不通過判定、不補假資料**:
沒回應就是 `未知(uncertain)`;缺同意時只印出「你自己要跑哪一行」,
不會偷偷幫你 `session consent`(除非你加 `--grant-consent` 明示授權)。

### 免費 Personal Team 的限制(會咬人的地方)

- **佈建描述檔 7 天到期**:過期後 App 直接拒絕啟動,每次驗收前都要重跑
  `device-build.sh` 重新簽章。腳本會在編完後印出描述檔到期時間。
- 每 7 天最多 **10 個 App ID**、同時最多 **3 台裝置**;超過會出現
  「No profiles for dev.interact-ai.companion」。
- 沒有推播權限(本 App 只用本地通知,不受影響)。

### 目前的實際狀態(2026-09-03)

本節先前記錄的狀態是 `device-build.sh --check-only` 在 Developer Mode 尚未開啟時停在第 3 關:

```
=== 2/5 Developer Mode ===
[閘門未通過] iPhone 的 Developer Mode 目前是「disabled」,devicectl 無法安裝或啟動 App。
```

H1~H2(Apple ID 登入、Developer Mode 開啟)已於 2026-09-03 完成,`device-build.sh` 五道閘門全過,
真機安裝與部分驗收也已完成(見上方「2026-09-03 更新(真機部分驗收)」與下方「真機驗收（2026-09-03）」)。
**仍未涵蓋**的列(observe-motion、BLE connect/GATT、系統終止 App 後的冷啟動恢復)不得寫成「已驗收」;
其餘列可以寫成「已在真機驗收」,完整逐列證據見
[`docs/releases/v0.5.0-iphone-device-evidence.md`](../../docs/releases/v0.5.0-iphone-device-evidence.md)。

## 停止全部感測的兩種原因(wire protocol 微調)

`stop-all` 可以帶 `"reason"`:

```jsonc
{"type":"stop-all","sensors":true,"reason":"user"}       // 使用者按了「停止所有感測」
{"type":"stop-all","sensors":true,"reason":"emergency"}  // 桌面緊急停止
{"type":"stop-all","sensors":true}                       // 舊桌面端:當成 emergency
```

- **`reason` 不改變停的範圍**——兩種原因停掉的東西完全一樣(動器,加上
  `sensors:true` 時的麥克風 / 位置 / BLE 閘道 / 電池 / 動作),兩種都**不自動恢復**。
  它只決定 App 在「感測」頁顯示哪一句:
  `user` → 「由桌面停止全部感測(麥克風/位置/BLE 閘道)」;
  `emergency` → 「因桌面緊急停止而停用(麥克風/位置/BLE 閘道)」。
- **缺席或無法辨識的值一律當成 `emergency`**:寧可把一般停止說得比較嚴重,
  也不要把真正的緊急停止淡化成一般停止(`StopAllReason(wire:)`,Protocol.swift)。
- stop-all 的回覆現在**回音 sensors 旗標**:`{"type":"ack","stopAll":true,"sensors":true|false}`,
  桌面端不必猜手機到底停了動器還是連感測一起停。
- **已於第二輪修復**:桌面端(`crates/interaction-runtime/src/mobile.rs`)現在會依觸發路徑送出
  `"reason":"user"`(使用者主動點「停止所有感測」)或 `"reason":"emergency"`(桌面緊急停止)——
  `STOP_REASON_USER`／`STOP_REASON_EMERGENCY`＋`stop_all_wire_reason()`,回歸測試
  `mobile_loop.rs::stop_all_wire_reason_only_calls_the_estop_path_emergency`。**仍為真的殘留**:
  桌面端尚未消費 App 端 ack 回聲的 `sensors` 欄位。

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
| `stop-all` | 無 | 立即停止 haptics/tts/torch/flash;`sensors:true` 時連感測一起停 → `{"type":"ack","stopAll":true,"sensors":<回音>}`;`reason` 只影響 UI 顯示的停用說明(見「停止全部感測的兩種原因」) |

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
3. `MotionClassifierTests`(repo 內,`InteractionCompanionTests/MotionClassifierTests.swift`,
   **8 個 `func test…`**)全過——lifted/shaken/placed/rotated 觸發、shaken 期間不誤報
   lifted、純靜止零事件、yaw 跨 ±π wrap、debounce ≥ 1.5s、滑動視窗 ≤ 3s。
4. `ProtocolTests`(repo 內,`InteractionCompanionTests/ProtocolTests.swift`,
   **13 個 `func test…`**,其中多個 test 方法各含數十個斷言)全過——
   status 五旗標/三權限鍵名精確、motion 帶 `at` 而 battery 不帶、
   `{"type":"ack","stopAll":true,"sensors":…}` 形狀與 sensors 回音、
   `stop-all` 的 `reason` 缺席/未知值一律降級為 `emergency`、
   count 序列化為整數、`ble.result` 未知 name 為 `null`、規格中每一種
   server→app 訊息的解碼、配對 payload 驗證(拒 v≠1/壞指紋)、
   HMAC-SHA256(key=配對碼, msg=nonce) 與 `openssl dgst -sha256 -hmac` 參考值一致。
   > 這兩個檔案在 2026-08-28 當下是**可重跑的回歸測試**(8 + 13 = 21 個 test 方法)。
   > 早期版本的 README 曾寫「33 項 / 11 項」,那是開發當下一次性檢查的斷言數,
   > 對應不到 repo 裡的任何測試,已更正為實際的 test 方法數。**`ProtocolTests.swift` 之後又
   > 新增 4 個 stop-all 緊急狀態誠實性測試(見下方「2026-09-03」與「XCTest 25/25」),目前是
   > 8 + 17 = 25 個 test 方法。**

### 2026-09-03:xcodeproj 與 XCTest(當時實測 21/21,之後補測到 25/25;仍是模擬器)

同一台機器(Xcode 26.6 / iOS 26.5 SDK / iPhone 17 模擬器 iOS 26.2):

1. `xcodebuild -list -project apps/interaction-ios/InteractionCompanion.xcodeproj`
   列出 target `InteractionCompanion`、`InteractionCompanionTests` 與 scheme
   `InteractionCompanion`。
2. `-target InteractionCompanion -sdk iphonesimulator -arch arm64 CODE_SIGNING_ALLOWED=NO build`
   → `** BUILD SUCCEEDED **`;產出的 `Info.plist` 六個隱私 key 齊全,
   `Info.plist.example` **沒有**被複製進 bundle(pbxproj 的 membershipExceptions 生效)。
3. `-target InteractionCompanion -sdk iphoneos -arch arm64 -configuration Release
   CODE_SIGNING_ALLOWED=NO build` → `** BUILD SUCCEEDED **`(裝置 SDK 編得過,只差簽章)。
4. 12 個 App `.swift` 對 `arm64-apple-ios17.0` + iphoneos26.5 SDK 的
   `swiftc -typecheck -D DEBUG`:**0 error、0 warning**。
5. `-target InteractionCompanionTests … build` 產出 app-hosted
   `InteractionCompanion.app/PlugIns/InteractionCompanionTests.xctest`;
   把 `_Testing_*.framework` 補進 `Frameworks/` 後 `simctl install` 並以
   `libXCTestBundleInject.dylib` 注入啟動(`-XCTest All`):
   當時(`ProtocolTests.swift` 只有 13 個 test 方法時)**Executed 21 tests, with 0 failures**
   (MotionClassifier 8 + Protocol 13)。
6. 反向驗證(把修好的行為改回舊寫法):`ack` 不回音 `sensors`、`stop-all` 忽略
   `reason` 時,同一組測試 **21 tests / 5 failures**——確認新測試真的抓得到迴歸。
7. **2026-09-03 補測(`docs-claims-070`)**:`ProtocolTests.swift` 在同一天內又新增 4 個 stop-all
   緊急狀態誠實性 async 測試(`testEmergencyStopAllSetsTheCharacterStateEvenIfCharacterPresentIsLost`／
   `testUserStopAllDoesNotFakeAnEmergencyCharacterState`／
   `testActuatorOnlyStopAllTouchesNeitherSensorsNorCharacterState`／
   `testOnlyTheRuntimeClearsTheEmergencyCharacterState`),但先前的執行紀錄一直停在 21/21,沒有人
   重跑過完整的 25 個。用同一套 `simctl` 注入流程重新執行:**Executed 25 tests, with 0 failures**
   (MotionClassifier 8 + Protocol 17)。這是目前 XCTest 的權威數字。

> ⚠️ 這一輪**全部在模擬器**,而且 `xcodebuild test -destination …` 在本機無法執行
> (Xcode 未安裝 iOS 26.5 平台元件,見上方 Xcode 專案章節的警告框)。**模擬器 XCTest 與真機驗收
> 是兩件事**——真機部分驗收見下方「真機」章節與
> [`docs/releases/v0.5.0-iphone-device-evidence.md`](../../docs/releases/v0.5.0-iphone-device-evidence.md)。

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

- **真機安裝與部分驗收已於 2026-09-03 完成**(見頂部「2026-09-03 更新」與下方「真機」章節)。
  Xcode 登入 Apple ID(Personal Team)、iPhone Developer Mode 開啟後,`scripts/device-build.sh`
  五道閘門全過,對 iPhone 11(`iPhone12,1`,iOS 26.3.1)裝上並啟動 App。**仍未涵蓋**的列:
  observe-motion(需人搖手機)、BLE connect/GATT(無測試用 peripheral)、系統終止 App 後的冷啟動
  恢復——這些列不得寫成「已驗收」;其餘列已在真機驗收,完整逐列證據見
  [`docs/releases/v0.5.0-iphone-device-evidence.md`](../../docs/releases/v0.5.0-iphone-device-evidence.md)。
- **`scripts/device-acceptance.sh --grant-consent` 已對真手機跑過**(2026-09-03):三道前置關卡
  (沒有 active session、iPhone 動器預設 disabled、policy allowlist 未含 `iphone.*`／通道)在加上
  `--grant-consent` 後依序打開,不再只是 `--dry-run`／`--list-rows` 層級的驗證。裡面每一列的斷言
  仍然寫成「印出 daemon 原文」而非判定,不讓腳本產生假的通過結論。
- **`xcodebuild -destination` 在本機不可用**:Xcode 26.6 沒安裝 iOS 26.5 平台元件,
  任何 iOS destination(含模擬器)都列不出來,連空的 SwiftPM package 也一樣。
  因此 CI/本機用 `-target … -sdk iphonesimulator`(模擬器)或 `-target … -sdk iphoneos`
  (真機,`device-build.sh` 偵測到這個情況會自動切換,見上方「真機」章節)驗證專案,模擬器測試用
  `simctl` 注入跑 XCTest。等平台元件裝好之後,README 上方那兩條 `-scheme … -destination …`
  指令才會通。
- **`stop-all` 的 `reason` 已於第二輪修復支援雙端**:桌面 runtime(`crates/interaction-runtime/src/
  mobile.rs`)現在依觸發路徑送出 `"reason":"user"`(使用者主動停止)或 `"reason":"emergency"`
  (桌面緊急停止),App 端據此顯示對應文案。**仍為真的殘留**:桌面端尚未消費 App 端 ack 回聲的
  `sensors` 欄位。
- **模擬器與真機各自的驗證範圍**(見頂部誠實聲明)。模擬器上實測到的事實:
  `utsname.machine` 回 `arm64`(非 `iPhone17,x`);CoreMotion 顯示「不可用:此裝置
  不支援 deviceMotion」;藍牙權限顯示「已授權」但 BLE 閘道預設關閉,桌面端
  `POST /v1/mobile/ble/scan` 得到誠實的 `{"type":"err","reason":"ble-gateway-disabled"}`。
  真機上 CoreMotion 可用、BLE scan 真的能掃到周邊(見下方「真機」章節),但 observe-motion 與 BLE
  connect/GATT 尚未在真機驗證。
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
  依平台不變量,**斷線後麥克風/位置/BLE 閘道不自動恢復**,需使用者重新開啟。**真機實測確認**
  (2026-09-03):App 切到背景後 daemon 於數秒內偵測斷線並強制停用高風險受器,這是 iOS 平台行為,
  不是本專案的缺陷,但**不得宣稱 App 能在背景永久保持連線**。
- **桌面 IP 變更需要重新配對**(真機實測發現,2026-09-03):App 沒有 Bonjour 探索,host 位址釘在
  配對當下——桌面 Wi-Fi 位址變更後(例如多接一張網卡),App 會用 Keychain 內的舊位址反覆重連,
  daemon 端完全收不到連線嘗試,`--auto-connect` 冷啟動也連不上;必須用新位址重新配對(新配對後
  數秒內連上,App 端會覆寫 Keychain)。
- **系統終止 App 後需要手動重新連線**(真機實測發現,2026-09-03):冷啟動的 App 不會自動重連,
  需要使用者點「連線」分頁的按鈕,或以 `--auto-connect` 啟動參數啟動(見上方 DEBUG 啟動參數表)。
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
