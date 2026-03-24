#!/usr/bin/env bash
# 使用 JSON 文件以 POST 方式验证 Platform API（PMS 入站接口）。
# 依赖: curl, jq（可选；无 jq 时仅打印说明并跳过需 token 的步骤）
#
# 用法:
#   export BASE_URL=http://127.0.0.1:3100
#   ./shells/verify_platform_api_json.sh
#
# 若 review_auth.enabled=false，部分接口可不传 JWT；否则先取 token 再跑 workflow/cache/delete。

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
JSON_DIR="$ROOT/shells/platform_api_json"
BASE_URL="${BASE_URL:-http://127.0.0.1:3100}"
BASE_URL="${BASE_URL%/}"

echo "== BASE_URL=$BASE_URL"

have_jq() { command -v jq >/dev/null 2>&1; }

echo ""
echo "== 1) POST /api/review/embed-url (embed_url.json)"
curl -sS -X POST "$BASE_URL/api/review/embed-url" \
  -H 'Content-Type: application/json' \
  -d @"$JSON_DIR/embed_url.json" | tee /tmp/platform_api_embed.json
echo ""

if ! have_jq; then
  echo "!! 未安装 jq：请手动将 JWT 填入 shells/platform_api_json/workflow_sync_*.json 等文件的 REPLACE_WITH_JWT 后执行 curl。"
  exit 0
fi

echo ""
echo "== 2) POST /api/auth/token (auth_token.json)"
AUTH_RAW=$(curl -sS -X POST "$BASE_URL/api/auth/token" \
  -H 'Content-Type: application/json' \
  -d @"$JSON_DIR/auth_token.json")
echo "$AUTH_RAW" | jq .

TOKEN=$(echo "$AUTH_RAW" | jq -r '.data.token // empty')
FORM_ID=$(echo "$AUTH_RAW" | jq -r '.data.form_id // empty')

if [[ -z "$TOKEN" || "$TOKEN" == "null" ]]; then
  echo "!! 未取得 token（可能 review_auth 关闭或接口失败）。跳过需鉴权的请求。"
  exit 0
fi

echo "   使用 form_id=$FORM_ID"

TMP=$(mktemp)
jq --arg t "$TOKEN" --arg f "$FORM_ID" \
  '.token = $t | .form_id = $f' \
  "$JSON_DIR/workflow_sync_query.json" >"$TMP"
echo ""
echo "== 3) POST /api/review/workflow/sync (query)"
curl -sS -X POST "$BASE_URL/api/review/workflow/sync" \
  -H 'Content-Type: application/json' \
  -d @"$TMP" | jq .
rm -f "$TMP"

TMP=$(mktemp)
jq --arg t "$TOKEN" --arg f "$FORM_ID" \
  '.token = $t | .form_id = $f' \
  "$JSON_DIR/workflow_sync_active.json" >"$TMP"
echo ""
echo "== 4) POST /api/review/workflow/sync (active)"
curl -sS -X POST "$BASE_URL/api/review/workflow/sync" \
  -H 'Content-Type: application/json' \
  -d @"$TMP" | jq .
rm -f "$TMP"

TMP=$(mktemp)
jq --arg t "$TOKEN" '.token = $t' "$JSON_DIR/cache_preload.json" >"$TMP"
echo ""
echo "== 5) POST /api/review/cache/preload"
curl -sS -X POST "$BASE_URL/api/review/cache/preload" \
  -H 'Content-Type: application/json' \
  -d @"$TMP" | jq .
rm -f "$TMP"

TMP=$(mktemp)
jq --arg t "$TOKEN" \
  --argjson ids "[\"$FORM_ID\"]" \
  '.token = $t | .form_ids = $ids' \
  "$JSON_DIR/review_delete.json" >"$TMP"
echo ""
echo "== 6) POST /api/review/delete（会删除该 form 关联数据，默认跳过）"
read -r -p "   确认执行删除? [y/N] " ans || true
if [[ "${ans:-}" =~ ^[yY]$ ]]; then
  curl -sS -X POST "$BASE_URL/api/review/delete" \
    -H 'Content-Type: application/json' \
    -d @"$TMP" | jq .
else
  echo "   已跳过。"
fi
rm -f "$TMP"

echo ""
echo "== 完成"
