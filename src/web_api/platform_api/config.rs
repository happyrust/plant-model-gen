//! Platform & external review system configuration.

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

/// 外部校审系统出站调用配置
#[derive(Clone, Debug)]
pub struct ExternalReviewConfig {
    pub base_url: String,
    pub workflow_delete_path: String,
    pub auth_secret: String,
    pub timeout_seconds: u64,
}

impl Default for ExternalReviewConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            workflow_delete_path: "/api/workflow/delete".to_string(),
            auth_secret: "shared-review-secret".to_string(),
            timeout_seconds: 15,
        }
    }
}

impl ExternalReviewConfig {
    pub fn from_config_file() -> Self {
        use config as cfg;

        let mut names = Vec::new();
        if let Ok(config_path) = std::env::var("DB_OPTION_FILE") {
            names.push(
                config_path
                    .strip_suffix(".toml")
                    .unwrap_or(&config_path)
                    .to_string(),
            );
        }
        names.extend([
            "db_options/DbOption".to_string(),
            "../db_options/DbOption".to_string(),
            "DbOption".to_string(),
        ]);

        for name in &names {
            let file_path = format!("{}.toml", name);
            if std::path::Path::new(&file_path).exists() {
                if let Ok(config) = cfg::Config::builder()
                    .add_source(cfg::File::with_name(name))
                    .build()
                {
                    return Self {
                        base_url: config
                            .get_string("external_review.base_url")
                            .unwrap_or_default(),
                        workflow_delete_path: config
                            .get_string("external_review.workflow_delete_path")
                            .unwrap_or_else(|_| "/api/workflow/delete".to_string()),
                        auth_secret: config
                            .get_string("external_review.auth_secret")
                            .unwrap_or_else(|_| "shared-review-secret".to_string()),
                        timeout_seconds: config
                            .get_int("external_review.timeout_seconds")
                            .unwrap_or(15) as u64,
                    };
                }
            }
        }
        Self::default()
    }

    /// base_url 为空时启用 mock 模式
    pub fn is_mock(&self) -> bool {
        self.base_url.trim().is_empty()
    }
}

lazy_static::lazy_static! {
    pub static ref PLATFORM_CONFIG: PlatformConfig = PlatformConfig::from_config_file();
    pub static ref EXTERNAL_REVIEW_CONFIG: ExternalReviewConfig = ExternalReviewConfig::from_config_file();
}
