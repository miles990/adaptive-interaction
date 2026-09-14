#!/usr/bin/env python3
"""K-04 / B-08: native session continuation + authorization retention.

Sequence per agent (real agent, isolated INTERACT_AI_HOME, read-only workdir):
  S1 turn1 (memorise secret) -> S1 turn2 (recall secret, same session)
  -> close S1 -> POST message expect 409
  -> S2 with resumeProviderSessionId=S1.providerSessionId (recall secret)
  -> resume-not-wider probes (allowWrite / different workdir / omitted workdir / wider ttl / wider maxMessages)
  -> resume with unknown provider session id
Never writes to ~/.adaptive-interaction, ~/.claude, ~/.codex (read-only evidence only).
"""
import argparse, json, os, random, string, subprocess, sys, threading, time
import urllib.request, urllib.error

BIN = os.environ.get("INTERACT_AI_BIN", "/Users/user/Workspace/claude-lab/adaptive-interaction/target/debug/interact-ai")

ap = argparse.ArgumentParser()
ap.add_argument("--agent", required=True)
ap.add_argument("--port", type=int, required=True)
ap.add_argument("--out", required=True)
ap.add_argument("--turn-timeout", type=float, default=180)
ap.add_argument("--approve", default="none", choices=["none", "approve", "deny"])
a = ap.parse_args()

out = a.out
os.makedirs(out, exist_ok=True)
home = os.path.join(out, "home")
os.makedirs(os.path.join(home, "config"), exist_ok=True)
with open(os.path.join(home, "config", "interaction.yaml"), "w") as f:
    f.write("apiHost: 127.0.0.1\napiPort: %d\n" % a.port)
work = os.path.join(out, "workdir")
os.makedirs(work, exist_ok=True)
with open(os.path.join(work, "NOTES.md"), "w") as f:
    f.write("# K-04 續接測試\n\n這是隔離的唯讀工作目錄。\n")
work_other = os.path.join(out, "workdir-other")
os.makedirs(work_other, exist_ok=True)
with open(os.path.join(work_other, "NOTES.md"), "w") as f:
    f.write("# K-04 另一個資料夾\n")

SECRET = "".join(random.choice("ABCDEFGHJKLMNPQRSTUVWXYZ23456789") for _ in range(8))
base = "http://127.0.0.1:%d" % a.port
LOG = {"agent": a.agent, "port": a.port, "secret": SECRET, "workdir": work,
       "workdirOther": work_other, "events": [], "cases": {}}
T0 = time.monotonic()


def now():
    return time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime()) + ".%03dZ" % (int(time.time() * 1000) % 1000)


def mark(k, **kw):
    rec = {"t": now(), "mono": round(time.monotonic() - T0, 3), "k": k}
    rec.update(kw)
    LOG["events"].append(rec)
    print(json.dumps(rec, ensure_ascii=False), flush=True)


def ready():
    try:
        return urllib.request.urlopen(base + "/v1/ready", timeout=1).status == 200
    except Exception:
        return False


env = dict(os.environ, INTERACT_AI_HOME=home, INTERACT_AI_MOBILE_ADVERTISE="0")
for k in list(env):
    if k.startswith("INTERACT_AI_CLAUDE_BIN") or k.startswith("INTERACT_AI_CODEX_BIN"):
        env.pop(k)
dlog = open(os.path.join(out, "daemon.log"), "ab")
daemon = subprocess.Popen([BIN, "serve"], env=env, stdout=dlog, stderr=subprocess.STDOUT,
                          stdin=subprocess.DEVNULL, start_new_session=True)
LOG["daemonPid"] = daemon.pid
mark("daemon-spawned", pid=daemon.pid)
with open(os.path.join(out, "daemon.pid"), "w") as f:
    f.write(str(daemon.pid))
for _ in range(160):
    if ready():
        break
    time.sleep(0.25)
else:
    mark("daemon-not-ready")
    sys.exit(2)
mark("daemon-ready")
tok = open(os.path.join(home, "state", "api-token")).read().strip()


def api(method, path, body=None, timeout=30):
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


