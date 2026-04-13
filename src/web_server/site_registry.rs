use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::models::{
    DatabaseConfig, DeploymentSite, DeploymentSiteQuery, DeploymentSiteStatus, E3dProjectInfo,
};

const DEFAULT_SQLITE_PATH: &str = "deployment_sites.sqlite";
pub const DEFAULT_REGISTRY_TTL_SECS: u64 = 120;
pub const DEFAULT_HEARTBEAT_INTERVAL_SECS: u64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebServerRuntimeConfig {
    pub bind_host: String,
    pub bind_port: u16,
    pub site_id: String,
    pub site_name: String,
    pub region: Option<String>,
    pub frontend_url: Option<String>,
    pub backend_url: String,
    pub registry_ttl_secs: u64,
    pub heartbeat_interval_secs: u64,
}

fn parse_time_string(value: Option<String>) -> Option<SystemTime> {
    let raw = value?.trim().to_string();
    if raw.is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(&raw).ok().and_then(|dt| {
        let millis = dt.timestamp_millis();
        if millis < 0 {
            None
        } else {
            Some(UNIX_EPOCH + Duration::from_millis(millis as u64))
        }
    })
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|v| {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn normalize_string(value: impl Into<String>) -> String {
    value.into().trim().to_string()
}

fn derive_frontend_url_from_backend(backend_url: &str, bind_host: &str) -> String {
    if let Ok(mut parsed) = reqwest::Url::parse(backend_url) {
        let _ = parsed.set_port(Some(5173));
        return parsed.to_string();
    }
    let host = if bind_host.trim().is_empty() || bind_host == "0.0.0.0" {
        "127.0.0.1"
    } else {
        bind_host
    };
    format!("http://{}:5173", host)
}

fn deployment_sites_sqlite_path() -> String {
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

fn open_registry() -> Result<Connection> {
    let db_path = deployment_sites_sqlite_path();
    let conn =
        Connection::open(&db_path).with_context(|| format!("打开站点注册表失败: {}", db_path))?;
    migrate_schema(&conn)?;
    Ok(conn)
}

fn migrate_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS deployment_sites (
            id TEXT PRIMARY KEY,
            site_id TEXT,
            name TEXT NOT NULL,
            description TEXT,
            region TEXT,
            project_name TEXT,
            project_path TEXT,
            project_code INTEGER,
            frontend_url TEXT,
            backend_url TEXT,
            bind_host TEXT,
            bind_port INTEGER,
            config_json TEXT NOT NULL DEFAULT '{}',
            selected_projects TEXT NOT NULL DEFAULT '[]',
            e3d_projects_json TEXT NOT NULL DEFAULT '[]',
            root_directory TEXT,
            parallel_processing BOOLEAN NOT NULL DEFAULT 0,
            max_concurrent INTEGER,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            task_id TEXT,
            status TEXT DEFAULT 'Configuring',
            health_url TEXT,
            last_health_check TEXT,
            env TEXT,
            owner TEXT,
            tags_json TEXT,
            notes TEXT,
            last_seen_at TEXT
        );
        CREATE TABLE IF NOT EXISTS wizard_tasks (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            task_type TEXT NOT NULL,
            status TEXT NOT NULL,
            config_json TEXT NOT NULL,
            wizard_config_json TEXT,
            priority TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            started_at TEXT,
            completed_at TEXT,
            progress_percentage REAL DEFAULT 0.0,
            current_step TEXT,
            logs_json TEXT DEFAULT '[]'
        );
        "#,
    )?;

    let alter_statements = [
        "ALTER TABLE deployment_sites ADD COLUMN site_id TEXT",
        "ALTER TABLE deployment_sites ADD COLUMN description TEXT",
        "ALTER TABLE deployment_sites ADD COLUMN region TEXT",
        "ALTER TABLE deployment_sites ADD COLUMN project_name TEXT",
        "ALTER TABLE deployment_sites ADD COLUMN project_path TEXT",
        "ALTER TABLE deployment_sites ADD COLUMN project_code INTEGER",
        "ALTER TABLE deployment_sites ADD COLUMN frontend_url TEXT",
        "ALTER TABLE deployment_sites ADD COLUMN backend_url TEXT",
        "ALTER TABLE deployment_sites ADD COLUMN bind_host TEXT",
        "ALTER TABLE deployment_sites ADD COLUMN bind_port INTEGER",
        "ALTER TABLE deployment_sites ADD COLUMN e3d_projects_json TEXT DEFAULT '[]'",
        "ALTER TABLE deployment_sites ADD COLUMN env TEXT",
        "ALTER TABLE deployment_sites ADD COLUMN owner TEXT",
        "ALTER TABLE deployment_sites ADD COLUMN tags_json TEXT",
        "ALTER TABLE deployment_sites ADD COLUMN notes TEXT",
        "ALTER TABLE deployment_sites ADD COLUMN last_seen_at TEXT",
    ];

    for sql in alter_statements {
        let _ = conn.execute(sql, []);
    }

    for sql in [
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_deployment_sites_site_id ON deployment_sites(site_id)",
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_deployment_sites_backend_url ON deployment_sites(backend_url) WHERE backend_url IS NOT NULL AND trim(backend_url) != ''",
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_deployment_sites_bind_addr ON deployment_sites(bind_host, bind_port) WHERE bind_host IS NOT NULL AND trim(bind_host) != '' AND bind_port IS NOT NULL",
    ] {
        if let Err(err) = conn.execute(sql, []) {
            eprintln!("站点注册表索引初始化失败: {}", err);
        }
    }

    Ok(())
}

fn site_status_from_str(raw: &str) -> DeploymentSiteStatus {
    match raw.trim().to_ascii_lowercase().as_str() {
        "configuring" => DeploymentSiteStatus::Configuring,
        "deploying" => DeploymentSiteStatus::Deploying,
        "running" => DeploymentSiteStatus::Running,
        "failed" => DeploymentSiteStatus::Failed,
        "stopped" => DeploymentSiteStatus::Stopped,
        "offline" => DeploymentSiteStatus::Offline,
        _ => DeploymentSiteStatus::Configuring,
    }
}

fn site_status_to_str(status: &DeploymentSiteStatus) -> &'static str {
    match status {
        DeploymentSiteStatus::Configuring => "Configuring",
        DeploymentSiteStatus::Deploying => "Deploying",
        DeploymentSiteStatus::Running => "Running",
        DeploymentSiteStatus::Failed => "Failed",
        DeploymentSiteStatus::Stopped => "Stopped",
        DeploymentSiteStatus::Offline => "Offline",
    }
}

