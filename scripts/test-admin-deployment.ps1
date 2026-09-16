# Admin 站点管理 - 端到端部署测试脚本
# 使用 AvevaPlantSample + aps7011_0001 进行回归测试
# 用法: .\scripts\test-admin-deployment.ps1 [-BaseUrl http://127.0.0.1:3100] [-ProjectPath "D:\path\to\e3d_models"]

param(
    [string]$BaseUrl = "http://127.0.0.1:3100",
    [string]$ProjectName = "AvevaPlantSample",
    [string]$ProjectPath = "",
    [int]$ProjectCode = 7011,
    [int]$DbPort = 18200,
    [int]$WebPort = 18100,
    [string]$ManualDbNums = "1",
    [int]$ParseTimeoutSec = 600,
    [int]$StartTimeoutSec = 120,
    [switch]$SkipCleanup
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$AdminUser = $env:ADMIN_USER
$AdminPass = $env:ADMIN_PASS
if (-not $AdminUser -or -not $AdminPass) {
    Write-Host "[FAIL] ADMIN_USER / ADMIN_PASS not set" -ForegroundColor Red
    exit 1
}

if (-not $ProjectPath) {
    $candidates = @(
        "D:\e3d_models",
        "D:\work\e3d_models",
        "C:\e3d_models",
        "/data/e3d_models",
        "/opt/e3d_models"
    )
    foreach ($c in $candidates) {
        $full = Join-Path $c $ProjectName
        if (Test-Path $full) { $ProjectPath = $c; break }
        if (Test-Path $c) { $ProjectPath = $c; break }
    }
    if (-not $ProjectPath) {
        Write-Host "[FAIL] ProjectPath not found, use -ProjectPath" -ForegroundColor Red
        exit 1
    }
}

$token = ""
$siteId = ""
$step = 0
$passed = 0
$failed = 0

function Step($name) {
    $script:step++
    Write-Host ""
    Write-Host "[$script:step] $name" -ForegroundColor Cyan
}

function Pass($msg) {
    $script:passed++
    Write-Host "  [PASS] $msg" -ForegroundColor Green
}

function Fail($msg) {
    $script:failed++
    Write-Host "  [FAIL] $msg" -ForegroundColor Red
}

function Info($msg) {
    Write-Host "  $msg" -ForegroundColor Gray
}

function Api($method, $path, $body) {
    $url = "$BaseUrl$path"
    $headers = @{ "Content-Type" = "application/json" }
    if ($token) { $headers["Authorization"] = "Bearer $token" }

    $params = @{
        Uri = $url
        Method = $method
        Headers = $headers
        UseBasicParsing = $true
    }
    if ($body) {
        $params["Body"] = ($body | ConvertTo-Json -Depth 10)
    }

    try {
        $resp = Invoke-WebRequest @params
        return @{
            Status = $resp.StatusCode
            Body = ($resp.Content | ConvertFrom-Json)
        }
    } catch {
        $status = 0
        $bodyObj = $null
        if ($_.Exception.Response) {
            $status = [int]$_.Exception.Response.StatusCode
            $reader = [System.IO.StreamReader]::new($_.Exception.Response.GetResponseStream())
            $text = $reader.ReadToEnd()
            $reader.Close()
            try { $bodyObj = $text | ConvertFrom-Json } catch { $bodyObj = @{ message = $text } }
        }
        return @{
            Status = $status
            Body = $bodyObj
            Error = $_.Exception.Message
        }
    }
}

# ─── Step 1: Login ───
Step "Login"
$r = Api POST "/api/admin/auth/login" @{ username = $AdminUser; password = $AdminPass }
if ($r.Status -eq 200 -and $r.Body.success) {
    $token = $r.Body.data.token
    Pass "logged in as $AdminUser, token=$($token.Substring(0,8))..."
} else {
    Fail "login failed: $($r.Body.message)"
    exit 1
}

# ─── Step 2: Create Site ───
Step "Create Site ($ProjectName)"
$dbNums = $ManualDbNums -split "," | ForEach-Object { [int]$_.Trim() }
$payload = @{
    project_name = $ProjectName
    project_path = $ProjectPath
    project_code = $ProjectCode
    manual_db_nums = $dbNums
    db_port = $DbPort
    web_port = $WebPort
    bind_host = "0.0.0.0"
    db_user = "root"
    db_password = "root"
}
$r = Api POST "/api/admin/sites" $payload
if ($r.Status -in @(200, 201) -and $r.Body.success) {
    $siteId = $r.Body.data.site_id
    Pass "site created: $siteId"
    Info "db_port=$DbPort, web_port=$WebPort, manual_db_nums=$ManualDbNums"
} else {
    Fail "create failed: $($r.Body.message)"
    exit 1
}

# ─── Step 3: Parse ───
Step "Trigger Parse"
$r = Api POST "/api/admin/sites/$siteId/parse"
if ($r.Status -in @(200, 202) -and $r.Body.success) {
    Pass "parse submitted"
} else {
    Fail "parse trigger failed: $($r.Body.message)"
}

# ─── Step 4: Wait for Parse ───
Step "Waiting for Parse (timeout=${ParseTimeoutSec}s)"
$elapsed = 0
$interval = 10
$parseOk = $false
while ($elapsed -lt $ParseTimeoutSec) {
    Start-Sleep -Seconds $interval
    $elapsed += $interval
    $r = Api GET "/api/admin/sites/$siteId/runtime"
    $ps = $r.Body.data.parse_status
    $stage = $r.Body.data.current_stage_label
    Info "  ${elapsed}s - parse_status=$ps, stage=$stage"
    if ($ps -eq "Parsed") {
        $parseOk = $true
        break
    }
    if ($ps -eq "Failed") {
        Fail "parse failed: $($r.Body.data.last_error)"
        break
    }
}
if ($parseOk) {
    $duration = $r.Body.data.resources.last_parse_duration_ms
    Pass "parse completed in $([math]::Round($duration / 1000, 1))s"
} elseif (-not $parseOk -and $elapsed -ge $ParseTimeoutSec) {
    Fail "parse timeout after ${ParseTimeoutSec}s"
}

# ─── Step 5: Start Site ───
Step "Start Site"
$r = Api POST "/api/admin/sites/$siteId/start"
if ($r.Status -in @(200, 202) -and $r.Body.success) {
    Pass "start submitted"
} else {
    Fail "start failed: $($r.Body.message)"
}

# ─── Step 6: Wait for Running ───
Step "Waiting for Running (timeout=${StartTimeoutSec}s)"
$elapsed = 0
$interval = 5
$startOk = $false
while ($elapsed -lt $StartTimeoutSec) {
    Start-Sleep -Seconds $interval
    $elapsed += $interval
    $r = Api GET "/api/admin/sites/$siteId/runtime"
    $st = $r.Body.data.status
    $stage = $r.Body.data.current_stage_label
    Info "  ${elapsed}s - status=$st, stage=$stage"
    if ($st -eq "Running") {
        $startOk = $true
        break
    }
    if ($st -eq "Failed") {
        Fail "start failed: $($r.Body.data.last_error)"
        break
    }
}
if ($startOk) {
    $entry = $r.Body.data.entry_url
    Pass "site running at $entry"
} elseif (-not $startOk -and $elapsed -ge $StartTimeoutSec) {
    Fail "start timeout after ${StartTimeoutSec}s"
}

# ─── Step 7: Health Check ───
Step "Health Check"
if ($startOk) {
    $healthUrl = "http://127.0.0.1:$WebPort/api/status"
    try {
        $resp = Invoke-WebRequest -Uri $healthUrl -UseBasicParsing -TimeoutSec 10
        if ($resp.StatusCode -eq 200) {
            Pass "health check OK at $healthUrl"
        } else {
            Fail "health returned $($resp.StatusCode)"
        }
    } catch {
        Fail "health check failed: $_"
    }
} else {
    Info "skipped (site not running)"
}

# ─── Step 8: Stop Site ───
Step "Stop Site"
$r = Api POST "/api/admin/sites/$siteId/stop"
if ($r.Status -eq 200 -and $r.Body.success) {
    Pass "site stopped"
} else {
    Fail "stop failed: $($r.Body.message)"
}

# ─── Step 9: Delete Site ───
if (-not $SkipCleanup) {
    Step "Delete Site"
    Start-Sleep -Seconds 2
    $r = Api DELETE "/api/admin/sites/$siteId"
    if ($r.Status -eq 200 -and $r.Body.success) {
        Pass "site deleted"
    } else {
        Fail "delete failed: $($r.Body.message)"
    }

    Step "Verify Cleanup"
    $r = Api GET "/api/admin/sites/$siteId"
    if ($r.Status -eq 404) {
        Pass "site confirmed removed"
    } else {
        Fail "site still exists after delete"
    }
} else {
    Info "cleanup skipped (-SkipCleanup)"
}

# ─── Summary ───
Write-Host ""
Write-Host "═══════════════════════════════════════" -ForegroundColor White
Write-Host " Results: $passed passed, $failed failed (total $step steps)" -ForegroundColor $(if ($failed -gt 0) { "Red" } else { "Green" })
Write-Host "═══════════════════════════════════════" -ForegroundColor White

exit $failed
