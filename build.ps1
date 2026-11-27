$ErrorActionPreference = "Stop"
$version = "1.0.1"
$csc = "C:\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.exe"
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$dist = Join-Path $root "dist"

if (Test-Path $dist) { Remove-Item $dist -Recurse -Force }
New-Item -ItemType Directory -Path $dist -Force | Out-Null

$appExe = Join-Path $dist "AntigravitySync.exe"
$appGz  = Join-Path $dist "AntigravitySync.exe.gz"
$setupExe = Join-Path $dist "AntigravitySync-Setup-v$version.exe"

Write-Host "Building AntigravitySync.exe v$version..." -ForegroundColor Cyan
& $csc /nologo /target:winexe /optimize /warnaserror `
    /r:System.Windows.Forms.dll /r:System.Drawing.dll /r:Microsoft.CSharp.dll `
    /out:$appExe (Join-Path $root "src\AntigravitySync.cs")
if ($LASTEXITCODE -ne 0) { throw "Failed to compile AntigravitySync.exe" }

Write-Host "Compressing payload..." -ForegroundColor Cyan
$inBytes = [System.IO.File]::ReadAllBytes($appExe)
$outStream = [System.IO.File]::Create($appGz)
$gz = New-Object System.IO.Compression.GZipStream($outStream, [System.IO.Compression.CompressionLevel]::Optimal)
$gz.Write($inBytes, 0, $inBytes.Length)
$gz.Close()
$outStream.Close()

Write-Host "Building AntigravitySync-Setup-v$version.exe..." -ForegroundColor Cyan
& $csc /nologo /target:winexe /optimize /warnaserror `
    /r:System.Windows.Forms.dll /r:System.Drawing.dll /r:Microsoft.CSharp.dll `
    /resource:"$appGz,AntigravitySync.exe.gz" `
    /out:$setupExe (Join-Path $root "src\Installer.cs")
if ($LASTEXITCODE -ne 0) { throw "Failed to compile Setup" }

Remove-Item $appGz -Force

# Verify PE Subsystem 2 (WINDOWS_GUI) on both binaries
function Assert-WindowsGuiSubsystem([string]$path) {
    $bytes = [System.IO.File]::ReadAllBytes($path)
    $peOffset = [BitConverter]::ToInt32($bytes, 0x3C)
    $subsystem = [BitConverter]::ToUInt16($bytes, $peOffset + 24 + 68)
    if ($subsystem -ne 2) {
        throw "$path has PE Subsystem $subsystem (expected 2 = WINDOWS_GUI)"
    }
    Write-Host "Verified PE Subsystem 2 (WINDOWS_GUI): $(Split-Path $path -Leaf)" -ForegroundColor Green
}

Assert-WindowsGuiSubsystem $appExe
Assert-WindowsGuiSubsystem $setupExe

# Generate SHA256SUMS.txt
$hash = (Get-FileHash $setupExe -Algorithm SHA256).Hash.ToLowerInvariant()
"$hash  AntigravitySync-Setup-v$version.exe" | Set-Content (Join-Path $dist "SHA256SUMS.txt") -Encoding ASCII
Write-Host "SHA-256: $hash" -ForegroundColor Green
