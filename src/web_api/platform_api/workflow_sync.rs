//! Inbound workflow sync handler — PMS calls this when submitting reviews.
//!
//! SurrealQL 遵循 plant-surrealdb 技能中的通用约定：列表仅取一列时用 `SELECT VALUE`，
//! 明确列投影而非 `SELECT *`，并保持与 `review_form::REVIEW_TASK_ACTIVE_SQL` 一致的任务可见性语义。

use axum::{extract::Json, http::StatusCode, response::IntoResponse};
use serde_json::Value;
use std::{collections::BTreeSet, path::Path, sync::OnceLock};
use surrealdb::types::SurrealValue;
use tracing::{info, warn};

use crate::web_api::review_api::ReviewTask;
use aios_core::project_primary_db;

use super::auth::verify_s2s_token;
use super::review_form::{
    find_task_by_form_id, get_review_form_by_form_id, sync_review_form_with_task_status,
};
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
    let action = request.action.trim().to_lowercase();
    let is_query = action == "query";
    let request_start_time = std::time::Instant::now();

    info!(
        "[WORKFLOW_SYNC] form_id={}, action={}, actor={}/{}{}",
        request.form_id,
        action,
        request.actor.id,
        request.actor.roles,
        if is_query { " (query)" } else { "" }
    );

    if let Err((_status, msg)) = verify_s2s_token(&request.token) {
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

    let mut response_next_step = None;
    if !is_query {
        match action.as_str() {
            "active" => match apply_workflow_active(&request).await {
                Ok(next_step) => {
                    response_next_step = next_step;
                }
                Err((status, msg)) => {
                    warn!(
                        "[WORKFLOW_SYNC] active 执行失败 - form_id={}, actor={}, reason={}",
                        request.form_id, request.actor.id, msg
                    );
                    return (
                        status,
                        Json(SyncWorkflowResponse {
                            code: status.as_u16() as i32,
                            message: msg,
                            data: None,
                        }),
                    );
                }
            },
            "agree" => match apply_workflow_agree(&request).await {
                Ok(next_step) => {
                    response_next_step = next_step;
                }
                Err((status, msg)) => {
                    warn!(
                        "[WORKFLOW_SYNC] agree 执行失败 - form_id={}, actor={}, reason={}",
                        request.form_id, request.actor.id, msg
                    );
                    return (
                        status,
                        Json(SyncWorkflowResponse {
                            code: status.as_u16() as i32,
                            message: msg,
                            data: None,
                        }),
                    );
                }
            },
            "return" => match apply_workflow_return(&request).await {
                Ok(next_step) => {
                    response_next_step = next_step;
                }
                Err((status, msg)) => {
                    warn!(
                        "[WORKFLOW_SYNC] return 执行失败 - form_id={}, actor={}, reason={}",
                        request.form_id, request.actor.id, msg
                    );
                    return (
                        status,
                        Json(SyncWorkflowResponse {
                            code: status.as_u16() as i32,
                            message: msg,
                            data: None,
                        }),
                    );
                }
            },
            "stop" => match apply_workflow_stop(&request).await {
                Ok(next_step) => {
                    response_next_step = next_step;
                }
                Err((status, msg)) => {
                    warn!(
                        "[WORKFLOW_SYNC] stop 执行失败 - form_id={}, actor={}, reason={}",
                        request.form_id, request.actor.id, msg
                    );
                    return (
                        status,
                        Json(SyncWorkflowResponse {
                            code: status.as_u16() as i32,
                            message: msg,
                            data: None,
                        }),
                    );
                }
            },
            _ => {
                let msg = format!("unsupported workflow action: {}", request.action);
                warn!("[WORKFLOW_SYNC] {}", msg);
                return (
                    StatusCode::BAD_REQUEST,
                    Json(SyncWorkflowResponse {
                        code: 400,
                        message: msg,
                        data: None,
                    }),
                );
            }
        }
    }

    let data = match query_workflow_data(&request.form_id, response_next_step.clone()).await {
        Ok(d) => {
            info!(
                "[WORKFLOW_SYNC] 数据查询完成 - form_id={}, models={}, records={}, comments={}, attachments={}, current_node={:?}, next_step={:?}",
                request.form_id,
                d.models.len(),
                d.records.len(),
                d.annotation_comments.len(),
                d.attachments.len(),
                d.current_node,
                d.next_step
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
        action,
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

fn normalize_workflow_node(raw: &str) -> String {
    raw.trim().to_lowercase()
}

fn workflow_node_rank(node: &str) -> Option<usize> {
    match node {
        "sj" => Some(0),
        "jd" => Some(1),
        "sh" => Some(2),
        "pz" => Some(3),
        _ => None,
    }
}

fn current_node_owner<'a>(task: &'a ReviewTask, current_node: &str) -> (&'a str, &'static str) {
    match current_node {
        "sj" => (task.requester_id.trim(), "requester"),
        "jd" => {
            let checker_id = task.checker_id.trim();
            if !checker_id.is_empty() {
                (checker_id, "checker")
            } else {
                (task.reviewer_id.trim(), "reviewer")
            }
        }
        "sh" | "pz" => (task.approver_id.trim(), "approver"),
        _ => ("", "none"),
    }
}

fn workflow_task_status_for_target_node(node: &str) -> &'static str {
    match node {
        "jd" => "submitted",
        "sh" | "pz" => "in_review",
        "sj" => "draft",
        _ => "submitted",
    }
}

fn assign_review_task_fields(
    target_node: &str,
    assignee_id: &str,
    assignee_name: &str,
) -> (String, String, String, String, String, String) {
    let mut checker_id = String::new();
    let mut checker_name = String::new();
    let mut reviewer_id = String::new();
    let mut reviewer_name = String::new();
    let mut approver_id = String::new();
    let mut approver_name = String::new();

    match target_node {
        "jd" => {
            checker_id = assignee_id.to_string();
            checker_name = assignee_name.to_string();
            reviewer_id = assignee_id.to_string();
            reviewer_name = assignee_name.to_string();
        }
        "sh" | "pz" => {
            approver_id = assignee_id.to_string();
            approver_name = assignee_name.to_string();
        }
        _ => {}
    }

    (
        checker_id,
        checker_name,
        reviewer_id,
        reviewer_name,
        approver_id,
        approver_name,
    )
}

async fn apply_workflow_active(
    request: &SyncWorkflowRequest,
) -> Result<Option<String>, (StatusCode, String)> {
    let next_step = request
        .next_step
        .as_ref()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "active 缺少 next_step".to_string()))?;

    let target_node = normalize_workflow_node(&next_step.roles);
    if target_node.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "active 的 next_step.roles 不能为空".to_string(),
        ));
    }

    let assignee_id = next_step.assignee_id.trim();
    if assignee_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "active 的 next_step.assignee_id 不能为空".to_string(),
        ));
    }
    let assignee_name = next_step.name.trim();

    let task = find_task_by_form_id(&request.form_id)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("查询任务失败: {}", error),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("form_id={} 未找到活动 review task", request.form_id),
            )
        })?;

    let current_node = normalize_workflow_node(&task.current_node);
    if current_node != "sj" {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "active 仅允许从 sj 发起，当前节点为 {}",
                if current_node.is_empty() {
                    "<empty>".to_string()
                } else {
                    current_node.clone()
                }
            ),
        ));
    }

    if !task.requester_id.trim().is_empty() && task.requester_id.trim() != request.actor.id.trim() {
        return Err((
            StatusCode::FORBIDDEN,
            format!(
                "active 权限不足：当前请求人 {} 不是编制负责人 {}",
                request.actor.id.trim(),
                task.requester_id.trim()
            ),
        ));
    }

    let next_status = workflow_task_status_for_target_node(&target_node);
    let (checker_id, checker_name, reviewer_id, reviewer_name, approver_id, approver_name) =
        assign_review_task_fields(&target_node, assignee_id, assignee_name);

    let update_sql = r#"
        UPDATE review_tasks SET
            current_node = $next_node,
            status = $status,
            checker_id = IF string::len(string::trim($checker_id)) > 0 THEN $checker_id ELSE checker_id END,
            checker_name = IF string::len(string::trim($checker_name)) > 0 THEN $checker_name ELSE checker_name END,
            reviewer_id = IF string::len(string::trim($reviewer_id)) > 0 THEN $reviewer_id ELSE reviewer_id END,
            reviewer_name = IF string::len(string::trim($reviewer_name)) > 0 THEN $reviewer_name ELSE reviewer_name END,
            approver_id = IF string::len(string::trim($approver_id)) > 0 THEN $approver_id ELSE approver_id END,
            approver_name = IF string::len(string::trim($approver_name)) > 0 THEN $approver_name ELSE approver_name END,
            return_reason = NONE,
            updated_at = time::now()
        WHERE record::id(id) = $task_id AND (deleted IS NONE OR deleted = false)
    "#;

    project_primary_db()
        .query(update_sql)
        .bind(("task_id", task.id.clone()))
        .bind(("next_node", target_node.clone()))
        .bind(("status", next_status))
        .bind(("checker_id", checker_id))
        .bind(("checker_name", checker_name))
        .bind(("reviewer_id", reviewer_id))
        .bind(("reviewer_name", reviewer_name))
        .bind(("approver_id", approver_id))
        .bind(("approver_name", approver_name))
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("更新 review_tasks 失败: {}", error),
            )
        })?;

    let review_form = get_review_form_by_form_id(&request.form_id)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("查询 review_forms 失败: {}", error),
            )
        })?;

    if let Err(error) = sync_review_form_with_task_status(
        &request.form_id,
        review_form.as_ref().map(|form| form.project_id.as_str()),
        Some(task.requester_id.as_str()),
        "workflow_sync_active",
        next_status,
    )
    .await
    {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("同步 review_forms 状态失败: {}", error),
        ));
    }

    let history_sql = r#"
        CREATE review_workflow_history CONTENT {
            task_id: $task_id,
            node: $from_node,
            action: 'submit',
            operator_id: $operator_id,
            operator_name: $operator_name,
            comment: $comment,
            timestamp: time::now()
        }
    "#;
    let history_comment = request
        .comments
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let _ = project_primary_db()
        .query(history_sql)
        .bind(("task_id", task.id))
        .bind(("from_node", current_node.clone()))
        .bind(("operator_id", request.actor.id.trim().to_string()))
        .bind(("operator_name", request.actor.name.trim().to_string()))
        .bind(("comment", history_comment))
        .await;

    Ok(Some(target_node))
}

