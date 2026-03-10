#!/usr/bin/env bash

set -euo pipefail

# Build and deploy web_server with required runtime directories.

REMOTE_HOST="${REMOTE_HOST:-123.57.182.243}"
REMOTE_USER="${REMOTE_USER:-root}"
REMOTE_PASS="${REMOTE_PASS:-Happytest123_}"
SERVICE_NAME="${SERVICE_NAME:-web-server}"

BUILD_BINARY="${BUILD_BINARY:-true}"
BINARY_SOURCE="${BINARY_SOURCE:-local}"
BUILD_PROFILE="${BUILD_PROFILE:-release}"
TARGET="${TARGET:-}"
BINARY_NAME="${BINARY_NAME:-web_server}"
FEATURES="${FEATURES:-ws,gen_model,manifold,project_hd,surreal-save,sqlite-index,web_server,parquet-export}"
GITHUB_RUN_ID="${GITHUB_RUN_ID:-}"
GITHUB_TAG="${GITHUB_TAG:-}"
ARTIFACT_NAME="${ARTIFACT_NAME:-linux-x64-release}"
LOCAL_BIN_OVERRIDE="${LOCAL_BIN_OVERRIDE:-}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

ASSETS_DIR="$PROJECT_DIR/assets"
OUTPUT_DIR="$PROJECT_DIR/output"
DB_OPTION_FILE="${DB_OPTION_FILE:-$PROJECT_DIR/db_options/DbOption.toml}"

SSH_OPTS=(
  -o PreferredAuthentications=password
  -o PubkeyAuthentication=no
  -o KbdInteractiveAuthentication=no
  -o StrictHostKeyChecking=no
  -o UserKnownHostsFile=/dev/null
)

if [[ -n "$TARGET" ]]; then
  TARGET_DIR="$PROJECT_DIR/target/$TARGET/$BUILD_PROFILE"
else
  TARGET_DIR="$PROJECT_DIR/target/$BUILD_PROFILE"
fi

LOCAL_BIN="$TARGET_DIR/$BINARY_NAME"
ARTIFACT_DOWNLOAD_DIR="$PROJECT_DIR/.tmp/github-artifacts/$ARTIFACT_NAME"
REMOTE_BIN_NEW="/root/${BINARY_NAME}.new"
REMOTE_BIN="/root/$BINARY_NAME"
REMOTE_ASSETS_DIR="/root/assets"
REMOTE_OUTPUT_DIR="/root/output"
REMOTE_DB_OPTION="/root/DbOption.toml"

log() {
  printf '[deploy-web-server] %s\n' "$*"
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    printf 'Missing required command: %s\n' "$1" >&2
    exit 1
  }
}

run_remote() {
  sshpass -p "$REMOTE_PASS" ssh "${SSH_OPTS[@]}" "$REMOTE_USER@$REMOTE_HOST" "$@"
}

run_rsync() {
  sshpass -p "$REMOTE_PASS" rsync -az --delete -e "ssh ${SSH_OPTS[*]}" "$@"
}

need_cmd sshpass
need_cmd rsync

[[ -d "$ASSETS_DIR" ]] || { printf 'Missing assets directory: %s\n' "$ASSETS_DIR" >&2; exit 1; }
[[ -d "$OUTPUT_DIR" ]] || { printf 'Missing output directory: %s\n' "$OUTPUT_DIR" >&2; exit 1; }
[[ -f "$DB_OPTION_FILE" ]] || { printf 'Missing DbOption file: %s\n' "$DB_OPTION_FILE" >&2; exit 1; }

if [[ -n "$LOCAL_BIN_OVERRIDE" ]]; then
  LOCAL_BIN="$LOCAL_BIN_OVERRIDE"
fi

