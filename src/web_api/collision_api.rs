//! 碰撞检测 API
//!
//! 提供 REST API 进行模型碰撞检测。
//!
//! ## Endpoints
//! - `POST /api/collision/detect` - 执行碰撞检测
//! - `GET /api/collision/status` - 检查服务状态

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::Json,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};

#[cfg(feature = "duckdb-export")]
use crate::fast_model::collision_detect::{
    CollisionConfig, CollisionDetector, CollisionEvent, CollisionStats,
};

// ============================================================================
// 状态
// ============================================================================

/// 碰撞检测 API 状态
#[derive(Clone)]
pub struct CollisionApiState {
    pub mesh_dir: PathBuf,
}

impl Default for CollisionApiState {
    fn default() -> Self {
        Self {
            mesh_dir: PathBuf::from("assets/meshes/lod_L0"),
        }
    }
}

// ============================================================================
// 请求/响应结构体
// ============================================================================

/// 碰撞检测请求
#[derive(Debug, Deserialize)]
pub struct CollisionDetectRequest {
    /// 类型过滤 (如 "PIPE", "EQUI")
    #[serde(default)]
    pub noun_filter: Option<String>,
    /// 碰撞容差 (米)
    #[serde(default = "default_tolerance")]
    pub tolerance: f32,
    /// 并发数
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    /// 限制候选对数量
    #[serde(default)]
    pub limit: Option<usize>,
}

fn default_tolerance() -> f32 {
    0.001
}

fn default_concurrency() -> usize {
    8
}

/// 碰撞检测响应
#[derive(Debug, Serialize)]
pub struct CollisionDetectResponse {
    pub success: bool,
    pub stats: Option<CollisionStatsResponse>,
    pub events: Vec<CollisionEventResponse>,
    pub error_message: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CollisionStatsResponse {
    pub candidate_pairs: usize,
    pub collision_events: usize,
    pub broad_phase_ms: u64,
    pub narrow_phase_ms: u64,
    pub total_ms: u64,
}

#[derive(Debug, Serialize)]
pub struct CollisionEventResponse {
    pub refno_a: u64,
    pub refno_b: u64,
    pub contact_point: Option<[f32; 3]>,
    pub penetration_depth: f32,
    pub normal: Option<[f32; 3]>,
}

/// 服务状态响应
#[derive(Debug, Serialize)]
pub struct CollisionStatusResponse {
    pub available: bool,
    pub duckdb_available: bool,
    pub mesh_dir: String,
}

// ============================================================================
// 路由创建
// ============================================================================

/// 创建碰撞检测路由
pub fn create_collision_routes(state: CollisionApiState) -> Router {
    Router::new()
        .route("/api/collision/detect", post(detect_collisions))
        .route("/api/collision/status", get(get_status))
        .with_state(state)
}

// ============================================================================
// 处理函数
// ============================================================================

/// 执行碰撞检测
#[cfg(feature = "duckdb-export")]
async fn detect_collisions(
    State(state): State<CollisionApiState>,
    Json(request): Json<CollisionDetectRequest>,
) -> Result<Json<CollisionDetectResponse>, StatusCode> {
    info!(
        "Collision detection request: noun_filter={:?}, tolerance={}, concurrency={}, limit={:?}",
        request.noun_filter, request.tolerance, request.concurrency, request.limit
    );

    // 构建配置
    let config = CollisionConfig {
        tolerance: request.tolerance,
        concurrency: request.concurrency,
        mesh_dir: state.mesh_dir.clone(),
        limit: request.limit,
    };

    // 创建检测器
    let detector = match CollisionDetector::new(config) {
        Ok(d) => d,
        Err(e) => {
            warn!("Failed to create collision detector: {}", e);
            return Ok(Json(CollisionDetectResponse {
                success: false,
                stats: None,
                events: vec![],
                error_message: Some(format!("Failed to create detector: {}", e)),
            }));
        }
    };

    // 执行检测
    match detector.detect_all(request.noun_filter.as_deref()).await {
        Ok((events, stats)) => {
            let response_events: Vec<CollisionEventResponse> = events
                .into_iter()
                .map(|e| CollisionEventResponse {
                    refno_a: e.pair.0.0,
                    refno_b: e.pair.1.0,
                    contact_point: e.contact_point,
                    penetration_depth: e.penetration_depth,
                    normal: e.normal,
                })
                .collect();

            Ok(Json(CollisionDetectResponse {
                success: true,
                stats: Some(CollisionStatsResponse {
                    candidate_pairs: stats.candidate_pairs,
                    collision_events: stats.collision_events,
                    broad_phase_ms: stats.broad_phase_ms,
                    narrow_phase_ms: stats.narrow_phase_ms,
                    total_ms: stats.total_ms,
                }),
                events: response_events,
                error_message: None,
            }))
        }
        Err(e) => {
            warn!("Collision detection failed: {}", e);
            Ok(Json(CollisionDetectResponse {
                success: false,
                stats: None,
                events: vec![],
                error_message: Some(format!("Detection failed: {}", e)),
            }))
        }
    }
}

#[cfg(not(feature = "duckdb-export"))]
async fn detect_collisions(
    State(_state): State<CollisionApiState>,
    Json(_request): Json<CollisionDetectRequest>,
) -> Result<Json<CollisionDetectResponse>, StatusCode> {
    Ok(Json(CollisionDetectResponse {
        success: false,
        stats: None,
        events: vec![],
        error_message: Some("Collision detection requires duckdb-export feature".to_string()),
    }))
}

/// 获取服务状态
#[cfg(feature = "duckdb-export")]
async fn get_status(
    State(state): State<CollisionApiState>,
) -> Result<Json<CollisionStatusResponse>, StatusCode> {
    // 检查 DuckDB latest.json 是否存在
    let latest_path = std::path::Path::new("assets/web_duckdb/latest.json");
    let duckdb_available = latest_path.exists();

    Ok(Json(CollisionStatusResponse {
        available: duckdb_available,
        duckdb_available,
        mesh_dir: state.mesh_dir.to_string_lossy().to_string(),
    }))
}

#[cfg(not(feature = "duckdb-export"))]
async fn get_status(
    State(state): State<CollisionApiState>,
) -> Result<Json<CollisionStatusResponse>, StatusCode> {
    Ok(Json(CollisionStatusResponse {
        available: false,
        duckdb_available: false,
        mesh_dir: state.mesh_dir.to_string_lossy().to_string(),
    }))
}
