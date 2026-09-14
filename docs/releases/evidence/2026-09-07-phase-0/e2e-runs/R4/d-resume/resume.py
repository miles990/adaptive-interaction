import json,urllib.request,urllib.error,time
R="/private/tmp/claude-501/-Users-user-Workspace-claude-lab-adaptive-interaction/95db6375-b13e-4494-9180-4b15da263657/scratchpad/e2e2/runs/R4"
base="http://127.0.0.1:19043"; tok=open(f"{R}/c-claude-write/home/state/api-token").read().strip()
work=f"{R}/c-claude-write/workdir"; PSID="e7b6974f-b93a-4774-80df-d37b6cb59dfd"
out=[]
def api(m,p,b=None):
    req=urllib.request.Request(base+p,method=m,headers={"Authorization":f"Bearer {tok}","content-type":"application/json"},data=(json.dumps(b).encode() if b is not None else None))
    try:
        with urllib.request.urlopen(req,timeout=60) as r: return r.status,json.loads(r.read() or b"null")
    except urllib.error.HTTPError as e:
        try: return e.code,json.loads(e.read() or b"null")
        except Exception: return e.code,None
def step(n,m,p,b=None):
    s,r=api(m,p,b); out.append({"step":n,"method":m,"path":p,"body":b,"status":s,"resp":r,"t":time.strftime("%FT%TZ",time.gmtime())})
    print(json.dumps({"step":n,"status":s},ensure_ascii=False),flush=True); return s,r
# widening attempt first (cheap, should be rejected before any provider spawn): ttl wider
step("resume-wider-ttl","POST","/v1/agent-sessions",{"agentId":"claude-code","label":"E0-10-resume-wider-ttl","workdir":work,"ttlMinutes":120,"maxMessages":40,"allowWrite":True,"toolScope":["workspace.write"],"consentScope":["agent-session:workspace-write"],"resumeProviderSessionId":PSID})
step("resume-wider-messages","POST","/v1/agent-sessions",{"agentId":"claude-code","label":"E0-10-resume-wider-msgs","workdir":work,"ttlMinutes":20,"maxMessages":200,"allowWrite":True,"toolScope":["workspace.write"],"consentScope":["agent-session:workspace-write"],"resumeProviderSessionId":PSID})
step("resume-wrong-workdir","POST","/v1/agent-sessions",{"agentId":"claude-code","label":"E0-10-resume-otherdir","workdir":f"{R}/b-claude/workdir","ttlMinutes":20,"maxMessages":40,"allowWrite":False,"toolScope":[],"consentScope":[],"resumeProviderSessionId":PSID})
step("resume-unknown-psid","POST","/v1/agent-sessions",{"agentId":"claude-code","label":"E0-10-resume-unknown","workdir":work,"ttlMinutes":20,"maxMessages":40,"allowWrite":False,"toolScope":[],"consentScope":[],"resumeProviderSessionId":"00000000-0000-0000-0000-000000000000"})
# narrower read-only resume: should be allowed, then the write task must be blocked
s,rec=step("resume-readonly-create","POST","/v1/agent-sessions",{"agentId":"claude-code","label":"E0-10-resume-readonly","workdir":work,"ttlMinutes":20,"maxMessages":40,"allowWrite":False,"toolScope":[],"consentScope":[],"resumeProviderSessionId":PSID})
if s<400:
    sid=rec["sessionId"]
    step("resume-task","POST",f"/v1/agent-sessions/{sid}/messages",{"kind":"task","body":{"task":"請在目前工作目錄建立檔案 hello2.txt，內容為 hi2。"}})
    t0=time.time(); last=None
    while time.time()-t0<150:
        s2,cur=api("GET",f"/v1/agent-sessions/{sid}")
        st=(cur or {}).get("state")
        if st!=last: print(json.dumps({"state":st,"mono":round(time.time()-t0,1)}),flush=True); last=st
        if st in ("claimed-completed","failed","timed-out","cancelled","expired","closed","unknown"): break
        time.sleep(1)
    step("resume-final","GET",f"/v1/agent-sessions/{sid}")
    step("resume-msgs","GET",f"/v1/agent-sessions/{sid}/messages?direction=from-session")
json.dump(out,open(f"{R}/d-resume/steps.json","w"),ensure_ascii=False,indent=1)
