# Model Version Management Refactor Implementation Strategy

Date: 2026-06-18
Scope: implement the Oracle-corrected model version management refactor in `plant-model-gen-cata-closure`

## Implementation Goal

Turn the current export/diff-oriented plan into a real model version management layer:

```text
Parquet export
  -> DuckLake release snapshot
  -> release graph
  -> component identity and lineage
  -> delivery unit membership versions
  -> unit_versions aggregate hashes
  -> deterministic impact and diff APIs
```

The first proof path remains a changed component such as `VALV`, but the implementation target is unit-level model version management, especially proving that a version-significant member change changes the containing `BRAN` aggregate hash.

## Refactor Boundary

Do not replace the generation writer in the first refactor.

Keep this path intact:

```text
gen/export existing data
  -> export_dbnum_instances_parquet
  -> instances/geo_instances/transforms/aabb/... parquet files
```

Add a new version-management layer after Parquet export:

```text
existing parquet package
  -> model version register/index/diff
```

This keeps risk low because `src/fast_model/gen_model/model_writer_ducklake.rs` and `src/pe_transform_store.rs` are not yet complete enough to be the sole source of truth.

## Feature Strategy

Add a new feature instead of reusing `model-writer-ducklake`:

```toml
model-version-ducklake = ["dep:duckdb", "parquet-export"]
```

Reason:

- `model-writer-ducklake` means generation-time writer.
- model version management is a publish/index/query layer.
- Separating the feature avoids pulling model writer semantics into release indexing.

CLI flags can exist unconditionally, but execution should return a clear error when `model-version-ducklake` is not compiled.

## Module Layout

Extend `src/version_management/` rather than mixing this into `fast_model/export_model`.

Recommended modules:

```text
src/version_management/
  mod.rs
  types.rs
  hashing.rs
  ducklake_store.rs
  model_release.rs
  release_graph.rs
  snapshot_import.rs
  component_identity.rs
  delivery_unit.rs
  component_version.rs
  component_lineage.rs
  unit_version.rs
  propagation_rules.rs
  unit_dependency.rs
  component_diff.rs
  unit_impact.rs
  cli.rs
```

Responsibilities:

- `types.rs`: shared DTOs, enums, request/response structs.
- `hashing.rs`: canonical serialization and SHA-256 helpers.
- `ducklake_store.rs`: connect, attach, create schema, transactions, SQL helpers.
- `model_release.rs`: register/list releases, file manifest, status transitions.
- `release_graph.rs`: parent/comparison edges.
- `snapshot_import.rs`: import Parquet package into `versioned_*` tables.
- `component_identity.rs`: compute and persist `component_identity_hash`.
- `delivery_unit.rs`: resolve unit membership and owner paths.
- `component_version.rs`: compute component-level hashes.
- `component_lineage.rs`: connect component versions across release edges.
- `unit_version.rs`: build BRAN/EQUI/WALL aggregate hashes.
- `propagation_rules.rs`: deterministic version-significance rules.
- `unit_dependency.rs`: persisted component-to-unit dependency edges.
- `component_diff.rs`: refno/identity component diff.
- `unit_impact.rs`: component change to impacted unit result.
- `cli.rs`: thin CLI orchestration, called from `src/cli_modes.rs`.

## Storage Schema Strategy

Use a dedicated DuckLake schema, for example:

```sql
CREATE SCHEMA IF NOT EXISTS model_version;
```

Do not reuse `ducklake-canonical`, because that schema belongs to the generation model writer raw-table experiment.

Tables should be split into three groups.

### 1. Release Registry

- `model_version.model_releases`
- `model_version.model_release_edges`
- `model_version.model_release_files`

Implementation notes:

- `release_id` should be generated from dbnum + timestamp or supplied by CLI.
- Store `manifest_path`, original Parquet file paths, file sizes, and optional content hashes.
- Store `parent_release_id`, `branch_id`, `derivation_type`, `semantic_version`, and `release_hash`.
- In MVP, only require one parent/comparison edge. Full branch/merge semantics can wait.

### 2. Imported Snapshot Tables

- `model_version.versioned_instances`
- `model_version.versioned_geo_instances`
- `model_version.versioned_tubings`
- `model_version.versioned_transforms`
- `model_version.versioned_aabb`
- `model_version.versioned_ptsets`
- `model_version.versioned_primitive_keypoints`

Implementation notes:

- Import from existing Parquet files with `release_id` and `dbnum` added.
- Prefer import/copy into DuckLake-managed tables for immutability.
- Keep `model_release_files` pointing to the original source package for traceability.
- Do not mutate existing export Parquet files.

