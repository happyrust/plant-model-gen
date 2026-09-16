<#
.SYNOPSIS
  External workflow mode 回归 smoke：固化「单元测试 + 可选 HTTP probe」一键脚本。

.DESCRIPTION
  分两段：
    Stage 1（必跑）：cargo test 针对 workflow_sync 模块，覆盖 external 路径的核心单测：
                   - external_workflow_next_step_preserves_raw_assignee_id
                   - missing_workflow_mode_defaults_to_external_semantics
                   - debug_token_none_claims_sets_internal_mode
                   - unexpected_workflow_mode_treated_as_external
                   - external_workflow_return_skips_owner_match
                   - external_workflow_verify_skips_owner_match
                   - internal_workflow_owner_mismatch_is_forbidden
    Stage 2（可选）：对一个已启动的 web_server 发 HTTP 探测：
                   - 健康检查
                   - workflow/verify with workflow_mode=external + 错误 actor
                     应当 NOT 因 owner 不匹配阻断（可能因 form 不存在 / token 等其它原因失败，
                     但 block_code 不应是 OWNER_MISMATCH/INVALID_ACTOR_ID 系列）

.PARAMETER BaseUrl
  web_server base URL，默认 http://127.0.0.1:3100。

.PARAMETER DebugToken
  S2S debug_token 值（与 web_server 端 PLATFORM_AUTH_CONFIG.debug_token 一致）。
  未提供则跳过 Stage 2。

.PARAMETER FormId
  Stage 2 探测使用的 form_id。不存在也无所谓——目的只是看 owner 校验是否被跳过。

.PARAMETER SkipCargo
  跳过 Stage 1，仅做 HTTP probe。

.EXAMPLE
  pwsh -File scripts/smoke/external_workflow_smoke.ps1

.EXAMPLE
  pwsh -File scripts/smoke/external_workflow_smoke.ps1 -BaseUrl "http://127.0.0.1:3100" -DebugToken "dev-debug" -FormId "FORM-SMOKE-1"
#>

[CmdletBinding()]
param(
    [string]$BaseUrl   = "http://127.0.0.1:3100",
    [string]$DebugToken,
    [string]$FormId    = "FORM-EXTERNAL-SMOKE",
    [switch]$SkipCargo
)

$ErrorActionPreference = "Stop"
$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
Set-Location $repoRoot

$failed = @()

function Write-Section($title) {
    Write-Host ""
    Write-Host "=========================================" -ForegroundColor Cyan
    Write-Host " $title" -ForegroundColor Cyan
    Write-Host "=========================================" -ForegroundColor Cyan
}

# ----------------------------------------------------------------------------
# Stage 1: cargo test
# ----------------------------------------------------------------------------
if (-not $SkipCargo) {
    Write-Section "Stage 1: cargo test workflow_sync external paths"
    $testFilters = @(
        "platform_api::workflow_sync::tests::external_workflow_next_step_preserves_raw_assignee_id",
        "platform_api::workflow_sync::tests::missing_workflow_mode_defaults_to_external_semantics",
        "platform_api::workflow_sync::tests::debug_token_none_claims_sets_internal_mode",
        "platform_api::workflow_sync::tests::unexpected_workflow_mode_treated_as_external",
        "platform_api::workflow_sync::tests::external_workflow_return_preserves_raw_assignee_id",
        "platform_api::workflow_sync::tests::external_workflow_return_skips_owner_match",
        "platform_api::workflow_sync::tests::external_workflow_verify_skips_owner_match",
        "platform_api::workflow_sync::tests::internal_workflow_owner_mismatch_is_forbidden"
    )
    foreach ($filter in $testFilters) {
        Write-Host "→ cargo test --lib $filter -- --nocapture" -ForegroundColor Yellow
        & cargo test --lib $filter -- --nocapture --quiet
        if ($LASTEXITCODE -ne 0) {
            $failed += "cargo: $filter"
        }
    }
} else {
    Write-Host "Stage 1 skipped (-SkipCargo)" -ForegroundColor DarkYellow
}