fn derive_e3d_projects(site: &DeploymentSite) -> Vec<E3dProjectInfo> {
    if !site.e3d_projects.is_empty() {
        return site.e3d_projects.clone();
    }

    let now = SystemTime::now();
    let project_name = if !site.project_name.trim().is_empty() {
        site.project_name.clone()
    } else {
        site.config.project_name.clone()
    };
    let project_path = site
        .project_path
        .clone()
        .or_else(|| {
            if site.config.project_path.trim().is_empty() {
                None
            } else {
                Some(site.config.project_path.clone())
            }
        })
        .unwrap_or_default();

    if project_name.is_empty() && project_path.is_empty() {
        return Vec::new();
    }

    vec![E3dProjectInfo {
        name: if project_name.is_empty() {
            project_path.clone()
        } else {
            project_name
        },
        path: project_path,
        project_code: site.project_code.or(Some(site.config.project_code)),
        db_file_count: 0,
        size_bytes: 0,
        last_modified: now,
        selected: true,
        description: None,
    }]
}

fn derive_root_directory(site: &DeploymentSite) -> Option<String> {
    if let Some(path) = site.project_path.as_ref().filter(|v| !v.trim().is_empty()) {
        return Some(path.clone());
    }
    if !site.config.project_path.trim().is_empty() {
        return Some(site.config.project_path.clone());
    }
    site.e3d_projects.first().and_then(|project| {
        if project.path.trim().is_empty() {
            None
        } else {
            Some(project.path.clone())
        }
    })
}

