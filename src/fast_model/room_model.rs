use aios_core::RecordId;
use aios_core::accel_tree::acceleration_tree::RStarBoundingBox;
use aios_core::options::DbOption;
use aios_core::room::algorithm::*;
use aios_core::shape::pdms_shape::PlantMesh;
use aios_core::{GeomInstQuery, GeomPtsQuery, ModelHashInst, RefU64, SUL_DB};
use aios_core::{RefnoEnum, init_demo_test_surreal, init_test_surreal};

// 使用改进的房间查询模块（暂时注释掉，因为这些模块可能不存在）
// #[cfg(all(not(target_arch = "wasm32"), any(feature = "sqlite-index", feature = "duckdb-feature")))]
// use aios_core::room::query_v2::{
//     query_room_number_by_point_v2,
//     batch_query_room_numbers,
//     get_room_query_stats,
//     clear_geometry_cache,
//     preheat_geometry_cache,
// };

// #[cfg(all(not(target_arch = "wasm32"), any(feature = "sqlite-index", feature = "duckdb-feature")))]
// use aios_core::spatial::hybrid_index::{get_hybrid_index, QueryOptions};

use bevy_transform::TransformPoint;
use bevy_transform::components::Transform;
use dashmap::DashMap;
use glam::{Mat4, Vec3};
use itertools::Itertools;
use parry3d::bounding_volume::Aabb;
use parry3d::math::{Isometry, Vector};
use parry3d::math::{Point, Real};
use parry3d::query::PointQuery;
use parry3d::query::{Ray, RayCast};
use parry3d::shape::{TriMesh, TriMeshFlags};
use rayon::prelude::{IntoParallelRefIterator, ParallelIterator};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

#[cfg(feature = "duckdb-feature")]
use crate::fast_model::export_model::get_or_init_duckdb_reader;

/// 房间关系构建统计信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomBuildStats {
    pub total_rooms: usize,
    pub total_panels: usize,
    pub total_components: usize,
    pub build_time_ms: u64,
    pub cache_hit_rate: f32,
    pub memory_usage_mb: f32,
}

#[derive(Debug, Clone, Copy)]
struct RoomComputeOptions {
    inside_tol: f32,
    concurrency: usize,
    candidate_limit: Option<usize>,
    candidate_concurrency: usize,
}

impl Default for RoomComputeOptions {
    fn default() -> Self {
        Self {
            inside_tol: 0.1,
            concurrency: default_room_concurrency(),
            candidate_limit: default_candidate_limit(),
            candidate_concurrency: default_candidate_concurrency(),
        }
    }
}

