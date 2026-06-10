//! 统一日志查询 API（spec 003-review-log-viewer T105）。
//!
//! 契约（spec 003 Decisions 5/7）：
//! - `GET /api/logs/types`：按角色裁剪的类型目录树；
//! - `GET /api/logs?type=&form_id=&task_id=&site_id=&level=&q=&from_ms=&to_ms=&cursor=&limit=`：
//!   统一 `LogEntry` 分页查询，按 `type` 分派到三种数据源 adapter：
//!   - `api.request`        → SurrealDB `api_request_log` 表（T102 采集）；
//!   - `review.workflow`    → SurrealDB `review_workflow_history` 表（既有）；
//!   - `site.file.{kind}`   → 站点文件日志 tail（既有 `managed_project_sites::tail_log`）。
//! - 鉴权：review JWT；`api.request` / `site.file.*` 仅 admin 角色可见。

use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Extension, Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::web_api::jwt_auth::TokenClaims;
use crate::web_api::review_db::{await_review_query, review_db_session};

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 200;
const SITE_FILE_KINDS: [&str; 5] = ["parse", "generate", "db", "web", "viewer"];

#[derive(Debug, Serialize)]
struct LogTypeInfo {
    id: String,
    name: String,
    filters: Vec<&'static str>,
    admin_only: bool,
}

#[derive(Debug, Default, Deserialize)]
pub struct LogQuery {
    #[serde(rename = "type")]
    pub log_type: Option<String>,
    pub form_id: Option<String>,
    pub task_id: Option<String>,
    pub site_id: Option<String>,
    pub level: Option<String>,
    pub q: Option<String>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
struct LogCorrelation {
    #[serde(skip_serializing_if = "Option::is_none")]
    form_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    site_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct LogEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    ts_ms: Option<i64>,
    #[serde(rename = "type")]
    log_type: String,
    level: String,
    summary: String,
    detail: Value,
    correlation: LogCorrelation,
}

fn is_admin(claims: &TokenClaims) -> bool {
    claims
        .role
        .as_deref()
        .map(|role| role.eq_ignore_ascii_case("admin"))
        .unwrap_or(false)
}

fn log_type_catalog(admin: bool) -> Vec<LogTypeInfo> {
    let mut types = vec![LogTypeInfo {
        id: "review.workflow".to_string(),
        name: "校审流转历史".to_string(),
        filters: vec!["form_id", "task_id"],
        admin_only: false,
    }];
    if admin {
        types.push(LogTypeInfo {
            id: "api.request".to_string(),
            name: "接口日志（request/response）".to_string(),
            filters: vec!["form_id", "task_id", "level", "q", "from_ms", "to_ms"],
            admin_only: true,
        });
        for kind in SITE_FILE_KINDS {
            types.push(LogTypeInfo {
                id: format!("site.file.{kind}"),
                name: format!("站点日志·{kind}"),
                filters: vec!["site_id", "level", "q"],
                admin_only: true,
            });
        }
    }
    types
}

pub fn create_logs_api_routes() -> Router {
    use crate::web_api::jwt_auth::{REVIEW_AUTH_CONFIG, review_auth_middleware};
    use axum::middleware;

    Router::new()
        .route("/api/logs/types", get(get_log_types))
        .route("/api/logs", get(get_logs))
        .layer(middleware::from_fn_with_state(
            REVIEW_AUTH_CONFIG.clone(),
            review_auth_middleware,
        ))
}

async fn get_log_types(Extension(claims): Extension<TokenClaims>) -> impl IntoResponse {
    let types = log_type_catalog(is_admin(&claims));
    Json(json!({ "success": true, "types": types }))
}

async fn get_logs(
    Extension(claims): Extension<TokenClaims>,
    Query(query): Query<LogQuery>,
) -> impl IntoResponse {
    let Some(log_type) = query.log_type.clone().filter(|t| !t.is_empty()) else {
        return error_response(StatusCode::BAD_REQUEST, "缺少必填参数 type");
    };

    let admin = is_admin(&claims);
    let admin_only = log_type != "review.workflow";
    if admin_only && !admin {
        return error_response(StatusCode::FORBIDDEN, "当前角色无权查看该类型日志");
    }

    let result = match log_type.as_str() {
        "api.request" => query_api_request_logs(&query).await,
        "review.workflow" => query_workflow_logs(&query).await,
        other => match other.strip_prefix("site.file.") {
            Some(kind) if SITE_FILE_KINDS.contains(&kind) => {
                query_site_file_logs(kind, &query).await
            }
            _ => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    &format!("未知日志类型: {log_type}"),
                );
            }
        },
    };

    match result {
        Ok((entries, next_cursor)) => Json(json!({
            "success": true,
            "type": log_type,
            "entries": entries,
            "next_cursor": next_cursor,
        }))
        .into_response(),
        Err(error) => {
            log::warn!("[logs_api] 查询失败 type={log_type}: {error}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("{error}"))
        }
    }
}

