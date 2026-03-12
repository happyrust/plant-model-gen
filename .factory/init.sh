#!/bin/bash
set -e

echo "🚀 Initializing room verify-json mission environment..."

if [ ! -f "Cargo.toml" ]; then
    echo "❌ Error: Must run from plant-model-gen repository root"
    exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "❌ Error: Rust/Cargo not found"
    exit 1
fi

echo "✅ Rust toolchain found"

if [ ! -f "tests/fixtures/room_compute_validation.json" ]; then
    echo "❌ Error: tests/fixtures/room_compute_validation.json not found"
    exit 1
fi

echo "✅ Validation fixture found"

if lsof -i :8020 >/dev/null 2>&1 || lsof -i :8009 >/dev/null 2>&1; then
    echo "✅ SurrealDB/listening DB port detected"
else
    echo "⚠️  Warning: SurrealDB port was not auto-detected; mission can still proceed if DB is reachable through configured settings"
fi

echo "ℹ️  Acceptance workflow for this mission:"
echo "   1. cargo run --bin aios-database -- room compute ..."
echo "   2. cargo run --bin aios-database -- room verify-json --input tests/fixtures/room_compute_validation.json"

echo "✅ Mission environment initialization complete"