fn default_room_concurrency() -> usize {
    std::env::var("ROOM_RELATION_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|c| *c > 0)
        .unwrap_or(4)
}

fn default_candidate_limit() -> Option<usize> {
    std::env::var("ROOM_RELATION_CANDIDATE_LIMIT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|c| *c > 0)
}

fn default_candidate_concurrency() -> usize {
    std::env::var("ROOM_RELATION_CANDIDATE_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|c| *c > 0)
        .unwrap_or_else(default_room_concurrency)
}

#[derive(Default)]
struct CacheMetrics {
    plant_hits: AtomicU64,
    plant_misses: AtomicU64,
    trimesh_hits: AtomicU64,
    trimesh_misses: AtomicU64,
}

impl CacheMetrics {
    const fn new() -> Self {
        Self {
            plant_hits: AtomicU64::new(0),
            plant_misses: AtomicU64::new(0),
            trimesh_hits: AtomicU64::new(0),
            trimesh_misses: AtomicU64::new(0),
        }
    }

    fn record_plant_hit(&self) {
        self.plant_hits.fetch_add(1, Ordering::Relaxed);
    }

    fn record_plant_miss(&self) {
        self.plant_misses.fetch_add(1, Ordering::Relaxed);
    }

    fn record_trimesh_hit(&self) {
        self.trimesh_hits.fetch_add(1, Ordering::Relaxed);
    }

    fn record_trimesh_miss(&self) {
        self.trimesh_misses.fetch_add(1, Ordering::Relaxed);
    }

    fn reset(&self) {
        self.plant_hits.store(0, Ordering::Relaxed);
        self.plant_misses.store(0, Ordering::Relaxed);
        self.trimesh_hits.store(0, Ordering::Relaxed);
        self.trimesh_misses.store(0, Ordering::Relaxed);
    }

    fn hit_rate(&self) -> f32 {
        let hits = self.plant_hits.load(Ordering::Relaxed) as f32
            + self.trimesh_hits.load(Ordering::Relaxed) as f32;
        let misses = self.plant_misses.load(Ordering::Relaxed) as f32
            + self.trimesh_misses.load(Ordering::Relaxed) as f32;
        let total = hits + misses;
        if total == 0.0 { 0.0 } else { hits / total }
    }
}

/// 改进的几何网格缓存
/// 使用 Arc 和 DashMap 提升并发性能和内存效率
static ENHANCED_GEOMETRY_CACHE: tokio::sync::OnceCell<DashMap<String, Arc<PlantMesh>>> =
    tokio::sync::OnceCell::const_new();

/// 预烘 TriMesh(L0) 缓存（未应用实例/世界变换）
static ENHANCED_TRIMESH_CACHE: tokio::sync::OnceCell<DashMap<String, Arc<TriMesh>>> =
    tokio::sync::OnceCell::const_new();

static CACHE_METRICS: CacheMetrics = CacheMetrics::new();

async fn get_enhanced_geometry_cache() -> &'static DashMap<String, Arc<PlantMesh>> {
    ENHANCED_GEOMETRY_CACHE
        .get_or_init(|| async { DashMap::new() })
        .await
}

async fn get_enhanced_trimesh_cache() -> &'static DashMap<String, Arc<TriMesh>> {
    ENHANCED_TRIMESH_CACHE
        .get_or_init(|| async { DashMap::new() })
        .await
}

/// 改进版本的房间关系构建函数
///
/// 主要改进：
/// 1. 使用混合空间索引提升查询性能
/// 2. 优化几何缓存机制，减少重复加载
/// 3. 添加详细的性能统计和监控
/// 4. 支持并发处理和批量操作
#[cfg(all(not(target_arch = "wasm32"), any(feature = "sqlite-index", feature = "duckdb-feature")))]
pub async fn build_room_relations(db_option: &DbOption) -> anyhow::Result<RoomBuildStats> {
    info!("开始构建房间关系 (改进版本)");

    let mesh_dir = db_option.get_meshes_path();
    let room_key_words = db_option.get_room_key_word();
    let compute_options = RoomComputeOptions::default();

    CACHE_METRICS.reset();

    // 1. 构建房间面板映射关系
    let room_panel_map = build_room_panels_relate(&room_key_words).await?;
    let exclude_panel_refnos = room_panel_map
        .iter()
        .map(|(_, _, panel_refnos)| panel_refnos.clone())
        .flatten()
        .collect::<HashSet<_>>();

    info!("找到 {} 个房间面板映射关系", room_panel_map.len());

    let stats = compute_room_relations(
        &mesh_dir,
        room_panel_map,
        exclude_panel_refnos,
        compute_options,
    )
    .await;

    info!(
        "房间关系构建完成: 处理 {} 个房间, {} 个面板, {} 个构件, 耗时 {:?}, 缓存命中率 {:.2}%",
        stats.total_rooms,
        stats.total_panels,
        stats.total_components,
        Duration::from_millis(stats.build_time_ms),
        stats.cache_hit_rate * 100.0
    );

    Ok(stats)
}

#[cfg(all(not(target_arch = "wasm32"), any(feature = "sqlite-index", feature = "duckdb-feature")))]
async fn compute_room_relations(
    mesh_dir: &PathBuf,
    room_panel_map: Vec<(RefnoEnum, String, Vec<RefnoEnum>)>,
    exclude_panel_refnos: HashSet<RefnoEnum>,
    options: RoomComputeOptions,
) -> RoomBuildStats {
    let start_time = Instant::now();
    let total_panels = exclude_panel_refnos.len();
    let exclude_panel_refnos = Arc::new(exclude_panel_refnos);

    use futures::stream::{self, StreamExt};

    let results = stream::iter(room_panel_map)
        .map(|(room_refno, room_num, panel_refnos)| {
            let mesh_dir = mesh_dir.clone();
            let exclude_panel_refnos = exclude_panel_refnos.clone();
            let room_num = room_num.clone();
            let options = options;
            async move {
                let mut room_components = 0;

                for panel_refno in panel_refnos {
                    room_components += process_panel_for_room(
                        &mesh_dir,
                        panel_refno,
                        &room_num,
                        exclude_panel_refnos.as_ref(),
                        options,
                    )
                    .await;
                }

                (room_refno, room_components)
            }
        })
        .buffer_unordered(options.concurrency.max(1))
        .collect::<Vec<_>>()
        .await;

    let total_rooms = results.len();
    let total_components: usize = results.iter().map(|(_, count)| *count).sum();
    let build_time = start_time.elapsed();

    RoomBuildStats {
        total_rooms,
        total_panels,
        total_components,
        build_time_ms: build_time.as_millis() as u64,
        cache_hit_rate: CACHE_METRICS.hit_rate(),
        memory_usage_mb: estimate_memory_usage().await,
    }
}

/// 构建房间面板查询 SQL（通过 OWNER 字段查询 FRMW -> SBFR -> PANE 层级）
fn build_room_panel_query_sql(room_key_word: &[String]) -> String {
    let filter = if room_key_word.is_empty() {
        "true".to_string()
    } else {
        room_key_word
            .iter()
            .map(|x| format!("'{}' in NAME", x.replace('\'', "''")))
            .join(" or ")
    };

    #[cfg(feature = "project_hd")]
    {
        // 通过 OWNER 字段递归查询：FRMW -> SBFR -> PANE
        return format!(
            r#"
            select value [
                id,
                array::last(string::split(NAME, '-')),
                array::flatten((select value (select value REFNO from PANE where OWNER = $parent.REFNO) from SBFR where OWNER = $parent.REFNO))
            ] from FRMW where NAME IS NOT NONE AND ({filter})
        "#
        );
    }

    #[cfg(feature = "project_hh")]
    {
        // project_hh: 从 SBFR 查询 PANE
        return format!(
            r#"
            select value [
                id,
                array::last(string::split(NAME, '-')),
                (select value REFNO from PANE where OWNER = $parent.REFNO)
            ] from SBFR where NAME IS NOT NONE AND ({filter})
        "#
        );
    }

    #[cfg(not(any(feature = "project_hd", feature = "project_hh")))]
    {
        // 默认：从 FRMW 查询 SBFR -> PANE
        format!(
            r#"
            select value [
                id,
                array::last(string::split(NAME, '-')),
                array::flatten((select value (select value REFNO from PANE where OWNER = $parent.REFNO) from SBFR where OWNER = $parent.REFNO))
            ] from FRMW where NAME IS NOT NONE AND ({filter})
        "#
        )
    }
}

/// 改进版本的房间面板关系构建
async fn build_room_panels_relate(
    room_key_word: &Vec<String>,
) -> anyhow::Result<Vec<(RefnoEnum, String, Vec<RefnoEnum>)>> {
    #[cfg(feature = "project_hd")]
    return build_room_panels_relate_common(room_key_word, match_room_name_hd).await;

    #[cfg(feature = "project_hh")]
    return build_room_panels_relate_common(room_key_word, match_room_name_hh).await;

    // 默认情况
    build_room_panels_relate_common(room_key_word, |_| true).await
}

/// 仅构建房间面板映射（不写入关系）
async fn build_room_panels_relate_for_query(
    room_key_word: &Vec<String>,
) -> anyhow::Result<Vec<(RefnoEnum, String, Vec<RefnoEnum>)>> {
    #[cfg(feature = "project_hd")]
    return build_room_panels_relate_common_with_persist(room_key_word, match_room_name_hd, false)
        .await;

    #[cfg(feature = "project_hh")]
    return build_room_panels_relate_common_with_persist(room_key_word, match_room_name_hh, false)
        .await;

    build_room_panels_relate_common_with_persist(room_key_word, |_| true, false).await
}

/// 改进版本的房间面板关系构建通用函数
async fn build_room_panels_relate_common<F>(
    room_key_word: &Vec<String>,
    match_room_fn: F,
) -> anyhow::Result<Vec<(RefnoEnum, String, Vec<RefnoEnum>)>>
where
    F: Fn(&str) -> bool + Send + Sync,
{
    build_room_panels_relate_common_with_persist(room_key_word, match_room_fn, true).await
}

async fn build_room_panels_relate_common_with_persist<F>(
    room_key_word: &Vec<String>,
    match_room_fn: F,
    persist: bool,
) -> anyhow::Result<Vec<(RefnoEnum, String, Vec<RefnoEnum>)>>
where
    F: Fn(&str) -> bool + Send + Sync,
{
    let start_time = Instant::now();

    let sql = build_room_panel_query_sql(room_key_word);

    let mut response = SUL_DB.query(sql).await?;
    let raw_result: Vec<(RecordId, String, Vec<RecordId>)> = response.take(0)?;

    // 转换并过滤结果
    let room_groups: Vec<(RefnoEnum, String, Vec<RefnoEnum>)> = raw_result
        .into_iter()
        .filter_map(|(room_thing, room_num, panel_things)| {
            // 验证房间号格式
            if !match_room_fn(&room_num) {
                debug!("跳过不匹配的房间号: {}", room_num);
                return None;
            }

            // 这里克隆一次以避免后续日志对 room_thing 的使用发生 move
            let room_refno = RefnoEnum::from(room_thing.clone());
            if !room_refno.is_valid() {
                warn!("无效的房间引用号: {:?}", room_thing);
                return None;
            }

            let panel_refnos: Vec<RefnoEnum> = panel_things
                .into_iter()
                .filter_map(|panel_thing| {
                    let panel_refno = RefnoEnum::from(panel_thing);
                    if panel_refno.is_valid() {
                        Some(panel_refno)
                    } else {
                        None
                    }
                })
                .collect();

            if panel_refnos.is_empty() {
                debug!("房间 {} 没有关联的面板", room_num);
                return None;
            }

            Some((room_refno, room_num, panel_refnos))
        })
        .collect();

    // 批量创建房间面板关系
    if persist && !room_groups.is_empty() {
        create_room_panel_relations_batch(&room_groups).await?;
    }

    if persist {
        info!(
            "房间面板关系构建完成: {} 个关系, 耗时 {:?}",
            room_groups.len(),
            start_time.elapsed()
        );
    } else {
        info!(
            "房间面板映射构建完成(未写入关系): {} 个关系, 耗时 {:?}",
            room_groups.len(),
            start_time.elapsed()
        );
    }

    Ok(room_groups)
}

/// 批量创建房间面板关系
async fn create_room_panel_relations_batch(
    room_groups: &[(RefnoEnum, String, Vec<RefnoEnum>)],
) -> anyhow::Result<()> {
    let mut sql_statements = Vec::new();

    for (room_refno, room_num_str, panel_refnos) in room_groups {
        let sql = format!(
            "relate {}->room_panel_relate->[{}] set room_num='{}';",
            room_refno.to_pe_key(),
            panel_refnos.iter().map(|x| x.to_pe_key()).join(","),
            room_num_str
        );
        sql_statements.push(sql);
    }

    // 批量执行 SQL
    let batch_sql = sql_statements.join("\n");
    SUL_DB.query(batch_sql).await?;

    Ok(())
}

#[cfg(all(not(target_arch = "wasm32"), any(feature = "sqlite-index", feature = "duckdb-feature")))]
async fn process_panel_for_room(
    mesh_dir: &PathBuf,
    panel_refno: RefnoEnum,
    room_num: &str,
    exclude_panel_refnos: &HashSet<RefnoEnum>,
    options: RoomComputeOptions,
) -> usize {
    match cal_room_refnos_with_options(
        mesh_dir,
        panel_refno,
        exclude_panel_refnos,
        options,
    )
    .await
    {
        Ok(refnos) => {
            if refnos.is_empty() {
                return 0;
            }

            if let Err(e) = save_room_relate(panel_refno, &refnos, room_num).await {
                error!("保存房间关系失败: panel={}, error={}", panel_refno, e);
                0
            } else {
                refnos.len()
            }
        }
        Err(e) => {
            warn!("计算房间构件失败: panel={}, error={}", panel_refno, e);
            0
        }
    }
}

/// 改进版本的房间构件计算（基于关键点检测）
#[cfg(all(not(target_arch = "wasm32"), any(feature = "sqlite-index", feature = "duckdb-feature")))]
pub async fn cal_room_refnos(
    mesh_dir: &PathBuf,
    panel_refno: RefnoEnum,
    exclude_refnos: &HashSet<RefnoEnum>,
    inside_tol: f32,
) -> anyhow::Result<HashSet<RefnoEnum>> {
    let mut options = RoomComputeOptions::default();
    options.inside_tol = inside_tol;

    cal_room_refnos_with_options(mesh_dir, panel_refno, exclude_refnos, options).await
}

#[cfg(all(not(target_arch = "wasm32"), any(feature = "sqlite-index", feature = "duckdb-feature")))]
async fn cal_room_refnos_with_options(
    mesh_dir: &PathBuf,
    panel_refno: RefnoEnum,
    exclude_refnos: &HashSet<RefnoEnum>,
    options: RoomComputeOptions,
) -> anyhow::Result<HashSet<RefnoEnum>> {
    let start_time = Instant::now();
    let inside_tol = options.inside_tol;

    // 步骤 1：查询面板的几何实例
    let panel_geom_insts: Vec<GeomInstQuery> = aios_core::query_insts(&[panel_refno], true)
        .await
        .unwrap_or_default();

    if panel_geom_insts.is_empty() {
        debug!("面板 {} 没有几何实例", panel_refno);
        return Ok(Default::default());
    }

    // 步骤 2：加载面板 TriMesh（用于点包含测试），并合并面板 AABB
    let mut panel_meshes: Vec<Arc<TriMesh>> = Vec::new();
    let mut panel_aabb: Option<Aabb> = None;

    for geom_inst in &panel_geom_insts {
        let Some(ref world_aabb) = geom_inst.world_aabb else { continue };
        let geom_aabb: Aabb = world_aabb.clone().into();
        panel_aabb = Some(match panel_aabb {
            None => geom_aabb,
            Some(acc) => merge_aabb(&acc, &geom_aabb),
        });

        if geom_inst.insts.is_empty() {
            debug!("面板 {} 的 insts 数组为空", panel_refno);
            continue;
        }

        for inst in &geom_inst.insts {
            match load_geometry_with_enhanced_cache(mesh_dir, &inst.geo_hash, geom_inst.world_trans, inst).await {
                Ok(mesh) => panel_meshes.push(mesh),
                Err(e) => {
                    warn!("加载面板几何文件失败: {}, error: {}", inst.geo_hash, e);
                }
            }
        }
    }

    let panel_aabb = match panel_aabb {
        Some(aabb) => aabb,
        None => {
            debug!("面板 {} 没有可用 AABB", panel_refno);
            return Ok(Default::default());
        }
    };

    if panel_meshes.is_empty() {
        debug!("面板 {} 无可用 TriMesh", panel_refno);
        return Ok(Default::default());
    }

    // 步骤 3：粗算 - 通过空间索引查询候选构件
    let coarse_start = Instant::now();

    // 克隆排除列表以避免生命周期问题
    let exclude_set: HashSet<RefU64> = exclude_refnos.iter().map(|r| r.refno()).collect();
    let candidate_limit = options.candidate_limit;

    let candidates = tokio::task::spawn_blocking({
        let panel_aabb = panel_aabb;
        let exclude_set = exclude_set;
        let panel_refno = panel_refno.clone();
        let candidate_limit = candidate_limit;

        move || -> anyhow::Result<Vec<RefnoEnum>> {
            #[cfg(feature = "duckdb-feature")]
            {
                let reader = match get_or_init_duckdb_reader() {
                    Ok(reader) => reader,
                    Err(e) => {
                        warn!("初始化 DuckDB 读取器失败: {}", e);
                        return Ok(vec![]);
                    }
                };

                let dbno = panel_refno.refno().get_0();
                let refno_strs = match reader.query_by_bounding_box_in_dbno(
                    dbno,
                    panel_aabb.mins.x as f64,
                    panel_aabb.mins.y as f64,
                    panel_aabb.mins.z as f64,
                    panel_aabb.maxs.x as f64,
                    panel_aabb.maxs.y as f64,
                    panel_aabb.maxs.z as f64,
                ) {
                    Ok(refnos) => refnos,
                    Err(e) => {
                        warn!("DuckDB 粗算查询失败: {}", e);
                        return Ok(vec![]);
                    }
                };

                let mut refnos = Vec::new();
                for refno_str in refno_strs {
                    let candidate = RefnoEnum::from(refno_str.as_str());
                    if !candidate.is_valid() {
                        continue;
                    }
                    if candidate == panel_refno {
                        continue;
                    }
                    if exclude_set.contains(&candidate.refno()) {
                        continue;
                    }
                    refnos.push(candidate);
                    if let Some(limit) = candidate_limit {
                        if refnos.len() >= limit {
                            warn!("面板 {} 候选数达到上限 {}，可能存在截断", panel_refno, limit);
                            break;
                        }
                    }
                }

                Ok(refnos)
            }
            #[cfg(not(feature = "duckdb-feature"))]
            {
                Ok(vec![])
            }
        }
    })
    .await??;

    let candidate_count = candidates.len();
    debug!(
        "🔍 粗算完成: 耗时 {:?}, 候选数 {}",
        coarse_start.elapsed(),
        candidate_count
    );

    // 步骤 4：细算 - 对每个候选构件进行关键点检测
    let fine_start = Instant::now();
    let panel_meshes = Arc::new(panel_meshes);

    use futures::stream::{self, StreamExt};

    let within_refnos: HashSet<RefnoEnum> = stream::iter(candidates)
        .map(|candidate_refno| {
            let panel_meshes = panel_meshes.clone();
            async move {
                // 查询候选构件的几何实例
                let candidate_insts = match aios_core::query_insts(&[candidate_refno], true).await {
                    Ok(insts) => insts,
                    Err(e) => {
                        warn!(
                            "查询候选构件几何实例失败: {}, error: {}",
                            candidate_refno, e
                        );
                        return None;
                    }
                };

                if candidate_insts.is_empty() {
                    return None;
                }

                // 提取候选构件的关键点
                let key_points = extract_geom_key_points(&candidate_insts);

                // 判断关键点是否在面板内
                if is_geom_in_panel(&key_points, panel_meshes.as_ref(), inside_tol) {
                    Some(candidate_refno)
                } else {
                    None
                }
            }
        })
        .buffer_unordered(options.candidate_concurrency.max(1))
        .filter_map(|item| async move { item })
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect();

    debug!(
        "✅ 细算完成: 耗时 {:?}, 结果数 {}",
        fine_start.elapsed(),
        within_refnos.len()
    );

    info!(
        "面板 {} 房间计算完成: 总耗时 {:?}, 粗算 {} -> 细算 {}",
        panel_refno,
        start_time.elapsed(),
        candidate_count,
        within_refnos.len()
    );

    Ok(within_refnos)
}

/// 使用增强缓存加载几何文件（优先使用 L0，回退到 L1）
async fn load_geometry_with_enhanced_cache(
    mesh_dir: &PathBuf,
    geo_hash: &str,
    world_trans: aios_core::PlantTransform,
    inst: &ModelHashInst,
) -> anyhow::Result<Arc<TriMesh>> {
    let cache = get_enhanced_geometry_cache().await;
    let trimesh_cache = get_enhanced_trimesh_cache().await;

    // mesh_dir 已经指向带 LOD 的目录（如 assets/meshes/lod_L1）
    // 需要获取基础目录（assets/meshes）来尝试不同的 LOD 级别
    let base_mesh_dir = mesh_dir
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| mesh_dir.clone());

    // 尝试的 LOD 级别顺序：L0 -> L1 -> L2 -> L3
    let lod_levels = ["L0", "L1", "L2", "L3"];

    for lod_level in lod_levels.iter() {
        let cache_key = format!("{}_{}", geo_hash, lod_level);
        
        // 1. 检查 TriMesh 缓存 (用于 GLB/GLTF 直接加载的结果)
        if let Some(cached_trimesh) = trimesh_cache.get(&cache_key) {
             // 这里的 cache 存储的是原始几何体的 TriMesh
             // 我们需要应用实例变换
             let transformed_mesh = transform_tri_mesh(&cached_trimesh, (world_trans * inst.transform).to_matrix());
             CACHE_METRICS.record_trimesh_hit();
             return Ok(Arc::new(transformed_mesh));
        }

        // 2. 检查 PlantMesh 缓存 (用于 .mesh 文件)
        if let Some(cached_mesh) = cache.get(&cache_key) {
            // 从缓存的 PlantMesh 构建 TriMesh
            if let Some(tri_mesh) = cached_mesh.get_tri_mesh_with_flag(
                (world_trans * inst.transform).to_matrix(),
                TriMeshFlags::ORIENTED | TriMeshFlags::MERGE_DUPLICATE_VERTICES,
            ) {
                CACHE_METRICS.record_plant_hit();
                return Ok(Arc::new(tri_mesh));
            }
        }

        let lod_subdir = format!("lod_{}", lod_level);
        
        // 3. 尝试加载 GLB/GLTF
        let glb_file_names = [
            format!("{}_{}.glb", geo_hash, lod_level),
            format!("{}_{}.gltf", geo_hash, lod_level),
        ];

        for glb_name in &glb_file_names {
            let glb_path = base_mesh_dir.join(&lod_subdir).join(glb_name);
            if glb_path.exists() {
                let glb_path_clone = glb_path.clone();
                match tokio::task::spawn_blocking(move || load_tri_mesh_from_glb(&glb_path_clone)).await {
                     Ok(Ok(trimesh)) => {
                         let trimesh_arc = Arc::new(trimesh);
                         // 存入 TriMesh 缓存
                         trimesh_cache.insert(cache_key.clone(), trimesh_arc.clone());
                         CACHE_METRICS.record_trimesh_miss();

                         // 应用变换返回
                         let transformed_mesh = transform_tri_mesh(&trimesh_arc, (world_trans * inst.transform).to_matrix());
                         return Ok(Arc::new(transformed_mesh));
                     }
                     Ok(Err(e)) => {
                         warn!("加载 GLB 失败: path={:?}, error={}", glb_path, e);
                     }
                     _ => {}
                }
            }
        }

        // 4. 尝试加载 .mesh 文件 (旧流程)
        let file_name = format!("{}_{}.mesh", geo_hash, lod_level);
        let file_path = base_mesh_dir.join(&lod_subdir).join(&file_name);

        if !file_path.exists() {
            continue; // 尝试下一个 LOD 级别
        }

        let file_path_clone = file_path.clone();
        match tokio::task::spawn_blocking(move || PlantMesh::des_mesh_file(&file_path_clone)).await {
            Ok(Ok(mesh)) => {
                // 构建 TriMesh
                if let Some(tri_mesh) = mesh.get_tri_mesh_with_flag(
                    (world_trans * inst.transform).to_matrix(),
                    TriMeshFlags::ORIENTED | TriMeshFlags::MERGE_DUPLICATE_VERTICES,
                ) {
                    // 更新缓存
                    cache.insert(cache_key, Arc::new(mesh));
                    CACHE_METRICS.record_plant_miss();

                    // 缓存管理
                    if cache.len() > 2000 {
                        cleanup_geometry_cache(&cache).await;
                    }

                    return Ok(Arc::new(tri_mesh));
                }
            }
            _ => continue, // 加载失败，尝试下一个 LOD 级别
        }
    }

    anyhow::bail!("无法加载几何文件: {}", geo_hash)
}

