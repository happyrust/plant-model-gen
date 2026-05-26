param(
    [int]$Port = 3100,
    [string]$Config = "db_options/DbOption",
    [switch]$NoBrowser,
    [switch]$Wait
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

function Resolve-InstallRoot {
    $scriptDir = $PSScriptRoot
    if (Test-Path -LiteralPath (Join-Path $scriptDir "bin/web_server.exe")) {
        return $scriptDir
    }
    return [System.IO.Path]::GetFullPath((Join-Path $scriptDir "../.."))
}

function Wait-HttpOk([string]$Url, [int]$TimeoutSec) {
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        try {
            $resp = Invoke-WebRequest -Uri $Url -UseBasicParsing -TimeoutSec 3
            if ([int]$resp.StatusCode -ge 200 -and [int]$resp.StatusCode -lt 500) {
                return $true
            }
        } catch {
            Start-Sleep -Milliseconds 500
        }
    }
    return $false
}

$Root = Resolve-InstallRoot
$WebServer = Join-Path $Root "bin/web_server.exe"
$LogDir = Join-Path $Root "logs"
$ViewerUrl = "http://127.0.0.1:$Port/viewer/"

if (-not (Test-Path -LiteralPath $WebServer -PathType Leaf)) {
    throw "web_server.exe not found: $WebServer"
}
New-Item -ItemType Directory -Force -Path $LogDir | Out-Null

$env:WEB_SERVER_PORT = "$Port"
$env:Path = (Join-Path $Root "bin/surreal") + ";" + $env:Path

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$stdout = Join-Path $LogDir "web_server-$stamp.out.log"
$stderr = Join-Path $LogDir "web_server-$stamp.err.log"
$args = @("--config", $Config)

Write-Host "Starting Plant3D AIOS from $Root" -ForegroundColor Cyan
Write-Host "Viewer: $ViewerUrl" -ForegroundColor Cyan

$process = Start-Process `
    -FilePath $WebServer `
    -ArgumentList $args `
    -WorkingDirectory $Root `
    -RedirectStandardOutput $stdout `
    -RedirectStandardError $stderr `
    -WindowStyle Hidden `
    -PassThru

if (-not (Wait-HttpOk "http://127.0.0.1:$Port/api/version" 90)) {
    Write-Warning "Backend did not become ready within 90 seconds. Check logs:"
    Write-Warning "  $stdout"
    Write-Warning "  $stderr"
} elseif (-not $NoBrowser) {
    Start-Process $ViewerUrl
}

Write-Host "PID: $($process.Id)"
Write-Host "Logs: $LogDir"

if ($Wait) {
    Wait-Process -Id $process.Id
}
