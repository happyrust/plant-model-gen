$headers = @{
    'Accept' = 'application/json'
    'surreal-ns' = '1516'
    'surreal-db' = 'AvevaMarineSample'
    'Authorization' = 'Basic ' + [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes('root:root'))
}

function Run-Query($sql, $label) {
    $r = Invoke-RestMethod -Uri 'http://localhost:8020/sql' -Method Post -Headers $headers -Body $sql -ContentType 'text/plain'
    Write-Host "${label}: $($r | ConvertTo-Json -Compress -Depth 4)"
}

Write-Host "=== Verify 24381_145018 full tubi_relate chain ==="
Run-Query "SELECT id, in, out, bore_size, bad FROM tubi_relate WHERE system = pe:``24381_145018`` ORDER BY id;" "tubi_relate chain (all 11)"

Write-Host "`n=== Check all children of 24381_145018 for inst_relate ==="
$children = @("145019","145020","145021","145022","145023","145024","145025","145026","145027","145028","145029","145030","145031","145032","145033","145034","145035")
foreach ($c in $children) {
    $sql = "SELECT count() FROM inst_relate WHERE in = pe:``24381_$c`` GROUP ALL;"
    $r = Invoke-RestMethod -Uri 'http://localhost:8020/sql' -Method Post -Headers $headers -Body $sql -ContentType 'text/plain'
    $cnt = 0
    if ($r.result -and $r.result.Count -gt 0 -and $r.result[0].count) {
        $cnt = $r.result[0].count
    }
    if ($cnt -gt 0) {
        Write-Host "  24381_$c inst_relate=$cnt"
    }
}

Write-Host "`n=== Check children pe type info ==="
$sql = "SELECT id, noun, dbnum FROM pe WHERE id IN ["
$refnos = $children | ForEach-Object { "pe:``24381_$_``" }
$sql += ($refnos -join ",")
$sql += "];"
Run-Query $sql "children pe info"

Write-Host "`n=== Check geo_relate for BRAN children ==="
foreach ($c in @("145019","145025","145026","145031","145032","145033")) {
    $sql = "SELECT count() FROM geo_relate WHERE in = pe:``24381_$c`` GROUP ALL;"
    $r = Invoke-RestMethod -Uri 'http://localhost:8020/sql' -Method Post -Headers $headers -Body $sql -ContentType 'text/plain'
    $cnt = 0
    if ($r.result -and $r.result.Count -gt 0 -and $r.result[0].count) {
        $cnt = $r.result[0].count
    }
    Write-Host "  24381_$c geo_relate=$cnt"
}

Write-Host "`n=== Check inst_relate_aabb for 24381_145018 children ==="
$sql = "SELECT count() FROM inst_relate_aabb WHERE in = pe:``24381_145032`` GROUP ALL;"
Run-Query $sql "inst_relate_aabb for 145032 (OLET)"
