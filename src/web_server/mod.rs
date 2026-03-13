use aios_core::DbOptionSurrealExt;
use axum::{
    Router,
    extract::{Query, State},
    http::{Method, StatusCode, header},
    response::{Html, Json},
    routing::{delete, get, post, put},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use uuid::Uuid;

pub mod handlers;
pub mod models;
pub mod ws; // WebSocket 模块
// pub mod templates; // 暂时禁用，有语法错误
pub mod batch_tasks_template;
pub mod database_diagnostics;
pub mod database_status_handlers;
pub mod db_connection;
pub mod db_startup_handlers;
pub mod db_startup_manager;
pub mod db_status_handlers;
pub mod db_status_template;
pub mod incremental_update_handlers;
pub mod instance_export;
pub mod layout;
pub mod litefs_handlers;
pub mod model_runtime;
pub mod output_instances_files;
pub mod parquet_compact_worker;
pub mod remote_runtime;
pub mod remote_sync_handlers;
pub mod remote_sync_template;
pub mod room_api;
pub mod room_page;
pub mod simple_templates;
pub mod site_metadata;
pub mod sqlite_spatial_api;
pub mod sse_handlers; // SSE 事件流处理器
pub mod stream_generate; // 流式模型生成模块
pub mod sync_control_center;
pub mod sync_control_handlers;
pub mod task_creation_handlers;
pub mod topology_handlers; // 拓扑配置处理器
pub mod wizard_handlers;
pub mod wizard_template; // 模型实时补齐 + parquet 增量队列

use crate::web_api::{
    CollisionApiState, E3dTreeApiState, NounHierarchyApiState, SearchApiState,
    SpatialQueryApiState, UploadApiState, create_collision_routes, create_e3d_tree_routes,
    create_jwt_auth_routes, create_mbd_pipe_routes, create_model_center_routes,
    create_noun_hierarchy_routes, create_pdms_attr_routes, create_pdms_model_query_routes,
    create_pipeline_annotation_routes, create_ptset_routes, create_review_api_routes,
    create_review_integration_routes, create_room_tree_routes, create_scene_tree_routes,
    create_search_routes, create_spatial_query_routes, create_upload_routes, create_version_routes,
};
use handlers::*;
use models::*;

/// Web UI应用状态
#[derive(Clone)]
pub struct AppState {
    /// 任务管理器
    pub task_manager: Arc<Mutex<TaskManager>>,
    /// 配置管理器
    pub config_manager: Arc<RwLock<ConfigManager>>,
    /// 进度广播中心（用于 WebSocket 和 gRPC）
    pub progress_hub: Arc<crate::shared::ProgressHub>,
}

/// 任务管理器
#[derive(Default)]
pub struct TaskManager {
    /// 活跃任务列表
    pub active_tasks: HashMap<String, TaskInfo>,
    /// 任务历史记录
    pub task_history: Vec<TaskInfo>,
}

/// 配置管理器
#[derive(Default)]
pub struct ConfigManager {
    /// 当前配置
    pub current_config: DatabaseConfig,
    /// 配置模板
    pub config_templates: HashMap<String, DatabaseConfig>,
}

impl AppState {
    pub fn new() -> Self {
        let mut config_manager = ConfigManager::default();

        // 添加一些预设配置模板
        config_manager.add_template(
            "default",
            DatabaseConfig {
                name: "默认配置".to_string(),
                manual_db_nums: vec![],
                gen_model: true,
                gen_mesh: true,
                gen_spatial_tree: true,
                apply_boolean_operation: true,
                mesh_tol_ratio: 3.0,
                room_keyword: "-RM".to_string(),
                project_name: "AvevaMarineSample".to_string(),
                project_path: "/Users/dongpengcheng/Documents/models/e3d_models".to_string(),
                project_code: 1516,
                ..Default::default()
            },
        );

        config_manager.add_template(
            "db_7999",
            DatabaseConfig {
                name: "数据库7999配置".to_string(),
                manual_db_nums: vec![7999],
                gen_model: true,
                gen_mesh: true,
                gen_spatial_tree: true,
                apply_boolean_operation: true,
                mesh_tol_ratio: 3.0,
                room_keyword: "-RM".to_string(),
                project_name: "AvevaMarineSample".to_string(),
                project_path: "/Users/dongpengcheng/Documents/models/e3d_models".to_string(),
                project_code: 1516,
                ..Default::default()
            },
        );

        // 创建任务管理器并恢复之前保存的任务
        let mut task_manager = TaskManager::default();

        // 从SQLite恢复任务
        let restored_tasks = wizard_handlers::restore_tasks_from_sqlite();
        for task in restored_tasks {
            task_manager.active_tasks.insert(task.id.clone(), task);
        }

        Self {
            task_manager: Arc::new(Mutex::new(task_manager)),
            config_manager: Arc::new(RwLock::new(config_manager)),
            progress_hub: Arc::new(crate::shared::ProgressHub::default()),
        }
    }
}

impl ConfigManager {
    pub fn add_template(&mut self, name: &str, config: DatabaseConfig) {
        self.config_templates.insert(name.to_string(), config);
    }
}

/// 启动Web UI服务器
pub async fn start_web_server(port: u16) -> anyhow::Result<()> {
    start_web_server_with_config(port, None).await
}

pub async fn start_web_server_with_config(
    port: u16,
    config_file: Option<&str>,
) -> anyhow::Result<()> {
    let app_state = AppState::new();

    // 如果指定了配置文件，设置环境变量
    if let Some(config_path) = config_file {
        unsafe {
            std::env::set_var("DB_OPTION_FILE", config_path);
        }
        println!("⚙️  使用配置文件: {}.toml", config_path);
    }

    // 🔧 修复：初始化数据库连接 - 使用统一的 initialize_databases 函数
    println!("🔄 正在初始化数据库连接...");
    println!("📂 当前工作目录: {:?}", std::env::current_dir()?);

    let config_name =
        std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption".to_string());
    println!("📄 尝试读取 {}.toml 配置文件...", config_name);

    // 预先初始化 OnceCell，确保配置已加载
    let _ = aios_core::get_db_option();

    // 获取配置并初始化数据库（包括 SurrealDB）
    let db_option = aios_core::get_db_option();
    match aios_core::initialize_databases(db_option).await {
        Ok(_) => {
            println!("✅ 数据库连接初始化成功");
        }
        Err(e) => {
            let error_msg = e.to_string();
            eprintln!("❌ 数据库初始化失败: {}", error_msg);
            eprintln!("💡 请确保:");
            eprintln!("   1. {}.toml 文件在当前目录", config_name);
            eprintln!("   2. SurrealDB 服务运行在配置的端口 (默认 8020)");
            eprintln!("   3. 配置文件中的连接信息正确");
            eprintln!("   配置信息: {}", db_option.connection_summary());
            // 不直接返回错误，允许 web-server 继续启动（某些功能可能不需要数据库）
            eprintln!("⚠️ 警告: 数据库连接失败，某些功能可能不可用");
        }
    }

    // 初始化 SurrealDB 中的 projects 表（若已存在忽略错误）
    crate::web_server::handlers::ensure_projects_schema().await;
    // 初始化 SurrealDB 中的 deployment_sites 表
    crate::web_server::handlers::ensure_deployment_sites_schema().await;

    // 确保 Scene Tree 已初始化
    println!("🌳 检查 Scene Tree 初始化状态...");
    match crate::scene_tree::ensure_initialized().await {
        Ok(_) => println!("✅ Scene Tree 初始化检查完成"),
        Err(e) => {
            eprintln!("⚠️ Scene Tree 初始化失败: {}", e);
            // 不阻塞启动，允许后续手动初始化
        }
    }

    // 启动 Parquet compact worker
    let compact_worker_config = parquet_compact_worker::CompactWorkerConfig {
        scan_interval_secs: 30,
        min_incremental_count: 50,
        output_dir: "output".to_string(),
    };
    let _compact_worker_handle =
        parquet_compact_worker::start_compact_worker(compact_worker_config);
    println!("🔄 Parquet compact worker 已启动 (每 30 秒扫描一次)");
    model_runtime::ensure_runtime_started();
    println!("🔄 Model runtime worker 已启动");

    // 初始化空间查询API
    let db_manager = crate::AiosDBManager::init_form_config().await?;
    let spatial_query_state = SpatialQueryApiState {
        db_manager: Arc::new(db_manager.clone()),
    };
    let spatial_query_routes = create_spatial_query_routes(spatial_query_state);

    // 初始化名词层级查询API
    let noun_hierarchy_state = NounHierarchyApiState {
        db_manager: Arc::new(db_manager.clone()),
    };
    let noun_hierarchy_routes = create_noun_hierarchy_routes(noun_hierarchy_state);

    // 初始化 E3D 树 API
    let e3d_tree_state = E3dTreeApiState {
        db_manager: Arc::new(db_manager),
    };
    let e3d_tree_routes = create_e3d_tree_routes(e3d_tree_state);

    // 初始化 Room 树 API（ARCH 房间分组树）
    let room_tree_routes = create_room_tree_routes();

    let pdms_attr_routes = create_pdms_attr_routes();

    // 初始化 Ptset API
    let ptset_routes = create_ptset_routes();

    // PDMS 模型查询辅助 API（type-info / children），用于前端在无 SurrealDB WS 连接时仍能应用 BRAN/HANG 规则
    let pdms_model_query_routes = create_pdms_model_query_routes();

    let room_worker = room_api::init_room_worker();
    let room_api_state = room_api::RoomApiState {
        task_manager: Arc::new(tokio::sync::RwLock::new(
            room_api::RoomTaskManager::default(),
        )),
        progress_hub: app_state.progress_hub.clone(),
        room_worker,
    };
    let room_routes = room_api::create_room_api_routes().with_state(room_api_state);

    // 初始化碰撞检测 API
    let collision_state = CollisionApiState::default();
    let collision_routes = create_collision_routes(collision_state);

    // 初始化检索 API（Meilisearch 可选；通过环境变量 MEILI_URL/MEILI_API_KEY/MEILI_PDMS_INDEX 配置）
    let search_routes = create_search_routes(SearchApiState::from_env());

    // 初始化上传 API
    let upload_state = UploadApiState {
        tasks: Arc::new(RwLock::new(HashMap::new())),
    };
    let upload_routes = create_upload_routes(upload_state);

    let app = Router::new()
        // API路由
        .route("/api/tasks", get(get_tasks).post(create_task))
        .route("/api/tasks/{id}", get(get_task).delete(delete_task))
        .route("/api/tasks/{id}/start", post(start_task))
        .route("/api/tasks/{id}/stop", post(stop_task))
        .route("/api/tasks/{id}/restart", post(restart_task))
        .route("/api/tasks/{id}/error", get(get_task_error_details))
        .route("/api/tasks/{id}/logs", get(get_task_logs))
        .route("/api/tasks/batch", post(create_batch_tasks))
        .route("/api/tasks/next-number", get(get_next_task_number))
        .route("/api/templates", get(get_task_templates))
        .route("/api/config", get(get_config).post(update_config))
        .route("/api/config/templates", get(get_config_templates))
        .route("/api/databases", get(get_available_databases))
        .route("/api/status", get(get_system_status))
        .route("/api/instances", get(handlers::api_get_instances))
        // 基于 Refno 的模型生成 API
        .route(
            "/api/model/generate-by-refno",
            post(handlers::api_generate_by_refno),
        )
        // 按需显示模型（不创建任务）
        .route(
            "/api/model/show-by-refno",
            post(handlers::api_show_by_refno),
        )
        // 流式增量生成模型（SSE 推送进度）
        .route(
            "/api/model/stream-generate",
            post(stream_generate::api_stream_generate),
        )
        // 流式增量生成模型（GET 版本，便于 EventSource）
        .route(
            "/api/model/stream-generate-by-root/{refno}",
            get(stream_generate::api_stream_generate_by_root),
        )
        // 实时查库返回实例数据（用于 parquet miss 回填）
        .route(
            "/api/model/realtime-instances-by-refnos",
            post(model_runtime::api_realtime_instances_by_refnos),
        )
        // parquet 增量导出入队（后台 worker 聚合去重）
        .route(
            "/api/model/parquet-incr-enqueue",
            post(model_runtime::api_parquet_incremental_enqueue),
        )
        // parquet 版本查询（前端轮询）
        .route(
            "/api/model/parquet-version/{dbno}",
            get(model_runtime::api_parquet_version),
        )
        // 获取指定 dbno 的 Parquet 文件列表
        .route(
            "/api/model/{dbno}/files",
            get(handlers::api_list_parquet_files),
        )
        // 获取指定 dbno 的 scene_tree Parquet 文件
        .route(
            "/api/model/{dbno}/scene-tree",
            get(handlers::api_get_scene_tree_file),
        )
        // SurrealDB 控制 (暂时注释掉有编译问题的路由)
        // .route("/api/surreal/start", post(handlers::start_surreal_server))
        // .route("/api/surreal/stop", post(handlers::stop_surreal_server))
        // .route("/api/surreal/restart", post(handlers::restart_surreal_server))
        .route("/api/surreal/status", get(handlers::get_surreal_status))
        .route("/api/surreal/test", post(handlers::test_surreal_connection))
        .route("/api/surreal/check-port", get(handlers::check_port_status))
        .route(
            "/api/surreal/kill-port",
            post(handlers::kill_port_processes_api),
        )
        // 数据库连接监控API
        .route(
            "/api/database/connection/check",
            get(handlers::check_database_connection),
        )
        .route(
            "/api/database/diagnostics",
            get(handlers::run_database_diagnostics_api),
        )
        .route(
            "/api/database/startup-scripts",
            get(handlers::get_startup_scripts),
        )
        .route(
            "/api/database/start-instance",
            post(handlers::start_database_instance),
        )
        // 数据库启动管理API
        .route(
            "/api/database/startup/start",
            post(db_startup_handlers::start_database_api),
        )
        .route(
            "/api/database/startup/status",
            get(db_startup_handlers::get_startup_status),
        )
        .route(
            "/api/database/startup/instances",
            get(db_startup_handlers::get_all_instances),
        )
        .route(
            "/api/database/startup/stop",
            post(db_startup_handlers::stop_database_api),
        )
        .route(
            "/api/database/startup/logs",
            get(db_startup_handlers::get_startup_logs),
        )
        // 增量更新检测API
        .route(
            "/api/incremental/status",
            get(incremental_update_handlers::get_all_incremental_status),
        )
        .route(
            "/api/incremental/site/{site_id}",
            get(incremental_update_handlers::get_site_incremental_details),
        )
        .route(
            "/api/incremental/detect/{site_id}",
            post(incremental_update_handlers::start_incremental_detection),
        )
        .route(
            "/api/incremental/sync/{site_id}",
            post(incremental_update_handlers::start_incremental_sync),
        )
        .route(
            "/api/incremental/task/{task_id}",
            get(incremental_update_handlers::get_detection_task_status),
        )
        .route(
            "/api/incremental/task/{task_id}/cancel",
            post(incremental_update_handlers::cancel_task),
        )
        .route(
            "/api/incremental/config",
            get(incremental_update_handlers::get_incremental_config),
        )
        .route(
            "/api/incremental/config",
            post(incremental_update_handlers::update_incremental_config),
        )
        // 增量更新页面
        .route("/incremental", get(serve_incremental_update_page))
        // 同步控制中心
        .route(
            "/sync-control",
            get(sync_control_handlers::sync_control_page),
        )
        .route(
            "/api/sync/start",
            post(sync_control_handlers::start_sync_service),
        )
        .route(
            "/api/sync/stop",
            post(sync_control_handlers::stop_sync_service),
        )
        .route(
            "/api/sync/restart",
            post(sync_control_handlers::restart_sync_service),
        )
        .route(
            "/api/sync/pause",
            post(sync_control_handlers::pause_sync_service),
        )
        .route(
            "/api/sync/resume",
            post(sync_control_handlers::resume_sync_service),
        )
        .route(
            "/api/sync/status",
            get(sync_control_handlers::get_sync_status),
        )
        .route(
            "/api/sync/events",
            get(sync_control_handlers::sync_events_stream),
        )
        .route(
            "/api/sync/metrics",
            get(sync_control_handlers::get_sync_metrics),
        )
        .route(
            "/api/sync/metrics/history",
            get(sync_control_handlers::get_sync_metrics_history),
        )
        .route(
            "/api/sync/queue",
            get(sync_control_handlers::get_sync_queue),
        )
        .route(
            "/api/sync/queue/clear",
            post(sync_control_handlers::clear_sync_queue),
        )
        .route(
            "/api/sync/config",
            get(sync_control_handlers::get_sync_config),
        )
        .route(
            "/api/sync/config",
            put(sync_control_handlers::update_sync_config),
        )
        .route(
            "/api/sync/test",
            post(sync_control_handlers::test_sync_connection),
        )
        .route("/api/sync/task", post(sync_control_handlers::add_sync_task))
        .route(
            "/api/sync/trigger-download",
            post(sync_control_handlers::trigger_file_download),
        )
        .route(
            "/api/sync/task/{id}/cancel",
            post(sync_control_handlers::cancel_sync_task),
        )
        .route(
            "/api/sync/history",
            get(sync_control_handlers::get_sync_history),
        )
        // SSE 事件流（使用独立路径避免与轮询接口冲突）
        .route(
            "/api/sync/events/stream",
            get(sse_handlers::sync_events_handler),
        )
        .route("/api/sync/events/test", get(sse_handlers::test_sse_handler))
        .route(
            "/api/sync/mqtt/start",
            post(sync_control_handlers::start_mqtt_server_api),
        )
        .route(
            "/api/sync/mqtt/stop",
            post(sync_control_handlers::stop_mqtt_server_api),
        )
        .route(
            "/api/sync/mqtt/status",
            get(sync_control_handlers::get_mqtt_server_status),
        )
        // 异地增量环境配置页面 + API
        .route("/remote-sync", get(remote_sync_handlers::remote_sync_page))
        .route(
            "/api/remote-sync/envs",
            get(remote_sync_handlers::list_envs).post(remote_sync_handlers::create_env),
        )
        .route(
            "/api/remote-sync/envs/{id}",
            get(remote_sync_handlers::get_env)
                .put(remote_sync_handlers::update_env)
                .delete(remote_sync_handlers::delete_env),
        )
        .route(
            "/api/remote-sync/envs/{id}/apply",
            post(remote_sync_handlers::apply_env),
        )
        .route(
            "/api/remote-sync/envs/{id}/activate",
            post(remote_sync_handlers::activate_env),
        )
        .route(
            "/api/remote-sync/runtime/stop",
            post(remote_sync_handlers::stop_runtime),
        )
        // .route("/api/remote-sync/envs/{id}/test-mqtt", post(remote_sync_handlers::test_mqtt_env))
        // .route("/api/remote-sync/envs/{id}/test-http", post(remote_sync_handlers::test_http_env))
        // .route("/api/remote-sync/sites/{id}/test-http", post(remote_sync_handlers::test_http_site))
        .route(
            "/api/remote-sync/runtime/status",
            get(remote_sync_handlers::runtime_status),
        )
        .route(
            "/api/remote-sync/runtime/config",
            get(remote_sync_handlers::runtime_config),
        )
        .route(
            "/api/remote-sync/envs/import-from-dboption",
            post(remote_sync_handlers::import_env_from_dboption),
        )
        .route(
            "/api/remote-sync/logs",
            get(remote_sync_handlers::list_logs),
        )
        .route(
            "/api/remote-sync/stats/daily",
            get(remote_sync_handlers::daily_stats),
        )
        .route(
            "/api/remote-sync/stats/flows",
            get(remote_sync_handlers::flow_stats),
        )
        .route(
            "/api/remote-sync/envs/{id}/sites",
            get(remote_sync_handlers::list_sites).post(remote_sync_handlers::create_site),
        )
        .route(
            "/api/remote-sync/sites/{id}",
            put(remote_sync_handlers::update_site).delete(remote_sync_handlers::delete_site),
        )
        .route(
            "/api/remote-sync/sites/{id}/metadata",
            get(remote_sync_handlers::get_site_metadata),
        )
        .route(
            "/api/remote-sync/sites/{id}/files/{*path}",
            get(remote_sync_handlers::serve_site_files),
        )
        // 拓扑配置 API
        .route(
            "/api/remote-sync/topology",
            get(topology_handlers::get_topology)
                .post(topology_handlers::save_topology)
                .delete(topology_handlers::delete_topology),
        )
        .route(
            "/api/remote-sync/sites/{id}/files",
            get(remote_sync_handlers::serve_site_files_root),
        )
        // LiteFS 节点状态和健康检查 API
        .route("/api/node-status", get(litefs_handlers::get_node_status))
        .route("/api/health", get(litefs_handlers::health_check))
        .route("/api/sync-status", get(litefs_handlers::sync_status))
        // 数据库状态管理API
        .route(
            "/api/database/status",
            get(database_status_handlers::get_all_database_status),
        )
        .route(
            "/api/database/{db_num}/details",
            get(database_status_handlers::get_database_details),
        )
        .route(
            "/api/database/{db_num}/parse",
            post(database_status_handlers::reparse_database),
        )
        .route(
            "/api/database/{db_num}/generate",
            post(database_status_handlers::regenerate_model),
        )
        .route(
            "/api/database/{db_num}/update",
            post(database_status_handlers::trigger_database_update),
        )
        .route(
            "/api/database/{db_num}/clear-cache",
            post(database_status_handlers::clear_database_cache),
        )
        .route(
            "/api/database/batch",
            post(database_status_handlers::execute_batch_operation),
        )
        .route(
            "/api/database/modules",
            get(database_status_handlers::get_module_list),
        )
        // 数据库状态页面
        .route("/database-status", get(serve_database_status_page))
        // 数据库状态管理API
        .route(
            "/api/db-status",
            get(db_status_handlers::get_db_status_list),
        )
        .route(
            "/api/db-status/{dbnum}",
            get(db_status_handlers::get_db_status_detail),
        )
        .route(
            "/api/db-status/update",
            post(db_status_handlers::execute_incremental_update),
        )
        .route(
            "/api/db-status/check-versions",
            get(db_status_handlers::check_file_versions),
        )
        .route(
            "/api/db-status/{dbnum}/auto-update-type",
            post(db_status_handlers::set_auto_update_type),
        )
        .route(
            "/api/db-status/{dbnum}/auto-update",
            post(db_status_handlers::set_auto_update),
        )
        // 本地扫描与同步
        .route(
            "/api/db-sync/scan",
            get(db_status_handlers::scan_local_files),
        )
        .route(
            "/api/db-sync/sync",
            post(db_status_handlers::sync_file_metadata),
        )
        .route(
            "/api/db-sync/rescan",
            post(db_status_handlers::rescan_and_cache),
        )
        // 项目管理 API（最小集：列表 + 创建）
        .route(
            "/api/projects",
            get(handlers::api_get_projects).post(handlers::api_create_project),
        )
        .route(
            "/api/projects/{id}",
            get(handlers::api_get_project)
                .put(handlers::api_update_project)
                .delete(handlers::api_delete_project),
        )
        .route("/api/projects/demo", post(handlers::api_projects_demo))
        .route(
            "/api/projects/{id}/healthcheck",
            post(handlers::api_healthcheck_project),
        )
        // 部署站点管理 API
        .route(
            "/api/deployment-sites/import-dboption",
            post(handlers::api_import_deployment_site_from_dboption),
        )
        .route(
            "/api/deployment-sites",
            get(handlers::api_get_deployment_sites).post(handlers::api_create_deployment_site),
        )
        .route(
            "/api/deployment-sites/{id}",
            get(handlers::api_get_deployment_site)
                .put(handlers::api_update_deployment_site)
                .delete(handlers::api_delete_deployment_site),
        )
        // .route(
        //     "/api/deployment-sites/{id}/browse-directory",
        //     get(handlers::api_browse_deployment_site_directory),
        // )
        .route(
            "/api/deployment-sites/{id}/tasks",
            post(handlers::api_create_deployment_site_task),
        )
        // .route(
        //     "/api/deployment-sites/{id}/healthcheck",
        //     post(handlers::api_healthcheck_deployment_site_post),
        // )
        .route(
            "/api/deployment-sites/{id}/export-config",
            get(handlers::api_export_deployment_site_config),
        )
        // 部署站点管理页面
        .route("/deployment-sites", get(handlers::deployment_sites_page))
        // 数据解析向导API
        .route(
            "/api/wizard/scan-directory",
            get(wizard_handlers::scan_directory),
        )
        .route(
            "/api/wizard/scan-database-files",
            get(wizard_handlers::scan_database_files),
        )
        .route(
            "/api/wizard/list-projects",
            get(wizard_handlers::list_projects),
        )
        .route(
            "/api/wizard/create-task",
            post(wizard_handlers::create_wizard_task),
        )
        .route(
            "/api/wizard/templates",
            get(wizard_handlers::get_wizard_templates),
        )
        .route(
            "/api/wizard/browse-directory",
            get(wizard_handlers::browse_directory),
        )
        // 任务创建API - 使用不同的路径避免冲突
        .route(
            "/api/task-creation",
            post(task_creation_handlers::create_task),
        )
        .route(
            "/api/task-templates",
            get(task_creation_handlers::get_task_templates),
        )
        .route(
            "/api/task-creation/validate-name",
            get(task_creation_handlers::validate_task_name),
        )
        .route(
            "/api/task-creation/preview",
            post(task_creation_handlers::preview_task_config),
        )
        .route(
            "/api/tasks/{task_id}/download",
            get(task_creation_handlers::download_task_export),
        )
        // SQLite 空间索引 API
        .route(
            "/api/sqlite-spatial/rebuild",
            post(handlers::api_sqlite_spatial_rebuild),
        )
        // 空间查询页面
        .route("/spatial-query", get(handlers::spatial_query_page))
        // 空间计算 API
        .route(
            "/api/space/suppo-trays",
            post(handlers::api_space_suppo_trays),
        )
        .route("/api/space/fitting", post(handlers::api_space_fitting))
        .route(
            "/api/space/wall-distance",
            post(handlers::api_space_wall_distance),
        )
        .route(
            "/api/space/fitting-offset",
            post(handlers::api_space_fitting_offset),
        )
        .route(
            "/api/space/steel-relative",
            post(handlers::api_space_steel_relative),
        )
        .route("/api/space/tray-span", post(handlers::api_space_tray_span))
        // SQLite RTree 空间索引：AABB 粗筛查询（供前端按需加载/最近点测量使用）
        .route(
            "/api/sqlite-spatial/query",
            get(sqlite_spatial_api::api_sqlite_spatial_query),
        )
        // SQLite 空间索引：统计与健康检查（诊断索引是否构建正确）
        .route(
            "/api/sqlite-spatial/stats",
            get(sqlite_spatial_api::api_sqlite_spatial_stats),
        )
        // 模型导出 API
        .route("/api/export/gltf", post(handlers::create_export_task))
        .route("/api/export/glb", post(handlers::create_export_task))
        .route(
            "/api/export/status/{task_id}",
            get(handlers::get_export_status),
        )
        .route(
            "/api/export/download/{task_id}",
            get(handlers::download_export),
        )
        .route("/api/export/tasks", get(handlers::list_export_tasks))
        .route("/api/export/cleanup", post(handlers::cleanup_export_tasks))
        // 静态文件服务
        .nest_service("/static", ServeDir::new("src/web_server/static"))
        // /files/output 下的静态文件服务（带 instances 兜底）
        //
        // 说明：不能在同一 Router 上同时注册 `/files/output` 的 nest_service 与其子路由，
        // 否则 axum 会在路由树插入阶段报冲突。因此把“兜底路由 + ServeDir”一起 nest 进去。
        .nest(
            "/files/output",
            Router::new()
                // instances 文件兜底：兼容 instances_cache_for_index 的落盘结构
                .route(
                    "/{project}/instances/{file}",
                    get(output_instances_files::get_project_instances_file),
                )
                .route(
                    "/instances/{file}",
                    get(output_instances_files::get_root_instances_file),
                )
                .fallback_service(ServeDir::new("output")),
        )
        .nest_service(
            "/files/output/database_models",
            ServeDir::new("assets/database_models"),
        )
        .nest_service(
            "/files/database_models",
            ServeDir::new("assets/database_models"),
        )
        .nest_service("/files/meshes", {
            let path = aios_core::get_db_option().get_meshes_path();
            let serve_path = if path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.starts_with("lod_"))
                .unwrap_or(false)
            {
                path.parent().unwrap_or(&path).to_path_buf()
            } else {
                path
            };
            println!("💡 Serving meshes from: {:?}", serve_path);
            ServeDir::new(serve_path)
        })
        // CBA 文件分发服务 - 用于远程站点下载增量数据包
        .nest_service("/assets/archives", ServeDir::new("assets/archives"))
        // 校审附件文件服务
        .nest_service(
            "/files/review_attachments",
            ServeDir::new("assets/review_attachments"),
        )
        // 主页面
        .route("/", get(index_page))
        .route("/dashboard", get(dashboard_page))
        .route("/config", get(config_page))
        .route("/tasks", get(tasks_page))
        .route("/tasks/{id}", get(task_detail_page))
        .route("/tasks/{id}/logs", get(task_logs_page))
        .route("/batch-tasks", get(batch_tasks_page))
        .route("/db-status", get(db_status_page))
        .route("/wizard", get(wizard_page))
        .route("/space-tools", get(space_tools_page))
        .route("/sqlite-spatial", get(handlers::sqlite_spatial_page))
        .route(
            "/database-connection",
            get(handlers::database_connection_page),
        )
        // 桥架支撑检测页面 + API
        .route("/tray-supports", get(handlers::tray_supports_page))
        .route(
            "/api/sqlite-tray-supports/detect",
            post(handlers::api_sqlite_tray_supports_detect),
        )
        // SCTN 测试流程（后台任务 + 进度 + 结果）
        .route("/sctn-test", get(handlers::sctn_test_page))
        .route("/api/sctn-test/run", post(handlers::api_sctn_test_run))
        .route(
            "/api/sctn-test/result/{id}",
            get(handlers::api_sctn_test_result),
        )
        // 空间查询可视化页面
        .route(
            "/spatial-visualization",
            get(handlers::spatial_visualization_page),
        )
        // 房间计算管理页面
        .route("/room-management", get(room_page::room_management_page))
        // WebSocket 路由
        .route("/ws/progress/{task_id}", get(ws::ws_progress_handler))
        .route("/ws/tasks", get(ws::ws_tasks_handler))
        .with_state(app_state.clone())
        .merge(spatial_query_routes)
        .merge(noun_hierarchy_routes)
        .merge(e3d_tree_routes)
        .merge(room_tree_routes)
        .merge(pdms_attr_routes)
        .merge(ptset_routes)
        .merge(pdms_model_query_routes)
        .merge(room_routes)
        .merge(collision_routes)
        .merge(search_routes)
        .merge(upload_routes)
        .merge(create_review_integration_routes())
        .merge(create_model_center_routes())
        .merge(create_jwt_auth_routes())
        .merge(create_review_api_routes())
        .merge(create_scene_tree_routes())
        .merge(create_mbd_pipe_routes())
        .nest("/api/pipeline", create_pipeline_annotation_routes())
        .nest("/api", create_version_routes())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::PUT,
                    Method::DELETE,
                    Method::OPTIONS,
                ])
                .allow_headers(Any),
        );

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    println!("🚀 Web UI服务器启动成功！");
    println!("📱 访问地址: http://localhost:{}", port);
    println!("🎯 功能包括:");
    println!("   - 数据库生成任务管理");
    println!("   - 实时进度监控");
    println!("   - 配置管理");
    println!("   - 任务历史记录");
    // 后台自动更新扫描任务（基于 auto_update + sesno 比较）
    // 注释掉自动调度器，因为数据库服务由配置管理
    // 先确保 SurrealDB 的表结构字段齐备（在生产环境中便于统一管理）
    // crate::web_server::db_status_handlers::ensure_dbnum_info_schema().await;
    // tokio::spawn(auto_update_scheduler(app_state.clone()));

    // 周期性项目健康检查（可通过 WEBUI_HEALTH_SCHED=0 关闭）
    // 也注释掉，避免启动时查询数据库
    // tokio::spawn(crate::web_server::handlers::projects_health_scheduler());

    axum::serve(listener, app).await?;
    Ok(())
}

