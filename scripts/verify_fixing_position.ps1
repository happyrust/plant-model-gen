<# 
  FIXING 方位检查测试 (通过 web-server API)

  验证 FIXING 17496/152153 的世界坐标是否与 PDMS 一致：
    q pos wrt /* → Position X 5760.911mm Y 15408.258mm Z 5700mm

  前置条件：web-server 已启动

  用法:
    .\scripts\verify_fixing_position.ps1
    .\scripts\verify_fixing_position.ps1 -BaseUrl "http://localhost:8080"
#>
param(
    [string]$BaseUrl = "http://localhost:3000"
)

$tolerance = 1.0  # mm

$testCases = @(
    @{
        Refno       = "17496_152153"
        ExpectedX   = 5760.911
        ExpectedY   = 15408.258
        ExpectedZ   = 5700.0
        Description = "FIXING on curved wall (JUSL linkage)"
    }
)

Write-Host "=" * 55
Write-Host "  FIXING 方位检查 (via web-server API)"
Write-Host "  Server: $BaseUrl"
Write-Host "=" * 55

$passed = 0
$failed = 0

foreach ($case in $testCases) {
    Write-Host ""
    Write-Host ("-" * 55)
    Write-Host "  $($case.Refno) ($($case.Description))"

    $url = "$BaseUrl/api/pdms/transform/compute/$($case.Refno)"
    try {
        $resp = Invoke-RestMethod -Uri $url -Method Get -ErrorAction Stop
    }
    catch {
        Write-Host "   [FAIL] request error: $_" -ForegroundColor Red
        Write-Host "   hint: ensure web-server is running"
        $failed++
        continue
    }

    # print element info
    Write-Host "   noun       = $($resp.noun)"
    Write-Host "   owner      = $($resp.owner_refno) ($($resp.owner_noun))"
    Write-Host "   attrs      = $($resp.attrs | ConvertTo-Json -Compress)"

    if ($resp.local_translation) {
        $lt = $resp.local_translation
        Write-Host ("   local_pos  = ({0:F3}, {1:F3}, {2:F3})" -f $lt[0], $lt[1], $lt[2])
    }
    else {
        Write-Host "   [WARN] local_mat = None" -ForegroundColor Yellow
    }

    if ($resp.world_translation) {
        $wt = $resp.world_translation
        $dx = $wt[0] - $case.ExpectedX
        $dy = $wt[1] - $case.ExpectedY
        $dz = $wt[2] - $case.ExpectedZ
        $dist = [math]::Sqrt($dx * $dx + $dy * $dy + $dz * $dz)

        Write-Host ("   world_pos  = ({0:F3}, {1:F3}, {2:F3})" -f $wt[0], $wt[1], $wt[2])
        Write-Host ("   expected   = ({0:F3}, {1:F3}, {2:F3})" -f $case.ExpectedX, $case.ExpectedY, $case.ExpectedZ)
        Write-Host ("   diff       = ({0:F3}, {1:F3}, {2:F3})  |{3:F3}| mm" -f $dx, $dy, $dz, $dist)

        if ($dist -lt $tolerance) {
            Write-Host "   [PASS]" -ForegroundColor Green
            $passed++
        }
        else {
            Write-Host ("   [FAIL] diff {0:F3}mm > {1:F1}mm" -f $dist, $tolerance) -ForegroundColor Red
            $failed++
        }
    }
    else {
        Write-Host "   [FAIL] cannot compute world transform: $($resp.error_message)" -ForegroundColor Red
        $failed++
    }
}

Write-Host ""
Write-Host "=" * 55
Write-Host "  Result: $passed passed, $failed failed (total $($testCases.Count))"
Write-Host "=" * 55

if ($failed -gt 0) { exit 1 }
