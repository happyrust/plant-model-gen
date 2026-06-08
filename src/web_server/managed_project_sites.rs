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
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sysinfo::{
    CpuRefreshKind, Disks, MemoryRefreshKind, Pid, ProcessRefreshKind, ProcessesToUpdate, System,
};
use tokio::process::Command;
use tokio::task;

use super::models::{
    AdminResourceSummary, AppendManagedSiteDbFileRequest, AppendManagedSiteDbFileResponse,
    CreateManagedSiteRequest, DatabaseConfig, ManagedProjectSite, ManagedRemoteDeployRequest,
    ManagedRemoteDeployStatus, ManagedRemoteTarget, ManagedRemoteTargetOs,
    ManagedRemoteTargetRequest, ManagedSiteActivitySummary, ManagedSiteDbMode,
    ManagedSiteDeployValidationCheck, ManagedSiteDeployValidationReport,
    ManagedSiteLogStreamSummary, ManagedSiteLogsResponse, ManagedSiteParseHealth,
    ManagedSiteParseHealthStatus, ManagedSiteParsePlan, ManagedSiteParsePlanMode,
    ManagedSiteParseStatus, ManagedSitePreflightCheck, ManagedSitePreflightReport,
    ManagedSitePreflightStatus, ManagedSiteProcessResource, ManagedSiteReconcileResponse,
    ManagedSiteResourceMetrics, ManagedSiteRiskLevel, ManagedSiteRuntimeStatus, ManagedSiteStatus,
    ParsePlanFact, PreviewManagedSiteParsePlanRequest, ProjectRole, QuickDeployTestRequest,
    QuickDeployTestResponse, ScanProjectsResult, SiteProject, UpdateManagedSiteRequest,
};

// ─── Constants ──────────────────────────────────────────────────────────────

const DEFAULT_SQLITE_PATH: &str = "deployment_sites.sqlite";
const TABLE_NAME: &str = "managed_project_sites";
const REMOTE_TARGETS_TABLE: &str = "managed_remote_targets";
const REMOTE_DEPLOY_STATUS_TABLE: &str = "managed_remote_deploy_status";
/// 受管子进程登记表：记录每个站点各角色进程的 pid + 启动时刻 token，
/// 用于 kill 前做「同一进程」双重校验，规避 PID 复用导致的误杀。
const PROC_REGISTRY_TABLE: &str = "managed_site_processes";
const DB_DIR_OWNER_TABLE: &str = "managed_db_dir_owners";
const ADMIN_RUNTIME_ROOT: &str = "runtime/admin_sites";
const LOG_LINES_LIMIT: usize = 120;

static REMOTE_DEPLOY_PASSWORDS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

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
const DEFAULT_PARSE_DB_TYPES: &[&str] = &["SYST", "DESI", "CATA", "DICT", "GLB", "GLOB"];
const SUPPORTED_PARSE_DB_TYPES: &[&str] = &["SYST", "DESI", "CATA", "DICT", "GLB", "GLOB"];
const REPARSE_REUSE_DB_TYPES: &[&str] = &["SYST"];
/// 无条件预解析库：无论 auto_parse_related_dbnums 开关与否，都强制纳入解析。
/// DICT 字典/属性定义、GLOB/GLB 全局库都是建模与解析的通用依赖，故与 SYST 同级无条件预解析。
const MANDATORY_PREPARSE_DB_TYPES: &[&str] = &["DICT", "GLOB", "GLB"];
/// auto_parse_related_dbnums 开启时额外纳入的关联依赖库（精确解析针对 CATA 元件库；
/// DICT 已改为无条件预解析，不再受此开关控制）。
const RELATED_DEPENDENCY_DB_TYPES: &[&str] = &["CATA"];
const AUTO_DB_PORT_START: u16 = 8020;
const AUTO_WEB_PORT_START: u16 = 8080;
const AUTO_PORT_END: u16 = 8999;

// 运行时等待/杀进程超时。
const WAIT_PORT_ATTEMPTS: usize = 30;
const WAIT_HTTP_ATTEMPTS: usize = 40;
const WAIT_STEP_MS: u64 = 500;
const KILL_GRACE_MS: u64 = 1500;
const SIDECAR_CANCEL_WAIT_ATTEMPTS: usize = 10;
/// 停止互斥模式后等待端口释放的最大轮询次数（× WAIT_STEP_MS）。
const WAIT_PORT_FREE_ATTEMPTS: usize = 20;

// 磁盘占用缓存 TTL。
const PATH_SIZE_CACHE_TTL_MS: u64 = 60_000;

// Schema 版本号：每次迁移 +1。
const SCHEMA_VERSION: u32 = 8;

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

/// quick-deploy-test 免鉴权端点开关（默认关闭）。
///
/// 该端点不挂 admin 鉴权中间件，能创建站点 / 解析 / 生成 / 起进程 / 占端口；
/// 为避免在生产被未授权调用，默认禁用，仅在显式 `AIOS_ENABLE_QUICK_DEPLOY_TEST=1`
/// （或配置 `admin_enable_quick_deploy_test=true`）时放行，落实
/// `docs/plans/2026-05-31-one-click-deploy-test-plan.md` §6「仅 debug/测试暴露」。
pub fn quick_deploy_test_enabled() -> bool {
    if let Ok(value) = std::env::var("AIOS_ENABLE_QUICK_DEPLOY_TEST") {
        return matches!(value.trim(), "1" | "true" | "yes" | "on");
    }
    load_config_builder()
        .and_then(|builder| builder.get_bool("admin_enable_quick_deploy_test").ok())
        .unwrap_or(false)
}

/// 是否允许把明文 SSH 密码持久化到 SQLite（默认不允许）。
///
/// 默认仅把密码保留在进程内缓存，并在部署时优先从 `password_env` 读取，避免
/// 明文落库（遵 AGENTS.md：SSH 密码仅通过环境变量 / CI Secrets 提供）。如确需
/// 在测试环境落库，显式设置 `AIOS_ALLOW_SSH_PASSWORD_PERSIST=1`。
fn ssh_password_persist_allowed() -> bool {
    if let Ok(value) = std::env::var("AIOS_ALLOW_SSH_PASSWORD_PERSIST") {
        return matches!(value.trim(), "1" | "true" | "yes" | "on");
    }
    load_config_builder()
        .and_then(|builder| builder.get_bool("admin_allow_ssh_password_persist").ok())
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

fn generation_config_path(site_id: &str) -> PathBuf {
    site_runtime_dir(site_id).join("DbOption-generate.toml")
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

fn unique_site_name(base: &str, used_names: &HashSet<String>) -> String {
    let base = base.trim();
    if !used_names.contains(base) {
        return base.to_string();
    }
    for suffix in 2.. {
        let candidate = format!("{base}-{suffix}");
        if !used_names.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!("unbounded suffix search must return a candidate")
}

fn collect_site_names_with_conn(conn: &Connection) -> Result<HashSet<String>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT site_name FROM {table}",
        table = TABLE_NAME
    ))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut names = HashSet::new();
    for row in rows {
        names.insert(row?);
    }
    Ok(names)
}

fn site_name_exists_with_conn(conn: &Connection, site_name: &str) -> Result<bool> {
    Ok(collect_site_names_with_conn(conn)?.contains(site_name.trim()))
}

fn project_name_conflict_with_conn(
    conn: &Connection,
    project_name: &str,
    exclude_site_id: Option<&str>,
) -> Result<Option<String>> {
    let target = project_name.trim().to_lowercase();
    if target.is_empty() {
        return Ok(None);
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT site_id, project_name FROM {table}",
        table = TABLE_NAME
    ))?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (site_id, existing_project_name) = row?;
        if matches!(exclude_site_id, Some(excluded) if site_id == excluded) {
            continue;
        }
        if existing_project_name.trim().to_lowercase() == target {
            return Ok(Some(existing_project_name));
        }
    }
    Ok(None)
}

