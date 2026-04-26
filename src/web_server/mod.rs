use aios_core::DbOptionSurrealExt;
use axum::{
    Router,
    body::Body,
    extract::{Query, State},
    http::{Method, StatusCode, header},
    middleware,
    response::{Html, Json, Response},
    routing::{delete, get, post, put},
};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use uuid::Uuid;

pub mod admin_auth_handlers;
pub mod admin_handlers;
pub mod admin_registry_handlers;
pub mod admin_response;
pub mod admin_task_handlers;
pub mod handlers;
pub mod managed_project_sites;
pub mod models;
pub mod ws; // WebSocket 模块
// pub mod templates; // 暂时禁用，有语法错误
pub mod batch_tasks_template;
pub mod collab_migrations;
pub mod dashboard_handlers;
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
pub mod mqtt_monitor_handlers;
pub mod output_instances_files;
pub mod parquet_compact_worker;
pub mod remote_runtime;
pub mod remote_sync_handlers;
pub mod remote_sync_template;
pub mod room_api;
pub mod room_page;
pub mod simple_templates;
pub mod site_config_handlers;
pub mod site_metadata;
pub mod site_registry;
pub mod sqlite_spatial_api;
pub mod sse_handlers; // SSE 事件流处理器
pub mod stream_generate; // 流式模型生成模块
pub mod sync_control_center;
pub mod sync_control_handlers;
pub mod task_creation_handlers;
pub mod topology_handlers; // 拓扑配置处理器
pub mod web_listen; // 当前进程 HTTP 监听与站点身份（一 web_server 一站）
pub mod wizard_handlers;
pub mod wizard_template; // 模型实时补齐 + parquet 增量队列

