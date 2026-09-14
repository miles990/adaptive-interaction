#!/usr/bin/env python3
"""Which guard fires for each resume-widening shape? No agent turn is spawned
(every request is expected to be rejected at create time)."""
import argparse, json, os, subprocess, sys, time, urllib.request, urllib.error

BIN = os.environ.get("INTERACT_AI_BIN", "/Users/user/Workspace/claude-lab/adaptive-interaction/target/debug/interact-ai")
ap = argparse.ArgumentParser()
ap.add_argument("--agent", required=True)
ap.add_argument("--port", type=int, required=True)
ap.add_argument("--home", required=True)
ap.add_argument("--workdir", required=True)
ap.add_argument("--resume-id", required=True)
ap.add_argument("--out", required=True)
a = ap.parse_args()
os.makedirs(a.out, exist_ok=True)
with open(os.path.join(a.home, "config", "interaction.yaml"), "w") as f:
    f.write("apiHost: 127.0.0.1\napiPort: %d\n" % a.port)
base = "http://127.0.0.1:%d" % a.port
env = dict(os.environ, INTERACT_AI_HOME=a.home, INTERACT_AI_MOBILE_ADVERTISE="0")
for k in list(env):
    if k.startswith("INTERACT_AI_CLAUDE_BIN") or k.startswith("INTERACT_AI_CODEX_BIN"):
        env.pop(k)
dlog = open(os.path.join(a.out, "daemon.log"), "ab")
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
tok = open(os.path.join(a.home, "state", "api-token")).read().strip()


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


B = {"agentId": a.agent, "workdir": a.workdir, "ttlMinutes": 20, "maxMessages": 40,
     "allowWrite": False, "toolScope": [], "consentScope": [], "dataScope": [],
     "resumeProviderSessionId": a.resume_id}
shapes = [
    ("allowWrite-no-scopes", dict(B, label="p1", allowWrite=True)),
    ("allowWrite-with-consent-only", dict(B, label="p2", allowWrite=True,
                                          consentScope=["agent-session:workspace-write"])),
    ("allowWrite-full", dict(B, label="p3", allowWrite=True, toolScope=["workspace.write"],
                             consentScope=["agent-session:workspace-write"])),
    ("dataScope-wider", dict(B, label="p4", dataScope=["domain:private"])),
    ("maxCost-added", dict(B, label="p5", maxCost=5.0)),
    ("workdir-dotdot", dict(B, label="p6", workdir=a.workdir + "/../workdir-other")),
    ("workdir-trailing-slash", dict(B, label="p7", workdir=a.workdir + "/")),
    ("ttl-narrower", dict(B, label="p8", ttlMinutes=5, maxMessages=10)),
    ("empty-resume-id", dict(B, label="p9", resumeProviderSessionId="")),
    ("resume-id-whitespace", dict(B, label="p10", resumeProviderSessionId="  ")),
]
res = []
for name, payload in shapes:
    s, rec = api("POST", "/v1/agent-sessions", payload)
    row = {"name": name, "status": s, "payload": payload,
           "error": (rec or {}).get("error") if s >= 400 else None,
           "sessionId": (rec or {}).get("sessionId") if s < 400 else None,
           "record": rec if s < 400 else None}
    if s < 400 and row["sessionId"]:
        sc, cr = api("POST", "/v1/agent-sessions/%s/close" % row["sessionId"], {"reason": "guard probe cleanup"})
        row["cleanupClose"] = {"status": sc, "state": (cr or {}).get("state")}
        time.sleep(1)
    res.append(row)
    print(json.dumps({"name": name, "status": s,
                      "msg": ((rec or {}).get("error") or {}).get("message", "")[:200] if s >= 400 else "ACCEPTED"},
                     ensure_ascii=False), flush=True)
s, aud = api("GET", "/v1/audit")
json.dump({"probes": res, "auditAfter": aud}, open(os.path.join(a.out, "raw.json"), "w"), ensure_ascii=False, indent=1)
d.terminate()
try:
    d.wait(timeout=20)
except subprocess.TimeoutExpired:
    d.kill()
print("DONE")
