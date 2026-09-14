#!/usr/bin/env python3
"""Multi-session isolation baseline on ONE isolated daemon (official HTTP path).
usage: multi_session.py --agents claude-code,claude-code --port N --out DIR [--cancel-index 0] [--approve] [--timeout S]
Each session gets its own workdir with a distinct secret token in NOTES.md; task asks the agent to report the token,
so cross-talk would be visible. Cancels session[cancel-index] via interrupt at --cancel-after seconds and checks the other finishes.
"""
import argparse, json, os, subprocess, sys, time, threading, urllib.request, urllib.error
ROOT="/Users/user/Workspace/claude-lab/adaptive-interaction"; BIN=f"{ROOT}/target/debug/interact-ai"
ap=argparse.ArgumentParser(); ap.add_argument("--agents",required=True); ap.add_argument("--port",type=int,required=True); ap.add_argument("--out",required=True)
ap.add_argument("--cancel-index",type=int,default=-1); ap.add_argument("--cancel-after",type=float,default=5); ap.add_argument("--approve",action="store_true"); ap.add_argument("--timeout",type=float,default=240)
a=ap.parse_args(); out=a.out; os.makedirs(out,exist_ok=True); agents=a.agents.split(",")
home=os.path.join(out,"home"); os.makedirs(os.path.join(home,"config"),exist_ok=True)
open(os.path.join(home,"config","interaction.yaml"),"w").write(f"apiHost: 127.0.0.1\napiPort: {a.port}\n")
base=f"http://127.0.0.1:{a.port}"; T0=time.monotonic(); log={"agents":agents,"port":a.port,"events":[],"sessions":[]}
def now(): return time.strftime("%Y-%m-%dT%H:%M:%S",time.gmtime())+f".{int(time.time()*1000)%1000:03d}Z"
def mark(k,**kw): rec={"t":now(),"mono":round(time.monotonic()-T0,3),"k":k,**kw}; log["events"].append(rec); print(json.dumps(rec,ensure_ascii=False),flush=True)
def ready():
    try: return urllib.request.urlopen(f"{base}/v1/ready",timeout=1).status==200
    except Exception: return False
env=dict(os.environ,INTERACT_AI_HOME=home,INTERACT_AI_MOBILE_ADVERTISE="0"); env.pop("INTERACT_AI_CLAUDE_BIN",None); env.pop("INTERACT_AI_CODEX_BIN",None)
daemon=subprocess.Popen([BIN,"serve"],env=env,stdout=open(os.path.join(out,"daemon.log"),"ab"),stderr=subprocess.STDOUT,stdin=subprocess.DEVNULL,start_new_session=True)
for _ in range(120):
    if ready(): break
    time.sleep(0.25)
mark("daemon-ready",pid=daemon.pid)
tok=open(os.path.join(home,"state","api-token")).read().strip()
def api(method,path,body=None):
    req=urllib.request.Request(base+path,method=method,headers={"Authorization":f"Bearer {tok}","content-type":"application/json"},data=(json.dumps(body).encode() if body is not None else None))
    try:
        with urllib.request.urlopen(req,timeout=30) as r: return r.status,json.loads(r.read() or b"null")
    except urllib.error.HTTPError as e:
        try: return e.code,json.loads(e.read() or b"null")
        except Exception: return e.code,None
sse_path=os.path.join(out,"sse.jsonl"); stop=threading.Event()
def sse():
    req=urllib.request.Request(base+"/v1/events",headers={"Authorization":f"Bearer {tok}","accept":"text/event-stream"})
    try:
        with urllib.request.urlopen(req,timeout=900) as r, open(sse_path,"a") as f:
            ev={}
            for raw in r:
                if stop.is_set(): break
                line=raw.decode("utf-8","replace").rstrip("\n")
                if line=="":
                    if ev: ev["_t"]=now(); f.write(json.dumps(ev,ensure_ascii=False)+"\n"); f.flush(); ev={}
                elif ":" in line:
                    k,v=line.split(":",1); ev.setdefault(k.strip(),""); ev[k.strip()]+=v.strip()
    except Exception as e:
        open(sse_path,"a").write(json.dumps({"_sse_error":str(e)})+"\n")
threading.Thread(target=sse,daemon=True).start(); time.sleep(0.4)
sessions=[]
for i,ag in enumerate(agents):
    wd=os.path.join(out,f"work-{i}"); os.makedirs(wd,exist_ok=True); secret=f"TOKEN-{i}-{os.urandom(3).hex()}"
    open(os.path.join(wd,"NOTES.md"),"w").write(f"# 隔離目錄 {i}\n\n本目錄的識別碼是 {secret}。\n")
    s,rec=api("POST","/v1/agent-sessions",{"agentId":ag,"label":f"phase0-multi-{i}","workdir":wd,"ttlMinutes":20,"maxMessages":40})
    sid=(rec or {}).get("sessionId"); mark("create",i=i,agent=ag,status=s,sid=sid,error=None if s<400 else rec)
    sessions.append({"i":i,"agent":ag,"sid":sid,"workdir":wd,"secret":secret,"create":rec,"approved":[],"states":[]})
