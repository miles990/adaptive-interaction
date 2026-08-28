# esp32-companion — 官方 ESP32 參考裝置韌體

adaptive-interaction 平台的官方參考硬體裝置。一片 ESP32-WROOM-32 DevKitC 加上
常見感測器與動器，透過**同一套 line-delimited JSON 協定**（proto 1）同時支援：

- **USB Serial**（115200 baud，一行一個 JSON object）
- **Wi-Fi / MQTT**（一則訊息一個 JSON object；無 TLS，僅限信任區域網路）
- **BLE**（可選，預設關閉；一次 write 一個 JSON object）

韌體版本 `1.0.0`。與 runtime 端宣告式 adapter（`transport: serial` / `transport: mqtt`）
搭配使用；YAML 範例見下方〈與 runtime 整合〉。

本韌體遵守平台誠實階梯：**ack 代表效果已在裝置上生效**（不是收到而已）；
`ack.applied` 回報的是經硬限制 clamp 後的**實際套用值**；讀不到的感測器誠實回
`-1` / `null`，絕不捏造。

---

## BOM（材料清單）

價格為台灣電子材料行／蝦皮的**粗估**行情（2026 年），僅供預算參考。

| 元件 | 數量 | 備註 | 約略價格 (TWD) |
|---|---|---|---|
| ESP32-WROOM-32 DevKitC（38-pin） | 1 | 主控板，USB micro-B | $180 |
| HC-SR04 超音波測距 | 1 | **ECHO 需 5V→3.3V 分壓**（1kΩ＋2kΩ） | $40 |
| DHT22（AM2302）溫濕度 | 1 | DATA 需 10kΩ 上拉；本協定僅回報溫度 | $120 |
| SG90 伺服馬達 | 1 | **必須外部 5V 供電**，與 ESP32 共地 | $60 |
| 共陰極 RGB LED（5mm） | 1 | 每色串 220Ω 限流電阻 | $5 |
| 光敏電阻（LDR 5528） | 1 | 與 10kΩ 組分壓 | $5 |
| 震動馬達（coin / 1027） | 1 | 經 NPN 電晶體驅動＋飛輪二極體 | $30 |
| 無源蜂鳴器（passive buzzer） | 1 | PWM 驅動；有源蜂鳴器不適用 | $15 |
| 按鈕（tact switch） | 1 | 接 GND，內部上拉 | $3 |
| NPN 電晶體（S8050 / 2N2222） | 1 | 震動馬達開關 | $5 |
| 二極體（1N4148 / 1N4007） | 1 | 馬達飛輪保護 | $3 |
| 電阻：220Ω×3、1kΩ×2、2kΩ×1、10kΩ×2 | 1 組 | LED 限流／分壓／上拉 | $20 |
| 麵包板＋杜邦線 | 1 組 | 830 孔麵包板 | $100 |
| 5V 外部電源（伺服＋馬達用） | 1 | 麵包板電源模組或 5V/2A USB | $50 |
| **合計** | | | **約 $640** |

---

## 接線圖

```
                         ESP32-WROOM-32 DevKitC
                        ┌────────────────────────┐
     RGB LED(共陰極)     │                        │
   R ──220Ω── GPIO25 ───┤ 25                  18 ├─── TRIG ── HC-SR04
   G ──220Ω── GPIO26 ───┤ 26                  19 ├─── ECHO ─┬─ 1kΩ ── HC-SR04 ECHO(5V)
   B ──220Ω── GPIO27 ───┤ 27                     │          └─ 2kΩ ── GND   ← 分壓!
   共陰極 ─────── GND    │                        │
                        │                     34 ├───┬── LDR ── 3V3
   按鈕 ─┬── GPIO32 ────┤ 32                     │   └── 10kΩ ── GND
        └── GND         │                      4 ├─── DHT22 DATA ─┬─ 10kΩ ── 3V3
   (INPUT_PULLUP)       │                        │                └─ DHT22(VCC=3V3)
                        │                     23 ├── 1kΩ ── NPN B極
   無源蜂鳴器 ─┬─ GPIO22 ┤ 22                     │   NPN C極 ── 震動馬達 ── +5V(外部)
             └─ GND     │                        │   NPN E極 ── GND
                        │                     13 ├─── SG90 訊號(橘)
                        │                        │    SG90 紅 ── +5V(外部)
                        │    GND ────────────────┤    SG90 棕 ── GND(共地!)
                        └────────────────────────┘
   HC-SR04: VCC=5V(VIN), GND 共地
   外部 5V 電源的 GND 必須與 ESP32 GND 相連（共地）
```

### 腳位表

