#!/usr/bin/env python3
"""ESP32 參考裝置【模擬器】—— serial(pty) 版。

明確標示：這是模擬器，不是真機。它與 firmware/esp32-companion 說同一套
線協定（hello/pair/cmd/ack/state/cancel/stop-all），並鏡射韌體的硬限制與
節奏（見下方常數，全部對照 esp32-companion.ino）：

  led.set     : r/g/b —— 整數 0–255 = 絕對值；浮點 0.0–1.0 = 比例
                （0.8 → 204）；浮點 > 1.0 當絕對值；缺漏/null = 0
  vibe.pulse  : strength clamp ≤ 0.8、durationMs clamp ≤ 3000、
                進行中或距上次結束 < 500ms → err rate-limited
  buzzer.beep : freqHz clamp 200..4000、durationMs clamp ≤ 2000
  servo.move  : angle clamp 10..170、距上次移動 < 300ms → err rate-limited
  數值參數    : 一律以浮點讀取後四捨五入＋clamp（主機的 {{magnitude}} 是
                JSON number/float）；非數值 → err bad-params（不靜默當 0）
  重放防護    : cmd id 與 nonce 各一組 16 筆環形緩衝，並行比對；命中任一
                → ack dup:true 不套用。只有「真的套用成功」才記入
                （rate-limited 之後重試應該要能成功）——與韌體一致
  單行上限    : 639 bytes（韌體 g_serialBuf[640]）；超過整行丟棄並回
                無 id 的 err bad-json——與韌體 pollSerial() 一致
  配對鎖定    : 連續 5 次錯碼 → 鎖定 30 s（--pair-lockout-ms 可縮短，測試
                用）；鎖定期間 pair 一律回 pair-fail reason:pair-locked，
                不比對、不延長；hello.pairingLocked 誠實回報——與韌體一致
  hello       : type/deviceId/fw/proto/caps/pairing/pairingLocked 七個欄位、
                順序與型別與韌體 sendHello() 相同（fw 值刻意不同：這是模擬器）
  state       : 每 5 秒自動推播一次（僅在已配對時，不對未配對連線洩漏感測）；
                按鈕邊緣（去彈跳後）立即推播一次——與韌體 loop() 一致

線協定 v1.1 的 `aip`（Character Session envelope）：

  →裝置   配對後一律**忽略**（不回 err、不當 unknown-type），配對前照舊
          `not-paired`——與韌體 handleMessage() 相同。
  裝置→   參考韌體不送；模擬器用控制通道（見下）代打，讓 host 端的
          Character Session 綁定在沒有真板時也驗得到。

線協定 v1.2 的 `aip-frag`（一則 envelope 的分片）：

  能力宣告 hello.caps 含 "aip.frag/1"（`--no-frag` 關閉，用來驗降級路徑）。
           **參考韌體不宣告它**——真板沒有重組緩衝，模擬器不得替它宣稱。
  →裝置   依 host 的同一套規則重組（有界 8 KiB／≤64 片／自最後一片起 2 秒
           逾時／crc32 核對／缺片重片亂序截斷一律整筆丟棄），組好之後與 `aip`
           一樣忽略，並在 log 記一行 `>+ <完整 envelope 行>`（`>+` ＝重組出來
           的，不是線上真的出現過的一行）。
  裝置→   控制通道要送的 envelope 若超過單行上限（639 bytes）就切片送出；
           `--no-frag` 時改為誠實拒絕送出（記 `## aip op refused`）。

控制通道（stdin，一行一則 JSON；仿 crates/interaction-runtime/examples/
fake_iphone.rs）：

  {"op":"aip-capability"}                    協商（role remote-renderer）
  {"op":"aip-touch","kind":"tap",            觸碰事件（可覆寫 source 測偽造）
   "expiresInMs":5000,"source":{..}?}
  {"op":"aip-resume","lastRevision":n,       續傳查詢
   "lastSequence":n,"epoch":n}
  {"op":"aip-raw","envelope":{..}}           任意 envelope（測未知／超大）
  {"op":"aip-partial","bytes":N}             只送一筆分片傳輸的**第一片**
                                             （host 端的取消／逾時路徑要驗）

  未配對時一律拒絕送出（記 `## aip op refused`）：未配對的通道不得送出
  session 流量，方向與 host 端的准入閘門一致。

感測面控制通道（韌體上是真實感測器；模擬器用這些替代，讓閉環的「獨立
觀察」半邊也能在模擬器上驗到）：

  --facts-file PATH   JSON 物件，內容變更時（每 tick 重讀）覆寫
                      button/distanceMm/lux/tempC；缺漏的鍵維持現值；
                      "tempC": null 與 "distanceMm": -1 原樣穿透
                      （韌體對讀不到的感測器就是回這兩個值）。
  SIGUSR1             翻轉按鈕（按下↔放開），與韌體按鈕邊緣同樣立即推播 state。
  --sensors-absent    啟動時就處於「感測器未接」：distanceMm=-1、tempC=null。

用法：esp32-serial-sim.py --device-id esp32-sim01 --pairing-code 9927 \
        --pty-path-file /tmp/sim-pty --log /tmp/sim.log
會建立一個 pty，把 slave 路徑寫進 --pty-path-file，然後在 master 端服務。
"""

import argparse
import json
import math
import os
import pty
import re
import select
import signal
import struct
import sys
import termios
import time
import tty
import zlib

# --- 韌體硬限制（對照 firmware/esp32-companion/esp32-companion.ino）-------
VIBE_MAX_STRENGTH = 0.8
VIBE_MAX_DURATION_MS = 3000
VIBE_MIN_GAP_MS = 500

BUZZ_MIN_FREQ_HZ = 200
BUZZ_MAX_FREQ_HZ = 4000
BUZZ_MAX_DURATION_MS = 2000

SERVO_MIN_ANGLE = 10
SERVO_MAX_ANGLE = 170
SERVO_MIN_GAP_MS = 300

