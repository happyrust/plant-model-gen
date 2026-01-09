//! Review API - 校审管理 API
//!
//! 实现提资单、确认记录、评论、附件等完整的 CRUD 操作

use axum::{
    Router,
    extract::{Json, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, patch, post},
};
use serde::{Deserialize, Serialize};
use surrealdb::types::{self as surrealdb_types, SurrealValue};
use tracing::{info, warn};

use aios_core::SUL_DB;

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
    pub reviewer_id: String,
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
    pub reviewer_id: String,
    pub reviewer_name: String,
    #[serde(default)]
    pub components: Vec<ReviewComponent>,
    pub attachments: Option<Vec<ReviewAttachment>>,
    pub review_comment: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub due_date: Option<i64>,
}

fn default_status() -> String {
    "draft".to_string()
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
    title: Option<String>,
    description: Option<String>,
    model_name: Option<String>,
    status: Option<String>,
    priority: Option<String>,
    requester_id: Option<String>,
    requester_name: Option<String>,
    reviewer_id: Option<String>,
    reviewer_name: Option<String>,
    components: Option<Vec<ReviewComponent>>,
    attachments: Option<Vec<ReviewAttachment>>,
    review_comment: Option<String>,
    created_at: Option<surrealdb::types::Datetime>,
    updated_at: Option<surrealdb::types::Datetime>,
    due_date: Option<surrealdb::types::Datetime>,
}

impl TaskRow {
    fn to_review_task(self) -> ReviewTask {
        // 从 RecordIdKey 提取实际的字符串 ID
        let id = match &self.id.key {
            surrealdb::types::RecordIdKey::String(s) => s.clone(),
            other => format!("{:?}", other),
        };
        ReviewTask {
            id,
            title: self.title.unwrap_or_default(),
            description: self.description.unwrap_or_default(),
            model_name: self.model_name.unwrap_or_default(),
            status: self.status.unwrap_or_else(default_status),
            priority: self.priority.unwrap_or_else(default_priority),
            requester_id: self.requester_id.unwrap_or_default(),
            requester_name: self.requester_name.unwrap_or_default(),
            reviewer_id: self.reviewer_id.unwrap_or_default(),
            reviewer_name: self.reviewer_name.unwrap_or_default(),
            components: self.components.unwrap_or_default(),
            attachments: self.attachments,
            review_comment: self.review_comment,
            created_at: datetime_to_millis(&self.created_at),
            updated_at: datetime_to_millis(&self.updated_at),
            due_date: self.due_date.map(|dt| datetime_to_millis(&Some(dt))),
        }
    }
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

// ============================================================================
// Routes
// ============================================================================

pub fn create_review_api_routes() -> Router {
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
        // 确认记录 CRUD（修复路由冲突）
        .route("/api/review/records", post(create_record))
        .route("/api/review/records/by-task/{task_id}", get(get_records_by_task))
        .route("/api/review/records/item/{record_id}", delete(delete_record))
        .route("/api/review/records/clear-task/{task_id}", delete(clear_records_by_task))
        // 评论 CRUD（修复路由冲突）
        .route("/api/review/comments", post(create_comment))
        .route("/api/review/comments/by-annotation/{annotation_id}", get(get_comments_by_annotation))
        .route("/api/review/comments/item/{comment_id}", delete(delete_comment))
        // 用户 API
        .route("/api/users", get(list_users))
        .route("/api/users/me", get(get_current_user))
        .route("/api/users/reviewers", get(get_reviewers))
}

// ============================================================================
// Handlers - 提资单 CRUD
// ============================================================================

/// POST /api/review/tasks - 创建提资单
async fn create_task(
    Json(request): Json<CreateTaskRequest>,
) -> impl IntoResponse {
    info!("Creating review task: title={}", request.title);
    
    let task_id = format!("task-{}", uuid::Uuid::new_v4());
    let now = chrono::Utc::now().to_rfc3339();
    
    // 获取请求者信息（暂时使用默认值，后续从 JWT 获取）
    let requester_id = "system";
    let requester_name = "系统用户";
    
    // 获取审核人名称（后续从用户表查询）
    let reviewer_name = "审核人";
    
    let sql = r#"
        CREATE ONLY review_tasks SET
            id = $id,
            title = $title,
            description = $description,
            model_name = $model_name,
            status = 'draft',
            priority = $priority,
            requester_id = $requester_id,
            requester_name = $requester_name,
            reviewer_id = $reviewer_id,
            reviewer_name = $reviewer_name,
            components = $components,
            attachments = $attachments,
            due_date = $due_date,
            created_at = time::now(),
            updated_at = time::now()
    "#;
    
    let result = SUL_DB
        .query(sql)
        .bind(("id", task_id.clone()))
        .bind(("title", request.title.clone()))
        .bind(("description", request.description.clone()))
        .bind(("model_name", request.model_name.clone()))
        .bind(("priority", request.priority.clone()))
        .bind(("requester_id", requester_id))
        .bind(("requester_name", requester_name))
        .bind(("reviewer_id", request.reviewer_id.clone()))
        .bind(("reviewer_name", reviewer_name))
        .bind(("components", request.components.clone()))
        .bind(("attachments", request.attachments.clone()))
        .bind(("due_date", request.due_date.map(|d| chrono::DateTime::from_timestamp_millis(d).map(|dt| dt.to_rfc3339())).flatten()))
        .await;
    
    match result {
        Ok(_response) => {
            // CREATE 成功，无需解析响应（避免 datetime 反序列化问题）
            info!("Created task: {}", task_id);
            let task = ReviewTask {
                id: task_id,
                title: request.title,
                description: request.description,
                model_name: request.model_name,
                status: "draft".to_string(),
                priority: request.priority,
                requester_id: requester_id.to_string(),
                requester_name: requester_name.to_string(),
                reviewer_id: request.reviewer_id,
                reviewer_name: reviewer_name.to_string(),
                components: request.components,
                attachments: request.attachments,
                review_comment: None,
                created_at: chrono::Utc::now().timestamp_millis(),
                updated_at: chrono::Utc::now().timestamp_millis(),
                due_date: request.due_date,
            };
            (StatusCode::OK, Json(TaskResponse {
                success: true,
                task: Some(task),
                error_message: None,
            }))
        }
        Err(e) => {
            warn!("Failed to create task: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(TaskResponse {
                success: false,
                task: None,
                error_message: Some(format!("创建提资单失败: {}", e)),
            }))
        }
    }
}

