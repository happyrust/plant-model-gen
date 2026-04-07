#!/usr/bin/env bash

set -euo pipefail

# Build and deploy web_server with required runtime directories.
#
# Cross-compilation support:
#   When BINARY_SOURCE=local and running on macOS, this script automatically uses
#   cargo zigbuild to cross-compile for Linux x86_64 (the remote deployment target).
#   This is controlled by the USE_ZIGBUILD and ZIGBUILD_TARGET variables.
#
# Usage examples:
#
# 1. Local build with automatic zigbuild cross-compilation (macOS → Linux):
#    ./deploy_web_server_bundle.sh
#
# 2. Local build forcing native target (no cross-compilation):
#    USE_ZIGBUILD=false ./deploy_web_server_bundle.sh
#
# 3. Deploy from GitHub Actions artifact:
#    BINARY_SOURCE=github-artifact GITHUB_RUN_ID=12345678 ./deploy_web_server_bundle.sh
#
# 4. Deploy from GitHub release tag:
#    BINARY_SOURCE=github-release GITHUB_TAG=v1.0.0 ./deploy_web_server_bundle.sh
#
# 5. 仅更新远端配置文件（不上传二进制 / assets / output）：
#    CONFIG_ONLY=true REMOTE_PASS='...' ./shells/deploy/deploy_web_server_bundle.sh
#    或 ./shells/deploy/deploy_config_only.sh
#    默认会 systemctl restart web-server（即重启 web_server 进程以加载新 DbOption）；
#    设 RESTART_AFTER_CONFIG_ONLY=false 则只写配置不重启。
#
# 上传 DbOption 前，默认按环境变量写入远端路径（由 apply_dboption_deploy_paths.py 按 TOML 区块替换键值，无 Mac 路径 sed）：
#   REMOTE_PROJECT_PATH        默认 /root/e3d_models        → 顶层 project_path
#   REMOTE_MESHES_PATH         默认 /root/assets/meshes     → 顶层 meshes_path
#   REMOTE_SURREAL_SCRIPT_DIR  默认 /root/resource/surreal   → 顶层 surreal_script_dir
#   REMOTE_SURREAL_DATA_PATH   默认 /root/surreal_data/ams-8020.db → [web_server].surreal_data_path、[surrealdb].path
#   REMOTE_SURREALKV_DATA_PATH 默认 /root/surreal_data/ams-8020.db.kv → [surrealkv].path
#   DEPLOY_APPLY_DB_PATH_OVERRIDES 默认 true；设为 false 则原样上传 DbOption（不写上述键）

REMOTE_HOST="${REMOTE_HOST:-123.57.182.243}"
REMOTE_USER="${REMOTE_USER:-root}"
REMOTE_PASS="${REMOTE_PASS:-}"
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

# Cross-compilation with cargo-zigbuild for macOS → Linux deployment
USE_ZIGBUILD="${USE_ZIGBUILD:-auto}"
ZIGBUILD_TARGET="${ZIGBUILD_TARGET:-x86_64-unknown-linux-gnu}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shells/deploy → 仓库根
PROJECT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

ASSETS_DIR="$PROJECT_DIR/assets"
OUTPUT_DIR="$PROJECT_DIR/output"
DB_OPTION_FILE="${DB_OPTION_FILE:-$PROJECT_DIR/db_options/DbOption-mac.toml}"
UPLOAD_ASSETS="${UPLOAD_ASSETS:-auto}"
UPLOAD_OUTPUT="${UPLOAD_OUTPUT:-auto}"

SSH_OPTS=(
  -o PreferredAuthentications=password
  -o PubkeyAuthentication=no
  -o KbdInteractiveAuthentication=no
  -o StrictHostKeyChecking=no
  -o UserKnownHostsFile=/dev/null
)

