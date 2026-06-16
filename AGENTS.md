---
description: 
alwaysApply: true
---

# Repository Guidelines
 不要创建或运行基于 `cargo test` 的测试用例；不要使用 test 或者编译任何 test。任何时候，针对 web_server 都要运行服务后通过 HTTP/POST 去测试，而不是使用 test。
 针对 aios-database，使用 cli + json的方式去测试验证。

## 部署目标服务器 
- 服务器：`123.57.182.243`
- SSH 用户：`root`
- SSH 密码：仅通过环境变量 / CI Secrets 提供（禁止写入仓库）


## Tools

<!-- sigmap-tools -->

```json
[
  {
    "name": "sigmap_ask",
    "description": "Rank source files by relevance to a natural-language query. Run before exploring the codebase.",
    "command": "sigmap ask \"$QUERY\""
  },
  {
    "name": "sigmap_validate",
    "description": "Validate SigMap config and measure context coverage. Run after changing config or source dirs.",
    "command": "sigmap validate"
  },
  {
    "name": "sigmap_judge",
    "description": "Score an LLM response for groundedness against source context. Use to verify answer quality.",
    "command": "sigmap judge --response \"$RESPONSE\" --context \"$CONTEXT\""
  },
  {
    "name": "sigmap_query",
    "description": "Rank all files by relevance using TF-IDF and write a focused mini-context.",
    "command": "sigmap --query \"$QUERY\" --context"
  },
  {
    "name": "sigmap_weights",
    "description": "Show learned file-ranking multipliers accumulated from past sessions.",
    "command": "sigmap weights"
  }
]
```

## Auto-generated signatures
<!-- Updated by gen-context.js -->
# Code signatures

## SigMap commands

| When | Command |
|------|---------|
| Before answering a question | `sigmap ask "<your question>"` |
| After code changes | `sigmap validate` |
| To query by topic | `sigmap --query "<topic>"` |

Always run `sigmap ask` or `sigmap --query` before searching for files relevant to a task.
## todos
```
src\fast_model\gen_model\query.rs:61  # TODO: visible 不应该在这里执行过滤
src\fast_model\gen_model\query.rs:67  # TODO: add other types
src\web_server\task_creation_handlers.rs:227  # TODO: 集成真实的任务管理器
src\web_server\task_creation_handlers.rs:271  # TODO: 从数据库加载部署站点
src\web_server\task_creation_handlers.rs:366  # TODO: 检查数据库中是否存在同名任务
src\web_server\task_creation_handlers.rs:378  # TODO: 根据任务类型和参数计算资源需求
src\web_server\task_creation_handlers.rs:412  # TODO: 实现数据库保存逻辑
src\web_server\task_creation_handlers.rs:471  # TODO: 更新任务状态为完成，并记录输出文件路径
src\web_server\task_creation_handlers.rs:506  # TODO: 应该从数据库中获取任务记录的文件路径
src\web_server\db_status_handlers.rs:356  # TODO: 需要确认是否有获取特定 dbnum sesno 的方法
src\fast_model\gen_model\utilities.rs:23  # TODO: 需要从原来的 E3D_DEBUG_ENABLED 获取
src\fast_model\gen_model\utilities.rs:36  # TODO: 需要从原来的 E3D_INFO_ENABLED 获取
src\fast_model\gen_model\utilities.rs:49  # TODO: 需要从原来的 E3D_TRACE_ENABLED 获取
src\fast_model\gen_model\query_compat.rs:180  # TODO: 需要实现 CATE 查询逻辑
src\fast_model\export_model\export_prepack_lod.rs:4251  # TODO: 从 pe 表查询 owner 的 owner_type
src\fast_model\export_model\export_prepack_lod.rs:5451  # TODO: 从 cache 中获取 owner 的 owner_type
src\versioned_db\database.rs:336  # TODO: 改成一对多的实现
src\versioned_db\database.rs:416  # TODO: 需要实现create_tables函数或使用现有的表创建逻辑
src\versioned_db\database.rs:1065  # TODO: 处理相关 Parquet 数据包
src\versioned_db\database.rs:1768  # TODO: 按照文件大小排序，只有小于多少的能开启多线程，模型一大就不合适了
```

## src

### src\data_interface\mesh_manager.rs
```
impl AiosDBManager
  pub async fn cache_plant_meshes(&self, geo_hashes: impl IntoIterator<Item = &u64>, overwrite: bool,) → anyhow::Result<bool>
  pub async fn get_plant_mesh(&self, geo_hash: u64) → anyhow::Result<Option<Plant...
  pub async fn get_transformed_plant_mesh(&self, geo_hash: u64, t: &Transform,) → anyhow::Result<Option<Plant...
  pub async fn get_transformed_mesh_elev(&self, geo_hash: u64, t: &Transform,) → anyhow::Result<Option<(f32,...
```

### src\dblist_parser\db_loader.rs
```
pub struct DblistLoader
impl DblistLoader
  pub fn new(elements: Vec<PdmsElement>) → Self
  pub async fn load_to_memory_db(&self) → Result<()>
  pub async fn get_all_refnos(&self) → Result<Vec<RefnoEnum>>
  pub async fn get_refnos_by_noun(&self, noun: &str) → Result<Vec<RefnoEnum>>
  pub fn print_statistics(&self)
```

### src\expression_fix.rs
```
pub struct ExpressionValidationError
pub struct ExpressionFixer
pub enum ExpressionErrorType
impl ExpressionValidationError
impl ExpressionFixer
  pub fn normalize_attrib_colon(expr: &str) → String
  pub fn normalize_pdms_prefix_operators(expr: &str) → String
  pub fn preprocess_attrib_expression(expr: &str) → String
  pub fn eval_expression_with_attrib_support(expr: &str, context: &CataContext, unit: &str,) → Result<f64>
pub fn eval_pdms_expression(expr: &str, context: &CataContext) → Result<f64>
pub fn eval_attrib_expression(expr: &str, context: &CataContext, unit: &str) → Result<f64>
pub fn eval_enhanced_expression(expr: &str, context: &CataContext, unit: &str) → Result<f64>
```

### src\fast_model\cal_model\equip_model.rs
```
pub async fn update_cal_equip() → anyhow::Result<()>
pub async fn update_cal_equip_wtrans() → anyhow::Result<()>
pub async fn cal_equip_nearest_floor() → anyhow::Result<()>
```

### src\fast_model\cache_flush.rs
```
pub async fn flush_latest_instance_cache_to_surreal(_cache_dir: &Path, _dbnums: Option<&[u32]>, _replace_exist: bool, _verbose: bool, _refno_filter: Option<&HashSet<RefnoEnum>>,) → anyhow::Result<usize>
```

### src\fast_model\convex_decomp.rs
```
pub struct ConvexDecompositionFileV1
pub struct ConvexDecompParamsV1
pub struct ConvexHullDataV1
pub struct ConvexRuntime
pub struct ConvexHullRuntime
pub enum ConvexSourceV1
pub fn normalize_base_mesh_dir(mesh_dir: &Path) → PathBuf
pub fn convex_file_path(base_mesh_dir: &Path, geo_hash: &str) → PathBuf
pub fn clear_convex_cache()
pub async fn load_or_build_convex_runtime(mesh_dir: &Path, geo_hash: &str,) → Result<Arc<ConvexRuntime>>
pub async fn build_and_save_convex_from_glb(base_mesh_dir: &Path, geo_hash: &str,) → Result<Arc<ConvexRuntime>>
pub fn component_overlaps_room(panel_meshes: &[Arc<TriMesh>], panel_world_aabb: &Aabb, component_mat: &Mat4, component_hulls: &ConvexRuntime, tolerance: f32,) → bool
```

### src\fast_model\export_model\export_gltf.rs
```
pub struct GltfExporter
impl GltfExporter
  pub fn new() → Self
impl GltfExporter
impl GltfExporter
pub async fn export_gltf_for_refnos(refnos: &[RefnoEnum], mesh_dir: &Path, output_path: &str, filter_nouns: Option<&[String]>, include_descendants: bool,) → Result<()>
```

### src\fast_model\export_model\export_glb.rs
```
pub struct MeshIndexMap
pub struct GlbExportResult
pub struct GlbExporter
impl MeshIndexMap
  pub fn get(&self, geo_hash: &str) → Option<usize>
impl GlbExporter
  pub fn new() → Self
impl GlbExporter
impl GlbExporter
pub async fn export_glb_for_refnos(refnos: &[RefnoEnum], mesh_dir: &Path, output_path: &str, filter_nouns: Option<&[String]>, include_descendants: bool,) → Result<()>
pub fn export_single_mesh_to_glb(mesh: &PlantMesh, output_path: &Path) → Result<()>
```

### src\fast_model\export_model\export_pdms_tree_parquet.rs
```
pub struct PdmsTreeParquetStats
pub struct WorldSitesParquetStats
pub async fn export_pdms_tree_parquet(dbnum: u32, output_dir: &Path, verbose: bool,) → Result<PdmsTreeParquetStats>
pub async fn export_world_sites_parquet(output_dir: &Path, verbose: bool,) → Result<WorldSitesParquetStats>
```

### src\fast_model\export_model\import_glb.rs
```
pub fn import_glb_to_mesh(path: &Path) → Result<PlantMesh>
```

### src\fast_model\export_model\export_unit_mesh_glb.rs
```
pub struct UnitMeshIndexMap
pub struct UnitMeshGlbExportResult
pub struct UnitMeshGlbExporter
impl UnitMeshIndexMap
  pub fn get(&self, geo_hash: &str) → Option<usize>
impl UnitMeshGlbExporter
  pub fn new() → Self
impl UnitMeshGlbExporter
impl UnitMeshGlbExporter
pub async fn export_unit_mesh_glb_for_refnos(refnos: &[RefnoEnum], mesh_dir: &Path, output_path: &str, filter_nouns: Option<&[String]>, include_descendants: bool,) → Result<()>
```

### src\fast_model\export_model\model_exporter.rs
```
pub struct CommonExportConfig
pub struct ObjExportConfig
pub struct GlbExportConfig
pub struct GltfExportConfig
pub struct XktExportConfig
pub struct ExportStats
pub trait ModelExporter
impl CommonExportConfig
impl CommonExportConfig
  pub fn with_unit_conversion(include_descendants: bool, filter_nouns: Option<Vec<String>>, verbose: bool, source_unit: LengthUnit, target_unit: LengthUnit,) → Self
  pub fn with_unit_conversion_str(include_descendants: bool, filter_nouns: Option<Vec<String>>, verbose: bool, source_unit: &str, target_unit: &str,) → Result<Self, String>
impl ObjExportConfig
impl GlbExportConfig
impl GltfExportConfig
impl XktExportConfig
impl ExportStats
  pub fn new() → Self
  pub fn print_summary(&self, format_name: &str)
pub async fn collect_export_refnos(input_refnos: &[RefnoEnum], include_descendants: bool, filter_nouns: Option<&[String]>, verbose: bool,) → Result<Vec<RefnoEnum>>
pub async fn query_geometry_instances(refnos: &[RefnoEnum], enable_holes: bool, verbose: bool,) → Result<Vec<GeomInstQuery>>
pub async fn query_geometry_instances_ext(refnos: &[RefnoEnum], enable_holes: bool, include_negative: bool, verbose: bool,) → Result<Vec<GeomInstQuery>>
pub async fn query_geometry_instances_ext_from_cache(refnos: &[RefnoEnum], cache_dir: &Path, enable_holes: bool, include_negative: bool, verbose: bool,) → Result<Vec<GeomInstQuery>>
```

### src\fast_model\export_model\parquet_stream_writer.rs
```
pub struct ParquetStreamWriter
impl ParquetStreamWriter
  pub fn new(output_dir: impl AsRef<Path>) → Result<Self>
  pub fn write_batch(&self, data: &ShapeInstancesData) → Result<(usize, usize, usize)>
  pub fn finalize(&self) → Result<()>
```

### src\fast_model\export_model\parquet_writer.rs
```
pub struct TransformRow
pub struct ParquetManager
impl ParquetManager
  pub fn new(base_dir: impl AsRef<Path>) → Self
  pub fn list_files(&self, dbnum: u32, prefix_type: &str) → Result<Vec<PathBuf>>
  pub fn check_existence(&self, dbnum: u32, refnos: &[String]) → Result<Vec<String>>
  pub fn write_incremental(&self, data: &ExportData, dbnum: u32) → Result<(PathBuf, PathBuf)>
pub fn dmat4_to_f32_array(mat: &glam::DMat4) → [f32
```

### src\fast_model\export_model\pe_parquet_writer.rs
```
pub struct PeRow
pub struct PeParquetManager
impl PeRow
  pub fn from_pe(pe: &SPdmsElement) → Self
impl PeParquetManager
  pub fn new(base_dir: impl AsRef<Path>) → Self
  pub fn write_incremental(&self, pes: &[SPdmsElement], dbnum: u32) → Result<PathBuf>
  pub fn compact(&self, dbnum: u32) → Result<Option<PathBuf>>
```

### src\fast_model\export_model\simple_color_palette.rs
```
pub struct SimpleColorPalette
impl SimpleColorPalette
  pub fn new() → Self
  pub fn index_for_noun(&mut self, noun: &str) → i32
  pub fn into_colors(mut self) → Vec<[f32
```

### src\fast_model\export_model\spec_info.rs
```
pub async fn build_spec_info_parquet(dbnum: u32, tree_dir: &Path, output_path: &Path, verbose: bool,) → Result<HashMap<u64, i64>>
pub async fn load_or_build_spec_info(dbnum: u32, tree_dir: &Path, output_dir: &Path, verbose: bool,) → Result<HashMap<u64, i64>>
```

### src\fast_model\gen_model\boolean_backfill.rs
```
pub async fn query_cata_backfill_candidates(existing_task_refnos: &HashSet<RefnoEnum>,) → anyhow::Result<Vec<RefnoEnum>>
pub async fn fetch_cata_bool_tasks_from_db(refnos: &[RefnoEnum],) → anyhow::Result<Vec<BooleanT...
pub async fn backfill_cata_tasks_from_db(existing_tasks: &mut Vec<BooleanTask>, use_surrealdb: bool,) → anyhow::Result<usize>
```

### src\fast_model\gen_model\boolean_task.rs
```
pub struct BooleanTask
pub struct CataNegBoolTask
pub struct CataGeoData
pub struct InstNegBoolTask
pub struct PosGeoData
pub struct NegEntityData
pub struct NegGeoData
pub struct BooleanTaskAccumulator
pub enum BooleanTaskType
impl BooleanTaskAccumulator
  pub fn merge_batch(&mut self, batch: &ShapeInstancesData)
  pub fn build_tasks(&self) → Vec<BooleanTask>
pub fn extract_cata_neg_tasks(shape_insts: &ShapeInstancesData) → Vec<BooleanTask>
pub fn extract_inst_neg_tasks(shape_insts: &ShapeInstancesData) → Vec<BooleanTask>
pub fn build_boolean_tasks(shape_insts: &ShapeInstancesData) → Vec<BooleanTask>
```

### src\fast_model\gen_model\cache_miss_report.rs
```
pub struct CacheMissBucket
pub struct CacheMissReport
impl CacheMissBucket
impl CacheMissReport
  pub fn new(db_option: &DbOptionExt, mode: impl Into<String>) → Self
  pub fn with_sample_limit(mut self, limit: usize) → Self
  pub fn record_refno_miss(&mut self, stage: &str, kind: &str, refno: RefnoEnum, note: Option<&str>,)
  pub fn record_simple_miss(&mut self, stage: &str, kind: &str, note: Option<&str>)
  pub fn default_report_path(db_option: &DbOptionExt) → PathBuf
  pub fn write_to(&self, path: &Path) → anyhow::Result<()>
  pub fn write_to_default_path(&self, db_option: &DbOptionExt) → anyhow::Result<PathBuf>
pub fn init_global_cache_miss_report(db_option: &DbOptionExt, mode: impl Into<String>)
pub fn with_global_report(f: impl FnOnce(&mut CacheMissReport) → R) -> Option<R>
pub fn snapshot_global_report() → Option<CacheMissReport>
```

