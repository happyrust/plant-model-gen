#!/usr/bin/env pwsh
# Verify ModelWriterBackend trait 契约（基于 RecordingBackend mock 后端）。
#
# 前置：
#   - 已完成 P2 的 T2.1 / T2.2
#   - cargo + nightly toolchain + NASM 在 PATH
#
# 用法：
#   pwsh -NoProfile -File docs/plans/2026-05-09-model-write-trait-followup/verify-mock.ps1
#   pwsh -NoProfile -File ... -VerboseRun         # 显示 cargo 完整输出
#
# 退出码：
#   0   — 通过
#   1   — binary build 失败 / trait 方法返回 Err
#   2   — snapshot 调用计数不符
#   3   — snapshot 顺序不符

[CmdletBinding()]
param(
    [string]$WorkdirPath = "$PSScriptRoot/../../../.worktrees/model-persistence-trait",
    [switch]$VerboseRun
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $WorkdirPath)) {
    Write-Host "[verify-mock] FAIL: worktree 不存在: $WorkdirPath"
    exit 1
}

# 准备 NASM PATH（如果默认 PATH 里没有，aws-lc-sys 需要）
if (-not (Get-Command nasm -ErrorAction SilentlyContinue)) {
    $candidate = "C:\Program Files\NASM"
    if (Test-Path "$candidate\nasm.exe") {
        $env:PATH = "$candidate;" + $env:PATH
        Write-Host "[verify-mock] NASM 已临时加入 PATH: $candidate"
    } else {
        Write-Host "[verify-mock] 警告：未找到 nasm，可能导致 aws-lc-sys 编译失败"
    }
}

# 准备 cmake PATH（如果默认 PATH 里没有，manifold-rs 需要）
if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
    $candidate = "C:\Program Files\CMake\bin"
    if (Test-Path "$candidate\cmake.exe") {
        $env:PATH = "$candidate;" + $env:PATH
        Write-Host "[verify-mock] cmake 已临时加入 PATH: $candidate"
    } else {
        Write-Host "[verify-mock] 警告：未找到 cmake，可能导致 manifold-rs 编译失败"
    }
}

# 启用 git-fetch-with-cli 避免 surrealdb 依赖 fetch 偶发性 HTTP 412
$env:CARGO_NET_GIT_FETCH_WITH_CLI = "true"

Push-Location $WorkdirPath
try {
    $started = Get-Date
    Write-Host "[verify-mock] 编译 + 运行 verify_model_writer_trait..."

    if ($VerboseRun) {
        cargo run --bin verify_model_writer_trait --features model-writer-mock 2>&1 | Tee-Object -Variable runOutput
    } else {
        $runOutput = cargo run --bin verify_model_writer_trait --features model-writer-mock 2>&1
    }
    $exit = $LASTEXITCODE
    $elapsed = [int]((Get-Date) - $started).TotalSeconds

    if ($exit -ne 0) {
        Write-Host "[verify-mock] FAIL: exit=$exit, elapsed=${elapsed}s"
        if (-not $VerboseRun) {
            $runOutput | Select-Object -Last 30 | ForEach-Object { Write-Host "  $_" }
        }
        exit $exit
    }

    Write-Host "[verify-mock] PASS — elapsed=${elapsed}s"
    exit 0
} finally {
    Pop-Location
}
