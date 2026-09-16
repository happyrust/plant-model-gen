<#
.SYNOPSIS
  BRAN 按需解析 + 模型生成的最短调试环：内存 SurrealDB + refno 级 CATA 闭包。

.DESCRIPTION
  串起四步，全程不碰任何已解析好的大 RocksDB 库：

    0) 起一个 `surreal start --bind 127.0.0.1:<Port> memory` 的内存实例（退出即清空）
    1) gen-cata-closure --seed-refnos <Refno>   → output/<项目>/scene_tree/cata_closure.json
    2) AIOS_CATA_CLOSURE_MODE=manifest + 空跑   → 按 manifest 只解析闭包命中的 CATA refno
    3) --debug-model <Refno> --regen-model      → 只重生成该 BRAN 子树的几何

  最后调 scripts/verify_sync_to_db_24381_145018.ps1 做 SQL 计数校验。

  内存库是易失的：进程一停数据全没。这正是要的——每轮都是干净库，结果可重现。
  想省掉解析那一步反复调生成，用 -KeepServer 留着库，下一轮加 -ReuseServer -GenOnly。

.EXAMPLE
  powershell -File scripts/debug_bran_mem.ps1

.EXAMPLE
  # 解析很吃 CPU，debug 二进制会慢一个数量级；迭代时用优化过的 production profile
  powershell -File scripts/debug_bran_mem.ps1 -BuildProfile production -Build

.EXAMPLE
  # 保留内存库，随后只重跑生成（改完生成代码重编译后）
  powershell -File scripts/debug_bran_mem.ps1 -KeepServer
  powershell -File scripts/debug_bran_mem.ps1 -ReuseServer -GenOnly

.EXAMPLE
  # 换一个参考号（注意 -Span/-ExpectedTubi 是给 24381_145018 调的，换号要一并给）
  powershell -File scripts/debug_bran_mem.ps1 -Refno 24381/145569 -ExpectedTubi -1 -Span 1
#>
param(
    # 支持 24381_145018 或 24381/145018
    [string]$Refno = "24381_145018",

    # 内存 SurrealDB 监听端口（避开常被占用的 8000/8009/8020/8031/8032）
    [int]$Port = 8041,

    # 基础配置（无扩展名）；端口与 -Port 不一致时会派生一份副本到 runtime/bran-mem/
    [string]$BaseConfig = "db_options/DbOption-bran-mem",

    # 构建 profile。debug 未优化，PDMS 属性解析会慢一个数量级（实测单 chunk 300s+），
    # 真要快速迭代解析用 production：它落在 target/production/，不会覆盖 release/
    # ——共享的 CARGO_TARGET_DIR 里 release/aios-database.exe 是别的仓库（rs-plant3-d）的产物。
    [ValidateSet('debug', 'release', 'production')]
    [string]$BuildProfile = "debug",

    # 覆盖 CARGO_TARGET_DIR（想彻底和其它仓库隔离时用）
    [string]$TargetDir = "",

    # 二进制缺失时自动 cargo build
    [switch]$Build,

    # 复用已在 -Port 上监听的实例（不自己起、也不负责停）
    [switch]$ReuseServer,

    # 跑完不停内存库，留给下一轮 -ReuseServer
    [switch]$KeepServer,

    # 整库解析种子所在的 DESI 库（老口径）。默认走按需：闭包把该 BRAN 的设计子树
    # + owner 祖先链写进 manifest，解析期据此裁剪 DESI，不再整库解 7997。
    [switch]$FullDesi,

    [switch]$SkipClosure,
    [switch]$SkipParse,
    # 只跑生成：等价于 -SkipClosure -SkipParse，通常配 -ReuseServer
    [switch]$GenOnly,
    [switch]$SkipVerify,

    # 解析期 SurrealDB 写入 worker 数。内存引擎是乐观 MVCC，默认的 16 路并发批量写
    # 会稳定报 Transaction conflict，所以这里默认串行；RocksDB 站点不受影响。
    [int]$WriteWorkers = 1,

    # 透传给校验脚本
    [int]$Span = 18,
    [int]$ExpectedTubi = 11,

    # 单步超时（秒）
    [int]$TimeoutSeconds = 3600
)

$ErrorActionPreference = "Stop"

# ── 定位仓库根：脚本在 scripts/ 下，配置与 output/ 全是相对路径，必须切过去 ──────
$RepoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $RepoRoot

$RunDir = Join-Path $RepoRoot "runtime/bran-mem"
New-Item -ItemType Directory -Force -Path $RunDir | Out-Null

$script:ServerPid = $null
$script:StartedServer = $false
$script:Timings = [ordered]@{}

