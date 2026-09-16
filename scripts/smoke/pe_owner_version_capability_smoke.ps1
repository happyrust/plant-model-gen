# specs/023 FR-011 能力验证 smoke
# 验证 fork surreal（versioned 实例）对以下能力的支持度：
#   C0 VERSION 点查（基线，022 已验证）
#   C1 VERSION + 图遍历 FROM pe:X<-pe_owner
#   C2 VERSION + 图 idiom <-pe_owner.in
#   C3 VERSION + record id 区间扫（数组 id）
#   C4 VERSION 点查 pe.children（保底路径）
#   C5 已删除记录的历史点查
#   C6/C7 INSERT RELATION 撞已有 id / INSERT IGNORE 行为（幂等策略选型）
#
# 用法：powershell -File scripts/smoke/pe_owner_version_capability_smoke.ps1 [-BaseUrl http://127.0.0.1:8030]
# 依赖：一个以 versioned=true 启动的 surreal 实例（db-data/run_surrealkv_versioned.ps1）
# 数据写入独立 ns/db（smoke023/cap），不触碰业务与夹具数据。

param(
    [string]$BaseUrl = "http://127.0.0.1:8030",
    [string]$User = "root",
    [string]$Pass = "root",
    [string]$Ns = "smoke023",
    [string]$Db = "cap",
    [string]$SurqlFile = "$PSScriptRoot\..\..\db-data\smoke_023_pe_owner_version.surql",
    [string]$ResultJson = "$PSScriptRoot\..\..\db-data\smoke_023_result.json"
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

# 按 surql 文件中的语句顺序打标签
$labels = @(
    "setup: REMOVE pe",
    "setup: REMOVE pe_owner",
    "setup: CREATE pe:p",
    "setup: CREATE pe:a",
    "setup: CREATE pe:b",
    "setup: INSERT RELATION [a,b]",
    "setup: SLEEP",
    "setup: LET t1",
    "setup: SLEEP",
    "incr: CREATE pe:c",
    "incr: DELETE edge [p,1]",
    "incr: DELETE pe:b",
    "incr: UPDATE pe:a rename",
    "incr: UPDATE pe:p children",
    "incr: INSERT RELATION [c]",
    "incr: SLEEP",
    "incr: LET t2",
    "incr: SLEEP",
    "C0a: pe:a name VERSION t1 (expect childA)",
    "C0b: pe:a name VERSION t2 (expect childA_renamed)",
    "C1a: FROM pe:p<-pe_owner VERSION t1 (expect [a,b])",
    "C1b: FROM pe:p<-pe_owner VERSION t2 (expect [a,c])",
    "C2a: <-pe_owner.in VERSION t1 (expect [a,b])",
    "C2b: <-pe_owner.in VERSION t2 (expect [a,c])",
    "C3a: id range scan VERSION t1 (expect [a,b])",
    "C3b: id range scan VERSION t2 (expect [a,c])",
    "C4a: pe:p.children VERSION t1 (expect [a,b])",
    "C4b: pe:p.children VERSION t2 (expect [a,c])",
    "C5a: deleted pe:b VERSION t1 (expect childB)",
    "C5b: deleted pe:b current (expect empty)",
    "C6:  INSERT RELATION conflict on [p,0]",
    "C6v: edge [p,0].in after conflict (expect pe:a)",
    "C8:  INSERT RELATION same-value on [p,0]",
    "C8v: edge [p,0].in after same-value (expect pe:a)"
)

for ($i = 0; $i -lt $resp.Count; $i++) {
    $label = if ($i -lt $labels.Count) { $labels[$i] } else { "stmt#$i" }
    $status = $resp[$i].status
    $result = ($resp[$i].result | ConvertTo-Json -Depth 5 -Compress)
    if ($null -eq $result) { $result = "null" }
    if ($result.Length -gt 160) { $result = $result.Substring(0, 160) + "..." }
    $color = if ($status -eq "OK") { "Green" } else { "Yellow" }
    Write-Host ("[{0,-3}] {1,-4} {2}" -f $i, $status, $label) -ForegroundColor $color
    Write-Host ("      -> {0}" -f $result)
}

function Get-Stmt([int]$idx) { if ($idx -lt $resp.Count) { $resp[$idx] } else { $null } }
function Is-OK([int]$idx) { $s = Get-Stmt $idx; $null -ne $s -and $s.status -eq "OK" }

Write-Host ""
Write-Host "===== 能力判定 ====="
$verdicts = [ordered]@{
    "C0 VERSION 点查(基线)"        = (Is-OK 18) -and (Is-OK 19)
    "C1 VERSION+图遍历"            = (Is-OK 20) -and (Is-OK 21)
    "C2 VERSION+图 idiom"          = (Is-OK 22) -and (Is-OK 23)
    "C3 VERSION+id 区间扫"         = (Is-OK 24) -and (Is-OK 25)
    "C4 VERSION 点查 children(保底)" = (Is-OK 26) -and (Is-OK 27)
    "C5 删除历史点查"              = (Is-OK 28)
    "C8 同值重插后边完好"           = (Is-OK 33)
}
foreach ($k in $verdicts.Keys) {
    $v = $verdicts[$k]
    $mark = if ($v) { "PASS(语法)" } else { "FAIL" }
    $color = if ($v) { "Green" } else { "Red" }
    Write-Host ("  {0,-34} {1}" -f $k, $mark) -ForegroundColor $color
}
Write-Host ""
Write-Host "注意：PASS 仅代表语法接受且无错误返回；数值正确性请对照上方每条期望值人工核对，"
Write-Host "     核对结论需回填 specs/023-versioned-model-tree-query/（research/plan 阶段）。"

if (-not $verdicts["C0 VERSION 点查(基线)"] -or -not $verdicts["C4 VERSION 点查 children(保底)"]) {
    Write-Host "基线能力(C0/C4)未通过：versioned 实例不可用或 fork 能力回退，阻断。" -ForegroundColor Red
    exit 1
}
exit 0
