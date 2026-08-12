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
    [string]$TimestampServer = "http://timestamp.digicert.com",
    # CPU 0-3 (P-core threads) only: this build machine is a 13900K that has
    # shown instability under all-core load, so every child process is pinned
    # to these four logical CPUs. Change to 0xFF for four full P-cores if the
    # machine's stability profile allows it.
    [int]$AffinityMask = 0xF
    ,
    # Skip CPU pinning entirely (GitHub Actions runners have few vCPUs and no
    # 13900K-style stability constraints).
    [switch]$SkipAffinity
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$RepoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $RepoRoot

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw "This release script must run on Windows (PyInstaller and NSIS require it)."
}

function Find-Tool([string]$Name, [string[]]$Fallbacks) {
    $cmd = Get-Command $Name -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    foreach ($candidate in $Fallbacks) {
        if (Test-Path $candidate) { return $candidate }
    }
    throw "Required tool '$Name' was not found on PATH or in standard locations."
}

# Quote arguments that contain spaces so they survive the Start-Process
# ArgumentList round-trip.
function Quote-Arg([string]$Arg) {
    if ($Arg -match " ") { return '"' + $Arg + '"' }
    return $Arg
}

# Runs a native command through Start-Process and enforces a zero exit code.
# Output stays on the console (no redirects): in PowerShell 5.1, redirecting
# switches the child to the parent environment, whose PATH is unreliable under
# WSL-launched PowerShell, while a plain Start-Process inherits the full
# registry PATH (node/pnpm/cargo/python are all there on this machine).
# CPU affinity is inherited by every child from this PowerShell process (set
# below), so cargo -> rustc and node -> cmd chains stay on $AffinityMask.
function Run-Native([string]$FilePath, [string[]]$Arguments, [string]$LogBase) {
    $quoted = @($Arguments | ForEach-Object { Quote-Arg $_ })
    $process = Start-Process -FilePath $FilePath -ArgumentList $quoted -NoNewWindow -PassThru -Wait
    if ($process.ExitCode -ne 0) {
        throw "Command failed (exit $($process.ExitCode)): $FilePath $($Arguments -join ' ')"
    }
}

function Get-JsonValue([string]$Path, [string]$Key) {
    $json = Get-Content -Raw $Path | ConvertFrom-Json
    return $json.$Key
}

# Waits for a build artifact to appear instead of relying on Start-Process
# -Wait: under WSL-launched PowerShell the process object sometimes never
# reports completion even though the child already finished and produced its
# output. Polls every 10 seconds, stops the leftover process once the artifact
# exists (or after the timeout) and reports whether it was found.
function Wait-ForArtifact(
    [System.Diagnostics.Process]$Process,
    [string]$ArtifactPath,
    [int]$TimeoutMinutes
) {
    $deadline = (Get-Date).AddMinutes($TimeoutMinutes)
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Seconds 10
        if (Test-Path $ArtifactPath) {
            if (-not $Process.HasExited) { Stop-Process -Id $Process.Id -Force -ErrorAction SilentlyContinue }
            return $true
        }
        if ($Process.HasExited) { return $false }
    }
    if (-not $Process.HasExited) { Stop-Process -Id $Process.Id -Force -ErrorAction SilentlyContinue }
    return $false
}

# ---------------------------------------------------------------------------
# 1. Version consistency (app 1.0.0 must be unified everywhere)
# ---------------------------------------------------------------------------
$Expected = @{
    "package.json"              = (Get-JsonValue "package.json" "version")
    "src-tauri/Cargo.toml"      = ((Get-Content "src-tauri/Cargo.toml" | Select-String '^version = "([^"]+)"').Matches[0].Groups[1].Value)
    "src-tauri/tauri.conf.json" = (Get-JsonValue "src-tauri/tauri.conf.json" "version")
    "python/pyproject.toml"     = ((Get-Content "python/pyproject.toml" | Select-String '^version = "([^"]+)"').Matches[0].Groups[1].Value)
}
foreach ($entry in $Expected.GetEnumerator()) {
    if ($entry.Value -ne $Version) {
        throw "Version mismatch: $($entry.Key) is $($entry.Value), expected $Version."
    }
}
Write-Host "[1/6] Versions unified at $Version"

$LogDir = Join-Path $RepoRoot "dist-release"
New-Item -ItemType Directory -Force -Path $LogDir | Out-Null