function Write-Step([string]$Text) {
    Write-Host ""
    Write-Host "─── $Text " -ForegroundColor Cyan -NoNewline
    Write-Host ("─" * [Math]::Max(0, 60 - $Text.Length)) -ForegroundColor DarkCyan
}

function Get-TargetRoot {
    if ($TargetDir) { return $TargetDir }
    if ($env:CARGO_TARGET_DIR) { return $env:CARGO_TARGET_DIR }
    return (Join-Path $RepoRoot "target")
}

function Get-CargoBuildArgs {
    $cargoArgs = @("build", "--bin", "aios-database")
    switch ($BuildProfile) {
        "release" { $cargoArgs += "--release" }
        "production" { $cargoArgs += @("--profile", "production") }
    }
    return $cargoArgs
}

# 不做跨 profile 回退：共享 target 里同名二进制可能来自别的仓库，静默顶上会很难查。
function Resolve-Binary {
    $targetRoot = Get-TargetRoot
    $bin = Join-Path $targetRoot (Join-Path $BuildProfile "aios-database.exe")
    if (Test-Path $bin) { return $bin }

    if ($Build) {
        Write-Host "⚙️  $bin 不存在，开始构建（profile=$BuildProfile）..." -ForegroundColor Yellow
        $cargoArgs = Get-CargoBuildArgs
        if ($TargetDir) { $env:CARGO_TARGET_DIR = $TargetDir }
        & cargo @cargoArgs
        if ($LASTEXITCODE -ne 0) { throw "cargo build 失败（exit $LASTEXITCODE）" }
        if (-not (Test-Path $bin)) { throw "构建成功但仍找不到 $bin" }
        return $bin
    }

    $have = @("debug", "release", "production") | Where-Object {
        Test-Path (Join-Path $targetRoot (Join-Path $_ "aios-database.exe"))
    }
    $hint = if ($have) { "（$targetRoot 下已有: $($have -join ', ')，但它们可能来自共享该 target 的其它仓库）" } else { "" }
    throw "找不到 $bin $hint`n先构建: $((Get-CargoBuildArgs) -join ' ')，或给本脚本加 -Build"
}

# CARGO_TARGET_DIR 是跨仓库共享的：rs-plant3-d 等仓库同样产出 aios-database.exe 并互相覆盖。
# --diagnose-surreal 不连库，正好当作「二进制属于本仓库 + 新配置能反序列化」的联合自检。
function Assert-BinaryUsable([string]$bin, [string]$cfg) {
    $savedEap = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $out = & $bin -c $cfg --diagnose-surreal 2>&1 | Out-String
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $savedEap
    }
    if ($code -ne 0) {
        throw @"
二进制自检失败（exit $code）: $bin
target 目录 $(Get-TargetRoot) 是跨仓库共享的，其它仓库会产出同名 aios-database.exe 覆盖它。
请在本仓库重新构建: $((Get-CargoBuildArgs) -join ' ')
自检输出:
$out
"@
    }
    $out -split "`r?`n" | Where-Object { $_ -match 'WS 连接目标|连接模式' } | ForEach-Object {
        Write-Host "  $($_.Trim())" -ForegroundColor DarkGray
    }
}

function Resolve-SurrealBinary {
    $cmd = Get-Command surreal -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $fallback = Join-Path $RepoRoot "bin/surreal.exe"
    if (Test-Path $fallback) { return $fallback }
    throw "PATH 里找不到 surreal，也没有 $fallback"
}

function Get-SurrealPortFromToml([string]$configNoExt) {
    $p = "$configNoExt.toml"
    if (-not (Test-Path $p)) { throw "配置文件不存在: $p" }
    $toml = Get-Content -LiteralPath $p -Raw
    # 顶层 surreal_port 是唯一无歧义的键：裸 `port` 在 [web_server] 和 [surrealdb] 两处都有
    $m = [regex]::Match($toml, '^\s*surreal_port\s*=\s*(\d+)\s*$', [Text.RegularExpressions.RegexOptions]::Multiline)
    if ($m.Success) { return [int]$m.Groups[1].Value }
    return -1
}

