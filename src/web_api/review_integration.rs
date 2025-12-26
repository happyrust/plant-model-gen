//! Auxiliary Review Data Integration API
//!
//! Implements the interface for external Model Center to query auxiliary data (Collision, Quality, etc.)

use axum::{
    Router,
    extract::{Json, Query},
    http::{StatusCode, HeaderMap},
    routing::post,
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

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

    // 2. Fetch Collision Data (Mock for now, to be connected to CollisionDetector)
    // In a real implementation, we would query the collision DB/tables for refnos in `request.model_refnos`.
    
    // Returning empty/mock data as per plan phase 1
    let response = AuxDataResponse {
        code: 200,
        message: "ok".to_string(),
        page: request.page,
        page_size: request.page_size,
        total: 0,
        data: AuxDataContent::default(),
    };

    Ok(Json(response))
}
