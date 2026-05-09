//! SQLite RTree 空间索引查询 API
//!
//! 目的：
//! - 为前端提供"周边模型尚未加载"时的粗筛能力（通过 output/spatial_index.sqlite）。
//! - 返回周边 refno 列表（以及 noun/aabb），便于前端按需加载后再做精确最近点计算。
//!
//! 约定：
//! - refno 使用字符串格式："dbnum_refno"（与前端/DTX 一致）。
//! - 失败时也尽量返回 HTTP 200 + {success:false, error:"..."}，避免前端 fetchJson 因非 2xx 直接抛错。
//!
//! ## Endpoints
//! - `GET /api/sqlite-spatial/query` - 按 refno 或 bbox 查询周边构件
//! - `GET /api/sqlite-spatial/stats` - 获取索引统计与健康信息

use aios_core::RefnoEnum;
use axum::{extract::Query, response::Json};
use parry3d::bounding_volume::{Aabb, BoundingVolume};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Mutex, OnceLock};

use crate::sqlite_index::{SqliteAabbIndex, i64_to_refno_str, refno_str_to_i64};

const DEFAULT_DISTANCE: f32 = 0.0;
const DEFAULT_MAX_HITS: usize = 5000;
const HARD_MAX_HITS: usize = 10_000;

// ============================================================================
// 全局惰性初始化索引（避免每次请求重新打开文件）
// ============================================================================

struct CachedIndex {
    idx: SqliteAabbIndex,
    path: PathBuf,
}
static TEST_INDEX_OVERRIDE: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
#[cfg(test)]
static TEST_GUARD: OnceLock<Mutex<()>> = OnceLock::new();

fn test_index_override() -> &'static Mutex<Option<PathBuf>> {
    TEST_INDEX_OVERRIDE.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
fn test_guard() -> &'static Mutex<()> {
    TEST_GUARD.get_or_init(|| Mutex::new(()))
}

fn get_cached_index() -> Result<&'static CachedIndex, String> {
    let path = sqlite_index_path();
    let idx =
        SqliteAabbIndex::open(&path).map_err(|e| format!("open sqlite index failed: {}", e))?;
    idx.init_schema()
        .map_err(|e| format!("init sqlite schema failed: {}", e))?;

    let cached = Box::leak(Box::new(CachedIndex { idx, path }));
    Ok(cached)
}

