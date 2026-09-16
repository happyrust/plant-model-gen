@echo off
setlocal

set "PORT=3100"
set "PORT_COUNT=10"
set "DB_PORT=8020"
set "VIEWER_PORT=80"
set "TASK_NAME=Plant3D-AIOS"
set "KEEP_NGINX=0"
set "KEEP_SURREAL=0"
set "SKIP_SCHEDULED_TASK=0"
set "FORCE_PORT_KILL=0"

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
if /I "%~1"=="/portcount" (
    set "PORT_COUNT=%~2"
    shift
    shift
    goto parse_args
)
if /I "%~1"=="-portcount" (
    set "PORT_COUNT=%~2"
    shift
    shift
    goto parse_args
)
if /I "%~1"=="/dbport" (
    set "DB_PORT=%~2"
    shift
    shift
    goto parse_args
)
if /I "%~1"=="-dbport" (
    set "DB_PORT=%~2"
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
if /I "%~1"=="/taskname" (
    set "TASK_NAME=%~2"
    shift
    shift
    goto parse_args
)
if /I "%~1"=="-taskname" (
    set "TASK_NAME=%~2"
    shift
    shift
    goto parse_args
)
if /I "%~1"=="/keepnginx" (
    set "KEEP_NGINX=1"
    shift
    goto parse_args
)
if /I "%~1"=="-keepnginx" (
    set "KEEP_NGINX=1"
    shift
    goto parse_args
)
if /I "%~1"=="/keepsurreal" (
    set "KEEP_SURREAL=1"
    shift
    goto parse_args
)
if /I "%~1"=="-keepsurreal" (
    set "KEEP_SURREAL=1"
    shift
    goto parse_args
)
if /I "%~1"=="/skipscheduledtask" (
    set "SKIP_SCHEDULED_TASK=1"
    shift
    goto parse_args
)
if /I "%~1"=="-skipscheduledtask" (
    set "SKIP_SCHEDULED_TASK=1"
    shift
    goto parse_args
)
if /I "%~1"=="/forceportkill" (
    set "FORCE_PORT_KILL=1"
    shift
    goto parse_args
)
if /I "%~1"=="-forceportkill" (
    set "FORCE_PORT_KILL=1"
    shift
    goto parse_args
)
echo Unknown argument: %~1
goto usage_error

:args_done
set "ROOT=%~dp0"
if not exist "%ROOT%bin\web_server.exe" if exist "%CD%\bin\web_server.exe" set "ROOT=%CD%\"
set "PS_STOP=%ROOT%stop-plant3d.ps1"

if exist "%PS_STOP%" (
    set "PS_KEEP_NGINX_ARG="
    set "PS_KEEP_SURREAL_ARG="
    set "PS_SKIP_TASK_ARG="
    set "PS_FORCE_PORT_ARG="
    if "%KEEP_NGINX%"=="1" set "PS_KEEP_NGINX_ARG=-KeepNginx"
    if "%KEEP_SURREAL%"=="1" set "PS_KEEP_SURREAL_ARG=-KeepSurreal"
    if "%SKIP_SCHEDULED_TASK%"=="1" set "PS_SKIP_TASK_ARG=-SkipScheduledTask"
    if "%FORCE_PORT_KILL%"=="1" set "PS_FORCE_PORT_ARG=-ForcePortKill"
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%PS_STOP%" -Port %PORT% -PortCount %PORT_COUNT% -DbPort %DB_PORT% -ViewerPort %VIEWER_PORT% -TaskName "%TASK_NAME%" %PS_KEEP_NGINX_ARG% %PS_KEEP_SURREAL_ARG% %PS_SKIP_TASK_ARG% %PS_FORCE_PORT_ARG%
    exit /b %ERRORLEVEL%
)

echo stop-plant3d.ps1 not found; using fallback process cleanup.
taskkill /F /IM web_server.exe >nul 2>nul
taskkill /F /IM aios-database.exe >nul 2>nul
if "%KEEP_NGINX%"=="0" taskkill /F /IM nginx.exe >nul 2>nul
if "%KEEP_SURREAL%"=="0" taskkill /F /IM surreal.exe >nul 2>nul

call :kill_port_range %PORT% %PORT_COUNT%
call :kill_port %DB_PORT%
call :kill_port %VIEWER_PORT%
exit /b 0

:kill_port_range
set /a END_PORT=%~1+%~2-1
for /l %%N in (%~1,1,%END_PORT%) do call :kill_port %%N
exit /b 0

:kill_port
if "%~1"=="" exit /b 0
if "%~1"=="0" exit /b 0
for /f "tokens=5" %%P in ('netstat -ano ^| findstr ":%~1 " ^| findstr "LISTENING"') do (
    if not "%%P"=="0" (
        echo [kill-port] port %~1 PID=%%P
        taskkill /F /PID %%P >nul 2>nul
    )
)
exit /b 0

:usage
echo Usage: stop-plant3d.bat [/Port 3100] [/PortCount 10] [/DbPort 8020] [/ViewerPort 80] [/TaskName Plant3D-AIOS] [/KeepNginx] [/KeepSurreal] [/SkipScheduledTask] [/ForcePortKill]
echo.
echo Stops the packaged Plant3D AIOS admin service started by start-plant3d.bat.
exit /b 0

:usage_error
echo Usage: stop-plant3d.bat [/Port 3100] [/PortCount 10] [/DbPort 8020] [/ViewerPort 80] [/TaskName Plant3D-AIOS] [/KeepNginx] [/KeepSurreal] [/SkipScheduledTask] [/ForcePortKill]
exit /b 2
