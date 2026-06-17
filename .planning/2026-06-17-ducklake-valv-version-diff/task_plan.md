# DuckLake Model Version Diff Development Plan

Date: 2026-06-17
Planner: Plannator-style working plan
Scope: `plant-model-gen-cata-closure` model export/versioning, with component diff plus delivery-unit impact propagation as the first user-visible path

## Goal

Use DuckLake as the versioned model release store so users can compare two generated model versions at component granularity and understand how a local component change propagates to delivery units such as `BRAN`.

The first concrete scenario is:

- Select two releases, for example "before" and "after".
- Select one changed component by refno. A valve `VALV` is only the first test case, with `VALE` treated as a user-facing alias if it appears in business wording.
- Show whether the component was added, deleted, moved, geometry-changed, owner-changed, spec-changed, or unchanged.
- Trace the component to its containing delivery unit before and after the change.
- Report whether this local change causes a `BRAN`/`EQUI`/`WALL` unit-level change, for example a `VALV` modification making a `BRAN` version dirty.
- Provide structured JSON diff first, then a 3D before/after overlay.

## Product Decision

Use DuckLake first as a versioned export/publish layer, not as the primary generation writer.

Rationale:

- The current `export_dbnum_instances_parquet` path already produces normalized tables close to what diff needs.
- The current DuckLake model writer exists but still has known generation-side gaps around some tubi/transform/refno-association coverage.
- A release-layer approach gives a smaller MVP: generate/export as today, then register the export as a release in DuckLake and build diff indexes from it.

## Terms

| Term | Meaning |
| --- | --- |
| Release | One published model snapshot, usually from one dbnum/project export and one generator configuration. |
| Delivery unit | Minimum delivery/control unit: `BRAN`, `HANG`, `EQUI`/`EQUIP`, `WALL`, `FLOOR`. |
| Component | A model element with stable refno, for example a valve `VALV` component. |
| Unit version | Version state of all members under one delivery unit in one release. |
| Component version | Fingerprint and detail rows for one refno in one release. |
| Impact propagation | Mapping from a changed component to affected parent units and aggregate unit version state. |
| Diff target | Usually `(dbnum, refno_str, old_release_id, new_release_id)`. |

## Baseline Facts

- `src/fast_model/export_model/export_dbnum_instances_parquet.rs` writes normalized Parquet tables such as `instances`, `geo_instances`, `tubings`, `transforms`, `aabb`, `ptsets`, `primitive_keypoints`, plus a manifest.
- `src/fast_model/export_model/export_dbnum_instances_web.rs` and `export_dbnum_instances_v3.rs` already contain grouping/export logic useful for viewer integration.
- `src/fast_model/export_model/spec_info.rs` defines delivery-unit nouns as `BRAN`, `HANG`, `EQUI`, `WALL`, `FLOOR`.
- `src/fast_model/gen_model/model_writer_ducklake.rs` has a DuckLake backend, but it is a raw writer path and should not be the MVP dependency for version comparison.
- `src/pe_transform_store.rs` has a DuckLake registration stub for transform storage, so transform DuckLake should not be assumed complete.
- `src/version_management/` currently covers status/log style data, not model release diff.

## Target Architecture

```mermaid
flowchart LR
  A["Current model generation"] --> B["Existing Parquet export"]
  B --> C["DuckLake release registration"]
  C --> D["Release metadata tables"]
  C --> E["Versioned model tables"]
  E --> F["Delivery unit membership index"]
  E --> G["Component version fingerprints"]
  F --> H["Unit diff and impact rollup"]
  G --> I["Component diff"]
  I --> H
  I --> J["CLI JSON"]
  I --> K["Web API"]
  K --> L["3D compare viewer"]
```

## DuckLake Usage

DuckLake should store both metadata and versioned Parquet-backed data:

- Use one DuckLake catalog per project/output root, or one shared catalog with `project_name` partitioning.
- Store the imported release tables as append-only versioned records, keyed by `release_id`.
- Keep app-level release IDs even though DuckLake has snapshots/time travel, because domain releases must map to generator config, dbnum, sesno, source manifest, and UI labels.
- Use DuckLake time travel later for forensic table-state queries, but use explicit `release_id` for application diff.
- Consider partitioning only after row counts are known; start with `dbnum` and possibly `release_id`, avoid over-partitioning by refno.

