//! Platform configuration (embed URL / frontend base).

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
                frontend_base_url: config
                    .get_string("model_center.frontend_base_url")
                    .unwrap_or_default(),
                ..Self::default()
            };
        }
        Self::default()
    }
}

lazy_static::lazy_static! {
    pub static ref PLATFORM_CONFIG: PlatformConfig = PlatformConfig::from_config_file();
}
