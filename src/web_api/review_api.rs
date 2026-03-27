//! Review API - 校审管理 API
//!
//! 实现提资单、确认记录、评论、附件等完整的 CRUD 操作
//!
//! `review_tasks` 软删过滤：凡出现 `(deleted IS NONE OR deleted = false)` 的语句须与
//! `platform_api::REVIEW_TASK_ACTIVE_SQL` 保持同步（见 plant-surrealdb 技能：可选 bool / 逻辑删除）。

use axum::{
    Router,
    extract::{Json, Multipart, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, patch, post},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use surrealdb::types::{self as surrealdb_types, SurrealValue};
use tracing::{info, warn};

use crate::web_api::jwt_auth::{TokenClaims, generate_form_id};
use crate::web_api::platform_api::{mark_review_form_deleted, sync_review_form_with_task_status};
use aios_core::project_primary_db;
use axum::extract::Extension;
use std::collections::HashSet;

// ============================================================================
// Request/Response Types
// ============================================================================

/// 创建提资单请求
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskRequest {
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub model_name: String,
    /// 校核人 ID（三段审批第二段，jd 节点负责人）
    pub checker_id: Option<String>,
    /// 校核人姓名（可选，不传则回退到 checker_id/reviewer_id）
    #[serde(default)]
    pub checker_name: Option<String>,
    /// 审核人 ID（三段审批第三段，sh 节点负责人）
    pub approver_id: Option<String>,
    /// 审核人姓名（可选，不传则回退到 approver_id）
    #[serde(default)]
    pub approver_name: Option<String>,
    /// 兼容旧字段：语义等同 checker_id
    #[serde(default)]
    pub reviewer_id: String,
    /// 外部传入的 form_id（若不传则后端生成）
    pub form_id: Option<String>,
    #[serde(default = "default_priority")]
    pub priority: String,
    #[serde(default)]
    pub components: Vec<ReviewComponent>,
    pub due_date: Option<i64>,
    pub attachments: Option<Vec<ReviewAttachment>>,
}

fn default_priority() -> String {
    "medium".to_string()
}

/// 更新提资单请求
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTaskRequest {
    pub title: Option<String>,
    pub description: Option<String>,
    pub priority: Option<String>,
    pub components: Option<Vec<ReviewComponent>>,
    pub due_date: Option<i64>,
    pub attachments: Option<Vec<ReviewAttachment>>,
}

/// 审核操作请求
#[derive(Debug, Deserialize)]
pub struct ReviewActionRequest {
    pub comment: Option<String>,
    pub reason: Option<String>,
}

/// 提交到下一节点请求
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitToNextRequest {
    pub comment: Option<String>,
    pub operator_id: Option<String>,
    pub operator_name: Option<String>,
}

/// 驳回请求
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReturnRequest {
    pub target_node: String, // 目标节点: sj/jd/sh
    pub reason: String,      // 驳回原因
    pub operator_id: Option<String>,
    pub operator_name: Option<String>,
}

/// 工作流步骤
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStep {
    pub node: String,            // 节点: sj/jd/sh/pz
    pub action: String,          // 动作: submit/return/approve/reject
    pub operator_id: String,     // 操作人ID
    pub operator_name: String,   // 操作人姓名
    pub comment: Option<String>, // 备注
    #[serde(serialize_with = "serialize_beijing_datetime_millis")]
    pub timestamp: i64, // 时间戳
}

/// 工作流节点顺序常量
pub const WORKFLOW_NODES: [&str; 4] = ["sj", "jd", "sh", "pz"];

/// 获取节点显示名称
pub fn get_node_display_name(node: &str) -> &'static str {
    match node {
        "sj" => "编制",
        "jd" => "校对",
        "sh" => "审核",
        "pz" => "批准",
        _ => "未知",
    }
}

/// 组件信息
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
#[serde(rename_all = "camelCase")]
pub struct ReviewComponent {
    pub id: String,
    pub name: String,
    pub ref_no: String,
    #[serde(default)]
    pub r#type: String,
}

/// 附件信息
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
#[serde(rename_all = "camelCase")]
pub struct ReviewAttachment {
    pub id: String,
    pub name: String,
    pub url: String,
    pub size: Option<i64>,
    pub mime_type: Option<String>,
}

/// 提资单
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
#[serde(rename_all = "camelCase")]
pub struct ReviewTask {
    pub id: String,
    #[serde(default)]
    pub form_id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub model_name: String,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default = "default_priority")]
    pub priority: String,
    pub requester_id: String,
    pub requester_name: String,
    /// 校核人 ID（jd 节点负责人）
    #[serde(default)]
    pub checker_id: String,
    #[serde(default)]
    pub checker_name: String,
    /// 审核人 ID（sh 节点负责人）
    #[serde(default)]
    pub approver_id: String,
    #[serde(default)]
    pub approver_name: String,
    /// 兼容旧字段（语义等同 checker_id）
    #[serde(default)]
    pub reviewer_id: String,
    #[serde(default)]
    pub reviewer_name: String,
    #[serde(default)]
    pub components: Vec<ReviewComponent>,
    pub attachments: Option<Vec<ReviewAttachment>>,
    pub review_comment: Option<String>,
    #[serde(serialize_with = "serialize_beijing_datetime_millis")]
    pub created_at: i64,
    #[serde(serialize_with = "serialize_beijing_datetime_millis")]
    pub updated_at: i64,
    #[serde(serialize_with = "serialize_optional_beijing_datetime_millis")]
    pub due_date: Option<i64>,
    #[serde(default = "default_current_node")]
    pub current_node: String,
    #[serde(default)]
    pub workflow_history: Vec<WorkflowStep>,
    pub return_reason: Option<String>,
}

fn default_current_node() -> String {
    "sj".to_string()
}

fn default_status() -> String {
    "draft".to_string()
}

pub(crate) fn format_beijing_datetime_millis(millis: i64) -> String {
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

pub(crate) fn serialize_beijing_datetime_millis<S>(
    millis: &i64,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&format_beijing_datetime_millis(*millis))
}

pub(crate) fn serialize_optional_beijing_datetime_millis<S>(
    millis: &Option<i64>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match millis {
        Some(value) => serializer.serialize_some(&format_beijing_datetime_millis(*value)),
        None => serializer.serialize_none(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CreateTaskResolvedNames {
    requester_name: String,
    checker_name: String,
    approver_name: String,
    reviewer_name: String,
}

fn preferred_name(value: Option<&str>, fallback: &str) -> String {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn resolve_create_task_names(
    claims: &TokenClaims,
    request: &CreateTaskRequest,
    checker_id: &str,
    approver_id: &str,
) -> CreateTaskResolvedNames {
    let requester_name = preferred_name(Some(claims.user_name.as_str()), claims.user_id.as_str());
    let checker_name = preferred_name(request.checker_name.as_deref(), checker_id);
    let approver_name = preferred_name(request.approver_name.as_deref(), approver_id);

    CreateTaskResolvedNames {
        requester_name,
        checker_name: checker_name.clone(),
        approver_name,
        reviewer_name: checker_name,
    }
}

/// 任务列表响应
#[derive(Debug, Serialize)]
pub struct TaskListResponse {
    pub success: bool,
    pub tasks: Vec<ReviewTask>,
    pub total: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// 单个任务响应
#[derive(Debug, Serialize)]
pub struct TaskResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<ReviewTask>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// 操作响应
#[derive(Debug, Serialize)]
pub struct ActionResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// 查询参数
#[derive(Debug, Deserialize)]
pub struct TaskListQuery {
    pub status: Option<String>,
    pub priority: Option<String>,
    pub requester_id: Option<String>,
    pub checker_id: Option<String>,
    pub approver_id: Option<String>,
    pub reviewer_id: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ============================================================================
// Database Row Types
// ============================================================================

#[derive(Debug, Deserialize, SurrealValue)]
struct TaskRow {
    id: surrealdb::types::RecordId,
    form_id: Option<String>,
    title: Option<String>,
    description: Option<String>,
    model_name: Option<String>,
    status: Option<String>,
    priority: Option<String>,
    requester_id: Option<String>,
    requester_name: Option<String>,
    checker_id: Option<String>,
    checker_name: Option<String>,
    approver_id: Option<String>,
    approver_name: Option<String>,
    reviewer_id: Option<String>,
    reviewer_name: Option<String>,
    components: Option<Vec<ReviewComponent>>,
    attachments: Option<Vec<ReviewAttachment>>,
    review_comment: Option<String>,
    created_at: Option<surrealdb::types::Datetime>,
    updated_at: Option<surrealdb::types::Datetime>,
    due_date: Option<surrealdb::types::Datetime>,
    current_node: Option<String>,
    workflow_history: Option<Vec<WorkflowStep>>,
    return_reason: Option<String>,
}

impl TaskRow {
    fn to_review_task(self) -> ReviewTask {
        let id = match &self.id.key {
            surrealdb::types::RecordIdKey::String(s) => s.clone(),
            other => format!("{:?}", other),
        };
        let checker_id = self
            .checker_id
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| self.reviewer_id.clone())
            .unwrap_or_default();
        let checker_name = self
            .checker_name
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| self.reviewer_name.clone())
            .unwrap_or_default();
        ReviewTask {
            id,
            form_id: self.form_id.unwrap_or_default(),
            title: self.title.unwrap_or_default(),
            description: self.description.unwrap_or_default(),
            model_name: self.model_name.unwrap_or_default(),
            status: self.status.unwrap_or_else(default_status),
            priority: self.priority.unwrap_or_else(default_priority),
            requester_id: self.requester_id.unwrap_or_default(),
            requester_name: self.requester_name.unwrap_or_default(),
            checker_id: checker_id.clone(),
            checker_name: checker_name.clone(),
            approver_id: self.approver_id.unwrap_or_default(),
            approver_name: self.approver_name.unwrap_or_default(),
            reviewer_id: self.reviewer_id.unwrap_or_else(|| checker_id),
            reviewer_name: self.reviewer_name.unwrap_or_else(|| checker_name),
            components: self.components.unwrap_or_default(),
            attachments: self.attachments,
            review_comment: self.review_comment,
            created_at: datetime_to_millis(&self.created_at),
            updated_at: datetime_to_millis(&self.updated_at),
            due_date: self.due_date.map(|dt| datetime_to_millis(&Some(dt))),
            current_node: self.current_node.unwrap_or_else(default_current_node),
            workflow_history: self.workflow_history.unwrap_or_default(),
            return_reason: self.return_reason,
        }
    }
}

fn normalize_record_id_string(raw: String) -> String {
    raw.rsplit(':').next().unwrap_or(raw.as_str()).to_string()
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Object(map) => map
            .get("id")
            .and_then(value_to_string)
            .or_else(|| map.get("key").and_then(value_to_string))
            .or_else(|| map.get("value").and_then(value_to_string))
            .map(normalize_record_id_string),
        _ => None,
    }
}

fn value_to_timestamp_millis(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_u64().and_then(|v| i64::try_from(v).ok())),
        Value::String(s) => s.parse::<i64>().ok().or_else(|| {
            chrono::DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|dt| dt.timestamp_millis())
        }),
        Value::Object(map) => map
            .get("$surrealdb::private::sql::Datetime")
            .and_then(value_to_timestamp_millis)
            .or_else(|| map.get("datetime").and_then(value_to_timestamp_millis))
            .or_else(|| map.get("value").and_then(value_to_timestamp_millis)),
        _ => None,
    }
}

fn component_from_value(value: &Value) -> ReviewComponent {
    let map = value.as_object();

    ReviewComponent {
        id: map
            .and_then(|entry| entry.get("id"))
            .and_then(value_to_string)
            .unwrap_or_default(),
        name: map
            .and_then(|entry| entry.get("name"))
            .and_then(value_to_string)
            .unwrap_or_default(),
        ref_no: map
            .and_then(|entry| entry.get("ref_no").or_else(|| entry.get("refNo")))
            .and_then(value_to_string)
            .unwrap_or_default(),
        r#type: map
            .and_then(|entry| entry.get("type"))
            .and_then(value_to_string)
            .unwrap_or_default(),
    }
}

fn attachment_from_value(value: &Value) -> ReviewAttachment {
    let map = value.as_object();

    ReviewAttachment {
        id: map
            .and_then(|entry| entry.get("id"))
            .and_then(value_to_string)
            .unwrap_or_default(),
        name: map
            .and_then(|entry| entry.get("name"))
            .and_then(value_to_string)
            .unwrap_or_default(),
        url: map
            .and_then(|entry| entry.get("url"))
            .and_then(value_to_string)
            .unwrap_or_default(),
        size: map
            .and_then(|entry| entry.get("size"))
            .and_then(value_to_timestamp_millis),
        mime_type: map
            .and_then(|entry| entry.get("mime_type").or_else(|| entry.get("mimeType")))
            .and_then(value_to_string),
    }
}

fn workflow_step_from_value(value: &Value) -> WorkflowStep {
    let map = value.as_object();

    WorkflowStep {
        node: map
            .and_then(|entry| entry.get("node"))
            .and_then(value_to_string)
            .unwrap_or_default(),
        action: map
            .and_then(|entry| entry.get("action"))
            .and_then(value_to_string)
            .unwrap_or_default(),
        operator_id: map
            .and_then(|entry| entry.get("operator_id").or_else(|| entry.get("operatorId")))
            .and_then(value_to_string)
            .unwrap_or_default(),
        operator_name: map
            .and_then(|entry| {
                entry
                    .get("operator_name")
                    .or_else(|| entry.get("operatorName"))
            })
            .and_then(value_to_string)
            .unwrap_or_default(),
        comment: map
            .and_then(|entry| entry.get("comment"))
            .and_then(value_to_string),
        timestamp: map
            .and_then(|entry| entry.get("timestamp"))
            .and_then(value_to_timestamp_millis)
            .unwrap_or_else(|| chrono::Utc::now().timestamp_millis()),
    }
}

