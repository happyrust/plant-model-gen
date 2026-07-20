# specs/023 T009（契约级）：增量语句形态 smoke
# 精确镜像 sesno_increment.rs T004 生成的语句形态（graph-delete + 先删后插 + UPSERT 顺序），
# 在 versioned 实例上验证当前态与 VERSION 历史语义。
# 真机端到端（真实 PDMS 源文件 + incremental-sesno --json）见 quickstart Scenario 1 / M4。
#
# 用法：powershell -File scripts/smoke/pe_owner_incr_shapes_smoke.ps1 [-BaseUrl http://127.0.0.1:8030]

param(
    [string]$BaseUrl = "http://127.0.0.1:8030",
    [string]$User = "root",
    [string]$Pass = "root",
    [string]$Ns = "smoke023",
    [string]$Db = "incr_shapes",
    [string]$SurqlFile = "$PSScriptRoot\..\..\db-data\smoke_023_incr_shapes.surql",
    [string]$ResultJson = "$PSScriptRoot\..\..\db-data\smoke_023_incr_shapes_result.json"
)

$ErrorActionPreference = "Stop"

$surql = Get-Content -Raw -Path $SurqlFile
$auth = [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes("${User}:${Pass}"))
$headers = @{
    Authorization = "Basic $auth"
    "Surreal-NS"  = $Ns
    "Surreal-DB"  = $Db
    Accept        = "application/json"
}

Write-Host "bootstrap ns/db ..."
$bootstrap = "DEFINE NAMESPACE IF NOT EXISTS $Ns; USE NS $Ns; DEFINE DATABASE IF NOT EXISTS $Db;"
Invoke-RestMethod -Method Post -Uri "$BaseUrl/sql" -Headers @{ Authorization = "Basic $auth"; Accept = "application/json" } -ContentType "text/plain" -Body $bootstrap | Out-Null

Write-Host "POST $BaseUrl/sql (ns=$Ns db=$Db) ..."
$resp = Invoke-RestMethod -Method Post -Uri "$BaseUrl/sql" -Headers $headers -ContentType "text/plain" -Body $surql
$resp | ConvertTo-Json -Depth 10 | Set-Content -Path $ResultJson -Encoding UTF8
Write-Host "原始结果已写入 $ResultJson"
Write-Host ""

function Compact($v) {
    $s = ($v | ConvertTo-Json -Depth 5 -Compress)
    if ($null -eq $s) { $s = "null" }
    if ($s.Length -gt 120) { $s = $s.Substring(0, 120) + "..." }
    return $s
}

# 语句索引（与 surql 文件顺序一致）：
# 0-1 清场; 2-4 UPSERT p/a/b; 5 DELETE p<-pe_owner; 6 INSERT [a,b];
# 7 SLEEP; 8 LET t1; 9 SLEEP;
# 10 DELETE pe:b; 11 DELETE b->pe_owner; 12 DELETE b<-pe_owner;
# 13 UPSERT p; 14 DELETE p<-pe_owner; 15 INSERT [a,c]; 16 UPSERT c;
# 17 SLEEP; 18 LET t2; 19 SLEEP;
# 20 V1a p.name; 21 V1b a.name; 22 V2a 当前 children(ORDER BY id); 23 V2b 当前 children(子查询排序);
# 24 V3 t1 children; 25 V4 t2 children; 26 V5a b@t1; 27 V5b b 当前;
# 28 V6a children@t1; 29 V6b children@t2; 30 V7 ok
$checks = @(
    @{ idx = 20; label = "V1a graph-delete 不伤源记录 p"; expect = '"parent"' },
    @{ idx = 21; label = "V1b graph-delete 不伤子记录 a"; expect = '"childA"' },
    @{ idx = 22; label = "V2a 当前 children(ORDER BY id)"; expect = '["pe:a","pe:c"]' },
    @{ idx = 23; label = "V2b 当前 children(子查询排序)";  expect = '["pe:a","pe:c"]' },
    @{ idx = 24; label = "V3  t1 children=[a,b]";          expect = '["pe:a","pe:b"]' },
    @{ idx = 25; label = "V4  t2 children=[a,c]";          expect = '["pe:a","pe:c"]' },
    @{ idx = 26; label = "V5a 已删 b 的 t1 快照";           expect = '"childB"' },
    @{ idx = 28; label = "V6a 保底 children 字段@t1";       expect = '["pe:a","pe:b"]' },
    @{ idx = 29; label = "V6b 保底 children 字段@t2";       expect = '["pe:a","pe:c"]' }
)

$failed = 0
foreach ($c in $checks) {
    $stmt = $resp[$c.idx]
    $actual = Compact $stmt.result
    # 包含匹配：SELECT VALUE 返回嵌套数组时 PowerShell 会包一层 {"value":[...],"Count":N}
    $ok = ($stmt.status -eq "OK") -and ($actual.Contains($c.expect))
    $mark = if ($ok) { "PASS" } else { "FAIL"; }
    $color = if ($ok) { "Green" } else { "Red" }
    Write-Host ("[{0,-3}] {1,-4} {2,-34} expect={3} actual={4}" -f $c.idx, $mark, $c.label, $c.expect, $actual) -ForegroundColor $color
    if (-not $ok) { $failed++ }
}

# V5b：已删除 b 当前应为空（null / [] 均接受）
$v5b = $resp[27]
$v5bActual = Compact $v5b.result
$v5bOk = ($v5b.status -eq "OK") -and ($v5bActual -eq "null" -or $v5bActual -eq "[]")
Write-Host ("[27 ] {0,-4} V5b 已删 b 当前为空                  actual={1}" -f $(if ($v5bOk) { "PASS" } else { "FAIL" }), $v5bActual) -ForegroundColor $(if ($v5bOk) { "Green" } else { "Red" })
if (-not $v5bOk) { $failed++ }

Write-Host ""
if ($failed -gt 0) {
    Write-Host "共 $failed 项失败" -ForegroundColor Red
    exit 1
}
Write-Host "全部通过：T004 语句形态在 versioned 实例上的当前态/历史语义正确" -ForegroundColor Green
exit 0
