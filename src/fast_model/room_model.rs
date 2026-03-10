use aios_core::RecordId;

use aios_core::options::DbOption;

use aios_core::room::algorithm::*;

use aios_core::shape::pdms_shape::PlantMesh;

use aios_core::{GeomInstQuery, ModelHashInst, RefU64, RefnoEnum, model_primary_db};



use dashmap::DashMap;

use glam::{Mat4, Vec3};

use itertools::Itertools;

use parry3d::bounding_volume::{Aabb, BoundingVolume};

use parry3d::math::{Isometry, Vector};

use parry3d::math::{Point, Real};

use parry3d::query::PointQuery;

use parry3d::query::{Ray, RayCast};

use parry3d::shape::{TriMesh, TriMeshFlags};

use regex::Regex;

use serde::{Deserialize, Serialize};

use serde_json::Value as JsonValue;

use std::collections::{HashMap, HashSet};

use std::env;

use std::path::{Path, PathBuf};

use std::sync::Arc;



#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

use crate::spatial_index::SqliteSpatialIndex;

use std::sync::atomic::{AtomicU64, Ordering};

use std::time::{Duration, Instant};

#[cfg(feature = "sqlite-index")]

use tokio_util::sync::CancellationToken;

use tracing::{debug, error, info, warn};

use indicatif::{ProgressBar, ProgressStyle};







/// Room calc environment config (replaces runtime unsafe env::set_var).

///

/// Initialized once at `build_room_relations` entry via `init_room_calc_config`,

/// then read via `get_room_calc_config()`.

#[derive(Debug, Clone)]

struct RoomCalcEnvConfig {

    cache_dir: PathBuf,

    use_cache: bool,

    force_cache: bool,

}



static ROOM_CALC_CONFIG: std::sync::OnceLock<RoomCalcEnvConfig> = std::sync::OnceLock::new();

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
static SPATIAL_INDEX_SCOPE: std::sync::LazyLock<std::sync::Mutex<Option<SpatialIndexScope>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpatialIndexScope {
    Full,
    Scoped,
}

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
impl SpatialIndexScope {
    fn from_filters(db_nums: Option<&[u32]>, refno_root: Option<RefnoEnum>) -> Self {
        if db_nums.is_none() && refno_root.is_none() {
            Self::Full
        } else {
            Self::Scoped
        }
    }
}

/// 确保 spatial_index.sqlite 已从 inst_relate_aabb 刷新（进程生命周期内至多执行一次）。
///
/// `force=true` 时忽略缓存、强制重新刷新（全量 build_room_relations 使用）。
/// `force=false` 时仅首次调用执行（单 panel cal_room_refnos 路径使用）。
#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
async fn ensure_spatial_index_ready(
    db_nums: Option<&[u32]>,
    refno_root: Option<RefnoEnum>,
    force: bool,
) -> anyhow::Result<()> {
    let requested_scope = SpatialIndexScope::from_filters(db_nums, refno_root);

    if !force && requested_scope == SpatialIndexScope::Full {
        let known_scope = *SPATIAL_INDEX_SCOPE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if known_scope == Some(SpatialIndexScope::Full) {
            if let Ok(idx) = SqliteSpatialIndex::with_default_path() {
                if idx.get_stats().map(|stats| stats.total_elements).unwrap_or(0) > 0 {
                    return Ok(());
                }
            }
        }
    }

    let result = refresh_sqlite_spatial_index_from_inst_relate_aabb(db_nums, refno_root).await;
    match &result {
        Ok(count) => {
            info!("[room_model] ensure_spatial_index_ready: 索引就绪, inserted={count}");
            *SPATIAL_INDEX_SCOPE
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(requested_scope);
        }
        Err(e) => {
            let msg = format!("{e:#}");
            error!("[room_model] ensure_spatial_index_ready: 索引刷新失败: {msg}");
            *SPATIAL_INDEX_SCOPE
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        }
    }
    result.map(|_| ())
}

/// Resolve room calc config from env vars (read-only) and DbOption defaults.

///

/// Does NOT modify any global env vars - safe for multi-thread/async use.

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

fn resolve_room_calc_env_config(db_option: &DbOption) -> RoomCalcEnvConfig {

    let cache_dir = env::var("MODEL_CACHE_DIR")

        .ok()

        .filter(|s| !s.trim().is_empty())

        .map(PathBuf::from)

        .unwrap_or_else(|| {

            PathBuf::from("output")

                .join(db_option.project_name.as_str())

                .join("instance_cache")

        });



    let force_cache = env::var("AIOS_ROOM_FORCE_CACHE")

        .ok()

        .and_then(|v| parse_bool(v.trim()))

        .unwrap_or(true);



    let use_cache = force_cache

        || env::var("AIOS_ROOM_USE_CACHE")

            .ok()

            .and_then(|v| parse_bool(v.trim()))

            .unwrap_or(true);



    RoomCalcEnvConfig {

        cache_dir,

        use_cache,

        force_cache,

    }

}



/// Initialize global room calc config (only first call takes effect).

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

fn init_room_calc_config(db_option: &DbOption) {

    let _ = ROOM_CALC_CONFIG.set(resolve_room_calc_env_config(db_option));

}



/// Get room calc config. If not initialized (e.g. called from test/tool directly),

/// returns fallback defaults based on env vars only.

fn get_room_calc_config() -> RoomCalcEnvConfig {

    if let Some(cfg) = ROOM_CALC_CONFIG.get() {

        return cfg.clone();

    }

    // Fallback: if init_room_calc_config not called, derive reasonable defaults from env

    let cache_dir = env::var("MODEL_CACHE_DIR")

        .ok()

        .filter(|s| !s.trim().is_empty())

        .map(PathBuf::from)

        .unwrap_or_else(|| PathBuf::from("output/instance_cache"));

    let force_cache = env::var("AIOS_ROOM_FORCE_CACHE")

        .ok()

        .and_then(|v| parse_bool(v.trim()))

        .unwrap_or(false);

    let use_cache = force_cache

        || env::var("AIOS_ROOM_USE_CACHE")

            .ok()

            .and_then(|v| parse_bool(v.trim()))

            .unwrap_or(true);

    RoomCalcEnvConfig {

        cache_dir,

        use_cache,

        force_cache,

    }

}



#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

async fn query_insts_for_room_calc(

    refnos: &[RefnoEnum],

    enable_holes: bool,

) -> anyhow::Result<Vec<GeomInstQuery>> {

    let config = get_room_calc_config();



    if !config.use_cache {

        return aios_core::query_insts(refnos, enable_holes).await;

    }



    crate::fast_model::export_model::model_exporter::query_geometry_instances_ext_from_cache(

        refnos,

        &config.cache_dir,

        enable_holes,

        false,

        parse_env_bool("AIOS_ROOM_QUERY_VERBOSE", false),

    )

    .await

}

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
#[derive(Debug, Deserialize, SurrealValue)]
struct InstRelateAabbRow {
    refno: RefnoEnum,
    #[serde(default)]
    noun: String,
    aabb: JsonValue,
}

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
fn parse_inst_relate_aabb(value: &JsonValue) -> Option<Aabb> {
    if value.is_null() {
        return None;
    }

    if let Ok(aabb) = serde_json::from_value::<Aabb>(value.clone()) {
        return Some(aabb);
    }

    let read_xyz = |node: &JsonValue| -> Option<[f32; 3]> {
        Some([
            node.get("x")?.as_f64()? as f32,
            node.get("y")?.as_f64()? as f32,
            node.get("z")?.as_f64()? as f32,
        ])
    };
    let read_arr3 = |node: &JsonValue| -> Option<[f32; 3]> {
        let arr = node.as_array()?;
        if arr.len() < 3 {
            return None;
        }
        Some([
            arr[0].as_f64()? as f32,
            arr[1].as_f64()? as f32,
            arr[2].as_f64()? as f32,
        ])
    };

    if let (Some(mins), Some(maxs)) = (value.get("mins"), value.get("maxs")) {
        if let (Some(min), Some(max)) = (read_xyz(mins), read_xyz(maxs)) {
            return Some(Aabb::new(min.into(), max.into()));
        }
        if let (Some(min), Some(max)) = (read_arr3(mins), read_arr3(maxs)) {
            return Some(Aabb::new(min.into(), max.into()));
        }
    }

    if let (Some(min), Some(max)) = (value.get("min"), value.get("max")) {
        if let (Some(min), Some(max)) = (read_arr3(min), read_arr3(max)) {
            return Some(Aabb::new(min.into(), max.into()));
        }
    }

    None
}

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
#[derive(Debug, Deserialize, SurrealValue)]
struct QueryAabbRowRaw {
    refno: RefnoEnum, // inst_relate_aabb 普通表字段，RefnoEnum 可直接反序列化
    aabb: JsonValue,
}

/// 从 inst_relate_aabb 批量查询 refno -> Aabb 映射。
/// inst_relate_aabb 为普通表（refno, aabb_id），通过 refno 过滤。
/// 同 refno 多条记录时取 union Aabb（merge）。
#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
pub(crate) async fn query_aabb_from_inst_relate_aabb(
    refnos: &[RefnoEnum],
) -> anyhow::Result<HashMap<RefnoEnum, Aabb>> {
    if refnos.is_empty() {
        return Ok(HashMap::new());
    }

    let pe_keys: Vec<String> = refnos.iter().map(|r| r.to_pe_key()).collect();
    let ids = pe_keys.join(",");

    let sql = format!(
        "SELECT refno, aabb_id.d as aabb FROM inst_relate_aabb WHERE refno IN [{ids}] AND aabb_id.d != NONE"
    );

    let mut response = model_primary_db().query(&sql).await?;
    let rows: Vec<QueryAabbRowRaw> = response.take(0)?;
    let total_rows = rows.len();

    let debug_query = env::var("AIOS_ROOM_DEBUG")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    let mut map = HashMap::new();
    let mut skipped_refno = 0usize;
    let mut skipped_aabb = 0usize;
    for row in rows {
        if !row.refno.is_valid() {
            skipped_refno += 1;
            if debug_query {
                debug!("query_aabb_from_inst_relate_aabb: 跳过 refno invalid {}", row.refno);
            }
            continue;
        }
        let Some(aabb) = parse_inst_relate_aabb(&row.aabb) else {
            skipped_aabb += 1;
            if debug_query {
                debug!(
                    "query_aabb_from_inst_relate_aabb: 跳过 aabb 解析失败 refno={}",
                    row.refno
                );
            }
            continue;
        };
        map.entry(row.refno)
            .and_modify(|acc| *acc = merge_aabb(acc, &aabb))
            .or_insert(aabb);
    }
    if debug_query {
        println!(
            "[room_debug] query_aabb_from_inst_relate_aabb: rows={} skipped_refno={} skipped_aabb={} map_size={}",
            total_rows, skipped_refno, skipped_aabb, map.len()
        );
        if total_rows > 0 && map.is_empty() {
            println!("[room_debug] 所有 {} 行均被过滤", total_rows);
        }
    }
    Ok(map)
}

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
async fn refresh_sqlite_spatial_index_from_inst_relate_aabb(
    db_nums: Option<&[u32]>,
    refno_root: Option<RefnoEnum>,
) -> anyhow::Result<usize> {
    use crate::data_interface::db_meta;
    use crate::fast_model::query_compat::query_visible_geo_descendants;

    const CHUNK_SIZE: usize = 5000;
    let db_filter: Option<HashSet<u32>> = db_nums.map(|nums| nums.iter().copied().collect());

    if db_filter.is_some() {
        db_meta().ensure_loaded().map_err(|e| {
            anyhow::anyhow!(
                "房间计算前刷新 SQLite 索引失败：无法加载 db_meta_info 映射: {}",
                e
            )
        })?;
    }

    let refno_filter: Option<HashSet<RefU64>> = if let Some(root) = refno_root {
        let set = query_visible_geo_descendants(root, true, None)
            .await?
            .into_iter()
            .map(|r| r.refno())
            .collect::<HashSet<_>>();
        Some(set)
    } else {
        None
    };

    let idx = SqliteSpatialIndex::with_default_path()?;
    idx.clear()?;

    let mut offset: usize = 0;
    let mut total_inserted: usize = 0;
    let mut skipped_by_db: usize = 0;
    let mut skipped_by_refno_root: usize = 0;
    let mut skipped_by_missing_dbmap: usize = 0;
    let mut skipped_by_invalid_aabb: usize = 0;

    loop {
        let sql = format!(
            "SELECT refno, refno.noun ?? '' as noun, aabb_id.d as aabb \
             FROM inst_relate_aabb \
             WHERE aabb_id.d != NONE \
             ORDER BY refno \
             LIMIT {CHUNK_SIZE} START {offset}"
        );
        let mut response = model_primary_db().query(&sql).await?;
        let rows: Vec<InstRelateAabbRow> = response.take(0)?;

        if rows.is_empty() {
            break;
        }
        offset += CHUNK_SIZE;

        let mut batch: Vec<(i64, String, f64, f64, f64, f64, f64, f64)> =
            Vec::with_capacity(rows.len());
        for row in rows {
            let ref_u64 = row.refno.refno();

            if let Some(filter) = &refno_filter {
                if !filter.contains(&ref_u64) {
                    skipped_by_refno_root += 1;
                    continue;
                }
            }

            if let Some(filter) = &db_filter {
                match db_meta().get_dbnum_by_refno(row.refno) {
                    Some(dbnum) if filter.contains(&dbnum) => {}
                    Some(_) => {
                        skipped_by_db += 1;
                        continue;
                    }
                    None => {
                        skipped_by_missing_dbmap += 1;
                        continue;
                    }
                }
            }

            let Some(aabb) = parse_inst_relate_aabb(&row.aabb) else {
                skipped_by_invalid_aabb += 1;
                continue;
            };

            batch.push((
                ref_u64.0 as i64,
                row.noun,
                aabb.mins.x as f64,
                aabb.maxs.x as f64,
                aabb.mins.y as f64,
                aabb.maxs.y as f64,
                aabb.mins.z as f64,
                aabb.maxs.z as f64,
            ));
        }

        if !batch.is_empty() {
            total_inserted += idx.inner().insert_aabbs_with_items(batch)?;
        }
    }

    info!(
        "[room_model] SQLite AABB 刷新完成: inserted={}, skipped_db={}, skipped_refno_root={}, skipped_missing_dbmap={}, skipped_invalid_aabb={}",
        total_inserted,
        skipped_by_db,
        skipped_by_refno_root,
        skipped_by_missing_dbmap,
        skipped_by_invalid_aabb
    );
    Ok(total_inserted)
}