case "$BINARY_SOURCE" in
  local)
    if [[ "$BUILD_BINARY" == "true" ]]; then
      need_cmd cargo
      log "Building $BINARY_NAME ($BUILD_PROFILE)..."
      if [[ "$BUILD_PROFILE" == "release" ]]; then
        if [[ -n "$TARGET" ]]; then
          cargo build --manifest-path "$PROJECT_DIR/Cargo.toml" --bin "$BINARY_NAME" --release --no-default-features --features "$FEATURES" --target "$TARGET"
        else
          cargo build --manifest-path "$PROJECT_DIR/Cargo.toml" --bin "$BINARY_NAME" --release --no-default-features --features "$FEATURES"
        fi
      else
        if [[ -n "$TARGET" ]]; then
          cargo build --manifest-path "$PROJECT_DIR/Cargo.toml" --bin "$BINARY_NAME" --no-default-features --features "$FEATURES" --target "$TARGET"
        else
          cargo build --manifest-path "$PROJECT_DIR/Cargo.toml" --bin "$BINARY_NAME" --no-default-features --features "$FEATURES"
        fi
      fi
    else
      log "Skipping build because BUILD_BINARY=false"
    fi
    ;;
  github-artifact)
    need_cmd gh
    [[ -n "$GITHUB_RUN_ID" ]] || { printf 'GITHUB_RUN_ID is required when BINARY_SOURCE=github-artifact\n' >&2; exit 1; }
    rm -rf "$ARTIFACT_DOWNLOAD_DIR"
    mkdir -p "$ARTIFACT_DOWNLOAD_DIR"
    log "Downloading artifact $ARTIFACT_NAME from run $GITHUB_RUN_ID"
    gh run download "$GITHUB_RUN_ID" -n "$ARTIFACT_NAME" -D "$ARTIFACT_DOWNLOAD_DIR" --repo happyrust/plant-model-gen
    LOCAL_BIN="$ARTIFACT_DOWNLOAD_DIR/$BINARY_NAME"
    [[ -f "$LOCAL_BIN" ]] || { printf 'Binary not found in artifact. Downloaded files:\n' >&2; ls -la "$ARTIFACT_DOWNLOAD_DIR" >&2; exit 1; }
    chmod +x "$LOCAL_BIN"
    if [[ -f "$ARTIFACT_DOWNLOAD_DIR/BUILD_INFO.txt" ]]; then
      log "Artifact build info:"
      cat "$ARTIFACT_DOWNLOAD_DIR/BUILD_INFO.txt"
    fi
    ;;
  github-release)
    need_cmd gh
    [[ -n "$GITHUB_TAG" ]] || { printf 'GITHUB_TAG is required when BINARY_SOURCE=github-release\n' >&2; exit 1; }
    rm -rf "$ARTIFACT_DOWNLOAD_DIR"
    mkdir -p "$ARTIFACT_DOWNLOAD_DIR"
    log "Downloading release asset $BINARY_NAME from tag $GITHUB_TAG"
    gh release download "$GITHUB_TAG" --repo happyrust/plant-model-gen --pattern "$BINARY_NAME" --dir "$ARTIFACT_DOWNLOAD_DIR"
    LOCAL_BIN="$ARTIFACT_DOWNLOAD_DIR/$BINARY_NAME"
    [[ -f "$LOCAL_BIN" ]] || { printf 'Binary not found in release. Downloaded files:\n' >&2; ls -la "$ARTIFACT_DOWNLOAD_DIR" >&2; exit 1; }
    chmod +x "$LOCAL_BIN"
    if gh release download "$GITHUB_TAG" --repo happyrust/plant-model-gen --pattern "BUILD_INFO.txt" --dir "$ARTIFACT_DOWNLOAD_DIR" 2>/dev/null; then
      log "Release build info:"
      cat "$ARTIFACT_DOWNLOAD_DIR/BUILD_INFO.txt"
    fi
    ;;
  *)
    printf 'Unsupported BINARY_SOURCE: %s\n' "$BINARY_SOURCE" >&2
    exit 1
    ;;
esac

if [[ "$BINARY_SOURCE" == "github-artifact" ]]; then
  if [[ ! -f "$LOCAL_BIN" ]]; then
    if [[ -f "$ARTIFACT_DOWNLOAD_DIR/${BINARY_NAME}.exe" ]]; then
      LOCAL_BIN="$ARTIFACT_DOWNLOAD_DIR/${BINARY_NAME}.exe"
    fi
  fi
fi

[[ -f "$LOCAL_BIN" ]] || { printf 'Missing built binary: %s\n' "$LOCAL_BIN" >&2; exit 1; }

log "Preparing remote directories and stopping service"
run_remote "set -e; mkdir -p '$REMOTE_ASSETS_DIR' '$REMOTE_OUTPUT_DIR'; systemctl stop '$SERVICE_NAME' || true"

log "Uploading binary"
sshpass -p "$REMOTE_PASS" rsync -az --chmod=755 -e "ssh ${SSH_OPTS[*]}" "$LOCAL_BIN" "$REMOTE_USER@$REMOTE_HOST:$REMOTE_BIN_NEW"

log "Uploading DbOption.toml"
sshpass -p "$REMOTE_PASS" rsync -az -e "ssh ${SSH_OPTS[*]}" "$DB_OPTION_FILE" "$REMOTE_USER@$REMOTE_HOST:$REMOTE_DB_OPTION"

log "Uploading assets/"
run_rsync "$ASSETS_DIR/" "$REMOTE_USER@$REMOTE_HOST:$REMOTE_ASSETS_DIR/"

log "Uploading output/"
run_rsync "$OUTPUT_DIR/" "$REMOTE_USER@$REMOTE_HOST:$REMOTE_OUTPUT_DIR/"

log "Activating binary and restarting service"
run_remote "set -e; mv '$REMOTE_BIN_NEW' '$REMOTE_BIN'; chmod +x '$REMOTE_BIN'; systemctl daemon-reload || true; systemctl restart '$SERVICE_NAME'; systemctl is-active '$SERVICE_NAME'"

log "Deployment finished"
log "Remote binary: $REMOTE_BIN"
log "Remote assets: $REMOTE_ASSETS_DIR"
log "Remote output: $REMOTE_OUTPUT_DIR"
