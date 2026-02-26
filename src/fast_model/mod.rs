pub mod gen_model;

pub use gen_model::cata_model;
pub mod cata_cache_gen;

// [foyer-removal] 桩模块 —— 提供编译兼容，运行时不应被调用
pub mod instance_cache;
pub mod model_cache;
pub mod model_store;
pub mod foyer_cache;
pub mod cache_flush;



pub mod reuse_unit;



pub use gen_model::prim_model;



pub use gen_model::loop_model;



pub mod shared;



pub mod error_macros;

pub use error_macros::ModelErrorKind;



pub mod refno_errors;

pub use refno_errors::{

    REFNO_ERROR_STORE, RefnoErrorKind, RefnoErrorStage, RefnoErrorSummary, record_refno_error,

};



pub use gen_model::manifold_bool;

pub use gen_model::mesh_generate;



pub mod room_model; // 改进版本的房间模型

#[cfg(feature = "convex-runtime")]

pub mod convex_decomp;

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

pub mod room_worker; // 后台房间计算 Worker



// Re-export room model functions

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

pub use room_model::{

    IncrementalUpdateResult, RoomBuildStats, build_room_relations,

    build_room_relations_with_cancel, rebuild_room_relations_for_rooms,

    rebuild_room_relations_for_rooms_with_cancel, regenerate_room_models_by_keywords,

    update_room_relations_incremental, update_room_relations_incremental_with_cancel,

};



#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

pub use room_worker::{

    RoomWorker, RoomWorkerConfig, RoomWorkerTask, RoomWorkerTaskStatus, RoomTaskType,

};



pub mod cal_model;



pub use gen_model::pdms_inst;



pub use gen_model::resolve;



pub use gen_model::query;

// inst_relate/geo_relate 查询（v2）：用于绕开旧 schema 字段，便于导出/诊断

pub use gen_model::inst_query;

pub use gen_model::query_compat;

pub use gen_model::query_provider; // 统一查询提供者

pub use gen_model::db_meta_cache;
pub use gen_model::transform_cache;



pub mod utils;

pub mod precheck;



pub mod export_model;



// 重新导出 scene_tree 模块（用于替代 inst_relate_aabb）

pub use crate::scene_tree;

pub use export_model::{export_glb, export_gltf, export_instanced_bundle, model_exporter};

pub mod material_config;

pub mod unit_converter;



pub mod parquet_fast_model;



pub mod aabb_tree;



pub mod incremental;



// aabb_cache 已废弃，改用 DuckDB

// #[cfg(feature = "sqlite-index")]

// pub mod aabb_cache;



// session 模块保留供 web_server handlers 使用

#[cfg(feature = "sqlite-index")]

pub mod session;



pub mod concurrency;



// 碰撞检测：改用 DuckDB 空间查询

#[cfg(feature = "duckdb-feature")]

pub mod collision_detect;

#[cfg(feature = "duckdb-feature")]

pub use collision_detect::{CollisionConfig, CollisionDetector, CollisionEvent, CollisionStats};



use aios_core::RefU64;

use dashmap::{DashMap, DashSet};

// 优先使用新的 gen_model 模块的 API

pub use gen_model::*;

// [foyer-removal] CaptureConfig 桩
use std::path::PathBuf;
use once_cell::sync::OnceCell;

#[derive(Clone, Debug)]
pub struct CaptureConfig {
    pub output_dir: PathBuf,
    pub width: u32,
    pub height: u32,
    pub include_descendants: bool,
    pub views: Option<Vec<String>>,
    pub baseline_dir: Option<PathBuf>,
    pub diff_dir: Option<PathBuf>,
}

impl CaptureConfig {
    pub fn new(
        output_dir: PathBuf,
        width: u32,
        height: u32,
        include_descendants: bool,
        views: u8,
        baseline_dir: Option<PathBuf>,
        diff_dir: Option<PathBuf>,
    ) -> Self {
        let _ = views;
        Self { output_dir, width, height, include_descendants, views: None, baseline_dir, diff_dir }
    }
}

