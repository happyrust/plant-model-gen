param(
    [uint32[]]$Dbnum = @(),
    [string]$Exe = "$PSScriptRoot\..\..\target\release\aios-database.exe"
)

$ErrorActionPreference = "Stop"
if (-not (Test-Path -LiteralPath $Exe)) {
    throw "aios-database executable not found: $Exe"
}

$cliArgs = @("model-version", "catch-up")
foreach ($value in $Dbnum) {
    $cliArgs += @("--dbnum", $value)
}
$cliArgs += @("--dry-run", "--json")

$raw = (& $Exe @cliArgs 2>&1 | Out-String)
if ($LASTEXITCODE -ne 0) {
    throw "model-version catch-up --dry-run failed:`n$raw"
}
$start = $raw.IndexOf('{')
$end = $raw.LastIndexOf('}')
if ($start -lt 0 -or $end -le $start) {
    throw "catch-up output contained no JSON object:`n$raw"
}
$report = $raw.Substring($start, $end - $start + 1) | ConvertFrom-Json
if (@($report.failures).Count -ne 0) {
    throw "catch-up dry-run reported failures: $($report.failures -join '; ')"
}
if ($Dbnum.Count -gt 0 -and @($report.results).Count -ne $Dbnum.Count) {
    throw "expected $($Dbnum.Count) result(s), got $(@($report.results).Count)"
}
if ($Dbnum.Count -eq 0 -and @($report.results).Count -eq 0) {
    throw "all-db catch-up returned no candidate results"
}
foreach ($result in $report.results) {
    if ($null -eq $result.stale_debt_reconciled) {
        throw "dbnum=$($result.dbnum) missing stale_debt_reconciled"
    }
    if ([int]$result.stale_debt_reconciled -ne 0) {
        throw "dry-run unexpectedly reconciled stale debt for dbnum=$($result.dbnum)"
    }
    foreach ($field in @(
        "data_watermark",
        "model_generation_watermark",
        "range_semantics",
        "debt_ranges",
        "consumable_debt_ranges",
        "stale_debt_ranges",
        "gap_ranges",
        "debt_bucket_counts",
        "consumable_bucket_counts",
        "coverage_complete",
        "needs_full_regen"
    )) {
        if ($null -eq $result.coverage.$field) {
            throw "dbnum=$($result.dbnum) missing coverage.$field"
        }
    }
    if ($result.coverage.range_semantics -ne "[from_sesno,to_sesno]") {
        throw "dbnum=$($result.dbnum) unexpected range semantics: $($result.coverage.range_semantics)"
    }
    foreach ($countsName in @("debt_bucket_counts", "consumable_bucket_counts")) {
        $counts = $result.coverage.$countsName
        foreach ($field in @("prim", "loop_owner", "bran_hanger", "basic_cata", "delete", "total")) {
            if ($null -eq $counts.$field) {
                throw "dbnum=$($result.dbnum) missing coverage.$countsName.$field"
            }
        }
    }
    if ($null -ne $result.model_gen_anchor) {
        throw "dry-run unexpectedly published model_gen anchor for dbnum=$($result.dbnum)"
    }
}

$scope = if ($Dbnum.Count -gt 0) { $Dbnum -join ',' } else { "all" }
Write-Host "PASS model_gen_debt dry-run: dbnums=$scope" -ForegroundColor Green
exit 0
