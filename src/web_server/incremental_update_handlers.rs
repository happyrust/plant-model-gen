use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::{fs, path::Path as FsPath, time::SystemTime};

use crate::web_server::AppState;

/// 增量更新检测状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UpdateDetectionStatus {
    /// 空闲，未运行检测
    Idle,
    /// 正在扫描变更
    Scanning,
    /// 检测到变更，等待处理
    ChangesDetected,
    /// 正在同步
    Syncing,
    /// 同步完成
    Completed,
    /// 错误
    Error(String),
}

/// 增量更新信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncrementalUpdateInfo {
    /// 站点ID
    pub site_id: String,
    /// 站点名称
    pub site_name: String,
    /// 上次同步时间
    pub last_sync_time: Option<DateTime<Utc>>,
    /// 检测状态
    pub detection_status: UpdateDetectionStatus,
    /// 待同步项目数
    pub pending_items: usize,
    /// 已同步项目数
    pub synced_items: usize,
    /// 变更文件列表
    pub changed_files: Vec<ChangedFile>,
    /// 增量大小（字节）
    pub increment_size: u64,
    /// 预计同步时间（秒）
    pub estimated_sync_time: u32,
}

/// 变更文件信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangedFile {
    /// 文件路径
    pub path: String,
    /// 变更类型
    pub change_type: ChangeType,
    /// 文件大小
    pub size: u64,
    /// 修改时间
    pub modified_time: DateTime<Utc>,
    /// 数据库编号
    pub db_num: Option<u32>,
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

/// 变更类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeType {
    Added,
    Modified,
    Deleted,
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

/// 获取所有部署站点的增量更新状态
pub async fn get_all_incremental_status(
    _state: State<AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // TODO: 从数据库获取实际的部署站点信息
    let mock_sites = vec![
        IncrementalUpdateInfo {
            site_id: "site_001".to_string(),
            site_name: "生产环境-主站".to_string(),
            last_sync_time: Some(Utc::now() - chrono::Duration::hours(2)),
            detection_status: UpdateDetectionStatus::ChangesDetected,
            pending_items: 15,
            synced_items: 0,
            changed_files: vec![
                ChangedFile {
                    path: "/desi/7999/model.xkt".to_string(),
                    change_type: ChangeType::Modified,
                    size: 1024 * 1024 * 5, // 5MB
                    modified_time: Utc::now() - chrono::Duration::minutes(30),
                    db_num: Some(7999),
                },
                ChangedFile {
                    path: "/desi/8001/model.xkt".to_string(),
                    change_type: ChangeType::Added,
                    size: 1024 * 1024 * 3, // 3MB
                    modified_time: Utc::now() - chrono::Duration::minutes(15),
                    db_num: Some(8001),
                },
            ],
            increment_size: 1024 * 1024 * 8, // 8MB total
            estimated_sync_time: 120,        // 2 minutes
        },
        IncrementalUpdateInfo {
            site_id: "site_002".to_string(),
            site_name: "测试环境".to_string(),
            last_sync_time: Some(Utc::now() - chrono::Duration::hours(6)),
            detection_status: UpdateDetectionStatus::Idle,
            pending_items: 0,
            synced_items: 235,
            changed_files: vec![],
            increment_size: 0,
            estimated_sync_time: 0,
        },
        IncrementalUpdateInfo {
            site_id: "site_003".to_string(),
            site_name: "开发环境".to_string(),
            last_sync_time: Some(Utc::now() - chrono::Duration::minutes(10)),
            detection_status: UpdateDetectionStatus::Syncing,
            pending_items: 8,
            synced_items: 12,
            changed_files: vec![],
            increment_size: 1024 * 1024 * 15, // 15MB
            estimated_sync_time: 180,         // 3 minutes
        },
    ];

    Ok(Json(json!({
        "success": true,
        "sites": mock_sites,
        "total_pending": 23,
        "total_synced": 247,
        "last_check": Utc::now(),
    })))
}

