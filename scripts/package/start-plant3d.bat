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
echo Unknown argument: %~1
goto usage_error

:args_done
set "ROOT=%~dp0"
if not exist "%ROOT%bin\web_server.exe" if exist "%CD%\bin\web_server.exe" set "ROOT=%CD%\"
set "WEB_SERVER=%ROOT%bin\web_server.exe"
set "LOG_DIR=%ROOT%logs"
set "OUT_LOG=%LOG_DIR%\web_server.out.log"
set "ERR_LOG=%LOG_DIR%\web_server.err.log"
set "ADMIN_URL=http://127.0.0.1:%PORT%/admin/#/sites"

if not exist "%WEB_SERVER%" (
    echo ERROR: web_server.exe not found: "%WEB_SERVER%"
    exit /b 1
)
if not exist "%LOG_DIR%" mkdir "%LOG_DIR%" >nul 2>nul

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
echo Usage: start-plant3d.bat [/Port 3100] [/Config db_options\DbOption] [/NoBrowser] [/Wait]
echo.
echo Starts bin\web_server.exe from the installation root without using PowerShell.
echo Logs are written to logs\web_server.out.log and logs\web_server.err.log.
exit /b 0

:usage_error
echo Usage: start-plant3d.bat [/Port 3100] [/Config db_options\DbOption] [/NoBrowser] [/Wait]
exit /b 2
