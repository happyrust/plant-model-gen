@echo off
setlocal

set "ADMIN_USER=admin"
set "ADMIN_PASS=admin"
set "AIOS_ALLOW_WEAK_DB_CREDS=1"
set "AIOS_ALLOW_PUBLIC_BIND=1"

set "PORT=3100"
set "CONFIG=db_options\DbOption"
set "NO_BROWSER=0"
set "WAIT=0"
set "ENABLE_NGINX=on"
set "VIEWER_HOST="
set "VIEWER_PORT=80"
set "REQUIRE_NGINX=1"
set "REQUIRE_NGINX_EXPLICIT=0"

:parse_args
if "%~1"=="" goto args_done
if /I "%~1"=="/help" goto usage
if /I "%~1"=="-help" goto usage
if /I "%~1"=="--help" goto usage
if /I "%~1"=="/?" goto usage
if /I "%~1"=="/port" (
    set "PORT=%~2"
    shift
    shift
    goto parse_args
)
if /I "%~1"=="-port" (
    set "PORT=%~2"
    shift
    shift
    goto parse_args
)
if /I "%~1"=="/config" (
    set "CONFIG=%~2"
    shift
    shift
    goto parse_args
)
if /I "%~1"=="-config" (
    set "CONFIG=%~2"
    shift
    shift
    goto parse_args
)
if /I "%~1"=="/nobrowser" (
    set "NO_BROWSER=1"
    shift
    goto parse_args
)
if /I "%~1"=="-nobrowser" (
    set "NO_BROWSER=1"
    shift
    goto parse_args
)
if /I "%~1"=="/wait" (
    set "WAIT=1"
    shift
    goto parse_args
)
if /I "%~1"=="-wait" (
    set "WAIT=1"
    shift
    goto parse_args
)
if /I "%~1"=="/enablenginx" (
    set "ENABLE_NGINX=%~2"
    shift
    shift
    goto parse_args
)
if /I "%~1"=="-enablenginx" (
    set "ENABLE_NGINX=%~2"
    shift
    shift
    goto parse_args
)
if /I "%~1"=="/viewerhost" (
    set "VIEWER_HOST=%~2"
    shift
    shift
    goto parse_args
)
if /I "%~1"=="-viewerhost" (
    set "VIEWER_HOST=%~2"
    shift
    shift
    goto parse_args
)
if /I "%~1"=="/viewerport" (
    set "VIEWER_PORT=%~2"
    shift
    shift
    goto parse_args
)
if /I "%~1"=="-viewerport" (
    set "VIEWER_PORT=%~2"
    shift
    shift
    goto parse_args
)
if /I "%~1"=="/requirenginx" (
    set "REQUIRE_NGINX=1"
    set "REQUIRE_NGINX_EXPLICIT=1"
    shift
    goto parse_args
)
if /I "%~1"=="-requirenginx" (
    set "REQUIRE_NGINX=1"
    set "REQUIRE_NGINX_EXPLICIT=1"
    shift
    goto parse_args
)
echo Unknown argument: %~1
goto usage_error

:args_done
if "%REQUIRE_NGINX_EXPLICIT%"=="0" (
    if /I "%ENABLE_NGINX%"=="on" set "REQUIRE_NGINX=1"
    if /I "%ENABLE_NGINX%"=="auto" set "REQUIRE_NGINX=0"
    if /I "%ENABLE_NGINX%"=="off" set "REQUIRE_NGINX=0"
)

set "ROOT=%~dp0"
set "KILL_PORTS_BAT=%ROOT%kill-plant3d-ports.bat"
if exist "%KILL_PORTS_BAT%" (
    call "%KILL_PORTS_BAT%" %PORT% 10
) else (
    call :kill_port_listeners %PORT%
    call :kill_web_server_processes
)
goto after_port_cleanup

:kill_port_listeners
set "TARGET_PORT=%~1"
if "%TARGET_PORT%"=="" exit /b 0
for /f "tokens=5" %%P in ('netstat -ano ^| findstr ":%TARGET_PORT% " ^| findstr "LISTENING"') do (
    if not "%%P"=="0" (
        echo [kill-port] port %TARGET_PORT% PID=%%P
        taskkill /F /PID %%P >nul 2>nul
    )
)
exit /b 0

:kill_web_server_processes
taskkill /F /IM web_server.exe >nul 2>nul
exit /b 0