/// 房间关系构建统计信息

#[derive(Debug, Clone, Serialize, Deserialize)]

pub struct RoomBuildStats {

    pub total_rooms: usize,

    pub total_panels: usize,

    pub total_components: usize,

    pub build_time_ms: u64,

    pub cache_hit_rate: f32,

    pub memory_usage_mb: f32,

    /// 计算失败的面板数（几何查询/加载/保存失败）

    #[serde(default)]

    pub failed_panels: usize,

    /// 候选构件缓存缺失数量

    #[serde(default)]

    pub missing_candidates: usize,

}



fn parse_env_bool(name: &str, default_value: bool) -> bool {

    env::var(name)

        .ok()

        .map(|v| {

            let v = v.trim();

            v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")

        })

        .unwrap_or(default_value)

}



#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

fn append_room_calc_missing_refnos(

    panel_refno: RefnoEnum,

    phase: &str,

    missing: &[RefnoEnum],

) -> anyhow::Result<()> {

    if missing.is_empty() {

        return Ok(());

    }



    // 仅当用户显式开启时才写文件；避免默认产生大量日志文件。

    // - AIOS_ROOM_MISSING_LOG=1/true：写到默认路径 output/room_calc_missing_refnos.jsonl

    // - AIOS_ROOM_MISSING_LOG=/path/to/file.jsonl：写到指定路径

    let Some(raw) = env::var("AIOS_ROOM_MISSING_LOG").ok().filter(|s| !s.trim().is_empty()) else {

        return Ok(());

    };



    let path = {

        let v = raw.trim();

        if v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes") {

            PathBuf::from("output").join("room_calc_missing_refnos.jsonl")

        } else {

            PathBuf::from(v)

        }

    };



    if let Some(parent) = path.parent() {

        let _ = std::fs::create_dir_all(parent);

    }



    let ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    let row = serde_json::json!({

        "ts": ts,

        "panel": panel_refno.to_string(),

        "phase": phase,

        "missing_refnos": missing.iter().map(|r| r.to_string()).collect::<Vec<_>>(),

    });



    use std::io::Write;

    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;

    writeln!(f, "{}", row.to_string())?;

    Ok(())

}



// ensure_room_calc_cache_env removed — replaced by init_room_calc_config / get_room_calc_config

// (no unsafe env::set_var needed; config is resolved once and stored in OnceLock)



#[cfg(all(

    not(target_arch = "wasm32"),

    feature = "sqlite-index",

    feature = "gen_model"

))]

#[cfg_attr(feature = "profile", tracing::instrument(skip_all, name = "pregen_panels_cache"))]

async fn pregen_room_panels_into_model_cache(

    db_option: &DbOption,

    room_panel_map: &[(RefnoEnum, String, Vec<RefnoEnum>)],

) -> anyhow::Result<()> {

    // 默认开启；需要禁用时可设置 AIOS_ROOM_PREGEN_PANELS=0。

    if !parse_env_bool("AIOS_ROOM_PREGEN_PANELS", true) {

        return Ok(());

    }



    use crate::data_interface::db_meta_manager::db_meta;

    use crate::fast_model::instance_cache::InstanceCacheManager;

    use crate::options::DbOptionExt;



    let mut panels: HashSet<RefnoEnum> = HashSet::new();

    for (_, _, ps) in room_panel_map {

        for p in ps {

            panels.insert(*p);

        }

    }

    if panels.is_empty() {

        return Ok(());

    }



    // 确保 db_meta 已加载，便于 ref0->cache_dbnum 推导。

    db_meta().ensure_loaded().map_err(|e| {

        anyhow::anyhow!(

            "房间计算无法加载 db_meta_info.json（需要 ref0->cache_dbnum 映射以写入 model cache）: {}",

            e

        )

    })?;



    let cache_dir = env::var("MODEL_CACHE_DIR")

        .ok()

        .filter(|s| !s.trim().is_empty())

        .map(PathBuf::from)

        .unwrap_or_else(|| PathBuf::from("output").join(db_option.project_name.as_str()).join("instance_cache"));



    let cache = InstanceCacheManager::new(&cache_dir).await?;



    // 按 dbnum 分组
    let mut groups: HashMap<u32, Vec<RefnoEnum>> = HashMap::new();
    for &p in &panels {
        let Some(dbnum) = db_meta().get_dbnum_by_refno(p) else {
            continue;
        };
        if dbnum == 0 {
            continue;
        }
        groups.entry(dbnum).or_default().push(p);
    }

    let mut missing: Vec<RefnoEnum> = Vec::new();
    for (dbnum, refnos) in groups {
        for r in refnos {
            let Some(info) = cache.get_inst_info(dbnum, r).await else {
                missing.push(r);
                continue;
            };
            // 检查是否有非空几何数据
            if !info.inst_key.is_empty() {
                if let Some(geos) = cache.get_inst_geos(dbnum, &info.inst_key).await {
                    if !geos.geos_data.insts.is_empty() {
                        continue; // 已找到，不缺失
                    }
                }
            }
            missing.push(r);
        }
    }



    if missing.is_empty() {

        return Ok(());

    }



    // 复用既有 model cache 定向生成流程（manual_refnos 路径）。

    // 注意：这里“不把模型写回 SurrealDB(inst_*/geo_*)”，只写 model cache + mesh + sqlite 索引（若启用）。

    let mut opt = DbOptionExt::from(db_option.clone());

    opt.export_instances = false;

    opt.inner.gen_model = true;

    opt.inner.gen_mesh = true;

    opt.inner.enable_sqlite_rtree = true;

    opt.inner.apply_boolean_operation = false;

    opt.inner.replace_mesh = Some(false);

    opt.inner.save_db = Some(false);



    // 手动 refnos 可能跨 dbnum：确保 db_meta 路径与 cache_dir 一致。

    opt.model_cache_dir = Some(cache_dir.to_string_lossy().to_string());



    // 输入数据仍需从 SurrealDB 读取（属性/loops/world_transform 等）。

    aios_core::init_surreal().await?;



    info!(

        "房间计算：检测到 {} 个 panel 缺失模型数据，开始定向补齐（写 model cache）",

        missing.len()

    );

    crate::fast_model::gen_model::gen_all_geos_data(missing.clone(), &opt, None, opt.target_sesno)

        .await

        .map_err(|e| anyhow::anyhow!("定向补齐 panel 模型失败: {}", e))?;



    Ok(())

}



#[derive(Debug, Clone, Copy)]

pub struct RoomComputeOptions {

    inside_tol: f32,

    concurrency: usize,

    candidate_limit: Option<usize>,

    candidate_concurrency: usize,

    refresh_spatial_index: bool,

    query_from_cache: bool,

}



impl Default for RoomComputeOptions {

    fn default() -> Self {

        Self {

            inside_tol: 0.1,

            concurrency: default_room_concurrency(),

            candidate_limit: default_candidate_limit(),

            candidate_concurrency: default_candidate_concurrency(),

            refresh_spatial_index: true,

            query_from_cache: true,

        }

    }

}

impl RoomComputeOptions {
    pub fn with_prebuilt_spatial_index(mut self) -> Self {
        self.refresh_spatial_index = false;
        self
    }

    pub fn refresh_spatial_index_enabled(&self) -> bool {
        self.refresh_spatial_index
    }

    pub fn with_surreal_query(mut self) -> Self {
        self.query_from_cache = false;
        self
    }

    pub fn query_from_cache_enabled(&self) -> bool {
        self.query_from_cache
    }
}



/// 地板 2D 回退判定的配置参数（从环境变量一次性读取，避免热路径重复读取）

#[derive(Debug, Clone, Copy)]

struct Floor2dConfig {

    enabled: bool,

    z_thickness_max: Real,

    extrude_height: Option<f32>,

}



impl Floor2dConfig {

    fn from_env() -> Self {

        Self {

            enabled: env_bool_opt("ROOM_RELATION_FLOOR_2D_FALLBACK").unwrap_or(true),

            z_thickness_max: env_f32("ROOM_RELATION_FLOOR_2D_THICKNESS_MAX", 0.2) as Real,

            extrude_height: env_f32_opt("ROOM_RELATION_FLOOR_2D_EXTRUDE_HEIGHT"),

        }

    }

}



fn parse_bool(s: &str) -> Option<bool> {

    let v = s.trim().to_ascii_lowercase();

    match v.as_str() {

        "1" | "true" | "yes" | "y" | "on" => Some(true),

        "0" | "false" | "no" | "n" | "off" => Some(false),

        _ => None,

    }

}



fn env_bool_opt(key: &str) -> Option<bool> {

    env::var(key).ok().and_then(|v| parse_bool(&v))

}



fn env_f32_opt(key: &str) -> Option<f32> {

    env::var(key).ok().and_then(|v| v.trim().parse::<f32>().ok())

}



