# Oracle Review Summary

Date: 2026-06-17
Session: `ducklake-model-version-management-review-4`
Transcript: `C:\Users\dpc\.oracle\sessions\ducklake-model-version-management-review-4\artifacts\transcript.md`

## Verdict

Oracle's core verdict:

> The current plan is a "snapshot store + diff engine", not yet a complete model version system.

DuckLake is still appropriate, but only as immutable snapshot storage and query layer. The model version system must be implemented as domain metadata and deterministic indexing on top.

## Required P0 Changes

1. Add a release graph:
   - `parent_release_id`
   - `branch_id`
   - `derivation_type`
   - `semantic_version`
   - `release_hash`
2. Add component identity:
   - `component_identity_hash`
   - identity strategy and confidence
   - diagnostics for non-refno matching
3. Add component lineage:
   - map identity to component versions across release edges
   - record added/deleted/changed/moved/identity-conflict states
4. Add unit versions:
   - `unit_version_id`
   - `release_id`
   - `unit_key`
   - `aggregate_hash`
   - member and change counters
   - `parent_unit_version_id`
5. Make impact propagation deterministic:
   - explicit propagation rules
   - dependency edges
   - rule id and path evidence in impact output

## Corrected MVP

The MVP should prove this chain:

```text
register two releases in DuckLake
  -> connect releases with a graph edge
  -> build component identities and lineage
  -> build delivery-unit membership versions
  -> build unit_versions aggregate hashes
  -> diff one changed component
  -> apply propagation rules
  -> prove containing BRAN aggregate hash changed
```

## Risk Classification

P0:

- Missing unit version model means BRAN has no real version semantics.
- Missing component identity means cross-release component tracking is unstable.
- Runtime-only propagation is not reproducible or auditable.

P1:

- Membership based only on direct owner can miss indirect chains like `VALV -> EQUI -> BRAN`.
- No release graph means no lineage.
- No aggregate unit hash means unit comparison is not stable.

P2:

- DuckLake as snapshot store is reasonable.
- Diff cache can come later.
- 3D viewer should wait until version anchors are stable.
