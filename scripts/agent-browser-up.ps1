# agent-browser-up.ps1
# Start a dedicated Chrome with remote debugging, then connect agent-browser to it.
# Idempotent: if Chrome on the chosen port is already up, just reconnect.
#
# Usage:
#   pwsh scripts\agent-browser-up.ps1                 # default port 9222, headed
#   pwsh scripts\agent-browser-up.ps1 -Port 9333      # custom port
#   pwsh scripts\agent-browser-up.ps1 -Headless       # headless=new
#   pwsh scripts\agent-browser-up.ps1 -ChromePath "<full path to chrome.exe>"
#
# Stop / restart: scripts\agent-browser-down.ps1

[CmdletBinding()]
param(
    [int]$Port = 9222,
    [string]$ChromePath = "",
    [string]$ProfileDir = "",
    [switch]$Headless,
    [switch]$Force
)

$ErrorActionPreference = "Stop"

function Resolve-ChromePath {
    param([string]$Hint)
    if ($Hint -and (Test-Path $Hint)) { return (Resolve-Path $Hint).Path }
    $candidates = @(
        "C:\Program Files\Google\Chrome\Application\chrome.exe",
        "C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        "$env:LOCALAPPDATA\Google\Chrome\Application\chrome.exe",
        "C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
        "C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
    )
    foreach ($c in $candidates) { if (Test-Path $c) { return $c } }
    throw "No Chrome/Edge/Brave found. Pass -ChromePath explicitly."
}

function Test-CdpUp {
    param([int]$Port)
    try {
        $resp = Invoke-WebRequest -Uri "http://127.0.0.1:$Port/json/version" -TimeoutSec 2 -UseBasicParsing -ErrorAction Stop
        return $resp.StatusCode -eq 200
    } catch { return $false }
}

if (-not $ProfileDir) {
    $ProfileDir = Join-Path $env:USERPROFILE ".agent-browser-profile"
}
if (-not (Test-Path $ProfileDir)) {
    New-Item -Path $ProfileDir -ItemType Directory -Force | Out-Null
}

if ($Force) {
    Write-Host "[force] killing existing agent-browser daemon ..."
    Get-Process | Where-Object { $_.ProcessName -match "agent-browser" } | Stop-Process -Force -ErrorAction SilentlyContinue
}

if (Test-CdpUp -Port $Port) {
    Write-Host "[ok] CDP already up on port $Port — reusing."
} else {
    $chrome = Resolve-ChromePath -Hint $ChromePath
    Write-Host "[chrome] $chrome"
    Write-Host "[profile] $ProfileDir"
    Write-Host "[port] $Port"

    $chromeArgs = @(
        "--remote-debugging-port=$Port",
        "--user-data-dir=`"$ProfileDir`"",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-features=GlobalMediaControls,MediaRouter"
    )
    if ($Headless) { $chromeArgs += "--headless=new" }
    $chromeArgs += "about:blank"

    Start-Process -FilePath $chrome -ArgumentList $chromeArgs -WindowStyle Minimized | Out-Null

    $deadline = (Get-Date).AddSeconds(15)
    while (-not (Test-CdpUp -Port $Port)) {
        if ((Get-Date) -gt $deadline) {
            throw "Chrome did not expose CDP on port $Port within 15s."
        }
        Start-Sleep -Milliseconds 300
    }
    Write-Host "[ok] Chrome CDP up on port $Port."
}

Write-Host ""
Write-Host "[agent-browser] connecting ..."
agent-browser connect $Port
if ($LASTEXITCODE -ne 0) { throw "agent-browser connect failed (exit $LASTEXITCODE)" }

Write-Host ""
Write-Host "Ready. Try:"
Write-Host "  agent-browser open https://example.com"
Write-Host "  agent-browser snapshot"
Write-Host "  agent-browser screenshot page.png"
Write-Host ""
Write-Host "Stop: pwsh scripts\agent-browser-down.ps1"