use crate::web_api::{
    CollisionApiState, E3dTreeApiState, NounHierarchyApiState, SearchApiState,
    SpatialQueryApiState, UploadApiState, assemble_stateless_web_api_routes,
    create_collision_routes, create_e3d_tree_routes, create_noun_hierarchy_routes,
    create_search_routes, create_spatial_query_routes, create_upload_routes,
    stateless_web_api_route_paths,
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
    /// Graceful shutdown 触发通道（B5 / Phase 10）。
    ///
    /// 持有方：`mod.rs::start_web_server_with_config` 在 axum 启动前用
    /// `tokio::sync::oneshot::channel` 创建一对 sender/receiver，把 sender
    /// 放到这里、receiver 喂给 `axum::serve(...).with_graceful_shutdown(...)`。
    ///
    /// 触发点：`site_config_handlers::save_site_config` /
    /// `site_config_handlers::restart_server` 收到请求并写盘成功后，
    /// 调用 `shutdown_tx.lock().await.take()` 拿走 sender 并 `send(())`，
    /// 进入 graceful shutdown：5s 内停止接受新请求，已有连接处理完成后退出。
    ///
    /// 配套：进程级 supervisor（systemd / nssm / pm2）需在 main 退出后自动拉起，
    /// 由此实现"改完配置自重启"的体验。
    pub shutdown_tx: Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
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
            shutdown_tx: Arc::new(tokio::sync::Mutex::new(None)),
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

/// 启动时打印"已注册路由清单"，便于"某个接口是否挂载"这类排障。
///
/// 触发条件：debug build 默认打印；release build 仅在 `AIOS_PRINT_ROUTES=1` 时打印。
///
/// 来源：
/// - stateless 部分来自 [`stateless_web_api_route_paths`]，与
///   [`assemble_stateless_web_api_routes`] 同步维护
/// - stateful 部分列出已知前缀（`search` / `upload` / `collision` / `e3d_tree` /
///   `noun_hierarchy` / `spatial_query` / `room_api` 等）与本文件手写注册的核心
///   段（`/api/tasks`、`/api/model/*`、`/api/surreal/*` 等），不做完整枚举
///
/// 设计动机：继 2026-04-23 `pdms_transform` 漏挂载事件之后，为启动期可观测性
/// 补一道"写完路由后能被看到"的护栏（PDMS Hardening M5，详见
/// `docs/plans/2026-04-24-pdms-hardening-m3-m5-implementation-plan.md`）。
fn maybe_print_registered_routes() {
    let should_print =
        cfg!(debug_assertions) || std::env::var("AIOS_PRINT_ROUTES").ok().as_deref() == Some("1");
    if !should_print {
        return;
    }

    println!("[web_server] registered routes (stateless web_api)");
    for path in stateless_web_api_route_paths() {
        println!("  {}", path);
    }

    println!("[web_server] registered routes (stateful web_api prefixes)");
    for prefix in [
        "/api/spatial/*        (spatial_query_api — 需 SpatialQueryApiState)",
        "/api/noun-hierarchy/* (noun_hierarchy_api — 需 NounHierarchyApiState)",
        "/api/e3d/*            (e3d_tree_api — 需 E3dTreeApiState)",
        "/api/room/*           (room_api — 需 RoomApiState)",
        "/api/collision/*      (collision_api — 需 CollisionApiState)",
        "/api/search/pdms      (search_api — 需 SearchApiState)",
        "/api/upload/*         (upload_api — 需 UploadApiState)",
    ] {
        println!("  {}", prefix);
    }

    println!("[web_server] registered routes (main router, manual in web_server/mod.rs)");
    for prefix in [
        "/api/tasks*           (任务管理：创建/列表/进度/结果)",
        "/api/model/*          (模型生成 / 查询 / Parquet 导出)",
        "/api/surreal/*        (SurrealDB 连接 / 状态 / 查询)",
        "/api/database/*       (数据库状态与诊断)",
        "/api/incremental/*    (增量更新 / parquet 增量队列)",
        "/api/sctn-test/*      (SCTN 测试流程)",
        "/ws/progress/{task_id} (WebSocket 进度)",
        "/ws/tasks             (WebSocket 任务事件)",
        "/admin/*              (管理端 UI + API，需 admin 会话)",
        "/console/*            (控制台 UI 页面)",
    ] {
        println!("  {}", prefix);
    }
    println!(
        "[web_server] route list above is maintained manually; toggle via AIOS_PRINT_ROUTES=1 (debug build prints by default)"
    );
}

pub async fn start_web_server_with_config(
    port: u16,
    config_file: Option<&str>,
) -> anyhow::Result<()> {
    // 如果指定了配置文件，设置环境变量
    if let Some(config_path) = config_file {
        unsafe {
            std::env::set_var("DB_OPTION_FILE", config_path);
        }
        println!("⚙️  使用配置文件: {}.toml", config_path);
    }

    let app_state = AppState::new();

    // Phase 1.6 · 异地协同 schema 幂等迁移（确保 remote_sync_sites.master_* 列与 node_config 表就位）
    collab_migrations::ensure_collab_schema();

    // 🔧 修复：初始化数据库连接 - 使用统一的 initialize_databases 函数
    println!("🔄 正在初始化数据库连接...");
    println!("📂 当前工作目录: {:?}", std::env::current_dir()?);

    let config_name =
        std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption".to_string());
    println!("📄 尝试读取 {}.toml 配置文件...", config_name);

    // 预先初始化 OnceCell，确保配置已加载
    let _ = aios_core::get_db_option();
    let runtime_site_config =
        crate::web_server::site_registry::load_web_server_runtime_config(port);

    // 获取配置并初始化数据库（包括 SurrealDB）
    let db_option = aios_core::get_db_option();
    {
        let mut config_manager = app_state.config_manager.write().await;
        let runtime_config = DatabaseConfig::from_db_option(&db_option);
        config_manager.current_config = runtime_config.clone();
        config_manager.add_template("runtime", runtime_config);
    }
    if db_option.effective_surrealdb().mode == aios_core::options::DbConnMode::Ws {
        match aios_core::connect_surdb(
            &db_option.surrealdb_conn_str(),
            &db_option.surreal_ns,
            &db_option.project_name,
            &db_option.surreal_user,
            &db_option.surreal_password,
        )
        .await
        {
            Ok(_) => {
                println!("✅ 数据库基础连接已就绪");
            }
            Err(e) if e.to_string().contains("Already connected") => {
                println!("⚠️ 数据库基础连接已存在，沿用当前连接");
            }
            Err(e) => {
                eprintln!("⚠️ 数据库基础连接失败，后续将继续后台重试: {}", e);
            }
        }
    }
    if let Err(e) = aios_core::use_ns_db_compat(
        &aios_core::SUL_DB,
        &db_option.surreal_ns,
        &db_option.project_name,
    )
    .await
    {
        eprintln!("⚠️ 数据库命名空间切换失败，后续将继续后台重试: {}", e);
    }
    if let Err(error) = crate::web_api::review_db::init_review_primary_db(&db_option).await {
        eprintln!(
            "⚠️ review 专用数据库连接初始化失败，后续校审接口可能不可用: {}",
            error
        );
    }
    let config_name_for_init = config_name.clone();
    let startup_ns = db_option.surreal_ns.clone();
    let startup_db = db_option.project_name.clone();
    tokio::spawn(async move {
        match aios_core::initialize_databases(db_option).await {
            Ok(_) => {
                if let Err(error) =
                    aios_core::use_ns_db_compat(&aios_core::SUL_DB, &startup_ns, &startup_db).await
                {
                    eprintln!("⚠️ 数据库初始化成功，但最终命名空间切换失败: {}", error);
                }
                println!("✅ 数据库连接初始化成功");
            }
            Err(e) => {
                let error_msg = e.to_string();
                eprintln!("❌ 数据库初始化失败: {}", error_msg);
                eprintln!("💡 请确保:");
                eprintln!("   1. {}.toml 文件在当前目录", config_name_for_init);
                eprintln!("   2. SurrealDB 服务运行在配置的端口 (默认 8020)");
                eprintln!("   3. 配置文件中的连接信息正确");
                eprintln!("   配置信息: {}", db_option.connection_summary());
                // 不直接返回错误，允许 web-server 继续启动（某些功能可能不需要数据库）
                eprintln!("⚠️ 警告: 数据库连接失败，某些功能可能不可用");
            }
        }
    });
    println!("🛰️ 数据库初始化已转为后台执行，Web 服务优先开始监听");

    // 不让启动期的项目/站点建表阻塞主服务监听；
    // 这些表初始化异常或卡住时，应降级为后台任务，避免 /api 与 /files 整体 502。
    tokio::spawn(async move {
        crate::web_server::handlers::ensure_projects_schema().await;
        crate::web_server::handlers::ensure_deployment_sites_schema().await;
        if let Err(err) = crate::web_server::managed_project_sites::ensure_schema() {
            eprintln!("⚠️ 初始化管理员站点表失败: {}", err);
        }
    });

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

    // 所有"无状态" web_api 路由（room_tree / pdms_attr / pdms_transform / ptset /
    // pdms_model_query / review_integration / platform_api / jwt_auth / review_api /
    // scene_tree / mbd_pipe / pipeline_annotation(nest) / version(nest)）
    // 统一交给 assemble_stateless_web_api_routes 装配，避免新增路由忘记 .merge() 的静默遗漏
    // （历史教训：2026-04-23 pdms_transform 漏挂载，详见 docs/plans/2026-04-23-*）
    let stateless_web_api_routes = assemble_stateless_web_api_routes();

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

    let admin_stateless_routes: Router<AppState> = Router::new()
        .merge(admin_handlers::create_admin_routes())
        .merge(admin_task_handlers::create_admin_task_routes())
        .route_layer(middleware::from_fn(
            admin_auth_handlers::admin_session_middleware,
        ))
        .with_state(());

    let admin_api_routes = Router::<AppState>::new()
        .merge(admin_stateless_routes)
        .merge(admin_registry_handlers::create_admin_registry_routes())
        .with_state(app_state.clone())
        // C4 · 修 G6：remote-sync routes 的 admin auth middleware 现已在
        // `create_remote_sync_routes()` 内部用 `.layer(...)` 注入（与其他
        // admin 路由风格一致），此处无需再外层 `.route_layer(...)`。
        .merge(remote_sync_handlers::create_remote_sync_routes());

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
        .route(
            "/api/dashboard/activities",
            get(dashboard_handlers::api_dashboard_activities),
        )
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
        .route(
            "/api/incremental/archives",
            get(incremental_update_handlers::list_incremental_archives),
        )
        // 增量更新页面
        .route(
            "/incremental",
            get(|uri: axum::http::Uri| async move {
                redirect_legacy_console_path(uri, "/console/sync/incremental").await
            }),
        )
        // 同步控制中心
        .route(
            "/sync-control",
            get(|uri: axum::http::Uri| async move {
                redirect_legacy_console_path(uri, "/console/sync/control").await
            }),
        )
        // ===== MQTT 节点监控 (Phase 1.2 · 从 web-server 迁入) =====
        .route(
            "/api/mqtt/nodes",
            get(mqtt_monitor_handlers::get_mqtt_nodes_status),
        )
        .route(
            "/api/mqtt/nodes/{location}",
            delete(mqtt_monitor_handlers::remove_mqtt_node),
        )
        .route(
            "/api/mqtt/nodes/client-unsubscribed",
            post(mqtt_monitor_handlers::client_unsubscribed),
        )
        .route(
            "/api/mqtt/messages",
            get(mqtt_monitor_handlers::get_message_delivery_status),
        )
        .route(
            "/api/mqtt/messages/{message_id}",
            get(mqtt_monitor_handlers::get_message_delivery_detail),
        )
        // ===== MQTT 订阅与主从控制 (Phase 1.3a · 简化 stub) =====
        .route(
            "/api/mqtt/broker/logs",
            get(sync_control_handlers::get_mqtt_broker_logs_api),
        )
        .route(
            "/api/mqtt/subscription/start",
            post(sync_control_handlers::start_mqtt_subscription_api),
        )
        .route(
            "/api/mqtt/subscription/stop",
            post(sync_control_handlers::stop_mqtt_subscription_api),
        )
        .route(
            "/api/mqtt/subscription/clear-master-config",
            post(sync_control_handlers::clear_master_config_api),
        )
        .route(
            "/api/mqtt/subscription/status",
            get(sync_control_handlers::get_mqtt_subscription_status),
        )
        .route(
            "/api/mqtt/node/set-master",
            post(sync_control_handlers::set_as_master_node),
        )
        .route(
            "/api/mqtt/node/set-client",
            post(sync_control_handlers::set_as_client_node),
        )
        // ===== 站点配置 (Phase 1.1 · 从 web-server 迁入) =====
        .route(
            "/api/site-config",
            get(site_config_handlers::get_site_config),
        )
        .route("/api/site/info", get(site_config_handlers::get_site_info))
        .route(
            "/api/site-config/save",
            post(site_config_handlers::save_site_config),
        )
        .route(
            "/api/site-config/validate",
            post(site_config_handlers::validate_site_config),
        )
        .route(
            "/api/site-config/reload",
            post(site_config_handlers::reload_site_config),
        )
        .route(
            "/api/site-config/restart",
            post(site_config_handlers::restart_server),
        )
        .route(
            "/api/site-config/server-ip",
            get(site_config_handlers::get_server_ip),
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
        // 异地增量环境 — 旧入口兼容跳转（API 路由已迁入 admin_stateless_routes 统一认证）
        .route(
            "/remote-sync",
            get(|uri: axum::http::Uri| async move {
                redirect_legacy_console_path(uri, "/admin/#/collaboration").await
            }),
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
        // 部署站点管理 API（一 web_server 进程对应一个运行站点；多站点 = 多进程 + 不同监听 IP/端口）
        .route("/api/site/identity", get(handlers::api_get_site_identity))
        // 站点清单（只读；创建/更新仍走 /api/deployment-sites）
        .route("/api/sites", get(handlers::api_get_deployment_sites))
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
        .route(
            "/api/deployment-sites/{id}/healthcheck",
            post(handlers::api_healthcheck_deployment_site_post),
        )
        .route(
            "/api/deployment-sites/{id}/export-config",
            get(handlers::api_export_deployment_site_config),
        )
        // 部署站点管理页面
        .route("/deployment-sites", get(admin_registry_redirect))
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
        .route(
            "/spatial-query",
            get(|uri: axum::http::Uri| async move {
                redirect_legacy_console_path(uri, "/console/db/sqlite-spatial").await
            }),
        )
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
        .route("/admin", get(admin_index_page).head(admin_head_page))
        .route("/admin/", get(admin_index_page).head(admin_head_page))
        .route("/console/deployment/sites", get(admin_registry_redirect))
        .route("/console/deployment/sites/", get(admin_registry_redirect))
        .route(
            "/console/sync/remote",
            get(|uri: axum::http::Uri| async move {
                redirect_legacy_console_path(uri, "/admin/#/collaboration").await
            }),
        )
        .route(
            "/console/sync/remote/",
            get(|uri: axum::http::Uri| async move {
                redirect_legacy_console_path(uri, "/admin/#/collaboration").await
            }),
        )
        .route("/console", get(console_index_page).head(console_head_page))
        .route("/console/", get(console_index_page).head(console_head_page))
        // 静态文件服务
        .nest_service(
            "/admin/static",
            ServeDir::new("src/web_server/static/admin"),
        )
        .nest_service(
            "/admin/assets",
            ServeDir::new("src/web_server/static/admin/assets"),
        )
        .nest_service("/static", ServeDir::new("src/web_server/static"))
        .nest_service("/console/assets", ServeDir::new("web_console/dist/assets"))
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
        .route("/", get(console_root_redirect))
        .route(
            "/dashboard",
            get(|uri: axum::http::Uri| async move {
                redirect_legacy_console_path(uri, "/console/dashboard").await
            }),
        )
        .route(
            "/config",
            get(|uri: axum::http::Uri| async move {
                redirect_legacy_console_path(uri, "/console/settings/config").await
            }),
        )
        .route(
            "/tasks",
            get(|uri: axum::http::Uri| async move {
                redirect_legacy_console_path(uri, "/console/tasks").await
            }),
        )
        .route(
            "/tasks/{id}",
            get(|uri: axum::http::Uri| async move {
                let task_id = uri.path().trim_start_matches("/tasks/");
                let task_id = task_id.trim_end_matches('/');
                redirect_response(&format!("/console/tasks/{}", task_id), uri.query())
            }),
        )
        .route(
            "/tasks/{id}/logs",
            get(|uri: axum::http::Uri| async move {
                let task_id = uri
                    .path()
                    .trim_start_matches("/tasks/")
                    .trim_end_matches("/logs")
                    .trim_end_matches('/');
                redirect_response(&format!("/console/tasks/{}/logs", task_id), uri.query())
            }),
        )
        .route(
            "/batch-tasks",
            get(|uri: axum::http::Uri| async move {
                redirect_legacy_console_path(uri, "/console/tasks/batch").await
            }),
        )
        .route(
            "/db-status",
            get(|uri: axum::http::Uri| async move {
                redirect_legacy_console_path(uri, "/console/db/status").await
            }),
        )
        .route(
            "/wizard",
            get(|uri: axum::http::Uri| async move {
                redirect_legacy_console_path(uri, "/console/deployment/wizard").await
            }),
        )
        .route(
            "/space-tools",
            get(|uri: axum::http::Uri| async move {
                redirect_legacy_console_path(uri, "/console/tools/space-tools").await
            }),
        )
        .route(
            "/sqlite-spatial",
            get(|uri: axum::http::Uri| async move {
                redirect_legacy_console_path(uri, "/console/db/sqlite-spatial").await
            }),
        )
        .route(
            "/database-connection",
            get(|uri: axum::http::Uri| async move {
                redirect_legacy_console_path(uri, "/console/settings/database-connection").await
            }),
        )
        // 桥架支撑检测页面 + API
        .route(
            "/tray-supports",
            get(|uri: axum::http::Uri| async move {
                redirect_legacy_console_path(uri, "/console/tools/tray-supports").await
            }),
        )
        .route(
            "/api/sqlite-tray-supports/detect",
            post(handlers::api_sqlite_tray_supports_detect),
        )
        // SCTN 测试流程（后台任务 + 进度 + 结果）
        .route(
            "/sctn-test",
            get(|uri: axum::http::Uri| async move {
                redirect_legacy_console_path(uri, "/console/tools/sctn-test").await
            }),
        )
        .route("/api/sctn-test/run", post(handlers::api_sctn_test_run))
        .route(
            "/api/sctn-test/result/{id}",
            get(handlers::api_sctn_test_result),
        )
        // 空间查询可视化页面
        .route(
            "/spatial-visualization",
            get(|uri: axum::http::Uri| async move {
                redirect_legacy_console_path(uri, "/console/db/spatial-visualization").await
            }),
        )
        // 房间计算管理页面
        .route(
            "/room-management",
            get(|uri: axum::http::Uri| async move {
                redirect_legacy_console_path(uri, "/console/tools/room-management").await
            }),
        )
        // WebSocket 路由
        .route("/ws/progress/{task_id}", get(ws::ws_progress_handler))
        .route("/ws/tasks", get(ws::ws_tasks_handler))
        .fallback(app_history_fallback)
        .with_state(app_state.clone())
        .merge(admin_auth_handlers::create_admin_auth_routes())
        .merge(admin_api_routes)
        .merge(spatial_query_routes)
        .merge(noun_hierarchy_routes)
        .merge(e3d_tree_routes)
        .merge(room_routes)
        .merge(collision_routes)
        .merge(search_routes)
        .merge(upload_routes)
        // 一次性合并所有无状态 web_api 路由（含 nest 前缀），见 web_api::assemble_stateless_web_api_routes
        .merge(stateless_web_api_routes)
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::PUT,
                    Method::PATCH,
                    Method::DELETE,
                    Method::OPTIONS,
                ])
                .allow_headers(Any),
        );

    let listen_host = runtime_site_config.bind_host.clone();
    let listen_port = runtime_site_config.bind_port;
    let listener =
        tokio::net::TcpListener::bind(format!("{}:{}", listen_host, listen_port)).await?;
    web_listen::init_web_listen(listen_host.clone(), listen_port);
    web_listen::init_site_identity(runtime_site_config.clone());
    if let Err(err) = crate::web_server::site_registry::upsert_runtime_site(&runtime_site_config) {
        eprintln!("⚠️  启动时注册当前站点失败: {}", err);
    }
    let heartbeat_runtime = runtime_site_config.clone();
    let heartbeat_handle = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(
                heartbeat_runtime.heartbeat_interval_secs,
            ))
            .await;
            if let Err(err) =
                crate::web_server::site_registry::upsert_runtime_site(&heartbeat_runtime)
            {
                eprintln!("站点心跳续约失败: {}", err);
            }
        }
    });
    admin_auth_handlers::start_session_cleanup_timer();
    maybe_print_registered_routes();
    println!("🚀 Web UI服务器启动成功！");
    println!("📱 访问地址: http://localhost:{}", listen_port);
    println!("🌐 对外后端地址: {}", runtime_site_config.backend_url);
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

    // B5 / Phase 10 · graceful shutdown 接入点
    //
    // 创建 oneshot 通道：sender 放进 AppState，可由
    // `site_config_handlers::save_site_config` 等处理器在写盘成功后取走并触发；
    // receiver 交给 `axum::serve(...).with_graceful_shutdown(...)`，触发后
    // axum 立即停止接受新连接，已建立连接走完 in-flight 请求再退出。
    //
    // 真正的"自动重启"由进程级 supervisor 接力（systemd / nssm / pm2 等），
    // 详见 `docs/plans/2026-04-26-site-admin-next-steps.md §4`。
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    {
        let mut slot = app_state.shutdown_tx.lock().await;
        *slot = Some(shutdown_tx);
    }
    let serve_result = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
            println!("📴 收到 graceful shutdown 信号，停止接受新请求；in-flight 请求处理完成后进程退出");
        })
        .await;
    heartbeat_handle.abort();
    if let Err(err) = crate::web_server::site_registry::mark_site_status(
        &runtime_site_config.site_id,
        DeploymentSiteStatus::Stopped,
    ) {
        eprintln!("退出时标记站点停止失败: {}", err);
    }
    serve_result?;
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

