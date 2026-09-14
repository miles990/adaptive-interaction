import json,urllib.request,urllib.error,time
R="/private/tmp/claude-501/-Users-user-Workspace-claude-lab-adaptive-interaction/95db6375-b13e-4494-9180-4b15da263657/scratchpad/e2e2/runs/R4"
base="http://127.0.0.1:19041"; tok=open(f"{R}/b-claude/home/state/api-token").read().strip()
work=f"{R}/b-claude/workdir"; PSID="d9fbce1a-7840-4846-820e-f834795c18c2"
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
    print(json.dumps({"step":n,"status":s,"msg":((r or {}).get("error") or {}).get("message")},ensure_ascii=False),flush=True); return s,r
step("original-record","GET","/v1/agent-sessions/asession-8a415a51-9e1b-4fd5-a08c-4dc73cb99f83")
# 1. resume a read-only session asking for write permission (scopes identical, only allowWrite widened)
step("resume-allowwrite-true","POST","/v1/agent-sessions",{"agentId":"claude-code","label":"E0-10-resume-allowwrite","workdir":work,"ttlMinutes":20,"maxMessages":40,"allowWrite":True,"toolScope":[],"consentScope":[],"resumeProviderSessionId":PSID})
# 2. resume asking for write tool scope + consent
step("resume-wider-scopes","POST","/v1/agent-sessions",{"agentId":"claude-code","label":"E0-10-resume-widerscope","workdir":work,"ttlMinutes":20,"maxMessages":40,"allowWrite":True,"toolScope":["workspace.write"],"consentScope":["agent-session:workspace-write"],"resumeProviderSessionId":PSID})
# 3. resume with wider ttl only
step("resume-wider-ttl-only","POST","/v1/agent-sessions",{"agentId":"claude-code","label":"E0-10-resume-widerttl","workdir":work,"ttlMinutes":120,"maxMessages":40,"allowWrite":False,"toolScope":[],"consentScope":[],"resumeProviderSessionId":PSID})
# 4. resume with wider maxMessages only
step("resume-wider-msgs-only","POST","/v1/agent-sessions",{"agentId":"claude-code","label":"E0-10-resume-widermsgs","workdir":work,"ttlMinutes":20,"maxMessages":500,"allowWrite":False,"toolScope":[],"consentScope":[],"resumeProviderSessionId":PSID})
# 5. omitted ttl (defaults to 120 > 20)
step("resume-omit-ttl","POST","/v1/agent-sessions",{"agentId":"claude-code","label":"E0-10-resume-omitttl","workdir":work,"maxMessages":40,"allowWrite":False,"toolScope":[],"consentScope":[],"resumeProviderSessionId":PSID})
json.dump(out,open(f"{R}/d-resume2/steps.json","w"),ensure_ascii=False,indent=1)
