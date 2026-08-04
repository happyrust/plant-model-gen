# Validation Plan

## Rule

Validation is CLI + SQL only. Do not use Rust tests for this mission.

## Validation levels

### 1. Static checks

- Confirm docs and implementation references include required Phase 1 table names.
- Confirm Cargo files do not reference `gitee.com/happydpc/surrealdb`.
- Confirm Cargo files keep SurrealDB source on `github.com/happyrust/surrealdb`.

### 2. Generation checks

Use the existing CLI flow for `aios-database` with JSON/config inputs. Generate the same dbnum/refno scope for SurrealDB and the candidate backend.

Expected evidence:

- command line used
- dbnum/refno scope
- rows written by canonical raw table
- rows written/refreshed by projection table
- elapsed time

### 2a. Canonical raw writer smoke check

Use the non-production CLI validation entry to exercise `CanonicalRawPlanner` and
the canonical Parquet writer scaffold without full model generation and without
SurrealDB writes:

```powershell
cargo run --bin aios-database -- model-writer validate-canonical-parquet --output output/model-writer-validation --project-name canonical-validation --dbnum 0 --batch-id 1
```

The command builds an empty/default `ShapeInstancesData`, writes canonical raw
table JSONL fallback files, and prints a JSON report containing:

- `summary_path`
- `raw_root`
- `total_rows`
- per-table `rows` and `path`

For the empty fixture, all row counts are expected to be `0`; the validation
still confirms the table file layout and summary JSON structure.

### 3. SQL parity checks

Run SQL comparisons for:

- row counts by table
- missing keys in either backend
- `inst_relate` refno-to-instance edges
- `geo_relate` instance-to-geometry edges
- `tubi_relate` tubing edges
- `neg_relate` and `ngmr_relate` dependency edges
- `inst_relate_aabb` bounds linkage
- orphan checks for `aabb`, `trans`, and `vec3`
- `refno_assoc_index` delete/index metadata coverage

## Acceptance criteria

Phase 1 passes when CLI + SQL evidence shows parity for every Phase 1 object and documents any intentional, reviewed projection-only representation.

Phase 1 does not require parity for:

- `inst_relate_bool`
- `inst_relate_cata_bool`

Those are Phase 2.

## Example SQL checks

```sql
-- Candidate backend missing instance relations present in SurrealDB export.
SELECT refno
FROM surreal_inst_relate
EXCEPT
SELECT refno
FROM candidate_raw_inst_relate;

-- Geometry relation cardinality by instance.
SELECT inst_id, COUNT(*) AS geo_edges
FROM candidate_raw_geo_relate
GROUP BY inst_id;
```