/// GET /api/review/tasks - 获取任务列表
async fn list_tasks(
    Query(query): Query<TaskListQuery>,
) -> impl IntoResponse {
    info!("Listing review tasks");
    
    let mut conditions = vec![];
    let mut bindings: Vec<(&str, String)> = vec![];
    
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
    if let Some(ref reviewer_id) = query.reviewer_id {
        conditions.push("reviewer_id = $reviewer_id");
        bindings.push(("reviewer_id", reviewer_id.clone()));
    }
    
    let where_clause = if conditions.is_empty() {
        "".to_string()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };
    
    let limit = query.limit.unwrap_or(100);
    let offset = query.offset.unwrap_or(0);
    
    let sql = format!(
        "SELECT * FROM review_tasks {} ORDER BY created_at DESC LIMIT {} START {}",
        where_clause, limit, offset
    );
    
    let mut q = SUL_DB.query(&sql);
    for (name, value) in &bindings {
        q = q.bind((*name, value.clone()));
    }
    
    match q.await {
        Ok(mut response) => {
            let rows: Vec<TaskRow> = response.take(0).unwrap_or_default();
            let tasks: Vec<ReviewTask> = rows.into_iter().map(|r| r.to_review_task()).collect();
            let total = tasks.len() as i64;
            
            (StatusCode::OK, Json(TaskListResponse {
                success: true,
                tasks,
                total,
                error_message: None,
            }))
        }
        Err(e) => {
            warn!("Failed to list tasks: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(TaskListResponse {
                success: false,
                tasks: vec![],
                total: 0,
                error_message: Some(format!("获取任务列表失败: {}", e)),
            }))
        }
    }
}

