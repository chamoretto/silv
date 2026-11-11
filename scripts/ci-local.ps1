#Requires -Version 7
<#
    Local CI gate: the same steps as .github/workflows/ci.yml, for every platform CI builds on.

    CI catches platform-gated code (the xkb keymap only compiles on unix, the macOS keymap only on
    macos), so a Windows-only `cargo test` is not enough. Run this before pushing:

        pwsh -File scripts/ci-local.ps1          # all gates
        pwsh -File scripts/ci-local.ps1 -Quick   # skips the slow WSL test build (lib tests only)
        pwsh -File scripts/ci-local.ps1 -SkipWsl -SkipMacos

    Requirements: cargo with the aarch64-apple-darwin target installed (rustup target add), and WSL
    with a Rust toolchain for the unix gate (SILV_WSL_DISTRO overrides the distro name).
#>
param(
    [switch]$Quick,
    [switch]$SkipWsl,
    [switch]$SkipMacos,
    [string]$WslDistro = $env:SILV_WSL_DISTRO
)

$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
$WslDistro = if ($WslDistro) { $WslDistro } else { 'Ubuntu-24.04' }
$MacTarget = 'aarch64-apple-darwin'

function Invoke-Step {
    param ([string]$Name, [scriptblock]$Body)
    Write-Host "==> $Name"
    & $Body
    if ($LASTEXITCODE -ne 0) {
        Write-Host "FAILED: $Name (exit $LASTEXITCODE)"
        exit $LASTEXITCODE
    }
}

Invoke-Step 'windows: fmt' { cargo fmt --all --check }
Invoke-Step 'windows: clippy' { cargo clippy --all-targets --all-features -- -D warnings }
Invoke-Step 'windows: build' { cargo build --all-targets }
Invoke-Step 'windows: test' { cargo test --all-targets }

if (-not $SkipWsl) {
    # The unix-only code paths (xkb layout detection) compile here; cargo comes from the login shell.
    $wslTest = if ($Quick) { 'cargo test --lib' } else { 'cargo test --all-targets' }
    Invoke-Step "wsl(${WslDistro}): clippy" {
        wsl -d $WslDistro bash -lc "cd /mnt/c$((Get-Location).Path.Replace('\', '/').Substring(2)) && cargo clippy --all-targets --all-features -- -D warnings"
    }
    Invoke-Step "wsl(${WslDistro}): test" {
        wsl -d $WslDistro bash -lc "cd /mnt/c$((Get-Location).Path.Replace('\', '/').Substring(2)) && $wslTest"
    }
}

if (-not $SkipMacos) {
    # Cross check compiles the macOS keymap (checks do not link, so the FFI is covered).
    Invoke-Step "macos($MacTarget): clippy" { cargo clippy --target $MacTarget --all-targets --all-features -- -D warnings }
    Invoke-Step "macos($MacTarget): build" { cargo check --target $MacTarget --all-targets }
}

Write-Host 'All gates passed.'
