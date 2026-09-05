// ===========================================================================
// esp32-companion — 官方 ESP32 參考裝置韌體 (adaptive-interaction platform)
//
// Target : ESP32-WROOM-32 DevKitC, Arduino ESP32 core 3.x
// Proto  : line-delimited JSON, proto version 1
//          - USB Serial 115200（一行一個 JSON object）
//          - MQTT（一則訊息一個 JSON object；topic 規則見 config.h.example）
//          - BLE（可選，ENABLE_BLE=1；一次 write 一個 JSON object）
//
// 誠實不變量（與平台 CLAUDE.md 對齊）：
//   * ack 代表「已在裝置上生效」——效果真的被套用（或啟動）之後才回 ack，
//     絕不先回 ack 再執行。
//   * ack.applied 回報的是「實際套用值」（經韌體硬限制 clamp 之後），
//     不是主機要求值。
//   * 讀不到的感測器誠實回報 -1 / null，不得捏造數據。
//   * 韌體硬限制在裝置端強制執行，主機端無法解除。
//
// 韌體硬限制（HARD LIMITS — 常數在下方，主機不可調）：
//   vibe   : strength duty 上限 0.8、durationMs ≤ 3000、脈衝間隔 ≥ 500ms
//   buzzer : 200..4000 Hz、durationMs ≤ 2000、PWM duty ≤ 50%
//   servo  : 角度 10..170、每 300ms 最多一次移動
//   led    : 各通道 0..255
//
// 數值參數規則（三端一致：本韌體／scripts/esp32-serial-sim.py／README 協定表）：
//   * 所有數值參數一律以浮點讀取後四捨五入＋clamp——主機的
//     `{{magnitude}}` 是 JSON number（float），用整數解析會落回預設值。
//   * led.set 的 r/g/b：JSON 整數 0–255 = 絕對值；JSON 浮點 0.0–1.0 = 比例
//     （0.8 → 204）；JSON 浮點 > 1.0 當絕對值；缺漏/null = 0。
//   * 非數值（字串／bool／物件／陣列）→ err bad-params（不靜默當 0）。
// ===========================================================================

#include <WiFi.h>
#include <PubSubClient.h>
#include <ArduinoJson.h>   // v7.x
#include <DHT.h>           // Adafruit "DHT sensor library"
#include <ESP32Servo.h>

#if defined(__has_include)
#  if __has_include("config.h")
#    include "config.h"
#  else
#    error "Missing config.h — copy config.h.example to config.h and edit it."
#  endif
#else
#  include "config.h"
#endif

#ifndef ENABLE_BLE
#define ENABLE_BLE 0   // 預設關閉；設 1 需安裝 NimBLE-Arduino（見 README）
#endif

#ifndef STATE_PERIOD_MS
#define STATE_PERIOD_MS 5000UL   // 自動推播 state 的週期
#endif

#if ENABLE_BLE
#include <NimBLEDevice.h>  // NimBLE-Arduino 2.x
#endif

// ---------------------------------------------------------------------------
// 韌體資訊
// ---------------------------------------------------------------------------
static const char* FW_VERSION   = "1.0.0";
static const int   PROTO_VERSION = 1;

// ---------------------------------------------------------------------------
// 腳位（與 README 接線圖一致）
// ---------------------------------------------------------------------------
static const int PIN_LED_R   = 25;  // RGB LED（共陰極）R，PWM
static const int PIN_LED_G   = 26;  // RGB LED G，PWM
static const int PIN_LED_B   = 27;  // RGB LED B，PWM
static const int PIN_BUTTON  = 32;  // 按鈕，INPUT_PULLUP，另一端接 GND
static const int PIN_TRIG    = 18;  // HC-SR04 TRIG
static const int PIN_ECHO    = 19;  // HC-SR04 ECHO（務必經分壓 5V→3.3V！）
static const int PIN_LDR     = 34;  // 光敏電阻分壓中點（ADC，input-only 腳）
static const int PIN_DHT     = 4;   // DHT22 DATA（10k 上拉）
static const int PIN_VIBE    = 23;  // 震動馬達，經 NPN 電晶體
static const int PIN_SERVO   = 13;  // SG90 訊號（馬達電源用外部 5V）
static const int PIN_BUZZER  = 22;  // 無源蜂鳴器

// ---------------------------------------------------------------------------
// 韌體硬限制（主機不可調；clamp 後以 ack.applied 誠實回報實際值）
// ---------------------------------------------------------------------------
static const float    VIBE_MAX_STRENGTH   = 0.8f;    // duty 上限 0.8
static const uint32_t VIBE_MAX_DURATION_MS = 3000;
static const uint32_t VIBE_MIN_GAP_MS      = 500;    // 兩次脈衝最小間隔

static const uint32_t BUZZ_MIN_FREQ_HZ     = 200;
static const uint32_t BUZZ_MAX_FREQ_HZ     = 4000;
static const uint32_t BUZZ_MAX_DURATION_MS = 2000;
// 10-bit PWM：512/1023 ≈ 50% duty 為硬上限；預設用 320（約 31%）。
static const uint16_t BUZZ_HARD_MAX_DUTY_10BIT = 512;
static const uint16_t BUZZ_DUTY_10BIT          = 320;

static const int      SERVO_MIN_ANGLE      = 10;
static const int      SERVO_MAX_ANGLE      = 170;
static const uint32_t SERVO_MIN_GAP_MS     = 300;    // 每 300ms 最多一次

// 感測節奏
static const uint32_t DHT_REFRESH_MS       = 3000;   // DHT22 最短讀取間隔 >2s
static const uint32_t BUTTON_DEBOUNCE_MS   = 30;
static const uint32_t HSR04_TIMEOUT_US     = 30000;  // ~5m；逾時回 -1

// BLE UUID（固定常數；一個 write characteristic、一個 notify characteristic）
#if ENABLE_BLE
static const char* BLE_SERVICE_UUID     = "7f2a0001-c701-4c9e-8f7e-2b3d5a1e9c01";
static const char* BLE_WRITE_CHAR_UUID  = "7f2a0002-c701-4c9e-8f7e-2b3d5a1e9c01";
static const char* BLE_NOTIFY_CHAR_UUID = "7f2a0003-c701-4c9e-8f7e-2b3d5a1e9c01";
#endif

// ---------------------------------------------------------------------------
// 以下型別／常數必須定義在這裡（檔案上半部）：Arduino 前處理器會把自動產生
// 的函式原型插在「第一個全域變數之前」，簽章裡用到的東西必須先出現，
// 否則會編出 'X was not declared in this scope'（原型行，不是定義行）。
// ---------------------------------------------------------------------------

// 連線通道（每個通道各自維護配對狀態）
enum Link : uint8_t { LINK_SERIAL = 0, LINK_MQTT = 1, LINK_BLE = 2, LINK_COUNT = 3 };

// 去重/重放環形緩衝的大小（cmd id 與 nonce 各一組）
static const size_t CMD_RING_SIZE = 16;
static const size_t CMD_ID_MAX    = 48;

