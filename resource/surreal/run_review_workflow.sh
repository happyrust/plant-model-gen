#!/bin/bash
set -euo pipefail

# 将 http 地址取自环境变量
: "${HTTP_ADDR:=http://127.0.0.1:8009}"
: "${DATABASE_NAME:=AvevaMarineSample}"
: "${NAMESPACE:=1516}"

surreal import --conn "$HTTP_ADDR" --namespace "$NAMESPACE" --database "$DATABASE_NAME" -u root -p root review_workflow.surql

