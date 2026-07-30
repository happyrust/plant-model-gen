#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
WORKSPACE_PARENT="$(cd "$PROJECT_DIR/.." && pwd)"

prepare_repo() {
  local repo_url="$1"
  local branch="$2"
  local target_dir="$3"

  if [[ -d "$target_dir/.git" ]]; then
    printf '[prepare-ci-patch-deps] update %s (%s)\n' "$target_dir" "$branch"
    git -C "$target_dir" fetch --depth 1 origin "$branch"
    git -C "$target_dir" checkout -B "$branch" "FETCH_HEAD"
    return 0
  fi

  rm -rf "$target_dir"
  printf '[prepare-ci-patch-deps] clone %s (%s) -> %s\n' "$repo_url" "$branch" "$target_dir"
  git clone --depth 1 --branch "$branch" "$repo_url" "$target_dir"
}

prepare_repo "https://github.com/happyrust/rs-core.git" "dev-3.1" \
  "$WORKSPACE_PARENT/rs-core"
prepare_repo "https://github.com/happyrust/pdms-io.git" "dev-3.1" \
  "$WORKSPACE_PARENT/pdms-io-fork"
prepare_repo "https://github.com/happyrust/dpc-sync.git" "master" \
  "$WORKSPACE_PARENT/dpc-sync"
