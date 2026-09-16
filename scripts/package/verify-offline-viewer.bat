@echo off
setlocal

set "ROOT=%~1"
if "%ROOT%"=="" set "ROOT=%~dp0."

powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0verify-offline-viewer.ps1" -Root "%ROOT%"
exit /b %ERRORLEVEL%
