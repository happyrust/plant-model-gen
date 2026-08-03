# ams7997 历史回溯验证专用实例
# 独立的 versioned RocksDB 数据目录，不触碰 ams.db / acp7320-perf.db
# 端口 8033（8020/8030/8032 已被占用或另作他用）

$port = 8033
$dbPath = 'D:/backup-dbs/ams7997-versioned.db'

$processes = Get-NetTCPConnection -LocalPort $port -ErrorAction SilentlyContinue
if ($processes) {
    Write-Host "清理端口 $port ..."
    foreach ($p in $processes) {
        Stop-Process -Id $p.OwningProcess -Force -ErrorAction SilentlyContinue
    }
    Start-Sleep -Seconds 1
}

Write-Host "=========================================="
Write-Host "  ams7997 versioned 验证实例"
Write-Host "  端口:     $port"
Write-Host "  引擎:     RocksDB (MVCC versioned)"
Write-Host "  数据目录: $dbPath"
Write-Host "  保留期:   0 (不裁剪历史)"
Write-Host "=========================================="

$env:SURREAL_PLANNER_STRATEGY = "compute-only"

surreal start `
    --user root `
    --pass root `
    --bind "127.0.0.1:$port" `
    --log info `
    "rocksdb://${dbPath}?versioned=true&retention=0"
