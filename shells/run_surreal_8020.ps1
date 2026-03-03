$port = 8020
$processes = Get-NetTCPConnection -LocalPort $port -ErrorAction SilentlyContinue
if ($processes) {
    Write-Host "Cleaning up port $port..."
    foreach ($p in $processes) {
        Stop-Process -Id $p.OwningProcess -Force -ErrorAction SilentlyContinue
    }
}

$cpu = [Environment]::ProcessorCount
$tmpDir = Join-Path $env:TEMP "surreal_tmp_$port"
New-Item -ItemType Directory -Force -Path $tmpDir | Out-Null

$env:SURREAL_SYNC_DATA = "false"
$env:SURREAL_ROCKSDB_THREAD_COUNT = "{0}" -f ([Math]::Min($cpu, 16))
$env:SURREAL_ROCKSDB_JOBS_COUNT = "{0}" -f ([Math]::Min($cpu * 2, 32))
$env:SURREAL_ROCKSDB_MAX_CONCURRENT_SUBCOMPACTIONS = "{0}" -f ($(if ($cpu -ge 16) { 8 } else { 4 }))
$env:SURREAL_ROCKSDB_MAX_OPEN_FILES = "4096"

$env:SURREAL_ROCKSDB_BLOCK_CACHE_SIZE = "16GB"
$env:SURREAL_ROCKSDB_WRITE_BUFFER_SIZE = "256MB"
$env:SURREAL_ROCKSDB_MAX_WRITE_BUFFER_NUMBER = "8"
$env:SURREAL_ROCKSDB_MIN_WRITE_BUFFER_NUMBER_TO_MERGE = "2"

$env:SURREAL_ROCKSDB_TARGET_FILE_SIZE_BASE = "256MB"
$env:SURREAL_ROCKSDB_TARGET_FILE_SIZE_MULTIPLIER = "2"
$env:SURREAL_ROCKSDB_FILE_COMPACTION_TRIGGER = "4"
$env:SURREAL_ROCKSDB_STORAGE_LOG_LEVEL = "warn"
$env:SURREAL_ROCKSDB_BLOB_COMPRESSION_TYPE = "lz4"
$env:SURREAL_PLANNER_STRATEGY = "compute-only"

surreal start --user root --pass root --bind 0.0.0.0:$port --log warn --temporary-directory $tmpDir rocksdb://ams-$port.db
