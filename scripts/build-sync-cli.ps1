# Fast release build for PE/ATT sync + version anchors (no web/manifold/parquet).
# Usage: powershell -File scripts/build-sync-cli.ps1
#
# Low-memory note: the workspace's .cargo/config.toml enables the parallel rustc
# frontend (-Zthreads) and release codegen-units=16. On memory-constrained hosts
# the parallel LLVM codegen of large deps (surrealdb-core etc.) can exhaust RAM
# and crash rustc with `LLVM ERROR: out of memory` / STATUS_STACK_BUFFER_OVERRUN.
# This script first tries the fast parallel build; if it fails it automatically
# retries serially (-j 1, codegen-units=1) which fits far less memory.
# Force the serial path directly with AIOS_LOWMEM_BUILD=1.
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

$env:CARGO_INCREMENTAL = "1"

function Build-Serial {
    Write-Host "Building aios-database (sync-cli, release, LOW-MEMORY serial)..."
    cargo build --release --bin aios-database `
        --no-default-features `
        --features sync-cli `
        -j 1 `
        --config "build.rustc-wrapper=''" `
        --config "profile.release.codegen-units=1" `
        --config "target.x86_64-pc-windows-msvc.rustflags=['-C','link-args=/STACK:16777216','-C','linker=rust-lld','-C','linker-flavor=lld-link']"
    return $LASTEXITCODE
}

function Build-Fast {
    Write-Host "Building aios-database (sync-cli, release, parallel)..."
    cargo build --release --bin aios-database `
        --no-default-features `
        --features sync-cli `
        --config "build.rustc-wrapper=''"
    return $LASTEXITCODE
}

if ($env:AIOS_LOWMEM_BUILD -eq "1") {
    $code = Build-Serial
} else {
    $code = Build-Fast
    if ($code -ne 0) {
        Write-Warning "Parallel build failed (exit $code) - likely OOM; retrying serial low-memory build..."
        $code = Build-Serial
    }
}

if ($code -ne 0) { exit $code }
Get-Item "target\release\aios-database.exe" | Format-List FullName, Length, LastWriteTime
