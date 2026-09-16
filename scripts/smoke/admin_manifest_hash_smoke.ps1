<#
.SYNOPSIS
  Admin quick-deploy smoke for parse-plan-manifest inputs_hash.

.DESCRIPTION
  Requires a running web_server and ADMIN_USER / ADMIN_PASS.
  Creates three quick-deploy admin sites:
    1. auto_parse_related_dbnums = false
    2. auto_parse_related_dbnums = false
    3. auto_parse_related_dbnums = true

  Between (1) and (2), it builds a preview db_index with the first manifest's
  inputs_hash. It then reads each site's `parse-plan-manifest.json` and verifies:
    - manifest schema has inputs_hash, entries, warnings
    - manifest carries site runtime db_index metadata tied to inputs_hash
    - matching preview db_index is promoted into the second site's runtime index
    - identical parse inputs produce the same inputs_hash
    - changing auto_parse_related_dbnums changes inputs_hash

.EXAMPLE
  pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/smoke/admin_manifest_hash_smoke.ps1 `
    -BaseUrl http://127.0.0.1:3304
#>

[CmdletBinding()]
param(
    [string]$BaseUrl = "http://127.0.0.1:3304",
    [string]$AdminUser = $env:ADMIN_USER,
    [string]$AdminPass = $env:ADMIN_PASS,
    [string]$ProjectName = "AvevaPlantSample",
    [string]$ProjectPath = "D:/AVEVA/Projects/E3D2.1/AvevaPlantSample",
    [int]$DbNum = 250160,
    [switch]$SkipPreviewIndexBuild,
    [switch]$SkipCleanup
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
Set-Location $repoRoot

if (-not $AdminUser -or -not $AdminPass) {
    throw "ADMIN_USER / ADMIN_PASS must be set, or pass -AdminUser / -AdminPass."
}
if (-not (Test-Path $ProjectPath)) {
    throw "ProjectPath not found: $ProjectPath"
}

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

function Invoke-AdminApi {
    param(
        [ValidateSet("GET", "POST", "DELETE")]
        [string]$Method,
        [string]$Path,
        $Body = $null,
        [int]$TimeoutSec = 120
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

function Get-Manifest {
    param([string]$SiteId)
    $path = Join-Path $repoRoot "runtime/admin_sites/$SiteId/parse-plan-manifest.json"
    if (-not (Test-Path $path)) {
        throw "manifest not found for ${SiteId}: $path"
    }
    return (Get-Content -Raw -Path $path | ConvertFrom-Json)
}

function Assert-ManifestShape {
    param($Manifest, [string]$Label)
    if ($Manifest.schema_version -ne 1) { throw "$Label schema_version=$($Manifest.schema_version), expected 1" }
    if (-not $Manifest.generated_at) { throw "$Label missing generated_at" }
    if (-not $Manifest.inputs_hash) { throw "$Label missing inputs_hash" }
    if (-not $Manifest.sidecar_version) { throw "$Label missing sidecar_version" }
    if ($null -eq $Manifest.db_index) { throw "$Label missing db_index" }
    if ($Manifest.db_index.role -ne "site_runtime") { throw "$Label db_index.role=$($Manifest.db_index.role), expected site_runtime" }
    if (-not $Manifest.db_index.path) { throw "$Label missing db_index.path" }
    if ($Manifest.db_index.inputs_hash -ne $Manifest.inputs_hash) { throw "$Label db_index.inputs_hash does not match manifest inputs_hash" }
    if ($null -eq $Manifest.entries) { throw "$Label missing entries" }
    if ($null -eq $Manifest.warnings) { throw "$Label missing warnings" }
    $entries = @($Manifest.entries)
    if ($entries.Count -eq 0) { throw "$Label entries is empty" }
    foreach ($entry in $entries) {
        if (-not $entry.file_name) { throw "$Label entry missing file_name" }
        if (-not $entry.source) { throw "$Label entry missing source for $($entry.file_name)" }
        if ($null -eq $entry.priority) { throw "$Label entry missing priority for $($entry.file_name)" }
    }
    Pass "$Label manifest shape OK hash=$($Manifest.inputs_hash.Substring(0, 12)) entries=$($entries.Count)"
}

function New-QuickDeployManifest {
    param([bool]$AutoDeps, [string]$Label)
    $payload = @{
        project_name = $ProjectName
        project_path = (Resolve-Path $ProjectPath).Path
        dbnum = $DbNum
        auto_parse_related_dbnums = $AutoDeps
        gen_model = $false
        gen_mesh = $false
        gen_spatial_tree = $false
        start_site = $false
        wait = $false
        pipeline_db_mode = "ws"
    }
    $created = Require-Success (Invoke-AdminApi POST "/api/admin/sites/quick-deploy" $payload) "quick-deploy $Label"
    $siteId = $created.site_id
    if (-not $siteId) { throw "quick-deploy $Label did not return site_id" }
    $script:CreatedSiteIds += $siteId
    $manifest = Get-Manifest $siteId
    Assert-ManifestShape $manifest $Label
    return [pscustomobject]@{ SiteId = $siteId; Manifest = $manifest }
}

$script:Token = ""
$script:CreatedSiteIds = @()
$script:PreviewIndexDirs = @()

try {
    Write-Section "Login"
    $login = Require-Success (Invoke-AdminApi POST "/api/admin/auth/login" @{
        username = $AdminUser
        password = $AdminPass
    }) "login"
    $script:Token = $login.token
    Pass "logged in as $AdminUser"

    Write-Section "Create manifests"
    $first = New-QuickDeployManifest -AutoDeps:$false -Label "same-input-a"

    if (-not $SkipPreviewIndexBuild) {
        Write-Section "Build matching preview index"
        $previewScript = Join-Path $repoRoot "scripts/smoke/sidecar_preview_index_smoke.ps1"
        & pwsh -NoProfile -ExecutionPolicy Bypass -File $previewScript `
            -ProjectName $ProjectName `
            -ProjectPath $ProjectPath `
            -ManualDbNums $DbNum `
            -InputsHash $first.Manifest.inputs_hash
        if ($LASTEXITCODE -ne 0) {
            throw "preview index smoke failed with exit code $LASTEXITCODE"
        }
        $previewPath = Join-Path $repoRoot "runtime/preview-index/$($first.Manifest.inputs_hash)"
        $script:PreviewIndexDirs += $previewPath
        Pass "matching preview index ready: $previewPath"
    }

    $second = New-QuickDeployManifest -AutoDeps:$false -Label "same-input-b"
    if (-not $SkipPreviewIndexBuild) {
        if (-not $second.Manifest.db_index.promoted_from_preview) {
            throw "same-input-b did not promote matching preview db_index"
        }
        if (-not $second.Manifest.db_index.preview_path) {
            throw "same-input-b manifest missing promoted preview_path"
        }
        if (-not (Test-Path $second.Manifest.db_index.path)) {
            throw "same-input-b promoted runtime db_index not found: $($second.Manifest.db_index.path)"
        }
        Pass "matching preview index promoted to $($second.Manifest.db_index.path)"
    }

    $changed = New-QuickDeployManifest -AutoDeps:$true -Label "changed-auto-deps"

    Write-Section "Hash assertions"
    if ($first.Manifest.inputs_hash -ne $second.Manifest.inputs_hash) {
        throw "same parse inputs produced different hashes: $($first.Manifest.inputs_hash) vs $($second.Manifest.inputs_hash)"
    }
    Pass "same parse inputs reuse hash: $($first.Manifest.inputs_hash)"

    if ($first.Manifest.inputs_hash -eq $changed.Manifest.inputs_hash) {
        throw "changed auto_parse_related_dbnums did not change inputs_hash"
    }
    Pass "changed parse input changed hash: $($changed.Manifest.inputs_hash)"

    Write-Section "Done"
    Pass "admin manifest hash smoke passed"
} finally {
    if (-not $SkipCleanup -and $script:Token) {
        foreach ($siteId in $script:CreatedSiteIds) {
            try {
                [void](Require-Success (Invoke-AdminApi DELETE "/api/admin/sites/$siteId") "delete $siteId")
                Pass "deleted $siteId"
            } catch {
                Info "cleanup failed for ${siteId}: $($_.Exception.Message)"
            }
        }
        foreach ($previewDir in $script:PreviewIndexDirs) {
            try {
                if (Test-Path $previewDir) {
                    Remove-Item -Recurse -Force -Path $previewDir
                    Pass "removed preview index $previewDir"
                }
            } catch {
                Info "preview index cleanup failed for ${previewDir}: $($_.Exception.Message)"
            }
        }
    }
}
