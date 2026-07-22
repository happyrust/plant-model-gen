# Implementation Plan

## Architecture

1. Define backend-neutral version, element, attribute, hierarchy, catalog and transform records.
2. Define batch-first capability traits and an immutable `VersionedReadSession`.
3. Move hierarchy and CATA traversal semantics into shared Rust services.
4. Implement the Surreal adapter first and migrate model generation to injected sessions.
5. Add the DuckLake authoritative schema, commit mapping, current-state bootstrap and atomic Surreal replica binding.
6. Implement a snapshot-pinned DuckLake adapter and a strict compare wrapper.
7. Replace generated-state database backfills with `GenerationArtifacts`.
8. Add parity, failure, performance, packaging and staged rollout gates.

## Deployment

The first implementation uses a local SQLite DuckLake metadata catalog and local Parquet data directory. Windows packages include a signed `ducklake.duckdb_extension` matching the embedded DuckDB version; runtime installation never depends on network access.

## Migration

Existing sites export only their current committed Surreal state into the first DuckLake snapshot. Counts, payload hashes and the input version manifest are verified before recording `history_start_snapshot` and the initial replica binding. Earlier history remains available only through legacy Surreal tools.

## Rollout

The default remains `surreal` until contract and end-to-end parity gates pass. `compare` is an explicit fail-closed canary mode. After acceptance, `ducklake` becomes the configured default; no mode performs automatic backend fallback.