fn load_site_from_row(row: &rusqlite::Row<'_>, ttl_secs: u64) -> rusqlite::Result<DeploymentSite> {
    let id: String = row.get("id")?;
    let site_id: Option<String> = row.get("site_id").ok();
    let name: String = row.get("name")?;
    let description: Option<String> = row.get("description").ok();
    let region: Option<String> = row.get("region").ok();
    let project_name: Option<String> = row.get("project_name").ok();
    let project_path: Option<String> = row.get("project_path").ok();
    let project_code: Option<u32> = row.get("project_code").ok();
    let frontend_url: Option<String> = row.get("frontend_url").ok();
    let backend_url: Option<String> = row.get("backend_url").ok();
    let bind_host: Option<String> = row.get("bind_host").ok();
    let bind_port: Option<u16> = row.get("bind_port").ok();
    let config_json: String = row.get("config_json").unwrap_or_else(|_| "{}".to_string());
    let selected_projects_json: String = row
        .get("selected_projects")
        .unwrap_or_else(|_| "[]".to_string());
    let e3d_projects_json: String = row
        .get("e3d_projects_json")
        .unwrap_or_else(|_| "[]".to_string());
    let root_directory: Option<String> = row.get("root_directory").ok();
    let created_at: Option<String> = row.get("created_at").ok();
    let updated_at: Option<String> = row.get("updated_at").ok();
    let status_raw: Option<String> = row.get("status").ok();
    let health_url: Option<String> = row.get("health_url").ok();
    let last_health_check: Option<String> = row.get("last_health_check").ok();
    let env: Option<String> = row.get("env").ok();
    let owner: Option<String> = row.get("owner").ok();
    let tags_json: Option<String> = row.get("tags_json").ok();
    let notes: Option<String> = row.get("notes").ok();
    let last_seen_at: Option<String> = row.get("last_seen_at").ok();

    let mut config: DatabaseConfig = serde_json::from_str(&config_json).unwrap_or_default();
    if let Some(name) = project_name.clone().filter(|v| !v.trim().is_empty()) {
        config.project_name = name;
    }
    if let Some(path) = project_path.clone().filter(|v| !v.trim().is_empty()) {
        config.project_path = path;
    }
    if let Some(code) = project_code {
        config.project_code = code;
    }
    let config_project_code = config.project_code;
    let config_project_name = config.project_name.clone();
    let config_project_path = config.project_path.clone();

    let mut e3d_projects: Vec<E3dProjectInfo> =
        serde_json::from_str(&e3d_projects_json).unwrap_or_default();
    if e3d_projects.is_empty() {
        let selected_projects: Vec<String> =
            serde_json::from_str(&selected_projects_json).unwrap_or_default();
        let now = SystemTime::now();
        e3d_projects = selected_projects
            .into_iter()
            .map(|path| {
                let name = Path::new(&path)
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or(&path)
                    .to_string();
                E3dProjectInfo {
                    name,
                    path,
                    project_code,
                    db_file_count: 0,
                    size_bytes: 0,
                    last_modified: now,
                    selected: true,
                    description: None,
                }
            })
            .collect();
    }

    if e3d_projects.is_empty() {
        let fallback_name = project_name
            .clone()
            .unwrap_or_else(|| config.project_name.clone());
        let fallback_path = project_path
            .clone()
            .unwrap_or_else(|| config.project_path.clone());
        if !fallback_name.trim().is_empty() || !fallback_path.trim().is_empty() {
            e3d_projects.push(E3dProjectInfo {
                name: if fallback_name.trim().is_empty() {
                    fallback_path.clone()
                } else {
                    fallback_name.clone()
                },
                path: fallback_path,
                project_code: project_code.or(Some(config.project_code)),
                db_file_count: 0,
                size_bytes: 0,
                last_modified: SystemTime::now(),
                selected: true,
                description: None,
            });
        }
    }

    let status = site_status_from_str(status_raw.as_deref().unwrap_or("Configuring"));
    let computed_status = if matches!(
        status,
        DeploymentSiteStatus::Stopped | DeploymentSiteStatus::Failed
    ) {
        status
    } else if let Some(last_seen) = last_seen_at.clone() {
        let parsed = parse_time_string(Some(last_seen));
        if let Some(last_seen_time) = parsed {
            match SystemTime::now().duration_since(last_seen_time) {
                Ok(elapsed) if elapsed.as_secs() > ttl_secs => DeploymentSiteStatus::Offline,
                _ => status,
            }
        } else {
            status
        }
    } else {
        status
    };

    let tags = tags_json.and_then(|value| serde_json::from_str::<serde_json::Value>(&value).ok());

    let project_name_final =
        normalize_string(project_name.unwrap_or_else(|| config_project_name.clone()));
    let project_path_final = normalize_optional_string(project_path)
        .or_else(|| normalize_optional_string(root_directory));

    Ok(DeploymentSite {
        id: Some(id.clone()),
        site_id: normalize_string(site_id.unwrap_or_else(|| id.clone())),
        name,
        description,
        e3d_projects,
        config,
        status: computed_status,
        url: normalize_optional_string(backend_url.clone()),
        health_url: normalize_optional_string(health_url),
        env: normalize_optional_string(env.clone()),
        owner: normalize_optional_string(owner),
        tags,
        notes: normalize_optional_string(notes),
        created_at: parse_time_string(created_at),
        updated_at: parse_time_string(updated_at),
        last_health_check: normalize_optional_string(last_health_check),
        region: normalize_optional_string(region).or_else(|| normalize_optional_string(env)),
        project_name: project_name_final,
        project_path: project_path_final
            .or_else(|| normalize_optional_string(Some(config_project_path))),
        project_code: project_code
            .or(Some(config_project_code))
            .filter(|value| *value > 0),
        frontend_url: normalize_optional_string(frontend_url),
        backend_url: normalize_optional_string(backend_url),
        bind_host: normalize_string(bind_host.unwrap_or_else(|| "0.0.0.0".to_string())),
        bind_port,
        last_seen_at: normalize_optional_string(last_seen_at),
    })
}