| 功能 | GPIO | 模式 | 備註 |
|---|---|---|---|
| RGB LED — R | 25 | PWM 8-bit, 5kHz | 共陰極，220Ω 限流 |
| RGB LED — G | 26 | PWM 8-bit, 5kHz | 同上 |
| RGB LED — B | 27 | PWM 8-bit, 5kHz | 同上 |
| 按鈕 | 32 | INPUT_PULLUP | 另一端接 GND，按下=LOW |
| HC-SR04 TRIG | 18 | OUTPUT | |
| HC-SR04 ECHO | 19 | INPUT | **必經 1kΩ/2kΩ 分壓**，5V 直入會傷 ESP32 |
| 光敏電阻 | 34 | ADC（input-only） | LDR 接 3V3、10kΩ 接 GND、中點進 34 |
| DHT22 DATA | 4 | 單線 | 10kΩ 上拉至 3V3 |
| 震動馬達 | 23 | PWM 8-bit, 2kHz | 經 NPN；馬達兩端並聯飛輪二極體（負極朝 +5V） |
| SG90 訊號 | 13 | 50Hz servo PWM | 馬達電源用**外部 5V**，勿吃板上 3V3/5V |
| 無源蜂鳴器 | 22 | PWM 10-bit | duty 韌體硬上限 50% |

---

## Flash 步驟（Arduino IDE 2.x）

1. **安裝 ESP32 板定義**
   - `File → Preferences → Additional boards manager URLs` 加入：
     `https://espressif.github.io/arduino-esp32/package_esp32_index.json`
   - `Tools → Board → Boards Manager` 搜尋 **esp32 by Espressif Systems**，
     安裝 **3.x** 版（本韌體以 core 3.x 的 LEDC API 撰寫，2.x 不相容）。
2. **選板子**：`Tools → Board → esp32 → ESP32 Dev Module`（預設參數即可）。
3. **安裝函式庫**（`Tools → Manage Libraries`）：

   | 函式庫 | 作者 | 版本 |
   |---|---|---|
   | ArduinoJson | Benoit Blanchon | ≥ 7.0（以 7.x API 撰寫） |
   | PubSubClient | Nick O'Leary | 2.8 |
   | DHT sensor library | Adafruit | ≥ 1.4.6 |
   | Adafruit Unified Sensor | Adafruit | 相依，一併安裝 |
   | ESP32Servo | Kevin Harrington / John K. Bennett | ≥ 3.0.5（支援 core 3.x） |
   | NimBLE-Arduino | h2zero | ≥ 2.1（**僅** `ENABLE_BLE 1` 時需要） |

4. **設定檔**：
   ```bash
   cd firmware/esp32-companion
   cp config.h.example config.h
   # 編輯 config.h：Wi-Fi、MQTT broker、DEVICE_ID、PAIRING_CODE
   ```
   `config.h` 已被 `.gitignore` 排除，不會進 git。
5. **上傳**：接上 USB，選對序列埠（macOS 常見 `/dev/cu.usbserial-0001`），
   按 Upload。若卡在 `Connecting...`，按住板上 **BOOT** 鍵直到開始燒錄。
6. **驗證**：開 Serial Monitor（**115200 baud**，行尾設 **Newline**），
   重置板子後應看到一行 `hello`：
   ```json
   {"type":"hello","deviceId":"esp32-companion-01","fw":"1.0.0","proto":1,"caps":["led.set","buzzer.beep","vibe.pulse","servo.move","sensors.read"],"pairing":true}
   ```

---

## 協定訊息一覽

| 方向 | 訊息 | 說明 |
|---|---|---|
| →裝置 | `{"type":"who"}` | 詢問身分；開機時裝置也主動送 hello |
| 裝置→ | `{"type":"hello","deviceId":..,"fw":"1.0.0","proto":1,"caps":[..],"pairing":bool}` | `pairing:true` = 此通道尚需配對 |
| →裝置 | `{"type":"pair","code":"..."}` | 近似常數時間比對 `PAIRING_CODE` |
| 裝置→ | `{"type":"pair-ok"}` / `{"type":"pair-fail"}` | |
| →裝置 | `{"type":"cmd","id":"..","nonce":"..","name":"led.set","params":{..}}` | id 必填；nonce 目前僅收不驗 |
| 裝置→ | `{"type":"ack","id":"..","applied":{..}}` | **applied = clamp 後實際值** |
| 裝置→ | `{"type":"ack","id":"..","dup":true}` | 重複 id：不重套效果（16 筆環形去重） |
| →裝置 | `{"type":"cancel","id":".."}` | 停掉該 id 進行中的計時效果 |
| 裝置→ | `{"type":"ack","id":"..","cancelled":true}` 或 `{"type":"err","id":"..","reason":"not-found"}` | |
| →裝置 | `{"type":"read"}` | 讀 state |
| 裝置→ | `{"type":"state","deviceId":..,"facts":{"button":bool,"distanceMm":int\|-1,"lux":int,"tempC":float\|null,"vibeActive":bool,"servoAngle":int,"led":{"r":..,"g":..,"b":..}}}` | 也會在按鈕邊緣與每 `STATE_PERIOD_MS`（預設 5000ms）自動推播 |
| →裝置 | `{"type":"stop-all"}` | 緊急停止；**不需配對**（fail-safe 方向） |
| 裝置→ | `{"type":"ack","stopAll":true}` | |
| 裝置→ | `{"type":"err","id":..,"reason":".."}` | `not-paired` / `bad-json` / `unknown-type` / `unknown-cmd` / `bad-params` / `rate-limited` / `not-found` |

