# Findings

Date: 2026-06-17

## Repository Findings

- The best source for MVP version data is the existing normalized Parquet export in `src/fast_model/export_model/export_dbnum_instances_parquet.rs`.
- Existing export tables already contain the comparison surfaces needed for a single component:
  - identity and ownership from `instances`
  - geometry membership from `geo_instances`
  - placement from `transforms`
  - bounds from `aabb`
  - optional tubing/ptset details from `tubings`, `ptsets`, and `primitive_keypoints`
- Delivery-unit nouns in current code are `BRAN`, `HANG`, `EQUI`, `WALL`, `FLOOR`. User wording `EQUIP` should be treated as an alias for `EQUI` unless real data shows a distinct noun.
- Valve wording needs confirmation. Existing code references `VALV` as a component noun; user wording `VALE` should be accepted as a search/display alias if needed.
- `src/fast_model/gen_model/model_writer_ducklake.rs` is real but scoped to model writer raw tables and still carries known gaps. It is useful later, but it should not block the release-diff MVP.
- `src/pe_transform_store.rs` has a DuckLake registration function that currently returns `Ok(())` under the feature, so transform DuckLake is not a complete source of truth.
- `src/version_management/` currently stores status/log style structures and can be extended, but it does not yet contain model release/diff concepts.

## DuckLake Findings

- DuckLake fits the plan because it combines SQL metadata with Parquet-backed data files.
- DuckLake time travel is useful for forensic table-state inspection, but app-level `release_id` is still required for user-facing model versions.
- DuckLake change feed may help later for incremental indexing, but the MVP should compute release-to-release diffs from explicit release tables.
- DuckLake partitioning should be delayed until real row counts are measured. `dbnum` and possibly `release_id` are safer early partition candidates than high-cardinality refno.

Official docs:

- https://ducklake.select/docs/stable/
- https://ducklake.select/docs/stable/duckdb/usage/time_travel
- https://ducklake.select/docs/stable/duckdb/advanced_features/partitioning
- https://ducklake.select/docs/stable/duckdb/advanced_features/data_change_feed

## Feasibility

Feasible with low-to-medium risk if DuckLake is introduced as a release registry/index layer over current exports.

Higher risk if the project attempts to replace the generation writer immediately, because the existing DuckLake writer and transform DuckLake paths are not yet complete enough for version comparison as a sole source of truth.

## User Clarification: Component Change Can Affect BRAN

The `VALV` valve is only a validation sample, not the whole design target.

The version model must support impact propagation:

- A local component modification can make its containing delivery unit dirty.
- For example, if a `VALV` under a `BRAN` changes geometry, transform, ownership, or relevant attributes, the `BRAN` version should be reported as impacted.
- If the component moves between BRANs, both the old BRAN and new BRAN are impacted with different impact kinds.
- Therefore the MVP must return old/new delivery-unit membership and impacted units, not only component-level changed fields.

## Key Design Decisions

- MVP starts after Parquet export, not inside geometry generation.
- Diff granularity is component refno, with delivery unit as grouping/index granularity.
- Component diff must feed delivery-unit impact analysis. Unit diff is not a separate afterthought.
- Release identity is explicit application metadata, not only a DuckLake snapshot id.
- Hashes are used for fast equality, but diff responses must include numeric/details deltas for user comprehension.
- Missing/unassigned delivery membership is reported, never silently filtered.

## Validation Constraints

- Do not create or run cargo tests.
- For `web_server`, run the service and verify with HTTP/POST.
- For aios-database behavior, use CLI + JSON checks.
