# Local Site A · web_server 启动器（由 plant-collab-monitor/scripts/local-remote-collab-setup.ps1 生成）
# 中继模式（sync_relay_mode = true）：不需要 SurrealDB。长驻进程：在你自己的终端里运行，Ctrl+C 结束。
# cwd 必须是 plant-model-gen（配置里全是相对路径；两站共用 cwd 下的 assets/archives 作 CBA 目录）。
$ErrorActionPreference = 'Stop'
Set-Location 'D:\work\plant-code\plant-model-gen'
$env:ADMIN_USER = 'admin'
$env:ADMIN_PASS = 'admin'
$env:WEB_SERVER_PORT = '4100'
$exe = 'D:\Rust\target\debug\web_server.exe'
Write-Host "[site-a] :4100 · location local-a · config runtime/local-collab/site-a/DbOption.toml · relay（无 SurrealDB）"
if (Test-Path $exe) {
  & $exe --config 'runtime/local-collab/site-a/DbOption'
} else {
  Write-Host "[site-a] 未找到 $exe，改用 cargo run（首次编译很慢）"
  cargo run --bin web_server --features web_server,relay-sync -- --config 'runtime/local-collab/site-a/DbOption'
}