STATE_PERIOD_MS = 5000
DEDUPE_RING = 16

# 韌體 g_serialBuf[640]：一行最多 639 bytes，第 640 個位元組起整行丟棄。
MAX_LINE_BYTES = 639

# --- 線協定 v1.2 的分片（對照 crates/interaction-adapter-declarative/src/fragment.rs）
FRAG_CAP = "aip.frag/1"
# 重組後的 envelope 上限（host 的 MAX_REASSEMBLED_BYTES）。分片是為了穿過
# 行上限，不是為了放寬 envelope 上限。
MAX_REASSEMBLED_BYTES = 8 * 1024
# 一筆傳輸最多幾片（host 的 MAX_FRAGMENTS）。
MAX_FRAGMENTS = 64
# 自最後一片起還能等多久（host 的 FRAGMENT_TIMEOUT）。
FRAGMENT_TIMEOUT_MS = 2000

# 配對暴力猜測防護（韌體 PAIR_MAX_FAILURES / PAIR_LOCKOUT_MS）。
PAIR_MAX_FAILURES = 5
PAIR_LOCKOUT_MS = 30000

# 韌體 readDistanceMm()/refreshDhtIfDue() 對「讀不到」的誠實值。
DISTANCE_ABSENT = -1
TEMP_ABSENT = None

parser = argparse.ArgumentParser()
parser.add_argument("--device-id", required=True)
parser.add_argument("--pairing-code", default="")
parser.add_argument("--pty-path-file", required=True)
parser.add_argument("--log", default="/dev/null")
parser.add_argument("--facts-file", default=None,
                    help="JSON 物件；內容變更時覆寫 button/distanceMm/lux/tempC")
parser.add_argument("--sensors-absent", action="store_true",
                    help="啟動即為感測器未接：distanceMm=-1、tempC=null")
parser.add_argument("--pair-lockout-ms", type=int, default=PAIR_LOCKOUT_MS,
                    help="配對鎖定時間（預設 30000，與韌體相同；測試可縮短）")
parser.add_argument("--no-frag", action="store_true",
                    help="不宣告 aip.frag/1：驗證 host 端「對端不會重組就誠實拒絕」的降級路徑")
args = parser.parse_args()

master, slave = pty.openpty()
# raw 模式：關掉 echo/canonical，host 端才能把 pty 當乾淨的位元組管道。
tty.setraw(master, when=termios.TCSANOW)
tty.setraw(slave, when=termios.TCSANOW)
slave_path = os.ttyname(slave)
with open(args.pty_path_file, "w") as f:
    f.write(slave_path)

log = open(args.log, "a")


def _reject_constant(name):
    """NaN／Infinity／-Infinity 字面值：韌體的 ArduinoJson 解析不了，
    整則訊息回 bad-json——模擬器一樣。"""
    raise ValueError(f"json constant not accepted by the firmware parser: {name}")


def now_ms():
    return int(time.monotonic() * 1000)


def emit(obj):
    line = dumps_like_firmware(obj) + "\n"
    os.write(master, line.encode())
    log.write(f"<< {line}")
    log.flush()


def note(text):
    log.write(f"## {text}\n")
    log.flush()


state = {
    "paired": args.pairing_code == "",
    # 配對暴力猜測防護（單一通道；韌體對 Serial/MQTT/BLE 各自一組）
    "pair_failures": 0,
    "pair_locked": False,
    "pair_locked_until_ms": 0,
    "seen_ids": [],
    "seen_nonces": [],
    "led": {"r": 0, "g": 0, "b": 0},
    # vibe：進行中的脈衝＋節流所需的時間戳
    "vibe_active": False,
    "vibe_cmd_id": "",
    "vibe_end_ms": 0,
    "vibe_ever_ran": False,
    "vibe_last_end_ms": 0,
    # buzzer：進行中的 beep
    "buzz_active": False,
    "buzz_cmd_id": "",
    "buzz_end_ms": 0,
    # servo：最後角度＋節流
    "servo_angle": 90,
    "servo_ever_moved": False,
    "servo_last_move_ms": 0,
    "last_state_push_ms": 0,
    # 控制通道送出的 aip envelope 序號（messageId 用；有界遞增）
    "aip_seq": 0,
    # 線協定 v1.2：出站分片的傳輸序號（有界遞增）。
    "aip_xfer": 0,
    # 進行中的入站分片傳輸（同時只有一筆——有界）。
    "aip_rx": None,
}

# 感測面（韌體上是真實感測器；這裡由控制通道決定）。
sensors = {
    "button": False,
    "distanceMm": DISTANCE_ABSENT if args.sensors_absent else 842,
    "lux": 133,
    "tempC": TEMP_ABSENT if args.sensors_absent else 24.5,
}

# SIGUSR1 → 翻轉按鈕（handler 只設旗標，實際處理在主迴圈——與韌體按鈕
# 邊緣在 loop() 裡處理同構）。
pending_button_toggle = False


def on_sigusr1(signum, frame):
    global pending_button_toggle
    pending_button_toggle = True


signal.signal(signal.SIGUSR1, on_sigusr1)


def clamp(value, lo, hi):
    return max(lo, min(hi, value))


# --- 數值參數解析（逐位鏡射韌體的 readNumber/roundToLong/readLedChannel）---
class BadParam(Exception):
    """非數值參數 → err bad-params（韌體同樣回 err，兩端一致）。"""


# roundToLong() 在超出範圍時的收斂目標。韌體的 long 是 32-bit，
# 之後一定會 clampLong 進 lo..hi；這裡用同樣量級的哨兵值，讓 clamp
# 得到與韌體相同的邊界值（不是讓模擬器崩潰）。
INT_SENTINEL = 2 ** 31 - 1