// ============================================================================
// 请求/响应结构体
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct SqliteSpatialQueryParams {
    /// bbox | refno | position
    pub mode: Option<String>,
    /// refno string like "17496_123456" (也兼容 "17496/123456")
    pub refno: Option<String>,
    /// position 模式：中心点坐标
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub z: Option<f32>,
    /// position 模式：查询半径（毫米）
    pub radius: Option<f32>,
    /// 额外扩张距离（毫米，默认 0）
    pub distance: Option<f32>,
    pub minx: Option<f32>,
    pub miny: Option<f32>,
    pub minz: Option<f32>,
    pub maxx: Option<f32>,
    pub maxy: Option<f32>,
    pub maxz: Option<f32>,
    /// 兼容旧参数：未传 per_page 时作为每页数量使用（默认 5000，硬上限 10000）
    pub max_results: Option<usize>,
    /// 分页页码，从 1 开始
    pub page: Option<usize>,
    /// 每页数量（硬上限 10000）
    pub per_page: Option<usize>,
    /// noun 过滤（逗号分隔，如 "EQUI,PIPE,TUBI"，空表示不过滤）
    pub nouns: Option<String>,
    /// 专业过滤（逗号分隔，如 "1,3"，空表示不过滤）
    pub spec_values: Option<String>,
    /// 是否包含自身（mode=refno 时有效，默认 true）
    pub include_self: Option<bool>,
    /// 查询形状："cube"（默认）| "sphere"（球体，会对结果做距离二次过滤）
    pub shape: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SpatialQueryResult {
    pub success: bool,
    pub results: Option<Vec<SpatialQueryResultItem>>,
    /// 是否还有更多结果；兼容旧字段名
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    /// 本次查询完整命中数量
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_count: Option<usize>,
    /// 当前页返回数量
    #[serde(skip_serializing_if = "Option::is_none")]
    pub returned_count: Option<usize>,
    /// 当前页码
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,
    /// 当前每页数量
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_page: Option<usize>,
    /// 是否还有下一页
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
    /// 实际查询使用的 AABB（便于调试）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_bbox: Option<AabbDto>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SpatialQueryResultItem {
    pub refno: String,
    pub noun: String,
    pub spec_value: i64,
    pub aabb: Option<AabbDto>,
    pub distance: Option<f32>,
}

#[derive(Debug, Serialize, Clone)]
pub struct AabbDto {
    pub min: Vec3Dto,
    pub max: Vec3Dto,
}

#[derive(Debug, Serialize, Clone)]
pub struct Vec3Dto {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// 索引统计响应
#[derive(Debug, Serialize)]
pub struct SpatialStatsResult {
    pub success: bool,
    pub total_elements: usize,
    pub index_type: String,
    pub index_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ============================================================================
// 辅助函数
// ============================================================================

fn sqlite_index_path() -> PathBuf {
    if let Some(path) = test_index_override()
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
    {
        return path;
    }
    // 兼容两个环境变量名
    for var in ["AIOS_SPATIAL_INDEX_SQLITE", "SQLITE_SPATIAL_INDEX_PATH"] {
        if let Ok(v) = std::env::var(var) {
            let v = v.trim().to_string();
            if !v.is_empty() {
                return PathBuf::from(v);
            }
        }
    }
    PathBuf::from("output").join("spatial_index.sqlite")
}

fn expand_aabb(mut aabb: Aabb, distance: f32) -> Aabb {
    if !(distance.is_finite()) || distance <= 0.0 {
        return aabb;
    }
    aabb.mins.x -= distance;
    aabb.mins.y -= distance;
    aabb.mins.z -= distance;
    aabb.maxs.x += distance;
    aabb.maxs.y += distance;
    aabb.maxs.z += distance;
    aabb
}

fn parse_mode(params: &SqliteSpatialQueryParams) -> &'static str {
    let mode = params.mode.as_deref().unwrap_or("").trim().to_lowercase();
    if mode == "refno" {
        return "refno";
    }
    if mode == "bbox" {
        return "bbox";
    }
    if mode == "position" {
        return "position";
    }
    // 未指定时：优先 position，其次 refno，最后 bbox
    if params.x.is_some() && params.y.is_some() && params.z.is_some() {
        return "position";
    }
    if params.refno.as_deref().unwrap_or("").trim().is_empty() {
        "bbox"
    } else {
        "refno"
    }
}

fn aabb_from_bbox_params(p: &SqliteSpatialQueryParams) -> Result<Aabb, String> {
    let minx = p.minx.ok_or_else(|| "missing minx".to_string())?;
    let miny = p.miny.ok_or_else(|| "missing miny".to_string())?;
    let minz = p.minz.ok_or_else(|| "missing minz".to_string())?;
    let maxx = p.maxx.ok_or_else(|| "missing maxx".to_string())?;
    let maxy = p.maxy.ok_or_else(|| "missing maxy".to_string())?;
    let maxz = p.maxz.ok_or_else(|| "missing maxz".to_string())?;
    if !(minx.is_finite()
        && miny.is_finite()
        && minz.is_finite()
        && maxx.is_finite()
        && maxy.is_finite()
        && maxz.is_finite())
    {
        return Err("bbox contains non-finite value".to_string());
    }
    if minx > maxx || miny > maxy || minz > maxz {
        return Err("bbox min > max".to_string());
    }
    Ok(Aabb::new(
        [minx, miny, minz].into(),
        [maxx, maxy, maxz].into(),
    ))
}

fn aabb_dto_from_row(minx: f32, miny: f32, minz: f32, maxx: f32, maxy: f32, maxz: f32) -> AabbDto {
    AabbDto {
        min: Vec3Dto {
            x: minx,
            y: miny,
            z: minz,
        },
        max: Vec3Dto {
            x: maxx,
            y: maxy,
            z: maxz,
        },
    }
}

fn aabb_to_dto(aabb: &Aabb) -> AabbDto {
    AabbDto {
        min: Vec3Dto {
            x: aabb.mins.x,
            y: aabb.mins.y,
            z: aabb.mins.z,
        },
        max: Vec3Dto {
            x: aabb.maxs.x,
            y: aabb.maxs.y,
            z: aabb.maxs.z,
        },
    }
}

/// 解析 noun 过滤参数为大写集合
fn parse_noun_filter(nouns: &Option<String>) -> Option<Vec<String>> {
    nouns.as_ref().and_then(|s| {
        let list: Vec<String> = s
            .split(',')
            .map(|n| n.trim().to_uppercase())
            .filter(|n| !n.is_empty())
            .collect();
        if list.is_empty() { None } else { Some(list) }
    })
}

fn parse_spec_value_filter(spec_values: &Option<String>) -> Option<Vec<i64>> {
    spec_values.as_ref().and_then(|s| {
        let list: Vec<i64> = s
            .split(',')
            .filter_map(|value| value.trim().parse::<i64>().ok())
            .collect();
        if list.is_empty() { None } else { Some(list) }
    })
}

fn error_spatial_query_result(
    error: impl Into<String>,
    query_bbox: Option<AabbDto>,
) -> SpatialQueryResult {
    SpatialQueryResult {
        success: false,
        results: None,
        truncated: None,
        total_count: None,
        returned_count: None,
        page: None,
        per_page: None,
        has_more: None,
        query_bbox,
        error: Some(error.into()),
    }
}

fn resolve_pagination(params: &SqliteSpatialQueryParams) -> (usize, usize) {
    let page = params.page.unwrap_or(1).max(1);
    let raw_per_page = params
        .per_page
        .or(params.max_results)
        .unwrap_or(DEFAULT_MAX_HITS);
    let per_page = raw_per_page.clamp(1, HARD_MAX_HITS);
    (page, per_page)
}

fn success_spatial_query_result(
    results: Vec<SpatialQueryResultItem>,
    total_count: usize,
    page: usize,
    per_page: usize,
    query_bbox: Option<AabbDto>,
) -> SpatialQueryResult {
    let returned_count = results.len();
    let end = page
        .saturating_sub(1)
        .saturating_mul(per_page)
        .saturating_add(returned_count);
    let has_more = end < total_count;

    SpatialQueryResult {
        success: true,
        results: Some(results),
        truncated: Some(has_more),
        total_count: Some(total_count),
        returned_count: Some(returned_count),
        page: Some(page),
        per_page: Some(per_page),
        has_more: Some(has_more),
        query_bbox,
        error: None,
    }
}

// ============================================================================
// Handler：GET /api/sqlite-spatial/query
// ============================================================================

/// GET /api/sqlite-spatial/query
pub async fn api_sqlite_spatial_query(
    Query(params): Query<SqliteSpatialQueryParams>,
) -> Json<SpatialQueryResult> {
    let fallback_refno_ids = match query_refno_visible_inst_ids_for_fallback(&params).await {
        Ok(ids) => ids,
        Err(e) => {
            return Json(error_spatial_query_result(e, None));
        }
    };

    // 将 SQLite 阻塞 I/O 放入 blocking 线程池
    let result =
        tokio::task::spawn_blocking(move || do_spatial_query(params, fallback_refno_ids)).await;
    match result {
        Ok(r) => Json(r),
        Err(e) => Json(error_spatial_query_result(
            format!("internal error: {}", e),
            None,
        )),
    }
}

async fn query_refno_visible_inst_ids_for_fallback(
    params: &SqliteSpatialQueryParams,
) -> Result<Option<Vec<i64>>, String> {
    if parse_mode(params) != "refno" {
        return Ok(None);
    }

    let refno = params.refno.as_deref().unwrap_or("").trim();
    let Some(id) = refno_str_to_i64(refno) else {
        return Ok(None);
    };

    let cached = get_cached_index()?;
    let conn = Connection::open(&cached.path)
        .map_err(|e| format!("open sqlite connection failed: {}", e))?;
    if query_aabb_row(&conn, id)
        .map_err(|e| format!("query refno aabb failed: {}", e))?
        .is_some()
    {
        return Ok(None);
    }

    let normalized = refno.replace('_', "/");
    let parsed_refno = RefnoEnum::from_str(&normalized)
        .map_err(|e| format!("invalid refno format (expected dbnum_refno): {}", e))?;
    let mut ids = crate::fast_model::query_compat::query_deep_visible_inst_refnos(parsed_refno)
        .await
        .map_err(|e| format!("query visible insts for refno fallback failed: {}", e))?
        .into_iter()
        .filter_map(|child| refno_str_to_i64(&child.to_string().replace('/', "_")))
        .collect::<Vec<_>>();

    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        Ok(None)
    } else {
        Ok(Some(ids))
    }
}

fn query_aabb_row(
    conn: &Connection,
    id: i64,
) -> rusqlite::Result<Option<(f32, f32, f32, f32, f32, f32)>> {
    conn.query_row(
        "SELECT min_x, min_y, min_z, max_x, max_y, max_z FROM aabb_index WHERE id = ?1",
        [id],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        },
    )
    .optional()
}

