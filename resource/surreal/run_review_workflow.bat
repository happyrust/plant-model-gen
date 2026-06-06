@echo off
REM 将 http 地址取自环境变量
if not defined HTTP_ADDR set HTTP_ADDR=http://127.0.0.1:8009
REM database name
if not defined DATABASE_NAME set DATABASE_NAME=AvevaMarineSample
REM namespace
if not defined NAMESPACE set NAMESPACE=1516

surreal import --conn %HTTP_ADDR% --namespace %NAMESPACE% --database %DATABASE_NAME% -u root -p root review_workflow.surql

