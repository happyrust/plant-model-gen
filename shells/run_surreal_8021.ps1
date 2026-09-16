$port = 8021
$processes = Get-NetTCPConnection -LocalPort $port -ErrorAction SilentlyContinue
if ($processes) {
    Write-Host "Cleaning up port $port..."
    foreach ($p in $processes) {
        Stop-Process -Id $p.OwningProcess -Force -ErrorAction SilentlyContinue
    }
}

surreal start --user root --pass root --bind 0.0.0.0:$port rocksdb://ams-$port.db
