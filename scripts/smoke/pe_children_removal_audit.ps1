# Phase 0 go/no-go 审计：去掉 pe.children、统一走 pe_owner 前的完整性闸门
# 执行 db-data/audit_pe_children_removal.surql，结果落 db-data/audit_pe_children_removal.out.md。
#
# 与 specs/023 的 pe_owner_children_audit.ps1 区别：
#   * 全量（默认 -Limit 0）逐 parent 比对 count(<-pe_owner) vs len(children)，不是抽样；
#   * 汇总各 dbnum 的 maintained_since_sesno，并列出没有可信边界的 dbnum。
#
# 退出码：
#   0  PASS（latest 边完整，可进入 Phase A/B；without_meta 仅告警）
#   1  FAIL（mismatched/stale > 0：删 children 前先 model-version rebuild-pe-owner）
#   2  ERROR（审计查询未全部 OK，结果不可用）
#
# 用法（业务库把 -Ns/-Db 换成站点实际 surreal_ns / 工程名）：
#   powershell -File scripts/smoke/pe_children_removal_audit.ps1 -Ns <surreal_ns> -Db <project> [-Dbnum <n>] [-Limit 0]
# 连接参数默认 127.0.0.1:8030 root/root（与 verify_022 系列一致），
# 可用参数或环境变量 AIOS_SURREAL_URL / AIOS_SURREAL_USER / AIOS_SURREAL_PASS 覆盖。

param(
    [string]$BaseUrl = $(if ($env:AIOS_SURREAL_URL) { $env:AIOS_SURREAL_URL } else { "http://127.0.0.1:8030" }),
    [string]$User = $(if ($env:AIOS_SURREAL_USER) { $env:AIOS_SURREAL_USER } else { "root" }),
    [string]$Pass = $(if ($env:AIOS_SURREAL_PASS) { $env:AIOS_SURREAL_PASS } else { "root" }),
    [string]$Ns = "smoke023",
    [string]$Db = "latest_tree",
    [int]$Dbnum = -1,
    # 0 = 全量（删字段闸门默认）；>0 = 每类抽样 LIMIT N（大表快速体检）
    [int]$Limit = 0,
    [string]$SurqlFile = "$PSScriptRoot\..\..\db-data\audit_pe_children_removal.surql",
    [string]$OutMd = "$PSScriptRoot\..\..\db-data\audit_pe_children_removal.out.md"
)

$ErrorActionPreference = "Stop"

$surql = Get-Content -Raw -Path $SurqlFile
if ($Dbnum -ge 0) {
    $surql = $surql -replace 'LET \$target_dbnum = NONE;', "LET `$target_dbnum = $Dbnum;"
}
# 全量 vs 抽样：/*__ROWS_LIMIT__*/ 占位符替换为空串或 "LIMIT N"
$limitClause = if ($Limit -gt 0) { "LIMIT $Limit" } else { "" }
$surql = $surql -replace '/\*__ROWS_LIMIT__\*/', $limitClause

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

$scanMode = if ($Limit -gt 0) { "sampled(LIMIT $Limit)" } else { "FULL" }
Write-Host "POST $BaseUrl/sql (ns=$Ns db=$Db dbnum=$(if ($Dbnum -ge 0) { $Dbnum } else { 'ALL' }) scan=$scanMode) ..."
$resp = Invoke-RestMethod -Method Post -Uri "$BaseUrl/sql" -Headers $headers -ContentType "text/plain" -Body $surql

# 语句顺序（与 surql 文件一一对应，LET 也占一个结果槽）：
#  0 banner0  1 LET target  2 target 回显
#  3 banner1  4 totals per dbnum  5 edge rows total
#  6 banner2  7 LET $rows  8 { parents, mismatched }  9 mismatch list
# 10 banner3 11 LET $stale 12 { stale_childless }     13 stale list
# 14 banner4 15 maintained_since per dbnum
# 16 banner5 17 LET $pe_dbnums 18 LET $meta_dbnums    19 { pe/meta/without_meta }
# 20 end banner
$totalsStmt   = $resp[4]
$edgeRowsStmt = $resp[5]
$summaryStmt  = $resp[8]
$mismatchStmt = $resp[9]
$staleSumStmt = $resp[12]
$staleStmt    = $resp[13]
$metaStmt     = $resp[15]
$boundaryStmt = $resp[19]

function Json($v, $fallback) {
    $j = ($v | ConvertTo-Json -Depth 8)
    if ($null -eq $j) { $fallback } else { $j }
}

$totalsJson   = Json $totalsStmt.result   "[]"
$edgeRowsJson = Json $edgeRowsStmt.result "[]"
$mismatchJson = Json $mismatchStmt.result "[]"
$staleJson    = Json $staleStmt.result    "[]"
$metaJson     = Json $metaStmt.result     "[]"
$boundaryJson = Json $boundaryStmt.result "{}"

