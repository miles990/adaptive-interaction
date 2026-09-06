#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TEMP="$(mktemp -d "${TMPDIR:-/tmp}/semantic-state-swift.XXXXXX")"
trap 'rm -rf "$TEMP"' EXIT
echo "Evidence: native macOS Swift pure-model fixture runner; not iOS simulator or real-device acceptance"
if ! command -v xcrun >/dev/null 2>&1; then
  echo "needs-environment: native Swift runner requires macOS Xcode command-line tools (xcrun)" >&2
  exit 2
fi
if ! xcrun --find swiftc >/dev/null 2>&1; then
  if [[ -z "${DEVELOPER_DIR:-}" && -d /Applications/Xcode.app/Contents/Developer ]]; then
    export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
    echo "Using installed Xcode for this process: $DEVELOPER_DIR"
  else
    echo "needs-environment: selected developer tools cannot locate swiftc; check DEVELOPER_DIR/xcode-select" >&2
    exit 2
  fi
fi
xcrun swiftc --version
xcrun swiftc -module-cache-path "$TEMP/modules" \
  "$ROOT"/apps/interaction-ios/InteractionCompanion/Models/*.swift \
  "$ROOT/scripts/tests/semantic-state-conformance.swift" -o "$TEMP/conformance"
"$TEMP/conformance" "$ROOT"
