#!/bin/bash
# 验证模型导出路径重构的脚本 - 强制 generate_by_refno

TASK_ID="validate_refactor_$(date +%s)"
REFNO="73965" # 使用 DB 24383 的 refno
DBNO="24383"

echo "🚀 发送 api_generate_by_refno 请求..."
GEN_RESPONSE=$(curl -s -X POST http://localhost:8080/api/model/generate-by-refno \
  -H "Content-Type: application/json" \
  -d "{
    \"db_num\": $DBNO,
    \"refnos\": [\"$REFNO\"],
    \"task_id\": \"$TASK_ID\",
    \"export_json\": true
  }")

echo "任务创建响应: $GEN_RESPONSE"

# 等待任务完成 (通常很快)
echo "等待 5 秒让任务执行..."
sleep 5

# 直接在 output 目录下查找 manifest.json，因为路径可能包含 task_id 两遍或结构不同
echo "🔍 正在检索生成的 manifest.json..."
FIND_RESULT=$(find output/tasks -name "manifest.json" | grep "$TASK_ID" | head -n 1)

if [ -n "$FIND_RESULT" ]; then
    echo "✅ 找到 Manifest: $FIND_RESULT"
    
    TARGET_DIR=$(dirname "$FIND_RESULT")
    echo "📂 检查目录结构: $TARGET_DIR"
    ls -R "$TARGET_DIR"
    
    if [ -d "$TARGET_DIR/archetypes" ]; then
        echo "❌ 错误: archetypes 目录仍然存在！"
    else
        echo "✅ Archetypes 目录已移除。"
    fi
    
    echo "📜 检查 manifest.json 关键内容 (geometry_url):"
    cat "$FIND_RESULT" | grep -A 5 "geometry_url"
else
    echo "❌ 错误: 未能在 output/tasks 中找到包含 $TASK_ID 的 manifest.json"
    echo "💡 当前任务列表状态:"
    curl -s http://localhost:8080/api/tasks | grep -A 10 "$TASK_ID"
fi