### 3. Version Index Tables

- `model_version.component_identities`
- `model_version.component_versions`
- `model_version.component_lineage`
- `model_version.delivery_units`
- `model_version.delivery_unit_membership_versions`
- `model_version.unit_versions`
- `model_version.propagation_rules`
- `model_version.unit_dependency_edges`
- `model_version.component_unit_impacts` optional/cache

These are the actual model version system. DuckLake storage alone is not enough.

## Hashing Rules

All hashes must include a version prefix:

```text
component_identity:v1|...
component_version:v1|...
unit_version:v1|...
release_hash:v1|...
rule_set:v1|...
```

Use stable canonical serialization:

- Sort arrays before hashing.
- Use explicit `null`/`missing` markers.
- Format floating point values with a fixed precision or tolerance policy.
- Include `hash_version` and `rule_set_hash` in stored rows.

Initial hash choices:

- `component_identity_hash`: `sha256("component_identity:v1|dbnum|refno_u64")`
- `geometry_hash`: sorted `(geo_index, geo_hash, geo_trans_hash)`
- `transform_hash`: `trans_hash` plus resolved matrix when available
- `aabb_hash`: `aabb_hash` plus resolved numeric values when available
- `membership_hash`: `unit_key`, owner path, member role
- `component_hash`: semantic + geometry + transform + aabb + membership
- `unit_versions.aggregate_hash`: sorted `(component_identity_hash, component_hash, member_role)` plus unit metadata and `rule_set_hash`

The initial identity strategy is refno-based, but storing `identity_strategy`, `confidence`, and diagnostics makes future non-refno matching possible.

## CLI Surface

Add args in `src/cli_args.rs` and route from `src/main.rs` to `src/cli_modes.rs`.

Recommended MVP flags:

```text
--model-version-register
--model-version-list
--model-version-index-components
--model-version-index-units
--model-version-diff-component
--model-version-impact-component
--model-version-diff-unit

--model-version-ducklake-metadata <PATH>
--model-version-ducklake-data-path <PATH>
--release-id <ID>
--parent-release-id <ID>
--old-release-id <ID>
--new-release-id <ID>
--release-label <LABEL>
--branch-id <ID>
--derivation-type <TYPE>
--parquet-dir <DIR>
--refno <REFNO>
--unit-key <KEY>
--unit-refno <REFNO>
--json
```

Keep CLI orchestration thin:

```text
main.rs -> cli_modes.rs -> version_management::cli
```

Do not place SQL or hashing logic in `main.rs` or `cli_modes.rs`.

## Implementation Slices

### Slice 1: Store and Release Registration

Files:

- `src/version_management/types.rs`
- `src/version_management/ducklake_store.rs`
- `src/version_management/model_release.rs`
- `src/version_management/release_graph.rs`
- `src/version_management/snapshot_import.rs`
- `src/version_management/cli.rs`
- `src/cli_args.rs`
- `src/cli_modes.rs`
- `src/main.rs`
- `Cargo.toml`

Deliverable:

- Register one Parquet package as a release.
- Create schema/tables.
- Import `instances`, `geo_instances`, `transforms`, `aabb` first.
- List releases as JSON.
- Connect two releases with one comparison edge.

Acceptance:

- CLI returns release id and table row counts.
- Re-registering the same release is idempotent or returns a clear duplicate error.
- DuckLake contains rows keyed by `release_id`.

### Slice 2: Component Identity and Membership

Files:

- `component_identity.rs`
- `delivery_unit.rs`
- `hashing.rs`

Deliverable:

- Compute `component_identity_hash`.
- Resolve delivery unit membership.
- Persist `delivery_units` and `delivery_unit_membership_versions`.

Important:

- Start with direct owner and TreeIndex/owner-chain lookup where available.
- If unit cannot be resolved, store `UNASSIGNED` with reason; never drop.
- Preserve `owner_path_json` for audit.

Acceptance:

- For a known component, CLI can return its identity and old/new unit membership.
- Unresolved counts are reported.

### Slice 3: Component Versions and Lineage

Files:

- `component_version.rs`
- `component_lineage.rs`

Deliverable:

- Compute component hashes from imported versioned tables.
- Persist `component_versions`.
- Link components across release edges in `component_lineage`.

Acceptance:

- Same release re-index produces identical component hashes.
- Changing geometry/transform/membership changes the expected hash group.
- A component can be followed by `component_identity_hash` across two releases.

### Slice 4: Unit Versions

Files:

- `unit_version.rs`
- `propagation_rules.rs`

Deliverable:

- Seed default propagation rules.
- Compute `unit_versions.aggregate_hash`.
- Store member counters and rule set hash.

Acceptance:

- Same release re-index produces identical unit aggregate hashes.
- A version-significant member change changes the containing BRAN aggregate hash.

### Slice 5: Diff and Impact CLI

Files:

- `component_diff.rs`
- `unit_dependency.rs`
- `unit_impact.rs`

Deliverable:

- Component diff by refno or identity.
- Impact component to unit using deterministic rules.
- Unit diff by unit key/refno.

Acceptance:

- For a changed component, output includes:
  - `component_identity_hash`
  - old/new component hashes
  - old/new unit versions
  - impacted units
  - rule id
  - dependency path evidence
- `VALV` sample can prove BRAN aggregate hash changed.

### Slice 6: Web API

Files:

- `src/web_api/model_version_api.rs`
- `src/web_api/mod.rs`

Deliverable:

- Read-only API over the CLI-equivalent core functions.

Routes:

- `GET /api/model-version/releases`
- `GET /api/model-version/unit-version/:unit_key`
- `POST /api/model-version/component-diff`
- `POST /api/model-version/component-impact`
- `POST /api/model-version/unit-diff`

Validation:

- Per project rule, verify by running web server and HTTP/POST.
- Do not add or run Rust tests.

## Import SQL Shape

Initial import can be SQL-driven through DuckDB/DuckLake:

```sql
INSERT INTO model_version.versioned_instances
SELECT
  $release_id AS release_id,
  *
FROM read_parquet($instances_path);
```

For safety:

- Validate expected columns before import.
- Fail with clear diagnostics when a table is missing.
- Allow optional tables such as `ptsets` and `primitive_keypoints` to be absent only if manifest confirms zero/optional.
- Wrap each release registration in a transaction.
- Mark release `failed` if registration or indexing fails after metadata creation.

## What Not To Refactor Yet

Do not change these in the first pass:

- `src/fast_model/gen_model/model_writer_ducklake.rs`
- `src/pe_transform_store.rs` DuckLake stub
- existing `export_dbnum_instances_parquet` output schema
- viewer/UI
- full branch/merge semantics
- generation-time ModelWriter path

The refactor should be additive until CLI evidence proves the version layer works.

## Validation Plan

Do not use `cargo test`.

Use CLI + JSON:

```powershell
aios-database --model-version-register --dbnum <DBNUM> --parquet-dir <OLD_DIR> --release-id old --json
aios-database --model-version-register --dbnum <DBNUM> --parquet-dir <NEW_DIR> --release-id new --parent-release-id old --json
aios-database --model-version-index-components --release-id old --json
aios-database --model-version-index-components --release-id new --json
aios-database --model-version-index-units --release-id old --json
aios-database --model-version-index-units --release-id new --json
aios-database --model-version-diff-component --old-release-id old --new-release-id new --refno <VALV_REFNO> --json
aios-database --model-version-impact-component --old-release-id old --new-release-id new --refno <VALV_REFNO> --json
```

Minimum evidence:

- Two releases registered.
- Release edge exists.
- Component identity exists.
- Component lineage exists.
- BRAN unit versions exist.
- Impact result includes rule id and dependency edge.
- BRAN aggregate hash differs for the changed version-significant sample.

For `web_server`, verify later by HTTP/POST only.

## Key Engineering Risks

| Risk | Mitigation |
| --- | --- |
| DuckDB dependency is gated by the wrong feature | Add `model-version-ducklake`; do not reuse `model-writer-ducklake`. |
| Direct owner cannot resolve BRAN | Use owner path/tree lookup and store unresolved reasons. |
| Refno identity is not stable enough | Store identity strategy/confidence and keep identity algorithm versioned. |
| Hash formula changes later | Store `hash_version` and `rule_set_hash`; allow re-indexing. |
| Runtime impact is not auditable | Persist propagation rules and dependency path evidence. |
| Old mesh assets disappear | Add release file/asset manifest before UI rollout. |
| CLI grows too much in `main.rs` | Keep orchestration in `version_management::cli`. |

## Recommended First Commit Stack

1. `feat(model-version): add feature flag and module skeleton`
2. `feat(model-version): create ducklake store and schema bootstrap`
3. `feat(model-version): register parquet release and release edge`
4. `feat(model-version): import versioned snapshot tables`
5. `feat(model-version): compute component identities and memberships`
6. `feat(model-version): compute component versions and lineage`
7. `feat(model-version): compute unit versions`
8. `feat(model-version): add component diff and impact CLI`

Keep each commit CLI-observable and avoid touching generation logic.