fn validate_site(site: &DeploymentSite, existing_id: Option<&str>) -> Result<()> {
    if site.site_id.trim().is_empty() {
        anyhow::bail!("site_id 不能为空");
    }
    if site.name.trim().is_empty() {
        anyhow::bail!("站点名称不能为空");
    }
    if site.region.as_deref().unwrap_or("").trim().is_empty() {
        anyhow::bail!("区域不能为空");
    }
    if site.project_name.trim().is_empty() {
        anyhow::bail!("项目不能为空");
    }
    if site.project_code.unwrap_or(0) == 0 {
        anyhow::bail!("project_code 不能为空");
    }
    if site.frontend_url.as_deref().unwrap_or("").trim().is_empty() {
        anyhow::bail!("前端地址不能为空");
    }
    if site.backend_url.as_deref().unwrap_or("").trim().is_empty() {
        anyhow::bail!("后端地址不能为空");
    }
    if site.bind_host.trim().is_empty() {
        anyhow::bail!("监听 Host 不能为空");
    }
    if site.bind_port.unwrap_or(0) == 0 {
        anyhow::bail!("监听 Port 不能为空");
    }

    let sites = list_sites(None)?;
    for existing in sites {
        let existing_id_str = existing
            .id
            .clone()
            .unwrap_or_else(|| existing.site_id.clone());
        if existing_id == Some(existing_id_str.as_str()) {
            continue;
        }
        if existing.site_id == site.site_id {
            anyhow::bail!("site_id 已存在: {}", site.site_id);
        }
        if existing
            .backend_url
            .as_deref()
            .is_some_and(|v| v == site.backend_url.as_deref().unwrap_or(""))
        {
            anyhow::bail!(
                "后端地址已存在: {}",
                site.backend_url.as_deref().unwrap_or("")
            );
        }
        if existing.bind_host == site.bind_host && existing.bind_port == site.bind_port {
            anyhow::bail!(
                "监听地址已存在: {}:{}",
                site.bind_host,
                site.bind_port.unwrap_or_default()
            );
        }
    }

    Ok(())
}

fn upsert_site_internal(
    conn: &Connection,
    site: &DeploymentSite,
    preserve_created_at: Option<String>,
) -> Result<()> {
    let e3d_projects = derive_e3d_projects(site);
    let selected_projects_json = serde_json::to_string(
        &e3d_projects
            .iter()
            .map(|project| project.path.clone())
            .collect::<Vec<_>>(),
    )?;
    let e3d_projects_json = serde_json::to_string(&e3d_projects)?;
    let config_json = serde_json::to_string(&site.config)?;
    let tags_json = site.tags.as_ref().map(|value| value.to_string());
    let created_at = preserve_created_at.unwrap_or_else(now_rfc3339);
    let updated_at = now_rfc3339();
    let root_directory = derive_root_directory(site);
    let bind_port = site.bind_port.map(|value| value as i64);

    conn.execute(
        r#"
        INSERT OR REPLACE INTO deployment_sites (
            id, site_id, name, description, region, project_name, project_path, project_code,
            frontend_url, backend_url, bind_host, bind_port, config_json, selected_projects,
            e3d_projects_json, root_directory, parallel_processing, max_concurrent, created_at,
            updated_at, task_id, status, health_url, last_health_check, env, owner, tags_json,
            notes, last_seen_at
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
            ?9, ?10, ?11, ?12, ?13, ?14,
            ?15, ?16, ?17, ?18, ?19,
            ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27,
            ?28, ?29
        )
        "#,
        params![
            site.id.clone().unwrap_or_else(|| site.site_id.clone()),
            site.site_id.clone(),
            site.name.clone(),
            site.description.clone(),
            site.region.clone(),
            if site.project_name.trim().is_empty() {
                site.config.project_name.clone()
            } else {
                site.project_name.clone()
            },
            site.project_path.clone().or_else(|| {
                if site.config.project_path.trim().is_empty() {
                    None
                } else {
                    Some(site.config.project_path.clone())
                }
            }),
            site.project_code.or(Some(site.config.project_code)),
            site.frontend_url.clone(),
            site.backend_url.clone(),
            site.bind_host.clone(),
            bind_port,
            config_json,
            selected_projects_json,
            e3d_projects_json,
            root_directory,
            false,
            1,
            created_at,
            updated_at,
            Option::<String>::None,
            site_status_to_str(&site.status),
            site.health_url.clone(),
            site.last_health_check.clone(),
            site.env.clone().or(site.region.clone()),
            site.owner.clone(),
            tags_json,
            site.notes.clone(),
            site.last_seen_at.clone(),
        ],
    )?;

    Ok(())
}

