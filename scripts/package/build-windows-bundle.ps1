param(
    [string]$FrontendRoot = "",
    [string]$OutputRoot = "",
    [string]$BundleName = "Plant3D-AIOS-win-x64",
    [ValidateSet("release", "debug", "both")]
    [string]$BuildProfile = "both",
    [string]$SurrealVersion = "3.1.0-alpha",
    [string]$SurrealExePath = "",
    [string]$SurrealSha256 = "",
    [ValidateSet("cranelift", "llvm")]
    [string]$DebugCodegenBackend = "cranelift",
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
$FrontendDist = Join-Path $FrontendRoot "dist"
$FrontendBuildCacheRoot = Join-Path $OutputRoot "_frontend-builds"
$ViewerFallbackDist = Join-Path $FrontendBuildCacheRoot "viewer"
$ViewerRootDist = Join-Path $FrontendBuildCacheRoot "viewer-root"
$AdminStaticDist = Join-Path $RepoRoot "src/web_server/static/admin"
$SurrealCacheExe = Join-Path $RepoRoot "tools/surrealdb/windows/surreal.exe"
$NginxCacheExe = Join-Path $RepoRoot "tools/nginx/windows/nginx.exe"
$SurrealResourceDir = [System.IO.Path]::GetFullPath((Join-Path $RepoRoot "../rs-core/resource/surreal"))
$Features = "ws,gen_model,manifold,project_hd,surreal-save,write-to-surrealdb,sqlite-index,web_server,parquet-export,kv-rocksdb"
if ($BuildProfile -eq "both") {
    $RequestedProfiles = @("release", "debug")
} else {
    $RequestedProfiles = @($BuildProfile)
}

function Step([string]$Message) {
    Write-Host ""
    Write-Host "==> $Message" -ForegroundColor Cyan
}

function Require-File([string]$Path, [string]$Label) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "$Label not found: $Path"
    }
}

function Get-ProfileTargetDir([string]$Profile) {
    return Join-Path (Join-Path $RepoRoot "target/$TargetTriple") $Profile
}

function Get-ProfileExe([string]$Profile, [string]$ExeName) {
    return Join-Path (Get-ProfileTargetDir $Profile) $ExeName
}

function Get-CargoProfileArgs([string]$Profile) {
    if ($Profile -eq "release") {
        return @("--release")
    }
    return @()
}

function Get-ProfileCodegenBackend([string]$Profile) {
    if ($Profile -eq "debug") {
        return $DebugCodegenBackend
    }
    return "llvm"
}

function Test-DebugCraneliftEnabled([string]$Profile) {
    return $Profile -eq "debug" -and $DebugCodegenBackend -eq "cranelift"
}

function Assert-CraneliftAvailable {
    $rustup = Get-Command rustup -ErrorAction SilentlyContinue
    if (-not $rustup) {
        throw "rustup is required for debug Cranelift builds. Install rustup or rerun with -DebugCodegenBackend llvm."
    }

    $installed = & rustup +nightly component list --installed 2>$null
    if ($LASTEXITCODE -ne 0) {
        throw "nightly toolchain is required for debug Cranelift builds. Run: rustup toolchain install nightly"
    }

    $hasCranelift = $installed | Where-Object {
        $_ -match '^rustc-codegen-cranelift(-preview)?-'
    } | Select-Object -First 1
    if (-not $hasCranelift) {
        throw "rustc_codegen_cranelift is required for debug builds. Run: rustup +nightly component add rustc-codegen-cranelift"
    }
}

function Invoke-BackendCargoBuild([string]$Profile) {
    $profileArgs = Get-CargoProfileArgs $Profile
    if (Test-DebugCraneliftEnabled $Profile) {
        Assert-CraneliftAvailable
        Write-Host "Using rustc_codegen_cranelift for debug backend build (profile.dev in .cargo/config.toml)" -ForegroundColor Yellow
        & cargo +nightly build @($profileArgs) --bin web_server --bin offline_deployer --bin aios-database --target $TargetTriple --no-default-features --features $Features
    } else {
        & cargo build @($profileArgs) --bin web_server --bin offline_deployer --bin aios-database --target $TargetTriple --no-default-features --features $Features
    }

    if ($LASTEXITCODE -ne 0) {
        throw "cargo build failed for $Profile profile"
    }
}

