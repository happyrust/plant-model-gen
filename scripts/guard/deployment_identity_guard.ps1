<#
.SYNOPSIS
  Guard the deployment-vs-E3D identity independence invariant (spec 018).

.DESCRIPTION
  The deployment project name is the SOLE outward identity (database name,
  runtime/output directory, external/viewer access name). E3D source project
  names are source-only (included_projects/project_dirs/source roots) and must
  never drive outward identity.

  This guard extracts the body of each outward-identity function in
  src/web_server/managed_project_sites.rs and fails if it references the E3D
  source-name helper `site_source_project_name`. The single allowed bridge is
  `site_deployment_project_name` itself (a defined fallback when the deployment
  name is empty), which is intentionally NOT in the checked set.
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

$target = Join-Path $RepoRoot "src/web_server/managed_project_sites.rs"
if (-not (Test-Path $target)) {
    throw "target file not found: $target"
}

$source = Get-Content -Path $target -Raw

# Outward-identity functions that MUST NOT use the E3D source-name helper.
$outwardFns = @(
    "site_runtime_database_name",
    "site_project_tree_dir",
    "build_viewer_url"
)

$forbiddenSymbol = "site_source_project_name"

function Get-RustFnBody {
    param([string]$Text, [string]$FnName)
    $sig = [regex]::Match($Text, "fn\s+$([regex]::Escape($FnName))\s*\(")
    if (-not $sig.Success) { return $null }
    $braceStart = $Text.IndexOf('{', $sig.Index)
    if ($braceStart -lt 0) { return $null }
    $depth = 0
    for ($i = $braceStart; $i -lt $Text.Length; $i++) {
        $ch = $Text[$i]
        if ($ch -eq '{') { $depth++ }
        elseif ($ch -eq '}') {
            $depth--
            if ($depth -eq 0) {
                return $Text.Substring($braceStart, $i - $braceStart + 1)
            }
        }
    }
    return $null
}

function Get-LineNumber {
    param([string]$Text, [int]$Index)
    if ($Index -lt 0) { return 0 }
    return ($Text.Substring(0, $Index) -split "`n").Count
}

$failures = @()
$missing = @()

# Ensure the canonical outward helper exists.
if (-not [regex]::IsMatch($source, "fn\s+site_deployment_project_name\s*\(")) {
    $missing += "site_deployment_project_name (canonical outward-identity helper) not found"
}

foreach ($fn in $outwardFns) {
    $body = Get-RustFnBody -Text $source -FnName $fn
    if ($null -eq $body) {
        $missing += "$fn not found"
        continue
    }
    $idx = $body.IndexOf($forbiddenSymbol)
    if ($idx -ge 0) {
        $absIndex = $source.IndexOf($body) + $idx
        $line = Get-LineNumber -Text $source -Index $absIndex
        $failures += [pscustomobject]@{ Fn = $fn; Line = $line }
    }
}

if ($missing.Count -gt 0) {
    Write-Host "deployment identity guard could not verify (structure changed):" -ForegroundColor Red
    foreach ($m in $missing) { Write-Host "  $m" }
    exit 2
}

if ($failures.Count -gt 0) {
    Write-Host "deployment identity guard FAILED: outward-identity function uses E3D source name." -ForegroundColor Red
    foreach ($f in $failures) {
        Write-Host ("  {0} references {1} at {2}:{3}" -f $f.Fn, $forbiddenSymbol, $target, $f.Line)
    }
    Write-Host "Outward identity must derive from site_deployment_project_name, not site_source_project_name." -ForegroundColor Yellow
    exit 1
}

Write-Host "[PASS] deployment identity guard passed ($($outwardFns.Count) outward-identity functions verified)" -ForegroundColor Green