fn review_task_from_value(value: Value) -> Option<ReviewTask> {
    if let Ok(row) = serde_json::from_value::<TaskRow>(value.clone()) {
        return Some(row.to_review_task());
    }

    let map = value.as_object()?;
    let checker_id = map
        .get("checker_id")
        .or_else(|| map.get("checkerId"))
        .and_then(value_to_string)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            map.get("reviewer_id")
                .or_else(|| map.get("reviewerId"))
                .and_then(value_to_string)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_default();
    let checker_name = map
        .get("checker_name")
        .or_else(|| map.get("checkerName"))
        .and_then(value_to_string)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            map.get("reviewer_name")
                .or_else(|| map.get("reviewerName"))
                .and_then(value_to_string)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_default();
    let reviewer_id = map
        .get("reviewer_id")
        .or_else(|| map.get("reviewerId"))
        .and_then(value_to_string)
        .unwrap_or_else(|| checker_id.clone());
    let reviewer_name = map
        .get("reviewer_name")
        .or_else(|| map.get("reviewerName"))
        .and_then(value_to_string)
        .unwrap_or_else(|| checker_name.clone());
    let components = map
        .get("components")
        .and_then(|value| value.as_array())
        .map(|items| items.iter().map(component_from_value).collect())
        .unwrap_or_default();
    let attachments = map
        .get("attachments")
        .and_then(|value| value.as_array())
        .map(|items| items.iter().map(attachment_from_value).collect::<Vec<_>>());
    let workflow_history = map
        .get("workflow_history")
        .or_else(|| map.get("workflowHistory"))
        .and_then(|value| value.as_array())
        .map(|items| items.iter().map(workflow_step_from_value).collect())
        .unwrap_or_default();

    Some(ReviewTask {
        id: map.get("id").and_then(value_to_string).unwrap_or_default(),
        form_id: map
            .get("form_id")
            .or_else(|| map.get("formId"))
            .and_then(value_to_string)
            .unwrap_or_default(),
        title: map
            .get("title")
            .and_then(value_to_string)
            .unwrap_or_default(),
        description: map
            .get("description")
            .and_then(value_to_string)
            .unwrap_or_default(),
        model_name: map
            .get("model_name")
            .or_else(|| map.get("modelName"))
            .and_then(value_to_string)
            .unwrap_or_default(),
        status: map
            .get("status")
            .and_then(value_to_string)
            .unwrap_or_else(default_status),
        priority: map
            .get("priority")
            .and_then(value_to_string)
            .unwrap_or_else(default_priority),
        requester_id: map
            .get("requester_id")
            .or_else(|| map.get("requesterId"))
            .and_then(value_to_string)
            .unwrap_or_default(),
        requester_name: map
            .get("requester_name")
            .or_else(|| map.get("requesterName"))
            .and_then(value_to_string)
            .unwrap_or_default(),
        checker_id,
        checker_name,
        approver_id: map
            .get("approver_id")
            .or_else(|| map.get("approverId"))
            .and_then(value_to_string)
            .unwrap_or_default(),
        approver_name: map
            .get("approver_name")
            .or_else(|| map.get("approverName"))
            .and_then(value_to_string)
            .unwrap_or_default(),
        reviewer_id,
        reviewer_name,
        components,
        attachments,
        review_comment: map
            .get("review_comment")
            .or_else(|| map.get("reviewComment"))
            .and_then(value_to_string),
        created_at: map
            .get("created_at")
            .or_else(|| map.get("createdAt"))
            .and_then(value_to_timestamp_millis)
            .unwrap_or_else(|| chrono::Utc::now().timestamp_millis()),
        updated_at: map
            .get("updated_at")
            .or_else(|| map.get("updatedAt"))
            .and_then(value_to_timestamp_millis)
            .unwrap_or_else(|| chrono::Utc::now().timestamp_millis()),
        due_date: map
            .get("due_date")
            .or_else(|| map.get("dueDate"))
            .and_then(value_to_timestamp_millis),
        current_node: map
            .get("current_node")
            .or_else(|| map.get("currentNode"))
            .and_then(value_to_string)
            .unwrap_or_else(default_current_node),
        workflow_history,
        return_reason: map
            .get("return_reason")
            .or_else(|| map.get("returnReason"))
            .and_then(value_to_string),
    })
}

fn review_tasks_from_values(values: Vec<Value>) -> (Vec<ReviewTask>, usize) {
    let mut parse_failures = 0;
    let tasks = values
        .into_iter()
        .filter_map(|value| match review_task_from_value(value.clone()) {
            Some(task) => Some(task),
            None => {
                parse_failures += 1;
                warn!("Skipping unreadable review task row: {}", value);
                None
            }
        })
        .collect();

    (tasks, parse_failures)
}

async fn query_review_task_page(
    where_clause: &str,
    bindings: &[(&'static str, String)],
    limit: i64,
    offset: i64,
) -> Result<Vec<ReviewTask>, surrealdb::Error> {
    let data_sql = format!(
        "SELECT * FROM review_tasks {} ORDER BY created_at DESC LIMIT {} START {}",
        where_clause, limit, offset
    );
    let mut q = project_primary_db().query(&data_sql);
    for (name, value) in bindings {
        q = q.bind((*name, value.clone()));
    }

    let mut response = q.await?;
    let rows: Vec<TaskRow> = response.take(0).unwrap_or_default();
    Ok(rows.into_iter().map(|row| row.to_review_task()).collect())
}

async fn recover_review_task_page_with_row_probes(
    where_clause: &str,
    bindings: &[(&'static str, String)],
    offset: i64,
    limit: i64,
    total: i64,
) -> (Vec<ReviewTask>, usize) {
    let mut recovered = Vec::new();
    let mut skipped_rows = 0usize;
    let mut probe_offset = offset.max(0);
    let probe_end = total.max(0);

    while probe_offset < probe_end && (recovered.len() as i64) < limit.max(0) {
        match query_review_task_page(where_clause, bindings, 1, probe_offset).await {
            Ok(mut rows) if !rows.is_empty() => recovered.push(rows.remove(0)),
            Ok(_) => skipped_rows += 1,
            Err(error) => {
                warn!(
                    "Failed to probe review task row at offset {} during fallback recovery: {}",
                    probe_offset, error
                );
                skipped_rows += 1;
            }
        }
        probe_offset += 1;
    }

    (recovered, skipped_rows)
}

fn datetime_to_millis(dt: &Option<surrealdb::types::Datetime>) -> i64 {
    dt.as_ref()
        .map(|d| d.timestamp_millis())
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis())
}

fn parse_datetime(s: &Option<String>) -> i64 {
    s.as_ref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.timestamp_millis())
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis())
}

fn parse_datetime_value(dt: &Option<surrealdb::types::Datetime>) -> i64 {
    dt.as_ref()
        .map(|value| value.timestamp_millis())
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis())
}

async fn lookup_task_form_id(id: &str) -> Option<String> {
    #[derive(Debug, Deserialize, SurrealValue)]
    struct TaskFormRow {
        form_id: Option<String>,
    }

    let mut resp = project_primary_db()
        .query("SELECT form_id FROM review_tasks WHERE record::id(id) = $id LIMIT 1")
        .bind(("id", id.to_string()))
        .await
        .ok()?;
    let rows: Vec<TaskFormRow> = resp.take(0).unwrap_or_default();
    rows.into_iter()
        .next()
        .and_then(|row| row.form_id)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

// ============================================================================
// Routes
// ============================================================================

pub fn create_review_api_routes() -> Router {
    use crate::web_api::jwt_auth::{REVIEW_AUTH_CONFIG, review_auth_middleware};
    use axum::middleware;

    Router::new()
        // 提资单 CRUD
        .route("/api/review/tasks", post(create_task))
        .route("/api/review/tasks", get(list_tasks))
        .route("/api/review/tasks/{id}", get(get_task))
        .route("/api/review/tasks/{id}", patch(update_task))
        .route("/api/review/tasks/{id}", delete(delete_task))
        // 审核操作
        .route("/api/review/tasks/{id}/start-review", post(start_review))
        .route("/api/review/tasks/{id}/approve", post(approve_task))
        .route("/api/review/tasks/{id}/reject", post(reject_task))
        .route("/api/review/tasks/{id}/cancel", post(cancel_task))
        .route("/api/review/tasks/{id}/history", get(get_task_history))
        // 多级审批流程 API
        .route("/api/review/tasks/{id}/submit", post(submit_to_next_node))
        .route("/api/review/tasks/{id}/return", post(return_to_node))
        .route("/api/review/tasks/{id}/workflow", get(get_workflow_history))
        // 确认记录 CRUD（修复路由冲突）
        .route("/api/review/records", post(create_record))
        .route(
            "/api/review/records/by-task/{task_id}",
            get(get_records_by_task),
        )
        .route(
            "/api/review/records/item/{record_id}",
            delete(delete_record),
        )
        .route(
            "/api/review/records/clear-task/{task_id}",
            delete(clear_records_by_task),
        )
        // 评论 CRUD（修复路由冲突）
        .route("/api/review/comments", post(create_comment))
        .route(
            "/api/review/comments/by-annotation/{annotation_id}",
            get(get_comments_by_annotation),
        )
        .route(
            "/api/review/comments/item/{comment_id}",
            delete(delete_comment),
        )
        // 附件 API
        .route("/api/review/attachments", post(upload_attachment))
        .route(
            "/api/review/attachments/{attachment_id}",
            delete(delete_attachment),
        )
        // 同步 API
        .route("/api/review/sync/export", post(export_review_data))
        .route("/api/review/sync/import", post(import_review_data))
        // 用户 API
        .route("/api/users", get(list_users))
        .route("/api/users/me", get(get_current_user))
        .route("/api/users/reviewers", get(get_reviewers))
        // 校审相关 API 默认强制 JWT；联调时可通过 review_auth.enabled=false 临时关闭
        .layer(middleware::from_fn_with_state(
            REVIEW_AUTH_CONFIG.clone(),
            review_auth_middleware,
        ))
}

// ============================================================================
// Handlers - 提资单 CRUD
// ============================================================================

/// POST /api/review/tasks - 创建提资单
async fn create_task(
    Extension(claims): Extension<TokenClaims>,
    Json(request): Json<CreateTaskRequest>,
) -> impl IntoResponse {
    info!("Creating review task: title={}", request.title);

    let task_id = format!("task-{}", uuid::Uuid::new_v4());
    let now = chrono::Utc::now().to_rfc3339();

    let requester_id = claims.user_id.clone();

    let checker_id = request
        .checker_id
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| request.reviewer_id.clone());
    let approver_id = request.approver_id.clone().unwrap_or_default();
    let resolved_names =
        resolve_create_task_names(&claims, &request, checker_id.as_str(), approver_id.as_str());
    let requester_name = resolved_names.requester_name.clone();

    let form_id = request
        .form_id
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(generate_form_id);

    let sql = r#"
        CREATE ONLY review_tasks SET
            id = $id,
            form_id = $form_id,
            title = $title,
            description = $description,
            model_name = $model_name,
            status = 'draft',
            priority = $priority,
            requester_id = $requester_id,
            requester_name = $requester_name,
            checker_id = $checker_id,
            checker_name = $checker_name,
            approver_id = $approver_id,
            approver_name = $approver_name,
            reviewer_id = $reviewer_id,
            reviewer_name = $reviewer_name,
            components = $components,
            attachments = $attachments,
            due_date = $due_date,
            current_node = 'sj',
            workflow_history = [],
            created_at = time::now(),
            updated_at = time::now()
    "#;

    let result = project_primary_db()
        .query(sql)
        .bind(("id", task_id.clone()))
        .bind(("form_id", form_id.clone()))
        .bind(("title", request.title.clone()))
        .bind(("description", request.description.clone()))
        .bind(("model_name", request.model_name.clone()))
        .bind(("priority", request.priority.clone()))
        .bind(("requester_id", requester_id.clone()))
        .bind(("requester_name", requester_name.clone()))
        .bind(("checker_id", checker_id.clone()))
        .bind(("checker_name", resolved_names.checker_name.clone()))
        .bind(("approver_id", approver_id.clone()))
        .bind(("approver_name", resolved_names.approver_name.clone()))
        .bind(("reviewer_id", request.reviewer_id.clone()))
        .bind(("reviewer_name", resolved_names.reviewer_name.clone()))
        .bind(("components", request.components.clone()))
        .bind(("attachments", request.attachments.clone()))
        .bind((
            "due_date",
            request
                .due_date
                .map(|d| chrono::DateTime::from_timestamp_millis(d).map(|dt| dt.to_rfc3339()))
                .flatten(),
        ))
        .await;

    match result {
        Ok(_response) => {
            // CREATE 成功，无需解析响应（避免 datetime 反序列化问题）
            info!("Created task: {}", task_id);

            let mut seen_refnos = HashSet::new();
            for comp in &request.components {
                let refno = comp.ref_no.trim();
                if refno.is_empty() || !seen_refnos.insert(refno.to_string()) {
                    continue;
                }
                let _ = project_primary_db()
                    .query(
                        r#"
                        CREATE ONLY review_form_model SET
                            form_id = $form_id,
                            model_refno = $model_refno,
                            created_at = time::now()
                        "#,
                    )
                    .bind(("form_id", form_id.clone()))
                    .bind(("model_refno", refno.to_string()))
                    .await;
            }

            if let Err(error) = sync_review_form_with_task_status(
                form_id.as_str(),
                Some(request.model_name.as_str()),
                Some(requester_id.as_str()),
                "create_task_backfill",
                "draft",
            )
            .await
            {
                warn!(
                    "Failed to sync review_forms after create_task, form_id={}: {}",
                    form_id, error
                );
            }

            let task = ReviewTask {
                id: task_id,
                form_id: form_id.clone(),
                title: request.title,
                description: request.description,
                model_name: request.model_name,
                status: "draft".to_string(),
                priority: request.priority,
                requester_id,
                requester_name,
                checker_id: checker_id.clone(),
                checker_name: resolved_names.checker_name.clone(),
                approver_id: approver_id.clone(),
                approver_name: resolved_names.approver_name.clone(),
                reviewer_id: request.reviewer_id,
                reviewer_name: resolved_names.reviewer_name.clone(),
                components: request.components,
                attachments: request.attachments,
                review_comment: None,
                created_at: chrono::Utc::now().timestamp_millis(),
                updated_at: chrono::Utc::now().timestamp_millis(),
                due_date: request.due_date,
                current_node: "sj".to_string(),
                workflow_history: vec![],
                return_reason: None,
            };
            (
                StatusCode::OK,
                Json(TaskResponse {
                    success: true,
                    task: Some(task),
                    error_message: None,
                }),
            )
        }
        Err(e) => {
            warn!("Failed to create task: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(TaskResponse {
                    success: false,
                    task: None,
                    error_message: Some(format!("创建提资单失败: {}", e)),
                }),
            )
        }
    }
}