ARTIFACT_DOWNLOAD_DIR="$PROJECT_DIR/.tmp/github-artifacts/$ARTIFACT_NAME"
REMOTE_BIN="/root/$BINARY_NAME"
REMOTE_BIN_BACKUP="/root/${BINARY_NAME}.backup_$(date +%Y%m%d_%H%M%S)"
REMOTE_ASSETS_DIR="/root/assets"
REMOTE_OUTPUT_DIR="/root/output"
REMOTE_DB_OPTION="/root/DbOption.toml"

# 使用 := 以便 CI 传入空字符串时仍回落到默认
: "${REMOTE_PROJECT_PATH:=/root/e3d_models}"
: "${REMOTE_SURREAL_DATA_PATH:=/root/surreal_data/ams-8020.db}"
: "${REMOTE_SURREALKV_DATA_PATH:=/root/surreal_data/ams-8020.db.kv}"
: "${REMOTE_MESHES_PATH:=/root/assets/meshes}"
: "${REMOTE_SURREAL_SCRIPT_DIR:=/root/resource/surreal}"
: "${DEPLOY_APPLY_DB_PATH_OVERRIDES:=true}"
: "${CONFIG_ONLY:=false}"
: "${RESTART_AFTER_CONFIG_ONLY:=true}"

log() {
  printf '[deploy-web-server] %s\n' "$*"
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

run_remote() {
  local attempt=1
  local max_attempts=5
  local delay=2
  
  while [[ $attempt -le $max_attempts ]]; do
    if sshpass -p "$REMOTE_PASS" ssh "${SSH_OPTS[@]}" "$REMOTE_USER@$REMOTE_HOST" "$@"; then
      return 0
    else
      local exit_code=$?
      if [[ $attempt -lt $max_attempts ]]; then
        log "SSH attempt $attempt/$max_attempts failed (exit code $exit_code), retrying in ${delay}s..."
        sleep "$delay"
        delay=$((delay * 2))
        attempt=$((attempt + 1))
      else
        log "All $max_attempts SSH attempts failed"
        return $exit_code
      fi
    fi
  done
}

run_rsync() {
  local attempt=1
  local max_attempts=5
  local delay=2
  
  while [[ $attempt -le $max_attempts ]]; do
    if sshpass -p "$REMOTE_PASS" rsync -az --delete -e "ssh ${SSH_OPTS[*]}" "$@"; then
      return 0
    else
      local exit_code=$?
      if [[ $attempt -lt $max_attempts ]]; then
        log "rsync attempt $attempt/$max_attempts failed (exit code $exit_code), retrying in ${delay}s..."
        sleep "$delay"
        delay=$((delay * 2))
        attempt=$((attempt + 1))
      else
        log "All $max_attempts rsync attempts failed"
        return $exit_code
      fi
    fi
  done
}

run_rsync_file() {
  local attempt=1
  local max_attempts=5
  local delay=2
  
  while [[ $attempt -le $max_attempts ]]; do
    if sshpass -p "$REMOTE_PASS" rsync -az -e "ssh ${SSH_OPTS[*]}" "$@"; then
      return 0
    else
      local exit_code=$?
      if [[ $attempt -lt $max_attempts ]]; then
        log "rsync attempt $attempt/$max_attempts failed (exit code $exit_code), retrying in ${delay}s..."
        sleep "$delay"
        delay=$((delay * 2))
        attempt=$((attempt + 1))
      else
        log "All $max_attempts rsync attempts failed"
        return $exit_code
      fi
    fi
  done
}

run_scp() {
  local attempt=1
  local max_attempts=5
  local delay=2
  
  while [[ $attempt -le $max_attempts ]]; do
    if sshpass -p "$REMOTE_PASS" scp "${SSH_OPTS[@]}" "$@"; then
      return 0
    else
      local exit_code=$?
      if [[ $attempt -lt $max_attempts ]]; then
        log "scp attempt $attempt/$max_attempts failed (exit code $exit_code), retrying in ${delay}s..."
        sleep "$delay"
        delay=$((delay * 2))
        attempt=$((attempt + 1))
      else
        log "All $max_attempts scp attempts failed"
        return $exit_code
      fi
    fi
  done
}

upload_tree() {
  local local_dir="$1"
  local remote_dir="$2"
  local label="$3"

  log "Uploading ${label}"
  if ! run_rsync "$local_dir/" "$REMOTE_USER@$REMOTE_HOST:$remote_dir/"; then
    log "rsync failed, falling back to SSH streaming for ${label}"
    tar -C "$local_dir" -cf - . | run_remote "set -e; mkdir -p '$remote_dir'; tar -xf - -C '$remote_dir'"
  fi
}

# 按环境变量写入 DbOption 中与部署相关的路径（上传前在本地副本上执行）
apply_dboption_deploy_paths_from_env() {
  local f="$1"
  if [[ "${DEPLOY_APPLY_DB_PATH_OVERRIDES}" != "true" ]]; then
    log "DEPLOY_APPLY_DB_PATH_OVERRIDES=false: DbOption 保持源文件中的路径"
    return 0
  fi
  need_cmd python3
  local out helper
  out="$(mktemp)"
  helper="$SCRIPT_DIR/apply_dboption_deploy_paths.py"
  [[ -f "$helper" ]] || {
    printf 'Missing DbOption path helper: %s\n' "$helper" >&2
    exit 1
  }
  python3 "$helper" "$f" "$out" \
    --project-path "$REMOTE_PROJECT_PATH" \
    --meshes-path "$REMOTE_MESHES_PATH" \
    --surreal-script-dir "$REMOTE_SURREAL_SCRIPT_DIR" \
    --surreal-data-path "$REMOTE_SURREAL_DATA_PATH" \
    --surrealkv-path "$REMOTE_SURREALKV_DATA_PATH"
  mv "$out" "$f"
  log "DbOption 已按环境变量写入路径: project_path meshes_path surreal_script_dir + [web_server]/[surrealdb]/[surrealkv]"
}

resolve_target_dir() {
  if [[ -n "$TARGET" ]]; then
    printf '%s\n' "$PROJECT_DIR/target/$TARGET/$BUILD_PROFILE"
  else
    printf '%s\n' "$PROJECT_DIR/target/$BUILD_PROFILE"
  fi
}

need_cmd sshpass
need_cmd rsync
[[ -n "$REMOTE_PASS" ]] || { printf 'REMOTE_PASS is required\n' >&2; exit 1; }

[[ -f "$DB_OPTION_FILE" ]] || { printf 'Missing DbOption file: %s\n' "$DB_OPTION_FILE" >&2; exit 1; }

if [[ "${CONFIG_ONLY}" == "true" ]]; then
  log "CONFIG_ONLY: uploading DbOption only -> $REMOTE_DB_OPTION (no binary/assets/output)"
  DB_OPTION_UPLOAD="$(mktemp)"
  cp "$DB_OPTION_FILE" "$DB_OPTION_UPLOAD"
  apply_dboption_deploy_paths_from_env "$DB_OPTION_UPLOAD"
  REMOTE_DB_OPTION_TMP="/root/DbOption.toml.tmp"
  if ! run_rsync_file "$DB_OPTION_UPLOAD" "$REMOTE_USER@$REMOTE_HOST:$REMOTE_DB_OPTION_TMP"; then
    log "rsync failed, falling back to SSH streaming for DbOption.toml upload"
    cat "$DB_OPTION_UPLOAD" | run_remote "cat > '$REMOTE_DB_OPTION_TMP'"
  fi
  rm -f "$DB_OPTION_UPLOAD"

  run_remote "set -e; mv /root/DbOption.toml.tmp /root/DbOption.toml"

  if [[ "${RESTART_AFTER_CONFIG_ONLY}" == "true" ]]; then
    log "Restarting $SERVICE_NAME (web_server binary) to apply DbOption"
    run_remote "set -e; \
      systemctl daemon-reload || true; \
      systemctl restart '$SERVICE_NAME'; \
      sleep 2; \
      ok=0; \
      for i in 1 2 3 4 5; do \
        if systemctl is-active '$SERVICE_NAME' >/dev/null 2>&1; then \
          echo 'Service active'; \
          ok=1; \
          break; \
        fi; \
        echo \"Waiting for service to activate (attempt \$i/5)...\"; \
        sleep 2; \
      done; \
      if [[ \"\$ok\" != \"1\" ]]; then \
        systemctl status '$SERVICE_NAME' || true; \
        exit 1; \
      fi; \
      if ! pgrep -x web_server >/dev/null 2>&1 && ! pgrep -f '/root/web_server' >/dev/null 2>&1; then \
        echo 'web_server process not found after restart'; \
        exit 1; \
      fi; \
      echo 'web_server process OK'"
  else
    log "RESTART_AFTER_CONFIG_ONLY=false: 未重启服务；请手动 systemctl restart $SERVICE_NAME"
  fi
  log "CONFIG_ONLY finished. Remote: $REMOTE_DB_OPTION"
  exit 0
fi

if [[ "$UPLOAD_ASSETS" == "auto" ]]; then
  if [[ -d "$ASSETS_DIR" ]]; then
    UPLOAD_ASSETS="true"
  else
    UPLOAD_ASSETS="false"
  fi
fi

if [[ "$UPLOAD_OUTPUT" == "auto" ]]; then
  if [[ -d "$OUTPUT_DIR" ]]; then
    UPLOAD_OUTPUT="true"
  else
    UPLOAD_OUTPUT="false"
  fi
fi

if [[ "$UPLOAD_ASSETS" == "true" ]] && [[ ! -d "$ASSETS_DIR" ]]; then
  printf 'Missing assets directory: %s\n' "$ASSETS_DIR" >&2
  exit 1
fi

if [[ "$UPLOAD_OUTPUT" == "true" ]] && [[ ! -d "$OUTPUT_DIR" ]]; then
  printf 'Missing output directory: %s\n' "$OUTPUT_DIR" >&2
  exit 1
fi

if [[ -n "$LOCAL_BIN_OVERRIDE" ]]; then
  LOCAL_BIN="$LOCAL_BIN_OVERRIDE"
fi

case "$BINARY_SOURCE" in
  local)
    if [[ "$BUILD_BINARY" == "true" ]]; then
      need_cmd cargo
      
      # Determine if we should use zigbuild for cross-compilation
      ACTUAL_USE_ZIGBUILD="false"
      if [[ "$USE_ZIGBUILD" == "auto" ]]; then
        # Auto-detect: use zigbuild on macOS for Linux deployment
        if [[ "$(uname -s)" == "Darwin" ]]; then
          ACTUAL_USE_ZIGBUILD="true"
          log "Auto-detected macOS → enabling cargo-zigbuild for Linux cross-compilation"
        fi
      elif [[ "$USE_ZIGBUILD" == "true" ]]; then
        ACTUAL_USE_ZIGBUILD="true"
      fi
      
      # Configure build command and target
      if [[ "$ACTUAL_USE_ZIGBUILD" == "true" ]]; then
        need_cmd cargo-zigbuild
        need_cmd zig
        BUILD_CMD="cargo zigbuild"
        if [[ -z "$TARGET" ]]; then
          TARGET="$ZIGBUILD_TARGET"
          log "Using zigbuild target: $TARGET"
        fi
      else
        BUILD_CMD="cargo build"
      fi

      TARGET_DIR="$(resolve_target_dir)"
      LOCAL_BIN="$TARGET_DIR/$BINARY_NAME"
      
      log "Building $BINARY_NAME ($BUILD_PROFILE) with $BUILD_CMD..."
      if [[ "$BUILD_PROFILE" == "release" ]]; then
        if [[ -n "$TARGET" ]]; then
          $BUILD_CMD --manifest-path "$PROJECT_DIR/Cargo.toml" --bin "$BINARY_NAME" --release --no-default-features --features "$FEATURES" --target "$TARGET"
        else
          $BUILD_CMD --manifest-path "$PROJECT_DIR/Cargo.toml" --bin "$BINARY_NAME" --release --no-default-features --features "$FEATURES"
        fi
      else
        if [[ -n "$TARGET" ]]; then
          $BUILD_CMD --manifest-path "$PROJECT_DIR/Cargo.toml" --bin "$BINARY_NAME" --no-default-features --features "$FEATURES" --target "$TARGET"
        else
          $BUILD_CMD --manifest-path "$PROJECT_DIR/Cargo.toml" --bin "$BINARY_NAME" --no-default-features --features "$FEATURES"
        fi
      fi
    else
      log "Skipping build because BUILD_BINARY=false"
    fi

    if [[ -z "${LOCAL_BIN:-}" ]]; then
      TARGET_DIR="$(resolve_target_dir)"
      LOCAL_BIN="$TARGET_DIR/$BINARY_NAME"
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