async fn auto_update_scheduler(state: AppState) {
    use crate::web_server::models::{IncrementalUpdateRequest, UpdateType};
    use aios_core::project_primary_db;
    use axum::{Json, extract::State as AxumState};
    use std::time::Duration;

    loop {
        // 每60秒扫描一次
        tokio::time::sleep(Duration::from_secs(60)).await;

        // 读取 auto_update 的记录
        let sql = "SELECT dbnum, file_name, sesno, project, auto_update, updating FROM dbnum_info_table WHERE auto_update = true";
        let rows = match project_primary_db().query(sql).await {
            Ok(mut resp) => resp.take::<Vec<serde_json::Value>>(0).unwrap_or_default(),
            Err(_) => continue,
        };

        for row in rows {
            let dbnum = row["dbnum"].as_u64().unwrap_or(0) as u32;
            let project = row["project"].as_str().unwrap_or("");
            let updating = row["updating"].as_bool().unwrap_or(false);

            // 计算是否需要更新
            // SESSION_STORE removed
            let cached_sesno = 0u32;
            let latest_file_sesno = {
                // TODO: Implement proper PDMS sesno extraction
                // This requires creating PdmsIO from project directory
                0
            };
            let needs_update = cached_sesno < latest_file_sesno;

            if needs_update && !updating {
                // 读取更新类型
                let typ = row["auto_update_type"].as_str().unwrap_or("ParseAndModel");
                let update_type = match typ {
                    "ParseOnly" => UpdateType::ParseOnly,
                    "Full" => UpdateType::Full,
                    _ => UpdateType::ParseAndModel,
                };
                // 构造并发起增量更新（解析+建模）
                let req = IncrementalUpdateRequest {
                    dbnums: vec![dbnum],
                    force_update: false,
                    update_type,
                    target_sesno: None,
                };
                let _ = crate::web_server::handlers::execute_incremental_update(
                    AxumState(state.clone()),
                    Json(req),
                )
                .await;
            }
        }
    }
}

/// 查询参数
#[derive(Deserialize)]
pub struct TaskQuery {
    pub status: Option<String>,
    pub limit: Option<usize>,
}

/// 创建任务请求
#[derive(Deserialize)]
pub struct CreateTaskRequest {
    pub name: String,
    pub task_type: TaskType,
    pub config: DatabaseConfig,
    /// 可选的元数据（用于批量任务的 batch_id 等）
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// 更新配置请求
#[derive(Deserialize)]
pub struct UpdateConfigRequest {
    pub config: DatabaseConfig,
}
