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
//! - `GET /api/sqlite-spatial/nearby` - 前端空间查询抽屉契约：按 refno 或点 + 半径查询
//! - `GET /api/sqlite-spatial/nearest-clearance` - 按 refno 或点查询最近净距
//! - `GET /api/sqlite-spatial/stats` - 获取索引统计与健康信息

use aios_core::{RefnoEnum, pdms_types::TOTAL_NEG_NOUN_NAMES};
use axum::{extract::Query, response::Json};
use glam::Vec3;
use parry3d::bounding_volume::{Aabb, BoundingVolume};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Mutex, OnceLock};

use crate::fast_model::gen_model::model_record_id::model_refno_range;
use crate::sqlite_index::{SqliteAabbIndex, i64_to_refno_str, refno_str_to_i64};

const DEFAULT_DISTANCE: f32 = 0.0;
const DEFAULT_MAX_HITS: usize = 5000;
const HARD_MAX_HITS: usize = 10_000;
const DEFAULT_CLEARANCE_RADIUS_MM: f32 = 5_000.0;
const MAX_CLEARANCE_RADIUS_MM: f32 = 100_000.0;
const DEFAULT_CLEARANCE_MAX_PER_GROUP: usize = 1;
const MAX_CLEARANCE_MAX_PER_GROUP: usize = 100;

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
    /// bbox | refno | position | bran_centerline
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
    /// 是否包含负实体（默认 false）
    pub include_negative: Option<bool>,
    /// 查询形状："cube"（默认）| "sphere"（球体，会对结果做距离二次过滤）
    pub shape: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SpatialQueryResult {
    pub success: bool,
    pub results: Option<Vec<SpatialQueryResultItem>>,
    /// /nearby 响应的查询中心；legacy /query 可能没有该字段
    #[serde(skip_serializing_if = "Option::is_none")]
    pub center: Option<SpatialNearbyCenterDto>,
    /// /nearby 响应使用的半径
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    /// /nearby 响应使用的形状
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
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
    /// 本次查询结果可用的过滤选项
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter_options: Option<SpatialQueryFilterOptions>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct SpatialQueryFilterOptions {
    pub nouns: Vec<SpatialQueryNounFilterOption>,
    pub spec_values: Vec<SpatialQuerySpecValueFilterOption>,
    pub include_negative: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct SpatialQueryNounFilterOption {
    pub value: String,
    pub count: usize,
    pub is_negative: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct SpatialQuerySpecValueFilterOption {
    pub value: i64,
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct SpatialQueryResultItem {
    pub refno: String,
    pub noun: String,
    pub spec_value: i64,
    pub aabb: Option<AabbDto>,
    pub distance: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub within_radius: Option<bool>,
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

#[derive(Debug, Serialize, Clone)]
pub struct SpatialNearbyCenterDto {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refno: Option<String>,
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

#[derive(Debug, Deserialize)]
pub struct NearestClearanceQueryParams {
    /// Source mode: aabb/default, point, bran_centerline.
    pub source_mode: Option<String>,
    /// Source refno, accepts "dbnum/refno" and "dbnum_refno".
    pub source_refno: Option<String>,
    /// Point source coordinates in mm. Used only when source_refno is absent.
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub z: Option<f32>,
    /// Comma-separated target NOUN filters.
    pub target_nouns: Option<String>,
    /// Comma-separated shortcut groups. wall -> WALL,PANE,GWALL,STWALL; column -> COLU,SCTN,GENSEC.
    pub target_groups: Option<String>,
    /// Search radius in mm. Default 5000. Valid range: 0 < radius <= 100000.
    pub radius: Option<f32>,
    /// same_dbnum | all_loaded | explicit_dbnums
    pub scope: Option<String>,
    /// Comma-separated u32 dbnums when scope=explicit_dbnums.
    pub dbnums: Option<String>,
    /// Per resolved group result limit. Default 1, clamped to 1..100.
    pub max_per_group: Option<usize>,
    /// Include the source refno itself when it also matches target filters. Default false.
    pub include_self: Option<bool>,
    /// Include diagnostic counters.
    pub debug: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct NearestClearanceResponse {
    pub success: bool,
    pub source: Option<NearestClearanceSource>,
    pub distance_method: &'static str,
    pub unit: &'static str,
    pub query_bbox: Option<AabbDto>,
    pub resolved_filters: Option<NearestClearanceResolvedFilters>,
    pub nearest_by_group: Vec<NearestClearanceGroupResult>,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug: Option<NearestClearanceDebug>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct NearestClearanceSource {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refno: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dbnum: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub point: Option<Vec3Dto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aabb: Option<AabbDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub centerline_bbox: Option<AabbDto>,
}

#[derive(Debug, Serialize, Clone)]
pub struct NearestClearanceResolvedFilters {
    pub target_nouns: Vec<String>,
    pub target_groups: Vec<ResolvedTargetGroup>,
    pub scope: String,
    pub dbnums: Option<Vec<u32>>,
    pub radius: f32,
    pub max_per_group: usize,
    pub include_self: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct ResolvedTargetGroup {
    pub name: String,
    pub nouns: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct NearestClearanceGroupResult {
    pub group: String,
    pub nouns: Vec<String>,
    pub candidates: Vec<NearestClearanceCandidate>,
}

#[derive(Debug, Serialize, Clone)]
pub struct NearestClearanceCandidate {
    pub refno: String,
    pub noun: String,
    pub spec_value: i64,
    pub distance_mm: f32,
    pub intersects: bool,
    pub aabb: AabbDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nearest: Option<NearestClearanceNearest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotation: Option<NearestClearanceAnnotation>,
}

#[derive(Debug, Serialize, Clone)]
pub struct NearestClearanceNearest {
    pub source_segment_refno: String,
    pub source_segment_order: Option<u32>,
    pub source_point: Vec3Dto,
    pub target_point: Vec3Dto,
    pub vector: Vec3DeltaDto,
}

#[derive(Debug, Serialize, Clone)]
pub struct NearestClearanceAnnotation {
    pub start_point: Vec3Dto,
    pub end_point: Vec3Dto,
    pub label_mm: f32,
}

#[derive(Debug, Serialize, Clone)]
pub struct Vec3DeltaDto {
    pub dx: f32,
    pub dy: f32,
    pub dz: f32,
}

#[derive(Debug, Serialize, Default)]
pub struct NearestClearanceDebug {
    pub candidate_ids: usize,
    pub rows_examined: usize,
    pub rows_missing_items: usize,
    pub rows_missing_aabb: usize,
    pub scope_filtered: usize,
    pub noun_filtered: usize,
    pub distance_filtered: usize,
    pub groups_with_hits: usize,
    pub returned_candidates: usize,
}

#[derive(Debug, Clone)]
struct BranCenterlineSegment {
    refno: RefnoEnum,
    order: Option<u32>,
    start: Vec3,
    end: Vec3,
}

enum ClearanceSourceGeometry {
    Aabb(Aabb),
    BranCenterline(Vec<BranCenterlineSegment>),
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

async fn ensure_sqlite_spatial_surreal_context() -> anyhow::Result<()> {
    let db_option = aios_core::get_db_option();
    if aios_core::use_ns_db_compat(
        &aios_core::SUL_DB,
        &db_option.surreal_ns,
        &db_option.project_name,
    )
    .await
    .is_ok()
    {
        return Ok(());
    }
    match aios_core::connect_surdb(
        &db_option.surrealdb_conn_str(),
        &db_option.surreal_ns,
        &db_option.project_name,
        &db_option.surreal_user,
        &db_option.surreal_password,
    )
    .await
    {
        Ok(_) => {}
        Err(e) if e.to_string().contains("Already connected") => {}
        Err(e) => return Err(e.into()),
    }
    aios_core::use_ns_db_compat(
        &aios_core::SUL_DB,
        &db_option.surreal_ns,
        &db_option.project_name,
    )
    .await?;
    Ok(())
}

async fn fetch_bran_centerline_segments(
    branch_refno: RefnoEnum,
) -> anyhow::Result<Vec<BranCenterlineSegment>> {
    use aios_core::rs_surreal::geometry_query::PlantTransform;
    use aios_core::shape::pdms_shape::RsVec3;
    use aios_core::{SUL_DB, SurrealQueryExt};
    use serde::{Deserialize, Serialize};
    use surrealdb::types::SurrealValue;

    #[derive(Debug, Serialize, Deserialize, SurrealValue)]
    struct TubiRelateRow {
        pub leave_refno: RefnoEnum,
        #[serde(default)]
        pub world_trans: Option<PlantTransform>,
        #[serde(default)]
        pub start_pt: Option<RsVec3>,
        #[serde(default)]
        pub end_pt: Option<RsVec3>,
        #[serde(default)]
        pub index: Option<i64>,
    }

    ensure_sqlite_spatial_surreal_context().await?;

    let tubi_range = model_refno_range("tubi_relate", branch_refno);
    let sql = format!(
        r#"
        SELECT
            in as leave_refno,
            world_trans.d as world_trans,
            start_pt.d as start_pt,
            end_pt.d as end_pt,
            id[2] as index
        FROM {tubi_range};
        "#
    );
    let db_option = aios_core::get_db_option();
    let sql = format!(
        "USE NS `{}` DB `{}`;\n{}",
        db_option.surreal_ns, db_option.project_name, sql
    );
    let rows: Vec<TubiRelateRow> = SUL_DB.query_take(&sql, 1).await?;
    if rows.is_empty() {
        anyhow::bail!(
            "tubi_relate returned no segments for branch_refno={} pe_key={}",
            branch_refno,
            branch_refno.to_pe_key()
        );
    }

    let mut segments = Vec::with_capacity(rows.len());
    for row in rows {
        let wt = row.world_trans.unwrap_or_default();
        let matrix = wt.to_matrix();
        let start = row
            .start_pt
            .map(|p| p.0)
            .unwrap_or_else(|| matrix.transform_point3(Vec3::new(0.0, 0.0, 0.0)));
        let end = row
            .end_pt
            .map(|p| p.0)
            .unwrap_or_else(|| matrix.transform_point3(Vec3::new(0.0, 0.0, 1.0)));
        segments.push(BranCenterlineSegment {
            refno: row.leave_refno,
            order: row.index.and_then(|i| u32::try_from(i).ok()),
            start,
            end,
        });
    }
    segments.sort_by(|a, b| {
        a.order
            .unwrap_or(u32::MAX)
            .cmp(&b.order.unwrap_or(u32::MAX))
            .then_with(|| a.refno.to_string().cmp(&b.refno.to_string()))
    });

    Ok(segments)
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
    if matches!(
        mode.as_str(),
        "bran" | "branch" | "bran_centerline" | "branch_centerline" | "centerline"
    ) {
        return "bran_centerline";
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

fn negative_nouns() -> &'static HashSet<String> {
    static NEGATIVE_NOUNS: OnceLock<HashSet<String>> = OnceLock::new();
    NEGATIVE_NOUNS.get_or_init(|| {
        TOTAL_NEG_NOUN_NAMES
            .iter()
            .map(|name| name.trim().to_uppercase())
            .collect()
    })
}

fn is_negative_noun(noun: &str) -> bool {
    negative_nouns().contains(&noun.trim().to_uppercase())
}

fn error_spatial_query_result(
    error: impl Into<String>,
    query_bbox: Option<AabbDto>,
) -> SpatialQueryResult {
    SpatialQueryResult {
        success: false,
        results: None,
        center: None,
        radius: None,
        shape: None,
        truncated: None,
        total_count: None,
        returned_count: None,
        page: None,
        per_page: None,
        has_more: None,
        query_bbox,
        filter_options: None,
        error: Some(error.into()),
    }
}

fn empty_filter_options(include_negative: bool) -> SpatialQueryFilterOptions {
    SpatialQueryFilterOptions {
        nouns: vec![],
        spec_values: vec![],
        include_negative,
    }
}

fn record_filter_option(
    noun_counts: &mut BTreeMap<String, usize>,
    spec_value_counts: &mut BTreeMap<i64, usize>,
    noun: &str,
    spec_value: i64,
) {
    let normalized_noun = noun.trim().to_uppercase();
    let noun = if normalized_noun.is_empty() {
        "UNKNOWN".to_string()
    } else {
        normalized_noun
    };
    *noun_counts.entry(noun).or_insert(0) += 1;
    *spec_value_counts.entry(spec_value).or_insert(0) += 1;
}

fn build_filter_options_from_counts(
    noun_counts: BTreeMap<String, usize>,
    spec_value_counts: BTreeMap<i64, usize>,
    include_negative: bool,
) -> SpatialQueryFilterOptions {
    SpatialQueryFilterOptions {
        nouns: noun_counts
            .into_iter()
            .map(|(value, count)| SpatialQueryNounFilterOption {
                is_negative: is_negative_noun(&value),
                value,
                count,
            })
            .collect(),
        spec_values: spec_value_counts
            .into_iter()
            .map(|(value, count)| SpatialQuerySpecValueFilterOption { value, count })
            .collect(),
        include_negative,
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
    filter_options: Option<SpatialQueryFilterOptions>,
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
        center: None,
        radius: None,
        shape: None,
        truncated: Some(has_more),
        total_count: Some(total_count),
        returned_count: Some(returned_count),
        page: Some(page),
        per_page: Some(per_page),
        has_more: Some(has_more),
        query_bbox,
        filter_options,
        error: None,
    }
}

fn error_nearest_clearance_response(
    error: impl Into<String>,
    source: Option<NearestClearanceSource>,
    query_bbox: Option<AabbDto>,
    resolved_filters: Option<NearestClearanceResolvedFilters>,
    debug: Option<NearestClearanceDebug>,
    distance_method: &'static str,
) -> NearestClearanceResponse {
    NearestClearanceResponse {
        success: false,
        source,
        distance_method,
        unit: "mm",
        query_bbox,
        resolved_filters,
        nearest_by_group: vec![],
        warnings: vec![],
        debug,
        error: Some(error.into()),
    }
}

fn success_nearest_clearance_response(
    source: NearestClearanceSource,
    query_bbox: AabbDto,
    resolved_filters: NearestClearanceResolvedFilters,
    nearest_by_group: Vec<NearestClearanceGroupResult>,
    warnings: Vec<String>,
    debug: Option<NearestClearanceDebug>,
    distance_method: &'static str,
) -> NearestClearanceResponse {
    NearestClearanceResponse {
        success: true,
        source: Some(source),
        distance_method,
        unit: "mm",
        query_bbox: Some(query_bbox),
        resolved_filters: Some(resolved_filters),
        nearest_by_group,
        warnings,
        debug,
        error: None,
    }
}

fn parse_csv_upper(value: &Option<String>) -> Vec<String> {
    value
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(|item| item.trim().to_uppercase())
        .filter(|item| !item.is_empty())
        .collect()
}

fn parse_csv_dbnums(value: &Option<String>) -> Result<Vec<u32>, String> {
    let mut out = Vec::new();
    for raw in value.as_deref().unwrap_or("").split(',') {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let dbnum = raw
            .parse::<u32>()
            .map_err(|_| format!("invalid dbnum `{}`", raw))?;
        out.push(dbnum);
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

fn dbnum_from_refno_id(id: i64) -> u32 {
    ((id as u64) >> 32) as u32
}

fn normalize_refno_string(refno: &str) -> Option<String> {
    refno_str_to_i64(refno).map(i64_to_refno_str)
}

struct NearbyQueryPlan {
    params: SqliteSpatialQueryParams,
    center_hint: Option<SpatialNearbyCenterDto>,
    center_source: String,
    center_refno: Option<String>,
    radius: f32,
    shape: String,
}

fn normalize_nearby_shape(shape: &Option<String>, mode: &str) -> String {
    let value = shape.as_deref().unwrap_or("").trim();
    if value.is_empty() {
        default_shape_for_mode(mode).to_string()
    } else {
        value.to_ascii_lowercase()
    }
}

fn validate_nearby_radius(radius: Option<f32>) -> Result<f32, String> {
    let radius = radius.ok_or_else(|| "nearby requires radius".to_string())?;
    if radius.is_finite() && radius > 0.0 && radius <= MAX_CLEARANCE_RADIUS_MM {
        Ok(radius)
    } else {
        Err(format!(
            "invalid radius (must be 0 < radius <= {} mm)",
            MAX_CLEARANCE_RADIUS_MM
        ))
    }
}

fn prepare_nearby_query(mut params: SqliteSpatialQueryParams) -> Result<NearbyQueryPlan, String> {
    let radius = validate_nearby_radius(params.radius)?;
    let refno = params
        .refno
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    if let Some(refno) = refno {
        let normalized = normalize_refno_string(&refno)
            .ok_or_else(|| "invalid refno format (expected dbnum_refno)".to_string())?;
        let shape = normalize_nearby_shape(&params.shape, "refno");
        params.mode = Some("refno".to_string());
        params.refno = Some(normalized.clone());
        params.radius = Some(radius);
        params.distance = Some(radius);
        params.shape = Some(shape.clone());

        return Ok(NearbyQueryPlan {
            params,
            center_hint: None,
            center_source: "refno_aabb_center".to_string(),
            center_refno: Some(normalized),
            radius,
            shape,
        });
    }

    let (Some(x), Some(y), Some(z)) = (params.x, params.y, params.z) else {
        return Err("nearby requires refno or x/y/z".to_string());
    };
    if !(x.is_finite() && y.is_finite() && z.is_finite()) {
        return Err("nearby position contains non-finite value".to_string());
    }

    let shape = normalize_nearby_shape(&params.shape, "position");
    params.mode = Some("position".to_string());
    params.radius = Some(radius);
    params.distance = None;
    params.shape = Some(shape.clone());

    Ok(NearbyQueryPlan {
        params,
        center_hint: Some(SpatialNearbyCenterDto {
            x,
            y,
            z,
            source: "point_input".to_string(),
            refno: None,
        }),
        center_source: "point_input".to_string(),
        center_refno: None,
        radius,
        shape,
    })
}

fn query_bbox_center(
    bbox: &AabbDto,
    source: String,
    refno: Option<String>,
) -> SpatialNearbyCenterDto {
    SpatialNearbyCenterDto {
        x: (bbox.min.x + bbox.max.x) * 0.5,
        y: (bbox.min.y + bbox.max.y) * 0.5,
        z: (bbox.min.z + bbox.max.z) * 0.5,
        source,
        refno,
    }
}

fn with_nearby_metadata(
    mut result: SpatialQueryResult,
    center_hint: Option<SpatialNearbyCenterDto>,
    center_source: String,
    center_refno: Option<String>,
    radius: f32,
    shape: String,
) -> SpatialQueryResult {
    let center = center_hint.or_else(|| {
        result
            .query_bbox
            .as_ref()
            .map(|bbox| query_bbox_center(bbox, center_source, center_refno))
    });
    result.center = center;
    result.radius = Some(radius);
    result.shape = Some(shape);
    result
}

fn parse_clearance_source_mode(params: &NearestClearanceQueryParams) -> &'static str {
    let mode = params
        .source_mode
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match mode.as_str() {
        "point" => "point",
        "bran_centerline" | "branch_centerline" | "centerline" => "bran_centerline",
        "aabb" | "refno" | "" => {
            if params
                .source_refno
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty())
            {
                "aabb"
            } else {
                "point"
            }
        }
        _ => "invalid",
    }
}

fn parse_refno_enum_for_source(refno: &str) -> Result<RefnoEnum, String> {
    RefnoEnum::from_str(&refno.trim().replace('_', "/")).map_err(|e| {
        format!("invalid source_refno format (expected dbnum_refno or dbnum/refno): {e}")
    })
}

fn resolve_target_groups(
    params: &NearestClearanceQueryParams,
) -> Result<Vec<ResolvedTargetGroup>, String> {
    let mut groups = Vec::new();
    let mut seen = HashSet::new();

    for group in parse_csv_upper(&params.target_groups) {
        let (name, nouns): (String, Vec<String>) = match group.as_str() {
            "WALL" => (
                "wall".to_string(),
                vec![
                    "WALL".to_string(),
                    "PANE".to_string(),
                    "GWALL".to_string(),
                    "STWALL".to_string(),
                ],
            ),
            "COLUMN" => (
                "column".to_string(),
                vec!["COLU".to_string(), "SCTN".to_string(), "GENSEC".to_string()],
            ),
            _ => {
                return Err(format!("unknown target_group `{}`", group));
            }
        };
        if seen.insert(name.clone()) {
            groups.push(ResolvedTargetGroup { name, nouns });
        }
    }

    let target_nouns = parse_csv_upper(&params.target_nouns);
    if !target_nouns.is_empty() {
        let mut nouns = target_nouns;
        nouns.sort();
        nouns.dedup();
        if seen.insert("target_nouns".to_string()) {
            groups.push(ResolvedTargetGroup {
                name: "target_nouns".to_string(),
                nouns,
            });
        }
    }

    if groups.is_empty() {
        return Err(
            "at least one of target_nouns or target_groups must resolve to NOUN filters"
                .to_string(),
        );
    }

    Ok(groups)
}

fn resolve_clearance_radius(radius: Option<f32>) -> Result<f32, String> {
    let radius = radius.unwrap_or(DEFAULT_CLEARANCE_RADIUS_MM);
    if radius.is_finite() && radius > 0.0 && radius <= MAX_CLEARANCE_RADIUS_MM {
        Ok(radius)
    } else {
        Err(format!(
            "invalid radius (must be finite and 0 < radius <= {} mm)",
            MAX_CLEARANCE_RADIUS_MM
        ))
    }
}

fn resolve_clearance_max_per_group(max_per_group: Option<usize>) -> usize {
    max_per_group
        .unwrap_or(DEFAULT_CLEARANCE_MAX_PER_GROUP)
        .clamp(1, MAX_CLEARANCE_MAX_PER_GROUP)
}

fn resolve_scope(
    params: &NearestClearanceQueryParams,
    source_dbnum: Option<u32>,
) -> Result<(String, Option<Vec<u32>>), String> {
    let default_scope = if source_dbnum.is_some() {
        "same_dbnum"
    } else {
        "all_loaded"
    };
    let scope = params
        .scope
        .as_deref()
        .unwrap_or(default_scope)
        .trim()
        .to_lowercase();

    match scope.as_str() {
        "same_dbnum" => {
            let dbnum =
                source_dbnum.ok_or_else(|| "scope=same_dbnum requires source_refno".to_string())?;
            Ok((scope, Some(vec![dbnum])))
        }
        "all_loaded" => Ok((scope, None)),
        "explicit_dbnums" => {
            let dbnums = parse_csv_dbnums(&params.dbnums)?;
            if dbnums.is_empty() {
                Err("scope=explicit_dbnums requires non-empty dbnums".to_string())
            } else {
                Ok((scope, Some(dbnums)))
            }
        }
        _ => Err("invalid scope (expected same_dbnum, all_loaded, or explicit_dbnums)".to_string()),
    }
}

fn union_group_nouns(groups: &[ResolvedTargetGroup]) -> HashSet<String> {
    groups
        .iter()
        .flat_map(|group| group.nouns.iter().cloned())
        .collect()
}

fn dbnum_matches_scope(dbnum: Option<u32>, scope_dbnums: &Option<Vec<u32>>) -> bool {
    match scope_dbnums {
        Some(allowed) => dbnum.is_some_and(|dbnum| allowed.contains(&dbnum)),
        None => true,
    }
}

fn item_dbnum_or_id_dbnum(item_dbnum: Option<u32>, id: i64) -> u32 {
    item_dbnum.unwrap_or_else(|| dbnum_from_refno_id(id))
}

fn query_item_row(
    stmt: &mut rusqlite::Statement<'_>,
    id: i64,
) -> rusqlite::Result<Option<(String, i64, Option<u32>)>> {
    stmt.query_row([id], |r| {
        Ok((
            r.get::<_, Option<String>>(0)?
                .unwrap_or_else(|| "UNKNOWN".to_string()),
            r.get::<_, i64>(1).unwrap_or(0),
            r.get::<_, Option<u32>>(2).unwrap_or(None),
        ))
    })
    .optional()
}

fn query_aabb_row_dto(
    stmt: &mut rusqlite::Statement<'_>,
    id: i64,
) -> rusqlite::Result<Option<(Aabb, AabbDto)>> {
    stmt.query_row([id], |r| {
        let minx: f32 = r.get(0)?;
        let miny: f32 = r.get(1)?;
        let minz: f32 = r.get(2)?;
        let maxx: f32 = r.get(3)?;
        let maxy: f32 = r.get(4)?;
        let maxz: f32 = r.get(5)?;
        Ok((
            aabb_from_row(minx, miny, minz, maxx, maxy, maxz),
            aabb_dto_from_row(minx, miny, minz, maxx, maxy, maxz),
        ))
    })
    .optional()
}

fn candidate_sort_key_scope_rank(
    candidate: &NearestClearanceCandidate,
    scope_dbnums: &Option<Vec<u32>>,
) -> usize {
    let candidate_dbnum = refno_str_to_i64(&candidate.refno).map(dbnum_from_refno_id);
    match scope_dbnums {
        Some(dbnums) => candidate_dbnum
            .and_then(|dbnum| dbnums.iter().position(|allowed| *allowed == dbnum))
            .unwrap_or(usize::MAX),
        None => 0,
    }
}

fn centerline_segment_aabb(segment: &BranCenterlineSegment) -> Aabb {
    Aabb::new(
        [
            segment.start.x.min(segment.end.x),
            segment.start.y.min(segment.end.y),
            segment.start.z.min(segment.end.z),
        ]
        .into(),
        [
            segment.start.x.max(segment.end.x),
            segment.start.y.max(segment.end.y),
            segment.start.z.max(segment.end.z),
        ]
        .into(),
    )
}

fn centerline_bbox(segments: &[BranCenterlineSegment]) -> Option<Aabb> {
    let mut iter = segments.iter();
    let first = iter.next()?;
    let mut bbox = centerline_segment_aabb(first);
    for segment in iter {
        bbox.merge(&centerline_segment_aabb(segment));
    }
    Some(bbox)
}

fn point_aabb_distance(point: Vec3, aabb: &Aabb) -> f32 {
    let dx = if point.x < aabb.mins.x {
        aabb.mins.x - point.x
    } else if point.x > aabb.maxs.x {
        point.x - aabb.maxs.x
    } else {
        0.0
    };
    let dy = if point.y < aabb.mins.y {
        aabb.mins.y - point.y
    } else if point.y > aabb.maxs.y {
        point.y - aabb.maxs.y
    } else {
        0.0
    };
    let dz = if point.z < aabb.mins.z {
        aabb.mins.z - point.z
    } else if point.z > aabb.maxs.z {
        point.z - aabb.maxs.z
    } else {
        0.0
    };
    (dx * dx + dy * dy + dz * dz).sqrt()
}

fn clamp_point_to_aabb(point: Vec3, aabb: &Aabb) -> Vec3 {
    Vec3::new(
        point.x.clamp(aabb.mins.x, aabb.maxs.x),
        point.y.clamp(aabb.mins.y, aabb.maxs.y),
        point.z.clamp(aabb.mins.z, aabb.maxs.z),
    )
}

#[derive(Debug, Clone)]
struct CenterlineAabbNearest {
    source_segment_refno: String,
    source_segment_order: Option<u32>,
    source_point: Vec3,
    target_point: Vec3,
    vector: Vec3,
    distance_mm: f32,
    intersects: bool,
}

impl CenterlineAabbNearest {
    fn to_nearest_dto(&self) -> NearestClearanceNearest {
        NearestClearanceNearest {
            source_segment_refno: self.source_segment_refno.clone(),
            source_segment_order: self.source_segment_order,
            source_point: vec3_to_dto(self.source_point),
            target_point: vec3_to_dto(self.target_point),
            vector: Vec3DeltaDto {
                dx: self.vector.x,
                dy: self.vector.y,
                dz: self.vector.z,
            },
        }
    }

    fn to_annotation_dto(&self) -> NearestClearanceAnnotation {
        NearestClearanceAnnotation {
            start_point: vec3_to_dto(self.source_point),
            end_point: vec3_to_dto(self.target_point),
            label_mm: self.distance_mm,
        }
    }
}

fn vec3_to_dto(point: Vec3) -> Vec3Dto {
    Vec3Dto {
        x: point.x,
        y: point.y,
        z: point.z,
    }
}

fn refno_enum_to_output_refno(refno: &RefnoEnum) -> String {
    refno.to_string().replace('/', "_")
}

fn segment_aabb_intersection_t(start: Vec3, end: Vec3, aabb: &Aabb) -> Option<f32> {
    let dir = end - start;
    let mut t_min = 0.0_f32;
    let mut t_max = 1.0_f32;
    for (origin, delta, min, max) in [
        (start.x, dir.x, aabb.mins.x, aabb.maxs.x),
        (start.y, dir.y, aabb.mins.y, aabb.maxs.y),
        (start.z, dir.z, aabb.mins.z, aabb.maxs.z),
    ] {
        if delta.abs() <= f32::EPSILON {
            if origin < min || origin > max {
                return None;
            }
            continue;
        }
        let inv = 1.0 / delta;
        let mut t1 = (min - origin) * inv;
        let mut t2 = (max - origin) * inv;
        if t1 > t2 {
            std::mem::swap(&mut t1, &mut t2);
        }
        t_min = t_min.max(t1);
        t_max = t_max.min(t2);
        if t_min > t_max {
            return None;
        }
    }
    Some(t_min.clamp(0.0, 1.0))
}

fn segment_intersects_aabb(start: Vec3, end: Vec3, aabb: &Aabb) -> bool {
    segment_aabb_intersection_t(start, end, aabb).is_some()
}

fn segment_aabb_nearest(segment: &BranCenterlineSegment, aabb: &Aabb) -> CenterlineAabbNearest {
    if let Some(t) = segment_aabb_intersection_t(segment.start, segment.end, aabb) {
        let point = segment.start + (segment.end - segment.start) * t;
        return CenterlineAabbNearest {
            source_segment_refno: refno_enum_to_output_refno(&segment.refno),
            source_segment_order: segment.order,
            source_point: point,
            target_point: point,
            vector: Vec3::ZERO,
            distance_mm: 0.0,
            intersects: true,
        };
    }

    let axis = segment.end - segment.start;
    let len_sq = axis.length_squared();
    if len_sq <= f32::EPSILON {
        let target_point = clamp_point_to_aabb(segment.start, aabb);
        let vector = target_point - segment.start;
        return CenterlineAabbNearest {
            source_segment_refno: refno_enum_to_output_refno(&segment.refno),
            source_segment_order: segment.order,
            source_point: segment.start,
            target_point,
            vector,
            distance_mm: vector.length(),
            intersects: false,
        };
    }

    // Exact segment-to-AABB distance for a convex, piecewise-quadratic function.
    // The closest target point is the source point clamped to the box. The active clamped axes
    // only change when the segment crosses an AABB face, so evaluate each interval between face
    // crossings and the local minimum inside that interval.
    let mut best_source = segment.start;
    let mut best_target = clamp_point_to_aabb(segment.start, aabb);
    let mut best_distance_sq = (best_target - best_source).length_squared();

    let mut consider_t = |t: f32| {
        let source_point = segment.start + axis * t.clamp(0.0, 1.0);
        let target_point = clamp_point_to_aabb(source_point, aabb);
        let distance_sq = (target_point - source_point).length_squared();
        let best_key = (
            best_source.x.to_bits(),
            best_source.y.to_bits(),
            best_source.z.to_bits(),
        );
        let current_key = (
            source_point.x.to_bits(),
            source_point.y.to_bits(),
            source_point.z.to_bits(),
        );
        if distance_sq < best_distance_sq
            || ((distance_sq - best_distance_sq).abs() <= 1.0e-5 && current_key < best_key)
        {
            best_source = source_point;
            best_target = target_point;
            best_distance_sq = distance_sq;
        }
    };

    let mut breaks = vec![0.0_f32, 1.0_f32];
    for (start_axis, delta_axis, min_axis, max_axis) in [
        (segment.start.x, axis.x, aabb.mins.x, aabb.maxs.x),
        (segment.start.y, axis.y, aabb.mins.y, aabb.maxs.y),
        (segment.start.z, axis.z, aabb.mins.z, aabb.maxs.z),
    ] {
        if delta_axis.abs() <= f32::EPSILON {
            continue;
        }
        for plane in [min_axis, max_axis] {
            let t = (plane - start_axis) / delta_axis;
            if (0.0..=1.0).contains(&t) {
                breaks.push(t);
            }
        }
    }
    breaks.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    breaks.dedup_by(|a, b| (*a - *b).abs() <= 1.0e-6);

    for t in &breaks {
        consider_t(*t);
    }

    for window in breaks.windows(2) {
        let lo = window[0];
        let hi = window[1];
        if hi - lo <= 1.0e-6 {
            continue;
        }
        let mid = (lo + hi) * 0.5;
        let mid_point = segment.start + axis * mid;
        let mut numerator = 0.0_f32;
        let mut denominator = 0.0_f32;
        for (start_axis, delta_axis, point_axis, min_axis, max_axis) in [
            (
                segment.start.x,
                axis.x,
                mid_point.x,
                aabb.mins.x,
                aabb.maxs.x,
            ),
            (
                segment.start.y,
                axis.y,
                mid_point.y,
                aabb.mins.y,
                aabb.maxs.y,
            ),
            (
                segment.start.z,
                axis.z,
                mid_point.z,
                aabb.mins.z,
                aabb.maxs.z,
            ),
        ] {
            let bound = if point_axis < min_axis {
                Some(min_axis)
            } else if point_axis > max_axis {
                Some(max_axis)
            } else {
                None
            };
            if let Some(bound) = bound {
                numerator += delta_axis * (bound - start_axis);
                denominator += delta_axis * delta_axis;
            }
        }
        if denominator > f32::EPSILON {
            consider_t((numerator / denominator).clamp(lo, hi));
        }
    }

    let vector = best_target - best_source;
    CenterlineAabbNearest {
        source_segment_refno: refno_enum_to_output_refno(&segment.refno),
        source_segment_order: segment.order,
        source_point: best_source,
        target_point: best_target,
        vector,
        distance_mm: vector.length(),
        intersects: false,
    }
}

fn segment_aabb_distance(segment: &BranCenterlineSegment, aabb: &Aabb) -> f32 {
    segment_aabb_nearest(segment, aabb).distance_mm
}

fn centerline_aabb_nearest(
    candidate: &Aabb,
    centerline: &[BranCenterlineSegment],
) -> Option<CenterlineAabbNearest> {
    centerline
        .iter()
        .map(|segment| segment_aabb_nearest(segment, candidate))
        .min_by(|a, b| {
            a.distance_mm
                .partial_cmp(&b.distance_mm)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    a.source_segment_order
                        .unwrap_or(u32::MAX)
                        .cmp(&b.source_segment_order.unwrap_or(u32::MAX))
                })
                .then_with(|| a.source_segment_refno.cmp(&b.source_segment_refno))
                .then_with(|| {
                    (
                        a.source_point.x.to_bits(),
                        a.source_point.y.to_bits(),
                        a.source_point.z.to_bits(),
                    )
                        .cmp(&(
                            b.source_point.x.to_bits(),
                            b.source_point.y.to_bits(),
                            b.source_point.z.to_bits(),
                        ))
                })
        })
}

fn min_distance_to_centerline(candidate: &Aabb, centerline: &[BranCenterlineSegment]) -> f32 {
    centerline_aabb_nearest(candidate, centerline)
        .map(|nearest| nearest.distance_mm)
        .unwrap_or(f32::INFINITY)
}

// ============================================================================
// Handler：GET /api/sqlite-spatial/query
// ============================================================================

/// GET /api/sqlite-spatial/nearest-clearance
pub async fn api_sqlite_spatial_nearest_clearance(
    Query(params): Query<NearestClearanceQueryParams>,
) -> Json<NearestClearanceResponse> {
    let include_debug = params.debug.unwrap_or(false);
    let source_mode = parse_clearance_source_mode(&params);
    let distance_method = if source_mode == "bran_centerline" {
        "centerline_aabb_clearance_mm"
    } else {
        "aabb_clearance_mm"
    };
    if source_mode == "invalid" {
        return Json(error_nearest_clearance_response(
            "invalid source_mode (expected aabb, point, or bran_centerline)",
            None,
            None,
            None,
            include_debug.then(NearestClearanceDebug::default),
            distance_method,
        ));
    }
    if let Err(e) = resolve_target_groups(&params) {
        return Json(error_nearest_clearance_response(
            e,
            None,
            None,
            None,
            include_debug.then(NearestClearanceDebug::default),
            distance_method,
        ));
    }
    if let Err(e) = resolve_clearance_radius(params.radius) {
        return Json(error_nearest_clearance_response(
            e,
            None,
            None,
            None,
            include_debug.then(NearestClearanceDebug::default),
            distance_method,
        ));
    }

    let prepared_centerline = if source_mode == "bran_centerline" {
        match params
            .source_refno
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(source_refno) => match parse_refno_enum_for_source(source_refno) {
                Ok(branch_refno) => match fetch_bran_centerline_segments(branch_refno).await {
                    Ok(segments) => Some(segments),
                    Err(e) => {
                        return Json(error_nearest_clearance_response(
                            format!("fetch BRAN centerline failed: {e}"),
                            Some(NearestClearanceSource {
                                kind: "bran_centerline".to_string(),
                                refno: normalize_refno_string(source_refno),
                                dbnum: refno_str_to_i64(source_refno).map(dbnum_from_refno_id),
                                point: None,
                                aabb: None,
                                segment_count: None,
                                centerline_bbox: None,
                            }),
                            None,
                            None,
                            include_debug.then(NearestClearanceDebug::default),
                            distance_method,
                        ));
                    }
                },
                Err(e) => {
                    return Json(error_nearest_clearance_response(
                        e,
                        None,
                        None,
                        None,
                        include_debug.then(NearestClearanceDebug::default),
                        distance_method,
                    ));
                }
            },
            None => {
                return Json(error_nearest_clearance_response(
                    "source_mode=bran_centerline requires source_refno",
                    None,
                    None,
                    None,
                    include_debug.then(NearestClearanceDebug::default),
                    distance_method,
                ));
            }
        }
    } else {
        None
    };
    let result = tokio::task::spawn_blocking(move || {
        do_nearest_clearance_query(params, prepared_centerline)
    })
    .await;
    match result {
        Ok(r) => Json(r),
        Err(e) => Json(error_nearest_clearance_response(
            format!("internal error: {}", e),
            None,
            None,
            None,
            None,
            "aabb_clearance_mm",
        )),
    }
}

/// GET /api/sqlite-spatial/query
pub async fn api_sqlite_spatial_query(
    Query(params): Query<SqliteSpatialQueryParams>,
) -> Json<SpatialQueryResult> {
    let prepared_centerline = if parse_mode(&params) == "bran_centerline" {
        match params
            .refno
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(refno) => match parse_refno_enum_for_source(refno) {
                Ok(branch_refno) => match fetch_bran_centerline_segments(branch_refno).await {
                    Ok(segments) => Some(segments),
                    Err(e) => {
                        return Json(error_spatial_query_result(
                            format!("fetch BRAN centerline failed: {e}"),
                            None,
                        ));
                    }
                },
                Err(e) => {
                    return Json(error_spatial_query_result(e, None));
                }
            },
            None => {
                return Json(error_spatial_query_result(
                    "mode=bran_centerline requires refno",
                    None,
                ));
            }
        }
    } else {
        None
    };

    let fallback_refno_ids = match query_refno_visible_inst_ids_for_fallback(&params).await {
        Ok(ids) => ids,
        Err(e) => {
            return Json(error_spatial_query_result(e, None));
        }
    };

    // 将 SQLite 阻塞 I/O 放入 blocking 线程池
    let result = tokio::task::spawn_blocking(move || {
        do_spatial_query(params, fallback_refno_ids, prepared_centerline)
    })
    .await;
    match result {
        Ok(r) => Json(r),
        Err(e) => Json(error_spatial_query_result(
            format!("internal error: {}", e),
            None,
        )),
    }
}

/// GET /api/sqlite-spatial/nearby
pub async fn api_sqlite_spatial_nearby(
    Query(params): Query<SqliteSpatialQueryParams>,
) -> Json<SpatialQueryResult> {
    let plan = match prepare_nearby_query(params) {
        Ok(plan) => plan,
        Err(e) => return Json(error_spatial_query_result(e, None)),
    };

    let NearbyQueryPlan {
        params,
        center_hint,
        mut center_source,
        center_refno,
        radius,
        shape,
    } = plan;

    let fallback_refno_ids = match query_refno_visible_inst_ids_for_fallback(&params).await {
        Ok(ids) => ids,
        Err(e) => {
            return Json(with_nearby_metadata(
                error_spatial_query_result(e, None),
                center_hint,
                center_source,
                center_refno,
                radius,
                shape,
            ));
        }
    };

    if fallback_refno_ids.is_some() && center_source == "refno_aabb_center" {
        center_source = "visible_children_aabb_center".to_string();
    }

    let result =
        tokio::task::spawn_blocking(move || do_spatial_query(params, fallback_refno_ids, None))
            .await;
    let query_result = match result {
        Ok(r) => r,
        Err(e) => error_spatial_query_result(format!("internal error: {}", e), None),
    };

    Json(with_nearby_metadata(
        query_result,
        center_hint,
        center_source,
        center_refno,
        radius,
        shape,
    ))
}

fn do_nearest_clearance_query(
    params: NearestClearanceQueryParams,
    prepared_centerline: Option<Vec<BranCenterlineSegment>>,
) -> NearestClearanceResponse {
    let include_debug = params.debug.unwrap_or(false);
    let source_mode = parse_clearance_source_mode(&params);
    let distance_method = if source_mode == "bran_centerline" {
        "centerline_aabb_clearance_mm"
    } else {
        "aabb_clearance_mm"
    };
    if source_mode == "invalid" {
        return error_nearest_clearance_response(
            "invalid source_mode (expected aabb, point, or bran_centerline)",
            None,
            None,
            None,
            include_debug.then(NearestClearanceDebug::default),
            distance_method,
        );
    }
    let groups = match resolve_target_groups(&params) {
        Ok(groups) => groups,
        Err(e) => {
            return error_nearest_clearance_response(
                e,
                None,
                None,
                None,
                include_debug.then(NearestClearanceDebug::default),
                distance_method,
            );
        }
    };
    let all_target_nouns = union_group_nouns(&groups);
    let radius = match resolve_clearance_radius(params.radius) {
        Ok(radius) => radius,
        Err(e) => {
            return error_nearest_clearance_response(
                e,
                None,
                None,
                None,
                include_debug.then(NearestClearanceDebug::default),
                distance_method,
            );
        }
    };
    let max_per_group = resolve_clearance_max_per_group(params.max_per_group);
    let include_self = params.include_self.unwrap_or(false);

    let cached = match get_cached_index() {
        Ok(c) => c,
        Err(e) => {
            return error_nearest_clearance_response(
                format!("{}. 请先运行 import-spatial-index 构建索引。", e),
                None,
                None,
                None,
                include_debug.then(NearestClearanceDebug::default),
                distance_method,
            );
        }
    };

    let conn = match Connection::open(&cached.path) {
        Ok(c) => c,
        Err(e) => {
            return error_nearest_clearance_response(
                format!("open sqlite connection failed: {}", e),
                None,
                None,
                None,
                include_debug.then(NearestClearanceDebug::default),
                distance_method,
            );
        }
    };

    let source_refno = params
        .source_refno
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let mut self_ids = HashSet::new();
    let (source, source_geometry, source_dbnum) = if source_mode == "bran_centerline" {
        let Some(source_refno) = source_refno else {
            return error_nearest_clearance_response(
                "source_mode=bran_centerline requires source_refno",
                None,
                None,
                None,
                include_debug.then(NearestClearanceDebug::default),
                distance_method,
            );
        };
        let Some(id) = refno_str_to_i64(source_refno) else {
            return error_nearest_clearance_response(
                "invalid source_refno format (expected dbnum_refno or dbnum/refno)",
                None,
                None,
                None,
                include_debug.then(NearestClearanceDebug::default),
                distance_method,
            );
        };
        self_ids.insert(id);
        let Some(centerline) = prepared_centerline else {
            return error_nearest_clearance_response(
                "BRAN centerline was not prepared",
                None,
                None,
                None,
                include_debug.then(NearestClearanceDebug::default),
                distance_method,
            );
        };
        if centerline.is_empty() {
            return error_nearest_clearance_response(
                "BRAN centerline has no segments",
                None,
                None,
                None,
                include_debug.then(NearestClearanceDebug::default),
                distance_method,
            );
        }
        for segment in &centerline {
            if let Some(segment_id) = refno_str_to_i64(&refno_enum_to_output_refno(&segment.refno))
            {
                self_ids.insert(segment_id);
            }
        }
        let bbox = centerline_bbox(&centerline).expect("non-empty centerline has bbox");
        let source = NearestClearanceSource {
            kind: "bran_centerline".to_string(),
            refno: Some(i64_to_refno_str(id)),
            dbnum: Some(dbnum_from_refno_id(id)),
            point: None,
            aabb: None,
            segment_count: Some(centerline.len()),
            centerline_bbox: Some(aabb_to_dto(&bbox)),
        };
        (
            source,
            ClearanceSourceGeometry::BranCenterline(centerline),
            Some(dbnum_from_refno_id(id)),
        )
    } else if source_mode == "aabb" {
        let Some(source_refno) = source_refno else {
            return error_nearest_clearance_response(
                "source_mode=aabb requires source_refno",
                None,
                None,
                None,
                include_debug.then(NearestClearanceDebug::default),
                distance_method,
            );
        };
        let Some(id) = refno_str_to_i64(source_refno) else {
            return error_nearest_clearance_response(
                "invalid source_refno format (expected dbnum_refno or dbnum/refno)",
                None,
                None,
                None,
                include_debug.then(NearestClearanceDebug::default),
                distance_method,
            );
        };
        self_ids.insert(id);
        let row = match query_aabb_row(&conn, id) {
            Ok(row) => row,
            Err(e) => {
                return error_nearest_clearance_response(
                    format!("query source refno aabb failed: {}", e),
                    None,
                    None,
                    None,
                    include_debug.then(NearestClearanceDebug::default),
                    distance_method,
                );
            }
        };
        let Some((minx, miny, minz, maxx, maxy, maxz)) = row else {
            return error_nearest_clearance_response(
                "source_refno not found in aabb_index",
                Some(NearestClearanceSource {
                    kind: "refno".to_string(),
                    refno: normalize_refno_string(source_refno),
                    dbnum: Some(dbnum_from_refno_id(id)),
                    point: None,
                    aabb: None,
                    segment_count: None,
                    centerline_bbox: None,
                }),
                None,
                None,
                include_debug.then(NearestClearanceDebug::default),
                distance_method,
            );
        };
        let aabb = aabb_from_row(minx, miny, minz, maxx, maxy, maxz);
        let source = NearestClearanceSource {
            kind: "refno".to_string(),
            refno: Some(i64_to_refno_str(id)),
            dbnum: Some(dbnum_from_refno_id(id)),
            point: None,
            aabb: Some(aabb_to_dto(&aabb)),
            segment_count: None,
            centerline_bbox: None,
        };
        (
            source,
            ClearanceSourceGeometry::Aabb(aabb),
            Some(dbnum_from_refno_id(id)),
        )
    } else {
        let (Some(x), Some(y), Some(z)) = (params.x, params.y, params.z) else {
            return error_nearest_clearance_response(
                "missing source_refno or point source parameters (x, y, z)",
                None,
                None,
                None,
                include_debug.then(NearestClearanceDebug::default),
                distance_method,
            );
        };
        if !(x.is_finite() && y.is_finite() && z.is_finite()) {
            return error_nearest_clearance_response(
                "point source contains non-finite value",
                None,
                None,
                None,
                include_debug.then(NearestClearanceDebug::default),
                distance_method,
            );
        }
        let aabb = Aabb::new([x, y, z].into(), [x, y, z].into());
        let source = NearestClearanceSource {
            kind: "point".to_string(),
            refno: None,
            dbnum: None,
            point: Some(Vec3Dto { x, y, z }),
            aabb: Some(aabb_to_dto(&aabb)),
            segment_count: None,
            centerline_bbox: None,
        };
        (source, ClearanceSourceGeometry::Aabb(aabb), None)
    };

    let (scope, scope_dbnums) = match resolve_scope(&params, source_dbnum) {
        Ok(scope) => scope,
        Err(e) => {
            return error_nearest_clearance_response(
                e,
                Some(source),
                None,
                None,
                include_debug.then(NearestClearanceDebug::default),
                distance_method,
            );
        }
    };

    let resolved_filters = NearestClearanceResolvedFilters {
        target_nouns: {
            let mut nouns = all_target_nouns.iter().cloned().collect::<Vec<_>>();
            nouns.sort();
            nouns
        },
        target_groups: groups.clone(),
        scope: scope.clone(),
        dbnums: scope_dbnums.clone(),
        radius,
        max_per_group,
        include_self,
    };

    let query_result = match &source_geometry {
        ClearanceSourceGeometry::Aabb(source_aabb) => {
            let query_aabb = expand_aabb(source_aabb.clone(), radius);
            cached
                .idx
                .query_intersect(
                    query_aabb.mins.x as f64,
                    query_aabb.maxs.x as f64,
                    query_aabb.mins.y as f64,
                    query_aabb.maxs.y as f64,
                    query_aabb.mins.z as f64,
                    query_aabb.maxs.z as f64,
                )
                .map(|ids| (ids, query_aabb))
                .map_err(|e| format!("query_intersect failed: {e}"))
        }
        ClearanceSourceGeometry::BranCenterline(centerline) => {
            let segment_aabbs = centerline
                .iter()
                .map(centerline_segment_aabb)
                .collect::<Vec<_>>();
            query_ids_for_regions(cached, &segment_aabbs, radius)
                .map(|(ids, bbox)| (ids, bbox.expect("non-empty centerline query has bbox")))
        }
    };
    let (ids, query_aabb) = match query_result {
        Ok(value) => value,
        Err(e) => {
            return error_nearest_clearance_response(
                e,
                Some(source),
                None,
                Some(resolved_filters),
                include_debug.then(NearestClearanceDebug::default),
                distance_method,
            );
        }
    };
    let query_bbox = aabb_to_dto(&query_aabb);

    let mut debug = NearestClearanceDebug {
        candidate_ids: ids.len(),
        ..Default::default()
    };
    let mut stmt_item =
        match conn.prepare("SELECT noun, spec_value, dbnum FROM items WHERE id = ?1") {
            Ok(s) => s,
            Err(e) => {
                return error_nearest_clearance_response(
                    format!("prepare item stmt failed: {}", e),
                    Some(source),
                    Some(query_bbox),
                    Some(resolved_filters),
                    include_debug.then_some(debug),
                    distance_method,
                );
            }
        };
    let mut stmt_aabb = match conn
        .prepare("SELECT min_x, min_y, min_z, max_x, max_y, max_z FROM aabb_index WHERE id = ?1")
    {
        Ok(s) => s,
        Err(e) => {
            return error_nearest_clearance_response(
                format!("prepare aabb stmt failed: {}", e),
                Some(source),
                Some(query_bbox),
                Some(resolved_filters),
                include_debug.then_some(debug),
                distance_method,
            );
        }
    };

    let mut candidates = Vec::new();
    for id in ids {
        debug.rows_examined += 1;

        if !include_self && self_ids.contains(&id) {
            continue;
        }

        let item_row = match query_item_row(&mut stmt_item, id) {
            Ok(row) => row,
            Err(_) => None,
        };
        let Some((noun, spec_value, item_dbnum)) = item_row else {
            debug.rows_missing_items += 1;
            continue;
        };
        let noun_upper = noun.to_uppercase();
        let dbnum = item_dbnum_or_id_dbnum(item_dbnum, id);
        if !dbnum_matches_scope(Some(dbnum), &scope_dbnums) {
            debug.scope_filtered += 1;
            continue;
        }
        if !all_target_nouns.contains(&noun_upper) {
            debug.noun_filtered += 1;
            continue;
        }

        let aabb_row = match query_aabb_row_dto(&mut stmt_aabb, id) {
            Ok(row) => row,
            Err(_) => None,
        };
        let Some((candidate_aabb, candidate_aabb_dto)) = aabb_row else {
            debug.rows_missing_aabb += 1;
            continue;
        };

        let centerline_nearest = match &source_geometry {
            ClearanceSourceGeometry::Aabb(_) => None,
            ClearanceSourceGeometry::BranCenterline(centerline) => {
                centerline_aabb_nearest(&candidate_aabb, centerline)
            }
        };
        let distance = match (&source_geometry, &centerline_nearest) {
            (ClearanceSourceGeometry::Aabb(source_aabb), _) => {
                aabb_min_distance(source_aabb, &candidate_aabb)
            }
            (ClearanceSourceGeometry::BranCenterline(_), Some(nearest)) => nearest.distance_mm,
            (ClearanceSourceGeometry::BranCenterline(_), None) => f32::INFINITY,
        };
        if distance > radius {
            debug.distance_filtered += 1;
            continue;
        }

        let nearest = centerline_nearest
            .as_ref()
            .map(CenterlineAabbNearest::to_nearest_dto);
        let annotation = centerline_nearest
            .as_ref()
            .map(CenterlineAabbNearest::to_annotation_dto);

        let intersects = centerline_nearest
            .as_ref()
            .map(|nearest| nearest.intersects)
            .unwrap_or(distance == 0.0);

        candidates.push((
            noun_upper,
            NearestClearanceCandidate {
                refno: i64_to_refno_str(id),
                noun,
                spec_value,
                distance_mm: distance,
                intersects,
                aabb: candidate_aabb_dto,
                nearest,
                annotation,
            },
        ));
    }

    let mut nearest_by_group = Vec::new();
    let mut warnings = Vec::new();
    for group in groups.iter() {
        let group_nouns: HashSet<String> = group.nouns.iter().cloned().collect();
        let mut group_candidates = candidates
            .iter()
            .filter(|(noun, _candidate)| group_nouns.contains(noun))
            .map(|(_noun, candidate)| candidate.clone())
            .collect::<Vec<_>>();

        // 组内排序：距离 → scope 内 dbnum 优先级 → refno。
        // 分组之间互不比较（各组独立截断），不需要组间 tiebreaker。
        group_candidates.sort_by(|a, b| {
            a.distance_mm
                .partial_cmp(&b.distance_mm)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    candidate_sort_key_scope_rank(a, &scope_dbnums)
                        .cmp(&candidate_sort_key_scope_rank(b, &scope_dbnums))
                })
                .then_with(|| a.refno.cmp(&b.refno))
        });
        if group_candidates.is_empty() {
            warnings.push(format!("no targets found for group `{}`", group.name));
        }
        group_candidates.truncate(max_per_group);
        debug.returned_candidates += group_candidates.len();
        if !group_candidates.is_empty() {
            debug.groups_with_hits += 1;
        }
        nearest_by_group.push(NearestClearanceGroupResult {
            group: group.name.clone(),
            nouns: group.nouns.clone(),
            candidates: group_candidates,
        });
    }
    if nearest_by_group
        .iter()
        .all(|group| group.candidates.is_empty())
    {
        warnings.push("no targets found for any group".to_string());
    }

    success_nearest_clearance_response(
        source,
        query_bbox,
        resolved_filters,
        nearest_by_group,
        warnings,
        include_debug.then_some(debug),
        distance_method,
    )
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
    prepared_centerline: Option<Vec<BranCenterlineSegment>>,
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
    } else if mode == "bran_centerline" {
        let Some(centerline) = prepared_centerline else {
            return error_spatial_query_result("BRAN centerline was not prepared", None);
        };
        if centerline.is_empty() {
            return error_spatial_query_result("BRAN centerline has no segments", None);
        }
        let search_distance = normalized_search_distance(params.distance, params.radius);
        return query_by_target_geometry(
            params,
            cached,
            QueryTargetGeometry::BranCenterline(centerline),
            search_distance,
            self_id,
        );
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
        // 读库失败必须冒泡：吞成 None 会让「查询出错」伪装成「refno 不存在」，
        // 静默走 fallback 或返回空结果。
        let row = match query_aabb_row(&conn, id) {
            Ok(row) => row,
            Err(e) => {
                return error_spatial_query_result(format!("query refno aabb failed: {}", e), None);
            }
        };
        let Some((minx, miny, minz, maxx, maxy, maxz)) = row else {
            if let Some(ids) = fallback_refno_ids.as_deref() {
                match query_aabbs_for_ids(&conn, ids) {
                    Ok(aabbs) if !aabbs.is_empty() => {
                        let distance = normalized_search_distance(params.distance, params.radius);
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

    let distance = normalized_search_distance(params.distance, params.radius);
    query_by_target_aabbs(params, cached, vec![base_aabb], distance, self_id)
}

fn empty_spatial_query_result(params: &SqliteSpatialQueryParams) -> SpatialQueryResult {
    let (page, per_page) = resolve_pagination(params);
    SpatialQueryResult {
        success: true,
        results: Some(vec![]),
        center: None,
        radius: None,
        shape: None,
        truncated: Some(false),
        total_count: Some(0),
        returned_count: Some(0),
        page: Some(page),
        per_page: Some(per_page),
        has_more: Some(false),
        query_bbox: None,
        filter_options: Some(empty_filter_options(
            params.include_negative.unwrap_or(false),
        )),
        error: None,
    }
}

fn normalized_search_distance(distance: Option<f32>, radius: Option<f32>) -> f32 {
    let distance = distance.or(radius).unwrap_or(DEFAULT_DISTANCE);
    if distance.is_finite() && distance > 0.0 {
        distance
    } else {
        DEFAULT_DISTANCE
    }
}

fn default_shape_for_mode(mode: &str) -> &'static str {
    if mode == "refno" || mode == "position" || mode == "bran_centerline" {
        "sphere"
    } else {
        "cube"
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

enum QueryTargetGeometry {
    Aabbs(Vec<Aabb>),
    BranCenterline(Vec<BranCenterlineSegment>),
}

fn query_by_target_aabbs(
    params: SqliteSpatialQueryParams,
    cached: &CachedIndex,
    target_aabbs: Vec<Aabb>,
    search_distance: f32,
    self_id: Option<i64>,
) -> SpatialQueryResult {
    query_by_target_geometry(
        params,
        cached,
        QueryTargetGeometry::Aabbs(target_aabbs),
        search_distance,
        self_id,
    )
}

fn query_by_target_geometry(
    params: SqliteSpatialQueryParams,
    cached: &CachedIndex,
    target_geometry: QueryTargetGeometry,
    search_distance: f32,
    self_id: Option<i64>,
) -> SpatialQueryResult {
    let (page, per_page) = resolve_pagination(&params);
    let noun_filter = parse_noun_filter(&params.nouns);
    let spec_value_filter = parse_spec_value_filter(&params.spec_values);
    let include_negative = params.include_negative.unwrap_or(false);
    let preferred_db_prefix = refno_db_prefix(params.refno.as_deref());

    let query_regions = match &target_geometry {
        QueryTargetGeometry::Aabbs(target_aabbs) => target_aabbs.clone(),
        QueryTargetGeometry::BranCenterline(centerline) => centerline
            .iter()
            .map(centerline_segment_aabb)
            .collect::<Vec<_>>(),
    };

    if query_regions.is_empty() {
        return success_spatial_query_result(
            vec![],
            0,
            page,
            per_page,
            None,
            Some(empty_filter_options(include_negative)),
        );
    }

    // 球体模式：使用候选 AABB 到目标 AABB/点的最小距离做二次过滤。
    let mode = parse_mode(&params);
    let is_sphere = params
        .shape
        .as_deref()
        .unwrap_or(default_shape_for_mode(mode))
        .eq_ignore_ascii_case("sphere");
    let (ids, query_aabb) = match query_ids_for_regions(cached, &query_regions, search_distance) {
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
    let mut noun_option_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut spec_value_option_counts: BTreeMap<i64, usize> = BTreeMap::new();

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

        if !include_negative && is_negative_noun(&noun) {
            continue;
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
            let min_distance = match &target_geometry {
                QueryTargetGeometry::Aabbs(target_aabbs) => {
                    min_distance_to_targets(&candidate_aabb, target_aabbs)
                }
                QueryTargetGeometry::BranCenterline(centerline) => {
                    min_distance_to_centerline(&candidate_aabb, centerline)
                }
            };

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

        record_filter_option(
            &mut noun_option_counts,
            &mut spec_value_option_counts,
            &noun,
            spec_value,
        );

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

        let refno = i64_to_refno_str(id as i64);
        results.push(SpatialQueryResultItem {
            refno,
            noun,
            spec_value,
            aabb,
            distance,
            within_radius: distance.map(|value| value <= search_distance),
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

    let filter_options = build_filter_options_from_counts(
        noun_option_counts,
        spec_value_option_counts,
        include_negative,
    );

    success_spatial_query_result(
        page_results,
        total_count,
        page,
        per_page,
        query_bbox_dto,
        Some(filter_options),
    )
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

    fn rid(dbnum: u32, refno: u32) -> i64 {
        ((dbnum as u64) << 32 | refno as u64) as i64
    }

    fn base_nearest_params() -> NearestClearanceQueryParams {
        NearestClearanceQueryParams {
            source_mode: None,
            source_refno: None,
            x: None,
            y: None,
            z: None,
            target_nouns: None,
            target_groups: None,
            radius: None,
            scope: None,
            dbnums: None,
            max_per_group: None,
            include_self: None,
            debug: None,
        }
    }

    fn create_nearest_test_index(path: &std::path::Path) {
        let idx = SqliteAabbIndex::open(path).unwrap();
        idx.init_schema().unwrap();
        idx.insert_aabbs_with_items_and_spec_values(vec![
            (
                rid(1, 1),
                "EQUI".to_string(),
                0,
                0.0,
                10.0,
                0.0,
                10.0,
                0.0,
                10.0,
            ),
            (
                rid(1, 2),
                "PIPE".to_string(),
                7,
                11.0,
                12.0,
                0.0,
                10.0,
                0.0,
                10.0,
            ),
            (
                rid(1, 3),
                "WALL".to_string(),
                11,
                20.0,
                30.0,
                0.0,
                10.0,
                0.0,
                10.0,
            ),
            (
                rid(1, 4),
                "COLU".to_string(),
                13,
                50.0,
                60.0,
                0.0,
                10.0,
                0.0,
                10.0,
            ),
            (
                rid(1, 5),
                "PANE".to_string(),
                17,
                40.0,
                45.0,
                0.0,
                10.0,
                0.0,
                10.0,
            ),
            (
                rid(2, 3),
                "WALL".to_string(),
                19,
                13.0,
                14.0,
                0.0,
                10.0,
                0.0,
                10.0,
            ),
            (
                rid(2, 4),
                "COLU".to_string(),
                23,
                15.0,
                16.0,
                0.0,
                10.0,
                0.0,
                10.0,
            ),
            (
                rid(1, 6),
                "WALL".to_string(),
                29,
                5.0,
                15.0,
                0.0,
                10.0,
                0.0,
                10.0,
            ),
        ])
        .unwrap();
    }

    fn create_centerline_corridor_test_index(path: &std::path::Path) {
        let idx = SqliteAabbIndex::open(path).unwrap();
        idx.init_schema().unwrap();
        idx.insert_aabbs_with_items_and_spec_values(vec![
            (
                rid(1, 100),
                "BRAN".to_string(),
                0,
                -1.0,
                101.0,
                -1.0,
                101.0,
                -1.0,
                1.0,
            ),
            (
                rid(1, 201),
                "WALL".to_string(),
                11,
                20.0,
                22.0,
                4.0,
                6.0,
                -1.0,
                1.0,
            ),
            (
                rid(1, 202),
                "COLU".to_string(),
                13,
                94.0,
                96.0,
                80.0,
                82.0,
                -1.0,
                1.0,
            ),
            (
                rid(1, 203),
                "WALL".to_string(),
                17,
                50.0,
                52.0,
                50.0,
                52.0,
                -1.0,
                1.0,
            ),
        ])
        .unwrap();
    }

    fn centerline_fixture() -> Vec<BranCenterlineSegment> {
        vec![
            BranCenterlineSegment {
                refno: RefnoEnum::from("1/301"),
                order: Some(0),
                start: Vec3::new(0.0, 0.0, 0.0),
                end: Vec3::new(100.0, 0.0, 0.0),
            },
            BranCenterlineSegment {
                refno: RefnoEnum::from("1/302"),
                order: Some(1),
                start: Vec3::new(100.0, 0.0, 0.0),
                end: Vec3::new(100.0, 100.0, 0.0),
            },
        ]
    }

    fn candidates_for_group<'a>(
        resp: &'a NearestClearanceResponse,
        group: &str,
    ) -> &'a [NearestClearanceCandidate] {
        resp.nearest_by_group
            .iter()
            .find(|item| item.group == group)
            .map(|item| item.candidates.as_slice())
            .unwrap_or(&[])
    }

    fn base_spatial_bbox_params() -> SqliteSpatialQueryParams {
        SqliteSpatialQueryParams {
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
            maxx: Some(3.5),
            maxy: Some(1.5),
            maxz: Some(1.5),
            max_results: None,
            page: None,
            per_page: None,
            nouns: None,
            spec_values: None,
            include_self: None,
            include_negative: None,
            shape: None,
        }
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
                include_negative: None,
                shape: None,
            };
            do_spatial_query(params, None, None)
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
                include_negative: None,
                shape: None,
            };
            do_spatial_query(params, None, None)
        });
        assert!(resp.success);
        let items = resp.results.unwrap_or_default();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].spec_value, 0);
    }

    #[test]
    fn position_query_defaults_to_sphere_filter() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("spatial_index.sqlite");
        let idx = SqliteAabbIndex::open(&db).unwrap();
        idx.init_schema().unwrap();
        idx.insert_aabbs_with_items_and_spec_values(vec![
            (
                ((1u64 << 32) | 2u64) as i64,
                "PIPE".to_string(),
                0,
                9.0,
                10.0,
                0.0,
                1.0,
                0.0,
                1.0,
            ),
            (
                ((1u64 << 32) | 3u64) as i64,
                "PIPE".to_string(),
                0,
                8.0,
                9.0,
                8.0,
                9.0,
                0.0,
                1.0,
            ),
        ])
        .unwrap();

        let resp = with_test_index(&db, || {
            let params = SqliteSpatialQueryParams {
                mode: Some("position".to_string()),
                refno: None,
                x: Some(0.0),
                y: Some(0.0),
                z: Some(0.0),
                radius: Some(10.0),
                distance: None,
                minx: None,
                miny: None,
                minz: None,
                maxx: None,
                maxy: None,
                maxz: None,
                max_results: None,
                page: None,
                per_page: None,
                nouns: None,
                spec_values: None,
                include_self: None,
                include_negative: None,
                shape: None,
            };
            do_spatial_query(params, None, None)
        });

        assert!(resp.success);
        let items = resp.results.unwrap_or_default();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].refno, "1_2");
    }

    #[test]
    fn refno_query_defaults_to_sphere_filter_and_can_exclude_self() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("spatial_index.sqlite");
        let idx = SqliteAabbIndex::open(&db).unwrap();
        idx.init_schema().unwrap();
        idx.insert_aabbs_with_items_and_spec_values(vec![
            (
                ((1u64 << 32) | 1u64) as i64,
                "EQUI".to_string(),
                0,
                0.0,
                1.0,
                0.0,
                1.0,
                0.0,
                1.0,
            ),
            (
                ((1u64 << 32) | 2u64) as i64,
                "PIPE".to_string(),
                0,
                6.0,
                7.0,
                0.0,
                1.0,
                0.0,
                1.0,
            ),
            (
                ((1u64 << 32) | 3u64) as i64,
                "PIPE".to_string(),
                0,
                5.0,
                6.0,
                5.0,
                6.0,
                0.0,
                1.0,
            ),
        ])
        .unwrap();

        let resp = with_test_index(&db, || {
            let params = SqliteSpatialQueryParams {
                mode: Some("refno".to_string()),
                refno: Some("1_1".to_string()),
                x: None,
                y: None,
                z: None,
                radius: None,
                distance: Some(5.0),
                minx: None,
                miny: None,
                minz: None,
                maxx: None,
                maxy: None,
                maxz: None,
                max_results: None,
                page: None,
                per_page: None,
                nouns: None,
                spec_values: None,
                include_self: Some(false),
                include_negative: None,
                shape: None,
            };
            do_spatial_query(params, None, None)
        });

        assert!(resp.success);
        let items = resp.results.unwrap_or_default();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].refno, "1_2");
    }

    #[test]
    fn spatial_query_excludes_negative_nouns_by_default() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("spatial_index.sqlite");
        let idx = SqliteAabbIndex::open(&db).unwrap();
        idx.init_schema().unwrap();
        idx.insert_aabbs_with_items_and_spec_values(vec![
            (
                rid(1, 1),
                "PIPE".to_string(),
                1,
                0.0,
                1.0,
                0.0,
                1.0,
                0.0,
                1.0,
            ),
            (
                rid(1, 2),
                "NBOX".to_string(),
                1,
                2.0,
                3.0,
                0.0,
                1.0,
                0.0,
                1.0,
            ),
        ])
        .unwrap();

        let resp = with_test_index(&db, || {
            do_spatial_query(base_spatial_bbox_params(), None, None)
        });

        assert!(resp.success);
        let items = resp.results.unwrap_or_default();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].refno, "1_1");
        assert!(items.iter().all(|item| item.noun != "NBOX"));

        let filter_options = resp.filter_options.unwrap();
        assert!(!filter_options.include_negative);
        assert_eq!(filter_options.nouns.len(), 1);
        assert_eq!(filter_options.nouns[0].value, "PIPE");
        assert!(!filter_options.nouns[0].is_negative);
        assert!(filter_options.nouns.iter().all(|item| item.value != "NBOX"));
    }

    #[test]
    fn spatial_query_includes_negative_nouns_when_requested() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("spatial_index.sqlite");
        let idx = SqliteAabbIndex::open(&db).unwrap();
        idx.init_schema().unwrap();
        idx.insert_aabbs_with_items_and_spec_values(vec![
            (
                rid(1, 1),
                "PIPE".to_string(),
                1,
                0.0,
                1.0,
                0.0,
                1.0,
                0.0,
                1.0,
            ),
            (
                rid(1, 2),
                "NBOX".to_string(),
                2,
                2.0,
                3.0,
                0.0,
                1.0,
                0.0,
                1.0,
            ),
        ])
        .unwrap();

        let resp = with_test_index(&db, || {
            let mut params = base_spatial_bbox_params();
            params.nouns = Some("NBOX".to_string());
            params.spec_values = Some("2".to_string());
            params.include_negative = Some(true);
            do_spatial_query(params, None, None)
        });

        assert!(resp.success);
        let items = resp.results.unwrap_or_default();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].refno, "1_2");
        assert_eq!(items[0].noun, "NBOX");
        assert_eq!(items[0].spec_value, 2);

        let filter_options = resp.filter_options.unwrap();
        assert!(filter_options.include_negative);
        assert!(
            filter_options
                .nouns
                .iter()
                .any(|item| item.value == "NBOX" && item.is_negative)
        );
    }

    #[test]
    fn nearest_clearance_refno_source_returns_wall_and_column_groups() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("spatial_index.sqlite");
        create_nearest_test_index(&db);

        let resp = with_test_index(&db, || {
            let mut params = base_nearest_params();
            params.source_refno = Some("1/1".to_string());
            params.target_groups = Some("wall,column".to_string());
            params.radius = Some(100.0);
            do_nearest_clearance_query(params, None)
        });

        assert!(resp.success);
        assert_eq!(resp.distance_method, "aabb_clearance_mm");
        assert_eq!(resp.unit, "mm");
        let wall = candidates_for_group(&resp, "wall");
        let column = candidates_for_group(&resp, "column");
        assert_eq!(wall.len(), 1);
        assert_eq!(wall[0].refno, "1_6");
        assert_eq!(wall[0].distance_mm, 0.0);
        assert!(wall[0].intersects);
        assert_eq!(column.len(), 1);
        assert_eq!(column[0].refno, "1_4");
        assert_eq!(column[0].distance_mm, 40.0);
    }

    #[test]
    fn nearest_clearance_point_source_returns_nearest_targets() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("spatial_index.sqlite");
        create_nearest_test_index(&db);

        let resp = with_test_index(&db, || {
            let mut params = base_nearest_params();
            params.x = Some(0.0);
            params.y = Some(0.0);
            params.z = Some(0.0);
            params.target_nouns = Some("WALL".to_string());
            params.radius = Some(25.0);
            params.max_per_group = Some(2);
            do_nearest_clearance_query(params, None)
        });

        assert!(resp.success);
        let candidates = candidates_for_group(&resp, "target_nouns");
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].refno, "1_6");
        assert_eq!(candidates[1].refno, "2_3");
    }

    #[test]
    fn nearest_clearance_radius_changes_returned_set() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("spatial_index.sqlite");
        create_nearest_test_index(&db);

        let small = with_test_index(&db, || {
            let mut params = base_nearest_params();
            params.source_refno = Some("1_1".to_string());
            params.target_nouns = Some("WALL".to_string());
            params.radius = Some(5.0);
            params.max_per_group = Some(10);
            do_nearest_clearance_query(params, None)
        });
        let large = with_test_index(&db, || {
            let mut params = base_nearest_params();
            params.source_refno = Some("1_1".to_string());
            params.target_nouns = Some("WALL".to_string());
            params.radius = Some(25.0);
            params.max_per_group = Some(10);
            do_nearest_clearance_query(params, None)
        });

        assert!(small.success);
        assert!(large.success);
        assert_eq!(candidates_for_group(&small, "target_nouns").len(), 1);
        assert_eq!(candidates_for_group(&large, "target_nouns").len(), 2);
    }

    #[test]
    fn nearest_clearance_noun_filter_excludes_nearer_wrong_noun() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("spatial_index.sqlite");
        create_nearest_test_index(&db);

        let resp = with_test_index(&db, || {
            let mut params = base_nearest_params();
            params.source_refno = Some("1_1".to_string());
            params.target_nouns = Some("WALL".to_string());
            params.radius = Some(20.0);
            do_nearest_clearance_query(params, None)
        });

        assert!(resp.success);
        let candidates = candidates_for_group(&resp, "target_nouns");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].noun, "WALL");
        assert_eq!(candidates[0].refno, "1_6");
        assert_ne!(candidates[0].refno, "1_2");
    }

    #[test]
    fn nearest_clearance_honors_same_dbnum_and_explicit_dbnums_scope() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("spatial_index.sqlite");
        create_nearest_test_index(&db);

        let same_db = with_test_index(&db, || {
            let mut params = base_nearest_params();
            params.source_refno = Some("1_1".to_string());
            params.target_nouns = Some("COLU".to_string());
            params.radius = Some(100.0);
            do_nearest_clearance_query(params, None)
        });
        let explicit_db = with_test_index(&db, || {
            let mut params = base_nearest_params();
            params.source_refno = Some("1_1".to_string());
            params.target_nouns = Some("COLU".to_string());
            params.radius = Some(100.0);
            params.scope = Some("explicit_dbnums".to_string());
            params.dbnums = Some("2".to_string());
            do_nearest_clearance_query(params, None)
        });

        assert!(same_db.success);
        assert!(explicit_db.success);
        assert_eq!(
            candidates_for_group(&same_db, "target_nouns")[0].refno,
            "1_4"
        );
        assert_eq!(
            candidates_for_group(&explicit_db, "target_nouns")[0].refno,
            "2_4"
        );
    }

    #[test]
    fn nearest_clearance_source_refno_not_found_returns_false() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("spatial_index.sqlite");
        create_nearest_test_index(&db);

        let resp = with_test_index(&db, || {
            let mut params = base_nearest_params();
            params.source_refno = Some("1_999".to_string());
            params.target_groups = Some("wall".to_string());
            do_nearest_clearance_query(params, None)
        });

        assert!(!resp.success);
        assert_eq!(
            resp.error.as_deref(),
            Some("source_refno not found in aabb_index")
        );
    }

    #[test]
    fn nearest_clearance_intersecting_aabb_distance_zero_and_intersects_true() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("spatial_index.sqlite");
        create_nearest_test_index(&db);

        let resp = with_test_index(&db, || {
            let mut params = base_nearest_params();
            params.source_refno = Some("1_1".to_string());
            params.target_nouns = Some("WALL".to_string());
            params.radius = Some(1.0);
            do_nearest_clearance_query(params, None)
        });

        assert!(resp.success);
        let candidates = candidates_for_group(&resp, "target_nouns");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].refno, "1_6");
        assert_eq!(candidates[0].distance_mm, 0.0);
        assert!(candidates[0].intersects);
    }

    #[test]
    fn segment_aabb_distance_handles_intersecting_near_and_far() {
        let segment = BranCenterlineSegment {
            refno: RefnoEnum::from("1/301"),
            order: Some(0),
            start: Vec3::new(0.0, 0.0, 0.0),
            end: Vec3::new(10.0, 0.0, 0.0),
        };

        let intersecting = Aabb::new([4.0, -1.0, -1.0].into(), [6.0, 1.0, 1.0].into());
        let near = Aabb::new([4.0, 3.0, -1.0].into(), [6.0, 5.0, 1.0].into());
        let far = Aabb::new([20.0, 0.0, 0.0].into(), [21.0, 1.0, 1.0].into());

        assert_eq!(segment_aabb_distance(&segment, &intersecting), 0.0);
        assert_eq!(segment_aabb_distance(&segment, &near), 3.0);
        assert_eq!(segment_aabb_distance(&segment, &far), 10.0);
    }

    #[test]
    fn nearest_clearance_bran_centerline_corridor_excludes_far_whole_aabb_hit() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("spatial_index.sqlite");
        create_centerline_corridor_test_index(&db);

        let resp = with_test_index(&db, || {
            let mut params = base_nearest_params();
            params.source_mode = Some("bran_centerline".to_string());
            params.source_refno = Some("1_100".to_string());
            params.target_nouns = Some("WALL".to_string());
            params.radius = Some(8.0);
            params.max_per_group = Some(10);
            do_nearest_clearance_query(params, Some(centerline_fixture()))
        });

        assert!(resp.success);
        assert_eq!(resp.distance_method, "centerline_aabb_clearance_mm");
        let source = resp.source.as_ref().unwrap();
        assert_eq!(source.kind, "bran_centerline");
        assert_eq!(source.segment_count, Some(2));

        let candidates = candidates_for_group(&resp, "target_nouns");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].refno, "1_201");
        assert_eq!(candidates[0].distance_mm, 4.0);
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.refno != "1_203"),
            "candidate inside whole BRAN AABB but far from centerline must be excluded"
        );
    }

    #[test]
    fn nearest_clearance_bran_centerline_supports_wall_and_column_groups() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("spatial_index.sqlite");
        create_centerline_corridor_test_index(&db);

        let resp = with_test_index(&db, || {
            let mut params = base_nearest_params();
            params.source_mode = Some("bran_centerline".to_string());
            params.source_refno = Some("1_100".to_string());
            params.target_groups = Some("wall,column".to_string());
            params.radius = Some(8.0);
            do_nearest_clearance_query(params, Some(centerline_fixture()))
        });

        assert!(resp.success);
        let wall = candidates_for_group(&resp, "wall");
        let column = candidates_for_group(&resp, "column");
        assert_eq!(wall.len(), 1);
        assert_eq!(wall[0].refno, "1_201");
        assert_eq!(column.len(), 1);
        assert_eq!(column[0].refno, "1_202");
    }
}
