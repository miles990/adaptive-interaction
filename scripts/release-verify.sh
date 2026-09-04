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
gate() { # gate <name> <ok:0|1> <detail>
  if [[ "$2" == "0" ]]; then echo "  ✔ $1${3:+ — $3}"; else echo "  ✘ $1${3:+ — $3}"; FAIL=1; fi
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
! git grep -nE -- '-----BEGIN (RSA |EC |OPENSSH )?PRIVATE KEY-----|ghp_[A-Za-z0-9]{30,}|sk-ant-[A-Za-z0-9-]{20,}|sk-[A-Za-z0-9]{40,}' -- ':!scripts/release-verify.sh' >/dev/null 2>&1
gate "no secrets in tracked files" $?

if [[ -f scripts/aip-codegen.mjs ]]; then
  node scripts/aip-codegen.mjs --check >/dev/null 2>&1; gate "AIP generated types not drifted" $?
fi

if [[ "$SKIP_CI" == "0" ]]; then
  if command -v gh >/dev/null 2>&1; then
    SHA=$(git rev-parse HEAD)
    CONCLUSIONS=$(gh api "repos/{owner}/{repo}/commits/${SHA}/check-runs" --jq '.check_runs[] | "\(.name)=\(.conclusion)"' 2>/dev/null || echo "gh-api-failed")
    if [[ "$CONCLUSIONS" == "gh-api-failed" || -z "$CONCLUSIONS" ]]; then
      gate "CI checks for HEAD" 1 "no check-runs found for ${SHA:0:7} (push the commit and wait for CI, or --skip-ci)"
    else
      echo "$CONCLUSIONS" | grep -vqE '=success$'; NOTGREEN=$?
      [[ "$NOTGREEN" == "1" ]]; gate "CI checks for HEAD all success" $? "$(echo "$CONCLUSIONS" | tr '\n' ' ')"
    fi
  else
    gate "CI checks for HEAD" 1 "gh not installed (use --skip-ci to bypass consciously)"
  fi
fi

if [[ "$RUN_TESTS" == "1" ]]; then
  cargo fmt --all --check >/dev/null 2>&1; gate "cargo fmt" $?
  cargo clippy --workspace --all-targets -- -D warnings >/dev/null 2>&1; gate "cargo clippy" $?
  cargo test --workspace --no-fail-fast >/dev/null 2>&1; gate "cargo test --workspace" $?
  (cd apps/interaction-desktop && pnpm typecheck >/dev/null 2>&1); gate "pnpm typecheck" $?
  (cd apps/interaction-desktop && pnpm test >/dev/null 2>&1); gate "pnpm test" $?
  (cd apps/interaction-desktop && pnpm build >/dev/null 2>&1); gate "pnpm build" $?
fi

if [[ "$FAIL" == "0" ]]; then echo "✔ all gates passed for ${TAG}"; else echo "✘ gates failed for ${TAG}; do not tag" >&2; fi
exit "$FAIL"
