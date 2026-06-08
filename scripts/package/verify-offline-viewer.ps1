param(
    [string]$Root = (Get-Location).Path
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

function Assert-DuckDbOfflineDist([string]$Dist, [string]$Label) {
    if (-not (Test-Path -LiteralPath (Join-Path $Dist "index.html") -PathType Leaf)) {
        throw "Viewer dist not found for ${Label}: $Dist"
    }

    $duckdbDir = Join-Path $Dist "duckdb"
    $required = @(
        "duckdb-browser-mvp.worker.js",
        "duckdb-browser-eh.worker.js",
        "duckdb-browser-coi.worker.js",
        "duckdb-browser-coi.pthread.worker.js",
        "duckdb-mvp.wasm",
        "duckdb-eh.wasm",
        "duckdb-coi.wasm"
    )
    foreach ($file in $required) {
        $path = Join-Path $duckdbDir $file
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "DuckDB offline asset missing for ${Label}: $path"
        }
    }

    $hits = Get-ChildItem -LiteralPath $Dist -Recurse -File -Include "*.js" -ErrorAction SilentlyContinue |
        Select-String -Pattern "https://cdn\.jsdelivr\.net/(npm/)?@duckdb/duckdb-wasm"
    if ($hits) {
        $first = $hits | Select-Object -First 1
        throw "DuckDB CDN URL found in ${Label}: $($first.Path)"
    }

    Write-Host "DuckDB offline viewer OK: $Label ($Dist)" -ForegroundColor Green
}

$resolvedRoot = [System.IO.Path]::GetFullPath($Root)
$targets = @()

foreach ($name in @("viewer-root", "viewer")) {
    $candidate = Join-Path $resolvedRoot $name
    if (Test-Path -LiteralPath (Join-Path $candidate "index.html") -PathType Leaf) {
        $targets += [pscustomobject]@{ Label = $name; Path = $candidate }
    }
}

if ($targets.Count -eq 0 -and (Test-Path -LiteralPath (Join-Path $resolvedRoot "index.html") -PathType Leaf)) {
    $targets += [pscustomobject]@{ Label = "viewer"; Path = $resolvedRoot }
}

if ($targets.Count -eq 0) {
    throw "No viewer dist found under $resolvedRoot. Pass the package root, viewer-root, or viewer directory."
}

foreach ($target in $targets) {
    Assert-DuckDbOfflineDist $target.Path $target.Label
}
