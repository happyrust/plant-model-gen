#![feature(let_chains)]
#![feature(async_closure)]
#![feature(exact_size_is_empty)]
#![feature(slice_take)]
#![feature(const_async_blocks)]
#![feature(type_alias_impl_trait)]
// 暂时屏蔽warnings
#![allow(warnings)]
#![recursion_limit = "256"]

use crate::data_interface::tidb_manager::AiosDBManager;

#[cfg(feature = "gen_model")]
use crate::fast_model::gen_all_geos_data;

// build_room_relations 支持 CLI/web_server + sqlite-index（实现依赖 gen_model 的 fast_model）

#[cfg(all(
    not(target_arch = "wasm32"),
    feature = "sqlite-index",
    feature = "gen_model"
))]
use crate::fast_model::room_model::build_room_relations;

// 当条件不满足时提供 stub

#[cfg(not(all(
    not(target_arch = "wasm32"),
    feature = "sqlite-index",
    feature = "gen_model"
)))]
pub async fn build_room_relations(
    _db_option: &aios_core::options::DbOption,

    _db_nums: Option<&[u32]>,

    _refno_root: Option<aios_core::RefnoEnum>,
) -> anyhow::Result<()> {
    log::info!("⚠️ build_room_relations 功能需要 sqlite-index + gen_model 特性");

    Ok(())
}

use crate::versioned_db::database::*;

use aios_core::init_model_tables;

use aios_core::options::DbOption;

use aios_core::pdms_data::AttInfoMap;

use aios_core::pdms_types::*;

use aios_core::shape::pdms_shape::PlantMesh;

use aios_core::ssc_setting::{
    set_pbs_fixed_node, set_pbs_node, set_pbs_room_major_node, set_pbs_room_node,
    set_pdms_major_code,
};

use aios_core::tool::db_tool::{db1_dehash, db1_hash};

use aios_core::utils::RecordIdExt;

use aios_core::DbOptionSurrealExt;
#[cfg(feature = "kv-rocksdb")]
use aios_core::connect_local_rocksdb;

use aios_core::{
    SurrealQueryExt, build_cate_relate, init_surreal_with_retry, init_test_surreal,
    project_primary_db,
};

use aios_core::{get_db_option, init_demo_test_surreal};

use anyhow::anyhow;

use chrono::{Datelike, Local, Timelike};

use dashmap::mapref::one::Ref;

use dashmap::{DashMap, DashSet};
use std::str::FromStr;

use itertools::Itertools;

use lazy_static::lazy_static;

use nom::combinator::map;

use serde_json::from_str;

use std::any::TypeId;

use std::collections::BTreeSet;

use std::fs::{self, File, OpenOptions};

use std::ops::Deref;

use std::path::PathBuf;

use std::sync::Arc;

use std::sync::atomic::{AtomicBool, Ordering};

use std::time::Instant;

use team_data::sync_team_data;

// use tokio::sync::mpsc::Sender;

use std::sync::mpsc;

use std::sync::mpsc::Sender;

use versioned_db::database::{define_dbnum_event, sync_pdms};

use log::{LevelFilter, error};

use simplelog::*;

static LOG_INITIALIZED: AtomicBool = AtomicBool::new(false);

pub mod api;

pub mod cata;

pub mod consts;

pub mod cli_args;

pub mod data_interface;

pub mod dblist_parser;

pub mod expression_fix;

pub mod tables;

// pub mod ssc;

pub mod defines;

pub mod team_data;

pub mod test;

pub mod init_project;

pub mod pe_transform_refresh;
pub mod pe_transform_store;

/// 站点部署任务级性能指标采集（spec 004-site-deploy-perf-stats）。
pub mod perf_metrics;

#[cfg(feature = "gui")]
pub mod gui;

#[cfg(feature = "gen_model")]
pub mod fast_model;

#[cfg(feature = "gen_model")]
pub mod scene_tree;

// #[cfg(feature = "gen_model")]

// pub mod xeokit_xtk_generator; // 暂时注释掉，待实现

pub mod versioned_db;

#[cfg(feature = "mqtt")]
pub mod mqtt_service;

pub mod options;

#[macro_use]

pub mod perf_timer;

pub mod profiling;

pub mod shared; // 共享模块（进度广播中心等）

#[cfg(feature = "tonic")]
pub mod grpc_service;

#[cfg(feature = "sqlite-index")]
pub mod sqlite_index;

#[cfg(feature = "sqlite-index")]
pub mod spatial_index;

#[cfg(feature = "web_server")]
pub mod parse_sidecar;

pub mod model_relation_store;

#[cfg(feature = "rvm-import")]
pub mod rvm_import;

#[cfg(all(feature = "rvm-import", feature = "parquet-export"))]
pub mod rvm_compare;