Reference docs:

- DuckLake overview: https://ducklake.select/docs/stable/
- Time travel: https://ducklake.select/docs/stable/duckdb/usage/time_travel
- Partitioning: https://ducklake.select/docs/stable/duckdb/advanced_features/partitioning
- Change feed: https://ducklake.select/docs/stable/duckdb/advanced_features/data_change_feed

## Proposed Tables

### Control Tables

`model_releases`

| Column | Purpose |
| --- | --- |
| `release_id` | Stable generated id, for example `db1112_20260617_001`. |
| `project_name` | Project/site key. |
| `dbnum` | Source dbnum. |
| `source_sesno_key` | Optional source session/version marker from existing db metadata. |
| `manifest_path` | Path to Parquet export manifest. |
| `generator_config_hash` | Hash of relevant generation/export settings. |
| `ducklake_snapshot_id` | Optional DuckLake snapshot id after commit. |
| `release_label` | UI label such as "before valve change". |
| `created_at` | Registration time. |
| `status` | `registered`, `indexed`, `failed`, `archived`. |

`model_release_files`

| Column | Purpose |
| --- | --- |
| `release_id` | Owner release. |
| `table_name` | `instances`, `geo_instances`, `transforms`, etc. |
| `file_path` | Physical Parquet path. |
| `row_count` | Imported row count. |
| `content_hash` | Optional file hash for reproducibility. |

`delivery_units`

| Column | Purpose |
| --- | --- |
| `release_id` | Release key. |
| `unit_key` | Stable key, for example `1112:BRAN:123456`. |
| `dbnum` | Source dbnum. |
| `unit_type` | `BRAN`, `HANG`, `EQUI`, `EQUIP_ALIAS`, `WALL`, `FLOOR`, `UNASSIGNED`. |
| `unit_refno_str` | Unit refno. |
| `unit_refno_u64` | Numeric unit refno. |
| `parent_unit_key` | Optional parent delivery unit. |
| `name` | Optional name from export fields or attrs. |

`delivery_unit_members`

| Column | Purpose |
| --- | --- |
| `release_id` | Release key. |
| `unit_key` | Delivery unit key. |
| `refno_str` | Member refno. |
| `refno_u64` | Numeric member refno. |
| `noun` | Member noun, for example `VALV`. |
| `owner_refno_str` | Direct owner from `instances`. |
| `owner_noun` | Direct owner noun. |
| `member_role` | `self`, `direct_child`, `descendant`, `fallback_owner`, `unassigned`. |

### Versioned Model Tables

Start with release-keyed copies/projections of existing Parquet tables:

- `versioned_instances`
- `versioned_geo_instances`
- `versioned_tubings`
- `versioned_transforms`
- `versioned_aabb`
- `versioned_ptsets`
- `versioned_primitive_keypoints`

Each table must include `release_id` and `dbnum`. Keep original columns unchanged where possible so existing export semantics remain traceable.

### Diff Index Tables

`component_versions`

| Column | Purpose |
| --- | --- |
| `release_id` | Release key. |
| `dbnum` | Source dbnum. |
| `refno_str` | Component refno. |
| `refno_u64` | Numeric component refno. |
| `noun` | Component noun. |
| `unit_key` | Delivery unit membership. |
| `semantic_hash` | Hash of owner/spec/name/noun/cata/flags. |
| `geometry_hash` | Hash of sorted geometry rows and geometry transforms. |
| `transform_hash` | Hash of resolved matrix/transform identity. |
| `aabb_hash` | Hash of resolved AABB values. |
| `membership_hash` | Hash of delivery-unit membership. |
| `component_hash` | Combined hash. |
| `detail_json` | Compact detail payload for diagnostics. |

`component_diff_cache` is optional in MVP. Add it only after CLI/API diff is stable and repeated queries become expensive.

`component_unit_impacts`

