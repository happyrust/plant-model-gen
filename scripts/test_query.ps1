$headers = @{
    'Accept'         = 'application/json'
    'surreal-ns'     = '1516'
    'surreal-db'     = 'AvevaMarineSample'
    'Authorization'  = 'Basic cm9vdDpyb290'
}

# 查询单个 refno
$sql1 = 'SELECT VALUE count() FROM inst_relate WHERE in = pe:`24381_145018`;'
$r1 = Invoke-RestMethod -Uri 'http://localhost:8020/sql' -Method Post -Headers $headers -Body $sql1 -ContentType 'text/plain'
Write-Host "count for pe:`24381_145018`: $($r1.result[0])"

# 查询样本记录
$sql2 = 'SELECT in, out FROM inst_relate WHERE in = pe:`24381_145018` LIMIT 3;'
$r2 = Invoke-RestMethod -Uri 'http://localhost:8020/sql' -Method Post -Headers $headers -Body $sql2 -ContentType 'text/plain'
Write-Host "Sample records:"
$r2.result[0] | ConvertTo-Json -Depth 3
