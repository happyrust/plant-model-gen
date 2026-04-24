param(
    [string]$BaseUrl = 'http://127.0.0.1:3100',
    [string]$BranchRefno = '24381_145018'
)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$OutputEncoding = [Console]::OutputEncoding

function Normalize-Refno([string]$Value) {
    if ($null -eq $Value) {
        return ''
    }
    return $Value.Trim().Replace('/', '_')
}

function Invoke-JsonGet([string]$Path) {
    return Invoke-RestMethod -Uri ($BaseUrl.TrimEnd('/') + $Path) -Method Get
}

function Invoke-JsonPost([string]$Path, $Body) {
    $json = $Body | ConvertTo-Json -Depth 8
    return Invoke-RestMethod -Uri ($BaseUrl.TrimEnd('/') + $Path) -Method Post -ContentType 'application/json' -Body $json
}

$branchKey = Normalize-Refno $BranchRefno
Write-Host "=== Ptset Verify For BRAN $branchKey ===" -ForegroundColor Cyan

$childrenResp = Invoke-JsonGet "/api/e3d/children/${branchKey}?limit=2000"
if (-not $childrenResp.success) {
    throw "children api failed: $($childrenResp.error_message)"
}

$childItems = @()
if ($null -ne $childrenResp.children) {
    foreach ($child in @($childrenResp.children)) {
        $childItems += $child
    }
}
$childrenCount = ($childItems | Measure-Object).Count
Write-Host "children_count=$childrenCount" -ForegroundColor Yellow
if ($childrenCount -eq 0) {
    Write-Host "children api raw response:" -ForegroundColor DarkYellow
    $childrenResp | ConvertTo-Json -Depth 6
    Write-Host "no direct children found" -ForegroundColor Yellow
    exit 0
}

$childRefnos = @($childItems | ForEach-Object { Normalize-Refno $_.refno })
$batchResp = Invoke-JsonPost '/api/pdms/ptset/batch-query' @{
    refnos = $childRefnos
}

if (-not $batchResp.success) {
    throw 'batch ptset api failed'
}

$resultByInput = @{}
foreach ($item in @($batchResp.results)) {
    $key = Normalize-Refno $item.input_refno
    if ($key) {
        $resultByInput[$key] = $item
    }
}

$rows = @()
foreach ($child in $childItems) {
    $childKey = Normalize-Refno $child.refno
    $item = $resultByInput[$childKey]
    $ptCount = 0
    if ($item -and $item.ptset) {
        $ptCount = @($item.ptset).Count
    }
    $rows += [PSCustomObject]@{
        refno = $childKey
        noun = $child.noun
        name = $child.name
        success = [bool]($item.success)
        pt_count = $ptCount
        error_message = $item.error_message
    }
}

$rows | Format-Table -AutoSize

$successRows = @($rows | Where-Object { $_.success -and $_.pt_count -gt 0 })
Write-Host ""
Write-Host "success_count=$($successRows.Count) total_count=$($rows.Count)" -ForegroundColor Yellow

if ($successRows.Count -gt 0) {
    $sample = $successRows[0]
    Write-Host "sample_refno=$($sample.refno)" -ForegroundColor Green
    $single = Invoke-JsonGet "/api/pdms/ptset/$($sample.refno)"
    [PSCustomObject]@{
        refno = $single.refno
        success = $single.success
        pt_count = @($single.ptset).Count
        has_world_transform = [bool]$single.world_transform
        error_message = $single.error_message
    } | Format-List
}

Write-Host "=== Verify Finished ===" -ForegroundColor Cyan
