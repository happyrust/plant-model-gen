# 7997 BRAN 模型生成性能测试脚本
#
# 功能：
# 1. 运行 7997 dbnum BRAN 模型生成
# 2. 自动收集性能数据（JSON + CSV）
# 3. 生成性能分析报告

$ErrorActionPreference = "Continue"

Write-Host "========================================" -ForegroundColor Cyan
Write-Host "7997 BRAN 模型生成性能测试" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

# 配置参数
$CONFIG_FILE = "DbOption-7997-bran-perf.toml"
$TIMESTAMP = Get-Date -Format "yyyyMMdd_HHmmss"
$LOG_DIR = "output/YCYK-E3D/logs"
$LOG_FILE = "$LOG_DIR/gen_model_7997_bran_$TIMESTAMP.log"

# 创建日志目录
New-Item -ItemType Directory -Force -Path $LOG_DIR | Out-Null

Write-Host "[1/4] 检查配置文件..." -ForegroundColor Yellow
if (!(Test-Path $CONFIG_FILE)) {
    Write-Host "错误: 配置文件不存在: $CONFIG_FILE" -ForegroundColor Red
    exit 1
}
Write-Host "配置文件: $CONFIG_FILE" -ForegroundColor Green

Write-Host ""
Write-Host "[2/4] 清理旧数据..." -ForegroundColor Yellow
# 可选：清理缓存
# Remove-Item -Path "foyer/instance_cache/*" -Recurse -Force -ErrorAction SilentlyContinue

Write-Host ""
Write-Host "[3/4] 开始模型生成（详细日志写入: $LOG_FILE）..." -ForegroundColor Yellow
Write-Host "命令: cargo run --release -- -c $CONFIG_FILE" -ForegroundColor Gray
Write-Host ""

# 运行模型生成，输出到控制台和日志文件
cargo run --release -- -c $CONFIG_FILE 2>&1 | Tee-Object -FilePath $LOG_FILE

$EXIT_CODE = $LASTEXITCODE

Write-Host ""
if ($EXIT_CODE -eq 0) {
    Write-Host "[4/4] 模型生成完成！" -ForegroundColor Green
} else {
    Write-Host "[4/4] 模型生成失败 (退出码: $EXIT_CODE)" -ForegroundColor Red
    Write-Host "详细日志: $LOG_FILE" -ForegroundColor Yellow
    exit $EXIT_CODE
}

Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "性能报告位置" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan

# 查找生成的性能报告
$PROFILE_DIR = "output/YCYK-E3D/profile"
if (Test-Path $PROFILE_DIR) {
    $JSON_FILES = Get-ChildItem -Path $PROFILE_DIR -Filter "perf_gen_model_full_noun_dbnum_7997_*.json" |
                  Sort-Object LastWriteTime -Descending |
                  Select-Object -First 1

    $CSV_FILES = Get-ChildItem -Path $PROFILE_DIR -Filter "perf_gen_model_full_noun_dbnum_7997_*.csv" |
                 Sort-Object LastWriteTime -Descending |
                 Select-Object -First 1

    if ($JSON_FILES) {
        Write-Host "JSON 报告: $($JSON_FILES.FullName)" -ForegroundColor Green
        Write-Host ""
        Write-Host "报告内容预览:" -ForegroundColor Yellow
        Get-Content $JSON_FILES.FullName -Encoding UTF8 | ConvertFrom-Json |
            Select-Object label, total_ms, @{Name='stages_count';Expression={$_.stages.Count}} |
            Format-List

        Write-Host "主要阶段耗时:" -ForegroundColor Yellow
        $report = Get-Content $JSON_FILES.FullName -Encoding UTF8 | ConvertFrom-Json
        $report.stages | Select-Object name, @{Name='duration_ms';Expression={[math]::Round($_.duration_ms,2)}}, @{Name='percentage';Expression={[math]::Round($_.percentage,1)}} | Format-Table -AutoSize
    }

    if ($CSV_FILES) {
        Write-Host ""
        Write-Host "CSV 报告: $($CSV_FILES.FullName)" -ForegroundColor Green
    }
} else {
    Write-Host "警告: 未找到性能报告目录: $PROFILE_DIR" -ForegroundColor Yellow
}

Write-Host ""
Write-Host "运行日志: $LOG_FILE" -ForegroundColor Green
Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "测试完成！" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
