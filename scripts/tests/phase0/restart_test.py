#!/usr/bin/env python3
"""E0-04 core restart: open a real agent session, kill the daemon while the agent is working, restart on the same home,
and record what the runtime says about the session (expect: not completed; 'unknown'/'expired' honesty) and whether the agent child survived.
usage: restart_test.py --agent claude-code --port N --out DIR --signal KILL|TERM
"""
import argparse, json, os, signal, subprocess, sys, time, urllib.request, urllib.error
ROOT=os.environ.get("INTERACT_AI_REPO") or os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)),"..","..","..")); BIN=os.environ.get("INTERACT_AI_BIN",f"{ROOT}/target/debug/interact-ai")
ap=argparse.ArgumentParser(); ap.add_argument("--agent",required=True); ap.add_argument("--port",type=int,required=True); ap.add_argument("--out",required=True); ap.add_argument("--signal",default="KILL")
a=ap.parse_args(); out=a.out; os.makedirs(out,exist_ok=True); home=os.path.join(out,"home"); os.makedirs(os.path.join(home,"config"),exist_ok=True)
open(os.path.join(home,"config","interaction.yaml"),"w").write(f"apiHost: 127.0.0.1\napiPort: {a.port}\n")
wd=os.path.join(out,"workdir"); os.makedirs(wd,exist_ok=True); open(os.path.join(wd,"NOTES.md"),"w").write("# 重啟測試\n")
base=f"http://127.0.0.1:{a.port}"; T0=time.monotonic(); log={"agent":a.agent,"signal":a.signal,"events":[]}
def now(): return time.strftime("%Y-%m-%dT%H:%M:%S",time.gmtime())+f".{int(time.time()*1000)%1000:03d}Z"
def mark(k,**kw): rec={"t":now(),"mono":round(time.monotonic()-T0,3),"k":k,**kw}; log["events"].append(rec); print(json.dumps(rec,ensure_ascii=False),flush=True)
def ready():
    try: return urllib.request.urlopen(f"{base}/v1/ready",timeout=1).status==200
    except Exception: return False
env=dict(os.environ,INTERACT_AI_HOME=home,INTERACT_AI_MOBILE_ADVERTISE="0"); env.pop("INTERACT_AI_CLAUDE_BIN",None); env.pop("INTERACT_AI_CODEX_BIN",None)
def start():
    d=subprocess.Popen([BIN,"serve"],env=env,stdout=open(os.path.join(out,"daemon.log"),"ab"),stderr=subprocess.STDOUT,stdin=subprocess.DEVNULL,start_new_session=True)
    for _ in range(120):
        if ready(): break
        time.sleep(0.25)
    return d
def children():
    return subprocess.run(["bash","-lc","ps -axo pid,pgid,ppid,etime,stat,command | grep -E '(claude -p --input-format|codex app-server)' | grep -v grep | grep -v ChatGPT.app | cut -c1-140"],capture_output=True,text=True).stdout.splitlines()
d=start(); mark("daemon-ready",pid=d.pid); tok=open(os.path.join(home,"state","api-token")).read().strip()
def api(method,path,body=None):
    req=urllib.request.Request(base+path,method=method,headers={"Authorization":f"Bearer {tok}","content-type":"application/json"},data=(json.dumps(body).encode() if body is not None else None))
    try:
        with urllib.request.urlopen(req,timeout=30) as r: return r.status,json.loads(r.read() or b"null")
    except urllib.error.HTTPError as e:
        try: return e.code,json.loads(e.read() or b"null")
        except Exception: return e.code,None
s,rec=api("POST","/v1/agent-sessions",{"agentId":a.agent,"label":"phase0-restart","workdir":wd,"ttlMinutes":20,"maxMessages":40}); sid=rec.get("sessionId"); mark("create",status=s,sid=sid)
s,_=api("POST",f"/v1/agent-sessions/{sid}/messages",{"kind":"task","body":{"task":"請用繁體中文寫一篇約 1500 字、分成十段的短文，主題是「核心重啟」。不要讀取、建立或修改任何檔案。"}}); mark("task-sent",status=s)
st=None
for _ in range(80):
    s,cur=api("GET",f"/v1/agent-sessions/{sid}"); st=cur.get("state")
    if st=="active": break
    time.sleep(0.25)
mark("state-before-kill",state=st,children=children())
os.kill(d.pid, signal.SIGKILL if a.signal=="KILL" else signal.SIGTERM); 
try: d.wait(timeout=20)
except subprocess.TimeoutExpired: pass
mark("daemon-killed",exitcode=d.returncode); time.sleep(2); mark("children-after-daemon-death",children=children())
time.sleep(3); mark("children-after-daemon-death+5s",children=children())
d2=start(); mark("daemon-restarted",pid=d2.pid)
tok=open(os.path.join(home,"state","api-token")).read().strip()
s,after=api("GET",f"/v1/agent-sessions/{sid}"); mark("session-after-restart",status=s,state=(after or {}).get("state"),detail=(after or {}).get("detail"),lease=(after or {}).get("lease"))
s,lst=api("GET","/v1/agent-sessions"); mark("list-after-restart",n=len(lst or []),states=[x.get("state") for x in (lst or [])])
s,msgs=api("GET",f"/v1/agent-sessions/{sid}/messages?direction=from-session"); mark("from-session-after-restart",kinds=[m.get("kind") for m in (msgs or [])])
s,r=api("POST",f"/v1/agent-sessions/{sid}/messages",{"kind":"task","body":{"task":"再來一次"}}); mark("send-after-restart",status=s,resp=(r if s>=400 else "accepted"))
s,r=api("POST",f"/v1/agent-sessions/{sid}/interrupt"); mark("interrupt-after-restart",status=s,resp=r)
s,r=api("POST",f"/v1/agent-sessions/{sid}/renew",{"extraMinutes":5}); mark("renew-after-restart",status=s,resp=(r if s>=400 else (r or {}).get("state")))
s,st2=api("GET","/v1/status"); log["statusAfter"]=st2
mark("children-final",children=children())
d2.terminate(); d2.wait(timeout=15); log["after"]=after; log["wallSeconds"]=round(time.monotonic()-T0,2)
json.dump(log,open(os.path.join(out,"result.json"),"w"),ensure_ascii=False,indent=1)
print("RESULT",json.dumps({"agent":a.agent,"signal":a.signal,"stateAfterRestart":(after or {}).get("state"),"childrenAfterDeath":len(log["events"][-6]["children"]) if False else None},ensure_ascii=False))