### src\fast_model\gen_model\cata_resolve_cache_pipeline.rs
```
pub struct PrefetchOutcome
pub async fn prefetch_cata_resolve_cache_for_target_map(_db_option: Arc<DbOptionExt>, _target_cata_map: Arc<DashMap<String, CataHashRefnoKV>>,) → anyhow::Result<PrefetchOutc...
```

### src\fast_model\gen_model\cate_single.rs
```
pub async fn gen_cata_single_geoms(design_refno: RefnoEnum, csg_shape_map: &CateCsgShapeMap, design_axis_map: &DashMap<RefnoEnum, PlantAxisMap>,) → anyhow::Result<bool>
```

### src\fast_model\gen_model\cate_processor.rs
```
pub async fn process_cate_refno_page(ctx: &NounProcessContext, loop_sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32) → Result<()>
```

### src\fast_model\gen_model\cate_helpers.rs
```
pub enum NgmrRemovedType
pub fn cal_sjus_value(sjus: &str, height: f32) → f32
pub async fn query_ngmr_owner(refno: RefnoEnum, ngmr_geo_refno: RefnoEnum,) → Result<Vec<RefnoEnum>>
```

### src\fast_model\gen_model\index_tree_mode.rs
```
pub struct NounTypeInfo
pub enum NounCategoryType
pub fn validate_sjus_map(sjus_map: &DashMap<RefnoEnum, (Vec3, f32) → Result<()>
pub async fn prequery_noun_counts(nouns: &[&'static str], dbnums: &[u32],) → Result<Vec<NounTypeInfo>>
pub async fn process_nouns_by_type(noun_infos: Vec<NounTypeInfo>, ctx: &NounProcessContext, category: NounCategoryType, loop_sjus_map: Arc<DashMap<RefnoEnum, (Vec3, f32) → Result<Vec<RefnoEnum>>
pub async fn gen_index_tree_geos_optimized(db_option: Arc<DbOptionExt>, config: &IndexTreeConfig, sender: flume::Sender<ShapeInstancesData>, seed_roots: Option<Vec<RefnoEnum>>,) → Result<CategorizedRefnos>
```

### src\fast_model\gen_model\loop_processor.rs
```
pub async fn process_loop_refno_page(ctx: &NounProcessContext, loop_sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32) → Result<()>
```

### src\fast_model\gen_model\neg_query.rs
```
pub fn group_by_dbnum(refnos: &[RefnoEnum], mut resolver: F,) → anyhow::Result<HashMap<u32,...
pub fn group_by_dbnum_best_effort(refnos: &[RefnoEnum], mut resolver: F,) → (HashMap<u32, Vec<RefnoEnum...
pub fn query_descendants_map_by_dbnum(tree_dir: impl AsRef<Path>, roots: &[RefnoEnum], nouns: &[&str], include_self: bool,) → anyhow::Result<HashMap<Refn...
```

### src\fast_model\gen_model\models.rs
```
pub struct DbModelInstRefnos
pub enum NounCategory
impl DbModelInstRefnos
  pub async fn execute_gen_inst_meshes(&self, db_option_arc: Option<Arc<DbOptionExt>>)
```

### src\fast_model\gen_model\query.rs
```
pub async fn query_gm_params(refno: RefnoEnum) → anyhow::Result<Vec<GmParam>>
```

### src\fast_model\gen_model\prim_processor.rs
```
pub async fn process_prim_refno_page(ctx: &NounProcessContext, sender: flume::Sender<ShapeInstancesData>, refnos: &[RefnoEnum],) → Result<()>
```

### src\fast_model\gen_model\processor.rs
```
pub struct NounProcessor
impl NounProcessor
  pub fn new(ctx: NounProcessContext, category_name: &'static str, debug_limit_per_noun: Option<usize>,) → Self
  pub async fn process_nouns(&self, nouns: &[&'static str], refno_sink: Arc<RwLock<HashSet<RefnoEnum>>>, page_processor: F,) → Result<()> where F: Fn(Vec<...
```

### src\fast_model\gen_model\query_provider.rs
```
pub async fn get_model_query_provider() → anyhow::Result<Arc<dyn Quer...
pub async fn get_descendants_by_types(root: RefnoEnum, nouns: &[&str], _max_depth: Option<usize>,) → anyhow::Result<Vec<RefnoEnum>>
pub async fn get_children_batch(refnos: &[RefnoEnum]) → anyhow::Result<Vec<RefnoEnum>>
pub async fn query_by_type(nouns: &[&str], dbnum: i32, has_children: Option<bool>,) → anyhow::Result<Vec<RefnoEnum>>
pub fn query_by_noun_all_db(nouns: &[&str]) → anyhow::Result<Vec<RefnoEnum>>
pub fn count_noun_all_db(noun: &str) → anyhow::Result<u64>
pub fn query_noun_page_all_db(noun: &str, start: usize, limit: usize,) → anyhow::Result<Vec<RefnoEnum>>
pub async fn get_pes_batch(refnos: &[RefnoEnum]) → anyhow::Result<Vec<PE>>
pub async fn get_pe(refno: RefnoEnum) → anyhow::Result<Option<PE>>
pub async fn get_children(refno: RefnoEnum) → anyhow::Result<Vec<RefnoEnum>>
pub async fn get_ancestors(refno: RefnoEnum) → anyhow::Result<Vec<RefnoEnum>>
pub async fn get_ancestors_of_type(refno: RefnoEnum, nouns: &[&str],) → anyhow::Result<Vec<RefnoEnum>>
pub async fn get_children_pes(refno: RefnoEnum) → anyhow::Result<Vec<PE>>
pub async fn get_attmaps_batch(refnos: &[RefnoEnum]) → anyhow::Result<Vec<NamedAtt...
pub async fn query_multi_descendants(refnos: &[RefnoEnum], nouns: &[&str],) → anyhow::Result<Vec<RefnoEnum>>
pub async fn query_multi_descendants_with_self(refnos: &[RefnoEnum], nouns: &[&str], include_self: bool,) → anyhow::Result<Vec<RefnoEnum>>
pub async fn get_provider_name() → String
pub async fn health_check() → anyhow::Result<bool>
```

### src\fast_model\gen_model\resolve.rs
```
pub async fn get_or_create_scom_info(cata_refno: RefnoEnum) → anyhow::Result<ScomInfo>
pub async fn resolve_axis_params(refno: RefnoEnum, context: Option<CataContext>,) → anyhow::Result<BTreeMap<i32...
pub async fn resolve_desi_comp(desi_refno: RefnoEnum, mut tubi_scom: Option<RefnoEnum>, desi_att_opt: Option<&NamedAttrMap>,) → anyhow::Result<CateGeomsInfo>
```

### src\fast_model\gen_model\transform_rkyv_cache.rs
```
pub struct TransformCacheFileV1
pub struct TransformCacheEntryV1
pub struct TransformRecordV1
pub struct LoadedTransformDbnum
impl TransformRecordV1
```

### src\fast_model\gen_model\sql_file_writer.rs
```
pub struct SqlFileWriter
impl SqlFileWriter
  pub fn new(path: &Path) → anyhow::Result<Self>
  pub fn default_path(project_output_dir: &Path, dbnum: Option<u32>) → PathBuf
  pub fn write_statement(&self, sql: &str) → anyhow::Result<()>
  pub fn write_statements(&self, sqls: &[String]) → anyhow::Result<()>
  pub fn write_comment(&self, comment: &str) → anyhow::Result<()>
  pub fn flush(&self) → anyhow::Result<()>
  pub fn path(&self) → &Path
  pub fn statement_count(&self) → usize
impl SqlFileWriter
pub async fn import_sql_file(path: &std::path::Path, batch_size: usize,) → anyhow::Result<(usize, usize)>
```

### src\fast_model\gen_model\tree_index_manager.rs
```
pub struct TreeIndexMissingError
pub struct TreeIndexManager
impl TreeIndexMissingError
impl TreeIndexMissingError
impl TreeIndexManager
  pub fn new(tree_dir: impl AsRef<Path>, dbnums: Vec<u32>) → Self
  pub fn with_default_dir(dbnums: Vec<u32>) → Self
  pub fn dbnums(&self) → &[u32]
  pub fn tree_dir(&self) → &Path
  pub fn load_index(&self, dbnum: u32) → anyhow::Result<Arc<TreeIndex>>
  pub fn tree_file_exists(&self, dbnum: u32) → bool
  pub fn get_missing_tree_files(&self) → Vec<u32>
  pub fn resolve_dbnum_for_refno(refno: RefnoEnum) → anyhow::Result<u32>
pub fn try_get_cached_index(tree_dir: impl AsRef<Path>, dbnum: u32) → Option<Arc<TreeIndex>>
pub fn load_index_with_large_stack(tree_dir: impl AsRef<Path>, dbnum: u32,) → anyhow::Result<Arc<TreeIndex>>
```

### src\fast_model\instance_cache.rs
```
pub struct CachedInstInfo
pub struct CachedInstGeos
pub struct InstanceCacheManager
impl InstanceCacheManager
  pub async fn new(cache_dir: &Path) → anyhow::Result<Self>
  pub fn list_refnos(&self, _dbnum: u32) → Vec<RefnoEnum>
  pub fn list_dbnums(&self) → Vec<u32>
  pub async fn get_inst_info(&self, _dbnum: u32, _refno: RefnoEnum) → Option<CachedInstInfo>
  pub async fn get_inst_geos(&self, _dbnum: u32, _inst_key: &str) → Option<CachedInstGeos>
  pub fn insert_from_shape(&self, _dbnum: u32, _shape: &ShapeInstancesData)
  pub async fn get_ptset_maps_for_refnos_auto(&self, _refnos: &[RefnoEnum],) → HashMap<RefnoEnum, BTreeMap...
  pub async fn close(&self) → anyhow::Result<()>
```

### src\fast_model\model_cache\geom_input_cache.rs
```
pub struct LoopInput
pub struct PrimInput
pub struct PrimPolyExtra
pub struct PrimPolygonData
pub struct CateInput
pub enum CacheRunMode
pub fn init_global_geom_input_cache()
pub async fn prefetch_all_geom_inputs(_db_option: &DbOptionExt, _loop_refs: &[RefnoEnum], _prim_refs: &[RefnoEnum], _cate_refs: &[RefnoEnum],) → anyhow::Result<()>
pub fn ensure_geom_inputs_present_for_refnos_from_global(_loop_refs: &[RefnoEnum], _prim_refs: &[RefnoEnum], _cate_refs: &[RefnoEnum],) → anyhow::Result<()>
pub fn load_cate_inputs_for_refnos_from_global(_refnos: &[RefnoEnum],) → anyhow::Result<HashMap<Refn...
```

### src\fast_model\model_cache\query.rs
```
pub async fn query_geometry_instances_ext_from_cache(_refnos: &[RefnoEnum], _cache_dir: &Path, _enable_holes: bool, _include_negative: bool, _verbose: bool,) → Result<Vec<GeomInstQuery>>
```

### src\fast_model\model_store.rs
```
pub async fn model_query_response(sql: S) → anyhow::Result<Response>
pub async fn model_query_take(sql: S, idx: usize) → anyhow::Result<T> where T: ...
```

### src\fast_model\utils.rs
```
pub async fn ensure_surreal_init() → anyhow::Result<()>
pub async fn ensure_inst_relate_relation_schema()
pub async fn save_aabb_to_surreal(aabb_map: &DashMap<String, Aabb>)
pub async fn save_inst_relate_bool(refno: RefnoEnum, mesh_id: Option<&str>, status: &str, source: &str,) → anyhow::Result<()>
pub async fn save_inst_relate_cata_bool(refno: RefnoEnum, mesh_id: Option<&str>, status: &str, source: &str,)
pub async fn save_inst_relate_aabb(inst_aabb_map: &DashMap<RefnoEnum, String>, _source: &str)
pub async fn save_inst_relate_booled_aabb(inst_aabb_map: &DashMap<RefnoEnum, String>, _source: &str,) → anyhow::Result<()>
pub async fn save_pts_to_surreal(vec3_map: &DashMap<u64, String>)
pub async fn save_transforms_to_surreal(trans_map: &HashMap<u64, String>) → anyhow::Result<()>
```

### src\fast_model\shared.rs
```
pub async fn get_owner_info_from_attr(attr: &NamedAttrMap) → (RefnoEnum, String)
pub fn aabb_apply_transform(aabb: &Aabb, t: &Transform) → Aabb
```

### src\fast_model\unit_converter.rs
```
pub struct UnitConverter
pub enum LengthUnit
impl LengthUnit
  pub fn name(&self) → &'static str
  pub fn full_name(&self) → &'static str
  pub fn to_meter_factor(&self) → f32
  pub fn from_str(s: &str) → Result<Self, String>
impl LengthUnit
impl UnitConverter
  pub fn new(source_unit: LengthUnit, target_unit: LengthUnit) → Self
  pub fn default() → Self
  pub fn conversion_factor(&self) → f32
  pub fn convert_value(&self, value: f32) → f32
  pub fn convert_vec3(&self, vec: &glam::Vec3) → glam::Vec3
  pub fn convert_vec3_array(&self, values: &[glam::Vec3]) → Vec<f32>
  pub fn convert_translation(&self, translation: &glam::Vec3) → glam::Vec3
  pub fn needs_conversion(&self) → bool
impl UnitConverter
```

### src\profiling.rs
```
pub fn init_chrome_tracing(trace_path: impl AsRef<Path>) → anyhow::Result<()>
pub fn init_chrome_tracing(_trace_path: impl AsRef<Path>) → anyhow::Result<()>
pub fn init_chrome_tracing_for_db_option(db_option: &crate::options::DbOptionExt, stage: &str,) → anyhow::Result<PathBuf>
```

### src\perf_timer.rs
```
pub struct StageRecord
pub struct PerfTimer
pub struct StageSummary
pub struct PerfReport
impl PerfTimer
  pub fn new(label: &str) → Self
  pub fn mark(&mut self, stage_name: &str)
  pub fn end_current(&mut self)
  pub fn total_ms(&self) → u128
  pub fn print_summary(&mut self)
  pub fn stages(&self) → &[StageRecord]
  pub fn generate_report(&mut self, metadata: serde_json::Value) → PerfReport
  pub fn save_json(&mut self, output_path: &std::path::Path, metadata: serde_json::Value,) → std::io::Result<()>
```

### src\scene_tree\parquet_export.rs
```
pub async fn export_scene_tree_parquet(dbnum: u32, output_dir: &Path) → Result<usize>
```

### src\scene_tree\init.rs
```
pub struct SceneTreeInitResult
pub async fn init_scene_tree(mdb_name: &str, force_rebuild: bool) → Result<SceneTreeInitResult>
pub async fn init_scene_tree_from_root(root_refno: RefnoEnum, force_rebuild: bool,) → Result<SceneTreeInitResult>
pub async fn init_scene_tree_by_dbno(dbnum: u32, force_rebuild: bool,) → Result<SceneTreeInitResult>
```

### src\scene_tree\mod.rs
```
pub fn is_geo_noun(noun: &str) → bool
pub async fn is_initialized() → Result<bool>
pub async fn ensure_initialized() → Result<()>
```

### src\scene_tree\schema.rs
```
pub async fn init_schema() → Result<()>
```

### src\scene_tree\query.rs
```
pub struct SceneNodeStatus
pub async fn query_generation_status(refnos: &[RefnoEnum]) → Result<Vec<SceneNodeStatus>>
pub async fn filter_ungenerated_geo_nodes(refnos: &[RefnoEnum]) → Result<Vec<i64>>
pub async fn mark_as_generated(ids: &[i64]) → Result<()>
pub async fn query_ungenerated_leaves(root_id: i64) → Result<Vec<i64>>
pub async fn query_children_ids(parent_id: i64, limit: usize) → Result<Vec<i64>>
pub async fn query_ancestor_ids(start_id: i64, limit: usize) → Result<Vec<i64>>
pub async fn update_scene_node_aabb(inst_aabb_map: &DashMap<RefnoEnum, String>) → Result<()>
pub async fn query_generated_refnos(refnos: &[RefnoEnum]) → Result<Vec<RefnoEnum>>
```

