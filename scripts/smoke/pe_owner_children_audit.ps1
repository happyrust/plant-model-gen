# specs/023 M0/T3: pe_owner 边 vs pe.children 完整性审计
# 执行 db-data/audit_pe_owner_vs_children.surql，结果落 db-data/audit_pe_owner_vs_children.out.md；
# 抽样发现 mismatch/stale 时退出码 1（该库切 pe_owner latest 树前必须先 rebuild-pe-owner）。
#
# 用法（业务库审计时把 -Ns/-Db 换成站点实际 surreal_ns / 工程名）：
#   powershell -File scripts/smoke/pe_owner_children_audit.ps1 -Ns <surreal_ns> -Db <project> [-Dbnum <n>]
# 连接参数默认 127.0.0.1:8030 root/root（与 verify_022 系列一致），
# 可用参数或环境变量 AIOS_SURREAL_URL / AIOS_SURREAL_USER / AIOS_SURREAL_PASS 覆盖。

param(
    [string]$BaseUrl = $(if ($env:AIOS_SURREAL_URL) { $env:AIOS_SURREAL_URL } else { "http://127.0.0.1:8030" }),
    [string]$User = $(if ($env:AIOS_SURREAL_USER) { $env:AIOS_SURREAL_USER } else { "root" }),
    [string]$Pass = $(if ($env:AIOS_SURREAL_PASS) { $env:AIOS_SURREAL_PASS } else { "root" }),
    [string]$Ns = "smoke023",
    [string]$Db = "latest_tree",
    [int]$Dbnum = -1,
    [string]$SurqlFile = "$PSScriptRoot\..\..\db-data\audit_pe_owner_vs_children.surql",
    [string]$OutMd = "$PSScriptRoot\..\..\db-data\audit_pe_owner_vs_children.out.md"
)

$ErrorActionPreference = "Stop"

$surql = Get-Content -Raw -Path $SurqlFile
if ($Dbnum -ge 0) {
    $surql = $surql -replace 'LET \$target_dbnum = NONE;', "LET `$target_dbnum = $Dbnum;"
}
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

Write-Host "POST $BaseUrl/sql (ns=$Ns db=$Db dbnum=$(if ($Dbnum -ge 0) { $Dbnum } else { 'ALL' })) ..."
$resp = Invoke-RestMethod -Method Post -Uri "$BaseUrl/sql" -Headers $headers -ContentType "text/plain" -Body $surql

# 语句顺序（与 surql 文件一一对应）：
#  0 banner0  1 LET target  2 target 回显
#  3 banner1  4 totals per dbnum  5 edge rows total
#  6 banner2  7 LET sample  8 sample verdict  9 mismatch list
# 10 banner3 11 LET stale  12 stale verdict  13 stale list
# 14 end banner
$sampleStmt = $resp[8]
$mismatchList = $resp[9]
$staleStmt = $resp[12]
$staleList = $resp[13]
$totalsJson = ($resp[4].result | ConvertTo-Json -Depth 6); if ($null -eq $totalsJson) { $totalsJson = "[]" }
$edgeRowsJson = ($resp[5].result | ConvertTo-Json -Depth 6); if ($null -eq $edgeRowsJson) { $edgeRowsJson = "[]" }
$sampleJson = ($sampleStmt.result | ConvertTo-Json -Depth 6); if ($null -eq $sampleJson) { $sampleJson = "{}" }
$mismatchJson = ($mismatchList.result | ConvertTo-Json -Depth 6); if ($null -eq $mismatchJson) { $mismatchJson = "[]" }
$staleJson = ($staleList.result | ConvertTo-Json -Depth 6); if ($null -eq $staleJson) { $staleJson = "[]" }

$queriesOk = ($sampleStmt.status -eq "OK") -and ($staleStmt.status -eq "OK")
$mismatched = if ($sampleStmt.result.mismatched) { [int]$sampleStmt.result.mismatched } else { 0 }
$sampled = if ($sampleStmt.result.sampled) { [int]$sampleStmt.result.sampled } else { 0 }
$stale = if ($staleStmt.result.stale) { [int]$staleStmt.result.stale } else { 0 }

$verdict = if (-not $queriesOk) {
    "ERROR：审计查询未全部返回 OK（sample=$($sampleStmt.status) stale=$($staleStmt.status)），结果不可用"
} elseif ($mismatched -gt 0 -or $stale -gt 0) {
    "FAIL：抽样 $sampled 个 parent 中 $mismatched 个边计数与 children 不一致；childless 抽样残留脏边 $stale 条。切 pe_owner latest 树前先对该 dbnum 执行 model-version rebuild-pe-owner。"
} else {
    "PASS：抽样 $sampled 个 parent 边计数与 children 全部一致，未发现残留脏边（抽样口径，全量修复口径见 rebuild-pe-owner）。"
}

$now = Get-Date -Format "yyyy-MM-dd HH:mm:ss zzz"
$md = @"
# audit_pe_owner_vs_children — pe_owner 边 vs pe.children 完整性审计结果

- 生成时间：$now（由 scripts/smoke/pe_owner_children_audit.ps1 生成，重跑会覆盖本文件）
- 实例：``$BaseUrl``（HTTP POST /sql，Basic $User/***）
- 环境：``ns=$Ns`` ``db=$Db`` ``dbnum=$(if ($Dbnum -ge 0) { $Dbnum } else { 'ALL' })``
- 审计 SQL：``db-data/audit_pe_owner_vs_children.surql``（判定规则与用法见该文件头注释）

## 判定

**$verdict**

## [1] 总量（per dbnum parents/children + 边行数）

``````json
$totalsJson
``````

``````json
$edgeRowsJson
``````

## [2] 抽样 parent 对比（count(<-pe_owner) vs len(children)）

``````json
$sampleJson
``````

不一致清单（最多 50 条）：

``````json
$mismatchJson
``````

## [3] childless 抽样残留脏边（最多 50 条）

``````json
$staleJson
``````

## 修复口径

边不完整/陈旧/脏边：对该 dbnum 执行 ``model-version rebuild-pe-owner --dbnum <n>``（先删后插全量重建，
幂等可重跑）或全量重灌；修完重跑本审计确认 PASS 后才允许该库走 pe_owner latest 树查询主路径。
"@

Set-Content -Path $OutMd -Value $md -Encoding UTF8
Write-Host ""
Write-Host "审计结果已写入 $OutMd"
Write-Host $verdict -ForegroundColor $(if ($mismatched -gt 0 -or $stale -gt 0 -or -not $queriesOk) { "Red" } else { "Green" })

if (-not $queriesOk) { exit 2 }
if ($mismatched -gt 0 -or $stale -gt 0) { exit 1 }
exit 0