fn aabb_from_row(minx: f32, miny: f32, minz: f32, maxx: f32, maxy: f32, maxz: f32) -> Aabb {
    Aabb::new([minx, miny, minz].into(), [maxx, maxy, maxz].into())
}

fn query_aabbs_for_ids(conn: &Connection, ids: &[i64]) -> rusqlite::Result<Vec<Aabb>> {
    let mut out = Vec::new();
    for id in ids {
        let Some((minx, miny, minz, maxx, maxy, maxz)) = query_aabb_row(conn, *id)? else {
            continue;
        };
        out.push(aabb_from_row(minx, miny, minz, maxx, maxy, maxz));
    }
    Ok(out)
}

fn do_spatial_query(
    params: SqliteSpatialQueryParams,
    fallback_refno_ids: Option<Vec<i64>>,
) -> SpatialQueryResult {
    let cached = match get_cached_index() {
        Ok(c) => c,
        Err(e) => {
            return error_spatial_query_result(
                format!("{}. 请先运行 import-spatial-index 构建索引。", e),
                None,
            );
        }
    };

    let mode = parse_mode(&params);
    let include_self = params.include_self.unwrap_or(true);

    // 记住 refno 对应的 i64 id（用于 include_self 过滤）
    let self_id: Option<i64> = if mode == "refno" && !include_self {
        params
            .refno
            .as_deref()
            .and_then(|s| refno_str_to_i64(s.trim()))
    } else {
        None
    };

    let base_aabb = if mode == "position" {
        // position 模式：从 x, y, z, radius 构建 AABB
        const MAX_QUERY_RADIUS: f32 = 100_000.0; // 100m in mm

        let x = params.x.ok_or_else(|| "missing x".to_string());
        let y = params.y.ok_or_else(|| "missing y".to_string());
        let z = params.z.ok_or_else(|| "missing z".to_string());
        let radius = params.radius.ok_or_else(|| "missing radius".to_string());

        match (x, y, z, radius) {
            (Ok(x), Ok(y), Ok(z), Ok(r)) => {
                if !(x.is_finite()
                    && y.is_finite()
                    && z.is_finite()
                    && r.is_finite()
                    && r > 0.0
                    && r <= MAX_QUERY_RADIUS)
                {
                    return error_spatial_query_result(
                        format!(
                            "invalid position or radius (must be 0 < radius <= {} mm)",
                            MAX_QUERY_RADIUS
                        ),
                        None,
                    );
                }
                return query_by_target_aabbs(
                    params,
                    cached,
                    vec![Aabb::new([x, y, z].into(), [x, y, z].into())],
                    r,
                    self_id,
                );
            }
            _ => {
                return error_spatial_query_result(
                    "missing position parameters (x, y, z, radius)",
                    None,
                );
            }
        }
    } else if mode == "refno" {
        let refno = params.refno.as_deref().unwrap_or("").trim();
        if refno.is_empty() {
            return error_spatial_query_result("missing refno", None);
        }
        let Some(id) = refno_str_to_i64(refno) else {
            return error_spatial_query_result("invalid refno format (expected dbnum_refno)", None);
        };
        // 查询该 refno 的 bbox（使用独立连接避免长期占用）
        let conn = match Connection::open(&cached.path) {
            Ok(c) => c,
            Err(e) => {
                return error_spatial_query_result(
                    format!("open sqlite connection failed: {}", e),
                    None,
                );
            }
        };
        let row = query_aabb_row(&conn, id).unwrap_or(None);
        let Some((minx, miny, minz, maxx, maxy, maxz)) = row else {
            if let Some(ids) = fallback_refno_ids.as_deref() {
                match query_aabbs_for_ids(&conn, ids) {
                    Ok(aabbs) if !aabbs.is_empty() => {
                        let distance = normalized_distance(params.distance);
                        return query_by_target_aabbs(params, cached, aabbs, distance, self_id);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        return error_spatial_query_result(
                            format!("query fallback aabb failed: {}", e),
                            None,
                        );
                    }
                }
            }
            return empty_spatial_query_result(&params);
        };
        aabb_from_row(minx, miny, minz, maxx, maxy, maxz)
    } else {
        match aabb_from_bbox_params(&params) {
            Ok(v) => v,
            Err(e) => {
                return error_spatial_query_result(e, None);
            }
        }
    };

    let distance = normalized_distance(params.distance);
    query_by_target_aabbs(params, cached, vec![base_aabb], distance, self_id)
}

