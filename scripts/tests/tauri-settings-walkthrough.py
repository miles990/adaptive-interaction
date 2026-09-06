#!/usr/bin/env python3
"""Native macOS settings export/import using the real AX file picker.
Runs with a task-owned temporary home. Downloads are copied to evidence and
only files created by this run are removed; pre-existing downloads stay intact.
"""
import argparse, hashlib, json, os, pathlib, shutil, signal, socket
import subprocess, tempfile, time, urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[2]
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--app', type=pathlib.Path, required=True)
p.add_argument('--cli', type=pathlib.Path, default=ROOT/'target/debug/interact-ai')
p.add_argument('--out', type=pathlib.Path, required=True)
a = p.parse_args()
a.out.mkdir(parents=True, exist_ok=False)
home = pathlib.Path(tempfile.mkdtemp(prefix='aip-native-settings-'))
binary = a.app.resolve()/'Contents/MacOS/interaction-desktop'
steps, commands, downloads = [], [], []
app = daemon = None
token = ''
log = (a.out/'processes.log').open('w')
env = {**os.environ, 'INTERACT_AI_HOME':str(home), 'INTERACT_AI_MOBILE_ADVERTISE':'0'}
started = time.monotonic()

def wait(check, seconds=25):
    deadline = time.monotonic()+seconds
    while time.monotonic() < deadline:
        try:
            value = check()
            if value: return value
        except (OSError, ValueError, AssertionError): pass
        time.sleep(.15)
    raise AssertionError('observable state did not arrive')

def stop(proc):
    if proc and proc.poll() is None:
        proc.send_signal(signal.SIGTERM)
        try: proc.wait(timeout=10)
        except subprocess.TimeoutExpired: proc.kill(); proc.wait(timeout=5)

def api(path, body=None):
    request = urllib.request.Request(f'http://127.0.0.1:{port}'+path,
        data=json.dumps(body).encode() if body is not None else None,
        headers={'Authorization':'Bearer '+token,'Content-Type':'application/json'})
    with urllib.request.urlopen(request, timeout=6) as response: return json.load(response)

def prefs(): return json.loads((home/'state/desktop.json').read_text())

def ax(*args):
    tick = time.monotonic()
    result = subprocess.run(['osascript',str(ROOT/'scripts/lib/tauri-ax.applescript'),
        f'pid:{app.pid}',*args],capture_output=True,text=True,timeout=20)
    commands.append({'command':args,'seconds':round(time.monotonic()-tick,3),'exit':result.returncode})
    if result.returncode: raise AssertionError(result.stderr.strip())
    return result.stdout.strip()

def launch():
    global app
    app = subprocess.Popen([str(binary)], env=env, stdout=log, stderr=log)
    wait(lambda: 'Interaction Control Center' in ax('windows'))
    wait(lambda: ax('exists','AXButton','現在') == 'yes')
    ax('navclick','2')
    wait(lambda: ax('exists','AXDisclosureTriangle','更換或加入角色') == 'yes')
    ax('click','AXDisclosureTriangle','更換或加入角色')

def restore(value, name):
    path = (a.out/(name+'.json')).resolve()
    path.write_text(json.dumps(value,ensure_ascii=False,indent=2)+'\n')
    ax('click','AXButton','選擇角色設定檔')
    wait(lambda: 'Open' in ax('windows'))
    ax('choosefile',str(path))

def note(name, status='completed', **extra):
    (a.out/(name+'-ax.txt')).write_text(ax('dump'))
    steps.append({'id':name,'status':status,'elapsedSeconds':round(time.monotonic()-started,3),**extra})
    print(name,status,flush=True)

