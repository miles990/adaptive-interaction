#!/usr/bin/env python3
"""E0-09 follow-up scenario: agent asks a clarifying question in turn 1 claim,
human answers 30s later via a second task message on the SAME session.
Records full state timeline + both turns' messages + daemon.log + ps children.
usage: followup_test.py --agent claude-code|codex --port N --out DIR
"""
import argparse, json, os, subprocess, sys, time, threading, urllib.request, urllib.error

ROOT = os.environ.get("INTERACT_AI_REPO", "/Users/user/Workspace/claude-lab/adaptive-interaction")
BIN = os.environ.get("INTERACT_AI_BIN", f"{ROOT}/target/debug/interact-ai")

ap = argparse.ArgumentParser()
ap.add_argument("--agent", required=True, choices=["claude-code", "codex"])
ap.add_argument("--port", type=int, required=True)
ap.add_argument("--out", required=True)
ap.add_argument("--turn1-timeout", type=float, default=90)
ap.add_argument("--turn2-timeout", type=float, default=90)
ap.add_argument("--answer-delay", type=float, default=30)
ap.add_argument("--keep-daemon", action="store_true")
a = ap.parse_args()

out = a.out
os.makedirs(out, exist_ok=True)
home = os.path.join(out, "home")
os.makedirs(os.path.join(home, "config"), exist_ok=True)
with open(os.path.join(home, "config", "interaction.yaml"), "w") as f:
    f.write(f"apiHost: 127.0.0.1\napiPort: {a.port}\n")
work = os.path.join(out, "workdir")
os.makedirs(work, exist_ok=True)
with open(os.path.join(work, "NOTES.md"), "w") as f:
    f.write("# E0-09 followup fixture\n\n這是隔離工作目錄的檔案，僅供讀取。\n")

base = f"http://127.0.0.1:{a.port}"
log = {"agent": a.agent, "port": a.port, "workdir": work, "events": []}

T0 = time.monotonic()


def now():
    return time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime()) + f".{int(time.time()*1000)%1000:03d}Z"


def mark(k, **kw):
    rec = {"t": now(), "mono": round(time.monotonic() - T0, 3), "k": k, **kw}
    log["events"].append(rec)
    print(json.dumps(rec, ensure_ascii=False), flush=True)


def ready():
    try:
        return urllib.request.urlopen(f"{base}/v1/ready", timeout=1).status == 200
    except Exception:
        return False


env = dict(os.environ, INTERACT_AI_HOME=home, INTERACT_AI_MOBILE_ADVERTISE="0")
for k in list(env):
    if k.startswith("INTERACT_AI_CLAUDE_BIN") or k.startswith("INTERACT_AI_CODEX_BIN"):
        env.pop(k)
dlog = open(os.path.join(out, "daemon.log"), "ab")
daemon = subprocess.Popen([BIN, "serve"], env=env, stdout=dlog, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, start_new_session=True)
mark("daemon-spawned", pid=daemon.pid)
for _ in range(120):
    if ready():
        break
    time.sleep(0.25)
else:
    mark("daemon-not-ready")
    sys.exit(2)
mark("daemon-ready")

tok = open(os.path.join(home, "state", "api-token")).read().strip()


