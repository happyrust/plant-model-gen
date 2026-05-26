//! 管理员站点部署与运行时管理。
//!
//! 主要改动（2026-04-21 P0/P1/P2 批量整改）：
//! - 凭据不再经命令行传给 surreal 子进程，改用环境变量；站点配置文件写入时降权 0600。
//! - `project_path` 在 create/update 时做白名单校验 + canonicalize，拒绝 symlink 逃逸。
//! - create/update/start/stop 走进程内互斥 + SQLite `BEGIN IMMEDIATE`，避免端口 TOCTOU 与并发覆盖。
//! - 子进程以独立 process group 启动；`stop_site` 以 `killpg` 方式清理整组（Unix），Windows 走 taskkill /T。
//! - `refresh_site` 改为纯派生函数，不再改写 `entry_url`；真正状态变更都走 `update_runtime`。
//! - 新增 `path_size_bytes` 的 TTL 缓存；递归扫描限制深度并跳过隐藏/符号链接。
//! - `open_db` 使用进程内共享连接 + 一次性 schema 升级；pid 存在性检查改用 `libc::kill(pid,0)`。

use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use parse_pdms_db::parse::parse_file_basic_info;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;
use sysinfo::{
    CpuRefreshKind, Disks, MemoryRefreshKind, Pid, ProcessRefreshKind, ProcessesToUpdate, System,
};
use tokio::process::Command;
use tokio::task;

use super::models::{
    AdminResourceSummary, CreateManagedSiteRequest, DatabaseConfig, ManagedProjectSite,
    ManagedSiteActivitySummary, ManagedSiteDeployValidationCheck,
    ManagedSiteDeployValidationReport, ManagedSiteLogStreamSummary, ManagedSiteLogsResponse,
    ManagedSiteParseHealth, ManagedSiteParseHealthStatus, ManagedSiteParsePlan,
    ManagedSiteParsePlanMode, ManagedSiteParseStatus, ManagedSitePreflightCheck,
    ManagedSitePreflightReport, ManagedSitePreflightStatus, ManagedSiteProcessResource,
    ManagedSiteResourceMetrics, ManagedSiteRiskLevel, ManagedSiteRuntimeStatus, ManagedSiteStatus,
    PreviewManagedSiteParsePlanRequest, UpdateManagedSiteRequest,
};

// ─── Constants ──────────────────────────────────────────────────────────────

const DEFAULT_SQLITE_PATH: &str = "deployment_sites.sqlite";
const TABLE_NAME: &str = "managed_project_sites";
const ADMIN_RUNTIME_ROOT: &str = "runtime/admin_sites";
const LOG_LINES_LIMIT: usize = 120;

// 机器与进程告警阈值。
const MACHINE_WARNING_CPU: f32 = 85.0;
const MACHINE_CRITICAL_CPU: f32 = 95.0;
const MACHINE_WARNING_MEMORY: f32 = 80.0;
const MACHINE_CRITICAL_MEMORY: f32 = 90.0;
const MACHINE_WARNING_DISK: f32 = 85.0;
const MACHINE_CRITICAL_DISK: f32 = 95.0;
const PROCESS_WARNING_CPU: f32 = 70.0;
const PROCESS_CRITICAL_CPU: f32 = 90.0;
const PROCESS_WARNING_MEMORY_BYTES: u64 = 1536 * 1024 * 1024;
const PROCESS_CRITICAL_MEMORY_BYTES: u64 = 3 * 1024 * 1024 * 1024;
const PARSE_WARNING_DURATION_MS: u64 = 10 * 60 * 1000;
const PARSE_CRITICAL_DURATION_MS: u64 = 30 * 60 * 1000;
const DEFAULT_PARSE_DB_TYPES: &[&str] = &["SYST", "DESI"];
const SUPPORTED_PARSE_DB_TYPES: &[&str] = &["SYST", "DESI", "CATA", "DICT", "GLB", "GLOB"];
const REPARSE_REUSE_DB_TYPES: &[&str] = &["SYST"];
const AUTO_DB_PORT_START: u16 = 8020;
const AUTO_WEB_PORT_START: u16 = 8080;
const AUTO_PORT_END: u16 = 8999;

// 运行时等待/杀进程超时。
const WAIT_PORT_ATTEMPTS: usize = 30;
const WAIT_HTTP_ATTEMPTS: usize = 40;
const WAIT_STEP_MS: u64 = 500;
const KILL_GRACE_MS: u64 = 1500;

// 递归扫描保护。
const SCAN_MAX_DEPTH: usize = 6;
const SCAN_MAX_FILES: usize = 200_000;

// 磁盘占用缓存 TTL。
const PATH_SIZE_CACHE_TTL_MS: u64 = 60_000;

// Schema 版本号：每次迁移 +1。
const SCHEMA_VERSION: u32 = 6;

// ─── Global state (opt-in, interior mutability) ─────────────────────────────

/// 全站点级互斥：用于 create/update/start/stop 等写流程之间的互斥。
/// 生产环境管理后台并发量低，用单个 Mutex 简化正确性，避免遗漏。
fn site_op_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(Debug)]
struct ResourceSampler {
    system: System,
    warmed_up: bool,
}

fn resource_sampler() -> &'static Mutex<ResourceSampler> {
    static SAMPLER: OnceLock<Mutex<ResourceSampler>> = OnceLock::new();
    SAMPLER.get_or_init(|| {
        Mutex::new(ResourceSampler {
            system: System::new(),
            warmed_up: false,
        })
    })
}

#[derive(Debug)]
struct PathSizeCacheEntry {
    value: u64,
    recorded_at: Instant,
}

fn path_size_cache() -> &'static Mutex<HashMap<PathBuf, PathSizeCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, PathSizeCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 共享的 SQLite 连接，避免每次 `open_db` 重新打开。
fn shared_conn() -> &'static Mutex<Connection> {
    static CONN: OnceLock<Mutex<Connection>> = OnceLock::new();
    CONN.get_or_init(|| {
        let path = sqlite_path();
        let conn = Connection::open(&path).unwrap_or_else(|err| {
            panic!("打开管理员站点数据库失败 ({path}): {err}");
        });
        if let Err(err) = conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON;",
        ) {
            tracing::warn!("初始化 SQLite pragma 失败: {err}");
        }
        if let Err(err) = ensure_schema_with_conn(&conn) {
            tracing::warn!("初始化站点 schema 失败: {err}");
        }
        Mutex::new(conn)
    })
}

fn with_conn<R>(handler: impl FnOnce(&Connection) -> Result<R>) -> Result<R> {
    let guard = shared_conn()
        .lock()
        .map_err(|_| anyhow!("站点数据库连接锁已中毒"))?;
    handler(&guard)
}

fn with_tx<R>(handler: impl FnOnce(&Connection) -> Result<R>) -> Result<R> {
    let guard = shared_conn()
        .lock()
        .map_err(|_| anyhow!("站点数据库连接锁已中毒"))?;
    guard.execute_batch("BEGIN IMMEDIATE")?;
    let outcome = handler(&guard);
    match outcome {
        Ok(value) => {
            guard.execute_batch("COMMIT")?;
            Ok(value)
        }
        Err(err) => {
            let _ = guard.execute_batch("ROLLBACK");
            Err(err)
        }
    }
}

// ─── Logging snapshot ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct LogSnapshot {
    key: &'static str,
    label: &'static str,
    path: PathBuf,
    exists: bool,
    has_content: bool,
    updated_at: Option<SystemTime>,
    updated_at_rfc3339: Option<String>,
    lines: Vec<String>,
    line_count: usize,
    last_line: Option<String>,
    last_key_log: Option<String>,
}

// ─── Config helpers ─────────────────────────────────────────────────────────

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

fn load_config_builder() -> Option<config::Config> {
    let cfg_name =
        std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption".to_string());
    let cfg_file = format!("{}.toml", cfg_name);
    if !Path::new(&cfg_file).exists() {
        return None;
    }
    config::Config::builder()
        .add_source(config::File::with_name(&cfg_name))
        .build()
        .ok()
}

fn sqlite_path() -> String {
    load_config_builder()
        .and_then(|builder| builder.get_string("deployment_sites_sqlite_path").ok())
        .unwrap_or_else(|| DEFAULT_SQLITE_PATH.to_string())
}

/// 从配置读取允许的 project 根目录白名单。
/// 未配置时按"兼容模式"返回空 vec —— 此时 `canonical_project_path` 会记录 warn 但放行。
fn admin_allowed_project_roots() -> Vec<PathBuf> {
    let Some(builder) = load_config_builder() else {
        return Vec::new();
    };
    let raw = builder
        .get_array("admin_allowed_project_roots")
        .or_else(|_| builder.get_array("allowed_project_roots"))
        .unwrap_or_default();
    raw.into_iter()
        .filter_map(|v| v.into_string().ok())
        .map(|s| PathBuf::from(s.trim()))
        .filter(|p| !p.as_os_str().is_empty())
        .collect()
}

fn admin_aios_database_binary_override() -> Option<PathBuf> {
    if let Ok(value) = std::env::var("ADMIN_AIOS_DATABASE_BINARY") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    load_config_builder().and_then(|builder| {
        builder
            .get_string("admin_aios_database_binary")
            .ok()
            .map(|s| PathBuf::from(s.trim()))
            .filter(|p| !p.as_os_str().is_empty())
    })
}

fn admin_allow_cargo_fallback() -> bool {
    if let Ok(value) = std::env::var("ADMIN_ALLOW_CARGO_RUN") {
        return matches!(value.trim(), "1" | "true" | "yes" | "on");
    }
    load_config_builder()
        .and_then(|builder| builder.get_bool("admin_allow_cargo_fallback").ok())
        .unwrap_or(false)
}

fn admin_allow_any_project_path() -> bool {
    if let Ok(value) = std::env::var("AIOS_ADMIN_ALLOW_ANY_PROJECT_PATH") {
        return matches!(value.trim(), "1" | "true" | "yes" | "on");
    }
    load_config_builder()
        .and_then(|builder| builder.get_bool("admin_allow_any_project_path").ok())
        .unwrap_or(false)
}

/// 规范化并校验 `project_path`：
/// - 绝对化 + `canonicalize`（解符号链接）；
/// - 若配置了白名单，拒绝不在白名单下的路径；
/// - 若未配置白名单，仅在显式开启兼容开关时放行。
fn canonical_project_path(raw: &str) -> Result<PathBuf> {
    let path = PathBuf::from(raw);
    if path.as_os_str().is_empty() {
        bail!("项目路径不能为空");
    }
    let canonical = fs::canonicalize(&path)
        .with_context(|| format!("项目路径无法访问或不存在: {}", path.display()))?;
    let roots = admin_allowed_project_roots();
    if roots.is_empty() {
        if !admin_allow_any_project_path() {
            bail!(
                "未配置 admin_allowed_project_roots，拒绝 project_path={}；如需兼容旧行为，请显式设置 AIOS_ADMIN_ALLOW_ANY_PROJECT_PATH=1",
                canonical.display()
            );
        }
        tracing::warn!(
            "未配置 admin_allowed_project_roots，因显式兼容开关放行 project_path={}（生产环境请配置白名单）",
            canonical.display()
        );
        return Ok(canonical);
    }
    for root in &roots {
        let Ok(canonical_root) = fs::canonicalize(root) else {
            continue;
        };
        if canonical.starts_with(&canonical_root) {
            return Ok(canonical);
        }
    }
    bail!(
        "project_path 未在允许的根目录白名单内: {}",
        canonical.display()
    );
}

// ─── Runtime path helpers ───────────────────────────────────────────────────

fn runtime_root() -> PathBuf {
    PathBuf::from(ADMIN_RUNTIME_ROOT)
}

fn site_runtime_dir(site_id: &str) -> PathBuf {
    runtime_root().join(site_id)
}

fn site_logs_dir(site_id: &str) -> PathBuf {
    site_runtime_dir(site_id).join("logs")
}

fn parse_log_path(site_id: &str) -> PathBuf {
    site_logs_dir(site_id).join("parse.log")
}

fn db_log_path(site_id: &str) -> PathBuf {
    site_logs_dir(site_id).join("surreal.log")
}

fn web_log_path(site_id: &str) -> PathBuf {
    site_logs_dir(site_id).join("web_server.log")
}

fn viewer_log_path(site_id: &str) -> PathBuf {
    site_logs_dir(site_id).join("viewer.log")
}

fn generate_log_path(site_id: &str) -> PathBuf {
    site_logs_dir(site_id).join("generate.log")
}

fn metadata_path(site_id: &str) -> PathBuf {
    site_runtime_dir(site_id).join("metadata.json")
}

fn config_path(site_id: &str) -> PathBuf {
    site_runtime_dir(site_id).join("DbOption.toml")
}

fn parse_config_path(site_id: &str) -> PathBuf {
    site_runtime_dir(site_id).join("DbOption-parse.toml")
}

fn db_data_path(site_id: &str) -> PathBuf {
    site_runtime_dir(site_id).join("data").join("surreal.db")
}

// ─── Slug / id helpers ──────────────────────────────────────────────────────

fn slugify(input: &str) -> String {
    let value = input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let compact = value
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if compact.is_empty() {
        "site".to_string()
    } else {
        compact
    }
}

fn infer_site_id(project_name: &str, web_port: u16) -> String {
    let slug = slugify(project_name);
    debug_assert!(
        !slug.contains("..") && !slug.contains('/') && !slug.contains('\\'),
        "slugify 结果必须是 [a-z0-9-]+: {slug}"
    );
    format!("{}-{}", slug, web_port)
}

fn normalize_host(host: Option<String>) -> String {
    host.map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "127.0.0.1".to_string())
}

/// 在写入 DB 之前对 `bind_host` 做安全校验。
///
/// - `0.0.0.0` 默认拒绝（公网暴露风险）
/// - `AIOS_ALLOW_PUBLIC_BIND=1` / `=true` 时放行，便于需要内网/跨机部署的场景
///
/// 设计动机：继 `normalize_host` 在空值时默认 `127.0.0.1` 之后，为"用户显式传
/// 0.0.0.0 也要兜一下"补第二道保险（PDMS Hardening 续篇：admin 站点安全收口，
/// 详见 `docs/plans/2026-04-24-admin-site-security-hardening-plan.md`）。
fn assert_bind_host_safe(host: &str) -> Result<()> {
    let trimmed = host.trim();
    if trimmed == "0.0.0.0" && !env_allow_public_bind() {
        bail!(
            "bind_host=0.0.0.0 会将站点暴露到所有网络接口。\
             请改用 127.0.0.1 或具体的内网地址；\
             如确需公网绑定，请设置 AIOS_ALLOW_PUBLIC_BIND=1 并自行承担风险。"
        );
    }
    Ok(())
}

fn env_allow_public_bind() -> bool {
    std::env::var("AIOS_ALLOW_PUBLIC_BIND")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn require_db_user(user: Option<String>) -> Result<String> {
    user.map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("数据库用户名不能为空"))
}

fn require_db_password(password: Option<String>) -> Result<String> {
    password
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("数据库密码不能为空"))
}

/// 常见弱凭据黑名单（小写比较）；后续可按需扩展。
const WEAK_CREDENTIAL_PAIRS: &[(&str, &str)] = &[
    ("root", "root"),
    ("admin", "admin"),
    ("admin", "123456"),
    ("root", "123456"),
    ("test", "test"),
];

