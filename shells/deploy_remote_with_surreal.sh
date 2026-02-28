#!/bin/bash

set -euo pipefail

# ==============================================================================
# Web Server 一键部署脚本（含 surreal 资源目录）
# - 目标: 123.57.182.243
# - 方式: sshpass + scp/ssh
# ==============================================================================

# --- 远端配置 ---
REMOTE_HOST="${REMOTE_HOST:-123.57.182.243}"
REMOTE_USER="${REMOTE_USER:-root}"
REMOTE_PASS="${REMOTE_PASS:-Happytest123_}"
SERVICE_NAME="${SERVICE_NAME:-web-server.service}"
SSH_OPTS="-o PreferredAuthentications=password -o PubkeyAuthentication=no -o KbdInteractiveAuthentication=no -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null"

# --- 构建配置 ---
BUILD_BINARY="${BUILD_BINARY:-false}" # true: 重新编译
TARGET="${TARGET:-x86_64-unknown-linux-gnu.2.35}" # 兼容 Ubuntu 22
BINARY_NAME="${BINARY_NAME:-web_server}"
FEATURES="${FEATURES:-web_server,parquet-export}"

# --- 本地路径 ---
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
LOCAL_BIN="${PROJECT_DIR}/target/x86_64-unknown-linux-gnu/debug/${BINARY_NAME}"
LOCAL_CONFIG="${PROJECT_DIR}/db_options/DbOption.toml"
LOCAL_RS_CORE_DIR="${LOCAL_RS_CORE_DIR:-/Volumes/DPC/work/plant-code/rs-core}"
LOCAL_SURREAL_DIR="${LOCAL_RS_CORE_DIR}/resource/surreal"
TMP_TAR="/tmp/surreal_resource_$(date +%Y%m%d_%H%M%S).tar.gz"

# --- 远端路径 ---
REMOTE_BIN_TMP="/root/${BINARY_NAME}.new"
REMOTE_BIN="/root/${BINARY_NAME}"
REMOTE_CONFIG="/root/DbOption.toml"
REMOTE_TAR="/root/surreal_resource.tar.gz"
REMOTE_RESOURCE_ROOT="/root/resource"
REMOTE_SURREAL_DIR="/root/resource/surreal"

echo "[1/6] 预检查本地文件"
[[ -f "${LOCAL_CONFIG}" ]] || { echo "缺少配置文件: ${LOCAL_CONFIG}"; exit 1; }
[[ -d "${LOCAL_SURREAL_DIR}" ]] || { echo "缺少目录: ${LOCAL_SURREAL_DIR}"; exit 1; }

if [[ "${BUILD_BINARY}" == "true" ]]; then
  echo "[2/6] 使用 cargo-zigbuild 编译 ${BINARY_NAME} (${TARGET})"
  cd "${PROJECT_DIR}"
  cargo zigbuild --bin "${BINARY_NAME}" --target "${TARGET}" --features "${FEATURES}"
fi

[[ -f "${LOCAL_BIN}" ]] || {
  echo "缺少二进制产物: ${LOCAL_BIN}"
  echo "可设置 BUILD_BINARY=true 自动编译。"
  exit 1
}

echo "[3/6] 打包 rs-core/resource/surreal"
tar -czf "${TMP_TAR}" -C "${LOCAL_RS_CORE_DIR}" resource/surreal

echo "[4/6] 上传二进制/配置/surreal 压缩包"
sshpass -p "${REMOTE_PASS}" scp ${SSH_OPTS} \
  "${LOCAL_BIN}" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_BIN_TMP}"
sshpass -p "${REMOTE_PASS}" scp ${SSH_OPTS} \
  "${LOCAL_CONFIG}" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_CONFIG}"
sshpass -p "${REMOTE_PASS}" scp ${SSH_OPTS} \
  "${TMP_TAR}" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_TAR}"

echo "[5/6] 远端解包 surreal 目录 + 替换程序 + 重启服务"
sshpass -p "${REMOTE_PASS}" ssh ${SSH_OPTS} \
  "${REMOTE_USER}@${REMOTE_HOST}" "
    set -e
    mkdir -p '${REMOTE_RESOURCE_ROOT}'
    rm -rf '${REMOTE_SURREAL_DIR}'
    tar -xzf '${REMOTE_TAR}' -C /root
    if grep -q '^surreal_script_dir[[:space:]]*=' '${REMOTE_CONFIG}'; then
      sed -i 's#^surreal_script_dir[[:space:]]*=.*#surreal_script_dir = \"resource/surreal\"#' '${REMOTE_CONFIG}'
    else
      printf '\nsurreal_script_dir = \"resource/surreal\"\n' >> '${REMOTE_CONFIG}'
    fi
    mv '${REMOTE_BIN_TMP}' '${REMOTE_BIN}'
    chmod +x '${REMOTE_BIN}'
    systemctl daemon-reload || true
    systemctl restart '${SERVICE_NAME}'
  "

echo "[6/6] 验证服务与资源目录"
sshpass -p "${REMOTE_PASS}" ssh ${SSH_OPTS} \
  "${REMOTE_USER}@${REMOTE_HOST}" "
    set -e
    systemctl is-active '${SERVICE_NAME}'
    pgrep -af '^/root/${BINARY_NAME}' || true
    ls -ld '${REMOTE_SURREAL_DIR}'
    find '${REMOTE_SURREAL_DIR}' -maxdepth 1 -type f -name '*.surql' | wc -l
    ss -ltnp | grep 8080 || true
    code=\$(curl -s -o /tmp/web_home.txt -w '%{http_code}' -m 8 http://127.0.0.1:8080/ || true)
    echo \"home_http_code=\$code\"
    grep -n '^surreal_script_dir' '${REMOTE_CONFIG}'
  "

rm -f "${TMP_TAR}"
echo "部署完成。"