// 數值參數解析狀態
enum ParamStatus : uint8_t {
  PARAM_MISSING = 0,   // 欄位缺漏或 null → 呼叫端用預設值
  PARAM_OK      = 1,
  PARAM_BAD     = 2,   // 非數值（字串／bool／物件／陣列）→ err bad-params
};

static bool g_linkPaired[LINK_COUNT] = { false, false, false };

// 配對暴力猜測防護（每條通道各自計數；規則與 scripts/esp32-serial-sim.py 一致）：
//   * 連續 PAIR_MAX_FAILURES 次錯碼 → 該通道鎖定 PAIR_LOCKOUT_MS。
//   * 鎖定期間的 pair 一律回 {"type":"pair-fail","reason":"pair-locked",
//     "retryAfterMs":N}——不比對碼、也不延長鎖定；hello 的 pairingLocked
//     誠實回報 true。
//   * 鎖定「不」隨 MQTT／BLE 斷線重置（否則重連一次就能繞過），只在
//     重開機後歸零；配對成功會把失敗計數歸零。
static const uint8_t  PAIR_MAX_FAILURES = 5;
static const uint32_t PAIR_LOCKOUT_MS   = 30000;
static uint8_t  g_pairFailures[LINK_COUNT]      = { 0, 0, 0 };
static bool     g_pairLocked[LINK_COUNT]        = { false, false, false };
static uint32_t g_pairLockedUntilMs[LINK_COUNT] = { 0, 0, 0 };

// 配對是否啟用（PAIRING_CODE 為空字串 = 停用；hello 仍誠實回報）
static bool pairingEnabled() { return PAIRING_CODE[0] != '\0'; }
static bool linkAuthorized(Link link) {
  return !pairingEnabled() || g_linkPaired[link];
}

// 這條通道目前是否在配對鎖定期內（期滿即解鎖並重新計數；millis 溢位安全）。
static bool pairLocked(Link link, uint32_t now) {
  if (!g_pairLocked[link]) return false;
  if ((int32_t)(now - g_pairLockedUntilMs[link]) >= 0) {
    g_pairLocked[link] = false;
    g_pairFailures[link] = 0;
    return false;
  }
  return true;
}

static uint32_t pairRetryAfterMs(Link link, uint32_t now) {
  return g_pairLocked[link] ? (uint32_t)(g_pairLockedUntilMs[link] - now) : 0;
}

// ---------------------------------------------------------------------------
// 全域狀態
// ---------------------------------------------------------------------------
WiFiClient   g_wifiClient;
PubSubClient g_mqtt(g_wifiClient);
DHT          g_dht(PIN_DHT, DHT22);
Servo        g_servo;

// MQTT 主題與 runtime 端 adapter（crates/interaction-adapter-declarative）對齊：
//   host → device : <MQTT_TOPIC_PREFIX>/to-device
//   device → host : <MQTT_TOPIC_PREFIX>/from-device
// topic 不是身分——runtime 仍會驗 hello.deviceId＋配對碼。
static char g_mqttTopicIn[160];   // <prefix>/to-device
static char g_mqttTopicOut[160];  // <prefix>/from-device

// MQTT 重連節奏（非阻塞退避；細節見 maintainMqtt 的註解）
static const uint32_t MQTT_RETRY_BASE_MS      = 10000;  // 首次失敗後最短重試間隔
static const uint32_t MQTT_RETRY_MAX_MS       = 60000;  // 指數退避上限
static const uint32_t MQTT_CONNECT_TIMEOUT_MS = 500;    // TCP connect 上限（預設 3000）
static const uint16_t MQTT_SOCKET_TIMEOUT_S   = 1;      // 等 CONNACK/封包上限（預設 15s）
static uint32_t g_lastMqttAttemptMs = 0;
static uint32_t g_mqttRetryDelayMs  = MQTT_RETRY_BASE_MS;
static bool     g_mqttFailedBefore  = false;   // 上一次嘗試就失敗了嗎

// Serial 逐行讀取緩衝
static char   g_serialBuf[640];
static size_t g_serialLen = 0;
static bool   g_serialOverflow = false;

// LED 目前實際值
static uint8_t g_ledR = 0, g_ledG = 0, g_ledB = 0;

// 計時效果（可被 cancel 的：vibe / buzzer）
static bool     g_vibeActive = false;
static char     g_vibeCmdId[48] = "";
static uint32_t g_vibeEndMs = 0;
static uint32_t g_vibeLastEndMs = 0;
static bool     g_vibeEverRan = false;

static bool     g_buzzActive = false;
static char     g_buzzCmdId[48] = "";
static uint32_t g_buzzEndMs = 0;

// Servo
static bool     g_servoAttached = false;
static int      g_servoAngle = 90;       // 最後套用角度（見 README 已知限制）
static uint32_t g_servoLastMoveMs = 0;
static bool     g_servoEverMoved = false;

// 按鈕去彈跳
static bool     g_buttonPressed = false;   // 去彈跳後的穩定值（true=按下）
static bool     g_buttonRawLast = false;
static uint32_t g_buttonLastEdgeMs = 0;

// DHT 快取（讀不到就誠實標記 invalid → JSON null）
static float    g_tempC = 0.0f;
static bool     g_tempValid = false;
static uint32_t g_dhtLastReadMs = 0;

// state 週期推播
static uint32_t g_lastStatePushMs = 0;

// 重放/重複防護：cmd id 與 nonce 各一組 16 筆環形緩衝，**並行**比對。
// nonce 由主機每次隨機產生（protocol.rs 的 new_nonce()，64-bit）；同一個
// nonce 再次出現＝重放（或主機有 bug），一律回 dup:true 不套用效果。
// （CMD_RING_SIZE／CMD_ID_MAX 定義在檔案上半部，原因見該處註解。）
static char   g_seenIds[CMD_RING_SIZE][CMD_ID_MAX];
static size_t g_seenNext = 0;
static char   g_seenNonces[CMD_RING_SIZE][CMD_ID_MAX];
static size_t g_nonceNext = 0;

#if ENABLE_BLE
static NimBLECharacteristic* g_bleNotifyChar = nullptr;
static volatile bool g_bleConnected = false;

