<#
.SYNOPSIS
  Direct sidecar smoke for a temporary preview db_index.

.DESCRIPTION
  Starts `aios-database serve`, calls `/parse/preview-plan` to validate preview
  facts, then calls `/db-index/rebuild` with an index path under:

    runtime/preview-index/<inputs_hash>/db_index.sqlite

  This verifies the Phase 3 boundary that preview indexes can be written outside
  `runtime/admin_sites/<site_id>/db_index.sqlite`, so preview work does not
  pollute the formal site runtime index.

.EXAMPLE
  pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/smoke/sidecar_preview_index_smoke.ps1
#>

[CmdletBinding()]
param(
    [string]$AiosDatabaseBin = "",
    [string]$BindHost = "127.0.0.1",
    [int]$HttpPort = 0,
    [string]$SiteKey = "",
    [string]$Token = "",
    [string]$RuntimeDir = "",
    [string]$ProjectName = "AvevaPlantSample",
    [string]$ProjectPath = "D:/AVEVA/Projects/E3D2.1/AvevaPlantSample",
    [string]$PreviewIndexRootPath = "D:/AVEVA/Projects/E3D2.1/AvevaPlantSample/aps000/aps250160_0001",
    [string]$PreviewIndexRootName = "preview-root",
    [string]$InputsHash = "",
    [int[]]$ManualDbNums = @(250160),
    [switch]$KeepSidecar
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
Set-Location $repoRoot

function Write-Section {
    param([string]$Title)
    Write-Host ""
    Write-Host "== $Title ==" -ForegroundColor Cyan
}

function Pass {
    param([string]$Message)
    Write-Host "[PASS] $Message" -ForegroundColor Green
}

function Info {
    param([string]$Message)
    Write-Host "[INFO] $Message" -ForegroundColor Gray
}

function Get-FreeTcpPort {
    $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Parse("127.0.0.1"), 0)
    $listener.Start()
    try {
        return [int]$listener.LocalEndpoint.Port
    } finally {
        $listener.Stop()
    }
}

function Resolve-AiosDatabaseBin {
    param([string]$Provided)
    if ($Provided) {
        if (-not (Test-Path $Provided)) { throw "AiosDatabaseBin not found: $Provided" }
        return (Resolve-Path $Provided).Path
    }
    $name = if ($IsWindows) { "aios-database.exe" } else { "aios-database" }
    $candidates = @(
        (Join-Path $repoRoot "target/debug/$name"),
        (Join-Path $repoRoot "target/release/$name")
    )
    foreach ($candidate in $candidates) {
        if (Test-Path $candidate) { return (Resolve-Path $candidate).Path }
    }
    throw "aios-database binary not found. Build it first or pass -AiosDatabaseBin."
}

function Invoke-SidecarJson {
    param(
        [ValidateSet("GET", "POST")]
        [string]$Method,
        [string]$Path,
        $Body = $null,
        [int]$TimeoutSec = 60
    )
    $params = @{
        Uri = "$script:BaseUrl$Path"
        Method = $Method
        Headers = @{ Authorization = "Bearer $script:Token" }
        TimeoutSec = $TimeoutSec
        UseBasicParsing = $true
        SkipHttpErrorCheck = $true
    }
    if ($null -ne $Body) {
        $params.Headers["Content-Type"] = "application/json"
        $params.Body = ($Body | ConvertTo-Json -Depth 20 -Compress)
    }
    $response = Invoke-WebRequest @params
    $json = $null
    if ($response.Content) {
        try { $json = $response.Content | ConvertFrom-Json } catch { $json = [pscustomobject]@{ raw = $response.Content } }
    }
    return [pscustomobject]@{ Status = [int]$response.StatusCode; Body = $json; Raw = $response.Content }
}

function Require-Success {
    param($Response, [string]$Action)
    if ($Response.Status -ge 200 -and $Response.Status -lt 300 -and $Response.Body.success -ne $false) {
        return $Response.Body.data
    }
    $message = if ($Response.Body -and $Response.Body.message) { $Response.Body.message } else { $Response.Raw }
    throw "$Action failed: status=$($Response.Status) message=$message"
}

function Get-Sha256Hex {
    param([string]$Text)
    $bytes = [Text.Encoding]::UTF8.GetBytes($Text)
    $hash = [System.Security.Cryptography.SHA256]::HashData($bytes)
    return (($hash | ForEach-Object { $_.ToString("x2") }) -join "")
}

if (-not (Test-Path $ProjectPath)) { throw "ProjectPath not found: $ProjectPath" }
if (-not (Test-Path $PreviewIndexRootPath)) { throw "PreviewIndexRootPath not found: $PreviewIndexRootPath" }
if (-not $SiteKey) { $SiteKey = "preview-index-$([Guid]::NewGuid().ToString('N').Substring(0, 8))" }
if (-not $Token) { $Token = [Guid]::NewGuid().ToString("N") }
if ($HttpPort -le 0) { $HttpPort = Get-FreeTcpPort }
if (-not $RuntimeDir) { $RuntimeDir = Join-Path $repoRoot "runtime/smoke/sidecar-$SiteKey" }

