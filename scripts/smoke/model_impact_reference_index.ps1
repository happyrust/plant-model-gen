# ADR-0011 P1 step-1：目录引用反向索引 cata_ref_index 的 SQL 形态 smoke。
# 执行 db-data/model_impact_reference_index.surql，验证 src/versioned_db/cata_ref_index.rs
# 发出的每种语句在引擎上成立（schema / INSERT[{id:...}] / DELETE WHERE source /
# 反查 target_refno IN / 出边 source_refno IN / count GROUP ALL / state UPSERT+SELECT），
# 并核对反查语义（改目录定义 SCOM 200_1 → 反查到引用它的 100_1、100_2）。
#
# 用法：powershell -File scripts/smoke/model_impact_reference_index.ps1 [-BaseUrl http://127.0.0.1:8030]
# 依赖：任一 fork/标准 surreal 实例（本 smoke 不用 VERSION / 递归 idiom，标准 3.x 亦可）。
# 数据写入独立 ns/db（smoke_crx/ref_index），不触碰业务与夹具数据。

param(
    [string]$BaseUrl = $(if ($env:AIOS_SURREAL_URL) { $env:AIOS_SURREAL_URL } else { "http://127.0.0.1:8030" }),
    [string]$User = $(if ($env:AIOS_SURREAL_USER) { $env:AIOS_SURREAL_USER } else { "root" }),
    [string]$Pass = $(if ($env:AIOS_SURREAL_PASS) { $env:AIOS_SURREAL_PASS } else { "root" }),
    [string]$Ns = "smoke_crx",
    [string]$Db = "ref_index",
    [string]$SurqlFile = "$PSScriptRoot\..\..\db-data\model_impact_reference_index.surql",
    [string]$ResultJson = "$PSScriptRoot\..\..\db-data\model_impact_reference_index.result.json"
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

$labels = @(
    "setup: REMOVE cata_ref_index",           # 0
    "setup: REMOVE cata_ref_index_state",     # 1
    "schema: DEFINE TABLE cata_ref_index",    # 2
    "schema: DEFINE INDEX idx_crx_target",    # 3
    "schema: DEFINE INDEX idx_crx_source",    # 4
    "schema: DEFINE TABLE state",             # 5
    "INSERT 4 edges (id array + explicit id)",# 6
    "count source_dbnum=100 (expect 4)",      # 7
    "inbound 200_1 (expect 100_1,100_2)",     # 8
    "inbound family CATR/SPRE (expect 100_1,100_2)", # 9
    "outbound 100_1 (expect 200_1,200_2)",    # 10
    "DELETE by source 100_1",                 # 11
    "count after delete (expect 2)",          # 12
    "re-INSERT 100_1 -> 200_9",               # 13
    "count after reinsert (expect 3)",        # 14
    "inbound 200_1 after move (expect 100_2)",# 15
    "inbound 200_9 (expect 100_1)",           # 16
    "state UPSERT",                           # 17
    "state read (expect ready=true,rows=3)"   # 18
)

for ($i = 0; $i -lt $resp.Count; $i++) {
    $label = if ($i -lt $labels.Count) { $labels[$i] } else { "stmt#$i" }
    $status = $resp[$i].status
    $result = ($resp[$i].result | ConvertTo-Json -Depth 6 -Compress)
    if ($null -eq $result) { $result = "null" }
    if ($result.Length -gt 200) { $result = $result.Substring(0, 200) + "..." }
    $color = if ($status -eq "OK") { "Green" } else { "Yellow" }
    Write-Host ("[{0,-3}] {1,-4} {2}" -f $i, $status, $label) -ForegroundColor $color
    Write-Host ("      -> {0}" -f $result)
}

function Get-Stmt([int]$idx) { if ($idx -lt $resp.Count) { $resp[$idx] } else { $null } }
function Is-OK([int]$idx) { $s = Get-Stmt $idx; $null -ne $s -and $s.status -eq "OK" }
function ResultSet([int]$idx) {
    $s = Get-Stmt $idx
    if ($null -eq $s -or $null -eq $s.result) { return @() }
    return @($s.result | ForEach-Object { "$_" })
}
function CountVal([int]$idx) {
    # `SELECT count() ... GROUP ALL` 返回 [{count:N}]
    $s = Get-Stmt $idx
    if ($null -ne $s -and $null -ne $s.result -and $s.result.Count -ge 1 -and $null -ne $s.result[0].count) {
        return [int]$s.result[0].count
    }
    return -1
}
function SameSet([string[]]$a, [string[]]$b) {
    if ($a.Count -ne $b.Count) { return $false }
    $sa = $a | Sort-Object; $sb = $b | Sort-Object
    for ($i = 0; $i -lt $sa.Count; $i++) { if ($sa[$i] -ne $sb[$i]) { return $false } }
    return $true
}

Write-Host ""
Write-Host "===== 判定（语法 OK + 反查语义核对）====="

# 全部语句语法 OK
$allOk = $true
for ($i = 0; $i -lt $resp.Count; $i++) { if (-not (Is-OK $i)) { $allOk = $false } }

$stateReady = $false
$stateRows = -1
$s18 = Get-Stmt 18
if ($null -ne $s18 -and $null -ne $s18.result -and $s18.result.Count -ge 1) {
    $stateReady = [bool]$s18.result[0].ready
    $stateRows = [int]$s18.result[0].row_count
}

$verdicts = [ordered]@{
    "所有语句语法 OK"                = $allOk
    "行数=4"                        = ((CountVal 7) -eq 4)
    "反查 200_1 = {100_1,100_2}"    = (SameSet (ResultSet 8) @("100_1", "100_2"))
    "属性族过滤 = {100_1,100_2}"    = (SameSet (ResultSet 9) @("100_1", "100_2"))
    "出边 100_1 有序 [200_1,200_2]" = ("$((ResultSet 10) -join ',')" -eq "200_1,200_2")
    "删后行数=2"                    = ((CountVal 12) -eq 2)
    "重插后行数=3"                  = ((CountVal 14) -eq 3)
    "移后反查 200_1 = {100_2}"      = (SameSet (ResultSet 15) @("100_2"))
    "反查 200_9 = {100_1}"          = (SameSet (ResultSet 16) @("100_1"))
    "state ready=true & rows=3"     = ($stateReady -and ($stateRows -eq 3))
}

$failed = 0
foreach ($k in $verdicts.Keys) {
    $v = $verdicts[$k]
    if (-not $v) { $failed++ }
    $mark = if ($v) { "PASS" } else { "FAIL" }
    $color = if ($v) { "Green" } else { "Red" }
    Write-Host ("  {0,-32} {1}" -f $k, $mark) -ForegroundColor $color
}

Write-Host ""
if ($failed -gt 0) {
    Write-Host "存在 $failed 项失败：cata_ref_index 的 SQL 形态/反查语义在该实例上不成立。" -ForegroundColor Red
    exit 1
}
Write-Host "全部通过：cata_ref_index schema/写入/反查/replace-by-source/state 的 SQL 形态可用。" -ForegroundColor Green
exit 0
