#!/usr/bin/env bash

set -euo pipefail

REMOTE_HOST="${REMOTE_HOST:-123.57.182.243}"
REMOTE_USER="${REMOTE_USER:-root}"
REMOTE_PASS="${REMOTE_PASS:-}"
REMOTE_DATA_PATH="${REMOTE_DATA_PATH:-/root/surreal_data}"
REMOTE_SHELL_PATH="${REMOTE_SHELL_PATH:-/root/shells}"

DB_NAME="${DB_NAME:-ams-8021.db}"
LOCAL_DB_PATH="${LOCAL_DB_PATH:-/Volumes/DPC/work/plant-code/gen-model-fork/ams-8021.db}"
LOCAL_RUN_SCRIPT="${LOCAL_RUN_SCRIPT:-/Volumes/DPC/work/plant-code/gen-model-fork/shells/run_surreal_8021.sh}"

SSH_OPTS=(
  -o PreferredAuthentications=password
  -o PubkeyAuthentication=no
  -o KbdInteractiveAuthentication=no
  -o StrictHostKeyChecking=no
  -o UserKnownHostsFile=/dev/null
)

log() {
  printf '[deploy-surreal-8021] %s\n' "$*"
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    printf 'Missing required command: %s\n' "$1" >&2
    exit 1
  }
}

need_cmd sshpass
need_cmd rsync

[[ -n "$REMOTE_PASS" ]] || { printf 'REMOTE_PASS is required\n' >&2; exit 1; }
[[ -e "$LOCAL_DB_PATH" ]] || { printf 'LOCAL_DB_PATH not found: %s\n' "$LOCAL_DB_PATH" >&2; exit 1; }
[[ -f "$LOCAL_RUN_SCRIPT" ]] || { printf 'LOCAL_RUN_SCRIPT not found: %s\n' "$LOCAL_RUN_SCRIPT" >&2; exit 1; }

run_remote() {
  sshpass -p "$REMOTE_PASS" ssh "${SSH_OPTS[@]}" "$REMOTE_USER@$REMOTE_HOST" "$@"
}

log "Preparing remote directories"
run_remote "set -e; mkdir -p '$REMOTE_DATA_PATH' '$REMOTE_SHELL_PATH'"

log "Syncing database $DB_NAME"
if [[ -d "$LOCAL_DB_PATH" ]]; then
  sshpass -p "$REMOTE_PASS" rsync -avz -e "ssh ${SSH_OPTS[*]}" \
    "$LOCAL_DB_PATH/" "$REMOTE_USER@$REMOTE_HOST:$REMOTE_DATA_PATH/$DB_NAME/"
else
  sshpass -p "$REMOTE_PASS" rsync -avz -e "ssh ${SSH_OPTS[*]}" \
    "$LOCAL_DB_PATH" "$REMOTE_USER@$REMOTE_HOST:$REMOTE_DATA_PATH/$DB_NAME"
fi

log "Syncing run script"
sshpass -p "$REMOTE_PASS" rsync -avz -e "ssh ${SSH_OPTS[*]}" \
  "$LOCAL_RUN_SCRIPT" "$REMOTE_USER@$REMOTE_HOST:$REMOTE_SHELL_PATH/run_surreal_8021.sh"
run_remote "chmod +x '$REMOTE_SHELL_PATH/run_surreal_8021.sh'"

log "Starting SurrealDB on remote (nohup)"
run_remote "cd '$REMOTE_DATA_PATH' && nohup '$REMOTE_SHELL_PATH/run_surreal_8021.sh' > surreal_8021.log 2>&1 &"

log "Verifying port 8021"
sleep 2
run_remote "set -e; (command -v ss >/dev/null 2>&1 && ss -ltnp | grep -q ':8021') || (command -v netstat >/dev/null 2>&1 && netstat -tuln | grep -q ':8021')"

log "Done"