def f32(v):
    """收斂成 IEEE754 單精度。韌體以 ArduinoJson 的 as<float>() 取值、之後
    全程用 float 運算；Python 預設是 double，取整前不鏡射精度的話，像
    {"r":0.3} 這種輸入會一端算 76、另一端算 77。

    超出 float32 範圍（例如 1e39）：ArduinoJson 的 as<float>() 會溢位成
    ±inf，韌體照樣 clamp 後回 ack。這裡鏡射同一件事——絕不讓 struct 的
    OverflowError 冒出去把整個模擬器打死（那會讓 host 看到「裝置憑空消失」，
    把參數問題誤診成傳輸問題）。"""
    try:
        return struct.unpack("<f", struct.pack("<f", v))[0]
    except (OverflowError, ValueError):
        return math.copysign(math.inf, v)


def read_number(params, key):
    """回傳 (值 or None, 是否為 JSON 整數字面值)；非數值丟 BadParam。
    缺漏與明寫 null 同義（回 None，呼叫端用預設值）——與韌體 isNull() 一致。"""
    if not isinstance(params, dict) or key not in params:
        return None, False
    v = params[key]
    if v is None:
        return None, False
    if isinstance(v, bool) or not isinstance(v, (int, float)):
        raise BadParam(key)
    return float(v), isinstance(v, int)


def round_to_int(v):
    """韌體 roundToLong()：float 加 0.5 後往零截斷（單精度）。
    ±inf／NaN 收斂成哨兵整數，交給後續的 clamp（韌體的 float→long 轉換
    同樣不會回傳一個「合法的中間值」，最終都被 clampLong 夾進邊界）。"""
    x = f32(f32(v) + 0.5)
    if math.isnan(x):
        return 0
    if math.isinf(x):
        return INT_SENTINEL if x > 0 else -INT_SENTINEL
    return int(x)


def read_int_param(params, key, fallback, lo, hi):
    raw, _ = read_number(params, key)
    value = fallback if raw is None else round_to_int(raw)
    return int(clamp(value, lo, hi))


# ---------------------------------------------------------------------------
# 浮點輸出：韌體怎麼寫上線，模擬器就怎麼寫
#
# 韌體的 ack 走 ArduinoJson v7：`applied["strength"] = strength;`（C `float`）
# → TextFormatter::writeFloat(value, sizeof(T) >= 8 ? 9 : 6) → 4 bytes 的
# float 用 **6 位小數**、去尾零，|v| >= 1e7 或 0 < |v| <= 1e-5 才走指數。
# Python 的 json.dumps 直接吐 float64 的 repr：float32 化之後的 0.7 會變成
# 0.699999988079071——真板永遠不會這樣送，兩端 ack 的位元組就不同了。
# 下面逐步鏡射 ArduinoJson 的 Numbers/FloatParts.hpp 與 Json/TextFormatter.hpp。
# ---------------------------------------------------------------------------

# ArduinoJson FloatTraits<double>：二進位冪次表（JsonFloat 預設是 double）。
_POS_BIN_POW10 = [1e1, 1e2, 1e4, 1e8, 1e16, 1e32, 1e64, 1e128, 1e256]
_NEG_BIN_POW10 = [1e-1, 1e-2, 1e-4, 1e-8, 1e-16, 1e-32, 1e-64, 1e-128, 1e-256]
POSITIVE_EXPONENTIATION_THRESHOLD = 1e7
NEGATIVE_EXPONENTIATION_THRESHOLD = 1e-5
FLOAT_DECIMAL_PLACES = 6            # sizeof(float) < 8 → 6（double 才是 9）
# 佔位字串的前綴帶一個每次啟動都不同的隨機 token：host 送來的 cmd id 會被
# 原樣回進 ack，不能讓一個猜得到的前綴被拿來注入裸字面值。
_FW_FLOAT_MARK = "fw-float-" + os.urandom(8).hex() + ":"
_FW_FLOAT_RE = re.compile('"' + re.escape(_FW_FLOAT_MARK) + r'([-0-9.enul]+)"')


def _normalize_like_arduinojson(value):
    """ArduinoJson normalize()：把 value 帶進 [1,10) 並回傳十進位指數。
    兩個迴圈共用 index／bit（第一個迴圈跑完後 bit 已為 0）——照抄，
    否則邊界值會與韌體分岔。"""
    powers = 0
    index = 8                        # sizeof(JsonFloat) == 8
    bit = 1 << index
    if value >= POSITIVE_EXPONENTIATION_THRESHOLD:
        while index >= 0:
            if value >= _POS_BIN_POW10[index]:
                value *= _NEG_BIN_POW10[index]
                powers += bit
            bit >>= 1
            index -= 1
    if value > 0 and value <= NEGATIVE_EXPONENTIATION_THRESHOLD:
        while index >= 0:
            if value < _NEG_BIN_POW10[index] * 10:
                value *= _POS_BIN_POW10[index]
                powers -= bit
            bit >>= 1
            index -= 1
    return value, powers


def firmware_float_text(value, decimal_places=FLOAT_DECIMAL_PLACES):
    """把一個值序列化成參考韌體會寫上線的**同一串位元組**。"""
    value = f32(value)
    if math.isnan(value):
        return "null"                # ARDUINOJSON_ENABLE_NAN 預設關閉
    sign = ""
    if math.isinf(value):
        return "null"                # ARDUINOJSON_ENABLE_INFINITY 預設關閉
    if value < 0.0:
        sign = "-"
        value = -value
    value = float(value)             # JsonFloat(double)，與韌體同一步
    max_decimal = 10 ** decimal_places
    value, exponent = _normalize_like_arduinojson(value)
    integral = int(value)            # uint32_t 截斷
    tmp = integral
    while tmp >= 10:                 # 整數位吃掉小數位額度
        max_decimal //= 10
        decimal_places -= 1
        tmp //= 10
    remainder = (value - float(integral)) * float(max_decimal)
    decimal = int(remainder)
    remainder -= float(decimal)
    decimal += int(remainder * 2)    # remainder >= 0.5 → 進位
    if decimal >= max_decimal:
        decimal = 0
        integral += 1
        if exponent and integral >= 10:
            exponent += 1
            integral = 1
    while decimal % 10 == 0 and decimal_places > 0:   # 去尾零
        decimal //= 10
        decimal_places -= 1
    text = f"{sign}{integral}"
    if decimal_places:
        text += f".{decimal:0{decimal_places}d}"
    if exponent:
        text += f"e{exponent}"
    return text


