@echo off
REM Bootstrap agent-browser via dedicated Chrome (connect mode).
REM Forwards all args to scripts\agent-browser-up.ps1.
pwsh -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\agent-browser-up.ps1" %*
