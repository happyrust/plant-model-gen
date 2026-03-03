# SurrealKV 版本管理测试数据库启动脚本
# 使用 SurrealKV 引擎 + MVCC versioned=true
# 端口: 8030 (避免与 8020/8010 冲突)

$port = 8030

# 清理占用端口的进程
$processes = Get-NetTCPConnection -LocalPort $port -ErrorAction SilentlyContinue
if ($processes) {
    Write-Host "清理端口 $port ..."
    foreach ($p in $processes) {
        Stop-Process -Id $p.OwningProcess -Force -ErrorAction SilentlyContinue
    }
    Start-Sleep -Seconds 1
}

$dbPath = Join-Path $PSScriptRoot "version-test.skv"

Write-Host "=========================================="
Write-Host "  SurrealKV 版本管理测试数据库"
Write-Host "  端口:    $port"
Write-Host "  引擎:    SurrealKV (MVCC versioned)"
Write-Host "  数据目录: $dbPath"
Write-Host "  保留期:  30天"
Write-Host "=========================================="

$env:SURREAL_PLANNER_STRATEGY = "compute-only"

surreal start `
    --user root `
    --pass root `
    --bind "127.0.0.1:$port" `
    --log info `
    "surrealkv://${dbPath}?versioned=true&retention=30d"
