param(
    [string]$BaseUrl = "http://127.0.0.1:3209",
    [string]$SiteId = "",
    [string]$ProjectName = "AvevaMarineSample",
    [string]$ProjectPath = "",
    [string]$AssociatedProject = "AvevaMarineSample",
    [int]$ProjectCode = 1516,
    [string]$ManualDbNums = "7997",
    [int]$DbPort = 3339,
    [int]$WebPort = 3340,
    [string]$DbUser = "codex_site_user",
    [string]$DbPassword = "codex-site-pass-123!",
    [int]$DeployTimeoutSec = 1800,
    [int]$ViewerBrowserTimeoutSec = 300,
    [switch]$SkipViewerBrowserCheck,
    [switch]$SkipCleanup
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$AdminUser = $env:ADMIN_USER
$AdminPass = $env:ADMIN_PASS
if (-not $AdminUser -or -not $AdminPass) {
    throw "ADMIN_USER / ADMIN_PASS must be set"
}

function Resolve-ProjectPath {
    param([string]$Name, [string]$Provided)
    if ($Provided -and (Test-Path $Provided)) { return (Resolve-Path $Provided).Path }
    $candidates = @(
        "D:\e3d_models\$Name",
        "D:\work\e3d_models\$Name",
        "D:\work\plant-code\e3d_models\$Name",
        "D:\e3d_models",
        "D:\work\e3d_models"
    )
    foreach ($candidate in $candidates) {
        if (Test-Path $candidate) { return (Resolve-Path $candidate).Path }
    }
    throw "ProjectPath not found. Pass -ProjectPath explicitly."
}

function Step {
    param([string]$Name)
    Write-Host ""
    Write-Host "== $Name ==" -ForegroundColor Cyan
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
        [string]$Method,
        [string]$Path,
        $Body = $null
    )
    $url = "$BaseUrl$Path"
    $headers = @{ "Content-Type" = "application/json" }
    if ($script:Token) { $headers["Authorization"] = "Bearer $script:Token" }
    $params = @{
        Uri = $url
        Method = $Method
        Headers = $headers
        UseBasicParsing = $true
        TimeoutSec = 60
    }
    if ($null -ne $Body) {
        $params.Body = ($Body | ConvertTo-Json -Depth 20)
    }
    try {
        $response = Invoke-WebRequest @params
        $json = $null
        if ($response.Content) { $json = $response.Content | ConvertFrom-Json }
        return [pscustomobject]@{ Status = [int]$response.StatusCode; Body = $json }
    } catch {
        $status = 0
        $bodyObj = $null
        if ($_.Exception.Response) {
            $status = [int]$_.Exception.Response.StatusCode
            $reader = [System.IO.StreamReader]::new($_.Exception.Response.GetResponseStream())
            $text = $reader.ReadToEnd()
            $reader.Close()
            if ($text) {
                try { $bodyObj = $text | ConvertFrom-Json } catch { $bodyObj = [pscustomobject]@{ message = $text } }
            }
        }
        return [pscustomobject]@{ Status = $status; Body = $bodyObj; Error = $_.Exception.Message }
    }
}

function Require-Success {
    param($Response, [string]$Action)
    if ($Response.Status -ge 200 -and $Response.Status -lt 300 -and $Response.Body.success -ne $false) {
        return $Response.Body.data
    }
    $message = if ($Response.Body -and $Response.Body.message) { $Response.Body.message } else { $Response.Error }
    throw "$Action failed: status=$($Response.Status) message=$message"
}

function Probe-Url {
    param([string]$Url, [int]$MinBytes = 1)
    $response = Invoke-WebRequest -Uri $Url -UseBasicParsing -TimeoutSec 30
    if ($response.StatusCode -lt 200 -or $response.StatusCode -ge 300) {
        throw "HTTP $($response.StatusCode) for $Url"
    }
    $length = 0
    if ($response.RawContentLength -gt 0) { $length = [int64]$response.RawContentLength }
    elseif ($response.Content) { $length = [int64]$response.Content.Length }
    if ($length -lt $MinBytes) {
        throw "Response too small for ${Url}: $length bytes, expected >= $MinBytes"
    }
    Pass "$Url ($length bytes)"
}