#[cfg(feature = "rvm-import")]
pub mod rvm_obj_export;

#[cfg(all(feature = "sqlite-index", feature = "tonic"))]
pub mod test_spatial_query;

// 添加options模块的重导出

pub use options::DbOptionExt;

pub use options::get_db_option_ext;

// 重新导出MDB相关函数供GRPC服务使用

// pub use crate::api::element::query_types_refnos_names;

// pub use crate::api::attr::{query_explicit_attr, query_numbdbs_by_mdb};

#[cfg(feature = "sql")]
pub use mdb::get_project_mdb;

// // 添加get_project_mdb函数的重新导出

// #[cfg(feature = "grpc")]

// pub async fn get_project_mdb(project_pool: &sqlx::Pool<sqlx::MySql>) -> anyhow::Result<dashmap::DashMap<String, Vec<u32>>> {

//     use crate::api::attr::{query_explicit_attr, query_numbdbs_by_mdb};

//     use crate::api::element::query_types_refnos_names;

//     use dashmap::DashMap;

//     let mut result = DashMap::new();

//     // 获取到所有的 mdb

//     let mdb = query_types_refnos_names(&vec!["MDB"], project_pool, None).await?;

//     for (mdb_refno, mut mdb_name) in mdb {

//         if mdb_name.starts_with("/") { mdb_name.remove(0); }

//         let mdb_attr = query_explicit_attr(mdb_refno, project_pool).await?;

//         let dbs = mdb_attr.get_refu64_vec("CURD");

//         if dbs.is_none() { continue; }

//         let dbs = dbs.unwrap();

//         let numbdbs = query_numbdbs_by_mdb(dbs, project_pool).await?;

//         result.entry(mdb_name).or_insert(numbdbs);

//     }

//     Ok(result)

// }

#[macro_use]
extern crate derive_more;

#[macro_use]
extern crate nom;

// pub async fn start_sync_task(

//     db_option: Arc<DbOption>,

//     progress_sender: Sender<f32>,

// ) -> anyhow::Result<()> {

//     if db_option.total_sync

//         || db_option.incr_sync

//         || db_option.only_sync_sys

//         || db_option.is_sync_history()

//     {

//         // log::info!("开始同步解析数据。");

//         // tokio::spawn(async move {

//         if let Err(e) = sync_pdms(&db_option).await {

//             log::error!("同步PDMS数据失败: {}", e);

//         }

//         //记录进度

//         progress_sender.send(50.0).await?;

//     }

//     if db_option.build_cate_relate() {

//         log::info!("初始化创建Cate relate关系");

//         build_cate_relate(false).await?;

//     }

//     Ok(())

// }