const WEB_CONSOLE_DIST_DIR: &str = "web_console/dist";
const WEB_CONSOLE_INDEX_FILE: &str = "web_console/dist/index.html";
const ADMIN_STATIC_DIR: &str = "src/web_server/static/admin";
const ADMIN_INDEX_FILE: &str = "src/web_server/static/admin/index.html";
const CONSOLE_ROUTE_MAPPINGS: &[(&str, &str)] = &[
    ("/dashboard", "/console/dashboard"),
    ("/tasks", "/console/tasks"),
    ("/batch-tasks", "/console/tasks/batch"),
    ("/deployment-sites", "/console/deployment/sites"),
    ("/wizard", "/console/deployment/wizard"),
    ("/sync-control", "/console/sync/control"),
    ("/remote-sync", "/console/sync/remote"),
    ("/incremental", "/console/sync/incremental"),
    ("/db-status", "/console/db/status"),
    ("/database-status", "/console/db/status"),
    ("/sqlite-spatial", "/console/db/sqlite-spatial"),
    ("/spatial-query", "/console/db/sqlite-spatial"),
    (
        "/spatial-visualization",
        "/console/db/spatial-visualization",
    ),
    (
        "/database-connection",
        "/console/settings/database-connection",
    ),
    ("/config", "/console/settings/config"),
    ("/space-tools", "/console/tools/space-tools"),
    ("/tray-supports", "/console/tools/tray-supports"),
    ("/sctn-test", "/console/tools/sctn-test"),
    ("/room-management", "/console/tools/room-management"),
];