// BLE write 回呼跑在 NimBLE host task（core 0），loop() 跑在 core 1。
// 兩邊直接共用全域狀態會race（例如 vibe 剛被 loop() 停掉、回呼卻已宣稱
// applied）。因此回呼「只」把整行訊息塞進這個有界佇列，實際處理與所有
// 回覆（ack/err/state）都由 loop() 在同一條路徑上做——與 Serial/MQTT 完全
// 一樣。佇列滿＝丟棄並由 loop() 回 err busy（誠實：沒收下就不假裝收下）。
static const size_t BLE_QUEUE_SLOTS = 8;
static const size_t BLE_MSG_MAX     = 512;   // 單筆 write ≥512 bytes 即拒（上限 511；host 端 480）
struct BleMsg { char line[BLE_MSG_MAX]; };
// device→host 方向也要有長度紀律。ATT 的 Handle-Value-Notification 可攜
// payload 只有「協商後 MTU − 3」；超過的部分由協定棧**直接截掉**，host 端
// 收到的是破 JSON（舊版無聲丟棄 → read 只會逾時，沒有任何線索指向真因）。
// 所以：(1) 主動把偏好 MTU 提高，(2) 送出時依實際協商值分段，
// (3) 以換行界定一則訊息，host 端（ble.rs 的 NotifyAssembler）據此重組。
static const uint16_t BLE_PREFERRED_MTU = 517;   // ATT 上限（BLE 4.2+ data length）
static const uint16_t BLE_MIN_ATT_MTU   = 23;    // 協商不到就用最保守值
static NimBLEServer* g_bleServer = nullptr;
static volatile uint16_t g_bleConnHandle = 0xFFFF;   // BLE_HS_CONN_HANDLE_NONE
static QueueHandle_t g_bleQueue = nullptr;
static volatile bool g_bleDropBusy    = false;  // 佇列滿：loop() 回 err busy
static volatile bool g_bleDropTooLong = false;  // 單筆過長：loop() 回 err bad-json
#endif

// ---------------------------------------------------------------------------
// 函式原型（避免 Arduino 前處理器對自動原型的順序問題）
// ---------------------------------------------------------------------------
static void handleMessage(const char* line, Link link);
static void sendLine(Link link, const char* line);
#if ENABLE_BLE
static size_t bleNotifyPayloadMax();
#endif
static void sendDoc(Link link, JsonDocument& doc);
static void pushStateToPairedLinks();
static void sendHello(Link link);
static void sendErr(Link link, const char* id, const char* reason);
static void stopAllEffects();

// ---------------------------------------------------------------------------
// 小工具
// ---------------------------------------------------------------------------

// 近似常數時間比較：不提早 return，長度差異也混進 diff。
static bool constantTimeEquals(const char* a, const char* b) {
  size_t la = strlen(a);
  size_t lb = strlen(b);
  uint8_t diff = (uint8_t)(la ^ lb);
  size_t n = (la > lb) ? la : lb;
  for (size_t i = 0; i < n; i++) {
    uint8_t ca = (i < la) ? (uint8_t)a[i] : 0;
    uint8_t cb = (i < lb) ? (uint8_t)b[i] : 0;
    diff |= (uint8_t)(ca ^ cb);
  }
  return diff == 0;
}

static long clampLong(long v, long lo, long hi) {
  if (v < lo) return lo;
  if (v > hi) return hi;
  return v;
}

static float clampFloat(float v, float lo, float hi) {
  if (v < lo) return lo;
  if (v > hi) return hi;
  return v;
}

// 去重環形緩衝（cmd id 與 nonce 共用同一組函式）
static bool ringContains(const char ring[CMD_RING_SIZE][CMD_ID_MAX], const char* v) {
  if (v == nullptr || v[0] == '\0') return false;
  for (size_t i = 0; i < CMD_RING_SIZE; i++) {
    if (ring[i][0] != '\0' && strncmp(ring[i], v, CMD_ID_MAX - 1) == 0) {
      return true;
    }
  }
  return false;
}

static void ringRemember(char ring[CMD_RING_SIZE][CMD_ID_MAX], size_t* next, const char* v) {
  if (v == nullptr || v[0] == '\0') return;
  strncpy(ring[*next], v, CMD_ID_MAX - 1);
  ring[*next][CMD_ID_MAX - 1] = '\0';
  *next = (*next + 1) % CMD_RING_SIZE;
}

// ---------------------------------------------------------------------------
// 數值參數解析（規則見檔頭；scripts/esp32-serial-sim.py 鏡射同一套規則）
//
// 為什麼不用 ArduinoJson 的 `params["r"] | 0L`：主機把 `{{magnitude}}` 以
// JSON number（float，例如 0.8）上線，而 `| 0L` 對 float 型別的值會落回
// 預設值 0——README 的 led.set 範例在真板上會「永遠不亮」。這裡一律以
// float 讀取，另外保留「這個 JSON 數字是不是整數字面值」的資訊
// （ArduinoJson 7：is<long>() 對 255 為 true、對 1.0 為 false）。
// 一律用 as<float>()：ArduinoJson 對短字面值本來就存 float，模擬器端
// 也鏡射同一個精度，兩端結果才會逐位一致。
// ---------------------------------------------------------------------------
static ParamStatus readNumber(JsonObjectConst params, const char* key,
                              float* value, bool* integerLiteral) {
  JsonVariantConst v = params[key];
  if (v.isNull()) return PARAM_MISSING;   // 缺漏與明寫 null 同義
  if (!v.is<float>()) return PARAM_BAD;   // is<float>()：整數與浮點皆 true
  *value = v.as<float>();
  if (integerLiteral != nullptr) *integerLiteral = v.is<long>();
  return PARAM_OK;
}

// 四捨五入（正負皆用「加 0.5 後截斷」，與模擬器的 int(x + 0.5) 一致）
static long roundToLong(float v) { return (long)(v + 0.5f); }

// 整數型參數：主機送 1500 或 1500.0 都接受；非數值 → err bad-params。
static bool readIntParam(Link link, const char* id, JsonObjectConst params,
                         const char* key, long fallback, long lo, long hi, long* out) {
  float raw = 0.0f;
  ParamStatus st = readNumber(params, key, &raw, nullptr);
  if (st == PARAM_BAD) { sendErr(link, id, "bad-params"); return false; }
  *out = clampLong((st == PARAM_MISSING) ? fallback : roundToLong(raw), lo, hi);
  return true;
}

// 浮點型參數（例如 vibe.pulse 的 strength 0..1）。
static bool readFloatParam(Link link, const char* id, JsonObjectConst params,
                           const char* key, float fallback, float lo, float hi, float* out) {
  float raw = 0.0f;
  ParamStatus st = readNumber(params, key, &raw, nullptr);
  if (st == PARAM_BAD) { sendErr(link, id, "bad-params"); return false; }
  *out = clampFloat((st == PARAM_MISSING) ? fallback : raw, lo, hi);
  return true;
}

// led.set 的 r/g/b：整數 0–255 = 絕對值；浮點 0.0–1.0 = 比例（0.8 → 204）；
// 浮點 > 1.0 當絕對值四捨五入；缺漏/null = 0；非數值 → err bad-params。
static bool readLedChannel(Link link, const char* id, JsonObjectConst params,
                           const char* key, long* out) {
  float raw = 0.0f;
  bool isInt = false;
  ParamStatus st = readNumber(params, key, &raw, &isInt);
  if (st == PARAM_BAD) { sendErr(link, id, "bad-params"); return false; }
  if (st == PARAM_MISSING) { *out = 0; return true; }
  float scaled = (!isInt && raw >= 0.0f && raw <= 1.0f) ? (raw * 255.0f) : raw;
  *out = clampLong(roundToLong(scaled), 0, 255);
  return true;
}