指令參數：`led.set {r,g,b}`、`buzzer.beep {freqHz,durationMs}`、
`vibe.pulse {strength 0..1, durationMs}`、`servo.move {angle 0..180}`。

MQTT 主題（與 runtime adapter 對齊）：下行 `<prefix>/to-device`、上行
`<prefix>/from-device`。topic 不是身分——runtime 仍會驗 `hello.deviceId` 與配對碼，
所以 prefix 請包含裝置專屬字串（見 `config.h.example`）。

BLE UUID（`ENABLE_BLE 1` 時）：

| 用途 | UUID |
|---|---|
| Service | `7f2a0001-c701-4c9e-8f7e-2b3d5a1e9c01` |
| Write characteristic（host→device） | `7f2a0002-c701-4c9e-8f7e-2b3d5a1e9c01` |
| Notify characteristic（device→host） | `7f2a0003-c701-4c9e-8f7e-2b3d5a1e9c01` |

---

## 韌體硬限制表

以下限制**寫死在韌體**，主機端（包括 runtime 的 policy）只能再收緊、不可能放寬。
超出範圍的請求會被 clamp，`ack.applied` 回報實際套用值。

| 動器 | 限制 |
|---|---|
| `vibe.pulse` | strength duty 上限 **0.8**；durationMs ≤ **3000**；兩次脈衝最小間隔 **500ms**（違反 → `err rate-limited`，脈衝進行中亦拒收） |
| `buzzer.beep` | freqHz clamp **200..4000**；durationMs ≤ **2000**；PWM duty 硬上限 **50%**（預設約 31%） |
| `servo.move` | angle clamp **10..170**；每 **300ms** 最多一次（違反 → `err rate-limited`） |
| `led.set` | r/g/b 各自 clamp **0..255** |

---

## 測試步驟

### 1. Serial Monitor 手動測試（逐條）

Serial Monitor 設 115200、行尾 Newline，逐行貼上並核對回覆：

| # | 送出 | 預期回覆 |
|---|---|---|
| 1 | `{"type":"who"}` | `hello`（`pairing:true`） |
| 2 | `{"type":"read"}` | `{"type":"err","reason":"not-paired"}`（配對前拒絕） |
| 3 | `{"type":"pair","code":"錯的碼"}` | `{"type":"pair-fail"}` |
| 4 | `{"type":"pair","code":"1234-5678"}` | `{"type":"pair-ok"}`（用你 config.h 裡的碼） |
| 5 | `{"type":"read"}` | `state`；量不到的感測器誠實顯示 `-1`/`null` |
| 6 | `{"type":"cmd","id":"t1","name":"led.set","params":{"r":255,"g":80,"b":0}}` | `{"type":"ack","id":"t1","applied":{"r":255,"g":80,"b":0}}`，LED 亮橘 |
| 7 | 重送同一行（同 `id":"t1"`） | `{"type":"ack","id":"t1","dup":true}`，效果**不**重套 |
| 8 | `{"type":"cmd","id":"t2","name":"vibe.pulse","params":{"strength":1.0,"durationMs":9999}}` | `applied":{"strength":0.8,"durationMs":3000}` ← 硬限制 clamp |
| 9 | 立刻再送（`id":"t3"`） | `{"type":"err","id":"t3","reason":"rate-limited"}` |
| 10 | `{"type":"cmd","id":"t4","name":"buzzer.beep","params":{"freqHz":880,"durationMs":1500}}` | ack 後立刻送 `{"type":"cancel","id":"t4"}` → `{"type":"ack","id":"t4","cancelled":true}`，聲音停 |
| 11 | `{"type":"cancel","id":"nope"}` | `{"type":"err","id":"nope","reason":"not-found"}` |
| 12 | `{"type":"cmd","id":"t5","name":"servo.move","params":{"angle":999}}` | `applied":{"angle":170}` ← clamp |
| 13 | `{"type":"stop-all"}` | `{"type":"ack","stopAll":true}`，LED 熄、伺服鬆脫 |
| 14 | 按實體按鈕 | 立即多推一則 `state`（`button:true`），放開再一則 |
| 15 | 等待 | 每 5 秒自動推播一則 `state` |

