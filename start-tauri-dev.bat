@echo off
setlocal
cd /d "%~dp0"

rem Keep all Windows-side builds and the dev app on CPU 0-3. Children
rem inherit the affinity mask, while Cargo/Rayon also receive hard limits.
set "PATH=C:\Windows\System32;C:\Program Files\nodejs;%APPDATA%\npm;%USERPROFILE%\.cargo\bin;%PATH%"
set "CARGO_BUILD_JOBS=4"
set "RAYON_NUM_THREADS=4"
set "TOKIO_WORKER_THREADS=4"

echo Starting Scene Vault with CPU affinity 0-3 and 4 build/runtime workers...
start "Scene Vault Dev - 4 P cores" /affinity F /wait cmd.exe /d /s /c "pnpm tauri dev"
exit /b %errorlevel%
