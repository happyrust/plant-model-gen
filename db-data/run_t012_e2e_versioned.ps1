# specs/022 T012 真机增量 E2E — versioned RocksDB 实例
# 端口: 8033（避开 8020/8010/8030/8031/8032）
# 引擎: fork surreal (D:\work\plant-code\surrealdb, dev-3.1) RocksDB versioned=true

$port = 8033
$surrealBin = 'D:\work\plant-code\surrealdb\target\release\surreal.exe'

$processes = Get-NetTCPConnection -LocalPort $port -ErrorAction SilentlyContinue
if ($processes) {
    Write-Host "清理端口 $port ..."
    foreach ($p in $processes) {
        Stop-Process -Id $p.OwningProcess -Force -ErrorAction SilentlyContinue
    }
    Start-Sleep -Seconds 1
}

$dbPath = Join-Path $PSScriptRoot "t012-e2e-versioned.rdb"

Write-Host "=========================================="
Write-Host "  T012 真机增量 E2E versioned 实例"
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