/// 从 GLB/GLTF 文件加载 TriMesh
fn load_tri_mesh_from_glb(path: &PathBuf) -> anyhow::Result<TriMesh> {
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    let glb = gltf::Gltf::from_reader(reader)?;
    
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    // 遍历所有 mesh 和 primitive
    for mesh in glb.meshes() {
        for primitive in mesh.primitives() {
            let reader = primitive.reader(|buffer| Some(glb.blob.as_ref()?.as_slice()));

            if let Some(iter) = reader.read_positions() {
                let base_index = vertices.len() as u32;
                for vertex in iter {
                    vertices.push(Point::new(vertex[0], vertex[1], vertex[2]));
                }

                if let Some(iter) = reader.read_indices() {
                    let iter = iter.into_u32();
                    let chunked_indices: Vec<u32> = iter.collect();
                     // 处理三角形索引
                    for chunk in chunked_indices.chunks(3) {
                        if chunk.len() == 3 {
                            indices.push([
                                base_index + chunk[0],
                                base_index + chunk[1],
                                base_index + chunk[2],
                            ]);
                        }
                    }
                }
            }
        }
    }

    if vertices.is_empty() {
        anyhow::bail!("GLB 文件不包含顶点数据");
    }

    // 创建 TriMesh (使用 ORIENTED 和 MERGE_DUPLICATE_VERTICES flag)
    // TriMesh::new 返回 Result，需要处理错误
    TriMesh::new(vertices, indices).map_err(|e| anyhow::anyhow!("构建 TriMesh 失败: {}", e))
}

