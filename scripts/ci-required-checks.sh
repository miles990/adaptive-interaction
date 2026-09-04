#!/usr/bin/env bash
# 必需 CI check 清單的判定器（release-verify.sh 與 release.yml 的 gate job 共用）。
#
#   gh api --paginate "repos/{owner}/{repo}/commits/<sha>/check-runs" \
#     --jq '.check_runs[] | "\(.name)=\(.conclusion)"' | scripts/ci-required-checks.sh
#
# 必需清單直接從 .github/workflows/ci.yml 的 job 名稱推導（job 改名／新增 job 時自動跟上），
# 逐一斷言「存在且 conclusion == success」。任何一個缺席或非 success 都 exit 1 並列出原因——
# 「只有現存的 check 全綠」不算通過，因為被刪掉、被改名、被 path filter 擋掉而根本沒建立
# check-run 的 job（例如最貴的 e2e）會靜默消失。
#
#   --workflow <path>   指定 ci.yml（預設 .github/workflows/ci.yml）
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORKFLOW="${ROOT}/.github/workflows/ci.yml"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --workflow) WORKFLOW="$2"; shift 2 ;;
    -h|--help) grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown flag $1" >&2; exit 2 ;;
  esac
done

[[ -f "$WORKFLOW" ]] || { echo "ci workflow not found: $WORKFLOW" >&2; exit 1; }

# stdin 先落盤：heredoc 會佔用 python 的 stdin，check-run 清單必須另外傳。
RUNS="$(mktemp)"
trap 'rm -f "$RUNS"' EXIT
cat > "$RUNS"

python3 - "$WORKFLOW" "$RUNS" <<'PY'
import re, sys

workflow = sys.argv[1]
src = open(workflow, encoding="utf-8").read()
if "\njobs:" not in src:
    print("ci workflow has no jobs: block", file=sys.stderr)
    sys.exit(1)

# job id → 顯示名稱（check-run 的名字就是 job 的 name:，沒寫 name 時是 job id）。
required = []
current = None
for line in src.split("\njobs:", 1)[1].splitlines():
    m = re.match(r"^  ([A-Za-z0-9_-]+):\s*$", line)
    if m:
        current = [m.group(1), m.group(1)]
        required.append(current)
        continue
    m = re.match(r"^    name:\s*(.+?)\s*$", line)
    if m and current is not None:
        current[1] = m.group(1).strip().strip(chr(34)).strip(chr(39))

names = [name for _id, name in required]
if not names:
    print("cannot derive the required check list from " + workflow, file=sys.stderr)
    sys.exit(1)

seen = {}
for raw in open(sys.argv[2], encoding="utf-8").read().splitlines():
    raw = raw.strip()
    if not raw or "=" not in raw:
        continue
    name, _sep, conclusion = raw.rpartition("=")
    # 同名 check 重跑時以最後一筆為準（gh 依時間排序）。
    seen[name.strip()] = conclusion.strip()

problems = []
for name in names:
    if name not in seen:
        problems.append("缺席: " + name)
    elif seen[name] != "success":
        problems.append("%s=%s" % (name, seen[name] or "null"))

if problems:
    print("必需 CI check 未全綠 — " + "；".join(problems), file=sys.stderr)
    extra = sorted(set(seen) - set(names))
    if extra:
        print("（其他 check：" + ", ".join(extra) + "）", file=sys.stderr)
    sys.exit(1)

print("%d required checks success: %s" % (len(names), ", ".join(names)))
PY
