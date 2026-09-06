from pathlib import Path
import os,subprocess,time,json
r=Path('/tmp/n1-independent-final-verify');vitest='/Users/user/Workspace/claude-lab/adaptive-interaction/apps/interaction-desktop/node_modules/vitest/vitest.mjs'
results=[]
for phase in ['baseline','current']:
 for lang in ['ts','rust']:
  cwd=r/phase/('apps/interaction-desktop' if lang=='ts' else '')
  cmd=['node',vitest,'run','--config','vitest.independent.config.ts'] if lang=='ts' else ['cargo','test','--offline','-p','interaction-session','--test','independent_restore','--','--nocapture']
  env={**os.environ,'CARGO_TARGET_DIR':str(r/('target' if phase=='baseline' else 'target-current')),'CARGO_INCREMENTAL':'0','CARGO_BUILD_JOBS':'4'}
  log=r/f'{phase}-{lang}.log';start=time.monotonic()
  with log.open('w') as f:rc=subprocess.run(cmd,cwd=cwd,env=env,stdout=f,stderr=subprocess.STDOUT).returncode
  result={'phase':phase,'language':lang,'cwd':str(cwd),'command':cmd,'exitCode':rc,'wallSeconds':round(time.monotonic()-start,3),'log':str(log),'CARGO_TARGET_DIR':env['CARGO_TARGET_DIR'],'CARGO_INCREMENTAL':'0','CARGO_BUILD_JOBS':'4'}
  results.append(result);(r/'runs.json').write_text(json.dumps(results,indent=2)+'\n');print(json.dumps(result),flush=True)
