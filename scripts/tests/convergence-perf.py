#!/usr/bin/env python3
"""Compare two committed source trees with the same existing Chromium perf rig.
Each side has one full warmup followed by three alternating measured runs.
No builds/tests/UI automation may run concurrently. See the predeclared plan.
"""
import argparse, hashlib, io, json, os, pathlib, platform, statistics
import subprocess, tarfile, tempfile, time

ROOT = pathlib.Path(__file__).resolve().parents[2]
APP = pathlib.Path('apps/interaction-desktop')
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--baseline', required=True)
p.add_argument('--candidate', default='HEAD')
p.add_argument('--out', type=pathlib.Path, required=True)
a = p.parse_args()
a.out.mkdir(parents=True, exist_ok=False)
work = pathlib.Path(tempfile.mkdtemp(prefix='aip-perf-comparison-'))
sources = {}
for label, ref in [('baseline', a.baseline), ('candidate', a.candidate)]:
    sha = subprocess.check_output(['git', 'rev-parse', ref + '^{commit}'], cwd=ROOT, text=True).strip()
    archive = subprocess.check_output(['git', 'archive', sha, str(APP/'src'), str(APP/'scripts'),
                                      str(APP/'package.json'), str(APP/'pnpm-lock.yaml')], cwd=ROOT)
    tree = work/label
    tree.mkdir()
    with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
        # Both archives come from explicitly selected local git commits.
        tar.extractall(tree, filter='data')
    (tree/APP/'node_modules').symlink_to(ROOT/APP/'node_modules', target_is_directory=True)
    files = {str(f.relative_to(tree)): hashlib.sha256(f.read_bytes()).hexdigest()
             for f in tree.rglob('*') if f.is_file() and 'node_modules' not in f.parts}
    sources[label] = {'sha': sha, 'archiveSha256': hashlib.sha256(archive).hexdigest(),
                      'inputFiles': files, 'tree': str(tree/APP)}
    (a.out/(label+'-source.json')).write_text(json.dumps(sources[label], indent=2)+'\n')

runs = []
for sample in range(4):
    for label in ('baseline', 'candidate'):
        name = f'{label}-' + ('warmup' if sample == 0 else f'run-{sample}')
        started = time.monotonic()
        output = (a.out/(name+'.json')).resolve()
        with (a.out/(name+'.log')).open('w') as log:
            completed = subprocess.run(['node', 'scripts/shu/perf-rig.mjs', str(output)],
                cwd=sources[label]['tree'], env={**os.environ, 'PERF_SOAK_MS':'60000'}, stdout=log, stderr=log)
        row = {'label': label, 'sample': sample, 'warmup': sample == 0,
               'seconds': round(time.monotonic()-started, 3), 'exit': completed.returncode,
               'artifact': output.name}
        runs.append(row)
        (a.out/'runs.json').write_text(json.dumps(runs, indent=2)+'\n')
        print(name, 'exit', completed.returncode, row['seconds'], 's', flush=True)
        if completed.returncode:
            raise SystemExit(completed.returncode)

def field(value, path):
    for key in path.split('.'):
        value = value[key]
    return value

metrics = {}
investigate = []
for metric in ('drawRig', 'stage', 'stage.rafGap', 'inputLatencyToyGrab', 'inputLatencyGaze'):
    for statistic in ('medianMs', 'p95Ms'):
        key = metric+'.'+statistic
        values = {}
        for label in ('baseline', 'candidate'):
            samples = [field(json.loads((a.out/r['artifact']).read_text()), metric)[statistic]
                       for r in runs if r['label'] == label and not r['warmup']]
            values[label] = {'runs':samples, 'medianAcrossRuns':statistics.median(samples)}
        before, after = (values[x]['medianAcrossRuns'] for x in ('baseline','candidate'))
        delta = after-before
        metrics[key] = {**values, 'absoluteDeltaMs':delta,
                        'relativeDeltaPercent':100*delta/before if before else None}
        absolute_budget = .2 if metric in ('drawRig','stage') else 2
        if statistic == 'p95Ms' and before and delta > absolute_budget and after/before > 1.2:
            investigate.append(key)

for r in runs:
    if r['warmup']: continue
    raw = json.loads((a.out/r['artifact']).read_text())
    if raw.get('error') or raw.get('memorySoak', {}).get('error'):
        investigate.append(r['artifact']+': reported error')
    if raw['stageLoop']['skipEveryOther']:
        investigate.append(r['artifact']+': half-frame mode')
    for metric in ('inputLatencyToyGrab','inputLatencyGaze'):
        if raw[metric]['confirmedFrames'] != raw[metric]['attempts']:
            investigate.append(r['artifact']+': '+metric+' incomplete')
    if raw['memorySoak']['deltaAfterGcBytes'] > 1048576:
        investigate.append(r['artifact']+': soak growth > 1 MiB')
    bounded = raw['memorySoak'].get('appLayer', {}).get('bounded', {})
    if bounded.get('eventRing', 0) > bounded.get('eventRingMax', 0):
        investigate.append(r['artifact']+': event ring overflow')
    if not raw['longRun']['allFinite'] or not raw['longRun']['allWithinClamp']:
        investigate.append(r['artifact']+': numeric simulation exceeded bounds')

summary = {'environment': {'platform':platform.platform(),
             'node':subprocess.check_output(['node','--version'], text=True).strip()},
           'sources': {label:{k:v for k,v in source.items() if k != 'inputFiles'} for label,source in sources.items()},
           'runs':runs, 'metrics':metrics, 'investigate':investigate,
           'method':'one full warmup per side excluded; three alternating runs; median of run medians and run p95s',
           'scope':'headless Chromium character rig/stage, not WKWebView or end-to-end latency',
           'longTermLeakConclusion':'not measured by 60-second soak'}
(a.out/'summary.json').write_text(json.dumps(summary, indent=2)+'\n')
print('comparison written', a.out/'summary.json', 'investigate:', investigate, flush=True)
# Crossing a predeclared budget requires investigation; it is not silently a pass.
raise SystemExit(1 if investigate else 0)