fn env_allow_weak_db_creds() -> bool {
    std::env::var("AIOS_ALLOW_WEAK_DB_CREDS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// 拒绝常见弱凭据；允许通过 `AIOS_ALLOW_WEAK_DB_CREDS=1` 逃生（开发/测试兼容）。
///
/// 约束理由：站点 SurrealDB 的 `user/password` 会以明文写入 per-site 配置，
/// 若误填 `root/root` 会导致站点 DB 对任意连接者可读写。SiteDrawer.vue 从
/// 2026-04-21 起已经取消默认 root/root 预填，但后端仍然只校验"非空"，
/// 手写或脚本化提交仍可能绕过；本函数在 `create_site` / `update_site` 两处
/// 统一兜一层硬拒绝。
fn assert_db_credentials_strong(user: &str, password: &str) -> Result<()> {
    if env_allow_weak_db_creds() {
        return Ok(());
    }
    let u = user.trim().to_ascii_lowercase();
    let p = password.trim().to_ascii_lowercase();
    for (weak_u, weak_p) in WEAK_CREDENTIAL_PAIRS {
        if u == *weak_u && p == *weak_p {
            bail!(
                "数据库凭据过于简单（{}/{}）。\
                 请使用更复杂的用户名/密码；如仅用于本地开发，\
                 可设置 AIOS_ALLOW_WEAK_DB_CREDS=1 临时放行。",
                user,
                password,
            );
        }
    }
    Ok(())
}

fn normalize_optional_db_user(user: Option<String>) -> Option<String> {
    user.map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_optional_db_password(password: Option<String>) -> Option<String> {
    password
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_manual_db_nums(values: Vec<u32>) -> Vec<u32> {
    let mut values = values
        .into_iter()
        .filter(|value| *value > 0)
        .collect::<Vec<_>>();
    values.sort_unstable();
    values.dedup();
    values
}

fn manual_db_nums_to_json(values: &[u32]) -> Result<String> {
    Ok(serde_json::to_string(values)?)
}

fn manual_db_nums_from_json(raw: Option<String>) -> Vec<u32> {
    raw.and_then(|value| serde_json::from_str::<Vec<u32>>(&value).ok())
        .map(normalize_manual_db_nums)
        .unwrap_or_default()
}

fn default_parse_db_types() -> Vec<String> {
    DEFAULT_PARSE_DB_TYPES
        .iter()
        .map(|value| (*value).to_string())
        .collect()
}

fn normalize_parse_db_types(values: Vec<String>) -> Vec<String> {
    let mut values = values
        .into_iter()
        .map(|value| value.trim().to_ascii_uppercase())
        .filter(|value| SUPPORTED_PARSE_DB_TYPES.contains(&value.as_str()))
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

fn parse_db_types_to_json(values: &[String]) -> Result<String> {
    Ok(serde_json::to_string(values)?)
}

fn parse_db_types_from_json(raw: Option<String>) -> Vec<String> {
    match raw {
        Some(value) => serde_json::from_str::<Vec<String>>(&value)
            .map(normalize_parse_db_types)
            .unwrap_or_default(),
        None => default_parse_db_types(),
    }
}

fn normalize_force_rebuild_system_db(
    force_rebuild_system_db: bool,
    parse_db_types: &[String],
) -> bool {
    force_rebuild_system_db
        && parse_db_types
            .iter()
            .any(|value| REPARSE_REUSE_DB_TYPES.contains(&value.as_str()))
}

fn default_generation_config() -> DatabaseConfig {
    DatabaseConfig::from_db_option(&aios_core::get_db_option())
}

fn normalize_mesh_tol_ratio(value: Option<f64>, fallback: f64) -> f64 {
    value
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(fallback)
}

// ─── Enum / string conversions ──────────────────────────────────────────────

fn status_to_str(status: &ManagedSiteStatus) -> &'static str {
    match status {
        ManagedSiteStatus::Draft => "Draft",
        ManagedSiteStatus::Parsed => "Parsed",
        ManagedSiteStatus::Starting => "Starting",
        ManagedSiteStatus::Running => "Running",
        ManagedSiteStatus::Stopping => "Stopping",
        ManagedSiteStatus::Stopped => "Stopped",
        ManagedSiteStatus::Failed => "Failed",
    }
}

fn parse_status_to_str(status: &ManagedSiteParseStatus) -> &'static str {
    match status {
        ManagedSiteParseStatus::Pending => "Pending",
        ManagedSiteParseStatus::Running => "Running",
        ManagedSiteParseStatus::Parsed => "Parsed",
        ManagedSiteParseStatus::Failed => "Failed",
    }
}

fn status_from_str(raw: &str) -> ManagedSiteStatus {
    match raw {
        "Parsed" => ManagedSiteStatus::Parsed,
        "Starting" => ManagedSiteStatus::Starting,
        "Running" => ManagedSiteStatus::Running,
        "Stopping" => ManagedSiteStatus::Stopping,
        "Stopped" => ManagedSiteStatus::Stopped,
        "Failed" => ManagedSiteStatus::Failed,
        "Draft" => ManagedSiteStatus::Draft,
        other => {
            tracing::warn!("status_from_str 收到未知状态: {other}，退回 Draft");
            ManagedSiteStatus::Draft
        }
    }
}

fn parse_status_from_str(raw: &str) -> ManagedSiteParseStatus {
    match raw {
        "Running" => ManagedSiteParseStatus::Running,
        "Parsed" => ManagedSiteParseStatus::Parsed,
        "Failed" => ManagedSiteParseStatus::Failed,
        "Pending" => ManagedSiteParseStatus::Pending,
        other => {
            tracing::warn!("parse_status_from_str 收到未知状态: {other}，退回 Pending");
            ManagedSiteParseStatus::Pending
        }
    }
}

// ─── Filesystem helpers ─────────────────────────────────────────────────────

fn ensure_runtime_dirs(site_id: &str) -> Result<()> {
    fs::create_dir_all(site_logs_dir(site_id))?;
    fs::create_dir_all(site_runtime_dir(site_id).join("data"))?;
    Ok(())
}

fn current_config_source() -> PathBuf {
    let cfg_name =
        std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption-mac".to_string());
    let path = PathBuf::from(format!("{}.toml", cfg_name));
    if path.exists() {
        path
    } else {
        PathBuf::from("db_options/DbOption-mac.toml")
    }
}

/// 将 `project_name` 从 `project_path` 中拆出，返回 (parent_dir, included_projects, project_dirs)。
/// 约定：
/// * `project_path` 末段 == `project_name` → 父目录为 `parent(project_path)`；
/// * 否则 `project_path` 本身被视作"项目根目录的同胞目录的父目录"，子目录名为 `project_name`。
fn split_project_root(project_name: &str, raw_path: &str) -> (String, Vec<String>, Vec<String>) {
    let path = PathBuf::from(raw_path);
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if file_name == project_name {
        let parent = path
            .parent()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| raw_path.to_string());
        return (
            parent,
            vec![project_name.to_string()],
            vec![project_name.to_string()],
        );
    }
    (
        raw_path.to_string(),
        vec![project_name.to_string()],
        vec![project_name.to_string()],
    )
}

fn site_source_project_name(site: &ManagedProjectSite) -> String {
    site.associated_project
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            PathBuf::from(&site.project_path)
                .file_name()
                .and_then(|value| value.to_str())
                .map(ToOwned::to_owned)
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| site.project_name.clone())
}

fn project_dir_candidates(project_name: &str, raw_path: &str) -> Vec<PathBuf> {
    let raw = PathBuf::from(raw_path);
    let file_name = raw.file_name().and_then(|value| value.to_str());
    let mut candidates = Vec::new();
    if matches!(file_name, Some(name) if name == project_name) {
        candidates.push(raw.clone());
        if let Some(parent) = raw.parent() {
            candidates.push(parent.join(project_name));
        }
    } else {
        candidates.push(raw.join(project_name));
        candidates.push(raw.clone());
    }
    candidates
}

fn is_safe_scan_entry(entry: &fs::DirEntry) -> bool {
    // 跳过隐藏目录和 symlink，避免遍历爆炸与符号链接逃逸。
    let Ok(meta) = entry.metadata() else {
        return false;
    };
    if meta.file_type().is_symlink() {
        return false;
    }
    let name = entry.file_name();
    let name_str = name.to_string_lossy();
    if name_str.starts_with('.') && name_str != "." && name_str != ".." {
        return false;
    }
    true
}

fn scan_db_file_name(
    root: &Path,
    target_dbnum: Option<u32>,
    target_types: Option<&[&str]>,
    depth: usize,
    visited: &mut usize,
    file_names: &mut Vec<String>,
) -> Result<bool> {
    if depth > SCAN_MAX_DEPTH {
        return Ok(false);
    }
    for entry in fs::read_dir(root)
        .with_context(|| format!("读取目录失败: {}", root.display()))?
        .flatten()
    {
        *visited += 1;
        if *visited > SCAN_MAX_FILES {
            bail!(
                "项目路径扫描文件数超过 {SCAN_MAX_FILES} 上限，请缩小 project_path 或在白名单中收紧"
            );
        }
        if !is_safe_scan_entry(&entry) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            if scan_db_file_name(
                &path,
                target_dbnum,
                target_types,
                depth + 1,
                visited,
                file_names,
            )? {
                return Ok(true);
            }
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if target_types.is_some() && file_name.contains('.') {
            continue;
        }
        let mut file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let mut buf = [0u8; 60];
        if file.read_exact(&mut buf).is_err() {
            continue;
        }
        let db_info = parse_file_basic_info(&buf);
        if let Some(dbnum) = target_dbnum {
            if db_info.dbnum == dbnum {
                file_names.push(file_name.to_string());
                return Ok(true);
            }
        }
        if let Some(types) = target_types {
            if types.contains(&db_info.db_type.as_str()) {
                file_names.push(file_name.to_string());
            }
        }
    }
    Ok(false)
}

fn find_db_file_name_for_dbnum(root: &Path, target_dbnum: u32) -> Result<Option<String>> {
    let mut visited = 0usize;
    let mut file_names = Vec::with_capacity(1);
    scan_db_file_name(
        root,
        Some(target_dbnum),
        None,
        0,
        &mut visited,
        &mut file_names,
    )?;
    Ok(file_names.into_iter().next())
}

fn collect_db_file_names_for_types(
    root: &Path,
    target_types: &[&str],
    file_names: &mut Vec<String>,
) -> Result<()> {
    let mut visited = 0usize;
    scan_db_file_name(root, None, Some(target_types), 0, &mut visited, file_names)?;
    Ok(())
}

fn should_include_system_db_files(site: &ManagedProjectSite) -> bool {
    if site.parse_status != ManagedSiteParseStatus::Parsed {
        let db_path = Path::new(&site.db_data_path);
        return !(site.last_parse_finished_at.is_some() && db_path.exists());
    }
    let db_path = Path::new(&site.db_data_path);
    !db_path.exists()
}

fn configured_parse_db_types(site: &ManagedProjectSite) -> Vec<String> {
    normalize_parse_db_types(site.parse_db_types.clone())
}

fn force_rebuild_system_db_enabled(site: &ManagedProjectSite) -> bool {
    let parse_db_types = configured_parse_db_types(site);
    normalize_force_rebuild_system_db(site.force_rebuild_system_db, &parse_db_types)
}

fn parse_scope_enabled(site: &ManagedProjectSite) -> bool {
    !site.manual_db_nums.is_empty() || !configured_parse_db_types(site).is_empty()
}

fn generation_enabled(site: &ManagedProjectSite) -> bool {
    site.gen_model || site.gen_mesh || site.gen_spatial_tree
}

fn resolve_included_db_files(site: &ManagedProjectSite) -> Result<Vec<String>> {
    let parse_db_types = configured_parse_db_types(site);
    if site.manual_db_nums.is_empty() && parse_db_types.is_empty() {
        return Ok(Vec::new());
    }

    let source_project_name = site_source_project_name(site);
    let project_root = project_dir_candidates(&source_project_name, &site.project_path)
        .into_iter()
        .find(|path| path.exists())
        .ok_or_else(|| anyhow!("项目路径不存在: {}", site.project_path))?;

    let mut file_names = Vec::new();
    let selected_non_system_types = parse_db_types
        .iter()
        .filter(|value| !REPARSE_REUSE_DB_TYPES.contains(&value.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let has_other_targets =
        !site.manual_db_nums.is_empty() || !selected_non_system_types.is_empty();
    let force_rebuild_system_db = force_rebuild_system_db_enabled(site);

    let include_reuse_types = parse_db_types
        .iter()
        .any(|value| REPARSE_REUSE_DB_TYPES.contains(&value.as_str()))
        && (force_rebuild_system_db || should_include_system_db_files(site) || !has_other_targets);

    tracing::debug!(
        site = %site.site_id,
        include_reuse_types,
        force_rebuild_system_db,
        parse_db_types = ?parse_db_types,
        manual_db_nums = ?site.manual_db_nums,
        "resolve_included_db_files"
    );

    if include_reuse_types {
        let reuse_refs = parse_db_types
            .iter()
            .filter(|value| REPARSE_REUSE_DB_TYPES.contains(&value.as_str()))
            .map(|value| value.as_str())
            .collect::<Vec<_>>();
        collect_db_file_names_for_types(&project_root, &reuse_refs, &mut file_names)?;
    }

    let include_desi_by_type =
        parse_db_types.iter().any(|value| value == "DESI") && site.manual_db_nums.is_empty();
    if include_desi_by_type {
        collect_db_file_names_for_types(&project_root, &["DESI"], &mut file_names)?;
    }

    let extra_type_refs = parse_db_types
        .iter()
        .filter(|value| {
            value.as_str() != "DESI" && !REPARSE_REUSE_DB_TYPES.contains(&value.as_str())
        })
        .map(|value| value.as_str())
        .collect::<Vec<_>>();
    if !extra_type_refs.is_empty() {
        collect_db_file_names_for_types(&project_root, &extra_type_refs, &mut file_names)?;
    }

    for dbnum in &site.manual_db_nums {
        let file_name = find_db_file_name_for_dbnum(&project_root, *dbnum)?
            .ok_or_else(|| anyhow!("项目路径下未找到 dbnum={} 对应的 db 文件", dbnum))?;
        file_names.push(file_name);
    }
    file_names.sort();
    file_names.dedup();
    Ok(file_names)
}

fn read_parse_config_included_db_files(site_id: &str) -> Vec<String> {
    let path = parse_config_path(site_id);
    let Ok(raw) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(value) = toml::from_str::<toml::Value>(&raw) else {
        return Vec::new();
    };
    value
        .get("included_db_files")
        .and_then(|entry| entry.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(|value| value.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn build_parse_plan_target_summary(
    site: &ManagedProjectSite,
    included_db_files: &[String],
) -> String {
    if !included_db_files.is_empty() {
        return included_db_files.join(", ");
    }
    if site.manual_db_nums.is_empty() {
        return "按项目配置全量解析".to_string();
    }
    let db_nums = site
        .manual_db_nums
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    format!("dbnum={}", db_nums.join(", "))
}

fn is_system_db_file(file_name: &str) -> bool {
    file_name.to_ascii_lowercase().contains("sys")
}

fn data_target_summary(site: &ManagedProjectSite, included_db_files: &[String]) -> String {
    let data_files = included_db_files
        .iter()
        .filter(|file_name| !is_system_db_file(file_name))
        .cloned()
        .collect::<Vec<_>>();
    if data_files.is_empty() {
        return "仅系统库".to_string();
    }
    build_parse_plan_target_summary(site, &data_files)
}

fn build_parse_type_summary(site: &ManagedProjectSite) -> String {
    let parse_db_types = configured_parse_db_types(site);
    if parse_db_types.is_empty() {
        return "未额外勾选类型".to_string();
    }
    parse_db_types.join(", ")
}

fn build_parse_plan_with_files(
    site: &ManagedProjectSite,
    included_db_files: Vec<String>,
) -> ManagedSiteParsePlan {
    let parse_type_summary = build_parse_type_summary(site);
    let parse_scope_enabled = parse_scope_enabled(site);
    let force_rebuild_system_db = force_rebuild_system_db_enabled(site);
    let selected_reuse_types = configured_parse_db_types(site)
        .iter()
        .any(|value| REPARSE_REUSE_DB_TYPES.contains(&value.as_str()));
    let needs_bootstrap_system_db = should_include_system_db_files(site);

    if !parse_scope_enabled {
        let detail = if included_db_files.is_empty() {
            "当前没有限制 db 文件，解析时会按项目配置做全量解析。".to_string()
        } else {
            format!(
                "当前按配置解析这些文件：{}。",
                build_parse_plan_target_summary(site, &included_db_files)
            )
        };
        return ManagedSiteParsePlan {
            mode: ManagedSiteParsePlanMode::Full,
            label: "全量解析".to_string(),
            detail,
            includes_system_db_files: true,
            included_db_files,
        };
    }

    let includes_system_db_files = if included_db_files.is_empty() {
        selected_reuse_types && (needs_bootstrap_system_db || force_rebuild_system_db)
    } else {
        included_db_files
            .iter()
            .any(|file_name| is_system_db_file(file_name))
    };
    let target_summary = build_parse_plan_target_summary(site, &included_db_files);
    let data_target_summary = data_target_summary(site, &included_db_files);

    if includes_system_db_files {
        if force_rebuild_system_db && !needs_bootstrap_system_db {
            ManagedSiteParsePlan {
                mode: ManagedSiteParsePlanMode::RebuildSystem,
                label: "重建系统库".to_string(),
                detail: format!(
                    "已勾选类型：{}。已开启强制重建系统库，本次会重新解析 SYST，再解析目标文件：{}。",
                    parse_type_summary, data_target_summary
                ),
                includes_system_db_files,
                included_db_files,
            }
        } else {
            ManagedSiteParsePlan {
                mode: ManagedSiteParsePlanMode::Bootstrap,
                label: "首次解析".to_string(),
                detail: format!(
                    "已勾选类型：{}。本次会补齐系统数据，再解析目标文件：{}。",
                    parse_type_summary, data_target_summary
                ),
                includes_system_db_files,
                included_db_files,
            }
        }
    } else if selected_reuse_types
        && needs_bootstrap_system_db == false
        && force_rebuild_system_db == false
        && (site.manual_db_nums.len() > 0 || !included_db_files.is_empty())
    {
        ManagedSiteParsePlan {
            mode: ManagedSiteParsePlanMode::FastReparse,
            label: "快速重解析".to_string(),
            detail: format!(
                "已勾选类型：{}。本次复用已解析的 SYST，只解析当前目标：{}。",
                parse_type_summary, target_summary
            ),
            includes_system_db_files,
            included_db_files,
        }
    } else {
        ManagedSiteParsePlan {
            mode: ManagedSiteParsePlanMode::Selective,
            label: "按范围解析".to_string(),
            detail: format!(
                "已勾选类型：{}。本次按当前范围解析：{}。",
                parse_type_summary, target_summary
            ),
            includes_system_db_files,
            included_db_files,
        }
    }
}

fn build_parse_plan(site: &ManagedProjectSite) -> ManagedSiteParsePlan {
    build_parse_plan_with_files(site, read_parse_config_included_db_files(&site.site_id))
}

fn annotate_site_parse_plan(site: &mut ManagedProjectSite) {
    site.parse_plan = build_parse_plan(site);
}

fn annotate_sites_parse_plans(sites: &mut [ManagedProjectSite]) {
    for site in sites.iter_mut() {
        annotate_site_parse_plan(site);
    }
}

// ─── TOML helpers ───────────────────────────────────────────────────────────

fn set_toml_string(table: &mut toml::value::Table, key: &str, value: impl Into<String>) {
    table.insert(key.to_string(), toml::Value::String(value.into()));
}

fn set_toml_integer(table: &mut toml::value::Table, key: &str, value: i64) {
    table.insert(key.to_string(), toml::Value::Integer(value));
}

fn set_toml_bool(table: &mut toml::value::Table, key: &str, value: bool) {
    table.insert(key.to_string(), toml::Value::Boolean(value));
}

fn set_toml_float(table: &mut toml::value::Table, key: &str, value: f64) {
    table.insert(key.to_string(), toml::Value::Float(value));
}

fn set_toml_array(table: &mut toml::value::Table, key: &str, values: Vec<String>) {
    table.insert(
        key.to_string(),
        toml::Value::Array(values.into_iter().map(toml::Value::String).collect()),
    );
}

fn set_toml_integer_array(table: &mut toml::value::Table, key: &str, values: Vec<u32>) {
    table.insert(
        key.to_string(),
        toml::Value::Array(
            values
                .into_iter()
                .map(|value| toml::Value::Integer(value as i64))
                .collect(),
        ),
    );
}

fn ensure_table<'a>(table: &'a mut toml::value::Table, key: &str) -> &'a mut toml::value::Table {
    let value = table
        .entry(key.to_string())
        .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
    if !value.is_table() {
        *value = toml::Value::Table(toml::value::Table::new());
    }
    value.as_table_mut().expect("table inserted")
}

// ─── Config builders ────────────────────────────────────────────────────────

fn build_site_config(
    site: &ManagedProjectSite,
    db_user: &str,
    db_password: &str,
) -> Result<String> {
    let template_path = current_config_source();
    let raw = fs::read_to_string(&template_path)
        .with_context(|| format!("读取模板配置失败: {}", template_path.display()))?;
    let mut value = toml::from_str::<toml::Value>(&raw)?;
    let table = value
        .as_table_mut()
        .ok_or_else(|| anyhow!("DbOption 模板不是 table 结构"))?;

    let source_project_name = site_source_project_name(site);
    let runtime_cfg = DatabaseConfig {
        project_name: source_project_name.clone(),
        project_path: site.project_path.clone(),
        project_code: site.project_code,
        manual_db_nums: site.manual_db_nums.clone(),
        surreal_ns: site.project_code,
        db_ip: "127.0.0.1".to_string(),
        db_port: site.db_port.to_string(),
        db_user: db_user.to_string(),
        db_password: db_password.to_string(),
        gen_model: site.gen_model,
        gen_mesh: site.gen_mesh,
        gen_spatial_tree: site.gen_spatial_tree,
        apply_boolean_operation: site.apply_boolean_operation,
        mesh_tol_ratio: site.mesh_tol_ratio,
        export_json: site.export_json,
        export_parquet: site.export_parquet,
        ..DatabaseConfig::from_db_option(&aios_core::get_db_option())
    };
    let db_option = runtime_cfg.to_runtime_db_option();
    let (project_root, included_projects, project_dirs) =
        split_project_root(&source_project_name, &site.project_path);

    set_toml_string(table, "project_name", source_project_name);
    set_toml_string(table, "project_path", project_root);
    set_toml_string(table, "project_code", site.project_code.to_string());
    set_toml_string(table, "surreal_ns", site.project_code.to_string());
    set_toml_string(table, "mdb_name", db_option.mdb_name.clone());
    set_toml_string(table, "module", db_option.module.clone());
    set_toml_string(table, "surreal_ip", "127.0.0.1");
    set_toml_integer(table, "surreal_port", site.db_port as i64);
    set_toml_string(table, "surreal_user", db_user.to_string());
    set_toml_string(table, "surreal_password", db_password.to_string());
    set_toml_string(table, "surreal_script_dir", "resource/surreal");
    table.remove("v_ip");
    table.remove("v_port");
    table.remove("v_user");
    table.remove("v_password");
    set_toml_array(table, "included_projects", included_projects);
    set_toml_array(table, "project_dirs", project_dirs);
    set_toml_integer_array(table, "manual_db_nums", runtime_cfg.manual_db_nums.clone());
    set_toml_bool(table, "gen_model", runtime_cfg.gen_model);
    set_toml_bool(table, "gen_mesh", runtime_cfg.gen_mesh);
    set_toml_bool(table, "gen_spatial_tree", runtime_cfg.gen_spatial_tree);
    set_toml_bool(
        table,
        "apply_boolean_operation",
        runtime_cfg.apply_boolean_operation,
    );
    set_toml_float(table, "mesh_tol_ratio", runtime_cfg.mesh_tol_ratio);
    set_toml_bool(table, "export_json", runtime_cfg.export_json);
    set_toml_bool(table, "export_parquet", runtime_cfg.export_parquet);
    set_toml_bool(
        table,
        "export_parquet_after_gen",
        runtime_cfg.export_parquet,
    );

    let web_server = ensure_table(table, "web_server");
    set_toml_integer(web_server, "port", site.web_port as i64);
    set_toml_string(web_server, "bind_host", site.bind_host.clone());
    set_toml_string(web_server, "site_id", site.site_id.clone());
    set_toml_string(web_server, "site_name", site.project_name.clone());
    set_toml_string(web_server, "region", "admin");
    let (local_url, _public_opt, effective_url) =
        derive_entry_urls(site.web_port, &site.bind_host, &site.public_base_url);
    let effective = effective_url.unwrap_or_else(|| local_url.clone().unwrap_or_default());
    let local = local_url.unwrap_or_default();
    set_toml_string(web_server, "frontend_url", effective.clone());
    set_toml_string(web_server, "public_base_url", effective);
    set_toml_string(web_server, "backend_url", local);
    set_toml_bool(web_server, "auto_start_surreal", false);
    set_toml_string(web_server, "surreal_bin", "surreal");
    set_toml_string(web_server, "surreal_data_path", site.db_data_path.clone());
    set_toml_string(
        web_server,
        "surreal_bind",
        format!("127.0.0.1:{}", site.db_port),
    );
    web_server.remove("surreal_user");
    web_server.remove("surreal_password");

    let surrealdb = ensure_table(table, "surrealdb");
    set_toml_string(surrealdb, "mode", "ws");
    set_toml_string(surrealdb, "ip", "127.0.0.1");
    set_toml_integer(surrealdb, "port", site.db_port as i64);
    set_toml_string(surrealdb, "user", db_user.to_string());
    set_toml_string(surrealdb, "password", db_password.to_string());
    set_toml_string(surrealdb, "path", site.db_data_path.replace('\\', "/"));

    let surrealkv = ensure_table(table, "surrealkv");
    set_toml_bool(surrealkv, "enabled", false);
    set_toml_string(
        surrealkv,
        "path",
        format!("{}.kv", site.db_data_path.replace('\\', "/")),
    );

    Ok(toml::to_string_pretty(&value)?)
}

fn build_parse_config(
    site: &ManagedProjectSite,
    db_user: &str,
    db_password: &str,
) -> Result<String> {
    let content = build_site_config(site, db_user, db_password)?;
    let mut value = toml::from_str::<toml::Value>(&content)?;
    let table = value
        .as_table_mut()
        .ok_or_else(|| anyhow!("DbOption 解析配置不是 table 结构"))?;
    table.remove("web_server");
    set_toml_bool(table, "total_sync", true);
    set_toml_bool(table, "incr_sync", false);
    set_toml_bool(table, "sync_history", false);
    set_toml_bool(table, "only_sync_sys", false);
    set_toml_bool(table, "gen_tree_only", false);
    set_toml_bool(table, "enable_log", true);
    set_toml_bool(table, "save_db", true);
    let included_db_files = resolve_included_db_files(site)?;
    if included_db_files.is_empty() {
        table.remove("included_db_files");
    } else {
        set_toml_array(table, "included_db_files", included_db_files);
    }
    Ok(toml::to_string_pretty(&value)?)
}

/// 原子地写入文件：先写同目录下 `*.tmp` 再 rename；Unix 上落地前将模式改为 0600。
fn write_file_atomic(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建父目录失败: {}", parent.display()))?;
    }
    let file_name = path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("pending");
    let tmp = path.with_file_name(format!("{file_name}.tmp"));
    fs::write(&tmp, content).with_context(|| format!("写入临时文件失败: {}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(err) = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600)) {
            tracing::warn!("降权 {} 失败: {err}", tmp.display());
        }
    }
    fs::rename(&tmp, path).with_context(|| {
        format!(
            "重命名临时文件失败: {} -> {}",
            tmp.display(),
            path.display()
        )
    })?;
    Ok(())
}

fn write_site_files(site: &ManagedProjectSite, db_user: &str, db_password: &str) -> Result<()> {
    ensure_runtime_dirs(&site.site_id)?;
    let content = build_site_config(site, db_user, db_password)?;
    write_file_atomic(Path::new(&site.config_path), &content)?;
    let parse_content = build_parse_config(site, db_user, db_password)?;
    write_file_atomic(&parse_config_path(&site.site_id), &parse_content)?;
    let metadata = serde_json::to_string_pretty(&json!({
        "site_id": site.site_id,
        "project_name": site.project_name,
        "project_code": site.project_code,
        "project_path": site.project_path,
        "manual_db_nums": site.manual_db_nums,
        "parse_db_types": site.parse_db_types,
        "force_rebuild_system_db": site.force_rebuild_system_db,
        "gen_model": site.gen_model,
        "gen_mesh": site.gen_mesh,
        "gen_spatial_tree": site.gen_spatial_tree,
        "apply_boolean_operation": site.apply_boolean_operation,
        "mesh_tol_ratio": site.mesh_tol_ratio,
        "export_json": site.export_json,
        "export_parquet": site.export_parquet,
        "db_port": site.db_port,
        "web_port": site.web_port,
        "entry_url": site.entry_url,
        "updated_at": site.updated_at,
    }))?;
    write_file_atomic(&metadata_path(&site.site_id), &metadata)?;
    Ok(())
}

fn derive_entry_urls(
    web_port: u16,
    bind_host: &str,
    public_base_url: &Option<String>,
) -> (Option<String>, Option<String>, Option<String>) {
    let local = format!("http://127.0.0.1:{}", web_port);
    let public = public_base_url
        .as_ref()
        .map(|url| url.trim_end_matches('/').to_string())
        .or_else(|| {
            let h = bind_host.trim();
            if !h.is_empty() && h != "0.0.0.0" && h != "127.0.0.1" && h != "localhost" {
                Some(format!("http://{}:{}", h, web_port))
            } else {
                None
            }
        });
    let entry = public.clone().unwrap_or_else(|| local.clone());
    (Some(local), public, Some(entry))
}

// ─── Row mapping ────────────────────────────────────────────────────────────

fn row_bool_or(row: &rusqlite::Row<'_>, column: &str, default: bool) -> bool {
    row.get::<_, Option<i64>>(column)
        .ok()
        .flatten()
        .map(|value| value != 0)
        .unwrap_or(default)
}

fn row_f64_or(row: &rusqlite::Row<'_>, column: &str, default: f64) -> f64 {
    row.get::<_, Option<f64>>(column)
        .ok()
        .flatten()
        .unwrap_or(default)
}

fn row_to_site(row: &rusqlite::Row<'_>) -> rusqlite::Result<ManagedProjectSite> {
    let web_port = row.get::<_, i64>("web_port")? as u16;
    let bind_host: String = row.get("bind_host")?;
    let public_base_url: Option<String> = row.get("public_base_url").unwrap_or(None);
    let associated_project: Option<String> = row.get("associated_project").unwrap_or(None);
    let (local_entry_url, public_entry_url, entry_url) =
        derive_entry_urls(web_port, &bind_host, &public_base_url);
    Ok(ManagedProjectSite {
        site_id: row.get("site_id")?,
        project_name: row.get("project_name")?,
        project_code: row.get::<_, i64>("project_code")? as u32,
        project_path: row.get("project_path")?,
        manual_db_nums: manual_db_nums_from_json(row.get("manual_db_nums")?),
        parse_db_types: parse_db_types_from_json(row.get("parse_db_types").unwrap_or(None)),
        force_rebuild_system_db: row
            .get::<_, Option<i64>>("force_rebuild_system_db")
            .unwrap_or(None)
            .unwrap_or(0)
            != 0,
        gen_model: row_bool_or(row, "gen_model", true),
        gen_mesh: row_bool_or(row, "gen_mesh", false),
        gen_spatial_tree: row_bool_or(row, "gen_spatial_tree", true),
        apply_boolean_operation: row_bool_or(row, "apply_boolean_operation", true),
        mesh_tol_ratio: row_f64_or(row, "mesh_tol_ratio", 3.0),
        export_json: row_bool_or(row, "export_json", false),
        export_parquet: row_bool_or(row, "export_parquet", true),
        config_path: row.get("config_path")?,
        runtime_dir: row.get("runtime_dir")?,
        db_data_path: row.get("db_data_path")?,
        db_port: row.get::<_, i64>("db_port")? as u16,
        web_port,
        viewer_port: row
            .get::<_, Option<i64>>("viewer_port")
            .unwrap_or(None)
            .map(|value| value as u16),
        bind_host,
        public_base_url,
        associated_project,
        db_pid: row
            .get::<_, Option<i64>>("db_pid")?
            .map(|value| value as u32),
        web_pid: row
            .get::<_, Option<i64>>("web_pid")?
            .map(|value| value as u32),
        viewer_pid: row
            .get::<_, Option<i64>>("viewer_pid")
            .unwrap_or(None)
            .map(|value| value as u32),
        viewer_url: row.get("viewer_url").unwrap_or(None),
        parse_pid: row
            .get::<_, Option<i64>>("parse_pid")?
            .map(|value| value as u32),
        status: status_from_str(&row.get::<_, String>("status")?),
        parse_status: parse_status_from_str(&row.get::<_, String>("parse_status")?),
        last_error: row.get("last_error")?,
        entry_url,
        local_entry_url,
        public_entry_url,
        last_parse_started_at: row.get("last_parse_started_at")?,
        last_parse_finished_at: row.get("last_parse_finished_at")?,
        last_parse_duration_ms: row
            .get::<_, Option<i64>>("last_parse_duration_ms")?
            .map(|value| value as u64),
        parse_plan: ManagedSiteParsePlan::default(),
        risk_level: ManagedSiteRiskLevel::Normal,
        risk_reasons: Vec::new(),
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

// ─── Schema / migrations ────────────────────────────────────────────────────

fn ensure_schema_with_conn(conn: &Connection) -> Result<()> {
    conn.execute_batch(&format!(
        r#"
        CREATE TABLE IF NOT EXISTS {table} (
            site_id TEXT PRIMARY KEY,
            project_name TEXT NOT NULL,
            project_code INTEGER NOT NULL,
            project_path TEXT NOT NULL,
            manual_db_nums TEXT NOT NULL DEFAULT '[]',
            parse_db_types TEXT NOT NULL DEFAULT '["SYST","DESI"]',
            force_rebuild_system_db INTEGER NOT NULL DEFAULT 0,
            gen_model INTEGER NOT NULL DEFAULT 1,
            gen_mesh INTEGER NOT NULL DEFAULT 0,
            gen_spatial_tree INTEGER NOT NULL DEFAULT 1,
            apply_boolean_operation INTEGER NOT NULL DEFAULT 1,
            mesh_tol_ratio REAL NOT NULL DEFAULT 3.0,
            export_json INTEGER NOT NULL DEFAULT 0,
            export_parquet INTEGER NOT NULL DEFAULT 1,
            config_path TEXT NOT NULL,
            runtime_dir TEXT NOT NULL,
            db_data_path TEXT NOT NULL,
            db_port INTEGER NOT NULL,
            web_port INTEGER NOT NULL,
            viewer_port INTEGER,
            bind_host TEXT NOT NULL,
            db_pid INTEGER,
            web_pid INTEGER,
            viewer_pid INTEGER,
            viewer_url TEXT,
            parse_pid INTEGER,
            status TEXT NOT NULL,
            parse_status TEXT NOT NULL,
            last_error TEXT,
            entry_url TEXT,
            db_user TEXT,
            db_password TEXT,
            last_parse_started_at TEXT,
            last_parse_finished_at TEXT,
            last_parse_duration_ms INTEGER,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_managed_project_sites_project_name ON {table}(project_name);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_managed_project_sites_db_port ON {table}(db_port);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_managed_project_sites_web_port ON {table}(web_port);
        "#,
        table = TABLE_NAME
    ))?;

    let mut current_version: u32 = conn
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .unwrap_or(0) as u32;

    if current_version < 1 {
        for column in [
            "manual_db_nums",
            "last_parse_started_at",
            "last_parse_finished_at",
            "last_parse_duration_ms",
            "public_base_url",
            "associated_project",
        ] {
            ensure_column_exists(conn, column)?;
        }
        current_version = 1;
        conn.pragma_update(None, "user_version", current_version as i64)?;
    }
    if current_version < 2 {
        // schema v2：显式保证所有 v1 新增列也存在（用于历史库的兜底）。
        for column in ["public_base_url", "associated_project"] {
            ensure_column_exists(conn, column)?;
        }
        current_version = 2;
        conn.pragma_update(None, "user_version", current_version as i64)?;
    }
    if current_version < 3 {
        ensure_column_exists(conn, "parse_db_types")?;
        current_version = 3;
        conn.pragma_update(None, "user_version", current_version as i64)?;
    }
    if current_version < 4 {
        ensure_column_exists(conn, "force_rebuild_system_db")?;
        current_version = 4;
        conn.pragma_update(None, "user_version", current_version as i64)?;
    }
    if current_version < 5 {
        for column in [
            "gen_model",
            "gen_mesh",
            "gen_spatial_tree",
            "apply_boolean_operation",
            "mesh_tol_ratio",
            "export_json",
            "export_parquet",
        ] {
            ensure_column_exists(conn, column)?;
        }
        current_version = 5;
        conn.pragma_update(None, "user_version", current_version as i64)?;
    }
    if current_version < 6 {
        for column in ["viewer_port", "viewer_pid", "viewer_url"] {
            ensure_column_exists(conn, column)?;
        }
        current_version = 6;
        conn.pragma_update(None, "user_version", current_version as i64)?;
    }
    debug_assert!(current_version <= SCHEMA_VERSION);
    Ok(())
}

fn ensure_column_exists(conn: &Connection, column: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({TABLE_NAME})"))?;
    let has_column = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .flatten()
        .any(|c| c == column);
    if !has_column {
        let column_type = match column {
            "last_parse_duration_ms" => "INTEGER",
            "viewer_port" | "viewer_pid" => "INTEGER",
            "manual_db_nums" => "TEXT NOT NULL DEFAULT '[]'",
            "parse_db_types" => "TEXT NOT NULL DEFAULT '[\"SYST\",\"DESI\"]'",
            "force_rebuild_system_db" => "INTEGER NOT NULL DEFAULT 0",
            "gen_model" => "INTEGER NOT NULL DEFAULT 1",
            "gen_mesh" => "INTEGER NOT NULL DEFAULT 0",
            "gen_spatial_tree" => "INTEGER NOT NULL DEFAULT 1",
            "apply_boolean_operation" => "INTEGER NOT NULL DEFAULT 1",
            "mesh_tol_ratio" => "REAL NOT NULL DEFAULT 3.0",
            "export_json" => "INTEGER NOT NULL DEFAULT 0",
            "export_parquet" => "INTEGER NOT NULL DEFAULT 1",
            _ => "TEXT",
        };
        conn.execute(
            &format!(
                "ALTER TABLE {table} ADD COLUMN {column} {column_type}",
                table = TABLE_NAME
            ),
            [],
        )?;
    }
    Ok(())
}

pub fn ensure_schema() -> Result<()> {
    with_conn(|conn| ensure_schema_with_conn(conn))
}

// ─── Low-level queries ──────────────────────────────────────────────────────

fn load_site_with_conn(conn: &Connection, site_id: &str) -> Result<Option<ManagedProjectSite>> {
    let sql = format!(
        "SELECT * FROM {table} WHERE site_id = ?1",
        table = TABLE_NAME
    );
    let site = conn.query_row(&sql, [site_id], row_to_site).optional()?;
    Ok(site)
}

fn persist_site_with_conn(
    conn: &Connection,
    site: &ManagedProjectSite,
    db_user: &str,
    db_password: &str,
) -> Result<()> {
    conn.execute(
        &format!(
            "INSERT OR REPLACE INTO {table} (
                site_id, project_name, project_code, project_path, config_path, runtime_dir,
                manual_db_nums, parse_db_types, force_rebuild_system_db,
                gen_model, gen_mesh, gen_spatial_tree, apply_boolean_operation, mesh_tol_ratio,
                export_json, export_parquet,
                db_data_path, db_port, web_port, viewer_port, bind_host, public_base_url,
                associated_project,
                db_pid, web_pid, viewer_pid, viewer_url, parse_pid,
                status, parse_status, last_error, entry_url, db_user, db_password,
                last_parse_started_at, last_parse_finished_at, last_parse_duration_ms,
                created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35, ?36, ?37, ?38, ?39)",
            table = TABLE_NAME
        ),
        params![
            &site.site_id,
            &site.project_name,
            site.project_code as i64,
            &site.project_path,
            &site.config_path,
            &site.runtime_dir,
            manual_db_nums_to_json(&site.manual_db_nums)?,
            parse_db_types_to_json(&site.parse_db_types)?,
            if site.force_rebuild_system_db { 1i64 } else { 0i64 },
            if site.gen_model { 1i64 } else { 0i64 },
            if site.gen_mesh { 1i64 } else { 0i64 },
            if site.gen_spatial_tree { 1i64 } else { 0i64 },
            if site.apply_boolean_operation { 1i64 } else { 0i64 },
            site.mesh_tol_ratio,
            if site.export_json { 1i64 } else { 0i64 },
            if site.export_parquet { 1i64 } else { 0i64 },
            &site.db_data_path,
            site.db_port as i64,
            site.web_port as i64,
            site.viewer_port.map(|value| value as i64),
            &site.bind_host,
            &site.public_base_url,
            &site.associated_project,
            site.db_pid.map(|value| value as i64),
            site.web_pid.map(|value| value as i64),
            site.viewer_pid.map(|value| value as i64),
            &site.viewer_url,
            site.parse_pid.map(|value| value as i64),
            status_to_str(&site.status),
            parse_status_to_str(&site.parse_status),
            &site.last_error,
            &site.entry_url,
            db_user,
            db_password,
            &site.last_parse_started_at,
            &site.last_parse_finished_at,
            site.last_parse_duration_ms.map(|value| value as i64),
            &site.created_at,
            &site.updated_at,
        ],
    )?;
    Ok(())
}

fn load_credentials_with_conn(conn: &Connection, site_id: &str) -> Result<(String, String)> {
    let sql = format!(
        "SELECT db_user, db_password FROM {table} WHERE site_id = ?1",
        table = TABLE_NAME
    );
    conn.query_row(&sql, [site_id], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?
                .unwrap_or_else(|| "root".to_string()),
            row.get::<_, Option<String>>(1)?
                .unwrap_or_else(|| "root".to_string()),
        ))
    })
    .optional()?
    .ok_or_else(|| anyhow!("站点不存在"))
}

fn load_site_and_credentials(site_id: &str) -> Result<(ManagedProjectSite, String, String)> {
    with_conn(|conn| {
        let site = load_site_with_conn(conn, site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
        let (db_user, db_password) = load_credentials_with_conn(conn, site_id)?;
        Ok((site, db_user, db_password))
    })
}

fn rewrite_site_files_from_storage(site_id: &str) -> Result<()> {
    let (site, db_user, db_password) = load_site_and_credentials(site_id)?;
    write_site_files(&site, &db_user, &db_password)
}

fn assert_port_available_with_conn(
    conn: &Connection,
    exclude_site_id: Option<&str>,
    db_port: u16,
    web_port: u16,
) -> Result<()> {
    if db_port == web_port {
        bail!("数据库端口和站点端口不能相同: {}", db_port);
    }
    let sql = format!(
        "SELECT site_id, db_port, web_port, viewer_port FROM {table} WHERE (?1 IS NULL OR site_id != ?1)",
        table = TABLE_NAME
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([exclude_site_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)? as u16,
            row.get::<_, i64>(2)? as u16,
            row.get::<_, Option<i64>>(3)?.map(|value| value as u16),
        ))
    })?;
    for row in rows {
        let (site_id, existing_db_port, existing_web_port, existing_viewer_port) = row?;
        for (candidate, label) in [(db_port, "数据库"), (web_port, "站点")] {
            if existing_db_port == candidate {
                bail!(
                    "{}端口 {} 已被站点 {} 的数据库端口使用",
                    label,
                    candidate,
                    site_id
                );
            }
            if existing_web_port == candidate {
                bail!(
                    "{}端口 {} 已被站点 {} 的站点端口使用",
                    label,
                    candidate,
                    site_id
                );
            }
            if existing_viewer_port == Some(candidate) {
                bail!(
                    "{}端口 {} 已被站点 {} 的 Viewer 端口使用",
                    label,
                    candidate,
                    site_id
                );
            }
        }
    }
    if port_in_use("127.0.0.1", db_port) {
        bail!("数据库端口 {} 已被当前机器上的其他进程占用", db_port);
    }
    if port_in_use("127.0.0.1", web_port) {
        bail!("站点端口 {} 已被当前机器上的其他进程占用", web_port);
    }
    Ok(())
}

fn collect_reserved_ports_with_conn(
    conn: &Connection,
    exclude_site_id: Option<&str>,
) -> Result<HashSet<u16>> {
    let sql = format!(
        "SELECT db_port, web_port, viewer_port FROM {table} WHERE (?1 IS NULL OR site_id != ?1)",
        table = TABLE_NAME
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([exclude_site_id], |row| {
        Ok((
            row.get::<_, i64>(0)? as u16,
            row.get::<_, i64>(1)? as u16,
            row.get::<_, Option<i64>>(2)?.map(|value| value as u16),
        ))
    })?;
    let mut used = HashSet::new();
    for row in rows {
        let (db_port, web_port, viewer_port) = row?;
        used.insert(db_port);
        used.insert(web_port);
        if let Some(port) = viewer_port {
            used.insert(port);
        }
    }
    Ok(used)
}

fn first_available_port(used: &mut HashSet<u16>, start: u16, end: u16) -> Result<u16> {
    for port in start..=end {
        if used.contains(&port) || port_in_use("127.0.0.1", port) {
            continue;
        }
        used.insert(port);
        return Ok(port);
    }
    bail!("没有可用端口: {}-{}", start, end)
}

fn reserve_explicit_port(used: &mut HashSet<u16>, port: u16, label: &str) -> Result<u16> {
    if used.contains(&port) {
        bail!("{}端口 {} 已被已有站点使用", label, port);
    }
    if port_in_use("127.0.0.1", port) {
        bail!("{}端口 {} 已被当前机器上的其他进程占用", label, port);
    }
    used.insert(port);
    Ok(port)
}

fn resolve_create_ports_with_conn(
    conn: &Connection,
    db_port: Option<u16>,
    web_port: Option<u16>,
) -> Result<(u16, u16)> {
    let mut used = collect_reserved_ports_with_conn(conn, None)?;
    let db_port = match db_port.filter(|port| *port != 0) {
        Some(port) => reserve_explicit_port(&mut used, port, "数据库")?,
        None => first_available_port(&mut used, AUTO_DB_PORT_START, AUTO_PORT_END)?,
    };
    let web_port = match web_port.filter(|port| *port != 0) {
        Some(port) => reserve_explicit_port(&mut used, port, "站点")?,
        None => first_available_port(&mut used, AUTO_WEB_PORT_START, AUTO_PORT_END)?,
    };
    if db_port == web_port {
        bail!("数据库端口和站点端口不能相同: {}", db_port);
    }
    Ok((db_port, web_port))
}

// ─── Public read-side API ───────────────────────────────────────────────────

pub fn get_site(site_id: &str) -> Result<Option<ManagedProjectSite>> {
    let mut site = with_conn(|conn| load_site_with_conn(conn, site_id))?;
    if let Some(item) = site.as_mut() {
        *item = derive_runtime_state(item.clone());
        annotate_site_parse_plan(item);
        annotate_site_risk(item);
    }
    Ok(site)
}

pub fn list_sites() -> Result<Vec<ManagedProjectSite>> {
    let mut items = with_conn(|conn| {
        let mut stmt = conn.prepare(&format!(
            "SELECT * FROM {table} ORDER BY updated_at DESC",
            table = TABLE_NAME
        ))?;
        let rows = stmt.query_map([], row_to_site)?;
        let mut collected = Vec::new();
        for row in rows {
            collected.push(row?);
        }
        Ok(collected)
    })?;
    for item in items.iter_mut() {
        *item = derive_runtime_state(item.clone());
    }
    annotate_sites_parse_plans(&mut items);
    annotate_sites_risks(&mut items);
    Ok(items)
}

// ─── Write-side API ─────────────────────────────────────────────────────────

fn lock_op() -> Result<MutexGuard<'static, ()>> {
    site_op_lock()
        .lock()
        .map_err(|_| anyhow!("站点操作锁已中毒"))
}

pub fn create_site(req: CreateManagedSiteRequest) -> Result<ManagedProjectSite> {
    if req.project_name.trim().is_empty() {
        bail!("项目名不能为空");
    }
    if req.project_path.trim().is_empty() {
        bail!("项目路径不能为空");
    }
    if req.project_code == 0 {
        bail!("项目代号必须大于 0");
    }
    let canonical_path = canonical_project_path(req.project_path.trim())?;

    let _guard = lock_op()?;

    let (db_port, web_port) =
        with_conn(|conn| resolve_create_ports_with_conn(conn, req.db_port, req.web_port))?;
    let site_id = infer_site_id(&req.project_name, web_port);
    let created_at = now_rfc3339();
    let bind_host = normalize_host(req.bind_host);
    assert_bind_host_safe(&bind_host)?;
    let public_base_url = req
        .public_base_url
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.trim().to_string());
    let associated_project = req
        .associated_project
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.trim().to_string());
    let (local_entry_url, public_entry_url, entry_url) =
        derive_entry_urls(web_port, &bind_host, &public_base_url);
    let db_user = require_db_user(req.db_user)?;
    let db_password = require_db_password(req.db_password)?;
    assert_db_credentials_strong(&db_user, &db_password)?;

    let parse_db_types = normalize_parse_db_types(req.parse_db_types);
    let generation_defaults = default_generation_config();
    let site = ManagedProjectSite {
        site_id: site_id.clone(),
        project_name: req.project_name.trim().to_string(),
        project_code: req.project_code,
        project_path: canonical_path.to_string_lossy().to_string(),
        manual_db_nums: normalize_manual_db_nums(req.manual_db_nums),
        force_rebuild_system_db: normalize_force_rebuild_system_db(
            req.force_rebuild_system_db,
            &parse_db_types,
        ),
        parse_db_types,
        gen_model: req.gen_model.unwrap_or(generation_defaults.gen_model),
        gen_mesh: req.gen_mesh.unwrap_or(generation_defaults.gen_mesh),
        gen_spatial_tree: req
            .gen_spatial_tree
            .unwrap_or(generation_defaults.gen_spatial_tree),
        apply_boolean_operation: req
            .apply_boolean_operation
            .unwrap_or(generation_defaults.apply_boolean_operation),
        mesh_tol_ratio: normalize_mesh_tol_ratio(
            req.mesh_tol_ratio,
            generation_defaults.mesh_tol_ratio,
        ),
        export_json: req.export_json.unwrap_or(generation_defaults.export_json),
        export_parquet: req
            .export_parquet
            .unwrap_or(generation_defaults.export_parquet),
        config_path: config_path(&site_id).to_string_lossy().to_string(),
        runtime_dir: site_runtime_dir(&site_id).to_string_lossy().to_string(),
        db_data_path: db_data_path(&site_id).to_string_lossy().to_string(),
        db_port,
        web_port,
        viewer_port: None,
        bind_host,
        public_base_url,
        associated_project,
        db_pid: None,
        web_pid: None,
        viewer_pid: None,
        viewer_url: None,
        parse_pid: None,
        status: ManagedSiteStatus::Draft,
        parse_status: ManagedSiteParseStatus::Pending,
        last_error: None,
        entry_url,
        local_entry_url,
        public_entry_url,
        last_parse_started_at: None,
        last_parse_finished_at: None,
        last_parse_duration_ms: None,
        parse_plan: ManagedSiteParsePlan::default(),
        risk_level: ManagedSiteRiskLevel::Normal,
        risk_reasons: Vec::new(),
        created_at: created_at.clone(),
        updated_at: created_at,
    };

    // 先持久化（事务中校验端口冲突），再落磁盘；失败时回滚并清掉孤儿目录。
    with_tx(|conn| {
        assert_port_available_with_conn(conn, None, site.db_port, site.web_port)?;
        persist_site_with_conn(conn, &site, &db_user, &db_password)?;
        Ok(())
    })?;

    if let Err(err) = write_site_files(&site, &db_user, &db_password) {
        tracing::error!(site = %site.site_id, "创建站点时写入配置失败: {err}");
        // DB 已经成功插入，尝试回滚磁盘后返回错误；DB 条目保留以便 UI 重试/删除。
        let _ = fs::remove_dir_all(site_runtime_dir(&site.site_id));
        return Err(err);
    }

    let mut site = site;
    annotate_site_parse_plan(&mut site);

    // D1 / Sprint D · 修 G8：写盘 + 落磁盘均成功后立即广播 admin 站点新增事件
    crate::web_server::sse_handlers::push_admin_site_created(&site.site_id, &site.project_name);

    Ok(site)
}

