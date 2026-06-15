# Quickstart: BRAN Scoped Generation

## Goal

Generate only one BRAN, such as `2013286704/476`, and open the result in plant3d-web for automated MBD flow-direction validation.

## Prerequisites

- `plant-model-gen-cata-closure` web_server is built and runnable.
- `AIOS_ENABLE_QUICK_DEPLOY_TEST=1` is set before starting web_server.
- Quick-deploy sample project files for AvevaPlantSample/dbnum `250160` are available.
- plant3d-web build or dev server is available for frontend validation.

## Start Backend

```powershell
cd D:\work\plant-code\plant-model-gen-cata-closure
$env:AIOS_ENABLE_QUICK_DEPLOY_TEST = "1"
$env:ADMIN_USER = "admin"
$env:ADMIN_PASS = "admin"
target\debug\web_server.exe
```

Use the configured web_server port for the POST examples below.

## Submit Scoped Quick Deploy

```powershell
$body = @{
  project_path = "D:/work/plant-code/sample/AvevaPlantSample"
  dbnum = 250160
  target_root_refno = "2013286704/476"
  gen_model = $true
  gen_mesh = $true
  gen_spatial_tree = $true
  start_site = $true
  wait = $true
  pipeline_db_mode = "ws"
} | ConvertTo-Json -Depth 10

Invoke-RestMethod `
  -Method Post `
  -Uri "http://127.0.0.1:3100/api/admin/quick-deploy-test" `
  -ContentType "application/json" `
  -Body $body
```

Expected:

- `success=true`
- `generated=true`
- `target_root_refno="2013286704/476"`
- `scoped_refno_count > 0`
- `scoped_viewer_url` is present and contains:
  - `show_refno=2013286704_476`
  - `mbd_refno=2013286704_476`
  - `data_source=parquet`

## Validate Scoped Output

Inspect logs or response metadata:

- Generation logs include `scoped_generation=true`.
- Generation logs include target root and scoped refno count.
- Parquet/model output contains fewer instances than full dbnum generation.
- BRAN pipe segment data needed by MBD pipe annotation is present.

## Validate Failure Cases

Submit invalid targets and verify failure before generation:

```powershell
# Invalid format
@{ project_path="..."; dbnum=250160; target_root_refno="bad-refno" } | ConvertTo-Json |
  Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:3100/api/admin/quick-deploy-test" -ContentType "application/json"

# Non-BRAN sample, if known
@{ project_path="..."; dbnum=250160; target_root_refno="2013286704/431" } | ConvertTo-Json |
  Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:3100/api/admin/quick-deploy-test" -ContentType "application/json"
```

Expected:

- HTTP response contains a clear message.
- No full generation fallback starts.
- No successful scoped output is reported.

## Frontend Automated Validation

Open the returned `scoped_viewer_url` with Playwright or a browser.

Expected:

1. plant3d-web opens with the scoped BRAN target.
2. The viewer loads the BRAN subtree via `show_refno`.
3. MBD pipe annotation is requested via `mbd_refno`.
4. The MBD pipe panel opens.
5. The `流向` control is present and can toggle.

If the returned URL points to a deployed viewer, use that URL directly. If it points to a backend-only site, run plant3d-web locally and translate the query string to the local frontend origin.

## Backward Compatibility Smoke

Run quick deploy without `target_root_refno`.

Expected:

- Existing full quick-deploy behavior remains unchanged.
- Response does not include scoped metadata except as absent/null.
- Logs do not mark scoped generation.
