Recommendation

Use SurrealDB + existing Parquet export as the production generation path, and add a model-version publish/index layer after Parquet export. Use DuckLake only inside that version-management layer: immutable release snapshot storage, release registry/query catalog, and SQL diff/index tables. Do not use DuckLake as the MVP generation writer, and do not rely on pe_transform DuckLake as a source of truth yet.

That is the lowest-risk architecture for this repo because the current verified path is:

pdms-io sesno increment
  -> SurrealDB PE/ATT/UDA persistence
  -> scoped gen_all_geos_data(...)
  -> post-generation export_dbnum_instances_parquet(...)
  -> viewer reads output/<project>/parquet/<dbnum>

The recent verification already proves this path for AvevaMarineSample, dbnum 1112, sesno 896 -> 897, with PE/ATT persistence, scoped generation, and Parquet output under output\AvevaMarineSample\parquet\1112. The observed manifest counts were instances=106, geo_instances=163, transforms=131, aabb=105, ptsets=237, and missing_geo_hashes=0. 

attachments-bundle

The core design should be:

existing generation writer: SurrealDB
existing viewer payload: Parquet package
new version layer: release registry + immutable package copy + DuckLake index + deterministic domain diff

DuckLake is a good fit for the publish/index/query layer because it stores metadata in a SQL catalog while data remains in Parquet/open storage, and DuckLake 1.0 currently supports time travel, change-feed queries, and partitioning. 
DuckDB
 
DuckLake
 
DuckLake
 
DuckLake
 But DuckLake snapshots are not enough by themselves: user-facing model versions need explicit domain release IDs, release metadata, component identity, component lineage, unit versions, and impact rules. The attached planning notes already capture that distinction: DuckLake is appropriate as immutable snapshot/query storage, but the actual model version system must be built as domain metadata and deterministic indexing on top. 

attachments-bundle

1. Best option and tradeoffs
Best option: additive model-version layer after Parquet export

Add a new version_management implementation that registers each generated Parquet package as a model release, imports or indexes its Parquet tables into DuckLake, computes component/unit hashes, and exposes diff/impact APIs.

Recommended flow:

SurrealDB generation
  -> post_gen_export
  -> immutable release package copy
  -> DuckLake model_version schema
  -> component identity + lineage
  -> delivery-unit membership
  -> unit_versions aggregate hashes
  -> diff/impact APIs
  -> two-view 3D compare

This matches the implementation strategy already proposed in the repo notes: keep gen/export -> export_dbnum_instances_parquet -> instances/geo_instances/transforms/aabb/... parquet intact, then add existing parquet package -> model version register/index/diff. 

attachments-bundle

Tradeoffs
Option	Pros	Cons	Verdict
A. Directory-only versioned Parquet packages + JSON manifests	Very fast MVP; no DuckDB/DuckLake dependency; viewer integration is simple	Diff queries, lineage, impact, and release graph become hand-built and harder to audit; weak for historical comparison at scale	Useful as a fallback, not the main architecture
B. Replace SurrealDB generation writer with DuckLake writer	Theoretically cleaner long term	Too risky now: existing DuckLake ModelWriter still has known gap tables, does not run downstream pipeline, and pe_transform DuckLake is incomplete	Do not do this for MVP
C. Keep SurrealDB writer, version immutable Parquet releases, index with DuckLake	Preserves verified generation path; aligns with current viewer; gives SQL diff/query power; supports release graph, lineage, unit impact	Requires new release/index schema and a publish step; DuckLake extension/offline packaging needs operational care	Recommended
D. Store all historical releases as isolated SurrealDB namespaces	Faithful to generation state; can regenerate from arbitrary sesno	Heavy, operationally complex, not viewer-native; replaying old sesno into current DB is dangerous	Use only for historical reconstruction jobs, not as primary viewer/diff store

The key reason not to choose option B is repo-specific: model-writer-ducklake is explicitly separated behind a feature, the writer has known gap tables, and the contract evidence says DuckLake writer does not write to SurrealDB or run the downstream pipeline. 

attachments-bundle

 

attachments-bundle

 

attachments-bundle

 The DuckLake writer file also documents raw-table gaps such as raw_tubi_info, raw_tubi_relate, raw_trans, and tubi AABB/vec3 coverage. 

attachments-bundle

 

attachments-bundle

 And pe_transform_store.rs currently has a DuckLake registration stub that returns Ok(()), so transform DuckLake is not complete enough to anchor comparison. 

attachments-bundle

2. Should DuckLake be used, and exactly where?

Yes, but only here:

src/version_management/
  ducklake_store.rs
  model_release.rs
  snapshot_import.rs
  component_identity.rs
  component_version.rs
  component_lineage.rs
  delivery_unit.rs
  unit_version.rs
  component_diff.rs
  unit_impact.rs

