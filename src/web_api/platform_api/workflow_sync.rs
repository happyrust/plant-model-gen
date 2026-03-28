//! Inbound workflow sync handler — PMS calls this when submitting reviews.
//!
//! SurrealQL 遵循 plant-surrealdb 技能中的通用约定：列表仅取一列时用 `SELECT VALUE`，
//! 明确列投影而非 `SELECT *`，并保持与 `review_form::REVIEW_TASK_ACTIVE_SQL` 一致的任务可见性语义。

use axum::{extract::Json, http::StatusCode, response::IntoResponse};
use serde_json::Value;
use std::{collections::BTreeSet, path::Path, sync::OnceLock};
use surrealdb::types::SurrealValue;
use tracing::{info, warn};

use aios_core::project_primary_db;

use super::auth::verify_s2s_token;
use super::review_form::{find_task_by_form_id, get_review_form_by_form_id};
use super::types::{
    SyncWorkflowData, SyncWorkflowRequest, SyncWorkflowResponse, WorkflowAnnotationComment,
    WorkflowAttachment, WorkflowRecord, normalize_review_form_status,
};

static WEB_PUBLIC_BASE_URL: OnceLock<Option<String>> = OnceLock::new();

fn format_beijing_datetime_millis(millis: i64) -> String {
    let Some(utc_dt) = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(millis) else {
        return String::new();
    };
    let Some(beijing_offset) = chrono::FixedOffset::east_opt(8 * 3600) else {
        return String::new();
    };

    utc_dt
        .with_timezone(&beijing_offset)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

fn load_web_public_base_url() -> Option<String> {
    WEB_PUBLIC_BASE_URL
        .get_or_init(|| {
            let config_name = std::env::var("DB_OPTION_FILE")
                .unwrap_or_else(|_| "db_options/DbOption".to_string());
            let config_file = format!("{}.toml", config_name);
            if !Path::new(&config_file).exists() {
                return None;
            }

            let cfg = config::Config::builder()
                .add_source(config::File::with_name(&config_name))
                .build()
                .ok()?;

            cfg.get_string("web_server.public_base_url")
                .ok()
                .or_else(|| cfg.get_string("web_server.backend_url").ok())
                .map(|value| value.trim().trim_end_matches('/').to_string())
                .filter(|value| !value.is_empty())
        })
        .clone()
}

fn normalize_route_url(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        String::new()
    } else if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        let scheme_sep = trimmed.find("://").map(|idx| idx + 3).unwrap_or(0);
        let path_start = trimmed[scheme_sep..]
            .find('/')
            .map(|idx| idx + scheme_sep)
            .unwrap_or(trimmed.len());
        let route = &trimmed[path_start..];
        if route.is_empty() {
            "/".to_string()
        } else if route.starts_with('/') {
            route.to_string()
        } else {
            format!("/{}", route)
        }
    } else if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{}", trimmed)
    }
}

fn build_public_url(route_url: &str) -> Option<String> {
    let normalized = normalize_route_url(route_url);
    if normalized.is_empty() {
        return None;
    }
    if normalized.starts_with("http://") || normalized.starts_with("https://") {
        return Some(normalized);
    }
    load_web_public_base_url().map(|base| format!("{}{}", base, normalized))
}

fn record_id_to_string(id: surrealdb::types::RecordId) -> String {
    match id.key {
        surrealdb::types::RecordIdKey::String(value) => value,
        other => format!("{:?}", other),
    }
}

fn extract_annotation_ids(values: &[Value], output: &mut BTreeSet<String>) {
    for value in values {
        match value {
            Value::Object(map) => {
                if let Some(id) = map
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                {
                    output.insert(id.to_string());
                }
            }
            Value::Array(items) => extract_annotation_ids(items, output),
            _ => {}
        }
    }
}

// ============================================================================
// Handler
// ============================================================================

