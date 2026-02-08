$headers = @{
    'Accept' = 'application/json'
    'surreal-ns' = '1516'
    'surreal-db' = 'AvevaMarineSample'
    'Authorization' = 'Basic cm9vdDpyb290'
}
$body = $args[0]
$result = Invoke-RestMethod -Uri 'http://localhost:8020/sql' -Method Post -Headers $headers -Body $body -ContentType 'text/plain'
$result | ConvertTo-Json -Depth 10
