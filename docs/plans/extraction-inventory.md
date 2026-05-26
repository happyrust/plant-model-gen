# `plant-model-core` 抽离清单（Phase 0 输出）

> 创建日期：2026-05-13
> 关联计划：[`2026-05-12-plant-model-core-extraction-plan.md`](./2026-05-12-plant-model-core-extraction-plan.md)、[`.plannotator/plan-plant-model-core-extraction-development.md`](../../.plannotator/plan-plant-model-core-extraction-development.md)（已 plannotator approved）
> 适用 phase：Phase 0 - 基线冻结与抽离清单
> 后续动作：Phase 1 创建新仓后，本文件复制为 `plant-model-core/docs/extraction-inventory.md`

---

## 1. 主仓 baseline 快照

| 项 | 值 |
|---|---|
| 仓库 | `D:\work\plant-code\plant-model-gen` |
| crate name | `aios-database` |
| lib name | `aios_database` |
| package version | `0.3.23` |
| edition | `2024` |
| baseline branch | `main` |
| baseline commit | `42e954f63f37bea643d5b1b15199d7ebe9acd759` |
| baseline message | `fix(review): filter task list by form id` |
| baseline date | `2026-05-13 15:35:59 +0800` |
| baseline 状态 | 工作区有未提交改动（多为 `.cursor/` 工具配置、`.factory/` 文档与 `web_api/review_api.rs` 的合并冲突标记，与本期抽离无关） |

### 主仓 Cargo features（关键能力相关）

```text
default = ["review"]
review = ["ws","gen_model","manifold","project_hd","surreal-save","write-to-surrealdb","sqlite-index","web_server"]
review-rocksdb = ["review","kv-rocksdb"]
parquet-export = [parquet, arrow-array, arrow-schema, polars]
transform-store-parquet = ["parquet-export"]
transform-store-ducklake = ["transform-store-parquet"]
mbd-pipe = ["parquet-export","mbd-iso"]
excel-export = ["dep:rust_xlsxwriter"]
spec-loader = ["dep:calamine","aios_core/spec-loader"]
gen_model = ["aios_core/gen_model"]
manifold = ["aios_core/manifold"]
mem-kv-save = ["aios_core/mem-kv-save"]
kv-rocksdb = ["aios_core/kv-rocksdb","surrealdb/kv-rocksdb"]
sqlite-index = ["dep:rusqlite","dep:tokio-util","aios_core/sqlite"]
rvm-import = ["dep:rvm-rs","sqlite-index"]
convex-runtime = []
convex-decomposition = ["convex-runtime","dep:miniacd","dep:glamx"]
surreal-save = []
write-to-surrealdb = []
model-writer-drain = []
debug_obj_export = []
debug_expr = ["aios_core/debug_expr"]
debug_e3d = []
profile = ["dep:tracing-chrome"]
full = ["review","kv-rocksdb","parquet-export","mbd-pipe","excel-export","spec-loader","mqtt"]
```

### 主仓 `[patch]` 配置（本期需要在新仓 1:1 复用）

```toml
[patch."https://github.com/happyrust/rs-core.git"]
aios_core = { path = "../rs-core" }

[patch."https://github.com/happyrust/pdms-io.git"]
pdms_io = { path = "../pdms-io-fork" }
parse_pdms_db = { path = "../pdms-io-fork/crates/parse_pdms_db" }
```

### 关键 git 依赖（新仓沿用同分支）

| crate | git | branch |
|---|---|---|
| `aios_core` | `https://github.com/happyrust/rs-core.git` | `dev-3.1` |
| `surrealdb` / `surrealdb-types` | `https://github.com/happyrust/surrealdb` | `dev-3.1` |
| `pdms_io` / `parse_pdms_db` | `https://github.com/happyrust/pdms-io.git` | `dev-3.1` |
| `indextree` | `https://github.com/happyrust/indextree` | `main` |
| `miniacd` | `https://github.com/happyrust/miniacd-fork.git` | `main` |
| `rvm-rs` | `https://github.com/happyrust/rvm-rs` | 默认 |

---

## 2. 抽离范围（共 99 个 .rs 文件，复制 97 / 排除 2）

### 2.1 复制范围（共 97 个 .rs，主仓零改动）

