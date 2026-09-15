#!/usr/bin/env python3
"""階段 1 真 Agent 追蹤驗收：同一個隔離 daemon 上依序跑多個情境，逐一查回因果鏈。

usage: trace_e2e.py --agent claude-code|codex --port N --out DIR [--scenarios normal,cancel,resume,failure,approval,restart]

只走正式路徑（interact-ai serve＋HTTP API）；不注入狀態、不偽造事件。每個情境記錄：
session 建立回應、終態、`GET /v1/trace?sessionId=`、`GET /v1/agent-sessions/{id}/activity`、
期望的 kind／outcome 是否出現。結果分類：passed／product-failed／not-run／blocked，寫進 result.json。
永不觸碰 ~/.adaptive-interaction；daemon 用 INTERACT_AI_HOME=<out>/home。
"""
import argparse, hashlib, json, os, signal, subprocess, sys, time, urllib.error, urllib.request

ROOT = os.environ.get("INTERACT_AI_REPO") or os.path.abspath(
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..")
)
BIN = os.environ.get("INTERACT_AI_BIN", f"{ROOT}/target/debug/interact-ai")
ap = argparse.ArgumentParser()
ap.add_argument("--agent", required=True)
ap.add_argument("--port", type=int, required=True)
ap.add_argument("--out", required=True)
ap.add_argument("--scenarios", default="normal,cancel,resume,failure,approval,restart")
ap.add_argument("--timeout", type=float, default=240)
a = ap.parse_args()
OUT = a.out
os.makedirs(OUT, exist_ok=True)
HOME = os.path.join(OUT, "home")
os.makedirs(os.path.join(HOME, "config"), exist_ok=True)
with open(os.path.join(HOME, "config", "interaction.yaml"), "w") as f:
    f.write(f"apiHost: 127.0.0.1\napiPort: {a.port}\n")
BASE = f"http://127.0.0.1:{a.port}"
T0 = time.monotonic()
RESULT = {"agent": a.agent, "port": a.port, "commit": None, "binarySha256": None, "scenarios": {}, "events": []}


def now():
    return time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime()) + f".{int(time.time() * 1000) % 1000:03d}Z"


def mark(k, **kw):
    rec = {"t": now(), "mono": round(time.monotonic() - T0, 3), "k": k, **kw}
    RESULT["events"].append(rec)
    print(json.dumps(rec, ensure_ascii=False), flush=True)


def sh(cmd):
    return subprocess.run(["bash", "-lc", cmd], capture_output=True, text=True).stdout.strip()


RESULT["commit"] = sh(f"cd {ROOT} && git rev-parse HEAD")
RESULT["worktreeDirty"] = bool(sh(f"cd {ROOT} && git status --porcelain"))
with open(BIN, "rb") as fb:
    RESULT["binarySha256"] = hashlib.sha256(fb.read()).hexdigest()
RESULT["versions"] = {
    "interact-ai": sh(f"{BIN} --version"),
    "claude": sh("claude --version 2>/dev/null"),
    "codex": sh("codex --version 2>/dev/null"),
    "os": sh("sw_vers -productVersion 2>/dev/null; uname -m"),
}

DAEMON = None


def ready():
    try:
        return urllib.request.urlopen(f"{BASE}/v1/ready", timeout=1).status == 200
    except Exception:
        return False


def start_daemon():
    global DAEMON, TOK
    env = dict(os.environ, INTERACT_AI_HOME=HOME, INTERACT_AI_MOBILE_ADVERTISE="0")
    for k in list(env):
        if k.startswith("INTERACT_AI_CLAUDE_BIN") or k.startswith("INTERACT_AI_CODEX_BIN"):
            env.pop(k)
    dlog = open(os.path.join(OUT, "daemon.log"), "ab")
    DAEMON = subprocess.Popen([BIN, "serve"], env=env, stdout=dlog, stderr=subprocess.STDOUT,
                              stdin=subprocess.DEVNULL, start_new_session=True)
    mark("daemon-spawned", pid=DAEMON.pid)
    for _ in range(160):
        if ready():
            break
        time.sleep(0.25)
    else:
        mark("daemon-not-ready")
        sys.exit(2)
    TOK = open(os.path.join(HOME, "state", "api-token")).read().strip()
    mark("daemon-ready")


def stop_daemon(sig=signal.SIGTERM):
    global DAEMON
    if not DAEMON:
        return
    if sig == signal.SIGKILL:
        os.killpg(DAEMON.pid, signal.SIGKILL)
    else:
        DAEMON.terminate()
    try:
        DAEMON.wait(timeout=15)
    except subprocess.TimeoutExpired:
        DAEMON.kill()
    mark("daemon-stopped", signal=int(sig))
    DAEMON = None


