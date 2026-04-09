use axum::{
    Router,
    extract::{Json, Path},
    http::StatusCode,
    middleware,
    response::IntoResponse,
    routing::{get, post},
};
use serde_json::{Value, json};

use crate::web_server::admin_auth_handlers::admin_auth_middleware;
use crate::web_server::admin_response;
use crate::web_server::models::{
    DatabaseConfig, TaskInfo, TaskPriority, TaskStatus, TaskType,
};
use crate::web_server::wizard_handlers::open_deployment_sites_sqlite;

const TABLE_NAME: &str = "admin_tasks";

fn ensure_admin_tasks_table() -> Result<(), Box<dyn std::error::Error>> {
    let conn = open_deployment_sites_sqlite()?;
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS {TABLE_NAME} (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            task_type TEXT NOT NULL DEFAULT 'ParsePdmsData',
            status TEXT NOT NULL DEFAULT 'Pending',
            priority TEXT NOT NULL DEFAULT 'Normal',
            config_json TEXT NOT NULL DEFAULT '{{}}',
            site_id TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT,
            started_at TEXT,
            completed_at TEXT,
            progress_pct REAL DEFAULT 0,
            current_step TEXT DEFAULT '',
            error TEXT,
            error_details TEXT,
            logs_json TEXT DEFAULT '[]'
        )"
    ))?;
    Ok(())
}

pub fn create_admin_task_routes() -> Router {
    if let Err(e) = ensure_admin_tasks_table() {
        eprintln!("⚠️ Failed to init admin_tasks table: {e}");
    }

    Router::new()
        .route("/api/admin/tasks", get(list_tasks).post(create_task))
        .route("/api/admin/tasks/{id}", get(get_task))
        .route("/api/admin/tasks/{id}/cancel", post(cancel_task))
        .route("/api/admin/tasks/{id}/retry", post(retry_task))
        .layer(middleware::from_fn(admin_auth_middleware))
}

async fn list_tasks() -> impl IntoResponse {
    match load_all_tasks() {
        Ok(tasks) => admin_response::ok("获取任务列表成功", tasks),
        Err(e) => admin_response::server_error(format!("读取任务列表失败: {e}")),
    }
}

async fn get_task(Path(task_id): Path<String>) -> impl IntoResponse {
    match load_task_by_id(&task_id) {
        Ok(Some(task)) => admin_response::ok("获取任务详情成功", task),
        Ok(None) => admin_response::not_found(format!("任务不存在: {task_id}")),
        Err(e) => admin_response::server_error(format!("读取任务失败: {e}")),
    }
}

#[derive(serde::Deserialize)]
struct CreateTaskRequest {
    task_name: String,
    task_type: Option<TaskType>,
    priority: Option<TaskPriority>,
    #[serde(default)]
    site_id: Option<String>,
    #[serde(default)]
    config_override: Option<serde_json::Value>,
}

async fn create_task(Json(payload): Json<CreateTaskRequest>) -> impl IntoResponse {
    let task_type = payload.task_type.unwrap_or(TaskType::ParsePdmsData);
    let priority = payload.priority.unwrap_or(TaskPriority::Normal);

    let mut config = DatabaseConfig::default();
    config.name = payload.task_name.clone();

    if let Some(overrides) = payload.config_override {
        apply_config_overrides(&mut config, &overrides);
    }

    let task = TaskInfo::new_with_priority(payload.task_name, task_type, config, priority);

    match save_task(&task, payload.site_id.as_deref()) {
        Ok(_) => admin_response::response(StatusCode::CREATED, true, "创建任务成功", Some(task)),
        Err(e) => admin_response::server_error(format!("保存任务失败: {e}")),
    }
}

async fn cancel_task(Path(task_id): Path<String>) -> impl IntoResponse {
    match load_task_by_id(&task_id) {
        Ok(Some(task)) => {
            if task.status == TaskStatus::Running || task.status == TaskStatus::Pending {
                if let Err(e) = update_task_status(&task_id, TaskStatus::Cancelled) {
                    return admin_response::server_error(format!("更新任务状态失败: {e}"));
                }
                admin_response::ok("取消任务成功", json!({ "task_id": task_id }))
            } else {
                admin_response::conflict(format!("任务状态 {:?} 不允许取消", task.status))
            }
        }
        Ok(None) => admin_response::not_found(format!("任务不存在: {task_id}")),
        Err(e) => admin_response::server_error(format!("读取任务失败: {e}")),
    }
}

