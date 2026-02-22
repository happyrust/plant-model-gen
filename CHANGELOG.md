# Changelog

## 2026-02-22

### Breaking

- **模型生成模式开关已移除，统一为 IndexTree 默认管线**
  - 删除 `full_noun_mode` 配置与 `FULL_NOUN_MODE` 环境分支
  - 删除旧的 `non_full_noun` 主路径与相关路由
  - `manual/debug/incremental/full` 统一收敛到单入口 `process_index_tree_generation`

### Changed

- **Full Noun 语义重命名为 IndexTree**
  - 文件重命名：`full_noun_mode.rs` -> `index_tree_mode.rs`
  - 类型/函数重命名：`FullNounConfig/Error` -> `IndexTreeConfig/Error`，`gen_full_noun_geos_optimized` -> `gen_index_tree_geos_optimized`
  - `noun_collection` 对外命名改为 `IndexTreeTargetCollection`
  - 日志与性能标签统一为 `index_tree_*` 语义

- **配置键统一为 `index_tree_*`**
  - `full_noun_max_concurrent_nouns` -> `index_tree_max_concurrent_targets`
  - `full_noun_batch_size` -> `index_tree_batch_size`
  - `full_noun_enabled_categories` -> `index_tree_enabled_target_types`
  - `full_noun_excluded_nouns` -> `index_tree_excluded_target_types`
  - `debug_limit_per_noun` -> `index_tree_debug_limit_per_target_type`
  - `db_options/*.toml` 全量切换到新键

### Fixed

- **`--gen-indextree <dbnum>` 配置读取路径与主流程统一**
  - `generate_single_indextree` 不再硬编码 `DbOption.toml`
  - 改为优先读取 `DB_OPTION_FILE`，默认 `db_options/DbOption.toml`

## 2026-02-13

### Refactored

- **instance_cache.rs: 消除冗余 V1 中间转换层，直接序列化 aios_core 原始类型**
  - 删除 `TransformV1`、`TubiDataV1`、`EleGeosInfoV1`、`EleInstGeoV1`、`EleInstGeosDataV1`、`CachedInstanceBatchV1` 共 6 个中间结构体及其转换函数（净删 ~210 行）
  - 新增 `CachedInstanceBatchRkyv`，直接引用 `aios_core` 已有 rkyv 派生的原始类型
  - schema 版本 bump 到 V3，旧缓存自动 miss 重建
  - 根因：V1/V2 中间层需要手动同步字段，遗漏 `tubi` 字段导致 tubi_relate 写入 0 条

### Fixed

- **修复 cache 模式下 tubi_relate 写入 0 条的问题**
  - 原因：`EleGeosInfoV1` 缺少 `tubi` 字段，cache 序列化/反序列化丢失 tubi 数据
  - 影响：BRAN 管道的 tubi 直段数据在 cache→SurrealDB flush 时全部丢失

### Added

- **cache-only 布尔运算增强：几何输入缓存与 STWALL 支持**
  - 新增 `geom_input_cache.rs`：缓存布尔运算的几何输入数据，支持 neg 重投影到 pos AABB
  - `manifold_bool.rs`：新增 `reproject_neg_to_pos_aabb` 修复 NGMR 负实体 Z 位置偏移
  - `query.rs`：布尔成功结果查询增加 debug 日志

- **模型生成流程优化**
  - `input_cache_pipeline.rs` / `full_noun_mode.rs` / `loop_processor.rs` / `prim_processor.rs`：重构几何输入管线
  - `orchestrator.rs`：优化编排逻辑
  - `context.rs`：扩展生成上下文
  - `cli_modes.rs` / `main.rs`：新增 CLI 选项
  - `handlers.rs`：新增 web API handler
  - `export_dbnum_instances_parquet.rs`：新增 Parquet 导出功能

## 2026-01-30

### Fixed

- **cache-only 导出读取 instance_cache 选取最新 batch，避免旧数据污染**
  - 修复：`query_geometry_instances_ext_from_cache` 按 `created_at` 选择最新的 `inst_relate_bool(Success)`，并且 inst_info/inst_geos/inst_tubi 仅取最新 batch
  - 影响：解决 `--regen-model --export-obj` 场景下部分子孙节点（例如 BOX）看似“未导出”的问题（实为命中旧 bool mesh_id/旧 inst_geo）
  - 修改位置：`src/fast_model/export_model/model_exporter.rs`

- **修复 RTOR（矩形环面体）尺寸被平方放大**
  - 原因：RTOR 的实际尺寸已由 `geo_param` 表达，若 `transform.scale` 仍携带尺寸会在导出阶段再次乘入，导致重复缩放（例如 160mm -> 25600mm）
  - 修复：生成 inst_geo 时对 `PrimRTorus` 清零 scale（非 unit_flag）
  - 修改位置：`src/fast_model/prim_model.rs`

### Added

