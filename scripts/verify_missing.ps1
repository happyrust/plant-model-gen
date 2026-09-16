$headers = @{
    'Accept' = 'application/json'
    'surreal-ns' = '1516'
    'surreal-db' = 'AvevaMarineSample'
    'Authorization' = 'Basic ' + [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes('root:root'))
}

function Run-Query($sql) {
    $r = Invoke-RestMethod -Uri 'http://localhost:8020/sql' -Method Post -Headers $headers -Body $sql -ContentType 'text/plain'
    return ($r | ConvertTo-Json -Compress -Depth 4)
}

Write-Host "=== Missing inst_relate children: 145019, 145021, 145023, 145025, 145033 ==="

# Check pe noun for each
$missing = @("145019","145021","145023","145025","145033")
foreach ($c in $missing) {
    $sql = "SELECT id, noun FROM pe:``24381_$c``;"
    Write-Host "  pe:24381_$c = $(Run-Query $sql)"
}

Write-Host "`n=== Check geo_relate for missing children ==="
foreach ($c in $missing) {
    $sql = "SELECT count() FROM geo_relate WHERE in = pe:``24381_$c`` GROUP ALL;"
    Write-Host "  24381_$c geo_relate = $(Run-Query $sql)"
}

Write-Host "`n=== Has inst_relate children: check a few ==="
$have = @("145020","145022","145024","145026")
foreach ($c in $have) {
    $sql = "SELECT id, noun FROM pe:``24381_$c``;"
    Write-Host "  pe:24381_$c = $(Run-Query $sql)"
}