class FirmwareFloat:
    """一個「已經決定好位元組」的浮點數：emit() 原樣寫出它的 text。
    數值本身仍可取用（`value`），給模擬器內部運算用。"""

    __slots__ = ("text", "value")

    def __init__(self, value, decimal_places=FLOAT_DECIMAL_PLACES):
        self.text = firmware_float_text(value, decimal_places)
        self.value = float(self.text) if self.text != "null" else None

    def __repr__(self):
        return f"FirmwareFloat({self.text})"


def _firmware_json_default(obj):
    if isinstance(obj, FirmwareFloat):
        return _FW_FLOAT_MARK + obj.text
    raise TypeError(f"not JSON serialisable: {obj!r}")


def dumps_like_firmware(obj):
    """json.dumps，但 FirmwareFloat 以韌體格式的裸字面值寫出。
    （先包成一個帶隨機前綴的字串，再把那整個字串換回字面值——字面值只會是
    數字／小數點／e／負號／null，不會有需要跳脫的字元。）"""
    line = json.dumps(obj, default=_firmware_json_default)
    return _FW_FLOAT_RE.sub(lambda m: m.group(1), line)


def read_float_param(params, key, fallback, lo, hi):
    raw, _ = read_number(params, key)
    value = fallback if raw is None else f32(raw)
    if math.isnan(value):
        value = fallback          # NaN 不可比較：退回預設（韌體 clampFloat 同樣不會留下 NaN）
    return clamp(value, lo, hi)


def read_led_channel(params, key):
    """整數 0–255 = 絕對值；浮點 0.0–1.0 = 比例（0.8 → 204）；浮點 > 1.0
    當絕對值四捨五入；缺漏/null = 0。與韌體 readLedChannel() 同規則。"""
    raw, is_int = read_number(params, key)
    if raw is None:
        return 0
    value = f32(raw)
    scaled = f32(value * 255.0) if (not is_int and 0.0 <= value <= 1.0) else value
    return int(clamp(round_to_int(scaled), 0, 255))


def stop_vibe(now):
    if not state["vibe_active"]:
        return
    state["vibe_active"] = False
    state["vibe_cmd_id"] = ""
    state["vibe_last_end_ms"] = now
    state["vibe_ever_ran"] = True


def stop_buzzer():
    state["buzz_active"] = False
    state["buzz_cmd_id"] = ""


def remember(cmd_id, nonce):
    state["seen_ids"] = (state["seen_ids"] + [cmd_id])[-DEDUPE_RING:]
    if nonce:  # 空 nonce 不記（與韌體 ringRemember 一致）
        state["seen_nonces"] = (state["seen_nonces"] + [nonce])[-DEDUPE_RING:]


def pair_locked(now):
    """鎖定期內？期滿即解鎖並重新計數（韌體 pairLocked()）。"""
    if not state["pair_locked"]:
        return False
    if now >= state["pair_locked_until_ms"]:
        state["pair_locked"] = False
        state["pair_failures"] = 0
        return False
    return True


def build_state():
    # 欄位集合與順序 = 韌體 buildState()（三方對齊測試會比對這裡）。
    return {
        "type": "state",
        "deviceId": args.device_id,
        "facts": {
            "button": sensors["button"],
            "distanceMm": sensors["distanceMm"],
            "lux": sensors["lux"],
            "tempC": sensors["tempC"],
            "vibeActive": state["vibe_active"],
            "buzzActive": state["buzz_active"],
            "servoAngle": state["servo_angle"],
            "led": state["led"],
        },
    }


def push_state_now(now):
    """按鈕邊緣：立即推播（僅已配對，與韌體 pushStateToPairedLinks 一致）。"""
    if state["paired"]:
        emit(build_state())
        state["last_state_push_ms"] = now


def apply_cmd(cid, name, params, now):
    """回傳 True＝真的套用了（可記入去重環）；False＝已回 err，不記。"""
    if name == "led.set":
        # 三個通道全部先解析成功才套用（部分失敗不留下半套效果）。
        applied = {k: read_led_channel(params, k) for k in ("r", "g", "b")}
        state["led"] = applied
        emit({"type": "ack", "id": cid, "applied": applied})
        return True

    if name == "vibe.pulse":
        # 韌體節流：進行中，或距上次結束 < 500ms → rate-limited。
        # （順序與韌體一致：節流檢查在參數解析之前。）
        if state["vibe_active"] or (
            state["vibe_ever_ran"] and (now - state["vibe_last_end_ms"]) < VIBE_MIN_GAP_MS
        ):
            emit({"type": "err", "id": cid, "reason": "rate-limited"})
            return False
        strength = read_float_param(params, "strength", 0.0, 0.0, VIBE_MAX_STRENGTH)
        duration = read_int_param(params, "durationMs", 200, 1, VIBE_MAX_DURATION_MS)
        state["vibe_active"] = True
        state["vibe_cmd_id"] = cid
        state["vibe_end_ms"] = now + duration
        emit({
            "type": "ack", "id": cid,
            # 韌體的 applied.strength 是 C float：位元組要一模一樣（0.7 而不是
            # Python 的 0.699999988079071）——見 firmware_float_text()。
            "applied": {"strength": FirmwareFloat(strength), "durationMs": duration},
        })
        return True

    if name == "buzzer.beep":
        freq = read_int_param(params, "freqHz", 1000, BUZZ_MIN_FREQ_HZ, BUZZ_MAX_FREQ_HZ)
        duration = read_int_param(params, "durationMs", 200, 1, BUZZ_MAX_DURATION_MS)
        stop_buzzer()  # 新指令覆蓋舊 beep（與韌體相同）
        state["buzz_active"] = True
        state["buzz_cmd_id"] = cid
        state["buzz_end_ms"] = now + duration
        emit({
            "type": "ack", "id": cid,
            "applied": {"freqHz": freq, "durationMs": duration},
        })
        return True

    if name == "servo.move":
        if state["servo_ever_moved"] and (now - state["servo_last_move_ms"]) < SERVO_MIN_GAP_MS:
            emit({"type": "err", "id": cid, "reason": "rate-limited"})
            return False
        angle = read_int_param(params, "angle", 90, SERVO_MIN_ANGLE, SERVO_MAX_ANGLE)
        state["servo_angle"] = angle
        state["servo_last_move_ms"] = now
        state["servo_ever_moved"] = True
        emit({"type": "ack", "id": cid, "applied": {"angle": angle}})
        return True

    emit({"type": "err", "id": cid, "reason": "unknown-cmd"})
    return False