/// 辅助函数：对 TriMesh 应用变换
fn transform_tri_mesh(mesh: &TriMesh, transform: Mat4) -> TriMesh {
    let vertices: Vec<Point<Real>> = mesh
        .vertices()
        .iter()
        .map(|v| {
            let p = transform.transform_point3(Vec3::new(v.x, v.y, v.z));
            Point::new(p.x, p.y, p.z)
        })
        .collect();
    
    // 索引不变
    let indices = mesh.indices().to_vec();
    
    // 这里我们假设变换后的几何体仍然是有效的，如果构建失败则 panic (或者应该返回 Result)
    TriMesh::new(vertices, indices).expect("变换后的几何体构建失败")
}

/// 清理几何缓存
async fn cleanup_geometry_cache(cache: &DashMap<String, Arc<PlantMesh>>) {
    // 简单的清理策略：移除一半的条目
    let keys_to_remove: Vec<String> = cache
        .iter()
        .take(cache.len() / 2)
        .map(|entry| entry.key().clone())
        .collect();

    for key in keys_to_remove {
        cache.remove(&key);
    }

    info!("几何缓存清理完成，当前大小: {}", cache.len());
}

fn merge_aabb(a: &Aabb, b: &Aabb) -> Aabb {
    let mins = Point::new(
        a.mins.x.min(b.mins.x),
        a.mins.y.min(b.mins.y),
        a.mins.z.min(b.mins.z),
    );
    let maxs = Point::new(
        a.maxs.x.max(b.maxs.x),
        a.maxs.y.max(b.maxs.y),
        a.maxs.z.max(b.maxs.z),
    );
    Aabb::new(mins, maxs)
}

/// 从 AABB 提取增强关键点
/// 包括：8个顶点 + 中心点 + 6个面中心 + 12条边中点
fn extract_aabb_key_points(aabb: &Aabb) -> Vec<Point<Real>> {
    let mut points = Vec::with_capacity(27);

    // 1. AABB 8个顶点
    let vertices = aabb.vertices();
    points.extend_from_slice(&vertices);

    // 2. 中心点
    points.push(aabb.center());

    // 3. 6个面的中心点
    let mins = &aabb.mins;
    let maxs = &aabb.maxs;
    let cx = (mins.x + maxs.x) / 2.0;
    let cy = (mins.y + maxs.y) / 2.0;
    let cz = (mins.z + maxs.z) / 2.0;

    points.push(Point::new(mins.x, cy, cz)); // 左面中心
    points.push(Point::new(maxs.x, cy, cz)); // 右面中心
    points.push(Point::new(cx, mins.y, cz)); // 前面中心
    points.push(Point::new(cx, maxs.y, cz)); // 后面中心
    points.push(Point::new(cx, cy, mins.z)); // 下面中心
    points.push(Point::new(cx, cy, maxs.z)); // 上面中心

    // 4. 12条边的中点
    // 底面4条边
    points.push(Point::from((vertices[0].coords + vertices[1].coords) / 2.0));
    points.push(Point::from((vertices[1].coords + vertices[3].coords) / 2.0));
    points.push(Point::from((vertices[3].coords + vertices[2].coords) / 2.0));
    points.push(Point::from((vertices[2].coords + vertices[0].coords) / 2.0));
    // 顶面4条边
    points.push(Point::from((vertices[4].coords + vertices[5].coords) / 2.0));
    points.push(Point::from((vertices[5].coords + vertices[7].coords) / 2.0));
    points.push(Point::from((vertices[7].coords + vertices[6].coords) / 2.0));
    points.push(Point::from((vertices[6].coords + vertices[4].coords) / 2.0));
    // 竖直4条边
    points.push(Point::from((vertices[0].coords + vertices[4].coords) / 2.0));
    points.push(Point::from((vertices[1].coords + vertices[5].coords) / 2.0));
    points.push(Point::from((vertices[2].coords + vertices[6].coords) / 2.0));
    points.push(Point::from((vertices[3].coords + vertices[7].coords) / 2.0));

    points
}

/// 从几何体实例提取增强关键点
/// 
/// 优先使用 GeomInstQuery 的 pts 字段（实际几何关键点），
/// 如果 pts 为空则使用 AABB 增强关键点作为回退
fn extract_geom_key_points(geom_insts: &[GeomInstQuery]) -> Vec<Point<Real>> {
    let mut all_points = Vec::with_capacity(geom_insts.len() * 30);

    for geom_inst in geom_insts {
        // 跳过没有 world_aabb 的实例
        let Some(ref world_aabb) = geom_inst.world_aabb else { continue };

        // 优先使用 pts 字段（来自几何体的实际关键点）
        if let Some(ref pts) = geom_inst.pts {
            if !pts.is_empty() {
                // 使用实际几何关键点
                for pt in pts {
                    all_points.push(Point::new(pt.0.x, pt.0.y, pt.0.z));
                }
                // 同时添加 AABB 中心点以提高鲁棒性
                let aabb: Aabb = world_aabb.clone().into();
                all_points.push(aabb.center());
                continue;
            }
        }

        // 回退：使用 AABB 增强关键点
        let aabb: Aabb = world_aabb.clone().into();
        let points = extract_aabb_key_points(&aabb);
        all_points.extend(points);
    }

    all_points
}

/// 从 TriMesh 顶点采样关键点
/// 
/// 策略：
/// 1. 如果顶点数 <= max_samples，使用所有顶点
/// 2. 否则均匀采样 max_samples 个顶点
/// 3. 始终包含 mesh 的质心
fn extract_key_points_from_mesh(mesh: &TriMesh, max_samples: usize) -> Vec<Point<Real>> {
    let vertices = mesh.vertices();
    let vertex_count = vertices.len();
    
    if vertex_count == 0 {
        return vec![];
    }
    
    let mut key_points = Vec::with_capacity(max_samples + 1);
    
    // 计算质心
    let mut centroid = Point::new(0.0, 0.0, 0.0);
    for v in vertices.iter() {
        centroid.x += v.x;
        centroid.y += v.y;
        centroid.z += v.z;
    }
    let n = vertex_count as Real;
    centroid = Point::new(centroid.x / n, centroid.y / n, centroid.z / n);
    key_points.push(centroid);
    
    if vertex_count <= max_samples {
        // 顶点数少，使用所有顶点
        for v in vertices.iter() {
            key_points.push(*v);
        }
    } else {
        // 均匀采样
        let step = vertex_count / max_samples;
        for i in 0..max_samples {
            key_points.push(vertices[i * step]);
        }
    }
    
    key_points
}

/// 判断关键点是否在面板 TriMesh 内
/// 使用投票策略：超过 50% 的关键点在面板内即判定为属于该房间
fn is_geom_in_panel(
    key_points: &[Point<Real>],
    panel_meshes: &[Arc<TriMesh>],
    tolerance: f32,
) -> bool {
    if key_points.is_empty() || panel_meshes.is_empty() {
        return false;
    }

    let mut points_inside = 0;
    let total_points = key_points.len();
    let tolerance_sq = (tolerance as Real).powi(2);
    let threshold = total_points / 2 + 1;

    for (idx, point) in key_points.iter().enumerate() {
        if is_point_inside_any_mesh(point, panel_meshes, tolerance_sq) {
            points_inside += 1;
        }

        let remaining = total_points - idx - 1;
        if points_inside >= threshold {
            return true;
        }
        if points_inside + remaining < threshold {
            return false;
        }
    }

    false
}

fn is_point_inside_any_mesh(
    point: &Point<Real>,
    panel_meshes: &[Arc<TriMesh>],
    tolerance_sq: Real,
) -> bool {
    for mesh in panel_meshes {
        // 使用射线投射法判断点是否在网格内部
        // parry3d 的 is_inside 对于某些封闭网格不可靠，射线投射法更准确
        if is_point_inside_mesh_raycast(point, mesh) {
            return true;
        }

        // 回退到距离检测：如果点非常接近表面，也认为在内部
        let projection = mesh.project_point(&Isometry::identity(), point, true);
        let distance_sq = (projection.point - point).norm_squared();
        if distance_sq <= tolerance_sq {
            return true;
        }
    }

    false
}

/// 使用射线投射法判断点是否在封闭网格内部
/// 向多个方向发射射线，如果在相对的两个方向上都有交点，则认为点在内部
fn is_point_inside_mesh_raycast(point: &Point<Real>, tri_mesh: &TriMesh) -> bool {
    let identity = Isometry::identity();

    // 向 +Z 和 -Z 方向发射射线
    let ray_pos_z = Ray::new(*point, Vector::new(0.0, 0.0, 1.0));
    let ray_neg_z = Ray::new(*point, Vector::new(0.0, 0.0, -1.0));

    let hit_pos_z = tri_mesh.cast_ray(&identity, &ray_pos_z, Real::MAX, true);
    let hit_neg_z = tri_mesh.cast_ray(&identity, &ray_neg_z, Real::MAX, true);

    // 如果 Z 方向两边都有交点，点在网格内部
    if hit_pos_z.is_some() && hit_neg_z.is_some() {
        return true;
    }

    // 备用检测：向 +X/-X 或 +Y/-Y 方向检测
    let ray_pos_x = Ray::new(*point, Vector::new(1.0, 0.0, 0.0));
    let ray_neg_x = Ray::new(*point, Vector::new(-1.0, 0.0, 0.0));

    let hit_pos_x = tri_mesh.cast_ray(&identity, &ray_pos_x, Real::MAX, true);
    let hit_neg_x = tri_mesh.cast_ray(&identity, &ray_neg_x, Real::MAX, true);

    if hit_pos_x.is_some() && hit_neg_x.is_some() {
        return true;
    }

    let ray_pos_y = Ray::new(*point, Vector::new(0.0, 1.0, 0.0));
    let ray_neg_y = Ray::new(*point, Vector::new(0.0, -1.0, 0.0));

    let hit_pos_y = tri_mesh.cast_ray(&identity, &ray_pos_y, Real::MAX, true);
    let hit_neg_y = tri_mesh.cast_ray(&identity, &ray_neg_y, Real::MAX, true);

    hit_pos_y.is_some() && hit_neg_y.is_some()
}

