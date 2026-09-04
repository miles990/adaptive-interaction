#!/usr/bin/env bash
# Release step 3/3 — tag：只在 release-verify.sh 通過、且 HEAD 已在 origin 上（預設要求等於 origin/main）時，
# 從**已驗證的 commit** 建 annotated tag。預設不 push；加 --push 才推 tag（推了才會觸發 Release workflow）。
#
#   scripts/release-tag.sh 0.6.0 [--push] [--allow-branch] [--skip-ci]
set -euo pipefail

VERSION="${1:?usage: scripts/release-tag.sh <version> [--push] [--allow-branch] [--skip-ci]}"
VERSION="${VERSION#v}"; TAG="v${VERSION}"; shift || true
PUSH=0; ALLOW_BRANCH=0; VERIFY_FLAGS=()
for arg in "$@"; do
  case "$arg" in
    --push) PUSH=1 ;;
    --allow-branch) ALLOW_BRANCH=1 ;;
    --skip-ci) VERIFY_FLAGS+=("--skip-ci") ;;
    *) echo "unknown flag $arg" >&2; exit 2 ;;
  esac
done
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; cd "$ROOT"

git fetch -q origin
HEAD_SHA=$(git rev-parse HEAD)
if [[ "$ALLOW_BRANCH" == "0" ]]; then
  MAIN_SHA=$(git rev-parse origin/main)
  [[ "$HEAD_SHA" == "$MAIN_SHA" ]] || { echo "HEAD ${HEAD_SHA:0:7} != origin/main ${MAIN_SHA:0:7}; merge first (or --allow-branch)" >&2; exit 1; }
else
  git merge-base --is-ancestor "$HEAD_SHA" "origin/$(git branch --show-current)" 2>/dev/null \
    || { echo "HEAD is not pushed to origin; push first" >&2; exit 1; }
fi

scripts/release-verify.sh "$VERSION" "${VERIFY_FLAGS[@]}"

git tag -a "$TAG" -m "adaptive-interaction ${TAG}" "$HEAD_SHA"
echo "✔ tagged ${TAG} -> ${HEAD_SHA:0:7}"
if [[ "$PUSH" == "1" ]]; then
  git push origin "$TAG"
  echo "✔ pushed ${TAG}（Release workflow 會從這個 tag 建置並發布）"
else
  echo "    git push origin ${TAG}    # 推 tag 才會觸發 Release"
fi