| Column | Purpose |
| --- | --- |
| `old_release_id` | Before release. |
| `new_release_id` | After release. |
| `dbnum` | Source dbnum. |
| `refno_str` | Changed component refno. |
| `old_unit_key` | Containing unit before the change. |
| `new_unit_key` | Containing unit after the change. |
| `impact_unit_key` | Unit that should be marked impacted. Usually old unit, new unit, or both. |
| `impact_unit_type` | `BRAN`, `HANG`, `EQUI`, `WALL`, `FLOOR`, `UNASSIGNED`. |
| `impact_kind` | `member_changed`, `member_added`, `member_deleted`, `member_moved_in`, `member_moved_out`, `unit_hash_changed`. |
| `component_change_type` | Component-level diff category. |
| `impact_path_json` | Owner/unit path used to justify the impact. |

This table can be computed on demand for MVP. Persist it only if repeated UI queries need caching.

## Diff Semantics

For one component refno:

1. Query old and new `component_versions`.
2. If old missing and new exists, return `added`.
3. If old exists and new missing, return `deleted`.
4. If `component_hash` matches, return `unchanged`.
5. Otherwise compare detail groups:
   - Semantic: `noun`, `owner_refno`, `owner_noun`, `spec_value`, `cata_hash`, `has_neg`, display name if available.
   - Membership: delivery unit changed, owner moved under another BRAN/EQUI/WALL.
   - Transform: `trans_hash`, resolved matrix, translation delta, rotation/scale signal where available.
   - Geometry: `geo_hash`, `geo_index`, `geo_trans_hash`, negative geometry relations where available.
   - AABB: numeric min/max/center/extent delta.
   - Ptset/tubing: optional detail, only if tables contain rows.
6. Return `multi_change` when more than one category changed, with category flags for UI.

For delivery-unit diff:

1. Query `delivery_unit_members` for old/new release and `unit_key`.
2. Compute added/deleted/common members by `refno_str`.
3. For common members, compare `component_hash`.
4. Aggregate counts by noun and change category.
5. Provide drill-down to component diff.

For impact propagation from component to delivery unit:

1. Run component diff for the target refno.
2. Resolve old and new membership from `delivery_unit_members`.
3. If old and new unit are the same, mark that unit as impacted when any component-level hash changed.
4. If old and new unit differ, mark old unit as `member_moved_out` and new unit as `member_moved_in`.
5. If old is missing and new exists, mark new unit as `member_added`.
6. If old exists and new is missing, mark old unit as `member_deleted`.
7. Recompute or query unit aggregate hash/counts to tell whether the containing `BRAN` version changed.
8. Return the impact path, for example `VALV -> branch component owner -> BRAN`, so the UI can explain why a BRAN is dirty.

## Development Phases

### Phase 0: Planning and Feasibility

Status: done.

Tasks:

- Run SigMap context discovery for DuckLake, export Parquet, versioning, and component diff.
- Confirm no callable `plannator` tool is exposed in the current tool list.
- Create this active planning package.

Acceptance:

- Plan files exist under `.planning/2026-06-17-ducklake-valv-version-diff/`.
- `.planning/.active_plan` points to this plan.

### Phase 1: Release Registration MVP

Goal: Register one existing Parquet export as a DuckLake model release.

Tasks:

- Add a small module, recommended path `src/version_management/model_release.rs`.
- Add a DuckDB/DuckLake access wrapper, recommended path `src/version_management/ducklake_store.rs`.
- Define release metadata structs with `serde` for CLI JSON output.
- Add CLI modes in `src/main.rs` and `src/cli_modes.rs`:
  - `--model-version-register`
  - `--model-version-list`
  - `--model-version-ducklake-metadata <path>`
  - `--model-version-ducklake-data-path <path>`
  - `--release-label <label>`
  - `--parquet-dir <dir>`
  - `--dbnum <dbnum>`
  - `--json`
- Read `manifest_{dbnum}.json` or `manifest.json` from the Parquet export directory.
- Create DuckLake schema and control tables if missing.
- Load/import or register the Parquet tables with `release_id` attached.
- Store row counts and file paths in `model_release_files`.

Acceptance:

- Registering one export returns JSON containing `release_id`, table counts, manifest path, and status `registered`.
- Listing releases returns the registered release.
- Running the command twice with the same manifest either produces a clear duplicate error or an explicit idempotent result.

Validation:

