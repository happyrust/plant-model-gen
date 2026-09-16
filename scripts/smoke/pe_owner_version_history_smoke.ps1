# Phase 0 第二项：pe_owner 图遍历在 VERSION 下返回“历史态”的自动断言 smoke
# 执行 db-data/smoke_pe_owner_version_history.surql，读末尾 RETURN 对象里的 gate_* 布尔判定。
#
# 与现有 pe_owner_version_capability_smoke.ps1 的区别：那个只判 status==OK（语法接受），
# 数值正确性靠人工核对；本脚本在 DB 内算出布尔 gate，直接自动判定“是否回溯到历史态”。
#
# 退出码：
#   0  PASS（边遍历 VERSION 返回历史态且 != 当前态 → drop-now 去 children 兜底后版本读安全）
#   1  FAIL（gate 不全 true：pe_owner 图遍历在 VERSION 下未回溯 → 不能删 children 兜底）
#   2  ERROR（查询报错，通常是实例非 versioned 或 fork 能力回退）
#
# 用法：powershell -File scripts/smoke/pe_owner_version_history_smoke.ps1 [-BaseUrl http://127.0.0.1:8030]
# 依赖：以 versioned=true 启动的 surreal 实例（db-data/run_surrealkv_versioned.ps1）。数据落独立 ns/db。

param(
    [string]$BaseUrl = $(if ($env:AIOS_SURREAL_URL) { $env:AIOS_SURREAL_URL } else { "http://127.0.0.1:8030" }),
    [string]$User = $(if ($env:AIOS_SURREAL_USER) { $env:AIOS_SURREAL_USER } else { "root" }),
    [string]$Pass = $(if ($env:AIOS_SURREAL_PASS) { $env:AIOS_SURREAL_PASS } else { "root" }),
    [string]$Ns = "smoke023",
    [string]$Db = "pehist",
    [string]$SurqlFile = "$PSScriptRoot\..\..\db-data\smoke_pe_owner_version_history.surql",
    [string]$ResultJson = "$PSScriptRoot\..\..\db-data\smoke_pe_owner_version_history.result.json"
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

# 先看有没有语句报错（非 versioned 实例会在带 VERSION 的语句上报错）
$failedStmts = @()
for ($i = 0; $i -lt $resp.Count; $i++) {
    if ($resp[$i].status -ne "OK") {
        $failedStmts += ("[{0}] {1}: {2}" -f $i, $resp[$i].status, ($resp[$i].result | Out-String).Trim())
    }
}

$final = $resp[$resp.Count - 1]
$g = $final.result

function B($v) { if ($null -eq $v) { $false } else { [bool]$v } }

$gateT1   = B $g.gate_children_t1_ok
$gateNow  = B $g.gate_children_now_ok
$gateTT   = B $g.gate_time_travels
$infoRange = B $g.info_idrange_t1_history_ok
$infoField = B $g.info_childrenfield_t1_ok

$gatesPass = $gateT1 -and $gateNow -and $gateTT
$queriesOk = ($final.status -eq "OK") -and ($failedStmts.Count -eq 0)

Write-Host ""
Write-Host "===== 原始取值 =====" 
Write-Host ("  edges_t1        -> {0}" -f (($g.edges_t1        | ConvertTo-Json -Compress)))
Write-Host ("  edges_now       -> {0}" -f (($g.edges_now       | ConvertTo-Json -Compress)))
Write-Host ("  idrange_t1      -> {0}" -f (($g.idrange_t1      | ConvertTo-Json -Compress)))
Write-Host ("  childrenfield_t1-> {0}" -f (($g.childrenfield_t1| ConvertTo-Json -Compress)))

Write-Host ""
Write-Host "===== gate（硬判定：边遍历 VERSION 是否回溯历史态）====="
Write-Host ("  gate_children_t1_ok  = {0}  (edges VERSION t1 == [a,b])" -f $gateT1)  -ForegroundColor $(if ($gateT1)  { "Green" } else { "Red" })
Write-Host ("  gate_children_now_ok = {0}  (edges 当前 == [a,c])"       -f $gateNow) -ForegroundColor $(if ($gateNow) { "Green" } else { "Red" })
Write-Host ("  gate_time_travels    = {0}  (t1==历史 且 != 当前)"       -f $gateTT)  -ForegroundColor $(if ($gateTT)  { "Green" } else { "Red" })

Write-Host ""
Write-Host "===== info（对照观测，不参与硬判定）====="
Write-Host ("  info_idrange_t1_history_ok = {0}  (research C3：预期 False=id区间扫在 VERSION 下不回溯)" -f $infoRange) -ForegroundColor $(if ($infoRange) { "Yellow" } else { "DarkGray" })
Write-Host ("  info_childrenfield_t1_ok   = {0}  (预期 True=pe.children 兜底可回溯，正是要放弃的)"       -f $infoField) -ForegroundColor $(if ($infoField) { "DarkGray" } else { "Yellow" })

Write-Host ""
if (-not $queriesOk) {
    Write-Host "ERROR：查询未全部 OK（很可能实例不是 versioned=true 启动，VERSION 查询被拒）。" -ForegroundColor Red
    if ($failedStmts.Count -gt 0) { $failedStmts | ForEach-Object { Write-Host ("  " + $_) -ForegroundColor Red } }
    Write-Host "用 db-data/run_surrealkv_versioned.ps1 起 versioned 实例后重跑。原始结果见 $ResultJson"
    exit 2
}
if ($gatesPass) {
    Write-Host "PASS：pe_owner 边遍历在 VERSION 下正确回溯历史态。drop-now 删掉 pe.children 兜底后，边界内版本读安全。" -ForegroundColor Green
    exit 0
} else {
    Write-Host "FAIL：pe_owner 边遍历在 VERSION 下未回溯历史态（gate 不全 true）。删 pe.children 会让边界内历史 children 查询静默错误——必须先解决该能力问题，drop-now 不可推进版本读部分。" -ForegroundColor Red
    exit 1
}
