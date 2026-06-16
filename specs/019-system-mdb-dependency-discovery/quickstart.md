# Quickstart: System-Library MDB Dependency Discovery

## Prerequisites

- E3D project roots are available locally, for example under `D:\AVEVA\Projects\E3D2.1`.
- The managed-site web server can be started locally.
- Quick deploy admin APIs are reachable from the local machine.
- Do not run `cargo test` for `web_server`; repository rules require running-service validation.

## 1. Static/Type Validation

From the repository root:

```powershell
cargo fmt -- src/data_interface/mdb_candidates.rs src/parse_sidecar.rs src/web_server/models.rs src/web_server/admin_handlers.rs
cargo check --bin web_server --no-default-features --features "ws,gen_model,manifold,project_hd,surreal-save,write-to-surrealdb,sqlite-index,web_server,parquet-export,rvm-import"
```

Expected result:

- Formatting completes successfully.
- `cargo check` exits successfully.
- Existing third-party/parser warnings may appear; no new error should be introduced by this feature.

## 2. Verify MDB Candidate Discovery

Start the web server, then call the admin MDB candidates endpoint with a known project root. Use the currently configured admin authentication/session method if required by the running instance.

Example payload shape:

```json
{
  "project_name": "quicktest-mdb-discovery",
  "project_path": "D:\\AVEVA\\Projects\\E3D2.1\\AvevaPlantSample",
  "projects": [
    {
      "name": "AvevaPlantSample",
      "path": "D:\\AVEVA\\Projects\\E3D2.1\\AvevaPlantSample",
      "role": "design",
      "is_primary": true,
      "sort_order": 0
    },
    {
      "name": "AvevaCatalogue",
      "path": "D:\\AVEVA\\Projects\\E3D2.1\\AvevaCatalogue",
      "role": "library",
      "is_primary": false,
      "sort_order": 1
    }
  ]
}
```

Expected result:

- Response contains candidate MDB rows.
- Matching candidates include `source_file` and `source_db_type`.
- Member DB rows include `available`, `missing`, or `ambiguous` status.
- A deployable MDB has `ready_to_deploy = true`, `missing_count = 0`, and `ambiguous_count = 0`.

Observed on 2026-06-16 against a local web server on port `18080` with
`ADMIN_AIOS_DATABASE_BINARY` pointing to the current sidecar build:

- `AvevaPlantSample` + `AvevaCatalogue` returned 6 candidates for the sample
  payload. `/ALL` candidates included `source_file`, `source_db_type = "SYST"`,
  and member statuses including `available` and `ambiguous`.
- In this local data set, `/ALL` is not deployable because duplicate template
  DB files produce ambiguous members. Use `AvevaMarineSample` `/ALL1` for the
  ready-to-deploy success-path check below.

## 3. Verify Quick Deploy By MDB Name

Submit a quick deploy/admin quick-create request using MDB mode:

```json
{
  "mbd_name": "/ALL1",
  "search_roots": ["D:\\AVEVA\\Projects\\E3D2.1\\AvevaMarineSample"],
  "project_name": "quicktest-mdb-discovery",
  "dbnum": 7997,
  "create_only": true,
  "start_site": false,
  "wait": false,
  "pipeline_db_mode": "ws"
}
```

Expected result:

- The request resolves `projects`, `project_path`, `dbnum`, and `db_file` before site creation.
- Warnings include a summary naming the resolved MDB, project, target DB, target file, and dependency project count.
- The created site contains the full source project collection needed by the MDB.

Observed on 2026-06-16:

- `/ALL1` in `AvevaMarineSample` resolved `dbnum = 7997` and
  `resolved_db_file = "ams7997_0001"`.
- The create-only request returned a site id, `parse_status = "Pending"`, and
  `generated = false`.

## 4. Verify Failure On Missing Dependency

Run the same request with a narrowed search root that excludes a known catalogue/library project required by the MDB.

Expected result:

- Quick deploy fails before site creation/generation.
- Error message states that the MDB dependencies are incomplete or that a member DB is missing.
- The response includes actionable discovery warnings.

Observed on 2026-06-16:

- `/ALL` with `search_roots = ["D:\\AVEVA\\Projects\\E3D2.1\\AvevaPlantSample"]`
  and `dbnum = 250204` failed before site creation with
  `MBD /ALL 依赖不完整: missing=24, ambiguous=0`.

## 5. Verify Failure On Ambiguous Dependency

Use a search root that intentionally includes duplicate DB files with the same member dbnum across different project paths.

Expected result:

- Quick deploy fails before site creation/generation.
- Error message states that the MDB matched ambiguous target/member candidates.
- Candidate paths are included so the operator can narrow `search_roots` or provide explicit `projects[]`.

Observed on 2026-06-16:

- `/ALL` with `search_roots = ["D:\\AVEVA\\Projects\\E3D2.1"]` and
  `dbnum = 250204` failed before site creation with
  `MBD /ALL 依赖不完整: missing=0, ambiguous=36`.
- The quick-deploy failure response currently reports aggregate missing and
  ambiguous counts. Full ambiguous candidate paths are available from
  `/api/admin/projects/mdb-candidates`.

## 6. Verify Legacy Dbfile Mode

Submit a request without `mbd_name`, using the existing `db_file` field.

Expected result:

- Existing dbfile quick deploy behavior remains unchanged.
- No system-library MDB discovery is required for this mode.

## 7. Verify Drawer Path Fill Semantics

Open the admin site drawer and fill:

```text
root = D:\AVEVA\Projects\E3D2.1
name = AvevaCatalogue
```

Expected result:

- The drawer fills `project_path = D:\AVEVA\Projects\E3D2.1\AvevaCatalogue`.
- It also fills source project name and scan root when appropriate.
- It does not claim dependencies are verified until backend discovery/preview runs.
