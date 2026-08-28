#!/usr/bin/env python3
"""ESP32 參考裝置【模擬器】—— serial(pty) 版。

明確標示：這是模擬器，不是真機。它與 firmware/esp32-companion 說同一套
線協定（hello/pair/cmd/ack/state/cancel/stop-all），並模擬韌體硬限制
（vibe strength clamp 0.8、durationMs clamp 3000）與 cmd id dedupe。

用法：esp32-serial-sim.py --device-id esp32-sim01 --pairing-code 9927 \
        --pty-path-file /tmp/sim-pty --log /tmp/sim.log
會建立一個 pty，把 slave 路徑寫進 --pty-path-file，然後在 master 端服務。
"""

import argparse
import json
import os
import pty
import sys
import termios
import time
import tty

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


def emit(obj):
    line = json.dumps(obj) + "\n"
    os.write(master, line.encode())
    log.write(f"<< {line}")
    log.flush()


state = {
    "paired": args.pairing_code == "",
    "seen_ids": [],
    "led": {"r": 0, "g": 0, "b": 0},
    "vibe_active": False,
}


def handle(msg):
    t = msg.get("type")
    if t == "who":
        emit({
            "type": "hello", "deviceId": args.device_id, "fw": "sim-1.0",
            "proto": 1, "caps": ["led.set", "vibe.pulse", "sensors.read"],
            "pairing": args.pairing_code != "",
        })
    elif t == "pair":
        if args.pairing_code and msg.get("code") == args.pairing_code:
            state["paired"] = True
            emit({"type": "pair-ok"})
        else:
            emit({"type": "pair-fail"})
    elif t in ("cmd", "read") and not state["paired"]:
        emit({"type": "err", "id": msg.get("id"), "reason": "not-paired"})
    elif t == "cmd":
        cid = msg.get("id", "")
        if cid in state["seen_ids"]:
            emit({"type": "ack", "id": cid, "dup": True})  # 冪等：不重放效果
            return
        state["seen_ids"] = (state["seen_ids"] + [cid])[-16:]
        name = msg.get("name")
        params = msg.get("params", {})
        if name == "led.set":
            applied = {k: max(0, min(255, int(params.get(k, 0) or 0))) for k in ("r", "g", "b")}
            state["led"] = applied
            emit({"type": "ack", "id": cid, "applied": applied})
        elif name == "vibe.pulse":
            # 模擬韌體硬限制。
            strength = min(float(params.get("strength", 0) or 0), 0.8)
            duration = min(int(params.get("durationMs", 0) or 0), 3000)
            state["vibe_active"] = True
            emit({"type": "ack", "id": cid, "applied": {"strength": strength, "durationMs": duration}})
        else:
            emit({"type": "err", "id": cid, "reason": "unknown-cmd"})
    elif t == "cancel":
        cid = msg.get("id", "")
        if state["vibe_active"]:
            state["vibe_active"] = False
            emit({"type": "ack", "id": cid, "cancelled": True})
        else:
            emit({"type": "err", "id": cid, "reason": "not-found"})
    elif t == "read":
        emit({
            "type": "state", "deviceId": args.device_id,
            "facts": {
                "button": False, "distanceMm": 842, "lux": 133,
                "tempC": 24.5, "vibeActive": state["vibe_active"],
                "led": state["led"],
            },
        })
    elif t == "stop-all":
        state["vibe_active"] = False
        state["led"] = {"r": 0, "g": 0, "b": 0}
        emit({"type": "ack", "stopAll": True})


buf = b""
sys.stderr.write(f"esp32-serial-sim: SIMULATOR on {slave_path}\n")
while True:
    try:
        chunk = os.read(master, 1024)
    except OSError:
        time.sleep(0.05)
        continue
    if not chunk:
        time.sleep(0.05)
        continue
    buf += chunk
    while b"\n" in buf:
        line, buf = buf.split(b"\n", 1)
        try:
            msg = json.loads(line.decode())
        except Exception:
            continue
        log.write(f">> {line.decode()}\n")
        log.flush()
        handle(msg)