### src\spatial_index.rs
```
pub struct SpatialIndexStats
pub struct SqliteSpatialIndex
impl SqliteSpatialIndex
  pub fn inner(&self) → &SqliteAabbIndex
  pub fn is_enabled() → bool
  pub fn default_path() → PathBuf
  pub fn with_default_path() → anyhow::Result<Self>
  pub fn clear(&self) → anyhow::Result<()>
  pub fn get_stats(&self) → anyhow::Result<SpatialIndex...
  pub fn get_aabb(&self, refno: RefU64) → anyhow::Result<Option<Aabb>>
  pub fn query_intersect(&self, query: &Aabb) → anyhow::Result<Vec<RefU64>>
```

### src\team_data.rs
```
pub struct SysDBData
pub async fn sync_team_data() → anyhow::Result<()>
```

### src\versioned_db\db_meta_info.rs
```
pub struct DbFileMetaUpdate
pub fn get_project_tree_dir(project_name: &str) → std::path::PathBuf
pub fn update_db_meta_info_json(output_dir: &Path, update: DbFileMetaUpdate) → anyhow::Result<()>
```

### src\versioned_db\tree_export.rs
```
pub struct TreeNodeMeta
pub fn export_tree_file(dbnum: u32, _db_basic: &DbBasicData, tree_nodes: &HashMap<RefU64, TreeNodeMeta>, children_map: &HashMap<RefU64, Vec<RefU64>>, output_dir: &Path,) → anyhow::Result<()>
```

### src\web_api\noun_hierarchy_api.rs
```
pub struct NounHierarchyApiState
pub struct NounHierarchyQueryRequest
pub struct NounHierarchyNode
pub struct NounHierarchyQueryResponse
pub struct NounTreeNode
pub struct NounTreeQueryRequest
pub struct NounTreeQueryResponse
pub fn create_noun_hierarchy_routes(state: NounHierarchyApiState) → Router
```

### src\web_api\pdms_model_query_api.rs
```
pub struct RefnoQuery
pub struct TypeInfoResponse
pub struct ChildrenResponse
pub fn create_pdms_model_query_routes() → Router
```

### src\web_api\pdms_attr_api.rs
```
pub struct UiAttrResponse
pub fn create_pdms_attr_routes() → Router
```

### src\web_api\pipeline_annotation_api.rs
```
pub struct AnnotationResponse
pub struct AnnotationData
pub enum AnnotationCommand
pub fn create_pipeline_annotation_routes() → Router
```

### src\web_api\scene_tree_api.rs
```
pub struct InitRequest
pub struct InitByRootRequest
pub struct InitByDbnoRequest
pub struct InitResponse
pub struct LeavesResponse
pub struct ChildrenQuery
pub struct ChildrenResponse
pub struct AncestorsQuery
pub struct AncestorsResponse
pub fn create_scene_tree_routes() → Router
```

### src\web_server\dashboard_handlers.rs
```
pub struct DashboardActivitiesQuery
pub struct DashboardActivityItem
pub struct DashboardActivitiesResponse
pub async fn api_dashboard_activities(Query(query) → Result<Json<DashboardActivi...
```

### src\web_api\upload_api.rs
```
pub struct UploadApiState
pub struct ParseTask
pub struct UploadResponse
pub struct TaskStatusResponse
pub enum TaskStatus
pub fn create_upload_routes(state: UploadApiState) → Router
```

### src\web_server\database_diagnostics.rs
```
pub struct DatabaseDiagnosticResult
pub struct DiagnosticCheck
pub struct ConnectionInfo
pub enum DiagnosticStatus
impl DatabaseDiagnosticResult
  pub fn new() → Self
  pub fn add_check(&mut self, check: DiagnosticCheck)
  pub fn add_recommendation(&mut self, recommendation: String)
pub async fn run_database_diagnostics() → DatabaseDiagnosticResult
```

### src\web_server\database_status_handlers.rs
```
pub struct DatabaseStatus
pub struct StatusQuery
pub struct BatchOperationRequest
pub enum ProcessStatus
pub async fn get_all_database_status(_state: State<AppState>, Query(query) → Result<Json<serde_json::Val...
pub async fn get_database_details(_state: State<AppState>, Path(db_num) → Result<Json<serde_json::Val...
pub async fn execute_batch_operation(state: State<AppState>, Json(request) → Result<Json<serde_json::Val...
pub async fn trigger_database_update(state: State<AppState>, Path(db_num) → Result<Json<serde_json::Val...
pub async fn reparse_database(state: State<AppState>, Path(db_num) → Result<Json<serde_json::Val...
pub async fn regenerate_model(state: State<AppState>, Path(db_num) → Result<Json<serde_json::Val...
pub async fn clear_database_cache(_state: State<AppState>, Path(db_num) → Result<Json<serde_json::Val...
pub async fn get_module_list(_state: State<AppState>,) → Result<Json<serde_json::Val...
```

### src\web_server\parquet_compact_worker.rs
```
pub struct CompactWorkerConfig
impl CompactWorkerConfig
pub fn start_compact_worker(config: CompactWorkerConfig) → tokio::task::JoinHandle<()>
```

### src\web_server\remote_runtime.rs
```
pub struct RuntimeState
pub async fn stop_runtime()
pub async fn start_runtime(env_id: String) → anyhow::Result<()>
```

### src\web_server\simple_templates.rs
```
pub fn render_simple_index_page() → String
pub fn render_xtk_viewer_page() → String
pub fn render_index_with_sidebar() → String
pub fn render_database_connection_page() → String
pub fn render_embed_url_tester_page() → String
pub fn render_simple_dashboard_page() → String
pub fn render_simple_config_page() → String
pub fn render_simple_generic_page(title: &str, content: &str) → String
pub fn render_advanced_tasks_page() → String
pub fn render_task_detail_page(task_id: String) → String
pub fn render_task_logs_page(task_id: String) → String
pub fn render_dashboard_page_with_sidebar() → String
pub fn render_config_page_with_sidebar() → String
pub fn render_deployment_sites_page_with_sidebar() → String
```

### src\web_server\task_creation_handlers.rs
```
pub struct TaskRequest
pub struct TaskOptions
pub struct TaskCreationRequest
pub struct TaskParameters
pub struct TaskCreationResponse
pub struct DeploymentSite
pub struct TaskTemplate
pub struct TaskNameValidationResponse
pub struct TaskPreviewResponse
pub struct ResourceRequirements
pub enum TaskType
pub enum TaskStatus
pub enum TaskPriority
pub async fn create_task(State(state) → Result<Json<TaskCreationRes...
pub async fn get_deployment_sites(State(_state) → Result<Json<Vec<DeploymentS...
pub async fn get_task_templates(State(_state) → Result<Json<Vec<TaskTemplat...
pub async fn validate_task_name(Query(params) → Result<Json<TaskNameValidat...
pub async fn preview_task_config(Json(request) → Result<Json<TaskPreviewResp...
pub async fn download_task_export(axum::extract::Path(task_id) → Result<axum::response::Resp...
```

### src\web_api\spatial_query_api.rs
```
pub struct SpatialQueryApiState
pub struct SpatialNode
pub struct SpatialQueryResponse
pub struct NodeInfoResponse
pub fn create_spatial_query_routes(state: SpatialQueryApiState) → Router
```

### src\web_server\db_status_handlers.rs
```
pub struct AutoUpdateRequest
pub struct AutoUpdateTypeRequest
pub struct LocalScanItem
pub struct LocalScanResult
pub struct SyncFileMetadataRequest
pub async fn get_db_status_list(State(_state) → Result<Json<serde_json::Val...
pub async fn get_db_status_detail(State(_state) → Result<Json<serde_json::Val...
pub async fn execute_incremental_update(State(state) → Result<Json<serde_json::Val...
pub async fn execute_incremental_update(State(_state) → Result<Json<serde_json::Val...
pub async fn check_file_versions(State(state) → Result<Json<serde_json::Val...
pub async fn check_file_versions(State(_state) → Result<Json<serde_json::Val...
pub async fn set_auto_update(Path(dbnum) → Result<Json<serde_json::Val...
pub async fn set_auto_update_type(Path(dbnum) → Result<Json<serde_json::Val...
pub async fn scan_local_files() → Result<Json<serde_json::Val...
pub async fn sync_file_metadata(Json(req) → Result<Json<serde_json::Val...
pub async fn rescan_and_cache(Json(req) → Result<Json<serde_json::Val...
```

### src\fast_model\gen_model\pdms_inst_v2.rs
```
pub async fn pre_cleanup_for_regen_v2(seed_refnos: &[RefnoEnum]) → Result<()>
pub async fn save_instance_data_to_sqlite(dbnum: u32, inst_relates: &[crate::model_relation_store::InstRelateRecord], geo_relates: &[(u64, u64) → Result<()>
```

### src\fast_model\gen_model\pdms_inst_v3.rs
```
pub async fn pre_cleanup_for_regen_v3(seed_refnos: &[RefnoEnum]) → Result<()>
pub async fn save_model_relations_v3(dbnum: u32, refno_data_map: HashMap<RefnoEnum, RefnoRelations>,) → Result<()>
```

### src\model_relation_store_v3.rs
```
pub struct RefnoRelations
pub struct ModelRelationStoreV3
impl ModelRelationStoreV3
  pub fn new(base_path: impl AsRef<Path>) → Self
  pub fn cleanup_by_refnos(&self, dbnum: u32, refnos: &[RefnoEnum]) → Result<usize>
  pub fn save_relations(&self, dbnum: u32, relations: &[RefnoRelations]) → Result<()>
  pub fn load_relations(&self, dbnum: u32, refnos: &[RefnoEnum]) → Result<Vec<RefnoRelations>>
  pub fn get_stats(&self, dbnum: u32) → Result<usize>
pub fn global_store_v3() → &'static ModelRelationStoreV3
```

### src\fast_model\export_model\export_dbnum_instances_parquet.rs
```
pub struct ParquetExportStats
pub async fn query_distinct_dbnums_from_inst_relate() → Result<Vec<u32>>
pub async fn export_dbnum_instances_parquet(dbnum: u32, output_dir: &Path, db_option: Arc<DbOption>, verbose: bool, target_unit: Option<LengthUnit>, root_refno: Option<RefnoEnum>,) → Result<ParquetExportStats>
```

### src\fast_model\export_model\export_instanced_bundle.rs
```
pub struct InstancedManifest
pub struct ArchetypeInfo
pub struct LodLevelInfo
pub struct InstancesData
pub struct InstanceInfo
pub struct InstancedBundleExporter
impl InstancedBundleExporter
  pub fn new(db_option: Arc<DbOption>, verbose: bool) → Self
  pub async fn export(&self, export_data: &ExportData, output_dir: &Path, mesh_dir: &Path,) → Result<()>
pub async fn export_instanced_bundle_for_refnos(refnos: &[RefnoEnum], mesh_dir: &Path, output_dir: &Path, db_option: Arc<DbOption>, verbose: bool,) → Result<super::export_common...
```

### src\fast_model\export_model\export_room_instances.rs
```
pub struct RoomComputeValidationFixture
pub struct RoomComputeValidationCase
pub struct AabbJson
pub struct RoomRelationsData
pub struct RoomGeometriesData
pub struct RoomGeometryGroup
pub struct RoomPanel
pub struct PanelInstance
pub struct RoomExportStats
pub struct RoomRelateRecord
pub struct RoomPanelRecord
impl RoomComputeValidationFixture
  pub fn load_from_path(path: &Path) → Result<Self>
impl AabbJson
  pub fn merge(&self, other: &AabbJson) → AabbJson
pub async fn query_room_relations_for_verify() → Result<Vec<RoomRelateRecord>>
pub async fn query_room_panel_relations_for_verify() → Result<Vec<RoomPanelRecord>>
pub async fn export_room_relations(output_path: &Path, verbose: bool) → Result<RoomExportStats>
pub async fn export_room_geometries(output_path: &Path, verbose: bool) → Result<RoomExportStats>
pub async fn export_room_instances(output_dir: &Path, verbose: bool,) → Result<(RoomExportStats, Ro...
```

### src\fast_model\gen_model\inst_query.rs
```
pub async fn query_insts_with_batch(refnos: &[RefnoEnum], enable_holes: bool, batch_size: Option<usize>,) → anyhow::Result<Vec<GeomInst...
pub async fn query_insts(refnos: &[RefnoEnum], enable_holes: bool,) → anyhow::Result<Vec<GeomInst...
```

### src\web_api\room_tree_api.rs
```
pub struct RoomTreeNodeDto
pub struct NodeResponse
pub struct ChildrenResponse
pub struct AncestorsResponse
pub struct SearchRequest
pub struct SearchResponse
pub struct ChildrenQuery
pub enum RoomTreeNodeId
pub fn create_room_tree_routes() → Router
pub async fn room_tree_children_core(id: &str, limit: usize) → anyhow::Result<ChildrenResp...
pub async fn room_tree_ancestors_core(id: &str) → anyhow::Result<AncestorsRes...
pub async fn room_tree_search_core(keyword: &str, limit: usize) → anyhow::Result<SearchResponse>
```

### src\web_api\search_api.rs
```
pub struct SearchApiState
pub struct PdmsSearchRequest
pub struct PdmsSearchItem
pub struct PdmsSearchResponse
impl SearchApiState
  pub fn from_env() → Self
pub fn create_search_routes(state: SearchApiState) → Router
```

### src\fast_model\gen_model\prim_model.rs
```
pub async fn gen_prim_geos(db_option: Arc<DbOptionExt>, prim_refnos: &[RefnoEnum], sender: flume::Sender<ShapeInstancesData>,) → anyhow::Result<bool>
```

### src\fast_model\gen_model\manifold_bool.rs
```
pub struct BoolWorkerReport
pub trait BoolResultWriter
impl DbBoolWriter
impl SqlBoolWriter
impl SqlBoolWriter
pub async fn apply_cata_neg_boolean_manifold(refnos: &[RefnoEnum], replace_exist: bool,) → anyhow::Result<()>
pub async fn apply_insts_boolean_manifold(refnos: &[RefnoEnum], replace_exist: bool,) → anyhow::Result<()>
pub async fn run_bool_worker_from_tasks(tasks: Vec<BooleanTask>, db_option: Arc<aios_core::options::DbOption>, sql_writer: Option<Arc<SqlFileWriter>>,) → anyhow::Result<BoolWorkerRe...
```

### src\fast_model\gen_model\context.rs
```
pub struct NounProcessContext
pub enum GenStage
impl GenStage
  pub fn as_str(self) → &'static str
impl NounProcessContext
  pub fn new(db_option: Arc<DbOptionExt>, batch_size: usize, batch_concurrency: usize) → Self
  pub fn with_stage(&self, stage: GenStage) → Self
  pub fn is_offline_generate(&self) → bool
  pub fn bounded_chunks(&self, total: usize) → Vec<(usize, usize)>
```

### src\fast_model\gen_model\mesh_state.rs
```
pub fn use_file_mesh_state() → bool
pub fn flush_aabb_cache()
pub fn mesh_exists(geo_hash: u64) → bool
pub fn get_cached_or_local_aabb(geo_hash: u64) → Option<Aabb>
pub fn get_cached_or_local_aabb_in_dir(_mesh_dir: &Path, geo_hash: u64) → Option<Aabb>
pub fn prime_cached_aabb_for_mesh_ids(mesh_ids: impl IntoIterator<Item = &'a str>)
```

### src\fast_model\model_cache\cata_resolve_cache.rs
```
pub struct PreparedInstGeo
pub struct CataResolvedComp
pub struct CataResolveCache
impl CataResolvedComp
  pub fn ptset_map(&self) → BTreeMap<i32, CateAxisParam>
impl CataResolveCache
  pub fn new() → Self
  pub fn get(&self, key: &str) → Option<CataResolvedComp>
  pub fn insert(&self, key: String, value: &CataResolvedComp)
pub fn init_global_cata_resolve_cache()
pub fn global_cata_resolve_cache() → Option<Arc<CataResolveCache>>
```

