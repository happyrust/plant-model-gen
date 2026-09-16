# 验证 SurrealDB 中 24381/145018 相关表的数据
# 用法:
#   1) 传统用法（8020 上的 RocksDB 库，--sync-to-db 之后）：
#        powershell -File scripts/verify_sync_to_db_24381_145018.ps1
#   2) 内存调试流程（scripts/debug_bran_mem.ps1 会自动带参调用）：
#        powershell -File scripts/verify_sync_to_db_24381_145018.ps1 -Port 8041 -FailOnMismatch
#
# 默认参数与历史行为完全一致（8020 / ns 1516 / AvevaMarineSample / BRAN 24381_145018）。

param(
    [int]$Port = 8020,
    [string]$SurrealHost = "localhost",
    [string]$Ns = "1516",
    [string]$Db = "AvevaMarineSample",
    [string]$User = "root",
    [string]$Pass = "root",

    # 支持 24381_145018 或 24381/145018 两种写法
    [string]$Refno = "24381_145018",

    # 调试范围内的连续 refno 个数：默认 18 即 145018..145035
    [int]$Span = 18,

    # tubi_relate 预期条数；<0 表示不做预期比对
    [int]$ExpectedTubi = 11,

    # 计数不符合预期时以非零码退出（供编排脚本判定成败）
    [switch]$FailOnMismatch
)

$ErrorActionPreference = "Stop"

$parts = $Refno -split '[_/]'
if ($parts.Count -ne 2) { throw "Refno 格式应为 24381_145018 或 24381/145018，实际: $Refno" }
$dbnum = [int]$parts[0]
$base = [int]$parts[1]

$baseUrl = "http://${SurrealHost}:${Port}/sql"
$basic = [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes("${User}:${Pass}"))
$headers = @{
    'Accept'         = 'application/json'
    'surreal-ns'     = $Ns
    'surreal-db'     = $Db
    'Authorization'  = "Basic $basic"
}

$script:mismatch = 0

function Invoke-SurrealQuery {
    param([string]$Sql)
    $result = Invoke-RestMethod -Uri $baseUrl -Method Post -Headers $headers -Body $Sql -ContentType 'text/plain'
    return $result
}

# 所有计数一律带 GROUP ALL：不带的话 `SELECT VALUE count()` 是逐行返回 1，
# 结果变成 [1,1,1,…]，取首元素就永远是 1（这是本脚本此前一直误报的原因）。
#
# 从 SurrealDB 响应中取出 count
# SELECT VALUE count() ... GROUP ALL 返回 [N] 或 [{count: N}]
function Get-CountFromResult($r) {
    # GROUP ALL 在空表上返回空数组：语句成功 + 无行 = 0，别显示成空白
    if (-not $r.result) {
        if ($r.status -eq 'OK') { return 0 }
        return $null
    }
    $first = $r.result[0]
    # 注意用 $null -ne 判存在，不能直接 if ($x.count)：count 为 0 时是假值，
    # 会被当成"没有这个字段"而漏掉，空表就显示成空白。
    if ($first -is [Array] -and $first.Count -gt 0) {
        $val = $first[0]
        if ($val -is [PSCustomObject]) {
            if ($null -ne $val.count) { return $val.count }
            if ($null -ne $val.cnt) { return $val.cnt }
        }
        return $val
    }
    if ($first -is [PSCustomObject]) {
        if ($null -ne $first.count) { return $first.count }
        if ($null -ne $first.cnt) { return $first.cnt }
    }
    # 直接是数字
    if ($first -is [int] -or $first -is [long]) { return $first }
    return $null
}

Write-Host ""
Write-Host "========== 验证 SurrealDB 数据 ($dbnum/$base @ ${SurrealHost}:${Port} ns=$Ns db=$Db) ==========" -ForegroundColor Cyan

