#!/usr/bin/env bash
# 在 Ubuntu 服务器上启动 SurrealDB（RocksDB），监听 0.0.0.0:8020。
# 数据目录默认：/root/surreal_data/ams-8020.db（与 sync_surreal_8020_to_remote.sh 一致）
set -euo pipefail

SURREAL_USER="${SURREAL_USER:-root}"
SURREAL_PASS="${SURREAL_PASS:-root}"
BIND="${SURREAL_BIND:-0.0.0.0:8020}"
DATA_DIR="${REMOTE_SURREAL_DATA_DIR:-/root/surreal_data}"
DB_DIR_NAME="${SURREAL_DB_DIR_NAME:-ams-8020.db}"

SURREAL_BIN="${SURREAL_BIN:-surreal}"
ROCKS_PATH="rocksdb://${DATA_DIR}/${DB_DIR_NAME}"

if ! command -v "$SURREAL_BIN" >/dev/null 2>&1; then
  echo "找不到 surreal 可执行文件（当前 SURREAL_BIN=$SURREAL_BIN）。请先安装或 export SURREAL_BIN=/绝对路径/surreal" >&2
  exit 1
fi

exec "$SURREAL_BIN" start --user "$SURREAL_USER" --pass "$SURREAL_PASS" --bind "$BIND" "$ROCKS_PATH"
