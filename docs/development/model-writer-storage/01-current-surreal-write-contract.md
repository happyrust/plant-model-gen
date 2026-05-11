# Current SurrealDB Write Contract

## Current behavior

`ModelWriterBackend` currently selects `SurrealModelWriterBackend` for normal generation. That backend delegates to existing generation helpers for cleanup, base instance writes, mesh result persistence, AABB relation writes, negative relation reconciliation, and boolean bridge execution.

SurrealDB remains the source-of-truth contract for Phase 1. New storage backends must match its observable rows and references for raw model data before adding backend-specific optimizations.

## Contract principles

- Record identity must remain stable across backends.
- Relation direction must preserve SurrealDB meaning: `in` is the source record and `out` is the target record.
- Shared payload tables (`aabb`, `trans`, `vec3`) are canonical value tables referenced by relation/projection tables.
- Boolean result tables are excluded from Phase 1 and handled in Phase 2.
- Validation is CLI + SQL only.

## Parity matrix

| Surreal object | Role | Canonical raw table | Canonical projection | Phase |
|---|---|---:|---:|---|
| `inst_info` | Instance payload keyed by catalog/hash identity | `raw_inst_info` | `projection_instances` | Phase 1 |
| `inst_relate` | `pe` to `inst_info` relation; primary refno-to-instance bridge | `raw_inst_relate` | `projection_instances` | Phase 1 |
| `inst_geo` | Geometry payload keyed by geometry hash/id, including mesh status and payload references | `raw_inst_geo` | `projection_geometry` | Phase 1 |
| `geo_relate` | `inst_info` to `inst_geo` relation with geometry refno, type, transform, visibility | `raw_geo_relate` | `projection_instance_geometry` | Phase 1 |
| `tubi_info` | Tubing segment payload | `raw_tubi_info` | `projection_tubing` | Phase 1 |
| `tubi_relate` | Instance/catalog to tubing payload relation | `raw_tubi_relate` | `projection_tubing` | Phase 1 |
| `neg_relate` | Negative geometry dependency relation | `raw_neg_relate` | `projection_dependencies` | Phase 1 |
| `ngmr_relate` | Non-geometry/negative mesh dependency relation | `raw_ngmr_relate` | `projection_dependencies` | Phase 1 |
| `aabb` | Shared AABB value table | `raw_aabb` | `projection_bounds` | Phase 1 |
| `trans` | Shared transform value table | `raw_trans` | `projection_transforms` | Phase 1 |
| `vec3` | Shared point/vector payload table | `raw_vec3` | `projection_mesh_payloads` | Phase 1 |
| `inst_relate_aabb` | Raw instance-to-AABB relation for original geometry bounds | `raw_inst_relate_aabb` | `projection_bounds` | Phase 1 |
| `refno_assoc_index` | Regeneration delete/index acceleration metadata | `raw_refno_assoc_index` | `projection_regen_index` | Phase 1 |
| `inst_relate_bool` | Instance-level boolean result status and mesh pointer | `raw_inst_relate_bool` | `projection_boolean_results` | Phase 2 |
| `inst_relate_cata_bool` | Catalog-level boolean result relation | `raw_inst_relate_cata_bool` | `projection_boolean_results` | Phase 2 |

## Backend parity requirements

1. A Phase 1 backend is complete only when every Phase 1 row class above is either written directly or intentionally represented by an equivalent canonical projection; `inst_relate_bool` and `inst_relate_cata_bool` remain Phase 2 and are not required for acceptance.
2. `inst_relate_bool` and `inst_relate_cata_bool` must not gate Phase 1 acceptance.
3. Backend validation compares SurrealDB and candidate backend output through CLI-generated data and SQL queries, not Rust tests.