/// GET /api/review/tasks/:id - 获取任务详情
async fn get_task(
    Path(id): Path<String>,
) -> impl IntoResponse {
    info!("Getting task: {}", id);
    
    // 使用 record::id(id) 提取 key 进行比较
    let sql = "SELECT * FROM review_tasks WHERE record::id(id) = $id LIMIT 1";
    
    match SUL_DB.query(sql).bind(("id", id.clone())).await {
        Ok(mut response) => {
            let rows: Vec<TaskRow> = response.take(0).unwrap_or_default();
            if let Some(row) = rows.into_iter().next() {
                (StatusCode::OK, Json(TaskResponse {
                    success: true,
                    task: Some(row.to_review_task()),
                    error_message: None,
                }))
            } else {
                (StatusCode::NOT_FOUND, Json(TaskResponse {
                    success: false,
                    task: None,
                    error_message: Some(format!("任务不存在: {}", id)),
                }))
            }
        }
        Err(e) => {
            warn!("Failed to get task: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(TaskResponse {
                success: false,
                task: None,
                error_message: Some(format!("获取任务失败: {}", e)),
            }))
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
    
    if request.title.is_some() { updates.push("title = $title"); }
    if request.description.is_some() { updates.push("description = $description"); }
    if request.priority.is_some() { updates.push("priority = $priority"); }
    if request.components.is_some() { updates.push("components = $components"); }
    if request.due_date.is_some() { updates.push("due_date = $due_date"); }
    if request.attachments.is_some() { updates.push("attachments = $attachments"); }
    
    let sql = format!(
        "UPDATE review_tasks SET {} WHERE record::id(id) = $id",
        updates.join(", ")
    );
    
    let mut q = SUL_DB.query(&sql).bind(("id", id.clone()));
    
    if let Some(ref title) = request.title { q = q.bind(("title", title.clone())); }
    if let Some(ref description) = request.description { q = q.bind(("description", description.clone())); }
    if let Some(ref priority) = request.priority { q = q.bind(("priority", priority.clone())); }
    if let Some(ref components) = request.components { q = q.bind(("components", components.clone())); }
    if let Some(due_date) = request.due_date { 
        let dt = chrono::DateTime::from_timestamp_millis(due_date).map(|d| d.to_rfc3339());
        q = q.bind(("due_date", dt)); 
    }
    if let Some(ref attachments) = request.attachments { q = q.bind(("attachments", attachments.clone())); }
    
    match q.await {
        Ok(_) => {
            // 返回更新后的任务
            let get_sql = "SELECT * FROM review_tasks WHERE record::id(id) = $id";
            if let Ok(mut resp) = SUL_DB.query(get_sql).bind(("id", id.clone())).await {
                let rows: Vec<TaskRow> = resp.take(0).unwrap_or_default();
                if let Some(row) = rows.into_iter().next() {
                    return (StatusCode::OK, Json(TaskResponse {
                        success: true,
                        task: Some(row.to_review_task()),
                        error_message: None,
                    }));
                }
            }
            (StatusCode::OK, Json(TaskResponse {
                success: true,
                task: None,
                error_message: None,
            }))
        }
        Err(e) => {
            warn!("Failed to update task: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(TaskResponse {
                success: false,
                task: None,
                error_message: Some(format!("更新任务失败: {}", e)),
            }))
        }
    }
}

/// DELETE /api/review/tasks/:id - 删除任务
async fn delete_task(
    Path(id): Path<String>,
) -> impl IntoResponse {
    info!("Deleting task: {}", id);
    
    let sql = "DELETE review_tasks WHERE record::id(id) = $id";
    
    match SUL_DB.query(sql).bind(("id", id.clone())).await {
        Ok(_) => {
            (StatusCode::OK, Json(ActionResponse {
                success: true,
                message: Some("任务已删除".to_string()),
                error_message: None,
            }))
        }
        Err(e) => {
            warn!("Failed to delete task: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ActionResponse {
                success: false,
                message: None,
                error_message: Some(format!("删除任务失败: {}", e)),
            }))
        }
    }
}

// ============================================================================
// Handlers - 审核操作
// ============================================================================

/// POST /api/review/tasks/:id/start-review - 开始审核
async fn start_review(
    Path(id): Path<String>,
) -> impl IntoResponse {
    update_task_status(id, "in_review".to_string(), None).await
}

/// POST /api/review/tasks/:id/approve - 通过审核
async fn approve_task(
    Path(id): Path<String>,
    Json(request): Json<ReviewActionRequest>,
) -> impl IntoResponse {
    update_task_status(id, "approved".to_string(), request.comment).await
}

