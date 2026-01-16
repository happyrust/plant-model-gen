$port = 8020
$processes = Get-NetTCPConnection -LocalPort $port -ErrorAction SilentlyContinue
if ($processes) {
    Write-Host "Cleaning up port $port..."
    foreach ($p in $processes) {
        Stop-Process -Id $p.OwningProcess -Force -ErrorAction SilentlyContinue
    }
}

surreal start --user root --pass root --bind 0.0.0.0:$port surrealkv://ams-$port.kv