fn error_response(status: StatusCode, message: &str) -> axum::response::Response {
    (
        status,
        Json(json!({ "success": false, "message": message })),
    )
        .into_response()
}

fn effective_limit(query: &LogQuery) -> usize {
    query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

// ─── adapter: api.request ────────────────────────────────────────────────────

async fn query_api_request_logs(
    query: &LogQuery,
) -> anyhow::Result<(Vec<LogEntry>, Option<String>)> {
    let limit = effective_limit(query);
    let mut conditions: Vec<&str> = Vec::new();
    if query.form_id.is_some() {
        conditions.push("form_id = $form_id");
    }
    if query.task_id.is_some() {
        conditions.push("task_id = $task_id");
    }
    if query.from_ms.is_some() {
        conditions.push("created_at_ms >= $from_ms");
    }
    if query.to_ms.is_some() {
        conditions.push("created_at_ms <= $to_ms");
    }
    let cursor_ms = query.cursor.as_deref().and_then(|c| c.parse::<i64>().ok());
    if cursor_ms.is_some() {
        conditions.push("created_at_ms < $cursor_ms");
    }
    match query.level.as_deref() {
        Some("error") => conditions.push("status >= 500"),
        Some("warn") => conditions.push("status >= 400"),
        _ => {}
    }
    if query.q.is_some() {
        conditions.push("string::contains(path, $q)");
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };
    let sql = format!(
        "SELECT * FROM api_request_log {where_clause} ORDER BY created_at_ms DESC LIMIT $limit"
    );

    let db = review_db_session().await?;
    let mut request = db.query(sql).bind(("limit", limit as i64));
    if let Some(form_id) = query.form_id.clone() {
        request = request.bind(("form_id", form_id));
    }
    if let Some(task_id) = query.task_id.clone() {
        request = request.bind(("task_id", task_id));
    }
    if let Some(from_ms) = query.from_ms {
        request = request.bind(("from_ms", from_ms));
    }
    if let Some(to_ms) = query.to_ms {
        request = request.bind(("to_ms", to_ms));
    }
    if let Some(cursor_ms) = cursor_ms {
        request = request.bind(("cursor_ms", cursor_ms));
    }
    if let Some(q) = query.q.clone() {
        request = request.bind(("q", q));
    }

    let mut response = await_review_query("logs.api_request", request).await?;
    let rows: Vec<Value> = response.take(0).unwrap_or_default();

    let entries: Vec<LogEntry> = rows.into_iter().map(api_request_row_to_entry).collect();
    let next_cursor = if entries.len() == limit {
        entries
            .last()
            .and_then(|entry| entry.ts_ms)
            .map(|ts| ts.to_string())
    } else {
        None
    };
    Ok((entries, next_cursor))
}

fn api_request_row_to_entry(row: Value) -> LogEntry {
    let status = row.get("status").and_then(Value::as_u64).unwrap_or(0);
    let level = if status >= 500 {
        "error"
    } else if status >= 400 {
        "warn"
    } else {
        "info"
    };
    let method = row.get("method").and_then(Value::as_str).unwrap_or("?");
    let path = row.get("path").and_then(Value::as_str).unwrap_or("?");
    let elapsed = row.get("elapsed_ms").and_then(Value::as_u64).unwrap_or(0);
    LogEntry {
        ts_ms: row.get("created_at_ms").and_then(Value::as_i64),
        log_type: "api.request".to_string(),
        level: level.to_string(),
        summary: format!("{method} {path} → {status} ({elapsed}ms)"),
        correlation: LogCorrelation {
            form_id: value_string(&row, "form_id"),
            task_id: value_string(&row, "task_id"),
            site_id: None,
            request_id: value_string(&row, "request_id"),
        },
        detail: row,
    }
}

// ─── adapter: review.workflow ────────────────────────────────────────────────

async fn query_workflow_logs(query: &LogQuery) -> anyhow::Result<(Vec<LogEntry>, Option<String>)> {
    let limit = effective_limit(query);
    let offset = query
        .cursor
        .as_deref()
        .and_then(|c| c.parse::<usize>().ok())
        .unwrap_or(0);

    let mut conditions: Vec<&str> = Vec::new();
    if query.form_id.is_some() {
        conditions.push("form_id = $form_id");
    }
    if query.task_id.is_some() {
        conditions.push("task_id = $task_id");
    }
    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };
    let sql = format!(
        "SELECT * FROM review_workflow_history {where_clause} ORDER BY timestamp DESC LIMIT $limit START $offset"
    );

    let db = review_db_session().await?;
    let mut request = db
        .query(sql)
        .bind(("limit", limit as i64))
        .bind(("offset", offset as i64));
    if let Some(form_id) = query.form_id.clone() {
        request = request.bind(("form_id", form_id));
    }
    if let Some(task_id) = query.task_id.clone() {
        request = request.bind(("task_id", task_id));
    }

    let mut response = await_review_query("logs.review_workflow", request).await?;
    let rows: Vec<Value> = response.take(0).unwrap_or_default();

    let row_count = rows.len();
    let entries: Vec<LogEntry> = rows.into_iter().map(workflow_row_to_entry).collect();
    let next_cursor = if row_count == limit {
        Some((offset + row_count).to_string())
    } else {
        None
    };
    Ok((entries, next_cursor))
}

