#!/usr/bin/env bash
# 将本地 SurrealDB RocksDB 目录同步到 Ubuntu 服务器并注册 systemd 服务 surreal-8020。
#
# 依赖：sshpass、rsync；远端需已安装与数据兼容的 surreal 二进制。
#
# 示例：
#   REMOTE_HOST=123.57.182.243 REMOTE_USER=root REMOTE_PASS='...' ./shells/sync_surreal_8020_to_remote.sh
#
# 可选环境变量：
#   LOCAL_SURREAL_DB   本地 rocksdb 目录，默认 /Volumes/DPC/work/db-data/ams-8020.db
#   LOCAL_SURREAL_KV   本地 surrealkv 路径（文件或目录），默认同目录下 ams-8020.db.kv（存在才同步）
#   REMOTE_SURREAL_DATA_DIR  远端父目录，默认 /root/surreal_data
#   REMOTE_SURREAL_BIN  远端 surreal 绝对路径，默认 /usr/bin/surreal
#   INSTALL_SYSTEMD   是否写入并启用 systemd，默认 true
#   SKIP_SYSTEMD      设为 1 则只 rsync，不装 systemd（需自行启动）
#   RESTART_ONLY      设为 1 则跳过 rsync，仅 SSH 执行 systemctl restart surreal-8020
#   RSYNC_FLAGS         覆盖默认 rsync 参数（默认含进度与断点续传）

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

REMOTE_HOST="${REMOTE_HOST:-123.57.182.243}"
REMOTE_USER="${REMOTE_USER:-root}"
REMOTE_PASS="${REMOTE_PASS:-}"

LOCAL_SURREAL_DB="${LOCAL_SURREAL_DB:-/Volumes/DPC/work/db-data/ams-8020.db}"
LOCAL_SURREAL_KV="${LOCAL_SURREAL_KV:-/Volumes/DPC/work/db-data/ams-8020.db.kv}"
REMOTE_SURREAL_DATA_DIR="${REMOTE_SURREAL_DATA_DIR:-/root/surreal_data}"
REMOTE_DB_DIR_NAME="${REMOTE_DB_DIR_NAME:-ams-8020.db}"
# 多数手动安装的 surreal 在 /usr/local/bin；包管理器可能在 /usr/bin
REMOTE_SURREAL_BIN="${REMOTE_SURREAL_BIN:-/usr/local/bin/surreal}"
INSTALL_SYSTEMD="${INSTALL_SYSTEMD:-true}"
SKIP_SYSTEMD="${SKIP_SYSTEMD:-0}"
RESTART_ONLY="${RESTART_ONLY:-0}"
RSYNC_FLAGS_DEFAULT='-az --delete --partial --info=progress2'
RSYNC_FLAGS="${RSYNC_FLAGS:-$RSYNC_FLAGS_DEFAULT}"

SSH_OPTS=(
  -o PreferredAuthentications=password
  -o PubkeyAuthentication=no
  -o KbdInteractiveAuthentication=no
  -o StrictHostKeyChecking=no
  -o UserKnownHostsFile=/dev/null
)

REMOTE_DB_PATH="${REMOTE_SURREAL_DATA_DIR}/${REMOTE_DB_DIR_NAME}"
REMOTE_RUN_HELPER="/root/shells/run_surreal_8020_ubuntu.sh"

log() { printf '[sync-surreal] %s\n' "$*"; }

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    printf '缺少命令: %s\n' "$1" >&2
    exit 1
  }
}

run_remote() {
  sshpass -p "$REMOTE_PASS" ssh "${SSH_OPTS[@]}" "$REMOTE_USER@$REMOTE_HOST" "$@"
}

run_rsync() {
  # shellcheck disable=SC2086
  sshpass -p "$REMOTE_PASS" rsync $RSYNC_FLAGS -e "ssh ${SSH_OPTS[*]}" "$@"
}

retry_rsync() {
  local attempt=1 max=5 delay=2
  while [[ "$attempt" -le "$max" ]]; do
    if run_rsync "$@"; then
      return 0
    fi
    log "rsync 第 ${attempt}/${max} 次失败，${delay}s 后重试..."
    sleep "$delay"
    delay=$((delay * 2))
    attempt=$((attempt + 1))
  done
  return 1
}

[[ -n "$REMOTE_PASS" ]] || { printf '请设置 REMOTE_PASS\n' >&2; exit 1; }
need_cmd sshpass
need_cmd rsync