$script:Token = $Token
$script:BaseUrl = "http://${BindHost}:${HttpPort}"
$bin = Resolve-AiosDatabaseBin $AiosDatabaseBin
$stdoutLog = Join-Path $RuntimeDir "sidecar.stdout.log"
$stderrLog = Join-Path $RuntimeDir "sidecar.stderr.log"
New-Item -ItemType Directory -Force -Path $RuntimeDir | Out-Null

$sidecar = $null
try {
    Write-Section "Start sidecar"
    $args = @(
        "serve",
        "--site-key", $SiteKey,
        "--bind-host", $BindHost,
        "--http-port", [string]$HttpPort,
        "--runtime-dir", $RuntimeDir,
        "--token", $Token
    )
    Info "$bin $($args -join ' ')"
    $sidecar = Start-Process -FilePath $bin -ArgumentList $args -WorkingDirectory $repoRoot `
        -RedirectStandardOutput $stdoutLog -RedirectStandardError $stderrLog -PassThru

    Write-Section "Health"
    $healthy = $false
    for ($i = 0; $i -lt 40; $i++) {
        try {
            $health = Invoke-SidecarJson -Method GET -Path "/health" -TimeoutSec 3
            if ($health.Status -ge 200 -and $health.Status -lt 300) {
                $healthy = $true
                break
            }
        } catch { }
        Start-Sleep -Milliseconds 250
    }
    if (-not $healthy) {
        throw "sidecar health check failed. stderr: $(Get-Content $stderrLog -ErrorAction SilentlyContinue | Select-Object -Last 20)"
    }
    Pass "health OK at $script:BaseUrl"

    Write-Section "Preview plan"
    $previewBody = [ordered]@{
        project_name = $ProjectName
        project_path = (Resolve-Path $ProjectPath).Path
        manual_db_nums = @($ManualDbNums)
        manual_db_files = @()
        parse_db_types = @()
        force_rebuild_system_db = $false
        auto_parse_related_dbnums = $true
    }
    $previewJson = ($previewBody | ConvertTo-Json -Depth 20 -Compress)
    $inputsHash = if ($InputsHash) { $InputsHash } else { Get-Sha256Hex $previewJson }
    $plan = Require-Success (Invoke-SidecarJson -Method POST -Path "/parse/preview-plan" -Body $previewBody -TimeoutSec 300) "preview-plan"
    $entries = @($plan.entries)
    if ($entries.Count -eq 0) { throw "preview plan entries is empty" }
    Pass "preview plan OK entries=$($entries.Count) inputs_hash=$($inputsHash.Substring(0, 12))"

    Write-Section "Preview db-index"
    $previewIndexDir = Join-Path $repoRoot "runtime/preview-index/$inputsHash"
    $previewIndexPath = Join-Path $previewIndexDir "db_index.sqlite"
    New-Item -ItemType Directory -Force -Path $previewIndexDir | Out-Null
    $adminSitesRoot = (Join-Path $repoRoot "runtime/admin_sites")
    $resolvedIndexPath = [System.IO.Path]::GetFullPath($previewIndexPath)
    $resolvedAdminRoot = [System.IO.Path]::GetFullPath($adminSitesRoot)
    if ($resolvedIndexPath.StartsWith($resolvedAdminRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "preview index path must not be under runtime/admin_sites: $resolvedIndexPath"
    }

    $indexBody = @{
        roots = @(@{
            name = $PreviewIndexRootName
            path = (Resolve-Path $PreviewIndexRootPath).Path
        })
        index_path = $resolvedIndexPath
        force = $true
        manual_db_nums = @($ManualDbNums)
    }
    $summary = Require-Success (Invoke-SidecarJson -Method POST -Path "/db-index/rebuild" -Body $indexBody -TimeoutSec 900) "preview db-index rebuild"
    if (-not (Test-Path $resolvedIndexPath)) {
        throw "preview db_index.sqlite was not created: $resolvedIndexPath"
    }
    if ([int]$summary.db_files -le 0) {
        throw "preview db-index did not scan any db files"
    }
    if ([int]$summary.errors -ne 0) {
        throw "preview db-index reported errors=$($summary.errors)"
    }
    Pass "preview db-index OK path=$resolvedIndexPath db_files=$($summary.db_files) ref0_total=$($summary.ref0_total) errors=$($summary.errors)"

    Write-Section "Done"
    Pass "sidecar preview index smoke passed"
} finally {
    if ($sidecar -and -not $sidecar.HasExited -and -not $KeepSidecar) {
        Stop-Process -Id $sidecar.Id -Force -ErrorAction SilentlyContinue
        Pass "sidecar stopped"
    } elseif ($sidecar -and -not $sidecar.HasExited) {
        Info "sidecar kept alive: pid=$($sidecar.Id) base=$script:BaseUrl token=$script:Token"
    }
}