### 2. 配對流程

- `PAIRING_CODE` 非空時，每條通道（Serial／MQTT／BLE）**各自**需要 `pair`；
  配對前 `cmd`/`read`/`cancel` 一律回 `not-paired`。
- `who`、`pair`、`stop-all` 配對前也接受（`stop-all` 是刻意的 fail-safe 設計）。
- MQTT／BLE 斷線後配對狀態重置；Serial 通道維持到裝置重開機。
- `PAIRING_CODE` 設空字串 = 停用配對，`hello` 誠實回 `pairing:false`。

### 3. MQTT 測試

```bash
# 終端 A：訂閱裝置上行
mosquitto_sub -h 192.168.1.10 -t 'interact-ai/companion/esp32-companion-01/from-device' -v

# 終端 B：下行送指令（與 Serial 完全同一協定）
P=interact-ai/companion/esp32-companion-01/to-device
mosquitto_pub -h 192.168.1.10 -t "$P" -m '{"type":"who"}'
mosquitto_pub -h 192.168.1.10 -t "$P" -m '{"type":"pair","code":"1234-5678"}'
mosquitto_pub -h 192.168.1.10 -t "$P" -m '{"type":"cmd","id":"m1","name":"led.set","params":{"r":0,"g":128,"b":255}}'
mosquitto_pub -h 192.168.1.10 -t "$P" -m '{"type":"read"}'
mosquitto_pub -h 192.168.1.10 -t "$P" -m '{"type":"stop-all"}'
```

裝置連上 broker 時會主動在 `out` 主題送一則 `hello`。

### 4. 與 runtime 整合（宣告式 YAML）

YAML 放到 `~/.adaptive-interaction/config/adapters/`（daemon 的 adapter 目錄）。
注意：對 `serial`／`mqtt`／`ble` 這幾種 link transport，**adapter 會自己組出**
`{"type":"cmd","id":...,"nonce":...}` 外層並處理 who/hello 身分驗證與 pair
配對流程；actuator 的 YAML 只用 `command:` 描述 `{"name":...,"params":{...}}`
的部分。receptor 的 `facts` JSON pointer 是套在裝置 `state` 訊息上
（根為 `{"deviceId":...,"facts":{...}}`，所以是 `/facts/...`）。
連線設定（`serial:`／`mqtt:`）寫在**每個 capability** 上；同一 adapter 內
所有同 transport 的 capability 連線設定必須一致。

**Serial transport：**

```yaml
schemaVersion: "1.0"
id: esp32-companion
displayName: ESP32 隨身互動裝置
provider:
  id: provider.device.esp32-companion-01
  kind: device
  displayName: ESP32 Companion
capabilities:
  - kind: receptor
    id: env
    category: environment
    transport: serial
    serial: &companion-serial
      port: /dev/cu.usbserial-0001         # Linux 常見 /dev/ttyUSB0
      baud: 115200
      expectedDeviceId: esp32-companion-01 # 必須等於 config.h 的 DEVICE_ID
      pairingCode: "1234-5678"             # 必須等於 config.h 的 PAIRING_CODE（建議 secret://）
    pollIntervalMs: 5000
    facts:
      distanceMm: "/facts/distanceMm"
      lux: "/facts/lux"
  - kind: actuator
    id: led
    channel: light
    transport: serial
    serial: *companion-serial
    confirmation: acknowledged
    command:
      name: led.set
      params: { r: "{{magnitude}}", g: 0, b: 0 }
  - kind: actuator
    id: vibe
    channel: haptic
    transport: serial
    serial: *companion-serial
    confirmation: acknowledged
    command:
      name: vibe.pulse
      params: { strength: "{{magnitude}}", durationMs: "{{durationMs}}" }
  - kind: actuator
    id: chime
    channel: audio
    transport: serial
    serial: *companion-serial
    confirmation: acknowledged
    command:
      name: buzzer.beep
      params: { freqHz: 880, durationMs: "{{durationMs}}" }
  - kind: actuator
    id: pointer
    channel: motion
    transport: serial
    serial: *companion-serial
    confirmation: acknowledged
    command:
      name: servo.move
      params: { angle: 90 }
```

