<#
.SYNOPSIS
  Direct aios-database sidecar smoke for health, WS events, job submit/status, and optional cancel/db-index.

.DESCRIPTION
  Starts `aios-database serve` on a local port, connects to `/events`, then exercises:
    - GET /health
    - POST /jobs/submit-cli
    - GET /jobs/{job_id}
    - optional POST /jobs/{job_id}/cancel
    - optional POST /db-index/rebuild

  By default the job uses a deliberately missing config path and is expected to fail quickly.
  Pass -ConfigNoExt and -ExpectJobSuccess to verify a real successful CLI job.

.EXAMPLE
  pwsh -File scripts/smoke/sidecar_job_events_smoke.ps1

.EXAMPLE
  pwsh -File scripts/smoke/sidecar_job_events_smoke.ps1 `
    -DbIndexRootPath "D:/AVEVA/Projects/E3D2.1/AvevaPlantSample/aps000/aps250160_0001" `
    -ManualDbNums 250160

.EXAMPLE
  pwsh -File scripts/smoke/sidecar_job_events_smoke.ps1 `
    -ConfigNoExt "D:/work/plant-code/plant-model-gen/runtime/sites/my-site/DbOption-parse" `
    -ExpectJobSuccess
#>

[CmdletBinding()]
param(
    [string]$AiosDatabaseBin = "",
    [string]$BindHost = "127.0.0.1",
    [int]$HttpPort = 0,
    [string]$SiteKey = "",
    [string]$Token = "",
    [string]$RuntimeDir = "",
    [string]$Cwd = "",
    [string]$ConfigNoExt = "",
    [switch]$SkipJob,
    [switch]$ExpectJobSuccess,
    [switch]$CancelJob,
    [int]$JobTimeoutSec = 120,
    [string]$DbIndexRootPath = "",
    [string]$DbIndexRootName = "smoke-root",
    [int[]]$ManualDbNums = @(),
    [int]$EventTimeoutSec = 20,
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

function Connect-Events {
    $ws = [System.Net.WebSockets.ClientWebSocket]::new()
    [void]$ws.Options.SetRequestHeader("Authorization", "Bearer $script:Token")
    $uri = [Uri]::new("$($script:BaseUrl.Replace('http://', 'ws://').Replace('https://', 'wss://'))/events")
    [void]$ws.ConnectAsync($uri, [Threading.CancellationToken]::None).GetAwaiter().GetResult()
    return ,$ws
}

function Receive-EventMessages {
    param(
        $Ws,
        [int]$TimeoutSec,
        [int]$MaxEvents = 0,
        [string]$StopAfterType = ""
    )
    $deadline = [DateTimeOffset]::UtcNow.AddSeconds($TimeoutSec)
    $events = @()
    while ([DateTimeOffset]::UtcNow -lt $deadline -and $Ws.State -eq [System.Net.WebSockets.WebSocketState]::Open) {
        $remainingMs = [Math]::Max(100, [int]($deadline - [DateTimeOffset]::UtcNow).TotalMilliseconds)
        $cts = [Threading.CancellationTokenSource]::new($remainingMs)
        $buffer = New-Object byte[] 65536
        $segment = [ArraySegment[byte]]::new($buffer)
        try {
            $result = $Ws.ReceiveAsync($segment, $cts.Token).GetAwaiter().GetResult()
            if ($result.MessageType -eq [System.Net.WebSockets.WebSocketMessageType]::Close) { break }
            $text = [Text.Encoding]::UTF8.GetString($buffer, 0, $result.Count)
            if ($text) {
                Info "event: $text"
                try { $events += ($text | ConvertFrom-Json) } catch { }
                if ($MaxEvents -gt 0 -and $events.Count -ge $MaxEvents) {
                    break
                }
                if ($StopAfterType -and $events.Count -gt 0 -and $events[-1].type -eq $StopAfterType) {
                    break
                }
            }
        } catch [System.OperationCanceledException] {
            break
        } finally {
            $cts.Dispose()
        }
    }
    return @($events)
}

function Assert-EventType {
    param($Events, [string]$Type)
    if (@($Events | Where-Object { $_.type -eq $Type }).Count -eq 0) {
        $seen = @($Events | ForEach-Object { $_.type }) -join ","
        throw "Expected event type '$Type' not seen. Seen: $seen"
    }
    Pass "event type seen: $Type"
}

if (-not $SiteKey) { $SiteKey = "smoke-$([Guid]::NewGuid().ToString('N').Substring(0, 8))" }
if (-not $Token) { $Token = [Guid]::NewGuid().ToString("N") }
if ($HttpPort -le 0) { $HttpPort = Get-FreeTcpPort }
if (-not $RuntimeDir) { $RuntimeDir = Join-Path $repoRoot "runtime/smoke/sidecar-$SiteKey" }
if (-not $Cwd) { $Cwd = $repoRoot.Path }

$script:Token = $Token
$script:BaseUrl = "http://${BindHost}:${HttpPort}"
$bin = Resolve-AiosDatabaseBin $AiosDatabaseBin
$stdoutLog = Join-Path $RuntimeDir "sidecar.stdout.log"
$stderrLog = Join-Path $RuntimeDir "sidecar.stderr.log"
New-Item -ItemType Directory -Force -Path $RuntimeDir | Out-Null

$sidecar = $null
$ws = $null
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

    Write-Section "Events"
    $ws = Connect-Events
    Start-Sleep -Milliseconds 200
    $events = @(Receive-EventMessages -Ws $ws -TimeoutSec 2 -MaxEvents 1)
    Assert-EventType $events "sidecar_hello"

    if ($DbIndexRootPath) {
        Write-Section "Optional db-index rebuild"
        if (-not (Test-Path $DbIndexRootPath)) { throw "DbIndexRootPath not found: $DbIndexRootPath" }
        $indexPath = Join-Path $RuntimeDir "smoke-db-index.sqlite"
        $body = @{
            roots = @(@{ name = $DbIndexRootName; path = (Resolve-Path $DbIndexRootPath).Path })
            index_path = $indexPath
            force = $true
            manual_db_nums = @($ManualDbNums)
        }
        $dbIndex = Invoke-SidecarJson -Method POST -Path "/db-index/rebuild" -Body $body -TimeoutSec 3600
        $dbIndexData = Require-Success $dbIndex "db-index rebuild"
        Pass "db-index rebuild OK db_files=$($dbIndexData.db_files) ref0_total=$($dbIndexData.ref0_total) errors=$($dbIndexData.errors)"
        $events += @(Receive-EventMessages -Ws $ws -TimeoutSec $EventTimeoutSec -StopAfterType "db_index_rebuild_done")
        Assert-EventType $events "db_index_rebuild_started"
        Assert-EventType $events "db_index_rebuild_done"
    } else {
        Info "db-index stage skipped (pass -DbIndexRootPath to enable)"
    }

    if (-not $SkipJob) {
        Write-Section "Job submit/status"
        $providedConfig = -not [string]::IsNullOrWhiteSpace($ConfigNoExt)
        if (-not $providedConfig) {
            $ConfigNoExt = Join-Path $RuntimeDir "missing-config"
            Info "ConfigNoExt not provided; using missing config to validate failed job path"
        }
        $jobStdout = Join-Path $RuntimeDir "job.stdout.log"
        $jobStderr = Join-Path $RuntimeDir "job.stderr.log"
        $submit = Invoke-SidecarJson -Method POST -Path "/jobs/submit-cli" -Body @{
            config_no_ext = $ConfigNoExt
            cwd = $Cwd
            stdout_path = $jobStdout
            stderr_path = $jobStderr
        }
        $submitted = Require-Success $submit "submit job"
        $jobId = $submitted.job_id
        if (-not $jobId) { throw "submit job response missing job_id" }
        Pass "job submitted: $jobId"

        if ($CancelJob) {
            $cancel = Invoke-SidecarJson -Method POST -Path "/jobs/$jobId/cancel" -Body @{}
            [void](Require-Success $cancel "cancel job")
            Pass "cancel requested: $jobId"
        }

        $deadline = [DateTimeOffset]::UtcNow.AddSeconds($JobTimeoutSec)
        $record = $null
        while ([DateTimeOffset]::UtcNow -lt $deadline) {
            Start-Sleep -Milliseconds 500
            $status = Invoke-SidecarJson -Method GET -Path "/jobs/$jobId" -TimeoutSec 10
            $record = Require-Success $status "job status"
            Info "job status=$($record.status) exit_code=$($record.exit_code)"
            if ($record.status -in @("succeeded", "failed", "cancelled")) { break }
        }
        if (-not $record -or $record.status -notin @("succeeded", "failed", "cancelled")) {
            throw "job did not reach a terminal status within ${JobTimeoutSec}s"
        }
        if ($CancelJob -and $record.status -ne "cancelled") {
            throw "expected cancelled job, got status=$($record.status)"
        }
        if ($ExpectJobSuccess -and $record.status -ne "succeeded") {
            throw "expected succeeded job, got status=$($record.status)"
        }
        if (-not $ExpectJobSuccess -and -not $CancelJob -and -not $providedConfig -and $record.status -ne "failed") {
            throw "expected missing-config job to fail, got status=$($record.status)"
        }
        Pass "job terminal status OK: $($record.status)"

        $events += @(Receive-EventMessages -Ws $ws -TimeoutSec $EventTimeoutSec)
        Assert-EventType $events "job_submitted"
        Assert-EventType $events "job_started"
        Assert-EventType $events "stage_changed"
        Assert-EventType $events "log_appended"
        if ($CancelJob) {
            Assert-EventType $events "job_cancel_requested"
            Assert-EventType $events "job_cancelled"
        } elseif ($ExpectJobSuccess) {
            Assert-EventType $events "job_done"
            Assert-EventType $events "artifact_ready"
        } else {
            Assert-EventType $events "job_failed"
        }
    } else {
        Info "job stage skipped (-SkipJob)"
    }

    Write-Section "Done"
    Pass "sidecar job/events smoke passed"
} finally {
    if ($ws -and $ws.State -eq [System.Net.WebSockets.WebSocketState]::Open) {
        try {
            # Avoid hanging on CloseAsync when a prior ReceiveAsync timeout or failed job left
            # the socket in an awkward state.
            $ws.Abort()
            $ws.Dispose()
        } catch { }
    }
    if ($sidecar -and -not $sidecar.HasExited -and -not $KeepSidecar) {
        Stop-Process -Id $sidecar.Id -Force -ErrorAction SilentlyContinue
        Pass "sidecar stopped"
    } elseif ($sidecar -and -not $sidecar.HasExited) {
        Info "sidecar kept alive: pid=$($sidecar.Id) base=$script:BaseUrl token=$script:Token"
    }
}