/// 改进版本的房间关系保存
async fn save_room_relate(
    panel_refno: RefnoEnum,
    within_refnos: &HashSet<RefnoEnum>,
    room_num: &str,
) -> anyhow::Result<()> {
    if within_refnos.is_empty() {
        return Ok(());
    }

    let mut sql_statements = Vec::new();

    for refno in within_refnos {
        let relation_id = format!("{}_{}", panel_refno, refno);
        let sql = format!(
            "relate {}->room_relate:{}->{}  set room_num='{}', confidence=0.9, created_at=time::now();",
            panel_refno.to_pe_key(),
            relation_id,
            refno.to_pe_key(),
            room_num
        );
        sql_statements.push(sql);
    }

    // 批量执行
    let batch_sql = sql_statements.join("\n");
    SUL_DB.query(&batch_sql).await?;

    debug!(
        "保存房间关系: panel={}, components={}",
        panel_refno,
        within_refnos.len()
    );
    Ok(())
}

/// 收集所有需要的几何哈希
async fn collect_geometry_hashes(
    room_panel_map: &[(RefnoEnum, String, Vec<RefnoEnum>)],
) -> anyhow::Result<Vec<String>> {
    let mut geo_hashes = HashSet::new();

    for (_, _, panel_refnos) in room_panel_map {
        for panel_refno in panel_refnos {
            let geom_insts: Vec<GeomInstQuery> = aios_core::query_insts(&[*panel_refno], true)
                .await
                .unwrap_or_default();

            for geom_inst in geom_insts {
                for inst in geom_inst.insts {
                    geo_hashes.insert(inst.geo_hash);
                }
            }
        }
    }

    Ok(geo_hashes.into_iter().collect())
}

/// 估算内存使用量
#[cfg(all(not(target_arch = "wasm32"), any(feature = "sqlite-index", feature = "duckdb-feature")))]
async fn estimate_memory_usage() -> f32 {
    let cache = get_enhanced_geometry_cache().await;
    let cache_size = cache.len() as f32 * 0.5; // 假设每个缓存项平均 0.5MB
    cache_size
}

#[cfg(not(all(not(target_arch = "wasm32"), any(feature = "sqlite-index", feature = "duckdb-feature"))))]
async fn estimate_memory_usage() -> f32 {
    // 在不启用 sqlite-index 特性时，返回一个保守估计
    let cache = get_enhanced_geometry_cache().await;
    let cache_size = cache.len() as f32 * 0.5;
    cache_size
}

/// 房间名称匹配函数 (HD项目)
pub fn match_room_name_hd(room_name: &str) -> bool {
    let regex = Regex::new(r"^[A-Z]\d{3}$").unwrap();
    regex.is_match(room_name)
}

