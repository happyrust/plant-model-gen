#!/usr/bin/env bash
# ─── production 构建脚本 ───────────────────────────────────────────────────────
# 使用 [profile.production]：opt-level=3, codegen-units=1, thin-LTO
# 编译时间较长，仅在需要上线发布时使用。日常开发请用 `cargo build --release`。
#
# 用法：
#   ./build_production.sh                    # 编译默认 bin
#   ./build_production.sh --bin web_server   # 编译指定 bin
#   ./build_production.sh --features parquet-export
# ──────────────────────────────────────────────────────────────────────────────
set -euo pipefail

echo ">>> [production build] opt-level=3 / codegen-units=1 / thin-LTO"
echo ">>> 编译时间较长，请耐心等待..."
echo ""

# 传递所有额外参数（--bin / --features 等）给 cargo
cargo build --profile production "$@"

echo ""
echo ">>> 完成。产物位于 target/production/"
