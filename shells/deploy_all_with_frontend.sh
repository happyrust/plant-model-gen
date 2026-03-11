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
FRONTEND_PROJECT_DIR="/Volumes/DPC/work/plant-code/plant3d-web"

REMOTE_HOST="${REMOTE_HOST:-123.57.182.243}"
REMOTE_USER="${REMOTE_USER:-root}"
REMOTE_PASS="${REMOTE_PASS:-Happytest123_}"

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
REMOTE_HOST="$REMOTE_HOST" REMOTE_USER="$REMOTE_USER" REMOTE_PASS="$REMOTE_PASS" \
  "$FRONTEND_SCRIPT"

log "Verifying remote health"
sshpass -p "$REMOTE_PASS" ssh "${SSH_OPTS[@]}" "$REMOTE_USER@$REMOTE_HOST" "set -e; \
  systemctl is-active web-server; \
  systemctl is-active nginx; \
  curl -fsS http://127.0.0.1:8080/ >/dev/null; \
  curl -fsS http://127.0.0.1:8080/api/projects >/dev/null"

curl -fsS "http://$REMOTE_HOST/" >/dev/null
curl -fsS "http://$REMOTE_HOST/api/projects" >/dev/null

log "Backend, frontend, and proxy verification passed"