fn workflow_row_to_entry(row: Value) -> LogEntry {
    let node = row.get("node").and_then(Value::as_str).unwrap_or("?");
    let action = row.get("action").and_then(Value::as_str).unwrap_or("?");
    let actor = value_string(&row, "actor_name")
        .or_else(|| value_string(&row, "operator_name"))
        .unwrap_or_else(|| "?".to_string());
    let ts_ms = ["timestamp", "created_at"]
        .iter()
        .find_map(|key| row.get(*key).and_then(Value::as_str))
        .and_then(parse_rfc3339_ms);
    LogEntry {
        ts_ms,
        log_type: "review.workflow".to_string(),
        level: "info".to_string(),
        summary: format!("[{node}] {action} by {actor}"),
        correlation: LogCorrelation {
            form_id: value_string(&row, "form_id"),
            task_id: value_string(&row, "task_id"),
            site_id: None,
            request_id: None,
        },
        detail: row,
    }
}

// ─── adapter: site.file.{kind} ───────────────────────────────────────────────

async fn query_site_file_logs(
    kind: &str,
    query: &LogQuery,
) -> anyhow::Result<(Vec<LogEntry>, Option<String>)> {
    let Some(site_id) = query.site_id.clone().filter(|s| !s.is_empty()) else {
        anyhow::bail!("site.file.* 类型必须携带 site_id 参数");
    };
    let limit = effective_limit(query);
    let kind_owned = kind.to_string();

    // tail 会整文件读入，放 spawn_blocking 防大文件阻塞 runtime。
    let tail = tokio::task::spawn_blocking({
        let site_id = site_id.clone();
        let kind = kind_owned.clone();
        move || crate::web_server::managed_project_sites::tail_log(&site_id, &kind, 2000)
    })
    .await??;

    let level_filter = query.level.clone().map(|l| l.to_lowercase());
    let q_filter = query.q.clone();
    let entries: Vec<LogEntry> = tail
        .lines
        .into_iter()
        .rev() // 最新在前，与其它 adapter 一致
        .filter(|line| match level_filter.as_deref() {
            Some("error") => line.to_lowercase().contains("error"),
            Some("warn") => {
                let lower = line.to_lowercase();
                lower.contains("warn") || lower.contains("error")
            }
            _ => true,
        })
        .filter(|line| {
            q_filter
                .as_deref()
                .map(|q| line.contains(q))
                .unwrap_or(true)
        })
        .take(limit)
        .map(|line| {
            let lower = line.to_lowercase();
            let level = if lower.contains("error") {
                "error"
            } else if lower.contains("warn") {
                "warn"
            } else {
                "info"
            };
            LogEntry {
                ts_ms: None,
                log_type: format!("site.file.{kind_owned}"),
                level: level.to_string(),
                summary: line.clone(),
                detail: Value::String(line),
                correlation: LogCorrelation {
                    form_id: None,
                    task_id: None,
                    site_id: Some(site_id.clone()),
                    request_id: None,
                },
            }
        })
        .collect();

    // tail 语义不支持 cursor 翻页（spec 003 plan 已声明此限制）。
    Ok((entries, None))
}

// ─── helpers ─────────────────────────────────────────────────────────────────

fn value_string(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn parse_rfc3339_ms(text: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|dt| dt.timestamp_millis())
}
