//! E3D 文件上传和解析 API

use axum::{
    Router,
    extract::{Multipart, Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};
use uuid::Uuid;

#[derive(Clone)]
pub struct UploadApiState {
    pub tasks: Arc<RwLock<HashMap<String, ParseTask>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseTask {
    pub task_id: String,
    pub filename: String,
    pub status: TaskStatus,
    pub progress: f32,
    pub message: String,
    pub created_at: String,
    pub project_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Uploading,
    Parsing,
    Completed,
    Failed,
}

#[derive(Serialize)]
pub struct UploadResponse {
    pub success: bool,
    pub task_id: Option<String>,
    pub message: String,
}

#[derive(Serialize)]
pub struct TaskStatusResponse {
    pub success: bool,
    pub task: Option<ParseTask>,
    pub error_message: Option<String>,
}

pub fn create_upload_routes(state: UploadApiState) -> Router {
    Router::new()
        .route("/api/upload/e3d", post(upload_e3d_file))
        .route("/api/upload/task/{task_id}", get(get_task_status))
        .with_state(state)
}

/// POST /api/upload/e3d - 上传 E3D 文件并触发解析
async fn upload_e3d_file(
    State(state): State<UploadApiState>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let task_id = Uuid::new_v4().to_string();
    let mut filename = String::new();
    let mut project_name = None;

    // 创建上传目录
    let upload_dir = std::path::Path::new("uploads");
    if let Err(e) = tokio::fs::create_dir_all(upload_dir).await {
        error!("创建上传目录失败: {}", e);
        return Json(UploadResponse {
            success: false,
            task_id: None,
            message: format!("创建上传目录失败: {}", e),
        });
    }

    // 处理 multipart 数据
    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();

        if name == "file" {
            filename = field.file_name().unwrap_or("unknown.e3d").to_string();
            let data = match field.bytes().await {
                Ok(d) => d,
                Err(e) => {
                    error!("读取文件数据失败: {}", e);
                    return Json(UploadResponse {
                        success: false,
                        task_id: None,
                        message: format!("读取文件失败: {}", e),
                    });
                }
            };

            let file_path = upload_dir.join(&filename);
            if let Err(e) = tokio::fs::write(&file_path, &data).await {
                error!("保存文件失败: {}", e);
                return Json(UploadResponse {
                    success: false,
                    task_id: None,
                    message: format!("保存文件失败: {}", e),
                });
            }

            info!("文件已保存: {:?}", file_path);
        } else if name == "project_name" {
            if let Ok(text) = field.text().await {
                project_name = Some(text);
            }
        }
    }

    if filename.is_empty() {
        return Json(UploadResponse {
            success: false,
            task_id: None,
            message: "未找到上传文件".to_string(),
        });
    }

    // 创建任务记录
    let task = ParseTask {
        task_id: task_id.clone(),
        filename: filename.clone(),
        status: TaskStatus::Uploading,
        progress: 0.0,
        message: "文件上传完成，等待解析".to_string(),
        created_at: chrono::Local::now().to_rfc3339(),
        project_name: project_name.clone(),
    };

    state
        .tasks
        .write()
        .await
        .insert(task_id.clone(), task.clone());

    // 异步触发解析任务
    let tasks_clone = state.tasks.clone();
    let task_id_clone = task_id.clone();
    let file_path = upload_dir.join(&filename);

    tokio::spawn(async move {
        parse_e3d_task(tasks_clone, task_id_clone, file_path, project_name).await;
    });

    Json(UploadResponse {
        success: true,
        task_id: Some(task_id),
        message: "文件上传成功，开始解析".to_string(),
    })
}

/// GET /api/upload/task/:task_id - 查询任务状态
async fn get_task_status(
    State(state): State<UploadApiState>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let tasks = state.tasks.read().await;

    if let Some(task) = tasks.get(&task_id) {
        Json(TaskStatusResponse {
            success: true,
            task: Some(task.clone()),
            error_message: None,
        })
    } else {
        Json(TaskStatusResponse {
            success: false,
            task: None,
            error_message: Some("任务不存在".to_string()),
        })
    }
}

/// 执行 E3D 解析任务
async fn parse_e3d_task(
    tasks: Arc<RwLock<HashMap<String, ParseTask>>>,
    task_id: String,
    file_path: std::path::PathBuf,
    project_name: Option<String>,
) {
    // 更新状态为解析中
    {
        let mut tasks_map = tasks.write().await;
        if let Some(task) = tasks_map.get_mut(&task_id) {
            task.status = TaskStatus::Parsing;
            task.progress = 10.0;
            task.message = "开始解析 E3D 文件".to_string();
        }
    }

    // 调用解析逻辑
    let result = parse_e3d_file(&file_path, project_name.as_deref()).await;

    // 更新最终状态
    let mut tasks_map = tasks.write().await;
    if let Some(task) = tasks_map.get_mut(&task_id) {
        match result {
            Ok(msg) => {
                task.status = TaskStatus::Completed;
                task.progress = 100.0;
                task.message = msg;
                info!("任务 {} 完成", task_id);
            }
            Err(e) => {
                task.status = TaskStatus::Failed;
                task.message = format!("解析失败: {}", e);
                error!("任务 {} 失败: {}", task_id, e);
            }
        }
    }
}

/// 解析 E3D 文件（调用现有逻辑）
async fn parse_e3d_file(
    file_path: &std::path::Path,
    project_name: Option<&str>,
) -> anyhow::Result<String> {
    use aios_core::init_surreal;

    info!("开始解析文件: {:?}", file_path);

    // 初始化数据库连接
    init_surreal().await?;

    // 获取配置
    let db_option = aios_core::get_db_option();
    let proj_name = project_name.unwrap_or(&db_option.project_name);

    // 调用 parse_pdms_db 解析
    let mdb_path = file_path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("无效的文件路径"))?;

    info!(
        "调用 parse_pdms_db: mdb={}, project={}",
        mdb_path, proj_name
    );

    // 使用 tokio::task::spawn_blocking 执行同步解析
    let mdb_path_owned = mdb_path.to_string();
    let proj_name_owned = proj_name.to_string();

    tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(async move {
            parse_pdms_db::parse_pdms_dir(&mdb_path_owned, &proj_name_owned, None, &None).await
        })
    })
    .await??;

    Ok(format!("E3D 文件解析完成，项目: {}", proj_name))
}
