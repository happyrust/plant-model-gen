use aios_core::SurrealQueryExt;
use axum::{
    Router,
    body::Body,
    extract::{Json, Path, Query},
    http::{StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;
use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;
use surrealdb::types::{Number as SurrealNumber, ToSql, Value as SurrealValue};
use surrealdb::{Connection, Surreal};
use tokio::io::AsyncReadExt;
use uuid::Uuid;

use crate::web_server::{
    admin_auth_handlers::admin_auth_middleware,
    admin_response::{self, ApiResponse},
    admin_task_handlers, managed_project_sites as managed_sites,
    models::{
        AdminResourceSummary, AppendManagedSiteDbFileRequest, CreateManagedSiteRequest,
        DatabaseConfig, ManagedProjectSite, ManagedRemoteDeployRequest, ManagedRemoteTargetRequest,
        ManagedSiteDbMode, ManagedSiteLogsResponse, ManagedSiteReconcileRequest,
        ManagedSiteRuntimeStatus, ManagedSiteStatus, PreviewManagedSiteParsePlanRequest,
        QuickDeployTestRequest, SiteProject, TaskPriority, TaskType, UpdateManagedSiteRequest,
    },
    parse_sidecar_client,
};

pub fn create_admin_routes() -> Router {
    Router::new()
        .route("/api/admin/resources/summary", get(get_resource_summary))
        .route("/api/admin/app-config", get(get_app_config))
        .route("/api/admin/data-browser/tables", get(list_data_tables))
        .route(
            "/api/admin/data-browser/tables/{table}/records",
            get(list_data_table_records),
        )
        .route(
            "/api/admin/sites/{id}/data-browser/tables",
            get(list_site_data_tables),
        )
        .route(
            "/api/admin/sites/{id}/data-browser/connection",
            get(get_site_data_browser_connection),
        )
        .route(
            "/api/admin/sites/{id}/data-browser/tables/{table}/records",
            get(list_site_data_table_records),
        )
        .route("/api/admin/ports/check", get(check_port))
        .route("/api/admin/ports/kill", post(kill_port))
        .route("/api/admin/projects/scan", get(scan_projects))
        .route(
            "/api/admin/remote-targets",
            get(list_remote_targets).post(upsert_remote_target),
        )
        .route("/api/admin/sites", get(list_sites).post(create_site))
        .route("/api/admin/sites/quick-deploy", post(quick_deploy_site))
        .route(
            "/api/admin/sites/preview-parse-plan",
            post(preview_parse_plan),
        )
        .route(
            "/api/admin/sites/{id}",
            get(get_site).put(update_site).delete(delete_site),
        )
        .route("/api/admin/sites/{id}/preflight", post(preflight_site))
        .route(
            "/api/admin/sites/{id}/remote-preflight",
            post(remote_preflight_site),
        )
        .route(
            "/api/admin/sites/{id}/remote-prepare",
            post(remote_prepare_site),
        )
        .route(
            "/api/admin/sites/{id}/remote-deploy",
            get(get_remote_deploy_status).post(remote_deploy_site),
        )
        .route(
            "/api/admin/sites/{id}/remote-deploy/status",
            get(get_remote_deploy_status),
        )
        .route(
            "/api/admin/sites/{id}/remote-agent-status",
            get(get_remote_agent_status),
        )
        .route("/api/admin/sites/{id}/parse", post(parse_site))
        .route(
            "/api/admin/sites/{id}/append-dbfile",
            post(append_site_dbfile),
        )
        .route(
            "/api/admin/sites/{id}/db-index/rebuild",
            post(rebuild_site_db_index),
        )
        .route("/api/admin/sites/{id}/generate", post(generate_site))
        .route("/api/admin/sites/{id}/deploy", post(deploy_site))
        .route("/api/admin/sites/{id}/redeploy", post(redeploy_site))
        .route("/api/admin/sites/{id}/start", post(start_site))
        .route("/api/admin/sites/{id}/stop", post(stop_site))
        .route("/api/admin/sites/{id}/restart", post(restart_site))
        .route("/api/admin/sites/{id}/runtime", get(get_site_runtime))
        .route("/api/admin/sites/{id}/logs", get(get_site_logs))
        .route(
            "/api/admin/sites/{id}/deploy-validation",
            get(get_site_deploy_validation).post(refresh_site_deploy_validation),
        )
        .route("/api/admin/sites/{id}/reconcile", post(reconcile_site))
        .route("/api/admin/sites/{id}/logs/{kind}", get(get_site_log_kind))
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
    /// `http://localhost:3101`。未显式配置时默认推断为 `http://<本机 IPv4>`。
    ///
    /// 来源：`AIOS_VIEWER_BASE_URL` 环境变量（优先级 1）；未设置时用本机 IPv4
    /// 拼出默认入口。默认本机 IP 不代表 Nginx 已接管 80 端口，前端需要结合
    /// `viewer_base_url_source` 和站点 `viewer_port` 拼出本机受管 Viewer URL。
    pub viewer_base_url: Option<String>,
    pub viewer_base_url_source: Option<&'static str>,
}

fn resolve_admin_app_config() -> AdminAppConfig {
    if let Some(viewer_base_url) = std::env::var("AIOS_VIEWER_BASE_URL")
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
    {
        return AdminAppConfig {
            viewer_base_url: Some(viewer_base_url),
            viewer_base_url_source: Some("env"),
        };
    }

    let viewer_base_url = super::get_local_ip_via_udp()
        .ok()
        .map(|ip| format!("http://{ip}"));
    let viewer_base_url_source = viewer_base_url.as_ref().map(|_| "local_ip");
    AdminAppConfig {
        viewer_base_url,
        viewer_base_url_source,
    }
}

fn submit_managed_site_task(
    site_id: &str,
    task_type: TaskType,
    action_label: &str,
) -> Result<String, String> {
    let site = managed_sites::get_site(site_id)
        .map_err(|err| err.to_string())?
        .ok_or_else(|| format!("站点不存在: {site_id}"))?;
    let mut config = DatabaseConfig::default();
    config.name = format!("{} - {}", site.project_name, action_label);
    let task = admin_task_handlers::create_and_dispatch_site_task(
        site.site_id.clone(),
        config.name.clone(),
        task_type,
        TaskPriority::Normal,
        config,
    )?;
    Ok(task.id)
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

#[derive(Debug, Deserialize)]
pub struct PortKillRequest {
    pub port: u16,
}

#[derive(Debug, Deserialize)]
pub struct DataBrowserRecordsQuery {
    #[serde(default)]
    pub page: Option<u32>,
    #[serde(default)]
    pub per_page: Option<u32>,
    #[serde(default)]
    pub sort: Option<String>,
    #[serde(default)]
    pub dir: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DataBrowserTable {
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct DataBrowserSiteContext {
    pub site_id: String,
    pub site_name: String,
    pub project_name: String,
    pub status: ManagedSiteStatus,
    pub runtime_db_mode: ManagedSiteDbMode,
    pub db_port: u16,
}

#[derive(Debug, Serialize)]
pub struct DataBrowserTablesResponse {
    pub tables: Vec<DataBrowserTable>,
    pub total: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<DataBrowserSiteContext>,
}

#[derive(Debug, Serialize)]
pub struct DataBrowserRecordsResponse {
    pub table: String,
    pub columns: Vec<String>,
    pub records: Vec<serde_json::Value>,
    pub total: usize,
    pub page: u32,
    pub per_page: u32,
    pub sort: String,
    pub dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<DataBrowserSiteContext>,
}

#[derive(Debug, Deserialize)]
pub struct DataBrowserConnectionQuery {
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DataBrowserConnectionMode {
    Reader,
    Editor,
}

impl DataBrowserConnectionMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Reader => "reader",
            Self::Editor => "editor",
        }
    }

    fn username(self) -> &'static str {
        match self {
            Self::Reader => "aios_browser_reader",
            Self::Editor => "aios_browser_editor",
        }
    }

    fn role(self) -> &'static str {
        match self {
            Self::Reader => "VIEWER",
            Self::Editor => "EDITOR",
        }
    }

    fn can_write(self) -> bool {
        matches!(self, Self::Editor)
    }
}

#[derive(Debug, Serialize)]
pub struct DataBrowserConnectionCredential {
    pub username: String,
    pub password: String,
    pub role: String,
    pub can_write: bool,
}

#[derive(Debug, Serialize)]
pub struct DataBrowserConnectionResponse {
    pub site: DataBrowserSiteContext,
    pub endpoint: String,
    pub namespace: String,
    pub database: String,
    pub mode: String,
    pub credential: DataBrowserConnectionCredential,
    pub local_only: bool,
}

struct NormalizedDataBrowserRecordsQuery {
    table: String,
    page: u32,
    per_page: u32,
    start: u32,
    sort: String,
    dir_sql: &'static str,
}

fn is_safe_surreal_ident(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn require_safe_surreal_ident(value: &str, label: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if is_safe_surreal_ident(trimmed) {
        Ok(trimmed.to_string())
    } else {
        Err(format!(
            "{label} 格式不正确，仅支持字母、数字、下划线且不能以数字开头: {trimmed}"
        ))
    }
}

fn normalize_data_browser_connection_mode(
    mode: Option<String>,
) -> Result<DataBrowserConnectionMode, String> {
    match mode
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("reader")
        .to_ascii_lowercase()
        .as_str()
    {
        "reader" | "read" => Ok(DataBrowserConnectionMode::Reader),
        "editor" | "write" => Ok(DataBrowserConnectionMode::Editor),
        value => Err(format!("mode 参数不支持: {value}，仅支持 reader 或 editor")),
    }
}

fn generate_data_browser_password(mode: DataBrowserConnectionMode) -> String {
    let prefix = if mode.can_write() {
        "AiosBrowserEditor"
    } else {
        "AiosBrowserReader"
    };
    format!("{prefix}{}", Uuid::new_v4().simple())
}

fn collect_record_columns(rows: &[serde_json::Value]) -> Vec<String> {
    let mut keys = BTreeSet::new();
    for row in rows {
        if let Some(object) = row.as_object() {
            for key in object.keys() {
                keys.insert(key.to_string());
            }
        }
    }
    let mut columns = Vec::new();
    if keys.remove("id") {
        columns.push("id".to_string());
    }
    columns.extend(keys);
    columns
}

fn surreal_value_to_json(value: SurrealValue) -> serde_json::Value {
    match value {
        SurrealValue::None | SurrealValue::Null => serde_json::Value::Null,
        SurrealValue::Bool(value) => serde_json::Value::Bool(value),
        SurrealValue::Number(SurrealNumber::Int(value)) => json!(value),
        SurrealValue::Number(SurrealNumber::Float(value)) if value.is_finite() => json!(value),
        SurrealValue::Number(value) => serde_json::Value::String(value.to_string()),
        SurrealValue::String(value) => serde_json::Value::String(value),
        SurrealValue::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(surreal_value_to_json).collect())
        }
        SurrealValue::Set(values) => {
            serde_json::Value::Array(values.into_iter().map(surreal_value_to_json).collect())
        }
        SurrealValue::Object(object) => serde_json::Value::Object(
            object
                .into_iter()
                .map(|(key, value)| (key, surreal_value_to_json(value)))
                .collect(),
        ),
        SurrealValue::RecordId(value) => serde_json::Value::String(value.to_sql()),
        SurrealValue::Datetime(value) => serde_json::Value::String(value.to_string()),
        SurrealValue::Duration(value) => serde_json::Value::String(value.to_string()),
        SurrealValue::Uuid(value) => serde_json::Value::String(value.to_string()),
        SurrealValue::Table(value) => serde_json::Value::String(value.to_string()),
        SurrealValue::Geometry(value) => serde_json::Value::String(value.to_string()),
        SurrealValue::Bytes(value) => serde_json::Value::String(value.to_string()),
        SurrealValue::File(value) => serde_json::Value::String(value.to_sql()),
        SurrealValue::Range(value) => serde_json::Value::String(value.to_sql()),
        SurrealValue::Regex(value) => serde_json::Value::String(value.to_string()),
    }
}

