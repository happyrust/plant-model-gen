//! 接口 request/response 日志采集（spec 003-review-log-viewer T102）。
//!
//! 仅挂在 review/platform 域路由上（T103），单一事实源覆盖浏览器与 PMS S2S 调用。
//! 设计要点（spec 003 Decisions 3/4/8）：
//! - body 截断 4KB 并打 `truncated` 标记；
//! - 不存任何请求/响应头（Authorization/Cookie 天然不落地）；
//! - JSON body 中 token/password/secret 类字段值替换为 `***`；
//! - 写入走 `tokio::spawn` 异步执行，失败仅告警，绝不影响业务响应；
//! - 保留期 7 天，首条日志写入时惰性启动每小时一次的清理任务。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use serde::Serialize;
use serde_json::Value;

/// body 入库截断上限（spec 003 决策 4）。
const BODY_STORE_LIMIT: usize = 4 * 1024;
/// body 读取缓冲上限：超过即不尝试解析，仅记录长度（防大请求内存峰值）。
const BODY_READ_LIMIT: usize = 512 * 1024;
/// 保留期：7 天。
const RETENTION_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// 清理周期：1 小时。
const CLEANUP_INTERVAL_SECS: u64 = 60 * 60;

static CLEANUP_STARTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Serialize)]
struct ApiRequestLogRecord {
    request_id: String,
    method: String,
    path: String,
    query: Option<String>,
    status: u16,
    elapsed_ms: u64,
    req_body: Option<String>,
    req_truncated: bool,
    resp_body: Option<String>,
    resp_truncated: bool,
    form_id: Option<String>,
    task_id: Option<String>,
    created_at_ms: i64,
}

/// axum 中间件：采集一次请求/响应并异步落库。
pub async fn api_request_log_layer(request: Request, next: Next) -> Response {
    let started = Instant::now();
    let method = request.method().to_string();
    let path = request.uri().path().to_string();
    let query = request.uri().query().map(|q| q.to_string());

    // 缓冲请求 body 后重组 Request（review/platform 域均为 JSON 小请求）。
    let (parts, body) = request.into_parts();
    let req_bytes = match to_bytes(body, BODY_READ_LIMIT).await {
        Ok(bytes) => bytes,
        Err(error) => {
            log::warn!("[api_request_log] 请求 body 读取失败（跳过本条日志）: {error}");
            // body 已被消费且无法恢复，直接返回 400 以避免把残缺请求交给 handler。
            return Response::builder()
                .status(axum::http::StatusCode::BAD_REQUEST)
                .body(Body::from("request body read failed"))
                .unwrap_or_default();
        }
    };
    let request = Request::from_parts(parts, Body::from(req_bytes.clone()));

    let response = next.run(request).await;

    // 缓冲响应 body 后重组 Response。
    let (parts, body) = response.into_parts();
    let resp_bytes = match to_bytes(body, BODY_READ_LIMIT).await {
        Ok(bytes) => bytes,
        Err(error) => {
            log::warn!("[api_request_log] 响应 body 读取失败（响应无法恢复）: {error}");
            return Response::builder()
                .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("response body buffer failed"))
                .unwrap_or_default();
        }
    };
    let response = Response::from_parts(parts, Body::from(resp_bytes.clone()));

    let elapsed_ms = started.elapsed().as_millis() as u64;
    let status = response.status().as_u16();

    let (req_body, req_truncated) = sanitize_body(&req_bytes);
    let (resp_body, resp_truncated) = sanitize_body(&resp_bytes);
    let (form_id, task_id) = extract_correlation(&path, query.as_deref(), &req_bytes);

    let record = ApiRequestLogRecord {
        request_id: uuid::Uuid::new_v4().to_string(),
        method,
        path,
        query,
        status,
        elapsed_ms,
        req_body,
        req_truncated,
        resp_body,
        resp_truncated,
        form_id,
        task_id,
        created_at_ms: chrono::Utc::now().timestamp_millis(),
    };

    tokio::spawn(async move {
        if let Err(error) = write_record(record).await {
            log::warn!("[api_request_log] 日志写入失败（业务不受影响）: {error}");
        }
    });
    maybe_start_cleanup_task();

    response
}