> 路径以 `src/fast_model/` 为前缀；右列为新仓相对 `src/` 的目标路径。

#### `gen_model/`（46 个，全部复制）

| 主仓路径 | 新仓路径 |
|---|---|
| `gen_model/boolean_backfill.rs` | `gen_model/boolean_backfill.rs` |
| `gen_model/boolean_task.rs` | `gen_model/boolean_task.rs` |
| `gen_model/cache_miss_report.rs` | `gen_model/cache_miss_report.rs` |
| `gen_model/cata_model.rs` | `gen_model/cata_model.rs` |
| `gen_model/cata_resolve_cache_pipeline.rs` | `gen_model/cata_resolve_cache_pipeline.rs` |
| `gen_model/cate_helpers.rs` | `gen_model/cate_helpers.rs` |
| `gen_model/cate_processor.rs` | `gen_model/cate_processor.rs` |
| `gen_model/cate_single.rs` | `gen_model/cate_single.rs` |
| `gen_model/categorized_refnos.rs` | `gen_model/categorized_refnos.rs` |
| `gen_model/config.rs` | `gen_model/config.rs` |
| `gen_model/context.rs` | `gen_model/context.rs` |
| `gen_model/db_meta_cache.rs` | `gen_model/db_meta_cache.rs` |
| `gen_model/errors.rs` | `gen_model/errors.rs` |
| `gen_model/index_tree_mode.rs` | `gen_model/index_tree_mode.rs` |
| `gen_model/inst_query.rs` | `gen_model/inst_query.rs` |
| `gen_model/loop_model.rs` | `gen_model/loop_model.rs` |
| `gen_model/loop_processor.rs` | `gen_model/loop_processor.rs` |
| `gen_model/manifold_bool.rs` | `gen_model/manifold_bool.rs` |
| `gen_model/mesh_generate.rs` | `gen_model/mesh_generate.rs` |
| `gen_model/mesh_processing.rs` | `gen_model/mesh_processing.rs` |
| `gen_model/mesh_state.rs` | `gen_model/mesh_state.rs` |
| `gen_model/mod.rs` | `gen_model/mod.rs` |
| `gen_model/model_writer.rs` | `gen_model/model_writer.rs` |
| `gen_model/models.rs` | `gen_model/models.rs` |
| `gen_model/neg_query.rs` | `gen_model/neg_query.rs` |
| `gen_model/noun_collection.rs` | `gen_model/noun_collection.rs` |
| `gen_model/orchestrator.rs` | `gen_model/orchestrator.rs` |
| `gen_model/pdms_inst.rs` | `gen_model/pdms_inst.rs` |
| `gen_model/pdms_inst_surreal.rs` | `gen_model/pdms_inst_surreal.rs` |
| `gen_model/pdms_inst_v2.rs` | `gen_model/pdms_inst_v2.rs` |
| `gen_model/pdms_inst_v3.rs` | `gen_model/pdms_inst_v3.rs` |
| `gen_model/precheck_coordinator.rs` | `gen_model/precheck_coordinator.rs` |
| `gen_model/prim_model.rs` | `gen_model/prim_model.rs` |
| `gen_model/prim_processor.rs` | `gen_model/prim_processor.rs` |
| `gen_model/processor.rs` | `gen_model/processor.rs` |
| `gen_model/query.rs` | `gen_model/query.rs` |
| `gen_model/query_compat.rs` | `gen_model/query_compat.rs` |
| `gen_model/query_provider.rs` | `gen_model/query_provider.rs` |
| `gen_model/refno_assoc_index.rs` | `gen_model/refno_assoc_index.rs` |
| `gen_model/resolve.rs` | `gen_model/resolve.rs` |
| `gen_model/sql_file_writer.rs` | `gen_model/sql_file_writer.rs` |
| `gen_model/transform_cache.rs` | `gen_model/transform_cache.rs` |
| `gen_model/transform_rkyv_cache.rs` | `gen_model/transform_rkyv_cache.rs` |
| `gen_model/tree_index_manager.rs` | `gen_model/tree_index_manager.rs` |
| `gen_model/utilities.rs` | `gen_model/utilities.rs` |

#### `cal_model/`（3 个，全部复制）

