#!/usr/bin/env python3
"""Real-agent smoke via the official HTTP path (isolated INTERACT_AI_HOME).
usage: agent_smoke.py --agent claude-code|codex --port N --out DIR [--task TEXT] [--allow-write] [--cancel-after SECS] [--timeout SECS]
Records: daemon log, SSE stream, session record snapshots, mailbox both directions, timings. Never touches ~/.adaptive-interaction.
"""
import argparse, json, os, subprocess, sys, time, threading, urllib.request, urllib.error, tempfile, signal, shutil
ROOT=os.environ.get("INTERACT_AI_REPO","/Users/user/Workspace/claude-lab/adaptive-interaction"); BIN=os.environ.get("INTERACT_AI_BIN",f"{ROOT}/target/debug/interact-ai")
ap=argparse.ArgumentParser(); ap.add_argument("--agent",required=True); ap.add_argument("--port",type=int,required=True); ap.add_argument("--out",required=True)
ap.add_argument("--task",default="請讀取目前工作目錄裡的 NOTES.md，用一句話（繁體中文）回答檔案的第一個標題是什麼。不要修改任何檔案。")
ap.add_argument("--allow-write",action="store_true"); ap.add_argument("--cancel-after",type=float,default=0); ap.add_argument("--cancel-mode",default="interrupt",choices=["interrupt","close"])
ap.add_argument("--timeout",type=float,default=240); ap.add_argument("--approve",default="none",choices=["none","approve","deny"]); ap.add_argument("--close-after-cancel",type=float,default=0); ap.add_argument("--home",default=""); ap.add_argument("--keep-daemon",action="store_true"); ap.add_argument("--label",default="phase0-smoke"); ap.add_argument("--reuse-daemon",action="store_true"); ap.add_argument("--cancel-when-active",action="store_true",help="only issue the cancel once state==active has been observed (cancel-after then counts from the first active observation)")
a=ap.parse_args(); out=a.out; os.makedirs(out,exist_ok=True)
home=a.home or os.path.join(out,"home"); os.makedirs(os.path.join(home,"config"),exist_ok=True)
with open(os.path.join(home,"config","interaction.yaml"),"w") as f: f.write(f"apiHost: 127.0.0.1\napiPort: {a.port}\n")
work=os.path.join(out,"workdir"); os.makedirs(work,exist_ok=True)
with open(os.path.join(work,"NOTES.md"),"w") as f: f.write("# 階段零煙霧測試\n\n這是隔離工作目錄的檔案。\n")
base=f"http://127.0.0.1:{a.port}"; log={"agent":a.agent,"port":a.port,"task":a.task,"allowWrite":a.allow_write,"events":[],"snapshots":[]}
def now(): return time.strftime("%Y-%m-%dT%H:%M:%S",time.gmtime())+f".{int(time.time()*1000)%1000:03d}Z"
def mark(k,**kw): rec={"t":now(),"mono":round(time.monotonic()-T0,3),"k":k,**kw}; log["events"].append(rec); print(json.dumps(rec,ensure_ascii=False),flush=True)
T0=time.monotonic(); daemon=None
def ready():
    try: return urllib.request.urlopen(f"{base}/v1/ready",timeout=1).status==200
    except Exception: return False
if not (a.reuse_daemon and ready()):
    env=dict(os.environ,INTERACT_AI_HOME=home,INTERACT_AI_MOBILE_ADVERTISE="0")
    for k in list(env):
        if k.startswith("INTERACT_AI_CLAUDE_BIN") or k.startswith("INTERACT_AI_CODEX_BIN"): env.pop(k)
    dlog=open(os.path.join(out,"daemon.log"),"ab")
    daemon=subprocess.Popen([BIN,"serve"],env=env,stdout=dlog,stderr=subprocess.STDOUT,stdin=subprocess.DEVNULL,start_new_session=True)
    mark("daemon-spawned",pid=daemon.pid)
    for _ in range(120):
        if ready(): break
        time.sleep(0.25)
    else: mark("daemon-not-ready"); sys.exit(2)
mark("daemon-ready")
tok=open(os.path.join(home,"state","api-token")).read().strip()
def api(method,path,body=None):
    req=urllib.request.Request(base+path,method=method,headers={"Authorization":f"Bearer {tok}","content-type":"application/json"},data=(json.dumps(body).encode() if body is not None else None))
    try:
        with urllib.request.urlopen(req,timeout=30) as r: return r.status,json.loads(r.read() or b"null")
    except urllib.error.HTTPError as e:
        try: return e.code,json.loads(e.read() or b"null")
        except Exception: return e.code,None
# SSE capture thread
sse_path=os.path.join(out,"sse.jsonl"); stop_sse=threading.Event()
def sse():
    req=urllib.request.Request(base+"/v1/events",headers={"Authorization":f"Bearer {tok}","accept":"text/event-stream"})
    try:
        with urllib.request.urlopen(req,timeout=600) as r, open(sse_path,"a") as f:
            ev={}
            for raw in r:
                if stop_sse.is_set(): break
                line=raw.decode("utf-8","replace").rstrip("\n")
                if line=="" :
                    if ev: ev["_t"]=now(); f.write(json.dumps(ev,ensure_ascii=False)+"\n"); f.flush(); ev={}
                elif ":" in line:
                    k,v=line.split(":",1); ev.setdefault(k.strip(),""); ev[k.strip()]+=v.strip()
    except Exception as e:
        with open(sse_path,"a") as f: f.write(json.dumps({"_sse_error":str(e),"_t":now()})+"\n")
