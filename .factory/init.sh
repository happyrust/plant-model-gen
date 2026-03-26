#!/bin/bash
set -e

echo "Initializing plant-model-gen console mission environment..."

if [ ! -f "Cargo.toml" ]; then
    echo "❌ Error: must run from the plant-model-gen repository root"
    exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "❌ Error: Rust/Cargo not found"
    exit 1
fi

echo "Rust toolchain found"

if ! command -v node >/dev/null 2>&1; then
    echo "❌ Error: Node.js not found"
    exit 1
fi

echo "Node.js found"

if [ ! -f "web_console/package.json" ]; then
    echo "❌ Error: web_console/package.json not found (expected Vue console app)"
    exit 1
fi

if [ ! -f ".factory/services.yaml" ]; then
    echo "❌ Error: .factory/services.yaml not found"
    exit 1
fi

echo "Mission service manifest found"

if [ ! -d "web_console/node_modules" ]; then
    echo "Note: web_console/node_modules not found yet. Run: npm --prefix web_console install"
fi

echo "Validation guidance:"
echo "  - No tests / no compiling tests (do not run cargo test or cargo check --tests)"
echo "  - Rust: cargo fmt; cargo check --features web_server --bin web_server"
echo "  - UI: start web_server on 3100 and validate /console with agent-browser"

echo "Initialization complete"
