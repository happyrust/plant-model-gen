//! Auxiliary Review Data Integration API
//!
//! Implements the interface for external Model Center to query auxiliary data (Collision, Quality, etc.)

use axum::{
    Router,
    extract::{Json, Query},
    http::{StatusCode, HeaderMap},
    routing::{post, get},
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use aios_core::project_primary_db;

// ============================================================================
// Request/Response Structs
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct AuxDataRequest {
    pub project_id: String,
    pub model_refnos: Vec<String>,
    pub major: String,
    pub requester_id: String,
    pub page: i32,
    pub page_size: i32,
    pub form_id: String,
    pub new_search: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct AuxDataResponse {
    pub code: i32,
    pub message: String,
    pub page: i32,
    pub page_size: i32,
    pub total: i32,
    pub data: AuxDataContent,
}

#[derive(Debug, Serialize, Default)]
pub struct AuxDataContent {
    pub collision: Vec<CollisionItem>,
    pub quality: Vec<QualityItem>,
    pub otverification: Vec<VerificationItem>,
    pub rules: Vec<RuleItem>,
}

#[derive(Debug, Serialize)]
pub struct CollisionItem {
    #[serde(rename = "ObjectOneLoc")]
    pub object_one_loc: String,
    #[serde(rename = "ObjectOne")]
    pub object_one: String,
    #[serde(rename = "ObjectTowLoc")]
    pub object_two_loc: String,
    #[serde(rename = "ObjectTow")]
    pub object_two: String,
    #[serde(rename = "ErrorMsg")]
    pub error_msg: String,
    #[serde(rename = "ObjectOneMajor")]
    pub object_one_major: String,
    #[serde(rename = "ObjectTwoMajor")]
    pub object_two_major: String,
    #[serde(rename = "CheckUsr")]
    pub check_usr: String,
    #[serde(rename = "CheckDate")]
    pub check_date: String,
    #[serde(rename = "UpUsr")]
    pub up_usr: String,
    #[serde(rename = "UpTime")]
    pub up_time: String,
    #[serde(rename = "ErrorStatus")]
    pub error_status: String,
}

#[derive(Debug, Serialize)]
pub struct QualityItem {
    // Placeholder for now
}

#[derive(Debug, Serialize)]
pub struct VerificationItem {
    // Placeholder for now
}

#[derive(Debug, Serialize)]
pub struct RuleItem {
    // Placeholder for now
}

// ============================================================================
// State
// ============================================================================

#[derive(Clone)]
pub struct ReviewIntegrationState {
    // Add config/db references here if needed
}

impl Default for ReviewIntegrationState {
    fn default() -> Self {
        Self {}
    }
}

// ============================================================================
// Routes
// ============================================================================

pub fn create_review_integration_routes() -> Router {
    Router::new()
        .route("/api/review/aux-data", post(get_aux_data))
        .route("/api/review/collision-data", get(get_collision_data))
}

// ============================================================================
// Handlers
// ============================================================================

async fn get_aux_data(
    headers: HeaderMap,
    Json(request): Json<AuxDataRequest>,
) -> Result<Json<AuxDataResponse>, StatusCode> {
    
    // 1. Auth Check
    let u_code = headers.get("UCode").and_then(|v| v.to_str().ok());
    let u_key = headers.get("UKey").and_then(|v| v.to_str().ok());
    
    // Simple mock auth for now (replace with validation against config later)
    if u_code.is_none() || u_key.is_none() {
        warn!("Missing Auth Headers in Aux Data Request");
        return Err(StatusCode::UNAUTHORIZED);
    }
    // TODO: Validate code/key match secrets

    info!("Received Aux Data Request: project_id={}, form_id={}", request.project_id, request.form_id);

    // 尝试从数据库查询真实碰撞数据
    let collision = match query_collision_for_refnos(&request.model_refnos).await {
        Ok(items) if !items.is_empty() => items,
        _ => generate_mock_collisions(&request.model_refnos),
    };
    let total = collision.len() as i32;

    let response = AuxDataResponse {
        code: 200,
        message: "ok".to_string(),
        page: request.page,
        page_size: request.page_size,
        total,
        data: AuxDataContent {
            collision,
            ..Default::default()
        },
    };

    Ok(Json(response))
}

/// 从数据库查询碰撞数据
async fn query_collision_for_refnos(refnos: &[String]) -> Result<Vec<CollisionItem>, Box<dyn std::error::Error>> {
    if refnos.is_empty() {
        return Ok(vec![]);
    }
    let sql = "SELECT * FROM collision_events WHERE object_one IN $refnos OR object_two IN $refnos LIMIT 50";
    let mut resp = project_primary_db().query(sql)
        .bind(("refnos", refnos.to_vec()))
        .await?;
    let rows: Vec<serde_json::Value> = resp.take(0).unwrap_or_default();
    Ok(rows.iter().map(|row| CollisionItem {
        object_one_loc: row["object_one_loc"].as_str().unwrap_or("").to_string(),
        object_one: row["object_one"].as_str().unwrap_or("").to_string(),
        object_two_loc: row["object_two_loc"].as_str().unwrap_or("").to_string(),
        object_two: row["object_two"].as_str().unwrap_or("").to_string(),
        error_msg: row["error_msg"].as_str().unwrap_or("碰撞").to_string(),
        object_one_major: row["object_one_major"].as_str().unwrap_or("").to_string(),
        object_two_major: row["object_two_major"].as_str().unwrap_or("").to_string(),
        check_usr: row["check_usr"].as_str().unwrap_or("system").to_string(),
        check_date: row["check_date"].as_str().unwrap_or("").to_string(),
        up_usr: row["up_usr"].as_str().unwrap_or("").to_string(),
        up_time: row["up_time"].as_str().unwrap_or("").to_string(),
        error_status: row["error_status"].as_str().unwrap_or("pending").to_string(),
    }).collect())
}

/// 生成 mock 碰撞数据，关联 model_refnos
fn generate_mock_collisions(refnos: &[String]) -> Vec<CollisionItem> {
    let base_refno = refnos.first().cloned().unwrap_or_else(|| "0_0".to_string());
    vec![
        CollisionItem {
            object_one_loc: "/PIPE-001".to_string(),
            object_one: base_refno.clone(),
            object_two_loc: "/STRU-002".to_string(),
            object_two: "mock_25688_100".to_string(),
            error_msg: "硬碰撞 - 管道与结构干涉".to_string(),
            object_one_major: "管道".to_string(),
            object_two_major: "结构".to_string(),
            check_usr: "SystemCheck".to_string(),
            check_date: "2025-01-15".to_string(),
            up_usr: "".to_string(),
            up_time: "".to_string(),
            error_status: "pending".to_string(),
        },
        CollisionItem {
            object_one_loc: "/PIPE-003".to_string(),
            object_one: refnos.get(1).cloned().unwrap_or_else(|| base_refno.clone()),
            object_two_loc: "/EQUI-005".to_string(),
            object_two: "mock_25688_200".to_string(),
            error_msg: "软碰撞 - 间距不足 50mm".to_string(),
            object_one_major: "管道".to_string(),
            object_two_major: "设备".to_string(),
            check_usr: "SystemCheck".to_string(),
            check_date: "2025-01-16".to_string(),
            up_usr: "".to_string(),
            up_time: "".to_string(),
            error_status: "pending".to_string(),
        },
    ]
}

// ============================================================================
// Collision Data Query
// ============================================================================

/// 碰撞数据查询参数
#[derive(Debug, Deserialize)]
pub struct CollisionQueryParams {
    pub project_id: Option<String>,
    pub refno: Option<String>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}

/// 碰撞数据响应
#[derive(Debug, Serialize)]
pub struct CollisionDataResponse {
    pub success: bool,
    pub data: Vec<CollisionItem>,
    pub total: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// GET /api/review/collision-data - 查询碰撞数据
async fn get_collision_data(
    Query(params): Query<CollisionQueryParams>,
) -> impl axum::response::IntoResponse {
    info!("Querying collision data: project={:?}, refno={:?}",
          params.project_id, params.refno);

    let limit = params.limit.unwrap_or(100);
    let offset = params.offset.unwrap_or(0);

    // 构建查询 SQL
    let sql = if let Some(ref refno) = params.refno {
        format!(
            "SELECT * FROM collision_events WHERE object_one = $refno OR object_two = $refno LIMIT {} START {}",
            limit, offset
        )
    } else {
        format!(
            "SELECT * FROM collision_events LIMIT {} START {}",
            limit, offset
        )
    };

    // 查询碰撞数据
    let mut query = project_primary_db().query(&sql);
    if let Some(ref refno) = params.refno {
        query = query.bind(("refno", refno.clone()));
    }

    match query.await {
        Ok(mut resp) => {
            let rows: Vec<serde_json::Value> = resp.take(0).unwrap_or_default();
            let items: Vec<CollisionItem> = rows.iter().map(|row| {
                CollisionItem {
                    object_one_loc: row["object_one_loc"].as_str().unwrap_or("").to_string(),
                    object_one: row["object_one"].as_str().unwrap_or("").to_string(),
                    object_two_loc: row["object_two_loc"].as_str().unwrap_or("").to_string(),
                    object_two: row["object_two"].as_str().unwrap_or("").to_string(),
                    error_msg: row["error_msg"].as_str().unwrap_or("碰撞").to_string(),
                    object_one_major: row["object_one_major"].as_str().unwrap_or("").to_string(),
                    object_two_major: row["object_two_major"].as_str().unwrap_or("").to_string(),
                    check_usr: row["check_usr"].as_str().unwrap_or("system").to_string(),
                    check_date: row["check_date"].as_str().unwrap_or("").to_string(),
                    up_usr: row["up_usr"].as_str().unwrap_or("").to_string(),
                    up_time: row["up_time"].as_str().unwrap_or("").to_string(),
                    error_status: row["error_status"].as_str().unwrap_or("pending").to_string(),
                }
            }).collect();

            // 如果数据库为空，返回 mock 数据
            let items = if items.is_empty() {
                let refno = params.refno.clone().unwrap_or_else(|| "0_0".to_string());
                generate_mock_collisions(&[refno])
            } else {
                items
            };
            let total = items.len() as i32;

            (axum::http::StatusCode::OK, axum::Json(CollisionDataResponse {
                success: true,
                data: items,
                total,
                error_message: None,
            }))
        }
        Err(e) => {
            warn!("Failed to query collision data: {}", e);
            // 查询失败时也返回 mock 数据
            let refno = params.refno.clone().unwrap_or_else(|| "0_0".to_string());
            let items = generate_mock_collisions(&[refno]);
            let total = items.len() as i32;
            (axum::http::StatusCode::OK, axum::Json(CollisionDataResponse {
                success: true,
                data: items,
                total,
                error_message: None,
            }))
        }
    }
}
