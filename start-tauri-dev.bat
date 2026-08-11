@echo off
setlocal
cd /d "%~dp0"

rem Keep all Windows-side builds and the dev app on CPU 0-3. Children
rem inherit the affinity mask. A clean build is serialized because several
rem concurrent rustc/cl.exe processes can exhaust this machine and crash with
rem STATUS_ACCESS_VIOLATION; incremental dev builds may safely use two jobs.
set "PATH=C:\Windows\System32;C:\Program Files\nodejs;%APPDATA%\npm;%USERPROFILE%\.cargo\bin;%PATH%"
set "CARGO_BUILD_JOBS=2"
if not exist "src-tauri\target\debug\scene_vault.exe" set "CARGO_BUILD_JOBS=1"
set "RAYON_NUM_THREADS=4"
set "TOKIO_WORKER_THREADS=4"

echo Starting Scene Vault on CPU 0-3 with %CARGO_BUILD_JOBS% Cargo build job(s)...
start "Scene Vault Dev - 4 P cores" /affinity F /wait cmd.exe /d /s /c "pnpm tauri dev"
exit /b %errorlevel%
