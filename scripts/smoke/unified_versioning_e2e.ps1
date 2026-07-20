# Spec 024 isolated end-to-end smoke (SC-001 .. SC-006).
# No cargo test is used. The script starts its own versioned RocksDB instance,
# imports deterministic SurrealQL fixtures, exercises the real CLI/export path,
# verifies the retired HTTP route, and writes a JSON evidence report.

param(
    [int]$SurrealPort = 8047,
    [int]$WebPort = 3247,
    [string]$SurrealBin = "",
    [switch]$SkipBuild,
    [switch]$KeepData,
    [string]$ReportPath = "db-data/unified_versioning_e2e.out.json"
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $repo

if ([string]::IsNullOrWhiteSpace($SurrealBin)) {
    $surrealCommand = Get-Command surreal -ErrorAction Stop
    $SurrealBin = $surrealCommand.Source
}

$startedAt = Get-Date
$failures = New-Object System.Collections.Generic.List[string]
$checks = New-Object System.Collections.Generic.List[object]
$surrealProc = $null
$webProc = $null
$lockJob = $null
$workDir = Join-Path $repo "db-data/spec024-unified-e2e"
$cargoTargetDir = if ([string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) {
    Join-Path $repo "target"
} elseif ([IO.Path]::IsPathRooted($env:CARGO_TARGET_DIR)) {
    $env:CARGO_TARGET_DIR
} else {
    Join-Path $repo $env:CARGO_TARGET_DIR
}
$cliBin = Join-Path $cargoTargetDir "debug/aios-database.exe"
$webBin = Join-Path $cargoTargetDir "debug/web_server.exe"
$dbPath = Join-Path $workDir "surreal.rdb"
$configArg = Join-Path $workDir "DbOption"
$configPath = "${configArg}.toml"
$exportDir = Join-Path $workDir "exports"
$serverOut = Join-Path $workDir "surreal.out.log"
$serverErr = Join-Path $workDir "surreal.err.log"
$webOut = Join-Path $workDir "web.out.log"
$webErr = Join-Path $workDir "web.err.log"
$outputRoot = (Join-Path $workDir "output").Replace("\", "/")

function Add-Check([string]$Id, [string]$Name, [bool]$Passed, [object]$Evidence) {
    $entry = [ordered]@{
        id       = $Id
        name     = $Name
        passed   = $Passed
        evidence = $Evidence
    }
    $script:checks.Add([pscustomobject]$entry)
    if (-not $Passed) {
        $script:failures.Add("${Id}: ${Name}")
    }
    $color = if ($Passed) { "Green" } else { "Red" }
    $mark = if ($Passed) { "PASS" } else { "FAIL" }
    Write-Host "[$mark] $Id $Name" -ForegroundColor $color
}

function Wait-Http([string]$Url, [int]$Attempts = 60) {
    foreach ($i in 1..$Attempts) {
        try {
            $response = Invoke-WebRequest -Uri $Url -UseBasicParsing -TimeoutSec 2
            if ([int]$response.StatusCode -ge 200 -and [int]$response.StatusCode -lt 500) {
                return $true
            }
        } catch {
            if ($null -ne $_.Exception.Response) {
                return $true
            }
        }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

function Invoke-Native([string]$File, [string[]]$Arguments) {
    # Windows PowerShell upgrades native stderr records to terminating errors
    # when the script-wide ErrorActionPreference is Stop. Capture diagnostics
    # without aborting; the native exit code remains the source of truth.
    $previousErrorActionPreference = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $lines = @(& $File @Arguments 2>&1 | ForEach-Object { "$_" })
        $nativeExitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    return [pscustomobject]@{
        exit = $nativeExitCode
        lines = $lines
        text = ($lines -join "`n")
    }
}

function ConvertFrom-MixedJson([string]$Text) {
    try {
        return ($Text | ConvertFrom-Json)
    } catch {}
    for ($i = 0; $i -lt $Text.Length; $i++) {
        if ($Text[$i] -ne '{' -and $Text[$i] -ne '[') { continue }
        $candidate = $Text.Substring($i).Trim()
        try {
            return ($candidate | ConvertFrom-Json)
        } catch {}
    }
    throw "CLI output did not contain a parseable JSON payload:`n$Text"
}

function Invoke-Cli([string[]]$Arguments, [bool]$ExpectSuccess = $true) {
    $allArgs = @("--config", $configArg) + $Arguments
    $result = Invoke-Native $cliBin $allArgs
    if ($ExpectSuccess -and $result.exit -ne 0) {
        throw "CLI failed (exit=$($result.exit)): $($allArgs -join ' ')`n$($result.text)"
    }
    return $result
}

function Invoke-Sql([string]$Ns, [string]$Db, [string]$Sql) {
    $auth = [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes("root:root"))
    $headers = @{
        Authorization = "Basic $auth"
        "Surreal-NS" = $Ns
        "Surreal-DB" = $Db
        Accept = "application/json"
    }
    return Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:$SurrealPort/sql" `
        -Headers $headers -ContentType "text/plain" -Body $Sql
}

try {
    if (Get-NetTCPConnection -LocalPort $SurrealPort -ErrorAction SilentlyContinue) {
        throw "SurrealPort $SurrealPort is already in use; refusing to stop an unrelated process"
    }
    if (Get-NetTCPConnection -LocalPort $WebPort -ErrorAction SilentlyContinue) {
        throw "WebPort $WebPort is already in use; refusing to stop an unrelated process"
    }
    if (Test-Path $workDir) {
        Remove-Item -Recurse -Force $workDir
    }
    New-Item -ItemType Directory -Path $workDir | Out-Null
    New-Item -ItemType Directory -Path $exportDir | Out-Null

    if (-not $SkipBuild) {
        $build = Invoke-Native "cargo" @(
            "build", "--features", "review",
            "--bin", "aios-database", "--bin", "web_server"
        )
        Add-Check "SC-004" "review binaries compile" ($build.exit -eq 0) @{
            command = "cargo build --features review --bin aios-database --bin web_server"
            exit = $build.exit
        }
        if ($build.exit -ne 0) { throw $build.text }

        $syncCheck = Invoke-Native "cargo" @(
            "check", "--no-default-features", "--features", "sync-cli,sqlite-index"
        )
        Add-Check "SC-004" "sync-cli feature matrix compiles" ($syncCheck.exit -eq 0) @{
            command = "cargo check --no-default-features --features sync-cli,sqlite-index"
            exit = $syncCheck.exit
        }
        if ($syncCheck.exit -ne 0) { throw $syncCheck.text }

        $fullCheck = Invoke-Native "cargo" @("check", "--features", "full")
        Add-Check "SC-004" "full/parquet feature matrix compiles" ($fullCheck.exit -eq 0) @{
            command = "cargo check --features full"
            exit = $fullCheck.exit
        }
        if ($fullCheck.exit -ne 0) { throw $fullCheck.text }
    }

    $dbUriPath = $dbPath.Replace("\", "/")
    $surrealProc = Start-Process -FilePath $SurrealBin -ArgumentList @(
        "start", "--user", "root", "--pass", "root",
        "--bind", "127.0.0.1:$SurrealPort", "--log", "error",
        "rocksdb://${dbUriPath}?versioned=true&retention=0"
    ) -RedirectStandardOutput $serverOut -RedirectStandardError $serverErr -PassThru -WindowStyle Hidden
    if (-not (Wait-Http "http://127.0.0.1:$SurrealPort/health")) {
        throw "isolated SurrealDB failed to start; see $serverOut / $serverErr"
    }

    $rangeImport = Invoke-Native $SurrealBin @(
        "import", "--endpoint", "http://127.0.0.1:$SurrealPort",
        "--username", "root", "--password", "root",
        "--namespace", "spec024", "--database", "model_range_gate",
        "db-data/verify_versioned_model_range.surql"
    )
    Add-Check "SC-003" "array-id range VERSION capability gate" ($rangeImport.exit -eq 0) @{
        fixture = "db-data/verify_versioned_model_range.surql"
        exit = $rangeImport.exit
    }
    if ($rangeImport.exit -ne 0) { throw $rangeImport.text }

    $historyImport = Invoke-Native $SurrealBin @(
        "import", "--endpoint", "http://127.0.0.1:$SurrealPort",
        "--username", "root", "--password", "root",
        "--namespace", "spec024", "--database", "spec024_unified_e2e",
        "db-data/verify_024_model_history.surql"
    )
    Add-Check "SC-001" "data/model_gen dual anchors ordered after writes" ($historyImport.exit -eq 0) @{
        fixture = "db-data/verify_024_model_history.surql"
        exit = $historyImport.exit
    }
    if ($historyImport.exit -ne 0) { throw $historyImport.text }

    $failClosedSql = Get-Content -Raw "db-data/verify_024_fail_closed.surql"
    $failClosedResponse = Invoke-Sql "test" "verify_024_fail_closed" $failClosedSql
    $lastFailStatement = @($failClosedResponse)[-1]
    $failState = Invoke-Sql "test" "verify_024_fail_closed" @"
SELECT status, last_error FROM version_commit_state:[24024, 2];
SELECT count() AS count FROM sesno_version_anchor WHERE dbnum = 24024 AND sesno = 2 GROUP ALL;
"@
    $pendingRows = @($failState[0].result)
    $anchorRows = @($failState[1].result)
    $anchorCount = if ($anchorRows.Count -eq 0) { 0 } else { [int]$anchorRows[0].count }
    $failClosedOk = ($lastFailStatement.status -eq "ERR") -and
        ($pendingRows.Count -eq 1) -and
        ($pendingRows[0].status -eq "pending") -and
        ($anchorCount -eq 0)
    Add-Check "SC-002" "statement error leaves pending and publishes no anchor" $failClosedOk @{
        statement_status = $lastFailStatement.status
        commit_status = if ($pendingRows.Count) { $pendingRows[0].status } else { $null }
        anchor_count = $anchorCount
    }

    @"
total_sync = false
replace_dbs = false
incr_sync = false
sync_history = false
enable_log = false
sync_graph_db = false
sync_versioned = false
sync_tidb = false
sync_live = false
enable_index = false
versioned_storage = true
version_retention = "0"
project_path = "."
included_projects = ["spec024"]
project_name = "spec024_unified_e2e"
project_code = "24024"
surreal_ns = "spec024"
surreal_ip = "127.0.0.1"
surreal_port = $SurrealPort
surreal_user = "root"
surreal_password = "root"
mdb_name = "ALL"
module = "DESI"
gen_model = false
gen_mesh = false
gen_model_batch_size = 16
mesh_tol_ratio = 3.0
save_db = true
apply_boolean_operation = false
gen_spatial_tree = false
load_spatial_tree = false
save_spatial_tree_to_db = false
rebuild_ssc_tree = false
only_sync_sys = false
need_sync_refno_basic = false
only_update_dbinfo = false
debug_print_world_transform = false
debug_refno_types = []
meshes_path = "./assets/meshes"
output_root = "$outputRoot"
ip = "127.0.0.1"
user = "root"
password = ""
port = "3306"
sql_threads_number = 1
batch_insert_sql_cnt = 100
mqtt_host = "127.0.0.1"
mqtt_port = 1883
location = "spec024"
location_dbs = [24024]
file_server_host = ""
remote_file_server_hosts = []
surreal_script_dir = "../rs-core/resource/surreal"
puhua_database_ip = ""
puhua_database_user = ""
puhua_database_password = ""

[web_server]
port = $WebPort
auto_start_surreal = false

[surrealdb]
mode = "ws"
ip = "127.0.0.1"
port = $SurrealPort
user = "root"
password = "root"
path = "$dbUriPath"
"@ | Set-Content -Path $configPath -Encoding UTF8

    $env:AIOS_QUIET_CONFIG = "1"
    $snapshotTimer = [Diagnostics.Stopwatch]::StartNew()
    $snapshotV1Raw = Invoke-Cli @(
        "model-version", "history", "model-snapshot",
        "--refno", "24001_100", "--sesno", "100", "--dbnum", "24024", "--json"
    )
    $snapshotTimer.Stop()
    $snapshotV1 = ConvertFrom-MixedJson $snapshotV1Raw.text
    Add-Check "SC-003" "model snapshot returns generation 1 below 2 seconds" (
        $snapshotV1.exists -and
        $snapshotV1.anchor.sesno -eq 100 -and
        $snapshotTimer.ElapsedMilliseconds -lt 2000
    ) @{
        elapsed_ms = $snapshotTimer.ElapsedMilliseconds
        resolved_sesno = $snapshotV1.anchor.sesno
        exact = $snapshotV1.anchor.exact
    }

    $fallbackRaw = Invoke-Cli @(
        "model-version", "history", "model-snapshot",
        "--refno", "24001_100", "--sesno", "150", "--dbnum", "24024", "--json"
    )
    $fallback = ConvertFrom-MixedJson $fallbackRaw.text
    Add-Check "SC-003" "missing sesno falls back to nearest model_gen anchor" (
        $fallback.anchor.sesno -eq 100 -and -not $fallback.anchor.exact
    ) @{
        requested_sesno = 150
        resolved_sesno = $fallback.anchor.sesno
        exact = $fallback.anchor.exact
    }

    $deletedBefore = ConvertFrom-MixedJson (Invoke-Cli @(
        "model-version", "history", "model-snapshot",
        "--refno", "24001_200", "--sesno", "100", "--dbnum", "24024", "--json"
    )).text
    $deletedAfter = ConvertFrom-MixedJson (Invoke-Cli @(
        "model-version", "history", "model-snapshot",
        "--refno", "24001_200", "--sesno", "200", "--dbnum", "24024", "--json"
    )).text
    Add-Check "SC-003" "hard deletion preserves complete pre-delete model generation" (
        $deletedBefore.exists -and -not $deletedAfter.exists
    ) @{
        before_exists = $deletedBefore.exists
        after_exists = $deletedAfter.exists
    }

    $diff = ConvertFrom-MixedJson (Invoke-Cli @(
        "model-version", "history", "model-diff",
        "--refnos", "24001_100,24001_200",
        "--from-sesno", "100", "--to-sesno", "200",
        "--dbnum", "24024", "--json"
    )).text
    Add-Check "SC-003" "model diff reports changed and deleted refnos" (
        @($diff).Count -eq 2 -and
        @($diff | Where-Object { $_.refno_u64 -eq $deletedBefore.refno_u64 -and "$($_.kind)" -match "Deleted" }).Count -eq 1
    ) @{
        diff_count = @($diff).Count
        kinds = @($diff | ForEach-Object { "$($_.kind)" })
    }

    $missingAnchor = Invoke-Cli @(
        "model-version", "history", "model-snapshot",
        "--refno", "24001_100", "--sesno", "100", "--dbnum", "99999", "--json"
    ) $false
    Add-Check "SC-003" "missing model anchor fails explicitly" (
        $missingAnchor.exit -ne 0 -and $missingAnchor.text -match "锚点|anchor"
    ) @{
        exit = $missingAnchor.exit
        message = $missingAnchor.text
    }

    $exportV1 = ConvertFrom-MixedJson (Invoke-Cli @(
        "model-version", "export", "--dbnum", "24024", "--sesno", "100",
        "--format", "v3-json", "--output", $exportDir, "--json"
    )).text
    $exportV2 = ConvertFrom-MixedJson (Invoke-Cli @(
        "model-version", "export", "--dbnum", "24024", "--sesno", "200",
        "--format", "v3-json", "--output", $exportDir, "--json"
    )).text
    $v1File = Join-Path $exportDir "instances_v3_24024_sesno_100.json"
    $v2File = Join-Path $exportDir "instances_v3_24024_sesno_200.json"
    $v1Json = Get-Content -Raw $v1File | ConvertFrom-Json
    $v2Json = Get-Content -Raw $v2File | ConvertFrom-Json
    $v1Rows = @($v1Json.ungrouped | ForEach-Object { @($_.geos).Count } | Measure-Object -Sum).Sum
    $v2Rows = @($v2Json.ungrouped | ForEach-Object { @($_.geos).Count } | Measure-Object -Sum).Sum
    $v1WorldTransform = $v1Json.transforms.PSObject.Properties["world_e_v1"].Value
    $v2WorldTransform = $v2Json.transforms.PSObject.Properties["world_e_v2"].Value
    $v1HasDeletedRefno = @($v1Json.ungrouped | Where-Object { $_.refno -match "24001.200" }).Count -eq 1
    $v2HasDeletedRefno = @($v2Json.ungrouped | Where-Object { $_.refno -match "24001.200" }).Count -gt 0
    Add-Check "SC-006" "anchored v3 exports preserve baseline row count" (
        $exportV1.resolved_sesno -eq 100 -and
        $exportV2.resolved_sesno -eq 200 -and
        $exportV1.source -eq "model_gen" -and
        $exportV2.source -eq "model_gen" -and
        $v1Rows -eq 2 -and $v2Rows -eq 2 -and
        $null -ne $v1WorldTransform -and $null -ne $v2WorldTransform -and
        [double]$v1WorldTransform[12] -eq 10.0 -and
        [double]$v2WorldTransform[12] -eq 11.0 -and
        $v1HasDeletedRefno -and -not $v2HasDeletedRefno
    ) @{
        generation_1_rows = $v1Rows
        generation_2_rows = $v2Rows
        generation_1_world_x = if ($null -ne $v1WorldTransform) { $v1WorldTransform[12] } else { $null }
        generation_2_world_x = if ($null -ne $v2WorldTransform) { $v2WorldTransform[12] } else { $null }
        deleted_refno_in_generation_1 = $v1HasDeletedRefno
        deleted_refno_in_generation_2 = $v2HasDeletedRefno
        generation_1_file = $v1File
        generation_2_file = $v2File
    }

    $idEvidence = ConvertFrom-MixedJson (Invoke-Cli @(
        "model-record-id-verify", "--refno", "24001_100", "--json"
    )).text
    $pureRefnoIds = ($idEvidence.inst_relate -eq "inst_relate:[24001,100]") -and
        ($idEvidence.geo_relate_0 -eq "geo_relate:[24001,100,0]") -and
        ($idEvidence.tubi_relate_0 -eq "tubi_relate:[24001,100,0]")

    $retiredFiles = @(
        "src/data_interface/increment_manager.rs",
        "src/web_api/model_version_api.rs",
        "src/version_management/ducklake_store.rs",
        "src/version_management/model_release.rs",
        "src/version_management/release_package.rs",
        "src/fast_model/gen_model/model_writer_ducklake.rs",
        "src/bin/ducklake_parity.rs"
    )
    $survivingRetiredFiles = @($retiredFiles | Where-Object { Test-Path $_ })
    $watchSymbols = & rg -n "init_watcher|execute_incr_update|async_watch|exec_watcher|spawn_exec_watcher|INCREMENT_DATA|target_sesno" src ui/admin/src 2>$null
    $watchRgExit = $LASTEXITCODE
    $releaseSymbols = & rg -n "release_id|unit_version|publication_handoff|history_replay|model_release|release_package" src 2>$null
    $releaseRgExit = $LASTEXITCODE
    Add-Check "SC-004" "pure-refno IDs and retired paths/symbols are absent" (
        $pureRefnoIds -and
        $survivingRetiredFiles.Count -eq 0 -and
        $watchRgExit -eq 1 -and
        $releaseRgExit -eq 1
    ) @{
        pure_refno_ids = $pureRefnoIds
        surviving_retired_files = $survivingRetiredFiles
        watch_symbol_hits = @($watchSymbols)
        release_symbol_hits = @($releaseSymbols)
    }

    $lockDir = Join-Path $workDir "output/spec024_unified_e2e"
    $lockPath = Join-Path $lockDir "incremental.lock"
    $lockReady = Join-Path $workDir "lock.ready"
    New-Item -ItemType Directory -Force -Path $lockDir | Out-Null
    $lockJob = Start-Job -ScriptBlock {
        param($Path, $Ready)
        $stream = [IO.File]::Open($Path, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::ReadWrite)
        try {
            $stream.Lock(0, 1)
            Set-Content -Path $Ready -Value "ready"
            Start-Sleep -Seconds 10
        } finally {
            try { $stream.Unlock(0, 1) } catch {}
            $stream.Dispose()
        }
    } -ArgumentList $lockPath, $lockReady
    foreach ($i in 1..50) {
        if (Test-Path $lockReady) { break }
        Start-Sleep -Milliseconds 100
    }
    if (-not (Test-Path $lockReady)) { throw "lock holder did not become ready" }
    $lockAttempt = Invoke-Cli @(
        "incremental-sesno", "--file", (Join-Path $workDir "missing.db"),
        "--from-sesno", "0", "--json"
    ) $false
    Add-Check "SC-005" "second mutating process is rejected by project lock" (
        $lockAttempt.exit -ne 0 -and $lockAttempt.text -match "锁|lock|held|占用"
    ) @{
        exit = $lockAttempt.exit
        message = $lockAttempt.text
    }
    Stop-Job $lockJob -ErrorAction SilentlyContinue
    Remove-Job $lockJob -Force -ErrorAction SilentlyContinue
    $lockJob = $null

    $webProc = Start-Process -FilePath $webBin `
        -ArgumentList @("--config", $configArg) `
        -RedirectStandardOutput $webOut -RedirectStandardError $webErr `
        -PassThru -WindowStyle Hidden
    if (-not (Wait-Http "http://127.0.0.1:$WebPort/api/version")) {
        throw "web_server failed to start; see $webOut / $webErr"
    }
    $oldRouteStatus = 0
    try {
        $oldRoute = Invoke-WebRequest -Uri "http://127.0.0.1:$WebPort/api/model-version/releases" `
            -UseBasicParsing -TimeoutSec 10
        $oldRouteStatus = [int]$oldRoute.StatusCode
    } catch {
        if ($null -ne $_.Exception.Response) {
            $oldRouteStatus = [int]$_.Exception.Response.StatusCode
        } else {
            throw
        }
    }
    Add-Check "SC-004" "retired /api/model-version route returns 404" ($oldRouteStatus -eq 404) @{
        url = "http://127.0.0.1:$WebPort/api/model-version/releases"
        status = $oldRouteStatus
    }
}
catch {
    $failures.Add("runner: $($_.Exception.Message)")
    Write-Host $_.Exception.Message -ForegroundColor Red
}
finally {
    if ($null -ne $lockJob) {
        Stop-Job $lockJob -ErrorAction SilentlyContinue
        Remove-Job $lockJob -Force -ErrorAction SilentlyContinue
    }
    if ($null -ne $webProc -and -not $webProc.HasExited) {
        Stop-Process -Id $webProc.Id -Force -ErrorAction SilentlyContinue
    }
    if ($null -ne $surrealProc -and -not $surrealProc.HasExited) {
        Stop-Process -Id $surrealProc.Id -Force -ErrorAction SilentlyContinue
    }

    $finishedAt = Get-Date
    $report = [ordered]@{
        schema_version = 1
        spec = "024-unified-rocksdb-versioning"
        started_at = $startedAt.ToString("o")
        finished_at = $finishedAt.ToString("o")
        elapsed_ms = [int64]($finishedAt - $startedAt).TotalMilliseconds
        passed = ($failures.Count -eq 0)
        surreal_port = $SurrealPort
        web_port = $WebPort
        checks = @($checks | ForEach-Object { $_ })
        failures = @($failures | ForEach-Object { $_ })
    }
    $reportParent = Split-Path -Parent (Join-Path $repo $ReportPath)
    if (-not (Test-Path $reportParent)) {
        New-Item -ItemType Directory -Force -Path $reportParent | Out-Null
    }
    $report | ConvertTo-Json -Depth 12 | Set-Content -Path (Join-Path $repo $ReportPath) -Encoding UTF8
    Write-Host "Evidence: $ReportPath"

    if (-not $KeepData -and $null -ne $surrealProc -and $surrealProc.HasExited) {
        # Keep JSON exports and logs until after report generation; remove only RocksDB.
        if (Test-Path $dbPath) {
            Remove-Item -Recurse -Force $dbPath -ErrorAction SilentlyContinue
        }
    }
}

if ($failures.Count -gt 0) {
    Write-Host "Spec 024 E2E failed: $($failures.Count) issue(s)" -ForegroundColor Red
    exit 1
}
Write-Host "Spec 024 SC-001..SC-006 passed" -ForegroundColor Green
exit 0
