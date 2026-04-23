use axum::{
    extract::{Json, Path},
    middleware,
    response::IntoResponse,
    routing::{delete, get, post, put},
    Router,
};
use serde_json::json;

use crate::web_server::{
    admin_auth_handlers::admin_auth_middleware,
    admin_response::{self, ApiResponse},
    managed_project_sites as managed_sites,
    models::{
        AdminResourceSummary, CreateManagedSiteRequest, ManagedSiteLogsResponse,
        ManagedSiteRuntimeStatus, PreviewManagedSiteParsePlanRequest, UpdateManagedSiteRequest,
    },
};

pub fn create_admin_routes() -> Router {
    Router::new()
        .route("/api/admin/resources/summary", get(get_resource_summary))
        .route("/api/admin/sites", get(list_sites).post(create_site))
        .route(
            "/api/admin/sites/preview-parse-plan",
            post(preview_parse_plan),
        )
        .route(
            "/api/admin/sites/{id}",
            get(get_site).put(update_site).delete(delete_site),
        )
        .route("/api/admin/sites/{id}/parse", post(parse_site))
        .route("/api/admin/sites/{id}/start", post(start_site))
        .route("/api/admin/sites/{id}/stop", post(stop_site))
        .route("/api/admin/sites/{id}/runtime", get(get_site_runtime))
        .route("/api/admin/sites/{id}/logs", get(get_site_logs))
        .layer(middleware::from_fn(admin_auth_middleware))
}

pub async fn list_sites() -> impl IntoResponse {
    match managed_sites::list_sites() {
        Ok(sites) => admin_response::ok("获取站点列表成功", sites),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

pub async fn get_resource_summary() -> impl IntoResponse {
    let summary = managed_sites::resource_summary().unwrap_or_else(|err| AdminResourceSummary {
        updated_at: chrono::Utc::now().to_rfc3339(),
        message: Some(err.to_string()),
        ..AdminResourceSummary::default()
    });
    admin_response::ok("获取资源摘要成功", summary)
}

pub async fn create_site(Json(payload): Json<CreateManagedSiteRequest>) -> impl IntoResponse {
    match managed_sites::create_site(payload) {
        Ok(site) => admin_response::response(
            axum::http::StatusCode::CREATED,
            true,
            "创建站点成功",
            Some(site),
        ),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

pub async fn preview_parse_plan(
    Json(payload): Json<PreviewManagedSiteParsePlanRequest>,
) -> impl IntoResponse {
    match managed_sites::preview_parse_plan(payload) {
        Ok(plan) => admin_response::ok("获取解析预览成功", plan),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

pub async fn get_site(Path(site_id): Path<String>) -> impl IntoResponse {
    match managed_sites::get_site(&site_id) {
        Ok(Some(site)) => admin_response::ok("获取站点详情成功", site),
        Ok(None) => admin_response::not_found(format!("站点不存在: {}", site_id)),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

pub async fn update_site(
    Path(site_id): Path<String>,
    Json(payload): Json<UpdateManagedSiteRequest>,
) -> impl IntoResponse {
    match managed_sites::update_site(&site_id, payload) {
        Ok(site) => admin_response::ok("更新站点成功", site),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

pub async fn delete_site(Path(site_id): Path<String>) -> impl IntoResponse {
    match managed_sites::delete_site(&site_id) {
        Ok(true) => admin_response::ok(
            "删除站点成功",
            json!({ "site_id": site_id, "deleted": true }),
        ),
        Ok(false) => admin_response::not_found(format!("站点不存在: {}", site_id)),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

pub async fn parse_site(Path(site_id): Path<String>) -> impl IntoResponse {
    match managed_sites::parse_site(site_id.clone()).await {
        Ok(()) => admin_response::accepted(
            "已提交解析任务",
            json!({ "site_id": site_id, "action": "parse" }),
        ),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

pub async fn start_site(Path(site_id): Path<String>) -> impl IntoResponse {
    match managed_sites::start_site(site_id.clone()).await {
        Ok(()) => admin_response::accepted(
            "已提交启动任务",
            json!({ "site_id": site_id, "action": "start" }),
        ),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

pub async fn stop_site(Path(site_id): Path<String>) -> impl IntoResponse {
    match managed_sites::stop_site(&site_id).await {
        Ok(result) if result.conflict => admin_response::conflict(format!(
            "受管进程已停止，但端口仍被外部进程占用: web={:?} db={:?}",
            result.web_conflict_pids, result.db_conflict_pids
        )),
        Ok(result) => admin_response::ok("停止站点成功", result.site),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

pub async fn get_site_runtime(Path(site_id): Path<String>) -> impl IntoResponse {
    match managed_sites::runtime_status(&site_id) {
        Ok(runtime) => runtime_ok(runtime),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

pub async fn get_site_logs(Path(site_id): Path<String>) -> impl IntoResponse {
    match managed_sites::logs(&site_id) {
        Ok(logs) => logs_ok(logs),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

fn runtime_ok(runtime: ManagedSiteRuntimeStatus) -> ApiResponse {
    admin_response::ok("获取站点运行状态成功", runtime)
}

fn logs_ok(logs: ManagedSiteLogsResponse) -> ApiResponse {
    admin_response::ok("获取站点日志成功", logs)
}