async fn retry_task(Path(task_id): Path<String>) -> impl IntoResponse {
    match load_task_by_id(&task_id) {
        Ok(Some(task)) if task.status == TaskStatus::Failed => {
            if let Err(e) = update_task_status(&task_id, TaskStatus::Pending) {
                return admin_response::server_error(format!("更新任务状态失败: {e}"));
            }
            let mut retried = task;
            retried.status = TaskStatus::Pending;
            retried.error = None;
            retried.error_details = None;
            admin_response::ok("重试任务成功", retried)
        }
        Ok(Some(_)) => admin_response::conflict("只有失败的任务可以重试"),
        Ok(None) => admin_response::not_found(format!("任务不存在: {task_id}")),
        Err(e) => admin_response::server_error(format!("读取任务失败: {e}")),
    }
}

fn apply_config_overrides(config: &mut DatabaseConfig, overrides: &serde_json::Value) {
    if let Some(obj) = overrides.as_object() {
        if let Some(v) = obj.get("gen_model").and_then(|v| v.as_bool()) {
            config.gen_model = v;
        }
        if let Some(v) = obj.get("gen_mesh").and_then(|v| v.as_bool()) {
            config.gen_mesh = v;
        }
        if let Some(v) = obj.get("gen_spatial_tree").and_then(|v| v.as_bool()) {
            config.gen_spatial_tree = v;
        }
        if let Some(v) = obj.get("apply_boolean_operation").and_then(|v| v.as_bool()) {
            config.apply_boolean_operation = v;
        }
        if let Some(v) = obj.get("mesh_tol_ratio").and_then(|v| v.as_f64()) {
            config.mesh_tol_ratio = v;
        }
        if let Some(v) = obj.get("manual_db_nums").and_then(|v| v.as_array()) {
            config.manual_db_nums =
                v.iter().filter_map(|n| n.as_u64().map(|n| n as u32)).collect();
        }
        if let Some(v) = obj.get("manual_refnos").and_then(|v| v.as_array()) {
            config.manual_refnos = v
                .iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect();
        }
        if let Some(v) = obj.get("export_json").and_then(|v| v.as_bool()) {
            config.export_json = v;
        }
        if let Some(v) = obj.get("export_parquet").and_then(|v| v.as_bool()) {
            config.export_parquet = v;
        }
    }
}

fn save_task(task: &TaskInfo, site_id: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let conn = open_deployment_sites_sqlite()?;
    let config_json = serde_json::to_string(&task.config)?;
    let logs_json = serde_json::to_string(&task.logs)?;
    let now = chrono::Utc::now().to_rfc3339();

    conn.execute(
        &format!(
            "INSERT OR REPLACE INTO {TABLE_NAME}
             (id, name, task_type, status, priority, config_json, site_id,
              created_at, updated_at, progress_pct, current_step, error, error_details, logs_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)"
        ),
        rusqlite::params![
            task.id,
            task.name,
            format!("{:?}", task.task_type),
            format!("{:?}", task.status),
            format!("{:?}", task.priority),
            config_json,
            site_id.unwrap_or(""),
            now,
            now,
            task.progress.percentage as f64,
            task.progress.current_step,
            task.error,
            task.error_details,
            logs_json,
        ],
    )?;
    Ok(())
}

fn update_task_status(
    task_id: &str,
    status: TaskStatus,
) -> Result<(), Box<dyn std::error::Error>> {
    let conn = open_deployment_sites_sqlite()?;
    let now = chrono::Utc::now().to_rfc3339();
    let status_str = format!("{:?}", status);

    conn.execute(
        &format!("UPDATE {TABLE_NAME} SET status = ?1, updated_at = ?2 WHERE id = ?3"),
        rusqlite::params![status_str, now, task_id],
    )?;
    Ok(())
}

