#!/usr/bin/env bash
# Release step 1/3 — prepare：同步四處版本號、整理 CHANGELOG 段落、重生 golden schemas。
# **不 commit、不 tag。** 之後由人（或 CI）審閱 diff → commit → push → PR → CI 綠 → merge，
# 再跑 release-verify.sh 與 release-tag.sh。
#
#   scripts/release-prepare.sh 0.6.0
set -euo pipefail

VERSION="${1:?usage: scripts/release-prepare.sh <version, e.g. 0.6.0>}"
VERSION="${VERSION#v}"
TAG="v${VERSION}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.]+)?$ ]] || { echo "bad version: $VERSION" >&2; exit 1; }
[[ -z "$(git status --porcelain)" ]] || { echo "working tree not clean; commit or stash first" >&2; exit 1; }
git rev-parse "$TAG" >/dev/null 2>&1 && { echo "tag $TAG already exists" >&2; exit 1; }

python3 - "$VERSION" <<'PY'
import json, re, sys
version = sys.argv[1]

def bump_toml(path):
    s = open(path).read()
    new, n = re.subn(r'(?m)^version = "[^"]+"$', f'version = "{version}"', s, count=1)
    assert n == 1, f"{path}: version line not found"
    open(path, "w").write(new)

bump_toml("Cargo.toml")
bump_toml("apps/interaction-desktop/src-tauri/Cargo.toml")

p = "apps/interaction-desktop/src-tauri/tauri.conf.json"
conf = json.load(open(p)); conf["version"] = version
json.dump(conf, open(p, "w"), indent=2, ensure_ascii=False); open(p, "a").write("\n")

p = "apps/interaction-desktop/package.json"
pkg = json.load(open(p)); pkg["version"] = version
json.dump(pkg, open(p, "w"), indent=2, ensure_ascii=False); open(p, "a").write("\n")
print(f"versions -> {version}")
PY

# CHANGELOG：(a) 已有 `## [<version>]` 段 → 不動；(b) 有 `## [Unreleased]` → 改名為版本段並重新放一個空的 Unreleased 標題；
# 兩者都沒有 → 失敗（不得靜默）。
python3 - "$VERSION" <<'PY'
import datetime, re, sys
version = sys.argv[1]
today = datetime.date.today().isoformat()
p = "CHANGELOG.md"
s = open(p).read()
if f"## [{version}]" in s:
    print(f"CHANGELOG: section {version} already present; left untouched")
elif "## [Unreleased]" in s:
    head, rest = s.split("## [Unreleased]", 1)
    body = rest.split("\n## [", 1)
    unreleased_body = body[0]
    if not unreleased_body.strip():
        sys.exit("CHANGELOG: the Unreleased section is empty; write the release notes before preparing")
    tail = ("\n## [" + body[1]) if len(body) > 1 else ""
    s = head + "## [Unreleased]\n\n" + f"## [{version}] - {today}" + unreleased_body + tail
    open(p, "w").write(s)
    assert f"## [{version}] - {today}" in open(p).read()
    print(f"CHANGELOG: renamed Unreleased -> {version} ({today}) and kept an empty Unreleased heading")
else:
    sys.exit(f"CHANGELOG.md has neither '## [{version}]' nor '## [Unreleased]'")
PY

cargo check --workspace -q
(cd apps/interaction-desktop/src-tauri && cargo check -q)
# Golden schemas 內嵌版本號——bump 後重生，再跑一次確認不漂移。
GOLDEN_UPDATE=1 cargo test -q -p interaction-e2e >/dev/null
cargo test -q -p interaction-e2e >/dev/null
# AIP 產生型別（TS／Swift）跟著 schema 重生。
if [[ -f scripts/aip-codegen.mjs ]]; then
  node scripts/aip-codegen.mjs >/dev/null
fi
# 執行前的鎖檔可能因版本號變動而更新。
cargo update -w -q 2>/dev/null || true

echo
echo "✔ prepared ${TAG}（未 commit、未 tag）。接下來："
echo "    git diff --stat                      # 審閱"
echo "    git commit -am \"release: ${TAG}\" && git push"
echo "    # PR → CI 綠 → merge → main CI 綠，然後："
echo "    scripts/release-verify.sh ${VERSION}"
echo "    scripts/release-tag.sh ${VERSION} [--push]"