function Convert-AgentBrowserJson {
    param([string]$Raw)
    if (-not $Raw) { return $null }
    $parsed = $Raw | ConvertFrom-Json
    if ($parsed -is [string]) {
        return ($parsed | ConvertFrom-Json)
    }
    return $parsed
}

function Invoke-AgentBrowserEvalJson {
    param([string]$Expression)
    $raw = & agent-browser eval $Expression
    if ($LASTEXITCODE -ne 0) {
        throw "agent-browser eval failed with exit code $LASTEXITCODE"
    }
    return Convert-AgentBrowserJson ($raw -join "`n")
}

function Assert-ViewerModelLoaded {
    param([string]$ViewerUrl, [int]$TimeoutSec)

    if ($SkipViewerBrowserCheck) {
        Info "viewer browser check skipped"
        return
    }
    if (-not (Get-Command agent-browser -ErrorAction SilentlyContinue)) {
        throw "agent-browser is required for viewer model validation. Install it or pass -SkipViewerBrowserCheck."
    }

    Step "Validate viewer model load in browser"
    & agent-browser network requests --clear | Out-Null
    & agent-browser console --clear | Out-Null
    & agent-browser open $ViewerUrl | Out-Null
    & agent-browser wait --load networkidle | Out-Null

    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    $last = $null
    while ((Get-Date) -lt $deadline) {
        $last = Invoke-AgentBrowserEvalJson "JSON.stringify({load: window.__dtxLastShowDbnumLoadResult || null, dtxObjectCount: window.__xeokitViewer?.__dtxLayer?.objectCount || 0, compatObjectCount: (window.__xeokitViewer?.scene?.objectIds || []).length})"
        $load = $last.load
        $status = if ($load) { [string]$load.status } else { "pending" }
        $objects = if ($load -and $null -ne $load.loadedObjects) { [int]$load.loadedObjects } else { 0 }
        $dtxObjects = if ($null -ne $last.dtxObjectCount) { [int]$last.dtxObjectCount } else { 0 }
        Info "viewer status=$status loadedObjects=$objects dtxObjectCount=$dtxObjects"
        if ($status -eq "loaded" -and $objects -gt 0 -and $dtxObjects -gt 0) {
            $meshMissing = if ($null -ne $load.mesh404Refnos) { [int]$load.mesh404Refnos } else { 0 }
            $noGeo = if ($null -ne $load.noGeoRowsRefnos) { [int]$load.noGeoRowsRefnos } else { 0 }
            if ($meshMissing -gt 0 -or $noGeo -gt 0) {
                throw "viewer loaded with missing geometry: mesh404Refnos=$meshMissing noGeoRowsRefnos=$noGeo"
            }

            $networkRaw = & agent-browser network requests --filter "/files/" --json
            $network = $networkRaw | ConvertFrom-Json
            $bad = @($network.data.requests | Where-Object { $_.status -ge 400 })
            if ($bad.Count -gt 0) {
                $summary = ($bad | Select-Object -First 5 | ForEach-Object { "$($_.status) $($_.url)" }) -join "; "
                throw "viewer has failed /files requests: $summary"
            }

            Pass "viewer loaded objects=$objects refnos=$($load.refnoCount) skipped=$($load.skippedRefnos)"
            return
        }
        if ($status -eq "error") {
            throw "viewer model load failed: $($load.error)"
        }
        Start-Sleep -Seconds 2
    }

    throw "viewer model did not load within ${TimeoutSec}s. Last=$($last | ConvertTo-Json -Depth 10 -Compress)"
}

Step "Login"
$login = Invoke-AdminApi POST "/api/admin/auth/login" @{ username = $AdminUser; password = $AdminPass }
$loginData = Require-Success $login "login"
$script:Token = $loginData.token
Pass "logged in as $AdminUser"