log "远端: ${REMOTE_USER}@${REMOTE_HOST}"

if [[ "$RESTART_ONLY" == "1" ]]; then
  log "RESTART_ONLY=1：仅重启 surreal-8020"
  run_remote "set -e; systemctl restart surreal-8020; sleep 2; systemctl is-active surreal-8020"
  log "已重启 surreal-8020"
  exit 0
fi

if [[ ! -e "$LOCAL_SURREAL_DB" ]]; then
  printf '本地库不存在: %s\n' "$LOCAL_SURREAL_DB" >&2
  exit 1
fi

log "本地库: $LOCAL_SURREAL_DB -> ${REMOTE_DB_PATH}/"

log "停止远端 surreal-8020（若存在）并释放 8020"
run_remote "set -e; systemctl stop surreal-8020 2>/dev/null || true; if command -v fuser >/dev/null 2>&1; then fuser -k 8020/tcp 2>/dev/null || true; fi; sleep 2"

log "准备远端目录"
run_remote "set -e; mkdir -p '${REMOTE_SURREAL_DATA_DIR}' /root/shells"

if [[ -d "$LOCAL_SURREAL_DB" ]]; then
  log "rsync 目录（含删除远端多余文件）"
  retry_rsync "${LOCAL_SURREAL_DB}/" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_DB_PATH}/"
else
  log "rsync 单文件"
  retry_rsync "$LOCAL_SURREAL_DB" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_DB_PATH}"
fi

if [[ -e "$LOCAL_SURREAL_KV" ]]; then
  log "同步 SurrealKV: $LOCAL_SURREAL_KV"
  if [[ -d "$LOCAL_SURREAL_KV" ]]; then
    retry_rsync "${LOCAL_SURREAL_KV}/" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_SURREAL_DATA_DIR}/$(basename "$LOCAL_SURREAL_KV")/"
  else
    retry_rsync "$LOCAL_SURREAL_KV" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_SURREAL_DATA_DIR}/$(basename "$LOCAL_SURREAL_KV")"
  fi
fi

log "安装远端启动脚本"
sshpass -p "$REMOTE_PASS" scp "${SSH_OPTS[@]}" "$SCRIPT_DIR/run_surreal_8020_ubuntu.sh" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_RUN_HELPER}"
run_remote "chmod +x '${REMOTE_RUN_HELPER}'"

if [[ "$SKIP_SYSTEMD" == "1" ]]; then
  log "SKIP_SYSTEMD=1，跳过 systemd。请手动: ${REMOTE_RUN_HELPER}"
  exit 0
fi

if [[ "$INSTALL_SYSTEMD" != "true" ]]; then
  log "INSTALL_SYSTEMD!=true，跳过 systemd"
  exit 0
fi

UNIT_PATH=/etc/systemd/system/surreal-8020.service
log "写入 systemd: ${UNIT_PATH}"

TMP_UNIT="$(mktemp)"
trap 'rm -f "$TMP_UNIT"' EXIT
cat > "$TMP_UNIT" <<EOF
[Unit]
Description=SurrealDB plant (8020, rocksdb)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=${REMOTE_SURREAL_DATA_DIR}
Environment=REMOTE_SURREAL_DATA_DIR=${REMOTE_SURREAL_DATA_DIR}
Environment=SURREAL_DB_DIR_NAME=${REMOTE_DB_DIR_NAME}
Environment=SURREAL_USER=root
Environment=SURREAL_PASS=root
Environment=SURREAL_BIND=0.0.0.0:8020
Environment=SURREAL_BIN=${REMOTE_SURREAL_BIN}
ExecStart=${REMOTE_RUN_HELPER}
Restart=on-failure
RestartSec=5
LimitNOFILE=1048576

[Install]
WantedBy=multi-user.target
EOF

sshpass -p "$REMOTE_PASS" scp "${SSH_OPTS[@]}" "$TMP_UNIT" "${REMOTE_USER}@${REMOTE_HOST}:/tmp/surreal-8020.service"
run_remote "set -e; mv /tmp/surreal-8020.service '${UNIT_PATH}'; systemctl daemon-reload; systemctl enable surreal-8020; systemctl restart surreal-8020; sleep 2; systemctl is-active surreal-8020"

log "完成。本机可检查: curl -sS http://${REMOTE_HOST}:8020/health 或浏览器 ws://${REMOTE_HOST}:8020/rpc"