fn unique_site_name_with_conn(conn: &Connection, site_name: &str) -> Result<String> {
    let used_names = collect_site_names_with_conn(conn)?;
    Ok(unique_site_name(site_name, &used_names))
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

/// 受管站点 web 监听 `bind_host` 的默认值（仅当请求未显式指定时生效）。
///
/// - 设置 `AIOS_ALLOW_PUBLIC_BIND=1`：默认 `0.0.0.0`，允许跨机直连站点 web_port
///   （与 `assert_bind_host_safe` 的同一开关一致放行）。
/// - 未设置：回退 `127.0.0.1`，仅本机/Nginx 回环代理可达（安全默认，维持原行为）。
fn default_web_bind_host() -> String {
    if env_allow_public_bind() {
        "0.0.0.0".to_string()
    } else {
        "127.0.0.1".to_string()
    }
}

/// 同 `normalize_host`，但空值时回退到调用方指定的默认（而非硬编码 127.0.0.1）。
fn normalize_host_or(host: Option<String>, default: &str) -> String {
    host.map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
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

fn resolve_manual_db_nums<F>(
    manual_db_nums: Vec<u32>,
    manual_db_files: Vec<String>,
    mut resolve_db_file: F,
) -> Result<Vec<u32>>
where
    F: FnMut(&str) -> Result<u32>,
{
    let mut values = manual_db_nums;
    for db_file in manual_db_files {
        let db_file = db_file.trim();
        if db_file.is_empty() {
            continue;
        }
        values.push(resolve_db_file(db_file)?);
    }
    Ok(normalize_manual_db_nums(values))
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

fn projects_to_json(values: &[SiteProject]) -> Result<String> {
    Ok(serde_json::to_string(values)?)
}

fn projects_from_json(raw: Option<String>) -> Vec<SiteProject> {
    raw.and_then(|value| serde_json::from_str::<Vec<SiteProject>>(&value).ok())
        .unwrap_or_default()
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
fn normalize_project_names(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    for raw in values {
        for value in raw.split(|ch: char| ch == ',' || ch == ';' || ch.is_whitespace()) {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            let key = value.to_ascii_lowercase();
            if seen.insert(key) {
                names.push(value.to_string());
            }
        }
    }
    names
}

fn split_project_root(project_name: &str, raw_path: &str) -> (String, Vec<String>, Vec<String>) {
    split_project_root_multi(&[project_name.to_string()], raw_path)
}

fn split_project_root_multi(
    project_names: &[String],
    raw_path: &str,
) -> (String, Vec<String>, Vec<String>) {
    let project_names = normalize_project_names(project_names.iter().cloned());
    let project_names = if project_names.is_empty() {
        vec!["".to_string()]
    } else {
        project_names
    };
    let path = PathBuf::from(raw_path);
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if project_names.iter().any(|name| name == file_name) {
        let parent = path
            .parent()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| raw_path.to_string());
        return (parent, project_names.clone(), project_names);
    }
    (raw_path.to_string(), project_names.clone(), project_names)
}

/// 站点主工程（多工程模型）：优先 is_primary，其次首个 Design，再次第一个。
fn site_primary_project(site: &ManagedProjectSite) -> Option<&SiteProject> {
    site.projects
        .iter()
        .find(|p| p.is_primary)
        .or_else(|| {
            site.projects
                .iter()
                .find(|p| matches!(p.role, ProjectRole::Design))
        })
        .or_else(|| site.projects.first())
}

/// 返回站点内 (included_projects 名字, project_dirs 绝对路径)，按 sort_order 对齐并按名去重。
/// 绝对 project_dirs 配合 `DbOption::get_project_path` 的 `join` 语义可统一同根/跨根（见 dev-plan §10）。
fn site_included_projects_and_dirs(site: &ManagedProjectSite) -> (Vec<String>, Vec<String>) {
    let mut ordered: Vec<&SiteProject> = site.projects.iter().collect();
    ordered.sort_by_key(|p| p.sort_order);
    let mut names = Vec::new();
    let mut dirs = Vec::new();
    let mut seen = HashSet::new();
    for p in ordered {
        let name = p.name.trim();
        if name.is_empty() {
            continue;
        }
        if !seen.insert(name.to_ascii_lowercase()) {
            continue;
        }
        names.push(name.to_string());
        dirs.push(p.path.clone());
    }
    (names, dirs)
}

fn site_source_project_name(site: &ManagedProjectSite) -> String {
    if let Some(primary) = site_primary_project(site) {
        let name = primary.name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
    }
    site.associated_project
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| {
            normalize_project_names([value.to_string()])
                .into_iter()
                .next()
        })
        .or_else(|| {
            PathBuf::from(&site.project_path)
                .file_name()
                .and_then(|value| value.to_str())
                .map(ToOwned::to_owned)
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| site.project_name.clone())
}

fn site_parse_project_names(site: &ManagedProjectSite) -> Vec<String> {
    // 多工程模型：直接取显式 projects 列表（事实源）。
    let (names, _) = site_included_projects_and_dirs(site);
    if !names.is_empty() {
        return names;
    }

    // 回退（无显式 projects 的旧站点）：按 associated/path/project_name 派生。
    // 已删除 AvevaPlantSample→AvevaCatalogue 硬编码：元件库改由显式 role=library 工程驱动。
    let mut names = normalize_project_names(
        [
            site.associated_project.clone().unwrap_or_default(),
            site_source_project_name(site),
        ]
        .into_iter(),
    );
    let display_project_name = site.project_name.trim();
    if !display_project_name.is_empty()
        && project_dir_candidates(display_project_name, &site.project_path)
            .iter()
            .any(|path| path.exists())
    {
        names =
            normalize_project_names(names.into_iter().chain([display_project_name.to_string()]));
    }
    if names.is_empty() {
        names.push(site.project_name.clone());
    }
    names
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
        if let Some(parent) = raw.parent() {
            candidates.push(parent.join(project_name));
        }
        candidates.push(raw.clone());
    }
    candidates
}

fn project_dir_candidates_multi(project_names: &[String], raw_path: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    for project_name in project_names {
        for path in project_dir_candidates(project_name, raw_path) {
            let key = path.to_string_lossy().to_ascii_lowercase();
            if seen.insert(key) {
                candidates.push(path);
            }
        }
    }
    candidates
}

fn existing_project_roots(project_names: &[String], raw_path: &str) -> Result<Vec<PathBuf>> {
    let roots = project_dir_candidates_multi(project_names, raw_path)
        .into_iter()
        .filter(|path| path.exists())
        .collect::<Vec<_>>();
    if roots.is_empty() {
        bail!("项目路径不存在: {}", raw_path);
    }
    Ok(roots)
}

/// 校验并规范化站点工程列表（T2.1）：逐条过白名单 + canonicalize，断言约束。
/// 返回规范化后的 projects（path 替换为 canonical 绝对路径，name 缺省取目录名）。
fn validate_and_canonicalize_projects(projects: &[SiteProject]) -> Result<Vec<SiteProject>> {
    if projects.is_empty() {
        bail!("至少需要一个工程");
    }
    let mut out = Vec::with_capacity(projects.len());
    let mut design_count = 0usize;
    let mut primary_count = 0usize;
    let mut seen_names = HashSet::new();
    for p in projects {
        let canonical = canonical_project_path(p.path.trim())?;
        let name = if p.name.trim().is_empty() {
            canonical
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_string()
        } else {
            p.name.trim().to_string()
        };
        if name.is_empty() {
            bail!("工程名不能为空: {}", canonical.display());
        }
        if !seen_names.insert(name.to_ascii_lowercase()) {
            bail!("工程名重复: {name}（同站点内工程名必须唯一）");
        }
        if matches!(p.role, ProjectRole::Design) {
            design_count += 1;
        }
        if p.is_primary {
            primary_count += 1;
        }
        out.push(SiteProject {
            path: canonical.to_string_lossy().to_string(),
            name,
            role: p.role,
            is_primary: p.is_primary,
            sort_order: p.sort_order,
        });
    }
    if design_count == 0 {
        bail!("至少需要一个 design 工程");
    }
    if primary_count != 1 {
        bail!("必须恰好指定一个主工程(primary)，当前为 {primary_count} 个");
    }
    Ok(out)
}

/// dbnum 冲突预检由 parse sidecar 负责。
///
/// `web_server` 不再扫描工程目录或读取 DB 文件头；这里只保留调用点的
/// 配置级占位，避免控制面重新获得 E3D 数据读取职责。
fn precheck_dbnum_conflicts(projects: &[SiteProject]) -> Result<()> {
    let _ = projects;
    Ok(())
}

pub fn scan_projects_under_root(raw_root: &str) -> Result<ScanProjectsResult> {
    let _ = &raw_root;
    bail!("web_server 不再扫描 E3D 工程；请调用 aios-database sidecar /projects/scan")
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

fn manual_db_only_scope(
    site: &ManagedProjectSite,
    parse_db_types: &[String],
    force_rebuild_system_db: bool,
) -> bool {
    !site.manual_db_nums.is_empty()
        && parse_db_types.is_empty()
        && !site.auto_parse_related_dbnums
        && !force_rebuild_system_db
}

fn generation_enabled(site: &ManagedProjectSite) -> bool {
    site.gen_model || site.gen_mesh || site.gen_spatial_tree
}

fn site_generate_db_nums(site: &ManagedProjectSite) -> Vec<u32> {
    if site.generate_db_nums.is_empty() {
        site.manual_db_nums.clone()
    } else {
        site.generate_db_nums.clone()
    }
}

/// 站点所有工程的实际根目录（多工程模型用 projects[].path 绝对路径；空则回退旧派生）。
fn site_existing_project_roots(site: &ManagedProjectSite) -> Result<Vec<PathBuf>> {
    if !site.projects.is_empty() {
        let mut ordered: Vec<&SiteProject> = site.projects.iter().collect();
        ordered.sort_by_key(|p| p.sort_order);
        let roots: Vec<PathBuf> = ordered
            .into_iter()
            .map(|p| PathBuf::from(&p.path))
            .filter(|path| path.exists())
            .collect();
        if !roots.is_empty() {
            return Ok(roots);
        }
    }
    existing_project_roots(&site_parse_project_names(site), &site.project_path)
}

/// primary 工程根目录（多工程模型用 primary.path；空则回退旧派生）。
fn site_primary_existing_roots(site: &ManagedProjectSite) -> Result<Vec<PathBuf>> {
    if let Some(primary) = site_primary_project(site) {
        let path = PathBuf::from(&primary.path);
        if path.exists() {
            return Ok(vec![path]);
        }
    }
    existing_project_roots(
        &[site_source_project_name(site), site.project_name.clone()],
        &site.project_path,
    )
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
    auto_related_db_files: Vec<String>,
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
            auto_related_db_files,
            entries: Vec::new(),
            warnings: Vec::new(),
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
                auto_related_db_files,
                entries: Vec::new(),
                warnings: Vec::new(),
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
                auto_related_db_files,
                entries: Vec::new(),
                warnings: Vec::new(),
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
            auto_related_db_files,
            entries: Vec::new(),
            warnings: Vec::new(),
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
            auto_related_db_files,
            entries: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

fn build_parse_plan(site: &ManagedProjectSite) -> ManagedSiteParsePlan {
    // 持久化口径：included_db_files 从已写入的 parse 配置读取，不重算依赖闭包，
    // auto_related_db_files 留空（依赖明细仅在预览路径实时计算）。
    let mut plan = build_parse_plan_with_files(
        site,
        read_parse_config_included_db_files(&site.site_id),
        Vec::new(),
    );
    hydrate_parse_plan_from_manifest(&site.site_id, &mut plan);
    plan
}

fn hydrate_parse_plan_from_manifest(site_id: &str, plan: &mut ManagedSiteParsePlan) {
    if plan.included_db_files.is_empty() {
        return;
    }
    let path = parse_plan_manifest_path(site_id);
    let Ok(raw) = fs::read_to_string(&path) else {
        return;
    };
    let Ok(manifest) = serde_json::from_str::<ParsePlanManifest>(&raw) else {
        return;
    };
    let included: HashSet<String> = plan.included_db_files.iter().cloned().collect();
    let manifest_files: HashSet<String> = manifest.included_db_files.iter().cloned().collect();
    if !included.is_subset(&manifest_files) {
        return;
    }
    plan.entries = manifest
        .entries
        .into_iter()
        .filter(|entry| included.contains(&entry.file_name))
        .collect();
    plan.warnings = manifest.warnings;
    plan.auto_related_db_files = manifest
        .auto_related_db_files
        .into_iter()
        .filter(|file| included.contains(file))
        .collect();
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

fn managed_db_mode_to_str(mode: ManagedSiteDbMode) -> &'static str {
    match mode {
        ManagedSiteDbMode::File => "file",
        ManagedSiteDbMode::Ws => "ws",
    }
}

fn db_mode_from_string(value: Option<String>, default: ManagedSiteDbMode) -> ManagedSiteDbMode {
    match value
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("ws" | "websocket") => ManagedSiteDbMode::Ws,
        Some("file" | "rocksdb" | "local") => ManagedSiteDbMode::File,
        _ => default,
    }
}

fn apply_site_db_mode_config(
    table: &mut toml::value::Table,
    site: &ManagedProjectSite,
    db_user: &str,
    db_password: &str,
    mode: ManagedSiteDbMode,
) {
    let web_server = ensure_table(table, "web_server");
    set_toml_bool(
        web_server,
        "auto_start_surreal",
        mode == ManagedSiteDbMode::Ws,
    );
    set_toml_string(web_server, "surreal_bin", managed_surreal_bin_string());
    set_toml_string(web_server, "surreal_data_path", site.db_data_path.clone());
    set_toml_string(
        web_server,
        "surreal_bind",
        format!("127.0.0.1:{}", site.db_port),
    );
    set_toml_string(web_server, "surreal_user", db_user.to_string());
    set_toml_string(web_server, "surreal_password", db_password.to_string());

    let surrealdb = ensure_table(table, "surrealdb");
    set_toml_string(surrealdb, "mode", managed_db_mode_to_str(mode));
    set_toml_string(surrealdb, "path", site.db_data_path.replace('\\', "/"));
    match mode {
        ManagedSiteDbMode::Ws => {
            set_toml_string(surrealdb, "ip", "127.0.0.1");
            set_toml_integer(surrealdb, "port", site.db_port as i64);
            set_toml_string(surrealdb, "user", db_user.to_string());
            set_toml_string(surrealdb, "password", db_password.to_string());
        }
        ManagedSiteDbMode::File => {
            surrealdb.remove("ip");
            surrealdb.remove("port");
            surrealdb.remove("user");
            surrealdb.remove("password");
        }
    }
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

fn set_or_remove_manual_db_nums(table: &mut toml::value::Table, values: Vec<u32>) {
    if values.is_empty() {
        table.remove("manual_db_nums");
    } else {
        set_toml_integer_array(table, "manual_db_nums", values);
    }
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
    let parse_project_names = site_parse_project_names(site);
    let runtime_cfg = DatabaseConfig {
        project_name: source_project_name.clone(),
        project_path: site.project_path.clone(),
        project_code: site.project_code,
        manual_db_nums: site_generate_db_nums(site),
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
    let (project_root, included_projects, project_dirs) = if !site.projects.is_empty() {
        // 多工程模型：included_projects=工程名、project_dirs=canonical 绝对路径（统一同根/跨根，见 dev-plan §10）。
        let (names, dirs) = site_included_projects_and_dirs(site);
        let primary_root = site_primary_project(site)
            .map(|p| {
                PathBuf::from(&p.path)
                    .parent()
                    .map(|parent| parent.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.path.clone())
            })
            .unwrap_or_else(|| site.project_path.clone());
        (primary_root, names, dirs)
    } else {
        split_project_root_multi(&parse_project_names, &site.project_path)
    };

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
    set_toml_string(table, "surreal_script_dir", resolve_surreal_script_dir());
    let output_root = site_runtime_dir(&site.site_id)
        .join("output")
        .to_string_lossy()
        .replace('\\', "/");
    set_toml_string(table, "output_root", output_root);
    table.remove("v_ip");
    table.remove("v_port");
    table.remove("v_user");
    table.remove("v_password");
    set_toml_array(table, "included_projects", included_projects);
    set_toml_array(table, "project_dirs", project_dirs);
    set_or_remove_manual_db_nums(table, runtime_cfg.manual_db_nums.clone());
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
    set_toml_string(
        web_server,
        "site_name",
        if site.site_name.trim().is_empty() {
            site.project_name.clone()
        } else {
            site.site_name.clone()
        },
    );
    set_toml_string(web_server, "region", "admin");
    let (local_url, _public_opt, effective_url) =
        derive_entry_urls(site.web_port, &site.bind_host, &site.public_base_url);
    let effective = effective_url.unwrap_or_else(|| local_url.clone().unwrap_or_default());
    let local = local_url.unwrap_or_default();
    set_toml_string(web_server, "frontend_url", effective.clone());
    set_toml_string(web_server, "public_base_url", effective);
    set_toml_string(web_server, "backend_url", local);
    set_toml_bool(web_server, "auto_start_surreal", false);
    set_toml_string(web_server, "surreal_bin", managed_surreal_bin_string());
    set_toml_string(web_server, "surreal_data_path", site.db_data_path.clone());
    set_toml_string(
        web_server,
        "surreal_bind",
        format!("127.0.0.1:{}", site.db_port),
    );
    apply_site_db_mode_config(table, site, db_user, db_password, ManagedSiteDbMode::Ws);

    let surrealkv = ensure_table(table, "surrealkv");
    set_toml_bool(surrealkv, "enabled", false);
    set_toml_string(
        surrealkv,
        "path",
        format!("{}.kv", site.db_data_path.replace('\\', "/")),
    );

    Ok(toml::to_string_pretty(&value)?)
}

fn build_parse_config_with_included_files(
    site: &ManagedProjectSite,
    db_user: &str,
    db_password: &str,
    included_db_files: &[String],
) -> Result<String> {
    let content = build_site_config(site, db_user, db_password)?;
    let mut value = toml::from_str::<toml::Value>(&content)?;
    let table = value
        .as_table_mut()
        .ok_or_else(|| anyhow!("DbOption 解析配置不是 table 结构"))?;
    table.remove("web_server");
    apply_site_db_mode_config(table, site, db_user, db_password, site.pipeline_db_mode);
    table.remove("web_server");
    set_or_remove_manual_db_nums(table, site.manual_db_nums.clone());
    set_toml_bool(table, "total_sync", true);
    set_toml_bool(table, "incr_sync", false);
    set_toml_bool(table, "sync_history", false);
    set_toml_bool(table, "only_sync_sys", false);
    set_toml_bool(table, "gen_tree_only", false);
    set_toml_bool(table, "enable_log", true);
    set_toml_bool(table, "save_db", true);
    if included_db_files.is_empty() {
        table.remove("included_db_files");
    } else {
        set_toml_array(table, "included_db_files", included_db_files.to_vec());
    }
    Ok(toml::to_string_pretty(&value)?)
}

fn build_parse_config(
    site: &ManagedProjectSite,
    db_user: &str,
    db_password: &str,
) -> Result<String> {
    // 配置文件仍由 web_server 负责写入，但解析事实不能由 web_server 扫描 E3D/DB
    // 得出。这里仅复用已经写入配置的 included_db_files；解析启动前会由
    // aios-database sidecar 刷新这份事实。
    let included_db_files = read_parse_config_included_db_files(&site.site_id);
    build_parse_config_with_included_files(site, db_user, db_password, &included_db_files)
}

fn build_generation_config(
    site: &ManagedProjectSite,
    db_user: &str,
    db_password: &str,
) -> Result<String> {
    let content = build_site_config(site, db_user, db_password)?;
    let mut value = toml::from_str::<toml::Value>(&content)?;
    let table = value
        .as_table_mut()
        .ok_or_else(|| anyhow!("DbOption 生成配置不是 table 结构"))?;
    // 解析和模型生成使用同一套管线 DB 模式；plant3d-web 运行配置始终使用 ws。
    apply_site_db_mode_config(table, site, db_user, db_password, site.pipeline_db_mode);
    table.remove("web_server");
    set_or_remove_manual_db_nums(table, site_generate_db_nums(site));
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
    write_site_files_with_parse_plan(site, db_user, db_password, None)
}

fn write_site_files_with_parse_plan(
    site: &ManagedProjectSite,
    db_user: &str,
    db_password: &str,
    parse_plan: Option<&ManagedSiteParsePlan>,
) -> Result<()> {
    ensure_runtime_dirs(&site.site_id)?;
    let content = build_site_config(site, db_user, db_password)?;
    write_file_atomic(Path::new(&site.config_path), &content)?;
    let included_db_files = parse_plan
        .map(|plan| plan.included_db_files.clone())
        .unwrap_or_else(|| read_parse_config_included_db_files(&site.site_id));
    let parse_content =
        build_parse_config_with_included_files(site, db_user, db_password, &included_db_files)?;
    write_file_atomic(&parse_config_path(&site.site_id), &parse_content)?;
    if let Some(plan) = parse_plan {
        write_parse_plan_manifest(site, plan)?;
    }
    let generation_content = build_generation_config(site, db_user, db_password)?;
    write_file_atomic(&generation_config_path(&site.site_id), &generation_content)?;
    let metadata = serde_json::to_string_pretty(&json!({
        "site_id": site.site_id,
        "project_name": site.project_name,
        "project_code": site.project_code,
        "project_path": site.project_path,
        "manual_db_nums": site.manual_db_nums,
        "generate_db_nums": site.generate_db_nums,
        "parse_db_types": site.parse_db_types,
        "force_rebuild_system_db": site.force_rebuild_system_db,
        "auto_parse_related_dbnums": site.auto_parse_related_dbnums,
        "gen_model": site.gen_model,
        "gen_mesh": site.gen_mesh,
        "gen_spatial_tree": site.gen_spatial_tree,
        "apply_boolean_operation": site.apply_boolean_operation,
        "mesh_tol_ratio": site.mesh_tol_ratio,
        "export_json": site.export_json,
        "export_parquet": site.export_parquet,
        "pipeline_db_mode": managed_db_mode_to_str(site.pipeline_db_mode),
        "runtime_db_mode": managed_db_mode_to_str(site.runtime_db_mode),
        "db_port": site.db_port,
        "web_port": site.web_port,
        "entry_url": site.entry_url,
        "output_root": site_runtime_dir(&site.site_id).join("output").to_string_lossy().replace('\\', "/"),
        "updated_at": site.updated_at,
    }))?;
    write_file_atomic(&metadata_path(&site.site_id), &metadata)?;
    Ok(())
}

fn parse_plan_manifest_path(site_id: &str) -> PathBuf {
    site_runtime_dir(site_id).join("parse-plan-manifest.json")
}

#[derive(Debug, Serialize, Deserialize)]
struct ParsePlanManifest {
    schema_version: u32,
    generated_at: String,
    inputs_hash: String,
    sidecar_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    db_index: Option<ParsePlanManifestDbIndex>,
    mode: ManagedSiteParsePlanMode,
    label: String,
    detail: String,
    includes_system_db_files: bool,
    included_db_files: Vec<String>,
    auto_related_db_files: Vec<String>,
    entries: Vec<ParsePlanFact>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ParsePlanManifestDbIndex {
    role: String,
    path: String,
    inputs_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    preview_path: Option<String>,
    promoted_from_preview: bool,
}

fn parse_plan_inputs_hash(site: &ManagedProjectSite) -> Result<String> {
    let input = json!({
        "project_name": &site.project_name,
        "project_path": &site.project_path,
        "projects": &site.projects,
        "manual_db_nums": &site.manual_db_nums,
        "manual_db_files": [],
        "parse_db_types": &site.parse_db_types,
        "force_rebuild_system_db": site.force_rebuild_system_db,
        "auto_parse_related_dbnums": site.auto_parse_related_dbnums,
    });
    let raw = serde_json::to_vec(&input)?;
    Ok(hex::encode(Sha256::digest(raw)))
}

#[cfg(feature = "sqlite-index")]
fn preview_db_index_path(inputs_hash: &str) -> Result<PathBuf> {
    Ok(repo_root()?
        .join("runtime")
        .join("preview-index")
        .join(inputs_hash)
        .join(crate::data_interface::db_index::DB_INDEX_FILE_NAME))
}

fn parse_plan_manifest_db_index(
    site_id: &str,
    inputs_hash: &str,
) -> Result<Option<ParsePlanManifestDbIndex>> {
    #[cfg(feature = "sqlite-index")]
    {
        let site_index_path = site_db_index_path(site_id);
        let preview_index_path = preview_db_index_path(inputs_hash)?;
        let mut promoted_from_preview = false;
        if preview_index_path.is_file() {
            if let Some(parent) = site_index_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("创建正式 db_index 目录失败: {}", parent.display()))?;
            }
            fs::copy(&preview_index_path, &site_index_path).with_context(|| {
                format!(
                    "提升 preview db_index 失败: {} -> {}",
                    preview_index_path.display(),
                    site_index_path.display()
                )
            })?;
            promoted_from_preview = true;
        }
        Ok(Some(ParsePlanManifestDbIndex {
            role: "site_runtime".to_string(),
            path: site_index_path.to_string_lossy().replace('\\', "/"),
            inputs_hash: inputs_hash.to_string(),
            preview_path: preview_index_path
                .is_file()
                .then(|| preview_index_path.to_string_lossy().replace('\\', "/")),
            promoted_from_preview,
        }))
    }
    #[cfg(not(feature = "sqlite-index"))]
    {
        let _ = site_id;
        let _ = inputs_hash;
        Ok(None)
    }
}

fn write_parse_plan_manifest(site: &ManagedProjectSite, plan: &ManagedSiteParsePlan) -> Result<()> {
    let inputs_hash = parse_plan_inputs_hash(site)?;
    let manifest = ParsePlanManifest {
        schema_version: 1,
        generated_at: now_rfc3339(),
        db_index: parse_plan_manifest_db_index(&site.site_id, &inputs_hash)?,
        inputs_hash,
        sidecar_version: env!("CARGO_PKG_VERSION").to_string(),
        mode: plan.mode.clone(),
        label: plan.label.clone(),
        detail: plan.detail.clone(),
        includes_system_db_files: plan.includes_system_db_files,
        included_db_files: plan.included_db_files.clone(),
        auto_related_db_files: plan.auto_related_db_files.clone(),
        entries: plan.entries.clone(),
        warnings: plan.warnings.clone(),
    };
    let raw = serde_json::to_string_pretty(&manifest)?;
    write_file_atomic(&parse_plan_manifest_path(&site.site_id), &raw)
}

fn preview_request_from_site(site: &ManagedProjectSite) -> PreviewManagedSiteParsePlanRequest {
    PreviewManagedSiteParsePlanRequest {
        site_id: Some(site.site_id.clone()),
        site_name: Some(site.site_name.clone()),
        projects: site.projects.clone(),
        project_name: site.project_name.clone(),
        project_path: site.project_path.clone(),
        manual_db_nums: site.manual_db_nums.clone(),
        manual_db_files: Vec::new(),
        generate_db_nums: site_generate_db_nums(site),
        generate_db_files: Vec::new(),
        parse_db_types: site.parse_db_types.clone(),
        force_rebuild_system_db: site.force_rebuild_system_db,
        auto_parse_related_dbnums: site.auto_parse_related_dbnums,
        web_port: site.web_port,
        bind_host: Some(site.bind_host.clone()),
        public_base_url: site.public_base_url.clone(),
        associated_project: site.associated_project.clone(),
    }
}

async fn load_parse_plan_from_sidecar(site: &ManagedProjectSite) -> Result<ManagedSiteParsePlan> {
    let value = crate::web_server::parse_sidecar_client::preview_parse_plan(
        preview_request_from_site(site),
    )
    .await
    .map_err(|err| anyhow!("aios-database sidecar 解析计划失败: {}", err.message))?;
    serde_json::from_value(value).context("解析 sidecar parse plan 响应失败")
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

fn is_unspecified_or_loopback_host(host: &str) -> bool {
    let normalized = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "" | "0.0.0.0" | "::" | "127.0.0.1" | "localhost" | "::1"
    )
}

fn url_host(host: &str) -> String {
    let trimmed = host.trim().trim_start_matches('[').trim_end_matches(']');
    if trimmed.contains(':') {
        format!("[{trimmed}]")
    } else {
        trimmed.to_string()
    }
}

fn site_probe_host(site: &ManagedProjectSite) -> String {
    if is_unspecified_or_loopback_host(&site.bind_host) {
        "127.0.0.1".to_string()
    } else {
        site.bind_host.trim().to_string()
    }
}

fn site_probe_base_url(site: &ManagedProjectSite) -> String {
    let host = site_probe_host(site);
    format!("http://{}:{}", url_host(&host), site.web_port)
}

fn site_access_base_url(site: &ManagedProjectSite) -> String {
    site.public_entry_url
        .clone()
        .or_else(|| site.entry_url.clone())
        .unwrap_or_else(|| site_probe_base_url(site))
}

// ─── Row mapping ────────────────────────────────────────────────────────────

fn row_bool_or(row: &rusqlite::Row<'_>, column: &str, default: bool) -> bool {
    row.get::<_, Option<i64>>(column)
        .ok()
        .flatten()
        .map(|value| value != 0)
        .unwrap_or(default)
}

fn row_bool_index(row: &rusqlite::Row<'_>, index: usize, default: bool) -> bool {
    row.get::<_, Option<i64>>(index)
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
        site_name: row
            .get::<_, Option<String>>("site_name")
            .unwrap_or(None)
            .unwrap_or_default(),
        project_name: row.get("project_name")?,
        project_code: row.get::<_, i64>("project_code")? as u32,
        project_path: row.get("project_path")?,
        projects: projects_from_json(row.get("projects_json").unwrap_or(None)),
        manual_db_nums: manual_db_nums_from_json(row.get("manual_db_nums")?),
        generate_db_nums: manual_db_nums_from_json(row.get("generate_db_nums").unwrap_or(None)),
        parse_db_types: parse_db_types_from_json(row.get("parse_db_types").unwrap_or(None)),
        force_rebuild_system_db: row
            .get::<_, Option<i64>>("force_rebuild_system_db")
            .unwrap_or(None)
            .unwrap_or(0)
            != 0,
        auto_parse_related_dbnums: row_bool_or(row, "auto_parse_related_dbnums", false),
        gen_model: row_bool_or(row, "gen_model", true),
        gen_mesh: row_bool_or(row, "gen_mesh", false),
        gen_spatial_tree: row_bool_or(row, "gen_spatial_tree", true),
        apply_boolean_operation: row_bool_or(row, "apply_boolean_operation", true),
        mesh_tol_ratio: row_f64_or(row, "mesh_tol_ratio", 3.0),
        export_json: row_bool_or(row, "export_json", false),
        export_parquet: row_bool_or(row, "export_parquet", true),
        pipeline_db_mode: db_mode_from_string(
            row.get("pipeline_db_mode").unwrap_or(None),
            ManagedSiteDbMode::Ws,
        ),
        runtime_db_mode: db_mode_from_string(
            row.get("runtime_db_mode").unwrap_or(None),
            ManagedSiteDbMode::Ws,
        ),
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
            site_name TEXT,
            projects_json TEXT NOT NULL DEFAULT '[]',
            manual_db_nums TEXT NOT NULL DEFAULT '[]',
            generate_db_nums TEXT NOT NULL DEFAULT '[]',
            parse_db_types TEXT NOT NULL DEFAULT '["SYST","DESI","CATA","DICT","GLB","GLOB"]',
            force_rebuild_system_db INTEGER NOT NULL DEFAULT 0,
            auto_parse_related_dbnums INTEGER NOT NULL DEFAULT 0,
            gen_model INTEGER NOT NULL DEFAULT 1,
            gen_mesh INTEGER NOT NULL DEFAULT 0,
            gen_spatial_tree INTEGER NOT NULL DEFAULT 1,
            apply_boolean_operation INTEGER NOT NULL DEFAULT 1,
            mesh_tol_ratio REAL NOT NULL DEFAULT 3.0,
            export_json INTEGER NOT NULL DEFAULT 0,
            export_parquet INTEGER NOT NULL DEFAULT 1,
            pipeline_db_mode TEXT NOT NULL DEFAULT 'ws',
            runtime_db_mode TEXT NOT NULL DEFAULT 'ws',
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
        CREATE UNIQUE INDEX IF NOT EXISTS idx_managed_project_sites_db_port ON {table}(db_port);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_managed_project_sites_web_port ON {table}(web_port);
        "#,
        table = TABLE_NAME
    ))?;
    conn.execute_batch(&format!(
        r#"
        CREATE TABLE IF NOT EXISTS {remote_targets} (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            target_os TEXT NOT NULL DEFAULT 'ubuntu22',
            host TEXT NOT NULL,
            ssh_port INTEGER NOT NULL,
            ssh_user TEXT NOT NULL,
            password_env TEXT NOT NULL,
            ssh_password TEXT,
            remote_root TEXT NOT NULL,
            remote_db_path TEXT NOT NULL,
            remote_web_port INTEGER NOT NULL,
            remote_db_port INTEGER NOT NULL,
            public_base_url TEXT,
            surreal_bin TEXT NOT NULL,
            remote_web_bin TEXT NOT NULL DEFAULT '/root/web_server',
            auto_prepare INTEGER NOT NULL DEFAULT 1,
            upload_web_server INTEGER NOT NULL DEFAULT 0,
            upload_surreal INTEGER NOT NULL DEFAULT 0,
            upload_resource INTEGER NOT NULL DEFAULT 0,
            upload_viewer INTEGER NOT NULL DEFAULT 0,
            open_firewall INTEGER NOT NULL DEFAULT 1,
            allowed_cidrs_json TEXT NOT NULL DEFAULT '["0.0.0.0/0"]',
            web_bind_host TEXT NOT NULL DEFAULT '0.0.0.0',
            db_bind_host TEXT NOT NULL DEFAULT '127.0.0.1',
            local_web_bin TEXT,
            local_surreal_bin TEXT,
            local_resource_dir TEXT,
            local_viewer_dir TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS {remote_status} (
            site_id TEXT PRIMARY KEY,
            target_id TEXT NOT NULL,
            deploy_id TEXT,
            deploy_task_id TEXT,
            deployment_mode TEXT,
            degraded INTEGER NOT NULL DEFAULT 0,
            status TEXT NOT NULL,
            current_step TEXT NOT NULL,
            remote_entry_url TEXT,
            checked_at TEXT NOT NULL,
            last_error TEXT,
            checks_json TEXT NOT NULL DEFAULT '[]'
        );
        "#,
        remote_targets = REMOTE_TARGETS_TABLE,
        remote_status = REMOTE_DEPLOY_STATUS_TABLE
    ))?;
    ensure_table_column_exists(
        conn,
        REMOTE_TARGETS_TABLE,
        "remote_web_bin",
        "TEXT NOT NULL DEFAULT '/root/web_server'",
    )?;
    for (column, column_type) in [
        ("target_os", "TEXT NOT NULL DEFAULT 'ubuntu22'"),
        ("ssh_password", "TEXT"),
        ("auto_prepare", "INTEGER NOT NULL DEFAULT 1"),
        ("upload_web_server", "INTEGER NOT NULL DEFAULT 0"),
        ("upload_surreal", "INTEGER NOT NULL DEFAULT 0"),
        ("upload_resource", "INTEGER NOT NULL DEFAULT 0"),
        ("upload_viewer", "INTEGER NOT NULL DEFAULT 0"),
        ("open_firewall", "INTEGER NOT NULL DEFAULT 1"),
        (
            "allowed_cidrs_json",
            "TEXT NOT NULL DEFAULT '[\"0.0.0.0/0\"]'",
        ),
        ("web_bind_host", "TEXT NOT NULL DEFAULT '0.0.0.0'"),
        ("db_bind_host", "TEXT NOT NULL DEFAULT '127.0.0.1'"),
        ("local_web_bin", "TEXT"),
        ("local_surreal_bin", "TEXT"),
        ("local_resource_dir", "TEXT"),
        ("local_viewer_dir", "TEXT"),
    ] {
        ensure_table_column_exists(conn, REMOTE_TARGETS_TABLE, column, column_type)?;
    }
    for (column, column_type) in [
        ("deploy_id", "TEXT"),
        ("deploy_task_id", "TEXT"),
        ("deployment_mode", "TEXT"),
        ("degraded", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        ensure_table_column_exists(conn, REMOTE_DEPLOY_STATUS_TABLE, column, column_type)?;
    }

    // 受管子进程登记表（PID + 启动时刻 token），用于 kill 前双重校验防误杀。
    conn.execute_batch(&format!(
        r#"
        CREATE TABLE IF NOT EXISTS {proc_table} (
            site_id TEXT NOT NULL,
            role TEXT NOT NULL,
            pid INTEGER NOT NULL,
            start_token INTEGER,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (site_id, role)
        );
        "#,
        proc_table = PROC_REGISTRY_TABLE
    ))?;

    // file/ws 互斥真源登记表：记录"谁打开了某个 db_data_path 的 RocksDB"。
    // Phase 1 仅建表 + CRUD，尚未接入启动/退出路径（Phase 3 接入）。
    conn.execute_batch(&format!(
        r#"
        CREATE TABLE IF NOT EXISTS {owner_table} (
            data_dir TEXT PRIMARY KEY,
            site_id TEXT NOT NULL,
            owner_pid INTEGER NOT NULL,
            mode TEXT NOT NULL,
            role TEXT NOT NULL,
            start_token INTEGER,
            updated_at TEXT NOT NULL
        );
        "#,
        owner_table = DB_DIR_OWNER_TABLE
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
    if current_version < 7 {
        for column in ["pipeline_db_mode", "runtime_db_mode"] {
            ensure_column_exists(conn, column)?;
        }
        current_version = 7;
        conn.pragma_update(None, "user_version", current_version as i64)?;
    }
    if current_version < 8 {
        current_version = 8;
        conn.pragma_update(None, "user_version", current_version as i64)?;
    }

    // 多工程合并站点升级（幂等列 + 索引切换，非版本门控；全新升级首跑前可直接删库重建）
    ensure_column_exists(conn, "site_name")?;
    ensure_column_exists(conn, "projects_json")?;
    ensure_column_exists(conn, "auto_parse_related_dbnums")?;
    ensure_column_exists(conn, "generate_db_nums")?;
    conn.execute(
        "DROP INDEX IF EXISTS idx_managed_project_sites_project_name",
        [],
    )?;
    conn.execute(
        &format!(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_managed_project_sites_site_name ON {table}(site_name)",
            table = TABLE_NAME
        ),
        [],
    )?;

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
            "manual_db_nums" | "generate_db_nums" => "TEXT NOT NULL DEFAULT '[]'",
            "parse_db_types" => {
                "TEXT NOT NULL DEFAULT '[\"SYST\",\"DESI\",\"CATA\",\"DICT\",\"GLB\",\"GLOB\"]'"
            }
            "force_rebuild_system_db" => "INTEGER NOT NULL DEFAULT 0",
            "auto_parse_related_dbnums" => "INTEGER NOT NULL DEFAULT 0",
            "gen_model" => "INTEGER NOT NULL DEFAULT 1",
            "gen_mesh" => "INTEGER NOT NULL DEFAULT 0",
            "gen_spatial_tree" => "INTEGER NOT NULL DEFAULT 1",
            "apply_boolean_operation" => "INTEGER NOT NULL DEFAULT 1",
            "mesh_tol_ratio" => "REAL NOT NULL DEFAULT 3.0",
            "export_json" => "INTEGER NOT NULL DEFAULT 0",
            "export_parquet" => "INTEGER NOT NULL DEFAULT 1",
            "pipeline_db_mode" => "TEXT NOT NULL DEFAULT 'ws'",
            "runtime_db_mode" => "TEXT NOT NULL DEFAULT 'ws'",
            "projects_json" => "TEXT NOT NULL DEFAULT '[]'",
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

fn ensure_table_column_exists(
    conn: &Connection,
    table: &str,
    column: &str,
    column_type: &str,
) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let has_column = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .flatten()
        .any(|c| c == column);
    if !has_column {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {column_type}"),
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
                manual_db_nums, generate_db_nums, parse_db_types, force_rebuild_system_db,
                gen_model, gen_mesh, gen_spatial_tree, apply_boolean_operation, mesh_tol_ratio,
                export_json, export_parquet, pipeline_db_mode, runtime_db_mode,
                db_data_path, db_port, web_port, viewer_port, bind_host, public_base_url,
                associated_project,
                db_pid, web_pid, viewer_pid, viewer_url, parse_pid,
                status, parse_status, last_error, entry_url, db_user, db_password,
                last_parse_started_at, last_parse_finished_at, last_parse_duration_ms,
                created_at, updated_at, site_name, projects_json, auto_parse_related_dbnums
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35, ?36, ?37, ?38, ?39, ?40, ?41, ?42, ?43, ?44, ?45)",
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
            manual_db_nums_to_json(&site.generate_db_nums)?,
            parse_db_types_to_json(&site.parse_db_types)?,
            if site.force_rebuild_system_db { 1i64 } else { 0i64 },
            if site.gen_model { 1i64 } else { 0i64 },
            if site.gen_mesh { 1i64 } else { 0i64 },
            if site.gen_spatial_tree { 1i64 } else { 0i64 },
            if site.apply_boolean_operation { 1i64 } else { 0i64 },
            site.mesh_tol_ratio,
            if site.export_json { 1i64 } else { 0i64 },
            if site.export_parquet { 1i64 } else { 0i64 },
            managed_db_mode_to_str(site.pipeline_db_mode),
            managed_db_mode_to_str(site.runtime_db_mode),
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
            &site.site_name,
            projects_to_json(&site.projects).unwrap_or_else(|_| "[]".to_string()),
            if site.auto_parse_related_dbnums { 1i64 } else { 0i64 },
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

/// 返回站点运行库连接所需的最小上下文。
///
/// 调用方只应在服务端使用 `db_user/db_password`；这些字段不能透传给前端。
pub fn get_site_runtime_db_context(
    site_id: &str,
) -> Result<(ManagedProjectSite, String, String, String)> {
    let (site, db_user, db_password) = load_site_and_credentials(site_id)?;
    let db_name = site_source_project_name(&site);
    Ok((site, db_user, db_password, db_name))
}

fn row_to_remote_target(row: &rusqlite::Row<'_>) -> rusqlite::Result<ManagedRemoteTarget> {
    let allowed_cidrs_json: String = row
        .get("allowed_cidrs_json")
        .unwrap_or_else(|_| "[\"0.0.0.0/0\"]".to_string());
    let allowed_cidrs = serde_json::from_str::<Vec<String>>(&allowed_cidrs_json)
        .unwrap_or_else(|_| vec!["0.0.0.0/0".to_string()]);
    Ok(ManagedRemoteTarget {
        id: row.get("id")?,
        name: row.get("name")?,
        target_os: row
            .get::<_, Option<String>>("target_os")
            .unwrap_or(None)
            .as_deref()
            .map(remote_target_os_from_str)
            .unwrap_or_default(),
        host: row.get("host")?,
        ssh_port: row.get::<_, i64>("ssh_port")? as u16,
        ssh_user: row.get("ssh_user")?,
        password_env: row.get("password_env")?,
        ssh_password: row.get("ssh_password").unwrap_or(None),
        remote_root: row.get("remote_root")?,
        remote_db_path: row.get("remote_db_path")?,
        remote_web_port: row.get::<_, i64>("remote_web_port")? as u16,
        remote_db_port: row.get::<_, i64>("remote_db_port")? as u16,
        public_base_url: row.get("public_base_url")?,
        surreal_bin: row.get("surreal_bin")?,
        remote_web_bin: row.get("remote_web_bin")?,
        auto_prepare: row_bool_or(row, "auto_prepare", true),
        upload_web_server: row_bool_or(row, "upload_web_server", false),
        upload_surreal: row_bool_or(row, "upload_surreal", false),
        upload_resource: row_bool_or(row, "upload_resource", false),
        upload_viewer: row_bool_or(row, "upload_viewer", false),
        open_firewall: row_bool_or(row, "open_firewall", true),
        allowed_cidrs,
        web_bind_host: row
            .get::<_, Option<String>>("web_bind_host")
            .unwrap_or(None)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "0.0.0.0".to_string()),
        db_bind_host: row
            .get::<_, Option<String>>("db_bind_host")
            .unwrap_or(None)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "127.0.0.1".to_string()),
        local_web_bin: row.get("local_web_bin").unwrap_or(None),
        local_surreal_bin: row.get("local_surreal_bin").unwrap_or(None),
        local_resource_dir: row.get("local_resource_dir").unwrap_or(None),
        local_viewer_dir: row.get("local_viewer_dir").unwrap_or(None),
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

fn default_remote_target_for_site(site_id: &str) -> ManagedRemoteTarget {
    let now = now_rfc3339();
    ManagedRemoteTarget {
        remote_db_path: format!("/root/surreal_data/{site_id}.db"),
        created_at: now.clone(),
        updated_at: now,
        ..ManagedRemoteTarget::default()
    }
}

fn normalize_remote_allowed_cidrs(values: Vec<String>) -> Vec<String> {
    let mut normalized = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if normalized.is_empty() {
        normalized.push("0.0.0.0/0".to_string());
    }
    normalized
}

fn normalize_remote_bind_host(value: String, default: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed.to_string()
    }
}

fn normalize_optional_path(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn remote_os_defaults(
    os: ManagedRemoteTargetOs,
    site_id: &str,
) -> (&'static str, String, String, String, String) {
    match os {
        ManagedRemoteTargetOs::Windows => (
            "默认 Windows 目标",
            "C:/Plant3D/sites".to_string(),
            format!("C:/Plant3D/runtime/surrealdb/{site_id}.db"),
            "C:/Plant3D/bin/surreal/surreal.exe".to_string(),
            "C:/Plant3D/bin/web_server.exe".to_string(),
        ),
        ManagedRemoteTargetOs::Centos79 => (
            "默认 CentOS 7.9 目标",
            "/opt/plant3d/sites".to_string(),
            format!("/root/surreal_data/{site_id}.db"),
            "/usr/local/bin/surreal".to_string(),
            "/root/web_server".to_string(),
        ),
        ManagedRemoteTargetOs::Ubuntu22 => (
            "默认 Ubuntu22 目标",
            "/opt/plant3d/sites".to_string(),
            format!("/root/surreal_data/{site_id}.db"),
            "/usr/local/bin/surreal".to_string(),
            "/root/web_server".to_string(),
        ),
    }
}

fn normalize_remote_target(mut target: ManagedRemoteTarget, site_id: &str) -> ManagedRemoteTarget {
    let (default_name, default_root, default_db_path, default_surreal, default_web) =
        remote_os_defaults(target.target_os, site_id);
    if target.id.trim().is_empty() {
        target.id = "default".to_string();
    }
    if target.name.trim().is_empty() {
        target.name = default_name.to_string();
    }
    if target.host.trim().is_empty() {
        target.host = "123.57.182.243".to_string();
    }
    if target.ssh_port == 0 {
        target.ssh_port = 22;
    }
    if target.ssh_user.trim().is_empty() {
        target.ssh_user = "root".to_string();
    }
    if target.password_env.trim().is_empty() {
        target.password_env = "REMOTE_PASS".to_string();
    }
    if target.remote_root.trim().is_empty() {
        target.remote_root = default_root;
    }
    if target.remote_db_path.trim().is_empty() {
        target.remote_db_path = default_db_path;
    }
    if target.remote_web_port == 0 {
        target.remote_web_port = 3100;
    }
    if target.remote_db_port == 0 {
        target.remote_db_port = 8020;
    }
    if target.surreal_bin.trim().is_empty() {
        target.surreal_bin = default_surreal;
    }
    if target.remote_web_bin.trim().is_empty() {
        target.remote_web_bin = default_web;
    }
    target.allowed_cidrs = normalize_remote_allowed_cidrs(target.allowed_cidrs);
    target.web_bind_host = normalize_remote_bind_host(target.web_bind_host, "0.0.0.0");
    target.db_bind_host = normalize_remote_bind_host(target.db_bind_host, "127.0.0.1");
    target.local_web_bin = normalize_optional_path(target.local_web_bin);
    target.local_surreal_bin = normalize_optional_path(target.local_surreal_bin);
    target.local_resource_dir = normalize_optional_path(target.local_resource_dir);
    target.local_viewer_dir = normalize_optional_path(target.local_viewer_dir);
    target
}

fn apply_remote_target_request(
    mut target: ManagedRemoteTarget,
    req: ManagedRemoteTargetRequest,
    site_id: &str,
) -> ManagedRemoteTarget {
    if let Some(value) = req.id.filter(|value| !value.trim().is_empty()) {
        target.id = value;
    }
    if let Some(value) = req.name.filter(|value| !value.trim().is_empty()) {
        target.name = value;
    }
    if let Some(value) = req.target_os {
        target.target_os = value;
    }
    if let Some(value) = req.host.filter(|value| !value.trim().is_empty()) {
        target.host = value;
    }
    if let Some(value) = req.ssh_port.filter(|value| *value != 0) {
        target.ssh_port = value;
    }
    if let Some(value) = req.ssh_user.filter(|value| !value.trim().is_empty()) {
        target.ssh_user = value;
    }
    if let Some(value) = req.password_env.filter(|value| !value.trim().is_empty()) {
        target.password_env = value;
    }
    if let Some(value) = req.ssh_password.filter(|value| !value.trim().is_empty()) {
        target.ssh_password = Some(value);
    }
    if let Some(value) = req.remote_root.filter(|value| !value.trim().is_empty()) {
        target.remote_root = value;
    }
    if let Some(value) = req.remote_db_path.filter(|value| !value.trim().is_empty()) {
        target.remote_db_path = value;
    }
    if let Some(value) = req.remote_web_port.filter(|value| *value != 0) {
        target.remote_web_port = value;
    }
    if let Some(value) = req.remote_db_port.filter(|value| *value != 0) {
        target.remote_db_port = value;
    }
    if req.public_base_url.is_some() {
        target.public_base_url = req.public_base_url.filter(|value| !value.trim().is_empty());
    }
    if let Some(value) = req.surreal_bin.filter(|value| !value.trim().is_empty()) {
        target.surreal_bin = value;
    }
    if let Some(value) = req.remote_web_bin.filter(|value| !value.trim().is_empty()) {
        target.remote_web_bin = value;
    }
    if let Some(value) = req.auto_prepare {
        target.auto_prepare = value;
    }
    if let Some(value) = req.upload_web_server {
        target.upload_web_server = value;
    }
    if let Some(value) = req.upload_surreal {
        target.upload_surreal = value;
    }
    if let Some(value) = req.upload_resource {
        target.upload_resource = value;
    }
    if let Some(value) = req.upload_viewer {
        target.upload_viewer = value;
    }
    if let Some(value) = req.open_firewall {
        target.open_firewall = value;
    }
    if let Some(values) = req.allowed_cidrs {
        target.allowed_cidrs = values;
    }
    if let Some(value) = req.web_bind_host {
        target.web_bind_host = value;
    }
    if let Some(value) = req.db_bind_host {
        target.db_bind_host = value;
    }
    if req.local_web_bin.is_some() {
        target.local_web_bin = normalize_optional_path(req.local_web_bin);
    }
    if req.local_surreal_bin.is_some() {
        target.local_surreal_bin = normalize_optional_path(req.local_surreal_bin);
    }
    if req.local_resource_dir.is_some() {
        target.local_resource_dir = normalize_optional_path(req.local_resource_dir);
    }
    if req.local_viewer_dir.is_some() {
        target.local_viewer_dir = normalize_optional_path(req.local_viewer_dir);
    }
    normalize_remote_target(target, site_id)
}

fn load_remote_target_with_conn(
    conn: &Connection,
    target_id: &str,
) -> Result<Option<ManagedRemoteTarget>> {
    let sql = format!("SELECT * FROM {REMOTE_TARGETS_TABLE} WHERE id = ?1");
    Ok(conn
        .query_row(&sql, [target_id], row_to_remote_target)
        .optional()?)
}

fn persist_remote_target_with_conn(conn: &Connection, target: &ManagedRemoteTarget) -> Result<()> {
    let allowed_cidrs_json = serde_json::to_string(&target.allowed_cidrs)?;
    // Q1.3 安全收口：默认不把明文 SSH 密码落库，仅在显式开启
    // AIOS_ALLOW_SSH_PASSWORD_PERSIST 时持久化。注意：只影响落库的绑定值，
    // 不修改入参 target，调用方（resolve_remote_target）当次部署仍可用 ssh_password。
    let ssh_password_to_store: Option<&String> = if ssh_password_persist_allowed() {
        target.ssh_password.as_ref()
    } else {
        None
    };
    conn.execute(
        &format!(
            "INSERT OR REPLACE INTO {REMOTE_TARGETS_TABLE} (
                id, name, target_os, host, ssh_port, ssh_user, password_env, ssh_password, remote_root, remote_db_path,
                remote_web_port, remote_db_port, public_base_url, surreal_bin, remote_web_bin,
                auto_prepare, upload_web_server, upload_surreal, upload_resource, upload_viewer,
                open_firewall, allowed_cidrs_json, web_bind_host, db_bind_host, local_web_bin,
                local_surreal_bin, local_resource_dir, local_viewer_dir, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25,
                ?26, ?27, ?28, ?29, ?30
            )"
        ),
        params![
            &target.id,
            &target.name,
            remote_target_os_to_str(target.target_os),
            &target.host,
            target.ssh_port as i64,
            &target.ssh_user,
            &target.password_env,
            ssh_password_to_store,
            &target.remote_root,
            &target.remote_db_path,
            target.remote_web_port as i64,
            target.remote_db_port as i64,
            &target.public_base_url,
            &target.surreal_bin,
            &target.remote_web_bin,
            target.auto_prepare as i64,
            target.upload_web_server as i64,
            target.upload_surreal as i64,
            target.upload_resource as i64,
            target.upload_viewer as i64,
            target.open_firewall as i64,
            &allowed_cidrs_json,
            &target.web_bind_host,
            &target.db_bind_host,
            &target.local_web_bin,
            &target.local_surreal_bin,
            &target.local_resource_dir,
            &target.local_viewer_dir,
            &target.created_at,
            &target.updated_at,
        ],
    )?;
    Ok(())
}

fn resolve_remote_target(
    site_id: &str,
    req: Option<ManagedRemoteDeployRequest>,
) -> Result<ManagedRemoteTarget> {
    with_conn(|conn| {
        let request = req.unwrap_or_default();
        let target_id = request
            .target_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("default");
        let base = load_remote_target_with_conn(conn, target_id)?
            .unwrap_or_else(|| default_remote_target_for_site(site_id));
        let mut target = if let Some(target_req) = request.target {
            apply_remote_target_request(base, target_req, site_id)
        } else {
            normalize_remote_target(base, site_id)
        };
        if let Some(password) = target.ssh_password.as_deref() {
            remember_remote_password(site_id, &target.id, password);
        } else {
            target.ssh_password = remembered_remote_password(site_id, &target.id);
        }
        let now = now_rfc3339();
        if target.created_at.trim().is_empty() {
            target.created_at = now.clone();
        }
        target.updated_at = now;
        persist_remote_target_with_conn(conn, &target)?;
        Ok(target)
    })
}

pub fn list_remote_targets() -> Result<Vec<ManagedRemoteTarget>> {
    with_conn(|conn| {
        let mut stmt = conn.prepare(&format!(
            "SELECT * FROM {REMOTE_TARGETS_TABLE} ORDER BY updated_at DESC"
        ))?;
        let targets = stmt
            .query_map([], row_to_remote_target)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(targets)
    })
}

pub fn upsert_remote_target(req: ManagedRemoteTargetRequest) -> Result<ManagedRemoteTarget> {
    with_conn(|conn| {
        let target_id = req
            .id
            .clone()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "default".to_string());
        let base = load_remote_target_with_conn(conn, &target_id)?
            .unwrap_or_else(|| default_remote_target_for_site("default"));
        let mut target = apply_remote_target_request(base, req, "default");
        target.id = target_id;
        let now = now_rfc3339();
        if target.created_at.trim().is_empty() {
            target.created_at = now.clone();
        }
        target.updated_at = now;
        persist_remote_target_with_conn(conn, &target)?;
        Ok(target)
    })
}

pub fn get_remote_deploy_status(site_id: &str) -> Result<ManagedRemoteDeployStatus> {
    with_conn(|conn| {
        let sql = format!(
            "SELECT site_id, target_id, deploy_id, deploy_task_id, deployment_mode, degraded,
                    status, current_step, remote_entry_url, checked_at, last_error, checks_json
             FROM {REMOTE_DEPLOY_STATUS_TABLE} WHERE site_id = ?1"
        );
        let status = conn
            .query_row(&sql, [site_id], |row| {
                let checks_json: String = row.get(11)?;
                let checks = serde_json::from_str(&checks_json).unwrap_or_default();
                Ok(ManagedRemoteDeployStatus {
                    site_id: row.get(0)?,
                    target_id: row.get(1)?,
                    deploy_id: row.get(2)?,
                    deploy_task_id: row.get(3)?,
                    deployment_mode: row.get(4)?,
                    degraded: row_bool_index(row, 5, false),
                    status: row.get(6)?,
                    current_step: row.get(7)?,
                    remote_entry_url: row.get(8)?,
                    checked_at: row.get(9)?,
                    last_error: row.get(10)?,
                    checks,
                })
            })
            .optional()?;
        Ok(status.unwrap_or_else(|| ManagedRemoteDeployStatus {
            site_id: site_id.to_string(),
            target_id: "default".to_string(),
            deploy_id: None,
            deploy_task_id: None,
            deployment_mode: None,
            degraded: false,
            status: "idle".to_string(),
            current_step: "尚未远端部署".to_string(),
            remote_entry_url: None,
            checked_at: now_rfc3339(),
            last_error: None,
            checks: Vec::new(),
        }))
    })
}

fn save_remote_deploy_status(status: &ManagedRemoteDeployStatus) -> Result<()> {
    let checks_json = serde_json::to_string(&status.checks)?;
    with_conn(|conn| {
        conn.execute(
            &format!(
                "INSERT OR REPLACE INTO {REMOTE_DEPLOY_STATUS_TABLE} (
                    site_id, target_id, deploy_id, deploy_task_id, deployment_mode, degraded,
                    status, current_step, remote_entry_url, checked_at, last_error, checks_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"
            ),
            params![
                &status.site_id,
                &status.target_id,
                &status.deploy_id,
                &status.deploy_task_id,
                &status.deployment_mode,
                status.degraded as i64,
                &status.status,
                &status.current_step,
                &status.remote_entry_url,
                &status.checked_at,
                &status.last_error,
                &checks_json,
            ],
        )?;
        Ok(())
    })
}

fn load_raw_site(site_id: &str) -> Result<ManagedProjectSite> {
    with_conn(|conn| load_site_with_conn(conn, site_id))?.ok_or_else(|| anyhow!("站点不存在"))
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

fn reassign_db_port_if_occupied(site_id: &str) -> Result<Option<ManagedProjectSite>> {
    let changed = with_tx(|conn| {
        let mut site = load_site_with_conn(conn, site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
        if !port_in_use("127.0.0.1", site.db_port) {
            return Ok(None);
        }

        let old_db_port = site.db_port;
        let mut used = collect_reserved_ports_with_conn(conn, Some(site_id))?;
        used.insert(site.web_port);
        if let Some(viewer_port) = site.viewer_port {
            used.insert(viewer_port);
        }
        site.db_port = first_available_port(&mut used, AUTO_DB_PORT_START, AUTO_PORT_END)?;
        site.db_pid = None;
        site.updated_at = now_rfc3339();

        let (db_user, db_password) = load_credentials_with_conn(conn, site_id)?;
        persist_site_with_conn(conn, &site, &db_user, &db_password)?;
        Ok(Some((site, db_user, db_password, old_db_port)))
    })?;

    let Some((site, db_user, db_password, old_db_port)) = changed else {
        return Ok(None);
    };

    write_site_files(&site, &db_user, &db_password)?;
    append_log_line(
        &db_log_path(&site.site_id),
        &format!(
            "DB 端口 {old} 被占用，启动前已自动改用空闲端口 {new}",
            old = old_db_port,
            new = site.db_port
        ),
    );
    crate::web_server::sse_handlers::push_admin_site_snapshot(
        &site.site_id,
        Some(&site.project_name),
        status_to_str(&site.status),
        parse_status_to_str(&site.parse_status),
        site.last_error.as_deref(),
    );
    Ok(Some(site))
}

// ─── Public read-side API ───────────────────────────────────────────────────

pub fn get_site(site_id: &str) -> Result<Option<ManagedProjectSite>> {
    let mut site = with_conn(|conn| load_site_with_conn(conn, site_id))?;
    if let Some(item) = site.as_mut() {
        *item = derive_runtime_state(item.clone());
        normalize_viewer_url_for_response(item);
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
        normalize_viewer_url_for_response(item);
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

    let site_name = req
        .site_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .unwrap_or_else(|| req.project_name.trim().to_string());
    if site_name.is_empty() {
        bail!("站点名称不能为空");
    }
    let projects = if req.projects.is_empty() {
        vec![SiteProject {
            path: canonical_path.to_string_lossy().to_string(),
            name: req.project_name.trim().to_string(),
            role: ProjectRole::Design,
            is_primary: true,
            sort_order: 0,
        }]
    } else {
        validate_and_canonicalize_projects(&req.projects)?
    };
    precheck_dbnum_conflicts(&projects)?;
    if !req.manual_db_files.is_empty() || !req.generate_db_files.is_empty() {
        bail!("web_server 不再解析 db_file；请先通过 aios-database sidecar 解析为 dbnum");
    }
    let manual_db_nums = normalize_manual_db_nums(req.manual_db_nums);
    let generate_db_nums = normalize_manual_db_nums(req.generate_db_nums);

    let _guard = lock_op()?;

    let (db_port, web_port) = with_conn(|conn| {
        if let Some(existing_project_name) =
            project_name_conflict_with_conn(conn, req.project_name.trim(), None)?
        {
            bail!(
                "项目名已存在：{}。请修改项目名称后再保存。",
                existing_project_name
            );
        }
        if site_name_exists_with_conn(conn, &site_name)? {
            bail!("站点名称已存在：{}。请修改站点名称后再创建。", site_name);
        }
        resolve_create_ports_with_conn(conn, req.db_port, req.web_port)
    })?;
    let site_id = infer_site_id(&site_name, web_port);
    let created_at = now_rfc3339();
    let bind_host = normalize_host_or(req.bind_host, &default_web_bind_host());
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
        site_name: site_name.clone(),
        project_name: req.project_name.trim().to_string(),
        project_code: req.project_code,
        project_path: canonical_path.to_string_lossy().to_string(),
        projects: projects.clone(),
        manual_db_nums,
        generate_db_nums,
        force_rebuild_system_db: normalize_force_rebuild_system_db(
            req.force_rebuild_system_db,
            &parse_db_types,
        ),
        auto_parse_related_dbnums: req.auto_parse_related_dbnums,
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
        pipeline_db_mode: req.pipeline_db_mode.unwrap_or(ManagedSiteDbMode::Ws),
        runtime_db_mode: ManagedSiteDbMode::Ws,
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

/// 未提供项目名时生成**稳定**默认站点名 `quicktest-<dbnum>`（G6 幂等）。
///
/// 旧实现按 `quicktest-{N}` 递增，导致同一 dbnum 多次快测每次都新建站点、无限累积。
/// 改为按 dbnum 取稳定名后，重复调用可命中同名站点并复用/重置（见 `quick_deploy_test`）。
fn default_quicktest_site_name(dbnum: u32) -> String {
    format!("quicktest-{}", dbnum)
}

#[derive(Debug, Clone, Copy)]
enum QuickDeployProfile {
    Test,
    Admin,
}

impl QuickDeployProfile {
    fn db_credentials(self, dbnum: u32) -> (String, String) {
        match self {
            QuickDeployProfile::Test => ("quicktest".to_string(), "QuickTest@2026".to_string()),
            QuickDeployProfile::Admin => (
                format!("siteadmin{dbnum}"),
                format!("AdminQuickDeploy@{dbnum}"),
            ),
        }
    }
}

async fn resolve_db_file_via_sidecar(project_root: &Path, db_file: &str) -> Result<(u32, String)> {
    let root = project_root.to_string_lossy().to_string();
    let resolved =
        crate::web_server::parse_sidecar_client::resolve_db_file(vec![root], db_file.to_string())
            .await
            .map_err(|err| anyhow!("aios-database sidecar DB 文件解析失败: {}", err.message))?;
    Ok((resolved.dbnum, resolved.file_name))
}

/// 从绝对 dbfile 推断 E3D 工程根。
///
/// 常见布局为 `<project>/<db-folder>/<db-file>`，例如
/// `AvevaPlantSample/aps000/aps250124_0001`。如果父目录看起来是 db 分片目录
/// （形如 `aps000`/`cat000`），取其父级作为工程根；否则退回 dbfile 所在目录。
fn infer_project_root_from_db_file(db_file: &str) -> Result<PathBuf> {
    let path = Path::new(db_file.trim());
    if !path.is_absolute() {
        bail!("未提供 project_path 时，db_file 必须是绝对路径");
    }
    if !path.is_file() {
        bail!("db_file 不存在或不可访问: {}", path.display());
    }
    let canonical_file =
        fs::canonicalize(path).with_context(|| format!("db_file 无法访问: {}", path.display()))?;
    let db_dir = canonical_file
        .parent()
        .ok_or_else(|| anyhow!("无法从 db_file 推断所在目录: {}", canonical_file.display()))?;
    let db_dir_name = db_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let looks_like_db_folder = db_dir_name.len() >= 6
        && db_dir_name
            .chars()
            .take(3)
            .all(|ch| ch.is_ascii_alphabetic())
        && db_dir_name.chars().skip(3).all(|ch| ch.is_ascii_digit());
    let root = if looks_like_db_folder {
        db_dir.parent().unwrap_or(db_dir)
    } else {
        db_dir
    };
    canonical_project_path(&root.to_string_lossy())
}

/// 一键部署测试（免鉴权快测）：建站 → 解析(单库, 可选含关联) → 生成 →(可选)启动。
pub async fn quick_deploy_test(req: QuickDeployTestRequest) -> Result<QuickDeployTestResponse> {
    quick_deploy(req, QuickDeployProfile::Test).await
}

/// Admin 鉴权版 quick deploy：只快速创建部署配置，不自动解析/生成/启动。
pub async fn quick_deploy_admin(req: QuickDeployTestRequest) -> Result<QuickDeployTestResponse> {
    quick_create_deploy_config(req, QuickDeployProfile::Admin).await
}

async fn quick_create_deploy_config(
    req: QuickDeployTestRequest,
    profile: QuickDeployProfile,
) -> Result<QuickDeployTestResponse> {
    let started = Instant::now();

    let project_path = req.project_path.trim().to_string();
    let canonical = if project_path.is_empty() {
        let db_file = req
            .db_file
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                anyhow!("必须提供 project_path，或提供可用于推断工程根的绝对 db_file")
            })?;
        infer_project_root_from_db_file(db_file)?
    } else {
        canonical_project_path(&project_path)?
    };

    let (dbnum, resolved_db_file) = match req.dbnum {
        Some(n) if n > 0 => (n, req.db_file.clone().unwrap_or_default()),
        _ => {
            let db_file = req
                .db_file
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow!("必须提供 db_file（文件名/路径）或 dbnum"))?;
            resolve_db_file_via_sidecar(&canonical, db_file).await?
        }
    };

    let provided_name = req
        .project_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    let e3d_project_name = provided_name.clone().unwrap_or_else(|| {
        canonical
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("project")
            .to_string()
    });
    let base_site_name = provided_name.unwrap_or_else(|| default_quicktest_site_name(dbnum));
    let site_name = with_conn(|conn| unique_site_name_with_conn(conn, &base_site_name))?;
    let mut warnings = Vec::new();
    if site_name != base_site_name {
        warnings.push(format!(
            "站点名称 {base} 已存在，快速创建部署已自动改名为 {name}",
            base = base_site_name,
            name = site_name
        ));
    }

    let project_code = req.project_code.filter(|code| *code > 0).unwrap_or(1);
    let (db_user, db_password) = profile.db_credentials(dbnum);
    let site = create_site(CreateManagedSiteRequest {
        site_name: Some(site_name),
        projects: Vec::new(),
        project_name: e3d_project_name,
        project_path: canonical.to_string_lossy().to_string(),
        project_code,
        manual_db_nums: vec![dbnum],
        manual_db_files: Vec::new(),
        generate_db_nums: Vec::new(),
        generate_db_files: Vec::new(),
        parse_db_types: Vec::new(),
        force_rebuild_system_db: false,
        auto_parse_related_dbnums: req.auto_parse_related_dbnums.unwrap_or(false),
        gen_model: Some(req.gen_model),
        gen_mesh: Some(req.gen_mesh),
        gen_spatial_tree: Some(req.gen_spatial_tree),
        apply_boolean_operation: None,
        mesh_tol_ratio: None,
        export_json: None,
        export_parquet: None,
        pipeline_db_mode: Some(req.pipeline_db_mode.unwrap_or(ManagedSiteDbMode::Ws)),
        runtime_db_mode: None,
        db_port: None,
        web_port: req.web_port,
        auto_deploy: false,
        bind_host: None,
        public_base_url: None,
        associated_project: None,
        db_user: Some(db_user.clone()),
        db_password: Some(db_password.clone()),
    })?;

    let parse_plan = load_parse_plan_from_sidecar(&site).await?;
    task::spawn_blocking({
        let site = site.clone();
        let db_user = db_user.clone();
        let db_password = db_password.clone();
        let parse_plan = parse_plan.clone();
        move || write_site_files_with_parse_plan(&site, &db_user, &db_password, Some(&parse_plan))
    })
    .await
    .context("写入 quick deploy parse manifest 失败 (join error)")??;

    let resolved_db_file = if resolved_db_file.is_empty() {
        None
    } else {
        Some(resolved_db_file)
    };

    Ok(QuickDeployTestResponse {
        success: true,
        site_id: site.site_id,
        dbnum: Some(dbnum),
        resolved_db_file,
        parse_status: parse_status_to_str(&ManagedSiteParseStatus::Pending).to_string(),
        generated: false,
        task_id: None,
        entry_url: None,
        duration_ms: started.elapsed().as_millis() as u64,
        parse_log_tail: Vec::new(),
        generate_log_tail: Vec::new(),
        warnings,
        message: Some("快速创建部署配置成功，请手动执行部署或启动。".to_string()),
    })
}

async fn quick_deploy(
    req: QuickDeployTestRequest,
    profile: QuickDeployProfile,
) -> Result<QuickDeployTestResponse> {
    let started = Instant::now();

    let project_path = req.project_path.trim().to_string();
    let canonical = if project_path.is_empty() {
        let db_file = req
            .db_file
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                anyhow!("必须提供 project_path，或提供可用于推断工程根的绝对 db_file")
            })?;
        infer_project_root_from_db_file(db_file)?
    } else {
        canonical_project_path(&project_path)?
    };

    // 1) dbnum 解析（dbnum 优先，否则按 db_file 读文件头）
    let (dbnum, resolved_db_file) = match req.dbnum {
        Some(n) if n > 0 => (n, req.db_file.clone().unwrap_or_default()),
        _ => {
            let db_file = req
                .db_file
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow!("必须提供 db_file（文件名/路径）或 dbnum"))?;
            resolve_db_file_via_sidecar(&canonical, db_file).await?
        }
    };

    // 2) 命名：未提供项目名 → E3D 项目名取目录名，站点显示名用默认 quicktest-<dbnum>。
    let provided_name = req
        .project_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    let e3d_project_name = provided_name.clone().unwrap_or_else(|| {
        canonical
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("project")
            .to_string()
    });
    let base_site_name = provided_name.unwrap_or_else(|| default_quicktest_site_name(dbnum));
    let site_name = with_conn(|conn| unique_site_name_with_conn(conn, &base_site_name))?;
    let mut name_warnings = Vec::new();
    if site_name != base_site_name {
        name_warnings.push(format!(
            "站点名称 {base} 已存在，快速部署已自动改名为 {name}",
            base = base_site_name,
            name = site_name
        ));
    }
    let project_code = req.project_code.filter(|code| *code > 0).unwrap_or(1);

    // 3) quick deploy 默认 db 凭据。免鉴权快测保持历史 quicktest；
    //    admin 入口使用 per-dbnum 凭据，避免正式归档站点写入测试固定用户。
    let (db_user, db_password) = profile.db_credentials(dbnum);
    let create_req = CreateManagedSiteRequest {
        site_name: Some(site_name.clone()),
        projects: Vec::new(),
        project_name: e3d_project_name.clone(),
        project_path: canonical.to_string_lossy().to_string(),
        project_code,
        manual_db_nums: vec![dbnum],
        manual_db_files: Vec::new(),
        generate_db_nums: Vec::new(),
        generate_db_files: Vec::new(),
        parse_db_types: Vec::new(),
        force_rebuild_system_db: false,
        auto_parse_related_dbnums: req.auto_parse_related_dbnums.unwrap_or(false),
        gen_model: Some(req.gen_model),
        gen_mesh: Some(req.gen_mesh),
        gen_spatial_tree: Some(req.gen_spatial_tree),
        apply_boolean_operation: None,
        mesh_tol_ratio: None,
        export_json: None,
        export_parquet: None,
        pipeline_db_mode: req.pipeline_db_mode,
        runtime_db_mode: None,
        db_port: None,
        web_port: req.web_port,
        auto_deploy: false,
        bind_host: None,
        public_base_url: None,
        associated_project: None,
        db_user: Some(db_user),
        db_password: Some(db_password),
    };

    // 4) quick deploy 遇到同名站点时只自动加后缀创建新站点，不删除/替换旧站点。
    let site = create_site(create_req)?;
    let site_id = site.site_id.clone();
    let want_generate = req.gen_model || req.gen_mesh || req.gen_spatial_tree;
    let resolved_db_file = if resolved_db_file.is_empty() {
        None
    } else {
        Some(resolved_db_file)
    };

    // 5) 后台模式：立即返回 site_id；pipeline 改由持久化任务表（admin_tasks）调度，
    //    替代原先裸 tokio::spawn 的 fire-and-forget —— 进程重启后任务记录仍可查询/对账，
    //    且与 /api/admin/sites/{id}/deploy 共用同一套调度 + 站点运行态对账。
    if !req.wait {
        let task_type = if req.start_site {
            crate::web_server::models::TaskType::DeployManagedSite
        } else if want_generate {
            crate::web_server::models::TaskType::FullGeneration
        } else {
            crate::web_server::models::TaskType::ParsePdmsData
        };
        let mut task_config = DatabaseConfig::default();
        task_config.name = format!("快速部署 - {}", site_name);

        let (message, task_warnings, task_id) =
            match crate::web_server::admin_task_handlers::create_and_dispatch_site_task(
                site_id.clone(),
                task_config.name.clone(),
                task_type,
                crate::web_server::models::TaskPriority::Normal,
                task_config,
            ) {
                Ok(task) => {
                    let task_id = task.id.clone();
                    (
                        Some(format!("已提交后台部署任务（task_id={task_id}）")),
                        vec![format!(
                            "wait=false：已创建持久化任务 task_id={tid}，可用 GET /api/admin/tasks/{tid} 或 GET /api/admin/sites/{{id}}/runtime 轮询进度",
                            tid = task_id
                        )],
                        Some(task_id),
                    )
                }
                Err(err) => {
                    // 持久化任务创建失败时回退到后台 spawn，保证行为不退化。
                    tracing::warn!(
                        site = %site_id,
                        "创建持久化部署任务失败，回退 fire-and-forget: {err}"
                    );
                    let site_id_bg = site_id.clone();
                    let start_site = req.start_site;
                    tokio::spawn(async move {
                        let result = if start_site {
                            run_deploy_pipeline(site_id_bg.clone()).await
                        } else if want_generate {
                            run_generation_pipeline(site_id_bg.clone(), true).await
                        } else {
                            run_parse_pipeline(site_id_bg.clone()).await
                        };
                        if let Err(err) = result {
                            let _ = update_runtime(
                                &site_id_bg,
                                RuntimeUpdate {
                                    status: Some(ManagedSiteStatus::Failed),
                                    last_error: Some(Some(err.to_string())),
                                    ..Default::default()
                                },
                            );
                        }
                    });
                    (
                        Some("已在后台启动一键部署测试（持久化任务创建失败，已降级为非持久化后台执行）".to_string()),
                        vec![format!(
                            "wait=false：持久化任务创建失败（{err}），已回退后台执行，请用 GET /api/admin/sites/{{id}}/runtime 轮询进度"
                        )],
                        None,
                    )
                }
            };
        let mut warnings = name_warnings;
        warnings.extend(task_warnings);

        return Ok(QuickDeployTestResponse {
            success: true,
            site_id,
            dbnum: Some(dbnum),
            resolved_db_file,
            parse_status: parse_status_to_str(&ManagedSiteParseStatus::Pending).to_string(),
            generated: false,
            task_id,
            entry_url: None,
            duration_ms: started.elapsed().as_millis() as u64,
            parse_log_tail: Vec::new(),
            generate_log_tail: Vec::new(),
            warnings,
            message,
        });
    }

    // 6) 同步模式：等 pipeline 结束再返回 summary
    let pipeline_result = if req.start_site {
        run_deploy_pipeline(site_id.clone()).await
    } else if want_generate {
        run_generation_pipeline(site_id.clone(), true).await
    } else {
        run_parse_pipeline(site_id.clone()).await
    };

    let final_site = get_site(&site_id)?.unwrap_or(site);
    let parse_log_tail = tail_log(&site_id, "parse", 40)
        .map(|tail| tail.lines)
        .unwrap_or_default();
    let generate_log_tail = if want_generate {
        tail_log(&site_id, "generate", 40)
            .map(|tail| tail.lines)
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let parsed = final_site.parse_status == ManagedSiteParseStatus::Parsed;
    let mut warnings = name_warnings;
    if let Err(err) = &pipeline_result {
        warnings.push(format!("pipeline 错误: {err}"));
    }

    Ok(QuickDeployTestResponse {
        success: pipeline_result.is_ok() && parsed,
        site_id,
        dbnum: Some(dbnum),
        resolved_db_file,
        parse_status: parse_status_to_str(&final_site.parse_status).to_string(),
        generated: want_generate && parsed && pipeline_result.is_ok(),
        task_id: None,
        entry_url: if req.start_site {
            final_site.entry_url.clone()
        } else {
            None
        },
        duration_ms: started.elapsed().as_millis() as u64,
        parse_log_tail,
        generate_log_tail,
        warnings,
        message: final_site.last_error.clone(),
    })
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
            site_name: req
                .site_name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string())
                .unwrap_or_else(|| project_name.to_string()),
            project_name: project_name.to_string(),
            project_code: 0,
            project_path: canonical_path.to_string_lossy().to_string(),
            projects: Vec::new(),
            manual_db_nums: Vec::new(),
            generate_db_nums: Vec::new(),
            parse_db_types: Vec::new(),
            force_rebuild_system_db: false,
            auto_parse_related_dbnums: false,
            gen_model: generation_defaults.gen_model,
            gen_mesh: generation_defaults.gen_mesh,
            gen_spatial_tree: generation_defaults.gen_spatial_tree,
            apply_boolean_operation: generation_defaults.apply_boolean_operation,
            mesh_tol_ratio: generation_defaults.mesh_tol_ratio,
            export_json: generation_defaults.export_json,
            export_parquet: generation_defaults.export_parquet,
            pipeline_db_mode: ManagedSiteDbMode::Ws,
            runtime_db_mode: ManagedSiteDbMode::Ws,
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
    if let Some(name) = req
        .site_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        site.site_name = name.to_string();
    } else if site.site_name.trim().is_empty() {
        site.site_name = project_name.to_string();
    }
    if !req.projects.is_empty() {
        site.projects = req.projects.clone();
    } else if site.projects.is_empty() {
        site.projects = vec![SiteProject {
            path: canonical_path.to_string_lossy().to_string(),
            name: project_name.to_string(),
            role: ProjectRole::Design,
            is_primary: true,
            sort_order: 0,
        }];
    }
    if !req.manual_db_files.is_empty() {
        bail!("web_server 不再解析 db_file；请先通过 aios-database sidecar 解析为 dbnum");
    }
    site.manual_db_nums = normalize_manual_db_nums(req.manual_db_nums);
    site.parse_db_types = parse_db_types;
    site.force_rebuild_system_db = force_rebuild_system_db;
    site.auto_parse_related_dbnums = req.auto_parse_related_dbnums;
    site.web_port = req.web_port;
    site.bind_host = normalize_host_or(req.bind_host, &default_web_bind_host());
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
    let _ = &req;
    bail!("web_server 不再生成解析预览；请调用 aios-database sidecar /parse/preview-plan")
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
    if let Some(value) = req.site_name.filter(|value| !value.trim().is_empty()) {
        site.site_name = value.trim().to_string();
    }
    if let Some(projects) = req.projects {
        if !projects.is_empty() {
            site.projects = validate_and_canonicalize_projects(&projects)?;
        }
    }
    precheck_dbnum_conflicts(&site.projects)?;
    if let Some(value) = req.project_path.filter(|value| !value.trim().is_empty()) {
        let canonical = canonical_project_path(value.trim())?;
        site.project_path = canonical.to_string_lossy().to_string();
    }
    if let Some(value) = req.project_code.filter(|value| *value > 0) {
        site.project_code = value;
    }
    if !req.manual_db_files.is_empty() || !req.generate_db_files.is_empty() {
        bail!("web_server 不再解析 db_file；请先通过 aios-database sidecar 解析为 dbnum");
    }
    if req.manual_db_nums.is_some() || !req.manual_db_files.is_empty() {
        let base = req
            .manual_db_nums
            .unwrap_or_else(|| site.manual_db_nums.clone());
        site.manual_db_nums = normalize_manual_db_nums(base);
    }
    if req.generate_db_nums.is_some() || !req.generate_db_files.is_empty() {
        let base = req
            .generate_db_nums
            .unwrap_or_else(|| site.generate_db_nums.clone());
        site.generate_db_nums = normalize_manual_db_nums(base);
    }
    if let Some(value) = req.parse_db_types {
        site.parse_db_types = normalize_parse_db_types(value);
    }
    if let Some(value) = req.force_rebuild_system_db {
        site.force_rebuild_system_db = value;
    }
    if let Some(value) = req.auto_parse_related_dbnums {
        site.auto_parse_related_dbnums = value;
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
    if let Some(value) = req.pipeline_db_mode {
        site.pipeline_db_mode = value;
    }
    site.runtime_db_mode = ManagedSiteDbMode::Ws;
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
        if let Some(existing_project_name) =
            project_name_conflict_with_conn(conn, &site.project_name, Some(site_id))?
        {
            bail!(
                "项目名已存在：{}。请修改项目名称后再保存。",
                existing_project_name
            );
        }
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

pub async fn append_db_file_to_site(
    site_id: &str,
    req: AppendManagedSiteDbFileRequest,
) -> Result<AppendManagedSiteDbFileResponse> {
    let db_file = req.db_file.trim().to_string();
    if db_file.is_empty() && req.dbnum.unwrap_or_default() == 0 {
        bail!("必须提供 db_file 或 dbnum");
    }

    let mut site = task::spawn_blocking({
        let site_id = site_id.to_string();
        move || get_site(&site_id)
    })
    .await
    .context("读取站点状态失败 (join error)")??
    .ok_or_else(|| anyhow!("站点不存在"))?;

    let canonical_root = canonical_project_path(&site.project_path)?;
    let (dbnum, resolved_db_file) = match req.dbnum {
        Some(value) if value > 0 => {
            let resolved = if db_file.is_empty() {
                None
            } else {
                Some(db_file.clone())
            };
            (value, resolved)
        }
        _ => {
            let (dbnum, rel) = resolve_db_file_via_sidecar(&canonical_root, &db_file).await?;
            (dbnum, Some(rel))
        }
    };

    let was_active = site_has_active_processes(&site)
        || matches!(
            site.status,
            ManagedSiteStatus::Running | ManagedSiteStatus::Starting | ManagedSiteStatus::Stopping
        )
        || site.parse_status == ManagedSiteParseStatus::Running;
    let mut stopped_site = false;
    if was_active {
        if !req.stop_running {
            bail!("站点运行中，不能追加 DB file；请先停止站点或启用 stop_running");
        }
        let stop_result = stop_site(site_id)
            .await
            .with_context(|| format!("追加 DB file 前停止站点 {site_id} 失败"))?;
        if stop_result.conflict {
            bail!(
                "追加 DB file 前停止站点时检测到端口冲突（web={:?} db={:?} viewer={:?}），请先排查外部占用",
                stop_result.web_conflict_pids,
                stop_result.db_conflict_pids,
                stop_result.viewer_conflict_pids
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        stopped_site = true;
        site = task::spawn_blocking({
            let site_id = site_id.to_string();
            move || get_site(&site_id)
        })
        .await
        .context("停止后读取站点状态失败 (join error)")??
        .ok_or_else(|| anyhow!("站点不存在"))?;
    }

    let already_present = site.manual_db_nums.contains(&dbnum);
    let mut manual_db_nums = site.manual_db_nums.clone();
    manual_db_nums.push(dbnum);
    manual_db_nums = normalize_manual_db_nums(manual_db_nums);

    let updated_site = update_site(
        site_id,
        UpdateManagedSiteRequest {
            manual_db_nums: Some(manual_db_nums.clone()),
            ..Default::default()
        },
    )?;

    Ok(AppendManagedSiteDbFileResponse {
        site_id: site_id.to_string(),
        dbnum,
        resolved_db_file,
        already_present,
        stopped_site,
        manual_db_nums,
        site: updated_site,
        task_id: None,
    })
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

fn local_http_json(
    host: &str,
    port: u16,
    path: &str,
    timeout: Duration,
) -> Result<serde_json::Value> {
    let addr = format!("{}:{port}", url_host(host));
    let socket = addr
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow!("无法解析本机地址: {addr}"))?;
    let mut stream = TcpStream::connect_timeout(&socket, timeout)
        .with_context(|| format!("连接本机 HTTP 端口失败: {addr}"))?;
    stream
        .set_read_timeout(Some(timeout))
        .with_context(|| format!("设置读取超时失败: {addr}"))?;
    stream
        .set_write_timeout(Some(timeout))
        .with_context(|| format!("设置写入超时失败: {addr}"))?;
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {}:{port}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
        url_host(host)
    );
    stream
        .write_all(request.as_bytes())
        .with_context(|| format!("发送 HTTP 请求失败: {addr}{path}"))?;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(header_end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..header_end]);
                    if let Some(content_length) = header.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    }) {
                        let body_len = bytes.len().saturating_sub(header_end + 4);
                        if body_len >= content_length {
                            break;
                        }
                    }
                }
            }
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) && !bytes.is_empty() =>
            {
                break;
            }
            Err(err) => {
                return Err(err).with_context(|| format!("读取 HTTP 响应失败: {addr}{path}"));
            }
        }
    }
    let raw = String::from_utf8_lossy(&bytes);
    let (head, body) = raw
        .split_once("\r\n\r\n")
        .or_else(|| raw.split_once("\n\n"))
        .ok_or_else(|| anyhow!("HTTP 响应格式异常: {addr}{path}"))?;
    let status_ok = head
        .lines()
        .next()
        .is_some_and(|line| line.contains(" 200 ") || line.ends_with(" 200"));
    if !status_ok {
        bail!(
            "HTTP 状态异常: {}",
            head.lines().next().unwrap_or("unknown")
        );
    }
    serde_json::from_str(body.trim()).with_context(|| format!("HTTP JSON 解析失败: {addr}{path}"))
}

