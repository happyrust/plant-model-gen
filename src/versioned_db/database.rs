#[cfg(feature = "surreal-save")]
use aios_core::project_primary_db;
use anyhow::Context;
use log::{debug, error, info, warn};

// 内存KV数据库全局连接（从 aios_core 导入）
#[cfg(feature = "mem-kv-save")]
#[allow(unused_imports)]
use aios_core::SUL_MEM_DB;

#[cfg(feature = "generation-read-ducklake")]
use aios_core::db::DbBasicData;
#[cfg(feature = "sql")]
use aios_core::db_pool::get_global_pool;
use aios_core::get_default_pdms_db_info;
use aios_core::helper::normalize_sql_string;
use aios_core::options::DbOption;
use aios_core::pdms_types::*;
use aios_core::tool::db_tool::db1_dehash;
use aios_core::tool::hash_tool::hash_str;
use aios_core::types::*;
use chrono::Local;
use dashmap::{DashMap, DashSet};
use futures::StreamExt;
use futures::channel::mpsc::unbounded;
use futures::stream::FuturesUnordered;
use itertools::Itertools;
use parse_pdms_db::parse::*;
use pdms_io::io::PdmsIO;
use petgraph::prelude::DiGraph;
#[cfg(feature = "sql")]
use sea_orm::{ConnectionTrait, Schema, Statement};
#[cfg(feature = "sql")]
use sqlx::{Connection, MySql, MySqlPool, Pool};
#[cfg(feature = "sql")]
use sqlx::{Error, Executor};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::hash::Hash;
use std::io::Read;
use std::mem::take;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};
use tokio::fs;
use tokio::fs::{File, create_dir_all};
use tokio::io::AsyncReadExt;
use tokio::sync::OnceCell;
// use tokio::sync::mpsc::Sender;
use std::sync::mpsc::Sender;

use crate::consts::*;
use crate::data_interface::tidb_manager::AiosDBManager;
// use crate::graph_db::pdms_arango::*;
use crate::tables::*;
#[cfg(feature = "generation-read-ducklake")]
use crate::version_store::replica::{ReplicaDbCatalogEntry, ReplicaElement};
#[cfg(feature = "generation-read-ducklake")]
use crate::version_store::{
    DbCatalogEntry, DuckLakeAuthority, DuckLakeConfig, DuckLakeParseStager, ParseStageVersion,
    ParsedFactBatch, ReplicaApplyBatch, SealedParseStage, SurrealReplicaStore, VersionStoreElement,
};
use crate::versioned_db::db_meta_info;
use crate::versioned_db::pe::*;
use crate::versioned_db::version_commit::{
    VersionCommitCounts, VersionCommitError, VersionCommitRequest, VersionCommitSource,
    commit_version, compute_commit_fingerprint, recover_version_commit,
};
use aios_core::tree_query::TreeNodeMeta;

pub enum SenderJsonsData {
    PEJson(Vec<String>),
    PERelateJson(Vec<String>),
    EleReuseRelateJson(Vec<String>),
    AttJson((String, Vec<String>)),
    // 项目名 , sql
    MysqlSql((String, String)),
    // 新增：用于更新dbnum_info_table
    DbnumInfoUpdate(Vec<String>),
    // 新增：用于按db_num分表保存简化的PE数据 (table_name, sql)
    PartitionedPEJson { table_name: String, sql: String },
    // Kuzu 数据: Vec<(PE, NamedAttrMap)>
}

#[inline]
fn env_usize(key: &str) -> Option<usize> {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
}

#[inline]
fn resolve_sync_chunk_size(configured: Option<u32>, default_chunk_size: usize) -> usize {
    env_usize("AIOS_SYNC_CHUNK_SIZE")
        .or_else(|| configured.map(|value| value as usize))
        .unwrap_or(default_chunk_size)
        .max(1)
}

/// Surreal write-path timing (consumer workers). Cumulative across all insert tasks.
static SYNC_WRITE_QUERIES: AtomicU64 = AtomicU64::new(0);
static SYNC_WRITE_MS: AtomicU64 = AtomicU64::new(0);
static SYNC_WRITE_ROWS: AtomicU64 = AtomicU64::new(0);

fn record_sync_write(kind: &str, rows: usize, ms: u64) {
    let n = SYNC_WRITE_QUERIES.fetch_add(1, Ordering::Relaxed) + 1;
    let total_ms = SYNC_WRITE_MS.fetch_add(ms, Ordering::Relaxed) + ms;
    let total_rows = SYNC_WRITE_ROWS.fetch_add(rows as u64, Ordering::Relaxed) + rows as u64;
    // Always print slow queries; otherwise sample every 25 to avoid drowning logs.
    if ms >= 200 || n % 25 == 0 || n == 1 {
        let rate = if total_ms > 0 {
            total_rows as f64 * 1000.0 / total_ms as f64
        } else {
            0.0
        };
        println!(
            "[sync-write] kind={} rows={} query_ms={} | cumulative queries={} rows={} ms={} ({:.0} rows/s)",
            kind, rows, ms, n, total_rows, total_ms, rate
        );
    }
}

#[cfg(feature = "surreal-save")]
async fn timed_primary_query(kind: &'static str, sql: &str, rows: usize) {
    let t0 = Instant::now();
    let response = project_primary_db()
        .query(sql)
        .await
        .unwrap_or_else(|e| panic!("[sync-write] {kind} transport failed: {e}"));
    if let Err(error) = response.check() {
        if kind == "att" {
            eprintln!("[sync-write][att-statement-failed] rows={rows} error={error} sql={sql}");
        }
        panic!("[sync-write] {kind} statement failed: {error}");
    }
    record_sync_write(kind, rows, t0.elapsed().as_millis() as u64);
}

#[inline]
fn resolve_indextree_chunk_concurrency(is_save_db: bool) -> usize {
    let default_concurrency = if is_save_db {
        1
    } else {
        std::thread::available_parallelism()
            .map(|n| n.get().saturating_div(2).max(1).min(8))
            .unwrap_or(2)
    };

    env_usize("AIOS_INDEXTREE_CHUNK_CONCURRENCY")
        .map(|v| v.max(1).min(8))
        .unwrap_or(default_concurrency)
}

#[inline]
fn resolve_single_indextree_chunk_size(db_option: &DbOption) -> usize {
    env_usize("AIOS_INDEXTREE_SINGLE_CHUNK_SIZE")
        .unwrap_or(db_option.att_chunk as usize)
        .max(1)
}

#[derive(Debug, Clone)]
struct ParsedDbArtifact {
    project_name: String,
    tree_dir: PathBuf,
    dbnum: u32,
    db_type: String,
    file_name: String,
    tree_node_count: usize,
}

fn parse_tree_output_dir(source_project_name: &str) -> PathBuf {
    let Some(active_tree_dir) = db_meta_info::get_current_project_tree_dir() else {
        return db_meta_info::get_project_tree_dir(source_project_name);
    };

    if let Some(active_project_name) = db_meta_info::get_current_project_name() {
        if active_project_name != source_project_name {
            info!(
                "[db_meta] 使用当前配置输出命名空间: source_project={}, output_project={}, tree_dir={}",
                source_project_name,
                active_project_name,
                active_tree_dir.display()
            );
        }
    }

    active_tree_dir
}

