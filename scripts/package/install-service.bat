@echo off
setlocal

set "TASK_NAME=Plant3D-AIOS"
set "PORT=3100"
set "RUN_NOW=0"
set "UNINSTALL=0"

:parse_args
if "%~1"=="" goto args_done
if /I "%~1"=="/help" goto usage
if /I "%~1"=="-help" goto usage
if /I "%~1"=="--help" goto usage
if /I "%~1"=="/?" goto usage
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
if /I "%~1"=="/runnow" (
    set "RUN_NOW=1"
    shift
    goto parse_args
)
if /I "%~1"=="-runnow" (
    set "RUN_NOW=1"
    shift
    goto parse_args
)
if /I "%~1"=="/uninstall" (
    set "UNINSTALL=1"
    shift
    goto parse_args
)
if /I "%~1"=="-uninstall" (
    set "UNINSTALL=1"
    shift
    goto parse_args
)
echo Unknown argument: %~1
goto usage_error

:args_done
set "ROOT=%~dp0"
if not exist "%ROOT%start-plant3d.bat" if exist "%CD%\start-plant3d.bat" set "ROOT=%CD%\"
set "START_SCRIPT=%ROOT%start-plant3d.bat"

net session >nul 2>nul
if errorlevel 1 (
    echo ERROR: please run this script as Administrator.
    exit /b 1
)

if "%UNINSTALL%"=="1" (
    schtasks.exe /Delete /TN "%TASK_NAME%" /F
    if errorlevel 1 exit /b 1
    echo Removed scheduled task: %TASK_NAME%
    exit /b 0
)

if not exist "%START_SCRIPT%" (
    echo ERROR: start script not found: "%START_SCRIPT%"
    exit /b 1
)

schtasks.exe /Create /TN "%TASK_NAME%" /SC ONLOGON /RL HIGHEST /F /TR "\"%START_SCRIPT%\" /Port %PORT% /NoBrowser"
if errorlevel 1 exit /b 1

echo Registered scheduled task: %TASK_NAME%
echo Install root: %ROOT%
echo Viewer URL: http://127.0.0.1:%PORT%/viewer/

if "%RUN_NOW%"=="1" (
    schtasks.exe /Run /TN "%TASK_NAME%"
    if errorlevel 1 exit /b 1
    echo Started scheduled task: %TASK_NAME%
)

exit /b 0

:usage
echo Usage: install-service.bat [/TaskName Plant3D-AIOS] [/Port 3100] [/RunNow] [/Uninstall]
echo.
echo Registers a Windows scheduled task without using PowerShell.
echo Run this script from an elevated Administrator command prompt.
exit /b 0

:usage_error
echo Usage: install-service.bat [/TaskName Plant3D-AIOS] [/Port 3100] [/RunNow] [/Uninstall]
exit /b 2
