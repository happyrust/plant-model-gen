# Site Deployment Data and Viewer Validation

> Contract for validating managed site deployment data, HTTP reachability, and customer-facing Viewer readiness.

---

## Purpose

Managed site deployment must provide a diagnostic path that answers two questions:

1. Are the generated data artifacts structurally valid and internally consistent?
2. Can the customer-facing Viewer entry reach the same API and file resources that the browser needs to load a model?

This validation exists to catch failures where backend APIs appear healthy but the browser fails during Parquet or model loading, such as DuckDB WASM errors caused by missing, stale, corrupted, or incorrectly proxied Parquet resources.

---

## Scope

### First Version

The first version validates:

- Site service health.
- Customer Viewer entry page reachability.
- Nginx or same-origin `/files/` resource reachability.
- Required Parquet file presence and HTTP accessibility.
- Required Parquet schema fields.
- Lightweight sampled cross-table consistency.
- Key E3D API responses used by the Viewer.

### Explicitly Out of Scope for First Version

The first version does not:

- Execute DuckDB WASM in the browser.
- Open a hidden iframe and run a real Viewer smoke test.
- Fully scan every row in large Parquet datasets.
- Create a standalone validation page.

Browser-native Viewer smoke validation is a second-version feature and should build on this contract.

---

## Existing Integration Points

Reuse the existing deployment validation surface:

```text
GET  /api/admin/sites/{id}/deploy-validation
POST /api/admin/sites/{id}/deploy-validation
```

Validation results are persisted to:

```text
runtime/admin_sites/<site_id>/deploy-validation.json
```

Frontend callers should use the existing `sitesApi.deployValidation(id)` and `sitesApi.refreshDeployValidation(id)` methods.

---

## Data Flow

```text
ManagedProjectSite
  -> site runtime directory
  -> output project name
  -> parquet manifest and dbnum directories
  -> backend filesystem validation
  -> customer-facing HTTP validation
  -> deploy-validation.json
  -> SiteDetailView grouped validation UI
```

For each boundary:

- Filesystem validation proves the server generated artifacts.
- HTTP validation proves the customer entry can reach those artifacts through the same route family used by the browser.
- Schema and sample consistency validation prove the artifacts are likely consumable before invoking browser DuckDB.

---

## Report Contract

Continue returning the flat check list shape:

```ts
interface ManagedSiteDeployValidationCheck {
  key: string
  label: string
  status: string
  message: string
  detail?: string | null
  url?: string | null
  bytes?: number | null
}

interface ManagedSiteDeployValidationReport {
  site_id: string
  exists: boolean
  checked_at?: string | null
  blocking_count: number
  warning_count: number
  checks: ManagedSiteDeployValidationCheck[]
}
```

Do not introduce nested report groups in the API for the first version. The frontend should group checks by `key` prefix to preserve compatibility.

---

## Status Semantics

Use these status values:

- `ok`: the check passed.
- `warning`: the site may still be usable, but the condition can affect optional features or diagnosis confidence.
- `blocking`: the customer Viewer or core model loading path is likely unusable.

Examples:

- Missing `instances.parquet`: `blocking`.
- Missing optional `ptsets.parquet`: `warning` unless the manifest requires it.
- Viewer HTML reachable but DuckDB asset cannot be detected from the HTML: `warning`.
- HTTP Parquet URL returns 404 or HTML instead of the expected file: `blocking`.

---

## Required Check Keys

Use stable, prefix-based keys so the frontend can group results.

### Service Health

```text
service_api_status
service_site_identity
```

### Viewer Entry

```text
viewer_entry_url
viewer_index_html
viewer_main_asset
viewer_duckdb_asset
viewer_wasm_asset
```

`viewer_duckdb_asset` and `viewer_wasm_asset` may be `warning` if the asset cannot be reliably discovered in the first version.

### Parquet HTTP Reachability