#[derive(Debug, Clone, Default)]
struct SiteConnectivityProbe {
    web_status_ok: Option<bool>,
    database_connected: Option<bool>,
    surrealdb_connected: Option<bool>,
    site_identity_ok: Option<bool>,
}

fn probe_site_connectivity(site: &ManagedProjectSite, web_running: bool) -> SiteConnectivityProbe {
    if !web_running {
        return SiteConnectivityProbe {
            web_status_ok: Some(false),
            database_connected: Some(false),
            surrealdb_connected: Some(false),
            site_identity_ok: Some(false),
        };
    }

    let timeout = Duration::from_secs(2);
    let probe_host = site_probe_host(site);
    let status = local_http_json(&probe_host, site.web_port, "/api/status", timeout);
    let (web_status_ok, database_connected, surrealdb_connected) = match status {
        Ok(value) => {
            let database_connected = value
                .get("database_connected")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let surrealdb_connected = value
                .get("surrealdb_connected")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            (
                Some(database_connected && surrealdb_connected),
                Some(database_connected),
                Some(surrealdb_connected),
            )
        }
        Err(_) => (Some(false), Some(false), Some(false)),
    };

    let identity = local_http_json(&probe_host, site.web_port, "/api/site/identity", timeout);
    let site_identity_ok = match identity {
        Ok(value) => {
            let site_id_ok = value
                .get("site_id")
                .and_then(|v| v.as_str())
                .is_some_and(|id| id == site.site_id);
            let port_ok = value
                .get("web_listen_port")
                .or_else(|| value.get("bind_port"))
                .and_then(|v| v.as_u64())
                .is_some_and(|port| port == site.web_port as u64);
            Some(site_id_ok && port_ok)
        }
        Err(_) => Some(false),
    };

    SiteConnectivityProbe {
        web_status_ok,
        database_connected,
        surrealdb_connected,
        site_identity_ok,
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
    let parse_project_names = site_parse_project_names(site);
    let candidates = project_dir_candidates_multi(&parse_project_names, &site.project_path);
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
    let files = read_parse_config_included_db_files(&site.site_id);
    if files.is_empty() && parse_scope_enabled(site) {
        preflight_warning(
            "parse_scope",
            "解析文件",
            "尚未由 aios-database sidecar 写入 included_db_files",
            Some(format!(
                "manual_db_nums={:?}, parse_db_types={:?}",
                site.manual_db_nums, site.parse_db_types
            )),
            Some("启动解析时会先请求 sidecar 计算解析文件范围".to_string()),
            Vec::new(),
        )
    } else if files.is_empty() {
        preflight_warning(
            "parse_scope",
            "解析文件",
            "未限制解析范围，可能触发全量解析",
            None,
            Some("建议选择 dbnum 或解析类型，避免误触发大范围解析".to_string()),
            Vec::new(),
        )
    } else {
        preflight_pass(
            "parse_scope",
            "解析文件",
            format!("已从配置读取 {} 个待解析 DB 文件", files.len()),
            Some(files.join(", ")),
        )
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

/// 解析站点解析/生成 worker 使用的 surql 脚本目录，返回一个【确实存在】的路径。
///
/// 历史上这里写死为 "resource/surreal"，但 worker 以 repo_root 为 CWD 运行，真实脚本
/// 位于 ../rs-core/resource/surreal，导致站点专属 SurrealDB 加载 schema 失败
/// （att_meta / 通用函数缺失）从而解析失败。这里按候选项探测，命中即用其路径：
/// AIOS_SURREAL_SCRIPT_DIR 环境变量 → 主配置 surreal_script_dir → repo_root/resource/surreal
/// → repo_root/../rs-core/resource/surreal；都不存在时回退旧默认值。
fn resolve_surreal_script_dir() -> String {
    let norm = |p: &Path| p.to_string_lossy().replace('\\', "/");

    if let Ok(v) = std::env::var("AIOS_SURREAL_SCRIPT_DIR") {
        let v = v.trim();
        if !v.is_empty() && Path::new(v).exists() {
            return norm(Path::new(v));
        }
    }

    let repo = repo_root().ok();
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(builder) = load_config_builder() {
        if let Ok(s) = builder.get_string("surreal_script_dir") {
            let s = s.trim().to_string();
            if !s.is_empty() {
                let p = PathBuf::from(&s);
                if p.is_absolute() {
                    candidates.push(p);
                } else if let Some(repo) = repo.as_ref() {
                    candidates.push(repo.join(&s));
                }
            }
        }
    }
    if let Some(repo) = repo.as_ref() {
        candidates.push(repo.join("resource/surreal"));
        if let Some(parent) = repo.parent() {
            candidates.push(parent.join("rs-core/resource/surreal"));
        }
    }
    for c in candidates {
        if c.exists() {
            return norm(&c);
        }
    }
    "resource/surreal".to_string()
}

fn current_exe_path() -> Result<PathBuf> {
    std::env::current_exe().context("获取当前 web_server 可执行文件失败")
}

fn packaged_install_root() -> Option<PathBuf> {
    let current = current_exe_path().ok()?;
    let bin_dir = current.parent()?;
    let root = bin_dir.parent()?;
    root.join("bin")
        .join("web_server.exe")
        .exists()
        .then_some(root.to_path_buf())
}

fn aios_database_exe_name() -> &'static str {
    if cfg!(windows) {
        "aios-database.exe"
    } else {
        "aios-database"
    }
}

fn surreal_exe_name() -> &'static str {
    if cfg!(windows) {
        "surreal.exe"
    } else {
        "surreal"
    }
}

fn bundled_surreal_binary() -> Option<PathBuf> {
    let current = current_exe_path().ok()?;
    let parent = current.parent()?;
    let candidate = parent.join("surreal").join(surreal_exe_name());
    candidate.exists().then_some(candidate)
}

fn managed_surreal_bin_string() -> String {
    bundled_surreal_binary()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| "surreal".to_string())
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

fn append_log_line(path: &Path, line: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}

#[derive(Clone, Copy)]
enum SidecarCliJobKind {
    Parse,
    Generate,
}

impl SidecarCliJobKind {
    fn label(self) -> &'static str {
        match self {
            SidecarCliJobKind::Parse => "解析",
            SidecarCliJobKind::Generate => "模型生成",
        }
    }

    fn key(self) -> &'static str {
        match self {
            SidecarCliJobKind::Parse => "parse",
            SidecarCliJobKind::Generate => "generate",
        }
    }

    fn log_path(self, site_id: &str) -> PathBuf {
        match self {
            SidecarCliJobKind::Parse => parse_log_path(site_id),
            SidecarCliJobKind::Generate => generate_log_path(site_id),
        }
    }
}

#[derive(Clone)]
struct ActiveSidecarJob {
    kind: SidecarCliJobKind,
    key: String,
    job_id: String,
    status: String,
}

fn active_sidecar_jobs() -> &'static Mutex<HashMap<String, ActiveSidecarJob>> {
    static ACTIVE_SIDECAR_JOBS: OnceLock<Mutex<HashMap<String, ActiveSidecarJob>>> =
        OnceLock::new();
    ACTIVE_SIDECAR_JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn active_sidecar_job(site_id: &str) -> Option<ActiveSidecarJob> {
    active_sidecar_jobs()
        .lock()
        .ok()
        .and_then(|guard| guard.get(site_id).cloned())
}

fn register_active_sidecar_job(site_id: &str, job: ActiveSidecarJob) {
    if let Ok(mut guard) = active_sidecar_jobs().lock() {
        guard.insert(site_id.to_string(), job);
    }
}

fn unregister_active_sidecar_job(site_id: &str, job_id: &str) {
    if let Ok(mut guard) = active_sidecar_jobs().lock() {
        let should_remove = guard
            .get(site_id)
            .map(|job| job.job_id == job_id)
            .unwrap_or(false);
        if should_remove {
            guard.remove(site_id);
        }
    }
}

async fn wait_for_sidecar_job_terminal(site_id: &str, job: &ActiveSidecarJob) -> bool {
    for _ in 0..SIDECAR_CANCEL_WAIT_ATTEMPTS {
        match active_sidecar_job(site_id) {
            Some(current) if current.job_id == job.job_id => {
                tokio::time::sleep(Duration::from_millis(WAIT_STEP_MS)).await;
            }
            _ => return true,
        }
    }
    false
}

