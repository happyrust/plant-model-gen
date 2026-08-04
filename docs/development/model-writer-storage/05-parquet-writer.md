# Parquet Writer

## Role

The Parquet writer is a file-oriented implementation of the canonical raw/projection contract. It is useful for export, offline inspection, and SQL validation through DuckDB or other Parquet-capable tools.

Parquet is not the final DuckLake architecture. DuckLake should use the Rust DuckDB binding directly for durable table writes.

## Layout

Recommended layout:

```text
output/<project>/model_writer_storage/
  raw/<table>/project_name=<project>/dbnum=<dbnum>/*.parquet
  projection/<table>/project_name=<project>/dbnum=<dbnum>/*.parquet
```

Tables should use the canonical names from `02-canonical-schema.md`.

## Write behavior

- Write raw canonical records as typed Parquet rows.
- Keep stable scalar ids for `refno`, `inst_id`, `geo_hash`, `aabb_id`, `trans_id`, and `vec3_id`.
- Avoid embedding SurrealDB record-id syntax as the only identifier.
- Emit projection Parquet files from the same canonical records or from SQL over raw files.

## Validation

Parquet validation is CLI + SQL:

1. Generate SurrealDB baseline data.
2. Generate Parquet data for the same dbnum/refno scope.
3. Query Parquet with SQL.
4. Compare counts, key sets, relation edges, and projection joins against SurrealDB exports.

## Phase boundary

Phase 1 Parquet excludes `inst_relate_bool` and `inst_relate_cata_bool`. Those boolean result tables are Phase 2.

The first implementation scaffold exposes `CanonicalParquetWriter` under `model_writer` without routing production writes to it by default. To avoid pulling the heavy optional Parquet/Polars stack into the current SurrealDB path before CLI parity checks exist, the scaffold writes canonical raw table JSON Lines files plus a row-count summary under the target Parquet layout boundary. Typed `.parquet` materialization remains the next Parquet worker step.