- **新增 cache 排查示例**
  - `examples/inspect_cache_geom_refno.rs`：按 refno 检查最新命中 batch 的 inst_info/inst_geos/geo_param/scale

- **凸分解（可选）与调试工具补全**
  - 新增 feature：`convex-decomposition`
  - 新增：`src/fast_model/convex_decomp.rs`、Meili 导入 spool 工具 `src/bin/meili_import_spool.rs`、以及相关配置样例 `DbOption-*.toml`

## 2026-01-29

### Fixed

- **修复 foyer cache 布尔运算结果错误（漏切/错切/退化）**
  - 问题：`run_boolean_worker_from_cache_manager` 仅按同一实例自身的 `GeoBasicType::Neg/CataNeg/CataCrossNeg` 做差集，未使用缓存中的 `neg_relate_map/ngmr_neg_relate_map` 关系语义，且在 world 坐标直接做布尔，导致结果不稳定/不符合预期
  - 修复：以 `neg_relate_map/ngmr_neg_relate_map` 为真源构建切割目标；负实体按 `inverse(pos_world) * neg_world` 映射到正实体局部坐标系执行；加入逐个 subtract 的退化保护与高精度重算兜底
  - 修改位置：`src/fast_model/manifold_bool.rs`

## 2026-01-28

### Fixed

- **修复 `--export-obj` + `--regen-model` 时包含子孙节点导致的布尔结果缺失**
  - 问题：导出默认包含子孙节点；但 regen 仅重建根节点，`replace_mesh` 会清理旧 ngmr/neg 关系，导致导出阶段回退到“未布尔”的正实体 mesh
  - 修复：当导出配置包含子孙节点时，regen 前先查询并合并子孙节点，确保“清理范围 == 重建范围”
  - 修改位置：`src/cli_modes.rs`

## 2026-01-22

### Changed

- **refno 查询改用 TreeIndex**
  - 按类型/分页/计数的 refno 查询切换到 TreeIndexManager
  - Full Noun 根节点筛选走 TreeIndex，并新增 db_meta_info.json 解析兜底
  - TreeIndexManager 增加全局缓存，避免重复加载 .tree

- **移除 cata_hash 复用的 DB 探测**
  - build_cata_hash_map_from_tree 不再访问 SurrealDB，避免依赖未初始化连接

## 2025-12-15

### Changed

- **BRAN 类型跳过布尔运算优化**
  - 修改位置：
    - `src/fast_model/manifold_bool.rs` - 在布尔运算入口过滤 BRAN 类型
    - `src/fast_model/mesh_generate.rs` - 删除 `fix_missing_neg_relates` 函数，新增 `process_meshes_bran` 专用函数
    - `src/fast_model/gen_model/models.rs` - BRAN 使用独立的网格处理流程
  - 优化内容：
    - BRAN 类型不再执行布尔运算和 neg_relate 检查
    - BRAN 使用非 deep 遍历的网格生成，提高性能
    - 减少了不必要的数据库查询和日志噪音
  - 影响：BRAN/HANG 类型的模型生成速度提升，避免无意义的布尔运算警告

- **简化 neg_relate/ngmr_relate 关系创建逻辑**
  - 修改位置：
    - `src/fast_model/pdms_inst.rs` - `save_instance_data_optimize`
  - 优化内容：
    - 创建关系时仅依赖当前批次缓存的 `geo_relate_ids`，不再在 cache miss 时回退查询数据库
    - 降低了跨批次关系补全的不确定性，并减少警告日志噪音

## 2025-11-27

### Fixed

- **修复 `has_tubi` 字段反序列化错误问题**
  - 问题：数据库中某些 `SPdmsElement` 记录的 `has_tubi` 字段为 null，而不是期望的 bool 类型，导致反序列化失败
  - 错误信息：`Failed to deserialize field 'has_tubi' on type 'SPdmsElement': Expected bool, got none`
  - 修复方案：
    - 从 [`../rs-core/src/types/pe.rs`](../rs-core/src/types/pe.rs:26) 中移除了 `has_tubi` 字段定义
    - 从 [`../rs-core/src/rs_surreal/inst_structs.rs`](../rs-core/src/rs_surreal/inst_structs.rs:85) 中移除了 `TubiRelate::to_surql` 方法中对 `has_tubi = true` 的设置
    - 修改了 [`src/fast_model/cata_model.rs`](src/fast_model/cata_model.rs:1633) 中的代码，移除了对 `has_tubi` 字段的更新逻辑，改为直接使用 `tubi_relate` 表判断
    - 修复了 [`src/dblist_parser/db_loader.rs`](src/dblist_parser/db_loader.rs:49) 中的导入路径问题
  - 影响：解决了 `cargo run --bin aios-database` 编译和运行时的反序列化错误
  - 相关提交：gen-model@f41002f4, rs-core@2dd7c11

### Changed