fn empty_spatial_query_result(params: &SqliteSpatialQueryParams) -> SpatialQueryResult {
    let (page, per_page) = resolve_pagination(params);
    SpatialQueryResult {
        success: true,
        results: Some(vec![]),
        truncated: Some(false),
        total_count: Some(0),
        returned_count: Some(0),
        page: Some(page),
        per_page: Some(per_page),
        has_more: Some(false),
        query_bbox: None,
        error: None,
    }
}

fn normalized_distance(distance: Option<f32>) -> f32 {
    let distance = distance.unwrap_or(DEFAULT_DISTANCE);
    if distance.is_finite() && distance > 0.0 {
        distance
    } else {
        DEFAULT_DISTANCE
    }
}

fn min_axis_gap(a_min: f32, a_max: f32, b_min: f32, b_max: f32) -> f32 {
    if a_max < b_min {
        b_min - a_max
    } else if b_max < a_min {
        a_min - b_max
    } else {
        0.0
    }
}

fn aabb_min_distance(a: &Aabb, b: &Aabb) -> f32 {
    let dx = min_axis_gap(a.mins.x, a.maxs.x, b.mins.x, b.maxs.x);
    let dy = min_axis_gap(a.mins.y, a.maxs.y, b.mins.y, b.maxs.y);
    let dz = min_axis_gap(a.mins.z, a.maxs.z, b.mins.z, b.maxs.z);
    (dx * dx + dy * dy + dz * dz).sqrt()
}

