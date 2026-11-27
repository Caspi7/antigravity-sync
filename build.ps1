$ErrorActionPreference = "Stop"
$version = "2.2.3"
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$dist = Join-Path $root "dist"

$cargo = "$env:USERPROFILE\.cargo\bin\cargo.exe"
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:LOCALAPPDATA\Microsoft\WinGet\Links;" + [Environment]::GetEnvironmentVariable("Path","User") + ";" + [Environment]::GetEnvironmentVariable("Path","Machine")
if (-not $env:CARGO_TARGET_DIR) {
    $env:CARGO_TARGET_DIR = Join-Path $env:LOCALAPPDATA "antigravity-sync-target"
}

function Find-Iscc {
    $cmd = Get-Command "ISCC.exe" -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $candidates = @(
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
        "$env:LOCALAPPDATA\Programs\Antigravity IDE\resources\app\node_modules\innosetup\bin\ISCC.exe"
    )
    foreach ($candidate in $candidates) {
        if ($candidate -and (Test-Path -LiteralPath $candidate -PathType Leaf)) {
            return $candidate
        }
    }
    throw "ISCC.exe (Inno Setup 6) not found."
}

function Assert-WindowsGuiSubsystem([string]$path) {
    $bytes = [System.IO.File]::ReadAllBytes($path)
    $peOffset = [BitConverter]::ToInt32($bytes, 0x3C)
    $subsystem = [BitConverter]::ToUInt16($bytes, $peOffset + 24 + 68)
    if ($subsystem -ne 2) {
        throw "$path has PE Subsystem $subsystem (expected 2 = WINDOWS_GUI)"
    }
    Write-Host "Verified PE Subsystem 2 (WINDOWS_GUI): $(Split-Path $path -Leaf)" -ForegroundColor Green
}

Push-Location $root
try {
    Write-Host "Running unit tests..." -ForegroundColor Cyan
    & $cargo test
    if ($LASTEXITCODE -ne 0) { throw "cargo test failed" }

    Write-Host "Building Rust + Slint Release binary v$version..." -ForegroundColor Cyan
    & $cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build --release failed" }
} finally {
    Pop-Location
}

$payloadDir = Join-Path $env:CARGO_TARGET_DIR "release"
$builtExe = Join-Path $payloadDir "AntigravitySync.exe"
Assert-WindowsGuiSubsystem $builtExe

if (Test-Path $dist) { Remove-Item $dist -Recurse -Force }
New-Item -ItemType Directory -Path $dist -Force | Out-Null

$iscc = Find-Iscc
Write-Host "Packaging AntigravitySync_Setup_x64_$version.exe with Inno Setup..." -ForegroundColor Cyan
& $iscc "/DAppVersion=$version" "/DPayloadDir=$payloadDir" "/DRepoRoot=$root" (Join-Path $root "installer\AntigravitySync.iss")
if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }

$setupExe = Join-Path $dist "AntigravitySync_Setup_x64_$version.exe"
if (-not (Test-Path $setupExe)) { throw "Installer not created: $setupExe" }
$hash = (Get-FileHash $setupExe -Algorithm SHA256).Hash.ToLowerInvariant()
"$hash  AntigravitySync_Setup_x64_$version.exe" | Set-Content (Join-Path $dist "SHA256SUMS.txt") -Encoding ASCII
$sizeMiB = [math]::Round((Get-Item $setupExe).Length / 1MB, 2)
Write-Host "Created $setupExe ($sizeMiB MiB)" -ForegroundColor Green
Write-Host "SHA-256: $hash" -ForegroundColor Green
