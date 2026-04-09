use axum::{
    Router,
    extract::Json,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::LazyLock;
use std::collections::HashMap;
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

async fn logout() -> impl IntoResponse {
    response::<Value>(StatusCode::OK, true, "登出成功", None)
}

async fn me() -> impl IntoResponse {
    response(
        StatusCode::OK,
        true,
        "获取用户信息成功",
        Some(json!({
            "username": "admin",
            "role": "admin"
        })),
    )
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