def handle_pair(msg, now):
    if not args.pairing_code:
        # 配對停用：韌體同樣誠實回 pair-ok（hello 的 pairing 已是 false）。
        state["paired"] = True
        emit({"type": "pair-ok"})
        return
    # 暴力猜測防護：鎖定期間不比對碼（也不延長鎖定），誠實回 pair-locked。
    if pair_locked(now):
        emit({
            "type": "pair-fail", "reason": "pair-locked",
            "retryAfterMs": state["pair_locked_until_ms"] - now,
        })
        return
    if msg.get("code") == args.pairing_code:
        state["paired"] = True
        state["pair_failures"] = 0
        emit({"type": "pair-ok"})
        return
    state["pair_failures"] += 1
    if state["pair_failures"] >= PAIR_MAX_FAILURES:
        # 第 N 次錯碼：這一則就開始鎖定，回覆一併說明（韌體同規則）。
        state["pair_locked"] = True
        state["pair_locked_until_ms"] = now + args.pair_lockout_ms
        state["pair_failures"] = 0
        emit({"type": "pair-fail", "reason": "pair-locked",
              "retryAfterMs": args.pair_lockout_ms})
        return
    emit({"type": "pair-fail"})


def handle(msg):
    now = now_ms()
    # 韌體順序：先看 type（缺漏/空 → unknown-type），才看配對。
    # 非物件的合法 JSON（例如 `[1,2]`）在韌體是 doc["type"] → null → unknown-type，
    # 這裡也一樣（以前會 AttributeError 讓模擬器整個死掉）。
    t = msg.get("type") if isinstance(msg, dict) else None
    if not t:
        emit({"type": "err", "reason": "unknown-type"})
        return
    if t == "who":
        emit({
            "type": "hello", "deviceId": args.device_id, "fw": "sim-1.0",
            "proto": 1,
            # 線協定 v1.2：只有模擬器宣告 aip.frag/1（它真的有重組緩衝）。
            # 參考韌體不宣告——真板沒有那塊記憶體，替它宣稱就是說謊。
            "caps": HELLO_CAPS,
            "pairing": args.pairing_code != "" and not state["paired"],
            "pairingLocked": args.pairing_code != "" and pair_locked(now),
        })
    elif t == "pair":
        handle_pair(msg, now)
    elif t == "stop-all":
        # 緊急停止不要求配對（fail-safe：只會把效果關掉）。
        stop_vibe(now)
        stop_buzzer()
        state["led"] = {"r": 0, "g": 0, "b": 0}
        emit({"type": "ack", "stopAll": True})
    elif not state["paired"]:
        # 配對前，who/pair/stop-all 以外一律 not-paired（含未知 type），
        # 順序與韌體 handleMessage 完全相同。
        err = {"type": "err", "reason": "not-paired"}
        if msg.get("id"):
            err = {"type": "err", "id": msg["id"], "reason": "not-paired"}
        emit(err)
    elif t == "aip":
        # 線協定 v1.1：這份參考裝置不參與角色 session——明確忽略，不回 err
        # （落到 unknown-type 會讓「不支援」長得像「壞掉」）。與韌體一致。
        note("ignored an inbound aip line (this reference device does not join sessions)")
    elif t == "aip-frag":
        # 線協定 v1.2：重組（規則與 host 相同）。組好之後與 `aip` 一樣忽略。
        handle_aip_frag(msg, now)
    elif t == "cmd":
        cid = msg.get("id", "")
        if not cid:
            emit({"type": "err", "reason": "bad-params"})
            return
        # 重放/重複：同 id 或同 nonce 都回 dup ack，不重放效果（韌體同規則）。
        nonce = msg.get("nonce") or ""
        if cid in state["seen_ids"] or (nonce and nonce in state["seen_nonces"]):
            emit({"type": "ack", "id": cid, "dup": True})
            return
        try:
            applied = apply_cmd(cid, msg.get("name"), msg.get("params", {}) or {}, now)
        except BadParam:
            # 非數值參數：與韌體一致回 err bad-params（不再讓模擬器整個崩潰）。
            emit({"type": "err", "id": cid, "reason": "bad-params"})
            return
        if applied:
            remember(cid, nonce)
    elif t == "cancel":
        cid = msg.get("id", "")
        if not cid:
            emit({"type": "err", "reason": "bad-params"})   # 韌體同樣要求 id
            return
        cancelled = False
        if state["vibe_active"] and state["vibe_cmd_id"] == cid:
            stop_vibe(now)
            cancelled = True
        if state["buzz_active"] and state["buzz_cmd_id"] == cid:
            stop_buzzer()
            cancelled = True
        if cancelled:
            emit({"type": "ack", "id": cid, "cancelled": True})
        else:
            emit({"type": "err", "id": cid, "reason": "not-found"})
    elif t == "read":
        emit(build_state())
    else:
        emit({"type": "err", "reason": "unknown-type"})   # 韌體同樣回這個