// ---------------------------------------------------------------------------
// 感測
// ---------------------------------------------------------------------------

// HC-SR04：回傳距離 mm；量不到（逾時/未接）誠實回 -1。
// 注意：pulseIn 最多阻塞 HSR04_TIMEOUT_US（30ms）——這是本韌體 loop 中
// 唯一允許的微秒級阻塞（協定所需的 TRIG 脈衝與回波量測）。
static long readDistanceMm() {
  digitalWrite(PIN_TRIG, LOW);
  delayMicroseconds(3);
  digitalWrite(PIN_TRIG, HIGH);
  delayMicroseconds(10);
  digitalWrite(PIN_TRIG, LOW);
  unsigned long echoUs = pulseIn(PIN_ECHO, HIGH, HSR04_TIMEOUT_US);
  if (echoUs == 0) return -1;                 // 逾時或未接 → 誠實回 -1
  // 音速 343 m/s，來回除以 2：mm = us * 0.343 / 2
  return (long)((echoUs * 343UL) / 2000UL);
}

// LDR：回傳 ADC 原始值 0..4095。這「不是」校準過的 lux——欄位名沿用協定
// 的 lux，但語意是「未校準相對亮度」，README 已誠實註明。
static int readLuxRaw() {
  return analogRead(PIN_LDR);
}

// DHT22：最多每 DHT_REFRESH_MS 讀一次；NaN → g_tempValid=false → JSON null。
static void refreshDhtIfDue(uint32_t now) {
  if (g_dhtLastReadMs != 0 && (now - g_dhtLastReadMs) < DHT_REFRESH_MS) return;
  g_dhtLastReadMs = now;
  float t = g_dht.readTemperature();
  if (isnan(t)) {
    g_tempValid = false;   // 誠實：讀失敗就是 null，不沿用舊值冒充新讀值
  } else {
    g_tempC = t;
    g_tempValid = true;
  }
}

// 按鈕去彈跳；狀態翻轉時回傳 true（呼叫端負責推播 state）。
static bool pollButtonEdge(uint32_t now) {
  bool raw = (digitalRead(PIN_BUTTON) == LOW);   // INPUT_PULLUP：LOW=按下
  if (raw != g_buttonRawLast) {
    g_buttonRawLast = raw;
    g_buttonLastEdgeMs = now;
  }
  if ((now - g_buttonLastEdgeMs) >= BUTTON_DEBOUNCE_MS && raw != g_buttonPressed) {
    g_buttonPressed = raw;
    return true;
  }
  return false;
}

// ---------------------------------------------------------------------------
// 動器（皆先套用、再回報；clamp 值即 applied 值）
// ---------------------------------------------------------------------------

static void applyLed(uint8_t r, uint8_t g, uint8_t b) {
  ledcWrite(PIN_LED_R, r);   // 共陰極：duty 即亮度
  ledcWrite(PIN_LED_G, g);
  ledcWrite(PIN_LED_B, b);
  g_ledR = r; g_ledG = g; g_ledB = b;
}

static void stopVibe(uint32_t now) {
  if (!g_vibeActive) return;
  ledcWrite(PIN_VIBE, 0);
  g_vibeActive = false;
  g_vibeCmdId[0] = '\0';
  g_vibeLastEndMs = now;
  g_vibeEverRan = true;
}

static void stopBuzzer() {
  if (!g_buzzActive) return;
  ledcWrite(PIN_BUZZER, 0);
  g_buzzActive = false;
  g_buzzCmdId[0] = '\0';
}

// SG90 無法「停在半空」，stop 的誠實語意是 detach（切斷 PWM，不再出力）。
static void releaseServo() {
  if (g_servoAttached) {
    g_servo.detach();
    g_servoAttached = false;
  }
}

// 計時效果到期（millis 溢位安全的減法比較）。loop() 在任何可能阻塞的呼叫
// 「之前」呼叫一次、之後再呼叫一次——vibe/buzzer 的 durationMs 硬上限不能
// 因為網路重連而被撐破。
static void expireTimedEffects(uint32_t now) {
  if (g_vibeActive && (int32_t)(now - g_vibeEndMs) >= 0) stopVibe(now);
  if (g_buzzActive && (int32_t)(now - g_buzzEndMs) >= 0) stopBuzzer();
}

static void stopAllEffects() {
  uint32_t now = millis();
  stopVibe(now);
  stopBuzzer();
  releaseServo();
  applyLed(0, 0, 0);
}

// ---------------------------------------------------------------------------
// JSON 輸出
// ---------------------------------------------------------------------------

#if ENABLE_BLE
// 這一次 notify 能塞多少 bytes：協商後的 ATT MTU − 3（opcode 1 + handle 2）。
// 協商不到（還沒連上、後端沒回報）就退回 ATT 預設 23 → 20 bytes：寧可多分
// 幾段，也不要送出去被截斷。
static size_t bleNotifyPayloadMax() {
  uint16_t mtu = BLE_MIN_ATT_MTU;
  if (g_bleServer != nullptr && g_bleConnHandle != 0xFFFF) {
    uint16_t peer = g_bleServer->getPeerMTU(g_bleConnHandle);
    if (peer >= BLE_MIN_ATT_MTU) mtu = peer;
  }
  return (size_t)(mtu - 3);
}
#endif

static void sendLine(Link link, const char* line) {
  switch (link) {
    case LINK_SERIAL:
      Serial.println(line);
      break;
    case LINK_MQTT:
      if (g_mqtt.connected()) {
        g_mqtt.publish(g_mqttTopicOut, line);
      }
      break;
    case LINK_BLE:
#if ENABLE_BLE
      if (g_bleConnected && g_bleNotifyChar != nullptr) {
        // 一次 notify 最多送得動 MTU-3 bytes；長訊息（state 在預設 deviceId
        // 下就有 193 bytes）必須自己分段，否則會被協定棧靜默截斷。
        const size_t maxPayload = bleNotifyPayloadMax();
        const size_t len = strlen(line);
        size_t offset = 0;
        while (offset < len) {
          size_t take = len - offset;
          if (take > maxPayload) take = maxPayload;
          g_bleNotifyChar->setValue((const uint8_t*)(line + offset), take);
          g_bleNotifyChar->notify();
          offset += take;
        }
        // 訊息結束符：host 端據此知道「這一則到齊了」。
        static const uint8_t kNewline = (uint8_t)'\n';
        g_bleNotifyChar->setValue(&kNewline, 1);
        g_bleNotifyChar->notify();
      }
#endif
      break;
    default:
      break;
  }
}

static void sendDoc(Link link, JsonDocument& doc) {
  char out[768];
  size_t n = serializeJson(doc, out, sizeof(out));
  if (n == 0 || n >= sizeof(out)) return;   // 序列化失敗就不送，不送殘缺 JSON
  sendLine(link, out);
}

