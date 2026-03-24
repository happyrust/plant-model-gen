//! Inbound workflow sync handler — PMS calls this when submitting reviews.
//!
//! SurrealQL 遵循 plant-surrealdb 技能中的通用约定：列表仅取一列时用 `SELECT VALUE`，
//! 明确列投影而非 `SELECT *`，并保持与 `review_form::REVIEW_TASK_ACTIVE_SQL` 一致的任务可见性语义。

use axum::{extract::Json, http::StatusCode, response::IntoResponse};
use surrealdb::types::SurrealValue;
use tracing::{info, warn};

use aios_core::project_primary_db;

use super::auth::verify_s2s_token;
use super::review_form::{find_task_by_form_id, get_review_form_by_form_id};
use super::types::{
    SyncWorkflowData, SyncWorkflowRequest, SyncWorkflowResponse, WorkflowAttachment,
    WorkflowOpinion, normalize_review_form_status,
};

// ============================================================================
// Handler
// ============================================================================

pub async fn sync_workflow_handler(
    Json(request): Json<SyncWorkflowRequest>,
) -> impl IntoResponse {
    let is_query = request.action.eq_ignore_ascii_case("query");
    let request_start_time = std::time::Instant::now();

    info!(
        "[WORKFLOW_SYNC] form_id={}, action={}, actor={}/{}{}",
        request.form_id,
        request.action,
        request.actor.id,
        request.actor.roles,
        if is_query { " (query)" } else { "" }
    );

    if let Err((_status, msg)) = verify_s2s_token(&request.token, Some(&request.form_id)) {
        warn!(
            "[WORKFLOW_SYNC] Token校验失败 - form_id={}, reason={}",
            request.form_id, msg
        );
        return (
            StatusCode::UNAUTHORIZED,
            Json(SyncWorkflowResponse {
                code: 401,
                message: "unauthorized".to_string(),
                data: None,
            }),
        );
    }

    if !is_query {
        let node = request.actor.roles.trim().to_string();
        let seq_order = match node.as_str() {
            "sj" => 1,
            "jd" => 2,
            "sh" => 3,
            "pz" => 4,
            _ => 0,
        };

        let model_refnos = query_workflow_models(&request.form_id)
            .await
            .unwrap_or_default();

        if let Some(comment) = request
            .comments
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            let db_result = project_primary_db()
                .query(
                    r#"
                    CREATE review_opinion CONTENT {
                        form_id: $form_id,
                        model_refnos: $model_refnos,
                        node: $node,
                        seq_order: $seq_order,
                        author: $author,
                        opinion: $opinion,
                        created_at: time::now()
                    }
                    "#,
                )
                .bind(("form_id", request.form_id.clone()))
                .bind(("model_refnos", model_refnos.clone()))
                .bind(("node", node))
                .bind(("seq_order", seq_order))
                .bind(("author", request.actor.id.clone()))
                .bind(("opinion", comment.to_string()))
                .await;

            match db_result {
                Ok(_) => info!(
                    "[WORKFLOW_SYNC] 审批意见写入成功 - form_id={}",
                    request.form_id
                ),
                Err(e) => warn!(
                    "[WORKFLOW_SYNC] 审批意见写入失败 - form_id={}, error={}",
                    request.form_id, e
                ),
            }
        }
    }

    let data = match query_workflow_data(&request.form_id).await {
        Ok(d) => {
            info!(
                "[WORKFLOW_SYNC] 数据查询完成 - form_id={}, models={}, opinions={}, attachments={}",
                request.form_id,
                d.models.len(),
                d.opinions.len(),
                d.attachments.len()
            );
            d
        }
        Err(e) => {
            warn!(
                "[WORKFLOW_SYNC] 数据查询失败 - form_id={}, error={}",
                request.form_id, e
            );
            SyncWorkflowData::default()
        }
    };

    info!(
        "[WORKFLOW_SYNC] 完成 - form_id={}, action={}, elapsed_ms={}",
        request.form_id,
        request.action,
        request_start_time.elapsed().as_millis()
    );

    (
        StatusCode::OK,
        Json(SyncWorkflowResponse {
            code: 200,
            message: "success".to_string(),
            data: Some(data),
        }),
    )
}