pub async fn run_cli(db_option_ext: options::DbOptionExt) -> anyhow::Result<()> {
    // dbg!("begin run task");

    // 为了兼容性，创建对 inner 的引用

    let db_option = &db_option_ext.inner;

    // 注意：日志初始化已移至 run_app_internal，避免重复初始化

    // 解析完成后重新定义EVENT

    // 注意：define_common_functions 已经在 initialize_databases 中调用

    log::info!("正在重新定义dbnum_event...");

    match define_dbnum_event().await {
        Ok(_) => log::info!("成功重新定义update_dbnum_event"),

        Err(e) => log::warn!("重新定义update_dbnum_event失败: {:?}", e),
    }

    log::info!("预加载方法完成。");

    // 初始化数据库索引

    if let Err(e) = init_model_tables().await {
        log::error!("初始化inst_relate索引失败: {}", e);
    }

    let sync_live = db_option.sync_live.unwrap_or(false);

    let db_option = Arc::new(db_option.clone());

    // initialize_global_db_sender().await;

    // start_sync_task(db_option.clone(), progress_sender.clone()).await?;

    //如果是解析任务，运行完就应该跳出

    if db_option.total_sync
        || db_option.incr_sync
        || db_option.only_sync_sys
        || db_option.is_sync_history()
    {
        // log::info!("开始同步解析数据。");

        // tokio::spawn(async move {

        if let Err(e) = sync_pdms(&db_option).await {
            log::error!("同步PDMS数据失败: {}", e);
            crate::perf_metrics::finalize_task_metrics(false);
            return Err(e);
        }

        //记录进度

        // progress_sender.send(90)?;

        if db_option.build_cate_relate() {
            log::info!("初始化创建Cate relate关系");

            build_cate_relate(false).await?;
        }

        // progress_sender.send(100)?;

        crate::perf_metrics::finalize_task_metrics(true);
        return Ok(());
    }

    let mgr = Arc::new(AiosDBManager::init_form_config().await?);

    // SQLite R*-tree initialization is handled automatically

    // progress_sender.send(10)?;

    //todo 还有个问题，可能需要通过队列来排队任务

    //如果没有生成完，需要等待

    #[cfg(not(feature = "gen_model"))]
    if db_option.is_gen_mesh_or_model() {
        anyhow::bail!("gen_mesh/gen_model 需要 gen_model feature（sync-cli 瘦构建不含模型生成）");
    }

    #[cfg(feature = "gen_model")]
    if db_option.is_gen_mesh_or_model() {
        let _mutation_lock =
            crate::version_management::project_mutation_lock::ProjectMutationLock::acquire_for_current_command(
                &db_option_ext,
            )?;
        log::info!("正在生成模型");

        let mut time = Instant::now();

        fs::create_dir_all("assets/meshes")?;

        //统计一下assets mesh 目录下有多少个mesh，直接忽略去生成

        let path: PathBuf = "assets/meshes".into();

        let generate_started = Instant::now();
        let gen_result = gen_all_geos_data(vec![], &db_option_ext, None).await;
        let generate_ms = generate_started.elapsed().as_millis() as u64;
        // spec 004：生成收尾统计——落库数量走 Surreal 计数（与验收口径一致），
        // 错误数取 failed_sql 转储计数，cache miss 取全局报告快照。
        {
            async fn surreal_count(table: &str) -> usize {
                use aios_core::{SurrealQueryExt, project_primary_db};
                let sql = format!("SELECT count() FROM {table} GROUP ALL;");
                match project_primary_db()
                    .query_take::<Vec<serde_json::Value>>(sql, 0)
                    .await
                {
                    Ok(rows) => rows
                        .first()
                        .and_then(|v| v.get("count"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as usize,
                    Err(_) => 0,
                }
            }
            let cache_miss =
                crate::fast_model::gen_model::cache_miss_report::snapshot_global_report()
                    .map(|r| r.buckets.values().map(|b| b.count as usize).sum())
                    .unwrap_or(0);
            crate::perf_metrics::finish_generate_stage(
                surreal_count("inst_relate").await,
                surreal_count("inst_info").await,
                surreal_count("inst_relate_aabb").await,
                surreal_count("tubi_relate").await,
                crate::fast_model::gen_model::pdms_inst::failed_sql_dump_count(),
                cache_miss,
                generate_ms,
            );
        }
        let gen_result = match gen_result {
            Ok(result) => result,
            Err(error) => {
                crate::perf_metrics::finalize_task_metrics(false);
                return Err(error.into());
            }
        };

        let parquet_report =
            crate::fast_model::export_model::post_gen_export::export_parquet_after_generation_if_enabled(
                &db_option_ext,
                db_option_ext.inner.manual_db_nums.clone(),
            )
            .await?;
        if parquet_report.enabled {
            log::info!(
                "生成后 Parquet 导出完成: dbnums={:?}, skipped={:?}",
                parquet_report.exported_dbnums,
                parquet_report.skipped_reason
            );
        }
        crate::versioned_db::version_commit::publish_model_gen_anchors_after_generation(
            &db_option_ext,
            gen_result.success,
            "full-generation",
            true,
        )
        .await?;
        crate::perf_metrics::finalize_task_metrics(true);
    }

    // 房间计算已迁移至独立 CLI 子命令：`aios-database room compute`

    // For now we'll remove aios_mgr usage and migrate functions to not require it

    // 生成材料表单

    let gen_material = db_option.gen_material.unwrap_or(false);

    if gen_material {

        // save_all_material_data().await?;
    }

    // sync TEAM_DATA数据

    if db_option.only_sync_sys {
        log::info!("开始生成SYS DATA");

        match sync_team_data().await {
            Ok(_) => {
                log::info!("TEAM DATA生成完成");
            }

            Err(e) => {
                dbg!(&e.to_string());
            }
        }
    }

    if db_option.rebuild_ssc_tree {
        dbg!("生成pbs节点");

        // set_pdms_major_code(&aios_mgr).await?;  // TODO: Fix this function call

        let mut handles = vec![];

        set_pbs_fixed_node(&mut handles).await?;

        let rooms = set_pbs_room_node(&mut handles).await?;

        set_pbs_room_major_node(&rooms, &mut handles).await?;

        set_pbs_node(&mut handles).await?;

        futures::future::join_all(handles).await;
    }

    if sync_live {
        let watch_options = crate::version_management::watch_incremental::WatchIncrementalOptions {
            requested_dbnums: db_option.manual_db_nums.clone().unwrap_or_default(),
            generate_model: db_option.is_gen_mesh_or_model(),
            ..Default::default()
        };

        #[cfg(feature = "mqtt")]
        {
            crate::data_interface::mqtt_file_sync::initialize_file_index(&mgr.watcher).await?;
            let watcher = mgr.watcher.clone();
            let (watch_result, _) = tokio::join!(
                crate::version_management::watch_incremental::run_watch_incremental(
                    &db_option_ext,
                    watch_options,
                ),
                crate::data_interface::mqtt_file_sync::poll_sync_e3d_mqtt_events(watcher),
            );
            watch_result?;
        }

        #[cfg(not(feature = "mqtt"))]
        crate::version_management::watch_incremental::run_watch_incremental(
            &db_option_ext,
            watch_options,
        )
        .await?;
    }

    Ok(())
}

/// 初始化日志系统（支持通过 AIOS_LOG_FILE 覆盖日志文件路径）

///

/// 约定：默认仅写文件，不输出到控制台（避免模型生成时日志刷屏导致“看似死循环”）。

/// 如需同时输出到控制台，可设置环境变量 `AIOS_LOG_TO_CONSOLE=1`。

pub fn init_logging(enable_log: bool) {
    if !enable_log {
        return;
    }

    if LOG_INITIALIZED.swap(true, Ordering::Relaxed) {
        return;
    }

    let now = Local::now();

    let default_filename = format!(
        "logs/{}-{:02}-{:02}_{:02}-{:02}-{:02}_parse.log",
        now.year(),
        now.month(),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );

    let filename = std::env::var("AIOS_LOG_FILE").unwrap_or(default_filename);

    let filename = if filename.trim().is_empty() {
        format!(
            "logs/{}-{:02}-{:02}_{:02}-{:02}-{:02}_parse.log",
            now.year(),
            now.month(),
            now.day(),
            now.hour(),
            now.minute(),
            now.second()
        )
    } else {
        filename
    };

    let log_path = PathBuf::from(&filename);

    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // 以追加方式打开，避免在重定向 stdout/stderr 后再次初始化 logger 时截断文件。

    if let Ok(file) = OpenOptions::new().create(true).append(true).open(&filename) {
        let redirected = std::env::var_os("AIOS_STDIO_REDIRECTED").is_some();

        let log_to_console = std::env::var("AIOS_LOG_TO_CONSOLE")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        let mut sinks: Vec<Box<dyn SharedLogger>> = Vec::new();

        // 文件日志：记录 Info 级别

        sinks.push(WriteLogger::new(LevelFilter::Info, Config::default(), file));

        // 仅在明确要求且未重定向 stdout/stderr 时输出到控制台。

        if log_to_console && !redirected {
            sinks.push(TermLogger::new(
                LevelFilter::Info,
                Config::default(),
                TerminalMode::Mixed,
                ColorChoice::Auto,
            ));
        }

        let _ = CombinedLogger::init(sinks);

        log::info!("日志系统初始化成功，日志文件: {}", filename);
    }
}

/// 运行app

pub async fn run_app(option: Option<DbOptionExt>) -> anyhow::Result<()> {
    use std::sync::mpsc;

    // 如果传入的是DbOptionExt，则使用它，否则从配置文件加载

    let db_option_ext = option.unwrap_or_else(|| get_db_option_ext());

    // 检查是否需要启动GRPC服务器

    #[cfg(feature = "grpc")]
    let start_grpc = std::env::var("AIOS_GRPC_ENABLED")
        .map(|v| v.to_lowercase() == "true")
        .unwrap_or(false);

    #[cfg(feature = "grpc")]
    if start_grpc {
        // 在后台启动GRPC服务器

        let grpc_handle = tokio::spawn(async {
            if let Err(e) = crate::grpc_service::start_grpc_server().await {
                log::error!("GRPC server error: {}", e);
            }
        });

        // 继续执行正常的应用逻辑，但不阻塞GRPC服务器

        let app_handle = tokio::spawn(async move { run_app_internal(db_option_ext).await });

        // 等待任一任务完成

        tokio::select! {

            result = app_handle => result?,

            _ = grpc_handle => {},

        }

        return Ok(());
    }

    // 调用内部实现

    run_app_internal(db_option_ext).await
}

/// 内部应用运行逻辑

async fn run_app_internal(db_option_ext: options::DbOptionExt) -> anyhow::Result<()> {
    // 初始化日志系统（在所有操作之前）

    init_logging(db_option_ext.inner.enable_log);

    // 使用 aios_core 统一的数据库初始化函数（file 模式内在连接前会释放冲突端口）

    aios_core::initialize_databases(&db_option_ext.inner).await?;

    run_cli(db_option_ext).await
}

/// aios_core 提供了 init_mem_db_with_retry

/// 改进的数据库连接初始化，支持重试和详细错误诊断
pub mod admin;

pub mod data_state;

// pub mod data_to_excel;

// pub mod data_to_file;

// pub mod other_plat;

// pub mod pcf;

// pub mod plug_in;

// pub mod rvm;

// pub mod ssc;

pub mod version_management;

#[cfg(feature = "web_server")]
pub mod web_api;

#[cfg(feature = "web_server")]
pub mod web_server;