function New-DerivedConfig([string]$configNoExt, [int]$port) {
    $toml = Get-Content -LiteralPath "$configNoExt.toml" -Raw
    $opt = [Text.RegularExpressions.RegexOptions]::Multiline
    $toml = [regex]::Replace($toml, '^\s*surreal_port\s*=\s*\d+\s*$', "surreal_port = $port", $opt)
    $toml = [regex]::Replace($toml, '^(\s*surreal_bind\s*=\s*")([^":]+):\d+(")', "`${1}`${2}:$port`${3}", $opt)
    # 裸 `port =` 只改 [surrealdb] 段内那一个：顶层 TiDB port 带引号不会命中，
    # 但 [web_server].port 同样是裸整数，不按段切会被一起改掉。
    $idx = $toml.IndexOf("[surrealdb]")
    if ($idx -ge 0) {
        $head = $toml.Substring(0, $idx)
        $tail = [regex]::Replace($toml.Substring($idx), '^\s*port\s*=\s*\d+\s*$', "port = $port", $opt)
        $toml = $head + $tail
    }
    $derivedNoExt = Join-Path $RunDir ("{0}-{1}" -f (Split-Path -Leaf $configNoExt), $port)
    Set-Content -LiteralPath "$derivedNoExt.toml" -Value $toml -Encoding UTF8
    return $derivedNoExt
}

function Test-PortListening([int]$port) {
    $null -ne (Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue)
}

function Wait-SurrealReady([int]$port, [int]$timeoutSec = 30) {
    $deadline = (Get-Date).AddSeconds($timeoutSec)
    while ((Get-Date) -lt $deadline) {
        try {
            $r = Invoke-WebRequest -Uri "http://127.0.0.1:$port/health" -UseBasicParsing -TimeoutSec 3
            if ($r.StatusCode -ge 200 -and $r.StatusCode -lt 300) { return $true }
        } catch {
            Start-Sleep -Milliseconds 300
        }
    }
    return $false
}

# 经 `cmd /c start /b` + 批处理内重定向拉起，而不是 Start-Process -RedirectStandard*：
# 后者起的 surreal 会握住调用方的输出管道，-KeepServer 保活时上层脚本/CI 读不到 EOF 而卡死
# （实测：脚本早已退出，外层管道一直挂着，kill 掉 surreal 才返回）。
function Start-MemoryServer([int]$port) {
    $surreal = Resolve-SurrealBinary
    $out = Join-Path $RunDir "surreal-$port.out.log"
    $err = Join-Path $RunDir "surreal-$port.err.log"
    $launcher = Join-Path $RunDir "start-surreal-$port.cmd"
    @(
        '@echo off',
        ('"{0}" start --user root --pass root --bind 127.0.0.1:{1} memory > "{2}" 2> "{3}"' -f $surreal, $port, $out, $err)
    ) | Set-Content -LiteralPath $launcher -Encoding ASCII

    Write-Host "🚀 起内存 SurrealDB: 127.0.0.1:$port (memory)" -ForegroundColor Green
    # '""' 是 start 的窗口标题占位（不能传空串，Start-Process 会拒绝）
    Start-Process -FilePath "cmd.exe" `
        -ArgumentList @("/c", "start", "/b", '""', "`"$launcher`"") `
        -WindowStyle Hidden | Out-Null

    if (-not (Wait-SurrealReady -port $port)) {
        throw "内存 SurrealDB 30s 内未就绪，看 $err"
    }
    $serverPid = (Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue |
        Select-Object -First 1).OwningProcess
    Write-Host "   PID $serverPid 已就绪（日志: $out）" -ForegroundColor DarkGray
    return $serverPid
}

function Invoke-Cli {
    param(
        [string]$Name,
        [string]$Bin,
        [string[]]$CliArgs,
        [hashtable]$ExtraEnv = @{}
    )
    $log = Join-Path $RunDir "$Name.log"
    Write-Step "$Name"
    Write-Host "  $Bin $($CliArgs -join ' ')" -ForegroundColor DarkGray
    if ($ExtraEnv.Count -gt 0) {
        Write-Host "  env: $(($ExtraEnv.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join ' ')" -ForegroundColor DarkGray
    }

    $saved = @{}
    foreach ($k in $ExtraEnv.Keys) {
        $saved[$k] = [Environment]::GetEnvironmentVariable($k)
        [Environment]::SetEnvironmentVariable($k, $ExtraEnv[$k])
    }

    $sw = [Diagnostics.Stopwatch]::StartNew()
    # 合并 stderr 时必须放宽 ErrorActionPreference：否则原生命令往 stderr 写一行就被当成终止性错误
    $savedEap = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        & $Bin @CliArgs 2>&1 | Tee-Object -FilePath $log
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $savedEap
        $sw.Stop()
        foreach ($k in $saved.Keys) { [Environment]::SetEnvironmentVariable($k, $saved[$k]) }
    }

    $script:Timings[$Name] = $sw.Elapsed
    if ($code -ne 0) { throw "$Name 失败（exit $code），日志: $log" }
    Write-Host "  ✅ $Name 用时 $([Math]::Round($sw.Elapsed.TotalSeconds, 1))s" -ForegroundColor Green
}

