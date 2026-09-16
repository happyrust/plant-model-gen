<#
  PDMS ptset 批量接口冒烟验证（通过 web-server HTTP POST）

  覆盖：
    - 单 refno 查询
    - 混合请求（有效 / 无数据 / 非法 refno）
    - 重复 refno 保序返回
    - 空数组

  前置条件：
    - web-server 已启动

  用法：
    .\scripts\verify_pdms_ptset_batch_api.ps1
    .\scripts\verify_pdms_ptset_batch_api.ps1 -BaseUrl "http://127.0.0.1:3333"
    .\scripts\verify_pdms_ptset_batch_api.ps1 -ValidDataRefno "24381_145018"
    .\scripts\verify_pdms_ptset_batch_api.ps1 -SkipDataCheck
#>

param(
    [string]$BaseUrl = "http://localhost:3100",
    [string]$Refno = "17496_152153",
    [string]$NoDataRefno = "17496_152153",
    [string]$ValidDataRefno = "",
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
        return ($Object | ConvertTo-Json -Depth 10 -Compress)
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

function Invoke-BatchQuery {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [string[]]$Refnos
    )

    $payload = @{
        refnos = $Refnos
    }

    $url = "$BaseUrl/api/pdms/ptset/batch-query"
    $body = $payload | ConvertTo-Json -Depth 6
    $json = Invoke-RestMethod -Uri $url -Method Post -ContentType "application/json" -Body $body -ErrorAction Stop

    return [pscustomobject]@{
        Url     = $url
        Payload = $payload
        Json    = $json
    }
}

function Test-BatchEnvelope {
    param(
        $Json,
        [int]$ExpectedCount
    )

    foreach ($name in @('success', 'results', 'total_count', 'success_count', 'failed_count')) {
        if (-not (Has-Property $Json $name)) {
            return New-CheckResult $false ("missing {0}" -f $name)
        }
    }

    if (-not ($Json.results -is [System.Array])) {
        return New-CheckResult $false "results is not an array"
    }

    if ($Json.results.Count -ne $ExpectedCount) {
        return New-CheckResult $false ("results count mismatch: expected {0}, actual {1}" -f $ExpectedCount, $Json.results.Count)
    }

    if ($Json.total_count -ne $Json.results.Count) {
        return New-CheckResult $false ("total_count mismatch: total_count={0}, results={1}" -f $Json.total_count, $Json.results.Count)
    }

    if (($Json.success_count + $Json.failed_count) -ne $Json.total_count) {
        return New-CheckResult $false ("counter mismatch: success_count + failed_count != total_count")
    }

    if (-not $Json.success) {
        return New-CheckResult $false "top-level success=false"
    }

    return New-CheckResult $true
}

function Test-SingleCase {
    param(
        $Json,
        [string]$ExpectedInput,
        [bool]$Skip,
        [bool]$ExpectSuccess
    )

    $envelope = Test-BatchEnvelope -Json $Json -ExpectedCount 1
    if (-not $envelope.Pass) {
        return $envelope
    }

    $item = $Json.results[0]
    foreach ($name in @('input_refno', 'success', 'ptset', 'error_message')) {
        if (-not (Has-Property $item $name)) {
            return New-CheckResult $false ("single item missing {0}" -f $name)
        }
    }

    if ($item.input_refno -ne $ExpectedInput) {
        return New-CheckResult $false ("single item input_refno mismatch: expected {0}, actual {1}" -f $ExpectedInput, $item.input_refno)
    }

    if (-not ($item.ptset -is [System.Array])) {
        return New-CheckResult $false "single item ptset is not an array"
    }

    if ($Skip) {
        return New-CheckResult $true
    }

    if ($ExpectSuccess) {
        if (-not $item.success) {
            return New-CheckResult $false ("expected success=true, got error: {0}" -f $item.error_message)
        }

        return New-CheckResult $true
    }

    if (-not $item.success) {
        return New-CheckResult $true ("route reachable but refno has no ptset data: {0}" -f $item.error_message)
    }

    return New-CheckResult $true
}

function Test-MixedCase {
    param(
        $Json,
        [string]$ExpectedFirst,
        [string]$ExpectedSecond,
        [bool]$Skip,
        [bool]$ExpectFirstSuccess
    )

    $envelope = Test-BatchEnvelope -Json $Json -ExpectedCount 3
    if (-not $envelope.Pass) {
        return $envelope
    }

    $items = $Json.results
    if ($items[0].input_refno -ne $ExpectedFirst -or $items[1].input_refno -ne $ExpectedSecond -or $items[2].input_refno -ne 'invalid_refno') {
        return New-CheckResult $false "mixed case input order mismatch"
    }

    if (-not ($items[0].ptset -is [System.Array])) {
        return New-CheckResult $false "mixed case first item ptset is not an array"
    }

    if (-not ($items[1].ptset -is [System.Array])) {
        return New-CheckResult $false "mixed case second item ptset is not an array"
    }

    if ($items[2].success -or $null -ne $items[2].refno) {
        return New-CheckResult $false "invalid refno item should be success=false and refno=null"
    }

    if ($Skip) {
        return New-CheckResult $true
    }

    if ($ExpectFirstSuccess -and -not $items[0].success) {
        return New-CheckResult $false ("mixed case first item should succeed: {0}" -f $items[0].error_message)
    }

    if ($items[1].success) {
        return New-CheckResult $false "mixed case no-data item should be success=false"
    }

    return New-CheckResult $true
}

