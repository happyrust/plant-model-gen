# Local Site B · plant-web-server 启动器（由 plant-collab-monitor/scripts/local-remote-collab-setup.ps1 生成）
# 中继模式（sync_relay_mode = true）：不需要 SurrealDB。长驻进程：在你自己的终端里运行，Ctrl+C 结束。
# --repo-root 让进程 chdir 到 plant-model-gen：配置里全是相对路径，两站也共用它下面的
# assets/archives 作 CBA 目录（对端按 file_server_host 到 /assets/archives 下载）。
# PLANT_WEB_RUNTIME_DIR 必须每站一个：不给的话两站共用 plant-web-server 源码树下那一份
# envs.json，谁后写谁赢，而且两边都不报错。
$ErrorActionPreference = 'Stop'
Set-Location 'D:\work\plant-code\plant-model-gen'
$env:ADMIN_USER = 'admin'
$env:ADMIN_PASS = 'admin'
$env:WEB_SERVER_PORT = '4101'
$env:PLANT_WEB_RUNTIME_DIR = 'D:\work\plant-code\plant-model-gen\runtime\local-collab\site-b\pws-runtime'
$exe = 'D:\Rust\target\debug\plant-web-server.exe'
Write-Host "[site-b] :4101 · location local-b · config runtime/local-collab/site-b/DbOption.toml · relay（plant-web-server，无 SurrealDB）"
if (Test-Path $exe) {
  & $exe --repo-root 'D:\work\plant-code\plant-model-gen' --config 'runtime/local-collab/site-b/DbOption'
} else {
  Write-Host "[site-b] 未找到 $exe，改用 cargo run（首次编译很慢）"
  Set-Location 'D:\work\plant-code\plant-web-server'
  cargo run --bin plant-web-server -- --repo-root 'D:\work\plant-code\plant-model-gen' --config 'runtime/local-collab/site-b/DbOption'
}