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

# Push from here: WSL cannot use the Windows credential helper. TLS to
# github.com from this network drops regularly, so retry rather than fail a
# release whose only remaining step is publishing the commit.
Write-Host "`n=== git push (Windows side) ===" -ForegroundColor Cyan
$pushed = $false
foreach ($attempt in 1..5) {
    git -C $repo -c http.sslBackend=openssl push origin main
    if ($LASTEXITCODE -eq 0) { $pushed = $true; break }
    Write-Host "push attempt $attempt failed, retrying..." -ForegroundColor Yellow
    Start-Sleep -Seconds ($attempt * 4)
}
if (-not $pushed) { throw "git push failed after 5 attempts" }

Write-Host "`n=== handing off to WSL for build + push + deploy ===" -ForegroundColor Cyan
$script = '/mnt/d/SelfMadeTool/TNexus/grokproxy/scripts/release.sh'
wsl -e bash -lc "SKIP_GATES=1 SKIP_GIT_PUSH=1 bash $script"
if ($LASTEXITCODE -ne 0) { throw "release.sh failed with $LASTEXITCODE" }