sse_path = os.path.join(out, "sse.jsonl")
stop_sse = threading.Event()


def sse():
    req = urllib.request.Request(base + "/v1/events",
                                 headers={"Authorization": "Bearer " + tok, "accept": "text/event-stream"})
    try:
        with urllib.request.urlopen(req, timeout=3600) as r, open(sse_path, "a") as f:
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
time.sleep(0.4)


def descendants(root):
    """pids whose ppid chain reaches `root`, with command lines."""
    ps = subprocess.run(["ps", "-axo", "pid=,ppid=,pgid=,etime=,command="],
                        capture_output=True, text=True).stdout.splitlines()
    rows = {}
    for line in ps:
        parts = line.split(None, 4)
        if len(parts) < 5:
            continue
        pid, ppid, pgid, etime, cmd = parts
        rows[int(pid)] = (int(ppid), int(pgid), etime, cmd)
    kids = []
    for pid, (ppid, pgid, etime, cmd) in rows.items():
        p, hops = ppid, 0
        while p > 1 and hops < 20:
            if p == root:
                kids.append({"pid": pid, "ppid": ppid, "pgid": pgid, "etime": etime, "cmd": cmd[:200]})
                break
            nxt = rows.get(p)
            if not nxt:
                break
            p = nxt[0]
            hops += 1
    return sorted(kids, key=lambda k: k["pid"])


def agent_children():
    return [c for c in descendants(daemon.pid)
            if ("claude" in c["cmd"] and "--input-format" in c["cmd"]) or "codex" in c["cmd"]]


TERMINAL = {"claimed-completed", "failed", "timed-out", "cancelled", "expired", "closed", "unknown"}


def run_turn(sid, task, tag, timeout=None):
    """Send one task and wait for the session to leave the working states."""
    timeout = timeout or a.turn_timeout
    st0, cur0 = api("GET", "/v1/agent-sessions/" + sid)
    before = (cur0 or {}).get("state")
    kids_before = agent_children()
    s, sent = api("POST", "/v1/agent-sessions/%s/messages" % sid, {"kind": "task", "body": {"task": task}})
    mark(tag + "/send", status=s, sessionState=before, resp=(sent if s >= 400 else {"messageId": (sent or {}).get("messageId")}))
    turn = {"tag": tag, "task": task, "sendStatus": s, "sendResp": sent, "stateBefore": before,
            "childrenBefore": kids_before, "transitions": [], "startedAt": now()}
    if s >= 400:
        turn["finishedAt"] = now()
        return turn
    t_send = time.monotonic()
    last = before
    saw_work = False
    approved = []
    final = None
    while time.monotonic() - t_send < timeout:
        st, cur = api("GET", "/v1/agent-sessions/" + sid)
        state = (cur or {}).get("state")
        if state != last:
            turn["transitions"].append({"mono": round(time.monotonic() - t_send, 2), "state": state,
                                        "children": agent_children()})
            mark(tag + "/state", state=state)
            last = state
        if state in ("active", "waiting-for-consent", "waiting-for-input"):
            saw_work = True
        if state == "waiting-for-consent" and a.approve != "none":
            s3, msgs = api("GET", "/v1/agent-sessions/%s/messages?direction=from-session" % sid)
            for m in (msgs or []):
                if m.get("kind") == "approval-request":
                    rid = (m.get("body") or {}).get("requestId")
                    if rid and rid not in approved:
                        s4, r4 = api("POST", "/v1/agent-sessions/%s/approve" % sid,
                                     {"requestId": rid, "approve": a.approve == "approve"})
                        approved.append(rid)
                        mark(tag + "/approve", requestId=rid, decision=a.approve, status=s4,
                             summary=(m.get("body") or {}).get("summary"))
                        turn.setdefault("approvals", []).append(
                            {"requestId": rid, "decision": a.approve, "status": s4,
                             "summary": (m.get("body") or {}).get("summary")})
        if saw_work and state in TERMINAL:
            final = cur
            break
        if not saw_work and state in TERMINAL and time.monotonic() - t_send > 60:
            # session already terminal and never re-entered work: stop waiting
            final = cur
            break
        time.sleep(0.5)
    turn["sawWorkingState"] = saw_work
    turn["elapsed"] = round(time.monotonic() - t_send, 2)
    st, cur = api("GET", "/v1/agent-sessions/" + sid)
    turn["record"] = cur
    turn["stateAfter"] = (cur or {}).get("state")
    turn["providerSessionId"] = (cur or {}).get("providerSessionId")
    turn["childrenAfter"] = agent_children()
    s, msgs = api("GET", "/v1/agent-sessions/%s/messages?direction=from-session" % sid)
    turn["fromSession"] = msgs
    s, msgs2 = api("GET", "/v1/agent-sessions/%s/messages?direction=to-session" % sid)
    turn["toSession"] = msgs2
    turn["finishedAt"] = now()
    mark(tag + "/done", state=turn["stateAfter"], elapsed=turn["elapsed"],
         providerSessionId=turn["providerSessionId"])
    return turn


