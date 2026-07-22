# specs/022 Version Commit 契约验证 — versioned RocksDB 实例
# 端口: 8034（避开 8010/8020/8030/8031/8032/8033）
# 引擎: fork surreal (D:\work\plant-code\surrealdb, dev-3.1) RocksDB versioned=true
# 用法：先跑本脚本起实例，再用 surreal sql 灌入 verify_version_commit_state.surql

$port = 8034
$surrealBin = 'D:\work\plant-code\surrealdb\target\release\surreal.exe'

$processes = Get-NetTCPConnection -LocalPort $port -ErrorAction SilentlyContinue
if ($processes) {
    Write-Host "清理端口 $port ..."
    foreach ($p in $processes) {
        Stop-Process -Id $p.OwningProcess -Force -ErrorAction SilentlyContinue
    }
    Start-Sleep -Seconds 1
}

$dbPath = Join-Path $PSScriptRoot "commit-state-verify.rdb"

Write-Host "=========================================="
Write-Host "  Version Commit 契约验证 versioned 实例"
Write-Host "  端口:    $port"
Write-Host "  引擎:    RocksDB (versioned=true, retention=0 无限保留)"
Write-Host "  数据目录: $dbPath"
Write-Host "=========================================="

$env:SURREAL_PLANNER_STRATEGY = "compute-only"

& $surrealBin start `
    --user root `
    --pass root `
    --bind "127.0.0.1:$port" `
    --log info `
    "rocksdb://${dbPath}?versioned=true&retention=0"