fn env_f32(key: &str, default: f32) -> f32 {

    env_f32_opt(key).unwrap_or(default)

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

/// 5. 支持 dbnum 和 refno 子树范围限制

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

#[cfg_attr(feature = "profile", tracing::instrument(skip(db_option)))]

pub async fn build_room_relations(

    db_option: &DbOption,

    db_nums: Option<&[u32]>,

    refno_root: Option<RefnoEnum>,

) -> anyhow::Result<RoomBuildStats> {

    build_room_relations_with_cancel(db_option, db_nums, refno_root, None, None).await

}



/// 支持取消和进度回调的房间关系构建

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

pub async fn build_room_relations_with_cancel(

    db_option: &DbOption,

    db_nums: Option<&[u32]>,

    refno_root: Option<RefnoEnum>,

    cancel_token: Option<CancellationToken>,

    progress_callback: Option<Box<dyn Fn(f32, &str) + Send + Sync>>,

) -> anyhow::Result<RoomBuildStats> {

    info!("开始构建房间关系 (支持取消和进度)");
    let full_rebuild = db_nums.is_none() && refno_root.is_none();



    if let Some(ref cb) = progress_callback {

        cb(0.0, "开始构建房间关系");

    }



    let mesh_dir = db_option.get_meshes_path();

    let room_key_words = db_option.get_room_key_word();

    let compute_options = RoomComputeOptions::default();



    CACHE_METRICS.reset();



    #[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

    init_room_calc_config(db_option);

    #[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
    {
        if let Some(ref cb) = progress_callback {
            cb(0.02, "正在刷新 SQLite AABB 索引");
        }
        ensure_spatial_index_ready(db_nums, refno_root.clone(), true).await?;
    }



    // 1. 构建房间面板映射关系

    if let Some(ref cb) = progress_callback {

        cb(0.05, "正在查询房间面板映射关系");

    }

    let mut room_panel_map = build_room_panels_relate_for_query(&room_key_words).await?;

    info!("查询到 {} 个房间面板映射关系", room_panel_map.len());



    // 2. 应用 dbnum 过滤

    if let Some(db_nums) = db_nums {

        use crate::data_interface::db_meta;

        let _ = db_meta().ensure_loaded();

        let db_num_set: HashSet<u32> = db_nums.iter().copied().collect();

        room_panel_map.retain(|(refno, _, _)| {

            // ref0 != dbnum，必须通过 db_meta 映射

            match db_meta().get_dbnum_by_refno(*refno) {

                Some(dbnum) => db_num_set.contains(&dbnum),

                None => {

                    log::warn!(

                        "[room_model] 缺少 ref0->dbnum 映射，跳过房间过滤: refno={}",

                        refno

                    );

                    false

                }

            }

        });

        info!("dbnum 过滤后剩余 {} 个房间", room_panel_map.len());

    }



    // 3. 应用 refno 子树过滤

    if let Some(root) = refno_root {

        use crate::fast_model::query_compat::query_visible_geo_descendants;

        let visible_refnos: HashSet<RefnoEnum> =

            query_visible_geo_descendants(root, true, None)

                .await?

                .into_iter()

                .collect();



        room_panel_map.retain(|(room_refno, _, panel_refnos)| {

            // 房间本身在子树内，或者有面板在子树内

            visible_refnos.contains(room_refno)

                || panel_refnos.iter().any(|p| visible_refnos.contains(p))

        });

        info!("refno 子树过滤后剩余 {} 个房间", room_panel_map.len());

    }



    let exclude_panel_refnos = room_panel_map

        .iter()

        .map(|(_, _, panel_refnos)| panel_refnos.clone())

        .flatten()

        .collect::<HashSet<_>>();



    info!("找到 {} 个房间面板映射关系", room_panel_map.len());



    #[cfg(all(

        not(target_arch = "wasm32"),

        feature = "sqlite-index",

        feature = "gen_model"

    ))]

    pregen_room_panels_into_model_cache(db_option, &room_panel_map).await?;



    if let Some(ref token) = cancel_token {

        if token.is_cancelled() {

            anyhow::bail!("任务已在查询面板关系后取消");

        }

    }

    let panels_to_delete: Vec<PanelRoom> = room_panel_map
        .iter()
        .flat_map(|(_, room_num, panels)| {
            panels.iter().map(move |panel| PanelRoom {
                panel: *panel,
                room_num: room_num.clone(),
            })
        })
        .collect();

    if full_rebuild {
        delete_all_room_relations().await?;
        create_room_panel_relations_batch(&room_panel_map).await?;
    } else {
        delete_room_relations_for_panels(&panels_to_delete).await?;
        sync_room_panel_relations(&room_panel_map, false).await?;
    }



    let stats = compute_room_relations_with_cancel(

        &mesh_dir,

        room_panel_map,

        exclude_panel_refnos,

        compute_options,

        cancel_token,

        progress_callback,

    )

    .await?;



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



#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

async fn compute_room_relations(

    mesh_dir: &PathBuf,

    room_panel_map: Vec<(RefnoEnum, String, Vec<RefnoEnum>)>,

    exclude_panel_refnos: HashSet<RefnoEnum>,

    options: RoomComputeOptions,

) -> anyhow::Result<RoomBuildStats> {

    compute_room_relations_with_cancel(

        mesh_dir,

        room_panel_map,

        exclude_panel_refnos,

        options,

        None,

        None,

    )

    .await

}



#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

#[cfg_attr(feature = "profile", tracing::instrument(skip_all, name = "compute_room_relations"))]

async fn compute_room_relations_with_cancel(

    mesh_dir: &PathBuf,

    room_panel_map: Vec<(RefnoEnum, String, Vec<RefnoEnum>)>,

    exclude_panel_refnos: HashSet<RefnoEnum>,

    options: RoomComputeOptions,

    cancel_token: Option<CancellationToken>,

    progress_callback: Option<Box<dyn Fn(f32, &str) + Send + Sync>>,

) -> anyhow::Result<RoomBuildStats> {

    let start_time = Instant::now();

    let total_panels = exclude_panel_refnos.len();

    let exclude_panel_refnos = Arc::new(exclude_panel_refnos);



    use futures::stream::{self, StreamExt};



    let total_rooms = room_panel_map.len();



    let pb = ProgressBar::new(total_rooms as u64);

    pb.set_style(

        ProgressStyle::default_bar()

            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}")

            .unwrap()

            .progress_chars("#>-"),

    );

    pb.set_message("房间计算中...");



    let results = stream::iter(room_panel_map)

        .map(|(room_refno, room_num, panel_refnos)| {

            let mesh_dir = mesh_dir.clone();

            let exclude_panel_refnos = exclude_panel_refnos.clone();

            let room_num = room_num.clone();

            let options = options;

            let cancel_token = cancel_token.clone();



            async move {

                // 检查取消

                if let Some(ref token) = cancel_token {

                    if token.is_cancelled() {

                        return (room_refno, 0, 0, true);

                    }

                }



                let mut room_components = 0;
                let mut failed_panels = 0;



                for panel_refno in panel_refnos {

                    if let Some(ref token) = cancel_token {

                        if token.is_cancelled() {

                            return (room_refno, room_components, failed_panels, true);

                        }

                    }

                    let outcome = process_panel_for_room(

                        &mesh_dir,

                        panel_refno,

                        &room_num,

                        exclude_panel_refnos.as_ref(),

                        options,

                    )

                    .await;
                    room_components += outcome.components;
                    failed_panels += usize::from(outcome.failed);

                }



                (room_refno, room_components, failed_panels, false)

            }

        })

        .buffer_unordered(options.concurrency.max(1))

        .map(|res| {

            pb.inc(1);

            // 保留原有的 progress_callback 以支持 Web/GRPC

            if let Some(ref cb) = progress_callback {

                let progress = 0.1 + (pb.position() as f32 / total_rooms as f32) * 0.85;

                cb(progress, &format!("已处理 {}/{} 个房间", pb.position(), total_rooms));

            }

            res

        })

        .collect::<Vec<_>>()

        .await;



    // 检查是否有被取消的

    if results.iter().any(|(_, _, _, cancelled)| *cancelled) {

        anyhow::bail!("任务在计算房间关系过程中被取消");

    }



    let total_rooms = results.len();

    let total_components: usize = results.iter().map(|(_, count, _, _)| *count).sum();
    let failed_panels: usize = results.iter().map(|(_, _, failed, _)| *failed).sum();

    let build_time = start_time.elapsed();



    pb.finish_with_message("房间计算完成");



    Ok(RoomBuildStats {

        total_rooms,

        total_panels,

        total_components,

        build_time_ms: build_time.as_millis() as u64,

        cache_hit_rate: CACHE_METRICS.hit_rate(),

        memory_usage_mb: estimate_memory_usage().await,

        failed_panels,

        missing_candidates: 0,

    })

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

pub async fn build_room_panels_relate_for_query(

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



#[cfg_attr(feature = "profile", tracing::instrument(skip_all, name = "build_room_panels_relate"))]

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



    let mut response = model_primary_db().query(sql).await?;

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



fn build_room_panel_relations_sql(
    room_groups: &[(RefnoEnum, String, Vec<RefnoEnum>)],
) -> String {
    room_groups
        .iter()
        .map(|(room_refno, room_num_str, panel_refnos)| {
            let room_num_escaped = room_num_str.replace('\'', "''");
            format!(
                "relate {}->room_panel_relate->[{}] set room_num='{}';",
                room_refno.to_pe_key(),
                panel_refnos.iter().map(|x| x.to_pe_key()).join(","),
                room_num_escaped
            )
        })
        .join("\n")
}

fn build_delete_room_panel_relations_sql(room_refnos: &[RefnoEnum]) -> Option<String> {
    if room_refnos.is_empty() {
        return None;
    }

    Some(format!(
        "LET $ids = SELECT VALUE id FROM room_panel_relate WHERE out IN [{}];\nDELETE $ids;",
        room_refnos.iter().map(RefnoEnum::to_pe_key).join(",")
    ))
}

fn build_delete_room_relations_sql_for_panels(panel_refnos: &[RefnoEnum]) -> Option<String> {
    if panel_refnos.is_empty() {
        return None;
    }

    Some(format!(
        "LET $ids = SELECT VALUE id FROM [{}]->room_relate;\nDELETE $ids;",
        panel_refnos.iter().map(RefnoEnum::to_pe_key).join(",")
    ))
}

async fn create_room_panel_relations_batch(
    room_groups: &[(RefnoEnum, String, Vec<RefnoEnum>)],
) -> anyhow::Result<()> {
    let batch_sql = build_room_panel_relations_sql(room_groups);
    if batch_sql.is_empty() {
        return Ok(());
    }

    model_primary_db().query(batch_sql).await?;
    Ok(())
}

async fn sync_room_panel_relations(
    room_groups: &[(RefnoEnum, String, Vec<RefnoEnum>)],
    clear_all_first: bool,
) -> anyhow::Result<()> {
    let mut sql_statements = Vec::new();

    if clear_all_first {
        sql_statements.push("DELETE room_panel_relate;".to_string());
    } else {
        let room_refnos: Vec<RefnoEnum> = room_groups.iter().map(|(room_refno, _, _)| *room_refno).collect();
        if let Some(sql) = build_delete_room_panel_relations_sql(&room_refnos) {
            sql_statements.push(sql);
        }
    }

    let batch_sql = build_room_panel_relations_sql(room_groups);
    if !batch_sql.is_empty() {
        sql_statements.push(batch_sql);
    }

    if sql_statements.is_empty() {
        return Ok(());
    }

    model_primary_db().query(sql_statements.join("\n")).await?;
    Ok(())
}

#[derive(Debug, Clone, Copy, Default)]
struct PanelProcessOutcome {
    components: usize,
    failed: bool,
}



#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

#[cfg_attr(feature = "profile", tracing::instrument(skip_all, name = "process_panel_for_room"))]

async fn process_panel_for_room(

    mesh_dir: &PathBuf,

    panel_refno: RefnoEnum,

    room_num: &str,

    exclude_panel_refnos: &HashSet<RefnoEnum>,

    options: RoomComputeOptions,
) -> PanelProcessOutcome {

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
                return PanelProcessOutcome::default();

            }



            if let Err(e) = save_room_relate(panel_refno, &refnos, room_num).await {

                error!("保存房间关系失败: panel={}, error={}", panel_refno, e);

                PanelProcessOutcome {
                    components: 0,
                    failed: true,
                }

            } else {

                PanelProcessOutcome {
                    components: refnos.len(),
                    failed: false,
                }

            }

        }

        Err(e) => {

            warn!("计算房间构件失败: panel={}, error={}", panel_refno, e);

            PanelProcessOutcome {
                components: 0,
                failed: true,
            }

        }

    }

}



