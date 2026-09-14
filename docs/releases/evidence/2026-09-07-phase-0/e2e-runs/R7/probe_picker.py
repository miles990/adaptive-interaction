#!/usr/bin/env python3
"""Instrumented probe: isolate whether the AX Open-panel selection or the
import handler is at fault. Repo files are NOT modified; this lives in the
run output directory. Isolated home + assigned port only."""
import json, os, pathlib, signal, subprocess, sys, time, urllib.request

ROOT = pathlib.Path('/Users/user/Workspace/claude-lab/adaptive-interaction')
AX = ROOT/'scripts/lib/tauri-ax.applescript'
OUT = pathlib.Path(sys.argv[1]); OUT.mkdir(parents=True, exist_ok=True)
PORT = int(sys.argv[2])
IMPORT_NAME = sys.argv[3] if len(sys.argv)>3 else '還原名稱'
IMPORT_EXPR = sys.argv[4] if len(sys.argv)>4 else 'natural'
IMPORT_SCHEMA = int(sys.argv[5]) if len(sys.argv)>5 else 1
APP = ROOT/'apps/interaction-desktop/src-tauri/target/release/bundle/macos/interaction-control-center.app'
BIN = APP/'Contents/MacOS/interaction-desktop'
CLI = ROOT/'target/debug/interact-ai'
home = OUT/'home'
(home/'config').mkdir(parents=True, exist_ok=True); (home/'state').mkdir(parents=True, exist_ok=True)
(home/'config/interaction.yaml').write_text(f'apiHost: 127.0.0.1\napiPort: {PORT}\n')
env = {**os.environ, 'INTERACT_AI_HOME':str(home), 'INTERACT_AI_MOBILE_ADVERTISE':'0'}
log = (OUT/'processes.log').open('w')
trace = []
app = daemon = None
token = ''

def api(path, body=None):
    r = urllib.request.Request(f'http://127.0.0.1:{PORT}'+path,
        data=json.dumps(body).encode() if body is not None else None,
        headers={'Authorization':'Bearer '+token,'Content-Type':'application/json'})
    with urllib.request.urlopen(r, timeout=6) as resp: return json.load(resp)

def wait(check, seconds=25):
    end = time.monotonic()+seconds
    last=None
    while time.monotonic() < end:
        try:
            v = check()
            if v: return v
        except Exception as e: last=str(e)
        time.sleep(.15)
    raise AssertionError(f'timeout ({last})')

def ax(*args, timeout=25):
    t=time.monotonic()
    r=subprocess.run(['osascript',str(AX),f'pid:{app.pid}',*args],capture_output=True,text=True,timeout=timeout)
    trace.append({'cmd':list(args),'exit':r.returncode,'seconds':round(time.monotonic()-t,3),'out':r.stdout.strip()[:400],'err':r.stderr.strip()[:400]})
    if r.returncode: raise AssertionError(r.stderr.strip())
    return r.stdout.strip()

def panel_dump(tag):
    script = '''on run argv
    set pidv to (item 1 of argv) as integer
    tell application "System Events"
      set p to first process whose unix id is pidv
      tell p
        set out to ""
        repeat with w in windows
          set out to out & "WINDOW: " & (name of w) & " subrole=" & (subrole of w) & linefeed
        end repeat
        if exists window "Open" then
          tell window "Open"
            set out to out & "OPEN-PANEL children:" & linefeed
            repeat with e in UI elements
              set lbl to ""
              try
                set lbl to (name of e) as text
              end try
              set out to out & "  " & (role of e) & " | " & lbl & linefeed
            end repeat
            if exists sheet 1 then
              set out to out & "SHEET present" & linefeed
              repeat with e in UI elements of sheet 1
                set lbl to ""
                try
                  set lbl to (name of e) as text
                end try
                set vv to ""
                try
                  set vv to (value of e) as text
                end try
                set out to out & "  SHEET " & (role of e) & " | " & lbl & " | " & vv & linefeed
              end repeat
            end if
            try
              set out to out & "TITLE-STATIC: " & (value of static text 1) & linefeed
            end try
          end tell
        end if
        return out
      end tell
    end tell
    end run'''
    p = OUT/'panel.applescript'; p.write_text(script)
    r = subprocess.run(['osascript',str(p),str(app.pid)],capture_output=True,text=True,timeout=40)
    (OUT/f'panel-{tag}.txt').write_text(r.stdout)
    return r.stdout

