#!/usr/bin/env bash
# 階段 0 基線的單步紀錄器：記錄精確命令、來源 commit、起訖時間、耗時、exit code 與 stdout/stderr 檔。
# 用法：PHASE0_OUT=<dir> scripts/tests/phase0/run-step.sh <name> <workdir> <cmd...>
# 每一步追加一行 JSON 到 $PHASE0_OUT/summary.jsonl；stdout/stderr 存成 <name>.stdout / <name>.stderr。
# 這支腳本只做紀錄，不解讀結果；passed/failed 數字要從各自的 stdout 讀（見 README.md）。
set -uo pipefail
OUT="${PHASE0_OUT:?set PHASE0_OUT to an evidence directory}"
mkdir -p "$OUT"
name="$1"; wd="$2"; shift 2
start=$(date -u +%FT%TZ); s=$(date +%s)
( cd "$wd" && "$@" ) >"$OUT/$name.stdout" 2>"$OUT/$name.stderr"; rc=$?
e=$(date +%s); end=$(date -u +%FT%TZ)
python3 - "$name" "$wd" "$start" "$end" $((e-s)) $rc "$OUT" "$@" <<'PY'
import json, subprocess, sys
name, wd, start, end, secs, rc, out = sys.argv[1:8]
cmd = sys.argv[8:]
sha = subprocess.run(["git", "rev-parse", "HEAD"], cwd=wd, capture_output=True, text=True).stdout.strip()
dirty = subprocess.run(["git", "status", "--porcelain"], cwd=wd, capture_output=True, text=True).stdout.strip() != ""
rec = {"name": name, "cwd": wd, "cmd": " ".join(cmd), "sourceCommit": sha, "worktreeDirty": dirty,
       "start": start, "end": end, "seconds": int(secs), "exit": int(rc),
       "stdout": f"{name}.stdout", "stderr": f"{name}.stderr"}
with open(f"{out}/summary.jsonl", "a") as f:
    f.write(json.dumps(rec, ensure_ascii=False) + "\n")
print(json.dumps(rec, ensure_ascii=False))
PY
exit $rc