/// GET /api/review/tasks - 获取任务列表
async fn list_tasks(Query(query): Query<TaskListQuery>) -> impl IntoResponse {
    info!("Listing review tasks");

    let mut conditions: Vec<&'static str> = vec!["(deleted IS NONE OR deleted = false)"];
    let mut bindings: Vec<(&'static str, String)> = vec![];

    if let Some(ref status) = query.status {
        if status != "all" {
            conditions.push("status = $status");
            bindings.push(("status", status.clone()));
        }
    }
    if let Some(ref priority) = query.priority {
        if priority != "all" {
            conditions.push("priority = $priority");
            bindings.push(("priority", priority.clone()));
        }
    }
    if let Some(ref requester_id) = query.requester_id {
        conditions.push("requester_id = $requester_id");
        bindings.push(("requester_id", requester_id.clone()));
    }
    if let Some(ref checker_id) = query.checker_id {
        conditions.push("(checker_id = $checker_id OR reviewer_id = $checker_id)");
        bindings.push(("checker_id", checker_id.clone()));
    }
    if let Some(ref approver_id) = query.approver_id {
        conditions.push("approver_id = $approver_id");
        bindings.push(("approver_id", approver_id.clone()));
    }
    if let Some(ref reviewer_id) = query.reviewer_id {
        conditions.push("(reviewer_id = $reviewer_id OR checker_id = $reviewer_id)");
        bindings.push(("reviewer_id", reviewer_id.clone()));
    }

    let where_clause = format!("WHERE {}", conditions.join(" AND "));

    let limit = query.limit.unwrap_or(100);
    let offset = query.offset.unwrap_or(0);

    let count_sql = format!(
        "SELECT count() AS total FROM review_tasks {} GROUP ALL",
        where_clause
    );
    let mut q = project_primary_db().query(&count_sql);
    for (name, value) in &bindings {
        q = q.bind((*name, value.clone()));
    }

    match q.await {
        Ok(mut response) => {
            #[derive(Debug, serde::Deserialize, SurrealValue)]
            struct CountRow {
                total: i64,
            }
            let count_rows: Vec<CountRow> = response.take(0).unwrap_or_default();
            let total = count_rows.first().map(|r| r.total).unwrap_or(0);

            let mut tasks =
                match query_review_task_page(&where_clause, &bindings, limit, offset).await {
                    Ok(rows) => rows,
                    Err(error) => {
                        warn!("Failed to query review task page: {}", error);
                        Vec::new()
                    }
                };
            let mut skipped_rows = 0usize;

            if tasks.is_empty() && total > 0 && limit > 1 {
                warn!(
                    "Review task page returned empty rows despite total={} (offset={}, limit={}); probing individual rows for recovery",
                    total, offset, limit
                );
                let (recovered_rows, recovered_skips) = recover_review_task_page_with_row_probes(
                    &where_clause,
                    &bindings,
                    offset,
                    limit,
                    total,
                )
                .await;
                tasks = recovered_rows;
                skipped_rows = recovered_skips;
            }

            let total = total.saturating_sub(skipped_rows as i64);

            (
                StatusCode::OK,
                Json(TaskListResponse {
                    success: true,
                    tasks,
                    total,
                    error_message: None,
                }),
            )
        }
        Err(e) => {
            warn!("Failed to list tasks: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(TaskListResponse {
                    success: false,
                    tasks: vec![],
                    total: 0,
                    error_message: Some(format!("获取任务列表失败: {}", e)),
                }),
            )
        }
    }
}

/// GET /api/review/tasks/:id - 获取任务详情
async fn get_task(Path(id): Path<String>) -> impl IntoResponse {
    info!("Getting task: {}", id);

    // 使用 record::id(id) 提取 key 进行比较
    let sql = "SELECT * FROM review_tasks WHERE record::id(id) = $id AND (deleted IS NONE OR deleted = false) LIMIT 1";

    match project_primary_db()
        .query(sql)
        .bind(("id", id.clone()))
        .await
    {
        Ok(mut response) => {
            let rows: Vec<TaskRow> = response.take(0).unwrap_or_default();
            if let Some(row) = rows.into_iter().next() {
                (
                    StatusCode::OK,
                    Json(TaskResponse {
                        success: true,
                        task: Some(row.to_review_task()),
                        error_message: None,
                    }),
                )
            } else {
                (
                    StatusCode::NOT_FOUND,
                    Json(TaskResponse {
                        success: false,
                        task: None,
                        error_message: Some(format!("任务不存在或已删除: {}", id)),
                    }),
                )
            }
        }
        Err(e) => {
            warn!("Failed to get task: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(TaskResponse {
                    success: false,
                    task: None,
                    error_message: Some(format!("获取任务失败: {}", e)),
                }),
            )
        }
    }
}

/// PATCH /api/review/tasks/:id - 更新任务
async fn update_task(
    Path(id): Path<String>,
    Json(request): Json<UpdateTaskRequest>,
) -> impl IntoResponse {
    info!("Updating task: {}", id);

    let mut updates = vec!["updated_at = time::now()"];

    if request.title.is_some() {
        updates.push("title = $title");
    }
    if request.description.is_some() {
        updates.push("description = $description");
    }
    if request.priority.is_some() {
        updates.push("priority = $priority");
    }
    if request.components.is_some() {
        updates.push("components = $components");
    }
    if request.due_date.is_some() {
        updates.push("due_date = $due_date");
    }
    if request.attachments.is_some() {
        updates.push("attachments = $attachments");
    }

    let sql = format!(
        "UPDATE review_tasks SET {} WHERE record::id(id) = $id AND (deleted IS NONE OR deleted = false)",
        updates.join(", ")
    );

    let mut q = project_primary_db().query(&sql).bind(("id", id.clone()));

    if let Some(ref title) = request.title {
        q = q.bind(("title", title.clone()));
    }
    if let Some(ref description) = request.description {
        q = q.bind(("description", description.clone()));
    }
    if let Some(ref priority) = request.priority {
        q = q.bind(("priority", priority.clone()));
    }
    if let Some(ref components) = request.components {
        q = q.bind(("components", components.clone()));
    }
    if let Some(due_date) = request.due_date {
        let dt = chrono::DateTime::from_timestamp_millis(due_date).map(|d| d.to_rfc3339());
        q = q.bind(("due_date", dt));
    }
    if let Some(ref attachments) = request.attachments {
        q = q.bind(("attachments", attachments.clone()));
    }

    match q.await {
        Ok(_) => {
            // 返回更新后的任务
            let get_sql = "SELECT * FROM review_tasks WHERE record::id(id) = $id AND (deleted IS NONE OR deleted = false)";
            if let Ok(mut resp) = project_primary_db()
                .query(get_sql)
                .bind(("id", id.clone()))
                .await
            {
                let rows: Vec<TaskRow> = resp.take(0).unwrap_or_default();
                if let Some(row) = rows.into_iter().next() {
                    return (
                        StatusCode::OK,
                        Json(TaskResponse {
                            success: true,
                            task: Some(row.to_review_task()),
                            error_message: None,
                        }),
                    );
                }
            }
            (
                StatusCode::OK,
                Json(TaskResponse {
                    success: true,
                    task: None,
                    error_message: Some("更新成功但无法读取任务（可能已删除）".to_string()),
                }),
            )
        }
        Err(e) => {
            warn!("Failed to update task: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(TaskResponse {
                    success: false,
                    task: None,
                    error_message: Some(format!("更新任务失败: {}", e)),
                }),
            )
        }
    }
}

/// DELETE /api/review/tasks/:id - 软删除任务（与 PMS 入站删除一致；不向 PMS 回调）
async fn delete_task(Path(id): Path<String>) -> impl IntoResponse {
    info!("Soft-deleting task: {}", id);
    let form_id = lookup_task_form_id(&id).await;

    let soft_sql = r#"
        UPDATE review_tasks SET
            deleted = true,
            deleted_at = time::now(),
            updated_at = time::now(),
            status = 'deleted'
        WHERE record::id(id) = $id
    "#;

    match project_primary_db()
        .query(soft_sql)
        .bind(("id", id.clone()))
        .await
    {
        Ok(_) => {
            if let Some(form_id) = form_id.as_deref() {
                if let Err(error) = mark_review_form_deleted(form_id).await {
                    warn!(
                        "Failed to mark review_form deleted after task soft-delete, form_id={}: {}",
                        form_id, error
                    );
                }
            }
            (
                StatusCode::OK,
                Json(ActionResponse {
                    success: true,
                    message: Some("任务已软删除".to_string()),
                    error_message: None,
                }),
            )
        }
        Err(e) => {
            warn!("Failed to soft-delete task: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ActionResponse {
                    success: false,
                    message: None,
                    error_message: Some(format!("软删除任务失败: {}", e)),
                }),
            )
        }
    }
}

// ============================================================================
// Handlers - 审核操作
// ============================================================================

/// POST /api/review/tasks/:id/start-review - 开始审核（兼容旧 API，映射到 jd 节点）
async fn start_review(Path(id): Path<String>) -> impl IntoResponse {
    update_task_status(id, "in_review".to_string(), Some("jd".to_string()), None).await
}

/// POST /api/review/tasks/:id/approve - 通过审核（兼容旧 API，映射到 approved + pz 节点）
async fn approve_task(
    Path(id): Path<String>,
    Json(request): Json<ReviewActionRequest>,
) -> impl IntoResponse {
    update_task_status(
        id,
        "approved".to_string(),
        Some("pz".to_string()),
        request.comment,
    )
    .await
}

/// POST /api/review/tasks/:id/reject - 驳回审核（兼容旧 API，驳回到 sj 节点）
async fn reject_task(
    Path(id): Path<String>,
    Json(request): Json<ReviewActionRequest>,
) -> impl IntoResponse {
    update_task_status(
        id,
        "rejected".to_string(),
        Some("sj".to_string()),
        request.comment,
    )
    .await
}

/// POST /api/review/tasks/:id/cancel - 取消任务
async fn cancel_task(
    Path(id): Path<String>,
    Json(request): Json<ReviewActionRequest>,
) -> impl IntoResponse {
    update_task_status(id, "cancelled".to_string(), None, request.reason).await
}

async fn update_task_status(
    id: String,
    status: String,
    target_node: Option<String>,
    comment: Option<String>,
) -> (StatusCode, Json<ActionResponse>) {
    info!(
        "Updating task {} status to {}, node to {:?}",
        id, status, target_node
    );
    let form_id = lookup_task_form_id(&id).await;

    let sql = match (&target_node, &comment) {
        (Some(_), Some(_)) => {
            "UPDATE review_tasks SET status = $status, current_node = $node, review_comment = $comment, updated_at = time::now() WHERE record::id(id) = $id AND (deleted IS NONE OR deleted = false)"
        }
        (Some(_), None) => {
            "UPDATE review_tasks SET status = $status, current_node = $node, updated_at = time::now() WHERE record::id(id) = $id AND (deleted IS NONE OR deleted = false)"
        }
        (None, Some(_)) => {
            "UPDATE review_tasks SET status = $status, review_comment = $comment, updated_at = time::now() WHERE record::id(id) = $id AND (deleted IS NONE OR deleted = false)"
        }
        (None, None) => {
            "UPDATE review_tasks SET status = $status, updated_at = time::now() WHERE record::id(id) = $id AND (deleted IS NONE OR deleted = false)"
        }
    };

    let mut q = project_primary_db()
        .query(sql)
        .bind(("id", id.clone()))
        .bind(("status", status.clone()));

    if let Some(ref node) = target_node {
        q = q.bind(("node", node.clone()));
    }
    if let Some(ref c) = comment {
        q = q.bind(("comment", c.clone()));
    }

    match q.await {
        Ok(_) => {
            if let Some(form_id) = form_id.as_deref() {
                if let Err(error) = sync_review_form_with_task_status(
                    form_id,
                    None,
                    None,
                    "create_task_backfill",
                    status.as_str(),
                )
                .await
                {
                    warn!(
                        "Failed to sync review_forms after status update, form_id={}: {}",
                        form_id, error
                    );
                }
            }
            let history_sql = r#"
                CREATE review_history CONTENT {
                    task_id: $task_id,
                    action: $action,
                    user_id: 'system',
                    user_name: '系统',
                    comment: $comment,
                    timestamp: time::now()
                }
            "#;
            let _ = project_primary_db()
                .query(history_sql)
                .bind(("task_id", id))
                .bind(("action", status.clone()))
                .bind(("comment", comment))
                .await;

            (
                StatusCode::OK,
                Json(ActionResponse {
                    success: true,
                    message: Some(format!("任务状态已更新为: {}", status)),
                    error_message: None,
                }),
            )
        }
        Err(e) => {
            warn!("Failed to update task status: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ActionResponse {
                    success: false,
                    message: None,
                    error_message: Some(format!("更新状态失败: {}", e)),
                }),
            )
        }
    }
}

