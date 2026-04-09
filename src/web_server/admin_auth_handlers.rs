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

use crate::web_server::admin_response;

pub static SESSIONS: LazyLock<Mutex<HashMap<String, AdminSession>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Clone, serde::Serialize)]
pub struct AdminSession {
    pub token: String,
    pub username: String,
    pub role: String,
    pub expires_at: String,
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

pub fn validate_token(token: &str) -> Option<AdminSession> {
    let sessions = SESSIONS.lock().ok()?;
    let session = sessions.get(token)?;
    let expires = DateTime::parse_from_rfc3339(&session.expires_at).ok()?;
    if Utc::now() > expires {
        return None;
    }
    Some(session.clone())
}

pub async fn admin_session_middleware(
    request: Request,
    next: Next,
) -> Result<Response, impl IntoResponse> {
    if request.method() == Method::OPTIONS {
        return Ok(next.run(request).await);
    }
    let token = extract_bearer_token(request.headers());
    match token.and_then(|t| validate_token(t)) {
        Some(_) => Ok(next.run(request).await),
        None => Err(admin_response::unauthorized("未授权访问，请先登录")),
    }
}

pub async fn admin_auth_middleware(request: Request, next: Next) -> Response {
    let token = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.trim().to_string());

    match token.as_deref().and_then(validate_token) {
        Some(_) => next.run(request).await,
        None => admin_response::unauthorized("未授权访问，请先登录").into_response(),
    }
}

async fn login(Json(payload): Json<LoginRequest>) -> impl IntoResponse {
    let admin_user = std::env::var("ADMIN_USER").unwrap_or_else(|_| "admin".to_string());
    let admin_pass = std::env::var("ADMIN_PASS").unwrap_or_else(|_| "admin".to_string());

    if payload.username != admin_user || payload.password != admin_pass {
        return admin_response::unauthorized("用户名或密码错误");
    }

    let token = Uuid::new_v4().to_string();
    let session = AdminSession {
        token: token.clone(),
        username: payload.username.clone(),
        role: "admin".to_string(),
        expires_at: chrono::Utc::now()
            .checked_add_signed(chrono::Duration::hours(24))
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339(),
    };

    if let Ok(mut sessions) = SESSIONS.lock() {
        sessions.insert(token, session.clone());
    }

    admin_response::ok("登录成功", session)
}

async fn logout(headers: HeaderMap) -> impl IntoResponse {
    if let Some(token) = extract_bearer_token(&headers) {
        if let Ok(mut sessions) = SESSIONS.lock() {
            sessions.remove(token);
        }
    }
    admin_response::response::<Value>(StatusCode::OK, true, "登出成功", None)
}

async fn me(headers: HeaderMap) -> impl IntoResponse {
    let token = extract_bearer_token(&headers).map(|t| t.to_string());

    match token.and_then(|t| validate_token(&t)) {
        Some(session) => admin_response::ok(
            "获取用户信息成功",
            json!({
                "username": session.username,
                "role": session.role
            }),
        ),
        None => admin_response::unauthorized("未登录或token已过期"),
    }
}

fn extract_bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
}
