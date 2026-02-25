param(
  # 注意：--config 参数传的是“无扩展名”的路径（与 aios-database 一致）
  [Parameter(Mandatory=$false)]
  [string]$BaseConfig = "db_options/DbOption",

  [Parameter(Mandatory=$false)]
  [int]$Dbnum = 7997,

  [Parameter(Mandatory=$false)]
  [string]$Noun = "BRAN"
)

$ErrorActionPreference = "Stop"

function Get-RepoRoot() {
  $scriptDir = $PSScriptRoot
  return (Resolve-Path (Join-Path $scriptDir "..")).Path
}

function Read-BaseToml([string]$configNoExt) {
  $p = "$configNoExt.toml"
  if (-not (Test-Path $p)) {
    throw "config toml not found: $p"
  }
  return Get-Content -LiteralPath $p -Raw
}

function Get-TomlStringValue([string]$toml, [string]$key) {
  $k = [regex]::Escape($key)
  $opt = [System.Text.RegularExpressions.RegexOptions]::Multiline

  $p1 = ('^\s*{0}\s*=\s*"([^"]*)"\s*$' -f $k)
  $m = [regex]::Match($toml, $p1, $opt)
  if ($m.Success) { return $m.Groups[1].Value }

  $p2 = ("^\s*{0}\s*=\s*'([^']*)'\s*$" -f $k)
  $m2 = [regex]::Match($toml, $p2, $opt)
  if ($m2.Success) { return $m2.Groups[1].Value }
  return $null
}

function Upsert-TomlLine([string]$toml, [string]$key, [string]$valueLiteral) {
  # valueLiteral: already formatted (e.g. true, 123, "xxx", [1,2])
  $k = [regex]::Escape($key)
  $pattern = ('^\s*{0}\s*=.*$' -f $k)
  $opt = [System.Text.RegularExpressions.RegexOptions]::Multiline
  if ([regex]::IsMatch($toml, $pattern, $opt)) {
    return [regex]::Replace($toml, $pattern, ("$key = $valueLiteral"), $opt)
  }
  # TOML 作用域：若文件后面已经进入 [table]，直接 append 会落到 table 里。
  # 因此缺失字段统一插入到“第一个 [table] 之前”，确保写在 root。
  $insert = "$key = $valueLiteral`r`n"
  $m = [regex]::Match($toml, '^\s*\[', $opt)
  if ($m.Success) {
    return $toml.Insert($m.Index, $insert)
  }
  return ($toml.TrimEnd() + "`r`n" + $insert)
}

function Ensure-DirEmpty([string]$p) {
  if (Test-Path $p) {
    Remove-Item -LiteralPath $p -Recurse -Force -ErrorAction SilentlyContinue
  }
  New-Item -ItemType Directory -Path $p | Out-Null
}

function Find-NewestLog([datetime]$since) {
  $logs = Get-ChildItem -Path "logs" -Filter "*.log" -ErrorAction SilentlyContinue |
    Where-Object { $_.LastWriteTime -ge $since } |
    Sort-Object LastWriteTime -Descending
  if (-not $logs -or $logs.Count -eq 0) { return $null }
  return $logs[0].FullName
}

function Parse-TimeFromLog([string]$logPath) {
  $content = Get-Content -LiteralPath $logPath -Raw
  $mTotal = [regex]::Match($content, "gen_all_geos_data 完成，总耗时\s+(\d+)\s+ms")
  $mMesh  = [regex]::Match($content, "完成 mesh 生成，用时\s+(\d+)\s+ms")
  $mBool  = [regex]::Match($content, "完成布尔运算，用时\s+(\d+)\s+ms")
  $totalMs = if ($mTotal.Success) { [int]$mTotal.Groups[1].Value } else { $null }
  $meshMs  = if ($mMesh.Success)  { [int]$mMesh.Groups[1].Value } else { $null }
  $boolMs  = if ($mBool.Success)  { [int]$mBool.Groups[1].Value } else { $null }
  return @{ total_ms = $totalMs; mesh_ms = $meshMs; bool_ms = $boolMs }
}