- Use CLI + JSON only. Do not create or run cargo tests.
- Inspect DuckLake with DuckDB SQL to confirm control table rows and imported row counts.

### Phase 2: Delivery Unit Resolver

Goal: Split model content by minimum delivery unit such as `BRAN`, `EQUI`/`EQUIP`, `WALL`.

Tasks:

- Add resolver module, recommended path `src/version_management/delivery_unit.rs`.
- Normalize noun aliases:
  - `EQUIP` user/business wording maps to code/data noun `EQUI` unless actual data proves otherwise.
  - `VALE` user wording maps to `VALV` search/display alias if needed.
- Build initial unit membership from `versioned_instances.owner_refno_str` and `owner_noun`.
- Treat `BRAN`, `HANG`, `EQUI`, `WALL`, `FLOOR` rows as units when present.
- For components whose direct owner is not a delivery unit, walk owner chains if available from export/tree data.
- Mark unresolved members with `unit_type = UNASSIGNED` and `member_role = unassigned`; do not silently drop them.
- Add counters by noun and unit type.

Acceptance:

- Every component row is either assigned to one unit or reported with an unresolved reason.
- Unit membership can answer: "which BRAN/EQUI/WALL contains this VALV refno in this release?"
- Output includes counts for assigned/unassigned rows.

Validation:

- Run resolver on a known dbnum export.
- Query one BRAN, one EQUI/EQUIP, one WALL, and one VALV refno by CLI JSON.

### Phase 3: Component Fingerprints

Goal: Compute stable component version fingerprints per release/refno.

Tasks:

- Add module, recommended path `src/version_management/component_version.rs`.
- For each `instances` row, collect linked `geo_instances`, `transforms`, `aabb`, and optional `tubings`/`ptsets`.
- Canonicalize arrays/maps before hashing:
  - Sort geometry rows by `geo_index`, `geo_hash`, `geo_trans_hash`.
  - Format floating point values with a defined tolerance or fixed precision.
  - Keep missing values explicit as `null`/`missing`, not empty strings.
- Compute `semantic_hash`, `geometry_hash`, `transform_hash`, `aabb_hash`, `membership_hash`, and combined `component_hash`.
- Write rows to `component_versions`.
- Add a CLI mode:
  - `--model-version-index-components --release-id <id> --json`

Acceptance:

- Re-indexing the same release yields stable hashes.
- A changed transform changes `transform_hash` and combined `component_hash`.
- A changed `geo_hash` changes `geometry_hash`.
- A pure owner/unit move changes `membership_hash` or `semantic_hash`, depending on owner semantics.

Validation:

- Use two small exports or one export plus a controlled copied Parquet change.
- Query component version JSON for a known `VALV` refno.

### Phase 4: Component Diff and Impact Seed CLI

Goal: Provide a precise before/after diff for one refno and include the delivery-unit memberships that can seed impact analysis.

Tasks:

- Add module, recommended path `src/version_management/component_diff.rs`.
- Add CLI mode:
  - `--model-version-diff-component`
  - `--old-release-id <id>`
  - `--new-release-id <id>`
  - `--refno <refno>`
  - `--dbnum <dbnum>`
  - `--json`
- Implement diff categories:
  - `added`
  - `deleted`
  - `unchanged`
  - `moved`
  - `geometry_changed`
  - `owner_changed`
  - `spec_changed`
  - `aabb_changed`
  - `multi_change`
- Return payload sections:
  - `summary`
  - `identity`
  - `old`
  - `new`
  - `changed_fields`
  - `geometry_changes`
  - `transform_delta`
  - `aabb_delta`
  - `unit_change`
  - `old_unit`
  - `new_unit`
  - `impact_seed`
  - `diagnostics`

Acceptance:

- For any component refno, CLI JSON clearly explains what changed. `VALV` is only the first validation sample.
- Missing old/new rows are reported as added/deleted rather than errors.
- Numeric transform/AABB deltas include tolerances.
- Response includes old and new containing units when resolvable.

Validation:

- Use CLI + JSON.
- Save at least one real command/output sample in `progress.md`.

### Phase 5: Impact Propagation and Delivery Unit Diff CLI

