<#
.SYNOPSIS
  Guard that web_server does not re-enter E3D/db parsing ownership.

.DESCRIPTION
  The admin web_server is the BFF/control plane. E3D project scanning,
  db file header parsing, dbnum inference, dependency closure, and
  parse/generate execution belong to the aios-database sidecar.

  This guard scans src/web_server for legacy parsing symbols that would
  violate that boundary.
#>

[CmdletBinding()]
param(
    [string]$RepoRoot = ""
)

$ErrorActionPreference = "Stop"

if (-not $RepoRoot) {
    $RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "../..")
} else {
    $RepoRoot = Resolve-Path $RepoRoot
}

$webServerDir = Join-Path $RepoRoot "src/web_server"
if (-not (Test-Path $webServerDir)) {
    throw "web_server directory not found: $webServerDir"
}

$forbidden = @(
    "parse_file_basic_info",
    "resolve_dbnum_from_db_file",
    "resolve_dbnum_from_db_file_roots",
    "resolve_included_db_files_detailed",
    "collect_project_dbnums",
    "related_db_file_names_precise",
    "parse_file_ref0"
)

$matches = @()
foreach ($pattern in $forbidden) {
    $found = Select-String -Path (Join-Path $webServerDir "*.rs") -Pattern $pattern -SimpleMatch -ErrorAction SilentlyContinue
    if ($found) {
        $matches += $found
    }
}

if ($matches.Count -gt 0) {
    Write-Host "web_server parse boundary guard failed:" -ForegroundColor Red
    foreach ($match in $matches) {
        Write-Host ("{0}:{1}: {2}" -f $match.Path, $match.LineNumber, $match.Line.Trim())
    }
    exit 1
}

Write-Host "[PASS] web_server parse boundary guard passed" -ForegroundColor Green
