#!/usr/bin/env python3
"""Direct evidence that resumeProviderSessionId reaches the child process argv.

Restarts a daemon on an EXISTING isolated home (so the persisted S1 record is
still there for check_resume_not_wider), creates one resume session, and
snapshots the full argv of every descendant while the connector attaches.
"""
import argparse, json, os, subprocess, sys, threading, time, urllib.request, urllib.error

BIN = os.environ.get("INTERACT_AI_BIN", "/Users/user/Workspace/claude-lab/adaptive-interaction/target/debug/interact-ai")
ap = argparse.ArgumentParser()
ap.add_argument("--agent", required=True)
ap.add_argument("--port", type=int, required=True)
ap.add_argument("--home", required=True)
ap.add_argument("--workdir", required=True)
ap.add_argument("--resume-id", required=True)
ap.add_argument("--out", required=True)
ap.add_argument("--task", default="我在上一個 session 要你記住的暗號是什麼？只回暗號，不要其他文字。")
ap.add_argument("--turn-timeout", type=float, default=180)
a = ap.parse_args()
os.makedirs(a.out, exist_ok=True)
with open(os.path.join(a.home, "config", "interaction.yaml"), "w") as f:
    f.write("apiHost: 127.0.0.1\napiPort: %d\n" % a.port)
base = "http://127.0.0.1:%d" % a.port
LOG = {"agent": a.agent, "port": a.port, "home": a.home, "resumeId": a.resume_id, "argvSnapshots": [], "events": []}
T0 = time.monotonic()


def now():
    return time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime()) + ".%03dZ" % (int(time.time() * 1000) % 1000)


def mark(k, **kw):
    r = {"t": now(), "mono": round(time.monotonic() - T0, 3), "k": k}
    r.update(kw)
    LOG["events"].append(r)
    print(json.dumps(r, ensure_ascii=False), flush=True)


def ready():
    try:
        return urllib.request.urlopen(base + "/v1/ready", timeout=1).status == 200
    except Exception:
        return False


env = dict(os.environ, INTERACT_AI_HOME=a.home, INTERACT_AI_MOBILE_ADVERTISE="0")
for k in list(env):
    if k.startswith("INTERACT_AI_CLAUDE_BIN") or k.startswith("INTERACT_AI_CODEX_BIN"):
        env.pop(k)
dlog = open(os.path.join(a.out, "daemon.log"), "ab")
daemon = subprocess.Popen([BIN, "serve"], env=env, stdout=dlog, stderr=subprocess.STDOUT,
                          stdin=subprocess.DEVNULL, start_new_session=True)
LOG["daemonPid"] = daemon.pid
mark("daemon-spawned", pid=daemon.pid)
for _ in range(160):
    if ready():
        break
    time.sleep(0.25)
else:
    mark("daemon-not-ready")
    sys.exit(2)
mark("daemon-ready")
tok = open(os.path.join(a.home, "state", "api-token")).read().strip()


def api(method, path, body=None, timeout=120):
    req = urllib.request.Request(base + path, method=method,
                                 headers={"Authorization": "Bearer " + tok, "content-type": "application/json"},
                                 data=(json.dumps(body).encode() if body is not None else None))
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, json.loads(r.read() or b"null")
    except urllib.error.HTTPError as e:
        raw = e.read()
        try:
            return e.code, json.loads(raw or b"null")
        except Exception:
            return e.code, {"_raw": raw.decode("utf-8", "replace")}


def snap_argv():
    ps = subprocess.run(["ps", "-axo", "pid=,ppid=,command="], capture_output=True, text=True).stdout.splitlines()
    rows = {}
    for line in ps:
        parts = line.split(None, 2)
        if len(parts) < 3:
            continue
        rows[int(parts[0])] = (int(parts[1]), parts[2])
    out = []
    for pid, (ppid, cmd) in rows.items():
        p, hops = ppid, 0
        while p > 1 and hops < 20:
            if p == daemon.pid:
                out.append({"pid": pid, "ppid": ppid, "cmd": cmd})
                break
            nxt = rows.get(p)
            if not nxt:
                break
            p = nxt[0]
            hops += 1
    return out


stop = threading.Event()
seen = set()


def watcher():
    while not stop.is_set():
        for c in snap_argv():
            key = (c["pid"], c["cmd"])
            if key not in seen:
                seen.add(key)
                LOG["argvSnapshots"].append({"t": now(), **c})
                print("ARGV", json.dumps(c, ensure_ascii=False)[:800], flush=True)
        time.sleep(0.05)


threading.Thread(target=watcher, daemon=True).start()

s, listed = api("GET", "/v1/agent-sessions")
LOG["sessionsAfterRestart"] = listed
payload = {"agentId": a.agent, "label": "K-04 argv probe", "workdir": a.workdir, "ttlMinutes": 20,
           "maxMessages": 40, "allowWrite": False, "toolScope": [], "consentScope": [], "dataScope": [],
           "resumeProviderSessionId": a.resume_id}
s, rec = api("POST", "/v1/agent-sessions", payload)
mark("create", status=s, sessionId=(rec or {}).get("sessionId"), body=(rec if s >= 400 else None))
LOG["create"] = {"status": s, "record": rec, "payload": payload}
if s < 400:
    sid = rec["sessionId"]
    s2, sent = api("POST", "/v1/agent-sessions/%s/messages" % sid, {"kind": "task", "body": {"task": a.task}})
    mark("task-sent", status=s2)
    t = time.monotonic()
    seen_states = []
    while time.monotonic() - t < a.turn_timeout:
        st, cur = api("GET", "/v1/agent-sessions/" + sid)
        state = (cur or {}).get("state")
        if not seen_states or seen_states[-1] != state:
            seen_states.append(state)
            mark("state", state=state)
        s3, msgs = api("GET", "/v1/agent-sessions/%s/messages?direction=from-session" % sid)
        if any(m.get("kind") == "result" for m in (msgs or [])):
            LOG["fromSession"] = msgs
            break
        time.sleep(0.5)
    LOG["states"] = seen_states
    s, LOG["finalRecord"] = api("GET", "/v1/agent-sessions/" + sid)
    s, c = api("POST", "/v1/agent-sessions/%s/close" % sid, {"reason": "argv probe done"})
    mark("close", status=s, state=(c or {}).get("state"))
time.sleep(1)
stop.set()
daemon.terminate()
try:
    daemon.wait(timeout=20)
except subprocess.TimeoutExpired:
    daemon.kill()
mark("daemon-stopped", returncode=daemon.returncode)
LOG["wallSeconds"] = round(time.monotonic() - T0, 2)
json.dump(LOG, open(os.path.join(a.out, "raw.json"), "w"), ensure_ascii=False, indent=1)
print("DONE")