### src\fast_model\model_cache\mod.rs
```
pub struct ModelCacheContext
impl ModelCacheContext
  pub async fn try_from_db_option(_db_option: &crate::options::DbOptionExt,) → anyhow::Result<Option<Self>>
  pub fn cache(&self) → &Self
  pub fn cache_arc(&self) → std::sync::Arc<Self>
  pub fn insert_from_shape(&self, _dbnum: u32, _shape_insts: &aios_core::geometry::ShapeInstancesData,)
  pub async fn close(&self) → anyhow::Result<()>
```

### src\rvm_obj_export.rs
```
pub struct RvmObjExportStats
pub fn export_rvm_obj_from_relation_store(dbnum: u32, relation_store_root: &Path, output_path: &Path, unit_converter: &UnitConverter, verbose: bool,) → Result<RvmObjExportStats>
```

### src\rvm_import.rs
```
pub struct RvmImportOptions
pub struct RvmImportStats
impl RelationBuilder
pub fn import_rvm_to_sqlite(options: &RvmImportOptions) → Result<RvmImportStats>
```

### src\fast_model\gen_model\pdms_inst_surreal.rs
```
pub struct RefnoRelations
pub async fn pre_cleanup_for_regen_surreal(seed_refnos: &[RefnoEnum]) → Result<()>
pub async fn save_refno_relations_surreal(relations: &[RefnoRelations]) → Result<()>
pub async fn load_refno_relations_surreal(refnos: &[RefnoEnum]) → Result<Vec<RefnoRelations>>
```

### src\fast_model\cata_cache_gen.rs
```
pub struct SimpleCataOutcome
pub struct SimpleBranOutcome
pub async fn gen_cata_geos_for_cache(db_option: Arc<DbOptionExt>, target_cata_map: Arc<DashMap<String, CataHashRefnoKV>>, _sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32) → anyhow::Result<SimpleCataOu...
pub async fn gen_bran_geos_for_cache(db_option: Arc<DbOptionExt>, branch_map: Arc<DashMap<RefnoEnum, Vec<SPdmsElement>>>, _sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32) → anyhow::Result<SimpleBranOu...
pub async fn gen_tubi_for_cache(db_option: Arc<DbOptionExt>, branch_refnos: &[RefnoEnum], sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32) → anyhow::Result<BranchTubiOu...
pub async fn gen_tubi_for_cache_with_cache_manager(db_option: Arc<DbOptionExt>, branch_refnos: &[RefnoEnum], sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32) → anyhow::Result<BranchTubiOu...
```

### src\data_interface\db_meta_manager.rs
```
pub struct DbMetaManager
pub struct DbFileInfo
impl DbMetaManager
  pub fn global() → &'static DbMetaManager
  pub fn load(&self, path: impl AsRef<Path>) → Result<()>
  pub fn try_load_default(&self) → Result<()>
pub fn db_meta() → &'static DbMetaManager
pub fn get_dbnum(ref0: u32) → Option<u32>
pub fn ref0s_to_dbnums(ref0s: &[u32]) → Vec<u32>
pub fn generate_desi_indextree(ignore_manual_dbnum: bool) → anyhow::Result<()>
pub fn generate_single_indextree(target_dbnum: u32) → anyhow::Result<()>
```

### src\fast_model\export_model\export_dbnum_instances_web.rs
```
pub struct WebExportStats
impl NameTable
pub async fn export_dbnum_instances_web(dbnum: u32, output_dir: &Path, db_option: Arc<DbOption>, verbose: bool, root_refno: Option<RefnoEnum>, mesh_base_dir: Option<PathBuf>,) → Result<WebExportStats>
```

### src\fast_model\export_model\export_obj.rs
```
pub struct PreparedObjExport
pub struct ObjExporter
impl ObjExporter
  pub fn new() → Self
impl ObjExporter
impl ObjExporter
pub async fn prepare_obj_export(refnos: &[RefnoEnum], mesh_dir: &Path, config: &CommonExportConfig,) → Result<PreparedObjExport>
pub async fn export_obj_for_refnos(refnos: &[RefnoEnum], mesh_dir: &Path, output_path: &str, filter_nouns: Option<&[String]>, include_descendants: bool,) → Result<()>
```

### src\fast_model\export_model\export_transform_config.rs
```
pub struct ExportTransformConfig
impl ExportTransformConfig
impl ExportTransformConfig
  pub fn needs_unit_conversion(&self) → bool
  pub fn to_manifest_json(&self) → serde_json::Value
```

### src\fast_model\gen_model\db_meta_cache.rs
```
pub fn get_dbnum_for_refno(refno: RefnoEnum) → Option<u32>
```

### src\fast_model\gen_model\mesh_generate.rs
```
pub struct MeshWorkerReport
pub struct RecentGeoDeduper
pub struct MeshTask
pub struct MeshResult
impl MeshWorkerReport
  pub fn print_summary(&self)
impl RecentGeoDeduper
  pub fn new(_capacity: usize) → Self
  pub fn insert(&self, value: u64) → bool
  pub fn preload(&self, ids: impl IntoIterator<Item = u64>)
  pub fn len(&self) → usize
impl MeshResult
  pub fn failed() → Self
  pub fn to_update_sql(&self, geo_hash: &str) → String
  pub fn to_insert_fields(&self) → String
pub fn query_existing_meshed_inst_geo_ids() → Vec<u64>
pub fn extract_mesh_tasks(data: &ShapeInstancesData) → Vec<MeshTask>
pub async fn generate_meshes_for_batch(tasks: &[MeshTask], db_option: &DbOption, deduper: &RecentGeoDeduper, aabb_map: &Arc<DashMap<String, Aabb>>, pts_json_map: &Arc<DashMap<u64, String>>,) → HashMap<u64, MeshResult>
pub fn dedup_classify_tasks(tasks: &[MeshTask], deduper: &RecentGeoDeduper,) → (Vec<MeshTask>, HashSet<u64>)
pub async fn gen_meshes_in_db(option: Option<Arc<DbOption>>, refnos: &[RefnoEnum],) → anyhow::Result<()>
pub async fn run_mesh_worker(db_option: Arc<DbOption>, batch_size: usize) → anyhow::Result<()>
pub async fn run_mesh_worker_from_channel(receiver: flume::Receiver<Vec<MeshTask>>, db_option: Arc<DbOption>, sql_writer: Option<Arc<super::sql_file_writer::SqlFileWriter>>,) → anyhow::Result<MeshWorkerRe...
pub async fn run_boolean_worker(db_option: Arc<DbOption>, batch_size: usize) → anyhow::Result<()>
pub async fn booleans_meshes_in_db(option: Option<Arc<DbOption>>, refnos: &[RefnoEnum],) → anyhow::Result<()>
pub async fn process_meshes_update_db(option: Option<Arc<DbOption>>, refnos: &[RefnoEnum],) → anyhow::Result<()>
```

### src\fast_model\gen_model\precheck_coordinator.rs
```
pub struct PrecheckConfig
pub struct PrecheckStats
impl PrecheckConfig
pub async fn run_precheck(db_option: &DbOptionExt, config: Option<PrecheckConfig>,) → Result<PrecheckStats>
```

### src\fast_model\precheck.rs
```
pub async fn ensure_pe_transform_for_refnos(refnos: &[RefnoEnum]) → anyhow::Result<()>
```

### src\fast_model\gen_model\utilities.rs
```
pub fn is_e3d_debug_enabled() → bool
pub fn is_e3d_info_enabled() → bool
pub fn is_e3d_trace_enabled() → bool
pub async fn query_tubi_size(refno: RefnoEnum, tubi_cat_ref: RefnoEnum, is_hang: bool,) → Result<TubiSize>
pub async fn build_cata_hash_map_from_tree_by_dbnum(dbnum: u32, refnos: &[RefnoEnum],) → Result<DashMap<String, Cata...
pub async fn build_cata_hash_map_from_tree(refnos: &[RefnoEnum],) → Result<DashMap<String, Cata...
```

### src\web_api\platform_api\config.rs
```
pub struct PlatformConfig
impl PlatformConfig
impl PlatformConfig
  pub fn from_config_file() → Self
```

### src\web_api\review_integration.rs
```
pub struct AuxDataRequest
pub struct AuxDataResponse
pub struct AuxDataContent
pub struct CollisionItem
pub struct QualityItem
pub struct VerificationItem
pub struct RuleItem
pub struct ReviewIntegrationState
pub struct CollisionQueryParams
pub struct CollisionDataResponse
impl ReviewIntegrationState
pub fn create_review_integration_routes() → Router
```

### src\web_api\version_api.rs
```
pub struct VersionInfo
pub fn create_version_routes() → Router
```

### src\web_server\litefs_handlers.rs
```
pub struct NodeStatus
pub struct LiteFSStatus
pub async fn get_node_status() → Result<Json<Value>, StatusC...
pub async fn health_check() → Result<Json<Value>, StatusC...
pub async fn sync_status() → Result<Json<Value>, StatusC...
```

### src\web_server\web_listen.rs
```
pub fn init_web_listen(host: impl Into<String>, port: u16)
pub fn init_site_identity(runtime: WebServerRuntimeConfig)
pub fn get_web_listen() → Option<(&'static str, u16)>
pub fn current_site_id() → Option<String>
pub fn site_identity_json() → serde_json::Value
```

### src\cli_args.rs
```
pub fn add_export_instance_args(command: Command) → Command
pub fn add_init_project_subcommand(command: Command) → Command
```

### src\model_relation_store.rs
```
pub struct ModelRelationStore
pub struct InstRelateRecord
pub struct InstGeoRecord
pub struct StoreStats
impl ModelRelationStore
  pub fn new(base_path: impl AsRef<Path>) → Self
  pub fn db_dir(&self, dbnum: u32) → PathBuf
  pub fn cleanup_by_refnos(&self, dbnum: u32, refnos: &[RefnoEnum]) → Result<usize>
  pub fn insert_inst_relates(&self, dbnum: u32, records: &[InstRelateRecord]) → Result<()>
pub fn global_store() → &'static ModelRelationStore
```

### src\web_api\platform_api\cache_preload.rs
```
pub async fn preload_cache(Json(request) → impl IntoResponse
```

### src\fast_model\gen_model\query_compat.rs
```
pub async fn query_type_refnos_by_dbnum(nouns: &[&str], dbnum: u32, has_children: Option<bool>, _include_history: bool,) → anyhow::Result<Vec<RefnoEnum>>
pub async fn query_multi_children_refnos(refnos: &[RefnoEnum]) → anyhow::Result<Vec<RefnoEnum>>
pub async fn query_use_cate_refnos_by_dbnum(cate_names: &[&str], dbnum: u32, _include_history: bool,) → anyhow::Result<Vec<RefnoEnum>>
pub async fn get_children_pes(refno: RefnoEnum) → anyhow::Result<Vec<PE>>
pub async fn get_pe(refno: RefnoEnum) → anyhow::Result<Option<PE>>
pub async fn get_pes_batch(refnos: &[RefnoEnum]) → anyhow::Result<Vec<PE>>
pub async fn get_children_refnos(refno: RefnoEnum) → anyhow::Result<Vec<RefnoEnum>>
pub async fn query_visible_geo_descendants(refno: RefnoEnum, include_self: bool, range_str: Option<&str>,) → anyhow::Result<Vec<RefnoEnum>>
pub async fn query_negative_geo_descendants(refno: RefnoEnum, include_self: bool, range_str: Option<&str>,) → anyhow::Result<Vec<RefnoEnum>>
pub async fn query_deep_visible_inst_refnos(refno: RefnoEnum) → anyhow::Result<Vec<RefnoEnum>>
pub async fn query_deep_neg_inst_refnos(refno: RefnoEnum) → anyhow::Result<Vec<RefnoEnum>>
pub async fn query_deep_children_refnos(refno: RefnoEnum) → anyhow::Result<Vec<RefnoEnum>>
pub async fn query_filter_deep_children(refno: RefnoEnum, nouns: &[&str],) → anyhow::Result<Vec<RefnoEnum>>
pub async fn query_ancestor_refnos(refno: RefnoEnum) → anyhow::Result<Vec<RefnoEnum>>
pub async fn query_filter_ancestors(refno: RefnoEnum, nouns: &[&str],) → anyhow::Result<Vec<RefnoEnum>>
pub async fn query_filter_deep_children_atts(refno: RefnoEnum, nouns: &[&str],) → anyhow::Result<Vec<NamedAtt...
pub async fn collect_children_filter_ids(refno: RefnoEnum, nouns: &[&str],) → anyhow::Result<Vec<RefnoEnum>>
```

### src\fast_model\mod.rs
```
pub struct CaptureConfig
pub struct AabbCacheFileV1
pub struct AabbCacheEntryV1
impl CaptureConfig
  pub fn new(output_dir: PathBuf, width: u32, height: u32, include_descendants: bool, views: u8, baseline_dir: Option<PathBuf>, diff_dir: Option<PathBuf>,) → Self
pub fn set_capture_config(config: Option<CaptureConfig>)
pub fn get_capture_config() → Option<CaptureConfig>
pub fn save_aabb_cache_to_disk()
pub fn preload_mesh_cache()
pub fn set_debug_model_errors_only(enabled: bool)
pub fn is_debug_model_errors_only() → bool
pub fn is_error_message_heuristic(message: &str) → bool
```

### src\fast_model\export_model\export_prepack_lod.rs
```
pub struct InstancesMeta
pub struct InstancesData
pub struct ComponentGroup
pub struct HierarchyGroup
pub struct GeoEntry
pub struct TubingInstance
pub struct GeometryManifest
pub struct GeometryEntry
pub struct CompactGeoInstance
pub struct CompactEntry
pub struct CompactGroup
pub enum GeometryType
impl NameTable
impl ColorPalette
impl HashCollector
pub async fn export_prepack_lod_for_refnos(refnos: &[RefnoEnum], mesh_dir: &Path, output_dir: &Path, db_option: Arc<DbOption>, include_descendants: bool, filter_nouns: Option<Vec<String>>, verbose: bool, name_config: Option<&super::name_config::NameConfig>, export_all_lods: bool, source_length_unit: LengthUnit, target_length_unit: LengthUnit,) → Result<()>
pub async fn export_all_relates_prepack_lod(dbnum: Option<u32>, verbose: bool, output_override: Option<PathBuf>, owner_types: Option<Vec<String>>, name_config: Option<super::name_config::NameConfig>, db_option: Arc<DbOption>, export_all_lods: bool, export_refnos: Option<String>, source_unit: String, target_unit: String,) → Result<()>
pub async fn export_all_relates_prepack_lod_parquet(dbnum: Option<u32>, verbose: bool, output_override: Option<PathBuf>, owner_types: Option<Vec<String>>, name_config: Option<super::name_config::NameConfig>, db_option: Arc<DbOption>, export_all_lods: bool, export_refnos: Option<String>, source_unit: String, target_unit: String,) → Result<()>
pub async fn export_trans_aabb_incremental(output_dir: &Path, needed_trans_hashes: &HashSet<String>, needed_aabb_hashes: &HashSet<String>, unit_converter: &UnitConverter, verbose: bool,) → Result<(usize, usize, usize...
pub async fn export_dbnum_instances_json(dbnum: u32, output_dir: &Path, db_option: std::sync::Arc<DbOption>, verbose: bool, target_unit: Option<LengthUnit>, root_refno: Option<RefnoEnum>, detailed: bool,) → Result<ExportStats>
pub async fn export_dbnum_instances_json_from_cache(dbnum: u32, output_dir: &Path, cache_dir: &Path, mesh_dir: Option<&Path>, mesh_lod_tag: Option<&str>, verbose: bool, target_unit: Option<LengthUnit>, detailed: bool,) → Result<(ExportStats, usize,...
pub async fn export_global_trans_aabb_json(output_dir: &Path, target_unit: Option<LengthUnit>, verbose: bool,) → Result<(usize, usize)>
pub async fn export_instances_json_for_dbnos(dbnos: &[u32], _mesh_dir: &Path, output_dir: &Path, db_option: Arc<DbOption>, _verbose: bool,) → anyhow::Result<()>
pub async fn export_instances_json_for_refnos_grouped_by_dbno(refnos: &[RefnoEnum], _mesh_dir: &Path, output_dir: &Path, db_option: Arc<DbOption>, verbose: bool,) → anyhow::Result<()>
pub async fn export_instances_json_for_refnos_grouped_by_dbno_merge(refnos: &[RefnoEnum], mesh_dir: &Path, output_dir: &Path, db_option: Arc<DbOption>, verbose: bool,) → anyhow::Result<()>
```

