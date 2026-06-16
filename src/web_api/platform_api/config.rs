//! Platform configuration (embed URL / frontend base).
use std::net::{IpAddr, Ipv4Addr, UdpSocket};

/// 平台前端配置
#[derive(Clone, Debug)]
pub struct PlatformConfig {
    pub frontend_relative_path: String,
    /// 前端基地址（用于拼接完整 URL），为空时不返回 url 字段
    pub frontend_base_url: String,
    /// 受管站点 ID。存在时 embed-url 可动态读取站点真实 Viewer 地址。
    pub site_id: Option<String>,
}

impl Default for PlatformConfig {
    fn default() -> Self {
        Self {
            frontend_relative_path: "/review/3d-view".to_string(),
            frontend_base_url: String::new(),
            site_id: None,
        }
    }
}

impl PlatformConfig {
    pub fn from_config_file() -> Self {
        if let Some(config) = super::super::jwt_auth::load_config() {
            return Self {
                frontend_base_url: resolve_frontend_base_url(&config),
                site_id: get_nonempty_config_string(&config, "web_server.site_id"),
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

    get_local_ip_via_udp().unwrap_or_else(|err| {
        tracing::warn!("无法推断本机真实 IPv4: {err}");
        "0.0.0.0".to_string()
    })
}

fn is_loopback_or_unspecified_host(host: &str) -> bool {
    let normalized = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    if matches!(
        normalized.as_str(),
        "" | "0.0.0.0" | "::" | "127.0.0.1" | "localhost" | "::1"
    ) {
        return true;
    }
    normalized
        .parse::<IpAddr>()
        .map(|ip| ip.is_loopback() || ip.is_unspecified())
        .unwrap_or(false)
}

fn get_local_ip_via_udp() -> Result<String, std::io::Error> {
    fn host_from_url_or_host(value: &str) -> Option<&str> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }
        let without_scheme = trimmed
            .strip_prefix("http://")
            .or_else(|| trimmed.strip_prefix("https://"))
            .unwrap_or(trimmed);
        without_scheme
            .split('/')
            .next()
            .and_then(|host_port| host_port.split(':').next())
            .map(str::trim)
            .filter(|host| !host.is_empty())
    }

    for key in [
        "AIOS_PUBLIC_HOST",
        "AIOS_LOCAL_IP",
        "AIOS_VIEWER_HOST",
        "AIOS_VIEWER_BASE_URL",
    ] {
        let Ok(value) = std::env::var(key) else {
            continue;
        };
        let Some(host) = host_from_url_or_host(&value) else {
            continue;
        };
        if let Ok(IpAddr::V4(ipv4)) = host.parse::<IpAddr>() {
            if !ipv4.is_unspecified() && !ipv4.is_loopback() {
                return Ok(ipv4.to_string());
            }
        }
    }

    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    socket.connect((Ipv4Addr::new(8, 8, 8, 8), 80))?;
    match socket.local_addr()?.ip() {
        IpAddr::V4(ipv4) if !ipv4.is_unspecified() && !ipv4.is_loopback() => Ok(ipv4.to_string()),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "无法推断本机真实 IPv4，请设置 AIOS_PUBLIC_HOST 或 AIOS_LOCAL_IP",
        )),
    }
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
