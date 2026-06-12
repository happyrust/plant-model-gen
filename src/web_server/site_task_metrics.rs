//! 站点部署任务级性能指标：产物入库 + REST API（spec 004-site-deploy-perf-stats）。
//!
//! sidecar 在 `runtime/admin_sites/<site_id>/metrics/<task_id>.json` 落指标产物
//! （见 `crate::perf_metrics`）；本模块在 sidecar job 完成钩子读取产物 →
//! `site_task_metrics`（admin SQLite）入库 → 每站点保留最近 50 条 → 提供查询 API。
//! web_server 不解析日志、不读 E3D 数据，只消费 sidecar 产物文件。

use std::path::PathBuf;

use axum::Json;
use axum::extract::{Path as AxumPath, Query};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::perf_metrics::{TASK_METRICS_PATH_ENV, TASK_METRICS_SCHEMA_VERSION};
use crate::web_server::admin_response;
use crate::web_server::wizard_handlers::open_deployment_sites_sqlite;

/// 每站点保留的指标行数上限。
pub const METRICS_RETAIN_PER_SITE: usize = 50;

const TABLE: &str = "site_task_metrics";

// ─── 产物路径与 env 注入 ─────────────────────────────────────────────────────

/// 站点指标产物目录：`runtime/admin_sites/<site_id>/metrics`。
pub fn metrics_dir(site_id: &str) -> PathBuf {
    PathBuf::from("runtime/admin_sites")
        .join(site_id)
        .join("metrics")
}

/// 指标产物文件路径（文件名 stem 即 task_id）。
pub fn metrics_file_path(site_id: &str, task_id: &str) -> PathBuf {
    metrics_dir(site_id).join(format!("{task_id}.json"))
}

/// 为 sidecar CLI job 构造指标采集 env（路径 + 任务类型）。
pub fn metrics_env(site_id: &str, task_id: &str, job_kind: &str) -> Vec<(String, String)> {
    vec![
        (
            TASK_METRICS_PATH_ENV.to_string(),
            metrics_file_path(site_id, task_id)
                .to_string_lossy()
                .to_string(),
        ),
        (
            crate::perf_metrics::TASK_METRICS_KIND_ENV.to_string(),
            job_kind.to_string(),
        ),
    ]
}

/// 生成一个对人类可读、站内唯一的 task_id（kind + 本地时间戳）。
pub fn new_metrics_task_id(job_kind: &str) -> String {
    format!(
        "{}-{}",
        job_kind,
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    )
}

// ─── SQLite 入库 ─────────────────────────────────────────────────────────────

fn ensure_metrics_schema(conn: &rusqlite::Connection) -> anyhow::Result<()> {
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS {TABLE} (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            site_id TEXT NOT NULL,
            task_id TEXT NOT NULL UNIQUE,
            job_kind TEXT NOT NULL,
            started_at TEXT NOT NULL,
            finished_at TEXT,
            duration_ms INTEGER,
            success INTEGER NOT NULL DEFAULT 0,
            stages_json TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_stm_site_time ON {TABLE}(site_id, started_at DESC);"
    ))?;
    Ok(())
}

/// sidecar job 完成钩子：读取产物文件入库；缺失/损坏只告警不阻塞任务状态流转。
pub fn ingest_task_metrics(site_id: &str, task_id: &str, job_success: bool) {
    let path = metrics_file_path(site_id, task_id);
    let content = match std::fs::read_to_string(&path) {
        Ok(v) => v,
        Err(err) => {
            tracing::warn!(
                site = %site_id, task = %task_id,
                "任务指标产物缺失（metrics_missing）：{}: {err}",
                path.display()
            );
            return;
        }
    };
    let file: Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(err) => {
            tracing::warn!(site = %site_id, task = %task_id, "任务指标产物解析失败: {err}");
            return;
        }
    };
    if file.get("schema_version").and_then(Value::as_u64) != Some(TASK_METRICS_SCHEMA_VERSION as u64)
    {
        tracing::warn!(site = %site_id, task = %task_id, "任务指标 schema_version 不匹配，跳过入库");
        return;
    }

    let job_kind = file
        .get("job_kind")
        .and_then(Value::as_str)
        .unwrap_or("parse")
        .to_string();
    let started_at = file
        .get("started_at")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let finished_at = file
        .get("finished_at")
        .and_then(Value::as_str)
        .map(|v| v.to_string());
    let duration_ms = file.get("duration_ms").and_then(Value::as_u64).unwrap_or(0) as i64;
    // 产物内 success 可能因进程被杀缺失；以 job 终态兜底。
    let success = file
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(job_success);
    let stages_json = file
        .get("stages")
        .cloned()
        .unwrap_or_else(|| json!({}))
        .to_string();

    let conn = match open_deployment_sites_sqlite() {
        Ok(c) => c,
        Err(err) => {
            tracing::warn!("打开 admin SQLite 失败，任务指标未入库: {err}");
            return;
        }
    };
    if let Err(err) = ensure_metrics_schema(&conn) {
        tracing::warn!("site_task_metrics 建表失败: {err}");
        return;
    }
    let insert = conn.execute(
        &format!(
            "INSERT INTO {TABLE} (site_id, task_id, job_kind, started_at, finished_at, duration_ms, success, stages_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(task_id) DO UPDATE SET
                finished_at = excluded.finished_at,
                duration_ms = excluded.duration_ms,
                success = excluded.success,
                stages_json = excluded.stages_json"
        ),
        rusqlite::params![
            site_id,
            task_id,
            job_kind,
            started_at,
            finished_at,
            duration_ms,
            if success { 1i64 } else { 0i64 },
            stages_json,
            chrono::Utc::now().to_rfc3339(),
        ],
    );
    match insert {
        Ok(_) => {
            tracing::info!(site = %site_id, task = %task_id, kind = %job_kind, "任务指标已入库");
            // 每站点按 started_at 保留最近 N 条。
            let _ = conn.execute(
                &format!(
                    "DELETE FROM {TABLE} WHERE site_id = ?1 AND id NOT IN (
                        SELECT id FROM {TABLE} WHERE site_id = ?1
                        ORDER BY started_at DESC LIMIT {METRICS_RETAIN_PER_SITE}
                    )"
                ),
                rusqlite::params![site_id],
            );
        }
        Err(err) => tracing::warn!(site = %site_id, task = %task_id, "任务指标入库失败: {err}"),
    }
}

