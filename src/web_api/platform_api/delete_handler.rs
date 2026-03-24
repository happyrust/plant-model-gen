//! Delete review data handler — PMS triggers soft deletion (no outbound callback).

use axum::{extract::Json, http::StatusCode, response::IntoResponse};
use tracing::warn;

use super::auth::verify_s2s_token;
use super::review_form::soft_delete_review_bundle;
use super::types::{DeleteReviewRequest, DeleteReviewResponse};

pub async fn delete_review_data(Json(request): Json<DeleteReviewRequest>) -> impl IntoResponse {
    if let Err((_status, msg)) = verify_s2s_token(&request.token, None) {
        warn!(
            "[REVIEW_DELETE] Token校验失败 - form_ids={:?}, reason={}",
            request.form_ids, msg
        );
        return (
            StatusCode::UNAUTHORIZED,
            Json(DeleteReviewResponse {
                code: 401,
                message: "unauthorized".to_string(),
            }),
        );
    }

    for form_id in &request.form_ids {
        if let Err(e) = soft_delete_review_bundle(form_id).await {
            warn!(
                "[REVIEW_DELETE] 软删除失败 - form_id={}, error={}",
                form_id, e
            );
        }
    }

    (
        StatusCode::OK,
        Json(DeleteReviewResponse {
            code: 200,
            message: "ok".to_string(),
        }),
    )
}