/// 房间名称匹配函数 (HH项目)
pub fn match_room_name_hh(room_name: &str) -> bool {
    true // HH项目接受所有房间名称
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================================================
    // 测试套件 1: 房间面板映射构建测试
    // ============================================================================

    #[tokio::test]
    async fn test_enhanced_geometry_cache() {
        let cache = get_enhanced_geometry_cache().await;
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn test_room_name_matching() {
        assert!(match_room_name_hd("A123"));
        assert!(!match_room_name_hd("AB123"));
        assert!(match_room_name_hh("任何名称"));
    }

    #[tokio::test]
    async fn test_memory_estimation() {
        let memory_mb = estimate_memory_usage().await;
        assert!(memory_mb >= 0.0);
    }

    #[test]
    fn test_build_room_panel_query_sql_contains_range_and_filter() {
        let sql = build_room_panel_query_sql(&vec!["AA".to_string(), "BB".to_string()]);
        assert!(sql.contains("select value ["));
        assert!(sql.contains("NAME IS NOT NONE"));
        assert!(sql.contains("'AA' in NAME") && sql.contains("'BB' in NAME"));
        #[cfg(feature = "project_hh")]
        assert!(sql.contains("from SBFR"));
        #[cfg(not(feature = "project_hh"))]
        assert!(sql.contains("from FRMW"));
    }

    /// 测试 SQL 生成 - 空关键词列表
    #[test]
    fn test_build_room_panel_query_sql_empty_keywords() {
        let sql = build_room_panel_query_sql(&vec![]);
        // 空关键词时 filter 固定为 true
        assert!(sql.contains("select value"));
        assert!(sql.contains("(true)"));
    }

    /// 测试 SQL 生成 - 单个关键词
    #[test]
    fn test_build_room_panel_query_sql_single_keyword() {
        let sql = build_room_panel_query_sql(&vec!["ROOM".to_string()]);
        assert!(sql.contains("'ROOM' in NAME"));
        assert!(!sql.contains(" or ")); // 单个关键词不应有 or
    }

    /// 测试 SQL 生成 - 多个关键词
    #[test]
    fn test_build_room_panel_query_sql_multiple_keywords() {
        let sql = build_room_panel_query_sql(&vec![
            "AA".to_string(),
            "BB".to_string(),
            "CC".to_string(),
        ]);
        assert!(sql.contains("'AA' in NAME"));
        assert!(sql.contains("'BB' in NAME"));
        assert!(sql.contains("'CC' in NAME"));
        assert!(sql.contains(" or ")); // 多个关键词应有 or 连接
    }

    // ============================================================================
    // 测试套件 2: 房间名格式验证测试
    // ============================================================================

    /// HD 项目房间名格式 - 有效格式测试
    #[test]
    fn test_match_room_name_hd_valid_formats() {
        // 标准格式: 一个大写字母 + 三个数字
        assert!(match_room_name_hd("A123"));
        assert!(match_room_name_hd("B456"));
        assert!(match_room_name_hd("Z999"));
        assert!(match_room_name_hd("A000"));
        assert!(match_room_name_hd("M500"));
    }

    /// HD 项目房间名格式 - 无效格式测试
    #[test]
    fn test_match_room_name_hd_invalid_formats() {
        // 小写字母开头
        assert!(!match_room_name_hd("a123"));
        // 两个字母开头
        assert!(!match_room_name_hd("AB123"));
        // 数字不足
        assert!(!match_room_name_hd("A12"));
        // 数字过多
        assert!(!match_room_name_hd("A1234"));
        // 空字符串
        assert!(!match_room_name_hd(""));
        // 纯数字
        assert!(!match_room_name_hd("1234"));
        // 带空格
        assert!(!match_room_name_hd("A 123"));
        // 带特殊字符
        assert!(!match_room_name_hd("A-123"));
        // 数字开头
        assert!(!match_room_name_hd("1A23"));
    }

    /// HH 项目房间名格式 - 所有格式都接受
    #[test]
    fn test_match_room_name_hh_accepts_all() {
        assert!(match_room_name_hh("任何格式"));
        assert!(match_room_name_hh("A123"));
        assert!(match_room_name_hh("房间-001"));
        assert!(match_room_name_hh(""));
        assert!(match_room_name_hh("特殊字符!@#$%"));
    }

    // ============================================================================
    // 测试套件 3: 关键点提取测试
    // ============================================================================

    /// 验证 AABB 关键点数量为 27
    #[test]
    fn test_extract_aabb_key_points_count() {
        let aabb = Aabb::new(
            Point::new(0.0, 0.0, 0.0),
            Point::new(10.0, 10.0, 10.0),
        );
        let points = extract_aabb_key_points(&aabb);
        
        // 27 = 8顶点 + 1中心 + 6面中心 + 12边中点
        assert_eq!(points.len(), 27, "应该生成 27 个关键点");
    }

    /// 验证 8 个顶点坐标正确
    #[test]
    fn test_extract_aabb_key_points_vertices() {
        let aabb = Aabb::new(
            Point::new(0.0, 0.0, 0.0),
            Point::new(10.0, 20.0, 30.0),
        );
        let points = extract_aabb_key_points(&aabb);
        
        // 前 8 个是顶点
        let vertices: Vec<_> = points.iter().take(8).collect();
        
        // 验证所有顶点坐标在边界上
        for v in &vertices {
            assert!(
                (v.x == 0.0 || v.x == 10.0) &&
                (v.y == 0.0 || v.y == 20.0) &&
                (v.z == 0.0 || v.z == 30.0),
                "顶点 {:?} 应在 AABB 边界上", v
            );
        }
    }

    /// 验证中心点坐标正确
    #[test]
    fn test_extract_aabb_key_points_center() {
        let aabb = Aabb::new(
            Point::new(0.0, 0.0, 0.0),
            Point::new(10.0, 20.0, 30.0),
        );
        let points = extract_aabb_key_points(&aabb);
        
        // 第 9 个点是中心点 (索引 8)
        let center = &points[8];
        assert_eq!(center.x, 5.0, "中心点 X 坐标应为 5.0");
        assert_eq!(center.y, 10.0, "中心点 Y 坐标应为 10.0");
        assert_eq!(center.z, 15.0, "中心点 Z 坐标应为 15.0");
    }

    /// 验证 6 个面中心坐标正确
    #[test]
    fn test_extract_aabb_key_points_face_centers() {
        let aabb = Aabb::new(
            Point::new(0.0, 0.0, 0.0),
            Point::new(10.0, 20.0, 30.0),
        );
        let points = extract_aabb_key_points(&aabb);
        
        // 面中心点从索引 9 开始，共 6 个
        let face_centers: Vec<_> = points.iter().skip(9).take(6).collect();
        
        // 左面中心 (x=0)
        assert_eq!(face_centers[0].x, 0.0);
        assert_eq!(face_centers[0].y, 10.0);
        assert_eq!(face_centers[0].z, 15.0);
        
        // 右面中心 (x=10)
        assert_eq!(face_centers[1].x, 10.0);
        assert_eq!(face_centers[1].y, 10.0);
        assert_eq!(face_centers[1].z, 15.0);
        
        // 前面中心 (y=0)
        assert_eq!(face_centers[2].x, 5.0);
        assert_eq!(face_centers[2].y, 0.0);
        assert_eq!(face_centers[2].z, 15.0);
        
        // 后面中心 (y=20)
        assert_eq!(face_centers[3].x, 5.0);
        assert_eq!(face_centers[3].y, 20.0);
        assert_eq!(face_centers[3].z, 15.0);
        
        // 下面中心 (z=0)
        assert_eq!(face_centers[4].x, 5.0);
        assert_eq!(face_centers[4].y, 10.0);
        assert_eq!(face_centers[4].z, 0.0);
        
        // 上面中心 (z=30)
        assert_eq!(face_centers[5].x, 5.0);
        assert_eq!(face_centers[5].y, 10.0);
        assert_eq!(face_centers[5].z, 30.0);
    }

    /// 验证 12 条边中点坐标正确
    #[test]
    fn test_extract_aabb_key_points_edge_midpoints() {
        let aabb = Aabb::new(
            Point::new(0.0, 0.0, 0.0),
            Point::new(10.0, 10.0, 10.0),
        );
        let points = extract_aabb_key_points(&aabb);
        
        // 边中点从索引 15 开始，共 12 个
        let edge_midpoints: Vec<_> = points.iter().skip(15).take(12).collect();
        
        assert_eq!(edge_midpoints.len(), 12, "应该有 12 个边中点");
        
        // 验证所有边中点都是有效坐标
        for (i, mp) in edge_midpoints.iter().enumerate() {
            assert!(
                mp.x >= 0.0 && mp.x <= 10.0 &&
                mp.y >= 0.0 && mp.y <= 10.0 &&
                mp.z >= 0.0 && mp.z <= 10.0,
                "边中点 {} {:?} 应在 AABB 范围内", i, mp
            );
        }
    }

    /// 测试零尺寸 AABB 的关键点提取
    #[test]
    fn test_extract_aabb_key_points_zero_size() {
        let aabb = Aabb::new(
            Point::new(5.0, 5.0, 5.0),
            Point::new(5.0, 5.0, 5.0),
        );
        let points = extract_aabb_key_points(&aabb);
        
        // 所有点都应该在同一位置
        for point in &points {
            assert_eq!(point.x, 5.0);
            assert_eq!(point.y, 5.0);
            assert_eq!(point.z, 5.0);
        }
    }

    /// 测试负坐标 AABB 的关键点提取
    #[test]
    fn test_extract_aabb_key_points_negative_coords() {
        let aabb = Aabb::new(
            Point::new(-10.0, -20.0, -30.0),
            Point::new(10.0, 20.0, 30.0),
        );
        let points = extract_aabb_key_points(&aabb);
        
        assert_eq!(points.len(), 27);
        
        // 中心点应在原点
        let center = &points[8];
        assert_eq!(center.x, 0.0);
        assert_eq!(center.y, 0.0);
        assert_eq!(center.z, 0.0);
    }

    // ============================================================================
    // 测试套件 4: 包含判断测试 (is_geom_in_panel)
    // ============================================================================

    /// 创建测试用的简单立方体 TriMesh（带 ORIENTED 标志）
    /// 注意：parry3d 的 TriMesh.project_point().is_inside 对于简单测试网格
    /// 可能无法正确判断内外部，因此这些测试主要验证函数的逻辑正确性
    fn create_test_cube_trimesh(min: Point<Real>, max: Point<Real>) -> TriMesh {
        let vertices = vec![
            Point::new(min.x, min.y, min.z),
            Point::new(max.x, min.y, min.z),
            Point::new(max.x, max.y, min.z),
            Point::new(min.x, max.y, min.z),
            Point::new(min.x, min.y, max.z),
            Point::new(max.x, min.y, max.z),
            Point::new(max.x, max.y, max.z),
            Point::new(min.x, max.y, max.z),
        ];

        let indices = vec![
            [0, 1, 2], [0, 2, 3],
            [4, 6, 5], [4, 7, 6],
            [0, 5, 1], [0, 4, 5],
            [2, 7, 3], [2, 6, 7],
            [0, 3, 7], [0, 7, 4],
            [1, 5, 6], [1, 6, 2],
        ];

        TriMesh::with_flags(
            vertices,
            indices,
            TriMeshFlags::ORIENTED | TriMeshFlags::MERGE_DUPLICATE_VERTICES,
        )
        .unwrap()
    }

    /// 测试空点列表 → 不应该通过（这是函数逻辑的核心边界条件）
    #[test]
    fn test_is_geom_in_panel_empty_points() {
        let panel_meshes = vec![Arc::new(create_test_cube_trimesh(
            Point::new(0.0, 0.0, 0.0),
            Point::new(100.0, 100.0, 100.0),
        ))];

        let key_points: Vec<Point<Real>> = vec![];

        let result = is_geom_in_panel(&key_points, &panel_meshes, 0.1);
        assert!(!result, "空点列表不应该通过");
    }

    /// 测试边界上的点 - 距离为0，应该通过容差检测
    #[test]
    fn test_is_geom_in_panel_on_boundary() {
        let panel_meshes = vec![Arc::new(create_test_cube_trimesh(
            Point::new(0.0, 0.0, 0.0),
            Point::new(100.0, 100.0, 100.0),
        ))];

        // 点正好在表面上（投影距离为0）
        let key_points = vec![
            Point::new(0.0, 50.0, 50.0),   // 左面上
            Point::new(100.0, 50.0, 50.0), // 右面上
            Point::new(50.0, 0.0, 50.0),   // 前面上
            Point::new(50.0, 100.0, 50.0), // 后面上
        ];

        // 表面上的点距离为0，应该被接受
        let result = is_geom_in_panel(&key_points, &panel_meshes, 0.1);
        assert!(result, "表面上的点应该通过（距离为0，在容差内）");
    }

    /// 测试阈值逻辑 - 使用大容差确保表面上的点被计入
    #[test]
    fn test_is_geom_in_panel_threshold_logic() {
        let panel_meshes = vec![Arc::new(create_test_cube_trimesh(
            Point::new(0.0, 0.0, 0.0),
            Point::new(100.0, 100.0, 100.0),
        ))];

        // 使用4个表面上的点（100%应该通过）
        let surface_points = vec![
            Point::new(0.0, 50.0, 50.0),
            Point::new(100.0, 50.0, 50.0),
            Point::new(50.0, 0.0, 50.0),
            Point::new(50.0, 100.0, 50.0),
        ];

        let result = is_geom_in_panel(&surface_points, &panel_meshes, 0.1);
        assert!(result, "100% 表面点应该通过");
    }

    /// 测试容差对表面附近点的影响
    #[test]
    fn test_is_geom_in_panel_tolerance_effect() {
        let panel_meshes = vec![Arc::new(create_test_cube_trimesh(
            Point::new(0.0, 0.0, 0.0),
            Point::new(100.0, 100.0, 100.0),
        ))];

        // 点略微在表面外
        let near_surface_points = vec![
            Point::new(50.0, 50.0, 100.05), // 距离顶面 0.05
            Point::new(50.0, 50.0, -0.05),  // 距离底面 0.05
        ];

        // 容差 0.1 的平方是 0.01，距离 0.05 的平方是 0.0025
        // 0.0025 < 0.01，所以这些点应该被接受
        let result_large_tolerance = is_geom_in_panel(&near_surface_points, &panel_meshes, 0.1);
        assert!(
            result_large_tolerance,
            "容差 0.1 应该接受距离 0.05 的点"
        );
    }

    /// 测试非常远的点不应该被计入（即使容差很大）
    #[test]
    fn test_is_geom_in_panel_far_points_excluded() {
        let panel_meshes = vec![Arc::new(create_test_cube_trimesh(
            Point::new(0.0, 0.0, 0.0),
            Point::new(100.0, 100.0, 100.0),
        ))];

        // 全部都是非常远的点
        let far_points = vec![
            Point::new(10000.0, 10000.0, 10000.0),
            Point::new(-10000.0, -10000.0, -10000.0),
            Point::new(20000.0, 0.0, 0.0),
        ];

        // 即使容差是 1.0，这些点也太远了
        let result = is_geom_in_panel(&far_points, &panel_meshes, 1.0);
        assert!(!result, "非常远的点不应该通过");
    }

    /// 测试混合点场景 - 部分在表面，部分很远
    #[test]
    fn test_is_geom_in_panel_mixed_points() {
        let panel_meshes = vec![Arc::new(create_test_cube_trimesh(
            Point::new(0.0, 0.0, 0.0),
            Point::new(100.0, 100.0, 100.0),
        ))];

        // 3个表面点 + 1个远点 = 75% 在容差内
        let mixed_points = vec![
            Point::new(0.0, 50.0, 50.0),      // 表面上
            Point::new(100.0, 50.0, 50.0),    // 表面上
            Point::new(50.0, 0.0, 50.0),      // 表面上
            Point::new(10000.0, 10000.0, 10000.0), // 很远
        ];

        let result = is_geom_in_panel(&mixed_points, &panel_meshes, 0.1);
        assert!(result, "超过 50% 点在容差内应该通过");
    }

    /// 测试低于阈值的场景
    #[test]
    fn test_is_geom_in_panel_below_threshold() {
        let panel_meshes = vec![Arc::new(create_test_cube_trimesh(
            Point::new(0.0, 0.0, 0.0),
            Point::new(100.0, 100.0, 100.0),
        ))];

        // 1个表面点 + 4个远点 = 20% 在容差内
        let mostly_far_points = vec![
            Point::new(0.0, 50.0, 50.0),      // 表面上 (1)
            Point::new(10000.0, 0.0, 0.0),    // 很远 (1)
            Point::new(-10000.0, 0.0, 0.0),   // 很远 (2)
            Point::new(0.0, 10000.0, 0.0),    // 很远 (3)
            Point::new(0.0, -10000.0, 0.0),   // 很远 (4)
        ];

        // 1/5 = 20% < 50%
        let result = is_geom_in_panel(&mostly_far_points, &panel_meshes, 0.1);
        assert!(!result, "20% 点在容差内不应该通过");
    }

    // ============================================================================
    // 测试套件 5: 缓存指标测试
    // ============================================================================

    #[test]
    fn test_cache_metrics_new() {
        let metrics = CacheMetrics::new();
        assert_eq!(metrics.plant_hits.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.plant_misses.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.trimesh_hits.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.trimesh_misses.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_cache_metrics_hit_rate() {
        let metrics = CacheMetrics::new();
        
        // 初始命中率为 0
        assert_eq!(metrics.hit_rate(), 0.0);
        
        // 记录一些命中和未命中
        metrics.record_plant_hit();
        metrics.record_plant_hit();
        metrics.record_plant_miss();
        
        // 2 命中 / 3 总计 = 0.666...
        let hit_rate = metrics.hit_rate();
        assert!((hit_rate - 0.6666666).abs() < 0.001, "命中率应约为 66.67%");
    }

    #[test]
    fn test_cache_metrics_reset() {
        let metrics = CacheMetrics::new();
        
        metrics.record_plant_hit();
        metrics.record_plant_miss();
        metrics.record_trimesh_hit();
        metrics.record_trimesh_miss();
        
        metrics.reset();
        
        assert_eq!(metrics.plant_hits.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.plant_misses.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.trimesh_hits.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.trimesh_misses.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.hit_rate(), 0.0);
    }

    // ============================================================================
    // 测试套件 6: RoomComputeOptions 测试
    // ============================================================================

    #[test]
    fn test_room_compute_options_default() {
        let options = RoomComputeOptions::default();
        assert_eq!(options.inside_tol, 0.1);
        // 并发度取决于环境变量或默认值 4
        assert!(options.concurrency > 0);
        assert!(options.candidate_concurrency > 0);
    }

    #[test]
    fn test_default_room_concurrency() {
        let concurrency = default_room_concurrency();
        // 默认值应该是 4（如果没有设置环境变量）
        assert!(concurrency > 0 && concurrency <= 64, "并发度应该在合理范围内");
    }

    // ============================================================================
    // 测试套件 7: 房间关系统计测试
    // ============================================================================

    #[test]
    fn test_room_build_stats_serialization() {
        let stats = RoomBuildStats {
            total_rooms: 10,
            total_panels: 50,
            total_components: 200,
            build_time_ms: 5000,
            cache_hit_rate: 0.85,
            memory_usage_mb: 128.5,
        };
        
        // 测试序列化
        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("\"total_rooms\":10"));
        assert!(json.contains("\"total_panels\":50"));
        
        // 测试反序列化
        let deserialized: RoomBuildStats = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.total_rooms, 10);
        assert_eq!(deserialized.total_panels, 50);
        assert_eq!(deserialized.total_components, 200);
    }

    // ============================================================================
    // 测试套件 8: IncrementalUpdateResult 测试
    // ============================================================================

    #[test]
    fn test_incremental_update_result_serialization() {
        let result = IncrementalUpdateResult {
            affected_rooms: 5,
            updated_elements: 25,
            duration_ms: 1500,
        };
        
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"affected_rooms\":5"));
        
        let deserialized: IncrementalUpdateResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.affected_rooms, 5);
        assert_eq!(deserialized.updated_elements, 25);
        assert_eq!(deserialized.duration_ms, 1500);
    }

    // ============================================================================
    // 测试套件 9: 几何实例关键点提取测试
    // ============================================================================

    /// 测试多个几何实例的关键点合并
    #[test]
    fn test_extract_geom_key_points_multiple_instances() {
        // 创建模拟的 GeomInstQuery 数据比较复杂，这里用 AABB 测试逻辑
        let aabb1 = Aabb::new(
            Point::new(0.0, 0.0, 0.0),
            Point::new(10.0, 10.0, 10.0),
        );
        let aabb2 = Aabb::new(
            Point::new(20.0, 20.0, 20.0),
            Point::new(30.0, 30.0, 30.0),
        );
        
        let points1 = extract_aabb_key_points(&aabb1);
        let points2 = extract_aabb_key_points(&aabb2);
        
        // 两个 AABB 应该各有 27 个点
        assert_eq!(points1.len(), 27);
        assert_eq!(points2.len(), 27);
        
        // 合并后应该有 54 个点
        let mut all_points = Vec::new();
        all_points.extend(points1);
        all_points.extend(points2);
        assert_eq!(all_points.len(), 54);
    }

    // ============================================================================
    // 测试套件 10: 边界条件和异常情况测试
    // ============================================================================

    /// 测试非常大的 AABB
    #[test]
    fn test_extract_aabb_key_points_large_aabb() {
        let aabb = Aabb::new(
            Point::new(-1e6, -1e6, -1e6),
            Point::new(1e6, 1e6, 1e6),
        );
        let points = extract_aabb_key_points(&aabb);
        
        assert_eq!(points.len(), 27);
        
        // 中心应该在原点
        let center = &points[8];
        assert!((center.x - 0.0).abs() < 1e-6);
        assert!((center.y - 0.0).abs() < 1e-6);
        assert!((center.z - 0.0).abs() < 1e-6);
    }

    /// 测试非常小的 AABB
    #[test]
    fn test_extract_aabb_key_points_tiny_aabb() {
        let aabb = Aabb::new(
            Point::new(0.0, 0.0, 0.0),
            Point::new(1e-6, 1e-6, 1e-6),
        );
        let points = extract_aabb_key_points(&aabb);
        
        assert_eq!(points.len(), 27);
        
        // 所有点应该非常接近
        for point in &points {
            assert!(point.x >= 0.0 && point.x <= 1e-6);
            assert!(point.y >= 0.0 && point.y <= 1e-6);
            assert!(point.z >= 0.0 && point.z <= 1e-6);
        }
    }

    /// 测试单个表面点应该通过
    #[test]
    fn test_is_geom_in_panel_single_surface_point() {
        let panel_meshes = vec![Arc::new(create_test_cube_trimesh(
            Point::new(0.0, 0.0, 0.0),
            Point::new(100.0, 100.0, 100.0),
        ))];

        // 单个表面上的点（距离为0，在容差内）
        let key_points = vec![Point::new(0.0, 50.0, 50.0)];

        let result = is_geom_in_panel(&key_points, &panel_meshes, 0.1);
        assert!(result, "单个表面点应该通过");
    }

    /// 测试单点边界条件：阈值为 1，需要单点通过判定
    #[test]
    fn test_is_geom_in_panel_single_point_threshold_edge_case() {
        let panel_meshes = vec![Arc::new(create_test_cube_trimesh(
            Point::new(0.0, 0.0, 0.0),
            Point::new(100.0, 100.0, 100.0),
        ))];

        // 单个远点 - 阈值为 1，应不通过
        let key_points = vec![Point::new(10000.0, 10000.0, 10000.0)];

        let result = is_geom_in_panel(&key_points, &panel_meshes, 0.1);
        assert!(!result, "单点远场应不通过");
    }

    /// 测试两个远点应该不通过（这是最小有效过滤场景）
    #[test]
    fn test_is_geom_in_panel_two_far_points() {
        let panel_meshes = vec![Arc::new(create_test_cube_trimesh(
            Point::new(0.0, 0.0, 0.0),
            Point::new(100.0, 100.0, 100.0),
        ))];

        // 两个远点 - 阈值为 2
        // 0 个点在内部，0 >= 2 是 false
        let key_points = vec![
            Point::new(10000.0, 10000.0, 10000.0),
            Point::new(-10000.0, -10000.0, -10000.0),
        ];

        let result = is_geom_in_panel(&key_points, &panel_meshes, 0.1);
        assert!(!result, "两个远点不应该通过（0 >= 2 是 false）");
    }
}

