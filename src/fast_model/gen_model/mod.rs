// IndexTree 模式模型生成 - 模块化重构版本
//
// 本模块将原先的 2,095 行单文件重构为模块化结构，解决以下问题：
// 1. 文件过大（超出 250 行限制 8.4 倍）
// 2. 代码冗余（90% 重复代码）
// 3. 配置混乱（双重配置机制）
// 4. 并发性能问题

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
pub mod cache_miss_report; // IndexTree cache-first 缺失报告（output/<project>/cache_miss_report.json）
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
pub mod prim_processor;
pub mod processor; // 通用处理器（消除冗余） // Prim 处理器

// IndexTree 主逻辑 (Phase 3 - 优化版本)
pub mod index_tree_mode;

// 编排器模块：主入口函数和流程协调
pub mod orchestrator;

// 实用工具
pub mod precheck_coordinator;
pub mod tree_index_manager;
pub mod utilities; // 预检查协调器

// Mesh 处理
pub mod mesh_processing;

// 从 fast_model 根目录迁入的模型生成管线模块
pub mod boolean_backfill; // 布尔任务 DB 补齐（enable_db_backfill）
pub mod boolean_task; // 布尔运算任务（内存驱动）
pub mod cata_model; // CATE 模型生成
pub mod cata_resolve_cache_pipeline;
pub mod db_meta_cache; // DB 元数据缓存
pub mod inst_query; // inst_relate/geo_relate 查询
pub mod loop_model; // LOOP 模型生成
pub mod manifold_bool; // 布尔运算
pub mod mesh_generate; // 网格生成
pub mod pdms_inst; // 实例数据保存
pub mod prim_model; // PRIM 模型生成
pub mod query; // 查询工具
pub mod query_compat; // 查询兼容层
pub mod query_provider; // TreeIndex 查询提供者
pub mod refno_assoc_index; // refno 关联聚合索引（删旧加速）
pub mod resolve; // 几何解析
pub mod sql_file_writer; // 延迟 SQL 文件写入器（零 DB 写入模式）
pub mod transform_cache; // 变换缓存
pub mod transform_rkyv_cache; // 变换 rkyv 磁盘缓存 // [foyer-removal] 桩模块

// 重新导出常用类型
pub use context::NounProcessContext;
pub use models::{DbModelInstRefnos, NounCategory};
pub use noun_collection::IndexTreeTargetCollection;
pub use processor::NounProcessor;

// Phase 2: 错误和配置
pub use config::{BatchSize, Concurrency, IndexTreeConfig};
pub use errors::{IndexTreeError, Result};

// Phase 3: 优化后的数据结构和主函数
pub use categorized_refnos::{CategorizedRefnos, CategoryStatistics};
pub use index_tree_mode::{gen_index_tree_geos_optimized, validate_sjus_map};

// 重新导出处理函数
pub use cate_processor::process_cate_refno_page;
pub use loop_processor::process_loop_refno_page;
pub use prim_processor::process_prim_refno_page;

// 编排器：主入口函数
pub use orchestrator::gen_all_geos_data;
pub use orchestrator::GenModelResult;

// 实用工具函数
pub use utilities::{
    is_e3d_debug_enabled, is_e3d_info_enabled, is_e3d_trace_enabled, query_tubi_size,
};

// Mesh 处理函数
pub use mesh_processing::process_meshes_by_dbnos;

// 预检查相关类型
pub use precheck_coordinator::{run_precheck, PrecheckConfig, PrecheckStats};

// 迁入模块的重导出
pub use mesh_generate::{
    booleans_meshes_in_db, gen_inst_meshes, gen_meshes_in_db, process_meshes_bran,
    process_meshes_update_db, process_meshes_update_db_deep, process_meshes_update_db_deep_default,
    run_mesh_worker,
};
pub use query::*;
pub use resolve::*;