/// 获取特定站点的增量更新详情
pub async fn get_site_incremental_details(
    _state: State<AppState>,
    Path(site_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // TODO: 从数据库获取实际数据
    let site_info = IncrementalUpdateInfo {
        site_id: site_id.clone(),
        site_name: "生产环境-主站".to_string(),
        last_sync_time: Some(Utc::now() - chrono::Duration::hours(2)),
        detection_status: UpdateDetectionStatus::ChangesDetected,
        pending_items: 15,
        synced_items: 0,
        changed_files: vec![
            ChangedFile {
                path: "/desi/7999/model.xkt".to_string(),
                change_type: ChangeType::Modified,
                size: 1024 * 1024 * 5,
                modified_time: Utc::now() - chrono::Duration::minutes(30),
                db_num: Some(7999),
            },
            ChangedFile {
                path: "/desi/8001/model.xkt".to_string(),
                change_type: ChangeType::Added,
                size: 1024 * 1024 * 3,
                modified_time: Utc::now() - chrono::Duration::minutes(15),
                db_num: Some(8001),
            },
            ChangedFile {
                path: "/desi/8002/metadata.json".to_string(),
                change_type: ChangeType::Modified,
                size: 1024 * 50,
                modified_time: Utc::now() - chrono::Duration::minutes(5),
                db_num: Some(8002),
            },
        ],
        increment_size: 1024 * 1024 * 8,
        estimated_sync_time: 120,
    };

    Ok(Json(json!({
        "success": true,
        "site": site_info,
        "sync_history": [
            {
                "time": Utc::now() - chrono::Duration::hours(2),
                "items_synced": 45,
                "size": 1024 * 1024 * 120,
                "duration": 300,
                "status": "completed"
            },
            {
                "time": Utc::now() - chrono::Duration::hours(8),
                "items_synced": 23,
                "size": 1024 * 1024 * 56,
                "duration": 180,
                "status": "completed"
            }
        ]
    })))
}

/// 启动增量检测
pub async fn start_incremental_detection(
    _state: State<AppState>,
    Path(site_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // TODO: 实际触发增量检测逻辑
    println!("启动站点 {} 的增量检测", site_id);

    Ok(Json(json!({
        "success": true,
        "message": format!("已启动站点 {} 的增量检测", site_id),
        "task_id": format!("detect_{}_{}",site_id, Utc::now().timestamp()),
    })))
}

/// 启动增量同步
pub async fn start_incremental_sync(
    _state: State<AppState>,
    Path(site_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // TODO: 实际触发增量同步逻辑
    println!("启动站点 {} 的增量同步", site_id);

    Ok(Json(json!({
        "success": true,
        "message": format!("已启动站点 {} 的增量同步", site_id),
        "task_id": format!("sync_{}_{}",site_id, Utc::now().timestamp()),
    })))
}

/// 获取检测任务状态
pub async fn get_detection_task_status(
    _state: State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // TODO: 从任务管理器获取实际状态
    Ok(Json(json!({
        "success": true,
        "task_id": task_id,
        "status": "running",
        "progress": 65,
        "scanned_files": 1250,
        "detected_changes": 15,
        "estimated_remaining": 30, // seconds
    })))
}

/// 取消检测或同步任务
pub async fn cancel_task(
    _state: State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // TODO: 实际取消任务逻辑
    println!("取消任务: {}", task_id);

    Ok(Json(json!({
        "success": true,
        "message": format!("任务 {} 已取消", task_id),
    })))
}

/// 获取增量更新配置
pub async fn get_incremental_config(
    _state: State<AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    Ok(Json(json!({
        "success": true,
        "config": {
            "auto_detect": true,
            "detect_interval": 300, // 5 minutes
            "auto_sync": false,
            "sync_batch_size": 10,
            "max_concurrent_syncs": 3,
            "retry_on_failure": true,
            "max_retries": 3,
            "notification_enabled": true,
            "notification_threshold": 50, // MB
        }
    })))
}

/// 更新增量更新配置
#[derive(Debug, Deserialize)]
pub struct UpdateConfigRequest {
    pub auto_detect: Option<bool>,
    pub detect_interval: Option<u32>,
    pub auto_sync: Option<bool>,
    pub sync_batch_size: Option<usize>,
    pub notification_enabled: Option<bool>,
}

pub async fn update_incremental_config(
    _state: State<AppState>,
    Json(config): Json<UpdateConfigRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // TODO: 保存配置到数据库
    println!("更新增量配置: {:?}", config);

    Ok(Json(json!({
        "success": true,
        "message": "配置已更新",
    })))
}