/// 改进版本的房间构件计算（支持关键点/凸包两方案）

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

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

/// 粗算（AABB 相交）诊断结果：用于验证 SQLite RTree 粗筛是否正确。
#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
#[derive(Debug, Clone)]
pub struct CoarseAabbDiagnostic {
    pub panel_aabb: Option<Aabb>,
    pub query_aabb: Option<Aabb>,
    pub expect_refno_aabb_intersects: Vec<(RefnoEnum, Option<Aabb>, bool)>,
    pub rtree_candidates: Vec<RefnoEnum>,
    pub expect_refno_in_rtree: Vec<(RefnoEnum, bool)>,
}

/// 粗算诊断：分析 AABB 相交查询是否正确。
#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
pub async fn diagnose_coarse_aabb_intersection(
    panel_refno: RefnoEnum,
    expect_refnos: &[RefnoEnum],
) -> anyhow::Result<CoarseAabbDiagnostic> {
    ensure_spatial_index_ready(None, None, false).await?;
    let floor_2d = Floor2dConfig::from_env();

    let mut panel_aabb: Option<Aabb> =
        query_aabb_from_inst_relate_aabb(&[panel_refno]).await.ok().and_then(|m| m.into_values().next());
    if panel_aabb.is_none() {
        let geom_insts = query_insts_for_room_calc(&[panel_refno], true).await.unwrap_or_default();
        for g in &geom_insts {
            let Some(ref world_aabb) = g.world_aabb else { continue };
            let a: Aabb = world_aabb.clone().into();
            panel_aabb = Some(match panel_aabb {
                None => a,
                Some(acc) => merge_aabb(&acc, &a),
            });
        }
    }

    let query_aabb = match &panel_aabb {
        None => None,
        Some(pa) => {
            let z_thickness = pa.maxs.z - pa.mins.z;
            let q = if !floor_2d.enabled || z_thickness > floor_2d.z_thickness_max {
                *pa
            } else {
                let x_ext = (pa.maxs.x - pa.mins.x).abs();
                let y_ext = (pa.maxs.y - pa.mins.y).abs();
                let extrude = floor_2d
                    .extrude_height
                    .map(|v| v as Real)
                    .unwrap_or_else(|| x_ext.max(y_ext).max(1.0));
                let mut a = *pa;
                a.mins.z -= 0.1;
                a.maxs.z += extrude;
                a
            };
            Some(q)
        }
    };

    let expect_aabb_map = query_aabb_from_inst_relate_aabb(expect_refnos).await.unwrap_or_default();
    let expect_refno_aabb_intersects: Vec<_> = expect_refnos
        .iter()
        .map(|r| {
            let aabb = expect_aabb_map.get(r).copied();
            let intersects = query_aabb
                .as_ref()
                .and_then(|qb| aabb.as_ref().map(|ab| ab.intersects(qb)))
                .unwrap_or(false);
            (*r, aabb, intersects)
        })
        .collect();

    let rtree_candidates = match &query_aabb {
        Some(qa) => {
            let idx = SqliteSpatialIndex::with_default_path()?;
            let ids = idx.query_intersect(qa)?;
            ids.into_iter()
                .filter_map(|id| {
                    let c = RefnoEnum::from(id);
                    if c.is_valid() && c != panel_refno {
                        Some(c)
                    } else {
                        None
                    }
                })
                .collect()
        }
        None => Vec::new(),
    };

    let candidate_set: HashSet<RefnoEnum> = rtree_candidates.iter().cloned().collect();
    let expect_refno_in_rtree: Vec<_> =
        expect_refnos.iter().map(|r| (*r, candidate_set.contains(r))).collect();

    Ok(CoarseAabbDiagnostic {
        panel_aabb,
        query_aabb,
        expect_refno_aabb_intersects,
        rtree_candidates,
        expect_refno_in_rtree,
    })
}

/// 单点验证：使用"简化算法（AABB 8 角点全在）"判断某个构件是否属于指定房间面板。

///

/// 说明：

/// - 该函数不依赖 SQLite 空间索引（不会枚举候选），仅对给定 candidate_refno 做一次判定。

/// - 主要用于测试/回归与现场快速核对（避免全量候选带来的耗时与不确定性）。

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

pub(crate) async fn is_refno_in_panel_by_aabb8(

    mesh_dir: &PathBuf,

    panel_refno: RefnoEnum,

    candidate_refno: RefnoEnum,

    inside_tol: f32,

) -> anyhow::Result<bool> {

    // 1) 加载 panel TriMesh

    let panel_geom_insts =

        query_insts_for_room_calc(&[panel_refno], true).await.unwrap_or_default();

    if panel_geom_insts.is_empty() {

        return Ok(false);

    }



    let mut panel_meshes: Vec<Arc<TriMesh>> = Vec::new();

    for geom_inst in &panel_geom_insts {

        for inst in &geom_inst.insts {

            if let Ok(mesh) = load_geometry_with_enhanced_cache(

                mesh_dir,

                &inst.geo_hash,

                geom_inst.world_trans,

                inst,

            )

            .await

            {

                panel_meshes.push(mesh);

            }

        }

    }

    if panel_meshes.is_empty() {

        return Ok(false);

    }



    // 2) 取 candidate 的 AABB：优先 inst_relate_aabb，缺失则用 query_insts world_aabb

    let mut candidate_aabb: Option<Aabb> = query_aabb_from_inst_relate_aabb(&[candidate_refno])
        .await
        .ok()
        .and_then(|m| m.into_values().next());

    if candidate_aabb.is_none() {
        let candidate_geom_groups =
            query_insts_for_room_calc(&[candidate_refno], true).await.unwrap_or_default();
        for g in &candidate_geom_groups {
            let Some(ref world_aabb) = g.world_aabb else { continue };
            let aabb: Aabb = world_aabb.clone().into();
            candidate_aabb = Some(match candidate_aabb {
                None => aabb,
                Some(acc) => merge_aabb(&acc, &aabb),
            });
        }
    }

    let Some(candidate_aabb) = candidate_aabb else {
        return Ok(false);
    };



    // 3) AABB 8 角点全在

    let corners = extract_aabb_corners(&candidate_aabb);

    let floor_2d = Floor2dConfig::from_env();

    Ok(are_all_points_in_panel(&corners, &panel_meshes, inside_tol, &floor_2d))

}



/// 单点验证：使用"候选 world AABB 的 27 个关键点投票(>50%)"判断归属。

///

/// 与批量房间计算中对候选的判定语义一致，但避免了候选枚举。

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

pub(crate) async fn is_refno_in_panel_by_aabb_vote(

    mesh_dir: &PathBuf,

    panel_refno: RefnoEnum,

    candidate_refno: RefnoEnum,

    inside_tol: f32,

) -> anyhow::Result<bool> {

    let panel_geom_insts =

        query_insts_for_room_calc(&[panel_refno], true).await.unwrap_or_default();

    if panel_geom_insts.is_empty() {

        return Ok(false);

    }



    let mut panel_meshes: Vec<Arc<TriMesh>> = Vec::new();

    for geom_inst in &panel_geom_insts {

        for inst in &geom_inst.insts {

            if let Ok(mesh) = load_geometry_with_enhanced_cache(

                mesh_dir,

                &inst.geo_hash,

                geom_inst.world_trans,

                inst,

            )

            .await

            {

                panel_meshes.push(mesh);

            }

        }

    }

    if panel_meshes.is_empty() {

        return Ok(false);

    }



    let candidate_geom_groups =

        query_insts_for_room_calc(&[candidate_refno], true).await.unwrap_or_default();

    if candidate_geom_groups.is_empty() {

        return Ok(false);

    }



    let mut candidate_aabb: Option<Aabb> = None;

    for g in &candidate_geom_groups {

        let Some(ref world_aabb) = g.world_aabb else { continue };

        let aabb: Aabb = world_aabb.clone().into();

        candidate_aabb = Some(match candidate_aabb {

            None => aabb,

            Some(acc) => merge_aabb(&acc, &aabb),

        });

    }

    let Some(candidate_aabb) = candidate_aabb else {

        return Ok(false);

    };



    let key_points = extract_aabb_key_points(&candidate_aabb);

    let floor_2d = Floor2dConfig::from_env();

    Ok(is_geom_in_panel(&key_points, &panel_meshes, inside_tol, &floor_2d))

}



/// 单点验证：使用候选构件"凸包/凸分解任意重叠"判断归属（已移除精算路径）。

/// 现在统一使用 `is_refno_in_panel_by_aabb_vote` 进行粗算判定。



#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

#[cfg_attr(feature = "profile", tracing::instrument(skip_all, name = "cal_room_refnos_with_options"))]

