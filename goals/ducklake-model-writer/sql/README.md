# DuckLake vs SurrealDB Parity SQL

Slice 6 (c) of `goals/ducklake-model-writer/`. Each of the 9 in-scope raw
tables gets a parity script that records:

1. row count
2. primary key set
3. 3 sample primary keys (`LIMIT 3 OFFSET 0 / midpoint / last`)

The DuckLake side runs through DuckDB CLI / Rust `duckdb` crate against
`output/<project>/model_writer_storage/ducklake/metadata.ducklake`. The
SurrealDB side runs through the `surreal sql` CLI or `surrealdb-rs` against
the live `ws://127.0.0.1:8020` / `ns=1516 db=AvevaMarineSample` setup.

## Files

| File | Table | DuckLake-side query | SurrealDB-side query |
| --- | --- | --- | --- |
| `parity_raw_inst_info.sql` | `raw_inst_info` | DuckDB | `parity_surreal_inst_info.surql` |
| `parity_raw_inst_relate.sql` | `raw_inst_relate` | DuckDB | `parity_surreal_inst_relate.surql` |
| `parity_raw_inst_geo.sql` | `raw_inst_geo` | DuckDB | `parity_surreal_inst_geo.surql` |
| `parity_raw_geo_relate.sql` | `raw_geo_relate` | DuckDB | `parity_surreal_geo_relate.surql` |
| `parity_raw_neg_relate.sql` | `raw_neg_relate` | DuckDB | `parity_surreal_neg_relate.surql` |
| `parity_raw_ngmr_relate.sql` | `raw_ngmr_relate` | DuckDB | `parity_surreal_ngmr_relate.surql` |
| `parity_raw_aabb.sql` | `raw_aabb` (mesh AABB only) | DuckDB | `parity_surreal_aabb.surql` |
| `parity_raw_vec3.sql` | `raw_vec3` (mesh pts only) | DuckDB | `parity_surreal_vec3.surql` |
| `parity_raw_inst_relate_aabb.sql` | `raw_inst_relate_aabb` | DuckDB | `parity_surreal_inst_relate_aabb.surql` |

`run_parity.ps1` runs every DuckDB query against the freshly generated
DuckLake metadata and appends the per-table summary as one JSONL line per
table into `goals/ducklake-model-writer/progress.jsonl`. The SurrealQL
equivalents must be run separately and diffed by hand or by a follow-up
helper script.

## Known Gaps

- `raw_neg_relate` rows include sentinel target `__reconcile_pending__` from
  Slice 4 conservative reconcile; parity SQL filters those out with
  `WHERE target_refno <> '__reconcile_pending__'` when comparing to
  SurrealDB. The sentinel-only rows are an explicit Known Gap until the
  reconcile resolver is implemented.
- `raw_aabb`, `raw_vec3`, `raw_inst_relate_aabb` cover only mesh-derived
  rows; tubi-side AABB / pts / transforms remain in `cata_model.rs` direct
  SurrealQL writes and are Known Gap per `brief.md` Q1=C scope.
- 5 categories of Phase 1 trait gap tables (`raw_tubi_info /
  raw_tubi_relate / raw_aabb(tubi) / raw_trans / raw_vec3(tubi) /
  raw_refno_assoc_index`) are NOT written to DuckLake; parity scripts for
  them are not provided in this slice.

## Run order

1. Wait for `cargo run --bin aios-database ... --regen-model --dbnum 7997
   --model-writer ducklake` to finish.
2. `pwsh -File goals/ducklake-model-writer/sql/run_parity.ps1` to capture
   DuckLake-side counts.
3. Optionally run the `.surql` siblings against `surreal sql` and diff the
   counts/keys/samples manually.
4. Append a `slice_6_c_parity` summary jsonl row to `progress.jsonl`.