// ─── 查询 API ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct MetricsListQuery {
    pub limit: Option<usize>,
}

#[derive(Debug)]
struct MetricsRow {
    task_id: String,
    job_kind: String,
    started_at: String,
    finished_at: Option<String>,
    duration_ms: i64,
    success: bool,
    stages_json: String,
}

fn row_to_json(row: &MetricsRow) -> Value {
    json!({
        "task_id": row.task_id,
        "job_kind": row.job_kind,
        "started_at": row.started_at,
        "finished_at": row.finished_at,
        "duration_ms": row.duration_ms,
        "success": row.success,
        "stages": serde_json::from_str::<Value>(&row.stages_json).unwrap_or_else(|_| json!({})),
    })
}

fn load_rows(site_id: &str, limit: usize) -> anyhow::Result<Vec<MetricsRow>> {
    let conn = open_deployment_sites_sqlite()
        .map_err(|e| anyhow::anyhow!("打开 admin SQLite 失败: {e}"))?;
    ensure_metrics_schema(&conn)?;
    let mut stmt = conn.prepare(&format!(
        "SELECT task_id, job_kind, started_at, finished_at, duration_ms, success, stages_json
         FROM {TABLE} WHERE site_id = ?1 ORDER BY started_at DESC LIMIT ?2"
    ))?;
    let rows = stmt
        .query_map(rusqlite::params![site_id, limit as i64], |row| {
            Ok(MetricsRow {
                task_id: row.get(0)?,
                job_kind: row.get(1)?,
                started_at: row.get(2)?,
                finished_at: row.get(3)?,
                duration_ms: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
                success: row.get::<_, i64>(5)? != 0,
                stages_json: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// 关键数量提取（delta 对比用）：总元素数 / inst_relate / 闭包 visited。
fn key_numbers(stages: &Value) -> (i64, i64, i64) {
    let total_elements = stages
        .pointer("/parse/total_elements")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let inst_relate = stages
        .pointer("/generate/inst_relate")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let visited = stages
        .pointer("/closure/visited_count")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    (total_elements, inst_relate, visited)
}

async fn list_site_metrics(
    AxumPath(site_id): AxumPath<String>,
    Query(query): Query<MetricsListQuery>,
) -> impl IntoResponse {
    let limit = query.limit.unwrap_or(10).clamp(1, METRICS_RETAIN_PER_SITE);
    // delta 需要看到比 limit 多一条同类任务，所以放大读取窗口。
    let rows = match load_rows(&site_id, METRICS_RETAIN_PER_SITE) {
        Ok(rows) => rows,
        Err(err) => return admin_response::server_error(format!("查询任务指标失败: {err}")),
    };

    let mut items = Vec::new();
    for (idx, row) in rows.iter().enumerate().take(limit) {
        let mut item = row_to_json(row);
        // 同 kind 的上一条（时间序更早）做 delta。
        if let Some(prev) = rows
            .iter()
            .skip(idx + 1)
            .find(|candidate| candidate.job_kind == row.job_kind)
        {
            let cur_stages: Value =
                serde_json::from_str(&row.stages_json).unwrap_or_else(|_| json!({}));
            let prev_stages: Value =
                serde_json::from_str(&prev.stages_json).unwrap_or_else(|_| json!({}));
            let (cur_elements, cur_inst, cur_visited) = key_numbers(&cur_stages);
            let (prev_elements, prev_inst, prev_visited) = key_numbers(&prev_stages);
            item["delta"] = json!({
                "prev_task_id": prev.task_id,
                "duration_ms": row.duration_ms - prev.duration_ms,
                "total_elements": cur_elements - prev_elements,
                "inst_relate": cur_inst - prev_inst,
                "closure_visited": cur_visited - prev_visited,
            });
        }
        items.push(item);
    }

    admin_response::ok("查询任务指标成功", json!({ "items": items }))
}

async fn get_site_metrics_detail(
    AxumPath((site_id, task_id)): AxumPath<(String, String)>,
) -> impl IntoResponse {
    let rows = match load_rows(&site_id, METRICS_RETAIN_PER_SITE) {
        Ok(rows) => rows,
        Err(err) => return admin_response::server_error(format!("查询任务指标失败: {err}")),
    };
    match rows.iter().find(|row| row.task_id == task_id) {
        Some(row) => admin_response::ok("查询任务指标成功", row_to_json(row)),
        None => admin_response::not_found(format!("任务指标不存在: {task_id}")),
    }
}

/// 指标查询路由（挂在 admin 鉴权层内）。
pub fn create_site_metrics_routes() -> Router {
    Router::new()
        .route("/api/admin/sites/{id}/metrics", get(list_site_metrics))
        .route(
            "/api/admin/sites/{id}/metrics/{task_id}",
            get(get_site_metrics_detail),
        )
}
