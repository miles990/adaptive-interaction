#!/usr/bin/env bash
# 已拆成三段（v0.6.0）：prepare → verify → tag。這個入口只印出流程，不再一次做完「改檔＋commit＋tag」。
#
#   scripts/release-prepare.sh <version>   # 改版本號／CHANGELOG／golden，不 commit
#   git commit -am "release: v<version>" && git push   # PR → CI 綠 → merge
#   scripts/release-verify.sh <version>    # 發布關卡（worktree、版本、CHANGELOG、secrets、codegen、CI）
#   scripts/release-tag.sh <version> --push   # 從已驗證 commit 建 annotated tag 並推送
#
# `--all-in-one <version> --i-know-there-is-no-ci` 只給沒有 PR 流程的離線緊急情況：
# prepare → commit → verify（**帶 --skip-ci：這個 commit 的 CI 狀態完全沒有被查過**）→ tag。
# 因為 commit 從未 push，它定義上就沒有 CI 證據；tag message 會把這件事寫進去，
# Release workflow 的 ci-gate 仍會在 tag 被 push 後擋下沒有綠 CI 的 commit。
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; cd "$ROOT"
if [[ "${1:-}" == "--all-in-one" ]]; then
  VERSION="${2:?usage: scripts/release.sh --all-in-one <version> --i-know-there-is-no-ci}"; VERSION="${VERSION#v}"
  if [[ "${3:-}" != "--i-know-there-is-no-ci" ]]; then
    cat >&2 <<EOF
--all-in-one 會在一個從未 push、因此沒有任何 CI 證據的本機 commit 上建 tag：
release-verify.sh 會以 --skip-ci 執行，CI 關卡完全不評估。
確定要這樣做請明示：

    scripts/release.sh --all-in-one ${VERSION} --i-know-there-is-no-ci

正常發布請走 prepare → push → PR → CI 綠 → verify → tag。
EOF
    exit 2
  fi
  scripts/release-prepare.sh "$VERSION"
  git add -A && git commit -m "release: v${VERSION}"
  scripts/release-verify.sh "$VERSION" --skip-ci
  git tag -a "v${VERSION}" -m "adaptive-interaction v${VERSION}

evidence: NO CI — 這個 tag 由 scripts/release.sh --all-in-one 從未 push 的本機 commit 建立，
release-verify.sh 以 --skip-ci 執行，四個必需 CI job 一個都沒有查過（unverified）。"
  echo "✔ tagged v${VERSION}（未 push；tag message 已記錄「無 CI 證據」）"
  echo "  發佈：git push && git push --tags — Release workflow 的 ci-gate 會再查一次 CI，沒綠就不會發布。"
  exit 0
fi
sed -n '2,$ p' "$0" | sed -n '/^#/p' | sed 's/^# \{0,1\}//'
exit 2
