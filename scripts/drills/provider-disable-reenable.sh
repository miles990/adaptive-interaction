#!/usr/bin/env bash
# Production CLI/daemon + pty simulator. Assertions and persistent evidence.
# Usage: bash scripts/drills/provider-disable-reenable.sh [port] [--output-dir DIR]
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
exec python3 "$ROOT/scripts/drills/provider-lifecycle.py" "$@"
