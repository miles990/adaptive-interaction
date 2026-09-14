#!/usr/bin/env python3
"""B-07: is POST /interrupt 'stop this turn' or 'end the session'?
Sends a long task, interrupts once active, then tries to send a NEW short task on the SAME session."""
import argparse, json, os, subprocess, sys, time, threading, urllib.request, urllib.error
BIN=os.environ.get("INTERACT_AI_BIN","/Users/user/Workspace/claude-lab/adaptive-interaction/target/debug/interact-ai")
ap=argparse.ArgumentParser(); ap.add_argument("--agent",required=True); ap.add_argument("--port",type=int,required=True); ap.add_argument("--out",required=True)
ap.add_argument("--long-task",default="請用繁體中文寫一篇約 1500 字、分成十段的短文，主題是「隔離工作目錄」。不要讀取、建立或修改任何檔案，直接在回覆中輸出全文。")
ap.add_argument("--short-task",default="只回覆一個字：好")
ap.add_argument("--cancel-after",type=float,default=3); ap.add_argument("--phase1-timeout",type=float,default=150); ap.add_argument("--phase2-timeout",type=float,default=150)
a=ap.parse_args(); out=a.out; os.makedirs(out,exist_ok=True)
home=os.path.join(out,"home"); os.makedirs(os.path.join(home,"config"),exist_ok=True)
open(os.path.join(home,"config","interaction.yaml"),"w").write(f"apiHost: 127.0.0.1\napiPort: {a.port}\n")
work=os.path.join(out,"workdir"); os.makedirs(work,exist_ok=True)
open(os.path.join(work,"NOTES.md"),"w").write("# 階段零 B-07\n\n隔離工作目錄。\n")
base=f"http://127.0.0.1:{a.port}"; log={"case":"B-07","agent":a.agent,"port":a.port,"events":[],"snapshots":[]}
def now(): return time.strftime("%Y-%m-%dT%H:%M:%S",time.gmtime())+f".{int(time.time()*1000)%1000:03d}Z"
T0=time.monotonic()
def mark(k,**kw):
    r={"t":now(),"mono":round(time.monotonic()-T0,3),"k":k,**kw}; log["events"].append(r); print(json.dumps(r,ensure_ascii=False)[:600],flush=True); return r
def ready():
    try: return urllib.request.urlopen(f"{base}/v1/ready",timeout=1).status==200
    except Exception: return False
env=dict(os.environ,INTERACT_AI_HOME=home,INTERACT_AI_MOBILE_ADVERTISE="0")
for k in list(env):
    if k.startswith("INTERACT_AI_CLAUDE_BIN") or k.startswith("INTERACT_AI_CODEX_BIN"): env.pop(k)
dlog=open(os.path.join(out,"daemon.log"),"ab")
daemon=subprocess.Popen([BIN,"serve"],env=env,stdout=dlog,stderr=subprocess.STDOUT,stdin=subprocess.DEVNULL,start_new_session=True)
DPID=daemon.pid; log["daemonPid"]=DPID; mark("daemon-spawned",pid=DPID)
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
sse_path=os.path.join(out,"sse.jsonl")
def sse():
    req=urllib.request.Request(base+"/v1/events",headers={"Authorization":f"Bearer {tok}","accept":"text/event-stream"})
    try:
        with urllib.request.urlopen(req,timeout=900) as r, open(sse_path,"a") as f:
            ev={}
            for raw in r:
                line=raw.decode("utf-8","replace").rstrip("\n")
                if line=="":
                    if ev: ev["_t"]=now(); f.write(json.dumps(ev,ensure_ascii=False)+"\n"); f.flush(); ev={}
                elif ":" in line:
                    k,v=line.split(":",1); ev.setdefault(k.strip(),""); ev[k.strip()]+=v.strip()
    except Exception as e:
        open(sse_path,"a").write(json.dumps({"_sse_error":str(e),"_t":now()})+"\n")
