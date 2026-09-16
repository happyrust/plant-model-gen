# agent-browser-down.ps1
# Stop agent-browser daemon and the dedicated Chrome started by agent-browser-up.ps1.
#
# Usage:
#   pwsh scripts\agent-browser-down.ps1
#   pwsh scripts\agent-browser-down.ps1 -ProfileDir "<custom dir>"  # match agent-browser-up.ps1

[CmdletBinding()]
param(
    [string]$ProfileDir = ""
)

if (-not $ProfileDir) {
    $ProfileDir = Join-Path $env:USERPROFILE ".agent-browser-profile"
}

Write-Host "[1/3] agent-browser close --all ..."
try { agent-browser close --all 2>&1 | Out-Null } catch {}

Write-Host "[2/3] killing agent-browser daemon ..."
Get-Process | Where-Object { $_.ProcessName -match "agent-browser" } | ForEach-Object {
    Write-Host "  kill $($_.ProcessName) ($($_.Id))"
    Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
}

Write-Host "[3/3] killing chrome processes using profile $ProfileDir ..."
$norm = ($ProfileDir -replace '\\','\\').ToLower()
Get-CimInstance Win32_Process -Filter "Name='chrome.exe'" -ErrorAction SilentlyContinue | ForEach-Object {
    $cmd = ($_.CommandLine | Out-String).ToLower()
    if ($cmd -match [Regex]::Escape($ProfileDir.ToLower())) {
        Write-Host "  kill chrome.exe ($($_.ProcessId))"
        Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue
    }
}

Write-Host ""
Write-Host "Done."
