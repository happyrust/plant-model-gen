<#
.SYNOPSIS
  Sweep orphaned aios-database `serve` sidecar processes scoped to one admin_sidecars root.

.DESCRIPTION
  Spec 017 (sidecar-process-reaper) maintenance helper. Enumerates `aios-database`
  processes running `serve` whose `--runtime-dir` is under the given -Root, and
  terminates them. Strictly scoped: processes whose runtime dir is outside -Root
  (for example a packaged release service or a sibling repository) are never touched.

.PARAMETER Root
  The admin_sidecars root to scope the sweep to. Default: ./runtime/admin_sidecars
  relative to the current working directory.

.PARAMETER DryRun
  List the matching sidecars without terminating them.

.EXAMPLE
  pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/cleanup_orphan_sidecars.ps1 -DryRun

.EXAMPLE
  pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/cleanup_orphan_sidecars.ps1 -Root .\runtime\admin_sidecars
#>

[CmdletBinding()]
param(
    [string]$Root = ".\runtime\admin_sidecars",
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

function Resolve-RootPath {
    param([string]$Path)
    $full = if ([System.IO.Path]::IsPathRooted($Path)) { $Path } else { Join-Path (Get-Location).Path $Path }
    # Normalize without requiring the directory to exist.
    $full = [System.IO.Path]::GetFullPath($full)
    return $full
}

function Normalize-PathStr {
    param([string]$Path)
    if (-not $Path) { return "" }
    return ($Path -replace '\\', '/').ToLowerInvariant().TrimEnd('/')
}

function Test-PathUnderRoot {
    param(
        [string]$PathNorm,
        [string]$RootNorm
    )
    if (-not $PathNorm -or -not $RootNorm) { return $false }
    if ($PathNorm -eq $RootNorm) { return $true }
    $rootPrefix = if ($RootNorm.EndsWith('/')) { $RootNorm } else { "$RootNorm/" }
    return $PathNorm.StartsWith($rootPrefix)
}

function Format-DisplayValue {
    param([string]$Value)
    if ([string]::IsNullOrWhiteSpace($Value)) { return "-" }
    return $Value
}

function Get-ArgValue {
    param([string]$CommandLine, [string]$Flag)
    # Matches: --flag "value with spaces"  OR  --flag value
    $pattern = [regex]::Escape($Flag) + '\s+(?:"([^"]+)"|([^\s"]+))'
    $m = [regex]::Match($CommandLine, $pattern)
    if (-not $m.Success) { return $null }
    if ($m.Groups[1].Success) { return $m.Groups[1].Value }
    return $m.Groups[2].Value
}

$rootFull = Resolve-RootPath -Path $Root
$rootNorm = Normalize-PathStr -Path $rootFull

Write-Host "== Sidecar orphan sweep ==" -ForegroundColor Cyan
Write-Host "scope_root : $rootFull"
Write-Host "mode       : $(if ($DryRun) { 'DRY-RUN (no kills)' } else { 'KILL' })"
Write-Host ""

$procs = Get-CimInstance Win32_Process -Filter "Name='aios-database.exe'" -ErrorAction SilentlyContinue
$matched = New-Object System.Collections.Generic.List[object]

foreach ($p in $procs) {
    $cmd = $p.CommandLine
    if (-not $cmd) { continue }
    if ($cmd -notmatch '(^|\s)serve(\s|$)') { continue }
    $runtimeDir = Get-ArgValue -CommandLine $cmd -Flag '--runtime-dir'
    if (-not $runtimeDir) { continue }
    $rdNorm = Normalize-PathStr -Path $runtimeDir
    if (-not (Test-PathUnderRoot -PathNorm $rdNorm -RootNorm $rootNorm)) { continue }
    $siteKey = Get-ArgValue -CommandLine $cmd -Flag '--site-key'
    $matched.Add([pscustomobject]@{
        ProcessId   = $p.ProcessId
        SiteKey     = $siteKey
        RuntimeDir  = $runtimeDir
    })
}

if ($matched.Count -eq 0) {
    Write-Host "No orphan sidecars found under scope_root." -ForegroundColor Green
    exit 0
}

Write-Host "Matched $($matched.Count) sidecar(s) under scope_root:" -ForegroundColor Yellow
foreach ($m in $matched) {
    $siteKey = Format-DisplayValue -Value $m.SiteKey
    Write-Host ("  PID {0,-8} {1,-40} {2}" -f $m.ProcessId, $siteKey, $m.RuntimeDir)
}
Write-Host ""

if ($DryRun) {
    Write-Host "DRY-RUN: nothing terminated. Re-run without -DryRun to kill." -ForegroundColor Cyan
    exit 0
}

$killed = 0
foreach ($m in $matched) {
    try {
        Stop-Process -Id $m.ProcessId -Force -ErrorAction Stop
        $killed++
        $siteKey = Format-DisplayValue -Value $m.SiteKey
        Write-Host ("  killed PID {0} ({1})" -f $m.ProcessId, $siteKey) -ForegroundColor Green
    } catch {
        Write-Host ("  failed PID {0}: {1}" -f $m.ProcessId, $_.Exception.Message) -ForegroundColor Red
    }
}

Write-Host ""
Write-Host "Terminated $killed / $($matched.Count) sidecar(s) under scope_root." -ForegroundColor Cyan
