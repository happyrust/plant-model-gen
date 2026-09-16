# 验证 dbnum=7997 中的数据（24381_145018 的实际 dbnum）

$headers = @{
    'Accept' = 'application/json'
    'surreal-ns' = '1516'
    'surreal-db' = 'AvevaMarineSample'
    'Authorization' = 'Basic cm9vdDpyb290'
}

Write-Host ""
Write-Host "========== 验证 dbnum=7997 中的数据 (24381_145018 的实际 dbnum) ==========" -ForegroundColor Cyan
Write-Host ""

# 1. 查询 dbnum=7997 的所有 inst_relate 记录（通过 pe 表的 dbnum 字段）
Write-Host "1. 查询 dbnum=7997 的 inst_relate 记录数量:" -ForegroundColor Yellow
$sql1 = "SELECT VALUE count() FROM inst_relate WHERE in.dbnum = 7997;"
try {
    $r = Invoke-RestMethod -Uri "http://localhost:8020/sql" -Method Post -Headers $headers -Body $sql1 -ContentType "text/plain"
    $cnt = $r.result[0]
    Write-Host "   count = $cnt" -ForegroundColor Green
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
}

# 2. 查询 24381_145018 的 inst_relate（使用 pe key）
Write-Host ""
Write-Host "2. 查询 pe:`24381_145018` 的 inst_relate:" -ForegroundColor Yellow
$peKey = 'pe:`24381_145018`'
$sql2 = "SELECT VALUE count() FROM inst_relate WHERE in = $peKey;"
try {
    $r = Invoke-RestMethod -Uri "http://localhost:8020/sql" -Method Post -Headers $headers -Body $sql2 -ContentType "text/plain"
    $cnt = $r.result[0]
    Write-Host "   count = $cnt" -ForegroundColor $(if ($cnt -gt 0) { "Green" } else { "Yellow" })
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
}

# 3. 查询 24381_145018 的详细 inst_relate 记录
Write-Host ""
Write-Host "3. 查询 pe:`24381_145018` 的详细 inst_relate 记录:" -ForegroundColor Yellow
$sql3 = "SELECT in, out, owner FROM inst_relate WHERE in = $peKey LIMIT 5;"
try {
    $r = Invoke-RestMethod -Uri "http://localhost:8020/sql" -Method Post -Headers $headers -Body $sql3 -ContentType "text/plain"
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
Write-Host "4. 查询 tubi_relate (ID Range):" -ForegroundColor Yellow
$sql4 = "SELECT VALUE count() FROM tubi_relate:[$peKey, 0]..[$peKey, ..];"
try {
    $r = Invoke-RestMethod -Uri "http://localhost:8020/sql" -Method Post -Headers $headers -Body $sql4 -ContentType "text/plain"
    $cnt = $r.result[0]
    Write-Host "   count = $cnt (预期 11)" -ForegroundColor $(if ($cnt -eq 11) { "Green" } else { "Yellow" })
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
}

# 5. 查询 tubi_relate 详细记录
Write-Host ""
Write-Host "5. 查询 tubi_relate 详细记录:" -ForegroundColor Yellow
$sql5 = "SELECT id[0] as bran, id[1] as idx, in as leave, out as arrive FROM tubi_relate:[$peKey, 0]..[$peKey, ..] LIMIT 5;"
try {
    $r = Invoke-RestMethod -Uri "http://localhost:8020/sql" -Method Post -Headers $headers -Body $sql5 -ContentType "text/plain"
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

# 6. 验证 pe 表记录
Write-Host ""
Write-Host "6. 验证 pe 表记录:" -ForegroundColor Yellow
$sql6 = "SELECT id, noun, name, dbnum FROM $peKey;"
try {
    $r = Invoke-RestMethod -Uri "http://localhost:8020/sql" -Method Post -Headers $headers -Body $sql6 -ContentType "text/plain"
    if ($r.result -and $r.result[0] -and $r.result[0].Count -gt 0) {
        $pe = $r.result[0][0]
        Write-Host "   ✅ pe 记录存在: id=$($pe.id), noun=$($pe.noun), name=$($pe.name), dbnum=$($pe.dbnum)" -ForegroundColor Green
    } else {
        Write-Host "   ⚠️ pe 记录不存在" -ForegroundColor Yellow
    }
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
}

Write-Host ""
Write-Host "========== 验证结束 ==========" -ForegroundColor Cyan