fn min_distance_to_targets(candidate: &Aabb, targets: &[Aabb]) -> f32 {
    targets
        .iter()
        .map(|target| aabb_min_distance(candidate, target))
        .fold(f32::INFINITY, f32::min)
}

fn refno_db_prefix(refno: Option<&str>) -> Option<String> {
    let normalized = refno?.trim().replace('/', "_");
    let dbnum = normalized.split('_').next()?.trim();
    if dbnum.is_empty() {
        None
    } else {
        Some(format!("{}_", dbnum))
    }
}

fn preferred_db_rank(item: &SpatialQueryResultItem, preferred_db_prefix: &Option<String>) -> u8 {
    match preferred_db_prefix {
        Some(prefix) if item.refno.starts_with(prefix) => 0,
        Some(_) => 1,
        None => 0,
    }
}

fn query_ids_for_regions(
    cached: &CachedIndex,
    target_aabbs: &[Aabb],
    distance: f32,
) -> Result<(Vec<i64>, Option<Aabb>), String> {
    let mut ids = HashSet::new();
    let mut query_union: Option<Aabb> = None;

    for target in target_aabbs {
        let query_aabb = expand_aabb((*target).clone(), distance);
        if let Some(current) = &mut query_union {
            current.merge(&query_aabb);
        } else {
            query_union = Some(query_aabb.clone());
        }

        let hits = cached
            .idx
            .query_intersect(
                query_aabb.mins.x as f64,
                query_aabb.maxs.x as f64,
                query_aabb.mins.y as f64,
                query_aabb.maxs.y as f64,
                query_aabb.mins.z as f64,
                query_aabb.maxs.z as f64,
            )
            .map_err(|e| format!("query_intersect failed: {}", e))?;
        ids.extend(hits);
    }

    let mut ids = ids.into_iter().collect::<Vec<_>>();
    ids.sort_unstable();
    Ok((ids, query_union))
}

