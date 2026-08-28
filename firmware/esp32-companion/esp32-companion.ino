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
static const char* BLE_SERVICE_UUID     = "7f2a0001-c701-4c9e-8f7e-2b3d5a1e9c01";
static const char* BLE_WRITE_CHAR_UUID  = "7f2a0002-c701-4c9e-8f7e-2b3d5a1e9c01";
static const char* BLE_NOTIFY_CHAR_UUID = "7f2a0003-c701-4c9e-8f7e-2b3d5a1e9c01";

// ---------------------------------------------------------------------------
// 連線通道（每個通道各自維護配對狀態）
// ---------------------------------------------------------------------------
enum Link : uint8_t { LINK_SERIAL = 0, LINK_MQTT = 1, LINK_BLE = 2, LINK_COUNT = 3 };

static bool g_linkPaired[LINK_COUNT] = { false, false, false };

// 配對是否啟用（PAIRING_CODE 為空字串 = 停用；hello 仍誠實回報）
static bool pairingEnabled() { return PAIRING_CODE[0] != '\0'; }
static bool linkAuthorized(Link link) {
  return !pairingEnabled() || g_linkPaired[link];
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
static uint32_t g_lastMqttAttemptMs = 0;

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

// 重複指令去重：最近 16 個 cmd id 的環形緩衝
static const size_t CMD_RING_SIZE = 16;
static const size_t CMD_ID_MAX    = 48;
static char   g_seenIds[CMD_RING_SIZE][CMD_ID_MAX];
static size_t g_seenNext = 0;

#if ENABLE_BLE
static NimBLECharacteristic* g_bleNotifyChar = nullptr;
static volatile bool g_bleConnected = false;
#endif

// ---------------------------------------------------------------------------
// 函式原型（避免 Arduino 前處理器對自動原型的順序問題）
// ---------------------------------------------------------------------------
static void handleMessage(const char* line, Link link);
static void sendLine(Link link, const char* line);
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

// cmd id 去重環形緩衝
static bool cmdIdSeen(const char* id) {
  for (size_t i = 0; i < CMD_RING_SIZE; i++) {
    if (g_seenIds[i][0] != '\0' && strncmp(g_seenIds[i], id, CMD_ID_MAX - 1) == 0) {
      return true;
    }
  }
  return false;
}

static void cmdIdRemember(const char* id) {
  strncpy(g_seenIds[g_seenNext], id, CMD_ID_MAX - 1);
  g_seenIds[g_seenNext][CMD_ID_MAX - 1] = '\0';
  g_seenNext = (g_seenNext + 1) % CMD_RING_SIZE;
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
        g_bleNotifyChar->setValue((const uint8_t*)line, strlen(line));
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
  long r = clampLong(params["r"] | 0L, 0, 255);
  long g = clampLong(params["g"] | 0L, 0, 255);
  long b = clampLong(params["b"] | 0L, 0, 255);
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
  long freq = clampLong(params["freqHz"] | 1000L, (long)BUZZ_MIN_FREQ_HZ, (long)BUZZ_MAX_FREQ_HZ);
  long dur  = clampLong(params["durationMs"] | 200L, 1, (long)BUZZ_MAX_DURATION_MS);
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
  float strength = clampFloat(params["strength"] | 0.0f, 0.0f, VIBE_MAX_STRENGTH);
  long dur = clampLong(params["durationMs"] | 200L, 1, (long)VIBE_MAX_DURATION_MS);
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
  long angle = clampLong(params["angle"] | 90L, SERVO_MIN_ANGLE, SERVO_MAX_ANGLE);
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
    if (constantTimeEquals(code, PAIRING_CODE)) {
      g_linkPaired[link] = true;
      JsonDocument out;
      out["type"] = "pair-ok";
      sendDoc(link, out);
    } else {
      JsonDocument out;
      out["type"] = "pair-fail";
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
    // 重放/重複：同 id 直接回 dup ack，「不」重新套用效果。
    if (cmdIdSeen(id)) {
      JsonDocument out;
      out["type"] = "ack";
      out["id"] = id;
      out["dup"] = true;
      sendDoc(link, out);
      return;
    }
    // nonce 欄位目前僅收下不驗（重放防護以 id 環形緩衝為準，README 有註明）。
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
    if (applied) cmdIdRemember(id);
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

// 非阻塞重連：每 3 秒最多嘗試一次（connect 本身可能短暫阻塞，見 README）。
static void maintainMqtt(uint32_t now) {
  if (!wifiConfigured() || WiFi.status() != WL_CONNECTED) return;
  if (g_mqtt.connected()) {
    g_mqtt.loop();
    return;
  }
  // 斷線＝這條通道的配對失效（下一個連上的人不能繼承配對）。
  g_linkPaired[LINK_MQTT] = false;
  if (g_lastMqttAttemptMs != 0 && (now - g_lastMqttAttemptMs) < 3000) return;
  g_lastMqttAttemptMs = now;
  if (g_mqtt.connect(DEVICE_ID)) {
    g_mqtt.subscribe(g_mqttTopicIn);
    sendHello(LINK_MQTT);
  }
}

// ---------------------------------------------------------------------------
// BLE（可選；ENABLE_BLE=1 時編譯）
// ---------------------------------------------------------------------------
#if ENABLE_BLE
class CompanionServerCallbacks : public NimBLEServerCallbacks {
  void onConnect(NimBLEServer* server, NimBLEConnInfo& connInfo) override {
    (void)server; (void)connInfo;
    g_bleConnected = true;
  }
  void onDisconnect(NimBLEServer* server, NimBLEConnInfo& connInfo, int reason) override {
    (void)server; (void)connInfo; (void)reason;
    g_bleConnected = false;
    g_linkPaired[LINK_BLE] = false;   // 斷線＝配對失效
    NimBLEDevice::startAdvertising();
  }
};

class CompanionWriteCallbacks : public NimBLECharacteristicCallbacks {
  void onWrite(NimBLECharacteristic* chr, NimBLEConnInfo& connInfo) override {
    (void)connInfo;
    // 假設一次 write 就是一個完整 JSON（≤ 協商後 MTU；README 有註明限制）。
    std::string v = chr->getValue();
    handleMessage(v.c_str(), LINK_BLE);
  }
};

static void setupBle() {
  NimBLEDevice::init(DEVICE_ID);
  NimBLEServer* server = NimBLEDevice::createServer();
  server->setCallbacks(new CompanionServerCallbacks());
  NimBLEService* svc = server->createService(BLE_SERVICE_UUID);
  NimBLECharacteristic* writeChar = svc->createCharacteristic(
      BLE_WRITE_CHAR_UUID, NIMBLE_PROPERTY::WRITE);
  writeChar->setCallbacks(new CompanionWriteCallbacks());
  g_bleNotifyChar = svc->createCharacteristic(
      BLE_NOTIFY_CHAR_UUID, NIMBLE_PROPERTY::NOTIFY);
  svc->start();
  NimBLEAdvertising* adv = NimBLEDevice::getAdvertising();
  adv->addServiceUUID(BLE_SERVICE_UUID);
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
  }

#if ENABLE_BLE
  setupBle();
#endif

  // 開機宣告（Serial 一定送；MQTT/BLE 於連上時各自送 hello）
  sendHello(LINK_SERIAL);
}

void loop() {
  uint32_t now = millis();

  // 1) 輸入
  pollSerial();
  maintainMqtt(now);

  // 2) 計時效果到期（millis 溢位安全的減法比較）
  if (g_vibeActive && (int32_t)(now - g_vibeEndMs) >= 0) {
    stopVibe(now);
  }
  if (g_buzzActive && (int32_t)(now - g_buzzEndMs) >= 0) {
    stopBuzzer();
  }

  // 3) 感測
  refreshDhtIfDue(now);
  if (pollButtonEdge(now)) {
    pushStateToPairedLinks();   // 按鈕邊緣（去彈跳後）→ 立即推播
    g_lastStatePushMs = now;
  }

  // 4) 週期推播
  if ((now - g_lastStatePushMs) >= STATE_PERIOD_MS) {
    g_lastStatePushMs = now;
    pushStateToPairedLinks();
  }
}
