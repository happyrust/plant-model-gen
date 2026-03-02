use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};

static RUNTIME_STARTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Deserialize, Default)]
pub struct RealtimeInstancesRequest {
    pub refnos: Option<Vec<String>>,
    pub dbnum: Option<u32>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ParquetIncrementalEnqueueRequest {
    pub dbnum: Option<u32>,
    pub reason: Option<String>,
}

pub fn ensure_runtime_started() {
    if !RUNTIME_STARTED.swap(true, Ordering::SeqCst) {
        log::warn!("[model-runtime] 当前为占位实现：后台 worker 未启用");
    }
}

pub async fn api_realtime_instances_by_refnos(
    Json(payload): Json<RealtimeInstancesRequest>,
) -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "success": false,
            "message": "model runtime 暂未启用（占位实现）",
            "refnos_count": payload.refnos.as_ref().map(|v| v.len()).unwrap_or(0),
            "dbnum": payload.dbnum
        })),
    )
}

pub async fn api_parquet_incremental_enqueue(
    Json(payload): Json<ParquetIncrementalEnqueueRequest>,
) -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "success": false,
            "message": "parquet 增量队列暂未启用（占位实现）",
            "dbnum": payload.dbnum,
            "reason": payload.reason
        })),
    )
}

pub async fn api_parquet_version(Path(dbno): Path<u32>) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "dbno": dbno,
            "version": 0,
            "source": "placeholder"
        })),
    )
}