threading.Thread(target=sse,daemon=True).start(); time.sleep(0.5)
def kids():
    o=subprocess.run(["bash","-lc","ps -axo pid,ppid,etime,stat,command"],capture_output=True,text=True).stdout.splitlines()
    mine=[]; pids={DPID}
    for _ in range(4):
        for l in o[1:]:
            p=l.split(None,4)
            if len(p)>=5 and p[1].isdigit() and int(p[1]) in pids and int(p[0]) not in pids:
                pids.add(int(p[0])); mine.append(l[:150])
    return mine
payload={"agentId":a.agent,"label":"B-07-"+a.agent,"workdir":work,"ttlMinutes":25,"maxMessages":40,"allowWrite":False,"toolScope":[],"consentScope":[]}
s,rec=api("POST","/v1/agent-sessions",payload); mark("create",status=s,state=(rec or {}).get("state"),id=(rec or {}).get("sessionId"))
if s>=400: log["createError"]=rec; json.dump(log,open(os.path.join(out,"result.json"),"w"),ensure_ascii=False,indent=1); daemon.terminate(); sys.exit(3)
sid=rec["sessionId"]; log["sessionId"]=sid; log["createRecord"]=rec
TERM={"claimed-completed","failed","timed-out","cancelled","expired","closed","unknown"}
def wait_state(pred,timeout,tag):
    last=None; t=time.monotonic()
    while time.monotonic()-t<timeout:
        s,cur=api("GET",f"/v1/agent-sessions/{sid}"); st=(cur or {}).get("state")
        if st!=last: mark("state",phase=tag,state=st); log["snapshots"].append({"phase":tag,"rec":cur}); last=st
        if pred(st): return st,cur
        time.sleep(0.5)
    return last,None
# --- phase 1: long task, interrupt once active
s,m1=api("POST",f"/v1/agent-sessions/{sid}/messages",{"kind":"task","body":{"task":a.long_task}}); mark("task1-sent",status=s)
st,_=wait_state(lambda x:x=="active" or x in TERM,a.phase1_timeout,"p1-await-active")
mark("p1-active-observed",state=st,children=kids())
if st=="active":
    time.sleep(a.cancel_after)
    s2,r2=api("POST",f"/v1/agent-sessions/{sid}/interrupt"); mark("interrupt-issued",status=s2,resp=r2); log["interruptResp"]=r2
st,cur=wait_state(lambda x:x in TERM,60,"p1-await-terminal")
mark("p1-terminal",state=st,children=kids()); log["p1Terminal"]=cur
time.sleep(2)
mark("p1-children-after-2s",children=kids())
s,msgs=api("GET",f"/v1/agent-sessions/{sid}/messages?direction=from-session"); log["p1From"]=msgs; mark("p1-from-session",count=len(msgs or []))
# --- phase 2: send a NEW short task on the SAME session
s3,r3=api("POST",f"/v1/agent-sessions/{sid}/messages",{"kind":"task","body":{"task":a.short_task}})
mark("task2-sent",status=s3,resp=r3); log["task2Status"]=s3; log["task2Resp"]=r3
if s3<400:
    st2,cur2=wait_state(lambda x:x in TERM and x!="cancelled" and x!="failed",a.phase2_timeout,"p2")
    # also just observe until timeout/terminal-change
    mark("p2-observed",state=st2,children=kids())
else:
    st2=None
s,fin=api("GET",f"/v1/agent-sessions/{sid}"); log["final"]=fin; mark("final",state=(fin or {}).get("state"),children=kids())
for d in ("to-session","from-session"):
    s,m=api("GET",f"/v1/agent-sessions/{sid}/messages?direction={d}"); log["messages-"+d]=m
s,stt=api("GET","/v1/status"); log["status"]=stt
s,aud=api("GET","/v1/audit"); log["auditTail"]=aud
mark("children-final",children=kids())
daemon.terminate()
try: daemon.wait(timeout=15)
except subprocess.TimeoutExpired: daemon.kill()
mark("daemon-stopped")
log["wallSeconds"]=round(time.monotonic()-T0,2)
json.dump(log,open(os.path.join(out,"result.json"),"w"),ensure_ascii=False,indent=1)
print("RESULT",json.dumps({"agent":a.agent,"p1":(log.get("p1Terminal") or {}).get("state"),"task2Status":s3,"final":(fin or {}).get("state")},ensure_ascii=False))