fn build_preview_site(req: PreviewManagedSiteParsePlanRequest) -> Result<ManagedProjectSite> {
    let project_name = req.project_name.trim();
    if project_name.is_empty() {
        bail!("项目名不能为空");
    }
    let project_path = req.project_path.trim();
    if project_path.is_empty() {
        bail!("项目路径不能为空");
    }
    if req.web_port == 0 {
        bail!("站点端口不能为空");
    }

    let canonical_path = canonical_project_path(project_path)?;
    let parse_db_types = normalize_parse_db_types(req.parse_db_types);
    let force_rebuild_system_db =
        normalize_force_rebuild_system_db(req.force_rebuild_system_db, &parse_db_types);

    let mut site = if let Some(site_id) = req
        .site_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        get_site(site_id)?.ok_or_else(|| anyhow!("站点不存在: {}", site_id))?
    } else {
        let site_id = infer_site_id(project_name, req.web_port);
        let bind_host = normalize_host(req.bind_host.clone());
        let public_base_url = req
            .public_base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());
        let associated_project = req
            .associated_project
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());
        let (local_entry_url, public_entry_url, entry_url) =
            derive_entry_urls(req.web_port, &bind_host, &public_base_url);
        let generation_defaults = default_generation_config();

        ManagedProjectSite {
            site_id: site_id.clone(),
            project_name: project_name.to_string(),
            project_code: 0,
            project_path: canonical_path.to_string_lossy().to_string(),
            manual_db_nums: Vec::new(),
            parse_db_types: Vec::new(),
            force_rebuild_system_db: false,
            gen_model: generation_defaults.gen_model,
            gen_mesh: generation_defaults.gen_mesh,
            gen_spatial_tree: generation_defaults.gen_spatial_tree,
            apply_boolean_operation: generation_defaults.apply_boolean_operation,
            mesh_tol_ratio: generation_defaults.mesh_tol_ratio,
            export_json: generation_defaults.export_json,
            export_parquet: generation_defaults.export_parquet,
            config_path: config_path(&site_id).to_string_lossy().to_string(),
            runtime_dir: site_runtime_dir(&site_id).to_string_lossy().to_string(),
            db_data_path: db_data_path(&site_id).to_string_lossy().to_string(),
            db_port: 0,
            web_port: req.web_port,
            viewer_port: None,
            bind_host,
            public_base_url,
            associated_project,
            db_pid: None,
            web_pid: None,
            viewer_pid: None,
            viewer_url: None,
            parse_pid: None,
            status: ManagedSiteStatus::Draft,
            parse_status: ManagedSiteParseStatus::Pending,
            last_error: None,
            entry_url,
            local_entry_url,
            public_entry_url,
            last_parse_started_at: None,
            last_parse_finished_at: None,
            last_parse_duration_ms: None,
            parse_plan: ManagedSiteParsePlan::default(),
            risk_level: ManagedSiteRiskLevel::Normal,
            risk_reasons: Vec::new(),
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
        }
    };

    site.project_name = project_name.to_string();
    site.project_path = canonical_path.to_string_lossy().to_string();
    site.manual_db_nums = normalize_manual_db_nums(req.manual_db_nums);
    site.parse_db_types = parse_db_types;
    site.force_rebuild_system_db = force_rebuild_system_db;
    site.web_port = req.web_port;
    site.bind_host = normalize_host(req.bind_host);
    site.public_base_url = req
        .public_base_url
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    site.associated_project = req
        .associated_project
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let (local_entry_url, public_entry_url, entry_url) =
        derive_entry_urls(site.web_port, &site.bind_host, &site.public_base_url);
    site.entry_url = entry_url;
    site.local_entry_url = local_entry_url;
    site.public_entry_url = public_entry_url;
    site.parse_plan = ManagedSiteParsePlan::default();
    Ok(site)
}

pub fn preview_parse_plan(req: PreviewManagedSiteParsePlanRequest) -> Result<ManagedSiteParsePlan> {
    let site = build_preview_site(req)?;
    let included_db_files = resolve_included_db_files(&site)?;
    Ok(build_parse_plan_with_files(&site, included_db_files))
}