fn query_by_target_aabbs(
    params: SqliteSpatialQueryParams,
    cached: &CachedIndex,
    target_aabbs: Vec<Aabb>,
    search_distance: f32,
    self_id: Option<i64>,
) -> SpatialQueryResult {
    let (page, per_page) = resolve_pagination(&params);
    let noun_filter = parse_noun_filter(&params.nouns);
    let spec_value_filter = parse_spec_value_filter(&params.spec_values);
    let preferred_db_prefix = refno_db_prefix(params.refno.as_deref());

    if target_aabbs.is_empty() {
        return success_spatial_query_result(vec![], 0, page, per_page, None);
    }

    // 球体模式：使用候选 AABB 到目标 AABB/点的最小距离做二次过滤。
    let is_sphere = params
        .shape
        .as_deref()
        .unwrap_or("cube")
        .eq_ignore_ascii_case("sphere");
    let (ids, query_aabb) = match query_ids_for_regions(cached, &target_aabbs, search_distance) {
        Ok(v) => v,
        Err(e) => {
            return error_spatial_query_result(e, None);
        }
    };
    let query_bbox_dto = query_aabb.as_ref().map(aabb_to_dto);

    // 打开连接获取 noun 和 aabb 信息（使用 prepared statements 批量查询）
    let conn = match Connection::open(&cached.path) {
        Ok(c) => c,
        Err(e) => {
            return error_spatial_query_result(
                format!("open sqlite connection failed: {}", e),
                query_bbox_dto.clone(),
            );
        }
    };

    let mut stmt_item = match conn.prepare("SELECT noun, spec_value FROM items WHERE id = ?1") {
        Ok(s) => s,
        Err(e) => {
            return error_spatial_query_result(
                format!("prepare item stmt failed: {}", e),
                query_bbox_dto.clone(),
            );
        }
    };
    let mut stmt_aabb = match conn
        .prepare("SELECT min_x, min_y, min_z, max_x, max_y, max_z FROM aabb_index WHERE id = ?1")
    {
        Ok(s) => s,
        Err(e) => {
            return error_spatial_query_result(
                format!("prepare aabb stmt failed: {}", e),
                query_bbox_dto.clone(),
            );
        }
    };

    let mut results: Vec<SpatialQueryResultItem> = Vec::with_capacity(ids.len().min(1024));

    for id in ids {
        // include_self 过滤
        if let Some(self_id) = self_id {
            if id == self_id {
                continue;
            }
        }

        let item_row: Option<(String, i64)> = stmt_item
            .query_row([id], |r| Ok((r.get(0)?, r.get(1).unwrap_or(0))))
            .optional()
            .unwrap_or(None);
        let (noun, spec_value) = item_row.unwrap_or_else(|| ("UNKNOWN".to_string(), 0));

        // noun 过滤
        if let Some(ref filter) = noun_filter {
            if !filter.contains(&noun.to_uppercase()) {
                continue;
            }
        }

        if let Some(ref filter) = spec_value_filter {
            if !filter.contains(&spec_value) {
                continue;
            }
        }

        let aabb_row: Option<(f32, f32, f32, f32, f32, f32)> = stmt_aabb
            .query_row([id], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })
            .optional()
            .unwrap_or(None);

        // 计算候选 AABB 到目标 AABB/点的最小距离，避免长模型因中心点较远被误排除。
        let distance = if let Some((minx, miny, minz, maxx, maxy, maxz)) = aabb_row {
            let candidate_aabb = aabb_from_row(minx, miny, minz, maxx, maxy, maxz);
            let min_distance = min_distance_to_targets(&candidate_aabb, &target_aabbs);

            if is_sphere && min_distance > search_distance {
                continue;
            }

            Some(min_distance)
        } else {
            None
        };

        let aabb = aabb_row.map(|(minx, miny, minz, maxx, maxy, maxz)| {
            aabb_dto_from_row(minx, miny, minz, maxx, maxy, maxz)
        });

        let refno = i64_to_refno_str(id as i64);
        results.push(SpatialQueryResultItem {
            refno,
            noun,
            spec_value,
            aabb,
            distance,
        });
    }

    // 按真实最小距离从近到远排序；距离相同按 refno 稳定排序。
    results.sort_by(|a, b| match (a.distance, b.distance) {
        (Some(da), Some(db)) => da
            .partial_cmp(&db)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                preferred_db_rank(a, &preferred_db_prefix)
                    .cmp(&preferred_db_rank(b, &preferred_db_prefix))
            })
            .then_with(|| a.refno.cmp(&b.refno)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => preferred_db_rank(a, &preferred_db_prefix)
            .cmp(&preferred_db_rank(b, &preferred_db_prefix))
            .then_with(|| a.refno.cmp(&b.refno)),
    });

    let total_count = results.len();
    let offset = page.saturating_sub(1).saturating_mul(per_page);
    let page_results = if offset >= total_count {
        Vec::new()
    } else {
        results.into_iter().skip(offset).take(per_page).collect()
    };

    success_spatial_query_result(page_results, total_count, page, per_page, query_bbox_dto)
}

