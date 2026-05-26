param(
    [string]$FrontendRoot = "",
    [string]$OutputRoot = "",
    [string]$BundleName = "Plant3D-AIOS-win-x64",
    [string]$SurrealVersion = "2.3.10",
    [string]$SurrealExePath = "",
    [string]$SurrealSha256 = "",
    [switch]$SkipBackendBuild,
    [switch]$SkipFrontendBuild,
    [switch]$SkipZip,
    [switch]$NoDownloadSurreal
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = [System.IO.Path]::GetFullPath((Join-Path $ScriptDir "../.."))
if (-not $FrontendRoot) {
    $FrontendRoot = [System.IO.Path]::GetFullPath((Join-Path $RepoRoot "../plant3d-web"))
}
if (-not $OutputRoot) {
    $OutputRoot = Join-Path $RepoRoot "dist/package"
}

$PackageRoot = Join-Path $OutputRoot $BundleName
$TargetTriple = "x86_64-pc-windows-msvc"
$BackendExe = Join-Path $RepoRoot "target/$TargetTriple/release/web_server.exe"
$AiosDatabaseExe = Join-Path $RepoRoot "target/$TargetTriple/release/aios-database.exe"
$FrontendDist = Join-Path $FrontendRoot "dist"
$SurrealCacheExe = Join-Path $RepoRoot "tools/surrealdb/windows/surreal.exe"
$SurrealResourceDir = [System.IO.Path]::GetFullPath((Join-Path $RepoRoot "../rs-core/resource/surreal"))
$Features = "ws,gen_model,manifold,project_hd,surreal-save,write-to-surrealdb,sqlite-index,web_server,parquet-export"

function Step([string]$Message) {
    Write-Host ""
    Write-Host "==> $Message" -ForegroundColor Cyan
}

function Require-File([string]$Path, [string]$Label) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "$Label not found: $Path"
    }
}

function Copy-Tree([string]$Source, [string]$Destination) {
    if (-not (Test-Path -LiteralPath $Source)) {
        throw "Source directory not found: $Source"
    }
    New-Item -ItemType Directory -Force -Path $Destination | Out-Null
    Copy-Item -Path (Join-Path $Source "*") -Destination $Destination -Recurse -Force
}

function Get-GitCommit([string]$Path) {
    try {
        return (& git -C $Path rev-parse HEAD 2>$null).Trim()
    } catch {
        return "unknown"
    }
}

