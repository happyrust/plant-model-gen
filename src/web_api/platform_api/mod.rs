//! Platform API — PMS ↔ 三维校审平台 interface.
//!
//! Inbound only: embed URL, workflow sync, cache preload, review soft-delete.

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

use axum::{Router, routing::post};

pub use review_form::{
    mark_review_form_deleted, sync_review_form_with_task_status, REVIEW_TASK_ACTIVE_SQL,
};
pub use types::derive_review_form_status_from_task_status;

/// Create platform API routes (replaces `create_model_center_routes`).
pub fn create_platform_api_routes() -> Router {
    Router::new()
        .route("/api/review/embed-url", post(embed_url::get_embed_url))
        .route(
            "/api/review/workflow/sync",
            post(workflow_sync::sync_workflow_handler),
        )
        .route(
            "/api/review/delete",
            post(delete_handler::delete_review_data),
        )
        .route(
            "/api/review/cache/preload",
            post(cache_preload::preload_cache),
        )
}
