#!/usr/bin/env python3
"""Native Tauri + production external daemon recovery walkthrough (macOS AX).
Faults live in an isolated loopback proxy, never in production feature flags.
No consent, emergency-unlock, hardware pairing, or third-party messages.
Usage: python3 scripts/tests/tauri-preset-recovery.py --app PATH.app --out DIR
"""
import argparse, hashlib, http.client, http.server, json, os, pathlib, signal, socket
import subprocess, tempfile, threading, time, urllib.request, urllib.error

ROOT = pathlib.Path(__file__).resolve().parents[2]
p = argparse.ArgumentParser(__doc__)
p.add_argument('--app', type=pathlib.Path, required=True)
p.add_argument('--cli', type=pathlib.Path, default=ROOT/'target/debug/interact-ai')
p.add_argument('--out', type=pathlib.Path, required=True)
p.add_argument('--cases', nargs='*', default=['refused', 'lost-reply', 'readback-failed', 'crash-before-runtime', 'crash-after-runtime', 'cleanup-failed', 'newer-choice', 'unrelated-prefs', 'future-marker','reconnect'])
a = p.parse_args()
a.out.mkdir(parents=True, exist_ok=False)
binary = a.app.resolve()/'Contents/MacOS/interaction-desktop'
assert binary.is_file() and a.cli.is_file()
results = []

def wait(check, seconds=20):
    end = time.monotonic()+seconds
    while time.monotonic() < end:
        try:
            value = check()
            if value: return value
        except (OSError, ValueError, AssertionError, urllib.error.URLError): pass
        time.sleep(.1)
    raise AssertionError('timed out waiting for observable state')

def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1',0)); return s.getsockname()[1]

def stop(proc, crash=False):
    if proc and proc.poll() is None:
        proc.send_signal(signal.SIGKILL if crash else signal.SIGTERM)
        try: proc.wait(timeout=8)
        except subprocess.TimeoutExpired: proc.kill(); proc.wait(timeout=5)