static void sendErr(Link link, const char* id, const char* reason) {
  JsonDocument doc;
  doc["type"] = "err";
  if (id != nullptr && id[0] != '\0') doc["id"] = id;
  doc["reason"] = reason;
  sendDoc(link, doc);
}

static void sendHello(Link link) {
  JsonDocument doc;
  doc["type"] = "hello";
  doc["deviceId"] = DEVICE_ID;
  doc["fw"] = FW_VERSION;
  doc["proto"] = PROTO_VERSION;
  JsonArray caps = doc["caps"].to<JsonArray>();
  caps.add("led.set");
  caps.add("buzzer.beep");
  caps.add("vibe.pulse");
  caps.add("servo.move");
  caps.add("sensors.read");
  // pairing=true 表示「此通道目前仍需配對」；配對停用或已配對則為 false。
  doc["pairing"] = pairingEnabled() && !g_linkPaired[link];
  // pairingLocked=true 表示「此通道因連續錯碼而在鎖定期內」（見 pairLocked）。
  doc["pairingLocked"] = pairingEnabled() && pairLocked(link, millis());
  sendDoc(link, doc);
}

// 組出 state 訊息。量不到的感測值誠實回 -1 / null。
static void buildState(JsonDocument& doc) {
  doc["type"] = "state";
  doc["deviceId"] = DEVICE_ID;
  JsonObject facts = doc["facts"].to<JsonObject>();
  facts["button"] = g_buttonPressed;
  facts["distanceMm"] = readDistanceMm();      // 未接/逾時 → -1
  facts["lux"] = readLuxRaw();                 // 未校準相對亮度（0..4095）
  if (g_tempValid) {
    facts["tempC"] = g_tempC;
  } else {
    facts["tempC"] = nullptr;                  // DHT 讀失敗 → 誠實 null
  }
  facts["vibeActive"] = g_vibeActive;
  facts["buzzActive"] = g_buzzActive;          // cancel 的獨立驗證靠它（與模擬器一致）
  facts["servoAngle"] = g_servoAngle;
  JsonObject led = facts["led"].to<JsonObject>();
  led["r"] = g_ledR;
  led["g"] = g_ledG;
  led["b"] = g_ledB;
}

static void sendStateTo(Link link) {
  JsonDocument doc;
  buildState(doc);
  sendDoc(link, doc);
}

// 自動推播只送「已配對」（或配對停用）且實際在線的通道，
// 避免對未配對的連線洩漏感測資料。
static void pushStateToPairedLinks() {
  if (linkAuthorized(LINK_SERIAL)) sendStateTo(LINK_SERIAL);
  if (g_mqtt.connected() && linkAuthorized(LINK_MQTT)) sendStateTo(LINK_MQTT);
#if ENABLE_BLE
  if (g_bleConnected && linkAuthorized(LINK_BLE)) sendStateTo(LINK_BLE);
#endif
}

// ---------------------------------------------------------------------------
// cmd 分派（回傳 true = 已套用且已 ack；false = 已送出 err）
// ---------------------------------------------------------------------------

static bool cmdLedSet(Link link, const char* id, JsonObjectConst params) {
  // 三個通道全部先解析成功才套用（部分失敗不留下半套效果）。
  long r = 0, g = 0, b = 0;
  if (!readLedChannel(link, id, params, "r", &r)) return false;
  if (!readLedChannel(link, id, params, "g", &g)) return false;
  if (!readLedChannel(link, id, params, "b", &b)) return false;
  applyLed((uint8_t)r, (uint8_t)g, (uint8_t)b);
  JsonDocument doc;
  doc["type"] = "ack";
  doc["id"] = id;
  JsonObject applied = doc["applied"].to<JsonObject>();
  applied["r"] = r; applied["g"] = g; applied["b"] = b;
  sendDoc(link, doc);
  return true;
}

static bool cmdBuzzerBeep(Link link, const char* id, JsonObjectConst params, uint32_t now) {
  long freq = 0, dur = 0;
  if (!readIntParam(link, id, params, "freqHz", 1000L,
                    (long)BUZZ_MIN_FREQ_HZ, (long)BUZZ_MAX_FREQ_HZ, &freq)) return false;
  if (!readIntParam(link, id, params, "durationMs", 200L,
                    1, (long)BUZZ_MAX_DURATION_MS, &dur)) return false;
  // 換頻率前先停掉進行中的 beep（新指令覆蓋舊指令）
  stopBuzzer();
  uint16_t duty = BUZZ_DUTY_10BIT;
  if (duty > BUZZ_HARD_MAX_DUTY_10BIT) duty = BUZZ_HARD_MAX_DUTY_10BIT; // 硬上限 50%
  ledcChangeFrequency(PIN_BUZZER, (uint32_t)freq, 10);
  ledcWrite(PIN_BUZZER, duty);
  g_buzzActive = true;
  g_buzzEndMs = now + (uint32_t)dur;
  strncpy(g_buzzCmdId, id, sizeof(g_buzzCmdId) - 1);
  g_buzzCmdId[sizeof(g_buzzCmdId) - 1] = '\0';

  JsonDocument doc;
  doc["type"] = "ack";
  doc["id"] = id;
  JsonObject applied = doc["applied"].to<JsonObject>();
  applied["freqHz"] = freq;
  applied["durationMs"] = dur;
  sendDoc(link, doc);
  return true;
}

static bool cmdVibePulse(Link link, const char* id, JsonObjectConst params, uint32_t now) {
  // 速率限制：脈衝進行中，或距上次結束 < 500ms → rate-limited
  if (g_vibeActive ||
      (g_vibeEverRan && (now - g_vibeLastEndMs) < VIBE_MIN_GAP_MS)) {
    sendErr(link, id, "rate-limited");
    return false;
  }
  float strength = 0.0f;
  long dur = 0;
  if (!readFloatParam(link, id, params, "strength", 0.0f, 0.0f, VIBE_MAX_STRENGTH,
                      &strength)) return false;
  if (!readIntParam(link, id, params, "durationMs", 200L,
                    1, (long)VIBE_MAX_DURATION_MS, &dur)) return false;
  uint8_t duty = (uint8_t)(strength * 255.0f + 0.5f);   // ≤ 204 (0.8*255)
  ledcWrite(PIN_VIBE, duty);
  g_vibeActive = true;
  g_vibeEndMs = now + (uint32_t)dur;
  strncpy(g_vibeCmdId, id, sizeof(g_vibeCmdId) - 1);
  g_vibeCmdId[sizeof(g_vibeCmdId) - 1] = '\0';

  JsonDocument doc;
  doc["type"] = "ack";
  doc["id"] = id;
  JsonObject applied = doc["applied"].to<JsonObject>();
  applied["strength"] = strength;
  applied["durationMs"] = dur;
  sendDoc(link, doc);
  return true;
}

