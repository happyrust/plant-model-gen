#!/usr/bin/env bash
# =============================================================================
# build.sh — 本地编译快速入口
#
# 用法：
#   ./shells/build.sh [场景]
#
# 场景（不传默认 dev）：
#   dev       日常迭代，改自己代码用（incremental，最快反馈）
#   check     只做类型检查，不生成二进制（最快，用于确认代码能编译）
#   dep       改了 rs-core / pdms-io-fork 之后用（sccache，跳过已缓存的 crate）
#   release   构建 release 包（用于测试性能/部署）
#   web       构建 web_server（dev 模式）
#   web-rel   构建 web_server（release 模式）
#
# 前置依赖：
#   dep/web-dep 场景需要先安装 sccache：brew install sccache
# =============================================================================

set -e
cd "$(dirname "$0")/.."

SCENARIO="${1:-dev}"

# ── 公共 feature 组合 ────────────────────────────────────────────────────────
FEATURES_DEFAULT="ws,gen_model,manifold,project_hd,surreal-save"
FEATURES_WEB="ws,sqlite-index,surreal-save,web_server"

# ── 颜色输出 ─────────────────────────────────────────────────────────────────
info()  { echo "[build] $*"; }
ok()    { echo "[build] OK: $*"; }
err()   { echo "[build] ERR: $*" >&2; exit 1; }

check_sccache() {
    if ! command -v sccache &>/dev/null; then
        err "sccache 未安装。请先执行: brew install sccache"
    fi
}

# ── 场景分发 ─────────────────────────────────────────────────────────────────
case "$SCENARIO" in

  # --------------------------------------------------------------------------
  # dev: 日常改代码，增量编译，最快
  # --------------------------------------------------------------------------
  dev)
    info "场景: dev（增量编译，改应用代码用）"
    cargo build --no-default-features --features "$FEATURES_DEFAULT"
    ok "dev build 完成 → target/debug/aios-database"
    ;;

  # --------------------------------------------------------------------------
  # check: 只做类型检查，不编译二进制，比 build 快 2-3x
  # --------------------------------------------------------------------------
  check)
    info "场景: check（仅类型检查，不生成二进制）"
    cargo check --no-default-features --features "$FEATURES_DEFAULT"
    ok "check 完成"
    ;;

  # --------------------------------------------------------------------------
  # dep: 改了 path 依赖（rs-core / pdms-io-fork）后用
  #   - 关闭 incremental（与 sccache 互斥）
  #   - sccache 命中未变动的 crate，跳过重编
  # --------------------------------------------------------------------------
  dep)
    info "场景: dep（path 依赖变更，启用 sccache）"
    check_sccache
    CARGO_INCREMENTAL=0 RUSTC_WRAPPER=sccache \
      cargo build --no-default-features --features "$FEATURES_DEFAULT"
    info "sccache 统计:"
    sccache --show-stats | grep -E "Cache (hits|misses|size)"
    ok "dep build 完成 → target/debug/aios-database"
    ;;

  # --------------------------------------------------------------------------
  # release: release 构建，用于性能测试或部署前验证
  # --------------------------------------------------------------------------
  release)
    info "场景: release（优化构建，耗时较长）"
    cargo build --release --no-default-features --features "$FEATURES_DEFAULT"
    ok "release build 完成 → target/release/aios-database"
    ;;

  # --------------------------------------------------------------------------
  # web: web_server dev 构建
  # --------------------------------------------------------------------------
  web)
    info "场景: web dev（增量编译）"
    cargo build --bin web_server \
      --no-default-features --features "$FEATURES_WEB"
    ok "web dev build 完成 → target/debug/web_server"
    ;;

  # --------------------------------------------------------------------------
  # web-rel: web_server release 构建
  # --------------------------------------------------------------------------
  web-rel)
    info "场景: web release"
    cargo build --release --bin web_server \
      --no-default-features --features "$FEATURES_WEB"
    ok "web release build 完成 → target/release/web_server"
    ;;

  *)
    echo "未知场景: $SCENARIO"
    echo ""
    echo "可用场景: dev | check | dep | release | web | web-rel"
    exit 1
    ;;
esac
