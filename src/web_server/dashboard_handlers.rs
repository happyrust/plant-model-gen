use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;
use tracing::warn;

use super::{
    AppState, TaskManager,
    models::{TaskInfo, TaskStatus, TaskType},
};

#[derive(Debug, Deserialize)]
pub struct DashboardActivitiesQuery {
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DashboardActivityItem {
    pub id: String,
    pub source: String,
    pub user_id: String,
    pub user_name: String,
    pub user_type: String,
    pub action_title: String,
    pub target_name: String,
    pub action_desc: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DashboardActivitiesResponse {
    pub success: bool,
    pub data: Vec<DashboardActivityItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, serde::Deserialize, SurrealValue)]
struct ReviewActivityRow {
    task_id: Option<String>,
    action: Option<String>,
    node: Option<String>,
    operator_id: Option<String>,
    operator_name: Option<String>,
    comment: Option<String>,
    timestamp: Option<surrealdb::types::Datetime>,
}

fn task_type_label(task_type: &TaskType) -> &'static str {
    match task_type {
        TaskType::DataGeneration => "数据生成",
        TaskType::SpatialTreeGeneration => "空间树构建",
        TaskType::FullGeneration => "完整生成",
        TaskType::MeshGeneration => "网格生成",
        TaskType::ParsePdmsData => "PDMS 解析",
        TaskType::GenerateGeometry => "几何生成",
        TaskType::BuildSpatialIndex => "空间索引构建",
        TaskType::BatchDatabaseProcess => "批量数据库处理",
        TaskType::BatchGeometryGeneration => "批量几何生成",
        TaskType::DataExport => "数据导出",
        TaskType::DataImport => "数据导入",
        TaskType::DataParsingWizard => "数据解析向导",
        TaskType::RefnoModelGeneration => "按 Refno 生成模型",
        TaskType::ModelExport => "模型导出",
        TaskType::Custom(_) => "自定义任务",
    }
}

fn workflow_action_label(action: &str) -> &'static str {
    match action {
        "submit" => "提交了校审任务",
        "return" => "退回了校审任务",
        "approve" => "通过了校审任务",
        "reject" => "驳回了校审任务",
        "created" => "创建了校审任务",
        _ => "更新了校审任务",
    }
}

fn workflow_node_label(node: &str) -> &'static str {
    match node {
        "sj" => "编制",
        "jd" => "校核",
        "sh" => "审核",
        "pz" => "批准",
        _ => "流程节点",
    }
}

fn to_iso_string(time: std::time::SystemTime) -> String {
    DateTime::<Utc>::from(time).to_rfc3339()
}

fn to_task_activity(task: &TaskInfo) -> DashboardActivityItem {
    let (action_title, action_desc, created_at) = match task.status {
        TaskStatus::Pending => (
            "创建了系统任务".to_string(),
            format!("{} · 等待执行", task_type_label(&task.task_type)),
            to_iso_string(task.created_at),
        ),
        TaskStatus::Running => (
            "启动了系统任务".to_string(),
            format!(
                "{} · {}",
                task_type_label(&task.task_type),
                task.progress.current_step
            ),
            task.started_at
                .map(to_iso_string)
                .unwrap_or_else(|| to_iso_string(task.created_at)),
        ),
        TaskStatus::Completed => (
            "完成了系统任务".to_string(),
            format!(
                "{} · 已处理 {} 项",
                task_type_label(&task.task_type),
                task.progress.processed_items
            ),
            task.completed_at
                .map(to_iso_string)
                .unwrap_or_else(|| to_iso_string(task.created_at)),
        ),
        TaskStatus::Failed => (
            "系统任务失败".to_string(),
            task.error
                .clone()
                .unwrap_or_else(|| task.progress.current_step.clone()),
            task.completed_at
                .map(to_iso_string)
                .unwrap_or_else(|| to_iso_string(task.created_at)),
        ),
        TaskStatus::Cancelled => (
            "取消了系统任务".to_string(),
            task.progress.current_step.clone(),
            task.completed_at
                .map(to_iso_string)
                .unwrap_or_else(|| to_iso_string(task.created_at)),
        ),
    };

    DashboardActivityItem {
        id: format!("task:{}", task.id),
        source: "task".to_string(),
        user_id: "system".to_string(),
        user_name: "系统".to_string(),
        user_type: "system_bot".to_string(),
        action_title,
        target_name: task.name.clone(),
        action_desc,
        created_at,
    }
}

fn merge_and_limit_activities(
    review_items: Vec<DashboardActivityItem>,
    task_items: Vec<DashboardActivityItem>,
    limit: usize,
) -> Vec<DashboardActivityItem> {
    let mut merged = review_items;
    merged.extend(task_items);
    merged.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    merged.truncate(limit);
    merged
}

fn collect_task_activities(task_manager: &TaskManager) -> Vec<DashboardActivityItem> {
    let mut items = Vec::new();
    items.extend(task_manager.active_tasks.values().map(to_task_activity));
    items.extend(task_manager.task_history.iter().map(to_task_activity));
    items
}

async fn load_review_activities(limit: usize) -> Vec<DashboardActivityItem> {
    let sql = r#"
        SELECT task_id, action, node, operator_id, operator_name, comment, timestamp
        FROM review_workflow_history
        ORDER BY timestamp DESC
        LIMIT $limit
    "#;

    match aios_core::project_primary_db()
        .query(sql)
        .bind(("limit", limit as i64))
        .await
    {
        Ok(mut response) => {
            let rows: Vec<ReviewActivityRow> = response.take(0).unwrap_or_default();
            rows.into_iter()
                .map(|row| DashboardActivityItem {
                    id: format!(
                        "review:{}:{}",
                        row.task_id.clone().unwrap_or_default(),
                        row.timestamp
                            .as_ref()
                            .map(|time| time.timestamp_millis())
                            .unwrap_or_default()
                    ),
                    source: "review".to_string(),
                    user_id: row.operator_id.unwrap_or_default(),
                    user_name: row.operator_name.unwrap_or_else(|| "未知用户".to_string()),
                    user_type: "human".to_string(),
                    action_title: workflow_action_label(row.action.as_deref().unwrap_or_default())
                        .to_string(),
                    target_name: row.task_id.unwrap_or_else(|| "未命名任务".to_string()),
                    action_desc: match row.comment {
                        Some(comment) if !comment.trim().is_empty() => comment,
                        _ => {
                            workflow_node_label(row.node.as_deref().unwrap_or_default()).to_string()
                        }
                    },
                    created_at: row
                        .timestamp
                        .map(|time| time.to_string())
                        .unwrap_or_else(|| Utc::now().to_rfc3339()),
                })
                .collect()
        }
        Err(error) => {
            warn!("[dashboard] load review activities failed: {}", error);
            Vec::new()
        }
    }
}

pub async fn api_dashboard_activities(
    Query(query): Query<DashboardActivitiesQuery>,
    State(state): State<AppState>,
) -> Result<Json<DashboardActivitiesResponse>, StatusCode> {
    let limit = query.limit.unwrap_or(10).clamp(1, 50);

    let review_items = load_review_activities(limit).await;
    let task_items = {
        let task_manager = state.task_manager.lock().await;
        collect_task_activities(&task_manager)
    };

    Ok(Json(DashboardActivitiesResponse {
        success: true,
        data: merge_and_limit_activities(review_items, task_items, limit),
        error_message: None,
    }))
}
