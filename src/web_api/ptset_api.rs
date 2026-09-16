use aios_core::parsed_data::CateAxisParam;
use aios_core::vec3_pool::{CateAxisParamCompact, decompress_ptset};
use aios_core::{RefnoEnum, SurrealQueryExt, project_primary_db};
use axum::{
    Json, Router,
    extract::{Path, Query, rejection::JsonRejection},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::str::FromStr;
use surrealdb::types::SurrealValue;

pub fn create_ptset_routes() -> Router {
    Router::new()
        .route(
            "/api/pdms/ptset/children/{refno}",
            get(get_ptset_children_by_owner),
        )
        .route("/api/pdms/ptset/{refno}", get(get_ptset_by_refno))
        .route("/api/pdms/ptset/batch-query", post(post_ptset_batch_query))
}

// 定义用于接收压缩格式 ptset 的结构
#[derive(Debug, Deserialize, SurrealValue)]
struct CompressedPtsetQueryResult {
    pub ptset: Option<Vec<CateAxisParamCompact>>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct InstRelatePtsetQueryResult {
    pub refno: RefnoEnum,
    pub ptset: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct TransformQueryResult {
    world_trans: Option<serde_json::Value>,
}

/// 单个轴点的信息（用于前端展示）
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PtsetPoint {
    /// 点编号
    pub number: i32,
    /// 3D 坐标 [x, y, z]
    pub pt: [f32; 3],
    /// 方向向量 [x, y, z]（可选）
    pub dir: Option<[f32; 3]>,
    /// 方向标志
    pub dir_flag: f32,
    /// 参考方向 [x, y, z]（可选）
    pub ref_dir: Option<[f32; 3]>,
    /// 管道外径
    pub pbore: f32,
    /// 宽度
    pub pwidth: f32,
    /// 高度
    pub pheight: f32,
    /// 连接信息
    pub pconnect: String,
}

impl From<&CateAxisParam> for PtsetPoint {
    fn from(param: &CateAxisParam) -> Self {
        PtsetPoint {
            number: param.number,
            pt: [param.pt.x, param.pt.y, param.pt.z],
            dir: param.dir.as_ref().map(|d| [d.x, d.y, d.z]),
            dir_flag: param.dir_flag,
            ref_dir: param.ref_dir.as_ref().map(|d| [d.x, d.y, d.z]),
            pbore: param.pbore,
            pwidth: param.pwidth,
            pheight: param.pheight,
            pconnect: param.pconnect.clone(),
        }
    }
}

/// ptset 查询响应
#[derive(Debug, Serialize, Deserialize)]
pub struct PtsetResponse {
    pub success: bool,
    pub refno: String,
    /// 点集数据，键为点编号
    pub ptset: Vec<PtsetPoint>,
    /// 世界坐标变换矩阵（4x4 列主序，16 个数字）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub world_transform: Option<Vec<f64>>,
    /// 对应 model instance_cache 的“快照版本”（通常取 meta_{dbno}.json 中的 batch_id）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<String>,
    /// 数据单位信息
    pub unit_info: Option<PtsetUnitInfo>,
    pub error_message: Option<String>,
}

/// 单位信息
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PtsetUnitInfo {
    /// 源单位（后端数据单位）
    pub source_unit: String,
    /// 目标单位（前端期望单位）
    pub target_unit: String,
    /// 转换因子：源单位 * factor = 目标单位
    /// 例如：mm -> dm，factor = 0.01
    pub conversion_factor: f64,
}

#[derive(Debug, Deserialize)]
pub struct PtsetQuery {
    /// dbno（可选；若不传则尝试从 output/scene_tree/db_meta_info.json 推导）
    pub dbno: Option<u32>,
    /// model instance_cache 的 batch_id（可选；若不传则默认按 latest）
    pub batch_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PtsetBatchQueryRequest {
    pub refnos: Vec<String>,
    pub dbno: Option<u32>,
    pub batch_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PtsetBatchItemResult {
    pub input_refno: String,
    pub refno: Option<String>,
    pub success: bool,
    pub ptset: Vec<PtsetPoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub world_transform: Option<Vec<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<String>,
    pub unit_info: Option<PtsetUnitInfo>,
    pub error_message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PtsetBatchQueryResponse {
    pub success: bool,
    pub results: Vec<PtsetBatchItemResult>,
    pub total_count: usize,
    pub success_count: usize,
    pub failed_count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PtsetChildrenResponse {
    pub success: bool,
    pub refno: String,
    pub results: Vec<PtsetBatchItemResult>,
    pub total_count: usize,
    pub success_count: usize,
    pub failed_count: usize,
    pub error_message: Option<String>,
}

#[derive(Debug)]
struct PtsetLookupResult {
    success: bool,
    refno: String,
    ptset: Vec<PtsetPoint>,
    world_transform: Option<Vec<f64>>,
    batch_id: Option<String>,
    unit_info: Option<PtsetUnitInfo>,
    error_message: Option<String>,
}

impl From<PtsetLookupResult> for PtsetResponse {
    fn from(value: PtsetLookupResult) -> Self {
        Self {
            success: value.success,
            refno: value.refno,
            ptset: value.ptset,
            world_transform: value.world_transform,
            batch_id: value.batch_id,
            unit_info: value.unit_info,
            error_message: value.error_message,
        }
    }
}

/// 从 inst_info 表查询 ptset 数据
async fn get_ptset_by_refno(
    Path(refno): Path<RefnoEnum>,
    Query(query): Query<PtsetQuery>,
) -> Result<Json<PtsetResponse>, StatusCode> {
    let result = query_ptset(refno, query.dbno, query.batch_id.as_deref()).await;
    Ok(Json(result.into()))
}

async fn get_ptset_children_by_owner(
    Path(refno): Path<RefnoEnum>,
    Query(query): Query<PtsetQuery>,
) -> Result<Json<PtsetChildrenResponse>, StatusCode> {
    let refno_str = refno.to_string();
    let lookup = query_child_ptsets(refno, query.dbno, query.batch_id.as_deref()).await;
    let results = match lookup {
        Ok(results) => results,
        Err(error_message) => {
            return Ok(Json(PtsetChildrenResponse {
                success: false,
                refno: refno_str,
                results: vec![],
                total_count: 0,
                success_count: 0,
                failed_count: 0,
                error_message: Some(error_message),
            }));
        }
    };
    let total_count = results.len();
    let success_count = results.iter().filter(|item| item.success).count();
    let failed_count = total_count.saturating_sub(success_count);

    Ok(Json(PtsetChildrenResponse {
        success: success_count > 0,
        refno: refno_str,
        results,
        total_count,
        success_count,
        failed_count,
        error_message: (success_count == 0).then(|| "未找到子元件 ptset 数据".to_string()),
    }))
}

async fn post_ptset_batch_query(
    request: Result<Json<PtsetBatchQueryRequest>, JsonRejection>,
) -> Result<Json<PtsetBatchQueryResponse>, StatusCode> {
    let Json(request) = request.map_err(|_| StatusCode::BAD_REQUEST)?;
    let PtsetBatchQueryRequest {
        refnos,
        dbno,
        batch_id,
    } = request;

    let mut results = Vec::with_capacity(refnos.len());
    let mut success_count = 0usize;

    for input_refno in refnos {
        let item = match parse_batch_refno(&input_refno) {
            Ok(refno) => {
                let lookup = query_ptset(refno, dbno, batch_id.as_deref()).await;
                lookup_to_batch_item(input_refno, lookup)
            }
            Err(error) => PtsetBatchItemResult {
                input_refno,
                refno: None,
                success: false,
                ptset: vec![],
                world_transform: None,
                batch_id: None,
                unit_info: None,
                error_message: Some(format!("无效的 refno: {error}")),
            },
        };

        if item.success {
            success_count += 1;
        }
        results.push(item);
    }

    let total_count = results.len();
    let failed_count = total_count.saturating_sub(success_count);

    Ok(Json(PtsetBatchQueryResponse {
        success: true,
        results,
        total_count,
        success_count,
        failed_count,
    }))
}

async fn query_ptset(
    refno: RefnoEnum,
    dbno: Option<u32>,
    batch_id: Option<&str>,
) -> PtsetLookupResult {
    let refno_str = refno.to_string();

    let db_lookup = query_ptset_from_db(&refno_str).await;
    if db_lookup.success {
        return db_lookup;
    }

    // 真源优先走 SurrealDB；仅在 DB 明确“无 ptset 数据”时尝试读取 cache 作为兼容兜底。
    if matches!(
        db_lookup.error_message.as_deref(),
        Some("未找到 ptset 数据")
    ) {
        if let Ok(Some((ptset_points, world_transform, snapshot_batch_id))) =
            try_get_ptset_from_cache(refno, dbno, batch_id).await
        {
            if !ptset_points.is_empty() {
                return PtsetLookupResult {
                    success: true,
                    refno: refno_str,
                    ptset: ptset_points,
                    world_transform: Some(world_transform),
                    batch_id: Some(snapshot_batch_id),
                    unit_info: Some(default_ptset_unit_info()),
                    error_message: None,
                };
            }
        }
    }

    db_lookup
}

async fn query_ptset_from_db(refno_str: &str) -> PtsetLookupResult {
    let refno = match RefnoEnum::from_str(&refno_str.replace('_', "/")) {
        Ok(value) => value,
        Err(error) => {
            return failure_lookup_result(refno_str.to_string(), format!("无效 refno: {error}"));
        }
    };
    let pe_key = refno.to_pe_key();
    let sql = format!(
        r#"
        SELECT
            in as refno,
            out.ptset as ptset
        FROM inst_relate
        WHERE in = {pe_key}
            AND out != NONE
        LIMIT 1
        "#
    );

    let rows: Vec<InstRelatePtsetQueryResult> = match project_primary_db().query_take(&sql, 0).await
    {
        Ok(result) => result,
        Err(error) => {
            return failure_lookup_result(
                refno_str.to_string(),
                format!("数据库查询失败: {error}"),
            );
        }
    };

    let Some(row) = rows.into_iter().next() else {
        return failure_lookup_result(refno_str.to_string(), "未找到 ptset 数据".to_string());
    };

    let ptset_points = parse_ptset_points(row.ptset);

    if ptset_points.is_empty() {
        return failure_lookup_result(refno_str.to_string(), "未找到 ptset 数据".to_string());
    }

    PtsetLookupResult {
        success: true,
        refno: row.refno.to_string(),
        ptset: ptset_points,
        world_transform: query_world_transform(row.refno).await,
        batch_id: None,
        unit_info: Some(default_ptset_unit_info()),
        error_message: None,
    }
}

fn lookup_to_batch_item(input_refno: String, lookup: PtsetLookupResult) -> PtsetBatchItemResult {
    PtsetBatchItemResult {
        input_refno,
        refno: Some(lookup.refno),
        success: lookup.success,
        ptset: lookup.ptset,
        world_transform: lookup.world_transform,
        batch_id: lookup.batch_id,
        unit_info: lookup.unit_info,
        error_message: lookup.error_message,
    }
}

fn failure_lookup_result(refno: String, error_message: String) -> PtsetLookupResult {
    PtsetLookupResult {
        success: false,
        refno,
        ptset: vec![],
        world_transform: None,
        batch_id: None,
        unit_info: None,
        error_message: Some(error_message),
    }
}

fn default_ptset_unit_info() -> PtsetUnitInfo {
    PtsetUnitInfo {
        source_unit: "mm".to_string(),
        target_unit: "mm".to_string(),
        conversion_factor: 1.0,
    }
}

fn parse_batch_refno(raw: &str) -> Result<RefnoEnum, String> {
    let normalized = raw.trim().replace('_', "/");
    let parts: Vec<&str> = normalized.split('/').collect();
    if parts.len() != 2 || parts.iter().any(|part| part.trim().is_empty()) {
        return Err("格式应为 dbnum_refno 或 dbnum/refno".to_string());
    }

    if parts[0].parse::<u32>().is_err() || parts[1].parse::<u32>().is_err() {
        return Err("格式应为 dbnum_refno 或 dbnum/refno".to_string());
    }

    RefnoEnum::from_str(&normalized).map_err(|error| error.to_string())
}

fn parse_ptset_points(value: Option<serde_json::Value>) -> Vec<PtsetPoint> {
    let Some(value) = value else {
        return vec![];
    };
    let value = match value {
        serde_json::Value::Array(mut values) if values.len() == 1 && values[0].is_array() => {
            values.remove(0)
        }
        value => value,
    };

    let mut points = match serde_json::from_value::<Vec<CateAxisParamCompact>>(value.clone()) {
        Ok(compact) => decompress_ptset(&compact)
            .iter()
            .map(PtsetPoint::from)
            .collect::<Vec<_>>(),
        Err(_) => serde_json::from_value::<Vec<CateAxisParam>>(value)
            .unwrap_or_default()
            .iter()
            .map(PtsetPoint::from)
            .collect::<Vec<_>>(),
    };
    points.sort_by_key(|p| p.number);
    points
}

async fn query_child_ptsets(
    owner_refno: RefnoEnum,
    _dbno: Option<u32>,
    batch_id: Option<&str>,
) -> Result<Vec<PtsetBatchItemResult>, String> {
    let owner_key = owner_refno.to_pe_key();
    let sql = format!(
        r#"
        SELECT
            in as refno,
            out.ptset as ptset
        FROM inst_relate
        WHERE owner_refno = {owner_key}
            AND out != NONE
        ORDER BY in
        "#
    );

    let rows: Vec<InstRelatePtsetQueryResult> = project_primary_db()
        .query_take(&sql, 0)
        .await
        .map_err(|error| format!("数据库查询失败: {error}"))?;

    let snapshot_id = batch_id.map(|value| value.to_string());
    let mut results = Vec::with_capacity(rows.len());
    for row in rows {
        let ptset = parse_ptset_points(row.ptset);
        let success = !ptset.is_empty();
        results.push(PtsetBatchItemResult {
            input_refno: row.refno.to_string(),
            refno: Some(row.refno.to_string()),
            success,
            ptset,
            world_transform: query_world_transform(row.refno).await,
            batch_id: snapshot_id.clone(),
            unit_info: success.then(default_ptset_unit_info),
            error_message: (!success).then(|| "未找到 ptset 数据".to_string()),
        });
    }

    Ok(results)
}

async fn query_world_transform(refno: RefnoEnum) -> Option<Vec<f64>> {
    let pe_transform_key = refno.to_pe_key().replace("pe:", "pe_transform:");
    let sql = format!(
        "SELECT world_trans.d as world_trans FROM {pe_transform_key} WHERE world_trans != none"
    );

    project_primary_db()
        .query_take::<Option<TransformQueryResult>>(&sql, 0)
        .await
        .ok()
        .flatten()
        .and_then(|row| parse_transform_matrix(row.world_trans))
}

fn parse_transform_matrix(trans: Option<serde_json::Value>) -> Option<Vec<f64>> {
    let trans = trans?;

    if let Some(obj) = trans.as_object() {
        if let Some(d) = obj.get("d")
            && let Some(arr) = d.as_array()
            && arr.len() == 16
        {
            return Some(arr.iter().filter_map(|v| v.as_f64()).collect());
        }

        if let (Some(t), Some(r), Some(s)) = (
            obj.get("translation").and_then(|v| v.as_array()),
            obj.get("rotation").and_then(|v| v.as_array()),
            obj.get("scale").and_then(|v| v.as_array()),
        ) && t.len() >= 3
            && r.len() >= 4
            && s.len() >= 3
        {
            return Some(compose_transform_matrix(
                [
                    t[0].as_f64().unwrap_or(0.0),
                    t[1].as_f64().unwrap_or(0.0),
                    t[2].as_f64().unwrap_or(0.0),
                ],
                [
                    r[0].as_f64().unwrap_or(0.0),
                    r[1].as_f64().unwrap_or(0.0),
                    r[2].as_f64().unwrap_or(0.0),
                    r[3].as_f64().unwrap_or(1.0),
                ],
                [
                    s[0].as_f64().unwrap_or(1.0),
                    s[1].as_f64().unwrap_or(1.0),
                    s[2].as_f64().unwrap_or(1.0),
                ],
            ));
        }
    }

    if let Some(arr) = trans.as_array()
        && arr.len() == 16
    {
        return Some(arr.iter().filter_map(|v| v.as_f64()).collect());
    }

    None
}

fn compose_transform_matrix(
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
    let yz = qy * y2;
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

async fn try_get_ptset_from_cache(
    refno: RefnoEnum,
    dbno: Option<u32>,
    batch_id: Option<&str>,
) -> anyhow::Result<Option<(Vec<PtsetPoint>, Vec<f64>, String)>> {
    use crate::data_interface::db_meta_manager::db_meta;
    use crate::fast_model::instance_cache::InstanceCacheManager;

    let dbnum = if let Some(dbno) = dbno {
        dbno
    } else {
        if db_meta().ensure_loaded().is_err() {
            return Ok(None);
        }
        db_meta().get_dbnum_by_refno(refno).unwrap_or(0)
    };
    if dbnum == 0 {
        return Ok(None);
    }

    // 运行时约定：默认读 output/instance_cache；也可由 MODEL_CACHE_DIR 指定。
    let cache_dir = std::env::var("MODEL_CACHE_DIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(default_model_cache_dir);

    let cache = InstanceCacheManager::new(&cache_dir).await?;

    // per-refno 存储：直接按 refno 读取，无需 batch 遍历。
    // batch_id 参数在 per-refno 模式下不再有意义（每个 refno 只有一条记录），保留签名兼容。
    let snapshot_id = batch_id
        .map(|b| b.to_string())
        .unwrap_or_else(|| "latest".to_string());

    let Some(cached) = cache.get_inst_info(dbnum, refno).await else {
        return Ok(None);
    };

    let mut points: Vec<PtsetPoint> = cached
        .info
        .ptset_map
        .values()
        .map(PtsetPoint::from)
        .collect();
    points.sort_by_key(|p| p.number);

    let m = cached
        .info
        .get_ele_world_transform()
        .to_matrix()
        .to_cols_array();
    let world_transform: Vec<f64> = m.iter().map(|v| *v as f64).collect();

    Ok(Some((points, world_transform, snapshot_id)))
}

fn default_model_cache_dir() -> PathBuf {
    let project_cache_dir = PathBuf::from("output/AvevaMarineSample/instance_cache");
    if project_cache_dir.exists() {
        return project_cache_dir;
    }
    PathBuf::from("output/instance_cache")
}
