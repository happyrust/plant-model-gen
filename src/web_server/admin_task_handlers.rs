use axum::{
    Router,
    extract::{Json, Path},
    http::StatusCode,
    middleware,
    response::IntoResponse,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use futures_util::FutureExt;
use rusqlite::Row;
use std::panic::AssertUnwindSafe;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::web_server::admin_auth_handlers::admin_auth_middleware;
use crate::web_server::admin_response;
use crate::web_server::managed_project_sites;
use crate::web_server::models::{
    DatabaseConfig, ErrorDetails, LogEntry, LogLevel, ManagedSiteParseStatus,
    ManagedSiteRuntimeStatus, ManagedSiteStatus, TaskInfo, TaskPriority, TaskProgress, TaskStatus,
    TaskType,
};
use crate::web_server::wizard_handlers::open_deployment_sites_sqlite;

const TABLE_NAME: &str = "admin_tasks";
const ADMIN_TASK_SUBMITTED_STEP: &str = "已提交，等待站点状态更新";
const ADMIN_TASK_TOTAL_STEPS: u32 = 4;

#[derive(Clone)]
struct StoredAdminTask {
    task: TaskInfo,
    site_id: Option<String>,
}

#[derive(serde::Deserialize)]
struct StoredLogEntry {
    timestamp: Option<u64>,
    level: LogLevel,
    message: String,
    error_code: Option<String>,
    stack_trace: Option<String>,
}

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

pub fn cleanup_old_tasks() {
    if let Ok(conn) = open_deployment_sites_sqlite() {
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(7)).to_rfc3339();
        let deleted = conn
            .execute(
                &format!(
                    "DELETE FROM {TABLE_NAME} WHERE status IN ('Failed', 'Cancelled', 'Completed') AND updated_at < ?1"
                ),
                rusqlite::params![cutoff],
            )
            .unwrap_or(0);
        if deleted > 0 {
            eprintln!("🧹 已清理 {deleted} 条超过 7 天的已完结 admin 任务");
        }
    }
}

pub fn insert_task(task: TaskInfo) {
    let site_id = task.site_id.clone();
    let _ = save_task(&task, site_id.as_deref());
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
    let site_id = normalize_site_id(payload.site_id);

    if !is_supported_admin_task_type(&task_type) {
        return admin_response::bad_request(
            "当前 admin 仅支持 ParsePdmsData、DataGeneration、FullGeneration",
        );
    }

    let Some(site_id) = site_id else {
        return admin_response::bad_request("创建 admin 任务必须指定 site_id");
    };

    let mut config = DatabaseConfig::default();
    config.name = payload.task_name.clone();

    if let Some(overrides) = payload.config_override {
        apply_config_overrides(&mut config, &overrides);
    }

    let mut task = TaskInfo::new_with_priority(payload.task_name, task_type, config, priority);
    mark_task_running(&mut task, ADMIN_TASK_SUBMITTED_STEP, 10.0);

    match save_task(&task, Some(site_id.as_str())) {
        Ok(_) => {
            let task_id = task.id.clone();
            tokio::spawn(async move {
                if let Err(e) = std::panic::AssertUnwindSafe(dispatch_admin_task(task_id.clone()))
                    .catch_unwind()
                    .await
                {
                    eprintln!("❌ dispatch_admin_task({task_id}) panicked: {e:?}");
                }
            });
            admin_response::response(StatusCode::CREATED, true, "创建任务成功", Some(task))
        }
        Err(e) => admin_response::server_error(format!("保存任务失败: {e}")),
    }
}

async fn cancel_task(Path(task_id): Path<String>) -> impl IntoResponse {
    match load_task_by_id(&task_id) {
        Ok(Some(_)) => admin_response::conflict("当前 admin 任务暂不支持取消"),
        Ok(None) => admin_response::not_found(format!("任务不存在: {task_id}")),
        Err(e) => admin_response::server_error(format!("读取任务失败: {e}")),
    }
}

