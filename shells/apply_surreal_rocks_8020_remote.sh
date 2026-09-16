#!/usr/bin/env bash
# 在 Ubuntu 上启用「与本地一致的 RocksDB」Surreal（surreal-8020），并停用 surrealkv 的 surrealdb.service。
# 数据目录：/root/surreal_data/ams-8020.db（须先 rsync，见 sync_surreal_8020_to_remote.sh）。
#
#   REMOTE_HOST=... REMOTE_USER=root REMOTE_PASS='...' ./shells/apply_surreal_rocks_8020_remote.sh
#
# 可选：SYNC_FIRST=1 时在本机先执行完整同步（需本地存在 LOCAL_SURREAL_DB，默认 Mac 路径）

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

REMOTE_HOST="${REMOTE_HOST:-123.57.182.243}"
REMOTE_USER="${REMOTE_USER:-root}"
REMOTE_PASS="${REMOTE_PASS:-}"
SYNC_FIRST="${SYNC_FIRST:-0}"

SSH_OPTS=(
  -o PreferredAuthentications=password
  -o PubkeyAuthentication=no
  -o KbdInteractiveAuthentication=no
  -o StrictHostKeyChecking=no
  -o UserKnownHostsFile=/dev/null
)

UNIT_LOCAL="${SCRIPT_DIR}/systemd/surreal-8020-rocksdb.service"
RUN_LOCAL="${SCRIPT_DIR}/run_surreal_8020_ubuntu.sh"
REMOTE_UNIT="/etc/systemd/system/surreal-8020.service"
REMOTE_RUN="/root/shells/run_surreal_8020_ubuntu.sh"

log() { printf '[apply-surreal-rocks] %s\n' "$*"; }

[[ -n "$REMOTE_PASS" ]] || { printf '请设置 REMOTE_PASS\n' >&2; exit 1; }
[[ -f "$UNIT_LOCAL" && -f "$RUN_LOCAL" ]] || { printf '缺少文件\n' >&2; exit 1; }
command -v sshpass >/dev/null || { printf '需要 sshpass\n' >&2; exit 1; }

run() { sshpass -p "$REMOTE_PASS" ssh "${SSH_OPTS[@]}" "$REMOTE_USER@$REMOTE_HOST" "$@"; }
scp_up() { sshpass -p "$REMOTE_PASS" scp "${SSH_OPTS[@]}" "$@"; }

if [[ "$SYNC_FIRST" == "1" ]]; then
  log "SYNC_FIRST=1：先执行 sync_surreal_8020_to_remote.sh"
  REMOTE_HOST="$REMOTE_HOST" REMOTE_USER="$REMOTE_USER" REMOTE_PASS="$REMOTE_PASS" \
    REMOTE_SURREAL_BIN="${REMOTE_SURREAL_BIN:-/usr/local/bin/surreal}" \
    "${SCRIPT_DIR}/sync_surreal_8020_to_remote.sh"
  exit 0
fi

log "远端 ${REMOTE_USER}@${REMOTE_HOST}"
log "停止 surrealdb（KV）/ surreal-8020，释放 8020"
run "set -e; systemctl stop surrealdb 2>/dev/null || true; systemctl stop surreal-8020 2>/dev/null || true; systemctl disable surrealdb 2>/dev/null || true; if command -v fuser >/dev/null 2>&1; then fuser -k 8020/tcp 2>/dev/null || true; fi; sleep 2"

log "上传启动脚本 -> ${REMOTE_RUN}"
run "mkdir -p /root/shells"
scp_up "$RUN_LOCAL" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_RUN}"
run "chmod +x '${REMOTE_RUN}'"

log "安装 systemd -> ${REMOTE_UNIT}"
scp_up "$UNIT_LOCAL" "${REMOTE_USER}@${REMOTE_HOST}:/tmp/surreal-8020.service"
run "set -e; mv /tmp/surreal-8020.service '${REMOTE_UNIT}'; systemctl daemon-reload; systemctl enable surreal-8020; systemctl start surreal-8020; sleep 3; systemctl is-active surreal-8020; ss -tlnp | grep ':8020' || true"

log "重启 web-server（重连 Surreal）"
run "set -e; systemctl restart web-server; sleep 6; systemctl is-active web-server || true"

log "完成。请确认已 rsync 本地 ams-8020.db；验证: 远端执行 surreal sql 查 MDB 或 du -sh /root/surreal_data/ams-8020.db"