pub async fn cal_room_refnos_with_options(

    mesh_dir: &PathBuf,

    panel_refno: RefnoEnum,

    exclude_refnos: &HashSet<RefnoEnum>,

    options: RoomComputeOptions,

) -> anyhow::Result<HashSet<RefnoEnum>> {

    let start_time = Instant::now();

    let inside_tol = options.inside_tol;

    let floor_2d = Floor2dConfig::from_env();

    let debug_enabled = env::var("AIOS_ROOM_DEBUG")

        .ok()

        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))

        .unwrap_or(false);



    if debug_enabled {

        println!(

            "[room_calc] panel={} inside_tol={}",

            panel_refno,

            inside_tol

        );

    }



    // 步骤 1：查询面板的几何实例（默认 cache-only）

    let mut panel_geom_insts: Vec<GeomInstQuery> = if options.query_from_cache_enabled() {
        query_insts_for_room_calc(&[panel_refno], true)
            .await
            .unwrap_or_default()
    } else {
        aios_core::query_insts(&[panel_refno], true)
            .await
            .unwrap_or_default()
    };



    // cache 缺失面板模型数据：在房间计算流程内按需补齐（复用 model cache 定向生成流程）。

    // 注意：内层自动补齐逻辑已移除——面板 cache 应在外层 pregen_room_panels_into_model_cache 统一预生成。

    #[cfg(all(

        not(target_arch = "wasm32"),

        feature = "sqlite-index",

        feature = "gen_model"

    ))]

    if options.query_from_cache_enabled()
        && panel_geom_insts.is_empty()
        && parse_env_bool("AIOS_ROOM_AUTOGEN_PANEL", true)
    {

        let db_opt = aios_core::get_db_option();

        let tmp = vec![(RefnoEnum::default(), String::new(), vec![panel_refno])];

        if let Err(e) = pregen_room_panels_into_model_cache(&db_opt, &tmp).await {

            warn!("房间计算自动补齐 panel 模型失败: panel={}, err={}", panel_refno, e);

        } else {

            panel_geom_insts =

                query_insts_for_room_calc(&[panel_refno], true).await.unwrap_or_default();

        }

    }



    if panel_geom_insts.is_empty() {

        debug!("面板 {} 没有几何实例", panel_refno);

        let _ = append_room_calc_missing_refnos(panel_refno, "panel_geom_insts_empty", &[panel_refno]);

        return Ok(Default::default());

    }



    if debug_enabled {

        println!(

            "[room_calc] panel={} geom_groups={}",

            panel_refno,

            panel_geom_insts.len()

        );

        let aabb_cnt = panel_geom_insts.iter().filter(|g| g.world_aabb.is_some()).count();

        println!("[room_calc] panel={} world_aabb_groups={}", panel_refno, aabb_cnt);

        if let Some(g) = panel_geom_insts.first() {

            println!(

                "[room_calc] panel sample: insts={} has_neg={}",

                g.insts.len(),

                g.has_neg

            );

        }

    }



    // 步骤 2：加载面板 TriMesh（用于点包含测试）；panel_aabb 优先从 inst_relate_aabb 获取，缺失则用 inst_info.world_aabb，再缺失则从 TriMesh 推导。

    let mut panel_aabb: Option<Aabb> = if options.query_from_cache_enabled() {
        query_aabb_from_inst_relate_aabb(&[panel_refno])
            .await
            .ok()
            .and_then(|m| m.into_values().next())
    } else {
        None
    };

    if options.query_from_cache_enabled() && panel_aabb.is_none() {
        for geom_inst in &panel_geom_insts {
            let Some(ref world_aabb) = geom_inst.world_aabb else { continue };
            let geom_aabb: Aabb = world_aabb.clone().into();
            panel_aabb = Some(match panel_aabb {
                None => geom_aabb,
                Some(acc) => merge_aabb(&acc, &geom_aabb),
            });
        }
    }



    // 加载面板 TriMesh，用于后续"点在体内/靠近表面"判定。

    let mut panel_meshes: Vec<Arc<TriMesh>> = Vec::new();

    for geom_inst in &panel_geom_insts {

        if geom_inst.insts.is_empty() {

            debug!("面板 {} 的 insts 数组为空", panel_refno);

            continue;

        }



        for inst in &geom_inst.insts {

            match load_geometry_with_enhanced_cache(

                mesh_dir,

                &inst.geo_hash,

                geom_inst.world_trans,

                inst,

            )

            .await

            {

                Ok(mesh) => panel_meshes.push(mesh),

                Err(e) => {

                    warn!("加载面板几何文件失败: {}, error: {}", inst.geo_hash, e);

                }

            }

        }

    }



    if panel_meshes.is_empty() {

        // 面板 mesh 缺失时无法进行“点在体内 / 与边界相交”等基于 TriMesh 的判定，因此直接跳过。

        warn!("面板 {} 无可用 TriMesh，跳过房间计算", panel_refno);

        return Ok(Default::default());

    }



    // panel_aabb 缺失：用已加载的 TriMesh 反推（TriMesh 已应用 world_transform，local_aabb 即 world AABB）。

    if panel_aabb.is_none() {

        for m in &panel_meshes {

            let aabb = m.local_aabb();

            panel_aabb = Some(match panel_aabb {

                None => aabb,

                Some(acc) => merge_aabb(&acc, &aabb),

            });

        }

    }



    let panel_aabb = match panel_aabb {

        Some(aabb) => aabb,

        None => {

            warn!("面板 {} 无法获得 panel_aabb（world_aabb/mesh 均缺失），跳过房间计算", panel_refno);

            return Ok(Default::default());

        }

    };



    if debug_enabled {

        println!(

            "[room_calc] panel={} merged_aabb=({:.3},{:.3},{:.3})..({:.3},{:.3},{:.3})",

            panel_refno,

            panel_aabb.mins.x,

            panel_aabb.mins.y,

            panel_aabb.mins.z,

            panel_aabb.maxs.x,

            panel_aabb.maxs.y,

            panel_aabb.maxs.z

        );

    }



    // 步骤 3：粗算 - 通过空间索引查询候选构件

    // 自动确保 SQLite 空间索引已从 inst_relate_aabb 刷新（进程内至多一次）
    #[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
    if options.refresh_spatial_index_enabled() {
        ensure_spatial_index_ready(None, None, false).await?;
    }

    let coarse_start = Instant::now();



    // 克隆排除列表以避免生命周期问题

    let exclude_set: HashSet<RefU64> = exclude_refnos.iter().map(|r| r.refno()).collect();

    let candidate_limit = options.candidate_limit;



    // 对“薄面板(地板式)”做候选查询 AABB 的 Z 外延：

    // - 3D AABB 相交会漏掉位于地板上方的构件（Z 不相交）

    // - 该外延需与点包含的 2D 兜底语义一致，保证“能枚举到候选，才能被细算命中”。

    let query_aabb = {

        if !floor_2d.enabled {

            panel_aabb

        } else {

            let z_thickness = panel_aabb.maxs.z - panel_aabb.mins.z;

            if z_thickness > floor_2d.z_thickness_max {

                panel_aabb

            } else {

                let x_extent = (panel_aabb.maxs.x - panel_aabb.mins.x).abs();

                let y_extent = (panel_aabb.maxs.y - panel_aabb.mins.y).abs();

                let extrude_height = floor_2d.extrude_height

                    .map(|v| v as Real)

                    .unwrap_or_else(|| x_extent.max(y_extent).max(1.0));



                let mut aabb = panel_aabb;

                aabb.mins.z -= inside_tol as Real;

                aabb.maxs.z += extrude_height;

                aabb

            }

        }

    };



    let candidates = tokio::task::spawn_blocking({

        let panel_aabb = query_aabb;

        let exclude_set = exclude_set;

        let panel_refno = panel_refno.clone();

        let candidate_limit = candidate_limit;



        move || -> anyhow::Result<Vec<RefnoEnum>> {

            // 使用 SQLite RTree 空间索引进行粗算：output/spatial_index.sqlite

            //

            // 说明：当前房间计算的空间索引链路依赖外部文件，容易因环境不齐导致失败；

            // 为保证 CLI 可用性，这里优先使用本仓库提供的 SQLite 空间索引（import-spatial-index 生成）。

            let idx = SqliteSpatialIndex::with_default_path()?;

            let ids = idx.query_intersect(&panel_aabb)?;



            let mut refnos = Vec::new();

            for id in ids {

                let candidate = RefnoEnum::from(id);

                if !candidate.is_valid() || candidate == panel_refno {

                    continue;

                }

                if exclude_set.contains(&candidate.refno()) {

                    continue;

                }

                refnos.push(candidate);

                if let Some(limit) = candidate_limit {

                    if refnos.len() >= limit {

                        warn!(

                            "面板 {} 候选数达到上限 {} (SQLite RTree)，可能存在截断",

                            panel_refno, limit

                        );

                        break;

                    }

                }

            }

            Ok(refnos)

        }

    })

    .await??;



    let candidate_count = candidates.len();

    debug!(

        "🔍 粗算完成: 耗时 {:?}, 候选数 {}",

        coarse_start.elapsed(),

        candidate_count

    );



    if debug_enabled {

        println!("[room_calc] panel={} candidates={}", panel_refno, candidate_count);

        for (i, r) in candidates.iter().take(10).enumerate() {

            println!("[room_calc] candidate[{}]={}", i, r);

        }

        if candidate_count > 10 {

            println!("[room_calc] ... candidates remaining={}", candidate_count - 10);

        }

    }



    // 步骤 4：粗算判定 — 候选 AABB 27 关键点投票 >50% 在 panel mesh 内
    // 候选 AABB 优先从 inst_relate_aabb 获取，缺失则跳过。

    let coarse_start = Instant::now();

    let mut candidate_aabb_map = match query_aabb_from_inst_relate_aabb(&candidates).await {
        Ok(m) => m,
        Err(e) => {
            warn!("批量查询候选构件 AABB (inst_relate_aabb) 失败: error={}", e);
            HashMap::new()
        }
    };

    #[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
    {
        if let Ok(idx) = SqliteSpatialIndex::with_default_path() {
            for candidate_refno in &candidates {
                if candidate_aabb_map.contains_key(candidate_refno) {
                    continue;
                }
                if let Ok(Some(aabb)) = idx.get_aabb(candidate_refno.refno()) {
                    candidate_aabb_map.insert(*candidate_refno, aabb);
                }
            }
        }
    }

    let missing_candidates: Vec<RefnoEnum> = candidates
        .iter()
        .copied()
        .filter(|r| !candidate_aabb_map.contains_key(r))
        .collect();
    if !missing_candidates.is_empty() {
        warn!(
            "房间计算候选构件 inst_relate_aabb 缺失: panel={}, missing_count={}",
            panel_refno,
            missing_candidates.len()
        );
        let _ = append_room_calc_missing_refnos(panel_refno, "candidate_aabb_inst_relate_missing", &missing_candidates);
    }

    let mut within_refnos = HashSet::<RefnoEnum>::new();
    for candidate_refno in &candidates {
        let Some(cand_aabb) = candidate_aabb_map.get(candidate_refno) else {
            continue;
        };
        let key_points = extract_aabb_key_points(cand_aabb);
        if is_geom_in_panel(&key_points, &panel_meshes, inside_tol, &floor_2d) {
            within_refnos.insert(*candidate_refno);
        }
    }



    debug!(

        "🧱 粗算完成: 耗时 {:?}, 结果数 {}",

        coarse_start.elapsed(),

        within_refnos.len()

    );



    info!(

        "面板 {} 房间计算完成: 总耗时 {:?}, 候选 {} -> 命中 {}",

        panel_refno,

        start_time.elapsed(),

        candidate_count,

        within_refnos.len()

    );



    if debug_enabled {

        println!(

            "[room_calc] panel={} within_refnos={} total_time_ms={}",

            panel_refno,

            within_refnos.len(),

            start_time.elapsed().as_millis()

        );

    }



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



    // mesh_dir 可能是基础目录（assets/meshes）或 LOD 子目录（assets/meshes/lod_L1）。

    // 这里统一溯源到不含 lod_ 的基础目录，避免拼错路径（例如误用 assets/lod_L1）。

    let mut base_mesh_dir = mesh_dir.clone();

    while let Some(last) = base_mesh_dir.file_name().and_then(|n| n.to_str()) {

        if last.starts_with("lod_") {

            base_mesh_dir.pop();

        } else {

            break;

        }

    }



    // 尝试的 LOD 级别顺序：L0 -> L1 -> L2 -> L3

    let lod_levels = ["L0", "L1", "L2", "L3"];



    for lod_level in lod_levels.iter() {

        let cache_key = format!("{}_{}", geo_hash, lod_level);



        // 1. 检查 TriMesh 缓存 (用于 GLB/GLTF 直接加载的结果)

        if let Some(cached_trimesh) = trimesh_cache.get(&cache_key) {

             // 这里的 cache 存储的是原始几何体的 TriMesh

             // 我们需要应用实例变换

             let transformed_mesh = transform_tri_mesh(&cached_trimesh, (world_trans * inst.geo_transform).to_matrix());

             CACHE_METRICS.record_trimesh_hit();

             return Ok(Arc::new(transformed_mesh));

        }



        // 2. 检查 PlantMesh 缓存

        if let Some(cached_mesh) = cache.get(&cache_key) {

            // 从缓存的 PlantMesh 构建 TriMesh

            if let Some(tri_mesh) = cached_mesh.get_tri_mesh_with_flag(

                (world_trans * inst.geo_transform).to_matrix(),

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

                         let transformed_mesh = transform_tri_mesh(&trimesh_arc, (world_trans * inst.geo_transform).to_matrix());

                         return Ok(Arc::new(transformed_mesh));

                     }

                     Ok(Err(e)) => {

                         warn!("加载 GLB 失败: path={:?}, error={}", glb_path, e);

                     }

                     _ => {}

                }

            }

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



fn aabb_contains_aabb_with_tol(panel: &Aabb, cand: &Aabb, tol: f32) -> bool {

    let tol = if tol.is_finite() && tol > 0.0 { tol as Real } else { 0.0 };



    cand.mins.x >= panel.mins.x - tol

        && cand.mins.y >= panel.mins.y - tol

        && cand.mins.z >= panel.mins.z - tol

        && cand.maxs.x <= panel.maxs.x + tol

        && cand.maxs.y <= panel.maxs.y + tol

        && cand.maxs.z <= panel.maxs.z + tol

}



/// 从 AABB 提取 8 个角点（与 extract_aabb_key_points 的 corner 顺序保持一致）。

fn extract_aabb_corners(aabb: &Aabb) -> [Point<Real>; 8] {

    let min = aabb.mins;

    let max = aabb.maxs;

    [

        Point::new(min.x, min.y, min.z),

        Point::new(max.x, min.y, min.z),

        Point::new(max.x, max.y, min.z),

        Point::new(min.x, max.y, min.z),

        Point::new(min.x, min.y, max.z),

        Point::new(max.x, min.y, max.z),

        Point::new(max.x, max.y, max.z),

        Point::new(min.x, max.y, max.z),

    ]

}



