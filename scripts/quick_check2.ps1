# Try both header formats for SurrealDB (v1 vs v2)
$headers = @{
    'Accept' = 'application/json'
    'surreal-ns' = '1516'
    'surreal-db' = 'AvevaMarineSample'
    'Authorization' = 'Basic ' + [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes('root:root'))
}

Write-Host "=== SurrealDB Record Counts (surreal-ns/surreal-db headers) ==="

$sql1 = "SELECT VALUE count() FROM inst_relate;"
$r1 = Invoke-RestMethod -Uri 'http://localhost:8020/sql' -Method Post -Headers $headers -Body $sql1 -ContentType 'text/plain'
Write-Host "inst_relate count: $($r1 | ConvertTo-Json -Compress)"

$sql2 = "SELECT VALUE count() FROM tubi_relate;"
$r2 = Invoke-RestMethod -Uri 'http://localhost:8020/sql' -Method Post -Headers $headers -Body $sql2 -ContentType 'text/plain'
Write-Host "tubi_relate count: $($r2 | ConvertTo-Json -Compress)"

$sql3 = "SELECT VALUE count() FROM geo_relate;"
$r3 = Invoke-RestMethod -Uri 'http://localhost:8020/sql' -Method Post -Headers $headers -Body $sql3 -ContentType 'text/plain'
Write-Host "geo_relate count: $($r3 | ConvertTo-Json -Compress)"

$sql4 = "SELECT VALUE count() FROM pe;"
$r4 = Invoke-RestMethod -Uri 'http://localhost:8020/sql' -Method Post -Headers $headers -Body $sql4 -ContentType 'text/plain'
Write-Host "pe count: $($r4 | ConvertTo-Json -Compress)"

$sql5 = "SELECT VALUE count() FROM inst_info;"
$r5 = Invoke-RestMethod -Uri 'http://localhost:8020/sql' -Method Post -Headers $headers -Body $sql5 -ContentType 'text/plain'
Write-Host "inst_info count: $($r5 | ConvertTo-Json -Compress)"

# Check sample data
$sql6 = "SELECT * FROM inst_relate LIMIT 3;"
$r6 = Invoke-RestMethod -Uri 'http://localhost:8020/sql' -Method Post -Headers $headers -Body $sql6 -ContentType 'text/plain'
Write-Host "`n=== Sample inst_relate ==="
Write-Host ($r6 | ConvertTo-Json -Depth 5 -Compress)

$sql7 = "SELECT * FROM tubi_relate LIMIT 3;"
$r7 = Invoke-RestMethod -Uri 'http://localhost:8020/sql' -Method Post -Headers $headers -Body $sql7 -ContentType 'text/plain'
Write-Host "`n=== Sample tubi_relate ==="
Write-Host ($r7 | ConvertTo-Json -Depth 5 -Compress)
