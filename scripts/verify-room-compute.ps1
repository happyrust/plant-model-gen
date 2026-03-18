param(
    [string]$Fixture = "verification/room_compute_validation.json",
    [string[]]$Keywords = @("-RM", "-ROOM"),
    [uint32[]]$DbNums,
    [string]$RefnoRoot,
    [switch]$GenPanelsMesh,
    [switch]$SkipClean,
    [switch]$SkipCompute,
    [switch]$SkipVerify,
    [string]$ExportOutput,
    [switch]$Release,
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $repoRoot

$fixturePath = if ([System.IO.Path]::IsPathRooted($Fixture)) {
    $Fixture
} else {
    Join-Path $repoRoot $Fixture
}

if (-not (Test-Path $fixturePath)) {
    throw "未找到验证 fixture：$fixturePath"
}

if (-not $SkipCompute -and -not $DbNums -and [string]::IsNullOrWhiteSpace($RefnoRoot)) {
    throw "为避免误跑全量，请至少提供 -DbNums 或 -RefnoRoot；若只校验已有结果，请加 -SkipCompute。"
}

$cargoArgs = @("run")
if ($Release) {
    $cargoArgs += "--release"
}
$cargoArgs += @("--bin", "aios-database", "--features", "ws,sqlite-index,web_server", "--")

function Invoke-RoomCli {
    param(
        [string[]]$SubCommand
    )

    $fullArgs = @($cargoArgs + $SubCommand)
    $rendered = ($fullArgs | ForEach-Object {
            if ($_ -match '\s') {
                '"' + $_ + '"'
            } else {
                $_
            }
        }) -join ' '

    Write-Host ""
    Write-Host ">>> cargo $rendered" -ForegroundColor Cyan

    if ($DryRun) {
        return
    }

    & cargo @fullArgs
    if ($LASTEXITCODE -ne 0) {
        throw "命令执行失败：cargo $rendered"
    }
}

Write-Host "🧪 房间计算 CLI 验证" -ForegroundColor Green
Write-Host "=========================================="
Write-Host "仓库目录: $repoRoot"
Write-Host "Fixture : $fixturePath"
if ($DbNums) {
    Write-Host "DBNums  : $($DbNums -join ',')"
}
if ($RefnoRoot) {
    Write-Host "Root    : $RefnoRoot"
}
Write-Host "Keywords: $($Keywords -join ',')"
Write-Host "DryRun  : $DryRun"

if (-not $SkipClean) {
    Invoke-RoomCli -SubCommand @("room", "clean")
}

if (-not $SkipCompute) {
    $computeArgs = @("room", "compute")

    if ($Keywords -and $Keywords.Count -gt 0) {
        $computeArgs += "--keywords"
        $computeArgs += ($Keywords -join ",")
    }

    if ($DbNums -and $DbNums.Count -gt 0) {
        $computeArgs += "--db-nums"
        $computeArgs += ($DbNums -join ",")
    }

    if (-not [string]::IsNullOrWhiteSpace($RefnoRoot)) {
        $computeArgs += "--refno-root"
        $computeArgs += $RefnoRoot
    }

    if ($GenPanelsMesh) {
        $computeArgs += "--gen-panels-mesh"
    }

    Invoke-RoomCli -SubCommand $computeArgs
}

if (-not $SkipVerify) {
    Invoke-RoomCli -SubCommand @("room", "verify-json", "--input", $fixturePath)
}

if (-not [string]::IsNullOrWhiteSpace($ExportOutput)) {
    Invoke-RoomCli -SubCommand @("room", "export", "--output", $ExportOutput)
}

Write-Host ""
Write-Host "✅ 房间计算 CLI 验证流程结束" -ForegroundColor Green