Add a separate feature:

TOML
model-version-ducklake = ["dep:duckdb", "parquet-export"]

Do not reuse model-writer-ducklake. The attached strategy already recommends a separate feature because model-writer-ducklake means generation-time writer, while model version management is a publish/index/query layer. 

attachments-bundle

Use DuckLake for:

Release registry tables
model_releases, model_release_edges, model_release_files.

Imported snapshot tables
versioned_instances, versioned_geo_instances, versioned_tubings, versioned_transforms, versioned_aabb, versioned_ptsets, versioned_primitive_keypoints.

Version index tables
component_identities, component_versions, component_lineage, delivery_units, delivery_unit_membership_versions, unit_versions, propagation_rules, unit_dependency_edges, optional component_unit_impacts.

Read/query acceleration for APIs
Component diff, unit diff, impacted delivery units, release list, old/new manifest lookup.

Do not use DuckLake for:

Replacing SurrealModelWriterBackend in the MVP.

Replacing export_dbnum_instances_parquet.

Replacing the viewer’s Parquet package format.

Treating DuckLake snapshot IDs as user-facing model release IDs.

Treating transform-store-ducklake as complete pe_transform truth.

DuckLake time travel is useful for forensic table-state inspection because DuckLake snapshots represent consistent database states and can be queried by snapshot version or timestamp. 
DuckLake
 DuckLake change feed can later help incremental indexing because table_changes can return inserts, deletes, and update pre/post images between snapshots. 
DuckLake
 But the user-facing release model still needs explicit release_id, parent_release_id, branch_id, semantic_version, and release_hash, because a model release is a business/domain artifact, not merely a storage-engine snapshot. 

attachments-bundle

One caution: DuckLake can register existing Parquet files without copying, but the docs state ownership transfers to DuckLake and compaction/cleanup may delete those files. 
DuckLake
 Therefore, for this repo, either import rows with INSERT INTO ... SELECT ... FROM read_parquet(...), or register copies under a DuckLake-owned data path. Do not hand DuckLake ownership of the viewer’s live Parquet package.

3. Model version data schema/entities

Create a dedicated DuckLake schema:

SQL
CREATE SCHEMA IF NOT EXISTS model_version;

The attached strategy already recommends a dedicated model_version schema rather than reusing ducklake-canonical, because the latter belongs to the generation ModelWriter experiment. 

attachments-bundle

3.1 Release registry
model_version.model_releases

Purpose: one row per published or draft model release.

Recommended columns:

SQL
release_id TEXT PRIMARY KEY,
project TEXT NOT NULL,
site TEXT,
dbnums_json TEXT NOT NULL,

release_label TEXT,
semantic_version TEXT,
branch_id TEXT NOT NULL DEFAULT 'main',
parent_release_id TEXT,
derivation_type TEXT NOT NULL, -- full, incremental_sesno, manual_import, reconstructed_history

source_kind TEXT NOT NULL,     -- e3d_sesno_range, current_parquet, archived_parquet
source_project_path TEXT,
source_db_file_path TEXT,
from_sesno INTEGER,
to_sesno INTEGER,
target_sesno INTEGER,

generation_task_id TEXT,
generation_started_at TIMESTAMP,
generation_completed_at TIMESTAMP,
created_at TIMESTAMP NOT NULL,
created_by TEXT,

status TEXT NOT NULL,          -- draft, generated, packaged, registered, indexed, published, failed
manifest_root TEXT NOT NULL,
parquet_package_root TEXT NOT NULL,
release_hash TEXT,
hash_version TEXT NOT NULL,
rule_set_hash TEXT,
metadata_json TEXT,
generation_report_json TEXT,
export_report_json TEXT,
validation_report_json TEXT
model_version.model_release_edges

Purpose: release graph and comparison lineage.

SQL
edge_id TEXT PRIMARY KEY,
old_release_id TEXT NOT NULL,
new_release_id TEXT NOT NULL,
edge_kind TEXT NOT NULL,       -- parent, compare, branch, merge
branch_id TEXT NOT NULL,
derivation_type TEXT,
created_at TIMESTAMP NOT NULL,
edge_metadata_json TEXT

This is required because the Oracle review identified a missing release graph as a P0/P1 gap, including parent_release_id, branch_id, derivation_type, semantic_version, and release_hash. 

attachments-bundle

model_version.model_release_files

Purpose: immutable asset/file manifest and audit.

