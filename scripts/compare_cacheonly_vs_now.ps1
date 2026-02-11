param(
  [Parameter(Mandatory=$false)]
  [string]$Config = "db_options/DbOption",

  [Parameter(Mandatory=$false)]
  [string]$DebugRefno = "24381/145018",

  [Parameter(Mandatory=$false)]
  [string]$ObjPath = "output/AvevaMarineSample/Copy-of-RCS0014-1R43012新.obj"
)

$ErrorActionPreference = "Stop"

function Get-LatestLog([string]$prefix) {
  $files = Get-ChildItem -Path "logs" -Filter "$prefix*.log" -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending
  if (-not $files -or $files.Count -eq 0) {
    return $null
  }
  return $files[0].FullName
}

function Parse-TimeFromLog([string]$logPath) {
  $content = Get-Content $logPath -Raw

  $mTotal = [regex]::Match($content, "gen_all_geos_data 完成，总耗时\s+(\d+)\s+ms")
  $mMesh  = [regex]::Match($content, "完成 mesh 生成，用时\s+(\d+)\s+ms")
  $mBool  = [regex]::Match($content, "完成布尔运算，用时\s+(\d+)\s+ms")

  $totalMs = if ($mTotal.Success) { [int]$mTotal.Groups[1].Value } else { $null }
  $meshMs  = if ($mMesh.Success)  { [int]$mMesh.Groups[1].Value } else { $null }
  $boolMs  = if ($mBool.Success)  { [int]$mBool.Groups[1].Value } else { $null }

  # 从 cache_flush 行提取结构统计（更贴近“模型数据是否一致”）
  $mFlush = [regex]::Match(
    $content,
    "\\[cache_flush\\].*inst_info=\\s*(\\d+)\\s+inst_geos=\\s*(\\d+)\\s+inst_tubi=\\s*(\\d+)\\s+neg=\\s*(\\d+)\\s+ngmr=\\s*(\\d+)\\s+bool=\\s*(\\d+)"
  )
  $flush = $null
  if ($mFlush.Success) {
    $flush = @{
      inst_info = [int]$mFlush.Groups[1].Value
      inst_geos = [int]$mFlush.Groups[2].Value
      inst_tubi = [int]$mFlush.Groups[3].Value
      neg = [int]$mFlush.Groups[4].Value
      ngmr = [int]$mFlush.Groups[5].Value
      bool = [int]$mFlush.Groups[6].Value
    }
  }

  return @{
    total_ms = $totalMs
    mesh_ms = $meshMs
    bool_ms = $boolMs
    cache_flush = $flush
  }
}