for sd in sessions:
    if not sd["sid"]: continue
    long = (a.cancel_index==sd["i"])
    task = ("請用繁體中文寫一篇約 1500 字、分成十段的短文，主題是「隔離工作目錄」。不要讀取、建立或修改任何檔案。" if long else
            "請讀取目前工作目錄裡的 NOTES.md，只回覆一行：『識別碼是 <檔案裡的識別碼>』。不要修改任何檔案。")
    sd["task"]=task
    s,r=api("POST",f"/v1/agent-sessions/{sd['sid']}/messages",{"kind":"task","body":{"task":task}}); mark("task-sent",i=sd["i"],status=s)
t_send=time.monotonic(); terminal={"claimed-completed","failed","timed-out","cancelled","expired","closed","unknown"}; cancelled=False
while time.monotonic()-t_send<a.timeout:
    alldone=True
    for sd in sessions:
        if not sd["sid"]: continue
        s,cur=api("GET",f"/v1/agent-sessions/{sd['sid']}"); st=(cur or {}).get("state")
        if not sd["states"] or sd["states"][-1][1]!=st: sd["states"].append((round(time.monotonic()-T0,3),st)); mark("state",i=sd["i"],state=st)
        if st=="waiting-for-consent" and a.approve:
            s3,msgs=api("GET",f"/v1/agent-sessions/{sd['sid']}/messages?direction=from-session")
            for m in (msgs or []):
                if m.get("kind")=="approval-request" and m["body"].get("requestId") not in sd["approved"]:
                    rid=m["body"]["requestId"]; s4,r4=api("POST",f"/v1/agent-sessions/{sd['sid']}/approve",{"requestId":rid,"approve":True}); sd["approved"].append(rid); mark("operator-approval",i=sd["i"],requestId=rid,summary=m["body"].get("summary"),status=s4)
        if a.cancel_index==sd["i"] and not cancelled and time.monotonic()-t_send>=a.cancel_after and st in ("active","created"):
            s2,r2=api("POST",f"/v1/agent-sessions/{sd['sid']}/interrupt"); cancelled=True; mark("cancel-issued",i=sd["i"],status=s2,resp=r2)
        if st not in terminal: alldone=False
        sd["last"]=cur
    if alldone: break
    time.sleep(0.5)
time.sleep(1.5)
for sd in sessions:
    if not sd["sid"]: continue
    s,sd["final"]=api("GET",f"/v1/agent-sessions/{sd['sid']}")
    s,sd["from"]=api("GET",f"/v1/agent-sessions/{sd['sid']}/messages?direction=from-session")
    s,sd["to"]=api("GET",f"/v1/agent-sessions/{sd['sid']}/messages?direction=to-session")
    res=[m for m in (sd["from"] or []) if m.get("kind")=="result"]; summ=(res[-1]["body"].get("summary") if res else None)
    sd["summary"]=summ; sd["ownSecretInSummary"]=bool(summ and sd["secret"] in summ)
    sd["otherSecretInSummary"]=[o["secret"] for o in sessions if o is not sd and summ and o["secret"] in summ]
    mark("final",i=sd["i"],agent=sd["agent"],state=(sd["final"] or {}).get("state"),ownSecret=sd["ownSecretInSummary"],crossTalk=sd["otherSecretInSummary"],summary=(summ or "")[:80])
s,log["list"]=api("GET","/v1/agent-sessions"); s,log["status"]=api("GET","/v1/status")
log["children"]=subprocess.run(["bash","-lc","ps -axo pid,pgid,ppid,etime,command | grep -E '(claude -p --input-format|codex app-server)' | grep -v grep | grep -v ChatGPT.app | cut -c1-120"],capture_output=True,text=True).stdout.splitlines()
mark("children-before-daemon-stop",n=len(log["children"]))
for sd in sessions:
    if sd["sid"] and (sd["final"] or {}).get("state") not in ("closed",): api("POST",f"/v1/agent-sessions/{sd['sid']}/close",{"reason":"phase0 multi cleanup"})
time.sleep(2)
daemon.terminate(); daemon.wait(timeout=15); stop.set()
log["childrenAfterStop"]=subprocess.run(["bash","-lc","ps -axo pid,pgid,ppid,etime,command | grep -E '(claude -p --input-format|codex app-server)' | grep -v grep | grep -v ChatGPT.app | cut -c1-120"],capture_output=True,text=True).stdout.splitlines()
log["sessions"]=sessions; log["wallSeconds"]=round(time.monotonic()-T0,2)
json.dump(log,open(os.path.join(out,"result.json"),"w"),ensure_ascii=False,indent=1,default=str)
print("RESULT",json.dumps([{ "i":sd["i"],"agent":sd["agent"],"state":(sd.get("final") or {}).get("state"),"own":sd.get("ownSecretInSummary"),"cross":sd.get("otherSecretInSummary")} for sd in sessions],ensure_ascii=False), "childrenAfterStop",len(log["childrenAfterStop"]))
