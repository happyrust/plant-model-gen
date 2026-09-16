<#
.SYNOPSIS
  Direct aios-database sidecar smoke for parse preview facts.

.DESCRIPTION
  Starts `aios-database serve`, calls `/parse/preview-plan`, and verifies the
  Phase 2 parse facts contract:
    - included_db_files stays populated for the selected input
    - entries exists and matches included_db_files by file_name
    - each entry has source and priority
    - dbnum/db_type are present when the sidecar can read DB headers
    - warnings is present

.EXAMPLE
  pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/smoke/sidecar_preview_facts_smoke.ps1

.EXAMPLE
  pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/smoke/sidecar_preview_facts_smoke.ps1 `
    -ProjectName "aps250160_0001" `
    -ProjectPath "D:/AVEVA/Projects/E3D2.1/AvevaPlantSample/aps000/aps250160_0001" `
    -ManualDbNums 250160
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
    [int[]]$ManualDbNums = @(250160),
    [string[]]$ParseDbTypes = @(),
    [switch]$AutoParseRelatedDbnums,
    [switch]$AllowEmptyIncluded,
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

function Assert-ArrayProperty {
    param($Object, [string]$Name)
    $prop = $Object.PSObject.Properties[$Name]
    if ($null -eq $prop) { throw "preview response missing '$Name'" }
    return @($prop.Value)
}

if (-not (Test-Path $ProjectPath)) {
    throw "ProjectPath not found: $ProjectPath"
}
if (-not $SiteKey) { $SiteKey = "preview-$([Guid]::NewGuid().ToString('N').Substring(0, 8))" }
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

    Write-Section "Preview facts"
    $body = @{
        project_name = $ProjectName
        project_path = (Resolve-Path $ProjectPath).Path
        manual_db_nums = @($ManualDbNums)
        manual_db_files = @()
        parse_db_types = @($ParseDbTypes)
        force_rebuild_system_db = $false
        auto_parse_related_dbnums = [bool]$AutoParseRelatedDbnums
    }
    $preview = Invoke-SidecarJson -Method POST -Path "/parse/preview-plan" -Body $body -TimeoutSec 300
    $plan = Require-Success $preview "preview-plan"

    $included = Assert-ArrayProperty $plan "included_db_files"
    $entries = Assert-ArrayProperty $plan "entries"
    [void](Assert-ArrayProperty $plan "warnings")
    [void](Assert-ArrayProperty $plan "auto_related_db_files")

    if (-not $AllowEmptyIncluded -and $included.Count -eq 0) {
        throw "included_db_files is empty; pass -AllowEmptyIncluded for full-parse smoke"
    }
    if ($entries.Count -ne $included.Count) {
        throw "entries count ($($entries.Count)) does not match included_db_files count ($($included.Count))"
    }

    $includedSet = @{}
    foreach ($fileName in $included) { $includedSet[[string]$fileName] = $true }
    foreach ($entry in $entries) {
        if (-not $entry.file_name) { throw "entry missing file_name: $($entry | ConvertTo-Json -Compress)" }
        if (-not $includedSet.ContainsKey([string]$entry.file_name)) {
            throw "entry file_name not present in included_db_files: $($entry.file_name)"
        }
        if (-not $entry.source) { throw "entry missing source: $($entry.file_name)" }
        if ($null -eq $entry.priority) { throw "entry missing priority: $($entry.file_name)" }
        if ($null -eq $entry.dbnum) { throw "entry missing dbnum: $($entry.file_name)" }
        if (-not $entry.db_type) { throw "entry missing db_type: $($entry.file_name)" }
    }

    if ($ManualDbNums.Count -gt 0) {
        $manualEntries = @($entries | Where-Object { $_.source -eq "manual_db_num" })
        if ($manualEntries.Count -eq 0) {
            throw "expected at least one manual_db_num entry"
        }
    }

    Pass "preview facts OK included=$($included.Count) entries=$($entries.Count) warnings=$(@($plan.warnings).Count)"
    Write-Section "Done"
    Pass "sidecar preview facts smoke passed"
} finally {
    if ($sidecar -and -not $sidecar.HasExited -and -not $KeepSidecar) {
        Stop-Process -Id $sidecar.Id -Force -ErrorAction SilentlyContinue
        Pass "sidecar stopped"
    } elseif ($sidecar -and -not $sidecar.HasExited) {
        Info "sidecar kept alive: pid=$($sidecar.Id) base=$script:BaseUrl token=$script:Token"
    }
}