/// POST /api/review/tasks/:id/reject - 驳回审核
async fn reject_task(
    Path(id): Path<String>,
    Json(request): Json<ReviewActionRequest>,
) -> impl IntoResponse {
    update_task_status(id, "rejected".to_string(), request.comment).await
}

/// POST /api/review/tasks/:id/cancel - 取消任务
async fn cancel_task(
    Path(id): Path<String>,
    Json(request): Json<ReviewActionRequest>,
) -> impl IntoResponse {
    update_task_status(id, "cancelled".to_string(), request.reason).await
}

async fn update_task_status(id: String, status: String, comment: Option<String>) -> (StatusCode, Json<ActionResponse>) {
    info!("Updating task {} status to {}", id, status);
    
    let sql = if comment.is_some() {
        "UPDATE review_tasks SET status = $status, review_comment = $comment, updated_at = time::now() WHERE record::id(id) = $id"
    } else {
        "UPDATE review_tasks SET status = $status, updated_at = time::now() WHERE record::id(id) = $id"
    };
    
    let mut q = SUL_DB.query(sql)
        .bind(("id", id.clone()))
        .bind(("status", status.clone()));
    
    if let Some(ref c) = comment {
        q = q.bind(("comment", c.clone()));
    }
    
    match q.await {
        Ok(_) => {
            // 记录历史
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
            let _ = SUL_DB.query(history_sql)
                .bind(("task_id", id))
                .bind(("action", status.clone()))
                .bind(("comment", comment))
                .await;
            
            (StatusCode::OK, Json(ActionResponse {
                success: true,
                message: Some(format!("任务状态已更新为: {}", status)),
                error_message: None,
            }))
        }
        Err(e) => {
            warn!("Failed to update task status: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ActionResponse {
                success: false,
                message: None,
                error_message: Some(format!("更新状态失败: {}", e)),
            }))
        }
    }
}

/// GET /api/review/tasks/:id/history - 获取审核历史
async fn get_task_history(
    Path(id): Path<String>,
) -> impl IntoResponse {
    info!("Getting task history: {}", id);
    
    #[derive(Debug, Serialize)]
    struct HistoryItem {
        id: String,
        task_id: String,
        action: String,
        user_id: String,
        user_name: String,
        comment: Option<String>,
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
        user_id: Option<String>,
        user_name: Option<String>,
        comment: Option<String>,
        timestamp: Option<String>,
    }
    
    let sql = "SELECT * FROM review_history WHERE task_id = $task_id ORDER BY timestamp DESC";
    
    match SUL_DB.query(sql).bind(("task_id", id.clone())).await {
        Ok(mut response) => {
            let rows: Vec<HistoryRow> = response.take(0).unwrap_or_default();
            let history: Vec<HistoryItem> = rows.into_iter().map(|r| HistoryItem {
                id: format!("{:?}", r.id.key),
                task_id: r.task_id.unwrap_or_default(),
                action: r.action.unwrap_or_default(),
                user_id: r.user_id.unwrap_or_default(),
                user_name: r.user_name.unwrap_or_default(),
                comment: r.comment,
                timestamp: parse_datetime(&r.timestamp),
            }).collect();
            
            (StatusCode::OK, Json(HistoryResponse {
                success: true,
                history,
                error_message: None,
            }))
        }
        Err(e) => {
            warn!("Failed to get task history: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(HistoryResponse {
                success: false,
                history: vec![],
                error_message: Some(format!("获取历史失败: {}", e)),
            }))
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
async fn create_record(
    Json(request): Json<ConfirmedRecordData>,
) -> impl IntoResponse {
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
    
    match SUL_DB.query(sql)
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
            (StatusCode::OK, Json(ConfirmedRecordResponse {
                success: true,
                record: Some(record),
                records: None,
                error_message: None,
            }))
        }
        Err(e) => {
            warn!("Failed to create record: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ConfirmedRecordResponse {
                success: false,
                record: None,
                records: None,
                error_message: Some(format!("保存记录失败: {}", e)),
            }))
        }
    }
}

