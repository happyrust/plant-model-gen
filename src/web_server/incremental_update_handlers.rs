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
use dashmap::DashMap;
use once_cell::sync::Lazy;
use surrealdb::types::SurrealValue;

use crate::versioned_db::version_commit::committed_watermark;

/// specs/022 候选4·方案B：HTTP 触发的增量运行注册表（内存态）。
/// sync = 真实落库（persist），detect = 只读试跑（no-persist）；两者都走
/// `version_management::increment_run::run_increment`（同一 IncrementRun /
/// Version Commit seam）。写侧安全由 commit_version 的 lease + Commit Pending
/// + 锚点固化兜底。进程重启后注册表清空（运行记录不持久，锚点才是权威）。
static INCREMENT_RUNS: Lazy<DashMap<String, IncrementRunStatus>> = Lazy::new(DashMap::new);

#[derive(Debug, Clone, Serialize)]
pub struct IncrementRunStatus {
    pub run_id: String,
    pub dbnum: u32,
    /// "sync"（落库）| "detect"（只读试跑）
    pub kind: String,
    /// "running" | "succeeded" | "failed"
    pub state: String,
    pub from_sesno: u32,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub summary: Option<serde_json::Value>,
    pub error: Option<String>,
}

fn new_run_id(kind: &str, dbnum: u32) -> String {
    format!(
        "{kind}-db{dbnum}-{}",
        chrono::Utc::now().format("%Y%m%dT%H%M%S%3fZ")
    )
}

/// 后台跑一次 IncrementRun 并把结果写回注册表。persist=false 即 detect 试跑。
async fn spawn_increment_run(dbnum: u32, persist: bool) -> Result<IncrementRunStatus, String> {
    let watermark = committed_watermark(dbnum)
        .await
        .map_err(|e| format!("查询 Committed Watermark 失败 dbnum={dbnum}: {e}"))?;
    if watermark == 0 {
        return Err(format!(
            "dbnum={dbnum} 无 Committed Watermark（从未全量解析），不能做增量；请先全量建库"
        ));
    }

    let kind = if persist { "sync" } else { "detect" };
    let run_id = new_run_id(kind, dbnum);
    let status = IncrementRunStatus {
        run_id: run_id.clone(),
        dbnum,
        kind: kind.to_string(),
        state: "running".to_string(),
        from_sesno: watermark,
        started_at: chrono::Utc::now().to_rfc3339(),
        finished_at: None,
        summary: None,
        error: None,
    };
    INCREMENT_RUNS.insert(run_id.clone(), status.clone());

    let run_id_task = run_id.clone();
    tokio::spawn(async move {
        let db_option_ext =
            crate::options::DbOptionExt::from((*aios_core::get_db_option()).clone());
        let options = crate::version_management::increment_run::IncrementRunOptions {
            file: None,
            dbnums: vec![dbnum],
            from_sesno: watermark,
            to_sesno: None,
            rescan_index: false,
            persist_data: persist,
            recover_pending: false,
            generate_model: false,
            source_observation_manifest: None,
            source_observation_manifest_hash: None,
            publication_handoff_dir: None,
            release_id_prefix: None,
            require_tree_index: false,
            verbose: false,
        };
        // web server 启动时已连 surreal；这里做一次轻量探针即可。
        let ensure = || async {
            project_primary_db()
                .query("RETURN 1;")
                .await
                .map(|_| ())
                .map_err(anyhow::Error::from)
        };
        let result =
            crate::version_management::increment_run::run_increment(&db_option_ext, options, ensure)
                .await;
        if let Some(mut entry) = INCREMENT_RUNS.get_mut(&run_id_task) {
            entry.finished_at = Some(chrono::Utc::now().to_rfc3339());
            match result {
                Ok(run) => {
                    entry.state = "succeeded".to_string();
                    entry.summary = Some(run.summary);
                }
                Err(err) => {
                    entry.state = "failed".to_string();
                    entry.error = Some(err.to_string());
                }
            }
        }
    });

    Ok(status)
}

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

fn parse_dbnum_path(raw: &str) -> Result<u32, (StatusCode, Json<serde_json::Value>)> {
    raw.trim().parse::<u32>().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "error": format!("site_id 需为 dbnum（数字），收到: {raw}"),
            })),
        )
    })
}

/// 触发增量检测（只读试跑，no-persist）：后台跑 IncrementRun 收集变更但不落库。
/// 路径参数为 dbnum。返回 run_id，用 get_detection_task_status 轮询。
pub async fn start_incremental_detection(
    _state: State<AppState>,
    Path(site_id): Path<String>,
) -> impl IntoResponse {
    let dbnum = match parse_dbnum_path(&site_id) {
        Ok(dbnum) => dbnum,
        Err(resp) => return resp,
    };
    match spawn_increment_run(dbnum, false).await {
        Ok(status) => (
            StatusCode::ACCEPTED,
            Json(json!({
                "success": true,
                "run_id": status.run_id,
                "dbnum": dbnum,
                "kind": "detect",
                "from_sesno": status.from_sesno,
                "message": "增量检测（只读试跑）已启动，用 /api/incremental/task/{run_id} 查询",
            })),
        ),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "error": err })),
        ),
    }
}

/// 触发增量同步（真实落库，persist）：后台跑 IncrementRun，经 commit_version
/// 固化 Version Anchor。persist-only，不触发模型生成（与 watch 语义一致）。
pub async fn start_incremental_sync(
    _state: State<AppState>,
    Path(site_id): Path<String>,
) -> impl IntoResponse {
    let dbnum = match parse_dbnum_path(&site_id) {
        Ok(dbnum) => dbnum,
        Err(resp) => return resp,
    };
    match spawn_increment_run(dbnum, true).await {
        Ok(status) => (
            StatusCode::ACCEPTED,
            Json(json!({
                "success": true,
                "run_id": status.run_id,
                "dbnum": dbnum,
                "kind": "sync",
                "from_sesno": status.from_sesno,
                "message": "增量同步已启动（persist-only，走 Version Commit seam），用 /api/incremental/task/{run_id} 查询",
            })),
        ),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "error": err })),
        ),
    }
}

/// 查询某次增量运行（sync/detect）的状态。
pub async fn get_detection_task_status(
    _state: State<AppState>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    match INCREMENT_RUNS.get(&task_id) {
        Some(status) => (
            StatusCode::OK,
            Json(json!({ "success": true, "run": status.value() })),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "success": false,
                "error": format!("未找到增量运行 run_id={task_id}（进程重启后运行记录会清空）"),
            })),
        ),
    }
}

/// 取消增量任务：IncrementRun 一旦进入落库阶段不可安全中断，故不支持中途取消。
/// Commit 的原子性/幂等由 commit_version 保证，失败自然回退等待重试。
pub async fn cancel_task(
    _state: State<AppState>,
    Path(_task_id): Path<String>,
) -> impl IntoResponse {
    not_implemented("取消进行中的增量运行（IncrementRun 落库不可中途安全中断，请等待完成或依赖 Commit Pending 恢复）")
}

/// 未实现：增量配置读取（无配置存储，检测/同步为按需触发）。
pub async fn get_incremental_config(_state: State<AppState>) -> impl IntoResponse {
    not_implemented("增量配置读取")
}

/// 未实现：增量配置更新（无配置存储）。
pub async fn update_incremental_config(
    _state: State<AppState>,
    Json(_config): Json<serde_json::Value>,
) -> impl IntoResponse {
    not_implemented("增量配置更新")
}