fn web_console_index_html() -> Option<String> {
    std::fs::read_to_string(WEB_CONSOLE_INDEX_FILE).ok()
}

fn admin_index_html() -> Option<String> {
    std::fs::read_to_string(ADMIN_INDEX_FILE).ok()
}

fn is_admin_route_request(path: &str) -> bool {
    path == "/admin" || path.starts_with("/admin/")
}

fn is_console_asset_request(path: &str) -> bool {
    path == "/console" || path.starts_with("/console/")
}

fn is_excluded_spa_path(path: &str) -> bool {
    path.starts_with("/api/")
        || path.starts_with("/files/")
        || path.starts_with("/static/")
        || path.starts_with("/assets/")
        || path.starts_with("/ws/")
        || path == "/api"
        || path == "/files"
        || path == "/static"
        || path == "/assets"
        || path == "/ws"
}

async fn console_index_page() -> Result<Html<String>, StatusCode> {
    web_console_index_html()
        .map(Html)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn admin_index_page() -> Result<Html<String>, StatusCode> {
    admin_index_html().map(Html).ok_or(StatusCode::NOT_FOUND)
}

async fn admin_head_page() -> Result<Response, StatusCode> {
    if admin_index_html().is_none() {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::empty())
        .unwrap_or_else(|_| Response::new(Body::empty())))
}