# ── 主流程 ────────────────────────────────────────────────────────────────────
try {
    if ($GenOnly) { $SkipClosure = $true; $SkipParse = $true }
    if ($GenOnly -and -not $ReuseServer) {
        Write-Host "⚠️  -GenOnly 配的是全新内存库：库里没有 PE/CATA 数据，生成必然是空的。通常应加 -ReuseServer" -ForegroundColor Yellow
    }

    $bin = Resolve-Binary
    $basePort = Get-SurrealPortFromToml $BaseConfig
    $cfg = if ($basePort -eq $Port) { $BaseConfig } else { New-DerivedConfig $BaseConfig $Port }

    Write-Host ""
    Write-Host "BRAN 内存调试环" -ForegroundColor Cyan
    Write-Host "  refno   : $Refno"
    Write-Host "  二进制  : $bin"
    Write-Host "  配置    : $cfg$(if ($cfg -ne $BaseConfig) { "  (由 $BaseConfig 派生，端口 $basePort → $Port)" })"
    Write-Host "  端口    : $Port"

    Write-Step "0-自检（二进制归属 + 配置反序列化）"
    Assert-BinaryUsable $bin $cfg
    Write-Host "  ✅ 自检通过" -ForegroundColor Green

    if ($ReuseServer) {
        if (-not (Test-PortListening $Port)) { throw "-ReuseServer 但 $Port 上没有监听的实例" }
        Write-Host "  服务    : 复用 $Port 上的现有实例" -ForegroundColor DarkGray
    } else {
        if (Test-PortListening $Port) {
            throw "$Port 已被占用。换 -Port，或加 -ReuseServer 复用它"
        }
        $script:ServerPid = Start-MemoryServer $Port
        $script:StartedServer = $true
    }

    $total = [Diagnostics.Stopwatch]::StartNew()

    if (-not $SkipClosure) {
        $closureArgs = @("-c", $cfg, "gen-cata-closure", "--seed-refnos", $Refno)
        if (-not $FullDesi) { $closureArgs += "--include-design-subtree" }
        Invoke-Cli -Name "1-closure" -Bin $bin -CliArgs $closureArgs
    }

    # 解析与生成都开 manifest：生成期命中未解析的 CATA refno 时，
    # resolve.rs 的 try_lazy_cata_fallback 会做一次小闭包补解析（仅 manifest 模式下生效）
    $closureEnv = @{ "AIOS_CATA_CLOSURE_MODE" = "manifest" }

    if (-not $SkipParse) {
        $parseEnv = $closureEnv.Clone()
        $parseEnv["AIOS_SYNC_WRITE_WORKERS"] = "$WriteWorkers"
        Invoke-Cli -Name "2-parse" -Bin $bin -CliArgs @("-c", $cfg) -ExtraEnv $parseEnv
    }

    Invoke-Cli -Name "3-generate" -Bin $bin `
        -CliArgs @("-c", $cfg, "--debug-model", $Refno, "--regen-model") -ExtraEnv $closureEnv

    $total.Stop()

    if (-not $SkipVerify) {
        Write-Step "4-verify"
        $global:LASTEXITCODE = 0
        & (Join-Path $PSScriptRoot "verify_sync_to_db_24381_145018.ps1") `
            -Port $Port -Ns "1516" -Db "AvevaMarineSample" `
            -Refno $Refno -Span $Span -ExpectedTubi $ExpectedTubi -FailOnMismatch
        if ($LASTEXITCODE -ne 0) { throw "校验未通过（exit $LASTEXITCODE）" }
    }

    Write-Host ""
    Write-Host "========== 耗时汇总 ==========" -ForegroundColor Cyan
    foreach ($k in $script:Timings.Keys) {
        Write-Host ("  {0,-12} {1,8:N1}s" -f $k, $script:Timings[$k].TotalSeconds)
    }
    Write-Host ("  {0,-12} {1,8:N1}s" -f "合计", $total.Elapsed.TotalSeconds) -ForegroundColor Green
} finally {
    if ($script:StartedServer -and $script:ServerPid) {
        if ($KeepServer) {
            Write-Host ""
            Write-Host "🟢 内存库保留在 $Port（PID $($script:ServerPid)）。下一轮：-ReuseServer -GenOnly；用完手动 Stop-Process -Id $($script:ServerPid)" -ForegroundColor Yellow
        } else {
            Write-Host ""
            Write-Host "🧹 停掉内存 SurrealDB（PID $($script:ServerPid)），数据随之清空" -ForegroundColor DarkGray
            Stop-Process -Id $script:ServerPid -Force -ErrorAction SilentlyContinue
        }
    }
    Pop-Location
}
