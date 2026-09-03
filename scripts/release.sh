#!/usr/bin/env bash
# Cut a release: sync the version across all four version-bearing files,
# refresh the lockfile, update CHANGELOG scaffolding, commit and tag.
#
#   scripts/release.sh 0.2.0
#   git push && git push --tags     # ← 這一步觸發 CI/CD 發佈
set -euo pipefail

VERSION="${1:?usage: scripts/release.sh <version, e.g. 0.2.0>}"
VERSION="${VERSION#v}"
TAG="v${VERSION}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

[[ -z "$(git status --porcelain)" ]] || { echo "working tree not clean" >&2; exit 1; }
git rev-parse "$TAG" >/dev/null 2>&1 && { echo "tag $TAG already exists" >&2; exit 1; }

python3 - "$VERSION" <<'EOF'
import json, re, sys
version = sys.argv[1]

# 1. workspace Cargo.toml
p = "Cargo.toml"
s = open(p).read()
s = re.sub(r'(?m)^version = "[^"]+"$', f'version = "{version}"', s, count=1)
open(p, "w").write(s)

# 2. src-tauri Cargo.toml
p = "apps/interaction-desktop/src-tauri/Cargo.toml"
s = open(p).read()
s = re.sub(r'(?m)^version = "[^"]+"$', f'version = "{version}"', s, count=1)
open(p, "w").write(s)

# 3. tauri.conf.json
p = "apps/interaction-desktop/src-tauri/tauri.conf.json"
conf = json.load(open(p))
conf["version"] = version
json.dump(conf, open(p, "w"), indent=2, ensure_ascii=False)

# 4. package.json
p = "apps/interaction-desktop/package.json"
pkg = json.load(open(p))
pkg["version"] = version
json.dump(pkg, open(p, "w"), indent=2, ensure_ascii=False)
print(f"versions -> {version}")
EOF

# CHANGELOG: move Unreleased into the new version section.
# 兩種合法狀態：(a) 已有 `## [<version>]` 段（整合者事先寫好，本腳本不動）；
# (b) 有 `## [Unreleased]` 標題可被轉成新版本段。兩者都沒有＝CHANGELOG 沒準備好，
# 必須失敗而不是印「成功」卻什麼都沒插入（v0.5.0 發布時 Unreleased 標題缺席，
# 這一步曾靜默跳過）。
python3 - "$VERSION" <<'EOF'
import datetime, sys
version = sys.argv[1]
today = datetime.date.today().isoformat()
p = "CHANGELOG.md"
s = open(p).read()
if f"## [{version}]" in s:
    print(f"CHANGELOG: section {version} already present; left untouched")
elif "## [Unreleased]" in s:
    s = s.replace("## [Unreleased]", f"## [Unreleased]\n\n## [{version}] - {today}", 1)
    open(p, "w").write(s)
    assert f"## [{version}] - {today}" in open(p).read(), "CHANGELOG section insertion failed"
    print(f"CHANGELOG: added section {version}")
else:
    sys.exit(
        f"CHANGELOG.md has neither '## [{version}]' nor '## [Unreleased]'; "
        "add the release section (or an Unreleased heading) before cutting the tag"
    )
EOF

cargo check --workspace -q
(cd apps/interaction-desktop/src-tauri && cargo check -q)
# Golden schemas embed the version — regenerate so they can never drift on a bump.
GOLDEN_UPDATE=1 cargo test -q -p interaction-e2e >/dev/null
cargo test -q -p interaction-e2e >/dev/null

git add -A
git commit -m "release: ${TAG}"
git tag -a "$TAG" -m "adaptive-interaction ${TAG}"
echo
echo "✔ tagged ${TAG}. 發佈："
echo "    git push && git push --tags"