```text
http_parquet_manifest_<dbnum>
http_parquet_instances_<dbnum>
http_parquet_geo_instances_<dbnum>
http_parquet_transforms_<dbnum>
http_parquet_aabb_<dbnum>
http_parquet_ptsets_<dbnum>
http_parquet_tubings_<dbnum>
```

Required files:

```text
manifest_<dbnum>.json
<dbnum>/instances.parquet
<dbnum>/geo_instances.parquet
<dbnum>/transforms.parquet
<dbnum>/aabb.parquet
```

Optional or warning-level files:

```text
<dbnum>/ptsets.parquet
<dbnum>/tubings.parquet
```

### Parquet Schema

```text
parquet_instances_schema_<dbnum>
parquet_geo_instances_schema_<dbnum>
parquet_transforms_schema_<dbnum>
parquet_aabb_schema_<dbnum>
```

Required fields:

`instances.parquet`:

```text
refno_str
refno_u64
noun
trans_hash
aabb_hash
dbnum
```

`geo_instances.parquet`:

```text
refno_str
refno_u64
geo_index
geo_hash
geo_trans_hash
```

`transforms.parquet`:

```text
trans_hash
m00
m10
m20
m30
m01
m11
m21
m31
m02
m12
m22
m32
m03
m13
m23
m33
```

`aabb.parquet`:

```text
aabb_hash
min_x
min_y
min_z
max_x
max_y
max_z
```

### Sample Consistency

```text
parquet_refs_sample_<dbnum>
mesh_refs_sample_<dbnum>
```

Sample checks should verify:

- Sampled `instances.trans_hash` values exist in `transforms.trans_hash`.
- Non-empty sampled `instances.aabb_hash` values exist in `aabb.aabb_hash`.
- Sampled `geo_instances.refno_u64` values exist in `instances.refno_u64`.
- Sampled `geo_instances.geo_hash` values have a matching mesh file candidate.

Sampling should be bounded. Prefer checking up to 1000 data rows and up to 100 mesh references per dbnum in the first version.

### Key API Checks

```text
api_e3d_world_root
api_e3d_subtree_refnos
api_e3d_visible_insts
```

These checks should use the same site backend reachable through the deployed route. If the customer Viewer is behind Nginx, validate through the same-origin `/api/` path where possible.

---

## Frontend UI Contract

The site detail page should expose a clear manual action:

```text
检查数据与前端加载
```

The action should call:

```ts
sitesApi.refreshDeployValidation(siteId)
```

The page should display:

- Overall state: pass, warning, failed, or not checked.
- Last checked time.
- Blocking count.
- Warning count.
- Grouped checks.
- First blocking failure as the most visible diagnostic.
- Clickable URLs for failed HTTP resource checks.

Frontend grouping should be derived from check key prefixes:

```text
service_*                    -> Service Health
viewer_*                     -> Viewer Entry
http_parquet_*               -> Parquet Files
parquet_*                    -> Data Consistency
mesh_*                       -> Model Resources
api_e3d_*                    -> Key APIs
```

Groups with `blocking` checks should expand by default. Groups with only `warning` checks should be highlighted but may stay collapsed.

---

## Triggering Rules

Validation should run:

- Manually from the site detail page.
- Automatically after successful `start`.
- Automatically after successful `restart`.
- Automatically after successful `deploy` or `redeploy`.
- Automatically after successful model generation when Parquet export is enabled.

Validation should not run:

- On every runtime poll.
- On every site list refresh.
- On log refresh.
- Implicitly on every page mount unless the user asks for it.

---

## Error Message Requirements

Validation messages must be actionable.

Bad:

```text
Parquet validation failed
```

Good:

```text
/files/output/AvevaPlantSample/parquet/250160/instances.parquet returned 404. Check output_project, Nginx /files proxy, and site output_root.
```

Good:

```text
instances.parquet is reachable, but schema is missing trans_hash. Regenerate Parquet output for dbnum 250160.
```

