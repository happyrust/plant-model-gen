#!/usr/bin/env bash

set -euo pipefail

# Orchestrate backend deploy, frontend deploy, and remote verification.
#
# Usage examples:
#
# 1. Deploy from local build (default):
#    ./deploy_all_with_frontend.sh
#
# 2. Deploy from GitHub Actions artifact:
#    BINARY_SOURCE=github-artifact GITHUB_RUN_ID=12345678 ./deploy_all_with_frontend.sh
#
# 3. Deploy from GitHub release tag:
#    BINARY_SOURCE=github-release GITHUB_TAG=v1.0.0 ./deploy_all_with_frontend.sh
#
# 4. Deploy with custom artifact name:
#    BINARY_SOURCE=github-artifact GITHUB_RUN_ID=12345678 ARTIFACT_NAME=linux-x64-release ./deploy_all_with_frontend.sh

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BACKEND_PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
# 默认与后端仓库同级目录下的 plant3d-web；可通过 FRONTEND_PROJECT_DIR 覆盖
if [[ -z "${FRONTEND_PROJECT_DIR:-}" ]]; then
  _fe_sibling="$BACKEND_PROJECT_DIR/../plant3d-web"
  if [[ -d "$_fe_sibling" ]]; then
    FRONTEND_PROJECT_DIR="$(cd "$_fe_sibling" && pwd)"
  else
    FRONTEND_PROJECT_DIR="/Volumes/DPC/work/plant-code/plant3d-web"
  fi
fi

REMOTE_HOST="${REMOTE_HOST:-123.57.182.243}"
REMOTE_USER="${REMOTE_USER:-root}"
REMOTE_PASS="${REMOTE_PASS:-Happytest123_}"
BACKEND_ORIGIN="${BACKEND_ORIGIN:-http://127.0.0.1:3100}"

# Backend deployment options
BINARY_SOURCE="${BINARY_SOURCE:-local}"
BUILD_BINARY="${BUILD_BINARY:-true}"
GITHUB_RUN_ID="${GITHUB_RUN_ID:-}"
GITHUB_TAG="${GITHUB_TAG:-}"
ARTIFACT_NAME="${ARTIFACT_NAME:-linux-x64-release}"

BACKEND_SCRIPT="$BACKEND_PROJECT_DIR/shells/deploy_web_server_bundle.sh"
FRONTEND_SCRIPT="$FRONTEND_PROJECT_DIR/deploy/deploy_frontend_bundle.sh"

SSH_OPTS=(
  -o PreferredAuthentications=password
  -o PubkeyAuthentication=no
  -o KbdInteractiveAuthentication=no
  -o StrictHostKeyChecking=no
  -o UserKnownHostsFile=/dev/null
)

log() {
  printf '[deploy-all] %s\n' "$*"
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    printf 'Missing required command: %s\n' "$1" >&2
    exit 1
  }
}

# Retry wrapper for transient SSH failures
retry_with_backoff() {
  local max_attempts=5
  local delay=2
  local attempt=1
  local exit_code=0

  while [[ $attempt -le $max_attempts ]]; do
    if "$@"; then
      return 0
    else
      exit_code=$?
      if [[ $attempt -lt $max_attempts ]]; then
        log "Attempt $attempt/$max_attempts failed (exit code $exit_code), retrying in ${delay}s..."
        sleep "$delay"
        delay=$((delay * 2))
        attempt=$((attempt + 1))
      else
        log "All $max_attempts attempts failed"
        return $exit_code
      fi
    fi
  done
}

need_cmd sshpass
[[ -x "$BACKEND_SCRIPT" ]] || { printf 'Backend deploy script is missing or not executable: %s\n' "$BACKEND_SCRIPT" >&2; exit 1; }
[[ -x "$FRONTEND_SCRIPT" ]] || { printf 'Frontend deploy script is missing or not executable: %s\n' "$FRONTEND_SCRIPT" >&2; exit 1; }

log "Deploying backend (BINARY_SOURCE=$BINARY_SOURCE)"
REMOTE_HOST="$REMOTE_HOST" \
  REMOTE_USER="$REMOTE_USER" \
  REMOTE_PASS="$REMOTE_PASS" \
  BINARY_SOURCE="$BINARY_SOURCE" \
  BUILD_BINARY="$BUILD_BINARY" \
  GITHUB_RUN_ID="$GITHUB_RUN_ID" \
  GITHUB_TAG="$GITHUB_TAG" \
  ARTIFACT_NAME="$ARTIFACT_NAME" \
  "$BACKEND_SCRIPT"

log "Deploying frontend"
REMOTE_HOST="$REMOTE_HOST" REMOTE_USER="$REMOTE_USER" REMOTE_PASS="$REMOTE_PASS" BACKEND_ORIGIN="$BACKEND_ORIGIN" \
  "$FRONTEND_SCRIPT"

log "Verifying remote health"
run_ssh_with_retry() {
  local attempt=1
  local max_attempts=5
  local delay=2
  
  while [[ $attempt -le $max_attempts ]]; do
    if sshpass -p "$REMOTE_PASS" ssh "${SSH_OPTS[@]}" "$REMOTE_USER@$REMOTE_HOST" "$@"; then
      return 0
    else
      local exit_code=$?
      if [[ $attempt -lt $max_attempts ]]; then
        log "SSH verification attempt $attempt/$max_attempts failed (exit code $exit_code), retrying in ${delay}s..."
        sleep "$delay"
        delay=$((delay * 2))
        attempt=$((attempt + 1))
      else
        log "All $max_attempts SSH verification attempts failed"
        return $exit_code
      fi
    fi
  done
}

run_curl_with_retry() {
  local attempt=1
  local max_attempts=5
  local delay=2
  
  while [[ $attempt -le $max_attempts ]]; do
    if curl -fsS "$@" >/dev/null; then
      return 0
    else
      local exit_code=$?
      if [[ $attempt -lt $max_attempts ]]; then
        log "curl attempt $attempt/$max_attempts failed (exit code $exit_code), retrying in ${delay}s..."
        sleep "$delay"
        delay=$((delay * 2))
        attempt=$((attempt + 1))
      else
        log "All $max_attempts curl attempts failed"
        return $exit_code
      fi
    fi
  done
}

run_ssh_with_retry "set -e; \
  systemctl is-active web-server; \
  systemctl is-active nginx; \
  curl -fsS ${BACKEND_ORIGIN%/}/ >/dev/null; \
  curl -fsS ${BACKEND_ORIGIN%/}/api/projects >/dev/null"

run_curl_with_retry "http://$REMOTE_HOST/"
run_curl_with_retry "http://$REMOTE_HOST/api/projects"

log "Backend, frontend, and proxy verification passed"
