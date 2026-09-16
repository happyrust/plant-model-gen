param(
    [string]$TaskName = "Plant3D-AIOS",
    [int]$Port = 3100,
    [ValidateSet("auto", "on", "off")]
    [string]$EnableNginx = "on",
    [string]$ViewerHost = "",
    [int]$ViewerPort = 80,
    [switch]$RequireNginx,
    [switch]$RunNow,
    [switch]$Uninstall
)

$ErrorActionPreference = "Stop"

function Resolve-InstallRoot {
    $scriptDir = $PSScriptRoot
    if (Test-Path -LiteralPath (Join-Path $scriptDir "bin/web_server.exe")) {
        return $scriptDir
    }
    return [System.IO.Path]::GetFullPath((Join-Path $scriptDir "../.."))
}

function Assert-Administrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw "Please run this script from an elevated PowerShell session."
    }
}

Assert-Administrator

if ($Uninstall) {
    $existing = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
    if ($existing) {
        Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false
        Write-Host "Removed scheduled task: $TaskName" -ForegroundColor Green
    } else {
        Write-Host "Scheduled task not found: $TaskName" -ForegroundColor Yellow
    }
    exit 0
}

$Root = Resolve-InstallRoot
$StartScript = Join-Path $Root "start-plant3d.ps1"
if (-not (Test-Path -LiteralPath $StartScript -PathType Leaf)) {
    throw "start script not found: $StartScript"
}

$effectiveRequireNginx = $RequireNginx -or $EnableNginx -eq "on"
$actionArgs = "-NoProfile -ExecutionPolicy Bypass -File `"$StartScript`" -Port $Port -NoBrowser -EnableNginx `"$EnableNginx`" -ViewerPort $ViewerPort"
if ($ViewerHost.Trim()) {
    $actionArgs += " -ViewerHost `"$ViewerHost`""
}
if ($effectiveRequireNginx) {
    $actionArgs += " -RequireNginx"
}
$action = New-ScheduledTaskAction -Execute "powershell.exe" -Argument $actionArgs -WorkingDirectory $Root
$trigger = New-ScheduledTaskTrigger -AtLogOn
$settings = New-ScheduledTaskSettingsSet `
    -AllowStartIfOnBatteries `
    -DontStopIfGoingOnBatteries `
    -ExecutionTimeLimit ([TimeSpan]::Zero) `
    -RestartCount 3 `
    -RestartInterval (New-TimeSpan -Minutes 1)
$principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive -RunLevel Highest

Register-ScheduledTask `
    -TaskName $TaskName `
    -Action $action `
    -Trigger $trigger `
    -Settings $settings `
    -Principal $principal `
    -Description "Start Plant3D AIOS local web_server and bundled SurrealDB" `
    -Force | Out-Null

Write-Host "Registered scheduled task: $TaskName" -ForegroundColor Green
Write-Host "Install root: $Root"
Write-Host "Fallback Viewer URL: http://127.0.0.1:$Port/viewer/"
Write-Host "Nginx mode: $EnableNginx"
Write-Host "Nginx required: $effectiveRequireNginx"

if ($RunNow) {
    Start-ScheduledTask -TaskName $TaskName
    Write-Host "Started scheduled task: $TaskName" -ForegroundColor Green
}
