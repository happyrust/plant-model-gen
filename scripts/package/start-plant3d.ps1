param(
    [int]$Port = 3100,
    [string]$Config = "db_options/DbOption",
    [switch]$NoBrowser,
    [switch]$Wait,
    [switch]$NoPortFallback,
    [ValidateSet("auto", "on", "off")]
    [string]$EnableNginx = "on",
    [string]$ViewerHost = "",
    [int]$ViewerPort = 80,
    [switch]$RequireNginx
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

function Invoke-LocalHttpGet([string]$Url, [int]$TimeoutSec = 3) {
    $handler = [System.Net.Http.HttpClientHandler]::new()
    $handler.UseProxy = $false
    $client = [System.Net.Http.HttpClient]::new($handler)
    $client.Timeout = [TimeSpan]::FromSeconds($TimeoutSec)
    try {
        return $client.GetAsync($Url).GetAwaiter().GetResult()
    } finally {
        $client.Dispose()
        $handler.Dispose()
    }
}

function Test-HttpOk([string]$Url) {
    $curl = Get-Command curl.exe -ErrorAction SilentlyContinue
    if ($curl) {
        $null = & $curl.Source --noproxy "*" --silent --fail --max-time 2 --output NUL $Url 2>$null
        return $LASTEXITCODE -eq 0
    }

    try {
        $resp = Invoke-LocalHttpGet $Url 2
        try {
            return ([int]$resp.StatusCode -ge 200 -and [int]$resp.StatusCode -lt 500)
        } finally {
            $resp.Dispose()
        }
    } catch {
        return $false
    }
}

function Wait-HttpOk([string]$Url, [int]$TimeoutSec, [System.Diagnostics.Process]$Process = $null) {
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        if ($Process -and $Process.HasExited) {
            return $false
        }
        if (Test-HttpOk $Url) {
            return $true
        }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

function Test-TcpOpen([int]$Port) {
    $client = [System.Net.Sockets.TcpClient]::new()
    try {
        $task = $client.ConnectAsync("127.0.0.1", $Port)
        if (-not $task.Wait(700)) {
            return $false
        }
        return $client.Connected
    } catch {
        return $false
    } finally {
        $client.Close()
    }
}

function Get-ListeningPidsOnPort([int]$Port) {
    $pattern = ":$Port"
    $pids = New-Object System.Collections.Generic.HashSet[int]
    $output = & netstat -ano 2>$null
    foreach ($line in $output) {
        if ($line -notmatch "LISTENING") { continue }
        if ($line -notmatch [regex]::Escape($pattern)) { continue }
        $parts = ($line -split "\s+") | Where-Object { $_ }
        if ($parts.Count -lt 1) { continue }
        $pidText = $parts[-1]
        if ($pidText -match "^\d+$") {
            [void]$pids.Add([int]$pidText)
        }
    }
    return @($pids)
}

function Test-PortBound([int]$Port) {
    if ((Get-ListeningPidsOnPort $Port).Count -gt 0) {
        return $true
    }
    return Test-TcpOpen $Port
}

function Test-PortBindable([int]$Port) {
    $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Any, $Port)
    try {
        $listener.Start()
        return $true
    } catch {
        return $false
    } finally {
        try { $listener.Stop() } catch {}
    }
}

function Clear-PortListeners([int]$Port) {
    $cleared = @()
    foreach ($listenerPid in (Get-ListeningPidsOnPort $Port)) {
        if ($listenerPid -le 0) { continue }
        $proc = Get-Process -Id $listenerPid -ErrorAction SilentlyContinue
        if (-not $proc) { continue }
        Write-Warning "Stopping process $($proc.ProcessName) (PID=$listenerPid) on port $Port..."
        Stop-Process -Id $listenerPid -Force -ErrorAction SilentlyContinue
        $cleared += $listenerPid
    }
    if ($cleared.Count -gt 0) {
        Start-Sleep -Milliseconds 500
    }
    return $cleared
}

function Get-LocalIpAddress {
    try {
        $socket = [System.Net.Sockets.Socket]::new([System.Net.Sockets.AddressFamily]::InterNetwork, [System.Net.Sockets.SocketType]::Dgram, [System.Net.Sockets.ProtocolType]::Udp)
        try {
            $socket.Connect("8.8.8.8", 80)
            return ([System.Net.IPEndPoint]$socket.LocalEndPoint).Address.ToString()
        } finally {
            $socket.Dispose()
        }
    } catch {
        return "127.0.0.1"
    }
}

function Resolve-NginxBin([string]$Root) {
    $candidates = @()
    if ($env:AIOS_NGINX_BIN) { $candidates += $env:AIOS_NGINX_BIN }
    $candidates += @(
        (Join-Path $Root "bin/nginx/nginx.exe"),
        (Join-Path $Root "nginx/nginx.exe"),
        "C:\nginx\nginx.exe",
        "D:\nginx\nginx.exe"
    )
    foreach ($candidate in $candidates) {
        if ($candidate -and (Test-Path -LiteralPath $candidate -PathType Leaf)) {
            return [System.IO.Path]::GetFullPath($candidate)
        }
    }
    return $null
}

function Enable-PackageNginxIfAvailable([string]$Root) {
    $effectiveRequireNginx = $RequireNginx -or $EnableNginx -eq "on"
    if (-not $effectiveRequireNginx) {
        Remove-Item Env:\AIOS_REQUIRE_NGINX -ErrorAction SilentlyContinue
    }
    if ($EnableNginx -eq "off") {
        if ($effectiveRequireNginx) {
            throw "-RequireNginx cannot be used with -EnableNginx off"
        }
        return $null
    }

    $staticRoot = Join-Path $Root "viewer-root"
    if (-not (Test-Path -LiteralPath (Join-Path $staticRoot "index.html") -PathType Leaf)) {
        $message = "Nginx customer viewer root not found: $staticRoot"
        if ($effectiveRequireNginx) { throw $message }
        Write-Warning "$message; using /viewer/ fallback."
        return $null
    }

    $nginxBin = Resolve-NginxBin $Root
    if (-not $nginxBin) {
        $message = "nginx.exe not found. Provide bin/nginx/nginx.exe or set AIOS_NGINX_BIN."
        if ($effectiveRequireNginx) { throw $message }
        Write-Warning "$message Using /viewer/ fallback."
        return $null
    }

    $hostValue = $ViewerHost.Trim()
    if (-not $hostValue) {
        $hostValue = Get-LocalIpAddress
    }
    $baseUrl = if ($ViewerPort -eq 80) {
        "http://$hostValue"
    } else {
        "http://$hostValue`:$ViewerPort"
    }

    $env:AIOS_NGINX_BIN = $nginxBin
    if (-not $env:AIOS_NGINX_ROOT) {
        $env:AIOS_NGINX_ROOT = Join-Path $Root "runtime/nginx"
    }
    $env:AIOS_VIEWER_STATIC_ROOT = $staticRoot
    if (-not $env:AIOS_VIEWER_BASE_URL) {
        $env:AIOS_VIEWER_BASE_URL = $baseUrl
    }
    if ($effectiveRequireNginx) {
        $env:AIOS_REQUIRE_NGINX = "1"
    } else {
        Remove-Item Env:\AIOS_REQUIRE_NGINX -ErrorAction SilentlyContinue
    }

    return [pscustomobject]@{
        BaseUrl = $env:AIOS_VIEWER_BASE_URL
        NginxBin = $env:AIOS_NGINX_BIN
        NginxRoot = $env:AIOS_NGINX_ROOT
        StaticRoot = $env:AIOS_VIEWER_STATIC_ROOT
    }
}

function Resolve-StartupPort([int]$RequestedPort) {
    function Test-PortUsable([int]$Port) {
        if (Test-PortBindable $Port) {
            return $true
        }
        $null = Clear-PortListeners $Port
        return Test-PortBindable $Port
    }

    $healthUrl = "http://127.0.0.1:$RequestedPort/api/version"
    if (Test-HttpOk $healthUrl) {
        return [pscustomobject]@{
            Port = $RequestedPort
            ExistingHealthy = $true
            Fallback = $false
            Reason = "healthy"
        }
    }

    if (Test-PortUsable $RequestedPort) {
        return [pscustomobject]@{
            Port = $RequestedPort
            ExistingHealthy = $false
            Fallback = $false
            Reason = "free"
        }
    }

    if ($NoPortFallback) {
        $ghostPids = Get-ListeningPidsOnPort $RequestedPort
        $ghostHint = if ($ghostPids.Count -gt 0) {
            " netstat reports LISTENING PID(s): $($ghostPids -join ', ')."
        } else {
            ""
        }
        throw "Port $RequestedPort is not bindable and http://127.0.0.1:$RequestedPort/api/version is not healthy.$ghostHint Free the port, reboot if a ghost listener remains, or rerun with /Port <port>."
    }

    for ($candidate = $RequestedPort + 1; $candidate -le $RequestedPort + 50; $candidate++) {
        $candidateHealthUrl = "http://127.0.0.1:$candidate/api/version"
        if (Test-HttpOk $candidateHealthUrl) {
            return [pscustomobject]@{
                Port = $candidate
                ExistingHealthy = $true
                Fallback = $true
                Reason = "requested port occupied/unhealthy; healthy fallback already running"
            }
        }
        if (Test-PortUsable $candidate) {
            return [pscustomobject]@{
                Port = $candidate
                ExistingHealthy = $false
                Fallback = $true
                Reason = "requested port occupied/unhealthy"
            }
        }
    }

    throw "Port $RequestedPort is not bindable and no fallback port was found in $($RequestedPort + 1)..$($RequestedPort + 50)."
}

$Root = Resolve-InstallRoot
$portChoice = Resolve-StartupPort $Port
$EffectivePort = [int]$portChoice.Port
$WebServer = Join-Path $Root "bin/web_server.exe"
$LogDir = Join-Path $Root "logs"
$AdminSitesUrl = "http://127.0.0.1:$EffectivePort/admin/#/sites"
$FallbackViewerUrl = "http://127.0.0.1:$EffectivePort/viewer/"
$nginxInfo = Enable-PackageNginxIfAvailable $Root

if (-not (Test-Path -LiteralPath $WebServer -PathType Leaf)) {
    throw "web_server.exe not found: $WebServer"
}
New-Item -ItemType Directory -Force -Path $LogDir | Out-Null

$env:WEB_SERVER_PORT = "$EffectivePort"
$env:Path = (Join-Path $Root "bin/surreal") + ";" + $env:Path
$env:ADMIN_USER = "admin"
$env:ADMIN_PASS = "admin"
$env:AIOS_ALLOW_WEAK_DB_CREDS = "1"
$env:AIOS_ALLOW_PUBLIC_BIND = "1"

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$stdout = Join-Path $LogDir "web_server-$stamp.out.log"
$stderr = Join-Path $LogDir "web_server-$stamp.err.log"
$args = @("--config", $Config)

Write-Host "Starting Plant3D AIOS from $Root" -ForegroundColor Cyan
if ($portChoice.Fallback) {
    Write-Warning "Port $Port is occupied but not serving a healthy AIOS backend. Using fallback port $EffectivePort."
}
Write-Host "Admin Sites: $AdminSitesUrl" -ForegroundColor Cyan
Write-Host "Fallback Viewer (available after backend is ready): $FallbackViewerUrl" -ForegroundColor Cyan
if ($nginxInfo) {
    Write-Host "Nginx Viewer (available after a managed site is started): $($nginxInfo.BaseUrl)" -ForegroundColor Cyan
    Write-Host "Nginx: $($nginxInfo.NginxBin)" -ForegroundColor Cyan
    Write-Host "Nginx Root: $($nginxInfo.NginxRoot)" -ForegroundColor Cyan
    Write-Host "Viewer Static Root: $($nginxInfo.StaticRoot)" -ForegroundColor Cyan
} else {
    Write-Host "Nginx Viewer: unavailable; use the fallback viewer above." -ForegroundColor Yellow
}

if ($portChoice.ExistingHealthy) {
    Write-Host "AIOS is already running on port $EffectivePort." -ForegroundColor Green
    if (-not $NoBrowser) {
        Start-Process $AdminSitesUrl
    }
    exit 0
}

$process = Start-Process `
    -FilePath $WebServer `
    -ArgumentList $args `
    -WorkingDirectory $Root `
    -RedirectStandardOutput $stdout `
    -RedirectStandardError $stderr `
    -WindowStyle Hidden `
    -PassThru

if (-not (Wait-HttpOk "http://127.0.0.1:$EffectivePort/api/version" 120 $process)) {
    Write-Warning "Backend did not become ready within 120 seconds. Check logs:"
    Write-Warning "  $stdout"
    Write-Warning "  $stderr"
    if ($process.HasExited) {
        Write-Warning "web_server.exe exited with code $($process.ExitCode)."
        if (Test-Path -LiteralPath $stderr) {
            Write-Warning "Recent stderr:"
            Get-Content -LiteralPath $stderr -Tail 20 | ForEach-Object { Write-Warning "  $_" }
        }
        exit $process.ExitCode
    }
} elseif (-not $NoBrowser) {
    Start-Process $AdminSitesUrl
}

Write-Host "PID: $($process.Id)"
Write-Host "Logs: $LogDir"

if ($Wait) {
    Wait-Process -Id $process.Id
}