async fn retry_task(Path(task_id): Path<String>) -> impl IntoResponse {
    match load_task_record_by_id(&task_id) {
        Ok(Some(stored)) if stored.task.status == TaskStatus::Failed => {
            let Some(site_id) = stored.site_id.clone() else {
                return admin_response::conflict("任务缺少 site_id，无法重试");
            };

            let mut retried = TaskInfo::new_with_priority(
                stored.task.name.clone(),
                stored.task.task_type.clone(),
                stored.task.config.clone(),
                stored.task.priority.clone(),
            );
            mark_task_running(&mut retried, ADMIN_TASK_SUBMITTED_STEP, 10.0);

            match save_task(&retried, Some(site_id.as_str())) {
                Ok(_) => {
                    let retried_id = retried.id.clone();
                    tokio::spawn(async move {
                        if let Err(e) = AssertUnwindSafe(dispatch_admin_task(retried_id.clone()))
                            .catch_unwind()
                            .await
                        {
                            eprintln!("❌ dispatch_admin_task({retried_id}) panicked: {e:?}");
                        }
                    });
                    admin_response::response(
                        StatusCode::CREATED,
                        true,
                        "重试任务成功",
                        Some(retried),
                    )
                }
                Err(e) => admin_response::server_error(format!("保存重试任务失败: {e}")),
            }
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
            config.manual_db_nums = v
                .iter()
                .filter_map(|n| n.as_u64().map(|n| n as u32))
                .collect();
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
    let created_at = system_time_to_rfc3339(task.created_at);
    let started_at = optional_system_time_to_rfc3339(task.started_at);
    let completed_at = optional_system_time_to_rfc3339(task.completed_at);

    conn.execute(
        &format!(
            "INSERT OR REPLACE INTO {TABLE_NAME}
             (id, name, task_type, status, priority, config_json, site_id,
              created_at, updated_at, started_at, completed_at, progress_pct, current_step, error, error_details, logs_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)"
        ),
        rusqlite::params![
            &task.id,
            &task.name,
            format!("{:?}", task.task_type),
            format!("{:?}", task.status),
            format!("{:?}", task.priority),
            &config_json,
            site_id,
            created_at,
            &now,
            started_at,
            completed_at,
            task.progress.percentage as f64,
            &task.progress.current_step,
            &task.error,
            task.error_details.as_ref().and_then(|d| serde_json::to_string(d).ok()),
            &logs_json,
        ],
    )?;
    Ok(())
}

fn load_all_tasks() -> Result<Vec<TaskInfo>, Box<dyn std::error::Error>> {
    let conn = open_deployment_sites_sqlite()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT id, name, task_type, status, priority, config_json, site_id,
                created_at, updated_at, started_at, completed_at,
                progress_pct, current_step, error, error_details, logs_json
         FROM {TABLE_NAME} ORDER BY created_at DESC LIMIT 100"
    ))?;

    let stored_tasks = stmt
        .query_map([], task_from_row)?
        .collect::<Result<Vec<_>, _>>()?;

    let mut tasks = Vec::with_capacity(stored_tasks.len());
    for stored in stored_tasks {
        tasks.push(reconcile_task_record(stored)?.task);
    }

    Ok(tasks)
}

fn load_task_by_id(task_id: &str) -> Result<Option<TaskInfo>, Box<dyn std::error::Error>> {
    Ok(load_task_record_by_id(task_id)?
        .map(reconcile_task_record)
        .transpose()?
        .map(|stored| stored.task))
}

