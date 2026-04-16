#!/usr/bin/env bash
set -euo pipefail

# ============================================================
# 异地协同本地快速测试脚本
# 用途：在单机上模拟两个站点通过 MQTT 同步增量数据
# 前置：brew install mosquitto
# ============================================================

MQTT_PORT=${MQTT_PORT:-1883}
SITE_A_WEB_PORT=${SITE_A_WEB_PORT:-3100}
SITE_B_WEB_PORT=${SITE_B_WEB_PORT:-3200}
ADMIN_USER=${ADMIN_USER:-admin}
ADMIN_PASS=${ADMIN_PASS:-admin}

echo "=== 异地协同测试 ==="
echo "MQTT: localhost:${MQTT_PORT}"
echo "Site A: :${SITE_A_WEB_PORT}"
echo "Site B: :${SITE_B_WEB_PORT}"

# Step 1: 启动 MQTT broker（后台）
echo ""
echo "--- Step 1: 启动 Mosquitto ---"
if pgrep -f "mosquitto.*-p ${MQTT_PORT}" > /dev/null 2>&1; then
  echo "Mosquitto 已在运行"
else
  mosquitto -p "${MQTT_PORT}" -d
  echo "Mosquitto 已启动 (port ${MQTT_PORT})"
fi

# Step 2: 登录获取 token
echo ""
echo "--- Step 2: 登录 Admin ---"
TOKEN=$(curl -s -X POST "http://localhost:${SITE_A_WEB_PORT}/api/admin/auth/login" \
  -H "Content-Type: application/json" \
  -d "{\"username\":\"${ADMIN_USER}\",\"password\":\"${ADMIN_PASS}\"}" \
  | python3 -c "import sys,json; print(json.load(sys.stdin).get('data',{}).get('token',''))" 2>/dev/null || echo "")

if [ -z "${TOKEN}" ]; then
  echo "⚠️  登录失败或 Admin 未配置认证，尝试无 token 模式"
  AUTH_HEADER=""
else
  echo "✅ 登录成功"
  AUTH_HEADER="Authorization: Bearer ${TOKEN}"
fi

# Step 3: 创建协同组
echo ""
echo "--- Step 3: 创建协同组 ---"
ENV_RESPONSE=$(curl -s -X POST "http://localhost:${SITE_A_WEB_PORT}/api/remote-sync/envs" \
  -H "Content-Type: application/json" \
  ${AUTH_HEADER:+-H "$AUTH_HEADER"} \
  -d "{
    \"name\": \"local-test-sync\",
    \"mqtt_host\": \"localhost\",
    \"mqtt_port\": ${MQTT_PORT},
    \"file_server_host\": \"http://localhost:${SITE_B_WEB_PORT}\",
    \"location\": \"test-site-a\",
    \"location_dbs\": null
  }")
echo "Response: ${ENV_RESPONSE}"

ENV_ID=$(echo "${ENV_RESPONSE}" | python3 -c "import sys,json; print(json.load(sys.stdin).get('id',''))" 2>/dev/null || echo "")
if [ -z "${ENV_ID}" ]; then
  echo "⚠️  协同组创建失败或已存在"
  echo "可以手动到 http://localhost:${SITE_A_WEB_PORT}/admin/#/collaboration 操作"
  exit 1
fi
echo "✅ 协同组 ID: ${ENV_ID}"

# Step 4: 添加远端站点
echo ""
echo "--- Step 4: 添加远端站点 ---"
SITE_RESPONSE=$(curl -s -X POST "http://localhost:${SITE_A_WEB_PORT}/api/remote-sync/envs/${ENV_ID}/sites" \
  -H "Content-Type: application/json" \
  ${AUTH_HEADER:+-H "$AUTH_HEADER"} \
  -d "{
    \"name\": \"site-b-shanghai\",
    \"location\": \"shanghai\",
    \"http_host\": \"http://localhost:${SITE_B_WEB_PORT}\",
    \"dbnums\": null,
    \"notes\": \"本地测试站点 B\"
  }")
echo "Response: ${SITE_RESPONSE}"

# Step 5: 诊断 MQTT
echo ""
echo "--- Step 5: 诊断 MQTT 连通性 ---"
MQTT_TEST=$(curl -s -X POST "http://localhost:${SITE_A_WEB_PORT}/api/remote-sync/envs/${ENV_ID}/test-mqtt" \
  -H "Content-Type: application/json" \
  ${AUTH_HEADER:+-H "$AUTH_HEADER"})
echo "MQTT 诊断: ${MQTT_TEST}"

# Step 6: 诊断 HTTP
echo ""
echo "--- Step 6: 诊断 HTTP 连通性 ---"
HTTP_TEST=$(curl -s -X POST "http://localhost:${SITE_A_WEB_PORT}/api/remote-sync/envs/${ENV_ID}/test-http" \
  -H "Content-Type: application/json" \
  ${AUTH_HEADER:+-H "$AUTH_HEADER"})
echo "HTTP 诊断: ${HTTP_TEST}"

# Step 7: 激活协同组
echo ""
echo "--- Step 7: 激活协同组 ---"
ACTIVATE_RESPONSE=$(curl -s -X POST "http://localhost:${SITE_A_WEB_PORT}/api/remote-sync/envs/${ENV_ID}/activate" \
  -H "Content-Type: application/json" \
  ${AUTH_HEADER:+-H "$AUTH_HEADER"})
echo "激活结果: ${ACTIVATE_RESPONSE}"

# Step 8: 发送 MQTT 测试消息
echo ""
echo "--- Step 8: 发送 MQTT 测试消息 ---"
mosquitto_pub -h localhost -p "${MQTT_PORT}" \
  -t "sync/e3d" \
  -m "{\"file_names\":[\"test-increment.e3d\"],\"file_hashes\":[],\"file_server_host\":\"http://localhost:${SITE_B_WEB_PORT}\",\"location\":\"shanghai\",\"timestamp\":\"$(date -u +%Y-%m-%dT%H:%M:%SZ)\"}"
echo "✅ MQTT 消息已发送"

# Step 9: 等待并查看日志
echo ""
echo "--- Step 9: 查看同步日志 ---"
sleep 2
LOGS=$(curl -s "http://localhost:${SITE_A_WEB_PORT}/api/remote-sync/logs?limit=5" \
  ${AUTH_HEADER:+-H "$AUTH_HEADER"})
echo "最近日志: ${LOGS}" | python3 -m json.tool 2>/dev/null || echo "${LOGS}"

echo ""
echo "=== 测试完成 ==="
echo "可以访问 http://localhost:${SITE_A_WEB_PORT}/admin/#/collaboration 查看完整状态"