# --- 感測面控制通道 ---------------------------------------------------------
_facts_last_text = None


def _valid_fact(key, value):
    """型別守門：控制檔寫錯型別不得讓模擬器回報韌體不可能回報的值。"""
    if key == "button":
        return isinstance(value, bool)
    if key in ("distanceMm", "lux"):
        return isinstance(value, int) and not isinstance(value, bool)
    if key == "tempC":
        return value is None or (isinstance(value, (int, float)) and not isinstance(value, bool))
    return False


def poll_facts_file(now):
    """--facts-file 內容變了就套用；按鈕因此翻轉時視同按鈕邊緣（立即推播）。"""
    global _facts_last_text
    if not args.facts_file:
        return
    try:
        with open(args.facts_file, "r", encoding="utf-8") as f:
            text = f.read()
    except OSError:
        return
    if text == _facts_last_text:
        return
    _facts_last_text = text
    try:
        obj = json.loads(text)
    except Exception as exc:
        note(f"facts-file ignored (bad json: {exc})")
        return
    if not isinstance(obj, dict):
        note("facts-file ignored (not an object)")
        return
    button_before = sensors["button"]
    for key, value in obj.items():
        if key not in sensors:
            note(f"facts-file key ignored: {key}")
            continue
        if not _valid_fact(key, value):
            note(f"facts-file value ignored for {key}: {value!r}")
            continue
        sensors[key] = value
    note(f"facts-file applied: {json.dumps(sensors)}")
    if sensors["button"] != button_before:
        push_state_now(now)


def poll_button_toggle(now):
    global pending_button_toggle
    if not pending_button_toggle:
        return
    pending_button_toggle = False
    sensors["button"] = not sensors["button"]
    note(f"button toggled by SIGUSR1: {sensors['button']}")
    push_state_now(now)


# --- 線協定 v1.2 的分片（切片＋重組；規則對照 host 的 fragment.rs）---------

HELLO_CAPS = ["led.set", "buzzer.beep", "vibe.pulse", "servo.move", "sensors.read"]
if not args.no_frag:
    HELLO_CAPS = HELLO_CAPS + [FRAG_CAP]


def frag_overhead():
    """一片表頭在**最壞情況**下佔幾個 byte（所有數字都取最大位數）。

    與 host 同一個做法：`total` 要切完才知道，而切多少又取決於 `total` 的
    位數；用最壞值一次算完，切出來的每一行保證 ≤ 上限。"""
    return len(dumps_like_firmware({
        "type": "aip-frag", "xfer": 4294967295, "seq": 65535,
        "total": 65535, "bytes": 4294967295, "crc": "00000000", "data": "",
    }))


def escaped_len(ch):
    """一個字元寫進 JSON 字串後佔幾個 byte（json.dumps 預設 ensure_ascii）。"""
    if ch in '"\\':
        return 2
    if ch in "\b\f\n\r\t":
        return 2
    code = ord(ch)
    if code < 0x20:
        return 6
    if code < 0x80:
        return 1
    # ensure_ascii：BMP 之外要用一對代理字（\uXXXX\uXXXX）。
    return 12 if code > 0xFFFF else 6


def fragment_envelope_line(text, limit, xfer):
    """把一份 envelope 的 JSON 文字切成 aip-frag 訊息（切點只在字元邊界）。

    回 None ＝切不出來（超過 8 KiB、片數超過上限、或上限小到放不下表頭）；
    切不出來就不送——不製造注定被丟棄的位元組。"""
    raw = text.encode("utf-8")
    if len(raw) > MAX_REASSEMBLED_BYTES:
        return None
    overhead = frag_overhead()
    if limit <= overhead:
        return None
    budget = limit - overhead
    crc = "%08x" % (zlib.crc32(raw) & 0xFFFFFFFF)
    chunks = []
    current = []
    used = 0
    for ch in text:
        cost = escaped_len(ch)
        if cost > budget:
            return None
        if used + cost > budget and current:
            chunks.append("".join(current))
            current = []
            used = 0
        current.append(ch)
        used += cost
    if current:
        chunks.append("".join(current))
    if not chunks:
        chunks = [""]
    if len(chunks) > MAX_FRAGMENTS:
        return None
    total = len(chunks)
    return [{
        "type": "aip-frag", "xfer": xfer, "seq": i, "total": total,
        "bytes": len(raw), "crc": crc, "data": data,
    } for i, data in enumerate(chunks)]


def drop_aip_rx(reason):
    """整筆丟棄並留痕（靜默丟棄＝把「組不回來」講成「主機沒說話」）。"""
    rx = state["aip_rx"]
    if rx is None:
        return
    state["aip_rx"] = None
    note("aip fragment dropped: reason=%s xfer=%s received=%d total=%d"
         % (reason, rx["xfer"], rx["next_seq"], rx["total"]))


def expire_aip_rx(now):
    rx = state["aip_rx"]
    if rx is not None and now - rx["last_ms"] >= FRAGMENT_TIMEOUT_MS:
        drop_aip_rx("timeout")


