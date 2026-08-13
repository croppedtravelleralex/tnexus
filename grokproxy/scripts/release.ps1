# One-command release from Windows.
#
#   pwsh -File grokproxy\scripts\release.ps1
#
# The Rust toolchain is on Windows and docker is in WSL, so the gates run here
# and the build/push/deploy runs there. Chain stays: build locally -> GHCR ->
# panda pulls. Nothing is compiled on panda and nothing is copied to it.

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$crate = Join-Path $repo 'grokproxy'

$env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path
Push-Location $crate
try {
    Write-Host "`n=== gates (same as CI would run) ===" -ForegroundColor Cyan
    cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { throw "cargo fmt --check failed" }
    cargo clippy --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw "clippy failed" }
    cargo test --locked
    if ($LASTEXITCODE -ne 0) { throw "tests failed" }
}
finally {
    Pop-Location
}

Write-Host "`n=== handing off to WSL for build + push + deploy ===" -ForegroundColor Cyan
$script = '/mnt/d/SelfMadeTool/TNexus/grokproxy/scripts/release.sh'
wsl -e bash -lc "SKIP_GATES=1 bash $script"
if ($LASTEXITCODE -ne 0) { throw "release.sh failed with $LASTEXITCODE" }
