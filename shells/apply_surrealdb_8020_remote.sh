#!/usr/bin/env bash
# 将 run_surrealdb_kv_8020.sh + systemd 单元同步到 Ubuntu，启用 8020，并重启 surrealdb、web-server。
#
#   REMOTE_HOST=... REMOTE_USER=root REMOTE_PASS='...' ./shells/apply_surrealdb_8020_remote.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

REMOTE_HOST="${REMOTE_HOST:-123.57.182.243}"
REMOTE_USER="${REMOTE_USER:-root}"
REMOTE_PASS="${REMOTE_PASS:-}"

SSH_OPTS=(
  -o PreferredAuthentications=password
  -o PubkeyAuthentication=no
  -o KbdInteractiveAuthentication=no
  -o StrictHostKeyChecking=no
  -o UserKnownHostsFile=/dev/null
)

UNIT_LOCAL="${SCRIPT_DIR}/systemd/surrealdb-8020.service"
RUN_LOCAL="${SCRIPT_DIR}/run_surrealdb_kv_8020.sh"
REMOTE_UNIT="/etc/systemd/system/surrealdb.service"
REMOTE_RUN="/root/shells/run_surrealdb_kv_8020.sh"

log() { printf '[apply-surreal-8020] %s\n' "$*"; }

[[ -n "$REMOTE_PASS" ]] || { printf '请设置 REMOTE_PASS\n' >&2; exit 1; }
[[ -f "$UNIT_LOCAL" && -f "$RUN_LOCAL" ]] || { printf '缺少文件: %s 或 %s\n' "$UNIT_LOCAL" "$RUN_LOCAL" >&2; exit 1; }
command -v sshpass >/dev/null || { printf '需要 sshpass\n' >&2; exit 1; }

run() { sshpass -p "$REMOTE_PASS" ssh "${SSH_OPTS[@]}" "$REMOTE_USER@$REMOTE_HOST" "$@"; }
scp_up() { sshpass -p "$REMOTE_PASS" scp "${SSH_OPTS[@]}" "$@"; }

log "远端 ${REMOTE_USER}@${REMOTE_HOST}"
log "停止 surrealdb / 释放 8020"
run "set -e; systemctl stop surrealdb 2>/dev/null || true; if command -v fuser >/dev/null 2>&1; then fuser -k 8020/tcp 2>/dev/null || true; fuser -k 8000/tcp 2>/dev/null || true; fi; sleep 2"

log "上传启动脚本 -> ${REMOTE_RUN}"
run "mkdir -p /root/shells"
scp_up "$RUN_LOCAL" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_RUN}"
run "chmod +x '${REMOTE_RUN}'"

log "安装 systemd -> ${REMOTE_UNIT}"
scp_up "$UNIT_LOCAL" "${REMOTE_USER}@${REMOTE_HOST}:/tmp/surrealdb.service"
run "set -e; mv /tmp/surrealdb.service '${REMOTE_UNIT}'; systemctl daemon-reload; systemctl enable surrealdb"

log "启动 surrealdb（8020）"
run "set -e; systemctl start surrealdb; sleep 3; systemctl is-active surrealdb; ss -tlnp | grep ':8020' || true"

log "重启 web-server（重连 Surreal）"
run "set -e; systemctl restart web-server; sleep 6; systemctl is-active web-server"

log "本机探测（等待 3100 就绪）"
run "set -e; for i in 1 2 3 4 5 6 7 8 9 10; do curl -sS -m 2 http://127.0.0.1:3100/api/health >/dev/null && break; sleep 1; done; curl -sS -m 15 http://127.0.0.1:3100/api/health | head -c 180; echo; curl -sS -m 20 -o /tmp/p.json -w 'projects %{http_code} t=%{time_total}s\n' http://127.0.0.1:3100/api/projects; head -c 200 /tmp/p.json; echo"

log "完成。公网 Surreal WebSocket: ws://${REMOTE_HOST}:8020（或带 /rpc，以 surreal 客户端为准）"
