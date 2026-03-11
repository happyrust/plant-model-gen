#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="/Volumes/DPC/work/plant-code/plant-model-gen"
FRONTEND_ROOT="/Volumes/DPC/work/plant-code/plant3d-web"

if [ -f "$REPO_ROOT/Cargo.toml" ]; then
  echo "backend repo present"
fi

if [ -f "$FRONTEND_ROOT/package.json" ]; then
  echo "frontend repo present"
fi

if [ ! -d "$FRONTEND_ROOT/node_modules" ]; then
  npm --prefix "$FRONTEND_ROOT" install
fi