async fn apply_workflow_return(
    request: &SyncWorkflowRequest,
) -> Result<Option<String>, (StatusCode, String)> {
    let next_step = request
        .next_step
        .as_ref()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "return 缺少 next_step".to_string()))?;

    let target_node = normalize_workflow_node(&next_step.roles);
    if target_node.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "return 的 next_step.roles 不能为空".to_string(),
        ));
    }

    let assignee_id = next_step.assignee_id.trim();
    if assignee_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "return 的 next_step.assignee_id 不能为空".to_string(),
        ));
    }
    let assignee_name = next_step.name.trim();

    let task = find_task_by_form_id(&request.form_id)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("查询任务失败: {}", error),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("form_id={} 未找到活动 review task", request.form_id),
            )
        })?;

    let current_node = normalize_workflow_node(&task.current_node);
    if current_node != "jd" && current_node != "sh" && current_node != "pz" {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "return 仅允许在 jd/sh/pz 节点执行，当前节点为 {}",
                if current_node.is_empty() {
                    "<empty>".to_string()
                } else {
                    current_node.clone()
                }
            ),
        ));
    }

    let current_rank = workflow_node_rank(&current_node).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            format!("return 无法识别当前节点 {}", current_node),
        )
    })?;
    let target_rank = workflow_node_rank(&target_node).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            format!("return 无法识别目标节点 {}", target_node),
        )
    })?;
    if target_rank >= current_rank {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "return 目标节点 {} 必须位于当前节点 {} 之前",
                target_node, current_node
            ),
        ));
    }

    let (owner_id, owner_source) = current_node_owner(&task, &current_node);
    if !owner_id.is_empty() && owner_id != request.actor.id.trim() {
        return Err((
            StatusCode::FORBIDDEN,
            format!(
                "return 权限不足：当前请求人 {} 不是 {} 节点负责人 {}",
                request.actor.id.trim(),
                owner_source,
                owner_id
            ),
        ));
    }

    let next_status = workflow_task_status_for_target_node(&target_node);
    let (checker_id, checker_name, reviewer_id, reviewer_name, approver_id, approver_name) =
        assign_review_task_fields(&target_node, assignee_id, assignee_name);
    let return_reason = request
        .comments
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "流程已退回".to_string());

    let update_sql = r#"
        UPDATE review_tasks SET
            current_node = $next_node,
            status = $status,
            checker_id = $checker_id,
            checker_name = $checker_name,
            reviewer_id = $reviewer_id,
            reviewer_name = $reviewer_name,
            approver_id = $approver_id,
            approver_name = $approver_name,
            return_reason = $return_reason,
            updated_at = time::now()
        WHERE record::id(id) = $task_id AND (deleted IS NONE OR deleted = false)
    "#;

    project_primary_db()
        .query(update_sql)
        .bind(("task_id", task.id.clone()))
        .bind(("next_node", target_node.clone()))
        .bind(("status", next_status))
        .bind(("checker_id", checker_id))
        .bind(("checker_name", checker_name))
        .bind(("reviewer_id", reviewer_id))
        .bind(("reviewer_name", reviewer_name))
        .bind(("approver_id", approver_id))
        .bind(("approver_name", approver_name))
        .bind(("return_reason", return_reason.clone()))
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("更新 review_tasks 失败: {}", error),
            )
        })?;

    let review_form = get_review_form_by_form_id(&request.form_id)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("查询 review_forms 失败: {}", error),
            )
        })?;

    if let Err(error) = sync_review_form_with_task_status(
        &request.form_id,
        review_form.as_ref().map(|form| form.project_id.as_str()),
        Some(task.requester_id.as_str()),
        "workflow_sync_return",
        next_status,
    )
    .await
    {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("同步 review_forms 状态失败: {}", error),
        ));
    }

    let history_sql = r#"
        CREATE review_workflow_history CONTENT {
            task_id: $task_id,
            node: $from_node,
            action: 'return',
            operator_id: $operator_id,
            operator_name: $operator_name,
            comment: $comment,
            timestamp: time::now()
        }
    "#;
    let _ = project_primary_db()
        .query(history_sql)
        .bind(("task_id", task.id))
        .bind(("from_node", current_node))
        .bind(("operator_id", request.actor.id.trim().to_string()))
        .bind(("operator_name", request.actor.name.trim().to_string()))
        .bind(("comment", Some(return_reason)))
        .await;

    Ok(Some(target_node))
}

