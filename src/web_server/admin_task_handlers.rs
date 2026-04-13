use axum::{
    Router,
    extract::{Json, Path},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::LazyLock;
use std::sync::Mutex;

use crate::web_server::{
    AppState,
    models::{DatabaseConfig, TaskInfo, TaskPriority, TaskStatus, TaskType},
};

type ApiResponse = (StatusCode, Json<Value>);

static TASK_STORE: LazyLock<Mutex<Vec<TaskInfo>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

pub fn create_admin_task_routes() -> Router<AppState> {
    Router::new()
        .route("/api/admin/tasks", get(list_tasks).post(create_task))
        .route("/api/admin/tasks/{id}", get(get_task))
        .route("/api/admin/tasks/{id}/cancel", post(cancel_task))
        .route("/api/admin/tasks/{id}/retry", post(retry_task))
}

async fn list_tasks() -> impl IntoResponse {
    let tasks = TASK_STORE
        .lock()
        .map(|store| store.clone())
        .unwrap_or_default();
    ok("获取任务列表成功", tasks)
}

async fn get_task(Path(task_id): Path<String>) -> impl IntoResponse {
    let store = TASK_STORE.lock().unwrap_or_else(|e| e.into_inner());
    match store.iter().find(|t| t.id == task_id) {
        Some(task) => ok("获取任务详情成功", task.clone()),
        None => not_found(format!("任务不存在: {}", task_id)),
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

    let task = TaskInfo::new_with_priority(
        payload.task_name,
        task_type,
        config,
        priority,
    );

    let task_clone = task.clone();

    if let Ok(mut store) = TASK_STORE.lock() {
        store.insert(0, task);
    }

    response(StatusCode::CREATED, true, "创建任务成功", Some(task_clone))
}

async fn cancel_task(Path(task_id): Path<String>) -> impl IntoResponse {
    let mut store = TASK_STORE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(task) = store.iter_mut().find(|t| t.id == task_id) {
        if task.status == TaskStatus::Running || task.status == TaskStatus::Pending {
            task.status = TaskStatus::Cancelled;
            return ok("取消任务成功", json!({ "task_id": task_id }));
        }
        return response::<Value>(
            StatusCode::CONFLICT,
            false,
            format!("任务状态 {:?} 不允许取消", task.status),
            None,
        );
    }
    not_found(format!("任务不存在: {}", task_id))
}

async fn retry_task(Path(task_id): Path<String>) -> impl IntoResponse {
    let store = TASK_STORE.lock().unwrap_or_else(|e| e.into_inner());
    match store.iter().find(|t| t.id == task_id) {
        Some(task) if task.status == TaskStatus::Failed => {
            let mut new_task = task.clone();
            new_task.status = TaskStatus::Pending;
            new_task.error = None;
            new_task.error_details = None;
            drop(store);
            if let Ok(mut s) = TASK_STORE.lock() {
                if let Some(t) = s.iter_mut().find(|t| t.id == task_id) {
                    *t = new_task.clone();
                }
            }
            ok("重试任务成功", new_task)
        }
        Some(_) => response::<Value>(
            StatusCode::CONFLICT,
            false,
            "只有失败的任务可以重试",
            None,
        ),
        None => not_found(format!("任务不存在: {}", task_id)),
    }
}

fn ok<T: Serialize>(message: impl Into<String>, data: T) -> ApiResponse {
    response(StatusCode::OK, true, message, Some(data))
}

fn not_found(message: impl Into<String>) -> ApiResponse {
    response::<Value>(StatusCode::NOT_FOUND, false, message, None)
}

fn response<T>(
    status: StatusCode,
    success: bool,
    message: impl Into<String>,
    data: Option<T>,
) -> ApiResponse
where
    T: Serialize,
{
    (
        status,
        Json(json!({
            "success": success,
            "message": message.into(),
            "data": data
        })),
    )
}