error = None
try:
    assert binary.is_file() and a.cli.is_file()
    (home/'config').mkdir(); (home/'state').mkdir()
    with socket.socket() as sock:
        sock.bind(('127.0.0.1',0)); port=sock.getsockname()[1]
    (home/'config/interaction.yaml').write_text(f'apiHost: 127.0.0.1\napiPort: {port}\n')
    daemon = subprocess.Popen([str(a.cli.resolve()),'serve','--host','127.0.0.1','--port',str(port)],
        env=env,stdout=log,stderr=log)
    wait(lambda: (home/'state/api-token').exists())
    token=(home/'state/api-token').read_text().strip()
    wait(lambda: api('/ready'))
    api('/v1/onboarding/commit',{}) # isolated setup, no consent or capability changes
    original_config=api('/v1/proactive-dialogue')['config']
    (home/'state/desktop.json').write_text(json.dumps({'schemaVersion':3,'companionPack':'plain-text',
        'companionName':'備份走查','companionOpacity':.67,'companionVisible':True,'openControlCenterOnStart':True}))
    launch()
    directory = pathlib.Path.home()/'Downloads'
    prior = set(directory.glob('companion-settings*.json'))
    ax('click','AXButton','匯出角色設定')
    def exported():
        for path in set(directory.glob('companion-settings*.json'))-prior:
            try:
                value=json.loads(path.read_text())
                if value.get('kind')=='companion-settings' and value.get('companionName')=='備份走查':
                    downloads.append(path)
                    return path,value
            except (OSError, ValueError): pass
    download, backup=wait(exported)
    shutil.copy2(download,a.out/'exported-settings.json')
    assert backup['schemaVersion']==1 and backup['characterId']=='plain-text'
    assert set(backup) <= {'kind','schemaVersion','characterId','companionPack','companionName',
        'companionPersona','companionExpressiveness','companionScene','companionPlay','companionCursorPlay',
        'companionApproach','companionDeskMove','companionFamiliars'}
    note('export-file',bytes=download.stat().st_size,sha256=hashlib.sha256(download.read_bytes()).hexdigest())

    restore({**backup,'companionName':'還原名稱'},'valid-import')
    wait(lambda: prefs()['companionName']=='還原名稱')
    wait(lambda: ax('exists','AXAny','已匯入角色設定並套用')=='yes')
    wait(lambda: '還原名稱' in ax('windows'))
    note('restore-settings',effectiveName=prefs()['companionName'])
    stop(app); app=None
    launch()
    assert prefs()['companionName']=='還原名稱' and prefs()['companionOpacity']==.67
    note('restored-settings-after-restart')

    before=prefs()
    restore({**backup,'companionPack':'missing-import-character','characterId':'missing-import-character'},'missing-character')
    wait(lambda: ax('exists','AXAny','不是這台電腦認得的角色')=='yes')
    assert prefs()==before
    note('missing-character-import','correctly-blocked',reason='Install the missing package before importing its settings; existing data preserved')

    restore({**backup,'schemaVersion':99},'future-settings')
    wait(lambda: ax('exists','AXAny','不支援的版本')=='yes')
    assert prefs()==before
    note('future-settings-import','correctly-blocked')

    restore({**backup,'companionPlay':'false'},'invalid-settings')
    wait(lambda: ax('exists','AXAny','必須是開關值')=='yes')
    assert prefs()==before
    note('invalid-settings-import','correctly-blocked')

    (home/'state').chmod(0o500)
    restore({**backup,'companionName':'儲存失敗候選'},'unwritable-settings')
    wait(lambda: ax('exists','AXAny','無法確認設定已完整套用')=='yes')
    assert prefs()==before
    note('unwritable-preferences','correctly-blocked',effectiveName=prefs()['companionName'])
    (home/'state').chmod(0o700)
    restore({**backup,'companionName':''},'blank-name')
    wait(lambda: prefs()['companionName']=='')
    wait(lambda: ax('exists','AXAny','已匯入角色設定並套用')=='yes')
    assert prefs()['companionOpacity']==.67
    assert api('/v1/proactive-dialogue')['config']==original_config
    note('retry-and-explicit-blank-name',effectiveName=prefs()['companionName'],unrelatedOpacity=.67)
except Exception as exc:
    error=str(exc)
    steps.append({'id':'unfinished','status':'failed','error':error})
    if app and app.poll() is None:
        try: (a.out/'failed-ax.txt').write_text(ax('dump'))
        except Exception: pass
finally:
    (home/'state').chmod(0o700)
    stop(app); stop(daemon); log.close()
    for path in downloads:
        path.unlink(missing_ok=True)
    data={'evidenceLevel':'native Tauri + production daemon + real AX download and file picker',
          'sourceSha':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
          'dirtyTree':bool(subprocess.check_output(['git','status','--porcelain'],cwd=ROOT,text=True).strip()),
          'binarySha256':hashlib.sha256(binary.read_bytes()).hexdigest(),
          'cliSha256':hashlib.sha256(a.cli.read_bytes()).hexdigest(),
          'steps':steps,'ax':commands,'seconds':round(time.monotonic()-started,3),'error':error,
          'limits':'No human timing; failed presentation acknowledgement is separately covered by component fault tests; unwritable store is a real task-owned directory'}
    (a.out/'result.json').write_text(json.dumps(data,ensure_ascii=False,indent=2)+'\n')
    process_log=a.out/'processes.log'
    process_log.write_text(process_log.read_text(errors='replace').replace(token,'[redacted]') if token else process_log.read_text(errors='replace'))
    shutil.rmtree(home)
raise SystemExit(1 if error else 0)
