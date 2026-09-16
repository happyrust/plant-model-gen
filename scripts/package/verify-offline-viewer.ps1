param(
    [string]$Root = (Get-Location).Path
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

function Get-ViewerTextFiles([string]$Dist) {
    Get-ChildItem -LiteralPath $Dist -Recurse -File -ErrorAction SilentlyContinue |
        Where-Object {
            $ext = $_.Extension.ToLowerInvariant()
            $ext -eq ".html" -or $ext -eq ".js" -or $ext -eq ".mjs" -or $ext -eq ".css"
        }
}

function Assert-NoExternalViewerLibraryUrls([string]$Dist, [string]$Label) {
    $forbidden = @(
        @{ Name = "DuckDB jsDelivr bundle"; Pattern = "https?://cdn\.jsdelivr\.net/(npm/)?@duckdb/duckdb-wasm" },
        @{ Name = "DuckDB jsDelivr helper"; Pattern = "getJsDelivrBundles" },
        @{ Name = "jsDelivr CDN"; Pattern = "https?://cdn\.jsdelivr\.net/" },
        @{ Name = "unpkg CDN"; Pattern = "https?://unpkg\.com/" },
        @{ Name = "cdnjs CDN"; Pattern = "https?://cdnjs\.cloudflare\.com/" },
        @{ Name = "esm.sh CDN"; Pattern = "https?://esm\.sh/" },
        @{ Name = "Skypack CDN"; Pattern = "https?://cdn\.skypack\.dev/" },
        @{ Name = "Google Fonts CSS"; Pattern = "https?://fonts\.googleapis\.com/" },
        @{ Name = "Google Fonts static"; Pattern = "https?://fonts\.gstatic\.com/" }
    )

    foreach ($file in (Get-ViewerTextFiles $Dist)) {
        $content = Get-Content -LiteralPath $file.FullName -Raw
        foreach ($rule in $forbidden) {
            if ($content -match $rule.Pattern) {
                throw "$($rule.Name) URL/helper found in ${Label}: $($file.FullName). Rebuild plant3d-web with offline local assets."
            }
        }
    }
}

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
        "duckdb-coi.wasm",
        "extensions/v1.5.3/wasm_eh/parquet.duckdb_extension.wasm",
        "extensions/v1.5.3/wasm_mvp/parquet.duckdb_extension.wasm",
        "extensions/v1.5.3/wasm_threads/parquet.duckdb_extension.wasm"
    )
    foreach ($file in $required) {
        $path = Join-Path $duckdbDir $file
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "DuckDB offline asset missing for ${Label}: $path"
        }
    }

    Assert-NoExternalViewerLibraryUrls $Dist $Label

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