# Tools are discovered from PATH first, then from standard install locations,
# so the script runs on a plain machine without manual PATH setup. pnpm is
# invoked through its JS entry point (node pnpm.cjs) because npm-style .cmd
# shims break under WSL-launched PowerShell.
$Node = Find-Tool "node" @("C:\Program Files\nodejs\node.exe")
$PnpmShim = Find-Tool "pnpm" @("C:\Users\WuHaoli\AppData\Roaming\npm\pnpm.cmd")
# pnpm.cjs lives in a different place depending on the install layout (npm
# global install vs. GitHub Actions standalone pnpm), so probe both locations
# next to the discovered shim/exe instead of assuming a single layout.
$PnpmJs = @(
    (Join-Path (Split-Path -Parent $PnpmShim) "node_modules/pnpm/bin/pnpm.cjs"),
    (Join-Path (Split-Path -Parent $PnpmShim) "pnpm.cjs")
) | Where-Object { Test-Path $_ } | Select-Object -First 1
$Cargo = Find-Tool "cargo" @("C:\Users\WuHaoli\.cargo\bin\cargo.exe")
if (-not $PnpmJs) {
    Write-Host "  WARNING: pnpm.cjs not found next to '$PnpmShim'; invoking pnpm via its command entry"
}
$NpmGlobal = ""
if ($PnpmJs) { $NpmGlobal = Split-Path -Parent (Split-Path -Parent $PnpmJs) }
$env:Path = "$(Split-Path -Parent $Node);$NpmGlobal;$(Split-Path -Parent $Cargo);" + $env:Path
# Cargo parallelism is capped to the pinned core count (rustc is sequential
# per job, and the affinity mask above already limits actual execution).
$env:CARGO_BUILD_JOBS = "4"
# Pin this PowerShell process (and therefore every child it spawns, which
# inherit the mask) to the allowed CPUs before any heavy work starts.
if (-not $SkipAffinity) {
    try {
        (Get-Process -Id $PID).ProcessorAffinity = $AffinityMask
    } catch {
        Write-Host "  WARNING: could not pin CPU affinity (0x$('{0:X}' -f $AffinityMask)): $_"
    }
}
if ($SkipAffinity) {
    Write-Host "[0/6] CPU affinity skipped (-SkipAffinity), CARGO_BUILD_JOBS=4"
} else {
    Write-Host "[0/6] CPU affinity 0x$('{0:X}' -f $AffinityMask) (CPU 0-3), CARGO_BUILD_JOBS=4"
}

# ---------------------------------------------------------------------------
# 2. Frontend production build
# ---------------------------------------------------------------------------
# Tools are invoked through their JS entry points with node directly. pnpm's
# lifecycle shell (cmd/sh) is unreliable in some environments (e.g. WSL-
# launched PowerShell), so `pnpm build` / `pnpm tauri build` are replaced by
# the equivalent node invocations. `pnpm install` still resolves the store.
Write-Host "[2/6] Building frontend"
$env:CI = "true"
if ($PnpmJs) {
    Run-Native $Node @($PnpmJs, "install") (Join-Path $LogDir "pnpm-install")
} else {
    Run-Native $PnpmShim @("install") (Join-Path $LogDir "pnpm-install")
}
Run-Native $Node @("node_modules/vue-tsc/bin/vue-tsc.js", "--noEmit") (Join-Path $LogDir "vue-tsc")
Run-Native $Node @("node_modules/vite/bin/vite.js", "build") (Join-Path $LogDir "vite-build")

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
    $Python = Find-Tool "python" @(
        "C:\Users\WuHaoli\AppData\Local\Programs\Python\Python312\python.exe",
        "C:\Users\WuHaoli\AppData\Local\Programs\Python\Python311\python.exe"
    )
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
    $VenvPython = Join-Path $BuildVenv "Scripts/python.exe"
    if (-not (Test-Path $VenvPython)) {
        Run-Native $Python @("-m", "venv", $BuildVenv) (Join-Path $LogDir "venv-create")
    }
    Run-Native $VenvPython @("-m", "pip", "install", "--quiet", "--upgrade", "pip") (Join-Path $LogDir "pip-upgrade")
    Run-Native $VenvPython @("-m", "pip", "install", "--quiet", "-e", "./python[vision,packaging]") (Join-Path $LogDir "pip-install")

    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $SidecarExe) | Out-Null
    $DistPath = Join-Path $env:TEMP "sv-ai-dist"
    # Remove stale artifacts from earlier runs so the poll cannot mistake them
    # for the fresh build (this bug shipped an old installer once).
    Remove-Item -Force (Join-Path $DistPath "scene-vault-ai.exe") -ErrorAction SilentlyContinue
    $sidecarProcess = Start-Process -FilePath $VenvPython -NoNewWindow -PassThru -ArgumentList @(
        "-m", "PyInstaller", "--noconfirm", "--clean",
        "--onefile", "--name", "scene-vault-ai",
        "--distpath", $DistPath,
        "python/sidecar.py"
    )
    $sidecarBuilt = Wait-ForArtifact $sidecarProcess (Join-Path $DistPath "scene-vault-ai.exe") 30
    if (-not $sidecarBuilt) { throw "PyInstaller did not produce scene-vault-ai.exe within 30 minutes" }
    Copy-Item -Force (Join-Path $DistPath "scene-vault-ai.exe") $SidecarExe
    Write-Host "  Sidecar -> $SidecarExe"
} else {
    Write-Host "[3/6] Skipping AI sidecar and models (-SkipAI)"
}

