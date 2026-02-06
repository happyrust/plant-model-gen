# 检查数据库中实际存在的记录格式

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

Write-Host ""
Write-Host "========== 检查数据库中实际存在的记录 ==========" -ForegroundColor Cyan
Write-Host ""

# 1. 查询所有 inst_relate 记录
Write-Host "1. 所有 inst_relate 记录:" -ForegroundColor Yellow
$sql1 = "SELECT in, out, owner FROM inst_relate LIMIT 10;"
try {
    $r = Invoke-SurrealQuery -Sql $sql1
    if ($r.result -and $r.result[0] -and $r.result[0].Count -gt 0) {
        Write-Host "   找到 $($r.result[0].Count) 条记录:" -ForegroundColor Green
        $r.result[0] | ForEach-Object -Begin { $i = 1 } -Process {
            Write-Host "   记录 $i :"
            Write-Host "     in = $($_.in)"
            Write-Host "     out = $($_.out)"
            Write-Host "     owner = $($_.owner)"
            Write-Host ""
            $i++
        }
    } else {
        Write-Host "   ⚠️ 未找到记录" -ForegroundColor Yellow
    }
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
    Write-Host "   Response: $($r | ConvertTo-Json -Depth 3)" -ForegroundColor Gray
}

# 2. 查询所有 tubi_relate 记录
Write-Host "2. 所有 tubi_relate 记录:" -ForegroundColor Yellow
$sql2 = "SELECT id, in, out FROM tubi_relate LIMIT 10;"
try {
    $r = Invoke-SurrealQuery -Sql $sql2
    if ($r.result -and $r.result[0] -and $r.result[0].Count -gt 0) {
        Write-Host "   找到 $($r.result[0].Count) 条记录:" -ForegroundColor Green
        $r.result[0] | ForEach-Object -Begin { $i = 1 } -Process {
            Write-Host "   记录 $i :"
            Write-Host "     id = $($_.id)"
            Write-Host "     in = $($_.in)"
            Write-Host "     out = $($_.out)"
            Write-Host ""
            $i++
        }
    } else {
        Write-Host "   ⚠️ 未找到记录" -ForegroundColor Yellow
    }
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
    Write-Host "   Response: $($r | ConvertTo-Json -Depth 3)" -ForegroundColor Gray
}

# 3. 尝试查询 pe 表（使用不同的格式）
Write-Host "3. 尝试查询 pe 表 (不同格式):" -ForegroundColor Yellow

# Format 1: pe:`24381_145018`
$sql3a = "SELECT id, noun, name FROM pe:`24381_145018`;"
Write-Host "   Format 1: pe:`24381_145018`" -ForegroundColor Gray
try {
    $r = Invoke-SurrealQuery -Sql $sql3a
    if ($r.result -and $r.result[0] -and $r.result[0].Count -gt 0) {
        Write-Host "   Found: $($r.result[0][0] | ConvertTo-Json -Compress)" -ForegroundColor Green
    } else {
        Write-Host "   Not found" -ForegroundColor Yellow
    }
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
}

# Format 2: pe:24381_145018 (no backticks)
$sql3b = "SELECT id, noun, name FROM pe:24381_145018;"
Write-Host "   Format 2: pe:24381_145018" -ForegroundColor Gray
try {
    $r = Invoke-SurrealQuery -Sql $sql3b
    if ($r.result -and $r.result[0] -and $r.result[0].Count -gt 0) {
        Write-Host "   Found: $($r.result[0][0] | ConvertTo-Json -Compress)" -ForegroundColor Green
    } else {
        Write-Host "   Not found" -ForegroundColor Yellow
    }
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
}

# Format 3: WHERE condition
$sql3c = "SELECT id, noun, name FROM pe WHERE id = pe:`24381_145018`;"
Write-Host "   Format 3: WHERE id = pe:`24381_145018`" -ForegroundColor Gray
try {
    $r = Invoke-SurrealQuery -Sql $sql3c
    if ($r.result -and $r.result[0] -and $r.result[0].Count -gt 0) {
        Write-Host "   Found: $($r.result[0][0] | ConvertTo-Json -Compress)" -ForegroundColor Green
    } else {
        Write-Host "   Not found" -ForegroundColor Yellow
    }
} catch {
    Write-Host "   Error: $_" -ForegroundColor Red
}

Write-Host ""
Write-Host "========== 检查结束 ==========" -ForegroundColor Cyan
