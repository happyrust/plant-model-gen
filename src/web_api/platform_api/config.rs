//! Platform configuration (embed URL / frontend base).
use crate::shared::net_util::{is_loopback_or_unspecified_host, local_ip_or_loopback};

/// 平台前端配置
#[derive(Clone, Debug)]
pub struct PlatformConfig {
    pub frontend_relative_path: String,
    /// 前端基地址（用于拼接完整 URL），为空时不返回 url 字段
    pub frontend_base_url: String,
}

impl Default for PlatformConfig {
    fn default() -> Self {
        Self {
            frontend_relative_path: "/review/3d-view".to_string(),
            frontend_base_url: String::new(),
        }
    }
}

impl PlatformConfig {
    pub fn from_config_file() -> Self {
        if let Some(config) = super::super::jwt_auth::load_config() {
            return Self {
                frontend_base_url: resolve_frontend_base_url(&config),
                ..Self::default()
            };
        }
        Self::default()
    }
}

fn resolve_frontend_base_url(config: &config::Config) -> String {
    get_nonempty_config_string(config, "web_server.frontend_url")
        .or_else(|| get_nonempty_config_string(config, "model_center.frontend_base_url"))
        .or_else(|| get_nonempty_config_string(config, "web_server.public_base_url"))
        .or_else(|| derive_frontend_url_from_web_server(config))
        .unwrap_or_default()
}

fn get_nonempty_config_string(config: &config::Config, key: &str) -> Option<String> {
    config
        .get_string(key)
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
}

fn derive_frontend_url_from_web_server(config: &config::Config) -> Option<String> {
    let port = config
        .get_int("web_server.port")
        .ok()
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| *value > 0)?;
    let bind_host = config
        .get_string("web_server.bind_host")
        .unwrap_or_else(|_| "0.0.0.0".to_string());
    let access_host = access_host_from_bind_host(&bind_host);
    Some(format!("http://{}:{}", url_host(&access_host), port))
}

fn access_host_from_bind_host(bind_host: &str) -> String {
    let trimmed = bind_host.trim();
    if !is_loopback_or_unspecified_host(trimmed) {
        return trimmed.to_string();
    }
    // 失败回退 127.0.0.1（specs/004：展示型地址禁止 0.0.0.0 / 空串）。
    local_ip_or_loopback()
}

fn url_host(host: &str) -> String {
    let trimmed = host.trim().trim_start_matches('[').trim_end_matches(']');
    if trimmed.contains(':') {
        format!("[{trimmed}]")
    } else {
        trimmed.to_string()
    }
}

lazy_static::lazy_static! {
    pub static ref PLATFORM_CONFIG: PlatformConfig = PlatformConfig::from_config_file();
}
