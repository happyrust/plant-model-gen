//! Review Embed URL API
//!
//! Provides the embed URL for 3D review interface.
//! This is the Platform's API that external systems (like Model Center) call.

use axum::{
    Router,
    extract::Json,
    http::StatusCode,
    routing::post,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

#[cfg(feature = "web_server")]
use super::jwt_auth::{create_token, generate_form_id};

// ============================================================================
// Configuration
// ============================================================================

/// 平台配置
#[derive(Clone, Debug)]
pub struct PlatformConfig {
    /// 前端相对路径
    pub frontend_relative_path: String,
}

impl Default for PlatformConfig {
    fn default() -> Self {
        Self {
            // plant3d-web 前端的校审页面路径
            frontend_relative_path: "/review/3d-view".to_string(),
        }
    }
}

impl PlatformConfig {
    /// 从配置文件加载
    pub fn from_config_file() -> Self {
        // 目前只有前端路径配置，直接使用默认值
        Self::default()
    }
}

// ============================================================================
// Request/Response Structs
// ============================================================================

/// 嵌入地址请求 (来自外部系统)
#[derive(Debug, Deserialize)]
pub struct EmbedUrlRequest {
    pub project_id: String,
    pub user_id: String,
    /// 外部传入的 token (用于验证请求合法性)
    pub token: Option<String>,
    #[serde(default)]
    pub extra_parameters: Option<serde_json::Value>,
}

/// 嵌入地址响应
#[derive(Debug, Serialize)]
pub struct EmbedUrlResponse {
    pub code: i32,
    pub message: String,
    pub data: Option<EmbedUrlData>,
}

#[derive(Debug, Serialize)]
pub struct EmbedUrlData {
    /// 前端相对路径
    pub relative_path: String,
    /// 平台生成的访问 token
    pub token: String,
    /// 查询参数
    pub query: EmbedUrlQuery,
}

#[derive(Debug, Serialize)]
pub struct EmbedUrlQuery {
    /// 平台生成的提资单 ID (UUID)
    pub form_id: String,
    /// 是否为审核人员
    pub is_reviewer: bool,
}

/// 缓存预加载请求 (我们调用模型中心)
#[derive(Debug, Deserialize)]
pub struct CachePreloadRequest {
    pub project_id: String,
    pub initiator: String,
}

/// 缓存预加载响应
#[derive(Debug, Serialize)]
pub struct CachePreloadResponse {
    pub success: bool,
    pub message: String,
    pub task_id: Option<String>,
}

// ============================================================================
// Axum Handlers
// ============================================================================

lazy_static::lazy_static! {
    static ref PLATFORM_CONFIG: PlatformConfig = PlatformConfig::from_config_file();
}

#[cfg(feature = "web_server")]
pub fn create_model_center_routes() -> Router {
    Router::new()
        // 我们提供的接口 (外部调用我们)
        .route("/api/review/embed-url", post(get_embed_url))
        // 代理接口 (我们调用模型中心) - 用于触发模型中心的缓存
        .route("/api/review/preload-cache", post(preload_cache))
}

/// 获取嵌入地址 - 平台提供给外部的接口
#[cfg(feature = "web_server")]
async fn get_embed_url(
    Json(request): Json<EmbedUrlRequest>,
) -> impl IntoResponse {
    info!("Embed URL request: project_id={}, user_id={}", request.project_id, request.user_id);
    
    // 1. 生成新的 form_id (UUID)
    let form_id = generate_form_id();
    
    // 2. 生成 JWT token
    match create_token(&request.project_id, &request.user_id, &form_id, None) {
        Ok((token, _expires_at)) => {
            // 3. 判断是否为审核人员 (可以根据 extra_parameters 或其他逻辑判断)
            let is_reviewer = request.extra_parameters
                .as_ref()
                .and_then(|p| p.get("is_reviewer"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            
            info!("Generated form_id={}, token_len={}", form_id, token.len());
            
            // 4. 返回响应
            (StatusCode::OK, Json(EmbedUrlResponse {
                code: 0,
                message: "ok".to_string(),
                data: Some(EmbedUrlData {
                    relative_path: PLATFORM_CONFIG.frontend_relative_path.clone(),
                    token,
                    query: EmbedUrlQuery {
                        form_id,
                        is_reviewer,
                    },
                }),
            }))
        }
        Err(e) => {
            warn!("Token generation failed: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(EmbedUrlResponse {
                code: -1,
                message: format!("Token generation failed: {}", e),
                data: None,
            }))
        }
    }
}

/// 触发缓存预加载 - 代理到模型中心
#[cfg(feature = "web_server")]
async fn preload_cache(
    Json(request): Json<CachePreloadRequest>,
) -> impl IntoResponse {
    info!("Cache preload request: project_id={}, initiator={}", request.project_id, request.initiator);
    
    // 这里可以调用模型中心的缓存接口
    // 目前返回一个占位响应
    (StatusCode::OK, Json(CachePreloadResponse {
        success: true,
        message: "Cache preload request accepted".to_string(),
        task_id: Some(format!("cache_{}", request.project_id)),
    }))
}
