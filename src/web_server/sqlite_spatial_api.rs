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

use aios_core::{RefnoEnum, SurrealQueryExt, project_primary_db};
use axum::{
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use parry3d::bounding_volume::{Aabb, BoundingVolume};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params_from_iter};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Mutex, OnceLock};
use surrealdb::types::SurrealValue;

use crate::sqlite_index::{SqliteAabbIndex, i64_to_refno_str, refno_str_to_i64};

const DEFAULT_DISTANCE: f32 = 0.0;
const DEFAULT_MAX_HITS: usize = 5000;
const HARD_MAX_HITS: usize = 10_000;
const HARD_MAX_PAGE: usize = 100_000;

// ============================================================================
// SQLite index path resolution and read-only opening helpers
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

fn get_cached_index() -> Result<CachedIndex, String> {
    let path = sqlite_index_path();
    if !path.exists() {
        return Err(format!(
            "sqlite spatial index not found: {}",
            path.display()
        ));
    }
    if !path.is_file() {
        return Err(format!(
            "sqlite spatial index path is not a file: {}",
            path.display()
        ));
    }
    let idx = SqliteAabbIndex::open_existing(&path).map_err(|e| {
        format!(
            "open sqlite spatial index read-only failed at {}: {}",
            path.display(),
            e
        )
    })?;
    Ok(CachedIndex { idx, path })
}

