<#
.SYNOPSIS
Builds the Windows 1.0 release installers (NSIS) in two modes:

  - scene-vault-<version>-setup.exe     small package, no AI engine
  - scene-vault-<version>-ai-setup.exe  includes the PyInstaller sidecar,
    YuNet + SFace models and the bundled annotation font

.PARAMETER Version
Release version used for artifact names. Must match package.json,
Cargo.toml, tauri.conf.json and the Python engine.

.PARAMETER SkipAI
Build only the small package (no sidecar, no models).

.PARAMETER SignCertificatePath
Optional code-signing certificate (.pfx). When provided, the main exe,
the sidecar and the NSIS installers are signed with signtool; the PFX
password is read from the SIGN_CERT_PASSWORD environment variable.

.PARAMETER TimestampServer
Timestamp server URL used when signing (requires network at build time).

.EXAMPLE
./scripts/release-windows.ps1 -SkipAI
./scripts/release-windows.ps1
$env:SIGN_CERT_PASSWORD = "..." ; ./scripts/release-windows.ps1 -SignCertificatePath C:\certs\scene-vault.pfx
#>
param(
    [string]$Version = "1.0.0",
    [switch]$SkipAI,
    [string]$SignCertificatePath = "",
    [string]$TimestampServer = "http://timestamp.digicert.com"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$RepoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $RepoRoot

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw "This release script must run on Windows (PyInstaller and NSIS require it)."
}

function Assert-Tool([string]$Name) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "Required tool '$Name' was not found on PATH."
    }
}

function Get-JsonValue([string]$Path, [string]$Key) {
    $json = Get-Content -Raw $Path | ConvertFrom-Json
    return $json.$Key
}

# ---------------------------------------------------------------------------
# 1. Version consistency (app 1.0.0 must be unified everywhere)
# ---------------------------------------------------------------------------
$Expected = @{
    "package.json"             = (Get-JsonValue "package.json" "version")
    "src-tauri/Cargo.toml"     = ((Get-Content "src-tauri/Cargo.toml" | Select-String '^version = "([^"]+)"').Matches[0].Groups[1].Value)
    "src-tauri/tauri.conf.json" = (Get-JsonValue "src-tauri/tauri.conf.json" "version")
    "python/pyproject.toml"    = ((Get-Content "python/pyproject.toml" | Select-String '^version = "([^"]+)"').Matches[0].Groups[1].Value)
}
foreach ($entry in $Expected.GetEnumerator()) {
    if ($entry.Value -ne $Version) {
        throw "Version mismatch: $($entry.Key) is $($entry.Value), expected $Version."
    }
}
Write-Host "[1/6] Versions unified at $Version"

# ---------------------------------------------------------------------------
# 2. Frontend production build
# ---------------------------------------------------------------------------
Assert-Tool "pnpm"
Write-Host "[2/6] Building frontend"
$env:CI = "true"
pnpm install
pnpm build

# ---------------------------------------------------------------------------
# 3. Bundled font (shipped in every installer; refresh from the official zip)
# ---------------------------------------------------------------------------
$FontsDir = Join-Path $RepoRoot "src-tauri/resources/fonts"
New-Item -ItemType Directory -Force -Path $FontsDir | Out-Null
$FontFile = Join-Path $FontsDir "SmileySans-Oblique.ttf"
if (-not (Test-Path $FontFile)) {
    Write-Host "  Downloading SmileySans 得意黑 v2.0.1 (OFL-1.1)"
    $Zip = Join-Path $env:TEMP "smiley-sans-v2.0.1.zip"
    Invoke-WebRequest -UseBasicParsing `
        -Uri "https://github.com/atelier-anchor/smiley-sans/releases/download/v2.0.1/smiley-sans-v2.0.1.zip" `
        -OutFile $Zip
    Expand-Archive -Force -Path $Zip -DestinationPath $FontsDir
    Remove-Item -Force $Zip
}
if (-not (Test-Path (Join-Path $FontsDir "OFL.txt"))) {
    Write-Host "  WARNING: OFL.txt not found next to the font; copy it from the official zip before release."
}

