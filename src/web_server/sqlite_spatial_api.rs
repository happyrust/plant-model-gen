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

use axum::{extract::Query, response::Json};
use parry3d::bounding_volume::Aabb;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
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
    /// 最大返回数量（默认 5000，硬上限 10000）
    pub max_results: Option<usize>,
    /// noun 过滤（逗号分隔，如 "EQUI,PIPE,TUBI"，空表示不过滤）
    pub nouns: Option<String>,
    /// 是否包含自身（mode=refno 时有效，默认 true）
    pub include_self: Option<bool>,
    /// 查询形状："cube"（默认）| "sphere"（球体，会对结果做距离二次过滤）
    pub shape: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SpatialQueryResult {
    pub success: bool,
    pub results: Option<Vec<SpatialQueryResultItem>>,
    /// 是否因 max_results 截断
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
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

// ============================================================================
// Handler：GET /api/sqlite-spatial/query
// ============================================================================

/// GET /api/sqlite-spatial/query
pub async fn api_sqlite_spatial_query(
    Query(params): Query<SqliteSpatialQueryParams>,
) -> Json<SpatialQueryResult> {
    // 将 SQLite 阻塞 I/O 放入 blocking 线程池
    let result = tokio::task::spawn_blocking(move || do_spatial_query(params)).await;
    match result {
        Ok(r) => Json(r),
        Err(e) => Json(SpatialQueryResult {
            success: false,
            results: None,
            truncated: None,
            query_bbox: None,
            error: Some(format!("internal error: {}", e)),
        }),
    }
}

fn do_spatial_query(params: SqliteSpatialQueryParams) -> SpatialQueryResult {
    let cached = match get_cached_index() {
        Ok(c) => c,
        Err(e) => {
            return SpatialQueryResult {
                success: false,
                results: None,
                truncated: None,
                query_bbox: None,
                error: Some(format!("{}. 请先运行 import-spatial-index 构建索引。", e)),
            };
        }
    };
    let idx = &cached.idx;

    let mode = parse_mode(&params);
    let distance = params.distance.unwrap_or(DEFAULT_DISTANCE);
    let max_results = params
        .max_results
        .unwrap_or(DEFAULT_MAX_HITS)
        .min(HARD_MAX_HITS);
    let noun_filter = parse_noun_filter(&params.nouns);
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
                    return SpatialQueryResult {
                        success: false,
                        results: None,
                        truncated: None,
                        query_bbox: None,
                        error: Some(format!(
                            "invalid position or radius (must be 0 < radius <= {} mm)",
                            MAX_QUERY_RADIUS
                        )),
                    };
                }
                Aabb::new([x - r, y - r, z - r].into(), [x + r, y + r, z + r].into())
            }
            _ => {
                return SpatialQueryResult {
                    success: false,
                    results: None,
                    truncated: None,
                    query_bbox: None,
                    error: Some("missing position parameters (x, y, z, radius)".to_string()),
                };
            }
        }
    } else if mode == "refno" {
        let refno = params.refno.as_deref().unwrap_or("").trim();
        if refno.is_empty() {
            return SpatialQueryResult {
                success: false,
                results: None,
                truncated: None,
                query_bbox: None,
                error: Some("missing refno".to_string()),
            };
        }
        let Some(id) = refno_str_to_i64(refno) else {
            return SpatialQueryResult {
                success: false,
                results: None,
                truncated: None,
                query_bbox: None,
                error: Some("invalid refno format (expected dbnum_refno)".to_string()),
            };
        };
        // 查询该 refno 的 bbox（使用独立连接避免长期占用）
        let conn = match Connection::open(&cached.path) {
            Ok(c) => c,
            Err(e) => {
                return SpatialQueryResult {
                    success: false,
                    results: None,
                    truncated: None,
                    query_bbox: None,
                    error: Some(format!("open sqlite connection failed: {}", e)),
                };
            }
        };
        let row: Option<(f32, f32, f32, f32, f32, f32)> = conn
            .query_row(
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
            .unwrap_or(None);
        let Some((minx, miny, minz, maxx, maxy, maxz)) = row else {
            return SpatialQueryResult {
                success: true,
                results: Some(vec![]),
                truncated: Some(false),
                query_bbox: None,
                error: None,
            };
        };
        Aabb::new([minx, miny, minz].into(), [maxx, maxy, maxz].into())
    } else {
        match aabb_from_bbox_params(&params) {
            Ok(v) => v,
            Err(e) => {
                return SpatialQueryResult {
                    success: false,
                    results: None,
                    truncated: None,
                    query_bbox: None,
                    error: Some(e),
                };
            }
        }
    };

    let query_aabb = expand_aabb(base_aabb, distance);
    let query_bbox_dto = aabb_to_dto(&query_aabb);

    // 计算查询中心点（用于距离计算）
    let query_center_x = (query_aabb.mins.x + query_aabb.maxs.x) * 0.5;
    let query_center_y = (query_aabb.mins.y + query_aabb.maxs.y) * 0.5;
    let query_center_z = (query_aabb.mins.z + query_aabb.maxs.z) * 0.5;

    // 球体模式：用于二次距离过滤
    let is_sphere = params
        .shape
        .as_deref()
        .unwrap_or("cube")
        .eq_ignore_ascii_case("sphere");
    let sphere_radius = (query_aabb.maxs.x - query_aabb.mins.x)
        .max((query_aabb.maxs.y - query_aabb.mins.y).max(query_aabb.maxs.z - query_aabb.mins.z))
        * 0.5;
    let sphere_radius_sq = sphere_radius * sphere_radius;

    let ids = match idx.query_intersect(
        query_aabb.mins.x as f64,
        query_aabb.maxs.x as f64,
        query_aabb.mins.y as f64,
        query_aabb.maxs.y as f64,
        query_aabb.mins.z as f64,
        query_aabb.maxs.z as f64,
    ) {
        Ok(v) => v,
        Err(e) => {
            return SpatialQueryResult {
                success: false,
                results: None,
                truncated: None,
                query_bbox: Some(query_bbox_dto),
                error: Some(format!("query_intersect failed: {}", e)),
            };
        }
    };

    // 打开连接获取 noun 和 aabb 信息（使用 prepared statements 批量查询）
    let conn = match Connection::open(&cached.path) {
        Ok(c) => c,
        Err(e) => {
            return SpatialQueryResult {
                success: false,
                results: None,
                truncated: None,
                query_bbox: Some(query_bbox_dto),
                error: Some(format!("open sqlite connection failed: {}", e)),
            };
        }
    };

    let mut stmt_item = match conn.prepare("SELECT noun, spec_value FROM items WHERE id = ?1") {
        Ok(s) => s,
        Err(e) => {
            return SpatialQueryResult {
                success: false,
                results: None,
                truncated: None,
                query_bbox: Some(query_bbox_dto),
                error: Some(format!("prepare item stmt failed: {}", e)),
            };
        }
    };
    let mut stmt_aabb = match conn
        .prepare("SELECT min_x, min_y, min_z, max_x, max_y, max_z FROM aabb_index WHERE id = ?1")
    {
        Ok(s) => s,
        Err(e) => {
            return SpatialQueryResult {
                success: false,
                results: None,
                truncated: None,
                query_bbox: Some(query_bbox_dto),
                error: Some(format!("prepare aabb stmt failed: {}", e)),
            };
        }
    };

    let mut results: Vec<SpatialQueryResultItem> = Vec::with_capacity(ids.len().min(1024));
    let mut truncated = false;

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

        // 计算距离（AABB 中心到查询中心的欧氏距离）
        let distance = if let Some((minx, miny, minz, maxx, maxy, maxz)) = aabb_row {
            let cx = (minx + maxx) * 0.5;
            let cy = (miny + maxy) * 0.5;
            let cz = (minz + maxz) * 0.5;
            let dx = cx - query_center_x;
            let dy = cy - query_center_y;
            let dz = cz - query_center_z;
            let dist_sq = dx * dx + dy * dy + dz * dz;

            // 球体模式：基于距离做精确过滤
            if is_sphere && dist_sq > sphere_radius_sq {
                continue;
            }

            Some(dist_sq.sqrt())
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

        if results.len() >= max_results {
            truncated = true;
            break;
        }
    }

    // 按距离从近到远排序
    results.sort_by(|a, b| match (a.distance, b.distance) {
        (Some(da), Some(db)) => da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });

    SpatialQueryResult {
        success: true,
        results: Some(results),
        truncated: Some(truncated),
        query_bbox: Some(query_bbox_dto),
        error: None,
    }
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
                nouns: None,
                include_self: None,
                shape: None,
            };
            do_spatial_query(params)
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
                nouns: None,
                include_self: None,
                shape: None,
            };
            do_spatial_query(params)
        });
        assert!(resp.success);
        let items = resp.results.unwrap_or_default();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].spec_value, 0);
    }
}
