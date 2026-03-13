use aios_core::parsed_data::CateAxisParam;
use aios_core::shape::pdms_shape::RsVec3;
use aios_core::vec3_pool::{CateAxisParamCompact, decompress_ptset};
use aios_core::{RefnoEnum, SurrealQueryExt, project_primary_db};
use axum::{Router, extract::Path, extract::Query, http::StatusCode, response::Json, routing::get};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use surrealdb::types::{self as surrealdb_types, SurrealValue};

pub fn create_ptset_routes() -> Router {
    Router::new().route("/api/pdms/ptset/{refno}", get(get_ptset_by_refno))
}

// 定义用于接收压缩格式 ptset 的结构
#[derive(Debug, Deserialize, SurrealValue)]
struct CompressedPtsetQueryResult {
    pub ptset: Option<Vec<CateAxisParamCompact>>,
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
#[derive(Debug, Serialize, Deserialize)]
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

/// 从 inst_info 表查询 ptset 数据
async fn get_ptset_by_refno(
    Path(refno): Path<RefnoEnum>,
    Query(query): Query<PtsetQuery>,
) -> Result<Json<PtsetResponse>, StatusCode> {
    let refno_str = refno.to_string();

    // 1) 优先从 model instance_cache 按需读取（与模型 instances.json 同源）
    //    - 若传入 dbno/batch_id：按“截至 batch 的最新快照”查找
    //    - 若未传：尝试推导 dbno 并回退到 latest batch
    if let Ok(Some((ptset_points, world_transform, snapshot_batch_id))) =
        try_get_ptset_from_cache(refno, query.dbno, query.batch_id.as_deref()).await
    {
        if !ptset_points.is_empty() {
            return Ok(Json(PtsetResponse {
                success: true,
                refno: refno_str,
                ptset: ptset_points,
                world_transform: Some(world_transform),
                batch_id: Some(snapshot_batch_id),
                // 当前约定：ptset 坐标与模型原始单位一致（mm）
                unit_info: Some(PtsetUnitInfo {
                    source_unit: "mm".to_string(),
                    target_unit: "mm".to_string(),
                    conversion_factor: 1.0,
                }),
                error_message: None,
            }));
        }
    }

    // 使用 SurrealDB 的关系查询语法：
    // inst_relate:24383_84631->inst_info 返回关联的 inst_info 记录
    let sql = format!(
        "SELECT ptset FROM inst_relate:{}->inst_info",
        refno.to_string()
    );

    // 使用 query_take 获取压缩格式的 ptset 数据
    let query_result: Option<CompressedPtsetQueryResult> =
        match project_primary_db().query_take(&sql, 0).await {
            Ok(result) => result,
            Err(e) => {
                return Ok(Json(PtsetResponse {
                    success: false,
                    refno: refno_str,
                    ptset: vec![],
                    world_transform: None,
                    batch_id: None,
                    unit_info: None,
                    error_message: Some(format!("数据库查询失败: {}", e)),
                }));
            }
        };

    // 处理查询结果：解压缩 ptset 数据
    let ptset_points: Vec<PtsetPoint> = match query_result {
        Some(result) => {
            if let Some(compacts) = result.ptset {
                // 解压缩
                let params = decompress_ptset(&compacts);
                params
                    .into_iter()
                    .map(|param| PtsetPoint::from(&param))
                    .collect()
            } else {
                vec![]
            }
        }
        None => vec![],
    };

    if !ptset_points.is_empty() {
        Ok(Json(PtsetResponse {
            success: true,
            refno: refno_str,
            ptset: ptset_points,
            world_transform: None,
            batch_id: None,
            unit_info: Some(PtsetUnitInfo {
                source_unit: "mm".to_string(),
                target_unit: "mm".to_string(),
                conversion_factor: 1.0,
            }),
            error_message: None,
        }))
    } else {
        Ok(Json(PtsetResponse {
            success: false,
            refno: refno_str,
            ptset: vec![],
            world_transform: None,
            batch_id: None,
            unit_info: None,
            error_message: Some("未找到 ptset 数据".to_string()),
        }))
    }
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
        .unwrap_or_else(|| PathBuf::from("output/instance_cache"));

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
