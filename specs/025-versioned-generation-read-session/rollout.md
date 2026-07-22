# Rollout

## Runtime contract

- `generation_read_backend` must be one of `surreal`, `ducklake`, or `compare`.
- A generation run opens one `VersionedReadSession` and remains pinned to one
  `authoritative_snapshot_id`; there is no automatic fallback.
- `compare` requires both adapters to cover the same manifest and fails the run on any
  normalized DTO, missing-set, or ordering difference.
- The formal versioned path requires `boolean_pipeline_mode = "memory_tasks"` and
  `enable_db_backfill = false`.

## Stages

1. Deploy the snapshot-pinned Surreal adapter as the explicit default.
2. Bootstrap the current committed state into the first DuckLake snapshot and record
   `history_start_snapshot`.
3. Replicate that snapshot atomically to Surreal and verify its binding and payload hash.
4. Enable `compare` for canary projects. Require zero parity differences, no hot-path
   N+1 violations, and no more than 10% elapsed-time regression against the Surreal
   release baseline.
5. Change the explicit default to `ducklake`. Keep `surreal` as an operator-selected
   rollback mode; never switch backend inside a running session.

Bootstrap and release-gate commands:

```powershell
aios-database model-version bootstrap-generation-read --json

./scripts/smoke/generation_read_perf_gate.ps1 `
  -SurrealBaseline output/<project>/profile/<surreal-report>.json `
  -DuckLakeCandidate output/<project>/profile/<ducklake-report>.json
```

Both performance reports must carry the same authoritative snapshot,
`GenerationArtifacts` semantic hash, and final-model semantic hash. The gate also rejects
capability call counts above one and an end-to-end regression greater than 10%.

## Windows offline bundle

The bundle contains:

- `runtime/ducklake/metadata`
- `runtime/ducklake/data`
- `runtime/ducklake/temp`
- `runtime/ducklake/extensions/ducklake.duckdb_extension`
- `runtime/ducklake/extensions/sqlite.duckdb_extension`

`scripts/package/build-windows-bundle.ps1` requires an explicit DuckDB core version and
SHA-256 values for both extensions. Packaging fails when either asset is absent or its
checksum differs. Production startup only executes `LOAD` on these local files; it never
executes an online `INSTALL`.

Example:

```powershell
./scripts/package/build-windows-bundle.ps1 `
  -DuckDbCoreVersion 1.5.4 `
  -DuckLakeExtensionSha256 <sha256> `
  -DuckDbSqliteExtensionSha256 <sha256>
```