/// 判断一组点是否“全部”在面板 TriMesh 内（容差内）。

fn are_all_points_in_panel(

    points: &[Point<Real>],

    panel_meshes: &[Arc<TriMesh>],

    tolerance: f32,

    floor_2d: &Floor2dConfig,

) -> bool {

    if points.is_empty() || panel_meshes.is_empty() {

        return false;

    }



    let tolerance_sq = (tolerance as Real).powi(2);

    points

        .iter()

        .all(|point| is_point_inside_any_mesh(point, panel_meshes, tolerance_sq, floor_2d))

}



/// 从 AABB 提取 27 个关键点：8 顶点 + 1 中心 + 6 面中心 + 12 边中点。

fn extract_aabb_key_points(aabb: &Aabb) -> Vec<Point<Real>> {

    let min = aabb.mins;

    let max = aabb.maxs;

    let cx = (min.x + max.x) * 0.5;

    let cy = (min.y + max.y) * 0.5;

    let cz = (min.z + max.z) * 0.5;



    let mut pts = Vec::with_capacity(27);



    // 8 corners

    pts.push(Point::new(min.x, min.y, min.z));

    pts.push(Point::new(max.x, min.y, min.z));

    pts.push(Point::new(max.x, max.y, min.z));

    pts.push(Point::new(min.x, max.y, min.z));

    pts.push(Point::new(min.x, min.y, max.z));

    pts.push(Point::new(max.x, min.y, max.z));

    pts.push(Point::new(max.x, max.y, max.z));

    pts.push(Point::new(min.x, max.y, max.z));



    // center

    pts.push(Point::new(cx, cy, cz));



    // 6 face centers

    pts.push(Point::new(cx, cy, min.z));

    pts.push(Point::new(cx, cy, max.z));

    pts.push(Point::new(cx, min.y, cz));

    pts.push(Point::new(cx, max.y, cz));

    pts.push(Point::new(min.x, cy, cz));

    pts.push(Point::new(max.x, cy, cz));



    // 12 edge midpoints

    // edges along X

    pts.push(Point::new(cx, min.y, min.z));

    pts.push(Point::new(cx, max.y, min.z));

    pts.push(Point::new(cx, min.y, max.z));

    pts.push(Point::new(cx, max.y, max.z));

    // edges along Y

    pts.push(Point::new(min.x, cy, min.z));

    pts.push(Point::new(max.x, cy, min.z));

    pts.push(Point::new(min.x, cy, max.z));

    pts.push(Point::new(max.x, cy, max.z));

    // edges along Z

    pts.push(Point::new(min.x, min.y, cz));

    pts.push(Point::new(min.x, max.y, cz));

    pts.push(Point::new(max.x, min.y, cz));

    pts.push(Point::new(max.x, max.y, cz));



    debug_assert_eq!(pts.len(), 27);

    pts

}



/// 从 TriMesh 顶点采样关键点

///

/// 判断关键点是否在面板 TriMesh 内

/// 使用投票策略：超过 50% 的关键点在面板内即判定为属于该房间