| 主仓路径 | 新仓路径 |
|---|---|
| `cal_model/bran_model.rs` | `cal_model/bran_model.rs` |
| `cal_model/equip_model.rs` | `cal_model/equip_model.rs` |
| `cal_model/mod.rs` | `cal_model/mod.rs` |

#### `export_model/`（22 个，全部复制）

| 主仓路径 | 新仓路径 |
|---|---|
| `export_model/export_common.rs` | `export_model/export_common.rs` |
| `export_model/export_dbnum_instances_parquet.rs` | `export_model/export_dbnum_instances_parquet.rs` |
| `export_model/export_dbnum_instances_v3.rs` | `export_model/export_dbnum_instances_v3.rs` |
| `export_model/export_dbnum_instances_web.rs` | `export_model/export_dbnum_instances_web.rs` |
| `export_model/export_glb.rs` | `export_model/export_glb.rs` |
| `export_model/export_gltf.rs` | `export_model/export_gltf.rs` |
| `export_model/export_instanced_bundle.rs` | `export_model/export_instanced_bundle.rs` |
| `export_model/export_obj.rs` | `export_model/export_obj.rs` |
| `export_model/export_pdms_tree_parquet.rs` | `export_model/export_pdms_tree_parquet.rs` |
| `export_model/export_prepack_lod.rs` | `export_model/export_prepack_lod.rs` |
| `export_model/export_room_instances.rs` | `export_model/export_room_instances.rs` |
| `export_model/export_rvm_semantic_debug.rs` | `export_model/export_rvm_semantic_debug.rs` |
| `export_model/export_transform_config.rs` | `export_model/export_transform_config.rs` |
| `export_model/export_unit_mesh_glb.rs` | `export_model/export_unit_mesh_glb.rs` |
| `export_model/export_xkt.rs` | `export_model/export_xkt.rs` |
| `export_model/import_glb.rs` | `export_model/import_glb.rs` |
| `export_model/mod.rs` | `export_model/mod.rs` |
| `export_model/model_exporter.rs` | `export_model/model_exporter.rs` |
| `export_model/name_config.rs` | `export_model/name_config.rs` |
| `export_model/parquet_stream_writer.rs` | `export_model/parquet_stream_writer.rs` |
| `export_model/parquet_writer.rs` | `export_model/parquet_writer.rs` |
| `export_model/pe_parquet_writer.rs` | `export_model/pe_parquet_writer.rs` |
| `export_model/simple_color_palette.rs` | `export_model/simple_color_palette.rs` |
| `export_model/spec_info.rs` | `export_model/spec_info.rs` |

> 注：`export_model/mod.rs` 复制后需删除对 `room_instances` 等不抽离模块的 re-export 行（若有），与 cal_model 类似，详见 Phase 2。

#### 缓存模块（`cache/` 命名空间，新仓内归类）

| 主仓路径 | 新仓路径 |
|---|---|
| `fast_model/foyer_cache/cata_resolve_cache.rs` | `cache/foyer_cache/cata_resolve_cache.rs` |
| `fast_model/foyer_cache/mod.rs` | `cache/foyer_cache/mod.rs` |
| `fast_model/instance_cache.rs` | `cache/instance_cache.rs` |
| `fast_model/model_cache/cata_resolve_cache.rs` | `cache/model_cache/cata_resolve_cache.rs` |
| `fast_model/model_cache/geom_input_cache.rs` | `cache/model_cache/geom_input_cache.rs` |
| `fast_model/model_cache/mesh.rs` | `cache/model_cache/mesh.rs` |
| `fast_model/model_cache/mod.rs` | `cache/model_cache/mod.rs` |
| `fast_model/model_cache/query.rs` | `cache/model_cache/query.rs` |
| `fast_model/model_store.rs` | `cache/model_store.rs` |
| `fast_model/cache_flush.rs` | `cache/cache_flush.rs` |
| `fast_model/cata_cache_gen.rs` | `cache/cata_cache_gen.rs` |

#### 错误与宏（`errors/`）

| 主仓路径 | 新仓路径 |
|---|---|
| `fast_model/error_macros.rs` | `errors/error_macros.rs` |
| `fast_model/refno_errors.rs` | `errors/refno_errors.rs` |

#### 支撑模块（`support/`）

