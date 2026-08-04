# Canonical Schema

## Schema layers

The storage architecture uses two schema layers:

- Raw tables: loss-minimized records corresponding to current SurrealDB objects.
- Projection tables: query-oriented tables for validation, export, and future runtime reads.

The DuckLake namespace for the canonical schema is `ducklake-canonical`.

## Raw schema tables

| Table | Key columns | Required payload | Notes |
|---|---|---|---|
| `raw_inst_info` | `inst_id`, `dbnum` | serialized instance info, catalog hash, refno metadata | Mirrors `inst_info`. |
| `raw_inst_relate` | `refno`, `dbnum` | `pe_id`, `inst_id`, relation id | Mirrors `inst_relate`. |
| `raw_inst_geo` | `geo_hash` | geometry params, mesh flags, AABB id, point/vector refs | Mirrors `inst_geo`. |
| `raw_geo_relate` | `inst_id`, `geo_hash`, `geom_refno` | `geo_type`, `trans_id`, `visible`, relation id | Mirrors `geo_relate`. |
| `raw_tubi_info` | `tubi_id` | tubing segment payload | Mirrors `tubi_info`. |
| `raw_tubi_relate` | `inst_id`, `tubi_id` | relation id, source refno/dbnum | Mirrors `tubi_relate`. |
| `raw_neg_relate` | `carrier_refno`, `target_refno` | relation id, dependency type | Mirrors `neg_relate`. |
| `raw_ngmr_relate` | `carrier_refno`, `target_refno` | relation id, dependency type | Mirrors `ngmr_relate`. |
| `raw_aabb` | `aabb_id` | min/max bounds payload | Mirrors `aabb`. |
| `raw_trans` | `trans_id` | transform matrix payload | Mirrors `trans`. |
| `raw_vec3` | `vec3_id` | point/vector array payload | Mirrors `vec3`. |
| `raw_inst_relate_aabb` | `refno` | `aabb_id`, source marker | Mirrors `inst_relate_aabb`. |
| `raw_refno_assoc_index` | `refno` | associated record ids grouped by table | Mirrors `refno_assoc_index`; retained for delete parity even where runtime use is disabled. |

## Projection schema tables

| Table | Grain | Source raw tables | Purpose |
|---|---|---|---|
| `projection_instances` | one row per `dbnum/refno` | `raw_inst_relate`, `raw_inst_info` | Fast instance existence and refno parity checks. |
| `projection_geometry` | one row per `geo_hash` | `raw_inst_geo`, `raw_aabb`, `raw_vec3` | Geometry payload and mesh status checks. |
| `projection_instance_geometry` | one row per instance-geometry edge | `raw_geo_relate`, `raw_inst_relate`, `raw_inst_geo`, `raw_trans` | Validate `geo_relate` cardinality and transform linkage. |
| `projection_tubing` | one row per tubing segment relation | `raw_tubi_relate`, `raw_tubi_info` | Tubing parity and export reads. |
| `projection_dependencies` | one row per dependency edge | `raw_neg_relate`, `raw_ngmr_relate` | Negative/dependency reconciliation checks. |
| `projection_bounds` | one row per refno bounds result | `raw_inst_relate_aabb`, `raw_aabb` | Room/spatial validation without Surreal-specific record ids. |
| `projection_transforms` | one row per transform id | `raw_trans` | Transform de-duplication and SQL comparison. |
| `projection_mesh_payloads` | one row per vector payload id | `raw_vec3` | Mesh payload availability checks. |
| `projection_regen_index` | one row per indexed refno | `raw_refno_assoc_index` | Cleanup/delete parity checks. |

## Phase 2 schema placeholder

Boolean outputs are modeled later as Phase 2 tables:

- `raw_inst_relate_bool`
- `raw_inst_relate_cata_bool`
- `projection_boolean_results`

They must not be required for Phase 1 acceptance.
