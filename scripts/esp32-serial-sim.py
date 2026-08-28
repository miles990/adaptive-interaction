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
  hello       : type/deviceId/fw/proto/caps/pairing 六個欄位、順序與型別
                與韌體 sendHello() 相同（fw 值刻意不同：這是模擬器）
  state       : 每 5 秒自動推播一次（僅在已配對時，不對未配對連線洩漏感測）

用法：esp32-serial-sim.py --device-id esp32-sim01 --pairing-code 9927 \
        --pty-path-file /tmp/sim-pty --log /tmp/sim.log
會建立一個 pty，把 slave 路徑寫進 --pty-path-file，然後在 master 端服務。
"""

import argparse
import json
import os
import pty
import select
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

parser = argparse.ArgumentParser()
parser.add_argument("--device-id", required=True)
parser.add_argument("--pairing-code", default="")
parser.add_argument("--pty-path-file", required=True)
parser.add_argument("--log", default="/dev/null")
args = parser.parse_args()

master, slave = pty.openpty()
# raw 模式：關掉 echo/canonical，host 端才能把 pty 當乾淨的位元組管道。
tty.setraw(master, when=termios.TCSANOW)
tty.setraw(slave, when=termios.TCSANOW)
slave_path = os.ttyname(slave)
with open(args.pty_path_file, "w") as f:
    f.write(slave_path)

log = open(args.log, "a")


def now_ms():
    return int(time.monotonic() * 1000)


def emit(obj):
    line = json.dumps(obj) + "\n"
    os.write(master, line.encode())
    log.write(f"<< {line}")
    log.flush()


state = {
    "paired": args.pairing_code == "",
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


def clamp(value, lo, hi):
    return max(lo, min(hi, value))


# --- 數值參數解析（逐位鏡射韌體的 readNumber/roundToLong/readLedChannel）---
class BadParam(Exception):
    """非數值參數 → err bad-params（韌體同樣回 err，兩端一致）。"""


def f32(v):
    """收斂成 IEEE754 單精度。韌體以 ArduinoJson 的 as<float>() 取值、之後
    全程用 float 運算；Python 預設是 double，取整前不鏡射精度的話，像
    {"r":0.3} 這種輸入會一端算 76、另一端算 77。"""
    return struct.unpack("<f", struct.pack("<f", v))[0]


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
    """韌體 roundToLong()：float 加 0.5 後往零截斷（單精度）。"""
    return int(f32(f32(v) + 0.5))


def read_int_param(params, key, fallback, lo, hi):
    raw, _ = read_number(params, key)
    value = fallback if raw is None else round_to_int(raw)
    return int(clamp(value, lo, hi))


def read_float_param(params, key, fallback, lo, hi):
    raw, _ = read_number(params, key)
    return clamp(fallback if raw is None else raw, lo, hi)


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


def build_state():
    return {
        "type": "state",
        "deviceId": args.device_id,
        "facts": {
            "button": False,
            "distanceMm": 842,
            "lux": 133,
            "tempC": 24.5,
            "vibeActive": state["vibe_active"],
            "buzzActive": state["buzz_active"],
            "servoAngle": state["servo_angle"],
            "led": state["led"],
        },
    }


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
        })
    elif t == "pair":
        if not args.pairing_code:
            # 配對停用：韌體同樣誠實回 pair-ok（hello 的 pairing 已是 false）。
            state["paired"] = True
            emit({"type": "pair-ok"})
        elif msg.get("code") == args.pairing_code:
            state["paired"] = True
            emit({"type": "pair-ok"})
        else:
            emit({"type": "pair-fail"})
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


def tick():
    """效果到期＋定期 state 推播（韌體 loop() 的等價物）。"""
    now = now_ms()
    if state["vibe_active"] and now >= state["vibe_end_ms"]:
        stop_vibe(now)
    if state["buzz_active"] and now >= state["buzz_end_ms"]:
        stop_buzzer()
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
                text = line.decode("utf-8", "replace")
                if not text.strip():
                    continue            # 空行忽略（韌體 handleMessage 同）
                log.write(f">> {text}\n")
                log.flush()
                try:
                    msg = json.loads(text)
                except Exception:
                    emit({"type": "err", "reason": "bad-json"})   # 韌體同樣回這個
                    continue
                handle(msg)
    tick()