SQL
release_id TEXT NOT NULL,
dbnum INTEGER NOT NULL,
table_name TEXT NOT NULL,      -- instances, geo_instances, transforms, ...
file_path TEXT NOT NULL,
file_size_bytes BIGINT,
content_sha256 TEXT,
row_count BIGINT,
schema_hash TEXT,
source_manifest_path TEXT,
mesh_lod_tag TEXT,
missing_geo_hashes INTEGER,
missing_owner_refnos INTEGER,
created_at TIMESTAMP NOT NULL,
PRIMARY KEY (release_id, dbnum, table_name)

This table is important for site delivery: old viewer packages must remain available for side-by-side comparison even after the current export directory changes.

3.2 Versioned snapshot tables

Mirror the existing export package. The exporter already emits instances, ptsets, primitive_keypoints, geo_instances, tubings, transforms, aabb, and manifest.json. 

attachments-bundle

versioned_instances

Columns should match instances.parquet plus release keys:

SQL
release_id TEXT NOT NULL,
dbnum INTEGER NOT NULL,
refno_str TEXT NOT NULL,
refno_u64 UBIGINT NOT NULL,
noun TEXT NOT NULL,
owner_refno_str TEXT,
owner_refno_u64 UBIGINT,
owner_noun TEXT,
cata_hash TEXT,
trans_hash TEXT,
aabb_hash TEXT,
spec_value UBIGINT,
has_neg BOOLEAN,
PRIMARY KEY (release_id, dbnum, refno_u64)

The current exporter already provides exactly these comparison surfaces: identity, noun, owner, cata hash, transform hash, AABB hash, spec value, and dbnum. 

attachments-bundle

 

attachments-bundle

versioned_geo_instances
SQL
release_id TEXT NOT NULL,
dbnum INTEGER NOT NULL,
refno_str TEXT NOT NULL,
refno_u64 UBIGINT NOT NULL,
geo_index INTEGER NOT NULL,
geo_hash TEXT NOT NULL,
geo_trans_hash TEXT NOT NULL,
PRIMARY KEY (release_id, dbnum, refno_u64, geo_index)

The exporter already emits geo_index, geo_hash, and geo_trans_hash, which are the basic geometry-diff inputs. 

attachments-bundle

 

attachments-bundle

versioned_tubings
SQL
release_id TEXT NOT NULL,
dbnum INTEGER NOT NULL,
tubi_refno_str TEXT NOT NULL,
tubi_refno_u64 UBIGINT NOT NULL,
owner_refno_str TEXT NOT NULL,
owner_refno_u64 UBIGINT NOT NULL,
ord INTEGER NOT NULL,
geo_hash TEXT NOT NULL,
trans_hash TEXT NOT NULL,
aabb_hash TEXT NOT NULL,
spec_value UBIGINT,
PRIMARY KEY (release_id, dbnum, tubi_refno_u64, ord)
versioned_transforms
SQL
release_id TEXT NOT NULL,
dbnum INTEGER NOT NULL,
trans_hash TEXT NOT NULL,
m00 DOUBLE, m10 DOUBLE, m20 DOUBLE, m30 DOUBLE,
m01 DOUBLE, m11 DOUBLE, m21 DOUBLE, m31 DOUBLE,
m02 DOUBLE, m12 DOUBLE, m22 DOUBLE, m32 DOUBLE,
m03 DOUBLE, m13 DOUBLE, m23 DOUBLE, m33 DOUBLE,
PRIMARY KEY (release_id, dbnum, trans_hash)
versioned_aabb
SQL
release_id TEXT NOT NULL,
dbnum INTEGER NOT NULL,
aabb_hash TEXT NOT NULL,
min_x DOUBLE, min_y DOUBLE, min_z DOUBLE,
max_x DOUBLE, max_y DOUBLE, max_z DOUBLE,
PRIMARY KEY (release_id, dbnum, aabb_hash)
Optional tables

Keep these optional but recorded:

SQL
versioned_ptsets
versioned_primitive_keypoints

They are useful for detailed component explanation and connection-point diffs, but the MVP component/unit comparison can start with instances, geometry, transforms, AABB, and tubings.

3.3 Component identity and versions
component_identities
SQL
component_identity_hash TEXT PRIMARY KEY,
project TEXT NOT NULL,
dbnum INTEGER NOT NULL,
refno_u64 UBIGINT,
refno_str TEXT,
identity_strategy TEXT NOT NULL,     -- refno:v1 initially
identity_confidence DOUBLE NOT NULL, -- 1.0 for refno match
diagnostics_json TEXT,
created_at TIMESTAMP NOT NULL

Start with:

sha256("component_identity:v1|project|dbnum|refno_u64")

The Oracle review explicitly required component_identity_hash, identity strategy/confidence, and diagnostics for non-refno matching. 

attachments-bundle

component_versions
SQL
component_version_id TEXT PRIMARY KEY,
release_id TEXT NOT NULL,
component_identity_hash TEXT NOT NULL,
dbnum INTEGER NOT NULL,
refno_u64 UBIGINT NOT NULL,
refno_str TEXT NOT NULL,
noun TEXT NOT NULL,