/// 增量更新结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncrementalUpdateResult {
    /// 影响的房间数量
    pub affected_rooms: usize,
    /// 更新的元素数量
    pub updated_elements: usize,
    /// 耗时（毫秒）
    pub duration_ms: u64,
}

/// 增量更新房间关系
///
/// 只更新指定 refnos 相关的房间关系，而不是全量重建
///
/// # 参数
/// * `refnos` - 需要更新关系的构件参考号列表
///
/// # 返回值
/// * `IncrementalUpdateResult` - 更新结果统计
#[cfg(all(not(target_arch = "wasm32"), any(feature = "sqlite-index", feature = "duckdb-feature")))]
pub async fn update_room_relations_incremental(
    refnos: &[RefnoEnum],
) -> anyhow::Result<IncrementalUpdateResult> {
    let start_time = Instant::now();
    info!("开始增量更新房间关系，涉及 {} 个构件", refnos.len());

    if refnos.is_empty() {
        return Ok(IncrementalUpdateResult {
            affected_rooms: 0,
            updated_elements: 0,
            duration_ms: 0,
        });
    }

    // 1. 查询这些 refnos 相关的房间面板
    let affected_panels = query_panels_containing_refnos(refnos).await?;
    info!("找到 {} 个受影响的房间面板", affected_panels.len());

    if affected_panels.is_empty() {
        warn!("没有找到受影响的房间面板");
        return Ok(IncrementalUpdateResult {
            affected_rooms: 0,
            updated_elements: refnos.len(),
            duration_ms: start_time.elapsed().as_millis() as u64,
        });
    }

    // 2. 删除这些面板的旧关系
    delete_room_relations_for_panels(&affected_panels).await?;
    info!("已删除 {} 个面板的旧房间关系", affected_panels.len());

    // 3. 重新计算并保存新关系
    let db_option = aios_core::get_db_option();
    let mesh_dir = db_option.get_meshes_path();

    // 获取所有房间面板（用于排除）
    let room_key_words = db_option.get_room_key_word();
    let all_room_panels = build_room_panels_relate_for_query(&room_key_words).await?;
    let exclude_panel_refnos: HashSet<RefnoEnum> = all_room_panels
        .iter()
        .flat_map(|(_, _, panels)| panels.clone())
        .collect();
    let exclude_panel_refnos = Arc::new(exclude_panel_refnos);

    let compute_options = RoomComputeOptions::default();
    CACHE_METRICS.reset();

    let mut updated_elements = 0;
    let affected_rooms = affected_panels.len();

    // 并发处理每个面板
    use futures::stream::{self, StreamExt};

    let results = stream::iter(affected_panels)
        .map(|(panel_refno, room_num)| {
            let mesh_dir = mesh_dir.clone();
            let exclude_panel_refnos = exclude_panel_refnos.clone();
            let options = compute_options;
            async move {
                process_panel_for_room(
                    &mesh_dir,
                    panel_refno,
                    &room_num,
                    exclude_panel_refnos.as_ref(),
                    options,
                )
                .await
            }
        })
        .buffer_unordered(compute_options.concurrency.max(1))
        .collect::<Vec<_>>()
        .await;

    updated_elements = results.iter().sum();

    let duration = start_time.elapsed();
    info!(
        "增量更新完成: {} 个房间, {} 个元素, 耗时 {:?}",
        affected_rooms, updated_elements, duration
    );

    Ok(IncrementalUpdateResult {
        affected_rooms,
        updated_elements,
        duration_ms: duration.as_millis() as u64,
    })
}