| 主仓路径 | 新仓路径 |
|---|---|
| `fast_model/aabb_tree.rs` | `support/aabb_tree.rs` |
| `fast_model/concurrency.rs` | `support/concurrency.rs` |
| `fast_model/convex_decomp.rs` | `support/convex_decomp.rs` |
| `fast_model/material_config.rs` | `support/material_config.rs` |
| `fast_model/precheck.rs` | `support/precheck.rs` |
| `fast_model/reuse_unit.rs` | `support/reuse_unit.rs` |
| `fast_model/rs_transform_ext.rs` | `support/rs_transform_ext.rs` |
| `fast_model/unit_converter.rs` | `support/unit_converter.rs` |

#### 顶层文件（lib 根）

| 主仓路径 | 新仓路径 |
|---|---|
| `fast_model/incremental/mod.rs` | `incremental/mod.rs` |
| `fast_model/mod.rs` | `lib.rs`（合并到新 `lib.rs`，剥离 `pub use crate::scene_tree;`） |
| `fast_model/session.rs` | `session.rs` |
| `fast_model/shared.rs` | `shared.rs` |
| `fast_model/utils.rs` | `utils.rs` |

### 2.2 排除范围（本期不抽离，主仓零改动保持原样）

| 主仓路径 | 排除理由 |
|---|---|
| `fast_model/room_model.rs` | room 域，与 gen/export 解耦，下期单独评估 |
| `fast_model/room_worker.rs` | 同上 |
| `scene_tree/*` | 主仓服务层，通过 `SceneTreeProvider` hook 注入 |
| `sqlite_index.rs` / 模块 | 通过 `SpatialIndexHook` hook 注入 |
| `spatial_index.rs` / 模块 | 同上 |
| `pe_transform_refresh.rs` | 通过 `TransformStore` hook 注入 |
| `pe_transform_store.rs` | 同上 |
| `versioned_db/` | 通过 hook / 常量复制 |
| `data_interface/` | 通过 `DbMetaProvider` + `PdmsDataInterface` hook 注入 |
| `options/`（含 `DbOptionExt`） | 通过 `DbOptionLike` hook 注入 |
| `web_server/` / `web_api/` | 服务层，禁止反向依赖 |
| `rvm_import/` | 不在抽离范围 |
| `rvm_obj_export.rs` | 同上 |

---

## 3. 跨边界引用分级表（move / core / hook / forbid）

按 `crate::<path>` 的目标分级。`move` = 内部模块复制；`core` = 已在 `aios_core`；`hook` = 通过 trait 注入；`forbid` = 不得依赖。

### 3.1 `hook`（需在 `api/hooks.rs` 抽 trait，由主仓/standalone 注入）