owner_refno_u64 UBIGINT,
owner_refno_str TEXT,
owner_noun TEXT,
owner_path_json TEXT,

geometry_hash TEXT,
transform_hash TEXT,
aabb_hash TEXT,
attribute_hash TEXT,
membership_hash TEXT,
component_hash TEXT NOT NULL,

hash_version TEXT NOT NULL,
is_deleted BOOLEAN NOT NULL DEFAULT false,
diagnostics_json TEXT

Suggested hash groups:

geometry_hash   = sorted geo_instances(refno, geo_index, geo_hash, geo_trans_hash)
transform_hash  = trans_hash + canonical matrix values
aabb_hash       = aabb_hash + canonical numeric bounds
attribute_hash  = noun + cata_hash + spec_value + has_neg
membership_hash = unit_key + owner path + member role
component_hash  = geometry_hash + transform_hash + aabb_hash + attribute_hash + membership_hash

The implementation notes already recommend version-prefixed hash formulas, stable canonical serialization, sorted arrays, explicit null markers, fixed float/tolerance policy, and stored hash_version/rule_set_hash. 

attachments-bundle

component_lineage
SQL
edge_id TEXT NOT NULL,
component_identity_hash TEXT NOT NULL,
old_release_id TEXT NOT NULL,
new_release_id TEXT NOT NULL,
old_component_version_id TEXT,
new_component_version_id TEXT,

change_kind TEXT NOT NULL, -- unchanged, added, deleted, changed, moved, identity_conflict
field_change_mask TEXT,    -- geometry, transform, aabb, attribute, membership
old_component_hash TEXT,
new_component_hash TEXT,
identity_confidence DOUBLE,
diagnostics_json TEXT,

PRIMARY KEY (edge_id, component_identity_hash)

This is required so the system can say what happened to the same logical component across two releases, not just list row differences.

3.4 Delivery units and unit versions

Delivery units should include:

BRAN
EQUI
WALL
FLOOR
HANG

The repo findings identify these as the delivery-unit nouns used by current code. 

attachments-bundle

delivery_units
SQL
unit_key TEXT PRIMARY KEY,          -- project|dbnum|unit_refno_u64 or UNASSIGNED bucket
project TEXT NOT NULL,
dbnum INTEGER NOT NULL,
unit_refno_u64 UBIGINT,
unit_refno_str TEXT,
unit_noun TEXT NOT NULL,            -- BRAN, EQUI, WALL, FLOOR, HANG, UNASSIGNED
unit_identity_hash TEXT NOT NULL,
identity_strategy TEXT NOT NULL,
diagnostics_json TEXT
delivery_unit_membership_versions
SQL
release_id TEXT NOT NULL,
unit_key TEXT NOT NULL,
component_identity_hash TEXT NOT NULL,
component_refno_u64 UBIGINT NOT NULL,
component_refno_str TEXT NOT NULL,
component_noun TEXT NOT NULL,

membership_kind TEXT NOT NULL,      -- direct_owner, owner_chain, tubing_owner, inferred, unassigned
member_role TEXT,                   -- physical_member, tubing_segment, semantic_owner, etc.
owner_path_json TEXT,
path_confidence DOUBLE NOT NULL,
unresolved_reason TEXT,

membership_hash TEXT NOT NULL,
PRIMARY KEY (release_id, unit_key, component_identity_hash)

Unresolved membership must be stored as UNASSIGNED, not dropped. The findings explicitly warn that missing/unassigned delivery membership must be reported, never silently filtered. 

attachments-bundle

unit_versions
SQL
unit_version_id TEXT PRIMARY KEY,
release_id TEXT NOT NULL,
unit_key TEXT NOT NULL,
unit_noun TEXT NOT NULL,

parent_unit_version_id TEXT,
aggregate_hash TEXT NOT NULL,
hash_version TEXT NOT NULL,
rule_set_hash TEXT NOT NULL,

member_count INTEGER NOT NULL,
added_count INTEGER NOT NULL DEFAULT 0,
deleted_count INTEGER NOT NULL DEFAULT 0,
changed_count INTEGER NOT NULL DEFAULT 0,
moved_in_count INTEGER NOT NULL DEFAULT 0,
moved_out_count INTEGER NOT NULL DEFAULT 0,
unresolved_member_count INTEGER NOT NULL DEFAULT 0,

diagnostics_json TEXT

The Oracle review called unit_versions P0 because BRAN/EQUI/WALL need real aggregate version hashes and member counters. 

attachments-bundle

 

attachments-bundle

Suggested aggregate hash:

sha256(
  "unit_version:v1|"
  + unit_key
  + unit_noun
  + rule_set_hash
  + sorted(component_identity_hash, component_hash, member_role, membership_hash)
)
3.5 Propagation and impact audit
propagation_rules
SQL
rule_set_hash TEXT NOT NULL,
rule_id TEXT NOT NULL,
rule_version TEXT NOT NULL,

source_change_kind TEXT NOT NULL,  -- geometry, transform, aabb, attribute, membership, add, delete
source_noun_filter TEXT,           -- VALV, ELBO, TUBI, *, etc.
target_unit_noun TEXT NOT NULL,    -- BRAN, EQUI, WALL, FLOOR, HANG
impact_kind TEXT NOT NULL,         -- content_changed, moved_in, moved_out, deleted_member, added_member
severity TEXT NOT NULL,            -- info, minor, major, blocking
threshold_json TEXT,
enabled BOOLEAN NOT NULL DEFAULT true,

PRIMARY KEY (rule_set_hash, rule_id)
unit_dependency_edges
SQL
release_id TEXT NOT NULL,
component_identity_hash TEXT NOT NULL,
unit_key TEXT NOT NULL,
dependency_kind TEXT NOT NULL,     -- owner_chain, direct_owner, tubi_owner, shared_ref, inferred
path_json TEXT NOT NULL,
path_confidence DOUBLE NOT NULL,
evidence_json TEXT,
PRIMARY KEY (release_id, component_identity_hash, unit_key, dependency_kind)
component_unit_impacts
SQL
edge_id TEXT NOT NULL,
component_identity_hash TEXT NOT NULL,
unit_key TEXT NOT NULL,
old_unit_version_id TEXT,
new_unit_version_id TEXT,

impact_kind TEXT NOT NULL,
rule_set_hash TEXT NOT NULL,
rule_id TEXT NOT NULL,
dependency_path_json TEXT NOT NULL,
evidence_json TEXT,

PRIMARY KEY (edge_id, component_identity_hash, unit_key, impact_kind)

This is what turns a component-level change into delivery-unit impact. The findings explicitly state that a local component change must make its containing delivery unit dirty, and a move between BRANs must impact both old and new BRANs. 

attachments-bundle

4. Incremental/update flow
Existing flow should remain unchanged

Keep the current path:

collect_pdms_increment_for_file/dbnums
  -> refresh db_meta
  -> persist_pdms_increment_files
  -> IncrGeoUpdateLog
  -> gen_all_geos_data(..., Some(update_log), target_sesno)
  -> post_gen_export helper
  -> refresh missing pe_transform coverage
  -> export dbnum Parquet
  -> refresh SQLite spatial index
  -> viewer reads output/<project>/parquet/<dbnum>

This is already described in the plan and is now centralized in post_gen_export.rs. 

attachments-bundle

 The helper checks export_parquet_after_gen, discovers dbnums from hints/manual/db_meta/fallback, refreshes missing pe_transform coverage, exports dbnum Parquet, and optionally refreshes the SQLite spatial index. 

attachments-bundle

 

attachments-bundle

 

attachments-bundle

New release-publish flow

Add this immediately after successful post-generation Parquet export:

1. Create model release metadata
2. Copy/hardlink current Parquet package to immutable release package dir
3. Register release in DuckLake
4. Import versioned snapshot tables
5. Compute component identities
6. Resolve delivery-unit memberships
7. Compute component_versions
8. Build component_lineage against parent release
9. Compute unit_versions aggregate hashes
10. Apply propagation rules and persist component_unit_impacts
11. Mark release indexed/published

Recommended output layout:

output/<project>/
  parquet/
    1112/                         # current viewer-compatible package
    manifest_1112.json

  model_versions/
    releases/
      <release_id>/
        parquet/
          1112/
            instances.parquet
            geo_instances.parquet
            transforms.parquet
            aabb.parquet
            tubings.parquet
            ptsets.parquet
            primitive_keypoints.parquet
            manifest.json
          manifest_1112.json
        release.json
        validation.json

    ducklake/
      metadata.ducklake
      data/

The current exporter already writes both a dbnum-local manifest.json and a web-compatible root manifest_<dbnum>.json whose table paths include the dbnum prefix. 

attachments-bundle

 

attachments-bundle

 That makes versioned package serving straightforward.

Release status transitions

Use explicit states:

draft
generated
packaged
registered
indexed
published
failed

Rules:

generated: SurrealDB generation completed.

packaged: immutable release directory exists and checksums are recorded.

registered: release metadata and files are stored in DuckLake.

indexed: component identities, lineage, memberships, unit versions, and impacts are computed.

published: viewer/API may list this release for side-by-side comparison.

failed: keep diagnostics and partial stage info, never silently delete.

