import json,urllib.request,urllib.error,time
R="/private/tmp/claude-501/-Users-user-Workspace-claude-lab-adaptive-interaction/95db6375-b13e-4494-9180-4b15da263657/scratchpad/e2e2/runs/R4"
base="http://127.0.0.1:19043"
human=open(f"{R}/c-claude-write/home/state/api-token").read().strip()
agent=open(f"{R}/c-claude-write/home/state/api-agent-token").read().strip()
SID_CLOSED="asession-e1e8ce07-aa6f-454a-8bc1-d3e2d0d63bca"
SID_RESUME="asession-1e51a3d0-474d-46f6-ad78-c3c64a23e7a0"
work=f"{R}/c-claude-write/workdir"
out=[]
def api(m,p,b=None,tok=agent):
    req=urllib.request.Request(base+p,method=m,headers={"Authorization":f"Bearer {tok}","content-type":"application/json"},data=(json.dumps(b).encode() if b is not None else None))
    try:
        with urllib.request.urlopen(req,timeout=30) as r: return r.status,json.loads(r.read() or b"null")
    except urllib.error.HTTPError as e:
        try: return e.code,json.loads(e.read() or b"null")
        except Exception: return e.code,None
def step(n,m,p,b=None,tok=agent,who="agent"):
    s,r=api(m,p,b,tok); out.append({"step":n,"who":who,"method":m,"path":p,"body":b,"status":s,"resp":r,"t":time.strftime("%FT%TZ",time.gmtime())})
    print(json.dumps({"step":n,"who":who,"status":s,"resp":json.dumps(r,ensure_ascii=False)[:200]},ensure_ascii=False),flush=True); return s,r
print("agent token prefix:",agent[:12],"| human prefix:",human[:12],flush=True)
step("agent-approve-closed","POST",f"/v1/agent-sessions/{SID_CLOSED}/approve",{"requestId":"0","approve":True})
step("agent-approve-resume","POST",f"/v1/agent-sessions/{SID_RESUME}/approve",{"requestId":"0","approve":True})
step("agent-create-session","POST","/v1/agent-sessions",{"agentId":"claude-code","label":"agent-token-should-fail","workdir":work,"ttlMinutes":10,"allowWrite":True,"toolScope":["workspace.write"],"consentScope":["agent-session:workspace-write"]})
step("agent-policy-patch","PATCH","/v1/policy",{"delegation":{"maxMessagesPerSession":9999}})
step("agent-estop-clear","POST","/v1/emergency-stop/clear")
step("agent-verify","POST",f"/v1/agent-sessions/{SID_RESUME}/verify",{"note":"agent should not verify"})
step("agent-get-session","GET",f"/v1/agent-sessions/{SID_RESUME}")
step("agent-renew","POST",f"/v1/agent-sessions/{SID_RESUME}/renew",{"extraMinutes":60})
step("agent-onboarding-commit","POST","/v1/onboarding/commit",{"enableActuators":["actuator.builtin.notify"]})
step("agent-estop-set","POST","/v1/emergency-stop",{"reason":"agent-token estop allowed by design"})
# restore: only human can clear
step("human-estop-clear","POST","/v1/emergency-stop/clear",None,human,"human")
step("human-status-after","GET","/v1/status",None,human,"human")
json.dump(out,open(f"{R}/f-agent-token/steps.json","w"),ensure_ascii=False,indent=1)