async fn write_record(record: ApiRequestLogRecord) -> anyhow::Result<()> {
    let db = crate::web_api::review_db::review_db_session().await?;
    crate::web_api::review_db::await_review_query(
        "api_request_log.write",
        db.query("CREATE api_request_log CONTENT $data")
            .bind(("data", serde_json::to_value(&record)?)),
    )
    .await?;
    Ok(())
}

/// 首条日志触发后启动保留期清理循环（每小时删除 7 天前的记录）。
fn maybe_start_cleanup_task() {
    if CLEANUP_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::spawn(async {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(CLEANUP_INTERVAL_SECS)).await;
            let cutoff = chrono::Utc::now().timestamp_millis() - RETENTION_MS;
            let result: anyhow::Result<()> = async {
                let db = crate::web_api::review_db::review_db_session().await?;
                crate::web_api::review_db::await_review_query_long(
                    "api_request_log.cleanup",
                    db.query("DELETE api_request_log WHERE created_at_ms < $cutoff")
                        .bind(("cutoff", cutoff)),
                )
                .await?;
                Ok(())
            }
            .await;
            if let Err(error) = result {
                log::warn!("[api_request_log] 保留期清理失败（下轮重试）: {error}");
            }
        }
    });
}

/// 截断 + 脱敏：JSON 走字段级打码，非 JSON 仅截断存文本。
fn sanitize_body(bytes: &[u8]) -> (Option<String>, bool) {
    if bytes.is_empty() {
        return (None, false);
    }
    if let Ok(mut json) = serde_json::from_slice::<Value>(bytes) {
        mask_sensitive_fields(&mut json);
        let text = json.to_string();
        let truncated = text.len() > BODY_STORE_LIMIT;
        let stored = if truncated {
            text.chars().take(BODY_STORE_LIMIT).collect()
        } else {
            text
        };
        return (Some(stored), truncated);
    }
    let text = String::from_utf8_lossy(bytes);
    let truncated = text.len() > BODY_STORE_LIMIT;
    let stored: String = text.chars().take(BODY_STORE_LIMIT).collect();
    (Some(stored), truncated)
}

fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    lower.contains("token") || lower.contains("password") || lower.contains("secret")
}

fn mask_sensitive_fields(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if is_sensitive_key(key) {
                    *child = Value::String("***".to_string());
                } else {
                    mask_sensitive_fields(child);
                }
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                mask_sensitive_fields(item);
            }
        }
        _ => {}
    }
}

/// 从 query / path / JSON body 尽力提取 form_id 与 task_id（排障关联键）。
fn extract_correlation(
    path: &str,
    query: Option<&str>,
    req_bytes: &[u8],
) -> (Option<String>, Option<String>) {
    let mut form_id = None;
    let mut task_id = None;

    if let Some(query) = query {
        for pair in query.split('&') {
            let mut kv = pair.splitn(2, '=');
            let key = kv.next().unwrap_or_default();
            let value = kv.next().unwrap_or_default();
            if value.is_empty() {
                continue;
            }
            match key {
                "form_id" | "formId" => form_id = Some(value.to_string()),
                "task_id" | "taskId" => task_id = Some(value.to_string()),
                _ => {}
            }
        }
    }

    // /api/review/tasks/{id}/... 的路径参数视作 task_id。
    if task_id.is_none() {
        if let Some(rest) = path.strip_prefix("/api/review/tasks/") {
            let id = rest.split('/').next().unwrap_or_default();
            if !id.is_empty() {
                task_id = Some(id.to_string());
            }
        }
    }

    if (form_id.is_none() || task_id.is_none()) && !req_bytes.is_empty() {
        if let Ok(json) = serde_json::from_slice::<Value>(req_bytes) {
            if form_id.is_none() {
                form_id = pick_string(&json, &["form_id", "formId"]);
            }
            if task_id.is_none() {
                task_id = pick_string(&json, &["task_id", "taskId"]);
            }
        }
    }

    (form_id, task_id)
}

fn pick_string(json: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = json.get(*key).and_then(|v| v.as_str()) {
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}
