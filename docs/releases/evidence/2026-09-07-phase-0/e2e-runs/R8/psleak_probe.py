#!/usr/bin/env python3
"""Does a codex gateway session's task text end up in a process command line
(world-readable via ps)? Fresh isolated home, one read-only turn."""
import json, os, random, subprocess, sys, threading, time, urllib.request, urllib.error

BIN = "/Users/user/Workspace/claude-lab/adaptive-interaction/target/debug/interact-ai"
OUT = sys.argv[1]
PORT = int(sys.argv[2])
os.makedirs(OUT, exist_ok=True)
home = os.path.join(OUT, "home")
os.makedirs(os.path.join(home, "config"), exist_ok=True)
open(os.path.join(home, "config", "interaction.yaml"), "w").write("apiHost: 127.0.0.1\napiPort: %d\n" % PORT)
work = os.path.join(OUT, "workdir")
os.makedirs(work, exist_ok=True)
open(os.path.join(work, "NOTES.md"), "w").write("# ps leak probe\n")
MARK = "PSLEAK" + "".join(random.choice("ABCDEFGHJKLMNPQRSTUVWXYZ23456789") for _ in range(8))
base = "http://127.0.0.1:%d" % PORT
env = dict(os.environ, INTERACT_AI_HOME=home, INTERACT_AI_MOBILE_ADVERTISE="0")
for k in list(env):
    if k.startswith("INTERACT_AI_CLAUDE_BIN") or k.startswith("INTERACT_AI_CODEX_BIN"):
        env.pop(k)
dlog = open(os.path.join(OUT, "daemon.log"), "ab")
d = subprocess.Popen([BIN, "serve"], env=env, stdout=dlog, stderr=subprocess.STDOUT,
                     stdin=subprocess.DEVNULL, start_new_session=True)


def ready():
    try:
        return urllib.request.urlopen(base + "/v1/ready", timeout=1).status == 200
    except Exception:
        return False


for _ in range(160):
    if ready():
        break
    time.sleep(0.25)
else:
    print("daemon-not-ready")
    sys.exit(2)
tok = open(os.path.join(home, "state", "api-token")).read().strip()


def api(method, path, body=None):
    req = urllib.request.Request(base + path, method=method,
                                 headers={"Authorization": "Bearer " + tok, "content-type": "application/json"},
                                 data=(json.dumps(body).encode() if body is not None else None))
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            return r.status, json.loads(r.read() or b"null")
    except urllib.error.HTTPError as e:
        raw = e.read()
        try:
            return e.code, json.loads(raw or b"null")
        except Exception:
            return e.code, {"_raw": raw.decode("utf-8", "replace")}


hits = []
stop = threading.Event()


def scan():
    while not stop.is_set():
        ps = subprocess.run(["ps", "-axww", "-o", "pid=,ppid=,command="], capture_output=True, text=True).stdout
        for line in ps.splitlines():
            if MARK in line and line not in [h["line"] for h in hits]:
                hits.append({"t": time.strftime("%H:%M:%S"), "line": line[:1500]})
                print("PS-HIT", line[:300], flush=True)
        time.sleep(0.08)


threading.Thread(target=scan, daemon=True).start()
s, rec = api("POST", "/v1/agent-sessions",
             {"agentId": "codex", "label": "ps leak probe", "workdir": work, "ttlMinutes": 10,
              "maxMessages": 10, "allowWrite": False, "toolScope": [], "consentScope": [], "dataScope": []})
print("create", s, (rec or {}).get("sessionId"), flush=True)
if s < 400:
    sid = rec["sessionId"]
    task = "請只回覆四個字「收到暗號」。這一句裡的識別碼是 %s，不要使用任何工具。" % MARK
    s2, _ = api("POST", "/v1/agent-sessions/%s/messages" % sid, {"kind": "task", "body": {"task": task}})
    print("send", s2, flush=True)
    t = time.monotonic()
    while time.monotonic() - t < 120:
        st, cur = api("GET", "/v1/agent-sessions/" + sid)
        if (cur or {}).get("state") in ("claimed-completed", "failed", "timed-out", "cancelled", "closed"):
            print("state", (cur or {}).get("state"), flush=True)
            break
        time.sleep(0.5)
    s, msgs = api("GET", "/v1/agent-sessions/%s/messages?direction=from-session" % sid)
    print("from-session", json.dumps(msgs, ensure_ascii=False)[:400], flush=True)
    api("POST", "/v1/agent-sessions/%s/close" % sid, {"reason": "ps leak probe done"})
time.sleep(2)
stop.set()
d.terminate()
try:
    d.wait(timeout=20)
except subprocess.TimeoutExpired:
    d.kill()
json.dump({"marker": MARK, "psHits": hits}, open(os.path.join(OUT, "raw.json"), "w"), ensure_ascii=False, indent=1)
print("HITS", len(hits))
print("DONE")