**MQTT transport（capabilities 同上，只換 transport 與連線區塊）：**

```yaml
schemaVersion: "1.0"
id: esp32-companion-mqtt
displayName: ESP32 隨身互動裝置（MQTT）
capabilities:
  - kind: receptor
    id: env
    category: environment
    transport: mqtt
    mqtt: &companion-mqtt
      brokerHost: 192.168.1.10
      brokerPort: 1883
      topicPrefix: interact-ai/companion/esp32-companion-01  # 等於 config.h 的 MQTT_TOPIC_PREFIX
      expectedDeviceId: esp32-companion-01
      pairingCode: "1234-5678"
    pollIntervalMs: 5000
    facts:
      distanceMm: "/facts/distanceMm"
      lux: "/facts/lux"
  - kind: actuator
    id: led
    channel: light
    transport: mqtt
    mqtt: *companion-mqtt
    confirmation: acknowledged
    command:
      name: led.set
      params: { r: "{{magnitude}}", g: 0, b: 0 }
```

**BLE transport（`ENABLE_BLE 1` 時；runtime 端僅 macOS／Windows 支援）：**

```yaml
  - kind: actuator
    id: led
    channel: light
    transport: ble
    ble:
      deviceName: esp32-companion-01           # 廣播名稱（掃描用，不是身分）
      serviceUuid: 7f2a0001-c701-4c9e-8f7e-2b3d5a1e9c01
      commandCharUuid: 7f2a0002-c701-4c9e-8f7e-2b3d5a1e9c01
      stateCharUuid: 7f2a0003-c701-4c9e-8f7e-2b3d5a1e9c01
      expectedDeviceId: esp32-companion-01
      pairingCode: "1234-5678"
    confirmation: acknowledged
    command:
      name: led.set
      params: { r: "{{magnitude}}", g: 0, b: 0 }
```

模板佔位符 `{{magnitude}}`／`{{durationMs}}` 由 runtime 以 **policy 裁剪後的
bounded 值**代入（不是 AI 的原始請求值）；韌體再套一層硬限制——兩層都只會
更保守，不會更寬。

---

## 已知限制（誠實列出）

1. **本韌體尚未在真實 ESP32 硬體上編譯與驗證**——僅經程式碼審閱，並與
   runtime 端 adapter 實作（`crates/interaction-adapter-declarative/src/protocol.rs`
   的 `DeviceMsg`/`HostMsg`）逐欄核對訊息型別、欄位名、MQTT 主題與錯誤碼。
   語法以 Arduino ESP32 core 3.x＋ArduinoJson 7.x 撰寫，首次燒錄前請預期
   可能需要小幅修正。
2. **BLE 預設關閉**（`ENABLE_BLE 0`），需自行在 `config.h` 開啟並安裝
   NimBLE-Arduino。BLE 假設一次 write 含完整 JSON（runtime 端單筆上限
   480 bytes），未實作分段重組；訊息較長時請改用 Serial／MQTT。
   runtime 端 BLE transport 僅支援 macOS／Windows。
3. **DHT22 偶發讀取失敗（NaN）**：`tempC` 誠實回 `null`，不以舊值冒充新讀值。
   未接 DHT22 時 `tempC` 恆為 `null`；未接 HC-SR04 時 `distanceMm` 恆為 `-1`。
4. **MQTT 無 TLS**：訊息（含配對碼）在網路上明文傳輸，**僅限信任的區域網路**
   使用；不要跨網際網路連 broker。
5. **`lux` 是未校準的 ADC 相對值（0..4095）**，不是真實 lux 單位；欄位名沿用
   協定，語意是「相對亮度」。
6. **配對狀態**：Serial 通道無法偵測 USB 拔插，配對維持到裝置重開機；
   MQTT／BLE 斷線即重置。`nonce` 欄位目前僅接收不驗證，重放防護依賴
   16 筆 cmd id 環形去重。
7. **`servoAngle` 是最後指令角度**：`stop-all` 會 detach 伺服（不再出力），
   之後實際角度可能被外力改變，但回報值不變。
8. **短暫阻塞點**：HC-SR04 的 `pulseIn` 最長阻塞 30ms（協定允許的微秒級量測）；
   MQTT broker 斷線重連時 `connect()` 可能阻塞數秒，期間 Serial 回應會延遲。
9. **單行訊息上限 639 bytes**，超過整行丟棄並回 `err bad-json`。
10. **`stop-all` 不需配對**：刻意的 fail-safe 設計——它只會關閉效果，
    寧可讓未配對的主機能緊急停下裝置。