for case in a.cases:
    started = time.monotonic()
    home = pathlib.Path(tempfile.mkdtemp(prefix='aip-native-preset-'))
    log = (a.out/(case+'.log')).open('w')
    app = daemon = server = None
    attempts = []
    try:
        real_port = free_port()
        token = ''
        state = {'mode':'healthy', 'conditionalWrites':0, 'trace':[], 'failRead':False}
        entered, release = threading.Event(), threading.Event()
        def api(path, body=None, method=None):
            data = json.dumps(body).encode() if body is not None else None
            req = urllib.request.Request(f'http://127.0.0.1:{real_port}'+path, data=data,
                headers={'Authorization':'Bearer '+token,'Content-Type':'application/json'}, method=method)
            with urllib.request.urlopen(req, timeout=6) as r: return json.load(r)
        class Proxy(http.server.BaseHTTPRequestHandler):
            def log_message(self,*_): pass
            def do_GET(self): self.forward()
            def do_POST(self): self.forward()
            def do_PATCH(self): self.forward()
            def do_DELETE(self): self.forward()
            def do_OPTIONS(self): self.forward()
            def forward(self):
                data = self.rfile.read(int(self.headers.get('Content-Length','0')))
                conditional = self.command == 'PATCH' and self.path == '/v1/proactive-dialogue' and b'operationId' in data
                mode = state['mode']
                state['trace'].append({'method':self.command,'path':self.path,'conditional':conditional,'fault':mode})
                if mode=='offline':
                    self.send_error(503); return
                if self.path == '/v1/proactive-dialogue' and self.command=='GET' and state['failRead']:
                    state['failRead']=False
                    self.send_error(503); return
                if conditional:
                    state['conditionalWrites'] += 1
                    if mode in ('refused','newer-choice','unrelated-prefs','reconnect'):
                        self.send_error(503); entered.set(); return
                    if mode=='crash-before-runtime':
                        entered.set(); release.wait(15)
                        self.close_connection=True; return
                conn = http.client.HTTPConnection('127.0.0.1',real_port, timeout=6)
                try:
                    headers = {k:v for k,v in self.headers.items() if k.lower() not in ('host','connection')}
                    conn.request(self.command,self.path,data,headers)
                    response=conn.getresponse()
                    if response.getheader('Content-Type','').startswith('text/event-stream'):
                        self.send_response(response.status)
                        for k,v in response.getheaders():
                            if k.lower() not in ('transfer-encoding','connection','content-length'): self.send_header(k,v)
                        self.end_headers()
                        while True:
                            chunk=response.read1(4096)
                            if not chunk: break
                            self.wfile.write(chunk); self.wfile.flush()
                        return
                    body=response.read()
                    if conditional:
                        if mode=='readback-failed': state['failRead']=True
                        if mode=='cleanup-failed': (home/'state').chmod(0o500)
                        if mode=='crash-after-runtime': entered.set(); release.wait(15)
                        if mode in ('lost-reply','crash-after-runtime'):
                            self.close_connection=True; return
                    self.send_response(response.status)
                    for k,v in response.getheaders():
                        if k.lower() not in ('transfer-encoding','connection','content-length'): self.send_header(k,v)
                    self.send_header('Content-Length',str(len(body))); self.end_headers()
                    self.wfile.write(body)
                except (BrokenPipeError, ConnectionResetError, TimeoutError, http.client.IncompleteRead): pass
                finally: conn.close()
        server = http.server.ThreadingHTTPServer(('127.0.0.1',0),Proxy)
        server.daemon_threads=True
        threading.Thread(target=server.serve_forever,daemon=True).start()
        (home/'config').mkdir(); (home/'state').mkdir()
        (home/'config/interaction.yaml').write_text(f'apiHost: 127.0.0.1\napiPort: {server.server_port}\n')
        env = {**os.environ,'INTERACT_AI_HOME':str(home),'INTERACT_AI_MOBILE_ADVERTISE':'0'}
        daemon = subprocess.Popen([str(a.cli.resolve()),'serve','--host','127.0.0.1','--port',str(real_port)],env=env,stdout=log,stderr=log)
        wait(lambda: (home/'state/api-token').exists())
        token=(home/'state/api-token').read_text().strip()
        wait(lambda: api('/ready'))
        api('/v1/onboarding/commit',{},'POST') # setup only: no capability/consent changes
        before = api('/v1/proactive-dialogue')
        prefs_file=home/'state/desktop.json'
        prefs_file.write_text(json.dumps({'schemaVersion':3,'companionPack':'plain-text','companionExpressiveness':'natural','companionDoNotDisturb':False,'companionVisible':True,'openControlCenterOnStart':True,'companionOpacity':.8}))
        def prefs(): return json.loads(prefs_file.read_text())
        def launch():
            proc=subprocess.Popen([str(binary)],env=env,stdout=log,stderr=log)
            wait(lambda: subprocess.run(['osascript','-e',f'tell application "System Events" to get name of (first process whose unix id is {proc.pid})'],capture_output=True,text=True).returncode==0)
            time.sleep(2)
            return proc
        def ax(*args):
            start=time.monotonic()
            r=subprocess.run(['osascript',str(ROOT/'scripts/lib/tauri-ax.applescript'),f'pid:{app.pid}',*args],capture_output=True,text=True,timeout=15)
            attempts.append({'command':args,'seconds':round(time.monotonic()-start,3),'exit':r.returncode})
            if r.returncode: raise AssertionError(r.stderr.strip())
            return r.stdout.strip()
        app=launch()
        wait(lambda: ax('navclick','2'))
        time.sleep(1)
        if case=='future-marker':
            stop(app,True); app=None
            raw={'format':99,'unknownIntent':'keep-me'}
            value=prefs(); value['companionPendingPresetOp']=raw; prefs_file.write_text(json.dumps(value))
            app=launch(); time.sleep(1)
            assert prefs()['companionPendingPresetOp']==raw
            assert api('/v1/proactive-dialogue')['config']==before['config']
            assert prefs()['companionOpacity']==.8
            ax('navclick','2')
            wait(lambda: ax('exists','AXAny','恢復標記')=='yes')
        else:
            state['mode']=case
            wait(lambda: ax('click','AXCheckBox','安靜'))
            if case.startswith('crash-'):
                assert entered.wait(8)
                assert prefs().get('companionPendingPresetOp')
                stop(app,True); app=None
                release.set()
            else:
                wait(lambda: prefs().get('companionExpressiveness')=='quiet')
                time.sleep(1)
                if case=='lost-reply':
                    wait(lambda: prefs().get('companionPendingPresetOp') is None)
                    assert api('/v1/proactive-dialogue')['config']['mode']=='necessary'
                else: assert prefs().get('companionPendingPresetOp'), 'fault must retain durable marker'
                if case=='newer-choice': api('/v1/proactive-dialogue',{'mode':'off'},'PATCH')
                if case!='reconnect': stop(app,True); app=None
            (home/'state').chmod(0o700)
            if case=='unrelated-prefs':
                value=prefs(); value['companionOpacity']=.6; prefs_file.write_text(json.dumps(value))
            if case=='reconnect':
                state['mode']='offline'; time.sleep(4)
            state['mode']='healthy'; state['failRead']=False
            writes_before=state['conditionalWrites']
            if case!='reconnect': app=launch()
            wait(lambda: prefs().get('companionPendingPresetOp') is None)
            after=api('/v1/proactive-dialogue')
            assert after['config']['mode']==('off' if case=='newer-choice' else 'necessary')
            assert prefs()['companionOpacity']==(.6 if case=='unrelated-prefs' else .8)
            for key,value in before['config'].items():
                if key!='mode': assert after['config'][key]==value, key
            if case in ('lost-reply','readback-failed','crash-after-runtime','cleanup-failed','newer-choice'):
                assert state['conditionalWrites']==writes_before, 'restart must not overwrite confirmed or newer config'
            ax('navclick','2')
            if case!='newer-choice': wait(lambda: ax('value','AXCheckBox','安靜')=='1')
        results.append({'id':case,'status':'completed','seconds':round(time.monotonic()-started,3),'evidenceLevel':'native Tauri window + production daemon, AX, isolated loopback fault proxy','effectiveConfig':api('/v1/proactive-dialogue')['config'],'prefs':prefs(),'requests':state['trace'],'ax':attempts})
    except Exception as e:
        if app and app.poll() is None:
            try: (a.out/(case+'-ax.txt')).write_text(ax('dump'))
            except Exception: pass
        results.append({'id':case,'status':'failed','seconds':round(time.monotonic()-started,3),'error':str(e),'ax':attempts})
    finally:
        (home/'state').chmod(0o700)
        release.set()
        stop(app); stop(daemon)
        if server: server.shutdown(); server.server_close()
        log.close()
        # Retain only the task-owned source data needed for diagnosis, no tokens.
        for name in ('api-token','api-agent-token'):
            (home/'state'/name).unlink(missing_ok=True)
        results[-1]['isolatedHome']=str(home)
        doc={'binarySha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'cliSha256':hashlib.sha256(a.cli.read_bytes()).hexdigest(),'humanOrHardwareEvidence':False,'results':results}
        (a.out/'result.json').write_text(json.dumps(doc,ensure_ascii=False,indent=2)+'\n')
        print(case,results[-1]['status'],results[-1].get('error',''),flush=True)
raise SystemExit(1 if any(r['status']=='failed' for r in results) else 0)
