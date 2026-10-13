@echo off
setlocal
cd /d "%~dp0"

rem Keep Windows build tools on CPU 0-9. The application restores its
rem runtime affinity at startup; build children keep this mask. A clean
rem build is serialized because several
rem concurrent rustc/cl.exe processes can exhaust this machine and crash with
rem STATUS_ACCESS_VIOLATION; incremental dev builds may safely use two jobs.
set "PATH=C:\Windows\System32;C:\Program Files\nodejs;%APPDATA%\npm;%USERPROFILE%\.cargo\bin;%PATH%"
set "SCENE_VAULT_DEV_RESTORE_AFFINITY=1"
set "CARGO_BUILD_JOBS=2"
if not exist "src-tauri\target\debug\scene_vault.exe" set "CARGO_BUILD_JOBS=1"

echo Building Scene Vault on CPU 0-9 with %CARGO_BUILD_JOBS% Cargo build job(s)...
start "Scene Vault Dev - Build CPU 0-9" /affinity 3FF /wait cmd.exe /d /s /c "pnpm tauri dev"
exit /b %errorlevel%