async fn apply_workflow_agree(
    request: &SyncWorkflowRequest,
) -> Result<Option<String>, (StatusCode, String)> {
    let task = find_task_by_form_id(&request.form_id)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("查询任务失败: {}", error),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("form_id={} 未找到活动 review task", request.form_id),
            )
        })?;

    let current_node = normalize_workflow_node(&task.current_node);
    if current_node != "jd" && current_node != "sh" && current_node != "pz" {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "agree 仅允许在 jd/sh/pz 节点执行，当前节点为 {}",
                if current_node.is_empty() {
                    "<empty>".to_string()
                } else {
                    current_node.clone()
                }
            ),
        ));
    }

    let (owner_id, owner_source) = current_node_owner(&task, &current_node);
    if !owner_id.is_empty() && owner_id != request.actor.id.trim() {
        return Err((
            StatusCode::FORBIDDEN,
            format!(
                "agree 权限不足：当前请求人 {} 不是 {} 节点负责人 {}",
                request.actor.id.trim(),
                owner_source,
                owner_id
            ),
        ));
    }

    let (
        target_node,
        next_status,
        checker_id,
        checker_name,
        reviewer_id,
        reviewer_name,
        approver_id,
        approver_name,
    ) = if current_node == "pz" {
        (
            current_node.clone(),
            "approved",
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        )
    } else {
        let next_step = request
            .next_step
            .as_ref()
            .ok_or_else(|| (StatusCode::BAD_REQUEST, "agree 缺少 next_step".to_string()))?;

        let target_node = normalize_workflow_node(&next_step.roles);
        if target_node.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                "agree 的 next_step.roles 不能为空".to_string(),
            ));
        }

        let assignee_id = next_step.assignee_id.trim();
        if assignee_id.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                "agree 的 next_step.assignee_id 不能为空".to_string(),
            ));
        }
        let assignee_name = next_step.name.trim();
        let next_status = workflow_task_status_for_target_node(&target_node);
        let (checker_id, checker_name, reviewer_id, reviewer_name, approver_id, approver_name) =
            assign_review_task_fields(&target_node, assignee_id, assignee_name);
        (
            target_node,
            next_status,
            checker_id,
            checker_name,
            reviewer_id,
            reviewer_name,
            approver_id,
            approver_name,
        )
    };

    let update_sql = r#"
        UPDATE review_tasks SET
            current_node = $next_node,
            status = $status,
            checker_id = IF string::len(string::trim($checker_id)) > 0 THEN $checker_id ELSE checker_id END,
            checker_name = IF string::len(string::trim($checker_name)) > 0 THEN $checker_name ELSE checker_name END,
            reviewer_id = IF string::len(string::trim($reviewer_id)) > 0 THEN $reviewer_id ELSE reviewer_id END,
            reviewer_name = IF string::len(string::trim($reviewer_name)) > 0 THEN $reviewer_name ELSE reviewer_name END,
            approver_id = IF string::len(string::trim($approver_id)) > 0 THEN $approver_id ELSE approver_id END,
            approver_name = IF string::len(string::trim($approver_name)) > 0 THEN $approver_name ELSE approver_name END,
            return_reason = NONE,
            updated_at = time::now()
        WHERE record::id(id) = $task_id AND (deleted IS NONE OR deleted = false)
    "#;

    project_primary_db()
        .query(update_sql)
        .bind(("task_id", task.id.clone()))
        .bind(("next_node", target_node.clone()))
        .bind(("status", next_status))
        .bind(("checker_id", checker_id))
        .bind(("checker_name", checker_name))
        .bind(("reviewer_id", reviewer_id))
        .bind(("reviewer_name", reviewer_name))
        .bind(("approver_id", approver_id))
        .bind(("approver_name", approver_name))
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("更新 review_tasks 失败: {}", error),
            )
        })?;

    let review_form = get_review_form_by_form_id(&request.form_id)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("查询 review_forms 失败: {}", error),
            )
        })?;

    if let Err(error) = sync_review_form_with_task_status(
        &request.form_id,
        review_form.as_ref().map(|form| form.project_id.as_str()),
        Some(task.requester_id.as_str()),
        "workflow_sync_agree",
        next_status,
    )
    .await
    {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("同步 review_forms 状态失败: {}", error),
        ));
    }

    let history_sql = r#"
        CREATE review_workflow_history CONTENT {
            task_id: $task_id,
            node: $from_node,
            action: 'approve',
            operator_id: $operator_id,
            operator_name: $operator_name,
            comment: $comment,
            timestamp: time::now()
        }
    "#;
    let history_comment = request
        .comments
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let _ = project_primary_db()
        .query(history_sql)
        .bind(("task_id", task.id))
        .bind(("from_node", current_node.clone()))
        .bind(("operator_id", request.actor.id.trim().to_string()))
        .bind(("operator_name", request.actor.name.trim().to_string()))
        .bind(("comment", history_comment))
        .await;

    if current_node == "pz" {
        Ok(None)
    } else {
        Ok(Some(target_node))
    }
}