/// GET /api/review/tasks/:id/history - 获取审核历史
async fn get_task_history(Path(id): Path<String>) -> impl IntoResponse {
    info!("Getting task history: {}", id);

    #[derive(Debug, Serialize)]
    struct HistoryItem {
        id: String,
        task_id: String,
        action: String,
        user_id: String,
        user_name: String,
        comment: Option<String>,
        #[serde(serialize_with = "serialize_beijing_datetime_millis")]
        timestamp: i64,
    }

    #[derive(Debug, Serialize)]
    struct HistoryResponse {
        success: bool,
        history: Vec<HistoryItem>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error_message: Option<String>,
    }

    #[derive(Debug, Deserialize, SurrealValue)]
    struct HistoryRow {
        id: surrealdb::types::RecordId,
        task_id: Option<String>,
        action: Option<String>,
        operator_id: Option<String>,
        operator_name: Option<String>,
        comment: Option<String>,
        timestamp: Option<surrealdb::types::Datetime>,
    }

    let sql =
        "SELECT * FROM review_workflow_history WHERE task_id = $task_id ORDER BY timestamp DESC";

    match project_primary_db()
        .query(sql)
        .bind(("task_id", id.clone()))
        .await
    {
        Ok(mut response) => {
            let rows: Vec<HistoryRow> = response.take(0).unwrap_or_default();
            let history: Vec<HistoryItem> = rows
                .into_iter()
                .map(|r| HistoryItem {
                    id: format!("{:?}", r.id.key),
                    task_id: r.task_id.unwrap_or_default(),
                    action: r.action.unwrap_or_default(),
                    user_id: r.operator_id.unwrap_or_default(),
                    user_name: r.operator_name.unwrap_or_default(),
                    comment: r.comment,
                    timestamp: parse_datetime_value(&r.timestamp),
                })
                .collect();

            (
                StatusCode::OK,
                Json(HistoryResponse {
                    success: true,
                    history,
                    error_message: None,
                }),
            )
        }
        Err(e) => {
            warn!("Failed to get task history: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(HistoryResponse {
                    success: false,
                    history: vec![],
                    error_message: Some(format!("获取历史失败: {}", e)),
                }),
            )
        }
    }
}

// ============================================================================
// Handlers - 确认记录 CRUD
// ============================================================================

/// 确认记录数据
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmedRecordData {
    pub task_id: String,
    #[serde(default)]
    pub r#type: String,
    #[serde(default)]
    pub annotations: Vec<serde_json::Value>,
    #[serde(default)]
    pub cloud_annotations: Vec<serde_json::Value>,
    #[serde(default)]
    pub rect_annotations: Vec<serde_json::Value>,
    #[serde(default)]
    pub obb_annotations: Vec<serde_json::Value>,
    #[serde(default)]
    pub measurements: Vec<serde_json::Value>,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmedRecordResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<ConfirmedRecordWithMeta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub records: Option<Vec<ConfirmedRecordWithMeta>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmedRecordWithMeta {
    pub id: String,
    pub task_id: String,
    pub r#type: String,
    pub annotations: Vec<serde_json::Value>,
    pub cloud_annotations: Vec<serde_json::Value>,
    pub rect_annotations: Vec<serde_json::Value>,
    pub obb_annotations: Vec<serde_json::Value>,
    pub measurements: Vec<serde_json::Value>,
    pub note: String,
    pub confirmed_at: i64,
}

/// POST /api/review/records - 保存确认记录
async fn create_record(Json(request): Json<ConfirmedRecordData>) -> impl IntoResponse {
    info!("Creating confirmed record for task: {}", request.task_id);

    let record_id = format!("record-{}", uuid::Uuid::new_v4());

    let sql = r#"
        CREATE review_records CONTENT {
            id: $id,
            task_id: $task_id,
            type: $type,
            annotations: $annotations,
            cloud_annotations: $cloud_annotations,
            rect_annotations: $rect_annotations,
            obb_annotations: $obb_annotations,
            measurements: $measurements,
            note: $note,
            confirmed_at: time::now()
        }
    "#;

    match project_primary_db()
        .query(sql)
        .bind(("id", record_id.clone()))
        .bind(("task_id", request.task_id.clone()))
        .bind(("type", request.r#type.clone()))
        .bind(("annotations", request.annotations.clone()))
        .bind(("cloud_annotations", request.cloud_annotations.clone()))
        .bind(("rect_annotations", request.rect_annotations.clone()))
        .bind(("obb_annotations", request.obb_annotations.clone()))
        .bind(("measurements", request.measurements.clone()))
        .bind(("note", request.note.clone()))
        .await
    {
        Ok(_) => {
            let record = ConfirmedRecordWithMeta {
                id: record_id,
                task_id: request.task_id,
                r#type: request.r#type,
                annotations: request.annotations,
                cloud_annotations: request.cloud_annotations,
                rect_annotations: request.rect_annotations,
                obb_annotations: request.obb_annotations,
                measurements: request.measurements,
                note: request.note,
                confirmed_at: chrono::Utc::now().timestamp_millis(),
            };
            (
                StatusCode::OK,
                Json(ConfirmedRecordResponse {
                    success: true,
                    record: Some(record),
                    records: None,
                    error_message: None,
                }),
            )
        }
        Err(e) => {
            warn!("Failed to create record: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ConfirmedRecordResponse {
                    success: false,
                    record: None,
                    records: None,
                    error_message: Some(format!("保存记录失败: {}", e)),
                }),
            )
        }
    }
}

/// GET /api/review/records/:task_id - 获取任务的确认记录
async fn get_records_by_task(Path(task_id): Path<String>) -> impl IntoResponse {
    info!("Getting records for task: {}", task_id);

    #[derive(Debug, Deserialize, SurrealValue)]
    struct RecordRow {
        id: surrealdb::types::RecordId,
        task_id: Option<String>,
        r#type: Option<String>,
        annotations: Option<Vec<serde_json::Value>>,
        cloud_annotations: Option<Vec<serde_json::Value>>,
        rect_annotations: Option<Vec<serde_json::Value>>,
        obb_annotations: Option<Vec<serde_json::Value>>,
        measurements: Option<Vec<serde_json::Value>>,
        note: Option<String>,
        confirmed_at: Option<surrealdb::types::Datetime>,
    }

    let sql = "SELECT * FROM review_records WHERE task_id = $task_id ORDER BY confirmed_at DESC";

    match project_primary_db()
        .query(sql)
        .bind(("task_id", task_id))
        .await
    {
        Ok(mut response) => {
            let rows: Vec<RecordRow> = response.take(0).unwrap_or_default();
            let records: Vec<ConfirmedRecordWithMeta> = rows
                .into_iter()
                .map(|r| ConfirmedRecordWithMeta {
                    id: format!("{:?}", r.id.key),
                    task_id: r.task_id.unwrap_or_default(),
                    r#type: r.r#type.unwrap_or_else(|| "batch".to_string()),
                    annotations: r.annotations.unwrap_or_default(),
                    cloud_annotations: r.cloud_annotations.unwrap_or_default(),
                    rect_annotations: r.rect_annotations.unwrap_or_default(),
                    obb_annotations: r.obb_annotations.unwrap_or_default(),
                    measurements: r.measurements.unwrap_or_default(),
                    note: r.note.unwrap_or_default(),
                    confirmed_at: parse_datetime_value(&r.confirmed_at),
                })
                .collect();

            (
                StatusCode::OK,
                Json(ConfirmedRecordResponse {
                    success: true,
                    record: None,
                    records: Some(records),
                    error_message: None,
                }),
            )
        }
        Err(e) => {
            warn!("Failed to get records: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ConfirmedRecordResponse {
                    success: false,
                    record: None,
                    records: None,
                    error_message: Some(format!("获取记录失败: {}", e)),
                }),
            )
        }
    }
}

/// DELETE /api/review/records/:record_id - 删除记录
async fn delete_record(Path(record_id): Path<String>) -> impl IntoResponse {
    info!("Deleting record: {}", record_id);

    let sql = "DELETE [type::record('review_records', $id)]";

    match project_primary_db()
        .query(sql)
        .bind(("id", record_id))
        .await
    {
        Ok(_) => (
            StatusCode::OK,
            Json(ActionResponse {
                success: true,
                message: Some("记录已删除".to_string()),
                error_message: None,
            }),
        ),
        Err(e) => {
            warn!("Failed to delete record: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ActionResponse {
                    success: false,
                    message: None,
                    error_message: Some(format!("删除记录失败: {}", e)),
                }),
            )
        }
    }
}

/// DELETE /api/review/records/task/:task_id - 清空任务的所有记录
async fn clear_records_by_task(Path(task_id): Path<String>) -> impl IntoResponse {
    info!("Clearing records for task: {}", task_id);

    let sql = r#"
        LET $ids = SELECT VALUE id FROM review_records WHERE task_id = $task_id;
        DELETE $ids;
    "#;

    match project_primary_db()
        .query(sql)
        .bind(("task_id", task_id))
        .await
    {
        Ok(_) => (
            StatusCode::OK,
            Json(ActionResponse {
                success: true,
                message: Some("记录已清空".to_string()),
                error_message: None,
            }),
        ),
        Err(e) => {
            warn!("Failed to clear records: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ActionResponse {
                    success: false,
                    message: None,
                    error_message: Some(format!("清空记录失败: {}", e)),
                }),
            )
        }
    }
}

// ============================================================================
// Handlers - 评论 CRUD
// ============================================================================

/// 评论数据
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationComment {
    pub id: String,
    pub annotation_id: String,
    pub annotation_type: String,
    pub author_id: String,
    pub author_name: String,
    pub author_role: String,
    pub content: String,
    pub reply_to_id: Option<String>,
    #[serde(serialize_with = "serialize_beijing_datetime_millis")]
    pub created_at: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCommentRequest {
    pub annotation_id: String,
    pub annotation_type: String,
    pub author_id: String,
    pub author_name: String,
    pub author_role: String,
    pub content: String,
    pub reply_to_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CommentResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<AnnotationComment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments: Option<Vec<AnnotationComment>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CommentQuery {
    pub r#type: Option<String>,
}

/// POST /api/review/comments - 添加评论
async fn create_comment(Json(request): Json<CreateCommentRequest>) -> impl IntoResponse {
    info!("Creating comment for annotation: {}", request.annotation_id);

    let comment_id = format!("comment-{}", uuid::Uuid::new_v4());

    let sql = r#"
        CREATE review_comments CONTENT {
            id: $id,
            annotation_id: $annotation_id,
            annotation_type: $annotation_type,
            author_id: $author_id,
            author_name: $author_name,
            author_role: $author_role,
            content: $content,
            reply_to_id: $reply_to_id,
            created_at: time::now()
        }
    "#;

    match project_primary_db()
        .query(sql)
        .bind(("id", comment_id.clone()))
        .bind(("annotation_id", request.annotation_id.clone()))
        .bind(("annotation_type", request.annotation_type.clone()))
        .bind(("author_id", request.author_id.clone()))
        .bind(("author_name", request.author_name.clone()))
        .bind(("author_role", request.author_role.clone()))
        .bind(("content", request.content.clone()))
        .bind(("reply_to_id", request.reply_to_id.clone()))
        .await
    {
        Ok(_) => {
            let comment = AnnotationComment {
                id: comment_id,
                annotation_id: request.annotation_id,
                annotation_type: request.annotation_type,
                author_id: request.author_id,
                author_name: request.author_name,
                author_role: request.author_role,
                content: request.content,
                reply_to_id: request.reply_to_id,
                created_at: chrono::Utc::now().timestamp_millis(),
            };
            (
                StatusCode::OK,
                Json(CommentResponse {
                    success: true,
                    comment: Some(comment),
                    comments: None,
                    error_message: None,
                }),
            )
        }
        Err(e) => {
            warn!("Failed to create comment: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(CommentResponse {
                    success: false,
                    comment: None,
                    comments: None,
                    error_message: Some(format!("创建评论失败: {}", e)),
                }),
            )
        }
    }
}

/// GET /api/review/comments/:annotation_id - 获取批注评论
async fn get_comments_by_annotation(
    Path(annotation_id): Path<String>,
    Query(query): Query<CommentQuery>,
) -> impl IntoResponse {
    info!("Getting comments for annotation: {}", annotation_id);

    #[derive(Debug, Deserialize, SurrealValue)]
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

    let sql = if query.r#type.is_some() {
        "SELECT * FROM review_comments WHERE annotation_id = $annotation_id AND annotation_type = $type ORDER BY created_at ASC"
    } else {
        "SELECT * FROM review_comments WHERE annotation_id = $annotation_id ORDER BY created_at ASC"
    };

    let mut q = project_primary_db()
        .query(sql)
        .bind(("annotation_id", annotation_id));
    if let Some(ref t) = query.r#type {
        q = q.bind(("type", t.clone()));
    }

    match q.await {
        Ok(mut response) => {
            let rows: Vec<CommentRow> = response.take(0).unwrap_or_default();
            let comments: Vec<AnnotationComment> = rows
                .into_iter()
                .map(|r| AnnotationComment {
                    id: format!("{:?}", r.id.key),
                    annotation_id: r.annotation_id.unwrap_or_default(),
                    annotation_type: r.annotation_type.unwrap_or_default(),
                    author_id: r.author_id.unwrap_or_default(),
                    author_name: r.author_name.unwrap_or_default(),
                    author_role: r.author_role.unwrap_or_default(),
                    content: r.content.unwrap_or_default(),
                    reply_to_id: r.reply_to_id,
                    created_at: parse_datetime_value(&r.created_at),
                })
                .collect();

            (
                StatusCode::OK,
                Json(CommentResponse {
                    success: true,
                    comment: None,
                    comments: Some(comments),
                    error_message: None,
                }),
            )
        }
        Err(e) => {
            warn!("Failed to get comments: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(CommentResponse {
                    success: false,
                    comment: None,
                    comments: None,
                    error_message: Some(format!("获取评论失败: {}", e)),
                }),
            )
        }
    }
}