# ----------------------------------------------------------------------------
# Stage 2: HTTP probe (optional)
# ----------------------------------------------------------------------------
if ($DebugToken -and -not [string]::IsNullOrWhiteSpace($DebugToken)) {
    Write-Section "Stage 2: HTTP probe — workflow/verify external mode"

    function Invoke-PlatformJson {
        param(
            [string]$Path,
            [hashtable]$Body
        )
        $json = $Body | ConvertTo-Json -Depth 8 -Compress
        $uri  = "$BaseUrl$Path"
        try {
            $response = Invoke-WebRequest -Method POST -Uri $uri `
                -Headers @{ "content-type" = "application/json" } `
                -Body $json -SkipHttpErrorCheck
            return [pscustomobject]@{
                Status = [int]$response.StatusCode
                Body   = ($response.Content | ConvertFrom-Json -AsHashtable -ErrorAction SilentlyContinue)
                Raw    = $response.Content
            }
        } catch {
            return [pscustomobject]@{
                Status = -1
                Body   = $null
                Raw    = $_.Exception.Message
            }
        }
    }

    # 2.1 healthz / 任意能确认服务在线的轻探测
    Write-Host "→ probing ${BaseUrl} reachable …"
    try {
        $ping = Invoke-WebRequest -Method GET -Uri $BaseUrl -SkipHttpErrorCheck -TimeoutSec 5
        Write-Host "  base reachable: status=$([int]$ping.StatusCode)"
    } catch {
        Write-Host "  base NOT reachable: $($_.Exception.Message)" -ForegroundColor Red
        $failed += "stage2: base unreachable"
    }

    # 2.2 verify with workflow_mode=external + 错误 actor.id —— 不应因 OWNER 阻断
    Write-Host "→ POST /api/review/workflow/verify (external + mismatched actor)"
    $verifyExt = Invoke-PlatformJson -Path "/api/review/workflow/verify" -Body @{
        form_id       = $FormId
        token         = $DebugToken
        action        = "agree"
        workflow_mode = "external"
        actor         = @{
            id    = "EXT_USER_SHOULD_NOT_MATCH"
            name  = "External Smoke"
            roles = "jd"
        }
        next_step     = @{
            assignee_id = "external_proofreader_99"
            name        = "External Proofreader"
            roles       = "sh"
        }
    }
    Write-Host "  status=$($verifyExt.Status)"
    Write-Host "  body  =$($verifyExt.Raw)"

    if ($verifyExt.Body) {
        $blockCode = $null
        if ($verifyExt.Body.ContainsKey("data") -and $verifyExt.Body.data) {
            $blockCode = $verifyExt.Body.data.block_code
        }
        if (-not $blockCode -and $verifyExt.Body.ContainsKey("error_code")) {
            $blockCode = $verifyExt.Body.error_code
        }
        $ownerBlockCodes = @("OWNER_MISMATCH", "INVALID_OWNER_ID", "INVALID_ACTOR_ID")
        if ($blockCode -and $ownerBlockCodes -contains $blockCode) {
            Write-Host "  EXPECT: not owner-related, GOT: $blockCode" -ForegroundColor Red
            $failed += "stage2: external mode triggered owner-related block_code=$blockCode"
        } else {
            Write-Host "  OK: external mode did not block on owner (block_code=$blockCode)" -ForegroundColor Green
        }
    }

    # 2.3 verify with workflow_mode=internal + 错误 actor.id —— 期望被阻断（OWNER_MISMATCH 或 INVALID_*）
    Write-Host "→ POST /api/review/workflow/verify (internal + mismatched actor)"
    $verifyInt = Invoke-PlatformJson -Path "/api/review/workflow/verify" -Body @{
        form_id       = $FormId
        token         = $DebugToken
        action        = "agree"
        workflow_mode = "internal"
        actor         = @{
            id    = "OTHER"
            name  = "Internal Smoke"
            roles = "jd"
        }
        next_step     = @{
            assignee_id = "SH"
            name        = "Internal Approver"
            roles       = "sh"
        }
    }
    Write-Host "  status=$($verifyInt.Status)"
    Write-Host "  body  =$($verifyInt.Raw)"
    # internal 模式下，若 form 真实存在，会得到 owner 相关的 block_code；若 form 不存在则可能得到 form not found。
    # 此处仅作信息输出，不强制断言（脚本不假设有真实 form）。
} else {
    Write-Host ""
    Write-Host "Stage 2 skipped (no -DebugToken passed)" -ForegroundColor DarkYellow
}

# ----------------------------------------------------------------------------
# Summary
# ----------------------------------------------------------------------------
Write-Section "Smoke summary"
if ($failed.Count -eq 0) {
    Write-Host "All stages passed." -ForegroundColor Green
    exit 0
} else {
    Write-Host "Failures:" -ForegroundColor Red
    foreach ($f in $failed) { Write-Host "  - $f" -ForegroundColor Red }
    exit 1
}