async fn console_head_page() -> Result<Response, StatusCode> {
    if web_console_index_html().is_none() {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::empty())
        .unwrap_or_else(|_| Response::new(Body::empty())))
}

async fn console_root_redirect() -> Response {
    redirect_response("/console", None)
}

async fn admin_registry_redirect() -> Response {
    redirect_response("/admin/#/registry", None)
}

async fn admin_history_fallback(uri: axum::http::Uri) -> Result<Response, StatusCode> {
    let path = uri.path();
    if !is_admin_route_request(path)
        || path.starts_with("/admin/static/")
        || path.starts_with("/admin/assets/")
        || path == "/admin/static"
        || path == "/admin/assets"
        || path == "/admin/api"
    {
        return Err(StatusCode::NOT_FOUND);
    }

    let relative = path
        .strip_prefix("/admin/")
        .or_else(|| path.strip_prefix("/admin"))
        .unwrap_or("")
        .trim_start_matches('/');
    let local_path = Path::new(ADMIN_STATIC_DIR).join(relative);
    if !relative.is_empty() && local_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let html = admin_index_html().ok_or(StatusCode::NOT_FOUND)?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(html))
        .unwrap_or_else(|_| Response::new(Body::from(Cow::Borrowed("")))))
}

async fn console_history_fallback(uri: axum::http::Uri) -> Result<Response, StatusCode> {
    let path = uri.path();
    if !is_console_asset_request(path) || is_excluded_spa_path(path) {
        return Err(StatusCode::NOT_FOUND);
    }

    let relative = path
        .strip_prefix("/console/")
        .or_else(|| path.strip_prefix("/console"))
        .unwrap_or("")
        .trim_start_matches('/');
    let local_path = Path::new(WEB_CONSOLE_DIST_DIR).join(relative);
    if !relative.is_empty() && local_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let html = web_console_index_html().ok_or(StatusCode::NOT_FOUND)?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(html))
        .unwrap_or_else(|_| Response::new(Body::from(Cow::Borrowed("")))))
}

async fn app_history_fallback(uri: axum::http::Uri) -> Result<Response, StatusCode> {
    if let Ok(response) = admin_history_fallback(uri.clone()).await {
        return Ok(response);
    }
    console_history_fallback(uri).await
}

fn redirect_response(target: &str, query: Option<&str>) -> Response {
    let mut location = target.to_string();
    if let Some(query) = query.filter(|value| !value.is_empty()) {
        location.push('?');
        location.push_str(query);
    }

    Response::builder()
        .status(StatusCode::TEMPORARY_REDIRECT)
        .header(header::LOCATION, location)
        .body(Body::empty())
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

async fn redirect_legacy_console_path(uri: axum::http::Uri, target: &'static str) -> Response {
    redirect_response(target, uri.query())
}