Incremental re-indexing

For MVP, re-index the full affected dbnum package, especially for dbnum 1112. This is simpler and safer.

Later optimization:

IncrGeoUpdateLog refnos
  -> component candidates
  -> owner-chain/delivery-unit expansion
  -> recompute changed components
  -> recompute impacted units only

DuckLake change feed can later help if release imports are represented as DuckLake table mutations, but do not base the MVP on it. The domain diff should compare explicit old_release_id and new_release_id, not “whatever changed between DuckLake snapshots.”

Historical releases

Do not replay old sesno ranges into the same current SurrealDB namespace. The plan explicitly warns that replaying older sesno after newer data has been persisted can regress current SurrealDB state, and recommends an isolated DB, snapshot namespace, or no-save/history mode for historical version comparison. 

attachments-bundle

 

attachments-bundle

For historical release reconstruction, use one of:

A. Existing archived Parquet package -> register as release
B. Isolated SurrealDB namespace -> generate at target sesno -> export Parquet -> register release
C. Future no-save/history generation mode -> export package without mutating current state

A is best for MVP if old packages exist. B is safer than replaying into current state. C is a later engineering improvement.

5. Viewer/API integration plan
Backend API

Add src/web_api/model_version_api.rs with read-only endpoints first:

http
GET  /api/model-version/releases
GET  /api/model-version/releases/:release_id
GET  /api/model-version/releases/:release_id/package/:dbnum/manifest
GET  /api/model-version/releases/:release_id/package/:dbnum/file/:table

POST /api/model-version/component-diff
POST /api/model-version/component-impact
POST /api/model-version/unit-diff
POST /api/model-version/compare

The implementation strategy already proposes the CLI/API surface around release list, component diff, component impact, and unit diff. 

attachments-bundle

Example request:

JSON
{
  "project": "AvevaMarineSample",
  "old_release_id": "ams-1112-sesno-896",
  "new_release_id": "ams-1112-sesno-897",
  "dbnum": 1112,
  "scope": {
    "unit_nouns": ["BRAN", "EQUI", "WALL", "FLOOR", "HANG"]
  }
}

Example response shape:

JSON
{
  "old_release_id": "ams-1112-sesno-896",
  "new_release_id": "ams-1112-sesno-897",
  "dbnum": 1112,
  "summary": {
    "components_added": 0,
    "components_deleted": 0,
    "components_changed": 12,
    "components_moved": 1,
    "units_impacted": 3
  },
  "packages": {
    "old_manifest_url": "/api/model-version/releases/ams-1112-sesno-896/package/1112/manifest",
    "new_manifest_url": "/api/model-version/releases/ams-1112-sesno-897/package/1112/manifest"
  },
  "changed_components": [
    {
      "component_identity_hash": "sha256:...",
      "refno_str": "...",
      "noun": "VALV",
      "change_kind": "changed",
      "field_change_mask": ["geometry", "transform"],
      "old_component_hash": "sha256:...",
      "new_component_hash": "sha256:...",
      "old_unit_key": "AvevaMarineSample|1112|...",
      "new_unit_key": "AvevaMarineSample|1112|..."
    }
  ],
  "impacted_units": [
    {
      "unit_key": "AvevaMarineSample|1112|...",
      "unit_noun": "BRAN",
      "impact_kind": "content_changed",
      "old_unit_version_id": "...",
      "new_unit_version_id": "...",
      "old_aggregate_hash": "sha256:...",
      "new_aggregate_hash": "sha256:...",
      "rule_id": "member_geometry_changes_unit:v1",
      "dependency_path": ["VALV", "BRAN"]
    }
  ]
}
Viewer integration

Do not redesign the viewer payload first. The current model viewer consumes Parquet packages generated by export_dbnum_instances_parquet, and the exporter already produces web-compatible manifests. Keep that contract.

Implement the two-view comparison like this:

left viewport  = load old_release manifest_1112.json
right viewport = load new_release manifest_1112.json
shared camera  = synchronized orbit/pan/zoom/clipping
diff overlay   = loaded from /api/model-version/compare
selection link = selecting component in one view selects matching component_identity_hash in the other

Visual states:

unchanged
added
deleted
changed_geometry
changed_transform
changed_attributes
moved_unit
unassigned_or_unresolved

The API, not the viewer, should own the domain logic. The viewer should only render two release packages and apply returned diff classifications.

Package serving strategy

Expose release packages through API paths that look like the current Parquet root:

/api/model-version/releases/<release_id>/parquet/manifest_1112.json
/api/model-version/releases/<release_id>/parquet/1112/instances.parquet
/api/model-version/releases/<release_id>/parquet/1112/geo_instances.parquet
...

