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

use crate::fast_model::cal_model::{update_cal_bran_component, update_cal_equip};

#[cfg(feature = "gen_model")]
use crate::fast_model::gen_all_geos_data;

// build_room_relations 支持 CLI/web_server + sqlite-index

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
use crate::fast_model::room_model::build_room_relations;

// 当条件不满足时提供 stub

#[cfg(not(all(not(target_arch = "wasm32"), feature = "sqlite-index")))]

pub async fn build_room_relations(
    _db_option: &aios_core::options::DbOption,

    _db_nums: Option<&[u32]>,

    _refno_root: Option<aios_core::RefnoEnum>,
) -> anyhow::Result<()> {
    log::info!("⚠️ build_room_relations 功能需要 sqlite-index 特性");

    Ok(())
}

use crate::fast_model::mesh_generate::{gen_inst_meshes, process_meshes_update_db_deep};

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

// pub mod model_relation_store_v3; // 暂时注释：SQLite 备选方案，当前使用 SurrealDB

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

    /// 创建db manager
    if sync_live {
        mgr.init_watcher().await?;
    }

    // SQLite R*-tree initialization is handled automatically

    // progress_sender.send(10)?;

    //todo 还有个问题，可能需要通过队列来排队任务

    //如果没有生成完，需要等待

    if db_option.is_gen_mesh_or_model() {
        log::info!("正在生成模型");

        let mut time = Instant::now();

        fs::create_dir_all("assets/meshes")?;

        //统计一下assets mesh 目录下有多少个mesh，直接忽略去生成

        let path: PathBuf = "assets/meshes".into();

        let generate_started = Instant::now();
        let gen_result = gen_all_geos_data(vec![], &db_option_ext, None, None).await;
        let generate_ms = generate_started.elapsed().as_millis() as u64;
        // spec 004：生成收尾统计——落库数量走 Surreal 计数（与验收口径一致），
        // 错误数取 failed_sql 转储计数，cache miss 取全局报告快照。
        {
            async fn surreal_count(table: &str) -> usize {
                let sql = format!("SELECT count() FROM {table} GROUP ALL;");
                match crate::fast_model::model_store::model_query_take::<Vec<serde_json::Value>, _>(
                    sql, 0,
                )
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
        if let Err(e) = gen_result {
            crate::perf_metrics::finalize_task_metrics(false);
            return Err(e.into());
        }

        // 模型生成完成后，若启用 export_parquet_after_gen：
        // - 指定 manual_db_nums 时按指定库导出；
        // - 全库部署时从 inst_relate 自动发现实际生成过的 DESI dbnum。
        if db_option_ext.export_parquet_after_gen {
            #[cfg(feature = "parquet-export")]
            {
                use crate::data_interface::db_meta_manager::db_meta;
                use crate::fast_model::export_model::export_dbnum_instances_parquet::{
                    export_dbnum_instances_parquet, query_distinct_dbnums_from_inst_relate,
                };
                use std::sync::Arc;

                // gen_all_geos_data has already initialized the active DB context.
                // Re-initializing here can fail in lean managed-site schemas while
                // still leaving the generated inst_relate data ready for export.
                let mut dbnums = db_option_ext
                    .inner
                    .manual_db_nums
                    .clone()
                    .filter(|values| !values.is_empty())
                    .unwrap_or_else(Vec::new);

                let dbnum_source = if dbnums.is_empty() {
                    if db_meta().ensure_loaded().is_ok() {
                        dbnums = db_meta().get_dbnums_by_type(&db_option_ext.inner.module);
                        "db_meta_module"
                    } else {
                        match query_distinct_dbnums_from_inst_relate().await {
                            Ok(discovered) if !discovered.is_empty() => {
                                log::warn!(
                                    "db_meta_info.json 不可用，临时从 inst_relate 发现 Parquet 导出 dbnum；可能包含非 DESI 库: {:?}",
                                    discovered
                                );
                                dbnums = discovered;
                                "inst_relate_fallback"
                            }
                            Ok(_) => {
                                log::warn!(
                                    "export_parquet_after_gen 已启用，但 inst_relate 未发现可导出的 dbnum"
                                );
                                "inst_relate_empty"
                            }
                            Err(err) => {
                                log::error!("自动发现 Parquet 导出 dbnum 失败: {}", err);
                                "inst_relate_error"
                            }
                        }
                    }
                } else {
                    "manual_db_nums"
                };

                dbnums.sort_unstable();
                dbnums.dedup();

                if dbnums.is_empty() {
                    log::warn!(
                        "export_parquet_after_gen 已启用，但没有可导出的 dbnum (source={})",
                        dbnum_source
                    );
                } else {
                    log::info!(
                        "📦 自动导出 Parquet: source={}, dbnums={:?}",
                        dbnum_source,
                        dbnums
                    );
                    log::info!("🔄 Parquet 导出前刷新 pe_transform: dbnums={:?}", dbnums);
                    let refreshed = crate::pe_transform_refresh::refresh_pe_transform_for_dbnums(
                        &dbnums,
                        &db_option_ext,
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("Parquet 导出前刷新 pe_transform 失败: {}", e))?;
                    log::info!(
                        "✅ Parquet 导出前 pe_transform 刷新完成: {} 个节点",
                        refreshed
                    );
                }

                let base_output_dir = db_option_ext.get_project_output_dir().join("parquet");
                let db_option = Arc::new(db_option_ext.inner.clone());
                let parquet_root_refno = db_option_ext
                    .inner
                    .debug_model_refnos
                    .as_ref()
                    .and_then(|values| values.first())
                    .and_then(|value| RefnoEnum::from_str(&value.replace('_', "/")).ok());
                let export_started = Instant::now();
                for dbnum in dbnums {
                    log::info!("📦 自动导出 dbnum={} 的 Parquet...", dbnum);
                    let output_dir = base_output_dir.join(dbnum.to_string());
                    export_dbnum_instances_parquet(
                        dbnum,
                        &output_dir,
                        db_option.clone(),
                        true, // verbose
                        None, // target_unit
                        parquet_root_refno,
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("Parquet 导出 dbnum={} 失败: {}", dbnum, e))?;

                    #[cfg(feature = "sqlite-index")]
                    {
                        use crate::spatial_index::SqliteSpatialIndex;
                        use crate::sqlite_index::SqliteAabbIndex;

                        let idx_path = SqliteSpatialIndex::default_path();
                        if let Some(parent) = idx_path.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        let idx = SqliteAabbIndex::open(&idx_path)?;
                        let import_stats =
                            idx.refresh_dbnum_from_parquet_dir(dbnum, &output_dir)?;
                        log::info!(
                            "SQLite spatial index refreshed from Parquet: dbnum={}, inserted={}, path={}",
                            dbnum,
                            import_stats.total_inserted,
                            idx_path.display()
                        );
                    }

                    #[cfg(not(feature = "sqlite-index"))]
                    {
                        log::warn!(
                            "SQLite spatial index refresh skipped because sqlite-index feature is disabled"
                        );
                    }
                }

                // spec 004：导出阶段统计——walk parquet 输出目录汇总文件数与字节数。
                {
                    let mut parquet_files = 0usize;
                    let mut parquet_bytes = 0u64;
                    let mut json_files = 0usize;
                    let mut json_bytes = 0u64;
                    for entry in walkdir::WalkDir::new(&base_output_dir)
                        .into_iter()
                        .filter_map(|e| e.ok())
                        .filter(|e| e.file_type().is_file())
                    {
                        let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
                        match entry.path().extension().and_then(|s| s.to_str()) {
                            Some("parquet") => {
                                parquet_files += 1;
                                parquet_bytes += len;
                            }
                            Some("json") => {
                                json_files += 1;
                                json_bytes += len;
                            }
                            _ => {}
                        }
                    }
                    crate::perf_metrics::record_export_stage(
                        parquet_files,
                        parquet_bytes,
                        json_files,
                        json_bytes,
                        export_started.elapsed().as_millis() as u64,
                    );
                }
            }
            #[cfg(not(feature = "parquet-export"))]
            {
                log::warn!(
                    "export_parquet_after_gen 已启用，但 parquet-export 特性未编译，跳过 Parquet 导出"
                );
            }
        }
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
        // cur_mgr.clone().unwrap().async_watch().await.unwrap();

        //todo 如何处理初始化的同步，第一次启动一定要同步一次，首先生成archive文件，然后再同步

        //是否需要重构下面的这行代码？

        #[cfg(feature = "mqtt")]

        tokio::join!(
            mgr.async_watch(),
            AiosDBManager::poll_sync_e3d_mqtt_events(mgr.watcher.clone()),
        );

        #[cfg(not(feature = "mqtt"))]
        mgr.async_watch().await;
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

    use crate::fast_model::aabb_tree::manual_update_aabbs;

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
    use crate::fast_model::aabb_tree::manual_update_aabbs;

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