pub async fn sync_workflow_handler(Json(request): Json<SyncWorkflowRequest>) -> impl IntoResponse {
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
        if let Some(comment) = request
            .comments
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            info!(
                "[WORKFLOW_SYNC] 已接收平台流程 comments，但不在模型中心持久化/回传 - form_id={}, actor={}, comment_len={}",
                request.form_id,
                request.actor.id,
                comment.len()
            );
        } else {
            info!(
                "[WORKFLOW_SYNC] 非 query 动作未携带 comments，按模型批注包只读聚合语义处理 - form_id={}",
                request.form_id
            );
        }
    }

    let data = match query_workflow_data(&request.form_id).await {
        Ok(d) => {
            info!(
                "[WORKFLOW_SYNC] 数据查询完成 - form_id={}, models={}, records={}, comments={}, attachments={}",
                request.form_id,
                d.models.len(),
                d.records.len(),
                d.annotation_comments.len(),
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
        .map(|r| {
            let route_url_raw = r.download_url.unwrap_or_default();
            let route_url = normalize_route_url(&route_url_raw);
            WorkflowAttachment {
                model: r.model_refnos.unwrap_or_default(),
                id: r.file_id.unwrap_or_default(),
                r#type: r.file_type.unwrap_or_default(),
                route_url: route_url.clone(),
                download_url: route_url.clone(),
                public_url: build_public_url(&route_url),
                description: r.description.unwrap_or_default(),
                file_ext: r.file_ext.unwrap_or_default(),
            }
        })
        .collect())
}

async fn query_workflow_records_by_form_id(form_id: &str) -> anyhow::Result<Vec<WorkflowRecord>> {
    #[derive(Debug, serde::Deserialize, SurrealValue)]
    struct RecordRow {
        id: surrealdb::types::RecordId,
        task_id: Option<String>,
        r#type: Option<String>,
        annotations: Option<Vec<Value>>,
        cloud_annotations: Option<Vec<Value>>,
        rect_annotations: Option<Vec<Value>>,
        obb_annotations: Option<Vec<Value>>,
        measurements: Option<Vec<Value>>,
        note: Option<String>,
        confirmed_at: Option<surrealdb::types::Datetime>,
    }

    let mut response = project_primary_db()
        .query(
            r#"
            SELECT id, task_id, type, annotations, cloud_annotations, rect_annotations, obb_annotations, measurements, note, confirmed_at
            FROM review_records
            WHERE form_id = $form_id
            ORDER BY confirmed_at DESC
            "#,
        )
        .bind(("form_id", form_id.to_string()))
        .await?;

    let rows: Vec<RecordRow> = response.take(0)?;
    Ok(rows
        .into_iter()
        .map(|row| WorkflowRecord {
            id: record_id_to_string(row.id),
            task_id: row.task_id.unwrap_or_default(),
            r#type: row.r#type.unwrap_or_else(|| "batch".to_string()),
            annotations: row.annotations.unwrap_or_default(),
            cloud_annotations: row.cloud_annotations.unwrap_or_default(),
            rect_annotations: row.rect_annotations.unwrap_or_default(),
            obb_annotations: row.obb_annotations.unwrap_or_default(),
            measurements: row.measurements.unwrap_or_default(),
            note: row.note.unwrap_or_default(),
            confirmed_at: row
                .confirmed_at
                .map(|dt| format_beijing_datetime_millis(dt.timestamp_millis()))
                .unwrap_or_default(),
        })
        .collect())
}

async fn query_workflow_records_by_task_id(task_id: &str) -> anyhow::Result<Vec<WorkflowRecord>> {
    #[derive(Debug, serde::Deserialize, SurrealValue)]
    struct RecordRow {
        id: surrealdb::types::RecordId,
        task_id: Option<String>,
        r#type: Option<String>,
        annotations: Option<Vec<Value>>,
        cloud_annotations: Option<Vec<Value>>,
        rect_annotations: Option<Vec<Value>>,
        obb_annotations: Option<Vec<Value>>,
        measurements: Option<Vec<Value>>,
        note: Option<String>,
        confirmed_at: Option<surrealdb::types::Datetime>,
    }

    let mut response = project_primary_db()
        .query(
            r#"
            SELECT id, task_id, type, annotations, cloud_annotations, rect_annotations, obb_annotations, measurements, note, confirmed_at
            FROM review_records
            WHERE task_id = $task_id
            ORDER BY confirmed_at DESC
            "#,
        )
        .bind(("task_id", task_id.to_string()))
        .await?;

    let rows: Vec<RecordRow> = response.take(0)?;
    Ok(rows
        .into_iter()
        .map(|row| WorkflowRecord {
            id: record_id_to_string(row.id),
            task_id: row.task_id.unwrap_or_else(|| task_id.to_string()),
            r#type: row.r#type.unwrap_or_else(|| "batch".to_string()),
            annotations: row.annotations.unwrap_or_default(),
            cloud_annotations: row.cloud_annotations.unwrap_or_default(),
            rect_annotations: row.rect_annotations.unwrap_or_default(),
            obb_annotations: row.obb_annotations.unwrap_or_default(),
            measurements: row.measurements.unwrap_or_default(),
            note: row.note.unwrap_or_default(),
            confirmed_at: row
                .confirmed_at
                .map(|dt| format_beijing_datetime_millis(dt.timestamp_millis()))
                .unwrap_or_default(),
        })
        .collect())
}

async fn query_annotation_comments(
    annotation_ids: &[String],
) -> anyhow::Result<Vec<WorkflowAnnotationComment>> {
    #[derive(Debug, serde::Deserialize, SurrealValue)]
    struct CommentRow {
        id: surrealdb::types::RecordId,
        annotation_id: Option<String>,
        annotation_type: Option<String>,
        author_id: Option<String>,
        author_name: Option<String>,
        author_role: Option<String>,
        content: Option<String>,
        reply_to_id: Option<String>,
        created_at: Option<surrealdb::types::Datetime>,
    }

    let mut comments = Vec::new();
    for annotation_id in annotation_ids {
        let mut response = project_primary_db()
            .query(
                r#"
                SELECT id, annotation_id, annotation_type, author_id, author_name, author_role, content, reply_to_id, created_at
                FROM review_comments
                WHERE annotation_id = $annotation_id
                ORDER BY created_at ASC
                "#,
            )
            .bind(("annotation_id", annotation_id.to_string()))
            .await?;

        let rows: Vec<CommentRow> = response.take(0)?;
        comments.extend(rows.into_iter().map(|row| {
            WorkflowAnnotationComment {
                id: record_id_to_string(row.id),
                annotation_id: row.annotation_id.unwrap_or_default(),
                annotation_type: row.annotation_type.unwrap_or_default(),
                author_id: row.author_id.unwrap_or_default(),
                author_name: row.author_name.unwrap_or_default(),
                author_role: row.author_role.unwrap_or_default(),
                content: row.content.unwrap_or_default(),
                reply_to_id: row.reply_to_id,
                created_at: row
                    .created_at
                    .map(|dt| format_beijing_datetime_millis(dt.timestamp_millis()))
                    .unwrap_or_default(),
            }
        }));
    }

    Ok(comments)
}

async fn query_workflow_data(form_id: &str) -> anyhow::Result<SyncWorkflowData> {
    let models = query_workflow_models(form_id).await.unwrap_or_default();
    let attachments = query_workflow_attachments(form_id)
        .await
        .unwrap_or_default();
    let review_form = get_review_form_by_form_id(form_id).await.unwrap_or(None);
    let task = find_task_by_form_id(form_id).await.unwrap_or(None);
    let task_id = task.as_ref().map(|t| t.id.clone());
    let records = {
        let by_form = query_workflow_records_by_form_id(form_id)
            .await
            .unwrap_or_default();
        if !by_form.is_empty() {
            by_form
        } else if let Some(task_id) = task_id.as_deref() {
            query_workflow_records_by_task_id(task_id)
                .await
                .unwrap_or_default()
        } else {
            Vec::new()
        }
    };
    let mut annotation_ids = BTreeSet::new();
    for record in &records {
        extract_annotation_ids(&record.annotations, &mut annotation_ids);
        extract_annotation_ids(&record.cloud_annotations, &mut annotation_ids);
        extract_annotation_ids(&record.rect_annotations, &mut annotation_ids);
        extract_annotation_ids(&record.obb_annotations, &mut annotation_ids);
    }
    let annotation_comments = if annotation_ids.is_empty() {
        Vec::new()
    } else {
        query_annotation_comments(&annotation_ids.into_iter().collect::<Vec<_>>())
            .await
            .unwrap_or_default()
    };
    let task_created = Some(task.is_some());
    let current_node = task.as_ref().map(|t| t.current_node.clone());
    let task_status = task.as_ref().map(|t| t.status.clone());
    let form_exists = review_form.is_some();
    let form_status = review_form
        .as_ref()
        .map(|form| normalize_review_form_status(form.status.as_str()));

    Ok(SyncWorkflowData {
        models,
        task_id,
        records,
        annotation_comments,
        attachments,
        form_exists,
        form_status,
        task_created,
        current_node,
        task_status,
    })
}
