use axum::{
    Router,
    extract::{Json, Request},
    http::{HeaderMap, Method, StatusCode, header::AUTHORIZATION},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::Mutex;
use uuid::Uuid;

type ApiResponse = (StatusCode, Json<Value>);

static SESSIONS: LazyLock<Mutex<HashMap<String, AdminSession>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Clone, Serialize)]
struct AdminSession {
    token: String,
    username: String,
    role: String,
    expires_at: String,
}

#[derive(Deserialize)]
pub struct LoginRequest {
    username: String,
    password: String,
}

pub fn create_admin_auth_routes() -> Router {
    Router::new()
        .route("/api/admin/auth/login", post(login))
        .route("/api/admin/auth/logout", post(logout))
        .route("/api/admin/auth/me", get(me))
}

pub async fn admin_session_middleware(
    request: Request,
    next: Next,
) -> Result<Response, ApiResponse> {
    if request.method() == Method::OPTIONS {
        return Ok(next.run(request).await);
    }

    require_admin_session(request.headers())?;
    Ok(next.run(request).await)
}

async fn login(Json(payload): Json<LoginRequest>) -> impl IntoResponse {
    let admin_user = std::env::var("ADMIN_USER").unwrap_or_else(|_| "admin".to_string());
    let admin_pass = std::env::var("ADMIN_PASS").unwrap_or_else(|_| "admin".to_string());

    if payload.username != admin_user || payload.password != admin_pass {
        return response::<Value>(StatusCode::UNAUTHORIZED, false, "用户名或密码错误", None);
    }

    let token = Uuid::new_v4().to_string();
    let session = AdminSession {
        token: token.clone(),
        username: payload.username.clone(),
        role: "admin".to_string(),
        expires_at: format!(
            "{}",
            chrono::Utc::now()
                .checked_add_signed(chrono::Duration::hours(24))
                .unwrap_or_else(chrono::Utc::now)
                .to_rfc3339()
        ),
    };

    if let Ok(mut sessions) = SESSIONS.lock() {
        sessions.insert(token, session.clone());
    }

    response(StatusCode::OK, true, "登录成功", Some(session))
}

async fn logout(headers: HeaderMap) -> impl IntoResponse {
    if let Some(token) = extract_bearer_token(&headers) {
        if let Ok(mut sessions) = SESSIONS.lock() {
            cleanup_expired_sessions(&mut sessions);
            sessions.remove(token);
        }
    }
    response(
        StatusCode::OK,
        true,
        "登出成功",
        Some(json!({
            "logged_out": true
        })),
    )
}

async fn me(headers: HeaderMap) -> Response {
    let session = match require_admin_session(&headers) {
        Ok(session) => session,
        Err(err) => return err.into_response(),
    };
    response(
        StatusCode::OK,
        true,
        "获取用户信息成功",
        Some(json!({
            "username": session.username,
            "role": session.role
        })),
    )
    .into_response()
}

fn require_admin_session(headers: &HeaderMap) -> Result<AdminSession, ApiResponse> {
    let token = extract_bearer_token(headers)
        .ok_or_else(|| unauthorized("缺少或无效的 Authorization"))?;
    validate_session_token(token).map_err(unauthorized)
}

fn validate_session_token(token: &str) -> Result<AdminSession, String> {
    let mut sessions = SESSIONS
        .lock()
        .map_err(|_| "管理员会话不可用，请稍后重试".to_string())?;
    cleanup_expired_sessions(&mut sessions);
    let session = sessions
        .get(token)
        .cloned()
        .ok_or_else(|| "管理员会话不存在或已失效".to_string())?;

    let expires_at = parse_expire_time(&session.expires_at)
        .ok_or_else(|| "管理员会话过期时间无效".to_string())?;
    if expires_at <= Utc::now() {
        sessions.remove(token);
        return Err("管理员会话已过期".to_string());
    }

    Ok(session)
}

fn cleanup_expired_sessions(sessions: &mut HashMap<String, AdminSession>) {
    let now = Utc::now();
    sessions.retain(|_, session| {
        parse_expire_time(&session.expires_at)
            .map(|expires_at| expires_at > now)
            .unwrap_or(false)
    });
}

fn parse_expire_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

fn extract_bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
}

fn unauthorized(message: impl Into<String>) -> ApiResponse {
    response::<Value>(StatusCode::UNAUTHORIZED, false, message, None)
}

fn response<T>(
    status: StatusCode,
    success: bool,
    message: impl Into<String>,
    data: Option<T>,
) -> ApiResponse
where
    T: Serialize,
{
    (
        status,
        Json(json!({
            "success": success,
            "message": message.into(),
            "data": data
        })),
    )
}