# ---------------------------------------------------------------------------
# 4. AI package: sidecar + models (skipped with -SkipAI)
# ---------------------------------------------------------------------------
$SidecarExe = Join-Path $RepoRoot "src-tauri/binaries/scene-vault-ai-x86_64-pc-windows-msvc.exe"
if (-not $SkipAI) {
    Assert-Tool "python"
    Write-Host "[3/6] Building AI sidecar (PyInstaller)"

    $ModelsDir = Join-Path $RepoRoot "src-tauri/resources/models"
    New-Item -ItemType Directory -Force -Path $ModelsDir | Out-Null
    $ModelUrls = @{
        "face_detection_yunet_2023mar.onnx"   = "https://github.com/opencv/opencv_zoo/raw/main/models/face_detection_yunet/face_detection_yunet_2023mar.onnx"
        "face_recognition_sface_2021dec.onnx" = "https://github.com/opencv/opencv_zoo/raw/main/models/face_recognition_sface/face_recognition_sface_2021dec.onnx"
    }
    foreach ($name in $ModelUrls.Keys) {
        $target = Join-Path $ModelsDir $name
        if (-not (Test-Path $target)) {
            Write-Host "  Downloading model $name"
            Invoke-WebRequest -UseBasicParsing -Uri $ModelUrls[$name] -OutFile $target
        }
    }

    $BuildVenv = Join-Path $env:TEMP "sv-ai-build-venv"
    if (-not (Test-Path (Join-Path $BuildVenv "Scripts/python.exe"))) {
        python -m venv $BuildVenv
    }
    & (Join-Path $BuildVenv "Scripts/python.exe") -m pip install --quiet --upgrade pip
    & (Join-Path $BuildVenv "Scripts/python.exe") -m pip install --quiet -e "./python[vision,packaging]"

    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $SidecarExe) | Out-Null
    & (Join-Path $BuildVenv "Scripts/pyinstaller.exe") --noconfirm --clean `
        --onefile --name scene-vault-ai `
        --distpath (Join-Path $env:TEMP "sv-ai-dist") `
        python/sidecar.py
    Copy-Item -Force (Join-Path $env:TEMP "sv-ai-dist/scene-vault-ai.exe") $SidecarExe
    Write-Host "  Sidecar -> $SidecarExe"
} else {
    Write-Host "[3/6] Skipping AI sidecar and models (-SkipAI)"
}

# ---------------------------------------------------------------------------
# 5. Tauri builds (NSIS) - small package, then AI variant via config merge
# ---------------------------------------------------------------------------
function Build-Installer([string]$ConfigArg, [string]$OutputName) {
    Write-Host "[4/6] tauri build $ConfigArg"
    if ($ConfigArg) {
        pnpm tauri build --bundles nsis --config $ConfigArg
    } else {
        pnpm tauri build --bundles nsis
    }
    $nsis = Join-Path $RepoRoot "src-tauri/target/release/bundle/nsis"
    $installer = Get-ChildItem $nsis -Filter "*setup.exe" | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if (-not $installer) { throw "NSIS installer not found under $nsis" }

    $ReleaseDir = Join-Path $RepoRoot "dist-release"
    New-Item -ItemType Directory -Force -Path $ReleaseDir | Out-Null
    $output = Join-Path $ReleaseDir $OutputName
    Copy-Item -Force $installer.FullName $output
    Write-Host "  $OutputName ($([math]::Round($installer.Length / 1MB, 1)) MB)"
    return $output
}

$SmallInstaller = Build-Installer "" "scene-vault-$Version-setup.exe"
$AiInstaller = $null
if (-not $SkipAI) {
    $AiInstaller = Build-Installer "src-tauri/tauri.ai.conf.json" "scene-vault-$Version-ai-setup.exe"
}

# ---------------------------------------------------------------------------
# 6. Optional code signing (only when a certificate is provided)
# ---------------------------------------------------------------------------
if ($SignCertificatePath) {
    Assert-Tool "signtool"
    $Password = $env:SIGN_CERT_PASSWORD
    $Common = @("/fd", "SHA256", "/f", $SignCertificatePath, "/tr", $TimestampServer, "/td", "SHA256")
    if ($Password) { $Common += @("/p", $Password) }
    foreach ($file in @($SmallInstaller, $AiInstaller)) {
        if ($file -and (Test-Path $file)) {
            & signtool sign @Common $file
        }
    }
    if (-not $SkipAI) {
        & signtool sign @Common $SidecarExe
        $MainExe = Join-Path $RepoRoot "src-tauri/target/release/scene_vault.exe"
        if (Test-Path $MainExe) { & signtool sign @Common $MainExe }
    }
}

Write-Host "[5/6] Done. Artifacts:"
Get-ChildItem (Join-Path $RepoRoot "dist-release") | ForEach-Object {
    Write-Host ("  {0}  {1} MB" -f $_.Name, [math]::Round($_.Length / 1MB, 1))
}
Write-Host "[6/6] Next: install on a clean Windows machine and run the acceptance checklist (see docs/decisions/2026-08-11-windows-delivery.md)."
