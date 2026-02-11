param(
  # 作为模板的基础配置（无扩展名）
  [Parameter(Mandatory=$false)]
  [string]$BaseConfig = "db_options/DbOption",

  [Parameter(Mandatory=$false)]
  [int]$Dbnum = 7997,

  [Parameter(Mandatory=$false)]
  [string]$Noun = "BRAN",

  # 为了可控：默认只跑前 N 个 refno（0 表示不限制，可能非常久）
  [Parameter(Mandatory=$false)]
  [int]$Limit = 200,

  # 两次运行：cold(清空缓存) + warm(不清空)，用于对比缓存命中带来的收益
  [Parameter(Mandatory=$false)]
  [switch]$WarmRun = $true,

  # 单次运行超时（秒），避免挂死
  [Parameter(Mandatory=$false)]
  [int]$TimeoutSeconds = 1800
)

$ErrorActionPreference = "Stop"

function Read-BaseToml([string]$configNoExt) {
  $p = "$configNoExt.toml"
  if (-not (Test-Path $p)) { throw "config toml not found: $p" }
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
  $k = [regex]::Escape($key)
  $pattern = ('^\s*{0}\s*=.*$' -f $k)
  $opt = [System.Text.RegularExpressions.RegexOptions]::Multiline
  if ([regex]::IsMatch($toml, $pattern, $opt)) {
    return [regex]::Replace($toml, $pattern, ("$key = $valueLiteral"), $opt)
  }
  # 插入到第一个 [table] 之前，确保在 root
  $insert = "$key = $valueLiteral`r`n"
  $m = [regex]::Match($toml, '^\s*\[', $opt)
  if ($m.Success) { return $toml.Insert($m.Index, $insert) }
  return ($toml.TrimEnd() + "`r`n" + $insert)
}

function Ensure-EmptyDir([string]$p) {
  if (Test-Path $p) { Remove-Item -LiteralPath $p -Recurse -Force -ErrorAction SilentlyContinue }
  New-Item -ItemType Directory -Path $p | Out-Null
}

function Find-NewestLog([datetime]$since) {
  $logs = Get-ChildItem -Path "logs" -Filter "*_parse.log" -ErrorAction SilentlyContinue |
    Where-Object { $_.LastWriteTime -ge $since } |
    Sort-Object LastWriteTime -Descending
  if (-not $logs -or $logs.Count -eq 0) { return $null }
  return $logs[0].FullName
}

function Find-NewestPerfCsv([datetime]$since, [string]$perfDir, [int]$dbnum) {
  if (-not (Test-Path $perfDir)) { return $null }
  $pattern = ("perf_gen_model_full_noun_dbnum_{0}_*.csv" -f $dbnum)
  $files = Get-ChildItem -Path $perfDir -Filter $pattern -ErrorAction SilentlyContinue |
    Where-Object { $_.LastWriteTime -ge $since } |
    Sort-Object LastWriteTime -Descending
  if (-not $files -or $files.Count -eq 0) { return $null }
  return $files[0].FullName
}

function Parse-TimeFromPerfCsv([string]$csvPath) {
  $rows = Import-Csv -LiteralPath $csvPath
  if (-not $rows) { return @{ total_ms = $null; mesh_ms = $null; bool_ms = $null } }

  $total = ($rows | Where-Object { $_.Stage -eq "TOTAL" } | Select-Object -First 1)."Duration(ms)"
  $mesh  = ($rows | Where-Object { $_.Stage -eq "mesh_generation" } | Select-Object -First 1)."Duration(ms)"
  $bool  = ($rows | Where-Object { $_.Stage -eq "boolean_operation" } | Select-Object -First 1)."Duration(ms)"

  $totalMs = if ($total) { [int][double]$total } else { $null }
  $meshMs  = if ($mesh)  { [int][double]$mesh }  else { $null }
  $boolMs  = if ($bool)  { [int][double]$bool }  else { $null }
  return @{ total_ms = $totalMs; mesh_ms = $meshMs; bool_ms = $boolMs }
}