| 主仓引用 | 出现处（节选） | 抽出 hook | 第一版策略 |
|---|---|---|---|
| `crate::data_interface::db_meta_manager::db_meta`<br>`crate::data_interface::db_meta` | `gen_model/{transform_cache, cata_model, query_provider, neg_query, transform_rkyv_cache, utilities, db_meta_cache, precheck_coordinator, orchestrator, mesh_generate, tree_index_manager, index_tree_mode}.rs`<br>`export_model/{export_dbnum_instances_v3, export_prepack_lod, parquet_stream_writer, model_exporter}.rs`<br>`support/precheck.rs`<br>`cache/cata_cache_gen.rs` | `DbMetaProvider` | 暴露 `get_dbnum_by_ref0`、`get_dbnum_by_refno`、`get_all_dbnums`、`get_db_file_info`、`get_ref0s_by_dbnum`、`ref0s_to_dbnums`、`is_loaded`、`ensure_loaded` 的子集 |
| `crate::data_interface::interface::PdmsDataInterface` | `gen_model/{cata_model, loop_model}.rs` | `PdmsDataInterface`（已是 trait，可重新导出或镜像声明） | 镜像声明为新仓内 trait，由 standalone 注入；签名锁定为 baseline commit 的形态 |
| `crate::data_interface::tidb_manager::AiosDBManager` | `gen_model/{pdms_inst, loop_model}.rs` | `AiosDbHandle` | trait 化或包装为 `Arc<dyn>`，新仓 standalone 用 mock |
| `crate::data_interface::sesno_increment::get_changes_at_sesno`<br>`crate::data_interface::increment_record::IncrGeoUpdateLog` | `gen_model/orchestrator.rs` | `IncrementSource`（hook） | 首版 no-op（返回空增量），standalone 不验证增量路径 |
| `crate::data_interface::structs::PlantAxisMap` | `gen_model/{cata_model, cate_single}.rs` | move | 类型 = `BTreeMap<i32, aios_core::parsed_data::CateAxisParam>`；在新仓 `support/types.rs` 复制别名 |
| `crate::data_interface::db_model::TUBI_TOL` | `export_model/`（间接）| move | `const TUBI_TOL: f32 = 1.0;`，复制到 `support/types.rs` 或 `support/constants.rs` |
| `crate::options::DbOptionExt` | 19+ 文件 | `DbOptionLike` | 抽出 gen/export 实际用到的 getter 子集（meshes_path、output_dir、surreal endpoint、capture flags、project_hd 等），用 `Arc<dyn DbOptionLike>` 取代直接引用 |
| `crate::pe_transform_store::load_entries_with_backend` 等 | `gen_model/transform_cache.rs` | `TransformStore` | 首版保留 SurrealDB + rkyv 后端；Parquet/Ducklake 后端作为 feature 后置 |
| `crate::pe_transform_refresh::refresh_pe_transform_for_dbnums_compat` | `support/precheck.rs` | `TransformRefresh` | 首版 no-op，可在配置中关闭刷新 |
| `crate::sqlite_index::{ImportConfig, SqliteAabbIndex}` | `gen_model/orchestrator.rs`（cfg(test) 路径） | `SpatialIndexHook` | 首版 no-op + warning |
| `crate::spatial_index::SqliteSpatialIndex` | `gen_model/orchestrator.rs`（cfg(test) 路径） | 同上 | 同上 |
| `crate::scene_tree::query_generated_refnos` | `gen_model/mesh_generate.rs` | `SceneTreeProvider`（含 `query_generated_refnos`） | 首版 standalone 用 mock，返回空集（表示全部待生成） |
| `crate::scene_tree`（reexport） | `fast_model/mod.rs:86` | 删除 reexport | 新仓 lib.rs 不写 `pub use scene_tree;` |
| `crate::versioned_db::database::sync_pdms` | `gen_model/query_provider.rs`（cfg(test) 路径） | `PdmsSyncHook` | 首版 no-op |
| `crate::versioned_db::db_meta_info::DEFAULT_TREE_DIR` | `gen_model/tree_index_manager.rs` | move | `pub const DEFAULT_TREE_DIR: &str = "output/scene_tree";`，复制到 `support/constants.rs` 或 `gen_model/tree_index_manager.rs` 顶部 |

### 3.2 `move`（内部模块或常量，直接搬运）

| 主仓引用前缀 | 处理 |
|---|---|
| `crate::fast_model::gen_model::*` | → `crate::gen_model::*` |
| `crate::fast_model::cal_model::*` | → `crate::cal_model::*` |
| `crate::fast_model::export_model::*` | → `crate::export_model::*` |
| `crate::fast_model::model_cache::*` | → `crate::cache::model_cache::*` |
| `crate::fast_model::foyer_cache::*` | → `crate::cache::foyer_cache::*` |
| `crate::fast_model::instance_cache::*` | → `crate::cache::instance_cache::*` |
| `crate::fast_model::model_store::*` | → `crate::cache::model_store::*` |
| `crate::fast_model::cache_flush::*` | → `crate::cache::cache_flush::*` |
| `crate::fast_model::cata_cache_gen::*` | → `crate::cache::cata_cache_gen::*` |
| `crate::fast_model::error_macros::*` | → `crate::errors::error_macros::*` |
| `crate::fast_model::refno_errors::*` | → `crate::errors::refno_errors::*` |
| `crate::fast_model::aabb_tree`、`concurrency`、`convex_decomp`、`material_config`、`precheck`、`reuse_unit`、`rs_transform_ext`、`unit_converter` | → `crate::support::*` |
| `crate::fast_model::shared` / `utils` / `session` / `incremental` | → `crate::shared` / `crate::utils` / `crate::session` / `crate::incremental` |
| `crate::fast_model::mod.rs` 中的全局 `EXIST_MESH_GEO_HASHES`、AABB rkyv 缓存、`CAPTURE_CONFIG`、`REFNO_ERROR_STORE`、smart debug 宏 | 集中到 `support/aabb_cache.rs` + `support/capture_config.rs` + `errors/refno_errors.rs`；从新仓 `lib.rs` 顶层 re-export |

