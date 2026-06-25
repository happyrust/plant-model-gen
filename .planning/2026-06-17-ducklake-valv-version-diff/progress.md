# Progress

Active plan: `2026-06-17-ducklake-valv-version-diff`

## Current Status

Oracle MCP review findings have been converted into a concrete goal and phased
development target. Release registration, component diff, internal 3D compare,
delivery-unit MVP, mesh asset indexing/materialization, and the
`publish-history` historical release boundary are implemented. The remaining P0
blocker is still the second real session-derived release generated through an
isolated replay path rather than a controlled fixture.

Active slice: DB1112 baseline hydrate discovery. The immediate question is
whether existing full-sync/init-project code can create a complete isolated
baseline before `incremental-sesno` applies a history range, or whether a new
guarded baseline-hydrate/restore command is required.

## Completed

- Ran SigMap query for DuckLake/version diff planning context.
- Checked tool discovery for `plannator`; no callable plannator-specific tool was exposed in the current environment.
- Read the repository's existing plannotator-style plan format.
- Reviewed relevant repository areas:
  - Parquet model export
  - DuckLake model writer
  - transform DuckLake stub
  - delivery-unit noun definitions
  - current `version_management` module
- Created active planning files.
- Updated the plan to include component-to-delivery-unit impact propagation.
- Attempted Oracle MCP architecture review:
  - API engine failed because `OPENAI_API_KEY` is not set.
  - Browser engine dry-run succeeded.
  - Browser engine live run failed because Oracle's private Chrome profile is not initialized at `C:\Users\dpc\.oracle\browser-profile`.
- Completed Oracle MCP architecture review using browser mode after login.
- Updated the plan to add Oracle P0 corrections:
  - release graph
  - component identity and lineage
  - unit versions with aggregate hashes
  - deterministic propagation rules and unit dependency edges
- Added implementation strategy for the refactor in `implementation_strategy.md`.
- Added `GOAL.md` with success criteria, constraints, phase boundaries, and edge cases.
- Confirmed the next production-grade slice is Phase 1 release registration/listing.
- Implemented Phase 1 model release catalog:
  - Cargo feature `model-version-ducklake`.
  - `src/version_management/types.rs` for release/package DTOs.
  - `src/version_management/hashing.rs` for SHA-256 file/package hashes.
  - `src/version_management/release_package.rs` for manifest validation and immutable package materialization.
  - `src/version_management/ducklake_store.rs` for DuckLake attach, schema setup, release registration, release files, metadata, and listing.
  - `src/version_management/model_release.rs` service orchestration.
  - `src/version_management/cli.rs` CLI command builder and handler.
  - `aios-database model-version register`.
  - `aios-database model-version list`.
- Added `AIOS_QUIET_CONFIG` handling so machine-readable `model-version --json` output is not polluted by config logging.
- Registered the real DB `1112` Parquet package as release `ams-1112-sesno-897-phase1`.

## Next Actions

1. Implement Phase 2 component identity/version indexing:
   - component identity key strategy
   - component version hash fields
   - changed/added/deleted diff query
   - JSON CLI diff output
2. Generate/register a second real or controlled DB `1112` release from another sesno/package.
3. Build two-release comparison API surface after the CLI diff is stable.
4. Confirm business identifiers before Phase 2/3 expansion:
   - Is the valve noun in real data `VALV`, `VALE`, or both?
   - Should `EQUIP` always map to `EQUI`?
   - What should identify a release: sesno, task id, timestamp, or manual label?
5. Confirm impact rule:
   - Does any descendant component hash change mark the containing `BRAN` dirty?
   - Are there attribute-only changes that should not affect BRAN version state?
6. Confirm release graph scope:
   - Linear parent chain only for MVP?
   - Need branch id now or later?
   - Which derivation types matter first?
7. Design exact schemas for `unit_versions`, `component_identities`, `component_lineage`, `propagation_rules`, and `unit_dependency_edges`.
8. Register two real or controlled Parquet exports.
9. Implement component identity, lineage, and unit versions before component impact API.
10. Record CLI JSON evidence here.

## Evidence Log

- Oracle browser session completed: `ducklake-model-version-management-review-4`.
- Oracle transcript: `C:\Users\dpc\.oracle\sessions\ducklake-model-version-management-review-4\artifacts\transcript.md`.
- Oracle follow-up session completed: `e3d-model-version-architectu-2`.
- Follow-up report: `oracle_followup_2026-06-19.md`.
- SigMap note: the latest `sigmap ask` attempt for model-version implementation context timed out after 64s, so implementation scoping continued with `rg` and direct file reads.
- Build check passed: `cargo check --bin aios-database`.
- Feature build check passed: `cargo check --bin aios-database --features model-version-ducklake`.
- Feature executable build passed: `cargo build --bin aios-database --features model-version-ducklake`.
- Release registration command:
  - `./target/debug/aios-database.exe -c db_options/DbOption model-version register --release-id ams-1112-sesno-897-phase1 --release-label "AMS 1112 sesno 897 Phase 1" --dbnum 1112 --parquet-dir output/AvevaMarineSample/parquet/1112 --metadata-json '{"source":"codex_phase1_validation","from_sesno":896,"to_sesno":897}' --json`
  - First run status: `created`.
  - Second run status: `already_exists`.
  - Package hash: `2528ac85a3bdb6093bcaab9c894f64a63234c6c17f23ca219e62b6dc0185f81d`.
  - Immutable package: `output\AvevaMarineSample\model_versions\releases\ams-1112-sesno-897-phase1\parquet\1112`.
  - DuckLake metadata: `output\AvevaMarineSample\model_versions\metadata.ducklake`.
  - DuckLake data path: `output\AvevaMarineSample\model_versions\data`.
  - Row counts: instances=106, geo_instances=163, transforms=131, aabb=105, tubings=0, ptsets=237, primitive_keypoints=0.
- Release list command passed:
  - `./target/debug/aios-database.exe -c db_options/DbOption model-version list --json`
  - Returned the `ams-1112-sesno-897-phase1` release for project `AvevaMarineSample`.
- Negative idempotency check passed:
  - Re-registering `ams-1112-sesno-897-phase1` with `--parent-release-id unexpected-parent` fails with a clear parent mismatch error.
- Concurrency edge observed:
  - Running two `model-version register` processes against the same DuckLake metadata path concurrently can fail with a DuckLake metadata file lock on Windows.
  - Treat per-metadata registration as a serialized operation until a process-level queue/lock is added.
- `git diff --check` passed for the touched Phase 1 files, with only existing CRLF warnings.

## Self Review

- The Phase 1 implementation keeps DuckLake behind an explicit feature and does not change the default generation path.
- Release ids are path-safe validated before package materialization.
- Package registration validates `manifest.json`, required Parquet table entries, file presence, dbnum consistency, SHA-256 hashes, and immutable package hash idempotency.
- JSON output is clean after building the feature executable; `cargo check` alone does not refresh `target/debug/aios-database.exe`.
- Duplicate release handling now compares package hash, dbnum, project, branch, and stored parent edge.
- Current scope does not yet import Parquet rows into DuckLake snapshot tables; this is intentional for Phase 1 and should be done with component identity/version indexing in Phase 2.

## Phase 2 Component Snapshot Progress - 2026-06-19

- Updated `GOAL.md` to make Phase 2 MVP explicit: component snapshot indexing and JSON diff before UI comparison.
- Updated architecture docs with `model_version.component_snapshots`, `component_snapshot:v1`, and the exact hash payload.
- Implemented component snapshot types and JSON contracts.
- Implemented DuckLake tables `component_snapshots` and `component_index_runs`.
- Implemented component indexing from immutable release `instances.parquet` + `geo_instances.parquet`.
- Implemented `model-version index` and `model-version diff` CLI commands.
- Changed `model-version register` to ensure existing releases are component-indexed.
- Added a metadata lock file beside the DuckLake metadata file to serialize CLI/API access on Windows.
- Validation passed:
  - `cargo check --bin aios-database --features model-version-ducklake`
  - `cargo check --bin aios-database`
  - `cargo build --bin aios-database --features model-version-ducklake`
  - `model-version register` on `ams-1112-sesno-897-phase1` returned `already_exists` and `component_index.component_count=106`.
  - `model-version index` returned `component_count=106`, `distinct_component_hashes=106`.
  - same-release `model-version diff` returned `added=0`, `deleted=0`, `changed=0`, `unchanged=106`.
  - concurrent `model-version diff` + `model-version index` no longer failed with DuckLake metadata file lock after adding the local lock file.

Self review:

- The Phase 2 MVP keeps default generation unchanged and stays behind `model-version-ducklake`.
- Diff identity is intentionally conservative: `dbnum:refno_u64`; this is correct for the first DB1112 case but future reconstructed-history imports need lineage conflict detection.
- The hash currently includes instance fields and ordered geometry rows by hash reference. Full transform/AABB numeric joins are deferred until tolerance-aware geometry diff.
- Remaining gap for Phase 2 completion: register a second session-derived release and validate non-zero added/deleted/changed output from real history.

## Phase 2 Diff Evidence - 2026-06-19

- Added process-level metadata lock file handling beside DuckLake metadata:
  - Previous concurrent `model-version diff` + `model-version index` reproduced a Windows metadata file lock.
  - After the lock implementation, the same parallel commands completed successfully.
- Added actual Parquet row-count validation during release package loading:
  - The loader now compares manifest table row counts against Parquet file metadata when `parquet-export` is enabled.
  - Negative validation passed: a controlled bad manifest with `instances.rows=107` against an actual 106-row file fails with a clear mismatch error.
- Built a controlled second release package for non-zero diff validation:
  - Source package: `output\AvevaMarineSample\parquet\1112_phase2_component_diff_fixture`
  - Changed one `instances.parquet` row: `refno_u64=75144748307309`
  - Registered release: `ams-1112-phase2-fixture-cata-change`
  - Derivation type: `controlled-fixture`
  - Important caveat: this validates diff mechanics only; it is not a real sesno-derived release.
- Non-zero diff validation passed:
  - Command: `model-version diff --from-release-id ams-1112-sesno-897-phase1 --to-release-id ams-1112-phase2-fixture-cata-change --json`
  - Summary: `added=0`, `deleted=0`, `changed=1`, `unchanged=105`, `total_old=106`, `total_new=106`.
  - Changed component: `1112:75144748307309`, noun `FLOOR`.
  - `--change-type changed` filter returned the same single changed row.

Self review:

- Component snapshot indexing and diff are now verified for zero-diff and non-zero changed cases.
- Package validation is stronger than Phase 1 because it no longer trusts manifest row counts blindly.
- The remaining Phase 2 gap is still real historical release generation. Current `incremental-sesno` can collect a session range, but it mutates the current persisted state rather than publishing an isolated reconstructed historical state. A production historical comparison still needs a snapshot/reconstruction path or a controlled second generated package from a real station update.

## Web API And Compare Entry Evidence - 2026-06-19

- Added `src/web_api/model_version_api.rs`.
- Registered stateless routes in `src/web_api/mod.rs`:
  - `GET /api/model-version/releases`
  - `POST /api/model-version/releases/{release_id}/index`
  - `GET /api/model-version/diff`
  - `GET /model-version/compare`
- API behavior:
  - Release list uses the current project's `output/<project>/model_versions/metadata.ducklake`.
  - Release list returns `package_url`, `manifest_url`, and `viewer_url` for immutable package access through `/files/output/...`.
  - Index and diff calls run in `tokio::task::spawn_blocking` because DuckLake/Parquet access is synchronous.
  - Invalid `change_type` returns HTTP 400 with a JSON error envelope.
- Build validation passed:
  - `cargo check --bin web_server --features "web_server,model-version-ducklake"`
  - `cargo check --bin aios-database --features model-version-ducklake`
  - `CARGO_TARGET_DIR=target/codex-http-smoke cargo build --bin web_server --features "web_server,model-version-ducklake"`
- HTTP validation used an isolated smoke binary and port `3199` so existing `target/debug/web_server.exe` processes were not touched.
- A temporary smoke config disabled startup model/spatial generation and Surreal auto-start; it was removed after validation.
- HTTP validation passed:
  - `GET /api/version` returned version `0.3.34` and build date `2026-06-19 18:05:17 UTC+8`.
  - `GET /api/model-version/releases?dbnum=1112` returned two releases:
    `ams-1112-sesno-897-phase1` and `ams-1112-phase2-fixture-cata-change`.
  - `GET /api/model-version/diff?...&change_type=changed&limit=5` returned
    `added=0`, `deleted=0`, `changed=1`, `unchanged=105`, emitted row
    `1112:75144748307309` (`FLOOR`).
  - same-release HTTP diff returned `added=0`, `deleted=0`, `changed=0`,
    `unchanged=106`.
  - `POST /api/model-version/releases/ams-1112-sesno-897-phase1/index` returned
    `component_count=106`, `distinct_component_hashes=106`.
  - invalid `change_type=bad` returned HTTP 400 with message
    `change_type must be one of added, deleted, changed`.
  - immutable release manifest URL returned HTTP 200 with `dbnum=1112` and
    `instances.rows=106`.
  - `GET /model-version/compare` returned HTTP 200 HTML (`Model Version Compare`).

Self review:

- The HTTP layer is intentionally thin and reuses the CLI/domain implementation.
- The API does not expose arbitrary DuckLake file path query parameters, reducing accidental file access risk.
- Existing `/files/output` static serving is reused for immutable package manifests/files.
- The compare page is a working API/UI entry and builds two viewer URLs, but the actual `viewer/` dist is absent in this workspace. Real release-specific 3D rendering still requires the viewer to honor `parquet_base_url`/`model_release_id`.
- Unit impact remains pending because delivery-unit membership/version tables are not implemented yet.

## Release Runtime Scene And 3D Compare Evidence - 2026-06-19

- Added release-specific runtime scene support:
  - `GET /api/model-version/releases/{release_id}`
  - `GET /api/model-version/releases/{release_id}/runtime-scene`
  - `GET /model-version/release-viewer`
- The runtime scene API reads immutable release Parquet files through the
  version-management domain layer, not ad hoc frontend Parquet parsing:
  - `instances.parquet`
  - `geo_instances.parquet`
  - `transforms.parquet`
  - `aabb.parquet`
- Added scene DTOs with component metadata, instance matrix, geometry matrix,
  AABB, component hash, and mesh URL pattern data.
- Updated `/model-version/compare` so each pane embeds
  `/model-version/release-viewer?project=...&release_id=...`.
- Build validation passed:
  - `cargo fmt --check`
  - `cargo check --bin aios-database --features model-version-ducklake`
  - `cargo check --bin web_server --features "web_server,model-version-ducklake"`
  - `CARGO_TARGET_DIR=target/codex-http-smoke cargo build --bin web_server --features "web_server,model-version-ducklake"`
- HTTP validation used isolated port `3199` and temporary
  `DbOption-codex-http-smoke.toml`, removed after validation.
- HTTP validation passed:
  - release detail returned `ams-1112-sesno-897-phase1` and manifest `dbnum=1112`;
  - runtime scene with `limit=20` returned `components=20`, `geometries=34`,
    `mesh_lod_tag=L1`;
  - first mesh URL returned HTTP 200 and 1784 bytes;
  - full runtime scene returned `components=106`, `geometries=163`,
    `truncated=false`, matching manifest rows;
  - compare page and release viewer returned HTTP 200.
- Browser validation used `agent-browser` session `release-viewer-smoke`:
  - single release viewer loaded `34/34` geometries with `failed=0`;
  - two-pane compare loaded both releases with `163/163` geometries each and
    `failed=0`;
  - compare metrics showed `Added0`, `Deleted0`, `Changed1`, `Unchanged105`,
    `Emitted1`;
  - first diff row was `changed 17496_496493 FLOOR ...`.
- Browser screenshots:
  - `.planning/2026-06-17-ducklake-valv-version-diff/release-viewer-smoke-agent-browser.png`
  - `.planning/2026-06-17-ducklake-valv-version-diff/model-version-compare-smoke-agent-browser.png`

Self review:

- The new runtime scene API narrows frontend responsibility: the browser loads
  JSON and GLB assets, while DuckLake/Parquet access stays backend-owned.
- The internal release viewer is intentionally small but now proves two real
  WebGL panes can load immutable release geometry in the current workspace.
- This closes the previous "viewer dist absent" validation gap for the DB1112
  comparison demo, but it does not replace the future richer plant3d-web
  release integration.
- The second compared release is still the controlled fixture, not a real
  session-derived historical release.

## Delivery Unit Version And Impact Evidence - 2026-06-19

- Used Oracle MCP for a follow-up second-opinion architecture review:
  - Dry run succeeded with about 57.7k tokens and the focused source/doc set.
  - Live browser consult completed as session `e3d-model-version-ducklake-review`
    with transcript at
    `C:\Users\dpc\.oracle\sessions\e3d-model-version-ducklake-review\artifacts\transcript.md`.
  - Oracle confirmed the chosen boundary: SurrealDB remains the generation
    writer, Parquet/GLB/runtime scene remains the viewer package, and DuckLake
    is appropriate as release catalog plus component/unit diff query/audit layer.
  - Oracle called out the same P0 blockers: no second real session-derived
    release yet, mesh assets are not release-local/immutable yet,
    `dbnum:refno_u64` identity is not enough for production lineage, owner-chain
    unit semantics are still incomplete, and plant3d-web has not been proven to
    load release-specific packages.
- Implemented the first delivery-unit slice in `version_management`:
  - DuckLake tables: `delivery_unit_memberships`, `unit_versions`,
    `unit_index_runs`.
  - CLI: `model-version index-units`, `model-version unit-diff`,
    `model-version impact`.
  - API:
    `POST /api/model-version/releases/{release_id}/index-units`,
    `GET /api/model-version/unit-diff`,
    `GET /api/model-version/component-impact`.
- Membership rule set is intentionally conservative:
  - `BRAN`/`EQUI`/`WALL`/`FLOOR`/`HANG` are self units.
  - Components whose immediate owner is one of those nouns use `direct_owner`.
  - Other components are `UNASSIGNED` with an unresolved reason.
  - Owner-chain/tree-index paths such as `VALV -> EQUI -> BRAN` are still
    future hardening.
- Fixed a determinism bug found during validation:
  - `membership_hash` originally included `release_id`, which made all unit
    aggregate hashes change across releases.
  - The hash now excludes `release_id`, so unchanged memberships remain stable.
- CLI JSON validation passed:
  - `index-units` returned `unit_count=5`, `member_count=106`,
    `unresolved_member_count=0` for both releases.
  - `unit-diff` from real release to fixture returned `added=0`, `deleted=0`,
    `changed=1`, `unchanged=4`.
  - same-release `unit-diff` returned zero changed rows.
  - `impact --component-key 1112:75144748307309` returned
    `component_changes=1`, `impacted_units=1`, `rule_id=component_hash_changes_delivery_unit:v1`.
- HTTP validation passed on isolated port `3199`:
  - `POST /api/model-version/releases/ams-1112-sesno-897-phase1/index-units`
    returned `unit_count=5`, `member_count=106`.
  - `POST /api/model-version/releases/ams-1112-phase2-fixture-cata-change/index-units`
    returned `unit_count=5`, `member_count=106`.
  - `GET /api/model-version/unit-diff?...` returned one changed `FLOOR` unit
    and four unchanged units.
  - `GET /api/model-version/component-impact?...component_key=1112%3A75144748307309`
    returned one impacted unit with dependency/evidence JSON.
  - Invalid `unit_noun=PIPE` returned HTTP 400.
- Build validation passed:
  - `cargo fmt --check`
  - `cargo check --bin aios-database --features model-version-ducklake`
  - `cargo check --bin web_server --features "web_server,model-version-ducklake"`
  - `CARGO_TARGET_DIR=target/codex-http-smoke cargo build --bin web_server --features "web_server,model-version-ducklake"`

Self review:

- The slice is useful for DB `1112` because the fixture change is a `FLOOR`
  self-unit and the impact is correctly explainable.
- It is not yet the final production impact engine: indirect ownership,
  movement between units, and persisted dependency edges still need to be added.
- The full goal remains open until there is a second real session-derived
  release and the final user-facing viewer path is integrated or explicitly
  accepted as the internal release viewer.

## Release Mesh Asset Index Evidence - 2026-06-19

- Reused Oracle MCP session `e3d-model-version-ducklake-review` for the current
  architecture decision:
  - DuckLake is appropriate for release catalog, snapshot/index tables, diff,
    and audit queries.
  - DuckLake should not become the GLB/object asset store.
  - Release replay must pin or copy mesh assets; Parquet-only release packages
    are not enough for long-term 3D replay.
- Implemented release mesh asset indexing:
  - DuckLake tables: `model_release_mesh_assets` and
    `model_release_mesh_asset_index_runs`.
  - CLI:
    `model-version index-assets` and `model-version mesh-assets`.
  - API:
    `POST /api/model-version/releases/{release_id}/index-assets` and
    `GET /api/model-version/releases/{release_id}/mesh-assets`.
  - Derived manifest:
    `output/<project>/model_versions/asset_indexes/<release_id>/<dbnum>/mesh_assets_manifest.json`.
- The index reads immutable release `geo_instances.parquet`, extracts unique
  `geo_hash` values, resolves LOD GLB paths from the configured mesh root,
  records URL, relative path, absolute path, bytes, SHA-256, and whether the
  hash is a builtin primitive.
- CLI JSON validation passed:
  - `model-version index-assets --release-id ams-1112-sesno-897-phase1 --json`
    returned `geo_hash_count=6`, `present_count=6`, `missing_count=0`,
    `builtin_count=1`, `total_bytes=57136`.
  - `model-version index-assets --release-id ams-1112-phase2-fixture-cata-change --json`
    returned the same asset index hash
    `2d0e463fbc0a83b8771497ab8fa8d9fcedfe7cd0d88b0481bded489a77a07505`.
  - `model-version mesh-assets --release-id ams-1112-sesno-897-phase1 --json --limit 20`
    returned 6 asset rows with GLB URLs under `/files/meshes/lod_L1`.
  - `model-version mesh-assets --release-id ams-1112-sesno-897-phase1 --missing-only --json`
    returned an empty asset list.
- HTTP validation passed on isolated port `3199`:
  - `POST /api/model-version/releases/ams-1112-sesno-897-phase1/index-assets`
    returned `success=true`, `geo_hash_count=6`, `present_count=6`,
    `missing_count=0`, and the same asset index hash.
  - `GET /api/model-version/releases/ams-1112-sesno-897-phase1/mesh-assets?limit=20`
    returned the 6 indexed assets.
  - `GET /api/model-version/releases/ams-1112-sesno-897-phase1/mesh-assets?missing_only=true`
    returned no missing assets.
- Build validation passed:
  - `cargo fmt --check`
  - `cargo check --bin aios-database --features model-version-ducklake`
  - `cargo build --bin aios-database --features model-version-ducklake`
  - `cargo check --bin web_server --features "web_server,model-version-ducklake"`
  - `CARGO_TARGET_DIR=target/codex-http-smoke cargo build --bin web_server --features "web_server,model-version-ducklake"`

Self review:

- This closes the immediate audit gap that the system could not enumerate which
  GLB mesh files a release depends on.
- It still does not make releases fully self-contained: the current slice
  indexes and hashes global mesh assets, but does not copy or hard-link them
  into a release-local immutable package/object store.
- The next production hardening step is to add release-local mesh asset
  materialization and make the runtime-scene API prefer release-local mesh URLs.

## Release-Local Mesh Asset Materialization Evidence - 2026-06-19

- Implemented release-local mesh asset materialization:
  - CLI: `model-version index-assets --materialize`.
  - API:
    `POST /api/model-version/releases/{release_id}/index-assets?materialize=true`.
  - Destination:
    `output/<project>/model_versions/releases/<release_id>/meshes/lod_<tag>/<geo_hash>_<tag>.glb`.
  - Existing destination files are treated as immutable: the command verifies
    SHA-256 and fails on content mismatch instead of overwriting.
  - New files are copied through a temporary file, hash-checked, then renamed.
- Runtime scene now prefers release-local pinned mesh URLs:
  - If `model_versions/releases/<release_id>/meshes/lod_<tag>` exists,
    `GET /api/model-version/releases/{release_id}/runtime-scene` returns
    `mesh_base_url=/files/output/<project>/model_versions/releases/<release_id>/meshes/lod_<tag>`.
  - If release-local assets are absent, the API keeps the previous
    `/files/meshes/lod_<tag>` fallback.
- CLI JSON validation passed:
  - `model-version index-assets --release-id ams-1112-sesno-897-phase1 --materialize --json`
    returned `geo_hash_count=6`, `present_count=6`, `missing_count=0`,
    `builtin_count=1`, `total_bytes=57136`, and asset index hash
    `a4c9a3db2f2f6337d542be8bede4f729924091730f232cbbcf3445d451245292`.
  - The same command for `ams-1112-phase2-fixture-cata-change` returned the
    same asset index hash.
  - A repeated materialize run for `ams-1112-sesno-897-phase1` returned the
    same hash, proving the existing-file hash verification path is idempotent.
  - `mesh-assets --release-id ams-1112-sesno-897-phase1 --json --limit 20`
    returned release-local `mesh_relative_path=meshes/lod_L1/...` and
    `mesh_url=/files/output/AvevaMarineSample/model_versions/releases/...`.
  - Both release-local mesh directories contain 6 GLB files matching the
    indexed sizes.
- HTTP validation passed on isolated port `3199`:
  - `POST /api/model-version/releases/ams-1112-sesno-897-phase1/index-assets?materialize=true`
    returned `success=true`, `present_count=6`, `missing_count=0`, and the
    materialized asset hash.
  - `GET /api/model-version/releases/ams-1112-sesno-897-phase1/runtime-scene?limit=5`
    returned release-local `mesh_base_url`.
  - The first pinned GLB URL returned HTTP `200` with `Content-Length=1784`.
- Browser validation passed with `agent-browser` session
  `release-local-assets`:
  - URL:
    `/model-version/compare?from=ams-1112-sesno-897-phase1&to=ams-1112-phase2-fixture-cata-change`.
  - Left pane loaded `ams-1112-sesno-897-phase1` with
    `components 106 | geometries 163/163 | failed 0`.
  - Right pane loaded `ams-1112-phase2-fixture-cata-change` with
    `components 106 | geometries 163/163 | failed 0`.
  - Diff metrics remained `Added0`, `Deleted0`, `Changed1`,
    `Unchanged105`, `Emitted1`.
  - First diff row remained `changed 17496_496493 FLOOR`.
  - Screenshot:
    `.planning/2026-06-17-ducklake-valv-version-diff/model-version-compare-release-local-assets.png`.
- Build validation passed:
  - `cargo check --bin aios-database --features model-version-ducklake`
  - `cargo build --bin aios-database --features model-version-ducklake`
  - `cargo check --bin web_server --features "web_server,model-version-ducklake"`
  - `CARGO_TARGET_DIR=target/codex-http-smoke cargo build --bin web_server --features "web_server,model-version-ducklake"`

Self review:

- This closes the release replay asset gap for the DB `1112` demo: the two-pane
  viewer no longer depends on mutable global `/files/meshes` paths once
  materialization has run.
- The implementation is still intentionally file-package based, not an object
  store; larger deployments may later switch the copy step to hard-link,
  content-addressed storage, or CDN publishing behind the same asset index.
- The full Goal remains open because the second release is still a controlled
  fixture rather than a real session-derived historical release, and owner-chain
  unit semantics plus production plant3d-web integration remain pending.

## Production Architecture Plan Evidence - 2026-06-19

- Added production implementation baseline:
  `docs/plans/2026-06-19-e3d-model-version-production-architecture-dev-plan.md`.
- Used Oracle MCP for a generation-pipeline follow-up review:
  - Session: `e3d-real-release-plan-review`.
  - Transcript:
    `C:\Users\dpc\.oracle\sessions\e3d-real-release-plan-review\artifacts\transcript.md`.
  - Input included `incremental-sesno`, `gen_all_geos_data`,
    `post_gen_export`, model-version types/store/release files, and the
    existing architecture docs.
- Oracle confirmed the refined plan:
  - Use isolated SurrealDB namespace/database replay as the P0 path for the
    second real session-derived release.
  - Keep `no-save` / history-provider generation as a later cleaner design.
  - Do not use current namespace replay as production evidence for historical
    release comparison.
  - Keep DuckLake as release catalog/index/query/audit, not as generation
    writer or GLB object store.
- The new plan now covers:
  - Requirement analysis and full edge-case inventory.
  - Layered architecture and release package layout.
  - DuckLake table boundaries.
  - P0/P1/P2/P3/P4 development plan.
  - CLI JSON, HTTP, and browser verification strategy without `cargo test`.

## Publish-History CLI Safety Slice - 2026-06-19

- Implemented the first P0 safety slice:
  - CLI subcommand:
    `aios-database model-version publish-history`.
  - Types:
    `ModelHistoryReleasePublishRequest`,
    `ModelHistoryReleaseSafetyChecks`,
    `ModelHistoryReleasePublishResponse`.
  - Business function:
    `publish_history_model_release`.
- Scope:
  - Publishes an already generated isolated/staged Parquet package as a
    historical release.
  - Does not run isolated SurrealDB replay yet.
  - Explicitly records
    `history_publish.generation_performed_by_command=false`.
  - Registers the release with derivation type
    `incremental-sesno-isolated`.
  - Rejects the current default Parquet directory
    `output/<project>/parquet/<dbnum>` to avoid accidentally turning current
    mutable output into historical evidence.
  - Supports optional `--materialize-assets` and `--index-units`.
- Build validation:
  - `cargo check --bin aios-database --features model-version-ducklake`
    passed.
  - `cargo build --bin aios-database --features model-version-ducklake`
    passed.
  - Both commands emitted only existing upstream `pdms-io` warnings.
- CLI validation:
  - Help showed `publish-history` and required arguments:
    `--release-id`, `--dbnum`, `--source-db-file`, `--from-sesno`,
    `--to-sesno`, and `--parquet-dir`.
  - Negative current-output guard:
    publishing from `output\AvevaMarineSample\parquet\1112` failed with
    `refusing to publish historical release from current Parquet directory`.
  - Positive isolated-package smoke used temporary DuckLake paths under
    `target\codex-history-publish-smoke\positive` and source package
    `output\AvevaMarineSample\parquet\1112_phase2_component_diff_fixture`.
  - Positive JSON result:
    - `status=created`
    - `derivation_type=incremental-sesno-isolated`
    - `component_count=106`
    - `geo_instances.rows=163`
    - `mesh_asset_index.missing_count=0`
    - `mesh_asset_index.present_count=6`
    - `unit_index.unit_count=5`
    - `unit_index.member_count=106`
    - `safety_checks.generation_performed_by_command=false`
  - Re-running the same command returned `status=already_exists`, proving
    registration idempotency for the publish-history path.
  - `model-version list` against the temporary DuckLake catalog returned the
    published smoke release.
  - Invalid range `--from-sesno 897 --to-sesno 897` failed with
    `from_sesno=897 must be less than to_sesno=897`.
- Self review:
  - This slice moves P0 forward by adding the safe historical release publish
    boundary and preventing the known current-state pollution hazard.
  - It does not yet satisfy the full P0 requirement because the isolated replay
    package still needs a complete baseline state before it can become a second
    real session-derived release.

## Oracle MCP Continued Architecture Analysis - 2026-06-19

- Used Oracle MCP session inspection to re-read the completed
  `e3d-real-release-plan-review` result.
- Attempted a new narrower Oracle MCP consult for the next replay slice:
  - Dry-run resolved a focused 12-file bundle at about 158k tokens.
  - Live browser consult failed because Oracle's private Chrome profile at
    `C:\Users\dpc\.oracle\browser-profile` needs manual login.
  - Proceeded with the completed Oracle transcript plus local source analysis.
- Source analysis findings:
  - `main.rs` sets `DB_OPTION_FILE` before `aios_core::get_db_option()`.
  - `incremental-sesno` always connects to SurrealDB and calls
    `persist_pdms_increment_files` before optional generation.
  - `incremental-sesno --generate-model` only clones `DbOptionExt` in memory to
    set `manual_db_nums`; downstream helpers still depend on global
    `DB_OPTION_FILE` in several places.
  - `post_gen_export` writes to
    `DbOptionExt::get_project_output_dir()/parquet`, so an isolated
    `output_root` is sufficient only when the whole generation process runs
    with the isolated config.
- Architecture decision:
  - Do not make the next P0 slice an in-process `publish-from-sesno`.
  - Implement `model-version prepare-history-replay` first.
  - The command should write an isolated replay DbOption TOML and print JSON
    commands for:
    1. `aios-database -c <replay-config> incremental-sesno --generate-model`
    2. `aios-database -c <base-config> model-version publish-history`
  - A later job-level `publish-from-sesno` may orchestrate those stages by
    spawning separate processes, but it should not bypass the generated replay
    config boundary.
- Documentation updated:
  - `docs/plans/2026-06-19-e3d-model-version-production-architecture-dev-plan.md`
    now includes the replay planning boundary, CLI contract, JSON response
    shape, generated DbOption requirements, safety failures, and P0 acceptance
    criteria.
  - `docs/plans/2026-06-19-model-version-ducklake-architecture-plan.md` now
    records that DuckLake remains catalog/index/query/audit only, while
    historical generation runs through isolated replay config plus
    `publish-history`.

Self review:

- The plan directly addresses the two known pollution hazards: current
  SurrealDB namespace reuse and current Parquet directory reuse.
- The plan avoids fragile temporary mutation of global `DB_OPTION_FILE` and
  `aios_core` option state.
- No code was changed in this analysis step; implementation and validation of
  `prepare-history-replay` remain pending.

## Prepare-History-Replay CLI Slice - 2026-06-19

- Updated `GOAL.md` before implementation to make `model-version
  prepare-history-replay` the active slice and define completion criteria.
- SigMap note:
  - `sigmap ask "model-version prepare-history-replay DbOption output_root surreal_ns CLI"`
    timed out after about 74s, so implementation proceeded with `rg` and direct
    source reads.
- Implemented the second P0 safety slice:
  - New module:
    `src/version_management/history_replay_plan.rs`.
  - New DTOs:
    `ModelHistoryReplayPrepareRequest`,
    `ModelHistoryReplayCommands`,
    `ModelHistoryReplaySafetyChecks`,
    `ModelHistoryReplayPrepareResponse`.
  - New CLI:
    `aios-database model-version prepare-history-replay`.
- Behavior:
  - Reads the current base config from `DB_OPTION_FILE` / `-c`.
  - Writes a replay DbOption TOML to `--replay-config-out` or the default
    `output/<project>/model_versions/replay_configs/<release_id>.toml`.
  - Derives default replay namespace as
    `<current_surreal_ns>_history_<sanitized_release_id>`.
  - Derives default replay output root as
    `<current_project_output>/model_versions/replay_work/<release_id>/output`.
  - Overrides replay TOML fields:
    `project_name`, `surreal_ns`, `output_root`, `manual_db_nums`,
    `export_parquet_after_gen`, and `index_tree_debug_limit_per_target_type`.
  - Removes custom derived path keys so model cache and transform outputs fall
    back under the isolated replay output root.
  - Emits JSON with copyable `generate` and `publish` commands.
- Build validation:
  - `rustfmt --edition 2024` on touched Rust files passed.
  - `cargo check --bin aios-database --features model-version-ducklake` passed
    with only existing upstream `pdms-io` warnings.
  - `cargo build --bin aios-database --features model-version-ducklake` passed
    with the same upstream warnings.
- CLI positive validation:
  - Command generated
    `target\codex-history-replay-plan\codex-plan-history-smoke-1112.toml`
    for DB `1112`, source file
    `D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001`,
    and sesno range `896 -> 897`.
  - JSON response included:
    - `current_surreal_ns=1516`
    - `replay_surreal_ns=1516_history_codex_plan_history_smoke_1112`
    - `current_parquet_dir=output\AvevaMarineSample\parquet\1112`
    - `replay_parquet_dir=target\codex-history-replay-plan\replay_output\AvevaMarineSample\parquet\1112`
    - `generation_is_external_process=true`
    - `materialize_assets_in_publish_command=true`
  - Generated TOML contains:
    - `surreal_ns = "1516_history_codex_plan_history_smoke_1112"`
    - `output_root = "target/codex-history-replay-plan/replay_output"`
    - `manual_db_nums = [1112]`
    - `export_parquet_after_gen = true`
    - `index_tree_debug_limit_per_target_type = 0`
  - The generated replay config loaded successfully via:
    `aios-database -c target\codex-history-replay-plan\codex-plan-history-smoke-1112 model-version list --all-projects --ducklake-metadata target\codex-history-replay-plan\load-smoke\metadata.ducklake --ducklake-data target\codex-history-replay-plan\load-smoke\data --json`,
    returning an empty release list.
  - A repeated positive run with `--force` returned `overwritten=true`, updated
    the copied publish command with `--json` last, and left no stale `.bak`
    file after the backup/replace path.
- CLI negative validation:
  - Re-running with the same `--replay-config-out` without `--force` fails with
    `replay config already exists`.
  - `--replay-surreal-ns 1516` fails with
    `replay_surreal_ns must differ from current SurrealDB namespace`.
  - `--from-sesno 897 --to-sesno 897` fails with the expected range error.
  - `--release-id bad/release` fails path-safety validation.
  - Missing source DB file fails before config creation.
  - `--replay-output-root output` fails because it equals the current
    `output_root`.
  - Failed negative cases did not create replay config files.
- Documentation updated:
  - `docs/plans/2026-06-19-e3d-model-version-production-architecture-dev-plan.md`
    now marks `prepare-history-replay` implemented and records validation.
  - `docs/plans/2026-06-19-model-version-ducklake-architecture-plan.md` now
    says the remaining P0 proof is baseline hydrate/restore, non-empty replay
    validation, and publishing the second real session-derived release.

Self review:

- The implementation preserves the production boundary chosen by Oracle:
  prepare writes a config and command plan; it does not mutate current SurrealDB
  or execute generation in-process.
- Safety checks directly cover the current-state pollution risks:
  namespace reuse, current output root reuse, current Parquet reuse, and config
  overwrite.
- The command is idempotent with explicit `--force`; without `--force` it stops
  before overwriting a replay config.
- The full P0 goal remains open because the generated `incremental-sesno
  --generate-model` command has now been executed in an empty isolated
  namespace and produced a zero-row package; the next slice must hydrate or
  restore a baseline before publishing.

## DB1112 Empty-Namespace Replay Evidence - 2026-06-19

- Used the Oracle skill and Oracle MCP:
  - Read `C:\Users\dpc\.codex\skills\oracle\SKILL.md`.
  - Ran `npx -y @steipete/oracle --help`.
  - `mcp__oracle.consult` dry-run for `e3d-ducklake-version-plan` resolved a
    16-file bundle at about 148k tokens.
  - Started live Oracle browser consult `e3d-ducklake-version-plan`; the MCP
    call timed out at 120s, but `mcp__oracle.sessions` shows the session is
    still `running` with prompt submitted. Do not start a duplicate consult;
    reattach to this session later.
  - Re-read existing completed Oracle notes from
    `oracle_review.md`, `oracle_followup_2026-06-19.md`, and `findings.md`.
- Executed the generated replay command for DB `1112` using the isolated config:

```text
aios-database -c target\codex-history-replay-plan\codex-plan-history-smoke-1112 \
  incremental-sesno \
  --file D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001 \
  --from-sesno 896 \
  --to-sesno 897 \
  --generate-model \
  --json
```

- Command exit code: `0`.
- Log path:
  `target\codex-history-replay-plan\replay-generate-rerun.log`.
- Increment collector/persist evidence:
  - `actual_start_sesno=897`
  - `actual_end_sesno=897`
  - `session_count=1`
  - `element_count=169`
  - `add_count=168`
  - `modify_count=1`
  - `delete_count=0`
  - `data saved: sessions=1 pe=169 att=169 uda=0 deletes=0 dbnum_info=1`
  - generation received `incr_updates=118`
- Generation/export evidence:
  - Isolated `scene_tree` contains only `db_meta_info.json`; no
    `scene_tree/1112.tree` exists.
  - Logs report `dbnum 1112 did not find root node`.
  - Cleanup falls back because loading
    `target/codex-history-replay-plan/replay_output\AvevaMarineSample\scene_tree/1112.tree`
    fails.
  - IndexTree generation emits `batches=0`, `aabb=0`, `pts=0`, and boolean
    tasks `total=0`.
  - Export manifest row counts under
    `target\codex-history-replay-plan\replay_output\AvevaMarineSample\parquet\1112`
    are all zero:
    `instances=0`, `geo_instances=0`, `transforms=0`, `aabb=0`,
    `tubings=0`, `ptsets=0`, `primitive_keypoints=0`.

Architecture conclusion:

- `pdms-io` / `incremental-sesno` can read and persist the historical range.
- A single sesno range replayed into an empty isolated namespace is only a
  patch, not a complete historical model state.
- Historical model releases require one of:
  1. build or restore a baseline state at `from_sesno` inside the isolated
     namespace, then apply the selected range;
  2. restore a previously published baseline namespace/package and generate
     against that;
  3. implement a read-only history-state provider that lets generation query a
     complete model state at `to_sesno`.
- Do not publish the current zero-row replay as a full release. It must be
  rejected or labelled `patch_only`.

Documentation updated:

- `docs/plans/2026-06-19-e3d-model-version-production-architecture-dev-plan.md`
  now records the baseline requirement, patch-only edge case, and non-empty
  package gate.
- `docs/plans/2026-06-19-model-version-ducklake-architecture-plan.md` now
  distinguishes the current-state successful generation from the failed
  empty-namespace historical release proof.
- `GOAL.md` now lists baseline reconstruction and zero-row rejection before
  publishing a second real session-derived release.

## Oracle Final Architecture Consult - 2026-06-19

- Oracle MCP session `e3d-ducklake-version-plan` completed.
- Transcript:
  `C:\Users\dpc\.oracle\sessions\e3d-ducklake-version-plan\artifacts\transcript.md`
- Usage reported by Oracle:
  - input tokens: `147693`
  - output tokens: `3631`
  - elapsed: about `8m33s`
  - browser model resolved to `Pro Extended`
- Oracle confirmed the central architecture:
  - Keep SurrealDB as the current generation writer.
  - Keep Parquet/GLB as the viewer/release package.
  - Use DuckLake as release catalog, index, diff, and audit layer only.
  - Do not use DuckLake as the generation writer or GLB binary store for MVP.
- Additional findings to fold into implementation:
  - A historical sesno range replayed into an empty namespace is a patch, not a
    full model state. Baseline seed/restore is required before publish.
  - `publish-history` should become a domain-level atomic state machine:
    stage, validate Parquet, materialize release-local meshes, compute hashes,
    write reports, then register as `published`.
  - Read APIs must not auto-index or mutate DuckLake. Missing component/unit/
    asset indexes should return a dependency error with the required command or
    POST endpoint.
  - `prepare-history-replay` should eventually emit argv arrays as well as
    human-readable command strings so job runners do not rely on shell quoting.
  - DB1112 needs history-window discovery before choosing the visual diff
    sample; `896 -> 897` is now a negative empty-baseline case.
  - Local metadata locks are a transition measure. Production multi-writer or
    multi-host deployments should use a single-writer queue and/or a PostgreSQL
    DuckLake catalog.
  - The internal release viewer proves the mechanics, but production
    plant3d-web still needs explicit release-specific package support or a
    documented decision that the internal viewer is the accepted comparison UI.
- Documentation updated after Oracle:
  - Production architecture doc now records the completed Oracle session,
    atomic publish rule, read API no-mutation rule, and future argv command
    plan.
- DuckLake architecture doc now records the completed Oracle session,
    baseline replay rule, atomic publish rule, read API no-mutation rule,
    history candidate validation, and PostgreSQL catalog guidance.

## Historical Release Safety Gates - 2026-06-19

- Updated `GOAL.md` before implementation with the active slice:
  - reject zero-row historical model packages by default;
  - expose argv arrays from `prepare-history-replay`;
  - validate through CLI + JSON, without `cargo test`.
- SigMap note:
  - `sigmap ask "model-version publish-history zero row package guard prepare-history-replay argv commands"`
    timed out after about 94s, so implementation proceeded with `rg` and direct
    source reads.
- Implemented package guard:
  - `ModelHistoryReleaseSafetyChecks` now records `instances_rows`,
    `geo_instances_rows`, `non_empty_model_package`, and
    `zero_model_package_guard_enabled`.
  - `publish_history_model_release` now calls `load_model_package` and rejects
    packages whose `instances` or `geo_instances` row count is zero before
    `register_model_release` opens/writes DuckLake.
  - Error message explains that a sesno range replayed into an empty namespace
    is a patch, not a full 3D release, and tells the user to build/restore a
    baseline first.
- Implemented argv output:
  - `ModelHistoryReplayCommands` now includes `generate_argv` and
    `publish_argv`.
  - `prepare_history_replay` builds raw argv arrays first, then derives
    human-readable command strings from those arrays.
  - `materialize_assets_in_publish_command` safety check now inspects argv
    rather than the display string.
- Build validation:
  - `rustfmt --edition 2024 src\version_management\types.rs src\version_management\history_replay_plan.rs src\version_management\model_release.rs`
    passed.
  - `rustfmt --edition 2024 --check src\version_management\types.rs src\version_management\history_replay_plan.rs src\version_management\model_release.rs`
    passed.
  - `cargo check --bin aios-database --features model-version-ducklake` passed
    with only existing upstream `pdms-io` warnings.
  - `cargo build --bin aios-database --features model-version-ducklake` passed
    with the same upstream warnings.
  - `git diff --check` on the touched code/docs passed with only the existing
    CRLF warning for `progress.md`.
- CLI negative validation:
  - Command attempted to publish
    `target\codex-history-replay-plan\replay_output\AvevaMarineSample\parquet\1112`
    as release `codex-zero-row-guard-1112`.
  - Exit code was `1`.
  - Error:
    `refusing to publish historical release package with zero model rows: instances=0 geo_instances=0`.
  - Target test catalog/release directory
    `target\codex-history-safety-guard\zero` created no files, proving the
    guard ran before DuckLake/release registration.
  - Log:
    `target\codex-history-safety-guard\zero-publish.log`.
- CLI positive validation:
  - Published non-empty fixture package
    `output\AvevaMarineSample\parquet\1112_phase2_component_diff_fixture`
    into temporary catalog/release root
    `target\codex-history-safety-guard\nonempty`.
  - Release id: `codex-nonempty-guard-fixture-1112`.
  - Exit code was `0`.
  - JSON response included:
    - `status=created`
    - `instances_rows=106`
    - `geo_instances_rows=163`
    - `non_empty_model_package=true`
    - `zero_model_package_guard_enabled=true`
    - `component_count=106`
  - Log:
    `target\codex-history-safety-guard\nonempty-publish.log`.
- CLI argv validation:
  - Ran `prepare-history-replay --json` for release `codex-argv-plan-1112`.
  - JSON response included:
    - `commands.generate_argv` length `12`;
    - `commands.publish_argv` length `26`;
    - generate argv contains `--json`;
    - publish argv contains `--materialize-assets`;
    - publish argv ends with `--json`;
    - safety check `materialize_assets_in_publish_command=true`.
  - Log:
    `target\codex-history-safety-guard\prepare-argv.log`.
- Documentation updated:
  - Production architecture doc now records the implemented zero-row guard and
    argv output.
  - DuckLake architecture doc now records the implemented negative DB1112 case
    and notes that the broader multi-state publish job remains pending.

Self review:

- This slice directly addresses the DB1112 empty-namespace replay failure mode:
  a patch-only replay can no longer become a full historical 3D release through
  `publish-history`.
- The guard is intentionally conservative. A future diagnostic patch artifact
  flow can be added separately, but it should not reuse the normal published
  release path without explicit status semantics.
- The broader Oracle recommendation for atomic publish is not fully complete:
  `publish-history` still registers before optional asset/unit indexing. The
  next production slice should introduce a publish job/state machine or at
  least keep incomplete releases out of read APIs.

## Read Path No-Mutation - 2026-06-19

- Updated `GOAL.md` before implementation with the active slice:
  - read/query paths must not auto-index or otherwise mutate DuckLake;
  - missing component/unit/asset indexes should fail with an explicit
    dependency error and remediation command/API;
  - explicit mutating paths remain `register`, `publish-history`, `index`,
    `index-units`, `index-assets`, and matching POST endpoints.
- SigMap note:
  - `sigmap ask "model-version read APIs auto-index ensure_release_components_indexed diff release_scene side effects"`
    timed out after about 94s, so implementation proceeded with `rg` and direct
    source reads.
- Implemented DuckLake read-path guards:
  - Added `require_release_components_indexed` and changed `diff_releases` and
    `release_scene` to call it instead of `ensure_release_components_indexed`.
  - Added `require_release_units_indexed` and changed `diff_units` and
    `component_unit_impacts` to call it instead of `ensure_release_units_indexed`.
  - The `require_*` helpers reject both missing indexes and stale row-count
    mismatches with actionable CLI/API remediation text.
  - Explicit index/publish/register paths still call `ensure_*` where mutation
    is intended.
- Implemented HTTP error mapping:
  - `src/web_api/model_version_api.rs` maps `"missing dependency"` and
    `"index is missing"` errors to HTTP `424 Failed Dependency`.
- Source inspection:
  - `ensure_release_components_indexed` remains only in registration/explicit
    component indexing/explicit unit indexing paths.
  - `diff_releases`, `release_scene`, `diff_units`, and
    `component_unit_impacts` now use `require_*`.
- Build validation:
  - `rustfmt --edition 2024 src\version_management\ducklake_store.rs src\web_api\model_version_api.rs`
    passed.
  - `cargo check --bin aios-database --features model-version-ducklake` passed.
  - `cargo check --bin web_server --features "web_server,model-version-ducklake"`
    passed.
  - `cargo build --bin aios-database --features model-version-ducklake` passed.
  - `cargo build --bin web_server --features "web_server,model-version-ducklake"`
    against the default target was blocked by two already-running local
    `web_server.exe` processes holding `target\debug\web_server.exe`.
  - A separate-target web build initially failed because the linker used the
    nearly-full C: temp drive; rerunning with `TEMP`/`TMP` pointed at
    `target\codex-build-temp` succeeded.
- CLI negative validation:
  - Temporary catalog:
    `target\codex-read-no-mutation-cli-20260619-212730`.
  - Registered two DB `1112` non-empty packages, then deleted only
    `model_version.component_index_runs` and
    `model_version.component_snapshots` from that temporary DuckLake catalog.
  - `model-version diff --json` exited with code `1`.
  - Error:
    `missing dependency: component index is missing for release 'read-nomut-from-20260619-212730'`.
  - Post-failure DuckLake counts stayed
    `component_index_runs=0`, `component_snapshots=0`, proving the read command
    did not auto-index.
- CLI positive validation:
  - Ran explicit `model-version index --json` for both releases.
  - Each index reported `component_count=106`.
  - The same `model-version diff --json` then succeeded with
    `changed=1`, `unchanged=105`, `total_old=106`, `total_new=106`.
  - The changed row is component key `1112:75144748307309`
    (`FLOOR`, refno `17496_496493`).
- HTTP negative validation:
  - Temporary web catalog/config:
    `target\codex-read-no-mutation-http-20260619-213117`.
  - `GET /api/model-version/diff?project=AvevaMarineSample&from_release_id=http-nomut-from-20260619-213117&to_release_id=http-nomut-to-20260619-213117&limit=10`
    returned HTTP `424`.
  - Response body had `success=false` and the same missing dependency message
    with both CLI and POST remediation.
  - Post-GET DuckLake counts stayed
    `component_index_runs=0`, `component_snapshots=0`, proving the web GET path
    did not auto-index.
  - Response body:
    `target\codex-read-no-mutation-http-20260619-213117\http-diff-missing-index-response.json`.
- Cleanup:
  - Removed the large separate build target
    `target\codex-read-no-mutation-web-target` and temporary linker directory
    `target\codex-build-temp`.

Self review:

- This slice closes the read-path mutation risk identified by Oracle: query
  APIs now fail fast when derived indexes are absent instead of hiding a broken
  publish/index job.
- Existing explicit index and publish flows remain mutating by design.
- The next production slice should still add release lifecycle state
  (`staged`, `indexed`, `published`, `failed`) so read APIs can reject
  incomplete releases before the caller reaches low-level missing-index errors.

## Historical Replay Baseline Gate - 2026-06-19

- Updated `GOAL.md` before implementation with the active slice:
  - add a read-only replay package validation command;
  - classify staged replay packages as publishable visual releases or
    patch-only/empty-baseline artifacts;
  - fail by default for zero-row visual packages and current mutable Parquet
    paths;
  - keep scene_tree evidence visible but optional by default because the sampled
    non-empty DB `1112` package is loadable without a `1112.tree` file in the
    current scene_tree directory.
- SigMap note:
  - `sigmap ask "history replay baseline restore gate incremental-sesno prepare-history-replay publish-history DB1112 baseline scene tree"`
    timed out after about 124s, so implementation proceeded with `rg` and
    direct source reads.
- Implemented validation model:
  - Added `ModelHistoryReplayValidationRequest`,
    `ModelHistoryReplayPathChecks`, `ModelHistoryReplayPackageEvidence`,
    `ModelHistoryReplaySceneTreeEvidence`, and
    `ModelHistoryReplayValidationResponse`.
  - Added `src/version_management/history_replay_validation.rs`.
  - Validation is read-only: it does not write DuckLake, generate models, or
    mutate SurrealDB.
  - Classification values exercised in validation:
    - `patch_only_empty_baseline`
    - `complete_visual_release_candidate`
    - `unsafe_current_output`
    - `missing_scene_tree_baseline`
- Implemented CLI:
  - Added `aios-database model-version validate-history-replay`.
  - Required args:
    `--dbnum`, `--source-db-file`, `--from-sesno`, `--to-sesno`,
    `--parquet-dir`.
  - Optional args:
    `--current-parquet-dir`, `--scene-tree-dir`, `--require-scene-tree`,
    `--allow-patch-only`, `--json`.
  - JSON output includes row counts, path checks, scene-tree evidence,
    classification, `ready_for_publish`, and recommended action.
  - `--allow-patch-only` keeps exit code `0` for diagnostics while still
    reporting `ready_for_publish=false`.
- Integrated publish guard:
  - `publish-history` now calls `validate_history_replay_package` and
    `ensure_history_replay_publishable`.
  - This keeps manual validation and publish-time rejection aligned.
  - `publish-history` metadata now records the validation classification.
- Build validation:
  - `rustfmt --edition 2024 src\version_management\types.rs src\version_management\history_replay_validation.rs src\version_management\model_release.rs src\version_management\cli.rs src\version_management\mod.rs`
    passed.
  - `rustfmt --edition 2024 --check ...` on the same files passed.
  - `cargo check --bin aios-database --features model-version-ducklake` passed.
  - `cargo build --bin aios-database --features model-version-ducklake` passed.
  - `git diff --check` for touched source/GOAL files passed with only an
    existing CRLF warning for `src/version_management/mod.rs`.
- CLI help validation:
  - `target\debug\aios-database.exe model-version validate-history-replay --help`
    showed the expected command and options.
- CLI negative validation:
  - Package:
    `target\codex-history-replay-plan\replay_output\AvevaMarineSample\parquet\1112`.
  - Command exited with code `1`.
  - JSON response:
    - `classification=patch_only_empty_baseline`
    - `ready_for_publish=false`
    - `instances_rows=0`
    - `geo_instances_rows=0`
  - Logs:
    - `target\codex-history-baseline-gate\validate-empty-replay.stdout.json`
    - `target\codex-history-baseline-gate\validate-empty-replay.stderr.txt`
- CLI positive validation:
  - Package:
    `output\AvevaMarineSample\parquet\1112_phase2_component_diff_fixture`.
  - Command exited with code `0`.
  - JSON response:
    - `classification=complete_visual_release_candidate`
    - `ready_for_publish=true`
    - `instances_rows=106`
    - `geo_instances_rows=163`
    - `transforms_rows=131`
    - `aabb_rows=105`
    - `scene_tree.tree_file_exists=false`
    - `scene_tree.required=false`
  - Logs:
    - `target\codex-history-baseline-gate\validate-nonempty-fixture.stdout.json`
    - `target\codex-history-baseline-gate\validate-nonempty-fixture.stderr.txt`
- Edge validation:
  - Current mutable path
    `output\AvevaMarineSample\parquet\1112` exits `1` with
    `classification=unsafe_current_output`.
  - The same non-empty fixture with `--require-scene-tree` exits `1` with
    `classification=missing_scene_tree_baseline`, proving scene_tree can be a
    hard gate when required.
  - Empty replay with `--allow-patch-only` exits `0` while still reporting
    `ready_for_publish=false`.
- Publish-history regression validation:
  - Empty replay still fails before creating DuckLake/release package files:
    only stdout/stderr log files exist under
    `target\codex-history-baseline-gate\publish-empty`.
  - Non-empty fixture publishes successfully into temporary catalog
    `target\codex-history-baseline-gate\publish-nonempty-20260619-215501`:
    - `status=created`
    - `instances_rows=106`
    - `geo_instances_rows=163`
    - `component_count=106`

Self review:

- This slice does not solve baseline restoration itself, but it makes the
  baseline boundary executable and automatable. A real operator can now check a
  staged replay package before attempting publish, and CI/job orchestration can
  branch on `classification`.
- The default gate is intentionally aligned with the current proven viewer
  contract: non-empty `instances` and `geo_instances` are required, while
  scene_tree can be explicitly required when the workflow depends on it.
- The next production blocker remains actual baseline hydrate/restore for DB
  `1112`, followed by publishing a second real session-derived release instead
  of a controlled fixture.

## DB1112 Baseline Hydrate Discovery And Command Plan - 2026-06-19

- Used the Oracle skill and Oracle MCP for the requested continued architecture
  analysis.
- Completed Oracle MCP session: `e3d-model-version-architectu-3`.
- Transcript:
  `C:\Users\dpc\.oracle\sessions\e3d-model-version-architectu-3\artifacts\transcript.md`.
- Oracle verdict:
  - The layered architecture remains the best option.
  - DuckLake should be the model-version catalog/index/diff/audit layer, not the
    first implementation's model-generation writer or GLB binary store.
  - The missing P0 piece is a complete historical baseline hydrate before
    applying `from_sesno -> to_sesno`.
  - `init-project` is not a PE/ATT baseline hydrate, and `incremental-sesno` is
    patch-on-existing-state.
- Source inspection:
  - `run_init_project_mode` generates DESI scene-tree/db-meta artifacts and
    refreshes transforms, but assumes PE data already exists.
  - Full sync can save PE/ATT rows when built with the default `review` feature
    set, which includes `surreal-save`.
  - `manual_db_nums` is the safer baseline config mechanism for DB `1112`
    because it still allows system/catalogue dependencies; `included_db_files`
    can accidentally exclude required system DBs.
  - Current full-sync reads the source DB file's current visible state. It does
    not reconstruct an arbitrary historical `from_sesno` state from a newer
    file.
- Implemented command-plan hardening:
  - `ModelHistoryReplayPrepareRequest` now accepts `baseline_release_id`,
    `baseline_dbnums`, and `baseline_config_arg`.
  - `prepare-history-replay` now writes both a baseline config and a replay
    config.
  - JSON output includes `baseline_release_id`, `target_parent_release_id`,
    `baseline_dbnums`, `baseline_config_arg`, `baseline_config_path`, and
    `baseline_plan_warning`.
  - `commands` now includes `baseline_parse`, `baseline_generate`,
    `baseline_register`, `generate`, and `publish`, plus process-safe argv
    arrays for all five stages.
  - Safety checks now include
    `baseline_parse_uses_current_file_state=true`,
    `baseline_target_sesno_reconstruction_supported=false`, and
    `baseline_source_must_already_match_from_sesno=true`.
- DB `1112` validation command:
  - Ran `target\debug\aios-database.exe -c db_options/DbOption model-version
    prepare-history-replay --release-id codex-baseline-plan-1112 --dbnum 1112
    --baseline-dbnum 1112,7997,5052,5054,5100,5101,251047,8191
    --source-db-file D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001
    --from-sesno 896 --to-sesno 897 --force --json`.
  - Output created:
    `target\codex-history-baseline-plan\codex-baseline-plan-1112-baseline.toml`
    and `target\codex-history-baseline-plan\codex-baseline-plan-1112.toml`.
  - JSON warning correctly states that `baseline_parse` uses visible/current
    file state and does not reconstruct `from_sesno=896`.
  - Baseline dbnums were normalized to
    `[1112, 5052, 5054, 5100, 5101, 7997, 8191, 251047]`.
- Build validation:
  - `cargo build --bin aios-database --features model-version-ducklake` passed.
  - Warnings were from upstream `pdms-io` / `parse_pdms_db`, not this slice.
- Documentation updated:
  - `docs/plans/2026-06-19-e3d-model-version-production-architecture-dev-plan.md`
  - `docs/plans/2026-06-19-model-version-ducklake-architecture-plan.md`
  - `docs/plans/2026-06-19-e3d-incremental-site-model-generation.md`

Self review:

- This slice should not be misread as proof of a real DB `1112` sesno `896`
  baseline. It is a corrected command architecture and safety contract.
- The next production implementation must add or integrate one of:
  physical source snapshot at `from_sesno`, restore from an already published
  baseline package/namespace, or a real pdms-io target-sesno hydrate provider.

## Target-Sesno Baseline Hydrate - 2026-06-19

- Updated `GOAL.md` with the active implementation slice:
  `Target-Sesno Baseline Hydrate`.
- SigMap note:
  - `sigmap ask "pdms io specify version sesno full database state hydrate baseline historical replay DB1112"`
    timed out after about 94s.
  - Continued with `rg` and direct source inspection, consistent with the
    project search fallback rule.
- pdms-io source findings:
  - `search_latest_refno(refno, Some(sesno))` selects the requested session's
    `index_root_pageno`, so pdms-io can locate a single refno at a historical
    session.
  - `get_element_at_session(refno, sesno)` wraps this single-refno lookup.
  - `collect_refno_locs(sesno)` and `collect_refno_locs_in_session` are not
    full visible-state enumeration helpers; they filter by page ranges between
    the previous and current session and therefore represent session changes.
  - `build_index_map_verbose` performs a breadth-first full traversal of the
    latest session index root. The same traversal shape can be adapted to a
    specified session root to enumerate all visible refnos at `target_sesno`.
- Current design decision:
  - Implement the target-sesno baseline entrypoint in this repository first,
    using public pdms-io methods (`get_ses_data`, `read_index_data`,
    `parse_element`) and a local index-tree traversal. This avoids changing the
    external pdms-io fork until the behavior is proven on DB `1112`.
  - Start with a CLI-visible read/inspect contract, then wire persistence into
    SurrealDB only after DB `1112` target-session enumeration and parse coverage
    are verified.

Self review:

- This is progress toward the real goal because it replaces the previous
  implicit baseline assumption with a concrete target-session enumeration
  strategy.
- It is not yet a completed baseline hydrate because SurrealDB persistence and
  model generation from that target-session state are still pending.

## Target-Sesno Baseline Inspect Evidence - 2026-06-19

- Implemented a read-only diagnostic entrypoint:
  `aios-database model-version inspect-history-baseline`.
- Files changed:
  - `src/version_management/history_baseline.rs`
  - `src/version_management/cli.rs`
  - `src/version_management/mod.rs`
- The command opens a specific E3D/PDMS DB file, resolves the requested sesno,
  traverses the session's index root using public pdms-io APIs, samples visible
  element parsing, and emits JSON without writing SurrealDB, DuckLake, Parquet,
  or scene_tree state.
- Error handling:
  - missing source DB file fails before pdms-io open;
  - missing exact session fails unless `--allow-nearest-sesno` is explicit;
  - empty `index_root_pageno` fails;
  - non-root child index read failures are recorded in `index_errors` and make
    `full_state_enumeration_supported=false`;
  - parse failures are sampled in `parse_errors` instead of hiding coverage
    gaps.
- Fixed the initial inspect implementation to match pdms-io's public
  `build_index_map_verbose` traversal shape: the session `index_root_pageno` is
  treated directly as an `IndexPageData` root. The unused `RootIndexPage`
  parsing attempt was removed because it added a false diagnostic warning.
- DB `1112` validation command:
  - `target\debug\aios-database.exe -c db_options/DbOption model-version
    inspect-history-baseline --source-db-file
    D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001
    --target-sesno 896 --parse-sample-limit 50 --json`
  - Exit code: `0`.
  - JSON evidence:
    - `requested_sesno=896`
    - `resolved_sesno=896`
    - `latest_sesno=897`
    - `exact_sesno_found=true`
    - `visible_refno_count=5`
    - `index_error_count=1`
    - `parsed_sample_count=3`
    - `parse_error_count=2`
    - `sample_noun_counts={"STRU":1,"ZONE":2}`
    - `full_state_enumeration_supported=false`
    - `persistence_performed=false`
    - recommended action:
      `target_sesno_index_not_publishable; index traversal is incomplete...`
- Latest-session comparison:
  - Running the same command for sesno `897` produced the same small visible
    count and index child-page error shape, so the current public pdms-io
    session-root/index traversal is not proven as a complete visible-state
    provider for this DB file.
- Cross-check attempt:
  - Running pdms-io's own `test_index_map` bin against the same DB file did not
    complete within the 240s command budget, so it was not used as evidence.
- Build validation:
  - `rustfmt --edition 2024 src\version_management\history_baseline.rs` passed.
  - `cargo check --bin aios-database --features model-version-ducklake` passed
    with only existing upstream `pdms-io` / `parse_pdms_db` warnings.
  - `cargo build --bin aios-database --features model-version-ducklake` passed
    with the same upstream warnings.
- Documentation updated:
  - `docs/plans/2026-06-19-e3d-model-version-production-architecture-dev-plan.md`
  - `docs/plans/2026-06-19-model-version-ducklake-architecture-plan.md`
  - `docs/plans/2026-06-19-e3d-incremental-site-model-generation.md`
  - `.planning/2026-06-17-ducklake-valv-version-diff/GOAL.md`

Self review:

- This creates the explicit unsupported-state contract required by the current
  slice: DB `1112` sesno `896` can be resolved, but current target-session
  index-only enumeration is incomplete and must not be treated as a historical
  visual baseline.
- DuckLake remains the correct release catalog/index/diff/audit layer; it
  should version immutable model packages and derived indexes, not compensate
  for a missing generation baseline.
- The next implementation step is not to publish this diagnostic result. The
  next valid implementation is one of: physical baseline snapshot workflow,
  baseline package/namespace restore, or a pdms-io full-state hydrate provider
  proven to produce non-empty `instances` and `geo_instances` for DB `1112`.

## Physical Baseline Snapshot Preparation - 2026-06-19

- Used Oracle MCP session `e3d-model-version-architectu-3` as the requested
  second-model analysis source.
- Oracle conclusion reaffirmed:
  - DuckLake remains the release catalog/index/diff/audit layer.
  - SurrealDB remains the generation writer.
  - Immutable Parquet/GLB packages remain the model release payload.
  - The missing P0 piece is a complete baseline source before running
    historical increments.
- SigMap note:
  - `sigmap ask "physical baseline snapshot namespace sanitation version_management CLI"`
    timed out after about 64s.
  - Continued with direct source inspection and CLI validation.
- Implemented a physical-source baseline bridge:
  - `src/version_management/physical_baseline_snapshot.rs`
  - `src/version_management/cli.rs`
  - `src/version_management/types.rs`
  - `src/version_management/mod.rs`
- New command:
  `aios-database model-version prepare-physical-baseline-snapshot`.
- Behavior:
  - validates snapshot id and source DB header dbnum;
  - finds the active DB file for the requested dbnum in the current project
    `*000` DB directory;
  - creates an isolated project snapshot under a caller-provided or default
    snapshot root;
  - hard-links the source DB directory into the snapshot, with copy fallback;
  - replaces the active target DB file inside the snapshot with the physical
    historical DB file;
  - writes a derived DbOption TOML with isolated `project_path`,
    `output_root`, and `surreal_ns`;
  - sets `total_sync=true`, `save_db=true`, `gen_model=false`,
    `gen_mesh=false`, and `export_parquet_after_gen=false`;
  - removes derived paths that could point back to the current workspace:
    `model_cache_dir`, `transform_parquet_dir`,
    `transform_ducklake_metadata`, and `transform_ducklake_data_path`;
  - emits command strings and argv arrays for the baseline parse and the next
    history-replay planning step.
- Safety hardening added after review:
  - default Surreal namespace now sanitizes the snapshot id like
    `prepare-history-replay` does;
  - generated snapshot namespace must differ from the current namespace;
  - generated config path must differ from the base config path;
  - snapshot output root must differ from current output root;
  - snapshot project directory must differ from the source project directory;
  - existing snapshot/config paths require explicit `--force`.
- DB `1112` candidate scan result:
  - no physical file with latest sesno exactly `896` was found;
  - usable physical history candidates exist, including top-level
    `ams1112_0001 copy` with latest sesno `791`;
  - exact sesno `896` exists only in later files whose target-session index
    inspection remains incomplete and therefore non-publishable as a full
    historical baseline.
- DB `1112` validation:
  - Candidate source:
    `D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams1112_0001 copy`.
  - `inspect-history-baseline --target-sesno 999999 --allow-nearest-sesno`
    resolved latest sesno `791` and header dbnum `1112`.
  - Snapshot command:
    `prepare-physical-baseline-snapshot --snapshot-id codex-ams1112-physical-791
    --dbnum 1112 --baseline-dbnum
    1112,7997,5052,5054,5100,5101,251047,8191 --source-db-file
    "D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams1112_0001 copy"
    --snapshot-root target\codex-physical-baseline\ams1112-791
    --config-out
    target\codex-physical-baseline\ams1112-791\DbOption-physical-baseline
    --output-root target\codex-physical-baseline\ams1112-791\output
    --surreal-ns codex_baseline_ams1112_791 --force --json`.
  - JSON result:
    - `file_count=448`
    - `hardlinked_count=448`
    - `copied_count=0`
    - `replaced_target=true`
    - `original_project_not_modified=true`
    - `surreal_ns=codex_baseline_ams1112_791`
    - baseline dbnums normalized to
      `[1112, 5052, 5054, 5100, 5101, 7997, 8191, 251047]`.
  - Snapshot replacement file
    `target\codex-physical-baseline\ams1112-791\project_path\AvevaMarineSample\ams000\ams1112_0001`
    resolves exact sesno `791`.
  - Original active file
    `D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001`
    still resolves latest sesno `897`, proving the source project was not
    replaced in place.
  - Generated config contains isolated `project_path`, isolated `output_root`,
    `surreal_ns=codex_baseline_ams1112_791`, `total_sync=true`,
    `incr_sync=false`, `sync_history=false`, `save_db=true`,
    `gen_model=false`, `gen_mesh=false`, and
    `export_parquet_after_gen=false`.
  - Default namespace smoke without `--surreal-ns` produced
    `1516_baseline_codex_ams1112_physical_791_sanitize_check`, proving
    snapshot ids with `-` are sanitized before being used as namespace
    fragments.
- Build validation:
  - `rustfmt --edition 2024 src\version_management\physical_baseline_snapshot.rs
    src\version_management\cli.rs src\version_management\types.rs
    src\version_management\mod.rs` passed.
  - `cargo check --bin aios-database --features model-version-ducklake` passed.
  - `cargo build --bin aios-database --features model-version-ducklake` passed.
  - Warnings were existing upstream `pdms-io` / `parse_pdms_db` warnings.
  - `cargo test` was not run, per repository rule.

Self review:

- This is the first safe, executable path for a physical historical baseline
  source. It still does not prove that sesno `896` can be reconstructed from
  the current DB file.
- The chosen 791 source is useful for version-pair testing with another
  physical file or a later incremental range from a matching baseline. It is
  not a substitute for DB `1112` `896` unless a true 896 physical snapshot or
  target-sesno hydrate provider is found.
- Next production step: run the generated baseline parse command against the
  snapshot config, generate/export a non-empty baseline model package, then
  pair it with a later physical or incremental state for the two-pane 3D
  comparison.

## 2026-06-20 DB1112 physical baseline generation/export validation

What changed:

- Fixed `src/fast_model/gen_model/pdms_inst.rs` relation writes:
  - `inst_relate` now deletes and reinserts by stable relation id in the same
    transaction group.
  - `inst_relate_aabb` uses the same replace-on-write behavior.
  - SQL-file output uses the same semantics as online SurrealDB writes.
- Fixed `src/main.rs` plain CLI flow:
  - `--regen-model --export-parquet-after-gen` now calls the shared
    post-generation Parquet helper after generation.
  - If no other export is requested, the command exits after post-gen export
    instead of falling through to default `run_app`.
  - `--export-parquet-after-gen` without a generation request fails fast.

Validation evidence:

- Rebuilt with:
  - `cargo check --bin aios-database --features model-version-ducklake`
  - `cargo build --bin aios-database --features model-version-ducklake`
  - Warnings are existing upstream `pdms-io` / `parse_pdms_db` warnings.
  - `cargo test` was not run, per repository rule.
- The original failing writer case was reproduced from the failed SQL dump:
  `inst_relate:[17496,254370,0]` previously tried to change `out` from
  `inst_info:17496_254370_766` to `inst_info:14658783752023738325`.
- After the fix, DB query in namespace `codex_baseline_ams1112_791` returns:
  - `inst_relate:[17496,254370,0]`
  - `in=pe:17496_254370`
  - `out=inst_info:14658783752023738325`
  - `owner_refno=pe:17496_254334`
  - `owner_type=FRMW`
- Model table counts after generation:
  - `pe_transform=149330`
  - `inst_relate=31044`
  - `inst_info=30632`
  - `inst_geo=1894`
  - `inst_relate_aabb=30484`
- Explicit isolated DB1112 Parquet export succeeded:
  - command:
    `target\debug\aios-database.exe -c target\codex-physical-baseline\ams1112-791\DbOption-physical-baseline --export-parquet --dbnum 1112 --output target\codex-physical-baseline\ams1112-791\validation-export-fixed --verbose`
  - output:
    `target\codex-physical-baseline\ams1112-791\validation-export-fixed\1112`
  - `instances=47698`
  - `geo_instances=31292`
  - `transforms=30495`
  - `aabb=28372`
  - `tubings=56`
  - `ptsets=6999`
  - elapsed `145.25s`
- Fast-fail guard validation:
  - `target\debug\aios-database.exe -c target\codex-physical-baseline\ams1112-791\DbOption-physical-baseline --export-parquet-after-gen --dbnum 1112`
  - exits with error:
    `--export-parquet-after-gen 需要与 --regen-model 或调试/导出模型生成请求一起使用`.

Remaining risks:

- Export manifest reports `missing_geo_hashes=24` and
  `missing_owner_refnos=42`; the 791 baseline package is non-empty but not yet
  visually publishable until these meshes are generated/materialized or
  explicitly classified.
- Some perf/diagnostic paths still use default `output\AvevaMarineSample`
  rather than the isolated replay output root.
- A full `--regen-model --export-parquet-after-gen` run exceeded the 15-minute
  shell timeout during export. The corrected branch was exercised because it
  entered explicit Parquet export, but production should run full replay as a
  managed job with progress and timeout controls.
- DB1112 sesno `791` is a valid physical historical baseline for pipeline
  validation. It is not a true `896` baseline; no physical DB1112 file with
  latest sesno exactly `896` was found.

## Oracle MCP follow-up and missing-mesh gate validation - 2026-06-20

Oracle MCP usage:

- Retrieved completed Oracle MCP sessions:
  - `e3d-model-version-ducklake-review`
  - `e3d-ducklake-version-plan`
  - `e3d-model-version-architectu-3`
- A new focused Oracle MCP consult for the DB1112 791 missing-mesh evidence was
  attempted with slug `e3d-baseline-mesh-version-plan`.
- The dry run succeeded with the intended context bundle.
- The browser run failed because ChatGPT showed a Cloudflare challenge. No new
  Oracle answer was produced, so the plan uses the completed Oracle sessions
  plus local validation evidence.

Architecture document update:

- Added
  `docs/plans/2026-06-20-e3d-model-version-mesh-baseline-architecture-dev-plan.md`.
- Updated:
  - `docs/plans/2026-06-19-model-version-ducklake-architecture-plan.md`
  - `docs/plans/2026-06-19-e3d-incremental-site-model-generation.md`
  - `.planning/2026-06-17-ducklake-valv-version-diff/GOAL.md`

Missing-mesh validation command:

```powershell
$env:AIOS_QUIET_CONFIG='1'
target\debug\aios-database.exe -c target\codex-physical-baseline\ams1112-791\DbOption-physical-baseline model-version validate-history-replay `
  --dbnum 1112 `
  --source-db-file "D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams1112_0001 copy" `
  --from-sesno 790 `
  --to-sesno 791 `
  --parquet-dir target\codex-physical-baseline\ams1112-791\validation-export-fixed\1112 `
  --current-parquet-dir output\AvevaMarineSample\parquet\1112 `
  --scene-tree-dir target\codex-physical-baseline\ams1112-791\output\AvevaMarineSample\scene_tree `
  --require-scene-tree `
  --json
```

Result:

- Exit code: `1`, as expected for an unpublishable visual release.
- JSON classification:
  - `classification=missing_mesh_assets`
  - `ready_for_publish=false`
  - `instances_rows=47698`
  - `geo_instances_rows=31292`
  - `transforms_rows=30495`
  - `aabb_rows=28372`
  - `missing_mesh_geo_hashes=24`
  - `missing_mesh_owner_refnos=42`
  - `mesh_assets_complete=false`
  - scene tree and `db_meta_info.json` evidence are present under the isolated
    output root.

Publish negative-gate command:

```powershell
$env:AIOS_QUIET_CONFIG='1'
target\debug\aios-database.exe -c target\codex-physical-baseline\ams1112-791\DbOption-physical-baseline model-version publish-history `
  --release-id codex-ams1112-physical-791-missing-mesh-gate `
  --dbnum 1112 `
  --source-db-file "D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams1112_0001 copy" `
  --from-sesno 790 `
  --to-sesno 791 `
  --parquet-dir target\codex-physical-baseline\ams1112-791\validation-export-fixed\1112 `
  --current-parquet-dir output\AvevaMarineSample\parquet\1112 `
  --json
```

Result:

- Exit code: `1`.
- Error message refuses publishing with
  `missing_geo_hashes=24 missing_owner_refnos=42`.
- `model-version list --json` contains no
  `codex-ams1112-physical-791-missing-mesh-gate` entry, confirming the gate
  fires before DuckLake release registration.

Decision:

- Missing mesh assets are a hard release gate for normal visual releases.
- DuckLake should store validation/generation evidence and release asset
  indexes, not GLB bodies.
- File-mode mesh generation may keep GLB payloads on disk, but it must persist
  success/failure status or a trusted sidecar so exporters can distinguish
  missing assets from classified bad/non-visual geometry.

## Missing Mesh Repair, Quarantine Export, And Publish Evidence - 2026-06-20

Code changes:

- Added `src/version_management/missing_mesh_repair.rs`.
- Added `model-version repair-missing-meshes`.
- Added `gen_inst_meshes_by_geo_ids_with_state(..., persist_state=true)` so
  targeted GLB regeneration can persist `inst_geo` mesh state even when
  file-mode mesh state is active.
- Extended Parquet `manifest.json.mesh_validation` with raw/render/quarantine
  evidence and dropped row counts.
- Extended `validate-history-replay` with
  `classification=quarantined_visual_release_candidate`.

Build validation:

```powershell
cargo build --bin aios-database --features model-version-ducklake
```

Result:

- Exit code `0`.
- Existing upstream `pdms_io` warnings remain.
- No `cargo test` was run.

Repair dry run:

```powershell
$env:AIOS_QUIET_CONFIG='1'
target\debug\aios-database.exe -c target\codex-physical-baseline\ams1112-791\DbOption-physical-baseline model-version repair-missing-meshes `
  --dbnum 1112 `
  --report-file target\codex-physical-baseline\ams1112-791\validation-export-fixed\1112\missing_mesh_report_1112.json `
  --limit 3 `
  --dry-run `
  --json
```

Result:

- `requested_hashes=24`
- `limited=true`
- first 3 rows are `dry_run_eligible`

Repair execution:

```powershell
$env:AIOS_QUIET_CONFIG='1'
target\debug\aios-database.exe -c target\codex-physical-baseline\ams1112-791\DbOption-physical-baseline model-version repair-missing-meshes `
  --dbnum 1112 `
  --report-file target\codex-physical-baseline\ams1112-791\validation-export-fixed\1112\missing_mesh_report_1112.json `
  --json
```

Result:

- `requested_hashes=24`
- `bad_skipped=3`
- `attempted_hashes=21`
- `generated_hashes=2`
- `still_missing_hashes=22`
- remaining rows are classified as `generation_failed_bad` or `bad_skipped`.

Quarantine export:

```powershell
$env:AIOS_QUIET_CONFIG='1'
$env:AIOS_PARQUET_DROP_MISSING_MESH_ROWS='1'
target\debug\aios-database.exe -c target\codex-physical-baseline\ams1112-791\DbOption-physical-baseline --export-parquet `
  --dbnum 1112 `
  --output target\codex-physical-baseline\ams1112-791\validation-export-repaired-quarantine `
  --verbose
```

Result:

- output:
  `target\codex-physical-baseline\ams1112-791\validation-export-repaired-quarantine\1112`
- `instances=29545`
- `geo_instances=31252`
- `transforms=30495`
- `aabb=28372`
- `tubings=56`
- `ptsets=6897`
- manifest mesh evidence:
  - `policy=quarantine_missing_mesh_rows`
  - `raw_missing_geo_hashes=22`
  - `raw_missing_owner_refnos=40`
  - `render_missing_geo_hashes=0`
  - `render_missing_owner_refnos=0`
  - `quarantined_geo_hashes=22`
  - `quarantined_owner_refnos=40`

Replay validation:

```powershell
$env:AIOS_QUIET_CONFIG='1'
target\debug\aios-database.exe -c target\codex-physical-baseline\ams1112-791\DbOption-physical-baseline model-version validate-history-replay `
  --dbnum 1112 `
  --source-db-file "D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams1112_0001 copy" `
  --from-sesno 790 `
  --to-sesno 791 `
  --parquet-dir target\codex-physical-baseline\ams1112-791\validation-export-repaired-quarantine\1112 `
  --current-parquet-dir output\AvevaMarineSample\parquet\1112 `
  --scene-tree-dir target\codex-physical-baseline\ams1112-791\output\AvevaMarineSample\scene_tree `
  --require-scene-tree `
  --json
```

Result:

- `classification=quarantined_visual_release_candidate`
- `ready_for_publish=true`
- `mesh_assets_complete=true`
- `raw_missing_mesh_geo_hashes=22`
- `quarantined_mesh_geo_hashes=22`
- scene tree evidence present.

Published release:

```powershell
$env:AIOS_QUIET_CONFIG='1'
target\debug\aios-database.exe -c target\codex-physical-baseline\ams1112-791\DbOption-physical-baseline model-version publish-history `
  --release-id codex-ams1112-physical-791-quarantine `
  --release-label "DB1112 physical sesno 791 quarantined visual baseline" `
  --dbnum 1112 `
  --source-db-file "D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams1112_0001 copy" `
  --from-sesno 790 `
  --to-sesno 791 `
  --parquet-dir target\codex-physical-baseline\ams1112-791\validation-export-repaired-quarantine\1112 `
  --materialize-assets `
  --json
```

Result:

- release status: `created`
- `release_id=codex-ams1112-physical-791-quarantine`
- `package_hash=2526d5f18fb1346672383ce7612d4784b2db04d40c3cbb4bb97c5ac685193ee3`
- component index:
  - `component_count=29545`
  - `distinct_component_hashes=29545`
- mesh asset index:
  - `geo_hash_count=1192`
  - `present_count=1192`
  - `missing_count=0`
  - `builtin_count=3`

Follow-up CLI checks:

```powershell
target\debug\aios-database.exe -c target\codex-physical-baseline\ams1112-791\DbOption-physical-baseline model-version list --project AvevaMarineSample --json
target\debug\aios-database.exe -c target\codex-physical-baseline\ams1112-791\DbOption-physical-baseline model-version mesh-assets --release-id codex-ams1112-physical-791-quarantine --missing-only --json
target\debug\aios-database.exe -c target\codex-physical-baseline\ams1112-791\DbOption-physical-baseline model-version diff --from-release-id codex-ams1112-physical-791-quarantine --to-release-id codex-ams1112-physical-791-quarantine --json
```

Results:

- `model-version list` returns the published test release.
- `mesh-assets --missing-only` returns `missing_count=0` and no rows.
- same-release diff returns:
  - `added=0`
  - `deleted=0`
  - `changed=0`
  - `unchanged=29545`

Target-sesno hydrate discovery:

```powershell
target\debug\aios-database.exe -c target\codex-physical-baseline\ams1112-791\DbOption-physical-baseline model-version inspect-history-baseline `
  --source-db-file "D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams1112_0001 copy" `
  --target-sesno 790 `
  --parse-sample-limit 20 `
  --json

target\debug\aios-database.exe -c target\codex-physical-baseline\ams1112-791\DbOption-physical-baseline model-version inspect-history-baseline `
  --source-db-file "D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams1112_0001 copy" `
  --target-sesno 791 `
  --parse-sample-limit 20 `
  --json
```

Result for both `790` and `791`:

- exact session resolved.
- `visible_refno_count=5`
- `index_error_count=1`
- `parse_error_count=2`
- `full_state_enumeration_supported=false`
- recommended action:
  `target_sesno_index_not_publishable; index traversal is incomplete`.

Decision:

- `codex-ams1112-physical-791-quarantine` is a valid quarantined visual
  baseline release for backend/package/viewer integration testing.
- It is not proof that pdms-io target-sesno full-state hydrate is solved.
- A true second session-derived release still requires a proven full-state
  hydrate provider, a second physical baseline source, or a restoreable
  baseline namespace/package strategy.

## Oracle MCP Follow-Up And Second Release Exploration - 2026-06-20

Oracle MCP evidence:

- `mcp__oracle.sessions` was used to re-read completed sessions
  `e3d-model-version-architectu-3` and `e3d-ducklake-version-plan`.
- Both sessions agree that DuckLake is the correct catalog/index/audit layer
  for this version, but not the generator writer or GLB/Parquet body store.
- Both sessions identify baseline hydrate/restore, publish atomicity, and
  release-local assets as the production-critical path.

DB1112 897 physical snapshot attempt:

- Active source file:
  `D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001`.
- Latest session resolved as `897`.
- `prepare-physical-baseline-snapshot` created
  `target\codex-physical-baseline\ams1112-897` with isolated namespace
  `codex_baseline_ams1112_897`.
- Snapshot result:
  - `file_count=448`
  - `hardlinked_count=448`
  - `copied_count=0`
  - `original_project_not_modified=true`
- Full parse/generation was started with the isolated DbOption but did not
  complete within the practical validation window. The process was stopped
  after progress evidence remained near early file reads while CPU continued
  accumulating.

Decision:

- The 897 physical snapshot is a valid source candidate.
- The current full snapshot parse path needs bounded progress, timeout,
  checkpoint, and resume diagnostics before it can be used as a reliable
  production release job.
- This attempt does not count as a second full visual release.

Fallback catalog-chain validation:

- Registered existing current DB1112 output as a deliberately partial release
  in the same DuckLake catalog:
  `codex-ams1112-current-897-partial`.
- Release rows:
  - `instances=106`
  - `geo_instances=163`
  - `transforms=131`
  - `aabb=105`
  - `ptsets=237`
- Asset index:
  - `geo_hash_count=6`
  - `present_count=6`
  - `missing_count=0`
  - `builtin_count=1`
- Package hash:
  `2528ac85a3bdb6093bcaab9c894f64a63234c6c17f23ca219e62b6dc0185f81d`.

Shared catalog validation:

```powershell
$env:AIOS_QUIET_CONFIG='1'
$meta='target\codex-physical-baseline\ams1112-791\output\AvevaMarineSample\model_versions\metadata.ducklake'
$data='target\codex-physical-baseline\ams1112-791\output\AvevaMarineSample\model_versions\data'
target\debug\aios-database.exe -c target\codex-physical-baseline\ams1112-791\DbOption-physical-baseline model-version list --ducklake-metadata $meta --ducklake-data $data --project AvevaMarineSample --json
target\debug\aios-database.exe -c target\codex-physical-baseline\ams1112-791\DbOption-physical-baseline model-version mesh-assets --release-id codex-ams1112-current-897-partial --ducklake-metadata $meta --ducklake-data $data --missing-only --json
```

Results:

- Catalog contains:
  - `codex-ams1112-physical-791-quarantine`
  - `codex-ams1112-current-897-partial`
- `mesh-assets --missing-only` for the partial release returns
  `missing_count=0` and no asset rows.

Cross-release diff validation:

```powershell
$env:AIOS_QUIET_CONFIG='1'
$meta='target\codex-physical-baseline\ams1112-791\output\AvevaMarineSample\model_versions\metadata.ducklake'
$data='target\codex-physical-baseline\ams1112-791\output\AvevaMarineSample\model_versions\data'
$json = target\debug\aios-database.exe -c target\codex-physical-baseline\ams1112-791\DbOption-physical-baseline model-version diff --from-release-id codex-ams1112-physical-791-quarantine --to-release-id codex-ams1112-current-897-partial --ducklake-metadata $meta --ducklake-data $data --json
($json | ConvertFrom-Json).summary | ConvertTo-Json -Depth 5
```

Result:

- `added=106`
- `deleted=29545`
- `changed=0`
- `unchanged=0`
- `total_old=29545`
- `total_new=106`
- `emitted=200`

Review:

- This proves two-release DuckLake catalog, immutable package registration,
  asset indexing, and diff query mechanics.
- This does not prove real incremental model correctness because the target
  release is a partial current output snapshot.
- The documented architecture and development plan has been updated in
  `docs/plans/2026-06-20-e3d-model-version-mesh-baseline-architecture-dev-plan.md`
  with the final DuckLake boundary, version vocabulary, second-release
  decision, and phased development plan.

## Web Startup And Two-Pane Viewer Validation - 2026-06-20

Oracle MCP follow-up:

- Re-read completed Oracle MCP browser sessions:
  - `e3d-model-version-architectu-3`
  - `e3d-ducklake-version-plan`
  - `e3d-model-version-ducklake-review`
- Prepared a focused `mcp__oracle.consult` dry run against current files:
  - 11 files
  - about 109k tokens
  - not executed live because the completed sessions already converge on the
    same decision.
- Consolidated decision:
  - DuckLake is the catalog/index/audit layer.
  - DuckLake is not the generation writer or GLB/Parquet body store.
  - A `sesno` range is source evidence, not a publishable model version unless
    a full baseline state exists before applying the increment.

Code changes:

- `src/web_server/mod.rs`
  - Moved startup scene-tree initialization behind a background task.
  - Reason: a full scene-tree build can block the HTTP listener, which made
    model-version read APIs and static pages unavailable during validation.
- `src/web_api/model_version_api.rs`
  - Release viewer now exposes `window.__MODEL_VERSION_VIEWER` for browser
    diagnostics.
  - Viewer refits after GLB load using xeokit `viewer.scene.aabb`.
  - Viewer attempts high-contrast model emphasis at `scene.models` level.

Build validation:

```powershell
$env:CARGO_TARGET_DIR='target\codex-web-validate-build'
cargo build --bin web_server --features model-version-ducklake
```

Result:

- Build passes.
- Warnings are existing upstream `pdms_io` / `parse_pdms_db` warnings.
- Default `target\debug\web_server.exe` was locked by existing user services on
  ports 18082 and 18083, so validation uses `target\codex-web-validate-build`.

Runtime validation:

- Validation config:
  `target\codex-physical-baseline\ams1112-791\DbOption-web-validate.toml`
- Effective settings:
  - `port=3910`
  - `auto_start_surreal=false`
  - shared DuckLake catalog under
    `target\codex-physical-baseline\ams1112-791\output\AvevaMarineSample\model_versions`
- Current validation server PID:
  `60668`
- Compare URL:
  `http://127.0.0.1:3910/model-version/compare?from=codex-ams1112-physical-791-quarantine&to=codex-ams1112-current-897-partial`

HTTP evidence:

- `GET /api/model-version/releases?project=AvevaMarineSample`
  returns two releases:
  - `codex-ams1112-physical-791-quarantine`
  - `codex-ams1112-current-897-partial`
- `GET /api/model-version/releases/codex-ams1112-physical-791-quarantine/runtime-scene?project=AvevaMarineSample&limit=2000`
  returns:
  - `component_count=2000`
  - `geometry_count=2090`
  - `truncated=true`
  - release-local mesh base URL
- `GET /api/model-version/releases/codex-ams1112-current-897-partial/runtime-scene?project=AvevaMarineSample&limit=500`
  returns:
  - `component_count=106`
  - `geometry_count=163`
  - `truncated=false`
  - release-local mesh base URL
- A release-local GLB HEAD request returns HTTP 200.
- `GET /api/model-version/diff?...`
  returns:
  - `added=106`
  - `deleted=29545`
  - `changed=0`
  - `unchanged=0`

Browser evidence:

- Agent-browser screenshot:
  `.planning\2026-06-17-ducklake-valv-version-diff\screenshot-1781897684788.png`
- Iframe state:
  - left pane loaded `2090/2090`, failed `0`;
  - right pane loaded `163/163`, failed `0`;
  - xeokit scene/model state is non-empty in both panes.

Known remaining UI risk:

- The compare page now proves two-pane release loading, asset loading, and diff
  table mechanics, but the screenshot still does not show a clearly
  distinguishable plant model body. The model surfaces appear mostly white/low
  contrast, with HUD and navigation cube visible.
- Do not mark the user-facing "two visible 3D model comparison" requirement as
  complete until the viewer renders the model body clearly. Candidate fixes:
  - repair xeokit GLB material/edge/color rendering for release package GLBs;
  - add a Three.js fallback viewer for release-local GLB instances;
  - convert/package release assets as XKT if the production viewer is XKT-first;
  - add camera sync and component highlight once visible rendering is reliable.

## Viewer Proxy Geometry Fix - 2026-06-20

Goal for this slice:

- Convert the two-pane page from a data-loading proof into a user-visible 3D
  comparison proof.
- Keep using the same runtime-scene API and release-local GLB assets; do not
  fake the backend state.

Implementation:

- `src/web_api/model_version_api.rs`
  - release viewer now imports xeokit `Mesh`, `ReadableGeometry`,
    `PhongMaterial`, and `buildBoxGeometry`;
  - builds high-contrast translucent AABB proxy boxes from each component's
    runtime-scene AABB;
  - caps proxy boxes at `1200` per pane to keep the large 791 page responsive;
  - still loads all GLB geometries from release-local asset URLs;
  - exposes browser diagnostics:
    - `data-loaded-geometries`
    - `data-expected-geometries`
    - `data-failed-geometries`
    - `data-proxy-geometries`.

Build validation:

```powershell
$env:CARGO_TARGET_DIR='target\codex-web-validate-build'
cargo build --bin web_server --features model-version-ducklake
```

Result:

- Build passes.
- Existing upstream `pdms_io` / `parse_pdms_db` warnings remain.

Runtime validation:

- Restarted web_server on port `3910`.
- Current validation PID after this slice:
  `71436`
- Compare URL:
  `http://127.0.0.1:3910/model-version/compare?from=codex-ams1112-physical-791-quarantine&to=codex-ams1112-current-897-partial&v=proxy-viewer`
- Screenshot:
  `.planning\2026-06-17-ducklake-valv-version-diff\screenshot-1781898139466.png`

Browser diagnostics:

- Left pane:
  - loaded geometries: `2090/2090`
  - failed geometries: `0`
  - proxy geometries: `1200`
- Right pane:
  - loaded geometries: `163/163`
  - failed geometries: `0`
  - proxy geometries: `106`

Review:

- The two panes now show visible 3D geometry and a diff table in the same
  browser page.
- The proxy geometry is derived from real release runtime-scene component
  AABBs, so it is valid spatial evidence for the loaded release packages.
- This does not fully replace production mesh rendering. The GLBs load with
  zero failures, but GLB material/edge rendering still needs hardening or a
  richer production viewer integration before final production sign-off.

## Oracle MCP Architecture Plan Refresh - 2026-06-20

Goal for this slice:

- Use Oracle MCP again against the current source and plan context.
- Decide the best architecture for model data versioning and DuckLake.
- Write a clear Chinese architecture/development plan before the next
  implementation slice.

Oracle MCP:

- Read local `oracle` skill instructions and ran `npx -y @steipete/oracle --help`.
- `sigmap ask` for the latest architecture topic timed out after about 69s, so
  scoping continued with `rg`, mcp sigmap signatures, source reads, and Oracle
  file attachments.
- Dry run with 26 files was too large at about 324k tokens.
- Reduced dry run to 13 files, about 146k tokens.
- Live browser consult completed:
  `e3d-model-version-architectu-20260620`.
- Transcript:
  `C:\Users\dpc\.oracle\sessions\e3d-model-version-architectu-20260620\artifacts\transcript.md`.
- Usage:
  - input tokens about 146k;
  - output tokens about 5.1k.

Oracle conclusion:

- Current direction is correct but still only a release-chain validation version,
  not a production model version system.
- Keep SurrealDB as mutable generation workspace.
- Keep immutable Parquet/GLB package as release payload truth.
- Keep DuckLake as catalog, manifest, component/unit index, diff/impact, and
  audit query layer.
- Do not use DuckLake as generation writer or GLB/Parquet body store.
- Add release status/source manifest/baseline state/generation job/asset hash
  fields before treating releases as production.
- Split DuckLake access into writer and readonly paths.
- Published runtime scenes must not fall back to current/global meshes.

Documentation:

- Added Chinese execution plan:
  `docs/plans/2026-06-20-e3d-incremental-model-version-ducklake-oracle-plan.md`.
- Updated architecture companion link in:
  `docs/plans/2026-06-20-e3d-model-version-mesh-baseline-architecture-dev-plan.md`.
- Updated current goal state in:
  `.planning/2026-06-17-ducklake-valv-version-diff/GOAL.md`.

CLI/build validation:

```powershell
$env:CARGO_TARGET_DIR='target\codex-cli-validate-build'
cargo build --bin aios-database --features model-version-ducklake
```

Result:

- Build passed.
- Existing upstream `pdms_io` / `parse_pdms_db` warnings remain.
- No `cargo test` was run.

`prepare-history-replay --json` safety validation for DB1112 `896 -> 897`:

```text
release_id=codex-ams1112-safety-check-897
baseline_release_id=codex-ams1112-safety-check-896
baseline_config_requests_save_db=true
baseline_binary_supports_surreal_save=true
baseline_target_sesno_reconstruction_supported=false
```

Generated baseline config evidence:

```text
gen_model = false
gen_tree_only = false
manual_db_nums = [1112]
save_db = true
total_sync = true
surreal_ns = "codex_baseline_ams1112_791_history_codex_ams1112_safety_check_897"
```

Review:

- The plan now explicitly separates parse version, baseline state version,
  generated model state, release version, asset version, and diff/index version.
- DuckLake is accepted for this version, but only after state machine and
  readonly/writer boundaries are added.
- The current two-pane proxy viewer remains useful backend evidence, but the
  next production proof must be a real second DB1112 release, not a partial
  current-output fixture.

## Release Publish Status Machine Slice - 2026-06-20

Goal for this slice:

- Begin implementing the Oracle-reviewed publish state machine.
- Prevent half-created or failed releases from being treated as normal readable
  model versions.
- Preserve compatibility with existing DB1112 published test releases.

SigMap:

- `sigmap ask "model version release status publish-history DuckLake model_releases web API list"`
  timed out after about 69s.
- Scoping continued with `rg`, direct source reads, and already-loaded Oracle
  context.

Code changes:

- `src/version_management/types.rs`
  - Added `ModelReleaseStatus`.
  - Added `ModelReleaseRecord.release_status`.
  - Added `ModelReleaseRegisterRequest.initial_status`.
- `src/version_management/ducklake_store.rs`
  - Added `model_releases.release_status`.
  - Added `model_release_status_events`.
  - Added backward-compatible migration/backfill: old rows default to
    `published`.
  - Changed release insert to use an explicit column list.
  - Added `update_release_status`.
  - Default `list_releases` returns only `published` releases.
  - Read paths now reject non-published releases:
    component diff, unit diff, component impact, runtime scene, and mesh asset
    reads.
- `src/version_management/model_release.rs`
  - Normal register inserts new releases as `staged`, indexes components, then
    promotes to requested status.
  - CLI `register` now requests `staged`; publishable visual releases should
    use `publish-history` so asset materialization and publish gates run.
  - `publish-history` creates a staged release, marks validation/materialization
    and indexing states, promotes to `published` only after successful steps,
    and marks `failed` if a post-registration step fails.
- `src/version_management/cli.rs`
  - Register requests now carry `initial_status=Staged`.

Validation:

```powershell
$env:CARGO_TARGET_DIR='target\codex-cli-validate-build'
cargo build --bin aios-database --features model-version-ducklake
```

Result:

- Build passed.
- Existing upstream `pdms_io` / `parse_pdms_db` warnings remain.
- No `cargo test` was run.

Temporary CLI end-to-end status validation:

- Created a temporary package/catalog under:
  `target\codex-status-machine-check\run-20260620-041436`.
- Ran `model-version publish-history --json` against a copied DB1112 package and
  a temporary DuckLake catalog.
- Result:

```text
release_id=codex-status-machine-smoke-041436
registration_status=created
release_status=published
listed_count=1
listed_status=published
component_count=106
safety_non_empty=true
```

Web build validation:

- A first attempt to build `web_server` in a brand-new
  `target\codex-web-status-build` failed because D: had about 1.3 GB free and
  `libduckdb-sys` archive creation hit OS error 112, disk full.
- The temporary target directory was verified to be under the workspace target
  directory and removed.
- The previous validation web server PID `71436` was stopped to release the
  existing `target\codex-web-validate-build` executable.
- After cleanup D: had about 10 GB free.

```powershell
$env:CARGO_TARGET_DIR='target\codex-web-validate-build'
cargo build --bin web_server --features model-version-ducklake
```

Result:

- Build passed.
- Existing upstream warnings remain.

HTTP validation:

- Restarted `web_server` on port `3910` using:
  `target\codex-physical-baseline\ams1112-791\DbOption-web-validate`.
- Current web validation PID:
  `41296`.
- `GET /api/model-version/releases?project=AvevaMarineSample` returns:

```text
release_count=2
statuses=published,published
releases=codex-ams1112-current-897-partial,codex-ams1112-physical-791-quarantine
```

- `GET /api/model-version/diff?...limit=1` still succeeds:

```text
added=106
deleted=29545
changed=0
unchanged=0
emitted=1
```

Formatting and whitespace:

- `cargo fmt --check` passed after manual rustfmt-aligned import/line wrapping.
- `git diff --check` passed for the touched files, with the existing CRLF
  warning for `progress.md`.

Review:

- Existing DB1112 releases are preserved by the migration and now expose
  `release_status=published`.
- New `publish-history` no longer exposes a release as normally readable until
  the publish steps complete.
- This is not the full production publish pipeline yet:
  - DuckLake `open_readonly()` / `open_writer()` split has now been implemented
    and validated in the next slice below.
  - Asset completeness must be made a strict publish-status transition, not
    only a validation convention.
  - The current 897 partial release remains a chain-validation fixture, not a
    true second session-derived release.

## DuckLake Readonly/Writer Split - 2026-06-20

Oracle MCP confirmation:

- Re-read session `e3d-model-version-architectu-20260620` via
  `mcp__oracle.sessions`.
- Oracle's production recommendation remains:
  DuckLake is the catalog/manifest/component/unit/diff/impact/audit layer, not
  the generation writer, not the SurrealDB workspace, and not the GLB/Parquet
  body store.
- Runtime reads for published releases must be immutable and side-effect free.
  GET APIs should not install extensions, create schemas, acquire writer locks,
  or auto-index.

Implemented code changes:

- `src/version_management/ducklake_store.rs`
  - Added writer/read mode separation.
  - `ModelVersionDuckLakeStore::open()` now aliases the writer path for
    backward compatibility.
  - Added `open_writer()`:
    creates parent directories, acquires `MetadataFileLock`, attaches DuckLake
    read-write, and runs schema creation/migration.
  - Added `open_readonly()`:
    requires existing metadata/data paths, does not acquire the writer lock,
    attaches DuckLake with `READ_ONLY`, and only validates the expected read
    schema.
  - Added a fast schema validation error for old catalogs missing
    `release_status`, with remediation pointing to writer commands
    (`register-release`, `publish-history`, `index-release`) rather than
    readonly `list`.
- `src/version_management/model_release.rs`
  - Writer commands now use `open_writer()`.
  - Read commands now use `open_readonly()`:
    list, component diff, unit diff, component impact, mesh-assets read, and
    runtime-scene read.

Validation:

```powershell
cargo fmt --check
$env:CARGO_TARGET_DIR='target\codex-cli-validate-build'
cargo build --bin aios-database --features model-version-ducklake
$env:CARGO_TARGET_DIR='target\codex-web-validate-build'
cargo build --bin web_server --features model-version-ducklake
```

Results:

- `cargo fmt --check` passed.
- CLI build passed.
- Web build passed.
- Existing upstream `pdms_io` / `parse_pdms_db` warnings remain.
- No `cargo test` was run.
- `git diff --check` for touched files passed; the only warning is the
  pre-existing CRLF conversion warning for this progress file.

CLI readonly validation against the shared AvevaMarineSample catalog:

```text
release_count=2
statuses=published,published
ids=codex-ams1112-current-897-partial,codex-ams1112-physical-791-quarantine
```

CLI diff validation:

```text
added=106
deleted=29545
changed=0
unchanged=0
emitted=1
```

Manual writer-lock proof:

- Created
  `target\codex-physical-baseline\ams1112-791\output\AvevaMarineSample\model_versions\metadata.ducklake.lock`
  by hand.
- Ran readonly CLI `model-version list --json` while that lock file existed.
- Result:

```text
lock_present_during_read=True
release_count=2
statuses=published,published
lock_removed=True
```

HTTP validation:

- Restarted isolated validation server from
  `target\codex-web-validate-build\debug\web_server.exe`.
- Current validation PID:
  `70296`.
- Port:
  `127.0.0.1:3910`.
- `GET /api/model-version/releases?project=AvevaMarineSample`:

```text
release_count=2
statuses=published,published
ids=codex-ams1112-current-897-partial,codex-ams1112-physical-791-quarantine
```

- `GET /api/model-version/diff?...limit=1`:

```text
added=106
deleted=29545
changed=0
unchanged=0
emitted=1
```

- Runtime-scene reads return release-local mesh URL patterns:

```text
codex-ams1112-current-897-partial:
mesh_base_url=/files/output/AvevaMarineSample/model_versions/releases/codex-ams1112-current-897-partial/meshes/lod_L1
mesh_url_pattern=/files/output/AvevaMarineSample/model_versions/releases/codex-ams1112-current-897-partial/meshes/lod_L1/{geo_hash}_L1.glb

codex-ams1112-physical-791-quarantine:
mesh_base_url=/files/output/AvevaMarineSample/model_versions/releases/codex-ams1112-physical-791-quarantine/meshes/lod_L1
mesh_url_pattern=/files/output/AvevaMarineSample/model_versions/releases/codex-ams1112-physical-791-quarantine/meshes/lod_L1/{geo_hash}_L1.glb
```

Remaining production blockers:

- Make asset completeness/release-local mesh retention a strict publish gate.
- Remove or explicitly guard any published-runtime fallback to current/global
  meshes.
- Add source manifest, baseline state manifest, generation job id, and asset
  manifest hash to model release records.
- Produce a true second DB1112 release from baseline hydrate/restore or a full
  second physical snapshot; the current 897 partial release is still only a
  chain-validation fixture.

## Release-Local Asset Gate - 2026-06-20

SigMap:

- `sigmap ask "model version release-local mesh asset runtime-scene fallback publish gate"`
  timed out after about 74 seconds, so this slice used direct `rg` and source
  inspection.

Implemented code changes:

- `src/version_management/ducklake_store.rs`
  - `release_scene()` now calls `require_release_mesh_assets_ready()` before
    reading Parquet runtime-scene rows.
  - Published visual releases with `geo_instances > 0` now require:
    - mesh asset index exists;
    - `missing_count == 0`;
    - indexed asset row count matches `geo_hash_count`;
    - non-builtin assets are materialized under release-local
      `meshes/lod_<lod>/`;
    - release-local mesh directory exists.
  - Missing/stale/non-local assets return actionable `missing dependency`
    errors.
- `src/web_api/model_version_api.rs`
  - Removed the runtime-scene fallback to `/files/meshes/lod_<lod>`.
  - If the release-local mesh directory is unavailable, HTTP runtime-scene now
    returns `424 Failed Dependency` with remediation.
- `src/version_management/model_release.rs`
  - `publish-history` now rejects visual packages when
    `materialize_assets=false`.
  - After materialization, `publish-history` rejects `missing_count > 0`
    before the release can become `published`.
- `src/version_management/cli.rs`
  - Direct low-level `model-version register` now creates `staged` releases by
    default instead of bypassing the visual asset gate.
  - The CLI output labels this as a staged registration so callers do not treat
    it as a published, viewer-ready model version.

Validation:

```powershell
cargo fmt --check
$env:CARGO_TARGET_DIR='target\codex-cli-validate-build'
cargo build --bin aios-database --features model-version-ducklake
$env:CARGO_TARGET_DIR='target\codex-web-validate-build'
cargo build --bin web_server --features model-version-ducklake
```

Results:

- All three checks/builds passed.
- Existing upstream `pdms_io` / `parse_pdms_db` warnings remain.
- No `cargo test` was run.
- Web validation server restarted on `127.0.0.1:3910`, PID `26416`.

Positive HTTP runtime-scene validation:

```text
codex-ams1112-current-897-partial:
success=True
component_count=2
geometry_count=2
mesh_base_url=/files/output/AvevaMarineSample/model_versions/releases/codex-ams1112-current-897-partial/meshes/lod_L1

codex-ams1112-physical-791-quarantine:
success=True
component_count=2
geometry_count=0
mesh_base_url=/files/output/AvevaMarineSample/model_versions/releases/codex-ams1112-physical-791-quarantine/meshes/lod_L1
```

Negative HTTP runtime-scene validation:

- Temporarily renamed:
  `target\codex-physical-baseline\ams1112-791\output\AvevaMarineSample\model_versions\releases\codex-ams1112-current-897-partial\meshes\lod_L1`
- Called runtime-scene for that release.
- Restored the directory in `finally`.

Result:

```text
status=424
message=missing dependency: release-local mesh directory is missing for published release 'codex-ams1112-current-897-partial' ...
restored=True
```

This proves published runtime-scene no longer falls back to current/global mesh
assets.

CLI asset index validation:

```text
codex-ams1112-current-897-partial:
asset_geo_hash_count=6
present=6
missing=0
builtin=1
first_relative=meshes/lod_L1/1390347506520517729_L1.glb

codex-ams1112-physical-791-quarantine:
asset_geo_hash_count=1192
present=1192
missing=0
builtin=3
total_bytes=19771568
```

Negative publish-history validation:

- Ran `publish-history --json` into a temporary catalog/release root without
  `--materialize-assets`.
- Result:

```text
exit_code=1
Error: visual historical release 'codex-asset-gate-no-materialize-20260620-044818' references 31252 geo_instances rows but materialize_assets=false
metadata_exists=False
```

Positive publish-history validation:

- Ran `publish-history --materialize-assets --json` into a temporary
  catalog/release root using the existing quarantined 791 package and
  release-local mesh root as source assets.
- Result:

```text
release_id=codex-asset-gate-success-20260620-044900
status=published
component_count=29545
mesh_present=1192
mesh_missing=0
mesh_builtin=3
mesh_geo_hash_count=1192
```

Direct register staged validation:

- Ran low-level `model-version register --json` into a temporary catalog using
  the existing quarantined 791 package.
- Result:

```text
registration_status=created
release_status=staged
component_count=29545
default_list_count=0
```

This keeps direct registration available for diagnostics and orchestration, but
prevents an unmaterialized visual package from appearing in the default
published release list.

HTTP regression:

```text
GET /api/model-version/releases?project=AvevaMarineSample
release_count=2
statuses=published,published
ids=codex-ams1112-current-897-partial,codex-ams1112-physical-791-quarantine

GET /api/model-version/diff?...limit=1
added=106
deleted=29545
changed=0
unchanged=0
emitted=1

GET /api/model-version/releases/codex-ams1112-current-897-partial/runtime-scene?project=AvevaMarineSample&limit=2
component_count=2
geometry_count=2
mesh_base_url=/files/output/AvevaMarineSample/model_versions/releases/codex-ams1112-current-897-partial/meshes/lod_L1

Temporarily hiding release-local meshes:
status=424
restored=True
```

Review:

- The production-critical fallback path for published runtime-scene is now
  closed.
- The publish-history path now refuses visual releases unless assets are
  materialized and complete.
- Manual `index-assets` still supports non-materialized/global URLs for
  diagnostics or legacy workflows, but runtime-scene will not accept them for a
  published visual release.
- Direct low-level `register` now stages releases, so the normal published read
  APIs no longer expose a package that skipped asset materialization.
- Remaining blockers:
  - release metadata still needs source manifest, baseline state manifest,
    generation job id, and asset manifest hash fields;
  - DB1112 still needs a true second release from baseline hydrate/restore or a
    full second physical snapshot;
  - browser two-pane validation should be rerun after the next UI-facing slice.

## Release Provenance Manifest Fields - 2026-06-20

SigMap:

- `sigmap ask "model release provenance source manifest baseline state generation job asset manifest hash version_management"`
  timed out after about 49 seconds, so this slice used direct `rg` and source
  inspection.
- CodeGraph MCP tools were not available in this thread.

Implemented code changes:

- `src/version_management/types.rs`
  - `ModelReleaseRecord` now includes explicit provenance fields:
    `source_manifest_path`, `source_manifest_hash`,
    `baseline_state_manifest_path`, `baseline_state_manifest_hash`,
    `generation_job_id`, `asset_manifest_path`, and `asset_manifest_hash`.
- `src/version_management/ducklake_store.rs`
  - `model_releases` schema adds the provenance columns with compatible
    writer migrations.
  - Readonly open now requires the provenance columns before serving read APIs.
  - `register_release` backfills missing provenance fields for idempotent
    existing releases without overwriting existing values.
  - `index_release_mesh_assets` now writes `asset_manifest_path` and
    `asset_manifest_hash` back to the release record after replacing the asset
    index.
- `src/version_management/model_release.rs`
  - Register hashes the source package `manifest.json` and stores it on the
    release record.
  - Optional baseline-state manifest path/hash metadata is validated; a hash
    mismatch fails before catalog creation.
  - `publish-history` writes an explicit `generation_job_id` into its metadata
    when the caller did not provide one.

Validation:

```powershell
cargo fmt
$env:CARGO_TARGET_DIR='target\codex-cli-validate-build'
cargo build --bin aios-database --features model-version-ducklake
$env:CARGO_TARGET_DIR='target\codex-web-validate-build'
cargo build --bin web_server --features model-version-ducklake
```

Results:

- CLI and web builds passed with only existing upstream `pdms_io` /
  `parse_pdms_db` warnings.
- No `cargo test` was run.
- Temporary provenance registration smoke:

```text
status=created
release_status=staged
default_list_count=0
source_manifest_hash_present=True
baseline_state_manifest_hash_present=True
generation_job_id=codex-smoke-20260620-050944
asset_manifest_hash_is_null=True
```

- Negative baseline manifest validation:

```text
exit_code=1
metadata_exists=False
error=baseline state manifest hash mismatch ...

missing-path exit_code=1
missing-path metadata_exists=False
missing-path error=baseline state manifest path is not a file ...
```

- Shared AvevaMarineSample validation catalog was migrated/backfilled through
  writer commands. Both published DB1112 validation releases now have:

```text
source_manifest_hash_present=True
asset_manifest_hash_present=True
generation_job_id present
asset_missing=0
```

- Web validation server restarted on `127.0.0.1:3910`, PID `46376`.
- HTTP release list:

```text
release_count=2
statuses=published,published
source_manifest_hashes_present=2
asset_manifest_hashes_present=2
generation_job_ids=codex-existing-backfill-current-897,codex-existing-backfill-physical-791
```

- HTTP release detail for `codex-ams1112-current-897-partial` exposes both
  `source_manifest_hash` and `asset_manifest_hash`.
- HTTP runtime-scene still succeeds with release-local mesh URL:

```text
component_count=1
geometry_count=1
mesh_base_url=/files/output/AvevaMarineSample/model_versions/releases/codex-ams1112-current-897-partial/meshes/lod_L1
```

- HTTP diff regression still matches prior evidence:

```text
added=106
deleted=29545
changed=0
unchanged=0
emitted=1
```

Review:

- Release list/detail now surface the provenance evidence needed by operators
  instead of burying it only in `extra_metadata_json`.
- Existing releases can be backfilled safely by rerunning writer commands.
- Baseline state manifest fields are optional because the current validation
  releases still do not come from a true target-sesno hydrate pipeline.
- Remaining blockers:
  - produce a true second DB1112 release from baseline hydrate/restore or a
    full second physical snapshot;
  - attach a real baseline state manifest to that release pair;
  - rerun browser two-pane validation after the true second release is ready.

## Physical Baseline State Manifest - 2026-06-20

Implemented code changes:

- `src/version_management/types.rs`
  - Added `ModelPhysicalBaselineStateManifest`.
  - `ModelPhysicalBaselineSnapshotResponse` now returns
    `baseline_state_manifest_path` and `baseline_state_manifest_hash`.
- `src/version_management/physical_baseline_snapshot.rs`
  - `prepare_physical_baseline_snapshot` now writes
    `baseline_state_manifest.json` under the snapshot root.
  - The manifest records:
    - manifest version;
    - snapshot id, project, dbnum, baseline dbnums;
    - source DB path/hash;
    - replacement DB path/hash;
    - source DB type and session-page evidence;
    - snapshot/config/output/surreal paths;
    - copy/link counts and safety checks.
  - The manifest is written atomically and then hashed.
- `src/version_management/cli.rs`
  - Non-JSON `prepare-physical-baseline-snapshot` output now prints the
    baseline state manifest path and SHA-256.

Validation:

```powershell
cargo fmt
$env:CARGO_TARGET_DIR='target\codex-cli-validate-build'
cargo build --bin aios-database --features model-version-ducklake
$env:CARGO_TARGET_DIR='target\codex-web-validate-build'
cargo build --bin web_server --features model-version-ducklake
```

Results:

- CLI and web builds passed with only existing upstream warnings.
- No `cargo test` was run.
- Real physical baseline snapshot smoke used:
  `D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams1112_0001`
- Output:

```text
snapshot_id=codex-baseline-manifest-20260620-052344
baseline_state_manifest_path=target\codex-baseline-manifest-check\codex-baseline-manifest-20260620-052344\baseline_state_manifest.json
baseline_state_manifest_hash=29372c887b997481fb27ad77391d73cc40fc86336d921c8dafd7525daf4eec68
manifest_version=physical_baseline_state_manifest:v1
source_db_sha256_present=True
replacement_db_sha256_matches=True
file_count=448
hardlinked_count=448
copied_count=0
original_project_not_modified=True
```

- Temporary `publish-history --materialize-assets` with that real baseline
  manifest in metadata published successfully into an isolated catalog:

```text
release_status=published
baseline_hash_matches=True
generation_job_id=codex-baseline-manifest-publish-20260620-052414
mesh_missing=0
mesh_present=1192
component_count=29545
```

- HTTP regression on the shared AvevaMarineSample validation catalog passed
  with isolated web server PID `67844`:

```text
release_count=2
source_manifest_hashes_present=2
asset_manifest_hashes_present=2
scene_success=True
scene_component_count=1
geometry_count=1
diff_added=106
diff_deleted=29545
diff_changed=0
diff_emitted=1
```

Review:

- Physical baseline snapshots now produce a first-class, hashable baseline
  state evidence file that can be attached to release metadata.
- The manifest proves what file state was used as the baseline snapshot; it
  does not by itself prove a target-sesno hydrate from pdms-io history.
- Remaining blockers:
  - produce or obtain a true second DB1112 full release;
  - attach the real baseline manifest to that true release pair;
  - rerun browser two-pane validation on the true pair.

## DB1112 897 Physical Candidate Audit - 2026-06-20

Oracle result reused:

- Reattached completed Oracle browser session
  `e3d-model-version-architectu-20260620`.
- Oracle agrees the best boundary is:
  - DuckLake for release/catalog/index/diff/audit;
  - SurrealDB/cache/work dirs for generation workspace;
  - release-local filesystem/object storage for GLB/Parquet bodies;
  - app-level `release_id` plus manifests for the actual model version.
- Oracle also flagged that the current 897 partial package is a smoke fixture,
  not a real engineering delta.

SigMap:

- `sigmap ask "DB1112 model version DuckLake incremental model generation architecture"`
  timed out again after about 49 seconds. Continued with narrowed CLI and file
  inspection.

DB1112 source audit:

```text
D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams1112_0001
latest_sesno=767
sha256=1529B93C6329AA6719D06A39006DD38EA134F59D3E36D50F22A79F0A1FAF7BF0

D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001
latest_sesno=897
sha256=70F18C70116F392EAE533B75FB8F4043D031A5F049448531CC1DFC43FAF7D3C2
```

Implementation change:

- `ModelPhysicalBaselineStateManifest` now includes
  `source_db_latest_sesno`.
- `ModelPhysicalBaselineSnapshotResponse` now includes
  `source_db_latest_sesno`.
- `prepare-physical-baseline-snapshot` reads it through
  `PdmsIO::get_latest_sesno`.
- Non-JSON CLI output prints `source_db_latest_sesno`.
- `ModelPhysicalBaselineSnapshotCommands` now includes
  `generate_full_model` / `generate_full_model_argv`.
- The generated full-model command is:
  `aios-database -c <snapshot-config> --regen-model --dbnum <dbnum> --export-parquet-after-gen`.

Validation:

```text
cargo fmt --check: passed
cargo build --bin aios-database --features model-version-ducklake: passed
HTTP release list spot-check on 127.0.0.1:3910: passed
published_release_count=2
git diff --check on touched files: passed with existing CRLF warning only
HTTP health after command-chain change: release_count=2 statuses=published,published
```

Created isolated 897 candidate snapshot:

```text
snapshot_id=codex-ams1112-897-candidate-20260620-053630
snapshot_root=target\codex-physical-baseline\ams1112-897-candidate-20260620-053630
baseline_state_manifest_hash=8766d612b70e6aa3e09200b54fb9daa9b7a10545a811d85324ac589fd03d0082
source_db_latest_sesno=897
source_db_sha256=70f18c70116f392eae533b75fb8f4043d031a5f049448531cc1dfc43faf7d3c2
replacement_db_sha256=70f18c70116f392eae533b75fb8f4043d031a5f049448531cc1dfc43faf7d3c2
hashes_match=True
file_count=448
hardlinked_count=448
copied_count=0
original_project_not_modified=True
```

Command-chain validation:

```text
snapshot_id=codex-ams1112-897-command-check-20260620-054350
source_db_latest_sesno=897
manifest_hash=5148883fd0ae477a4f0a1568fa5e38efebed2cab552d00dd5f6fd7cb7e5849ef
parse=aios-database -c target\codex-physical-baseline\ams1112-897-command-check-20260620-054350\DbOption-physical-897
generate_full_model=aios-database -c target\codex-physical-baseline\ams1112-897-command-check-20260620-054350\DbOption-physical-897 --regen-model --dbnum 1112 --export-parquet-after-gen
generate_has_regen_model=True
generate_has_export=True
file_count=448
hardlinked_count=448
original_project_not_modified=True
```

Review:

- The preferred next input for a real second DB1112 full release is now
  `D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001`.
- The current shared `codex-ams1112-current-897-partial` release must remain
  a UI/API fixture only until a full 897 physical release is generated.
- The physical snapshot prepare response now gives the exact parse and full
  generation/export commands, reducing operator handoff risk.
- `run_regen_model` was checked and the generated command aligns with the
  existing backend generation path: it forces mesh generation/boolean work,
  resolves DB1112 SITE roots through `--dbnum`, and exports Parquet through the
  existing post-generation exporter.
- `inspect-history-baseline` still cannot prove target-sesno full-state
  enumeration, so the immediate production path is full physical snapshot
  comparison, not final 896 -> 897 incremental hydrate proof.

## DB1112 897 Physical Parse Attempt - 2026-06-20

Goal:

- Move beyond command generation by running the real 897 physical snapshot
  parse/save_db path against an isolated project copy and isolated Surreal
  namespace.

SigMap:

- `sigmap ask "DB1112 physical 897 snapshot parse save_db Surreal namespace validation"`
  timed out again after about 49 seconds.

Created sanitized 897 snapshot:

```text
snapshot_id=codex-ams1112-897-parse-20260620_054746
snapshot_root=target\codex-physical-baseline\ams1112-897-parse-20260620_054746
source_db_latest_sesno=897
baseline_state_manifest_hash=89d57707270478c3631b4aab5cee69ddb8e7bde3cc37be25b4995d32c84aba22
surreal_ns=codex_baseline_ams1112_897_parse_20260620_054746
config_arg=target\codex-physical-baseline\ams1112-897-parse-20260620_054746\DbOption-physical-897
file_count=448
hardlinked_count=448
original_project_not_modified=True
```

Executed real parse command:

```text
aios-database -c target\codex-physical-baseline\ams1112-897-parse-20260620_054746\DbOption-physical-897
```

Observed evidence:

```text
process_pid=68024
started_at=2026-06-20T05:48:12+08:00
stopped_at=2026-06-20T06:34:39+08:00
cpu_seconds_at_stop=2814.64
stdout_log=target\codex-physical-baseline\ams1112-897-parse-20260620_054746\parse.stdout.log
stderr_log=target\codex-physical-baseline\ams1112-897-parse-20260620_054746\parse.stderr.log
surreal_namespace=codex_baseline_ams1112_897_parse_20260620_054746
db1112_refnos_read=422107
output_file_count=5
output_total_bytes=22045
scene_tree_files_written=True
```

Important stdout evidence:

```text
SurrealDB connected: ws://127.0.0.1:8020
namespace: codex_baseline_ams1112_897_parse_20260620_054746
db_type is DESI
read file ...\ams1112_0001 finished in 34.7717ms
All refnos count: 422107
```

Reason for stopping:

- The process was still CPU-active after about 46 minutes but emitted no
  additional progress after the initial DB read lines.
- It was stopped intentionally because this was an isolated debug-build
  validation run with no progress signal and no bounded completion estimate.
- No `aios-database` process was left running after the stop.

Post-stop validation:

```text
aios_database_processes_after_stop=0
source_db_sha256=70F18C70116F392EAE533B75FB8F4043D031A5F049448531CC1DFC43FAF7D3C2
HTTP release list spot-check: success=True release_count=2 statuses=published,published
```

Review:

- This is real E2E progress: the 897 physical source can be mounted in an
  isolated snapshot, the parser opens it, and DB1112 resolves to 422107 refnos.
- It is not a completed 897 full parse and therefore not ready for model
  generation or publishing.
- The run exposes a production-grade gap: full parse needs either a release
  build validation path, progress counters, resumable/checkpointed parse, or a
  supervised background task with observable status before a developer/operator
  can safely run it as part of the model-version workflow.
- The next implementation slice should add/identify a bounded runner for
  physical full parse/generation and surface progress before attempting
  `commands.generate_full_model`.

## Parse Progress Heartbeat - 2026-06-20

Problem addressed:

- The 897 physical parse run was CPU-active for about 46 minutes but emitted no
  progress after `All refnos count: 422107`.
- Operators need to know which DB file is being parsed, total refnos/chunks,
  current chunk progress, and elapsed time before this can be considered
  production-ready.

Implementation:

- Added stdout heartbeat lines in `src/versioned_db/database.rs` for the
  non-callback full parse path:
  - `[parse-progress] file_start ...`
  - `[parse-progress] db_basic_done ...`
  - `[parse-progress] chunk_done ...`
- The DB1112 path now prints project, file, dbnum, db_type, save_db,
  refno count, chunk count, completed chunks, parsed attrs, and elapsed seconds.

Validation:

```text
sigmap ask "versioned_db full sync parse progress heartbeat sync_pdms database.rs": timed out after about 49s
cargo fmt --check: passed
cargo build --bin aios-database --features model-version-ducklake: passed
```

Heartbeat E2E smoke:

```text
snapshot_id=codex-ams1112-897-heartbeat-20260620_064024
source_db_latest_sesno=897
surreal_ns=codex_baseline_ams1112_897_heartbeat_20260620_064024
baseline_state_manifest_hash=4fbf311603bcfb0a495693d34ba0690d9b41218bf30d3829718cf40eddcacc44
file_count=448
hardlinked_count=448
original_project_not_modified=True
```

Observed heartbeat output:

```text
[parse-progress] file_start project=AvevaMarineSample file=ams5100_0001 dbnum=5100 db_type=DICT save_db=true
[parse-progress] db_basic_done project=AvevaMarineSample file=ams5100_0001 dbnum=5100 refnos=243 chunks=1 db_basic_ms=2
[parse-progress] chunk_done project=AvevaMarineSample file=ams5100_0001 dbnum=5100 completed_chunks=1/1 last_chunk=1 parsed_attrs=225 elapsed_s=0.2
[parse-progress] file_start project=AvevaMarineSample file=ams1112_0001 dbnum=1112 db_type=DESI save_db=true
[parse-progress] db_basic_done project=AvevaMarineSample file=ams1112_0001 dbnum=1112 refnos=422107 chunks=5 db_basic_ms=1211
```

Stop/health validation:

```text
heartbeat_process_pid=70700
progress_line_count=14
stopped_cleanly=True
aios_database_processes_after_stop=0
source_db_sha256=70F18C70116F392EAE533B75FB8F4043D031A5F049448531CC1DFC43FAF7D3C2
HTTP release list: success=True release_count=2 statuses=published,published
```

Review:

- The immediate observability gap is reduced: the next full parse attempt will
  show the current DB and total chunk count before entering expensive chunk
  work.
- This is not yet the complete bounded runner requirement. There is still no
  persisted task status, automatic timeout policy, resume/checkpoint, or web
  task cancellation surface for physical full parse/generation.
- The next slice should either wrap `commands.parse` in the existing web
  task/sidecar infrastructure or add a dedicated CLI runner that writes a
  status JSON with heartbeat timestamps, exit status, and cancellation reason.

## Parse Progress Metrics File - 2026-06-20

Problem addressed:

- Stdout heartbeat is useful for local CLI runs, but web/sidecar supervision
  needs a durable machine-readable status artifact.
- The existing `AIOS_TASK_METRICS_PATH` collector only flushed parse summary
  records after DB completion, so long DB1112 chunk work still looked silent to
  external supervisors.

Implementation:

- Extended `src/perf_metrics.rs`:
  - `ParseStageMetrics.progress: Option<ParseProgressMetrics>`.
  - `ParseProgressMetrics` records stage, project, file, dbnum, db_type,
    save_db, total refnos/chunks, completed chunks, last chunk, parsed attrs,
    elapsed ms, and `updated_at`.
  - `record_parse_progress(ParseProgressUpdate)` updates the same JSON metrics
    file used by existing task instrumentation.
- Updated `src/versioned_db/database.rs` to call `record_parse_progress` when
  emitting each `[parse-progress]` heartbeat:
  - `file_start`
  - `db_basic_done`
  - `chunk_done`

Validation:

```text
sigmap ask "incremental E3D model version DuckLake parse progress metrics versioned_db perf_metrics": timed out after about 64s
cargo fmt --check: passed
cargo build --bin aios-database --features model-version-ducklake --target-dir target\codex-cli-validate-build: passed
```

Real 897 metrics smoke:

```text
snapshot_id=codex-ams1112-897-metrics-20260620_065144
surreal_ns=codex_baseline_ams1112_897_metrics_20260620_065144
baseline_state_manifest_hash=3c9dcb0f4a7be165cc78b9b308335508923ad6d09b2430542cb9c2b73e5393ef
source_db_latest_sesno=897
file_count=448
hardlinked_count=448
original_project_not_modified=True
AIOS_TASK_METRICS_PATH=target\codex-physical-baseline\ams1112-897-metrics-20260620_065144\parse-metrics.json
```

Observed persisted progress:

```text
observed_stage=db_basic_done
observed_dbnum=1112
observed_refnos_total=422107
observed_chunks_total=5
observed_chunks_completed=0
observed_parsed_attrs=0
observed_updated_at=2026-06-20T06:52:21.543669300+08:00
```

Post-run safety/regression:

```text
aios_database_processes_after_stop=0
source_db_sha256=70F18C70116F392EAE533B75FB8F4043D031A5F049448531CC1DFC43FAF7D3C2
CLI release list: 2 published releases
HTTP release list: success=True release_count=2 statuses=published,published
published_release_ids=codex-ams1112-current-897-partial,codex-ams1112-physical-791-quarantine
```

Review:

- The next parse attempt is now observable both by logs and by a durable JSON
  sidecar file.
- This is still a progress instrumentation slice, not proof of full 897 parse
  completion.
- The remaining production gap is the bounded runner: normal-exit capture,
  timeout/cancel policy, final success/failure state, and then full
  `commands.parse` -> `commands.generate_full_model` execution.

## Oracle Follow-Up Review - 2026-06-20

Oracle usage:

```text
oracle --help: completed
large-context dry run: ~250944 tokens, too large
reduced-context dry run: ~86610 tokens, upload still unstable
small-context dry run: ~32435 tokens
browser upload attempts: failed twice because attachments did not finish uploading
successful run: oracle --engine browser --browser-attachments never --model gpt-5.5-pro --slug e3d-version-inline-review
session=e3d-version-inline-review
transcript=C:\Users\dpc\.oracle\sessions\e3d-version-inline-review\artifacts\transcript.md
```

Oracle agreement:

- Keep DuckLake as the controlled release catalog / manifest / index / diff /
  impact / audit layer.
- Keep immutable model data truth in release packages: Parquet, GLB/XKT,
  `manifest.json`, source/baseline/generation/asset manifests, and hashes.
- Do not use DuckLake as the parser target, generation workspace, GLB body
  store, Parquet body store, or user-facing version clock.
- `release_id` plus package/source/baseline/generation/asset hashes is the
  user-facing model-data version.

Oracle design corrections to carry forward:

- Split release lifecycle from release quality. Lifecycle should be staged /
  validating / assets_materialized / indexed / published / failed. Quality
  should carry complete_visual / quarantined_visual / degraded_visual /
  patch_only / non_visual.
- Remove or isolate legacy `None -> Published` status fallback after catalog
  migration.
- Serialize DuckLake writes through a single-writer publish/index queue for
  register, publish, asset indexing, component/unit indexing, failure records,
  and repair/backfill.
- Runtime-scene GET must remain read-only: no repair, no index building, no
  global mesh fallback, and HTTP 424 for missing release-local assets/indexes.
- Treat `codex-ams1112-current-897-partial` and
  `codex-ams1112-physical-791-quarantine` as smoke/UI/catalog validation only,
  not production proof of real DB1112 model delta correctness.

Additional production requirements from Oracle:

- Harden provenance: every publish should expose source DB file, dbnum,
  latest_sesno, source hash, replacement hash, baseline manifest hash, package
  hash, asset manifest hash, generation job id, and validation report hash.
- Metrics/runner should add run id, attempt, pid/process group, argv,
  working dir, env summary, snapshot id, source hash before/after, final status,
  exit code, timeout/cancel reason, heartbeat timestamps, stdout/stderr paths,
  and artifact paths.
- Parse metrics should add heartbeat sequence, DB index/total, refnos processed,
  persist batch/row counters, failed SQL count, stage timestamps, and explicit
  final `parse_done`/`failed`/`cancelled` stages.
- Generation/export needs its own heartbeat before running full DB1112 897
  generation.
- Hardlinked physical snapshots are efficient but must be re-hashed before
  parse/generate/publish, because hardlinks to mutable source trees are an audit
  risk.

Resulting next implementation order:

1. Split release lifecycle and quality in the model/version APIs.
2. Add append-only status/failure/validation evidence for publish.
3. Implement bounded runner with durable task state and process-tree kill.
4. Expand parse metrics and add generation metrics.
5. Rerun 897 physical parse to normal exit.
6. Run `commands.generate_full_model`, validate package/assets, publish a full
   897 physical release.
7. Validate 791 vs 897 through CLI JSON, HTTP runtime-scene, and browser
   two-pane compare with real GLB/XKT geometry.

## Release Lifecycle / Quality Split - 2026-06-20

Problem addressed:

- Oracle pointed out that the old `release_status` mixed operational lifecycle
  and visual/data quality.
- A quarantined or degraded release can be published for debugging or smoke
  comparison, but it must not look equivalent to a complete production visual
  release.

Implementation:

- Added explicit release lifecycle and quality fields:
  - lifecycle: `staged`, `validating`, `assets_materialized`, `indexed`,
    `published`, `failed`.
  - quality: `complete_visual`, `quarantined_visual`, `degraded_visual`,
    `patch_only`, `non_visual`.
- Kept legacy `release_status` for compatibility while writing
  `release_lifecycle` and `release_quality` into DuckLake.
- Added schema migration/backfill for existing catalog rows.
- Updated release registration to infer quality from explicit metadata first,
  then legacy status/labels/derivation/row counts.
- Updated CLI list output to show lifecycle, quality, and legacy status.
- Updated HTTP release list to return the new fields and accept quality filters.

Validation:

```text
sigmap ask "model release status lifecycle quality release_status ModelReleaseStatus DuckLake model_version list APIs": timed out after about 64s
cargo fmt --check: passed
cargo build --bin aios-database --features model-version-ducklake --target-dir target\codex-cli-validate-build: passed
cargo check --bin web_server --features "web_server,model-version-ducklake" --target-dir target\codex-web-validate-build: passed
cargo build --bin web_server --features "web_server,model-version-ducklake" --target-dir target\codex-web-validate-build: passed
git diff --check on touched source files: passed
```

Catalog migration/index validation:

```text
model-version index --release-id codex-ams1112-current-897-partial --json: success, component_count=106
model-version index --release-id codex-ams1112-physical-791-quarantine --json: success, component_count=29545
model-version list --json: release_count=2
codex-ams1112-current-897-partial: lifecycle=published quality=degraded_visual legacy_status=published
codex-ams1112-physical-791-quarantine: lifecycle=published quality=quarantined_visual legacy_status=published
```

HTTP validation against isolated `web_server` on `127.0.0.1:3910`:

```text
GET /api/model-version/releases?project=AvevaMarineSample&dbnum=1112
  -> success=True release_count=2
GET /api/model-version/releases?project=AvevaMarineSample&dbnum=1112&quality=degraded_visual
  -> success=True release_count=1 release_id=codex-ams1112-current-897-partial
GET /api/model-version/releases?project=AvevaMarineSample&dbnum=1112&quality=quarantined_visual
  -> success=True release_count=1 release_id=codex-ams1112-physical-791-quarantine
GET /api/model-version/releases?project=AvevaMarineSample&dbnum=1112&complete_visual_only=true
  -> success=True release_count=0
GET /api/model-version/releases?project=AvevaMarineSample&dbnum=1112&quality=bad
  -> HTTP 400
```

Design note:

- The current API still returns all `published` lifecycle releases by default
  for compatibility with existing smoke workflows, but every row is visibly
  marked with `release_quality` and can be filtered.
- Production UI acceptance should prefer `complete_visual` once a full 897
  physical release exists.

Remaining gaps:

- Remove or isolate the legacy missing-status-to-published fallback after
  catalog migration is mandatory.
- Add single-writer DuckLake queue before concurrent web publish/index jobs.
- Add append-only validation/failure evidence.
- Implement bounded runner and generation/export metrics before rerunning the
  full DB1112 897 path.

## Bounded Runner CLI Slice - 2026-06-20

Problem addressed:

- The real DB1112 897 physical parse previously ran as an ad hoc foreground
  process. It could be stopped, but there was no durable run state, timeout
  record, cancellation reason, stdout/stderr artifact contract, or source hash
  before/after evidence.
- Oracle recommended a bounded runner before another full parse/generate
  attempt.

Architecture decision:

- Implement a CLI-first foreground supervisor under `model-version`.
- The supervisor itself can later be started by `web_server` or the existing
  sidecar, but the process it supervises is always spawned from an argv array,
  never a shell command string.
- State is written to JSON under a run directory so another process can poll or
  cancel the run while the supervisor is still alive.

Implementation:

- Added `src/version_management/bounded_runner.rs`.
- Added `pub mod bounded_runner` in `src/version_management/mod.rs`.
- Added CLI commands in `src/version_management/cli.rs`:
  - `model-version run-command`
  - `model-version run-status`
  - `model-version cancel-run`
- Run state records:
  - run id, kind, status, pid, executable, argv, cwd, env keys;
  - stdout/stderr paths;
  - metrics path snapshot;
  - timeout and stale heartbeat settings;
  - submitted/started/updated/finished timestamps;
  - exit code and error;
  - cancel timestamp/reason;
  - source DB hash before/after and unchanged flag.
- Windows process tree cancellation uses `taskkill /PID <pid> /T`, then
  `taskkill /PID <pid> /T /F` after a short grace period.

Validation:

```text
sigmap ask "bounded runner task sidecar command execution model-version parse generate_full_model status cancel metrics process tree": timed out after about 74s
mcp sigmap impact for src/version_management/cli.rs: impacted src/version_management/mod.rs and src/lib.rs
cargo fmt --check: passed
cargo build --bin aios-database --features model-version-ducklake --target-dir target\codex-cli-validate-build: passed
cargo check --bin web_server --features "web_server,model-version-ducklake" --target-dir target\codex-web-validate-build: passed
HTTP release list health check on 127.0.0.1:3910: success=True complete_visual_only release_count=0
```

CLI success/status validation:

```text
run_id=runner-help-smoke
command=aios-database --help
run-command status=succeeded exit_code=0
run-status status=succeeded exit_code=0
stdout_bytes=12891
stderr_empty=True
```

CLI failure validation:

```text
run_id=runner-list-smoke
command=aios-database -c db_options/DbOption model-version list --json
status=failed exit_code=1
stderr=model-version DuckLake catalog is missing release provenance columns...
```

This failure is expected for the default local catalog and proves stderr capture
and non-zero status persistence.

Timeout validation:

```text
run_id=runner-timeout-smoke
command=C:\WINDOWS\System32\WindowsPowerShell\v1.0\powershell.exe -NoProfile -Command "Start-Sleep -Seconds 10"
timeout_secs=1
status=timed_out
error="command timed out after 1 seconds"
timed_out_child_pid=58048
child_process_after_timeout=not_found
```

Cancel validation:

```text
run_id=runner-cancel-smoke2
before_status=running
before_child_pid=8928
cancel_kill_attempted=True
after_status=cancelled
after_exit_code=1
after_cancel_reason="validation cancel smoke"
child_still_alive=False
```

Source hash and metrics validation:

```text
run_id=runner-hash-metrics-smoke
status=succeeded
source_db_hash_unchanged=True
metrics_exists=True
metrics_stage=fixture_done
metrics_success=True
metrics_updated_at=2026-06-20T07:50:00+08:00
```

Review:

- This closes the durable supervision gap enough to rerun full DB1112 897 parse
  under a bounded wrapper in the next slice.
- It does not yet add a web_server HTTP management surface. The intended next
  integration is a thin web endpoint that starts the same CLI supervisor and
  reads the same run JSON.
- It does not yet add generation/export heartbeat fields. The runner can
  observe a metrics file once the generation path writes one.
- `--force` deletes an existing run directory. Parent process stdout/stderr and
  argv files must not be placed inside the same run directory when using
  `--force`, as Windows file locks can block removal.

## Bounded Runner Command-Plan Compatibility And 897 Parse Smoke - 2026-06-20

Problem addressed:

- `prepare-physical-baseline-snapshot` and `prepare-history-replay` emit
  command-plan argv arrays whose first item is `aios-database`.
- The first bounded runner implementation treated argv as child arguments only.
  That would make a real prepared parse run fail by passing a stray
  `aios-database` argument.

Implementation:

- `BoundedRunRecord` now records both:
  - `argv`: original argv from the command plan;
  - `child_argv`: actual args passed to the spawned child.
- The runner automatically strips a leading executable name when it matches the
  configured executable file name or stem.
- The JSON status includes `argv_included_executable`.
- CLI help now documents that prepared command-plan arrays may include the
  leading executable.

Compatibility validation:

```text
run_id=runner-command-plan-argv-smoke
input_argv=["aios-database","--help"]
status=succeeded
exit_code=0
argv_included_executable=True
child_argv=["--help"]
cargo fmt --check: passed
cargo build --bin aios-database --features model-version-ducklake --target-dir target\codex-cli-validate-build: passed
cargo check --bin web_server --features "web_server,model-version-ducklake" --target-dir target\codex-web-validate-build: passed
```

Real DB1112 897 runner smoke:

```text
snapshot_id=codex-ams1112-897-runner-smoke-20260620_0810
source_db_file=D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001
source_db_sha256=70F18C70116F392EAE533B75FB8F4043D031A5F049448531CC1DFC43FAF7D3C2
source_db_latest_sesno=897
baseline_state_manifest_hash=77e4bf240a935cfb548405b3dd24a3315438650f38b7b253a7e88cb90ff3ee9d
surreal_ns=codex_baseline_ams1112_897_runner_smoke_20260620_0810
```

Runner invocation shape:

```text
run_id=runner-897-parse-smoke-20260620_0810
kind=parse_smoke
argv_file=target\codex-physical-baseline\codex-ams1112-897-runner-smoke-20260620_0810\parse-argv.json
AIOS_TASK_METRICS_PATH=target\codex-physical-baseline\codex-ams1112-897-runner-smoke-20260620_0810\parse-metrics.json
timeout_secs=30
```

Observed result:

```text
status=timed_out
exit_code=1
argv_included_executable=True
child_argv=-c target\codex-physical-baseline\codex-ams1112-897-runner-smoke-20260620_0810\DbOption-physical-897
source_db_hash_unchanged=True
metrics_exists=True
metrics_stage=db_basic_done
metrics_updated_at=2026-06-20T07:58:04.879045900+08:00
aios_database_processes_after_timeout=0
```

Stdout evidence:

```text
[parse-progress] file_start project=AvevaMarineSample file=amssys dbnum=8191 db_type=SYST save_db=true
[parse-progress] chunk_done project=AvevaMarineSample file=amssys dbnum=8191 completed_chunks=1/1 parsed_attrs=1229
[parse-progress] file_start project=AvevaMarineSample file=ams5100_0001 dbnum=5100 db_type=DICT save_db=true
[parse-progress] chunk_done project=AvevaMarineSample file=ams5100_0001 completed_chunks=1/1 parsed_attrs=225
[parse-progress] file_start project=AvevaMarineSample file=ams1112_0001 dbnum=1112 db_type=DESI save_db=true
All refnos count: 422107
[parse-progress] db_basic_done project=AvevaMarineSample file=ams1112_0001 dbnum=1112 refnos=422107 chunks=5
```

Review:

- The runner now consumes the exact argv shape emitted by the snapshot/replay
  command planners.
- `child_argv` and `argv_included_executable` are serde-defaulted so older
  `run.json` files can still be read by `run-status` and `cancel-run`.
- The 30-second smoke is not a full parse completion, but it proves the real
  DB1112 897 parse enters the expensive DB1112 stage under durable supervision.
- The next full parse should use the same run path with an operator-approved
  timeout and should be allowed to finish normally before model generation.

Oracle MCP follow-up:

```text
session=e3d-version-ducklake-architectu-slim
status=error
error=Attachments did not finish uploading before timeout
note=the new slim follow-up did not produce a review; keep using the completed Oracle sessions already recorded in this plan
```

## Generation Metrics First Slice And Failure-Path Runner Smoke - 2026-06-20

Problem addressed:

- The bounded runner could supervise long parse/generation commands, but the
  generation path did not yet write a durable progress signal into the task
  metrics JSON.
- Without that signal, `run-status` could distinguish process state but not
  answer where model generation was currently spending time.

Implementation:

- Added `GenerateProgressMetrics` and `TaskMetrics.generate.progress`.
- Added `record_generate_progress(stage, detail, elapsed_ms)` for lightweight
  heartbeats from CLI and generation internals.
- Added `finish_generate_stage_from_model_store(duration_ms)`, which snapshots
  Surreal-backed model-store counts into the metrics JSON when a generation
  command reaches a terminal point.
- Hooked progress reporting into:
  - `run_generate_model`;
  - `run_regen_model`;
  - direct `incremental-sesno --generate-model`;
  - IndexTree generation internals.
- Main command handling now finalizes task metrics on direct generation,
  incremental generation, and post-generation export errors instead of leaving a
  stale in-progress metrics file.

Observed stages now include:

```text
connect_surreal
collect_transform_refresh_roots
collect_generation_targets
pre_cleanup_for_regen
gen_all_geos_data_started
gen_all_geos_data_finished
gen_all_geos_data_failed
incremental_sesno_generate_started
incremental_sesno_generate_finished
incremental_sesno_generate_failed
index_tree_init
geometry_generation
geometry_generation_done
instance_data_write
batch_barrier_done
boolean_operation
web_bundle_export
sqlite_spatial_index
index_tree_finished
```

Failure-path runner smoke:

```text
run_id=runner-generate-metrics-fail-20260620_082231
state_root=target\codex-generation-metrics-smoke\20260620_082231
command=aios-database -c db_options/DbOption --regen-model --dbnum 0 --export-parquet-after-gen
AIOS_TASK_METRICS_PATH=target\codex-generation-metrics-smoke\20260620_082231\generate-metrics.json
AIOS_TASK_METRICS_KIND=generate
status=failed
exit_code=1
argv_included_executable=True
child_argv=["-c","db_options/DbOption","--regen-model","--dbnum","0","--export-parquet-after-gen"]
metrics_exists=True
metrics_success=False
metrics_stage=collect_transform_refresh_roots
stderr=Error: dbnum=0 下未找到任何 SITE，无法刷新 pe_transform
```

Build validation:

```text
cargo fmt --check: passed
cargo check --bin aios-database --features model-version-ducklake --target-dir target\codex-cli-validate-build: passed
cargo build --bin aios-database --features model-version-ducklake --target-dir target\codex-cli-validate-build: passed
cargo check --bin web_server --features "web_server,model-version-ducklake" --target-dir target\codex-web-validate-build: passed
```

Review:

- This closes the first generation observability gap: a runner-managed generate
  command now leaves a durable failure or progress trail in `generate-metrics`.
- The smoke intentionally used an invalid DB selection to verify error handling
  and metrics finalization. It is not evidence of successful generation.
- Full production acceptance still requires a normal DB1112 897 parse exit,
  followed by supervised `commands.generate_full_model`, release packaging,
  publish/register, diff, and real two-pane visual comparison.

Oracle MCP follow-up:

```text
session=e3d-model-version-ducklake-no
status=error
error=Missing OPENAI_API_KEY

session=e3d-model-version-ducklake-browser
status=completed
attachments=none
elapsed=6m28s
transcript=C:\Users\dpc\.oracle\sessions\e3d-model-version-ducklake-browser\artifacts\transcript.md
```

Oracle synthesis:

- Architecture should remain a staged chain:
  `SourceObservation -> ParseRun -> CanonicalSnapshot -> Diff -> GenerationPlan
  -> immutable ReleasePackage -> rebuildable DuckLake catalog`.
- `latest` must always be resolved to a concrete sesno and source hash before
  it enters manifest/evidence.
- The first production-grade DB1112 path should use full-state diff between
  `sesno=897` and the resolved latest version; native sesno-range delta comes
  later and must prove equivalence against full parse snapshots.
- `release_id` is the user-facing model version. `sesno`, parse run id,
  canonical snapshot id, payload hash, DuckLake snapshot id, and software
  version are separate evidence/version domains.
- DuckLake is useful for release/entity/chunk/diff/audit queries, but it must be
  droppable and rebuildable from immutable release packages.
- The largest risks called out by Oracle are partial E3D writes, treating file
  events as consistency boundaries, path/name-based identity, unstable hash
  inputs, single huge GLB payloads, and letting DuckLake manage the only copy of
  release Parquet files.

## 2026-06-20 - HTTP Runner Management API and Current Architecture Review

Implemented backend operations surface for supervised parse/generation commands:

- Added HTTP runner management routes:
  - `POST /api/model-version/runs`
  - `GET /api/model-version/runs/{run_id}`
  - `POST /api/model-version/runs/{run_id}/cancel`
- The API accepts `project`, `state_dir`, `run_id`, `kind`, `argv`,
  `executable`, `cwd`, optional stdout/stderr/metrics paths, timeout,
  stale-heartbeat settings, source DB hash evidence, and `force`.
- The web API deliberately restricts execution to an `aios-database`
  executable. It rejects arbitrary executables before starting the background
  runner.
- `AIOS_TASK_METRICS_KIND` is injected from `kind`. If `metrics_path` is
  provided, `AIOS_TASK_METRICS_PATH` is injected as well.
- `run_bounded_command` now writes a terminal failed `run.json` when child
  process spawn fails after the initial state file has been created, including
  final source-file hash evidence.
- Non-`model-version-ducklake` builds now expose matching DuckLake stub
  methods for `open_writer`, `open_readonly`, and `update_release_status`.
  This keeps feature-gated web_server builds compiling while still returning a
  clear feature-required error for DuckLake-backed release operations.

HTTP validation through a real web_server process:

```text
build=cargo build --bin web_server --features web_server --target-dir target\codex-web-runner-api-lite-build
port=3921
config=db_options/DbOption-codex-live-view
health=GET /api/version -> 0.3.34
run_id=http-runner-help-20260620-0904
start=POST /api/model-version/runs
command=aios-database --help
start_success=True
launch_observed=True
argv_included_executable=True
child_argv=["--help"]
status=GET /api/model-version/runs/{run_id}
final_status=succeeded
exit_code=0
stdout_head="AIOS Database Processing Tool"
cancel=POST /api/model-version/runs/{run_id}/cancel
cancel_previous_status=succeeded
kill_attempted=False
negative_executable_check=powershell.exe rejected before launch
```

The temporary web_server process and temporary lite build directory were removed
after validation. The small smoke artifacts remain under
`target\codex-http-runner-api-smoke`.

Oracle MCP status:

- Attachment-based Oracle MCP review completed:
  `e3d-model-version-ducklake-current`.
- Transcript:
  `C:\Users\dpc\.oracle\sessions\e3d-model-version-ducklake-current\artifacts\transcript.md`.
- Input tokens: ~79k; elapsed: 5m52s.
- The new review agreed with the immutable-release / rebuildable-DuckLake
  architecture and added several concrete hardening recommendations:
  - HTTP runner should be narrowed from a generic `aios-database argv` endpoint
    into domain-specific pipeline run kinds such as `parse-baseline`,
    `generate-full-model`, `validate-package`, `publish-release`, and
    `index-release`.
  - `state_dir`, `cwd`, stdout/stderr paths, and metrics paths need server-side
    sandboxing under the project output/run root; HTTP callers should not choose
    arbitrary filesystem targets.
  - source evidence should become a source observation manifest covering primary
    DB file plus dependency DB/catalog/spec/material files, not just one
    `source_db_file` hash.
  - runner heartbeat should evolve from metrics-file mtime into stage-aware
    progress with heartbeat sequence, stage budget, checkpoint, and terminal
    metrics completeness.
  - DB1112 `sesno=897` normal full parse/generate/release is the next priority;
    expanding DuckLake tables should not outrun the successful model path.

Current architecture decision after local validation plus Oracle review:

- Keep model version truth as immutable release packages:
  `manifest.json`, immutable Parquet payloads, GLB/chunk assets, hashes, and
  evidence chain.
- Keep SurrealDB as mutable generation workspace and operational state, not as
  release truth.
- Keep DuckLake optional in this version and use it for release/entity/chunk/
  diff/metrics/audit indexes that can be deleted and rebuilt from release
  packages.
- Do not let DuckLake own the only copy of release Parquet payloads. If DuckLake
  registers Parquet files, use derived/copy data that can be regenerated.
- The DB1112 production validation path remains:
  full parse `sesno=897` -> full parse resolved latest -> canonical diff ->
  incremental generation plan -> immutable release packages -> DuckLake index
  rebuild -> two-pane 3D comparison.

## 2026-06-20 - Structured Pipeline Endpoint and Source Observation Evidence

Problem addressed:

- The first HTTP runner endpoint proved durable process supervision, but it
  still accepted an arbitrary `aios-database` argv array from the caller.
- Oracle recommended turning HTTP operations into structured model-version
  pipeline requests, with server-generated paths and a stronger source
  observation boundary before long DB1112 parse/generate jobs.

Implementation:

- Added source observation data contracts:
  - `ModelSourceObservationManifest`
  - `ModelSourceObservationFileEvidence`
  - `ModelSourceObservationQuiescence`
- Added `src/version_management/source_observation.rs`:
  - validates path-safe observation ids;
  - hashes the primary DB file before and after an optional quiescence window;
  - hashes caller-provided dependency files;
  - writes the manifest atomically and returns its SHA-256.
- Added domain-specific HTTP endpoint:
  - `POST /api/model-version/runs/prepare-physical-snapshot`
- The new endpoint builds the bounded-run request server-side:
  - command kind: `prepare_physical_snapshot`;
  - state root: `output/<project>/model_versions/runs`;
  - snapshot root:
    `output/<project>/model_versions/physical_baselines/<snapshot_id>`;
  - source observation root:
    `output/<project>/model_versions/runs/_source_observations/<run_id>`;
  - metrics/stdout/stderr paths under the run directory;
  - physical snapshot DbOption/output paths under the snapshot directory.
- The endpoint rejects unsafe run/snapshot ids, duplicate state without
  `force`, path escapes from controlled roots, non-file source DB paths, and
  non-`aios-database` executables.
- A regression found during negative HTTP validation was fixed:
  executable allowlist validation now happens before source observation
  manifest creation, so rejected requests do not leave partial evidence.

Oracle MCP status:

```text
completed_session=e3d-model-version-ducklake-current
transcript=C:\Users\dpc\.oracle\sessions\e3d-model-version-ducklake-current\artifacts\transcript.md
confirmed=immutable release truth + rebuildable DuckLake catalog/index/diff/audit

followup_session=e3d-model-version-ducklake-followup
status=error
error=Attachments did not finish uploading before timeout
note=the failed follow-up produced no new architecture answer; this slice uses the completed Oracle review plus local HTTP evidence
```

Build and formatting validation:

```text
sigmap ask "model-version bounded runner pipeline source observation manifest HTTP API parse baseline generate full model": timed out after 124s
cargo fmt: passed
cargo check --bin web_server --features "web_server,model-version-ducklake" --target-dir target\codex-web-validate-build: passed
cargo build --bin web_server --features web_server --target-dir target\codex-web-pipeline-api-build: passed
```

Positive HTTP validation:

```text
server=http://127.0.0.1:3922
config=db_options/DbOption-codex-live-view
run_id=http-prepare-physical-1112-20260620-0937
snapshot_id=http-prepare-physical-1112-20260620-0937
source_db_file=D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001
dbnum=1112
endpoint=POST /api/model-version/runs/prepare-physical-snapshot
launch_observed=True
source_observation_manifest_hash=106f1e741665c74add5ad91e2658cb3562a2c236b8a0baaa02e3e366a9d8c821
primary_sha256=70f18c70116f392eae533b75fb8f4043d031a5f049448531cc1dfc43faf7d3c2
primary_bytes=99080192
quiescence_stable=True
quiescence_checks=2
status=succeeded
exit_code=0
elapsed_ms=8472
source_db_hash_unchanged=True
source_db_latest_sesno=897
file_count=448
hardlinked_count=448
copied_count=0
baseline_state_manifest_hash=c9dc2ff8bedb6b8ebd5b75d0a78697ab4f8d2fdd20659b2eef6d20111672cc7d
parse_command_count=1
generate_command_count=1
```

Negative HTTP validation:

```text
run_id=http-prepare-physical-reject-exe-20260620-0935
executable=powershell.exe
result=HTTP 500 error envelope
message="executable is missing or not a file: powershell.exe"
manifest_exists=False

run_id=http-prepare-physical-reject-exe-abs-20260620-0936
executable=C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe
result=HTTP 500 error envelope
message="HTTP model-version runs only allow the aios-database executable, got C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
manifest_exists=False
```

Cleanup:

- The temporary `target\codex-web-pipeline-api-build` web_server process was
  stopped after validation.
- The temporary build and HTTP smoke directories created for this slice were
  removed after path checks.
- Existing unrelated web_server processes were left untouched.

Review:

- This completes the first Oracle-recommended hardening step: HTTP now has a
  structured model-version pipeline operation instead of only a generic argv
  launcher.
- The endpoint is intentionally scoped to physical snapshot preparation. It
  does not yet run the full parse, generation, validation, publish, or compare
  pipeline.
- Dependency source observation is currently explicit/caller-provided. Automatic
  catalogue/system/material dependency discovery remains a production task.
- The final acceptance boundary is unchanged: DB1112 897 full parse, latest full
  parse, model generation, immutable release publication, DuckLake indexing,
  diff, and real two-pane 3D comparison still need to be completed.

## 2026-06-20 - Structured Parse Baseline Endpoint

Problem addressed:

- `prepare-physical-snapshot` produced an isolated DbOption and command plan,
  but the next parse step still required callers/operators to submit or copy an
  argv array.
- The full DB1112 897 parse must eventually run through a server-controlled
  backend endpoint with evidence that the prepared snapshot and source DB file
  are unchanged before parse starts.

Implementation:

- Added domain-specific HTTP endpoint:
  - `POST /api/model-version/runs/parse-baseline`
- Request shape:
  - `project`
  - `run_id`
  - `snapshot_id`
  - optional `dbnum`
  - optional dependency files and quiescence window
  - executable/timeout/stale-heartbeat/poll/force controls
- The endpoint derives all filesystem paths server-side:
  - snapshot root:
    `output/<project>/model_versions/physical_baselines/<snapshot_id>`;
  - baseline state manifest:
    `<snapshot_root>/baseline_state_manifest.json`;
  - run root:
    `output/<project>/model_versions/runs/<run_id>`;
  - source observation manifest:
    `output/<project>/model_versions/runs/_source_observations/<run_id>/source_observation_manifest.json`.
- The endpoint reads `baseline_state_manifest.json` and validates:
  - `snapshot_id`;
  - project;
  - optional dbnum;
  - manifest paths stay under the controlled snapshot root;
  - config file exists;
  - replacement DB file exists.
- It recomputes `baseline_state_manifest_hash` and emits it in the API response
  plus `AIOS_BASELINE_STATE_MANIFEST_SHA256`.
- It builds a new `source_observation_manifest.json` for the snapshot
  replacement DB file and includes the baseline-state manifest as dependency
  evidence.
- It refuses to start if the observed replacement DB hash differs from the
  replacement hash recorded by the baseline-state manifest.
- It launches the bounded runner with only:
  `aios-database -c <snapshot DbOption>`.

Build and formatting validation:

```text
sigmap ask "model-version parse baseline structured HTTP endpoint bounded runner physical snapshot command plan source observation": timed out after 124s
cargo fmt --check: initially reported one rustfmt line-wrap diff
cargo fmt: applied
cargo check --bin web_server --features "web_server,model-version-ducklake" --target-dir target\codex-web-validate-build: passed
cargo build --bin web_server --features web_server --target-dir target\codex-web-parse-api-build: passed
```

Positive HTTP validation through a real web_server process:

```text
server=http://127.0.0.1:3923
config=db_options/DbOption-codex-live-view
endpoint=POST /api/model-version/runs/parse-baseline
run_id=http-parse-baseline-1112-timeout-20260620-0954
snapshot_id=http-prepare-physical-1112-20260620-0937
dbnum=1112
timeout_secs=15
command_argv=["aios-database","-c","output\\AvevaMarineSample\\model_versions\\physical_baselines\\http-prepare-physical-1112-20260620-0937\\DbOption-physical-baseline"]
baseline_state_manifest_hash=c9dc2ff8bedb6b8ebd5b75d0a78697ab4f8d2fdd20659b2eef6d20111672cc7d
source_observation_manifest_hash=77066c1a8911a374d1c2a1daac24edad02ccadb5d49623b315fc3a153d2dd80c
primary_sha256=70f18c70116f392eae533b75fb8f4043d031a5f049448531cc1dfc43faf7d3c2
requested_sesno=physical-snapshot:897
resolved_sesno=897
launch_observed=True
final_status=timed_out
exit_code=1
error="command timed out after 15 seconds"
metrics_exists=True
metrics_stage=stages.parse.progress.stage=db_basic_done
metrics_dbnum=1112
metrics_refnos_total=422107
source_db_hash_unchanged=True
timed_out_pid=53000
child_process_after_timeout=not_found
```

Stdout evidence:

```text
[parse-progress] file_start project=AvevaMarineSample file=ams5100_0001 dbnum=5100 db_type=DICT save_db=true
[parse-progress] chunk_done project=AvevaMarineSample file=ams5100_0001 dbnum=5100 completed_chunks=1/1 parsed_attrs=225
[parse-progress] file_start project=AvevaMarineSample file=amssys dbnum=8191 db_type=SYST save_db=true
[parse-progress] chunk_done project=AvevaMarineSample file=amssys dbnum=8191 completed_chunks=1/1 parsed_attrs=1229
[parse-progress] file_start project=AvevaMarineSample file=ams1112_0001 dbnum=1112 db_type=DESI save_db=true
All refnos count: 422107
[parse-progress] db_basic_done project=AvevaMarineSample file=ams1112_0001 dbnum=1112 refnos=422107 chunks=5
```

Negative HTTP validation:

```text
run_id=http-parse-baseline-reject-exe-20260620-0955
executable=C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe
result=HTTP 500 error envelope
message="HTTP model-version runs only allow the aios-database executable, got C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
source_observation_manifest_exists=False

run_id=http-parse-baseline-missing-snapshot-20260620-0956
snapshot_id=missing-snapshot-for-parse-baseline
result=HTTP 500 error envelope
message="baseline state manifest is missing; prepare the physical snapshot first: output\\AvevaMarineSample\\model_versions\\physical_baselines\\missing-snapshot-for-parse-baseline\\baseline_state_manifest.json"
source_observation_manifest_exists=False
```

Cleanup:

- The temporary `target\codex-web-parse-api-build` web_server process was
  stopped after validation.
- Temporary parse API build/log directories were removed after checking they
  were under the workspace.
- Existing unrelated web_server processes were left untouched.

Review:

- This is still not the final successful DB1112 897 parse. The 15-second
  timeout was intentional to validate the new structured endpoint, runner
  supervision, source evidence, metrics, and process cleanup.
- The endpoint now provides the exact production path needed for the next long
  operator-approved parse run: same command, same source observation, longer
  timeout, and no user-provided argv/config paths.
- The next endpoint should be `generate-full-model`, gated by parse run
  evidence so accidental generation against an unparsed or wrong namespace is
  rejected by default.

## 2026-06-20 Structured Generate Full Model Endpoint

Oracle update:

- Loaded the `oracle` skill and ran `oracle --help`.
- Started Oracle MCP consult `e3d-version-ducklake-review` after a dry-run
  reduced the bundle from roughly 302k tokens to roughly 88k tokens.
- That new consult failed with
  `Attachments did not finish uploading before timeout`.
- Used completed Oracle MCP sessions from earlier on 2026-06-20 as the
  effective second opinion. Their conclusion remained consistent with the
  local design:
  - DuckLake belongs in catalog/index/diff/audit only.
  - Immutable release package plus manifests remain the payload truth.
  - `release_id`, not raw `sesno` or DuckLake snapshot id, is the user-facing
    model version.
  - `generate-full-model` should be gated by successful parse evidence; any
    bypass must be explicitly diagnostic.

SigMap:

```text
sigmap ask "model-version generate full model structured HTTP endpoint parse run evidence bounded runner baseline state manifest"
result: timed out after 124s
```

Implementation:

- Added `POST /api/model-version/runs/generate-full-model`.
- Added `GenerateFullModelRunRequest`.
- Extended `ModelVersionPipelineRunApiData` and `PreparedPipelineRun` with:
  `parse_run_id`, `parse_run_status`, and `diagnostic_reason`.
- Implemented `build_generate_full_model_pipeline_run`.
- Added the route to the route registry in `src/web_api/mod.rs`.
- Updated `classify_error` so executable allowlist violations return HTTP 400
  instead of HTTP 500.

Production gate implemented:

```text
parse_run_id required unless allow_incomplete_parse=true with diagnostic_reason
parse run kind must be parse_baseline
parse run status must be succeeded
parse run source_db_hash_unchanged must be true
parse run source DB path must match baseline replacement DB file
parse run source hash before/after must match baseline replacement hash
snapshot replacement DB is re-observed before generation
observed replacement hash must match baseline_state_manifest.replacement_db_sha256
```

Generated command:

```text
aios-database -c <snapshot DbOption> --regen-model --dbnum <dbnum> --export-parquet-after-gen
```

Build and formatting validation:

```text
cargo fmt --check: passed
cargo check --bin web_server --features "web_server,model-version-ducklake" --target-dir target\codex-web-generate-api-build: passed
cargo build --bin web_server --features "web_server,model-version-ducklake" --target-dir target\codex-web-generate-api-build: passed
```

HTTP validation through a real web_server process:

```text
server=http://127.0.0.1:3924
config=db_options/DbOption-codex-live-view
binary=target\codex-web-generate-api-build\debug\web_server.exe
pid=53552 first run, pid=51300 after status-code fix
```

Negative validation: missing parse run:

```text
run_id=http-generate-full-missing-parse-1112-20260620-1019
status=400
success=false
message="parse_run_id is required unless allow_incomplete_parse=true with a diagnostic_reason"
source_observation_manifest_exists=false
```

Negative validation: timed-out parse run:

```text
run_id=http-generate-full-timeout-parse-gate-1112-20260620-1019
parse_run_id=http-parse-baseline-1112-timeout-20260620-0954
status=424
success=false
message="missing dependency: parse_run_id 'http-parse-baseline-1112-timeout-20260620-0954' must have status succeeded before generate-full-model, got TimedOut"
source_observation_manifest_exists=false
```

Diagnostic smoke validation:

```text
run_id=http-generate-full-diagnostic-1112-20260620-1019
allow_incomplete_parse=true
diagnostic_reason="short HTTP smoke: endpoint and bounded runner validation before successful full parse"
status=200
success=true
launch_observed=true
source_observation_manifest_exists=true
final_status=failed
error="command exited with status exit code: 1"
source_db_hash_unchanged=true
pid=66168
child_process_after_terminal=not_found
metrics_stage=collect_transform_refresh_roots
```

Negative validation: executable allowlist:

```text
run_id=http-generate-full-bad-exe-1112-20260620-1021
executable=C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe
status=400
success=false
message="HTTP model-version runs only allow the aios-database executable, got C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
source_observation_manifest_exists=false
```

Cleanup:

- The temporary `target\codex-web-generate-api-build` web_server process was
  stopped after validation.
- Existing unrelated web_server processes were left untouched.

Review:

- This endpoint is deliberately not a release publisher. It starts a bounded
  full generation command only after source, baseline, and parse evidence are
  coherent.
- The diagnostic mode is useful for HTTP/router/supervisor smoke checks only.
  It is not a production model generation success path.
- The next hard requirement remains a full DB1112 897 `parse-baseline` normal
  success, followed by production-mode `generate-full-model` using that parse
  run id.

## 2026-06-20 Long Parse Baseline Attempt: Heartbeat Gap Found

Purpose:

- Move beyond short endpoint smoke and test whether DB1112 897
  `parse-baseline` can progress toward a normal successful parse.

Run:

```text
server=http://127.0.0.1:3925
run_id=http-parse-baseline-1112-full-20260620-1030
snapshot_id=http-prepare-physical-1112-20260620-0937
timeout_secs=1800
stale_heartbeat_secs=600
source_observation_manifest_hash=9ba02c1bc256a02edb5d0ec73f775b98f9e87a5f63ff2f366dcbd79748e01f95
baseline_state_manifest_hash=c9dc2ff8bedb6b8ebd5b75d0a78697ab4f8d2fdd20659b2eef6d20111672cc7d
```

Observed:

```text
status=running
pid=61740
metrics_stage=db_basic_done
stdout showed DB1112 entered multi-thread parse/write after:
  All refnos count: 422107
  [parse-progress] db_basic_done ... dbnum=1112 refnos=422107 chunks=5
process CPU increased from ~125s to ~446s
working_set grew from ~257MB to ~331MB
metrics_updated_at did not advance after db_basic_done
```

Decision:

- Cancelled the run before the 600-second stale heartbeat watchdog could
  falsely classify an active parse as stale.
- Cancellation evidence:

```text
final_status=cancelled
cancel_reason=stale-heartbeat-risk-before-metrics-fix
pid_alive=false
source_db_hash_unchanged=true
metrics_stage=db_basic_done
```

Review:

- The parser itself appeared active, not deadlocked.
- The production gap is observability: long DB1112 parse/write work after
  `db_basic_done` does not update task metrics frequently enough for bounded
  stale-heartbeat supervision.
- Next action is to add parse progress heartbeat updates around file/chunk
  parse stages, then rerun `parse-baseline` with bounded supervision.

## 2026-06-20 Parse Baseline Heartbeat Fix And Long DB1112 Run

Purpose:

- Fix the backend observability failure that made a still-active DB1112 parse
  look stale to the bounded runner.
- Re-run DB1112 897 against a clean physical snapshot until it can produce a
  successful `parse_baseline` run record for the production `generate-full-model`
  gate.

Implementation:

- Added a parse progress heartbeat around chunk parse/write work in
  `src/versioned_db/database.rs`.
- The first attempt used a Tokio task, but real DB1112 validation showed that
  the parser path blocks the async runtime while `parse_file_with_chunk` is
  working. The metrics file still sat at `db_basic_done`.
- Replaced that heartbeat with an OS thread plus stop/join guard, so heartbeat
  writes continue even while the parser is doing blocking CPU/DB work.
- Added `AIOS_SYNC_CHUNK_SIZE` as an operational override for parse chunk size.

Validation evidence:

```text
server=http://127.0.0.1:3925
aios-database=E:\codex-targets\plant-cli-validate-build\debug\aios-database.exe

snapshot_id=http-prepare-physical-1112-smallchunk-long-20260620-1113
snapshot_status=succeeded
snapshot_elapsed_ms=8486
baseline_state_manifest_hash=a15de8ff2efa6945cbfba7a03b689842319df89fa1c8622f757784bf8b89f4ab
replacement_db_sha256=70f18c70116f392eae533b75fb8f4043d031a5f049448531cc1dfc43faf7d3c2
source_db_latest_sesno=897
surreal_ns=1516_baseline_http_prepare_physical_1112_smallchunk_long_20260620_1113

run_id=http-parse-baseline-1112-smallchunk-long-20260620-1113
kind=parse_baseline
timeout_secs=7200
stale_heartbeat_secs=600
AIOS_SYNC_CHUNK_SIZE=5000
status=succeeded
exit_code=0
elapsed_ms=2923152
observed_progress=85/85 chunks
observed_stage=chunk_done
observed_parsed_attrs=422074
source_db_hash_unchanged=true
metrics_success=true
```

Review:

- The heartbeat fix is now proven against the real blocking DB1112 path: metrics
  advance through `chunk_pending` and `chunk_done` instead of staying at
  `db_basic_done`.
- The long run reached a normal `succeeded` terminal state and confirmed
  `source_db_hash_unchanged=true`.
- This is the first successful production-shaped DB1112 897 parse-baseline
  evidence for the new pipeline. `generate-full-model` can now be run in
  production mode with `parse_run_id=http-parse-baseline-1112-smallchunk-long-20260620-1113`.
- Follow-up code review added a `Drop` fallback to the heartbeat guard so a
  future panic/early unwind cannot leave the OS heartbeat thread running.

Oracle follow-up:

- Reattached completed Oracle session `e3d-model-version-ducklake-current`.
- The second-model recommendation matches the local architecture decision:
  DuckLake should remain the rebuildable catalog/index/diff/audit layer, not
  the source of truth for model payload versions.
- Immutable release packages remain the truth boundary: manifest, Parquet,
  release-local GLB/XKT/mesh assets, validation report, source/baseline
  evidence, generation job evidence, asset manifest hash, and package hash.
- The current `parse-baseline` and `generate-full-model` endpoints move in the
  right direction, but the remaining hardening item is to keep shrinking the
  HTTP production surface from generic argv execution into typed pipeline
  operations with server-derived paths, manifests, and run evidence.
- Oracle also called out mandatory release-publish gates: source/dependency
  hash evidence, stage-aware metrics, successful full parse/generate, package
  validation, release-local mesh asset completeness, same-release diff zero,
  and two-pane runtime-scene evidence before treating a release as production
  visual truth.

## 2026-06-20 Generate Full Model: Site Table Fixed, Long-Stage Heartbeats Added

Purpose:

- Use the successful DB1112 897 parse evidence to run production-mode
  `generate-full-model`.
- Fix backend site model generation and continue hardening long-stage task
  supervision.

Evidence:

```text
snapshot_id=http-prepare-physical-1112-smallchunk-long-20260620-1113
parse_run_id=http-parse-baseline-1112-smallchunk-long-20260620-1113
baseline_state_manifest_hash=a15de8ff2efa6945cbfba7a03b689842319df89fa1c8622f757784bf8b89f4ab
replacement_db_sha256=70f18c70116f392eae533b75fb8f4043d031a5f049448531cc1dfc43faf7d3c2
```

Production gate validation:

```text
POST /api/model-version/runs/generate-full-model
parse_run_status=succeeded
allow_incomplete_parse=false
source_observation primary hash=70f18c70116f392eae533b75fb8f4043d031a5f049448531cc1dfc43faf7d3c2
```

Observed fix:

- The earlier diagnostic generation failed at `collect_transform_refresh_roots`
  with `The table 'SITE' does not exist`.
- After the successful DB1112 parse-baseline, generation found real site roots:

```text
transform 刷新 roots: dbnum=1112 下 16 个 SITE
DbMetaManager loaded db_meta_info.json, ref0 mappings=8, db_files=5
```

New observability gaps and fixes:

- First production generate run:
  `http-generate-full-1112-after-parse-20260620-1205`
  was cancelled with reason `rerun-with-transform-refresh-heartbeat`.
  It was still active, but metrics stayed at `collect_transform_refresh_roots`
  while `refresh_pe_transform_for_root_refnos` processed a long 373k-node
  subtree. Source DB hash remained unchanged.
- Added `refresh_pe_transform_*` generate metrics in
  `src/pe_transform_refresh.rs`.
- Second production generate run:
  `http-generate-full-1112-heartbeat-20260620-1215`
  proved the new transform heartbeat, reaching:

```text
refresh_pe_transform_batch_saved
root_index=16/16
processed=371000+
```

- It was then cancelled with reason `rerun-with-pre-cleanup-heartbeat` after
  entering a similarly long `pre_cleanup_for_regen` stage with no finer-grained
  metrics. Source DB hash remained unchanged.
- Added `pre_cleanup_for_regen_started`, `pre_cleanup_for_regen_progress`,
  `pre_cleanup_for_regen_tubi_cleanup`, and `pre_cleanup_for_regen_done`
  metrics in `src/fast_model/gen_model/pdms_inst.rs`.
- Third production generate run:
  `http-generate-full-1112-cleanup-heartbeat-20260620-1241`
  is now running with both transform and pre-cleanup heartbeat support.

Build validation:

```text
cargo fmt --check: pass
cargo check --bin web_server --features "web_server,model-version-ducklake": pass
cargo build --bin aios-database --target-dir E:\codex-targets\plant-cli-validate-build: pass
```

Review:

- The backend site generation issue was not a missing query path; it was
  caused by incomplete baseline data. A complete parse makes SITE roots
  available to generation.
- Long generate stages now need the same production observability standard as
  parse stages. Transform refresh and pre-cleanup were the first two places
  found and fixed.
- The current run must still reach terminal `succeeded` before this can be
  treated as a full model generation success.

## 2026-06-20 DB1112 897 Generate Full Model Succeeded

Run:

```text
run_id=http-generate-full-1112-cleanup-heartbeat-20260620-1241
snapshot_id=http-prepare-physical-1112-smallchunk-long-20260620-1113
parse_run_id=http-parse-baseline-1112-smallchunk-long-20260620-1113
status=succeeded
exit_code=0
source_hash_unchanged=true
duration_ms=2566190
```

Generation metrics:

```text
inst_relate=30455
inst_info=29962
inst_relate_aabb=29583
tubi_count=49
mesh_generated=983
mesh_cache_hit=6009
boolean_success=0
boolean_failed=0
error_count=0
cache_miss=0
```

Parquet export metrics:

```text
parquet_files=8
parquet_bytes=3610172
json_files=3
json_bytes=15555
export_duration_ms=175180
```

Parquet manifest summary for dbnum 1112:

```text
instances.parquet=52020 rows
geo_instances.parquet=28704 rows
transforms.parquet=29001 rows
aabb.parquet=27649 rows
tubings.parquet=42 rows
ptsets.parquet=0 rows
primitive_keypoints.parquet=0 rows
spec_info_1112.parquet=901 rows
```

Important quality gate:

```text
mesh_validation.checked_geo_hashes=1324
mesh_validation.missing_geo_hashes=23
mesh_validation.missing_owner_refnos=208
mesh_validation.policy=retain_missing_mesh_rows
missing_mesh_report=output/AvevaMarineSample/model_versions/physical_baselines/http-prepare-physical-1112-smallchunk-long-20260620-1113/output/AvevaMarineSample/parquet/1112/missing_mesh_report_1112.json
```

Review:

- The backend site-model generation problem is fixed for the real DB1112 897
  evidence chain: complete parse -> production generate gate -> full model
  regeneration -> Parquet export all completed with source hash unchanged.
- The run proved why stage-aware metrics are part of correctness. The old
  binary would have appeared stale during:
  - precheck pe_transform refresh by dbnum;
  - `gen_all_geos_data` inner long await;
  - post-generation Parquet export.
- Added hardening for future runs:
  - `record_generate_heartbeat` in `src/perf_metrics.rs`;
  - OS-thread generate heartbeat around CLI `gen_all_geos_data` awaits in
    `src/cli_modes.rs`;
  - dbnum-path pe_transform refresh metrics in `src/pe_transform_refresh.rs`;
  - post-generation export progress markers in
    `src/fast_model/export_model/post_gen_export.rs`.
- This generated package should be treated as `degraded_visual` or
  `quarantined_visual` until missing mesh repair/materialization closes the
  23 geo-hash gap. It is still valid evidence that parse/save/generate/export
  works end-to-end for DB1112 897.

Validation:

```text
cargo fmt --check: pass
cargo check --bin web_server --features "web_server,model-version-ducklake": pass
cargo build --bin aios-database --target-dir E:\codex-targets\plant-cli-validate-build: pass
HTTP GET run status: succeeded, exit_code=0, source_hash_unchanged=true
```

Note:

- A temporary attempt to build a fresh isolated CLI target at
  `E:\codex-targets\plant-cli-heartbeat-build` failed because `manifold-rs`
  tried to clone Clipper2 and hit a TLS handshake failure. The cached
  production validation target built successfully after the running child
  process released `aios-database.exe`.

## 2026-06-20 DB1112 897 Mesh Gate And Release Decision

Purpose:

- Validate whether the successful DB1112 897 parse/generate/export package can
  be promoted to a visual model release.
- Verify the missing mesh repair path before deciding between complete visual
  release, quarantined visual release, or deeper geometry fixes.

Validation command shape:

```text
aios-database.exe model-version validate-history-replay
  --project AvevaMarineSample
  --dbnum 1112
  --source-db-file D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001
  --from-sesno 896
  --to-sesno 897
  --parquet-dir output\AvevaMarineSample\model_versions\physical_baselines\http-prepare-physical-1112-smallchunk-long-20260620-1113\output\AvevaMarineSample\parquet\1112
  --current-parquet-dir output\AvevaMarineSample\parquet\1112
  --scene-tree-dir output\AvevaMarineSample\model_versions\physical_baselines\http-prepare-physical-1112-smallchunk-long-20260620-1113\output\AvevaMarineSample\scene_tree
  --json
```

Observed validation result:

```text
classification=missing_mesh_assets
ready_for_publish=false
missing_geo_hashes=23
missing_owner_refnos=208
recommended_action=generate/materialize all GLB mesh assets before publishing
```

Targeted repair dry-run:

```text
requested_hashes=23
dry_run_eligible=23
missing_inst_geo=0
param_missing=0
bad_skipped=0
```

Targeted repair actual run:

```text
requested_hashes=23
attempted_hashes=23
generated_hashes=0
still_missing_hashes=23
status for rows=generation_failed_bad
message="generation did not produce a GLB and inst_geo is marked bad"
```

Decision:

- The 897 package proves backend parse/save/model generation/export is working
  end-to-end, but it is not a `complete_visual` release.
- The 23 missing geo hashes cannot be fixed by a simple missing-asset repair
  pass; generation marks them as bad geometry.
- A production publish must either:
  - fix the bad geometry generation path and re-export until
    `classification=complete_visual_release_candidate`; or
  - explicitly quarantine/drop those render rows, refresh manifest
    `mesh_validation` so `render_missing_geo_hashes=0`, and publish as
    `quarantined_visual` with visible UI evidence.
- The system must not silently publish this package as a complete visual model.

Next action:

- Implement or reuse the quarantine/export policy that was proven for the 791
  baseline, apply it to the 897 package, then publish it as a quarantined visual
  release only if validation returns
  `classification=quarantined_visual_release_candidate` and same-release diff
  remains zero.

## 2026-06-20 Real 791 vs 897 Quarantined Visual Comparison

Purpose:

- Move from "897 generated but not publishable" to two real DB1112 releases
  loaded side by side in the model-version compare UI.
- Keep the missing mesh policy explicit: both releases are
  `quarantined_visual`, not `complete_visual`.

Build validation:

```text
cargo build --bin aios-database --features model-version-ducklake --target-dir E:\codex-targets\plant-cli-ducklake-build
```

Result:

- Exit code `0`.
- No `cargo test` was run.
- Existing upstream `pdms_io`/parser warnings remain.

897 quarantine export:

```text
config=output\AvevaMarineSample\model_versions\physical_baselines\http-prepare-physical-1112-smallchunk-long-20260620-1113\DbOption-physical-baseline
AIOS_PARQUET_DROP_MISSING_MESH_ROWS=1
output=output\AvevaMarineSample\model_versions\physical_baselines\http-prepare-physical-1112-smallchunk-long-20260620-1113\validation-export-quarantine\1112
```

Result:

```text
instances=28651
geo_instances=28496
transforms=29001
aabb=27649
tubings=42
raw_missing_geo_hashes=23
raw_missing_owner_refnos=208
render_missing_geo_hashes=0
render_missing_owner_refnos=0
quarantined_geo_hashes=23
quarantined_owner_refnos=208
```

897 replay validation:

```text
classification=quarantined_visual_release_candidate
ready_for_publish=true
mesh_assets_complete=true
scene_tree required=true, tree_file_exists=true, db_meta_info_exists=true
```

897 published release:

```text
release_id=codex-ams1112-physical-897-quarantine
release_lifecycle=published
release_quality=quarantined_visual
package_hash=f01dde24c706e3127007c0df080123a378c44f77bf8e586da2087b8d8422290d
source_manifest_hash=9981dd6d512459aab4456a1203b00f96b63be0c6a14603200e5ad84abd8b627c
baseline_state_manifest_hash=a15de8ff2efa6945cbfba7a03b689842319df89fa1c8622f757784bf8b89f4ab
generation_job_id=http-generate-full-1112-cleanup-heartbeat-20260620-1241
component_count=28651
mesh_geo_hash_count=1303
mesh_missing_count=0
unit_count=758
unresolved_member_count=26779
```

791 recovery and publish:

- The previous 791 package was not present in the current global release root.
- The physical source still exists:
  `D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams1112_0001 copy`.
- A new snapshot/config was prepared:
  `codex-ams1112-physical-791-reuse-20260620`.
- It reused the documented Surreal namespace:
  `codex_baseline_ams1112_791`.
- Export from that namespace succeeded, proving the previous 791 model state is
  still present.

791 quarantine export result:

```text
instances=26117
geo_instances=31252
transforms=30495
aabb=28372
tubings=56
ptsets=6897
raw_missing_geo_hashes=22
raw_missing_owner_refnos=40
render_missing_geo_hashes=0
render_missing_owner_refnos=0
quarantined_geo_hashes=22
quarantined_owner_refnos=40
```

791 replay validation:

```text
classification=quarantined_visual_release_candidate
ready_for_publish=true
mesh_assets_complete=true
```

791 published release:

```text
release_id=codex-ams1112-physical-791-quarantine
release_lifecycle=published
release_quality=quarantined_visual
package_hash=770d6470a32d8699a60c4fc2b0037a48db39f30804b28a54fe1eedd961c68c4c
source_manifest_hash=cc3dc9d0585897b364c968ff25afd908a377c51c54a0fe0ea51161ee877fc83f
baseline_state_manifest_hash=7b6fbada31126a9a19add6707fb09bbbcc87a64565dc781966c95584de182948
generation_job_id=reused-surreal-namespace-codex_baseline_ams1112_791
component_count=26117
mesh_geo_hash_count=1192
mesh_missing_count=0
unit_count=684
unresolved_member_count=24238
```

CLI validation:

```text
mesh-assets --missing-only codex-ams1112-physical-791-quarantine:
  missing_count=0, assets=[]

mesh-assets --missing-only codex-ams1112-physical-897-quarantine:
  missing_count=0, assets=[]

diff 791 -> 791:
  added=0 deleted=0 changed=0 unchanged=26117

diff 897 -> 897:
  added=0 deleted=0 changed=0 unchanged=28651

diff 791 -> 897:
  added=5059 deleted=2525 changed=43 unchanged=23549
  total_old=26117 total_new=28651

unit-diff 791 -> 897:
  added=91 deleted=17 changed=119 unchanged=548
  total_old=684 total_new=758
```

HTTP validation on existing web_server:

```text
server=http://127.0.0.1:3925
GET /api/version -> 0.3.34
GET /api/model-version/releases?project=AvevaMarineSample&dbnum=1112
  returns both codex-ams1112-physical-791-quarantine and
  codex-ams1112-physical-897-quarantine

GET /api/model-version/diff?from_release_id=codex-ams1112-physical-791-quarantine&to_release_id=codex-ams1112-physical-897-quarantine&limit=5
  added=5059 deleted=2525 changed=43 unchanged=23549 emitted=5

GET /api/model-version/releases/codex-ams1112-physical-897-quarantine/mesh-assets?missing_only=true
  missing_count=0, assets=[]

GET /api/model-version/releases/codex-ams1112-physical-897-quarantine/runtime-scene?limit=20
  release-local mesh_base_url used
  component_count=20 geometry_count=12 truncated=true
```

Browser validation:

```text
url=http://127.0.0.1:3925/model-version/compare?project=AvevaMarineSample&from=codex-ams1112-physical-791-quarantine&to=codex-ams1112-physical-897-quarantine
tool=agent-browser
left iframe:
  components 2000
  geometries 2288/2288
  failed 0
  webgl=true
right iframe:
  components 2000
  geometries 2041/2041
  failed 0
  webgl=true
diff metrics:
  Added 5059
  Deleted 2525
  Changed 43
  Unchanged 23549
  Emitted 200
```

Screenshot:

```text
.planning/2026-06-17-ducklake-valv-version-diff/model-version-compare-791-897-quarantine-agent-browser.png
```

Self review:

- The two-pane model comparison now uses two real DB1112 physical releases,
  not the earlier controlled fixture and not the partial 897 package.
- Both releases are honest `quarantined_visual` packages with zero render-missing
  mesh dependencies after dropping bad rows.
- The UI has proven that both panes load WebGL geometry with `failed=0`.
- Remaining production hardening is still needed before marking the whole goal
  complete:
  - quarantine quality/provenance should be more visible in the compare UI;
  - runtime-scene is still capped at 2000 components and needs paging/tiling for
    full-site production use;
  - 791 was recovered by reusing an existing Surreal namespace, so a completely
    reproducible 791 parse/generate rerun should be scheduled before final
    operator handoff;
  - native pdms-io sesno delta remains future optimization and still needs
    full-state equivalence proof.

## Goal Continuation Update - 2026-06-20 14:30

Completed compare UI quality/provenance hardening.

Code changed:

```text
src/web_api/model_version_api.rs
  - compare page now renders release quality badges for both panes
  - provenance meta now shows lifecycle, quality, package hash, asset manifest
    hash, baseline hash, generation job, manifest URL, and package URL
  - release metadata rendered through escapeHtml() before innerHTML insertion
```

Build validation:

```text
cargo fmt --check
  passed

cargo check --bin web_server --features "web_server,model-version-ducklake" --target-dir target/codex-web-compare-quality-build
  passed
  notes: only existing pdms-io / parse_pdms_db warnings

cargo build --bin web_server --features "web_server,model-version-ducklake" --target-dir target/codex-web-compare-quality-build
  passed
  notes: only existing pdms-io / parse_pdms_db warnings
```

Temporary validation server:

```text
server=http://127.0.0.1:3926
pid=33116
GET /api/version -> 0.3.34, buildDate=2026-06-20 14:28:05 UTC+8
```

HTTP validation:

```text
GET /api/model-version/releases?project=AvevaMarineSample&dbnum=1112
  codex-ams1112-physical-791-quarantine:
    lifecycle=published
    quality=quarantined_visual
    package_hash=770d6470a32d...
    asset_manifest_hash=b627f3095869...
    generation_job_id=reused-surreal-namespace-codex_baseline_ams1112_791
  codex-ams1112-physical-897-quarantine:
    lifecycle=published
    quality=quarantined_visual
    package_hash=f01dde24c706...
    asset_manifest_hash=1100d09b9173...
    generation_job_id=http-generate-full-1112-cleanup-heartbeat-20260620-1241

GET /api/model-version/diff?from_release_id=codex-ams1112-physical-791-quarantine&to_release_id=codex-ams1112-physical-897-quarantine&limit=5
  added=5059 deleted=2525 changed=43 unchanged=23549 emitted=5

GET /model-version/compare?...791...897...
  status=200
  contains quality-badge=true
  contains renderQuality=true
  contains meta-grid=true
  contains escapeHtml=true
  contains mobile meta-grid media rule=true
```

Browser validation:

```text
url=http://127.0.0.1:3926/model-version/compare?project=AvevaMarineSample&from=codex-ams1112-physical-791-quarantine&to=codex-ams1112-physical-897-quarantine
tool=agent-browser

badges:
  from: text=quarantined_visual class="quality-badge quarantined"
  to:   text=quarantined_visual class="quality-badge quarantined"
  mobileMetaGridCss=true

left iframe:
  components 2000
  geometries 2288/2288
  failed 0
  webgl=true

right iframe:
  components 2000
  geometries 2041/2041
  failed 0
  webgl=true

diff metrics:
  Added 5059
  Deleted 2525
  Changed 43
  Unchanged 23549
  Emitted 200
```

Screenshot:

```text
.planning/2026-06-17-ducklake-valv-version-diff/model-version-compare-791-897-quality-agent-browser.png
```

Oracle MCP status:

```text
First full-context browser consult:
  e3d-ducklake-model-version-review-3
  failed: attachments did not finish uploading before timeout

Reduced inline consult:
  e3d-ducklake-review-core-inline
  status: running at time of this progress update
```

Self review:

- The compare page now makes quarantined visual quality visible in the first
  viewport instead of hiding it in JSON.
- The UI exposes enough provenance for a user to verify that the panes are real
  physical 791/897 releases and not an arbitrary runtime snapshot.
- Escaping was added after review because provenance is rendered via innerHTML.
- Remaining hardening before final goal closure:
  - absorb the Oracle MCP second opinion once the reduced inline session returns;
  - decide whether to keep the temporary 3926 server alive for user inspection or
    stop it after handoff;
  - document the 791 spec_info fallback as a known release-quality caveat;
  - schedule a fully reproducible 791 parse/generate rerun independent of the
    reused Surreal namespace.

## Goal Continuation Update - 2026-06-20 14:45

Oracle MCP:

```text
session=e3d-ducklake-review-core-inline
status=completed
mode=browser GPT-5.5 Pro
elapsed=7m38s
input_tokens=18801
output_tokens=5992
```

Oracle conclusion:

- DuckLake is the right layer for release registry, component/unit/mesh indexes,
  diff metadata, lifecycle, and audit lookup.
- DuckLake should not become the payload truth. Immutable Parquet package
  manifests plus content-addressed GLB assets remain the release payload truth.
- SurrealDB remains a replay/generation workspace and cache, not a durable
  published-release source of truth.
- SQLite is acceptable as a local DuckLake catalog or ops DB, but not as a second
  release registry.
- `quarantined_visual` is a valid quality for DB1112 791/897 only when raw
  missing mesh rows are fully accounted for by quarantine and render-missing
  dependencies are zero.

Oracle P0 hardening implemented in this continuation:

```text
src/version_management/history_replay_validation.rs
  - missing mesh_validation no longer defaults to complete
  - render missing owner_refnos now gate publish together with geo_hashes
  - raw/render/quarantined missing counts must conserve for geo_hashes and owner_refnos
  - package evidence exposes mesh_validation_present and quarantine_counts_consistent

src/version_management/types.rs
  - ModelHistoryReplayPackageEvidence now records mesh_validation_present
    and quarantine_counts_consistent

src/version_management/model_release.rs
  - release_id is validated before register/publish
  - user metadata release_quality/quality is read from history_publish.user_metadata
  - baseline_state_manifest_hash without a path is rejected for publish evidence
  - failed status update errors are returned with context
  - publish response reloads the final DuckLake release record after Published
```

Known Oracle P0 items intentionally left as follow-up because they need migration
or wider schema design:

- `ModelReleaseStatus::from_storage(None)` still uses the current compatibility
  default and should be migrated instead of changed in-place.
- `release_quality` should become a typed explicit publish input; current string
  inference is retained as a migration fallback.
- `spec_info fallback=0` needs first-class validation flags / spec_source fields
  before the release can claim complete semantic diff quality.

Latest build validation:

```text
cargo fmt --check
  passed

cargo build --bin web_server --features "web_server,model-version-ducklake" --target-dir target/codex-web-compare-quality-build
  passed
  notes: only existing pdms-io / parse_pdms_db warnings

cargo build --bin aios-database --features "model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  initial attempt failed at link due E: no space on device
  cleared only E:\codex-targets\plant-cli-ducklake-build after absolute path check
  rerun passed
  notes: only existing pdms-io / parse_pdms_db warnings
```

Latest temporary validation server:

```text
server=http://127.0.0.1:3926
pid=61844
GET /api/version -> 0.3.34, commit=6eda7c194efa39b3ce14d0708cea1dc7683527e6, buildDate=2026-06-20 14:39:34 UTC+8
```

Latest CLI JSON gate validation:

```text
791 validate-history-replay:
  classification=quarantined_visual_release_candidate
  ready_for_publish=true
  rows: instances=26117 geo_instances=31252 transforms=30495 aabb=28372
  raw_missing_geo_hashes=22
  quarantined_geo_hashes=22
  raw_missing_owner_refnos=40
  quarantined_owner_refnos=40
  render_missing_geo_hashes=0
  render_missing_owner_refnos=0
  mesh_validation_present=true
  quarantine_counts_consistent=true

897 validate-history-replay:
  classification=quarantined_visual_release_candidate
  ready_for_publish=true
  rows: instances=28651 geo_instances=28496 transforms=29001 aabb=27649
  raw_missing_geo_hashes=23
  quarantined_geo_hashes=23
  raw_missing_owner_refnos=208
  quarantined_owner_refnos=208
  render_missing_geo_hashes=0
  render_missing_owner_refnos=0
  mesh_validation_present=true
  quarantine_counts_consistent=true
```

Latest CLI catalog/diff validation:

```text
model-version list:
  codex-ams1112-physical-791-quarantine lifecycle=published quality=quarantined_visual
  codex-ams1112-physical-897-quarantine lifecycle=published quality=quarantined_visual

component diff 791 -> 897:
  added=5059
  deleted=2525
  changed=43
  unchanged=23549
  total_old=26117
  total_new=28651

unit diff 791 -> 897:
  added=91
  deleted=17
  changed=119
  unchanged=548
  total_old=684
  total_new=758
```

Latest HTTP validation:

```text
GET /api/model-version/releases?project=AvevaMarineSample&dbnum=1112
  success=true
  data.releases contains both DB1112 791/897 quarantined_visual releases

GET /api/model-version/diff?...791...897...limit=5
  success=true
  added=5059 deleted=2525 changed=43 unchanged=23549 emitted=5

GET /model-version/compare?...791...897...
  status=200
  contains quality-badge=true
  contains meta-grid=true
  contains escapeHtml=true
  responsive CSS uses @media (max-width: 900px)

GET /api/model-version/releases/codex-ams1112-physical-791-quarantine/runtime-scene?limit=2000
  quality=quarantined_visual
  components=2000
  geometry_count=2288
  package_url=/files/output/AvevaMarineSample/model_versions/releases/codex-ams1112-physical-791-quarantine/parquet/1112

GET /api/model-version/releases/codex-ams1112-physical-897-quarantine/runtime-scene?limit=2000
  quality=quarantined_visual
  components=2000
  geometry_count=2041
  package_url=/files/output/AvevaMarineSample/model_versions/releases/codex-ams1112-physical-897-quarantine/parquet/1112
```

## 2026-06-20 Quality Annotation And Oracle Follow-Up

Oracle MCP:

- Re-read completed session `e3d-ducklake-review-core-inline`; its storage
  decision remains the basis for this slice: DuckLake is the
  release/catalog/index/diff/audit layer, immutable Parquet/GLB packages remain
  payload truth, and SurrealDB remains workspace/cache.
- Attempted a larger browser follow-up as `e3d-version-arch-followup-core-2`.
  The session completed with only `Something went wrong`, so no new answer from
  that attempt was used.

Implementation:

- Added persisted release quality evidence:
  - `release_quality_reason`
  - `validation_flags`
  - `spec_info_fallback_count`
- Added DuckLake migrations and row mapping for those fields.
- Added `model-version annotate` so existing releases can receive catalog-only
  quality evidence without re-copying immutable Parquet/GLB packages.
- Updated compare metadata UI to show quality reason, validation flags, and spec
  fallback.

Applied annotations:

```text
codex-ams1112-physical-791-quarantine:
  flags=mesh_missing_rows_quarantined,
        spec_info_fallback,
        spec_info_fallback_unquantified

codex-ams1112-physical-897-quarantine:
  flags=mesh_missing_rows_quarantined
```

Validation:

```text
cargo fmt --check
  passed

cargo build --bin aios-database --features "model-version-ducklake"
  passed, existing pdms-io warnings only

cargo build --bin web_server --features "web_server,model-version-ducklake"
  passed, existing pdms-io warnings only

model-version list --project AvevaMarineSample --json
  returns both real DB1112 release quality reasons and flags

model-version diff 791 -> 897
  added=5059 changed=43 deleted=2525 unchanged=23549

model-version unit-diff 791 -> 897
  added=91 deleted=17 changed=119 unchanged=548

HTTP runtime-scene limit=2000:
  791 components=2000 geometries=2288 quality=quarantined_visual
  897 components=2000 geometries=2041 quality=quarantined_visual

agent-browser screenshot:
  .planning/2026-06-17-ducklake-valv-version-diff/model-version-compare-791-897-quality-annotated-agent-browser.png
```

Remaining:

- Do not invent a numeric `spec_info_fallback_count` for 791; it remains null
  until a reliable count is computed.
- Add dedicated DuckLake migrate/reconcile/publish-attempt commands before
  multi-process operator deployment.

## 2026-06-20 Explicit DuckLake Catalog Migration

Oracle MCP:

- Reused completed sessions `e3d-model-version-ducklake-current` and
  `e3d-ducklake-review-core-inline`.
- Both reinforce the same architecture decision:
  immutable Parquet/GLB release packages are payload truth; DuckLake is only
  release/catalog/index/diff/audit metadata; SurrealDB remains mutable
  generation workspace/cache.
- Prepared a new focused Oracle bundle for the migrate implementation
  (`e3d-ducklake-migrate-review`, dry-run with files report, about 128k input
  tokens). It was not submitted as a new browser run in this slice because the
  existing completed Oracle answers already covered the storage boundary and
  the local implementation/verification loop was unblocked.

Implementation:

- Added `ModelVersionCatalogMigrationReport` to
  `src/version_management/types.rs`.
- Added required table/column readiness checks and
  `catalog_migration_report()` to `src/version_management/ducklake_store.rs`.
- Added `migrate_model_version_catalog()` wrapper in
  `src/version_management/model_release.rs`.
- Added `aios-database model-version migrate --project <name> --json` in
  `src/version_management/cli.rs`.
- Read-only DuckLake schema errors now instruct operators to run the explicit
  migration command instead of allowing GET APIs to mutate schema.

Validation:

```text
sigmap ask "model version DuckLake schema migration CLI migrate incremental model architecture"
  timed out twice; continued with targeted rg/direct source inspection.

cargo fmt --check
  passed

cargo build --bin aios-database --features "model-version-ducklake"
  passed with existing pdms-io warnings only

cargo check --bin aios-database
  passed with existing pdms-io warnings only; confirms the shared DTO changes do
  not break the default non-DuckLake-feature build

first cargo check attempt with a fresh E:\codex-targets target
  failed because E: was full during libduckdb-sys unpack
  cleaned only E:\codex-targets\plant-cli-ducklake-check
  reran against warmed E:\codex-targets\plant-cli-ducklake-build
```

CLI migrate evidence:

```text
aios-database model-version migrate --project AvevaMarineSample --json
  release_count=4
  required_tables all true
  required_release_columns all true
  release_quality_columns_present=true
  migrated=true

same command repeated
  same report, proving idempotence
```

CLI read/diff evidence after migration:

```text
model-version list --project AvevaMarineSample --json
  returns both DB1112 791/897 releases with quality reasons and flags

model-version diff 791 -> 897 --limit 3 --json
  added=5059
  deleted=2525
  changed=43
  unchanged=23549

model-version unit-diff 791 -> 897 --limit 3 --json
  added=91
  deleted=17
  changed=119
  unchanged=548
```

web_server validation:

```text
cargo build --bin web_server --features "web_server,model-version-ducklake"
  first attempt failed only because the old validation web_server process held
  target\codex-web-compare-quality-build\debug\web_server.exe

Stopped the verified process on 127.0.0.1:3926:
  pid=69608
  path=target\codex-web-compare-quality-build\debug\web_server.exe

Rebuilt web_server:
  passed with existing pdms-io warnings only

Restarted web_server:
  pid=41120
  port=3926
```

HTTP read evidence after migration:

```text
GET /api/version
  version=0.3.34
  buildDate=2026-06-20 16:01:25 UTC+8

GET /api/model-version/releases?project=AvevaMarineSample
  success=true
  exposes release_quality_reason, validation_flags,
  spec_info_fallback_count

GET /api/model-version/diff?...791...897...limit=3
  success=true
  added=5059 deleted=2525 changed=43 unchanged=23549

GET /api/model-version/releases/codex-ams1112-physical-791-quarantine/runtime-scene?project=AvevaMarineSample&limit=20
  success=true
  quality=quarantined_visual
  components=20
  geometry_count=20
  mesh_base_url points at release-local meshes
  truncated=true

GET /api/model-version/releases/codex-ams1112-physical-897-quarantine/runtime-scene?project=AvevaMarineSample&limit=20
  success=true
  quality=quarantined_visual
  components=20
  geometry_count=12
  mesh_base_url points at release-local meshes
  truncated=true
```

Browser evidence:

```text
agent-browser page:
  /model-version/compare
  selected from=codex-ams1112-physical-791-quarantine
  selected to=codex-ams1112-physical-897-quarantine
  diff table populated with added rows
  both iframe panes present

screenshot:
  .planning/2026-06-17-ducklake-valv-version-diff/
    model-version-compare-791-897-post-migrate-agent-browser.png
```

Review notes:

- The `migrate` command is the right production boundary for read-only
  web_server deployments, because GET APIs remain non-mutating.
- The current report checks readiness but does not yet include DuckLake
  extension version, catalog backend type, last schema migration id, or an
  operator-facing `missing_*` array; those are useful P1 improvements.
- The compatibility default where missing release status can still read as
  `published` remains a P0/P1 migration hardening item.
- Publish attempt/reconcile commands are still needed for crash recovery across
  staged, validating, asset indexing, unit indexing, and published states.

## DuckLake Schema Migration Audit - 2026-06-20 16:45

Goal:

- Make `model-version migrate` produce durable audit evidence of which schema
  migrations/backfills have been applied.
- Make read-only `web_server` deployments fail fast when the migration audit
  infrastructure is absent, instead of mutating DuckLake from GET routes.

Oracle MCP basis:

```text
accepted:
  e3d-model-version-ducklake-current
  e3d-ducklake-review-core-inline

ignored:
  e3d-version-arch-followup-core-2
  reason: stored response only says "Something went wrong"
```

SigMap:

```text
sigmap ask "DuckLake model-version migration audit schema model data version architecture"
  timed out after ~64s
fallback:
  used rg/direct file inspection and Oracle MCP session transcripts
```

Implemented:

```text
src/version_management/types.rs
  ModelVersionCatalogMigrationReport now includes:
    schema_migration_count
    applied_schema_migrations
    missing_tables
    missing_release_columns

src/version_management/ducklake_store.rs
  required_tables() includes model_version_schema_migrations
  ensure_schema() creates model_version_schema_migrations
  ensure_schema_migrations() records ids:
    0001_base_model_version_schema
    0002_release_lifecycle_quality_columns
    0003_release_quality_evidence_columns
    0004_release_provenance_columns
    0005_release_status_lifecycle_quality_backfill
  validate_read_schema() requires the migration audit table
  catalog_migration_report() returns ids/count and missing arrays

src/version_management/cli.rs
  non-json migrate output prints migration count and applied ids
```

Build and format:

```text
cargo fmt --check
  passed

cargo build --bin aios-database --features "model-version-ducklake" \
  --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo build --bin web_server --features "web_server,model-version-ducklake" \
  --target-dir target\codex-web-compare-quality-build
  passed with existing pdms-io warnings only

cargo check --bin aios-database \
  --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only
```

Operational note:

```text
first CLI build attempt:
  failed during link with "no space on device" on E:

cleanup:
  deleted only old generated directories under verified root E:\codex-targets:
    plant-cli-validate-build
    plant-model-gen-roomtree
    plant-cli-heartbeat-build
```

CLI migrate evidence:

```text
aios-database model-version migrate --project AvevaMarineSample --json
  release_count=4
  schema_migration_count=5
  applied_schema_migrations=
    0001_base_model_version_schema,
    0002_release_lifecycle_quality_columns,
    0003_release_quality_evidence_columns,
    0004_release_provenance_columns,
    0005_release_status_lifecycle_quality_backfill
  required_tables.model_version_schema_migrations=true
  release_quality_columns_present=true
  missing_tables=[]
  missing_release_columns=[]
  migrated=true

same command repeated:
  schema_migration_count=5
  release_count=4
  no duplicate migration ids observed
```

CLI regression evidence:

```text
model-version list --project AvevaMarineSample --json
  release_count=4
  791 lifecycle=published quality=quarantined_visual
  897 lifecycle=published quality=quarantined_visual

model-version diff 791 -> 897 --json
  added=5059
  deleted=2525
  changed=43
  unchanged=23549
  total_old=26117
  total_new=28651
  emitted=200

model-version unit-diff 791 -> 897 --json
  added=91
  deleted=17
  changed=119
  unchanged=548
  total_old=684
  total_new=758
  emitted=200

model-version diff 897 -> 897 --json
  added=0
  deleted=0
  changed=0
  unchanged=28651
  emitted=0
```

web_server validation:

```text
stopped previous validation web_server:
  pid=41120
  port=3926

started rebuilt web_server:
  pid=56044
  port=3926

GET /api/version
  version=0.3.34
  buildDate=2026-06-20 16:28:25 UTC+8

GET /api/model-version/releases?project=AvevaMarineSample
  success=true
  release_count=4
  quality reasons and validation flags returned for both physical releases

GET /api/model-version/diff?...791...897
  success=true
  added=5059 changed=43 deleted=2525 unchanged=23549

GET /api/model-version/releases/{791}/runtime-scene?limit=10
  success=true
  quality=quarantined_visual
  mesh_url_pattern points at release-local meshes/lod_L1

GET /api/model-version/releases/{897}/runtime-scene?limit=10
  success=true
  quality=quarantined_visual
  mesh_url_pattern points at release-local meshes/lod_L1
```

Browser evidence:

```text
agent-browser session:
  e3d-schema-audit

page:
  http://127.0.0.1:3926/model-version/compare

selected:
  from=codex-ams1112-physical-791-quarantine
  to=codex-ams1112-physical-897-quarantine

screenshot:
  .planning/2026-06-17-ducklake-valv-version-diff/
    model-version-compare-791-897-schema-audit-agent-browser.png
```

Observed page state:

```text
two WebGL panes loaded
both panes show quarantined_visual badges
quality reasons visible
diff cards:
  Added=5059
  Deleted=2525
  Changed=43
  Unchanged=23549
  Emitted=200
```

Remaining risks:

- Read-only validation currently requires the audit table but does not yet
  verify the exact required migration id set.
- Migration id idempotence is app-level; production concurrent writers still
  need a writer lock/single-writer queue or a server catalog.
- The compatibility default where missing release status may read as
  `published` still needs a final explicit backfill/removal plan.
- Publish attempt/reconcile and richer validation/quarantine report hashes are
  still P0/P1 before declaring the whole model-version feature production-grade.

## 2026-06-20 16:55 - Required Migration Id Enforcement

Oracle MCP continuation:

```text
used sessions:
  e3d-model-version-ducklake-current
  e3d-ducklake-review-core-inline

accepted architecture decision:
  DuckLake is appropriate for release/catalog/index/diff/audit metadata.
  Immutable release package Parquet + release-local GLB assets remain payload
  truth. SurrealDB remains mutable parse/generation workspace. User-facing
  model version remains release_id; sesno, package hash, source hash,
  baseline manifest hash, generation job id, asset manifest hash, and DuckLake
  snapshot are evidence fields.
```

SigMap note:

```text
sigmap ask "DuckLake model-version read schema required migration ids catalog migration report"
  timed out after about 64 seconds

fallback:
  used direct source reads and rg after recording the timeout
```

Implementation:

```text
src/version_management/types.rs
  ModelVersionCatalogMigrationReport now includes:
    required_schema_migrations
    missing_schema_migrations

src/version_management/ducklake_store.rs
  migration ids are held as constants and exposed through one required-id list
  required_schema_migrations() is the single current-binary required id list
  validate_read_schema() now fails if any required id is missing
  catalog_migration_report() reports required and missing id arrays
  ensure_schema_migrations() records the required list instead of duplicated
  hard-coded calls

src/version_management/cli.rs
  non-json migrate output prints required_schema_migrations and
  missing_schema_migrations
```

Temporary negative catalog validation:

```text
catalog:
  target/codex-ducklake-migration-id-negative

steps:
  model-version migrate --project NegativeMigrationId --json
    schema_migration_count=5
    missing_schema_migrations=[]

  deleted only migration id 0005_release_status_lifecycle_quality_backfill from
  the temporary DuckLake metadata database using Python DuckDB

  model-version list --project NegativeMigrationId --json
    exit_code=1
    message contains 0005_release_status_lifecycle_quality_backfill

  model-version migrate --project NegativeMigrationId --json
    schema_migration_count=5
    missing_schema_migrations=[]

  model-version list --project NegativeMigrationId --json
    exit_code=0
```

Build and CLI validation:

```text
cargo fmt --check
  passed

cargo build --bin aios-database --features "model-version-ducklake" \
  --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo check --bin aios-database \
  --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

aios-database model-version migrate --project AvevaMarineSample --json
  schema_migration_count=5
  required_schema_migrations=
    0001_base_model_version_schema,
    0002_release_lifecycle_quality_columns,
    0003_release_quality_evidence_columns,
    0004_release_provenance_columns,
    0005_release_status_lifecycle_quality_backfill
  missing_schema_migrations=[]

same command repeated:
  schema_migration_count=5
  missing_schema_migrations=[]
```

CLI regression evidence:

```text
model-version list --project AvevaMarineSample --json
  release_count=4

model-version diff 791 -> 897 --json
  added=5059
  deleted=2525
  changed=43
  unchanged=23549
  emitted=200

model-version unit-diff 791 -> 897 --json
  added=91
  deleted=17
  changed=119
  unchanged=548
  emitted=200

model-version diff 897 -> 897 --json
  added=0
  deleted=0
  changed=0
  unchanged=28651
  emitted=0
```

web_server validation:

```text
stopped previous validation web_server:
  pid=27576
  port=3926

started rebuilt web_server:
  pid=65428
  port=3926

GET /api/version
  version=0.3.34
  buildDate=2026-06-20 17:00:54 UTC+8

GET /api/model-version/releases?project=AvevaMarineSample
  success=true
  release_count=4

GET /api/model-version/diff?...791...897
  success=true
  added=5059 changed=43 deleted=2525 unchanged=23549 emitted=200

GET /api/model-version/releases/{791}/runtime-scene?limit=10
  success=true
  quality=quarantined_visual
  flags=mesh_missing_rows_quarantined,spec_info_fallback,spec_info_fallback_unquantified
  component_count=10 geometry_count=10 truncated=true

GET /api/model-version/releases/{897}/runtime-scene?limit=10
  success=true
  quality=quarantined_visual
  flags=mesh_missing_rows_quarantined
  component_count=10 geometry_count=2 truncated=true
```

Browser evidence:

```text
page:
  http://127.0.0.1:3926/model-version/compare?from=codex-ams1112-physical-791-quarantine&to=codex-ams1112-physical-897-quarantine

screenshot:
  .planning/2026-06-17-ducklake-valv-version-diff/
    model-version-compare-791-897-required-migration-ids-agent-browser.png

observed:
  two WebGL panes loaded with release-local models
  both panes show quarantined_visual badges
  diff cards show Added=5059 Deleted=2525 Changed=43 Unchanged=23549 Emitted=200

final browser note:
  after the final web_server rebuild to pid=65428, HTTP smoke was repeated and
  passed. A follow-up attempt to recapture the same screenshot hit an
  agent-browser daemon EOF/timeout, so the screenshot file remains the earlier
  successful capture of the same compare URL and release pair. No UI/source
  behavior changed between the capture and the final rebuild; only migration id
  string constants were deduplicated.
```

Remaining risks:

- Migration id insertion is still app-level idempotent; concurrent writers need
  a single-writer queue or server catalog.
- The missing-status-as-published compatibility behavior still needs an
  explicit removal/backfill plan.
- Publish attempt/reconcile, richer quarantine/validation report hashes, and
  paged/tiled two-pane comparison remain open P0/P1 items for the larger goal.

## 2026-06-20 17:15 - Publish Input Safety And Provenance Ordering

SigMap note:

```text
sigmap ask "model-version publish release_id path safety baseline_state_manifest_hash without path validation"
  timed out after about 74 seconds

fallback:
  used rg and direct source inspection
```

Implementation:

```text
src/version_management/model_release.rs
  register_model_release()
    validates ids before materialization
    validates baseline state manifest evidence before materialization
    rejects unsafe release package path relationships before materialization

  validate_release_register_request()
    release_id/project_name/branch_id/parent_release_id must be path-safe
    parent_release_id cannot equal release_id

  validate_history_publish_request()
    applies the same id checks to publish-history
    rejects release package destination nested under source/current Parquet
    rejects source/current Parquet nested under release destination

  ensure_release_package_path_boundaries()
    uses lexical absolute path normalization, with case-insensitive component
    comparison on Windows
    still allows idempotent register-from-existing-release-package when source
    and destination are the same existing package directory
```

Build validation:

```text
cargo fmt --check
  passed

cargo build --bin aios-database --features "model-version-ducklake" \
  --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo check --bin aios-database \
  --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only
```

CLI negative/positive validation:

```text
temporary root:
  target/codex-publish-safety

source fixture:
  output/AvevaMarineSample/parquet/1112_phase2_component_diff_fixture

bad release_id:
  exit=1
  message mentions unsafe characters
  release root not created

bad project_name:
  exit=1
  message mentions project_name
  release root not created

bad branch_id:
  exit=1
  message mentions branch_id
  release root not created

baseline_state_manifest_hash without baseline_state_manifest_path:
  exit=1
  message mentions "without a baseline manifest path"
  package directory not created

release_root nested inside source_parquet_dir:
  exit=1
  message mentions "inside source_parquet_dir"
  nested release root not created

publish-history release_root nested inside current_parquet_dir:
  exit=1
  message mentions "inside current_parquet_dir"
  nested release root not created

positive temporary register:
  exit=0
  target/codex-publish-safety/positive-release-root/
    safe-positive-release/parquet/1112/manifest.json exists
```

Real AvevaMarineSample CLI regression:

```text
model-version migrate --project AvevaMarineSample --json
  schema_migration_count=5
  missing_schema_migrations=[]

model-version list --project AvevaMarineSample --json
  release_count=4

model-version diff 791 -> 897 --json
  added=5059
  deleted=2525
  changed=43
  unchanged=23549
  emitted=200

model-version diff 897 -> 897 --json
  added=0
  deleted=0
  changed=0
  unchanged=28651
  emitted=0
```

web_server validation:

```text
stopped previous validation web_server:
  pid=65428
  port=3926

started rebuilt web_server:
  pid=39416
  port=3926

GET /api/version
  version=0.3.34
  buildDate=2026-06-20 17:14:36 UTC+8

GET /api/model-version/releases?project=AvevaMarineSample
  success=true
  release_count=4

GET /api/model-version/diff?...791...897
  success=true
  added=5059 changed=43 deleted=2525 unchanged=23549 emitted=200

GET /api/model-version/releases/{791}/runtime-scene?limit=10
  success=true
  quality=quarantined_visual
  component_count=10

GET /api/model-version/releases/{897}/runtime-scene?limit=10
  success=true
  quality=quarantined_visual
  component_count=10
```

Self-review:

```text
aligned with Goal:
  yes, this closes a writer-path safety risk before further incremental
  generation/publish automation.

not complete for overall Goal:
  publish attempt/reconcile, writer concurrency, richer validation evidence,
  and production-scale two-pane compare remain open.
```

## 2026-06-20 17:45 - Release Events And Reconcile Diagnostics

SigMap note:

```text
sigmap ask "model version publish attempt events reconcile release crash recovery DuckLake model_release ducklake_store"
  timed out after about 94 seconds

mcp__sigmap query_context for the same topic returned mostly old worktree docs,
not enough to guide current implementation.

fallback:
  used rg and direct source inspection in src/version_management and
  src/web_api/model_version_api.rs.
```

Implementation:

```text
src/version_management/types.rs
  ModelReleaseStatusEvent
  ModelReleaseEventsResponse
  ModelReleaseReconcileReport

src/version_management/ducklake_store.rs
  release_events()
  reconcile_release()
  list_release_status_events()
  feature-disabled stubs for both new methods

src/version_management/model_release.rs
  get_model_release_events()
  reconcile_model_release()

src/version_management/cli.rs
  model-version release-events
  model-version reconcile-release

src/web_api/model_version_api.rs
  GET  /api/model-version/releases/{release_id}/events
  POST /api/model-version/releases/{release_id}/reconcile

src/web_api/mod.rs
  route inventory updated
```

Reconcile behavior:

```text
default:
  read/diagnose only, no status mutation

--publish-if-complete / publish_if_complete=true:
  if evidence has no blocking problems and lifecycle is not published, mark
  release as published

--fail-if-unusable / fail_if_unusable=true:
  if evidence has blocking problems and lifecycle is not failed, mark release
  as failed

evidence checked:
  immutable package dir
  package manifest
  required package files
  component index presence/staleness
  mesh asset index presence/missing/release-local evidence/hash
  unit index presence as warning
  status events
```

Build validation:

```text
cargo fmt
cargo fmt --check
  passed

cargo build --bin aios-database --features "model-version-ducklake" \
  --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo check --bin aios-database \
  --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo build --bin web_server --features "web_server,model-version-ducklake" \
  --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only
```

Build/runtime notes:

```text
new D: target attempt:
  target/codex-web-reconcile-build failed in libduckdb-sys C++ compilation
  during full new-target build

existing D: target attempt:
  failed with os error 112, disk full

cleanup:
  verified target/codex-web-reconcile-build resolved inside workspace
  removed only that temporary target

final build:
  used E:\codex-targets\plant-cli-ducklake-build successfully
```

CLI validation:

```text
model-version release-events --release-id codex-ams1112-physical-791-quarantine --json
  status=published
  lifecycle=published
  event_count=5

model-version reconcile-release --release-id codex-ams1112-physical-791-quarantine --json
  previous_status=published
  current_status=published
  publishable=true
  applied=false
  action_taken=none
  problem_count=0
  warning_count=0
  event_count=5

model-version reconcile-release --release-id codex-ams1112-physical-897-quarantine --json
  previous_status=published
  current_status=published
  publishable=true
  applied=false
  action_taken=none
  problem_count=0
  warning_count=0
  event_count=5

model-version diff 791 -> 897 --json
  added=5059
  deleted=2525
  changed=43
  unchanged=23549
  emitted=200
```

Concurrency observation:

```text
Two reconcile commands run in parallel hit a DuckLake metadata file lock:
  Cannot open metadata.ducklake: another program is using this file

Decision:
  treat this as useful evidence, not a test failure. Production still needs a
  single-writer queue or server catalog. All subsequent writer validation was
  run sequentially.
```

web_server validation:

```text
started rebuilt web_server:
  pid=68488
  url=http://127.0.0.1:3100
  buildDate=2026-06-20 17:39:04 UTC+8

GET /api/model-version/releases/{791}/events?project=AvevaMarineSample
  success=true
  event_count=5

POST /api/model-version/releases/{791}/reconcile?project=AvevaMarineSample
  success=true
  publishable=true
  applied=false
  problem_count=0

GET /api/model-version/diff?...791...897
  success=true
  added=5059 changed=43 deleted=2525 unchanged=23549 emitted=200

GET /api/model-version/releases/{897}/runtime-scene?project=AvevaMarineSample&limit=10
  success=true
  quality=quarantined_visual
  component_count=10
```

Browser evidence:

```text
page:
  http://127.0.0.1:3100/model-version/compare?from=codex-ams1112-physical-791-quarantine&to=codex-ams1112-physical-897-quarantine

screenshot:
  .planning/2026-06-17-ducklake-valv-version-diff/
    model-version-compare-791-897-reconcile-events-agent-browser.png

observed:
  two WebGL panes loaded
  both release panes show quarantined_visual badges
  diff cards show Added=5059 Deleted=2525 Changed=43 Unchanged=23549 Emitted=200
```

Self-review:

```text
aligned with Goal:
  yes, this removes a production handoff gap by making publish lifecycle
  events and reconcile evidence available through CLI and HTTP.

not complete for overall Goal:
  writer queue/server catalog, richer validation reports, GLB readability
  verification, and production-scale tiled compare remain open.
```

## 2026-06-20 18:05 - Local DuckLake Catalog Access Serialization

Scope:

- Continue the Oracle-assisted architecture review request.
- Close the immediate local DuckLake metadata lock failure observed when
  read-only release-events and writer reconcile operations overlapped.
- Keep the fix conservative and local to `src/version_management/ducklake_store.rs`.

Discovery:

```text
sigmap ask "DuckLake metadata file lock open_readonly open_writer MetadataFileLock concurrent readers writers"
  timed out after roughly 124 seconds

mcp__sigmap.get_impact("src/version_management/ducklake_store.rs")
  direct importer: src/version_management/mod.rs
  transitive importer: src/lib.rs

Oracle MCP:
  oracle --help: succeeded
  broad MCP dry-run: ~367,780 tokens, too large
  narrowed MCP dry-run: ~36,824 tokens, acceptable
  live browser consult: failed because the local Oracle Chrome profile had no
    ChatGPT cookies and the model selector could not be found
  no API-cost Oracle run was started
```

Architecture decision:

```text
Short term:
  DuckLake remains the model-version catalog for release metadata, indexes,
  events, diff, and reconcile.
  Immutable Parquet/JSON release packages remain the durable data plane.
  Local DuckLake file catalog access is serialized for both read-only and
  writer opens.

Long term:
  Use a single-writer queue and/or service/server catalog for production
  multi-user deployments.
  Local sidecar locking is a bridge for deterministic single-workstation and
  CI-style validation, not the final distributed concurrency model.
```

Implementation:

```text
src/version_management/ducklake_store.rs
  ModelVersionDuckLakeStore::open_inner now acquires MetadataFileLock for
  both writer and read-only modes before DuckLake ATTACH.

  Error context now includes:
    acquire DuckLake metadata access lock for {writer|read-only} open failed
    metadata path
```

Build/format validation:

```text
cargo fmt
cargo fmt --check
  passed

cargo build --bin aios-database --features "model-version-ducklake" \
  --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo check --bin aios-database \
  --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo build --bin web_server --features "web_server,model-version-ducklake" \
  --target-dir E:\codex-targets\plant-cli-ducklake-build
  first attempt failed with os error 5 because the old web_server.exe was
  still running

verified and stopped only:
  pid=68488
  port=3100
  path=E:\codex-targets\plant-cli-ducklake-build\debug\web_server.exe

second web_server build:
  passed with existing pdms-io warnings only
```

CLI validation:

```text
sequential release-events/reconcile:
  791 event_count=5 status=published lifecycle=published
  791 reconcile publishable=true applied=false problems=0 warnings=0
  897 event_count=5 status=published lifecycle=published
  897 reconcile publishable=true applied=false problems=0 warnings=0

parallel CLI read/write jobs:
  events-791       exit=0 elapsed_ms=895
  reconcile-897    exit=0 elapsed_ms=4597 publishable=true
  events-897       exit=0 elapsed_ms=2820
  reconcile-791    exit=0 elapsed_ms=7097 publishable=true
  events-791-b     exit=0 elapsed_ms=5211
  reconcile-897-b  exit=0 elapsed_ms=1594 publishable=true

component diff 791 -> 897:
  added=5059
  deleted=2525
  changed=43
  unchanged=23549
  emitted=200
```

web_server validation:

```text
started rebuilt web_server:
  pid=38960
  url=http://127.0.0.1:3100
  buildDate=2026-06-20 17:56:25 UTC+8

GET /api/model-version/releases/{791}/events?project=AvevaMarineSample
  success=true
  event_count=5
  last_event_status=published
  release_status=published

POST /api/model-version/releases/{791}/reconcile?project=AvevaMarineSample
  success=true
  publishable=true
  applied=false
  problem_count=0

GET /api/model-version/diff?...791...897
  success=true
  added=5059 changed=43 deleted=2525 unchanged=23549 emitted=200

GET /api/model-version/releases/{897}/runtime-scene?project=AvevaMarineSample&limit=10
  success=true
  quality=quarantined_visual
  component_count=10
  geometry_count=2

parallel HTTP read/write jobs:
  events-791     success=true elapsed_ms=1005
  reconcile-897  success=true elapsed_ms=6347
  diff-791-897   success=true elapsed_ms=1951
  runtime-897    success=true elapsed_ms=2953
  reconcile-791  success=true elapsed_ms=4359
  events-897     success=true elapsed_ms=6771
```

Self-review:

```text
fixed:
  local file-catalog read/write concurrency no longer fails fast with
  metadata.ducklake "another program is using this file" under the validated
  CLI and HTTP scenarios.

tradeoff:
  all local catalog opens are serialized, so a long read can delay a writer.

still open:
  production writer queue/server catalog
  automatic reconcile repair jobs
  validation/quarantine report hashes
  GLB readability/hash verification
  tiled/synchronized production compare UI
```

## Mesh Asset GLB Readability Evidence - 2026-06-20 18:30

### Problem

The release package and mesh asset index already proved that release-local GLB
files existed and had SHA-256 hashes, but they did not prove the GLB payloads
were parseable. That left a production gap: component diff could be correct
while one side of the two-pane 3D comparison failed late in the browser.

### Oracle MCP Review

Oracle was used as a second-opinion architecture review in this slice.

```text
oracle --help
  succeeded

MCP dry run with broad file set
  ~302,687 tokens, too large

MCP dry run with narrowed file set
  ~181,089 tokens, accepted

MCP live consult
  session=e3d-ducklake-architectu-current
  status=completed
```

Oracle's actionable conclusion matched the local design:

```text
DuckLake:
  release registry, snapshot index, diff event index, lineage DAG,
  asset pointers, quality gate status

SurrealDB:
  generation/runtime graph helper and working state

Parquet:
  replay/debug/package data plane

Mesh files:
  release-local immutable render artifacts

P0 risk:
  diff can be correct while visual output is wrong if runtime-scene falls back
  to global mesh cache or if GLB readability is not fail-closed.
```

### Implementation

```text
src/version_management/types.rs
  ModelReleaseMeshAsset:
    glb_readable
    glb_validation_error

  ModelReleaseMeshAssetIndexStats:
    glb_checked_count
    glb_readable_count
    glb_unreadable_count

src/version_management/ducklake_store.rs
  migration:
    0006_mesh_asset_glb_readability_columns

  model_release_mesh_assets:
    glb_readable BOOLEAN
    glb_validation_error TEXT

  model_release_mesh_asset_index_runs:
    glb_checked_count BIGINT
    glb_readable_count BIGINT
    glb_unreadable_count BIGINT

  index_release_mesh_assets:
    prefer release-local immutable mesh paths
    validate GLB parseability with gltf::import
    require at least one mesh primitive and non-empty POSITION accessor
    compact validation errors to 500 chars

  reconcile_release:
    block unreadable GLB assets
    block missing readability evidence
    block checked_count != present_count

  require_release_mesh_assets_ready:
    runtime-scene fails closed before returning a visual scene when readability
    evidence is missing or unreadable.

src/version_management/model_release.rs
  publish/index readiness rejects unreadable or missing GLB readability evidence.

src/version_management/cli.rs
  index-assets non-JSON output now prints GLB checked/readable/unreadable counts.
```

### Build And CLI Validation

No `cargo test` was run.

```text
cargo fmt --check
  passed

cargo build --bin aios-database --features "model-version-ducklake" \
  --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo check --bin aios-database \
  --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo build --bin web_server --features "web_server,model-version-ducklake" \
  --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed after stopping old web_server PID 38960 on port 3100
```

DuckLake migration:

```text
model-version migrate --project AvevaMarineSample --json
  schema_migration_count=6
  required_schema_migrations includes 0006_mesh_asset_glb_readability_columns
  missing_schema_migrations=[]
  migrated=true
```

Release-local asset indexes:

```text
791:
  geo_hash_count=1192
  present_count=1192
  missing_count=0
  glb_checked_count=1192
  glb_readable_count=1192
  glb_unreadable_count=0
  asset_index_hash=1ce4eb4a21c4f448d71b45ae26c712767b1d8ca46385ac7bab23b0fbeafb4bb9

897:
  geo_hash_count=1303
  present_count=1303
  missing_count=0
  glb_checked_count=1303
  glb_readable_count=1303
  glb_unreadable_count=0
  asset_index_hash=4ae6ce2a27aa254e84f4b37af50a841befece45126f4599b1c03602c94ad0059
```

Reconcile:

```text
791:
  publishable=true
  problems=0
  glb_checked=1192
  present=1192
  glb_unreadable=0

897:
  publishable=true
  problems=0
  glb_checked=1303
  present=1303
  glb_unreadable=0

negative read-only check:
  ams-1112-sesno-897-phase1
  publishable=false
  problem="mesh asset index lacks GLB readability evidence; rerun index-assets --materialize with this build"
  glb_checked_count=null
```

Component diff regression:

```text
791 -> 897:
  added=5059
  deleted=2525
  changed=43
  unchanged=23549
  emitted=200
```

### HTTP And Browser Validation

Rebuilt server:

```text
pid=43684
url=http://127.0.0.1:3100
buildDate=2026-06-20 18:15:38 UTC+8
```

HTTP evidence:

```text
POST /api/model-version/releases/{791}/reconcile?project=AvevaMarineSample
  success=true publishable=true problems=0 glb_unreadable=0 glb_checked=1192 present=1192

POST /api/model-version/releases/{897}/reconcile?project=AvevaMarineSample
  success=true publishable=true problems=0 glb_unreadable=0 glb_checked=1303 present=1303

GET /api/model-version/releases/{897}/mesh-assets?project=AvevaMarineSample&limit=3
  success=true returned=3
  stats.glb_checked_count=1303
  stats.glb_readable_count=1303
  stats.glb_unreadable_count=0
  first_glb_readable=true
  first_mesh_relative_path=meshes/lod_L1/10000065025527072764_L1.glb

GET /api/model-version/releases/{897}/runtime-scene?project=AvevaMarineSample&limit=10
  success=true
  release=codex-ams1112-physical-897-quarantine
  quality=quarantined_visual
  component_count=10
  row_geo_instances=28496
  mesh_base_url=/files/output/AvevaMarineSample/model_versions/releases/codex-ams1112-physical-897-quarantine/meshes/lod_L1

GET /api/model-version/diff?...791...897
  success=true
  added=5059 deleted=2525 changed=43 unchanged=23549 emitted=200
```

Browser evidence:

```text
page:
  http://127.0.0.1:3100/model-version/compare?from=codex-ams1112-physical-791-quarantine&to=codex-ams1112-physical-897-quarantine

iframe introspection:
  from model:
    codex-ams1112-physical-791-quarantine
    components 2000
    geometries 2288/2288
    failed 0
    canvasCount 3

  to model:
    codex-ams1112-physical-897-quarantine
    components 2000
    geometries 2041/2041
    failed 0
    canvasCount 3

  diffRows=200

screenshot:
  .planning/2026-06-17-ducklake-valv-version-diff/
    model-version-compare-791-897-glb-readability-agent-browser.png
```

### Review Notes

- The current version now fails closed before runtime-scene for visual releases
  whose release-local mesh asset index has unreadable GLBs or lacks readability
  evidence.
- GLB validation is intentionally done once at indexing time, then persisted in
  DuckLake/manifest for cheap reconcile/API reads.
- Existing older releases may need explicit `index-assets --materialize` with
  the current binary before they satisfy the stricter gate.
- Remaining production hardening: explicit GPU/drawability evidence, stronger
  component-to-mesh lineage, single-writer orchestration, automatic reconcile
  repair jobs, and tiled/synchronized compare UI.

## Paged Runtime Scene Loading - 2026-06-20

### Why This Slice

The DB1112 791/897 browser compare is real, but it still relies on a bounded
single runtime-scene payload. Full-site comparison needs deterministic paging so
the browser can continue loading release-local GLBs without requesting the whole
site in one response.

### Discovery

```text
sigmap ask "model version mesh asset GLB drawability component mesh lineage runtime scene compare production remaining risk"
  timed out after roughly 124 seconds

rg/source inspection:
  RuntimeSceneQuery only had project + limit
  ModelReleaseSceneResponse only had limit + truncated
  ModelVersionDuckLakeStore::release_scene ordered by refno_u64 and applied only LIMIT
  release-viewer fetched one runtime-scene page and had no append/next-page path
```

### Architecture Decision

```text
Backend owns page order:
  ORDER BY refno_u64

Page unit:
  component rows, not geometry rows

Compatibility:
  limit-only callers keep offset=0 default behavior

Response metadata:
  offset
  next_offset
  total_components
  has_more
```

### Success Criteria

```text
CLI/build:
  no cargo test
  cargo fmt --check passes
  aios-database/web_server builds pass with existing warnings only

HTTP:
  runtime-scene limit=10 offset=0 returns offset=0 next_offset=10 has_more=true
  runtime-scene limit=10 offset=10 returns offset=10 and a different first refno
  default no-offset request remains compatible

Browser:
  compare page loads both iframes
  both iframes expose initial geometry counts
  Load more in each iframe appends at least one additional page
  failed geometry count remains 0 for the validated pages
```

### Implementation

Code changes:

```text
src/version_management/types.rs
  ModelReleaseSceneResponse now includes total_components, offset,
  next_offset, and has_more.

src/version_management/ducklake_store.rs
  release_scene now accepts offset, applies ORDER BY refno_u64 LIMIT/OFFSET,
  reports total_components from release row-count metadata, and returns
  deterministic next_offset/has_more metadata.

src/version_management/model_release.rs
  get_model_release_scene passes offset through to the DuckLake store.

src/web_api/model_version_api.rs
  runtime-scene accepts offset.
  release-viewer loads the first page and appends later pages with Load more.
  compare page passes viewer_limit into both release-viewer iframes.
  Load more sets pageLoading before fetching the next page to prevent duplicate
  same-offset requests from rapid clicks.
```

### Validation

No `cargo test` was run.

Build/check:

```text
cargo fmt
  passed

cargo build --bin aios-database --features "model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo build --bin web_server --features "web_server,model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo check --bin aios-database --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only
```

Rebuilt server:

```text
pid=3792
url=http://127.0.0.1:3100
buildDate=2026-06-20 18:54:59 UTC+8
```

HTTP pagination evidence:

```text
GET /api/model-version/releases/{897}/runtime-scene?project=AvevaMarineSample&limit=10&offset=0
  success=true offset=0 limit=10 next_offset=10 has_more=true
  total_components=28651 component_count=10 geometry_count=2
  first_refno=9304_0

GET /api/model-version/releases/{897}/runtime-scene?project=AvevaMarineSample&limit=10&offset=10
  success=true offset=10 limit=10 next_offset=20 has_more=true
  total_components=28651 component_count=10 geometry_count=10
  first_refno=17496_72469

GET /api/model-version/releases/{897}/runtime-scene?project=AvevaMarineSample&limit=10
  offset=0 next_offset=10 has_more=true first_refno=9304_0

GET /api/model-version/releases/{897}/runtime-scene?project=AvevaMarineSample&limit=10&offset=999999
  success=true offset=999999 next_offset=null has_more=false
  total_components=28651 component_count=0 geometry_count=0

GET /api/model-version/releases/{791}/runtime-scene?project=AvevaMarineSample&limit=10&offset=0
  success=true offset=0 next_offset=10 has_more=true
  total_components=26117 component_count=10 geometry_count=10
  first_refno=17496_72443

GET /api/model-version/releases/{791}/runtime-scene?project=AvevaMarineSample&limit=10&offset=10
  success=true offset=10 next_offset=20 has_more=true
  total_components=26117 component_count=10 geometry_count=10
  first_refno=17496_72625

GET /api/model-version/releases/{897}/runtime-scene?project=AvevaMarineSample&limit=0&offset=0
  success=true limit=1
  note: handler clamps limit=0 to limit=1 for compatibility
```

Browser evidence:

```text
page:
  http://127.0.0.1:3100/model-version/compare?from=codex-ams1112-physical-791-quarantine&to=codex-ams1112-physical-897-quarantine&viewer_limit=10

initial iframe introspection:
  from model:
    loadedComponents=10 totalComponents=26117
    loadedGeometries=10 expectedGeometries=10 failedGeometries=0
    nextOffset=10 hasMore=true pageLoading=false canvasCount=3

  to model:
    loadedComponents=10 totalComponents=28651
    loadedGeometries=2 expectedGeometries=2 failedGeometries=0
    nextOffset=10 hasMore=true pageLoading=false canvasCount=3

after clicking Load more in both iframes:
  from model:
    loadedComponents=20 totalComponents=26117
    loadedGeometries=20 expectedGeometries=20 failedGeometries=0
    nextOffset=20 hasMore=true pageLoading=false pageCount=2 canvasCount=3

  to model:
    loadedComponents=20 totalComponents=28651
    loadedGeometries=12 expectedGeometries=12 failedGeometries=0
    nextOffset=20 hasMore=true pageLoading=false pageCount=2 canvasCount=3

  diffRows=200

screenshot:
  .planning/2026-06-17-ducklake-valv-version-diff/
    model-version-compare-791-897-paged-runtime-scene-agent-browser.png
```

### Review Notes

- The paging contract is deterministic and compatible with existing
  `limit`-only callers.
- The browser append path de-duplicates component keys before loading GLBs,
  so repeated page responses cannot duplicate scene objects.
- This is still component-row paging, not spatial tiling. Production-scale
  compare still needs tiled/bbox filtering, synchronized camera/selection, and
  diff-row-to-render-object highlighting.

## Diff Row Selection And Viewer Highlight - 2026-06-20

### Why This Slice

The current compare page proves that two release panes can load DB1112 geometry
and append pages, but it is not yet operator-actionable: the diff rows are
passive text. A production comparison workflow needs the user to click a diff
row and immediately locate/highlight that component in the loaded release
viewers.

### Architecture Decision

```text
release-viewer iframe:
  owns xeokit model ids, object ids, camera, AABB, and highlight state
  exposes window.__MODEL_VERSION_SELECT_COMPONENT(componentKey, options)
  returns found/loaded status evidence for browser tests and compare UI

compare page:
  owns diff row selection
  passes component_key to both iframes
  displays per-pane selection status

scope:
  select already-loaded components only
  do not auto-page through the full site yet
```

### Success Criteria

```text
Browser:
  click a visible diff row
  selected table row is marked
  both iframes receive the same component_key
  loaded side highlights/focuses the object and reports found=true
  missing/unloaded side reports found=false without throwing

Validation:
  no cargo test
  rebuilt web_server + browser automation
  screenshot/eval evidence recorded after implementation
```

### Implementation Delta

The first design note said selection would cover only already-loaded
components. Browser validation with `viewer_limit=10` showed that the first
`changed` rows were outside the initial release-viewer page, so the
implementation now uses a bounded targeted-load path instead of full auto
paging:

```text
compare row click
  -> component_key
  -> from iframe __MODEL_VERSION_SELECT_COMPONENT
  -> to iframe __MODEL_VERSION_SELECT_COMPONENT
  -> if missing locally, iframe calls runtime-scene?component_key=<key>&limit=1
  -> append that single immutable-release component
  -> highlight/focus and return selection evidence
```

Backend/API changes:

```text
GET /api/model-version/releases/{release_id}/runtime-scene
  query: component_key=<dbnum:refno_u64>
  returns: one matching component window, has_more=false
```

Release viewer changes:

```text
window.__MODEL_VERSION_SELECT_COMPONENT(componentKey, options)
window.__MODEL_VERSION_CLEAR_SELECTION()
window.__MODEL_VERSION_COMPONENT_INDEX

body datasets:
  selectedComponentKey
  selectionFound
  selectedModelCount
```

Compare page changes:

```text
diff rows are selectable/clickable
selected row is highlighted
selection status reports from/to found state
window.__MODEL_VERSION_SELECTED_DIFF_ROW records row + pane results
```

### Validation

No `cargo test` was run.

Build and formatting:

```text
cargo fmt --check
  passed

cargo build --bin aios-database --features "model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo build --bin web_server --features "web_server,model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo check --bin aios-database --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only
```

Rebuilt server:

```text
pid=64892
url=http://127.0.0.1:3100
buildDate=2026-06-20 19:17:06 UTC+8
```

HTTP evidence:

```text
GET /api/model-version/diff?project=AvevaMarineSample&from_release_id=codex-ams1112-physical-791-quarantine&to_release_id=codex-ams1112-physical-897-quarantine&limit=200
  added=5059 deleted=2525 changed=43 unchanged=23549 emitted=200

GET /api/model-version/releases/{791}/runtime-scene?component_key=1112%3A75144748061191&limit=1
  component_count=1 geometry_count=1 has_more=false

GET /api/model-version/releases/{897}/runtime-scene?component_key=1112%3A75144748061191&limit=1
  component_count=1 geometry_count=0 has_more=false
  note: component exists but has no renderable geometry after quarantine

GET /api/model-version/releases/{791}/runtime-scene?component_key=1112%3A75144748061193&limit=1
  component_count=1 geometry_count=1 has_more=false

GET /api/model-version/releases/{897}/runtime-scene?component_key=1112%3A75144748061193&limit=1
  component_count=1 geometry_count=1 has_more=false
```

Browser evidence:

```text
page:
  http://127.0.0.1:3100/model-version/compare?from=codex-ams1112-physical-791-quarantine&to=codex-ams1112-physical-897-quarantine&viewer_limit=10

filter:
  changeType=changed
  rowCount=43

selected row:
  component_key=1112:75144748061193
  refno=17496_250377
  noun=BOX
  change_type=changed

from iframe:
  selectedComponentKey=1112:75144748061193
  selectionFound=true
  selectedModelCount=1
  loadedComponents=11
  loadedGeometries=11/11
  failedGeometries=0

to iframe:
  selectedComponentKey=1112:75144748061193
  selectionFound=true
  selectedModelCount=1
  loadedComponents=11
  loadedGeometries=3/3
  failedGeometries=0

status:
  changed 17496_250377
  from found 1 geometries
  to found 1 geometries
```

Screenshot:

```text
.planning/2026-06-17-ducklake-valv-version-diff/
  model-version-compare-791-897-diff-selection-agent-browser.png
```

Oracle MCP:

- Re-read completed session `e3d-model-version-ducklake-current`.
- Re-read completed session `e3d-ducklake-architectu-current`.
- Re-read attempted session `e3d-version-arch-followup-core-2`; it returned
  only a ChatGPT error page and was not used as architecture evidence.
- The implementation follows the usable Oracle conclusion: DuckLake remains a
  catalog/index/audit layer; release-local Parquet/GLB packages remain the
  immutable payload; runtime-scene must not silently substitute a different
  release's geometry.

### Review Notes

- The selected changed row proves diff-row-to-render-object mapping works even
  when the target component is outside the initial page.
- Added/deleted rows are expected to report `found=false` on the missing side;
  changed rows can still report `found=false` on one side if quarantine removed
  renderable geometry, as seen for `1112:75144748061191` on the 897 pane.
- This slice does not implement camera synchronization or spatial tiling. Those
  remain production compare hardening items.

## Two-Pane Camera Sync - 2026-06-20

### Why This Slice

The compare page is now operator-actionable for a selected diff row, but the
two 3D panes still require manual camera alignment. This slice makes the
side-by-side visual comparison behave like one comparison workspace instead of
two unrelated viewers.

### Architecture Decision

```text
release-viewer iframe:
  owns xeokit viewer.camera
  exposes get/set camera snapshot APIs
  exposes rounded camera signature for verification

compare page:
  owns both iframes
  owns camera sync checkbox and status
  polls rounded signatures
  propagates the changed snapshot to the opposite pane
```

The sync control is deliberately explicit. It avoids forcing identical camera
state during initial independent scene fitting, while still letting an operator
turn on synchronized inspection when comparing the two releases.

### Success Criteria

```text
Browser:
  open DB1112 791/897 compare page
  enable camera sync
  apply a camera snapshot to one iframe
  opposite iframe receives the same rounded signature
  no geometry load failures are introduced

Validation:
  no cargo test
  rebuilt web_server + HTTP/browser automation
  screenshot/eval evidence recorded after implementation
```

### Implementation

Release viewer API:

```text
window.__MODEL_VERSION_GET_CAMERA()
window.__MODEL_VERSION_SET_CAMERA(snapshot, options)
window.__MODEL_VERSION_GET_CAMERA_SIGNATURE()

body datasets:
  cameraSignature
  cameraSyncSeq
  cameraLastSource
```

Compare page orchestration:

```text
Camera sync checkbox
cameraSyncTick interval
rounded eye/look/up signatures
from -> to propagation
to -> from propagation
window.__MODEL_VERSION_CAMERA_SYNC_STATE for diagnostics
```

The implementation intentionally uses rounded signatures rather than exact
floating-point comparison so xeokit camera drift does not produce endless
sync ping-pong.

### Validation

No `cargo test` was run.

Build and formatting:

```text
cargo fmt --check
  passed

cargo build --bin web_server --features "web_server,model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo build --bin aios-database --features "model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo check --bin aios-database --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only
```

Rebuilt server:

```text
pid=56480
url=http://127.0.0.1:3100
buildDate=2026-06-20 19:34:08 UTC+8
stderr log length=0
```

HTTP evidence:

```text
GET /model-version/compare
  status=200
  contains Camera sync=true
  contains __MODEL_VERSION_GET_CAMERA=true
  contains __MODEL_VERSION_SET_CAMERA=true
  contains cameraSyncTick=true

GET /api/model-version/diff?...change_type=changed&limit=20
  added=5059 deleted=2525 changed=43 unchanged=23549 emitted=20
```

Browser evidence:

```text
page:
  http://127.0.0.1:3100/model-version/compare?from=codex-ams1112-physical-791-quarantine&to=codex-ams1112-physical-897-quarantine&viewer_limit=10

initial wait:
  two iframes loaded
  pageLoading=false
  both expose __MODEL_VERSION_GET_CAMERA and __MODEL_VERSION_SET_CAMERA

left-to-right sync:
  enabled Camera sync
  applied camera snapshot to from iframe
  status=from -> to
  matched=true

right-to-left sync:
  applied camera snapshot to to iframe
  status=to -> from
  matched=true

final state:
  cameraSyncEnabled=true
  cameraSyncStatus=to -> from
  cameraSyncLastSource=to
  from cameraSignature=18476.063,-19176.363,12679.848|5147.793,-2649.309,3616.625|0,0,1
  to cameraSignature=18476.063,-19176.363,12679.848|5147.793,-2649.309,3616.625|0,0,1
  from loadedGeometries=10/10 failed=0
  to loadedGeometries=2/2 failed=0
```

Screenshot:

```text
.planning/2026-06-17-ducklake-valv-version-diff/
  model-version-compare-791-897-camera-sync-agent-browser.png
```

### Review Notes

- Camera sync is explicit, not forced on initial load. This avoids fighting the
  independent first-frame fit of releases with different extents.
- The sync bridge works in both directions and does not require xeokit-specific
  camera event names.
- Remaining full-site compare hardening still includes spatial/tiled queries,
  tombstone/absence visualization for added/deleted rows, and stronger
  component-to-asset lineage.

## Added/Deleted Absence Visualization - 2026-06-20

### Why This Slice

The compare UI can select changed rows and synchronize cameras, but added and
deleted rows still rely on a textual `found=false` result. This is too easy to
misread as a viewer/page-load issue. The missing side needs an explicit visual
absence state.

### Architecture Decision

```text
compare page:
  derives expected presence from diff change_type and pane side
  added   => from absent, to present
  deleted => from present, to absent
  changed => both present

release-viewer iframe:
  attempts targeted component_key lookup as before
  if no component row is returned, shows absence notice
  if component exists but model_count=0, shows no-renderable-geometry notice
```

This slice intentionally does not draw ghost/tombstone geometry. Current diff
rows carry `old_aabb_hash`/`new_aabb_hash`, not full AABB coordinates. Rendering
spatial tombstone boxes needs a later contract extension that carries old/new
AABB coordinates or a tombstone scene payload.

### Success Criteria

```text
Browser:
  select one added row
  from pane shows absent notice
  to pane highlights/focuses renderable geometry
  select one deleted row
  from pane highlights/focuses renderable geometry
  to pane shows absent notice

Validation:
  no cargo test
  rebuilt web_server + HTTP/browser automation
  screenshot/eval evidence recorded after implementation
```

### Implementation

Release-viewer additions:

```text
absence notice overlay:
  Absent in this release
  No renderable geometry

body datasets:
  absenceVisible
  absenceReason
  absenceTitle
  absenceDetail
  selectionReason
```

Compare-page additions:

```text
expectedPresenceForPane(row, prefix)
  added   -> from=false, to=true
  deleted -> from=true,  to=false
  changed -> from=true,  to=true

selection status datasets:
  fromReason / toReason
  fromExpectedPresence / toExpectedPresence

URL parameter:
  diff_limit=<n>
  default remains 200
```

`diff_limit` exists only to make large-diff operator inspection and browser
validation possible without changing the default emitted row count.

### Validation

No `cargo test` was run.

Build and formatting:

```text
cargo fmt --check
  passed

cargo build --bin web_server --features "web_server,model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo build --bin aios-database --features "model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo check --bin aios-database --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only
```

Rebuilt server:

```text
pid=60464
url=http://127.0.0.1:3100
buildDate=2026-06-20 19:55:46 UTC+8
```

Candidate discovery:

```text
Used local duckdb Python package to read immutable release Parquet packages.

added candidate:
  component_key=1112:75144748078198
  refno=17496_267382
  noun=CYLI
  new release geometries=1

deleted candidate:
  component_key=1112:75144747883391
  refno=17496_72575
  noun=FLOOR
  old release geometries=1
```

HTTP evidence:

```text
added candidate:
  791 runtime-scene component_count=0 geometry_count=0
  897 runtime-scene component_count=1 geometry_count=1

deleted candidate:
  791 runtime-scene component_count=1 geometry_count=1
  897 runtime-scene component_count=0 geometry_count=0
```

Browser evidence:

```text
page:
  http://127.0.0.1:3100/model-version/compare?from=codex-ams1112-physical-791-quarantine&to=codex-ams1112-physical-897-quarantine&viewer_limit=10&diff_limit=6000

added row selected:
  component_key=1112:75144748078198
  refno=17496_267382
  noun=CYLI
  from:
    found=false
    expected_presence=false
    reason=component_absent_expected
    absenceVisible=true
    absenceTitle=Absent in this release
  to:
    found=true
    expected_presence=true
    selectedModelCount=1
    absenceVisible=false
    failedGeometries=0

deleted row selected:
  component_key=1112:75144747883391
  refno=17496_72575
  noun=FLOOR
  from:
    found=true
    expected_presence=true
    selectedModelCount=1
    absenceVisible=false
    failedGeometries=0
  to:
    found=false
    expected_presence=false
    reason=component_absent_expected
    absenceVisible=true
    absenceTitle=Absent in this release
```

Screenshots:

```text
.planning/2026-06-17-ducklake-valv-version-diff/
  model-version-compare-791-897-added-absence-agent-browser.png
  model-version-compare-791-897-deleted-absence-agent-browser.png
```

### Review Notes

- The first added-row attempt clicked while iframes were being reloaded after a
  filter change; the iframe fetch was cancelled and returned `Failed to fetch`.
  Retesting after waiting for both iframe APIs and `pageLoading=false` passed.
- The implementation does not pretend AABB hashes are spatial coordinates.
  True ghost/tombstone boxes remain a later API contract extension.
- This slice improves operator clarity without changing release package truth,
  DuckLake schema, or release-local GLB invariants.

## BaselineStateManager Gate - 2026-06-20

### Goal Alignment

This slice starts the P0 architecture item from
`docs/plans/2026-06-20-e3d-incremental-model-version-oracle-mcp-v2.md`:
make baseline state a first-class code gate instead of a loose metadata
convention.

Target:

```text
publish-history
  -> validate replay package
  -> validate baseline state manifest
  -> register immutable release
```

### Current Finding

SigMap was attempted first, as required by project instructions, but timed out
again after roughly two minutes. Direct source inspection found:

- `publish_history_model_release` already rejects patch-only replay packages and
  unsafe current Parquet directories.
- `register_model_release` already extracts optional
  `baseline_state_manifest_path` and `baseline_state_manifest_hash` from
  metadata.
- That extraction was still a local helper in `model_release.rs`; it verified
  file existence and hash, but not project/dbnum/sesno/source DB consistency.

### Decision

Add `src/version_management/baseline_state.rs` and move baseline evidence into a
small manager-style module. The new module should:

- preserve compatibility with existing metadata locations;
- require a path+hash pair when strict publish validation is requested;
- parse `ModelPhysicalBaselineStateManifest`;
- verify `project_name`, `dbnum`, `from_sesno`, replacement DB existence, and
  replacement DB hash;
- reject manifests whose safety checks do not prove an isolated physical
  baseline.

This makes `publish-history` fail before DuckLake registration when the baseline
cannot be proven.

### Implementation

Added:

```text
src/version_management/baseline_state.rs
```

The module now owns baseline evidence extraction and validation:

```text
optional_baseline_state_evidence_from_metadata
required_baseline_state_evidence_from_metadata
validate_baseline_state_evidence
```

`publish_history_model_release` now requires a baseline manifest path+hash pair
before release registration. The strict gate validates:

- manifest version `physical_baseline_state_manifest:v1`;
- `project_name`;
- `dbnum`;
- `from_sesno == source_db_latest_sesno`;
- replacement DB file exists;
- replacement DB SHA-256 still matches the manifest;
- physical baseline safety checks prove isolated project/output/config state.

`register_model_release` remains backward-compatible: baseline evidence is
optional for generic registration, but when present it is parsed and validated
against project/dbnum before being persisted in DuckLake.

### Validation

No `cargo test` was run.

Build and formatting:

```text
cargo fmt --check
  passed

cargo build --bin aios-database --features "model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo build --bin web_server --features "web_server,model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo check --bin aios-database --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only
```

Negative CLI gate:

```text
command:
  aios-database model-version publish-history
    --release-id codex-baseline-gate-negative
    --dbnum 1112
    --source-db-file D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001
    --from-sesno 791
    --to-sesno 897
    --parquet-dir output\AvevaMarineSample\model_versions\releases\codex-ams1112-physical-791-quarantine\parquet\1112
    --current-parquet-dir output\AvevaMarineSample\parquet\1112
    --release-root target\codex-baseline-gate-negative\releases
    --ducklake-metadata target\codex-baseline-gate-negative\metadata.ducklake
    --ducklake-data target\codex-baseline-gate-negative\data
    --materialize-assets
    --mesh-root output\AvevaMarineSample\model_versions\releases\codex-ams1112-physical-791-quarantine\meshes\lod_L1
    --json

exit:
  1

error:
  baseline_missing: publish-history requires baseline_state_manifest_path and
  baseline_state_manifest_hash metadata; prepare a physical baseline snapshot or
  restore a proven baseline release before publishing

post-check:
  target\codex-baseline-gate-negative\metadata.ducklake did not exist
  target\codex-baseline-gate-negative\releases did not exist
```

Positive CLI smoke:

```text
baseline manifest:
  output\AvevaMarineSample\model_versions\physical_baselines\
    codex-ams1112-physical-791-reuse-20260620\baseline_state_manifest.json

baseline manifest hash:
  7b6fbada31126a9a19add6707fb09bbbcc87a64565dc781966c95584de182948

manifest facts:
  manifest_version=physical_baseline_state_manifest:v1
  project_name=AvevaMarineSample
  dbnum=1112
  source_db_latest_sesno=791
  replacement_db_sha256=5ea0c56bef3030f8a450ffd1c136948f1c1581b20b6f55de79ccf0410766e385
```

Temporary publish with valid baseline metadata:

```text
release_id=codex-baseline-gate-positive-full
catalog=target\codex-baseline-gate-positive-full\metadata.ducklake
release_status=published
release_quality=quarantined_visual
baseline_state_manifest_hash=7b6fbada31126a9a19add6707fb09bbbcc87a64565dc781966c95584de182948
component_count=26117
mesh_asset_index:
  geo_hash_count=1192
  present_count=1192
  missing_count=0
  glb_checked_count=1192
  glb_readable_count=1192
  glb_unreadable_count=0
```

The first positive attempt used `meshes\lod_L1` as mesh root and correctly
failed after baseline validation because asset discovery expects the release
`meshes` root. The failed release was marked `failed` with status events
`staged -> validating -> failed`, proving the new gate had already passed before
asset materialization.

Web server validation:

```text
server:
  pid=53096
  url=http://127.0.0.1:3100
  buildDate=2026-06-20 20:29:40 UTC+8

GET /api/version:
  version=0.3.34

GET /api/model-version/releases?project=AvevaMarineSample&dbnum=1112:
  success=true
  release_count=4
  statuses=published,published,published,published
  791 baseline hash=7b6fbada31126a9a19add6707fb09bbbcc87a64565dc781966c95584de182948
  897 baseline hash=a15de8ff2efa6945cbfba7a03b689842319df89fa1c8622f757784bf8b89f4ab

GET /api/model-version/releases/codex-ams1112-physical-791-quarantine/runtime-scene?limit=1:
  success=true
  scene.component_count=1
  release-local mesh_base_url=/files/output/AvevaMarineSample/model_versions/releases/codex-ams1112-physical-791-quarantine/meshes/lod_L1

GET /api/model-version/diff?from=791&to=897&limit=1:
  success=true
  added=5059
  deleted=2525
  changed=43
  unchanged=23549
  emitted=1
```

### Review Notes

- The new gate improves trust before DuckLake mutation: missing baseline
  evidence fails before metadata or package directories are created.
- Generic `register` remains compatible with old/fixture releases, but any
  provided baseline evidence must now be structurally valid.
- HTTP read paths remain read-only and were validated against the rebuilt
  `web_server`.
- Remaining production gap: this does not yet implement automatic baseline
  hydrate/restore or directory monitoring. It turns the existing physical
  baseline manifest into an enforceable publish contract.

## Baseline State Validation CLI - 2026-06-20

### Goal Alignment

Add an operator/CI-facing preflight command for physical baseline manifests so
historical model generation can prove the DB replacement state before a
`publish-history` attempt mutates DuckLake metadata or release directories.

### Design Decision

- Reuse the same `BaselineStateManager` validation path used by
  `publish-history`.
- Treat manifest hash as optional for inspection but mandatory for publication:
  the validation command computes and returns the hash when it is not supplied.
- Keep validation synchronous and file-local; it verifies manifest identity,
  project/dbnum/sesno expectation, replacement DB existence/hash, and snapshot
  safety checks.
- Return machine-readable JSON with `ready=true` only after the exact same
  checks the publish gate depends on have passed.

### Planned Implementation

- Add request/response types for baseline state validation.
- Add `validate_baseline_state_request` in `baseline_state.rs`.
- Add `aios-database model-version validate-baseline-state --json`.
- Validate positive, sesno mismatch, and manifest-hash mismatch cases through
  the CLI, then rebuild the allowed binaries without running `cargo test`.

### Implementation

Added:

```text
aios-database model-version validate-baseline-state
```

Arguments:

```text
--project
--dbnum
--from-sesno
--baseline-state-manifest
--baseline-state-manifest-hash
--json
```

The command returns `ready=true` only after shared baseline validation succeeds.
If `--baseline-state-manifest-hash` is omitted, the command computes and returns
the manifest hash so automation can capture it for the later strict
`publish-history` metadata pair.

### Validation

No `cargo test` was run.

Build and formatting:

```text
cargo fmt --check
  passed

cargo build --bin aios-database --features "model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo check --bin aios-database --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo build --bin web_server --features "web_server,model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only
```

Positive CLI:

```text
command:
  aios-database model-version validate-baseline-state
    --project AvevaMarineSample
    --dbnum 1112
    --from-sesno 791
    --baseline-state-manifest output\AvevaMarineSample\model_versions\physical_baselines\
      codex-ams1112-physical-791-reuse-20260620\baseline_state_manifest.json
    --baseline-state-manifest-hash 7b6fbada31126a9a19add6707fb09bbbcc87a64565dc781966c95584de182948
    --json

result:
  ready=true
  manifest_version=physical_baseline_state_manifest:v1
  source_db_latest_sesno=791
  baseline_state_manifest_hash=7b6fbada31126a9a19add6707fb09bbbcc87a64565dc781966c95584de182948
  replacement_db_sha256=5ea0c56bef3030f8a450ffd1c136948f1c1581b20b6f55de79ccf0410766e385
```

No-hash CLI:

```text
same command without --baseline-state-manifest-hash:
  ready=true
  computed baseline_state_manifest_hash=7b6fbada31126a9a19add6707fb09bbbcc87a64565dc781966c95584de182948
```

Negative CLI:

```text
--from-sesno 790:
  exit=1
  error=baseline_state_manifest sesno mismatch: from_sesno=790 requires baseline latest sesno 790, got 791

--baseline-state-manifest-hash deadbeef:
  exit=1
  error=baseline state manifest hash mismatch ... expected deadbeef, got 7b6fbada31126a9a19add6707fb09bbbcc87a64565dc781966c95584de182948
```

Rebuilt web server:

```text
pid=11768
url=http://127.0.0.1:3100
buildDate=2026-06-20 20:48:12 UTC+8
```

HTTP evidence:

```text
GET /api/version:
  version=0.3.34

GET /api/model-version/releases?project=AvevaMarineSample&dbnum=1112:
  success=true
  release_count=4
  statuses=published,published,published,published

GET /api/model-version/diff?from_release_id=codex-ams1112-physical-791-quarantine&to_release_id=codex-ams1112-physical-897-quarantine&limit=1:
  success=true
  added=5059
  deleted=2525
  changed=43
  unchanged=23549
  emitted=1

GET /api/model-version/releases/codex-ams1112-physical-791-quarantine/runtime-scene?limit=1:
  success=true
  scene.component_count=1
  mesh_base_url=/files/output/AvevaMarineSample/model_versions/releases/codex-ams1112-physical-791-quarantine/meshes/lod_L1
```

### Review Notes

- Validation CLI and `publish-history` now share the same baseline evidence
  checks, keeping operator preflight and publish behavior aligned.
- The command is read-only: it hashes/loads files and does not mutate DuckLake,
  release directories, or the E3D source project.
- This is still a preflight/gating slice. Directory monitoring, automatic
  baseline hydrate/restore, and incremental model generation orchestration
  remain the next P0 implementation steps.

## HTTP Pipeline Baseline State Reuse - 2026-06-20

### Goal Alignment

The structured HTTP runner endpoints are now part of the production path for
long-running DB1112 physical baseline parse and full model generation. They
must not maintain a separate, weaker copy of baseline manifest validation from
the CLI/publish path.

### Planned Implementation

- Replace hand-written baseline manifest parse/hash/project/dbnum checks in
  `POST /api/model-version/runs/parse-baseline` and
  `POST /api/model-version/runs/generate-full-model` builders with the shared
  `BaselineStateManager` validation entrypoint.
- Keep all HTTP-specific path sandbox checks and parse-run dependency checks.
- Preserve response fields and runner behavior so existing UI/API clients stay
  compatible.
- Validate through `web_server` build plus real HTTP calls; do not run
  `cargo test`.

### Expected Value

This makes the long-running backend model generation path and the CLI
publication path fail for the same baseline evidence reasons: manifest hash,
project, dbnum, sesno, replacement DB hash, and physical snapshot safety.

### Implementation Completed

- Added `snapshot_root` to the shared baseline validation response so HTTP
  callers can enforce snapshot sandboxing without reparsing manifest JSON.
- Made `ModelBaselineStateValidationRequest.dbnum` optional for callers that
  want to trust the manifest dbnum, while the CLI still passes an explicit
  dbnum for operator preflight.
- Routed both `parse-baseline` and `generate-full-model` HTTP builders through
  `validate_baseline_state_request`.
- Kept HTTP-only safety checks for `manifest.snapshot_root`, `config_path`,
  `output_root`, and `replacement_db_file` under the selected physical snapshot
  root.
- Preserved parse-run dependency checks for `generate-full-model` and the
  existing response shape for model-version run clients.

### Validation Evidence

```text
cargo fmt --check
  passed

cargo build --bin aios-database --features "model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo build --bin web_server --features "web_server,model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

web_server
  pid=34272
  url=http://127.0.0.1:3100
  buildDate=2026-06-20 21:02:21 UTC+8
```

CLI regression:

```text
aios-database model-version validate-baseline-state
  --project AvevaMarineSample
  --dbnum 1112
  --from-sesno 791
  --baseline-state-manifest output\AvevaMarineSample\model_versions\physical_baselines\
    codex-ams1112-physical-791-reuse-20260620\baseline_state_manifest.json
  --baseline-state-manifest-hash 7b6fbada31126a9a19add6707fb09bbbcc87a64565dc781966c95584de182948
  --json

result:
  ready=true
  source_db_latest_sesno=791
  baseline_state_manifest_hash=7b6fbada31126a9a19add6707fb09bbbcc87a64565dc781966c95584de182948
  replacement_db_sha256=5ea0c56bef3030f8a450ffd1c136948f1c1581b20b6f55de79ccf0410766e385
  snapshot_root=output\AvevaMarineSample\model_versions\physical_baselines\codex-ams1112-physical-791-reuse-20260620
```

HTTP positive parse baseline smoke:

```text
POST /api/model-version/runs/parse-baseline
  run_id=http-baseline-manager-parse-smoke-20260620-2104
  snapshot_id=codex-ams1112-physical-791-reuse-20260620
  dbnum=1112
  timeout_secs=1

response:
  success=true
  kind=parse_baseline
  launch_observed=true
  baseline_state_manifest_hash=7b6fbada31126a9a19add6707fb09bbbcc87a64565dc781966c95584de182948
  source_observation.primary.sha256=5ea0c56bef3030f8a450ffd1c136948f1c1581b20b6f55de79ccf0410766e385

status:
  status=timed_out
  source_db_hash_unchanged=true
  source_db_sha256_before=5ea0c56bef3030f8a450ffd1c136948f1c1581b20b6f55de79ccf0410766e385
  source_db_sha256_after=5ea0c56bef3030f8a450ffd1c136948f1c1581b20b6f55de79ccf0410766e385
  metrics.stage=chunk_pending
```

HTTP negative parse baseline smoke:

```text
POST /api/model-version/runs/parse-baseline with dbnum=9999
  status=400
  error=baseline_state_manifest dbnum mismatch: expected 9999, got 1112
  no run directory created
```

HTTP positive generate full model smoke:

```text
POST /api/model-version/runs/generate-full-model
  run_id=http-baseline-manager-generate-smoke-20260620-2105
  snapshot_id=codex-ams1112-physical-791-reuse-20260620
  dbnum=1112
  allow_incomplete_parse=true
  timeout_secs=1

response:
  success=true
  kind=generate_full_model
  launch_observed=true
  baseline_state_manifest_hash=7b6fbada31126a9a19add6707fb09bbbcc87a64565dc781966c95584de182948
  command includes --regen-model --dbnum 1112 --export-parquet-after-gen

status:
  status=timed_out
  source_db_hash_unchanged=true
  source_db_sha256_before=5ea0c56bef3030f8a450ffd1c136948f1c1581b20b6f55de79ccf0410766e385
  source_db_sha256_after=5ea0c56bef3030f8a450ffd1c136948f1c1581b20b6f55de79ccf0410766e385
  metrics.stage=refresh_pe_transform_batch_saved
```

HTTP negative generate full model smoke:

```text
POST /api/model-version/runs/generate-full-model with dbnum=9999
  status=400
  error=baseline_state_manifest dbnum mismatch: expected 9999, got 1112
  no run directory created
```

Read-path regression:

```text
GET /api/model-version/diff?...791-quarantine...897-quarantine&limit=1
  success=true
  added=5059
  deleted=2525
  changed=43
  unchanged=23549
  emitted=1

GET /api/model-version/releases/codex-ams1112-physical-791-quarantine/runtime-scene?limit=1
  success=true
  component_count=1
  geometry_count=1
  mesh_base_url=/files/output/AvevaMarineSample/model_versions/releases/codex-ams1112-physical-791-quarantine/meshes/lod_L1
```

### Review Notes

- The HTTP runner now shares the same baseline evidence gate as CLI validation
  and release publication.
- A wrong dbnum is rejected before launching any long-running runner process,
  which protects both DuckLake state and generated model output directories.
- The smoke runs intentionally used `timeout_secs=1` to validate launch,
  evidence capture, source hash stability, timeout handling, and cleanup
  without performing a full DB1112 parse/model build.
- No `cargo test` was run, per repository policy.

## Continuation Slice - Source Observation CLI Contract

Why this slice matters:

- Directory monitoring and incremental generation need a shared, auditable
  source-file observation contract before any parse/generation job is launched.
- The existing `SourceObservation` helper was only used inside HTTP run
  builders. Operators and automation did not have a direct CLI preflight that
  could prove DB file identity, latest sesno, SHA-256, and quiet-window
  stability.
- Without this contract, `watch-incremental` can observe a sesno increase but
  cannot hand the model-version pipeline a durable source evidence artifact.

Implementation completed:

- Added `model-version observe-source`.
- Added `ModelSourceObservationResponse` so CLI/automation receives
  `ready_for_increment`, `status`, manifest path/hash, primary file hash,
  latest/resolved sesno, and recommended action.
- The command is read-only with respect to E3D, SurrealDB, model output, and
  DuckLake. It only writes a source observation manifest JSON.
- Source DB resolution supports explicit `--source-db-file`, dbnum lookup from
  `db_index.sqlite` when the binary has `sqlite-index`, and optional
  `--rescan-index` before dbnum lookup.
- Safety gates: source file exists, source file header dbnum matches `--dbnum`,
  latest sesno is readable unless `--resolved-sesno` is supplied, existing
  manifest paths require `--force`, and `--require-stable` can turn unstable
  evidence into a non-zero automation gate.

Validation evidence:

```text
cargo fmt --check
  passed

cargo build --bin aios-database --features "model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only
```

Explicit DB1112 source file observation:

```text
command:
  aios-database -c db_options/DbOption model-version observe-source
    --project AvevaMarineSample
    --dbnum 1112
    --source-db-file D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001
    --observation-id codex-db1112-source-observe-current-20260620
    --manifest-out target\codex-source-observation\db1112-current.json
    --quiescence-window-ms 10
    --force
    --json

result:
  ready_for_increment=true
  status=stable
  resolved_sesno=897
  primary.bytes=99080192
  primary.sha256=70f18c70116f392eae533b75fb8f4043d031a5f049448531cc1dfc43faf7d3c2
  observation_manifest_hash=fe3af0fc7d7a1699d4e88863ee4841daa8df43fc4a416d7d17fd7e0c7e9ef47f
```

db_index-based DB1112 source file observation:

```text
command:
  aios-database -c db_options/DbOption model-version observe-source
    --project AvevaMarineSample
    --dbnum 1112
    --observation-id codex-db1112-source-observe-index-20260620
    --manifest-out target\codex-source-observation\db1112-index.json
    --quiescence-window-ms 0
    --force
    --json

result:
  ready_for_increment=true
  status=stable
  resolved_sesno=897
  source_db_file=D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001
  primary.sha256=70f18c70116f392eae533b75fb8f4043d031a5f049448531cc1dfc43faf7d3c2
```

Automation gate positive path:

```text
same source file with --require-stable:
  exit=0
  ready_for_increment=true
  status=stable
  resolved_sesno=897
```

Negative DB identity path:

```text
same source file with --dbnum 1113:
  exit=1
  error=source DB dbnum mismatch: expected 1113, got 1112
  manifest_exists=false
```

Negative overwrite path:

```text
same manifest path without --force:
  exit=1
  error=source observation manifest already exists; pass --force to overwrite
```

Review notes:

- This slice does not yet replace `watch-incremental`; it creates the
  production evidence unit that watcher/run-trigger code should call before
  parsing or generating.
- The command deliberately does not write DuckLake. Source observation evidence
  is an input artifact; release/catalog mutation remains a later publish/index
  step.
- The next production step should thread this manifest into `incremental-sesno`
  or a model-version run trigger so source hash stability is checked before and
  after parse/generation, not only during manual preflight.
- No `cargo test` was run, per repository policy.

## Continuation Slice - Incremental Sesno Source Observation Gate

Why this slice matters:

- The previous slice made `observe-source` available, but the actual
  `incremental-sesno` parse/save/generate path could still run without proving
  that the source DB file matched the observation evidence.
- A production-grade watcher/run-trigger must fail before parsing if the
  manifest path is replaced, the requested sesno range is outside the observed
  source latest sesno, or the DB file hash changed before the job starts.
- It must also fail after parsing/generation if the source DB file changes
  during the job.

Implementation completed:

- Added `--source-observation-manifest` and
  `--source-observation-manifest-hash` to `incremental-sesno`.
- Added reusable source observation helpers:
  - load and hash a source observation manifest;
  - validate manifest version, project, dbnum, stable quiet-window evidence,
    and sesno range;
  - verify the primary DB file hash against the manifest.
- `run_incremental_sesno_once` now performs:
  - manifest/hash validation before any increment collection;
  - source primary hash verification before parse/save/generate;
  - source primary hash verification after parse/save/generate;
  - JSON summary output with `source_observation.source_hash_unchanged`.
- Current scope deliberately gates exactly one source file/dbnum per manifest.
  Multi-dbnum runs require either a future multi-source observation manifest or
  one supervised job per observed dbnum.

Validation evidence:

```text
cargo fmt --check
  passed

cargo build --bin aios-database --features "model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only
```

DB1112 positive guarded increment:

```text
command:
  aios-database -c db_options/DbOption incremental-sesno
    --file D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001
    --from-sesno 896
    --to-sesno 897
    --source-observation-manifest target\codex-source-observation\db1112-current.json
    --source-observation-manifest-hash fe3af0fc7d7a1699d4e88863ee4841daa8df43fc4a416d7d17fd7e0c7e9ef47f
    --json

result:
  exit=0
  file_count=1
  session_count=1
  element_count=169
  data_persist.sessions=1
  data_persist.pe=169
  data_persist.att=169
  data_persist.deletes=0
  category_counts.total=118
  generation_dbnums=[1112]
  source_observation.resolved_sesno=897
  source_observation.primary_sha256=70f18c70116f392eae533b75fb8f4043d031a5f049448531cc1dfc43faf7d3c2
  source_observation.source_sha256_before=70f18c70116f392eae533b75fb8f4043d031a5f049448531cc1dfc43faf7d3c2
  source_observation.source_sha256_after=70f18c70116f392eae533b75fb8f4043d031a5f049448531cc1dfc43faf7d3c2
  source_observation.source_hash_unchanged=true
```

Negative manifest-hash guard:

```text
same command with --source-observation-manifest-hash deadbeef:
  exit=1
  error=source observation manifest hash mismatch
  parse/save/generate not entered
```

Negative sesno-range guard:

```text
same manifest with --to-sesno 898:
  exit=1
  error=source observation resolved_sesno 897 is older than to_sesno 898
  parse/save/generate not entered
```

Review notes:

- The guarded `incremental-sesno` path now has real before/after source DB hash
  evidence in the CLI JSON summary.
- `watch-incremental` now auto-generates observation manifests in the follow-up
  slice below.
- No `cargo test` was run, per repository policy.

## Continuation Slice - Watch Incremental Source Observation Gate

Why this slice matters:

- A production watcher must not parse and save an increment from a moving E3D
  source file without binding the run to immutable observation evidence.
- The manual `incremental-sesno` gate proved the runtime contract, but
  `watch-incremental` still needed to create one source observation manifest
  per detected update and pass its hash into the guarded parser.
- This keeps the future model-version release chain auditable: detected
  directory change -> source observation -> incremental parse/save -> optional
  model generation -> release publication.

Implementation completed:

- Added `--observation-quiescence-window-ms` to `watch-incremental`
  (`1000` ms default).
- Added `--source-observation-dir` to control where watcher-created manifests
  are written.
- Added a watcher-only helper that writes source observation manifests named
  `watch-db<dbnum>-<from>-to-<to>-<timestamp>.json`.
- `watch-incremental` now passes both the manifest path and manifest hash into
  `run_incremental_sesno_once` for each detected single-dbnum update.
- The guarded parser still validates manifest hash, project, dbnum, sesno
  range, quiet-window stability, and source DB hash before and after the job.

Validation evidence:

```text
cargo fmt --check
  passed

cargo build --bin aios-database --features "model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

git diff --check -- src/main.rs src/version_management/source_observation.rs \
  .planning/2026-06-17-ducklake-valv-version-diff/progress.md \
  .planning/2026-06-17-ducklake-valv-version-diff/GOAL.md
  passed with CRLF warnings only
```

DB1112 watcher positive path:

```text
setup:
  backed up output\AvevaMarineSample\scene_tree\db_index.sqlite
  temporarily changed db_file_index.dbnum=1112 to latest_sesno=896 and a
  mismatching fingerprint so rebuild_from_config(false) would rescan it

command:
  E:\codex-targets\plant-cli-ducklake-build\debug\aios-database.exe
    -c db_options/DbOption
    watch-incremental
    --dbnum 1112
    --once
    --interval-secs 1
    --observation-quiescence-window-ms 0
    --source-observation-dir target\codex-watch-observation\source_observations
    --json

result:
  exit=0
  detected sesno 896 -> 897
  generated manifest:
    target\codex-watch-observation\source_observations\watch-db1112-896-to-897-20260620T141819449Z.json
  manifest_hash=8b59db81d38ea98d21e3571b477b15fc574f63e27c3374d09eefcef1f3090a83
  primary_sha256=70f18c70116f392eae533b75fb8f4043d031a5f049448531cc1dfc43faf7d3c2
  source_observation.source_hash_unchanged=true
  file_count=1
  session_count=1
  element_count=169
  data_persist.sessions=1
  data_persist.pe=169
  data_persist.att=169
  data_persist.deletes=0
  category_counts.total=118
  generation_dbnums=[1112]
  generation_success=null because --generate-model was intentionally omitted

cleanup:
  restored output\AvevaMarineSample\scene_tree\db_index.sqlite from backup
  restored DB1112 row latest_sesno=897 fingerprint=1776960016834979500:99080192
```

Review notes:

- The watcher is now evidence-bound for incremental parse/save. The next
  production slice should decide whether `--generate-model` should immediately
  publish a model-version release or only refresh runtime/generated caches.
- The current watcher contract remains one manifest per detected dbnum update.
  A later multi-db atomic release should group multiple per-db observations into
  a run manifest before publication.
- No `cargo test` was run, per repository policy.

## Continuation Slice - Incremental Generation Publication Handoff

Why this slice matters:

- `watch-incremental` and `incremental-sesno` can now parse/save with source DB
  evidence, and `--generate-model` can regenerate the affected model scope.
- A generated affected-scope Parquet package is not automatically a complete
  user-visible model version. Treating it as complete would recreate the same
  "patch-only release published as full release" risk that Oracle warned about.
- The safe production boundary is therefore: generation writes current mutable
  output; a handoff manifest records the candidate package and exact explicit
  registration argv; registration copies the package into an immutable release
  and marks it `patch_only` unless a full baseline/hydration path proves it is
  complete.

Implementation completed:

- Added `--publication-handoff-dir` to `incremental-sesno` and
  `watch-incremental`.
- Added `--release-id-prefix` to both commands for generated suggested release
  ids.
- `run_incremental_sesno_once` now adds `publication_handoff` to its JSON
  summary.
- When `--generate-model` succeeds and post-generation Parquet export succeeds,
  the handoff builder:
  - loads and validates the candidate Parquet package manifest;
  - records package hash and rows;
  - writes `incremental_publication_handoff:v1` JSON;
  - includes a `model-version register` argv array;
  - forces the suggested registration to `--release-quality patch_only`;
  - adds validation flags `incremental_handoff_affected_scope` and
    `explicit_release_registration_required`.
- The command deliberately does not auto-register a DuckLake release.

Validation evidence:

```text
incremental-sesno --help
  shows --publication-handoff-dir and --release-id-prefix

watch-incremental --help
  shows --publication-handoff-dir and --release-id-prefix

cargo fmt --check
  passed

cargo build --bin aios-database --features "model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only
```

DB1112 guarded incremental generation and handoff:

```text
command:
  E:\codex-targets\plant-cli-ducklake-build\debug\aios-database.exe
    -c db_options/DbOption
    incremental-sesno
    --file D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001
    --from-sesno 896
    --to-sesno 897
    --source-observation-manifest target\codex-source-observation\db1112-current.json
    --source-observation-manifest-hash fe3af0fc7d7a1699d4e88863ee4841daa8df43fc4a416d7d17fd7e0c7e9ef47f
    --generate-model
    --publication-handoff-dir target\codex-publication-handoff
    --release-id-prefix codex-incr
    --json

result:
  exit=0
  source hash checked before/after by incremental-sesno gate
  data_persist.sessions=1
  data_persist.pe=169
  data_persist.att=169
  generation_dbnums=[1112]
  generation_success=true
  post-generation Parquet exported output\AvevaMarineSample\parquet\1112
  candidate package rows:
    instances=106
    geo_instances=163
    transforms=131
    aabb=105
```

Latest handoff manifest evidence:

```text
manifest:
  target\codex-publication-handoff\incremental-db1112-896-to-897-20260620T145043903Z.json

sha256:
  4b19519c725b8cd30c73a5b8d96c78acd9fada95fb7c5abc66dbaeef9464c9fe

manifest_version:
  incremental_publication_handoff:v1

candidate:
  dbnum=1112
  suggested_release_id=codex-incr-db1112-sesno897-pkgf7c3d89c040f
  suggested_release_quality=patch_only
  package_hash=f7c3d89c040fe4cd0892f2b660483de5a36c381f5a672c9b9d437bbb5e6cbbc2
  register_argv contains --release-quality patch_only
  register_argv contains validation flag incremental_handoff_affected_scope
```

Handoff argv execution proof:

```text
ran register_argv from the latest manifest

result:
  exit=0
  registration_status=created
  release_id=codex-incr-db1112-sesno897-pkgf7c3d89c040f
  release_lifecycle=staged
  release_status=staged
  release_quality=patch_only
  validation_flags=[
    incremental_handoff_affected_scope,
    explicit_release_registration_required
  ]
  immutable_package_dir=output\AvevaMarineSample\model_versions\releases\codex-incr-db1112-sesno897-pkgf7c3d89c040f\parquet\1112
```

Review notes:

- This slice resolves the `--generate-model` production decision: generation
  does not publish automatically; it produces a machine-readable explicit
  registration handoff.
- The generated affected-scope package can be registered for audit/debug as a
  `patch_only` release, but it is not a complete visual version until a full
  baseline/hydration path proves completeness.
- During validation, model generation reported missing
  `output\AvevaMarineSample\scene_tree\1112.tree` and fell back through slower
  DB-based paths. Generation still succeeded, but the next hardening slice
  should fix or explicitly validate DB1112 tree index availability before
  calling this production-grade.
- No `cargo test` was run, per repository policy.

## Continuation Analysis - Oracle Hardening Architecture Plan

Why this slice matters:

- The latest implementation made source observation, guarded incremental
  parse/save, watcher evidence, incremental model generation, and publication
  handoff work together for DB `1112`.
- The remaining production design question is no longer whether DuckLake should
  exist. The question is where the model-version boundary lives, and how to
  keep affected-scope generation from being mistaken for a complete visual
  release.
- DB `1112` still lacks
  `output\AvevaMarineSample\scene_tree\1112.tree`. A direct
  `--gen-indextree 1112` validation attempt timed out after about 904 seconds
  without producing the tree file, so implicit tree auto-generation is not an
  acceptable watcher/default path.

Oracle evidence:

- Reattached completed Oracle MCP session
  `e3d-incrementa-ducklake-architectu-core-2`.
- Oracle's completed recommendation remains decisive:
  - DuckLake belongs in the release catalog/index/diff/impact/audit layer.
  - DuckLake must not become the model generation writer in this version.
  - DuckLake must not own GLB/Parquet payload bodies.
  - User-visible versions are immutable `release_id`s backed by package hashes,
    not sesno values or DuckLake snapshot ids.
- A new narrowed Oracle dry-run for the hardening question succeeded at about
  `144,290` tokens.
- The live browser consult was blocked by the local ChatGPT cookie/model
  selector state. No API-cost Oracle run was started.

Architecture decision:

- Keep the current layered architecture:
  source observation -> increment evidence -> baseline state -> SurrealDB
  generation workspace -> immutable release package -> DuckLake indexes ->
  read-only API/viewer.
- Treat `incremental-sesno --generate-model` output as an affected-scope
  generation candidate. It may create a publication handoff, but it must not
  publish a full visual release automatically.
- Default incremental generation may continue in degraded mode when
  `1112.tree` is missing, but the JSON summary and handoff must record explicit
  tree-index evidence and suggested release quality must remain `patch_only`
  or `quarantined`.
- Add an opt-in strict gate such as `--require-tree-index` for production/full
  visual releases. In strict mode, missing `scene_tree/<dbnum>.tree` fails
  before model generation.
- Implement tree-index generation as a bounded explicit job later, not as an
  implicit watcher side effect.

Documentation produced:

- `docs/plans/2026-06-20-e3d-version-ducklake-hardening-architecture-dev-plan.md`

The document includes:

- requirement analysis;
- edge cases by source observation, sesno parsing, baseline, generation,
  release/assets, DuckLake, and API/viewer;
- final layered architecture;
- model data version entities and package layout;
- DuckLake allow/deny boundary;
- DB1112 missing `1112.tree` handling policy;
- recommended file/module structure;
- CLI/API contracts;
- error codes;
- phased development plan;
- CLI/HTTP/browser validation strategy;
- performance and maintainability notes.

Next implementation target:

- Add generation precheck evidence and `--require-tree-index` to
  `incremental-sesno`/`watch-incremental`.
- Include tree-index readiness/degraded evidence in publication handoff
  manifests.
- Validate DB1112 strict failure and default degraded generation via
  `aios-database` CLI JSON.

## Continuation Slice - Generation Tree-Index Evidence Gate

Why this slice matters:

- DB `1112` incremental model generation can currently succeed through slower
  fallback paths even though
  `output\AvevaMarineSample\scene_tree\1112.tree` is absent.
- For production, this must not remain a log-only warning. Operators need a
  strict fail-fast mode for full visual releases and a machine-readable
  degraded evidence record for patch-only handoff validation.
- A previous direct `--gen-indextree 1112` attempt timed out after about 904
  seconds without producing `1112.tree`, so default watcher/incremental paths
  must not implicitly start long tree-generation work.

Implementation completed:

- Added `--require-tree-index` to `incremental-sesno`.
- Added `--require-tree-index` to `watch-incremental` and passed it into the
  shared one-shot runner.
- Added `incremental_tree_index_evidence:v1` summary generation:
  - `ready`;
  - `mode` (`strict_required`, `ready`, or `degraded_allowed`);
  - `scene_tree_dir`;
  - `db_meta_info_file` / `db_meta_info_exists`;
  - `checked_dbnums`;
  - `missing_dbnums`;
  - per-dbnum tree file path/existence/bytes/mtime;
  - operator recommendation.
- Strict mode now fails before calling `gen_all_geos_data` when required tree
  files are missing.
- Default mode continues generation but includes tree-index evidence in:
  - top-level `incremental-sesno --json` summary;
  - publication handoff manifest;
  - candidate register metadata JSON.
- Non-JSON summary now prints `tree_index: ready=<...> mode=<...>
  missing_dbnums=<...>`.

Validation evidence:

```text
sigmap ask "incremental-sesno watch-incremental generation tree index precheck require-tree-index publication handoff"
  timed out after ~154s; continued with direct source inspection.

cargo fmt --check
  passed

cargo build --bin aios-database --features "model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

incremental-sesno --help
  shows --require-tree-index

watch-incremental --help
  shows --require-tree-index

local tree evidence precondition:
  output\AvevaMarineSample\scene_tree\1112.tree exists=false
  output\AvevaMarineSample\scene_tree\db_meta_info.json exists=true
```

Strict DB1112 negative path:

```text
command:
  E:\codex-targets\plant-cli-ducklake-build\debug\aios-database.exe
    -c db_options/DbOption
    incremental-sesno
    --file D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001
    --from-sesno 896
    --to-sesno 897
    --source-observation-manifest target\codex-source-observation\db1112-current.json
    --source-observation-manifest-hash fe3af0fc7d7a1699d4e88863ee4841daa8df43fc4a416d7d17fd7e0c7e9ef47f
    --generate-model
    --require-tree-index
    --json

result:
  exit=1
  error contains tree_index_missing
  error contains output\AvevaMarineSample\scene_tree\1112.tree
  error evidence:
    ready=false
    mode=strict_required
    missing_dbnums=[1112]
    db_meta_info_exists=true
  generation start markers were absent, so gen_all_geos_data was not entered
```

Default DB1112 degraded generation path:

```text
command:
  E:\codex-targets\plant-cli-ducklake-build\debug\aios-database.exe
    -c db_options/DbOption
    incremental-sesno
    --file D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001
    --from-sesno 896
    --to-sesno 897
    --source-observation-manifest target\codex-source-observation\db1112-current.json
    --source-observation-manifest-hash fe3af0fc7d7a1699d4e88863ee4841daa8df43fc4a416d7d17fd7e0c7e9ef47f
    --generate-model
    --publication-handoff-dir target\codex-publication-handoff-tree-evidence
    --release-id-prefix codex-incr-tree
    --json

result:
  exit=0
  summary contains tree_index
  summary contains degraded_allowed
  handoff:
    target\codex-publication-handoff-tree-evidence\incremental-db1112-896-to-897-20260620T155135644Z.json
  handoff sha256:
    96a0ea948b9162e2ab199b252b47a138c1a2f79441fa33a2c572a2d32a71b865
  handoff tree evidence:
    ready=false
    mode=degraded_allowed
    missing_dbnums=[1112]
  candidate:
    suggested_release_id=codex-incr-tree-db1112-sesno897-pkge15b179d3c0b
    suggested_release_quality=patch_only
    package_hash=e15b179d3c0bc5529598c7e1b29ba6bde928054e2dfc0846798a618176288500
    instances.rows=106
    geo_instances.rows=163
  register_argv metadata also contains tree_index.ready=false and
  tree_index.mode=degraded_allowed
  register_argv still contains --release-quality patch_only
```

Review notes:

- This slice deliberately does not generate `1112.tree`; it records the missing
  evidence and gives operators a strict gate.
- Strict mode still runs after parse/save because generation dbnums are derived
  from parsed increment files. It fails before model generation, which is the
  production boundary needed for this slice.
- The default path remains useful for affected-scope handoff/debug, but the
  handoff cannot be mistaken for a complete visual release because quality is
  still `patch_only` and tree degraded evidence travels with the metadata.
- No `cargo test` was run, per repository policy.

## Continuation Slice - Baseline Scene-Tree Readiness Evidence

Why this slice matters:

- `baseline_state_manifest.json` proves the physical snapshot DB hash and path
  containment, but that is not the same as proving the baseline workspace is
  ready for full visual replay.
- DB `1112` currently has baseline DB evidence for the reused 791 snapshot, but
  its isolated baseline output root is missing `scene_tree/1112.tree`.
- Operators need both modes: a non-blocking readiness report for diagnostics and
  a strict fail-closed gate for complete visual release workflows.

Oracle MCP continuation:

- Initial full-context browser consult
  `e3d-ducklake-architectu-review` failed because ChatGPT attachment upload did
  not complete before timeout.
- A compact inline Oracle MCP consult completed successfully:
  - session: `e3d-ducklake-architectu-compact`
  - transcript:
    `C:\Users\dpc\.oracle\sessions\e3d-ducklake-architectu-compact\artifacts\transcript.md`
  - input: 34.4k tokens, output: 1.6k tokens.
- Oracle confirmed the current architecture boundary:
  - SurrealDB workspace is the single truth compute zone.
  - Parquet/GLB release packages are immutable release facts.
  - DuckLake should remain catalog/index/diff/query/audit only, not a model
    generation writer and not payload storage.
  - Source observation, baseline validation, release package integrity, mesh
    availability for `complete_visual`, and release-id/package-hash binding must
    be fail-fast.
  - DuckLake indexes, status events, scene-tree regeneration jobs, and
    component/unit impact graphs may be rebuilt eventually, but publish state
    and package binding must not be eventually consistent.

Implementation completed:

- Extended `ModelBaselineStateValidationRequest` with:
  - `scene_tree_dir`;
  - `require_scene_tree`.
- Extended `ModelBaselineStateValidationResponse` with
  `scene_tree: ModelHistoryReplaySceneTreeEvidence`.
- `validate_baseline_state_request` now infers the baseline scene-tree
  directory as `<baseline output_root>/<project>/scene_tree` unless explicitly
  overridden.
- Default validation reports:
  - scene-tree directory;
  - `scene_tree/<dbnum>.tree` path/existence;
  - `db_meta_info.json` path/existence;
  - strict requirement flag.
- `--require-scene-tree` fails with `baseline_scene_tree_missing` before a
  caller can treat the baseline as full visual ready.
- `model-version validate-baseline-state` now accepts:
  - `--scene-tree-dir`;
  - `--require-scene-tree`.
- Existing HTTP baseline validation call sites compile and pass
  `require_scene_tree=false`, preserving current non-strict behavior.

Validation evidence:

```text
sigmap ask "web_server port config model-version parse-baseline endpoint baseline state validation scene tree"
  timed out after ~124s; continued with rg/direct source inspection.

oracle --help
  passed

Oracle MCP dry-run full context:
  ~376k tokens, too large.

Oracle MCP dry-run reduced context:
  ~159k tokens, browser upload later timed out.

Oracle MCP compact inline run:
  session=e3d-ducklake-architectu-compact
  status=completed

cargo fmt --check
  passed

cargo build --bin aios-database --features "model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo check --bin web_server --features "web_server,model-version-ducklake"
  passed with existing warnings only

cargo build --bin web_server --features "web_server,model-version-ducklake" --target-dir E:\codex-targets\plant-web-baseline-readiness-build
  passed with existing warnings only
```

Default DB1112 baseline readiness:

```text
command:
  E:\codex-targets\plant-cli-ducklake-build\debug\aios-database.exe
    -c db_options/DbOption
    model-version validate-baseline-state
    --project AvevaMarineSample
    --dbnum 1112
    --from-sesno 791
    --baseline-state-manifest output\AvevaMarineSample\model_versions\physical_baselines\codex-ams1112-physical-791-reuse-20260620\baseline_state_manifest.json
    --baseline-state-manifest-hash 7b6fbada31126a9a19add6707fb09bbbcc87a64565dc781966c95584de182948
    --json

result:
  exit=0
  ready=true
  baseline_state_manifest_hash=7b6fbada31126a9a19add6707fb09bbbcc87a64565dc781966c95584de182948
  scene_tree.required=false
  scene_tree.tree_file_exists=false
  scene_tree.db_meta_info_exists=true
  scene_tree.tree_file=output\AvevaMarineSample\model_versions\physical_baselines\codex-ams1112-physical-791-reuse-20260620\output\AvevaMarineSample\scene_tree\1112.tree
  recommended_action says physical snapshot is verifiable but baseline
    scene_tree artifacts are incomplete.
```

Strict DB1112 baseline readiness:

```text
same command with --require-scene-tree:
  exit=1
  error contains baseline_scene_tree_missing
  error contains 1112.tree
```

HTTP validation:

```text
setup:
  started isolated rebuilt web_server on 127.0.0.1:3197 using
  target\codex-web-baseline-readiness\DbOption-codex-web-baseline.toml
  with startup model/spatial generation and Surreal auto-start disabled.

POST /api/model-version/runs/parse-baseline:
  run_id=http-baseline-scene-readiness-smoke-20260620-0031
  snapshot_id=codex-ams1112-physical-791-reuse-20260620
  dbnum=1112
  executable=E:\codex-targets\plant-cli-ducklake-build\debug\aios-database.exe
  timeout_secs=1

result:
  HTTP success=true
  message="parse baseline run started"
  baseline_state_manifest_hash=7b6fbada31126a9a19add6707fb09bbbcc87a64565dc781966c95584de182948
  source_observation.primary.sha256=5ea0c56bef3030f8a450ffd1c136948f1c1581b20b6f55de79ccf0410766e385
  source_observation.quiescence.stable=true
  bounded run status eventually timed_out as expected
  source_db_hash_unchanged=true

GET /api/model-version/runs/http-baseline-scene-readiness-smoke-20260620-0031:
  success=true
  status=timed_out
  metrics.stage=chunk_pending

POST /api/model-version/runs/parse-baseline with dbnum=9999:
  HTTP 400
  message contains "baseline_state_manifest dbnum mismatch: expected 9999, got 1112"

cleanup:
  stopped only the isolated web_server PID 52864
  verified port 3197 closed
  verified no aios-database process remained from the timed-out run
```

Review notes:

- `ready=true` in `validate-baseline-state` still means physical baseline
  readiness. Full visual readiness is now represented by the explicit
  `scene_tree` evidence and strict flag.
- The HTTP route intentionally remains non-strict so existing parse-baseline
  jobs are not broken; full visual publish/replay paths should opt into strict
  scene-tree requirements.
- During HTTP status validation, `GET .../{run_id}?project=...` triggered the
  existing path-safe run-id guard as if the query string were part of the
  extracted path; retrying without the query succeeded because the smoke config
  already points at `AvevaMarineSample`.
- No `cargo test` was run, per repository policy.

## Continuation Slice - Publish-History Scene-Tree Gate

Why this slice matters:

- Oracle session `e3d-ducklake-architectu-compact` called out that baseline
  state and scene-tree readiness must not drift across validation and publish
  paths.
- `validate-history-replay` already supports `scene_tree_dir` and
  `require_scene_tree`, but `publish-history` was still hard-coded to
  non-strict scene-tree validation.
- Complete visual production releases need a fail-fast publish switch, while
  existing quarantined visual releases must remain publishable when operators
  deliberately accept non-strict scene-tree evidence.

Implementation completed:

- Extended `ModelHistoryReleasePublishRequest` with:
  - `scene_tree_dir`;
  - `require_scene_tree`.
- Extended `ModelHistoryReleaseSafetyChecks` with optional scene-tree evidence.
- `publish_history_model_release` now passes the request's scene-tree fields
  into `validate_history_replay_package`.
- Release metadata now records:
  - `history_publish.scene_tree`;
  - `history_publish.scene_tree_required`.
- `model-version publish-history` now accepts:
  - `--scene-tree-dir`;
  - `--require-scene-tree`.
- Non-JSON publish output prints scene-tree readiness when evidence is present.
- Architecture doc updated:
  `docs/plans/2026-06-20-e3d-version-ducklake-hardening-architecture-dev-plan.md`.

Validation evidence:

```text
sigmap ask "publish-history scene tree release gate model version architecture plan"
  timed out after ~129s; continued with rg/direct source inspection.

oracle --help
  passed

oracle session e3d-ducklake-architectu-compact --render
  reattached completed second-model review and confirmed DuckLake/index
  boundary plus scene-tree/publish consistency risk.

cargo fmt --check
  passed

cargo build --bin aios-database --features "model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

model-version publish-history --help
  shows --scene-tree-dir
  shows --require-scene-tree
```

Strict DB1112 publish negative path:

```text
command:
  E:\codex-targets\plant-cli-ducklake-build\debug\aios-database.exe
    -c db_options/DbOption
    model-version publish-history
    --release-id codex-publish-history-require-tree-negative-20260621
    --project AvevaMarineSample
    --dbnum 1112
    --source-db-file D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001
    --from-sesno 791
    --to-sesno 897
    --parquet-dir output\AvevaMarineSample\model_versions\physical_baselines\http-prepare-physical-1112-smallchunk-long-20260620-1113\validation-export-quarantine\1112
    --scene-tree-dir target\codex-publish-history-scene-tree-negative\missing-scene-tree
    --require-scene-tree
    --release-root target\codex-publish-history-scene-tree-negative\releases
    --ducklake-metadata target\codex-publish-history-scene-tree-negative\metadata.ducklake
    --ducklake-data target\codex-publish-history-scene-tree-negative\data
    --json

result:
  command exit code=1
  error contains classification=missing_scene_tree_baseline
  no temp metadata.ducklake created
  no temp releases directory created
  real catalog list does not contain codex-publish-history-require-tree-negative-20260621
```

Default DB1112 replay validation evidence:

```text
model-version validate-history-replay --json
  package=897 validation-export-quarantine
  scene-tree-dir=target\codex-publish-history-scene-tree-negative\missing-scene-tree

result:
  exit=0
  classification=quarantined_visual_release_candidate
  ready_for_publish=true
  scene_tree.required=false
  scene_tree.tree_file_exists=false
  scene_tree.db_meta_info_exists=false
  instances_rows=28651
  geo_instances_rows=28496
  render_missing_mesh_geo_hashes=0
  quarantined_mesh_geo_hashes=23
```

Review notes:

- The strict publish gate now fails before release registration because it is
  part of replay package validation, before baseline metadata validation,
  asset materialization, DuckLake registration, or release directory creation.
- Default publish behavior remains non-strict for existing quarantined visual
  packages, but scene-tree evidence is now visible in safety checks and
  release metadata.
- `git diff --check` over the whole worktree still reports pre-existing
  trailing whitespace in `MEMORY.md`; the same check limited to the touched
  files passes.
- No `cargo test` was run, per repository policy.

## Continuation Slice - Component-To-Mesh Asset Lineage

Why this slice matters:

- The release runtime API already required a release-local mesh asset index, but
  a selected diff row did not yet expose which GLB file(s) were used.
- For production visual diff review, the operator needs an audit path:
  diff row -> component_key -> geometry row -> geo_hash -> release-local GLB
  URL/SHA/readability.
- Without this lineage, a two-pane comparison could look correct while still
  silently loading current/global mesh files instead of immutable release files.

Oracle/SigMap notes:

```text
sigmap ask "E3D incremental model version production remaining risks two pane compare component asset lineage release scene"
  timed out after ~129s; continued with rg/direct source inspection.

oracle --help
  passed

mcp__oracle.consult dryRun=true
  session slug=e3d-ducklake-version-review-mcp
  engine=browser
  model=gpt-5.5-pro
  context ~=96,233 tokens
  attachments=6 focused files

mcp__oracle.consult live browser
  started but stayed in Oracle private Chrome profile
  promptSubmitted=false
  no new Oracle answer produced
  API mode was not used because it can incur cost and no explicit cost approval
  was given
```

Implementation completed:

- Added `ModelReleaseSceneMeshAssetEvidence`.
- Added `mesh_asset: Option<ModelReleaseSceneMeshAssetEvidence>` to
  `ModelReleaseSceneGeometry`.
- `release_scene` now left-joins `model_release_mesh_assets` by
  `release_id + geo_hash` and returns:
  - `mesh_relative_path`;
  - `mesh_absolute_path`;
  - `mesh_url`;
  - `bytes`;
  - `sha256`;
  - `exists`;
  - `builtin`;
  - `glb_readable`;
  - `glb_validation_error`.
- The release viewer records geometry asset evidence in the selected component
  entry and loads GLBs from `geo.mesh_asset.mesh_url` when present.
- The compare page selection status now displays and exposes:
  - `fromAssetCount` / `toAssetCount`;
  - `fromReadableAssetCount` / `toReadableAssetCount`;
  - `fromAssetHashes` / `toAssetHashes`;
  - `fromAssetUrls` / `toAssetUrls`;
  - `fromAssetSha256` / `toAssetSha256`.

Build validation:

```text
cargo fmt --check
  passed

cargo build --bin aios-database --features "model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo build --bin web_server --features "web_server,model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  first attempt was blocked by a running validation web_server.exe
  stopped only that PID and reran
  passed with existing pdms-io warnings only
```

HTTP validation:

```text
web_server
  binary=E:\codex-targets\plant-cli-ducklake-build\debug\web_server.exe
  config=target\codex-web-baseline-readiness\DbOption-codex-web-baseline
  port=3197

GET /api/model-version/diff
  from=codex-ams1112-physical-791-quarantine
  to=codex-ams1112-physical-897-quarantine
  change_type=changed

first changed row:
  component_key=1112:75144748061191
  refno=17496_250375
  noun=PANE
  from asset_count=1
  to geometry_count=0
  interpretation=valid quarantine/no-renderable edge case, not a browser both-pane proof target

both-pane proof row:
  component_key=1112:75144748061193
  refno=17496_250377
  noun=BOX
  from geometry_count=1
  to geometry_count=1
  from asset_count=1
  to asset_count=1
  from mesh_url=/files/output/AvevaMarineSample/model_versions/releases/codex-ams1112-physical-791-quarantine/meshes/lod_L1/1_L1.glb
  to mesh_url=/files/output/AvevaMarineSample/model_versions/releases/codex-ams1112-physical-897-quarantine/meshes/lod_L1/1_L1.glb
```

Browser validation:

```text
URL:
  http://127.0.0.1:3197/model-version/compare?from=codex-ams1112-physical-791-quarantine&to=codex-ams1112-physical-897-quarantine&viewer_limit=10&diff_limit=200

steps:
  set Change=changed
  selected component_key=1112:75144748061193

selection status datasets:
  fromFound=true
  toFound=true
  fromAssetCount=1
  toAssetCount=1
  fromReadableAssetCount=1
  toReadableAssetCount=1
  fromAssetHashes=1
  toAssetHashes=1
  fromAssetUrls=/files/output/AvevaMarineSample/model_versions/releases/codex-ams1112-physical-791-quarantine/meshes/lod_L1/1_L1.glb
  toAssetUrls=/files/output/AvevaMarineSample/model_versions/releases/codex-ams1112-physical-897-quarantine/meshes/lod_L1/1_L1.glb
  fromAssetSha256=0ecd246b587d82f8853559eb951da07ae6b6ea56a35ecef43e6ac11fb95c5701
  toAssetSha256=0ecd246b587d82f8853559eb951da07ae6b6ea56a35ecef43e6ac11fb95c5701

desktop canvas pixel probe:
  from nonTransparent=9 nonBlack=9 uniqueCount=7
  to nonTransparent=9 nonBlack=9 uniqueCount=7

mobile viewport 390x844 pixel probe:
  from nonTransparent=5 nonBlack=5 uniqueCount=5
  to nonTransparent=5 nonBlack=5 uniqueCount=4

browser errors:
  none after mobile validation
```

Screenshots:

```text
.planning/2026-06-17-ducklake-valv-version-diff/model-version-compare-791-897-asset-lineage-agent-browser-full.png
.planning/2026-06-17-ducklake-valv-version-diff/model-version-compare-791-897-asset-lineage-mobile-agent-browser.png
```

Review notes:

- The first changed row proves the UI/API must handle no-renderable/quarantine
  target-side geometry without pretending it is a failed asset lookup.
- The second changed row proves the intended both-pane path: two release-local
  GLB URLs, same SHA, readable on both sides, and nonblank WebGL canvases.
- `release_scene` uses a left join so geometry remains visible even when asset
  evidence is missing, but runtime-scene still calls the existing
  `require_release_mesh_assets_ready` gate before query execution.
- `replace_mesh_asset_index` deletes existing asset rows for a release before
  inserting, so joining on `release_id + geo_hash` should not amplify scene
  rows during normal reindexing.
- No `cargo test` was run, per repository policy.

## Continuation Slice - Release Pair Production Readiness Gate

Why this slice matters:

- The two-pane DB1112 `791 -> 897` compare now renders and exposes
  release-local asset lineage, but both releases are still marked
  `quarantined_visual`.
- Without an explicit pair-level gate, an operator could mistake a useful
  diagnostic/demo comparison for production sign-off.
- The larger historical baseline hydrate/restore path is still not complete;
  this slice intentionally fails clear instead of pretending quarantine is
  production-ready.

SigMap/Oracle notes:

```text
sigmap ask "E3D model version production remaining P0 gaps DB1112 791 897 incremental generation final review release readiness"
  timed out after about 64s

mcp SigMap query_context for baseline hydrate/restore
  returned unrelated .worktrees/model-persistence-trait results

Oracle
  no new paid/API Oracle run in this slice
  continued from the existing Oracle-driven conclusion that publish/compare
  boundaries must be fail-closed and machine-readable
```

Implementation completed:

- Added `ModelReleaseReadinessEvidence` and
  `ModelReleasePairReadinessResponse`.
- Added `DuckLakeStore::compare_readiness` with per-release checks for:
  lifecycle, quality, validation flags, baseline manifest evidence, component
  index, component snapshot count, mesh asset index, release-local asset
  violations, unit index, problems, warnings, and recommended action.
- Added pair classification:
  `production_ready`, `quarantined_visual`, `incomplete_indexes`,
  `missing_release`, and `not_production_ready`.
- Added facade
  `validate_model_release_pair_readiness`.
- Added CLI:
  `model-version validate-compare-readiness --from-release-id ... --to-release-id ... --json`.
- Added HTTP:
  `GET /api/model-version/compare-readiness`.
- Updated `/model-version/compare` to fetch readiness before diff and render a
  compact readiness status above the two-pane viewer.
- Updated the stateless route print list so startup logs include the new
  read-only endpoint.

CLI validation:

```text
cargo fmt --check
  passed

cargo build --bin aios-database --features "model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo build --bin web_server --features "web_server,model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  first retry was blocked by the running validation web_server.exe
  stopped only that validation PID and reran
  passed with existing pdms-io warnings only

aios-database.exe -c db_options/DbOption model-version validate-compare-readiness --help
  command and arguments are exposed

aios-database.exe -c db_options/DbOption model-version validate-compare-readiness \
  --from-release-id codex-ams1112-physical-791-quarantine \
  --to-release-id codex-ams1112-physical-897-quarantine \
  --json
  classification=quarantined_visual
  production_ready=false
  production_comparison_allowed=false
  both_releases_exist=true
  same_project=true
  same_dbnum=true
  both_published=true
  both_complete_visual=false
  component_indexes_ready=true
  mesh_assets_ready=true
  diff added=5059 deleted=2525 changed=43 unchanged=23549
```

HTTP validation:

```text
web_server
  binary=E:\codex-targets\plant-cli-ducklake-build\debug\web_server.exe
  config=target\codex-web-baseline-readiness\DbOption-codex-web-baseline
  port=3197

GET /api/model-version/compare-readiness
  from_release_id=codex-ams1112-physical-791-quarantine
  to_release_id=codex-ams1112-physical-897-quarantine
  HTTP 200
  success=true
  classification=quarantined_visual
  production_ready=false
  both_published=true
  both_complete_visual=false
  component_indexes_ready=true
  mesh_assets_ready=true
  recommended_action=comparison may be used for diagnosis/demo, but production sign-off requires complete_visual releases with resolved quarantine evidence
```

Browser validation:

```text
URL:
  http://127.0.0.1:3197/model-version/compare?from=codex-ams1112-physical-791-quarantine&to=codex-ams1112-physical-897-quarantine&viewer_limit=10&diff_limit=200

readiness DOM datasets:
  classification=quarantined_visual
  productionReady=false
  componentIndexesReady=true
  meshAssetsReady=true
  height=60

summary:
  Added=5059
  Deleted=2525
  Changed=43
  Unchanged=23549
  Emitted=200

diff table rows:
  200

browser errors:
  none
```

Screenshot:

```text
.planning/2026-06-17-ducklake-valv-version-diff/model-version-compare-791-897-readiness-agent-browser.png
```

Review notes:

- The DB1112 pair is explicitly classified as a diagnostic/quarantine visual
  comparison, not a production release comparison.
- Component indexes and release-local mesh asset indexes are ready, so the
  non-production classification is due to release quality/completeness, not
  missing indexes.
- `compare_readiness` is read-only and does not migrate, index, reconcile, or
  materialize assets.
- The compare page now surfaces readiness before the diff summary, closing the
  operator handoff ambiguity.
- This slice does not solve full historical target-sesno baseline
  hydrate/restore; the overall goal remains active.
- No `cargo test` was run, per repository policy.

## Continuation Slice - Historical Baseline Inspect HTTP Preflight

Why this slice matters:

- The user wants to choose a DB1112 historical record and use it for incremental
  update/model comparison tests.
- The backend already has a CLI-only pdms-io target-sesno inspector, but web/API
  callers could not get the same evidence before launching history replay or
  generation.
- Current DB1112 `791` and `897` sessions can be found exactly, but pdms-io
  index traversal is not sufficient to prove full-state visual baseline hydrate.
- The production-safe next step is therefore a read-only HTTP preflight that
  exposes the evidence and fails clear, not a fake hydrate implementation.

Discovery notes:

```text
sigmap ask "E3D DB1112 historical baseline hydrate restore pdms-io version incremental model generation remaining production gap"
  timed out after about 124s

direct source inspection:
  src/version_management/history_baseline.rs
  src/version_management/history_replay_plan.rs
  src/version_management/cli.rs
  src/web_api/model_version_api.rs
```

CLI evidence before implementation:

```text
aios-database.exe -c db_options/DbOption model-version inspect-history-baseline \
  --source-db-file D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001 \
  --target-sesno 791 \
  --parse-sample-limit 10 \
  --json

result:
  header_dbnum=1112
  requested_sesno=791
  resolved_sesno=791
  latest_sesno=897
  exact_sesno_found=true
  visible_refno_count=5
  index_error_count=1
  parsed_sample_count=3
  parse_error_count=2
  full_state_enumeration_supported=false
  action=target_sesno_index_not_publishable; use physical baseline snapshot,
         restore a published baseline package, or add a proven hydrate provider

same command for target_sesno=897:
  resolved_sesno=897
  exact_sesno_found=true
  visible_refno_count=5
  index_error_count=1
  parse_error_count=2
  full_state_enumeration_supported=false
```

Implementation completed:

- Added read-only route:
  `GET /api/model-version/history-baseline-inspect`.
- Added query contract:
  `project`, `source_db_file`, `target_sesno`, `parse_sample_limit`,
  `allow_nearest_sesno`, and `detail`.
- Added `HistoryBaselineInspectApiData` wrapper.
- Reused `HistoryBaselineInspectRequest` and `inspect_history_baseline`.
- Added a dedicated blocking wrapper for the async pdms-io inspector so large
  file/index reads do not run on the web runtime worker.
- Clamped `parse_sample_limit` with:
  `DEFAULT_HISTORY_BASELINE_SAMPLE_LIMIT=100` and
  `MAX_HISTORY_BASELINE_SAMPLE_LIMIT=1000`.
- Synchronized startup route logging and the architecture verification plan.

Build validation:

```text
cargo fmt
cargo fmt --check
  passed

git diff --check -- src/web_api/model_version_api.rs src/web_api/mod.rs \
  .planning/2026-06-17-ducklake-valv-version-diff/GOAL.md \
  docs/plans/2026-06-20-e3d-version-ducklake-hardening-architecture-dev-plan.md
  passed with CRLF warnings only

cargo build --bin aios-database --features "model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo build --bin web_server --features "web_server,model-version-ducklake" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only
```

HTTP validation:

```text
web_server
  binary=E:\codex-targets\plant-cli-ducklake-build\debug\web_server.exe
  config=target\codex-web-baseline-readiness\DbOption-codex-web-baseline
  port=3197
  pid=58756

startup route list includes:
  GET /api/model-version/history-baseline-inspect

GET /api/model-version/history-baseline-inspect
  source_db_file=D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001
  target_sesno=791
  parse_sample_limit=10

result:
  HTTP 200
  success=true
  resolved_sesno=791
  latest_sesno=897
  exact_sesno_found=true
  full_state_enumeration_supported=false
  visible_refno_count=5
  index_error_count=1
  parse_error_count=2

target_sesno=897:
  HTTP 200
  exact_sesno_found=true
  resolved_sesno=897
  full_state_enumeration_supported=false
  visible_refno_count=5
  index_error_count=1
  parse_error_count=2

target_sesno=999999 without allow_nearest_sesno:
  HTTP 404
  message=requested session 999999 does not exist

target_sesno=999999&allow_nearest_sesno=true&parse_sample_limit=0:
  HTTP 200
  requested_sesno=999999
  resolved_sesno=897
  exact_sesno_found=false
  parsed_sample_count=0

missing source_db_file:
  HTTP 404
  message=source DB file is missing or not a file: ...
```

Review notes:

- The endpoint is intentionally read-only: it does not write DuckLake, start a
  bounded run, mutate DbOption, or materialize model assets.
- The endpoint moves history-version selection closer to production readiness by
  making pdms-io target session evidence available to backend/UI callers.
- The evidence currently contradicts full historical baseline hydrate
  readiness for DB1112 791/897, so production workflow should continue to use
  physical baseline snapshots or a proven future hydrate provider.
- This slice does not implement full-state target-sesno hydrate/restore; the
  overall goal remains active.
- No `cargo test` was run, per repository policy.

## Continuation Slice - Prepare-History Baseline Proof Gate

Why this slice matters:

- `prepare-history-replay` is the command that hands a developer runnable
  baseline/replay DbOptions.
- Before this slice, it could write those configs while only warning that
  `baseline_parse` uses the source DB file's visible/current state.
- For DB1112 791/897, pdms-io inspect currently contradicts target-sesno
  full-state hydrate readiness, so the replay plan must not proceed unless the
  input source file is already a physical baseline for `from_sesno`.

Implementation completed:

- Added `baseline_source_confirmed_at_from_sesno` to:
  - `ModelHistoryReplayPrepareRequest`;
  - `ModelHistoryReplaySafetyChecks`.
- Added CLI flag:
  `--baseline-source-confirmed-at-from-sesno`.
- `prepare_history_replay` now fails before writing replay/baseline config
  files unless that flag/request field is true.
- The failure message explains:
  - current-file full-sync is not pdms-io target-sesno hydrate;
  - the source DB must already be an isolated physical baseline;
  - alternatives are physical baseline snapshot, published baseline restore, or
    a proven hydrate provider.
- Non-JSON CLI output now includes
  `source_confirmed_at_from_sesno`.
- `prepare-physical-baseline-snapshot` hint argv now:
  - points `--source-db-file` to the snapshot replacement DB file;
  - includes `--baseline-source-confirmed-at-from-sesno`.
- Architecture plan now documents the fail-closed default and verification
  expectations.

Build validation:

```text
cargo fmt --check
  passed

git diff --check -- src/version_management/types.rs \
  src/version_management/history_replay_plan.rs \
  src/version_management/cli.rs \
  src/version_management/physical_baseline_snapshot.rs \
  .planning/2026-06-17-ducklake-valv-version-diff/GOAL.md \
  docs/plans/2026-06-20-e3d-version-ducklake-hardening-architecture-dev-plan.md
  passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only
```

CLI validation:

```text
prepare-history-replay --help
  exposes --baseline-source-confirmed-at-from-sesno
  help text says the flag is required because prepare-history-replay does not
  hydrate target sesno from pdms-io history

DB1112 source:
  D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001

without confirmation flag:
  release_id=codex-proof-gate-20260621033048
  from_sesno=791
  to_sesno=897
  replay_config=target\codex-history-proof-gate-20260621033048\DbOption-replay.toml
  baseline_config=target\codex-history-proof-gate-20260621033048\DbOption-baseline.toml
  exit=1
  message contains "requires explicit baseline source confirmation"
  replay config written=false
  baseline config written=false

with confirmation flag:
  exit=0
  written=true
  safety_checks.baseline_source_confirmed_at_from_sesno=true
  safety_checks.baseline_target_sesno_reconstruction_supported=false
  source_db_file=D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001
```

Physical snapshot hint validation:

```text
prepare-physical-baseline-snapshot
  snapshot_id=codex-physical-hint-20260621033212
  dbnum=1112
  source_db_file=D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001
  snapshot_root=target\codex-physical-hint-20260621033212
  exit=0

commands.prepare_history_replay_hint_argv:
  has --baseline-source-confirmed-at-from-sesno=true
  --source-db-file=target\codex-physical-hint-20260621033212\project_path\AvevaMarineSample\ams000\ams1112_0001
  replacement_db_file=target\codex-physical-hint-20260621033212\project_path\AvevaMarineSample\ams000\ams1112_0001
  source arg is replacement=true
  source_db_latest_sesno=897
```

Review notes:

- This is a deliberate breaking safety change for a dangerous default. A caller
  that wants to replay from current DB state must now acknowledge that the file
  is already the physical baseline for `from_sesno`.
- The confirmation flag does not claim pdms-io target-sesno hydrate is solved;
  `baseline_target_sesno_reconstruction_supported` remains false.
- The physical snapshot workflow now produces a safer handoff because its hint
  references the snapshot replacement DB, not the mutable live project file.
- This slice still does not implement full-state target-sesno hydrate/restore;
  the overall goal remains active.
- No `cargo test` was run, per repository policy.

## Continuation Slice - Oracle Model-Version Architecture Review

Why this slice matters:

- The user requested an Oracle-backed review before continuing implementation.
- The next backend slice should not just add another endpoint; it should fit a
  clear model-version architecture that separates source observation,
  baseline state, increment evidence, generation jobs, immutable release
  packages, and DuckLake read-model indexes.

Oracle evidence:

```text
mcp__oracle.consult
  result=failed immediately
  error=Transport closed

oracle CLI browser consult
  session=e3d-model-version-plan-review
  reattach=oracle session e3d-model-version-plan-review
  input_tokens=~94,726
  mode=browser foreground
  model=gpt-5.5-pro / Pro Extended evidence
  api_paid_mode=not used
```

Key findings:

- Model data versions should be constrained as an immutable lineage graph:
  `SOID -> BSID -> IEVID -> GJID -> RID`.
- `release_id + package_hash` is the user-visible model version; `sesno` is an
  anchor/cursor only.
- DuckLake can enter this version as catalog, read-model, component/unit/asset
  index, diff, impact, lineage, and audit storage.
- DuckLake must not be generation writer, baseline restore, GLB/Parquet payload
  body store, session replay logic, or user-facing version identity.
- Current HTTP structured run APIs are still missing a production-grade
  `prepare-history-replay` endpoint, plus later publish/register, incremental
  handoff, and release state machine endpoints.

Documentation completed:

- Updated
  `docs/plans/2026-06-20-e3d-version-ducklake-hardening-architecture-dev-plan.md`
  with:
  - latest Oracle evidence;
  - the `SOID -> BSID -> IEVID -> GJID -> RID` lineage contract;
  - revised final architecture flow;
  - recommended new modules:
    `increment_evidence.rs`, `generation_job.rs`,
    `release_state_machine.rs`, and `publication_handoff.rs`;
  - structured HTTP mutation endpoint plan;
  - revised P0/P0.5 development plan.

Next implementation target:

- Implement `POST /api/model-version/runs/prepare-history-replay`.
- Prefer `snapshot_id` mode so the endpoint uses
  `baseline_state_manifest.json` and the snapshot replacement DB file.
- Automatically pass `--baseline-source-confirmed-at-from-sesno` only for that
  proven physical snapshot path.
- Fail closed for direct source-file requests without explicit physical
  baseline confirmation.
- Validate via `web_server` HTTP and CLI JSON with DB1112 evidence; no
  `cargo test`.

## Continuation Slice - Structured Prepare-History Replay HTTP Run API

Why this slice matters:

- The CLI proof gate already prevented unsafe replay planning from live/current
  DB files.
- Backend callers still needed a structured, bounded-run API that could launch
  the same safe command plan without manually building Windows command lines.
- Oracle's latest architecture review called out `prepare-history-replay` as
  the first missing structured safety API before `publish/register`,
  incremental handoff, and release state machine work.

SigMap notes:

```text
sigmap ask "web_server model-version prepare-history-replay structured run API baseline_state_manifest physical snapshot"
  timed out after about 134s

mcp SigMap query_context
  returned mostly unrelated context, so implementation proceeded from the
  already inspected model_version_api/version-management files
```

Implementation completed:

- Added route:
  `POST /api/model-version/runs/prepare-history-replay`.
- Added `PrepareHistoryReplayRunRequest` and
  `PrepareHistoryReplayRunEvidence`.
- Extended prepared run API data with `history_replay` evidence while keeping
  existing physical snapshot, parse-baseline, and generate-full-model run
  responses compatible.
- Added `build_prepare_history_replay_pipeline_run`:
  - validates run id, release id, dbnum, file paths, output roots, and
    `from_sesno < to_sesno`;
  - constrains replay configs and output roots under
    `output/<project>/model_versions`;
  - supports `snapshot_id` mode by reading and validating
    `baseline_state_manifest.json`;
  - derives the safe source DB file from the snapshot replacement DB;
  - rejects conflicting request `source_db_file` in snapshot mode;
  - automatically sets `baseline_source_confirmed_at_from_sesno=true` only for
    validated physical snapshot mode;
  - rejects direct source-file mode without explicit confirmation;
  - records source observation manifest dependency and source DB hash evidence;
  - builds bounded argv for
    `aios-database -c <base-config> model-version prepare-history-replay ...`.
- Updated route logging in `src/web_api/mod.rs`.
- Mapped `requires ...` validation failures to HTTP 400 so fail-closed user
  input errors do not look like server faults.

Build validation:

```text
cargo fmt --check
  passed

git diff --check -- src/web_api/model_version_api.rs src/web_api/mod.rs
  passed with existing CRLF warning for src/web_api/mod.rs

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only
```

HTTP validation:

```text
web_server
  binary=E:\codex-targets\plant-cli-ducklake-build\debug\web_server.exe
  config=target\codex-web-baseline-readiness\DbOption-codex-web-baseline
  port=3198

startup route list includes:
  POST /api/model-version/runs/prepare-history-replay

negative direct request
  project=AvevaMarineSample
  dbnum=1112
  source_db_file=<physical snapshot replacement DB>
  from_sesno=791
  to_sesno=897
  baseline_source_confirmed_at_from_sesno omitted

result:
  HTTP 400
  success=false
  message=direct prepare-history-replay requires baseline_source_confirmed_at_from_sesno=true; use snapshot_id for a validated physical baseline or explicitly confirm source_db_file is already the from_sesno baseline

positive snapshot request
  snapshot_id=codex-ams1112-physical-791-reuse-20260620
  dbnum=1112
  baseline_dbnums=[1112]
  from_sesno=791
  to_sesno=897
  run_id=codex-http-history-snapshot-20260621041907

result:
  HTTP 200 start response
  status endpoint: succeeded
  kind=prepare_history_replay
  exit_code=0
  source_db_hash_unchanged=true
  source_db_sha256_before=5ea0c56bef3030f8a450ffd1c136948f1c1581b20b6f55de79ccf0410766e385
  source_db_sha256_after=5ea0c56bef3030f8a450ffd1c136948f1c1581b20b6f55de79ccf0410766e385
```

stdout safety evidence:

```text
generate_argv includes:
  incremental-sesno --file <snapshot replacement DB> --from-sesno 791 --to-sesno 897 --generate-model --json

publish_argv includes:
  model-version publish-history --release-id <release> --dbnum 1112 --from-sesno 791 --to-sesno 897 --materialize-assets --index-units --json

safety_checks:
  replay_namespace_differs_from_current=true
  replay_output_root_differs_from_current=true
  replay_project_output_differs_from_current=true
  replay_parquet_differs_from_current=true
  baseline_binary_supports_surreal_save=true
  baseline_parse_uses_current_file_state=true
  baseline_target_sesno_reconstruction_supported=false
  baseline_source_must_already_match_from_sesno=true
  baseline_source_confirmed_at_from_sesno=true
```

Review notes:

- The endpoint intentionally prepares and launches only the safe replay-plan
  command. It does not publish a release and does not make GET paths mutate
  DuckLake or model assets.
- Snapshot mode is the preferred backend path because it can prove the source
  DB file is a physical baseline source. Direct source-file mode remains
  possible only with explicit confirmation.
- This closes the first structured backend safety API gap, but it does not yet
  implement full replay generation orchestration, publish/register POST APIs,
  or the release state machine.
- No `cargo test` was run, per repository policy.

## Continuation Slice - Structured Publish/Register HTTP APIs

Why this slice matters:

- `prepare-history-replay` now produces a safe command plan, but the backend
  still lacked explicit structured mutation APIs for turning validated packages
  into releases.
- Oracle's architecture review explicitly called out `publish/register` as the
  next missing safety boundary.
- The implementation must reuse existing domain gates instead of inventing a
  parallel HTTP-only publish path.

SigMap note:

```text
sigmap ask "model-version publish-history register HTTP structured API release state machine backend endpoints"
  timed out after about 124s
```

Implementation completed:

- Added route:
  `POST /api/model-version/releases/register`.
- Added route:
  `POST /api/model-version/releases/publish-history`.
- Added structured JSON request DTOs:
  `RegisterReleaseRequest` and `PublishHistoryReleaseRequest`.
- Added response wrappers:
  `RegisterReleaseApiData` and `PublishHistoryReleaseApiData`.
- `register` builds `ModelReleaseRegisterRequest` and calls
  `register_model_release` in a blocking worker.
- `publish-history` builds `ModelHistoryReleasePublishRequest` and calls
  `publish_history_model_release` in a blocking worker.
- HTTP builder defaults:
  - release root:
    `output/<project>/model_versions/releases`;
  - current Parquet directory:
    `output/<project>/parquet/<dbnum>`;
  - DuckLake metadata/data from `version_context`;
  - branch id default: `main`.
- HTTP path safety:
  - register/publish Parquet paths must stay under `output`;
  - release root must stay under `output/<project>/model_versions`;
  - optional scene tree path must stay under `output`.
- Added release quality parsing compatible with CLI values:
  `complete_visual`, `quarantined_visual`, `degraded_visual`, `patch_only`,
  and `non_visual`.
- Fixed existing `GET /api/model-version/releases/{release_id}` detail lookup:
  it now reads release id directly from DuckLake instead of going through the
  published-only list projection, so staged releases created by register can be
  queried.
- Startup route logging now includes both new POST endpoints.

Build validation:

```text
cargo fmt
cargo fmt --check
  passed after formatting

git diff --check -- src/web_api/model_version_api.rs src/web_api/mod.rs .planning/2026-06-17-ducklake-valv-version-diff/GOAL.md
  passed with existing CRLF warning for src/web_api/mod.rs

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only
```

HTTP validation:

```text
web_server
  binary=E:\codex-targets\plant-cli-ducklake-build\debug\web_server.exe
  config=target\codex-web-baseline-readiness\DbOption-codex-web-baseline
  port=3198

startup route list includes:
  POST /api/model-version/releases/register
  POST /api/model-version/releases/publish-history

POST /api/model-version/releases/register
  release_id=codex-http-register-final-20260621045248
  dbnum=1112
  parquet_dir=output\AvevaMarineSample\parquet\1112
  release_quality=quarantined_visual

result:
  HTTP 200
  success=true
  registration.status=created
  release_lifecycle=staged
  release_quality=quarantined_visual
  component_index.component_count=106

repeat same POST:
  HTTP 200
  success=true
  registration.status=already_exists

GET /api/model-version/releases/codex-http-register-final-20260621045248
  HTTP 200
  success=true
  release_lifecycle=staged
  release_quality=quarantined_visual

POST /api/model-version/releases/publish-history
  release_id=codex-http-publish-negative-20260621045248
  dbnum=1112
  source_db_file=output\AvevaMarineSample\model_versions\physical_baselines\codex-ams1112-physical-791-reuse-20260620\project_path\AvevaMarineSample\ams000\ams1112_0001
  from_sesno=791
  to_sesno=897
  parquet_dir=output\AvevaMarineSample\model_versions\releases\codex-ams1112-physical-897-quarantine\parquet\1112
  materialize_assets=true
  metadata_json lacks baseline_state_manifest_path/hash

result:
  HTTP 400
  success=false
  message=baseline_missing: publish-history requires baseline_state_manifest_path and baseline_state_manifest_hash metadata; prepare a physical baseline snapshot or restore a proven baseline release before publishing
```

Review notes:

- This slice gives backend/UI callers a structured mutation path while keeping
  GET/read APIs mutation-free.
- `publish-history` still enforces the existing domain gates: current Parquet
  rejection, zero-row visual guard, baseline state evidence, materialize-assets
  requirement, and optional unit indexing.
- The detail GET fix is important for operability: a staged release created by
  `register` is now queryable even though the release list intentionally shows
  only published releases.
- This slice does not yet orchestrate the full replay generate/publish flow or
  add incremental handoff/release state machine APIs.
- No `cargo test` was run, per repository policy.

## 2026-06-21 — Structured Incremental Handoff HTTP API

Oracle / context:

- Reused completed Oracle browser consult
  `e3d-model-version-plan-review`.
- Oracle confirmed the intended boundary:
  DuckLake stays catalog/read-model/index/audit only; affected-scope
  incremental handoff must register staged `patch_only`/quarantined evidence and
  must not publish as `complete_visual`.
- `sigmap ask "model version incremental handoff ducklake architecture plan"`
  timed out after about 64s, so implementation proceeded with direct source
  inspection and existing Oracle evidence.

Implementation:

- Added route:
  `POST /api/model-version/incremental/handoff`.
- Added structured request/response DTOs in
  `src/web_api/model_version_api.rs`.
- Added `build_incremental_handoff_register_request`:
  - loads and hashes the supplied handoff manifest;
  - requires `manifest_version=incremental_publication_handoff:v1`;
  - requires `policy=explicit_register_required`;
  - requires `generation_success=true`;
  - selects a candidate by `candidate_index` or `dbnum`;
  - keeps the handoff manifest under the workspace and candidate
    `source_parquet_dir` under the configured output root;
  - loads the Parquet package with `load_model_package`;
  - checks manifest `package_hash` and optional `rows_by_table` against the
    loaded package;
  - forces a non-complete release quality, defaulting to candidate
    `suggested_release_quality` or `patch_only`;
  - rejects `complete_visual` before any registration;
  - calls existing `register_model_release` with
    `initial_status=ModelReleaseStatus::Staged`.
- Added validation flags:
  `incremental_handoff_affected_scope`,
  `explicit_release_registration_required`,
  `http_incremental_handoff_reviewed`.
- Added route log entry in `src/web_api/mod.rs`.
- Updated error classification so
  `incremental handoff cannot register complete_visual` is HTTP 400 instead of
  an internal 500.

Validation:

```text
cargo fmt --check
  passed

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

git diff --check -- src/web_api/model_version_api.rs src/web_api/mod.rs .planning/2026-06-17-ducklake-valv-version-diff/GOAL.md docs/plans/2026-06-20-e3d-version-ducklake-hardening-architecture-dev-plan.md .planning/2026-06-17-ducklake-valv-version-diff/progress.md
  passed with existing CRLF warnings only
```

HTTP validation through rebuilt `web_server`:

```text
binary=E:\codex-targets\plant-cli-ducklake-build\debug\web_server.exe
config=target\codex-web-baseline-readiness\DbOption-codex-web-baseline
port=3197

startup route list includes:
  POST /api/model-version/incremental/handoff

handoff_manifest_path:
  target\codex-publication-handoff-tree-evidence\incremental-db1112-896-to-897-20260620T155135644Z.json

POST /api/model-version/incremental/handoff
  release_id=codex-http-handoff-20260621052235
  dbnum=1112
  release_quality=patch_only

result:
  HTTP 200
  success=true
  registration.status=created
  release_lifecycle=staged
  release_quality=patch_only
  selected_candidate.dbnum=1112
  selected_candidate.suggested_release_quality=patch_only
  handoff_manifest_hash=96a0ea948b9162e2ab199b252b47a138c1a2f79441fa33a2c572a2d32a71b865
  validation_flags=codex_http_validation,incremental_handoff_affected_scope,explicit_release_registration_required,http_incremental_handoff_reviewed

repeat same POST:
  HTTP 200
  registration.status=already_exists

POST with release_quality=complete_visual:
  HTTP 400
  success=false
  message=incremental handoff cannot register complete_visual; use patch_only, quarantined_visual, degraded_visual, or non_visual until full baseline evidence is proven

GET /api/model-version/releases/codex-http-handoff-20260621052235
  HTTP 200
  success=true
  release_lifecycle=staged
  release_quality=patch_only
```

Cleanup:

- Stopped the isolated validation `web_server` process.
- Confirmed port `3197` no longer had the validation process.

Review notes:

- The endpoint is intentionally a handoff/register bridge only. It does not run
  generation, publish a release, or infer production readiness.
- The package is copied through the existing immutable release registration
  path, so `release_id + package_hash` remains the durable version identity.
- This closes the structured HTTP gap for incremental publication handoff; the
  remaining design gap is a centralized release state machine for promotion.
- No `cargo test` was run, per repository policy.

## 2026-06-21 — Release State Machine Safety Gate

Context:

- `sigmap ask "release state machine model-version staged patch_only publish readiness reconcile"`
  timed out after about 64s.
- Direct source inspection showed existing `reconcile_release` already checks
  package files, component index, mesh asset index, and unit index, but it did
  not centralize the stricter production-promotion policy from the architecture
  plan.
- The new slice keeps `reconcile` as diagnostics and adds a stricter
  state-machine facade for production lifecycle mutation.

Implementation:

- Added `src/version_management/release_state_machine.rs`.
- Added `pub mod release_state_machine` in `src/version_management/mod.rs`.
- Added `POST /api/model-version/releases/{release_id}/state-machine`.
- Added HTTP request/response wrappers in `src/web_api/model_version_api.rs`.
- Added route log entry in `src/web_api/mod.rs`.
- State-machine actions:
  - `review`: side-effect-free evidence review.
  - `publish_if_ready`: mark `published` only if all production blockers are
    absent.
  - `fail_if_unusable`: mark `failed` only when blockers exist.
- Production publication blockers include:
  - reconcile/package/index blockers;
  - release lifecycle already `failed`;
  - release quality not `complete_visual`;
  - missing baseline state manifest path/hash;
  - missing generation job id unless disabled for legacy migration;
  - missing release-local mesh asset manifest evidence for visual packages.
- Incremental handoff metadata now writes:
  `generation_job_id=<handoff run_id>`.

Validation:

```text
cargo fmt --check
  passed

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

git diff --check -- src/version_management/release_state_machine.rs src/version_management/mod.rs src/web_api/model_version_api.rs src/web_api/mod.rs .planning/2026-06-17-ducklake-valv-version-diff/GOAL.md .planning/2026-06-17-ducklake-valv-version-diff/progress.md docs/plans/2026-06-20-e3d-version-ducklake-hardening-architecture-dev-plan.md
  passed with existing CRLF warnings only
```

HTTP validation through rebuilt `web_server`:

```text
binary=E:\codex-targets\plant-cli-ducklake-build\debug\web_server.exe
config=target\codex-web-baseline-readiness\DbOption-codex-web-baseline
port=3197

startup route list includes:
  POST /api/model-version/releases/{release_id}/state-machine

POST /api/model-version/releases/codex-http-handoff-20260621052235/state-machine
  action=review

result:
  HTTP 200
  transition_allowed=false
  applied=false
  current_lifecycle=staged
  current_status=staged
  blockers include:
    baseline state manifest path/hash evidence is required
    generation_job_id evidence is required
    mesh asset index is missing
    release quality is patch_only
    release-local mesh asset manifest path/hash evidence is required

POST /api/model-version/releases/codex-http-handoff-20260621052235/state-machine
  action=publish_if_ready

result:
  HTTP 200
  transition_allowed=false
  applied=false
  action_taken=none

GET /api/model-version/releases/codex-http-handoff-20260621052235
  release_lifecycle=staged
  release_status=staged
  release_quality=patch_only
```

New handoff lineage validation:

```text
POST /api/model-version/incremental/handoff
  release_id=codex-http-sm-handoff-20260621054048
  dbnum=1112
  release_quality=patch_only

GET /api/model-version/releases/codex-http-sm-handoff-20260621054048
  generation_job_id=incremental-db1112-896-to-897-20260620T155135644Z
  release_lifecycle=staged
  release_quality=patch_only

POST /api/model-version/releases/codex-http-sm-handoff-20260621054048/state-machine
  action=publish_if_ready

result:
  HTTP 200
  transition_allowed=false
  applied=false
  blockers no longer include missing generation_job_id
  blockers still include baseline state evidence, mesh asset evidence, and patch_only quality
```

Error handling validation:

```text
POST /api/model-version/releases/codex-http-sm-handoff-20260621054048/state-machine
  action=launch_into_orbit

result:
  HTTP 400
  message=invalid release state-machine action 'launch_into_orbit'; expected review, publish_if_ready, or fail_if_unusable
```

Cleanup:

- Stopped the isolated validation `web_server` process.
- Confirmed port `3197` no longer had the validation process.

Review notes:

- This slice does not complete the final two-pane production comparison. It
  gives the orchestration layer a safe promotion gate so diagnostic/patch-only
  releases cannot be silently promoted.
- Existing `reconcile` remains useful for diagnostics. Production mutation
  should use the new state-machine endpoint.
- No `cargo test` was run, per repository policy.

## 2026-06-21 — Oracle MCP Continuation And Architecture Plan Refresh

Context:

- The user requested continuing the analysis with Oracle MCP before further
  implementation, specifically asking for the best model-version architecture,
  whether DuckLake should be used, and a complete architecture/development plan.
- `sigmap ask "E3D incremental update model version DuckLake architecture versioned model data"`
  timed out after about 74s.
- `sigmap ask "prepare-history-replay stdout generate_argv bounded runner execute replay plan web api"`
  had also timed out earlier in this continuation.

Oracle usage:

- Read the local Oracle skill instructions and ran `oracle --help`, satisfying
  the repository Oracle preflight rule.
- `oracle status --hours 72 --limit 20` showed multiple completed related
  browser sessions; the latest relevant completed session was
  `e3d-model-version-plan-review`.
- Reattached the completed Oracle browser session with:
  `oracle session e3d-model-version-plan-review --render`.
- Oracle's completed answer reconfirmed:
  - model data versions should be immutable lineage objects:
    `SOID -> BSID -> IEVID -> GJID -> RID`;
  - `release_id + package_hash`, not `sesno`, mutable output directories, or
    DuckLake snapshot ids, is the user-facing version identity;
  - DuckLake should be used in this version as release catalog/read model,
    component/unit/asset index, diff/impact, and audit only;
  - DuckLake must not become the generation writer, baseline restore provider,
    GLB/Parquet payload owner, or job-state truth;
  - the remaining backend gap is not architectural uncertainty, but safe
    orchestration from a prepared history replay plan into bounded
    generate/publish runs.
- Tried to use the newly discovered `mcp__oracle.sessions` tool directly, but
  it returned `Transport closed`. No paid/API Oracle run was started.
- Ran two Oracle dry-runs to size context:
  - broad bundle: about `380,771` tokens, too large;
  - focused bundle: about `192,293` tokens, too close to the browser/model
    budget for another reliable attachment run.

Documentation update:

- Updated
  `docs/plans/2026-06-20-e3d-version-ducklake-hardening-architecture-dev-plan.md`
  with:
  - this Oracle MCP/CLI continuation evidence;
  - the final DuckLake allow/deny boundary;
  - the implementation decision that immutable release packages are payload
    truth and DuckLake is rebuildable read model;
  - concrete model data version fields:
    `source_observation_id`, `baseline_state_id`, `increment_evidence_id`,
    `generation_job_id`, `release_id`, `package_hash`,
    `asset_manifest_hash`, and `index_rule_hash`;
  - a new `P0.6` development slice for
    `execute-history-replay-plan`;
  - an updated review summary stating that handoff and release state machine
    are already complete, and the next implementation target is replay-plan
    execution via bounded runner.

Current architecture decision:

- Proceed with DuckLake in this version, but only as a rebuildable catalog /
  index / diff / impact / audit layer.
- Keep SurrealDB and the existing generation pipeline as the compute zone.
- Keep Parquet/GLB/manifests under immutable release packages as the model data
  truth.
- Route all production status mutation through the release state machine.
- Add the safe execution bridge:
  `prepare-history-replay -> execute-history-replay-plan -> bounded generate -> handoff/register -> state-machine review`.

Next implementation target:

- Implement the structured HTTP executor for a prepared history replay plan.
- It must read the succeeded prepare run stdout JSON, select a whitelisted argv
  phase, validate source hash and command semantics, and launch through the
  bounded runner.
- Validation must use web_server HTTP and aios-database CLI/JSON/build only; no
  `cargo test`.

## 2026-06-21 — Execute History Replay Plan HTTP Runner

Context:

- The active architecture plan had narrowed the next backend gap to:
  `prepare-history-replay stdout plan -> bounded generate/publish execution`.
- `sigmap ask "execute history replay plan bounded runner model-version API prepare stdout generate publish"`
  timed out after about 74s.
- `mcp__sigmap.explain_file` for the relevant files returned no indexed
  signatures, so implementation proceeded with focused `rg` and direct source
  reads.

Implementation:

- Added route:
  `POST /api/model-version/runs/execute-history-replay-plan`.
- Added the route to the startup route log in `src/web_api/mod.rs`.
- Added request DTO:
  `ExecuteHistoryReplayPlanRunRequest`.
- Added response/evidence DTOs:
  `ExecuteHistoryReplayPlanRunApiData`,
  `HistoryReplayPlanExecutionSummary`, and
  `PreparedHistoryReplayPlanRun`.
- Added handler:
  `post_execute_history_replay_plan_run`.
- Added builder and validators:
  `build_execute_history_replay_plan_run`,
  `normalize_history_replay_plan_phase`,
  `select_history_replay_phase_argv`,
  `validate_history_replay_phase_argv`,
  `require_argv_token`,
  `require_argv_sequence`, and
  `require_argv_flag_value`.

Safety behavior:

- The endpoint never accepts arbitrary caller-provided argv.
- It reads a succeeded `prepare_history_replay` bounded run record and parses
  that run's stdout as `ModelHistoryReplayPrepareResponse`.
- It requires:
  - `prepare_run.status=succeeded`;
  - `prepare_run.exit_code=0`;
  - `prepare_run.source_db_hash_unchanged=true`;
  - prepare stdout exists and can be hashed;
  - plan project equals the HTTP project context;
  - plan `source_db_file` equals the prepare run record source file;
  - `baseline_source_confirmed_at_from_sesno=true`;
  - selected phase is one of
    `baseline_parse`, `baseline_generate`, `baseline_register`, `generate`, or
    `publish`.
- `generate` argv must contain:
  `incremental-sesno --file <source> --from-sesno <from> --to-sesno <to> --generate-model --json`.
- `publish` argv must contain:
  `model-version publish-history --release-id <release> --dbnum <dbnum> --source-db-file <source> --from-sesno <from> --to-sesno <to> --parquet-dir <replay_parquet> --parent-release-id <baseline_release> --materialize-assets --json`.
- The spawned bounded run carries the source DB hash from the prepare run as
  `expected_source_db_sha256`, so source replacement between prepare and
  execution fails before command execution.

Build validation:

```text
cargo fmt --check
  passed

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only
```

HTTP validation through rebuilt `web_server`:

```text
binary=E:\codex-targets\plant-cli-ducklake-build\debug\web_server.exe
config=target\codex-web-baseline-readiness\DbOption-codex-web-baseline
port=3197

startup route list includes:
  POST /api/model-version/runs/execute-history-replay-plan

POST /api/model-version/runs/execute-history-replay-plan
  prepare_run_id=codex-http-history-snapshot-20260621041907
  phase=bogus

result:
  HTTP 400
  message=unsupported history replay plan phase 'bogus'; expected baseline_parse, baseline_generate, baseline_register, generate, or publish

POST /api/model-version/runs/execute-history-replay-plan
  prepare_run_id=missing-prepare-run
  phase=publish

result:
  HTTP 404
  message=prepare run not found or unreadable: missing-prepare-run

POST /api/model-version/runs/execute-history-replay-plan
  prepare_run_id=codex-http-history-snapshot-20260621041907
  phase=publish
  run_id=codex-http-exec-publish-20260621061200
  timeout_secs=120
  force=true

result:
  HTTP 200
  message=history replay plan run started
  kind=history_replay_plan_publish
  launch_observed=true
  prepare_stdout_hash=d016d4db922e5a0965f3ecf79866f232f5ab6b7175ea8076817fab73d4c335fa
  expected_source_db_sha256=5ea0c56bef3030f8a450ffd1c136948f1c1581b20b6f55de79ccf0410766e385
  command_argv came from prepare stdout publish_argv

GET /api/model-version/runs/codex-http-exec-publish-20260621061200?project=AvevaMarineSample

result:
  HTTP 200
  status=failed
  exit_code=1
  error=command exited with status exit code: 1
  source_db_hash_unchanged=true
  source_db_sha256_before=5ea0c56bef3030f8a450ffd1c136948f1c1581b20b6f55de79ccf0410766e385
  source_db_sha256_after=5ea0c56bef3030f8a450ffd1c136948f1c1581b20b6f55de79ccf0410766e385
  stderr=historical release source Parquet directory does not exist: output\AvevaMarineSample\model_versions\replay_work\codex-http-history-release-20260621041907\output\AvevaMarineSample\parquet\1112
```

Cleanup:

- Stopped the isolated validation `web_server` process.
- Confirmed no validation output remained on port `3197`.

Review notes:

- This slice intentionally proves a safe execution bridge, not a successful
  publish. The publish phase failed because the replay-generated Parquet package
  does not exist yet, which is the correct fail-closed behavior.
- The endpoint now gives UI/backend orchestration a structured way to launch
  the next real `generate` phase once we decide to run the heavier model
  generation job.
- No `cargo test` was run, per repository policy.

Next implementation target:

- Run the `generate` phase through this endpoint only when ready to spend the
  heavier DB1112 generation time, then feed its handoff/register output back
  into the already implemented incremental handoff and release state machine.
- Continue toward the final two-pane complete visual comparison by producing or
  restoring the missing full baseline/target release package evidence.

## 2026-06-21 — Oracle MCP Retry And DB1112 Source Split Correction

Context:

- The user requested continuing the architecture analysis with Oracle MCP,
  specifically asking for the best model-data version implementation and
  whether DuckLake should be used.
- The local Oracle skill was read and `oracle --help` was run.
- `tool_search` exposed `mcp__oracle`, but direct calls to
  `mcp__oracle.sessions` and `mcp__oracle.consult` both failed with
  `Transport closed`.
- No API/paid Oracle run was started. The analysis fell back to the same Oracle
  session store via CLI and reattached the completed
  `e3d-model-version-plan-review` browser session.
- `sigmap ask` for the current architecture context timed out, consistent with
  prior SigMap behavior in this repository.

Architecture conclusion:

- Keep the model-version contract as immutable lineage:
  `SOID -> BSID -> IEVID -> GJID -> RID`.
- The user-visible model version remains `release_id + package_hash`, not
  `sesno`, not a mutable output path, and not a DuckLake snapshot id.
- DuckLake belongs in this version only as rebuildable catalog/read-model,
  component/unit/asset index, diff/impact, and audit.
- DuckLake must not become the generation writer, baseline restore provider,
  GLB/Parquet payload store, session replay engine, or job-state truth.

Implementation correction:

- While validating DB1112 `791 -> 897`, an old prepare run failed because
  `snapshot_id` mode had made the physical baseline replacement DB the
  incremental source. That replacement DB only reaches `sesno=791`, so
  `to_sesno=897` is invalid for it.
- `prepare-history-replay` now separates:
  - the `snapshot_id`/baseline manifest, which proves `from_sesno=791`;
  - the optional `source_db_file`, which may point to the current/history DB
    file capable of reading through `to_sesno=897`.
- New source modes:
  - `physical_snapshot_replacement_source` when the replacement DB is also the
    replay source;
  - `physical_snapshot_with_history_source` when the baseline snapshot proves
    the starting state but a different history/current DB supplies the session
    range.

Validation result:

- New prepare run:
  `codex-http-history-targetsrc-20260621062500`.
- Source mode:
  `physical_snapshot_with_history_source`.
- Source DB hash before execution:
  `70f18c70116f392eae533b75fb8f4043d031a5f049448531cc1dfc43faf7d3c2`.
- Baseline replacement DB hash:
  `5ea0c56bef3030f8a450ffd1c136948f1c1581b20b6f55de79ccf0410766e385`.
- Generate run launched through structured HTTP runner:
  `codex-http-exec-generate-targetsrc-20260621062600`.
- Command phase:
  `incremental-sesno --file <current-history-db> --from-sesno 791 --to-sesno 897 --generate-model --json`.
- The run stayed active for about 14 minutes with CPU increasing, but stdout
  did not advance past startup, stderr stayed empty, `task-metrics.json` was
  never created, and the replay Parquet directory was not created.
- To avoid leaving an opaque backend process running, it was cancelled through:
  `POST /api/model-version/runs/codex-http-exec-generate-targetsrc-20260621062600/cancel?project=AvevaMarineSample`.
- Cancellation evidence:
  - previous status: `running`;
  - final status: `cancelled`;
  - cancel reason:
    `cancelled_by_codex_after_14min_no_metrics_or_replay_parquet`;
  - `source_db_hash_unchanged=true`;
  - source SHA before/after:
    `70f18c70116f392eae533b75fb8f4043d031a5f049448531cc1dfc43faf7d3c2`;
  - child process PID `38700` no longer exists.

Next implementation target:

- Add heartbeat/stage evidence to the generation path so the backend can
  surface progress rather than a silent long-running process.
- Re-run the DB1112 `791 -> 897` generate phase after metrics instrumentation.
- If it succeeds, validate replay Parquet and handoff evidence, then drive it
  through handoff/register/state-machine and DuckLake indexes.

## 2026-06-21 — Incremental Generate Metrics Heartbeat

Context:

- The DB1112 replay generate run proved the source split fix, but also showed a
  production observability gap: the child process can consume CPU for many
  minutes before any `task-metrics.json` exists.
- Bounded runner already supports `metrics_path` and `stale_heartbeat_secs`,
  but that mechanism is ineffective until the child creates and periodically
  updates the metrics file.
- `sigmap ask "incremental-sesno generate-model task metrics heartbeat stage progress perf_metrics record_generate_heartbeat"`
  timed out after about 34 seconds, so source inspection proceeded with `rg`
  and direct reads.

Planned implementation:

- Add an explicit heartbeat guard in `perf_metrics` that can refresh generate
  progress from a background thread during synchronous/blocking phases.
- In `run_incremental_sesno_once`, write generate progress before source
  collection and wrap long phases:
  source collection, db-meta refresh, SurrealDB persist, tree-index evidence,
  `gen_all_geos_data`, post-generation Parquet export, and publication handoff.
- Validate by running `aios-database incremental-sesno` with injected
  `AIOS_TASK_METRICS_PATH` and checking that the metrics JSON exists even for
  a short failing/negative CLI run.

Implementation:

- Added `GenerateHeartbeatGuard` and `start_generate_heartbeat` in
  `src/perf_metrics.rs`.
  - It is a no-op when `AIOS_TASK_METRICS_PATH` is not configured.
  - It writes an immediate `record_generate_progress` snapshot and then
    refreshes `record_generate_heartbeat` at a fixed interval.
  - Drop wakes and joins the worker thread, avoiding detached heartbeat
    threads after fast phase transitions.
- Instrumented `run_incremental_sesno_once` in `src/main.rs`:
  - `incremental_sesno_started`;
  - `incremental_sesno_collecting_file`;
  - `incremental_sesno_collecting_dbnums`;
  - `incremental_sesno_refreshing_db_meta`;
  - `incremental_sesno_connecting_model_store`;
  - `incremental_sesno_persisting`;
  - `incremental_sesno_checking_tree_index`;
  - `incremental_sesno_generate_running`;
  - `incremental_sesno_exporting_parquet`;
  - `incremental_sesno_building_handoff`.

Validation:

```text
cargo fmt --check
  passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only
```

CLI metrics smoke:

```text
AIOS_TASK_METRICS_PATH=target\codex-metrics-smoke\incremental-negative-20260621065446.json
aios-database -c db_options\DbOption incremental-sesno --file <ams1112_0001> --from-sesno 897 --to-sesno 897 --generate-model --json

result:
  exit_code=0
  metrics_exists=true
  job_kind=history_replay_plan_generate_metrics_smoke
  success=true
  final stage=incremental_sesno_handoff_built
```

Long-stage metrics smoke:

```text
AIOS_TASK_METRICS_PATH=target\codex-metrics-smoke\incremental-long-cancel-final-20260621065819.json
aios-database -c output\AvevaMarineSample\model_versions\replay_configs\codex-http-history-targetsrc-release-20260621062500\DbOption-replay incremental-sesno --file <ams1112_0001> --from-sesno 791 --to-sesno 897 --generate-model --json

result after 25s:
  process_running_before_stop=true
  process_exited_after_stop=true
  metrics_exists=true
  metrics_length=894
  job_kind=history_replay_plan_generate_metrics_long_smoke_final
  success=null
  stage=incremental_sesno_collecting_file
  detail=file=\\?\D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001
  stderr_length=0
```

HTTP bounded-run smoke:

```text
web_server:
  binary=E:\codex-targets\plant-cli-ducklake-build\debug\web_server.exe
  config=target\codex-web-baseline-readiness\DbOption-codex-web-baseline
  port=3197

POST /api/model-version/runs/execute-history-replay-plan
  prepare_run_id=codex-http-history-targetsrc-20260621062500
  phase=generate
  run_id=codex-http-exec-generate-metrics-smoke-clean-20260621070645

GET /api/model-version/runs/{run_id}
  status_before_cancel=running
  metrics.exists=true
  metrics.stage=incremental_sesno_collecting_file
  metrics.updated_at=2026-06-21T07:07:21.995864300+08:00

POST /api/model-version/runs/{run_id}/cancel
  kill_attempted=true

GET /api/model-version/runs/{run_id}
  status_after_cancel=cancelled
  cancel_reason=metrics_http_smoke_complete
  source_db_hash_unchanged=true
```

Self review:

- The original failure mode is addressed: the bounded runner will now see a
  metrics file before the long `collect_pdms_increment_for_file` call completes.
- The long-stage smoke proves the metrics file is present while the child is
  still running and identifies the current stage.
- The HTTP bounded-run smoke proves the web API can surface that metrics stage
  through run status before cancellation.
- This does not yet prove the full DB1112 replay generate succeeds; it makes
  the next rerun diagnosable rather than opaque.

## DB1112 Generate Bottleneck Architecture Review - 2026-06-21

Purpose:

- Continue the Oracle-backed architecture analysis for E3D incremental model
  versions after the metrics heartbeat slice.
- Decide whether the next work should keep advancing DuckLake/release layers or
  first address the DB1112 generate bottleneck now exposed by metrics.

Oracle/tooling evidence:

- `oracle --help` was run per repository guidance.
- `sigmap ask "E3D incremental sesno model version DuckLake architecture history replay DB1112"`
  timed out after about 64 seconds; SigMap MCP signature searches also had no
  hits for the new untracked modules, so source inspection proceeded with `rg`
  and direct reads.
- A full Oracle context package dry-run was about `360,736` tokens and too
  large for a useful browser consult.
- Added focused context document:
  `docs/plans/2026-06-21-e3d-model-version-oracle-bottleneck-context.md`.
- Focused Oracle dry-run with the context doc, architecture plan, and selected
  source files was about `155,812` tokens.
- Live browser consult session
  `e3d-model-version-bottleneck-review` failed because Oracle's private Chrome
  profile was not logged into ChatGPT. API mode was not used, to avoid a paid
  call without explicit authorization.
- Existing completed Oracle sessions
  `e3d-model-version-plan-review` and
  `e3d-ducklake-architectu-compact` were re-read and remain the active second
  opinion: DuckLake is appropriate only as read-model/catalog/index/diff/audit;
  the immutable version lineage remains `SOID -> BSID -> IEVID -> GJID -> RID`.

HTTP observation:

```text
run_id=codex-http-exec-generate-observed-20260621071222
phase=generate
prepare_run_id=codex-http-history-targetsrc-20260621062500
argv=incremental-sesno --file <current-history-db> --from-sesno 791 --to-sesno 897 --generate-model --json
status_before_cancel=running
metrics.stage=incremental_sesno_collecting_file
heartbeat=refreshing every 15s
elapsed_before_cancel=about 6.4 minutes
cpu=increasing
status_after_cancel=cancelled
source_db_hash_unchanged=true
```

Source finding:

- `run_incremental_sesno_once` calls
  `collect_pdms_increment_for_file` to build `update_log` and
  `element_changes`.
- The same run then calls `persist_pdms_increment_files`, whose
  `persist_pdms_increment_file` opens pdms-io again and calls
  `collect_increment_eles` for the same actual sesno range.
- `pdms-io` operation data is `Clone`, so the immediate repo-local optimization
  can reuse the same grouped operations without changing pdms-io first.

Decision:

- The architecture does not change: keep SurrealDB as the generation compute
  zone, immutable Parquet/GLB package as release truth, and DuckLake as
  rebuildable read model.
- The next active implementation slice is now P0.8: collect pdms-io operations
  once per file/range, reuse them for report + persist + handoff, and add
  per-session collection progress/cancel evidence.

Documentation updates:

- Updated
  `docs/plans/2026-06-20-e3d-version-ducklake-hardening-architecture-dev-plan.md`
  with:
  - the new Oracle attempt and browser-login blocker;
  - new edge cases for duplicate operation collection and long collection
    observability;
  - P0.8 single-collection implementation plan;
  - performance/maintainability guidance;
  - revised final review summary.

Next requirement:

- Implement P0.8 before another long DB1112 generate run:
  - internal collector returns grouped operations once;
  - persist reuses grouped operations;
  - public JSON-compatible APIs remain stable;
  - metrics prove no double `collect_increment_eles` on the main CLI/HTTP path;
  - validation remains CLI/JSON + HTTP, with no `cargo test`.

## Single-Pass Increment Evidence Collection - 2026-06-21

Purpose:

- Remove duplicate pdms-io collection from the `incremental-sesno` main path.
- Keep public JSON-compatible collection APIs stable while allowing the CLI/HTTP
  runner path to reuse runtime `EleOperationData`.

Implementation:

- Added runtime-only collected structures in
  `src/data_interface/sesno_increment.rs`:
  - `PdmsSesnoCollectedFile`
  - `PdmsSesnoCollectedOutcome`
- Added `collect_pdms_increment_for_file_with_operations`.
  - It returns the existing public `PdmsSesnoIncrementOutcome` plus grouped
    pdms-io operations.
  - Empty/no-op ranges still produce a file report with empty grouped
    operations.
- Kept `collect_pdms_increment_for_file` as a compatibility wrapper that drops
  grouped operations and returns the existing JSON-serializable outcome.
- Added `collect_pdms_increment_for_dbnums_from_index_with_operations` and kept
  the old `collect_pdms_increment_for_dbnums_from_index` wrapper.
- Split persist into:
  - old `persist_pdms_increment_files`, which remains compatible and can still
    recollect from reports;
  - new `persist_collected_pdms_increment_files`, which persists directly from
    grouped operations.
- Updated `run_incremental_sesno_once` in `src/main.rs`:
  - collection now accumulates `PdmsSesnoCollectedOutcome`;
  - db-meta refresh, update log, generation handoff, and JSON summary still use
    the public outcome;
  - persist uses `persist_collected_pdms_increment_files`, so it does not open
    pdms-io or call `collect_increment_eles` again.

Validation:

```text
cargo fmt --check
  passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only
```

CLI no-change smoke:

```text
AIOS_TASK_METRICS_KIND=single_pass_nochange_smoke
aios-database -c db_options\DbOption incremental-sesno --file <ams1112_0001> --from-sesno 897 --to-sesno 897 --generate-model --json

result:
  exit_code=0
  metrics_exists=true
  metrics_path=target\codex-single-pass-smoke\nochange-20260621074603.json
  final_stage=incremental_sesno_handoff_built
```

CLI real small-range smoke:

```text
AIOS_TASK_METRICS_KIND=single_pass_smallrange_smoke
aios-database -c db_options\DbOption --verbose incremental-sesno --file <ams1112_0001> --from-sesno 896 --to-sesno 897 --json

result:
  exit_code=0
  metrics_exists=true
  metrics_path=target\codex-single-pass-smoke\smallrange-20260621074637.json
  session_count=1
  element_count=169
  collect_increment_eles_count_in_verbose_output=1
```

Self review:

- The main CLI/HTTP runner path now has concrete no-double-collect evidence for
  the DB1112 `896 -> 897` range.
- Public JSON-compatible APIs remain available for other callers.
- The old `persist_pdms_increment_files` intentionally remains as a compatibility
  fallback; the optimized path uses the new collected persist entrypoint.
- This does not yet provide true per-session progress inside pdms-io's long
  `collect_increment_eles` call. The next hardening step is a pdms-io callback
  or incremental collector loop that can update metrics with current session /
  processed session counts before trying another long DB1112 `791 -> 897`
  generate run.

## Per-Session Increment Collection Progress - 2026-06-21

Purpose:

- Finish P0.8 by making the long pdms-io collection phase observable at
  session granularity.
- Preserve the existing `collect_increment_eles` API while giving
  `incremental-sesno` a progress callback path.

Oracle/SigMap notes:

- The local Oracle skill instructions were read.
- Oracle MCP `sessions` and `consult` both returned `Transport closed` in this
  continuation, so no new paid/API Oracle run was started.
- `oracle status --hours 120` listed the already completed Oracle browser
  sessions used for the architecture decision, including
  `e3d-model-version-plan-review` and `e3d-ducklake-architectu-compact`.
- The architecture conclusion remains unchanged: DuckLake is a rebuildable
  release catalog/read-model/index/diff/audit layer, not the model generation
  writer, baseline restore provider, payload body store, or user-facing version
  id.
- `sigmap ask "E3D incremental parsing model version ducklake architecture current implementation"`
  timed out again; direct file inspection and MCP SigMap signature searches
  were used as fallback.

Implementation:

- In `D:\work\plant-code\pdms-io-fork\src\io.rs`:
  - added `IncrementCollectProgress`;
  - kept `collect_increment_eles` as a wrapper;
  - added `collect_increment_eles_with_progress`;
  - emitted phases `started`, `session_started`,
    `session_locations_collected`, `session_refnos_processing`,
    `session_finished`, and `finished`;
  - callback returns `anyhow::Result<()>`, leaving a safe path for future
    cancellation without changing existing callers.
- In `src/data_interface/sesno_increment.rs`:
  - imported `IncrementCollectProgress`;
  - added `record_pdms_collect_progress`;
  - switched the collected-operation path to
    `collect_increment_eles_with_progress`;
  - wrote metrics stage `incremental_sesno_collecting_file_progress` with
    file path, phase, current sesno, processed/total sessions, refno location
    count, unique/duplicate refnos, and operation count.

Validation:

```text
rustfmt --edition 2024 --check D:\work\plant-code\pdms-io-fork\src\io.rs
  passed

cargo fmt --check
  passed

git diff --check -- <touched repo files>
  passed

git -C D:\work\plant-code\pdms-io-fork diff --check -- src\io.rs
  passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing pdms-io warnings only
```

Long-range progress smoke:

```text
AIOS_TASK_METRICS_KIND=single_pass_longrange_progress_smoke
aios-database -c db_options\DbOption incremental-sesno --file <ams1112_0001> --from-sesno 791 --to-sesno 897 --json

observed_progress=true
observed_after=about 7.5s
stage=incremental_sesno_collecting_file_progress
detail=file=<ams1112_0001> phase=session_locations_collected sesno=792 sessions=0/106 refno_locs=31 unique_refnos=0 duplicate_refnos=0 operations=0
action=validation script killed child before persist
exit_code=-1
```

Small-range complete smoke after progress callback:

```text
AIOS_TASK_METRICS_KIND=single_pass_smallrange_progress_smoke
aios-database -c db_options\DbOption --verbose incremental-sesno --file <ams1112_0001> --from-sesno 896 --to-sesno 897 --json

exit_code=0
metrics_exists=true
metrics_path=target\codex-collect-progress-smoke\smallrange-progress-20260621075953.json
final_stage=incremental_sesno_handoff_built
session_count=1
element_count=169
collect_increment_eles_count_in_verbose_output=1
```

HTTP bounded-run progress smoke:

```text
WEB_SERVER_PORT=3197
web_server --config db_options/DbOption-codex-live-view

POST /api/model-version/runs/execute-history-replay-plan
  prepare_run_id=codex-http-history-targetsrc-20260621062500
  phase=generate
  run_id=codex-http-exec-generate-progress-20260621081108

run.json:
  status=cancelled
  exit_code=1
  metrics.stage=incremental_sesno_collecting_file_progress
  source_db_hash_unchanged=true
  no aios-database process remained after cancellation

GET /api/model-version/runs/codex-http-exec-generate-progress-20260621081108?project=AvevaMarineSample:
  success=true
  data.run.status=cancelled
  data.run.metrics.stage=incremental_sesno_collecting_file_progress
  data.run.source_db_hash_unchanged=true

task-metrics.json:
  stage=incremental_sesno_collecting_file_progress
  detail=file=<ams1112_0001> phase=session_locations_collected sesno=793 sessions=1/106 refno_locs=31 unique_refnos=0 duplicate_refnos=0 operations=0
```

Self review:

- P0.8 is complete for single-pass collection reuse and per-session collection
  observability.
- The callback currently provides cancellation capability to callers via
  `Result`, but the production HTTP runner still cancels at process boundary.
- The HTTP bounded-run metrics path now exposes per-session collection progress.
- The next validation should let DB1112 `791 -> 897` generate run to normal
  completion rather than cancelling immediately after first progress evidence.
- Overall goal remains incomplete until the DB1112 generated package is
  published/indexed and two release-local 3D models are visible in the compare
  UI.

## Oracle Architecture Refinement - 2026-06-21

Purpose:

- Continue the Oracle review requested by the user and turn the result into a
  concrete architecture/development plan before more implementation.

Oracle evidence:

- `mcp__oracle.sessions` and `mcp__oracle.consult` were attempted after
  loading the Oracle MCP tool; both returned `Transport closed`.
- Fallback stayed within the same Oracle toolchain using CLI/browser mode.
  `oracle --help` and dry-run/files-report were run first.
- Successful browser consult:
  `e3d-version-ducklake-compact-plan`.
- Reattach:
  `oracle session e3d-version-ducklake-compact-plan`.
- Input was about `23,053` tokens with two attachments:
  `docs/plans/2026-06-20-e3d-version-ducklake-hardening-architecture-dev-plan.md`
  and
  `docs/plans/2026-06-21-e3d-model-version-oracle-bottleneck-context.md`.
- No API/paid Oracle call was used.

Architecture decision recorded:

- Keep the pipeline:
  `SourceObservation -> BaselineState -> IncrementEvidence -> GenerationJob -> SurrealDB workspace -> immutable ReleasePackage -> DuckLake projection/read-model -> read-only API -> two-pane compare`.
- Tighten the identity contract to immutable version algebra:
  `RID = f(SOID, BSID, IEVID, GJID)`, with user-facing identity
  `release_id + package_hash`.
- Treat SurrealDB as ephemeral compute cache only.
- Treat DuckLake as append-only projection/read-model only:
  derived graph, query acceleration, diff/impact, and audit/event log.
- Do not use DuckLake as generation writer, payload truth, baseline restore
  source, job truth, or UI version id.

Documentation updates:

- Updated
  `docs/plans/2026-06-20-e3d-version-ducklake-hardening-architecture-dev-plan.md`
  with the successful Oracle consult evidence, refined final decision, DuckLake
  boundary, falsification gates, and new P0.9
  `版本代数与 projection hard boundary` development item.
- Updated `.planning/2026-06-17-ducklake-valv-version-diff/GOAL.md` with the
  current immutable version algebra decision.

Next implementation order:

1. Continue DB1112 `791 -> 897` bounded generate validation using the
   single-pass/progress path.
2. If generate completes, proceed to immutable release package,
   handoff/register, DuckLake projection index, compare readiness, and two-pane
   3D UI verification.
3. Before production publish, implement P0.9 projection freshness/falsification
   gates so a stale or inconsistent DuckLake projection cannot masquerade as
   release truth.

## Oracle MCP Continuation, DB1112 Increment Validation, And Compare Evidence - 2026-06-21

Purpose:

- Continue the Oracle-assisted architecture analysis requested by the user.
- Decide whether DuckLake should own model data versions.
- Validate the current incremental generate path on a real DB1112 history slice.
- Reconfirm that two-pane 3D comparison can display two release-local models
  while preserving production-readiness semantics.

Oracle evidence:

- Loaded the local Oracle skill instructions.
- `mcp__oracle.sessions` for `e3d-version-ducklake-compact-plan` returned
  `Transport closed`.
- Fallback used the Oracle CLI/browser session:
  `oracle session e3d-version-ducklake-compact-plan --render`.
- The rendered Oracle answer restated the architecture rule: model versions
  need immutable version algebra, and DuckLake must be append-only
  projection/read-model rather than generation truth.
- `sigmap ask "model-version incremental handoff state machine publish_if_ready release index staged patch_only DuckLake"`
  timed out after about 34 seconds; direct file inspection and HTTP evidence
  were used as fallback.

Architecture decision:

```text
SOID -> BSID -> IEVID -> GJID -> RID -> package_hash
```

- User-facing identity remains `release_id + package_hash`.
- DuckLake is acceptable for catalog/projection/diff/audit/read APIs, but not
  for payload truth, generation writes, baseline restore, job truth, or UI
  version identity.
- Immutable release packages remain the model payload truth.

Implementation adjustment in `D:\work\plant-code\pdms-io-fork\src\io.rs`:

- Changed the large-session owner children cache from cached `RefU64Vec`
  membership scans to cached `HashSet<RefU64>` membership lookups.
- Kept the existing fast current-state progress path and API shape.

Build and formatting validation:

```text
rustfmt --edition 2024 D:\work\plant-code\pdms-io-fork\src\io.rs
  passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing warnings only

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing warnings only
```

Fast-path validation:

```text
smallrange before owner HashSet:
  target\codex-fast-path-smoke\smallrange-fast-20260621091049.json
  exit_code=0
  final_stage=incremental_sesno_handoff_built
  collect_increment_eles_mentions=1

longrange before owner HashSet:
  target\codex-fast-path-smoke\longrange-fast-20260621091138.json
  range=791 -> 897
  killed_after=241s
  first_892_at_s=74
  fast_path_at_s=105
  last_detail=phase=session_refnos_processing sesno=892 sessions=100/106 refno_locs=220296 unique_refnos=9300 duplicate_refnos=0 operations=0

smallrange after owner HashSet:
  target\codex-owner-hashset-smoke\smallrange-owner-hashset-20260621091951.json
  exit_code=0
  final_stage=incremental_sesno_handoff_built
  collect_increment_eles_mentions=1

longrange after owner HashSet:
  target\codex-owner-hashset-smoke\longrange-owner-hashset-20260621092039.json
  range=791 -> 897
  killed_after=181s
  first_892_at_s=70
  fast_path_at_s=103
  last_detail=phase=session_refnos_processing sesno=892 sessions=100/106 refno_locs=220296 unique_refnos=5300 duplicate_refnos=0 operations=0
```

Conclusion:

- The single-pass collection and progress instrumentation work.
- The large-session fast path is entered.
- Owner membership lookup is not the dominant bottleneck.
- The remaining DB1112 `791 -> 897` bottleneck is per-refno current-state
  parsing/checking for session 892 with `refno_locs=220296`.

Real DB1112 small history generate:

```text
range=896 -> 897
metrics=target\codex-smallrange-generate\generate-896-897-20260621092553.json
exit_code=0
elapsed=50s
final_stage=incremental_sesno_handoff_built
mesh_generated=2
mesh_cache_hit=13
inst_relate=51731
inst_info=51694
inst_relate_aabb=51318
tubi_count=836
error_count=0
parquet_files=36
parquet_bytes=492613
```

Handoff manifest:

```text
target\codex-smallrange-generate\handoffs\incremental-db1112-896-to-897-20260621T012640971Z.json

package_hash:
  6e2bfaaafe091aa0ae178420c3e3953dcff1c5d8062f898eca478e4ff04d2c31

release_id:
  codex-smallrange-896-897-20260621092553-db1112-sesno897-pkg6e2bfaaafe09

rows_by_table:
  aabb=105
  geo_instances=163
  instances=106
  ptsets=237
  transforms=131
  primitive_keypoints=0
  tubings=0
```

HTTP handoff/register/index/state-machine validation on port `3197`:

```text
POST /api/model-version/incremental/handoff
  success=true
  release_lifecycle=staged
  release_quality=patch_only
  package_hash=6e2bfaaafe091aa0ae178420c3e3953dcff1c5d8062f898eca478e4ff04d2c31

POST /api/model-version/releases/{release_id}/index
  component_count=106
  distinct_component_hashes=106

POST /api/model-version/releases/{release_id}/index-units
  unit_count=5
  member_count=106
  unresolved_member_count=0

POST /api/model-version/releases/{release_id}/index-assets?materialize=true
  geo_hash_count=6
  present_count=6
  missing_count=0
  glb_checked_count=6
  glb_readable_count=6
  glb_unreadable_count=0

POST /api/model-version/releases/{release_id}/state-machine action=review
  transition_allowed=false
  blockers:
    - baseline state manifest path/hash evidence is required for production publication
    - release quality is patch_only, expected complete_visual for production publication

POST /api/model-version/releases/{release_id}/state-machine action=publish_if_ready
  applied=false
  current_lifecycle=staged
```

Interpretation:

- The small `896 -> 897` real incremental package is valid as staged
  `patch_only` evidence and as a regression fixture.
- It must not be published as production `complete_visual`; the state machine
  correctly blocks that path.

Existing published DB1112 compare pair:

```text
from=codex-ams1112-physical-791-quarantine
to=codex-ams1112-physical-897-quarantine

GET /api/model-version/compare-readiness:
  classification=quarantined_visual
  both_published=true
  component_indexes_ready=true
  mesh_assets_ready=true
  production_ready=false
  production_comparison_allowed=false
  diff added=5059 deleted=2525 changed=43 unchanged=23549

791 mesh assets:
  geo_hash_count=1192
  glb_checked_count=1192
  glb_readable_count=1192
  missing_count=0

897 mesh assets:
  geo_hash_count=1303
  glb_checked_count=1303
  glb_readable_count=1303
  missing_count=0
```

Browser validation:

```text
agent-browser session=e3d-compare
url=http://127.0.0.1:3197/model-version/compare?from=codex-ams1112-physical-791-quarantine&to=codex-ams1112-physical-897-quarantine&viewer_limit=200&diff_limit=50

observed:
  readiness=quarantined_visual / not production ready
  from iframe visible 3D geometry
  to iframe visible 3D geometry
  diff table populated with added rows

screenshot:
  .planning\2026-06-17-ducklake-valv-version-diff\model-version-compare-791-897-oracle-architecture-agent-browser.png
```

The screenshot shows both panes loading release-local geometry:

```text
791 pane: components 200/26117, geometries 220/220 loaded
897 pane: components 200/28651, geometries 137/137 loaded
```

Current status:

- Diagnostic two-pane 3D comparison is working for the published quarantine
  791/897 pair.
- Production sign-off remains blocked until a `complete_visual` pair exists.
- The broader DB1112 `791 -> 897` generate path still needs P0.10 collector
  optimization around session 892 current-state parsing before it is likely to
  complete comfortably.

## DB1112 Large-Session Collector Hardening - 2026-06-21

Purpose:

- Finish the P0.10 collector optimization needed before another full DB1112
  `791 -> 897 --generate-model` run.
- Separate pure collection diagnostics from SurrealDB persistence cost.
- Keep the Oracle architecture boundary unchanged: DuckLake remains
  release/read-model projection only, not generation truth.

Oracle/SigMap evidence:

- The Oracle skill instructions were reloaded.
- `mcp__oracle.sessions` for `e3d-version-ducklake-compact-plan` still returned
  `Transport closed`.
- The already-rendered Oracle CLI/browser conclusion remains the active design:
  `SOID -> BSID -> IEVID -> GJID -> RID -> package_hash`, immutable packages as
  payload truth, DuckLake as append-only projection/read-model.
- `sigmap ask "pdms-io incremental collect current-state fast path session 892 refno_locs owner cache"`
  timed out after about 49 seconds; implementation continued with direct source
  inspection and CLI validation.

Implementation:

- Added `incremental-sesno --no-persist` in `src/main.rs`.
  - Default behavior is unchanged: db_meta refresh, SurrealDB connect, PE/ATT
    persist, and optional model generation still run.
  - `--no-persist` skips db_meta refresh, SurrealDB connection, and PE/ATT
    writes for diagnostics/preflight.
  - `--no-persist --generate-model` fails fast because generation requires
    persisted PE/ATT input.
- Extended pdms-io progress evidence:
  - `session_processed_refnos`
  - `session_current_refno`
  - `session_current_offset`
- Optimized large session current-state collection in
  `D:\work\plant-code\pdms-io-fork\src\io.rs`:
  - dedupe by refno, keeping the latest physical offset;
  - process current elements in offset order;
  - for large owner sets, build one full index map and resolve owner offsets in
    batch instead of calling `search_latest_refno` per owner;
  - parse owner elements in offset order and cache children in `HashSet`;
  - encode missing owner as empty children so owner checking marks the child
    deleted without repeating B-tree search;
  - parse errors remain conservative `None` operations and are logged.

Validation:

```text
rustfmt --edition 2024 D:\work\plant-code\pdms-io-fork\src\io.rs
  passed

cargo fmt --check
  passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing warnings only

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing warnings only
```

Small-range default persist regression:

```text
range=896 -> 897
metrics=target\codex-two-phase-smoke\smallrange-default-persist-final-20260621110654.json
exit_code=0
duration_ms=5612
session_count=1
element_count=169
data_persist sessions=1 pe=169 att=169 uda=0 deletes=0 dbnum_info=1
```

Small-range no-persist regression:

```text
range=896 -> 897
metrics=target\codex-two-phase-smoke\smallrange-no-persist-20260621102651.json
exit_code=0
duration_ms=4718
summary=data_persist skipped (--no-persist requested)
stdout contained no SurrealDB startup/connection block
```

Invalid option validation:

```text
command=incremental-sesno --no-persist --generate-model
exit_code=1
stderr=--no-persist cannot be combined with --generate-model; incremental model generation requires persisted PE/ATT data
```

Long-range no-persist validation:

```text
range=791 -> 897
metrics=target\codex-two-phase-smoke\longrange-no-persist-owner-nosearch-20260621110306.json
exit_code=0
metrics_duration_ms=153307
process_wall_time=159.8s
session_count=106
element_count=224384
total_changes=223588
prim=2417
loop_owner=62
bran_hanger=31
basic_cata=766
delete=220312
data_persist=skipped
```

Self review:

- The DB1112 session 892 collector bottleneck is no longer blocking pure
  collection: `791 -> 897 --no-persist` completed in about 2.6 minutes.
- The default persist path still works on the small real DB1112 range.
- The new diagnostic mode avoids contaminating collection performance with
  SurrealDB write cost.
- The final production goal remains incomplete: the full
  `791 -> 897 --generate-model` run, handoff/register, DuckLake projection,
  release readiness, and browser compare validation still need to be executed
  after this collector hardening.

## 2026-06-21 — Oracle MCP v3 Architecture Refresh

User request:

- Continue the Oracle MCP analysis.
- Decide the best model-data version architecture.
- Decide whether DuckLake belongs in this version.
- Produce a complete architecture and development plan before continuing
  implementation.

Oracle/tooling evidence:

- Local Oracle skill instructions were loaded again and `oracle --help` was run.
- `tool_search` exposed `mcp__oracle`.
- `mcp__oracle.consult` with the focused architecture prompt returned
  `Transport closed`; no answer was produced by MCP.
- Oracle CLI dry-run/files-report succeeded with a focused 13-file bundle:
  about `150,686` tokens.
- Oracle CLI browser run was started as `e3d-ducklake-version-plan-2`, but failed
  because Oracle's private Chrome profile had no usable ChatGPT login/model
  selector state.
- No API/paid Oracle run was started.
- Reused completed Oracle browser session
  `e3d-version-ducklake-compact-plan` via:

```text
oracle session e3d-version-ducklake-compact-plan --render
```

Decision:

- Keep the existing direction:

```text
SourceObservation -> BaselineState -> IncrementEvidence -> GenerationJob
-> SurrealDB workspace -> immutable ReleasePackage
-> DuckLake append-only projection/read-model
-> read-only API -> two-pane compare UI
```

- Model version identity must be `RID = f(SOID, BSID, IEVID, GJID)`.
- User-facing version is `release_id + package_hash`.
- `sesno`, DB path, output directory and DuckLake snapshot id are not model
  version identity.
- DuckLake is accepted only after release package publication, as append-only
  catalog/index/diff/impact/audit. It must not be generation writer, payload
  truth, baseline restore source, job truth or UI version id.

New planning artifact:

```text
docs/plans/2026-06-21-e3d-model-version-ducklake-oracle-mcp-v3.md
```

The new document contains:

- model-data version contract;
- DuckLake responsibility boundary;
- complete edge-case list;
- architecture and file structure;
- CLI/API contract;
- error handling and recovery rules;
- P0/P1/P2 development plan;
- validation gates that avoid `cargo test` and use CLI/JSON plus HTTP/POST.

Next implementation priority after this planning slice:

1. Continue P0 full DB1112 `791 -> 897 --generate-model` validation.
2. Finish/verify the current `pe_transform` blocker hardening.
3. Produce a self-validating release package.
4. Register/index/review through HTTP and DuckLake projection.
5. Validate two `complete_visual` releases in `/model-version/compare`.

## 2026-06-21 — DB1112 pe_transform Marker Hardening Validation

Goal:

- Verify that the latest `pe_transform` hardening actually clears the previous
  DB1112 refresh blocker at around `2280/3975`, where `get_local_mat4` stalled
  on datum refno `17496_272526`.

Validation command:

```text
aios-database -c output\AvevaMarineSample\model_versions\replay_configs\codex-http-history-targetsrc-release-20260621062500\DbOption-replay --refresh-transform 1112
```

Evidence:

```text
run_root=target\codex-transform-refresh\db1112-marker-20260621122842
metrics=target\codex-transform-refresh\db1112-marker-20260621122842\task-metrics.json
stdout=target\codex-transform-refresh\db1112-marker-20260621122842\stdout.log
stage=refresh_pe_transform_dbnums_done
processed=3655
primed=3552
duration_ms≈20700
```

Observed stdout:

```text
progress reached 3650/3975 (91%)
完成！共处理 3655 个节点，预热 transform_cache 3552 个节点
pe_transform 刷新完成，共处理 3655 个节点
```

Decision:

- The previous `JLDATU`/`PLDATU` datum marker stall is cleared for this DB1112
  replay config.
- `pe_transform` is no longer the immediate blocker for the next full
  `791 -> 897 --generate-model` run.
- Continue with the real persisted/generation path, keeping the same metrics
  discipline and stopping only if a new stage-specific blocker appears.

Self review:

- This was a real CLI validation against DB1112 and the same replay config used
  by the planned full generate.
- The command did not rely on unit tests or `cargo test`.
- The metrics file reports `success=null` because this CLI path does not appear
  to call the generic task-metrics finalizer, but the stage and stdout provide
  direct completion evidence.

## 2026-06-21 — DB1112 Full Generate + HTTP Handoff Registered

Full generate validation:

```text
command=aios-database incremental-sesno --file D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001 --from-sesno 791 --to-sesno 897 --generate-model --json
run_root=target\codex-full-generate\full-791-897-marker-20260621123126
metrics=target\codex-full-generate\full-791-897-marker-20260621123126\task-metrics.json
handoff=target\codex-full-generate\full-791-897-marker-20260621123126\handoffs\incremental-db1112-791-to-897-20260621T050159936Z.json
duration_ms=1837576
success=true
package_hash=b509906b4f83f876cd874266366dcd3cc7237eb0e3312575648a9f72cf0069e5
```

Generated affected-scope package:

```text
instances=46469
geo_instances=1867
transforms=2631
aabb=2434
tubings=0
ptsets=0
primitive_keypoints=0
missing_mesh_geo_hashes=1
missing_tree_files=[1112]
```

Implementation hardening added during registration validation:

- DuckDB `read_parquet()` paths are normalized to absolute paths.
- DuckLake attach uses `OVERRIDE_DATA_PATH true` so catalogs initialized with
  older relative `DATA_PATH` values can be opened through the current config.
- Long user-facing `release_id` values no longer become long package directory
  names; storage dir is shortened to `prefix-hash`.
- DuckLake metadata/data default location now falls back to
  `output\<project>\model_versions_ducklake` when the normal project output
  path would produce unsafe Windows/DuckDB long paths.

HTTP validation:

```text
web_server=http://127.0.0.1:3100
logs=target\codex-web-server\full-generate-handoff-fixed3-202606211350
release_id=codex-fullrange-791-897-marker-20260621123126-db1112-sesno897-pkgb509906b4f83-http-fixed
ducklake_metadata_path=output\AvevaMarineSample\model_versions_ducklake\metadata.ducklake
ducklake_data_path=output\AvevaMarineSample\model_versions_ducklake\data
```

```text
POST /api/model-version/incremental/handoff
status=200
success=true
release_lifecycle=staged
release_quality=patch_only
component_count=46469
distinct_component_hashes=46469
```

```text
POST /api/model-version/releases/{release_id}/index-units
status=200
unit_count=1470
member_count=46469
unresolved_member_count=42565
```

```text
POST /api/model-version/releases/{release_id}/index-assets?materialize=false
status=200
geo_hash_count=59
present_count=58
missing_count=1
glb_readable_count=58
```

Quality gate validation:

```text
POST /api/model-version/releases/{release_id}/state-machine
status=200
applied=false
transition_allowed=false
blockers=baseline evidence missing; mesh asset index was missing before index-assets; release quality is patch_only; asset manifest evidence missing
```

Decision:

- The incremental model generation and model-version handoff path is now
  working for DB1112 `791 -> 897`.
- This artifact is a valid staged `patch_only` release, not a production
  `complete_visual` release.
- Production completion still requires `scene_tree/1112.tree`, the remaining
  missing mesh asset, and a final two-pane compare using two `complete_visual`
  releases.

## 2026-06-21 — DB1112 Scene Tree Artifact Restore

Goal:

- Close the `scene_tree/1112.tree` evidence gap without relying on an opaque
  long-running `--gen-indextree 1112` process.
- Preserve the current site's DB identity while restoring the proven DB1112
  tree artifact from the physical sesno 897 baseline workspace.

Implementation:

- Added `model-version restore-scene-tree-artifact`.
- The command validates a source scene_tree directory containing
  `<dbnum>.tree` and `db_meta_info.json`, computes SHA-256, checks ref0 mapping
  conflicts, copies the tree file atomically, and merges only the target dbnum's
  missing ref0 mappings into the target `db_meta_info.json`.
- It deliberately keeps an existing target `db_files[dbnum]` entry instead of
  overwriting the current station file path with the physical baseline path.

Validation:

```text
dry_run:
  source_scene_tree_dir=output\AvevaMarineSample\model_versions\physical_baselines\http-prepare-physical-1112-smallchunk-long-20260620-1113\output\AvevaMarineSample\scene_tree
  target_scene_tree_dir=output\AvevaMarineSample\scene_tree
  source_latest_sesno=897
  target_latest_sesno_before=897
  tree_would_copy=true
  added_ref0s=[25688]
```

```text
restore:
  target_tree=output\AvevaMarineSample\scene_tree\1112.tree
  bytes=8979086
  sha256=e30df205fc690c7fb1ef89ebbdfb88faafea7af87731b9831b103e28c7f43389
  db_meta 1112 ref0s=9304,17496,25688
  ref0_to_dbnum[25688]=1112
```

```text
validate-history-replay --require-scene-tree:
  classification=quarantined_visual_release_candidate
  ready_for_publish=true
  scene_tree.tree_file_exists=true
  scene_tree.db_meta_info_exists=true
```

Operational finding:

- Direct `aios-database --gen-indextree 1112` was stopped after about 15
  minutes with no `1112.tree` output and no task metrics. That path is not
  acceptable as the current production restore mechanism without additional
  progress evidence.

Self review:

- This closes the missing scene_tree artifact for the current DB1112 sesno 897
  workspace.
- It does not solve `complete_visual`: the physical 791/897 packages still have
  quarantined missing mesh rows, and the 791 release still carries
  `spec_info_fallback` risk.

## 2026-06-21 — Release Readiness Validation-Flag Hardening

Goal:

- Prevent production compare readiness from being bypassed by manually
  annotating a risky release as `complete_visual`.

Implementation:

- `compare_readiness` now treats selected release validation flags as hard
  readiness problems:
  - `mesh_missing_rows_quarantined`
  - `spec_info_fallback`
  - `spec_info_fallback_unquantified`
  - `incremental_handoff_affected_scope`
  - `tree_index_missing*`

Validation:

```text
cargo fmt --check
  passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
  passed with existing warnings only

model-version validate-compare-readiness \
  --from-release-id codex-ams1112-physical-791-quarantine \
  --to-release-id codex-ams1112-physical-897-quarantine \
  --json

classification=quarantined_visual
production_ready=false
from.problems includes:
  release has quarantined missing mesh rows
  release has spec_info fallback validation risk
to.problems includes:
  release has quarantined missing mesh rows
```

Self review:

- This is a production safety improvement. It does not make the current pair
  complete; it makes the current pair harder to accidentally misclassify.

## 2026-06-21 — Missing Mesh Repair Attempt

Goal:

- Determine whether the current `repair-missing-meshes --retry-bad` path can
  repair the quarantined geometry rows and unblock `complete_visual`.

Validation:

```text
791 release:
  command=model-version repair-missing-meshes --retry-bad
  report=output\AvevaMarineSample\model_versions\releases\codex-ams1112-physical-791-quarantine\parquet\1112\missing_mesh_report_1112.json
  output=target\codex-mesh-repair\repair-791-retry-bad.out
  requested_hashes=22
  attempted_hashes=22
  generated_hashes=0
  still_missing_hashes=22
  first_status=generation_failed_bad
```

```text
897 release:
  command=model-version repair-missing-meshes --retry-bad
  report=output\AvevaMarineSample\model_versions\releases\codex-ams1112-physical-897-quarantine\parquet\1112\missing_mesh_report_1112.json
  output=target\codex-mesh-repair\repair-897-retry-bad.out
  requested_hashes=23
  attempted_hashes=23
  generated_hashes=0
  still_missing_hashes=23
  first_status=generation_failed_bad
```

Observed generator errors include CSG profile failures such as self-intersecting
constraint edges and profiles with fewer than three vertices.

Decision:

- The current missing-mesh repair entrypoint is operational, but it cannot
  repair this DB1112 quarantine set.
- The existing 791/897 pair must remain diagnostic `quarantined_visual`.
- A production `complete_visual` pair requires a geometry-generation fix for
  these CSG/profile failures or a new, explicitly reviewed non-visual
  classification that removes the affected rows from the release contract.

Self review:

- This is a real CLI validation with saved outputs.
- It proves the current release pair cannot honestly be promoted to
  `complete_visual` in this turn.

## 2026-06-21 — Oracle Follow-Up And HTTP/Browser Readiness Verification

Oracle session:

```text
session=e3d-version-ducklake-compact-plan
status=completed
model=gpt-5.5-pro
mode=browser/foreground
transcript=C:\Users\dpc\.oracle\sessions\e3d-version-ducklake-compact-plan\artifacts\transcript.md
```

Oracle review conclusion adopted for the architecture:

- Keep the flow:
  `SourceObservation -> BaselineState -> IncrementEvidence -> GenerationJob -> ReleasePackage -> DuckLake projection -> API -> two-pane compare`.
- Make `ReleasePackage` the immutable truth. User-visible model version is
  `release_id + package_hash`.
- Treat `sesno`, source DB file, physical snapshot, SurrealDB workspace, and
  DuckLake catalog as anchors/projections, not as model version identity.
- Scope DuckLake to append-only projection/read-model: release/component/unit
  graph, query acceleration, diff/impact queries, and audit events. It must not
  become generation writer, baseline restore source, job truth, or UI version id.

HTTP validation:

```text
web_server=http://127.0.0.1:3198
pid=62636
config=target\codex-web-server\readiness-hardening-20260621-150457\DbOption-web-smoke.toml
logs=target\codex-web-server\readiness-hardening-20260621-150457
```

```text
GET /api/version
status=200
version=0.3.34
buildDate=2026-06-21 14:56:29 UTC+8
```

```text
GET /api/model-version/compare-readiness
from=codex-ams1112-physical-791-quarantine
to=codex-ams1112-physical-897-quarantine
status=200
success=true
classification=quarantined_visual
production_ready=false
both_published=true
both_complete_visual=false
component_indexes_ready=true
mesh_assets_ready=true
from.problems=mesh_missing_rows_quarantined; spec_info_fallback
to.problems=mesh_missing_rows_quarantined
json=target\codex-web-server\readiness-hardening-20260621-150457\compare-readiness.json
```

```text
GET /api/model-version/diff?limit=50
status=200
summary.added=5059
summary.deleted=2525
summary.changed=43
summary.unchanged=23549
summary.emitted=50
json=target\codex-web-server\readiness-hardening-20260621-150457\diff-limit50.json
```

```text
GET /model-version/compare
status=200
title=Model Version Compare
html=target\codex-web-server\readiness-hardening-20260621-150457\compare-page.html
```

Browser validation:

```text
url=http://127.0.0.1:3198/model-version/compare?from=codex-ams1112-physical-791-quarantine&to=codex-ams1112-physical-897-quarantine&viewer_limit=200&diff_limit=50
title=Model Version Compare
visible_readiness=quarantined_visual / not production ready
iframe_count=2
diff_rows=50
screenshot=.planning\2026-06-17-ducklake-valv-version-diff\model-version-compare-791-897-readiness-hardening-agent-browser.png
```

Canvas/screenshot pixel check:

```text
left_viewer.non_white_ratio=0.2820
right_viewer.non_white_ratio=0.2845
```

The temporary browser session was closed and the temporary web_server process
was stopped after validation.

Additional restore helper hardening:

```text
restore_scene_tree_artifact now replaces existing target files by moving the
old file to a per-process backup first. If the final rename fails, the backup
is restored instead of leaving the target path empty.
```

Smoke validation against a temporary target:

```text
command=model-version restore-scene-tree-artifact --json
target=target\codex-scene-tree-restore-smoke\20260621-restore-helper\scene_tree
tree_copied=true
db_meta_written=true
source_tree_sha256=e30df205fc690c7fb1ef89ebbdfb88faafea7af87731b9831b103e28c7f43389
target_tree_sha256_after=e30df205fc690c7fb1ef89ebbdfb88faafea7af87731b9831b103e28c7f43389
output=target\codex-scene-tree-restore-smoke\20260621-restore-helper\restore-scene-tree-artifact.json
```

Self review:

- The two-pane compare UI is usable for diagnostic/quarantine comparison and
  visibly loads both release-local 3D panes.
- The readiness gate now correctly prevents the current DB1112 791/897 pair
  from being treated as production `complete_visual`.
- The remaining production blocker is not the compare architecture; it is the
  unrepaired bad CSG/profile geometry behind quarantined missing mesh rows.

## 2026-06-21 — Missing Mesh Classification And Degraded CSG Repair

Oracle/MCP note:

```text
mcp__oracle.consult=Transport closed
mcp__oracle.sessions=Transport closed
fallback=Oracle CLI session history reused
completed_session=e3d-version-ducklake-compact-plan
```

Implemented:

```text
D:\work\plant-code\rs-core\src\prim_geo\profile_processor.rs
  ProfileProcessor now skips degenerate inner-hole contours instead of failing
  the whole extrusion profile.

D:\work\plant-code\rs-core\src\geometry\csg.rs
  Added opt-in FRADIUS fallback:
    AIOS_CSG_ALLOW_DEGRADED_PROFILE_FALLBACK=1
    AIOS_CSG_DEGRADED_PROFILE_FALLBACK_LOG=<path>
  When enabled, failed extrusion profiles with nonzero FRADIUS retry with
  zero radii and append an audit log row on success.

src\version_management\missing_mesh_repair.rs
  Missing mesh repair now classifies non-renderable source inputs before retry:
    Unknown / CompoundShape without generator
    PrimExtrusion with no wires, zero height, or no wire with >=3 distinct points
    PrimPolyhedron without any polygon loop with >=3 distinct points
  JSON response now includes degraded fallback evidence fields.

src\version_management\ducklake_store.rs
  compare readiness treats validation flag degraded_geometry_fallback as a
  hard production blocker.
```

Validation:

```text
cargo fmt --check
status=passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings
```

DB1112 897 repair after degenerate-inner-hole skip:

```text
output=target\codex-mesh-repair\repair-897-after-hole-skip-20260621\repair-897-after-hole-skip.out
requested_hashes=23
generated_hashes=3
generated_glbs=12654697601786016860,8937215077082469557,14319542730487621827
still_missing_hashes=20
```

DB1112 897 repair with degraded FRADIUS fallback enabled:

```text
output=target\codex-mesh-repair\repair-897-degraded-fallback-20260621\repair-897-degraded-fallback.out
degraded_log=target\codex-mesh-repair\repair-897-degraded-fallback-20260621\degraded-fradius.log
requested_hashes=23
skipped_existing=3
non_renderable_inputs=8
attempted_hashes=12
generated_hashes=3
degraded_fradius_fallback_rows=3
still_missing_hashes=17
new_generated_glbs=17460324199787200015,2906034641340674981,6035494803822051812
recommended_action=register as degraded_visual with validation flag degraded_geometry_fallback unless reviewed
```

DB1112 791 repair with degraded FRADIUS fallback enabled:

```text
output=target\codex-mesh-repair\repair-791-degraded-fallback-20260621\repair-791-degraded-fallback.out
requested_hashes=22
skipped_existing=6
missing_inst_geo=1
non_renderable_inputs=6
attempted_hashes=9
generated_hashes=0
degraded_fradius_fallback_rows=0
still_missing_hashes=16
```

Important evidence limitation:

- The rs-core fallback log currently records the generator refno as `0_0` for
  these inst_geo-driven repairs, so the authoritative geo_hash mapping remains
  the `repair-missing-meshes` JSON rows.
- Fallback-generated GLBs are useful for diagnostic two-pane visual comparison,
  but they are not exact geometry evidence. They must keep the release out of
  `complete_visual` unless a later review signs off the approximation.

Current conclusion:

- The backend model generation issue is partially repaired, not solved.
- The compare UI architecture and DuckLake projection are no longer the main
  blocker.
- Production `complete_visual` still requires either exact CSG/profile fixes
  for the remaining self-intersecting profiles or an explicit non-visual/
  degraded contract in the immutable ReleasePackage.

## 2026-06-21 — Self-Intersect Mesh Evidence Contract

Oracle evidence:

```text
session=e3d-version-ducklake-compact-plan
status=completed and rendered through Oracle CLI
new_oracle_dry_run_tokens=488767
new_browser_or_api_run=not started
architecture_decision=ReleasePackage truth; DuckLake append-only projection/read-model
```

Implemented:

```text
src\version_management\types.rs
  ModelMissingMeshRepairResponse now includes self_intersecting_inputs.

src\version_management\missing_mesh_repair.rs
  repair-missing-meshes now detects self-intersecting PrimExtrusion wires.
  Self-intersecting source profiles are reported as self_intersecting_input
  instead of remaining as opaque generation_failed_bad rows.
  FRADIUS rows may still be attempted when the explicit degraded fallback env
  flag is enabled, but failed attempts are reclassified with source geometry
  evidence.

src\version_management\ducklake_store.rs
  compare readiness treats validation flags self_intersecting_input and
  self_intersecting_profile as hard production blockers.
```

Validation:

```text
cargo fmt --check
status=passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings
```

DB1112 897 classified repair:

```text
output=target\codex-mesh-repair\repair-897-self-intersect-classify-20260621\repair-897-self-intersect-classify.out
requested_hashes=23
skipped_existing=6
missing_inst_geo=0
non_renderable_inputs=8
self_intersecting_inputs=9
attempted_hashes=5
generated_hashes=0
degraded_fradius_fallback_rows=0
still_missing_hashes=17
statuses=already_present:6, non_renderable_input:8, self_intersecting_input:9
```

DB1112 791 classified repair:

```text
output=target\codex-mesh-repair\repair-791-self-intersect-classify-20260621\repair-791-self-intersect-classify.out
requested_hashes=22
skipped_existing=6
missing_inst_geo=1
non_renderable_inputs=6
self_intersecting_inputs=9
attempted_hashes=5
generated_hashes=0
degraded_fradius_fallback_rows=0
still_missing_hashes=16
statuses=already_present:6, missing_inst_geo:1, non_renderable_input:6, self_intersecting_input:9
```

Current conclusion:

- The remaining missing mesh rows are now quality-classified rather than
  opaque generator failures.
- The current 791/897 releases must stay `quarantined_visual` until exact
  profile repair or an explicit ReleasePackage-level non-visual/degraded
  sign-off exists.
- DuckLake remains only a projection/read-model; it must not hide or reinterpret
  these source-geometry blockers.

## 2026-06-21 — Metadata-Driven Mesh Quality Flags

Goal:

- Remove one manual step from release registration: if a caller passes the
  `repair-missing-meshes` summary in release metadata, registration/publish now
  persists the corresponding validation flags automatically.

Implementation:

```text
src\version_management\model_release.rs
  validation_flags_from_metadata() now also reads missing_mesh_repair,
  mesh_repair, or repair_missing_meshes metadata objects.
  The inferred flags are:
    still_missing_hashes > 0              -> mesh_missing_rows_quarantined
    degraded_fradius_fallback_rows > 0    -> degraded_geometry_fallback
    self_intersecting_inputs > 0          -> self_intersecting_input
    non_renderable_inputs > 0             -> non_renderable_input
    missing_inst_geo > 0                  -> missing_inst_geo

src\version_management\ducklake_store.rs
  compare readiness now treats non_renderable_input and missing_inst_geo as
  hard production blockers, alongside self_intersecting_input.
```

Validation:

```text
rustfmt --edition 2024 --check src\version_management\model_release.rs src\version_management\ducklake_store.rs src\version_management\missing_mesh_repair.rs src\version_management\types.rs
status=passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings
```

CLI smoke:

```text
catalog=target\codex-metadata-flag-smoke\20260621-self-intersect-flags\metadata.ducklake
release_id=codex-metadata-flag-smoke-897
metadata.missing_mesh_repair.still_missing_hashes=17
metadata.missing_mesh_repair.self_intersecting_inputs=9
metadata.missing_mesh_repair.non_renderable_inputs=8

registered_flags=mesh_missing_rows_quarantined,self_intersecting_input,non_renderable_input
```

Readiness smoke:

```text
classification=quarantined_visual
production_ready=false
from_flags=mesh_missing_rows_quarantined,self_intersecting_input,non_renderable_input
from.problems include:
  release contains non-renderable source geometry
  release contains self-intersecting source profiles
  release has quarantined missing mesh rows
```

Known validation note:

- Global `cargo fmt --check` is currently blocked by an unrelated formatting
  diff in `src\web_api\mbd_pipe_api.rs`; this continuation did not touch that
  file.

## 2026-06-21 — ReleasePackage Sidecar Quality Evidence

Goal:

- Make each immutable ReleasePackage self-explanatory enough to survive being
  moved or inspected outside the DuckLake catalog.
- Keep DuckLake as the query/index projection and avoid changing Parquet
  package hashes or schema for this metadata-only hardening slice.

Implementation:

```text
src\version_management\model_release.rs
  register_model_release() now writes <release-root>/<release-id>/release.json.
  publish_history_model_release() rewrites the same sidecar after the final
  Published status is read back from DuckLake.
  annotate_model_release() rewrites the sidecar after quality/flag annotation.
```

The sidecar records:

```text
schema_version=model_release_sidecar:v1
release_id/project/branch/dbnum
release_lifecycle/release_quality/release_quality_reason/release_status
validation_flags/spec_info_fallback_count
package_hash/rows_by_table/source_manifest_hash
baseline_state_manifest_hash/asset_manifest_hash when present
```

Validation:

```text
rustfmt --edition 2024 --check src\version_management\model_release.rs src\version_management\ducklake_store.rs src\version_management\missing_mesh_repair.rs src\version_management\types.rs
status=passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings
```

CLI smoke:

```text
catalog=target\codex-release-sidecar-smoke\20260621-release-json\metadata.ducklake
release_id=codex-sidecar-smoke-897
source_package=output\AvevaMarineSample\model_versions\releases\codex-ams1112-physical-897-quarantine\parquet\1112
sidecar=target\codex-release-sidecar-smoke\20260621-release-json\releases\codex-sidecar-smoke-897\release.json
sidecar.schema_version=model_release_sidecar:v1
sidecar.release_quality=quarantined_visual
sidecar.release_lifecycle=staged
sidecar.validation_flags=mesh_missing_rows_quarantined,self_intersecting_input,non_renderable_input
sidecar.rows_by_table.instances=28651
sidecar.rows_by_table.geo_instances=28496
```

Review:

- This closes a package portability gap without adding a new database
  migration.
- The sidecar intentionally does not participate in the existing Parquet
  `package_hash`; it describes the release wrapper, while `package_hash`
  remains the deterministic hash of the viewer package payload.

## 2026-06-21 — Reconcile Gate for ReleasePackage Sidecar

Goal:

- Make `reconcile-release` verify the new `release.json` sidecar instead of
  merely writing it during registration.
- Keep this in the existing reconcile path so operators have one release health
  check, not another command to remember.

Implementation:

```text
src\version_management\types.rs
  ModelReleaseReconcileReport now includes:
    release_sidecar_path
    release_sidecar_exists
    release_sidecar_hash

src\version_management\ducklake_store.rs
  reconcile_release() now requires <release-root>/release.json.
  It parses the sidecar and checks key fields against DuckLake release record:
    schema_version
    release_id/project/branch/dbnum
    release_lifecycle/release_quality/release_quality_reason/release_status
    package_hash
    validation_flags
    rows_by_table
```

Validation:

```text
rustfmt --edition 2024 --check src\version_management\ducklake_store.rs src\version_management\types.rs src\version_management\model_release.rs
status=passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings
```

CLI smoke, sidecar present:

```text
catalog=target\codex-release-sidecar-reconcile\20260621-sidecar-gate\metadata.ducklake
release_id=codex-sidecar-reconcile-897
output=target\codex-release-sidecar-reconcile\20260621-sidecar-gate\reconcile-sidecar-present.out
release_sidecar_exists=true
release_sidecar_hash=f2dcff034416efafd41360fe92a5ebf030d631fd15235b37cacbec8aaf38c6ad
sidecar_problems=0
other_expected_problem=mesh asset index is missing for visual release
```

CLI smoke, sidecar missing:

```text
output=target\codex-release-sidecar-reconcile\20260621-sidecar-gate\reconcile-sidecar-missing.out
release_sidecar_exists=false
release_sidecar_hash=null
problems include:
  release sidecar is missing
```

HTTP validation note:

- `web_server` binary builds with the new response shape.
- A local HTTP smoke was attempted on ports `3217` and `3218`, but the service
  did not expose `/api/model-version/releases` within the 30s/180s readiness
  windows because startup was still busy with SurrealDB init, E3D increment
  scan, and full route initialization. Logs are recorded at:
  `target\codex-release-sidecar-reconcile\20260621-sidecar-gate\web_server-3217.log`
  and `web_server-3218.log`.
- No panic was observed in the captured logs; this is a service-startup
  validation limitation for this slice, not a compile/runtime failure of the
  reconcile code path.

Review:

- This closes the obvious drift gap: a release can no longer reconcile as
  publishable if DuckLake says one thing and `release.json` is missing or
  inconsistent.
- Existing older releases without sidecars will now fail reconcile until they
  are re-registered or annotated by a build that writes `release.json`; this is
  intentional for production package evidence.

## 2026-06-21 — Sidecar Sync After State Transitions

Goal:

- Prevent `release.json` from becoming stale when a state transition is applied
  by `reconcile-release` or the release state machine.
- Keep the fix in the existing status-change paths; do not add a new repair
  command.

Implementation:

```text
src\version_management\model_release.rs
  reconcile_model_release() writes release.json when reconcile applies a
  status transition such as --fail-if-unusable or --publish-if-complete.

src\version_management\release_state_machine.rs
  run_model_release_state_machine() writes release.json after a successful
  publish_if_ready or fail_if_unusable transition.
```

Why:

- Before this slice, DuckLake status could change from `staged` to `failed` or
  `published` while the sidecar still said `staged`.
- The next reconcile would then correctly reject the package for sidecar
  mismatch, creating a self-inflicted drift after a valid transition.

Validation:

```text
rustfmt --edition 2024 --check src\version_management\model_release.rs src\version_management\release_state_machine.rs src\version_management\ducklake_store.rs src\version_management\types.rs
status=passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings
```

CLI smoke:

```text
catalog=target\codex-release-sidecar-reconcile\20260621-status-sync\metadata.ducklake
release_id=codex-sidecar-status-sync-897

steps:
  register DB1112 897 quarantined release
  reconcile-release --fail-if-unusable
  reconcile-release again

first_reconcile.applied=true
first_reconcile.current_status=failed
sidecar.release_status=failed
sidecar.release_lifecycle=failed
second_reconcile.current_status=failed
second_reconcile.release_sidecar_exists=true
second_reconcile.release_sidecar_hash_present=true
second_reconcile.exact_sidecar_problem_count=0
remaining_expected_problem=mesh asset index is missing for visual release
```

Review:

- This fixes the transition-induced drift without changing DuckLake schema,
  Parquet package hashes, or release state-machine policy.
- HTTP state-machine behavior is covered by `web_server` build here; full HTTP
  smoke remains limited by the slow local `web_server` startup noted in the
  previous section.

## 2026-06-21 — Oracle Architecture Review for DuckLake Model Versions

Goal:

- Continue the architecture analysis with Oracle as requested by the user.
- Re-check whether DuckLake is the right tool for the model-version data layer.
- Convert the review into a concrete architecture/development plan before
  continuing implementation.

Oracle execution:

```text
mcp_attempt=mcp__oracle.consult
mcp_result=Transport closed
fallback=Oracle CLI browser mode
session=e3d-ducklake-architectu-20260621
model=gpt-5.5-pro
dry_run_tokens~=133702
output=target\oracle-e3d-ducklake-architecture-20260621.md
api_paid_call=false
```

Decision:

```text
ReleasePackage = immutable payload truth
DuckLake       = rebuildable projection/read-model
SurrealDB      = generation workspace/cache
HTTP GET       = read-only
HTTP POST      = register/index/reconcile/publish/repair
RID            = release_id + package_hash
```

The review explicitly rejects using DuckLake as:

```text
model generation writer
payload truth
baseline restore source
job truth
UI version id
implicit read-path repair mechanism
```

Architecture document update:

```text
docs\plans\2026-06-21-e3d-model-version-ducklake-oracle-mcp-v3.md
  updated Oracle evidence section
  added section 13.9 with DB1112 E2E flow, P0/P1/P2 plan, and final gates
```

P0 plan captured:

- Unify release validation across `publish-history`, `incremental/handoff`, and
  direct `register`.
- Keep affected-scope incremental handoff out of `complete_visual` unless a
  complete baseline/hydration proof is present.
- Treat sidecar missing/mismatch as a blocker in reconcile/state-machine/readiness.
- Require baseline manifest/hash, generation job id, release-local asset
  manifest/hash, GLB readability evidence, component index, and release sidecar
  before production publish.
- Keep production publish behind state-machine/reconcile gates.

P1 plan captured:

- Add explicit DuckLake projection rebuild/freshness evidence.
- Store projection hashes/events/rule hashes and include them in diff responses.
- Verify deleting DuckLake and rebuilding from ReleasePackage keeps diff results
  invariant.

P2 plan captured:

- Add watcher/replay queue as a long-running service after P0 is closed.
- Use source observation quiescence before queueing work.
- Serialize replay per dbnum, allow cross-dbnum concurrency, and add
  release-local copy-on-write asset reuse.
- Add large-scene runtime pagination/tile support and retention/GC policy.

Current status:

- No code was changed in this slice.
- No new build was required for this analysis-only update.
- The implementation goal remains active because DB1112 still needs two
  production-valid `complete_visual` releases and a final two-pane 3D comparison
  validation.

## 2026-06-21 — Readiness Baseline Gate

Goal:

- Align pair readiness with the existing state-machine production policy.
- Prevent a `complete_visual` release without baseline state manifest evidence
  from being considered production-ready.

Implementation:

```text
src\version_management\ducklake_store.rs
  release_readiness() now upgrades missing baseline state manifest path/hash
  from warning to problem when release_quality=complete_visual.
```

Validation:

```text
rustfmt --edition 2024 --check src\version_management\ducklake_store.rs
status=passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings
```

CLI smoke:

```text
catalog=target\codex-readiness-baseline-gate\20260621\metadata.ducklake
release_id=codex-baseline-gate-complete-897
source_package=output\AvevaMarineSample\model_versions\releases\codex-ams1112-physical-897-quarantine\parquet\1112
release_quality=complete_visual
baseline_state_manifest_path/hash=missing

validate-compare-readiness:
  classification=not_production_ready
  production_ready=false
  from.problems includes release has no baseline state manifest evidence
  to.problems includes release has no baseline state manifest evidence
```

Review:

- This deliberately does not add `release_validator.rs` yet; the existing
  readiness branch was the shortest safe place to close this P0 gap.
- HTTP smoke was not repeated for this tiny shared-core change; `web_server`
  build passed, and prior local HTTP attempts are slow due unrelated startup
  work.

## 2026-06-21 — Index-Assets Sidecar Sync

Goal:

- Prevent explicit asset indexing from making DuckLake and ReleasePackage
  sidecar evidence disagree.

Implementation:

```text
src\version_management\model_release.rs
  index_model_release_mesh_assets() now fetches the updated release row and
  rewrites release.json after store.index_release_mesh_assets().
```

Validation:

```text
rustfmt --edition 2024 --check src\version_management\model_release.rs src\version_management\ducklake_store.rs
status=passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings
```

CLI smoke:

```text
catalog=target\codex-index-assets-sidecar-sync\20260621\metadata.ducklake
release_id=codex-index-assets-sidecar-sync-1112
source_package=output\AvevaMarineSample\parquet\1112

steps:
  model-version register
  model-version index-assets
  model-version reconcile-release

index_assets.asset_index_hash=88095cedb89f8c701cc3c3badc54e1c984dc22a2d81b3a8db6fec869c45d3d16
release_sidecar_exists=true
sidecar_specific_problems=0
release.json.asset_manifest_path == DuckLake release.asset_manifest_path
release.json.asset_manifest_hash == DuckLake release.asset_manifest_hash
remaining_expected_problem=mesh asset index has 5 non release-local or missing asset rows
```

Review:

- This closes a real drift path with one wrapper-level sidecar write.
- No new abstraction was added; `index-assets` is the only index command here
  that mutates release-row provenance fields.

## 2026-06-21 — Compare Readiness Sidecar Gate

Goal:

- Make compare readiness reject a release whose immutable ReleasePackage
  sidecar is missing or inconsistent, instead of relying only on DuckLake
  catalog rows.

Implementation:

```text
src\version_management\ducklake_store.rs
  release_readiness() now calls the existing release sidecar validation path
  and records a problem when release.json is missing.
```

Validation:

```text
rustfmt --edition 2024 --check src\version_management\ducklake_store.rs src\version_management\model_release.rs
status=passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings
```

CLI smoke:

```text
catalog=target\codex-readiness-sidecar-gate\20260621\metadata.ducklake
release_id=codex-readiness-sidecar-gate-1112
source_package=output\AvevaMarineSample\parquet\1112
mutation=delete ReleasePackage release.json

validate-compare-readiness:
  classification=not_production_ready
  production_ready=false
  from.problems includes release sidecar is missing:
  to.problems includes release sidecar is missing:
```

Review:

- This keeps the current architecture simple: ReleasePackage remains truth,
  DuckLake remains projection, and readiness now checks both.
- Full browser/two-pane visual validation is still the next larger milestone,
  after the P0 readiness gates are fully closed.

## 2026-06-21 — Sidecar Evidence Field Gate

Goal:

- Make sidecar/catalog validation cover the source, baseline, and asset
  evidence fields that production compare readiness depends on.

Implementation:

```text
src\version_management\ducklake_store.rs
  validate_release_sidecar() now compares existing release.json evidence
  fields against the DuckLake release row:
    derivation_type
    generation_job_id
    immutable_package_dir/source_package_dir
    source_manifest_path/hash
    baseline_state_manifest_path/hash
    asset_manifest_path/hash
```

Validation:

```text
rustfmt --edition 2024 --check src\version_management\ducklake_store.rs src\version_management\model_release.rs
status=passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings

git diff --check -- src\version_management\ducklake_store.rs src\version_management\model_release.rs
status=passed
```

CLI smoke:

```text
catalog=target\codex-sidecar-evidence-gate\20260621\metadata.ducklake
release_id=codex-sidecar-evidence-gate-1112
source_package=output\AvevaMarineSample\parquet\1112
mutation=release.json source_manifest_hash -> codex-tampered-source-manifest-hash

validate-compare-readiness:
  production_ready=false
  from.problems includes release sidecar source_manifest_hash mismatch:
  to.problems includes release sidecar source_manifest_hash mismatch:
```

Review:

- This closes a concrete readiness evidence gap without adding tables or
  abstractions.
- Full projection rebuild/freshness evidence is still a larger P1 milestone;
  this gate prevents stale ReleasePackage evidence from being silently trusted.

## 2026-06-21 — Evidence File Hash Gate

Goal:

- Ensure sidecar/catalog agreement is backed by readable evidence files with
  matching hashes.

Implementation:

```text
src\version_management\ducklake_store.rs
  validate_release_sidecar() now calls verify_optional_evidence_file() for:
    source_manifest
    baseline_state_manifest
    asset_manifest
```

Validation:

```text
rustfmt --edition 2024 --check src\version_management\ducklake_store.rs src\version_management\model_release.rs
status=passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings
```

CLI smoke:

```text
catalog=target\codex-evidence-file-gate\20260621\metadata.ducklake
release_id=codex-evidence-file-gate-1112
mutation=delete release-local manifest.json

validate-compare-readiness:
  production_ready=false
  from.problems includes release evidence source_manifest is missing:
  to.problems includes release evidence source_manifest is missing:
```

Review:

- The smoke does not mutate the real `output\AvevaMarineSample\parquet\1112`
  package; it uses package copies and release-local artifacts under `target`.
- This keeps the validator boring and fail-closed. Projection rebuild remains a
  separate larger milestone.

## 2026-06-21 — Release-Local Source Manifest Evidence

Goal:

- Keep release evidence self-contained so deleting or rotating the original
  source parquet directory does not break a valid ReleasePackage.

Implementation:

```text
src\version_management\model_release.rs
  register_model_release() now hashes and stores
  package.package_dir\manifest.json as source_manifest_path/hash.
```

Validation:

```text
rustfmt --edition 2024 --check src\version_management\ducklake_store.rs src\version_management\model_release.rs
status=passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings
```

CLI smoke:

```text
catalog=target\codex-release-local-source-manifest\20260621\metadata.ducklake
release_id=codex-release-local-source-manifest-1112
sidecar_source_manifest_path=target\codex-release-local-source-manifest\20260621\releases\codex-bbbc1a08c9192ce0\parquet\1112\manifest.json
source_dir_deleted_without_source_manifest_problem=true
release_local_manifest_deleted_problem=release evidence source_manifest is missing: target\codex-release-local-source-manifest\20260621\releases\codex-bbbc1a08c9192ce0\parquet\1112\manifest.json
```

Review:

- This corrects the durability boundary introduced by evidence-file validation:
  the checked manifest now lives in the immutable release package.
- Existing already-registered releases with external source manifest paths are
  not migrated by this slice.

## 2026-06-21 — Source Manifest Release-Local Gate

Goal:

- Reject source manifest evidence that is outside the immutable ReleasePackage,
  even when the external file exists and its hash matches the catalog.

Implementation:

```text
src\version_management\ducklake_store.rs
  validate_release_sidecar() now calls verify_evidence_path_under()
  for source_manifest_path and immutable_package_dir.
```

Validation:

```text
rustfmt --edition 2024 --check src\version_management\ducklake_store.rs src\version_management\model_release.rs
status=passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings
```

CLI smoke:

```text
catalog=target\codex-source-manifest-release-local-gate\20260621\metadata.ducklake
release_id=codex-source-manifest-local-gate-1112
mutation=source_manifest_path points to target\codex-source-manifest-release-local-gate\20260621\external-source\manifest.json

validate-compare-readiness:
  production_ready=false
  from.problems includes release evidence source_manifest is not release-local:
  to.problems includes release evidence source_manifest is not release-local:
  source_manifest_missing_or_hash_mismatch=false
```

Review:

- This keeps the ReleasePackage as the auditable boundary for production model
  comparison.
- Already-registered releases with external source manifest evidence need
  re-registration or an explicit evidence repair before final production
  sign-off.

## 2026-06-21 — Index-Assets Repair Hint Alignment

Goal:

- Keep readiness/runtime repair guidance executable for operators.

Implementation:

```text
src\version_management\ducklake_store.rs
  compare-readiness recommended_action now uses:
    aios-database model-version index-assets --release-id <id> --materialize
```

Validation:

```text
rustfmt --edition 2024 --check src\version_management\ducklake_store.rs
status=passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings
```

CLI smoke:

```text
aios-database model-version index-assets --help
  has --materialize=true
  has --materialize-assets=false

catalog=target\codex-source-manifest-release-local-gate\20260621\metadata.ducklake
release_id=codex-source-manifest-local-gate-1112

validate-compare-readiness:
  recommended_action contains index-assets --materialize: 2
  recommended_action contains index-assets --materialize-assets: 0
```

Review:

- This is a one-string production repair-path fix, not a behavior change to
  asset indexing.

## 2026-06-21 — Reconcile Evidence Repair

Goal:

- Make explicit `reconcile-release` repair old release evidence drift exposed by
  the new readiness gates.

Implementation:

```text
src\version_management\ducklake_store.rs
  index-assets stores release.asset_manifest_hash as sha256(mesh_assets_manifest.json)
  added repair_release_source_manifest_to_package()

src\version_management\model_release.rs
  reconcile_model_release() repairs old source_manifest evidence,
  writes missing/stale release.json sidecar, then refreshes the report.
```

Validation:

```text
rustfmt --edition 2024 --check src\version_management\ducklake_store.rs src\version_management\model_release.rs
status=passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings
```

DB1112 CLI evidence:

```text
releases:
  codex-ams1112-physical-791-quarantine
  codex-ams1112-physical-897-quarantine

index-assets --materialize:
  791 missing_count=0 glb_unreadable_count=0
  897 missing_count=0 glb_unreadable_count=0

reconcile-release after repair:
  791 action_taken=source manifest evidence repaired
  897 action_taken=source manifest evidence repaired
  sidecar_exists=true
  source_manifest_path=output\AvevaMarineSample\model_versions\releases\<release>\parquet\1112\manifest.json
  source_not_local=0
  asset_manifest_hash_mismatch=0
  problems=[]
```

HTTP evidence:

```text
temporary web_server port=3997
GET /api/model-version/compare-readiness?from_release_id=codex-ams1112-physical-791-quarantine&to_release_id=codex-ams1112-physical-897-quarantine

classification=quarantined_visual
production_ready=false
both_published=true
both_complete_visual=false
mesh_assets_ready=true
remaining_from_problems=mesh_missing_rows_quarantined, spec_info_fallback
remaining_to_problems=mesh_missing_rows_quarantined
```

Review:

- Package/source/asset/sidecar evidence is now consistent for the real DB1112
  quarantine pair.
- Remaining work is true visual-quality closure, not metadata drift.

## 2026-06-21 — Oracle MCP Follow-up and Fresh Compare Link Validation

Goal:

- Continue the Oracle-backed architecture review request and ensure the final
  two-pane compare page can be opened directly from canonical release IDs.

Oracle MCP:

```text
mcp__oracle.consult dryRun:
  engine=browser
  model=gpt-5.5-pro
  files=5
  estimated_tokens=124339
  status=passed

mcp__oracle.consult real browser run:
  status=failed
  reason=private Oracle Chrome profile is not signed in to ChatGPT

api_mode_started=false
reason=API mode can incur usage cost and needs explicit consent
```

Architecture decision recorded:

```text
DuckLake/DuckDB catalog:
  role=query catalog, snapshot inspection, delta analysis
  not role=sole canonical production artifact

Canonical model-version boundary:
  immutable ReleasePackage manifest
  release-local source manifest
  parquet/tree snapshot manifest
  mesh asset manifest
  content hashes
  readiness/reconcile evidence
```

Implementation:

```text
src\web_api\model_version_api.rs
  added queryReleaseId(canonical, legacy)
  compare page accepts:
    from_release_id / to_release_id
    from / to
```

Validation:

```text
rustfmt --edition 2024 --check src\web_api\model_version_api.rs
status=passed

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings
```

Browser evidence:

```text
temporary web_server port=3997
fresh_url=/model-version/compare?from_release_id=codex-ams1112-physical-791-quarantine&to_release_id=codex-ams1112-physical-897-quarantine

top_frame:
  selected_from=codex-ams1112-physical-791-quarantine
  selected_to=codex-ams1112-physical-897-quarantine
  has_quarantined_visual=true
  has_not_production_ready=true
  has_mesh_missing_rows_quarantined=true
  has_spec_info_fallback=true
  iframes=2

from_iframe:
  release=codex-ams1112-physical-791-quarantine
  components=2000/26117
  geometries=2288/2288
  visible_canvas_count=3

to_iframe:
  release=codex-ams1112-physical-897-quarantine
  components=2000/28651
  geometries=2041/2041
  visible_canvas_count=3

screenshot=.planning\2026-06-17-ducklake-valv-version-diff\model-version-compare-after-evidence-repair-agent-browser.png
```

Review:

- The DB1112 791/897 pair now opens directly as two visible 3D panes from
  canonical release-id query parameters.
- This closes a usability gap in the final comparison path, but does not change
  readiness: the pair remains `quarantined_visual` until true visual-quality
  blockers are closed.

## 2026-06-21 — Missing Mesh Repair Immutable Boundary

Goal:

- Keep missing-mesh repair aligned with the production ReleasePackage boundary.

Implementation:

```text
src\version_management\missing_mesh_repair.rs
  non-dry-run repair now refuses mesh_root under model_versions\releases\...
  override escape hatch: AIOS_ALLOW_RELEASE_PACKAGE_MESH_REPAIR=1
```

Validation:

```text
rustfmt --edition 2024 --check src\version_management\missing_mesh_repair.rs
status=passed after formatting

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings
```

DB1112 repair evidence:

```text
dry_run_release_local:
  791 requested=22 missing_inst_geo=19 non_renderable=1 still_missing=20
  897 requested=23 missing_inst_geo=20 non_renderable=1 still_missing=21

plain_repair_release_local_before_guard:
  791 attempted=2 generated=0 still_missing=22
  897 attempted=2 generated=0 still_missing=23
  attempted rows became generation_failed_bad

degraded_fallback_scratch_smoke:
  env=AIOS_CSG_ALLOW_DEGRADED_PROFILE_FALLBACK=1
  791 attempted=2 generated=2 degraded_fradius_fallback_rows=2 still_missing=20
  897 attempted=2 generated=2 degraded_fradius_fallback_rows=2 still_missing=21
  recommended_action=register degraded_visual + degraded_geometry_fallback

guard_after_implementation:
  release-local non-dry-run exit_code=1
  error=refusing to write missing-mesh repair into immutable ReleasePackage path
  scratch non-dry-run exit_code=0
```

Review:

- No existing release should be mutated to hide missing mesh evidence.
- Degraded fallback can be used only to create a new degraded release with
  explicit evidence; it does not satisfy `complete_visual`.

## 2026-06-21 — Spec Info Fallback Readiness Evidence

Goal:

- Make spec-info fallback evidence explicit enough for DB1112 model-version
  comparison operators.

Implementation:

```text
src\version_management\ducklake_store.rs
  release_validation_flag_problems now reports:
    known count -> release has <count> spec_info fallback rows
    unknown count -> release has unquantified spec_info fallback risk
```

Validation:

```text
rustfmt --edition 2024 --check src\version_management\ducklake_store.rs
status=passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings
```

Evidence:

```text
cli_json=target\codex-db1112-791-897-readiness-after-spec-message-20260621.json
http_json=target\codex-spec-message-web-20260621\compare-readiness.json

http_result:
  classification=quarantined_visual
  production_ready=false
  mesh_assets_ready=true
  from_problem_count=2
  to_problem_count=1
  has_unquantified_spec_info_message=true
```

Review:

- This fixes evidence ambiguity only. The real DB1112 pair remains
  `quarantined_visual` until missing mesh and spec-info quality blockers are
  repaired or explicitly released under a lower visual-quality contract.

## 2026-06-21 — Generated Spec Info Fallback Count

Goal:

- Stop relying on manual `--spec-info-fallback-count` when a generated parquet
  package can carry the evidence itself.

Implementation:

```text
src\fast_model\export_model\export_dbnum_instances_parquet.rs
  added internal spec_info_fallback markers for emitted instance/tubing rows
  writes manifest fields:
    spec_info_fallback_count
    spec_info_validation.fallback_count
    spec_info_validation.instance_fallback_rows
    spec_info_validation.tubing_fallback_rows

src\version_management\model_release.rs
  register_model_release count source priority:
    explicit request -> metadata JSON -> package manifest
  adds spec_info_fallback flag when count > 0

src\cli_modes.rs
  prints spec_info fallback 数量 in the parquet export summary
```

Validation:

```text
sigmap ask "spec_info_fallback model release readiness where sidecar count generated"
status=timed out after 124s; fell back to rg/direct reads

rustfmt --edition 2024 --check src\fast_model\export_model\export_dbnum_instances_parquet.rs src\version_management\model_release.rs
status=passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings
```

CLI evidence:

```text
synthetic_register_smoke=target\codex-spec-info-manifest-register-smoke-20260621\assertion.json
  manifest_count=7
  sidecar_count=7
  validation_flags=spec_info_fallback

real_db1112_export=target\codex-db1112-spec-info-manifest-export-20260621\assertion.json
  manifest=target\codex-db1112-spec-info-manifest-export-20260621\1112\manifest.json
  spec_info_fallback_count=40072
  instance_fallback_rows=40072
  tubing_fallback_rows=0
  instances_rows=47490
  geo_instances_rows=163

real_db1112_register_smoke=target\codex-db1112-spec-info-real-register-20260621\assertion.json
  sidecar_count=40072
  validation_flags=spec_info_fallback
```

Review:

- This makes future generated releases self-report spec fallback risk.
- Existing 791/897 releases remain unchanged and still must not be promoted to
  `complete_visual` without matching historical evidence or regeneration.

## 2026-06-22 — Legacy Spec Info Audit Gate

Goal:

- Provide a read-only operator gate for legacy release packages that predate
  generated `spec_info_fallback_count` evidence.

Implementation:

```text
src\version_management\cli.rs
  added:
    model-version audit-spec-info --release-id <ID> --project <PROJECT> --json

  reads:
    release immutable package manifest
    instances.parquet spec_value column
    tubings.parquet spec_value column

  writes:
    nothing
```

Validation:

```text
sigmap ask "model release spec_info_fallback_count reconcile package manifest existing release 791 897 readiness"
status=timed out after 124s; fell back to rg/direct reads

rustfmt --edition 2024 --check src\version_management\cli.rs
status=passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings
```

DB1112 evidence:

```text
summary=target\codex-db1112-spec-info-legacy-audit-20260622\summary.json

codex-ams1112-physical-791-quarantine:
  manifest_count=null
  instance_rows=26117
  instance_zero=26117
  tubing_rows=56
  tubing_zero=56
  legacy_zero=26173

codex-ams1112-physical-897-quarantine:
  manifest_count=null
  instance_rows=28651
  instance_zero=28651
  tubing_rows=42
  tubing_zero=42
  legacy_zero=28693
```

Review:

- 897 is not clean just because it lacked the historical `spec_info_fallback`
  flag.
- This stays lazy and safe: no automatic annotation, no catalog mutation, no
  HTTP readiness parquet scan on page load.

## 2026-06-22 — Manifest-Level Spec Info Evidence Gate

Goal:

- Make readiness enforce generated package evidence instead of relying only on
  historical sidecar flags.

Implementation:

```text
src\version_management\types.rs
  added readiness evidence fields:
    spec_info_manifest_evidence_present
    spec_info_manifest_fallback_count

src\version_management\ducklake_store.rs
  compare-readiness now reads release package manifest.json
  complete_visual without generated spec-info evidence becomes a problem
  quarantine/degraded without generated spec-info evidence becomes a warning
```

Validation:

```text
sigmap ask "model release compare readiness manifest spec_info_validation missing evidence production gate"
status=timed out after 124s; fell back to rg/direct reads

rustfmt --edition 2024 --check src\version_management\ducklake_store.rs src\version_management\types.rs src\version_management\cli.rs src\version_management\model_release.rs src\version_management\missing_mesh_repair.rs src\web_api\model_version_api.rs src\fast_model\export_model\export_dbnum_instances_parquet.rs src\cli_modes.rs
status=passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings
```

Evidence:

```text
cli_json=target\codex-db1112-readiness-spec-manifest-gate-20260622.json
http_json=target\codex-readiness-spec-manifest-web-20260622\compare-readiness.json

result:
  classification=quarantined_visual
  production_ready=false
  from.spec_info_manifest_evidence_present=false
  to.spec_info_manifest_evidence_present=false
  warnings include manifest lacks generated spec_info fallback evidence
```

Review:

- DuckLake remains the best fit for version catalog and incremental release
  indexing; generated model data quality belongs in immutable package manifests.
- 791/897 remain useful for two-pane visual comparison, but not production-ready
  visual baselines.

## 2026-06-22 — ReleasePackage File Integrity Gate

Goal:

- Make readiness/reconcile catch immutable ReleasePackage payload drift, not
  only sidecar/evidence-manifest drift.

Implementation:

```text
src\version_management\ducklake_store.rs
  reconcile-release validates model_release_files against immutable_package_dir
  compare-readiness validates model_release_files against immutable_package_dir
  checks:
    unsafe relative path
    required file missing
    bytes mismatch
    sha256 mismatch
    catalog file set package_hash mismatch
```

Validation:

```text
sigmap ask "incremental E3D database update model generation version compare DB1112"
status=passed; context=.context\query-context.md

rustfmt --edition 2024 --check src\version_management\ducklake_store.rs
status=passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings
```

Negative smoke:

```text
root=target\codex-package-file-gate-20260622
release_id=codex-package-file-gate-1112
action=append one byte to release-local instances.parquet
result:
  reconcile reported release file bytes mismatch
  reconcile reported release file sha256 mismatch
  compare-readiness reported release file bytes mismatch
  compare-readiness reported release file sha256 mismatch
```

Real DB1112 regression:

```text
cli_json=target\codex-package-file-gate-real-readiness-20260622.json
http_json=target\codex-package-file-gate-http-20260622b\compare-readiness.json

result:
  classification=quarantined_visual
  production_ready=false
  no release file bytes/hash/missing false positive
```

Review:

- This deliberately avoids a schema migration; existing `model_release_files`
  already carries enough evidence.
- It does not make 791/897 production-ready. It only ensures a future
  production release cannot pass readiness after payload files drift.

## 2026-06-22 — CompleteVisual Validation Flag Publish Gate

Goal:

- Prevent manual `complete_visual` annotation or `publish_if_complete` reconcile
  from bypassing existing release validation flags.

Oracle / source review:

```text
npx -y @steipete/oracle --help
oracle session e3d-ducklake-architectu-20260621
sigmap ask "complete_visual validation flags publish gate model release reconcile state machine"
```

Implementation:

```text
src\version_management\ducklake_store.rs
  reconcile-release now:
    blocks publish_if_complete unless release_quality=complete_visual
    blocks complete_visual or publish_if_complete releases when validation_flags
    contain production blockers
```

Validation:

```text
rustfmt --edition 2024 --check src\version_management\ducklake_store.rs
status=passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
warning_scope=existing pdms-io/parse_pdms_db warnings
```

Negative smoke:

```text
root=target\codex-complete-visual-flag-gate-20260622-005325

case 1:
  release_id=codex-complete-visual-flag-gate-1112
  release_quality=complete_visual
  validation_flag=mesh_missing_rows_quarantined
  reconcile --publish-if-complete:
    publishable=false
    applied=false
    blocker=release has quarantined missing mesh rows

case 2:
  release_id=codex-publish-quality-gate-1112
  release_quality=quarantined_visual
  reconcile --publish-if-complete:
    publishable=false
    applied=false
    blocker=release quality is quarantined_visual, expected complete_visual for publish_if_complete
```

Real DB1112 regression:

```text
cli_json_dir=target\codex-complete-visual-flag-gate-real-20260622
http_json_dir=target\codex-complete-visual-flag-gate-http-20260622

result:
  reconcile applied=false
  compare-readiness classification=quarantined_visual
  production_ready=false
  production_comparison_allowed=false
```

HTTP state-machine isolation smoke:

```text
root=target\codex-state-machine-flag-gate-http-20260622-010754
web_server cwd=root
ducklake=root\output\AvevaMarineSample\model_versions
release_id=codex-http-state-machine-flag-gate-1112
release_quality=complete_visual
validation_flag=mesh_missing_rows_quarantined

POST /api/model-version/releases/register
POST /api/model-version/releases/{release_id}/state-machine action=publish_if_ready

result:
  transition_allowed=false
  applied=false
  action_taken=none
  current_status=staged
  current_lifecycle=staged
  blocker=release has quarantined missing mesh rows

evidence:
  target\codex-state-machine-flag-gate-http-20260622-010754\http-register.json
  target\codex-state-machine-flag-gate-http-20260622-010754\http-state-machine-publish-if-ready.json
```

Review:

- This closes a P0 publish safety gap identified by the Oracle architecture
  review: release quality and validation flags must be enforced by the same
  production gate used by reconcile/state-machine.
- No `cargo test` was run, per repository rule.
- 791/897 remain diagnostic releases, not final two-pane production comparison
  baselines.

## 2026-06-22 — Current Two-Pane 3D Compare Regression

Goal:

- Verify that the current DB1112 791/897 diagnostic compare UI still shows two
  3D model panes and exposes the expected non-production readiness state.

Validation:

```text
web_server:
  url=http://127.0.0.1:4026
  exe=E:\codex-targets\plant-cli-ducklake-build\debug\web_server.exe
  config=db_options\DbOption-codex-live-view.toml

GET /api/model-version/compare-readiness
  classification=quarantined_visual
  both_published=true
  both_complete_visual=false
  component_indexes_ready=true
  mesh_assets_ready=true
  production_ready=false
  diff added=5059 changed=43 deleted=2525 unchanged=23549

Browser compare page:
  two iframes present: from model, to model
  readiness banner present: not production ready
  diff table present

Standalone release viewer 791:
  canvasCount=3
  components=2000/26117
  geometries=2288/2288
  failed=0

Standalone release viewer 897:
  canvasCount=3
  components=2000/28651
  geometries=2041/2041
  failed=0
```

Evidence:

```text
target\codex-current-compare-ui-20260622\compare-readiness.json
target\codex-current-compare-ui-20260622\runtime-scene-791-sample.json
target\codex-current-compare-ui-20260622\runtime-scene-897-sample.json
.planning\2026-06-17-ducklake-valv-version-diff\model-version-compare-current-791-897-20260622-agent-browser.png
.planning\2026-06-17-ducklake-valv-version-diff\release-viewer-791-20260622.png
.planning\2026-06-17-ducklake-valv-version-diff\release-viewer-897-20260622.png
```

Review:

- The two-pane model comparison is currently usable for diagnosis and visual
  inspection.
- It remains correctly blocked for production because neither side is
  `complete_visual`.

## 2026-06-22 — Builtin/Sentinel Geo Hash Mesh-Gate Fix

Goal:

- Remove the false missing-mesh signal caused by treating `geo_hash=0` as a
  required external GLB asset.

Implementation:

```text
src\fast_model\export_model\export_dbnum_instances_parquet.rs
src\version_management\ducklake_store.rs
src\web_server\site_data_validation.rs
src\version_management\missing_mesh_repair.rs

Change:
  is_builtin_geo_hash now matches 0/1/2/3.
  repair-missing-meshes skips 0/1/2/3 while ingesting report hashes.
```

Validation:

```text
rustfmt --edition 2024 --check <changed rust files>
status=passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed

cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
```

DB1112 report proof:

```text
897 old_missing_hashes=23
897 old_missing_rows=208
897 external_missing_hashes=22
897 external_missing_rows=39
897 skipped_builtin_or_sentinel_rows=169

791 old_missing_hashes=22
791 external_missing_hashes=22
791 skipped_builtin_or_sentinel_rows=0
```

CLI proof:

```text
command=repair-missing-meshes --dry-run --retry-bad --json
json=target\codex-builtin-geo-hash-fix-20260622\repair-897-dry-run.json
requested_hashes=23
row_count=22
has_zero_row=false
```

HTTP proof:

```text
web_server=http://127.0.0.1:4031
POST /api/admin/auth/login
POST /api/admin/sites/quicktest-7997-8080/deploy-validation
json=target\codex-builtin-geo-hash-fix-20260622\web-auth\deploy-validation-quicktest-7997-8080.json
result=success=true
mesh_refs_sample_7997=pass
```

Regression:

```text
json=target\codex-builtin-geo-hash-fix-20260622\compare-readiness-791-897.json
classification=quarantined_visual
production_ready=false
both_complete_visual=false
```

Review:

- This is not a production promotion. It only removes a false missing-mesh
  bucket so future regeneration works from a cleaner blocker set.
- Existing 791/897 releases remain correctly blocked until complete visual
  packages are regenerated and validated.

## 2026-06-22 — Spec Info Fallback Quantification

Goal:

- Replace the unquantified spec_info blocker with auditable counts for the
  current 791/897 quarantine releases.

Implementation:

```text
src\version_management\ducklake_store.rs

Change:
  annotate --spec-info-fallback-count now removes spec_info_fallback_unquantified.
  count > 0 also ensures spec_info_fallback remains present.
```

Validation:

```text
rustfmt --edition 2024 --check src\version_management\ducklake_store.rs
status=passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed
```

Audit evidence:

```text
791 legacy_zero_spec_value_count=26173 instances=26117/26117 tubings=56/56
897 legacy_zero_spec_value_count=28693 instances=28651/28651 tubings=42/42
```

CLI proof:

```text
target\codex-builtin-geo-hash-fix-20260622\annotate-spec-info-791.json
quality=quarantined_visual
spec_info_fallback_count=26173
flags=mesh_missing_rows_quarantined,spec_info_fallback

target\codex-builtin-geo-hash-fix-20260622\annotate-spec-info-897.json
quality=quarantined_visual
spec_info_fallback_count=28693
flags=mesh_missing_rows_quarantined,spec_info_fallback
```

Regression:

```text
target\codex-builtin-geo-hash-fix-20260622\compare-readiness-791-897-after-spec-annotation.json
classification=quarantined_visual
production_ready=false
both_complete_visual=false
```

Review:

- This reduced ambiguity but did not reduce the real production blocker.
- The next production-grade fix must regenerate/populate non-zero spec_info,
  then publish new complete visual candidates.

## 2026-06-22 — Spec Info Generation Repair

Goal:

- Fix the model package generation path so DB1112 does not emit all-zero
  `spec_value` for AvevaMarineSample professional SITE data.

Implementation:

```text
src\fast_model\export_model\spec_info.rs
  SITE token mapping now covers:
    ELEC / DIANQI / 电气 -> 2
    CIVI / CIVIL / ARCH -> 5
    STRU / STRUCT -> 6
  spec_info parquet now writes SITE rows plus BRAN/HANG/EQUI/WALL/FLOOR rows.

src\fast_model\export_model\export_dbnum_instances_parquet.rs
  zero raw spec_value now resolves through self, owner, then TreeIndex ancestors.
  ancestor search depth is capped at 64.
  manifest definition now says self/owner/ancestor lookup.
```

Validation:

```text
rustfmt --edition 2024 --check
  src\fast_model\export_model\export_dbnum_instances_parquet.rs
  src\fast_model\export_model\spec_info.rs
status=passed

cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build
status=passed

aios-database -c db_options\DbOption-codex-live-view --export-parquet --dbnum 1112 --output target\codex-spec-info-site-ancestor-fix-20260622\parquet -v
status=passed
```

Evidence:

```text
target\codex-spec-info-site-ancestor-fix-20260622\spec-distribution.txt

legacy release 791:
  instances.parquet zero_spec=26117/26117
  tubings.parquet zero_spec=56/56

new scratch export:
  spec_info_1112.parquet rows=917 zero=282 nonzero=635
  instances.parquet rows=47490 zero=12232 nonzero=35258
  spec distribution:
    spec_value=2 rows=1589
    spec_value=5 rows=31943
    spec_value=6 rows=1726
  manifest spec_info_fallback_count=12232
```

Remaining fallback:

```text
target\codex-spec-info-site-ancestor-fix-20260622\unmapped-sites-1112.json

Unmapped SITE names are blank, metadata, missing-elements, model-problem, and
issue-summary buckets. These are not forced into a production discipline code.
```

Review:

- This improves future release package generation and keeps historical package
  immutability intact.
- This still does not satisfy final Done: new 791/897 candidates need to be
  regenerated, indexed, readiness-checked, and visually compared.

## 2026-06-22 — Oracle MCP and DuckLake Architecture Review

Goal:

- Continue the architecture analysis with Oracle MCP where possible, then lock
  down the best model-version data strategy and implementation plan.

Oracle MCP:

```text
npx -y @steipete/oracle --help
status=passed

MCP dry-run 1:
  context ~= 384976 tokens
  action=too large, reduced file set

MCP dry-run 2:
  context ~= 92934 tokens
  files=focused version_management/spec_info/web validation context
  status=ready

live consult:
  status=failed
  reason=ChatGPT/Cloudflare challenge detected
  api_fallback=not run
```

DuckLake check:

- DuckLake 1.0 is available through the DuckDB `ducklake` extension.
- It uses a SQL metadata catalog plus Parquet data files and exposes snapshots
  and table change functions.
- It does not replace project-level release gates because traditional database
  constraints/indexes are not the right authority boundary for visual release
  completeness.

Decision:

- Keep DuckLake/DuckDB/Parquet.
- Make `ReleasePackage` the immutable truth.
- Make DuckLake a rebuildable catalog/index projection.
- Keep release lifecycle/quality/readiness in the project state machine.

Documented:

```text
docs\plans\2026-06-21-e3d-model-version-ducklake-oracle-mcp-v3.md
  section=13.31 Oracle MCP Follow-up and DuckLake Version Architecture
```

The new section includes:

- end-to-end architecture
- file structure
- release/package data model
- component hash contract
- edge cases
- development sequence
- CLI/HTTP/UI validation approach

Review:

- This is an architecture/planning step, not a production promotion.
- The next executable step is to regenerate DB1112 791/897 candidate packages
  with the repaired spec_info path and then validate them through CLI, HTTP, and
  two-pane 3D comparison.

## 2026-06-22 — DB1112 791/897 Candidate Regeneration Audit

Goal:

- Test whether the repaired backend export/spec_info path improves real DB1112
  791/897 historical model packages.

Existing replay package:

```text
command:
  model-version validate-history-replay
  --project AvevaMarineSample
  --dbnum 1112
  --source-db-file \\?\D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001
  --from-sesno 791
  --to-sesno 897
  --parquet-dir output\AvevaMarineSample\model_versions\replay_work\codex-http-history-targetsrc-release-20260621062500\output\AvevaMarineSample\parquet\1112
  --current-parquet-dir output\AvevaMarineSample\parquet\1112
  --scene-tree-dir output\AvevaMarineSample\model_versions\replay_work\codex-http-history-targetsrc-release-20260621062500\output\AvevaMarineSample\scene_tree
  --allow-patch-only
  --json

json:
  target\codex-791-897-candidate-audit-20260622\validate-history-replay-existing.json

result:
  classification=missing_mesh_assets
  ready_for_publish=false
  instances=46469
  geo_instances=1867
  render_missing_geo_hashes=1
```

791 baseline:

```text
first export:
  status=passed
  log=target\codex-regenerate-791-897-20260622\export-791.log
  issue=spec_info failed because scene_tree\1112.tree was missing
  fallback=31100

tree repair:
  command=aios-database -c <791 physical baseline config> --gen-indextree 1112
  status=passed
  nodes=191967
  log=target\codex-regenerate-791-897-20260622\gen-indextree-791.log

export after tree:
  status=passed
  log=target\codex-regenerate-791-897-20260622\export-791-after-tree.log
  spec_info_rows=847
  instances=47698
  geo_instances=31292
  spec_info_fallback_count=11601
  render_missing_geo_hashes=16
```

897 baseline:

```text
export:
  status=passed
  log=target\codex-regenerate-791-897-20260622\export-897.log
  spec_info_rows=917
  instances=52020
  geo_instances=28704
  spec_info_fallback_count=15032
  render_missing_geo_hashes=16
```

Missing mesh dry-run with the correct baseline namespaces:

```text
791:
  json=target\codex-regenerate-791-897-20260622\repair-791-after-tree-baseline-dry-run.json
  requested_hashes=16
  missing_inst_geo=0
  non_renderable_inputs=6
  self_intersecting_inputs=9
  dry_run_eligible=1

897:
  json=target\codex-regenerate-791-897-20260622\repair-897-baseline-dry-run.json
  requested_hashes=16
  missing_inst_geo=0
  non_renderable_inputs=7
  self_intersecting_inputs=9
  dry_run_eligible=0
```

Summary artifact:

```text
target\codex-regenerate-791-897-20260622\summary.json
```

Review:

- 791 had a real backend generation evidence problem: the physical baseline
  workspace lacked `scene_tree/1112.tree`, which forced all spec_info to zero.
- Rebuilding that tree improved 791 from `31100` fallback rows to `11601`.
- 897 already had a tree and improved to `15032` fallback rows with the repaired
  exporter.
- Neither package is production-ready: both still have 16 render-required
  missing mesh hashes and non-zero spec_info fallback.
- The packages were not registered in DuckLake because doing so would only add
  known-bad candidates.
