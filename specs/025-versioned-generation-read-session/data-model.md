# Data Model

## Domain records

### InputVersionManifest

- `authoritative_snapshot_id: u64`
- `versions: BTreeMap<u32, DataVersion>`
- `manifest_hash: String`
- `history_start_snapshot: u64`

`DataVersion` contains `dbnum`, `sesno` and `commit_fingerprint`. The manifest hash is computed from canonical, dbnum-sorted data and never includes the selected read backend.

### VersionedReadSession

An immutable runtime view containing the manifest, backend kind and session metrics. A session cannot change backend or snapshot.

### ElementSnapshot / AttributeSet

`ElementSnapshot` stores backend-neutral identity and projected PE facts. `AttributeSet` is a sorted map of explicit tagged values. The persisted payload has a codec version and canonical hash.

### HierarchyRow / CatalogNode

`HierarchyRow` stores `(dbnum, parent, child, ordinal)`. `CatalogNode` stores element identity, noun, owner, ordered children and ordered outbound reference edges. Shared Rust code owns all traversal behavior.

### ReplicaSnapshotBinding

- `authoritative_snapshot_id`
- `previous_snapshot_id`
- `replica_version_time`
- `manifest_hash`
- `payload_hash`
- `status = applied`

Bindings form one contiguous sequence starting at `history_start_snapshot`.

## DuckLake tables

- `data_version_state(dbnum, sesno, commit_fingerprint, source)`
- `db_catalog(dbnum, ref0, db_type, project)`
- `element(dbnum, refno, owner_refno, noun, name, attr_codec_version, attr_payload, attr_hash)`
- `hierarchy_edge(dbnum, parent_refno, child_refno, ordinal)`
- `reference_edge(dbnum, source_refno, attr_name, target_refno, ordinal)`
- `transform(dbnum, refno, local_transform, world_transform, transform_hash)`

All data tables are partitioned by `dbnum`. One DuckLake transaction mutates domain tables and `data_version_state`; its commit metadata includes the unique commit fingerprint.

## Invariants

1. One input manifest resolves to one real global DuckLake snapshot.
2. A Surreal session opens only when an applied binding with the same manifest hash exists.
3. Missing required rows are data errors, not empty/default values.
4. Child ordering uses persisted ordinal; unordered sets are sorted before hashing.
5. Attribute reference edges are derived from the same decoded payload written to `element`.
6. A failed replica transaction writes neither partial domain changes nor an applied binding.