function Pick-LikelyInstancesJson([string]$projectName, [int]$dbnum, [datetime]$since) {
  $candidates = Get-ChildItem -Path "output" -Recurse -Filter ("instances_{0}.json" -f $dbnum) -ErrorAction SilentlyContinue |
    Where-Object { $_.LastWriteTime -ge $since } |
    Where-Object { $_.FullName -like "*\\$projectName\\*" } |
    Sort-Object Length -Descending
  if ($candidates -and $candidates.Count -gt 0) {
    return $candidates[0].FullName
  }
  # fallback: any newest
  $fallback = Get-ChildItem -Path "output" -Recurse -Filter ("instances_{0}.json" -f $dbnum) -ErrorAction SilentlyContinue |
    Where-Object { $_.LastWriteTime -ge $since } |
    Sort-Object Length -Descending
  if ($fallback -and $fallback.Count -gt 0) { return $fallback[0].FullName }
  return $null
}

function Mesh-DirSig([string]$meshDir) {
  if (-not (Test-Path $meshDir)) { return $null }
  $files = Get-ChildItem -Path $meshDir -Recurse -File -ErrorAction SilentlyContinue
  if (-not $files) { return @{ file_count = 0; total_bytes = 0; sig_sha256 = "EMPTY" } }

  $items = $files | ForEach-Object {
    $rel = $_.FullName.Substring($meshDir.Length).TrimStart("\","/")
    "{0}`t{1}" -f $rel.Replace("\","/"), $_.Length
  } | Sort-Object

  $tmp = [System.IO.Path]::GetTempFileName()
  Set-Content -LiteralPath $tmp -Value ($items -join "`n") -Encoding UTF8
  $h = (Get-FileHash -Algorithm SHA256 $tmp).Hash
  Remove-Item -LiteralPath $tmp -ErrorAction SilentlyContinue

  $total = ($files | Measure-Object -Property Length -Sum).Sum
  return @{
    file_count = ($files | Measure-Object).Count
    total_bytes = [int64]$total
    sig_sha256 = $h
  }
}

function Run-One([string]$mode, [hashtable]$envs, [string]$cfgNoExt, [string]$cacheDir, [string]$meshDir, [string]$projectName) {
  Write-Host ""
  Write-Host ("==== RUN: {0} (dbnum={1}, noun={2} deprecated/ignored) ====" -f $mode, $Dbnum, $Noun)

  # 公平：清空缓存与 mesh 输出目录
  Ensure-DirEmpty $cacheDir
  Ensure-DirEmpty $meshDir

  # 同时清理 instances 导出目录（避免误读旧文件）
  $projInstDir = Join-Path "output" (Join-Path $projectName "instances")
  if (Test-Path $projInstDir) {
    Remove-Item -LiteralPath $projInstDir -Recurse -Force -ErrorAction SilentlyContinue
  }
  $projIndexDir = Join-Path "output" (Join-Path $projectName (Join-Path "instances_cache_for_index" $Dbnum))
  if (Test-Path $projIndexDir) {
    Remove-Item -LiteralPath $projIndexDir -Recurse -Force -ErrorAction SilentlyContinue
  }

  $old = @{}
  foreach ($k in $envs.Keys) {
    $old[$k] = [System.Environment]::GetEnvironmentVariable($k, "Process")
    [System.Environment]::SetEnvironmentVariable($k, $envs[$k], "Process")
  }
  if ($envs.ContainsKey("_clear")) {
    foreach ($k in ($envs["_clear"] -split ",")) {
      $key = $k.Trim()
      if ($key) {
        $old[$key] = [System.Environment]::GetEnvironmentVariable($key, "Process")
        [System.Environment]::SetEnvironmentVariable($key, $null, "Process")
      }
    }
  }

  $exe = "target/debug/aios-database.exe"
  if (-not (Test-Path $exe)) {
    cargo build --bin aios-database | Out-Null
  }

  $since = Get-Date
  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  & $exe --config $cfgNoExt --regen-model | Out-Null
  $sw.Stop()

  $logPath = Find-NewestLog $since
  if (-not $logPath) { throw "no log found since $since" }
  $times = Parse-TimeFromLog $logPath

  $instJson = Pick-LikelyInstancesJson $projectName $Dbnum $since

  $outBase = Join-Path "output/_compare" ("bran_{0}" -f $Dbnum)
  if (-not (Test-Path $outBase)) { New-Item -ItemType Directory -Path $outBase | Out-Null }

  $logCopy = Join-Path $outBase ("{0}.log" -f $mode)
  Copy-Item -Force $logPath $logCopy

  $instCopy = $null
  $instSig = $null
  if ($instJson) {
    $instCopy = Join-Path $outBase ("{0}_instances_{1}.json" -f $mode, $Dbnum)
    Copy-Item -Force $instJson $instCopy
    $instSig = & powershell -ExecutionPolicy Bypass -File scripts/json_sig.ps1 $instCopy
  }

  $meshSig = Mesh-DirSig $meshDir

  foreach ($k in $old.Keys) {
    [System.Environment]::SetEnvironmentVariable($k, $old[$k], "Process")
  }

  Write-Host ("log           : {0}" -f $logCopy)
  Write-Host ("wall_ms        : {0}" -f $sw.ElapsedMilliseconds)
  if ($times.total_ms -ne $null) { Write-Host ("gen_total_ms   : {0}" -f $times.total_ms) }
  if ($times.mesh_ms -ne $null)  { Write-Host ("mesh_ms        : {0}" -f $times.mesh_ms) }
  if ($times.bool_ms -ne $null)  { Write-Host ("bool_ms        : {0}" -f $times.bool_ms) }
  if ($instCopy) { Write-Host ("instances_json : {0}" -f $instCopy) } else { Write-Host "instances_json : <not found>" }
  if ($meshSig) {
    Write-Host ("mesh_dir       : {0}" -f $meshDir)
    Write-Host ("mesh_sig       : files={0} bytes={1} sha256={2}" -f $meshSig.file_count, $meshSig.total_bytes, $meshSig.sig_sha256)
  }

  $instHash = $null
  if ($instCopy) {
    $instHash = (Get-FileHash -Algorithm SHA256 $instCopy).Hash
  }

  return @{
    mode = $mode
    log = $logCopy
    wall_ms = [int]$sw.ElapsedMilliseconds
    gen_total_ms = $times.total_ms
    mesh_ms = $times.mesh_ms
    bool_ms = $times.bool_ms
    instances_json = $instCopy
    instances_file_sha256 = $instHash
    mesh_sig = $meshSig
    json_sig_output = $instSig
  }
}

$repoRoot = Get-RepoRoot
Push-Location $repoRoot
try {
  $baseToml = Read-BaseToml $BaseConfig
  $projectName = Get-TomlStringValue $baseToml "project_name"
  if (-not $projectName) { throw "failed to parse project_name from $BaseConfig.toml" }

  $tmpDir = "db_options/_tmp"
  if (-not (Test-Path $tmpDir)) { New-Item -ItemType Directory -Path $tmpDir | Out-Null }

  $cacheBase = Join-Path "output/_compare_cache" ("bran_{0}" -f $Dbnum)

  $cfgNowNoExt = Join-Path $tmpDir ("DbOption-bran-{0}-now" -f $Dbnum)
  $cfgCacheNoExt = Join-Path $tmpDir ("DbOption-bran-{0}-cacheonly" -f $Dbnum)

  $nowCacheDir = Join-Path $cacheBase "now/instance_cache"
  $nowMeshDir  = Join-Path $cacheBase "now/meshes"
  $coCacheDir  = Join-Path $cacheBase "cache_only/instance_cache"
  $coMeshDir   = Join-Path $cacheBase "cache_only/meshes"

  $tomlNow = $baseToml
  $tomlNow = Upsert-TomlLine $tomlNow "enable_log" "true"
  $tomlNow = Upsert-TomlLine $tomlNow "export_instances" "true"
  # 为公平对比：避免 precheck 扫全库 + 避免 DB 写入，把输出统一落在 foyer/meshes
  $tomlNow = Upsert-TomlLine $tomlNow "use_surrealdb" "false"
  $tomlNow = Upsert-TomlLine $tomlNow "save_db" "false"
  $tomlNow = Upsert-TomlLine $tomlNow "index_tree_enabled_target_types" "[]"
  $tomlNow = Upsert-TomlLine $tomlNow "index_tree_excluded_target_types" "[]"
  $tomlNow = Upsert-TomlLine $tomlNow "index_tree_debug_limit_per_target_type" "0"
  $tomlNow = Upsert-TomlLine $tomlNow "manual_db_nums" ("[{0}]" -f $Dbnum)
  $tomlNow = Upsert-TomlLine $tomlNow "foyer_cache_dir" ("`"{0}`"" -f $nowCacheDir.Replace('\','/'))
  $tomlNow = Upsert-TomlLine $tomlNow "meshes_path" ("`"{0}`"" -f $nowMeshDir.Replace('\','/'))
  Set-Content -LiteralPath ("$cfgNowNoExt.toml") -Value $tomlNow -Encoding UTF8

  $tomlCO = $baseToml
  $tomlCO = Upsert-TomlLine $tomlCO "enable_log" "true"
  $tomlCO = Upsert-TomlLine $tomlCO "export_instances" "true"
  $tomlCO = Upsert-TomlLine $tomlCO "use_surrealdb" "false"
  $tomlCO = Upsert-TomlLine $tomlCO "save_db" "false"
  $tomlCO = Upsert-TomlLine $tomlCO "index_tree_enabled_target_types" "[]"
  $tomlCO = Upsert-TomlLine $tomlCO "index_tree_excluded_target_types" "[]"
  $tomlCO = Upsert-TomlLine $tomlCO "index_tree_debug_limit_per_target_type" "0"
  $tomlCO = Upsert-TomlLine $tomlCO "manual_db_nums" ("[{0}]" -f $Dbnum)
  $tomlCO = Upsert-TomlLine $tomlCO "foyer_cache_dir" ("`"{0}`"" -f $coCacheDir.Replace('\','/'))
  $tomlCO = Upsert-TomlLine $tomlCO "meshes_path" ("`"{0}`"" -f $coMeshDir.Replace('\','/'))
  Set-Content -LiteralPath ("$cfgCacheNoExt.toml") -Value $tomlCO -Encoding UTF8

  $now = Run-One "now" @{
    "_clear" = "AIOS_GEN_INPUT_CACHE,AIOS_GEN_INPUT_CACHE_ONLY,AIOS_GEN_INPUT_CACHE_PIPELINE"
  } $cfgNowNoExt $nowCacheDir $nowMeshDir $projectName

  $cacheOnly = Run-One "cache_only" @{
    "AIOS_GEN_INPUT_CACHE" = "1"
    "AIOS_GEN_INPUT_CACHE_ONLY" = "1"
    "_clear" = "AIOS_GEN_INPUT_CACHE_PIPELINE"
  } $cfgCacheNoExt $coCacheDir $coMeshDir $projectName

  Write-Host ""
  Write-Host "==== COMPARE RESULT ===="
  if ($now.instances_json -and $cacheOnly.instances_json) {
    $sigNow = (& powershell -ExecutionPolicy Bypass -File scripts/json_sig.ps1 $now.instances_json | Select-String -Pattern "sig\\.sha256\\s*:\\s*([0-9A-F]+)").Matches.Groups[1].Value
    $sigCO  = (& powershell -ExecutionPolicy Bypass -File scripts/json_sig.ps1 $cacheOnly.instances_json | Select-String -Pattern "sig\\.sha256\\s*:\\s*([0-9A-F]+)").Matches.Groups[1].Value
    Write-Host ("instances_sig_equal : {0}" -f ($sigNow -eq $sigCO))
    Write-Host ("instances_sig(now)  : {0}" -f $sigNow)
    Write-Host ("instances_sig(co)   : {0}" -f $sigCO)
  } else {
    Write-Host "instances_sig_equal : <skip (instances json missing)>"
  }

  if ($now.gen_total_ms -ne $null -and $cacheOnly.gen_total_ms -ne $null) {
    Write-Host ("gen_total_ms(now)   : {0}" -f $now.gen_total_ms)
    Write-Host ("gen_total_ms(co)    : {0}" -f $cacheOnly.gen_total_ms)
    Write-Host ("gen_total_ms_delta  : {0}" -f ($cacheOnly.gen_total_ms - $now.gen_total_ms))
  }
  if ($now.mesh_ms -ne $null -and $cacheOnly.mesh_ms -ne $null) {
    Write-Host ("mesh_ms(now)        : {0}" -f $now.mesh_ms)
    Write-Host ("mesh_ms(co)         : {0}" -f $cacheOnly.mesh_ms)
    Write-Host ("mesh_ms_delta       : {0}" -f ($cacheOnly.mesh_ms - $now.mesh_ms))
  }
  if ($now.bool_ms -ne $null -and $cacheOnly.bool_ms -ne $null) {
    Write-Host ("bool_ms(now)        : {0}" -f $now.bool_ms)
    Write-Host ("bool_ms(co)         : {0}" -f $cacheOnly.bool_ms)
    Write-Host ("bool_ms_delta       : {0}" -f ($cacheOnly.bool_ms - $now.bool_ms))
  }

  if ($now.mesh_sig -and $cacheOnly.mesh_sig) {
    $meshEq = ($now.mesh_sig.sig_sha256 -eq $cacheOnly.mesh_sig.sig_sha256)
    Write-Host ("mesh_sig_equal      : {0}" -f $meshEq)
    Write-Host ("mesh_sig(now)       : {0}" -f $now.mesh_sig.sig_sha256)
    Write-Host ("mesh_sig(co)        : {0}" -f $cacheOnly.mesh_sig.sig_sha256)
  }
} finally {
  Pop-Location | Out-Null
}