struct ParseProgressHeartbeat {
    stop: Arc<AtomicBool>,
    wake: Arc<(Mutex<bool>, Condvar)>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl ParseProgressHeartbeat {
    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let (lock, cvar) = &*self.wake;
        if let Ok(mut stopped) = lock.lock() {
            *stopped = true;
            cvar.notify_one();
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }

    fn stop(mut self) {
        self.shutdown();
    }
}

impl Drop for ParseProgressHeartbeat {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[allow(clippy::too_many_arguments)]
fn start_parse_progress_heartbeat(
    project_name: String,
    file_name: String,
    dbnum: u32,
    db_type: String,
    save_db: bool,
    refnos_total: usize,
    chunks_total: usize,
    chunks_completed: usize,
    last_chunk: Option<usize>,
    parsed_attrs: usize,
    chunk_stage_start: Instant,
) -> ParseProgressHeartbeat {
    crate::perf_metrics::record_parse_progress(crate::perf_metrics::ParseProgressUpdate {
        stage: "chunk_pending",
        project_name: &project_name,
        file_name: &file_name,
        dbnum,
        db_type: &db_type,
        save_db,
        refnos_total,
        chunks_total,
        chunks_completed,
        last_chunk,
        parsed_attrs,
        elapsed_ms: chunk_stage_start.elapsed().as_millis() as u64,
    });

    let stop = Arc::new(AtomicBool::new(false));
    let wake = Arc::new((Mutex::new(false), Condvar::new()));
    let task_stop = stop.clone();
    let task_wake = wake.clone();

    let handle = std::thread::spawn(move || {
        loop {
            let (lock, cvar) = &*task_wake;
            let stopped = match lock.lock() {
                Ok(stopped) => stopped,
                Err(_) => break,
            };
            let stopped = match cvar.wait_timeout_while(
                stopped,
                Duration::from_secs(15),
                |stopped| !*stopped,
            ) {
                Ok((stopped, _)) => stopped,
                Err(_) => break,
            };
            if *stopped || task_stop.load(Ordering::Relaxed) {
                break;
            }
            drop(stopped);

            crate::perf_metrics::record_parse_progress(crate::perf_metrics::ParseProgressUpdate {
                stage: "chunk_pending",
                project_name: &project_name,
                file_name: &file_name,
                dbnum,
                db_type: &db_type,
                save_db,
                refnos_total,
                chunks_total,
                chunks_completed,
                last_chunk,
                parsed_attrs,
                elapsed_ms: chunk_stage_start.elapsed().as_millis() as u64,
            });
        }
    });

    ParseProgressHeartbeat {
        stop,
        wake,
        handle: Some(handle),
    }
}

fn validate_parse_scene_tree_artifacts(artifacts: &[ParsedDbArtifact]) -> anyhow::Result<()> {
    if artifacts.is_empty() {
        warn!("[db_meta] 本轮解析没有实际处理任何 DB 文件，跳过 db_meta 产物校验");
        return Ok(());
    }

    let mut errors = Vec::new();

    for artifact in artifacts {
        let tree_dir = &artifact.tree_dir;
        let meta_path = tree_dir.join("db_meta_info.json");
        let meta = match std::fs::read_to_string(&meta_path) {
            Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                Ok(value) => Some(value),
                Err(err) => {
                    errors.push(format!(
                        "dbnum={} file={} db_meta_info.json 解析失败({}): {}",
                        artifact.dbnum,
                        artifact.file_name,
                        meta_path.display(),
                        err
                    ));
                    None
                }
            },
            Err(err) => {
                errors.push(format!(
                    "dbnum={} file={} 缺少 db_meta_info.json({}): {}",
                    artifact.dbnum,
                    artifact.file_name,
                    meta_path.display(),
                    err
                ));
                None
            }
        };

        if let Some(meta) = meta.as_ref() {
            let dbnum_key = artifact.dbnum.to_string();
            let has_db_file = meta
                .get("db_files")
                .and_then(|value| value.as_object())
                .map(|files| files.contains_key(&dbnum_key))
                .unwrap_or(false);
            if !has_db_file {
                errors.push(format!(
                    "dbnum={} file={} 未写入 db_meta_info.json.db_files",
                    artifact.dbnum, artifact.file_name
                ));
            }
        }

        if artifact.tree_node_count == 0 {
            warn!(
                "[db_meta] dbnum={} file={} db_type={} 解析得到 0 个 hierarchy node，按约定只告警不失败",
                artifact.dbnum, artifact.file_name, artifact.db_type
            );
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("scene_tree 解析期产物校验失败:\n{}", errors.join("\n"))
    }
}

const SYSTEM_SYNC_DB_TYPES: &[&str] = &["DICT", "SYST", "GLB", "GLOB"];
const DEFAULT_DATA_SYNC_DB_TYPES: &[&str] = &["DESI", "CATA"];

fn collect_project_db_files(project_dir: impl AsRef<Path>) -> anyhow::Result<Vec<PathBuf>> {
    let project_dir = project_dir.as_ref();
    let mut children_files = {
        let target_dir = std::fs::read_dir(project_dir)?
            .into_iter()
            .map(|entry| {
                let entry = entry.unwrap();
                entry.path()
            })
            .find(|x| x.is_dir() && x.file_name().unwrap().to_str().unwrap().ends_with("000"))
            .ok_or_else(|| {
                anyhow::anyhow!("项目目录下未找到 000 数据目录: {}", project_dir.display())
            })?;
        std::fs::read_dir(target_dir)?
            .into_iter()
            .map(|entry| {
                let entry = entry.unwrap();
                entry.path()
            })
            .collect::<Vec<PathBuf>>()
    };

    let mut file_map = HashMap::new();
    for path in children_files.iter() {
        let file_name = path.file_stem().unwrap().to_str().unwrap();
        if let Some(base_name) = file_name.strip_suffix("_0001") {
            file_map.insert(base_name.to_string(), path.clone());
        } else if !file_map.contains_key(file_name) {
            file_map.insert(file_name.to_string(), path.clone());
        }
    }

    children_files = file_map.into_values().collect();
    children_files.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
    Ok(children_files)
}

fn selected_db_file_names(db_option: &DbOption) -> HashSet<String> {
    let mut selected = HashSet::new();
    if let Some(files) = &db_option.included_db_files {
        for file in files {
            let trimmed = file.trim();
            if trimmed.is_empty() {
                continue;
            }
            selected.insert(trimmed.to_string());
            if let Some(stem) = Path::new(trimmed)
                .file_stem()
                .and_then(|value| value.to_str())
            {
                selected.insert(stem.to_string());
            }
        }
    }
    selected
}

fn selected_dbnums(db_option: &DbOption) -> HashSet<u32> {
    db_option
        .manual_db_nums
        .clone()
        .unwrap_or_default()
        .into_iter()
        .filter(|dbnum| *dbnum > 0)
        .collect()
}

fn should_process_sync_file(
    file_name: &str,
    dbnum: u32,
    selected_file_names: &HashSet<String>,
    selected_dbnums: &HashSet<u32>,
    force_include: bool,
) -> bool {
    if force_include {
        return true;
    }
    if !selected_file_names.is_empty() {
        return selected_file_names.contains(file_name);
    }
    if !selected_dbnums.is_empty() {
        return selected_dbnums.contains(&dbnum);
    }
    true
}

fn resolve_data_sync_db_types(db_option: &DbOption, project: &str) -> anyhow::Result<Vec<String>> {
    let selected_file_names = selected_db_file_names(db_option);
    let selected_dbnums = selected_dbnums(db_option);
    if selected_file_names.is_empty() && selected_dbnums.is_empty() {
        return Ok(DEFAULT_DATA_SYNC_DB_TYPES
            .iter()
            .map(|value| value.to_string())
            .collect());
    }

    let project_dir = db_option
        .get_project_path(project)
        .ok_or_else(|| anyhow::anyhow!("项目路径不存在: {}", project))?;
    let mut db_types = BTreeSet::new();
    for path in collect_project_db_files(&project_dir)? {
        let file_name = path.file_name().unwrap().to_str().unwrap().to_string();
        if file_name.contains('.') {
            continue;
        }
        let mut file = std::fs::File::open(&path)?;
        let mut buf = [0u8; 60];
        file.read_exact(&mut buf)?;
        let db_basic_info = parse_file_basic_info(&buf);
        if !should_process_sync_file(
            &file_name,
            db_basic_info.dbnum,
            &selected_file_names,
            &selected_dbnums,
            false,
        ) {
            continue;
        }
        if !SYSTEM_SYNC_DB_TYPES.contains(&db_basic_info.db_type.as_str()) {
            db_types.insert(db_basic_info.db_type);
        }
    }
    Ok(db_types.into_iter().collect())
}

/// 兼容旧版 pdms_io：缺少 `sync_history` 时降级为 no-op，避免阻塞 web_server 编译。
#[inline]
async fn pdms_sync_history_compat(_io: &mut PdmsIO) -> anyhow::Result<()> {
    warn!("pdms_io 未提供 sync_history，跳过该步骤（兼容模式）");
    Ok(())
}

/// 兼容旧版 pdms_io：缺少 `store_all_refno_sesno_map` 时降级为 no-op。
#[inline]
async fn pdms_store_refno_sesno_map_compat(_io: &mut PdmsIO) -> anyhow::Result<()> {
    warn!("pdms_io 未提供 store_all_refno_sesno_map，跳过该步骤（兼容模式）");
    Ok(())
}

/// 兼容旧版 parse_pdms_db：缺少 `preload_uda_name_cache` 时降级为 no-op。
#[inline]
async fn preload_uda_name_cache_compat() -> anyhow::Result<()> {
    warn!("parse_pdms_db 未启用 preload_uda_name_cache，跳过预加载（兼容模式）");
    Ok(())
}

#[cfg(feature = "surreal-save")]
static ELE_REUSE_RELATE_SCHEMA_INIT: OnceCell<()> = OnceCell::const_new();

#[cfg(feature = "surreal-save")]
async fn ensure_ele_reuse_relate_relation_schema() {
    ELE_REUSE_RELATE_SCHEMA_INIT
        .get_or_init(|| async {
            let _ = project_primary_db().query("REMOVE TABLE ele_reuse_relate;").await;

            let _ = project_primary_db()
                .query("DEFINE TABLE ele_reuse_relate TYPE RELATION;")
                .await;

            let _ = project_primary_db()
                .query("REMOVE FIELD in ON TABLE ele_reuse_relate;")
                .await;
            let _ = project_primary_db()
                .query("REMOVE FIELD out ON TABLE ele_reuse_relate;")
                .await;
            let _ = project_primary_db()
                .query("DEFINE FIELD in ON TABLE ele_reuse_relate TYPE record<pe>;")
                .await;
            let _ = project_primary_db()
                .query("DEFINE FIELD out ON TABLE ele_reuse_relate TYPE record<inst_info>;")
                .await;
            let _ = project_primary_db()
                .query(
                    "DEFINE INDEX idx_ele_reuse_relate_in ON TABLE ele_reuse_relate FIELDS in UNIQUE;",
                )
                .await;
            let _ = project_primary_db()
                .query(
                    "DEFINE INDEX idx_ele_reuse_relate_out ON TABLE ele_reuse_relate FIELDS out;",
                )
                .await;
        })
        .await;
}

static SESNO_VERSION_ANCHOR_SCHEMA_INIT: OnceCell<()> = OnceCell::const_new();

/// 确保 sesno_version_anchor 锚点表 schema 存在（specs/022 T008）。
///
/// 表用途：把业务版本号 sesno 固化为存储时间戳锚点
///（dbnum + sesno + source → anchored_at），供按 sesno 的历史查询
///（`SELECT ... VERSION`）换算时间戳。数据提交写 `full`/`incremental`，
/// 模型生成全部成功后写 `model_gen`。
///
/// 幂等：DDL 全部使用 IF NOT EXISTS；进程内经 OnceCell 成功后只执行一次，
/// 失败不缓存、下次调用重试并向调用方传播错误（锚点写入前必须 schema 就绪）。
pub async fn ensure_sesno_version_anchor_schema() -> anyhow::Result<()> {
    SESNO_VERSION_ANCHOR_SCHEMA_INIT
        .get_or_try_init(|| async {
            let sql = r#"
DEFINE TABLE IF NOT EXISTS sesno_version_anchor SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS dbnum ON TABLE sesno_version_anchor TYPE int;
DEFINE FIELD IF NOT EXISTS sesno ON TABLE sesno_version_anchor TYPE int;
DEFINE FIELD IF NOT EXISTS anchored_at ON TABLE sesno_version_anchor TYPE datetime DEFAULT time::now();
DEFINE FIELD OVERWRITE source ON TABLE sesno_version_anchor TYPE string ASSERT $value IN ['full', 'incremental', 'model_gen'];
DEFINE FIELD IF NOT EXISTS note ON TABLE sesno_version_anchor TYPE option<string>;
REMOVE INDEX IF EXISTS idx_sesno_version_anchor_dbnum_sesno ON sesno_version_anchor;
DEFINE INDEX IF NOT EXISTS idx_sesno_version_anchor_dbnum_sesno_source ON TABLE sesno_version_anchor FIELDS dbnum, sesno, source UNIQUE;
"#;
            aios_core::project_primary_db()
                .query(sql)
                .await
                .map_err(|e| anyhow::anyhow!("定义 sesno_version_anchor schema 失败: {e}"))?
                .check()
                .map_err(|e| anyhow::anyhow!("sesno_version_anchor schema 语句执行失败: {e}"))?;
            info!("sesno_version_anchor 锚点表 schema 已就绪");
            Ok::<(), anyhow::Error>(())
        })
        .await?;
    // OVERWRITE 幂等：表 OnceCell 命中后仍刷新函数，便于升级已跑进程外的新部署。
    ensure_sesno_version_lookup_functions().await?;
    Ok(())
}

/// specs/024：库内按数据/模型语义分别查找 `sesno → anchored_at`。
///
/// - `fn::data_sesno_version*` 只解析 `full`/`incremental`
/// - `fn::model_sesno_version*` 只解析 `model_gen`
/// 精确命中优先；否则同 dbnum 下 `sesno <= 请求` 的最大一条（`exact=false`）。
async fn ensure_sesno_version_lookup_functions() -> anyhow::Result<()> {
    let sql = r#"
DEFINE FUNCTION OVERWRITE fn::data_sesno_version($dbnum: number, $sesno: number) {
    LET $exact = (
        SELECT VALUE anchored_at FROM sesno_version_anchor
        WHERE dbnum = $dbnum AND sesno = $sesno
            AND source IN ['full', 'incremental']
        ORDER BY anchored_at DESC
        LIMIT 1
    );
    IF array::len($exact) > 0 {
        RETURN $exact[0];
    };
    LET $fb = (
        SELECT VALUE anchored_at FROM sesno_version_anchor
        WHERE dbnum = $dbnum AND sesno <= $sesno
            AND source IN ['full', 'incremental']
        ORDER BY sesno DESC, anchored_at DESC
        LIMIT 1
    );
    RETURN IF array::len($fb) > 0 { $fb[0] } ELSE { NONE };
};

DEFINE FUNCTION OVERWRITE fn::data_sesno_version_hit($dbnum: number, $sesno: number) {
    LET $exact = (
        SELECT dbnum, sesno, source, anchored_at FROM sesno_version_anchor
        WHERE dbnum = $dbnum AND sesno = $sesno
            AND source IN ['full', 'incremental']
        ORDER BY anchored_at DESC
        LIMIT 1
    );
    IF array::len($exact) > 0 {
        LET $r = $exact[0];
        RETURN {
            dbnum: $r.dbnum,
            sesno: $r.sesno,
            source: $r.source,
            anchored_at: $r.anchored_at,
            exact: true
        };
    };
    LET $fb = (
        SELECT dbnum, sesno, source, anchored_at FROM sesno_version_anchor
        WHERE dbnum = $dbnum AND sesno <= $sesno
            AND source IN ['full', 'incremental']
        ORDER BY sesno DESC, anchored_at DESC
        LIMIT 1
    );
    IF array::len($fb) > 0 {
        LET $r = $fb[0];
        RETURN {
            dbnum: $r.dbnum,
            sesno: $r.sesno,
            source: $r.source,
            anchored_at: $r.anchored_at,
            exact: false
        };
    };
    RETURN NONE;
};

DEFINE FUNCTION OVERWRITE fn::model_sesno_version($dbnum: number, $sesno: number) {
    LET $hit = (
        SELECT VALUE anchored_at FROM sesno_version_anchor
        WHERE dbnum = $dbnum AND sesno <= $sesno AND source = 'model_gen'
        ORDER BY sesno DESC, anchored_at DESC
        LIMIT 1
    );
    RETURN IF array::len($hit) > 0 { $hit[0] } ELSE { NONE };
};

DEFINE FUNCTION OVERWRITE fn::model_sesno_version_hit($dbnum: number, $sesno: number) {
    LET $hit = (
        SELECT dbnum, sesno, source, anchored_at FROM sesno_version_anchor
        WHERE dbnum = $dbnum AND sesno <= $sesno AND source = 'model_gen'
        ORDER BY sesno DESC, anchored_at DESC
        LIMIT 1
    );
    IF array::len($hit) > 0 {
        LET $r = $hit[0];
        RETURN {
            dbnum: $r.dbnum,
            sesno: $r.sesno,
            source: $r.source,
            anchored_at: $r.anchored_at,
            exact: $r.sesno = $sesno
        };
    };
    RETURN NONE;
};
"#;
    aios_core::project_primary_db()
        .query(sql)
        .await
        .map_err(|e| anyhow::anyhow!("定义 fn::*_sesno_version* 失败: {e}"))?
        .check()
        .map_err(|e| anyhow::anyhow!("fn::*_sesno_version* 语句执行失败: {e}"))?;
    Ok(())
}

/// specs/022 T010：把本轮全量解析成功的 (dbnum, latest_sesno) 固化为 `source='full'` 锚点。
///
/// 调用时机约束：必须在本轮写库任务全部 join 之后（sync_total_async_threaded* 内
/// `drop(sender)` + 等待 insert_handles 排空之后）调用，保证 anchored_at 晚于该
/// dbnum 本轮全部 PE/ATT 写入；不能在解析循环里逐文件写（写库经 flume channel
/// 异步 flush，逐文件时刻数据未必已落库）。
///
/// specs/023 T008：pe_owner 边（PERelateJson）走同一 sender/sink 通道，上述 join
/// 约束同样保证"边全部落库先于 full 锚点固化"——锚点时刻的 VERSION 查询必然
/// 覆盖本轮全部边。锚点成功后按 dbnum 写 `pe_owner_version_meta`（full_reload
/// 重置可信起点；仅 surreal-save 构建下有边可信可言）。
///
/// 锚点失败必须向上传播。数据写入已经完成，调用方可只重试收尾；但在锚点成功前
/// 不能把本次 full sync 宣告为可供历史查询的完整提交。
async fn write_full_version_anchors(
    pending: &[(u32, u32, String)],
) -> anyhow::Result<Vec<crate::data_interface::sesno_increment::VersionAnchorRecord>> {
    let mut written = Vec::with_capacity(pending.len());
    for (dbnum, sesno, source_evidence) in pending {
        let fingerprint_input = format!("full-sync-v1:{dbnum}:{sesno}");
        let fingerprint = compute_commit_fingerprint(
            *dbnum,
            *sesno,
            *sesno,
            VersionCommitSource::Full,
            None,
            [fingerprint_input.as_str(), source_evidence.as_str()],
        );
        let counts = VersionCommitCounts::default();
        let request = VersionCommitRequest {
            dbnum: *dbnum,
            from_sesno: *sesno,
            to_sesno: *sesno,
            source: VersionCommitSource::Full,
            fingerprint,
            source_hash: None,
            expected_counts: Some(counts.clone()),
        };
        let outcome = match commit_version(request.clone(), || async { Ok(counts.clone()) }).await {
            Ok(outcome) => outcome,
            Err(VersionCommitError::PendingCommit { pending_sesno, .. })
                if pending_sesno == *sesno =>
            {
                recover_version_commit(request, || async { Ok(counts) })
                    .await
                    .with_context(|| {
                        format!(
                            "full sync 数据已写入，但 pending 锚点恢复失败(dbnum={dbnum} sesno={sesno})"
                        )
                    })?
            }
            Err(error) => {
                return Err(anyhow::Error::new(error).context(format!(
                    "full sync 数据已写入，但锚点发布失败(dbnum={dbnum} sesno={sesno})"
                )));
            }
        };
        let record = crate::data_interface::sesno_increment::VersionAnchorRecord {
            dbnum: outcome.dbnum,
            sesno: outcome.to_sesno,
            source: "full".to_string(),
            anchored_at: Some(outcome.anchored_at),
            fingerprint: Some(outcome.fingerprint),
            idempotent: outcome.idempotent,
            recovered: outcome.recovered,
        };
        info!(
            "sesno_version_anchor(full) 已写入: dbnum={} sesno={} anchored_at={:?}",
            record.dbnum, record.sesno, record.anchored_at
        );
        // specs/023 T008：full 重灌以先删后插覆盖了全部 owner 的边，
        // 自本 sesno 起 pe_owner 历史可信；meta 失败不阻断（读侧回退 pe.children 天然安全）。
        #[cfg(feature = "surreal-save")]
        if let Err(e) = crate::versioned_db::pe_owner_meta::upsert_maintained_since(
            *dbnum,
            *sesno,
            crate::versioned_db::pe_owner_meta::META_SOURCE_FULL_RELOAD,
        )
        .await
        {
            log::warn!(
                "pe_owner_version_meta(full_reload) 写入失败(dbnum={dbnum} sesno={sesno}): {e}"
            );
        }
        written.push(record);
    }
    Ok(written)
}

fn full_sync_source_evidence(path: &Path) -> String {
    let Ok(metadata) = std::fs::metadata(path) else {
        return format!("{}|metadata=unavailable", path.display());
    };
    let modified_nanos = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!(
        "{}|len={}|modified_ns={modified_nanos}",
        path.display(),
        metadata.len()
    )
}

/// 初始化project database
#[cfg(feature = "sql")]
pub async fn create_project_database(project: &str, url: &str) -> anyhow::Result<()> {
    let pool = MySqlPool::connect(url).await.unwrap();
    sqlx::query(&format!(
        "CREATE DATABASE IF NOT EXISTS {project} DEFAULT CHARSET UTF8"
    ))
    .execute(&pool)
    .await?;
    Ok(())
}

/// 初始化 info 库和表
#[cfg(feature = "sql")]
pub async fn create_info_database(db_option: &DbOption) -> anyhow::Result<()> {
    let pool = get_global_pool(db_option).await?;
    let project_name = db_option.project_name.clone();
    pool.execute(
        format!(
            "CREATE DATABASE IF NOT EXISTS {PDMS_INFO_DB}_{};",
            project_name
        )
        .as_str(),
    )
    .await?;

    //todo 改成一对多的实现
    let mut sql = String::new();
    sql.push_str(&format!(r#"CREATE TABLE IF NOT EXISTS {} ("#, {
        PDMS_REFNO_INFOS_TABLE
    }));
    // sql.push_str(&format!(r#"{} BIGINT NOT NULL PRIMARY KEY ,"#, "REF0"));
    sql.push_str(&format!(r#"{} BIGINT UNSIGNED PRIMARY KEY ,"#, "ID"));
    sql.push_str(&format!(r#"{} BIGINT NOT NULL ,"#, "REF0"));
    //允许有多个project的存在
    sql.push_str(&format!(r#"{} VARCHAR(100)"#, "PROJECT"));

    sql.push_str(");");
    let result = pool.execute(sql.as_str()).await;
    match result {
        Ok(_) => {}
        Err(e) => {
            dbg!(e);
            dbg!(sql.as_str());
        }
    }

    let result = pool
        .execute(gen_create_dbno_infos_tables_sql().as_str())
        .await;
    match result {
        Ok(_) => {}
        Err(e) => {
            dbg!(&e);
        }
    }
    let result = pool
        .execute(gen_create_version_info_table_sql(&project_name).as_str())
        .await;
    match result {
        Ok(_) => {}
        Err(e) => {
            dbg!(&e);
        }
    }
    let pool = aios_mgr.get_project_pool().await?;
    let result = pool.execute(gen_create_element_tables_sql().as_str()).await;
    match result {
        Ok(_) => {}
        Err(e) => {
            dbg!(&e);
        }
    }

    Ok(())
}

/// 带进度回调的同步pdms数据到数据库
pub async fn sync_pdms_with_callback<F>(
    db_option: &DbOption,
    mut progress_callback: Option<F>,
) -> anyhow::Result<()>
where
    F: FnMut(&str, usize, usize, usize, usize, usize, usize) + Send,
{
    if db_option.included_projects.is_empty() {
        return Err(anyhow::anyhow!("没有包含的项目"));
    }
    #[cfg(feature = "generation-read-ducklake")]
    {
        let options = crate::options::get_db_option_ext();
        if options.parse_storage_backend.uses_ducklake() {
            options.validate_parse_storage_features()?;
            let callback = progress_callback
                .as_mut()
                .map(|callback| callback as &mut SyncProgressCallback<'_>);
            return sync_pdms_to_ducklake(
                db_option,
                options.parse_storage_config(),
                options.ducklake_config(),
                callback,
            )
            .await;
        }
    }

    // 开始同步pdms/E3D项目的数据
    info!("开始同步pdms/E3D: {} 的数据", &db_option.project_name);
    let mut time = tokio::time::Instant::now();

    #[cfg(feature = "surreal-save")]
    {
        // 解析前移除EVENT，防止大量的event触发
        info!("正在移除dbnum_event以提高解析性能...");
        let remove_event_sql = "REMOVE EVENT update_dbnum_event ON pe;";
        match project_primary_db().query(remove_event_sql).await {
            Ok(_) => info!("成功移除update_dbnum_event"),
            Err(e) => info!("移除update_dbnum_event失败（可能不存在）: {:?}", e),
        }

        // 项目库 schema 初始化：sesno 版本锚点表（specs/022 T008）
        if let Err(e) = ensure_sesno_version_anchor_schema().await {
            warn!("初始化 sesno_version_anchor schema 失败（锚点写入前会重试）: {e:?}");
        }
    }

    // 创建表
    let create_tables_start = time.elapsed().as_millis();
    // TODO: 需要实现create_tables函数或使用现有的表创建逻辑
    // create_tables().await?;
    let create_tables_elapse = time.elapsed().as_millis() - create_tables_start;

    let mut dbno_set = Arc::new(DashSet::new());

    // 执行多线程解析
    dbg!("执行多线程解析");
    let proj_progress_chunk = 80 / db_option.included_projects.len();
    let total_projects = db_option.included_projects.len();

    // 遍历所有包含的项目
    for (project_index, project) in db_option.included_projects.iter().enumerate() {
        // 解析时不应该受 debug_model_refnos 影响，只用于模型生成调试
        let debug_refnos: Vec<RefU64> = Vec::new(); // 暂时禁用解析调试模式
        let data_db_types = resolve_data_sync_db_types(db_option, project)?;
        let data_db_types_refs = data_db_types.iter().map(String::as_str).collect::<Vec<_>>();

        // 统计项目中的文件数量
        let project_dir = db_option
            .get_project_path(project)
            .ok_or_else(|| anyhow::anyhow!("项目路径不存在: {}", project))?;
        let total_files = if Path::new(&project_dir).exists() {
            let target_dir = std::fs::read_dir(&project_dir)
                .unwrap()
                .into_iter()
                .map(|entry| {
                    let entry = entry.unwrap();
                    entry.path()
                })
                .find(|x| x.is_dir() && x.file_name().unwrap().to_str().unwrap().ends_with("000"))
                .unwrap();

            let children_files: Vec<PathBuf> = std::fs::read_dir(target_dir)?
                .into_iter()
                .map(|entry| {
                    let entry = entry.unwrap();
                    entry.path()
                })
                .collect();

            // 处理文件名_0001和文件名同时存在的情况
            let mut file_map = HashMap::new();
            for path in children_files.iter() {
                let file_name = path.file_stem().unwrap().to_str().unwrap();
                if let Some(base_name) = file_name.strip_suffix("_0001") {
                    file_map.insert(base_name.to_string(), path.clone());
                } else {
                    if !file_map.contains_key(file_name) {
                        file_map.insert(file_name.to_string(), path.clone());
                    }
                }
            }
            file_map.len()
        } else {
            0
        };

        // 通知进度回调开始处理项目
        if let Some(ref mut callback) = progress_callback {
            callback(
                project,
                project_index + 1,
                total_projects,
                0,
                total_files,
                0,
                0,
            );
        }

        //debug 不保存数据，只复杂查看属性值
        let is_debug = !debug_refnos.is_empty();
        let cur_dbno_set = dbno_set.clone();
        if is_debug || db_option.only_sync_sys || db_option.total_sync {
            match sync_total_async_threaded_with_callback(
                &db_option,
                project,
                cur_dbno_set,
                SYSTEM_SYNC_DB_TYPES,
                proj_progress_chunk,
                &mut progress_callback,
                project_index + 1,
                total_projects,
            )
            .await
            {
                Ok(_) => {
                    info!("同步UDA和SYS数据成功。");
                    // SYST 解析完成后预加载 UDA 名称缓存
                    if let Err(e) = preload_uda_name_cache_compat().await {
                        warn!("预加载 UDA 名称缓存失败: {}", e);
                    }
                }
                Err(e) => {
                    return Err(e.context(format!("同步系统库失败(project={})", project)));
                }
            }
        }

        //只同步"DICT", "SYST", "GLB", "GLOB" 这些信息
        if db_option.only_sync_sys {
            continue;
        }

        if !data_db_types_refs.is_empty() {
            let cur_dbno_set = Arc::new(DashSet::new());
            match sync_total_async_threaded_with_callback(
                &db_option,
                project,
                cur_dbno_set,
                &data_db_types_refs,
                proj_progress_chunk,
                &mut progress_callback,
                project_index + 1,
                total_projects,
            )
            .await
            {
                Ok(_) => {
                    info!("同步数据成功。");
                }
                Err(e) => {
                    return Err(e.context(format!("同步数据失败(project={})", project)));
                }
            }
        }
    }

    // 解析完成后重新定义EVENT
    info!("正在重新定义dbnum_event...");
    match aios_core::define_dbnum_event().await {
        Ok(_) => info!("成功重新定义update_dbnum_event"),
        Err(e) => info!("重新定义update_dbnum_event失败: {:?}", e),
    }

    // 输出创建表所花费的时间
    info!("创建表花费时间: {} ms", create_tables_elapse);
    // 输出初始化数据库所花费的时间
    info!(
        "初始化数据库时间: {} ms",
        time.elapsed().as_millis() - create_tables_elapse
    );

    Ok(())
}

/// 初始化同步pdms数据到数据
pub async fn sync_pdms(db_option: &DbOption) -> anyhow::Result<()> {
    if db_option.included_projects.is_empty() {
        return Err(anyhow::anyhow!("没有包含的项目"));
    }
    #[cfg(feature = "generation-read-ducklake")]
    {
        let options = crate::options::get_db_option_ext();
        if options.parse_storage_backend.uses_ducklake() {
            options.validate_parse_storage_features()?;
            return sync_pdms_to_ducklake(
                db_option,
                options.parse_storage_config(),
                options.ducklake_config(),
                None,
            )
            .await;
        }
    }
    // 开始同步pdms/E3D项目的数据
    info!("开始同步pdms/E3D: {} 的数据", &db_option.project_name);
    // 计时器开始
    let mut time = tokio::time::Instant::now();

    #[cfg(feature = "surreal-save")]
    {
        // 解析前移除EVENT，防止大量的event触发
        info!("正在移除dbnum_event以提高解析性能...");
        let remove_event_sql = "REMOVE EVENT update_dbnum_event ON pe;";
        match project_primary_db().query(remove_event_sql).await {
            Ok(_) => info!("成功移除update_dbnum_event"),
            Err(e) => info!("移除update_dbnum_event失败（可能不存在）: {:?}", e),
        }

        // 项目库 schema 初始化：sesno 版本锚点表（specs/022 T008）
        if let Err(e) = ensure_sesno_version_anchor_schema().await {
            warn!("初始化 sesno_version_anchor schema 失败（锚点写入前会重试）: {e:?}");
        }
    }

    // 获取默认的数据库连接字符串
    if db_option.sync_tidb.unwrap_or(false) {
        #[cfg(feature = "sql")]
        {
            create_info_database(db_option).await?;
        }
    }

    //只有重新同步时，才需要定义index
    let enable_index = db_option.total_sync || db_option.enable_index.unwrap_or(true);
    if enable_index {
        // 主库创建索引
        aios_core::define_owner_index().await.unwrap();
        aios_core::create_geom_index().await.unwrap();
        // aios_core::define_fullname_index().await.unwrap();
        aios_core::define_pe_index().await.unwrap();

        // 备份内存KV库也创建相同索引（幂等）
        #[cfg(feature = "mem-kv-save")]
        {
            use aios_core::SUL_MEM_DB;
            // 使用新增的带连接版本
            let _ = aios_core::rs_surreal::index::define_owner_index_with(&SUL_MEM_DB).await;
            let _ = aios_core::rs_surreal::index::create_geom_index_with(&SUL_MEM_DB).await;
            let _ = aios_core::rs_surreal::index::define_pe_index_with(&SUL_MEM_DB).await;
        }
    }
    if db_option.is_sync_history() {
        aios_core::define_ses_index().await.unwrap();
        #[cfg(feature = "mem-kv-save")]
        {
            use aios_core::SUL_MEM_DB;
            let _ = aios_core::rs_surreal::index::define_ses_index_with(&SUL_MEM_DB).await;
        }
    }

    let mut dbno_set = Arc::new(DashSet::new());
    let mut create_tables_elapse = 0;
    // 执行多线程解析
    dbg!("执行多线程解析");
    let proj_progress_chunk = 80 / db_option.included_projects.len();
    // 遍历所有包含的项目
    for project in &db_option.included_projects {
        let data_db_types = resolve_data_sync_db_types(db_option, project)?;
        let data_db_types_refs = data_db_types.iter().map(String::as_str).collect::<Vec<_>>();
        // 解析时不应该受 debug_model_refnos 影响，只用于模型生成调试
        let debug_refnos: Vec<RefU64> = Vec::new(); // 暂时禁用解析调试模式
        //debug 不保存数据，只复杂查看属性值
        let is_debug = !debug_refnos.is_empty();
        let cur_dbno_set = dbno_set.clone();
        if is_debug || db_option.only_sync_sys || db_option.total_sync {
            // let progress_sender = progress_sender.clone();
            match sync_total_async_threaded(
                &db_option,
                project,
                cur_dbno_set,
                SYSTEM_SYNC_DB_TYPES,
                // progress_sender,
                proj_progress_chunk,
            )
            .await
            {
                Ok(_) => {
                    // 同步数据成功
                    info!("同步UDA和SYS数据成功。");
                    // SYST 解析完成后预加载 UDA 名称缓存
                    if let Err(e) = preload_uda_name_cache_compat().await {
                        warn!("预加载 UDA 名称缓存失败: {}", e);
                    }
                }
                Err(e) => {
                    // 同步数据失败，打印错误信息
                    return Err(e.context(format!("同步系统库失败(project={})", project)));
                }
            }
        }
        //只同步"DICT", "SYST", "GLB", "GLOB" 这些信息
        if db_option.only_sync_sys {
            continue;
        }
        if !data_db_types_refs.is_empty() {
            // 第二次调用使用新的 dbno_set，避免被第一次调用的 dbnum 过滤
            let cur_dbno_set = Arc::new(DashSet::new());
            match sync_total_async_threaded(
                &db_option,
                project,
                cur_dbno_set,
                &data_db_types_refs,
                // progress_sender,
                proj_progress_chunk,
            )
            .await
            {
                Ok(_) => {
                    // 同步数据成功
                    info!("同步数据成功。");
                }
                Err(e) => {
                    // 同步数据失败，打印错误信息
                    return Err(e.context(format!("同步数据失败(project={})", project)));
                }
            }
        }
    }

    // 解析完成后重新定义EVENT
    info!("正在重新定义dbnum_event...");
    match aios_core::define_dbnum_event().await {
        Ok(_) => info!("成功重新定义update_dbnum_event"),
        Err(e) => info!("重新定义update_dbnum_event失败: {:?}", e),
    }

    // 输出创建表所花费的时间
    info!("创建表花费时间: {} ms", create_tables_elapse);
    // 输出初始化数据库所花费的时间
    info!(
        "初始化数据库时间: {} ms",
        time.elapsed().as_millis() - create_tables_elapse
    );

    Ok(())
}

#[cfg(feature = "surreal-save")]
#[deprecated(
    note = "已迁移到 aios_core::define_dbnum_event，请使用 aios_core::define_dbnum_event() 代替"
)]
pub async fn define_dbnum_event() -> anyhow::Result<()> {
    // 调用 aios_core 中的实现
    aios_core::define_dbnum_event().await
}

#[cfg(not(feature = "surreal-save"))]
#[deprecated(
    note = "已迁移到 aios_core::define_dbnum_event，请使用 aios_core::define_dbnum_event() 代替"
)]
pub async fn define_dbnum_event() -> anyhow::Result<()> {
    aios_core::define_dbnum_event().await
}

/// 定义dbnum_info_table的更新事件, pe 的id 为array的情况
#[cfg(feature = "surreal-save")]
pub async fn define_dbnum_event_array_id() -> anyhow::Result<()> {
    let event_sql = r#"
DEFINE EVENT OVERWRITE update_dbnum_event ON pe WHEN $event = "CREATE" OR $event = "UPDATE" OR $event = "DELETE" THEN {
            -- 获取当前记录的 dbnum
            LET $dbnum = $value.dbnum;
            LET $id = record::id($value.id);
            let $ref_0 = array::at($id, 0);
            let $ref_1 = array::at($id, 1);
            let $is_delete = $value.deleted and $event = "UPDATE";
            let $max_sesno = if $after.sesno > $before.sesno?:0 { $after.sesno } else { $before.sesno };
            -- 根据事件类型处理  type::record("dbnum_info_table", $ref_0)
            IF $event = "CREATE"   {
                UPSERT type::record('dbnum_info_table', $ref_0) MERGE {
                    dbnum: $dbnum,
                    count: count?:0 + 1,
                    sesno: $max_sesno,
                    max_ref1: $ref_1
                };
            } ELSE IF $event = "DELETE" OR $is_delete  {
                UPSERT type::record('dbnum_info_table', $ref_0) MERGE {
                    count: count - 1,
                    sesno: $max_sesno,
                    max_ref1: $ref_1
                }
                WHERE count > 0;
            };
        };
    "#;

    project_primary_db().query(event_sql).await?;
    Ok(())
}

#[cfg(not(feature = "surreal-save"))]
pub async fn define_dbnum_event_array_id() -> anyhow::Result<()> {
    Ok(())
}

#[cfg(feature = "sql")]
pub async fn execute_sql(conn: &Pool<MySql>, sql: &str) -> bool {
    return match conn.execute(sql).await {
        Ok(_) => true,
        Err(e) => {
            match &e {
                Error::Database(error) => {
                    //index already exist
                    if error.code() == Some(Cow::from("42000")) {
                    } else {
                        dbg!(sql);
                    }
                }
                _ => {
                    dbg!(&e);
                }
            }
            false
        }
    };
}

/// 带进度回调的多线程同步数据
pub async fn sync_total_async_threaded_with_callback<F>(
    db_option: &DbOption,
    project: &str,
    cur_dbno_set: Arc<DashSet<u32>>,
    db_types: &[&str],
    proj_progress_chunk: usize,
    progress_callback: &mut Option<F>,
    current_project: usize,
    total_projects: usize,
) -> anyhow::Result<()>
where
    F: FnMut(&str, usize, usize, usize, usize, usize, usize) + Send,
{
    info!("开始解析 {project} 的 {:?}", db_types);
    let db_option_arc = Arc::new(db_option.clone());

    let project_dir = db_option
        .get_project_path(project)
        .ok_or_else(|| anyhow::anyhow!("项目路径不存在: {}", project))?;

    if !Path::new(&project_dir).exists() {
        dbg!("项目文件夹指定不正确");
        return Err(anyhow::anyhow!("项目文件夹指定不正确"));
    }

    // 获取并统计文件
    let mut children_files = {
        let target_dir = std::fs::read_dir(&project_dir)
            .unwrap()
            .into_iter()
            .map(|entry| {
                let entry = entry.unwrap();
                entry.path()
            })
            .find(|x| x.is_dir() && x.file_name().unwrap().to_str().unwrap().ends_with("000"))
            .unwrap();
        std::fs::read_dir(target_dir)?
            .into_iter()
            .map(|entry| {
                let entry = entry.unwrap();
                entry.path()
            })
            .collect::<Vec<PathBuf>>()
    };

    // 处理文件名_0001和文件名同时存在的情况
    let mut file_map = HashMap::new();
    for path in children_files.iter() {
        let file_name = path.file_stem().unwrap().to_str().unwrap();
        if let Some(base_name) = file_name.strip_suffix("_0001") {
            file_map.insert(base_name.to_string(), path.clone());
        } else {
            if !file_map.contains_key(file_name) {
                file_map.insert(file_name.to_string(), path.clone());
            }
        }
    }

    children_files = file_map.into_values().collect();
    let total_files = children_files.len();

    // 通知进度回调文件统计完成
    if let Some(callback) = progress_callback {
        callback(
            project,
            current_project,
            total_projects,
            0,
            total_files,
            0,
            0,
        );
    }

    // 继续原有的处理逻辑...
    let project = Arc::new(project.to_string());
    let mut is_replace = db_option_arc.replace_dbs;
    let replace_types = db_option_arc.replace_types.clone();
    let b_replace_types = replace_types.is_some();
    let b_save_mysql = db_option_arc.sync_tidb.unwrap_or(false);
    if b_replace_types {
        is_replace = true;
    }
    let chunk_size = resolve_sync_chunk_size(db_option_arc.sync_chunk_size, 10_0000);

    const CHUNK_SIZE: usize = 100;
    let (sender, receiver) = flume::unbounded();

    // 启动数据库写入任务
    let mut insert_handles = FuturesUnordered::new();
    for i in 0..16 {
        let receiver: flume::Receiver<SenderJsonsData> = receiver.clone();
        #[cfg(feature = "sql")]
        let pool = AiosDBManager::get_project_pool().await.unwrap().clone();

        let insert_handle = tokio::task::spawn(async move {
            // 使用 ready_chunks 而不是 chunks，这样可以在 channel 关闭时立即处理剩余数据
            use futures::stream::StreamExt;
            let mut record_stream = receiver.into_stream().ready_chunks(200);
            while let Some(stream) = record_stream.next().await {
                for data in stream {
                    match data {
                        #[cfg(feature = "surreal-save")]
                        SenderJsonsData::PEJson(pes) => {
                            if !pes.is_empty() {
                                dbg!(pes.len());
                                let sql = format!("INSERT IGNORE INTO pe [{}]", pes.join(","));

                                // 保存到主数据库
                                project_primary_db()
                                    .query(&sql)
                                    .await
                                    .expect("insert pes transport failed")
                                    .check()
                                    .expect("insert pes statement failed");

                                // 如果启用了 mem-kv-save，同时保存到备份数据库
                                #[cfg(feature = "mem-kv-save")]
                                {
                                    match SUL_MEM_DB.query(&sql).await {
                                        Ok(_) => {}
                                        Err(e) => {
                                            log::warn!("保存PE到内存KV数据库失败: {}", e);
                                        }
                                    }
                                }
                            }
                        }
                        #[cfg(not(feature = "surreal-save"))]
                        SenderJsonsData::PEJson(pes) => {
                            let _ = pes;
                        }
                        #[cfg(feature = "surreal-save")]
                        SenderJsonsData::PERelateJson(relates) => {
                            if !relates.is_empty() {
                                // specs/023 T007：批内元素已是完整语句
                                // （每 owner 先 DELETE 后 INSERT RELATION，幂等），直接拼接执行。
                                let sql = relates.join("\n");

                                // 保存到主数据库
                                project_primary_db()
                                    .query(&sql)
                                    .await
                                    .expect("insert pe_owner transport failed")
                                    .check()
                                    .expect("insert pe_owner statement failed");

                                // 如果启用了 mem-kv-save，同时保存到备份数据库
                                #[cfg(feature = "mem-kv-save")]
                                {
                                    match SUL_MEM_DB.query(&sql).await {
                                        Ok(_) => {}
                                        Err(e) => {
                                            log::warn!("保存PE关系到内存KV数据库失败: {}", e);
                                        }
                                    }
                                }
                            }
                        }
                        #[cfg(not(feature = "surreal-save"))]
                        SenderJsonsData::PERelateJson(relates) => {
                            let _ = relates;
                        }
                        #[cfg(feature = "surreal-save")]
                        SenderJsonsData::EleReuseRelateJson(relates) => {
                            if !relates.is_empty() {
                                ensure_ele_reuse_relate_relation_schema().await;
                                let sql = format!(
                                    "INSERT RELATION INTO ele_reuse_relate [{}]",
                                    relates.join(",")
                                );

                                // 保存到主数据库
                                project_primary_db()
                                    .query(&sql)
                                    .await
                                    .expect("insert ele_reuse_relate transport failed")
                                    .check()
                                    .expect("insert ele_reuse_relate statement failed");

                                // 如果启用了 mem-kv-save，同时保存到备份数据库
                                #[cfg(feature = "mem-kv-save")]
                                {
                                    match SUL_MEM_DB.query(&sql).await {
                                        Ok(_) => {}
                                        Err(e) => {
                                            log::warn!(
                                                "保存ele_reuse_relate到内存KV数据库失败: {}",
                                                e
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        #[cfg(not(feature = "surreal-save"))]
                        SenderJsonsData::EleReuseRelateJson(relates) => {
                            let _ = relates;
                        }
                        #[cfg(feature = "surreal-save")]
                        SenderJsonsData::AttJson((type_name, jsons)) => {
                            if !jsons.is_empty() {
                                let sql = format!(
                                    "INSERT IGNORE INTO {} [{}]",
                                    type_name,
                                    jsons.join(",")
                                );
                                project_primary_db()
                                    .query(sql)
                                    .await
                                    .expect("insert att transport failed")
                                    .check()
                                    .expect("insert att statement failed");
                            }
                        }
                        #[cfg(not(feature = "surreal-save"))]
                        SenderJsonsData::AttJson((_type_name, jsons)) => {
                            let _ = jsons;
                        }
                        #[cfg(feature = "surreal-save")]
                        SenderJsonsData::DbnumInfoUpdate(sqls) => {
                            for sql in sqls {
                                project_primary_db()
                                    .query(sql)
                                    .await
                                    .expect("update dbnum_info transport failed")
                                    .check()
                                    .expect("update dbnum_info statement failed");
                            }
                        }
                        #[cfg(not(feature = "surreal-save"))]
                        SenderJsonsData::DbnumInfoUpdate(sqls) => {
                            let _ = sqls;
                        }
                        #[cfg(feature = "surreal-save")]
                        SenderJsonsData::PartitionedPEJson { table_name, sql } => {
                            // 保存简化PE数据到分表
                            log::debug!("插入到分表 {}", table_name);
                            project_primary_db()
                                .query(&sql)
                                .await
                                .expect("insert partitioned pe transport failed")
                                .check()
                                .expect("insert partitioned pe statement failed");

                            // 如果启用了 mem-kv-save，同时保存到备份数据库
                            #[cfg(feature = "mem-kv-save")]
                            {
                                match SUL_MEM_DB.query(&sql).await {
                                    Ok(_) => {}
                                    Err(e) => {
                                        log::warn!(
                                            "保存分表PE到内存KV数据库失败: {} | 表: {}",
                                            e,
                                            table_name
                                        );
                                    }
                                }
                            }
                        }
                        #[cfg(not(feature = "surreal-save"))]
                        SenderJsonsData::PartitionedPEJson { table_name, sql } => {
                            let _ = (table_name, sql);
                        }
                        #[cfg(feature = "sql")]
                        SenderJsonsData::MySqlJson((table_name, jsons)) => {
                            if b_save_mysql && !jsons.is_empty() {
                                let sql = format!(
                                    "INSERT IGNORE INTO {} VALUES {}",
                                    table_name,
                                    jsons.join(",")
                                );
                                match sqlx::query(&sql).execute(&pool).await {
                                    Ok(_) => {}
                                    Err(e) => {
                                        dbg!(e.to_string());
                                    }
                                }
                            }
                        }
                        SenderJsonsData::MysqlSql((table_name, sql)) => {
                            // 处理MySQL SQL语句
                            #[cfg(feature = "sql")]
                            if b_save_mysql {
                                match sqlx::query(&sql).execute(&pool).await {
                                    Ok(_) => {}
                                    Err(e) => {
                                        dbg!(e.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });
        insert_handles.push(insert_handle);
    }
    // ========== 文件解析与写库主循环（带进度回调） ==========
    // 为保持与非回调版本一致，这里直接内联主循环，避免将 progress_callback 移入子任务造成生命周期/Send 限制。

    // 与非回调版本保持一致的控制参数
    let db_types_clone = db_types.iter().map(|&x| x.to_string()).collect::<Vec<_>>();
    let is_parse_sys = db_types_clone.contains(&"SYST".to_string());
    let gen_db_meta_only = db_option.gen_db_meta_only;
    #[cfg(feature = "surreal-save")]
    let is_save_db = db_option.is_save_db()
        // gen_db_meta_only 仅用于 total_sync 全量解析时“只生成 tree”的场景；
        // 其它情况下（尤其是模型生成）不应影响写库。
        && !(gen_db_meta_only && db_option.total_sync);
    #[cfg(not(feature = "surreal-save"))]
    let is_save_db = false;
    let is_sync_history = db_option.is_sync_history();
    let is_total_sync = db_option.total_sync;
    let sync_versioned = db_option.sync_versioned.unwrap_or(false);
    let selected_file_names = selected_db_file_names(db_option);
    let selected_dbnums = selected_dbnums(db_option);
    let force_include = is_parse_sys && is_total_sync && selected_file_names.is_empty();
    // CATA 闭包部分解析过滤器（spec 002 T006b）：默认 Off=None=整库解析；
    // AIOS_CATA_CLOSURE_MODE=manifest 时从 cata_closure.json 加载，缺失即整库回退。
    let cata_filter =
        crate::data_interface::cata_closure::load_sync_filter(project.as_str(), &db_types_clone);
    let mut parsed_artifacts = Vec::new();
    // specs/022 T010：本轮解析成功的 (dbnum, latest_sesno)，写库任务 join 后固化为 full 锚点。
    let mut pending_full_version_anchors: Vec<(u32, u32, String)> = Vec::new();
    // spec 004：解析阶段总耗时计时起点。
    let sync_stage_started = Instant::now();

    for (file_idx, path) in children_files.into_iter().enumerate() {
        let total_files = total_files; // 仅为语义清晰
        let file_name = path.file_name().unwrap().to_str().unwrap().to_string();
        if file_name.contains('.') {
            // 进入文件（将其计入进度），随即跳过
            if let Some(cb) = progress_callback.as_mut() {
                cb(
                    project.as_str(),
                    current_project,
                    total_projects,
                    file_idx + 1,
                    total_files,
                    0,
                    0,
                );
            }
            continue;
        }

        // 进入文件 - 上报当前文件号
        if let Some(cb) = progress_callback.as_mut() {
            cb(
                project.as_str(),
                current_project,
                total_projects,
                file_idx + 1,
                total_files,
                0,
                0,
            );
        }

        let dbno_set = cur_dbno_set.clone();
        let mut time = Instant::now();

        // 读取文件头，判定 db_type / dbnum
        let mut file = File::open(&path).await.unwrap();
        let mut buf = vec![0u8; 60];
        file.read_exact(&mut buf).await.unwrap();
        let db_basic_info = parse_file_basic_info(&buf);
        let db_type = db_basic_info.db_type;
        let dbnum = db_basic_info.dbnum;

        if !should_process_sync_file(
            &file_name,
            dbnum,
            &selected_file_names,
            &selected_dbnums,
            force_include,
        ) {
            if let Some(cb) = progress_callback.as_mut() {
                cb(
                    project.as_str(),
                    current_project,
                    total_projects,
                    file_idx + 1,
                    total_files,
                    0,
                    0,
                );
            }
            continue;
        }

        // 类型过滤
        if !db_types_clone.contains(&db_type) {
            // 依然汇报一次该文件完成
            if let Some(cb) = progress_callback.as_mut() {
                cb(
                    project.as_str(),
                    current_project,
                    total_projects,
                    file_idx + 1,
                    total_files,
                    0,
                    0,
                );
            }
            continue;
        }
        // 避免重复
        if dbno_set.contains(&dbnum) {
            if let Some(cb) = progress_callback.as_mut() {
                cb(
                    project.as_str(),
                    current_project,
                    total_projects,
                    file_idx + 1,
                    total_files,
                    0,
                    0,
                );
            }
            continue;
        }
        dbno_set.insert(dbnum);

        // 读取 sesno、存储 refno->sesno map
        let mut ses_range_map: BTreeMap<i32, Range<u32>> = BTreeMap::new();
        let mut sesno = 0;
        let mut sesno_timestamp: Option<i64> = None;
        {
            let mut io = PdmsIO::new(project.as_str(), path.clone(), true);
            if io.open().is_ok() {
                sesno = match io.get_latest_sesno() {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(
                            "get_latest_sesno failed(file={}): {} (fallback sesno=0)",
                            file_name, e
                        );
                        0
                    }
                };
                if sesno > 0 {
                    sesno_timestamp = io.get_sesno_timestamp(sesno).ok();
                }

                if sesno == 0 && is_sync_history {
                    // 同步历史需要有效 sesno；否则无法进行该流程。
                    if let Some(cb) = progress_callback.as_mut() {
                        cb(
                            project.as_str(),
                            current_project,
                            total_projects,
                            file_idx + 1,
                            total_files,
                            0,
                            0,
                        );
                    }
                    continue;
                }

                if is_sync_history {
                    if let Err(e) = pdms_sync_history_compat(&mut io).await {
                        warn!("sync_history failed(file={}): {}", file_name, e);
                    }
                    if let Some(cb) = progress_callback.as_mut() {
                        cb(
                            project.as_str(),
                            current_project,
                            total_projects,
                            file_idx + 1,
                            total_files,
                            0,
                            0,
                        );
                    }
                    continue;
                } else if sesno > 0 {
                    // 仅在需要保存数据库时才存储 refno sesno map
                    if is_save_db {
                        if let Err(e) = pdms_store_refno_sesno_map_compat(&mut io).await {
                            warn!(
                                "store_all_refno_sesno_map failed(file={}): {} (continue parsing)",
                                file_name, e
                            );
                        }
                    }
                    // pdms-io-fork 的 ses_range_map 使用 RangeInclusive；parse_pdms_db 仍期望 Range（右开区间）
                    ses_range_map = io
                        .ses_range_map
                        .into_iter()
                        .map(|(k, r)| {
                            let start = *r.start();
                            let end_exclusive = r.end().saturating_add(1);
                            (k, start..end_exclusive)
                        })
                        .collect();
                }
            } else {
                // open 失败时仍允许继续解析（Meili/Tree 生成不依赖 session 信息）
                warn!(
                    "PdmsIO::open failed(file={}): continue without ses range map",
                    file_name
                );
            }
        }

        let project_name = project.as_str().to_string();
        let mut db_basic =
            parse_file_db_basic_data(&path, &file_name, project_name.as_str()).unwrap_or_default();
        let all_refnos: Vec<_> = db_basic
            .refno_table_map
            .iter()
            .map(|entry| *entry.key())
            .collect();
        // CATA 闭包部分解析（spec 002 T006b）：manifest 命中则裁剪 refno 全集，否则整库回退。
        let all_refnos = crate::data_interface::cata_closure::apply_sync_filter(
            cata_filter.as_ref(),
            &db_type,
            dbnum,
            all_refnos,
        );
        let total_chunks = std::cmp::max(1, (all_refnos.len() + chunk_size - 1) / chunk_size);

        let db_basic = Arc::new(db_basic);
        if is_save_db {
            save_pe_relates(&db_basic, sender.clone()).await;
        }
        // 解析时不应该受 debug_model_refnos 影响，只用于模型生成调试
        // 如果需要调试解析过程，应该使用独立的 debug_parse_refnos 配置
        let debug_refnos: Vec<RefU64> = Vec::new(); // 暂时禁用解析调试模式
        let is_debug = !debug_refnos.is_empty();
        if is_debug {
            if let Some(children) = db_basic.children_map.get(&debug_refnos[0]) {
                dbg!(children);
            }
        }
        let debug_refnos = Arc::new(debug_refnos);
        let mut tree_nodes: HashMap<RefU64, TreeNodeMeta> = HashMap::new();

        let chunk_stage_start = Instant::now();
        let mut total_cnt = 0;
        for (chunk_index, chunk) in all_refnos.chunks(chunk_size).enumerate() {
            let db_option_clone = db_option_arc.clone();
            let file_name_clone = file_name.clone();
            let chunk_refnos = chunk.to_vec();
            let project_name_clone = project_name.clone();
            let db_basic_clone = db_basic.clone();
            let debug_refnos = debug_refnos.clone();
            let ses_range_map_clone = ses_range_map.clone();
            let ignore_world_refno = true;

            let parse_heartbeat = start_parse_progress_heartbeat(
                project.as_str().to_string(),
                file_name.clone(),
                dbnum,
                db_type.clone(),
                is_save_db,
                all_refnos.len(),
                total_chunks,
                chunk_index,
                chunk_index.checked_sub(1).map(|idx| idx + 1),
                total_cnt,
                chunk_stage_start,
            );
            let parse_result = parse_file_with_chunk(
                db_basic_clone.clone(),
                &file_name_clone,
                project_name_clone.as_str(),
                &chunk_refnos,
                &ses_range_map_clone,
                ignore_world_refno,
            )
            .await;

            match parse_result {
                Ok(PdmsDbData {
                    total_attr_map,
                    type_ele_map,
                    dbnum: dbnum,
                    ..
                }) => {
                    let total_attr_map_arc = Arc::new(total_attr_map);
                    total_cnt += total_attr_map_arc.len();
                    for entry in total_attr_map_arc.iter() {
                        let refno = *entry.key();
                        let att = entry.value();
                        let noun = att.get_type_hash();
                        let owner = att.get_owner().refno();
                        let cata_hash = att.cal_cata_hash();
                        tree_nodes.entry(refno).or_insert(TreeNodeMeta {
                            refno,
                            owner,
                            noun,
                            cata_hash,
                        });
                    }
                    let should_save = !is_debug && is_save_db;
                    if should_save {
                        save_pes(
                            &db_basic_clone,
                            &total_attr_map_arc,
                            dbnum as i32,
                            &file_name_clone,
                            &db_type,
                            &db_option_clone,
                            sender.clone(),
                        )
                        .await
                        .expect("save pes failed");
                    }
                    // UDA 类型写入
                    for kv in type_ele_map.iter() {
                        let noun: i32 = *kv.key() as _;
                        let type_name = db1_dehash(noun as _);
                        if type_name.is_empty() {
                            continue;
                        }
                        for refnos in &kv.value().iter().chunks(db_option_clone.att_chunk as _) {
                            let mut json_vec = vec![];
                            let mut uda_json_vec = vec![];
                            for refno in refnos {
                                let att = total_attr_map_arc.get(refno).unwrap();
                                if is_debug {
                                    if debug_refnos.contains(&att.get_refno_or_default().refno()) {
                                        dbg!(att.value());
                                    } else {
                                        continue;
                                    }
                                }
                                if !is_save_db {
                                    continue;
                                }
                                if let Some(json) = att.gen_sur_json() {
                                    json_vec.push(json);
                                }
                                if let Some(json) = att.gen_sur_json_uda(&[]) {
                                    uda_json_vec.push(normalize_sql_string(&json));
                                }
                            }
                            if is_save_db {
                                if !json_vec.is_empty() {
                                    sender
                                        .send(SenderJsonsData::AttJson((
                                            type_name.clone(),
                                            json_vec,
                                        )))
                                        .expect("send attmap sql failed");
                                }
                                if !uda_json_vec.is_empty() {
                                    sender
                                        .send(SenderJsonsData::AttJson((
                                            "ATT_UDA".to_string(),
                                            uda_json_vec,
                                        )))
                                        .expect("send attmap sql failed");
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    dbg!(e.to_string());
                }
            }

            // 分块进度
            if let Some(cb) = progress_callback.as_mut() {
                cb(
                    project.as_str(),
                    current_project,
                    total_projects,
                    file_idx + 1,
                    total_files,
                    chunk_index + 1,
                    total_chunks,
                );
            }
            crate::perf_metrics::record_parse_progress(crate::perf_metrics::ParseProgressUpdate {
                stage: "chunk_done",
                project_name: project.as_str(),
                file_name: &file_name,
                dbnum,
                db_type: &db_type,
                save_db: is_save_db,
                refnos_total: all_refnos.len(),
                chunks_total: total_chunks,
                chunks_completed: chunk_index + 1,
                last_chunk: Some(chunk_index + 1),
                parsed_attrs: total_cnt,
                elapsed_ms: chunk_stage_start.elapsed().as_millis() as u64,
            });
            parse_heartbeat.stop();
        }
        let output_dir = parse_tree_output_dir(&project_name);
        // ref0s 必须基于整库 refno 全集（refno_table_map）收集，而非 tree_nodes：
        // CATA 闭包部分解析时 tree_nodes 只覆盖闭包子集，db_meta 的 ref0->dbnum 映射不能缩水。
        let mut ref0s = BTreeSet::new();
        for entry in db_basic.refno_table_map.iter() {
            let ref0 = entry.key().get_0();
            if ref0 != 0 && ref0 != 0x8000_0001 {
                ref0s.insert(ref0);
            }
        }

        let header_hex_60 = (|| -> Option<String> {
            let mut f = std::fs::File::open(&path).ok()?;
            let mut buf = [0u8; 60];
            f.read_exact(&mut buf).ok()?;
            Some(hex::encode(buf))
        })();

        db_meta_info::update_db_meta_info_json(
            &output_dir,
            db_meta_info::DbFileMetaUpdate {
                dbnum,
                db_type: &db_type,
                file_name: &file_name,
                file_path: &path,
                header_hex_60,
                header_debug: None,
                latest_sesno: Some(sesno as u32),
                sesno_timestamp,
                ref0s,
            },
        )
        .map_err(|e| {
            anyhow::anyhow!(
                "[db_meta_info] 更新失败(dbnum={}, file={}): {}",
                dbnum,
                file_name,
                e
            )
        })?;

        // specs/022 T010：登记本轮解析完成的 dbnum，收尾（写库任务 join 后）统一固化 full 锚点。
        if is_save_db && sesno > 0 {
            pending_full_version_anchors.push((
                dbnum,
                sesno as u32,
                full_sync_source_evidence(&path),
            ));
        }

        parsed_artifacts.push(ParsedDbArtifact {
            project_name: project_name.clone(),
            tree_dir: output_dir.clone(),
            dbnum,
            db_type: db_type.clone(),
            file_name: file_name.clone(),
            tree_node_count: tree_nodes.len(),
        });

        // spec 004：解析阶段每库指标（mode/total 已由 apply_sync_filter 记录）。
        crate::perf_metrics::record_parse_db(
            dbnum,
            &db_type,
            total_cnt as usize,
            time.elapsed().as_millis() as u64,
        );

        info!(
            "解析任务完成, 耗时: {} s, 总数量: {}",
            time.elapsed().as_secs_f32(),
            total_cnt
        );
        // 文件完成：若无分块也至少回报一次
        if let Some(cb) = progress_callback.as_mut() {
            cb(
                project.as_str(),
                current_project,
                total_projects,
                file_idx + 1,
                total_files,
                total_chunks,
                total_chunks,
            );
        }
    }

    // 等待所有写入任务完成
    drop(sender);
    while let Some(result) = insert_handles.next().await {
        result.map_err(|e| anyhow::anyhow!("全量写入 worker 失败: {e}"))?;
    }
    // specs/022 T010：写库任务已全部 join，此刻固化 full 锚点晚于本轮全部写入。
    write_full_version_anchors(&pending_full_version_anchors).await?;
    validate_parse_scene_tree_artifacts(&parsed_artifacts)?;
    crate::perf_metrics::finish_parse_stage(
        parse_failed_sql_count(),
        sync_stage_started.elapsed().as_millis() as u64,
    );

    Ok(())
}

/// spec 004：解析阶段错误计数（failed_sql 转储数；未编译 gen_model 时恒为 0）。
fn parse_failed_sql_count() -> usize {
    #[cfg(feature = "gen_model")]
    {
        crate::fast_model::gen_model::pdms_inst::failed_sql_dump_count()
    }
    #[cfg(not(feature = "gen_model"))]
    {
        0
    }
}

#[cfg(feature = "generation-read-ducklake")]
type SyncProgressCallback<'a> =
    dyn FnMut(&str, usize, usize, usize, usize, usize, usize) + Send + 'a;

#[cfg(feature = "generation-read-ducklake")]
#[derive(Debug, Clone, serde::Serialize)]
struct DuckLakeParseDbReport {
    run_id: String,
    project: String,
    file_name: String,
    dbnum: u32,
    db_type: String,
    sesno: u32,
    stage_path: PathBuf,
    stage_fingerprint: String,
    authoritative_snapshot_id: u64,
    manifest_hash: String,
    replica_version_time: String,
    counts: crate::version_store::ParseStageCounts,
    authority_idempotent: bool,
}

#[cfg(feature = "generation-read-ducklake")]
fn ducklake_parse_run_id() -> String {
    static RUN_SEQUENCE: AtomicU64 = AtomicU64::new(0);
    if let Ok(explicit) = std::env::var("AIOS_PARSE_RUN_ID")
        && !explicit.trim().is_empty()
    {
        return explicit;
    }
    format!(
        "parse-{}-{}-{}",
        chrono::Utc::now().format("%Y%m%dT%H%M%S%3fZ"),
        std::process::id(),
        RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(feature = "generation-read-ducklake")]
fn build_parsed_fact_batch(
    batch_id: String,
    dbnum: u32,
    project: &str,
    db_type: &str,
    db_basic: &DbBasicData,
    total_attr_map: &DashMap<RefU64, NamedAttrMap>,
    include_catalog: bool,
) -> anyhow::Result<ParsedFactBatch> {
    let mut elements = Vec::with_capacity(total_attr_map.len());
    let mut hierarchy_rows = Vec::with_capacity(total_attr_map.len());
    let mut ordered = total_attr_map
        .iter()
        .map(|entry| (*entry.key(), entry.value().clone()))
        .collect::<Vec<_>>();
    ordered.sort_by_key(|(refno, _)| refno.0);
    for (raw_refno, attributes) in ordered {
        let refno = RefnoEnum::from(raw_refno);
        anyhow::ensure!(refno.is_valid(), "解析结果包含非法 refno={raw_refno}");
        let owner = attributes.get_owner();
        anyhow::ensure!(
            owner.is_valid() || owner.is_unset(),
            "解析结果包含非法 owner: refno={refno} owner={owner}"
        );
        let name =
            crate::api::element::cal_default_name(raw_refno, &attributes, &db_basic.children_map);
        let has_children = db_basic
            .children_map
            .get(&raw_refno)
            .is_some_and(|children| !children.is_empty());
        elements.push(VersionStoreElement {
            element: crate::generation_read::ElementSnapshot {
                refno,
                dbnum,
                owner,
                noun: attributes.get_type_str().to_string(),
                name,
                has_children,
            },
            attributes: crate::generation_read::AttributeSet::from_named_attr_map(
                refno,
                &attributes,
            ),
        });
        if owner.is_valid() {
            let ordinal = db_basic
                .children_map
                .get(&owner.refno())
                .and_then(|children| children.iter().position(|child| *child == raw_refno))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "children_map 缺少 owner-child 顺序: owner={owner} child={refno}"
                    )
                })?;
            hierarchy_rows.push(crate::generation_read::HierarchyRow {
                dbnum,
                parent: owner,
                child: refno,
                ordinal: u32::try_from(ordinal)
                    .map_err(|_| anyhow::anyhow!("hierarchy ordinal 超出 u32: {ordinal}"))?,
            });
        }
    }

    let db_catalog = if include_catalog {
        let mut ref0 = db_basic.world_refno.get_0();
        if ref0 == 0 || ref0 == 0x8000_0001 {
            ref0 = db_basic
                .refno_table_map
                .iter()
                .map(|entry| entry.key().get_0())
                .find(|value| *value != 0 && *value != 0x8000_0001)
                .unwrap_or_default();
        }
        vec![DbCatalogEntry {
            dbnum,
            ref0: (ref0 != 0).then_some(ref0),
            db_type: db_type.to_string(),
            project: project.to_string(),
        }]
    } else {
        Vec::new()
    };
    let batch = ParsedFactBatch {
        batch_id,
        dbnum,
        elements,
        hierarchy_rows,
        pline_facts: Vec::new(),
        db_catalog,
    };
    batch.validate()?;
    Ok(batch)
}

#[cfg(feature = "generation-read-ducklake")]
fn write_ducklake_parse_artifacts(
    project: &str,
    path: &PathBuf,
    file_name: &str,
    dbnum: u32,
    db_type: &str,
    sesno: u32,
    sesno_timestamp: Option<i64>,
    db_basic: &DbBasicData,
    tree_nodes: &HashMap<RefU64, TreeNodeMeta>,
) -> anyhow::Result<ParsedDbArtifact> {
    let output_dir = parse_tree_output_dir(project);
    let ref0s = db_basic
        .refno_table_map
        .iter()
        .map(|entry| entry.key().get_0())
        .filter(|ref0| *ref0 != 0 && *ref0 != 0x8000_0001)
        .collect();
    let header_hex_60 = (|| -> Option<String> {
        let mut file = std::fs::File::open(path).ok()?;
        let mut buffer = [0u8; 60];
        file.read_exact(&mut buffer).ok()?;
        Some(hex::encode(buffer))
    })();
    db_meta_info::update_db_meta_info_json(
        &output_dir,
        db_meta_info::DbFileMetaUpdate {
            dbnum,
            db_type,
            file_name,
            file_path: path,
            header_hex_60,
            header_debug: None,
            latest_sesno: Some(sesno),
            sesno_timestamp,
            ref0s,
        },
    )?;
    Ok(ParsedDbArtifact {
        project_name: project.to_string(),
        tree_dir: output_dir,
        dbnum,
        db_type: db_type.to_string(),
        file_name: file_name.to_string(),
        tree_node_count: tree_nodes.len(),
    })
}

#[cfg(feature = "generation-read-ducklake")]
async fn publish_ducklake_stage(
    stager: DuckLakeParseStager,
    version: ParseStageVersion,
    authority: DuckLakeAuthority,
    project: String,
    file_name: String,
    db_type: String,
) -> anyhow::Result<DuckLakeParseDbReport> {
    let counts = stager.finalize_transforms().await?;
    let sealed = stager.seal(version)?;
    drop(stager);

    let authority_for_commit = authority.clone();
    let stage_for_commit = sealed.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        authority_for_commit.commit_staged_db(&stage_for_commit)
    })
    .await
    .map_err(|error| anyhow::anyhow!("DuckLake staged commit task join failed: {error}"))??;

    let marker = DuckLakeParseStager::open_path(&sealed.path)?;
    let committed = marker.mark_authority_committed(outcome.snapshot_id)?;
    drop(marker);
    let payload = committed.load_payload()?;
    let previous_snapshot_id = {
        let authority = authority.clone();
        tokio::task::spawn_blocking(move || {
            authority.previous_data_snapshot_id(outcome.snapshot_id)
        })
        .await
        .map_err(|error| anyhow::anyhow!("读取 DuckLake 前序 snapshot 失败: {error}"))??
    };
    let replica = SurrealReplicaStore;
    let binding = if let Some(binding) = replica.binding(outcome.snapshot_id).await? {
        anyhow::ensure!(
            binding.manifest_hash == outcome.manifest.manifest_hash,
            "已有 replica binding 与 authority manifest 不一致"
        );
        binding
    } else {
        let batch = ReplicaApplyBatch {
            authoritative_snapshot_id: outcome.snapshot_id,
            previous_snapshot_id,
            manifest: outcome.manifest.clone(),
            replace_dbnums: BTreeSet::from([committed.dbnum]),
            upsert_elements: payload
                .elements
                .into_iter()
                .map(|item| ReplicaElement {
                    element: item.element,
                    attributes: item.attributes,
                })
                .collect(),
            delete_refnos: BTreeMap::new(),
            hierarchy_rows: payload.hierarchy_rows,
            transforms: payload.transforms,
            db_catalog: payload
                .db_catalog
                .into_iter()
                .map(|entry| ReplicaDbCatalogEntry {
                    dbnum: entry.dbnum,
                    db_type: entry.db_type,
                    project: entry.project,
                })
                .collect(),
            payload_hash: String::new(),
        }
        .seal()?;
        replica.apply(&batch).await?
    };
    let replica_manifest = replica.manifest_at(&binding).await?;
    anyhow::ensure!(
        replica_manifest.manifest_hash == outcome.manifest.manifest_hash,
        "replica/authority manifest 双向校验失败: replica={} authority={}",
        replica_manifest.manifest_hash,
        outcome.manifest.manifest_hash
    );
    let marker = DuckLakeParseStager::open_path(&committed.path)?;
    let applied = marker.mark_replica_applied(&binding.replica_version_time)?;
    drop(marker);
    Ok(DuckLakeParseDbReport {
        run_id: applied.run_id,
        project,
        file_name,
        dbnum: applied.dbnum,
        db_type,
        sesno: applied.version.to_sesno,
        stage_path: applied.path,
        stage_fingerprint: applied.fingerprint,
        authoritative_snapshot_id: outcome.snapshot_id,
        manifest_hash: outcome.manifest.manifest_hash,
        replica_version_time: binding.replica_version_time,
        counts,
        authority_idempotent: outcome.idempotent,
    })
}

#[cfg(feature = "generation-read-ducklake")]
async fn sync_pdms_to_ducklake(
    db_option: &DbOption,
    parse_config: crate::options::ParseStorageConfig,
    authority_config: DuckLakeConfig,
    mut progress_callback: Option<&mut SyncProgressCallback<'_>>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        parse_config.backend.uses_ducklake(),
        "sync_pdms_to_ducklake 仅接受 ducklake backend"
    );
    anyhow::ensure!(!db_option.included_projects.is_empty(), "没有包含的项目");
    let save_db = db_option.is_save_db() && !(db_option.gen_db_meta_only && db_option.total_sync);
    let run_id = ducklake_parse_run_id();
    let authority = if save_db {
        Some(
            tokio::task::spawn_blocking(move || DuckLakeAuthority::open(authority_config))
                .await
                .map_err(|error| anyhow::anyhow!("打开 DuckLake authority 失败: {error}"))??,
        )
    } else {
        None
    };
    let selected_file_names = selected_db_file_names(db_option);
    let selected_dbnums = selected_dbnums(db_option);
    let mut reports = Vec::new();
    let mut artifacts = Vec::new();

    for (project_index, project) in db_option.included_projects.iter().enumerate() {
        let project_dir = db_option
            .get_project_path(project)
            .ok_or_else(|| anyhow::anyhow!("项目路径不存在: {project}"))?;
        let files = collect_project_db_files(&project_dir)?;
        let mut allowed_types = resolve_data_sync_db_types(db_option, project)?
            .into_iter()
            .collect::<BTreeSet<_>>();
        if db_option.total_sync || db_option.only_sync_sys {
            allowed_types.extend(SYSTEM_SYNC_DB_TYPES.iter().map(|value| value.to_string()));
        }
        if db_option.only_sync_sys {
            allowed_types.retain(|value| SYSTEM_SYNC_DB_TYPES.contains(&value.as_str()));
        }
        let total_files = files.len();
        if let Some(callback) = progress_callback.as_deref_mut() {
            callback(
                project,
                project_index + 1,
                db_option.included_projects.len(),
                0,
                total_files,
                0,
                0,
            );
        }

        for (file_index, path) in files.into_iter().enumerate() {
            let file_name = path
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| anyhow::anyhow!("PDMS 文件名不是 UTF-8: {}", path.display()))?
                .to_string();
            if file_name.contains('.') {
                continue;
            }
            let mut header = [0_u8; 60];
            std::fs::File::open(&path)?.read_exact(&mut header)?;
            let basic_info = parse_file_basic_info(&header);
            let dbnum = basic_info.dbnum;
            let db_type = basic_info.db_type;
            if !allowed_types.contains(&db_type)
                || !should_process_sync_file(
                    &file_name,
                    dbnum,
                    &selected_file_names,
                    &selected_dbnums,
                    false,
                )
            {
                continue;
            }

            let (sesno, sesno_timestamp) = {
                let mut io = PdmsIO::new(project, path.clone(), true);
                io.open()
                    .with_context(|| format!("打开 PDMS 文件失败: {}", path.display()))?;
                let sesno = io
                    .get_latest_sesno()
                    .with_context(|| format!("读取 latest sesno 失败: {file_name}"))?;
                anyhow::ensure!(
                    !save_db || sesno > 0,
                    "DuckLake 发布要求有效 sesno: file={file_name}"
                );
                (
                    sesno,
                    (sesno > 0)
                        .then(|| io.get_sesno_timestamp(sesno).ok())
                        .flatten(),
                )
            };
            let db_basic = Arc::new(
                parse_file_db_basic_data(&path, &file_name, project)
                    .with_context(|| format!("解析 DB basic data 失败: {file_name}"))?,
            );
            let all_refnos = db_basic
                .refno_table_map
                .iter()
                .map(|entry| *entry.key())
                .collect::<Vec<_>>();
            anyhow::ensure!(!all_refnos.is_empty(), "DB 文件没有 refno: {file_name}");
            let cata_filter = crate::data_interface::cata_closure::load_sync_filter(
                project,
                std::slice::from_ref(&db_type),
            );
            let all_refnos = crate::data_interface::cata_closure::apply_sync_filter(
                cata_filter.as_ref(),
                &db_type,
                dbnum,
                all_refnos,
            );
            let chunk_size = resolve_sync_chunk_size(db_option.sync_chunk_size, 10_000);
            let chunk_jobs = all_refnos
                .chunks(chunk_size)
                .enumerate()
                .map(|(index, chunk)| (index, chunk.to_vec()))
                .collect::<Vec<_>>();
            let total_chunks = chunk_jobs.len();
            let mut stager = if save_db {
                Some(DuckLakeParseStager::open(
                    &parse_config.staging_directory,
                    &run_id,
                    dbnum,
                )?)
            } else {
                None
            };
            let ses_range_map = BTreeMap::new();
            let concurrency = resolve_indextree_chunk_concurrency(save_db);
            let mut stream =
                futures::stream::iter(chunk_jobs.into_iter().map(|(chunk_index, chunk_refnos)| {
                    let db_basic = db_basic.clone();
                    let file_name = file_name.clone();
                    let ses_range_map = ses_range_map.clone();
                    async move {
                        (
                            chunk_index,
                            parse_file_with_chunk(
                                db_basic,
                                &file_name,
                                project,
                                &chunk_refnos,
                                &ses_range_map,
                                true,
                            )
                            .await,
                        )
                    }
                }))
                .buffered(concurrency);
            let mut tree_nodes = HashMap::new();
            let mut parsed_count = 0_usize;
            while let Some((chunk_index, parsed)) = stream.next().await {
                let parsed = parsed.with_context(|| {
                    format!("parse_file_with_chunk 失败: file={file_name} chunk={chunk_index}")
                })?;
                let total_attr_map = parsed.total_attr_map;
                parsed_count += total_attr_map.len();
                for entry in total_attr_map.iter() {
                    let refno = *entry.key();
                    let attributes = entry.value();
                    tree_nodes.entry(refno).or_insert(TreeNodeMeta {
                        refno,
                        owner: attributes.get_owner().refno(),
                        noun: attributes.get_type_hash(),
                        cata_hash: attributes.cal_cata_hash(),
                    });
                }
                if let Some(stager) = stager.as_ref() {
                    let batch = build_parsed_fact_batch(
                        format!("{file_name}:{chunk_index:08}"),
                        dbnum,
                        project,
                        &db_type,
                        &db_basic,
                        &total_attr_map,
                        chunk_index == 0,
                    )?;
                    stager.write_batch(&batch)?;
                }
                if let Some(callback) = progress_callback.as_deref_mut() {
                    callback(
                        project,
                        project_index + 1,
                        db_option.included_projects.len(),
                        file_index + 1,
                        total_files,
                        chunk_index + 1,
                        total_chunks,
                    );
                }
            }
            anyhow::ensure!(
                parsed_count == all_refnos.len(),
                "解析覆盖不完整: file={file_name} parsed={parsed_count} expected={}",
                all_refnos.len()
            );
            artifacts.push(write_ducklake_parse_artifacts(
                project,
                &path,
                &file_name,
                dbnum,
                &db_type,
                sesno,
                sesno_timestamp,
                &db_basic,
                &tree_nodes,
            )?);
            if let (Some(stager), Some(authority)) = (stager.take(), authority.as_ref()) {
                reports.push(
                    publish_ducklake_stage(
                        stager,
                        ParseStageVersion {
                            from_sesno: 1,
                            to_sesno: sesno,
                            source: "total".to_string(),
                            source_hash: Some(full_sync_source_evidence(&path)),
                        },
                        authority.clone(),
                        project.clone(),
                        file_name.clone(),
                        db_type.clone(),
                    )
                    .await?,
                );
            }
        }
    }
    anyhow::ensure!(
        selected_dbnums.is_empty()
            || selected_dbnums
                .iter()
                .all(|dbnum| artifacts.iter().any(|artifact| artifact.dbnum == *dbnum)),
        "未找到全部 manual_db_nums 对应的 PDMS 文件"
    );
    validate_parse_scene_tree_artifacts(&artifacts)?;
    if save_db {
        let report_path = parse_config
            .staging_directory
            .join(&run_id)
            .join("parse-report.json");
        std::fs::write(&report_path, serde_json::to_vec_pretty(&reports)?)?;
    }
    Ok(())
}

//分成两部分，一部分先保存UDA 和 SYS 这些数据
///多线程同步数据，包括增量同步
pub async fn sync_total_async_threaded(
    db_option: &DbOption,
    project: &str,
    cur_dbno_set: Arc<DashSet<u32>>,
    db_types: &[&str],
    // progress_sender: Sender<i32>,
    proj_progress_chunk: usize,
) -> anyhow::Result<()> {
    info!("开始解析 {project} 的 {:?}", db_types);
    let db_option_arc = Arc::new(db_option.clone()); // 创建一个Arc对象，表示数据库选项

    let project_dir = db_option
        .get_project_path(project)
        .ok_or_else(|| anyhow::anyhow!("项目路径不存在: {}", project))?; // 创建一个Path对象，表示项目目录的路径
    dbg!(&project_dir);

    if !Path::new(&project_dir).exists() {
        dbg!("项目文件夹指定不正确");
        // 如果项目目录不存在，则抛出错误
        return Err(anyhow::anyhow!("项目文件夹指定不正确"));
    }
    let mut children_files = {
        // 获取子文件列表
        let target_dir = std::fs::read_dir(&project_dir)
            .unwrap()
            .into_iter()
            .map(|entry| {
                let entry = entry.unwrap();
                entry.path()
            })
            .find(|x| x.is_dir() && x.file_name().unwrap().to_str().unwrap().ends_with("000"))
            .unwrap();
        std::fs::read_dir(target_dir)?
            .into_iter()
            .map(|entry| {
                let entry = entry.unwrap();
                entry.path()
            })
            .collect::<Vec<PathBuf>>()
    };
    // 处理文件名_0001和文件名同时存在的情况
    let mut file_map = HashMap::new();
    for path in children_files.iter() {
        let file_name = path.file_stem().unwrap().to_str().unwrap();
        if let Some(base_name) = file_name.strip_suffix("_0001") {
            file_map.insert(base_name.to_string(), path.clone());
        } else {
            // 只有当没有_0001版本时才插入普通版本
            if !file_map.contains_key(file_name) {
                file_map.insert(file_name.to_string(), path.clone());
            }
        }
    }

    // 更新children_files只包含需要处理的文件
    children_files = file_map.into_values().collect();

    let project = Arc::new(project.to_string()); // 创建一个Arc对象，表示项目名称
    let mut is_replace = db_option_arc.replace_dbs; // 是否替换数据库的数据
    let replace_types = db_option_arc.replace_types.clone(); // 获取替换的类型列表
    let b_replace_types = replace_types.is_some(); // 是否存在替换的类型列表
    // 是否保存到tidb
    let b_save_mysql = db_option_arc.sync_tidb.unwrap_or(false);
    if b_replace_types {
        is_replace = true;
    }
    let chunk_size = resolve_sync_chunk_size(db_option_arc.sync_chunk_size, 1_0000);

    const CHUNK_SIZE: usize = 100;
    // let (sender, receiver) = flume::bounded(CHUNK_SIZE);
    let (sender, receiver) = flume::unbounded();
    let mut insert_handles = FuturesUnordered::new();
    for i in 0..16 {
        let receiver: flume::Receiver<SenderJsonsData> = receiver.clone();
        #[cfg(feature = "sql")]
        let pool = AiosDBManager::get_project_pool().await.unwrap().clone();

        let insert_handle = tokio::task::spawn(async move {
            // 使用 ready_chunks 而不是 chunks，这样可以在 channel 关闭时立即处理剩余数据
            use futures::stream::StreamExt;
            let mut record_stream = receiver.into_stream().ready_chunks(200);
            // let mut cnt = 0;
            while let Some(stream) = record_stream.next().await {
                // while let Ok(data) = receiver.recv_async().await {
                for data in stream {
                    match data {
                        #[cfg(feature = "surreal-save")]
                        SenderJsonsData::PEJson(pes) => {
                            if !pes.is_empty() {
                                let rows = pes.len();
                                let sql = format!("INSERT IGNORE INTO pe [{}]", pes.join(","));
                                timed_primary_query("pe", &sql, rows).await;
                                // 如果启用了 mem-kv-save，同时保存到备份数据库
                                #[cfg(feature = "mem-kv-save")]
                                {
                                    match SUL_MEM_DB.query(&sql).await {
                                        Ok(_) => {}
                                        Err(e) => {
                                            log::warn!("保存PE到内存KV数据库失败: {}", e);
                                        }
                                    }
                                }
                            }
                        }
                        #[cfg(not(feature = "surreal-save"))]
                        SenderJsonsData::PEJson(pes) => {
                            let _ = pes;
                        }
                        #[cfg(feature = "surreal-save")]
                        SenderJsonsData::PERelateJson(relates) => {
                            if !relates.is_empty() {
                                let rows = relates.len();
                                // specs/023 T007：批内元素已是完整语句
                                // （每 owner 先 DELETE 后 INSERT RELATION，幂等），直接拼接执行。
                                let sql = relates.join("\n");
                                timed_primary_query("pe_owner", &sql, rows).await;

                                // 如果启用了 mem-kv-save，同时保存到备份数据库
                                #[cfg(feature = "mem-kv-save")]
                                {
                                    match SUL_MEM_DB.query(&sql).await {
                                        Ok(_) => {}
                                        Err(e) => {
                                            log::warn!("保存PE关系到内存KV数据库失败: {}", e);
                                        }
                                    }
                                }
                            }
                        }
                        #[cfg(not(feature = "surreal-save"))]
                        SenderJsonsData::PERelateJson(relates) => {
                            let _ = relates;
                        }
                        #[cfg(feature = "surreal-save")]
                        SenderJsonsData::EleReuseRelateJson(relates) => {
                            if !relates.is_empty() {
                                ensure_ele_reuse_relate_relation_schema().await;
                                let rows = relates.len();
                                let sql = format!(
                                    "INSERT RELATION INTO ele_reuse_relate [{}]",
                                    relates.join(",")
                                );
                                timed_primary_query("ele_reuse_relate", &sql, rows).await;

                                // 如果启用了 mem-kv-save，同时保存到备份数据库
                                #[cfg(feature = "mem-kv-save")]
                                {
                                    match SUL_MEM_DB.query(&sql).await {
                                        Ok(_) => {}
                                        Err(e) => {
                                            log::warn!(
                                                "保存ele_reuse_relate到内存KV数据库失败: {}",
                                                e
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        #[cfg(not(feature = "surreal-save"))]
                        SenderJsonsData::EleReuseRelateJson(relates) => {
                            let _ = relates;
                        }
                        #[cfg(feature = "surreal-save")]
                        SenderJsonsData::AttJson((table, atts)) => {
                            if !atts.is_empty() {
                                let rows = atts.len();
                                let sql =
                                    format!("INSERT IGNORE INTO {} [{}]", table, atts.join(","));
                                timed_primary_query("att", &sql, rows).await;

                                // 如果启用了 mem-kv-save，同时保存到备份数据库
                                #[cfg(feature = "mem-kv-save")]
                                {
                                    // match SUL_MEM_DB.query(&sql).await {
                                    //     Ok(_) => {},
                                    //     Err(e) => {
                                    //         log::warn!("保存属性到内存KV数据库失败: {}", e);
                                    //     }
                                    // }
                                }
                            }
                        }
                        #[cfg(not(feature = "surreal-save"))]
                        SenderJsonsData::AttJson((table, atts)) => {
                            let _ = (table, atts);
                        }
                        #[cfg(feature = "surreal-save")]
                        SenderJsonsData::DbnumInfoUpdate(updates) => {
                            if !updates.is_empty() {
                                // 使用UPSERT语法来更新或插入dbnum_info_table记录
                                for update in updates {
                                    timed_primary_query("dbnum_info", update.as_str(), 1).await;

                                    // 同步到内存KV备份库
                                    #[cfg(feature = "mem-kv-save")]
                                    {
                                        if let Err(e) = SUL_MEM_DB.query(update.as_str()).await {
                                            log::warn!(
                                                "保存DbnumInfo到内存KV数据库失败: {} | SQL: {}",
                                                e,
                                                update
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        #[cfg(not(feature = "surreal-save"))]
                        SenderJsonsData::DbnumInfoUpdate(updates) => {
                            let _ = updates;
                        }
                        #[cfg(feature = "surreal-save")]
                        SenderJsonsData::PartitionedPEJson { table_name, sql } => {
                            // 保存简化PE数据到分表
                            log::debug!("插入到分表 {}", table_name);
                            timed_primary_query("pe_partition", &sql, 1).await;

                            // 如果启用了 mem-kv-save，同时保存到备份数据库
                            #[cfg(feature = "mem-kv-save")]
                            {
                                match SUL_MEM_DB.query(&sql).await {
                                    Ok(_) => {}
                                    Err(e) => {
                                        log::warn!(
                                            "保存分表PE到内存KV数据库失败: {} | 表: {}",
                                            e,
                                            table_name
                                        );
                                    }
                                }
                            }
                        }
                        #[cfg(not(feature = "surreal-save"))]
                        SenderJsonsData::PartitionedPEJson { table_name, sql } => {
                            let _ = (table_name, sql);
                        }
                        #[cfg(feature = "sql")]
                        SenderJsonsData::MysqlSql((project, sql)) => {
                            // let Some(pool) = pools_clone.get(&project) else {
                            //     continue;
                            // };
                            let mut conn = pool.acquire().await.expect("get pool failed");
                            match conn.execute(sql.as_str()).await {
                                Ok(_) => {}
                                Err(e) => {
                                    dbg!(e.to_string());
                                    dbg!(&sql);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            // if cnt > 0 {
            //     info!("thread {i} Imported records: {}", cnt);
            // }
        });
        insert_handles.push(insert_handle);
    }
    let db_types_clone = db_types
        .into_iter()
        .map(|&x| x.to_string())
        .collect::<Vec<_>>();
    let is_parse_sys = db_types_clone.contains(&"SYST".to_string());
    let gen_db_meta_only = db_option.gen_db_meta_only;
    #[cfg(feature = "surreal-save")]
    let is_save_db = db_option.is_save_db()
        // 同上：只在 total_sync 场景下让 gen_db_meta_only 生效。
        && !(gen_db_meta_only && db_option.total_sync);
    #[cfg(not(feature = "surreal-save"))]
    let is_save_db = false;
    let is_sync_history = db_option.is_sync_history();
    let is_total_sync = db_option.total_sync;
    let sync_versioned = db_option.sync_versioned.unwrap_or(false);
    let selected_file_names = selected_db_file_names(db_option);
    let selected_dbnums = selected_dbnums(db_option);
    let force_include = is_parse_sys && is_total_sync && selected_file_names.is_empty();
    // CATA 闭包部分解析过滤器（spec 002 T006b）：默认 Off=None=整库解析；
    // AIOS_CATA_CLOSURE_MODE=manifest 时从 cata_closure.json 加载，缺失即整库回退。
    let cata_filter =
        crate::data_interface::cata_closure::load_sync_filter(project.as_str(), &db_types_clone);

    let sender_clone = sender.clone();
    let children_files_len = children_files.len();
    let db_file_progress_chunk = (proj_progress_chunk as f32 / children_files_len as f32) as usize;
    // let progress_sender_clone = progress_sender.clone();
    // 解析任务返回本轮成功解析的 (dbnum, latest_sesno)，供收尾固化 full 锚点（specs/022 T010）。
    let pending_full_version_anchors = tokio::spawn(async move {
        let mut parsed_artifacts = Vec::new();
        // specs/022 T010：本轮解析成功的 (dbnum, latest_sesno)，写库任务 join 后固化为 full 锚点。
        let mut pending_full_version_anchors: Vec<(u32, u32, String)> = Vec::new();
        // spec 004：解析阶段总耗时计时起点。
        let sync_stage_started = Instant::now();
        //todo 按照文件大小排序，只有小于多少的能开启多线程，模型一大就不合适了
        // let mut db_info_sql = vec![];
        for path in children_files {
            let file_name = path.file_name().unwrap().to_str().unwrap().to_string(); // 获取文件名
            if file_name.contains(".") {
                continue;
            }
            let dbno_set = cur_dbno_set.clone();
            let mut time = Instant::now();
            let scan_stage_start = Instant::now();

            if !is_total_sync {
                // progress_sender_clone.send(db_file_progress_chunk).await.unwrap();
            }
            // dbg!(&file_name);
            let mut file = File::open(&path).await.unwrap();
            let mut buf = vec![0u8; 60];
            file.read_exact(&mut buf).await.unwrap();
            let db_basic_info = parse_file_basic_info(&buf);
            let db_type = db_basic_info.db_type;

            let dbnum = db_basic_info.dbnum;
            if !should_process_sync_file(
                &file_name,
                dbnum,
                &selected_file_names,
                &selected_dbnums,
                force_include,
            ) {
                continue;
            }
            //如果不是全部解析，需要检查类型，全部解析一定要解析syst等配置文件数据库
            if !db_types_clone.contains(&db_type) {
                continue;
            }
            println!("db_type is {db_type}");
            //保证不重复加载相同dbno的数据
            if dbno_set.contains(&dbnum) {
                continue;
            }
            // dbg!(dbnum);
            dbno_set.insert(dbnum);
            println!(
                "[parse-progress] file_start project={} file={} dbnum={} db_type={} save_db={}",
                project, file_name, dbnum, db_type, is_save_db
            );
            crate::perf_metrics::record_parse_progress(crate::perf_metrics::ParseProgressUpdate {
                stage: "file_start",
                project_name: project.as_str(),
                file_name: &file_name,
                dbnum,
                db_type: &db_type,
                save_db: is_save_db,
                refnos_total: 0,
                chunks_total: 0,
                chunks_completed: 0,
                last_chunk: None,
                parsed_attrs: 0,
                elapsed_ms: 0,
            });
            // 如果需要解析的文件列表为空或包含当前文件名,则执行以下代码块
            info!("path={:?}", &file_name); // 打印文件路径
            let mut ses_range_map: BTreeMap<i32, Range<u32>> = BTreeMap::new();
            let mut sesno = 0;
            let mut sesno_timestamp: Option<i64> = None;
            // let mut dt = Local::now().naive_local();
            {
                let mut io = PdmsIO::new(project.as_str(), path.clone(), true);

                    //打开文件
                    if io.open().is_ok() {
                        //获取最新sesno
                        sesno = match io.get_latest_sesno() {
                            Ok(v) => v,
                            Err(e) => {
                                // 某些 DB 文件可能存在异常 session page，导致读取 sesno 失败；此时仍可继续解析元素数据。
                                warn!(
                                    "get_latest_sesno failed(file={}): {} (fallback sesno=0)",
                                    file_name, e
                                );
                                0
                            }
                        };
                        if sesno > 0 {
                            // 获取 sesno 对应的时间戳
                            sesno_timestamp = io.get_sesno_timestamp(sesno).ok();
                            // let sql = format!(
                            //     "
                            //     DELETE db_file_info:{0};
                            //     INSERT INTO db_file_info (id, db_type, sesno, dbnum, dt) VALUES ('{0}', '{1}', {2}, {3}, '{4}');",
                            //     &file_name, db_type, sesno, dbnum, dt.and_utc().to_rfc3339()
                            // );
                            // project_primary_db().query(&sql).await.expect("save db_info failed");
                            // if sync_versioned {
                            //     continue;
                            // }
                        } else if is_sync_history {
                            // 同步历史需要有效 sesno；否则无法进行该流程。
                            warn!(
                                "skip sync_history(file={}): latest sesno is 0 (session read failed?)",
                                file_name
                            );
                            continue;
                        }

                        if is_sync_history {
                            //同步历史纪录
                            if let Err(e) = pdms_sync_history_compat(&mut io).await {
                                warn!("sync_history failed(file={}): {}", file_name, e);
                            }
                            //同步完历史纪录就返回
                            continue;
                        } else if sesno > 0 {
                            //存储所有refno sesno map（仅在 sesno 可用时执行；失败不阻塞解析）
                            if is_save_db {
                                if let Err(e) = pdms_store_refno_sesno_map_compat(&mut io).await {
                                    warn!(
                                        "store_all_refno_sesno_map failed(file={}): {} (continue parsing)",
                                        file_name, e
                                    );
                                }
                            }
                            //获取sesno range
                            // pdms-io-fork 的 ses_range_map 使用 RangeInclusive；parse_pdms_db 仍期望 Range（右开区间）
                            ses_range_map = io
                                .ses_range_map
                                .into_iter()
                                .map(|(k, r)| {
                                    let start = *r.start();
                                    let end_exclusive = r.end().saturating_add(1);
                                    (k, start..end_exclusive)
                                })
                                .collect();
                        }
                    }
                }
                let file_scan_ms = scan_stage_start.elapsed().as_millis();

                let project_name = project.as_str().to_string(); // 获取项目名称的字符串
                let db_basic_stage_start = Instant::now();
                let mut db_basic = match parse_file_db_basic_data(
                    &path,
                    &file_name,
                    project_name.clone().as_str(),
                ) {
                    Ok(v) => v,
                    Err(e) => {
                        // 之前这里用 unwrap_or_default 会导致“静默跳过解析”，很难排查为什么没产出数据。
                        warn!(
                            "parse_file_db_basic_data failed(file={}): {}",
                            file_name, e
                        );
                        continue;
                    }
                };
                let all_refnos: Vec<_> = db_basic
                    .refno_table_map
                    .iter()
                    .map(|entry| *entry.key())
                    .collect();
                if all_refnos.is_empty() {
                    // 这里为空会导致后续 parse_file_with_chunk 全部跳过，从而不会触发 save_pes / Meili 索引。
                    println!(
                        "[warn] empty refno_table_map(file={}): parse_file_db_basic_data returned no refnos",
                        file_name
                    );
                    continue;
                }
                // CATA 闭包部分解析（spec 002 T006b）：manifest 命中则裁剪 refno 全集，否则整库回退。
                let all_refnos = crate::data_interface::cata_closure::apply_sync_filter(
                    cata_filter.as_ref(),
                    &db_type,
                    dbnum,
                    all_refnos,
                );
                let db_basic_parse_ms = db_basic_stage_start.elapsed().as_millis();
                let total_chunks = std::cmp::max(1, (all_refnos.len() + chunk_size - 1) / chunk_size);
                println!(
                    "[parse-progress] db_basic_done project={} file={} dbnum={} refnos={} chunks={} db_basic_ms={}",
                    project,
                    file_name,
                    dbnum,
                    all_refnos.len(),
                    total_chunks,
                    db_basic_parse_ms
                );
                crate::perf_metrics::record_parse_progress(
                    crate::perf_metrics::ParseProgressUpdate {
                        stage: "db_basic_done",
                        project_name: project.as_str(),
                        file_name: &file_name,
                        dbnum,
                        db_type: &db_type,
                        save_db: is_save_db,
                        refnos_total: all_refnos.len(),
                        chunks_total: total_chunks,
                        chunks_completed: 0,
                        last_chunk: None,
                        parsed_attrs: 0,
                        elapsed_ms: time.elapsed().as_millis() as u64,
                    },
                );

                let db_basic = Arc::new(db_basic);
                if is_save_db {
                    save_pe_relates(&db_basic, sender_clone.clone()).await;
                }
                // 解析时不应该受 debug_model_refnos 影响，只用于模型生成调试
                let debug_refnos: Vec<RefU64> = Vec::new(); // 暂时禁用解析调试模式
                //debug 不保存数据，只复杂查看属性值
                let is_debug = !debug_refnos.is_empty();
                if is_debug {
                    let debug_refno = debug_refnos[0];
                    if let Some(children) = db_basic.children_map.get(&debug_refno) {
                        dbg!(children);
                    }
                }
                let debug_refnos = Arc::new(debug_refnos);


                let mut tree_nodes: HashMap<RefU64, TreeNodeMeta> = HashMap::new();
                let mut total_cnt = 0;
                let chunk_stage_start = Instant::now();
                let chunk_concurrency = resolve_indextree_chunk_concurrency(is_save_db);
                info!(
                    "[indextree] 开始 chunk 解析: file={}, chunk_size={}, chunk_concurrency={}, refnos={}",
                    file_name,
                    chunk_size,
                    chunk_concurrency,
                    all_refnos.len()
                );

                let chunk_jobs: Vec<(usize, Vec<RefU64>)> = all_refnos
                    .chunks(chunk_size)
                    .enumerate()
                    .map(|(chunk_index, chunk)| (chunk_index, chunk.to_vec()))
                    .collect();
                let total_chunks = std::cmp::max(1, chunk_jobs.len());

                let mut chunk_stream = futures::stream::iter(
                    chunk_jobs.into_iter().map(|(chunk_index, chunk_refnos)| {
                        let file_name_clone = file_name.clone();
                        let project_name_clone = project_name.clone();
                        let db_basic_clone = db_basic.clone();
                        let ses_range_map_clone = ses_range_map.clone();
                        async move {
                            let parse_t0 = Instant::now();
                            let result = parse_file_with_chunk(
                                db_basic_clone,
                                &file_name_clone,
                                project_name_clone.as_str(),
                                &chunk_refnos,
                                &ses_range_map_clone,
                                true,
                            )
                            .await;
                            let parse_ms = parse_t0.elapsed().as_millis() as u64;
                            (chunk_index, parse_ms, result)
                        }
                    }),
                )
                .buffer_unordered(chunk_concurrency);

                let mut completed_chunks = 0usize;
                let mut last_completed_chunk = None;
                let mut last_progress_print = Instant::now();
                while completed_chunks < total_chunks {
                    let parse_heartbeat = start_parse_progress_heartbeat(
                        project.as_str().to_string(),
                        file_name.clone(),
                        dbnum,
                        db_type.clone(),
                        is_save_db,
                        all_refnos.len(),
                        total_chunks,
                        completed_chunks,
                        last_completed_chunk,
                        total_cnt,
                        chunk_stage_start,
                    );
                    let next_chunk = chunk_stream.next().await;
                    let Some((chunk_index, parse_ms, parse_result)) = next_chunk else {
                        parse_heartbeat.stop();
                        break;
                    };

                    let mut save_pes_ms = 0u64;
                    let mut att_send_ms = 0u64;
                    let mut chunk_attrs = 0usize;

                    match parse_result {
                        Ok(PdmsDbData {
                            total_attr_map,
                            type_ele_map,
                            dbnum: dbnum,
                            ..
                        }) => {
                            //类型暂时不多线程
                            let total_attr_map_arc = Arc::new(total_attr_map);


                            total_cnt += total_attr_map_arc.len();
                            chunk_attrs = total_attr_map_arc.len();
                            for entry in total_attr_map_arc.iter() {
                                let refno = *entry.key();
                                let att = entry.value();
                                let noun = att.get_type_hash();
                                let owner = att.get_owner().refno();
                                let cata_hash = att.cal_cata_hash();
                                tree_nodes.entry(refno).or_insert(TreeNodeMeta {
                                    refno,
                                    owner,
                                    noun,
                                    cata_hash,
                                });
                            }
                            let should_save = !is_debug && is_save_db;
                            if should_save {
                                //开始执行保存数据
                                info!("开始保存pe数量: {}", total_attr_map_arc.len());
                                let save_t0 = Instant::now();
                                save_pes(
                                    &db_basic,
                                    &total_attr_map_arc,
                                    dbnum as i32,
                                    &file_name,
                                    &db_type,
                                    db_option_arc.as_ref(),
                                    sender_clone.clone(),
                                )
                                .await
                                .expect("save pes failed");
                                save_pes_ms = save_t0.elapsed().as_millis() as u64;
                            }
                            if b_save_mysql && !gen_db_meta_only {
                                #[cfg(feature = "sql")]
                                save_pes_mysql(
                                    &db_basic,
                                    &project_name,
                                    &total_attr_map_arc,
                                    &pool,
                                    db_option_arc.as_ref(),
                                    dbnum as i32,
                                    &sender_clone,
                                )
                                .await;
                            }
                            if is_save_db {
                                let att_t0 = Instant::now();
                                for kv in type_ele_map.iter() {
                                    let noun: i32 = *kv.key() as _;
                                    let type_name = db1_dehash(noun as _);
                                    if type_name.is_empty() {
                                        continue;
                                    }
                                    //UDA 还是要单独存，不然数据很容易混乱
                                    for refnos in
                                        &kv.value().iter().chunks(db_option_arc.att_chunk as _)
                                    {
                                        let mut json_vec = vec![];
                                        let mut uda_json_vec = vec![];
                                        for refno in refnos {
                                            let att = total_attr_map_arc.get(refno).unwrap();
                                            //调试时，只解析这个单独的refno
                                            if is_debug {
                                                if debug_refnos
                                                    .contains(&att.get_refno_or_default().refno())
                                                {
                                                    dbg!(att.value());
                                                } else {
                                                    continue;
                                                }
                                            }
                                            let Some(json) = att.gen_sur_json() else {
                                                continue;
                                            };
                                            json_vec.push(json);
                                            let Some(json) = att.gen_sur_json_uda(&[]) else {
                                                continue;
                                            };
                                            uda_json_vec.push(normalize_sql_string(&json));
                                        }
                                        if !json_vec.is_empty() {
                                            sender_clone
                                                .send(SenderJsonsData::AttJson((
                                                    type_name.clone(),
                                                    json_vec,
                                                )))
                                                .expect("send attmap sql failed");
                                        }

                                        if !uda_json_vec.is_empty() {
                                            // dbg!(&uda_json_vec);
                                            sender_clone
                                                .send(SenderJsonsData::AttJson((
                                                    "ATT_UDA".to_string(),
                                                    uda_json_vec,
                                                )))
                                                .expect("send attmap sql failed");
                                        }
                                    }
                                }
                                att_send_ms = att_t0.elapsed().as_millis() as u64;
                            }
                        }
                        Err(e) => {
                            warn!(
                                "parse_file_with_chunk 失败(file={}, chunk={}): {}",
                                file_name, chunk_index, e
                            );
                        }
                    }
                    completed_chunks += 1;
                    last_completed_chunk = Some(chunk_index + 1);
                    println!(
                        "[parse-progress] chunk_timing project={} file={} dbnum={} chunk={}/{} attrs={} parse_ms={} save_pes_ms={} att_send_ms={} producer_ms={}",
                        project,
                        file_name,
                        dbnum,
                        completed_chunks,
                        total_chunks,
                        chunk_attrs,
                        parse_ms,
                        save_pes_ms,
                        att_send_ms,
                        parse_ms + save_pes_ms + att_send_ms
                    );
                    if completed_chunks == 1
                        || completed_chunks == total_chunks
                        || completed_chunks % 5 == 0
                        || last_progress_print.elapsed().as_secs() >= 10
                    {
                        println!(
                            "[parse-progress] chunk_done project={} file={} dbnum={} completed_chunks={}/{} last_chunk={} parsed_attrs={} elapsed_s={:.1}",
                            project,
                            file_name,
                            dbnum,
                            completed_chunks,
                            total_chunks,
                            chunk_index + 1,
                            total_cnt,
                            chunk_stage_start.elapsed().as_secs_f32()
                        );
                        crate::perf_metrics::record_parse_progress(
                            crate::perf_metrics::ParseProgressUpdate {
                                stage: "chunk_done",
                                project_name: project.as_str(),
                                file_name: &file_name,
                                dbnum,
                                db_type: &db_type,
                                save_db: is_save_db,
                                refnos_total: all_refnos.len(),
                                chunks_total: total_chunks,
                                chunks_completed: completed_chunks,
                                last_chunk: Some(chunk_index + 1),
                                parsed_attrs: total_cnt,
                                elapsed_ms: chunk_stage_start.elapsed().as_millis() as u64,
                            },
                        );
                        last_progress_print = Instant::now();
                    }
                    parse_heartbeat.stop();
                }
                let chunk_parse_ms = chunk_stage_start.elapsed().as_millis();



                // 解析期：每处理完一个 db 文件就更新 db_meta_info.json（即使 save_db=false 且 gen_db_meta_only=true 也要生成）。
                //
                // 该文件用于：refno(ref_0) -> dbnum 的快速映射，以及记录 db 文件头的关键信息以便排查。
                let output_dir = parse_tree_output_dir(&project_name);
                let db_meta_stage_start = Instant::now();
                {
                    // ref_0 为 RefU64 高 32 位（注意：ref_0 并非 dbnum）。
                    // ref0s 必须基于整库 refno 全集（refno_table_map）收集，而非 tree_nodes：
                    // CATA 闭包部分解析时 tree_nodes 只覆盖闭包子集，db_meta 的 ref0->dbnum 映射不能缩水。
                    let mut ref0s = BTreeSet::new();
                    for entry in db_basic.refno_table_map.iter() {
                        let ref0 = entry.key().get_0();
                        if ref0 != 0 && ref0 != 0x8000_0001 {
                            ref0s.insert(ref0);
                        }
                    }

                    let header_hex_60 = (|| -> Option<String> {
                        let mut f = std::fs::File::open(&path).ok()?;
                        let mut buf = [0u8; 60];
                        f.read_exact(&mut buf).ok()?;
                        Some(hex::encode(buf))
                    })();

                    // parse_file_basic_info 的返回值在本函数里已被“部分 move”（db_type/dbnum 等），
                    // 这里避免再引用它导致借用错误；用 header_hex_60 足以做 header 排查。
                    let header_debug = None;

                    db_meta_info::update_db_meta_info_json(
                        &output_dir,
                        db_meta_info::DbFileMetaUpdate {
                            dbnum,
                            db_type: &db_type,
                            file_name: &file_name,
                            file_path: &path,
                            header_hex_60,
                            header_debug,
                            latest_sesno: Some(sesno as u32),
                            sesno_timestamp,
                            ref0s,
                        },
                    )
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "[db_meta_info] 更新失败(dbnum={}, file={}): {}",
                            dbnum,
                            file_name,
                            e
                        )
                    })?;
                }

                // specs/022 T010：登记本轮解析完成的 dbnum，收尾（写库任务 join 后）统一固化 full 锚点。
                if is_save_db && sesno > 0 {
                    pending_full_version_anchors.push((
                        dbnum,
                        sesno as u32,
                        full_sync_source_evidence(&path),
                    ));
                }
                let db_meta_update_ms = db_meta_stage_start.elapsed().as_millis();

                parsed_artifacts.push(ParsedDbArtifact {
                    project_name: project_name.clone(),
                    tree_dir: output_dir,
                    dbnum,
                    db_type: db_type.clone(),
                    file_name: file_name.clone(),
                    tree_node_count: tree_nodes.len(),
                });

                // spec 004：解析阶段每库指标（mode/total 已由 apply_sync_filter 记录）。
                crate::perf_metrics::record_parse_db(
                    dbnum,
                    &db_type,
                    total_cnt as usize,
                    time.elapsed().as_millis() as u64,
                );

            info!(
                "解析任务完成 file={} dbnum={} 总耗时={:.3}s 总数量={} [scan={}ms, db_basic={}ms, chunk={}ms, db_meta={}ms]",
                file_name,
                dbnum,
                time.elapsed().as_secs_f32(),
                total_cnt,
                file_scan_ms,
                db_basic_parse_ms,
                chunk_parse_ms,
                db_meta_update_ms
            );
            //单个文件多线程
            // if !handles.is_empty() {
            //     dbg!(handles.len());
            //
            //     futures::future::join_all(take(&mut handles)).await;
            //
            // }
            //重新更新一下database info，有可能发生了更新
            // let db_info = get_default_pdms_db_info();
            // let _ = db_info.save(None);
        }

        //执行保存db_info sql
        // let db_info_sql = db_info_sql.join(";");
        // if !db_info_sql.is_empty() {
        //     project_primary_db().query(&db_info_sql).await.expect("save db_info failed");
        // }
        validate_parse_scene_tree_artifacts(&parsed_artifacts)?;
        crate::perf_metrics::finish_parse_stage(
            parse_failed_sql_count(),
            sync_stage_started.elapsed().as_millis() as u64,
        );
        anyhow::Ok(pending_full_version_anchors)
    })
    .await
    .map_err(|e| anyhow::anyhow!("解析任务 join 失败: {}", e))??;
    drop(sender);
    // insert_handles.push(parse_handle);
    while let Some(result) = insert_handles.next().await {
        result.map_err(|e| anyhow::anyhow!("全量写入 worker 失败: {e}"))?;
    }
    // specs/022 T010：写库任务已全部 join，此刻固化 full 锚点晚于本轮全部写入。
    write_full_version_anchors(&pending_full_version_anchors).await?;
    // all_handles.push(parse_handle);
    // futures::future::join_all(take(&mut all_handles)).await;
    // futures::future::join_all(&mut [parse_handle]).await;
    Ok(())
}

/// 给对应类型的参考号赋上 uda 默认值
fn set_uda_attr(
    type_ele_map: &DashMap<u32, HashSet<RefU64>>,
    total_attr_map: &DashMap<RefU64, WholeAttMap>,
    uda_map: &mut HashMap<i32, AttrMap>,
) -> anyhow::Result<()> {
    // if let Some(uda_refnos) = type_ele_map.get(&db1_hash("UDA")) {
    //     // 获取每个 uda 的 ELEL , DFLT , UDNA属性
    //     for uda_refno in uda_refnos.value() {
    //         let uda_att = total_attr_map.get(uda_refno);
    //         if uda_att.is_none() {
    //             continue;
    //         }
    //         let uda_att = uda_att.unwrap();
    //         let uda_implicit_att = &uda_att.implicit_attmap;
    //         let uda_explicit_att = &uda_att.explicit_attmap;

    //         let ukey = uda_implicit_att.get_i32("UKEY");
    //         if ukey.is_none() {
    //             continue;
    //         }
    //         let ukey = ukey.unwrap();
    //         // 若udna中没有值，则可能在显式属性的dyudna中
    //         let mut udna = uda_implicit_att.get_str("UDNA");
    //         if udna == Some("") {
    //             udna = uda_explicit_att.get_str("DYUDNA");
    //         }
    //         let elel = uda_explicit_att.get_i32_vec("ELEL");
    //         let default = uda_explicit_att.get_val("DFLT");
    //         if elel.is_none() || default.is_none() {
    //             continue;
    //         }
    //         // let udna = udna.unwrap();
    //         let elel = elel.unwrap();
    //         let default = default.unwrap();
    //         for noun in elel {
    //             uda_map
    //                 .entry(noun)
    //                 .or_insert_with(AttrMap::default)
    //                 .entry((ukey as u32))
    //                 .or_insert(default.clone());
    //         }
    //     }
    // }
    Ok(())
}

// pub fn gen_pdms_element_insert_sql(att: &WholeAttMap, name: &str, dbnum: u32, order: usize, children_count: usize) -> String {
//     let attmap = &att.att_map();
//     let refno = attmap.get_refno().unwrap();
//     let type_name = attmap.get_type();
//     let owner = attmap.get_owner();
//
//     let mut sql = String::new();
//     sql.push_str(&format!(r#"({}, '{}', '{}', {},'{}' , {} , {} , {} ,0 ) ,"#,
//                           refno.0, refno.to_pdms_str(), type_name, owner.0, name, dbnum, order, children_count));
//     sql
// }

#[tokio::test]
async fn test_threads() {
    let mut map = Arc::new(DashSet::new());
    let mut handles = vec![];
    for i in 0..10 {
        let map_clone = map.clone();
        let handle = tokio::spawn(async move {
            map_clone.insert(i);
        });
        handles.push(handle);
    }
    futures::future::join_all(take(&mut handles)).await;
    dbg!(&map.len());
    for v in Arc::try_unwrap(map).unwrap() {
        dbg!(v);
    }
}

#[cfg(test)]
mod scene_tree_artifact_tests {
    use super::{ParsedDbArtifact, validate_parse_scene_tree_artifacts};
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn unique_temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "aios-scene-tree-test-{}-{}",
            name,
            std::process::id()
        ))
    }

    fn with_output_root<T>(name: &str, test: impl FnOnce(PathBuf) -> T) -> T {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = unique_temp_dir(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("DbOption.toml");
        fs::write(
            &config_path,
            format!(
                "project_name = \"demo\"\noutput_root = \"{}\"\n",
                dir.join("output").to_string_lossy().replace('\\', "/")
            ),
        )
        .unwrap();
        let old = std::env::var("DB_OPTION_FILE").ok();
        unsafe {
            std::env::set_var(
                "DB_OPTION_FILE",
                config_path.with_extension("").to_string_lossy().to_string(),
            );
        }

        let result = test(dir.clone());

        unsafe {
            if let Some(old) = old {
                std::env::set_var("DB_OPTION_FILE", old);
            } else {
                std::env::remove_var("DB_OPTION_FILE");
            }
        }
        let _ = fs::remove_dir_all(&dir);
        result
    }

    fn artifact(nodes: usize) -> ParsedDbArtifact {
        ParsedDbArtifact {
            project_name: "demo".to_string(),
            tree_dir: crate::versioned_db::db_meta_info::get_project_tree_dir("demo"),
            dbnum: 42,
            db_type: "DESI".to_string(),
            file_name: "DESI0001".to_string(),
            tree_node_count: nodes,
        }
    }

    #[test]
    fn validate_parse_scene_tree_artifacts_requires_meta() {
        with_output_root("missing-meta", |_| {
            let err = validate_parse_scene_tree_artifacts(&[artifact(1)]).unwrap_err();
            assert!(err.to_string().contains("缺少 db_meta_info.json"));
        });
    }

    #[test]
    fn validate_parse_scene_tree_artifacts_allows_empty_hierarchy_nodes_with_meta() {
        with_output_root("empty-tree", |_| {
            let tree_dir = crate::versioned_db::db_meta_info::get_project_tree_dir("demo");
            fs::create_dir_all(&tree_dir).unwrap();
            fs::write(
                tree_dir.join("db_meta_info.json"),
                serde_json::to_string(&json!({
                    "version": 1,
                    "ref0_to_dbnum": {},
                    "db_files": { "42": { "dbnum": 42, "db_type": "DESI", "file_name": "DESI0001", "ref0s": [] } }
                }))
                .unwrap(),
            )
            .unwrap();

            validate_parse_scene_tree_artifacts(&[artifact(0)]).unwrap();
        });
    }
}

/// 解析单个 db 文件并生成 indextree
pub async fn parse_single_db_file(
    db_option: &DbOption,
    project_name: &str,
    file_path: &str,
    target_dbnum: u32,
) -> anyhow::Result<()> {
    let time = Instant::now();
    let chunk_size = resolve_single_indextree_chunk_size(db_option);
    let chunk_concurrency = resolve_indextree_chunk_concurrency(false);
    let path = PathBuf::from(file_path);
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();

    println!("🔄 开始解析文件: {} (dbnum={})", file_name, target_dbnum);

    // 读取文件头获取 db_type
    let db_type = {
        let mut file = std::fs::File::open(&path)?;
        let mut buf = [0u8; 60];
        file.read_exact(&mut buf)?;
        parse_file_basic_info(&buf).db_type
    };

    // 解析基本数据
    let db_basic_stage_start = Instant::now();
    let db_basic = match parse_file_db_basic_data(&path, &file_name, project_name) {
        Ok(data) => data,
        Err(e) => {
            anyhow::bail!("parse_file_db_basic_data 失败: {}", e);
        }
    };
    let db_basic_parse_ms = db_basic_stage_start.elapsed().as_millis();

    let all_refnos: Vec<_> = db_basic
        .refno_table_map
        .iter()
        .map(|entry| *entry.key())
        .collect();
    if all_refnos.is_empty() {
        anyhow::bail!("文件 {} 中没有找到任何 refno", file_name);
    }

    // CATA 闭包部分解析（spec 002 T006b）：与 sync 流水线同一开关/回退语义。
    let cata_filter = crate::data_interface::cata_closure::load_sync_filter(
        project_name,
        std::slice::from_ref(&db_type),
    );
    let all_refnos = crate::data_interface::cata_closure::apply_sync_filter(
        cata_filter.as_ref(),
        &db_type,
        target_dbnum,
        all_refnos,
    );

    println!("📊 找到 {} 个 refno，开始解析...", all_refnos.len());

    let db_basic = Arc::new(db_basic);
    let mut tree_nodes: HashMap<RefU64, TreeNodeMeta> = HashMap::new();
    let ses_range_map: BTreeMap<i32, Range<u32>> = BTreeMap::new();

    // 分块解析
    let chunk_stage_start = Instant::now();
    info!(
        "[indextree-single] 开始 chunk 解析: file={}, chunk_size={}, chunk_concurrency={}, refnos={}",
        file_name,
        chunk_size,
        chunk_concurrency,
        all_refnos.len()
    );
    let chunk_jobs: Vec<(usize, Vec<RefU64>)> = all_refnos
        .chunks(chunk_size)
        .enumerate()
        .map(|(chunk_index, chunk)| (chunk_index, chunk.to_vec()))
        .collect();

    let mut chunk_stream =
        futures::stream::iter(chunk_jobs.into_iter().map(|(chunk_index, chunk_refnos)| {
            let db_basic_clone = db_basic.clone();
            let file_name_clone = file_name.clone();
            let ses_range_map_clone = ses_range_map.clone();
            async move {
                let result = parse_file_with_chunk(
                    db_basic_clone,
                    &file_name_clone,
                    project_name,
                    &chunk_refnos,
                    &ses_range_map_clone,
                    true,
                )
                .await;
                (chunk_index, result)
            }
        }))
        .buffer_unordered(chunk_concurrency);

    while let Some((chunk_index, parse_result)) = chunk_stream.next().await {
        match parse_result {
            Ok(PdmsDbData {
                total_attr_map,
                dbnum,
                ..
            }) => {
                for entry in total_attr_map.iter() {
                    let refno = *entry.key();
                    let att = entry.value();
                    let noun = att.get_type_hash();
                    let owner = att.get_owner().refno();
                    let cata_hash = att.cal_cata_hash();
                    tree_nodes.entry(refno).or_insert(TreeNodeMeta {
                        refno,
                        owner,
                        noun,
                        cata_hash,
                    });
                }
            }
            Err(e) => {
                warn!(
                    "parse_file_with_chunk 失败(file={}, chunk={}): {}",
                    file_name, chunk_index, e
                );
            }
        }
    }
    let chunk_parse_ms = chunk_stage_start.elapsed().as_millis();

    let output_dir = parse_tree_output_dir(project_name);

    // 收集 ref0s 并更新 db_meta_info.json
    // ref0s 必须基于整库 refno 全集（refno_table_map）收集，而非 tree_nodes：
    // CATA 闭包部分解析时 tree_nodes 只覆盖闭包子集，db_meta 的 ref0->dbnum 映射不能缩水。
    let db_meta_stage_start = Instant::now();
    let ref0s: std::collections::BTreeSet<u32> = db_basic
        .refno_table_map
        .iter()
        .map(|entry| entry.key().get_0())
        .filter(|&ref0| ref0 != 0 && ref0 != 0x8000_0001)
        .collect();

    let file_path_buf = PathBuf::from(file_path);

    // 读取文件头 60 字节转 hex
    let header_hex_60 = (|| -> Option<String> {
        let mut f = std::fs::File::open(&path).ok()?;
        let mut buf = [0u8; 60];
        f.read_exact(&mut buf).ok()?;
        Some(hex::encode(buf))
    })();

    // 获取 latest_sesno (通过 PdmsIO)
    let latest_sesno = {
        let mut io = PdmsIO::new(project_name, path.clone(), true);
        if io.open().is_ok() {
            match io.get_latest_sesno() {
                Ok(v) => Some(v),
                Err(e) => {
                    warn!("get_latest_sesno failed: {}", e);
                    None
                }
            }
        } else {
            None
        }
    };

    db_meta_info::update_db_meta_info_json(
        &output_dir,
        db_meta_info::DbFileMetaUpdate {
            dbnum: target_dbnum,
            db_type: &db_type,
            file_name: &file_name,
            file_path: &file_path_buf,
            header_hex_60,
            header_debug: None,
            latest_sesno,
            sesno_timestamp: None,
            ref0s,
        },
    )
    .map_err(|e| {
        anyhow::anyhow!(
            "[db_meta_info] 更新失败(dbnum={}, file={}): {}",
            target_dbnum,
            file_name,
            e
        )
    })?;
    let db_meta_update_ms = db_meta_stage_start.elapsed().as_millis();

    validate_parse_scene_tree_artifacts(&[ParsedDbArtifact {
        project_name: project_name.to_string(),
        tree_dir: output_dir,
        dbnum: target_dbnum,
        db_type: db_type.clone(),
        file_name: file_name.clone(),
        tree_node_count: tree_nodes.len(),
    }])?;

    println!(
        "✅ 解析完成，耗时: {:.2}s，生成 {} 个节点 [db_basic={}ms, chunk={}ms, db_meta={}ms]",
        time.elapsed().as_secs_f32(),
        tree_nodes.len(),
        db_basic_parse_ms,
        chunk_parse_ms,
        db_meta_update_ms
    );

    // spec 004：单库解析模式的指标记录（与多库 sync 同口径，元素数取 tree_nodes）。
    crate::perf_metrics::record_parse_db(
        target_dbnum,
        &db_type,
        tree_nodes.len(),
        time.elapsed().as_millis() as u64,
    );
    crate::perf_metrics::finish_parse_stage(
        parse_failed_sql_count(),
        time.elapsed().as_millis() as u64,
    );

    // specs/022 R1：本路径只更新 db_meta，不向 Surreal 写入 PE/ATT，
    // 因此不写 sesno_version_anchor。full 锚点由 sync_pdms* 在写库 join 完成后固化。
    Ok(())
}