pub fn update_site(site_id: &str, req: UpdateManagedSiteRequest) -> Result<ManagedProjectSite> {
    let _guard = lock_op()?;

    let (mut site, stored_db_user, stored_db_password) = with_conn(|conn| {
        let site = load_site_with_conn(conn, site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
        let (u, p) = load_credentials_with_conn(conn, site_id)?;
        Ok((site, u, p))
    })?;

    if site.parse_status == ManagedSiteParseStatus::Running
        || site_has_active_processes(&site)
        || matches!(
            site.status,
            ManagedSiteStatus::Running | ManagedSiteStatus::Starting | ManagedSiteStatus::Stopping
        )
    {
        bail!("站点运行中，不能修改配置");
    }

    if let Some(value) = req.project_name.filter(|value| !value.trim().is_empty()) {
        site.project_name = value.trim().to_string();
    }
    if let Some(value) = req.project_path.filter(|value| !value.trim().is_empty()) {
        let canonical = canonical_project_path(value.trim())?;
        site.project_path = canonical.to_string_lossy().to_string();
    }
    if let Some(value) = req.project_code.filter(|value| *value > 0) {
        site.project_code = value;
    }
    if let Some(value) = req.manual_db_nums {
        site.manual_db_nums = normalize_manual_db_nums(value);
    }
    if let Some(value) = req.parse_db_types {
        site.parse_db_types = normalize_parse_db_types(value);
    }
    if let Some(value) = req.force_rebuild_system_db {
        site.force_rebuild_system_db = value;
    }
    if let Some(value) = req.gen_model {
        site.gen_model = value;
    }
    if let Some(value) = req.gen_mesh {
        site.gen_mesh = value;
    }
    if let Some(value) = req.gen_spatial_tree {
        site.gen_spatial_tree = value;
    }
    if let Some(value) = req.apply_boolean_operation {
        site.apply_boolean_operation = value;
    }
    if req.mesh_tol_ratio.is_some() {
        site.mesh_tol_ratio = normalize_mesh_tol_ratio(req.mesh_tol_ratio, site.mesh_tol_ratio);
    }
    if let Some(value) = req.export_json {
        site.export_json = value;
    }
    if let Some(value) = req.export_parquet {
        site.export_parquet = value;
    }
    if let Some(value) = req.bind_host.filter(|value| !value.trim().is_empty()) {
        let value = value.trim().to_string();
        assert_bind_host_safe(&value)?;
        site.bind_host = normalize_host(Some(value));
    }
    if let Some(value) = req.public_base_url {
        site.public_base_url = if value.trim().is_empty() {
            None
        } else {
            Some(value.trim().to_string())
        };
    }
    if let Some(value) = req.associated_project {
        site.associated_project = if value.trim().is_empty() {
            None
        } else {
            Some(value.trim().to_string())
        };
    }
    if let Some(value) = req.db_port {
        site.db_port = value;
    }
    if let Some(value) = req.web_port {
        site.web_port = value;
    }

    if matches!(req.db_user.as_ref(), Some(value) if value.trim().is_empty()) {
        bail!("数据库用户名不能为空");
    }
    if matches!(req.db_password.as_ref(), Some(value) if value.trim().is_empty()) {
        bail!("数据库密码不能为空");
    }
    let db_user = normalize_optional_db_user(req.db_user).unwrap_or(stored_db_user);
    let db_password = normalize_optional_db_password(req.db_password).unwrap_or(stored_db_password);
    assert_db_credentials_strong(&db_user, &db_password)?;
    site.force_rebuild_system_db =
        normalize_force_rebuild_system_db(site.force_rebuild_system_db, &site.parse_db_types);

    site.updated_at = now_rfc3339();
    let (local_entry_url, public_entry_url, entry_url) =
        derive_entry_urls(site.web_port, &site.bind_host, &site.public_base_url);
    site.entry_url = entry_url;
    site.local_entry_url = local_entry_url;
    site.public_entry_url = public_entry_url;
    site.status = ManagedSiteStatus::Draft;
    site.parse_status = ManagedSiteParseStatus::Pending;
    site.db_pid = None;
    site.web_pid = None;
    site.viewer_port = None;
    site.viewer_pid = None;
    site.viewer_url = None;
    site.parse_pid = None;
    site.last_error = None;

    with_tx(|conn| {
        assert_port_available_with_conn(conn, Some(site_id), site.db_port, site.web_port)?;
        persist_site_with_conn(conn, &site, &db_user, &db_password)?;
        Ok(())
    })?;

    write_site_files(&site, &db_user, &db_password)?;
    annotate_site_parse_plan(&mut site);

    // D1 / Sprint D · 修 G8：元数据更新成功后广播 admin 站点快照事件
    // （update_site 内部不走 update_runtime，所以单独注入）
    crate::web_server::sse_handlers::push_admin_site_snapshot(
        &site.site_id,
        Some(&site.project_name),
        status_to_str(&site.status),
        parse_status_to_str(&site.parse_status),
        site.last_error.as_deref(),
    );

    Ok(site)
}

#[derive(Default)]
pub struct RuntimeUpdate {
    pub status: Option<ManagedSiteStatus>,
    pub parse_status: Option<ManagedSiteParseStatus>,
    pub db_pid: Option<Option<u32>>,
    pub web_pid: Option<Option<u32>>,
    pub viewer_port: Option<Option<u16>>,
    pub viewer_pid: Option<Option<u32>>,
    pub viewer_url: Option<Option<String>>,
    pub parse_pid: Option<Option<u32>>,
    pub last_error: Option<Option<String>>,
    pub entry_url: Option<Option<String>>,
    pub last_parse_started_at: Option<Option<String>>,
    pub last_parse_finished_at: Option<Option<String>>,
    pub last_parse_duration_ms: Option<Option<u64>>,
}

pub fn update_runtime(site_id: &str, update: RuntimeUpdate) -> Result<()> {
    let RuntimeUpdate {
        status,
        parse_status,
        db_pid,
        web_pid,
        viewer_port,
        viewer_pid,
        viewer_url,
        parse_pid,
        last_error,
        entry_url,
        last_parse_started_at,
        last_parse_finished_at,
        last_parse_duration_ms,
    } = update;

    let updated_site = with_tx(|conn| {
        let mut site = load_site_with_conn(conn, site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
        if let Some(value) = status {
            site.status = value;
        }
        if let Some(value) = parse_status {
            site.parse_status = value;
        }
        if let Some(value) = db_pid {
            site.db_pid = value;
        }
        if let Some(value) = web_pid {
            site.web_pid = value;
        }
        if let Some(value) = viewer_port {
            site.viewer_port = value;
        }
        if let Some(value) = viewer_pid {
            site.viewer_pid = value;
        }
        if let Some(value) = viewer_url {
            site.viewer_url = value;
        }
        if let Some(value) = parse_pid {
            site.parse_pid = value;
        }
        if let Some(value) = last_error {
            site.last_error = value;
        }
        if let Some(value) = entry_url {
            site.entry_url = value;
        }
        if let Some(value) = last_parse_started_at {
            site.last_parse_started_at = value;
        }
        if let Some(value) = last_parse_finished_at {
            site.last_parse_finished_at = value;
        }
        if let Some(value) = last_parse_duration_ms {
            site.last_parse_duration_ms = value;
        }
        site.updated_at = now_rfc3339();
        let (db_user, db_password) = load_credentials_with_conn(conn, site_id)?;
        persist_site_with_conn(conn, &site, &db_user, &db_password)?;
        Ok(site)
    })?;

    // D1 / Sprint D · 修 G7/G8：事务 commit 成功后立即广播 admin 站点快照事件
    // 覆盖 start/stop/parse/restart 全路径（这些 action 都最终走 update_runtime）
    crate::web_server::sse_handlers::push_admin_site_snapshot(
        &updated_site.site_id,
        Some(&updated_site.project_name),
        status_to_str(&updated_site.status),
        parse_status_to_str(&updated_site.parse_status),
        updated_site.last_error.as_deref(),
    );

    Ok(())
}

fn record_site_error(
    site_id: &str,
    message: impl Into<String>,
    status: Option<ManagedSiteStatus>,
    parse_status: Option<ManagedSiteParseStatus>,
) {
    let message = message.into();
    if let Err(err) = update_runtime(
        site_id,
        RuntimeUpdate {
            status,
            parse_status,
            last_error: Some(Some(message.clone())),
            ..Default::default()
        },
    ) {
        tracing::warn!(site = %site_id, "记录站点错误失败 ({message}): {err}");
    }
}

// ─── Pure runtime state derivation ─────────────────────────────────────────

fn port_in_use(host: &str, port: u16) -> bool {
    let host = if host == "0.0.0.0" { "127.0.0.1" } else { host };
    let addr = format!("{}:{}", host, port);
    match addr.to_socket_addrs() {
        Ok(mut addrs) => addrs
            .any(|socket| TcpStream::connect_timeout(&socket, Duration::from_millis(300)).is_ok()),
        Err(_) => false,
    }
}

#[derive(Debug, Clone, Default)]
struct PortRuntimeProbe {
    managed_running: bool,
    conflict_pids: Vec<u32>,
}

fn probe_managed_port(port: u16, managed_pid: Option<u32>) -> PortRuntimeProbe {
    let port_pids = collect_port_pids_sync(port);
    let managed_pid = managed_pid.filter(|pid| pid_running(Some(*pid)));
    let managed_running = managed_pid
        .map(|pid| port_pids.iter().any(|port_pid| *port_pid == pid))
        .unwrap_or(false);
    let conflict_pids = port_pids
        .into_iter()
        .filter(|pid| !(managed_running && Some(*pid) == managed_pid))
        .collect();
    PortRuntimeProbe {
        managed_running,
        conflict_pids,
    }
}

/// 根据 pid / 端口等信号派生出当前运行时状态，不写库、不覆盖 `entry_url`。
fn derive_runtime_state(mut site: ManagedProjectSite) -> ManagedProjectSite {
    let db_running = probe_managed_port(site.db_port, site.db_pid).managed_running;
    let web_running = probe_managed_port(site.web_port, site.web_pid).managed_running;
    let parse_running = pid_running(site.parse_pid);

    if parse_running && site.parse_status != ManagedSiteParseStatus::Parsed {
        site.parse_status = ManagedSiteParseStatus::Running;
    }
    if web_running
        && !(matches!(site.status, ManagedSiteStatus::Starting) && site.viewer_url.is_none())
    {
        site.status = ManagedSiteStatus::Running;
    } else if matches!(
        site.status,
        ManagedSiteStatus::Running | ManagedSiteStatus::Starting
    ) {
        if db_running {
            site.status = ManagedSiteStatus::Starting;
        } else if site.parse_status == ManagedSiteParseStatus::Parsed {
            site.status = ManagedSiteStatus::Stopped;
        } else if site.parse_status == ManagedSiteParseStatus::Failed {
            site.status = ManagedSiteStatus::Failed;
        } else {
            site.status = ManagedSiteStatus::Draft;
        }
    }
    // entry_url 始终由 `derive_entry_urls` 生成，row_to_site 初始化时已经正确，此处不再覆盖。
    site
}

fn site_db_running(site: &ManagedProjectSite) -> bool {
    probe_managed_port(site.db_port, site.db_pid).managed_running
}

fn site_web_running(site: &ManagedProjectSite) -> bool {
    probe_managed_port(site.web_port, site.web_pid).managed_running
}

fn site_viewer_running(site: &ManagedProjectSite) -> bool {
    if pid_running(site.viewer_pid) {
        return true;
    }
    site.viewer_port
        .map(|port| {
            port_in_use("127.0.0.1", port)
                && site.viewer_url.is_some()
                && matches!(
                    site.status,
                    ManagedSiteStatus::Running | ManagedSiteStatus::Starting
                )
        })
        .unwrap_or(false)
}

fn site_parse_running(site: &ManagedProjectSite) -> bool {
    pid_running(site.parse_pid)
}

fn site_has_active_processes(site: &ManagedProjectSite) -> bool {
    site_db_running(site)
        || site_web_running(site)
        || pid_running(site.viewer_pid)
        || site_parse_running(site)
}

// ─── pid existence check ────────────────────────────────────────────────────

#[cfg(unix)]
fn pid_running(pid: Option<u32>) -> bool {
    let Some(pid) = pid else {
        return false;
    };
    // SAFETY: kill(pid, 0) 仅探测进程是否存在，不发送信号。
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(windows)]
fn pid_running(pid: Option<u32>) -> bool {
    let Some(pid) = pid else {
        return false;
    };
    let output = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {}", pid), "/NH", "/FO", "CSV"])
        .output();
    match output {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout);
            text.contains(&pid.to_string())
        }
        Err(_) => false,
    }
}

#[cfg(not(any(unix, windows)))]
fn pid_running(pid: Option<u32>) -> bool {
    let _ = pid;
    false
}

// ─── Resource sampler ───────────────────────────────────────────────────────

fn with_resource_sampler<R>(target_pids: &[u32], handler: impl FnOnce(bool, &System) -> R) -> R {
    let mut sampler = resource_sampler()
        .lock()
        .expect("resource sampler lock poisoned");
    let cpu_ready = sampler.warmed_up;
    sampler
        .system
        .refresh_memory_specifics(MemoryRefreshKind::everything());
    sampler
        .system
        .refresh_cpu_specifics(CpuRefreshKind::nothing().with_cpu_usage());
    let pids = target_pids
        .iter()
        .copied()
        .map(Pid::from_u32)
        .collect::<Vec<_>>();
    if pids.is_empty() {
        sampler.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_memory().with_cpu(),
        );
    } else {
        sampler.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            true,
            ProcessRefreshKind::nothing().with_memory().with_cpu(),
        );
    }
    sampler.warmed_up = true;
    handler(cpu_ready, &sampler.system)
}

// ─── Path size with TTL cache ───────────────────────────────────────────────

fn path_size_bytes_uncached(path: &Path) -> u64 {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return 0,
    };
    if metadata.is_file() {
        return metadata.len();
    }
    if !metadata.is_dir() {
        return 0;
    }
    fs::read_dir(path)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| path_size_bytes_uncached(&entry.path()))
        .sum()
}

fn path_size_bytes(path: &Path) -> u64 {
    let key = path.to_path_buf();
    if let Ok(cache) = path_size_cache().lock() {
        if let Some(entry) = cache.get(&key) {
            if entry.recorded_at.elapsed() < Duration::from_millis(PATH_SIZE_CACHE_TTL_MS) {
                return entry.value;
            }
        }
    }
    let value = path_size_bytes_uncached(path);
    if let Ok(mut cache) = path_size_cache().lock() {
        cache.insert(
            key,
            PathSizeCacheEntry {
                value,
                recorded_at: Instant::now(),
            },
        );
    }
    value
}

// ─── Disk usage and risk ────────────────────────────────────────────────────

fn site_data_dir(site: &ManagedProjectSite) -> PathBuf {
    let path = PathBuf::from(&site.db_data_path);
    if path.is_dir() {
        return path;
    }
    path.parent().map(Path::to_path_buf).unwrap_or(path)
}

fn disk_usage_for_path(path: &Path) -> Option<f32> {
    let disks = Disks::new_with_refreshed_list();
    let best_disk = disks.list().iter().fold(None, |best, disk| {
        if !path.starts_with(disk.mount_point()) {
            return best;
        }
        let depth = disk.mount_point().components().count();
        match best {
            Some((best_depth, usage)) if best_depth >= depth => Some((best_depth, usage)),
            _ => {
                let total = disk.total_space();
                let usage = if total == 0 {
                    0.0
                } else {
                    ((total.saturating_sub(disk.available_space())) as f32 / total as f32) * 100.0
                };
                Some((depth, usage))
            }
        }
    });
    best_disk.map(|(_, usage)| usage)
}

fn build_process_resource(
    pid: Option<u32>,
    running: bool,
    system: &System,
    cpu_ready: bool,
) -> ManagedSiteProcessResource {
    let mut resource = ManagedSiteProcessResource {
        pid,
        running,
        cpu_usage: None,
        memory_bytes: None,
    };
    let Some(pid_value) = pid else {
        return resource;
    };
    let Some(process) = system.process(Pid::from_u32(pid_value)) else {
        return resource;
    };
    resource.memory_bytes = Some(process.memory());
    if cpu_ready {
        resource.cpu_usage = Some(process.cpu_usage());
    }
    resource
}

fn risk_score(level: &ManagedSiteRiskLevel) -> u8 {
    match level {
        ManagedSiteRiskLevel::Normal => 0,
        ManagedSiteRiskLevel::Warning => 1,
        ManagedSiteRiskLevel::Critical => 2,
    }
}

fn promote_risk(level: &mut ManagedSiteRiskLevel, candidate: ManagedSiteRiskLevel) {
    if risk_score(&candidate) > risk_score(level) {
        *level = candidate;
    }
}

fn format_duration_label(duration_ms: u64) -> String {
    if duration_ms < 1_000 {
        return format!("{} ms", duration_ms);
    }
    let seconds = duration_ms / 1_000;
    if seconds < 60 {
        return format!("{} 秒", seconds);
    }
    let minutes = seconds / 60;
    let remain_seconds = seconds % 60;
    if minutes < 60 {
        return format!("{} 分 {} 秒", minutes, remain_seconds);
    }
    let hours = minutes / 60;
    let remain_minutes = minutes % 60;
    format!("{} 小时 {} 分", hours, remain_minutes)
}

fn evaluate_machine_risk(
    cpu_usage: Option<f32>,
    memory_usage: Option<f32>,
    disk_usage: Option<f32>,
) -> (ManagedSiteRiskLevel, Vec<String>) {
    let mut risk_level = ManagedSiteRiskLevel::Normal;
    let mut warnings = Vec::new();

    if let Some(value) = cpu_usage {
        if value >= MACHINE_CRITICAL_CPU {
            promote_risk(&mut risk_level, ManagedSiteRiskLevel::Critical);
            warnings.push("CPU 占用过高".to_string());
        } else if value >= MACHINE_WARNING_CPU {
            promote_risk(&mut risk_level, ManagedSiteRiskLevel::Warning);
            warnings.push("CPU 占用过高".to_string());
        }
    }

    if let Some(value) = memory_usage {
        if value >= MACHINE_CRITICAL_MEMORY {
            promote_risk(&mut risk_level, ManagedSiteRiskLevel::Critical);
            warnings.push("内存占用过高".to_string());
        } else if value >= MACHINE_WARNING_MEMORY {
            promote_risk(&mut risk_level, ManagedSiteRiskLevel::Warning);
            warnings.push("内存占用过高".to_string());
        }
    }

    if let Some(value) = disk_usage {
        if value >= MACHINE_CRITICAL_DISK {
            promote_risk(&mut risk_level, ManagedSiteRiskLevel::Critical);
            warnings.push("磁盘空间紧张".to_string());
        } else if value >= MACHINE_WARNING_DISK {
            promote_risk(&mut risk_level, ManagedSiteRiskLevel::Warning);
            warnings.push("磁盘空间紧张".to_string());
        }
    }

    (risk_level, warnings)
}

fn build_site_resource_metrics(
    site: &ManagedProjectSite,
    db_running: bool,
    web_running: bool,
    viewer_running: bool,
    parse_running: bool,
    system: &System,
    cpu_ready: bool,
) -> ManagedSiteResourceMetrics {
    let runtime_dir = PathBuf::from(&site.runtime_dir);
    let data_dir = site_data_dir(site);

    ManagedSiteResourceMetrics {
        db_process: build_process_resource(site.db_pid, db_running, system, cpu_ready),
        web_process: build_process_resource(site.web_pid, web_running, system, cpu_ready),
        viewer_process: build_process_resource(site.viewer_pid, viewer_running, system, cpu_ready),
        parse_process: build_process_resource(site.parse_pid, parse_running, system, cpu_ready),
        runtime_dir_size_bytes: path_size_bytes(&runtime_dir),
        data_dir_size_bytes: path_size_bytes(&data_dir),
        runtime_dir_missing: !runtime_dir.exists(),
        data_dir_missing: !data_dir.exists(),
        last_parse_started_at: site.last_parse_started_at.clone(),
        last_parse_finished_at: site.last_parse_finished_at.clone(),
        last_parse_duration_ms: site.last_parse_duration_ms,
    }
}

fn collect_site_resource_metrics(
    site: &ManagedProjectSite,
    db_running: bool,
    web_running: bool,
    viewer_running: bool,
    parse_running: bool,
) -> ManagedSiteResourceMetrics {
    let tracked_pids = [site.db_pid, site.web_pid, site.viewer_pid, site.parse_pid]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    with_resource_sampler(&tracked_pids, |cpu_ready, system| {
        build_site_resource_metrics(
            site,
            db_running,
            web_running,
            viewer_running,
            parse_running,
            system,
            cpu_ready,
        )
    })
}

fn evaluate_parse_health(
    site: &ManagedProjectSite,
    resources: &ManagedSiteResourceMetrics,
) -> ManagedSiteParseHealth {
    if site.parse_status == ManagedSiteParseStatus::Failed {
        return ManagedSiteParseHealth {
            status: ManagedSiteParseHealthStatus::Critical,
            label: "解析失败".to_string(),
            detail: site
                .last_error
                .clone()
                .or_else(|| Some("最近一次解析执行失败".to_string())),
        };
    }

    if site.parse_status == ManagedSiteParseStatus::Running {
        return ManagedSiteParseHealth {
            status: ManagedSiteParseHealthStatus::Unknown,
            label: "解析进行中".to_string(),
            detail: resources
                .last_parse_started_at
                .as_ref()
                .map(|value| format!("开始于 {}", value)),
        };
    }

    if let Some(duration_ms) = resources.last_parse_duration_ms {
        if duration_ms >= PARSE_CRITICAL_DURATION_MS {
            return ManagedSiteParseHealth {
                status: ManagedSiteParseHealthStatus::Critical,
                label: "解析耗时过长".to_string(),
                detail: Some(format!(
                    "最近一次解析耗时 {}",
                    format_duration_label(duration_ms)
                )),
            };
        }
        if duration_ms >= PARSE_WARNING_DURATION_MS {
            return ManagedSiteParseHealth {
                status: ManagedSiteParseHealthStatus::Warning,
                label: "解析耗时偏长".to_string(),
                detail: Some(format!(
                    "最近一次解析耗时 {}",
                    format_duration_label(duration_ms)
                )),
            };
        }
        return ManagedSiteParseHealth {
            status: ManagedSiteParseHealthStatus::Normal,
            label: "解析正常".to_string(),
            detail: Some(format!(
                "最近一次解析耗时 {}",
                format_duration_label(duration_ms)
            )),
        };
    }

    if site.parse_status == ManagedSiteParseStatus::Pending {
        return ManagedSiteParseHealth {
            status: ManagedSiteParseHealthStatus::Unknown,
            label: "暂无解析记录".to_string(),
            detail: None,
        };
    }

    if site.parse_status == ManagedSiteParseStatus::Parsed {
        return ManagedSiteParseHealth {
            status: ManagedSiteParseHealthStatus::Normal,
            label: "解析正常".to_string(),
            detail: None,
        };
    }

    ManagedSiteParseHealth {
        status: ManagedSiteParseHealthStatus::Unknown,
        label: "暂无解析记录".to_string(),
        detail: None,
    }
}

fn apply_process_risk(
    label: &str,
    process: &ManagedSiteProcessResource,
    risk_level: &mut ManagedSiteRiskLevel,
    warnings: &mut Vec<String>,
) {
    if !process.running {
        return;
    }

    if let Some(cpu_usage) = process.cpu_usage {
        if cpu_usage >= PROCESS_CRITICAL_CPU {
            promote_risk(risk_level, ManagedSiteRiskLevel::Critical);
            warnings.push(format!("{} 进程 CPU 占用过高", label));
        } else if cpu_usage >= PROCESS_WARNING_CPU {
            promote_risk(risk_level, ManagedSiteRiskLevel::Warning);
            warnings.push(format!("{} 进程 CPU 占用过高", label));
        }
    }

    if let Some(memory_bytes) = process.memory_bytes {
        if memory_bytes >= PROCESS_CRITICAL_MEMORY_BYTES {
            promote_risk(risk_level, ManagedSiteRiskLevel::Critical);
            warnings.push(format!("{} 进程内存占用过高", label));
        } else if memory_bytes >= PROCESS_WARNING_MEMORY_BYTES {
            promote_risk(risk_level, ManagedSiteRiskLevel::Warning);
            warnings.push(format!("{} 进程内存占用过高", label));
        }
    }
}

fn evaluate_site_risk(
    site: &ManagedProjectSite,
    resources: &ManagedSiteResourceMetrics,
) -> (ManagedSiteRiskLevel, Vec<String>, ManagedSiteParseHealth) {
    let mut risk_level = ManagedSiteRiskLevel::Normal;
    let mut warnings = Vec::new();

    if site.status == ManagedSiteStatus::Failed {
        promote_risk(&mut risk_level, ManagedSiteRiskLevel::Critical);
        warnings.push("站点当前状态失败".to_string());
    }

    apply_process_risk("DB", &resources.db_process, &mut risk_level, &mut warnings);
    apply_process_risk(
        "Web",
        &resources.web_process,
        &mut risk_level,
        &mut warnings,
    );
    apply_process_risk(
        "Viewer",
        &resources.viewer_process,
        &mut risk_level,
        &mut warnings,
    );
    apply_process_risk(
        "Parse",
        &resources.parse_process,
        &mut risk_level,
        &mut warnings,
    );

    if site.parse_status == ManagedSiteParseStatus::Failed {
        promote_risk(&mut risk_level, ManagedSiteRiskLevel::Critical);
        warnings.push("Parse 最近一次执行失败".to_string());
    } else if let Some(duration_ms) = resources.last_parse_duration_ms {
        if duration_ms >= PARSE_CRITICAL_DURATION_MS {
            promote_risk(&mut risk_level, ManagedSiteRiskLevel::Critical);
            warnings.push("Parse 最近耗时过长".to_string());
        } else if duration_ms >= PARSE_WARNING_DURATION_MS {
            promote_risk(&mut risk_level, ManagedSiteRiskLevel::Warning);
            warnings.push("Parse 最近耗时过长".to_string());
        }
    }

    if resources.runtime_dir_missing
        && matches!(
            site.status,
            ManagedSiteStatus::Starting | ManagedSiteStatus::Running
        )
    {
        promote_risk(&mut risk_level, ManagedSiteRiskLevel::Warning);
        warnings.push("运行目录缺失".to_string());
    }

    if resources.data_dir_missing
        && matches!(
            site.status,
            ManagedSiteStatus::Starting | ManagedSiteStatus::Running | ManagedSiteStatus::Parsed
        )
    {
        promote_risk(&mut risk_level, ManagedSiteRiskLevel::Warning);
        warnings.push("数据目录缺失".to_string());
    }

    let parse_health = evaluate_parse_health(site, resources);
    (risk_level, warnings, parse_health)
}