/// DELETE /api/review/comments/:comment_id - 删除评论
async fn delete_comment(Path(comment_id): Path<String>) -> impl IntoResponse {
    info!("Deleting comment: {}", comment_id);

    let sql = "DELETE [type::record('review_comments', $id)]";

    match project_primary_db()
        .query(sql)
        .bind(("id", comment_id))
        .await
    {
        Ok(_) => (
            StatusCode::OK,
            Json(ActionResponse {
                success: true,
                message: Some("评论已删除".to_string()),
                error_message: None,
            }),
        ),
        Err(e) => {
            warn!("Failed to delete comment: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ActionResponse {
                    success: false,
                    message: None,
                    error_message: Some(format!("删除评论失败: {}", e)),
                }),
            )
        }
    }
}

// ============================================================================
// Handlers - 用户 API
// ============================================================================

/// 用户信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
    pub name: String,
    pub email: String,
    pub role: String,
    pub department: Option<String>,
    pub avatar: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UserListResponse {
    pub success: bool,
    pub users: Vec<User>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UserResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<User>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UserListQuery {
    pub role: Option<String>,
    pub status: Option<String>,
}

fn build_mock_review_users() -> Vec<User> {
    vec![
        User {
            id: "designer_001".to_string(),
            username: "designer".to_string(),
            name: "王设计师".to_string(),
            email: "designer@company.com".to_string(),
            role: "designer".to_string(),
            department: Some("设计部".to_string()),
            avatar: None,
        },
        User {
            id: "proofreader_001".to_string(),
            username: "proofreader".to_string(),
            name: "张校对员".to_string(),
            email: "proofreader@company.com".to_string(),
            role: "proofreader".to_string(),
            department: Some("质量部".to_string()),
            avatar: None,
        },
        User {
            id: "reviewer_001".to_string(),
            username: "reviewer".to_string(),
            name: "李审核员".to_string(),
            email: "reviewer@company.com".to_string(),
            role: "reviewer".to_string(),
            department: Some("技术部".to_string()),
            avatar: None,
        },
        User {
            id: "manager_001".to_string(),
            username: "manager".to_string(),
            name: "陈经理".to_string(),
            email: "manager@company.com".to_string(),
            role: "manager".to_string(),
            department: Some("工程部".to_string()),
            avatar: None,
        },
        User {
            id: "PZ".to_string(),
            username: "PZ".to_string(),
            name: "PZ批准人".to_string(),
            email: "pz@company.com".to_string(),
            role: "manager".to_string(),
            department: Some("工程部".to_string()),
            avatar: None,
        },
        User {
            id: "admin_001".to_string(),
            username: "admin".to_string(),
            name: "系统管理员".to_string(),
            email: "admin@company.com".to_string(),
            role: "admin".to_string(),
            department: Some("信息技术部".to_string()),
            avatar: None,
        },
    ]
}

fn default_mock_user() -> User {
    build_mock_review_users()
        .into_iter()
        .find(|user| user.role == "designer")
        .unwrap_or(User {
            id: "designer_001".to_string(),
            username: "designer".to_string(),
            name: "王设计师".to_string(),
            email: "designer@company.com".to_string(),
            role: "designer".to_string(),
            department: Some("设计部".to_string()),
            avatar: None,
        })
}

fn map_claim_role_to_user_role(role: Option<&str>) -> String {
    match role.unwrap_or("viewer") {
        "sj" => "designer".to_string(),
        "jd" => "proofreader".to_string(),
        "sh" => "reviewer".to_string(),
        "pz" => "manager".to_string(),
        "admin" => "admin".to_string(),
        other => other.to_string(),
    }
}

fn current_user_from_claims(claims: &crate::web_api::jwt_auth::TokenClaims) -> User {
    build_mock_review_users()
        .into_iter()
        .find(|user| user.id == claims.user_id)
        .unwrap_or(User {
            id: claims.user_id.clone(),
            username: claims.user_id.clone(),
            name: preferred_name(Some(claims.user_name.as_str()), claims.user_id.as_str()),
            email: format!("{}@example.com", claims.user_id),
            role: map_claim_role_to_user_role(claims.role.as_deref()),
            department: None,
            avatar: None,
        })
}

/// GET /api/users - 获取用户列表
async fn list_users(Query(query): Query<UserListQuery>) -> impl IntoResponse {
    info!("Listing users");

    let mock_users = build_mock_review_users();

    let users = if let Some(ref role) = query.role {
        mock_users.into_iter().filter(|u| &u.role == role).collect()
    } else {
        mock_users
    };

    (
        StatusCode::OK,
        Json(UserListResponse {
            success: true,
            users,
            error_message: None,
        }),
    )
}

/// GET /api/users/me - 获取当前用户
async fn get_current_user(request: axum::extract::Request) -> impl IntoResponse {
    // 尝试从 JWT Claims 获取用户信息
    use crate::web_api::jwt_auth::TokenClaims;

    if let Some(claims) = request.extensions().get::<TokenClaims>() {
        let user = current_user_from_claims(claims);
        return (
            StatusCode::OK,
            Json(UserResponse {
                success: true,
                user: Some(user),
                error_message: None,
            }),
        );
    }

    // 如果没有 JWT，返回 mock 用户
    let user = default_mock_user();

    (
        StatusCode::OK,
        Json(UserResponse {
            success: true,
            user: Some(user),
            error_message: None,
        }),
    )
}

/// GET /api/users/reviewers - 获取审核人员列表
async fn get_reviewers() -> impl IntoResponse {
    info!("Getting reviewers");

    let reviewers = build_mock_review_users()
        .into_iter()
        .filter(|user| {
            matches!(
                user.role.as_str(),
                "proofreader" | "reviewer" | "manager" | "admin"
            )
        })
        .collect();

    (
        StatusCode::OK,
        Json(UserListResponse {
            success: true,
            users: reviewers,
            error_message: None,
        }),
    )
}

// ============================================================================
// Handlers - 多级审批流程
// ============================================================================

/// 获取下一个节点
fn get_next_node(current: &str) -> Option<&'static str> {
    match current {
        "sj" => Some("jd"),
        "jd" => Some("sh"),
        "sh" => Some("pz"),
        "pz" => None, // 已是最后节点
        _ => None,
    }
}

/// 验证是否可以驳回到目标节点
fn can_return_to(current: &str, target: &str) -> bool {
    let current_idx = WORKFLOW_NODES.iter().position(|&n| n == current);
    let target_idx = WORKFLOW_NODES.iter().position(|&n| n == target);
    match (current_idx, target_idx) {
        (Some(c), Some(t)) => t < c,
        _ => false,
    }
}

/// POST /api/review/tasks/:id/submit - 提交到下一节点
async fn submit_to_next_node(
    Extension(claims): Extension<TokenClaims>,
    Path(id): Path<String>,
    Json(request): Json<SubmitToNextRequest>,
) -> impl IntoResponse {
    info!(
        "Submitting task {} to next node, operator={}",
        id, claims.user_id
    );

    // 1. 获取当前任务
    let get_sql = "SELECT * FROM review_tasks WHERE record::id(id) = $id AND (deleted IS NONE OR deleted = false) LIMIT 1";
    let task_result = project_primary_db()
        .query(get_sql)
        .bind(("id", id.clone()))
        .await;

    let task_row = match task_result {
        Ok(mut resp) => {
            let rows: Vec<TaskRow> = resp.take(0).unwrap_or_default();
            match rows.into_iter().next() {
                Some(row) => row,
                None => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(ActionResponse {
                            success: false,
                            message: None,
                            error_message: Some(format!("任务不存在或已删除: {}", id)),
                        }),
                    );
                }
            }
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ActionResponse {
                    success: false,
                    message: None,
                    error_message: Some(format!("查询任务失败: {}", e)),
                }),
            );
        }
    };

    let current_node = task_row
        .current_node
        .clone()
        .unwrap_or_else(|| "sj".to_string());

    // 1.1 权限校验：检查当前用户是否为本节点负责人
    let operator_user = &claims.user_id;
    let has_permission = match current_node.as_str() {
        "sj" => task_row.requester_id.as_deref() == Some(operator_user),
        "jd" => {
            task_row.checker_id.as_deref() == Some(operator_user)
                || task_row.reviewer_id.as_deref() == Some(operator_user)
        }
        "sh" => task_row.approver_id.as_deref() == Some(operator_user),
        "pz" => task_row.approver_id.as_deref() == Some(operator_user),
        _ => false,
    };

    if !has_permission {
        return (
            StatusCode::FORBIDDEN,
            Json(ActionResponse {
                success: false,
                message: None,
                error_message: Some(format!(
                    "权限不足：用户 {} 不是「{}」节点的负责人",
                    operator_user,
                    get_node_display_name(&current_node)
                )),
            }),
        );
    }

    // 2. 操作人信息
    let op_id = if claims.user_id.is_empty() {
        request
            .operator_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("system")
    } else {
        &claims.user_id
    };
    let op_name = request
        .operator_name
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(op_id);

    // 3. 判断是否为最终节点批准，还是向下流转
    let (next_node_str, next_status, action_label) = if current_node == "pz" {
        // pz 是最终节点：批准完成
        ("pz".to_string(), "approved", "approve")
    } else {
        match get_next_node(&current_node) {
            Some(n) => {
                let status = match n {
                    "jd" => "submitted",
                    "sh" | "pz" => "in_review",
                    _ => "submitted",
                };
                (n.to_string(), status, "submit")
            }
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ActionResponse {
                        success: false,
                        message: None,
                        error_message: Some("当前已是最后节点，无法继续提交".to_string()),
                    }),
                );
            }
        }
    };

    // 4. 更新任务节点和状态
    let update_sql = r#"
        UPDATE review_tasks SET
            current_node = $next_node,
            status = $status,
            return_reason = NONE,
            updated_at = time::now()
        WHERE record::id(id) = $id AND (deleted IS NONE OR deleted = false)
    "#;

    if let Err(e) = project_primary_db()
        .query(update_sql)
        .bind(("id", id.clone()))
        .bind(("next_node", next_node_str.clone()))
        .bind(("status", next_status))
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ActionResponse {
                success: false,
                message: None,
                error_message: Some(format!("更新任务失败: {}", e)),
            }),
        );
    }

    if let Some(form_id) = task_row
        .form_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Err(error) = sync_review_form_with_task_status(
            form_id,
            Some(task_row.model_name.as_deref().unwrap_or_default()),
            Some(task_row.requester_id.as_deref().unwrap_or_default()),
            "create_task_backfill",
            next_status,
        )
        .await
        {
            warn!(
                "Failed to sync review_forms after submit_to_next_node, form_id={}: {}",
                form_id, error
            );
        }
    }

    // 5. 记录工作流历史
    let history_sql = r#"
        CREATE review_workflow_history CONTENT {
            task_id: $task_id,
            node: $from_node,
            action: $action,
            operator_id: $operator_id,
            operator_name: $operator_name,
            comment: $comment,
            timestamp: time::now()
        }
    "#;

    let _ = project_primary_db()
        .query(history_sql)
        .bind(("task_id", id.clone()))
        .bind(("from_node", current_node.clone()))
        .bind(("action", action_label.to_string()))
        .bind(("operator_id", op_id.to_string()))
        .bind(("operator_name", op_name.to_string()))
        .bind(("comment", request.comment))
        .await;

    let from_name = get_node_display_name(&current_node);

    if current_node == "pz" {
        (
            StatusCode::OK,
            Json(ActionResponse {
                success: true,
                message: Some(format!("「{}」已批准，审批流程完成", from_name)),
                error_message: None,
            }),
        )
    } else {
        let to_name = get_node_display_name(&next_node_str);
        (
            StatusCode::OK,
            Json(ActionResponse {
                success: true,
                message: Some(format!("已从「{}」提交到「{}」", from_name, to_name)),
                error_message: None,
            }),
        )
    }
}