static CAPTURE_CONFIG: OnceCell<std::sync::Mutex<Option<CaptureConfig>>> = OnceCell::new();

pub fn set_capture_config(config: Option<CaptureConfig>) {
    let cell = CAPTURE_CONFIG.get_or_init(|| std::sync::Mutex::new(None));
    *cell.lock().unwrap() = config;
}

pub fn get_capture_config() -> Option<CaptureConfig> {
    CAPTURE_CONFIG.get().and_then(|m| m.lock().unwrap().clone())
}



// ✅ 已完成迁移：
// - 模型生成统一入口：gen_model::gen_all_geos_data（IndexTree 单管线）

// - process_meshes_by_dbnos → gen_model::mesh_processing (✅ 已完成)

// - query_tubi_size → gen_model::utilities (✅ 已完成)

// - ElementInfo 和 AiosDBManagerExt → 死代码，已移除

use once_cell::sync::Lazy;

use parry3d::bounding_volume::Aabb;



/// 全局缓存：已存在的 mesh geo_hash 到 AABB 的映射

pub static EXIST_MESH_GEO_HASHES: Lazy<DashMap<String, Aabb>> = Lazy::new(|| DashMap::new());



/// 从数据库预加载已存在的几何网格信息到内存缓存

/// 

/// 该函数扫描 `inst_geo` 表中所有已网格化 (`meshed = true`) 且拥有包围盒的数据，

/// 将其 `geo_hash` 和 `aabb` 载入 `EXIST_MESH_GEO_HASHES` 总，

/// 以便在后续生成的过程中通过内存直接跳过已处理项目，提升性能。

pub async fn preload_mesh_cache() -> anyhow::Result<()> {

    use aios_core::SUL_DB;

    use aios_core::types::PlantAabb;

    use surrealdb::types::SurrealValue;

    

    debug_model!("🚚 正在从数据库预加载几何缓存...");

    let start = std::time::Instant::now();

    

    // 查询所有已网格化的几何及其 AABB

    // 注意：geo_hash 在 SurrealDB 中是 inst_geo 的 ID

    let sql = "SELECT id, aabb.d as aabb_data FROM inst_geo WHERE meshed = true AND aabb != NONE";

    

    #[derive(serde::Deserialize, SurrealValue)]

    struct GeoCacheRow {

        id: surrealdb::types::RecordId,

        // 历史脏数据里可能出现 aabb.d 的内部字段为 null（如 mins/maxs 某一维为 null），

        // 直接反序列化成 PlantAabb 会导致整个预加载失败，进而让 mesh worker 全面崩溃。

        aabb_data: Option<serde_json::Value>,

    }

    

    let mut response = SUL_DB.query(sql).await?;

    let rows: Vec<GeoCacheRow> = response.take(0)?;

    

    let count = rows.len();

    for row in rows {

        // 使用 RecordId 的 key 字段作为缓存键

        let mesh_id = format!("{:?}", row.id.key);

        match row.aabb_data {

            Some(v) => match serde_json::from_value::<PlantAabb>(v) {

                Ok(plant_aabb) => {

                    // PlantAabb 是 tuple struct，使用 .0 获取内部 Aabb

                    EXIST_MESH_GEO_HASHES.insert(mesh_id, plant_aabb.0);

                }

                Err(e) => {

                    debug_model_warn!(

                        "⚠️ preload_mesh_cache: 跳过脏 aabb.d（mesh_id={}）: {}",

                        mesh_id,

                        e

                    );

                    // 仍写入 invalid，用于后续跳过重复 mesh 生成（避免无限重试）

                    EXIST_MESH_GEO_HASHES.insert(mesh_id, Aabb::new_invalid());

                }

            },

            None => {

                // 如果只有 meshed=true 但没 aabb，存一个空的，仅用于跳过生成

                EXIST_MESH_GEO_HASHES.insert(mesh_id, Aabb::new_invalid());

            }

        };

    }

    

    debug_model!(

        "✅ 几何缓存预加载完成: 已载入 {} 个记录，耗时 {} ms",

        count,

        start.elapsed().as_millis()

    );

    

    Ok(())

}