/// GET /api/review/records/:task_id - 获取任务的确认记录
async fn get_records_by_task(
    Path(task_id): Path<String>,
) -> impl IntoResponse {
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
        confirmed_at: Option<String>,
    }
    
    let sql = "SELECT * FROM review_records WHERE task_id = $task_id ORDER BY confirmed_at DESC";
    
    match SUL_DB.query(sql).bind(("task_id", task_id)).await {
        Ok(mut response) => {
            let rows: Vec<RecordRow> = response.take(0).unwrap_or_default();
            let records: Vec<ConfirmedRecordWithMeta> = rows.into_iter().map(|r| ConfirmedRecordWithMeta {
                id: format!("{:?}", r.id.key),
                task_id: r.task_id.unwrap_or_default(),
                r#type: r.r#type.unwrap_or_else(|| "batch".to_string()),
                annotations: r.annotations.unwrap_or_default(),
                cloud_annotations: r.cloud_annotations.unwrap_or_default(),
                rect_annotations: r.rect_annotations.unwrap_or_default(),
                obb_annotations: r.obb_annotations.unwrap_or_default(),
                measurements: r.measurements.unwrap_or_default(),
                note: r.note.unwrap_or_default(),
                confirmed_at: parse_datetime(&r.confirmed_at),
            }).collect();
            
            (StatusCode::OK, Json(ConfirmedRecordResponse {
                success: true,
                record: None,
                records: Some(records),
                error_message: None,
            }))
        }
        Err(e) => {
            warn!("Failed to get records: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ConfirmedRecordResponse {
                success: false,
                record: None,
                records: None,
                error_message: Some(format!("获取记录失败: {}", e)),
            }))
        }
    }
}

/// DELETE /api/review/records/:record_id - 删除记录
async fn delete_record(
    Path(record_id): Path<String>,
) -> impl IntoResponse {
    info!("Deleting record: {}", record_id);
    
    let sql = "DELETE review_records WHERE record::id(id) = $id";
    
    match SUL_DB.query(sql).bind(("id", record_id)).await {
        Ok(_) => (StatusCode::OK, Json(ActionResponse {
            success: true,
            message: Some("记录已删除".to_string()),
            error_message: None,
        })),
        Err(e) => {
            warn!("Failed to delete record: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ActionResponse {
                success: false,
                message: None,
                error_message: Some(format!("删除记录失败: {}", e)),
            }))
        }
    }
}

/// DELETE /api/review/records/task/:task_id - 清空任务的所有记录
async fn clear_records_by_task(
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    info!("Clearing records for task: {}", task_id);
    
    let sql = "DELETE FROM review_records WHERE task_id = $task_id";
    
    match SUL_DB.query(sql).bind(("task_id", task_id)).await {
        Ok(_) => (StatusCode::OK, Json(ActionResponse {
            success: true,
            message: Some("记录已清空".to_string()),
            error_message: None,
        })),
        Err(e) => {
            warn!("Failed to clear records: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ActionResponse {
                success: false,
                message: None,
                error_message: Some(format!("清空记录失败: {}", e)),
            }))
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
async fn create_comment(
    Json(request): Json<CreateCommentRequest>,
) -> impl IntoResponse {
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
    
    match SUL_DB.query(sql)
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
            (StatusCode::OK, Json(CommentResponse {
                success: true,
                comment: Some(comment),
                comments: None,
                error_message: None,
            }))
        }
        Err(e) => {
            warn!("Failed to create comment: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(CommentResponse {
                success: false,
                comment: None,
                comments: None,
                error_message: Some(format!("创建评论失败: {}", e)),
            }))
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
        created_at: Option<String>,
    }
    
    let sql = if query.r#type.is_some() {
        "SELECT * FROM review_comments WHERE annotation_id = $annotation_id AND annotation_type = $type ORDER BY created_at ASC"
    } else {
        "SELECT * FROM review_comments WHERE annotation_id = $annotation_id ORDER BY created_at ASC"
    };
    
    let mut q = SUL_DB.query(sql).bind(("annotation_id", annotation_id));
    if let Some(ref t) = query.r#type {
        q = q.bind(("type", t.clone()));
    }
    
    match q.await {
        Ok(mut response) => {
            let rows: Vec<CommentRow> = response.take(0).unwrap_or_default();
            let comments: Vec<AnnotationComment> = rows.into_iter().map(|r| AnnotationComment {
                id: format!("{:?}", r.id.key),
                annotation_id: r.annotation_id.unwrap_or_default(),
                annotation_type: r.annotation_type.unwrap_or_default(),
                author_id: r.author_id.unwrap_or_default(),
                author_name: r.author_name.unwrap_or_default(),
                author_role: r.author_role.unwrap_or_default(),
                content: r.content.unwrap_or_default(),
                reply_to_id: r.reply_to_id,
                created_at: parse_datetime(&r.created_at),
            }).collect();
            
            (StatusCode::OK, Json(CommentResponse {
                success: true,
                comment: None,
                comments: Some(comments),
                error_message: None,
            }))
        }
        Err(e) => {
            warn!("Failed to get comments: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(CommentResponse {
                success: false,
                comment: None,
                comments: None,
                error_message: Some(format!("获取评论失败: {}", e)),
            }))
        }
    }
}