/// POST /api/review/tasks/:id/return - 驳回到指定节点
async fn return_to_node(
    Extension(claims): Extension<TokenClaims>,
    Path(id): Path<String>,
    Json(request): Json<ReturnRequest>,
) -> impl IntoResponse {
    info!(
        "Returning task {} to node {}, operator={}",
        id, request.target_node, claims.user_id
    );

    // 1. 获取当前任务
    let get_sql = "SELECT * FROM review_tasks WHERE record::id(id) = $id AND (deleted IS NONE OR deleted = false) LIMIT 1";
    let task_result = project_primary_db()
        .query(get_sql)
        .bind(("id", id.clone()))
        .await;

    let task_row = match task_result {
        Ok(mut resp) => {
            let rows: Vec<TaskRow> = resp.take(0).unwrap_or_default();
            match rows.into_iter().next() {
                Some(row) => row,
                None => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(ActionResponse {
                            success: false,
                            message: None,
                            error_message: Some(format!("任务不存在或已删除: {}", id)),
                        }),
                    );
                }
            }
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ActionResponse {
                    success: false,
                    message: None,
                    error_message: Some(format!("查询任务失败: {}", e)),
                }),
            );
        }
    };

    let current_node = task_row
        .current_node
        .clone()
        .unwrap_or_else(|| "sj".to_string());

    // 1.1 权限校验
    let operator_user = &claims.user_id;
    let has_permission = match current_node.as_str() {
        "jd" => {
            task_row.checker_id.as_deref() == Some(operator_user)
                || task_row.reviewer_id.as_deref() == Some(operator_user)
        }
        "sh" | "pz" => task_row.approver_id.as_deref() == Some(operator_user),
        _ => false,
    };

    if !has_permission {
        return (
            StatusCode::FORBIDDEN,
            Json(ActionResponse {
                success: false,
                message: None,
                error_message: Some(format!(
                    "权限不足：用户 {} 不是「{}」节点的负责人",
                    operator_user,
                    get_node_display_name(&current_node)
                )),
            }),
        );
    }

    // 2. 验证目标节点
    if !can_return_to(&current_node, &request.target_node) {
        let from_name = get_node_display_name(&current_node);
        let to_name = get_node_display_name(&request.target_node);
        return (
            StatusCode::BAD_REQUEST,
            Json(ActionResponse {
                success: false,
                message: None,
                error_message: Some(format!("无法从「{}」驳回到「{}」", from_name, to_name)),
            }),
        );
    }

    // 3. 更新任务节点和驳回原因
    let next_status = match request.target_node.as_str() {
        "sj" => "draft",
        "jd" => "submitted",
        "sh" | "pz" => "in_review",
        _ => "draft",
    };

    let update_sql = r#"
        UPDATE review_tasks SET
            current_node = $target_node,
            status = $status,
            return_reason = $reason,
            updated_at = time::now()
        WHERE record::id(id) = $id AND (deleted IS NONE OR deleted = false)
    "#;

    if let Err(e) = project_primary_db()
        .query(update_sql)
        .bind(("id", id.clone()))
        .bind(("target_node", request.target_node.clone()))
        .bind(("status", next_status))
        .bind(("reason", request.reason.clone()))
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ActionResponse {
                success: false,
                message: None,
                error_message: Some(format!("更新任务失败: {}", e)),
            }),
        );
    }

    if let Some(form_id) = task_row
        .form_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Err(error) = sync_review_form_with_task_status(
            form_id,
            Some(task_row.model_name.as_deref().unwrap_or_default()),
            Some(task_row.requester_id.as_deref().unwrap_or_default()),
            "create_task_backfill",
            next_status,
        )
        .await
        {
            warn!(
                "Failed to sync review_forms after return_to_node, form_id={}: {}",
                form_id, error
            );
        }
    }

    // 4. 记录工作流历史
    let op_id = if claims.user_id.is_empty() {
        request
            .operator_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("system")
    } else {
        &claims.user_id
    };
    let op_name = request
        .operator_name
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(op_id);
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
        .bind(("task_id", id.clone()))
        .bind(("from_node", current_node.clone()))
        .bind(("operator_id", op_id.to_string()))
        .bind(("operator_name", op_name.to_string()))
        .bind(("comment", Some(request.reason.clone())))
        .await;

    let from_name = get_node_display_name(&current_node);
    let to_name = get_node_display_name(&request.target_node);

    (
        StatusCode::OK,
        Json(ActionResponse {
            success: true,
            message: Some(format!("已从「{}」驳回到「{}」", from_name, to_name)),
            error_message: None,
        }),
    )
}

/// 工作流历史响应
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowHistoryResponse {
    pub success: bool,
    pub current_node: String,
    pub current_node_name: String,
    pub history: Vec<WorkflowStep>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// GET /api/review/tasks/:id/workflow - 获取工作流历史
async fn get_workflow_history(Path(id): Path<String>) -> impl IntoResponse {
    info!("Getting workflow history for task {}", id);

    // 1. 获取当前任务的节点信息
    #[derive(Debug, Deserialize, SurrealValue)]
    struct CurrentNodeRow {
        current_node: Option<String>,
    }

    let get_sql = "SELECT current_node FROM review_tasks WHERE record::id(id) = $id AND (deleted IS NONE OR deleted = false) LIMIT 1";
    let current_node = match project_primary_db()
        .query(get_sql)
        .bind(("id", id.clone()))
        .await
    {
        Ok(mut resp) => {
            let rows: Vec<CurrentNodeRow> = resp.take(0).unwrap_or_default();
            match rows.into_iter().next() {
                Some(row) => row.current_node.unwrap_or_else(|| "sj".to_string()),
                None => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(WorkflowHistoryResponse {
                            success: false,
                            current_node: String::new(),
                            current_node_name: String::new(),
                            history: vec![],
                            error_message: Some(format!("任务不存在: {}", id)),
                        }),
                    );
                }
            }
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(WorkflowHistoryResponse {
                    success: false,
                    current_node: String::new(),
                    current_node_name: String::new(),
                    history: vec![],
                    error_message: Some(format!("查询任务失败: {}", e)),
                }),
            );
        }
    };

    // 2. 查询工作流历史
    #[derive(Debug, Deserialize, SurrealValue)]
    struct WorkflowRow {
        task_id: Option<String>,
        node: Option<String>,
        action: Option<String>,
        operator_id: Option<String>,
        operator_name: Option<String>,
        comment: Option<String>,
        timestamp: Option<surrealdb::types::Datetime>,
    }

    let history_sql = r#"
        SELECT * FROM review_workflow_history
        WHERE task_id = $task_id
        ORDER BY timestamp ASC
    "#;

    let history = match project_primary_db()
        .query(history_sql)
        .bind(("task_id", id.clone()))
        .await
    {
        Ok(mut resp) => {
            let rows: Vec<WorkflowRow> = resp.take(0).unwrap_or_default();
            rows.into_iter()
                .map(|r| WorkflowStep {
                    node: r.node.unwrap_or_default(),
                    action: r.action.unwrap_or_default(),
                    operator_id: r.operator_id.unwrap_or_default(),
                    operator_name: r.operator_name.unwrap_or_default(),
                    comment: r.comment,
                    timestamp: r
                        .timestamp
                        .map(|dt| dt.timestamp_millis())
                        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis()),
                })
                .collect()
        }
        Err(e) => {
            warn!("Failed to get workflow history: {}", e);
            vec![]
        }
    };

    let current_node_name = get_node_display_name(&current_node).to_string();

    (
        StatusCode::OK,
        Json(WorkflowHistoryResponse {
            success: true,
            current_node: current_node.clone(),
            current_node_name,
            history,
            error_message: None,
        }),
    )
}

// ============================================================================
// Handlers - 附件管理
// ============================================================================

/// 附件上传响应
#[derive(Debug, Serialize)]
pub struct AttachmentUploadResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<ReviewAttachment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// POST /api/review/attachments - 上传附件
async fn upload_attachment(mut multipart: Multipart) -> impl IntoResponse {
    info!("Uploading attachment");

    fn normalize_attachment_file_type(raw: Option<&str>) -> String {
        let normalized = raw
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_else(|| "file".to_string());

        match normalized.as_str() {
            "markup" | "file" => normalized,
            _ => "file".to_string(),
        }
    }

    let mut task_id: Option<String> = None;
    let mut form_id: Option<String> = None;
    let mut model_refnos: Option<Vec<String>> = None;
    let mut file_type: Option<String> = None;
    let mut description: Option<String> = None;
    let mut file_name: Option<String> = None;
    let mut file_data: Option<Vec<u8>> = None;
    let mut mime_type: Option<String> = None;

    // 解析 multipart 表单
    while let Ok(Some(field)) = multipart.next_field().await {
        let name: String = field.name().unwrap_or("").to_string();
        let field_name = name.trim().to_ascii_lowercase();

        match field_name.as_str() {
            "taskid" | "task_id" => {
                if let Ok(text) = field.text().await {
                    task_id = Some(text);
                }
            }
            "formid" | "form_id" => {
                if let Ok(text) = field.text().await {
                    form_id = Some(text);
                }
            }
            "modelrefnos" | "model_refnos" => {
                if let Ok(text) = field.text().await {
                    // 支持 JSON 数组或逗号分隔字符串
                    if let Ok(v) = serde_json::from_str::<Vec<String>>(&text) {
                        model_refnos = Some(v);
                    } else {
                        let items = text
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>();
                        model_refnos = Some(items);
                    }
                }
            }
            "type" | "filetype" | "file_type" => {
                if let Ok(text) = field.text().await {
                    file_type = Some(text);
                }
            }
            "description" => {
                if let Ok(text) = field.text().await {
                    description = Some(text);
                }
            }
            "file" => {
                file_name = field.file_name().map(|s: &str| s.to_string());
                mime_type = field.content_type().map(|s: &str| s.to_string());
                if let Ok(bytes) = field.bytes().await {
                    file_data = Some(bytes.to_vec());
                }
            }
            _ => {}
        }
    }

    // 验证必要字段
    let file_name = match file_name {
        Some(name) => name,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(AttachmentUploadResponse {
                    success: false,
                    attachment: None,
                    error_message: Some("缺少文件".to_string()),
                }),
            );
        }
    };

    let file_data = match file_data {
        Some(data) => data,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(AttachmentUploadResponse {
                    success: false,
                    attachment: None,
                    error_message: Some("文件数据为空".to_string()),
                }),
            );
        }
    };

    // 生成附件 ID 和保存路径
    let attachment_id = format!("att-{}", uuid::Uuid::new_v4());
    let file_ext = std::path::Path::new(&file_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin");
    let stored_name = format!("{}.{}", attachment_id, file_ext);

    // 确保上传目录存在
    let upload_dir = "assets/review_attachments";
    if let Err(e) = std::fs::create_dir_all(upload_dir) {
        warn!("Failed to create upload directory: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(AttachmentUploadResponse {
                success: false,
                attachment: None,
                error_message: Some(format!("创建上传目录失败: {}", e)),
            }),
        );
    }

    // 保存文件
    let file_path = format!("{}/{}", upload_dir, stored_name);
    if let Err(e) = std::fs::write(&file_path, &file_data) {
        warn!("Failed to save file: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(AttachmentUploadResponse {
                success: false,
                attachment: None,
                error_message: Some(format!("保存文件失败: {}", e)),
            }),
        );
    }

    let file_size = file_data.len() as i64;
    let url = format!("/files/review_attachments/{}", stored_name);

    // 写入 workflow 附件表（用于 /api/review/workflow/sync 汇总）
    // form_id：优先请求传入；否则尝试从 task_id 反查 review_tasks.form_id
    let mut resolved_form_id = form_id
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let mut resolved_model_refnos = model_refnos.unwrap_or_default();

    if resolved_form_id.is_none() {
        if let Some(tid) = task_id.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            #[derive(Debug, Deserialize, SurrealValue)]
            struct TaskLookupRow {
                form_id: Option<String>,
                components: Option<Vec<ReviewComponent>>,
            }

            let sql = "SELECT form_id, components FROM review_tasks WHERE record::id(id) = $id AND (deleted IS NONE OR deleted = false) LIMIT 1";
            if let Ok(mut resp) = project_primary_db()
                .query(sql)
                .bind(("id", tid.to_string()))
                .await
            {
                let rows: Vec<TaskLookupRow> = resp.take(0).unwrap_or_default();
                if let Some(row) = rows.into_iter().next() {
                    resolved_form_id = row
                        .form_id
                        .as_ref()
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string());

                    if resolved_model_refnos.is_empty() {
                        let mut set = HashSet::<String>::new();
                        if let Some(comps) = row.components {
                            for c in comps {
                                let refno = c.ref_no.trim();
                                if !refno.is_empty() {
                                    set.insert(refno.to_string());
                                }
                            }
                        }
                        resolved_model_refnos = set.into_iter().collect();
                    }
                }
            }
        }
    }

    let resolved_form_id = match resolved_form_id {
        Some(v) => v,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(AttachmentUploadResponse {
                    success: false,
                    attachment: None,
                    error_message: Some("缺少 formId（且无法由 taskId 反查）".to_string()),
                }),
            );
        }
    };

    let resolved_file_type = normalize_attachment_file_type(file_type.as_deref());

    let resolved_description = description
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| file_name.clone());

    let file_ext_with_dot = format!(".{}", file_ext);

    let insert_sql = r#"
        CREATE review_attachment CONTENT {
            form_id: $form_id,
            model_refnos: $model_refnos,
            file_id: $file_id,
            file_type: $file_type,
            download_url: $download_url,
            description: $description,
            file_ext: $file_ext,
            created_at: time::now()
        }
    "#;

    if let Err(e) = project_primary_db()
        .query(insert_sql)
        .bind(("form_id", resolved_form_id))
        .bind(("model_refnos", resolved_model_refnos))
        .bind(("file_id", attachment_id.clone()))
        .bind(("file_type", resolved_file_type))
        .bind(("download_url", url.clone()))
        .bind(("description", resolved_description))
        .bind(("file_ext", file_ext_with_dot))
        .await
    {
        warn!("Failed to insert review_attachment: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(AttachmentUploadResponse {
                success: false,
                attachment: None,
                error_message: Some(format!("附件入库失败: {}", e)),
            }),
        );
    }

    // 创建附件记录
    let attachment = ReviewAttachment {
        id: attachment_id.clone(),
        name: file_name,
        url,
        size: Some(file_size),
        mime_type,
    };

    info!(
        "Attachment uploaded: id={}, task_id={:?}",
        attachment_id, task_id
    );

    (
        StatusCode::OK,
        Json(AttachmentUploadResponse {
            success: true,
            attachment: Some(attachment),
            error_message: None,
        }),
    )
}

