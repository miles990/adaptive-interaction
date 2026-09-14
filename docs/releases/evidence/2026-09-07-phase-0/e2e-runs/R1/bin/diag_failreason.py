#!/usr/bin/env python3
"""DIAGNOSTIC (separate from the first-failure record): after a Claude interrupt -> failed,
is the reason retrievable anywhere via the API? Checks record.detail, mailbox, /v1/observations/query, /v1/audit."""
import json,os,subprocess,sys,time,urllib.request,urllib.error
BIN=os.environ.get("INTERACT_AI_BIN","/Users/user/Workspace/claude-lab/adaptive-interaction/target/debug/interact-ai")
out=sys.argv[1]; port=int(sys.argv[2]); os.makedirs(out,exist_ok=True)
home=os.path.join(out,"home"); os.makedirs(os.path.join(home,"config"),exist_ok=True)
open(os.path.join(home,"config","interaction.yaml"),"w").write(f"apiHost: 127.0.0.1\napiPort: {port}\n")
work=os.path.join(out,"workdir"); os.makedirs(work,exist_ok=True)
base=f"http://127.0.0.1:{port}"; log={"events":[]}
def now(): return time.strftime("%Y-%m-%dT%H:%M:%S",time.gmtime())+"Z"
def mark(k,**kw): r={"t":now(),"k":k,**kw}; log["events"].append(r); print(json.dumps(r,ensure_ascii=False)[:600],flush=True)
def ready():
    try: return urllib.request.urlopen(f"{base}/v1/ready",timeout=1).status==200
    except Exception: return False
env=dict(os.environ,INTERACT_AI_HOME=home,INTERACT_AI_MOBILE_ADVERTISE="0")
for k in list(env):
    if k.startswith("INTERACT_AI_CLAUDE_BIN") or k.startswith("INTERACT_AI_CODEX_BIN"): env.pop(k)
d=subprocess.Popen([BIN,"serve"],env=env,stdout=open(os.path.join(out,"daemon.log"),"ab"),stderr=subprocess.STDOUT,stdin=subprocess.DEVNULL,start_new_session=True)
log["daemonPid"]=d.pid; mark("daemon-spawned",pid=d.pid)
for _ in range(160):
    if ready(): break
    time.sleep(0.25)
tok=open(os.path.join(home,"state","api-token")).read().strip()
def api(m,p,b=None):
    req=urllib.request.Request(base+p,method=m,headers={"Authorization":f"Bearer {tok}","content-type":"application/json"},data=(json.dumps(b).encode() if b is not None else None))
    try:
        with urllib.request.urlopen(req,timeout=30) as r: return r.status,json.loads(r.read() or b"null")
    except urllib.error.HTTPError as e:
        try: return e.code,json.loads(e.read() or b"null")
        except Exception: return e.code,None
s,rec=api("POST","/v1/agent-sessions",{"agentId":"claude-code","label":"diag-failreason","workdir":work,"ttlMinutes":15,"maxMessages":10,"allowWrite":False,"toolScope":[],"consentScope":[]})
sid=rec["sessionId"]; mark("create",status=s,id=sid)
api("POST",f"/v1/agent-sessions/{sid}/messages",{"kind":"task","body":{"task":"請用繁體中文寫一篇約 1500 字、分成十段的短文，主題是「隔離工作目錄」。不要讀取、建立或修改任何檔案，直接在回覆中輸出全文。"}})
t=time.monotonic(); last=None
while time.monotonic()-t<180:
    s,cur=api("GET",f"/v1/agent-sessions/{sid}"); st=cur.get("state")
    if st!=last: mark("state",state=st); last=st
    if st=="active": break
    time.sleep(0.5)
time.sleep(3)
s,r=api("POST",f"/v1/agent-sessions/{sid}/interrupt"); mark("interrupt",status=s,resp=r)
t=time.monotonic()
while time.monotonic()-t<60:
    s,cur=api("GET",f"/v1/agent-sessions/{sid}"); st=cur.get("state")
    if st!=last: mark("state",state=st); last=st
    if st in ("failed","cancelled","unknown","timed-out","closed","claimed-completed"): break
    time.sleep(0.5)
log["record"]=cur; mark("record",state=cur.get("state"),detail=cur.get("detail"))
for dirn in ("to-session","from-session"):
    s,m=api("GET",f"/v1/agent-sessions/{sid}/messages?direction={dirn}"); log["msgs-"+dirn]=m; mark("mailbox",dir=dirn,count=len(m or []))
s,obs=api("POST","/v1/observations/query",{"receptorId":"agent.session","limit":50}); log["observations"]={"status":s,"resp":obs}
mark("observations",status=s,count=(len(obs) if isinstance(obs,list) else None))
if isinstance(obs,list):
    for o in obs[-6:]: print("  OBS",json.dumps(o,ensure_ascii=False)[:600],flush=True)
s,aud=api("GET","/v1/audit"); log["audit"]=aud
hits=[x for x in (aud or []) if 'exit' in json.dumps(x,ensure_ascii=False)]
mark("audit-exit-mentions",count=len(hits))
for h in hits[:3]: print("  AUDIT",json.dumps(h,ensure_ascii=False)[:400],flush=True)
d.terminate()
try: d.wait(timeout=15)
except subprocess.TimeoutExpired: d.kill()
mark("daemon-stopped")
json.dump(log,open(os.path.join(out,"result.json"),"w"),ensure_ascii=False,indent=1)
