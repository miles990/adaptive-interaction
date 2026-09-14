#!/usr/bin/env python3
"""A-07: Emergency Stop against a live real-agent session, plus restart persistence."""
import argparse, json, os, subprocess, sys, time, threading, urllib.request, urllib.error, signal
BIN=os.environ.get("INTERACT_AI_BIN","/Users/user/Workspace/claude-lab/adaptive-interaction/target/debug/interact-ai")
ap=argparse.ArgumentParser(); ap.add_argument("--agent",default="claude-code"); ap.add_argument("--port",type=int,required=True); ap.add_argument("--out",required=True)
ap.add_argument("--task",default="請用繁體中文寫一篇約 1500 字、分成十段的短文，主題是「隔離工作目錄」。不要讀取、建立或修改任何檔案，直接在回覆中輸出全文。")
ap.add_argument("--await-active",type=float,default=180); ap.add_argument("--estop-after",type=float,default=3)
a=ap.parse_args(); out=a.out; os.makedirs(out,exist_ok=True)
home=os.path.join(out,"home"); os.makedirs(os.path.join(home,"config"),exist_ok=True)
open(os.path.join(home,"config","interaction.yaml"),"w").write(f"apiHost: 127.0.0.1\napiPort: {a.port}\n")
work=os.path.join(out,"workdir"); os.makedirs(work,exist_ok=True)
open(os.path.join(work,"NOTES.md"),"w").write("# 階段零 A-07\n\n隔離工作目錄。\n")
base=f"http://127.0.0.1:{a.port}"; log={"case":"A-07","agent":a.agent,"port":a.port,"events":[],"daemonPids":[]}
def now(): return time.strftime("%Y-%m-%dT%H:%M:%S",time.gmtime())+f".{int(time.time()*1000)%1000:03d}Z"
T0=time.monotonic()
def mark(k,**kw):
    r={"t":now(),"mono":round(time.monotonic()-T0,3),"k":k,**kw}; log["events"].append(r); print(json.dumps(r,ensure_ascii=False)[:700],flush=True); return r
def ready():
    try: return urllib.request.urlopen(f"{base}/v1/ready",timeout=1).status==200
    except Exception: return False
DPID=None
def spawn(tag):
    global DPID
    env=dict(os.environ,INTERACT_AI_HOME=home,INTERACT_AI_MOBILE_ADVERTISE="0")
    for k in list(env):
        if k.startswith("INTERACT_AI_CLAUDE_BIN") or k.startswith("INTERACT_AI_CODEX_BIN"): env.pop(k)
    dlog=open(os.path.join(out,f"daemon-{tag}.log"),"ab")
    p=subprocess.Popen([BIN,"serve"],env=env,stdout=dlog,stderr=subprocess.STDOUT,stdin=subprocess.DEVNULL,start_new_session=True)
    DPID=p.pid; log["daemonPids"].append({"tag":tag,"pid":p.pid}); mark("daemon-spawned",tag=tag,pid=p.pid)
    for _ in range(160):
        if ready(): break
        time.sleep(0.25)
    else: mark("daemon-not-ready",tag=tag); sys.exit(2)
    mark("daemon-ready",tag=tag); return p
def tok(name="api-token"): return open(os.path.join(home,"state",name)).read().strip()
def api(method,path,body=None,token=None):
    t=token if token is not None else tok()
    req=urllib.request.Request(base+path,method=method,headers={"Authorization":f"Bearer {t}","content-type":"application/json"},data=(json.dumps(body).encode() if body is not None else None))
    try:
        with urllib.request.urlopen(req,timeout=30) as r: return r.status,json.loads(r.read() or b"null")
    except urllib.error.HTTPError as e:
        try: return e.code,json.loads(e.read() or b"null")
        except Exception: return e.code,None
    except Exception as e: return -1,{"transport_error":str(e)}
def sse_thread(tag,token):
    path=os.path.join(out,f"sse-{tag}.jsonl")
    def run():
        req=urllib.request.Request(base+"/v1/events",headers={"Authorization":f"Bearer {token}","accept":"text/event-stream"})
        try:
            with urllib.request.urlopen(req,timeout=900) as r, open(path,"a") as f:
                ev={}
                for raw in r:
                    line=raw.decode("utf-8","replace").rstrip("\n")
                    if line=="":
                        if ev: ev["_t"]=now(); f.write(json.dumps(ev,ensure_ascii=False)+"\n"); f.flush(); ev={}
                    elif ":" in line:
                        k,v=line.split(":",1); ev.setdefault(k.strip(),""); ev[k.strip()]+=v.strip()
        except Exception as e:
            open(path,"a").write(json.dumps({"_sse_error":str(e),"_t":now()})+"\n")
    threading.Thread(target=run,daemon=True).start()
def kids():
    o=subprocess.run(["bash","-lc","ps -axo pid,ppid,etime,stat,command"],capture_output=True,text=True).stdout.splitlines()
    pids={DPID}; mine=[]
    for _ in range(4):
        for l in o[1:]:
            p=l.split(None,4)
            if len(p)>=5 and p[1].isdigit() and int(p[1]) in pids and int(p[0]) not in pids:
                pids.add(int(p[0])); mine.append(l[:150])
    return mine
