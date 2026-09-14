import json,urllib.request,urllib.error,time
R="/private/tmp/claude-501/-Users-user-Workspace-claude-lab-adaptive-interaction/95db6375-b13e-4494-9180-4b15da263657/scratchpad/e2e2/runs/R4"
base="http://127.0.0.1:19045"; tok=open(f"{R}/e-ttl/home/state/api-token").read().strip()
work=f"{R}/e-ttl/workdir"; out=[]
def api(m,p,b=None):
    req=urllib.request.Request(base+p,method=m,headers={"Authorization":f"Bearer {tok}","content-type":"application/json"},data=(json.dumps(b).encode() if b is not None else None))
    try:
        with urllib.request.urlopen(req,timeout=30) as r: return r.status,json.loads(r.read() or b"null")
    except urllib.error.HTTPError as e:
        try: return e.code,json.loads(e.read() or b"null")
        except Exception: return e.code,None
def step(n,m,p,b=None):
    s,r=api(m,p,b); out.append({"step":n,"method":m,"path":p,"status":s,"resp":r,"t":time.strftime("%FT%TZ",time.gmtime())})
    print(json.dumps({"step":n,"status":s,"state":(r or {}).get("state"),"msg":((r or {}).get("error") or {}).get("message")},ensure_ascii=False),flush=True); return s,r
s,rec=step("create-ttl1","POST","/v1/agent-sessions",{"agentId":"claude-code","label":"E0-10-ttl-expire-2","workdir":work,"ttlMinutes":1,"maxMessages":10,"allowWrite":False})
sid=rec["sessionId"]; exp=rec["lease"]["expiresAt"]
print("sid",sid,"expiresAt",exp,flush=True)
deadline=time.time()+150; last=None
while time.time()<deadline:
    s2,cur=api("GET",f"/v1/agent-sessions/{sid}"); st=(cur or {}).get("state")
    if st!=last: print(json.dumps({"t":time.strftime("%FT%TZ",time.gmtime()),"state":st}),flush=True); last=st
    if st in ("expired","closed","failed","timed-out","cancelled"): break
    time.sleep(2)
step("get-after-expiry","GET",f"/v1/agent-sessions/{sid}")
step("message-after-expiry","POST",f"/v1/agent-sessions/{sid}/messages",{"kind":"task","body":{"task":"這個任務不應該被接受"}})
step("renew-after-expiry","POST",f"/v1/agent-sessions/{sid}/renew",{"extraMinutes":30})
step("interrupt-after-expiry","POST",f"/v1/agent-sessions/{sid}/interrupt")
step("verify-after-expiry","POST",f"/v1/agent-sessions/{sid}/verify",{"note":"should fail"})
json.dump({"sessionId":sid,"expiresAt":exp,"steps":out},open(f"{R}/e-ttl/ttl-steps.json","w"),ensure_ascii=False,indent=1)