def api(method, path, body=None, token=None):
    req = urllib.request.Request(BASE + path, method=method,
                                 headers={"Authorization": f"Bearer {token or TOK}", "content-type": "application/json"},
                                 data=(json.dumps(body).encode() if body is not None else None))
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return r.status, json.loads(r.read() or b"null")
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.loads(e.read() or b"null")
        except Exception:
            return e.code, None


def workdir(name):
    w = os.path.join(OUT, f"work-{name}")
    os.makedirs(w, exist_ok=True)
    with open(os.path.join(w, "NOTES.md"), "w") as f:
        f.write(f"# 階段一追蹤驗收（{name}）\n\n暗號：{name}-{a.port}\n")
    return w


TERMINAL = {"claimed-completed", "failed", "timed-out", "cancelled", "expired", "closed", "unknown"}
LONG_TASK = "請用繁體中文寫一篇約 2500 字、分十段、每段都有小標題的短文，主題是「山中的一間小屋與四季」。不要讀取或修改任何檔案，直接輸出全文。"
READ_TASK = "請讀取目前工作目錄裡的 NOTES.md，用一句話（繁體中文）回答檔案的第一個標題與暗號是什麼。不要修改任何檔案。"


def create(name, extra=None, task=READ_TASK, send=True):
    payload = {"agentId": a.agent, "label": f"phase1-{name}", "workdir": workdir(name), "ttlMinutes": 20,
               "maxMessages": 40, "allowWrite": False, "toolScope": [], "consentScope": []}
    payload.update(extra or {})
    s, rec = api("POST", "/v1/agent-sessions", payload)
    mark("create", scenario=name, status=s, id=(rec or {}).get("sessionId"), error=None if s < 400 else rec)
    if s >= 400 or not send:
        return s, rec, None
    sid = rec["sessionId"]
    s2, sent = api("POST", f"/v1/agent-sessions/{sid}/messages", {"kind": "task", "body": {"task": task}})
    mark("task-sent", scenario=name, status=s2, messageId=(sent or {}).get("messageId") if isinstance(sent, dict) else None)
    return s, rec, sent


APPROVED = set()


def approve_pending(sid, name):
    """codex 在唯讀 sandbox 下連讀檔都要核可：正常情境由人類（本 harness）核准，並留下紀錄。"""
    s3, msgs = api("GET", f"/v1/agent-sessions/{sid}/messages?direction=from-session")
    for m in (msgs or []):
        rid = (m.get("body") or {}).get("requestId")
        # codex app-server 的 request id 每個子程序都從 0 重數：去重要連 session 一起看。
        if m.get("kind") == "approval-request" and rid and (sid, rid) not in APPROVED:
            APPROVED.add((sid, rid))
            s4, r4 = api("POST", f"/v1/agent-sessions/{sid}/approve", {"requestId": rid, "approve": True})
            mark("approve", scenario=name, requestId=rid, status=s4, summary=((m.get("body") or {}).get("summary") or "")[:80])


def wait_state(sid, pred, timeout, name, on_state=None, approve=False):
    last = None
    t = time.monotonic()
    while time.monotonic() - t < timeout:
        s, cur = api("GET", f"/v1/agent-sessions/{sid}")
        st = (cur or {}).get("state")
        ph = (cur or {}).get("phase")
        if (st, ph) != last:
            mark("state", scenario=name, state=st, phase=ph)
            last = (st, ph)
            if on_state:
                on_state(cur)
        if approve and st == "waiting-for-consent":
            approve_pending(sid, name)
        if pred(cur or {}):
            return cur
        time.sleep(0.5)
    return None


def trace_for(sid):
    s, t = api("GET", f"/v1/trace?sessionId={sid}&limit=200")
    s2, act = api("GET", f"/v1/agent-sessions/{sid}/activity")
    return (s, t), (s2, act)


def kinds(t):
    items = (t or {}).get("items") if isinstance(t, dict) else None
    return [(i.get("kind"), i.get("outcome"), i.get("code")) for i in (items or [])]


