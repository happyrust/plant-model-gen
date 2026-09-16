//! Delete review data handler — PMS triggers soft deletion (no outbound callback).

use axum::{extract::Json, http::StatusCode, response::IntoResponse};
use tracing::warn;

use super::auth::verify_s2s_token;
use super::review_form::soft_delete_review_bundle;
use super::types::{DeleteReviewRequest, DeleteReviewResponse, DeleteReviewResult};

pub async fn delete_review_data(Json(request): Json<DeleteReviewRequest>) -> impl IntoResponse {
    if let Err((_status, msg)) = verify_s2s_token(&request.token) {
        warn!(
            "[REVIEW_DELETE] Token校验失败 - form_ids={:?}, reason={}",
            request.form_ids, msg
        );
        return (
            StatusCode::UNAUTHORIZED,
            Json(DeleteReviewResponse {
                code: 401,
                message: "unauthorized".to_string(),
                results: Vec::new(),
            }),
        );
    }

    let mut results = Vec::new();
    let mut has_failure = false;

    for form_id in &request.form_ids {
        match soft_delete_review_bundle(form_id).await {
            Ok(_) => {
                results.push(DeleteReviewResult {
                    form_id: form_id.clone(),
                    success: true,
                    message: "已清理 review 主链".to_string(),
                });
            }
            Err(e) => {
                warn!(
                    "[REVIEW_DELETE] 删除失败 - form_id={}, error={}",
                    form_id, e
                );
                has_failure = true;
                results.push(DeleteReviewResult {
                    form_id: form_id.clone(),
                    success: false,
                    message: e.to_string(),
                });
            }
        }
    }

    let status = if has_failure {
        StatusCode::INTERNAL_SERVER_ERROR
    } else {
        StatusCode::OK
    };
    let message = if has_failure {
        "部分 form_id 删除失败".to_string()
    } else {
        "ok".to_string()
    };

    (
        status,
        Json(DeleteReviewResponse {
            code: status.as_u16() as i32,
            message,
            results,
        }),
    )
}
