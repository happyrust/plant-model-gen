//! specs/022 候选4（方案 A）：增量状态 HTTP 面。
//!
//! 状态类端点是 Version Commit 存储（`sesno_version_anchor` / `version_commit_state`
//! / `dbnum_info_table`）之上的只读 adapter——与 CLI 共享同一事实源，不再返回 mock。
//! 动作类端点（触发检测/同步、任务、配置）从未有过实现，统一返回 501 并指引走
//! CLI `incremental-sesno` / `watch-incremental`；未来要回填时应接
//! `version_management::increment_run`（同一 IncrementRun seam），而非旁路实现。

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{fs, path::Path as FsPath, time::SystemTime};

use crate::web_server::AppState;
use aios_core::project_primary_db;
use surrealdb::types::SurrealValue;

#[derive(Debug, Clone, Serialize)]
pub struct ArchiveFile {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub modified: Option<String>,
    pub dbnum: Option<u32>,
    pub sesno: Option<u32>,
}

fn system_time_to_rfc3339(time: SystemTime) -> String {
    DateTime::<Utc>::from(time).to_rfc3339()
}

fn digit_runs(input: &str) -> Vec<u32> {
    let mut runs = Vec::new();
    let mut current = String::new();

    for ch in input.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else if !current.is_empty() {
            if let Ok(value) = current.parse::<u32>() {
                runs.push(value);
            }
            current.clear();
        }
    }

    if !current.is_empty() {
        if let Ok(value) = current.parse::<u32>() {
            runs.push(value);
        }
    }

    runs
}

fn infer_dbnum(file_stem: &str) -> Option<u32> {
    digit_runs(file_stem)
        .into_iter()
        .find(|value| *value >= 1000)
}

fn infer_sesno(file_stem: &str, dbnum: Option<u32>) -> Option<u32> {
    digit_runs(file_stem)
        .into_iter()
        .filter(|value| Some(*value) != dbnum)
        .next_back()
}

/// 列出本地已生成的 CBA 归档包，供 collab monitor 的归档页面展示与下载。
pub async fn list_incremental_archives() -> Result<Json<serde_json::Value>, StatusCode> {
    let archive_dir = FsPath::new("assets/archives");
    let mut files = Vec::new();

    if archive_dir.exists() {
        let entries = fs::read_dir(archive_dir).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let name = match path.file_name().and_then(|name| name.to_str()) {
                Some(name) if name.to_ascii_lowercase().ends_with(".cba") => name.to_string(),
                _ => continue,
            };
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            let stem = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("");
            let dbnum = infer_dbnum(stem);

            files.push(ArchiveFile {
                path: format!("/assets/archives/{}", name),
                name,
                size: metadata.len(),
                modified: metadata.modified().ok().map(system_time_to_rfc3339),
                dbnum,
                sesno: infer_sesno(stem, dbnum),
            });
        }
    }

    files.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.name.cmp(&b.name))
    });

    Ok(Json(json!({
        "success": true,
        "files": files,
    })))
}

