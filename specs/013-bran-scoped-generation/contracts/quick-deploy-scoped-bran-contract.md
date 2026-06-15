# Contract: Quick Deploy Scoped BRAN

## Endpoint

`POST /api/admin/quick-deploy-test`

The admin-authenticated quick-deploy endpoint may use the same request/response shape, but v1 validation is centered on the unauthenticated local/test endpoint because the feature is a fast-test mode.

## Request

```json
{
  "project_path": "D:/path/to/AvevaPlantSample",
  "dbnum": 250160,
  "target_root_refno": "2013286704/476",
  "gen_model": true,
  "gen_mesh": true,
  "gen_spatial_tree": true,
  "start_site": true,
  "wait": true,
  "pipeline_db_mode": "ws"
}
```

### New Field

`target_root_refno`

- Type: string
- Optional
- Accepted forms: `2013286704/476`, `2013286704_476`
- Meaning: BRAN root to generate for fast scoped testing

### Backward Compatibility

If `target_root_refno` is absent or empty, the endpoint must behave as it does today.

## Successful Scoped Response

```json
{
  "success": true,
  "site_id": "quicktest-250160-8080",
  "dbnum": 250160,
  "parse_status": "Parsed",
  "generated": true,
  "entry_url": "http://127.0.0.1:8080/",
  "target_root_refno": "2013286704/476",
  "scoped_refno_count": 12,
  "scoped_viewer_url": "http://127.0.0.1:3101/?output_project=AvevaPlantSample&show_refno=2013286704_476&mbd_refno=2013286704_476&data_source=parquet",
  "warnings": []
}
```

Required response semantics:

- `target_root_refno` is canonical slash notation.
- `scoped_refno_count` is greater than zero.
- `scoped_viewer_url` uses the same target refno for `show_refno` and `mbd_refno`.
- Existing response fields remain present.

## Failure Responses

### Invalid Refno Format

```json
{
  "success": false,
  "message": "target_root_refno is not a valid refno: not-a-refno"
}
```

### Missing Target

```json
{
  "success": false,
  "message": "target_root_refno 2013286704/476 was not found in dbnum 250160"
}
```

### Dbnum Mismatch

```json
{
  "success": false,
  "message": "target_root_refno 2013286704/476 does not belong to dbnum 250160"
}
```

### Non-BRAN Target

```json
{
  "success": false,
  "message": "target_root_refno 2013286704/431 is EQUI; scoped generation v1 only supports BRAN"
}
```

Failure semantics:

- Failure occurs before generation starts.
- Failure must not create a full-generation fallback site.
- Failure must be visible in HTTP response body and logs.

## Frontend Validation Contract

Given `scoped_viewer_url`, automation must verify:

1. URL opens in plant3d-web.
2. Model loading targets `show_refno`.
3. MBD annotation targets `mbd_refno`.
4. MBD pipe panel opens.
5. Flow-direction control can toggle.

## Logging Contract

Scoped generation logs must include:

- `target_root_refno`
- `dbnum`
- scoped refno count
- scoped mode marker, such as `scoped_generation=true`
- generated viewer URL when available