/// DELETE /api/review/comments/:comment_id - 删除评论
async fn delete_comment(
    Path(comment_id): Path<String>,
) -> impl IntoResponse {
    info!("Deleting comment: {}", comment_id);
    
    let sql = "DELETE review_comments WHERE record::id(id) = $id";
    
    match SUL_DB.query(sql).bind(("id", comment_id)).await {
        Ok(_) => (StatusCode::OK, Json(ActionResponse {
            success: true,
            message: Some("评论已删除".to_string()),
            error_message: None,
        })),
        Err(e) => {
            warn!("Failed to delete comment: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ActionResponse {
                success: false,
                message: None,
                error_message: Some(format!("删除评论失败: {}", e)),
            }))
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

#[derive(Debug, Serialize)]
pub struct UserListResponse {
    pub success: bool,
    pub users: Vec<User>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, Serialize)]
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

/// GET /api/users - 获取用户列表
async fn list_users(
    Query(query): Query<UserListQuery>,
) -> impl IntoResponse {
    info!("Listing users");
    
    // 暂时返回 mock 数据，后续从数据库查询
    let mock_users = vec![
        User {
            id: "user-001".to_string(),
            username: "designer1".to_string(),
            name: "设计师小张".to_string(),
            email: "zhang@example.com".to_string(),
            role: "designer".to_string(),
            department: Some("设计部".to_string()),
            avatar: None,
        },
        User {
            id: "user-002".to_string(),
            username: "reviewer1".to_string(),
            name: "校对员小李".to_string(),
            email: "li@example.com".to_string(),
            role: "proofreader".to_string(),
            department: Some("校审部".to_string()),
            avatar: None,
        },
        User {
            id: "user-003".to_string(),
            username: "reviewer2".to_string(),
            name: "审核员小王".to_string(),
            email: "wang@example.com".to_string(),
            role: "reviewer".to_string(),
            department: Some("校审部".to_string()),
            avatar: None,
        },
    ];
    
    let users = if let Some(ref role) = query.role {
        mock_users.into_iter().filter(|u| &u.role == role).collect()
    } else {
        mock_users
    };
    
    (StatusCode::OK, Json(UserListResponse {
        success: true,
        users,
        error_message: None,
    }))
}

/// GET /api/users/me - 获取当前用户
async fn get_current_user() -> impl IntoResponse {
    // 暂时返回 mock 用户，后续从 JWT 获取
    let user = User {
        id: "user-001".to_string(),
        username: "designer1".to_string(),
        name: "设计师小张".to_string(),
        email: "zhang@example.com".to_string(),
        role: "designer".to_string(),
        department: Some("设计部".to_string()),
        avatar: None,
    };
    
    (StatusCode::OK, Json(UserResponse {
        success: true,
        user: Some(user),
        error_message: None,
    }))
}

/// GET /api/users/reviewers - 获取审核人员列表
async fn get_reviewers() -> impl IntoResponse {
    info!("Getting reviewers");
    
    // 返回可以审核的用户（校对员和审核员）
    let reviewers = vec![
        User {
            id: "user-002".to_string(),
            username: "reviewer1".to_string(),
            name: "校对员小李".to_string(),
            email: "li@example.com".to_string(),
            role: "proofreader".to_string(),
            department: Some("校审部".to_string()),
            avatar: None,
        },
        User {
            id: "user-003".to_string(),
            username: "reviewer2".to_string(),
            name: "审核员小王".to_string(),
            email: "wang@example.com".to_string(),
            role: "reviewer".to_string(),
            department: Some("校审部".to_string()),
            avatar: None,
        },
    ];
    
    (StatusCode::OK, Json(UserListResponse {
        success: true,
        users: reviewers,
        error_message: None,
    }))
}
