use aios_core::{RefnoEnum, SUL_DB, SurrealQueryExt, get_named_attmap, get_type_name};
use surrealdb::types::SurrealValue;
use axum::{
    Router,
    extract::Path,
    http::StatusCode,
    response::Json,
    routing::get,
};
use serde::{Deserialize, Serialize};

pub fn create_pdms_transform_routes() -> Router {
    Router::new()
        .route("/api/pdms/transform/{refno}", get(get_transform))
        .route("/api/pdms/transform/compute/{refno}", get(compute_transform))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TransformResponse {
    pub success: bool,
    pub refno: String,
    /// 世界变换矩阵 (4x4 列主序)
    pub world_transform: Option<Vec<f64>>,
    /// Owner refno
    pub owner: Option<String>,
    pub error_message: Option<String>,
}

/// 查询元素的变换矩阵和 owner
async fn get_transform(Path(refno): Path<RefnoEnum>) -> Result<Json<TransformResponse>, StatusCode> {
    let refno_str = refno.to_string();
    let pe_transform_key = refno.to_pe_key().replace("pe:", "pe_transform:");

    // 查询 pe_transform 获取 world_trans
    let sql = format!(
        r#"
        SELECT 
            world_trans.d as world_trans
        FROM {}
        WHERE world_trans != none
        "#,
        pe_transform_key
    );

    #[derive(Deserialize, SurrealValue)]
    struct TransformQueryResult {
        world_trans: Option<serde_json::Value>,
    }

    match SUL_DB.query_take::<Option<TransformQueryResult>>(&sql, 0).await {
        Ok(Some(result)) => {
            // 解析变换矩阵
            let world_transform = parse_transform_matrix(result.world_trans);
            let owner_sql = format!(
                "SELECT record::id(owner ?? id) as owner FROM {}",
                refno.to_pe_key()
            );

            #[derive(Deserialize, SurrealValue)]
            struct OwnerQueryResult {
                owner: Option<String>,
            }

            let owner = match SUL_DB.query_take::<Option<OwnerQueryResult>>(&owner_sql, 0).await {
                Ok(Some(result)) => result.owner,
                _ => None,
            };

            Ok(Json(TransformResponse {
                success: true,
                refno: refno_str,
                world_transform,
                owner,
                error_message: None,
            }))
        }
        Ok(None) => {
            Ok(Json(TransformResponse {
                success: false,
                refno: refno_str,
                world_transform: None,
                owner: None,
                error_message: Some("未找到变换矩阵数据".to_string()),
            }))
        }
        Err(e) => {
            Ok(Json(TransformResponse {
                success: false,
                refno: refno_str,
                world_transform: None,
                owner: None,
                error_message: Some(format!("数据库查询失败: {}", e)),
            }))
        }
    }
}

/// 实时计算变换的响应
#[derive(Debug, Serialize, Deserialize)]
pub struct ComputeTransformResponse {
    pub success: bool,
    pub refno: String,
    pub noun: String,
    pub owner_refno: Option<String>,
    pub owner_noun: Option<String>,
    /// 本地变换矩阵位移 (x, y, z)
    pub local_translation: Option<[f64; 3]>,
    /// 世界变换矩阵位移 (x, y, z)
    pub world_translation: Option<[f64; 3]>,
    /// 世界变换矩阵 (4x4 列主序)
    pub world_transform: Option<Vec<f64>>,
    /// 元素关键属性（用于诊断）
    pub attrs: serde_json::Value,
    pub error_message: Option<String>,
}

/// 实时计算元素的 local/world 变换矩阵（不走缓存）
async fn compute_transform(
    Path(refno): Path<RefnoEnum>,
) -> Result<Json<ComputeTransformResponse>, StatusCode> {
    let refno_str = refno.to_string();

    // 获取属性
    let att = match get_named_attmap(refno).await {
        Ok(a) => a,
        Err(e) => {
            return Ok(Json(ComputeTransformResponse {
                success: false,
                refno: refno_str,
                noun: String::new(),
                owner_refno: None,
                owner_noun: None,
                local_translation: None,
                world_translation: None,
                world_transform: None,
                attrs: serde_json::Value::Null,
                error_message: Some(format!("get_named_attmap 失败: {e}")),
            }));
        }
    };

    let cur_type = att.get_type_str().to_string();
    let owner_refno = att.get_owner();
    let owner_noun = get_type_name(owner_refno).await.unwrap_or_default();

    // 收集诊断属性
    let mut diag = serde_json::Map::new();
    if let Some(pos) = att.get_position() {
        diag.insert("POS".into(), serde_json::json!([pos.x, pos.y, pos.z]));
    }
    if let Some(posl) = att.get_str("POSL") {
        diag.insert("POSL".into(), serde_json::json!(posl));
    }
    if let Some(pkdi) = att.get_f64("PKDI") {
        diag.insert("PKDI".into(), serde_json::json!(pkdi));
    }
    if let Some(zdis) = att.get_f64("ZDIS") {
        diag.insert("ZDIS".into(), serde_json::json!(zdis));
    }
    if let Some(delp) = att.get_dvec3("DELP") {
        diag.insert("DELP".into(), serde_json::json!([delp.x, delp.y, delp.z]));
    }
    if let Some(bang) = att.get_f32("BANG") {
        diag.insert("BANG".into(), serde_json::json!(bang));
    }

    // 计算 local transform
    let local_mat = aios_core::transform::get_local_mat4(refno).await.ok().flatten();
    let local_translation = local_mat.map(|m| {
        let t = m.col(3);
        [t.x, t.y, t.z]
    });

    // 计算 world transform
    let world_mat = aios_core::transform::get_world_mat4(refno, false).await.ok().flatten();
    let world_translation = world_mat.map(|m| {
        let t = m.col(3);
        [t.x, t.y, t.z]
    });
    let world_transform = world_mat.map(|m| m.to_cols_array().iter().map(|v| *v).collect::<Vec<f64>>());

    Ok(Json(ComputeTransformResponse {
        success: world_translation.is_some(),
        refno: refno_str,
        noun: cur_type,
        owner_refno: Some(owner_refno.to_string()),
        owner_noun: Some(owner_noun),
        local_translation,
        world_translation,
        world_transform,
        attrs: serde_json::Value::Object(diag),
        error_message: if world_translation.is_none() {
            Some("无法计算世界变换".into())
        } else {
            None
        },
    }))
}

/// 解析变换矩阵
/// 支持多种格式：
/// - { d: [16个数字] }
/// - { translation: [x, y, z], rotation: [qx, qy, qz, qw], scale: [sx, sy, sz] }
/// - [16个数字]
fn parse_transform_matrix(trans: Option<serde_json::Value>) -> Option<Vec<f64>> {
    let trans = trans?;
    
    // 处理 { d: [...] } 格式
    if let Some(obj) = trans.as_object() {
        if let Some(d) = obj.get("d") {
            if let Some(arr) = d.as_array() {
                if arr.len() == 16 {
                    return Some(arr.iter().filter_map(|v| v.as_f64()).collect());
                }
            }
        }
        
        // 处理 { translation, rotation, scale } 格式
        if let (Some(t), Some(r), Some(s)) = (
            obj.get("translation").and_then(|v| v.as_array()),
            obj.get("rotation").and_then(|v| v.as_array()),
            obj.get("scale").and_then(|v| v.as_array()),
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
                
                return Some(compose_transform_matrix(translation, rotation, scale));
            }
        }
    }
    
    // 处理直接数组格式
    if let Some(arr) = trans.as_array() {
        if arr.len() == 16 {
            return Some(arr.iter().filter_map(|v| v.as_f64()).collect());
        }
    }
    
    None
}

/// 从平移、旋转（四元数）、缩放合成 4x4 变换矩阵（列主序）
fn compose_transform_matrix(
    translation: [f64; 3],
    rotation: [f64; 4], // [qx, qy, qz, qw]
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
        (1.0 - (yy + zz)) * sx, // [0]
        (xy + wz) * sx,          // [1]
        (xz - wy) * sx,          // [2]
        0.0,                      // [3]
        (xy - wz) * sy,          // [4]
        (1.0 - (xx + zz)) * sy,  // [5]
        (yz + wx) * sy,          // [6]
        0.0,                      // [7]
        (xz + wy) * sz,          // [8]
        (yz - wx) * sz,          // [9]
        (1.0 - (xx + yy)) * sz,  // [10]
        0.0,                      // [11]
        x,                        // [12]
        y,                        // [13]
        z,                        // [14]
        1.0,                      // [15]
    ]
}