/// DELETE /api/review/attachments/:attachment_id - 删除附件
async fn delete_attachment(Path(attachment_id): Path<String>) -> impl IntoResponse {
    info!("Deleting attachment: {}", attachment_id);

    // 尝试删除文件（支持多种扩展名）
    let upload_dir = "assets/review_attachments";
    let extensions = ["png", "jpg", "jpeg", "gif", "pdf", "bin"];
    let mut deleted = false;

    // 优先使用 DB 中记录的 file_ext
    let mut db_ext: Option<String> = None;
    if let Ok(mut resp) = project_primary_db()
        .query("SELECT file_ext FROM review_attachment WHERE file_id = $file_id LIMIT 1")
        .bind(("file_id", attachment_id.clone()))
        .await
    {
        #[derive(Debug, Deserialize, SurrealValue)]
        struct ExtRow {
            file_ext: Option<String>,
        }
        let rows: Vec<ExtRow> = resp.take(0).unwrap_or_default();
        db_ext = rows.into_iter().next().and_then(|r| r.file_ext);
    }

    if let Some(ext) = db_ext.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        let normalized = ext.trim_start_matches('.');
        let file_path = format!("{}/{}.{}", upload_dir, attachment_id, normalized);
        if std::path::Path::new(&file_path).exists() {
            if let Err(e) = std::fs::remove_file(&file_path) {
                warn!("Failed to delete file {}: {}", file_path, e);
            } else {
                deleted = true;
            }
        }
    }

    for ext in &extensions {
        if deleted {
            break;
        }
        let file_path = format!("{}/{}.{}", upload_dir, attachment_id, ext);
        if std::path::Path::new(&file_path).exists() {
            if let Err(e) = std::fs::remove_file(&file_path) {
                warn!("Failed to delete file {}: {}", file_path, e);
            } else {
                deleted = true;
                break;
            }
        }
    }

    let _ = project_primary_db()
        .query(
            r#"
            LET $ids = SELECT VALUE id FROM review_attachment WHERE file_id = $file_id;
            DELETE $ids;
            "#,
        )
        .bind(("file_id", attachment_id.clone()))
        .await;

    if deleted {
        (
            StatusCode::OK,
            Json(ActionResponse {
                success: true,
                message: Some("附件已删除".to_string()),
                error_message: None,
            }),
        )
    } else {
        // 文件可能已被删除，仍返回成功
        (
            StatusCode::OK,
            Json(ActionResponse {
                success: true,
                message: Some("附件记录已清除".to_string()),
                error_message: None,
            }),
        )
    }
}

// ============================================================================
// Handlers - 同步接口
// ============================================================================

/// 导出请求
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub task_ids: Option<Vec<String>>,
    pub include_attachments: Option<bool>,
    pub include_comments: Option<bool>,
    pub include_records: Option<bool>,
}

/// 导出响应
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResponse {
    pub success: bool,
    pub tasks: Vec<ReviewTask>,
    pub comments: Option<Vec<AnnotationComment>>,
    pub records: Option<Vec<ConfirmedRecordWithMeta>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// POST /api/review/sync/export - 导出校审数据
async fn export_review_data(Json(request): Json<ExportRequest>) -> impl IntoResponse {
    info!("Exporting review data");

    let (sql, use_ids_param) = if let Some(ref ids) = request.task_ids {
        if ids.is_empty() {
            (
                "SELECT * FROM review_tasks WHERE (deleted IS NONE OR deleted = false) ORDER BY created_at DESC LIMIT 100".to_string(),
                false,
            )
        } else {
            (
                "SELECT * FROM review_tasks WHERE (deleted IS NONE OR deleted = false) AND record::id(id) IN $task_ids".to_string(),
                true,
            )
        }
    } else {
        (
            "SELECT * FROM review_tasks WHERE (deleted IS NONE OR deleted = false) ORDER BY created_at DESC LIMIT 100".to_string(),
            false,
        )
    };

    let mut q = project_primary_db().query(&sql);
    if use_ids_param {
        q = q.bind(("task_ids", request.task_ids.clone().unwrap_or_default()));
    }

    let tasks: Vec<ReviewTask> = match q.await {
        Ok(mut resp) => {
            let rows: Vec<TaskRow> = resp.take(0).unwrap_or_default();
            rows.into_iter().map(|r| r.to_review_task()).collect()
        }
        Err(e) => {
            warn!("Failed to export tasks: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ExportResponse {
                    success: false,
                    tasks: vec![],
                    comments: None,
                    records: None,
                    error_message: Some(format!("导出失败: {}", e)),
                }),
            );
        }
    };

    let task_ids: Vec<String> = tasks.iter().map(|t| t.id.clone()).collect();

    // 可选导出评论（当前评论不和 task_id 直接关联，这里按“全量导出”处理）
    let include_comments = request.include_comments.unwrap_or(false);
    let comments: Option<Vec<AnnotationComment>> = if include_comments {
        #[derive(Debug, Deserialize, SurrealValue)]
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

        let sql = "SELECT * FROM review_comments ORDER BY created_at ASC LIMIT 10000";
        match project_primary_db().query(sql).await {
            Ok(mut resp) => {
                let rows: Vec<CommentRow> = resp.take(0).unwrap_or_default();
                Some(
                    rows.into_iter()
                        .map(|r| AnnotationComment {
                            id: format!("{:?}", r.id.key),
                            annotation_id: r.annotation_id.unwrap_or_default(),
                            annotation_type: r.annotation_type.unwrap_or_default(),
                            author_id: r.author_id.unwrap_or_default(),
                            author_name: r.author_name.unwrap_or_default(),
                            author_role: r.author_role.unwrap_or_default(),
                            content: r.content.unwrap_or_default(),
                            reply_to_id: r.reply_to_id,
                            created_at: parse_datetime_value(&r.created_at),
                        })
                        .collect(),
                )
            }
            Err(e) => {
                warn!("Failed to export comments: {}", e);
                Some(vec![])
            }
        }
    } else {
        None
    };

    // 可选导出确认记录（按 task_id 过滤）
    let include_records = request.include_records.unwrap_or(false);
    let records: Option<Vec<ConfirmedRecordWithMeta>> = if include_records {
        #[derive(Debug, Deserialize, SurrealValue)]
        struct RecordRow {
            id: surrealdb::types::RecordId,
            task_id: Option<String>,
            r#type: Option<String>,
            annotations: Option<Vec<serde_json::Value>>,
            cloud_annotations: Option<Vec<serde_json::Value>>,
            rect_annotations: Option<Vec<serde_json::Value>>,
            obb_annotations: Option<Vec<serde_json::Value>>,
            measurements: Option<Vec<serde_json::Value>>,
            note: Option<String>,
            confirmed_at: Option<surrealdb::types::Datetime>,
        }

        if task_ids.is_empty() {
            Some(vec![])
        } else {
            let sql = "SELECT * FROM review_records WHERE task_id IN $task_ids ORDER BY confirmed_at ASC LIMIT 10000";
            match project_primary_db()
                .query(sql)
                .bind(("task_ids", task_ids))
                .await
            {
                Ok(mut resp) => {
                    let rows: Vec<RecordRow> = resp.take(0).unwrap_or_default();
                    Some(
                        rows.into_iter()
                            .map(|r| ConfirmedRecordWithMeta {
                                id: format!("{:?}", r.id.key),
                                task_id: r.task_id.unwrap_or_default(),
                                r#type: r.r#type.unwrap_or_else(|| "batch".to_string()),
                                annotations: r.annotations.unwrap_or_default(),
                                cloud_annotations: r.cloud_annotations.unwrap_or_default(),
                                rect_annotations: r.rect_annotations.unwrap_or_default(),
                                obb_annotations: r.obb_annotations.unwrap_or_default(),
                                measurements: r.measurements.unwrap_or_default(),
                                note: r.note.unwrap_or_default(),
                                confirmed_at: parse_datetime_value(&r.confirmed_at),
                            })
                            .collect(),
                    )
                }
                Err(e) => {
                    warn!("Failed to export records: {}", e);
                    Some(vec![])
                }
            }
        }
    } else {
        None
    };

    (
        StatusCode::OK,
        Json(ExportResponse {
            success: true,
            tasks,
            comments,
            records,
            error_message: None,
        }),
    )
}

/// 导入请求
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    pub tasks: Vec<ReviewTask>,
    pub overwrite: Option<bool>,
}