fn is_geom_in_panel(

    key_points: &[Point<Real>],

    panel_meshes: &[Arc<TriMesh>],

    tolerance: f32,

    floor_2d: &Floor2dConfig,

) -> bool {

    if key_points.is_empty() || panel_meshes.is_empty() {

        return false;

    }



    let mut points_inside = 0;

    let total_points = key_points.len();

    let tolerance_sq = (tolerance as Real).powi(2);

    let threshold = total_points / 2 + 1;



    for (idx, point) in key_points.iter().enumerate() {

        if is_point_inside_any_mesh(point, panel_meshes, tolerance_sq, floor_2d) {

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

    floor_2d: &Floor2dConfig,

) -> bool {

    let tolerance = tolerance_sq.sqrt();



    for mesh in panel_meshes {

        // 使用射线投射法判断点是否在网格内部

        // parry3d 的 is_inside 对于某些封闭网格不可靠，射线投射法更准确

        if is_point_inside_mesh_raycast(point, mesh) {

            return true;

        }



        // 回退到距离检测：如果点非常接近表面，也认为在内部

        let projection = mesh.project_local_point(point, true);

        let distance_sq = (projection.point - point).norm_squared();

        if distance_sq <= tolerance_sq {

            return true;

        }



        // 兜底：当 panel TriMesh 是“薄片”(例如地板式面板，Z 方向厚度很小)时，

        // 3D 射线法通常会失败（网格非闭合）；此时改用 XY 投影的 2D 三角面覆盖测试，

        // 并沿 +Z 方向做有限外延，近似“地板区域 → 房间水平投影”语义。

        if floor_2d.enabled && is_point_inside_floor_panel_2d(point, mesh, tolerance, floor_2d) {

            return true;

        }

    }



    false

}



fn is_point_inside_floor_panel_2d(point: &Point<Real>, tri_mesh: &TriMesh, tolerance: Real, floor_2d: &Floor2dConfig) -> bool {

    let aabb = tri_mesh.local_aabb();

    let z_thickness = aabb.maxs.z - aabb.mins.z;

    if z_thickness > floor_2d.z_thickness_max {

        return false;

    }



    // 沿 +Z 外延的默认高度：若未显式配置，则用面板的 XY 尺度做自适应（单位自洽）。

    let extrude_height = floor_2d.extrude_height

        .map(|v| v as Real)

        .unwrap_or_else(|| {

            let x_extent = (aabb.maxs.x - aabb.mins.x).abs();

            let y_extent = (aabb.maxs.y - aabb.mins.y).abs();

            x_extent.max(y_extent).max(1.0)

        });



    // 允许略低于地板（tolerance），并允许在地板上方一定高度内算“在房间内”。

    if point.z < aabb.mins.z - tolerance {

        return false;

    }

    if point.z > aabb.maxs.z + extrude_height {

        return false;

    }



    is_point_in_trimesh_xy(point, tri_mesh, tolerance)

}



fn is_point_in_trimesh_xy(point: &Point<Real>, tri_mesh: &TriMesh, tolerance: Real) -> bool {

    let px = point.x;

    let py = point.y;



    let verts = tri_mesh.vertices();

    let indices = tri_mesh.indices();



    // 2D 容差：随 inside_tol 缩放，避免大坐标下过严。

    let eps = (tolerance.abs() * 1e-3).max(1e-6);



    for idx in indices {

        let ia = idx[0] as usize;

        let ib = idx[1] as usize;

        let ic = idx[2] as usize;

        if ia >= verts.len() || ib >= verts.len() || ic >= verts.len() {

            continue;

        }



        let a = &verts[ia];

        let b = &verts[ib];

        let c = &verts[ic];



        let minx = a.x.min(b.x).min(c.x) - tolerance;

        let maxx = a.x.max(b.x).max(c.x) + tolerance;

        let miny = a.y.min(b.y).min(c.y) - tolerance;

        let maxy = a.y.max(b.y).max(c.y) + tolerance;

        if px < minx || px > maxx || py < miny || py > maxy {

            continue;

        }



        // barycentric in XY

        let v0x = c.x - a.x;

        let v0y = c.y - a.y;

        let v1x = b.x - a.x;

        let v1y = b.y - a.y;

        let v2x = px - a.x;

        let v2y = py - a.y;



        let dot00 = v0x * v0x + v0y * v0y;

        let dot01 = v0x * v1x + v0y * v1y;

        let dot02 = v0x * v2x + v0y * v2y;

        let dot11 = v1x * v1x + v1y * v1y;

        let dot12 = v1x * v2x + v1y * v2y;



        let denom = dot00 * dot11 - dot01 * dot01;

        if denom.abs() <= eps {

            continue;

        }

        let inv = 1.0 / denom;

        let u = (dot11 * dot02 - dot01 * dot12) * inv;

        let v = (dot00 * dot12 - dot01 * dot02) * inv;



        if u >= -eps && v >= -eps && (u + v) <= 1.0 + eps {

            return true;

        }

    }



    false

}



/// 判断点是否在封闭网格内部

///

/// 使用 Möller–Trumbore 射线-三角形相交 + 奇偶计数法（ray-crossing parity），

/// 对凸形和凹形网格均正确。射线方向采用微偏轴以减少恰好穿过边/顶点的退化情况。

fn is_point_inside_mesh_raycast(point: &Point<Real>, tri_mesh: &TriMesh) -> bool {

    // 微偏轴方向：避免恰好与面对齐导致退化

    let direction = Vector::new(1.0, 0.31415926, 0.27182818);



    let vertices = tri_mesh.vertices();

    let indices = tri_mesh.indices();

    let mut crossings = 0u32;



    for tri in indices {

        let ia = tri[0] as usize;

        let ib = tri[1] as usize;

        let ic = tri[2] as usize;

        if ia >= vertices.len() || ib >= vertices.len() || ic >= vertices.len() {

            continue;

        }



        let a = &vertices[ia];

        let edge1 = vertices[ib] - a;

        let edge2 = vertices[ic] - a;



        let h = direction.cross(&edge2);

        let det = edge1.dot(&h);

        if det.abs() < 1e-10 {

            continue; // 射线与三角面平行

        }



        let inv_det = 1.0 / det;

        let s = point - a;

        let u = inv_det * s.dot(&h);

        if u < 0.0 || u > 1.0 {

            continue;

        }



        let q = s.cross(&edge1);

        let v = inv_det * direction.dot(&q);

        if v < 0.0 || u + v > 1.0 {

            continue;

        }



        let t = inv_det * edge2.dot(&q);

        if t > 1e-10 {

            crossings += 1;

        }

    }



    crossings % 2 == 1

}



/// 改进版本的房间关系保存

pub async fn save_room_relate(

    panel_refno: RefnoEnum,

    within_refnos: &HashSet<RefnoEnum>,

    room_num: &str,

) -> anyhow::Result<()> {

    if within_refnos.is_empty() {

        return Ok(());

    }



    let room_num_escaped = room_num.replace('\'', "''");

    let mut sql_statements = Vec::new();



    for refno in within_refnos {

        let relation_id = format!("{}_{}", panel_refno, refno);

        let sql = format!(

            "relate {}->room_relate:{}->{}  set room_num='{}', confidence=0.9, created_at=time::now();",

            panel_refno.to_pe_key(),

            relation_id,

            refno.to_pe_key(),

            room_num_escaped.as_str()

        );

        sql_statements.push(sql);

    }



    // 批量执行

    let batch_sql = sql_statements.join("\n");

    model_primary_db().query(&batch_sql).await?;



    debug!(

        "保存房间关系: panel={}, components={}",

        panel_refno,

        within_refnos.len()

    );

    Ok(())

}





/// 估算内存使用量

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

async fn estimate_memory_usage() -> f32 {

    let cache = get_enhanced_geometry_cache().await;

    let cache_size = cache.len() as f32 * 0.5; // 假设每个缓存项平均 0.5MB

    cache_size

}



#[cfg(not(all(not(target_arch = "wasm32"), feature = "sqlite-index")))]

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



    fn test_floor_2d_config() -> Floor2dConfig {

        Floor2dConfig { enabled: true, z_thickness_max: 0.2, extrude_height: None }

    }



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



    #[test]

    fn test_spatial_index_scope_from_filters() {

        assert_eq!(SpatialIndexScope::from_filters(None, None), SpatialIndexScope::Full);

        assert_eq!(

            SpatialIndexScope::from_filters(Some(&[1112, 1113]), None),

            SpatialIndexScope::Scoped

        );

        assert_eq!(

            SpatialIndexScope::from_filters(None, Some(RefnoEnum::from("1112/1"))),

            SpatialIndexScope::Scoped

        );

    }



    #[test]

    fn test_build_delete_room_panel_relations_sql_targets_rooms() {
        let room1 = RefnoEnum::from("1112/1");
        let room2 = RefnoEnum::from("1112/2");

        let sql = build_delete_room_panel_relations_sql(&[room1, room2])

        .expect("sql");

        assert!(sql.contains("LET $ids = SELECT VALUE id FROM room_panel_relate"));

        assert!(sql.contains(&format!("out IN [{},{}]", room1.to_pe_key(), room2.to_pe_key())));

        assert!(sql.contains("DELETE $ids;"));

    }



    #[test]

    fn test_build_delete_room_relations_sql_for_panels_targets_panels() {
        let panel1 = RefnoEnum::from("1112/10");
        let panel2 = RefnoEnum::from("1112/11");

        let sql = build_delete_room_relations_sql_for_panels(&[panel1, panel2])

        .expect("sql");

        assert!(sql.contains(&format!(
            "LET $ids = SELECT VALUE id FROM [{},{}]->room_relate;",
            panel1.to_pe_key(),
            panel2.to_pe_key()
        )));

        assert!(sql.contains("DELETE $ids;"));

    }



    #[test]

    fn test_build_room_panel_relations_sql_escapes_room_num() {
        let room_refno = RefnoEnum::from("1112/1");
        let panel1 = RefnoEnum::from("1112/10");
        let panel2 = RefnoEnum::from("1112/11");

        let sql = build_room_panel_relations_sql(&[(

            room_refno,

            "R'M-01".to_string(),

            vec![panel1, panel2],

        )]);

        assert!(sql.contains(&format!(
            "relate {}->room_panel_relate->[{},{}]",
            room_refno.to_pe_key(),
            panel1.to_pe_key(),
            panel2.to_pe_key()
        )));

        assert!(sql.contains("room_num='R''M-01'"));

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



    fn assert_close(a: Real, b: Real, eps: Real) {

        assert!((a - b).abs() <= eps, "assert_close failed: a={} b={} eps={}", a, b, eps);

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



        let result = is_geom_in_panel(&key_points, &panel_meshes, 0.1, &test_floor_2d_config());

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

        let result = is_geom_in_panel(&key_points, &panel_meshes, 0.1, &test_floor_2d_config());

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



        let result = is_geom_in_panel(&surface_points, &panel_meshes, 0.1, &test_floor_2d_config());

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

        let result_large_tolerance = is_geom_in_panel(&near_surface_points, &panel_meshes, 0.1, &test_floor_2d_config());

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

        let result = is_geom_in_panel(&far_points, &panel_meshes, 1.0, &test_floor_2d_config());

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



        let result = is_geom_in_panel(&mixed_points, &panel_meshes, 0.1, &test_floor_2d_config());

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

        let result = is_geom_in_panel(&mostly_far_points, &panel_meshes, 0.1, &test_floor_2d_config());

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

    fn test_aabb_contains_aabb_with_tol() {

        let panel = Aabb::new(Point::new(0.0, 0.0, 0.0), Point::new(10.0, 10.0, 10.0));



        // 严格包含：true

        let inside = Aabb::new(Point::new(1.0, 1.0, 1.0), Point::new(9.0, 9.0, 9.0));

        assert!(aabb_contains_aabb_with_tol(&panel, &inside, 0.0));



        // 边界略超出：无 tol -> false；有 tol -> true

        let slight_out = Aabb::new(Point::new(1.0, 1.0, 1.0), Point::new(10.05, 9.0, 9.0));

        assert!(!aabb_contains_aabb_with_tol(&panel, &slight_out, 0.0));

        assert!(aabb_contains_aabb_with_tol(&panel, &slight_out, 0.1));



        // 明显超出：有 tol 也应 false

        let far_out = Aabb::new(Point::new(1.0, 1.0, 1.0), Point::new(10.2, 9.0, 9.0));

        assert!(!aabb_contains_aabb_with_tol(&panel, &far_out, 0.1));



        // 非法 tol（负数/NaN）按 0 处理：不应误判为 true

        assert!(!aabb_contains_aabb_with_tol(&panel, &slight_out, -1.0));

        assert!(!aabb_contains_aabb_with_tol(&panel, &slight_out, f32::NAN));

    }



    #[test]

    fn test_extract_aabb_key_points_count_and_basic_positions() {

        let aabb = Aabb::new(Point::new(0.0, 0.0, 0.0), Point::new(10.0, 20.0, 30.0));

        let pts = extract_aabb_key_points(&aabb);

        assert_eq!(pts.len(), 27);



        // corners

        assert!(pts.contains(&Point::new(0.0, 0.0, 0.0)));

        assert!(pts.contains(&Point::new(10.0, 20.0, 30.0)));



        // center

        assert!(pts.contains(&Point::new(5.0, 10.0, 15.0)));



        // a face center sample

        assert!(pts.contains(&Point::new(5.0, 10.0, 0.0)));

        assert!(pts.contains(&Point::new(5.0, 0.0, 15.0)));

        assert!(pts.contains(&Point::new(0.0, 10.0, 15.0)));

    }



    #[test]

    fn test_extract_aabb_corners_count_and_positions() {

        let aabb = Aabb::new(Point::new(0.0, 0.0, 0.0), Point::new(10.0, 20.0, 30.0));

        let corners = extract_aabb_corners(&aabb);

        assert_eq!(corners.len(), 8);



        let corners_vec = corners.to_vec();

        assert!(corners_vec.contains(&Point::new(0.0, 0.0, 0.0)));

        assert!(corners_vec.contains(&Point::new(10.0, 20.0, 30.0)));

        assert!(corners_vec.contains(&Point::new(10.0, 0.0, 0.0)));

        assert!(corners_vec.contains(&Point::new(0.0, 20.0, 30.0)));

    }



    #[test]

    fn test_are_all_points_in_panel_all_inside() {

        let panel_meshes = vec![Arc::new(create_test_cube_trimesh(

            Point::new(0.0, 0.0, 0.0),

            Point::new(100.0, 100.0, 100.0),

        ))];



        let cand = Aabb::new(Point::new(10.0, 10.0, 10.0), Point::new(20.0, 20.0, 20.0));

        let corners = extract_aabb_corners(&cand);

        assert!(are_all_points_in_panel(&corners, &panel_meshes, 0.1, &test_floor_2d_config()));

    }



    #[test]

    fn test_are_all_points_in_panel_has_outside_corner() {

        let panel_meshes = vec![Arc::new(create_test_cube_trimesh(

            Point::new(0.0, 0.0, 0.0),

            Point::new(100.0, 100.0, 100.0),

        ))];



        let cand = Aabb::new(Point::new(-1.0, 10.0, 10.0), Point::new(20.0, 20.0, 20.0));

        let corners = extract_aabb_corners(&cand);

        assert!(!are_all_points_in_panel(&corners, &panel_meshes, 0.1, &test_floor_2d_config()));

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

            failed_panels: 0,

            missing_candidates: 0,

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

    // 测试套件 10: 边界条件和异常情况测试

    // ============================================================================



    /// 测试单个表面点应该通过

    #[test]

    fn test_is_geom_in_panel_single_surface_point() {

        let panel_meshes = vec![Arc::new(create_test_cube_trimesh(

            Point::new(0.0, 0.0, 0.0),

            Point::new(100.0, 100.0, 100.0),

        ))];



        // 单个表面上的点（距离为0，在容差内）

        let key_points = vec![Point::new(0.0, 50.0, 50.0)];



        let result = is_geom_in_panel(&key_points, &panel_meshes, 0.1, &test_floor_2d_config());

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



        let result = is_geom_in_panel(&key_points, &panel_meshes, 0.1, &test_floor_2d_config());

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



        let result = is_geom_in_panel(&key_points, &panel_meshes, 0.1, &test_floor_2d_config());

        assert!(!result, "两个远点不应该通过（0 >= 2 是 false）");

    }



    // ============================================================================

    // 测试套件: 射线投射法 (is_point_inside_mesh_raycast)

    // ============================================================================



    #[test]

    fn test_raycast_point_inside_closed_box() {

        let mesh = create_test_cube_trimesh(

            Point::new(0.0, 0.0, 0.0),

            Point::new(10.0, 10.0, 10.0),

        );

        // 中心点 → 应在内部

        assert!(is_point_inside_mesh_raycast(&Point::new(5.0, 5.0, 5.0), &mesh));

        // 偏移但仍在内部

        assert!(is_point_inside_mesh_raycast(&Point::new(1.0, 1.0, 1.0), &mesh));

        assert!(is_point_inside_mesh_raycast(&Point::new(9.0, 9.0, 9.0), &mesh));

    }



    #[test]

    fn test_raycast_point_outside_closed_box() {

        let mesh = create_test_cube_trimesh(

            Point::new(0.0, 0.0, 0.0),

            Point::new(10.0, 10.0, 10.0),

        );

        // 明显在外部的点

        assert!(!is_point_inside_mesh_raycast(&Point::new(20.0, 5.0, 5.0), &mesh));

        assert!(!is_point_inside_mesh_raycast(&Point::new(-5.0, 5.0, 5.0), &mesh));

        assert!(!is_point_inside_mesh_raycast(&Point::new(5.0, 5.0, 20.0), &mesh));

    }



    // ============================================================================

    // 测试套件: 距离回退 (is_point_inside_any_mesh 方法B)

    // ============================================================================



    #[test]

    fn test_distance_fallback_near_surface() {

        let mesh = Arc::new(create_test_cube_trimesh(

            Point::new(0.0, 0.0, 0.0),

            Point::new(10.0, 10.0, 10.0),

        ));

        let panel_meshes = vec![mesh];

        let floor_2d = test_floor_2d_config();

        let tol = 0.1_f32;

        let tol_sq = (tol as Real).powi(2);



        // 点紧贴表面外侧（距离 < tolerance）→ 应通过距离回退

        let near_point = Point::new(10.05, 5.0, 5.0);

        assert!(is_point_inside_any_mesh(&near_point, &panel_meshes, tol_sq, &floor_2d));

    }



    #[test]

    fn test_distance_fallback_far_from_surface() {

        let mesh = Arc::new(create_test_cube_trimesh(

            Point::new(0.0, 0.0, 0.0),

            Point::new(10.0, 10.0, 10.0),

        ));

        let panel_meshes = vec![mesh];

        let floor_2d = test_floor_2d_config();

        let tol = 0.1_f32;

        let tol_sq = (tol as Real).powi(2);



        // 点远离表面（距离 >> tolerance）→ 不应通过

        let far_point = Point::new(20.0, 5.0, 5.0);

        assert!(!is_point_inside_any_mesh(&far_point, &panel_meshes, tol_sq, &floor_2d));

    }



    // ============================================================================

    // 测试套件: 地板 2D 回退 (is_point_inside_floor_panel_2d)

    // ============================================================================



    /// 创建一个 Z 方向极薄的面板（模拟地板）

    fn create_thin_floor_trimesh(x_min: f32, x_max: f32, y_min: f32, y_max: f32, z: f32) -> TriMesh {

        // Z 方向厚度 0.01，远小于 0.2 阈值

        create_test_cube_trimesh(

            Point::new(x_min, y_min, z),

            Point::new(x_max, y_max, z + 0.01),

        )

    }



    #[test]

    fn test_floor_2d_thin_panel_point_above() {

        let mesh = create_thin_floor_trimesh(0.0, 10.0, 0.0, 10.0, 0.0);

        let floor_2d = test_floor_2d_config();

        let tol = 0.1;



        // XY 在面板内，Z 在面板上方（外延范围内）→ 应通过

        let point = Point::new(5.0, 5.0, 3.0);

        assert!(is_point_inside_floor_panel_2d(&point, &mesh, tol, &floor_2d));

    }



    #[test]

    fn test_floor_2d_thin_panel_point_outside_xy() {

        let mesh = create_thin_floor_trimesh(0.0, 10.0, 0.0, 10.0, 0.0);

        let floor_2d = test_floor_2d_config();

        let tol = 0.1;



        // XY 在面板外 → 不应通过

        let point = Point::new(20.0, 5.0, 3.0);

        assert!(!is_point_inside_floor_panel_2d(&point, &mesh, tol, &floor_2d));

    }



    #[test]

    fn test_floor_2d_thick_panel_skipped() {

        // Z 厚度 > 0.2 的厚面板 → 不走 2D 回退

        let mesh = create_test_cube_trimesh(

            Point::new(0.0, 0.0, 0.0),

            Point::new(10.0, 10.0, 5.0), // Z 厚度 = 5.0 >> 0.2

        );

        let floor_2d = test_floor_2d_config();

        let tol = 0.1;



        // 即使 XY 在面板内，厚面板也不走 2D 回退

        let point = Point::new(5.0, 5.0, 8.0);

        assert!(!is_point_inside_floor_panel_2d(&point, &mesh, tol, &floor_2d));

    }



    #[test]

    fn test_floor_2d_point_far_below() {

        let mesh = create_thin_floor_trimesh(0.0, 10.0, 0.0, 10.0, 0.0);

        let floor_2d = test_floor_2d_config();

        let tol = 0.1;



        // Z 远低于地板 → 不应通过

        let point = Point::new(5.0, 5.0, -5.0);

        assert!(!is_point_inside_floor_panel_2d(&point, &mesh, tol, &floor_2d));

    }



    // ============================================================================

    // 测试套件: 投票逻辑边界 (is_geom_in_panel 阈值)

    // ============================================================================



    #[test]

    fn test_voting_exact_threshold_27_points() {

        // 构造 27 个点，恰好 14 个在内 → 应通过（14 >= 14）

        let panel_meshes = vec![Arc::new(create_test_cube_trimesh(

            Point::new(0.0, 0.0, 0.0),

            Point::new(100.0, 100.0, 100.0),

        ))];

        let floor_2d = test_floor_2d_config();



        let mut points = Vec::with_capacity(27);

        // 14 个在内部

        for i in 0..14 {

            let v = 10.0 + i as f32 * 5.0;

            points.push(Point::new(v, v, v));

        }

        // 13 个在外部

        for i in 0..13 {

            let v = 200.0 + i as f32 * 10.0;

            points.push(Point::new(v, v, v));

        }

        assert_eq!(points.len(), 27);



        let result = is_geom_in_panel(&points, &panel_meshes, 0.1, &floor_2d);

        assert!(result, "恰好 14/27 在内应通过");

    }



    #[test]

    fn test_voting_below_threshold_27_points() {

        // 构造 27 个点，只有 13 个在内 → 不应通过（13 < 14）

        let panel_meshes = vec![Arc::new(create_test_cube_trimesh(

            Point::new(0.0, 0.0, 0.0),

            Point::new(100.0, 100.0, 100.0),

        ))];

        let floor_2d = test_floor_2d_config();



        let mut points = Vec::with_capacity(27);

        // 13 个在内部

        for i in 0..13 {

            let v = 10.0 + i as f32 * 5.0;

            points.push(Point::new(v, v, v));

        }

        // 14 个在外部

        for i in 0..14 {

            let v = 200.0 + i as f32 * 10.0;

            points.push(Point::new(v, v, v));

        }

        assert_eq!(points.len(), 27);



        let result = is_geom_in_panel(&points, &panel_meshes, 0.1, &floor_2d);

        assert!(!result, "只有 13/27 在内不应通过");

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

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

pub async fn update_room_relations_incremental(

    refnos: &[RefnoEnum],

) -> anyhow::Result<IncrementalUpdateResult> {

    update_room_relations_incremental_with_cancel(

        &aios_core::get_db_option(),

        None,

        None,

    )

    .await

    .map(|stats| IncrementalUpdateResult {

        affected_rooms: stats.total_rooms,

        updated_elements: stats.total_components,

        duration_ms: stats.build_time_ms,

    })

}



/// 支持取消和进度回调的房间关系增量更新

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

pub async fn update_room_relations_incremental_with_cancel(

    db_option: &DbOption,

    cancel_token: Option<CancellationToken>,

    progress_callback: Option<Box<dyn Fn(f32, &str) + Send + Sync>>,

) -> anyhow::Result<RoomBuildStats> {

    // 逻辑：增量更新实际上是找到受影响的房间并重新计算

    // 为了简单起见，这里重用重建逻辑，但只针对受影响的房间（如果能找到的话）

    // 或者直接调用 build_room_relations_with_cancel 作为一个安全的回退

    build_room_relations_with_cancel(db_option, None, None, cancel_token, progress_callback).await

}



#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

pub async fn rebuild_room_relations_for_rooms_with_cancel(

    room_numbers: Vec<String>,

    db_option: &DbOption,

    cancel_token: Option<CancellationToken>,

    progress_callback: Option<Box<dyn Fn(f32, &str) + Send + Sync>>,

) -> anyhow::Result<RoomBuildStats> {

    info!("开始重建房间关系 (指定房间，支持取消)");



    if let Some(ref cb) = progress_callback {

        cb(0.0, "开始重建房间关系");

    }



    let start_time = Instant::now();

    let mesh_dir = db_option.get_meshes_path();

    let room_key_words = db_option.get_room_key_word();

    let compute_options = RoomComputeOptions::default();



    #[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

    init_room_calc_config(db_option);

    #[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
    {
        if let Some(ref cb) = progress_callback {
            cb(0.02, "正在刷新 SQLite AABB 索引");
        }
        ensure_spatial_index_ready(None, None, true).await?;
    }



    // 1. 查询房间面板关系

    if let Some(ref cb) = progress_callback {

        cb(0.05, "查询所有房间面板映射关系");

    }

    let mut room_panel_map = build_room_panels_relate_for_query(&room_key_words).await?;



    // 2. 过滤指定房间

    let numbers_set: HashSet<String> = room_numbers.into_iter().collect();

    room_panel_map.retain(|(_, room_num, _)| numbers_set.contains(room_num));

    info!("过滤后处理 {} 个房间", room_panel_map.len());



    if room_panel_map.is_empty() {

        return Ok(RoomBuildStats {

            total_rooms: 0,

            total_panels: 0,

            total_components: 0,

            build_time_ms: 0,

            cache_hit_rate: 0.0,

            memory_usage_mb: 0.0,

            failed_panels: 0,

            missing_candidates: 0,

        });

    }



    if let Some(ref token) = cancel_token {

        if token.is_cancelled() {

            anyhow::bail!("任务在过滤后取消");

        }

    }



    let exclude_panel_refnos: HashSet<RefnoEnum> = room_panel_map

        .iter()

        .flat_map(|(_, _, panels)| panels.clone())

        .collect();



    #[cfg(all(

        not(target_arch = "wasm32"),

        feature = "sqlite-index",

        feature = "gen_model"

    ))]

    pregen_room_panels_into_model_cache(db_option, &room_panel_map).await?;

    let panels_to_delete: Vec<PanelRoom> = room_panel_map
        .iter()
        .flat_map(|(_, room_num, panels)| {
            panels.iter().map(move |panel| PanelRoom {
                panel: *panel,
                room_num: room_num.clone(),
            })
        })
        .collect();

    delete_room_relations_for_panels(&panels_to_delete).await?;
    sync_room_panel_relations(&room_panel_map, false).await?;



    let stats = compute_room_relations_with_cancel(

        &mesh_dir,

        room_panel_map,

        exclude_panel_refnos,

        compute_options,

        cancel_token,

        progress_callback,

    )

    .await?;



    info!("✅ 房间关系重建完成，耗时 {:?}", start_time.elapsed());

    Ok(stats)

}



#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

pub async fn update_room_relations_incremental_original(

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

    #[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

    init_room_calc_config(&db_option);

    #[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
    ensure_spatial_index_ready(None, None, true).await?;



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

        .map(|pr| {

            let mesh_dir = mesh_dir.clone();

            let exclude_panel_refnos = exclude_panel_refnos.clone();

            let options = compute_options;

            async move {

                process_panel_for_room(

                    &mesh_dir,

                    pr.panel,

                    &pr.room_num,

                    exclude_panel_refnos.as_ref(),

                    options,

                )

                .await

            }

        })

        .buffer_unordered(compute_options.concurrency.max(1))

        .collect::<Vec<_>>()

        .await;



    updated_elements = results.iter().map(|outcome| outcome.components).sum();



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



use surrealdb::types::{self as surrealdb_types, SurrealValue};



#[derive(Debug, serde::Deserialize, SurrealValue)]

struct PanelRoom {

    panel: RefnoEnum,

    room_num: String,

}



/// 查询包含指定 refnos 的房间面板

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

async fn query_panels_containing_refnos(

    refnos: &[RefnoEnum],

) -> anyhow::Result<Vec<PanelRoom>> {

    if refnos.is_empty() {

        return Ok(Vec::new());

    }



    // 构建查询条件

    let refno_keys: Vec<String> = refnos.iter().map(|r| r.to_pe_key()).collect();

    let refno_list = refno_keys.join(",");



    // 查询包含这些 refnos 的房间面板关系

    // 使用图遍历语法: refno <-room_relate 获取 in(panel) 和 room_num

    let sql = format!(

        r#"

        SELECT VALUE {{ panel: in, room_num: room_num }}

        FROM array::distinct([{}]<-room_relate)

        "#,

        refno_list

    );



    let mut response = model_primary_db().query(&sql).await?;

    let panels: Vec<PanelRoom> = response.take(0)?;



    Ok(panels)

}



/// 删除指定面板的房间关系

#[cfg(all(
    not(target_arch = "wasm32"),
    feature = "sqlite-index"
))]

async fn delete_all_room_relations() -> anyhow::Result<()> {
    model_primary_db()
        .query("DELETE room_relate;\nDELETE room_panel_relate;")
        .await?;
    Ok(())
}

#[cfg(all(
    not(target_arch = "wasm32"),
    feature = "sqlite-index"
))]

