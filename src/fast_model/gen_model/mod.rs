// GenPipeline模型生成 - 模块化重构版本
//
// 本模块将原先的 2,095 行单文件重构为模块化结构，解决以下问题：
// 1. 文件过大（超出 250 行限制 8.4 倍）
// 2. 代码冗余（90% 重复代码）
// 3. 配置混乱（双重配置机制）
// 4. 并发性能问题
//
// # 错误分层策略（run 级 vs 元素级）
//
// 生成管线对失败采用两级口径，边界以"错误根因属于谁"划分：
//
// - **基础设施/阶段级失败 → 整个 run 失败（fail-closed）**：预检查、
//   pe_transform 失效、cata_hash_map 构建、tubi 生成、writer/mesh/boolean
//   任一阶段报错都必须把 `Err` 传播到 `gen_all_geos_data*` 返回值。这些
//   失败意味着"本应生成的内容没有生成"，若吞掉则 model_gen 水位照常推进、
//   欠账被消费，缺失将永远不被重试。
// - **元素级数据质量问题 → 记录并跳过（run 仍成功）**：单个元素 owner
//   无效（E-REF-002）、几何全部转换失败（E-GEO-003）等源数据缺陷通过
//   `model_error!` 落入 refno_errors 报告后跳过该元素。源数据不修复，
//   重试也不会有不同结果，阻断整库生成只会放大故障面。
//
// 判断新错误属于哪一级：问"重跑一次（数据不变）结果会不同吗？"——会
// （网络/DB/并发/资源类）就必须让 run 失败；不会（源数据自身缺陷）就
// 记录 refno_errors 后跳过。

/// E3D 调试宏
#[macro_export]
macro_rules! e3d_dbg {
    ($($arg:tt)*) => {{
        if $crate::fast_model::gen_model::is_e3d_debug_enabled() {
            println!($($arg)*);
        }
    }};
}

// 核心模块
pub mod cache_miss_report; // GenPipeline cache-first 缺失报告（output/<project>/cache_miss_report.json）
pub mod categorized_refnos;
pub mod config; // 配置管理 (Phase 2)
pub mod context; // 处理上下文
pub mod errors; // 错误类型 (Phase 2)
pub mod models; // 数据模型定义
pub mod neg_query;
pub mod noun_collection; // Noun 收集和分类 // 分类 Refno 存储 (Phase 3) // TreeIndex 批量查询辅助（按 dbnum 分组，返回 root -> Vec<desc>）

// 处理器模块
pub mod cate_helpers; // Cate 工具函数
pub mod cate_processor; // Cate 处理器
pub mod cate_single; // Cate 单元件处理
pub mod loop_processor; // Loop 处理器
pub mod prim_processor; // Prim 处理器

// GenPipeline 主逻辑 (Phase 3 - 优化版本)
pub mod gen_pipeline;

// 编排器模块：主入口函数和流程协调
pub mod orchestrator;

// 实用工具
pub mod hier_view; // specs/023 M2：层级视图（pe_owner 快照）
pub mod precheck_coordinator;
pub mod utilities; // 预检查协调器

// Mesh 处理
pub mod mesh_processing;
pub mod model_record_id;
pub mod model_writer;
pub(crate) mod write_pipeline;

// 从 fast_model 根目录迁入的模型生成管线模块
pub mod boolean_backfill; // 布尔任务 DB 补齐（enable_db_backfill）
pub mod boolean_task; // 布尔运算任务（内存驱动）
pub mod cata_model; // CATE 模型生成
pub mod db_meta_cache; // DB 元数据缓存
pub mod inst_query; // inst_relate/geo_relate 查询
pub mod loop_model; // LOOP 模型生成
pub mod manifold_bool; // 布尔运算
pub mod mesh_generate; // 网格生成
pub mod mesh_state; // mesh 状态源（file/db）
pub mod pdms_inst; // 实例数据保存
pub mod pdms_inst_surreal; // SurrealDB 极简存储
pub mod prim_model; // PRIM 模型生成
pub mod query; // 查询工具
pub mod query_compat; // 查询兼容层
pub mod query_provider; // 层级查询提供者（pe 快照）
pub mod resolve; // 几何解析
pub mod session_query; // VersionedReadSession 到现有几何算法类型的适配
pub mod sql_file_writer; // 延迟 SQL 文件写入器（零 DB 写入模式）
pub mod transform_cache; // 变换缓存
pub mod transform_rkyv_cache; // 变换 rkyv 磁盘缓存 // [foyer-removal] 桩模块

// 重新导出常用类型
pub use context::{GenerationReadContext, NounProcessContext};
pub use models::{DbModelInstRefnos, NounCategory};
pub use noun_collection::GenPipelineTargetCollection;

// Phase 2: 错误和配置
pub use config::{BatchSize, Concurrency, GenPipelineConfig};
pub use errors::{GenPipelineError, Result};

// Phase 3: 优化后的数据结构和主函数
pub use categorized_refnos::{CategorizedRefnos, CategoryStatistics};
pub use gen_pipeline::{gen_pipeline_geos, validate_sjus_map};

// 重新导出处理函数
pub use cate_processor::process_cate_refno_page;
pub use loop_processor::process_loop_refno_page;
pub use prim_processor::process_prim_refno_page;

// 编排器：主入口函数
pub use orchestrator::gen_all_geos_data;
pub use orchestrator::gen_all_geos_data_with_read_spec;
pub use orchestrator::gen_all_geos_data_with_read_specs;
pub use orchestrator::gen_all_geos_data_with_session;
pub use orchestrator::{GenModelResult, GenerationRunProvenance};

// 实用工具函数
pub use utilities::{
    is_e3d_debug_enabled, is_e3d_info_enabled, is_e3d_trace_enabled, query_tubi_size,
};

// Mesh 处理函数
pub use mesh_processing::process_meshes_by_dbnos;

// 预检查相关类型
pub use precheck_coordinator::{
    PeTransformPrecheckMode, PrecheckConfig, PrecheckStats, run_precheck,
};

// 迁入模块的重导出
pub use mesh_generate::{
    booleans_meshes_in_db, gen_inst_meshes, gen_meshes_in_db, process_meshes_bran,
    process_meshes_update_db, process_meshes_update_db_deep, process_meshes_update_db_deep_default,
    run_mesh_worker,
};
pub use query::*;
pub use resolve::*;