### 3.3 `core`（继续使用 `aios_core`，不抽离）

| 引用 | 模块 |
|---|---|
| `aios_core::RefU64`、`RefnoEnum` | core 基础类型 |
| `aios_core::parsed_data::{CateAxisParam, CateGeomsInfo}` | PDMS 解析数据结构 |
| `aios_core::material::*`、`aios_core::mbd::*`、`aios_core::ssc_setting::*`（受 features 控制） | 已存在 |
| `aios_core::*`（其他） | 直接通过新仓 `Cargo.toml` 引入相同 git + path patch |

### 3.4 `forbid`（新仓不得依赖）

| 主仓引用 | 当前 fast_model 中是否出现 | 措施 |
|---|---|---|
| `crate::web_server::*` | **未出现** ✓ | 新仓 grep 守护：CI/手动检查不得新增 |
| `crate::web_api::*` | **未出现** ✓ | 同上 |
| `crate::rvm_import::*` | **未出现** ✓ | 同上 |
| `crate::rvm_obj_export::*` | **未出现** ✓ | 同上 |

---

## 4. 全局静态与宏迁移表

| 主仓位置 | 主仓符号 | 新仓位置 | 备注 |
|---|---|---|---|
| `fast_model/mod.rs` | `EXIST_MESH_GEO_HASHES`（OnceCell） | `support/aabb_cache.rs` | rkyv 序列化保留独立副本 |
| `fast_model/mod.rs` | `AabbCacheFileV1`（rkyv struct） | `support/aabb_cache.rs` | 复制 |
| `fast_model/mod.rs` | `GLOBAL_TRANSFORM_CACHE`（OnceLock）| `gen_model/transform_cache.rs` | 复制；读取后端通过 `TransformStore` 注入 |
| `fast_model/mod.rs` | `CAPTURE_CONFIG`、`set_capture_config`、`get_capture_config` | `support/capture_config.rs` | 复制 |
| `fast_model/refno_errors.rs` | `REFNO_ERROR_STORE`、`DEBUG_MODEL_ERRORS_ONLY` 等 | `errors/refno_errors.rs` | 复制 |
| `fast_model/error_macros.rs` | `smart_debug_model!`、`smart_debug_error!` 等宏 | `errors/error_macros.rs` | 复制；宏内部 `$crate::` 路径自动指向新 crate |

> **关键约束**：本期新旧仓**分进程运行**，全局静态各有独立副本，不共享内存；A/B 对比仅比产物（GLB hash / Parquet 行数 / Transform delta）。

---

## 5. Phase 2 路径替换规则（机械化）

> 在新仓内统一执行，主仓不改。建议批量正则 + 人工 review。