### src\fast_model\export_model\export_rvm_semantic_debug.rs
```
pub struct SemanticDebugExportStats
impl ModelRelationStore
pub fn export_rvm_semantic_debug(dbnum: u32, relation_store_root: &Path, output_dir: &Path, root_refno: RefnoEnum, verbose: bool,) → Result<SemanticDebugExportS...
```

### src\init_project.rs
```
pub fn resolve_target_dbnums(cli_dbnums: Option<Vec<u32>>, discovered_dbnums: Vec<u32>,) → Result<Vec<u32>>
pub async fn run_init_project_mode(db_option_ext: DbOptionExt, cli_dbnums: Option<Vec<u32>>,) → Result<()>
```

### src\fast_model\room_worker.rs
```
pub struct RoomWorkerConfig
pub struct RoomWorkerTask
pub struct ProgressEvent
pub struct RoomWorker
pub enum RoomTaskType
pub enum RoomWorkerTaskStatus
impl RoomWorkerConfig
impl RoomWorkerTaskStatus
  pub fn is_terminal(&self) → bool
impl RoomWorkerTask
  pub fn new(id: String, task_type: RoomTaskType, db_option: DbOption) → Self
  pub fn with_priority(mut self, priority: u8) → Self
impl RoomWorker
  pub fn new(config: RoomWorkerConfig) → Self
  pub fn start(config: RoomWorkerConfig) → (Arc<Self>, JoinHandle<()>)
  pub fn stop(&self)
  pub async fn submit_task(&self, task: RoomWorkerTask) → String
  pub async fn cancel_task(&self, task_id: &str) → bool
  pub fn get_task_status(&self, task_id: &str) → Option<RoomWorkerTaskStatus>
  pub async fn queue_len(&self) → usize
  pub fn active_count(&self) → usize
```

### src\web_server\spatial_query_handlers.rs
```
pub async fn api_spatial_query(Json(request) → Response
pub async fn api_spatial_stats() → Response
```

### src\versioned_db\database.rs
```
pub enum SenderJsonsData
pub trait MySqlMethods
pub async fn create_project_database(project: &str, url: &str) → anyhow::Result<()>
pub async fn create_info_database(db_option: &DbOption) → anyhow::Result<()>
pub async fn sync_pdms_with_callback(db_option: &DbOption, mut progress_callback: Option<F>,) → anyhow::Result<()> where F:...
pub async fn sync_pdms(db_option: &DbOption) → anyhow::Result<()>
pub async fn define_dbnum_event() → anyhow::Result<()>
pub async fn define_dbnum_event() → anyhow::Result<()>
pub async fn define_dbnum_event_array_id() → anyhow::Result<()>
pub async fn define_dbnum_event_array_id() → anyhow::Result<()>
pub async fn execute_sql(conn: &Pool<MySql>, sql: &str) → bool
pub async fn sync_total_async_threaded_with_callback(db_option: &DbOption, project: &str, cur_dbno_set: Arc<DashSet<u32>>, db_types: &[&str], proj_progress_chunk: usize, progress_callback: &mut Option<F>, current_project: usize, total_projects: usize,) → anyhow::Result<()> where F:...
pub async fn sync_total_async_threaded(db_option: &DbOption, project: &str, cur_dbno_set: Arc<DashSet<u32>>, db_types: &[&str], proj_progress_chunk: usize,) → anyhow::Result<()>
pub async fn parse_single_db_file(db_option: &DbOption, project_name: &str, file_path: &str, target_dbnum: u32,) → anyhow::Result<()>
```

### src\web_api\platform_api\delete_handler.rs
```
pub async fn delete_review_data(Json(request) → impl IntoResponse
```

### src\web_server\admin_auth_handlers.rs
```
pub struct AdminSession
pub struct LoginRequest
pub fn create_admin_auth_routes() → Router
pub fn validate_token(token: &str) → Option<AdminSession>
pub async fn admin_auth_middleware(request: Request<Body>, next: Next) → Response
pub async fn admin_session_middleware(request: Request<Body>, next: Next) → Response
pub fn cleanup_expired_sessions()
pub fn start_session_cleanup_timer()
```

### src\fast_model\gen_model\loop_model.rs
```
pub async fn gen_loop_geos(db_option: Arc<DbOptionExt>, loop_owner_refnos: &[RefnoEnum], sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32) → anyhow::Result<bool>
```

### src\api\children.rs
```
pub struct SimpleNodeDataForPlat
impl AiosDBManager
  pub fn get_ancestor_refno_of_type_data(&self, mut refno: RefU64, att_type: &str) → anyhow::Result<RefU64>
  pub fn get_ancestor_refno_till_type(&self, mut refno: RefU64, att_types: &[&str]) → Option<RefU64>
  pub fn traverse_ancestor(&self, mut refno: RefU64, func: impl Fn(RefU64) → bool) -> Option<RefU64>
  pub async fn traverse_foreign(&self, mut refno: RefU64, foreigns: &[&str], func: impl Fn(RefU64) → bool) -> Option<RefU64>
  pub async fn query_children_eles_order(&self, refno: RefU64, filter: &[&str], db_types: &[&str],) → anyhow::Result<Vec<PdmsElem...
pub async fn travel_children_eles(refno: RefU64, pool: &Pool<MySql>) → anyhow::Result<Vec<RefU64>>
pub async fn travel_children_without_leaf(refno: RefU64, pool: &Pool<MySql>) → anyhow::Result<Vec<RefU64>>
pub async fn travel_children_for_elenode(refno: RefU64, pool: &Pool<MySql>) → anyhow::Result<Vec<EleTreeN...
pub async fn travel_children_for_elenode_without_children_count(refno: RefU64, pool: &Pool<MySql>) → anyhow::Result<Vec<EleTreeN...
pub async fn travel_children_with_type(refno: RefU64, att_type: String, pool: &Pool<MySql>) → anyhow::Result<Vec<EleTreeN...
pub async fn travel_children_with_refno(refno: RefU64, pool: &Pool<MySql>) → anyhow::Result<Vec<RefU64>>
pub async fn query_children_id_name_with_type(pool: &Pool<MySql>, refno: RefU64, att_type: &str) → anyhow::Result<Vec<(RefU64,...
pub async fn fuzzy_query_refnos_by_name(att_type: String, name: String, pool: &Pool<MySql>) → anyhow::Result<Vec<(RefU64,...
pub async fn fuzzy_query_refnos_by_name_limit(name: String, numbdbs: &BTreeSet<i32>, pool: &Pool<MySql>) → anyhow::Result<Vec<(RefU64,...
pub async fn query_numbdb_from_refnos(refnos: Vec<RefU64>, pool: &Pool<MySql>) → anyhow::Result<Vec<i32>>
pub async fn query_db_num_by_refno(refno: RefU64, pool: &Pool<MySql>) → anyhow::Result<i32>
pub async fn query_owner_type_from_id(refno: RefU64, pool: &Pool<MySql>) → anyhow::Result<Option<(RefU...
pub async fn query_ancestor_of_type(mut refno: RefU64, att_type: &str, pool: &Pool<MySql>) → anyhow::Result<Option<RefU64>>
pub fn query_ancestor_of_type_from_cache(refno: RefU64, att_type: &str) → Option<(RefU64, String)>
pub async fn query_ancestor_refnos_till_type(mut refno: RefU64, att_type: &str, pool: &Pool<MySql>) → anyhow::Result<Vec<RefU64>>
pub async fn query_ancestor_refnos_till_type_aql(database: &ArDatabase, mut refno: RefU64, att_type: &str) → anyhow::Result<Vec<RefU64>>
pub async fn query_children_contains_types(refno: RefU64, pool: &Pool<MySql>) → anyhow::Result<Option<Vec<S...
pub async fn query_owner_till_type(mut refno: RefU64, types: Vec<String>, pool: &Pool<MySql>) → anyhow::Result<RefU64>
```

### src\cli_modes.rs
```
pub struct ExportConfig
pub struct RoomComputeCliConfig
pub struct SpatialQueryVerifyResultItem
pub struct SpatialQueryVerifySnapshot
impl ExportConfig
impl ExportConfig
  pub fn new(refnos_str: Vec<String>) → Self
  pub fn with_output_path(mut self, output_path: Option<String>) → Self
  pub fn with_filter_nouns(mut self, filter_nouns: Option<Vec<String>>) → Self
  pub fn with_include_descendants(mut self, include_descendants: bool) → Self
  pub fn with_unit_conversion(mut self, source_unit: &str, target_unit: &str) → Self
  pub fn with_verbose(mut self, verbose: bool) → Self
  pub fn with_regenerate_plant_mesh(mut self, regenerate_plant_mesh: bool) → Self
  pub fn with_run_all_dbnos(mut self, run_all_dbnos: bool) → Self
impl RoomVerifySummary
impl ScopedEnvVar
impl ScopedEnvVar
impl RoomComputeCliReport
pub fn kill_process_on_port(port: u16)
pub async fn ensure_surreal_connected(db_option_ext: &DbOptionExt) → Result<()>
pub async fn room_verify_json_mode(input: &Path, db_option_ext: &DbOptionExt) → Result<()>
pub async fn room_verify_json_mode(_input: &Path, _db_option_ext: &DbOptionExt) → Result<()>
pub async fn run_generate_model(config: &ExportConfig, db_option_ext: &DbOptionExt,) → Result<aios_database::fast_...
pub async fn run_regen_model(config: &ExportConfig, db_option_ext: &DbOptionExt,) → Result<aios_database::fast_...
pub async fn export_obj_mode(config: ExportConfig, db_option_ext: &DbOptionExt) → Result<()>
```

### src\data_interface\increment_manager.rs
```
pub struct IncrementInfo
impl IncrementInfo
  pub fn is_modified(&self) → bool
  pub fn is_deleted(&self) → bool
  pub fn is_added(&self) → bool
impl AiosDBManager
  pub async fn execute_incr_update(&self, increment_ranges_map: IndexMap<PathBuf, (DbPageBasicInfo, RangeInclusive<i32>) → anyhow::Result<bool>
  pub async fn init_watcher(&self) → anyhow::Result<()>
```

### src\data_interface\tidb_manager.rs
```
pub struct AiosDBManager
impl AiosDBManager
impl AiosDBManager
```

### src\fast_model\export_model\export_common.rs
```
pub struct PrimitiveSegment
pub struct GeometryInstance
pub struct ComponentRecord
pub struct TubiRecord
pub struct GltfMeshCache
pub struct ExportData
pub struct InstRelateRow
pub struct InstRelateAabbRow
impl GltfMeshCache
  pub fn new() → Self
  pub fn load_or_get(&self, geo_hash: &str, mesh_dir: &Path) → Result<Arc<PlantMesh>>
  pub fn cache_stats(&self) → (usize, usize, usize)
pub fn sanitize_node_name(name: &str) → String
pub fn trim_leading_slash(name: &str) → String
pub async fn collect_export_data(geom_insts: Vec<GeomInstQuery>, _refnos: &[RefnoEnum], mesh_dir: &Path, verbose: bool, bran_roots: Option<&[RefnoEnum]>, tubi_use_inst_world_only: bool,) → Result<ExportData>
pub async fn query_inst_relate_batch(refnos: &[RefnoEnum], include_name: bool, verbose: bool,) → Result<Vec<InstRelateRow>>
pub async fn query_inst_relate_aabb_batch(refnos: &[RefnoEnum], verbose: bool,) → Result<std::collections::Ha...
```

### src\fast_model\export_model\export_dbnum_instances_v3.rs
```
pub struct V3ExportStats
pub struct V3MergeStats
pub async fn export_dbnum_instances_v3(dbnum: u32, output_dir: &Path, db_option: Arc<DbOption>, verbose: bool, transform_config: ExportTransformConfig, root_refno: Option<RefnoEnum>,) → Result<V3ExportStats>
pub async fn export_all_instances_v3(output_dir: &Path, db_option: Arc<DbOption>, verbose: bool, transform_config: ExportTransformConfig,) → Result<V3ExportStats>
pub fn merge_v3_instances(v3_bundle_dir: &Path, verbose: bool) → Result<V3MergeStats>
```

### src\fast_model\gen_model\cata_model.rs
```
pub struct CateGenOutcome
pub struct BranchTubiOutcome
pub struct GenOutcome
pub struct BranchPrefetchResult
pub struct BranchMetaPrefetched
pub enum NgmrRemovedType
pub fn init_chrome_tracing() → anyhow::Result<()>
pub async fn prefetch_tubi_size_and_branch_meta(all_child_refnos: &[RefnoEnum], branch_refnos: &[RefnoEnum],) → anyhow::Result<BranchPrefet...
pub async fn gen_cata_geos(db_option: Arc<DbOptionExt>, target_cata_map: Arc<DashMap<String, CataHashRefnoKV>>, branch_map: Arc<DashMap<RefnoEnum, Vec<SPdmsElement>>>, sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32) → anyhow::Result<bool>
pub async fn gen_cata_instances(db_option: Arc<DbOptionExt>, target_cata_map: Arc<DashMap<String, CataHashRefnoKV>>, sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32) → anyhow::Result<CateGenOutcome>
pub async fn gen_branch_tubi(db_option: Arc<DbOptionExt>, branch_map: Arc<DashMap<RefnoEnum, Vec<SPdmsElement>>>, sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)
pub async fn gen_branch_tubi_from_db_with_prefetch(db_option: Arc<DbOptionExt>, branch_map: Arc<DashMap<RefnoEnum, Vec<SPdmsElement>>>, sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)
pub async fn gen_branch_tubi_from_db(db_option: Arc<DbOptionExt>, branch_map: Arc<DashMap<RefnoEnum, Vec<SPdmsElement>>>, sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)
pub async fn gen_branch_tubi_cache_only(db_option: Arc<DbOptionExt>, branch_map: Arc<DashMap<RefnoEnum, Vec<SPdmsElement>>>, sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)
pub async fn query_ngmr_owner(refno: RefnoEnum, ngmr_geo_refno: RefnoEnum,) → anyhow::Result<Vec<RefnoEnum>>
pub async fn gen_tubi_from_db(db_option: Arc<DbOptionExt>, branch_refnos: &[RefnoEnum], sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32) → anyhow::Result<BranchTubiOu...
```

### src\fast_model\gen_model\pdms_inst.rs
```
pub struct SaveInstanceDataReport
pub struct InstRelatePrecomputed
impl TransactionBatcher
impl InstRelatePrecomputed
  pub async fn build(refnos: &[RefnoEnum]) → Self
pub async fn save_tubi_info_batch_with_replace(tubi_info_map: &dashmap::DashMap<String, TubiInfoData>, _replace_exist: bool,) → anyhow::Result<usize>
pub async fn pre_cleanup_for_regen(seed_refnos: &[RefnoEnum]) → anyhow::Result<()>
pub async fn save_instance_data_optimize(inst_mgr: &ShapeInstancesData, replace_exist: bool, mesh_results: &HashMap<u64, MeshResult>, mesh_aabb_map: &DashMap<String, Aabb>,) → anyhow::Result<()>
pub async fn save_instance_data_with_options(inst_mgr: &ShapeInstancesData, replace_exist: bool, mesh_results: &HashMap<u64, MeshResult>, mesh_aabb_map: &DashMap<String, Aabb>, write_inst_relate_aabb: bool,) → anyhow::Result<()>
pub async fn save_instance_data_with_report(inst_mgr: &ShapeInstancesData, replace_exist: bool, mesh_results: &HashMap<u64, MeshResult>, mesh_aabb_map: &DashMap<String, Aabb>, write_inst_relate_aabb: bool,) → anyhow::Result<SaveInstance...
pub fn build_inst_relate_aabb_rows(inst_mgr: &ShapeInstancesData, mesh_results: &HashMap<u64, MeshResult>, mesh_aabb_map: &DashMap<String, Aabb>,) → anyhow::Result<(HashMap<u64...
pub async fn save_inst_relate_aabb_rows(aabb_map: &HashMap<u64, String>, inst_relate_aabb_rows: &[String], inst_relate_aabb_ids: &[String],) → anyhow::Result<()>
pub async fn save_tubi_info_batch(tubi_info_map: &DashMap<String, TubiInfoData>,) → anyhow::Result<usize>
pub async fn reconcile_missing_neg_relate(all_refnos: &[RefnoEnum], candidate_carriers: &[RefnoEnum],) → anyhow::Result<usize>
pub async fn save_instance_data_to_sql_file(inst_mgr: &ShapeInstancesData, replace_exist: bool, writer: &SqlFileWriter, precomputed: &InstRelatePrecomputed, mesh_results: &HashMap<u64, MeshResult>, mesh_aabb_map: &DashMap<String, Aabb>,) → anyhow::Result<()>
```

