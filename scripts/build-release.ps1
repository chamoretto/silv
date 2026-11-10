# Builds release binaries (static CRT where the platform supports it) into dist/.
# Usage: scripts/build-release.ps1 [-Musl]
param(
    [switch]$Musl
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (cargo metadata --no-deps --format-version 1 | ConvertFrom-Json).packages[0].version
$dist = Join-Path $root "dist"
New-Item -ItemType Directory -Force -Path $dist | Out-Null

function Build-Target([string]$target, [string[]]$rustflags) {
    Write-Host "==> release build for $target"
    $env:RUSTFLAGS = $rustflags -join " "
    cargo build --release --target $target
    $binName = if ($target -like "*windows*") { "silv.exe" } else { "silv" }
    $src = Join-Path $root "target\$target\release\$binName"
    $dstExt = if ($target -like "*windows*") { ".exe" } else { "" }
    $dst = Join-Path $dist ("silv-{0}-{1}{2}" -f $version, $target, $dstExt)
    Copy-Item -Force $src $dst
    $hash = (Get-FileHash -Algorithm SHA256 $dst).Hash.ToLower()
    "$hash  $(Split-Path -Leaf $dst)" | Out-File -Append -Encoding ascii (Join-Path $dist "SHA256SUMS")
    Write-Host "    -> $dst"
}

Remove-Item (Join-Path $dist "SHA256SUMS") -ErrorAction SilentlyContinue

Build-Target "x86_64-pc-windows-msvc" @("-C", "target-feature=+crt-static")

if ($Musl) {
    Build-Target "x86_64-unknown-linux-musl" @("-C", "target-feature=+crt-static")
}

Write-Host "==> dist contents:"
Get-ChildItem $dist | Select-Object Name, Length | Format-Table -AutoSize