| 原模式 | 替换 |
|---|---|
| `crate::fast_model::gen_model::` | `crate::gen_model::` |
| `crate::fast_model::cal_model::` | `crate::cal_model::` |
| `crate::fast_model::export_model::` | `crate::export_model::` |
| `crate::fast_model::model_cache::` | `crate::cache::model_cache::` |
| `crate::fast_model::foyer_cache::` | `crate::cache::foyer_cache::` |
| `crate::fast_model::instance_cache` | `crate::cache::instance_cache` |
| `crate::fast_model::model_store` | `crate::cache::model_store` |
| `crate::fast_model::cache_flush` | `crate::cache::cache_flush` |
| `crate::fast_model::cata_cache_gen` | `crate::cache::cata_cache_gen` |
| `crate::fast_model::error_macros` | `crate::errors::error_macros` |
| `crate::fast_model::refno_errors` | `crate::errors::refno_errors` |
| `crate::fast_model::aabb_tree` | `crate::support::aabb_tree` |
| `crate::fast_model::concurrency` | `crate::support::concurrency` |
| `crate::fast_model::convex_decomp` | `crate::support::convex_decomp` |
| `crate::fast_model::material_config` | `crate::support::material_config` |
| `crate::fast_model::precheck` | `crate::support::precheck` |
| `crate::fast_model::reuse_unit` | `crate::support::reuse_unit` |
| `crate::fast_model::rs_transform_ext` | `crate::support::rs_transform_ext` |
| `crate::fast_model::unit_converter` | `crate::support::unit_converter` |
| `crate::fast_model::shared` | `crate::shared` |
| `crate::fast_model::utils` | `crate::utils` |
| `crate::fast_model::session` | `crate::session` |
| `crate::fast_model::incremental` | `crate::incremental` |
| `crate::fast_model::` | （兜底）`crate::` |
| `crate::scene_tree::` | `// TODO(hook): SceneTreeProvider` |
| `crate::sqlite_index::` | `// TODO(hook): SpatialIndexHook` |
| `crate::spatial_index::` | `// TODO(hook): SpatialIndexHook` |
| `crate::data_interface::db_meta` 系列 | `// TODO(hook): DbMetaProvider` |
| `crate::data_interface::interface::PdmsDataInterface` | 镜像 trait `crate::api::hooks::PdmsDataInterface` |
| `crate::data_interface::structs::PlantAxisMap` | `crate::support::types::PlantAxisMap` |
| `crate::data_interface::db_model::TUBI_TOL` | `crate::support::constants::TUBI_TOL` |
| `crate::options::DbOptionExt` | `crate::api::hooks::DbOptionLike` |
| `crate::pe_transform_store::` | `crate::api::hooks::TransformStore` |
| `crate::pe_transform_refresh::` | `crate::api::hooks::TransformRefresh` |
| `crate::versioned_db::database::sync_pdms` | `crate::api::hooks::PdmsSyncHook::sync` |
| `crate::versioned_db::db_meta_info::DEFAULT_TREE_DIR` | `crate::support::constants::DEFAULT_TREE_DIR` |

---

## 6. Phase 1 新仓骨架核对清单

完成 Phase 1 后应满足：

- [ ] `D:\work\plant-code\plant-model-core` Git 仓存在，初始 commit
- [ ] `Cargo.toml` crate name = `plant_model_core`，edition = `2024`
- [ ] 依赖子集对齐第 1 节中"git 依赖"和"Cargo features"
- [ ] `[patch]` 与主仓一致（指向 `../rs-core` / `../pdms-io-fork`）
- [ ] `src/lib.rs` 声明 `pub mod api; mod gen_model; mod cal_model; mod export_model; mod cache; mod support; mod errors; mod shared; mod utils; mod session; mod incremental;`（首版以编译通过为准，public 表面在 Phase 3 收口）
- [ ] `src/api/mod.rs` 至少有 `pub mod traits; pub mod hooks; pub mod context; pub mod types; pub mod default_impl; pub mod mock_hooks;` 的占位
- [ ] `.gitignore`、`AGENTS.md`、`README.md`、`CHANGELOG.md`、`docs/extraction-inventory.md`（本文件复制版）
- [ ] `cargo metadata` 通过；`cargo check --lib --features gen_model,manifold` 仅缺业务模块（不缺依赖）

---

## 7. 本期不做事项（明示）

- 不修改任何 `plant-model-gen/src/` 文件
- 不在 `plant-model-gen` 的 `Cargo.toml` 中加入 `plant_model_core` 依赖
- 不运行 `cargo test`
- 不抽离 `room_model` / `room_worker` / `scene_tree` / `sqlite_index` / `spatial_index` / `pe_transform_*` / `versioned_db` / `web_server` / `web_api` / `rvm_import`
- 不在新仓引入 axum / tower / rusqlite 等服务层依赖
- 不在本期切换主仓到新仓依赖（下期）
- 不在本期删除主仓 `fast_model` 代码（下期）

---

## 8. 验收

Phase 0 完成的标志：

- [x] 抽离清单（97 复制 + 2 排除）已列出，路径映射完整
- [x] 跨边界引用按 move / core / hook / forbid 分级完成
- [x] 全局静态迁移表已确定
- [x] 路径替换规则机械化可执行
- [x] baseline commit / branch / features / patch 已快照
- [x] 主仓零改动（除 `.gitignore` 已有 `.plannotator/` 和本文件之外的所有源码均未触动）

Phase 0 → Phase 1 触发条件：用户确认本清单。
