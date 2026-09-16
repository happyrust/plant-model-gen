//! Cache preload handler — proxy to trigger model-center caching.

use axum::{extract::Json, http::StatusCode, response::IntoResponse};
use tracing::{info, warn};

use super::auth::verify_s2s_token;
use super::types::{CachePreloadRequest, CachePreloadResponse};

pub async fn preload_cache(Json(request): Json<CachePreloadRequest>) -> impl IntoResponse {
    info!(
        "Cache preload request: project_id={}, initiator={}",
        request.project_id, request.initiator
    );

    if let Err((_status, msg)) = verify_s2s_token(&request.token) {
        warn!(
            "[CACHE_PRELOAD] Token校验失败 - project_id={}, reason={}",
            request.project_id, msg
        );
        return (
            StatusCode::UNAUTHORIZED,
            Json(CachePreloadResponse {
                code: 401,
                message: "unauthorized".to_string(),
                data: None,
            }),
        );
    }

    (
        StatusCode::OK,
        Json(CachePreloadResponse {
            code: 0,
            message: "accepted".to_string(),
            data: Some(serde_json::json!({
                "task_id": format!("cache_{}", request.project_id)
            })),
        }),
    )
}
