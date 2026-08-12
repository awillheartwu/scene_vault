# Scene Vault Windows development environment setup (one-click).
#
# Creates a Python venv with the vision extras, downloads the YuNet and SFace
# ONNX models, and writes the settings into the app database so the vision
# engine and face-bank suggestions work out of the box.
#
# Usage (PowerShell):
#   powershell -ExecutionPolicy Bypass -File scripts\setup-windows.ps1
#
# The app must be closed before the settings are written.

$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
$VenvDir = Join-Path $Root "python\.venv"
$VenvPython = Join-Path $VenvDir "Scripts\python.exe"
$ModelsDir = Join-Path $env:LOCALAPPDATA "SceneVault\models"
$DbPath = Join-Path $env:APPDATA "com.scenevault.desktop\scene-vault.db"

Write-Host "== Scene Vault Windows setup =="
Write-Host "Repo root : $Root"
Write-Host "Venv      : $VenvDir"
Write-Host "Models    : $ModelsDir"
Write-Host ""

# 1. Locate a Python 3.11+ interpreter.
$Python = $null
foreach ($candidate in @("py", "python")) {
    try {
        if ($candidate -eq "py") {
            $probe = & py -3 -c "import sys; print(sys.executable)" 2>$null
        } else {
            $probe = & python -c "import sys; print(sys.executable)" 2>$null
        }
        if ($LASTEXITCODE -eq 0 -and $probe) { $Python = $probe.Trim(); break }
    } catch { }
}
if (-not $Python) {
    throw "No Python 3.11+ found. Install Python from https://www.python.org/downloads/ (check 'Add to PATH' or use the py launcher) and re-run."
}
Write-Host "[1/4] Python: $Python"

# 2. Create the venv and install the vision extras.
if (-not (Test-Path $VenvPython)) {
    Write-Host "[2/4] Creating venv and installing vision dependencies (first run downloads ~100 MB)..."
    & $Python -m venv $VenvDir
    if ($LASTEXITCODE -ne 0) { throw "venv creation failed" }
} else {
    Write-Host "[2/4] Venv already exists, refreshing dependencies..."
}
& $VenvPython -m pip install --quiet --upgrade pip
& $VenvPython -m pip install --quiet --upgrade ("-e", (Join-Path $Root "python[vision]"))
if ($LASTEXITCODE -ne 0) { throw "pip install failed" }

# 3. Download the ONNX models (skipped when already present).
New-Item -ItemType Directory -Force -Path $ModelsDir | Out-Null
$YunetPath = Join-Path $ModelsDir "face_detection_yunet_2023mar.onnx"
$SfacePath = Join-Path $ModelsDir "face_recognition_sface_2021dec.onnx"
$YunetUrl = "https://github.com/opencv/opencv_zoo/raw/main/models/face_detection_yunet/face_detection_yunet_2023mar.onnx"
$SfaceUrl = "https://github.com/opencv/opencv_zoo/raw/main/models/face_recognition_sface/face_recognition_sface_2021dec.onnx"
Write-Host "[3/4] Downloading models and fonts"
Write-Host "  Models into $ModelsDir"
if (-not (Test-Path $YunetPath)) {
    Write-Host "  YuNet ..."
    Invoke-WebRequest -UseBasicParsing -Uri $YunetUrl -OutFile $YunetPath
} else {
    Write-Host "  YuNet (cached)"
}
if (-not (Test-Path $SfacePath)) {
    Write-Host "  SFace (~38 MB) ..."
    Invoke-WebRequest -UseBasicParsing -Uri $SfaceUrl -OutFile $SfacePath
} else {
    Write-Host "  SFace (cached)"
}

