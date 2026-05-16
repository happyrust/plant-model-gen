# Phase Roadmap

## Worker boundaries

| Worker area | Owns | Does not own |
|---|---|---|
| Contract worker | Canonical raw/projection schema, parity matrix, table naming | Backend-specific IO tuning |
| Surreal compatibility worker | Current write contract extraction and Surreal parity exports | DuckLake/Parquet primary implementation |
| Writer trait worker | `ModelWriterBackend` extension points and canonical adapter boundary | Changing generation semantics |
| DuckLake worker | Rust DuckDB binding writer, `ducklake-canonical` schema, SQL projections | Temp-Parquet-plus-SQL as final architecture |
| Parquet worker | Canonical Parquet layout and file SQL validation | DuckLake metadata ownership |
| Orchestrator worker | Backend selection, batch lifecycle, compare mode integration | Backend table internals |
| Validation worker | CLI + SQL scripts, parity reports, Cargo source checks | Rust tests |

## Phase 0: Docs-first contract

- Create mission docs.
- Freeze Phase 1 vs Phase 2 table boundaries.
- Confirm `inst_relate_bool` and `inst_relate_cata_bool` are Phase 2.

## Phase 1: Canonical raw writer boundary

- Add canonical record structs for all Phase 1 objects.
- Adapt existing generation batches into canonical raw records.
- Preserve current SurrealDB writer behavior.
- Add row-count summaries by canonical table.

## Phase 2: Candidate backend writers

- Implement Parquet writer against canonical raw/projection schema.
- Implement DuckLake writer through Rust DuckDB binding.
- Create `ducklake-canonical` schema and projection refresh SQL.
- Keep DuckLake final architecture independent of temp-Parquet-plus-SQL.

## Phase 3: Orchestrator and compare mode

- Add explicit backend selection.
- Add optional dual-write/compare path.
- Fail fast on write errors.
- Report table-level parity summaries.

## Phase 4: CLI + SQL validation

- Add CLI commands or modes for backend validation.
- Add SQL checks for every Phase 1 object.
- Confirm Cargo files do not contain `gitee.com/happydpc/surrealdb`.
- Confirm SurrealDB source remains `github.com/happyrust/surrealdb`.

## Phase 5: Boolean result storage

- Add canonical records for `inst_relate_bool`.
- Add canonical records for `inst_relate_cata_bool`.
- Extend projections with `projection_boolean_results`.
- Validate through CLI + SQL only.
