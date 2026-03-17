pub mod gen_model;

pub use gen_model::cata_model;
pub mod cata_cache_gen;

// [foyer-removal] 桩模块 —— 提供编译兼容，运行时不应被调用
pub mod cache_flush;
pub mod foyer_cache;
pub mod instance_cache;
pub mod model_cache;
pub mod model_store;

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
    CoarseAabbDiagnostic, IncrementalUpdateResult, RoomBuildStats, build_room_relations,
    build_room_relations_with_cancel, diagnose_coarse_aabb_intersection,
    rebuild_room_relations_for_rooms, rebuild_room_relations_for_rooms_with_cancel,
    regenerate_room_models_by_keywords, update_room_relations_incremental,
    update_room_relations_incremental_with_cancel,
};

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
pub use room_worker::{
    RoomTaskType, RoomWorker, RoomWorkerConfig, RoomWorkerTask, RoomWorkerTaskStatus,
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

pub mod aabb_tree;

pub mod incremental;

// aabb_cache 已废弃

// #[cfg(feature = "sqlite-index")]

// pub mod aabb_cache;

// session 模块保留供 web_server handlers 使用

#[cfg(feature = "sqlite-index")]
pub mod session;

pub mod concurrency;

// 碰撞检测模块已移除

use aios_core::RefU64;

use dashmap::{DashMap, DashSet};

// 优先使用新的 gen_model 模块的 API

pub use gen_model::*;

#[cfg(feature = "sqlite-index")]
pub use gen_model::orchestrator::update_sqlite_spatial_index_from_cache;

// [foyer-removal] CaptureConfig 桩
use once_cell::sync::OnceCell;
use std::path::PathBuf;

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
        Self {
            output_dir,
            width,
            height,
            include_descendants,
            views: if views > 1 {
                Some((2..=views).map(|idx| format!("view{:02}", idx)).collect())
            } else {
                None
            },
            baseline_dir,
            diff_dir,
        }
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

// ── AABB rkyv 缓存 ──

const AABB_CACHE_FILENAME: &str = "aabb_cache.rkyv";
const AABB_CACHE_VERSION: u32 = 1;

#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct AabbCacheFileV1 {
    pub version: u32,
    pub entries: Vec<AabbCacheEntryV1>,
}

#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct AabbCacheEntryV1 {
    pub geo_hash: u64,
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
}

/// 将 `EXIST_MESH_GEO_HASHES` 中有效的 AABB 持久化到 `meshes/aabb_cache.rkyv`
pub fn save_aabb_cache_to_disk() {
    let mesh_dir = aios_core::get_db_option().get_meshes_path();
    let cache_path = mesh_dir.join(AABB_CACHE_FILENAME);
    if let Err(e) = std::fs::create_dir_all(&mesh_dir) {
        eprintln!(
            "[aabb_cache] 创建缓存目录失败: {} - {}",
            mesh_dir.display(),
            e
        );
        return;
    }

    let tmp_path = mesh_dir.join(format!(
        "{}.{}.{}.tmp",
        AABB_CACHE_FILENAME,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));

    let mut entries = Vec::new();
    for kv in EXIST_MESH_GEO_HASHES.iter() {
        let aabb = *kv.value();
        let ext_mag = aabb.extents().magnitude();
        if ext_mag > 1e-4 && ext_mag < f32::INFINITY {
            if let Ok(geo_hash) = kv.key().parse::<u64>() {
                entries.push(AabbCacheEntryV1 {
                    geo_hash,
                    mins: [aabb.mins.x, aabb.mins.y, aabb.mins.z],
                    maxs: [aabb.maxs.x, aabb.maxs.y, aabb.maxs.z],
                });
            }
        }
    }

    let file = AabbCacheFileV1 {
        version: AABB_CACHE_VERSION,
        entries,
    };

    match rkyv::to_bytes::<rkyv::rancor::Error>(&file) {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(&tmp_path, &bytes) {
                eprintln!("[aabb_cache] 写入临时文件失败: {}", e);
                return;
            }
            if let Err(e) = std::fs::rename(&tmp_path, &cache_path) {
                eprintln!("[aabb_cache] rename 失败: {}", e);
                return;
            }
            debug_model!(
                "✅ AABB 缓存已保存: {} 条有效 AABB → {} ({} bytes)",
                file.entries.len(),
                cache_path.display(),
                bytes.len(),
            );
        }
        Err(e) => {
            eprintln!("[aabb_cache] rkyv 序列化失败: {:?}", e);
        }
    }
}

/// 从 `meshes/aabb_cache.rkyv` 加载 AABB 缓存到 `EXIST_MESH_GEO_HASHES`
///
/// 返回成功加载的有效 AABB 条数。文件不存在或解析失败时返回 0。
fn load_aabb_cache_from_disk(mesh_dir: &std::path::Path) -> usize {
    use parry3d::math::Point;

    let cache_path = mesh_dir.join(AABB_CACHE_FILENAME);
    let data = match std::fs::read(&cache_path) {
        Ok(d) => d,
        Err(_) => return 0,
    };

    let file: AabbCacheFileV1 =
        match rkyv::from_bytes::<AabbCacheFileV1, rkyv::rancor::Error>(&data) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("[aabb_cache] rkyv 反序列化失败，将忽略缓存文件: {:?}", e);
                return 0;
            }
        };

    if file.version != AABB_CACHE_VERSION {
        eprintln!(
            "[aabb_cache] 版本不匹配: file={} expected={}, 忽略",
            file.version, AABB_CACHE_VERSION
        );
        return 0;
    }

    let mut count = 0usize;
    for entry in &file.entries {
        let aabb = Aabb::new(Point::from(entry.mins), Point::from(entry.maxs));
        EXIST_MESH_GEO_HASHES.insert(entry.geo_hash.to_string(), aabb);
        count += 1;
    }
    count
}

/// 从 `aabb_cache.rkyv` 预加载几何网格缓存
///
/// 以 `aabb_cache.rkyv` 为 mesh 存在性的唯一判据，不再扫描磁盘 GLB 文件。
pub fn preload_mesh_cache() {
    let mesh_dir = aios_core::get_db_option().get_meshes_path();

    // 从 rkyv 缓存读取有效 AABB
    let cache_count = load_aabb_cache_from_disk(&mesh_dir);

    debug_model!(
        "✅ 缓存预加载: rkyv={} 条有效 AABB（以 aabb_cache.rkyv 为唯一判据）",
        cache_count,
    );
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