function Copy-Tree([string]$Source, [string]$Destination) {
    if (-not (Test-Path -LiteralPath $Source)) {
        throw "Source directory not found: $Source"
    }
    New-Item -ItemType Directory -Force -Path $Destination | Out-Null
    Copy-Item -Path (Join-Path $Source "*") -Destination $Destination -Recurse -Force
}

function Assert-DuckDbOfflineDist([string]$Dist, [string]$Label) {
    $duckdbDir = Join-Path $Dist "duckdb"
    $required = @(
        "duckdb-browser-mvp.worker.js",
        "duckdb-browser-eh.worker.js",
        "duckdb-browser-coi.worker.js",
        "duckdb-browser-coi.pthread.worker.js",
        "duckdb-mvp.wasm",
        "duckdb-eh.wasm",
        "duckdb-coi.wasm"
    )
    foreach ($file in $required) {
        $path = Join-Path $duckdbDir $file
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "DuckDB offline asset missing for ${Label}: $path"
        }
    }

    $jsFiles = Get-ChildItem -LiteralPath $Dist -Recurse -File -Include "*.js" -ErrorAction SilentlyContinue
    foreach ($file in $jsFiles) {
        $content = Get-Content -LiteralPath $file.FullName -Raw
        if ($content -match "https://cdn\.jsdelivr\.net/(npm/)?@duckdb/duckdb-wasm") {
            throw "DuckDB CDN URL found in ${Label}: $($file.FullName). Rebuild plant3d-web with local DuckDB bundles before packaging."
        }
    }
}

