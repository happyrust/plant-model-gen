param(
    [Parameter(Mandatory = $true)]
    [string]$SurrealBaseline,
    [Parameter(Mandatory = $true)]
    [string]$DuckLakeCandidate,
    [double]$MaxRegressionRatio = 0.10
)

$ErrorActionPreference = "Stop"

function Read-PerfReport([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Performance report not found: $Path"
    }
    return Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

function Assert-BatchFirst([object]$Report, [string]$Label) {
    $calls = $Report.metadata.generation_read_metrics.backend_calls
    $capabilities = @(
        "element.query",
        "hierarchy.load",
        "attribute.load",
        "catalog.load",
        "transform.load"
    )
    foreach ($property in $calls.PSObject.Properties) {
        $matched = $false
        foreach ($capability in $capabilities) {
            if ($property.Name -eq $capability -or $property.Name.EndsWith(".$capability")) {
                $matched = $true
                break
            }
        }
        if ($matched -and [uint64]$property.Value -gt 1) {
            throw "$Label violates batch-first gate: $($property.Name) calls=$($property.Value)"
        }
    }
}

$baseline = Read-PerfReport $SurrealBaseline
$candidate = Read-PerfReport $DuckLakeCandidate

if ($baseline.metadata.authoritative_snapshot_id -ne $candidate.metadata.authoritative_snapshot_id) {
    throw "Snapshot mismatch: baseline=$($baseline.metadata.authoritative_snapshot_id) candidate=$($candidate.metadata.authoritative_snapshot_id)"
}
if ($baseline.metadata.generation_artifacts_semantic_hash -ne $candidate.metadata.generation_artifacts_semantic_hash) {
    throw "GenerationArtifacts semantic hash mismatch"
}
if ($baseline.metadata.final_model_semantic_hash -ne $candidate.metadata.final_model_semantic_hash) {
    throw "Final model semantic hash mismatch"
}
if ($baseline.metadata.generation_read_backend -ne "surreal") {
    throw "Baseline backend must be surreal, actual=$($baseline.metadata.generation_read_backend)"
}
if ($candidate.metadata.generation_read_backend -ne "ducklake") {
    throw "Candidate backend must be ducklake, actual=$($candidate.metadata.generation_read_backend)"
}

Assert-BatchFirst $baseline "Surreal baseline"
Assert-BatchFirst $candidate "DuckLake candidate"

if ([double]$baseline.total_ms -le 0) {
    throw "Surreal baseline total_ms must be positive"
}
$limit = [double]$baseline.total_ms * (1.0 + $MaxRegressionRatio)
if ([double]$candidate.total_ms -gt $limit) {
    throw "End-to-end regression gate failed: candidate=$($candidate.total_ms)ms baseline=$($baseline.total_ms)ms limit=$([math]::Round($limit, 2))ms"
}

Write-Host "generation-read performance gate passed: snapshot=$($candidate.metadata.authoritative_snapshot_id) baseline=$($baseline.total_ms)ms candidate=$($candidate.total_ms)ms"
