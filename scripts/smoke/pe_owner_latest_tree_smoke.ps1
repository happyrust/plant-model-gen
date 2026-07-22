# specs/023 M0/T1：latest pe_owner 图查询原语 smoke（单源；TreeIndex 双源 diff 已退役）
# 执行 db-data/smoke_023_pe_owner_latest.surql，验证 PeOwnerTreeStore 依赖的全部 SQL 形态：
#   L1  children 同胞有序（<owner><-pe_owner ORDER BY id）
#   L2  递归全子孙（.{..256+collect}<-pe_owner<-pe，SurrealDB 3.1 递归 idiom）
#   L3  深度受限递归（.{..2+collect}）
#   L4  祖先链 owner 链接递归（.{..256+collect}(.owner)）
#   L5  children 计数 + pe.children 字段回退
#   L6  批量 kids（边优先/字段回退）
#   L7  节点元信息批查（cata_hash 字符串字段）
#   L8-L11 noun 表级扫 + D3 idx_pe_dbnum_noun 索引
#
# 用法：powershell -File scripts/smoke/pe_owner_latest_tree_smoke.ps1 [-BaseUrl http://127.0.0.1:8030]
# 依赖：任一 fork surreal 实例（versioned 与否均可，本 smoke 不用 VERSION）。
# 数据写入独立 ns/db（smoke023/latest_tree），不触碰业务与夹具数据。

param(
    [string]$BaseUrl = $(if ($env:AIOS_SURREAL_URL) { $env:AIOS_SURREAL_URL } else { "http://127.0.0.1:8030" }),
    [string]$User = $(if ($env:AIOS_SURREAL_USER) { $env:AIOS_SURREAL_USER } else { "root" }),
    [string]$Pass = $(if ($env:AIOS_SURREAL_PASS) { $env:AIOS_SURREAL_PASS } else { "root" }),
    [string]$Ns = "smoke023",
    [string]$Db = "latest_tree",
    [string]$SurqlFile = "$PSScriptRoot\..\..\db-data\smoke_023_pe_owner_latest.surql",
    [string]$ResultJson = "$PSScriptRoot\..\..\db-data\smoke_023_pe_owner_latest.result.json"
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

# 语句顺序：0-2 清场，3-9 CREATE，10-12 INSERT RELATION，13 起为探测
$labels = @(
    "setup: REMOVE pe",
    "setup: REMOVE pe_owner",
    "setup: REMOVE ele_reuse_relate",
    "setup: CREATE pe:w",
    "setup: CREATE pe:s1",
    "setup: CREATE pe:s2",
    "setup: CREATE pe:z1",
    "setup: CREATE pe:e1",
    "setup: CREATE pe:e2",
    "setup: CREATE pe:f",
    "setup: INSERT RELATION w",
    "setup: INSERT RELATION s1",
    "setup: INSERT RELATION z1",
    "L1 : children ordered (expect [e1,e2])",
    "L2 : recurse all descendants (expect [s1,s2,z1,e1,e2])",
    "L3 : recurse depth<=2 (expect [s1,s2,z1])",
    "L4 : ancestors via (.owner) (expect [z1,s1,w])",
    "L5 : children counts + field fallback",
    "L6 : batch kids edge/field",
    "L7 : node metas with cata_hash",
    "L8 : noun scan before index (expect [e1,e2])",
    "L9 : DEFINE INDEX idx_pe_dbnum_noun",
    "L10: noun scan after index (expect same as L8)",
    "L11: noun group count"
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
function SameSet([string[]]$a, [string[]]$b) {
    if ($a.Count -ne $b.Count) { return $false }
    $sa = $a | Sort-Object; $sb = $b | Sort-Object
    for ($i = 0; $i -lt $sa.Count; $i++) { if ($sa[$i] -ne $sb[$i]) { return $false } }
    return $true
}

Write-Host ""
Write-Host "===== 判定（语法 OK + 关键值核对）====="
$l1 = ResultSet 13
$l2 = ResultSet 14
$l3 = ResultSet 15
$l4 = ResultSet 16
$verdicts = [ordered]@{
    "L1 children 有序"          = (Is-OK 13) -and ("$($l1 -join ',')" -eq "pe:e1,pe:e2")
    "L2 递归全子孙"             = (Is-OK 14) -and (SameSet $l2 @("pe:s1","pe:s2","pe:z1","pe:e1","pe:e2"))
    "L3 深度受限递归"           = (Is-OK 15) -and (SameSet $l3 @("pe:s1","pe:s2","pe:z1"))
    "L4 祖先链 owner 递归"      = (Is-OK 16) -and ("$($l4 -join ',')" -eq "pe:z1,pe:s1,pe:w")
    "L5 计数+字段回退"          = (Is-OK 17)
    "L6 批量 kids"              = (Is-OK 18)
    "L7 元信息批查"             = (Is-OK 19)
    "L8-L10 noun 扫描+索引"     = (Is-OK 20) -and (Is-OK 21) -and (Is-OK 22) -and (SameSet (ResultSet 20) (ResultSet 22))
    "L11 noun 分组计数"         = (Is-OK 23)
}
$failed = 0
foreach ($k in $verdicts.Keys) {
    $v = $verdicts[$k]
    if (-not $v) { $failed++ }
    $mark = if ($v) { "PASS" } else { "FAIL" }
    $color = if ($v) { "Green" } else { "Red" }
    Write-Host ("  {0,-30} {1}" -f $k, $mark) -ForegroundColor $color
}

Write-Host ""
if ($failed -gt 0) {
    Write-Host "存在 $failed 项失败：PeOwnerTreeStore 依赖的 SQL 形态在该实例上不成立，阻断 M1 切换。" -ForegroundColor Red
    exit 1
}
Write-Host "全部通过：latest pe_owner 图查询原语可用（L4 祖先序 / L2 BFS 序已含值核对）。" -ForegroundColor Green
exit 0