Goal: Compare a BRAN/EQUI/WALL delivery unit across releases, and answer "which BRAN became impacted because this component changed?"

Tasks:

- Add impact CLI mode:
  - `--model-version-impact-component`
  - `--old-release-id <id>`
  - `--new-release-id <id>`
  - `--refno <refno>`
  - `--dbnum <dbnum>`
  - `--json`
- Add CLI mode:
  - `--model-version-diff-unit`
  - `--old-release-id <id>`
  - `--new-release-id <id>`
  - `--unit-refno <refno>`
  - `--unit-type <BRAN|EQUI|EQUIP|WALL|FLOOR>`
  - `--json`
- Implement member set diff.
- Aggregate by noun and change category.
- Implement component-to-unit impact propagation:
  - same unit: mark containing unit impacted
  - moved component: mark old and new units impacted
  - added component: mark new unit impacted
  - deleted component: mark old unit impacted
- Compute unit aggregate hashes/counts so a single member change can update the BRAN-level version state.
- Include sample changed refnos, capped by CLI option.

Acceptance:

- A BRAN/EQUI/WALL diff shows added/deleted/changed/unchanged component counts.
- A changed component can be discovered from its containing unit diff.
- A `VALV` sample modification can report the impacted `BRAN` when membership resolves to a branch.
- If a component moves between BRANs, both BRANs are reported with distinct impact kinds.

### Phase 6: Web API

Goal: Expose release and diff capabilities to the UI.

Tasks:

- Add API module, recommended path `src/web_api/model_version_api.rs`.
- Add routes:
  - `GET /api/model-version/releases?dbnum=...`
  - `GET /api/model-version/component-history/:refno?dbnum=...`
  - `POST /api/model-version/component-diff`
  - `POST /api/model-version/component-impact`
  - `POST /api/model-version/unit-diff`
- Keep request/response DTOs aligned with CLI JSON.
- Add precise errors for missing release, missing refno, unindexed release, and DuckLake unavailable.

Acceptance:

- HTTP `POST /api/model-version/component-diff` returns the same summary as CLI for the known valve.
- HTTP `POST /api/model-version/component-impact` returns impacted delivery units for a changed component.
- `GET /api/model-version/releases` lists release labels and statuses.

Validation:

- Per repo rule, start `web_server` and verify with HTTP/POST.
- Do not use Rust test binaries or cargo test.

### Phase 7: 3D Compare Viewer

Goal: Make the diff inspectable, not only textual.

Tasks:

- Decide whether this UI lives in current web server templates or sibling viewer application.
- Add release selectors, refno search, and diff summary panel.
- Reuse existing exported model package paths where possible.
- Render old/new component states:
  - old only: red transparent
  - new only: green or yellow solid
  - both changed: overlay old transparent and new solid
  - unchanged context: muted gray
- For moved components, draw transform delta indicator or show before/after centers.
- Show changed fields and geometry hashes in a side panel.

Acceptance:

- User can select two releases and a `VALV`/`VALE` refno and visually compare before/after.
- UI clearly distinguishes moved versus geometry changed.
- Text does not overlap and large IDs are truncated/copyable.

Validation:

- Manual UI check plus HTTP traces.
- Capture screenshots for a known changed valve.

### Phase 8: Pipeline Integration and Hardening

Goal: Make the feature reliable in regular model export workflows.

Tasks:

- Add optional post-export registration hook after `export_dbnum_instances_parquet`.
- Record generator/export settings into `generator_config_hash`.
- Add retention policy for Parquet/mesh assets referenced by releases.
- Add a DuckLake availability diagnostic:
  - extension installed
  - metadata path writable
  - data path writable
  - can attach catalog
- Add performance counters:
  - release registration time
  - index build time
  - component diff query time
  - unit diff query time
- Add recovery path for partial/failed release registration.

Acceptance:

- A failed registration does not corrupt previous releases.
- Release metadata can tell exactly which manifest/files were used.
- Operators can diagnose offline DuckLake extension problems.

## MVP Slice

The smallest useful delivery should include:

