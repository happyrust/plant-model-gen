$headers = @{
    'Accept' = 'application/json'
    'NS' = '1516'
    'DB' = 'AvevaMarineSample'
    'Authorization' = 'Basic ' + [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes('root:root'))
}

Write-Host "=== SurrealDB Record Counts ==="

# Query 1: inst_relate count
$sql1 = "SELECT VALUE count() FROM inst_relate;"
$r1 = Invoke-RestMethod -Uri 'http://localhost:8020/sql' -Method Post -Headers $headers -Body $sql1 -ContentType 'text/plain'
Write-Host "inst_relate count: $($r1 | ConvertTo-Json -Compress)"

# Query 2: tubi_relate count
$sql2 = "SELECT VALUE count() FROM tubi_relate;"
$r2 = Invoke-RestMethod -Uri 'http://localhost:8020/sql' -Method Post -Headers $headers -Body $sql2 -ContentType 'text/plain'
Write-Host "tubi_relate count: $($r2 | ConvertTo-Json -Compress)"

# Query 3: geo_relate count
$sql3 = "SELECT VALUE count() FROM geo_relate;"
$r3 = Invoke-RestMethod -Uri 'http://localhost:8020/sql' -Method Post -Headers $headers -Body $sql3 -ContentType 'text/plain'
Write-Host "geo_relate count: $($r3 | ConvertTo-Json -Compress)"

# Query 4: inst_info count
$sql4 = "SELECT VALUE count() FROM inst_info;"
$r4 = Invoke-RestMethod -Uri 'http://localhost:8020/sql' -Method Post -Headers $headers -Body $sql4 -ContentType 'text/plain'
Write-Host "inst_info count: $($r4 | ConvertTo-Json -Compress)"

# Query 5: pe count
$sql5 = "SELECT VALUE count() FROM pe;"
$r5 = Invoke-RestMethod -Uri 'http://localhost:8020/sql' -Method Post -Headers $headers -Body $sql5 -ContentType 'text/plain'
Write-Host "pe count: $($r5 | ConvertTo-Json -Compress)"

# Query 6: check surreal version/info
$sql6 = "INFO FOR DB;"
$r6 = Invoke-RestMethod -Uri 'http://localhost:8020/sql' -Method Post -Headers $headers -Body $sql6 -ContentType 'text/plain'
Write-Host "`n=== DB Info ==="
Write-Host ($r6 | ConvertTo-Json -Depth 3 -Compress)

# Query 7: sample inst_relate records
$sql7 = "SELECT * FROM inst_relate LIMIT 3;"
$r7 = Invoke-RestMethod -Uri 'http://localhost:8020/sql' -Method Post -Headers $headers -Body $sql7 -ContentType 'text/plain'
Write-Host "`n=== Sample inst_relate ==="
Write-Host ($r7 | ConvertTo-Json -Depth 4 -Compress)

# Query 8: sample tubi_relate records  
$sql8 = "SELECT * FROM tubi_relate LIMIT 3;"
$r8 = Invoke-RestMethod -Uri 'http://localhost:8020/sql' -Method Post -Headers $headers -Body $sql8 -ContentType 'text/plain'
Write-Host "`n=== Sample tubi_relate ==="
Write-Host ($r8 | ConvertTo-Json -Depth 4 -Compress)