// ============================================================================
// Handler：GET /api/sqlite-spatial/stats
// ============================================================================

/// GET /api/sqlite-spatial/stats
pub async fn api_sqlite_spatial_stats() -> Json<SpatialStatsResult> {
    let result = tokio::task::spawn_blocking(do_spatial_stats).await;
    match result {
        Ok(r) => Json(r),
        Err(e) => Json(SpatialStatsResult {
            success: false,
            total_elements: 0,
            index_type: String::new(),
            index_path: sqlite_index_path().display().to_string(),
            error: Some(format!("internal error: {}", e)),
        }),
    }
}

fn do_spatial_stats() -> SpatialStatsResult {
    let path = sqlite_index_path();
    let cached = match get_cached_index() {
        Ok(c) => c,
        Err(msg) => {
            return SpatialStatsResult {
                success: false,
                total_elements: 0,
                index_type: String::new(),
                index_path: path.display().to_string(),
                error: Some(format!("{}. 请先运行 import-spatial-index 构建索引。", msg)),
            };
        }
    };

    // 查询总元素数
    let conn = match Connection::open(&cached.path) {
        Ok(c) => c,
        Err(e) => {
            return SpatialStatsResult {
                success: false,
                total_elements: 0,
                index_type: "sqlite-rtree".to_string(),
                index_path: cached.path.display().to_string(),
                error: Some(format!("open connection failed: {}", e)),
            };
        }
    };

    let total: i64 = conn
        .query_row("SELECT COUNT(1) FROM aabb_index", [], |row| row.get(0))
        .unwrap_or(0);

    SpatialStatsResult {
        success: true,
        total_elements: total.max(0) as usize,
        index_type: "sqlite-rtree".to_string(),
        index_path: cached.path.display().to_string(),
        error: None,
    }
}