if (-not $SiteId) {
    Step "Create site with auto deploy"
    $resolvedProjectPath = Resolve-ProjectPath $ProjectName $ProjectPath
    $dbNums = @($ManualDbNums -split "[,\s]+" | Where-Object { $_ } | ForEach-Object { [int]$_ })
    $payload = @{
        project_name = $ProjectName
        project_path = $resolvedProjectPath
        project_code = $ProjectCode
        associated_project = $AssociatedProject
        manual_db_nums = $dbNums
        db_port = $DbPort
        web_port = $WebPort
        bind_host = "127.0.0.1"
        gen_model = $true
        gen_mesh = $false
        gen_spatial_tree = $true
        apply_boolean_operation = $true
        export_json = $false
        export_parquet = $true
        db_user = $DbUser
        db_password = $DbPassword
        auto_deploy = $true
    }
    $created = Require-Success (Invoke-AdminApi POST "/api/admin/sites" $payload) "create site"
    $SiteId = $created.site_id
    $taskId = $created.deployment_task_id
    Pass "created site $SiteId"
    if (-not $taskId) {
        Info "create response did not include deployment_task_id; submitting deploy explicitly"
        $deploy = Require-Success (Invoke-AdminApi POST "/api/admin/sites/$SiteId/deploy") "deploy site"
        $taskId = $deploy.task_id
    }
} else {
    Step "Submit deploy for existing site"
    $deploy = Require-Success (Invoke-AdminApi POST "/api/admin/sites/$SiteId/deploy") "deploy site"
    $taskId = $deploy.task_id
}
if (-not $taskId) { throw "Deploy task id is empty" }
Pass "deploy task $taskId"

Step "Poll deploy task"
$deadline = (Get-Date).AddSeconds($DeployTimeoutSec)
$task = $null
$runtime = $null
while ((Get-Date) -lt $deadline) {
    $task = Require-Success (Invoke-AdminApi GET "/api/admin/tasks/$taskId") "get task"
    $runtime = Require-Success (Invoke-AdminApi GET "/api/admin/sites/$SiteId/runtime") "get runtime"
    $pct = if ($task.progress) { [math]::Round($task.progress.percentage, 0) } else { 0 }
    $step = if ($task.progress) { $task.progress.current_step } else { "" }
    Info "task=$($task.status) pct=$pct runtime=$($runtime.status) stage=$($runtime.current_stage) parse=$($runtime.parse_status) step=$step"
    if ($task.status -eq "Completed") { break }
    if ($task.status -eq "Failed") { throw "Deploy task failed: $($task.error)" }
    Start-Sleep -Seconds 5
}
if (-not $task -or $task.status -ne "Completed") {
    throw "Deploy did not complete within ${DeployTimeoutSec}s"
}
Pass "deploy task completed"

Step "Validate runtime"
$runtime = Require-Success (Invoke-AdminApi GET "/api/admin/sites/$SiteId/runtime") "get runtime"
if ($runtime.status -ne "Running") { throw "runtime status is $($runtime.status), expected Running" }
if (-not $runtime.db_running) { throw "db_running is false" }
if (-not $runtime.web_running) { throw "web_running is false" }
if (-not ($runtime.viewer_running -or $runtime.viewer_url)) { throw "viewer is not running and viewer_url is empty" }
Pass "runtime db/web/viewer is ready"

Step "Probe URLs"
Probe-Url "http://127.0.0.1:$($runtime.web_port)/api/status" 2
if ($runtime.viewer_url) { Probe-Url $runtime.viewer_url 64 }
if ($runtime.viewer_url) { Assert-ViewerModelLoaded $runtime.viewer_url $ViewerBrowserTimeoutSec }

Step "Validate backend deploy-validation report"
$report = Require-Success (Invoke-AdminApi GET "/api/admin/sites/$SiteId/deploy-validation") "get deploy validation"
if (-not $report.exists) {
    throw "deploy-validation report does not exist for $SiteId"
}
$blocking = @($report.checks | Where-Object { $_.status -eq "blocking" })
if ($blocking.Count -gt 0) {
    $summary = ($blocking | ForEach-Object { "$($_.label): $($_.message)" }) -join "; "
    throw "deploy-validation report contains blocking checks: $summary"
}
Pass "deploy-validation report has no blocking checks"
foreach ($check in @($report.checks | Where-Object { $_.url })) {
    Probe-Url $check.url 1
}

if (-not $SkipCleanup) {
    Step "Stop site"
    [void](Require-Success (Invoke-AdminApi POST "/api/admin/sites/$SiteId/stop") "stop site")
    Pass "site stopped"
}

Step "Summary"
Pass "one-click deploy smoke passed for $SiteId"


