# WebSocket2Serial Windows packaging script.
#
# 1) Build x86_64-pc-windows-msvc release with Rust 1.77.2 and static CRT
#    (-C target-feature=+crt-static) so the target machine needs no extra
#    VC runtime.
# 2) Stage exe / README / config template into a fixed directory layout and
#    zip it:
#
#      websocket2serial/
#        websocket2serial.exe
#        README.md
#        config.toml.example
#
# Output: dist/websocket2serial-windows-x86_64.zip
#
# Usage: powershell -ExecutionPolicy Bypass -File scripts/package-windows.ps1

#Requires -Version 5.0
$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
$Target = "x86_64-pc-windows-msvc"
$StageRoot = Join-Path $Root "target\package\windows"
$StageDir = Join-Path $StageRoot "websocket2serial"
$DistDir = Join-Path $Root "dist"
$ZipPath = Join-Path $DistDir "websocket2serial-windows-x86_64.zip"

# Static CRT: append to existing RUSTFLAGS (idempotent).
if ($env:RUSTFLAGS) {
    if ($env:RUSTFLAGS -notmatch "crt-static") {
        $env:RUSTFLAGS = "$env:RUSTFLAGS -C target-feature=+crt-static"
    }
} else {
    $env:RUSTFLAGS = "-C target-feature=+crt-static"
}
Write-Host "RUSTFLAGS=$env:RUSTFLAGS"

Write-Host "Building: cargo +1.77.2 build --release --target $Target"
cargo +1.77.2 build --release --target $Target
if ($LASTEXITCODE -ne 0) {
    throw "cargo build failed (exit $LASTEXITCODE)"
}

$Exe = Join-Path $Root "target\$Target\release\websocket2serial.exe"
if (-not (Test-Path $Exe)) {
    throw "Artifact not found: $Exe"
}

# Recreate staging directory.
if (Test-Path $StageDir) {
    Remove-Item -Recurse -Force $StageDir
}
New-Item -ItemType Directory -Force -Path $StageDir | Out-Null

Copy-Item $Exe (Join-Path $StageDir "websocket2serial.exe")
Copy-Item (Join-Path $Root "README.md") (Join-Path $StageDir "README.md")
Copy-Item (Join-Path $Root "packaging\config.toml.example") (Join-Path $StageDir "config.toml.example")

New-Item -ItemType Directory -Force -Path $DistDir | Out-Null
if (Test-Path $ZipPath) {
    Remove-Item -Force $ZipPath
}
Compress-Archive -Path $StageDir -DestinationPath $ZipPath

$Size = (Get-Item $ZipPath).Length
Write-Host "Created: $ZipPath ($Size bytes)"