function Test-DuplicateCase {
    param(
        $Json,
        [string]$ExpectedInput
    )

    $envelope = Test-BatchEnvelope -Json $Json -ExpectedCount 2
    if (-not $envelope.Pass) {
        return $envelope
    }

    $items = $Json.results
    if ($items[0].input_refno -ne $ExpectedInput -or $items[1].input_refno -ne $ExpectedInput) {
        return New-CheckResult $false "duplicate case did not preserve repeated input_refno"
    }

    if (-not ($items[0].ptset -is [System.Array]) -or -not ($items[1].ptset -is [System.Array])) {
        return New-CheckResult $false "duplicate case ptset is not an array"
    }

    return New-CheckResult $true
}

function Test-EmptyCase {
    param($Json)

    $envelope = Test-BatchEnvelope -Json $Json -ExpectedCount 0
    if (-not $envelope.Pass) {
        return $envelope
    }

    if ($Json.success_count -ne 0 -or $Json.failed_count -ne 0) {
        return New-CheckResult $false "empty case counters should all be zero"
    }

    return New-CheckResult $true
}

$BaseUrl = $BaseUrl.TrimEnd('/')
$singleRefno = if ([string]::IsNullOrWhiteSpace($ValidDataRefno)) { $Refno } else { $ValidDataRefno }
$expectSingleSuccess = -not [string]::IsNullOrWhiteSpace($ValidDataRefno)

$cases = @(
    @{
        Name    = 'single'
        Invoke  = { Invoke-BatchQuery -Refnos @($singleRefno) }
        Validate = {
            param($Json)
            Test-SingleCase -Json $Json -ExpectedInput $singleRefno -Skip ([bool]$SkipDataCheck) -ExpectSuccess $expectSingleSuccess
        }
    },
    @{
        Name    = 'mixed'
        Invoke  = { Invoke-BatchQuery -Refnos @($singleRefno, $NoDataRefno, 'invalid_refno') }
        Validate = {
            param($Json)
            Test-MixedCase -Json $Json -ExpectedFirst $singleRefno -ExpectedSecond $NoDataRefno -Skip ([bool]$SkipDataCheck) -ExpectFirstSuccess $expectSingleSuccess
        }
    },
    @{
        Name    = 'duplicates'
        Invoke  = { Invoke-BatchQuery -Refnos @($singleRefno, $singleRefno) }
        Validate = {
            param($Json)
            Test-DuplicateCase -Json $Json -ExpectedInput $singleRefno
        }
    },
    @{
        Name    = 'empty'
        Invoke  = { Invoke-BatchQuery -Refnos @() }
        Validate = {
            param($Json)
            Test-EmptyCase -Json $Json
        }
    }
)

Write-Host ("=" * 64)
Write-Host "  PDMS ptset batch API verification"
Write-Host "  Server       : $BaseUrl"
Write-Host "  Single Refno : $singleRefno"
Write-Host "  NoData Refno : $NoDataRefno"
Write-Host "  Mode         : " -NoNewline
if ($SkipDataCheck) {
    Write-Host "route + JSON structure only" -ForegroundColor Yellow
}
elseif ($expectSingleSuccess) {
    Write-Host "full response validation" -ForegroundColor Green
}
else {
    Write-Host "structure + mixed failure semantics" -ForegroundColor Yellow
}
Write-Host ("=" * 64)

$passed = 0
$failed = 0

foreach ($case in $cases) {
    Write-Host ""
    Write-Host ("-" * 64)
    Write-Host ("  {0}" -f $case.Name)

    try {
        $result = & $case.Invoke
    }
    catch {
        Write-Host ("   [FAIL] request error: {0}" -f $_.Exception.Message) -ForegroundColor Red
        Write-Host "   hint: ensure web-server is running and route is registered"
        $failed++
        continue
    }

    Write-Host ("  POST {0}" -f $result.Url)
    Write-Host ("  payload = {0}" -f (As-JsonSnippet $result.Payload))

    $check = & $case.Validate $result.Json
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
    Write-Host ("   payload = {0}" -f (As-JsonSnippet $result.Json))
    $failed++
}

Write-Host ""
Write-Host ("=" * 64)
Write-Host ("  Result: {0} passed, {1} failed (total {2})" -f $passed, $failed, $cases.Count)
Write-Host ("=" * 64)

if ($failed -gt 0) {
    exit 1
}
