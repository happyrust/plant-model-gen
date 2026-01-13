#!/bin/bash
# 启动 web_server 服务
# 使用方法: ./run_web_server.sh

cd "$(dirname "$0")"

echo "🚀 启动 web_server..."
cargo run --bin web_server --features web_server
