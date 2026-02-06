# 验证 --sync-to-db 后 SurrealDB 中 24381/145018 相关表的数据
# 用法: 确保 SurrealDB 已启动且已执行过 aios-database --debug-model 24381/145018 --regen-model --export-obj --sync-to-db

$ErrorActionPreference = "Stop"
$baseUrl = "http://localhost:8020/sql"
$headers = @{
    'Accept'         = 'application/json'
    'surreal-ns'     = '1516'
    'surreal-db'     = 'AvevaMarineSample'
    'Authorization'  = 'Basic cm9vdDpyb290'
}

function Invoke-SurrealQuery {
    param([string]$Sql)
    $result = Invoke-RestMethod -Uri $baseUrl -Method Post -Headers $headers -Body $Sql -ContentType 'text/plain'
    return $result
}

# 从 SurrealDB 响应中取出 count
# SELECT VALUE count() 返回 [N] 或 [{count: N}]
# SELECT count() AS cnt 返回 [{cnt: N}]
function Get-CountFromResult($r) {
    if (-not $r.result) { return $null }
    $first = $r.result[0]
    # 如果是数组，取第一个元素
    if ($first -is [Array] -and $first.Count -gt 0) {
        $val = $first[0]
        if ($val -is [PSCustomObject]) {
            if ($val.count) { return $val.count }
            if ($val.cnt) { return $val.cnt }
        }
        return $val
    }
    # 如果是对象
    if ($first -is [PSCustomObject]) {
        if ($first.count) { return $first.count }
        if ($first.cnt) { return $first.cnt }
    }
    # 直接是数字
    if ($first -is [int] -or $first -is [long]) { return $first }
    return $null
}

Write-Host ""
Write-Host "========== 验证 sync-to-db 结果 (24381/145018) ==========" -ForegroundColor Cyan

# 1. inst_relate: 调试范围内 refno (24381_145018 .. 24381_145035) 应有记录
# pe key 格式: pe:`24381_145018` (带反引号，在 PowerShell 中用单引号避免转义)
$peList = (145018..145035) | ForEach-Object { 'pe:`24381_' + $_ + '`' }
$peListStr = $peList -join ","
$sqlInstRelate = "SELECT VALUE count() FROM inst_relate WHERE in IN [$peListStr];"
Write-Host ""
Write-Host "1. inst_relate (in 为 24381_145018..145035):" -ForegroundColor Yellow
try {
    $r = Invoke-SurrealQuery -Sql $sqlInstRelate
    $cnt = Get-CountFromResult $r
    Write-Host "   count = $cnt" -ForegroundColor $(if ($cnt -gt 0) { "Green" } else { "Gray" })
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
}

# 2. geo_relate 总数（与 inst 相关）
$sqlGeoRelate = "SELECT VALUE count() FROM geo_relate;"
Write-Host ""
Write-Host "2. geo_relate 总条数:" -ForegroundColor Yellow
try {
    $r = Invoke-SurrealQuery -Sql $sqlGeoRelate
    $cnt = Get-CountFromResult $r
    Write-Host "   count = $cnt" -ForegroundColor $(if ($cnt -gt 0) { "Green" } else { "Gray" })
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
}

# 3. neg_relate 总条数
$sqlNegRelate = "SELECT VALUE count() FROM neg_relate;"
Write-Host ""
Write-Host "3. neg_relate 总条数:" -ForegroundColor Yellow
try {
    $r = Invoke-SurrealQuery -Sql $sqlNegRelate
    $cnt = Get-CountFromResult $r
    Write-Host "   count = $cnt" -ForegroundColor $(if ($cnt -gt 0) { "Green" } else { "Gray" })
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
}

# 4. ngmr_relate 总条数
$sqlNgmrRelate = "SELECT VALUE count() FROM ngmr_relate;"
Write-Host ""
Write-Host "4. ngmr_relate 总条数:" -ForegroundColor Yellow
try {
    $r = Invoke-SurrealQuery -Sql $sqlNgmrRelate
    $cnt = Get-CountFromResult $r
    Write-Host "   count = $cnt" -ForegroundColor $(if ($cnt -gt 0) { "Green" } else { "Gray" })
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
}

# 5. tubi_relate: BRAN 24381_145018 使用 ID Range 查询（推荐方式）
# pe key 格式: pe:`24381_145018` (带反引号，在 PowerShell 中用单引号避免转义)
$branPeKey = 'pe:`24381_145018`'
$sqlTubi = "SELECT VALUE count() FROM tubi_relate:[$branPeKey, 0]..[$branPeKey, ..];"
Write-Host ""
Write-Host "5. tubi_relate (BRAN $branPeKey, 使用 ID Range):" -ForegroundColor Yellow
try {
    $r = Invoke-SurrealQuery -Sql $sqlTubi
    $cnt = Get-CountFromResult $r
    $ok = ($cnt -eq 11)
    Write-Host "   count = $cnt (预期 11)" -ForegroundColor $(if ($ok) { "Green" } else { "Yellow" })
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
}

# 6. inst_relate_aabb: 上述 refno 中应有部分带 aabb
$sqlAabb = "SELECT VALUE count() FROM inst_relate_aabb WHERE in IN [$peListStr];"
Write-Host ""
Write-Host "6. inst_relate_aabb (in 为 24381_145018..145035):" -ForegroundColor Yellow
try {
    $r = Invoke-SurrealQuery -Sql $sqlAabb
    $cnt = Get-CountFromResult $r
    Write-Host "   count = $cnt" -ForegroundColor $(if ($cnt -gt 0) { "Green" } else { "Gray" })
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
}

Write-Host ""
Write-Host "========== 验证结束 ==========" -ForegroundColor Cyan
