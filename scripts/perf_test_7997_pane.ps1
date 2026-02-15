<#
.SYNOPSIS
    PANE 实例生成性能对比脚本（dbnum=7997）

.DESCRIPTION
    对比两种实例生成路径的性能：
      方案 A（基线）：冷启动（清空 foyer cache），SurrealDB 作为输入源
      方案 B（缓存）：热启动（foyer cache 已填充），缓存命中

    每方案各运行 1 轮，PerfTimer JSON/CSV 由程序自动输出到 output/YCYK-E3D/profile/。
    脚本最后汇总两轮 JSON 生成 Markdown 性能报告。

.PARAMETER SkipBuild
    跳过 cargo build 步骤（适用于已编译场景）

.EXAMPLE
    .\scripts\perf_test_7997_pane.ps1
    .\scripts\perf_test_7997_pane.ps1 -SkipBuild
#>

param(
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

# ========== 配置 ==========
$ConfigName   = "db_options/DbOption-7997-pane-perf"
$BinaryName   = "aios-database"
# 该脚本使用的配置文件项目名为 AvevaMarineSample，因此这里需与程序输出目录一致；
# 否则会导致无法定位 perf JSON，进而生成空报告。
$ProjectName  = "AvevaMarineSample"
$ProfileDir   = "output/$ProjectName/profile"
$CacheDir     = "output/$ProjectName/instance_cache"
$ReportDir    = "output/$ProjectName/profile"

# ========== 构建 ==========
if (-not $SkipBuild) {
    Write-Host "`n========== 编译 (debug) ==========" -ForegroundColor Cyan
    cargo build --bin $BinaryName
    if ($LASTEXITCODE -ne 0) {
        Write-Error "cargo build 失败，退出"
        exit 1
    }
}

$BinaryPath = "target/debug/$BinaryName.exe"
if (-not (Test-Path $BinaryPath)) {
    Write-Error "找不到二进制文件: $BinaryPath"
    exit 1
}

# ========== 辅助函数 ==========
function Get-LatestPerfJson {
    <# 返回 profile 目录中最新的 perf JSON 文件路径 #>
    if (-not (Test-Path $ProfileDir)) { return $null }
    $files = Get-ChildItem -Path $ProfileDir -Filter "perf_gen_model_full_noun_dbnum_7997_*.json" |
             Sort-Object LastWriteTime -Descending
    if ($files.Count -gt 0) { return $files[0].FullName }
    return $null
}

function Run-ModelGen {
    param(
        [string]$Label,
        [string]$Phase   # "cold" 或 "hot"
    )
    Write-Host "`n========== $Label ($Phase) ==========" -ForegroundColor Yellow

    $beforeJson = Get-LatestPerfJson

    $env:FORCE_REPLACE_MESH = "true"
    & $BinaryPath --config $ConfigName --regen-model 2>&1 | ForEach-Object { Write-Host $_ }
    $exitCode = $LASTEXITCODE
    Remove-Item Env:\FORCE_REPLACE_MESH -ErrorAction SilentlyContinue

    if ($exitCode -ne 0) {
        Write-Warning "$Label ($Phase) 执行失败 (exit=$exitCode)，继续..."
    }

    # 找到本次新生成的 JSON
    $afterJson = Get-LatestPerfJson
    if ($afterJson -and ($afterJson -ne $beforeJson)) {
        Write-Host "  -> perf JSON: $afterJson" -ForegroundColor Green
        return $afterJson
    } else {
        Write-Warning "  未检测到新的 perf JSON 文件"
        return $null
    }
}

# ========== 方案 A：基线（冷启动） ==========
Write-Host "`n========== 清空 foyer cache ==========" -ForegroundColor Magenta
if (Test-Path $CacheDir) {
    Remove-Item -Recurse -Force $CacheDir
    Write-Host "  已清空: $CacheDir"
} else {
    Write-Host "  缓存目录不存在，跳过清理"
}

$jsonCold = Run-ModelGen -Label "方案A-基线" -Phase "cold"

# ========== 方案 B：缓存（热启动） ==========
$jsonHot  = Run-ModelGen -Label "方案B-缓存" -Phase "hot"

# ========== 生成 Markdown 报告 ==========
Write-Host "`n========== 生成性能报告 ==========" -ForegroundColor Cyan

$timestamp = Get-Date -Format "yyyyMMdd_HHmmss"
$reportPath = Join-Path $ReportDir "perf_report_pane_$timestamp.md"

# 确保输出目录存在
if (-not (Test-Path $ReportDir)) {
    New-Item -ItemType Directory -Path $ReportDir -Force | Out-Null
}

function Parse-PerfJson {
    param([string]$JsonPath)
    if (-not $JsonPath -or -not (Test-Path $JsonPath)) { return $null }
    return Get-Content $JsonPath -Raw | ConvertFrom-Json
}

$cold = Parse-PerfJson $jsonCold
$hot  = Parse-PerfJson $jsonHot

$report = @"
# PANE 实例生成性能对比报告

- **生成时间**: $(Get-Date -Format "yyyy-MM-dd HH:mm:ss")
- **dbnum**: 7997
- **noun**: PANE (LOOP 管线)
- **gen_mesh**: false
- **use_surrealdb**: false (不回写)
- **配置文件**: ``$ConfigName``

## 总耗时对比

| 方案 | 总耗时 (ms) | 说明 |
|------|------------|------|

"@

if ($cold) {
    $report += "| 方案A-基线（冷启动） | $($cold.total_ms) | SurrealDB 输入，foyer cache 未命中 |`n"
}
if ($hot) {
    $report += "| 方案B-缓存（热启动） | $($hot.total_ms) | foyer cache 命中 |`n"
}

if ($cold -and $hot -and $cold.total_ms -gt 0) {
    $speedup = [math]::Round(($cold.total_ms - $hot.total_ms) / $cold.total_ms * 100, 1)
    $report += "`n**缓存加速比**: $speedup% (冷 $($cold.total_ms) ms -> 热 $($hot.total_ms) ms)`n"
}

$report += @"

## 各阶段耗时明细

### 方案A-基线（冷启动）

| 阶段 | 耗时 (ms) | 占比 (%) |
|------|----------|---------|

"@

if ($cold) {
    foreach ($stage in $cold.stages) {
        $dur = [math]::Round($stage.duration_ms, 1)
        $pct = [math]::Round($stage.percentage, 1)
        $report += "| $($stage.name) | $dur | $pct |`n"
    }
}

$report += @"

### 方案B-缓存（热启动）

| 阶段 | 耗时 (ms) | 占比 (%) |
|------|----------|---------|

"@

if ($hot) {
    foreach ($stage in $hot.stages) {
        $dur = [math]::Round($stage.duration_ms, 1)
        $pct = [math]::Round($stage.percentage, 1)
        $report += "| $($stage.name) | $dur | $pct |`n"
    }
}

# 阶段对比表
if ($cold -and $hot) {
    $report += @"

## 阶段对比

| 阶段 | 冷 (ms) | 热 (ms) | 差异 (ms) | 加速 (%) |
|------|--------|--------|----------|---------|

"@

    # 按阶段名对齐
    $coldMap = @{}
    foreach ($s in $cold.stages) { $coldMap[$s.name] = $s.duration_ms }
    foreach ($s in $hot.stages) {
        $name = $s.name
        $hotMs  = [math]::Round($s.duration_ms, 1)
        $coldMs = if ($coldMap.ContainsKey($name)) { [math]::Round($coldMap[$name], 1) } else { "-" }
        if ($coldMs -ne "-" -and $coldMs -gt 0) {
            $diff = [math]::Round($coldMs - $hotMs, 1)
            $pct  = [math]::Round($diff / $coldMs * 100, 1)
        } else {
            $diff = "-"
            $pct  = "-"
        }
        $report += "| $name | $coldMs | $hotMs | $diff | $pct |`n"
    }
}

$report += @"

## 元数据

"@

if ($cold) {
    $report += "- **冷启动 JSON**: ``$jsonCold```n"
    $report += "- **冷启动 metadata**: ``$($cold.metadata | ConvertTo-Json -Compress)```n"
}
if ($hot) {
    $report += "- **热启动 JSON**: ``$jsonHot```n"
    $report += "- **热启动 metadata**: ``$($hot.metadata | ConvertTo-Json -Compress)```n"
}

# 写入报告
$report | Out-File -FilePath $reportPath -Encoding utf8
Write-Host "`n========== 完成 ==========" -ForegroundColor Green
Write-Host "性能报告: $reportPath"
Write-Host ""