async fn apply_workflow_stop(
    request: &SyncWorkflowRequest,
) -> Result<Option<String>, (StatusCode, String)> {
    let task = find_task_by_form_id(&request.form_id)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("查询任务失败: {}", error),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("form_id={} 未找到活动 review task", request.form_id),
            )
        })?;

    let current_node = normalize_workflow_node(&task.current_node);
    if current_node != "jd" && current_node != "sh" && current_node != "pz" {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "stop 仅允许在 jd/sh/pz 节点执行，当前节点为 {}",
                if current_node.is_empty() {
                    "<empty>".to_string()
                } else {
                    current_node.clone()
                }
            ),
        ));
    }

    let (owner_id, owner_source) = current_node_owner(&task, &current_node);
    if !owner_id.is_empty() && owner_id != request.actor.id.trim() {
        return Err((
            StatusCode::FORBIDDEN,
            format!(
                "stop 权限不足：当前请求人 {} 不是 {} 节点负责人 {}",
                request.actor.id.trim(),
                owner_source,
                owner_id
            ),
        ));
    }

    let stop_reason = request
        .comments
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "流程已终止".to_string());

    let update_sql = r#"
        UPDATE review_tasks SET
            status = 'cancelled',
            return_reason = $stop_reason,
            updated_at = time::now()
        WHERE record::id(id) = $task_id AND (deleted IS NONE OR deleted = false)
    "#;

    project_primary_db()
        .query(update_sql)
        .bind(("task_id", task.id.clone()))
        .bind((
            "stop_reason",
            format!("[stop@{}] {}", current_node, stop_reason),
        ))
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("更新 review_tasks 失败: {}", error),
            )
        })?;

    let review_form = get_review_form_by_form_id(&request.form_id)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("查询 review_forms 失败: {}", error),
            )
        })?;

    if let Err(error) = sync_review_form_with_task_status(
        &request.form_id,
        review_form.as_ref().map(|form| form.project_id.as_str()),
        Some(task.requester_id.as_str()),
        "workflow_sync_stop",
        "cancelled",
    )
    .await
    {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("同步 review_forms 状态失败: {}", error),
        ));
    }

    let history_sql = r#"
        CREATE review_workflow_history CONTENT {
            task_id: $task_id,
            node: $from_node,
            action: 'stop',
            operator_id: $operator_id,
            operator_name: $operator_name,
            comment: $comment,
            timestamp: time::now()
        }
    "#;
    let _ = project_primary_db()
        .query(history_sql)
        .bind(("task_id", task.id))
        .bind(("from_node", current_node))
        .bind(("operator_id", request.actor.id.trim().to_string()))
        .bind(("operator_name", request.actor.name.trim().to_string()))
        .bind(("comment", Some(stop_reason)))
        .await;

    Ok(None)
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

async fn query_workflow_data(
    form_id: &str,
    next_step: Option<String>,
) -> anyhow::Result<SyncWorkflowData> {
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
        next_step,
    })
}