fn terminal_sidecar_status_from_event(
    event: &Value,
) -> Option<crate::web_server::parse_sidecar_client::RunCliJobStatus> {
    let event_type = event.get("type").and_then(Value::as_str)?;
    if !matches!(event_type, "job_done" | "job_failed" | "job_cancelled") {
        return None;
    }
    let status = event
        .get("job")
        .and_then(|job| job.get("status"))
        .and_then(Value::as_str)
        .or_else(|| event.get("status").and_then(Value::as_str))?;
    let exit_code = event
        .get("job")
        .and_then(|job| job.get("exit_code"))
        .and_then(Value::as_i64)
        .and_then(|code| i32::try_from(code).ok());
    Some(crate::web_server::parse_sidecar_client::RunCliJobStatus {
        status: status.to_string(),
        exit_code,
    })
}

async fn terminal_sidecar_event_response(
    job_id: Arc<Mutex<Option<String>>>,
    terminal_status: Arc<Mutex<Option<crate::web_server::parse_sidecar_client::RunCliJobStatus>>>,
) -> Option<crate::web_server::parse_sidecar_client::RunCliJobResponse> {
    for _ in 0..20 {
        let job_id_value = job_id.lock().ok().and_then(|guard| guard.clone());
        let status = terminal_status.lock().ok().and_then(|guard| guard.clone());
        if let (Some(job_id), Some(status)) = (job_id_value, status) {
            if matches!(status.status.as_str(), "succeeded" | "failed" | "cancelled") {
                return Some(crate::web_server::parse_sidecar_client::RunCliJobResponse {
                    success: status.status == "succeeded",
                    exit_code: status.exit_code,
                    job_id,
                });
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    None
}

async fn run_sidecar_cli_job_with_site_events(
    site_id: &str,
    kind: SidecarCliJobKind,
    key: String,
    config_no_ext: String,
    cwd: String,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
) -> std::result::Result<
    crate::web_server::parse_sidecar_client::RunCliJobResponse,
    crate::web_server::parse_sidecar_client::SidecarProxyError,
> {
    let site_id_for_status = site_id.to_string();
    let log_path = stdout_path.clone();
    let label = kind.label();
    let key_for_status = key.clone();
    let submitted_job_id = Arc::new(Mutex::new(None::<String>));
    let terminal_status = Arc::new(Mutex::new(
        None::<crate::web_server::parse_sidecar_client::RunCliJobStatus>,
    ));
    let fallback_job_id = submitted_job_id.clone();
    let fallback_terminal_status = terminal_status.clone();
    let fallback_log_path = stdout_path.clone();
    let fallback_label = label.to_string();
    let mut event_stream_started = false;
    let result = crate::web_server::parse_sidecar_client::run_cli_job_with_status(
        &key,
        config_no_ext,
        cwd,
        stdout_path.to_string_lossy().to_string(),
        stderr_path.to_string_lossy().to_string(),
        move |job_id, status| {
            if let Ok(mut guard) = submitted_job_id.lock() {
                *guard = Some(job_id.to_string());
            }
            let exit_code = status
                .exit_code
                .map(|code| format!(", exit_code={code}"))
                .unwrap_or_default();
            append_log_line(
                &log_path,
                &format!(
                    "🛰️ sidecar {label} job {job_id} status={}{}",
                    status.status, exit_code
                ),
            );
            if status.status == "submitted" && !event_stream_started {
                event_stream_started = true;
                let mut event_rx =
                    crate::web_server::parse_sidecar_client::subscribe_cli_job_events(
                        key_for_status.clone(),
                        job_id.to_string(),
                    );
                let event_log_path = log_path.clone();
                let event_site_id = site_id_for_status.clone();
                let event_label = label.to_string();
                let event_terminal_status = terminal_status.clone();
                tokio::spawn(async move {
                    while let Some(event) = event_rx.recv().await {
                        append_log_line(
                            &event_log_path,
                            &sidecar_job_event_log_line(&event_label, &event),
                        );
                        if let Some(status) = terminal_sidecar_status_from_event(&event) {
                            if let Ok(mut guard) = event_terminal_status.lock() {
                                *guard = Some(status);
                            }
                        }
                        if let Err(err) = update_runtime(&event_site_id, RuntimeUpdate::default()) {
                            tracing::warn!(
                                site = %event_site_id,
                                "广播 sidecar job event 失败: {err}"
                            );
                        }
                    }
                });
            }
            match status.status.as_str() {
                "submitted" | "queued" | "running" | "cancelling" => {
                    register_active_sidecar_job(
                        &site_id_for_status,
                        ActiveSidecarJob {
                            kind,
                            key: key_for_status.clone(),
                            job_id: job_id.to_string(),
                            status: status.status.clone(),
                        },
                    );
                }
                "succeeded" | "failed" | "cancelled" => {
                    unregister_active_sidecar_job(&site_id_for_status, job_id);
                }
                _ => {}
            }
            if let Err(err) = update_runtime(&site_id_for_status, RuntimeUpdate::default()) {
                tracing::warn!(
                    site = %site_id_for_status,
                    job_id,
                    status = %status.status,
                    "广播 sidecar job 状态失败: {err}"
                );
            }
        },
    )
    .await;
    match result {
        Ok(response) => Ok(response),
        Err(err) => {
            if let Some(response) =
                terminal_sidecar_event_response(fallback_job_id, fallback_terminal_status).await
            {
                append_log_line(
                    &fallback_log_path,
                    &format!(
                        "⚠️ sidecar {fallback_label} HTTP 终态轮询失败，但已收到 websocket 终态事件；按 event status={}{} 继续",
                        if response.success {
                            "succeeded"
                        } else {
                            "failed/cancelled"
                        },
                        response
                            .exit_code
                            .map(|code| format!(", exit_code={code}"))
                            .unwrap_or_default()
                    ),
                );
                return Ok(response);
            }
            Err(err)
        }
    }
}

fn sidecar_job_event_log_line(label: &str, event: &Value) -> String {
    let event_type = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if event_type == "log_appended" {
        let stream = event.get("stream").and_then(Value::as_str).unwrap_or("log");
        let line = event.get("line").and_then(Value::as_str).unwrap_or("");
        return format!("🛰️ sidecar {label} {stream}: {line}");
    }
    let status = event
        .get("job")
        .and_then(|job| job.get("status"))
        .and_then(Value::as_str)
        .or_else(|| event.get("status").and_then(Value::as_str));
    let exit_code = event
        .get("job")
        .and_then(|job| job.get("exit_code"))
        .and_then(Value::as_i64)
        .map(|code| format!(", exit_code={code}"))
        .unwrap_or_default();
    match status {
        Some(status) => {
            format!("🛰️ sidecar {label} event {event_type}: status={status}{exit_code}")
        }
        None => format!("🛰️ sidecar {label} event {event_type}"),
    }
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

/// 等待端口被释放（与 `wait_for_port` 相反），用于停止互斥模式后确认端口已腾出。
async fn wait_for_port_free(port: u16, attempts: usize, delay_ms: u64) -> bool {
    for _ in 0..attempts {
        if !port_in_use("127.0.0.1", port) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
    !port_in_use("127.0.0.1", port)
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
pub(crate) struct DeployValidationReport {
    site_id: String,
    checked_at: String,
    blocking_count: usize,
    warning_count: usize,
    checks: Vec<DeployValidationCheck>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct DeployValidationCheck {
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

pub(crate) fn deploy_validation_check(
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

pub async fn refresh_deploy_validation_report(
    site_id: &str,
) -> Result<ManagedSiteDeployValidationReport> {
    let site = task::spawn_blocking({
        let site_id = site_id.to_string();
        move || get_site(&site_id)
    })
    .await
    .context("读取站点状态失败 (join error)")??
    .ok_or_else(|| anyhow!("站点不存在"))?;
    let _ = validate_deploy_readiness(&site).await?;
    deploy_validation_report(site_id)
}

fn spawn_deploy_validation_refresh(site_id: String, reason: &'static str) {
    tokio::spawn(async move {
        let site = match task::spawn_blocking({
            let site_id = site_id.clone();
            move || get_site(&site_id)
        })
        .await
        {
            Ok(Ok(Some(site))) => site,
            Ok(Ok(None)) => return,
            Ok(Err(err)) => {
                append_log_line(
                    &viewer_log_path(&site_id),
                    &format!("⚠️ 自动刷新部署验收失败（{reason}）：读取站点失败：{err}"),
                );
                return;
            }
            Err(err) => {
                append_log_line(
                    &viewer_log_path(&site_id),
                    &format!("⚠️ 自动刷新部署验收失败（{reason}）：join error：{err}"),
                );
                return;
            }
        };
        match validate_deploy_readiness(&site).await {
            Ok(report) => append_log_line(
                &viewer_log_path(&site_id),
                &format!(
                    "✅ 自动刷新部署验收完成（{reason}）：{} 个阻断 / {} 个警告",
                    report.blocking_count, report.warning_count
                ),
            ),
            Err(err) => append_log_line(
                &viewer_log_path(&site_id),
                &format!("⚠️ 自动刷新部署验收失败（{reason}）：{err:#}"),
            ),
        }
    });
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

fn discover_parquet_manifest_dbnums(parquet_root: &Path) -> Vec<u32> {
    let Ok(entries) = fs::read_dir(parquet_root) else {
        return Vec::new();
    };
    let mut dbnums = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let dbnum = name
                .strip_prefix("manifest_")
                .and_then(|value| value.strip_suffix(".json"))?;
            dbnum.parse::<u32>().ok()
        })
        .collect::<Vec<_>>();
    dbnums.sort_unstable();
    dbnums.dedup();
    dbnums
}

fn parquet_validation_dbnums(site: &ManagedProjectSite, parquet_root: &Path) -> Vec<u32> {
    if !site.manual_db_nums.is_empty() {
        return site.manual_db_nums.clone();
    }
    discover_parquet_manifest_dbnums(parquet_root)
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

async fn push_status_json_checks(
    report: &mut DeployValidationReport,
    client: &reqwest::Client,
    url: String,
    require_database: bool,
) {
    match client.get(&url).send().await {
        Ok(response) if response.status().is_success() => {
            let status = response.status();
            match response.json::<serde_json::Value>().await {
                Ok(value) => {
                    let database_connected = value
                        .get("database_connected")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let surrealdb_connected = value
                        .get("surrealdb_connected")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let ok = !require_database || (database_connected && surrealdb_connected);
                    report.push(deploy_validation_check(
                        "web_status",
                        "站点 /api/status JSON",
                        if ok { "pass" } else { "blocking" },
                        if ok {
                            if require_database {
                                format!("HTTP {status}，JSON 可解析且业务连接正常")
                            } else {
                                format!("HTTP {status}，JSON 可解析；当前站点未启用模型/Parquet 产物生成，数据库健康降级为提示项")
                            }
                        } else {
                            format!(
                                "HTTP {status}，但业务连接未就绪: database_connected={database_connected}, surrealdb_connected={surrealdb_connected}"
                            )
                        },
                        Some(value.to_string()),
                        Some(url.clone()),
                        None,
                    ));
                    report.push(deploy_validation_check(
                        "database_connected",
                        "业务数据库连接",
                        if database_connected { "pass" } else if require_database { "blocking" } else { "warning" },
                        if database_connected {
                            "database_connected=true".to_string()
                        } else if require_database {
                            "database_connected=false，Web 200 但业务数据库不可用".to_string()
                        } else {
                            "database_connected=false；轻量部署未启用模型/Parquet 生成，本项不阻断站点启动验收".to_string()
                        },
                        None,
                        Some(url.clone()),
                        None,
                    ));
                    report.push(deploy_validation_check(
                        "surrealdb_connected",
                        "SurrealDB 连接",
                        if surrealdb_connected {
                            "pass"
                        } else if require_database {
                            "blocking"
                        } else {
                            "warning"
                        },
                        if surrealdb_connected {
                            "surrealdb_connected=true".to_string()
                        } else if require_database {
                            "surrealdb_connected=false，站点 SurrealDB 连接不可用".to_string()
                        } else {
                            "surrealdb_connected=false；轻量部署未启用模型/Parquet 生成，本项不阻断站点启动验收".to_string()
                        },
                        None,
                        Some(url),
                        None,
                    ));
                }
                Err(err) => {
                    report.push(deploy_validation_check(
                        "web_status",
                        "站点 /api/status JSON",
                        "blocking",
                        format!("HTTP {status} 但响应不是有效 JSON: {err}"),
                        None,
                        Some(url.clone()),
                        None,
                    ));
                    report.push(deploy_validation_check(
                        "database_connected",
                        "业务数据库连接",
                        if require_database {
                            "blocking"
                        } else {
                            "warning"
                        },
                        "无法从 /api/status JSON 读取 database_connected".to_string(),
                        None,
                        Some(url.clone()),
                        None,
                    ));
                    report.push(deploy_validation_check(
                        "surrealdb_connected",
                        "SurrealDB 连接",
                        if require_database {
                            "blocking"
                        } else {
                            "warning"
                        },
                        "无法从 /api/status JSON 读取 surrealdb_connected".to_string(),
                        None,
                        Some(url),
                        None,
                    ));
                }
            }
        }
        Ok(response) => report.push(deploy_validation_check(
            "web_status",
            "站点 /api/status JSON",
            "blocking",
            format!("HTTP 状态异常: {}", response.status()),
            None,
            Some(url),
            None,
        )),
        Err(err) => report.push(deploy_validation_check(
            "web_status",
            "站点 /api/status JSON",
            "blocking",
            format!("HTTP 请求失败: {err}"),
            None,
            Some(url),
            None,
        )),
    }
}

async fn push_site_identity_check(
    report: &mut DeployValidationReport,
    client: &reqwest::Client,
    site: &ManagedProjectSite,
    url: String,
) {
    match client.get(&url).send().await {
        Ok(response) if response.status().is_success() => {
            let status = response.status();
            match response.json::<serde_json::Value>().await {
                Ok(value) => {
                    let site_id_ok = value
                        .get("site_id")
                        .and_then(|v| v.as_str())
                        .is_some_and(|id| id == site.site_id);
                    let port_ok = value
                        .get("web_listen_port")
                        .or_else(|| value.get("bind_port"))
                        .and_then(|v| v.as_u64())
                        .is_some_and(|port| port == site.web_port as u64);
                    let ok = site_id_ok && port_ok;
                    report.push(deploy_validation_check(
                        "site_identity",
                        "站点身份",
                        if ok { "pass" } else { "blocking" },
                        if ok {
                            format!("HTTP {status}，site_id/web_port 与受管站点一致")
                        } else {
                            format!(
                                "HTTP {status}，站点身份不匹配: site_id_ok={site_id_ok}, web_port_ok={port_ok}"
                            )
                        },
                        Some(value.to_string()),
                        Some(url),
                        None,
                    ));
                }
                Err(err) => report.push(deploy_validation_check(
                    "site_identity",
                    "站点身份",
                    "blocking",
                    format!("HTTP {status} 但响应不是有效 JSON: {err}"),
                    None,
                    Some(url),
                    None,
                )),
            }
        }
        Ok(response) => report.push(deploy_validation_check(
            "site_identity",
            "站点身份",
            "blocking",
            format!("HTTP 状态异常: {}", response.status()),
            None,
            Some(url),
            None,
        )),
        Err(err) => report.push(deploy_validation_check(
            "site_identity",
            "站点身份",
            "blocking",
            format!("HTTP 请求失败: {err}"),
            None,
            Some(url),
            None,
        )),
    }
}

fn extract_refno_for_url(value: &serde_json::Value) -> Option<String> {
    let refno = value
        .get("node")
        .and_then(|node| node.get("refno"))
        .or_else(|| value.get("refno"))?;
    match refno {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

async fn push_json_success_check(
    report: &mut DeployValidationReport,
    client: &reqwest::Client,
    key: impl Into<String>,
    label: impl Into<String>,
    url: String,
    failure_status: &'static str,
) -> Option<serde_json::Value> {
    let key = key.into();
    let label = label.into();
    match client.get(&url).send().await {
        Ok(response) if response.status().is_success() => {
            let status = response.status();
            match response.json::<serde_json::Value>().await {
                Ok(value) => {
                    let success = value
                        .get("success")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(true);
                    report.push(deploy_validation_check(
                        key,
                        label,
                        if success { "pass" } else { failure_status },
                        if success {
                            format!("HTTP {status}，JSON 响应可用")
                        } else {
                            format!("HTTP {status}，业务响应 success=false")
                        },
                        Some(value.to_string()),
                        Some(url),
                        None,
                    ));
                    Some(value)
                }
                Err(err) => {
                    report.push(deploy_validation_check(
                        key,
                        label,
                        "blocking",
                        format!("HTTP {status} 但响应不是有效 JSON: {err}"),
                        None,
                        Some(url),
                        None,
                    ));
                    None
                }
            }
        }
        Ok(response) => {
            report.push(deploy_validation_check(
                key,
                label,
                "blocking",
                format!("HTTP 状态异常: {}", response.status()),
                None,
                Some(url),
                None,
            ));
            None
        }
        Err(err) => {
            report.push(deploy_validation_check(
                key,
                label,
                "blocking",
                format!("HTTP 请求失败: {err}"),
                None,
                Some(url),
                None,
            ));
            None
        }
    }
}

async fn push_e3d_api_checks(
    report: &mut DeployValidationReport,
    client: &reqwest::Client,
    base_url: &str,
) {
    let world_url = format!("{base_url}/api/e3d/world-root");
    let Some(world_value) = push_json_success_check(
        report,
        client,
        "api_e3d_world_root",
        "E3D world-root API",
        world_url,
        "blocking",
    )
    .await
    else {
        return;
    };

    let Some(root_refno) = extract_refno_for_url(&world_value) else {
        report.push(deploy_validation_check(
            "api_e3d_root_refno",
            "E3D root refno",
            "blocking",
            "world-root 响应缺少 node.refno，无法继续验证 subtree/visible-insts",
            Some(world_value.to_string()),
            None,
            None,
        ));
        return;
    };
    let encoded_root = urlencoding::encode(&root_refno);

    push_json_success_check(
        report,
        client,
        "api_e3d_subtree_refnos",
        "E3D subtree-refnos API",
        format!("{base_url}/api/e3d/subtree-refnos/{encoded_root}?include_self=true&max_depth=1&limit=20"),
        "warning",
    )
    .await;
    push_json_success_check(
        report,
        client,
        "api_e3d_visible_insts",
        "E3D visible-insts API",
        format!("{base_url}/api/e3d/visible-insts/{encoded_root}"),
        "blocking",
    )
    .await;
}

async fn wait_for_business_status_ok(site: &ManagedProjectSite) -> bool {
    let client = match reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };
    let url = format!("{}/api/status", site_probe_base_url(site));
    for _ in 0..WAIT_HTTP_ATTEMPTS {
        let ok = match client.get(&url).send().await {
            Ok(response) if response.status().is_success() => {
                match response.json::<serde_json::Value>().await {
                    Ok(value) => {
                        value
                            .get("database_connected")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                            && value
                                .get("surrealdb_connected")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false)
                    }
                    Err(_) => false,
                }
            }
            _ => false,
        };
        if ok {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(WAIT_STEP_MS)).await;
    }
    false
}

fn site_config_meshes_path(site: &ManagedProjectSite) -> Option<PathBuf> {
    let raw = fs::read_to_string(&site.config_path).ok()?;
    let value = toml::from_str::<toml::Value>(&raw).ok()?;
    value
        .get("meshes_path")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn normalize_mesh_serve_root(path: PathBuf) -> PathBuf {
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

fn mesh_serve_root_for_site(site: &ManagedProjectSite) -> PathBuf {
    let path = site_config_meshes_path(site)
        .unwrap_or_else(|| aios_core::get_db_option().get_meshes_path());
    normalize_mesh_serve_root(path)
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
    let local_base = site_probe_base_url(site);
    let access_base = site_access_base_url(site);

    let require_database_health = site.gen_model || site.gen_mesh || site.export_parquet;
    push_status_json_checks(
        &mut report,
        &client,
        format!("{local_base}/api/status"),
        require_database_health,
    )
    .await;
    push_site_identity_check(
        &mut report,
        &client,
        site,
        format!("{local_base}/api/site/identity"),
    )
    .await;
    push_e3d_api_checks(&mut report, &client, &local_base).await;

    if let Some(viewer_url) = site.viewer_url.clone() {
        push_required_http_check(
            &mut report,
            &client,
            "viewer_entry_url",
            "plant3d-web Viewer",
            viewer_url,
            64,
        )
        .await;
    } else {
        report.push(deploy_validation_check(
            "viewer_entry_url",
            "plant3d-web Viewer",
            "warning",
            "未记录 plant3d-web Viewer URL，跳过 Viewer 验收",
            Some(access_base),
            None,
            None,
        ));
    }

    if site.export_parquet {
        let output_project = site_source_project_name(site);
        let parquet_root = site_runtime_dir(&site.site_id)
            .join("output")
            .join(&output_project)
            .join("parquet");
        let validation_dbnums = parquet_validation_dbnums(site, &parquet_root);
        if validation_dbnums.is_empty() {
            report.push(deploy_validation_check(
                "parquet_scope",
                "Parquet 目标",
                "blocking",
                "未发现可验收的 Parquet manifest；全库部署时至少需要生成一个 manifest_<dbnum>.json",
                Some(parquet_root.display().to_string()),
                None,
                None,
            ));
        }
        for dbnum in validation_dbnums {
            let manifest_path = parquet_root.join(format!("manifest_{dbnum}.json"));
            let dbnum_dir = parquet_root.join(dbnum.to_string());
            let instances_path = dbnum_dir.join("instances.parquet");
            let geo_instances_path = dbnum_dir.join("geo_instances.parquet");
            let transforms_path = dbnum_dir.join("transforms.parquet");
            let aabb_path = dbnum_dir.join("aabb.parquet");

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
            push_required_file_check(
                &mut report,
                format!("parquet_transforms_{dbnum}"),
                format!("transforms.parquet {dbnum}"),
                &transforms_path,
                1,
            );
            push_required_file_check(
                &mut report,
                format!("parquet_aabb_{dbnum}"),
                format!("aabb.parquet {dbnum}"),
                &aabb_path,
                1,
            );
            for check in crate::web_server::site_data_validation::validate_dbnum_parquet_data(
                dbnum,
                &dbnum_dir,
                &mesh_serve_root_for_site(site),
            ) {
                report.push(check);
            }

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
            push_required_http_check(
                &mut report,
                &client,
                format!("http_parquet_transforms_{dbnum}"),
                format!("HTTP transforms.parquet {dbnum}"),
                format!(
                    "{local_base}/files/output/{encoded_project}/parquet/{dbnum}/transforms.parquet"
                ),
                1,
            )
            .await;
            push_required_http_check(
                &mut report,
                &client,
                format!("http_parquet_aabb_{dbnum}"),
                format!("HTTP aabb.parquet {dbnum}"),
                format!("{local_base}/files/output/{encoded_project}/parquet/{dbnum}/aabb.parquet"),
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

    let mesh_root = mesh_serve_root_for_site(site);
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
    } else if site.gen_model || site.gen_mesh {
        report.push(deploy_validation_check(
            "mesh_glb_file",
            "GLB 模型文件",
            "blocking",
            "未找到可供 plant3d-web 加载的 .glb 模型文件",
            Some(mesh_root.display().to_string()),
            None,
            None,
        ));
    } else {
        report.push(deploy_validation_check(
            "mesh_glb_file",
            "GLB 模型文件",
            "warning",
            "站点未启用模型/网格生成，跳过 GLB 验收",
            Some(mesh_root.display().to_string()),
            None,
            None,
        ));
    }

    write_deploy_validation_report(&report)?;
    Ok(report)
}

// ─── Port helpers ───────────────────────────────────────────────────────────

#[cfg(windows)]
fn netstat_listening_pid_for_port(line: &str, port: u16) -> Option<u32> {
    if !line.contains("LISTENING") {
        return None;
    }

    let mut fields = line.split_whitespace();
    let _proto = fields.next()?;
    let local_addr = fields.next()?;
    let local_port = local_addr.rsplit(':').next()?.parse::<u16>().ok()?;
    if local_port != port {
        return None;
    }

    fields.last()?.parse::<u32>().ok()
}

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
        let ids = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| netstat_listening_pid_for_port(line, port))
            .collect::<Vec<_>>();
        Ok(ids)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = port;
        Ok(Vec::new())
    }
}

pub(crate) async fn kill_processes_on_port(port: u16) -> Result<(Vec<u32>, Vec<u32>)> {
    let current_pid = std::process::id();
    let mut pids = process_ids_on_port(port).await?;
    pids.sort_unstable();
    pids.dedup();

    let targets = pids
        .into_iter()
        .filter(|pid| *pid != 0 && *pid != current_pid)
        .collect::<Vec<_>>();
    for pid in &targets {
        kill_pid(*pid).await?;
    }

    let mut remaining = process_ids_on_port(port).await?;
    remaining.sort_unstable();
    remaining.dedup();
    Ok((targets, remaining))
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
        match output {
            Ok(out) => String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|line| netstat_listening_pid_for_port(line, port))
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

// ─── Process registry（PID + 启动时刻 token，防误杀） ─────────────────────────
//
// 仅靠 pid 杀进程在 pid 被 OS 复用后会误杀无关进程。这里在子进程拉起时登记
// (site_id, role) -> (pid, start_token)，其中 start_token 取自该 pid 的进程启动时刻
// （`sysinfo::Process::start_time()`，Unix 秒）。kill 前重新读取目标 pid 的启动时刻，
// 与登记值一致才执行 kill，否则判定为「pid 已被复用」并跳过，避免误杀。

/// 进程角色（与 ManagedProjectSite 的各 *_pid 一一对应）。
const PROC_ROLE_DB: &str = "db";
const PROC_ROLE_WEB: &str = "web";
const PROC_ROLE_VIEWER: &str = "viewer";
const PROC_ROLE_PARSE: &str = "parse";

/// 读取指定 pid 的进程启动时刻（Unix 秒）作为「同一进程」判定 token。
/// 返回 None 表示进程不存在或无法采样（调用方据此判定该 pid 已消亡）。
fn process_start_token(pid: u32) -> Option<u64> {
    if pid == 0 {
        return None;
    }
    let target = Pid::from_u32(pid);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[target]),
        true,
        ProcessRefreshKind::nothing(),
    );
    system.process(target).map(|proc| proc.start_time())
}

/// 登记一个受管子进程（覆盖式 upsert）。失败仅告警，不影响主流程。
fn register_process(site_id: &str, role: &str, pid: u32) {
    if pid == 0 {
        return;
    }
    let token = process_start_token(pid).map(|value| value as i64);
    let result = with_conn(|conn| {
        conn.execute(
            &format!(
                "INSERT OR REPLACE INTO {table} (site_id, role, pid, start_token, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                table = PROC_REGISTRY_TABLE
            ),
            params![site_id, role, pid as i64, token, now_rfc3339()],
        )?;
        Ok(())
    });
    if let Err(err) = result {
        tracing::warn!(site = %site_id, role, pid, "登记受管进程失败: {err}");
    }
}

/// 移除某站点某角色的进程登记。
fn unregister_process(site_id: &str, role: &str) {
    let _ = with_conn(|conn| {
        conn.execute(
            &format!(
                "DELETE FROM {table} WHERE site_id = ?1 AND role = ?2",
                table = PROC_REGISTRY_TABLE
            ),
            params![site_id, role],
        )?;
        Ok(())
    });
}

/// 清空某站点的全部进程登记（停站 / 删除站点时调用）。
fn unregister_site_processes(site_id: &str) {
    let _ = with_conn(|conn| {
        conn.execute(
            &format!(
                "DELETE FROM {table} WHERE site_id = ?1",
                table = PROC_REGISTRY_TABLE
            ),
            params![site_id],
        )?;
        Ok(())
    });
}

fn registered_site_processes(site_id: &str) -> Vec<(String, u32)> {
    with_conn(|conn| {
        let mut stmt = conn.prepare(&format!(
            "SELECT role, pid FROM {table} WHERE site_id = ?1",
            table = PROC_REGISTRY_TABLE
        ))?;
        let rows = stmt.query_map([site_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u32))
        })?;
        let mut processes = Vec::new();
        for row in rows {
            let (role, pid) = row?;
            if pid != 0 {
                processes.push((role, pid));
            }
        }
        Ok(processes)
    })
    .unwrap_or_default()
}

async fn kill_registered_site_processes(site_id: &str) {
    for (role, pid) in registered_site_processes(site_id) {
        let _ = kill_pid_guarded(site_id, &role, pid).await;
    }
}

/// 读取登记的 start_token；外层 None=无登记行，内层 None=登记时未取到 token。
fn registered_start_token(site_id: &str, role: &str) -> Option<Option<u64>> {
    with_conn(|conn| {
        let token = conn
            .query_row(
                &format!(
                    "SELECT start_token FROM {table} WHERE site_id = ?1 AND role = ?2",
                    table = PROC_REGISTRY_TABLE
                ),
                params![site_id, role],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()?;
        Ok(token.map(|inner| inner.map(|value| value as u64)))
    })
    .ok()
    .flatten()
}

// ─── Data-dir 所有权登记表（file/ws 互斥真源：db_data_path 的 RocksDB 锁）──────
//
// 互斥真源是 `db_data_path` 的 RocksDB 排他锁；本表记录"当前是哪个进程/模式/角色
// 打开了某个 data dir"，供启动前做：自方优雅停 / 进行中写保护 / stale 清理 /
// 陌生进程 fail-fast。Phase 1 仅提供模型 + 规范化 + 持久化 CRUD + 活性判定，
// 尚未接入 ensure_site_db_started / spawn_* 路径（Phase 3 接入时移除 dead_code 标注）。

const DB_DIR_ROLE_SERVING: &str = "serving";
const DB_DIR_ROLE_GENERATING: &str = "generating";
const DB_DIR_ROLE_PARSING: &str = "parsing";

/// 将 `db_data_path` 规范化为跨平台唯一键：去除 Windows `\\?\` 前缀、统一为正斜杠、
/// 去掉结尾斜杠；Windows 文件系统大小写不敏感，故统一小写。
fn canonical_data_dir(path: &str) -> String {
    let trimmed = path.trim();
    let without_prefix = trimmed
        .strip_prefix(r"\\?\")
        .or_else(|| trimmed.strip_prefix("//?/"))
        .unwrap_or(trimmed);
    let normalized = without_prefix.replace('\\', "/");
    let tail_trimmed = normalized.trim_end_matches('/');
    let result = if tail_trimmed.is_empty() {
        normalized
    } else {
        tail_trimmed.to_string()
    };
    if cfg!(windows) {
        result.to_ascii_lowercase()
    } else {
        result
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct DbDirOwner {
    data_dir: String,
    site_id: String,
    owner_pid: u32,
    mode: ManagedSiteDbMode,
    role: String,
    start_token: Option<u64>,
    updated_at: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DbDirOwnerLiveness {
    /// 登记进程仍存活且启动时刻一致（同一进程）。
    Alive,
    /// 无存活进程（已亡 / PID 被复用 / token 不符），属陈旧残留。
    Stale,
}

/// 登记某 data dir 当前持有者（覆盖式 upsert）。失败仅告警。
fn register_db_dir_owner(
    data_dir: &str,
    site_id: &str,
    pid: u32,
    mode: ManagedSiteDbMode,
    role: &str,
) {
    if pid == 0 {
        return;
    }
    let key = canonical_data_dir(data_dir);
    let token = process_start_token(pid).map(|value| value as i64);
    let result = with_conn(|conn| {
        conn.execute(
            &format!(
                "INSERT OR REPLACE INTO {table} \
                 (data_dir, site_id, owner_pid, mode, role, start_token, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                table = DB_DIR_OWNER_TABLE
            ),
            params![
                key,
                site_id,
                pid as i64,
                managed_db_mode_to_str(mode),
                role,
                token,
                now_rfc3339()
            ],
        )?;
        Ok(())
    });
    if let Err(err) = result {
        tracing::warn!(site = %site_id, data_dir = %key, role, pid, "登记 data dir 持有者失败: {err}");
    }
}

/// 移除某 data dir 的持有者登记。
fn unregister_db_dir_owner(data_dir: &str) {
    let key = canonical_data_dir(data_dir);
    let _ = with_conn(|conn| {
        conn.execute(
            &format!(
                "DELETE FROM {table} WHERE data_dir = ?1",
                table = DB_DIR_OWNER_TABLE
            ),
            params![key],
        )?;
        Ok(())
    });
}

/// 读取某 data dir 的当前持有者登记。
#[allow(dead_code)]
fn db_dir_owner(data_dir: &str) -> Option<DbDirOwner> {
    let key = canonical_data_dir(data_dir);
    with_conn(|conn| {
        let owner = conn
            .query_row(
                &format!(
                    "SELECT data_dir, site_id, owner_pid, mode, role, start_token, updated_at \
                     FROM {table} WHERE data_dir = ?1",
                    table = DB_DIR_OWNER_TABLE
                ),
                params![key],
                |row| {
                    Ok(DbDirOwner {
                        data_dir: row.get::<_, String>(0)?,
                        site_id: row.get::<_, String>(1)?,
                        owner_pid: row.get::<_, i64>(2)? as u32,
                        mode: db_mode_from_string(
                            row.get::<_, Option<String>>(3)?,
                            ManagedSiteDbMode::Ws,
                        ),
                        role: row.get::<_, String>(4)?,
                        start_token: row.get::<_, Option<i64>>(5)?.map(|value| value as u64),
                        updated_at: row.get::<_, String>(6)?,
                    })
                },
            )
            .optional()?;
        Ok(owner)
    })
    .ok()
    .flatten()
}

/// 判定登记持有者是否仍是"同一活进程"（pid 存活且启动时刻一致）。
/// 无 token 时退化为仅看 pid 是否存活。
#[allow(dead_code)]
fn db_dir_owner_liveness(owner: &DbDirOwner) -> DbDirOwnerLiveness {
    if !pid_running(Some(owner.owner_pid)) {
        return DbDirOwnerLiveness::Stale;
    }
    match owner.start_token {
        Some(expected) => match process_start_token(owner.owner_pid) {
            Some(actual) if actual == expected => DbDirOwnerLiveness::Alive,
            _ => DbDirOwnerLiveness::Stale,
        },
        None => DbDirOwnerLiveness::Alive,
    }
}

// ─── Phase 2：data dir 互斥决策引擎（尚未接入；Phase 3 接入时移除 dead_code）─────

/// 每个 canonical data dir 一把可跨 await 持有的异步锁，串行化"查-停-取 + 调用方
/// 随后的 spawn/register"，保证对同一数据目录的接管是原子的。
#[allow(dead_code)]
fn data_dir_lock(data_dir: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>,
    > = OnceLock::new();
    let map = LOCKS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let key = canonical_data_dir(data_dir);
    let mut guard = map.lock().expect("data_dir_lock 注册表锁中毒");
    guard
        .entry(key)
        .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

/// 探测某 data dir 的 RocksDB 锁当前是否可获取（true=空闲/可接管）。
/// Windows：RocksDB 以独占方式持有 `LOCK` 文件，能以写方式打开即说明无人持锁。
/// Unix：RocksDB 用 advisory flock，open 判不出，保守返回 true（由登记表 + Phase 4 lsof 兜底）。
#[allow(dead_code)]
fn data_dir_lock_acquirable(data_dir: &str) -> bool {
    let lock_path = std::path::Path::new(data_dir).join("LOCK");
    if !lock_path.exists() {
        return true;
    }
    #[cfg(windows)]
    {
        std::fs::OpenOptions::new()
            .write(true)
            .open(&lock_path)
            .is_ok()
    }
    #[cfg(not(windows))]
    {
        let _ = lock_path;
        true
    }
}

/// 互斥获取结果：复用现有持有者（无需重启），或目录已空闲可继续 spawn。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DataDirAcquire {
    ReuseExisting,
    Proceed,
}

/// 优雅停掉本站点登记的旧持有者，并有界等待 RocksDB 锁释放。
#[allow(dead_code)]
async fn graceful_stop_db_dir_owner(site: &ManagedProjectSite, owner: &DbDirOwner) -> Result<()> {
    // kill_pid 内部已是"先温和(SIGTERM/taskkill /T)→宽限→必要时强杀(/F)"，给 RocksDB flush 机会。
    let _ = kill_pid(owner.owner_pid).await;
    unregister_db_dir_owner(&site.db_data_path);
    for _ in 0..WAIT_PORT_FREE_ATTEMPTS {
        if data_dir_lock_acquirable(&site.db_data_path) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(WAIT_STEP_MS)).await;
    }
    if data_dir_lock_acquirable(&site.db_data_path) {
        Ok(())
    } else {
        bail!(
            "停止旧持有者(pid={})后 data dir 锁仍未释放: {}",
            owner.owner_pid,
            canonical_data_dir(&site.db_data_path)
        )
    }
}

/// 决策矩阵（以 data dir 的 RocksDB 锁为真源）：
/// - 自方 serving-ws + 想要 ws 同目录且健康 → 复用；
/// - 自方 serving + 想要 file / 需全新 ws → 优雅停旧 server → 继续；
/// - 自方 generating/parsing（在写）→ 绝不杀，fail-fast（等待交由调用层有界重试）；
/// - 登记为陈旧残留（无活进程）→ 清理登记 → 继续；
/// - 活着但非本站（陌生/外部）持有 → fail-fast，不杀；
/// - 无登记但锁被外部进程持有 → fail-fast。
#[allow(dead_code)]
async fn resolve_data_dir_conflict(
    site: &ManagedProjectSite,
    desired_mode: ManagedSiteDbMode,
    desired_role: &str,
) -> Result<DataDirAcquire> {
    let dir = canonical_data_dir(&site.db_data_path);
    match db_dir_owner(&site.db_data_path) {
        Some(owner) => match db_dir_owner_liveness(&owner) {
            DbDirOwnerLiveness::Stale => {
                tracing::info!(
                    site = %site.site_id, data_dir = %dir, owner_pid = owner.owner_pid,
                    "data dir 持有者登记为陈旧残留，清理后接管"
                );
                unregister_db_dir_owner(&site.db_data_path);
                Ok(DataDirAcquire::Proceed)
            }
            DbDirOwnerLiveness::Alive => {
                if owner.site_id != site.site_id {
                    bail!(
                        "data dir {dir} 正被其他持有者占用 (site={}, pid={}, mode={}, role={})，为避免误杀拒绝接管",
                        owner.site_id,
                        owner.owner_pid,
                        managed_db_mode_to_str(owner.mode),
                        owner.role
                    );
                }
                match owner.role.as_str() {
                    DB_DIR_ROLE_GENERATING | DB_DIR_ROLE_PARSING => bail!(
                        "data dir {dir} 上有进行中的 {} 操作 (pid={})，请等待其完成后再启动（不中断在写进程）",
                        owner.role,
                        owner.owner_pid
                    ),
                    _ => {
                        if desired_mode == ManagedSiteDbMode::Ws
                            && owner.mode == ManagedSiteDbMode::Ws
                            && desired_role == DB_DIR_ROLE_SERVING
                            && site_db_running(site)
                        {
                            return Ok(DataDirAcquire::ReuseExisting);
                        }
                        graceful_stop_db_dir_owner(site, &owner).await?;
                        Ok(DataDirAcquire::Proceed)
                    }
                }
            }
        },
        None => {
            if data_dir_lock_acquirable(&site.db_data_path) {
                return Ok(DataDirAcquire::Proceed);
            }
            // 无登记但锁被占用：db_port 为本站点建站唯一预留的专属端口，占用者多为本站
            // 未登记的残留 server（如旧方案/崩溃遗留）。按 D2 二级兜底：停掉占用本站点
            // db_port 的进程后复检锁；仍未释放才 fail-fast（不盲杀陌生进程）。
            if stop_site_ws_db_for_exclusivity(site).await
                && data_dir_lock_acquirable(&site.db_data_path)
            {
                return Ok(DataDirAcquire::Proceed);
            }
            bail!("data dir {dir} 的 RocksDB 锁被未登记进程持有且无法清理，拒绝接管（避免误杀）");
        }
    }
}

/// 互斥地为某站点接管 data dir：持锁 → 应用决策矩阵 → 返回结果 + 锁守卫。
/// 调用方需持有返回的 guard 直到完成 spawn + register_db_dir_owner 后再 drop，
/// 以保证"查-停-取-登记"整体原子。
#[allow(dead_code)]
async fn acquire_data_dir(
    site: &ManagedProjectSite,
    desired_mode: ManagedSiteDbMode,
    desired_role: &str,
) -> Result<(DataDirAcquire, tokio::sync::OwnedMutexGuard<()>)> {
    let guard = data_dir_lock(&site.db_data_path).lock_owned().await;
    let outcome = resolve_data_dir_conflict(site, desired_mode, desired_role).await?;
    Ok((outcome, guard))
}

/// 守卫式 kill：用 (pid, 启动时刻) 双重校验确认仍是登记的同一进程后才杀，
/// 杀完移除登记。返回 true=已执行 kill；false=因身份不符/进程已亡而跳过。
async fn kill_pid_guarded(site_id: &str, role: &str, pid: u32) -> Result<bool> {
    if pid == 0 {
        unregister_process(site_id, role);
        return Ok(false);
    }
    match registered_start_token(site_id, role) {
        Some(Some(expected)) => match process_start_token(pid) {
            Some(actual) if actual == expected => {
                kill_pid(pid).await?;
            }
            Some(actual) => {
                tracing::warn!(
                    site = %site_id, role, pid, expected, actual,
                    "进程启动时刻不匹配，疑似 PID 已被系统复用，跳过 kill 防误杀"
                );
                unregister_process(site_id, role);
                return Ok(false);
            }
            None => {
                // 目标进程已不存在，无需 kill。
                unregister_process(site_id, role);
                return Ok(false);
            }
        },
        // 有登记行但当初没采到 token：尽力按 pid 存活性兜底，避免误杀已消亡 pid。
        Some(None) => {
            if !pid_running(Some(pid)) {
                unregister_process(site_id, role);
                return Ok(false);
            }
            tracing::warn!(
                site = %site_id, role, pid,
                "登记缺少启动时刻 token，按 pid 存活性兜底执行 kill（无法做复用校验）"
            );
            kill_pid(pid).await?;
        }
        // 完全没有登记（历史站点 / 登记前的旧数据）：保持旧逻辑直接 kill。
        None => {
            tracing::warn!(
                site = %site_id, role, pid,
                "无进程登记，按旧逻辑直接 kill（无法做复用校验）"
            );
            kill_pid(pid).await?;
        }
    }
    unregister_process(site_id, role);
    Ok(true)
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
        let _ = kill_pid_guarded(site_id, PROC_ROLE_DB, pid).await;
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

    let parse_plan = load_parse_plan_from_sidecar(&site).await?;

    task::spawn_blocking({
        let site = site.clone();
        let db_user = db_user.clone();
        let db_password = db_password.clone();
        let parse_plan = parse_plan.clone();
        move || write_site_files_with_parse_plan(&site, &db_user, &db_password, Some(&parse_plan))
    })
    .await
    .context("写入站点配置失败 (join error)")??;

    let config_path = parse_config_path(&site.site_id);
    let config_no_ext = config_path_without_toml(&config_path);
    let repo = repo_root()?;

    let parse_started_at = now_rfc3339();
    let parse_started_instant = Instant::now();
    update_runtime(
        &site.site_id,
        RuntimeUpdate {
            status: Some(ManagedSiteStatus::Draft),
            parse_status: Some(ManagedSiteParseStatus::Running),
            parse_pid: Some(None),
            last_error: Some(None),
            last_parse_started_at: Some(Some(parse_started_at)),
            last_parse_finished_at: Some(None),
            last_parse_duration_ms: Some(None),
            ..Default::default()
        },
    )?;

    let job = run_sidecar_cli_job_with_site_events(
        &site.site_id,
        SidecarCliJobKind::Parse,
        format!("parse:{}", site.site_id),
        config_no_ext,
        repo.to_string_lossy().to_string(),
        parse_log_path(&site.site_id),
        parse_log_path(&site.site_id),
    )
    .await
    .map_err(|err| anyhow!("aios-database sidecar 解析作业失败: {}", err.message))?;
    let parse_finished_at = now_rfc3339();
    let parse_duration_ms = parse_started_instant.elapsed().as_millis() as u64;
    if job.success {
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
        let message = format!("解析失败，退出码: {:?}", job.exit_code);
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

    let gen_config_path = generation_config_path(&site.site_id);
    let config_no_ext = config_path_without_toml(&gen_config_path);
    let repo = repo_root()?;
    update_runtime(
        &site.site_id,
        RuntimeUpdate {
            status: Some(ManagedSiteStatus::Starting),
            parse_status: Some(ManagedSiteParseStatus::Parsed),
            parse_pid: Some(None),
            last_error: Some(None),
            ..Default::default()
        },
    )?;

    let job = run_sidecar_cli_job_with_site_events(
        &site.site_id,
        SidecarCliJobKind::Generate,
        format!("generate:{}", site.site_id),
        config_no_ext,
        repo.to_string_lossy().to_string(),
        generate_log_path(&site.site_id),
        generate_log_path(&site.site_id),
    )
    .await
    .map_err(|err| anyhow!("aios-database sidecar 模型生成作业失败: {}", err.message))?;
    if job.success {
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
        let message = format!("模型生成失败，退出码: {:?}", job.exit_code);
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
    let mut command = Command::new(managed_surreal_bin_string());
    command
        .arg("start")
        .arg("--log")
        .arg("info")
        .arg("--user")
        .arg(&db_user)
        .arg("--pass")
        .arg(&db_password)
        .arg("--bind")
        .arg(format!("127.0.0.1:{}", site.db_port))
        .arg(format!("rocksdb://{}", site.db_data_path))
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    isolate_process_group(&mut command);
    let child = command.spawn().context("启动 SurrealDB 失败")?;
    let pid = child.id().unwrap_or_default();
    register_process(&site.site_id, PROC_ROLE_DB, pid);
    // ws 运行时：登记本站点为 db_data_path 的 RocksDB 持有者（serving）。
    register_db_dir_owner(
        &site.db_data_path,
        &site.site_id,
        pid,
        ManagedSiteDbMode::Ws,
        DB_DIR_ROLE_SERVING,
    );
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
    let pid = child.id().unwrap_or_default();
    register_process(&site.site_id, PROC_ROLE_WEB, pid);
    Ok(pid)
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

fn managed_nginx_required() -> bool {
    std::env::var("AIOS_REQUIRE_NGINX")
        .map(|value| matches!(value.trim(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
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
    if let Some(port) = site.viewer_port {
        if port == site.db_port || port == site.web_port {
            tracing::warn!(
                site = %site.site_id,
                port,
                "历史 Viewer 端口与站点 DB/Web 端口冲突，改为自动选择端口"
            );
        } else if port_in_use("127.0.0.1", port) {
            if viewer_http_ok(port).await {
                return Ok((port, true));
            }
            tracing::warn!(
                site = %site.site_id,
                port,
                "历史 Viewer 端口已被非 plant3d-web 进程占用，改为自动选择端口"
            );
        } else {
            return Ok((port, false));
        }
    }

    if let Some(port) = configured_port {
        if port == site.db_port || port == site.web_port {
            bail!("AIOS_VIEWER_PORT {} 与站点 DB/Web 端口冲突", port);
        }
        if port_in_use("127.0.0.1", port) {
            if viewer_http_ok(port).await {
                return Ok((port, true));
            }
            bail!("AIOS_VIEWER_PORT {} 已被非 plant3d-web 进程占用", port);
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

/// 探测 plant3d-web 构建产物的 base 路径。
///
/// plant3d-web 可能以非根 base 构建（例如随安装包发布时 `VITE_BASE_PATH=/viewer/`），
/// 此时 `dist/index.html` 会引用 `/viewer/assets/...`。受管 Viewer 必须保持
/// plant3d-web 原生根路由，所以这里仅用于识别旧的非根 dist 并触发重建。
fn detect_viewer_base_path(viewer_dir: &Path) -> String {
    let index = viewer_dir.join("dist").join("index.html");
    let Ok(html) = fs::read_to_string(&index) else {
        return "/".to_string();
    };
    // 取第一处引用 assets/ 的绝对路径前缀：从 "assets/" 向前回溯到最近的引号。
    if let Some(apos) = html.find("assets/") {
        if let Some(qpos) = html[..apos].rfind('"') {
            let prefix = &html[qpos + 1..apos];
            if prefix.starts_with('/') {
                return prefix.to_string();
            }
        }
    }
    "/".to_string()
}

fn normalize_viewer_base_url(value: impl AsRef<str>) -> Option<String> {
    let trimmed = value.as_ref().trim().trim_end_matches('/').to_string();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn configured_viewer_base_url(site: &ManagedProjectSite) -> Option<String> {
    std::env::var("AIOS_VIEWER_BASE_URL")
        .ok()
        .and_then(|value| normalize_viewer_base_url(value))
        .or_else(|| {
            site.public_entry_url
                .as_deref()
                .and_then(|value| normalize_viewer_base_url(value))
        })
}

fn build_viewer_url(site: &ManagedProjectSite, port: u16) -> String {
    let base = configured_viewer_base_url(site)
        .or_else(|| {
            super::get_local_ip_via_udp()
                .ok()
                .map(|ip| format!("http://{ip}:{port}"))
        })
        .unwrap_or_else(|| format!("http://127.0.0.1:{port}"));
    let project = site_source_project_name(site);
    // plant3d-web is a standalone customer-facing site. It should discover the
    // backend through same-origin config/proxying, not through admin-only
    // `backend=...` query wrapping.
    let mut url = format!(
        "{}/?output_project={}",
        base.trim_end_matches('/'),
        urlencoding::encode(&project)
    );
    if site.manual_db_nums.len() == 1 {
        let dbnum = site.manual_db_nums[0];
        url.push_str("&show_dbnum=");
        url.push_str(&dbnum.to_string());
    }
    url
}

fn is_legacy_viewer_url(url: &str) -> bool {
    url.contains("/viewer/")
        || url.contains("backend=")
        || url.contains("backend%3D")
        || url.contains("data_source=")
}

fn viewer_url_needs_managed_port(site: &ManagedProjectSite, url: &str, viewer_port: u16) -> bool {
    configured_viewer_base_url(site).is_none() && !url.contains(&format!(":{viewer_port}"))
}

fn normalize_viewer_url_for_response(site: &mut ManagedProjectSite) {
    let Some(viewer_url) = site.viewer_url.as_deref() else {
        return;
    };
    let Some(viewer_port) = site.viewer_port else {
        return;
    };
    if is_legacy_viewer_url(viewer_url)
        || viewer_url_needs_managed_port(site, viewer_url, viewer_port)
    {
        site.viewer_url = Some(build_viewer_url(site, viewer_port));
    }
}

/// Viewer 是否使用 Vite dev server（开发服务器 / HMR / 未构建）。
///
/// 默认 false：构建 plant3d-web 生产产物（`npm run build`）并以 `vite preview`
/// 提供静态服务，避免把开发服务器当部署态。设 `AIOS_VIEWER_MODE=dev` 时回退到
/// dev server（仅本地开发场景）。
fn viewer_use_dev_server() -> bool {
    std::env::var("AIOS_VIEWER_MODE")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "dev" | "development"
            )
        })
        .unwrap_or(false)
}

/// 是否强制重建 plant3d-web 生产产物（即使 `dist/index.html` 已存在）。
fn viewer_force_build() -> bool {
    std::env::var("AIOS_VIEWER_FORCE_BUILD")
        .map(|value| matches!(value.trim(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

fn managed_viewer_bind_host() -> String {
    std::env::var("AIOS_VIEWER_BIND_HOST")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "0.0.0.0".to_string())
}

/// 构造调用 npm 的 Command（Windows 走 `cmd /C npm`，其余平台直接 `npm`）。
fn npm_command() -> Command {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg("npm");
        cmd
    }
    #[cfg(not(windows))]
    {
        Command::new("npm")
    }
}

#[cfg(windows)]
#[derive(Debug, Clone)]
struct WindowsNginxConfig {
    bin: PathBuf,
    root: PathBuf,
    main_conf: PathBuf,
    conf_dir: PathBuf,
}

#[cfg(windows)]
fn bundled_nginx_binary() -> Option<PathBuf> {
    let root = packaged_install_root()?;
    let candidate = root.join("bin").join("nginx").join("nginx.exe");
    candidate.exists().then_some(candidate)
}

#[cfg(windows)]
fn windows_nginx_config(site_id: &str) -> Option<WindowsNginxConfig> {
    let bin = std::env::var("AIOS_NGINX_BIN")
        .ok()
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .or_else(bundled_nginx_binary)
        .or_else(|| {
            ["C:\\nginx\\nginx.exe", "D:\\nginx\\nginx.exe"]
                .into_iter()
                .map(PathBuf::from)
                .find(|path| path.exists())
        })?;

    let root = std::env::var("AIOS_NGINX_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            packaged_install_root()
                .map(|root| root.join("runtime").join("nginx"))
                .unwrap_or_else(|| site_runtime_dir(site_id).join("nginx"))
        });

    let conf_root = root.join("conf");
    let conf_dir = std::env::var("AIOS_NGINX_CONF_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| conf_root.join("conf.d"));

    Some(WindowsNginxConfig {
        bin,
        root,
        main_conf: conf_root.join("nginx.conf"),
        conf_dir,
    })
}

fn nginx_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn viewer_static_root_for_nginx(viewer_dir: Option<&Path>) -> PathBuf {
    if let Ok(value) = std::env::var("AIOS_VIEWER_STATIC_ROOT") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    if let Ok(repo) = repo_root() {
        let packaged_root = repo.join("viewer-root");
        if packaged_root.join("index.html").exists() {
            return packaged_root;
        }
    }

    if let Some(root) = packaged_install_root() {
        let packaged_root = root.join("viewer-root");
        if packaged_root.join("index.html").exists() {
            return packaged_root;
        }
    }

    if let Some(viewer_dir) = viewer_dir {
        let dist_root = viewer_dir.join("dist");
        if dist_root.join("index.html").exists() {
            return dist_root;
        }

        return viewer_dir.to_path_buf();
    }

    PathBuf::from("viewer-root")
}

fn viewer_static_root_exists_for_nginx(viewer_dir: Option<&Path>) -> bool {
    viewer_static_root_for_nginx(viewer_dir)
        .join("index.html")
        .exists()
}

fn admin_web_port_for_nginx() -> u16 {
    super::web_listen::get_web_listen()
        .map(|(_, port)| port)
        .or_else(|| {
            std::env::var("WEB_SERVER_PORT")
                .ok()
                .and_then(|value| value.trim().parse::<u16>().ok())
        })
        .unwrap_or(3100)
}

fn viewer_base_host_for_nginx(site: &ManagedProjectSite) -> String {
    let base = configured_viewer_base_url(site).unwrap_or_else(|| {
        super::get_local_ip_via_udp()
            .map(|ip| format!("http://{ip}"))
            .unwrap_or_else(|_| "http://localhost".to_string())
    });
    let without_scheme = base
        .trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    let host_port = without_scheme.split('/').next().unwrap_or_default();
    let host = host_port.split(':').next().unwrap_or_default().trim();
    if host.is_empty() {
        "_".to_string()
    } else {
        host.to_string()
    }
}

fn viewer_base_listen_port(site: &ManagedProjectSite) -> u16 {
    let Some(base) = configured_viewer_base_url(site) else {
        return 80;
    };
    let is_https = base.trim().starts_with("https://");
    let without_scheme = base
        .trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    let host_port = without_scheme.split('/').next().unwrap_or_default();
    host_port
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse::<u16>().ok())
        .unwrap_or(if is_https { 443 } else { 80 })
}

async fn choose_static_nginx_viewer_port(site: &ManagedProjectSite) -> Result<u16> {
    if configured_viewer_base_url(site).is_some() {
        return Ok(viewer_base_listen_port(site));
    }

    if site.viewer_port == Some(80) {
        let mut site_without_legacy_port = site.clone();
        site_without_legacy_port.viewer_port = None;
        return choose_viewer_port(&site_without_legacy_port)
            .await
            .map(|(port, _)| port);
    }

    choose_viewer_port(site).await.map(|(port, _)| port)
}

fn render_plant3d_web_nginx_conf(
    site: &ManagedProjectSite,
    static_root: &Path,
    listen_port: u16,
    admin_port: u16,
) -> String {
    let dist_dir = nginx_path(static_root);
    let server_name = viewer_base_host_for_nginx(site);
    let web_port = site.web_port;
    format!(
        r#"server {{
    listen {listen_port};
    server_name {server_name};

    root "{dist_dir}";
    index index.html;

    location / {{
        try_files $uri $uri/ /index.html;
        add_header Cache-Control "no-store, must-revalidate" always;
    }}

    location /assets/ {{
        try_files $uri =404;
        add_header Cache-Control "public, max-age=31536000, immutable" always;
    }}

    location /duckdb/ {{
        try_files $uri =404;
        add_header Cache-Control "no-store, must-revalidate" always;
    }}

    location /api/ {{
        proxy_pass http://127.0.0.1:{web_port}/api/;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_read_timeout 120s;
        proxy_buffering off;
    }}

    location = /api/admin {{
        proxy_pass http://127.0.0.1:{admin_port}/api/admin;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_read_timeout 120s;
        proxy_buffering off;
    }}

    location /api/admin/ {{
        proxy_pass http://127.0.0.1:{admin_port}/api/admin/;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_read_timeout 120s;
        proxy_buffering off;
    }}

    location /files/ {{
        proxy_pass http://127.0.0.1:{web_port}/files/;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_read_timeout 300s;
    }}

    location /ws/ {{
        proxy_pass http://127.0.0.1:{web_port}/ws/;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_read_timeout 3600s;
    }}

    location = /admin {{
        return 302 /admin/;
    }}

    location /admin/ {{
        proxy_pass http://127.0.0.1:{admin_port}/admin/;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_read_timeout 120s;
    }}
}}
"#
    )
}

#[cfg(windows)]
fn render_windows_nginx_main_conf() -> String {
    r#"worker_processes  1;
error_log  logs/error.log warn;
pid        logs/nginx.pid;

events {
    worker_connections  1024;
}

http {
    types {
        text/html html htm;
        text/css css;
        application/javascript js mjs;
        application/json json;
        image/png png;
        image/jpeg jpg jpeg;
        image/gif gif;
        image/svg+xml svg;
        image/x-icon ico;
        font/woff woff;
        font/woff2 woff2;
        application/wasm wasm;
        application/octet-stream parquet glb bin;
    }
    default_type application/octet-stream;
    sendfile on;
    keepalive_timeout 65;
    include conf.d/*.conf;
}
"#
    .to_string()
}

#[cfg(windows)]
async fn configure_windows_nginx_if_available(
    site: &ManagedProjectSite,
    viewer_dir: Option<&Path>,
    listen_port_override: Option<u16>,
) -> Result<bool> {
    let Some(config) = windows_nginx_config(&site.site_id) else {
        if managed_nginx_required() {
            bail!("RequireNginx 已启用，但未检测到 Windows Nginx");
        }
        append_log_line(
            &viewer_log_path(&site.site_id),
            "ℹ️ 未检测到 Windows Nginx（AIOS_NGINX_BIN/AIOS_NGINX_ROOT 未配置且常见路径不存在），使用受管 vite preview fallback",
        );
        return Ok(false);
    };

    let fallback_or_fail = |message: String| -> Result<bool> {
        append_log_line(
            &viewer_log_path(&site.site_id),
            &format!("⚠️ {message}；继续使用受管 vite preview fallback"),
        );
        if managed_nginx_required() {
            bail!(message);
        }
        Ok(false)
    };

    if let Err(err) = fs::create_dir_all(config.root.join("logs")) {
        return fallback_or_fail(format!("创建 Windows Nginx logs 目录失败: {err}"));
    }
    if let Err(err) = fs::create_dir_all(config.root.join("temp")) {
        return fallback_or_fail(format!("创建 Windows Nginx temp 目录失败: {err}"));
    }
    if let Err(err) = write_file_atomic(&config.main_conf, &render_windows_nginx_main_conf()) {
        return fallback_or_fail(format!(
            "写入 Windows Nginx 主配置失败 ({}): {err}",
            config.main_conf.display()
        ));
    }

    let conf_path = config
        .conf_dir
        .join(format!("plant3d-web-{}.conf", site.site_id));
    let static_root = viewer_static_root_for_nginx(viewer_dir);
    let conf = render_plant3d_web_nginx_conf(
        site,
        &static_root,
        listen_port_override.unwrap_or_else(|| viewer_base_listen_port(site)),
        admin_web_port_for_nginx(),
    );
    if let Err(err) = write_file_atomic(&conf_path, &conf) {
        return fallback_or_fail(format!(
            "写入 Windows Nginx 站点配置失败 ({}): {err}",
            conf_path.display()
        ));
    }
    append_log_line(
        &viewer_log_path(&site.site_id),
        &format!("🧩 已生成 Windows Nginx 配置: {}", conf_path.display()),
    );

    let validate = match Command::new(&config.bin)
        .arg("-p")
        .arg(&config.root)
        .arg("-t")
        .output()
        .await
    {
        Ok(output) => output,
        Err(err) => {
            return fallback_or_fail(format!(
                "执行 Windows Nginx 配置校验失败 ({}): {err}",
                config.bin.display()
            ));
        }
    };
    if !validate.status.success() {
        return fallback_or_fail(format!(
            "Windows Nginx 配置校验失败: {}",
            String::from_utf8_lossy(&validate.stderr).trim()
        ));
    }

    let reload = match Command::new(&config.bin)
        .arg("-p")
        .arg(&config.root)
        .arg("-s")
        .arg("reload")
        .output()
        .await
    {
        Ok(output) => output,
        Err(err) => {
            return fallback_or_fail(format!(
                "执行 Windows Nginx reload 失败 ({}): {err}",
                config.bin.display()
            ));
        }
    };

    if reload.status.success() {
        append_log_line(
            &viewer_log_path(&site.site_id),
            "✅ Windows Nginx 配置校验通过并已 reload",
        );
        return Ok(true);
    }

    let (stdout, stderr) = open_log_file(&viewer_log_path(&site.site_id))?;
    let mut start = Command::new(&config.bin);
    start
        .arg("-p")
        .arg(&config.root)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    isolate_process_group(&mut start);
    let child = match start.spawn() {
        Ok(child) => child,
        Err(err) => {
            return fallback_or_fail(format!(
                "启动 Windows Nginx 失败 ({}): {err}",
                config.bin.display()
            ));
        }
    };
    append_log_line(
        &viewer_log_path(&site.site_id),
        &format!(
            "✅ Windows Nginx 未运行，已启动 nginx.exe (pid={})",
            child.id().unwrap_or_default()
        ),
    );
    Ok(true)
}

#[cfg(not(windows))]
async fn configure_windows_nginx_if_available(
    _site: &ManagedProjectSite,
    _viewer_dir: Option<&Path>,
    _listen_port_override: Option<u16>,
) -> Result<bool> {
    Ok(false)
}

#[cfg(not(windows))]
#[derive(Debug, Clone)]
struct LinuxNginxConfig {
    bin: PathBuf,
    conf_dir: PathBuf,
    conf_path: PathBuf,
}

#[cfg(not(windows))]
fn linux_nginx_config(site_id: &str) -> LinuxNginxConfig {
    let bin = std::env::var("AIOS_NGINX_BIN")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("nginx"));
    let conf_dir = std::env::var("AIOS_NGINX_CONF_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/etc/nginx/conf.d"));
    let conf_path = conf_dir.join(format!("plant3d-web-{site_id}.conf"));
    LinuxNginxConfig {
        bin,
        conf_dir,
        conf_path,
    }
}

#[cfg(not(windows))]
fn command_output_summary(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[cfg(not(windows))]
fn linux_nginx_manual_hint(config: &LinuxNginxConfig) -> String {
    format!(
        "可手动执行: sudo mkdir -p {conf_dir} && sudo tee {conf_path} >/dev/null && sudo nginx -t && sudo systemctl reload nginx",
        conf_dir = config.conf_dir.display(),
        conf_path = config.conf_path.display()
    )
}

#[cfg(not(windows))]
async fn configure_linux_nginx_if_available(
    site: &ManagedProjectSite,
    viewer_dir: Option<&Path>,
    listen_port_override: Option<u16>,
) -> Result<bool> {
    let config = linux_nginx_config(&site.site_id);
    let log_path = viewer_log_path(&site.site_id);

    let probe = Command::new(&config.bin).arg("-v").output().await;
    if let Err(err) = probe {
        append_log_line(
            &log_path,
            &format!(
                "ℹ️ 未检测到 Linux Nginx（{} -v 失败: {err}），使用受管 vite preview fallback",
                config.bin.display()
            ),
        );
        return Ok(false);
    }

    if let Err(err) = fs::create_dir_all(&config.conf_dir) {
        append_log_line(
            &log_path,
            &format!(
                "⚠️ 无法创建 Nginx 配置目录 {}: {err}。{}",
                config.conf_dir.display(),
                linux_nginx_manual_hint(&config)
            ),
        );
        return Ok(false);
    }

    let static_root = viewer_static_root_for_nginx(viewer_dir);
    let conf = render_plant3d_web_nginx_conf(
        site,
        &static_root,
        listen_port_override.unwrap_or_else(|| viewer_base_listen_port(site)),
        admin_web_port_for_nginx(),
    );
    if let Err(err) = write_file_atomic(&config.conf_path, &conf) {
        append_log_line(
            &log_path,
            &format!(
                "⚠️ 无法写入 Linux Nginx 配置 {}: {err}。{}",
                config.conf_path.display(),
                linux_nginx_manual_hint(&config)
            ),
        );
        return Ok(false);
    }

    append_log_line(
        &log_path,
        &format!("🧩 已生成 Linux Nginx 配置: {}", config.conf_path.display()),
    );

    let validate = Command::new(&config.bin).arg("-t").output().await;
    let Ok(validate) = validate else {
        append_log_line(
            &log_path,
            &format!(
                "⚠️ 执行 Linux Nginx 配置校验失败。{}",
                linux_nginx_manual_hint(&config)
            ),
        );
        return Ok(false);
    };
    if !validate.status.success() {
        append_log_line(
            &log_path,
            &format!(
                "⚠️ Linux Nginx 配置校验失败，已阻止 reload: {}",
                command_output_summary(&validate)
            ),
        );
        return Ok(false);
    }

    let reload = Command::new("systemctl")
        .arg("reload")
        .arg("nginx")
        .output()
        .await;
    if matches!(reload.as_ref(), Ok(output) if output.status.success()) {
        append_log_line(
            &log_path,
            "✅ Linux Nginx 配置校验通过并已 systemctl reload",
        );
        return Ok(true);
    }

    let enable_now = Command::new("systemctl")
        .arg("enable")
        .arg("--now")
        .arg("nginx")
        .output()
        .await;
    if matches!(enable_now.as_ref(), Ok(output) if output.status.success()) {
        append_log_line(
            &log_path,
            "✅ Linux Nginx 配置校验通过并已 systemctl enable --now",
        );
        return Ok(true);
    }

    let reload_fallback = Command::new(&config.bin)
        .arg("-s")
        .arg("reload")
        .output()
        .await;
    if matches!(reload_fallback.as_ref(), Ok(output) if output.status.success()) {
        append_log_line(&log_path, "✅ Linux Nginx 配置校验通过并已 nginx -s reload");
        return Ok(true);
    }

    let reload_err = reload
        .as_ref()
        .ok()
        .map(command_output_summary)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "systemctl reload nginx 不可用或执行失败".to_string());
    let enable_err = enable_now
        .as_ref()
        .ok()
        .map(command_output_summary)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "systemctl enable --now nginx 不可用或执行失败".to_string());
    let fallback_err = reload_fallback
        .as_ref()
        .ok()
        .map(command_output_summary)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "nginx -s reload 不可用或执行失败".to_string());
    append_log_line(
        &log_path,
        &format!(
            "⚠️ Linux Nginx 配置校验通过，但自动 reload/start 失败；继续使用受管 vite preview fallback。reload={reload_err}; enable={enable_err}; fallback={fallback_err}。{}",
            linux_nginx_manual_hint(&config)
        ),
    );
    Ok(false)
}

#[cfg(windows)]
async fn configure_linux_nginx_if_available(
    _site: &ManagedProjectSite,
    _viewer_dir: Option<&Path>,
    _listen_port_override: Option<u16>,
) -> Result<bool> {
    Ok(false)
}

async fn spawn_viewer_process(site: &ManagedProjectSite) -> Result<Option<ViewerLaunch>> {
    if !managed_viewer_enabled() {
        tracing::info!(site = %site.site_id, "受管 plant3d-web Viewer 启动已禁用");
        return Ok(None);
    }
    let Some(viewer_dir) = viewer_project_dir()? else {
        if viewer_static_root_exists_for_nginx(None) {
            let listen_port = choose_static_nginx_viewer_port(site).await?;
            let windows_nginx_configured =
                configure_windows_nginx_if_available(site, None, Some(listen_port)).await?;
            let linux_nginx_configured =
                configure_linux_nginx_if_available(site, None, Some(listen_port)).await?;
            if windows_nginx_configured || linux_nginx_configured {
                let url = build_viewer_url(site, listen_port);
                tracing::info!(
                    site = %site.site_id,
                    port = listen_port,
                    "使用 release 包静态 viewer-root 配置 Nginx Viewer"
                );
                return Ok(Some(ViewerLaunch {
                    port: listen_port,
                    pid: None,
                    url,
                }));
            }
        }
        tracing::warn!(
            site = %site.site_id,
            "未找到 plant3d-web 目录，且未成功配置静态 viewer-root Nginx，跳过受管 Viewer 启动（可设置 AIOS_VIEWER_PROJECT_DIR 或 AIOS_VIEWER_STATIC_ROOT）"
        );
        return Ok(None);
    };

    let base_path = "/".to_string();
    let (port, reuse_existing) = choose_viewer_port(site).await?;
    if reuse_existing {
        let dist_base_path = detect_viewer_base_path(&viewer_dir);
        if dist_base_path == base_path {
            configure_windows_nginx_if_available(site, Some(&viewer_dir), None).await?;
            configure_linux_nginx_if_available(site, Some(&viewer_dir), None).await?;
            let url = build_viewer_url(site, port);
            tracing::info!(site = %site.site_id, port, "复用已运行的 plant3d-web Viewer");
            return Ok(Some(ViewerLaunch {
                port,
                pid: None,
                url,
            }));
        }

        append_log_line(
            &viewer_log_path(&site.site_id),
            &format!(
                "♻️ 已运行 Viewer 的构建 base 为 {dist_base_path:?}，需重建为 {base_path:?}；停止旧 Viewer 后重新启动"
            ),
        );
        for pid in process_ids_on_port(port).await.unwrap_or_default() {
            let _ = kill_pid(pid).await;
        }
    }

    // 生产模式（默认）：先构建 plant3d-web 生产产物，再以 `vite preview` 提供静态服务，
    // 不再用 `vite dev`（开发服务器 / HMR）充当部署态。设 AIOS_VIEWER_MODE=dev 可回退。
    let use_dev_server = viewer_use_dev_server();
    if !use_dev_server {
        let dist_index = viewer_dir.join("dist").join("index.html");
        let dist_base_path = detect_viewer_base_path(&viewer_dir);
        if !dist_index.exists() || viewer_force_build() || dist_base_path != base_path {
            append_log_line(
                &viewer_log_path(&site.site_id),
                "🏗️ 构建 plant3d-web 生产产物 (npm run build)...",
            );
            let (build_out, build_err) = open_log_file(&viewer_log_path(&site.site_id))?;
            let mut build_command = npm_command();
            build_command
                .arg("run")
                .arg("build")
                .current_dir(&viewer_dir)
                .env("VITE_BASE_PATH", &base_path)
                .env("VITE_BACKEND_PORT", site.web_port.to_string())
                .env("VITE_BACKEND_URL", site_access_base_url(site))
                .env("VITE_API_BASE_URL", site_access_base_url(site))
                .stdout(Stdio::from(build_out))
                .stderr(Stdio::from(build_err));
            isolate_process_group(&mut build_command);
            let build_status = build_command
                .status()
                .await
                .context("启动 plant3d-web 构建失败 (npm run build)")?;
            if !build_status.success() {
                bail!(
                    "plant3d-web 构建失败 (npm run build 退出码 {:?})，请检查 viewer 日志",
                    build_status.code()
                );
            }
            append_log_line(
                &viewer_log_path(&site.site_id),
                "✅ plant3d-web 构建完成，启动 vite preview 静态服务",
            );
        }
    }

    configure_windows_nginx_if_available(site, Some(&viewer_dir), None).await?;
    configure_linux_nginx_if_available(site, Some(&viewer_dir), None).await?;

    let url = build_viewer_url(site, port);
    let viewer_bind_host = managed_viewer_bind_host();

    let run_script = if use_dev_server { "dev" } else { "preview" };
    let (stdout, stderr) = open_log_file(&viewer_log_path(&site.site_id))?;
    let mut command = npm_command();
    command
        .arg("run")
        .arg(run_script)
        .arg("--")
        .arg("--host")
        .arg(&viewer_bind_host)
        .arg("--port")
        .arg(port.to_string())
        .arg("--strictPort")
        .current_dir(&viewer_dir)
        .env("BROWSER", "none")
        .env("VITE_BASE_PATH", &base_path)
        .env("VITE_BACKEND_PORT", site.web_port.to_string())
        .env("VITE_BACKEND_URL", site_access_base_url(site))
        .env("VITE_API_BASE_URL", site_access_base_url(site))
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    isolate_process_group(&mut command);

    let child = command.spawn().with_context(|| {
        format!(
            "启动 plant3d-web Viewer 失败 (dir={}, port={}, script={})",
            viewer_dir.display(),
            port,
            run_script
        )
    })?;
    let pid = child.id().unwrap_or_default();
    register_process(&site.site_id, PROC_ROLE_VIEWER, pid);
    if !wait_for_http_ok(
        &format!("http://127.0.0.1:{port}{base_path}"),
        WAIT_HTTP_ATTEMPTS,
        WAIT_STEP_MS,
    )
    .await
    {
        if pid != 0 {
            let _ = kill_pid_guarded(&site.site_id, PROC_ROLE_VIEWER, pid).await;
        }
        bail!("plant3d-web Viewer 未在端口 {} 启动成功", port);
    }

    Ok(Some(ViewerLaunch {
        port,
        pid: (pid != 0).then_some(pid),
        url,
    }))
}

/// file 与 ws 模式互斥的核心停止动作：停止本站点 ws 模式 DB（独立 surreal server）。
///
/// file（离线/嵌入式）与 ws 模式共享同一 RocksDB 数据目录（`db_data_path`），
/// RocksDB 仅允许单进程持有排他锁，因此两种模式无法同时打开同一数据目录。ws server
/// 绑定本站点专属的 `db_port`（建站时按站点唯一预留），且持有该 RocksDB 的排他锁；
/// 端口上的监听进程必属于本站点 DB，可安全清理。
///
/// 返回是否实际执行了停止动作（用于调用方判断是否需要等待端口释放 / 记录日志）。
async fn stop_site_ws_db_for_exclusivity(site: &ManagedProjectSite) -> bool {
    let mut acted = false;
    // 1) 守卫式停止登记在册的 db 进程（pid + 启动时刻双校验，防 PID 复用误杀）。
    if let Some(pid) = site.db_pid {
        if matches!(
            kill_pid_guarded(&site.site_id, PROC_ROLE_DB, pid).await,
            Ok(true)
        ) {
            acted = true;
        }
    }
    // 2) 兜底：清理仍监听本站点 db 端口的任何残留/未登记 surreal server。
    if port_in_use("127.0.0.1", site.db_port) {
        let pids = process_ids_on_port(site.db_port).await.unwrap_or_default();
        for pid in pids {
            if kill_pid(pid).await.is_ok() {
                acted = true;
            }
        }
    }
    if acted {
        let _ = update_runtime(
            &site.site_id,
            RuntimeUpdate {
                db_pid: Some(None),
                ..Default::default()
            },
        );
        // ws server 已停：注销其 data dir 持有者登记。
        unregister_db_dir_owner(&site.db_data_path);
    }
    acted
}

async fn ensure_site_db_started(
    site: &ManagedProjectSite,
    status: ManagedSiteStatus,
    mode: ManagedSiteDbMode,
) -> Result<Option<u32>> {
    // 统一以 db_data_path 的 RocksDB 锁为互斥真源（旧端口式决策已移除）。
    // 持有每目录锁守卫，覆盖 spawn + register，整体原子。
    let (acquire, _guard) = acquire_data_dir(site, mode, DB_DIR_ROLE_SERVING).await?;

    if mode == ManagedSiteDbMode::File {
        // file：无独立 server；acquire 已优雅停掉占用同一数据目录的 ws server 并确认锁可用。
        if matches!(acquire, DataDirAcquire::Proceed) {
            append_log_line(
                &db_log_path(&site.site_id),
                "🔌 file 离线模式启动：data dir 互斥就绪，RocksDB 排他锁可用",
            );
        }
        update_runtime(
            &site.site_id,
            RuntimeUpdate {
                status: Some(status),
                db_pid: Some(None),
                last_error: Some(None),
                ..Default::default()
            },
        )?;
        return Ok(None);
    }

    // ws：健康同模式持有者直接复用，否则拉起全新 server。
    if matches!(acquire, DataDirAcquire::ReuseExisting) {
        return Ok(None);
    }
    let db_pid = spawn_db_process(site).await?; // 内部已 register_db_dir_owner(serving)
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
        unregister_db_dir_owner(&site.db_data_path);
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

/// 站点级 db_index.sqlite 路径（runtime/admin_sites/<site_id>/db_index.sqlite）。
#[cfg(feature = "sqlite-index")]
fn site_db_index_path(site_id: &str) -> PathBuf {
    site_runtime_dir(site_id).join(crate::data_interface::db_index::DB_INDEX_FILE_NAME)
}

/// 站点预扫描的 (project_name, root_path) 列表（多工程用 projects[]，否则回退派生根）。
#[cfg(feature = "sqlite-index")]
fn site_prescan_roots(site: &ManagedProjectSite) -> Vec<(String, PathBuf)> {
    if !site.projects.is_empty() {
        let mut ordered: Vec<&SiteProject> = site.projects.iter().collect();
        ordered.sort_by_key(|p| p.sort_order);
        let roots: Vec<(String, PathBuf)> = ordered
            .into_iter()
            .map(|p| (p.name.clone(), PathBuf::from(&p.path)))
            .filter(|(_, path)| path.exists())
            .collect();
        if !roots.is_empty() {
            return roots;
        }
    }
    site_existing_project_roots(site)
        .unwrap_or_default()
        .into_iter()
        .map(|root| {
            let name = root
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| site.project_name.clone());
            (name, root)
        })
        .collect()
}

/// db_index 预扫描结果摘要（用于 admin/CLI 反馈）。
#[cfg(feature = "sqlite-index")]
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct DbIndexRebuildSummary {
    pub scanned: usize,
    pub skipped: usize,
    pub db_files: usize,
    pub ref0_total: usize,
    pub dependency_edges: usize,
    pub errors: usize,
}

/// 预扫描核心（两阶段），返回摘要。
/// - Phase 1（index-only）：遍历全部 db 文件，构建全局 `ref0 -> dbnum` 归属表。
/// - Phase 2（Stage 4）：为设计库抽取外向引用 → 经全局表反查 → 记录精确依赖边。
#[cfg(feature = "sqlite-index")]
async fn db_index_prescan_core(
    site: &ManagedProjectSite,
    force: bool,
) -> Result<DbIndexRebuildSummary> {
    let roots = site_prescan_roots(site);
    if roots.is_empty() {
        bail!("db_index 预扫描根目录为空");
    }
    let index_path = site_db_index_path(&site.site_id);
    let sidecar_roots = roots
        .into_iter()
        .map(
            |(name, path)| crate::web_server::parse_sidecar_client::DbIndexRoot {
                name,
                path: path.to_string_lossy().to_string(),
            },
        )
        .collect::<Vec<_>>();
    match crate::web_server::parse_sidecar_client::rebuild_db_index(
        &site.site_id,
        sidecar_roots,
        index_path.to_string_lossy().to_string(),
        force,
        site.manual_db_nums.clone(),
    )
    .await
    {
        Ok(value) => serde_json::from_value::<DbIndexRebuildSummary>(value)
            .context("解析 sidecar db_index 响应失败"),
        Err(err) => bail!("aios-database sidecar db_index 重建失败: {}", err.message),
    }
}

/// 解析前自动预扫（包装核心 + 日志，失败不致命）。
#[cfg(feature = "sqlite-index")]
async fn run_db_index_prescan(site: &ManagedProjectSite, force: bool) -> DbIndexRebuildSummary {
    let summary = match db_index_prescan_core(site, force).await {
        Ok(summary) => summary,
        Err(err) => {
            tracing::warn!(
                site = %site.site_id,
                "db_index 自动预扫描失败，按非致命错误继续: {err}"
            );
            let mut summary = DbIndexRebuildSummary::default();
            summary.errors += 1;
            summary
        }
    };
    tracing::info!(
        site = %site.site_id,
        scanned = summary.scanned,
        skipped = summary.skipped,
        db_files = summary.db_files,
        ref0_total = summary.ref0_total,
        edges = summary.dependency_edges,
        errors = summary.errors,
        "db_index 预扫描完成"
    );
    summary
}

#[cfg(feature = "sqlite-index")]
fn should_run_db_index_prescan(site: &ManagedProjectSite) -> bool {
    // `auto_parse_related_dbnums` 的语义是优先使用 db_index 精确依赖闭包。
    // quick deploy / 单库解析也必须预扫；否则首次建站时没有 db_index.sqlite，
    // 解析配置会在精确依赖为空时失去可观测证据。
    site.auto_parse_related_dbnums
}

/// 手动重建站点 db_index（admin『重建索引』/ CLI 强制全量重扫）。
#[cfg(feature = "sqlite-index")]
pub async fn rebuild_site_db_index(site_id: String, force: bool) -> Result<DbIndexRebuildSummary> {
    let site = task::spawn_blocking({
        let site_id = site_id.clone();
        move || get_site(&site_id)
    })
    .await
    .context("读取站点状态失败 (join error)")??
    .ok_or_else(|| anyhow!("站点不存在"))?;
    db_index_prescan_core(&site, force).await
}

async fn run_parse_pipeline(site_id: String) -> Result<()> {
    let mut site = task::spawn_blocking({
        let site_id = site_id.clone();
        move || get_site(&site_id)
    })
    .await
    .context("读取站点状态失败 (join error)")??
    .ok_or_else(|| anyhow!("站点不存在"))?;

    let parse_started_at = now_rfc3339();
    update_runtime(
        &site.site_id,
        RuntimeUpdate {
            status: Some(ManagedSiteStatus::Draft),
            parse_status: Some(ManagedSiteParseStatus::Running),
            parse_pid: Some(None),
            last_error: Some(None),
            last_parse_started_at: Some(Some(parse_started_at)),
            last_parse_finished_at: Some(None),
            last_parse_duration_ms: Some(None),
            ..Default::default()
        },
    )?;

    // 解析前自动预扫，刷新站点级 db_index.sqlite（全局 ref0->dbnum + 依赖边）。
    //
    // 首次 quick-deploy 创建站点时还没有 db_index.sqlite，`write_site_files` 会回退到
    // 粗粒度 CATA 纳入。预扫完成后必须重写一次配置，才能让本次解析立即使用精确依赖闭包，
    // 避免“一键部署 smoke”被放大成全 CATA 解析。
    #[cfg(feature = "sqlite-index")]
    {
        if should_run_db_index_prescan(&site) {
            append_log_line(
                &parse_log_path(&site.site_id),
                "🧭 db_index 预扫描中：解析进程启动前正在刷新依赖索引...",
            );
            let summary = run_db_index_prescan(&site, false).await;
            append_log_line(
                &parse_log_path(&site.site_id),
                &format!(
                    "✅ db_index 预扫描完成：scanned={} skipped={} db_files={} ref0_total={} edges={} errors={}",
                    summary.scanned,
                    summary.skipped,
                    summary.db_files,
                    summary.ref0_total,
                    summary.dependency_edges,
                    summary.errors
                ),
            );
            let (fresh_site, db_user, db_password) = task::spawn_blocking({
                let site_id = site_id.clone();
                move || load_site_and_credentials(&site_id)
            })
            .await
            .context("读取站点凭据失败 (join error)")??;
            task::spawn_blocking({
                let site = fresh_site.clone();
                move || write_site_files(&site, &db_user, &db_password)
            })
            .await
            .context("刷新站点解析配置失败 (join error)")??;
            site = fresh_site;
        } else {
            append_log_line(
                &parse_log_path(&site.site_id),
                "⏭️ 跳过 db_index 预扫描：未启用自动关联依赖；系统库补齐由解析计划直接处理。",
            );
        }
    }

    let started_db_pid =
        ensure_site_db_started(&site, site.status.clone(), site.pipeline_db_mode).await?;
    let parse_result = spawn_parse_process(site_id.clone()).await;

    if let Some(db_pid) = started_db_pid {
        let _ = kill_pid_guarded(&site_id, PROC_ROLE_DB, db_pid).await;
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
    let mut site = task::spawn_blocking({
        let site_id = site_id.clone();
        move || get_site(&site_id)
    })
    .await
    .context("读取站点状态失败 (join error)")??
    .ok_or_else(|| anyhow!("站点不存在"))?;

    if parse_first && site.parse_status != ManagedSiteParseStatus::Parsed {
        let parse_started_at = now_rfc3339();
        update_runtime(
            &site.site_id,
            RuntimeUpdate {
                status: Some(ManagedSiteStatus::Draft),
                parse_status: Some(ManagedSiteParseStatus::Running),
                parse_pid: Some(None),
                last_error: Some(None),
                last_parse_started_at: Some(Some(parse_started_at)),
                last_parse_finished_at: Some(None),
                last_parse_duration_ms: Some(None),
                ..Default::default()
            },
        )?;
    }

    // 完整部署/生成会在 `parse_first=true` 时直接调用 `spawn_parse_process`，
    // 不经过 `run_parse_pipeline`。这里同样要在首次解析前刷新 db_index 并重写解析配置，
    // 否则 quick-deploy + gen_model 会继续使用建站时的粗粒度 CATA 列表。
    #[cfg(feature = "sqlite-index")]
    if parse_first && site.parse_status != ManagedSiteParseStatus::Parsed {
        if should_run_db_index_prescan(&site) {
            append_log_line(
                &parse_log_path(&site.site_id),
                "🧭 db_index 预扫描中：模型生成前正在刷新依赖索引...",
            );
            let summary = run_db_index_prescan(&site, false).await;
            append_log_line(
                &parse_log_path(&site.site_id),
                &format!(
                    "✅ db_index 预扫描完成：scanned={} skipped={} db_files={} ref0_total={} edges={} errors={}",
                    summary.scanned,
                    summary.skipped,
                    summary.db_files,
                    summary.ref0_total,
                    summary.dependency_edges,
                    summary.errors
                ),
            );
            let (fresh_site, db_user, db_password) = task::spawn_blocking({
                let site_id = site_id.clone();
                move || load_site_and_credentials(&site_id)
            })
            .await
            .context("读取站点凭据失败 (join error)")??;
            task::spawn_blocking({
                let site = fresh_site.clone();
                move || write_site_files(&site, &db_user, &db_password)
            })
            .await
            .context("刷新站点解析配置失败 (join error)")??;
            site = fresh_site;
        } else {
            append_log_line(
                &parse_log_path(&site.site_id),
                "⏭️ 跳过 db_index 预扫描：未启用自动关联依赖；系统库补齐由解析计划直接处理。",
            );
        }
    }

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
                let parse_db_pid = ensure_site_db_started(
                    &site,
                    ManagedSiteStatus::Starting,
                    site.pipeline_db_mode,
                )
                .await?;
                let parse_result = spawn_parse_process(site_id.clone()).await;
                cleanup_started_db(&site_id, parse_db_pid).await;
                parse_result?;
            } else {
                bail!("站点尚未解析，请先执行解析或选择完整生成");
            }
        }

        let generation_db_pid =
            ensure_site_db_started(&site, ManagedSiteStatus::Starting, site.pipeline_db_mode)
                .await?;
        let generation_result = spawn_generation_process(site_id.clone()).await;
        cleanup_started_db(&site_id, generation_db_pid).await;
        generation_result
    }
    .await;

    result
}

async fn run_generation_then_start_pipeline(site_id: String, parse_first: bool) -> Result<()> {
    run_generation_pipeline(site_id.clone(), parse_first).await?;
    run_start_pipeline(site_id).await
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
    let _ = wait_for_business_status_ok(&site).await;
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
    let mut site = task::spawn_blocking({
        let site_id = site_id.clone();
        move || get_site(&site_id)
    })
    .await
    .context("读取站点状态失败 (join error)")??
    .ok_or_else(|| anyhow!("站点不存在"))?;
    if let Some(updated_site) = task::spawn_blocking({
        let site_id = site_id.clone();
        move || reassign_db_port_if_occupied(&site_id)
    })
    .await
    .context("自动调整 DB 端口失败 (join error)")??
    {
        site = updated_site;
    }

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
    if site.parse_status != ManagedSiteParseStatus::Parsed {
        let parse_db_pid =
            ensure_site_db_started(&site, ManagedSiteStatus::Starting, site.pipeline_db_mode)
                .await?;
        let parse_result = spawn_parse_process(site_id.clone()).await;
        cleanup_started_db(&site_id, parse_db_pid).await;
        if let Err(err) = parse_result {
            let stopped_by_user = site_was_stopped_by_user(&site_id);
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
    let db_pid =
        ensure_site_db_started(&site, ManagedSiteStatus::Starting, site.runtime_db_mode).await?;

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
            entry_url: Some(Some(site_access_base_url(&site))),
            ..Default::default()
        },
    )?;
    let status_url = format!("{}/api/status", site_probe_base_url(&site));
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
            entry_url: Some(Some(site_access_base_url(&site))),
            ..Default::default()
        },
    )?;
    if mark_running {
        spawn_deploy_validation_refresh(site_id.clone(), "start");
    }
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
    if site.status == ManagedSiteStatus::Running {
        update_runtime(
            &site_id,
            RuntimeUpdate {
                last_error: Some(None),
                ..Default::default()
            },
        )?;
        spawn_deploy_validation_refresh(site_id.clone(), "start_already_running");
        return Ok(());
    }
    if matches!(
        site.status,
        ManagedSiteStatus::Starting | ManagedSiteStatus::Stopping
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
        if let Err(err) = run_generation_then_start_pipeline(site_id.clone(), parse_first).await {
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

/// 重新部署前的清理：若运行中先停站（释放 DB 锁/子进程）→ 删除旧数据目录
/// （`<runtime>/data/`，即 SurrealDB 数据，保留 DbOption 配置与站点注册行）→
/// 重置状态为 Draft/Pending。之后由调用方提交 `DeployManagedSite` 任务重新走
/// 「解析 → 生成 → 启动」全流程。
pub async fn redeploy_reset_site(site_id: &str) -> Result<()> {
    let site = task::spawn_blocking({
        let site_id = site_id.to_string();
        move || get_site(&site_id)
    })
    .await
    .context("读取站点状态失败 (join error)")??
    .ok_or_else(|| anyhow!("站点不存在"))?;

    // 1) 运行中/解析中则先停站，给子进程释放文件句柄留出时间。
    let needs_stop = site_has_active_processes(&site)
        || matches!(
            site.status,
            ManagedSiteStatus::Running | ManagedSiteStatus::Starting | ManagedSiteStatus::Stopping
        )
        || site.parse_status == ManagedSiteParseStatus::Running;
    if needs_stop {
        let _ = stop_site(site_id).await;
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    }

    // 2) 删除旧数据目录（保留 DbOption.toml / 注册行 / 日志）。
    let data_dir = site_runtime_dir(site_id).join("data");
    task::spawn_blocking(move || -> Result<()> {
        if data_dir.exists() {
            fs::remove_dir_all(&data_dir)
                .with_context(|| format!("删除旧数据目录失败: {}", data_dir.display()))?;
        }
        Ok(())
    })
    .await
    .context("删除旧数据失败 (join error)")??;

    // 3) 重置状态为草稿/待解析，清理 pid、错误与解析时间戳。
    update_runtime(
        site_id,
        RuntimeUpdate {
            status: Some(ManagedSiteStatus::Draft),
            parse_status: Some(ManagedSiteParseStatus::Pending),
            db_pid: Some(None),
            web_pid: Some(None),
            viewer_pid: Some(None),
            parse_pid: Some(None),
            last_error: Some(None),
            last_parse_started_at: Some(None),
            last_parse_finished_at: Some(None),
            last_parse_duration_ms: Some(None),
            ..Default::default()
        },
    )?;
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

fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn remote_target_os_to_str(os: ManagedRemoteTargetOs) -> &'static str {
    match os {
        ManagedRemoteTargetOs::Ubuntu22 => "ubuntu22",
        ManagedRemoteTargetOs::Centos79 => "centos79",
        ManagedRemoteTargetOs::Windows => "windows",
    }
}

fn remote_target_os_from_str(value: &str) -> ManagedRemoteTargetOs {
    match value.trim().to_ascii_lowercase().as_str() {
        "centos79" | "centos7.9" | "centos" => ManagedRemoteTargetOs::Centos79,
        "windows" | "win" | "win64" => ManagedRemoteTargetOs::Windows,
        _ => ManagedRemoteTargetOs::Ubuntu22,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteExecutionMode {
    Root,
    Sudo,
    User,
}

impl RemoteExecutionMode {
    fn status_label(self) -> &'static str {
        match self {
            RemoteExecutionMode::Root => "root",
            RemoteExecutionMode::Sudo => "sudo",
            RemoteExecutionMode::User => "user",
        }
    }

    fn degraded(self) -> bool {
        matches!(self, RemoteExecutionMode::User)
    }

    fn privileged_shell(self, script: &str) -> String {
        match self {
            RemoteExecutionMode::Root | RemoteExecutionMode::User => script.to_string(),
            RemoteExecutionMode::Sudo => format!("sudo -n sh -c {}", sh_quote(script)),
        }
    }
}

fn remote_execution_mode_from_output(output: &str) -> RemoteExecutionMode {
    match output.trim().lines().last().unwrap_or_default().trim() {
        "root" => RemoteExecutionMode::Root,
        "sudo" => RemoteExecutionMode::Sudo,
        _ => RemoteExecutionMode::User,
    }
}

async fn detect_remote_execution_mode(target: &ManagedRemoteTarget) -> Result<RemoteExecutionMode> {
    let output = run_ssh(
        target,
        "set -e; if [ \"$(id -u)\" = \"0\" ]; then echo root; elif command -v sudo >/dev/null 2>&1 && sudo -n true >/dev/null 2>&1; then echo sudo; else echo user; fi",
    )
    .await?;
    Ok(remote_execution_mode_from_output(&output))
}

fn remote_password_cache_key(site_id: &str, target_id: &str) -> String {
    format!("{site_id}:{target_id}")
}

fn remember_remote_password(site_id: &str, target_id: &str, password: &str) {
    let password = password.trim();
    if password.is_empty() {
        return;
    }
    let key = remote_password_cache_key(site_id, target_id);
    let cache = REMOTE_DEPLOY_PASSWORDS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut guard) = cache.lock() {
        guard.insert(key, password.to_string());
    }
}

fn remembered_remote_password(site_id: &str, target_id: &str) -> Option<String> {
    let key = remote_password_cache_key(site_id, target_id);
    REMOTE_DEPLOY_PASSWORDS
        .get()
        .and_then(|cache| cache.lock().ok()?.get(&key).cloned())
}

fn remote_site_dir(target: &ManagedRemoteTarget, site_id: &str) -> String {
    format!("{}/{}", target.remote_root.trim_end_matches('/'), site_id)
}

fn remote_entry_url(target: &ManagedRemoteTarget) -> String {
    target
        .public_base_url
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("http://{}:{}", target.host, target.remote_web_port))
}

fn remote_site_token(site_id: &str, deploy_id: Option<&str>) -> String {
    format!("{site_id}:{}", deploy_id.unwrap_or("unknown"))
}

fn ssh_target(target: &ManagedRemoteTarget) -> String {
    format!("{}@{}", target.ssh_user, target.host)
}

fn command_env_password(target: &ManagedRemoteTarget) -> Result<String> {
    if let Some(password) = target
        .ssh_password
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(password.to_string());
    }
    std::env::var(&target.password_env)
        .with_context(|| format!("远端部署密码环境变量未设置: {}", target.password_env))
}

fn connect_native_ssh(target: &ManagedRemoteTarget) -> Result<ssh2::Session> {
    let password = command_env_password(target)?;
    let tcp = TcpStream::connect((target.host.as_str(), target.ssh_port))
        .with_context(|| format!("连接远端 SSH 失败: {}:{}", target.host, target.ssh_port))?;
    let timeout = Some(Duration::from_secs(30));
    let _ = tcp.set_read_timeout(timeout);
    let _ = tcp.set_write_timeout(timeout);

    let mut session = ssh2::Session::new().context("创建原生 SSH session 失败")?;
    session.set_tcp_stream(tcp);
    session.handshake().context("SSH 握手失败")?;
    session
        .userauth_password(&target.ssh_user, &password)
        .with_context(|| format!("SSH 密码认证失败: {}", ssh_target(target)))?;
    if !session.authenticated() {
        bail!("SSH 认证未通过: {}", ssh_target(target));
    }
    Ok(session)
}

fn exec_native_ssh(session: &ssh2::Session, remote_cmd: &str) -> Result<String> {
    let mut channel = session.channel_session().context("创建 SSH channel 失败")?;
    channel
        .exec(remote_cmd)
        .with_context(|| format!("执行远端命令失败: {remote_cmd}"))?;
    let mut stdout = String::new();
    channel
        .read_to_string(&mut stdout)
        .context("读取远端命令 stdout 失败")?;
    let mut stderr = String::new();
    channel
        .stderr()
        .read_to_string(&mut stderr)
        .context("读取远端命令 stderr 失败")?;
    channel.wait_close().context("等待远端命令结束失败")?;
    let exit_status = channel.exit_status().unwrap_or(-1);
    if exit_status == 0 {
        Ok(stdout.trim().to_string())
    } else {
        bail!(
            "SSH 命令失败: status={exit_status}; stderr={}; stdout={}",
            stderr.trim(),
            stdout.trim()
        );
    }
}

async fn run_ssh(target: &ManagedRemoteTarget, remote_cmd: &str) -> Result<String> {
    let target = target.clone();
    let remote_cmd = remote_cmd.to_string();
    task::spawn_blocking(move || {
        let session = connect_native_ssh(&target)?;
        exec_native_ssh(&session, &remote_cmd)
    })
    .await
    .context("执行原生 SSH 任务失败")?
}

fn normalize_remote_path(path: &str) -> String {
    path.trim()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string()
}

fn ensure_safe_remote_delete_dir(remote_path: &str) -> Result<String> {
    let normalized = normalize_remote_path(remote_path);
    if normalized.is_empty()
        || normalized == "/"
        || normalized == "."
        || normalized == "~"
        || normalized.contains('\0')
        || normalized.contains('*')
    {
        bail!("拒绝清理危险远端路径: {remote_path}");
    }
    if !normalized.starts_with('/') {
        bail!("Linux 远端路径必须是绝对路径: {remote_path}");
    }
    if normalized.matches('/').count() < 2 {
        bail!("远端路径层级过浅，拒绝清理: {remote_path}");
    }
    Ok(normalized)
}

fn remote_join(base: &str, rel: &Path) -> String {
    let mut out = normalize_remote_path(base);
    for component in rel.components() {
        let value = component.as_os_str().to_string_lossy();
        if value.is_empty() || value == "." {
            continue;
        }
        out.push('/');
        out.push_str(&value.replace('\\', "/"));
    }
    out
}

fn create_remote_dir_recursive(sftp: &ssh2::Sftp, remote_dir: &str) -> Result<()> {
    let remote_dir = normalize_remote_path(remote_dir);
    if remote_dir.is_empty() || remote_dir == "/" {
        return Ok(());
    }
    let mut current = String::new();
    for part in remote_dir.split('/').filter(|part| !part.is_empty()) {
        current.push('/');
        current.push_str(part);
        let _ = sftp.mkdir(Path::new(&current), 0o755);
    }
    Ok(())
}

fn upload_file_native(sftp: &ssh2::Sftp, local_path: &Path, remote_path: &str) -> Result<()> {
    let parent = remote_parent_dir(remote_path, "/tmp");
    create_remote_dir_recursive(sftp, &parent)?;
    let mut local = fs::File::open(local_path)
        .with_context(|| format!("打开本地上传文件失败: {}", local_path.display()))?;
    let mut remote = sftp
        .create(Path::new(remote_path))
        .with_context(|| format!("创建远端文件失败: {remote_path}"))?;
    std::io::copy(&mut local, &mut remote)
        .with_context(|| format!("上传文件失败: {} -> {remote_path}", local_path.display()))?;
    Ok(())
}

fn upload_path_native_sync(
    local_path: &Path,
    target: &ManagedRemoteTarget,
    remote_path: &str,
    copy_contents: bool,
    delete_extra: bool,
) -> Result<()> {
    let session = connect_native_ssh(target)?;
    if delete_extra {
        let safe_dir = ensure_safe_remote_delete_dir(remote_path)?;
        exec_native_ssh(
            &session,
            &format!(
                "set -e; rm -rf {}; mkdir -p {}",
                sh_quote(&safe_dir),
                sh_quote(&safe_dir)
            ),
        )?;
    } else if local_path.is_dir() {
        exec_native_ssh(
            &session,
            &format!("set -e; mkdir -p {}", sh_quote(remote_path)),
        )?;
    } else {
        let parent = remote_parent_dir(remote_path, "/tmp");
        exec_native_ssh(&session, &format!("set -e; mkdir -p {}", sh_quote(&parent)))?;
    }

    let sftp = session.sftp().context("创建 SFTP session 失败")?;
    if local_path.is_file() {
        upload_file_native(&sftp, local_path, remote_path)?;
        return Ok(());
    }
    if !local_path.is_dir() {
        bail!("本地上传路径不存在: {}", local_path.display());
    }

    let remote_root = normalize_remote_path(remote_path);
    create_remote_dir_recursive(&sftp, &remote_root)?;
    let base = if copy_contents {
        local_path.to_path_buf()
    } else {
        local_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| local_path.to_path_buf())
    };
    for entry in walkdir::WalkDir::new(local_path).follow_links(false) {
        let entry =
            entry.with_context(|| format!("遍历本地上传目录失败: {}", local_path.display()))?;
        let path = entry.path();
        let rel = path
            .strip_prefix(&base)
            .with_context(|| format!("计算上传相对路径失败: {}", path.display()))?;
        if copy_contents && rel.as_os_str().is_empty() {
            continue;
        }
        let remote = remote_join(&remote_root, rel);
        if entry.file_type().is_dir() {
            create_remote_dir_recursive(&sftp, &remote)?;
        } else if entry.file_type().is_file() {
            upload_file_native(&sftp, path, &remote)?;
        }
    }
    Ok(())
}

async fn upload_path_native(
    local_path: &Path,
    target: &ManagedRemoteTarget,
    remote_path: &str,
    copy_contents: bool,
    delete_extra: bool,
) -> Result<()> {
    let local_path = local_path.to_path_buf();
    let target = target.clone();
    let remote_path = remote_path.to_string();
    task::spawn_blocking(move || {
        upload_path_native_sync(
            &local_path,
            &target,
            &remote_path,
            copy_contents,
            delete_extra,
        )
    })
    .await
    .context("执行原生 SFTP 上传任务失败")?
}

async fn upload_db_native(site: &ManagedProjectSite, target: &ManagedRemoteTarget) -> Result<()> {
    let source = Path::new(&site.db_data_path);
    if !source.exists() {
        bail!("本地数据库目录不存在: {}", source.display());
    }
    upload_path_native(
        source,
        target,
        &target.remote_db_path,
        source.is_dir(),
        true,
    )
    .await
}

async fn upload_file_native_async(
    local_path: &Path,
    target: &ManagedRemoteTarget,
    remote_path: &str,
) -> Result<()> {
    upload_path_native(local_path, target, remote_path, false, false).await
}

async fn upload_resource_path_native(
    local_path: &Path,
    target: &ManagedRemoteTarget,
    remote_path: &str,
    copy_contents: bool,
    delete_extra: bool,
) -> Result<()> {
    upload_path_native(local_path, target, remote_path, copy_contents, delete_extra).await
}

fn remote_parent_dir(remote_path: &str, default: &str) -> String {
    let trimmed = remote_path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) => "/".to_string(),
        Some(idx) => trimmed[..idx].to_string(),
        None => default.to_string(),
    }
}

fn path_is_windows_exe(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("exe"))
        .unwrap_or(false)
}

fn find_runtime_artifact(file_name: &str) -> Option<PathBuf> {
    let runtime = repo_root().ok()?.join("runtime");
    let mut candidates = fs::read_dir(runtime)
        .ok()?
        .flatten()
        .map(|entry| entry.path().join(file_name))
        .filter(|path| path.exists() && path.is_file())
        .collect::<Vec<_>>();
    candidates.sort_by_key(|path| {
        path.metadata()
            .and_then(|meta| meta.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    });
    candidates.pop()
}

fn resolve_local_web_bin(target: &ManagedRemoteTarget) -> Result<Option<PathBuf>> {
    if !target.upload_web_server {
        return Ok(None);
    }
    if let Some(path) = target.local_web_bin.as_deref() {
        let path = PathBuf::from(path);
        if path.exists() && path.is_file() {
            return Ok(Some(path));
        }
        bail!("本地 web_server 产物不存在: {}", path.display());
    }
    let repo = repo_root()?;
    let current = current_exe_path().ok();
    let mut candidates = vec![
        repo.join("target/x86_64-unknown-linux-gnu/release/web_server"),
        repo.join("target/release/web_server"),
    ];
    if let Some(path) = find_runtime_artifact("web_server") {
        candidates.push(path);
    }
    if let Some(path) = current.filter(|path| !path_is_windows_exe(path)) {
        candidates.push(path);
    }
    candidates
        .into_iter()
        .find(|path| path.exists() && path.is_file())
        .map(Some)
        .ok_or_else(|| {
            anyhow!("未找到可上传的 Linux web_server，请设置 local_web_bin 或先准备 Linux 产物")
        })
}

fn resolve_local_surreal_bin(target: &ManagedRemoteTarget) -> Result<Option<PathBuf>> {
    if !target.upload_surreal {
        return Ok(None);
    }
    if let Some(path) = target.local_surreal_bin.as_deref() {
        let path = PathBuf::from(path);
        if path.exists() && path.is_file() {
            return Ok(Some(path));
        }
        bail!("本地 SurrealDB 产物不存在: {}", path.display());
    }
    let repo = repo_root()?;
    let mut candidates = vec![
        repo.join("tools/surrealdb/linux/surreal"),
        repo.join("tools/surrealdb/surreal"),
    ];
    if let Some(path) = bundled_surreal_binary().filter(|path| !path_is_windows_exe(path)) {
        candidates.push(path);
    }
    candidates
        .into_iter()
        .find(|path| path.exists() && path.is_file())
        .map(Some)
        .ok_or_else(|| {
            anyhow!("未找到可上传的 Linux surreal，请设置 local_surreal_bin 或关闭 upload_surreal")
        })
}

fn resolve_local_resource_dir(target: &ManagedRemoteTarget) -> Result<Option<PathBuf>> {
    if !target.upload_resource {
        return Ok(None);
    }
    let path = target
        .local_resource_dir
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            repo_root()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join("resource/surreal")
        });
    if path.exists() && path.is_dir() {
        Ok(Some(path))
    } else {
        bail!("本地 resource/surreal 目录不存在: {}", path.display());
    }
}

fn resolve_local_viewer_dir(target: &ManagedRemoteTarget) -> Result<Option<PathBuf>> {
    if !target.upload_viewer {
        return Ok(None);
    }
    let path = target
        .local_viewer_dir
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            repo_root()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join("viewer")
        });
    if path.join("index.html").exists() {
        Ok(Some(path))
    } else {
        bail!(
            "本地 viewer 目录不存在或缺少 index.html: {}",
            path.display()
        );
    }
}

fn push_remote_local_artifact_check(
    checks: &mut Vec<ManagedSitePreflightCheck>,
    key: &str,
    label: &str,
    result: Result<Option<PathBuf>>,
) {
    match result {
        Ok(Some(path)) => checks.push(preflight_pass(
            key,
            label,
            format!("本地上传源可用: {}", path.display()),
            Some(path.display().to_string()),
        )),
        Ok(None) => checks.push(preflight_pass(
            key,
            label,
            "未启用上传，跳过本地源检查",
            None,
        )),
        Err(err) => checks.push(preflight_blocking(
            key,
            label,
            err.to_string(),
            None,
            Some("补齐本地 Linux 产物路径或关闭对应上传开关".to_string()),
            Vec::new(),
        )),
    }
}

fn remote_firewall_command(target: &ManagedRemoteTarget, mode: RemoteExecutionMode) -> String {
    if !target.open_firewall {
        return "echo FIREWALL_SKIPPED".to_string();
    }
    if mode == RemoteExecutionMode::User {
        return "echo FIREWALL_SKIPPED_USER_MODE".to_string();
    }
    let mut cmd = String::from("if command -v ufw >/dev/null 2>&1; then ");
    for cidr in normalize_remote_allowed_cidrs(target.allowed_cidrs.clone()) {
        if cidr == "0.0.0.0/0" || cidr == "::/0" {
            cmd.push_str(&format!(
                "ufw allow {}/tcp >/dev/null 2>&1 || true; ",
                target.remote_web_port
            ));
        } else {
            cmd.push_str(&format!(
                "ufw allow from {} to any port {} proto tcp >/dev/null 2>&1 || true; ",
                sh_quote(&cidr),
                target.remote_web_port
            ));
        }
    }
    cmd.push_str("echo FIREWALL_UFW_CONFIGURED; ");
    cmd.push_str("elif command -v firewall-cmd >/dev/null 2>&1; then ");
    cmd.push_str(&format!(
        "firewall-cmd --permanent --add-port={}/tcp >/dev/null 2>&1 || true; firewall-cmd --reload >/dev/null 2>&1 || true; echo FIREWALLD_CONFIGURED; ",
        target.remote_web_port
    ));
    cmd.push_str("else echo FIREWALL_TOOL_MISSING; fi");
    mode.privileged_shell(&cmd)
}

fn remote_prepare_dirs_command(
    target: &ManagedRemoteTarget,
    site_dir: &str,
    mode: RemoteExecutionMode,
) -> String {
    let db_parent = remote_parent_dir(&target.remote_db_path, "/root/surreal_data");
    let web_parent = remote_parent_dir(&target.remote_web_bin, "/root");
    let surreal_parent = remote_parent_dir(&target.surreal_bin, "/usr/local/bin");
    let runtime_dirs = if mode == RemoteExecutionMode::User {
        format!(
            " {site_dir}/runtime/pids {site_dir}/runtime/logs",
            site_dir = sh_quote(site_dir)
        )
    } else {
        String::new()
    };
    let script = format!(
        "set -e; mkdir -p {site_dir} {db_parent} {web_parent} {surreal_parent} {resource_dir} {viewer_dir}",
        site_dir = sh_quote(site_dir),
        db_parent = sh_quote(&db_parent),
        web_parent = sh_quote(&web_parent),
        surreal_parent = sh_quote(&surreal_parent),
        resource_dir = sh_quote(&format!("{site_dir}/resource/surreal")),
        viewer_dir = sh_quote(&format!("{site_dir}/viewer")),
    );
    mode.privileged_shell(&format!("{script}{runtime_dirs}"))
}

async fn prepare_remote_server(
    site_id: &str,
    target: &ManagedRemoteTarget,
    mode: RemoteExecutionMode,
) -> Result<Vec<ManagedSitePreflightCheck>> {
    let site_dir = remote_site_dir(target, site_id);
    let local_web_bin = resolve_local_web_bin(target)?;
    let local_surreal_bin = resolve_local_surreal_bin(target)?;
    let local_resource_dir = resolve_local_resource_dir(target)?;
    let local_viewer_dir = resolve_local_viewer_dir(target)?;
    let mut checks = Vec::new();

    run_ssh(
        target,
        &remote_prepare_dirs_command(target, &site_dir, mode),
    )
    .await?;
    checks.push(preflight_pass(
        "remote_prepare_dirs",
        "远端目录",
        "远端运行目录已创建",
        Some(site_dir.clone()),
    ));

    if let Some(path) = local_web_bin {
        upload_resource_path_native(&path, target, &target.remote_web_bin, false, false).await?;
        run_ssh(
            target,
            &format!("set -e; chmod +x {}", sh_quote(&target.remote_web_bin)),
        )
        .await?;
        checks.push(preflight_pass(
            "remote_upload_web_server",
            "上传 web_server",
            "web_server 已上传并授予执行权限",
            Some(target.remote_web_bin.clone()),
        ));
    }

    if let Some(path) = local_surreal_bin {
        upload_resource_path_native(&path, target, &target.surreal_bin, false, false).await?;
        run_ssh(
            target,
            &format!("set -e; chmod +x {}", sh_quote(&target.surreal_bin)),
        )
        .await?;
        checks.push(preflight_pass(
            "remote_upload_surreal",
            "上传 SurrealDB",
            "SurrealDB 已上传并授予执行权限",
            Some(target.surreal_bin.clone()),
        ));
    }

    if let Some(path) = local_resource_dir {
        let remote_resource = format!("{site_dir}/resource/surreal");
        upload_resource_path_native(&path, target, &remote_resource, true, true).await?;
        checks.push(preflight_pass(
            "remote_upload_resource",
            "上传 resource/surreal",
            "Surreal 初始化脚本已同步",
            Some(remote_resource),
        ));
    }

    if let Some(path) = local_viewer_dir {
        let remote_viewer = format!("{site_dir}/viewer");
        upload_resource_path_native(&path, target, &remote_viewer, true, true).await?;
        checks.push(preflight_pass(
            "remote_upload_viewer",
            "上传 Viewer",
            "Viewer 静态资源已同步",
            Some(remote_viewer),
        ));
    }

    let firewall_output = run_ssh(target, &remote_firewall_command(target, mode)).await?;
    checks.push(preflight_pass(
        "remote_firewall",
        "远端防火墙",
        if mode == RemoteExecutionMode::User {
            "普通用户模式跳过防火墙配置"
        } else if target.open_firewall {
            "Web 端口防火墙配置已执行"
        } else {
            "未启用自动防火墙配置"
        },
        Some(firewall_output),
    ));

    Ok(checks)
}

fn build_remote_site_config(
    site: &ManagedProjectSite,
    target: &ManagedRemoteTarget,
    db_user: &str,
    db_password: &str,
) -> Result<String> {
    let raw = fs::read_to_string(&site.config_path)
        .with_context(|| format!("读取站点 DbOption 失败: {}", site.config_path))?;
    let mut value = toml::from_str::<toml::Value>(&raw)?;
    let table = value
        .as_table_mut()
        .ok_or_else(|| anyhow!("站点 DbOption 不是 table 结构"))?;
    set_toml_string(table, "surreal_ip", "127.0.0.1");
    set_toml_integer(table, "surreal_port", target.remote_db_port as i64);
    set_toml_string(table, "surreal_user", db_user.to_string());
    set_toml_string(table, "surreal_password", db_password.to_string());
    set_toml_string(table, "surreal_script_dir", resolve_surreal_script_dir());

    let web_server = ensure_table(table, "web_server");
    set_toml_integer(web_server, "port", target.remote_web_port as i64);
    set_toml_string(web_server, "bind_host", target.web_bind_host.clone());
    set_toml_string(web_server, "public_base_url", remote_entry_url(target));
    set_toml_string(web_server, "frontend_url", remote_entry_url(target));
    set_toml_string(
        web_server,
        "backend_url",
        format!("http://127.0.0.1:{}", target.remote_web_port),
    );
    set_toml_bool(web_server, "auto_start_surreal", false);
    set_toml_string(web_server, "surreal_bin", target.surreal_bin.clone());
    set_toml_string(
        web_server,
        "surreal_data_path",
        target.remote_db_path.clone(),
    );
    set_toml_string(
        web_server,
        "surreal_bind",
        format!("{}:{}", target.db_bind_host, target.remote_db_port),
    );
    set_toml_string(web_server, "surreal_user", db_user.to_string());
    set_toml_string(web_server, "surreal_password", db_password.to_string());

    let surrealdb = ensure_table(table, "surrealdb");
    set_toml_string(surrealdb, "mode", "ws");
    set_toml_string(surrealdb, "ip", "127.0.0.1");
    set_toml_integer(surrealdb, "port", target.remote_db_port as i64);
    set_toml_string(surrealdb, "user", db_user.to_string());
    set_toml_string(surrealdb, "password", db_password.to_string());
    set_toml_string(surrealdb, "path", target.remote_db_path.clone());
    Ok(toml::to_string_pretty(&value)?)
}

fn remote_service_unit_names(site_id: &str) -> (String, String) {
    (
        format!("plant3d-surreal-{site_id}.service"),
        format!("plant3d-web-{site_id}.service"),
    )
}

async fn install_remote_services(
    site: &ManagedProjectSite,
    target: &ManagedRemoteTarget,
    db_user: &str,
    db_password: &str,
    deploy_id: Option<&str>,
    mode: RemoteExecutionMode,
) -> Result<()> {
    let site_dir = remote_site_dir(target, &site.site_id);
    let remote_config = format!("{site_dir}/DbOption.toml");
    let (surreal_unit, web_unit) = remote_service_unit_names(&site.site_id);
    let web_bin = target.remote_web_bin.trim();
    let site_token = remote_site_token(&site.site_id, deploy_id);
    let surreal_unit_content = format!(
        r#"[Unit]
Description=Plant3D SurrealDB {site_id}
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart={surreal_bin} start --log info --user {db_user} --pass {db_password} --bind {db_bind}:{db_port} rocksdb://{db_path}
Restart=on-failure
RestartSec=5
LimitNOFILE=1048576

[Install]
WantedBy=multi-user.target
"#,
        site_id = site.site_id,
        surreal_bin = target.surreal_bin,
        db_user = db_user,
        db_password = db_password,
        db_bind = target.db_bind_host,
        db_port = target.remote_db_port,
        db_path = target.remote_db_path
    );
    let web_unit_content = format!(
        r#"[Unit]
Description=Plant3D Web Site {site_id}
After=network-online.target {surreal_unit}
Wants=network-online.target {surreal_unit}

[Service]
Type=simple
WorkingDirectory={site_dir}
Environment=PLANT3D_DEPLOY_ID={deploy_id}
Environment=PLANT3D_DEPLOYMENT_MODE={deployment_mode}
Environment=PLANT3D_DEPLOY_DEGRADED=false
Environment=PLANT3D_SITE_TOKEN={site_token}
ExecStart={web_bin} --config {config_no_ext}
Restart=on-failure
RestartSec=5
LimitNOFILE=1048576

[Install]
WantedBy=multi-user.target
"#,
        site_id = site.site_id,
        surreal_unit = surreal_unit,
        site_dir = site_dir,
        deploy_id = deploy_id.unwrap_or("unknown"),
        deployment_mode = mode.status_label(),
        site_token = site_token,
        web_bin = web_bin,
        config_no_ext = remote_config.trim_end_matches(".toml")
    );
    let script = format!(
        "set -e; mkdir -p {site_dir} {db_parent}; cat > /etc/systemd/system/{surreal_unit} <<'EOF_SUR'\n{surreal_unit_content}EOF_SUR\ncat > /etc/systemd/system/{web_unit} <<'EOF_WEB'\n{web_unit_content}EOF_WEB\nsystemctl daemon-reload",
        site_dir = sh_quote(&site_dir),
        db_parent = sh_quote(&remote_parent_dir(
            &target.remote_db_path,
            "/root/surreal_data"
        )),
        surreal_unit = surreal_unit,
        web_unit = web_unit,
        surreal_unit_content = surreal_unit_content,
        web_unit_content = web_unit_content,
    );
    run_ssh(target, &mode.privileged_shell(&script)).await?;
    Ok(())
}

async fn restart_remote_services(
    site_id: &str,
    target: &ManagedRemoteTarget,
    mode: RemoteExecutionMode,
) -> Result<()> {
    let (surreal_unit, web_unit) = remote_service_unit_names(site_id);
    let script = format!(
        "set -e; systemctl stop {web_unit} 2>/dev/null || true; systemctl stop {surreal_unit} 2>/dev/null || true; systemctl enable --now {surreal_unit}; sleep 2; systemctl enable --now {web_unit}; sleep 2; systemctl is-active {surreal_unit}; systemctl is-active {web_unit}",
        web_unit = web_unit,
        surreal_unit = surreal_unit
    );
    run_ssh(target, &mode.privileged_shell(&script)).await?;
    Ok(())
}

fn remote_user_script_paths(site_dir: &str) -> (String, String, String) {
    (
        format!("{site_dir}/start.sh"),
        format!("{site_dir}/stop.sh"),
        format!("{site_dir}/status.sh"),
    )
}

fn user_mode_start_script(
    site: &ManagedProjectSite,
    target: &ManagedRemoteTarget,
    db_user: &str,
    db_password: &str,
    deploy_id: Option<&str>,
) -> String {
    let site_dir = remote_site_dir(target, &site.site_id);
    let remote_config = format!("{site_dir}/DbOption.toml");
    let deploy_id = deploy_id.unwrap_or("unknown");
    let site_token = remote_site_token(&site.site_id, Some(deploy_id));
    format!(
        r#"#!/usr/bin/env bash
set -euo pipefail
SITE_DIR={site_dir}
PID_DIR="$SITE_DIR/runtime/pids"
LOG_DIR="$SITE_DIR/runtime/logs"
DB_PID_FILE="$PID_DIR/surreal.pid"
WEB_PID_FILE="$PID_DIR/web_server.pid"
mkdir -p "$PID_DIR" "$LOG_DIR"

is_running() {{
  local pid_file="$1"
  [ -f "$pid_file" ] && kill -0 "$(cat "$pid_file")" >/dev/null 2>&1
}}

if is_running "$DB_PID_FILE"; then
  echo "surreal already running: $(cat "$DB_PID_FILE")"
else
  rm -f "$DB_PID_FILE"
  nohup {surreal_bin} start --log info --user {db_user} --pass {db_password} --bind {db_bind}:{db_port} rocksdb://{db_path} >> "$LOG_DIR/surreal.log" 2>&1 &
  echo $! > "$DB_PID_FILE"
fi

sleep 2

if is_running "$WEB_PID_FILE"; then
  echo "web_server already running: $(cat "$WEB_PID_FILE")"
else
  rm -f "$WEB_PID_FILE"
  PLANT3D_DEPLOY_ID={deploy_id} \
  PLANT3D_DEPLOYMENT_MODE=user \
  PLANT3D_DEPLOY_DEGRADED=true \
  PLANT3D_SITE_TOKEN={site_token} \
  nohup {web_bin} --config {config_no_ext} >> "$LOG_DIR/web_server.log" 2>&1 &
  echo $! > "$WEB_PID_FILE"
fi

"$SITE_DIR/status.sh"
"#,
        site_dir = sh_quote(&site_dir),
        surreal_bin = sh_quote(&target.surreal_bin),
        db_user = sh_quote(db_user),
        db_password = sh_quote(db_password),
        db_bind = sh_quote(&target.db_bind_host),
        db_port = target.remote_db_port,
        db_path = sh_quote(&target.remote_db_path),
        deploy_id = sh_quote(deploy_id),
        site_token = sh_quote(&site_token),
        web_bin = sh_quote(&target.remote_web_bin),
        config_no_ext = sh_quote(remote_config.trim_end_matches(".toml")),
    )
}

fn user_mode_stop_script() -> String {
    r#"#!/usr/bin/env bash
set -euo pipefail
SITE_DIR="$(cd "$(dirname "$0")" && pwd)"
PID_DIR="$SITE_DIR/runtime/pids"

stop_pid_file() {
  local label="$1"
  local pid_file="$2"
  if [ ! -f "$pid_file" ]; then
    echo "$label not running"
    return 0
  fi
  local pid
  pid="$(cat "$pid_file" 2>/dev/null || true)"
  if [ -z "$pid" ] || ! kill -0 "$pid" >/dev/null 2>&1; then
    rm -f "$pid_file"
    echo "$label stale pid cleared"
    return 0
  fi
  kill "$pid" >/dev/null 2>&1 || true
  for _ in 1 2 3 4 5; do
    if ! kill -0 "$pid" >/dev/null 2>&1; then
      rm -f "$pid_file"
      echo "$label stopped"
      return 0
    fi
    sleep 1
  done
  kill -9 "$pid" >/dev/null 2>&1 || true
  rm -f "$pid_file"
  echo "$label killed"
}

stop_pid_file web_server "$PID_DIR/web_server.pid"
stop_pid_file surreal "$PID_DIR/surreal.pid"
"#
    .to_string()
}

fn user_mode_status_script() -> String {
    r#"#!/usr/bin/env bash
set -euo pipefail
SITE_DIR="$(cd "$(dirname "$0")" && pwd)"
PID_DIR="$SITE_DIR/runtime/pids"

status_pid_file() {
  local label="$1"
  local pid_file="$2"
  if [ -f "$pid_file" ] && kill -0 "$(cat "$pid_file")" >/dev/null 2>&1; then
    echo "$label=running pid=$(cat "$pid_file")"
  else
    echo "$label=stopped"
    return 1
  fi
}

db_ok=0
web_ok=0
status_pid_file surreal "$PID_DIR/surreal.pid" || db_ok=1
status_pid_file web_server "$PID_DIR/web_server.pid" || web_ok=1
exit $((db_ok + web_ok))
"#
    .to_string()
}

async fn install_remote_user_scripts(
    site: &ManagedProjectSite,
    target: &ManagedRemoteTarget,
    db_user: &str,
    db_password: &str,
    deploy_id: Option<&str>,
) -> Result<()> {
    let site_dir = remote_site_dir(target, &site.site_id);
    let (start_path, stop_path, status_path) = remote_user_script_paths(&site_dir);
    let start_script = user_mode_start_script(site, target, db_user, db_password, deploy_id);
    let stop_script = user_mode_stop_script();
    let status_script = user_mode_status_script();
    let cmd = format!(
        "set -e; mkdir -p {site_dir} {pid_dir} {log_dir}; cat > {start_path} <<'EOF_START'\n{start_script}EOF_START\ncat > {stop_path} <<'EOF_STOP'\n{stop_script}EOF_STOP\ncat > {status_path} <<'EOF_STATUS'\n{status_script}EOF_STATUS\nchmod +x {start_path} {stop_path} {status_path}",
        site_dir = sh_quote(&site_dir),
        pid_dir = sh_quote(&format!("{site_dir}/runtime/pids")),
        log_dir = sh_quote(&format!("{site_dir}/runtime/logs")),
        start_path = sh_quote(&start_path),
        stop_path = sh_quote(&stop_path),
        status_path = sh_quote(&status_path),
        start_script = start_script,
        stop_script = stop_script,
        status_script = status_script,
    );
    run_ssh(target, &cmd).await?;
    Ok(())
}

async fn restart_remote_user_scripts(site_id: &str, target: &ManagedRemoteTarget) -> Result<()> {
    let site_dir = remote_site_dir(target, site_id);
    let (start_path, stop_path, status_path) = remote_user_script_paths(&site_dir);
    let cmd = format!(
        "set -e; if [ -x {stop_path} ]; then {stop_path} || true; fi; {start_path}; sleep 2; {status_path}",
        stop_path = sh_quote(&stop_path),
        start_path = sh_quote(&start_path),
        status_path = sh_quote(&status_path),
    );
    run_ssh(target, &cmd).await?;
    Ok(())
}

async fn validate_remote_http(site_id: &str, target: &ManagedRemoteTarget) -> Result<()> {
    let base_url = remote_entry_url(target);
    let base_url = base_url.trim_end_matches('/');
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(15))
        .build()
        .context("创建远端验收 HTTP client 失败")?;
    let status_url = format!("{base_url}/api/status");
    let status_value = client
        .get(&status_url)
        .send()
        .await
        .with_context(|| format!("请求远端 /api/status 失败: {status_url}"))?
        .error_for_status()
        .with_context(|| format!("远端 /api/status HTTP 状态异常: {status_url}"))?
        .json::<serde_json::Value>()
        .await
        .context("读取远端 /api/status JSON 失败")?;
    let status_database_connected = status_value
        .get("database_connected")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let status_surrealdb_connected = status_value
        .get("surrealdb_connected")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let db_check_url = format!("{base_url}/api/database/connection/check");
    let db_check = client
        .get(&db_check_url)
        .send()
        .await
        .with_context(|| format!("请求远端数据库连接检查失败: {db_check_url}"))?
        .error_for_status()
        .context("远端数据库连接检查 HTTP 状态异常")?
        .json::<serde_json::Value>()
        .await
        .context("读取远端数据库连接检查 JSON 失败")?;
    let db_check_connected = db_check
        .get("connected")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !(status_database_connected && status_surrealdb_connected) && !db_check_connected {
        let surreal_status_url = format!("{base_url}/api/surreal/status");
        let surreal_status = client
            .get(&surreal_status_url)
            .send()
            .await
            .ok()
            .and_then(|resp| resp.error_for_status().ok());
        let surreal_status_json = match surreal_status {
            Some(resp) => resp.json::<serde_json::Value>().await.ok(),
            None => None,
        };
        let surreal_listening = surreal_status_json
            .as_ref()
            .and_then(|v| v.get("listening"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        bail!(
            "远端状态未通过: status.database_connected={status_database_connected}, status.surrealdb_connected={status_surrealdb_connected}, database_check.connected={db_check_connected}, surreal.listening={surreal_listening}"
        );
    }
    let identity_url = format!("{base_url}/api/site/identity");
    let identity = client
        .get(&identity_url)
        .send()
        .await
        .with_context(|| format!("请求远端站点身份失败: {identity_url}"))?
        .error_for_status()
        .context("远端站点身份 HTTP 状态异常")?
        .json::<serde_json::Value>()
        .await
        .context("读取远端站点身份 JSON 失败")?;
    let remote_site_id = identity
        .get("site_id")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if !remote_site_id.is_empty() && remote_site_id != site_id {
        bail!("远端站点身份不一致: expected={site_id}, actual={remote_site_id}");
    }
    let agent_status_url = format!("{base_url}/api/site/agent-status");
    client
        .get(&agent_status_url)
        .send()
        .await
        .with_context(|| format!("请求远端站点 Agent 状态失败: {agent_status_url}"))?
        .error_for_status()
        .context("远端站点 Agent 状态 HTTP 状态异常")?;
    if target.upload_viewer {
        let viewer_url = format!("{base_url}/");
        client
            .get(&viewer_url)
            .send()
            .await
            .with_context(|| format!("请求远端 Viewer 失败: {viewer_url}"))?
            .error_for_status()
            .with_context(|| format!("远端 Viewer HTTP 状态异常: {viewer_url}"))?;
    }
    Ok(())
}

pub async fn remote_prepare_site(
    site_id: &str,
    req: Option<ManagedRemoteDeployRequest>,
) -> Result<ManagedRemoteDeployStatus> {
    get_site(site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
    let target = resolve_remote_target(site_id, req)?;
    let mode = detect_remote_execution_mode(&target).await?;
    let mut status = ManagedRemoteDeployStatus {
        site_id: site_id.to_string(),
        target_id: target.id.clone(),
        deploy_id: Some(format!(
            "remote-{site_id}-{}",
            chrono::Utc::now().timestamp()
        )),
        deploy_task_id: None,
        deployment_mode: Some(mode.status_label().to_string()),
        degraded: mode.degraded(),
        status: "running".to_string(),
        current_step: "remote_prepare".to_string(),
        remote_entry_url: Some(remote_entry_url(&target)),
        checked_at: now_rfc3339(),
        last_error: None,
        checks: Vec::new(),
    };
    save_remote_deploy_status(&status)?;

    match prepare_remote_server(site_id, &target, mode).await {
        Ok(mut checks) => {
            if mode == RemoteExecutionMode::User {
                checks.push(preflight_warning(
                    "remote_user_mode_degraded",
                    "普通用户降级部署",
                    "目标用户无 root/sudo，已跳过 systemd 和防火墙等系统级配置",
                    None,
                    Some("使用 start.sh/stop.sh/status.sh 管理进程；不承诺开机自启".to_string()),
                    Vec::new(),
                ));
            }
            status.status = "prepared".to_string();
            status.current_step = "远端服务器准备完成".to_string();
            status.checked_at = now_rfc3339();
            status.checks = checks;
            status.last_error = None;
            save_remote_deploy_status(&status)?;
            Ok(status)
        }
        Err(err) => {
            status.status = "failed".to_string();
            status.current_step = "远端服务器准备失败".to_string();
            status.checked_at = now_rfc3339();
            status.last_error = Some(err.to_string());
            status.checks = vec![preflight_blocking(
                "remote_prepare",
                "远端服务器准备",
                err.to_string(),
                Some(format!("{}@{}", target.ssh_user, target.host)),
                Some("检查 SSH 权限、本地产物路径、远端目录和防火墙工具".to_string()),
                Vec::new(),
            )];
            let _ = save_remote_deploy_status(&status);
            Err(err)
        }
    }
}

pub async fn remote_preflight_site(
    site_id: &str,
    req: Option<ManagedRemoteDeployRequest>,
) -> Result<ManagedRemoteDeployStatus> {
    let site = get_site(site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
    let target = resolve_remote_target(site_id, req)?;
    let mut checks = Vec::new();

    if target.target_os == ManagedRemoteTargetOs::Windows {
        checks.push(preflight_blocking(
            "target_os_windows",
            "目标操作系统",
            "Windows 目标已可在向导中选择，但当前远端执行器仍是 Linux/systemd 命令，暂不直接执行",
            Some("windows".to_string()),
            Some("下一步需要补 Windows OpenSSH + PowerShell 服务安装适配器".to_string()),
            Vec::new(),
        ));
    } else {
        checks.push(preflight_pass(
            "target_os",
            "目标操作系统",
            format!("使用 {:?} 的 Linux/systemd 部署适配器", target.target_os),
            Some(remote_target_os_to_str(target.target_os).to_string()),
        ));
    }

    if site.parse_status == ManagedSiteParseStatus::Running {
        checks.push(preflight_blocking(
            "local_parse_state",
            "本地解析状态",
            "解析任务正在运行，不能复制 RocksDB 目录",
            None,
            Some("等待解析结束或先停止站点".to_string()),
            Vec::new(),
        ));
    } else if matches!(
        site.status,
        ManagedSiteStatus::Running | ManagedSiteStatus::Starting | ManagedSiteStatus::Stopping
    ) {
        checks.push(preflight_blocking(
            "local_site_state",
            "本地站点状态",
            format!("当前状态为 {:?}，不能安全复制数据库目录", site.status),
            None,
            Some("先停止本机受管站点后再远端部署".to_string()),
            Vec::new(),
        ));
    } else {
        checks.push(preflight_pass(
            "local_site_state",
            "本地站点状态",
            "本地站点处于可复制状态",
            None,
        ));
    }

    let db_path = Path::new(&site.db_data_path);
    if !db_path.exists() {
        checks.push(preflight_blocking(
            "local_db_path",
            "本地数据库目录",
            "本地 RocksDB 目录不存在",
            Some(site.db_data_path.clone()),
            Some("先完成解析/生成，确保 runtime/admin_sites 下存在 surreal.db".to_string()),
            Vec::new(),
        ));
    } else if path_size_bytes(db_path) == 0 {
        checks.push(preflight_blocking(
            "local_db_size",
            "本地数据库大小",
            "本地 RocksDB 目录为空",
            Some(site.db_data_path.clone()),
            Some("重新解析或检查数据库生成结果".to_string()),
            Vec::new(),
        ));
    } else {
        checks.push(preflight_pass(
            "local_db_path",
            "本地数据库目录",
            format!("本地数据库目录存在，大小 {}", path_size_bytes(db_path)),
            Some(site.db_data_path.clone()),
        ));
    }

    checks.push(preflight_pass(
        "native_ssh_sftp",
        "Rust 原生 SSH/SFTP",
        "远端命令和文件上传使用内置 ssh2/SFTP，不再依赖本机 sshpass/ssh/rsync 命令",
        None,
    ));

    if target
        .ssh_password
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        checks.push(preflight_pass(
            "ssh_password",
            "SSH 密码",
            "已配置 SSH 密码（测试阶段允许落库）",
            None,
        ));
    } else if std::env::var(&target.password_env)
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
    {
        checks.push(preflight_pass(
            "password_env",
            "SSH 密码环境变量",
            format!("已读取 {}", target.password_env),
            None,
        ));
    } else {
        checks.push(preflight_blocking(
            "password_env",
            "SSH 密码环境变量",
            format!("未设置 {}", target.password_env),
            None,
            Some("通过环境变量提供 SSH 密码，禁止写入仓库".to_string()),
            Vec::new(),
        ));
    }

    push_remote_local_artifact_check(
        &mut checks,
        "local_web_server_artifact",
        "本地 web_server 产物",
        resolve_local_web_bin(&target),
    );
    push_remote_local_artifact_check(
        &mut checks,
        "local_surreal_artifact",
        "本地 SurrealDB 产物",
        resolve_local_surreal_bin(&target),
    );
    push_remote_local_artifact_check(
        &mut checks,
        "local_resource_artifact",
        "本地 resource/surreal",
        resolve_local_resource_dir(&target),
    );
    push_remote_local_artifact_check(
        &mut checks,
        "local_viewer_artifact",
        "本地 Viewer",
        resolve_local_viewer_dir(&target),
    );

    if !matches!(
        target.db_bind_host.as_str(),
        "127.0.0.1" | "localhost" | "::1"
    ) {
        checks.push(preflight_warning(
            "remote_db_bind_host",
            "远端数据库监听地址",
            format!(
                "SurrealDB 将监听 {}:{}，建议默认只绑定 127.0.0.1",
                target.db_bind_host, target.remote_db_port
            ),
            None,
            Some("除非明确需要外部访问数据库，否则保持 db_bind_host=127.0.0.1".to_string()),
            Vec::new(),
        ));
    }

    let mut remote_mode = None;
    if checks
        .iter()
        .all(|check| check.status != ManagedSitePreflightStatus::Blocking)
    {
        match detect_remote_execution_mode(&target).await {
            Ok(mode) => {
                let status = if mode == RemoteExecutionMode::User {
                    ManagedSitePreflightStatus::Warning
                } else {
                    ManagedSitePreflightStatus::Pass
                };
                checks.push(ManagedSitePreflightCheck {
                    key: "remote_execution_mode".to_string(),
                    label: "远端执行模式".to_string(),
                    status,
                    message: match mode {
                        RemoteExecutionMode::Root => "SSH 用户为 root，将使用 systemd 完整部署".to_string(),
                        RemoteExecutionMode::Sudo => {
                            "SSH 用户可免密 sudo，将使用 systemd 完整部署".to_string()
                        }
                        RemoteExecutionMode::User => {
                            "SSH 用户无免密 sudo，将使用脚本降级部署".to_string()
                        }
                    },
                    detail: Some(mode.status_label().to_string()),
                    action_hint: (mode == RemoteExecutionMode::User).then(|| {
                        "需确认 remote_root/remote_db_path/web_server/surreal 路径当前用户可读写；不会配置 systemd/防火墙/开机自启".to_string()
                    }),
                    pids: Vec::new(),
                });
                remote_mode = Some(mode);
            }
            Err(err) => checks.push(preflight_blocking(
                "remote_execution_mode",
                "远端执行模式",
                err.to_string(),
                Some(format!("{}@{}", target.ssh_user, target.host)),
                Some("检查 SSH 账号、密码和 sudo 配置".to_string()),
                Vec::new(),
            )),
        }
    }

    if checks
        .iter()
        .all(|check| check.status != ManagedSitePreflightStatus::Blocking)
    {
        let mode = remote_mode.unwrap_or(RemoteExecutionMode::User);
        let (surreal_unit, web_unit) = remote_service_unit_names(site_id);
        let surreal_required = !(target.auto_prepare && target.upload_surreal);
        let web_required = !(target.auto_prepare && target.upload_web_server);
        let surreal_parent = remote_parent_dir(&target.surreal_bin, "/usr/local/bin");
        let web_parent = remote_parent_dir(&target.remote_web_bin, "/root");
        let db_parent = remote_parent_dir(&target.remote_db_path, "/root/surreal_data");
        let site_dir = remote_site_dir(&target, site_id);
        let script = if mode == RemoteExecutionMode::User {
            format!(
                "set -e; surreal={surreal}; web_bin={web_bin}; mkdir -p {root} {site_dir} {db_parent} {surreal_parent} {web_parent} {pid_dir} {log_dir}; if [ -x \"$surreal\" ]; then echo \"surreal=$surreal\"; elif command -v \"$surreal\" >/dev/null 2>&1; then command -v \"$surreal\"; elif [ {surreal_required} -eq 1 ]; then echo SURREAL_MISSING:$surreal; exit 43; else echo SURREAL_WILL_UPLOAD:$surreal; fi; if [ -x \"$web_bin\" ]; then echo \"web_bin=$web_bin\"; elif [ {web_required} -eq 1 ]; then echo WEB_BIN_MISSING:$web_bin; exit 44; else echo WEB_BIN_WILL_UPLOAD:$web_bin; fi; db_pid={db_pid_file}; web_pid={web_pid_file}; if ss -ltn 2>/dev/null | grep -Eq '(^|[[:space:]])[^[:space:]]*:{db_port}[[:space:]]' && ! ([ -f \"$db_pid\" ] && kill -0 \"$(cat \"$db_pid\")\" >/dev/null 2>&1); then echo DB_PORT_IN_USE; exit 45; fi; if ss -ltn 2>/dev/null | grep -Eq '(^|[[:space:]])[^[:space:]]*:{web_port}[[:space:]]' && ! ([ -f \"$web_pid\" ] && kill -0 \"$(cat \"$web_pid\")\" >/dev/null 2>&1); then echo WEB_PORT_IN_USE; exit 46; fi; df -Pk {root} | tail -1; echo USER_MODE_READY",
                surreal = sh_quote(&target.surreal_bin),
                web_bin = sh_quote(&target.remote_web_bin),
                root = sh_quote(&target.remote_root),
                site_dir = sh_quote(&site_dir),
                db_parent = sh_quote(&db_parent),
                surreal_parent = sh_quote(&surreal_parent),
                web_parent = sh_quote(&web_parent),
                pid_dir = sh_quote(&format!("{site_dir}/runtime/pids")),
                log_dir = sh_quote(&format!("{site_dir}/runtime/logs")),
                db_pid_file = sh_quote(&format!("{site_dir}/runtime/pids/surreal.pid")),
                web_pid_file = sh_quote(&format!("{site_dir}/runtime/pids/web_server.pid")),
                surreal_required = if surreal_required { 1 } else { 0 },
                web_required = if web_required { 1 } else { 0 },
                db_port = target.remote_db_port,
                web_port = target.remote_web_port,
            )
        } else {
            mode.privileged_shell(&format!(
                "set -e; surreal={surreal}; web_bin={web_bin}; mkdir -p {root} {db_parent} {surreal_parent} {web_parent}; if [ -x \"$surreal\" ]; then echo \"surreal=$surreal\"; elif command -v \"$surreal\" >/dev/null 2>&1; then command -v \"$surreal\"; elif [ {surreal_required} -eq 1 ]; then echo SURREAL_MISSING:$surreal; exit 43; else echo SURREAL_WILL_UPLOAD:$surreal; fi; if [ -x \"$web_bin\" ]; then echo \"web_bin=$web_bin\"; elif [ {web_required} -eq 1 ]; then echo WEB_BIN_MISSING:$web_bin; exit 44; else echo WEB_BIN_WILL_UPLOAD:$web_bin; fi; if ss -ltn 2>/dev/null | grep -Eq '(^|[[:space:]])[^[:space:]]*:{db_port}[[:space:]]' && ! systemctl is-active --quiet {surreal_unit}; then echo DB_PORT_IN_USE; exit 45; fi; if ss -ltn 2>/dev/null | grep -Eq '(^|[[:space:]])[^[:space:]]*:{web_port}[[:space:]]' && ! systemctl is-active --quiet {web_unit}; then echo WEB_PORT_IN_USE; exit 46; fi; df -Pk {root} | tail -1",
                surreal = sh_quote(&target.surreal_bin),
                web_bin = sh_quote(&target.remote_web_bin),
                root = sh_quote(&target.remote_root),
                db_parent = sh_quote(&db_parent),
                surreal_parent = sh_quote(&surreal_parent),
                web_parent = sh_quote(&web_parent),
                surreal_required = if surreal_required { 1 } else { 0 },
                web_required = if web_required { 1 } else { 0 },
                db_port = target.remote_db_port,
                web_port = target.remote_web_port,
                surreal_unit = surreal_unit,
                web_unit = web_unit,
            ))
        };
        match run_ssh(&target, &script).await {
            Ok(out) => checks.push(preflight_pass(
                "remote_machine",
                "远端机器",
                "SSH、目录、SurrealDB/Web 二进制、端口预检通过",
                Some(out),
            )),
            Err(err) => checks.push(preflight_blocking(
                "remote_machine",
                "远端机器",
                err.to_string(),
                Some(format!("{}@{}", target.ssh_user, target.host)),
                Some("检查远端端口、磁盘、surreal 路径和 SSH 权限".to_string()),
                Vec::new(),
            )),
        }
    }

    let blocking_count = checks
        .iter()
        .filter(|check| check.status == ManagedSitePreflightStatus::Blocking)
        .count();
    let warning_count = checks
        .iter()
        .filter(|check| check.status == ManagedSitePreflightStatus::Warning)
        .count();
    let status = ManagedRemoteDeployStatus {
        site_id: site_id.to_string(),
        target_id: target.id.clone(),
        deploy_id: None,
        deploy_task_id: None,
        deployment_mode: remote_mode.map(|mode| mode.status_label().to_string()),
        degraded: remote_mode
            .map(RemoteExecutionMode::degraded)
            .unwrap_or(false),
        status: if blocking_count == 0 {
            "ready".to_string()
        } else {
            "blocked".to_string()
        },
        current_step: format!("{blocking_count} 个阻断 / {warning_count} 个警告"),
        remote_entry_url: Some(remote_entry_url(&target)),
        checked_at: now_rfc3339(),
        last_error: None,
        checks,
    };
    save_remote_deploy_status(&status)?;
    Ok(status)
}

pub async fn remote_deploy_site(
    site_id: String,
    req: Option<ManagedRemoteDeployRequest>,
) -> Result<ManagedRemoteDeployStatus> {
    remote_deploy_site_with_task_id(site_id, req, None).await
}

pub async fn remote_deploy_site_with_task_id(
    site_id: String,
    req: Option<ManagedRemoteDeployRequest>,
    task_id: Option<String>,
) -> Result<ManagedRemoteDeployStatus> {
    let request = match req {
        Some(req) => Some(req),
        None => {
            let saved = get_remote_deploy_status(&site_id)?;
            (saved.target_id != "default").then_some(ManagedRemoteDeployRequest {
                target_id: Some(saved.target_id),
                target: None,
            })
        }
    };
    let target = resolve_remote_target(&site_id, request)?;
    let mut status = remote_preflight_site(
        &site_id,
        Some(ManagedRemoteDeployRequest {
            target_id: Some(target.id.clone()),
            target: None,
        }),
    )
    .await?;
    if status.status == "blocked" {
        bail!("远端部署预检未通过: {}", status.current_step);
    }
    let mode = detect_remote_execution_mode(&target).await?;
    status.deploy_id = Some(format!(
        "remote-{site_id}-{}",
        chrono::Utc::now().timestamp()
    ));
    status.deploy_task_id = task_id;
    status.deployment_mode = Some(mode.status_label().to_string());
    status.degraded = mode.degraded();
    status.checked_at = now_rfc3339();
    save_remote_deploy_status(&status)?;
    if target.auto_prepare {
        status.status = "running".to_string();
        status.current_step = "remote_prepare".to_string();
        status.checked_at = now_rfc3339();
        save_remote_deploy_status(&status)?;
        let prepared = remote_prepare_site(
            &site_id,
            Some(ManagedRemoteDeployRequest {
                target_id: Some(target.id.clone()),
                target: None,
            }),
        )
        .await?;
        status.checks = prepared.checks;
    }
    let (site, db_user, db_password) = load_site_and_credentials(&site_id)?;
    status.status = "running".to_string();
    status.current_step = "preflight".to_string();
    status.checked_at = now_rfc3339();
    save_remote_deploy_status(&status)?;

    if generation_enabled(&site) && site.parse_status != ManagedSiteParseStatus::Parsed {
        status.current_step = "local_generation".to_string();
        status.checked_at = now_rfc3339();
        save_remote_deploy_status(&status)?;
        run_generation_pipeline(site_id.clone(), true).await?;
    }

    let site = load_raw_site(&site_id)?;
    let site_dir = remote_site_dir(&target, &site_id);
    status.current_step = "remote_stop".to_string();
    status.checked_at = now_rfc3339();
    save_remote_deploy_status(&status)?;
    let (surreal_unit, web_unit) = remote_service_unit_names(&site_id);
    let stop_script = if mode == RemoteExecutionMode::User {
        let (_, stop_path, _) = remote_user_script_paths(&site_dir);
        format!(
            "set -e; if [ -x {stop_path} ]; then {stop_path} || true; fi; mkdir -p {site_dir} {db_parent} {pid_dir} {log_dir}",
            stop_path = sh_quote(&stop_path),
            site_dir = sh_quote(&site_dir),
            db_parent = sh_quote(&remote_parent_dir(
                &target.remote_db_path,
                "/root/surreal_data"
            )),
            pid_dir = sh_quote(&format!("{site_dir}/runtime/pids")),
            log_dir = sh_quote(&format!("{site_dir}/runtime/logs")),
        )
    } else {
        mode.privileged_shell(&format!(
            "set -e; systemctl stop {web_unit} 2>/dev/null || true; systemctl stop {surreal_unit} 2>/dev/null || true; mkdir -p {site_dir} {db_parent}",
            web_unit = web_unit,
            surreal_unit = surreal_unit,
            site_dir = sh_quote(&site_dir),
            db_parent = sh_quote(&remote_parent_dir(&target.remote_db_path, "/root/surreal_data"))
        ))
    };
    run_ssh(&target, &stop_script).await?;

    status.current_step = "upload".to_string();
    status.checked_at = now_rfc3339();
    save_remote_deploy_status(&status)?;
    upload_db_native(&site, &target).await?;

    status.current_step = "remote_config".to_string();
    status.checked_at = now_rfc3339();
    save_remote_deploy_status(&status)?;
    let remote_config = build_remote_site_config(&site, &target, &db_user, &db_password)?;
    let local_remote_dir = site_runtime_dir(&site_id).join("remote");
    fs::create_dir_all(&local_remote_dir)?;
    let local_remote_config = local_remote_dir.join("DbOption.remote.toml");
    write_file_atomic(&local_remote_config, &remote_config)?;
    upload_file_native_async(
        &local_remote_config,
        &target,
        &format!("{site_dir}/DbOption.toml"),
    )
    .await?;
    if mode == RemoteExecutionMode::User {
        install_remote_user_scripts(
            &site,
            &target,
            &db_user,
            &db_password,
            status.deploy_id.as_deref(),
        )
        .await?;
    } else {
        install_remote_services(
            &site,
            &target,
            &db_user,
            &db_password,
            status.deploy_id.as_deref(),
            mode,
        )
        .await?;
    }

    status.current_step = "remote_start".to_string();
    status.checked_at = now_rfc3339();
    save_remote_deploy_status(&status)?;
    if mode == RemoteExecutionMode::User {
        restart_remote_user_scripts(&site_id, &target).await?;
    } else {
        restart_remote_services(&site_id, &target, mode).await?;
    }

    status.current_step = "validation".to_string();
    status.checked_at = now_rfc3339();
    save_remote_deploy_status(&status)?;
    validate_remote_http(&site_id, &target).await?;

    status.status = "completed".to_string();
    status.current_step = if mode == RemoteExecutionMode::User {
        "远端部署完成（普通用户降级模式）".to_string()
    } else {
        "远端部署完成".to_string()
    };
    status.remote_entry_url = Some(remote_entry_url(&target));
    status.checked_at = now_rfc3339();
    status.last_error = None;
    save_remote_deploy_status(&status)?;
    Ok(status)
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

    let active_job = active_sidecar_job(site_id);
    if let Some(job) = &active_job {
        let log_path = job.kind.log_path(site_id);
        append_log_line(
            &log_path,
            &format!(
                "🛑 请求取消 sidecar {} job {}",
                job.kind.label(),
                job.job_id
            ),
        );
        match crate::web_server::parse_sidecar_client::cancel_cli_job(&job.key, &job.job_id).await {
            Ok(_) => {
                append_log_line(
                    &log_path,
                    &format!(
                        "✅ sidecar {} job {} 取消请求已发送",
                        job.kind.label(),
                        job.job_id
                    ),
                );
                if wait_for_sidecar_job_terminal(site_id, job).await {
                    append_log_line(
                        &log_path,
                        &format!(
                            "✅ sidecar {} job {} 已进入终态",
                            job.kind.label(),
                            job.job_id
                        ),
                    );
                } else {
                    append_log_line(
                        &log_path,
                        &format!(
                            "⚠️ 等待 sidecar {} job {} 进入终态超时，将继续强制停止站点进程",
                            job.kind.label(),
                            job.job_id
                        ),
                    );
                    tracing::warn!(
                        site = %site_id,
                        job_id = %job.job_id,
                        "等待 sidecar job 取消完成超时，将继续停止站点进程"
                    );
                }
            }
            Err(err) => {
                append_log_line(
                    &log_path,
                    &format!(
                        "⚠️ sidecar {} job {} 取消请求失败: {}",
                        job.kind.label(),
                        job.job_id,
                        err.message
                    ),
                );
                tracing::warn!(
                    site = %site_id,
                    job_id = %job.job_id,
                    status = %err.status,
                    "请求取消 sidecar job 失败: {}",
                    err.message
                );
            }
        }
    }

    // 顺序：生产者先停（parse、web），消费者（db）最后停，避免 parse 写库时 db 突然消失。
    // 用守卫式 kill：仅在 (pid, 启动时刻) 与登记一致时才杀，防 PID 复用误杀。
    if let Some(pid) = site.parse_pid {
        kill_pid_guarded(site_id, PROC_ROLE_PARSE, pid).await?;
    }
    if let Some(pid) = site.viewer_pid {
        kill_pid_guarded(site_id, PROC_ROLE_VIEWER, pid).await?;
    }
    if let Some(pid) = site.web_pid {
        kill_pid_guarded(site_id, PROC_ROLE_WEB, pid).await?;
    }
    if let Some(pid) = site.db_pid {
        kill_pid_guarded(site_id, PROC_ROLE_DB, pid).await?;
    }
    kill_registered_site_processes(site_id).await;
    let _ = stop_site_ws_db_for_exclusivity(&site).await;
    // 兜底清理本站点的全部进程登记（覆盖 pid 为 None 未走 kill 的角色与历史残留行）。
    unregister_site_processes(site_id);

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

    let operation_was_running = site.parse_pid.is_some() || active_job.is_some();
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

pub async fn delete_site(site_id: &str) -> Result<bool> {
    let Some(site) = get_site(site_id)? else {
        return Ok(false);
    };

    let should_stop = site_has_active_processes(&site)
        || matches!(
            site.status,
            ManagedSiteStatus::Running | ManagedSiteStatus::Starting | ManagedSiteStatus::Stopping
        )
        || site.parse_status == ManagedSiteParseStatus::Running;
    if should_stop {
        let stop_result = stop_site(site_id).await?;
        if stop_result.conflict {
            bail!(
                "删除站点前停止进程时检测到端口冲突（web={:?} db={:?} viewer={:?}），请先排查外部占用",
                stop_result.web_conflict_pids,
                stop_result.db_conflict_pids,
                stop_result.viewer_conflict_pids
            );
        }
    } else {
        kill_registered_site_processes(site_id).await;
    }

    let _guard = lock_op()?;
    let changed = with_tx(|conn| {
        if load_site_with_conn(conn, site_id)?.is_none() {
            return Ok(0);
        }
        let rows = conn.execute(
            &format!("DELETE FROM {table} WHERE site_id = ?1", table = TABLE_NAME),
            [site_id],
        )?;
        Ok(rows)
    })?;
    unregister_site_processes(site_id);
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

fn reconcile_runtime_update(
    site: &ManagedProjectSite,
    cleanup_orphans: bool,
    actions: &mut Vec<String>,
) -> RuntimeUpdate {
    let db_probe = probe_managed_port(site.db_port, site.db_pid);
    let web_probe = probe_managed_port(site.web_port, site.web_pid);
    let viewer_probe = site
        .viewer_port
        .map(|port| probe_managed_port(port, site.viewer_pid))
        .unwrap_or_default();
    let db_running = db_probe.managed_running;
    let web_running = web_probe.managed_running;
    let viewer_running = viewer_probe.managed_running || pid_running(site.viewer_pid);
    let db_pid_alive = pid_running(site.db_pid);
    let web_pid_alive = pid_running(site.web_pid);
    let parse_running = pid_running(site.parse_pid);
    let mut update = RuntimeUpdate::default();
    let mut last_error = site.last_error.clone();

    if site.db_pid.is_some() && !db_running && !db_pid_alive {
        update.db_pid = Some(None);
        actions.push("清除失效 DB PID".to_string());
    }
    if site.web_pid.is_some() && !web_running && !web_pid_alive {
        update.web_pid = Some(None);
        actions.push("清除失效 Web PID".to_string());
    }
    if site.viewer_pid.is_some() && !viewer_running {
        update.viewer_pid = Some(None);
        actions.push("清除失效 Viewer PID".to_string());
    }
    if site.parse_pid.is_some() && !parse_running {
        update.parse_pid = Some(None);
        actions.push("清除失效 Parse PID".to_string());
        if site.parse_status == ManagedSiteParseStatus::Running {
            update.parse_status = Some(ManagedSiteParseStatus::Failed);
            last_error = Some("对账发现解析状态为 Running，但解析进程已退出".to_string());
            actions.push("修正解析状态为 Failed".to_string());
        }
    }

    if matches!(
        site.status,
        ManagedSiteStatus::Running | ManagedSiteStatus::Starting | ManagedSiteStatus::Stopping
    ) {
        if web_running && db_running {
            // 正常运行，无需改写 status。
        } else if site.status == ManagedSiteStatus::Starting && (web_pid_alive || parse_running) {
            // Web 进程已拉起但还未 bind 完成时不要过早判定半启动失败。
            actions.push("启动中：等待 Web 进程完成监听".to_string());
        } else if db_running && !web_running {
            update.status = Some(ManagedSiteStatus::Failed);
            last_error =
                Some("对账发现 Web 未监听，但 DB 仍在运行；请刷新验收或清理残留进程".to_string());
            actions.push("修正半启动状态为 Failed".to_string());
            if cleanup_orphans {
                update.db_pid = Some(None);
                if site.viewer_pid.is_some() {
                    update.viewer_pid = Some(None);
                    update.viewer_url = Some(None);
                }
                actions.push("已请求清理孤立 DB/Viewer 进程".to_string());
            }
        } else if !db_running && !web_running && !viewer_running && !parse_running {
            if site.status == ManagedSiteStatus::Starting {
                update.status = Some(ManagedSiteStatus::Failed);
                last_error = Some("启动中断：未发现有效 DB/Web/Parse 进程或监听端口".to_string());
                actions.push("修正无进程 Starting 为 Failed".to_string());
            } else if site.status == ManagedSiteStatus::Stopping {
                update.status = Some(ManagedSiteStatus::Stopped);
                last_error = None;
                actions.push("修正无进程 Stopping 为 Stopped".to_string());
            } else {
                update.status = Some(ManagedSiteStatus::Stopped);
                last_error = None;
                actions.push("修正无进程 Running 为 Stopped".to_string());
            }
        }
    }

    if last_error != site.last_error {
        update.last_error = Some(last_error);
    }
    update
}

fn runtime_update_has_changes(update: &RuntimeUpdate) -> bool {
    update.status.is_some()
        || update.parse_status.is_some()
        || update.db_pid.is_some()
        || update.web_pid.is_some()
        || update.viewer_port.is_some()
        || update.viewer_pid.is_some()
        || update.viewer_url.is_some()
        || update.parse_pid.is_some()
        || update.last_error.is_some()
        || update.entry_url.is_some()
        || update.last_parse_started_at.is_some()
        || update.last_parse_finished_at.is_some()
        || update.last_parse_duration_ms.is_some()
}

pub async fn reconcile_site(
    site_id: &str,
    cleanup_orphans: bool,
) -> Result<ManagedSiteReconcileResponse> {
    let site = task::spawn_blocking({
        let site_id = site_id.to_string();
        move || load_raw_site(&site_id)
    })
    .await
    .context("读取站点状态失败 (join error)")??;
    let mut actions = Vec::new();

    if cleanup_orphans {
        let db_orphan = probe_managed_port(site.db_port, site.db_pid).managed_running
            && !probe_managed_port(site.web_port, site.web_pid).managed_running;
        if db_orphan {
            if let Some(pid) = site.db_pid {
                kill_pid(pid).await?;
                actions.push(format!("清理孤立 DB 进程 PID {pid}"));
            }
            if let Some(pid) = site.viewer_pid {
                let _ = kill_pid(pid).await;
                actions.push(format!("清理孤立 Viewer 进程 PID {pid}"));
            }
        }
    }

    let fresh_site = task::spawn_blocking({
        let site_id = site_id.to_string();
        move || load_raw_site(&site_id)
    })
    .await
    .context("重新读取站点状态失败 (join error)")??;
    let update = reconcile_runtime_update(&fresh_site, cleanup_orphans, &mut actions);
    let changed = runtime_update_has_changes(&update);
    if changed {
        update_runtime(site_id, update)?;
    }
    let runtime = runtime_status(site_id)?;
    Ok(ManagedSiteReconcileResponse {
        site_id: site_id.to_string(),
        changed,
        actions,
        runtime,
    })
}

pub fn reconcile_sites_on_startup() -> Result<usize> {
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
    let mut changed = 0usize;
    for site in sites {
        let mut actions = Vec::new();
        let update = reconcile_runtime_update(&site, false, &mut actions);
        if runtime_update_has_changes(&update) {
            if let Err(err) = update_runtime(&site.site_id, update) {
                tracing::warn!(site = %site.site_id, "启动对账写回失败: {err}");
            } else {
                changed += 1;
            }
        }
    }
    Ok(changed)
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
    let active_sidecar_job = active_sidecar_job(site_id);

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
    let connectivity = probe_site_connectivity(&site, web_running);

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
        sidecar_job_kind: active_sidecar_job
            .as_ref()
            .map(|job| job.kind.key().to_string()),
        sidecar_job_id: active_sidecar_job.as_ref().map(|job| job.job_id.clone()),
        sidecar_job_status: active_sidecar_job.as_ref().map(|job| job.status.clone()),
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
        web_status_ok: connectivity.web_status_ok,
        database_connected: connectivity.database_connected,
        surrealdb_connected: connectivity.surrealdb_connected,
        site_identity_ok: connectivity.site_identity_ok,
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
    if site.parse_status == ManagedSiteParseStatus::Running {
        return (
            "parse-preparing".to_string(),
            "解析准备中".to_string(),
            parse_detail.or(Some(
                "解析进程启动前正在准备依赖索引或数据库环境".to_string(),
            )),
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
    fn unique_site_name_returns_base_when_available() {
        let used = HashSet::from(["OtherSite".to_string()]);
        assert_eq!(
            unique_site_name("AvevaPlantSample", &used),
            "AvevaPlantSample"
        );
    }

    #[test]
    fn unique_site_name_appends_next_available_suffix() {
        let used = HashSet::from([
            "AvevaPlantSample".to_string(),
            "AvevaPlantSample-2".to_string(),
        ]);
        assert_eq!(
            unique_site_name("AvevaPlantSample", &used),
            "AvevaPlantSample-3"
        );
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
    fn resolve_manual_db_nums_includes_db_files() {
        let got = resolve_manual_db_nums(
            vec![20, 0],
            vec![
                "DESI001".to_string(),
                "CATA002".to_string(),
                "DESI001".to_string(),
                "  ".to_string(),
            ],
            |db_file| match db_file {
                "DESI001" => Ok(10),
                "CATA002" => Ok(30),
                other => bail!("unexpected db file {other}"),
            },
        )
        .expect("manual db nums");

        assert_eq!(got, vec![10, 20, 30]);
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

    #[test]
    fn build_parse_config_sets_site_output_root() {
        let now = now_rfc3339();
        let site = ManagedProjectSite {
            site_id: "output-root-test-8124".to_string(),
            site_name: "Output Root Test".to_string(),
            project_name: "Demo".to_string(),
            project_code: 1,
            project_path: "D:/models/Demo".to_string(),
            projects: Vec::new(),
            manual_db_nums: Vec::new(),
            generate_db_nums: Vec::new(),
            parse_db_types: Vec::new(),
            force_rebuild_system_db: false,
            auto_parse_related_dbnums: false,
            gen_model: false,
            gen_mesh: false,
            gen_spatial_tree: false,
            apply_boolean_operation: false,
            mesh_tol_ratio: 0.001,
            export_json: false,
            export_parquet: false,
            pipeline_db_mode: ManagedSiteDbMode::File,
            runtime_db_mode: ManagedSiteDbMode::Ws,
            config_path: String::new(),
            runtime_dir: String::new(),
            db_data_path: String::new(),
            db_port: 8123,
            web_port: 8124,
            viewer_port: None,
            bind_host: "127.0.0.1".to_string(),
            public_base_url: None,
            associated_project: None,
            db_pid: None,
            web_pid: None,
            viewer_pid: None,
            viewer_url: None,
            parse_pid: None,
            status: ManagedSiteStatus::Stopped,
            parse_status: ManagedSiteParseStatus::Pending,
            last_error: None,
            entry_url: None,
            local_entry_url: None,
            public_entry_url: None,
            last_parse_started_at: None,
            last_parse_finished_at: None,
            last_parse_duration_ms: None,
            parse_plan: ManagedSiteParsePlan::default(),
            risk_level: ManagedSiteRiskLevel::Normal,
            risk_reasons: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
        };

        let raw = build_parse_config(&site, "root", "root").expect("parse config");
        let value = toml::from_str::<toml::Value>(&raw).expect("valid toml");
        let output_root = value
            .get("output_root")
            .and_then(|entry| entry.as_str())
            .expect("output_root");

        assert_eq!(
            output_root,
            site_runtime_dir(&site.site_id)
                .join("output")
                .to_string_lossy()
                .replace('\\', "/")
        );
    }

    #[test]
    fn parse_config_does_not_inherit_generation_db_scope() {
        let now = now_rfc3339();
        let site = ManagedProjectSite {
            site_id: "separate-db-scope-test-8124".to_string(),
            site_name: "Separate Db Scope Test".to_string(),
            project_name: "Demo".to_string(),
            project_code: 1,
            project_path: "D:/models/Demo".to_string(),
            projects: Vec::new(),
            manual_db_nums: Vec::new(),
            generate_db_nums: vec![202],
            parse_db_types: Vec::new(),
            force_rebuild_system_db: false,
            auto_parse_related_dbnums: false,
            gen_model: true,
            gen_mesh: false,
            gen_spatial_tree: false,
            apply_boolean_operation: false,
            mesh_tol_ratio: 0.001,
            export_json: false,
            export_parquet: false,
            pipeline_db_mode: ManagedSiteDbMode::File,
            runtime_db_mode: ManagedSiteDbMode::Ws,
            config_path: String::new(),
            runtime_dir: String::new(),
            db_data_path: String::new(),
            db_port: 8123,
            web_port: 8124,
            viewer_port: None,
            bind_host: "127.0.0.1".to_string(),
            public_base_url: None,
            associated_project: None,
            db_pid: None,
            web_pid: None,
            viewer_pid: None,
            viewer_url: None,
            parse_pid: None,
            status: ManagedSiteStatus::Stopped,
            parse_status: ManagedSiteParseStatus::Pending,
            last_error: None,
            entry_url: None,
            local_entry_url: None,
            public_entry_url: None,
            last_parse_started_at: None,
            last_parse_finished_at: None,
            last_parse_duration_ms: None,
            parse_plan: ManagedSiteParsePlan::default(),
            risk_level: ManagedSiteRiskLevel::Normal,
            risk_reasons: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
        };

        let parse_raw = build_parse_config(&site, "root", "root").expect("parse config");
        let generate_raw =
            build_generation_config(&site, "root", "root").expect("generation config");
        let parse_value = toml::from_str::<toml::Value>(&parse_raw).expect("valid parse toml");
        let generate_value =
            toml::from_str::<toml::Value>(&generate_raw).expect("valid generation toml");

        assert_eq!(toml_manual_db_nums(&parse_value), None);
        assert_eq!(toml_manual_db_nums(&generate_value), Some(vec![202]));
    }

    #[test]
    fn site_runtime_config_is_always_ws_mode() {
        let now = now_rfc3339();
        let site = ManagedProjectSite {
            site_id: "runtime-ws-test-8124".to_string(),
            site_name: "Runtime Ws Test".to_string(),
            project_name: "Demo".to_string(),
            project_code: 1,
            project_path: "D:/models/Demo".to_string(),
            projects: Vec::new(),
            manual_db_nums: Vec::new(),
            generate_db_nums: Vec::new(),
            parse_db_types: Vec::new(),
            force_rebuild_system_db: false,
            auto_parse_related_dbnums: false,
            gen_model: false,
            gen_mesh: false,
            gen_spatial_tree: false,
            apply_boolean_operation: false,
            mesh_tol_ratio: 0.001,
            export_json: false,
            export_parquet: false,
            pipeline_db_mode: ManagedSiteDbMode::File,
            runtime_db_mode: ManagedSiteDbMode::File,
            config_path: String::new(),
            runtime_dir: String::new(),
            db_data_path: String::new(),
            db_port: 8123,
            web_port: 8124,
            viewer_port: None,
            bind_host: "127.0.0.1".to_string(),
            public_base_url: None,
            associated_project: None,
            db_pid: None,
            web_pid: None,
            viewer_pid: None,
            viewer_url: None,
            parse_pid: None,
            status: ManagedSiteStatus::Stopped,
            parse_status: ManagedSiteParseStatus::Pending,
            last_error: None,
            entry_url: None,
            local_entry_url: None,
            public_entry_url: None,
            last_parse_started_at: None,
            last_parse_finished_at: None,
            last_parse_duration_ms: None,
            parse_plan: ManagedSiteParsePlan::default(),
            risk_level: ManagedSiteRiskLevel::Normal,
            risk_reasons: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
        };

        let raw = build_site_config(&site, "root", "root").expect("site config");
        let value = toml::from_str::<toml::Value>(&raw).expect("valid toml");
        let surrealdb = value
            .get("surrealdb")
            .and_then(|value| value.as_table())
            .expect("surrealdb table");
        let web_server = value
            .get("web_server")
            .and_then(|value| value.as_table())
            .expect("web_server table");

        assert_eq!(
            surrealdb.get("mode").and_then(|value| value.as_str()),
            Some("ws")
        );
        assert_eq!(
            web_server
                .get("auto_start_surreal")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
    }

    fn toml_manual_db_nums(value: &toml::Value) -> Option<Vec<i64>> {
        value
            .get("manual_db_nums")
            .and_then(|value| value.as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_integer())
                    .collect()
            })
    }
}
