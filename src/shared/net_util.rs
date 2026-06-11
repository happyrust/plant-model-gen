//! 本机网络地址推断工具。
//!
//! `web_server` 与 `web_api` 共用的唯一实现（specs/004-site-input-validation-net-util，
//! 消除 NEW-2 双份漂移）。展示型 URL 的失败回退统一用 [`local_ip_or_loopback`]，
//! 禁止再以空串或 `0.0.0.0` 拼接用户可见地址。

use std::net::{IpAddr, Ipv4Addr, UdpSocket};

/// host 是否为回环/未指定地址（含 `[]` 包裹的 IPv6 字面量）。
pub fn is_loopback_or_unspecified_host(host: &str) -> bool {
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

/// 推断本机真实 IPv4：优先显式环境变量，其次 UDP 路由探测（不发包）。
pub fn get_local_ip_via_udp() -> Result<String, std::io::Error> {
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

/// 展示型 URL / 本机探测的统一回退：推断失败时返回 `127.0.0.1` 并告警。
///
/// 离线无路由环境下本机回环至少可访问，绝不产生 `http://:port` 或
/// `http://0.0.0.0:port` 这类损坏地址。
pub fn local_ip_or_loopback() -> String {
    get_local_ip_via_udp().unwrap_or_else(|err| {
        tracing::warn!(
            "无法推断本机真实 IPv4，回退 127.0.0.1（可设 AIOS_PUBLIC_HOST 覆盖）: {err}"
        );
        "127.0.0.1".to_string()
    })
}
