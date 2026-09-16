<#
.SYNOPSIS
    一键部署 plant-model-gen / plant3d-web 到生产服务器。

.DESCRIPTION
    本脚本是 deploy_to_production.py 的 PowerShell wrapper：
        1. 检查工作区状态 + 当前分支匹配
        2. push 当前分支到 origin（GitHub），自动触发 GitHub Actions Deploy workflow
        3. SSH 监听远端 /root/web_server / /var/www/plant3d-web/ 时间戳更新
        4. CI 完成后跑 backend sync(query) 与前端 HEAD 健康检查
        5. 输出部署 summary（JSON）

.PARAMETER Repo
    目标仓库：plant-model-gen / plant3d-web / all（默认 all）

.PARAMETER NoPush
    仅检查状态与远端时间戳，不 push（dry-run）

.PARAMETER SkipVerify
    push 后不等远端产物更新（仅验证 push 成功）

.PARAMETER Timeout
    等待 CI 完成的最大秒数（默认 1800）

.EXAMPLE
    .\scripts\deploy\deploy.ps1
    部署两个仓库（all），等待 CI 完成验证

.EXAMPLE
    .\scripts\deploy\deploy.ps1 -Repo plant3d-web
    仅部署 plant3d-web

.EXAMPLE
    .\scripts\deploy\deploy.ps1 -NoPush
    dry-run：只检查状态、记录远端 mtime，不 push 也不等待

.NOTES
    需要先安装 paramiko：pip install paramiko
    SSH 密码默认从 setup-deploy-server.sh 一致；可用环境变量 DEPLOY_PASSWORD 覆盖
#>

[CmdletBinding()]
param(
    [ValidateSet("all", "plant-model-gen", "plant3d-web")]
    [string]$Repo = "all",

    [switch]$NoPush,
    [switch]$SkipVerify,
    [switch]$StrictBranch,
    [switch]$NoGh,
    [int]$Timeout = 1800
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$pyScript = Join-Path $scriptDir "deploy_to_production.py"

if (-not (Test-Path $pyScript)) {
    Write-Error "未找到 $pyScript"
    exit 1
}

# 检查 python 可用
$python = Get-Command python -ErrorAction SilentlyContinue
if (-not $python) {
    $python = Get-Command python3 -ErrorAction SilentlyContinue
}
if (-not $python) {
    Write-Error "未找到 python / python3，请先安装"
    exit 1
}

# 组装参数
$argsList = @($pyScript)
$argsList += "--repo"; $argsList += $Repo
if ($NoPush) { $argsList += "--no-push" }
if ($SkipVerify) { $argsList += "--skip-verify" }
if ($StrictBranch) { $argsList += "--strict-branch" }
if ($NoGh) { $argsList += "--no-gh" }
$argsList += "--timeout"; $argsList += "$Timeout"

Write-Host "========================================" -ForegroundColor Magenta
Write-Host "  plant-code 一键部署 → 123.57.182.243" -ForegroundColor Magenta
Write-Host "========================================" -ForegroundColor Magenta
Write-Host "目标仓库: $Repo" -ForegroundColor Cyan
Write-Host "no-push : $NoPush" -ForegroundColor Cyan
Write-Host "skip-verify: $SkipVerify" -ForegroundColor Cyan
Write-Host "timeout : ${Timeout}s" -ForegroundColor Cyan
Write-Host ""

& $python.Path @argsList
exit $LASTEXITCODE