### src\fast_model\gen_model\refno_assoc_index.rs
```
pub struct RefnoAssocIndexEntry
pub struct RefnoAssocIndexBatch
pub struct RefnoAssocDeleteSummary
impl RefnoAssocIndexEntry
impl RefnoAssocIndexBatch
  pub fn is_empty(&self) → bool
  pub fn add_inst_relate_id(&mut self, refno: RefnoEnum, id: String)
  pub fn add_inst_info_id(&mut self, refno: RefnoEnum, id: String)
  pub fn add_geo_relate_id(&mut self, refno: RefnoEnum, id: String)
  pub fn add_geo_hash(&mut self, refno: RefnoEnum, hash: String)
  pub fn add_neg_relate_id(&mut self, refno: RefnoEnum, id: String)
  pub fn add_ngmr_relate_id(&mut self, refno: RefnoEnum, id: String)
  pub fn add_inst_relate_bool_id(&mut self, refno: RefnoEnum, id: String)
pub async fn build_delete_sql_by_refnos(refnos: &[RefnoEnum], chunk_size: usize,) → anyhow::Result<Option<Vec<S...
pub async fn delete_by_refnos(refnos: &[RefnoEnum], chunk_size: usize,) → anyhow::Result<RefnoAssocDe...
```

### src\fast_model\room_model.rs
```
pub struct RoomBuildStats
pub struct RoomComputeOptions
pub struct CoarseAabbDiagnostic
pub struct IncrementalUpdateResult
impl SpatialIndexScope
impl SpatialIndexSpecResolver
impl SpatialIndexSpecResolver
impl RoomComputeOptions
impl RoomComputeOptions
  pub fn with_prebuilt_spatial_index(mut self) → Self
  pub fn refresh_spatial_index_enabled(&self) → bool
  pub fn with_surreal_query(mut self) → Self
  pub fn query_from_cache_enabled(&self) → bool
  pub fn with_preload_panel_meshes(mut self, enabled: bool) → Self
  pub fn preload_enabled(&self) → bool
impl Floor2dConfig
impl CacheMetrics
impl PanelProcessOutcome
impl ComputedRoomRelations
pub async fn refresh_sqlite_spatial_index_from_inst_relate_aabb(db_nums: Option<&[u32]>, refno_root: Option<RefnoEnum>,) → anyhow::Result<usize>
pub async fn build_room_relations(db_option: &DbOption, db_nums: Option<&[u32]>, refno_root: Option<RefnoEnum>,) → anyhow::Result<RoomBuildStats>
pub async fn build_room_relations_with_model_generation(db_option: &DbOption, db_nums: Option<&[u32]>, refno_root: Option<RefnoEnum>,) → anyhow::Result<RoomBuildStats>
pub async fn build_room_relations_with_overrides(db_option: &DbOption, db_nums: Option<&[u32]>, refno_root: Option<RefnoEnum>, room_keywords_override: Option<&[String]>, force_rebuild: bool,) → anyhow::Result<RoomBuildStats>
pub async fn build_room_relations_with_cancel(db_option: &DbOption, db_nums: Option<&[u32]>, refno_root: Option<RefnoEnum>, cancel_token: Option<CancellationToken>, progress_callback: Option<Box<dyn Fn(f32, &str) → anyhow::Result<RoomBuildStats>
pub async fn build_room_panels_relate_for_query(room_key_word: &Vec<String>,) → anyhow::Result<Vec<(RefnoEn...
```

### src\sqlite_index.rs
```
pub struct SqliteAabbIndex
pub struct ImportConfig
pub struct ImportStats
impl SqliteAabbIndex
  pub fn path(&self) → &Path
  pub fn open(path: P) → Result<Self>
  pub fn init_schema(&self) → Result<()>
  pub fn insert_many(&self, iter: I) → Result<usize> where I: Into...
  pub fn query_intersect(&self, minx: f64, maxx: f64, miny: f64, maxy: f64, minz: f64, maxz: f64,) → Result<Vec<i64>>
  pub fn query_range_x(&self, minx: f64, maxx: f64) → Result<Vec<i64>>
  pub fn query_all_aabbs(&self) → Result<Vec<(i64, f64, f64, ...
  pub fn insert_items(&self, iter: I) → Result<usize> where I: Into...
impl ImportConfig
impl SqliteAabbIndex
  pub fn import_from_instances_json(&self, json_path: &Path, config: &ImportConfig,) → anyhow::Result<ImportStats>
  pub fn import_from_json_value(&self, json: &serde_json::Value, config: &ImportConfig,) → anyhow::Result<ImportStats>
pub fn refno_str_to_i64(refno: &str) → Option<i64>
pub fn i64_to_refno_str(id: i64) → String
```

### src\web_api\mod.rs
```
pub fn create_mbd_pipe_routes() → axum::Router
pub fn assemble_stateless_web_api_routes() → axum::Router
pub fn stateless_web_api_route_paths() → Vec<&'static str>
```

### src\web_api\pdms_transform_api.rs
```
pub struct TransformResponse
pub struct ComputeTransformResponse
pub fn create_pdms_transform_routes() → Router
```

### src\web_api\platform_api\mod.rs
```
pub fn create_platform_api_routes() → Router
```

### src\web_api\ptset_api.rs
```
pub struct PtsetPoint
pub struct PtsetResponse
pub struct PtsetUnitInfo
pub struct PtsetQuery
pub struct PtsetBatchQueryRequest
pub struct PtsetBatchItemResult
pub struct PtsetBatchQueryResponse
pub fn create_ptset_routes() → Router
```

### src\web_server\admin_handlers.rs
```
pub struct AdminAppConfig
pub struct PortCheckQuery
pub struct LogsTailQuery
pub fn create_admin_routes() → Router
pub async fn list_sites() → impl IntoResponse
pub async fn get_resource_summary() → impl IntoResponse
pub async fn get_app_config() → impl IntoResponse
pub async fn check_port(Query(params) → impl IntoResponse
pub async fn create_site(Json(payload) → impl IntoResponse
pub async fn preview_parse_plan(Json(payload) → impl IntoResponse
pub async fn get_site(Path(site_id) → impl IntoResponse
pub async fn update_site(Path(site_id) → impl IntoResponse
pub async fn delete_site(Path(site_id) → impl IntoResponse
pub async fn parse_site(Path(site_id) → impl IntoResponse
pub async fn start_site(Path(site_id) → impl IntoResponse
pub async fn stop_site(Path(site_id) → impl IntoResponse
pub async fn restart_site(Path(site_id) → impl IntoResponse
pub async fn get_site_runtime(Path(site_id) → impl IntoResponse
pub async fn get_site_logs(Path(site_id) → impl IntoResponse
pub async fn get_site_log_kind(Path((site_id, kind) → impl IntoResponse
pub async fn download_site_log(Path((site_id, kind) → Response
```

### src\web_server\admin_registry_handlers.rs
```
pub struct AdminRegistryTaskRequest
pub fn create_admin_registry_routes() → Router<AppState>
```

### src\web_server\admin_response.rs
```
pub fn ok(message: impl Into<String>, data: T) → ApiResponse
pub fn accepted(message: impl Into<String>, data: T) → ApiResponse
pub fn not_found(message: impl Into<String>) → ApiResponse
pub fn server_error(message: impl Into<String>) → ApiResponse
pub fn conflict(message: impl Into<String>) → ApiResponse
pub fn service_unavailable(message: impl Into<String>) → ApiResponse
pub fn unauthorized(message: impl Into<String>) → ApiResponse
pub fn bad_request(message: impl Into<String>) → ApiResponse
pub fn response(status: StatusCode, success: bool, message: impl Into<String>, data: Option<T>,) → ApiResponse where T: Serial...
pub fn classify_error_status(message: &str) → StatusCode
pub fn managed_error(message: String) → ApiResponse
```

### src\web_server\admin_task_handlers.rs
```
pub fn cleanup_old_tasks()
pub fn insert_task(task: TaskInfo)
pub fn create_and_dispatch_site_task(site_id: String, task_name: String, task_type: TaskType, priority: TaskPriority, config: DatabaseConfig,) → Result<TaskInfo, String>
pub fn create_admin_task_routes() → Router
```

### src\web_server\collab_migrations.rs
```
pub fn ensure_collab_schema()
```

### src\web_server\incremental_update_handlers.rs
```
pub struct IncrementalUpdateInfo
pub struct ChangedFile
pub struct ArchiveFile
pub struct UpdateConfigRequest
pub enum UpdateDetectionStatus
pub enum ChangeType
pub async fn list_incremental_archives() → Result<Json<serde_json::Val...
pub async fn get_all_incremental_status(_state: State<AppState>,) → Result<Json<serde_json::Val...
pub async fn get_site_incremental_details(_state: State<AppState>, Path(site_id) → Result<Json<serde_json::Val...
pub async fn start_incremental_detection(_state: State<AppState>, Path(site_id) → Result<Json<serde_json::Val...
pub async fn start_incremental_sync(_state: State<AppState>, Path(site_id) → Result<Json<serde_json::Val...
pub async fn get_detection_task_status(_state: State<AppState>, Path(task_id) → Result<Json<serde_json::Val...
pub async fn cancel_task(_state: State<AppState>, Path(task_id) → Result<Json<serde_json::Val...
pub async fn get_incremental_config(_state: State<AppState>,) → Result<Json<serde_json::Val...
pub async fn update_incremental_config(_state: State<AppState>, Json(config) → Result<Json<serde_json::Val...
```

### src\web_server\models.rs
```
pub struct TaskInfo
pub struct TaskProgress
pub struct LogEntry
pub struct ErrorDetails
pub struct DatabaseConfig
pub struct E3dProjectInfo
pub struct DeploymentSite
pub struct DeploymentSiteCreateRequest
pub struct DeploymentSiteImportRequest
pub struct DeploymentSiteUpdateRequest
pub struct DeploymentSiteQuery
pub struct DeploymentSiteTaskRequest
pub struct ManagedSiteParseHealth
pub struct ManagedSiteParsePlan
pub struct ManagedProjectSite
pub struct ManagedSiteProcessResource
pub struct ManagedSiteResourceMetrics
pub struct CreateManagedSiteRequest
pub struct UpdateManagedSiteRequest
pub struct PreviewManagedSiteParsePlanRequest
pub struct ManagedSiteRuntimeStatus
pub struct AdminResourceSummary
pub struct ManagedSiteActivitySummary
pub struct ManagedSiteLogStreamSummary
pub struct ManagedSiteLogsResponse
```

### src\web_server\model_runtime.rs
```
pub struct RealtimeInstancesRequest
pub struct ParquetIncrementalEnqueueRequest
pub fn ensure_runtime_started()
pub async fn api_realtime_instances_by_refnos(Json(payload) → impl IntoResponse
pub async fn api_parquet_incremental_enqueue(Json(payload) → impl IntoResponse
pub async fn api_parquet_version(Path(dbno) → impl IntoResponse
```

### src\web_server\remote_sync_handlers.rs
```
pub struct RemoteSyncEnv
pub struct RemoteSyncSite
pub struct RemoteSyncLogRecord
pub struct MetadataQuery
pub struct EnvCreateRequest
pub struct SiteCreateRequest
pub struct LogQueryParams
pub struct DailyStatsQuery
pub struct FlowStatsQuery
pub struct RemoteSyncActiveTask
pub struct RemoteSyncFailedTask
pub struct RemoteSyncEnvConfig
pub struct ListActiveTasksQuery
pub struct ListFailedTasksQuery
pub struct CleanupFailedTasksQuery
pub enum RemoteSyncEvent
impl RemoteSyncEnvConfig
pub fn emit_remote_sync_event(event: RemoteSyncEvent)
pub fn create_remote_sync_routes() → Router
pub async fn remote_sync_page() → Html<String>
pub fn open_sqlite() → Result<rusqlite::Connection...
pub async fn list_envs() → Result<Json<serde_json::Val...
pub async fn create_env(Json(req) → Result<Json<serde_json::Val...
pub async fn get_env(Path(id) → Result<Json<serde_json::Val...
pub async fn update_env(Path(id) → Result<Json<serde_json::Val...
```

### src\web_server\mqtt_monitor_handlers.rs
```
pub struct MqttNodeStatus
pub struct MessageDeliveryStatus
pub struct ReceiverStatus
pub struct BrokerLogEntry
pub enum ReceiverProcessStatus
pub async fn push_broker_log(level: &str, event: &str, location: Option<&str>, message: impl Into<String>,)
pub async fn read_broker_logs(limit: Option<usize>) → Vec<BrokerLogEntry>
pub async fn update_node_heartbeat(location: String, node_name: String, subscribed_topics: Vec<String>,)
pub async fn update_subscription_status(location: String, node_name: String, connected: bool)
pub async fn record_message_received(location: String, message_id: String)
pub async fn record_message_sent(message_id: String, sender_location: String, session_range: Option<String>, file_count: usize, expected_receivers: Vec<String>,)
pub async fn check_offline_nodes(timeout_secs: i64)
pub async fn get_mqtt_nodes_status(_state: State<AppState>,) → Result<Json<serde_json::Val...
pub async fn remove_mqtt_node(axum::extract::Path(location) → Result<Json<serde_json::Val...
pub async fn client_unsubscribed(Json(payload) → Result<Json<serde_json::Val...
pub async fn check_site_http_status(http_host: Option<&str>) → bool
pub async fn get_message_delivery_status(_state: State<AppState>, Query(params) → Result<Json<serde_json::Val...
pub async fn get_message_delivery_detail(_state: State<AppState>, axum::extract::Path(message_id) → Result<Json<serde_json::Val...
pub async fn cleanup_old_messages()
```

### src\web_server\room_api.rs
```
pub struct RoomApiState
pub struct RoomTaskManager
pub struct RoomComputeTask
pub struct RoomComputeConfig
pub struct ValidationOptions
pub struct ModelGenerationOptions
pub struct RoomComputeResult
pub struct RoomStatistics
pub struct CreateRoomTaskRequest
pub struct RoomQueryRequest
pub struct RoomCodeRequest
pub struct BatchRoomQueryRequest
pub struct RoomQueryResponse
pub struct BatchRoomQueryResponse
pub struct RoomCodeResponse
pub struct RoomCodeProcessResult
pub struct RoomSystemStatusResponse
pub struct CacheStatus
pub enum RoomTaskType
pub enum TaskStatus
pub enum ModelOutputFormat
pub enum ModelQuality
pub fn init_room_worker() → Arc<RoomWorker>
pub async fn create_room_task(State(state) → Result<Json<RoomComputeTask...
pub async fn get_task_status(State(state) → Result<Json<RoomComputeTask...
```

### src\web_server\site_config_handlers.rs
```
pub struct SiteConfig
pub async fn get_server_ip(_state: State<crate::web_server::AppState>,) → Result<Json<serde_json::Val...
pub async fn get_site_config(_state: State<crate::web_server::AppState>,) → Result<Json<serde_json::Val...
pub async fn get_site_info(_state: State<crate::web_server::AppState>,) → Result<Json<serde_json::Val...
pub async fn save_site_config(state: State<crate::web_server::AppState>, Json(config) → Result<Json<serde_json::Val...
pub async fn restart_server(state: State<crate::web_server::AppState>,) → Result<Json<serde_json::Val...
pub async fn reload_site_config(state: State<crate::web_server::AppState>,) → Result<Json<serde_json::Val...
pub async fn validate_site_config(_state: State<crate::web_server::AppState>, Json(config) → Result<Json<serde_json::Val...
```

