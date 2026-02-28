#!/bin/bash

set -euo pipefail

# ==============================================================================
# Web Server RSYNC 一键部署脚本（含 surreal 资源目录）
# ==============================================================================

REMOTE_HOST="${REMOTE_HOST:-123.57.182.243}"
REMOTE_USER="${REMOTE_USER:-root}"
REMOTE_PASS="${REMOTE_PASS:-Happytest123_}"
SERVICE_NAME="${SERVICE_NAME:-web-server.service}"

BUILD_BINARY="${BUILD_BINARY:-false}" # true 时重新编译
TARGET="${TARGET:-x86_64-unknown-linux-gnu.2.35}" # Ubuntu 22 兼容目标
BINARY_NAME="${BINARY_NAME:-web_server}"
FEATURES="${FEATURES:-web_server,parquet-export}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
LOCAL_BIN="${PROJECT_DIR}/target/x86_64-unknown-linux-gnu/debug/${BINARY_NAME}"
LOCAL_CONFIG="${PROJECT_DIR}/db_options/DbOption.toml"
LOCAL_RS_CORE_DIR="${LOCAL_RS_CORE_DIR:-/Volumes/DPC/work/plant-code/rs-core}"
LOCAL_SURREAL_DIR="${LOCAL_RS_CORE_DIR}/resource/surreal"

REMOTE_BIN_NEW="/root/${BINARY_NAME}.new"
REMOTE_BIN="/root/${BINARY_NAME}"
REMOTE_CONFIG="/root/DbOption.toml"
REMOTE_SURREAL_DIR="/root/resource/surreal"

SSH_OPTS="-o PreferredAuthentications=password -o PubkeyAuthentication=no -o KbdInteractiveAuthentication=no -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null"
RSYNC_RSH="ssh ${SSH_OPTS}"
RSYNC_RSH_WITH_PASS="sshpass -p ${REMOTE_PASS} ssh ${SSH_OPTS}"

echo "[1/7] 本地预检查"
command -v sshpass >/dev/null || { echo "缺少 sshpass"; exit 1; }
command -v rsync >/dev/null || { echo "缺少 rsync"; exit 1; }
[[ -f "${LOCAL_CONFIG}" ]] || { echo "缺少配置文件: ${LOCAL_CONFIG}"; exit 1; }
[[ -d "${LOCAL_SURREAL_DIR}" ]] || { echo "缺少目录: ${LOCAL_SURREAL_DIR}"; exit 1; }

if [[ "${BUILD_BINARY}" == "true" ]]; then
  echo "[2/7] cargo-zigbuild 编译"
  cd "${PROJECT_DIR}"
  cargo zigbuild --bin "${BINARY_NAME}" --target "${TARGET}" --features "${FEATURES}"
else
  echo "[2/7] 跳过编译 (BUILD_BINARY=false)"
fi

[[ -f "${LOCAL_BIN}" ]] || {
  echo "缺少二进制产物: ${LOCAL_BIN}"
  echo "可设置 BUILD_BINARY=true 自动编译。"
  exit 1
}

echo "[3/7] 远端准备目录并停服务"
sshpass -p "${REMOTE_PASS}" ssh ${SSH_OPTS} "${REMOTE_USER}@${REMOTE_HOST}" "
  set -e
  mkdir -p /root/resource/surreal
  systemctl stop '${SERVICE_NAME}' || true
"

echo "[4/7] rsync 同步二进制和配置"
rsync -av --inplace --chmod=755 -e "${RSYNC_RSH_WITH_PASS}" \
  "${LOCAL_BIN}" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_BIN_NEW}"
rsync -av -e "${RSYNC_RSH_WITH_PASS}" \
  "${LOCAL_CONFIG}" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_CONFIG}"

echo "[5/7] rsync 同步 rs-core/resource/surreal"
rsync -av --delete -e "${RSYNC_RSH_WITH_PASS}" \
  "${LOCAL_SURREAL_DIR}/" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_SURREAL_DIR}/"

echo "[6/7] 修正配置并重启服务"
sshpass -p "${REMOTE_PASS}" ssh ${SSH_OPTS} "${REMOTE_USER}@${REMOTE_HOST}" "
  set -e
  if grep -q '^surreal_script_dir[[:space:]]*=' '${REMOTE_CONFIG}'; then
    sed -i 's#^surreal_script_dir[[:space:]]*=.*#surreal_script_dir = \"resource/surreal\"#' '${REMOTE_CONFIG}'
  else
    printf '\nsurreal_script_dir = \"resource/surreal\"\n' >> '${REMOTE_CONFIG}'
  fi
  mv '${REMOTE_BIN_NEW}' '${REMOTE_BIN}'
  chmod +x '${REMOTE_BIN}'
  systemctl daemon-reload || true
  systemctl restart '${SERVICE_NAME}'
"

echo "[7/7] 验证"
sshpass -p "${REMOTE_PASS}" ssh ${SSH_OPTS} "${REMOTE_USER}@${REMOTE_HOST}" "
  set -e
  systemctl is-active '${SERVICE_NAME}'
  pgrep -af '^/root/${BINARY_NAME}' || true
  ls -ld '${REMOTE_SURREAL_DIR}'
  find '${REMOTE_SURREAL_DIR}' -maxdepth 1 -type f -name '*.surql' | wc -l
  grep -n '^surreal_script_dir' '${REMOTE_CONFIG}'
  ss -ltnp | grep 8080 || true
  code=\$(curl -s -o /tmp/web_home_rsync.txt -w '%{http_code}' -m 8 http://127.0.0.1:8080/ || true)
  echo \"home_http_code=\$code\"
  journalctl -u '${SERVICE_NAME}' --since '-2 min' --no-pager | grep -E 'Failed to define common functions|初始化通用函数失败' || true
"

echo "RSYNC 部署完成。"
