//! JWT Authentication Module
//!
//! Provides JWT token generation, verification, and decoding for API authentication.

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
use jsonwebtoken::{encode, decode, Header, Algorithm, Validation, EncodingKey, DecodingKey, TokenData};

// ============================================================================
// Configuration
// ============================================================================

/// JWT 配置
#[derive(Clone, Debug)]
pub struct JwtConfig {
    /// 密钥 (用于签名和验证)
    pub secret: String,
    /// Token 过期时间（小时）
    pub expiration_hours: u64,
}

impl Default for JwtConfig {
    fn default() -> Self {
        Self {
            secret: std::env::var("JWT_SECRET")
                .unwrap_or_else(|_| "default-jwt-secret-key".to_string()),
            expiration_hours: 24,
        }
    }
}

impl JwtConfig {
    /// 从配置文件加载
    pub fn from_config_file() -> Self {
        let config_path = "DbOption.toml";
        if let Ok(content) = std::fs::read_to_string(config_path) {
            if let Ok(toml_value) = content.parse::<toml::Value>() {
                if let Some(mc) = toml_value.get("model_center") {
                    let secret = mc.get("token_secret")
                        .and_then(|v| v.as_str())
                        .unwrap_or("default-jwt-secret-key")
                        .to_string();
                    
                    let expiration_hours = mc.get("token_expiration_hours")
                        .and_then(|v| v.as_integer())
                        .unwrap_or(24) as u64;
                    
                    return Self {
                        secret,
                        expiration_hours,
                    };
                }
            }
        }
        Self::default()
    }
}

// ============================================================================
// Role Definition
// ============================================================================

/// 角色枚举
/// - admin: 管理员
/// - sj: 设计（编）
/// - sh: 审核（审）
/// - jd: 校对（校）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Sj,
    Sh,
    Jd,
}

impl Role {
    /// 从字符串解析角色
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "admin" => Some(Role::Admin),
            "sj" => Some(Role::Sj),
            "sh" => Some(Role::Sh),
            "jd" => Some(Role::Jd),
            _ => None,
        }
    }
    
    /// 转换为字符串
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Sj => "sj",
            Role::Sh => "sh",
            Role::Jd => "jd",
        }
    }
    
    /// 获取角色显示名称
    pub fn display_name(&self) -> &'static str {
        match self {
            Role::Admin => "管理员",
            Role::Sj => "设计（编）",
            Role::Sh => "审核（审）",
            Role::Jd => "校对（校）",
        }
    }
    
    /// 获取所有有效角色值
    pub fn valid_values() -> &'static [&'static str] {
        &["admin", "sj", "sh", "jd"]
    }
}

// ============================================================================
// JWT Claims
// ============================================================================

/// JWT Payload (Claims)
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TokenClaims {
    /// 项目号
    pub project_id: String,
    /// 用户ID
    pub user_id: String,
    /// 表单ID
    pub form_id: String,
    /// 角色 (可选)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// 过期时间戳 (Unix timestamp)
    pub exp: u64,
    /// 签发时间戳 (Unix timestamp)
    pub iat: u64,
}

// ============================================================================
// Request/Response Structs
// ============================================================================

/// Token 获取请求
/// 支持两种格式:
/// 1. 新格式: {"username": "xxx", "project": "xxx", "role": "xxx"}
/// 2. 旧格式: {"project_id": "xxx", "user_id": "xxx", "form_id": "xxx"}
#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    /// 项目号 (新格式使用 project，旧格式使用 project_id)
    #[serde(alias = "project")]
    pub project_id: Option<String>,
    /// 用户ID (新格式使用 username，旧格式使用 user_id)
    #[serde(alias = "username")]
    pub user_id: Option<String>,
    /// 可选的 form_id，如果不传则自动生成
    pub form_id: Option<String>,
    /// 角色 (可选，用于权限控制)
    pub role: Option<String>,
}

