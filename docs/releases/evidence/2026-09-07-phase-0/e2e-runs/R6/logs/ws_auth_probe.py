#!/usr/bin/env python3
"""Ad-hoc WS auth probe — works around a fake_iphone.rs bug (die() on any
connect failure inside reconnect(), see crates/interaction-runtime/examples/
fake_iphone.rs:167,298) that makes the real fixture binary exit instead of
reporting reconnect-failed when the daemon is offline. This script performs
only the same `{"type":"auth",...}` handshake fake_iphone's reconnect() does,
using Python's websockets lib, so it can survive a refused connection and be
retried after the daemon comes back up. Written to the scratchpad only; does
not modify any repo file.
"""
import asyncio
import json
import ssl
import sys

import websockets


async def main():
    port = int(sys.argv[1])
    device_id = sys.argv[2]
    token = sys.argv[3]
    uri = f"wss://127.0.0.1:{port}/"
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    try:
        async with websockets.connect(uri, ssl=ctx, open_timeout=3, close_timeout=2) as ws:
            await ws.send(json.dumps({"type": "auth", "deviceId": device_id, "token": token}))
            reply = await asyncio.wait_for(ws.recv(), timeout=3)
            print(json.dumps({"outcome": "reply", "reply": json.loads(reply)}))
    except Exception as e:
        print(json.dumps({"outcome": "error", "error": f"{type(e).__name__}: {e}"}))


if __name__ == "__main__":
    asyncio.run(main())
