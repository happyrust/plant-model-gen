#!/bin/bash
# 验证模型导出路径重构的脚本

TASK_ID="test_path_refactor_$(date +%s)"
REFNO="7124353125105622438" # 使用已知的 refno

echo "🚀 发送 api_show_by_refno 请求 (refno: $REFNO)..."

# 端口 8080 是 WebServer 端口
# api_show_by_refno 实际路径是 /api/model/show-by-refno
RESPONSE=$(curl -s -X POST http://localhost:8080/api/model/show-by-refno \
  -H "Content-Type: application/json" \
  -d "{
    \"refnos\": [\"$REFNO\"],
    \"regen_model\": true
  }")

echo "收到的响应: $RESPONSE"

# 尝试使用 api_generate_by_refno，它会生成明确的任务路径
# api_generate_by_refno 实际路径是 /api/model/generate-by-refno
echo -e "\n🚀 发送 api_generate_by_refno 请求..."
GEN_RESPONSE=$(curl -s -X POST http://localhost:8080/api/model/generate-by-refno \
  -H "Content-Type: application/json" \
  -d "{
    \"db_num\": 8020,
    \"refnos\": [\"$REFNO\"],
    \"task_id\": \"$TASK_ID\",
    \"export_json\": true
  }")

echo "任务创建响应: $GEN_RESPONSE"

# 等待任务完成
echo "等待 5 秒让任务执行..."
sleep 5

TARGET_DIR="output/tasks/$TASK_ID"

if [ -d "$TARGET_DIR" ]; then
    echo "✅ 任务目录存在: $TARGET_DIR"
    
    echo "📂 目录结构内容:"
    ls -R "$TARGET_DIR"
    
    if [ -d "$TARGET_DIR/archetypes" ]; then
        echo "❌ 错误: archetypes 目录仍然存在！"
    else
        echo "✅ Archetypes 目录已移除。"
    fi
    
    if [ -f "$TARGET_DIR/manifest.json" ]; then
        echo "📜 检查 manifest.json 文件内容..."
        cat "$TARGET_DIR/manifest.json" | grep -A 5 "geometry_url"
    else
        echo "❌ 错误: manifest.json 未生成！"
    fi
else
    echo "❌ 错误: 任务目录不存在: $TARGET_DIR"
    echo "💡 检查 output 目录下的最新任务:"
    ls -t output/tasks | head -n 1
fi
