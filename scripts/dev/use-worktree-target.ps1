# 让当前 worktree 使用独立的 cargo 构建目录。
#
# 背景：本机 CARGO_TARGET_DIR 是全局环境变量（默认 D:\Rust\target），所有 git worktree
# 共用同一个 target 目录 —— 两个 worktree 同时跑 cargo 会互相阻塞在 cargo lock 上，
# 来回切换还会反复重编 aios_database 本体。多 session 并行推进 spec 030 的三条线时，
# 必须让每个 worktree 有自己的 target 目录。
#
# 注意：环境变量 CARGO_TARGET_DIR 的优先级高于 .cargo/config.toml 的 build.target-dir，
# 所以只能靠改环境变量，改配置文件无效。
#
# 用法（必须 dot-source，否则改不到当前会话）：
#   . scripts/dev/use-worktree-target.ps1
#
# 基址优先级：$env:CARGO_TARGET_ROOT > 当前 CARGO_TARGET_DIR 的父目录 > 仓库的同级目录。
# 重复执行是幂等的。

$root = (git rev-parse --show-toplevel 2>$null)
if (-not $root) {
    Write-Error "不在 git 仓库中，无法推断 worktree 名称"
    return
}
$root = $root.Trim()
$leaf = Split-Path $root -Leaf

$base = if ($env:CARGO_TARGET_ROOT) {
    $env:CARGO_TARGET_ROOT
} elseif ($env:CARGO_TARGET_DIR) {
    Split-Path $env:CARGO_TARGET_DIR -Parent
} else {
    Split-Path $root -Parent
}

$env:CARGO_TARGET_DIR = Join-Path $base "target-$leaf"
Write-Host "CARGO_TARGET_DIR = $env:CARGO_TARGET_DIR"