function Get-FileSha256([string]$Path) {
    return (Get-FileHash -Path $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Assert-NasmAvailable {
    $nasm = Get-Command nasm.exe -ErrorAction SilentlyContinue
    if ($nasm) { return }

    foreach ($dir in @("C:\Program Files\NASM", "C:\Program Files (x86)\NASM")) {
        if (Test-Path -LiteralPath (Join-Path $dir "nasm.exe") -PathType Leaf) {
            $env:Path = "$dir;$env:Path"
            return
        }
    }

    throw "nasm.exe is required for the release build. Install NASM (for example: winget install NASM.NASM) and rerun this script."
}

function Assert-Sha256([string]$Path, [string]$Expected) {
    if (-not $Expected) { return }
    $actual = Get-FileSha256 $Path
    if ($actual -ne $Expected.ToLowerInvariant()) {
        throw "SHA256 mismatch for $Path. expected=$Expected actual=$actual"
    }
}

function Expand-SurrealArchive([string]$ArchivePath, [string]$DestinationDir) {
    if (Test-Path -LiteralPath $DestinationDir) {
        Remove-Item -LiteralPath $DestinationDir -Recurse -Force
    }
    New-Item -ItemType Directory -Force -Path $DestinationDir | Out-Null

    if ($ArchivePath.EndsWith(".zip", [System.StringComparison]::OrdinalIgnoreCase)) {
        Expand-Archive -LiteralPath $ArchivePath -DestinationPath $DestinationDir -Force
    } else {
        & tar -xzf $ArchivePath -C $DestinationDir
        if ($LASTEXITCODE -ne 0) {
            throw "Failed to extract $ArchivePath"
        }
    }

    $candidate = Get-ChildItem -Path $DestinationDir -Filter "surreal.exe" -Recurse -File | Select-Object -First 1
    if (-not $candidate) {
        throw "surreal.exe not found in $ArchivePath"
    }
    return $candidate.FullName
}

function Download-SurrealExe([string]$Version, [string]$DestinationExe) {
    if ($NoDownloadSurreal) {
        throw "SurrealDB cache is missing and -NoDownloadSurreal was specified"
    }
    if (-not $Version) {
        throw "SurrealVersion is required when SurrealDB is not provided locally"
    }

    $downloadDir = Join-Path $RepoRoot "tools/surrealdb/downloads"
    New-Item -ItemType Directory -Force -Path $downloadDir | Out-Null
    $urls = @(
        "https://github.com/surrealdb/surrealdb/releases/download/v$Version/surreal-v$Version.windows-amd64.exe",
        "https://github.com/surrealdb/surrealdb/releases/download/v$Version/surreal-v$Version.windows-amd64.zip",
        "https://github.com/surrealdb/surrealdb/releases/download/v$Version/surreal-v$Version.windows-amd64.tgz",
        "https://github.com/surrealdb/surrealdb/releases/download/v$Version/surreal-v$Version.windows-amd64.tar.gz"
    )

    foreach ($url in $urls) {
        $fileName = Split-Path $url -Leaf
        $tmp = Join-Path $downloadDir $fileName
        try {
            Write-Host "Trying $url"
            Invoke-WebRequest -Uri $url -OutFile $tmp -UseBasicParsing
            if ($fileName.EndsWith(".exe", [System.StringComparison]::OrdinalIgnoreCase)) {
                New-Item -ItemType Directory -Force -Path (Split-Path -Parent $DestinationExe) | Out-Null
                Copy-Item -LiteralPath $tmp -Destination $DestinationExe -Force
            } else {
                $expanded = Expand-SurrealArchive $tmp (Join-Path $downloadDir "expanded-$Version")
                New-Item -ItemType Directory -Force -Path (Split-Path -Parent $DestinationExe) | Out-Null
                Copy-Item -LiteralPath $expanded -Destination $DestinationExe -Force
            }
            return $DestinationExe
        } catch {
            Write-Warning "Download candidate failed: $url ($($_.Exception.Message))"
        }
    }

    throw "Unable to download SurrealDB v$Version for Windows x64"
}

function Resolve-SurrealExe() {
    if ($SurrealExePath) {
        Require-File $SurrealExePath "SurrealDB executable"
        return [System.IO.Path]::GetFullPath($SurrealExePath)
    }
    if (Test-Path -LiteralPath $SurrealCacheExe -PathType Leaf) {
        return $SurrealCacheExe
    }
    return Download-SurrealExe $SurrealVersion $SurrealCacheExe
}

function Update-PackageDbOption([string]$Path) {
    $updates = @{
        "__root__.meshes_path" = '"./assets/meshes"'
        "__root__.surreal_script_dir" = '"resource/surreal"'
        "web_server.port" = "3100"
        "web_server.auto_start_surreal" = "true"
        "web_server.surreal_bin" = '"bin/surreal/surreal.exe"'
        "web_server.surreal_data_path" = '"runtime/surrealdb"'
        "web_server.surreal_bind" = '"127.0.0.1:8020"'
        "web_server.surreal_user" = '"root"'
        "web_server.surreal_password" = '"root"'
        "surrealdb.mode" = '"ws"'
        "surrealdb.path" = '"runtime/surrealdb"'
        "surrealkv.mode" = '"file"'
        "surrealkv.path" = '"runtime/surrealkv"'
    }
    $section = "__root__"
    $seen = @{}
    $lines = Get-Content -LiteralPath $Path
    $out = foreach ($line in $lines) {
        if ($line -match '^\s*\[([^\]]+)\]\s*$') {
            $section = $Matches[1]
            $line
            continue
        }
        if ($line -match '^(\s*)([A-Za-z0-9_-]+)(\s*=\s*)(.*)$') {
            $fullKey = "$section.$($Matches[2])"
            if ($updates.ContainsKey($fullKey)) {
                $seen[$fullKey] = $true
                "$($Matches[1])$($Matches[2])$($Matches[3])$($updates[$fullKey])"
                continue
            }
        }
        $line
    }
    Set-Content -LiteralPath $Path -Value $out -Encoding UTF8
}

Step "Validate input repositories"
if (-not (Test-Path -LiteralPath (Join-Path $RepoRoot "Cargo.toml"))) {
    throw "Backend repo not found: $RepoRoot"
}
if (-not (Test-Path -LiteralPath (Join-Path $FrontendRoot "package.json"))) {
    throw "Frontend repo not found: $FrontendRoot"
}
if (-not $SkipBackendBuild) {
    Step "Build backend web_server and aios-database"
    Assert-NasmAvailable
    Push-Location $RepoRoot
    try {
        & cargo build --release --bin web_server --bin aios-database --target $TargetTriple --no-default-features --features $Features
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    } finally {
        Pop-Location
    }
}
Require-File $BackendExe "Backend executable"
Require-File $AiosDatabaseExe "aios-database executable"

if (-not $SkipFrontendBuild) {
    Step "Build plant3d-web for /viewer/"
    Push-Location $FrontendRoot
    $oldBase = $env:VITE_BASE_PATH
    $oldApi = $env:VITE_GEN_MODEL_API_BASE_URL
    try {
        $env:VITE_BASE_PATH = "/viewer/"
        Remove-Item Env:\VITE_GEN_MODEL_API_BASE_URL -ErrorAction SilentlyContinue
        & npm run build-only
        if ($LASTEXITCODE -ne 0) { throw "frontend build failed" }
    } finally {
        if ($null -ne $oldBase) { $env:VITE_BASE_PATH = $oldBase } else { Remove-Item Env:\VITE_BASE_PATH -ErrorAction SilentlyContinue }
        if ($null -ne $oldApi) { $env:VITE_GEN_MODEL_API_BASE_URL = $oldApi } else { Remove-Item Env:\VITE_GEN_MODEL_API_BASE_URL -ErrorAction SilentlyContinue }
        Pop-Location
    }
}
if (-not (Test-Path -LiteralPath (Join-Path $FrontendDist "index.html") -PathType Leaf)) {
    throw "Frontend dist missing: $FrontendDist"
}

Step "Resolve bundled SurrealDB"
$ResolvedSurreal = Resolve-SurrealExe
Require-File $ResolvedSurreal "SurrealDB executable"
Assert-Sha256 $ResolvedSurreal $SurrealSha256
$surrealVersionText = ""
try {
    $surrealVersionText = (& $ResolvedSurreal version 2>$null) -join " "
} catch {
    $surrealVersionText = "unknown"
}
$surrealSha = Get-FileSha256 $ResolvedSurreal
$aiosDatabaseSha = Get-FileSha256 $AiosDatabaseExe

Step "Create package layout"
if (Test-Path -LiteralPath $PackageRoot) {
    Remove-Item -LiteralPath $PackageRoot -Recurse -Force
}
foreach ($dir in @("bin", "bin/surreal", "viewer", "db_options", "resource/surreal", "runtime/surrealdb", "runtime/surrealkv", "output", "assets/meshes", "logs")) {
    New-Item -ItemType Directory -Force -Path (Join-Path $PackageRoot $dir) | Out-Null
}

Copy-Item -LiteralPath $BackendExe -Destination (Join-Path $PackageRoot "bin/web_server.exe") -Force
Copy-Item -LiteralPath $AiosDatabaseExe -Destination (Join-Path $PackageRoot "bin/aios-database.exe") -Force
Copy-Item -LiteralPath $ResolvedSurreal -Destination (Join-Path $PackageRoot "bin/surreal/surreal.exe") -Force
Copy-Tree $FrontendDist (Join-Path $PackageRoot "viewer")
if (-not (Test-Path -LiteralPath $SurrealResourceDir -PathType Container)) {
    throw "Surreal resource directory not found: $SurrealResourceDir"
}
Copy-Tree $SurrealResourceDir (Join-Path $PackageRoot "resource/surreal")
Copy-Item -Path (Join-Path $RepoRoot "db_options/*") -Destination (Join-Path $PackageRoot "db_options") -Recurse -Force
Update-PackageDbOption (Join-Path $PackageRoot "db_options/DbOption.toml")

Copy-Item -LiteralPath (Join-Path $ScriptDir "start-plant3d.ps1") -Destination (Join-Path $PackageRoot "start-plant3d.ps1") -Force
Copy-Item -LiteralPath (Join-Path $ScriptDir "install-service.ps1") -Destination (Join-Path $PackageRoot "install-service.ps1") -Force

$buildInfo = [ordered]@{
    name = "Plant3D AIOS Windows x64"
    bundle = $BundleName
    builtAt = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    target = $TargetTriple
    backendCommit = Get-GitCommit $RepoRoot
    frontendCommit = Get-GitCommit $FrontendRoot
    surrealVersion = $SurrealVersion
    surrealVersionText = $surrealVersionText
    surrealSha256 = $surrealSha
    surrealBundled = $true
    aiosDatabaseBundled = $true
    aiosDatabaseSha256 = $aiosDatabaseSha
    databaseDataIncluded = $false
    webPort = 3100
    viewerUrl = "http://127.0.0.1:3100/viewer/"
}
$buildInfo | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $PackageRoot "BUILD_INFO.json") -Encoding UTF8

@"
# Plant3D AIOS Windows x64 安装说明

## 快速启动

1. 解压整个目录，不要只复制 `bin`。
2. 双击或在 PowerShell 中运行：

```powershell
.\start-plant3d.ps1
```

3. 浏览器打开：

```text
http://127.0.0.1:3100/viewer/
```

## 后台自启动

以管理员 PowerShell 运行：

```powershell
.\install-service.ps1 -RunNow
```

该脚本使用 Windows 计划任务启动 `start-plant3d.ps1 -NoBrowser`，确保工作目录固定为安装根目录。

## 目录说明

- `bin/web_server.exe`：后端服务。
- `bin/aios-database.exe`：站点部署/解析使用的数据库导入执行文件。
- `bin/surreal/surreal.exe`：随包分发的 SurrealDB。
- `resource/surreal/`：初始化函数与 `att_meta` 属性元数据脚本。
- `viewer/`：plant3d-web 静态前端，挂载在 `/viewer/`。
- `db_options/DbOption.toml`：默认运行配置。
- `runtime/surrealdb/`：目标电脑首次启动时创建的新 SurrealDB 数据目录；安装包不包含本机数据库数据。
- `runtime/surrealkv/`：预留的空运行目录；安装包不复制本机数据库数据。
- `output/`、`assets/meshes/`：模型输出与网格资源目录。
- `logs/`：启动日志。

## 数据说明

该安装包只携带数据库执行文件 `bin/surreal/surreal.exe`，不携带本机 SurrealDB 数据。部署到另一台电脑后会从空的 `runtime/surrealdb/` 开始创建新站点数据。

## 修改端口

编辑 `db_options/DbOption.toml`：

```toml
[web_server]
port = 3100
surreal_bind = "127.0.0.1:8020"
```

修改后重新运行 `start-plant3d.ps1 -Port <新端口>`。
"@ | Set-Content -LiteralPath (Join-Path $PackageRoot "README-安装说明.md") -Encoding UTF8

if (-not $SkipZip) {
    Step "Create zip archive"
    $zipPath = Join-Path $OutputRoot "$BundleName.zip"
    if (Test-Path -LiteralPath $zipPath) {
        Remove-Item -LiteralPath $zipPath -Force
    }
    Compress-Archive -LiteralPath $PackageRoot -DestinationPath $zipPath -Force
    Write-Host "ZIP: $zipPath" -ForegroundColor Green
}

Write-Host ""
Write-Host "Package ready: $PackageRoot" -ForegroundColor Green
