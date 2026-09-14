#!/usr/bin/env python3
"""Probe (not a product test): learn the Open-panel AX shape and dump costs."""
import json, os, pathlib, signal, subprocess, sys, time, urllib.request

ROOT = pathlib.Path('/Users/user/Workspace/claude-lab/adaptive-interaction')
BASE = pathlib.Path(__file__).resolve().parent
PROBE_AX = BASE/'ax_probe.applescript'
REPO_AX = ROOT/'scripts/lib/tauri-ax.applescript'
OUT = pathlib.Path(sys.argv[1]); OUT.mkdir(parents=True, exist_ok=True)
MODE = sys.argv[2] if len(sys.argv) > 2 else 'clip'
APP = ROOT/'apps/interaction-desktop/src-tauri/target/release/bundle/macos/interaction-control-center.app'
BIN = APP/'Contents/MacOS/interaction-desktop'
CLI = ROOT/'target/debug/interact-ai'
home = OUT/'home'
(home/'config').mkdir(parents=True, exist_ok=True); (home/'state').mkdir(parents=True, exist_ok=True)
import socket
with socket.socket() as s:
    s.bind(('127.0.0.1', 0)); PORT = s.getsockname()[1]
(home/'config/interaction.yaml').write_text(f'apiHost: 127.0.0.1\napiPort: {PORT}\n')
env = {**os.environ, 'INTERACT_AI_HOME': str(home), 'INTERACT_AI_MOBILE_ADVERTISE': '0'}
log = (OUT/'processes.log').open('w')
app = daemon = None
token = ''
trace = []

def api(path, body=None):
    r = urllib.request.Request(f'http://127.0.0.1:{PORT}'+path,
        data=json.dumps(body).encode() if body is not None else None,
        headers={'Authorization': 'Bearer '+token, 'Content-Type': 'application/json'})
    with urllib.request.urlopen(r, timeout=6) as resp:
        return json.load(resp)

def wait(check, seconds=25):
    end = time.monotonic()+seconds
    last = None
    while time.monotonic() < end:
        try:
            v = check()
            if v: return v
        except Exception as e: last = str(e)
        time.sleep(.15)
    raise AssertionError(f'timeout ({last})')

def run_ax(script, *args, timeout=90):
    t = time.monotonic()
    r = subprocess.run(['osascript', str(script), f'pid:{app.pid}', *args],
                       capture_output=True, text=True, timeout=timeout)
    trace.append({'script': script.name, 'cmd': list(args), 'exit': r.returncode,
                  'seconds': round(time.monotonic()-t, 3), 'err': r.stderr.strip()[:300]})
    if r.returncode: raise AssertionError(r.stderr.strip())
    return r.stdout

def probe(*args, **kw): return run_ax(PROBE_AX, *args, **kw)
def repo(*args, **kw): return run_ax(REPO_AX, *args, **kw).strip()