fn open_sqlite_readonly(path: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
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

#[derive(Debug, Serialize, Clone)]
pub struct SpatialQueryResultItem {
    pub refno: String,
    pub noun: String,
    pub spec_value: i64,
    pub aabb: Option<AabbDto>,
    pub distance: Option<f32>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct SqliteSpatialNearbyParams {
    /// refno center, accepted as "dbnum_refno" or "dbnum/refno"
    pub refno: Option<String>,
    /// point center coordinates
    pub x: Option<String>,
    pub y: Option<String>,
    pub z: Option<String>,
    /// required positive finite search radius
    pub radius: Option<String>,
    /// sphere (default) | cube
    pub shape: Option<String>,
    /// comma-separated noun filter
    pub nouns: Option<String>,
    /// comma-separated spec value filter
    pub spec_values: Option<String>,
    /// refno mode self inclusion, default false for nearby
    pub include_self: Option<String>,
    /// page number, 1-based
    pub page: Option<String>,
    /// page size
    pub per_page: Option<String>,
    /// legacy/compat page-size alias when per_page is omitted
    pub max_results: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct SpatialNearbyCenter {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refno: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SpatialNearbyResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub center: Option<SpatialNearbyCenter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_bbox: Option<AabbDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub results: Option<Vec<SpatialQueryResultItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub returned_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_page: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated_candidates: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated_results: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_cap: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_cap: Option<usize>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedRefno {
    normalized: String,
    id: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NearbyShape {
    Sphere,
    Cube,
}

impl NearbyShape {
    fn as_str(self) -> &'static str {
        match self {
            Self::Sphere => "sphere",
            Self::Cube => "cube",
        }
    }
}

#[derive(Debug, Clone)]
enum NearbyCenterInput {
    Point { x: f32, y: f32, z: f32 },
    Refno(NormalizedRefno),
}

#[derive(Debug, Clone)]
struct NearbyQueryRequest {
    center: NearbyCenterInput,
    radius: f32,
    shape: NearbyShape,
    noun_filter: Option<Vec<String>>,
    spec_value_filter: Option<Vec<i64>>,
    include_self: bool,
    page: usize,
    per_page: usize,
    max_results: usize,
}

#[derive(Debug, Clone)]
struct ResolvedNearbyCenter {
    x: f32,
    y: f32,
    z: f32,
    source: String,
    refno: Option<String>,
    self_id: Option<i64>,
}

#[derive(Debug, Clone)]
struct NearbyApiError {
    status: StatusCode,
    message: String,
}

impl NearbyApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn service_unavailable(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }
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

fn non_empty_param(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn parse_finite_f32(value: &str, name: &str) -> Result<f32, NearbyApiError> {
    let parsed = value
        .parse::<f32>()
        .map_err(|_| NearbyApiError::bad_request(format!("invalid {name}: must be numeric")))?;
    if !parsed.is_finite() {
        return Err(NearbyApiError::bad_request(format!(
            "invalid {name}: must be finite"
        )));
    }
    Ok(parsed)
}

fn parse_positive_finite_f32(value: Option<&str>, name: &str) -> Result<f32, NearbyApiError> {
    let value = value.ok_or_else(|| NearbyApiError::bad_request(format!("{name} is required")))?;
    let parsed = parse_finite_f32(value, name)?;
    if parsed <= 0.0 {
        return Err(NearbyApiError::bad_request(format!(
            "invalid {name}: must be positive"
        )));
    }
    Ok(parsed)
}

fn parse_positive_usize_param(
    value: Option<&str>,
    name: &str,
    default: usize,
) -> Result<usize, NearbyApiError> {
    let Some(value) = value else {
        return Ok(default);
    };
    let parsed = value.parse::<usize>().map_err(|_| {
        NearbyApiError::bad_request(format!("invalid {name}: must be a positive integer"))
    })?;
    if parsed == 0 {
        return Err(NearbyApiError::bad_request(format!(
            "invalid {name}: must be positive"
        )));
    }
    Ok(parsed)
}

fn parse_limited_usize_param(
    value: Option<&str>,
    name: &str,
    default: usize,
    hard_max: usize,
) -> Result<usize, NearbyApiError> {
    let parsed = parse_positive_usize_param(value, name, default)?;
    Ok(parsed.min(hard_max))
}

fn parse_page_param(value: Option<&str>) -> Result<usize, NearbyApiError> {
    let page = parse_positive_usize_param(value, "page", 1)?;
    if page > HARD_MAX_PAGE {
        return Err(NearbyApiError::bad_request(format!(
            "invalid page: must be <= {}",
            HARD_MAX_PAGE
        )));
    }
    Ok(page)
}

fn parse_nearby_shape(shape: Option<&str>) -> Result<NearbyShape, NearbyApiError> {
    match shape.unwrap_or("sphere").to_ascii_lowercase().as_str() {
        "sphere" => Ok(NearbyShape::Sphere),
        "cube" => Ok(NearbyShape::Cube),
        other => Err(NearbyApiError::bad_request(format!(
            "unsupported shape `{other}` (expected sphere or cube)"
        ))),
    }
}

fn parse_nearby_bool(
    value: Option<&str>,
    name: &str,
    default: bool,
) -> Result<bool, NearbyApiError> {
    let Some(value) = value else {
        return Ok(default);
    };
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(NearbyApiError::bad_request(format!(
            "invalid {name}: expected true or false"
        ))),
    }
}

fn parse_nearby_spec_filter(
    spec_values: &Option<String>,
) -> Result<Option<Vec<i64>>, NearbyApiError> {
    let Some(raw) = spec_values else {
        return Ok(None);
    };
    let mut out = Vec::new();
    for value in raw
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let parsed = value.parse::<i64>().map_err(|_| {
            NearbyApiError::bad_request(format!(
                "invalid spec_values entry `{value}`: expected integer"
            ))
        })?;
        out.push(parsed);
    }
    if out.is_empty() {
        Ok(None)
    } else {
        Ok(Some(out))
    }
}

fn normalize_nearby_refno(input: &str) -> Result<NormalizedRefno, NearbyApiError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(NearbyApiError::bad_request("missing refno"));
    }
    let id = refno_str_to_i64(trimmed).ok_or_else(|| {
        NearbyApiError::bad_request("invalid refno format (expected dbnum_refno or dbnum/refno)")
    })?;
    let normalized = i64_to_refno_str(id);
    let slash_form = normalized.replace('_', "/");
    RefnoEnum::from_str(&slash_form)
        .map_err(|e| NearbyApiError::bad_request(format!("invalid refno `{trimmed}`: {e}")))?;
    Ok(NormalizedRefno { normalized, id })
}

fn parse_normalized_refno_enum(normalized: &str) -> Result<RefnoEnum, NearbyApiError> {
    let slash_form = normalized.replace('_', "/");
    RefnoEnum::from_str(&slash_form).map_err(|e| {
        NearbyApiError::bad_request(format!("invalid normalized refno `{normalized}`: {e}"))
    })
}

fn parse_nearby_request(
    params: &SqliteSpatialNearbyParams,
) -> Result<NearbyQueryRequest, NearbyApiError> {
    let has_refno = non_empty_param(&params.refno).is_some();
    let coord_count = [&params.x, &params.y, &params.z]
        .into_iter()
        .filter(|value| non_empty_param(value).is_some())
        .count();

    let center = if has_refno && coord_count > 0 {
        return Err(NearbyApiError::bad_request(
            "ambiguous center: provide either refno or x,y,z, not both",
        ));
    } else if has_refno {
        NearbyCenterInput::Refno(normalize_nearby_refno(
            non_empty_param(&params.refno).unwrap_or_default(),
        )?)
    } else if coord_count == 0 {
        return Err(NearbyApiError::bad_request(
            "missing center: provide refno or complete x,y,z coordinates",
        ));
    } else if coord_count != 3 {
        return Err(NearbyApiError::bad_request(
            "incomplete point center: x, y, and z are all required",
        ));
    } else {
        NearbyCenterInput::Point {
            x: parse_finite_f32(
                non_empty_param(&params.x).unwrap_or_default(),
                "x coordinate",
            )?,
            y: parse_finite_f32(
                non_empty_param(&params.y).unwrap_or_default(),
                "y coordinate",
            )?,
            z: parse_finite_f32(
                non_empty_param(&params.z).unwrap_or_default(),
                "z coordinate",
            )?,
        }
    };

    let radius = parse_positive_finite_f32(non_empty_param(&params.radius), "radius")?;
    let shape = parse_nearby_shape(non_empty_param(&params.shape))?;
    let include_self =
        parse_nearby_bool(non_empty_param(&params.include_self), "include_self", false)?;
    let page = parse_page_param(non_empty_param(&params.page))?;
    let explicit_per_page = non_empty_param(&params.per_page)
        .map(|_| {
            parse_limited_usize_param(
                non_empty_param(&params.per_page),
                "per_page",
                DEFAULT_MAX_HITS,
                HARD_MAX_HITS,
            )
        })
        .transpose()?;
    let explicit_max_results = non_empty_param(&params.max_results)
        .map(|_| {
            parse_limited_usize_param(
                non_empty_param(&params.max_results),
                "max_results",
                DEFAULT_MAX_HITS,
                HARD_MAX_HITS,
            )
        })
        .transpose()?;
    let max_results = explicit_max_results
        .unwrap_or_else(|| {
            explicit_per_page
                .unwrap_or(DEFAULT_MAX_HITS)
                .max(DEFAULT_MAX_HITS)
        })
        .min(HARD_MAX_HITS);
    let per_page = explicit_per_page
        .unwrap_or_else(|| explicit_max_results.unwrap_or(DEFAULT_MAX_HITS))
        .min(max_results)
        .min(HARD_MAX_HITS);

    Ok(NearbyQueryRequest {
        center,
        radius,
        shape,
        noun_filter: parse_noun_filter(&params.nouns),
        spec_value_filter: parse_nearby_spec_filter(&params.spec_values)?,
        include_self,
        page,
        per_page,
        max_results,
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

fn error_nearby_result(error: impl Into<String>) -> SpatialNearbyResult {
    SpatialNearbyResult {
        success: false,
        center: None,
        radius: None,
        shape: None,
        query_bbox: None,
        results: None,
        total_count: None,
        returned_count: None,
        page: None,
        per_page: None,
        has_more: None,
        truncated: None,
        truncated_candidates: None,
        truncated_results: None,
        candidate_count: None,
        candidate_cap: None,
        result_cap: None,
        error: Some(error.into()),
    }
}

fn nearby_error_response(error: NearbyApiError) -> Response {
    (error.status, Json(error_nearby_result(error.message))).into_response()
}

#[derive(Debug, Clone, Copy)]
struct NearbyQueryMetadata {
    candidate_count: usize,
    candidate_cap: usize,
    truncated_candidates: bool,
    truncated_results: bool,
}

fn success_nearby_result(
    request: &NearbyQueryRequest,
    center: &ResolvedNearbyCenter,
    query_bbox: AabbDto,
    results: Vec<SpatialQueryResultItem>,
    total_count: usize,
    metadata: NearbyQueryMetadata,
) -> SpatialNearbyResult {
    let returned_count = results.len();
    let end = request
        .page
        .saturating_sub(1)
        .saturating_mul(request.per_page)
        .saturating_add(returned_count);
    let has_more = end < total_count;

    SpatialNearbyResult {
        success: true,
        center: Some(SpatialNearbyCenter {
            x: center.x,
            y: center.y,
            z: center.z,
            source: center.source.clone(),
            refno: center.refno.clone(),
        }),
        radius: Some(request.radius),
        shape: Some(request.shape.as_str().to_string()),
        query_bbox: Some(query_bbox),
        results: Some(results),
        total_count: Some(total_count),
        returned_count: Some(returned_count),
        page: Some(request.page),
        per_page: Some(request.per_page),
        has_more: Some(has_more),
        truncated: Some(has_more || metadata.truncated_candidates || metadata.truncated_results),
        truncated_candidates: Some(metadata.truncated_candidates),
        truncated_results: Some(metadata.truncated_results),
        candidate_count: Some(metadata.candidate_count),
        candidate_cap: Some(metadata.candidate_cap),
        result_cap: Some(request.max_results),
        error: None,
    }
}

fn resolved_point_center(x: f32, y: f32, z: f32) -> ResolvedNearbyCenter {
    ResolvedNearbyCenter {
        x,
        y,
        z,
        source: "point".to_string(),
        refno: None,
        self_id: None,
    }
}

fn center_from_world_transform_matrix(
    matrix: &[f64],
    source: &str,
    refno: Option<String>,
    self_id: Option<i64>,
) -> Option<ResolvedNearbyCenter> {
    if matrix.len() != 16 {
        return None;
    }
    let x = matrix[12];
    let y = matrix[13];
    let z = matrix[14];
    if !(x.is_finite() && y.is_finite() && z.is_finite()) {
        return None;
    }
    let x = x as f32;
    let y = y as f32;
    let z = z as f32;
    if !(x.is_finite() && y.is_finite() && z.is_finite()) {
        return None;
    }
    Some(ResolvedNearbyCenter {
        x,
        y,
        z,
        source: source.to_string(),
        refno,
        self_id,
    })
}

fn parse_nearby_transform_matrix(trans: Option<serde_json::Value>) -> Option<Vec<f64>> {
    let trans = trans?;

    if let Some(obj) = trans.as_object() {
        if let Some(d) = obj.get("d").and_then(|value| value.as_array()) {
            if d.len() == 16 {
                let values = d
                    .iter()
                    .map(|value| value.as_f64())
                    .collect::<Option<Vec<_>>>()?;
                return Some(values);
            }
        }

        if let (Some(t), Some(r), Some(s)) = (
            obj.get("translation").and_then(|value| value.as_array()),
            obj.get("rotation").and_then(|value| value.as_array()),
            obj.get("scale").and_then(|value| value.as_array()),
        ) {
            if t.len() >= 3 && r.len() >= 4 && s.len() >= 3 {
                let translation = [
                    t[0].as_f64().unwrap_or(0.0),
                    t[1].as_f64().unwrap_or(0.0),
                    t[2].as_f64().unwrap_or(0.0),
                ];
                let rotation = [
                    r[0].as_f64().unwrap_or(0.0),
                    r[1].as_f64().unwrap_or(0.0),
                    r[2].as_f64().unwrap_or(0.0),
                    r[3].as_f64().unwrap_or(1.0),
                ];
                let scale = [
                    s[0].as_f64().unwrap_or(1.0),
                    s[1].as_f64().unwrap_or(1.0),
                    s[2].as_f64().unwrap_or(1.0),
                ];

                return Some(compose_nearby_transform_matrix(
                    translation,
                    rotation,
                    scale,
                ));
            }
        }
    }

    if let Some(arr) = trans.as_array() {
        if arr.len() == 16 {
            return arr.iter().map(|value| value.as_f64()).collect();
        }
    }

    None
}

fn compose_nearby_transform_matrix(
    translation: [f64; 3],
    rotation: [f64; 4],
    scale: [f64; 3],
) -> Vec<f64> {
    let [x, y, z] = translation;
    let [qx, qy, qz, qw] = rotation;
    let [sx, sy, sz] = scale;

    let x2 = qx + qx;
    let y2 = qy + qy;
    let z2 = qz + qz;
    let xx = qx * x2;
    let xy = qx * y2;
    let xz = qx * z2;
    let yy = qy * y2;
    let yz = qy * z2;
    let zz = qz * z2;
    let wx = qw * x2;
    let wy = qw * y2;
    let wz = qw * z2;

    vec![
        (1.0 - (yy + zz)) * sx,
        (xy + wz) * sx,
        (xz - wy) * sx,
        0.0,
        (xy - wz) * sy,
        (1.0 - (xx + zz)) * sy,
        (yz + wx) * sy,
        0.0,
        (xz + wy) * sz,
        (yz - wx) * sz,
        (1.0 - (xx + yy)) * sz,
        0.0,
        x,
        y,
        z,
        1.0,
    ]
}

async fn resolve_refno_nearby_center(
    refno: &NormalizedRefno,
) -> Result<ResolvedNearbyCenter, NearbyApiError> {
    let refno_enum = parse_normalized_refno_enum(&refno.normalized)?;
    let pe_transform_key = refno_enum.to_pe_key().replace("pe:", "pe_transform:");

    #[derive(Deserialize, SurrealValue)]
    struct TransformQueryResult {
        world_trans: Option<serde_json::Value>,
    }

    let sql = format!(
        "SELECT world_trans.d as world_trans FROM {} WHERE world_trans != none",
        pe_transform_key
    );

    let cached = project_primary_db()
        .query_take::<Option<TransformQueryResult>>(&sql, 0)
        .await
        .ok()
        .flatten()
        .and_then(|row| parse_nearby_transform_matrix(row.world_trans))
        .and_then(|matrix| {
            center_from_world_transform_matrix(
                &matrix,
                "transform_cache",
                Some(refno.normalized.clone()),
                Some(refno.id),
            )
        });

    if let Some(center) = cached {
        return Ok(center);
    }

    let refno_enum = parse_normalized_refno_enum(&refno.normalized)?;
    let fallback_matrix = aios_core::transform::get_world_mat4(refno_enum, false)
        .await
        .map_err(|e| {
            NearbyApiError::bad_request(format!(
                "unable to resolve world transform center for refno {}: {}",
                refno.normalized, e
            ))
        })?
        .map(|matrix| matrix.to_cols_array().iter().copied().collect::<Vec<f64>>());

    if let Some(center) = fallback_matrix.and_then(|matrix| {
        center_from_world_transform_matrix(
            &matrix,
            "world_transform_fallback",
            Some(refno.normalized.clone()),
            Some(refno.id),
        )
    }) {
        return Ok(center);
    }

    Err(NearbyApiError::bad_request(format!(
        "unable to resolve world transform center for refno {}",
        refno.normalized
    )))
}

fn point_radius_aabb(center: &ResolvedNearbyCenter, radius: f32) -> Aabb {
    Aabb::new(
        [center.x - radius, center.y - radius, center.z - radius].into(),
        [center.x + radius, center.y + radius, center.z + radius].into(),
    )
}

fn point_axis_gap(point: f32, min: f32, max: f32) -> f32 {
    if point < min {
        min - point
    } else if point > max {
        point - max
    } else {
        0.0
    }
}

fn point_to_aabb_distance(center: &ResolvedNearbyCenter, aabb: &Aabb) -> f32 {
    let dx = point_axis_gap(center.x, aabb.mins.x, aabb.maxs.x);
    let dy = point_axis_gap(center.y, aabb.mins.y, aabb.maxs.y);
    let dz = point_axis_gap(center.z, aabb.mins.z, aabb.maxs.z);
    (dx * dx + dy * dy + dz * dz).sqrt()
}

#[derive(Debug)]
struct NearbyCandidateRow {
    id: i64,
    noun: String,
    spec_value: i64,
    aabb: Aabb,
}

fn query_nearby_candidate_rows(
    conn: &Connection,
    ids: &[i64],
) -> rusqlite::Result<Vec<NearbyCandidateRow>> {
    let mut rows_by_id: HashMap<i64, NearbyCandidateRow> = HashMap::with_capacity(ids.len());

    for chunk in ids.chunks(900) {
        if chunk.is_empty() {
            continue;
        }
        let placeholders = (0..chunk.len()).map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT a.id, a.min_x, a.max_x, a.min_y, a.max_y, a.min_z, a.max_z, \
                    COALESCE(i.noun, 'UNKNOWN') as noun, COALESCE(i.spec_value, 0) as spec_value \
             FROM aabb_index a \
             LEFT JOIN items i ON i.id = a.id \
             WHERE a.id IN ({placeholders})"
        );
        let mut stmt = conn.prepare(&sql)?;
        let mapped = stmt.query_map(params_from_iter(chunk.iter()), |row| {
            let id: i64 = row.get(0)?;
            let minx: f32 = row.get(1)?;
            let maxx: f32 = row.get(2)?;
            let miny: f32 = row.get(3)?;
            let maxy: f32 = row.get(4)?;
            let minz: f32 = row.get(5)?;
            let maxz: f32 = row.get(6)?;
            let noun: String = row.get(7)?;
            let spec_value: i64 = row.get(8)?;
            Ok(NearbyCandidateRow {
                id,
                noun,
                spec_value,
                aabb: aabb_from_row(minx, miny, minz, maxx, maxy, maxz),
            })
        })?;
        for row in mapped {
            let row = row?;
            rows_by_id.insert(row.id, row);
        }
    }

    let mut rows = ids
        .iter()
        .filter_map(|id| rows_by_id.remove(id))
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| row.id);
    Ok(rows)
}

fn do_nearby_query(
    request: NearbyQueryRequest,
    center: ResolvedNearbyCenter,
) -> Result<SpatialNearbyResult, NearbyApiError> {
    let cached = get_cached_index().map_err(|e| {
        NearbyApiError::service_unavailable(format!(
            "{}. 请先运行 import-spatial-index 构建索引。",
            e
        ))
    })?;
    let query_aabb = point_radius_aabb(&center, request.radius);
    let query_bbox_dto = aabb_to_dto(&query_aabb);

    let mut ids = cached
        .idx
        .query_intersect_limited(
            query_aabb.mins.x as f64,
            query_aabb.maxs.x as f64,
            query_aabb.mins.y as f64,
            query_aabb.maxs.y as f64,
            query_aabb.mins.z as f64,
            query_aabb.maxs.z as f64,
            HARD_MAX_HITS.saturating_add(1),
        )
        .map_err(|e| NearbyApiError::internal(format!("query_intersect failed: {}", e)))?;
    ids.sort_unstable();
    ids.dedup();
    let truncated_candidates = ids.len() > HARD_MAX_HITS;
    if truncated_candidates {
        ids.truncate(HARD_MAX_HITS);
    }
    let candidate_count = ids.len();

    let conn = open_sqlite_readonly(&cached.path).map_err(|e| {
        NearbyApiError::service_unavailable(format!("open sqlite connection failed: {}", e))
    })?;
    let candidate_rows = query_nearby_candidate_rows(&conn, &ids)
        .map_err(|e| NearbyApiError::internal(format!("query candidate rows failed: {}", e)))?;

    let mut results: Vec<SpatialQueryResultItem> = Vec::with_capacity(candidate_rows.len());
    for row in candidate_rows {
        if !request.include_self && center.self_id == Some(row.id) {
            continue;
        }

        if let Some(filter) = &request.noun_filter {
            if !filter.contains(&row.noun.to_uppercase()) {
                continue;
            }
        }

        if let Some(filter) = &request.spec_value_filter {
            if !filter.contains(&row.spec_value) {
                continue;
            }
        }

        let distance = point_to_aabb_distance(&center, &row.aabb);
        if request.shape == NearbyShape::Sphere && distance > request.radius {
            continue;
        }

        results.push(SpatialQueryResultItem {
            refno: i64_to_refno_str(row.id),
            noun: row.noun,
            spec_value: row.spec_value,
            aabb: Some(aabb_to_dto(&row.aabb)),
            distance: Some(distance),
        });
    }

    results.sort_by(|a, b| match (a.distance, b.distance) {
        (Some(da), Some(db)) => da
            .partial_cmp(&db)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.refno.cmp(&b.refno)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.refno.cmp(&b.refno),
    });

    let truncated_results = results.len() > request.max_results;
    if truncated_results {
        results.truncate(request.max_results);
    }
    let total_count = results.len();
    let offset = request
        .page
        .saturating_sub(1)
        .saturating_mul(request.per_page);
    let page_results = if offset >= total_count {
        Vec::new()
    } else {
        results
            .into_iter()
            .skip(offset)
            .take(request.per_page)
            .collect()
    };

    Ok(success_nearby_result(
        &request,
        &center,
        query_bbox_dto,
        page_results,
        total_count,
        NearbyQueryMetadata {
            candidate_count,
            candidate_cap: HARD_MAX_HITS,
            truncated_candidates,
            truncated_results,
        },
    ))
}

// ============================================================================
// Handler：GET /api/sqlite-spatial/nearby
// ============================================================================

/// GET /api/sqlite-spatial/nearby
pub async fn api_sqlite_spatial_nearby(
    Query(params): Query<SqliteSpatialNearbyParams>,
) -> Response {
    let request = match parse_nearby_request(&params) {
        Ok(request) => request,
        Err(error) => return nearby_error_response(error),
    };

    let center = match &request.center {
        NearbyCenterInput::Point { x, y, z } => resolved_point_center(*x, *y, *z),
        NearbyCenterInput::Refno(refno) => match resolve_refno_nearby_center(refno).await {
            Ok(center) => center,
            Err(error) => return nearby_error_response(error),
        },
    };

    let result = tokio::task::spawn_blocking(move || do_nearby_query(request, center)).await;
    match result {
        Ok(Ok(response)) => (StatusCode::OK, Json(response)).into_response(),
        Ok(Err(error)) => nearby_error_response(error),
        Err(error) => nearby_error_response(NearbyApiError::internal(format!(
            "internal error: {}",
            error
        ))),
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
    let conn = open_sqlite_readonly(&cached.path)
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
    Ok(query_nearby_candidate_rows(conn, ids)?
        .into_iter()
        .map(|row| row.aabb)
        .collect())
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
                    &cached,
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
        let conn = match open_sqlite_readonly(&cached.path) {
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
                        return query_by_target_aabbs(params, &cached, aabbs, distance, self_id);
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
    query_by_target_aabbs(params, &cached, vec![base_aabb], distance, self_id)
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
    let conn = match open_sqlite_readonly(&cached.path) {
        Ok(c) => c,
        Err(e) => {
            return error_spatial_query_result(
                format!("open sqlite connection failed: {}", e),
                query_bbox_dto.clone(),
            );
        }
    };

    let candidate_rows = match query_nearby_candidate_rows(&conn, &ids) {
        Ok(rows) => rows,
        Err(e) => {
            return error_spatial_query_result(
                format!("query candidate rows failed: {}", e),
                query_bbox_dto.clone(),
            );
        }
    };

    let mut results: Vec<SpatialQueryResultItem> =
        Vec::with_capacity(candidate_rows.len().min(1024));

    for row in candidate_rows {
        // include_self 过滤
        if let Some(self_id) = self_id {
            if row.id == self_id {
                continue;
            }
        }

        // noun 过滤
        if let Some(ref filter) = noun_filter {
            if !filter.contains(&row.noun.to_uppercase()) {
                continue;
            }
        }

        if let Some(ref filter) = spec_value_filter {
            if !filter.contains(&row.spec_value) {
                continue;
            }
        }

        // 计算候选 AABB 到目标 AABB/点的最小距离，避免长模型因中心点较远被误排除。
        let min_distance = min_distance_to_targets(&row.aabb, &target_aabbs);

        if is_sphere && min_distance > search_distance {
            continue;
        }

        let refno = i64_to_refno_str(row.id);
        results.push(SpatialQueryResultItem {
            refno,
            noun: row.noun,
            spec_value: row.spec_value,
            aabb: Some(aabb_to_dto(&row.aabb)),
            distance: Some(min_distance),
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
    let conn = match open_sqlite_readonly(&cached.path) {
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

    fn nearby_params() -> SqliteSpatialNearbyParams {
        SqliteSpatialNearbyParams {
            refno: None,
            x: None,
            y: None,
            z: None,
            radius: None,
            shape: None,
            nouns: None,
            spec_values: None,
            include_self: None,
            page: None,
            per_page: None,
            max_results: None,
        }
    }

    fn point_nearby_request(
        x: impl ToString,
        y: impl ToString,
        z: impl ToString,
        radius: impl ToString,
        shape: Option<&str>,
    ) -> NearbyQueryRequest {
        let mut params = nearby_params();
        params.x = Some(x.to_string());
        params.y = Some(y.to_string());
        params.z = Some(z.to_string());
        params.radius = Some(radius.to_string());
        params.shape = shape.map(str::to_string);
        parse_nearby_request(&params).unwrap()
    }

    fn resolved_point(x: f32, y: f32, z: f32) -> ResolvedNearbyCenter {
        ResolvedNearbyCenter {
            x,
            y,
            z,
            source: "point".to_string(),
            refno: None,
            self_id: None,
        }
    }

    #[test]
    fn nearby_point_defaults_to_sphere_and_filters_corner_candidates() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("spatial_index.sqlite");
        let idx = SqliteAabbIndex::open(&db).unwrap();
        idx.init_schema().unwrap();
        idx.insert_aabbs_with_items_and_spec_values(vec![
            (
                ((1u64 << 32) | 10u64) as i64,
                "PIPE".to_string(),
                1,
                2.0,
                3.0,
                0.0,
                1.0,
                0.0,
                1.0,
            ),
            (
                ((1u64 << 32) | 11u64) as i64,
                "PIPE".to_string(),
                1,
                9.0,
                10.0,
                9.0,
                10.0,
                9.0,
                10.0,
            ),
        ])
        .unwrap();

        let sphere = with_test_index(&db, || {
            do_nearby_query(
                point_nearby_request(0, 0, 0, 10, None),
                resolved_point(0.0, 0.0, 0.0),
            )
            .unwrap()
        });
        assert!(sphere.success);
        assert_eq!(sphere.shape.as_deref(), Some("sphere"));
        assert_eq!(sphere.center.as_ref().unwrap().source, "point");
        assert_eq!(sphere.query_bbox.as_ref().unwrap().min.x, -10.0);
        assert_eq!(sphere.query_bbox.as_ref().unwrap().max.x, 10.0);
        let sphere_refnos: Vec<_> = sphere
            .results
            .as_ref()
            .unwrap()
            .iter()
            .map(|item| item.refno.as_str())
            .collect();
        assert_eq!(sphere_refnos, vec!["1_10"]);

        let cube = with_test_index(&db, || {
            do_nearby_query(
                point_nearby_request(0, 0, 0, 10, Some("cube")),
                resolved_point(0.0, 0.0, 0.0),
            )
            .unwrap()
        });
        let cube_refnos: Vec<_> = cube
            .results
            .as_ref()
            .unwrap()
            .iter()
            .map(|item| item.refno.as_str())
            .collect();
        assert_eq!(cube_refnos, vec!["1_10", "1_11"]);
    }

    #[test]
    fn nearby_refno_normalizes_and_include_self_controls_self_match() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("spatial_index.sqlite");
        let idx = SqliteAabbIndex::open(&db).unwrap();
        idx.init_schema().unwrap();
        idx.insert_aabbs_with_items_and_spec_values(vec![
            (
                ((1u64 << 32) | 100u64) as i64,
                "EQUI".to_string(),
                0,
                99.0,
                101.0,
                -1.0,
                1.0,
                -1.0,
                1.0,
            ),
            (
                ((1u64 << 32) | 101u64) as i64,
                "PIPE".to_string(),
                0,
                102.0,
                103.0,
                0.0,
                1.0,
                0.0,
                1.0,
            ),
            (
                ((1u64 << 32) | 102u64) as i64,
                "PIPE".to_string(),
                0,
                102.0,
                103.0,
                0.0,
                1.0,
                0.0,
                1.0,
            ),
        ])
        .unwrap();

        let slash = normalize_nearby_refno("1/100").unwrap();
        let underscore = normalize_nearby_refno("1_100").unwrap();
        assert_eq!(slash.normalized, "1_100");
        assert_eq!(slash.id, underscore.id);

        let mut params = nearby_params();
        params.refno = Some("1/100".to_string());
        params.radius = Some("5".to_string());
        params.include_self = Some("false".to_string());
        let request = parse_nearby_request(&params).unwrap();
        let center = center_from_world_transform_matrix(
            &[
                1.0, 0.0, 0.0, 0.0, //
                0.0, 1.0, 0.0, 0.0, //
                0.0, 0.0, 1.0, 0.0, //
                100.0, 0.0, 0.0, 1.0,
            ],
            "transform_cache",
            Some(slash.normalized.clone()),
            Some(slash.id),
        )
        .unwrap();

        let without_self =
            with_test_index(&db, || do_nearby_query(request, center.clone()).unwrap());
        let without_self_refnos: Vec<_> = without_self
            .results
            .as_ref()
            .unwrap()
            .iter()
            .map(|item| item.refno.as_str())
            .collect();
        assert_eq!(without_self_refnos, vec!["1_101", "1_102"]);

        params.include_self = Some("true".to_string());
        let with_self_request = parse_nearby_request(&params).unwrap();
        let with_self =
            with_test_index(&db, || do_nearby_query(with_self_request, center).unwrap());
        let with_self_refnos: Vec<_> = with_self
            .results
            .as_ref()
            .unwrap()
            .iter()
            .map(|item| item.refno.as_str())
            .collect();
        assert_eq!(with_self_refnos, vec!["1_100", "1_101", "1_102"]);
    }

    #[test]
    fn nearby_filters_before_pagination_and_sorts_by_distance_then_refno() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("spatial_index.sqlite");
        let idx = SqliteAabbIndex::open(&db).unwrap();
        idx.init_schema().unwrap();
        idx.insert_aabbs_with_items_and_spec_values(vec![
            (
                ((1u64 << 32) | 20u64) as i64,
                "PIPE".to_string(),
                7,
                1.0,
                2.0,
                0.0,
                1.0,
                0.0,
                1.0,
            ),
            (
                ((1u64 << 32) | 21u64) as i64,
                "PIPE".to_string(),
                7,
                1.0,
                2.0,
                0.0,
                1.0,
                0.0,
                1.0,
            ),
            (
                ((1u64 << 32) | 22u64) as i64,
                "WALL".to_string(),
                7,
                1.0,
                2.0,
                0.0,
                1.0,
                0.0,
                1.0,
            ),
            (
                ((1u64 << 32) | 23u64) as i64,
                "PIPE".to_string(),
                8,
                1.0,
                2.0,
                0.0,
                1.0,
                0.0,
                1.0,
            ),
        ])
        .unwrap();

        let mut params = nearby_params();
        params.x = Some("0".to_string());
        params.y = Some("0".to_string());
        params.z = Some("0".to_string());
        params.radius = Some("10".to_string());
        params.nouns = Some("pipe".to_string());
        params.spec_values = Some("7".to_string());
        params.page = Some("2".to_string());
        params.per_page = Some("1".to_string());
        let request = parse_nearby_request(&params).unwrap();

        let resp = with_test_index(&db, || {
            do_nearby_query(request, resolved_point(0.0, 0.0, 0.0)).unwrap()
        });
        assert_eq!(resp.total_count, Some(2));
        assert_eq!(resp.returned_count, Some(1));
        assert_eq!(resp.page, Some(2));
        assert_eq!(resp.per_page, Some(1));
        assert_eq!(resp.has_more, Some(false));
        assert_eq!(resp.results.as_ref().unwrap()[0].refno, "1_21");
    }

    #[test]
    fn nearby_limit_params_are_validated_and_clamped() {
        let mut params = nearby_params();
        params.x = Some("0".to_string());
        params.y = Some("0".to_string());
        params.z = Some("0".to_string());
        params.radius = Some("10".to_string());
        params.max_results = Some("2".to_string());
        let request = parse_nearby_request(&params).unwrap();
        assert_eq!(request.per_page, 2);
        assert_eq!(request.max_results, 2);

        params.max_results = None;
        params.per_page = Some("20000".to_string());
        let request = parse_nearby_request(&params).unwrap();
        assert_eq!(request.per_page, HARD_MAX_HITS);
        assert_eq!(request.max_results, HARD_MAX_HITS);

        params.per_page = Some("0".to_string());
        assert!(
            parse_nearby_request(&params)
                .unwrap_err()
                .message
                .contains("per_page")
        );

        params.per_page = Some("1".to_string());
        params.max_results = Some("0".to_string());
        assert!(
            parse_nearby_request(&params)
                .unwrap_err()
                .message
                .contains("max_results")
        );

        params.max_results = None;
        params.page = Some((HARD_MAX_PAGE + 1).to_string());
        assert!(
            parse_nearby_request(&params)
                .unwrap_err()
                .message
                .contains("page")
        );
    }

    #[test]
    fn nearby_caps_results_and_reports_truncation_metadata() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("spatial_index.sqlite");
        let idx = SqliteAabbIndex::open(&db).unwrap();
        idx.init_schema().unwrap();
        idx.insert_aabbs_with_items_and_spec_values((1_u64..=5).map(|refno| {
            (
                ((1_u64 << 32) | refno) as i64,
                "PIPE".to_string(),
                1,
                refno as f64,
                refno as f64 + 0.5,
                0.0,
                0.5,
                0.0,
                0.5,
            )
        }))
        .unwrap();

        let mut params = nearby_params();
        params.x = Some("0".to_string());
        params.y = Some("0".to_string());
        params.z = Some("0".to_string());
        params.radius = Some("100".to_string());
        params.max_results = Some("2".to_string());
        let request = parse_nearby_request(&params).unwrap();

        let resp = with_test_index(&db, || {
            do_nearby_query(request, resolved_point(0.0, 0.0, 0.0)).unwrap()
        });
        assert!(resp.success);
        assert_eq!(resp.returned_count, Some(2));
        assert_eq!(resp.total_count, Some(2));
        assert_eq!(resp.result_cap, Some(2));
        assert_eq!(resp.candidate_count, Some(5));
        assert_eq!(resp.candidate_cap, Some(HARD_MAX_HITS));
        assert_eq!(resp.truncated_results, Some(true));
        assert_eq!(resp.truncated_candidates, Some(false));
        assert_eq!(resp.results.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn nearby_truncates_candidates_before_loading_rows() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("spatial_index.sqlite");
        let idx = SqliteAabbIndex::open(&db).unwrap();
        idx.init_schema().unwrap();
        idx.insert_aabbs_with_items_and_spec_values((1_u64..=(HARD_MAX_HITS as u64 + 1)).map(
            |refno| {
                (
                    ((1_u64 << 32) | refno) as i64,
                    "PIPE".to_string(),
                    1,
                    0.0,
                    1.0,
                    0.0,
                    1.0,
                    0.0,
                    1.0,
                )
            },
        ))
        .unwrap();

        let request = point_nearby_request(0, 0, 0, 10, Some("cube"));
        let resp = with_test_index(&db, || {
            do_nearby_query(request, resolved_point(0.0, 0.0, 0.0)).unwrap()
        });
        assert!(resp.success);
        assert_eq!(resp.candidate_count, Some(HARD_MAX_HITS));
        assert_eq!(resp.candidate_cap, Some(HARD_MAX_HITS));
        assert_eq!(resp.truncated_candidates, Some(true));
    }

    #[test]
    fn missing_index_path_returns_explicit_error_without_creating_file() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("missing").join("spatial_index.sqlite");

        let err = with_test_index(&db, || {
            do_nearby_query(
                point_nearby_request(0, 0, 0, 10, None),
                resolved_point(0.0, 0.0, 0.0),
            )
            .unwrap_err()
        });
        assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(err.message.contains("not found"));
        assert!(!db.exists());

        let stats = with_test_index(&db, do_spatial_stats);
        assert!(!stats.success);
        assert!(
            stats
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("not found")
        );
        assert!(!db.exists());
    }

    #[test]
    fn nearby_validation_rejects_missing_ambiguous_and_invalid_inputs() {
        let mut params = nearby_params();
        params.radius = Some("10".to_string());
        assert!(
            parse_nearby_request(&params)
                .unwrap_err()
                .message
                .contains("missing center")
        );

        params.x = Some("0".to_string());
        params.y = Some("0".to_string());
        assert!(
            parse_nearby_request(&params)
                .unwrap_err()
                .message
                .contains("incomplete point")
        );

        params.z = Some("0".to_string());
        params.refno = Some("1_2".to_string());
        assert!(
            parse_nearby_request(&params)
                .unwrap_err()
                .message
                .contains("ambiguous center")
        );

        let mut invalid_radius = nearby_params();
        invalid_radius.x = Some("0".to_string());
        invalid_radius.y = Some("0".to_string());
        invalid_radius.z = Some("0".to_string());
        invalid_radius.radius = Some("NaN".to_string());
        assert!(
            parse_nearby_request(&invalid_radius)
                .unwrap_err()
                .message
                .contains("radius")
        );

        invalid_radius.radius = Some("0".to_string());
        assert!(
            parse_nearby_request(&invalid_radius)
                .unwrap_err()
                .message
                .contains("radius")
        );

        invalid_radius.radius = Some("10".to_string());
        invalid_radius.x = Some("abc".to_string());
        assert!(
            parse_nearby_request(&invalid_radius)
                .unwrap_err()
                .message
                .contains("coordinate")
        );

        invalid_radius.x = Some("0".to_string());
        invalid_radius.shape = Some("pyramid".to_string());
        assert!(
            parse_nearby_request(&invalid_radius)
                .unwrap_err()
                .message
                .contains("shape")
        );
    }

    #[tokio::test]
    async fn nearby_handler_returns_client_error_for_missing_center() {
        let mut params = nearby_params();
        params.radius = Some("10".to_string());
        let response = api_sqlite_spatial_nearby(Query(params)).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn transform_matrix_center_uses_column_major_translation_and_source() {
        let center = center_from_world_transform_matrix(
            &[
                1.0, 0.0, 0.0, 0.0, //
                0.0, 1.0, 0.0, 0.0, //
                0.0, 0.0, 1.0, 0.0, //
                12.5, -3.0, 8.25, 1.0,
            ],
            "world_transform_fallback",
            Some("1_2".to_string()),
            Some(((1u64 << 32) | 2u64) as i64),
        )
        .unwrap();

        assert_eq!(center.x, 12.5);
        assert_eq!(center.y, -3.0);
        assert_eq!(center.z, 8.25);
        assert_eq!(center.source, "world_transform_fallback");
        assert_eq!(center.refno.as_deref(), Some("1_2"));
        assert_eq!(center.self_id, Some(((1u64 << 32) | 2u64) as i64));

        assert!(
            center_from_world_transform_matrix(
                &[1.0, 0.0, 0.0],
                "transform_cache",
                Some("1_2".to_string()),
                Some(((1u64 << 32) | 2u64) as i64),
            )
            .is_none()
        );
    }
}
