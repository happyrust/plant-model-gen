use axum::{
    body::Body,
    extract::{Json, Path, Query},
    http::{header, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::AsyncReadExt;

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
        .route("/api/admin/app-config", get(get_app_config))
        .route("/api/admin/ports/check", get(check_port))
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
        .route("/api/admin/sites/{id}/restart", post(restart_site))
        .route("/api/admin/sites/{id}/runtime", get(get_site_runtime))
        .route("/api/admin/sites/{id}/logs", get(get_site_logs))
        .route(
            "/api/admin/sites/{id}/logs/{kind}",
            get(get_site_log_kind),
        )
        .route(
            "/api/admin/sites/{id}/logs/{kind}/download",
            get(download_site_log),
        )
        .layer(middleware::from_fn(admin_auth_middleware))
}

/// Admin 前端在启动时一次性拉取的"运行期可配置"项。
///
/// 取舍：不把这些字段做进每个站点的 DB 行里（与具体站点解耦），也不做进前端
/// Vite build-time env（避免改基础址必须重出前端构建），而是由 web_server 进程
/// 从环境变量解析后按需发布给前端。
#[derive(Debug, Serialize, Default)]
pub struct AdminAppConfig {
    /// Viewer 三维看图页面的基础 URL，形如 `https://viewer.example.com` 或
    /// `http://localhost:3101`。空值 / None 表示未配置，前端应隐藏 Viewer 按钮。
    ///
    /// 来源：`AIOS_VIEWER_BASE_URL` 环境变量（优先级 1）。未来若接入 admin
    /// 配置项或 DB 存储，扩展此 resolver 即可，不需要改动前端。
    pub viewer_base_url: Option<String>,
}

fn resolve_admin_app_config() -> AdminAppConfig {
    let viewer_base_url = std::env::var("AIOS_VIEWER_BASE_URL")
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty());
    AdminAppConfig { viewer_base_url }
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

pub async fn get_app_config() -> impl IntoResponse {
    admin_response::ok("获取应用配置成功", resolve_admin_app_config())
}

/// 端口占用预检（D4 / Sprint D · 修 G12）
///
/// 给前端 `SiteDrawer` 的端口字段 onBlur 校验用，**仅在 admin 鉴权后** 暴露。
/// 复用 `managed_project_sites::process_ids_on_port` 探测 PID 列表，规避
/// "前端啥都没说，提交才报冲突"的尴尬期。
///
/// 行为：
/// - `port == 0` 视为非法，返回 400-style error
/// - 端口空闲：`{ in_use: false, pids: [] }`
/// - 端口占用：`{ in_use: true, pids: [...] }`
/// - host 仅作 echo，不参与判定（同一进程 bind 0.0.0.0 会与 127.0.0.1 冲突）
#[derive(Debug, Deserialize)]
pub struct PortCheckQuery {
    pub port: u16,
    #[serde(default)]
    pub host: Option<String>,
}

pub async fn check_port(Query(params): Query<PortCheckQuery>) -> impl IntoResponse {
    if params.port == 0 {
        return admin_response::managed_error("port 参数不能为 0".to_string());
    }
    let pids = match crate::web_server::managed_project_sites::process_ids_on_port(params.port)
        .await
    {
        Ok(pids) => pids,
        Err(err) => return admin_response::managed_error(err.to_string()),
    };
    admin_response::ok(
        "端口探测完成",
        json!({
            "port": params.port,
            "host": params.host,
            "in_use": !pids.is_empty(),
            "pids": pids,
        }),
    )
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

/// 重启站点（C6 / Sprint C · 修 G10）
///
/// 提交一个 stop → start 的串联任务并立即返回 202 Accepted；后端实际状态
/// 翻转通过 `/api/admin/sites/{id}/runtime` 轮询或 SSE（Sprint D · D1）感知。
pub async fn restart_site(Path(site_id): Path<String>) -> impl IntoResponse {
    match managed_sites::restart_site(&site_id).await {
        Ok(()) => admin_response::accepted(
            "已提交重启任务",
            json!({ "site_id": site_id, "action": "restart" }),
        ),
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

/// 单条日志类别的分页尾部查询（D5 / Sprint D · 修 G13）
///
/// `GET /api/admin/sites/{id}/logs/{kind}?limit=N`
/// - `kind` ∈ parse / db / web
/// - `limit` 默认 200，上限 5000；超出会被钳制
/// - 响应包含 `total_lines` 与 `truncated` 让前端决定是否展示「加载更多」
#[derive(Debug, Deserialize)]
pub struct LogsTailQuery {
    #[serde(default)]
    pub limit: Option<usize>,
}

pub async fn get_site_log_kind(
    Path((site_id, kind)): Path<(String, String)>,
    Query(params): Query<LogsTailQuery>,
) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(200);
    match managed_sites::tail_log(&site_id, &kind, limit) {
        Ok(payload) => admin_response::ok("获取日志尾部成功", payload),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

/// 单条日志类别的全量下载（D5）
///
/// `GET /api/admin/sites/{id}/logs/{kind}/download`
/// - 直接以 `text/plain; charset=utf-8` + `Content-Disposition: attachment` 响应
/// - 文件名格式 `<site_id>-<kind>-<UTC>.log`，便于一次性归档
/// - 大文件场景：当前一次性读入内存；后续若需流式可改 axum::body::Body::from_stream
pub async fn download_site_log(Path((site_id, kind)): Path<(String, String)>) -> Response {
    let path = match managed_sites::full_log_path(&site_id, &kind) {
        Ok(p) => p,
        Err(err) => {
            return admin_response::managed_error(err.to_string()).into_response();
        }
    };
    let mut file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                format!("日志文件不存在: {}", path.display()),
            )
                .into_response();
        }
    };
    let mut buf = Vec::new();
    if let Err(err) = file.read_to_end(&mut buf).await {
        return admin_response::managed_error(format!("读取日志文件失败: {}", err))
            .into_response();
    }
    let filename = format!(
        "{}-{}-{}.log",
        site_id,
        kind,
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ"),
    );
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", filename),
        )
        .body(Body::from(buf))
        .unwrap_or_else(|_| {
            (StatusCode::INTERNAL_SERVER_ERROR, "构造下载响应失败").into_response()
        })
}

fn runtime_ok(runtime: ManagedSiteRuntimeStatus) -> ApiResponse {
    admin_response::ok("获取站点运行状态成功", runtime)
}

fn logs_ok(logs: ManagedSiteLogsResponse) -> ApiResponse {
    admin_response::ok("获取站点日志成功", logs)
}