/// Token 获取响应
#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub code: i32,
    pub message: String,
    pub data: Option<TokenData_>,
}

#[derive(Debug, Serialize)]
pub struct TokenData_ {
    /// JWT Token
    pub token: String,
    /// Token 过期时间 (Unix timestamp)
    pub expires_at: u64,
    /// 表单ID
    pub form_id: String,
}

/// Token 验证请求
#[derive(Debug, Deserialize)]
pub struct VerifyRequest {
    pub token: String,
}

/// Token 验证响应
#[derive(Debug, Serialize)]
pub struct VerifyResponse {
    pub code: i32,
    pub message: String,
    pub data: Option<VerifyData>,
}

#[derive(Debug, Serialize)]
pub struct VerifyData {
    /// 是否有效
    pub valid: bool,
    /// 解码后的 Claims
    pub claims: Option<TokenClaims>,
    /// 如果无效，错误原因
    pub error: Option<String>,
}

// ============================================================================
// JWT Functions
// ============================================================================

lazy_static::lazy_static! {
    static ref JWT_CONFIG: JwtConfig = JwtConfig::from_config_file();
}

/// 生成 form_id (UUID)
pub fn generate_form_id() -> String {
    format!("FORM-{}", uuid::Uuid::new_v4().to_string().replace("-", "").to_uppercase()[..12].to_string())
}

/// 生成 JWT Token
#[cfg(feature = "web_server")]
pub fn create_token(project_id: &str, user_id: &str, form_id: &str, role: Option<&str>) -> Result<(String, u64), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    
    let exp = now + JWT_CONFIG.expiration_hours * 3600;
    
    let claims = TokenClaims {
        project_id: project_id.to_string(),
        user_id: user_id.to_string(),
        form_id: form_id.to_string(),
        role: role.map(|s| s.to_string()),
        exp,
        iat: now,
    };
    
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(JWT_CONFIG.secret.as_bytes()),
    ).map_err(|e| e.to_string())?;
    
    Ok((token, exp))
}

/// 验证 JWT Token 并返回 Claims
#[cfg(feature = "web_server")]
pub fn verify_token(token: &str) -> Result<TokenClaims, String> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    
    let token_data: TokenData<TokenClaims> = decode(
        token,
        &DecodingKey::from_secret(JWT_CONFIG.secret.as_bytes()),
        &validation,
    ).map_err(|e| e.to_string())?;
    
    Ok(token_data.claims)
}

/// 解码 JWT Token（不验证签名，用于调试）
#[cfg(feature = "web_server")]
pub fn decode_token_unsafe(token: &str) -> Result<TokenClaims, String> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.insecure_disable_signature_validation();
    validation.validate_exp = false;
    
    let token_data: TokenData<TokenClaims> = decode(
        token,
        &DecodingKey::from_secret(&[]), // 不需要密钥
        &validation,
    ).map_err(|e| e.to_string())?;
    
    Ok(token_data.claims)
}

// ============================================================================
// Axum Handlers
// ============================================================================

/// 创建 JWT 认证路由
#[cfg(feature = "web_server")]
pub fn create_jwt_auth_routes() -> Router {
    Router::new()
        .route("/api/auth/token", post(get_token))
        .route("/api/auth/verify", post(verify_token_handler))
}