# 1. inst_relate: 调试范围内 refno (base .. base+Span-1) 应有记录
# pe key 格式: pe:`24381_145018` (带反引号，在 PowerShell 中用单引号避免转义)
$peList = ($base..($base + $Span - 1)) | ForEach-Object { 'pe:`' + $dbnum + '_' + $_ + '`' }
$peListStr = $peList -join ","
$sqlInstRelate = "SELECT VALUE count() FROM inst_relate WHERE in IN [$peListStr] GROUP ALL;"
Write-Host ""
Write-Host "1. inst_relate (in 为 ${dbnum}_${base}..$($base + $Span - 1)):" -ForegroundColor Yellow
try {
    $r = Invoke-SurrealQuery -Sql $sqlInstRelate
    $cnt = Get-CountFromResult $r
    Write-Host "   count = $cnt" -ForegroundColor $(if ($cnt -gt 0) { "Green" } else { "Gray" })
    if (-not ($cnt -gt 0)) { $script:mismatch++ }
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
    $script:mismatch++
}

# 2. geo_relate 总数（与 inst 相关）
$sqlGeoRelate = "SELECT VALUE count() FROM geo_relate GROUP ALL;"
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
$sqlNegRelate = "SELECT VALUE count() FROM neg_relate GROUP ALL;"
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
$sqlNgmrRelate = "SELECT VALUE count() FROM ngmr_relate GROUP ALL;"
Write-Host ""
Write-Host "4. ngmr_relate 总条数:" -ForegroundColor Yellow
try {
    $r = Invoke-SurrealQuery -Sql $sqlNgmrRelate
    $cnt = Get-CountFromResult $r
    Write-Host "   count = $cnt" -ForegroundColor $(if ($cnt -gt 0) { "Green" } else { "Gray" })
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
}

# 5. tubi_relate: BRAN 使用 ID Range 查询（推荐方式）
# 与 model_record_id::model_refno_range 同构：tubi_relate:[ref0, ref1, NONE]..=[ref0, ref1, ..]
# （旧的 pe 链接式 id `tubi_relate:[pe:`…`, 0]..` 在数组 record id 改造后恒返回 null）
$branRange = "tubi_relate:[$dbnum, $base, NONE]..=[$dbnum, $base, ..]"
$sqlTubi = "SELECT VALUE count() FROM $branRange GROUP ALL;"
Write-Host ""
Write-Host "5. tubi_relate (BRAN ${dbnum}_${base}, 使用 ID Range):" -ForegroundColor Yellow
try {
    $r = Invoke-SurrealQuery -Sql $sqlTubi
    $cnt = Get-CountFromResult $r
    if ($ExpectedTubi -ge 0) {
        $ok = ($cnt -eq $ExpectedTubi)
        Write-Host "   count = $cnt (预期 $ExpectedTubi)" -ForegroundColor $(if ($ok) { "Green" } else { "Yellow" })
        if (-not $ok) { $script:mismatch++ }
    } else {
        Write-Host "   count = $cnt" -ForegroundColor $(if ($cnt -gt 0) { "Green" } else { "Gray" })
    }
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
    $script:mismatch++
}

# 6. inst_relate_aabb: 上述 refno 中应有部分带 aabb（id 形如 [ref0, ref1]，按区间数）
$sqlAabb = "SELECT VALUE count() FROM inst_relate_aabb:[$dbnum, $base]..=[$dbnum, $($base + $Span - 1)] GROUP ALL;"
Write-Host ""
Write-Host "6. inst_relate_aabb (in 为 ${dbnum}_${base}..$($base + $Span - 1)):" -ForegroundColor Yellow
try {
    $r = Invoke-SurrealQuery -Sql $sqlAabb
    $cnt = Get-CountFromResult $r
    Write-Host "   count = $cnt" -ForegroundColor $(if ($cnt -gt 0) { "Green" } else { "Gray" })
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
}

Write-Host ""
if ($script:mismatch -gt 0) {
    Write-Host "========== 验证结束：$($script:mismatch) 项不符合预期 ==========" -ForegroundColor Yellow
} else {
    Write-Host "========== 验证结束：全部符合预期 ==========" -ForegroundColor Green
}

if ($FailOnMismatch -and $script:mismatch -gt 0) { exit 1 }
