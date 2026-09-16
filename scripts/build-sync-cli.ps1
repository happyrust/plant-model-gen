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

# sccache 与增量互斥：全仓库统一走 sccache（见 .cargo/config.toml），这里显式关闭增量。
$env:CARGO_INCREMENTAL = "0"

function Build-Serial {
    Write-Host "Building aios-database (sync-cli, release, LOW-MEMORY serial)..."
    cargo build --release --bin aios-database `
        --no-default-features `
        --features sync-cli `
        -j 1 `
        --config "profile.release.codegen-units=1" `
        --config "target.x86_64-pc-windows-msvc.rustflags=['-C','link-args=/STACK:16777216','-C','linker=rust-lld','-C','linker-flavor=lld-link']"
    return $LASTEXITCODE
}

function Build-Fast {
    Write-Host "Building aios-database (sync-cli, release, parallel)..."
    cargo build --release --bin aios-database `
        --no-default-features `
        --features sync-cli
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
$targetDir = if ([string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) {
    Join-Path (Get-Location) "target"
} elseif ([System.IO.Path]::IsPathRooted($env:CARGO_TARGET_DIR)) {
    $env:CARGO_TARGET_DIR
} else {
    Join-Path (Get-Location) $env:CARGO_TARGET_DIR
}
Get-Item (Join-Path $targetDir "release\aios-database.exe") |
    Format-List FullName, Length, LastWriteTime