def check(name, sid, expect, classify=None, extra=None):
    (s, t), (s2, act) = trace_for(sid)
    ks = kinds(t)
    missing = [e for e in expect if not any(all(x is None or x == y for x, y in zip(e, k)) for k in ks)]
    sec = {"sessionId": sid, "traceStatus": s, "activityStatus": s2, "kinds": ks, "expected": expect,
           "missing": missing, "activity": act if s2 < 400 else None}
    # 敏感字串掃描：detail 不得含 token／secret／本機使用者名稱
    blob = json.dumps(t, ensure_ascii=False)
    leaks = [w for w in ("iat-session-", "Bearer ", "sk-ant", "/Users/user") if w in blob]
    sec["leaks"] = leaks
    if s == 404 or s2 == 404:
        sec["verdict"] = "blocked"
        sec["reason"] = "trace／activity endpoint 不存在（404）"
    elif classify:
        sec["verdict"], sec["reason"] = classify(sec)
    else:
        sec["verdict"] = "passed" if not missing and not leaks else "product-failed"
        sec["reason"] = ("missing " + json.dumps(missing) if missing else "") + (" leaks " + json.dumps(leaks) if leaks else "")
    if extra:
        sec.update(extra)
    RESULT["scenarios"][name] = sec
    mark("verdict", scenario=name, verdict=sec["verdict"], reason=sec.get("reason"))
    return sec


def close(sid, reason):
    s, r = api("POST", f"/v1/agent-sessions/{sid}/close", {"reason": reason})
    mark("close", status=s, state=(r or {}).get("state"))
    return r


# ---------------------------------------------------------------------------
start_daemon()
scen = [x for x in a.scenarios.split(",") if x]
prov = {}

if "normal" in scen:
    s, rec, _ = create("normal")
    sid = rec["sessionId"]
    fin = wait_state(sid, lambda c: c.get("state") in TERMINAL, a.timeout, "normal", approve=True)
    if fin and fin.get("state") == "claimed-completed":
        sv, vr = api("POST", f"/v1/agent-sessions/{sid}/verify", {"note": "phase1 human verify"})
        mark("verify", status=sv, humanVerified=bool((vr or {}).get("humanVerified")))
    s, rec2 = api("GET", f"/v1/agent-sessions/{sid}")
    prov["normal"] = rec2
    close(sid, "phase1 normal done")
    exp = [("agent-session.capability-issued", None, None), ("agent-session.dispatched", None, None),
           ("agent-session.task-delivered", None, None), ("agent-session.outcome", "claimed", "outcome.claimed-completed"),
           ("agent-session.verified", "verified", None), ("agent-session.closed", None, None)]
    if APPROVED:
        exp.append(("agent.approval", "accepted", "approval.human-approved"))
    check("normal", sid, exp, extra={"finalState": (fin or {}).get("state"), "actualModel": rec2.get("actualModel"),
                                     "providerSessionId": rec2.get("providerSessionId"), "phase": rec2.get("phase")})

if "cancel" in scen:
    s, rec, _ = create("cancel", task=LONG_TASK)
    sid = rec["sessionId"]
    seen_active = {}

    def on_state(c):
        if c.get("state") == "active" and "t" not in seen_active:
            seen_active["t"] = time.monotonic()

    def pred(c):
        return c.get("state") in TERMINAL or ("t" in seen_active and time.monotonic() - seen_active["t"] >= 3)

    cur = wait_state(sid, pred, a.timeout, "cancel", on_state)
    if cur and cur.get("state") == "active":
        si, ri = api("POST", f"/v1/agent-sessions/{sid}/interrupt")
        mark("interrupt", status=si, resp=ri)
        fin = wait_state(sid, lambda c: c.get("state") in TERMINAL, 60, "cancel")
    else:
        fin = cur
    time.sleep(1.5)
    close(sid, "phase1 cancel done")
    s, rec2 = api("GET", f"/v1/agent-sessions/{sid}")
    exp = [("agent-session.interrupt-requested", "accepted", "interrupt.sent"),
           ("agent-session.outcome", "cancelled", "outcome.cancelled")]

    def cls(sec):
        st = (fin or {}).get("state")
        if st == "cancelled" and not sec["missing"] and not sec["leaks"]:
            return "passed", ""
        return "product-failed", f"final={st} missing={sec['missing']} leaks={sec['leaks']}"

    check("cancel", sid, exp, cls, extra={"finalState": (fin or {}).get("state"), "detail": rec2.get("detail")})

