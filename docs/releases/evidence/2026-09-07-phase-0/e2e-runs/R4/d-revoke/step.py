import json,urllib.request,urllib.error,sys,time,os
R="/private/tmp/claude-501/-Users-user-Workspace-claude-lab-adaptive-interaction/95db6375-b13e-4494-9180-4b15da263657/scratchpad/e2e2/runs/R4"
base="http://127.0.0.1:19043"
tok=open(f"{R}/c-claude-write/home/state/api-token").read().strip()
SID="asession-e1e8ce07-aa6f-454a-8bc1-d3e2d0d63bca"
out=[]
def api(method,path,body=None,token=None):
    t=token or tok
    req=urllib.request.Request(base+path,method=method,headers={"Authorization":f"Bearer {t}","content-type":"application/json"},data=(json.dumps(body).encode() if body is not None else None))
    try:
        with urllib.request.urlopen(req,timeout=30) as r: return r.status,json.loads(r.read() or b"null")
    except urllib.error.HTTPError as e:
        try: return e.code,json.loads(e.read() or b"null")
        except Exception: return e.code,None
def step(name,method,path,body=None,token=None):
    s,b=api(method,path,body,token); rec={"step":name,"method":method,"path":path,"status":s,"resp":b,"t":time.strftime("%FT%TZ",time.gmtime())}
    out.append(rec); print(json.dumps({k:rec[k] for k in ("step","status")},ensure_ascii=False),flush=True)
    return s,b
# 1) verify
step("verify","POST",f"/v1/agent-sessions/{SID}/verify",{"note":"phase0 human verification: hello.txt exists with content hi"})
step("after-verify-get","GET",f"/v1/agent-sessions/{SID}")
step("verify-twice","POST",f"/v1/agent-sessions/{SID}/verify",{"note":"second"})
# 2) close (revoke)
step("close","POST",f"/v1/agent-sessions/{SID}/close",{"reason":"phase0 revoke test"})
step("after-close-get","GET",f"/v1/agent-sessions/{SID}")
step("post-close-message","POST",f"/v1/agent-sessions/{SID}/messages",{"kind":"task","body":{"task":"再建立 hello3.txt"}})
step("post-close-approve","POST",f"/v1/agent-sessions/{SID}/approve",{"requestId":"0","approve":True})
step("post-close-interrupt","POST",f"/v1/agent-sessions/{SID}/interrupt")
step("post-close-renew","POST",f"/v1/agent-sessions/{SID}/renew",{"extraMinutes":10})
json.dump(out,open(f"{R}/d-revoke/steps.json","w"),ensure_ascii=False,indent=1)
