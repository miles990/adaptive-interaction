#!/usr/bin/env bash
# Real XCTest in a task-owned simulator; never boot/reset/delete the user's device.
# Usage: bash scripts/tests/ios-simulator.sh --out NEW_DIR [--build-dir OWN_CACHE]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
TASK_OUT=""; TASK_BUILD=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --out) TASK_OUT="$2"; shift 2 ;;
    --build-dir) TASK_BUILD="$2"; shift 2 ;;
    *) echo "Usage: $0 --out NEW_DIR [--build-dir OWN_CACHE]" >&2; exit 2 ;;
  esac
done
[[ -n "$TASK_OUT" && ! -e "$TASK_OUT" ]] || { echo "--out requires a new directory" >&2; exit 2; }
mkdir -p "$TASK_OUT"
TASK_OUT="$(cd "$TASK_OUT" && pwd)"
TASK_BUILD="${TASK_BUILD:-$TASK_OUT/build}"
mkdir -p "$TASK_BUILD"
TASK_BUILD="$(cd "$TASK_BUILD" && pwd)"
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
TASK_SIM="$(xcrun simctl create 'Codex convergence XCTest' com.apple.CoreSimulator.SimDeviceType.iPhone-17 com.apple.CoreSimulator.SimRuntime.iOS-26-2)"
cleanup() {
  xcrun simctl shutdown "$TASK_SIM" >/dev/null 2>&1 || true
  xcrun simctl delete "$TASK_SIM" >/dev/null 2>&1 || true
}
trap cleanup EXIT
git rev-parse HEAD > "$TASK_OUT/source-sha.txt"
git status --porcelain > "$TASK_OUT/worktree.txt"
xcodebuild -version > "$TASK_OUT/xcode-version.txt"
xcodebuild -project apps/interaction-ios/InteractionCompanion.xcodeproj -target InteractionCompanionTests \
  -configuration Debug -sdk iphonesimulator -arch arm64 CODE_SIGNING_ALLOWED=NO \
  CONFIGURATION_BUILD_DIR="$TASK_BUILD/out" OBJROOT="$TASK_BUILD/obj" SYMROOT="$TASK_BUILD/sym" build \
  > "$TASK_OUT/build.log" 2>&1
TASK_PLATFORM="$DEVELOPER_DIR/Platforms/iPhoneSimulator.platform/Developer"
TASK_FW="$TASK_BUILD/out/InteractionCompanion.app/Frameworks"
cp "$TASK_PLATFORM/usr/lib/lib_TestingInterop.dylib" "$TASK_FW/"
for TASK_FRAMEWORK in _Testing_CoreGraphics _Testing_CoreImage _Testing_Foundation _Testing_UIKit; do
  cp -R "$TASK_PLATFORM/Library/Frameworks/$TASK_FRAMEWORK.framework" "$TASK_FW/"
done
xcrun simctl boot "$TASK_SIM"
xcrun simctl bootstatus "$TASK_SIM" -b
xcrun simctl install "$TASK_SIM" "$TASK_BUILD/out/InteractionCompanion.app"
TASK_APP="$(xcrun simctl get_app_container "$TASK_SIM" dev.interact-ai.companion)"
SIMCTL_CHILD_DYLD_INSERT_LIBRARIES='@executable_path/Frameworks/libXCTestBundleInject.dylib' \
  SIMCTL_CHILD_XCInjectBundleInto="$TASK_APP/InteractionCompanion" \
  xcrun simctl launch --console-pty "$TASK_SIM" dev.interact-ai.companion -XCTest All \
  "$TASK_APP/PlugIns/InteractionCompanionTests.xctest" > "$TASK_OUT/xctest.log" 2>&1
python3 scripts/tests/xctest-result.py "$TASK_OUT/xctest.log" | tee "$TASK_OUT/result.json"