fn annotate_site_risk(site: &mut ManagedProjectSite) {
    let db_running = pid_running(site.db_pid) || port_in_use("127.0.0.1", site.db_port);
    let web_running = pid_running(site.web_pid) || port_in_use("127.0.0.1", site.web_port);
    let viewer_running = site_viewer_running(site);
    let parse_running = pid_running(site.parse_pid);
    let resources =
        collect_site_resource_metrics(site, db_running, web_running, viewer_running, parse_running);
    let (risk_level, risk_reasons, _) = evaluate_site_risk(site, &resources);
    site.risk_level = risk_level;
    site.risk_reasons = risk_reasons;
}

fn annotate_sites_risks(sites: &mut [ManagedProjectSite]) {
    let runtime_states = sites
        .iter()
        .map(|site| {
            (
                pid_running(site.db_pid) || port_in_use("127.0.0.1", site.db_port),
                pid_running(site.web_pid) || port_in_use("127.0.0.1", site.web_port),
                site_viewer_running(site),
                pid_running(site.parse_pid),
            )
        })
        .collect::<Vec<_>>();
    let tracked_pids = sites
        .iter()
        .flat_map(|site| [site.db_pid, site.web_pid, site.viewer_pid, site.parse_pid])
        .flatten()
        .collect::<Vec<_>>();

    with_resource_sampler(&tracked_pids, |cpu_ready, system| {
        for (site, (db_running, web_running, viewer_running, parse_running)) in
            sites.iter_mut().zip(runtime_states.into_iter())
        {
            let resources = build_site_resource_metrics(
                site,
                db_running,
                web_running,
                viewer_running,
                parse_running,
                system,
                cpu_ready,
            );
            let (risk_level, risk_reasons, _) = evaluate_site_risk(site, &resources);
            site.risk_level = risk_level;
            site.risk_reasons = risk_reasons;
        }
    });
}

pub fn resource_summary() -> Result<AdminResourceSummary> {
    let sites = with_conn(|conn| {
        let mut stmt = conn.prepare(&format!(
            "SELECT * FROM {table} ORDER BY updated_at DESC",
            table = TABLE_NAME
        ))?;
        let rows = stmt.query_map([], row_to_site)?;
        let mut collected = Vec::new();
        for row in rows {
            collected.push(row?);
        }
        Ok(collected)
    })?;

    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let admin_runtime_root = runtime_root();
    let managed_data_size_bytes = sites
        .iter()
        .map(|site| path_size_bytes(Path::new(&site.db_data_path)))
        .sum();

    Ok(with_resource_sampler(&[], |cpu_ready, system| {
        let cpu_usage = cpu_ready.then_some(system.global_cpu_usage());
        let memory_usage = {
            let total = system.total_memory();
            if total == 0 {
                None
            } else {
                Some((system.used_memory() as f32 / total as f32) * 100.0)
            }
        };
        let disk_usage = disk_usage_for_path(&current_dir);
        let (risk_level, warnings) = evaluate_machine_risk(cpu_usage, memory_usage, disk_usage);
        AdminResourceSummary {
            cpu_usage,
            memory_usage,
            disk_usage,
            admin_runtime_size_bytes: path_size_bytes(&admin_runtime_root),
            managed_data_size_bytes,
            risk_level,
            warnings,
            updated_at: now_rfc3339(),
            message: None,
        }
    }))
}

fn preflight_check(
    key: &str,
    label: &str,
    status: ManagedSitePreflightStatus,
    message: impl Into<String>,
    detail: Option<String>,
    action_hint: Option<String>,
    pids: Vec<u32>,
) -> ManagedSitePreflightCheck {
    ManagedSitePreflightCheck {
        key: key.to_string(),
        label: label.to_string(),
        status,
        message: message.into(),
        detail,
        action_hint,
        pids,
    }
}

fn preflight_pass(
    key: &str,
    label: &str,
    message: impl Into<String>,
    detail: Option<String>,
) -> ManagedSitePreflightCheck {
    preflight_check(
        key,
        label,
        ManagedSitePreflightStatus::Pass,
        message,
        detail,
        None,
        Vec::new(),
    )
}

fn preflight_warning(
    key: &str,
    label: &str,
    message: impl Into<String>,
    detail: Option<String>,
    action_hint: Option<String>,
    pids: Vec<u32>,
) -> ManagedSitePreflightCheck {
    preflight_check(
        key,
        label,
        ManagedSitePreflightStatus::Warning,
        message,
        detail,
        action_hint,
        pids,
    )
}

fn preflight_blocking(
    key: &str,
    label: &str,
    message: impl Into<String>,
    detail: Option<String>,
    action_hint: Option<String>,
    pids: Vec<u32>,
) -> ManagedSitePreflightCheck {
    preflight_check(
        key,
        label,
        ManagedSitePreflightStatus::Blocking,
        message,
        detail,
        action_hint,
        pids,
    )
}

fn preflight_port_check(
    site: &ManagedProjectSite,
    key: &str,
    label: &str,
    port: u16,
    managed_pid: Option<u32>,
) -> ManagedSitePreflightCheck {
    let pids = collect_port_pids_sync(port);
    if pids.is_empty() {
        return preflight_pass(key, label, format!("{label}端口 {port} 可用"), None);
    }
    if let Some(pid) = managed_pid.filter(|pid| pid_running(Some(*pid))) {
        if pids.iter().any(|value| *value == pid) {
            return preflight_warning(
                key,
                label,
                format!("{label}端口 {port} 正由当前站点的已记录进程占用"),
                Some(format!("site_id={} pid={pid}", site.site_id)),
                Some("如需完整重新部署，请先停止站点再部署".to_string()),
                pids,
            );
        }
    }
    preflight_blocking(
        key,
        label,
        format!("{label}端口 {port} 已被其他进程占用"),
        Some(format!("PIDs: {:?}", pids)),
        Some("更换端口，或停止占用该端口的外部进程".to_string()),
        pids,
    )
}

fn preflight_aios_database() -> ManagedSitePreflightCheck {
    let repo = match repo_root() {
        Ok(repo) => repo,
        Err(err) => {
            return preflight_blocking(
                "aios_database",
                "aios-database",
                "无法定位仓库根目录",
                Some(err.to_string()),
                Some("从 plant-model-gen 仓库根目录启动 admin web_server".to_string()),
                Vec::new(),
            );
        }
    };
    match aios_database_binary() {
        Ok(Some(path)) => preflight_pass(
            "aios_database",
            "aios-database",
            "已找到 aios-database 可执行文件",
            Some(path.display().to_string()),
        ),
        Ok(None) if should_run_aios_database_from_source(&repo) => preflight_warning(
            "aios_database",
            "aios-database",
            "未找到二进制，将回退到 cargo run --bin aios-database",
            Some(repo.display().to_string()),
            Some("生产环境建议配置 ADMIN_AIOS_DATABASE_BINARY 指向已编译二进制".to_string()),
            Vec::new(),
        ),
        Ok(None) => preflight_blocking(
            "aios_database",
            "aios-database",
            "未找到 aios-database 二进制",
            None,
            Some(
                "配置 ADMIN_AIOS_DATABASE_BINARY，或设置 ADMIN_ALLOW_CARGO_RUN=1 仅用于本地开发"
                    .to_string(),
            ),
            Vec::new(),
        ),
        Err(err) => preflight_blocking(
            "aios_database",
            "aios-database",
            "aios-database 配置不可用",
            Some(err.to_string()),
            Some("修正 admin_aios_database_binary / ADMIN_AIOS_DATABASE_BINARY".to_string()),
            Vec::new(),
        ),
    }
}

fn preflight_surreal() -> ManagedSitePreflightCheck {
    match std::process::Command::new("surreal")
        .arg("--version")
        .output()
    {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            preflight_pass(
                "surreal",
                "SurrealDB",
                "surreal 命令可用",
                (!version.is_empty()).then_some(version),
            )
        }
        Ok(output) => preflight_blocking(
            "surreal",
            "SurrealDB",
            "surreal 命令执行失败",
            Some(String::from_utf8_lossy(&output.stderr).trim().to_string()),
            Some("确认 surreal 已安装并在 PATH 中".to_string()),
            Vec::new(),
        ),
        Err(err) => preflight_blocking(
            "surreal",
            "SurrealDB",
            "未找到 surreal 命令",
            Some(err.to_string()),
            Some("安装 SurrealDB CLI，或把 surreal 所在目录加入 PATH".to_string()),
            Vec::new(),
        ),
    }
}

fn preflight_project_path(site: &ManagedProjectSite) -> ManagedSitePreflightCheck {
    let source_project_name = site_source_project_name(site);
    let candidates = project_dir_candidates(&source_project_name, &site.project_path);
    if let Some(path) = candidates.into_iter().find(|path| path.exists()) {
        preflight_pass(
            "project_path",
            "项目路径",
            "项目路径可访问",
            Some(path.display().to_string()),
        )
    } else {
        preflight_blocking(
            "project_path",
            "项目路径",
            "项目路径不存在或不可访问",
            Some(site.project_path.clone()),
            Some("检查 project_path 或 associated_project 是否指向真实 AVEVA 项目目录".to_string()),
            Vec::new(),
        )
    }
}

fn preflight_parse_scope(site: &ManagedProjectSite) -> ManagedSitePreflightCheck {
    match resolve_included_db_files(site) {
        Ok(files) if files.is_empty() && parse_scope_enabled(site) => preflight_warning(
            "parse_scope",
            "解析文件",
            "未解析出明确 included_db_files，将按 DbOption 规则执行",
            Some(format!(
                "manual_db_nums={:?}, parse_db_types={:?}",
                site.manual_db_nums, site.parse_db_types
            )),
            Some("确认项目目录下存在目标 DB 文件，或收紧解析范围".to_string()),
            Vec::new(),
        ),
        Ok(files) if files.is_empty() => preflight_warning(
            "parse_scope",
            "解析文件",
            "未限制解析范围，可能触发全量解析",
            None,
            Some("建议选择 dbnum 或解析类型，避免误触发大范围解析".to_string()),
            Vec::new(),
        ),
        Ok(files) => preflight_pass(
            "parse_scope",
            "解析文件",
            format!("已定位 {} 个待解析 DB 文件", files.len()),
            Some(files.join(", ")),
        ),
        Err(err) => preflight_blocking(
            "parse_scope",
            "解析文件",
            "解析文件检查失败",
            Some(err.to_string()),
            Some("检查 project_path、manual_db_nums 和 parse_db_types 配置".to_string()),
            Vec::new(),
        ),
    }
}

async fn preflight_viewer(site: &ManagedProjectSite) -> ManagedSitePreflightCheck {
    if !managed_viewer_enabled() {
        return preflight_warning(
            "viewer",
            "plant3d-web",
            "受管 Viewer 启动已禁用",
            Some("AIOS_MANAGED_VIEWER_ENABLED=0".to_string()),
            Some(
                "如需一键打开三维 Viewer，请启用受管 Viewer 或配置 AIOS_VIEWER_BASE_URL"
                    .to_string(),
            ),
            Vec::new(),
        );
    }

    let viewer_dir = match viewer_project_dir() {
        Ok(Some(path)) => path,
        Ok(None) => {
            return preflight_warning(
                "viewer",
                "plant3d-web",
                "未找到 plant3d-web 目录，部署将只启动后端站点",
                None,
                Some(
                    "设置 AIOS_VIEWER_PROJECT_DIR，或将 plant3d-web 放在 plant-model-gen 同级目录"
                        .to_string(),
                ),
                Vec::new(),
            );
        }
        Err(err) => {
            return preflight_blocking(
                "viewer",
                "plant3d-web",
                "Viewer 配置不可用",
                Some(err.to_string()),
                Some("修正 AIOS_VIEWER_PROJECT_DIR".to_string()),
                Vec::new(),
            );
        }
    };

    if !viewer_dir.join("package.json").exists() {
        return preflight_blocking(
            "viewer",
            "plant3d-web",
            "plant3d-web 缺少 package.json",
            Some(viewer_dir.display().to_string()),
            Some("确认 AIOS_VIEWER_PROJECT_DIR 指向 plant3d-web 项目根目录".to_string()),
            Vec::new(),
        );
    }

    let (port, reuse_existing) = match choose_viewer_port(site).await {
        Ok(result) => result,
        Err(err) => {
            return preflight_blocking(
                "viewer",
                "plant3d-web",
                "未找到可用 Viewer 端口",
                Some(err.to_string()),
                Some("释放 3101..3120 端口，或设置 AIOS_VIEWER_PORT 指向可用端口".to_string()),
                Vec::new(),
            );
        }
    };
    let pids = collect_port_pids_sync(port);
    if reuse_existing {
        return preflight_warning(
            "viewer",
            "plant3d-web",
            format!("将复用已运行的 plant3d-web Viewer 端口 {port}"),
            Some(viewer_dir.display().to_string()),
            Some("停止站点不会关闭被复用的外部 Viewer 进程".to_string()),
            pids,
        );
    }
    if !viewer_dir.join("node_modules").exists() {
        return preflight_warning(
            "viewer",
            "plant3d-web",
            format!("Viewer 端口 {port} 可用，但 node_modules 不存在"),
            Some(viewer_dir.display().to_string()),
            Some("先在 plant3d-web 执行 npm install，再进行一键部署".to_string()),
            Vec::new(),
        );
    }
    preflight_pass(
        "viewer",
        "plant3d-web",
        format!("Viewer 目录和端口 {port} 可用"),
        Some(viewer_dir.display().to_string()),
    )
}

fn preflight_machine_resources() -> ManagedSitePreflightCheck {
    match resource_summary() {
        Ok(summary) if summary.risk_level == ManagedSiteRiskLevel::Critical => preflight_blocking(
            "machine_resources",
            "机器资源",
            "当前机器资源处于严重风险状态",
            Some(summary.warnings.join("; ")),
            Some("释放 CPU/内存/磁盘后再部署".to_string()),
            Vec::new(),
        ),
        Ok(summary) if summary.risk_level == ManagedSiteRiskLevel::Warning => preflight_warning(
            "machine_resources",
            "机器资源",
            "当前机器资源存在告警",
            Some(summary.warnings.join("; ")),
            Some("资源紧张时部署可能变慢或失败".to_string()),
            Vec::new(),
        ),
        Ok(summary) => preflight_pass(
            "machine_resources",
            "机器资源",
            "机器资源检查通过",
            Some(format!(
                "cpu={:?} memory={:?} disk={:?}",
                summary.cpu_usage, summary.memory_usage, summary.disk_usage
            )),
        ),
        Err(err) => preflight_warning(
            "machine_resources",
            "机器资源",
            "机器资源检查失败，继续部署但风险未知",
            Some(err.to_string()),
            Some("检查运行目录权限和磁盘状态".to_string()),
            Vec::new(),
        ),
    }
}

pub async fn preflight_site(site_id: &str) -> Result<ManagedSitePreflightReport> {
    let site = get_site(site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
    let mut checks = Vec::new();

    if site.parse_status == ManagedSiteParseStatus::Running {
        checks.push(preflight_blocking(
            "site_state",
            "站点状态",
            "解析任务正在运行，暂不能部署",
            None,
            Some("等待当前解析结束，或先停止站点".to_string()),
            Vec::new(),
        ));
    } else if matches!(
        site.status,
        ManagedSiteStatus::Running | ManagedSiteStatus::Starting | ManagedSiteStatus::Stopping
    ) {
        checks.push(preflight_blocking(
            "site_state",
            "站点状态",
            format!("当前状态为 {:?}，暂不能完整部署", site.status),
            None,
            Some("先停止站点，待状态稳定后重新部署".to_string()),
            Vec::new(),
        ));
    } else {
        checks.push(preflight_pass(
            "site_state",
            "站点状态",
            format!("当前状态 {:?} 可提交部署", site.status),
            None,
        ));
    }

    checks.push(preflight_project_path(&site));
    checks.push(preflight_parse_scope(&site));
    checks.push(preflight_aios_database());
    checks.push(preflight_surreal());
    checks.push(preflight_port_check(
        &site,
        "db_port",
        "数据库",
        site.db_port,
        site.db_pid,
    ));
    checks.push(preflight_port_check(
        &site,
        "web_port",
        "Web",
        site.web_port,
        site.web_pid,
    ));
    checks.push(preflight_viewer(&site).await);
    checks.push(preflight_machine_resources());

    let blocking_count = checks
        .iter()
        .filter(|check| check.status == ManagedSitePreflightStatus::Blocking)
        .count();
    let warning_count = checks
        .iter()
        .filter(|check| check.status == ManagedSitePreflightStatus::Warning)
        .count();

    Ok(ManagedSitePreflightReport {
        site_id: site.site_id,
        ready: blocking_count == 0,
        blocking_count,
        warning_count,
        updated_at: now_rfc3339(),
        checks,
    })
}

fn preflight_blocking_summary(report: &ManagedSitePreflightReport) -> String {
    report
        .checks
        .iter()
        .filter(|check| check.status == ManagedSitePreflightStatus::Blocking)
        .map(|check| format!("{}: {}", check.label, check.message))
        .collect::<Vec<_>>()
        .join("; ")
}

// ─── Process spawn helpers ──────────────────────────────────────────────────

fn repo_root() -> Result<PathBuf> {
    std::env::current_dir().context("获取当前工作目录失败")
}

fn current_exe_path() -> Result<PathBuf> {
    std::env::current_exe().context("获取当前 web_server 可执行文件失败")
}

fn aios_database_exe_name() -> &'static str {
    if cfg!(windows) {
        "aios-database.exe"
    } else {
        "aios-database"
    }
}

fn aios_database_binary() -> Result<Option<PathBuf>> {
    if let Some(override_path) = admin_aios_database_binary_override() {
        if override_path.exists() {
            return Ok(Some(override_path));
        }
        bail!(
            "admin_aios_database_binary 指向的文件不存在: {}",
            override_path.display()
        );
    }
    let current = current_exe_path()?;
    let parent = current
        .parent()
        .ok_or_else(|| anyhow!("无法定位当前二进制目录"))?;
    let mut candidates = vec![parent.join(aios_database_exe_name())];
    if let Ok(repo) = repo_root() {
        candidates.push(
            repo.join("target")
                .join("debug")
                .join(aios_database_exe_name()),
        );
        candidates.push(
            repo.join("target")
                .join("release")
                .join(aios_database_exe_name()),
        );
    }
    Ok(candidates.into_iter().find(|candidate| candidate.exists()))
}

fn should_run_aios_database_from_source(repo: &Path) -> bool {
    if !admin_allow_cargo_fallback() {
        return false;
    }
    let current = match current_exe_path() {
        Ok(path) => path,
        Err(_) => return false,
    };
    repo.join("Cargo.toml").exists()
        && current
            .components()
            .any(|component| component.as_os_str() == "target")
}