async fn delete_room_relations_for_panels(panels: &[PanelRoom]) -> anyhow::Result<()> {

    if panels.is_empty() {

        return Ok(());

    }



    let panel_refnos: Vec<RefnoEnum> = panels.iter().map(|p| p.panel).collect();

    if let Some(sql) = build_delete_room_relations_sql_for_panels(&panel_refnos) {
        model_primary_db().query(sql).await?;
    }

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

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

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

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

pub async fn rebuild_room_relations_for_rooms(

    room_numbers: Option<Vec<String>>,

    db_option: &DbOption,

) -> anyhow::Result<RoomBuildStats> {

    info!("开始重建房间关系");

    let start_time = Instant::now();



    let mesh_dir = db_option.get_meshes_path();

    let room_key_words = db_option.get_room_key_word();

    let compute_options = RoomComputeOptions::default();



    #[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]

    init_room_calc_config(db_option);



    // 1. 查询房间面板关系

    let mut room_panel_map = build_room_panels_relate_for_query(&room_key_words).await?;



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

            failed_panels: 0,

            missing_candidates: 0,

        });

    }



    let exclude_panel_refnos: HashSet<RefnoEnum> = room_panel_map

        .iter()

        .flat_map(|(_, _, panels)| panels.clone())

        .collect();


    CACHE_METRICS.reset();



    #[cfg(all(

        not(target_arch = "wasm32"),

        feature = "sqlite-index",

        feature = "gen_model"

    ))]

    pregen_room_panels_into_model_cache(db_option, &room_panel_map).await?;

    let panels_to_delete: Vec<PanelRoom> = room_panel_map
        .iter()
        .flat_map(|(_, room_num, panels)| {
            panels.iter().map(move |panel| PanelRoom {
                panel: *panel,
                room_num: room_num.clone(),
            })
        })
        .collect();

    delete_room_relations_for_panels(&panels_to_delete).await?;
    sync_room_panel_relations(&room_panel_map, false).await?;



    let stats = compute_room_relations(

        &mesh_dir,

        room_panel_map,

        exclude_panel_refnos,

        compute_options,

    )

    .await?;



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