fn extract_info_for_db_tables(value: &serde_json::Value) -> Vec<String> {
    let info = value
        .as_array()
        .and_then(|rows| rows.first())
        .unwrap_or(value);
    let mut names = info
        .get("tables")
        .and_then(|tables| tables.as_object())
        .map(|tables| {
            tables
                .keys()
                .filter(|name| is_safe_surreal_ident(name))
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    names.sort_by_key(|name| name.to_ascii_lowercase());
    names
}

fn data_browser_site_context(site: &ManagedProjectSite) -> DataBrowserSiteContext {
    DataBrowserSiteContext {
        site_id: site.site_id.clone(),
        site_name: site.site_name.clone(),
        project_name: site.project_name.clone(),
        status: site.status.clone(),
        runtime_db_mode: site.runtime_db_mode,
        db_port: site.db_port,
    }
}

fn normalize_data_browser_records_query(
    table: String,
    params: DataBrowserRecordsQuery,
) -> Result<NormalizedDataBrowserRecordsQuery, String> {
    let table = require_safe_surreal_ident(&table, "table")?;
    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(25).clamp(1, 100);
    let start = (page - 1) * per_page;
    let sort = match params.sort.as_deref().unwrap_or("id") {
        value if value.trim().is_empty() => "id".to_string(),
        value => require_safe_surreal_ident(value, "sort")?,
    };
    let dir_sql = if matches!(params.dir.as_deref(), Some(value) if value.eq_ignore_ascii_case("desc"))
    {
        "DESC"
    } else {
        "ASC"
    };
    Ok(NormalizedDataBrowserRecordsQuery {
        table,
        page,
        per_page,
        start,
        sort,
        dir_sql,
    })
}

async fn query_data_browser_tables<C: Connection>(
    db: &Surreal<C>,
    site: Option<DataBrowserSiteContext>,
) -> Result<DataBrowserTablesResponse, String> {
    let mut resp = db
        .query_response("INFO FOR DB;")
        .await
        .map_err(|err| format!("读取 SurrealDB 表列表失败: {err}"))?;
    let info: Option<serde_json::Value> = resp.take(0).unwrap_or_default();
    let info = info.unwrap_or(serde_json::Value::Null);
    let names = extract_info_for_db_tables(&info);
    let tables = names
        .into_iter()
        .map(|name| DataBrowserTable { name })
        .collect::<Vec<_>>();
    Ok(DataBrowserTablesResponse {
        total: tables.len(),
        tables,
        site,
    })
}

async fn query_data_browser_records<C: Connection>(
    db: &Surreal<C>,
    query: NormalizedDataBrowserRecordsQuery,
    site: Option<DataBrowserSiteContext>,
) -> Result<DataBrowserRecordsResponse, String> {
    let table = query.table;
    let sql = format!(
        "SELECT *, id AS id FROM {table} ORDER BY {sort} {dir} LIMIT {per_page} START {start};",
        sort = query.sort,
        dir = query.dir_sql,
        per_page = query.per_page,
        start = query.start,
    );
    let count_sql = format!("SELECT count() AS total FROM {table} GROUP ALL;");

    let records = match db.query_response(&sql).await {
        Ok(mut resp) => match resp.take::<Vec<SurrealValue>>(0) {
            Ok(records) => records,
            Err(err) => {
                return Err(format!("解析表 {table} 记录失败: {err}"));
            }
        },
        Err(err) => {
            return Err(format!("读取表 {table} 记录失败: {err}"));
        }
    };
    let total = match db.query_response(&count_sql).await {
        Ok(mut resp) => {
            let rows: Vec<serde_json::Value> = resp.take(0).unwrap_or_default();
            rows.first()
                .and_then(|row| row.get("total"))
                .and_then(|value| value.as_u64())
                .unwrap_or(0) as usize
        }
        Err(_) => 0,
    };
    let records = records
        .into_iter()
        .map(surreal_value_to_json)
        .collect::<Vec<_>>();
    let columns = collect_record_columns(&records);

    Ok(DataBrowserRecordsResponse {
        table,
        columns,
        records,
        total,
        page: query.page,
        per_page: query.per_page,
        sort: query.sort,
        dir: query.dir_sql.to_ascii_lowercase(),
        site,
    })
}

async fn connect_site_data_browser_db(
    site_id: &str,
) -> Result<(ManagedProjectSite, Surreal<Client>, String), String> {
    let (site, db_user, db_password, db_name) =
        managed_sites::get_site_runtime_db_context(site_id).map_err(|err| err.to_string())?;
    if site.runtime_db_mode != ManagedSiteDbMode::Ws {
        return Err(format!(
            "站点 {} 当前 runtime_db_mode={}，数据浏览器第一版仅支持 ws 运行库",
            site.site_id,
            serde_json::to_value(site.runtime_db_mode)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_else(|| "unknown".to_string())
        ));
    }
    if site.status != ManagedSiteStatus::Running {
        return Err(format!(
            "站点 {} 当前状态为 {:?}，请先启动站点后再浏览数据",
            site.site_id, site.status
        ));
    }
    let address = format!("127.0.0.1:{}", site.db_port);
    let db = Surreal::new::<Ws>(address.as_str())
        .await
        .map_err(|err| format!("连接站点数据库 {address} 失败: {err}"))?;
    db.signin(Root {
        username: db_user,
        password: db_password,
    })
    .await
    .map_err(|err| format!("站点数据库认证失败: {err}"))?;
    aios_core::use_ns_db_compat(&db, &site.project_code.to_string(), &db_name)
        .await
        .map_err(|err| format!("切换站点数据库命名空间失败: {err}"))?;
    Ok((site, db, db_name))
}

async fn ensure_data_browser_user(
    db: &Surreal<Client>,
    mode: DataBrowserConnectionMode,
    password: &str,
) -> Result<(), String> {
    let sql = format!(
        "DEFINE USER OVERWRITE {username} ON DATABASE PASSWORD '{password}' ROLES {role} DURATION FOR TOKEN 1h FOR SESSION 1h COMMENT 'AIOS data browser {mode} user';",
        username = mode.username(),
        role = mode.role(),
        mode = mode.as_str(),
    );
    db.query_response(&sql)
        .await
        .map(|_| ())
        .map_err(|err| format!("创建/刷新站点数据浏览器 {} 用户失败: {err}", mode.as_str()))
}

pub async fn list_data_tables() -> impl IntoResponse {
    use aios_core::project_primary_db;

    match query_data_browser_tables(project_primary_db(), None).await {
        Ok(result) => admin_response::ok("获取数据表列表成功", result),
        Err(err) => admin_response::managed_error(err),
    }
}

pub async fn list_data_table_records(
    Path(table): Path<String>,
    Query(params): Query<DataBrowserRecordsQuery>,
) -> impl IntoResponse {
    use aios_core::project_primary_db;

    let query = match normalize_data_browser_records_query(table, params) {
        Ok(value) => value,
        Err(message) => return admin_response::bad_request(message),
    };
    match query_data_browser_records(project_primary_db(), query, None).await {
        Ok(result) => admin_response::ok("获取数据表记录成功", result),
        Err(err) => admin_response::managed_error(err),
    }
}

pub async fn list_site_data_tables(Path(site_id): Path<String>) -> impl IntoResponse {
    let (site, db, _) = match connect_site_data_browser_db(&site_id).await {
        Ok(value) => value,
        Err(err) => return admin_response::managed_error(err),
    };
    match query_data_browser_tables(&db, Some(data_browser_site_context(&site))).await {
        Ok(result) => admin_response::ok("获取站点数据表列表成功", result),
        Err(err) => admin_response::managed_error(err),
    }
}

pub async fn list_site_data_table_records(
    Path((site_id, table)): Path<(String, String)>,
    Query(params): Query<DataBrowserRecordsQuery>,
) -> impl IntoResponse {
    let query = match normalize_data_browser_records_query(table, params) {
        Ok(value) => value,
        Err(message) => return admin_response::bad_request(message),
    };
    let (site, db, _) = match connect_site_data_browser_db(&site_id).await {
        Ok(value) => value,
        Err(err) => return admin_response::managed_error(err),
    };
    match query_data_browser_records(&db, query, Some(data_browser_site_context(&site))).await {
        Ok(result) => admin_response::ok("获取站点数据表记录成功", result),
        Err(err) => admin_response::managed_error(err),
    }
}

pub async fn get_site_data_browser_connection(
    Path(site_id): Path<String>,
    Query(params): Query<DataBrowserConnectionQuery>,
) -> impl IntoResponse {
    let mode = match normalize_data_browser_connection_mode(params.mode) {
        Ok(mode) => mode,
        Err(message) => return admin_response::bad_request(message),
    };
    let (site, db, database) = match connect_site_data_browser_db(&site_id).await {
        Ok(value) => value,
        Err(err) => return admin_response::managed_error(err),
    };
    let password = generate_data_browser_password(mode);
    if let Err(err) = ensure_data_browser_user(&db, mode, &password).await {
        return admin_response::managed_error(err);
    }

    let response = DataBrowserConnectionResponse {
        site: data_browser_site_context(&site),
        endpoint: format!("ws://127.0.0.1:{}/rpc", site.db_port),
        namespace: site.project_code.to_string(),
        database,
        mode: mode.as_str().to_string(),
        credential: DataBrowserConnectionCredential {
            username: mode.username().to_string(),
            password,
            role: mode.role().to_string(),
            can_write: mode.can_write(),
        },
        local_only: true,
    };
    admin_response::ok("获取站点数据浏览器连接上下文成功", response)
}

pub async fn check_port(Query(params): Query<PortCheckQuery>) -> impl IntoResponse {
    if params.port == 0 {
        return admin_response::managed_error("port 参数不能为 0".to_string());
    }
    let pids =
        match crate::web_server::managed_project_sites::process_ids_on_port(params.port).await {
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

pub async fn kill_port(Json(payload): Json<PortKillRequest>) -> impl IntoResponse {
    if payload.port == 0 {
        return admin_response::managed_error("port 参数不能为 0".to_string());
    }
    match managed_sites::kill_processes_on_port(payload.port).await {
        Ok((killed_pids, remaining_pids)) => admin_response::ok(
            "端口清理完成",
            json!({
                "port": payload.port,
                "killed_pids": killed_pids,
                "remaining_pids": remaining_pids,
                "released": remaining_pids.is_empty(),
            }),
        ),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

/// Phase 3：工程扫描。给一个根路径，自动发现候选工程（读 db 文件头），
/// 推断 Design/Library 角色、建议主工程，并预标跨工程 dbnum 冲突。
///
/// 复用 `managed_project_sites::scan_projects_under_root`（白名单 canonicalize +
/// 扫描 + 角色推断 + 冲突标注）。**仅在 admin 鉴权后** 暴露。
#[derive(Debug, Deserialize)]
pub struct ProjectScanQuery {
    pub root: String,
}

pub async fn scan_projects(Query(params): Query<ProjectScanQuery>) -> impl IntoResponse {
    let root = params.root.trim().to_string();
    if root.is_empty() {
        return admin_response::managed_error("root 参数不能为空".to_string());
    }
    match parse_sidecar_client::scan_projects(&root).await {
        Ok(result) => admin_response::ok("工程扫描完成", result),
        Err(err) => admin_response::response(err.status, false, err.message, Some(err.body)),
    }
}

fn project_roots_from_parts(projects: &[SiteProject], fallback_project_path: &str) -> Vec<String> {
    let roots = projects
        .iter()
        .map(|project| project.path.trim())
        .filter(|path| !path.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if roots.is_empty() {
        let fallback = fallback_project_path.trim();
        if fallback.is_empty() {
            Vec::new()
        } else {
            vec![fallback.to_string()]
        }
    } else {
        roots
    }
}

async fn resolve_db_files_to_nums(
    project_roots: Vec<String>,
    db_files: &[String],
) -> Result<Vec<u32>, parse_sidecar_client::SidecarProxyError> {
    let mut dbnums = Vec::new();
    for db_file in db_files
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        let resolved =
            parse_sidecar_client::resolve_db_file(project_roots.clone(), db_file.to_string())
                .await?;
        dbnums.push(resolved.dbnum);
    }
    Ok(dbnums)
}

async fn resolve_create_site_db_files(
    payload: &mut CreateManagedSiteRequest,
) -> Result<(), parse_sidecar_client::SidecarProxyError> {
    let roots = project_roots_from_parts(&payload.projects, &payload.project_path);
    if !payload.manual_db_files.is_empty() {
        let resolved = resolve_db_files_to_nums(roots.clone(), &payload.manual_db_files).await?;
        payload.manual_db_nums.extend(resolved);
        payload.manual_db_files.clear();
    }
    if !payload.generate_db_files.is_empty() {
        let resolved = resolve_db_files_to_nums(roots, &payload.generate_db_files).await?;
        payload.generate_db_nums.extend(resolved);
        payload.generate_db_files.clear();
    }
    Ok(())
}

async fn resolve_update_site_db_files(
    site_id: &str,
    payload: &mut UpdateManagedSiteRequest,
) -> Result<(), parse_sidecar_client::SidecarProxyError> {
    if payload.manual_db_files.is_empty() && payload.generate_db_files.is_empty() {
        return Ok(());
    }
    let site = managed_sites::get_site(site_id)
        .map_err(|err| sidecar_proxy_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or_else(|| {
            sidecar_proxy_error(StatusCode::NOT_FOUND, format!("站点不存在: {site_id}"))
        })?;
    let fallback_project_path = payload
        .project_path
        .as_deref()
        .unwrap_or(&site.project_path);
    let roots = payload
        .projects
        .as_deref()
        .map(|projects| project_roots_from_parts(projects, fallback_project_path))
        .unwrap_or_else(|| project_roots_from_parts(&site.projects, fallback_project_path));

    if !payload.manual_db_files.is_empty() {
        let resolved = resolve_db_files_to_nums(roots.clone(), &payload.manual_db_files).await?;
        let mut values = payload
            .manual_db_nums
            .take()
            .unwrap_or_else(|| site.manual_db_nums.clone());
        values.extend(resolved);
        payload.manual_db_nums = Some(values);
        payload.manual_db_files.clear();
    }
    if !payload.generate_db_files.is_empty() {
        let resolved = resolve_db_files_to_nums(roots, &payload.generate_db_files).await?;
        let mut values = payload
            .generate_db_nums
            .take()
            .unwrap_or_else(|| site.generate_db_nums.clone());
        values.extend(resolved);
        payload.generate_db_nums = Some(values);
        payload.generate_db_files.clear();
    }
    Ok(())
}

fn sidecar_proxy_error(
    status: StatusCode,
    message: String,
) -> parse_sidecar_client::SidecarProxyError {
    parse_sidecar_client::SidecarProxyError {
        status,
        message: message.clone(),
        body: json!({
            "success": false,
            "message": message,
        }),
    }
}

pub async fn create_site(Json(mut payload): Json<CreateManagedSiteRequest>) -> impl IntoResponse {
    let auto_deploy = payload.auto_deploy;
    if let Err(err) = resolve_create_site_db_files(&mut payload).await {
        return admin_response::response(err.status, false, err.message, Some(err.body));
    }
    match managed_sites::create_site(payload) {
        Ok(site) => {
            let mut response = serde_json::to_value(&site).unwrap_or_else(|_| json!({}));
            let mut deployment_submitted = false;
            let mut deployment_error = None;
            let mut deployment_task_id = None;
            if auto_deploy {
                match submit_managed_site_task(
                    &site.site_id,
                    TaskType::DeployManagedSite,
                    "完整部署",
                ) {
                    Ok(task_id) => {
                        deployment_submitted = true;
                        deployment_task_id = Some(task_id);
                    }
                    Err(err) => {
                        deployment_error = Some(err);
                    }
                }
                if let Ok(Some(updated)) = managed_sites::get_site(&site.site_id) {
                    response = serde_json::to_value(updated).unwrap_or(response);
                }
            }
            if let Some(obj) = response.as_object_mut() {
                obj.insert("auto_deploy".to_string(), json!(auto_deploy));
                obj.insert(
                    "deployment_submitted".to_string(),
                    json!(deployment_submitted),
                );
                obj.insert("deployment_error".to_string(), json!(deployment_error));
                obj.insert("deployment_task_id".to_string(), json!(deployment_task_id));
            }
            let message = if auto_deploy && deployment_submitted {
                "创建站点成功，已提交完整部署任务"
            } else if auto_deploy {
                "创建站点成功，但提交完整部署任务失败"
            } else {
                "创建站点成功"
            };
            admin_response::response(
                axum::http::StatusCode::CREATED,
                true,
                message,
                Some(response),
            )
        }
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

pub async fn preview_parse_plan(
    Json(payload): Json<PreviewManagedSiteParsePlanRequest>,
) -> impl IntoResponse {
    match parse_sidecar_client::preview_parse_plan(payload).await {
        Ok(plan) => admin_response::ok("获取解析预览成功", plan),
        Err(err) => admin_response::response(err.status, false, err.message, Some(err.body)),
    }
}

pub async fn preflight_site(Path(site_id): Path<String>) -> impl IntoResponse {
    match managed_sites::preflight_site(&site_id).await {
        Ok(report) => admin_response::ok("部署预检完成", report),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

pub async fn list_remote_targets() -> impl IntoResponse {
    match managed_sites::list_remote_targets() {
        Ok(targets) => admin_response::ok("获取远端部署目标成功", targets),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

pub async fn upsert_remote_target(
    Json(payload): Json<ManagedRemoteTargetRequest>,
) -> impl IntoResponse {
    match managed_sites::upsert_remote_target(payload) {
        Ok(target) => admin_response::ok("保存远端部署目标成功", target),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

pub async fn remote_preflight_site(
    Path(site_id): Path<String>,
    payload: Option<Json<ManagedRemoteDeployRequest>>,
) -> impl IntoResponse {
    let request = payload.map(|Json(value)| value);
    match managed_sites::remote_preflight_site(&site_id, request).await {
        Ok(report) => admin_response::ok("远端部署预检完成", report),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

pub async fn remote_prepare_site(
    Path(site_id): Path<String>,
    payload: Option<Json<ManagedRemoteDeployRequest>>,
) -> impl IntoResponse {
    let request = payload.map(|Json(value)| value);
    match managed_sites::remote_prepare_site(&site_id, request).await {
        Ok(report) => admin_response::ok("远端服务器准备完成", report),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

pub async fn remote_deploy_site(
    Path(site_id): Path<String>,
    payload: Option<Json<ManagedRemoteDeployRequest>>,
) -> impl IntoResponse {
    if let Some(Json(request)) = payload {
        if let Err(err) = managed_sites::remote_preflight_site(&site_id, Some(request)).await {
            return admin_response::managed_error(err.to_string());
        }
    }
    match submit_managed_site_task(&site_id, TaskType::RemoteDeployManagedSite, "远端部署") {
        Ok(task_id) => admin_response::accepted(
            "已提交远端部署任务",
            json!({ "site_id": site_id, "action": "remote_deploy", "task_id": task_id }),
        ),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

pub async fn get_remote_deploy_status(Path(site_id): Path<String>) -> impl IntoResponse {
    match managed_sites::get_remote_deploy_status(&site_id) {
        Ok(status) => admin_response::ok("获取远端部署状态成功", status),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

pub async fn get_remote_agent_status(Path(site_id): Path<String>) -> impl IntoResponse {
    let status = match managed_sites::get_remote_deploy_status(&site_id) {
        Ok(status) => status,
        Err(err) => return admin_response::managed_error(err.to_string()),
    };
    let Some(remote_entry_url) = status.remote_entry_url.as_deref() else {
        return admin_response::managed_error(
            "当前站点尚无远端访问地址，无法拉取 Agent 状态".to_string(),
        );
    };
    let base_url = remote_entry_url.trim_end_matches('/');
    let url = format!("{base_url}/api/site/agent-status");
    let client = match reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(8))
        .build()
    {
        Ok(client) => client,
        Err(err) => return admin_response::managed_error(format!("创建 HTTP client 失败: {err}")),
    };
    match client.get(&url).send().await {
        Ok(resp) => {
            let status_code = resp.status();
            match resp.error_for_status() {
                Ok(resp) => match resp.json::<serde_json::Value>().await {
                    Ok(agent_status) => admin_response::ok(
                        "获取远端 Agent 状态成功",
                        json!({
                            "site_id": site_id,
                            "remote_entry_url": remote_entry_url,
                            "agent_status_url": url,
                            "checked_at": chrono::Utc::now().to_rfc3339(),
                            "agent_status": agent_status,
                        }),
                    ),
                    Err(err) => admin_response::managed_error(format!(
                        "远端 Agent 状态 JSON 解析失败: {err}"
                    )),
                },
                Err(err) => admin_response::managed_error(format!(
                    "远端 Agent 状态 HTTP 异常: status={status_code}; {err}"
                )),
            }
        }
        Err(err) => admin_response::managed_error(format!("请求远端 Agent 状态失败: {err}")),
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
    Json(mut payload): Json<UpdateManagedSiteRequest>,
) -> impl IntoResponse {
    if let Err(err) = resolve_update_site_db_files(&site_id, &mut payload).await {
        return admin_response::response(err.status, false, err.message, Some(err.body));
    }
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

pub async fn append_site_dbfile(
    Path(site_id): Path<String>,
    Json(payload): Json<AppendManagedSiteDbFileRequest>,
) -> impl IntoResponse {
    let mut response = match managed_sites::append_db_file_to_site(&site_id, payload).await {
        Ok(response) => response,
        Err(err) => return admin_response::managed_error(err.to_string()),
    };
    match submit_managed_site_task(&site_id, TaskType::DeployManagedSite, "追加 DB file 部署") {
        Ok(task_id) => {
            response.task_id = Some(task_id.clone());
            admin_response::accepted(
                if response.already_present {
                    "DB file 已在解析范围内，已重新提交追加部署任务"
                } else {
                    "已追加 DB file，并提交解析/生成/启动任务"
                },
                response,
            )
        }
        Err(err) => admin_response::managed_error(err),
    }
}

/// 手动重建站点 db_index 预扫描索引（全局 ref0→dbnum + 精确依赖边，强制全量重扫）。
pub async fn rebuild_site_db_index(Path(site_id): Path<String>) -> impl IntoResponse {
    match managed_sites::rebuild_site_db_index(site_id.clone(), true).await {
        Ok(summary) => admin_response::ok(
            "已重建 db_index 预扫描索引",
            json!({ "site_id": site_id, "action": "db-index-rebuild", "summary": summary }),
        ),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

pub async fn generate_site(Path(site_id): Path<String>) -> impl IntoResponse {
    match managed_sites::generate_site(site_id.clone(), true).await {
        Ok(()) => admin_response::accepted(
            "已提交模型生成并启动 plant3d-web 任务",
            json!({ "site_id": site_id, "action": "generate" }),
        ),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

pub async fn deploy_site(Path(site_id): Path<String>) -> impl IntoResponse {
    match submit_managed_site_task(&site_id, TaskType::DeployManagedSite, "完整部署") {
        Ok(task_id) => admin_response::accepted(
            "已提交完整部署任务",
            json!({ "site_id": site_id, "action": "deploy", "task_id": task_id }),
        ),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

/// 重新部署：先删除旧数据（停站 + 清空 `<runtime>/data/`，保留配置），再提交
/// 完整部署任务重新走「解析 → 生成 → 启动」。
pub async fn redeploy_site(Path(site_id): Path<String>) -> impl IntoResponse {
    if let Err(err) = managed_sites::redeploy_reset_site(&site_id).await {
        return admin_response::managed_error(err.to_string());
    }
    match submit_managed_site_task(&site_id, TaskType::DeployManagedSite, "重新部署") {
        Ok(task_id) => admin_response::accepted(
            "已删除旧数据并提交重新部署任务",
            json!({ "site_id": site_id, "action": "redeploy", "task_id": task_id }),
        ),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

/// Admin 鉴权版快速部署：POST /api/admin/sites/quick-deploy
///
/// 复用 quick deploy 的 dbfile/dbnum 解析与建站逻辑，但 admin 入口只创建配置，
/// 不自动提交解析/生成/启动任务。
/// 支持只传绝对 dbfile，后端会自动推断 project_path 并读取文件头得到 dbnum。
pub async fn quick_deploy_site(Json(payload): Json<QuickDeployTestRequest>) -> impl IntoResponse {
    match managed_sites::quick_deploy_admin(payload).await {
        Ok(resp) => admin_response::accepted("快速创建部署配置成功", resp),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

/// 一键部署测试（免鉴权快测）：POST /api/admin/quick-deploy-test
///
/// 传 project_path + db_file/dbnum，单次完成 建站→解析(单库)→生成→(可选)启动。
/// 该端点不挂 admin 鉴权中间件（在主路由注册），仅用于本地/测试快速验证。
pub async fn quick_deploy_test(Json(payload): Json<QuickDeployTestRequest>) -> impl IntoResponse {
    // P0 安全收口：该端点不挂 admin 鉴权中间件，默认禁用，避免在生产被未授权调用。
    // 如需本地 / 测试快测，显式设置 AIOS_ENABLE_QUICK_DEPLOY_TEST=1 后重启 web_server。
    if !managed_sites::quick_deploy_test_enabled() {
        return admin_response::forbidden(
            "quick-deploy-test 已禁用；如需本地 / 测试快测，请设置 AIOS_ENABLE_QUICK_DEPLOY_TEST=1 后重启 web_server",
        );
    }
    match managed_sites::quick_deploy_test(payload).await {
        Ok(resp) => admin_response::ok("一键部署测试完成", resp),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

pub async fn start_site(Path(site_id): Path<String>) -> impl IntoResponse {
    match submit_managed_site_task(&site_id, TaskType::StartManagedSite, "启动站点") {
        Ok(task_id) => admin_response::accepted(
            "已提交启动任务",
            json!({ "site_id": site_id, "action": "start", "task_id": task_id }),
        ),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

pub async fn stop_site(Path(site_id): Path<String>) -> impl IntoResponse {
    match managed_sites::stop_site(&site_id).await {
        Ok(result) if result.conflict => admin_response::conflict(format!(
            "受管进程已停止，但端口仍被外部进程占用: web={:?} db={:?} viewer={:?}",
            result.web_conflict_pids, result.db_conflict_pids, result.viewer_conflict_pids
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

pub async fn get_site_deploy_validation(Path(site_id): Path<String>) -> impl IntoResponse {
    match managed_sites::deploy_validation_report(&site_id) {
        Ok(report) => admin_response::ok("获取部署验收报告成功", report),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

pub async fn refresh_site_deploy_validation(Path(site_id): Path<String>) -> impl IntoResponse {
    match managed_sites::refresh_deploy_validation_report(&site_id).await {
        Ok(report) => admin_response::ok("刷新部署验收报告成功", report),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

pub async fn reconcile_site(
    Path(site_id): Path<String>,
    payload: Option<Json<ManagedSiteReconcileRequest>>,
) -> impl IntoResponse {
    let cleanup_orphans = payload
        .map(|Json(req)| req.cleanup_orphans)
        .unwrap_or(false);
    match managed_sites::reconcile_site(&site_id, cleanup_orphans).await {
        Ok(report) => admin_response::ok("站点运行态对账完成", report),
        Err(err) => admin_response::managed_error(err.to_string()),
    }
}

/// 单条日志类别的分页尾部查询（D5 / Sprint D · 修 G13）
///
/// `GET /api/admin/sites/{id}/logs/{kind}?limit=N`
/// - `kind` ∈ parse / generate / db / web
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
        return admin_response::managed_error(format!("读取日志文件失败: {}", err)).into_response();
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
        .unwrap_or_else(|_| (StatusCode::INTERNAL_SERVER_ERROR, "构造下载响应失败").into_response())
}

fn runtime_ok(runtime: ManagedSiteRuntimeStatus) -> ApiResponse {
    admin_response::ok("获取站点运行状态成功", runtime)
}

fn logs_ok(logs: ManagedSiteLogsResponse) -> ApiResponse {
    admin_response::ok("获取站点日志成功", logs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_browser_connection_mode_defaults_to_reader() {
        let mode = normalize_data_browser_connection_mode(None).unwrap();
        assert_eq!(mode.as_str(), "reader");
        assert!(!mode.can_write());
    }

    #[test]
    fn data_browser_connection_mode_accepts_editor() {
        let mode = normalize_data_browser_connection_mode(Some("editor".to_string())).unwrap();
        assert_eq!(mode.as_str(), "editor");
        assert!(mode.can_write());
    }

    #[test]
    fn data_browser_connection_mode_rejects_unknown_values() {
        let err = normalize_data_browser_connection_mode(Some("owner".to_string())).unwrap_err();
        assert!(err.contains("mode"));
    }
}
