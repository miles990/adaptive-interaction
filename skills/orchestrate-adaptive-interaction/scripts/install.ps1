# Build and install the interact-ai CLI (cargo required).
$ErrorActionPreference = "Stop"
$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..\..")
cargo install --path (Join-Path $repoRoot "crates\interaction-cli") --locked
Write-Host "installed. start the daemon with: interact-ai serve"
