@echo off
setlocal enabledelayedexpansion

set "BASE_PORT=3100"
set "PORT_COUNT=10"

if not "%~1"=="" set "BASE_PORT=%~1"
if not "%~2"=="" set "PORT_COUNT=%~2"

set /a END_PORT=%BASE_PORT%+%PORT_COUNT%-1
echo Killing listeners on ports %BASE_PORT%..%END_PORT% and web_server.exe ...

taskkill /F /IM web_server.exe >nul 2>nul

for /l %%N in (%BASE_PORT%,1,%END_PORT%) do (
    for /f "tokens=5" %%P in ('netstat -ano ^| findstr ":%%N " ^| findstr "LISTENING"') do (
        if not "%%P"=="0" (
            echo [kill-port] port %%N PID=%%P
            taskkill /F /PID %%P >nul 2>nul
        )
    )
)

timeout /t 1 /nobreak >nul 2>nul

echo Remaining listeners on 310x:
netstat -ano | findstr "LISTENING" | findstr ":310"
exit /b 0