$queriesOk = ($summaryStmt.status -eq "OK") -and ($staleSumStmt.status -eq "OK") -and `
    ($metaStmt.status -eq "OK") -and ($boundaryStmt.status -eq "OK") -and ($totalsStmt.status -eq "OK")

$parents    = if ($summaryStmt.result.parents)      { [int]$summaryStmt.result.parents }        else { 0 }
$mismatched = if ($summaryStmt.result.mismatched)   { [int]$summaryStmt.result.mismatched }     else { 0 }
$stale      = if ($staleSumStmt.result.stale_childless) { [int]$staleSumStmt.result.stale_childless } else { 0 }
$withoutMeta = @($boundaryStmt.result.without_meta | Where-Object { $null -ne $_ })
$withoutMetaCount = $withoutMeta.Count

$verdict = if (-not $queriesOk) {
    "ERROR：审计查询未全部返回 OK（totals=$($totalsStmt.status) summary=$($summaryStmt.status) stale=$($staleSumStmt.status) meta=$($metaStmt.status) boundary=$($boundaryStmt.status)），结果不可用"
} elseif ($mismatched -gt 0 -or $stale -gt 0) {
    "FAIL：$parents 个有子 parent 中 $mismatched 个边计数与 children 不一致；childless 残留脏边 $stale 条。删 pe.children 前先对这些 dbnum 执行 model-version rebuild-pe-owner，重跑本审计 PASS 后再进入 Phase A/B。"
} elseif ($withoutMetaCount -gt 0) {
    "PASS(带告警)：latest 边完整（$parents 个 parent 全部一致，无脏边）。但有 $withoutMetaCount 个 dbnum 无 pe_owner 可信起点：drop-now 删 children 后它们的边界前历史层级将彻底不可回溯（已按你的决策接受）。清单见 [5]。"
} else {
    "PASS：latest 边完整（$parents 个 parent 全部一致，无脏边），且全部 dbnum 均有 pe_owner 可信起点。可进入 Phase A/B。"
}

$now = Get-Date -Format "yyyy-MM-dd HH:mm:ss zzz"
$md = @"
# audit_pe_children_removal — 删 pe.children / 统一 pe_owner 的 Phase 0 go/no-go 审计

- 生成时间：$now（由 scripts/smoke/pe_children_removal_audit.ps1 生成，重跑会覆盖本文件）
- 实例：``$BaseUrl``（HTTP POST /sql，Basic $User/***）
- 环境：``ns=$Ns`` ``db=$Db`` ``dbnum=$(if ($Dbnum -ge 0) { $Dbnum } else { 'ALL' })`` ``scan=$scanMode``
- 审计 SQL：``db-data/audit_pe_children_removal.surql``（判定规则见该文件头注释）

## 判定

**$verdict**

## [1] 总量（per dbnum parents_with_children / children_total + 边行总数）

``````json
$totalsJson
``````

``````json
$edgeRowsJson
``````

## [2] 全量 parent 对比（count(<-pe_owner) vs len(children)）——不一致清单（最多 100 条）

- parents=$parents mismatched=$mismatched

``````json
$mismatchJson
``````

## [3] childless 行残留脏边（最多 100 条）

- stale_childless=$stale

``````json
$staleJson
``````

## [4] 各 dbnum 的 maintained_since_sesno（pe_owner_version_meta）

``````json
$metaJson
``````

## [5] 无 pe_owner 可信起点的 dbnum（without_meta）

- without_meta 数量=$withoutMetaCount（drop-now 后这些 dbnum 的边界前历史层级不可查）

``````json
$boundaryJson
``````

## 修复口径

- [2]/[3] 不绿：对相关 dbnum 执行 ``model-version rebuild-pe-owner --dbnum <n>``（先删后插全量重建，幂等可重跑），重跑本审计 PASS 后才进入 Phase A/B。
- [5] without_meta：若要保留这些 dbnum 的历史层级回溯，需在删 children 前对其 full 重灌或 rebuild-pe-owner 把可信起点建立起来；drop-now 策略下可接受其历史层级不可用。
"@

Set-Content -Path $OutMd -Value $md -Encoding UTF8
Write-Host ""
Write-Host "审计结果已写入 $OutMd"
Write-Host $verdict -ForegroundColor $(if (-not $queriesOk) { "Red" } elseif ($mismatched -gt 0 -or $stale -gt 0) { "Red" } elseif ($withoutMetaCount -gt 0) { "Yellow" } else { "Green" })

if (-not $queriesOk) { exit 2 }
if ($mismatched -gt 0 -or $stale -gt 0) { exit 1 }
exit 0
