#!/bin/bash
set -e

echo "🚀 Initializing room-compute 3x mission environment..."

if [ ! -f "Cargo.toml" ]; then
    echo "❌ Error: must run from the plant-model-gen repository root"
    exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "❌ Error: Rust/Cargo not found"
    exit 1
fi

echo "✅ Rust toolchain found"

if [ ! -f ".factory/services.yaml" ]; then
    echo "❌ Error: .factory/services.yaml not found"
    exit 1
fi

if [ ! -f "verification/room/compute/room_compute_validation.json" ]; then
    echo "⚠️  Notice: verification/room/compute/room_compute_validation.json not found; skip json-specific checks if unavailable"
fi

echo "✅ Mission service manifest found"

if [ -f "output/spatial_index.sqlite" ]; then
    echo "✅ Existing spatial_index.sqlite detected"
else
    echo "⚠️  output/spatial_index.sqlite not found yet; the mission may need an explicit rebuild before full validation"
fi

powershell -NoProfile -Command "Get-CimInstance Win32_OperatingSystem | Select-Object TotalVisibleMemorySize,FreePhysicalMemory | Format-List" || true

echo "ℹ️  Validation path dry-run guidance:"
echo "   - cargo run --bin aios-database -- room --help"
echo "   - cargo run --release --bin aios-database -- room compute"
echo "   - cargo run --release --bin aios-database -- room compute-panel --panel-refno <refno>"
echo "   - cargo check --release --bin aios-database"
echo "ℹ️  Acceptance workflow for this mission:"
echo "   1. cargo run --bin aios-database -- room compute ..."
echo "   2. cargo run --bin aios-database -- room verify-json --input verification/room/compute/room_compute_validation.json"

echo "✅ Mission environment initialization complete"