if "resume" in scen and prov.get("normal") and prov["normal"].get("providerSessionId"):
    psid = prov["normal"]["providerSessionId"]
    base_extra = {"resumeProviderSessionId": psid, "ttlMinutes": 20, "maxMessages": 40, "workdir": prov["normal"].get("resolvedWorkdir")}
    # 越權：放寬 ttl
    wid = dict(base_extra, ttlMinutes=60)
    cs, rec, _ = create("resume-reject", wid, send=False)
    before = api("GET", "/v1/agent-sessions")[1] or []
    s, t = api("GET", "/v1/trace?kind=agent-session.resume-checked&limit=20")
    items = (t or {}).get("items") if isinstance(t, dict) else []
    rej = [i for i in (items or []) if i.get("outcome") == "rejected"]
    rr = RESULT["scenarios"]["resume-reject"] = {"createStatus": cs, "createError": rec if cs >= 400 else None}
    rr["rejectedRows"] = [(i.get("code"), i.get("traceId"), i.get("sessionId")) for i in rej]
    rr["sessionCreated"] = any(x.get("label") == "phase1-resume-reject" for x in before)
    rr["traceStatus"] = s
    blob = json.dumps(rej, ensure_ascii=False)
    rr["leaks"] = [w for w in ("iat-session-", "Bearer ", "/Users/user") if w in blob]
    if s == 404:
        rr["verdict"], rr["reason"] = "blocked", "trace endpoint 404"
    elif cs == 403 and rej and not rr["sessionCreated"] and rej[0].get("traceId") == prov["normal"]["sessionId"] and not rr["leaks"]:
        rr["verdict"], rr["reason"] = "passed", ""
    else:
        rr["verdict"], rr["reason"] = "product-failed", f"create={cs} rows={rr['rejectedRows']} created={rr['sessionCreated']} leaks={rr['leaks']}"
    mark("verdict", scenario="resume-reject", verdict=rr["verdict"], reason=rr["reason"])
    # 合法接續
    s, rec, _ = create("resume-accept", base_extra)
    if s < 400:
        sid = rec["sessionId"]
        fin = wait_state(sid, lambda c: c.get("state") in TERMINAL, a.timeout, "resume-accept", approve=True)
        close(sid, "phase1 resume done")
        exp = [("agent-session.resume-checked", "accepted", "resume.ok"), ("agent-session.dispatched", None, None)]

        def cls(sec):
            rows = [i for i in ((sec.get("activity") or {}).get("records") or []) if i.get("kind") == "agent-session.resume-checked"] if sec.get("activity") else []
            tr = trace_for(sid)[0][1]
            rr2 = [i for i in ((tr or {}).get("items") or []) if i.get("kind") == "agent-session.resume-checked"]
            ok_trace = bool(rr2) and rr2[0].get("traceId") == prov["normal"]["sessionId"]
            if not sec["missing"] and ok_trace and not sec["leaks"]:
                return "passed", ""
            return "product-failed", f"missing={sec['missing']} traceLinksOriginal={ok_trace} leaks={sec['leaks']}"

        check("resume-accept", sid, exp, cls, extra={"finalState": (fin or {}).get("state"), "createStatus": s})
    else:
        RESULT["scenarios"]["resume-accept"] = {"verdict": "product-failed", "reason": f"create {s}: {rec}"}
        mark("verdict", scenario="resume-accept", verdict="product-failed", reason=str(rec))

if "failure" in scen:
    # claude：maxCost 極小 → provider 端預算錯誤（真 provider 失敗路徑）；codex：對子程序樹送 SIGKILL → 無結局＝unknown（誠實）。
    PGIDS_BEFORE_FAILURE = set(sh("ps -axo pgid,command | grep -E 'codex app-server|codex exec' | grep -v grep | awk '{print $1}'").split())
    if a.agent == "claude-code":
        s, rec, _ = create("failure", {"maxCost": 0.0001}, task=LONG_TASK)
        expect_state = {"failed"}
        exp = [("agent-session.outcome", "failed", "outcome.connector-error")]
    else:
        s, rec, _ = create("failure", task=LONG_TASK)
        expect_state = {"unknown", "failed"}
        exp = [("agent-session.outcome", None, None)]
    sid = rec["sessionId"]
    if a.agent != "claude-code":
        cur = wait_state(sid, lambda c: c.get("state") in TERMINAL or c.get("state") == "active", a.timeout, "failure")
        time.sleep(2)
        # 只殺這個 session 的子程序樹：取「建立後新出現」的 codex 程序群組，不碰前一個情境殘留的。
        after = set(sh("ps -axo pgid,command | grep -E 'codex app-server|codex exec' | grep -v grep | awk '{print $1}'").split())
        new_groups = sorted(after - PGIDS_BEFORE_FAILURE)
        if new_groups:
            os.killpg(int(new_groups[-1]), signal.SIGKILL)
            mark("subprocess-killed", pgid=new_groups[-1], candidates=new_groups)
        else:
            mark("subprocess-not-found", before=sorted(PGIDS_BEFORE_FAILURE), after=sorted(after))
    fin = wait_state(sid, lambda c: c.get("state") in TERMINAL, a.timeout, "failure")
    s, rec2 = api("GET", f"/v1/agent-sessions/{sid}")
    close(sid, "phase1 failure done")

    def cls(sec):
        st = (fin or {}).get("state")
        if st in expect_state and not sec["missing"] and not sec["leaks"]:
            return "passed", ""
        return "product-failed", f"final={st} expected={sorted(expect_state)} missing={sec['missing']} leaks={sec['leaks']}"

    check("failure", sid, exp, cls, extra={"finalState": (fin or {}).get("state"), "detail": rec2.get("detail"),
                                          "failureReason": ((trace_for(sid)[1][1] or {}) or {}).get("failureReason")})

