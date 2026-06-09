@echo off
setlocal enabledelayedexpansion

set "BASE_PORT=3100"
set "PORT_COUNT=10"

if not "%~1"=="" set "BASE_PORT=%~1"
if not "%~2"=="" set "PORT_COUNT=%~2"

set /a END_PORT=%BASE_PORT%+%PORT_COUNT%-1
echo Stopping Plant3D web_server.exe listeners on ports %BASE_PORT%..%END_PORT% ...

for /l %%N in (%BASE_PORT%,1,%END_PORT%) do (
    for /f "tokens=5" %%P in ('netstat -ano ^| findstr ":%%N " ^| findstr "LISTENING"') do (
        if not "%%P"=="0" (
            set "IMAGE_NAME="
            for /f "tokens=1" %%I in ('tasklist /FI "PID eq %%P" /NH 2^>nul') do set "IMAGE_NAME=%%I"
            if /I "!IMAGE_NAME!"=="web_server.exe" (
                echo [kill-port] port %%N PID=%%P image=!IMAGE_NAME!
                taskkill /F /PID %%P >nul 2>nul
            ) else (
                echo [skip-port] port %%N PID=%%P image=!IMAGE_NAME!
            )
        )
    )
)

timeout /t 1 /nobreak >nul 2>nul

echo Remaining listeners on 310x:
netstat -ano | findstr "LISTENING" | findstr ":310"
exit /b 0