function Run-One([string]$modeLabel, [hashtable]$envs) {
  Write-Host ""
  Write-Host ("==== RUN: {0} ====" -f $modeLabel)

  $exe = "target/debug/aios-database.exe"
  if (-not (Test-Path $exe)) {
    Write-Host "[info] building aios-database (dev profile)..."
    cargo build --bin aios-database | Out-Null
  }

  # set env vars for this run
  $old = @{}
  foreach ($k in $envs.Keys) {
    $old[$k] = [System.Environment]::GetEnvironmentVariable($k, "Process")
    [System.Environment]::SetEnvironmentVariable($k, $envs[$k], "Process")
  }

  # clear conflicting env vars when explicitly asked
  if ($envs.ContainsKey("_clear")) {
    foreach ($k in ($envs["_clear"] -split ",")) {
      $key = $k.Trim()
      if ($key) {
        $old[$key] = [System.Environment]::GetEnvironmentVariable($key, "Process")
        [System.Environment]::SetEnvironmentVariable($key, $null, "Process")
      }
    }
  }

  $safeRef = $DebugRefno.Replace("/", "_")
  $beforeLog = Get-LatestLog $safeRef

  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  & $exe --config $Config --debug-model $DebugRefno --regen-model --export-obj --sync-to-db | Out-Null
  $sw.Stop()

  $afterLog = Get-LatestLog $safeRef
  if (-not $afterLog) {
    throw "no log generated (prefix=$safeRef)"
  }
  if ($beforeLog -and ($beforeLog -eq $afterLog)) {
    Write-Host ("[warn] latest log did not change: {0}" -f $afterLog)
  }

  if (-not (Test-Path $ObjPath)) {
    throw "obj file not found after run: $ObjPath"
  }

  $outDir = "output/_compare"
  if (-not (Test-Path $outDir)) { New-Item -ItemType Directory -Path $outDir | Out-Null }
  $objCopy = Join-Path $outDir ("{0}_{1}.obj" -f $safeRef, $modeLabel)
  Copy-Item -Force $ObjPath $objCopy

  $logCopy = Join-Path $outDir ("{0}_{1}.log" -f $safeRef, $modeLabel)
  Copy-Item -Force $afterLog $logCopy

  $times = Parse-TimeFromLog $logCopy

  Write-Host ("log: {0}" -f $logCopy)
  Write-Host ("obj: {0}" -f $objCopy)
  Write-Host ("wall_ms: {0}" -f $sw.ElapsedMilliseconds)
  if ($times.total_ms -ne $null) { Write-Host ("gen_total_ms: {0}" -f $times.total_ms) }
  if ($times.mesh_ms -ne $null)  { Write-Host ("mesh_ms     : {0}" -f $times.mesh_ms) }
  if ($times.bool_ms -ne $null)  { Write-Host ("bool_ms     : {0}" -f $times.bool_ms) }
  if ($times.cache_flush) {
    Write-Host ("cache_flush : inst_info={0} inst_geos={1} inst_tubi={2} neg={3} ngmr={4} bool={5}" -f `
      $times.cache_flush.inst_info, $times.cache_flush.inst_geos, $times.cache_flush.inst_tubi, `
      $times.cache_flush.neg, $times.cache_flush.ngmr, $times.cache_flush.bool)
  }

  # signature
  $sigJson = Join-Path $outDir ("{0}_{1}.sig.json" -f $safeRef, $modeLabel)
  & powershell -ExecutionPolicy Bypass -File scripts/obj_sig.ps1 $objCopy -OutJson $sigJson | Out-Null
  $sigSha = (Get-FileHash -Algorithm SHA256 $sigJson).Hash

  # restore env vars
  foreach ($k in $old.Keys) {
    [System.Environment]::SetEnvironmentVariable($k, $old[$k], "Process")
  }

  return @{
    mode = $modeLabel
    log = $logCopy
    obj = $objCopy
    sig_sha256 = $sigSha
    wall_ms = [int]$sw.ElapsedMilliseconds
    gen_total_ms = $times.total_ms
    mesh_ms = $times.mesh_ms
    bool_ms = $times.bool_ms
    cache_flush = $times.cache_flush
  }
}

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path
Push-Location $repoRoot
try {
  $now = Run-One "now" @{
    "_clear" = "AIOS_GEN_INPUT_CACHE,AIOS_GEN_INPUT_CACHE_ONLY,AIOS_GEN_INPUT_CACHE_PIPELINE"
  }

  $cacheOnly = Run-One "cache_only" @{
    "AIOS_GEN_INPUT_CACHE" = "1"
    "AIOS_GEN_INPUT_CACHE_ONLY" = "1"
    "_clear" = "AIOS_GEN_INPUT_CACHE_PIPELINE"
  }

  Write-Host ""
  Write-Host "==== COMPARE ===="
  Write-Host ("sig(now)        : {0}" -f $now.sig_sha256)
  Write-Host ("sig(cache_only) : {0}" -f $cacheOnly.sig_sha256)
  Write-Host ("sig_equal       : {0}" -f ($now.sig_sha256 -eq $cacheOnly.sig_sha256))

  Write-Host ("gen_total_ms(now)        : {0}" -f $now.gen_total_ms)
  Write-Host ("gen_total_ms(cache_only) : {0}" -f $cacheOnly.gen_total_ms)
  if ($now.gen_total_ms -ne $null -and $cacheOnly.gen_total_ms -ne $null) {
    Write-Host ("gen_total_ms_delta       : {0}" -f ($cacheOnly.gen_total_ms - $now.gen_total_ms))
  }

  Write-Host ("mesh_ms(now)        : {0}" -f $now.mesh_ms)
  Write-Host ("mesh_ms(cache_only) : {0}" -f $cacheOnly.mesh_ms)
  if ($now.mesh_ms -ne $null -and $cacheOnly.mesh_ms -ne $null) {
    Write-Host ("mesh_ms_delta      : {0}" -f ($cacheOnly.mesh_ms - $now.mesh_ms))
  }

  Write-Host ("bool_ms(now)        : {0}" -f $now.bool_ms)
  Write-Host ("bool_ms(cache_only) : {0}" -f $cacheOnly.bool_ms)
  if ($now.bool_ms -ne $null -and $cacheOnly.bool_ms -ne $null) {
    Write-Host ("bool_ms_delta      : {0}" -f ($cacheOnly.bool_ms - $now.bool_ms))
  }

  if ($now.cache_flush -and $cacheOnly.cache_flush) {
    $eq = ($now.cache_flush.inst_info -eq $cacheOnly.cache_flush.inst_info) -and `
          ($now.cache_flush.inst_geos -eq $cacheOnly.cache_flush.inst_geos) -and `
          ($now.cache_flush.inst_tubi -eq $cacheOnly.cache_flush.inst_tubi) -and `
          ($now.cache_flush.neg -eq $cacheOnly.cache_flush.neg) -and `
          ($now.cache_flush.ngmr -eq $cacheOnly.cache_flush.ngmr) -and `
          ($now.cache_flush.bool -eq $cacheOnly.cache_flush.bool)
    Write-Host ("cache_flush_equal  : {0}" -f $eq)
  }
} finally {
  Pop-Location | Out-Null
}