function Parse-TimeFromLog([string]$logPath) {
  $content = Get-Content -LiteralPath $logPath -Raw
  # 日志文案在不同版本间有差异，这里做兼容解析（优先匹配新文案）。
  $mTotal = [regex]::Match($content, "gen_all_geos_data 总耗时:\s*(\d+)\s*ms")
  if (-not $mTotal.Success) {
    $mTotal = [regex]::Match($content, "gen_all_geos_data 完成，总耗时\s+(\d+)\s+ms")
  }

  $mMesh  = [regex]::Match($content, "mesh 生成完成，用时\s*(\d+)\s*ms")
  if (-not $mMesh.Success) {
    $mMesh = [regex]::Match($content, "完成 mesh 生成.*用时\s+(\d+)\s+ms")
  }

  $mBool  = [regex]::Match($content, "布尔运算完成，用时\s*(\d+)\s*ms")
  if (-not $mBool.Success) {
    $mBool = [regex]::Match($content, "完成布尔运算.*用时\s+(\d+)\s+ms")
  }
  $totalMs = if ($mTotal.Success) { [int]$mTotal.Groups[1].Value } else { $null }
  $meshMs  = if ($mMesh.Success)  { [int]$mMesh.Groups[1].Value } else { $null }
  $boolMs  = if ($mBool.Success)  { [int]$mBool.Groups[1].Value } else { $null }
  return @{ total_ms = $totalMs; mesh_ms = $meshMs; bool_ms = $boolMs }
}

function Run-One([string]$label, [string]$cfgNoExt, [string]$cacheDir, [string]$meshDir, [string]$perfDir, [bool]$clearCache) {
  Write-Host ""
  Write-Host ("==== PROFILE RUN: {0} ====" -f $label)
  Write-Host ("config : {0}" -f $cfgNoExt)
  Write-Host ("cache  : {0}" -f $cacheDir)
  Write-Host ("meshes : {0}" -f $meshDir)
  Write-Host ("limit  : {0}" -f $Limit)

  if ($clearCache) {
    Ensure-EmptyDir $cacheDir
    Ensure-EmptyDir $meshDir
  }

  $exe = "target/debug/aios-database.exe"
  if (-not (Test-Path $exe)) {
    Write-Host "[info] building aios-database (dev profile)..."
    cargo build --bin aios-database | Out-Null
  }

  $since = Get-Date
  $sw = [System.Diagnostics.Stopwatch]::StartNew()

  $p = Start-Process -FilePath $exe -ArgumentList @("--config", $cfgNoExt, "--regen-model") -NoNewWindow -PassThru
  $exited = $p.WaitForExit($TimeoutSeconds * 1000)
  $sw.Stop()

  if (-not $exited) {
    try { Stop-Process -Id $p.Id -Force } catch {}
    throw "timeout after ${TimeoutSeconds}s (killed pid=$($p.Id))"
  }

  $outDir = "output/_profile/bran_${Dbnum}"
  if (-not (Test-Path $outDir)) { New-Item -ItemType Directory -Path $outDir | Out-Null }
  # 1) 复制 parse log（有些环境下该文件不包含 perf summary，但仍有初始化/错误信息）
  $logCopy = $null
  $logPath = Find-NewestLog $since
  if ($logPath) {
    $logCopy = Join-Path $outDir ("{0}.log" -f $label)
    Copy-Item -Force $logPath $logCopy
  }

  # 2) 优先用 perf csv/json（它是结构化输出，最稳定）
  $perfCsv = Find-NewestPerfCsv $since $perfDir $Dbnum
  $times = $null
  if ($perfCsv) {
    $perfCsvCopy = Join-Path $outDir ("{0}.perf.csv" -f $label)
    Copy-Item -Force $perfCsv $perfCsvCopy
    $perfJson = [System.IO.Path]::ChangeExtension($perfCsv, ".json")
    if (Test-Path $perfJson) {
      $perfJsonCopy = Join-Path $outDir ("{0}.perf.json" -f $label)
      Copy-Item -Force $perfJson $perfJsonCopy
    }
    $times = Parse-TimeFromPerfCsv $perfCsvCopy
  } elseif ($logPath) {
    $times = Parse-TimeFromLog $logCopy
  } else {
    throw "no perf csv or *_parse.log found since $since"
  }

  if ($logCopy) { Write-Host ("log         : {0}" -f $logCopy) }
  if ($perfCsv) { Write-Host ("perf_csv    : {0}" -f (Join-Path $outDir ("{0}.perf.csv" -f $label))) }
  Write-Host ("wall_ms     : {0}" -f $sw.ElapsedMilliseconds)
  if ($times.total_ms -ne $null) { Write-Host ("gen_total_ms : {0}" -f $times.total_ms) }
  if ($times.mesh_ms -ne $null)  { Write-Host ("mesh_ms      : {0}" -f $times.mesh_ms) }
  if ($times.bool_ms -ne $null)  { Write-Host ("bool_ms      : {0}" -f $times.bool_ms) }

  return @{ label = $label; log = $logCopy; wall_ms = [int]$sw.ElapsedMilliseconds; times = $times }
}

