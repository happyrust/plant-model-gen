use axum::{
    Json, Router,
    extract::{Path, Query},
    http::StatusCode,
    middleware,
    response::IntoResponse,
    routing::{delete, get, post, put},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::web_server::{
    AppState, admin_auth_handlers,
    admin_response::{self, ApiResponse},
    admin_task_handlers, handlers,
    models::{
        DatabaseConfig, DeploymentSiteCreateRequest, DeploymentSiteImportRequest,
        DeploymentSiteQuery, DeploymentSiteUpdateRequest,
    },
};

const REDACTED_SECRET: &str = "********";

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
        .layer(middleware::from_fn(
            admin_auth_handlers::admin_auth_middleware,
        ))
}

async fn list_sites(Query(params): Query<DeploymentSiteQuery>) -> impl IntoResponse {
    match handlers::api_get_deployment_sites(Query(params)).await {
        Ok(Json(value)) => {
            admin_response::ok("获取注册表站点列表成功", redact_registry_value(value))
        }
        Err(status) => {
            admin_response::response::<Value>(status, false, "获取注册表站点列表失败", None)
        }
    }
}

async fn create_site(Json(payload): Json<DeploymentSiteCreateRequest>) -> impl IntoResponse {
    match handlers::api_create_deployment_site(Json(payload)).await {
        Ok(Json(value)) => admin_response::accepted(
            "创建注册表站点成功",
            redact_registry_value(unwrap_item_or_value(value)),
        ),
        Err((status, Json(value))) => extract_proxy_error(status, value, "创建注册表站点失败"),
    }
}

async fn get_site(Path(site_id): Path<String>) -> impl IntoResponse {
    match handlers::api_get_deployment_site(Path(site_id)).await {
        Ok(Json(value)) => {
            admin_response::ok("获取注册表站点详情成功", redact_registry_value(value))
        }
        Err(status) => {
            admin_response::response::<Value>(status, false, "获取注册表站点详情失败", None)
        }
    }
}

async fn update_site(
    Path(site_id): Path<String>,
    Json(payload): Json<DeploymentSiteUpdateRequest>,
) -> impl IntoResponse {
    match handlers::api_update_deployment_site(Path(site_id), Json(payload)).await {
        Ok(Json(value)) => admin_response::ok(
            "更新注册表站点成功",
            redact_registry_value(unwrap_item_or_value(value)),
        ),
        Err((status, Json(value))) => extract_proxy_error(status, value, "更新注册表站点失败"),
    }
}

async fn delete_site(Path(site_id): Path<String>) -> impl IntoResponse {
    match handlers::api_delete_deployment_site(Path(site_id.clone())).await {
        Ok(_) => admin_response::ok(
            "已移除注册表记录，未删除项目或运行数据目录",
            json!({"site_id": site_id, "deleted": true, "removed_registry_record": true, "deleted_runtime_data": false}),
        ),
        Err(status) => admin_response::response::<Value>(status, false, "删除注册表站点失败", None),
    }
}

async fn import_site_from_dboption(
    payload: Option<Json<DeploymentSiteImportRequest>>,
) -> impl IntoResponse {
    match handlers::api_import_deployment_site_from_dboption(payload).await {
        Ok(Json(value)) => admin_response::accepted(
            "导入注册表站点成功",
            redact_registry_value(unwrap_item_or_value(value)),
        ),
        Err((status, Json(value))) => extract_proxy_error(status, value, "导入注册表站点失败"),
    }
}

async fn healthcheck_site(Path(site_id): Path<String>) -> impl IntoResponse {
    match handlers::api_healthcheck_deployment_site_post(Path(site_id), None).await {
        Ok(Json(value)) => admin_response::ok("站点健康检查完成", redact_registry_value(value)),
        Err((status, Json(value))) => extract_proxy_error(status, value, "站点健康检查失败"),
    }
}

async fn export_site_config(Path(site_id): Path<String>) -> impl IntoResponse {
    match handlers::api_export_deployment_site_config(Path(site_id)).await {
        Ok(Json(value)) => admin_response::ok("导出站点配置成功", redact_registry_value(value)),
        Err(status) => admin_response::response::<Value>(status, false, "导出站点配置失败", None),
    }
}

async fn create_site_task(
    Path(site_id): Path<String>,
    Json(payload): Json<AdminRegistryTaskRequest>,
) -> impl IntoResponse {
    let site = match crate::web_server::site_registry::get_site(&site_id) {
        Ok(Some(site)) => site,
        Ok(None) => return admin_response::not_found(format!("站点不存在: {}", site_id)),
        Err(err) => return admin_response::server_error(format!("站点查询失败: {}", err)),
    };
    let task_type = payload
        .task_type
        .unwrap_or(crate::web_server::models::TaskType::ParsePdmsData);
    let task_name = payload
        .task_name
        .unwrap_or_else(|| format!("{} - {:?}", site.name, task_type));
    let config = payload.config_override.unwrap_or(site.config);

    match admin_task_handlers::create_and_dispatch_site_task(
        site.site_id.clone(),
        task_name,
        task_type,
        payload.priority.unwrap_or_default(),
        config,
    ) {
        Ok(task) => admin_response::accepted(
            "创建注册表任务成功",
            sanitize_task_response(json!({
                "task_id": task.id,
                "message": "任务已提交，等待站点状态更新",
            })),
        ),
        Err(message) => admin_response::bad_request(message),
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

fn redact_registry_value(mut value: Value) -> Value {
    redact_secret_fields(&mut value);
    value
}

fn redact_secret_fields(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, nested) in object.iter_mut() {
                if matches!(
                    key.as_str(),
                    "db_password" | "password" | "surreal_password"
                ) {
                    *nested = Value::String(REDACTED_SECRET.to_string());
                } else {
                    redact_secret_fields(nested);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_secret_fields(item);
            }
        }
        _ => {}
    }
}

fn extract_proxy_error(status: StatusCode, value: Value, fallback_message: &str) -> ApiResponse {
    let message = value
        .get("error")
        .and_then(Value::as_str)
        .or_else(|| value.get("message").and_then(Value::as_str))
        .unwrap_or(fallback_message);
    admin_response::response::<Value>(status, false, message, None)
}
