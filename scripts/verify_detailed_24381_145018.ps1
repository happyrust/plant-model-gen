# 详细验证 24381_145018 的模型生成后回写数据库情况
# 基于 plant-surrealdb skill 的最佳实践

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

function Get-CountFromResult($r) {
    if (-not $r.result) { return $null }
    $first = $r.result[0]
    if ($first -is [Array] -and $first.Count -gt 0) {
        $val = $first[0]
        if ($val -is [PSCustomObject]) {
            if ($val.count) { return $val.count }
            if ($val.cnt) { return $val.cnt }
        }
        return $val
    }
    if ($first -is [PSCustomObject]) {
        if ($first.count) { return $first.count }
        if ($first.cnt) { return $first.cnt }
    }
    if ($first -is [int] -or $first -is [long]) { return $first }
    return $null
}

Write-Host ""
Write-Host "========== 详细验证 24381_145018 模型回写数据库 ==========" -ForegroundColor Cyan
Write-Host ""

# 1. 首先验证 pe 表记录是否存在（确认 pe key 格式）
Write-Host "1. 验证 pe 表记录是否存在:" -ForegroundColor Yellow
$peKey = 'pe:`24381_145018`'
$sqlPe = "SELECT id, noun, name, dbnum FROM $peKey;"
try {
    $r = Invoke-SurrealQuery -Sql $sqlPe
    if ($r.result -and $r.result[0] -and $r.result[0].Count -gt 0) {
        $pe = $r.result[0][0]
        Write-Host "   ✅ pe 记录存在: id=$($pe.id), noun=$($pe.noun), name=$($pe.name), dbnum=$($pe.dbnum)" -ForegroundColor Green
    } else {
        Write-Host "   ⚠️ pe 记录不存在" -ForegroundColor Yellow
    }
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
}

# 2. 查询 inst_relate 数量（单个 refno）
Write-Host ""
Write-Host "2. inst_relate 数量 (in = $peKey):" -ForegroundColor Yellow
$sqlInstRelate = "SELECT VALUE count() FROM inst_relate WHERE in = $peKey;"
try {
    $r = Invoke-SurrealQuery -Sql $sqlInstRelate
    $cnt = Get-CountFromResult $r
    Write-Host "   count = $cnt" -ForegroundColor $(if ($cnt -gt 0) { "Green" } else { "Yellow" })
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
}

# 3. 查询 inst_relate 详细记录
Write-Host ""
Write-Host "3. inst_relate 详细记录 (前 5 条):" -ForegroundColor Yellow
$sqlInstDetail = "SELECT in, out, owner FROM inst_relate WHERE in = $peKey LIMIT 5;"
try {
    $r = Invoke-SurrealQuery -Sql $sqlInstDetail
    if ($r.result -and $r.result[0] -and $r.result[0].Count -gt 0) {
        Write-Host "   找到 $($r.result[0].Count) 条记录:" -ForegroundColor Green
        $r.result[0] | ForEach-Object -Begin { $i = 1 } -Process {
            Write-Host "   记录 $i : in=$($_.in), out=$($_.out), owner=$($_.owner)"
            $i++
        }
    } else {
        Write-Host "   ⚠️ 未找到记录" -ForegroundColor Yellow
    }
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
}

# 4. 查询 tubi_relate（使用 ID Range）
Write-Host ""
Write-Host "4. tubi_relate 数量 (使用 ID Range):" -ForegroundColor Yellow
$sqlTubi = "SELECT VALUE count() FROM tubi_relate:[$peKey, 0]..[$peKey, ..];"
try {
    $r = Invoke-SurrealQuery -Sql $sqlTubi
    $cnt = Get-CountFromResult $r
    Write-Host "   count = $cnt (预期 11)" -ForegroundColor $(if ($cnt -eq 11) { "Green" } else { "Yellow" })
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
}

# 5. 查询 tubi_relate 详细记录
Write-Host ""
Write-Host "5. tubi_relate 详细记录 (前 5 条):" -ForegroundColor Yellow
$sqlTubiDetail = "SELECT id[0] as bran, id[1] as idx, in as leave, out as arrive FROM tubi_relate:[$peKey, 0]..[$peKey, ..] LIMIT 5;"
try {
    $r = Invoke-SurrealQuery -Sql $sqlTubiDetail
    if ($r.result -and $r.result[0] -and $r.result[0].Count -gt 0) {
        Write-Host "   找到 $($r.result[0].Count) 条记录:" -ForegroundColor Green
        $r.result[0] | ForEach-Object -Begin { $i = 1 } -Process {
            Write-Host "   记录 $i : bran=$($_.bran), idx=$($_.idx), leave=$($_.leave), arrive=$($_.arrive)"
            $i++
        }
    } else {
        Write-Host "   ⚠️ 未找到记录" -ForegroundColor Yellow
    }
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
}

# 6. 查询所有 inst_relate 总数（用于对比）
Write-Host ""
Write-Host "6. 数据库中 inst_relate 总记录数:" -ForegroundColor Yellow
$sqlTotal = "SELECT VALUE count() FROM inst_relate;"
try {
    $r = Invoke-SurrealQuery -Sql $sqlTotal
    $total = Get-CountFromResult $r
    Write-Host "   总记录数 = $total" -ForegroundColor Cyan
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
}

# 7. 查询最近的 inst_relate 记录（按时间排序）
Write-Host ""
Write-Host "7. 最近的 inst_relate 记录 (按 dt 排序，前 5 条):" -ForegroundColor Yellow
$sqlRecent = "SELECT in, out, dt FROM inst_relate ORDER BY dt DESC LIMIT 5;"
try {
    $r = Invoke-SurrealQuery -Sql $sqlRecent
    if ($r.result -and $r.result[0] -and $r.result[0].Count -gt 0) {
        Write-Host "   最近的记录:" -ForegroundColor Green
        $r.result[0] | ForEach-Object -Begin { $i = 1 } -Process {
            Write-Host "   记录 $i : in=$($_.in), out=$($_.out), dt=$($_.dt)"
            $i++
        }
    } else {
        Write-Host "   ⚠️ 未找到记录" -ForegroundColor Yellow
    }
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
}

Write-Host ""
Write-Host "========== 验证结束 ==========" -ForegroundColor Cyan
