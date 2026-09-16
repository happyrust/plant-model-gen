# specs/022 T4: sesno_version_anchor 锚点链连续性审计
# 执行 db-data/audit_anchor_continuity.surql，输出全量锚点链 + 断链可疑项，
# 结果落 db-data/audit_anchor_continuity.out.md；存在可疑项时退出码 1。
#
# 用法（业务库审计时把 -Ns/-Db 换成站点实际 surreal_ns / 工程名）：
#   powershell -File scripts/smoke/anchor_continuity_audit.ps1 -Ns <surreal_ns> -Db <project>
# 连接参数默认 127.0.0.1:8030 root/root（与 verify_022 系列一致），
# 可用参数或环境变量 AIOS_SURREAL_URL / AIOS_SURREAL_USER / AIOS_SURREAL_PASS 覆盖。
#
# 修复口径（specs/022-versioned-pe-att-storage/ops-notes.md 第四节）：
# 历史断链只能对该 dbnum 全量重灌——锚点不可变，不做"补洞"式回填。

param(
    [string]$BaseUrl = $(if ($env:AIOS_SURREAL_URL) { $env:AIOS_SURREAL_URL } else { "http://127.0.0.1:8030" }),
    [string]$User = $(if ($env:AIOS_SURREAL_USER) { $env:AIOS_SURREAL_USER } else { "root" }),
    [string]$Pass = $(if ($env:AIOS_SURREAL_PASS) { $env:AIOS_SURREAL_PASS } else { "root" }),
    [string]$Ns = "vc_verify",
    [string]$Db = "continuity_gate",
    [string]$SurqlFile = "$PSScriptRoot\..\..\db-data\audit_anchor_continuity.surql",
    [string]$OutMd = "$PSScriptRoot\..\..\db-data\audit_anchor_continuity.out.md"
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

# surql 语句顺序：0=banner1 1=LET 全链快照 2=锚点链 3=banner2 4=可疑项 5=end banner
$chainStmt = $resp[2]
$suspectStmt = $resp[4]
$chainJson = $chainStmt.result | ConvertTo-Json -Depth 6
if ($null -eq $chainJson) { $chainJson = "[]" }
$suspectJson = $suspectStmt.result | ConvertTo-Json -Depth 6
if ($null -eq $suspectJson) { $suspectJson = "[]" }
$suspectCount = @($suspectStmt.result).Count
$chainCount = @($chainStmt.result).Count
$queriesOk = ($chainStmt.status -eq "OK") -and ($suspectStmt.status -eq "OK")

$verdict = if (-not $queriesOk) {
    "ERROR：审计查询未全部返回 OK（chain=$($chainStmt.status) suspects=$($suspectStmt.status)），结果不可用"
} elseif ($suspectCount -gt 0) {
    "FAIL：发现 $suspectCount 条断链可疑项——该 dbnum 历史存在未采集的 sesno 区间；修复口径=对该 dbnum 全量重灌（锚点不可变，不做补洞回填）"
} else {
    "PASS：未发现断链可疑项（Legacy Anchor 与 source='full' 基线重置锚点不参与判定；每 dbnum 首条锚点无前一锚点、无从判定）"
}

$now = Get-Date -Format "yyyy-MM-dd HH:mm:ss zzz"
$md = @"
# audit_anchor_continuity — sesno_version_anchor 锚点链连续性审计结果

- 生成时间：$now（由 scripts/smoke/anchor_continuity_audit.ps1 生成，重跑会覆盖本文件）
- 实例：``$BaseUrl``（HTTP POST /sql，Basic $User/***）
- 环境：``ns=$Ns`` ``db=$Db``
- 审计 SQL：``db-data/audit_anchor_continuity.surql``（判定规则与用法见该文件头注释）

## 判定

**$verdict**

## [1] 锚点链全量（dbnum, sesno 升序，共 $chainCount 条）

``````json
$chainJson
``````

## [2] 断链可疑项（from_sesno 存在、source != 'full'、from_sesno != 前一锚点 sesno + 1，共 $suspectCount 条）

``````json
$suspectJson
``````

## 修复口径

历史断链只能对该 dbnum **全量重灌**（重建锚点链基线）；锚点是 create-once 不可变发布记录，
不做"补洞"式回填——详见 ``specs/022-versioned-pe-att-storage/ops-notes.md`` 第四节。
"@

Set-Content -Path $OutMd -Value $md -Encoding UTF8
Write-Host ""
Write-Host "审计结果已写入 $OutMd"
Write-Host $verdict -ForegroundColor $(if ($suspectCount -gt 0 -or -not $queriesOk) { "Red" } else { "Green" })

if (-not $queriesOk) { exit 2 }
if ($suspectCount -gt 0) { exit 1 }
exit 0
