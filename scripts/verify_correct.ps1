$headers = @{
    'Accept' = 'application/json'
    'surreal-ns' = '1516'
    'surreal-db' = 'AvevaMarineSample'
    'Authorization' = 'Basic ' + [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes('root:root'))
}

function Run-Query($sql, $label) {
    $r = Invoke-RestMethod -Uri 'http://localhost:8020/sql' -Method Post -Headers $headers -Body $sql -ContentType 'text/plain'
    Write-Host "${label}: $($r | ConvertTo-Json -Compress -Depth 3)"
}

$ref0 = 24381
$dbMetaPath = "output\\AvevaMarineSample\\scene_tree\\db_meta_info.json"
$dbnum = $null
if (Test-Path $dbMetaPath) {
    try {
        $m = (Get-Content $dbMetaPath -Raw | ConvertFrom-Json)
        $mapped = $m.ref0_to_dbnum."$ref0"
        if ($mapped) {
            $dbnum = [int]$mapped
        } else {
            Write-Error "未在 db_meta_info.json 中找到 ref0=$ref0 的 dbnum 映射；请先生成/更新 db_meta_info.json，或手动指定正确 dbnum。"
            exit 1
        }
    } catch {
        Write-Error "解析 db_meta_info.json 失败；请先修复该文件或重新生成。"
        exit 1
    }
} else {
    Write-Error "未找到 $dbMetaPath；请先生成/同步 scene_tree 元数据（db_meta_info.json）。"
    exit 1
}
Write-Host "✅ ref0=$ref0 -> dbnum=$dbnum"

Write-Host "=== 1. Total Counts (GROUP ALL) ==="
Run-Query "SELECT count() FROM inst_relate GROUP ALL;" "inst_relate total"
Run-Query "SELECT count() FROM tubi_relate GROUP ALL;" "tubi_relate total"
Run-Query "SELECT count() FROM geo_relate GROUP ALL;" "geo_relate total"
Run-Query "SELECT count() FROM inst_info GROUP ALL;" "inst_info total"

Write-Host "`n=== 2. Check 24381_145018 specifically ==="

# Check if pe record exists
Run-Query "SELECT id, noun, dbnum FROM pe:``24381_145018``;" "pe:24381_145018"

# Check inst_relate for 24381_145018
Run-Query "SELECT count() FROM inst_relate WHERE in = pe:``24381_145018`` GROUP ALL;" "inst_relate in=24381_145018"

# Check tubi_relate for 24381_145018 (BRAN -> children)
Run-Query "SELECT id, in, out FROM tubi_relate WHERE in = pe:``24381_145018`` LIMIT 5;" "tubi_relate in=24381_145018"

# tubi_relate system field
Run-Query "SELECT id, in, out, system FROM tubi_relate WHERE system = pe:``24381_145018`` LIMIT 5;" "tubi_relate system=24381_145018"

Write-Host "`n=== 3. Check children of 24381_145018 ==="
# Check inst_relate for children (e.g., 24381_145019 to 24381_145035)
Run-Query "SELECT count() FROM inst_relate WHERE in = pe:``24381_145019`` GROUP ALL;" "inst_relate in=145019"
Run-Query "SELECT count() FROM inst_relate WHERE in = pe:``24381_145025`` GROUP ALL;" "inst_relate in=145025"
Run-Query "SELECT count() FROM inst_relate WHERE in = pe:``24381_145032`` GROUP ALL;" "inst_relate in=145032"

Write-Host "`n=== 4. Sample inst_relate for 24381_* ==="
Run-Query "SELECT id, in, out FROM inst_relate WHERE in.dbnum = $dbnum LIMIT 5;" "inst_relate dbnum=$dbnum sample (ref0=$ref0)"

Write-Host "`n=== 5. Sample tubi_relate ==="
Run-Query "SELECT id, in, out, system FROM tubi_relate LIMIT 5;" "tubi_relate sample"

Write-Host "`n=== 6. tubi_relate count by system ==="
Run-Query "SELECT system, count() FROM tubi_relate GROUP BY system;" "tubi_relate by system"