def handle_aip_frag(msg, now):
    """一片入站分片。規則與 host 的 Reassembler 相同（有界、整筆丟棄）。"""
    if args.no_frag:
        # 沒有宣告 aip.frag/1 的裝置不該收到分片：忽略並留痕（不回 err——
        # 舊韌體對未知型別的處置是丟棄，這裡保持同樣的方向）。
        note("ignored an aip-frag line (this device did not advertise %s)" % FRAG_CAP)
        return
    expire_aip_rx(now)
    xfer = msg.get("xfer")
    seq = msg.get("seq")
    total = msg.get("total")
    bytes_total = msg.get("bytes")
    crc = msg.get("crc")
    data = msg.get("data")
    if not isinstance(xfer, int) or not isinstance(seq, int) \
            or not isinstance(total, int) or not isinstance(bytes_total, int) \
            or not isinstance(crc, str) or not isinstance(data, str):
        drop_aip_rx("bad-header")
        note("aip fragment dropped: reason=bad-header")
        return
    rx = state["aip_rx"]
    if rx is not None and rx["xfer"] != xfer:
        drop_aip_rx("superseded")
        rx = None
    if rx is None:
        if seq != 0:
            note("aip fragment dropped: reason=unknown-xfer xfer=%s" % xfer)
            return
        if total <= 0 or total > MAX_FRAGMENTS:
            note("aip fragment dropped: reason=bad-total xfer=%s" % xfer)
            return
        if bytes_total <= 0 or bytes_total > MAX_REASSEMBLED_BYTES:
            note("aip fragment dropped: reason=bad-bytes xfer=%s" % xfer)
            return
        if len(crc) != 8 or any(c not in "0123456789abcdefABCDEF" for c in crc):
            note("aip fragment dropped: reason=bad-crc xfer=%s" % xfer)
            return
        rx = {"xfer": xfer, "total": total, "bytes": bytes_total,
              "crc": crc.lower(), "next_seq": 0, "buf": "", "last_ms": now}
        state["aip_rx"] = rx
    if total != rx["total"] or bytes_total != rx["bytes"] or crc.lower() != rx["crc"]:
        drop_aip_rx("header-mismatch")
        return
    if seq != rx["next_seq"]:
        drop_aip_rx("out-of-order")
        return
    if len((rx["buf"] + data).encode("utf-8")) > rx["bytes"]:
        drop_aip_rx("over-declared-bytes")
        return
    rx["buf"] += data
    rx["next_seq"] += 1
    rx["last_ms"] = now
    if rx["next_seq"] < rx["total"]:
        return
    state["aip_rx"] = None
    raw = rx["buf"].encode("utf-8")
    if len(raw) != rx["bytes"]:
        note("aip fragment dropped: reason=truncated xfer=%s" % xfer)
        return
    if ("%08x" % (zlib.crc32(raw) & 0xFFFFFFFF)) != rx["crc"]:
        note("aip fragment dropped: reason=crc-mismatch xfer=%s" % xfer)
        return
    try:
        json.loads(rx["buf"])
    except Exception:
        note("aip fragment dropped: reason=bad-json xfer=%s" % xfer)
        return
    # `>+` ＝從分片重組出來的一則完整 envelope（線上從來沒有出現過這一行；
    # 用不同的前綴，才不會被誤讀成「裝置真的收到了這麼長的一行」）。
    log.write('>+ {"type":"aip","envelope":%s}\n' % rx["buf"])
    log.flush()
    note("aip reassembled: xfer=%s bytes=%d fragments=%d" % (xfer, rx["bytes"], rx["total"]))


# --- AIP 控制通道（stdin）---------------------------------------------------

def _aip_envelope(message_type, name, prefix):
    """一則最小合規 envelope（欄位與 fake_iphone.rs 的 base_envelope 相同）。"""
    state["aip_seq"] += 1
    return {
        "specVersion": "aip/1.0",
        "messageId": f"sim-{prefix}-{state['aip_seq']}",
        "messageType": message_type,
        "name": name,
        # 宣稱身分＝這台裝置自己的 deviceId。host 端會拿它跟 spec 的
        # expectedDeviceId 比對——宣稱不是身分，比對才是。
        "source": {"kind": "device", "id": args.device_id},
        "sessionId": "session.home",
        "occurredAt": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "payload": {},
    }


def handle_aip_op(op):
    """一則控制指令 → 送出一則 aip 行（未配對一律拒絕）。"""
    name = op.get("op")
    if not state["paired"]:
        note(f"aip op refused: {name} (this channel is not paired)")
        return
    if name == "aip-capability":
        envelope = _aip_envelope("capability", "character.session.capability", "cap")
        envelope["payload"] = {
            "specVersions": ["aip/1.0"],
            "role": "remote-renderer",
            "profiles": ["character-session"],
            "syncClasses": ["semantic"],
            "intents": ["react-happily-to-touch", "celebrate", "idle"],
            "inputs": ["character.interaction.touch"],
            "features": {"haptic": True, "reducedMotion": False},
            "limits": {"maxMessageBytes": 8192},
        }
    elif name == "aip-touch":
        envelope = _aip_envelope("event", "character.interaction.touch", "touch")
        if isinstance(op.get("messageId"), str):
            envelope["messageId"] = op["messageId"]
        # 偽造身分測試：呼叫端可以覆寫 source（host 必須拒絕）。
        if isinstance(op.get("source"), dict):
            envelope["source"] = op["source"]
        ttl = op.get("expiresInMs", 5000)
        envelope["expiresAt"] = time.strftime(
            "%Y-%m-%dT%H:%M:%SZ", time.gmtime(time.time() + ttl / 1000.0)
        )
        envelope["payload"] = {"kind": op.get("kind", "tap")}
    elif name == "aip-resume":
        envelope = _aip_envelope("query", "character.session.resume", "resume")
        envelope["target"] = {"kind": "session", "id": "session.home"}
        envelope["payload"] = {
            "lastRevision": op.get("lastRevision", 0),
            "lastSequence": op.get("lastSequence", 0),
            "sessionEpoch": op.get("epoch", 0),
        }
    elif name == "aip-raw":
        envelope = op.get("envelope", {})
    elif name == "aip-partial":
        # 只送第一片：host 端會把它收進重組緩衝，然後停在半路。用來驗
        # 「取消（hello／stop-all／revoke）與逾時都留得下痕跡」——一筆停在
        # 半路的傳輸靜默消失，會讓「裝置說了話但我們組不回來」與「裝置沒說話」
        # 長得一模一樣。
        pad = int(op.get("bytes", 1200))
        pad = max(0, min(pad, MAX_REASSEMBLED_BYTES - 400))
        partial = _aip_envelope("event", "character.interaction.touch", "partial")
        partial["payload"] = {"kind": "tap", "pad": "p" * pad}
        text = dumps_like_firmware(partial)
        state["aip_xfer"] += 1
        frames = fragment_envelope_line(text, MAX_LINE_BYTES, state["aip_xfer"])
        if frames is None or len(frames) < 2:
            note("aip-partial needs an envelope that splits into at least two fragments")
            return
        emit(frames[0])
        note("aip-partial: sent fragment 1/%d of xfer=%s (the rest is never sent)"
             % (len(frames), state["aip_xfer"]))
        return
    else:
        note(f"unknown control op: {name}")
        return
    emit_aip(envelope)