static bool cmdServoMove(Link link, const char* id, JsonObjectConst params, uint32_t now) {
  if (g_servoEverMoved && (now - g_servoLastMoveMs) < SERVO_MIN_GAP_MS) {
    sendErr(link, id, "rate-limited");
    return false;
  }
  long angle = 0;
  if (!readIntParam(link, id, params, "angle", 90L,
                    SERVO_MIN_ANGLE, SERVO_MAX_ANGLE, &angle)) return false;
  if (!g_servoAttached) {
    g_servo.setPeriodHertz(50);
    g_servo.attach(PIN_SERVO, 500, 2400);   // SG90 典型脈寬
    g_servoAttached = true;
  }
  g_servo.write((int)angle);
  g_servoAngle = (int)angle;
  g_servoLastMoveMs = now;
  g_servoEverMoved = true;

  JsonDocument doc;
  doc["type"] = "ack";
  doc["id"] = id;
  JsonObject applied = doc["applied"].to<JsonObject>();
  applied["angle"] = angle;
  sendDoc(link, doc);
  return true;
}

// ---------------------------------------------------------------------------
// 訊息處理（三種通道共用同一協定）
// ---------------------------------------------------------------------------
static void handleMessage(const char* line, Link link) {
  // 跳過空行
  const char* p = line;
  while (*p == ' ' || *p == '\t' || *p == '\r') p++;
  if (*p == '\0') return;

  JsonDocument doc;
  DeserializationError err = deserializeJson(doc, p);
  if (err) {
    sendErr(link, nullptr, "bad-json");
    return;
  }
  const char* type = doc["type"] | "";
  if (type[0] == '\0') {
    sendErr(link, nullptr, "unknown-type");
    return;
  }

  uint32_t now = millis();

  // ---- 配對前也允許的訊息 -------------------------------------------------
  if (strcmp(type, "who") == 0) {
    sendHello(link);
    return;
  }
  if (strcmp(type, "pair") == 0) {
    const char* code = doc["code"] | "";
    if (!pairingEnabled()) {
      // 配對停用：誠實回 pair-ok（hello 的 pairing 欄位已是 false）
      g_linkPaired[link] = true;
      JsonDocument out;
      out["type"] = "pair-ok";
      sendDoc(link, out);
      return;
    }
    // 暴力猜測防護：鎖定期間不比對碼（也不延長鎖定），誠實回 pair-locked。
    if (pairLocked(link, now)) {
      JsonDocument out;
      out["type"] = "pair-fail";
      out["reason"] = "pair-locked";
      out["retryAfterMs"] = pairRetryAfterMs(link, now);
      sendDoc(link, out);
      return;
    }
    if (constantTimeEquals(code, PAIRING_CODE)) {
      g_linkPaired[link] = true;
      g_pairFailures[link] = 0;
      JsonDocument out;
      out["type"] = "pair-ok";
      sendDoc(link, out);
    } else {
      JsonDocument out;
      out["type"] = "pair-fail";
      if (++g_pairFailures[link] >= PAIR_MAX_FAILURES) {
        // 第 PAIR_MAX_FAILURES 次錯碼：這一則就開始鎖定，回覆一併說明。
        g_pairLocked[link] = true;
        g_pairLockedUntilMs[link] = now + PAIR_LOCKOUT_MS;
        g_pairFailures[link] = 0;
        out["reason"] = "pair-locked";
        out["retryAfterMs"] = PAIR_LOCKOUT_MS;
      }
      sendDoc(link, out);
    }
    return;
  }
  // 緊急停止「不」要求配對：方向是 fail-safe（只會把效果關掉），
  // 寧可讓未配對主機能停下裝置，也不要在緊急時被配對擋住。
  if (strcmp(type, "stop-all") == 0) {
    stopAllEffects();
    JsonDocument out;
    out["type"] = "ack";
    out["stopAll"] = true;
    sendDoc(link, out);
    return;
  }

  // ---- 其餘訊息（cmd / read / cancel）一律要求已配對 ----------------------
  if (!linkAuthorized(link)) {
    const char* id = doc["id"] | "";
    sendErr(link, id[0] ? id : nullptr, "not-paired");
    return;
  }

  if (strcmp(type, "read") == 0) {
    sendStateTo(link);
    return;
  }

  // 線協定 v1.1 的 `aip`（Character Session envelope）。這份參考韌體**不**
  // 參與角色 session：明確忽略，不回 err。
  //
  // 為什麼不落到 unknown-type：主機每送一則 session frame 就換回一則錯誤，
  // 會讓「這台裝置不支援 AIP」在收據與 log 上長得像「這台裝置壞了」。忽略
  // 是誠實的降級——主機那端本來就必須等對方主動送 envelope 才算它在 session
  // 裡，沒有 envelope 就沒有成員，不會有人以為它加入了。
  if (strcmp(type, "aip") == 0) {
    return;
  }

  if (strcmp(type, "cancel") == 0) {
    const char* id = doc["id"] | "";
    if (id[0] == '\0') {
      sendErr(link, nullptr, "bad-params");
      return;
    }
    bool cancelled = false;
    if (g_vibeActive && strncmp(g_vibeCmdId, id, sizeof(g_vibeCmdId)) == 0) {
      stopVibe(now);
      cancelled = true;
    }
    if (g_buzzActive && strncmp(g_buzzCmdId, id, sizeof(g_buzzCmdId)) == 0) {
      stopBuzzer();
      cancelled = true;
    }
    if (cancelled) {
      JsonDocument out;
      out["type"] = "ack";
      out["id"] = id;
      out["cancelled"] = true;
      sendDoc(link, out);
    } else {
      sendErr(link, id, "not-found");
    }
    return;
  }

  if (strcmp(type, "cmd") == 0) {
    const char* id = doc["id"] | "";
    if (id[0] == '\0') {
      sendErr(link, nullptr, "bad-params");
      return;
    }
    // 重放/重複：同 id 或同 nonce 都直接回 dup ack，「不」重新套用效果。
    // 兩個環並行：主機每次送新的隨機 nonce，所以「nonce 重複」只會是重放
    // （或主機 bug）；判成 dup 是 fail-safe 方向——寧可不動，不重複實體效果。
    const char* nonce = doc["nonce"] | "";
    if (ringContains(g_seenIds, id) || ringContains(g_seenNonces, nonce)) {
      JsonDocument out;
      out["type"] = "ack";
      out["id"] = id;
      out["dup"] = true;
      sendDoc(link, out);
      return;
    }
    const char* name = doc["name"] | "";
    JsonObjectConst params = doc["params"].as<JsonObjectConst>();

    bool applied = false;
    if      (strcmp(name, "led.set") == 0)     applied = cmdLedSet(link, id, params);
    else if (strcmp(name, "buzzer.beep") == 0) applied = cmdBuzzerBeep(link, id, params, now);
    else if (strcmp(name, "vibe.pulse") == 0)  applied = cmdVibePulse(link, id, params, now);
    else if (strcmp(name, "servo.move") == 0)  applied = cmdServoMove(link, id, params, now);
    else {
      sendErr(link, id, "unknown-cmd");
      return;
    }
    // 只有真的套用成功才記進去重環（rate-limited 的重試之後應該要能成功）。
    if (applied) {
      ringRemember(g_seenIds, &g_seenNext, id);
      ringRemember(g_seenNonces, &g_nonceNext, nonce);   // 空 nonce 不記
    }
    return;
  }

  sendErr(link, nullptr, "unknown-type");
}

