use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
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

use super::models::{
    AdminResourceSummary, CreateManagedSiteRequest, DatabaseConfig, ManagedProjectSite,
    ManagedSiteActivitySummary, ManagedSiteLogStreamSummary, ManagedSiteLogsResponse,
    ManagedSiteParseHealth, ManagedSiteParseHealthStatus, ManagedSiteParseStatus,
    ManagedSiteProcessResource, ManagedSiteResourceMetrics, ManagedSiteRiskLevel,
    ManagedSiteRuntimeStatus, ManagedSiteStatus, UpdateManagedSiteRequest,
};

const DEFAULT_SQLITE_PATH: &str = "deployment_sites.sqlite";
const TABLE_NAME: &str = "managed_project_sites";
const ADMIN_RUNTIME_ROOT: &str = "runtime/admin_sites";
const LOG_LINES_LIMIT: usize = 120;
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
const ADMIN_PARSE_REQUIRED_SYSTEM_DB_TYPES: &[&str] = &["SYST"];

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

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

fn sqlite_path() -> String {
    use config as cfg;

    let cfg_name =
        std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption".to_string());
    let cfg_file = format!("{}.toml", cfg_name);
    if Path::new(&cfg_file).exists() {
        if let Ok(builder) = cfg::Config::builder()
            .add_source(cfg::File::with_name(&cfg_name))
            .build()
        {
            return builder
                .get_string("deployment_sites_sqlite_path")
                .unwrap_or_else(|_| DEFAULT_SQLITE_PATH.to_string());
        }
    }
    DEFAULT_SQLITE_PATH.to_string()
}

fn open_db() -> Result<Connection> {
    let db_path = sqlite_path();
    let conn = Connection::open(&db_path)
        .with_context(|| format!("打开管理员站点数据库失败: {}", db_path))?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
    ensure_schema_with_conn(&conn)?;
    Ok(conn)
}

fn ensure_schema_with_conn(conn: &Connection) -> Result<()> {
    conn.execute_batch(&format!(
        r#"
        CREATE TABLE IF NOT EXISTS {table} (
            site_id TEXT PRIMARY KEY,
            project_name TEXT NOT NULL,
            project_code INTEGER NOT NULL,
            project_path TEXT NOT NULL,
            manual_db_nums TEXT NOT NULL DEFAULT '[]',
            config_path TEXT NOT NULL,
            runtime_dir TEXT NOT NULL,
            db_data_path TEXT NOT NULL,
            db_port INTEGER NOT NULL,
            web_port INTEGER NOT NULL,
            bind_host TEXT NOT NULL,
            db_pid INTEGER,
            web_pid INTEGER,
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
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", TABLE_NAME))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !columns.iter().any(|column| column == "manual_db_nums") {
        conn.execute(
            &format!(
                "ALTER TABLE {table} ADD COLUMN manual_db_nums TEXT NOT NULL DEFAULT '[]'",
                table = TABLE_NAME
            ),
            [],
        )?;
    }
    if !columns
        .iter()
        .any(|column| column == "last_parse_started_at")
    {
        conn.execute(
            &format!(
                "ALTER TABLE {table} ADD COLUMN last_parse_started_at TEXT",
                table = TABLE_NAME
            ),
            [],
        )?;
    }
    if !columns
        .iter()
        .any(|column| column == "last_parse_finished_at")
    {
        conn.execute(
            &format!(
                "ALTER TABLE {table} ADD COLUMN last_parse_finished_at TEXT",
                table = TABLE_NAME
            ),
            [],
        )?;
    }
    if !columns
        .iter()
        .any(|column| column == "last_parse_duration_ms")
    {
        conn.execute(
            &format!(
                "ALTER TABLE {table} ADD COLUMN last_parse_duration_ms INTEGER",
                table = TABLE_NAME
            ),
            [],
        )?;
    }
    if !columns
        .iter()
        .any(|column| column == "public_base_url")
    {
        conn.execute(
            &format!(
                "ALTER TABLE {table} ADD COLUMN public_base_url TEXT",
                table = TABLE_NAME
            ),
            [],
        )?;
    }
    if !columns
        .iter()
        .any(|column| column == "associated_project")
    {
        conn.execute(
            &format!(
                "ALTER TABLE {table} ADD COLUMN associated_project TEXT",
                table = TABLE_NAME
            ),
            [],
        )?;
    }
    Ok(())
}

pub fn ensure_schema() -> Result<()> {
    let conn = open_db()?;
    ensure_schema_with_conn(&conn)
}

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
    format!("{}-{}", slugify(project_name), web_port)
}

fn normalize_host(host: Option<String>) -> String {
    host.map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "127.0.0.1".to_string())
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
        _ => ManagedSiteStatus::Draft,
    }
}

fn parse_status_from_str(raw: &str) -> ManagedSiteParseStatus {
    match raw {
        "Running" => ManagedSiteParseStatus::Running,
        "Parsed" => ManagedSiteParseStatus::Parsed,
        "Failed" => ManagedSiteParseStatus::Failed,
        _ => ManagedSiteParseStatus::Pending,
    }
}

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