// 以下重导出已通过 pub use gen_model::* 覆盖（query/resolve/mesh_generate 已迁入 gen_model）



pub const SEND_INST_SIZE: usize = 500;

// Re-export debug macros from aios_core

pub use aios_core::{debug_model, debug_model_debug, debug_model_trace, debug_model_warn};



// 错误日志模式的全局变量

static DEBUG_MODEL_ERRORS_ONLY: std::sync::atomic::AtomicBool =

    std::sync::atomic::AtomicBool::new(false);



/// 设置调试模型错误日志模式

pub fn set_debug_model_errors_only(enabled: bool) {

    DEBUG_MODEL_ERRORS_ONLY.store(enabled, std::sync::atomic::Ordering::Relaxed);

}



/// 检查是否启用了调试模型错误日志模式

pub fn is_debug_model_errors_only() -> bool {

    DEBUG_MODEL_ERRORS_ONLY.load(std::sync::atomic::Ordering::Relaxed)

}



/// 智能调试宏：在错误日志模式下只输出错误，否则输出所有调试信息

#[macro_export]

macro_rules! smart_debug_model {

    ($($arg:tt)*) => {{

        if aios_core::is_debug_model_enabled() {

            if $crate::fast_model::is_debug_model_errors_only() {

                // 在错误日志模式下，只输出包含错误关键词的信息

                let message = format!($($arg)*);

                if message.contains("错误") || message.contains("失败") || message.contains("Error") || message.contains("error") || message.contains("ERROR") {

                    println!("{}", message);

                }

            } else {

                // 正常调试模式，输出所有信息

                println!($($arg)*);

            }

        }

    }};

}



/// 智能调试宏：专门用于输出错误信息（在错误日志模式下总是输出）

#[macro_export]

macro_rules! smart_debug_error {

    ($($arg:tt)*) => {{

        if aios_core::is_debug_model_enabled() {

            println!($($arg)*);

        }

    }};

}



/// 智能包装器：包装 debug_model_debug 宏

#[macro_export]

macro_rules! smart_debug_model_debug {

    ($($arg:tt)*) => {{

        if aios_core::is_debug_model_enabled() {

            if $crate::fast_model::is_debug_model_errors_only() {

                // 在错误日志模式下，只输出包含错误关键词的信息

                let message = format!($($arg)*);

                if message.contains("错误") || message.contains("失败") || message.contains("Error") || message.contains("error") || message.contains("ERROR") ||

                   message.contains("Failed") || message.contains("failed") || message.contains("❌") || message.contains("⚠️") {

                    $crate::fast_model::debug_model_debug!($($arg)*);

                }

            } else {

                // 正常调试模式，输出所有信息

                $crate::fast_model::debug_model_debug!($($arg)*);

            }

        }

    }};

}



/// 智能包装器：包装 debug_model_trace 宏

#[macro_export]

macro_rules! smart_debug_model_trace {

    ($($arg:tt)*) => {{

        if aios_core::is_debug_model_enabled() {

            if $crate::fast_model::is_debug_model_errors_only() {

                // 在错误日志模式下，trace 信息通常不包含错误，所以跳过

            } else {

                // 正常调试模式，输出所有信息

                $crate::fast_model::debug_model_trace!($($arg)*);

            }

        }

    }};

}



/// 智能包装器：包装 debug_model_warn 宏

#[macro_export]

macro_rules! smart_debug_model_warn {

    ($($arg:tt)*) => {{

        if aios_core::is_debug_model_enabled() {

            if $crate::fast_model::is_debug_model_errors_only() {

                // 在错误日志模式下，警告信息通常包含错误相关内容，所以输出

                $crate::fast_model::debug_model_warn!($($arg)*);

            } else {

                // 正常调试模式，输出所有信息

                $crate::fast_model::debug_model_warn!($($arg)*);

            }

        }

    }};

}
