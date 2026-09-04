#!/usr/bin/env bash
# Release step 2/3 — verify：發布關卡（不改任何檔案、不 tag）。任一關卡不過即非 0 結束並列出原因。
#
#   scripts/release-verify.sh 0.6.0 [--run-tests] [--skip-ci]
set -uo pipefail

VERSION="${1:?usage: scripts/release-verify.sh <version> [--run-tests] [--skip-ci]}"
VERSION="${VERSION#v}"; TAG="v${VERSION}"; shift || true
RUN_TESTS=0; SKIP_CI=0
for arg in "$@"; do
  case "$arg" in
    --run-tests) RUN_TESTS=1 ;;
    --skip-ci) SKIP_CI=1 ;;
    *) echo "unknown flag $arg" >&2; exit 2 ;;
  esac
done
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; cd "$ROOT"
FAIL=0
SKIPPED=0
gate() { # gate <name> <ok:0|1> <detail>
  if [[ "$2" == "0" ]]; then echo "  ✔ $1${3:+ — $3}"; else echo "  ✘ $1${3:+ — $3}"; FAIL=1; fi
}
# 誠實階梯：沒有評估的關卡是「跳過」，不是「通過」。
skip() { # skip <name> <why>
  echo "  ⊘ $1 — SKIPPED（跳過：${2}）"; SKIPPED=$((SKIPPED + 1))
}
echo "release-verify ${TAG} @ $(git rev-parse --short HEAD)"

[[ -z "$(git status --porcelain)" ]]; gate "worktree clean" $? "$(git status --porcelain | wc -l | tr -d ' ') dirty paths"
git diff --check >/dev/null 2>&1; gate "git diff --check" $?

V_CARGO=$(grep -E '^version = ' Cargo.toml | head -1 | sed -E 's/.*"([^"]+)".*/\1/')
V_TAURI=$(grep -E '^version = ' apps/interaction-desktop/src-tauri/Cargo.toml | head -1 | sed -E 's/.*"([^"]+)".*/\1/')
V_CONF=$(python3 -c 'import json;print(json.load(open("apps/interaction-desktop/src-tauri/tauri.conf.json"))["version"])')
V_PKG=$(python3 -c 'import json;print(json.load(open("apps/interaction-desktop/package.json"))["version"])')
[[ "$V_CARGO" == "$VERSION" && "$V_TAURI" == "$VERSION" && "$V_CONF" == "$VERSION" && "$V_PKG" == "$VERSION" ]]
gate "versions in sync" $? "Cargo=$V_CARGO tauri=$V_TAURI conf=$V_CONF pkg=$V_PKG"

# release-provenance-078：release-prepare.sh 只改 [workspace.package] 的版本。任何寫死自有
# 版本的 crate 都會永遠停在舊值，而 interaction-agent-gateway 的版本會經由 clientInfo.version
# 送給外部 agent。白名單是「已知漂移」，以 ⚠ 明列——它不是通過，只是還沒修。
VERSION_DRIFT=$(python3 - "$VERSION" <<'PY'
import os, re, sys
version = sys.argv[1]
# 已知漂移：這兩個 crate 目前刻意保留自有版本號（記於 CHANGELOG 已知限制）。
known = {"interaction-adapter-declarative", "adapters-media"}
drift, known_drift = [], []
roots = []
for base in ("crates", "adapters"):
    if os.path.isdir(base):
        roots += [os.path.join(base, d) for d in sorted(os.listdir(base))]
for root in roots:
    manifest = os.path.join(root, "Cargo.toml")
    if not os.path.isfile(manifest):
        continue
    src = open(manifest, encoding="utf-8").read()
    pkg = src.split("[package]", 1)[-1].split("\n[", 1)[0]
    if re.search(r"^version\.workspace\s*=\s*true", pkg, re.M):
        continue
    m = re.search(r'^version\s*=\s*"([^"]+)"', pkg, re.M)
    literal = m.group(1) if m else "<none>"
    if literal == version:
        continue
    name = os.path.basename(root) if root.startswith("crates") else root.replace("/", "-")
    (known_drift if name in known else drift).append("%s=%s" % (manifest, literal))
if known_drift:
    print("KNOWN " + " ".join(known_drift))
if drift:
    print("DRIFT " + " ".join(drift))
sys.exit(1 if drift else 0)
PY
); DRIFT_RC=$?
gate "every crate version follows the workspace" $DRIFT_RC "$(printf '%s' "$VERSION_DRIFT" | grep '^DRIFT' | tr '\n' ' ')"
KNOWN_DRIFT=$(printf '%s' "$VERSION_DRIFT" | grep '^KNOWN' || true)
if [[ -n "$KNOWN_DRIFT" ]]; then
  # 已知限制，明說而不是靜默通過（CLAUDE.md：已知限制記在 CHANGELOG／acceptance-evidence）。
  echo "  ⚠ 已知版本漂移（尚未修，記於 CHANGELOG 已知限制）：${KNOWN_DRIFT#KNOWN }"
fi

python3 - "$VERSION" <<'PY'; gate "CHANGELOG has a non-empty [$VERSION] section" $?
import re, sys
v = sys.argv[1]; s = open("CHANGELOG.md").read()
m = re.search(rf"^## \[{re.escape(v)}\][^\n]*\n(.*?)(?=^## \[|\Z)", s, re.S | re.M)
sys.exit(0 if m and m.group(1).strip() else 1)
PY

! git rev-parse "$TAG" >/dev/null 2>&1; gate "tag $TAG does not exist yet" $?

