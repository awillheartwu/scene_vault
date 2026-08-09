@echo off
rem One-click Scene Vault AI setup: double-click this file, or run it from
rem a terminal. It closes nothing by itself; close the app first so the
rem database settings can be written.
setlocal
cd /d "%~dp0"
echo Starting Scene Vault setup...
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0setup-windows.ps1"
echo.
pause