# Bundled annotation fonts (SIL OFL, redistributable). Downloaded into the
# app-local fonts directory; the settings UI lists them automatically and the
# default annotation font is SmileySans.
$FontPath = $null
$FontsDir = Join-Path $env:LOCALAPPDATA "com.scenevault.desktop\fonts"
New-Item -ItemType Directory -Force -Path $FontsDir | Out-Null
$SmileyZip = Join-Path $FontsDir "smiley-sans.zip"
$SmileyFont = Join-Path $FontsDir "SmileySans-Oblique.ttf"
$LxgwFont = Join-Path $FontsDir "LXGWWenKai-Regular.ttf"
$NotoFont = Join-Path $FontsDir "NotoSansSC-Regular.ttf"
Write-Host "  Fonts into $FontsDir"
if (-not (Test-Path $SmileyFont)) {
    Write-Host "    SmileySans 得意黑 ..."
    Invoke-WebRequest -UseBasicParsing -Uri "https://github.com/atelier-anchor/smiley-sans/releases/download/v2.0.1/smiley-sans-v2.0.1.zip" -OutFile $SmileyZip
    Expand-Archive -Force -Path $SmileyZip -DestinationPath $FontsDir
    Remove-Item -Force $SmileyZip
} else {
    Write-Host "    SmileySans (cached)"
}
if (-not (Test-Path $LxgwFont)) {
    Write-Host "    LXGW WenKai 霞鹜文楷 (~25 MB) ..."
    Invoke-WebRequest -UseBasicParsing -Uri "https://github.com/lxgw/LxgwWenKai/releases/download/v1.520/LXGWWenKai-Regular.ttf" -OutFile $LxgwFont
} else {
    Write-Host "    LXGW WenKai (cached)"
}
if (-not (Test-Path $NotoFont)) {
    Write-Host "    Noto Sans SC (~18 MB) ..."
    Invoke-WebRequest -UseBasicParsing -Uri "https://github.com/google/fonts/raw/main/ofl/notosanssc/NotoSansSC%5Bwght%5D.ttf" -OutFile $NotoFont
} else {
    Write-Host "    Noto Sans SC (cached)"
}
$ZcoolKuaiLeFont = Join-Path $FontsDir "ZCOOLKuaiLe-Regular.ttf"
$ZcoolQingKeFont = Join-Path $FontsDir "ZCOOLQingKeHuangYou-Regular.ttf"
$ZcoolXiaoWeiFont = Join-Path $FontsDir "ZCOOLXiaoWei-Regular.ttf"
if (-not (Test-Path $ZcoolKuaiLeFont)) {
    Write-Host "    ZCOOL KuaiLe 站酷快乐体 (~5 MB) ..."
    Invoke-WebRequest -UseBasicParsing -Uri "https://github.com/google/fonts/raw/main/ofl/zcoolkuaile/ZCOOLKuaiLe-Regular.ttf" -OutFile $ZcoolKuaiLeFont
} else {
    Write-Host "    ZCOOL KuaiLe (cached)"
}
if (-not (Test-Path $ZcoolQingKeFont)) {
    Write-Host "    ZCOOL QingKe HuangYou 站酷高端黑 (~3 MB) ..."
    Invoke-WebRequest -UseBasicParsing -Uri "https://github.com/google/fonts/raw/main/ofl/zcoolqingkehuangyou/ZCOOLQingKeHuangYou-Regular.ttf" -OutFile $ZcoolQingKeFont
} else {
    Write-Host "    ZCOOL QingKe HuangYou (cached)"
}
if (-not (Test-Path $ZcoolXiaoWeiFont)) {
    Write-Host "    ZCOOL XiaoWei 站酷小薇 (~3 MB) ..."
    Invoke-WebRequest -UseBasicParsing -Uri "https://github.com/google/fonts/raw/main/ofl/zcoolxiaowei/ZCOOLXiaoWei-Regular.ttf" -OutFile $ZcoolXiaoWeiFont
} else {
    Write-Host "    ZCOOL XiaoWei (cached)"
}
if (Test-Path $SmileyFont) { $FontPath = $SmileyFont }

# 4. Write the settings into the app database (the app must be closed).
Write-Host ""
$AppProcess = Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.ProcessName -like "*scene*" -or $_.ProcessName -like "*Scene Vault*" }
if ($AppProcess) {
    Write-Host "WARNING: Scene Vault appears to be running. Close it and re-run the script so the settings can be written safely."
} else {
    if (Test-Path $DbPath) {
        Write-Host "[4/4] Writing settings into $DbPath"
        $ConfigureArgs = @(
            "--db", $DbPath,
            "--python-exe", $VenvPython,
            "--module-root", (Join-Path $Root "python\src"),
            "--yunet", $YunetPath,
            "--sface", $SfacePath
        )
        if ($FontPath) { $ConfigureArgs += @("--font", $FontPath) }
        & $VenvPython (Join-Path $PSScriptRoot "configure_vision_settings.py") @ConfigureArgs
        if ($LASTEXITCODE -ne 0) { throw "settings write failed" }
    } else {
        Write-Host "[4/4] App database not found at $DbPath"
        Write-Host "  Start Scene Vault once so it creates the database, close it, then re-run this script."
    }
}

# Verification.
Write-Host ""
Write-Host "== Verifying the engine =="
& $VenvPython -m scene_vault_ai health
Write-Host ""
Write-Host "Done. Open Scene Vault -> Settings -> 视觉引擎 -> 检查引擎 to confirm."