/// 查询包含指定 refnos 的房间面板
#[cfg(all(not(target_arch = "wasm32"), any(feature = "sqlite-index", feature = "duckdb-feature")))]
async fn query_panels_containing_refnos(
    refnos: &[RefnoEnum],
) -> anyhow::Result<Vec<(RefnoEnum, String)>> {
    if refnos.is_empty() {
        return Ok(Vec::new());
    }

    // 构建查询条件
    let refno_keys: Vec<String> = refnos.iter().map(|r| r.to_pe_key()).collect();
    let refno_list = refno_keys.join(",");

    // 查询包含这些 refnos 的房间面板关系
    // 查询包含这些 refnos 的房间面板关系
    let sql = format!(
        r#"
        select value [`in`, room_num] 
        from room_relate 
        where `out` in [{}]
        group by `in`, room_num
        "#,
        refno_list
    );

    let mut response = SUL_DB.query(sql).await?;
    let raw_result: Vec<(RecordId, String)> = response.take(0)?;

    let panels: Vec<(RefnoEnum, String)> = raw_result
        .into_iter()
        .map(|(panel_id, room_num)| (RefnoEnum::from(panel_id), room_num))
        .collect();

    Ok(panels)
}

/// 删除指定面板的房间关系
#[cfg(all(not(target_arch = "wasm32"), any(feature = "sqlite-index", feature = "duckdb-feature")))]
async fn delete_room_relations_for_panels(panels: &[(RefnoEnum, String)]) -> anyhow::Result<()> {
    if panels.is_empty() {
        return Ok(());
    }

    let panel_keys: Vec<String> = panels.iter().map(|(p, _)| p.to_pe_key()).collect();
    let panel_list = panel_keys.join(",");

    let sql = format!("delete room_relate where `in` in [{}];", panel_list);

    SUL_DB.query(sql).await?;
    debug!("已删除 {} 个面板的房间关系", panels.len());

    Ok(())
}

/// 专门的房间模型重新生成函数
///
/// 根据房间关键词查询房间，收集所有相关构件，重新生成模型并更新关系
///
/// # 参数
/// * `room_keywords` - 房间关键词列表
/// * `db_option` - 数据库配置
/// * `force_regenerate` - 是否强制重新生成
///
/// # 返回值
/// * `(房间数, 元素数, 耗时ms)` - 处理结果统计
#[cfg(all(not(target_arch = "wasm32"), any(feature = "sqlite-index", feature = "duckdb-feature")))]
pub async fn regenerate_room_models_by_keywords(
    room_keywords: &Vec<String>,
    db_option: &DbOption,
    force_regenerate: bool,
) -> anyhow::Result<(usize, usize, u64)> {
    let start_time = Instant::now();
    info!("开始重新生成房间模型，关键词: {:?}", room_keywords);

    // 1. 查询房间和面板关系
    let room_panel_map = build_room_panels_relate(room_keywords).await?;
    let room_count = room_panel_map.len();
    info!("找到 {} 个房间", room_count);

    if room_panel_map.is_empty() {
        warn!("没有找到匹配的房间");
        return Ok((0, 0, start_time.elapsed().as_millis() as u64));
    }

    // 2. 收集所有需要生成的 refnos（面板 + 房间内构件）
    let mut all_refnos = HashSet::new();
    let mesh_dir = db_option.get_meshes_path();
    let exclude_panel_refnos: HashSet<RefnoEnum> = room_panel_map
        .iter()
        .flat_map(|(_, _, panels)| panels.clone())
        .collect();

    // 收集面板
    for (_, _, panel_refnos) in &room_panel_map {
        for panel_refno in panel_refnos {
            all_refnos.insert(*panel_refno);
        }
    }

    // 收集房间内构件
    info!("正在查询房间内构件...");
    for (_, _, panel_refnos) in &room_panel_map {
        for panel_refno in panel_refnos {
            match cal_room_refnos(&mesh_dir, *panel_refno, &exclude_panel_refnos, 0.1).await {
                Ok(refnos) => {
                    all_refnos.extend(refnos);
                }
                Err(e) => {
                    warn!("查询房间构件失败: panel={}, error={}", panel_refno, e);
                }
            }
        }
    }

    let element_count = all_refnos.len();
    info!("需要重新生成 {} 个元素的模型", element_count);

    // 3. 重新生成模型（这里需要调用模型生成函数）
    // 注意：实际的模型生成需要在调用方完成，这里只返回需要生成的 refnos
    // 因为模型生成函数 gen_all_geos_data 需要更多的配置参数

    let duration_ms = start_time.elapsed().as_millis() as u64;
    Ok((room_count, element_count, duration_ms))
}

/// 针对特定房间重建关系（不生成模型）
///
/// # 参数
/// * `room_numbers` - 房间号列表（可选，为空则处理所有房间）
/// * `db_option` - 数据库配置
///
/// # 返回值
/// * `RoomBuildStats` - 构建统计信息
#[cfg(all(not(target_arch = "wasm32"), any(feature = "sqlite-index", feature = "duckdb-feature")))]
pub async fn rebuild_room_relations_for_rooms(
    room_numbers: Option<Vec<String>>,
    db_option: &DbOption,
) -> anyhow::Result<RoomBuildStats> {
    info!("开始重建房间关系");
    let start_time = Instant::now();

    let mesh_dir = db_option.get_meshes_path();
    let room_key_words = db_option.get_room_key_word();
    let compute_options = RoomComputeOptions::default();

    // 1. 查询房间面板关系
    let mut room_panel_map = build_room_panels_relate(&room_key_words).await?;

    // 2. 如果指定了房间号，进行过滤
    if let Some(ref numbers) = room_numbers {
        let numbers_set: HashSet<String> = numbers.iter().cloned().collect();
        room_panel_map.retain(|(_, room_num, _)| numbers_set.contains(room_num));
        info!("过滤后剩余 {} 个房间", room_panel_map.len());
    }

    if room_panel_map.is_empty() {
        warn!("没有找到需要处理的房间");
        return Ok(RoomBuildStats {
            total_rooms: 0,
            total_panels: 0,
            total_components: 0,
            build_time_ms: 0,
            cache_hit_rate: 0.0,
            memory_usage_mb: 0.0,
        });
    }

    let exclude_panel_refnos: HashSet<RefnoEnum> = room_panel_map
        .iter()
        .flat_map(|(_, _, panels)| panels.clone())
        .collect();

    // 3. 删除旧关系
    let panels_to_delete: Vec<(RefnoEnum, String)> = room_panel_map
        .iter()
        .flat_map(|(_, room_num, panels)| panels.iter().map(move |p| (*p, room_num.clone())))
        .collect();
    delete_room_relations_for_panels(&panels_to_delete).await?;
    info!("已删除 {} 个面板的旧关系", panels_to_delete.len());

    CACHE_METRICS.reset();

    let stats = compute_room_relations(
        &mesh_dir,
        room_panel_map,
        exclude_panel_refnos,
        compute_options,
    )
    .await;

    info!(
        "房间关系重建完成: {} 个房间, {} 个面板, {} 个构件, 耗时 {:?}, 缓存命中率 {:.2}%",
        stats.total_rooms,
        stats.total_panels,
        stats.total_components,
        Duration::from_millis(stats.build_time_ms),
        stats.cache_hit_rate * 100.0
    );

    Ok(stats)
}