### src\web_server\site_registry.rs
```
pub struct WebServerRuntimeConfig
pub fn ensure_registry_schema() → Result<()>
pub fn list_sites(query: Option<&DeploymentSiteQuery>) → Result<Vec<DeploymentSite>>
pub fn get_site(id: &str) → Result<Option<DeploymentSite>>
pub fn create_site(mut site: DeploymentSite) → Result<DeploymentSite>
pub fn update_site(id: &str, req: &super::models::DeploymentSiteUpdateRequest,) → Result<DeploymentSite>
pub fn delete_site(id: &str) → Result<bool>
pub fn update_health(site_id: &str, status: DeploymentSiteStatus, timestamp: &str,) → Result<DeploymentSite>
pub fn mark_site_status(site_id: &str, status: DeploymentSiteStatus) → Result<()>
pub fn upsert_runtime_site(runtime: &WebServerRuntimeConfig) → Result<DeploymentSite>
pub fn load_web_server_runtime_config(explicit_port: u16) → WebServerRuntimeConfig
```

### src\web_server\sse_handlers.rs
```
pub enum SyncEvent
impl SyncEvent
pub async fn sync_events_handler() → impl IntoResponse
pub async fn test_sse_handler() → impl IntoResponse
pub fn push_admin_site_snapshot(site_id: &str, project_name: Option<&str>, status: &str, parse_status: &str, last_error: Option<&str>,)
pub fn push_admin_site_created(site_id: &str, project_name: &str)
pub fn push_admin_site_deleted(site_id: &str)
```

### src\web_server\sync_control_handlers.rs
```
pub struct StartSyncRequest
pub struct TestConnectionRequest
pub struct AddTaskRequest
pub struct EventQuery
pub struct QueueQuery
pub struct HistoryQuery
pub struct StartMqttRequest
pub struct MetricsHistoryQuery
pub struct StartSubscriptionRequest
pub struct SetNodeRequest
pub async fn trigger_file_download(_state: State<AppState>, Json(request) → Result<Json<serde_json::Val...
pub async fn trigger_file_download(_state: State<AppState>, Json(_request) → Result<Json<serde_json::Val...
pub async fn start_sync_service(_state: State<AppState>, Json(request) → Result<Json<serde_json::Val...
pub async fn stop_sync_service(_state: State<AppState>,) → Result<Json<serde_json::Val...
pub async fn restart_sync_service(_state: State<AppState>,) → Result<Json<serde_json::Val...
pub async fn pause_sync_service(_state: State<AppState>,) → Result<Json<serde_json::Val...
pub async fn resume_sync_service(_state: State<AppState>,) → Result<Json<serde_json::Val...
pub async fn get_sync_status(_state: State<AppState>,) → Result<Json<serde_json::Val...
pub async fn sync_events_stream(_state: State<AppState>, Query(params) → Result<Json<serde_json::Val...
pub async fn get_sync_metrics(_state: State<AppState>,) → Result<Json<serde_json::Val...
pub async fn get_sync_queue(_state: State<AppState>, Query(params) → Result<Json<serde_json::Val...
pub async fn get_sync_config(_state: State<AppState>,) → Result<Json<serde_json::Val...
pub async fn update_sync_config(_state: State<AppState>, Json(config) → Result<Json<serde_json::Val...
pub async fn test_sync_connection(_state: State<AppState>, Json(request) → Result<Json<serde_json::Val...
pub async fn add_sync_task(_state: State<AppState>, Json(request) → Result<Json<serde_json::Val...
```

### src\web_api\review_annotation_state.rs
```
pub struct ApplyAnnotationStateRequest
pub struct ApplyAnnotationStateResponse
pub struct AnnotationStateView
pub struct QueryAnnotationStatesRequest
pub struct QueryAnnotationStatesResponse
pub fn create_annotation_state_routes() → Router
pub async fn sync_annotation_states_from_snapshot(form_id: &str, task_id: &str, current_node: &str, operator_id: &str, operator_name: &str, operator_role: &str, annotations: &[Value], cloud_annotations: &[Value], rect_annotations: &[Value],)
pub async fn load_annotation_states_by_task(form_id: &str, task_id: &str,) → Result<Vec<AnnotationStateV...
pub async fn delete_annotation_states_by_form_id(form_id: &str) → Result<(), String>
```

### src\web_api\platform_api\auth.rs
```
pub fn verify_s2s_token_with_claims(token: &str,) → Result<Option<TokenClaims>,...
pub fn verify_s2s_token(token: &str) → Result<(), (StatusCode, Str...
```

### src\web_api\platform_api\review_form.rs
```
pub async fn get_review_form_by_form_id(form_id: &str) → anyhow::Result<Option<Revie...
pub async fn ensure_review_form_stub(form_id: &str, project_id: &str, requester_id: &str, requester_role: Option<&str>, source: &str,) → anyhow::Result<ReviewForm>
pub async fn sync_review_form_with_task_status(form_id: &str, project_id: Option<&str>, requester_id: Option<&str>, source: &str, task_status: &str,) → anyhow::Result<()>
pub async fn mark_review_form_deleted(form_id: &str) → anyhow::Result<()>
pub async fn soft_delete_review_bundle(form_id: &str) → anyhow::Result<()>
pub async fn find_task_by_form_id(form_id: &str) → anyhow::Result<Option<Revie...
```

### src\web_server\managed_project_sites.rs
```
pub struct RuntimeUpdate
pub struct StopSiteResult
pub struct TailLogResponse
pub fn ensure_schema() → Result<()>
pub fn get_site(site_id: &str) → Result<Option<ManagedProjec...
pub fn list_sites() → Result<Vec<ManagedProjectSi...
pub fn create_site(req: CreateManagedSiteRequest) → Result<ManagedProjectSite>
pub fn preview_parse_plan(req: PreviewManagedSiteParsePlanRequest) → Result<ManagedSiteParsePlan>
pub fn update_site(site_id: &str, req: UpdateManagedSiteRequest) → Result<ManagedProjectSite>
pub fn update_runtime(site_id: &str, update: RuntimeUpdate) → Result<()>
pub fn resource_summary() → Result<AdminResourceSummary>
pub async fn start_site(site_id: String) → Result<()>
pub async fn parse_site(site_id: String) → Result<()>
pub async fn restart_site(site_id: &str) → Result<()>
pub async fn stop_site(site_id: &str) → Result<StopSiteResult>
pub fn delete_site(site_id: &str) → Result<bool>
pub fn runtime_status(site_id: &str) → Result<ManagedSiteRuntimeSt...
pub fn tail_log(site_id: &str, kind: &str, limit: usize) → Result<TailLogResponse>
pub fn full_log_path(site_id: &str, kind: &str) → Result<PathBuf>
pub fn logs(site_id: &str) → Result<ManagedSiteLogsRespo...
```

### src\web_server\sqlite_spatial_api.rs
```
pub struct SqliteSpatialQueryParams
pub struct SpatialQueryResult
pub struct SpatialQueryResultItem
pub struct AabbDto
pub struct Vec3Dto
pub struct SpatialStatsResult
pub async fn api_sqlite_spatial_query(Query(params) → Json<SpatialQueryResult>
pub async fn api_sqlite_spatial_stats() → Json<SpatialStatsResult>
```

### src\data_interface\db_model.rs
```
impl AiosDBManager
  pub async fn init_form_config() → anyhow::Result<Self>
  pub async fn exec_watcher(mgr: Arc<AiosDBManager>) → anyhow::Result<()>
  pub async fn run_e3d_clone_bg_task(mgr: Arc<AiosDBManager>) → anyhow::Result<()>
  pub async fn exec_delta_clone_remotes(watcher: &PdmsWatcher, sync_msg: SyncE3dFileMsg,) → anyhow::Result<bool>
  pub async fn spawn_exec_watcher(mgr: Arc<AiosDBManager>) → anyhow::Result<()>
  pub async fn demo_mqtt_requests()
  pub async fn poll_sync_e3d_mqtt_events(watcher: Arc<PdmsWatcher>)
```

### src\fast_model\export_model\name_config.rs
```
pub struct NameConfig
impl NameConfig
  pub fn load_from_excel(path: P) → Result<Self>
  pub fn convert_name(&self, model_name: &str) → String
  pub fn has_mapping(&self, model_name: &str) → bool
  pub fn len(&self) → usize
  pub fn is_empty(&self) → bool
```

### src\web_api\e3d_tree_api.rs
```
pub struct E3dTreeApiState
pub struct TreeNodeDto
pub struct NodeResponse
pub struct ChildrenResponse
pub struct AncestorsResponse
pub struct SubtreeRefnosResponse
pub struct VisibleInstsResponse
pub struct SearchRequest
pub struct SearchResponse
pub struct NodeAabb
pub struct SiteNodeDto
pub struct SiteNodesResponse
pub struct ChildrenQuery
pub struct SubtreeQuery
pub fn create_e3d_tree_routes(state: E3dTreeApiState) → Router
```

### src\web_api\mbd_pipe_api.rs
```
pub struct MbdPipeQuery
pub struct MbdPipeResponse
pub struct MbdPipeData
pub struct MbdPipeStats
pub struct BranchAttrsDto
pub struct MbdPipeSegmentDto
pub struct MbdDimDto
pub struct MbdWeldDto
pub struct MbdSlopeDto
pub struct MbdLayoutHint
pub struct MbdCutTubiDto
pub struct MbdFittingDto
pub struct MbdTagDto
pub struct MbdBendDto
pub struct MbdPipeDebugInfo
pub struct MbdExportStats
pub struct MbdExportFailure
pub struct MbdManifest
pub struct MbdManifestEntry
pub enum MbdPipeSource
pub enum MbdPipeMode
pub enum MbdDimKind
pub enum MbdWeldType
pub enum MbdFittingKind
pub enum MbdBendMode
```

### src\web_server\instance_export.rs
```
pub async fn export_model_bundle(refnos: &[RefnoEnum], task_id: &str, output_dir: &Path, mesh_dir: &Path,) → Result<PathBuf>
pub async fn export_model_bundle_with_dbno(refnos: &[RefnoEnum], task_id: &str, output_dir: &Path, mesh_dir: &Path, dbno: Option<u32>,) → Result<PathBuf>
```

### src\web_server\stream_generate.rs
```
pub struct StreamGenerateRequest
pub struct StreamGenerateQuery
pub enum StreamGenerateEvent
pub async fn query_visible_descendants(root_refnos: &[RefnoEnum], max_depth: u32,) → anyhow::Result<Vec<RefnoEnum>>
pub async fn filter_missing_inst_relate(refnos: &[RefnoEnum]) → anyhow::Result<Vec<RefnoEnum>>
pub async fn api_stream_generate(State(state) → Response
pub async fn api_stream_generate_by_root(State(state) → Response
```

### src\lib.rs
```
pub async fn build_room_relations(_db_option: &aios_core::options::DbOption, _db_nums: Option<&[u32]>, _refno_root: Option<aios_core::RefnoEnum>,) → anyhow::Result<()>
pub async fn run_cli(db_option_ext: options::DbOptionExt) → anyhow::Result<()>
pub fn init_logging(enable_log: bool)
pub async fn run_app(option: Option<DbOptionExt>) → anyhow::Result<()>
```

### src\web_api\review_db.rs
```
pub async fn init_review_primary_db(db_option: &DbOption) → Result<()>
pub async fn fresh_review_db() → Result<Surreal<Client>>
pub async fn ensure_review_primary_db_context() → Result<()>
pub fn review_primary_db() → &'static Surreal<Client>
pub async fn ensure_review_workflow_history_schema() → Result<()>
```

### src\fast_model\gen_model\model_writer.rs
```
pub struct ModelWriterStageReport
pub struct ModelWriterContractEvidence
pub struct DrainOnlyStats
pub struct ModelWriteBatchReport
pub struct ModelWriterFinishReport
pub struct BooleanBridgeRequest
pub struct BooleanBridgeReport
pub struct SurrealModelWriterBackend
pub struct DrainOnlyModelWriterBackend
pub enum ModelWriterStageStatus
pub trait ModelWriterBackend
impl ModelWriterStageReport
impl DrainOnlyStats
  pub fn print_summary(&self)
impl SurrealModelWriterBackend
  pub fn new(mesh_aabb_map: Arc<DashMap<String, Aabb>>, missing_neg_carriers: Arc<Mutex<HashSet<RefnoEnum>>>,) → Self
impl SurrealModelWriterBackend
impl DrainOnlyModelWriterBackend
  pub fn new() → Self
impl DrainOnlyModelWriterBackend
impl DrainOnlyModelWriterBackend
pub fn create_model_writer(mode: ModelWriterMode, mesh_aabb_map: Arc<DashMap<String, Aabb>>, missing_neg_carriers: Arc<Mutex<HashSet<RefnoEnum>>>,) → Arc<dyn ModelWriterBackend>
pub async fn run_model_writer_sink(receiver: flume::Receiver<ShapeInstancesData>, writer: Arc<dyn ModelWriterBackend>,) → anyhow::Result<ModelWriterF...
pub async fn run_drain_only_sink(receiver: flume::Receiver<ShapeInstancesData>,) → anyhow::Result<DrainOnlyStats>
pub fn model_writer_contract_evidence(mode: ModelWriterMode) → ModelWriterContractEvidence
```

### src\web_server\model_writer_verify.rs
```
pub struct ModelWriterVerifyRequest
pub async fn api_model_writer_verify(Json(req) → impl IntoResponse
```

### src\fast_model\gen_model\orchestrator.rs
```
pub struct GenModelResult
impl BatchStageJoiner
pub async fn gen_all_geos_data(manual_refnos: Vec<RefnoEnum>, db_option: &DbOptionExt, incr_updates: Option<IncrGeoUpdateLog>, target_sesno: Option<u32>,) → Result<GenModelResult>
pub async fn update_sqlite_spatial_index_from_cache(db_option: &DbOptionExt, dbnums: &[u32],) → Result<()>
pub async fn update_sqlite_spatial_index_from_cache(_db_option: &DbOptionExt, _dbnums: &[u32],) → Result<()>
```

### src\web_api\platform_api\types.rs
```
pub struct EmbedUrlRequest
pub struct EmbedUrlResponse
pub struct EmbedUrlData
pub struct EmbedUrlQuery
pub struct EmbedLineage
pub struct ReviewFormSummary
pub struct CachePreloadRequest
pub struct CachePreloadResponse
pub struct SyncWorkflowRequest
pub struct WorkflowActor
pub struct WorkflowNextStep
pub struct WorkflowVerifyNextStepDiagnostic
pub struct SyncWorkflowResponse
pub struct VerifyWorkflowResponse
pub struct VerifyWorkflowData
pub struct SyncWorkflowData
pub struct WorkflowRecord
pub struct WorkflowAnnotationComment
pub struct WorkflowAttachment
pub struct DeleteReviewRequest
pub struct DeleteReviewResponse
pub struct DeleteReviewResult
pub struct ReviewForm
pub struct ReviewFormRow
impl SyncWorkflowRequest
```

### src\bin\model_writer_verify.rs
```
impl DrainOnlyExecEvidence
```

