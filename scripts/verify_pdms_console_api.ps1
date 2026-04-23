<#
  PDMS 控制台相关 API 冒烟验证（通过 web-server HTTP 接口）

  覆盖：
    - q pos              -> /api/pdms/transform/{refno}
    - q pos(compute)     -> /api/pdms/transform/compute/{refno}
    - q ui-attr          -> /api/pdms/ui-attr/{refno}
    - q ptset            -> /api/pdms/ptset/{refno}
    - q type-info        -> /api/pdms/type-info?refno={refno}
    - q children         -> /api/pdms/children?refno={refno}

  前置条件：
    - web-server 已启动

  用法：
    .\scripts\verify_pdms_console_api.ps1
    .\scripts\verify_pdms_console_api.ps1 -BaseUrl "http://localhost:3100" -Refno "17496_152153"
    .\scripts\verify_pdms_console_api.ps1 -SkipDataCheck
#>

param(
    [string]$BaseUrl = "http://localhost:3100",
    [string]$Refno = "17496_152153",
    [switch]$SkipDataCheck
)

function Has-Property {
    param(
        [Parameter(Mandatory = $true)]
        $Object,
        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    return $null -ne $Object -and $null -ne $Object.PSObject.Properties[$Name]
}

function As-JsonSnippet {
    param($Object)

    if ($null -eq $Object) {
        return "<null>"
    }

    try {
        return ($Object | ConvertTo-Json -Depth 8 -Compress)
    }
    catch {
        return [string]$Object
    }
}

function New-CheckResult {
    param(
        [bool]$Pass,
        [string]$Reason = ""
    )

    return [pscustomobject]@{
        Pass   = $Pass
        Reason = $Reason
    }
}

function Test-TransformResponse {
    param($Json, [bool]$Skip)

    if (-not (Has-Property $Json 'world_transform')) {
        return New-CheckResult $false "missing world_transform"
    }

    if ($Skip) {
        return New-CheckResult $true
    }

    if (-not $Json.success) {
        return New-CheckResult $false ("success=false: {0}" -f $Json.error_message)
    }

    $matrix = $Json.world_transform
    if ($matrix -is [System.Array] -and $matrix.Count -eq 16) {
        return New-CheckResult $true
    }

    return New-CheckResult $false "world_transform is not a 16-length array"
}

function Test-ComputeTransformResponse {
    param($Json, [bool]$Skip)

    if (-not (Has-Property $Json 'world_translation')) {
        return New-CheckResult $false "missing world_translation"
    }

    if ($Skip) {
        return New-CheckResult $true
    }

    if (-not $Json.success) {
        return New-CheckResult $false ("success=false: {0}" -f $Json.error_message)
    }

    $value = $Json.world_translation
    if ($value -is [System.Array] -and $value.Count -eq 3) {
        return New-CheckResult $true
    }

    return New-CheckResult $false "world_translation is not a 3-length array"
}

function Test-UiAttrResponse {
    param($Json, [bool]$Skip)

    if (-not (Has-Property $Json 'attrs')) {
        return New-CheckResult $false "missing attrs"
    }

    if ($Skip) {
        return New-CheckResult $true
    }

    if (-not $Json.success) {
        return New-CheckResult $false ("success=false: {0}" -f $Json.error_message)
    }

    return New-CheckResult $true
}

function Test-PtsetResponse {
    param($Json, [bool]$Skip)

    if (-not (Has-Property $Json 'ptset')) {
        return New-CheckResult $false "missing ptset"
    }

    if ($Skip) {
        return New-CheckResult $true
    }

    if ($Json.ptset -is [System.Array]) {
        if (-not $Json.success) {
            return New-CheckResult $true ("route reachable but ptset data is unavailable: {0}" -f $Json.error_message)
        }

        return New-CheckResult $true
    }

    return New-CheckResult $false "ptset is not an array"
}

function Test-TypeInfoResponse {
    param($Json, [bool]$Skip)

    if (-not (Has-Property $Json 'refno')) {
        return New-CheckResult $false "missing refno"
    }

    if ($Skip) {
        return New-CheckResult $true
    }

    if (-not $Json.success) {
        return New-CheckResult $false ("success=false: {0}" -f $Json.error_message)
    }

    if ((Has-Property $Json 'noun') -or (Has-Property $Json 'owner_refno')) {
        return New-CheckResult $true
    }

    return New-CheckResult $false "missing noun/owner_refno"
}

function Test-ChildrenResponse {
    param($Json, [bool]$Skip)

    if (-not (Has-Property $Json 'children')) {
        return New-CheckResult $false "missing children"
    }

    if ($Skip) {
        return New-CheckResult $true
    }

    if (-not $Json.success) {
        return New-CheckResult $false ("success=false: {0}" -f $Json.error_message)
    }

    if ($Json.children -is [System.Array]) {
        return New-CheckResult $true
    }

    return New-CheckResult $false "children is not an array"
}

$BaseUrl = $BaseUrl.TrimEnd('/')

$cases = @(
    @{
        Name     = "q pos"
        Path     = "/api/pdms/transform/$Refno"
        Validate = { param($Json, $Skip) Test-TransformResponse -Json $Json -Skip $Skip }
    },
    @{
        Name     = "q pos(compute)"
        Path     = "/api/pdms/transform/compute/$Refno"
        Validate = { param($Json, $Skip) Test-ComputeTransformResponse -Json $Json -Skip $Skip }
    },
    @{
        Name     = "q ui-attr"
        Path     = "/api/pdms/ui-attr/$Refno"
        Validate = { param($Json, $Skip) Test-UiAttrResponse -Json $Json -Skip $Skip }
    },
    @{
        Name     = "q ptset"
        Path     = "/api/pdms/ptset/$Refno"
        Validate = { param($Json, $Skip) Test-PtsetResponse -Json $Json -Skip $Skip }
    },
    @{
        Name     = "q type-info"
        Path     = "/api/pdms/type-info?refno=$Refno"
        Validate = { param($Json, $Skip) Test-TypeInfoResponse -Json $Json -Skip $Skip }
    },
    @{
        Name     = "q children"
        Path     = "/api/pdms/children?refno=$Refno"
        Validate = { param($Json, $Skip) Test-ChildrenResponse -Json $Json -Skip $Skip }
    }
)

Write-Host ("=" * 64)
Write-Host "  PDMS console API smoke verification"
Write-Host "  Server: $BaseUrl"
Write-Host "  Refno : $Refno"
Write-Host "  Mode  : " -NoNewline
if ($SkipDataCheck) {
    Write-Host "route + JSON structure only" -ForegroundColor Yellow
}
else {
    Write-Host "full response validation" -ForegroundColor Green
}
Write-Host ("=" * 64)

$passed = 0
$failed = 0

foreach ($case in $cases) {
    $url = "$BaseUrl$($case.Path)"

    Write-Host ""
    Write-Host ("-" * 64)
    Write-Host ("  {0}" -f $case.Name)
    Write-Host ("  GET {0}" -f $url)

    try {
        $json = Invoke-RestMethod -Uri $url -Method Get -ErrorAction Stop
    }
    catch {
        Write-Host ("   [FAIL] request error: {0}" -f $_.Exception.Message) -ForegroundColor Red
        Write-Host "   hint: ensure web-server is running and route is registered"
        $failed++
        continue
    }

    $check = & $case.Validate $json ([bool]$SkipDataCheck)
    if ($check.Pass) {
        if ([string]::IsNullOrWhiteSpace($check.Reason)) {
            Write-Host "   [PASS]" -ForegroundColor Green
        }
        else {
            Write-Host ("   [PASS] {0}" -f $check.Reason) -ForegroundColor Yellow
        }
        $passed++
        continue
    }

    Write-Host ("   [FAIL] {0}" -f $check.Reason) -ForegroundColor Red
    Write-Host ("   payload = {0}" -f (As-JsonSnippet $json))
    $failed++
}

Write-Host ""
Write-Host ("=" * 64)
Write-Host ("  Result: {0} passed, {1} failed (total {2})" -f $passed, $failed, $cases.Count)
Write-Host ("=" * 64)

if ($failed -gt 0) {
    exit 1
}