try:
    daemon = subprocess.Popen([str(CLI), 'serve', '--host', '127.0.0.1', '--port', str(PORT)],
                              env=env, stdout=log, stderr=log)
    wait(lambda: (home/'state/api-token').exists())
    token = (home/'state/api-token').read_text().strip()
    wait(lambda: api('/ready'))
    api('/v1/onboarding/commit', {})
    (home/'state/desktop.json').write_text(json.dumps({'schemaVersion': 3, 'companionPack': 'plain-text',
        'companionName': 'PROBE-START', 'companionOpacity': .67, 'companionVisible': True,
        'openControlCenterOnStart': True}))
    target = OUT/'zz-target-import.json'
    target.write_text(json.dumps({'kind': 'companion-settings', 'schemaVersion': 1,
        'companionName': 'PROBE-IMPORTED', 'companionPack': 'plain-text', 'characterId': 'plain-text',
        'companionPersona': '', 'companionExpressiveness': 'natural', 'companionScene': '',
        'companionPlay': True, 'companionCursorPlay': True, 'companionApproach': True,
        'companionDeskMove': True, 'companionFamiliars': []}, ensure_ascii=False, indent=2)+'\n')
    decoy = OUT/'aaa-decoy-import.json'
    decoy.write_text(json.dumps({'kind': 'companion-settings', 'schemaVersion': 1,
        'companionName': 'PROBE-DECOY', 'companionPack': 'plain-text', 'characterId': 'plain-text',
        'companionPersona': '', 'companionExpressiveness': 'natural', 'companionScene': '',
        'companionPlay': True, 'companionCursorPlay': True, 'companionApproach': True,
        'companionDeskMove': True, 'companionFamiliars': []}, ensure_ascii=False, indent=2)+'\n')
    app = subprocess.Popen([str(BIN)], env=env, stdout=log, stderr=log)
    wait(lambda: 'Interaction Control Center' in repo('windows'))
    wait(lambda: repo('exists', 'AXButton', '現在') == 'yes')
    repo('navclick', '2')
    wait(lambda: repo('exists', 'AXDisclosureTriangle', '更換或加入角色') == 'yes')
    repo('click', 'AXDisclosureTriangle', '更換或加入角色')

    # ---- dump cost comparison on the settings page
    t = time.monotonic(); slow = probe('slowdump'); slow_s = round(time.monotonic()-t, 3)
    t = time.monotonic()
    try:
        fast = probe('batchdump'); fast_s = round(time.monotonic()-t, 3); fast_err = None
    except Exception as e:
        fast, fast_s, fast_err = '', round(time.monotonic()-t, 3), str(e)
    (OUT/'dump-slow.txt').write_text(slow)
    (OUT/'dump-fast.txt').write_text(fast)
    print(f'slowdump {slow_s}s len={len(slow)} | batchdump {fast_s}s len={len(fast)} err={fast_err}', flush=True)
    print('identical:', slow == fast, flush=True)

    # ---- Open panel structure
    repo('click', 'AXButton', '選擇角色設定檔')
    wait(lambda: 'Open' in repo('windows'))
    (OUT/'panel-1-initial.txt').write_text(probe('panelfull'))
    probe('gotofolder')
    got = wait(lambda: probe('sheetexists').strip() == 'yes' or None, 8)
    (OUT/'sheet-1-empty.txt').write_text(probe('readsheet'))
    if MODE == 'clip':
        subprocess.run(['osascript', '-e', f'set the clipboard to {json.dumps(str(target))}'], check=True)
        probe('pasteclip')
    else:
        probe('typepath', str(target))
    time.sleep(0.8)
    (OUT/'sheet-2-filled.txt').write_text(probe('readsheet'))
    probe('return')
    time.sleep(1.5)
    (OUT/'panel-2-after-goto.txt').write_text(probe('panelfull'))
    print('sheet after goto:', probe('sheetexists').strip(), flush=True)
    probe('return')
    time.sleep(1.5)
    (OUT/'windows-after-open.txt').write_text(probe('windows'))
    ok = False
    for _ in range(20):
        try:
            if json.loads((home/'state/desktop.json').read_text()).get('companionName') != 'PROBE-START':
                ok = True; break
        except Exception: pass
        time.sleep(1)
    name = json.loads((home/'state/desktop.json').read_text()).get('companionName')
    print('import happened:', ok, 'name:', repr(name), '(want PROBE-IMPORTED)', flush=True)
    (OUT/'main-final.txt').write_text(probe('slowdump'))
finally:
    for p in (app, daemon):
        if p and p.poll() is None:
            p.send_signal(signal.SIGTERM)
            try: p.wait(timeout=10)
            except subprocess.TimeoutExpired: p.kill(); p.wait(timeout=5)
    log.close()
    (OUT/'trace.json').write_text(json.dumps(trace, ensure_ascii=False, indent=2))
    for n in ('api-token', 'api-agent-token'):
        (home/'state'/n).unlink(missing_ok=True)
