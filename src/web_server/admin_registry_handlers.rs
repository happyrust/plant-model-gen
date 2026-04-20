use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    middleware,
    response::IntoResponse,
    routing::{delete, get, post, put},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::web_server::{
    AppState, admin_auth_handlers, admin_task_handlers,
    admin_response::{self, ApiResponse},
    handlers,
    models::{
        DatabaseConfig, DeploymentSiteCreateRequest, DeploymentSiteImportRequest,
        DeploymentSiteQuery, DeploymentSiteTaskRequest, DeploymentSiteUpdateRequest,
        TaskInfo,
    },
};

#[derive(Debug, Deserialize)]
pub struct AdminRegistryTaskRequest {
    #[serde(default)]
    pub task_name: Option<String>,
    #[serde(default)]
    pub task_type: Option<crate::web_server::models::TaskType>,
    #[serde(default)]
    pub priority: Option<crate::web_server::models::TaskPriority>,
    #[serde(default)]
    pub config_override: Option<DatabaseConfig>,
}

pub fn create_admin_registry_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/admin/registry/sites",
            get(list_sites).post(create_site),
        )
        .route(
            "/api/admin/registry/sites/{id}",
            get(get_site).put(update_site).delete(delete_site),
        )
        .route(
            "/api/admin/registry/import-dboption",
            post(import_site_from_dboption),
        )
        .route(
            "/api/admin/registry/sites/{id}/healthcheck",
            post(healthcheck_site),
        )
        .route(
            "/api/admin/registry/sites/{id}/export-config",
            get(export_site_config),
        )
        .route(
            "/api/admin/registry/sites/{id}/tasks",
            post(create_site_task),
        )
        .layer(middleware::from_fn(admin_auth_handlers::admin_auth_middleware))
}

async fn list_sites(Query(params): Query<DeploymentSiteQuery>) -> impl IntoResponse {
    match handlers::api_get_deployment_sites(Query(params)).await {
        Ok(Json(value)) => admin_response::ok("获取注册表站点列表成功", value),
        Err(status) => admin_response::response::<Value>(status, false, "获取注册表站点列表失败", None),
    }
}

async fn create_site(Json(payload): Json<DeploymentSiteCreateRequest>) -> impl IntoResponse {
    match handlers::api_create_deployment_site(Json(payload)).await {
        Ok(Json(value)) => admin_response::accepted("创建注册表站点成功", unwrap_item_or_value(value)),
        Err((status, Json(value))) => extract_proxy_error(status, value, "创建注册表站点失败"),
    }
}

async fn get_site(Path(site_id): Path<String>) -> impl IntoResponse {
    match handlers::api_get_deployment_site(Path(site_id)).await {
        Ok(Json(value)) => admin_response::ok("获取注册表站点详情成功", value),
        Err(status) => admin_response::response::<Value>(status, false, "获取注册表站点详情失败", None),
    }
}

async fn update_site(
    Path(site_id): Path<String>,
    Json(payload): Json<DeploymentSiteUpdateRequest>,
) -> impl IntoResponse {
    match handlers::api_update_deployment_site(Path(site_id), Json(payload)).await {
        Ok(Json(value)) => admin_response::ok("更新注册表站点成功", unwrap_item_or_value(value)),
        Err((status, Json(value))) => extract_proxy_error(status, value, "更新注册表站点失败"),
    }
}

async fn delete_site(Path(site_id): Path<String>) -> impl IntoResponse {
    match handlers::api_delete_deployment_site(Path(site_id.clone())).await {
        Ok(_) => admin_response::ok("删除注册表站点成功", json!({"site_id": site_id, "deleted": true})),
        Err(status) => admin_response::response::<Value>(status, false, "删除注册表站点失败", None),
    }
}

async fn import_site_from_dboption(
    payload: Option<Json<DeploymentSiteImportRequest>>,
) -> impl IntoResponse {
    match handlers::api_import_deployment_site_from_dboption(payload).await {
        Ok(Json(value)) => admin_response::accepted("导入注册表站点成功", unwrap_item_or_value(value)),
        Err((status, Json(value))) => extract_proxy_error(status, value, "导入注册表站点失败"),
    }
}

async fn healthcheck_site(Path(site_id): Path<String>) -> impl IntoResponse {
    match handlers::api_healthcheck_deployment_site_post(Path(site_id), None).await {
        Ok(Json(value)) => admin_response::ok("站点健康检查完成", value),
        Err((status, Json(value))) => extract_proxy_error(status, value, "站点健康检查失败"),
    }
}

async fn export_site_config(Path(site_id): Path<String>) -> impl IntoResponse {
    match handlers::api_export_deployment_site_config(Path(site_id)).await {
        Ok(Json(value)) => admin_response::ok("导出站点配置成功", value),
        Err(status) => admin_response::response::<Value>(status, false, "导出站点配置失败", None),
    }
}

async fn create_site_task(
    State(state): State<AppState>,
    Path(site_id): Path<String>,
    Json(payload): Json<AdminRegistryTaskRequest>,
) -> impl IntoResponse {
    let task_type = payload
        .task_type
        .unwrap_or(crate::web_server::models::TaskType::ParsePdmsData);
    let request = DeploymentSiteTaskRequest {
        site_id: site_id.clone(),
        task_type: task_type.clone(),
        task_name: payload.task_name.clone(),
        priority: payload.priority.clone(),
        config_override: payload.config_override.clone(),
    };

    match handlers::api_create_deployment_site_task(State(state), Json(request)).await {
        Ok(Json(value)) => {
            let site_label = value
                .get("site_label")
                .and_then(|v| v.as_str())
                .map(String::from);
            let task_id = value
                .get("task_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let task_name = payload
                .task_name
                .clone()
                .unwrap_or_else(|| format!("registry-{:?}", task_type));
            let config = payload
                .config_override
                .unwrap_or_default();
            let mut unified_task = TaskInfo::new_with_priority(
                task_name,
                task_type,
                config,
                payload.priority.unwrap_or_default(),
            );
            if let Some(tid) = &task_id {
                unified_task.id = tid.clone();
            }
            unified_task.site_id = Some(site_id);
            unified_task.site_label = site_label;
            admin_task_handlers::insert_task(unified_task);

            admin_response::ok("创建注册表任务成功", sanitize_task_response(value))
        }
        Err((status, Json(value))) => extract_proxy_error(status, value, "创建注册表任务失败"),
    }
}

fn sanitize_task_response(value: Value) -> Value {
    json!({
        "task_id": value.get("task_id").cloned().unwrap_or(Value::Null),
        "message": value
            .get("message")
            .cloned()
            .unwrap_or_else(|| Value::String("任务创建成功".to_string()))
    })
}

fn unwrap_item_or_value(value: Value) -> Value {
    value.get("item").cloned().unwrap_or(value)
}

fn extract_proxy_error(status: StatusCode, value: Value, fallback_message: &str) -> ApiResponse {
    let message = value
        .get("error")
        .and_then(Value::as_str)
        .or_else(|| value.get("message").and_then(Value::as_str))
        .unwrap_or(fallback_message);
    admin_response::response::<Value>(status, false, message, None)
}
