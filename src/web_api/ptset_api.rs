use aios_core::{RefnoEnum, SUL_DB, SurrealQueryExt};
use aios_core::parsed_data::CateAxisParam;
use aios_core::shape::pdms_shape::RsVec3;
use aios_core::vec3_pool::{decompress_ptset, CateAxisParamCompact};
use axum::{
    Router,
    extract::Path,
    http::StatusCode,
    response::Json,
    routing::get,
};
use serde::{Deserialize, Serialize};
use surrealdb::types::{self as surrealdb_types, SurrealValue};

pub fn create_ptset_routes() -> Router {
    Router::new()
        .route("/api/pdms/ptset/{refno}", get(get_ptset_by_refno))
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

/// 从 inst_info 表查询 ptset 数据
async fn get_ptset_by_refno(
    Path(refno): Path<RefnoEnum>,
) -> Result<Json<PtsetResponse>, StatusCode> {
    let refno_str = refno.to_string();

    // 使用 SurrealDB 的关系查询语法：
    // inst_relate:24383_84631->inst_info 返回关联的 inst_info 记录
    let sql = format!(
        "SELECT ptset FROM inst_relate:{}->inst_info",
        refno.to_string()
    );

    // 使用 query_take 获取压缩格式的 ptset 数据
    let query_result: Option<CompressedPtsetQueryResult> = match SUL_DB.query_take(&sql, 0).await {
        Ok(result) => result,
        Err(e) => {
            return Ok(Json(PtsetResponse {
                success: false,
                refno: refno_str,
                ptset: vec![],
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
            unit_info: Some(PtsetUnitInfo {
                source_unit: "mm".to_string(),
                target_unit: "dm".to_string(),
                conversion_factor: 0.01, // mm -> dm
            }),
            error_message: None,
        }))
    } else {
        Ok(Json(PtsetResponse {
            success: false,
            refno: refno_str,
            ptset: vec![],
            unit_info: None,
            error_message: Some("未找到 ptset 数据".to_string()),
        }))
    }
}