/// 获取 Token
#[cfg(feature = "web_server")]
async fn get_token(
    Json(request): Json<TokenRequest>,
) -> impl IntoResponse {
    // 获取 project_id 和 user_id (支持两种格式)
    let project_id = match &request.project_id {
        Some(id) if !id.is_empty() => id.clone(),
        _ => {
            warn!("Missing project_id/project");
            return (StatusCode::BAD_REQUEST, Json(TokenResponse {
                code: -1,
                message: "Missing required field: project_id or project".to_string(),
                data: None,
            }));
        }
    };
    
    let user_id = match &request.user_id {
        Some(id) if !id.is_empty() => id.clone(),
        _ => {
            warn!("Missing user_id/username");
            return (StatusCode::BAD_REQUEST, Json(TokenResponse {
                code: -1,
                message: "Missing required field: user_id or username".to_string(),
                data: None,
            }));
        }
    };
    
    info!("Token request: project_id={}, user_id={}, role={:?}", project_id, user_id, request.role);
    
    // 验证 role (如果提供了)
    let validated_role = if let Some(ref role_str) = request.role {
        match Role::from_str(role_str) {
            Some(role) => Some(role.as_str()),
            None => {
                warn!("Invalid role: {}", role_str);
                return (StatusCode::BAD_REQUEST, Json(TokenResponse {
                    code: -1,
                    message: format!("Invalid role: '{}'. Valid values are: {:?}", role_str, Role::valid_values()),
                    data: None,
                }));
            }
        }
    } else {
        None
    };
    
    // 生成或使用传入的 form_id
    let form_id = request.form_id.unwrap_or_else(generate_form_id);
    
    // 生成 token
    match create_token(&project_id, &user_id, &form_id, validated_role) {
        Ok((token, expires_at)) => {
            info!("Token generated for user={}, form_id={}", user_id, form_id);
            (StatusCode::OK, Json(TokenResponse {
                code: 0,
                message: "ok".to_string(),
                data: Some(TokenData_ {
                    token,
                    expires_at,
                    form_id,
                }),
            }))
        }
        Err(e) => {
            warn!("Token generation failed: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(TokenResponse {
                code: -1,
                message: format!("Token generation failed: {}", e),
                data: None,
            }))
        }
    }
}

/// 验证 Token
#[cfg(feature = "web_server")]
async fn verify_token_handler(
    Json(request): Json<VerifyRequest>,
) -> impl IntoResponse {
    info!("Token verification request");
    
    match verify_token(&request.token) {
        Ok(claims) => {
            info!("Token verified: user_id={}, form_id={}", claims.user_id, claims.form_id);
            (StatusCode::OK, Json(VerifyResponse {
                code: 0,
                message: "ok".to_string(),
                data: Some(VerifyData {
                    valid: true,
                    claims: Some(claims),
                    error: None,
                }),
            }))
        }
        Err(e) => {
            warn!("Token verification failed: {}", e);
            (StatusCode::OK, Json(VerifyResponse {
                code: 0,
                message: "ok".to_string(),
                data: Some(VerifyData {
                    valid: false,
                    claims: None,
                    error: Some(e),
                }),
            }))
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[cfg(feature = "web_server")]
mod tests {
    use super::*;
    
    #[test]
    fn test_create_and_verify_token() {
        let (token, _exp) = create_token("2410", "kangwp", "FORM-TEST123", None).unwrap();
        assert!(!token.is_empty());
        
        let claims = verify_token(&token).unwrap();
        assert_eq!(claims.project_id, "2410");
        assert_eq!(claims.user_id, "kangwp");
        assert_eq!(claims.form_id, "FORM-TEST123");
        assert_eq!(claims.role, None);
    }
    
    #[test]
    fn test_create_token_with_role() {
        let (token, _exp) = create_token("testproject", "testuser", "FORM-TEST123", Some("admin")).unwrap();
        assert!(!token.is_empty());
        
        let claims = verify_token(&token).unwrap();
        assert_eq!(claims.project_id, "testproject");
        assert_eq!(claims.user_id, "testuser");
        assert_eq!(claims.role, Some("admin".to_string()));
    }
    
    #[test]
    fn test_decode_token_unsafe() {
        let (token, _exp) = create_token("2410", "kangwp", "FORM-TEST123", None).unwrap();
        
        let claims = decode_token_unsafe(&token).unwrap();
        assert_eq!(claims.project_id, "2410");
        assert_eq!(claims.user_id, "kangwp");
    }
    
    #[test]
    fn test_invalid_token() {
        let result = verify_token("invalid-token");
        assert!(result.is_err());
    }
}