/// 每库增量/版本状态（真实数据）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DbIncrementStatus {
    pub dbnum: u32,
    /// Committed Watermark：已发布 Version Anchor 的最高 sesno（0 = 无锚点）
    pub committed_watermark: u32,
    /// dbnum_info_table 记录级最大 sesno（存量/记账口径，Commit Pending 时可能领先锚点）
    pub legacy_max_sesno: u32,
    /// 最近一条锚点
    pub last_anchor_sesno: Option<u32>,
    pub last_anchored_at: Option<String>,
    pub last_anchor_source: Option<String>,
    /// 未恢复的 Commit Pending（阻塞该 dbnum 更高 sesno 提交）
    pub pending_commits: Vec<PendingCommitInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingCommitInfo {
    pub to_sesno: u32,
    pub status: String,
    pub last_error: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct AnchorAggRow {
    dbnum: u32,
    max_sesno: Option<u32>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct AnchorLatestRow {
    dbnum: u32,
    sesno: u32,
    anchored_at: Option<String>,
    source: Option<String>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct PendingRow {
    dbnum: u32,
    to_sesno: u32,
    status: String,
    last_error: Option<String>,
    updated_at: Option<String>,
}

fn statement_missing_table(error: &surrealdb::Error) -> bool {
    error.to_string().contains("does not exist")
}

async fn collect_db_increment_status() -> anyhow::Result<Vec<DbIncrementStatus>> {
    use std::collections::BTreeMap;

    let sql = r#"
SELECT dbnum, math::max(sesno) AS max_sesno FROM dbnum_info_table GROUP BY dbnum;
SELECT dbnum, math::max(sesno) AS max_sesno FROM sesno_version_anchor GROUP BY dbnum;
SELECT dbnum, sesno, type::string(anchored_at) AS anchored_at, source FROM sesno_version_anchor ORDER BY anchored_at DESC LIMIT 200;
SELECT dbnum, to_sesno, status, last_error, type::string(updated_at) AS updated_at FROM version_commit_state WHERE status IN ['preparing', 'pending'];
"#;
    let mut response = project_primary_db().query(sql).await?;

    // 表不存在（未启用锚点/从未解析）按空处理，其余错误上抛
    let legacy_rows: Vec<AnchorAggRow> = match response.take(0) {
        Ok(rows) => rows,
        Err(error) if statement_missing_table(&error) => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    let anchor_rows: Vec<AnchorAggRow> = match response.take(1) {
        Ok(rows) => rows,
        Err(error) if statement_missing_table(&error) => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    let latest_rows: Vec<AnchorLatestRow> = match response.take(2) {
        Ok(rows) => rows,
        Err(error) if statement_missing_table(&error) => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    let pending_rows: Vec<PendingRow> = match response.take(3) {
        Ok(rows) => rows,
        Err(error) if statement_missing_table(&error) => Vec::new(),
        Err(error) => return Err(error.into()),
    };

    let mut by_dbnum: BTreeMap<u32, DbIncrementStatus> = BTreeMap::new();
    for row in legacy_rows {
        let entry = by_dbnum.entry(row.dbnum).or_insert_with(|| DbIncrementStatus {
            dbnum: row.dbnum,
            ..Default::default()
        });
        entry.legacy_max_sesno = row.max_sesno.unwrap_or_default();
    }
    for row in anchor_rows {
        let entry = by_dbnum.entry(row.dbnum).or_insert_with(|| DbIncrementStatus {
            dbnum: row.dbnum,
            ..Default::default()
        });
        entry.committed_watermark = row.max_sesno.unwrap_or_default();
    }
    // Committed Watermark 语义：无锚点回退 legacy（与 committed_watermark() 一致）
    for status in by_dbnum.values_mut() {
        if status.committed_watermark == 0 {
            status.committed_watermark = status.legacy_max_sesno;
        }
    }
    for row in latest_rows {
        if let Some(entry) = by_dbnum.get_mut(&row.dbnum) {
            if entry.last_anchor_sesno.is_none() {
                entry.last_anchor_sesno = Some(row.sesno);
                entry.last_anchored_at = row.anchored_at;
                entry.last_anchor_source = row.source;
            }
        }
    }
    for row in pending_rows {
        if let Some(entry) = by_dbnum.get_mut(&row.dbnum) {
            entry.pending_commits.push(PendingCommitInfo {
                to_sesno: row.to_sesno,
                status: row.status,
                last_error: row.last_error,
                updated_at: row.updated_at,
            });
        }
    }

    Ok(by_dbnum.into_values().collect())
}

/// 全部 dbnum 的增量/版本状态（真实数据：锚点 + 水位 + Commit Pending）。
pub async fn get_all_incremental_status(_state: State<AppState>) -> impl IntoResponse {
    match collect_db_increment_status().await {
        Ok(databases) => {
            let pending_total: usize = databases
                .iter()
                .map(|status| status.pending_commits.len())
                .sum();
            (
                StatusCode::OK,
                Json(json!({
                    "success": true,
                    "databases": databases,
                    "pending_commit_total": pending_total,
                    "last_check": Utc::now(),
                    "source": "sesno_version_anchor + dbnum_info_table + version_commit_state",
                })),
            )
        }
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "success": false,
                "error": format!("查询增量状态失败: {error}"),
            })),
        ),
    }
}

#[derive(Debug, Deserialize, SurrealValue)]
struct AnchorDetailRow {
    sesno: u32,
    from_sesno: Option<u32>,
    source: Option<String>,
    anchored_at: Option<String>,
    fingerprint: Option<String>,
    pe_rows: Option<i64>,
    att_rows: Option<i64>,
    uda_rows: Option<i64>,
    delete_count: Option<i64>,
}

/// 单库增量详情：锚点时间线（最近 50 条）+ Commit Pending。
/// 路径参数为 dbnum（历史路由名为 site_id，语义即数据库编号）。
pub async fn get_site_incremental_details(
    _state: State<AppState>,
    Path(site_id): Path<String>,
) -> impl IntoResponse {
    let Ok(dbnum) = site_id.trim().parse::<u32>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "error": format!("site_id 需为 dbnum（数字），收到: {site_id}"),
            })),
        );
    };

    let sql = format!(
        "SELECT sesno, from_sesno, source, type::string(anchored_at) AS anchored_at, fingerprint, \
         pe_rows, att_rows, uda_rows, delete_count \
         FROM sesno_version_anchor WHERE dbnum = {dbnum} ORDER BY sesno DESC LIMIT 50;\n\
         SELECT dbnum, to_sesno, status, last_error, type::string(updated_at) AS updated_at \
         FROM version_commit_state WHERE dbnum = {dbnum} AND status IN ['preparing', 'pending'];"
    );
    let mut response = match project_primary_db().query(sql).await {
        Ok(response) => response,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "success": false,
                    "error": format!("查询 dbnum={dbnum} 增量详情失败: {error}"),
                })),
            );
        }
    };
    let anchors: Vec<AnchorDetailRow> = match response.take(0) {
        Ok(rows) => rows,
        Err(error) if statement_missing_table(&error) => Vec::new(),
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "success": false,
                    "error": format!("读取锚点失败: {error}"),
                })),
            );
        }
    };
    let pending: Vec<PendingRow> = match response.take(1) {
        Ok(rows) => rows,
        Err(error) if statement_missing_table(&error) => Vec::new(),
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "success": false,
                    "error": format!("读取 Commit Pending 失败: {error}"),
                })),
            );
        }
    };

    let anchors: Vec<serde_json::Value> = anchors
        .into_iter()
        .map(|row| {
            json!({
                "sesno": row.sesno,
                "from_sesno": row.from_sesno,
                "source": row.source,
                "anchored_at": row.anchored_at,
                "fingerprint": row.fingerprint,
                "counts": {
                    "pe_rows": row.pe_rows,
                    "att_rows": row.att_rows,
                    "uda_rows": row.uda_rows,
                    "delete_count": row.delete_count,
                },
            })
        })
        .collect();
    let pending: Vec<serde_json::Value> = pending
        .into_iter()
        .map(|row| {
            json!({
                "to_sesno": row.to_sesno,
                "status": row.status,
                "last_error": row.last_error,
                "updated_at": row.updated_at,
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "dbnum": dbnum,
            "anchors": anchors,
            "pending_commits": pending,
        })),
    )
}

