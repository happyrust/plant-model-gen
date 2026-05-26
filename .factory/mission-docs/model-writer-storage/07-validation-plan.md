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
