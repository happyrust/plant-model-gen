use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Router,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use crate::fast_model::export_model::export_parquet::export_db_models_parquet;

#[derive(Debug, Deserialize)]
pub struct ExportRequest {
    pub db_nums: Option<Vec<i64>>,
    pub output_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExportResponse {
    pub success: bool,
    pub message: String,
}

/// 创建导出模块路由
pub fn create_export_api_routes() -> Router {
    Router::new()
        // .route("/api/export/parquet", post(handle_export_parquet))
}

/// 处理 Parquet 导出请求
async fn handle_export_parquet(
    Json(payload): Json<ExportRequest>,
) -> impl IntoResponse {
    let output_dir = payload.output_path.unwrap_or_else(|| "assets/parquet".to_string());
    let target_path = Path::new(&output_dir);

    match export_db_models_parquet(target_path, payload.db_nums).await {
        Ok(_) => {
            (
                StatusCode::OK,
                Json(ExportResponse {
                    success: true,
                    message: format!("Successfully exported to {}", output_dir),
                }),
            )
        }
        Err(e) => {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ExportResponse {
                    success: false,
                    message: format!("Export failed: {}", e),
                }),
            )
        }
    }
}