1. Register two existing Parquet exports into DuckLake.
2. Index `component_versions` for both releases.
3. Run CLI component diff for one changed component refno, using `VALV`/`VALE` only as the first sample.
4. Resolve old/new delivery-unit membership for that component.
5. Return JSON with change category, old/new owner, old/new unit, impacted unit list, transform delta, geometry hash delta, and AABB delta.
6. Prove that a single `VALV` change can mark its containing `BRAN` as impacted when the membership path resolves to BRAN.

This MVP does not need:

- Generation-time DuckLake writer replacement.
- Full branching/merge semantics.
- 3D viewer.
- Cached unit diff table.
- Full history UI.

## Validation Matrix

| Area | Method | Rule |
| --- | --- | --- |
| aios-database CLI | CLI command with `--json`, inspect output | No cargo tests. |
| DuckLake data | DuckDB SQL against metadata catalog | Verify row counts and release ids. |
| web_server | Run service, call HTTP/POST | No Rust tests. |
| UI | Manual browser flow and screenshot | Verify visual before/after overlay. |
| Regression | Compare row counts and hashes between repeated runs | Same export should produce same hashes. |

## Risks

| Risk | Impact | Mitigation |
| --- | --- | --- |
| `VALE` vs `VALV` naming mismatch | User cannot find target valve | Add alias layer and show resolved noun/refno in responses. |
| DuckLake extension unavailable offline | Registration fails on first run | Add diagnostic and document extension/cache setup. |
| DuckLake raw writer gaps are mistaken as blocker | Scope expands too early | Use Parquet export registration for MVP. |
| Delivery unit ownership is incomplete for WALL/FLOOR | Some components become unassigned | Store unresolved reason and improve with tree index owner walk. |
| Component change does not roll up to BRAN | BRAN version looks unchanged even when a member changed | Store old/new unit membership and compute aggregate unit hashes from member component hashes. |
| Component moves between units | Only one side of the change is visible | Emit both `member_moved_out` and `member_moved_in` impacts. |
| Hash-only diff hides numeric details | User cannot understand movement magnitude | Always compute matrix/AABB deltas for changed components. |
| Mesh/GLB assets are not retained per release | 3D compare cannot show old geometry | Add release asset manifest and retention policy before viewer rollout. |
| Large dbnum exports create too many small files | Slow import/query | Start unpartitioned or low-cardinality partitioned, then benchmark. |

## Open Questions

- Which field should define the business release: sesno, export timestamp, manually supplied label, or pipeline task id?
- Does live data use `VALV`, `VALE`, or both for valve components?
- For a component under a BRAN, is the BRAN impact rule always "any descendant component hash changed", or are some attributes geometry-only and not BRAN-version-significant?
- For `WALL`, is membership best derived from direct owner, scene tree ancestry, spatial relation, or a separate discipline rule?
- Should the first viewer integration happen in this repo's web server, or in the sibling 3D viewer application?
- Do old releases need immutable mesh/GLB asset snapshots, or can geometry hashes always resolve to retained shared mesh cache?

## File Touch Plan

Likely backend files:

- `src/version_management/mod.rs`
- `src/version_management/model_release.rs`
- `src/version_management/ducklake_store.rs`
- `src/version_management/delivery_unit.rs`
- `src/version_management/component_version.rs`
- `src/version_management/component_diff.rs`
- `src/version_management/unit_impact.rs`
- `src/cli_modes.rs`
- `src/main.rs`
- `src/web_api/model_version_api.rs`
- `src/web_api/mod.rs`

Likely documentation/ops files:

- `.planning/2026-06-17-ducklake-valv-version-diff/progress.md`
- `docs/plans/` if a published handoff plan is needed later

## Definition of Done

- Two releases are registered in DuckLake from existing Parquet exports.
- A known changed component refno can be diffed by CLI JSON, with `VALV`/`VALE` used as one validation sample.
- The diff identifies added/deleted/unchanged/moved/geometry/owner/spec/AABB changes.
- The diff reports old/new containing delivery units and impacted units.
- At least one `VALV` sample change can be shown to impact a containing `BRAN` when membership resolves to BRAN.
- At least one BRAN/EQUI/WALL unit diff can list changed member refnos and aggregate member changes into unit-level version state.
- Web API returns the same diff as CLI.
- Verification evidence is recorded without cargo tests.