if "approval" in scen and a.agent == "codex":
    s, rec, _ = create("approval", task=READ_TASK)
    sid = rec["sessionId"]
    cur = wait_state(sid, lambda c: c.get("state") in TERMINAL or c.get("state") == "waiting-for-consent", a.timeout, "approval")
    if cur and cur.get("state") == "waiting-for-consent":
        s3, msgs = api("GET", f"/v1/agent-sessions/{sid}/messages?direction=from-session")
        pend = [m for m in (msgs or []) if m.get("kind") == "approval-request"]
        for m in pend[:1]:
            rid = m["body"]["requestId"]
            s4, r4 = api("POST", f"/v1/agent-sessions/{sid}/approve", {"requestId": rid, "approve": False})
            mark("deny", requestId=rid, status=s4)
        fin = wait_state(sid, lambda c: c.get("state") in TERMINAL, 90, "approval")
    else:
        fin = cur
    close(sid, "phase1 approval done")
    exp = [("agent.approval", "rejected", "approval.human-denied")]
    check("approval", sid, exp, extra={"finalState": (fin or {}).get("state"), "reachedConsent": bool(cur and cur.get("state") == "waiting-for-consent")})

if "restart" in scen:
    s, rec, _ = create("restart", task=LONG_TASK)
    sid = rec["sessionId"]
    cur = wait_state(sid, lambda c: c.get("state") in TERMINAL or c.get("state") == "active", a.timeout, "restart")
    time.sleep(2)
    stop_daemon(signal.SIGKILL)
    time.sleep(1)
    start_daemon()
    s, rec2 = api("GET", f"/v1/agent-sessions/{sid}")
    mark("after-restart", state=(rec2 or {}).get("state"), phase=(rec2 or {}).get("phase"), detail=(rec2 or {}).get("detail"))
    exp = [("agent-session.outcome", "unknown", "runtime.restarted")]

    def cls(sec):
        if (rec2 or {}).get("state") == "expired" and not sec["missing"]:
            return "passed", ""
        return "product-failed", f"state={(rec2 or {}).get('state')} missing={sec['missing']}"

    check("restart", sid, exp, cls, extra={"stateAfterRestart": (rec2 or {}).get("state"), "phaseAfterRestart": (rec2 or {}).get("phase")})

# 跨 session 隔離：任一 session 的 trace 不得含別的 session id
ids = [v.get("sessionId") for v in RESULT["scenarios"].values() if isinstance(v, dict) and v.get("sessionId")]
iso = {"verdict": "not-run"}
if len(ids) >= 2:
    bad = []
    for sid in ids:
        (s, t), _ = trace_for(sid)
        for i in ((t or {}).get("items") or []):
            if i.get("sessionId") and i.get("sessionId") != sid:
                bad.append((sid, i.get("sessionId"), i.get("kind")))
    iso = {"verdict": "passed" if not bad else "product-failed", "sessions": ids, "crossRows": bad}
RESULT["scenarios"]["isolation"] = iso
mark("verdict", scenario="isolation", verdict=iso["verdict"])

# agent token 不得讀 trace
atok_path = os.path.join(HOME, "state", "api-agent-token")
if os.path.exists(atok_path):
    atok = open(atok_path).read().strip()
    s, _ = api("GET", "/v1/trace?limit=1", token=atok)
    RESULT["scenarios"]["agent-token-forbidden"] = {"status": s, "verdict": "passed" if s == 403 else ("blocked" if s == 404 else "product-failed")}
    mark("verdict", scenario="agent-token-forbidden", status=s)

s, st = api("GET", "/v1/status")
RESULT["statusAfter"] = {k: (st or {}).get(k) for k in ("traceWriteFailures", "traceCounts", "version", "agentSessions")}
stop_daemon()
RESULT["wallSeconds"] = round(time.monotonic() - T0, 2)
json.dump(RESULT, open(os.path.join(OUT, "result.json"), "w"), ensure_ascii=False, indent=1)
print("RESULT", json.dumps({k: v.get("verdict") for k, v in RESULT["scenarios"].items()}, ensure_ascii=False))