pub fn ensure_registry_schema() -> Result<()> {
    let _ = open_registry()?;
    Ok(())
}

pub fn list_sites(query: Option<&DeploymentSiteQuery>) -> Result<Vec<DeploymentSite>> {
    let conn = open_registry()?;
    let ttl_secs = query
        .and_then(|q| q.registry_ttl_secs)
        .unwrap_or(DEFAULT_REGISTRY_TTL_SECS);
    let mut stmt = conn.prepare(
        r#"
        SELECT id, site_id, name, description, region, project_name, project_path, project_code,
               frontend_url, backend_url, bind_host, bind_port, config_json, selected_projects,
               e3d_projects_json, root_directory, created_at, updated_at, status, health_url,
               last_health_check, env, owner, tags_json, notes, last_seen_at
        FROM deployment_sites
        ORDER BY updated_at DESC, created_at DESC
        "#,
    )?;

    let mut items: Vec<DeploymentSite> = stmt
        .query_map([], |row| load_site_from_row(row, ttl_secs))?
        .filter_map(|row| row.ok())
        .collect();

    if let Some(query) = query {
        if let Some(q) = query.q.as_ref().filter(|value| !value.trim().is_empty()) {
            let ql = q.to_lowercase();
            items.retain(|site| {
                site.name.to_lowercase().contains(&ql)
                    || site.project_name.to_lowercase().contains(&ql)
                    || site
                        .project_code
                        .map(|value| value.to_string().contains(&ql))
                        .unwrap_or(false)
                    || site.site_id.to_lowercase().contains(&ql)
                    || site
                        .region
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&ql)
                    || site
                        .frontend_url
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&ql)
                    || site
                        .backend_url
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&ql)
            });
        }
        if let Some(status) = query
            .status
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            let status = status.to_ascii_lowercase();
            items.retain(|site| site_status_to_str(&site.status).eq_ignore_ascii_case(&status));
        }
        if let Some(owner) = query
            .owner
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            items.retain(|site| site.owner.as_deref() == Some(owner.as_str()));
        }
        if let Some(env) = query.env.as_ref().filter(|value| !value.trim().is_empty()) {
            items.retain(|site| {
                site.env.as_deref() == Some(env.as_str())
                    || site.region.as_deref() == Some(env.as_str())
            });
        }
        if let Some(region) = query
            .region
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            items.retain(|site| site.region.as_deref() == Some(region.as_str()));
        }
        if let Some(project_name) = query
            .project_name
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            items.retain(|site| site.project_name == *project_name);
        }

        match query.sort.as_deref() {
            Some("name:asc") => items.sort_by(|a, b| a.name.cmp(&b.name)),
            Some("name:desc") => items.sort_by(|a, b| b.name.cmp(&a.name)),
            Some("updated_at:asc") => items.sort_by(|a, b| a.updated_at.cmp(&b.updated_at)),
            Some("updated_at:desc") => items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at)),
            Some("project_name:asc") => items.sort_by(|a, b| a.project_name.cmp(&b.project_name)),
            Some("project_name:desc") => items.sort_by(|a, b| b.project_name.cmp(&a.project_name)),
            _ => items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at)),
        }
    }

    Ok(items)
}

