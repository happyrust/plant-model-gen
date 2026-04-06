#!/bin/bash
set -e

echo "Initializing plant-model-gen RVM delivery prepack mission..."

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

echo "Primary implementation surfaces:"
echo "- src/main.rs"
echo "- src/cli_args.rs"
echo "- src/cli_modes.rs"
echo "- src/fast_model/export_model/export_dbnum_instances_v3.rs"
echo "- src/fast_model/export_model/export_prepack_lod.rs"
echo "- src/fast_model/export_model/export_instanced_bundle.rs"
echo "- src/fast_model/export_model/export_glb.rs"
echo "- src/web_server/mod.rs"
echo "- src/web_server/output_instances_files.rs"
echo "- src/web_server/instance_export.rs"
echo "- src/web_server/stream_generate.rs"
echo "- src/bin/web_server.rs"

echo "Mission runtime contract:"
echo "- Reuse SurrealDB on 127.0.0.1:8021"
echo "- Run validation web_server on 127.0.0.1:3200"
echo "- Do not use Rust tests as the primary validation path"
echo "- Validate with CLI/json evidence and web_server POST/static checks"
