//! 当前 web_server 进程的 HTTP 监听信息（一进程对应一个站点边界）。
use serde_json::json;
use std::sync::OnceLock;

use super::site_registry::WebServerRuntimeConfig;

static WEB_LISTEN: OnceLock<(String, u16)> = OnceLock::new();
static SITE_RUNTIME: OnceLock<WebServerRuntimeConfig> = OnceLock::new();

/// 在 `start_web_server_with_config` 成功 `bind` 之后调用一次。
pub fn init_web_listen(host: impl Into<String>, port: u16) {
    let _ = WEB_LISTEN.set((host.into(), port));
}

pub fn init_site_identity(runtime: WebServerRuntimeConfig) {
    let _ = SITE_RUNTIME.set(runtime);
}

pub fn get_web_listen() -> Option<(&'static str, u16)> {
    WEB_LISTEN.get().map(|(h, p)| (h.as_str(), *p))
}

pub fn current_site_id() -> Option<String> {
    SITE_RUNTIME.get().map(|runtime| runtime.site_id.clone())
}

/// `GET /api/site/identity` 返回体：便于网关/前端区分「当前是哪个站点的 web_server」。
pub fn site_identity_json() -> serde_json::Value {
    let (host, port) = get_web_listen().unwrap_or(("0.0.0.0", 0));
    let runtime = SITE_RUNTIME.get();
    let db_option = aios_core::get_db_option();
    let project_code = db_option.project_code.parse::<u32>().ok();

    json!({
        "deployment_model": "one_web_server_per_site",
        "web_listen_host": host,
        "web_listen_port": port,
        "site_id": runtime.map(|v| v.site_id.clone()),
        "site_name": runtime.map(|v| v.site_name.clone()),
        "region": runtime.and_then(|v| v.region.clone()),
        "frontend_url": runtime.and_then(|v| v.frontend_url.clone()),
        "backend_url": runtime.map(|v| v.backend_url.clone()),
        "public_base_url": runtime.map(|v| v.backend_url.clone()),
        "project_name": db_option.project_name,
        "project_code": project_code,
        "project_path": db_option.project_path,
        "bind_host": runtime.map(|v| v.bind_host.clone()).unwrap_or_else(|| host.to_string()),
        "bind_port": runtime.map(|v| v.bind_port).unwrap_or(port),
        "registration_status": if runtime.is_some() { "registered" } else { "unregistered" },
        "sites_list_endpoints": ["/api/sites", "/api/deployment-sites"],
        "note": "一个 web_server 进程只代表一个站点与一个项目；站点清单来自中心 SQLite 注册表。"
    })
}