// ---------------------------------------------------------------------------
// Serial：逐行讀（非阻塞）
// ---------------------------------------------------------------------------
static void pollSerial() {
  while (Serial.available() > 0) {
    char c = (char)Serial.read();
    if (c == '\n') {
      if (g_serialOverflow) {
        g_serialOverflow = false;   // 整行丟棄，回報一次錯誤
        g_serialLen = 0;
        sendErr(LINK_SERIAL, nullptr, "bad-json");
        continue;
      }
      g_serialBuf[g_serialLen] = '\0';
      g_serialLen = 0;
      handleMessage(g_serialBuf, LINK_SERIAL);
    } else if (g_serialLen < sizeof(g_serialBuf) - 1) {
      g_serialBuf[g_serialLen++] = c;
    } else {
      g_serialOverflow = true;      // 行太長：吃掉到換行為止
    }
  }
}

// ---------------------------------------------------------------------------
// MQTT
// ---------------------------------------------------------------------------
static void mqttCallback(char* topic, byte* payload, unsigned int length) {
  (void)topic;
  if (length >= 640) {
    sendErr(LINK_MQTT, nullptr, "bad-json");
    return;
  }
  char buf[640];
  memcpy(buf, payload, length);
  buf[length] = '\0';
  handleMessage(buf, LINK_MQTT);
}

static bool wifiConfigured() { return WIFI_SSID[0] != '\0'; }

// 重連策略（broker 不通時「不」可以每輪 loop 都去撞一次）：
//   1) 已連線：只跑非阻塞的 g_mqtt.loop()。
//   2) 未連線：最多每 g_mqttRetryDelayMs 試一次；失敗就指數退避
//      （10s → 20s → 40s → 60s 上限），連上後歸零。
//   3) vibe/buzzer 進行中一律「不」嘗試連線——connect() 是本韌體唯一
//      可能阻塞數百毫秒的呼叫（TCP connect ≤500ms＋等 CONNACK ≤1s），
//      絕不能撐破 durationMs 硬上限。最多延後 3s（vibe 上限）再連。
//   4) 整段完全不影響 Serial：pollSerial() 在 loop() 中排在本函式之前，
//      且不論 Wi-Fi/MQTT 狀態如何都會執行。
static void maintainMqtt(uint32_t now) {
  if (!wifiConfigured() || WiFi.status() != WL_CONNECTED) return;
  if (g_mqtt.connected()) {
    g_mqtt.loop();
    return;
  }
  // 斷線＝這條通道的配對失效（下一個連上的人不能繼承配對）。
  g_linkPaired[LINK_MQTT] = false;
  if (g_vibeActive || g_buzzActive) return;             // 見 (3)
  if (g_lastMqttAttemptMs != 0 && (now - g_lastMqttAttemptMs) < g_mqttRetryDelayMs) return;
  g_lastMqttAttemptMs = (now == 0) ? 1 : now;           // 0 是「還沒試過」的哨兵
  if (g_mqtt.connect(DEVICE_ID)) {
    g_mqttRetryDelayMs = MQTT_RETRY_BASE_MS;
    g_mqttFailedBefore = false;
    g_mqtt.subscribe(g_mqttTopicIn);
    sendHello(LINK_MQTT);
  } else {
    // 第一次失敗後等 MQTT_RETRY_BASE_MS，之後每次失敗把間隔加倍到上限：
    // 10s → 20s → 40s → 60s → 60s …（連上後歸零）
    if (g_mqttFailedBefore) {
      uint32_t next = g_mqttRetryDelayMs * 2;
      g_mqttRetryDelayMs = (next > MQTT_RETRY_MAX_MS) ? MQTT_RETRY_MAX_MS : next;
    }
    g_mqttFailedBefore = true;
  }
}

// ---------------------------------------------------------------------------
// BLE（可選；ENABLE_BLE=1 時編譯）
// ---------------------------------------------------------------------------
#if ENABLE_BLE
class CompanionServerCallbacks : public NimBLEServerCallbacks {
  void onConnect(NimBLEServer* server, NimBLEConnInfo& connInfo) override {
    // 記下 server 與這條連線的 handle：送出時要問「這條連線協商到多大的
    // MTU」，才知道一次 notify 塞得下多少 bytes（見 bleNotifyPayloadMax）。
    g_bleServer = server;
    g_bleConnHandle = connInfo.getConnHandle();
    g_bleConnected = true;
  }
  void onDisconnect(NimBLEServer* server, NimBLEConnInfo& connInfo, int reason) override {
    (void)server; (void)connInfo; (void)reason;
    g_bleConnected = false;
    g_bleConnHandle = 0xFFFF;
    // 這兩個是 BLE task 唯一會寫的共享狀態，且方向都是「收緊」
    // （斷線＝配對失效、不再送資料）；效果與回覆一律由 loop() 處理。
    g_linkPaired[LINK_BLE] = false;
    NimBLEDevice::startAdvertising();
  }
};

class CompanionWriteCallbacks : public NimBLECharacteristicCallbacks {
  void onWrite(NimBLECharacteristic* chr, NimBLEConnInfo& connInfo) override {
    (void)connInfo;
    // 這裡是 NimBLE host task（core 0）。**不**在這裡處理訊息、也不在這裡
    // 送任何回覆——否則會與 loop()（core 1）競用 g_vibe*／g_buzz*／notify
    // characteristic（曾經的後果：vibe 被 loop() 停掉，回呼卻已回 ack
    // 宣稱 applied）。只做「複製進有界佇列」這一件事。
    // 假設一次 write 就是一個完整 JSON（≤ 協商後 MTU；README 有註明限制）。
    static BleMsg staged;   // NimBLE 回呼是單執行緒，暫存區可安全重用
    std::string v = chr->getValue();
    if (v.size() >= BLE_MSG_MAX) { g_bleDropTooLong = true; return; }
    memcpy(staged.line, v.data(), v.size());
    staged.line[v.size()] = '\0';
    if (g_bleQueue == nullptr || xQueueSend(g_bleQueue, &staged, 0) != pdTRUE) {
      g_bleDropBusy = true;   // 有界佇列滿：誠實回 err busy，不假裝收下
    }
  }
};

