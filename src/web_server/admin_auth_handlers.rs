use axum::{
    Router,
    body::Body,
    extract::Json,
    http::{Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Once;
use uuid::Uuid;

use crate::web_server::admin_response;
use crate::web_server::wizard_handlers::open_deployment_sites_sqlite;

const SESSIONS_TABLE: &str = "admin_sessions";
const USERS_TABLE: &str = "admin_users";
const ADMIN_AUTH_UNAVAILABLE_MESSAGE: &str =
    "管理员凭据未配置，请先设置 ADMIN_USER 与 ADMIN_PASS";

static ADMIN_AUTH_LOG_ONCE: Once = Once::new();

fn ensure_auth_tables() {
    if let Ok(conn) = open_deployment_sites_sqlite() {
        let _ = conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {SESSIONS_TABLE} (
                token TEXT PRIMARY KEY,
                username TEXT NOT NULL,
                role TEXT NOT NULL DEFAULT 'admin',
                expires_at TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS {USERS_TABLE} (
                username TEXT PRIMARY KEY,
                password_hash TEXT NOT NULL,
                salt TEXT NOT NULL,
                role TEXT NOT NULL DEFAULT 'admin',
                created_at TEXT NOT NULL
            );"
        ));
    }
}

// TODO: migrate to argon2/bcrypt for production hardening (SHA256 is fast, brute-force vulnerable)
fn hash_password(password: &str, salt: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update(password.as_bytes());
    hasher.update(salt.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn verify_password(password: &str, salt: &str, stored_hash: &str) -> bool {
    hash_password(password, salt) == stored_hash
}

fn admin_credentials() -> Option<(String, String)> {
    let username = std::env::var("ADMIN_USER").ok()?;
    let password = std::env::var("ADMIN_PASS").ok()?;
    let username = username.trim().to_string();
    let password = password.trim().to_string();
    if username.is_empty() || password.is_empty() {
        return None;
    }
    Some((username, password))
}

fn admin_auth_configured() -> bool {
    admin_credentials().is_some()
}

fn log_admin_auth_unavailable() {
    ADMIN_AUTH_LOG_ONCE.call_once(|| {
        let mut missing = Vec::new();
        if std::env::var("ADMIN_USER")
            .ok()
            .map(|value| value.trim().is_empty())
            .unwrap_or(true)
        {
            missing.push("ADMIN_USER");
        }
        if std::env::var("ADMIN_PASS")
            .ok()
            .map(|value| value.trim().is_empty())
            .unwrap_or(true)
        {
            missing.push("ADMIN_PASS");
        }
        eprintln!(
            "⚠️ Admin 后台鉴权未启用：缺少环境变量 {}。访问 /api/admin/* 将返回 503，请先配置管理员凭据。",
            missing.join(", ")
        );
    });
}

fn ensure_default_admin() {
    let Some((admin_user, admin_pass)) = admin_credentials() else {
        log_admin_auth_unavailable();
        return;
    };

    let conn = match open_deployment_sites_sqlite() {
        Ok(c) => c,
        Err(err) => {
            eprintln!("⚠️ 初始化 admin 用户失败：无法打开 SQLite：{err}");
            return;
        }
    };

    let existing: Option<(String, String)> = conn
        .query_row(
            &format!("SELECT password_hash, salt FROM {USERS_TABLE} WHERE username = ?1"),
            rusqlite::params![admin_user],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .ok();

    match existing {
        Some((stored_hash, salt)) => {
            if !verify_password(&admin_pass, &salt, &stored_hash) {
                let new_salt = Uuid::new_v4().to_string();
                let new_hash = hash_password(&admin_pass, &new_salt);
                let _ = conn.execute(
                    &format!(
                        "UPDATE {USERS_TABLE} SET password_hash = ?1, salt = ?2 WHERE username = ?3"
                    ),
                    rusqlite::params![new_hash, new_salt, admin_user],
                );
                eprintln!("🔑 Admin 用户 {admin_user} 密码已按环境变量更新");
            }
        }
        None => {
            let salt = Uuid::new_v4().to_string();
            let password_hash = hash_password(&admin_pass, &salt);
            let now = chrono::Utc::now().to_rfc3339();
            let _ = conn.execute(
                &format!(
                    "INSERT INTO {USERS_TABLE} (username, password_hash, salt, role, created_at)
                     VALUES (?1, ?2, ?3, 'admin', ?4)"
                ),
                rusqlite::params![admin_user, password_hash, salt, now],
            );
        }
    }
}

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
    ensure_auth_tables();
    ensure_default_admin();
    cleanup_expired_sessions();

    Router::new()
        .route("/api/admin/auth/login", post(login))
        .route("/api/admin/auth/logout", post(logout))
        .route("/api/admin/auth/me", get(me))
}

pub fn validate_token(token: &str) -> Option<AdminSession> {
    if !admin_auth_configured() {
        return None;
    }

    let conn = open_deployment_sites_sqlite().ok()?;
    let mut stmt = conn
        .prepare(&format!(
            "SELECT token, username, role, expires_at FROM {SESSIONS_TABLE} WHERE token = ?1"
        ))
        .ok()?;

    let session = stmt
        .query_row(rusqlite::params![token], |row| {
            Ok(AdminSession {
                token: row.get(0)?,
                username: row.get(1)?,
                role: row.get(2)?,
                expires_at: row.get(3)?,
            })
        })
        .ok()?;

    if let Ok(expires) = chrono::DateTime::parse_from_rfc3339(&session.expires_at) {
        if chrono::Utc::now() > expires {
            let _ = conn.execute(
                &format!("DELETE FROM {SESSIONS_TABLE} WHERE token = ?1"),
                rusqlite::params![token],
            );
            return None;
        }
    }

    Some(session)
}

pub async fn admin_auth_middleware(request: Request<Body>, next: Next) -> Response {
    if !admin_auth_configured() {
        log_admin_auth_unavailable();
        return admin_response::service_unavailable(ADMIN_AUTH_UNAVAILABLE_MESSAGE).into_response();
    }

    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.trim().to_string());

    match token {
        Some(t) if validate_token(&t).is_some() => next.run(request).await,
        _ => admin_response::unauthorized("未授权访问，请先登录").into_response(),
    }
}

async fn login(Json(payload): Json<LoginRequest>) -> impl IntoResponse {
    if !admin_auth_configured() {
        log_admin_auth_unavailable();
        return admin_response::service_unavailable(ADMIN_AUTH_UNAVAILABLE_MESSAGE);
    }

    let conn = match open_deployment_sites_sqlite() {
        Ok(c) => c,
        Err(_) => return admin_response::server_error("数据库连接失败"),
    };

    let user_row = conn
        .query_row(
            &format!(
                "SELECT username, password_hash, salt, role FROM {USERS_TABLE} WHERE username = ?1"
            ),
            rusqlite::params![payload.username],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .ok();

    let (username, role) = match user_row {
        Some((u, stored_hash, salt, role)) => {
            if !verify_password(&payload.password, &salt, &stored_hash) {
                return admin_response::unauthorized("用户名或密码错误");
            }
            (u, role)
        }
        None => return admin_response::unauthorized("用户名或密码错误"),
    };

    let token = Uuid::new_v4().to_string();
    let now = chrono::Utc::now();
    let expires_at = now
        .checked_add_signed(chrono::Duration::hours(24))
        .unwrap_or(now)
        .to_rfc3339();

    let session = AdminSession {
        token: token.clone(),
        username: username.clone(),
        role: role.clone(),
        expires_at: expires_at.clone(),
    };

    let _ = conn.execute(
        &format!(
            "INSERT INTO {SESSIONS_TABLE} (token, username, role, expires_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)"
        ),
        rusqlite::params![token, username, session.role, expires_at, now.to_rfc3339()],
    );

    admin_response::ok(
        "登录成功",
        json!({
            "token": session.token,
            "user": {
                "username": session.username,
                "role": session.role,
            },
            "expires_at": session.expires_at,
        }),
    )
}

async fn logout(request: Request<Body>) -> impl IntoResponse {
    if !admin_auth_configured() {
        log_admin_auth_unavailable();
        return admin_response::service_unavailable(ADMIN_AUTH_UNAVAILABLE_MESSAGE);
    }

    if let Some(token) = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        if let Ok(conn) = open_deployment_sites_sqlite() {
            let _ = conn.execute(
                &format!("DELETE FROM {SESSIONS_TABLE} WHERE token = ?1"),
                rusqlite::params![token.trim()],
            );
        }
    }
    admin_response::response::<Value>(StatusCode::OK, true, "登出成功", None)
}

async fn me(request: Request<Body>) -> impl IntoResponse {
    if !admin_auth_configured() {
        log_admin_auth_unavailable();
        return admin_response::service_unavailable(ADMIN_AUTH_UNAVAILABLE_MESSAGE);
    }

    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.trim().to_string());

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

pub fn cleanup_expired_sessions() {
    if let Ok(conn) = open_deployment_sites_sqlite() {
        let now = chrono::Utc::now().to_rfc3339();
        let _ = conn.execute(
            &format!("DELETE FROM {SESSIONS_TABLE} WHERE expires_at < ?1"),
            rusqlite::params![now],
        );
    }
}

pub fn start_session_cleanup_timer() {
    tokio::spawn(async {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            cleanup_expired_sessions();
            crate::web_server::admin_task_handlers::cleanup_old_tasks();
        }
    });
}
