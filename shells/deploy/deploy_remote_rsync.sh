#!/bin/bash

set -euo pipefail

# ==============================================================================
# Web Server RSYNC 一键部署脚本
# 目标: 123.57.182.243 (Ubuntu 22.04 x86_64)
# ==============================================================================

REMOTE_HOST="${REMOTE_HOST:-123.57.182.243}"
REMOTE_USER="${REMOTE_USER:-root}"
REMOTE_PASS="${REMOTE_PASS:-Happytest123_}"
SERVICE_NAME="${SERVICE_NAME:-web-server}"

BUILD_BINARY="${BUILD_BINARY:-false}"        # true 时重新编译
BUILD_PROFILE="${BUILD_PROFILE:-release}"     # release / debug
TARGET="${TARGET:-x86_64-unknown-linux-gnu}"  # 交叉编译目标
BINARY_NAME="${BINARY_NAME:-web_server}"
FEATURES="${FEATURES:-ws,gen_model,manifold,project_hd,surreal-save,sqlite-index,web_server,parquet-export}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

# 二进制路径根据 profile 自动选择
LOCAL_BIN="${PROJECT_DIR}/target/${TARGET}/${BUILD_PROFILE}/${BINARY_NAME}"

# 运行时资源目录
ASSETS_DIR="${PROJECT_DIR}/assets"
OUTPUT_DIR="${PROJECT_DIR}/output"
DB_OPTION_FILE="${DB_OPTION_FILE:-${PROJECT_DIR}/db_options/DbOption-mac.toml}"

# 与 deploy_web_server_bundle.sh 一致：上传前按环境变量写入 DbOption 路径（apply_dboption_deploy_paths.py）
: "${REMOTE_PROJECT_PATH:=/root/e3d_models}"
: "${REMOTE_SURREAL_DATA_PATH:=/root/surreal_data/ams-8020.db}"
: "${REMOTE_SURREALKV_DATA_PATH:=/root/surreal_data/ams-8020.db.kv}"
: "${REMOTE_MESHES_PATH:=/root/assets/meshes}"
: "${REMOTE_SURREAL_SCRIPT_DIR:=/root/resource/surreal}"
: "${DEPLOY_APPLY_DB_PATH_OVERRIDES:=true}"

# 远端路径
REMOTE_BIN_NEW="/root/${BINARY_NAME}.new"
REMOTE_BIN="/root/${BINARY_NAME}"
REMOTE_ASSETS_DIR="/root/assets"
REMOTE_OUTPUT_DIR="/root/output"
REMOTE_DB_OPTION="/root/DbOption.toml"

SSH_OPTS="-o PreferredAuthentications=password -o PubkeyAuthentication=no -o KbdInteractiveAuthentication=no -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null"
RSYNC_RSH="sshpass -p ${REMOTE_PASS} ssh ${SSH_OPTS}"

run_remote() {
  sshpass -p "${REMOTE_PASS}" ssh ${SSH_OPTS} "${REMOTE_USER}@${REMOTE_HOST}" "$@"
}

# ── [1/7] 本地预检查 ──────────────────────────────────────────────────────────
echo "[1/7] 本地预检查"
command -v sshpass >/dev/null || { echo "缺少 sshpass"; exit 1; }
command -v rsync   >/dev/null || { echo "缺少 rsync";   exit 1; }
command -v python3 >/dev/null || { echo "缺少 python3";  exit 1; }
[[ -d "${ASSETS_DIR}" ]]     || { echo "缺少 assets 目录: ${ASSETS_DIR}"; exit 1; }
[[ -f "${DB_OPTION_FILE}" ]] || { echo "缺少配置文件: ${DB_OPTION_FILE}"; exit 1; }

# ── [2/7] 编译 ────────────────────────────────────────────────────────────────
if [[ "${BUILD_BINARY}" == "true" ]]; then
  echo "[2/7] cargo build --${BUILD_PROFILE} (target: ${TARGET})"
  cd "${PROJECT_DIR}"
  BUILD_ARGS=(
    --manifest-path "${PROJECT_DIR}/Cargo.toml"
    --bin "${BINARY_NAME}"
    --no-default-features
    --features "${FEATURES}"
    --target "${TARGET}"
  )
  if [[ "${BUILD_PROFILE}" == "release" ]]; then
    BUILD_ARGS+=(--release)
  fi
  cargo build "${BUILD_ARGS[@]}"
