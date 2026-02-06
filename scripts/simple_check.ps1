$headers = @{
    'Accept' = 'application/json'
    'surreal-ns' = '1516'
    'surreal-db' = 'AvevaMarineSample'
    'Authorization' = 'Basic cm9vdDpyb290'
}

Write-Host "Checking inst_relate records..."
$sql = "SELECT in, out FROM inst_relate LIMIT 5;"
$r = Invoke-RestMethod -Uri "http://localhost:8020/sql" -Method Post -Headers $headers -Body $sql -ContentType "text/plain"
Write-Host "Result:"
$r.result[0] | ConvertTo-Json -Depth 3

Write-Host "`nChecking tubi_relate records..."
$sql2 = "SELECT id, in, out FROM tubi_relate LIMIT 5;"
$r2 = Invoke-RestMethod -Uri "http://localhost:8020/sql" -Method Post -Headers $headers -Body $sql2 -ContentType "text/plain"
Write-Host "Result:"
$r2.result[0] | ConvertTo-Json -Depth 3
