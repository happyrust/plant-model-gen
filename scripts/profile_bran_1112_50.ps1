# dbnum 1112 的 50 个 BRAN 性能测试
# 用于性能分析，定位耗时环节
# 正常执行模式（非 --regen-model），走标准 sync + gen_model 管线
# 用法: .\scripts\profile_bran_1112_50.ps1

param(
  [string]$BaseConfig = "db_options/DbOption",
  [switch]$Release = $true,  # 默认 release 模式
  [int]$Dbnum = 1112,
  [int]$Limit = 50,
  [switch]$WarmRun = $false,  # 仅冷启动一次，便于专注分析
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
  $pattern = "perf_gen_model_index_tree_dbnum_{0}_*.csv" -f $dbnum
  $files = Get-ChildItem -Path $perfDir -Filter $pattern -ErrorAction SilentlyContinue |
  Where-Object { $_.LastWriteTime -ge $since } |
  Sort-Object LastWriteTime -Descending
  if ($files -and $files.Count -gt 0) { return $files[0].FullName }
  return $null
}

Push-Location (Resolve-Path ".").Path
try {
  $baseToml = Read-BaseToml $BaseConfig
  $projectName = Get-TomlStringValue $baseToml "project_name"
  if (-not $projectName) { throw "failed to parse project_name from $BaseConfig.toml" }

  if (-not (Test-Path "db_options/_tmp")) { New-Item -ItemType Directory -Path "db_options/_tmp" | Out-Null }
  $cfgNoExt = "db_options/_tmp/DbOption-profile-bran-${Dbnum}-${Limit}"

  $cacheBase = "output/_profile_cache/bran_${Dbnum}_${Limit}"
  $cacheDir = (Join-Path $cacheBase "instance_cache").Replace('\', '/')
  $meshDir = (Join-Path $cacheBase "meshes").Replace('\', '/')

  $toml = $baseToml
  $toml = Upsert-TomlLine $toml "enable_log" "true"
  $toml = Upsert-TomlLine $toml "export_instances" "false"
  $toml = Upsert-TomlLine $toml "save_db" "false"
  $toml = Upsert-TomlLine $toml "use_surrealdb" "false"
  $toml = Upsert-TomlLine $toml "use_cache" "true"
  $toml = Upsert-TomlLine $toml "manual_db_nums" ("[{0}]" -f $Dbnum)
  $toml = Upsert-TomlLine $toml "index_tree_enabled_target_types" '["BRAN"]'
  $toml = Upsert-TomlLine $toml "index_tree_excluded_target_types" "[]"
  $toml = Upsert-TomlLine $toml "index_tree_debug_limit_per_target_type" "$Limit"
  $toml = Upsert-TomlLine $toml "foyer_cache_dir" ("`"{0}`"" -f $cacheDir)
  $toml = Upsert-TomlLine $toml "meshes_path" ("`"{0}`"" -f $meshDir)

  Set-Content -LiteralPath ("$cfgNoExt.toml") -Value $toml -Encoding UTF8

  $perfDir = ("output/{0}/profile" -f $projectName).Replace('\', '/')
  $outDir = "output/_profile/bran_${Dbnum}_${Limit}"
  if (-not (Test-Path $outDir)) { New-Item -ItemType Directory -Path $outDir | Out-Null }

  Write-Host ""
  Write-Host "==== 性能测试: dbnum=$Dbnum, 50 BRAN, Release=$Release ===="
  Write-Host "config : $cfgNoExt"
  Write-Host "cache  : $cacheDir"
  Write-Host "mode   : $(if ($Release) { 'release' } else { 'debug' })"

  Ensure-EmptyDir $cacheDir
  Ensure-EmptyDir $meshDir

  $exe = $(if ($Release) { "target/release/aios-database.exe" } else { "target/debug/aios-database.exe" })
  if (-not (Test-Path $exe)) {
    if ($Release) {
      Write-Host "[info] building aios-database (release)..."
      cargo build --release --bin aios-database
    } else {
      Write-Host "[info] building aios-database (debug)..."
      cargo build --bin aios-database
    }
  }

  $since = Get-Date
  $sw = [System.Diagnostics.Stopwatch]::StartNew()

  # 正常执行模式（不使用 --regen-model），走标准 sync + gen_model 管线
  $p = Start-Process -FilePath $exe -ArgumentList @("--config", $cfgNoExt) -NoNewWindow -PassThru
  $exited = $p.WaitForExit($TimeoutSeconds * 1000)
  $sw.Stop()

  if (-not $exited) {
    try { Stop-Process -Id $p.Id -Force } catch {}
    throw "timeout after ${TimeoutSeconds}s"
  }

  $logPath = Find-NewestLog $since
  if ($logPath) {
    $logCopy = Join-Path $outDir "run.log"
    Copy-Item -Force $logPath $logCopy
    Write-Host "log    : $logCopy"
  }

  $perfCsv = Find-NewestPerfCsv $since $perfDir $Dbnum
  if ($perfCsv) {
    $perfCopy = Join-Path $outDir "perf.csv"
    Copy-Item -Force $perfCsv $perfCopy
    Write-Host "perf   : $perfCopy"
    Write-Host ""
    Write-Host "==== 耗时分布（perf.csv）===="
    Import-Csv -LiteralPath $perfCopy | ForEach-Object {
      Write-Host ("  {0,-35} {1,10} ms" -f $_.Stage, $_.'Duration(ms)')
    }
  }

  $perfJson = [System.IO.Path]::ChangeExtension($perfCsv, ".json")
  if ($perfJson -and (Test-Path $perfJson)) {
    $jsonCopy = Join-Path $outDir "perf.json"
    Copy-Item -Force $perfJson $jsonCopy
    Write-Host "perf_json: $jsonCopy"
  }

  Write-Host ""
  Write-Host ("wall_ms: {0}" -f $sw.ElapsedMilliseconds)
}
finally {
  Pop-Location | Out-Null
}