fn config_path_without_toml(path: &Path) -> String {
    path.to_string_lossy()
        .to_string()
        .strip_suffix(".toml")
        .map(|value| value.to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

fn config_string_without_toml(path: &str) -> String {
    path.strip_suffix(".toml")
        .map(|value| value.to_string())
        .unwrap_or_else(|| path.to_string())
}

fn aios_database_command(repo: &Path, config_no_ext: &str) -> Result<Command> {
    if let Some(binary) = aios_database_binary()? {
        let mut cmd = Command::new(binary);
        cmd.arg("-c").arg(config_no_ext);
        Ok(cmd)
    } else if should_run_aios_database_from_source(repo) {
        let mut cmd = Command::new("cargo");
        cmd.arg("run")
            .arg("--bin")
            .arg("aios-database")
            .arg("--")
            .arg("-c")
            .arg(config_no_ext);
        Ok(cmd)
    } else {
        bail!(
            "未找到 aios-database 二进制（请配置 admin_aios_database_binary 或设置 ADMIN_ALLOW_CARGO_RUN=1）"
        );
    }
}

fn open_log_file(path: &Path) -> Result<(std::fs::File, std::fs::File)> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    let stdout = OpenOptions::new().create(true).append(true).open(path)?;
    let stderr = OpenOptions::new().create(true).append(true).open(path)?;
    Ok((stdout, stderr))
}

/// 把 `tokio::process::Command` 放进一个新的进程组；停止时可以按组杀。
fn isolate_process_group(command: &mut Command) {
    #[cfg(unix)]
    {
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NEW_PROCESS_GROUP = 0x00000200
        command.creation_flags(0x00000200);
    }
}

// ─── Wait helpers ───────────────────────────────────────────────────────────

async fn wait_for_port(port: u16, attempts: usize, delay_ms: u64) -> bool {
    for _ in 0..attempts {
        if port_in_use("127.0.0.1", port) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
    false
}

async fn wait_for_http_ok(url: &str, attempts: usize, delay_ms: u64) -> bool {
    let client = match reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };
    for _ in 0..attempts {
        if let Ok(response) = client.get(url).send().await {
            if response.status().is_success() {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
    false
}

// ─── Deploy readiness validation ────────────────────────────────────────────

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct DeployValidationReport {
    site_id: String,
    checked_at: String,
    blocking_count: usize,
    warning_count: usize,
    checks: Vec<DeployValidationCheck>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct DeployValidationCheck {
    key: String,
    label: String,
    status: String,
    message: String,
    detail: Option<String>,
    url: Option<String>,
    bytes: Option<u64>,
}

impl DeployValidationReport {
    fn new(site_id: &str) -> Self {
        Self {
            site_id: site_id.to_string(),
            checked_at: now_rfc3339(),
            blocking_count: 0,
            warning_count: 0,
            checks: Vec::new(),
        }
    }

    fn push(&mut self, check: DeployValidationCheck) {
        match check.status.as_str() {
            "blocking" => self.blocking_count += 1,
            "warning" => self.warning_count += 1,
            _ => {}
        }
        self.checks.push(check);
    }
}

fn deploy_validation_check(
    key: impl Into<String>,
    label: impl Into<String>,
    status: &'static str,
    message: impl Into<String>,
    detail: Option<String>,
    url: Option<String>,
    bytes: Option<u64>,
) -> DeployValidationCheck {
    DeployValidationCheck {
        key: key.into(),
        label: label.into(),
        status: status.to_string(),
        message: message.into(),
        detail,
        url,
        bytes,
    }
}

fn deploy_validation_report_path(site_id: &str) -> PathBuf {
    site_runtime_dir(site_id).join("deploy-validation.json")
}

pub fn deploy_validation_report(site_id: &str) -> Result<ManagedSiteDeployValidationReport> {
    let _ = get_site(site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
    let path = deploy_validation_report_path(site_id);
    if !path.exists() {
        return Ok(ManagedSiteDeployValidationReport {
            site_id: site_id.to_string(),
            exists: false,
            ..Default::default()
        });
    }

    let raw = fs::read_to_string(&path)
        .with_context(|| format!("读取部署验收报告失败: {}", path.display()))?;
    let report: DeployValidationReport = serde_json::from_str(&raw)
        .with_context(|| format!("解析部署验收报告失败: {}", path.display()))?;
    Ok(ManagedSiteDeployValidationReport {
        site_id: report.site_id,
        exists: true,
        checked_at: Some(report.checked_at),
        blocking_count: report.blocking_count,
        warning_count: report.warning_count,
        checks: report
            .checks
            .into_iter()
            .map(|check| ManagedSiteDeployValidationCheck {
                key: check.key,
                label: check.label,
                status: check.status,
                message: check.message,
                detail: check.detail,
                url: check.url,
                bytes: check.bytes,
            })
            .collect(),
    })
}

fn write_deploy_validation_report(report: &DeployValidationReport) -> Result<()> {
    let path = deploy_validation_report_path(&report.site_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建部署验收报告目录失败: {}", parent.display()))?;
    }
    let content = serde_json::to_string_pretty(report)?;
    fs::write(&path, content).with_context(|| format!("写入部署验收报告失败: {}", path.display()))
}

fn deploy_validation_blocking_summary(report: &DeployValidationReport) -> String {
    report
        .checks
        .iter()
        .filter(|check| check.status == "blocking")
        .map(|check| format!("{}: {}", check.label, check.message))
        .collect::<Vec<_>>()
        .join("; ")
}

fn readable_file_size(path: &Path) -> Result<u64> {
    let meta = fs::metadata(path).with_context(|| format!("文件不存在: {}", path.display()))?;
    if !meta.is_file() {
        bail!("不是普通文件: {}", path.display());
    }
    Ok(meta.len())
}

fn push_required_file_check(
    report: &mut DeployValidationReport,
    key: impl Into<String>,
    label: impl Into<String>,
    path: &Path,
    min_bytes: u64,
) {
    let key = key.into();
    let label = label.into();
    match readable_file_size(path) {
        Ok(bytes) if bytes >= min_bytes => report.push(deploy_validation_check(
            key,
            label,
            "pass",
            format!("文件存在且大小 {bytes} bytes"),
            Some(path.display().to_string()),
            None,
            Some(bytes),
        )),
        Ok(bytes) => report.push(deploy_validation_check(
            key,
            label,
            "blocking",
            format!("文件过小: {bytes} bytes，期望至少 {min_bytes} bytes"),
            Some(path.display().to_string()),
            None,
            Some(bytes),
        )),
        Err(err) => report.push(deploy_validation_check(
            key,
            label,
            "blocking",
            err.to_string(),
            Some(path.display().to_string()),
            None,
            None,
        )),
    }
}

fn push_required_json_file_check(
    report: &mut DeployValidationReport,
    key: impl Into<String>,
    label: impl Into<String>,
    path: &Path,
    min_bytes: u64,
) {
    let key = key.into();
    let label = label.into();
    match readable_file_size(path) {
        Ok(bytes) if bytes >= min_bytes => match fs::read_to_string(path)
            .with_context(|| format!("读取 JSON 文件失败: {}", path.display()))
            .and_then(|raw| {
                serde_json::from_str::<serde_json::Value>(&raw)
                    .with_context(|| format!("JSON 解析失败: {}", path.display()))
            }) {
            Ok(_) => report.push(deploy_validation_check(
                key,
                label,
                "pass",
                format!("JSON 文件存在且可解析，大小 {bytes} bytes"),
                Some(path.display().to_string()),
                None,
                Some(bytes),
            )),
            Err(err) => report.push(deploy_validation_check(
                key,
                label,
                "blocking",
                err.to_string(),
                Some(path.display().to_string()),
                None,
                Some(bytes),
            )),
        },
        Ok(bytes) => report.push(deploy_validation_check(
            key,
            label,
            "blocking",
            format!("JSON 文件过小: {bytes} bytes，期望至少 {min_bytes} bytes"),
            Some(path.display().to_string()),
            None,
            Some(bytes),
        )),
        Err(err) => report.push(deploy_validation_check(
            key,
            label,
            "blocking",
            err.to_string(),
            Some(path.display().to_string()),
            None,
            None,
        )),
    }
}

async fn push_required_http_check(
    report: &mut DeployValidationReport,
    client: &reqwest::Client,
    key: impl Into<String>,
    label: impl Into<String>,
    url: String,
    min_bytes: u64,
) {
    let key = key.into();
    let label = label.into();
    match client.get(&url).send().await {
        Ok(response) if response.status().is_success() => {
            let status = response.status();
            match response.bytes().await {
                Ok(bytes) if bytes.len() as u64 >= min_bytes => {
                    report.push(deploy_validation_check(
                        key,
                        label,
                        "pass",
                        format!("HTTP {status}，返回 {} bytes", bytes.len()),
                        None,
                        Some(url),
                        Some(bytes.len() as u64),
                    ));
                }
                Ok(bytes) => report.push(deploy_validation_check(
                    key,
                    label,
                    "blocking",
                    format!(
                        "HTTP {status} 但响应过小: {} bytes，期望至少 {min_bytes} bytes",
                        bytes.len()
                    ),
                    None,
                    Some(url),
                    Some(bytes.len() as u64),
                )),
                Err(err) => report.push(deploy_validation_check(
                    key,
                    label,
                    "blocking",
                    format!("读取 HTTP 响应失败: {err}"),
                    None,
                    Some(url),
                    None,
                )),
            }
        }
        Ok(response) => report.push(deploy_validation_check(
            key,
            label,
            "blocking",
            format!("HTTP 状态异常: {}", response.status()),
            None,
            Some(url),
            None,
        )),
        Err(err) => report.push(deploy_validation_check(
            key,
            label,
            "blocking",
            format!("HTTP 请求失败: {err}"),
            None,
            Some(url),
            None,
        )),
    }
}

fn mesh_serve_root_from_current_config() -> PathBuf {
    let path = aios_core::get_db_option().get_meshes_path();
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("lod_"))
    {
        path.parent().unwrap_or(&path).to_path_buf()
    } else {
        path
    }
}

fn find_first_glb(root: &Path) -> Option<PathBuf> {
    const MAX_DEPTH: usize = 4;
    const MAX_FILES: usize = 20_000;

    fn visit(path: &Path, depth: usize, visited: &mut usize) -> Option<PathBuf> {
        if depth > MAX_DEPTH || *visited > MAX_FILES {
            return None;
        }
        let entries = fs::read_dir(path).ok()?;
        for entry in entries.flatten() {
            *visited += 1;
            let entry_path = entry.path();
            if entry_path.is_dir() {
                if let Some(found) = visit(&entry_path, depth + 1, visited) {
                    return Some(found);
                }
                continue;
            }
            if entry_path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("glb"))
            {
                return Some(entry_path);
            }
        }
        None
    }

    let mut visited = 0usize;
    visit(root, 0, &mut visited)
}

fn relative_path_to_url(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .map(|segment| urlencoding::encode(segment).to_string())
        .collect::<Vec<_>>()
        .join("/")
}

async fn validate_deploy_readiness(site: &ManagedProjectSite) -> Result<DeployValidationReport> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .context("创建部署验收 HTTP client 失败")?;
    let mut report = DeployValidationReport::new(&site.site_id);
    let local_base = format!("http://127.0.0.1:{}", site.web_port);

    push_required_http_check(
        &mut report,
        &client,
        "web_status",
        "站点 Web 健康",
        format!("{local_base}/api/status"),
        2,
    )
    .await;

    if let Some(viewer_url) = site.viewer_url.clone() {
        push_required_http_check(
            &mut report,
            &client,
            "viewer",
            "plant3d-web Viewer",
            viewer_url,
            64,
        )
        .await;
    } else if managed_viewer_enabled() {
        report.push(deploy_validation_check(
            "viewer",
            "plant3d-web Viewer",
            "blocking",
            "部署未产生 viewer_url，无法确认 plant3d-web 已接入当前站点",
            None,
            None,
            None,
        ));
    } else {
        report.push(deploy_validation_check(
            "viewer",
            "plant3d-web Viewer",
            "warning",
            "受管 Viewer 已禁用，跳过 Viewer 验收",
            Some("AIOS_MANAGED_VIEWER_ENABLED=0".to_string()),
            None,
            None,
        ));
    }

    if site.export_parquet {
        let output_project = site_source_project_name(site);
        let parquet_root = PathBuf::from("output")
            .join(&output_project)
            .join("parquet");
        if site.manual_db_nums.is_empty() {
            report.push(deploy_validation_check(
                "parquet_scope",
                "Parquet 目标",
                "warning",
                "未配置 manual_db_nums，无法按 dbnum 验收 Parquet manifest",
                Some(parquet_root.display().to_string()),
                None,
                None,
            ));
        }
        for dbnum in &site.manual_db_nums {
            let manifest_path = parquet_root.join(format!("manifest_{dbnum}.json"));
            let dbnum_dir = parquet_root.join(dbnum.to_string());
            let instances_path = dbnum_dir.join("instances.parquet");
            let geo_instances_path = dbnum_dir.join("geo_instances.parquet");

            push_required_json_file_check(
                &mut report,
                format!("parquet_manifest_{dbnum}"),
                format!("Parquet manifest {dbnum}"),
                &manifest_path,
                2,
            );
            push_required_file_check(
                &mut report,
                format!("parquet_instances_{dbnum}"),
                format!("instances.parquet {dbnum}"),
                &instances_path,
                1,
            );
            push_required_file_check(
                &mut report,
                format!("parquet_geo_instances_{dbnum}"),
                format!("geo_instances.parquet {dbnum}"),
                &geo_instances_path,
                1,
            );

            let encoded_project = urlencoding::encode(&output_project);
            push_required_http_check(
                &mut report,
                &client,
                format!("http_parquet_manifest_{dbnum}"),
                format!("HTTP manifest {dbnum}"),
                format!(
                    "{local_base}/files/output/{encoded_project}/parquet/manifest_{dbnum}.json"
                ),
                2,
            )
            .await;
            push_required_http_check(
                &mut report,
                &client,
                format!("http_parquet_instances_{dbnum}"),
                format!("HTTP instances.parquet {dbnum}"),
                format!(
                    "{local_base}/files/output/{encoded_project}/parquet/{dbnum}/instances.parquet"
                ),
                1,
            )
            .await;
            push_required_http_check(
                &mut report,
                &client,
                format!("http_parquet_geo_instances_{dbnum}"),
                format!("HTTP geo_instances.parquet {dbnum}"),
                format!("{local_base}/files/output/{encoded_project}/parquet/{dbnum}/geo_instances.parquet"),
                1,
            )
            .await;
        }
    } else {
        report.push(deploy_validation_check(
            "parquet_disabled",
            "Parquet 产物",
            "warning",
            "站点未启用 export_parquet，跳过 Parquet 验收",
            None,
            None,
            None,
        ));
    }

    let mesh_root = mesh_serve_root_from_current_config();
    if let Some(glb_path) = find_first_glb(&mesh_root) {
        push_required_file_check(&mut report, "mesh_glb_file", "GLB 模型文件", &glb_path, 1);
        let rel = glb_path.strip_prefix(&mesh_root).unwrap_or(&glb_path);
        let rel_url = relative_path_to_url(rel);
        push_required_http_check(
            &mut report,
            &client,
            "http_mesh_glb",
            "HTTP GLB 模型文件",
            format!("{local_base}/files/meshes/{rel_url}"),
            1,
        )
        .await;
    } else {
        report.push(deploy_validation_check(
            "mesh_glb_file",
            "GLB 模型文件",
            "blocking",
            "未找到可供 plant3d-web 加载的 .glb 模型文件",
            Some(mesh_root.display().to_string()),
            None,
            None,
        ));
    }

    write_deploy_validation_report(&report)?;
    Ok(report)
}

// ─── Port helpers ───────────────────────────────────────────────────────────

/// 列出占用指定端口的进程 PID 列表。
///
/// 实现：Unix 走 `lsof -i:PORT -sTCP:LISTEN`，Windows 走
/// `netstat -ano` + 过滤 `LISTENING`。返回空 Vec 表示端口未被占用（或
/// 占用进程已退出）。
///
/// `pub(crate)` 是为了让 `admin_handlers::ports_check` 端点（D4）复用，
/// 避免在多处重复实现端口探测逻辑。
pub(crate) async fn process_ids_on_port(port: u16) -> Result<Vec<u32>> {
    #[cfg(unix)]
    {
        let output = Command::new("lsof")
            .args(["-nP", "-ti", &format!("tcp:{port}"), "-sTCP:LISTEN"])
            .output()
            .await
            .context("读取端口进程失败")?;
        let ids = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.trim().parse::<u32>().ok())
            .collect::<Vec<_>>();
        Ok(ids)
    }
    #[cfg(windows)]
    {
        let output = Command::new("netstat")
            .args(["-ano"])
            .output()
            .await
            .context("读取端口进程失败")?;
        let want = format!(":{port}");
        let ids = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| line.contains(&want) && line.contains("LISTENING"))
            .filter_map(|line| line.split_whitespace().last()?.trim().parse::<u32>().ok())
            .collect::<Vec<_>>();
        Ok(ids)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = port;
        Ok(Vec::new())
    }
}

fn collect_port_pids_sync(port: u16) -> Vec<u32> {
    #[cfg(unix)]
    {
        let output = std::process::Command::new("lsof")
            .args(["-nP", "-ti", &format!("tcp:{port}"), "-sTCP:LISTEN"])
            .output();
        match output {
            Ok(out) => String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|line| line.trim().parse::<u32>().ok())
                .collect(),
            Err(_) => Vec::new(),
        }
    }
    #[cfg(windows)]
    {
        let output = std::process::Command::new("netstat")
            .args(["-ano"])
            .output();
        let want = format!(":{port}");
        match output {
            Ok(out) => String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter(|line| line.contains(&want) && line.contains("LISTENING"))
                .filter_map(|line| line.split_whitespace().last()?.trim().parse::<u32>().ok())
                .collect(),
            Err(_) => Vec::new(),
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = port;
        Vec::new()
    }
}

// ─── Kill helpers ───────────────────────────────────────────────────────────

#[cfg(unix)]
fn killpg_group(pid: u32, sig: libc::c_int) -> bool {
    // SAFETY: killpg 对 pgid 发信号；对象是我们通过 process_group(0) 启动的子进程。
    let pgid = unsafe { libc::getpgid(pid as libc::pid_t) };
    if pgid <= 0 {
        return false;
    }
    unsafe { libc::killpg(pgid, sig) == 0 }
}

async fn kill_pid(pid: u32) -> Result<()> {
    #[cfg(unix)]
    {
        // 先按整个进程组发 SIGTERM；若组查询失败再单独对 pid 发。
        if !killpg_group(pid, libc::SIGTERM) {
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
        }
        tokio::time::sleep(Duration::from_millis(KILL_GRACE_MS)).await;
        if pid_running(Some(pid)) {
            if !killpg_group(pid, libc::SIGKILL) {
                unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
            }
        }
    }
    #[cfg(windows)]
    {
        // /T：连同子进程一起结束；先温和 /T，再 /F。
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T"])
            .output()
            .await;
        tokio::time::sleep(Duration::from_millis(KILL_GRACE_MS)).await;
        if pid_running(Some(pid)) {
            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .output()
                .await;
        }
    }
    Ok(())
}

// ─── Parse / start pipelines ────────────────────────────────────────────────

fn site_was_stopped_by_user(site_id: &str) -> bool {
    matches!(
        get_site(site_id),
        Ok(Some(site)) if site.status == ManagedSiteStatus::Stopped && site.parse_pid.is_none()
    )
}

async fn cleanup_started_db(site_id: &str, db_pid: Option<u32>) {
    if let Some(pid) = db_pid {
        let _ = kill_pid(pid).await;
        let _ = update_runtime(
            site_id,
            RuntimeUpdate {
                db_pid: Some(None),
                ..Default::default()
            },
        );
    }
}

async fn spawn_parse_process(site_id: String) -> Result<()> {
    let (site, db_user, db_password) = task::spawn_blocking({
        let site_id = site_id.clone();
        move || load_site_and_credentials(&site_id)
    })
    .await
    .context("加载站点凭据失败 (join error)")??;

    task::spawn_blocking({
        let site = site.clone();
        let db_user = db_user.clone();
        let db_password = db_password.clone();
        move || write_site_files(&site, &db_user, &db_password)
    })
    .await
    .context("写入站点配置失败 (join error)")??;

    let config_path = parse_config_path(&site.site_id);
    let config_no_ext = config_path_without_toml(&config_path);
    let single_dbnum = site
        .manual_db_nums
        .first()
        .copied()
        .filter(|_| site.manual_db_nums.len() == 1);
    let (stdout, stderr) = open_log_file(&parse_log_path(&site.site_id))?;
    let repo = repo_root()?;

    let mut command = aios_database_command(&repo, &config_no_ext)?;
    if let Some(dbnum) = single_dbnum {
        command.arg("--dbnum").arg(dbnum.to_string());
    }
    command
        .current_dir(&repo)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    isolate_process_group(&mut command);

    let parse_started_at = now_rfc3339();
    let parse_started_instant = Instant::now();
    let mut child = command.spawn().context("启动解析进程失败")?;
    let pid = child.id();
    update_runtime(
        &site.site_id,
        RuntimeUpdate {
            status: Some(ManagedSiteStatus::Draft),
            parse_status: Some(ManagedSiteParseStatus::Running),
            parse_pid: Some(pid),
            last_error: Some(None),
            last_parse_started_at: Some(Some(parse_started_at)),
            last_parse_finished_at: Some(None),
            last_parse_duration_ms: Some(None),
            ..Default::default()
        },
    )?;

    let exit = child.wait().await.context("等待解析进程失败")?;
    let parse_finished_at = now_rfc3339();
    let parse_duration_ms = parse_started_instant.elapsed().as_millis() as u64;
    if exit.success() {
        update_runtime(
            &site.site_id,
            RuntimeUpdate {
                status: Some(ManagedSiteStatus::Parsed),
                parse_status: Some(ManagedSiteParseStatus::Parsed),
                parse_pid: Some(None),
                last_error: Some(None),
                last_parse_finished_at: Some(Some(parse_finished_at)),
                last_parse_duration_ms: Some(Some(parse_duration_ms)),
                ..Default::default()
            },
        )?;
        task::spawn_blocking({
            let site_id = site.site_id.clone();
            move || rewrite_site_files_from_storage(&site_id)
        })
        .await
        .context("刷新解析配置失败 (join error)")??;
    } else {
        if site_was_stopped_by_user(&site.site_id) {
            bail!("站点操作已被手动停止");
        }
        let message = format!("解析失败，退出码: {:?}", exit.code());
        update_runtime(
            &site.site_id,
            RuntimeUpdate {
                status: Some(ManagedSiteStatus::Failed),
                parse_status: Some(ManagedSiteParseStatus::Failed),
                parse_pid: Some(None),
                last_error: Some(Some(message.clone())),
                last_parse_finished_at: Some(Some(parse_finished_at)),
                last_parse_duration_ms: Some(Some(parse_duration_ms)),
                ..Default::default()
            },
        )?;
        bail!(message);
    }
    let _ = db_user;
    let _ = db_password;
    Ok(())
}

async fn spawn_generation_process(site_id: String) -> Result<()> {
    let (site, db_user, db_password) = task::spawn_blocking({
        let site_id = site_id.clone();
        move || load_site_and_credentials(&site_id)
    })
    .await
    .context("加载站点凭据失败 (join error)")??;

    if !generation_enabled(&site) {
        tracing::info!(
            site = %site.site_id,
            "模型生成配置均未启用，跳过 generation process"
        );
        return Ok(());
    }

    task::spawn_blocking({
        let site = site.clone();
        let db_user = db_user.clone();
        let db_password = db_password.clone();
        move || write_site_files(&site, &db_user, &db_password)
    })
    .await
    .context("写入站点配置失败 (join error)")??;

    let config_no_ext = config_string_without_toml(&site.config_path);
    let (stdout, stderr) = open_log_file(&generate_log_path(&site.site_id))?;
    let repo = repo_root()?;
    let mut command = aios_database_command(&repo, &config_no_ext)?;
    command
        .current_dir(&repo)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    isolate_process_group(&mut command);

    let mut child = command.spawn().context("启动模型生成进程失败")?;
    let pid = child.id();
    update_runtime(
        &site.site_id,
        RuntimeUpdate {
            status: Some(ManagedSiteStatus::Starting),
            parse_status: Some(ManagedSiteParseStatus::Parsed),
            parse_pid: Some(pid),
            last_error: Some(None),
            ..Default::default()
        },
    )?;

    let exit = child.wait().await.context("等待模型生成进程失败")?;
    if exit.success() {
        update_runtime(
            &site.site_id,
            RuntimeUpdate {
                status: Some(ManagedSiteStatus::Parsed),
                parse_status: Some(ManagedSiteParseStatus::Parsed),
                parse_pid: Some(None),
                last_error: Some(None),
                ..Default::default()
            },
        )?;
    } else {
        if site_was_stopped_by_user(&site.site_id) {
            bail!("站点操作已被手动停止");
        }
        let message = format!("模型生成失败，退出码: {:?}", exit.code());
        update_runtime(
            &site.site_id,
            RuntimeUpdate {
                status: Some(ManagedSiteStatus::Failed),
                parse_status: Some(ManagedSiteParseStatus::Parsed),
                parse_pid: Some(None),
                last_error: Some(Some(message.clone())),
                ..Default::default()
            },
        )?;
        bail!(message);
    }
    Ok(())
}

async fn spawn_db_process(site: &ManagedProjectSite) -> Result<u32> {
    let (db_user, db_password) = task::spawn_blocking({
        let site_id = site.site_id.clone();
        move || -> Result<_> { with_conn(|conn| load_credentials_with_conn(conn, &site_id)) }
    })
    .await
    .context("加载 DB 凭据失败 (join error)")??;
    let (stdout, stderr) = open_log_file(&db_log_path(&site.site_id))?;
    let mut command = Command::new("surreal");
    command
        .arg("start")
        .arg("--log")
        .arg("info")
        .arg("--bind")
        .arg(format!("127.0.0.1:{}", site.db_port))
        .arg(format!("rocksdb://{}", site.db_data_path))
        // 通过环境变量传递凭据，避免 `ps` 泄漏到其它用户。
        .env("SURREAL_USER", &db_user)
        .env("SURREAL_PASS", &db_password)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    isolate_process_group(&mut command);
    let child = command.spawn().context("启动 SurrealDB 失败")?;
    let pid = child.id().unwrap_or_default();
    Ok(pid)
}

async fn spawn_web_process(site: &ManagedProjectSite) -> Result<u32> {
    let config_no_ext = site
        .config_path
        .strip_suffix(".toml")
        .map(|value| value.to_string())
        .unwrap_or_else(|| site.config_path.clone());
    let exe = current_exe_path()?;
    let repo = repo_root()?;
    let (stdout, stderr) = open_log_file(&web_log_path(&site.site_id))?;
    let mut command = Command::new(exe);
    command
        .arg("--config")
        .arg(config_no_ext)
        .env("WEB_SERVER_PORT", site.web_port.to_string())
        .current_dir(repo)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    isolate_process_group(&mut command);
    let child = command.spawn().context("启动项目 web_server 失败")?;
    Ok(child.id().unwrap_or_default())
}

#[derive(Debug, Clone)]
struct ViewerLaunch {
    port: u16,
    pid: Option<u32>,
    url: String,
}

fn managed_viewer_enabled() -> bool {
    std::env::var("AIOS_MANAGED_VIEWER_ENABLED")
        .map(|value| !matches!(value.trim(), "0" | "false" | "off" | "no"))
        .unwrap_or(true)
}

fn viewer_project_dir() -> Result<Option<PathBuf>> {
    if let Ok(value) = std::env::var("AIOS_VIEWER_PROJECT_DIR") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            let path = PathBuf::from(trimmed);
            if path.exists() {
                return Ok(Some(path));
            }
            bail!(
                "AIOS_VIEWER_PROJECT_DIR 指向的 plant3d-web 目录不存在: {}",
                path.display()
            );
        }
    }

    let repo = repo_root()?;
    let Some(parent) = repo.parent() else {
        return Ok(None);
    };
    let candidate = parent.join("plant3d-web");
    Ok(candidate.exists().then_some(candidate))
}

fn configured_viewer_port() -> Result<Option<u16>> {
    let value = match std::env::var("AIOS_VIEWER_PORT") {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let port = trimmed
        .parse::<u16>()
        .with_context(|| format!("AIOS_VIEWER_PORT 不是有效端口: {trimmed}"))?;
    if port == 0 {
        bail!("AIOS_VIEWER_PORT 不能为 0");
    }
    Ok(Some(port))
}

async fn viewer_http_ok(port: u16) -> bool {
    let client = match reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };
    let url = format!("http://127.0.0.1:{port}/");
    let Ok(response) = client.get(url).send().await else {
        return false;
    };
    if !response.status().is_success() {
        return false;
    }
    let text = response.text().await.unwrap_or_default();
    text.contains("plant3d") || text.contains("Vite")
}

async fn choose_viewer_port(site: &ManagedProjectSite) -> Result<(u16, bool)> {
    let configured_port = configured_viewer_port()?;
    let explicit_port = site.viewer_port.or(configured_port);
    if let Some(port) = explicit_port {
        if port == site.db_port || port == site.web_port {
            bail!("Viewer 端口 {} 与站点 DB/Web 端口冲突", port);
        }
        if port_in_use("127.0.0.1", port) {
            if viewer_http_ok(port).await {
                return Ok((port, true));
            }
            bail!("Viewer 端口 {} 已被非 plant3d-web 进程占用", port);
        }
        return Ok((port, false));
    }

    for port in 3101..=3120 {
        if port == site.db_port || port == site.web_port {
            continue;
        }
        if port_in_use("127.0.0.1", port) {
            if viewer_http_ok(port).await {
                return Ok((port, true));
            }
            continue;
        }
        return Ok((port, false));
    }
    bail!("未找到可用的 Viewer 端口 (3101..3120)");
}