- **优化 tubi 关系查询逻辑**
  - 不再依赖 `has_tubi` 字段来判断是否有 tubi 关系
  - 直接使用 `tubi_relate` 表的 `in` 字段来判断，更加可靠和准确
  - [`../rs-core/src/rs_surreal/inst.rs`](../rs-core/src/rs_surreal/inst.rs:71) 中的 `query_tubi_insts_by_brans` 函数已经使用这种方式查询
  - 提高了数据一致性和查询性能

## 2025-11-26

### Added

- **为 `test_full_boolean_flow` 添加 OBJ 模型导出功能**
  - 功能：在布尔运算完成后自动导出布尔前后的 OBJ 模型用于可视化验证
  - 实现位置：`src/bin/test_full_boolean_flow.rs`
  - 新增函数：`get_mesh_dir_with_lod()` - 根据配置获取正确的 LOD mesh 目录
  - 导出文件：
    - `test_output/boolean_exports/before_boolean_{refno}.obj` - 布尔运算前的正实体
    - `test_output/boolean_exports/after_boolean_{refno}.obj` - 布尔运算后的结果
  - 用途：
    - 可在 Blender、MeshLab 等 3D 软件中打开查看
    - 对比布尔运算前后的几何变化
    - 验证负实体是否正确被减去
  - 依赖：`aios_database::fast_model::export_model::export_obj::export_obj_for_refnos`

### Changed

- **更新完整布尔运算测试指南**
  - 文件：`llmdoc/guides/complete_boolean_test_guide.md`
  - 更新内容：
    - 在测试流程中添加"步骤 4: 导出 OBJ 模型"
    - 更新测试输出示例，包含 OBJ 导出日志
    - 添加输出文件路径和使用说明
    - 在总结部分添加 OBJ 相关的关键指标和成功标准
  - 新增章节：详细说明如何导出和查看 OBJ 模型

### Documentation
- **新增 `llmdoc/agent/boolean_obj_export_implementation.md`**
  - 完整记录 OBJ 导出功能的实现细节
  - 包含技术实现、使用方法、示例输出
  - 提供后续改进建议

## 2025-10-27

### Fixed
- **修复 SurrealDB 查询错误 "Expected any, got record"**
  - 问题：在多个查询中使用 `in.id != none` 条件导致 SurrealDB 执行记录存在性检查，触发类型不匹配错误
  - 影响：导致模型生成过程中 panic，错误信息为 "更新模型数据失败: Internal error: Expected any, got record"
  - 修复位置：
    - `src/fast_model/manifold_bool.rs` (第 79-85, 286-290 行) - 移除 2 处 `in.id != none` 条件
    - `src/fast_model/occ_generate.rs` (第 705-713 行) - 移除 `in.id != none` 条件
    - `src/web_server/handlers.rs` (第 1730-1735 行) - 移除 `in.id != none` 条件
  - 原理：`inst_relate` 关系表的 `in` 字段总是指向有效的 `pe:{refno}`，不需要额外的存在性检查
  - 结果：程序现在可以正常运行，成功处理 GLB 模型导出

### Changed
- **优化 SurrealDB 查询性能**
  - 移除冗余的 `in.id != none` 检查条件，减少不必要的数据库操作
  - 简化查询逻辑，提升查询效率

## 2025-10-16

### Fixed
- **修复 `get_ancestor_attmaps` 中 NONE 值导致的反序列化失败问题**
  - 问题：`fn::ancestor({}).refno.*` 查询返回的祖先链中包含 NONE 值（当节点的属性记录不存在时），导致 `try_into::<NamedAttrMap>()` 反序列化失败
  - 影响：导致 `get_world_transform` 无法获取世界变换，几何体生成被跳过
  - 修复：在 `AttributeQueryService::get_ancestor_attmaps` 和 `query.rs::get_ancestor_attmaps` 中使用 `filter_map` + `try_into().ok()` 过滤掉无法转换的 NONE 值
  - 相关提交：rs-core@2dd7c11, gen-model@f41002f4

### Added
- **为 `gen_prim_geos` 添加详细调试日志**
  - 添加函数入口/出口日志，记录总数量、批次策略
  - 添加每批次的详细执行日志（开始、完成、耗时）
  - 添加详细的错误处理日志，区分不同跳过原因（世界变换失败、brep_shape 创建失败等）
  - 添加实体处理进度跟踪（处理数、跳过数、发送次数）
  - 使用 `e3d_dbg!` 宏统一调试日志输出

## 2025-02-14

- 将 `external/rs-corel` 中的连接、类型封装全部合并进 `external/rs-core`。
- 主项目直接使用 `aios_core` 公开的运行时 / WebUI 接口，移除独立 `rs-corel` 依赖。
- Web UI 及增量数据模块改为使用 `aios_core` 的 `RecordId`、`Datetime` 与连接工具。
- 构建脚本验证通过（`cargo check`）。