# ---------------------------------------------------------------------------
# 5. Tauri builds (NSIS) - small package, then AI variant via config merge
# ---------------------------------------------------------------------------
function Build-Installer([string]$ConfigArg, [string]$OutputName) {
    # beforeBuildCommand is empty in tauri.conf.json: PATH resolution is
    # unreliable in some environments (WSL-launched PowerShell), so the
    # frontend is built explicitly in step 2 and cargo is passed by absolute
    # path through --runner.
    $arguments = @("node_modules/@tauri-apps/cli/tauri.js", "build", "--bundles", "nsis", "--runner", $Cargo)
    if ($ConfigArg) { $arguments += @("--config", $ConfigArg) }
    Write-Host "[4/6] tauri build $ConfigArg"

    $nsis = Join-Path $RepoRoot "src-tauri/target/release/bundle/nsis"
    # Remove stale installers from earlier runs: the poll below must only ever
    # see the installer produced by THIS build.
    if (Test-Path $nsis) {
        Remove-Item -Force (Join-Path $nsis "*setup.exe") -ErrorAction SilentlyContinue
    }
    $tauriProcess = Start-Process -FilePath $Node -ArgumentList $arguments -NoNewWindow -PassThru

    $deadline = (Get-Date).AddMinutes(60)
    $installer = $null
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Seconds 10
        $installer = Get-ChildItem $nsis -Filter "*setup.exe" -ErrorAction SilentlyContinue |
            Sort-Object LastWriteTime -Descending | Select-Object -First 1
        if ($installer) { break }
        if ($tauriProcess.HasExited) { break }
    }
    # The installer file appears near the very end of the bundle step; give the
    # CLI a short grace period to finish (signing/cleanup) before force-killing.
    $graceDeadline = (Get-Date).AddMinutes(3)
    while (-not $tauriProcess.HasExited -and (Get-Date) -lt $graceDeadline) {
        Start-Sleep -Seconds 10
    }
    if (-not $tauriProcess.HasExited) { Stop-Process -Id $tauriProcess.Id -Force -ErrorAction SilentlyContinue }
    if (-not $installer) { throw "NSIS installer not found under $nsis" }

    $output = Join-Path $LogDir $OutputName
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
    $Signtool = Find-Tool "signtool" @(
        (Get-ChildItem "C:\Program Files (x86)\Windows Kits\10\bin\*\x64\signtool.exe" -ErrorAction SilentlyContinue |
            Sort-Object FullName -Descending | Select-Object -First 1 -ExpandProperty FullName)
    )
    $Password = $env:SIGN_CERT_PASSWORD
    $Common = @("sign", "/fd", "SHA256", "/f", $SignCertificatePath, "/tr", $TimestampServer, "/td", "SHA256")
    if ($Password) { $Common += @("/p", $Password) }
    foreach ($file in @($SmallInstaller, $AiInstaller)) {
        if ($file -and (Test-Path $file)) {
            Run-Native $Signtool ($Common + $file) (Join-Path $LogDir "sign")
        }
    }
    if (-not $SkipAI) {
        Run-Native $Signtool ($Common + $SidecarExe) (Join-Path $LogDir "sign-sidecar")
        $MainExe = Join-Path $RepoRoot "src-tauri/target/release/scene_vault.exe"
        if (Test-Path $MainExe) { Run-Native $Signtool ($Common + $MainExe) (Join-Path $LogDir "sign-main") }
    }
}

Write-Host "[5/6] Done. Artifacts:"
Get-ChildItem $LogDir -Filter "scene-vault-*-setup.exe" | ForEach-Object {
    Write-Host ("  {0}  {1} MB" -f $_.Name, [math]::Round($_.Length / 1MB, 1))
}
Write-Host "[6/6] Next: install on a clean Windows machine and run the acceptance checklist (see docs/decisions/2026-08-11-windows-delivery.md)."
