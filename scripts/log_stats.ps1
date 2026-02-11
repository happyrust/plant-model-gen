param(
  [Parameter(Mandatory=$true, Position=0)]
  [Alias("LogPath")]
  [string]$Path
)

if (-not (Test-Path $Path)) {
  Write-Error "log file not found: $Path"
  exit 1
}

function Count-Match([string]$pattern) {
  return (Select-String -Path $Path -Pattern $pattern -ErrorAction SilentlyContinue | Measure-Object).Count
}

$hit  = Count-Match "resolve_desi_comp cache hit \(foyer/rkyv\)"
$miss = Count-Match "resolve_desi_comp cache miss \(foyer/rkyv\)"
$gen  = Count-Match "Calling gen_cata_single_geoms"
$prefetchStart  = Count-Match "\[cata_resolve_cache_pipeline\] prefetch start:"
$prefetchFinish = Count-Match "\[cata_resolve_cache_pipeline\] prefetch finish:"

Write-Host ("log: {0}" -f $Path)
Write-Host ("  cata_resolve_cache_hit   : {0}" -f $hit)
Write-Host ("  cata_resolve_cache_miss  : {0}" -f $miss)
Write-Host ("  gen_cata_single_geoms    : {0}" -f $gen)
Write-Host ("  prefetch_start_lines     : {0}" -f $prefetchStart)
Write-Host ("  prefetch_finish_lines    : {0}" -f $prefetchFinish)

$finishLine = (Select-String -Path $Path -Pattern "\[cata_resolve_cache_pipeline\] prefetch finish:" -ErrorAction SilentlyContinue | Select-Object -Last 1).Line
if ($finishLine) {
  Write-Host ("  prefetch_finish_last     : {0}" -f $finishLine)
}