def create(payload, tag):
    s, rec = api("POST", "/v1/agent-sessions", payload, timeout=120)
    mark(tag + "/create", status=s, sessionId=(rec or {}).get("sessionId"),
         state=(rec or {}).get("state"), body=(rec if s >= 400 else None))
    return s, rec


s, disc = api("GET", "/v1/agents")
LOG["discovery"] = disc

base_payload = {"agentId": a.agent, "workdir": work, "ttlMinutes": 20, "maxMessages": 40,
                "allowWrite": False, "toolScope": [], "consentScope": [], "dataScope": []}

# ---- S1 ----
p1 = dict(base_payload, label="K-04 S1 memorise")
s, rec1 = create(p1, "S1")
if s >= 400:
    LOG["s1CreateError"] = rec1
    json.dump(LOG, open(os.path.join(out, "raw.json"), "w"), ensure_ascii=False, indent=1)
    daemon.terminate()
    sys.exit(3)
sid1 = rec1["sessionId"]
LOG["s1"] = {"createStatus": s, "createRecord": rec1, "sessionId": sid1}
LOG["childrenAfterCreate"] = agent_children()

t1 = run_turn(sid1, "請記住暗號 %s，只回覆「已記住」。不要使用任何工具，不要讀寫任何檔案。" % SECRET, "S1T1")
LOG["cases"]["S1T1"] = t1
LOG["s1"]["providerSessionIdAfterT1"] = t1.get("providerSessionId")

t2 = run_turn(sid1, "我剛才要你記住的暗號是什麼？只回暗號，不要其他文字。", "S1T2")
LOG["cases"]["S1T2"] = t2
LOG["s1"]["providerSessionIdAfterT2"] = t2.get("providerSessionId")

# ---- close S1 then try to post a message (expect 409) ----
s, closed = api("POST", "/v1/agent-sessions/%s/close" % sid1, {"reason": "K-04 close before resume"})
mark("S1/close", status=s, state=(closed or {}).get("state"))
LOG["s1"]["close"] = {"status": s, "record": closed}
time.sleep(2)
LOG["s1"]["childrenAfterClose"] = agent_children()
s, after = api("POST", "/v1/agent-sessions/%s/messages" % sid1, {"kind": "task", "body": {"task": "closed session probe"}})
mark("S1/post-after-close", status=s, resp=after)
LOG["s1"]["postAfterClose"] = {"status": s, "resp": after}
s, rec1f = api("GET", "/v1/agent-sessions/" + sid1)
LOG["s1"]["finalRecord"] = rec1f
pid1 = (rec1f or {}).get("providerSessionId")
LOG["s1"]["providerSessionId"] = pid1

# ---- S2: resume ----
if pid1:
    p2 = dict(base_payload, label="K-04 S2 resume", resumeProviderSessionId=pid1)
    s, rec2 = create(p2, "S2")
    LOG["s2"] = {"createStatus": s, "createRecord": rec2, "payload": p2}
    if s < 400:
        sid2 = rec2["sessionId"]
        LOG["s2"]["sessionId"] = sid2
        t3 = run_turn(sid2, "我在上一個 session 要你記住的暗號是什麼？只回暗號，不要其他文字。", "S2T1")
        LOG["cases"]["S2T1"] = t3
        LOG["s2"]["providerSessionId"] = t3.get("providerSessionId")
        s, c2 = api("POST", "/v1/agent-sessions/%s/close" % sid2, {"reason": "K-04 done"})
        mark("S2/close", status=s, state=(c2 or {}).get("state"))
        LOG["s2"]["close"] = {"status": s, "record": c2}