log "Backing up existing binary"
run_remote "set -e; \
  if [ -f '$REMOTE_BIN' ]; then \
    cp -p '$REMOTE_BIN' '$REMOTE_BIN_BACKUP'; \
    echo 'Backed up to $REMOTE_BIN_BACKUP'; \
  fi"

log "Uploading new binary"
# Upload directly to final location with retry logic
if ! run_rsync_file --chmod=755 "$LOCAL_BIN" "$REMOTE_USER@$REMOTE_HOST:$REMOTE_BIN"; then
  log "rsync failed after retries, falling back to SSH streaming for binary upload"
  cat "$LOCAL_BIN" | run_remote "cat > '$REMOTE_BIN' && chmod 755 '$REMOTE_BIN'"
fi

log "Uploading DbOption.toml"
DB_OPTION_UPLOAD="$(mktemp)"
cp "$DB_OPTION_FILE" "$DB_OPTION_UPLOAD"
apply_dboption_deploy_paths_from_env "$DB_OPTION_UPLOAD"
# Upload to temp location then atomically replace
REMOTE_DB_OPTION_TMP="/root/DbOption.toml.tmp"
if ! run_rsync_file "$DB_OPTION_UPLOAD" "$REMOTE_USER@$REMOTE_HOST:$REMOTE_DB_OPTION_TMP"; then
  log "rsync failed after retries, falling back to SSH streaming for DbOption.toml upload"
  cat "$DB_OPTION_UPLOAD" | run_remote "cat > '$REMOTE_DB_OPTION_TMP'"
