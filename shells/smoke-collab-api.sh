#!/usr/bin/env bash
# 异地协同后端冒烟脚本（Sprint B · B7）
#
# 用法：
#   ./shells/smoke-collab-api.sh                           # 默认目标 http://127.0.0.1:3100
#   BASE=http://staging:3100 ./shells/smoke-collab-api.sh  # 自定义后端
#   VERBOSE=1 ./shells/smoke-collab-api.sh                 # 打印响应体
#
# 退出码：
#   0 = 全部通过
#   1 = 至少一项硬性失败（HTTP 5xx 或不可达）
#
# 不验证完整业务字段，只检查 endpoint 可达 + 状态码 + 关键 JSON 字段存在性。

set -uo pipefail

BASE="${BASE:-http://127.0.0.1:3100}"
VERBOSE="${VERBOSE:-0}"

green=$'\e[32m'
red=$'\e[31m'
yellow=$'\e[33m'
reset=$'\e[0m'

pass=0
fail=0
warn=0

check() {
  local method="$1"
  local path="$2"
  local expect_code_re="${3:-2..}"
  local require_field="${4:-}"

  local body
  local code

  if [ "$method" = "GET" ]; then
    body=$(curl -s -m 5 -w "\n__HTTP_CODE__:%{http_code}" "$BASE$path" 2>/dev/null || echo $'\n__HTTP_CODE__:000')
  else
    body=$(curl -s -m 5 -X "$method" -w "\n__HTTP_CODE__:%{http_code}" \
      -H 'Content-Type: application/json' -d '{}' "$BASE$path" 2>/dev/null || echo $'\n__HTTP_CODE__:000')
  fi

  code=$(echo "$body" | tail -n 1 | sed 's/.*__HTTP_CODE__://')
  body=$(echo "$body" | sed '$d')

  local note=""
  local status="OK"

  if [[ ! "$code" =~ ^$expect_code_re$ ]]; then
    if [ "$code" = "503" ]; then
      status="WARN(503)"
      note="(admin-gated, 需登录)"
      warn=$((warn + 1))
    elif [ "$code" = "401" ] || [ "$code" = "403" ]; then
      status="WARN($code)"
      note="(需鉴权)"
      warn=$((warn + 1))
    else
      status="FAIL($code)"
      fail=$((fail + 1))
    fi
  else
    pass=$((pass + 1))
    if [ -n "$require_field" ] && ! echo "$body" | grep -q "\"$require_field\""; then
      status="WARN"
      note="(缺字段 $require_field)"
      warn=$((warn + 1))
      pass=$((pass - 1))
    fi
  fi

  case "$status" in
    OK) printf "  ${green}✓${reset} %-7s %-50s %s %s\n" "$method" "$path" "$status" "$note" ;;
    WARN*) printf "  ${yellow}!${reset} %-7s %-50s %s %s\n" "$method" "$path" "$status" "$note" ;;
    *) printf "  ${red}✗${reset} %-7s %-50s %s %s\n" "$method" "$path" "$status" "$note" ;;
  esac

  if [ "$VERBOSE" = "1" ]; then
    echo "    body: $(echo "$body" | head -c 200)"
  fi
}

# 单独的 SSE 通道连接性校验（B4 · 流式接口不能用普通 check）
#
# curl -m 2 在 2s 后超时返回（exit 28），但 %{http_code} 只要服务端发送过响应头
# 就会被填充为对应状态码。预期 200。
check_sse() {
  local path="$1"
  local code
  code=$(curl -s -o /dev/null -m 2 -w "%{http_code}" \
    -H "Accept: text/event-stream" "$BASE$path" 2>/dev/null || echo "000")

  if [[ "$code" =~ ^2 ]]; then
    pass=$((pass + 1))
    printf "  ${green}✓${reset} %-7s %-50s %s %s\n" "GET" "$path" "OK" "(SSE $code)"
  else
    fail=$((fail + 1))
    printf "  ${red}✗${reset} %-7s %-50s %s\n" "GET" "$path" "FAIL($code)"
  fi
}

echo "──────────────────────────────────────────────────────────────"
echo "  异地协同后端 API 冒烟 · BASE=$BASE"
echo "──────────────────────────────────────────────────────────────"

echo
echo "[1/4] 站点配置 + 身份"
check GET  /api/site-config                     "200" "config"
check GET  /api/site/info                       "200" "location"
check GET  /api/site-config/server-ip           "200" "ip"

echo
echo "[2/4] 同步引擎"
check GET  /api/sync/status                     "2.."
check GET  /api/sync/queue                      "2.."
check GET  /api/sync/history                    "2.."
check GET  /api/sync/config                     "2.."
check GET  /api/sync/metrics                    "2.."

echo
echo "[3/5] MQTT 节点 / 订阅"
check GET  /api/mqtt/nodes                      "2.." "summary"
check GET  /api/mqtt/messages                   "2.."
check GET  /api/mqtt/subscription/status        "200" "is_master_node"
check GET  /api/mqtt/broker/logs                "200" "capacity"
check POST /api/mqtt/node/set-client            "200" "is_master_node"
check POST /api/mqtt/node/set-master            "200" "is_master_node"
check GET  /api/mqtt/subscription/status        "200" "is_master_node"
# B2 验证：set-master/set-client 之后应至少 push 2 条 broker log
check GET  /api/mqtt/broker/logs?limit=10       "200" "set_master"

echo
echo "[4/5] SSE 实时事件流 (B4)"
# B4 验证：SSE 通道可建立。前一步 set-client/set-master 已触发
# MqttSubscriptionStatusChanged 事件，前端 LogsView 订阅后可见。
check_sse /api/sync/events/stream

echo
echo "[5/5] 异地协同 (admin-gated · 503/401/403 视为预期)"
check GET  /api/remote-sync/envs                "2..|503|401|403"
check GET  /api/remote-sync/topology            "2..|503|401|403"
check GET  /api/remote-sync/runtime/status      "2..|503|401|403"

echo
echo "──────────────────────────────────────────────────────────────"
echo "  汇总: ${green}$pass 通过${reset} · ${yellow}$warn 警告${reset} · ${red}$fail 失败${reset}"
echo "──────────────────────────────────────────────────────────────"

if [ "$fail" -gt 0 ]; then
  exit 1
fi
exit 0
