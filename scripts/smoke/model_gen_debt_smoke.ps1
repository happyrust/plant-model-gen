param(
    [Parameter(Mandatory = $true)]
    [uint32[]]$Dbnum,
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
if (@($report.results).Count -ne $Dbnum.Count) {
    throw "expected $($Dbnum.Count) result(s), got $(@($report.results).Count)"
}
foreach ($result in $report.results) {
    foreach ($field in @("data_watermark", "model_generation_watermark", "debt_ranges", "coverage_complete", "needs_full_regen")) {
        if ($null -eq $result.coverage.$field) {
            throw "dbnum=$($result.dbnum) missing coverage.$field"
        }
    }
    if ($null -ne $result.model_gen_anchor) {
        throw "dry-run unexpectedly published model_gen anchor for dbnum=$($result.dbnum)"
    }
}

Write-Host "PASS model_gen_debt dry-run: dbnums=$($Dbnum -join ',')" -ForegroundColor Green
exit 0
