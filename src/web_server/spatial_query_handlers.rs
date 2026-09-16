use aios_core::spatial::{
    SpatialQueryError, SpatialQueryRequest, SpatialQueryResponse, SpatialQueryService,
    SpatialStatsResponse,
};
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};

pub async fn api_spatial_query(Json(request): Json<SpatialQueryRequest>) -> Response {
    let service = SpatialQueryService::new();
    match service.query(request).await {
        Ok(response) => Json(response).into_response(),
        Err(err) => error_response(err),
    }
}

pub async fn api_spatial_stats() -> Response {
    let service = SpatialQueryService::new();
    match service.stats().await {
        Ok(response) => Json(response).into_response(),
        Err(err) => error_stats_response(err),
    }
}

fn error_response(err: SpatialQueryError) -> Response {
    let status = match err {
        SpatialQueryError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
        SpatialQueryError::QueryFailed(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    let response = SpatialQueryResponse {
        success: false,
        items: Vec::new(),
        total: 0,
        truncated: false,
        query_aabb: None,
        backend: "sqlite-index".to_string(),
        error: Some(err.to_string()),
    };
    (status, Json(response)).into_response()
}

fn error_stats_response(err: SpatialQueryError) -> Response {
    let status = match err {
        SpatialQueryError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
        SpatialQueryError::QueryFailed(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    let response = SpatialStatsResponse {
        success: false,
        backend: "sqlite-index".to_string(),
        total_elements: 0,
        index_type: "sqlite-rtree".to_string(),
        error: Some(err.to_string()),
    };
    (status, Json(response)).into_response()
}
