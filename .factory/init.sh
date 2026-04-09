#!/bin/bash
set -e

echo "Initializing plant-model-gen /admin site-management mission..."

if [ ! -f "Cargo.toml" ]; then
  echo "Error: must run from the plant-model-gen repository root"
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "Error: Rust/Cargo not found"
  exit 1
fi

if ! command -v curl >/dev/null 2>&1; then
  echo "Error: curl not found"
  exit 1
fi

if ! command -v agent-browser >/dev/null 2>&1; then
  echo "Warning: agent-browser not found in PATH; browser validation will be blocked until it is available"
fi

echo "Primary implementation surfaces:"
echo "- src/web_server/static/admin/index.html"
echo "- src/web_server/static/admin/admin.css"
echo "- src/web_server/static/admin/admin.js"
echo "- src/web_server/admin_handlers.rs"
echo "- src/web_server/managed_project_sites.rs"
echo "- src/web_server/models.rs"
echo "- src/web_server/mod.rs"

echo "Mission runtime contract:"
echo "- Reuse shared SurrealDB on 127.0.0.1:8021"
echo "- Run isolated mission web_server on 127.0.0.1:3333"
echo "- Keep unrelated 127.0.0.1:3100 and 127.0.0.1:3101 instances untouched"
echo "- Do not use Rust tests; validate via running web_server + curl/POST + agent-browser"
