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
#        runtime/webview2/...  (optional fixed runtime)
#
# Output: dist/websocket2serial-windows-x86_64.zip
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File scripts/package-windows.ps1
#   powershell -ExecutionPolicy Bypass -File scripts/package-windows.ps1 `
#     -WebView2RuntimeCab <fixed-109.cab> -WebView2RuntimeSha256 <sha256>

#Requires -Version 5.0
param(
    # Official WebView2 Fixed Version CAB; hash and version are validated.
    [string]$WebView2RuntimeCab = "",
    [string]$WebView2RuntimeSha256 = "",
    [string]$WebView2RuntimeVersion = "109.0.1518.140",
    # Pre-extracted runtime directory (no CAB hash validation).
    [string]$WebView2RuntimeDir = ""
)
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

# Optional Win7-compatible WebView2 Fixed Version runtime (109).
# Offline releases must provide CAB + SHA-256; without it the app still uses Evergreen.
$RuntimeSource = ""
if ($WebView2RuntimeCab -and $WebView2RuntimeDir) {
    throw "Provide only one of WebView2RuntimeCab or WebView2RuntimeDir"
}
if ($WebView2RuntimeCab) {
    if ($WebView2RuntimeSha256 -notmatch '^[0-9A-Fa-f]{64}$') {
        throw "WebView2RuntimeCab requires a 64-character hexadecimal WebView2RuntimeSha256"
    }
    $Cab = (Resolve-Path $WebView2RuntimeCab).Path
    $ActualHash = (Get-FileHash -Algorithm SHA256 $Cab).Hash
    if ($ActualHash -ne $WebView2RuntimeSha256) {
        throw "WebView2 Runtime SHA-256 mismatch: expected $WebView2RuntimeSha256, got $ActualHash"
    }
    $ExtractDir = Join-Path $StageRoot "webview2-extracted"
    if (Test-Path $ExtractDir) {
        Remove-Item -Recurse -Force $ExtractDir
    }
    New-Item -ItemType Directory -Force -Path $ExtractDir | Out-Null
    & expand.exe $Cab -F:* $ExtractDir | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "expand.exe failed to extract WebView2 Runtime (exit $LASTEXITCODE)"
    }
    $RuntimeExe = Get-ChildItem -Path $ExtractDir -Recurse -Filter "msedgewebview2.exe" |
        Select-Object -First 1
    if (-not $RuntimeExe) {
        throw "msedgewebview2.exe not found in WebView2 Runtime CAB"
    }
    $RuntimeSource = $RuntimeExe.DirectoryName
} elseif ($WebView2RuntimeDir) {
    $RuntimeExe = Get-ChildItem -Path $WebView2RuntimeDir -Recurse -Filter "msedgewebview2.exe" |
        Select-Object -First 1
    if (-not $RuntimeExe) {
        throw "msedgewebview2.exe not found in WebView2RuntimeDir"
    }
    $RuntimeSource = $RuntimeExe.DirectoryName
}

if ($RuntimeSource) {
    $ActualVersion = $RuntimeExe.VersionInfo.ProductVersion.Trim()
    if (-not $ActualVersion.StartsWith($WebView2RuntimeVersion)) {
        throw "WebView2 Runtime version mismatch: expected $WebView2RuntimeVersion, got $ActualVersion"
    }
    $RuntimeDest = Join-Path $StageDir "runtime\webview2"
    New-Item -ItemType Directory -Force -Path $RuntimeDest | Out-Null
    Copy-Item -Path (Join-Path $RuntimeSource "*") -Destination $RuntimeDest -Recurse -Force
    Write-Host "Bundled WebView2 Runtime: $ActualVersion"
} else {
    Write-Warning "No fixed WebView2 Runtime bundled: Win7 needs Runtime 109 already installed, or pass -WebView2RuntimeCab/-WebView2RuntimeDir"
}

New-Item -ItemType Directory -Force -Path $DistDir | Out-Null
if (Test-Path $ZipPath) {
    Remove-Item -Force $ZipPath
}
Compress-Archive -Path $StageDir -DestinationPath $ZipPath

$Size = (Get-Item $ZipPath).Length
Write-Host "Created: $ZipPath ($Size bytes)"