fn split_project_root(project_name: &str, raw_path: &str) -> (String, Vec<String>, Vec<String>) {
    let path = PathBuf::from(raw_path);
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(project_name)
        .to_string();
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

fn find_db_file_name_for_dbnum(root: &Path, target_dbnum: u32) -> Result<Option<String>> {
    for entry in fs::read_dir(root)
        .with_context(|| format!("读取目录失败: {}", root.display()))?
        .flatten()
    {
        let path = entry.path();
        if path.is_dir() {
            if let Some(file_name) = find_db_file_name_for_dbnum(&path, target_dbnum)? {
                return Ok(Some(file_name));
            }
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let mut file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(_) => continue,
        };
        let mut buf = [0u8; 60];
        if file.read_exact(&mut buf).is_err() {
            continue;
        }
        let db_info = parse_file_basic_info(&buf);
        if db_info.dbnum == target_dbnum {
            if let Some(file_name) = path.file_name().and_then(|value| value.to_str()) {
                return Ok(Some(file_name.to_string()));
            }
        }
    }
    Ok(None)
}

fn collect_db_file_names_for_types(
    root: &Path,
    target_types: &[&str],
    file_names: &mut Vec<String>,
) -> Result<()> {
    for entry in fs::read_dir(root)
        .with_context(|| format!("读取目录失败: {}", root.display()))?
        .flatten()
    {
        let path = entry.path();
        if path.is_dir() {
            collect_db_file_names_for_types(&path, target_types, file_names)?;
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if file_name.contains('.') {
            continue;
        }
        let mut file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(_) => continue,
        };
        let mut buf = [0u8; 60];
        if file.read_exact(&mut buf).is_err() {
            continue;
        }
        let db_info = parse_file_basic_info(&buf);
        if target_types.contains(&db_info.db_type.as_str()) {
            file_names.push(file_name.to_string());
        }
    }
    Ok(())
}

fn resolve_included_db_files(site: &ManagedProjectSite) -> Result<Vec<String>> {
    if site.manual_db_nums.is_empty() {
        return Ok(Vec::new());
    }

    let project_root = project_dir_candidates(&site.project_name, &site.project_path)
        .into_iter()
        .find(|path| path.exists())
        .ok_or_else(|| anyhow!("项目路径不存在: {}", site.project_path))?;

    let mut file_names = Vec::new();
    collect_db_file_names_for_types(
        &project_root,
        ADMIN_PARSE_REQUIRED_SYSTEM_DB_TYPES,
        &mut file_names,
    )?;
    for dbnum in &site.manual_db_nums {
        let file_name = find_db_file_name_for_dbnum(&project_root, *dbnum)?
            .ok_or_else(|| anyhow!("项目路径下未找到 dbnum={} 对应的 db 文件", dbnum))?;
        file_names.push(file_name);
    }
    file_names.sort();
    file_names.dedup();
    Ok(file_names)
}

fn set_toml_string(table: &mut toml::value::Table, key: &str, value: impl Into<String>) {
    table.insert(key.to_string(), toml::Value::String(value.into()));
}

fn set_toml_integer(table: &mut toml::value::Table, key: &str, value: i64) {
    table.insert(key.to_string(), toml::Value::Integer(value));
}

fn set_toml_bool(table: &mut toml::value::Table, key: &str, value: bool) {
    table.insert(key.to_string(), toml::Value::Boolean(value));
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

    let runtime_cfg = DatabaseConfig {
        project_name: site.project_name.clone(),
        project_path: site.project_path.clone(),
        project_code: site.project_code,
        manual_db_nums: site.manual_db_nums.clone(),
        surreal_ns: site.project_code,
        db_ip: "127.0.0.1".to_string(),
        db_port: site.db_port.to_string(),
        db_user: db_user.to_string(),
        db_password: db_password.to_string(),
        ..DatabaseConfig::from_db_option(&aios_core::get_db_option())
    };
    let db_option = runtime_cfg.to_runtime_db_option();
    let (project_root, included_projects, project_dirs) =
        split_project_root(&site.project_name, &site.project_path);

    set_toml_string(table, "project_name", site.project_name.clone());
    set_toml_string(table, "project_path", project_root);
    set_toml_string(table, "project_code", site.project_code.to_string());
    set_toml_string(table, "surreal_ns", site.project_code.to_string());
    set_toml_string(table, "mdb_name", db_option.mdb_name.clone());
    set_toml_string(table, "module", db_option.module.clone());
    set_toml_string(table, "surreal_ip", "127.0.0.1");
    set_toml_integer(table, "surreal_port", site.db_port as i64);
    set_toml_string(table, "surreal_user", db_user.to_string());
    set_toml_string(table, "surreal_password", db_password.to_string());
    table.remove("v_ip");
    table.remove("v_port");
    table.remove("v_user");
    table.remove("v_password");
    set_toml_array(table, "included_projects", included_projects);
    set_toml_array(table, "project_dirs", project_dirs);
    set_toml_integer_array(table, "manual_db_nums", runtime_cfg.manual_db_nums.clone());

    let web_server = ensure_table(table, "web_server");
    set_toml_integer(web_server, "port", site.web_port as i64);
    set_toml_string(web_server, "bind_host", site.bind_host.clone());
    set_toml_string(web_server, "site_id", site.site_id.clone());
    set_toml_string(web_server, "site_name", site.project_name.clone());
    set_toml_string(web_server, "region", "admin");
    let local_url = format!("http://127.0.0.1:{}", site.web_port);
    let public_url = site
        .public_base_url
        .as_ref()
        .map(|u| u.trim_end_matches('/').to_string())
        .or_else(|| {
            let h = site.bind_host.trim();
            if !h.is_empty() && h != "0.0.0.0" && h != "127.0.0.1" && h != "localhost" {
                Some(format!("http://{}:{}", h, site.web_port))
            } else {
                None
            }
        });
    let effective_url = public_url.as_deref().unwrap_or(&local_url);
    set_toml_string(web_server, "frontend_url", effective_url);
    set_toml_string(web_server, "public_base_url", effective_url);
    set_toml_string(web_server, "backend_url", local_url);
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
    set_toml_string(surrealdb, "path", site.db_data_path.clone());

    let surrealkv = ensure_table(table, "surrealkv");
    set_toml_bool(surrealkv, "enabled", false);
    set_toml_string(surrealkv, "path", format!("{}.kv", site.db_data_path));

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

fn write_site_files(site: &ManagedProjectSite, db_user: &str, db_password: &str) -> Result<()> {
    ensure_runtime_dirs(&site.site_id)?;
    let content = build_site_config(site, db_user, db_password)?;
    fs::write(&site.config_path, content)?;
    let parse_content = build_parse_config(site, db_user, db_password)?;
    fs::write(parse_config_path(&site.site_id), parse_content)?;
    fs::write(
        metadata_path(&site.site_id),
        serde_json::to_vec_pretty(&json!({
            "site_id": site.site_id,
            "project_name": site.project_name,
            "project_code": site.project_code,
            "project_path": site.project_path,
            "manual_db_nums": site.manual_db_nums,
            "db_port": site.db_port,
            "web_port": site.web_port,
            "entry_url": site.entry_url,
            "updated_at": site.updated_at,
        }))?,
    )?;
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
        config_path: row.get("config_path")?,
        runtime_dir: row.get("runtime_dir")?,
        db_data_path: row.get("db_data_path")?,
        db_port: row.get::<_, i64>("db_port")? as u16,
        web_port,
        bind_host,
        public_base_url,
        associated_project,
        db_pid: row
            .get::<_, Option<i64>>("db_pid")?
            .map(|value| value as u32),
        web_pid: row
            .get::<_, Option<i64>>("web_pid")?
            .map(|value| value as u32),
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
        risk_level: ManagedSiteRiskLevel::Normal,
        risk_reasons: Vec::new(),
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

fn port_in_use(host: &str, port: u16) -> bool {
    let host = if host == "0.0.0.0" { "127.0.0.1" } else { host };
    let addr = format!("{}:{}", host, port);
    match addr.to_socket_addrs() {
        Ok(mut addrs) => addrs
            .any(|socket| TcpStream::connect_timeout(&socket, Duration::from_millis(300)).is_ok()),
        Err(_) => false,
    }
}

fn refresh_site(site: &mut ManagedProjectSite) {
    let db_running = pid_running(site.db_pid) || port_in_use("127.0.0.1", site.db_port);
    let web_running = pid_running(site.web_pid) || port_in_use("127.0.0.1", site.web_port);
    let parse_running = pid_running(site.parse_pid);

    if parse_running {
        site.parse_status = ManagedSiteParseStatus::Running;
    }
    if web_running {
        site.status = ManagedSiteStatus::Running;
        site.entry_url = Some(format!("http://127.0.0.1:{}", site.web_port));
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
}

fn site_db_running(site: &ManagedProjectSite) -> bool {
    pid_running(site.db_pid) || port_in_use("127.0.0.1", site.db_port)
}

fn site_web_running(site: &ManagedProjectSite) -> bool {
    pid_running(site.web_pid) || port_in_use("127.0.0.1", site.web_port)
}

fn site_parse_running(site: &ManagedProjectSite) -> bool {
    pid_running(site.parse_pid)
}

fn site_has_active_processes(site: &ManagedProjectSite) -> bool {
    site_db_running(site) || site_web_running(site) || site_parse_running(site)
}

fn record_site_error(
    site_id: &str,
    message: impl Into<String>,
    status: Option<ManagedSiteStatus>,
    parse_status: Option<ManagedSiteParseStatus>,
) {
    let _ = update_runtime(site_id, RuntimeUpdate {
        status,
        parse_status,
        last_error: Some(Some(message.into())),
        ..Default::default()
    });
}

fn assert_port_available(
    conn: &Connection,
    exclude_site_id: Option<&str>,
    db_port: u16,
    web_port: u16,
) -> Result<()> {
    let sql = format!(
        "SELECT site_id, db_port, web_port FROM {table} WHERE (?1 IS NULL OR site_id != ?1)",
        table = TABLE_NAME
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([exclude_site_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)? as u16,
            row.get::<_, i64>(2)? as u16,
        ))
    })?;
    for row in rows {
        let (site_id, existing_db_port, existing_web_port) = row?;
        if existing_db_port == db_port {
            bail!("数据库端口 {} 已被站点 {} 使用", db_port, site_id);
        }
        if existing_web_port == web_port {
            bail!("站点端口 {} 已被站点 {} 使用", web_port, site_id);
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

fn load_site_with_conn(conn: &Connection, site_id: &str) -> Result<Option<ManagedProjectSite>> {
    let sql = format!(
        "SELECT * FROM {table} WHERE site_id = ?1",
        table = TABLE_NAME
    );
    let site = conn.query_row(&sql, [site_id], row_to_site).optional()?;
    Ok(site)
}

pub fn get_site(site_id: &str) -> Result<Option<ManagedProjectSite>> {
    let conn = open_db()?;
    let mut site = load_site_with_conn(&conn, site_id)?;
    if let Some(item) = site.as_mut() {
        refresh_site(item);
        annotate_site_risk(item);
    }
    Ok(site)
}

pub fn list_sites() -> Result<Vec<ManagedProjectSite>> {
    let conn = open_db()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT * FROM {table} ORDER BY updated_at DESC",
        table = TABLE_NAME
    ))?;
    let rows = stmt.query_map([], row_to_site)?;
    let mut items = Vec::new();
    for row in rows {
        let mut site = row?;
        refresh_site(&mut site);
        items.push(site);
    }
    annotate_sites_risks(&mut items);
    Ok(items)
}

fn persist_site(
    conn: &Connection,
    site: &ManagedProjectSite,
    db_user: &str,
    db_password: &str,
) -> Result<()> {
    conn.execute(
        &format!(
            "INSERT OR REPLACE INTO {table} (
                site_id, project_name, project_code, project_path, config_path, runtime_dir,
                manual_db_nums, db_data_path, db_port, web_port, bind_host, public_base_url,
                associated_project,
                db_pid, web_pid, parse_pid,
                status, parse_status, last_error, entry_url, db_user, db_password,
                last_parse_started_at, last_parse_finished_at, last_parse_duration_ms,
                created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27)",
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
            &site.db_data_path,
            site.db_port as i64,
            site.web_port as i64,
            &site.bind_host,
            &site.public_base_url,
            &site.associated_project,
            site.db_pid.map(|value| value as i64),
            site.web_pid.map(|value| value as i64),
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

    let conn = open_db()?;
    assert_port_available(&conn, None, req.db_port, req.web_port)?;

    let site_id = infer_site_id(&req.project_name, req.web_port);
    let created_at = now_rfc3339();
    let bind_host = normalize_host(req.bind_host);
    let public_base_url = req
        .public_base_url
        .filter(|v| !v.trim().is_empty());
    let associated_project = req
        .associated_project
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.trim().to_string());
    let (local_entry_url, public_entry_url, entry_url) =
        derive_entry_urls(req.web_port, &bind_host, &public_base_url);
    let site = ManagedProjectSite {
        site_id: site_id.clone(),
        project_name: req.project_name.trim().to_string(),
        project_code: req.project_code,
        project_path: req.project_path.trim().to_string(),
        manual_db_nums: normalize_manual_db_nums(req.manual_db_nums),
        config_path: config_path(&site_id).to_string_lossy().to_string(),
        runtime_dir: site_runtime_dir(&site_id).to_string_lossy().to_string(),
        db_data_path: db_data_path(&site_id).to_string_lossy().to_string(),
        db_port: req.db_port,
        web_port: req.web_port,
        bind_host,
        public_base_url,
        associated_project,
        db_pid: None,
        web_pid: None,
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
        risk_level: ManagedSiteRiskLevel::Normal,
        risk_reasons: Vec::new(),
        created_at: created_at.clone(),
        updated_at: created_at,
    };
    let db_user = require_db_user(req.db_user)?;
    let db_password = require_db_password(req.db_password)?;
    write_site_files(&site, &db_user, &db_password)?;
    persist_site(&conn, &site, &db_user, &db_password)?;
    Ok(site)
}

fn load_credentials(conn: &Connection, site_id: &str) -> Result<(String, String)> {
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

pub fn update_site(site_id: &str, req: UpdateManagedSiteRequest) -> Result<ManagedProjectSite> {
    let conn = open_db()?;
    let mut site = load_site_with_conn(&conn, site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
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
        site.project_path = value.trim().to_string();
    }
    if let Some(value) = req.project_code.filter(|value| *value > 0) {
        site.project_code = value;
    }
    if let Some(value) = req.manual_db_nums {
        site.manual_db_nums = normalize_manual_db_nums(value);
    }
    if let Some(value) = req.bind_host.filter(|value| !value.trim().is_empty()) {
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
    site.parse_pid = None;
    site.last_error = None;

    assert_port_available(&conn, Some(site_id), site.db_port, site.web_port)?;
    let (stored_db_user, stored_db_password) = load_credentials(&conn, site_id)?;
    if matches!(req.db_user.as_ref(), Some(value) if value.trim().is_empty()) {
        bail!("数据库用户名不能为空");
    }
    if matches!(req.db_password.as_ref(), Some(value) if value.trim().is_empty()) {
        bail!("数据库密码不能为空");
    }
    let db_user = normalize_optional_db_user(req.db_user).unwrap_or(stored_db_user);
    let db_password = normalize_optional_db_password(req.db_password).unwrap_or(stored_db_password);
    write_site_files(&site, &db_user, &db_password)?;
    persist_site(&conn, &site, &db_user, &db_password)?;
    Ok(site)
}

#[derive(Default)]
pub struct RuntimeUpdate {
    pub status: Option<ManagedSiteStatus>,
    pub parse_status: Option<ManagedSiteParseStatus>,
    pub db_pid: Option<Option<u32>>,
    pub web_pid: Option<Option<u32>>,
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
        parse_pid,
        last_error,
        entry_url,
        last_parse_started_at,
        last_parse_finished_at,
        last_parse_duration_ms,
    } = update;
    let conn = open_db()?;
    let mut site = load_site_with_conn(&conn, site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
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
    let (db_user, db_password) = load_credentials(&conn, site_id)?;
    persist_site(&conn, &site, &db_user, &db_password)
}

fn repo_root() -> Result<PathBuf> {
    std::env::current_dir().context("获取当前工作目录失败")
}

fn current_exe_path() -> Result<PathBuf> {
    std::env::current_exe().context("获取当前 web_server 可执行文件失败")
}

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

fn path_size_bytes(path: &Path) -> u64 {
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
        .map(|entry| path_size_bytes(&entry.path()))
        .sum()
}

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
    parse_running: bool,
    system: &System,
    cpu_ready: bool,
) -> ManagedSiteResourceMetrics {
    let runtime_dir = PathBuf::from(&site.runtime_dir);
    let data_dir = site_data_dir(site);

    ManagedSiteResourceMetrics {
        db_process: build_process_resource(site.db_pid, db_running, system, cpu_ready),
        web_process: build_process_resource(site.web_pid, web_running, system, cpu_ready),
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
    parse_running: bool,
) -> ManagedSiteResourceMetrics {
    let tracked_pids = [site.db_pid, site.web_pid, site.parse_pid]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    with_resource_sampler(&tracked_pids, |cpu_ready, system| {
        build_site_resource_metrics(site, db_running, web_running, parse_running, system, cpu_ready)
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
                detail: Some(format!("最近一次解析耗时 {}", format_duration_label(duration_ms))),
            };
        }
        if duration_ms >= PARSE_WARNING_DURATION_MS {
            return ManagedSiteParseHealth {
                status: ManagedSiteParseHealthStatus::Warning,
                label: "解析耗时偏长".to_string(),
                detail: Some(format!("最近一次解析耗时 {}", format_duration_label(duration_ms))),
            };
        }
        return ManagedSiteParseHealth {
            status: ManagedSiteParseHealthStatus::Normal,
            label: "解析正常".to_string(),
            detail: Some(format!("最近一次解析耗时 {}", format_duration_label(duration_ms))),
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
    apply_process_risk("Web", &resources.web_process, &mut risk_level, &mut warnings);
    apply_process_risk("Parse", &resources.parse_process, &mut risk_level, &mut warnings);

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
        && matches!(site.status, ManagedSiteStatus::Starting | ManagedSiteStatus::Running)
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
    let parse_running = pid_running(site.parse_pid);
    let resources = collect_site_resource_metrics(site, db_running, web_running, parse_running);
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
                pid_running(site.parse_pid),
            )
        })
        .collect::<Vec<_>>();
    let tracked_pids = sites
        .iter()
        .flat_map(|site| [site.db_pid, site.web_pid, site.parse_pid])
        .flatten()
        .collect::<Vec<_>>();

    with_resource_sampler(&tracked_pids, |cpu_ready, system| {
        for (site, (db_running, web_running, parse_running)) in
            sites.iter_mut().zip(runtime_states.into_iter())
        {
            let resources = build_site_resource_metrics(
                site,
                db_running,
                web_running,
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
    let conn = open_db()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT * FROM {table} ORDER BY updated_at DESC",
        table = TABLE_NAME
    ))?;
    let rows = stmt.query_map([], row_to_site)?;
    let mut sites = Vec::new();
    for row in rows {
        sites.push(row?);
    }

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

#[cfg(unix)]
fn pid_running(pid: Option<u32>) -> bool {
    let Some(pid) = pid else {
        return false;
    };
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
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

async fn process_ids_on_port(port: u16) -> Result<Vec<u32>> {
    #[cfg(unix)]
    {
        let output = Command::new("sh")
            .arg("-c")
            .arg(format!("lsof -ti tcp:{} 2>/dev/null || true", port))
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
        let output = Command::new("cmd")
            .args(["/C", &format!("netstat -ano | findstr :{} | findstr LISTENING", port)])
            .output()
            .await
            .context("读取端口进程失败")?;
        let ids = String::from_utf8_lossy(&output.stdout)
            .lines()
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
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("lsof -ti tcp:{} 2>/dev/null || true", port))
            .output();
        match output {
            Ok(out) => String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|line| line.trim().parse::<u32>().ok())
                .collect(),
            Err(_) => Vec::new(),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = port;
        Vec::new()
    }
}

fn aios_database_binary() -> Result<Option<PathBuf>> {
    let current = current_exe_path()?;
    let parent = current
        .parent()
        .ok_or_else(|| anyhow!("无法定位当前二进制目录"))?;
    let sibling = parent.join("aios-database");
    if sibling.exists() {
        Ok(Some(sibling))
    } else {
        Ok(None)
    }
}

fn should_run_aios_database_from_source(repo: &Path) -> bool {
    let current = match current_exe_path() {
        Ok(path) => path,
        Err(_) => return false,
    };
    repo.join("Cargo.toml").exists()
        && current
            .components()
            .any(|component| component.as_os_str() == "target")
}

fn open_log_file(path: &Path) -> Result<(std::fs::File, std::fs::File)> {
    let stdout = OpenOptions::new().create(true).append(true).open(path)?;
    let stderr = OpenOptions::new().create(true).append(true).open(path)?;
    Ok((stdout, stderr))
}

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
    for _ in 0..attempts {
        if let Ok(response) = reqwest::get(url).await {
            if response.status().is_success() {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
    false
}

async fn spawn_parse_process(site_id: String) -> Result<()> {
    let conn = open_db()?;
    let site = load_site_with_conn(&conn, &site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
    let (db_user, db_password) = load_credentials(&conn, &site.site_id)?;
    write_site_files(&site, &db_user, &db_password)?;
    let config_path = parse_config_path(&site.site_id);
    let config_no_ext = config_path
        .to_string_lossy()
        .to_string()
        .strip_suffix(".toml")
        .map(|value| value.to_string())
        .unwrap_or_else(|| config_path.to_string_lossy().to_string());
    let single_dbnum = site
        .manual_db_nums
        .first()
        .copied()
        .filter(|_| site.manual_db_nums.len() == 1);
    let (stdout, stderr) = open_log_file(&parse_log_path(&site.site_id))?;
    let repo = repo_root()?;
    let mut command = if let Some(binary) = aios_database_binary()? {
        let mut cmd = Command::new(binary);
        cmd.arg("-c").arg(config_no_ext);
        if let Some(dbnum) = single_dbnum {
            cmd.arg("--dbnum").arg(dbnum.to_string());
        }
        cmd
    } else if should_run_aios_database_from_source(&repo) {
        let mut cmd = Command::new("cargo");
        cmd.arg("run")
            .arg("--bin")
            .arg("aios-database")
            .arg("--")
            .arg("-c")
            .arg(config_no_ext);
        if let Some(dbnum) = single_dbnum {
            cmd.arg("--dbnum").arg(dbnum.to_string());
        }
        cmd
    } else {
        let mut cmd = Command::new("cargo");
        cmd.arg("run")
            .arg("--bin")
            .arg("aios-database")
            .arg("--")
            .arg("-c")
            .arg(config_no_ext);
        if let Some(dbnum) = single_dbnum {
            cmd.arg("--dbnum").arg(dbnum.to_string());
        }
        cmd
    };
    command
        .current_dir(repo)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));

    let parse_started_at = now_rfc3339();
    let parse_started_instant = Instant::now();
    let mut child = command.spawn().context("启动解析进程失败")?;
    let pid = child.id();
    update_runtime(&site.site_id, RuntimeUpdate {
        status: Some(ManagedSiteStatus::Draft),
        parse_status: Some(ManagedSiteParseStatus::Running),
        parse_pid: Some(pid),
        last_error: Some(None),
        last_parse_started_at: Some(Some(parse_started_at)),
        last_parse_finished_at: Some(None),
        last_parse_duration_ms: Some(None),
        ..Default::default()
    })?;

    let exit = child.wait().await.context("等待解析进程失败")?;
    let parse_finished_at = now_rfc3339();
    let parse_duration_ms = parse_started_instant.elapsed().as_millis() as u64;
    if exit.success() {
        update_runtime(&site.site_id, RuntimeUpdate {
            status: Some(ManagedSiteStatus::Parsed),
            parse_status: Some(ManagedSiteParseStatus::Parsed),
            parse_pid: Some(None),
            last_error: Some(None),
            last_parse_finished_at: Some(Some(parse_finished_at)),
            last_parse_duration_ms: Some(Some(parse_duration_ms)),
            ..Default::default()
        })?;
    } else {
        update_runtime(&site.site_id, RuntimeUpdate {
            status: Some(ManagedSiteStatus::Failed),
            parse_status: Some(ManagedSiteParseStatus::Failed),
            parse_pid: Some(None),
            last_error: Some(Some(format!("解析失败，退出码: {:?}", exit.code()))),
            last_parse_finished_at: Some(Some(parse_finished_at)),
            last_parse_duration_ms: Some(Some(parse_duration_ms)),
            ..Default::default()
        })?;
    }
    Ok(())
}

async fn spawn_db_process(site: &ManagedProjectSite) -> Result<u32> {
    let conn = open_db()?;
    let (db_user, db_password) = load_credentials(&conn, &site.site_id)?;
    let (stdout, stderr) = open_log_file(&db_log_path(&site.site_id))?;
    let mut command = Command::new("surreal");
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
    let child = command.spawn().context("启动项目 web_server 失败")?;
    Ok(child.id().unwrap_or_default())
}

async fn ensure_site_db_started(
    site: &ManagedProjectSite,
    status: ManagedSiteStatus,
) -> Result<Option<u32>> {
    if port_in_use("127.0.0.1", site.db_port) {
        if pid_running(site.db_pid) {
            return Ok(None);
        }
        bail!("数据库端口 {} 已被占用", site.db_port);
    }

    let db_pid = spawn_db_process(site).await?;
    update_runtime(&site.site_id, RuntimeUpdate {
        status: Some(status),
        db_pid: Some(Some(db_pid)),
        last_error: Some(None),
        ..Default::default()
    })?;
    if !wait_for_port(site.db_port, 30, 500).await {
        let _ = kill_pid(db_pid).await;
        bail!("SurrealDB 未在端口 {} 成功启动", site.db_port);
    }
    Ok(Some(db_pid))
}

async fn run_parse_pipeline(site_id: String) -> Result<()> {
    let site = get_site(&site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
    let started_db_pid = ensure_site_db_started(&site, site.status.clone()).await?;
    let parse_result = spawn_parse_process(site_id.clone()).await;

    if let Some(db_pid) = started_db_pid {
        let _ = kill_pid(db_pid).await;
        let _ = update_runtime(&site_id, RuntimeUpdate {
            db_pid: Some(None),
            ..Default::default()
        });
    }

    parse_result
}

async fn run_start_pipeline(site_id: String) -> Result<()> {
    let site = get_site(&site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
    let conn = open_db()?;
    assert_port_available(&conn, Some(&site_id), site.db_port, site.web_port)?;
    drop(conn);

    if site.parse_status == ManagedSiteParseStatus::Running {
        bail!("解析任务仍在运行，请稍后再启动站点");
    }
    update_runtime(&site_id, RuntimeUpdate {
        status: Some(ManagedSiteStatus::Starting),
        last_error: Some(None),
        ..Default::default()
    })?;

    let site = get_site(&site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
    let db_pid = ensure_site_db_started(&site, ManagedSiteStatus::Starting).await?;

    let site = get_site(&site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
    if site.parse_status != ManagedSiteParseStatus::Parsed {
        if let Err(err) = spawn_parse_process(site_id.clone()).await {
            if let Some(pid) = db_pid {
                let _ = kill_pid(pid).await;
            }
            let _ = update_runtime(&site_id, RuntimeUpdate {
                status: Some(ManagedSiteStatus::Failed),
                parse_status: Some(ManagedSiteParseStatus::Failed),
                db_pid: Some(None),
                last_error: Some(Some(format!("启动解析失败: {err}"))),
                ..Default::default()
            });
            return Err(err);
        }
    }

    let site = get_site(&site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
    let web_pid = spawn_web_process(&site).await?;
    update_runtime(&site_id, RuntimeUpdate {
        status: Some(ManagedSiteStatus::Starting),
        web_pid: Some(Some(web_pid)),
        last_error: Some(None),
        entry_url: Some(Some(format!("http://127.0.0.1:{}", site.web_port))),
        ..Default::default()
    })?;
    let status_url = format!("http://127.0.0.1:{}/api/status", site.web_port);
    if !wait_for_http_ok(&status_url, 40, 500).await {
        let _ = kill_pid(web_pid).await;
        if let Some(pid) = db_pid {
            let _ = kill_pid(pid).await;
            let _ = update_runtime(&site_id, RuntimeUpdate {
                db_pid: Some(None),
                ..Default::default()
            });
        }
        bail!("项目站点未在 {} 启动成功", status_url);
    }

    update_runtime(&site_id, RuntimeUpdate {
        status: Some(ManagedSiteStatus::Running),
        parse_status: Some(ManagedSiteParseStatus::Parsed),
        parse_pid: Some(None),
        last_error: Some(None),
        entry_url: Some(Some(format!("http://127.0.0.1:{}", site.web_port))),
        ..Default::default()
    })?;
    Ok(())
}

pub async fn start_site(site_id: String) -> Result<()> {
    let site = get_site(&site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
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
    update_runtime(&site_id, RuntimeUpdate {
        status: Some(ManagedSiteStatus::Starting),
        last_error: Some(None),
        ..Default::default()
    })?;
    tokio::spawn(async move {
        if let Err(err) = run_start_pipeline(site_id.clone()).await {
            let _ = update_runtime(&site_id, RuntimeUpdate {
                status: Some(ManagedSiteStatus::Failed),
                parse_pid: Some(None),
                last_error: Some(Some(err.to_string())),
                ..Default::default()
            });
        }
    });
    Ok(())
}

pub async fn parse_site(site_id: String) -> Result<()> {
    let site = get_site(&site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
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
            let _ = update_runtime(&site_id, RuntimeUpdate {
                status: Some(ManagedSiteStatus::Failed),
                parse_status: Some(ManagedSiteParseStatus::Failed),
                parse_pid: Some(None),
                last_error: Some(Some(err.to_string())),
                ..Default::default()
            });
        }
    });
    Ok(())
}

async fn kill_pid(pid: u32) -> Result<()> {
    #[cfg(unix)]
    {
        let _ = Command::new("sh")
            .arg("-c")
            .arg(format!("kill -TERM {} >/dev/null 2>&1 || true", pid))
            .status()
            .await;
        tokio::time::sleep(Duration::from_millis(600)).await;
        if pid_running(Some(pid)) {
            let _ = Command::new("sh")
                .arg("-c")
                .arg(format!("kill -KILL {} >/dev/null 2>&1 || true", pid))
                .status()
                .await;
        }
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string()])
            .output()
            .await;
        tokio::time::sleep(Duration::from_millis(600)).await;
        if pid_running(Some(pid)) {
            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/F"])
                .output()
                .await;
        }
    }
    Ok(())
}

pub async fn stop_site(site_id: &str) -> Result<StopSiteResult> {
    let site = get_site(site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
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
    update_runtime(site_id, RuntimeUpdate {
        status: Some(ManagedSiteStatus::Stopping),
        last_error: Some(None),
        ..Default::default()
    })?;
    if let Some(pid) = site.web_pid {
        kill_pid(pid).await?;
    }
    if let Some(pid) = site.db_pid {
        kill_pid(pid).await?;
    }
    if let Some(pid) = site.parse_pid {
        kill_pid(pid).await?;
    }

    let web_conflict_pids = process_ids_on_port(site.web_port).await.unwrap_or_default();
    let db_conflict_pids = process_ids_on_port(site.db_port).await.unwrap_or_default();
    let has_conflict = !web_conflict_pids.is_empty() || !db_conflict_pids.is_empty();

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
        let conflict_msg = reasons.join("; ");
        update_runtime(site_id, RuntimeUpdate {
            status: Some(ManagedSiteStatus::Failed),
            db_pid: Some(None),
            web_pid: Some(None),
            parse_pid: Some(None),
            last_error: Some(Some(format!("端口冲突: {}", conflict_msg))),
            ..Default::default()
        })?;
        let updated = get_site(site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
        return Ok(StopSiteResult {
            site: updated,
            conflict: true,
            web_conflict_pids,
            db_conflict_pids,
        });
    }

    update_runtime(site_id, RuntimeUpdate {
        status: Some(ManagedSiteStatus::Stopped),
        parse_status: Some(if site.parse_status == ManagedSiteParseStatus::Running {
            ManagedSiteParseStatus::Pending
        } else {
            site.parse_status.clone()
        }),
        db_pid: Some(None),
        web_pid: Some(None),
        parse_pid: Some(None),
        last_error: Some(None),
        ..Default::default()
    })?;
    let updated = get_site(site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
    Ok(StopSiteResult {
        site: updated,
        conflict: false,
        web_conflict_pids: Vec::new(),
        db_conflict_pids: Vec::new(),
    })
}

pub struct StopSiteResult {
    pub site: ManagedProjectSite,
    pub conflict: bool,
    pub web_conflict_pids: Vec<u32>,
    pub db_conflict_pids: Vec<u32>,
}

pub fn delete_site(site_id: &str) -> Result<bool> {
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
    let conn = open_db()?;
    let changed = conn.execute(
        &format!("DELETE FROM {table} WHERE site_id = ?1", table = TABLE_NAME),
        [site_id],
    )?;
    let runtime = site_runtime_dir(site_id);
    if runtime.exists() {
        let _ = fs::remove_dir_all(runtime);
    }
    Ok(changed > 0)
}

pub fn runtime_status(site_id: &str) -> Result<ManagedSiteRuntimeStatus> {
    let mut site = get_site(site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
    refresh_site(&mut site);
    let db_running = pid_running(site.db_pid) || port_in_use("127.0.0.1", site.db_port);
    let web_running = pid_running(site.web_pid) || port_in_use("127.0.0.1", site.web_port);
    let parse_running = pid_running(site.parse_pid);
    let resources = collect_site_resource_metrics(&site, db_running, web_running, parse_running);
    let snapshots = collect_log_snapshots(site_id);
    let parse_snapshot = snapshots.iter().find(|snapshot| snapshot.key == "parse");
    let db_snapshot = snapshots.iter().find(|snapshot| snapshot.key == "db");
    let web_snapshot = snapshots.iter().find(|snapshot| snapshot.key == "web");
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
        parse_snapshot.and_then(|snapshot| snapshot.last_key_log.clone()),
        db_snapshot.and_then(|snapshot| snapshot.last_key_log.clone()),
        web_snapshot.and_then(|snapshot| snapshot.last_key_log.clone()),
    );

    let (risk_level, mut warnings, parse_health) = evaluate_site_risk(&site, &resources);

    let managed_db_pids: Vec<u32> = site.db_pid.into_iter().collect();
    let managed_web_pids: Vec<u32> = site.web_pid.into_iter().collect();
    let db_port_pids = collect_port_pids_sync(site.db_port);
    let web_port_pids = collect_port_pids_sync(site.web_port);
    let db_conflict_pids: Vec<u32> = db_port_pids
        .into_iter()
        .filter(|pid| !managed_db_pids.contains(pid))
        .collect();
    let web_conflict_pids: Vec<u32> = web_port_pids
        .into_iter()
        .filter(|pid| !managed_web_pids.contains(pid))
        .collect();
    let db_port_conflict = !db_conflict_pids.is_empty();
    let web_port_conflict = !web_conflict_pids.is_empty();
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

    Ok(ManagedSiteRuntimeStatus {
        site_id: site.site_id,
        status: site.status,
        parse_status: site.parse_status,
        current_stage,
        current_stage_label,
        current_stage_detail,
        db_running,
        web_running,
        parse_running,
        db_pid: site.db_pid,
        web_pid: site.web_pid,
        parse_pid: site.parse_pid,
        db_port: site.db_port,
        web_port: site.web_port,
        entry_url: site.entry_url,
        local_entry_url: site.local_entry_url,
        public_entry_url: site.public_entry_url,
        db_port_conflict,
        web_port_conflict,
        db_conflict_pids,
        web_conflict_pids,
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

fn strip_ansi_codes(line: &str) -> String {
    let mut cleaned = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if matches!(chars.peek(), Some('[')) {
                let _ = chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            continue;
        }
        cleaned.push(ch);
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
        log_snapshot("db", "数据库日志", db_log_path(site_id)),
        log_snapshot("web", "站点日志", web_log_path(site_id)),
    ]
}

fn current_stage(
    site: &ManagedProjectSite,
    db_running: bool,
    web_running: bool,
    parse_running: bool,
    parse_detail: Option<String>,
    db_detail: Option<String>,
    web_detail: Option<String>,
) -> (String, String, Option<String>) {
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
    if matches!(site.status, ManagedSiteStatus::Failed)
        || site.parse_status == ManagedSiteParseStatus::Failed
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

pub fn logs(site_id: &str) -> Result<ManagedSiteLogsResponse> {
    let site = get_site(site_id)?.ok_or_else(|| anyhow!("站点不存在"))?;
    let snapshots = collect_log_snapshots(site_id);
    let parse_log = snapshots
        .iter()
        .find(|snapshot| snapshot.key == "parse")
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

    Ok(ManagedSiteLogsResponse {
        site_id: site.site_id,
        parse_log,
        db_log,
        web_log,
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