fn build_viewer_url(site: &ManagedProjectSite, port: u16) -> String {
    let base = format!("http://127.0.0.1:{port}");
    let backend = site
        .local_entry_url
        .clone()
        .or_else(|| site.entry_url.clone())
        .unwrap_or_else(|| format!("http://127.0.0.1:{}", site.web_port));
    let project = site
        .associated_project
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&site.project_name);
    let mut url = format!(
        "{base}/?backendPort={}&backend={}&output_project={}",
        site.web_port,
        urlencoding::encode(&backend),
        urlencoding::encode(project)
    );
    if let Some(dbnum) = site.manual_db_nums.first() {
        url.push_str("&show_dbnum=");
        url.push_str(&dbnum.to_string());
    }
    if site.export_parquet {
        url.push_str("&data_source=parquet");
    }
    url
}

async fn spawn_viewer_process(site: &ManagedProjectSite) -> Result<Option<ViewerLaunch>> {
    if !managed_viewer_enabled() {
        tracing::info!(site = %site.site_id, "受管 plant3d-web Viewer 启动已禁用");
        return Ok(None);
    }
    let Some(viewer_dir) = viewer_project_dir()? else {
        tracing::warn!(
            site = %site.site_id,
            "未找到 plant3d-web 目录，跳过受管 Viewer 启动（可设置 AIOS_VIEWER_PROJECT_DIR）"
        );
        return Ok(None);
    };

    let (port, reuse_existing) = choose_viewer_port(site).await?;
    let url = build_viewer_url(site, port);
    if reuse_existing {
        tracing::info!(site = %site.site_id, port, "复用已运行的 plant3d-web Viewer");
        return Ok(Some(ViewerLaunch {
            port,
            pid: None,
            url,
        }));
    }

    let (stdout, stderr) = open_log_file(&viewer_log_path(&site.site_id))?;
    #[cfg(windows)]
    let mut command = {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg("npm");
        cmd
    };
    #[cfg(not(windows))]
    let mut command = Command::new("npm");

    command
        .arg("run")
        .arg("dev")
        .arg("--")
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--strictPort")
        .current_dir(&viewer_dir)
        .env("BROWSER", "none")
        .env("VITE_BACKEND_PORT", site.web_port.to_string())
        .env(
            "VITE_BACKEND_URL",
            format!("http://127.0.0.1:{}", site.web_port),
        )
        .env(
            "VITE_API_BASE_URL",
            format!("http://127.0.0.1:{}", site.web_port),
        )
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    isolate_process_group(&mut command);

    let child = command.spawn().with_context(|| {
        format!(
            "启动 plant3d-web Viewer 失败 (dir={}, port={})",
            viewer_dir.display(),
            port
        )
    })?;
    let pid = child.id().unwrap_or_default();
    if !wait_for_http_ok(
        &format!("http://127.0.0.1:{port}/"),
        WAIT_HTTP_ATTEMPTS,
        WAIT_STEP_MS,
    )
    .await
    {
        if pid != 0 {
            let _ = kill_pid(pid).await;
        }
        bail!("plant3d-web Viewer 未在端口 {} 启动成功", port);
    }

    Ok(Some(ViewerLaunch {
        port,
        pid: (pid != 0).then_some(pid),
        url,
    }))
}

async fn ensure_site_db_started(
    site: &ManagedProjectSite,
    status: ManagedSiteStatus,
) -> Result<Option<u32>> {
    if port_in_use("127.0.0.1", site.db_port) {
        if probe_managed_port(site.db_port, site.db_pid).managed_running {
            return Ok(None);
        }
        bail!("数据库端口 {} 已被占用", site.db_port);
    }

    let db_pid = spawn_db_process(site).await?;
    update_runtime(
        &site.site_id,
        RuntimeUpdate {
            status: Some(status),
            db_pid: Some(Some(db_pid)),
            last_error: Some(None),
            ..Default::default()
        },
    )?;
    if !wait_for_port(site.db_port, WAIT_PORT_ATTEMPTS, WAIT_STEP_MS).await {
        let _ = kill_pid(db_pid).await;
        let _ = update_runtime(
            &site.site_id,
            RuntimeUpdate {
                db_pid: Some(None),
                ..Default::default()
            },
        );
        bail!("SurrealDB 未在端口 {} 成功启动", site.db_port);
    }
    Ok(Some(db_pid))
}

async fn run_parse_pipeline(site_id: String) -> Result<()> {
    let site = task::spawn_blocking({
        let site_id = site_id.clone();
        move || get_site(&site_id)
    })
    .await
    .context("读取站点状态失败 (join error)")??
    .ok_or_else(|| anyhow!("站点不存在"))?;

    let started_db_pid = ensure_site_db_started(&site, site.status.clone()).await?;
    let parse_result = spawn_parse_process(site_id.clone()).await;

    if let Some(db_pid) = started_db_pid {
        let _ = kill_pid(db_pid).await;
        let _ = update_runtime(
            &site_id,
            RuntimeUpdate {
                db_pid: Some(None),
                ..Default::default()
            },
        );
    }

    parse_result
}

async fn run_generation_pipeline(site_id: String, parse_first: bool) -> Result<()> {
    let site = task::spawn_blocking({
        let site_id = site_id.clone();
        move || get_site(&site_id)
    })
    .await
    .context("读取站点状态失败 (join error)")??
    .ok_or_else(|| anyhow!("站点不存在"))?;

    let started_db_pid = ensure_site_db_started(&site, ManagedSiteStatus::Starting).await?;

    let result = async {
        let site = task::spawn_blocking({
            let site_id = site_id.clone();
            move || get_site(&site_id)
        })
        .await
        .context("读取站点状态失败 (join error)")??
        .ok_or_else(|| anyhow!("站点不存在"))?;

        if site.parse_status != ManagedSiteParseStatus::Parsed {
            if parse_first {
                spawn_parse_process(site_id.clone()).await?;
            } else {
                bail!("站点尚未解析，请先执行解析或选择完整生成");
            }
        }

        spawn_generation_process(site_id.clone()).await
    }
    .await;

    if let Some(db_pid) = started_db_pid {
        let _ = kill_pid(db_pid).await;
        let _ = update_runtime(
            &site_id,
            RuntimeUpdate {
                db_pid: Some(None),
                ..Default::default()
            },
        );
    }

    result
}

async fn run_deploy_pipeline(site_id: String) -> Result<()> {
    let site = task::spawn_blocking({
        let site_id = site_id.clone();
        move || get_site(&site_id)
    })
    .await
    .context("读取站点状态失败 (join error)")??
    .ok_or_else(|| anyhow!("站点不存在"))?;

    if generation_enabled(&site) {
        run_generation_pipeline(site_id.clone(), true).await?;
    } else if site.parse_status != ManagedSiteParseStatus::Parsed {
        run_parse_pipeline(site_id.clone()).await?;
    }

    run_start_pipeline_for_deploy(site_id.clone()).await?;

    let site = task::spawn_blocking({
        let site_id = site_id.clone();
        move || get_site(&site_id)
    })
    .await
    .context("读取站点状态失败 (join error)")??
    .ok_or_else(|| anyhow!("站点不存在"))?;
    let validation = validate_deploy_readiness(&site).await?;
    if validation.blocking_count > 0 {
        let blocking = deploy_validation_blocking_summary(&validation);
        bail!("部署后验收未通过: {blocking}");
    }
    update_runtime(
        &site_id,
        RuntimeUpdate {
            status: Some(ManagedSiteStatus::Running),
            last_error: Some(None),
            ..Default::default()
        },
    )?;
    Ok(())
}

async fn run_start_pipeline(site_id: String) -> Result<()> {
    run_start_pipeline_inner(site_id, true).await
}

async fn run_start_pipeline_for_deploy(site_id: String) -> Result<()> {
    run_start_pipeline_inner(site_id, false).await
}

async fn run_start_pipeline_inner(site_id: String, mark_running: bool) -> Result<()> {
    let site = task::spawn_blocking({
        let site_id = site_id.clone();
        move || get_site(&site_id)
    })
    .await
    .context("读取站点状态失败 (join error)")??
    .ok_or_else(|| anyhow!("站点不存在"))?;

    task::spawn_blocking({
        let site_id = site_id.clone();
        let db_port = site.db_port;
        let web_port = site.web_port;
        move || -> Result<()> {
            with_tx(|conn| assert_port_available_with_conn(conn, Some(&site_id), db_port, web_port))
        }
    })
    .await
    .context("端口校验失败 (join error)")??;

    if site.parse_status == ManagedSiteParseStatus::Running {
        bail!("解析任务仍在运行，请稍后再启动站点");
    }
    update_runtime(
        &site_id,
        RuntimeUpdate {
            status: Some(ManagedSiteStatus::Starting),
            last_error: Some(None),
            viewer_port: Some(None),
            viewer_pid: Some(None),
            viewer_url: Some(None),
            ..Default::default()
        },
    )?;

    let site = task::spawn_blocking({
        let site_id = site_id.clone();
        move || get_site(&site_id)
    })
    .await
    .context("读取站点状态失败 (join error)")??
    .ok_or_else(|| anyhow!("站点不存在"))?;
    let db_pid = ensure_site_db_started(&site, ManagedSiteStatus::Starting).await?;

    let site = task::spawn_blocking({
        let site_id = site_id.clone();
        move || get_site(&site_id)
    })
    .await
    .context("读取站点状态失败 (join error)")??
    .ok_or_else(|| anyhow!("站点不存在"))?;
    if site.parse_status != ManagedSiteParseStatus::Parsed {
        if let Err(err) = spawn_parse_process(site_id.clone()).await {
            let stopped_by_user = site_was_stopped_by_user(&site_id);
            cleanup_started_db(&site_id, db_pid).await;
            if stopped_by_user {
                return Err(err);
            }
            let _ = update_runtime(
                &site_id,
                RuntimeUpdate {
                    status: Some(ManagedSiteStatus::Failed),
                    parse_status: Some(ManagedSiteParseStatus::Failed),
                    db_pid: Some(None),
                    last_error: Some(Some(format!("启动解析失败: {err}"))),
                    ..Default::default()
                },
            );
            return Err(err);
        }
    }

    let site = task::spawn_blocking({
        let site_id = site_id.clone();
        move || get_site(&site_id)
    })
    .await
    .context("读取站点状态失败 (join error)")??
    .ok_or_else(|| anyhow!("站点不存在"))?;
    let web_pid = match spawn_web_process(&site).await {
        Ok(pid) => pid,
        Err(err) => {
            cleanup_started_db(&site_id, db_pid).await;
            return Err(err);
        }
    };
    update_runtime(
        &site_id,
        RuntimeUpdate {
            status: Some(ManagedSiteStatus::Starting),
            web_pid: Some(Some(web_pid)),
            last_error: Some(None),
            entry_url: Some(Some(format!("http://127.0.0.1:{}", site.web_port))),
            ..Default::default()
        },
    )?;
    let status_url = format!("http://127.0.0.1:{}/api/status", site.web_port);
    if !wait_for_http_ok(&status_url, WAIT_HTTP_ATTEMPTS, WAIT_STEP_MS).await {
        let _ = kill_pid(web_pid).await;
        cleanup_started_db(&site_id, db_pid).await;
        let _ = update_runtime(
            &site_id,
            RuntimeUpdate {
                web_pid: Some(None),
                ..Default::default()
            },
        );
        bail!("项目站点未在 {} 启动成功", status_url);
    }

    let viewer = match spawn_viewer_process(&site).await {
        Ok(viewer) => viewer,
        Err(err) => {
            let _ = kill_pid(web_pid).await;
            cleanup_started_db(&site_id, db_pid).await;
            let _ = update_runtime(
                &site_id,
                RuntimeUpdate {
                    status: Some(ManagedSiteStatus::Failed),
                    web_pid: Some(None),
                    viewer_port: Some(None),
                    viewer_pid: Some(None),
                    viewer_url: Some(None),
                    last_error: Some(Some(format!("启动 Viewer 失败: {err}"))),
                    ..Default::default()
                },
            );
            return Err(err);
        }
    };
    let (viewer_port, viewer_pid, viewer_url) = viewer
        .map(|launch| (Some(launch.port), launch.pid, Some(launch.url)))
        .unwrap_or((None, None, None));

    update_runtime(
        &site_id,
        RuntimeUpdate {
            status: Some(if mark_running {
                ManagedSiteStatus::Running
            } else {
                ManagedSiteStatus::Starting
            }),
            parse_status: Some(ManagedSiteParseStatus::Parsed),
            parse_pid: Some(None),
            viewer_port: Some(viewer_port),
            viewer_pid: Some(viewer_pid),
            viewer_url: Some(viewer_url),
            last_error: Some(None),
            entry_url: Some(Some(format!("http://127.0.0.1:{}", site.web_port))),
            ..Default::default()
        },
    )?;
    Ok(())
}

pub async fn start_site(site_id: String) -> Result<()> {
    let site = task::spawn_blocking({
        let site_id = site_id.clone();
        move || get_site(&site_id)
    })
    .await
    .context("读取站点状态失败 (join error)")??
    .ok_or_else(|| anyhow!("站点不存在"))?;
    if matches!(
        site.status,
        ManagedSiteStatus::Running | ManagedSiteStatus::Starting | ManagedSiteStatus::Stopping
    ) {
        let message = if site.status == ManagedSiteStatus::Stopping {
            "站点停止中，请稍后再启动".to_string()
        } else {
            "站点已在运行中".to_string()
        };
        record_site_error(&site_id, message.clone(), Some(site.status.clone()), None);
        bail!(message);
    }
    if site.parse_status == ManagedSiteParseStatus::Running {
        let message = "解析任务仍在运行，请稍后再启动站点".to_string();
        record_site_error(
            &site_id,
            message.clone(),
            Some(site.status.clone()),
            Some(ManagedSiteParseStatus::Running),
        );
        bail!(message);
    }
    update_runtime(
        &site_id,
        RuntimeUpdate {
            status: Some(ManagedSiteStatus::Starting),
            last_error: Some(None),
            ..Default::default()
        },
    )?;
    tokio::spawn(async move {
        if let Err(err) = run_start_pipeline(site_id.clone()).await {
            if site_was_stopped_by_user(&site_id) {
                return;
            }
            let _ = update_runtime(
                &site_id,
                RuntimeUpdate {
                    status: Some(ManagedSiteStatus::Failed),
                    parse_pid: Some(None),
                    last_error: Some(Some(err.to_string())),
                    ..Default::default()
                },
            );
        }
    });
    Ok(())
}

pub async fn parse_site(site_id: String) -> Result<()> {
    let site = task::spawn_blocking({
        let site_id = site_id.clone();
        move || get_site(&site_id)
    })
    .await
    .context("读取站点状态失败 (join error)")??
    .ok_or_else(|| anyhow!("站点不存在"))?;
    if site.parse_status == ManagedSiteParseStatus::Running {
        let message = "解析任务正在运行".to_string();
        record_site_error(
            &site_id,
            message.clone(),
            Some(site.status.clone()),
            Some(ManagedSiteParseStatus::Running),
        );
        bail!(message);
    }
    if matches!(
        site.status,
        ManagedSiteStatus::Running | ManagedSiteStatus::Starting | ManagedSiteStatus::Stopping
    ) {
        let message = match site.status {
            ManagedSiteStatus::Running => "站点运行中，请先停止站点再解析",
            ManagedSiteStatus::Starting => "站点启动中，请先停止站点再解析",
            ManagedSiteStatus::Stopping => "站点停止中，请稍后再解析",
            _ => "当前状态不能执行解析",
        }
        .to_string();
        record_site_error(&site_id, message.clone(), Some(site.status.clone()), None);
        bail!(message);
    }
    tokio::spawn(async move {
        if let Err(err) = run_parse_pipeline(site_id.clone()).await {
            if site_was_stopped_by_user(&site_id) {
                return;
            }
            let _ = update_runtime(
                &site_id,
                RuntimeUpdate {
                    status: Some(ManagedSiteStatus::Failed),
                    parse_status: Some(ManagedSiteParseStatus::Failed),
                    parse_pid: Some(None),
                    last_error: Some(Some(err.to_string())),
                    ..Default::default()
                },
            );
        }
    });
    Ok(())
}

pub async fn generate_site(site_id: String, parse_first: bool) -> Result<()> {
    let site = task::spawn_blocking({
        let site_id = site_id.clone();
        move || get_site(&site_id)
    })
    .await
    .context("读取站点状态失败 (join error)")??
    .ok_or_else(|| anyhow!("站点不存在"))?;
    if !generation_enabled(&site) {
        let message = "模型生成配置未启用，请先在站点配置中开启生成模型、网格或空间树".to_string();
        record_site_error(&site_id, message.clone(), Some(site.status.clone()), None);
        bail!(message);
    }
    if site.parse_status == ManagedSiteParseStatus::Running {
        let message = "解析任务正在运行，请稍后再生成模型".to_string();
        record_site_error(
            &site_id,
            message.clone(),
            Some(site.status.clone()),
            Some(ManagedSiteParseStatus::Running),
        );
        bail!(message);
    }
    if matches!(
        site.status,
        ManagedSiteStatus::Running | ManagedSiteStatus::Starting | ManagedSiteStatus::Stopping
    ) {
        let message = match site.status {
            ManagedSiteStatus::Running => "站点运行中，请先停止站点再生成模型",
            ManagedSiteStatus::Starting => "站点启动中，请稍后再生成模型",
            ManagedSiteStatus::Stopping => "站点停止中，请稍后再生成模型",
            _ => "当前状态不能执行模型生成",
        }
        .to_string();
        record_site_error(&site_id, message.clone(), Some(site.status.clone()), None);
        bail!(message);
    }
    let preflight = preflight_site(&site_id).await?;
    if !preflight.ready {
        let blocking = preflight_blocking_summary(&preflight);
        let message = format!("部署预检未通过: {blocking}");
        record_site_error(&site_id, message.clone(), Some(site.status.clone()), None);
        bail!(message);
    }
    update_runtime(
        &site_id,
        RuntimeUpdate {
            status: Some(ManagedSiteStatus::Starting),
            last_error: Some(None),
            ..Default::default()
        },
    )?;
    tokio::spawn(async move {
        if let Err(err) = run_generation_pipeline(site_id.clone(), parse_first).await {
            if site_was_stopped_by_user(&site_id) {
                return;
            }
            let _ = update_runtime(
                &site_id,
                RuntimeUpdate {
                    status: Some(ManagedSiteStatus::Failed),
                    parse_pid: Some(None),
                    last_error: Some(Some(err.to_string())),
                    ..Default::default()
                },
            );
        }
    });
    Ok(())
}

pub async fn deploy_site(site_id: String) -> Result<()> {
    let site = task::spawn_blocking({
        let site_id = site_id.clone();
        move || get_site(&site_id)
    })
    .await
    .context("读取站点状态失败 (join error)")??
    .ok_or_else(|| anyhow!("站点不存在"))?;
    if site.parse_status == ManagedSiteParseStatus::Running {
        let message = "解析任务正在运行，请稍后再完整部署".to_string();
        record_site_error(
            &site_id,
            message.clone(),
            Some(site.status.clone()),
            Some(ManagedSiteParseStatus::Running),
        );
        bail!(message);
    }
    if matches!(
        site.status,
        ManagedSiteStatus::Running | ManagedSiteStatus::Starting | ManagedSiteStatus::Stopping
    ) {
        let message = match site.status {
            ManagedSiteStatus::Running => "站点运行中，请先停止站点再完整部署",
            ManagedSiteStatus::Starting => "站点启动中，请稍后再完整部署",
            ManagedSiteStatus::Stopping => "站点停止中，请稍后再完整部署",
            _ => "当前状态不能执行完整部署",
        }
        .to_string();
        record_site_error(&site_id, message.clone(), Some(site.status.clone()), None);
        bail!(message);
    }
    let preflight = preflight_site(&site_id).await?;
    if !preflight.ready {
        let blocking = preflight_blocking_summary(&preflight);
        let message = format!("部署预检未通过: {blocking}");
        record_site_error(&site_id, message.clone(), Some(site.status.clone()), None);
        bail!(message);
    }
    update_runtime(
        &site_id,
        RuntimeUpdate {
            status: Some(ManagedSiteStatus::Starting),
            last_error: Some(None),
            ..Default::default()
        },
    )?;
    tokio::spawn(async move {
        if let Err(err) = run_deploy_pipeline(site_id.clone()).await {
            if site_was_stopped_by_user(&site_id) {
                return;
            }
            let _ = update_runtime(
                &site_id,
                RuntimeUpdate {
                    status: Some(ManagedSiteStatus::Failed),
                    parse_pid: Some(None),
                    last_error: Some(Some(err.to_string())),
                    ..Default::default()
                },
            );
        }
    });
    Ok(())
}