fn load_task_record_by_id(
    task_id: &str,
) -> Result<Option<StoredAdminTask>, Box<dyn std::error::Error>> {
    let conn = open_deployment_sites_sqlite()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT id, name, task_type, status, priority, config_json, site_id,
                created_at, updated_at, started_at, completed_at,
                progress_pct, current_step, error, error_details, logs_json
         FROM {TABLE_NAME} WHERE id = ?1"
    ))?;

    let mut rows = stmt.query(rusqlite::params![task_id])?;
    if let Some(row) = rows.next()? {
        return Ok(Some(task_from_row(row)?));
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

async fn dispatch_admin_task(task_id: String) {
    let Some(mut stored) = load_task_record_by_id(&task_id).ok().flatten() else {
        return;
    };

    let result: Result<(), String> = match (&stored.task.task_type, stored.site_id.as_deref()) {
        (TaskType::ParsePdmsData, Some(sid)) => {
            mark_task_running(&mut stored.task, "已提交解析任务，等待站点状态更新", 10.0);
            let _ = save_task(&stored.task, stored.site_id.as_deref());
            match crate::web_server::managed_project_sites::parse_site(sid.to_string()).await {
                Ok(()) => Ok(()),
                Err(e) => Err(e.to_string()),
            }
        }
        (TaskType::DataGeneration | TaskType::FullGeneration, Some(sid)) => {
            mark_task_running(&mut stored.task, "已提交启动任务，等待站点状态更新", 10.0);
            let _ = save_task(&stored.task, stored.site_id.as_deref());
            match crate::web_server::managed_project_sites::start_site(sid.to_string()).await {
                Ok(()) => Ok(()),
                Err(e) => Err(e.to_string()),
            }
        }
        (_, Some(_)) => {
            Err("当前 admin 仅支持 ParsePdmsData、DataGeneration、FullGeneration".into())
        }
        (_, None) => Err("创建 admin 任务必须指定 site_id".into()),
    };

    match result {
        Ok(()) => {}
        Err(msg) => {
            mark_task_failed(&mut stored.task, "提交站点动作失败", &msg);
            let _ = save_task(&stored.task, stored.site_id.as_deref());
        }
    }
}

fn normalize_site_id(site_id: Option<String>) -> Option<String> {
    site_id.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn is_supported_admin_task_type(task_type: &TaskType) -> bool {
    matches!(
        task_type,
        TaskType::ParsePdmsData | TaskType::DataGeneration | TaskType::FullGeneration
    )
}

fn task_from_row(row: &Row<'_>) -> rusqlite::Result<StoredAdminTask> {
    let id: String = row.get("id")?;
    let name: String = row.get("name")?;
    let task_type = parse_task_type(&row.get::<_, String>("task_type")?);
    let status = parse_task_status(&row.get::<_, String>("status")?);
    let priority = parse_task_priority(&row.get::<_, String>("priority")?);
    let config_json: String = row.get("config_json")?;
    let config = serde_json::from_str::<DatabaseConfig>(&config_json).unwrap_or_default();
    let site_id = normalize_site_id(row.get::<_, Option<String>>("site_id")?);
    let created_at = row
        .get::<_, String>("created_at")
        .ok()
        .and_then(|value| parse_rfc3339_to_system_time(&value))
        .unwrap_or_else(SystemTime::now);
    let started_at = row
        .get::<_, Option<String>>("started_at")?
        .and_then(|value| parse_rfc3339_to_system_time(&value));
    let completed_at = row
        .get::<_, Option<String>>("completed_at")?
        .and_then(|value| parse_rfc3339_to_system_time(&value));
    let progress_pct = row.get::<_, Option<f64>>("progress_pct")?.unwrap_or(0.0) as f32;
    let current_step = row
        .get::<_, Option<String>>("current_step")?
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "初始化".to_string());
    let error: Option<String> = row.get("error")?;
    let error_details = row
        .get::<_, Option<String>>("error_details")?
        .and_then(|value| serde_json::from_str::<ErrorDetails>(&value).ok());
    let logs = row
        .get::<_, Option<String>>("logs_json")?
        .map(|value| parse_logs_json(&value))
        .unwrap_or_default();

    Ok(StoredAdminTask {
        site_id: site_id.clone(),
        task: TaskInfo {
            id,
            name,
            task_type,
            status,
            config,
            created_at,
            started_at,
            completed_at,
            progress: build_progress(&current_step, progress_pct),
            error,
            error_details,
            logs,
            priority,
            dependencies: Vec::new(),
            estimated_duration: None,
            actual_duration: compute_actual_duration(started_at, completed_at),
            metadata: None,
            site_id,
            site_label: None,
        },
    })
}

fn reconcile_task_record(
    mut stored: StoredAdminTask,
) -> Result<StoredAdminTask, Box<dyn std::error::Error>> {
    if !is_supported_admin_task_type(&stored.task.task_type) {
        return Ok(stored);
    }
    if !matches!(
        stored.task.status,
        TaskStatus::Running | TaskStatus::Pending
    ) {
        return Ok(stored);
    }

    let Some(site_id) = stored.site_id.clone() else {
        mark_task_failed(
            &mut stored.task,
            "缺少关联站点",
            "任务缺少 site_id，无法对账运行状态",
        );
        save_task(&stored.task, None)?;
        return Ok(stored);
    };

    match managed_project_sites::runtime_status(&site_id) {
        Ok(runtime) => {
            apply_runtime_to_task(&mut stored.task, &runtime);
            save_task(&stored.task, Some(site_id.as_str()))?;
        }
        Err(err) => {
            mark_task_failed(&mut stored.task, "读取站点状态失败", &err.to_string());
            save_task(&stored.task, Some(site_id.as_str()))?;
        }
    }
    Ok(stored)
}

fn apply_runtime_to_task(task: &mut TaskInfo, runtime: &ManagedSiteRuntimeStatus) {
    let runtime_step = runtime_step(runtime);
    match task.task_type {
        TaskType::ParsePdmsData => {
            if runtime.parse_status == ManagedSiteParseStatus::Failed
                || runtime
                    .last_error
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
            {
                let message = runtime
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "站点解析失败".to_string());
                mark_task_failed(task, &runtime_step, &message);
            } else if runtime.parse_status == ManagedSiteParseStatus::Parsed {
                mark_task_completed(task, &runtime_step);
            } else if runtime.parse_status == ManagedSiteParseStatus::Running {
                mark_task_running(task, &runtime_step, 40.0);
            } else {
                mark_task_running(task, &runtime_step, 10.0);
            }
        }
        TaskType::DataGeneration | TaskType::FullGeneration => {
            if runtime.status == ManagedSiteStatus::Failed
                || runtime
                    .last_error
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
            {
                let message = runtime
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "站点启动失败".to_string());
                mark_task_failed(task, &runtime_step, &message);
            } else if runtime.status == ManagedSiteStatus::Running {
                mark_task_completed(task, &runtime_step);
            } else if runtime.parse_status == ManagedSiteParseStatus::Running {
                mark_task_running(task, &runtime_step, 40.0);
            } else if runtime.parse_status == ManagedSiteParseStatus::Parsed
                && runtime.status != ManagedSiteStatus::Starting
            {
                mark_task_running(task, &runtime_step, 70.0);
            } else if runtime.status == ManagedSiteStatus::Starting {
                mark_task_running(task, &runtime_step, 85.0);
            } else {
                mark_task_running(task, &runtime_step, 10.0);
            }
        }
        _ => {}
    }
}