Good:

```text
Sampled 1000 geo_instances rows; 23 refno_u64 values were not found in instances.parquet.
```

---

## Implementation Guidance

Backend validation logic should be extracted from `managed_project_sites.rs` if the implementation grows beyond small helper functions.

Preferred module:

```text
src/web_server/site_data_validation.rs
```

`managed_project_sites.rs` should remain responsible for orchestration and persistence of the deployment validation report.

The validation implementation should avoid introducing a DuckDB dependency for the first version. Use existing Rust Parquet/Arrow capabilities already present in the project.

---

## Verification

Do not use Rust test suites for `web_server` validation work in this project.

Verify with:

- Running `web_server`.
- Running or starting a managed site.
- Calling `POST /api/admin/sites/{id}/deploy-validation`.
- Inspecting `runtime/admin_sites/<site_id>/deploy-validation.json`.
- Opening the site detail page and checking grouped results.
- Temporarily breaking one Parquet resource path or file and confirming the page reports the exact failing URL.

Recommended smoke cases:

1. Healthy site with generated Parquet returns zero blocking checks.
2. Missing `instances.parquet` returns a blocking check with a clickable URL.
3. Missing optional `ptsets.parquet` returns warning, not blocking.
4. Viewer entry HTML is unreachable returns blocking.
5. `/api/e3d/visible-insts` failure returns blocking with response detail.

---

## Future Version: Browser-Native Smoke

The second version may add a hidden iframe or separate Viewer smoke mode:

```text
/?output_project=<project>&show_dbnum=<dbnum>&smoke=1
```

The Viewer should report via `postMessage`:

```ts
{
  duckdbReady: boolean
  manifestLoaded: boolean
  instancesCount?: number
  geoInstancesCount?: number
  modelLoaded: boolean
  errors: Array<{ stage: string; message: string; url?: string }>
}
```

This should be implemented only after first-version backend validation is in place.

### DuckDB-WASM Trap Diagnostics

Browser errors such as:

```text
RuntimeError: null function or function signature mismatch
RuntimeError: table index is out of bounds
... at gt.OnMessage
... at _duckdb_web_query_run_buffer
```

must be treated as DuckDB-WASM worker traps, not as direct business-function stack traces.

`gt.OnMessage` is typically a minified worker message handler. It does not identify the application function that caused the failure.

Common causes to distinguish:

- Browser worker memory pressure or OOM corrupts the worker state.
- A specific Parquet query, `registerFileURL`, or HTTP Range read triggers a DuckDB-WASM internal bug.
- Remote deployment mixes stale browser cache with new static assets, causing old worker code and new `.wasm` binaries to be used together.
- Reverse proxy or cache serves mismatched Viewer assets from different builds.
- `-SkipFrontendBuild` or manual package overwrites leave the remote package with assets that do not match the local build.

Browser-native smoke should therefore report these stages separately:

```text
viewer_asset_version
duckdb_worker_asset
duckdb_wasm_asset
duckdb_worker_boot
duckdb_register_manifest
duckdb_register_parquet_files
duckdb_query_instances
duckdb_query_geo_instances
model_scene_load
```

When a DuckDB-WASM trap occurs, the smoke report must include:

- Main JS asset URL and cache status.
- DuckDB worker JS URL and cache status.
- DuckDB WASM URL and cache status.
- Parquet URL being registered or queried.
- Whether the response used HTTP Range.
- Row count or query that was executing.
- Browser name/version and available memory signal when available.

The first remediation step for this class of error is not code-level stack chasing. It is to prove asset and data coherence:

1. Disable browser cache or use an incognito window.
2. Verify all Viewer, worker, and WASM assets come from the same deployment build.
3. Verify Parquet URLs return real Parquet bytes, not HTML/JSON error pages.
4. Reproduce with a minimal DuckDB query against one Parquet file.
5. Only then investigate DuckDB-WASM version or Parquet encoding compatibility.
