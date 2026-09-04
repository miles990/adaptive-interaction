#!/usr/bin/env bash
# 已拆成三段（v0.6.0）：prepare → verify → tag。這個入口只印出流程，不再一次做完「改檔＋commit＋tag」。
#
#   scripts/release-prepare.sh <version>   # 改版本號／CHANGELOG／golden，不 commit
#   git commit -am "release: v<version>" && git push   # PR → CI 綠 → merge
#   scripts/release-verify.sh <version>    # 發布關卡（worktree、版本、CHANGELOG、secrets、codegen、CI）
#   scripts/release-tag.sh <version> --push   # 從已驗證 commit 建 annotated tag 並推送
#
# 保留 `--all-in-one <version>` 給只在本機、沒有 PR 流程的緊急情況：prepare → commit → tag（不 push、不跳過 verify）。
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; cd "$ROOT"
if [[ "${1:-}" == "--all-in-one" ]]; then
  VERSION="${2:?usage: scripts/release.sh --all-in-one <version>}"; VERSION="${VERSION#v}"
  scripts/release-prepare.sh "$VERSION"
  git add -A && git commit -m "release: v${VERSION}"
  scripts/release-verify.sh "$VERSION" --skip-ci
  git tag -a "v${VERSION}" -m "adaptive-interaction v${VERSION}"
  echo "✔ tagged v${VERSION}（未 push）。發佈：git push && git push --tags"
  exit 0
fi
sed -n '2,9p' "$0" | sed 's/^# \{0,1\}//'
exit 2