#[cfg(all(test, feature = "sqlite-index"))]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn with_test_index<T>(path: &std::path::Path, f: impl FnOnce() -> T) -> T {
        let _guard = test_guard().lock().unwrap();
        clear_test_index_path();
        set_test_index_path(path);
        let result = f();
        clear_test_index_path();
        result
    }

    fn set_test_index_path(path: &std::path::Path) {
        *test_index_override().lock().unwrap() = Some(path.to_path_buf());
    }

    fn clear_test_index_path() {
        *test_index_override().lock().unwrap() = None;
    }

    #[test]
    fn bbox_query_returns_refno_strings() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("spatial_index.sqlite");
        let idx = SqliteAabbIndex::open(&db).unwrap();
        idx.init_schema().unwrap();
        idx.insert_aabbs_with_items_and_spec_values(vec![
            // id = (1<<32)+2 => "1_2"
            (
                ((1u64 << 32) | 2u64) as i64,
                "PIPE".to_string(),
                42,
                0.0,
                1.0,
                0.0,
                1.0,
                0.0,
                1.0,
            ),
            (
                ((1u64 << 32) | 3u64) as i64,
                "WALL".to_string(),
                0,
                10.0,
                11.0,
                0.0,
                1.0,
                0.0,
                1.0,
            ),
        ])
        .unwrap();

        let resp = with_test_index(&db, || {
            let params = SqliteSpatialQueryParams {
                mode: Some("bbox".to_string()),
                refno: None,
                x: None,
                y: None,
                z: None,
                radius: None,
                distance: Some(0.0),
                minx: Some(-0.5),
                miny: Some(-0.5),
                minz: Some(-0.5),
                maxx: Some(1.5),
                maxy: Some(1.5),
                maxz: Some(1.5),
                max_results: None,
                page: None,
                per_page: None,
                nouns: None,
                spec_values: None,
                include_self: None,
                shape: None,
            };
            do_spatial_query(params, None)
        });
        assert!(resp.success);
        let items = resp.results.unwrap_or_default();
        assert!(
            items
                .iter()
                .any(|x| x.refno == "1_2" && x.noun == "PIPE" && x.spec_value == 42)
        );
        assert!(items.iter().any(|x| x.refno == "1_2"));
    }

    #[test]
    fn bbox_query_returns_zero_spec_value_when_missing() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("spatial_index.sqlite");
        let idx = SqliteAabbIndex::open(&db).unwrap();
        idx.init_schema().unwrap();
        idx.insert_aabbs_with_items_and_spec_values(vec![(
            ((1u64 << 32) | 9u64) as i64,
            "PIPE".to_string(),
            0,
            0.0,
            1.0,
            0.0,
            1.0,
            0.0,
            1.0,
        )])
        .unwrap();

        let resp = with_test_index(&db, || {
            let params = SqliteSpatialQueryParams {
                mode: Some("bbox".to_string()),
                refno: None,
                x: None,
                y: None,
                z: None,
                radius: None,
                distance: Some(0.0),
                minx: Some(-0.5),
                miny: Some(-0.5),
                minz: Some(-0.5),
                maxx: Some(1.5),
                maxy: Some(1.5),
                maxz: Some(1.5),
                max_results: None,
                page: None,
                per_page: None,
                nouns: None,
                spec_values: None,
                include_self: None,
                shape: None,
            };
            do_spatial_query(params, None)
        });
        assert!(resp.success);
        let items = resp.results.unwrap_or_default();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].spec_value, 0);
    }
}
