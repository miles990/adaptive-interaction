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
import select
import signal
import struct
import sys
import termios
import time
import tty

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
    line = json.dumps(obj) + "\n"
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
            "applied": {"strength": strength, "durationMs": duration},
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
            "caps": ["led.set", "buzzer.beep", "vibe.pulse", "servo.move", "sensors.read"],
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


def tick():
    """效果到期＋感測控制通道＋定期 state 推播（韌體 loop() 的等價物）。"""
    now = now_ms()
    if state["vibe_active"] and now >= state["vibe_end_ms"]:
        stop_vibe(now)
    if state["buzz_active"] and now >= state["buzz_end_ms"]:
        stop_buzzer()
    poll_button_toggle(now)
    poll_facts_file(now)
    if state["paired"] and (now - state["last_state_push_ms"]) >= STATE_PERIOD_MS:
        state["last_state_push_ms"] = now
        emit(build_state())


buf = b""
sys.stderr.write(f"esp32-serial-sim: SIMULATOR on {slave_path}\n")
state["last_state_push_ms"] = now_ms()
while True:
    try:
        ready, _, _ = select.select([master], [], [], 0.1)
    except (OSError, ValueError):
        break
    if ready:
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
