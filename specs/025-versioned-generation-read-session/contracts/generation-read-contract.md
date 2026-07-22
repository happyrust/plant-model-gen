# Generation Read Contract

## Session opening

`open_session(manifest)` performs all validation before returning:

1. backend contains the requested authoritative snapshot;
2. the snapshot's canonical input version manifest equals `manifest`;
3. for SurrealDB, the contiguous replica watermark covers the snapshot and the applied binding hash matches;
4. the session pins that physical version for its entire lifetime.

There is no fallback. A caller may explicitly open a new session on another backend after the first attempt fails.

## Capability semantics

- `ElementRead::load_elements(refnos)` returns one result per unique input key plus a sorted missing set.
- `AttributeRead::load_attribute_sets(refnos)` returns the complete typed set for each found element; absent optional fields stay absent.
- `HierarchyRead::load_hierarchy(dbnums)` returns all nodes and ordered direct edges required to construct a shared immutable hierarchy snapshot.
- `CatalogGraphRead::load_catalog_nodes(refnos)` returns owner, noun, ordered children and all outbound Refno edges for shared closure expansion.
- `TransformRead::load_transforms(refnos)` returns local and world transforms from the same session snapshot.

Empty input returns an empty result without backend I/O. Duplicate inputs are deduplicated before I/O. Point helpers, when present, delegate to one batch call and are forbidden in generation hot loops.

## Errors

- `SnapshotUnavailable`: requested authoritative snapshot does not exist.
- `ManifestMismatch`: physical snapshot and requested manifest differ.
- `ReplicaLagging`: replica watermark is behind the requested snapshot.
- `ReplicaBindingMissing`: watermark or time-travel binding is incomplete.
- `MissingRequiredData`: one or more required keys are absent.
- `PayloadCorrupt`: codec version, decode or canonical hash validation failed.
- `BackendQuery`: backend query failed; SQL text and credentials are not exposed to callers.
- `ParityMismatch`: compare mode found unequal canonical results.

## Compare mode

Compare mode executes both adapters against the same input manifest, canonicalizes key order, compares values, missing sets and ordered relationships, and returns the configured primary result only when they match. A mismatch fails the generation run.
