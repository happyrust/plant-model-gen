# specs/023 T021 端到端验收 smoke：按版本实时查询模型树
#
# 环境：T012 versioned fixture（AvevaMarineSample，dbnum=1112，surreal 8033 已运行，
#       锚点 sesno=897；scene_tree/1112.tree 与库同源）
# 流程：
#   Phase 0  直连 fixture surreal(8033) /sql 幂等 DELETE pe_owner_version_meta:1112 →
#            复位上次 run 的 rebuild 痕迹，保证脚本可重跑（Phase A fallback 断言的前提）
#   Phase A  起 web_server(3211) → 锚点列表 → 不传 sesno 基线 → sesno=897 版本查询
#            （此时无 pe_owner_version_meta → source 应为 pe_children_fallback）
#            → 404 AnchorMissing → 400 VersionUnsupported → node/ancestors/subtree 对照
#   Rebuild  model-version rebuild-pe-owner --dbnum 1112 --json（显式 --config 置于子命令前，
#            R0 断言连接横幅 ws 目标为 fixture 8033，出现 8020 主库立即 FAIL）
#   Phase B  同一批查询复跑 → source 应切换为 pe_owner，结果集与 Phase A 一致
#   Perf     大 owner（children≥900）版本 children 计时 ×5（SC-002 抽样）
#
# 依赖 curl.exe（Windows 10 1803+ 系统自带）：Windows PowerShell 5.1 的 Invoke-WebRequest
# 捕获非 200 响应时 $_.ErrorDetails.Message 常为空、且 GetResponseStream 已被消费，读不到
# 响应体（run2 中 A2/A3 由此误判 FAIL）；GetJson 对非 200 改用 curl.exe 重放同一幂等 GET 取证。
#
# 用法：powershell -File scripts/smoke/tree_version_smoke.ps1
#（可选 -SkipServerStart 复用已运行的 3211 服务）

