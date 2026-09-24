@echo off
rem 双击即可开始采样；结果写到 %LOCALAPPDATA%\com.scenevault.desktop\diagnostics
setlocal
set "PWSH=%ProgramFiles%\PowerShell\7\pwsh.exe"
if not exist "%PWSH%" set "PWSH=pwsh"
"%PWSH%" -NoLogo -NoProfile -NoExit -ExecutionPolicy Bypass -File "%~dp0monitor-scene-vault.ps1" %*
endlocal