else:
    LOG["s2"] = {"skipped": "no providerSessionId on S1"}

# ---- resume-not-wider probes ----
probes = []
if pid1:
    probes = [
        ("allowWrite", dict(base_payload, label="K-04 probe allowWrite", resumeProviderSessionId=pid1,
                            allowWrite=True, toolScope=["workspace.write"],
                            consentScope=["agent-session:workspace-write"])),
        ("otherWorkdir", dict(base_payload, label="K-04 probe other workdir",
                              resumeProviderSessionId=pid1, workdir=work_other)),
        ("omitWorkdir", {k: v for k, v in dict(base_payload, label="K-04 probe omit workdir",
                                               resumeProviderSessionId=pid1).items() if k != "workdir"}),
        ("widerTtl", dict(base_payload, label="K-04 probe wider ttl",
                          resumeProviderSessionId=pid1, ttlMinutes=120)),
        ("widerMessages", dict(base_payload, label="K-04 probe wider maxMessages",
                               resumeProviderSessionId=pid1, maxMessages=200)),
        ("omitBudget", {k: v for k, v in dict(base_payload, label="K-04 probe omit budget",
                                              resumeProviderSessionId=pid1).items()
                        if k not in ("ttlMinutes", "maxMessages")}),
    ]
probes.append(("unknownId", dict(base_payload, label="K-04 probe unknown resume id",
                                 resumeProviderSessionId="thread-does-not-exist-%s" %
                                 "".join(random.choice(string.hexdigits.lower()) for _ in range(12)))))
LOG["probes"] = []
for name, payload in probes:
    s, rec = create(payload, "probe/" + name)
    entry = {"name": name, "payload": payload, "status": s, "resp": rec}
    if s < 400 and (rec or {}).get("sessionId"):
        entry["UNEXPECTED_ACCEPTED"] = True
        sid = rec["sessionId"]
        entry["childrenAfterCreate"] = agent_children()
        sc, cr = api("POST", "/v1/agent-sessions/%s/close" % sid, {"reason": "K-04 probe cleanup"})
        entry["cleanupClose"] = {"status": sc, "state": (cr or {}).get("state")}
        mark("probe/" + name + "/cleanup", status=sc)
        time.sleep(1)
    LOG["probes"].append(entry)

s, LOG["status"] = api("GET", "/v1/status")
s, aud = api("GET", "/v1/audit")
LOG["auditTail"] = aud
s, LOG["sessionsList"] = api("GET", "/v1/agent-sessions")
LOG["childrenAtEnd"] = agent_children()
LOG["descendantsAtEnd"] = descendants(daemon.pid)

daemon.terminate()
try:
    daemon.wait(timeout=20)
except subprocess.TimeoutExpired:
    daemon.kill()
mark("daemon-stopped", returncode=daemon.returncode)
time.sleep(1)
LOG["strayAfterDaemonStop"] = subprocess.run(
    ["bash", "-lc", "ps -axo pid,pgid,ppid,etime,command | grep -E 'claude -p --input-format|codex app-server' | grep -v grep | cut -c1-180"],
    capture_output=True, text=True).stdout.splitlines()
stop_sse.set()
LOG["wallSeconds"] = round(time.monotonic() - T0, 2)
LOG["finishedAt"] = now()
json.dump(LOG, open(os.path.join(out, "raw.json"), "w"), ensure_ascii=False, indent=1)
print("DONE", json.dumps({"agent": a.agent, "secret": SECRET, "s1": LOG.get("s1", {}).get("sessionId"),
                          "pid1": pid1, "wall": LOG["wallSeconds"]}, ensure_ascii=False))
