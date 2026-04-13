use axum::{
    Router,
    extract::{Json, Path},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::web_server::{
    AppState,
    managed_project_sites as managed_sites,
    models::{
        CreateManagedSiteRequest, ManagedSiteLogsResponse, ManagedSiteRuntimeStatus,
        UpdateManagedSiteRequest,
    },
};

type ApiResponse = (StatusCode, Json<Value>);

pub fn create_admin_routes() -> Router<AppState> {
    Router::new()
        .route("/api/admin/sites", get(list_sites).post(create_site))
        .route(
            "/api/admin/sites/{id}",
            get(get_site).put(update_site).delete(delete_site),
        )
        .route("/api/admin/sites/{id}/parse", post(parse_site))
        .route("/api/admin/sites/{id}/start", post(start_site))
        .route("/api/admin/sites/{id}/stop", post(stop_site))
        .route("/api/admin/sites/{id}/runtime", get(get_site_runtime))
        .route("/api/admin/sites/{id}/logs", get(get_site_logs))
}

pub async fn list_sites() -> impl IntoResponse {
    match managed_sites::list_sites() {
        Ok(sites) => ok("获取站点列表成功", sites),
        Err(err) => managed_error(err.to_string()),
    }
}

pub async fn create_site(Json(payload): Json<CreateManagedSiteRequest>) -> impl IntoResponse {
    match managed_sites::create_site(payload) {
        Ok(site) => response(StatusCode::CREATED, true, "创建站点成功", Some(site)),
        Err(err) => managed_error(err.to_string()),
    }
}

pub async fn get_site(Path(site_id): Path<String>) -> impl IntoResponse {
    match managed_sites::get_site(&site_id) {
        Ok(Some(site)) => ok("获取站点详情成功", site),
        Ok(None) => not_found(format!("站点不存在: {}", site_id)),
        Err(err) => managed_error(err.to_string()),
    }
}

pub async fn update_site(
    Path(site_id): Path<String>,
    Json(payload): Json<UpdateManagedSiteRequest>,
) -> impl IntoResponse {
    match managed_sites::update_site(&site_id, payload) {
        Ok(site) => ok("更新站点成功", site),
        Err(err) => managed_error(err.to_string()),
    }
}

pub async fn delete_site(Path(site_id): Path<String>) -> impl IntoResponse {
    match managed_sites::delete_site(&site_id) {
        Ok(true) => ok(
            "删除站点成功",
            json!({
                "site_id": site_id,
                "deleted": true
            }),
        ),
        Ok(false) => not_found(format!("站点不存在: {}", site_id)),
        Err(err) => managed_error(err.to_string()),
    }
}

pub async fn parse_site(Path(site_id): Path<String>) -> impl IntoResponse {
    match managed_sites::parse_site(site_id.clone()).await {
        Ok(()) => accepted(
            "已提交解析任务",
            json!({
                "site_id": site_id,
                "action": "parse"
            }),
        ),
        Err(err) => managed_error(err.to_string()),
    }
}

pub async fn start_site(Path(site_id): Path<String>) -> impl IntoResponse {
    match managed_sites::start_site(site_id.clone()).await {
        Ok(()) => accepted(
            "已提交启动任务",
            json!({
                "site_id": site_id,
                "action": "start"
            }),
        ),
        Err(err) => managed_error(err.to_string()),
    }
}

pub async fn stop_site(Path(site_id): Path<String>) -> impl IntoResponse {
    match managed_sites::stop_site(&site_id).await {
        Ok(site) => ok("停止站点成功", site),
        Err(err) => managed_error(err.to_string()),
    }
}

pub async fn get_site_runtime(Path(site_id): Path<String>) -> impl IntoResponse {
    match managed_sites::runtime_status(&site_id) {
        Ok(runtime) => runtime_ok(runtime),
        Err(err) => managed_error(err.to_string()),
    }
}

pub async fn get_site_logs(Path(site_id): Path<String>) -> impl IntoResponse {
    match managed_sites::logs(&site_id) {
        Ok(logs) => logs_ok(logs),
        Err(err) => managed_error(err.to_string()),
    }
}

fn runtime_ok(runtime: ManagedSiteRuntimeStatus) -> ApiResponse {
    ok("获取站点运行状态成功", runtime)
}

fn logs_ok(logs: ManagedSiteLogsResponse) -> ApiResponse {
    ok("获取站点日志成功", logs)
}

fn ok<T>(message: impl Into<String>, data: T) -> ApiResponse
where
    T: Serialize,
{
    response(StatusCode::OK, true, message, Some(data))
}

fn accepted<T>(message: impl Into<String>, data: T) -> ApiResponse
where
    T: Serialize,
{
    response(StatusCode::ACCEPTED, true, message, Some(data))
}

fn not_found(message: impl Into<String>) -> ApiResponse {
    response::<Value>(StatusCode::NOT_FOUND, false, message, None)
}

fn managed_error(message: String) -> ApiResponse {
    response::<Value>(classify_error_status(&message), false, message, None)
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

fn classify_error_status(message: &str) -> StatusCode {
    if message.contains("站点不存在") {
        StatusCode::NOT_FOUND
    } else if message.contains("不能为空") || message.contains("必须大于") {
        StatusCode::BAD_REQUEST
    } else if message.contains("运行中")
        || message.contains("正在运行")
        || message.contains("已在运行中")
        || message.contains("不能删除")
        || message.contains("不能修改配置")
        || message.contains("已被站点")
        || message.contains("已被当前机器")
        || message.contains("已被占用")
        || message.contains("端口")
    {
        StatusCode::CONFLICT
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}