function Invoke-FrontendBuild([string]$BasePath, [string]$Label, [string]$Destination) {
    Step "Build plant3d-web for $Label"
    Push-Location $FrontendRoot
    $oldBase = $env:VITE_BASE_PATH
    $oldApi = $env:VITE_GEN_MODEL_API_BASE_URL
    try {
        $env:VITE_BASE_PATH = $BasePath
        Remove-Item Env:\VITE_GEN_MODEL_API_BASE_URL -ErrorAction SilentlyContinue
        & npm run build-only
        if ($LASTEXITCODE -ne 0) { throw "frontend build failed for $Label" }
    } finally {
        if ($null -ne $oldBase) { $env:VITE_BASE_PATH = $oldBase } else { Remove-Item Env:\VITE_BASE_PATH -ErrorAction SilentlyContinue }
        if ($null -ne $oldApi) { $env:VITE_GEN_MODEL_API_BASE_URL = $oldApi } else { Remove-Item Env:\VITE_GEN_MODEL_API_BASE_URL -ErrorAction SilentlyContinue }
        Pop-Location
    }
    if (-not (Test-Path -LiteralPath (Join-Path $FrontendDist "index.html") -PathType Leaf)) {
        throw "Frontend dist missing after $Label build: $FrontendDist"
    }
    if (Test-Path -LiteralPath $Destination) {
        Remove-Item -LiteralPath $Destination -Recurse -Force
    }
    Copy-Tree $FrontendDist $Destination
    Assert-DuckDbOfflineDist $Destination $Label
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

function Get-SurrealVersionText([string]$ExePath) {
    try {
        return ((& $ExePath version 2>$null) -join " ")
    } catch {
        return "unknown"
    }
}

function Test-SurrealVersion([string]$ExePath, [string]$ExpectedVersion) {
    if (-not $ExpectedVersion) { return $true }
    $text = Get-SurrealVersionText $ExePath
    return $text -like "$ExpectedVersion*"
}

function Resolve-SurrealExe() {
    if ($SurrealExePath) {
        Require-File $SurrealExePath "SurrealDB executable"
        if (-not (Test-SurrealVersion $SurrealExePath $SurrealVersion)) {
            $actual = Get-SurrealVersionText $SurrealExePath
            throw "SurrealDB executable version mismatch: expected $SurrealVersion, actual $actual"
        }
        return [System.IO.Path]::GetFullPath($SurrealExePath)
    }
    if (Test-Path -LiteralPath $SurrealCacheExe -PathType Leaf) {
        if (Test-SurrealVersion $SurrealCacheExe $SurrealVersion) {
            return $SurrealCacheExe
        }
        $actual = Get-SurrealVersionText $SurrealCacheExe
        Write-Warning "Cached SurrealDB version mismatch: expected $SurrealVersion, actual $actual. Downloading replacement."
    }
    return Download-SurrealExe $SurrealVersion $SurrealCacheExe
}

function Update-PackageDbOption([string]$Path) {
    $updates = @{
        "__root__.meshes_path" = '"./assets/meshes"'
        "__root__.surreal_script_dir" = '"resource/surreal"'
        "__root__.surreal_ip" = '"127.0.0.1"'
        "__root__.surreal_port" = "8020"
        "__root__.surreal_user" = '"root"'
        "__root__.surreal_password" = '"root"'
        "web_server.port" = "3100"
        "web_server.auto_start_surreal" = "true"
        "web_server.surreal_bin" = '"bin/surreal/surreal.exe"'
        "web_server.surreal_data_path" = '"runtime/surrealdb"'
        "web_server.surreal_bind" = '"127.0.0.1:8020"'
        "web_server.surreal_user" = '"root"'
        "web_server.surreal_password" = '"root"'
        "surrealdb.mode" = '"ws"'
        "surrealdb.ip" = '"127.0.0.1"'
        "surrealdb.port" = "8020"
        "surrealdb.user" = '"root"'
        "surrealdb.password" = '"root"'
        "surrealdb.path" = '"runtime/surrealdb"'
        "surrealkv.mode" = '"file"'
        "surrealkv.path" = '"runtime/surrealkv"'
    }
    $sectionOrder = @("__root__", "web_server", "surrealdb", "surrealkv")
    $keysBySection = @{
        "__root__" = @("meshes_path", "surreal_script_dir", "surreal_ip", "surreal_port", "surreal_user", "surreal_password")
        "web_server" = @("port", "auto_start_surreal", "surreal_bin", "surreal_data_path", "surreal_bind", "surreal_user", "surreal_password")
        "surrealdb" = @("mode", "ip", "port", "user", "password", "path")
        "surrealkv" = @("mode", "path")
    }
    $section = "__root__"
    $seen = @{}
    $lines = Get-Content -LiteralPath $Path
    $out = New-Object System.Collections.Generic.List[string]
    foreach ($line in $lines) {
        if ($line -match '^\s*\[([^\]]+)\]\s*$') {
            Add-MissingPackageDbOptionKeys $out $updates $keysBySection $seen $section
            $section = $Matches[1]
            $out.Add($line)
            continue
        }
        if ($line -match '^(\s*)([A-Za-z0-9_-]+)(\s*=\s*)(.*)$') {
            $fullKey = "$section.$($Matches[2])"
            if ($updates.ContainsKey($fullKey)) {
                $seen[$fullKey] = $true
                $out.Add("$($Matches[1])$($Matches[2])$($Matches[3])$($updates[$fullKey])")
                continue
            }
        }
        $out.Add($line)
    }
    Add-MissingPackageDbOptionKeys $out $updates $keysBySection $seen $section
    foreach ($targetSection in $sectionOrder) {
        Add-MissingPackageDbOptionKeys $out $updates $keysBySection $seen $targetSection $true
    }
    Set-Content -LiteralPath $Path -Value $out -Encoding UTF8
}

function Add-MissingPackageDbOptionKeys(
    [System.Collections.Generic.List[string]]$Out,
    [hashtable]$Updates,
    [hashtable]$KeysBySection,
    [hashtable]$Seen,
    [string]$Section,
    [bool]$CreateSection = $false
) {
    if (-not $KeysBySection.ContainsKey($Section)) { return }
    $missing = @()
    foreach ($key in $KeysBySection[$Section]) {
        $fullKey = "$Section.$key"
        if ($Updates.ContainsKey($fullKey) -and -not $Seen.ContainsKey($fullKey)) {
            $missing += $key
        }
    }
    if ($missing.Count -eq 0) { return }
    if ($CreateSection -and $Section -ne "__root__") {
        $Out.Add("")
        $Out.Add("[$Section]")
    }
    foreach ($key in $missing) {
        $fullKey = "$Section.$key"
        $Out.Add("$key = $($Updates[$fullKey])")
        $Seen[$fullKey] = $true
    }
}

Step "Validate input repositories"
if (-not (Test-Path -LiteralPath (Join-Path $RepoRoot "Cargo.toml"))) {
    throw "Backend repo not found: $RepoRoot"
}
if (-not (Test-Path -LiteralPath (Join-Path $FrontendRoot "package.json"))) {
    throw "Frontend repo not found: $FrontendRoot"
}
if (-not (Test-Path -LiteralPath (Join-Path $AdminStaticDist "index.html") -PathType Leaf)) {
    throw "Admin static dist not found: $AdminStaticDist"
}
if (-not $SkipBackendBuild) {
    Step "Build backend web_server, offline_deployer and aios-database ($($RequestedProfiles -join ', '))"
    $env:CARGO_INCREMENTAL = "1"
    if ($RequestedProfiles -contains "release") {
        Assert-NasmAvailable
    }
    Push-Location $RepoRoot
    try {
        foreach ($profile in $RequestedProfiles) {
            Invoke-BackendCargoBuild $profile
        }
    } finally {
        Pop-Location
    }
}
foreach ($profile in $RequestedProfiles) {
    Require-File (Get-ProfileExe $profile "web_server.exe") "$profile backend executable"
    Require-File (Get-ProfileExe $profile "offline_deployer.exe") "$profile offline deployer executable"
    Require-File (Get-ProfileExe $profile "aios-database.exe") "$profile aios-database executable"
}

if (-not $SkipFrontendBuild) {
    if (Test-Path -LiteralPath $FrontendBuildCacheRoot) {
        Remove-Item -LiteralPath $FrontendBuildCacheRoot -Recurse -Force
    }
    New-Item -ItemType Directory -Force -Path $FrontendBuildCacheRoot | Out-Null
    Invoke-FrontendBuild "/viewer/" "/viewer/ fallback" $ViewerFallbackDist
    Invoke-FrontendBuild "/" "Nginx root /" $ViewerRootDist
} else {
    Write-Warning "-SkipFrontendBuild specified; using existing $FrontendDist for both viewer/ and viewer-root/. Ensure the dist base path matches your target."
    $ViewerFallbackDist = $FrontendDist
    $ViewerRootDist = $FrontendDist
}
foreach ($dist in @($ViewerFallbackDist, $ViewerRootDist)) {
    if (-not (Test-Path -LiteralPath (Join-Path $dist "index.html") -PathType Leaf)) {
        throw "Frontend dist missing: $dist"
    }
}
Assert-DuckDbOfflineDist $ViewerFallbackDist "/viewer/ fallback"
Assert-DuckDbOfflineDist $ViewerRootDist "Nginx root /"

Step "Resolve bundled SurrealDB"
$ResolvedSurreal = Resolve-SurrealExe
Require-File $ResolvedSurreal "SurrealDB executable"
Assert-Sha256 $ResolvedSurreal $SurrealSha256
$surrealVersionText = Get-SurrealVersionText $ResolvedSurreal
$surrealSha = Get-FileSha256 $ResolvedSurreal

Step "Create package layout"
if (Test-Path -LiteralPath $PackageRoot) {
    Remove-Item -LiteralPath $PackageRoot -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $PackageRoot | Out-Null
if (-not (Test-Path -LiteralPath $SurrealResourceDir -PathType Container)) {
    throw "Surreal resource directory not found: $SurrealResourceDir"
}
foreach ($profile in $RequestedProfiles) {
    $profilePackageRoot = Join-Path $PackageRoot $profile
    foreach ($dir in @("bin", "bin/surreal", "bin/nginx", "viewer", "viewer-root", "src/web_server/static/admin", "db_options", "resource/surreal", "runtime/surrealdb", "runtime/surrealkv", "output", "assets/meshes", "logs")) {
        New-Item -ItemType Directory -Force -Path (Join-Path $profilePackageRoot $dir) | Out-Null
    }

    $backendExe = Get-ProfileExe $profile "web_server.exe"
    $offlineDeployerExe = Get-ProfileExe $profile "offline_deployer.exe"
    $aiosDatabaseExe = Get-ProfileExe $profile "aios-database.exe"
    $aiosDatabaseSha = Get-FileSha256 $aiosDatabaseExe

    Copy-Item -LiteralPath $backendExe -Destination (Join-Path $profilePackageRoot "bin/web_server.exe") -Force
    Copy-Item -LiteralPath $offlineDeployerExe -Destination (Join-Path $profilePackageRoot "offline_deployer.exe") -Force
    Copy-Item -LiteralPath $aiosDatabaseExe -Destination (Join-Path $profilePackageRoot "bin/aios-database.exe") -Force
    Copy-Item -LiteralPath $ResolvedSurreal -Destination (Join-Path $profilePackageRoot "bin/surreal/surreal.exe") -Force
    $nginxBundled = $false
    if (Test-Path -LiteralPath $NginxCacheExe -PathType Leaf) {
        Copy-Item -LiteralPath $NginxCacheExe -Destination (Join-Path $profilePackageRoot "bin/nginx/nginx.exe") -Force
        $nginxBundled = $true
    } else {
        Write-Warning "Optional nginx.exe not found at $NginxCacheExe; package will use /viewer/ fallback unless AIOS_NGINX_BIN is provided at runtime."
    }
    Copy-Tree $ViewerFallbackDist (Join-Path $profilePackageRoot "viewer")
    Copy-Tree $ViewerRootDist (Join-Path $profilePackageRoot "viewer-root")
    Copy-Tree $AdminStaticDist (Join-Path $profilePackageRoot "src/web_server/static/admin")
    Copy-Tree $SurrealResourceDir (Join-Path $profilePackageRoot "resource/surreal")
    Copy-Item -Path (Join-Path $RepoRoot "db_options/*") -Destination (Join-Path $profilePackageRoot "db_options") -Recurse -Force
    Update-PackageDbOption (Join-Path $profilePackageRoot "db_options/DbOption.toml")

    Copy-Item -LiteralPath (Join-Path $ScriptDir "start-plant3d.bat") -Destination (Join-Path $profilePackageRoot "start-plant3d.bat") -Force
    Copy-Item -LiteralPath (Join-Path $ScriptDir "start-plant3d.ps1") -Destination (Join-Path $profilePackageRoot "start-plant3d.ps1") -Force
    Copy-Item -LiteralPath (Join-Path $ScriptDir "stop-plant3d.bat") -Destination (Join-Path $profilePackageRoot "stop-plant3d.bat") -Force
    Copy-Item -LiteralPath (Join-Path $ScriptDir "stop-plant3d.ps1") -Destination (Join-Path $profilePackageRoot "stop-plant3d.ps1") -Force
    Copy-Item -LiteralPath (Join-Path $ScriptDir "verify-offline-viewer.bat") -Destination (Join-Path $profilePackageRoot "verify-offline-viewer.bat") -Force
    Copy-Item -LiteralPath (Join-Path $ScriptDir "verify-offline-viewer.ps1") -Destination (Join-Path $profilePackageRoot "verify-offline-viewer.ps1") -Force
    Copy-Item -LiteralPath (Join-Path $ScriptDir "install-service.bat") -Destination (Join-Path $profilePackageRoot "install-service.bat") -Force
    Copy-Item -LiteralPath (Join-Path $ScriptDir "install-service.ps1") -Destination (Join-Path $profilePackageRoot "install-service.ps1") -Force

    $buildInfo = [ordered]@{
        name = "Plant3D AIOS Windows x64"
        bundle = $BundleName
        profile = $profile
        builtAt = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
        target = $TargetTriple
        rustCodegenBackend = Get-ProfileCodegenBackend $profile
        cargoIncremental = $true
        backendCommit = Get-GitCommit $RepoRoot
        frontendCommit = Get-GitCommit $FrontendRoot
        surrealVersion = $SurrealVersion
        surrealVersionText = $surrealVersionText
        surrealSha256 = $surrealSha
        surrealBundled = $true
        nginxBundled = $nginxBundled
        nginxExe = if ($nginxBundled) { "bin/nginx/nginx.exe" } else { $null }
        aiosDatabaseBundled = $true
        aiosDatabaseSha256 = $aiosDatabaseSha
        databaseDataIncluded = $false
        webPort = 3100
        viewerUrl = "http://127.0.0.1:3100/viewer/"
        viewerFallbackUrl = "http://127.0.0.1:3100/viewer/"
        customerViewerUrlTemplate = "http://<host>/?output_project=<project>&show_dbnum=<dbnum>"
        nginxMode = "on"
        nginxRequiredByDefault = $true
        nginxStaticRoot = "viewer-root"
        offlineDeployWizardUrl = "http://127.0.0.1:3100/admin/#/offline-deploy"
        offlineDeployExe = "offline_deployer.exe"
        startupScript = "start-plant3d.bat"
        stopScript = "stop-plant3d.bat"
        stopPowerShellScript = "stop-plant3d.ps1"
        offlineViewerVerifyScript = "verify-offline-viewer.bat"
        serviceScript = "install-service.bat"
        servicePowerShellScript = "install-service.ps1"
    }
    $buildInfo | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $profilePackageRoot "BUILD_INFO.json") -Encoding UTF8

    @'
# Plant3D AIOS Windows x64 安装说明

## 快速启动

1. 解压整个目录，不要只复制 `bin`。
2. 双击或在命令提示符中运行：

```bat
start-plant3d.bat
```

3. 主后台启动成功后，本地 fallback Viewer 可打开：

```text
http://127.0.0.1:3100/viewer/
```

默认启动脚本会准备 Nginx 配置，把 `viewer-root/` 作为客户根入口。注意：
Nginx 根入口会在站点完成部署并启动后监听，不是主后台刚启动时立即监听。
站点运行后可在管理后台查看实际 `viewer_url`，或使用如下模板：

```text
http://<本机IP>/?output_project=<project>&show_dbnum=<dbnum>
```

如需临时回到非强制 fallback 模式，可运行：

```bat
start-plant3d.bat /EnableNginx auto
```

## 停止后台

停止当前安装包启动的站点部署 admin、包内 Nginx/SurrealDB 进程，并释放默认端口：

```bat
stop-plant3d.bat
```

如只想停止 admin，保留包内 Nginx 或 SurrealDB，可运行：

```bat
stop-plant3d.bat /KeepNginx /KeepSurreal
```

## 离线部署向导

如果只想打开"把本机站点推送到远端服务器"的安装部署向导，双击：

```bat
offline_deployer.exe
```

它会启动同一个 Rust Web 服务，并自动打开：

```text
http://127.0.0.1:3100/admin/#/offline-deploy
```

## 后台自启动

以管理员命令提示符运行：

```bat
install-service.bat /RunNow
```

该脚本使用 Windows 计划任务启动 `start-plant3d.bat /NoBrowser`，确保工作目录固定为安装根目录。
卸载自启动任务：

```bat
install-service.bat /Uninstall
```

## 离线 Viewer 验证

打包后可在安装根目录运行：

```bat
verify-offline-viewer.bat
```

或直接调用 PowerShell 脚本：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\verify-offline-viewer.ps1
```

该脚本会检查 `viewer/` 与 `viewer-root/` 是否包含 DuckDB-WASM 离线资源（worker + wasm），并阻断 `cdn.jsdelivr.net` 等公网 CDN 旧构建产物。

## 目录说明

- `bin/web_server.exe`：后端服务。
- `offline_deployer.exe`：离线部署向导入口，启动本地网页并打开 `/admin/#/offline-deploy`。
- `start-plant3d.bat` / `stop-plant3d.bat`：启动 / 停止站点部署 admin 的便捷脚本。
- `verify-offline-viewer.bat` / `verify-offline-viewer.ps1`：离线 Viewer 验证脚本，用于确认前端不依赖公网 CDN。
- `bin/aios-database.exe`：站点部署/解析使用的数据库导入执行文件。
- `bin/surreal/surreal.exe`：随包分发的 SurrealDB。
- `bin/nginx/nginx.exe`：随包分发的 Nginx。默认启动会准备 Nginx 配置；站点启动阶段会配置/启动 Nginx 根入口。显式 `/EnableNginx auto` 时允许回退到 `/viewer/` fallback。
- `resource/surreal/`：初始化函数与 `att_meta` 属性元数据脚本。
- `viewer/`：plant3d-web 静态前端，使用 `/viewer/` base，挂载在内置 fallback `/viewer/`。
- `viewer-root/`：plant3d-web 静态前端，使用 `/` base，供 Nginx 客户根入口使用。
- `src/web_server/static/admin/`：站点部署管理后台，挂载在 `/admin`。
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

修改后重新运行 `start-plant3d.bat /Port <新端口>`。
'@ | Set-Content -LiteralPath (Join-Path $profilePackageRoot "README-安装说明.md") -Encoding UTF8
}

@'
# Plant3D AIOS Windows x64 版本说明

本目录按构建 profile 保留可并存的完整安装包：

- `release/`：发布版，适合正式部署和性能验证。
- `debug/`：调试版，适合快速迭代、日志分析和本机问题复现。

进入对应目录后运行：

```bat
start-plant3d.bat
```
'@ | Set-Content -LiteralPath (Join-Path $PackageRoot "README-版本说明.md") -Encoding UTF8

if (-not $SkipZip) {
    Step "Create zip archive"
    if ($BuildProfile -eq "both") {
        $zipName = "$BundleName.zip"
    } else {
        $zipName = "$BundleName-$BuildProfile.zip"
    }
    $zipPath = Join-Path $OutputRoot $zipName
    if (Test-Path -LiteralPath $zipPath) {
        Remove-Item -LiteralPath $zipPath -Force
    }
    Compress-Archive -LiteralPath $PackageRoot -DestinationPath $zipPath -Force
    Write-Host "ZIP: $zipPath" -ForegroundColor Green
}

Write-Host ""
Write-Host "Package ready: $PackageRoot" -ForegroundColor Green