param(
    [string]$ApiBase = "http://127.0.0.1:3211",
    [string]$ConfigPath = "db_options/DbOption-t012-e2e",
    [uint32]$Dbnum = 1112,
    [uint32]$Sesno = 897,
    [string]$ParentRefno = "17496_161421",   # SITE /生成模型问题汇总，7 子
    [string]$BigOwnerRefno = "17496_121844", # CFLOOR，924 子（perf 抽样）
    [string]$SurrealHttp = "http://127.0.0.1:8033",  # fixture surreal HTTP 端点（Phase 0 复位用）
    [string]$SurrealNs = "1516",
    [string]$SurrealDb = "AvevaMarineSample",
    [switch]$SkipServerStart
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $repo
if (-not (Get-Command curl.exe -ErrorAction SilentlyContinue)) {
    throw "需要 curl.exe（Windows 10 1803+ 自带）：GetJson 依赖它读取非 200 响应体"
}

$failed = 0
$serverProc = $null
function Check([string]$label, [bool]$ok, [string]$detail = "") {
    $mark = if ($ok) { "PASS" } else { "FAIL" }
    $color = if ($ok) { "Green" } else { "Red" }
    Write-Host ("[{0,-4}] {1} {2}" -f $mark, $label, $detail) -ForegroundColor $color
    if (-not $ok) { $script:failed++ }
}
function GetJson([string]$url) {
    try {
        $resp = Invoke-WebRequest -Uri $url -UseBasicParsing -TimeoutSec 60
        return @{ status = [int]$resp.StatusCode; body = ($resp.Content | ConvertFrom-Json) }
    } catch {
        if ($null -eq $_.Exception.Response) { throw }
        # 缺陷2 修复：Windows PowerShell 5.1 捕获非 200 时 $_.ErrorDetails.Message 常为空、
        # GetResponseStream 又已被 Invoke-WebRequest 消费，响应体取不到（run2 的 A2/A3 因此误判 FAIL，
        # 服务端行为实测正确）。本脚本命中非 200 的端点均为幂等 GET → 用 curl.exe 重放同一 URL，
        # 稳定取回 status + body。
        # curl 自行把 -w 格式里的 \n 转义为换行（避免向原生 exe 传带换行的参数）
        $curlOut = & curl.exe --silent --max-time 60 --output - --write-out '\n%{http_code}' $url
        $lines = @($curlOut)
        if ($lines.Count -lt 1) { throw "curl.exe 重放 $url 无输出" }
        $status = [int](("" + $lines[-1]).Trim())
        $raw = ""
        if ($lines.Count -ge 2) { $raw = ($lines[0..($lines.Count - 2)] -join "`n") }
        $parsed = $null; try { $parsed = $raw | ConvertFrom-Json } catch {}
        return @{ status = $status; body = $parsed; raw = $raw }
    }
}
function RefnoSet($children) { ($children | ForEach-Object { $_.refno }) -join "," }

try {
    # ===== Phase 0：复位 fixture 的 rebuild 痕迹（缺陷3·可重跑性）=====
    # Phase A 断言 source=pe_children_fallback 的前提是库内没有 pe_owner_version_meta:$Dbnum；
    # 一旦某次 run 的 rebuild 成功写入该 meta，脚本就不可重跑。fixture 中该表仅此一条记录，
    # 直连 fixture surreal /sql 删除（幂等无害），使每次 run 都从 rebuild 前状态出发。
    Write-Host "Phase 0：复位 pe_owner_version_meta:$Dbnum（$SurrealHttp，幂等）..."
    $basic = [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes("root:root"))
    try {
        $resetResp = Invoke-WebRequest -Uri "$SurrealHttp/sql" -Method Post -UseBasicParsing -TimeoutSec 30 `
            -ContentType "text/plain" `
            -Headers @{ Authorization = "Basic $basic"; "surreal-ns" = $SurrealNs; "surreal-db" = $SurrealDb; Accept = "application/json" } `
            -Body "DELETE pe_owner_version_meta:$Dbnum;"
    } catch {
        throw "Phase 0 复位失败：fixture surreal $SurrealHttp 不可达或 /sql 拒绝（$($_.Exception.Message)）；请确认 t012 fixture 实例在运行"
    }
    $resetOk = ($resetResp.StatusCode -eq 200) -and ($resetResp.Content -match '"status"\s*:\s*"OK"')
    Check "Phase0 DELETE pe_owner_version_meta:$Dbnum" $resetOk ("http=" + [int]$resetResp.StatusCode)

    # ===== 起服务 =====
    if (-not $SkipServerStart) {
        Write-Host "启动 web_server ($ApiBase, config=$ConfigPath) ..."
        $serverProc = Start-Process -FilePath "$repo\target\debug\web_server.exe" `
            -ArgumentList "--config", $ConfigPath `
            -RedirectStandardOutput "$repo\db-data\tree_version_smoke_server.out.log" `
            -RedirectStandardError "$repo\db-data\tree_version_smoke_server.err.log" `
            -PassThru -WindowStyle Hidden
    }
    $ready = $false
    foreach ($i in 1..60) {
        Start-Sleep -Seconds 2
        try {
            $null = Invoke-WebRequest -Uri "$ApiBase/api/model-history/anchors?dbnum=$Dbnum" -UseBasicParsing -TimeoutSec 5
            $ready = $true; break
        } catch {
            if ($null -ne $serverProc -and $serverProc.HasExited) { throw "web_server 进程已退出，见 db-data/tree_version_smoke_server.*.log" }
        }
    }
    if (-not $ready) { throw "web_server $ApiBase 未就绪" }

    # ===== 0. 锚点列表 =====
    $anchors = GetJson "$ApiBase/api/model-history/anchors?dbnum=$Dbnum"
    $hasAnchor = ($anchors.status -eq 200) -and (@($anchors.body.data.anchors | Where-Object { $_.sesno -eq $Sesno }).Count -ge 1)
    Check "anchors 列表含 sesno=$Sesno" $hasAnchor

    # ===== Phase A：rebuild 前（期望 fallback 源）=====
    $baseline = GetJson "$ApiBase/api/e3d/children/$ParentRefno"
    Check "A0 不传 sesno 基线 200/success" (($baseline.status -eq 200) -and $baseline.body.success) ("children=" + @($baseline.body.children).Count)
    Check "A0 基线响应不含 version 字段" ($null -eq $baseline.body.version)

    $verA = GetJson "$ApiBase/api/e3d/children/${ParentRefno}?sesno=$Sesno"
    Check "A1 children?sesno=$Sesno 200/success" (($verA.status -eq 200) -and $verA.body.success)
    Check "A1 version 信封(resolved=$Sesno, exact)" (($verA.body.version.resolved_sesno -eq $Sesno) -and $verA.body.version.exact)
    Check "A1 source=pe_children_fallback（rebuild 前）" ($verA.body.version.source -eq "pe_children_fallback") ("actual=" + $verA.body.version.source)
    Check "A1 集合与基线一致" ((RefnoSet $verA.body.children) -eq (RefnoSet $baseline.body.children))
    Check "A1 children_count 恒 null" (@($verA.body.children | Where-Object { $null -ne $_.children_count }).Count -eq 0)

    $miss = GetJson "$ApiBase/api/e3d/children/${ParentRefno}?sesno=1"
    Check "A2 sesno=1 → 404 AnchorMissing" (($miss.status -eq 404) -and ($miss.body.error.code -eq "AnchorMissing"))

    $unsup = GetJson "$ApiBase/api/e3d/visible-insts/${ParentRefno}?sesno=$Sesno"
    Check "A3 visible-insts?sesno → 400 VersionUnsupported" (($unsup.status -eq 400) -and ($unsup.body.error.code -eq "VersionUnsupported"))
    $unsup2 = GetJson "$ApiBase/api/e3d/site-nodes/${ParentRefno}?sesno=$Sesno"
    Check "A3 site-nodes?sesno → 400 VersionUnsupported" (($unsup2.status -eq 400) -and ($unsup2.body.error.code -eq "VersionUnsupported"))

    $node = GetJson "$ApiBase/api/e3d/node/${ParentRefno}?sesno=$Sesno"
    Check "A4 node?sesno 成功且带 version" (($node.status -eq 200) -and $node.body.success -and ($null -ne $node.body.version))

    $firstChild = @($baseline.body.children)[0].refno -replace "/", "_"
    $ancLatest = GetJson "$ApiBase/api/e3d/ancestors/$firstChild"
    $ancVer = GetJson "$ApiBase/api/e3d/ancestors/${firstChild}?sesno=$Sesno"
    Check "A5 ancestors?sesno 成功" (($ancVer.status -eq 200) -and $ancVer.body.success)
    $ancVerTail = (@($ancVer.body.refnos) | Select-Object -Last 2) -join ","
    $ancLatestTail = (@($ancLatest.body.refnos) | Select-Object -Last 2) -join ","
    Check "A5 版本祖先链尾部与 latest 一致" ($ancVerTail -eq $ancLatestTail) ("ver=" + (@($ancVer.body.refnos) -join "|") + " latest=" + (@($ancLatest.body.refnos) -join "|"))

    $subLatest = GetJson "$ApiBase/api/e3d/subtree-refnos/${ParentRefno}?max_depth=2&limit=500"
    $subVer = GetJson "$ApiBase/api/e3d/subtree-refnos/${ParentRefno}?max_depth=2&limit=500&sesno=$Sesno"
    $subLatestSet = @($subLatest.body.refnos) | Sort-Object
    $subVerSet = @($subVer.body.refnos) | Sort-Object
    # fallback 源以 pe.children 为准：tree 中存在但 pe 行缺失的"组节点"（本 fixture 的解析
    # 裁剪产物）其子树在 fallback 模式下取不到——只要求 ver ⊆ latest；严格等值在 Phase B 验证
    $extraInVer = @($subVerSet | Where-Object { $subLatestSet -notcontains $_ })
    Check "A6 subtree?sesno(fallback) ⊆ latest" ($extraInVer.Count -eq 0) ("ver_n=" + $subVerSet.Count + " latest_n=" + $subLatestSet.Count)

    # ===== Rebuild =====
    Write-Host ""
    Write-Host "执行 rebuild-pe-owner --dbnum $Dbnum ..."
    # 注意：必须用 --config 且置于子命令之前——main.rs 会用 --config（默认 db_options/DbOption，
    # 指向 8020 主开发库）覆盖 DB_OPTION_FILE 环境变量，仅设 env 会让 rebuild 打到错误实例。
    $env:DB_OPTION_FILE = $ConfigPath
    $rebuildOut = & "$repo\target\debug\aios-database.exe" --config $ConfigPath model-version rebuild-pe-owner --dbnum $Dbnum --json 2>&1
    $rebuildExit = $LASTEXITCODE
    $rebuildOut | Set-Content "$repo\db-data\tree_version_smoke_rebuild.out.txt" -Encoding UTF8

    # R0（缺陷1 防回归）：rebuild 必须连到 fixture 实例的 ws 端点。--config 丢失或位置不对时
    # main.rs 会回落默认 db_options/DbOption → 连接横幅出现 ws://…:8020（主开发库）→ 立即 FAIL。
    # 只匹配 ws:// 连接目标行，避开横幅里合法出现 8020 的 bind 配置回显行。
    $fixtureHostPort = $SurrealHttp -replace '^https?://', ''
    $rebuildText = ($rebuildOut | ForEach-Object { "$_" }) -join "`n"
    $wsTargets = @([regex]::Matches($rebuildText, 'ws://[\w\.\-]+:\d+') | ForEach-Object { $_.Value } | Select-Object -Unique)
    $connOk = ($wsTargets -contains "ws://$fixtureHostPort") -and (@($wsTargets | Where-Object { $_ -match ':8020$' }).Count -eq 0)
    Check "R0 rebuild 连接目标=ws://$fixtureHostPort（禁 8020 主库）" $connOk ("targets=" + ($wsTargets -join ","))

    Check "R1 rebuild-pe-owner 退出码 0" ($rebuildExit -eq 0) ("exit=" + $rebuildExit)
    $rebuildJsonText = ($rebuildOut | Where-Object { $_ -match '^\s*[\{\}"]' }) -join "`n"
    $rebuildJson = $null; try { $rebuildJson = $rebuildJsonText | ConvertFrom-Json } catch {}
    if ($null -ne $rebuildJson) {
        # 字段名与 src/version_management/cli.rs handle_rebuild_pe_owner_command 的 JSON summary 对齐：
        # nodes_processed / owners_with_children / owners_skipped / owners_rewritten /
        # edges_inserted / ghost_edges_deleted / maintained_since_sesno / meta_source / duration_ms
        Check "R2 rebuild 摘要 maintained_since_sesno=$Sesno, meta_source=rebuild_cli" `
            (($rebuildJson.maintained_since_sesno -eq $Sesno) -and ($rebuildJson.meta_source -eq "rebuild_cli")) `
            ("nodes=" + $rebuildJson.nodes_processed + " rewritten=" + $rebuildJson.owners_rewritten + " edges=" + $rebuildJson.edges_inserted + " ghost=" + $rebuildJson.ghost_edges_deleted)
    } else {
        Check "R2 rebuild JSON 摘要可解析" $false ("raw 见 db-data/tree_version_smoke_rebuild.out.txt")
    }

    # ===== Phase B：rebuild 后（期望 pe_owner 源、结果不变）=====
    $verB = GetJson "$ApiBase/api/e3d/children/${ParentRefno}?sesno=$Sesno"
    Check "B1 source=pe_owner（rebuild 后）" ($verB.body.version.source -eq "pe_owner") ("actual=" + $verB.body.version.source)
    Check "B1 集合与 Phase A 一致" ((RefnoSet $verB.body.children) -eq (RefnoSet $verA.body.children))
    Check "B1 名称与基线一致" ((@($verB.body.children | ForEach-Object { $_.name }) -join "|") -eq (@($baseline.body.children | ForEach-Object { $_.name }) -join "|"))

    $subVerB = GetJson "$ApiBase/api/e3d/subtree-refnos/${ParentRefno}?max_depth=2&limit=500&sesno=$Sesno"
    $subVerBSet = (@($subVerB.body.refnos) | Sort-Object) -join ","
    Check "B2 subtree（pe_owner 源）集合仍一致" ($subVerBSet -eq ($subVerSet -join ","))

    # ===== Perf 抽样（T022 / SC-002）=====
    Write-Host ""
    Write-Host "Perf 抽样：大 owner children?sesno（$BigOwnerRefno）×5 ..."
    $times = @()
    foreach ($i in 1..5) {
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        $perf = GetJson "$ApiBase/api/e3d/children/${BigOwnerRefno}?sesno=$Sesno&limit=2000"
        $sw.Stop()
        $times += $sw.ElapsedMilliseconds
        if ($i -eq 1) {
            Check "P0 大 owner 版本 children 成功" (($perf.status -eq 200) -and $perf.body.success) ("n=" + @($perf.body.children).Count + " source=" + $perf.body.version.source)
        }
    }
    $max = ($times | Measure-Object -Maximum).Maximum
    $avg = [math]::Round(($times | Measure-Object -Average).Average, 0)
    Check "P1 大 owner children P95≈max ≤ 1000ms" ($max -le 1000) ("times=" + ($times -join ",") + "ms avg=" + $avg + "ms")

    $ancTimes = @()
    foreach ($i in 1..5) {
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        $null = GetJson "$ApiBase/api/e3d/ancestors/${firstChild}?sesno=$Sesno"
        $sw.Stop()
        $ancTimes += $sw.ElapsedMilliseconds
    }
    $ancMax = ($ancTimes | Measure-Object -Maximum).Maximum
    Check "P2 ancestors ≤ 1000ms" ($ancMax -le 1000) ("times=" + ($ancTimes -join ",") + "ms")
}
finally {
    if ($null -ne $serverProc -and -not $serverProc.HasExited) {
        Stop-Process -Id $serverProc.Id -Force -ErrorAction SilentlyContinue
        Write-Host "web_server 已停止"
    }
}

Write-Host ""
if ($failed -gt 0) {
    Write-Host "共 $failed 项失败" -ForegroundColor Red
    exit 1
}
Write-Host "T021 端到端验收全部通过" -ForegroundColor Green
exit 0