fn runtime_step(runtime: &ManagedSiteRuntimeStatus) -> String {
    let label = runtime.current_stage_label.trim();
    let detail = runtime
        .current_stage_detail
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    match (label.is_empty(), detail) {
        (false, Some(detail)) => format!("{label}：{detail}"),
        (false, None) => label.to_string(),
        (true, Some(detail)) => detail.to_string(),
        (true, None) => ADMIN_TASK_SUBMITTED_STEP.to_string(),
    }
}

fn mark_task_running(task: &mut TaskInfo, step: &str, percentage: f32) {
    task.status = TaskStatus::Running;
    if task.started_at.is_none() {
        task.started_at = Some(SystemTime::now());
    }
    task.completed_at = None;
    task.actual_duration = None;
    task.error = None;
    task.error_details = None;
    task.progress = build_progress(step, percentage);
}

fn mark_task_completed(task: &mut TaskInfo, step: &str) {
    task.status = TaskStatus::Completed;
    if task.started_at.is_none() {
        task.started_at = Some(SystemTime::now());
    }
    task.completed_at = Some(SystemTime::now());
    task.actual_duration = compute_actual_duration(task.started_at, task.completed_at);
    task.error = None;
    task.error_details = None;
    task.progress = build_progress(step, 100.0);
}

fn mark_task_failed(task: &mut TaskInfo, step: &str, message: &str) {
    task.status = TaskStatus::Failed;
    if task.started_at.is_none() {
        task.started_at = Some(SystemTime::now());
    }
    task.completed_at = Some(SystemTime::now());
    task.actual_duration = compute_actual_duration(task.started_at, task.completed_at);
    task.error = Some(message.to_string());
    task.error_details = None;
    task.progress = build_progress(step, task.progress.percentage.max(10.0));
}

fn build_progress(step: &str, percentage: f32) -> TaskProgress {
    let mut progress = TaskProgress::default();
    let normalized_percentage = percentage.clamp(0.0, 100.0);
    progress.current_step = if step.trim().is_empty() {
        "初始化".to_string()
    } else {
        step.to_string()
    };
    progress.total_steps = ADMIN_TASK_TOTAL_STEPS;
    progress.current_step_number = if normalized_percentage >= 100.0 {
        ADMIN_TASK_TOTAL_STEPS
    } else if normalized_percentage >= 70.0 {
        3
    } else if normalized_percentage >= 40.0 {
        2
    } else if normalized_percentage >= 10.0 {
        1
    } else {
        0
    };
    progress.percentage = normalized_percentage;
    progress
}

fn compute_actual_duration(
    started_at: Option<SystemTime>,
    completed_at: Option<SystemTime>,
) -> Option<u64> {
    let started_at = started_at?;
    let completed_at = completed_at?;
    completed_at
        .duration_since(started_at)
        .ok()
        .map(|duration| duration.as_millis() as u64)
}

fn system_time_to_rfc3339(time: SystemTime) -> String {
    let datetime: DateTime<Utc> = time.into();
    datetime.to_rfc3339()
}

fn optional_system_time_to_rfc3339(time: Option<SystemTime>) -> Option<String> {
    time.map(system_time_to_rfc3339)
}

fn parse_rfc3339_to_system_time(raw: &str) -> Option<SystemTime> {
    let datetime = DateTime::parse_from_rfc3339(raw).ok()?;
    Some(datetime.with_timezone(&Utc).into())
}

fn parse_logs_json(raw: &str) -> Vec<LogEntry> {
    serde_json::from_str::<Vec<StoredLogEntry>>(raw)
        .map(|entries| {
            entries
                .into_iter()
                .map(|entry| LogEntry {
                    timestamp: entry
                        .timestamp
                        .map(|millis| UNIX_EPOCH + std::time::Duration::from_millis(millis))
                        .unwrap_or_else(SystemTime::now),
                    level: entry.level,
                    message: entry.message,
                    error_code: entry.error_code,
                    stack_trace: entry.stack_trace,
                })
                .collect()
        })
        .unwrap_or_default()
}