d1=spawn("d1"); sse_thread("d1",tok()); time.sleep(0.4)
s,st0=api("GET","/v1/status"); mark("status-before",status=s,emergencyStop=(st0 or {}).get("emergencyStop")); log["statusBefore"]=st0
payload={"agentId":a.agent,"label":"A-07-estop","workdir":work,"ttlMinutes":25,"maxMessages":40,"allowWrite":False,"toolScope":[],"consentScope":[]}
s,rec=api("POST","/v1/agent-sessions",payload); mark("create",status=s,state=(rec or {}).get("state"),id=(rec or {}).get("sessionId"))
if s>=400: log["createError"]=rec; json.dump(log,open(os.path.join(out,"result.json"),"w"),ensure_ascii=False,indent=1); d1.terminate(); sys.exit(3)
sid=rec["sessionId"]; log["sessionId"]=sid; log["createRecord"]=rec
s,_=api("POST",f"/v1/agent-sessions/{sid}/messages",{"kind":"task","body":{"task":a.task}}); mark("task-sent",status=s)
TERM={"claimed-completed","failed","timed-out","cancelled","expired","closed","unknown"}
last=None; t=time.monotonic(); cur=None
while time.monotonic()-t<a.await_active:
    s,cur=api("GET",f"/v1/agent-sessions/{sid}"); stt=(cur or {}).get("state")
    if stt!=last: mark("state",state=stt); last=stt
    if stt=="active" or stt in TERM: break
    time.sleep(0.5)
mark("pre-estop",state=last,children=kids()); log["preEstop"]=cur
if last=="active": time.sleep(a.estop_after)
# ---- ESTOP (human token)
s,es=api("POST","/v1/emergency-stop",{}); mark("emergency-stop",status=s,resp=json.loads(json.dumps(es))if es else None); log["estopResp"]=es
time.sleep(1.0)
s,cur=api("GET",f"/v1/agent-sessions/{sid}"); mark("session-after-estop",status=s,state=(cur or {}).get("state"),detail=(cur or {}).get("detail")); log["afterEstop"]=cur
s,st1=api("GET","/v1/status"); mark("status-after-estop",emergencyStop=(st1 or {}).get("emergencyStop")); log["statusAfterEstop"]=st1
mark("children-after-estop",children=kids())
time.sleep(2); mark("children-after-estop-3s",children=kids())
s,fm=api("GET",f"/v1/agent-sessions/{sid}/messages?direction=from-session"); log["fromSessionAfterEstop"]=fm; mark("from-session-after-estop",count=len(fm or []))
# new session must be refused
s,r=api("POST","/v1/agent-sessions",dict(payload,label="A-07-after-estop")); mark("create-while-estopped",status=s,resp=r); log["createWhileEstopped"]={"status":s,"resp":r}
# AI (restricted agent token) must NOT be able to clear
try: atok=tok("api-agent-token")
except Exception as e: atok=None; mark("agent-token-missing",err=str(e))
if atok:
    s,r=api("POST","/v1/emergency-stop/clear",{},token=atok); mark("agent-clear-attempt",status=s,resp=r); log["agentClear"]={"status":s,"resp":r}
    s,r=api("GET","/v1/status",token=atok); mark("status-via-agent-token",status=s,emergencyStop=(r or {}).get("emergencyStop"))
# ---- restart
mark("sigterm-daemon",pid=d1.pid); d1.terminate()
try: d1.wait(timeout=20); mark("daemon-exited",code=d1.returncode)
except subprocess.TimeoutExpired: d1.kill(); mark("daemon-killed-after-timeout")
time.sleep(1.5); mark("ready-after-stop",ready=ready())
d2=spawn("d2"); sse_thread("d2",tok()); time.sleep(0.4)
s,st2=api("GET","/v1/status"); mark("status-after-restart",status=s,emergencyStop=(st2 or {}).get("emergencyStop")); log["statusAfterRestart"]=st2
s,cur2=api("GET",f"/v1/agent-sessions/{sid}"); mark("session-after-restart",status=s,state=(cur2 or {}).get("state"),detail=(cur2 or {}).get("detail")); log["sessionAfterRestart"]=cur2
s,r=api("POST","/v1/agent-sessions",dict(payload,label="A-07-after-restart")); mark("create-after-restart",status=s,resp=r); log["createAfterRestart"]={"status":s,"resp":r}
# ---- human clear
s,r=api("POST","/v1/emergency-stop/clear",{}); mark("human-clear",status=s,resp=r); log["humanClear"]={"status":s,"resp":r}
s,st3=api("GET","/v1/status"); mark("status-after-clear",emergencyStop=(st3 or {}).get("emergencyStop")); log["statusAfterClear"]=st3
s,r=api("POST","/v1/agent-sessions",dict(payload,label="A-07-after-clear")); mark("create-after-clear",status=s,state=(r or {}).get("state"),id=(r or {}).get("sessionId")); log["createAfterClear"]={"status":s,"resp":r}
if s<400 and r and r.get("sessionId"):
    s2,r2=api("POST",f"/v1/agent-sessions/{r['sessionId']}/close",{"reason":"A-07 cleanup"}); mark("close-probe-session",status=s2,state=(r2 or {}).get("state"))
s,aud=api("GET","/v1/audit"); log["auditTail"]=aud
mark("children-final",children=kids())
d2.terminate()
try: d2.wait(timeout=20)
except subprocess.TimeoutExpired: d2.kill()
mark("daemon-stopped",tag="d2")
log["wallSeconds"]=round(time.monotonic()-T0,2)
json.dump(log,open(os.path.join(out,"result.json"),"w"),ensure_ascii=False,indent=1)
print("RESULT",json.dumps({"estopStatus":log.get("statusAfterEstop",{}).get("emergencyStop"),"afterRestart":log.get("statusAfterRestart",{}).get("emergencyStop"),"sessionAfterEstop":(log.get("afterEstop") or {}).get("state")},ensure_ascii=False))
