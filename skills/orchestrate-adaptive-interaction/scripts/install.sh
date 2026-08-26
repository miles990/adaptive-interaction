#!/usr/bin/env bash
# Build and install the interact-ai CLI (cargo required).
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cargo install --path "$repo_root/crates/interaction-cli" --locked
echo "installed: $(command -v interact-ai)"
echo "start the daemon with: interact-ai serve"
