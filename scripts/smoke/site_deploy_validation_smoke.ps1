<#
.SYNOPSIS
  Admin managed-site deployment validation smoke.

.DESCRIPTION
  Requires a running web_server and a managed site id. It calls
  POST /api/admin/sites/{id}/deploy-validation, prints the grouped validation
  result, and fails when blocking checks exist unless -AllowBlocking is set.

.EXAMPLE
  pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/smoke/site_deploy_validation_smoke.ps1 `
    -BaseUrl http://127.0.0.1:3304 `
    -SiteId quicktest-250160-8080 `
    -AdminUser admin `
    -AdminPass $env:ADMIN_PASS

.EXAMPLE
  pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/smoke/site_deploy_validation_smoke.ps1 `
    -BaseUrl http://127.0.0.1:3304 `
    -SiteId quicktest-250160-8080 `
    -Token $env:ADMIN_TOKEN
#>

[CmdletBinding()]
param(
    [string]$BaseUrl = "http://127.0.0.1:3304",
    [Parameter(Mandatory = $true)]
    [string]$SiteId,
    [string]$AdminUser = $env:ADMIN_USER,
    [string]$AdminPass = $env:ADMIN_PASS,
    [string]$Token = "",
    [switch]$AllowBlocking
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

function Warn {
    param([string]$Message)
    Write-Host "[WARN] $Message" -ForegroundColor Yellow
}

function Fail-Line {
    param([string]$Message)
    Write-Host "[FAIL] $Message" -ForegroundColor Red
}

function Invoke-AdminApi {
    param(
        [ValidateSet("GET", "POST")]
        [string]$Method,
        [string]$Path,
        $Body = $null,
        [int]$TimeoutSec = 180
    )
    $headers = @{ "Content-Type" = "application/json" }
    if ($script:Token) { $headers["Authorization"] = "Bearer $script:Token" }
    $params = @{
        Uri = "$BaseUrl$Path"
        Method = $Method
        Headers = $headers
        UseBasicParsing = $true
        TimeoutSec = $TimeoutSec
        SkipHttpErrorCheck = $true
    }
    if ($null -ne $Body) {
        $params.Body = ($Body | ConvertTo-Json -Depth 20 -Compress)
    }
    $response = Invoke-WebRequest @params
    $json = $null
    if ($response.Content) {
        try {
            $json = $response.Content | ConvertFrom-Json
        } catch {
            $json = [pscustomobject]@{ raw = $response.Content }
        }
    }
    return [pscustomobject]@{ Status = [int]$response.StatusCode; Body = $json; Raw = $response.Content }
}

function Require-Success {
    param($Response, [string]$Action)
    if ($Response.Status -ge 200 -and $Response.Status -lt 300 -and $Response.Body.success -ne $false) {
        if ($null -ne $Response.Body.data) { return $Response.Body.data }
        return $Response.Body
    }
    $message = if ($Response.Body -and $Response.Body.message) { $Response.Body.message } else { $Response.Raw }
    throw "$Action failed: status=$($Response.Status) message=$message"
}

function Group-Key {
    param([string]$Key)
    if ($Key -like "service_*" -or $Key -eq "web_status" -or $Key -eq "site_identity") { return "服务健康" }
    if ($Key -like "viewer_*" -or $Key -eq "viewer") { return "Viewer 入口" }
    if ($Key -like "http_parquet_*" -or $Key -like "parquet_manifest_*" -or $Key -like "parquet_instances_*" -or $Key -like "parquet_geo_instances_*" -or $Key -like "parquet_transforms_*" -or $Key -like "parquet_aabb_*") { return "Parquet 文件" }
    if ($Key -like "parquet_*") { return "数据一致性" }
    if ($Key -like "mesh_*" -or $Key -like "http_mesh_*") { return "模型资源" }
    if ($Key -like "api_e3d_*") { return "关键 API" }
    return "其他检查"
}

function Print-Check {
    param($Check)
    $status = [string]$Check.status
    $prefix = if ($status -eq "blocking") { "[BLOCK]" } elseif ($status -eq "warning") { "[WARN] " } else { "[OK]   " }
    $line = "$prefix $($Check.label): $($Check.message)"
    if ($status -eq "blocking") {
        Fail-Line $line
    } elseif ($status -eq "warning") {
        Warn $line
    } else {
        Write-Host $line -ForegroundColor Green
    }
    if ($Check.url) { Write-Host "       url: $($Check.url)" -ForegroundColor DarkGray }
    if ($Check.detail) { Write-Host "    detail: $($Check.detail)" -ForegroundColor DarkGray }
}

$script:Token = if ($PSBoundParameters.ContainsKey("Token")) { $Token } else { "" }
if (-not $script:Token -and (-not $AdminUser -or -not $AdminPass) -and $env:ADMIN_TOKEN) {
    $script:Token = $env:ADMIN_TOKEN
}

if (-not $script:Token) {
    if (-not $AdminUser -or -not $AdminPass) {
        throw "ADMIN_TOKEN or ADMIN_USER / ADMIN_PASS must be set, or pass -Token / -AdminUser / -AdminPass."
    }
    Write-Section "Login"
    $login = Require-Success (Invoke-AdminApi POST "/api/admin/auth/login" @{
        username = $AdminUser
        password = $AdminPass
    }) "login"
    $script:Token = $login.token
    Pass "logged in as $AdminUser"
}

Write-Section "Refresh deploy validation"
$encodedSiteId = [System.Uri]::EscapeDataString($SiteId)
$report = Require-Success (Invoke-AdminApi POST "/api/admin/sites/$encodedSiteId/deploy-validation" $null 300) "deploy-validation"

Write-Host "site_id       : $($report.site_id)"
Write-Host "checked_at    : $($report.checked_at)"
Write-Host "blocking_count: $($report.blocking_count)"
Write-Host "warning_count : $($report.warning_count)"
Write-Host "checks        : $(@($report.checks).Count)"

$groups = @($report.checks) | Group-Object { Group-Key $_.key }
foreach ($group in $groups) {
    Write-Section $group.Name
    foreach ($check in $group.Group) {
        Print-Check $check
    }
}

if ([int]$report.blocking_count -gt 0) {
    if ($AllowBlocking) {
        Warn "deploy validation has blocking checks, but -AllowBlocking was set"
    } else {
        throw "deploy validation failed: blocking_count=$($report.blocking_count)"
    }
} else {
    Pass "deploy validation passed without blocking checks"
}