/// 重启站点（C6 / Sprint C · 修 G10）
///
/// 串联 `stop_site` → 短暂等待 → `start_site`，作为单个原子化的"重启"动作
/// 暴露给 admin 前端，避免用户手动两步操作期间的状态尴尬期
/// （Stopping → Stopped → Starting）。
///
/// 实现要点：
/// - stop 阶段如发生端口冲突（外部进程占用），直接 bail，由前端展示原因
/// - stop 与 start 之间留 500ms 缓冲，让进程组完全退出 + socket TIME_WAIT
///   清理一部分；端口完全可用的兜底由 `start_site` 内部的 `WAIT_PORT_ATTEMPTS`
///   （30 次 × 500ms）承担
/// - start 失败后状态会被 `start_site` spawn 的内部错误路径写为 Failed，
///   外部调用方只需关注函数返回的 Result
pub async fn restart_site(site_id: &str) -> Result<()> {
    let stop_result = stop_site(site_id).await?;
    if stop_result.conflict {
        bail!(
            "停止站点时检测到端口冲突（web={:?} db={:?} viewer={:?}），无法继续重启；请先排查外部占用",
            stop_result.web_conflict_pids,
            stop_result.db_conflict_pids,
            stop_result.viewer_conflict_pids
        );
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    start_site(site_id.to_string()).await
}

pub async fn stop_site(site_id: &str) -> Result<StopSiteResult> {
    // 注：stop_site 不持 lock_op()——std::sync::MutexGuard 无法跨 await 持有，
    // 而 create/update/delete 都有 `site_has_active_processes` 的状态校验兜底，
    // 并发场景下 update_runtime 的事务保证最终状态一致。
    let site = task::spawn_blocking({
        let site_id = site_id.to_string();
        move || get_site(&site_id)
    })
    .await
    .context("读取站点状态失败 (join error)")??
    .ok_or_else(|| anyhow!("站点不存在"))?;
    let can_stop = matches!(
        site.status,
        ManagedSiteStatus::Running | ManagedSiteStatus::Starting | ManagedSiteStatus::Stopping
    ) || site.parse_status == ManagedSiteParseStatus::Running
        || site_has_active_processes(&site);
    if !can_stop {
        let message = "站点未在运行中，无需停止".to_string();
        record_site_error(site_id, message.clone(), Some(site.status.clone()), None);
        bail!(message);
    }
    update_runtime(
        site_id,
        RuntimeUpdate {
            status: Some(ManagedSiteStatus::Stopping),
            last_error: Some(None),
            ..Default::default()
        },
    )?;

    // 顺序：生产者先停（parse、web），消费者（db）最后停，避免 parse 写库时 db 突然消失。
    if let Some(pid) = site.parse_pid {
        kill_pid(pid).await?;
    }
    if let Some(pid) = site.viewer_pid {
        kill_pid(pid).await?;
    }
    if let Some(pid) = site.web_pid {
        kill_pid(pid).await?;
    }
    if let Some(pid) = site.db_pid {
        kill_pid(pid).await?;
    }

    let web_conflict_pids = process_ids_on_port(site.web_port).await.unwrap_or_default();
    let db_conflict_pids = process_ids_on_port(site.db_port).await.unwrap_or_default();
    let viewer_conflict_pids = match (site.viewer_pid, site.viewer_port) {
        (Some(_), Some(port)) => process_ids_on_port(port).await.unwrap_or_default(),
        _ => Vec::new(),
    };
    let has_conflict = !web_conflict_pids.is_empty()
        || !db_conflict_pids.is_empty()
        || !viewer_conflict_pids.is_empty();

    if has_conflict {
        let mut reasons = Vec::new();
        if !web_conflict_pids.is_empty() {
            reasons.push(format!(
                "web 端口 {} 被外部进程占用 (PIDs: {:?})",
                site.web_port, web_conflict_pids
            ));
        }
        if !db_conflict_pids.is_empty() {
            reasons.push(format!(
                "db 端口 {} 被外部进程占用 (PIDs: {:?})",
                site.db_port, db_conflict_pids
            ));
        }
        if let Some(port) = site.viewer_port {
            if !viewer_conflict_pids.is_empty() {
                reasons.push(format!(
                    "viewer 端口 {} 被外部进程占用 (PIDs: {:?})",
                    port, viewer_conflict_pids
                ));
            }
        }
        let conflict_msg = reasons.join("; ");
        update_runtime(
            site_id,
            RuntimeUpdate {
                status: Some(ManagedSiteStatus::Failed),
                db_pid: Some(None),
                web_pid: Some(None),
                viewer_pid: Some(None),
                viewer_url: Some(None),
                parse_pid: Some(None),
                last_error: Some(Some(format!("端口冲突: {}", conflict_msg))),
                ..Default::default()
            },
        )?;
        let updated = get_site(site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
        return Ok(StopSiteResult {
            site: updated,
            conflict: true,
            web_conflict_pids,
            db_conflict_pids,
            viewer_conflict_pids,
        });
    }

    let operation_was_running = site.parse_pid.is_some();
    let parse_was_running = site.parse_status == ManagedSiteParseStatus::Running;
    let next_parse_status = if parse_was_running {
        ManagedSiteParseStatus::Pending
    } else {
        site.parse_status.clone()
    };
    let aborted_finished_at = if parse_was_running {
        Some(Some(now_rfc3339()))
    } else {
        None
    };
    let aborted_error = if operation_was_running {
        Some(Some("站点操作被手动中止".to_string()))
    } else {
        Some(None)
    };

    update_runtime(
        site_id,
        RuntimeUpdate {
            status: Some(ManagedSiteStatus::Stopped),
            parse_status: Some(next_parse_status),
            db_pid: Some(None),
            web_pid: Some(None),
            viewer_pid: Some(None),
            viewer_url: Some(None),
            parse_pid: Some(None),
            last_error: aborted_error,
            last_parse_finished_at: aborted_finished_at,
            ..Default::default()
        },
    )?;
    let updated = get_site(site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
    Ok(StopSiteResult {
        site: updated,
        conflict: false,
        web_conflict_pids: Vec::new(),
        db_conflict_pids: Vec::new(),
        viewer_conflict_pids: Vec::new(),
    })
}

pub struct StopSiteResult {
    pub site: ManagedProjectSite,
    pub conflict: bool,
    pub web_conflict_pids: Vec<u32>,
    pub db_conflict_pids: Vec<u32>,
    pub viewer_conflict_pids: Vec<u32>,
}

pub fn delete_site(site_id: &str) -> Result<bool> {
    let _guard = lock_op()?;

    if let Some(site) = get_site(site_id)? {
        if site_has_active_processes(&site)
            || matches!(
                site.status,
                ManagedSiteStatus::Running
                    | ManagedSiteStatus::Starting
                    | ManagedSiteStatus::Stopping
            )
            || site.parse_status == ManagedSiteParseStatus::Running
        {
            let message = "站点运行中，不能删除".to_string();
            record_site_error(site_id, message.clone(), Some(site.status.clone()), None);
            bail!(message);
        }
    } else {
        return Ok(false);
    }
    let changed = with_tx(|conn| {
        let rows = conn.execute(
            &format!("DELETE FROM {table} WHERE site_id = ?1", table = TABLE_NAME),
            [site_id],
        )?;
        Ok(rows)
    })?;
    let runtime = site_runtime_dir(site_id);
    if runtime.exists() {
        if let Err(err) = fs::remove_dir_all(&runtime) {
            tracing::warn!(
                site = %site_id,
                "清理站点运行目录失败（请手动检查 {}）: {}",
                runtime.display(),
                err
            );
        }
    }

    // D1 / Sprint D · 修 G8：仅当 SQLite 真正删除了一行时广播 deleted 事件
    // （changed == 0 表示站点不存在，无需通知前端）
    if changed > 0 {
        crate::web_server::sse_handlers::push_admin_site_deleted(site_id);
    }

    Ok(changed > 0)
}

pub fn runtime_status(site_id: &str) -> Result<ManagedSiteRuntimeStatus> {
    let site = get_site(site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
    let db_probe = probe_managed_port(site.db_port, site.db_pid);
    let web_probe = probe_managed_port(site.web_port, site.web_pid);
    let viewer_probe = site
        .viewer_port
        .map(|port| probe_managed_port(port, site.viewer_pid))
        .unwrap_or_default();
    let db_running = db_probe.managed_running;
    let web_running = web_probe.managed_running;
    let viewer_pid_running = pid_running(site.viewer_pid);
    let viewer_adopted_running = site
        .viewer_port
        .map(|port| {
            port_in_use("127.0.0.1", port)
                && site.viewer_url.is_some()
                && matches!(
                    site.status,
                    ManagedSiteStatus::Running | ManagedSiteStatus::Starting
                )
        })
        .unwrap_or(false);
    let viewer_running =
        viewer_probe.managed_running || viewer_pid_running || viewer_adopted_running;
    let parse_running = pid_running(site.parse_pid);
    let resources = collect_site_resource_metrics(
        &site,
        db_running,
        web_running,
        viewer_running,
        parse_running,
    );
    let snapshots = collect_log_snapshots(site_id);
    let parse_snapshot = snapshots.iter().find(|snapshot| snapshot.key == "parse");
    let generate_snapshot = snapshots.iter().find(|snapshot| snapshot.key == "generate");
    let db_snapshot = snapshots.iter().find(|snapshot| snapshot.key == "db");
    let web_snapshot = snapshots.iter().find(|snapshot| snapshot.key == "web");
    let viewer_snapshot = snapshots.iter().find(|snapshot| snapshot.key == "viewer");
    let recent = snapshots
        .iter()
        .filter_map(|snapshot| {
            snapshot.updated_at.map(|updated_at| {
                (
                    updated_at,
                    ManagedSiteActivitySummary {
                        source: snapshot.key.to_string(),
                        label: snapshot.label.to_string(),
                        updated_at: snapshot.updated_at_rfc3339.clone(),
                        summary: snapshot.last_key_log.clone(),
                    },
                )
            })
        })
        .max_by_key(|(updated_at, _)| *updated_at);
    let active_log_kind = recent.as_ref().map(|(_, summary)| summary.source.clone());
    let last_log_at = recent
        .as_ref()
        .and_then(|(_, summary)| summary.updated_at.clone());
    let recent_log_source = active_log_kind.clone();
    let recent_log_at = last_log_at.clone();
    let last_key_log = recent
        .as_ref()
        .and_then(|(_, summary)| summary.summary.clone());
    let last_key_log_source = recent_log_source.clone();
    let recent_activity = recent.map(|(_, summary)| summary);
    let (current_stage, current_stage_label, current_stage_detail) = current_stage(
        &site,
        db_running,
        web_running,
        parse_running,
        generate_snapshot.and_then(|snapshot| snapshot.last_key_log.clone()),
        parse_snapshot.and_then(|snapshot| snapshot.last_key_log.clone()),
        db_snapshot.and_then(|snapshot| snapshot.last_key_log.clone()),
        web_snapshot
            .and_then(|snapshot| snapshot.last_key_log.clone())
            .or_else(|| viewer_snapshot.and_then(|snapshot| snapshot.last_key_log.clone())),
    );

    let (risk_level, mut warnings, parse_health) = evaluate_site_risk(&site, &resources);

    let db_conflict_pids = db_probe.conflict_pids;
    let web_conflict_pids = web_probe.conflict_pids;
    let viewer_conflict_pids = if viewer_probe.managed_running || viewer_adopted_running {
        Vec::new()
    } else {
        viewer_probe.conflict_pids
    };
    let db_port_conflict = !db_conflict_pids.is_empty();
    let web_port_conflict = !web_conflict_pids.is_empty();
    let viewer_port_conflict = !viewer_conflict_pids.is_empty();
    if db_port_conflict {
        warnings.push(format!(
            "db 端口 {} 被外部进程占用 (PIDs: {:?})",
            site.db_port, db_conflict_pids
        ));
    }
    if web_port_conflict {
        warnings.push(format!(
            "web 端口 {} 被外部进程占用 (PIDs: {:?})",
            site.web_port, web_conflict_pids
        ));
    }
    if viewer_port_conflict {
        if let Some(port) = site.viewer_port {
            warnings.push(format!(
                "viewer 端口 {} 被外部进程占用 (PIDs: {:?})",
                port, viewer_conflict_pids
            ));
        }
    }

    Ok(ManagedSiteRuntimeStatus {
        site_id: site.site_id,
        status: site.status,
        parse_status: site.parse_status,
        parse_plan: site.parse_plan,
        current_stage,
        current_stage_label,
        current_stage_detail,
        db_running,
        web_running,
        viewer_running,
        parse_running,
        db_pid: site.db_pid,
        web_pid: site.web_pid,
        viewer_pid: site.viewer_pid,
        parse_pid: site.parse_pid,
        db_port: site.db_port,
        web_port: site.web_port,
        viewer_port: site.viewer_port,
        viewer_url: site.viewer_url,
        entry_url: site.entry_url,
        local_entry_url: site.local_entry_url,
        public_entry_url: site.public_entry_url,
        db_port_conflict,
        web_port_conflict,
        viewer_port_conflict,
        db_conflict_pids,
        web_conflict_pids,
        viewer_conflict_pids,
        last_error: site.last_error,
        active_log_kind,
        last_log_at,
        recent_log_source,
        recent_log_at,
        last_key_log,
        last_key_log_source,
        recent_activity,
        resources: Some(resources),
        risk_level,
        warnings,
        parse_health,
    })
}

// ─── Log snapshots ──────────────────────────────────────────────────────────

fn tail_file(path: &Path) -> Vec<String> {
    let file = match OpenOptions::new().read(true).open(path) {
        Ok(file) => file,
        Err(_) => return Vec::new(),
    };
    let reader = BufReader::new(file);
    let mut lines = reader.lines().map_while(Result::ok).collect::<Vec<_>>();
    if lines.len() > LOG_LINES_LIMIT {
        lines = lines.split_off(lines.len() - LOG_LINES_LIMIT);
    }
    lines
}

fn system_time_to_rfc3339(time: SystemTime) -> String {
    DateTime::<Utc>::from(time).to_rfc3339()
}

/// 轻量 ANSI 转义清理：处理 CSI `\x1b[...`、OSC `\x1b]...BEL/ST`、以及单字节 `\x1b?`。
fn strip_ansi_codes(line: &str) -> String {
    let mut cleaned = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            cleaned.push(ch);
            continue;
        }
        match chars.peek() {
            Some(&'[') => {
                let _ = chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            Some(&']') => {
                let _ = chars.next();
                while let Some(next) = chars.next() {
                    if next == '\u{7}' {
                        break;
                    }
                    if next == '\u{1b}' {
                        if let Some(&'\\') = chars.peek() {
                            let _ = chars.next();
                            break;
                        }
                    }
                }
            }
            Some(_) => {
                let _ = chars.next();
            }
            None => {}
        }
    }
    cleaned
}

fn last_non_empty_line(lines: &[String]) -> Option<String> {
    lines.iter().rev().find_map(|line| {
        let normalized = strip_ansi_codes(line);
        let trimmed = normalized.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn summarize_log_line(key: &str, line: Option<&str>) -> Option<String> {
    let line = strip_ansi_codes(line?).trim().to_string();
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    if matches!(line, "Goodbye!" | "✓ 功能测试通过" | "✓ 数据库初始化完成") {
        return None;
    }
    if line.starts_with('.')
        || line.starts_with('d')
        || line.starts_with('Y')
        || line.starts_with('\'')
    {
        let compact = line.replace(' ', "");
        if compact.contains("888") {
            return None;
        }
    }

    if key == "parse" {
        if line.contains("数据库连接成功") {
            return Some("解析环境已连上数据库".to_string());
        }
        if line.contains("数据库初始化完成") {
            return Some("解析环境初始化完成".to_string());
        }
        if line.contains("执行多线程解析") {
            return Some("开始执行解析".to_string());
        }
        if let Some((_, rest)) = line.split_once("read file ") {
            let path = rest
                .split_whitespace()
                .next()
                .unwrap_or(rest)
                .trim_matches('"');
            let name = Path::new(path)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or(path);
            return Some(format!("最近解析文件 {}", name));
        }
        if let Some((_, rest)) = line.split_once("db_type is ") {
            return Some(format!("正在处理 {} 数据", rest.trim()));
        }
        if let Some((_, rest)) = line.split_once("All refnos count:") {
            return Some(format!("最近 refno 计数 {}", rest.trim()));
        }
    }

    if key == "web" {
        if line.contains("Web UI服务器启动成功") {
            return Some("站点服务已启动".to_string());
        }
        if let Some((_, rest)) = line.split_once("访问地址:") {
            return Some(format!("站点入口 {}", rest.trim()));
        }
    }

    if key == "db" {
        if line.contains("SIGTERM received") {
            return Some("数据库收到停止信号".to_string());
        }
        if line.contains("Credentials were provided") {
            return Some("数据库已启动，沿用现有 root 用户".to_string());
        }
        if line.contains("root user") {
            return Some("数据库保留现有 root 用户".to_string());
        }
    }

    Some(line.to_string())
}

fn log_snapshot(key: &'static str, label: &'static str, path: PathBuf) -> LogSnapshot {
    let exists = path.exists();
    let lines = tail_file(&path);
    let line_count = if exists {
        OpenOptions::new()
            .read(true)
            .open(&path)
            .ok()
            .map(|file| BufReader::new(file).lines().map_while(Result::ok).count())
            .unwrap_or(lines.len())
    } else {
        0
    };
    let has_content = line_count > 0 || lines.iter().any(|line| !line.trim().is_empty());
    let updated_at = fs::metadata(&path)
        .ok()
        .and_then(|meta| meta.modified().ok());
    let updated_at_rfc3339 = updated_at.map(system_time_to_rfc3339);
    let last_line = last_non_empty_line(&lines);
    let last_key_log = lines
        .iter()
        .rev()
        .find_map(|line| summarize_log_line(key, Some(line.as_str())));

    LogSnapshot {
        key,
        label,
        path,
        exists,
        has_content,
        updated_at,
        updated_at_rfc3339,
        lines,
        line_count,
        last_line,
        last_key_log,
    }
}

fn collect_log_snapshots(site_id: &str) -> Vec<LogSnapshot> {
    vec![
        log_snapshot("parse", "解析日志", parse_log_path(site_id)),
        log_snapshot("generate", "生成日志", generate_log_path(site_id)),
        log_snapshot("db", "数据库日志", db_log_path(site_id)),
        log_snapshot("web", "站点日志", web_log_path(site_id)),
        log_snapshot("viewer", "Viewer 日志", viewer_log_path(site_id)),
    ]
}

fn current_stage(
    site: &ManagedProjectSite,
    db_running: bool,
    web_running: bool,
    parse_running: bool,
    generate_detail: Option<String>,
    parse_detail: Option<String>,
    db_detail: Option<String>,
    web_detail: Option<String>,
) -> (String, String, Option<String>) {
    if matches!(site.status, ManagedSiteStatus::Failed)
        || site.parse_status == ManagedSiteParseStatus::Failed
        || site
            .last_error
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    {
        return (
            "failed".to_string(),
            "失败".to_string(),
            site.last_error
                .clone()
                .or(parse_detail)
                .or(db_detail)
                .or(web_detail),
        );
    }
    if parse_running
        && site.parse_status == ManagedSiteParseStatus::Parsed
        && matches!(site.status, ManagedSiteStatus::Starting)
    {
        return (
            "generating".to_string(),
            "模型生成中".to_string(),
            generate_detail.or(Some("模型生成进程正在运行".to_string())),
        );
    }
    if parse_running {
        return (
            "parsing".to_string(),
            "解析中".to_string(),
            parse_detail.or(Some("解析任务正在运行".to_string())),
        );
    }
    if matches!(site.status, ManagedSiteStatus::Starting) {
        let detail = if !db_running {
            db_detail.or(Some("等待数据库启动".to_string()))
        } else if site.parse_status != ManagedSiteParseStatus::Parsed {
            parse_detail.or(Some("等待解析完成".to_string()))
        } else if !web_running {
            web_detail.or(Some("等待站点服务启动".to_string()))
        } else {
            web_detail
        };
        return ("starting".to_string(), "启动中".to_string(), detail);
    }
    if matches!(site.status, ManagedSiteStatus::Stopping) {
        return (
            "stopping".to_string(),
            "停止中".to_string(),
            db_detail.or(web_detail).or(parse_detail),
        );
    }
    if web_running || matches!(site.status, ManagedSiteStatus::Running) {
        return (
            "running".to_string(),
            "运行中".to_string(),
            web_detail.or(Some("站点服务已可访问".to_string())),
        );
    }
    if site.parse_status == ManagedSiteParseStatus::Parsed && db_running {
        return (
            "parsed-db-ready".to_string(),
            "解析完成，数据库在线".to_string(),
            db_detail.or(parse_detail),
        );
    }
    if site.parse_status == ManagedSiteParseStatus::Parsed {
        return (
            "parsed".to_string(),
            "解析完成".to_string(),
            parse_detail.or(Some("解析结果已生成".to_string())),
        );
    }
    if matches!(site.status, ManagedSiteStatus::Stopped) {
        return (
            "stopped".to_string(),
            "已停止".to_string(),
            db_detail.or(web_detail).or(parse_detail),
        );
    }
    (
        "draft".to_string(),
        "待处理".to_string(),
        parse_detail.or(db_detail).or(web_detail),
    )
}

/// 单条日志类别的尾部读取（D5 / Sprint D · 修 G13）
///
/// 返回 `{ lines, total_lines, returned_lines, truncated }`：
/// - `lines`：文件最后 `limit` 行（按文件出现顺序，旧 → 新）
/// - `total_lines`：文件实际总行数
/// - `returned_lines`：本次返回行数
/// - `truncated`：当 `total_lines > returned_lines` 时为 true
///
/// 路径：runtime/admin_sites/<site_id>/logs/<kind>.log
/// `kind` 必须是 "parse" / "generate" / "db" / "web" / "viewer"。
pub fn tail_log(site_id: &str, kind: &str, limit: usize) -> Result<TailLogResponse> {
    let _ = get_site(site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
    let path = log_file_path(site_id, kind)?;
    let limit = limit.clamp(1, 5000);
    let (total_lines, lines) = read_tail_with_total(&path, limit);
    Ok(TailLogResponse {
        kind: kind.to_string(),
        path: path.to_string_lossy().to_string(),
        total_lines,
        returned_lines: lines.len(),
        truncated: total_lines > lines.len(),
        limit,
        lines,
    })
}

/// 单条日志类别的完整路径（D5 · 全量下载用）
pub fn full_log_path(site_id: &str, kind: &str) -> Result<PathBuf> {
    let _ = get_site(site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
    log_file_path(site_id, kind)
}

fn log_file_path(site_id: &str, kind: &str) -> Result<PathBuf> {
    match kind {
        "parse" | "generate" | "db" | "web" | "viewer" => {}
        other => bail!(
            "非法日志类型: {} (必须为 parse / generate / db / web / viewer)",
            other
        ),
    }
    let safe_id = sanitize_site_id_for_path(site_id);
    let mut p = PathBuf::from(ADMIN_RUNTIME_ROOT);
    p.push(safe_id);
    p.push("logs");
    p.push(format!("{}.log", kind));
    Ok(p)
}

fn read_tail_with_total(path: &Path, limit: usize) -> (usize, Vec<String>) {
    let file = match OpenOptions::new().read(true).open(path) {
        Ok(file) => file,
        Err(_) => return (0, Vec::new()),
    };
    let reader = BufReader::new(file);
    let lines: Vec<String> = reader.lines().map_while(Result::ok).collect();
    let total = lines.len();
    if total <= limit {
        (total, lines)
    } else {
        let tail = lines[total - limit..].to_vec();
        (total, tail)
    }
}

fn sanitize_site_id_for_path(site_id: &str) -> String {
    site_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

#[derive(Debug, serde::Serialize)]
pub struct TailLogResponse {
    pub kind: String,
    pub path: String,
    pub total_lines: usize,
    pub returned_lines: usize,
    pub truncated: bool,
    pub limit: usize,
    pub lines: Vec<String>,
}

pub fn logs(site_id: &str) -> Result<ManagedSiteLogsResponse> {
    let site = get_site(site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
    let snapshots = collect_log_snapshots(site_id);
    let parse_log = snapshots
        .iter()
        .find(|snapshot| snapshot.key == "parse")
        .map(|snapshot| snapshot.lines.clone())
        .unwrap_or_default();
    let generate_log = snapshots
        .iter()
        .find(|snapshot| snapshot.key == "generate")
        .map(|snapshot| snapshot.lines.clone())
        .unwrap_or_default();
    let db_log = snapshots
        .iter()
        .find(|snapshot| snapshot.key == "db")
        .map(|snapshot| snapshot.lines.clone())
        .unwrap_or_default();
    let web_log = snapshots
        .iter()
        .find(|snapshot| snapshot.key == "web")
        .map(|snapshot| snapshot.lines.clone())
        .unwrap_or_default();
    let viewer_log = snapshots
        .iter()
        .find(|snapshot| snapshot.key == "viewer")
        .map(|snapshot| snapshot.lines.clone())
        .unwrap_or_default();

    Ok(ManagedSiteLogsResponse {
        site_id: site.site_id,
        parse_log,
        generate_log,
        db_log,
        web_log,
        viewer_log,
        streams: snapshots
            .into_iter()
            .map(|snapshot| ManagedSiteLogStreamSummary {
                key: snapshot.key.to_string(),
                label: snapshot.label.to_string(),
                path: snapshot.path.to_string_lossy().to_string(),
                exists: snapshot.exists,
                has_content: snapshot.has_content,
                updated_at: snapshot.updated_at_rfc3339,
                line_count: snapshot.line_count,
                last_line: snapshot.last_line,
                last_key_log: snapshot.last_key_log,
            })
            .collect(),
    })
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_coerces_non_alnum_to_dashes() {
        assert_eq!(slugify("AvevaPlantSample"), "avevaplantsample");
        assert_eq!(slugify("My Project #1"), "my-project-1");
        assert_eq!(slugify(""), "site");
        assert_eq!(slugify("///"), "site");
        assert_eq!(slugify(".."), "site");
    }

    #[test]
    fn infer_site_id_is_filesystem_safe() {
        let id = infer_site_id("Evil/../Name", 8080);
        assert!(!id.contains(".."));
        assert!(!id.contains('/'));
    }

    #[test]
    fn split_project_root_handles_exact_match() {
        let (root, included, dirs) = split_project_root("Proj", "/data/models/Proj");
        assert_eq!(root, "/data/models");
        assert_eq!(included, vec!["Proj".to_string()]);
        assert_eq!(dirs, vec!["Proj".to_string()]);
    }

    #[test]
    fn split_project_root_handles_non_exact() {
        let (root, included, dirs) = split_project_root("Proj", "/data/models");
        assert_eq!(root, "/data/models");
        assert_eq!(included, vec!["Proj".to_string()]);
        assert_eq!(dirs, vec!["Proj".to_string()]);
    }

    #[test]
    fn manual_db_nums_normalize_sorts_and_dedups() {
        let got = normalize_manual_db_nums(vec![3, 1, 1, 2, 0, 2]);
        assert_eq!(got, vec![1, 2, 3]);
    }

    #[test]
    fn derive_entry_urls_prefers_public_base_url() {
        let (local, public, entry) = derive_entry_urls(
            8080,
            "0.0.0.0",
            &Some("https://ops.example.com/admin/".to_string()),
        );
        assert_eq!(local.as_deref(), Some("http://127.0.0.1:8080"));
        assert_eq!(public.as_deref(), Some("https://ops.example.com/admin"));
        assert_eq!(entry.as_deref(), Some("https://ops.example.com/admin"));
    }

    #[test]
    fn derive_entry_urls_falls_back_to_bind_host_when_public_missing() {
        let (local, public, entry) = derive_entry_urls(8080, "10.0.0.3", &None);
        assert_eq!(local.as_deref(), Some("http://127.0.0.1:8080"));
        assert_eq!(public.as_deref(), Some("http://10.0.0.3:8080"));
        assert_eq!(entry.as_deref(), Some("http://10.0.0.3:8080"));
    }

    #[test]
    fn strip_ansi_codes_removes_csi_and_osc() {
        assert_eq!(strip_ansi_codes("\u{1b}[31mhello\u{1b}[0m"), "hello");
        assert_eq!(strip_ansi_codes("\u{1b}]0;title\u{7}body"), "body");
    }
}