### src\fast_model\gen_model\transform_cache.rs
```
pub struct TransformCacheManager
impl TransformCacheManager
  pub fn new() → Self
  pub fn get_world_transform(&self, dbnum: u32, refno: RefnoEnum) → Option<Transform>
  pub fn get_local_transform(&self, dbnum: u32, refno: RefnoEnum) → Option<Transform>
  pub fn remove(&self, dbnum: u32, refno: RefnoEnum)
  pub fn insert_world_transform(&self, dbnum: u32, refno: RefnoEnum, world: Transform)
  pub fn insert_local_transform(&self, dbnum: u32, refno: RefnoEnum, local: Transform)
  pub fn is_dbnum_loaded(&self, dbnum: u32) → bool
  pub fn load_dbnum_snapshot(&self, dbnum: u32, snapshot: LoadedTransformDbnum)
pub fn init_global_transform_cache()
pub fn prime_global_transform_cache_from_pe_entries(entries: &[PeTransformEntry]) → usize
pub fn clear_global_transform_cache() → usize
pub fn clear_global_transform_cache_for_refnos(refnos: &[RefnoEnum]) → usize
pub fn pin_global_transform_cache_for_refnos(refnos: &[RefnoEnum]) → usize
pub fn release_global_transform_cache_for_refnos(refnos: &[RefnoEnum]) → usize
pub async fn get_world_transform_cache_first(db_option: Option<&DbOptionExt>, refno: RefnoEnum,) → anyhow::Result<Option<Trans...
pub async fn get_local_transform_cache_first(db_option: Option<&DbOptionExt>, refno: RefnoEnum,) → anyhow::Result<Option<Trans...
pub async fn get_world_transforms_cache_only_batch(db_option: &DbOptionExt, refnos: &[RefnoEnum],) → anyhow::Result<HashMap<Refn...
pub async fn get_local_transforms_cache_only_batch(db_option: &DbOptionExt, refnos: &[RefnoEnum],) → anyhow::Result<HashMap<Refn...
pub async fn get_world_transform_cache_only(db_option: &DbOptionExt, refno: RefnoEnum,) → anyhow::Result<Transform>
pub async fn get_local_transform_cache_only(db_option: &DbOptionExt, refno: RefnoEnum,) → anyhow::Result<Transform>
pub async fn ensure_world_transforms_present(db_option: &DbOptionExt, refnos: &[RefnoEnum],) → anyhow::Result<()>
pub async fn ensure_local_transforms_present(db_option: &DbOptionExt, refnos: &[RefnoEnum],) → anyhow::Result<()>
pub async fn get_world_transforms_cache_first_batch(db_option: Option<&DbOptionExt>, refnos: &[RefnoEnum],) → anyhow::Result<HashMap<Refn...
```

### src\options.rs
```
pub struct DbOptionExt
pub enum MeshFormat
pub enum ModelWriterMode
pub enum TransformWriteBackend
pub enum TransformReadBackend
pub enum BooleanPipelineMode
pub enum RegenDeleteMode
impl MeshFormat
impl ModelWriterMode
impl ModelWriterMode
  pub fn as_str(&self) → &'static str
  pub fn writes_to_surreal(&self) → bool
impl TransformWriteBackend
impl TransformWriteBackend
  pub fn as_str(&self) → &'static str
  pub fn writes_to_surreal(&self) → bool
  pub fn writes_to_parquet(&self) → bool
  pub fn uses_ducklake(&self) → bool
impl TransformReadBackend
impl TransformReadBackend
  pub fn as_str(&self) → &'static str
  pub fn needs_parquet_feature(&self) → bool
  pub fn needs_ducklake_feature(&self) → bool
impl BooleanPipelineMode
impl RegenDeleteMode
```

### src\pe_transform_store.rs
```
pub struct BackendCompareStats
pub async fn save_entries_with_backend(db_option: &DbOptionExt, entries: &[PeTransformEntry],) → Result<()>
pub async fn load_entries_with_backend(db_option: &DbOptionExt, backend: TransformReadBackend, refnos: &[RefnoEnum],) → Result<Vec<PeTransformEntry>>
pub async fn clear_pe_transform_for_dbnums(dbnums: &[u32]) → Result<usize>
pub async fn compare_backends_for_dbnums(db_option: &DbOptionExt, dbnums: &[u32],) → Result<Vec<BackendCompareSt...
```

### src\pe_transform_refresh.rs
```
pub async fn refresh_pe_transform_for_dbnums_compat(dbnums: &[u32]) → Result<usize>
pub async fn refresh_pe_transform_for_dbnums(dbnums: &[u32], db_option: &DbOptionExt,) → Result<usize>
pub async fn refresh_pe_transform_for_root_refnos_compat(root_refnos: &[RefnoEnum],) → Result<usize>
pub async fn refresh_pe_transform_for_root_refnos(root_refnos: &[RefnoEnum], db_option: &DbOptionExt,) → Result<usize>
```

### src\shared\progress_hub.rs
```
pub struct ProgressHub
pub struct ProgressMessage
pub struct ProgressMessageBuilder
pub enum TaskStatus
impl TaskStatus
impl ProgressHub
  pub fn new(buffer_size: usize) → Self
  pub fn default() → Self
  pub fn register(&self, task_id: String) → broadcast::Receiver<Progres...
  pub fn restore_state(&self, message: ProgressMessage) → broadcast::Receiver<Progres...
  pub fn subscribe(&self, task_id: &str) → broadcast::Receiver<Progres...
  pub fn publish(&self, message: ProgressMessage) → Result<usize, String>
  pub fn get_task_state(&self, task_id: &str) → Option<ProgressMessage>
  pub fn has_task(&self, task_id: &str) → bool
impl ProgressMessageBuilder
  pub fn new(task_id: impl Into<String>) → Self
  pub fn status(mut self, status: TaskStatus) → Self
  pub fn percentage(mut self, percentage: f32) → Self
  pub fn step(mut self, name: impl Into<String>, current: u32, total: u32) → Self
  pub fn items(mut self, processed: u64, total: u64) → Self
  pub fn message(mut self, message: impl Into<String>) → Self
  pub fn details(mut self, details: serde_json::Value) → Self
  pub fn build(self) → ProgressMessage
```

### src\web_api\jwt_auth.rs
```
pub struct JwtConfig
pub struct PlatformAuthConfig
pub struct ReviewAuthConfig
pub struct TokenClaims
pub struct TokenRequest
pub struct TokenResponse
pub struct TokenData_
pub struct VerifyRequest
pub struct VerifyResponse
pub struct VerifyData
pub enum Role
impl JwtConfig
impl PlatformAuthConfig
impl ReviewAuthConfig
impl JwtConfig
  pub fn from_config_file() → Self
impl PlatformAuthConfig
  pub fn from_config_file() → Self
impl ReviewAuthConfig
  pub fn from_config_file() → Self
impl Role
  pub fn from_str(s: &str) → Option<Self>
  pub fn as_str(&self) → &'static str
  pub fn display_name(&self) → &'static str
  pub fn valid_values() → &'static [&'static str]
```

### src\web_api\platform_api\annotation_check.rs
```
pub struct AnnotationCheckRequest
pub struct AnnotationCheckResponse
pub struct AnnotationCheckResult
pub struct AnnotationCheckSummary
pub struct AnnotationCheckBlocker
pub struct AnnotationCheckContext
pub struct AnnotationCheckOptions
pub enum AnnotationCheckIntent
impl AnnotationCheckIntent
  pub fn as_str(self) → &'static str
impl AnnotationGateState
pub fn annotation_check_failed_response(result: AnnotationCheckResult) → AnnotationCheckResponse
pub fn build_annotation_check_context(task_id: impl Into<String>, form_id: impl Into<String>, current_node: impl Into<String>,) → AnnotationCheckContext
pub async fn check_annotations_handler(headers: HeaderMap, Json(request) → impl IntoResponse
pub async fn resolve_annotation_check_context(request: &AnnotationCheckRequest,) → Result<AnnotationCheckConte...
pub async fn evaluate_annotation_check(context: &AnnotationCheckContext, options: AnnotationCheckOptions,) → Result<AnnotationCheckResul...
```

### src\web_api\platform_api\embed_url.rs
```
pub async fn get_embed_url(Json(request) → impl IntoResponse
```

### src\web_api\platform_api\workflow_sync.rs
```
impl WorkflowSyncActionError
pub async fn verify_workflow_handler(Json(mut request) → impl IntoResponse
pub async fn sync_workflow_handler(Json(mut request) → impl IntoResponse
```

### src\web_api\review_api.rs
```
pub struct CreateTaskRequest
pub struct UpdateTaskRequest
pub struct ReviewActionRequest
pub struct SubmitToNextRequest
pub struct ReturnRequest
pub struct WorkflowStep
pub struct ReviewComponent
pub struct ReviewAttachment
pub struct ReviewTask
pub struct TaskListResponse
pub struct TaskResponse
pub struct ActionResponse
pub struct TaskListQuery
pub struct ConfirmedRecordData
pub struct ConfirmedRecordResponse
pub struct ConfirmedRecordWithMeta
pub struct AnnotationComment
pub struct CreateCommentRequest
pub struct CommentResponse
pub struct CommentQuery
pub struct EditCommentRequest
pub struct AnnotationSeverityTypeQuery
pub struct AnnotationSeverityBody
pub struct AnnotationSeverityResponse
pub struct AnnotationBasicFieldsBody
```

### src\web_server\handlers.rs
```
pub struct CreateBatchTaskRequest
pub struct CreateTaskTemplateRequest
pub struct SshOptions
pub struct SurrealControlRequest
pub struct SurrealTestRequest
pub struct SqliteSpatialQuery
pub struct DeploymentSiteBrowseQuery
pub struct GetInstancesRequest
pub struct ModelDataResponse
pub struct SurrealStatusQuery
pub struct TraySupportsDetectRequest
pub struct SctnTestRequest
pub struct DatabaseConnectionStatus
pub struct DatabaseConnectionConfig
pub struct StartupScript
pub struct DbConnCheckQuery
pub struct StartDatabaseRequest
pub struct ExportRequest
pub struct ExportResponse
pub struct ExportStatusResponse
pub struct ListFilesQuery
pub struct SceneTreeFileResponse
pub async fn kill_port_processes(port: u16) → Result<Vec<u32>, String>
pub async fn check_port_status(Query(params) → Result<Json<serde_json::Val...
pub async fn kill_port_processes_api(Json(req) → Result<Json<serde_json::Val...
```

### src\web_server\mod.rs
```
pub struct AppState
pub struct TaskManager
pub struct ConfigManager
pub struct TaskQuery
pub struct CreateTaskRequest
pub struct UpdateConfigRequest
impl AppState
  pub fn new() → Self
impl ConfigManager
  pub fn add_template(&mut self, name: &str, config: DatabaseConfig)
pub async fn start_web_server(port: u16) → anyhow::Result<()>
pub async fn start_web_server_with_config(port: u16, config_file: Option<&str>,) → anyhow::Result<()>
```

### src\web_server\wizard_handlers.rs
```
pub struct DatabaseFileInfo
pub struct DatabaseFileScanRequest
pub struct DatabaseFileScanResult
pub struct BrowseDirectoryRequest
pub struct DirectoryEntry
pub struct BrowseDirectoryResponse
pub async fn scan_directory(State(_state) → Result<Json<DirectoryScanRe...
pub async fn list_projects(State(_state) → Result<Json<Vec<ProjectInfo...
pub async fn create_wizard_task(State(state) → Result<Json<TaskInfo>, (Sta...
pub async fn get_wizard_templates(State(_state) → Result<Json<Vec<TaskTemplat...
pub async fn scan_database_files(State(_state) → Result<Json<DatabaseFileSca...
pub fn open_deployment_sites_sqlite() → Result<rusqlite::Connection...
pub fn persist_task_progress_to_sqlite(task: &TaskInfo) → Result<(), Box<dyn std::err...
pub async fn browse_directory(Query(request) → Result<Json<BrowseDirectory...
pub fn delete_deployment_site_from_sqlite(site_id: &str) → Result<(), Box<dyn std::err...
pub fn load_wizard_config_by_task_id(task_id: &str) → Option<DataParsingWizardCon...
pub fn restore_tasks_from_sqlite() → Vec<TaskInfo>
pub fn load_deployment_sites_from_sqlite() → Result<Vec<serde_json::Valu...
pub fn load_deployment_site_by_id_from_sqlite(site_id: &str,) → Result<Option<serde_json::V...
pub fn update_deployment_site_health(site_id: &str, status: &str, timestamp: &str,) → Result<(), Box<dyn std::err...
pub fn save_api_deployment_site(site: &DeploymentSite,) → Result<String, Box<dyn std:...
```


---

## Model routing hints
<!-- Generated by SigMap routing module — update gen-context.config.json to disable -->

Select the model tier based on the task complexity and the files involved.

### Fast (low-cost)
**Examples:** claude-haiku-4-5, gpt-5-1-codex-mini, gemini-3-flash  
**Cost:** ~$0.0008 / 1K tokens

**Use for tasks like:**
- Autocomplete and inline suggestions
- Edit config or markup files
- Fix typos and rename symbols
- Format, lint, and trivial style changes
- Explain a short utility function
- Generate simple shell scripts or Dockerfiles

**Files in this tier:**
- `src\fast_model\cache_flush.rs`
- `src\fast_model\export_model\import_glb.rs`
- `src\fast_model\export_model\spec_info.rs`
- `src\fast_model\gen_model\cata_resolve_cache_pipeline.rs`
- `src\fast_model\gen_model\cate_single.rs`
- `src\fast_model\gen_model\cate_processor.rs`
- `src\fast_model\gen_model\loop_processor.rs`
- `src\fast_model\gen_model\query.rs`
- `src\fast_model\gen_model\prim_processor.rs`
- `src\fast_model\model_cache\query.rs`
- `src\fast_model\model_store.rs`
- `src\fast_model\shared.rs`
- `src\scene_tree\parquet_export.rs`
- `src\scene_tree\schema.rs`
- `src\team_data.rs`
- … and 24 more

### Balanced (mid-tier)
**Examples:** claude-sonnet-4-6, gpt-5-2, gemini-3-1-pro  
**Cost:** ~$0.003 / 1K tokens

**Use for tasks like:**
- Write unit or integration tests
- Implement a well-scoped feature function
- Debug a runtime error with stack trace
- Refactor a module (< 200 lines)
- Generate a PR description
- Explain a multi-function module

**Files in this tier:**
- `src\data_interface\mesh_manager.rs`
- `src\dblist_parser\db_loader.rs`
- `src\fast_model\cal_model\equip_model.rs`
- `src\fast_model\export_model\export_gltf.rs`
- `src\fast_model\export_model\export_glb.rs`
- `src\fast_model\export_model\export_pdms_tree_parquet.rs`
- `src\fast_model\export_model\export_unit_mesh_glb.rs`
- `src\fast_model\export_model\parquet_stream_writer.rs`
- `src\fast_model\export_model\parquet_writer.rs`
- `src\fast_model\export_model\pe_parquet_writer.rs`
- `src\fast_model\export_model\simple_color_palette.rs`
- `src\fast_model\gen_model\boolean_backfill.rs`
- `src\fast_model\gen_model\cate_helpers.rs`
- `src\fast_model\gen_model\index_tree_mode.rs`
- `src\fast_model\gen_model\neg_query.rs`
- … and 69 more

### Powerful (high-cost)
**Examples:** claude-opus-4-6, gpt-5-4, gemini-2-5-pro  
**Cost:** ~$0.015 / 1K tokens

**Use for tasks like:**
- Cross-cutting architecture decisions
- Multi-file refactor spanning 5+ files
- Security audit (OWASP Top 10)
- Complex debugging across async boundaries
- Migration plan for a library/framework upgrade
- Designing a new module from requirements

**Files in this tier:**
- `src\expression_fix.rs`
- `src\fast_model\convex_decomp.rs`
- `src\fast_model\export_model\model_exporter.rs`
- `src\fast_model\gen_model\boolean_task.rs`
- `src\fast_model\gen_model\cache_miss_report.rs`
- `src\fast_model\gen_model\query_provider.rs`
- `src\fast_model\gen_model\sql_file_writer.rs`
- `src\fast_model\gen_model\tree_index_manager.rs`
- `src\fast_model\instance_cache.rs`
- `src\fast_model\unit_converter.rs`
- `src\perf_timer.rs`
- `src\spatial_index.rs`
- `src\web_server\database_status_handlers.rs`
- `src\web_server\simple_templates.rs`
- `src\web_server\task_creation_handlers.rs`
- … and 39 more

> **Tip:** Run `node gen-context.js --routing` to regenerate routing hints.
> See `docs/MODEL_ROUTING.md` for full routing guide and cost optimisation tips.

<!-- SPECKIT START -->
For additional context about technologies to be used, project structure,
shell commands, and other important information, read the current plan
`specs/019-system-mdb-dependency-discovery/plan.md`
<!-- SPECKIT END -->