/// 导入响应
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResponse {
    pub success: bool,
    pub imported_count: i32,
    pub skipped_count: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// POST /api/review/sync/import - 导入校审数据
async fn import_review_data(Json(request): Json<ImportRequest>) -> impl IntoResponse {
    info!("Importing {} review tasks", request.tasks.len());

    let overwrite = request.overwrite.unwrap_or(false);
    let mut imported = 0;
    let mut skipped = 0;

    for task in request.tasks {
        // 检查任务是否已存在
        let check_sql = "SELECT id FROM review_tasks WHERE record::id(id) = $id";
        let exists = match project_primary_db()
            .query(check_sql)
            .bind(("id", task.id.clone()))
            .await
        {
            Ok(mut resp) => {
                let rows: Vec<serde_json::Value> = resp.take(0).unwrap_or_default();
                !rows.is_empty()
            }
            Err(_) => false,
        };

        if exists && !overwrite {
            skipped += 1;
            continue;
        }

        // 插入或更新任务
        let sql = if exists {
            r#"UPDATE review_tasks SET
                title = $title,
                description = $description,
                status = $status,
                priority = $priority,
                form_id = $form_id,
                current_node = $current_node,
                updated_at = time::now()
            WHERE record::id(id) = $id"#
        } else {
            r#"CREATE review_tasks SET
                id = $id,
                form_id = $form_id,
                title = $title,
                description = $description,
                model_name = $model_name,
                status = $status,
                priority = $priority,
                requester_id = $requester_id,
                requester_name = $requester_name,
                reviewer_id = $reviewer_id,
                reviewer_name = $reviewer_name,
                current_node = $current_node,
                created_at = time::now(),
                updated_at = time::now()"#
        };

        let result = project_primary_db()
            .query(sql)
            .bind(("id", task.id.clone()))
            .bind(("form_id", task.form_id.clone()))
            .bind(("title", task.title.clone()))
            .bind(("description", task.description.clone()))
            .bind(("model_name", task.model_name.clone()))
            .bind(("status", task.status.clone()))
            .bind(("priority", task.priority.clone()))
            .bind(("requester_id", task.requester_id.clone()))
            .bind(("requester_name", task.requester_name.clone()))
            .bind(("reviewer_id", task.reviewer_id.clone()))
            .bind(("reviewer_name", task.reviewer_name.clone()))
            .bind(("current_node", task.current_node.clone()))
            .await;

        match result {
            Ok(_) => {
                if !task.form_id.trim().is_empty() {
                    if let Err(error) = sync_review_form_with_task_status(
                        task.form_id.as_str(),
                        Some(task.model_name.as_str()),
                        Some(task.requester_id.as_str()),
                        "import_backfill",
                        task.status.as_str(),
                    )
                    .await
                    {
                        warn!(
                            "Failed to sync review_forms during import, form_id={}: {}",
                            task.form_id, error
                        );
                    }
                }

                // 同步写入 review_form_model（用于 workflow/sync 汇总）
                if !task.form_id.trim().is_empty() {
                    for comp in &task.components {
                        if comp.ref_no.trim().is_empty() {
                            continue;
                        }
                        let _ = project_primary_db()
                            .query(
                                r#"
                                CREATE ONLY review_form_model SET
                                    form_id = $form_id,
                                    model_refno = $model_refno,
                                    created_at = time::now()
                                "#,
                            )
                            .bind(("form_id", task.form_id.clone()))
                            .bind(("model_refno", comp.ref_no.clone()))
                            .await;
                    }
                }
                imported += 1;
            }
            Err(e) => {
                warn!("Failed to import task {}: {}", task.id, e);
                skipped += 1;
            }
        }
    }

    info!(
        "Import complete: {} imported, {} skipped",
        imported, skipped
    );

    (
        StatusCode::OK,
        Json(ImportResponse {
            success: true,
            imported_count: imported,
            skipped_count: skipped,
            error_message: None,
        }),
    )
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{self, Body},
        extract::Extension,
        http::{Request, StatusCode},
    };
    use serde_json::json;
    use tower::ServiceExt;

    use crate::web_api::jwt_auth::TokenClaims;

    #[test]
    fn test_get_next_node() {
        assert_eq!(get_next_node("sj"), Some("jd"));
        assert_eq!(get_next_node("jd"), Some("sh"));
        assert_eq!(get_next_node("sh"), Some("pz"));
        assert_eq!(get_next_node("pz"), None);
        assert_eq!(get_next_node("unknown"), None);
        assert_eq!(get_next_node(""), None);
    }

    #[test]
    fn test_can_return_to() {
        assert!(can_return_to("jd", "sj"));
        assert!(can_return_to("sh", "sj"));
        assert!(can_return_to("sh", "jd"));
        assert!(can_return_to("pz", "sj"));
        assert!(can_return_to("pz", "jd"));
        assert!(can_return_to("pz", "sh"));

        assert!(!can_return_to("sj", "jd"));
        assert!(!can_return_to("sj", "sj"));
        assert!(!can_return_to("jd", "sh"));
        assert!(!can_return_to("jd", "jd"));
        assert!(!can_return_to("unknown", "sj"));
        assert!(!can_return_to("sj", "unknown"));
    }

    #[test]
    fn test_get_node_display_name() {
        assert_eq!(get_node_display_name("sj"), "编制");
        assert_eq!(get_node_display_name("jd"), "校对");
        assert_eq!(get_node_display_name("sh"), "审核");
        assert_eq!(get_node_display_name("pz"), "批准");
        assert_eq!(get_node_display_name("invalid"), "未知");
    }

    #[test]
    fn test_workflow_nodes_order() {
        assert_eq!(WORKFLOW_NODES, ["sj", "jd", "sh", "pz"]);
        assert_eq!(WORKFLOW_NODES.len(), 4);
    }

    #[test]
    fn test_default_values() {
        assert_eq!(default_priority(), "medium");
        assert_eq!(default_status(), "draft");
        assert_eq!(default_current_node(), "sj");
    }

    #[test]
    fn test_task_row_to_review_task_compat() {
        let row = TaskRow {
            id: surrealdb::types::RecordId {
                table: "review_tasks".to_string().into(),
                key: surrealdb::types::RecordIdKey::String("task-123".to_string()),
            },
            form_id: Some("form-456".to_string()),
            title: Some("Test Task".to_string()),
            description: None,
            model_name: Some("Model A".to_string()),
            status: Some("draft".to_string()),
            priority: Some("high".to_string()),
            requester_id: Some("user-1".to_string()),
            requester_name: Some("张三".to_string()),
            checker_id: None,
            checker_name: None,
            approver_id: Some("user-3".to_string()),
            approver_name: Some("王五".to_string()),
            reviewer_id: Some("user-2".to_string()),
            reviewer_name: Some("李四".to_string()),
            components: None,
            attachments: None,
            review_comment: None,
            created_at: None,
            updated_at: None,
            due_date: None,
            current_node: Some("jd".to_string()),
            workflow_history: None,
            return_reason: None,
        };

        let task = row.to_review_task();

        assert_eq!(task.id, "task-123");
        assert_eq!(task.form_id, "form-456");
        assert_eq!(task.title, "Test Task");
        assert_eq!(task.model_name, "Model A");
        assert_eq!(task.status, "draft");
        assert_eq!(task.priority, "high");
        assert_eq!(task.current_node, "jd");
        // checker_id 为空时应回退到 reviewer_id
        assert_eq!(task.checker_id, "user-2");
        assert_eq!(task.checker_name, "李四");
        assert_eq!(task.approver_id, "user-3");
        assert_eq!(task.approver_name, "王五");
        // reviewer 字段兼容
        assert_eq!(task.reviewer_id, "user-2");
        assert_eq!(task.reviewer_name, "李四");
    }

    #[test]
    fn test_task_row_to_review_task_checker_preferred() {
        let row = TaskRow {
            id: surrealdb::types::RecordId {
                table: "review_tasks".to_string().into(),
                key: surrealdb::types::RecordIdKey::String("task-789".to_string()),
            },
            form_id: None,
            title: Some("Task with checker".to_string()),
            description: None,
            model_name: None,
            status: None,
            priority: None,
            requester_id: None,
            requester_name: None,
            checker_id: Some("checker-1".to_string()),
            checker_name: Some("校核员".to_string()),
            approver_id: Some("approver-1".to_string()),
            approver_name: Some("审核员".to_string()),
            reviewer_id: Some("old-reviewer".to_string()),
            reviewer_name: Some("旧审核人".to_string()),
            components: None,
            attachments: None,
            review_comment: None,
            created_at: None,
            updated_at: None,
            due_date: None,
            current_node: None,
            workflow_history: None,
            return_reason: None,
        };

        let task = row.to_review_task();

        // checker_id 有值时优先使用
        assert_eq!(task.checker_id, "checker-1");
        assert_eq!(task.checker_name, "校核员");
        assert_eq!(task.approver_id, "approver-1");
        // reviewer 字段仍保留原值
        assert_eq!(task.reviewer_id, "old-reviewer");
        assert_eq!(task.reviewer_name, "旧审核人");
        // 默认值回退
        assert_eq!(task.status, "draft");
        assert_eq!(task.priority, "medium");
        assert_eq!(task.current_node, "sj");
    }

    #[test]
    fn test_resolve_create_task_names_prefers_explicit_names_and_claim_user_name() {
        let claims = TokenClaims {
            project_id: "project-123".to_string(),
            user_id: "designer_001".to_string(),
            user_name: "张设计".to_string(),
            form_id: "FORM-123".to_string(),
            role: Some("sj".to_string()),
            exp: 4_102_444_800,
            iat: 1_704_067_200,
        };
        let request = CreateTaskRequest {
            title: "Task".to_string(),
            description: "".to_string(),
            model_name: "Model".to_string(),
            checker_id: Some("checker-001".to_string()),
            checker_name: Some("李校核".to_string()),
            approver_id: Some("approver-001".to_string()),
            approver_name: Some("王审核".to_string()),
            reviewer_id: "reviewer-legacy".to_string(),
            form_id: None,
            priority: "medium".to_string(),
            components: vec![],
            due_date: None,
            attachments: None,
        };

        let names = resolve_create_task_names(&claims, &request, "checker-001", "approver-001");

        assert_eq!(names.requester_name, "张设计");
        assert_eq!(names.checker_name, "李校核");
        assert_eq!(names.approver_name, "王审核");
        assert_eq!(names.reviewer_name, "李校核");
    }

    #[test]
    fn test_resolve_create_task_names_falls_back_to_ids_when_names_missing() {
        let claims = TokenClaims {
            project_id: "project-123".to_string(),
            user_id: "designer_001".to_string(),
            user_name: "".to_string(),
            form_id: "FORM-123".to_string(),
            role: Some("sj".to_string()),
            exp: 4_102_444_800,
            iat: 1_704_067_200,
        };
        let request = CreateTaskRequest {
            title: "Task".to_string(),
            description: "".to_string(),
            model_name: "Model".to_string(),
            checker_id: Some("checker-001".to_string()),
            checker_name: Some("".to_string()),
            approver_id: Some("approver-001".to_string()),
            approver_name: None,
            reviewer_id: "reviewer-legacy".to_string(),
            form_id: None,
            priority: "medium".to_string(),
            components: vec![],
            due_date: None,
            attachments: None,
        };

        let names = resolve_create_task_names(&claims, &request, "checker-001", "approver-001");

        assert_eq!(names.requester_name, "designer_001");
        assert_eq!(names.checker_name, "checker-001");
        assert_eq!(names.approver_name, "approver-001");
        assert_eq!(names.reviewer_name, "checker-001");
    }

    #[test]
    fn test_review_task_from_value_falls_back_for_legacy_row_shapes() {
        let raw = json!({
            "id": { "tb": "review_tasks", "id": "task-legacy-1" },
            "form_id": "FORM-LEGACY-001",
            "title": "Legacy Task",
            "description": "legacy payload",
            "model_name": "Legacy Model",
            "status": "submitted",
            "priority": "medium",
            "requester_id": "designer_001",
            "requester_name": "Designer",
            "checker_id": "user-002",
            "checker_name": "Reviewer",
            "approver_id": "manager_001",
            "approver_name": "Manager",
            "reviewer_id": "user-002",
            "reviewer_name": "Reviewer",
            "components": [
                {
                    "id": "comp-1",
                    "name": "Pipe-1",
                    "refNo": "17496_248588",
                    "type": "pipe"
                }
            ],
            "attachments": {
                "legacy": true
            },
            "created_at": "2026-03-18T15:00:00Z",
            "updated_at": "2026-03-18T15:01:00Z",
            "current_node": "jd",
            "workflow_history": "legacy-string-payload"
        });

        let task = review_task_from_value(raw).expect("legacy row should still normalize");

        assert_eq!(task.id, "task-legacy-1");
        assert_eq!(task.form_id, "FORM-LEGACY-001");
        assert_eq!(task.requester_id, "designer_001");
        assert_eq!(task.checker_id, "user-002");
        assert_eq!(task.current_node, "jd");
        assert_eq!(task.workflow_history.len(), 0);
        assert!(task.created_at > 0);
        assert!(task.updated_at >= task.created_at);
    }

    #[test]
    fn test_review_tasks_from_values_keeps_pages_readable_when_one_row_is_legacy() {
        let values = vec![
            json!({
                "id": { "tb": "review_tasks", "id": "task-good-1" },
                "title": "Good Task",
                "description": "ok",
                "model_name": "Model A",
                "status": "submitted",
                "priority": "high",
                "requester_id": "designer_001",
                "requester_name": "Designer",
                "checker_id": "user-002",
                "checker_name": "Reviewer",
                "approver_id": "manager_001",
                "approver_name": "Manager",
                "created_at": 1773845013517i64,
                "updated_at": 1773845013518i64,
                "current_node": "jd",
                "components": [],
                "workflow_history": []
            }),
            json!({
                "id": { "tb": "review_tasks", "id": "task-legacy-2" },
                "title": "Legacy Task",
                "description": "legacy",
                "model_name": "Model B",
                "status": "draft",
                "priority": "medium",
                "requester_id": "designer_001",
                "requester_name": "Designer",
                "checker_id": "user-002",
                "checker_name": "Reviewer",
                "approver_id": "manager_001",
                "approver_name": "Manager",
                "created_at": "2026-03-18T15:00:00Z",
                "updated_at": "2026-03-18T15:01:00Z",
                "current_node": "sj",
                "components": [],
                "workflow_history": "legacy-string-payload"
            }),
        ];

        let (tasks, parse_failures) = review_tasks_from_values(values);

        assert_eq!(parse_failures, 0);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].id, "task-good-1");
        assert_eq!(tasks[1].id, "task-legacy-2");
    }

    #[test]
    fn test_build_mock_review_users_matches_frontend_contract() {
        let users = build_mock_review_users();
        let ids = users
            .iter()
            .map(|user| user.id.as_str())
            .collect::<Vec<_>>();

        assert!(ids.contains(&"designer_001"));
        assert!(ids.contains(&"proofreader_001"));
        assert!(ids.contains(&"reviewer_001"));
        assert!(ids.contains(&"manager_001"));
        assert!(ids.contains(&"PZ"));
        assert!(ids.contains(&"admin_001"));
    }

    #[test]
    fn test_build_mock_reviewers_only_returns_review_capable_roles() {
        let reviewers = build_mock_review_users()
            .into_iter()
            .filter(|user| {
                matches!(
                    user.role.as_str(),
                    "proofreader" | "reviewer" | "manager" | "admin"
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(reviewers.len(), 5);
        assert!(reviewers.iter().all(|user| user.id != "designer_001"));
        assert!(reviewers.iter().any(|user| user.id == "proofreader_001"));
        assert!(reviewers.iter().any(|user| user.id == "reviewer_001"));
        assert!(reviewers.iter().any(|user| user.id == "manager_001"));
        assert!(reviewers.iter().any(|user| user.id == "PZ"));
        assert!(reviewers.iter().any(|user| user.id == "admin_001"));
    }

    #[test]
    fn test_default_mock_user_matches_frontend_designer_contract() {
        let user = default_mock_user();

        assert_eq!(user.id, "designer_001");
        assert_eq!(user.username, "designer");
        assert_eq!(user.role, "designer");
    }

    #[tokio::test]
    async fn test_get_current_user_returns_frontend_designer_contract_without_claims() {
        let app = Router::new().route("/api/users/me", get(get_current_user));

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/users/me")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: UserResponse = serde_json::from_slice(&body).unwrap();
        let user = payload.user.expect("expected current user payload");

        assert_eq!(user.id, "designer_001");
        assert_eq!(user.username, "designer");
        assert_eq!(user.role, "designer");
    }

    #[tokio::test]
    async fn test_get_reviewers_returns_only_review_capable_users() {
        let app = create_review_api_routes();

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/users/reviewers")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: UserListResponse = serde_json::from_slice(&body).unwrap();
        let ids = payload
            .users
            .iter()
            .map(|user| user.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(payload.users.len(), 5);
        assert!(!ids.contains(&"designer_001"));
        assert!(ids.contains(&"proofreader_001"));
        assert!(ids.contains(&"reviewer_001"));
        assert!(ids.contains(&"manager_001"));
        assert!(ids.contains(&"PZ"));
        assert!(ids.contains(&"admin_001"));
    }

    #[tokio::test]
    async fn test_get_current_user_maps_pz_claim_to_explicit_pz_user() {
        let app = Router::new().route("/api/users/me", get(get_current_user));
        let claims = TokenClaims {
            project_id: "project-123".to_string(),
            user_id: "PZ".to_string(),
            user_name: "PZ批准人".to_string(),
            form_id: "FORM-123".to_string(),
            role: Some("pz".to_string()),
            exp: 4_102_444_800,
            iat: 1_704_067_200,
        };

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/users/me")
                    .extension(claims)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: UserResponse = serde_json::from_slice(&body).unwrap();
        let user = payload.user.expect("expected current user payload");

        assert_eq!(user.id, "PZ");
        assert_eq!(user.username, "PZ");
        assert_eq!(user.name, "PZ批准人");
        assert_eq!(user.role, "manager");
        assert_eq!(user.email, "pz@company.com");
    }

    #[tokio::test]
    async fn test_get_current_user_uses_token_claims_when_present() {
        let app = Router::new().route("/api/users/me", get(get_current_user));
        let claims = TokenClaims {
            project_id: "project-123".to_string(),
            user_id: "reviewer_001".to_string(),
            user_name: "李审核员".to_string(),
            form_id: "FORM-123".to_string(),
            role: Some("sh".to_string()),
            exp: 4_102_444_800,
            iat: 1_704_067_200,
        };

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/users/me")
                    .extension(claims)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: UserResponse = serde_json::from_slice(&body).unwrap();
        let user = payload.user.expect("expected current user payload");

        assert_eq!(user.id, "reviewer_001");
        assert_eq!(user.username, "reviewer");
        assert_eq!(user.name, "李审核员");
        assert_eq!(user.role, "reviewer");
        assert_eq!(user.email, "reviewer@company.com");
    }
}