pub fn get_site(id: &str) -> Result<Option<DeploymentSite>> {
    let conn = open_registry()?;
    let ttl_secs = DEFAULT_REGISTRY_TTL_SECS;
    let mut stmt = conn.prepare(
        r#"
        SELECT id, site_id, name, description, region, project_name, project_path, project_code,
               frontend_url, backend_url, bind_host, bind_port, config_json, selected_projects,
               e3d_projects_json, root_directory, created_at, updated_at, status, health_url,
               last_health_check, env, owner, tags_json, notes, last_seen_at
        FROM deployment_sites
        WHERE id = ?1 OR site_id = ?1
        LIMIT 1
        "#,
    )?;

    let site = stmt
        .query_row([id], |row| load_site_from_row(row, ttl_secs))
        .optional()?;
    Ok(site)
}

pub fn create_site(mut site: DeploymentSite) -> Result<DeploymentSite> {
    if site.site_id.trim().is_empty() {
        site.site_id = site
            .id
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| site.name.clone());
    }
    site.id = Some(site.site_id.clone());
    site.url = site.backend_url.clone();
    site.e3d_projects = derive_e3d_projects(&site);
    validate_site(&site, None)?;

    let conn = open_registry()?;
    upsert_site_internal(&conn, &site, None)?;
    get_site(&site.site_id)?.ok_or_else(|| anyhow!("创建站点后读取失败"))
}

pub fn update_site(
    id: &str,
    req: &super::models::DeploymentSiteUpdateRequest,
) -> Result<DeploymentSite> {
    let mut site = get_site(id)?.ok_or_else(|| anyhow!("未找到站点"))?;
    let original_record_id = site.id.clone().unwrap_or_else(|| site.site_id.clone());
    let mut backend_url_changed = false;
    let preserve_created_at = site
        .created_at
        .map(|time| chrono::DateTime::<Utc>::from(time).to_rfc3339());

    if let Some(site_id) = req.site_id.as_ref() {
        site.site_id = normalize_string(site_id.clone());
        site.id = Some(site.site_id.clone());
    }
    if let Some(name) = req.name.as_ref() {
        site.name = normalize_string(name.clone());
    }
    if let Some(description) = req.description.as_ref() {
        site.description = normalize_optional_string(Some(description.clone()));
    }
    if let Some(config) = req.config.as_ref() {
        site.config = config.clone();
    }
    if let Some(status) = req.status.as_ref() {
        site.status = status.clone();
    }
    if let Some(url) = req.url.as_ref() {
        site.url = normalize_optional_string(Some(url.clone()));
    }
    if let Some(env) = req.env.as_ref() {
        site.env = normalize_optional_string(Some(env.clone()));
    }
    if let Some(owner) = req.owner.as_ref() {
        site.owner = normalize_optional_string(Some(owner.clone()));
    }
    if let Some(health_url) = req.health_url.as_ref() {
        site.health_url = normalize_optional_string(Some(health_url.clone()));
    }
    if let Some(tags) = req.tags.as_ref() {
        site.tags = Some(tags.clone());
    }
    if let Some(notes) = req.notes.as_ref() {
        site.notes = normalize_optional_string(Some(notes.clone()));
    }
    if let Some(region) = req.region.as_ref() {
        site.region = normalize_optional_string(Some(region.clone()));
    }
    if let Some(project_name) = req.project_name.as_ref() {
        site.project_name = normalize_string(project_name.clone());
    }
    if let Some(project_path) = req.project_path.as_ref() {
        site.project_path = normalize_optional_string(Some(project_path.clone()));
    }
    if let Some(project_code) = req.project_code {
        site.project_code = Some(project_code);
        site.config.project_code = project_code;
    }
    if let Some(frontend_url) = req.frontend_url.as_ref() {
        site.frontend_url = normalize_optional_string(Some(frontend_url.clone()));
    }
    if let Some(backend_url) = req.backend_url.as_ref() {
        let normalized = normalize_optional_string(Some(backend_url.clone()));
        backend_url_changed = normalized != site.backend_url;
        site.backend_url = normalized.clone();
        site.url = normalized;
    }
    if let Some(bind_host) = req.bind_host.as_ref() {
        site.bind_host = normalize_string(bind_host.clone());
    }
    if let Some(bind_port) = req.bind_port {
        site.bind_port = Some(bind_port);
    }
    if let Some(last_seen_at) = req.last_seen_at.as_ref() {
        site.last_seen_at = normalize_optional_string(Some(last_seen_at.clone()));
    }
    if req.health_url.is_none() && backend_url_changed {
        site.health_url = site
            .backend_url
            .as_ref()
            .map(|value| format!("{}/api/health", value.trim_end_matches('/')));
    }

    site.e3d_projects = derive_e3d_projects(&site);
    validate_site(&site, Some(original_record_id.as_str()))?;

    let conn = open_registry()?;
    upsert_site_internal(&conn, &site, preserve_created_at)?;
    if original_record_id != site.site_id {
        conn.execute(
            "DELETE FROM deployment_sites WHERE id = ?1 OR site_id = ?1",
            [original_record_id.as_str()],
        )?;
    }
    get_site(&site.site_id)?.ok_or_else(|| anyhow!("更新站点后读取失败"))
}

