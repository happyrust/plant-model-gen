#!/usr/bin/env bash
# Ubuntu 生产环境：固定用 SurrealKV 数据目录，监听 0.0.0.0:8020（与 DbOption surreal_port / 前端 VITE_SURREAL_URL 一致）。
# 由 systemd surrealdb.service 调用；也可手动执行排查。
set -euo pipefail

SURREAL_BIN="${SURREAL_BIN:-/usr/local/bin/surreal}"
SURREAL_USER="${SURREAL_USER:-root}"
SURREAL_PASS="${SURREAL_PASS:-root}"
SURREAL_BIND="${SURREAL_BIND:-0.0.0.0:8020}"
# 与历史部署 surrealdb.service 中 surrealkv 路径保持一致
SURREAL_STORAGE="${SURREAL_STORAGE:-surrealkv:///var/lib/surrealdb/data}"

if [[ ! -x "$SURREAL_BIN" ]] && ! command -v "$SURREAL_BIN" >/dev/null 2>&1; then
  echo "找不到 surreal: $SURREAL_BIN" >&2
  exit 1
fi

exec "$SURREAL_BIN" start --log info \
  --user "$SURREAL_USER" \
  --pass "$SURREAL_PASS" \
  --bind "$SURREAL_BIND" \
  "$SURREAL_STORAGE"