fn load_all_tasks() -> Result<Vec<TaskInfo>, Box<dyn std::error::Error>> {
    let conn = open_deployment_sites_sqlite()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT id, name, task_type, status, priority, config_json, created_at, error, error_details
         FROM {TABLE_NAME} ORDER BY created_at DESC LIMIT 100"
    ))?;

    let tasks = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let name: String = row.get(1)?;
            let task_type_str: String = row.get(2)?;
            let status_str: String = row.get(3)?;
            let priority_str: String = row.get(4)?;
            let config_json: String = row.get(5)?;
            let _created_at: String = row.get(6)?;
            let error: Option<String> = row.get(7)?;
            let error_details: Option<String> = row.get(8)?;

            let task_type = parse_task_type(&task_type_str);
            let status = parse_task_status(&status_str);
            let priority = parse_task_priority(&priority_str);
            let config =
                serde_json::from_str::<DatabaseConfig>(&config_json).unwrap_or_default();

            let mut task = TaskInfo::new_with_priority(name, task_type, config, priority);
            task.id = id;
            task.status = status;
            task.error = error;
            task.error_details = error_details;
            Ok(task)
        })?
        .filter_map(|r| r.ok())
        .collect();

    Ok(tasks)
}

fn load_task_by_id(task_id: &str) -> Result<Option<TaskInfo>, Box<dyn std::error::Error>> {
    let conn = open_deployment_sites_sqlite()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT id, name, task_type, status, priority, config_json, created_at, error, error_details
         FROM {TABLE_NAME} WHERE id = ?1"
    ))?;

    let mut rows = stmt.query(rusqlite::params![task_id])?;
    if let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let name: String = row.get(1)?;
        let task_type_str: String = row.get(2)?;
        let status_str: String = row.get(3)?;
        let priority_str: String = row.get(4)?;
        let config_json: String = row.get(5)?;
        let error: Option<String> = row.get(7)?;
        let error_details: Option<String> = row.get(8)?;

        let config = serde_json::from_str::<DatabaseConfig>(&config_json).unwrap_or_default();
        let mut task = TaskInfo::new_with_priority(
            name,
            parse_task_type(&task_type_str),
            config,
            parse_task_priority(&priority_str),
        );
        task.id = id;
        task.status = parse_task_status(&status_str);
        task.error = error;
        task.error_details = error_details;
        return Ok(Some(task));
    }
    Ok(None)
}

fn parse_task_type(s: &str) -> TaskType {
    match s {
        "DataGeneration" => TaskType::DataGeneration,
        "SpatialTreeGeneration" => TaskType::SpatialTreeGeneration,
        "FullGeneration" => TaskType::FullGeneration,
        "MeshGeneration" => TaskType::MeshGeneration,
        "ParsePdmsData" => TaskType::ParsePdmsData,
        "GenerateGeometry" => TaskType::GenerateGeometry,
        "BuildSpatialIndex" => TaskType::BuildSpatialIndex,
        "BatchDatabaseProcess" => TaskType::BatchDatabaseProcess,
        "BatchGeometryGeneration" => TaskType::BatchGeometryGeneration,
        "DataExport" => TaskType::DataExport,
        "DataImport" => TaskType::DataImport,
        "DataParsingWizard" => TaskType::DataParsingWizard,
        "RefnoModelGeneration" => TaskType::RefnoModelGeneration,
        "ModelExport" => TaskType::ModelExport,
        other => TaskType::Custom(other.to_string()),
    }
}

fn parse_task_status(s: &str) -> TaskStatus {
    match s {
        "Pending" => TaskStatus::Pending,
        "Running" => TaskStatus::Running,
        "Completed" => TaskStatus::Completed,
        "Failed" => TaskStatus::Failed,
        "Cancelled" => TaskStatus::Cancelled,
        _ => TaskStatus::Pending,
    }
}

fn parse_task_priority(s: &str) -> TaskPriority {
    match s {
        "Low" => TaskPriority::Low,
        "Normal" => TaskPriority::Normal,
        "High" => TaskPriority::High,
        "Urgent" | "Critical" => TaskPriority::Urgent,
        _ => TaskPriority::Normal,
    }
}