else
  echo "[2/7] 跳过编译 (BUILD_BINARY=false)"
fi

[[ -f "${LOCAL_BIN}" ]] || {
  echo "缺少二进制产物: ${LOCAL_BIN}"
  echo "可设置 BUILD_BINARY=true 自动编译。"
  exit 1
}
echo "  二进制文件: ${LOCAL_BIN} ($(du -h "${LOCAL_BIN}" | cut -f1))"

# ── [3/7] 远端准备：创建目录并停止服务 ────────────────────────────────────────
echo "[3/7] 远端准备目录并停止服务"
run_remote "
  set -e
  mkdir -p '${REMOTE_ASSETS_DIR}' '${REMOTE_OUTPUT_DIR}'
  systemctl stop '${SERVICE_NAME}' || true
"

# ── [4/7] rsync 同步二进制 ────────────────────────────────────────────────────
echo "[4/7] rsync 同步二进制"
rsync -avz --inplace --chmod=755 -e "${RSYNC_RSH}" \
  "${LOCAL_BIN}" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_BIN_NEW}"

# ── [5/7] rsync 同步配置和资源目录 ────────────────────────────────────────────
echo "[5/7] rsync 同步配置和资源"
# DbOption.toml（环境变量覆盖路径）
DB_OPTION_UPLOAD="$(mktemp)"
cp "${DB_OPTION_FILE}" "${DB_OPTION_UPLOAD}"
if [[ "${DEPLOY_APPLY_DB_PATH_OVERRIDES}" == "true" ]]; then
  echo "  按 REMOTE_* 环境变量写入 DbOption 路径后上传"
  _helper="${SCRIPT_DIR}/apply_dboption_deploy_paths.py"
  [[ -f "${_helper}" ]] || { echo "缺少 ${_helper}"; exit 1; }
  _out="$(mktemp)"
  python3 "${_helper}" "${DB_OPTION_UPLOAD}" "${_out}" \
    --project-path "${REMOTE_PROJECT_PATH}" \
    --meshes-path "${REMOTE_MESHES_PATH}" \
    --surreal-script-dir "${REMOTE_SURREAL_SCRIPT_DIR}" \
    --surreal-data-path "${REMOTE_SURREAL_DATA_PATH}" \
    --surrealkv-path "${REMOTE_SURREALKV_DATA_PATH}"
  mv "${_out}" "${DB_OPTION_UPLOAD}"
fi
rsync -avz -e "${RSYNC_RSH}" \
  "${DB_OPTION_UPLOAD}" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_DB_OPTION}"
rm -f "${DB_OPTION_UPLOAD}"

# assets/
rsync -avz --delete -e "${RSYNC_RSH}" \
  "${ASSETS_DIR}/" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_ASSETS_DIR}/"

# output/ (可能不存在则跳过)
if [[ -d "${OUTPUT_DIR}" ]]; then
  rsync -avz --delete -e "${RSYNC_RSH}" \
    "${OUTPUT_DIR}/" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_OUTPUT_DIR}/"
else
  echo "  跳过 output/ (本地目录不存在)"
fi

# ── [6/7] 激活二进制并重启服务 ────────────────────────────────────────────────
echo "[6/7] 激活二进制并重启服务"
run_remote "
  set -e
  mv '${REMOTE_BIN_NEW}' '${REMOTE_BIN}'
  chmod +x '${REMOTE_BIN}'
  systemctl daemon-reload || true
  systemctl restart '${SERVICE_NAME}'
"

# ── [7/7] 验证 ────────────────────────────────────────────────────────────────
echo "[7/7] 验证部署"
run_remote "
  set -e
  systemctl is-active '${SERVICE_NAME}'
  pgrep -af '^/root/${BINARY_NAME}' || true
  ls -lh '${REMOTE_BIN}'
  ss -ltnp | grep 8080 || true
  code=\$(curl -s -o /dev/null -w '%{http_code}' -m 10 http://127.0.0.1:8080/ || true)
  echo \"home_http_code=\$code\"
"

echo "✅ 部署完成。"