pub fn delete_site(id: &str) -> Result<bool> {
    let conn = open_registry()?;
    let changed = conn.execute(
        "DELETE FROM deployment_sites WHERE id = ?1 OR site_id = ?1",
        [id],
    )?;
    Ok(changed > 0)
}

pub fn update_health(
    site_id: &str,
    status: DeploymentSiteStatus,
    timestamp: &str,
) -> Result<DeploymentSite> {
    let conn = open_registry()?;
    conn.execute(
        "UPDATE deployment_sites SET status = ?1, last_health_check = ?2, updated_at = ?2 WHERE id = ?3 OR site_id = ?3",
        params![site_status_to_str(&status), timestamp, site_id],
    )?;
    get_site(site_id)?.ok_or_else(|| anyhow!("更新健康状态后读取失败"))
}

pub fn mark_site_status(site_id: &str, status: DeploymentSiteStatus) -> Result<()> {
    let now = now_rfc3339();
    let conn = open_registry()?;
    conn.execute(
        "UPDATE deployment_sites SET status = ?1, updated_at = ?2 WHERE id = ?3 OR site_id = ?3",
        params![site_status_to_str(&status), now, site_id],
    )?;
    Ok(())
}

pub fn upsert_runtime_site(runtime: &WebServerRuntimeConfig) -> Result<DeploymentSite> {
    let db_option = aios_core::get_db_option();
    let config = DatabaseConfig::from_db_option(&db_option);
    let now = SystemTime::now();
    let project_path = if config.project_path.trim().is_empty() {
        None
    } else {
        Some(config.project_path.clone())
    };
    let project_code = if config.project_code == 0 {
        None
    } else {
        Some(config.project_code)
    };
    let health_url = Some(format!(
        "{}/api/health",
        runtime.backend_url.trim_end_matches('/')
    ));

    let mut site = get_site(&runtime.site_id)?.unwrap_or(DeploymentSite {
        id: Some(runtime.site_id.clone()),
        site_id: runtime.site_id.clone(),
        name: runtime.site_name.clone(),
        description: Some("当前 web_server 进程自动注册的站点".to_string()),
        e3d_projects: vec![E3dProjectInfo {
            name: config.project_name.clone(),
            path: config.project_path.clone(),
            project_code,
            db_file_count: 0,
            size_bytes: 0,
            last_modified: now,
            selected: true,
            description: None,
        }],
        config: config.clone(),
        status: DeploymentSiteStatus::Running,
        url: Some(runtime.backend_url.clone()),
        health_url,
        env: runtime.region.clone(),
        owner: None,
        tags: Some(json!({"source": "web_server_runtime"})),
        notes: Some("由当前 web_server 进程自动续约".to_string()),
        created_at: Some(now),
        updated_at: Some(now),
        last_health_check: None,
        region: runtime.region.clone(),
        project_name: config.project_name.clone(),
        project_path,
        project_code,
        frontend_url: runtime.frontend_url.clone(),
        backend_url: Some(runtime.backend_url.clone()),
        bind_host: runtime.bind_host.clone(),
        bind_port: Some(runtime.bind_port),
        last_seen_at: Some(now_rfc3339()),
    });

    site.id = Some(runtime.site_id.clone());
    site.site_id = runtime.site_id.clone();
    site.name = runtime.site_name.clone();
    site.status = DeploymentSiteStatus::Running;
    site.region = runtime.region.clone();
    site.project_name = config.project_name.clone();
    site.project_path = if config.project_path.trim().is_empty() {
        None
    } else {
        Some(config.project_path.clone())
    };
    site.project_code = if config.project_code == 0 {
        None
    } else {
        Some(config.project_code)
    };
    site.frontend_url = runtime.frontend_url.clone();
    site.backend_url = Some(runtime.backend_url.clone());
    site.url = Some(runtime.backend_url.clone());
    site.bind_host = runtime.bind_host.clone();
    site.bind_port = Some(runtime.bind_port);
    site.health_url = Some(format!(
        "{}/api/health",
        runtime.backend_url.trim_end_matches('/')
    ));
    site.last_seen_at = Some(now_rfc3339());
    site.config = config;
    site.e3d_projects = derive_e3d_projects(&site);

    validate_site(&site, site.id.as_deref())?;
    let preserve_created_at = get_site(&runtime.site_id)?
        .and_then(|existing| existing.created_at)
        .map(|time| chrono::DateTime::<Utc>::from(time).to_rfc3339());
    let conn = open_registry()?;
    upsert_site_internal(&conn, &site, preserve_created_at)?;
    get_site(&runtime.site_id)?.ok_or_else(|| anyhow!("注册当前站点失败"))
}