fi
rm -f "$DB_OPTION_UPLOAD"

run_remote "set -e; mv /root/DbOption.toml.tmp /root/DbOption.toml"

if [[ "$UPLOAD_ASSETS" == "true" ]]; then
  upload_tree "$ASSETS_DIR" "$REMOTE_ASSETS_DIR" "assets/"
else
  log "Skipping assets upload (UPLOAD_ASSETS=$UPLOAD_ASSETS, dir=$ASSETS_DIR)"
fi

if [[ "$UPLOAD_OUTPUT" == "true" ]]; then
  upload_tree "$OUTPUT_DIR" "$REMOTE_OUTPUT_DIR" "output/"
else
  log "Skipping output upload (UPLOAD_OUTPUT=$UPLOAD_OUTPUT, dir=$OUTPUT_DIR)"
fi

log "Restarting service with new binary and configuration"
run_remote "set -e; \
  systemctl daemon-reload || true; \
  systemctl restart '$SERVICE_NAME'; \
  sleep 2; \
  for i in 1 2 3 4 5; do \
    if systemctl is-active '$SERVICE_NAME' >/dev/null 2>&1; then \
      echo 'Service active'; \
      exit 0; \
    fi; \
    echo \"Waiting for service to activate (attempt \$i/5)...\"; \
    sleep 2; \
  done; \
  systemctl status '$SERVICE_NAME' || true; \
  exit 1"

log "Deployment finished"
log "Remote binary: $REMOTE_BIN"
log "Remote assets: $REMOTE_ASSETS_DIR"
log "Remote output: $REMOTE_OUTPUT_DIR"
