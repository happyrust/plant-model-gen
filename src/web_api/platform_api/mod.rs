//! Platform API — PMS ↔ 三维校审平台 interface.
//!
//! Inbound only: embed URL, workflow sync, cache preload, review soft-delete.

pub mod annotation_check;
mod auth;
mod cache_preload;
pub mod config;
mod delete_handler;
mod embed_url;
pub mod review_form;
pub mod types;
mod workflow_sync;

#[cfg(test)]
mod tests;

use axum::{
    Router,
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    routing::post,
};
use tracing::warn;

pub use review_form::{
    REVIEW_TASK_ACTIVE_SQL, mark_review_form_deleted, sync_review_form_with_task_status,
};
pub use types::derive_review_form_status_from_task_status;

async fn ensure_review_db_context(request: Request, next: Next) -> Result<Response, StatusCode> {
    let db_option = aios_core::get_db_option();
    if let Err(error) = aios_core::use_ns_db_compat(
        &aios_core::SUL_DB,
        &db_option.surreal_ns,
        &db_option.project_name,
    )
    .await
    {
        warn!(
            "platform review db context ensure failed: ns={}, db={}, error={}",
            db_option.surreal_ns, db_option.project_name, error
        );
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    if let Err(error) = crate::web_api::review_db::ensure_review_primary_db_context().await {
        warn!(
            "platform review primary db context ensure failed: ns={}, db={}, error={}",
            db_option.surreal_ns, db_option.project_name, error
        );
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    Ok(next.run(request).await)
}

/// Create platform API routes (replaces `create_model_center_routes`).
pub fn create_platform_api_routes() -> Router {
    Router::new()
        .route("/api/review/embed-url", post(embed_url::get_embed_url))
        .route(
            "/api/review/annotations/check",
            post(annotation_check::check_annotations_handler),
        )
        .route(
            "/api/review/workflow/sync",
            post(workflow_sync::sync_workflow_handler),
        )
        .route(
            "/api/review/workflow/verify",
            post(workflow_sync::verify_workflow_handler),
        )
        .route(
            "/api/review/delete",
            post(delete_handler::delete_review_data),
        )
        .route(
            "/api/review/cache/preload",
            post(cache_preload::preload_cache),
        )
        .layer(middleware::from_fn(ensure_review_db_context))
}