That avoids changing the low-level Parquet loader. It only needs a configurable package root per viewport.

API performance

Start with precomputed diff/index tables. Avoid calculating owner-chain membership, component hashes, and unit aggregate hashes inside every HTTP request. The compare endpoint should be a lookup over:

component_lineage
component_versions
delivery_unit_membership_versions
unit_versions
component_unit_impacts
6. Edge cases and validation strategy
Edge cases to handle explicitly

The incremental plan already lists many important generation/export edge cases: wrong historical db file path, to_sesno greater than latest, from_sesno >= to_sesno, no model-affecting elements, delete-only increments, unknown nouns, dbnum discovery failure, exclude_db_nums removing all candidates, missing/stale pe_transform, disabled parquet-export or sqlite-index, unwritable output, missing scene_tree, and historical replay risk. 

attachments-bundle

Add these version-specific cases:

Release package already registered
Registration should be idempotent if file hashes match, and a hard duplicate error if the same release_id points to different content.

Missing optional Parquet tables
ptsets and primitive_keypoints may be empty or optional; instances, geo_instances, transforms, and aabb should be required for 3D comparison.

Schema drift
Store schema_hash per file. Fail import if required columns are missing. Allow optional columns only through explicit schema-version logic.

Old mesh assets missing
Release package must record mesh validation counts and asset roots. A release with missing GLBs can still be indexed, but the viewer/API should show degraded renderability.

Component deleted and recreated
Refno identity may treat it as the same component. Store identity_strategy, identity_confidence, and diagnostics so a future identity algorithm can distinguish recreation.

Component moved between units
Both old and new delivery units are impacted: old gets moved_out, new gets moved_in.

Indirect ownership
Cases like VALV -> EQUI -> BRAN must not be lost. Resolve owner chains and store path evidence. The Oracle findings call out direct-owner-only membership as a risk. 

attachments-bundle

Shared references / over-propagation
Shared components should impact only units with explicit dependency edges. Do not infer every possible BRAN without evidence.

Tiny geometry changes
Use rule thresholds for transform/geometry tolerance. Store threshold in propagation_rules.threshold_json.

Floating-point instability
Canonicalize matrix/AABB values using fixed precision or tolerance policy before hashing.

Unassigned membership
Store UNASSIGNED with reason. Do not drop the component from diff or impact reports.

Branch comparisons
MVP supports one parent edge. Later branch/merge can extend model_release_edges.

DuckLake maintenance deleting files
Do not register viewer-owned Parquet files directly with ducklake_add_data_files; import rows or register DuckLake-owned copies because DuckLake may assume ownership of registered Parquet files. 
DuckLake

Validation strategy

Do not validate this with broad cargo test. The repo planning notes explicitly say not to create/run cargo tests for this work and to use CLI + JSON checks; web-server behavior should be verified by running the service and HTTP/POST. 

attachments-bundle

 

attachments-bundle

Phase 1: generation/export stays green

Repeat the known-good generation path:

PowerShell
target\debug\aios-database.exe -c db_options/DbOption `
  --export-parquet-after-gen `
  incremental-sesno `
  --file D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001 `
  --from-sesno 896 `
  --to-sesno 897 `
  --generate-model `
  --json

Expected evidence:

dbnum = 1112
generation scoped to dbnum 1112
parquet_export.enabled = true
exported_dbnums includes 1112
output_dir = output/AvevaMarineSample/parquet
manifest row counts match actual Parquet files
missing_geo_hashes = 0 for the current validation sample
Phase 2: release registration

Add CLI:

PowerShell
aios-database --model-version-register `
  --project AvevaMarineSample `
  --dbnum 1112 `
  --parquet-dir output\AvevaMarineSample\model_versions\releases\ams-1112-sesno-896\parquet\1112 `
  --release-id ams-1112-sesno-896 `
  --json

aios-database --model-version-register `
  --project AvevaMarineSample `
  --dbnum 1112 `
  --parquet-dir output\AvevaMarineSample\model_versions\releases\ams-1112-sesno-897\parquet\1112 `
  --release-id ams-1112-sesno-897 `
  --parent-release-id ams-1112-sesno-896 `
  --json

Acceptance:

model_releases has two releases
model_release_edges has one parent edge
model_release_files row counts match manifest
registration is idempotent when checksums match
Phase 3: component and unit indexing
PowerShell
aios-database --model-version-index-components --release-id ams-1112-sesno-896 --json
aios-database --model-version-index-components --release-id ams-1112-sesno-897 --json
aios-database --model-version-index-units --release-id ams-1112-sesno-896 --json
aios-database --model-version-index-units --release-id ams-1112-sesno-897 --json

Acceptance:

component_identities exist
component_versions exist for both releases
delivery_unit_membership_versions exist
BRAN/EQUI/WALL/FLOOR/HANG unit_versions exist where present
same release re-index produces identical hashes
unresolved membership count is reported
Phase 4: diff and impact proof
PowerShell
aios-database --model-version-diff-component `
  --old-release-id ams-1112-sesno-896 `
  --new-release-id ams-1112-sesno-897 `
  --refno <KNOWN_CHANGED_REFNO> `
  --json

aios-database --model-version-impact-component `
  --old-release-id ams-1112-sesno-896 `
  --new-release-id ams-1112-sesno-897 `
  --refno <KNOWN_CHANGED_REFNO> `
  --json

Minimum proof:

component_identity_hash is stable
component_hash changes when geometry/transform/AABB/attribute/membership changes
component_lineage row explains added/deleted/changed/moved
old and new unit membership are returned
impacted BRAN/EQUI/WALL/FLOOR/HANG units are returned
rule_id and dependency_path evidence are present
containing BRAN aggregate_hash changes for a version-significant member change

That exact MVP closure is already identified in the planning review: release snapshot -> component identity/lineage -> unit membership versions -> unit aggregate hash -> rule-based impact -> BRAN version hash changes. 

attachments-bundle

Phase 5: HTTP/API validation

Run the web server with generation disabled in startup config, then verify:

http
GET  /api/model-version/releases
POST /api/model-version/compare
POST /api/model-version/component-diff
POST /api/model-version/component-impact
POST /api/model-version/unit-diff

Acceptance:

release list shows both releases
compare returns old/new manifest URLs
component diff count matches CLI
unit impact count matches CLI
viewer can load old and new manifests simultaneously
selecting a changed component highlights corresponding old/new component identity
Phased development plan for this repo
Phase 0 — lock the boundary

Do not touch:

src/fast_model/gen_model/model_writer_ducklake.rs
src/pe_transform_store.rs DuckLake stub
export_dbnum_instances_parquet output schema
current viewer package format
SurrealDB generation writer

The attached strategy explicitly says the first pass should be additive and should not refactor those pieces. 

attachments-bundle

Phase 1 — model-version feature and schema bootstrap

Add:

Cargo.toml:
  model-version-ducklake = ["dep:duckdb", "parquet-export"]

src/version_management/
  types.rs
  hashing.rs
  ducklake_store.rs
  model_release.rs
  release_graph.rs
  snapshot_import.rs
  cli.rs

Acceptance:

CLI creates DuckLake schema
registers one release
imports instances/geo_instances/transforms/aabb
lists releases as JSON
Phase 2 — immutable release package

Add a post-export publisher:

Rust
publish_model_release_from_parquet(
    project,
    dbnums,
    current_parquet_root,
    release_metadata
)

It should copy or hardlink:

output/<project>/parquet/<dbnum>
  -> output/<project>/model_versions/releases/<release_id>/parquet/<dbnum>

Then checksum files and write release.json.

Phase 3 — component identity, membership, component versions

Add:

component_identity.rs
delivery_unit.rs
component_version.rs
component_lineage.rs

Acceptance:

known refno returns same component_identity_hash across two releases
component_hash changes when expected
old/new delivery membership is returned
Phase 4 — unit versions and impact rules

Add:

unit_version.rs
propagation_rules.rs
unit_dependency.rs
unit_impact.rs

Acceptance:

BRAN/EQUI/WALL/FLOOR/HANG aggregate hashes are deterministic
member change changes containing unit hash when rule says it is version-significant
impact output includes rule_id and path evidence
Phase 5 — diff API and two-view integration

Add read-only API endpoints and release package serving.

Acceptance:

two release manifests load in side-by-side viewer
component/unit diff overlays come from API
no viewer dependency on DuckLake internals
Phase 6 — optimization

After correctness:

partition DuckLake tables by release_id and dbnum
cache common compare results
incrementally re-index affected units
support branch/merge release graph
add non-refno identity matching
add no-save/history generation mode

DuckLake partitioning can help once row counts are measured; partitioning by release_id and dbnum is safer than high-cardinality refno early on, and DuckLake’s partition docs confirm partition keys affect new data and are used for file pruning. 
DuckLake

Final answer

Use DuckLake, but not as the model generator. The best architecture is:

SurrealDB remains the generation source of truth for MVP.
Parquet remains the viewer package format.
DuckLake becomes the model-version release/index/query layer.
Domain version tables provide release graph, component identity, lineage, delivery-unit versions, and auditable impact propagation.

This gives users side-by-side 3D comparison without destabilizing the recently fixed incremental-sesno -> scoped generation -> Parquet export pipeline, while still creating the foundation needed for real historical releases, component diff, and BRAN/EQUI/WALL/FLOOR/HANG delivery-unit impact.
