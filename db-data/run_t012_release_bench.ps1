# Release bench: fresh versioned RocksDB on 8034
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

$dbPath = Join-Path $PSScriptRoot "t012-release-bench.rdb"
if (Test-Path $dbPath) {
    Write-Host "清理旧数据目录 $dbPath ..."
    Remove-Item -Recurse -Force $dbPath
}

Write-Host "=========================================="
Write-Host "  Release bench versioned instance"
Write-Host "  端口:    $port"
Write-Host "  数据目录: $dbPath"
Write-Host "=========================================="

$env:SURREAL_PLANNER_STRATEGY = "compute-only"

& $surrealBin start `
    --user root `
    --pass root `
    --bind "127.0.0.1:$port" `
    --log warn `
    "rocksdb://${dbPath}?versioned=true&retention=0"