:after_port_cleanup
if not exist "%ROOT%bin\web_server.exe" if exist "%CD%\bin\web_server.exe" set "ROOT=%CD%\"
set "WEB_SERVER=%ROOT%bin\web_server.exe"
set "PS_START=%ROOT%start-plant3d.ps1"
set "LOG_DIR=%ROOT%logs"
set "OUT_LOG=%LOG_DIR%\web_server.out.log"
set "ERR_LOG=%LOG_DIR%\web_server.err.log"
set "ADMIN_URL=http://127.0.0.1:%PORT%/admin/#/sites"

if not exist "%WEB_SERVER%" (
    echo ERROR: web_server.exe not found: "%WEB_SERVER%"
    exit /b 1
)
if not exist "%LOG_DIR%" mkdir "%LOG_DIR%" >nul 2>nul

if exist "%PS_START%" (
    set "PS_WAIT_ARG="
    set "PS_BROWSER_ARG="
    set "PS_REQUIRE_NGINX_ARG="
    set "PS_VIEWER_HOST_ARG="
    if "%WAIT%"=="1" set "PS_WAIT_ARG=-Wait"
    if "%NO_BROWSER%"=="1" set "PS_BROWSER_ARG=-NoBrowser"
    if "%REQUIRE_NGINX%"=="1" set "PS_REQUIRE_NGINX_ARG=-RequireNginx"
    if not "%VIEWER_HOST%"=="" set "PS_VIEWER_HOST_ARG=-ViewerHost %VIEWER_HOST%"
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%PS_START%" -Port %PORT% -Config "%CONFIG%" %PS_BROWSER_ARG% %PS_WAIT_ARG% -EnableNginx "%ENABLE_NGINX%" %PS_VIEWER_HOST_ARG% -ViewerPort %VIEWER_PORT% %PS_REQUIRE_NGINX_ARG%
    exit /b %ERRORLEVEL%
)

set "WEB_SERVER_PORT=%PORT%"
set "PATH=%ROOT%bin\surreal;%PATH%"

echo Starting Plant3D AIOS from "%ROOT%"
echo Admin Sites: %ADMIN_URL%
echo Viewer: http://127.0.0.1:%PORT%/viewer/
echo Logs:
echo   %OUT_LOG%
echo   %ERR_LOG%

if "%WAIT%"=="1" (
    if "%NO_BROWSER%"=="0" start "" "%ADMIN_URL%"
    "%WEB_SERVER%" --config "%CONFIG%" >> "%OUT_LOG%" 2>> "%ERR_LOG%"
    exit /b %ERRORLEVEL%
)

start "Plant3D AIOS" /D "%ROOT%" cmd /c "set ADMIN_USER=%ADMIN_USER%&& set ADMIN_PASS=%ADMIN_PASS%&& set AIOS_ALLOW_WEAK_DB_CREDS=%AIOS_ALLOW_WEAK_DB_CREDS%&& set AIOS_ALLOW_PUBLIC_BIND=%AIOS_ALLOW_PUBLIC_BIND%&& set WEB_SERVER_PORT=%PORT%&& bin\web_server.exe --config %CONFIG% >> logs\web_server.out.log 2>> logs\web_server.err.log"
if errorlevel 1 (
    echo ERROR: failed to launch web_server.exe
    exit /b 1
)

if "%NO_BROWSER%"=="0" (
    timeout /t 6 /nobreak >nul 2>nul
    start "" "%ADMIN_URL%"
)

echo Started in background. If the page is not ready yet, wait and refresh the admin URL.
exit /b 0

:usage
echo Usage: start-plant3d.bat [/Port 3100] [/Config db_options\DbOption] [/NoBrowser] [/Wait] [/EnableNginx auto^|on^|off] [/ViewerHost host] [/ViewerPort 80] [/RequireNginx]
echo Default: /EnableNginx on, which requires Nginx configuration to succeed. Use /EnableNginx auto for fallback mode.
echo.
echo Starts bin\web_server.exe from the installation root without using PowerShell.
echo Logs are written to logs\web_server.out.log and logs\web_server.err.log.
exit /b 0

:usage_error
echo Usage: start-plant3d.bat [/Port 3100] [/Config db_options\DbOption] [/NoBrowser] [/Wait] [/EnableNginx auto^|on^|off] [/ViewerHost host] [/ViewerPort 80] [/RequireNginx]
echo Default: /EnableNginx on, which requires Nginx configuration to succeed. Use /EnableNginx auto for fallback mode.
exit /b 2