fn not_implemented(action: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "success": false,
            "error": format!(
                "{action} 未实现：请使用 CLI incremental-sesno / watch-incremental（specs/022，写路径统一走 Version Commit seam）"
            ),
        })),
    )
}

/// 未实现：触发增量检测（走 CLI）。
pub async fn start_incremental_detection(
    _state: State<AppState>,
    Path(_site_id): Path<String>,
) -> impl IntoResponse {
    not_implemented("HTTP 触发增量检测")
}

/// 未实现：触发增量同步（走 CLI）。
pub async fn start_incremental_sync(
    _state: State<AppState>,
    Path(_site_id): Path<String>,
) -> impl IntoResponse {
    not_implemented("HTTP 触发增量同步")
}

/// 未实现：增量任务状态查询。
pub async fn get_detection_task_status(
    _state: State<AppState>,
    Path(_task_id): Path<String>,
) -> impl IntoResponse {
    not_implemented("增量任务状态查询")
}

/// 未实现：取消增量任务。
pub async fn cancel_task(
    _state: State<AppState>,
    Path(_task_id): Path<String>,
) -> impl IntoResponse {
    not_implemented("取消增量任务")
}

/// 未实现：增量配置读取。
pub async fn get_incremental_config(_state: State<AppState>) -> impl IntoResponse {
    not_implemented("增量配置读取")
}

/// 未实现：增量配置更新。
pub async fn update_incremental_config(
    _state: State<AppState>,
    Json(_config): Json<serde_json::Value>,
) -> impl IntoResponse {
    not_implemented("增量配置更新")
}
