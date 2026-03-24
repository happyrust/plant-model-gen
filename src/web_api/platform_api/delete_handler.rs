//! Delete review data handler — PMS triggers deletion of review forms.

use axum::{extract::Json, http::StatusCode, response::IntoResponse};
use surrealdb::types::SurrealValue;
use tracing::warn;

use aios_core::project_primary_db;

use super::auth::verify_s2s_token;
use super::review_form::mark_review_form_deleted;
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
        if let Ok(mut resp) = project_primary_db()
            .query("SELECT file_id, file_ext FROM review_attachment WHERE form_id = $form_id")
            .bind(("form_id", form_id.clone()))
            .await
        {
            #[derive(Debug, serde::Deserialize, SurrealValue)]
            struct AttachmentFileRow {
                file_id: Option<String>,
                file_ext: Option<String>,
            }

            let rows: Vec<AttachmentFileRow> = resp.take(0).unwrap_or_default();
            for row in rows {
                let file_id = row.file_id.unwrap_or_default();
                if file_id.trim().is_empty() {
                    continue;
                }
                let ext = row.file_ext.unwrap_or_default();
                let ext = ext.trim();
                let file_name = if ext.is_empty() {
                    file_id.clone()
                } else if ext.starts_with('.') {
                    format!("{}{}", file_id, ext)
                } else {
                    format!("{}.{}", file_id, ext)
                };
                let path = format!("assets/review_attachments/{}", file_name);
                let _ = std::fs::remove_file(&path);
            }
        }

        let _ = project_primary_db()
            .query(
                "LET $task_ids = SELECT VALUE id FROM review_tasks WHERE form_id = $form_id;\nDELETE FROM review_records WHERE task_id IN $task_ids;\nDELETE FROM review_workflow_history WHERE task_id IN $task_ids;\nDELETE FROM review_history WHERE task_id IN $task_ids;",
            )
            .bind(("form_id", form_id.clone()))
            .await;
        let _ = project_primary_db()
            .query(
                "LET $ids = SELECT VALUE id FROM review_form_model WHERE form_id = $form_id;\nDELETE $ids;",
            )
            .bind(("form_id", form_id.clone()))
            .await;
        let _ = project_primary_db()
            .query(
                "LET $ids = SELECT VALUE id FROM review_opinion WHERE form_id = $form_id;\nDELETE $ids;",
            )
            .bind(("form_id", form_id.clone()))
            .await;
        let _ = project_primary_db()
            .query(
                "LET $ids = SELECT VALUE id FROM review_attachment WHERE form_id = $form_id;\nDELETE $ids;",
            )
            .bind(("form_id", form_id.clone()))
            .await;
        let _ = project_primary_db()
            .query(
                "LET $ids = SELECT VALUE id FROM review_tasks WHERE form_id = $form_id;\nDELETE $ids;",
            )
            .bind(("form_id", form_id.clone()))
            .await;
        let _ = mark_review_form_deleted(form_id).await;
    }

    (
        StatusCode::OK,
        Json(DeleteReviewResponse {
            code: 200,
            message: "ok".to_string(),
        }),
    )
}