pub fn load_web_server_runtime_config(explicit_port: u16) -> WebServerRuntimeConfig {
    use config as cfg;

    let db_option = aios_core::get_db_option();
    let config_name =
        std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption".to_string());
    let config_file = format!("{}.toml", config_name);
    let cfg_builder = if Path::new(&config_file).exists() {
        cfg::Config::builder()
            .add_source(cfg::File::with_name(&config_name))
            .build()
            .ok()
    } else {
        None
    };

    let bind_host = cfg_builder
        .as_ref()
        .and_then(|cfg| cfg.get_string("web_server.bind_host").ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "0.0.0.0".to_string());

    let bind_port = if explicit_port > 0 {
        explicit_port
    } else {
        cfg_builder
            .as_ref()
            .and_then(|cfg| cfg.get_int("web_server.port").ok())
            .and_then(|value| u16::try_from(value).ok())
            .filter(|value| *value > 0)
            .unwrap_or(3100)
    };

    let region = cfg_builder
        .as_ref()
        .and_then(|cfg| cfg.get_string("web_server.region").ok())
        .or_else(|| {
            let trimmed = db_option.location.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });

    let backend_url = cfg_builder
        .as_ref()
        .and_then(|cfg| cfg.get_string("web_server.public_base_url").ok())
        .or_else(|| {
            cfg_builder
                .as_ref()
                .and_then(|cfg| cfg.get_string("web_server.backend_url").ok())
        })
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("http://127.0.0.1:{}", bind_port));

    let frontend_url = cfg_builder
        .as_ref()
        .and_then(|cfg| cfg.get_string("web_server.frontend_url").ok())
        .or_else(|| {
            cfg_builder
                .as_ref()
                .and_then(|cfg| cfg.get_string("model_center.frontend_base_url").ok())
        })
        .filter(|value| !value.trim().is_empty())
        .or_else(|| Some(derive_frontend_url_from_backend(&backend_url, &bind_host)));

    let registry_ttl_secs = cfg_builder
        .as_ref()
        .and_then(|cfg| cfg.get_int("web_server.registry_ttl_secs").ok())
        .and_then(|value| u64::try_from(value).ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_REGISTRY_TTL_SECS);

    let heartbeat_interval_secs = cfg_builder
        .as_ref()
        .and_then(|cfg| cfg.get_int("web_server.heartbeat_interval_secs").ok())
        .and_then(|value| u64::try_from(value).ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_HEARTBEAT_INTERVAL_SECS);

    let default_site_id = {
        let project_name = if db_option.project_name.trim().is_empty() {
            "site".to_string()
        } else {
            db_option
                .project_name
                .chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() {
                        ch.to_ascii_lowercase()
                    } else {
                        '-'
                    }
                })
                .collect::<String>()
                .trim_matches('-')
                .to_string()
        };
        format!("{}-{}", project_name, bind_port)
    };

    let site_id = cfg_builder
        .as_ref()
        .and_then(|cfg| cfg.get_string("web_server.site_id").ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(default_site_id);

    let site_name = cfg_builder
        .as_ref()
        .and_then(|cfg| cfg.get_string("web_server.site_name").ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            if db_option.project_name.trim().is_empty() {
                format!("站点-{}", bind_port)
            } else {
                db_option.project_name.clone()
            }
        });

    WebServerRuntimeConfig {
        bind_host,
        bind_port,
        site_id,
        site_name,
        region,
        frontend_url,
        backend_url,
        registry_ttl_secs,
        heartbeat_interval_secs,
    }
}