Push-Location (Resolve-Path ".").Path
try {
  $baseToml = Read-BaseToml $BaseConfig
  $projectName = Get-TomlStringValue $baseToml "project_name"
  if (-not $projectName) { throw "failed to parse project_name from $BaseConfig.toml" }

  # 临时 config（db_options/_tmp 已在 .gitignore）
  if (-not (Test-Path "db_options/_tmp")) { New-Item -ItemType Directory -Path "db_options/_tmp" | Out-Null }
  $cfgNoExt = "db_options/_tmp/DbOption-profile-bran-${Dbnum}"

  $cacheBase = "output/_profile_cache/bran_${Dbnum}"
  $cacheDir = (Join-Path $cacheBase "instance_cache").Replace('\','/')
  $meshDir  = (Join-Path $cacheBase "meshes").Replace('\','/')

  $toml = $baseToml
  $toml = Upsert-TomlLine $toml "enable_log" "true"
  $toml = Upsert-TomlLine $toml "export_instances" "false"
  # profile: 减少非核心开销（不写 DB / 不跑 precheck）
  $toml = Upsert-TomlLine $toml "save_db" "false"
  $toml = Upsert-TomlLine $toml "use_surrealdb" "false"
  # 只跑指定 dbnum / noun
  $toml = Upsert-TomlLine $toml "manual_db_nums" ("[{0}]" -f $Dbnum)
  $toml = Upsert-TomlLine $toml "full_noun_mode" "true"
  $toml = Upsert-TomlLine $toml "full_noun_enabled_categories" ("[`"{0}`"]" -f $Noun)
  $toml = Upsert-TomlLine $toml "full_noun_excluded_nouns" "[]"
  $toml = Upsert-TomlLine $toml "debug_limit_per_noun" ("{0}" -f $Limit)
  # 输出目录隔离（便于清空）
  $toml = Upsert-TomlLine $toml "foyer_cache_dir" ("`"{0}`"" -f $cacheDir)
  $toml = Upsert-TomlLine $toml "meshes_path" ("`"{0}`"" -f $meshDir)

  Set-Content -LiteralPath ("$cfgNoExt.toml") -Value $toml -Encoding UTF8

  $perfDir = ("output/{0}/profile" -f $projectName).Replace('\','/')

  $cold = Run-One "cold" $cfgNoExt $cacheDir $meshDir $perfDir $true

  $warm = $null
  if ($WarmRun) {
    $warm = Run-One "warm" $cfgNoExt $cacheDir $meshDir $perfDir $false
  }

  Write-Host ""
  Write-Host "==== PROFILE SUMMARY ===="
  Write-Host ("cold.wall_ms     : {0}" -f $cold.wall_ms)
  Write-Host ("cold.gen_total_ms: {0}" -f $cold.times.total_ms)
  Write-Host ("cold.mesh_ms     : {0}" -f $cold.times.mesh_ms)
  Write-Host ("cold.bool_ms     : {0}" -f $cold.times.bool_ms)
  if ($warm) {
    Write-Host ("warm.wall_ms     : {0}" -f $warm.wall_ms)
    Write-Host ("warm.gen_total_ms: {0}" -f $warm.times.total_ms)
    Write-Host ("warm.mesh_ms     : {0}" -f $warm.times.mesh_ms)
    Write-Host ("warm.bool_ms     : {0}" -f $warm.times.bool_ms)
    if ($cold.times.total_ms -ne $null -and $warm.times.total_ms -ne $null) {
      Write-Host ("delta.gen_total_ms (warm-cold): {0}" -f ($warm.times.total_ms - $cold.times.total_ms))
    }
  }
} finally {
  Pop-Location | Out-Null
}
