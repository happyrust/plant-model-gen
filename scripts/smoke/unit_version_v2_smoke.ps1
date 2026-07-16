# specs/023 B5: unit_versions_v2 smoke
# Usage:
#   pwsh scripts/smoke/unit_version_v2_smoke.ps1
#   pwsh scripts/smoke/unit_version_v2_smoke.ps1 -Bin path\to\aios-database.exe
#
# Prefer E: target dir when D: is low on space:
#   $env:CARGO_TARGET_DIR='E:\cargo-target\plant-model-gen'

param(
    [string]$Bin = "",
    [string]$WorkDir = ""
)

$ErrorActionPreference = "Stop"
$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
Set-Location $repoRoot

if (-not $Bin) {
    $candidates = @(
        "E:\cargo-target\plant-model-gen\debug\aios-database.exe",
        "target\debug\aios-database.exe",
        "target\debug\aios_database.exe"
    )
    foreach ($c in $candidates) {
        if (Test-Path $c) { $Bin = (Resolve-Path $c).Path; break }
    }
}

if (-not $Bin) {
    Write-Host "Building aios-database with model-version-ducklake on E: target..."
    $env:CARGO_TARGET_DIR = "E:\cargo-target\plant-model-gen"
    New-Item -ItemType Directory -Force -Path $env:CARGO_TARGET_DIR | Out-Null
    cargo build -p aios-database --features model-version-ducklake --bin aios-database --config "build.rustc-wrapper=''" --config "source.crates-io.replace-with='tuna'"
    $Bin = (Resolve-Path "E:\cargo-target\plant-model-gen\debug\aios-database.exe").Path
}

if (-not $WorkDir) {
    $WorkDir = Join-Path $env:TEMP ("aios-unit-v2-smoke-" + [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())
}

Write-Host "Bin=$Bin"
Write-Host "WorkDir=$WorkDir"

& $Bin model-version unit-v2-smoke --work-dir $WorkDir --json
if ($LASTEXITCODE -ne 0) {
    throw "unit-v2-smoke failed with exit $LASTEXITCODE"
}
Write-Host "unit-v2-smoke OK"