python3 - "$VERSION" <<'PY'; gate "openapi.json info.version == $VERSION" $?
import json, sys
v = sys.argv[1]
info = json.load(open("schemas/openapi.json")).get("info", {})
sys.exit(0 if info.get("version") == v else 1)
PY

# secret 掃描（tracked 檔案）：私鑰、GitHub token、Anthropic／OpenAI key 形狀。
# 形狀：私鑰、GitHub（ghp_／gho_／ghu_／ghs_／ghr_／github_pat_）、Anthropic／OpenAI（sk-ant-／sk-proj-／sk-）、
# AWS access key（AKIA）、Google API key（AIza）、Slack token（xox[abprs]-）。掃描排除本檔與測試腳本本身。
! git grep -nE -- '-----BEGIN (RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----|gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{20,}|sk-ant-[A-Za-z0-9-]{20,}|sk-proj-[A-Za-z0-9_-]{20,}|sk-[A-Za-z0-9]{40,}|AKIA[0-9A-Z]{16}|AIza[0-9A-Za-z_-]{35}|xox[abprs]-[0-9A-Za-z-]{10,}' -- ':!scripts/release-verify.sh' ':!scripts/tests/release-scripts.sh' >/dev/null 2>&1
gate "no secrets in tracked files" $?

if [[ -f scripts/aip-codegen.mjs ]]; then
  node scripts/aip-codegen.mjs --check >/dev/null 2>&1; gate "AIP generated types not drifted" $?
fi

# evidence-honesty-012/013/014/016 + release-provenance-074/075/080：
# 文件對「程式碼現況」的可驗證陳述必須與 repo 一致。
if [[ -f scripts/tests/docs-claims.sh ]]; then
  DOCS_OUT=$(/bin/bash scripts/tests/docs-claims.sh 2>&1); DOCS_RC=$?
  gate "docs claims match the code" $DOCS_RC "$(printf '%s' "$DOCS_OUT" | tail -1)"
  [[ "$DOCS_RC" == 0 ]] || printf '%s\n' "$DOCS_OUT" | sed 's/^/    /'
else
  skip "docs claims match the code" "scripts/tests/docs-claims.sh 不存在"
fi

if [[ -f scripts/tests/release-scripts.sh ]]; then
  REL_OUT=$(/bin/bash scripts/tests/release-scripts.sh 2>&1); REL_RC=$?
  gate "release scripts + workflow self-tests" $REL_RC "$(printf '%s' "$REL_OUT" | tail -1)"
  [[ "$REL_RC" == 0 ]] || printf '%s\n' "$REL_OUT" | grep '✘' | sed 's/^/    /'
else
  skip "release scripts + workflow self-tests" "scripts/tests/release-scripts.sh 不存在"
fi

if [[ "$SKIP_CI" == "0" ]]; then
  if command -v gh >/dev/null 2>&1; then
    SHA=$(git rev-parse HEAD)
    # --paginate：check-run 預設每頁 30 筆，job 數成長時不得靜默截斷。
    CONCLUSIONS=$(gh api --paginate "repos/{owner}/{repo}/commits/${SHA}/check-runs" --jq '.check_runs[] | "\(.name)=\(.conclusion)"' 2>/dev/null || echo "gh-api-failed")
    if [[ "$CONCLUSIONS" == "gh-api-failed" || -z "$CONCLUSIONS" ]]; then
      gate "CI required checks for HEAD" 1 "no check-runs found for ${SHA:0:7} (push the commit and wait for CI, or --skip-ci)"
    else
      # 「現存的 check 全綠」不算通過：ci.yml 定義的每個 job 都必須在場且 success。
      DETAIL=$(printf '%s\n' "$CONCLUSIONS" | scripts/ci-required-checks.sh 2>&1); REQ=$?
      gate "CI required checks for HEAD all success" $REQ "$(printf '%s' "$DETAIL" | tr '\n' ' ')"
    fi
  else
    gate "CI required checks for HEAD" 1 "gh not installed (use --skip-ci to bypass consciously)"
  fi
else
  skip "CI required checks for HEAD" "--skip-ci；這個 commit 的 CI 狀態未查"
fi

if [[ "$RUN_TESTS" == "1" ]]; then
  cargo fmt --all --check >/dev/null 2>&1; gate "cargo fmt" $?
  cargo clippy --workspace --all-targets -- -D warnings >/dev/null 2>&1; gate "cargo clippy" $?
  cargo test --workspace --no-fail-fast >/dev/null 2>&1; gate "cargo test --workspace" $?
  (cd apps/interaction-desktop && pnpm typecheck >/dev/null 2>&1); gate "pnpm typecheck" $?
  (cd apps/interaction-desktop && pnpm test >/dev/null 2>&1); gate "pnpm test" $?
  (cd apps/interaction-desktop && pnpm build >/dev/null 2>&1); gate "pnpm build" $?
else
  skip "full test matrix (fmt/clippy/cargo test/typecheck/pnpm test/build)" "沒有 --run-tests；本機測試未跑"
fi

if [[ "$FAIL" != "0" ]]; then
  echo "✘ gates failed for ${TAG}; do not tag" >&2
elif [[ "$SKIPPED" == "0" ]]; then
  echo "✔ all gates passed for ${TAG}"
else
  # queued≠completed、未評估≠通過：跳過的關卡不得被寫成通過。
  echo "✔ passed-with-skips for ${TAG}：已執行的關卡全過，但有 ${SKIPPED} 個關卡被跳過（未驗證）"
  echo "    這不是完整驗證；要完整關卡請重跑不帶 --skip-ci 並加 --run-tests。"
fi
exit "$FAIL"