threading.Thread(target=sse,daemon=True).start(); time.sleep(0.5)
s,disc=api("GET","/v1/agents"); log["discovery"]=disc
payload={"agentId":a.agent,"label":a.label,"workdir":work,"ttlMinutes":20,"maxMessages":40,
         "allowWrite":a.allow_write,"toolScope":(["workspace.write"] if a.allow_write else []),
         "consentScope":(["agent-session:workspace-write"] if a.allow_write else [])}
s,rec=api("POST","/v1/agent-sessions",payload); mark("create",status=s,state=(rec or {}).get("state"),id=(rec or {}).get("sessionId"),error=None if s<400 else rec)
if s>=400: log["createError"]=rec; json.dump(log,open(os.path.join(out,"result.json"),"w"),ensure_ascii=False,indent=1); (daemon and daemon.terminate()); sys.exit(3)
sid=rec.get("sessionId") or rec.get("id"); log["sessionId"]=sid; log["createRecord"]=rec
s,sent=api("POST",f"/v1/agent-sessions/{sid}/messages",{"kind":"task","body":{"task":a.task}}); mark("task-sent",status=s,resp=sent if s>=400 else {k:sent.get(k) for k in ("id","kind","direction","contextBundle") if isinstance(sent,dict)})
log["taskSend"]=sent
last=None; terminal={"claimed-completed","failed","timed-out","cancelled","expired","closed","unknown"}; cancelled=False; t_send=time.monotonic()
while time.monotonic()-t_send<a.timeout:
    s,cur=api("GET",f"/v1/agent-sessions/{sid}"); st=(cur or {}).get("state")
    if st!=last: mark("state",state=st,delegation=(cur or {}).get("delegationState") or (cur or {}).get("delegation")); log["snapshots"].append(cur); last=st
    if st=="active" and "t_active" not in log: log["t_active"]=round(time.monotonic()-T0,3); t_active=time.monotonic()
    if a.cancel_after and not cancelled and ((not a.cancel_when_active and time.monotonic()-t_send>=a.cancel_after and st in ("active","created","waiting-for-consent")) or (a.cancel_when_active and "t_active" in log and time.monotonic()-t_active>=a.cancel_after)):
        if a.cancel_mode=="interrupt": s2,r2=api("POST",f"/v1/agent-sessions/{sid}/interrupt")
        else: s2,r2=api("POST",f"/v1/agent-sessions/{sid}/close",{"reason":"phase0 cancel test"})
        mark("cancel-issued",mode=a.cancel_mode,status=s2,resp=r2); cancelled=True; log["cancelResp"]=r2
    if st in ("waiting-for-consent",) and a.approve!="none":
        s3,msgs=api("GET",f"/v1/agent-sessions/{sid}/messages?direction=from-session")
        pend=[m for m in (msgs or []) if m.get("kind")=="approval-request" and m.get("body",{}).get("requestId") not in log.setdefault("approved",[])]
        for m in pend:
            rid=m["body"]["requestId"]; s4,r4=api("POST",f"/v1/agent-sessions/{sid}/approve",{"requestId":rid,"approve":a.approve=="approve"})
            log["approved"].append(rid); mark("operator-approval",decision=a.approve,requestId=rid,summary=m["body"].get("summary"),status=s4,resp=r4)
    if st in terminal and (not a.cancel_after or cancelled): break
    time.sleep(0.5)
time.sleep(1.5)
def child_procs():
    return subprocess.run(["bash","-lc","ps -axo pid,pgid,ppid,etime,stat,command | grep -E '(claude -p --input-format|codex app-server$|codex exec)' | grep -v grep | cut -c1-160"],capture_output=True,text=True).stdout.splitlines()
if cancelled:
    s,mid=api("GET",f"/v1/agent-sessions/{sid}"); mark("after-cancel",state=(mid or {}).get("state"),children=child_procs()); log["afterCancel"]=mid
    if a.close_after_cancel:
        time.sleep(a.close_after_cancel)
        s5,r5=api("POST",f"/v1/agent-sessions/{sid}/close",{"reason":"phase0 close after cancel"}); mark("close-issued",status=s5,resp=r5)
        time.sleep(2); mark("after-close",children=child_procs())
s,final=api("GET",f"/v1/agent-sessions/{sid}"); log["final"]=final; mark("final",state=(final or {}).get("state"))
for d in ("to-session","from-session"):
    s,m=api("GET",f"/v1/agent-sessions/{sid}/messages?direction={d}"); log[f"messages-{d}"]=m
s,st=api("GET","/v1/status"); log["status"]=st
s,aud=api("GET","/v1/audit"); log["auditTail"]=(aud if isinstance(aud,list) else (aud or {}).get("items") or aud)
# process check: any child claude/codex still alive?
ps=subprocess.run(["bash","-lc","ps -axo pid,pgid,ppid,etime,command | grep -E '(claude|codex)' | grep -v grep | grep -v agmsg | grep -v 'claude-code-guide' | cut -c1-200"],capture_output=True,text=True).stdout
log["psAgents"]=ps.splitlines()
if not a.keep_daemon and daemon:
    daemon.terminate()
    try: daemon.wait(timeout=15)
    except subprocess.TimeoutExpired: daemon.kill()
    mark("daemon-stopped")
stop_sse.set()
log["finishedAt"]=now(); log["wallSeconds"]=round(time.monotonic()-T0,2)
json.dump(log,open(os.path.join(out,"result.json"),"w"),ensure_ascii=False,indent=1)
print("RESULT", json.dumps({"agent":a.agent,"final":(final or {}).get("state"),"sid":sid,"wall":log["wallSeconds"]},ensure_ascii=False))
