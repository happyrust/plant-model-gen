param(
    [int]$Port = 3100,
    [int]$PortCount = 10,
    [int]$DbPort = 8020,
    [int]$ViewerPort = 80,
    [string]$TaskName = "Plant3D-AIOS",
    [switch]$KeepNginx,
    [switch]$KeepSurreal,
    [switch]$SkipScheduledTask,
    [switch]$ForcePortKill
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

function Resolve-InstallRoot {
    $scriptDir = $PSScriptRoot
    if (Test-Path -LiteralPath (Join-Path $scriptDir "bin/web_server.exe")) {
        return [System.IO.Path]::GetFullPath($scriptDir)
    }
    return [System.IO.Path]::GetFullPath((Join-Path $scriptDir "../.."))
}

function Get-ProcessImagePath([System.Diagnostics.Process]$Process) {
    try {
        if ($Process.Path) {
            return [System.IO.Path]::GetFullPath($Process.Path)
        }
    } catch {}
    try {
        if ($Process.MainModule -and $Process.MainModule.FileName) {
            return [System.IO.Path]::GetFullPath($Process.MainModule.FileName)
        }
    } catch {}
    return ""
}

function Test-PathUnderRoot([string]$Path, [string]$Root) {
    if (-not $Path) { return $false }
    return $Path.StartsWith($Root, [System.StringComparison]::OrdinalIgnoreCase)
}

function Stop-MatchingProcess([string]$ExePath, [string]$Label) {
    if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) {
        return 0
    }

    $exeName = [System.IO.Path]::GetFileNameWithoutExtension($ExePath)
    $expected = [System.IO.Path]::GetFullPath($ExePath)
    $stopped = 0
    foreach ($proc in Get-Process -Name $exeName -ErrorAction SilentlyContinue) {
        $actual = Get-ProcessImagePath $proc
        if (-not $actual.Equals($expected, [System.StringComparison]::OrdinalIgnoreCase)) {
            continue
        }
        Write-Host "Stopping $Label PID=$($proc.Id) ($actual)"
        Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
        $stopped += 1
    }
    return $stopped
}

function Stop-ProcessNameUnderRoot([string]$ExeName, [string]$Label, [string]$Root) {
    $stopped = 0
    foreach ($proc in Get-Process -Name $ExeName -ErrorAction SilentlyContinue) {
        $actual = Get-ProcessImagePath $proc
        if (-not (Test-PathUnderRoot $actual $Root)) {
            continue
        }
        Write-Host "Stopping $Label PID=$($proc.Id) ($actual)"
        Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
        $stopped += 1
    }
    return $stopped
}

function Get-ListeningPidsOnPort([int]$TargetPort) {
    $pids = New-Object System.Collections.Generic.HashSet[int]
    $output = & netstat -ano 2>$null
    foreach ($line in $output) {
        if ($line -notmatch "LISTENING") { continue }
        if ($line -notmatch [regex]::Escape(":$TargetPort")) { continue }
        $parts = ($line -split "\s+") | Where-Object { $_ }
        if ($parts.Count -lt 1) { continue }
        $pidText = $parts[-1]
        if ($pidText -match "^\d+$") {
            [void]$pids.Add([int]$pidText)
        }
    }
    return @($pids)
}

function Stop-ListenerOnPort([int]$TargetPort, [string]$Root) {
    $stopped = 0
    foreach ($pidValue in Get-ListeningPidsOnPort $TargetPort) {
        if ($pidValue -le 0) { continue }
        $proc = Get-Process -Id $pidValue -ErrorAction SilentlyContinue
        if (-not $proc) { continue }
        $path = Get-ProcessImagePath $proc
        if (-not $ForcePortKill -and -not (Test-PathUnderRoot $path $Root)) {
            Write-Warning "Port $TargetPort is used by PID=$pidValue outside this package ($path). Skipping. Use -ForcePortKill to stop it."
            continue
        }
        Write-Host "Stopping listener on port $TargetPort PID=$pidValue ($($proc.ProcessName))"
        Stop-Process -Id $pidValue -Force -ErrorAction SilentlyContinue
        $stopped += 1
    }
    return $stopped
}

function Stop-PackageScheduledTask([string]$Name) {
    if (-not (Get-Command Get-ScheduledTask -ErrorAction SilentlyContinue)) {
        return 0
    }
    $task = Get-ScheduledTask -TaskName $Name -ErrorAction SilentlyContinue
    if (-not $task) {
        return 0
    }
    if ($task.State -eq "Running") {
        Write-Host "Stopping scheduled task: $Name"
        Stop-ScheduledTask -TaskName $Name -ErrorAction SilentlyContinue
        return 1
    }
    Write-Host "Scheduled task is not running: $Name"
    return 0
}

$Root = Resolve-InstallRoot
$stopped = 0

Write-Host "Stopping Plant3D AIOS from $Root" -ForegroundColor Cyan

if (-not $SkipScheduledTask) {
    try {
        $stopped += Stop-PackageScheduledTask $TaskName
    } catch {
        Write-Warning "Could not stop scheduled task ${TaskName}: $($_.Exception.Message)"
    }
}

$stopped += Stop-MatchingProcess (Join-Path $Root "bin/web_server.exe") "web_server.exe"
$stopped += Stop-MatchingProcess (Join-Path $Root "bin/aios-database.exe") "aios-database.exe"
$stopped += Stop-ProcessNameUnderRoot "aios-database" "aios-database sidecar" $Root
if (-not $KeepNginx) {
    $stopped += Stop-MatchingProcess (Join-Path $Root "bin/nginx/nginx.exe") "nginx.exe"
}
if (-not $KeepSurreal) {
    $stopped += Stop-MatchingProcess (Join-Path $Root "bin/surreal/surreal.exe") "surreal.exe"
}

for ($candidate = $Port; $candidate -lt ($Port + $PortCount); $candidate++) {
    $stopped += Stop-ListenerOnPort $candidate $Root
}
if ($DbPort -gt 0) {
    $stopped += Stop-ListenerOnPort $DbPort $Root
}
if ($ViewerPort -gt 0) {
    $stopped += Stop-ListenerOnPort $ViewerPort $Root
}

Start-Sleep -Milliseconds 500

if ($stopped -eq 0) {
    Write-Host "No Plant3D AIOS processes were stopped." -ForegroundColor Yellow
} else {
    Write-Host "Stopped $stopped process(es)/listener(s)." -ForegroundColor Green
}