def prefs(): return json.loads((home/'state/desktop.json').read_text())

try:
    daemon = subprocess.Popen([str(CLI),'serve','--host','127.0.0.1','--port',str(PORT)],env=env,stdout=log,stderr=log)
    print('daemon pid', daemon.pid, flush=True)
    wait(lambda: (home/'state/api-token').exists())
    token=(home/'state/api-token').read_text().strip()
    wait(lambda: api('/ready'))
    api('/v1/onboarding/commit', {})
    (home/'state/desktop.json').write_text(json.dumps({'schemaVersion':3,'companionPack':'plain-text',
        'companionName':'PROBE-START','companionOpacity':.67,'companionVisible':True,'openControlCenterOnStart':True}))
    imp = pathlib.Path(os.environ.get('IMPORT_PATH', str(OUT/'valid-import.json')))
    imp.parent.mkdir(parents=True, exist_ok=True)
    imp.write_text(json.dumps({'kind':os.environ.get('IMPORT_KIND','companion-settings'),'schemaVersion':IMPORT_SCHEMA,'companionName':IMPORT_NAME,
        'companionPack':'plain-text','characterId':'plain-text','companionPersona':'','companionExpressiveness':IMPORT_EXPR,
        'companionScene':'','companionPlay':True,'companionCursorPlay':True,'companionApproach':True,
        'companionDeskMove':True,'companionFamiliars':[]},ensure_ascii=False,indent=2)+'\n')
    app = subprocess.Popen([str(BIN)],env=env,stdout=log,stderr=log)
    print('app pid', app.pid, flush=True)
    wait(lambda: 'Interaction Control Center' in ax('windows'))
    wait(lambda: ax('exists','AXButton','現在')=='yes')
    ax('navclick','2')
    wait(lambda: ax('exists','AXDisclosureTriangle','更換或加入角色')=='yes')
    ax('click','AXDisclosureTriangle','更換或加入角色')
    time.sleep(1)
    ax('click','AXButton','選擇角色設定檔')
    wait(lambda: 'Open' in ax('windows'))
    panel_dump('before-choosefile')
    ax('choosefile',str(imp))
    time.sleep(1)
    panel_dump('after-choosefile')
    ok=False
    for i in range(20):
        try:
            if prefs().get('companionName')==IMPORT_NAME: ok=True; break
        except Exception: pass
        time.sleep(1)
    print('imported:', ok, 'name:', repr(prefs().get('companionName')), 'expr:', prefs().get('companionExpressiveness'), 'wanted:', repr(IMPORT_NAME), IMPORT_EXPR, flush=True)
    panel_dump('final')
    (OUT/'main-window-final.txt').write_text(ax('dump'))
    (OUT/'prefs-final.json').write_text(json.dumps(prefs(),ensure_ascii=False,indent=2))
    (OUT/'trace.json').write_text(json.dumps(trace,ensure_ascii=False,indent=2))
finally:
    for p in (app,daemon):
        if p and p.poll() is None:
            p.send_signal(signal.SIGTERM)
            try: p.wait(timeout=10)
            except subprocess.TimeoutExpired: p.kill()
    log.close()
    (OUT/'trace.json').write_text(json.dumps(trace,ensure_ascii=False,indent=2))
    for n in ('api-token','api-agent-token'): (home/'state'/n).unlink(missing_ok=True)