// loop() 端：把佇列裡的訊息用與 Serial/MQTT 完全相同的路徑處理，
// 所有 ack/err/state 都在這條 task 上送出。
static void drainBleQueue() {
  if (g_bleQueue == nullptr) return;
  static BleMsg msg;
  if (g_bleDropTooLong) { g_bleDropTooLong = false; sendErr(LINK_BLE, nullptr, "bad-json"); }
  if (g_bleDropBusy)    { g_bleDropBusy = false;    sendErr(LINK_BLE, nullptr, "busy"); }
  // 每輪最多取 BLE_QUEUE_SLOTS 筆（有界，不會被灌爆成無限迴圈）。
  for (size_t i = 0; i < BLE_QUEUE_SLOTS; i++) {
    if (xQueueReceive(g_bleQueue, &msg, 0) != pdTRUE) return;
    // 對端已離線：丟棄（配對已於 onDisconnect 失效，也不該再送效果）。
    if (!g_bleConnected) continue;
    handleMessage(msg.line, LINK_BLE);
  }
}

static void setupBle() {
  g_bleQueue = xQueueCreate(BLE_QUEUE_SLOTS, sizeof(BleMsg));
  NimBLEDevice::init(DEVICE_ID);
  // 偏好 MTU：對端同意的話，一則 state（193 bytes）一次就送得完。
  // 對端不同意也沒關係——sendLine 會依實際協商值分段。
  NimBLEDevice::setMTU(BLE_PREFERRED_MTU);
  NimBLEServer* server = NimBLEDevice::createServer();
  server->setCallbacks(new CompanionServerCallbacks());
  NimBLEService* svc = server->createService(BLE_SERVICE_UUID);
  NimBLECharacteristic* writeChar = svc->createCharacteristic(
      BLE_WRITE_CHAR_UUID, NIMBLE_PROPERTY::WRITE);
  writeChar->setCallbacks(new CompanionWriteCallbacks());
  g_bleNotifyChar = svc->createCharacteristic(
      BLE_NOTIFY_CHAR_UUID, NIMBLE_PROPERTY::NOTIFY);
  // NimBLE-Arduino 2.x：service 隨 server->start() 一起啟動；
  // 舊版 svc->start() 已 deprecated（無效果），故不再呼叫。
  server->start();
  NimBLEAdvertising* adv = NimBLEDevice::getAdvertising();
  adv->addServiceUUID(BLE_SERVICE_UUID);
  // NimBLE-Arduino 2.x 預設「不」廣播裝置名稱、也不開 scan response
  // （init(DEVICE_ID) 只設 GAP device name，要連上後才讀得到）。runtime 端
  // 掃描以 service UUID 為主、名稱為輔，但名稱仍必須真的廣播出去：
  // flags(3B)＋128-bit service UUID(18B) 已佔掉 31B 主封包的大半，名稱只
  // 放得進 scan response——所以先 enableScanResponse 再 setName（順序
  // 反過來名稱會被塞進主封包而放不下）。
  adv->enableScanResponse(true);
  adv->setName(DEVICE_ID);
  adv->start();
}
#endif

// ---------------------------------------------------------------------------
// setup / loop
// ---------------------------------------------------------------------------
void setup() {
  Serial.begin(115200);

  // GPIO
  pinMode(PIN_BUTTON, INPUT_PULLUP);
  pinMode(PIN_TRIG, OUTPUT);
  digitalWrite(PIN_TRIG, LOW);
  pinMode(PIN_ECHO, INPUT);
  analogReadResolution(12);

  // PWM（Arduino ESP32 core 3.x 的 pin-based LEDC API）
  ledcAttach(PIN_LED_R, 5000, 8);
  ledcAttach(PIN_LED_G, 5000, 8);
  ledcAttach(PIN_LED_B, 5000, 8);
  ledcAttach(PIN_VIBE, 2000, 8);
  ledcAttach(PIN_BUZZER, 2000, 10);
  ledcWrite(PIN_LED_R, 0);
  ledcWrite(PIN_LED_G, 0);
  ledcWrite(PIN_LED_B, 0);
  ledcWrite(PIN_VIBE, 0);
  ledcWrite(PIN_BUZZER, 0);

  // Servo 計時器（ESP32Servo 建議先配置）
  ESP32PWM::allocateTimer(0);
  ESP32PWM::allocateTimer(1);
  ESP32PWM::allocateTimer(2);
  ESP32PWM::allocateTimer(3);

  g_dht.begin();
  memset(g_seenIds, 0, sizeof(g_seenIds));
  memset(g_seenNonces, 0, sizeof(g_seenNonces));

  // Wi-Fi（非阻塞：begin 之後由 loop 的 maintainMqtt 檢查狀態）
  if (wifiConfigured()) {
    WiFi.mode(WIFI_STA);
    WiFi.begin(WIFI_SSID, WIFI_PASS);
    snprintf(g_mqttTopicIn, sizeof(g_mqttTopicIn), "%s/to-device",
             MQTT_TOPIC_PREFIX);
    snprintf(g_mqttTopicOut, sizeof(g_mqttTopicOut), "%s/from-device",
             MQTT_TOPIC_PREFIX);
    g_mqtt.setServer(MQTT_HOST, MQTT_PORT);
    g_mqtt.setCallback(mqttCallback);
    g_mqtt.setBufferSize(1024);   // state 訊息可能超過預設 256 bytes
    // 把兩個「可能阻塞很久」的預設值改短：TCP connect 預設 3000ms、
    // PubSubClient 等 CONNACK/封包預設 15s。broker 不通時這兩者就是
    // loop() 的阻塞長度，必須壓到硬限制容忍得起的量級。
    g_wifiClient.setConnectionTimeout(MQTT_CONNECT_TIMEOUT_MS);
    g_mqtt.setSocketTimeout(MQTT_SOCKET_TIMEOUT_S);
  }

#if ENABLE_BLE
  setupBle();
#endif

  // 開機宣告（Serial 一定送；MQTT/BLE 於連上時各自送 hello）
  sendHello(LINK_SERIAL);
}

void loop() {
  uint32_t now = millis();

  // 1) 計時效果到期最優先——排在任何「可能阻塞」的呼叫之前，
  //    vibe/buzzer 的 durationMs 硬上限才不會被網路重連撐破。
  expireTimedEffects(now);

  // 2) 輸入。Serial 永遠排在網路之前：Wi-Fi/MQTT 不通完全不影響 Serial。
  pollSerial();
#if ENABLE_BLE
  drainBleQueue();   // BLE 訊息也在這條 task 上處理與回覆
#endif
  maintainMqtt(now);

  // 3) 阻塞點之後再檢一次到期（maintainMqtt 的 connect 最多 ~1.5s，
  //    且只在沒有效果進行中時才會發生——這裡是保險）。
  now = millis();
  expireTimedEffects(now);

  // 4) 感測
  refreshDhtIfDue(now);
  if (pollButtonEdge(now)) {
    pushStateToPairedLinks();   // 按鈕邊緣（去彈跳後）→ 立即推播
    g_lastStatePushMs = now;
  }

  // 5) 週期推播
  if ((now - g_lastStatePushMs) >= STATE_PERIOD_MS) {
    g_lastStatePushMs = now;
    pushStateToPairedLinks();
  }
}