def emit_aip(envelope):
    """送一則 aip envelope；放不進單行上限時切片（`--no-frag` 則誠實不送）。"""
    line = dumps_like_firmware({"type": "aip", "envelope": envelope})
    if len(line.encode("utf-8")) <= MAX_LINE_BYTES:
        emit({"type": "aip", "envelope": envelope})
        return
    if args.no_frag:
        note("aip op refused: envelope is %d bytes, over the %d byte line limit and this device "
             "did not advertise %s" % (len(line.encode("utf-8")), MAX_LINE_BYTES, FRAG_CAP))
        return
    text = dumps_like_firmware(envelope)
    state["aip_xfer"] += 1
    frames = fragment_envelope_line(text, MAX_LINE_BYTES, state["aip_xfer"])
    if frames is None:
        note("aip op refused: envelope cannot be fragmented (over 8 KiB or too many fragments)")
        return
    for frame in frames:
        emit(frame)


def tick():
    """效果到期＋感測控制通道＋定期 state 推播（韌體 loop() 的等價物）。"""
    now = now_ms()
    if state["vibe_active"] and now >= state["vibe_end_ms"]:
        stop_vibe(now)
    if state["buzz_active"] and now >= state["buzz_end_ms"]:
        stop_buzzer()
    poll_button_toggle(now)
    poll_facts_file(now)
    # 停在半路的入站分片不得無限期佔著緩衝（有界；與 host 同一個窗）。
    expire_aip_rx(now)
    if state["paired"] and (now - state["last_state_push_ms"]) >= STATE_PERIOD_MS:
        state["last_state_push_ms"] = now
        emit(build_state())


buf = b""
control_buf = b""
control_fd = sys.stdin.fileno()
sys.stderr.write(f"esp32-serial-sim: SIMULATOR on {slave_path}\n")
state["last_state_push_ms"] = now_ms()
while True:
    watch = [master] + ([control_fd] if control_fd is not None else [])
    try:
        ready, _, _ = select.select(watch, [], [], 0.1)
    except (OSError, ValueError):
        break
    if control_fd is not None and control_fd in ready:
        try:
            chunk = os.read(control_fd, 4096)
        except OSError:
            chunk = b""
        if not chunk:
            # EOF：不再監看，否則 select 會一直回 ready 而空轉（有界）。
            control_fd = None
        else:
            control_buf += chunk
            while b"\n" in control_buf:
                line, control_buf = control_buf.split(b"\n", 1)
                text = line.decode("utf-8", "replace").strip()
                if not text:
                    continue
                try:
                    op = json.loads(text)
                except Exception:
                    note(f"control op is not json: {text[:120]}")
                    continue
                if not isinstance(op, dict):
                    note("control op is not an object")
                    continue
                try:
                    handle_aip_op(op)
                except Exception as exc:
                    # 控制通道出錯不得讓模擬器整個消失（那會被誤診成拔線）。
                    note(f"handle_aip_op() raised {type(exc).__name__}: {exc}")
            # 控制通道的緩衝有界：沒有換行的超長輸入直接丟棄。
            if len(control_buf) > 64 * 1024:
                note("control buffer overflow; dropped")
                control_buf = b""
    if master in ready:
        try:
            chunk = os.read(master, 1024)
        except OSError:
            chunk = b""
        if chunk:
            buf += chunk
            while b"\n" in buf:
                line, buf = buf.split(b"\n", 1)
                if len(line) > MAX_LINE_BYTES:
                    # 韌體 pollSerial()：第 640 個位元組起整行丟棄，換行時回一次
                    # 無 id 的 bad-json——超長 cmd 在模擬器上不得「照常成功」。
                    log.write(f">> <{len(line)} bytes dropped: over {MAX_LINE_BYTES}>\n")
                    log.flush()
                    emit({"type": "err", "reason": "bad-json"})
                    continue
                text = line.decode("utf-8", "replace")
                if not text.strip():
                    continue            # 空行忽略（韌體 handleMessage 同）
                log.write(f">> {text}\n")
                log.flush()
                try:
                    # parse_constant：ArduinoJson 不接受 NaN／Infinity 字面值，
                    # Python 的 json 預設會接受——這裡拒掉，兩端才一致。
                    msg = json.loads(text, parse_constant=_reject_constant)
                except Exception:
                    emit({"type": "err", "reason": "bad-json"})   # 韌體同樣回這個
                    continue
                try:
                    handle(msg)
                except Exception as exc:
                    # 最後一道防線：模擬器絕不能以「整個程序消失」當作協定
                    # 回覆。裝置憑空不見時 host 只會看到 ack/read 逾時，
                    # 把參數問題誤診成傳輸問題。誠實回一則 err 並記進 log。
                    note(f"handle() raised {type(exc).__name__}: {exc}")
                    err = {"type": "err", "reason": "bad-params"}
                    if isinstance(msg, dict) and msg.get("id"):
                        err = {"type": "err", "id": msg["id"], "reason": "bad-params"}
                    emit(err)
    tick()