def api(method, path, body=None):
    req = urllib.request.Request(
        base + path,
        method=method,
        headers={"Authorization": f"Bearer {tok}", "content-type": "application/json"},
        data=(json.dumps(body).encode() if body is not None else None),
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return r.status, json.loads(r.read() or b"null")
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.loads(e.read() or b"null")
        except Exception:
            return e.code, None


sse_path = os.path.join(out, "sse.jsonl")
stop_sse = threading.Event()


def sse():
    req = urllib.request.Request(base + "/v1/events", headers={"Authorization": f"Bearer {tok}", "accept": "text/event-stream"})
    try:
        with urllib.request.urlopen(req, timeout=600) as r, open(sse_path, "a") as f:
            ev = {}
            for raw in r:
                if stop_sse.is_set():
                    break
                line = raw.decode("utf-8", "replace").rstrip("\n")
                if line == "":
                    if ev:
                        ev["_t"] = now()
                        f.write(json.dumps(ev, ensure_ascii=False) + "\n")
                        f.flush()
                        ev = {}
                elif ":" in line:
                    k, v = line.split(":", 1)
                    ev.setdefault(k.strip(), "")
                    ev[k.strip()] += v.strip()
    except Exception as e:
        with open(sse_path, "a") as f:
            f.write(json.dumps({"_sse_error": str(e), "_t": now()}) + "\n")


threading.Thread(target=sse, daemon=True).start()
time.sleep(0.5)


def child_procs():
    return subprocess.run(
        ["bash", "-lc", "ps -axo pid,pgid,ppid,etime,stat,command | grep -E '(claude -p --input-format|codex app-server$)' | grep -v grep | cut -c1-200"],
        capture_output=True, text=True,
    ).stdout.splitlines()


s, disc = api("GET", "/v1/agents")
log["discovery"] = disc

task1 = "在回答之前，你必須先問我一個釐清問題：我想要的輸出語言是繁體中文還是英文？請只提出這個問題並等待我的回答，不要自行假設，也不要讀取或修改任何檔案。"
payload = {"agentId": a.agent, "label": "e0-09-followup", "workdir": work, "ttlMinutes": 20, "maxMessages": 40, "allowWrite": False, "toolScope": [], "consentScope": []}
s, rec = api("POST", "/v1/agent-sessions", payload)
mark("create", status=s, state=(rec or {}).get("state"), id=(rec or {}).get("sessionId"))
if s >= 400:
    log["createError"] = rec
    json.dump(log, open(os.path.join(out, "result.json"), "w"), ensure_ascii=False, indent=1)
    daemon.terminate()
    sys.exit(3)
sid = rec.get("sessionId") or rec.get("id")
log["sessionId"] = sid
log["createRecord"] = rec

s, sent1 = api("POST", f"/v1/agent-sessions/{sid}/messages", {"kind": "task", "body": {"task": task1}})
mark("turn1-task-sent", status=s)
log["turn1Send"] = sent1

terminal = {"claimed-completed", "failed", "timed-out", "cancelled", "expired", "closed", "unknown"}
last = None
t_send = time.monotonic()
while time.monotonic() - t_send < a.turn1_timeout:
    s, cur = api("GET", f"/v1/agent-sessions/{sid}")
    st = (cur or {}).get("state")
    if st != last:
        mark("turn1-state", state=st, providerSessionId=(cur or {}).get("providerSessionId"))
        log.setdefault("turn1Snapshots", []).append(cur)
        last = st
    if st in terminal:
        break
    time.sleep(0.5)
s, turn1_final = api("GET", f"/v1/agent-sessions/{sid}")
log["turn1Final"] = turn1_final
mark("turn1-final", state=(turn1_final or {}).get("state"), providerSessionId=(turn1_final or {}).get("providerSessionId"))

s, msgs_from_1 = api("GET", f"/v1/agent-sessions/{sid}/messages?direction=from-session")
log["turn1MessagesFromSession"] = msgs_from_1
mark("turn1-children", children=child_procs())

# --- simulate "human answers later": wait, then send follow-up task on SAME session ---
mark("waiting-before-answer", delaySeconds=a.answer_delay)
time.sleep(a.answer_delay)

task2 = "繁體中文。請繼續，並用一句話用繁體中文說明你收到了這個回答。"
claim_before_turn2 = (turn1_final or {}).get("claimId")

s, sent2 = api("POST", f"/v1/agent-sessions/{sid}/messages", {"kind": "task", "body": {"task": task2}})
mark("turn2-task-sent", status=s, resp=(sent2 if s >= 400 else None))
log["turn2Send"] = sent2
log["turn2SendStatus"] = s

# Session `state` does not flip synchronously on send (it only updates when
# the async GatewayEvent loop processes TaskAccepted/etc from the connector),
# so an immediate poll can still read turn 1's stale claimed-completed. Only
# treat the loop as done once we've observed EITHER a new claimId (a fresh
# claim landed) or a genuinely new non-terminal state (active/waiting-*),
# THEN settle back into a terminal state.
last = None
saw_fresh_activity = False
t_send2 = time.monotonic()
while time.monotonic() - t_send2 < a.turn2_timeout:
    s, cur = api("GET", f"/v1/agent-sessions/{sid}")
    st = (cur or {}).get("state")
    cid = (cur or {}).get("claimId")
    if st != last:
        mark("turn2-state", state=st, providerSessionId=(cur or {}).get("providerSessionId"), claimId=cid, staleClaim=(cid == claim_before_turn2))
        log.setdefault("turn2Snapshots", []).append(cur)
        last = st
    if not saw_fresh_activity:
        if st in ("active", "waiting-for-input", "waiting-for-consent"):
            saw_fresh_activity = True
        elif st in terminal and cid != claim_before_turn2:
            saw_fresh_activity = True
    if saw_fresh_activity and st in terminal and cid != claim_before_turn2:
        break
    time.sleep(0.5)
else:
    mark("turn2-timeout-no-fresh-claim", claimBeforeTurn2=claim_before_turn2)

s, turn2_final = api("GET", f"/v1/agent-sessions/{sid}")
log["turn2Final"] = turn2_final
mark("turn2-final", state=(turn2_final or {}).get("state"), providerSessionId=(turn2_final or {}).get("providerSessionId"))

s, msgs_from_2 = api("GET", f"/v1/agent-sessions/{sid}/messages?direction=from-session")
log["turn2MessagesFromSession"] = msgs_from_2
s, msgs_to = api("GET", f"/v1/agent-sessions/{sid}/messages?direction=to-session")
log["messagesToSession"] = msgs_to
mark("turn2-children", children=child_procs())

s, status = api("GET", "/v1/status")
log["status"] = status
s, aud = api("GET", "/v1/audit")
log["auditTail"] = aud

ps = subprocess.run(["bash", "-lc", "ps -axo pid,pgid,ppid,etime,command | grep -E '(claude|codex)' | grep -v grep | grep -v agmsg | cut -c1-220"], capture_output=True, text=True).stdout
log["psAgents"] = ps.splitlines()

if not a.keep_daemon:
    daemon.terminate()
    try:
        daemon.wait(timeout=15)
    except subprocess.TimeoutExpired:
        daemon.kill()
    mark("daemon-stopped")
else:
    log["daemonPid"] = daemon.pid
    mark("daemon-kept", pid=daemon.pid)

stop_sse.set()
log["finishedAt"] = now()
log["wallSeconds"] = round(time.monotonic() - T0, 2)
json.dump(log, open(os.path.join(out, "result.json"), "w"), ensure_ascii=False, indent=1)
print("RESULT", json.dumps({"agent": a.agent, "turn1": (turn1_final or {}).get("state"), "turn2": (turn2_final or {}).get("state"), "sid": sid, "wall": log["wallSeconds"]}, ensure_ascii=False))