// ============================================================================
// DB queries
// ============================================================================

async fn query_workflow_models(form_id: &str) -> anyhow::Result<Vec<String>> {
    let mut response = project_primary_db()
        .query(
            r#"
            SELECT VALUE model_refno FROM review_form_model
            WHERE form_id = $form_id AND model_refno != NONE
            "#,
        )
        .bind(("form_id", form_id.to_string()))
        .await?;

    let rows: Vec<Option<String>> = response.take(0)?;
    Ok(rows
        .into_iter()
        .flatten()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect())
}

async fn query_workflow_opinions(form_id: &str) -> anyhow::Result<Vec<WorkflowOpinion>> {
    let mut response = project_primary_db()
        .query(
            r#"
            SELECT model_refnos, node, seq_order, author, opinion, created_at
            FROM review_opinion
            WHERE form_id = $form_id
            ORDER BY seq_order ASC
            "#,
        )
        .bind(("form_id", form_id.to_string()))
        .await?;

    #[derive(Debug, serde::Deserialize, SurrealValue)]
    struct OpinionRow {
        model_refnos: Option<Vec<String>>,
        node: Option<String>,
        seq_order: Option<i32>,
        author: Option<String>,
        opinion: Option<String>,
        created_at: Option<surrealdb::types::Datetime>,
    }

    let rows: Vec<OpinionRow> = response.take(0)?;
    Ok(rows
        .into_iter()
        .map(|r| WorkflowOpinion {
            model: r.model_refnos.unwrap_or_default(),
            node: r.node.unwrap_or_default(),
            order: r.seq_order.unwrap_or(0),
            author: r.author.unwrap_or_default(),
            opinion: r.opinion.unwrap_or_default(),
            created_at: r.created_at.map(|dt| dt.to_string()).unwrap_or_default(),
        })
        .collect())
}

async fn query_workflow_attachments(form_id: &str) -> anyhow::Result<Vec<WorkflowAttachment>> {
    let mut response = project_primary_db()
        .query(
            r#"
            SELECT model_refnos, file_id, file_type, download_url, description, file_ext
            FROM review_attachment
            WHERE form_id = $form_id
            "#,
        )
        .bind(("form_id", form_id.to_string()))
        .await?;

    #[derive(Debug, serde::Deserialize, SurrealValue)]
    struct AttachmentRow {
        model_refnos: Option<Vec<String>>,
        file_id: Option<String>,
        file_type: Option<String>,
        download_url: Option<String>,
        description: Option<String>,
        file_ext: Option<String>,
    }

    let rows: Vec<AttachmentRow> = response.take(0)?;
    Ok(rows
        .into_iter()
        .map(|r| WorkflowAttachment {
            model: r.model_refnos.unwrap_or_default(),
            id: r.file_id.unwrap_or_default(),
            r#type: r.file_type.unwrap_or_default(),
            download_url: r.download_url.unwrap_or_default(),
            description: r.description.unwrap_or_default(),
            file_ext: r.file_ext.unwrap_or_default(),
        })
        .collect())
}

async fn query_workflow_data(form_id: &str) -> anyhow::Result<SyncWorkflowData> {
    let models = query_workflow_models(form_id).await.unwrap_or_default();
    let opinions = query_workflow_opinions(form_id).await.unwrap_or_default();
    let attachments = query_workflow_attachments(form_id)
        .await
        .unwrap_or_default();

    let review_form = get_review_form_by_form_id(form_id).await.unwrap_or(None);
    let task = find_task_by_form_id(form_id).await.unwrap_or(None);
    let task_created = Some(task.is_some());
    let current_node = task.as_ref().map(|t| t.current_node.clone());
    let task_status = task.as_ref().map(|t| t.status.clone());
    let form_exists = review_form.is_some();
    let form_status = review_form
        .as_ref()
        .map(|form| normalize_review_form_status(form.status.as_str()));

    Ok(SyncWorkflowData {
        models,
        opinions,
        attachments,
        form_exists,
        form_status,
        task_created,
        current_node,
        task_status,
    })
}
