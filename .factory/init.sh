#!/bin/bash
set -e

echo "🚀 Initializing mission environment..."

# Check if we're in the correct directory
if [ ! -f "Cargo.toml" ]; then
    echo "❌ Error: Must run from plant-model-gen repository root"
    exit 1
fi

# Backend: Check Rust toolchain
if ! command -v cargo &> /dev/null; then
    echo "❌ Error: Rust/Cargo not found. Please install Rust."
    exit 1
fi

echo "✅ Rust toolchain found"

# Frontend: Check Node.js
if ! command -v node &> /dev/null; then
    echo "❌ Error: Node.js not found. Please install Node.js."
    exit 1
fi

echo "✅ Node.js found"

# Check if SurrealDB is running
if lsof -i :8020 &> /dev/null; then
    echo "✅ SurrealDB running on port 8020"
else
    echo "⚠️  Warning: SurrealDB not detected on port 8020"
fi

# Check if backend is running
if lsof -i :3100 &> /dev/null; then
    echo "✅ Backend running on port 3100"
else
    echo "ℹ️  Backend not running (will be started by workers if needed)"
fi

# Check if frontend is running
if lsof -i :3101 &> /dev/null; then
    echo "✅ Frontend running on port 3101"
else
    echo "ℹ️  Frontend not running (will be started by workers if needed)"
fi

echo "✅ Environment initialization complete"
