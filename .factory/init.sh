#!/bin/bash
set -e

echo "Initializing plant-model-gen workflow/sync backend investigation..."

if [ ! -f "Cargo.toml" ]; then
  echo "Error: must run from the plant-model-gen repository root"
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "Error: Rust/Cargo not found"
  exit 1
fi

echo "Rust toolchain found"

echo "Primary evidence surfaces:"
echo "- src/web_api/platform_api/workflow_sync.rs"
echo "- src/web_api/review_api.rs"
echo "- review/model-center integration helpers referenced by workflow sync"
echo "- shells/platform_api_json/workflow_sync_query.json"
echo "- shells/platform_api_json/workflow_sync_active.json"
echo "Reminder: this is an analysis-only mission. Do not change product code unless the orchestrator explicitly expands scope."
