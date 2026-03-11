#!/bin/bash
# E3D 文件上传和解析测试脚本

set -e

BASE_URL="http://localhost:8080"
E3D_FILE="${1:-test_data/sample.e3d}"
PROJECT_NAME="${2:-test_project}"

echo "=========================================="
echo "E3D 远程上传解析测试"
echo "=========================================="
echo "服务器地址: $BASE_URL"
echo "E3D 文件: $E3D_FILE"
echo "项目名称: $PROJECT_NAME"
echo ""

# 检查文件是否存在
if [ ! -f "$E3D_FILE" ]; then
    echo "❌ 错误: 文件不存在 $E3D_FILE"
    exit 1
fi

echo "1️⃣  上传 E3D 文件..."
RESPONSE=$(curl -s -X POST "$BASE_URL/api/upload/e3d" \
    -F "file=@$E3D_FILE" \
    -F "project_name=$PROJECT_NAME")

echo "响应: $RESPONSE"
echo ""

# 提取 task_id
TASK_ID=$(echo "$RESPONSE" | grep -o '"task_id":"[^"]*"' | cut -d'"' -f4)

if [ -z "$TASK_ID" ]; then
    echo "❌ 上传失败，未获取到 task_id"
    exit 1
fi

echo "✅ 上传成功，任务ID: $TASK_ID"
echo ""

# 轮询任务状态
echo "2️⃣  查询解析状态..."
MAX_ATTEMPTS=60
ATTEMPT=0

while [ $ATTEMPT -lt $MAX_ATTEMPTS ]; do
    sleep 2
    ATTEMPT=$((ATTEMPT + 1))
    
    STATUS_RESPONSE=$(curl -s "$BASE_URL/api/upload/task/$TASK_ID")
    echo "[$ATTEMPT/$MAX_ATTEMPTS] $STATUS_RESPONSE"
    
    # 检查状态
    if echo "$STATUS_RESPONSE" | grep -q '"status":"completed"'; then
        echo ""
        echo "✅ 解析完成！"
        break
    elif echo "$STATUS_RESPONSE" | grep -q '"status":"failed"'; then
        echo ""
        echo "❌ 解析失败"
        echo "$STATUS_RESPONSE"
        exit 1
    fi
done

if [ $ATTEMPT -eq $MAX_ATTEMPTS ]; then
    echo ""
    echo "⏱️  超时：解析未在预期时间内完成"
    exit 1
fi

echo ""
echo "3️⃣  测试数据查询 API..."

# 查询 World Root
echo "查询 World Root..."
curl -s "$BASE_URL/api/e3d/world-root" | head -c 200
echo ""

echo ""
echo "=========================================="
echo "✅ 测试完成"
echo "=========================================="
