#![feature(let_chains)]
#![feature(duration_constructors)]
// 暂时屏蔽warnings
#![allow(warnings)]
#![recursion_limit = "256"]

#[macro_use]
extern crate clap;
#[macro_use]
extern crate nom;

extern crate strum;

use std::path::{Path, PathBuf};

use chrono::{Datelike, Local, Timelike};

// #region agent log
// 轻量 NDJSON 记录器：用于 debug-mode 下确认“本次运行的二进制确实生效并能写日志”。
// 注意：本仓库的其它模块也有各自的 agent_log；这里做一个最小实现，避免 main.rs 直接调用时报未定义。
#[cfg(not(feature = "gui"))]
fn agent_now_ms() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

#[cfg(not(feature = "gui"))]
fn agent_log(hypothesis_id: &str, location: &str, message: &str, data: serde_json::Value) {
    if std::env::var_os("AIOS_AGENT_DEBUG").is_none()
        && std::env::var_os("AIOS_AGENT_DEBUG_REFNO").is_none()
        && std::env::var_os("AIOS_AGENT_DEBUG_GEOM_REFNO").is_none()
        && std::env::var_os("AIOS_LOG_FILE").is_none()
    {
        return;
    }

    use serde_json::json;
    use std::fs::OpenOptions;
    use std::io::Write;

    let run_id = std::env::var("AIOS_AGENT_RUNID").unwrap_or_else(|_| "run1".to_string());
    let payload = json!({
        "runId": run_id,
        "hypothesisId": hypothesis_id,
        "location": location,
        "message": message,
        "data": data,
        "timestamp": agent_now_ms(),
    });

    let path = r"d:\work\plant-code\gen_model-dev\.cursor\debug.log";
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{}", payload.to_string());
    }
}
// #endregion

#[cfg(not(feature = "gui"))]
mod cli_modes;

#[cfg(not(feature = "gui"))]
fn parse_lod_level(s: &str) -> Option<aios_core::mesh_precision::LodLevel> {
    use aios_core::mesh_precision::LodLevel;
    match s.trim().to_ascii_uppercase().as_str() {
        "L0" => Some(LodLevel::L0),
        "L1" => Some(LodLevel::L1),
        "L2" => Some(LodLevel::L2),
        "L3" => Some(LodLevel::L3),
        "L4" => Some(LodLevel::L4),
        _ => None,
    }
}

/// 构建导出配置的辅助函数
fn build_export_config(
    refnos_vec: Vec<String>,
    output_path: Option<String>,
    filter_nouns: Option<Vec<String>>,
    include_descendants: bool,
    source_unit: &str,
    target_unit: &str,
    verbose: bool,
    regenerate_plant_mesh: bool,
    dbnum: Option<u32>,
    split_by_site: bool,
    include_negative: bool,
    export_svg: bool,
) -> ExportConfig {
    let run_all_dbnos = refnos_vec.is_empty() && dbnum.is_none();
    ExportConfig {
        refnos_str: refnos_vec,
        output_path,
        filter_nouns,
        include_descendants,
        source_unit: source_unit.to_string(),
        target_unit: target_unit.to_string(),
        verbose,
        regenerate_plant_mesh,
        dbnum,
        use_basic_materials: false,
        run_all_dbnos,
        split_by_site,
        include_negative,
        export_svg,
    }
}

/// 模型生成完成后同步缓存数据到 SurrealDB 的辅助函数
///
/// `debug_model_refnos`: 当指定时，仅同步这些 refno 的子孙节点数据（避免同步整个 cache）。
#[cfg(not(feature = "gui"))]
async fn sync_cache_to_db_if_enabled(
    sync_enabled: bool,
    db_option_ext: &aios_database::options::DbOptionExt,
    debug_model_refnos: Option<&[String]>,
) -> anyhow::Result<()> {
    if !sync_enabled {
        return Ok(());
    }

    // 确保数据库已连接
    init_surreal().await?;

    // 如果有 debug_model_refnos，收集子孙节点构建 refno_filter
    let refno_filter = if let Some(refno_strs) = debug_model_refnos {
        if !refno_strs.is_empty() {
            use aios_core::pdms_types::RefnoEnum;
            use std::str::FromStr;

            let roots: Vec<RefnoEnum> = refno_strs
                .iter()
                .filter_map(|s| {
                    let r = s.replace('_', "/");
                    RefnoEnum::from_str(&r).ok()
                })
                .collect();

            if !roots.is_empty() {
                println!(
                    "\n🗄️  --sync-to-db: 仅同步 debug-model 指定节点的子孙数据: {:?}",
                    refno_strs
                );
                // 查询子孙节点（包含自身）
                let descendants =
                    aios_core::collect_descendant_filter_ids(&roots, &[], None).await?;
                let mut filter: std::collections::HashSet<RefnoEnum> =
                    descendants.into_iter().collect();
                // 包含根节点自身
                filter.extend(roots.iter().copied());
                println!("   收集到 {} 个子孙 refno（含根节点）", filter.len());
                Some(filter)
            } else {
                println!("\n🗄️  --sync-to-db: 模型生成完成，开始同步缓存数据到 SurrealDB...");
                None
            }
        } else {
            println!("\n🗄️  --sync-to-db: 模型生成完成，开始同步缓存数据到 SurrealDB...");
            None
        }
    } else {
        println!("\n🗄️  --sync-to-db: 模型生成完成，开始同步缓存数据到 SurrealDB...");
        None
    };

    let cache_dir = db_option_ext.get_model_cache_dir();
    let flushed = aios_database::fast_model::cache_flush::flush_latest_instance_cache_to_surreal(
        &cache_dir,
        None, // 同步所有 dbnums（refno_filter 会在 merge 后精确过滤）
        true, // replace_exist = true，覆盖已有数据
        true, // verbose
        refno_filter.as_ref(),
    )
    .await?;

    println!(
        "✅ 数据同步完成：cache_dir={} flushed_dbnums={}",
        cache_dir.display(),
        flushed
    );

    Ok(())
}

/// 导入 .surql 并执行后处理：reconcile_missing_neg_relate / boolean / inst_relate_aabb。
#[cfg(not(feature = "gui"))]
async fn run_import_and_post_process(
    sql_path: &Path,
    db_option_ext: &aios_database::options::DbOptionExt,
) -> anyhow::Result<()> {
    if !sql_path.exists() {
        anyhow::bail!("--import-sql 文件不存在: {}", sql_path.display());
    }

    println!("\n🗂️  import-sql: 导入 {} 到 SurrealDB", sql_path.display());
    init_surreal().await?;
    println!("✅ 数据库连接成功");

    // Phase 2 Step 1: 初始化表结构
    println!("[import-sql] Phase 2.1: 初始化 inst_relate 表结构...");
    aios_core::rs_surreal::inst::init_model_tables().await?;

    // Phase 2 Step 2: 批量导入 SQL
    println!("[import-sql] Phase 2.2: 批量导入 SQL 语句...");
    let (success, failed) =
        aios_database::fast_model::gen_model::sql_file_writer::import_sql_file(sql_path, 500)
            .await?;
    if failed > 0 {
        eprintln!(
            "[import-sql] ⚠️ 导入存在失败: 成功={}, 失败={}",
            success, failed
        );
    }

    use aios_core::SurrealQueryExt;
    let sql = "SELECT value in FROM inst_relate;";
    let refnos: Vec<aios_core::RefnoEnum> =
        aios_core::SUL_DB.query_take(sql, 0).await.unwrap_or_default();

    // Phase 2 Step 3: reconcile_missing_neg_relate
    println!("[import-sql] Phase 2.3: reconcile_missing_neg_relate...");
    if !refnos.is_empty() {
        if let Err(e) =
            aios_database::fast_model::gen_model::pdms_inst::reconcile_missing_neg_relate(&refnos)
                .await
        {
            eprintln!("[import-sql] reconcile_missing_neg_relate 失败: {}", e);
        }
    }

    // Phase 2 Step 4: boolean worker
    if db_option_ext.inner.apply_boolean_operation {
        println!("[import-sql] Phase 2.4: 布尔运算...");
        if let Err(e) = aios_database::fast_model::mesh_generate::run_boolean_worker(
            std::sync::Arc::new(db_option_ext.inner.clone()),
            100,
        )
        .await
        {
            eprintln!("[import-sql] 布尔运算失败: {}", e);
        }
    }

    // Phase 2 Step 5: update aabb
    println!("[import-sql] Phase 2.5: 更新 inst_relate_aabb...");
    if !refnos.is_empty() {
        if let Err(e) = aios_database::fast_model::mesh_generate::update_inst_relate_aabbs_by_refnos(
            &refnos,
            db_option_ext.is_replace_mesh(),
        )
        .await
        {
            eprintln!("[import-sql] 更新 inst_relate_aabb 失败: {}", e);
        }
    }

    println!("✅ import-sql 全部完成");
    Ok(())
}

/// debug-model 流程的后置步骤：sync-to-db + export-dbnum-instances（parquet/json）
///
/// 将 sync + 导出合并为一个调用，避免 debug-model 分支中重复编写。
#[cfg(not(feature = "gui"))]
async fn post_export_steps(
    matches: &clap::ArgMatches,
    db_option_ext: &aios_database::options::DbOptionExt,
    debug_model_refnos: Option<&[String]>,
    verbose: bool,
) -> anyhow::Result<()> {
    // 1. sync-to-db
    sync_cache_to_db_if_enabled(
        matches.get_flag("sync-to-db"),
        db_option_ext,
        debug_model_refnos,
    )
    .await?;

    // 2. export-dbnum-instances-parquet / export-dbnum-instances-json
    let want_parquet = matches.get_flag("export-dbnum-instances-parquet")
        || matches.get_flag("export-dbnum-instances");
    let want_json = matches.get_flag("export-dbnum-instances-json");

    if !want_parquet && !want_json {
        return Ok(());
    }

    // 从 debug-model refno 推导 dbnum + root_refno
    use aios_core::pdms_types::RefnoEnum;
    use std::str::FromStr;

    let first_refno_str = debug_model_refnos
        .and_then(|v| v.first())
        .map(|s| s.replace('_', "/"));
    let root_refno: Option<RefnoEnum> = first_refno_str
        .as_deref()
        .and_then(|s| RefnoEnum::from_str(s).ok());

    // 自动推导 dbnum：优先 CLI --dbnum，否则从 refno 推导
    let dbnum = matches.get_one::<u32>("dbnum").copied().or_else(|| {
        root_refno.and_then(|r| aios_database::data_interface::db_meta().get_dbnum_by_refno(r))
    });

    let Some(dbnum) = dbnum else {
        eprintln!("⚠️  无法推导 dbnum，跳过 export-dbnum-instances");
        return Ok(());
    };

    let output_override = matches
        .get_one::<String>("output")
        .map(std::path::PathBuf::from);

    // 确保数据库已连接
    init_surreal().await?;

    #[cfg(feature = "parquet-export")]
    if want_parquet {
        let fill_missing_cache = matches.get_flag("fill-missing-cache");
        println!(
            "\n📦 后置步骤：从 cache 导出 dbnum={} 实例数据为 Parquet",
            dbnum
        );
        if fill_missing_cache {
            println!("   - 导出前策略: 自动补齐缺失 refno");
        } else {
            println!("   - 导出前策略: 仅导出已生成 cache（默认）");
        }
        crate::cli_modes::export_dbnum_instances_parquet_from_cache_mode(
            dbnum,
            verbose,
            output_override.clone(),
            db_option_ext,
            fill_missing_cache,
        )
        .await?;
    }

    if want_json {
        let from_cache = matches.get_flag("from-cache");
        let detailed = matches.get_flag("detailed");
        println!("\n📦 后置步骤：导出 dbnum={} 实例数据为 JSON", dbnum);
        crate::cli_modes::export_dbnum_instances_json_mode(
            dbnum,
            verbose,
            output_override,
            db_option_ext,
            true, // autorun
            root_refno,
            from_cache,
            detailed,
        )
        .await?;
    }

    Ok(())
}

#[cfg(all(not(feature = "gui"), feature = "grpc"))]
use crate::cli_modes::start_grpc_server_mode;
#[cfg(not(feature = "gui"))]
use crate::cli_modes::{
    ExportConfig, export_glb_mode, export_gltf_mode, export_model_mode, export_obj_mode,
    get_output_filename_for_refno,
};
#[cfg(not(feature = "gui"))]
use aios_core::geometry::csg::clear_ploop_debug_cache;
#[cfg(not(feature = "gui"))]
use aios_core::{DBType, init_surreal, query_mdb_db_nums};
#[cfg(feature = "gui")]
use aios_database::gui;
#[cfg(not(feature = "gui"))]
use aios_database::options::{MeshFormat, get_db_option_ext_from_path};
#[cfg(not(feature = "gui"))]
use aios_database::run_app;
#[cfg(not(feature = "gui"))]
use clap::{Arg, Command};
#[cfg(not(feature = "gui"))]
use std::process::{Command as StdCommand, Stdio};

#[cfg(feature = "gui")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    gui::run_gui();
    Ok(())
}

#[cfg(not(feature = "gui"))]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 默认把模型生成/导出过程的所有 stdout/stderr 写入日志文件，避免控制台刷屏导致“看似死循环”。
    // 仅在显式 `--verbose` 或后台服务模式（如 --grpc-server）时保留控制台输出。
    maybe_redirect_stdio_to_log_file();

    // #region agent log
    // 进程启动心跳：只要启用了 --debug-model（即 AIOS_LOG_FILE 存在）或显式 AIOS_AGENT_DEBUG，就写入 NDJSON，
    // 用于确认本次运行确实“能写到 debug.log”，避免出现“日志文件不存在”的情况。
    if std::env::var_os("AIOS_LOG_FILE").is_some()
        || std::env::var_os("AIOS_AGENT_DEBUG").is_some()
        || std::env::var_os("AIOS_AGENT_DEBUG_REFNO").is_some()
        || std::env::var_os("AIOS_AGENT_DEBUG_GEOM_REFNO").is_some()
    {
        agent_log(
            "H0",
            "main.rs:main",
            "startup",
            serde_json::json!({
                "AIOS_LOG_FILE": std::env::var("AIOS_LOG_FILE").ok(),
                "AIOS_AGENT_RUNID": std::env::var("AIOS_AGENT_RUNID").ok(),
                "AIOS_AGENT_DEBUG_REFNO": std::env::var("AIOS_AGENT_DEBUG_REFNO").ok(),
                "AIOS_AGENT_DEBUG_GEOM_REFNO": std::env::var("AIOS_AGENT_DEBUG_GEOM_REFNO").ok(),
            }),
        );
    }
    // #endregion

    let matches = Command::new("aios-database")
        .version("0.1.3")
        .about("AIOS Database Processing Tool")
        .arg(
            Arg::new("config")
                .long("config")
                .short('c')
                .help("Path to the configuration file (Without extension)")
                .value_name("CONFIG_PATH")
                .default_value("db_options/DbOption"),
        )
        .arg(
            Arg::new("gen-lod")
                .long("gen-lod")
                .help("Override mesh generation LOD level for this run (L0-L4). Defaults to db_options/DbOption.toml")
                .value_name("LOD")
                .value_parser(["L0", "L1", "L2", "L3", "L4"]),
        )
        .arg(
            Arg::new("grpc-server")
                .long("grpc-server")
                .help("Start GRPC server")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("grpc-port")
                .long("grpc-port")
                .help("GRPC server port")
                .value_name("PORT")
                .default_value("50051"),
        )
        .arg(
            Arg::new("debug-model")
                .long("debug-model")
                .help("Enable debug model output. Can optionally specify reference numbers (comma-separated)")
                .value_name("REFNOS")
                .value_delimiter(',')
                .num_args(0..),
        )
        .arg(
            Arg::new("debug-model-errors-only")
                .long("debug-model-errors-only")
                .help("Only log errors during model generation (reduces log verbosity)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("log-model-error")
                .long("log-model-error")
                .help("Record model generation errors for statistical analysis (automatically enables debug-model and errors-only mode)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("gen-indextree")
                .long("gen-indextree")
                .help("生成 indextree 文件。可选指定 dbnum，不指定则生成所有 DESI 类型")
                .value_name("DBNUM")
                .num_args(0..=1),
        )
        .arg(
            Arg::new("gen-all-desi-indextree")
                .long("gen-all-desi-indextree")
                .help("强制生成所有 DESI 类型的 indextree 文件（绕过配置文件中的 manual_db_nums 限制）")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("capture")
                .long("capture")
                .help("After model generation, export OBJ and capture screenshots (optionally provide output directory)")
                .value_name("DIR")
                .num_args(0..=1)
                .default_missing_value("output/screenshots"),
        )
        .arg(
            Arg::new("capture-width")
                .long("capture-width")
                .help("Screenshot width in pixels (default 800)")
                .value_name("PX")
                .value_parser(clap::value_parser!(u32))
                .requires("capture"),
        )
        .arg(
            Arg::new("capture-height")
                .long("capture-height")
                .help("Screenshot height in pixels (default 600)")
                .value_name("PX")
                .value_parser(clap::value_parser!(u32))
                .requires("capture"),
        )
        .arg(
            Arg::new("capture-views")
                .long("capture-views")
                .help("Extra camera views to render (>=1). When >1, saves `{basename}_viewXX.png` alongside `{basename}.png`")
                .value_name("N")
                .value_parser(clap::value_parser!(u8))
                .requires("capture"),
        )
        .arg(
            Arg::new("capture-include-descendants")
                .long("capture-include-descendants")
                .help("Include descendants when exporting OBJ for capture (default: true). You can pass `--capture-include-descendants=false` to disable.")
                // 兼容两种写法：
                // - 旧：`--capture-include-descendants`（无值）=> true
                // - 新：`--capture-include-descendants=true/false`
                .num_args(0..=1)
                .default_missing_value("true")
                .default_value("true")
                .value_parser(clap::value_parser!(bool))
                .requires("capture"),
        )
        .arg(
            Arg::new("capture-baseline")
                .long("capture-baseline")
                .help("Compare captured screenshots with baseline directory (expects same filename .png)")
                .value_name("DIR")
                .requires("capture"),
        )
        .arg(
            Arg::new("capture-diff")
                .long("capture-diff")
                .help("Output directory for diff images (default: <capture-dir>/diff)")
                .value_name("DIR")
                .requires("capture"),
        )
        .arg(
            Arg::new("export-obj")
                .long("export-obj")
                .help("Export OBJ model when using --debug-model")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-svg")
                .long("export-svg")
                .help("Export profile SVG when using --debug-model")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("regen-model")
                .long("regen-model")
                .help("Regenerate model data (forces replace_mesh mode). With export flags: regenerate first then export; without export flags: regenerate only")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("defer-db-write")
                .long("defer-db-write")
                .help("Defer DB writes: output all SQL to .surql files instead of writing to SurrealDB during model generation")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("import-sql")
                .long("import-sql")
                .help("Import a .surql file into SurrealDB and run post-processing (reconcile/boolean/aabb)")
                .value_name("PATH")
                .num_args(1),
        )
        .arg(
            Arg::new("flush-cache-to-db")
                .long("flush-cache-to-db")
                .help("Flush model instance_cache to SurrealDB (backup). Requires SurrealDB config in DbOption")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("flush-cache-dbnums")
                .long("flush-cache-dbnums")
                .help("Only flush specified dbnums (comma-separated, e.g. 1112,1113). Default: all dbnums in cache")
                .value_name("DBNUMS")
                .value_delimiter(',')
                .value_parser(clap::value_parser!(u32))
                .num_args(1..)
                .requires("flush-cache-to-db"),
        )
        .arg(
            Arg::new("flush-cache-replace")
                .long("flush-cache-replace")
                .help("When flushing cache to SurrealDB, delete/replace existing instance records (危险：会覆盖 DB 侧数据)")
                .action(clap::ArgAction::SetTrue)
                .requires("flush-cache-to-db"),
        )
        .arg(
            Arg::new("sync-to-db")
                .long("sync-to-db")
                .help("After model generation, sync cache data to SurrealDB (模型生成完成后同步数据到数据库)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-glb")
                .long("export-glb")
                .help("Export GLB model when using --debug-model")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-gltf")
                .long("export-gltf")
                .help("Export glTF model when using --debug-model")
                .action(clap::ArgAction::SetTrue),
        )
        // 新增独立导出命令 - 不启用调试模式
        .arg(
            Arg::new("export-obj-refnos")
                .long("export-obj-refnos")
                .help("Export OBJ model for specified reference numbers (comma-separated, no debug mode)")
                .value_name("REFNOS")
                .value_delimiter(',')
                .num_args(1..),
        )
        .arg(
            Arg::new("export-glb-refnos")
                .long("export-glb-refnos")
                .help("Export GLB model for specified reference numbers (comma-separated, no debug mode)")
                .value_name("REFNOS")
                .value_delimiter(',')
                .num_args(1..),
        )
        .arg(
            Arg::new("export-gltf-refnos")
                .long("export-gltf-refnos")
                .help("Export glTF model for specified reference numbers (comma-separated, no debug mode)")
                .value_name("REFNOS")
                .value_delimiter(',')
                .num_args(1..),
        )
        .arg(
            Arg::new("export-obj-output")
                .long("export-obj-output")
                .help("Output path for exported OBJ file (optional, defaults to PE name)")
                .value_name("OUTPUT_PATH"),
        )
        .arg(
            Arg::new("use-surrealdb")
                .long("use-surrealdb")
                .help("Force enable SurrealDB instances source / model-data writes for export/debug flows (default follows config)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("include-negative")
                .long("include-negative")
                .help("Include negative entities (Neg type geometries) in export")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-filter-nouns")
                .long("export-filter-nouns")
                .help("Filter by noun types (comma-separated, e.g., EQUI,PIPE,VALV)")
                .value_name("NOUNS")
                .value_delimiter(',')
                .num_args(0..),
        )
        .arg(
            Arg::new("export-include-descendants")
                .long("export-include-descendants")
                .help("Include all descendants of specified refnos")
                .value_name("BOOL")
                .default_value("true")
                .value_parser(clap::value_parser!(bool)),
        )
        .arg(
            Arg::new("export-format")
                .long("export-format")
                .help("Export format (obj, glb, gltf)")
                .value_name("FORMAT")
                .default_value("obj"),
        )
        .arg(
            Arg::new("dbnum")
                .long("dbnum")
                .help("Database number for export / model generation. When running gen_model, overrides manual_db_nums")
                .value_name("DBNO")
                .value_parser(clap::value_parser!(u32)),
        )
        .arg(
            Arg::new("root-refno")
                .long("root-refno")
                .help("Root refno for scoped export (e.g. 24381_145018 or 24381/145018)")
                .value_name("REFNO"),
        )
        .arg(
            Arg::new("gen-nouns")
                .long("gen-nouns")
                .help("Only generate specified noun types (comma-separated, e.g. BRAN,PANE). Overrides index_tree_enabled_target_types in DbOption")
                .value_name("NOUNS")
                .value_delimiter(',')
                .num_args(1..),
        )
        .arg(
            Arg::new("gen-limit-per-noun")
                .long("gen-limit-per-noun")
                .help("Limit max instances per noun type during generation (e.g. 50). 0 means unlimited.")
                .value_name("LIMIT")
                .value_parser(clap::value_parser!(usize)),
        )
        .arg(
            Arg::new("verbose")
                .long("verbose")
                .short('v')
                .help("Enable verbose output")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-source-unit")
                .long("export-source-unit")
                .help("Source unit for export (mm, cm, m, in, ft, yd)")
                .value_name("UNIT")
                .default_value("mm"),
        )
        .arg(
            Arg::new("export-target-unit")
                .long("export-target-unit")
                .help("Target unit for export (mm, cm, dm, m, in, ft, yd)")
                .value_name("UNIT")
                .default_value("mm"),
        )
        .arg(
            Arg::new("basic-materials")
                .long("basic-materials")
                .help("Use basic (unlit) materials instead of PBR when exporting GLB/GLTF")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("split-site")
                .long("split-site")
                .help("Split each SITE into separate files (default: merge all SITEs in the same dbnum)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("output")
                .long("output")
                .help("Override the export output directory (defaults vary by subcommand)")
                .value_name("DIR"),
        )
        .arg(
            Arg::new("export-refnos")
                .long("export-refnos")
                .help("Export only specified refnos (comma-separated, e.g., '24381_46959,24381_46960')")
                .value_name("REFNOS"),
        )
        .arg(
            Arg::new("export-all-relates")
                .long("export-all-relates")
                .help("Export all inst_relate entities in Prepack LOD format (按 zone 分组, 默认仅 L1)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-all-parquet")
                .long("export-all-parquet")
                .help("Export all inst_relate entities in Prepack LOD format with additional Parquet manifests (instances.parquet + geometry_manifest.parquet)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-dbnum-instances-json")
                .long("export-dbnum-instances-json")
                .help("Export dbnum instances as JSON (default: SurrealDB + compact format)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("from-cache")
                .long("from-cache")
                .help("Use model cache instead of SurrealDB for export")
                .action(clap::ArgAction::SetTrue)
                .requires("export-dbnum-instances-json"),
        )
        .arg(
            Arg::new("detailed")
                .long("detailed")
                .help("Export detailed JSON format with all fields (default: compact)")
                .action(clap::ArgAction::SetTrue)
                .requires("export-dbnum-instances-json"),
        )
        .arg(
            Arg::new("export-dbnum-instances-parquet")
                .long("export-dbnum-instances-parquet")
                .help("Export dbnum instances as multi-table Parquet (instances/geo_instances/tubings/transforms/aabb) for DuckDB querying")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("fill-missing-cache")
                .long("fill-missing-cache")
                .help("When exporting dbnum parquet from cache, auto-generate missing refnos before export (default: disabled)")
                .action(clap::ArgAction::SetTrue)
                .conflicts_with("from-surrealdb"),
        )
        .arg(
            Arg::new("from-surrealdb")
                .long("from-surrealdb")
                .help("Use SurrealDB as data source for parquet export (instead of model cache)")
                .action(clap::ArgAction::SetTrue)
                .conflicts_with("fill-missing-cache"),
        )
        .arg(
            Arg::new("export-pdms-tree-parquet")
                .long("export-pdms-tree-parquet")
                .help("Export PDMS TreeIndex + pe.name as Parquet (pdms_tree_{dbnum}.parquet) for DuckDB-WASM model tree queries")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-world-sites-parquet")
                .long("export-world-sites-parquet")
                .help("Export WORL->SITE nodes as Parquet (world_sites.parquet) for DuckDB-WASM (Full Parquet Mode)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-dbnum-instances")
                .long("export-dbnum-instances")
                .help("Export dbnum instances (default: Parquet format; use --export-dbnum-instances-json for JSON)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-room-instances")
                .long("export-room-instances")
                .help("Export room calculation results as JSON (room_relations.json + room_geometries.json)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("import-spatial-index")
                .long("import-spatial-index")
                .help("Import instances.json to SQLite spatial index")
                .value_name("JSON_PATH"),
        )
        .arg(
            Arg::new("spatial-index-output")
                .long("spatial-index-output")
                .help("Output path for SQLite spatial index (default: output/spatial_index.sqlite)")
                .value_name("SQLITE_PATH"),
        )
        .arg(
            Arg::new("export-all-lods")
                .long("export-all-lods")
                .help("Export all LOD levels (L1, L2, L3). Without this, only L1 is exported")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("owner-types")
                .long("owner-types")
                .help("Filter by owner_type (comma-separated, e.g., 'BRAN,HANG')")
                .value_name("TYPES"),
        )
        .arg(
            Arg::new("name-config")
                .long("name-config")
                .help("Excel file for name mapping (三维模型节点 -> PID对象)")
                .value_name("EXCEL_PATH"),
        )
        .arg(
            Arg::new("mesh-type")
                .long("mesh-type")
                .alias("mesh_type")
                .help("Mesh format to generate (pdmsmesh, glb, obj). Multiple values allowed.")
                .value_name("TYPE")
                .value_delimiter(',')
                .num_args(1..),
        )
        // ========== 房间计算命令 ==========
        .arg(
            Arg::new("room-compute")
                .long("room-compute")
                .help("Run room relation computation (build spatial relationships between rooms and components)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("room-keywords")
                .long("room-keywords")
                .help("Room keywords for filtering (comma-separated, e.g., '-RM,-ROOM')")
                .value_name("KEYWORDS")
                .value_delimiter(',')
                .num_args(1..),
        )
        .arg(
            Arg::new("room-force-rebuild")
                .long("room-force-rebuild")
                .help("Force rebuild all room relations (ignore existing data)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("room-db-nums")
                .long("room-db-nums")
                .help("Database numbers to process (comma-separated, e.g., '1112,1113')")
                .value_name("DB_NUMS")
                .value_delimiter(',')
                .num_args(1..),
        )
        .arg(
            Arg::new("room-refno-root")
                .long("room-refno-root")
                .help("Root refno for room calculation scope (e.g., '21491_10000'). Only rooms under this subtree will be processed.")
                .value_name("REFNO")
                .action(clap::ArgAction::Set),
        )
        // ========== pe_transform 刷新命令 ==========
        .arg(
            Arg::new("refresh-transform")
                .long("refresh-transform")
                .help("Refresh pe_transform cache for specified dbnums (comma-separated, e.g., '1112,1113')")
                .value_name("DB_NUMS")
                .value_delimiter(',')
                .num_args(1..),
        )
        // ========== MBD JSON 预生成 ==========
        .arg(
            Arg::new("export-mbd")
                .long("export-mbd")
                .help("预生成所有 BRAN/HANG 的 MBD 标注 JSON 文件（按 --dbnum 过滤，不传则全量）")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-mbd-refno")
                .long("export-mbd-refno")
                .help("预生成指定 refno 及其子孙 BRAN/HANG 的 MBD 标注 JSON 文件")
                .value_name("REFNO"),
        )
        .get_matches();

    // 获取配置文件路径
    let config_path = matches
        .get_one::<String>("config")
        .expect("default value ensures this exists");

    // 设置环境变量，让 rs-core 库使用正确的配置文件
    unsafe {
        std::env::set_var("DB_OPTION_FILE", config_path);
    }

    // 预先初始化 OnceCell，避免后续第一次 get_db_option() 时覆盖 active_precision
    let _ = aios_core::get_db_option();

    let export_all_lods = matches.get_flag("export-all-lods");
    unsafe {
        if export_all_lods {
            std::env::set_var("EXPORT_ALL_LODS", "true");
        } else {
            std::env::remove_var("EXPORT_ALL_LODS");
        }
    }

    // 创建自定义的 DbOptionExt
    let mut db_option_ext = get_db_option_ext_from_path(config_path)?;

    if let Some(lod_str) = matches.get_one::<String>("gen-lod").map(|s| s.as_str()) {
        if let Some(lod) = parse_lod_level(lod_str) {
            println!(
                "🔧 CLI 覆盖 default_lod: {:?} -> {:?}",
                db_option_ext.inner.mesh_precision.default_lod, lod
            );
            db_option_ext.inner.mesh_precision.default_lod = lod;
        }
    }

    if let Some(mesh_types) = matches.get_many::<String>("mesh-type") {
        let mut formats = Vec::new();
        for mt in mesh_types {
            match mt.to_lowercase().as_str() {
                "pdmsmesh" | "mesh" => formats.push(MeshFormat::PdmsMesh),
                "glb" => formats.push(MeshFormat::Glb),
                "obj" => formats.push(MeshFormat::Obj),
                _ => println!("⚠️ 忽略未知的网格格式: {}", mt),
            }
        }
        if !formats.is_empty() {
            println!(
                "🔧 CLI 覆盖 mesh_formats: {:?} -> {:?}",
                db_option_ext.mesh_formats, formats
            );
            db_option_ext.mesh_formats = formats;
        }
    }

    // CLI 覆盖：模型生成的 dbnum / noun 类型（无需修改 DbOption.toml）
    if let Some(dbnum) = matches.get_one::<u32>("dbnum").copied() {
        db_option_ext.inner.manual_db_nums = Some(vec![dbnum]);
        println!("🔧 CLI 覆盖 manual_db_nums -> [{}]", dbnum);
    }
    if let Some(nouns) = matches.get_many::<String>("gen-nouns") {
        let v: Vec<String> = nouns.map(|s| s.to_uppercase()).collect();
        if !v.is_empty() {
            println!(
                "🔧 CLI 覆盖 index_tree_enabled_target_types: {:?} -> {:?}",
                db_option_ext.index_tree_enabled_target_types, v
            );
            db_option_ext.index_tree_enabled_target_types = v;
        }
    }
    if let Some(limit) = matches.get_one::<usize>("gen-limit-per-noun").copied() {
        let override_limit = if limit == 0 { None } else { Some(limit) };
        println!(
            "🔧 CLI 覆盖 index_tree_debug_limit_per_target_type: {:?} -> {:?}",
            db_option_ext.index_tree_debug_limit_per_target_type, override_limit
        );
        db_option_ext.index_tree_debug_limit_per_target_type = override_limit;
    }

    // 同步精度配置到 rs-core 全局 active_precision，保证布尔/导出等逻辑使用同一套 LOD
    aios_core::mesh_precision::set_active_precision(db_option_ext.inner.mesh_precision.clone());

    // ========== import-sql：导入 .surql 文件并执行后处理 ==========
    if let Some(sql_path) = matches.get_one::<String>("import-sql") {
        let path = std::path::Path::new(sql_path);
        run_import_and_post_process(path, &db_option_ext).await?;
        return Ok(());
    }

    // ========== cache -> SurrealDB：一键备份落库 ==========
    if matches.get_flag("flush-cache-to-db") {
        println!("\n🗄️  flush-cache-to-db: 将 model instance_cache 写入 SurrealDB（备份）");
        init_surreal().await?;
        println!("✅ 数据库连接成功");

        let cache_dir = db_option_ext.get_model_cache_dir();
        let dbnums: Option<Vec<u32>> = matches
            .get_many::<u32>("flush-cache-dbnums")
            .map(|v| v.copied().collect());
        let replace_exist = matches.get_flag("flush-cache-replace");

        let flushed =
            aios_database::fast_model::cache_flush::flush_latest_instance_cache_to_surreal(
                &cache_dir,
                dbnums.as_deref(),
                replace_exist,
                true,
                None, // 全量备份，不按 refno 过滤
            )
            .await?;

        println!(
            "✅ flush-cache-to-db 完成：cache_dir={} flushed_dbnums={}",
            cache_dir.display(),
            flushed
        );
        return Ok(());
    }



    // ========== MBD JSON 预生成 ==========
    #[cfg(feature = "web_server")]
    if matches.get_flag("export-mbd") || matches.get_one::<String>("export-mbd-refno").is_some() {
        use aios_database::web_api::{MbdExportScope, export_mbd_json_batch, get_mbd_output_dir};

        init_surreal().await?;
        let output_dir = get_mbd_output_dir();

        let scope = if let Some(refno_str) = matches.get_one::<String>("export-mbd-refno") {
            use aios_core::pdms_types::RefnoEnum;
            use std::str::FromStr;
            let refno_str = refno_str.replace('_', "/");
            let refno = RefnoEnum::from_str(&refno_str)
                .map_err(|e| anyhow::anyhow!("无效的 refno '{}': {e}", refno_str))?;
            println!("🎯 MBD 预生成：指定 refno={} 及其子孙 BRAN/HANG", refno);
            MbdExportScope::ByRefno(refno)
        } else if let Some(dbnum) = matches.get_one::<u32>("dbnum").copied() {
            println!("🎯 MBD 预生成：dbnum={} 下所有 BRAN/HANG", dbnum);
            MbdExportScope::ByDbnum(dbnum)
        } else {
            println!("🎯 MBD 预生成：全量 BRAN/HANG");
            MbdExportScope::AllDbnums
        };

        let stats = export_mbd_json_batch(&output_dir, scope).await?;
        println!(
            "✅ MBD 预生成完成：{}/{} 成功，输出目录 {}",
            stats.success, stats.total, stats.output_dir
        );
        return Ok(());
    }

    // 调试：显示配置加载结果
    println!("🔧 配置加载完成:");
    println!("   - 配置文件路径: {}", config_path);
    println!(
        "   - index_tree_enabled_target_types: {:?}",
        db_option_ext.index_tree_enabled_target_types
    );
    println!(
        "   - index_tree_excluded_target_types: {:?}",
        db_option_ext.index_tree_excluded_target_types
    );

    println!("✅ IndexTree 默认生成管线已启用（无模式开关）");
    let config_debug_refnos: Option<Vec<String>> = db_option_ext.inner.debug_model_refnos.clone();
    let log_model_error = matches.get_flag("log-model-error");
    let debug_model_requested = matches.contains_id("debug-model") || log_model_error;
    let debug_model_errors_only = matches.get_flag("debug-model-errors-only") || log_model_error;

    if log_model_error {
        println!("📊 启用模型错误记录模式（自动开启 debug-model + errors-only）");
    }

    if !debug_model_requested && db_option_ext.inner.debug_model_refnos.is_some() {
        println!("ℹ️ 未开启调试模式，本次运行将忽略配置中的 debug_model_refnos");
    }
    if !debug_model_requested {
        aios_core::set_debug_model_enabled(false);
        db_option_ext.inner.debug_model_refnos = None;
    }

    // 设置错误日志模式
    if debug_model_errors_only {
        aios_database::fast_model::set_debug_model_errors_only(true);
        if !log_model_error {
            println!("✅ 启用仅错误日志模式");
        }
    }

    // 获取通用参数
    let output_path = matches.get_one::<String>("export-obj-output").cloned();
    let filter_nouns: Option<Vec<String>> = matches
        .get_many::<String>("export-filter-nouns")
        .map(|nouns| nouns.map(|s| s.to_string()).collect());
    let include_descendants = matches
        .get_one::<bool>("export-include-descendants")
        .copied()
        .unwrap_or(true);
    let verbose = matches.get_flag("verbose");
    let use_basic_materials = matches.get_flag("basic-materials");

    // 获取单位转换参数
    let source_unit = matches
        .get_one::<String>("export-source-unit")
        .unwrap()
        .as_str();
    let target_unit = matches
        .get_one::<String>("export-target-unit")
        .unwrap()
        .as_str();

    // 获取 dbnum 参数（用于按 SITE 导出）
    let dbnum = matches.get_one::<u32>("dbnum").copied();

    // 获取 split-site 参数（默认合并，有此参数才拆分）
    let split_by_site = matches.get_flag("split-site");

    // 获取 include-negative 参数（是否包含负实体）
    let include_negative = matches.get_flag("include-negative");

    let capture_dir = matches.get_one::<String>("capture").cloned();
    let capture_width = matches
        .get_one::<u32>("capture-width")
        .copied()
        .unwrap_or(1200);
    let capture_height = matches
        .get_one::<u32>("capture-height")
        .copied()
        .unwrap_or(900);
    let capture_views = matches.get_one::<u8>("capture-views").copied().unwrap_or(1);
    // 截图链路默认包含子孙节点（与导出默认语义一致），否则像 BRAN/HANG 这类“几何主要在子孙节点/关联表”时
    // 会只截到一小段 TUBI，从而误判“导出管道不对”。
    let capture_include_descendants = matches
        .get_one::<bool>("capture-include-descendants")
        .copied()
        .unwrap_or(true);
    let capture_baseline_dir = matches.get_one::<String>("capture-baseline").cloned();
    let capture_diff_dir = matches.get_one::<String>("capture-diff").cloned();

    if let Some(ref dir) = capture_dir {
        let output_dir = PathBuf::from(dir.clone());
        aios_database::fast_model::set_capture_config(Some(
            aios_database::fast_model::CaptureConfig::new(
                output_dir,
                capture_width,
                capture_height,
                capture_include_descendants,
                capture_views,
                capture_baseline_dir.map(PathBuf::from),
                capture_diff_dir.map(PathBuf::from),
            ),
        ));
    } else {
        aios_database::fast_model::set_capture_config(None);
    }

    // ========== 首先处理 --debug-model 参数（必须在所有导出逻辑之前） ==========
    let debug_model_refnos: Option<Vec<String>> = if debug_model_requested {
        aios_core::set_debug_model_enabled(true);
        clear_ploop_debug_cache(); // 清理PLOOP调试文件缓存，允许重新生成
        println!("✅ 已启用 debug_model 调试信息打印");

        if !db_option_ext.inner.gen_mesh {
            println!("🔄 调试模式自动开启 gen_mesh");
            db_option_ext.inner.gen_mesh = true;
        }

        // 确保 gen_model 也被启用，以便 is_gen_mesh_or_model() 返回 true
        if !db_option_ext.inner.gen_model {
            println!("🔄 调试模式自动开启 gen_model");
            db_option_ext.inner.gen_model = true;
        }

        let cli_refnos: Vec<String> = matches
            .get_many::<String>("debug-model")
            .map(|values| values.map(|s| s.to_string()).collect())
            .unwrap_or_else(Vec::new);

        if !cli_refnos.is_empty() {
            println!("🔍 使用命令行指定的 debug-model 参考号: {:?}", cli_refnos);
            db_option_ext.inner.debug_model_refnos = Some(cli_refnos.clone());
            Some(cli_refnos)
        } else if let Some(config_refnos) = config_debug_refnos.as_ref() {
            if config_refnos.is_empty() {
                println!("💡 仅启用调试模式，未指定参考号");
                db_option_ext.inner.debug_model_refnos = Some(Vec::new());
                None
            } else {
                println!(
                    "🗂️ 使用配置文件中的 debug_model_refnos: {:?}",
                    config_refnos
                );
                db_option_ext.inner.debug_model_refnos = Some(config_refnos.clone());
                Some(config_refnos.clone())
            }
        } else {
            println!("💡 仅启用调试模式，未指定参考号");
            db_option_ext.inner.debug_model_refnos = None;
            None
        }
    } else {
        None
    };

    if debug_model_requested {
        db_option_ext.inner.enable_log = true;
        let now = Local::now();
        let log_refno = debug_model_refnos
            .as_ref()
            .and_then(|refnos| refnos.first().map(|s| s.as_str()))
            .unwrap_or("debug");
        let log_filename = format!(
            "logs/{}_{}-{:02}-{:02}_{:02}-{:02}-{:02}.log",
            log_refno,
            now.year(),
            now.month(),
            now.day(),
            now.hour(),
            now.minute(),
            now.second()
        );
        unsafe {
            std::env::set_var("AIOS_LOG_FILE", log_filename);
        }
        aios_database::init_logging(true);
    }

    // ========== 处理 --gen-all-desi-indextree 参数 ==========
    if matches.get_flag("gen-all-desi-indextree") {
        println!("🔄 生成所有 DESI 类型的 indextree (忽略 manual_db_nums)...");
        aios_database::data_interface::db_meta_manager::generate_desi_indextree(true)?;
        println!("✅ indextree 生成完成");
        return Ok(());
    }

    // ========== 处理 --gen-indextree 参数 ==========
    if matches.contains_id("gen-indextree") {
        let dbnum: Option<u32> = matches
            .get_one::<String>("gen-indextree")
            .and_then(|s| s.parse().ok());

        if let Some(dbnum) = dbnum {
            println!("🔄 生成指定 dbnum={} 的 indextree...", dbnum);
            aios_database::data_interface::db_meta_manager::generate_single_indextree(dbnum)?;
        } else {
            println!("🔄 生成所有 DESI 类型的 indextree...");
            aios_database::data_interface::db_meta_manager::generate_desi_indextree(false)?;
        }
        println!("✅ indextree 生成完成");
        return Ok(());
    }

    // ========== 处理 --regen-model 参数 ==========
    let regen_model_requested = matches.get_flag("regen-model");
    if regen_model_requested {
        println!("🔄 检测到 --regen-model 参数，强制开启 replace_mesh 模式");
        // 与 replace_mesh 配合：强制 mesh_worker 忽略 mesh_sig 缓存，确保本次能看到最新代码/配置效果。
        unsafe {
            std::env::set_var("FORCE_REGEN_MESH", "1");
        }
        db_option_ext.inner.replace_mesh = Some(true);
        // 元件库(cata_neg)/设计型负实体导出依赖布尔结果（CatePos），因此 regen-model 必须开启布尔运算。
        if !db_option_ext.inner.apply_boolean_operation {
            println!("🔄 --regen-model 自动开启 apply_boolean_operation（生成 CatePos 布尔结果）");
            db_option_ext.inner.apply_boolean_operation = true;
        }
    }

    // --defer-db-write：模型生成阶段不写 SurrealDB，SQL 输出到 .surql 文件
    let defer_db_write_explicit = matches.get_flag("defer-db-write");
    if defer_db_write_explicit {
        println!("🗂️ 检测到 --defer-db-write 参数，模型生成阶段将跳过 SurrealDB 写入，SQL 输出到 .surql 文件");
        db_option_ext.defer_db_write = true;
    }

    // 调试模式下，如果配置开启了 gen_mesh，默认也应强制重新生成 mesh
    if debug_model_requested && db_option_ext.inner.gen_mesh {
        if db_option_ext.inner.replace_mesh != Some(true) {
            println!("🔄 调试模式启用 gen_mesh，默认开启 replace_mesh 以重新生成模型数据");
        }
        db_option_ext.inner.replace_mesh = Some(true);
    }

    // 模型导出请求：默认只导出不触发生成；仅当显式 --regen-model 才前置生成。
    let model_export_requested = matches.get_flag("export-obj")
        || matches.get_flag("export-svg")
        || matches.get_flag("export-glb")
        || matches.get_flag("export-gltf")
        || matches.contains_id("export-obj-refnos")
        || matches.contains_id("export-glb-refnos")
        || matches.contains_id("export-gltf-refnos")
        || (debug_model_requested && capture_dir.is_some());
    let any_export_requested = model_export_requested
        || matches.get_flag("export-all-parquet")
        || matches.get_flag("export-all-relates")
        || matches.get_flag("export-dbnum-instances-json")
        || matches.get_flag("export-dbnum-instances-parquet")
        || matches.get_flag("export-dbnum-instances")
        || matches.get_flag("export-room-instances")
        || matches.get_flag("export-pdms-tree-parquet")
        || matches.get_flag("export-world-sites-parquet");

    // ========== 仅在显式 --regen-model 时执行生成 ==========
    if regen_model_requested {
        // 确定 regen 的目标 refnos：优先 debug-model 指定的 refnos，其次 CLI 独立 refno 参数，
        // 再次 dbnum（查询所有 SITE），最后全库模式。
        let regen_refnos_vec: Vec<String> = if let Some(ref refnos) = debug_model_refnos {
            refnos.clone()
        } else if let Some(refnos) = matches.get_many::<String>("export-obj-refnos") {
            refnos.map(|s| s.to_string()).collect()
        } else if let Some(refnos) = matches.get_many::<String>("export-glb-refnos") {
            refnos.map(|s| s.to_string()).collect()
        } else if let Some(refnos) = matches.get_many::<String>("export-gltf-refnos") {
            refnos.map(|s| s.to_string()).collect()
        } else {
            vec![]
        };
        let regen_config = build_export_config(
            regen_refnos_vec,
            None,
            None,
            include_descendants,
            source_unit,
            target_unit,
            verbose,
            false,
            dbnum,
            split_by_site,
            include_negative,
            false,
        );
        let regen_result = cli_modes::run_regen_model(&regen_config, &db_option_ext).await?;

        // 仅在“重建 + 导出”时，defer 模式自动导入 SQL 后再导出，避免导出读到旧数据。
        if any_export_requested {
            if let Some(sql_path) = regen_result.deferred_sql_path.as_deref() {
                println!(
                    "🗂️ 检测到 defer_db_write 产物，开始自动导入并后处理: {}",
                    sql_path.display()
                );
                run_import_and_post_process(sql_path, &db_option_ext).await?;
                db_option_ext.defer_db_write = false;
            }
        } else {
            println!("✅ --regen-model 单独执行完成（未请求导出，流程到此结束）");
            return Ok(());
        }
    }

    // ========== 模型导出：默认沿用配置，--use-surrealdb 可强制切到 SurrealDB ==========
    //
    // 约定：
    // - 默认（不传 --use-surrealdb）时，沿用 DbOption 中 use_cache/use_surrealdb；
    // - 传 --use-surrealdb 时，强制使用 SurrealDB（并关闭 cache）；
    // - 这不是 fallback：cache 与 surrealdb 两条路径同时存在仅用于验证准确性。
    if model_export_requested {
        if matches.get_flag("use-surrealdb") {
            // SurrealDB-only：用于对照验证，避免与 cache 混用
            db_option_ext.use_surrealdb = true;
            db_option_ext.use_cache = false;
        }

        if !db_option_ext.use_cache && !db_option_ext.use_surrealdb {
            anyhow::bail!(
                "模型导出时 use_cache/use_surrealdb 同时为 false，请在配置中启用其一，或添加 --use-surrealdb"
            );
        }

        if !db_option_ext.use_surrealdb {
            println!(
                "📦 cache-only：OBJ 导出默认使用 indextree + model 缓存（instances 从缓存读，不写入 inst_*）。SurrealDB 仍作为输入数据源连接（PE/属性/世界矩阵等）。如需写入/对照验证，请添加 --use-surrealdb。"
            );
        }
    }

    // ========== 处理 --debug-model 与导出标志的组合 ==========
    if let Some(refnos_vec) = &debug_model_refnos {
        // 如果用户开启了 --capture 但没有指定任何导出标志，则默认走 OBJ 导出流程。
        // 这样可保证 `--debug-model ... --capture ...` 行为稳定：统一复用导出链路收集几何并触发截图。
        if capture_dir.is_some()
            && !matches.get_flag("export-obj")
            && !matches.get_flag("export-svg")
            && !matches.get_flag("export-glb")
            && !matches.get_flag("export-gltf")
        {
            println!(
                "📸 调试模式 + 截图模式：生成模型并截图（默认走 OBJ 导出流程）: {:?}",
                refnos_vec
            );
            let config = build_export_config(
                refnos_vec.clone(),
                output_path,
                filter_nouns,
                capture_include_descendants,
                source_unit,
                target_unit,
                verbose,
                false, // regen-model 已在导出前集中处理
                None,
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            let result = export_obj_mode(config, &db_option_ext).await;
            post_export_steps(
                &matches,
                &db_option_ext,
                debug_model_refnos.as_deref(),
                verbose,
            )
            .await?;
            return result;
        }

        // 检查是否有导出标志
        if matches.get_flag("export-obj") {
            println!("🎯 导出 OBJ 模型 (调试模式): {:?}", refnos_vec);

            // debug-model + export-obj 时自动启用截图（如果用户没有显式指定 --capture）
            if capture_dir.is_none() {
                let auto_capture_dir = db_option_ext.get_project_output_dir().join("screenshots");
                println!("📸 自动启用截图: {}", auto_capture_dir.display());
                aios_database::fast_model::set_capture_config(Some(
                    aios_database::fast_model::CaptureConfig::new(
                        auto_capture_dir,
                        capture_width,
                        capture_height,
                        include_descendants,
                        capture_views,
                        None,
                        None,
                    ),
                ));
            }

            let config = build_export_config(
                refnos_vec.clone(),
                output_path,
                filter_nouns,
                include_descendants,
                source_unit,
                target_unit,
                verbose,
                false, // regen-model 已在导出前集中处理
                None,
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            let result = export_obj_mode(config, &db_option_ext).await;
            post_export_steps(
                &matches,
                &db_option_ext,
                debug_model_refnos.as_deref(),
                verbose,
            )
            .await?;
            return result;
        }

        if matches.get_flag("export-svg") {
            println!("🎯 导出 SVG 截面 (调试模式): {:?}", refnos_vec);
            let config = build_export_config(
                refnos_vec.clone(),
                output_path,
                filter_nouns,
                include_descendants,
                source_unit,
                target_unit,
                verbose,
                false, // regen-model 已在导出前集中处理
                None,
                split_by_site,
                include_negative,
                true, // export_svg = true
            );
            let result = export_obj_mode(config, &db_option_ext).await;
            post_export_steps(
                &matches,
                &db_option_ext,
                debug_model_refnos.as_deref(),
                verbose,
            )
            .await?;
            return result;
        }

        if matches.get_flag("export-glb") {
            println!("🎯 导出 GLB 模型 (调试模式): {:?}", refnos_vec);
            let mut config = build_export_config(
                refnos_vec.clone(),
                output_path,
                filter_nouns,
                include_descendants,
                source_unit,
                target_unit,
                verbose,
                false, // regen-model 已在导出前集中处理
                None,
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            config.use_basic_materials = use_basic_materials;
            let result = export_glb_mode(config, &db_option_ext).await;
            post_export_steps(
                &matches,
                &db_option_ext,
                debug_model_refnos.as_deref(),
                verbose,
            )
            .await?;
            return result;
        }

        if matches.get_flag("export-gltf") {
            println!("🎯 导出 glTF 模型 (调试模式): {:?}", refnos_vec);
            let mut config = build_export_config(
                refnos_vec.clone(),
                output_path,
                filter_nouns,
                include_descendants,
                source_unit,
                target_unit,
                verbose,
                false, // regen-model 已在导出前集中处理
                None,
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            config.use_basic_materials = use_basic_materials;
            let result = export_gltf_mode(config, &db_option_ext).await;
            post_export_steps(
                &matches,
                &db_option_ext,
                debug_model_refnos.as_deref(),
                verbose,
            )
            .await?;
            return result;
        }
    }

    // ========== 然后处理导出命令 ==========
    // 首先处理带 dbnum 的导出命令（查询所有 SITE 并分别导出）
    if let Some(dbnum) = dbnum {
        if matches.get_flag("export-obj") {
            println!("🎯 导出 OBJ 模型 (按 dbnum={} 的所有 SITE):", dbnum);
            let config = build_export_config(
                vec![], // 不传 refnos，由 dbnum 自动查询 SITE
                output_path,
                filter_nouns,
                include_descendants,
                source_unit,
                target_unit,
                verbose,
                false, // regen-model 已在导出前集中处理
                Some(dbnum),
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            let result = export_obj_mode(config, &db_option_ext).await;
            post_export_steps(
                &matches,
                &db_option_ext,
                debug_model_refnos.as_deref(),
                verbose,
            )
            .await?;
            return result;
        }

        if matches.get_flag("export-glb") {
            println!("🎯 导出 GLB 模型 (按 dbnum={} 的所有 SITE):", dbnum);
            let mut config = build_export_config(
                vec![],
                output_path,
                filter_nouns,
                include_descendants,
                source_unit,
                target_unit,
                verbose,
                false, // regen-model 已在导出前集中处理
                Some(dbnum),
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            config.use_basic_materials = use_basic_materials;
            let result = export_glb_mode(config, &db_option_ext).await;
            post_export_steps(
                &matches,
                &db_option_ext,
                debug_model_refnos.as_deref(),
                verbose,
            )
            .await?;
            return result;
        }

        if matches.get_flag("export-gltf") {
            println!("🎯 导出 glTF 模型 (按 dbnum={} 的所有 SITE):", dbnum);
            let mut config = build_export_config(
                vec![],
                output_path,
                filter_nouns,
                include_descendants,
                source_unit,
                target_unit,
                verbose,
                false, // regen-model 已在导出前集中处理
                Some(dbnum),
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            config.use_basic_materials = use_basic_materials;
            let result = export_gltf_mode(config, &db_option_ext).await;
            post_export_steps(
                &matches,
                &db_option_ext,
                debug_model_refnos.as_deref(),
                verbose,
            )
            .await?;
            return result;
        }
    }

    // no-dbnum 情况的默认"全库导出"由各导出模式内部处理（config.run_all_dbnos）

    // 然后处理独立的导出命令（不启用调试模式）
    if let Some(refnos) = matches.get_many::<String>("export-obj-refnos") {
        let refnos_vec: Vec<String> = refnos.map(|s| s.to_string()).collect();
        if !refnos_vec.is_empty() {
            println!("🎯 导出 OBJ 模型 (非调试模式): {:?}", refnos_vec);
            let config = build_export_config(
                refnos_vec,
                output_path,
                filter_nouns,
                include_descendants,
                source_unit,
                target_unit,
                verbose,
                false, // regen-model 已在导出前集中处理
                None,
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            let result = export_obj_mode(config, &db_option_ext).await;
            post_export_steps(
                &matches,
                &db_option_ext,
                debug_model_refnos.as_deref(),
                verbose,
            )
            .await?;
            return result;
        }
    }

    if let Some(refnos) = matches.get_many::<String>("export-glb-refnos") {
        let refnos_vec: Vec<String> = refnos.map(|s| s.to_string()).collect();
        if !refnos_vec.is_empty() {
            println!("🎯 导出 GLB 模型 (非调试模式): {:?}", refnos_vec);
            let mut config = build_export_config(
                refnos_vec,
                output_path,
                filter_nouns,
                include_descendants,
                source_unit,
                target_unit,
                verbose,
                false, // GLB 不需要 regenerate_plant_mesh
                None,
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            config.use_basic_materials = use_basic_materials;
            let result = export_glb_mode(config, &db_option_ext).await;
            post_export_steps(
                &matches,
                &db_option_ext,
                debug_model_refnos.as_deref(),
                verbose,
            )
            .await?;
            return result;
        }
    }

    if let Some(refnos) = matches.get_many::<String>("export-gltf-refnos") {
        let refnos_vec: Vec<String> = refnos.map(|s| s.to_string()).collect();
        if !refnos_vec.is_empty() {
            println!("🎯 导出 glTF 模型 (非调试模式): {:?}", refnos_vec);
            let mut config = build_export_config(
                refnos_vec,
                output_path,
                filter_nouns,
                include_descendants,
                source_unit,
                target_unit,
                verbose,
                false, // glTF 不需要 regenerate_plant_mesh
                None,
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            config.use_basic_materials = use_basic_materials;
            let result = export_gltf_mode(config, &db_option_ext).await;
            post_export_steps(
                &matches,
                &db_option_ext,
                debug_model_refnos.as_deref(),
                verbose,
            )
            .await?;
            return result;
        }
    }

    // ========== 处理单独的导出标志（无 dbnum、无 refnos 时默认全库导出） ==========
    // 这是兜底逻辑：如果前面的条件都没匹配，说明用户只设置了导出标志

    if matches.get_flag("export-gltf") {
        println!("🎯 导出 glTF 模型 (全库模式 - MDB 所有 dbnum)");
        let config = ExportConfig::build_for_all_dbnos(
            output_path,
            filter_nouns,
            include_descendants,
            source_unit.to_string(),
            target_unit.to_string(),
            verbose,
            false, // regen-model 已在导出前集中处理
            use_basic_materials,
            split_by_site,
            include_negative,
            matches.get_flag("export-svg"),
        );
        let result = export_gltf_mode(config, &db_option_ext).await;
        post_export_steps(
            &matches,
            &db_option_ext,
            debug_model_refnos.as_deref(),
            verbose,
        )
        .await?;
        return result;
    }

    if matches.get_flag("export-glb") {
        println!("🎯 导出 GLB 模型 (全库模式 - MDB 所有 dbnum)");
        let config = ExportConfig::build_for_all_dbnos(
            output_path,
            filter_nouns,
            include_descendants,
            source_unit.to_string(),
            target_unit.to_string(),
            verbose,
            false, // regen-model 已在导出前集中处理
            use_basic_materials,
            split_by_site,
            include_negative,
            matches.get_flag("export-svg"),
        );
        let result = export_glb_mode(config, &db_option_ext).await;
        post_export_steps(
            &matches,
            &db_option_ext,
            debug_model_refnos.as_deref(),
            verbose,
        )
        .await?;
        return result;
    }

    if matches.get_flag("export-obj") {
        println!("🎯 导出 OBJ 模型 (全库模式 - MDB 所有 dbnum)");
        let config = ExportConfig::build_for_all_dbnos(
            output_path,
            filter_nouns,
            include_descendants,
            source_unit.to_string(),
            target_unit.to_string(),
            verbose,
            false, // regen-model 已在导出前集中处理
            use_basic_materials,
            split_by_site,
            include_negative,
            matches.get_flag("export-svg"),
        );
        let result = export_obj_mode(config, &db_option_ext).await;
        post_export_steps(
            &matches,
            &db_option_ext,
            debug_model_refnos.as_deref(),
            verbose,
        )
        .await?;
        return result;
    }

    if matches.get_flag("export-all-parquet") {
        use crate::cli_modes::export_all_parquet_mode;

        let dbnum = matches.get_one::<u32>("dbnum").copied();
        let export_bundle_dir = matches.get_one::<String>("output").map(PathBuf::from);
        let export_all_lods = matches.get_flag("export-all-lods");
        let export_refnos = matches.get_one::<String>("export-refnos").cloned();
        let owner_types: Option<Vec<String>> = matches
            .get_one::<String>("owner-types")
            .map(|s| s.split(',').map(|t| t.trim().to_uppercase()).collect());
        let name_config_path = matches.get_one::<String>("name-config").map(PathBuf::from);

        println!("🎯 导出 inst_relate 实体 (Prepack LOD + Parquet)");
        if let Some(ref refnos) = export_refnos {
            println!("   - 🎯 仅导出指定 refnos={}", refnos);
        } else if let Some(dbnum) = dbnum {
            println!("   - 按 dbnum={} 过滤", dbnum);
        } else {
            println!("   - 全表扫描（所有 dbnum）");
        }
        if let Some(ref types) = owner_types {
            println!("   - 按 owner_type 过滤: {:?}", types);
        }
        if let Some(ref path) = name_config_path {
            println!("   - 名称配置文件: {}", path.display());
        }

        return export_all_parquet_mode(
            dbnum,
            verbose,
            export_bundle_dir,
            owner_types,
            name_config_path,
            export_all_lods,
            export_refnos,
            source_unit.to_string(),
            target_unit.to_string(),
            &db_option_ext,
        )
        .await;
    }

    if matches.get_flag("export-dbnum-instances-json") {
        use crate::cli_modes::export_dbnum_instances_json_mode;
        use aios_core::pdms_types::RefnoEnum;
        use std::str::FromStr;

        let dbnum = matches.get_one::<u32>("dbnum").copied();
        let export_bundle_dir = matches.get_one::<String>("output").map(PathBuf::from);

        // 解析 --debug-model 参数作为 root_refno
        let root_refno: Option<RefnoEnum> = matches
            .get_many::<String>("debug-model")
            .and_then(|values| values.into_iter().next())
            .and_then(|s| {
                let refno_str = s.replace('_', "/");
                RefnoEnum::from_str(&refno_str).ok()
            });

        // 必须提供 dbnum 参数
        let dbnum = match dbnum {
            Some(n) => n,
            None => {
                eprintln!("❌ 错误: --export-dbnum-instances-json 需要提供 --dbnum 参数");
                eprintln!("   例如: cargo run -- --export-dbnum-instances-json --dbnum 1112");
                std::process::exit(1);
            }
        };

        let from_cache = matches.get_flag("from-cache");
        let detailed = matches.get_flag("detailed");

        // 处理 --use-surrealdb 参数
        let cli_use_surrealdb = matches.get_flag("use-surrealdb");
        if cli_use_surrealdb {
            db_option_ext.use_surrealdb = true;
        }

        println!("🎯 导出 dbnum 实例数据为 JSON（含 AABB）");
        println!("   - 按 dbnum={} 过滤", dbnum);
        println!(
            "   - 数据源: {}",
            if from_cache {
                "model cache"
            } else {
                "SurrealDB"
            }
        );
        println!(
            "   - 格式: {}",
            if detailed {
                "详细模式 (version 3)"
            } else {
                "精简模式 (version 4)"
            }
        );
        if let Some(ref refno) = root_refno {
            println!("   - 仅导出 {} 的 visible 子孙", refno);
        }
        if let Some(ref dir) = export_bundle_dir {
            println!("   - 输出目录: {}", dir.display());
        }

        return export_dbnum_instances_json_mode(
            dbnum,
            verbose,
            export_bundle_dir,
            &db_option_ext,
            true, // autorun=true
            root_refno,
            from_cache,
            detailed,
        )
        .await;
    }

    // 导出 WORL -> SITE 节点列表为 Parquet（Full Parquet Mode 的根节点 children 数据源）
    #[cfg(feature = "parquet-export")]
    if matches.get_flag("export-world-sites-parquet") {
        use crate::cli_modes::export_world_sites_parquet_mode;
        let export_bundle_dir = matches.get_one::<String>("output").map(PathBuf::from);
        return export_world_sites_parquet_mode(verbose, export_bundle_dir, &db_option_ext).await;
    }

    // 导出指定 dbnum 的 PDMS Tree 为 Parquet（TreeIndex + pe.name）
    #[cfg(feature = "parquet-export")]
    if matches.get_flag("export-pdms-tree-parquet") {
        use crate::cli_modes::export_pdms_tree_parquet_mode;
        let dbnum = matches.get_one::<u32>("dbnum").copied();
        let export_bundle_dir = matches.get_one::<String>("output").map(PathBuf::from);
        let dbnum = match dbnum {
            Some(n) => n,
            None => {
                eprintln!("❌ 错误: --export-pdms-tree-parquet 需要提供 --dbnum 参数");
                eprintln!("   例如: cargo run -- --export-pdms-tree-parquet --dbnum 7997");
                std::process::exit(1);
            }
        };
        return export_pdms_tree_parquet_mode(dbnum, verbose, export_bundle_dir, &db_option_ext)
            .await;
    }

    // 导出 dbnum 实例数据为 Parquet（显式 --export-dbnum-instances-parquet）
    // 或默认格式（--export-dbnum-instances，默认 Parquet）
    if matches.get_flag("export-dbnum-instances-parquet")
        || matches.get_flag("export-dbnum-instances")
    {
        use aios_core::pdms_types::RefnoEnum;
        use std::str::FromStr;

        let dbnum_cli = matches.get_one::<u32>("dbnum").copied();
        let root_refno: Option<RefnoEnum> = matches
            .get_one::<String>("root-refno")
            .and_then(|s| {
                let refno_str = s.replace('_', "/");
                RefnoEnum::from_str(&refno_str).ok()
            });
        let dbnum_from_root = root_refno.as_ref().and_then(|r| {
            aios_database::data_interface::db_meta().get_dbnum_by_refno(*r)
        });

        let fill_missing_cache = matches.get_flag("fill-missing-cache");
        let export_bundle_dir = matches.get_one::<String>("output").map(PathBuf::from);

        if let (Some(dbnum_cli), Some(dbnum_root), Some(root)) =
            (dbnum_cli, dbnum_from_root, root_refno.as_ref())
        {
            if dbnum_cli != dbnum_root {
                eprintln!(
                    "❌ 错误: --dbnum={} 与 --root-refno={} 推导 dbnum={} 不一致",
                    dbnum_cli, root, dbnum_root
                );
                std::process::exit(1);
            }
        }

        let dbnum = match (dbnum_cli, dbnum_from_root) {
            (Some(n), _) => n,
            (None, Some(n)) => n,
            (None, None) => {
                eprintln!("❌ 错误: --export-dbnum-instances-parquet 需要提供 --dbnum 或 --root-refno");
                eprintln!("   例如: cargo run -- --export-dbnum-instances-parquet --dbnum 7997");
                eprintln!("   或者: cargo run -- --export-dbnum-instances-parquet --root-refno 24381_145018");
                std::process::exit(1);
            }
        };

        let mut from_surrealdb = matches.get_flag("from-surrealdb");
        if root_refno.is_some() && !from_surrealdb {
            println!("⚠️  检测到 --root-refno，自动切换到 SurrealDB 数据源（cache 模式不支持按 root 范围导出）");
            from_surrealdb = true;
        }

        println!("🎯 导出 dbnum 实例数据为 Parquet（多表，供 DuckDB 查询）");
        println!("   - 按 dbnum={} 过滤", dbnum);
        if let Some(ref root) = root_refno {
            println!("   - 根节点: {}（仅导出其 visible 子孙）", root);
        }
        if from_surrealdb {
            println!("   - 数据源: SurrealDB");
        } else {
            println!("   - 数据源: model cache");
            if fill_missing_cache {
                println!("   - 导出前策略: 自动补齐缺失 refno");
            } else {
                println!("   - 导出前策略: 仅导出已生成 cache（默认）");
            }
        }
        if let Some(ref dir) = export_bundle_dir {
            println!("   - 输出目录: {}", dir.display());
        }

        #[cfg(feature = "parquet-export")]
        if from_surrealdb {
            return crate::cli_modes::export_dbnum_instances_parquet_mode(
                dbnum,
                verbose,
                export_bundle_dir,
                &db_option_ext,
                root_refno,
            )
            .await;
        }

        #[cfg(feature = "parquet-export")]
        return crate::cli_modes::export_dbnum_instances_parquet_from_cache_mode(
            dbnum,
            verbose,
            export_bundle_dir,
            &db_option_ext,
            fill_missing_cache,
        )
        .await;
    }

    // 导出房间实例数据
    if matches.get_flag("export-room-instances") {
        use crate::cli_modes::export_room_instances_mode;

        let output_dir = matches.get_one::<String>("output").map(PathBuf::from);

        return export_room_instances_mode(output_dir, verbose).await;
    }

    // 导入 instances.json 到 SQLite 空间索引
    if let Some(json_path) = matches.get_one::<String>("import-spatial-index") {
        use crate::cli_modes::import_spatial_index_mode;

        let sqlite_path = matches
            .get_one::<String>("spatial-index-output")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("output/spatial_index.sqlite"));

        return import_spatial_index_mode(Path::new(json_path), &sqlite_path, verbose);
    }

    if matches.get_flag("export-all-relates") {
        use crate::cli_modes::export_all_relates_mode;

        let dbnum = matches.get_one::<u32>("dbnum").copied();
        let export_bundle_dir = matches.get_one::<String>("output").map(PathBuf::from);
        let export_all_lods = matches.get_flag("export-all-lods");
        let export_refnos = matches.get_one::<String>("export-refnos").cloned();

        // 解析 owner-types 参数（逗号分隔）
        let owner_types: Option<Vec<String>> = matches
            .get_one::<String>("owner-types")
            .map(|s| s.split(',').map(|t| t.trim().to_uppercase()).collect());

        // 获取名称配置文件路径
        let name_config_path = matches.get_one::<String>("name-config").map(PathBuf::from);

        println!("🎯 导出 inst_relate 实体 (Prepack LOD 格式)");
        if let Some(ref refnos) = export_refnos {
            println!("   - 🎯 仅导出指定 refnos={}", refnos);
        } else if let Some(dbnum) = dbnum {
            println!("   - 按 dbnum={} 过滤", dbnum);
        } else {
            println!("   - 全表扫描（所有 dbnum）");
        }
        if let Some(ref types) = owner_types {
            println!("   - 按 owner_type 过滤: {:?}", types);
        }
        if let Some(ref path) = name_config_path {
            println!("   - 名称配置文件: {}", path.display());
        }

        return export_all_relates_mode(
            dbnum,
            verbose,
            export_bundle_dir,
            owner_types,
            name_config_path,
            export_all_lods,
            export_refnos,
            source_unit.to_string(),
            target_unit.to_string(),
            &db_option_ext,
        )
        .await;
    }

    // ========== 处理 --room-compute 房间计算命令 ==========
    if matches.get_flag("room-compute") {
        use crate::cli_modes::room_compute_mode;
        use aios_core::RefnoEnum;
        use std::str::FromStr;

        let room_keywords: Option<Vec<String>> = matches
            .get_many::<String>("room-keywords")
            .map(|kws| kws.map(|s| s.to_string()).collect());

        let force_rebuild = matches.get_flag("room-force-rebuild");

        let db_nums: Option<Vec<u32>> = matches
            .get_many::<String>("room-db-nums")
            .map(|nums| nums.filter_map(|s| s.parse::<u32>().ok()).collect());

        let refno_root: Option<RefnoEnum> =
            matches.get_one::<String>("room-refno-root").and_then(|s| {
                let refno_str = s.replace('_', "/");
                RefnoEnum::from_str(&refno_str).ok()
            });

        println!("🏠 启动房间计算模式");
        if let Some(ref kws) = room_keywords {
            println!("   - 房间关键词: {:?}", kws);
        }
        if let Some(ref nums) = db_nums {
            println!("   - 数据库编号: {:?}", nums);
        }
        if let Some(ref root) = refno_root {
            println!("   - refno 子树根: {}", root);
        }
        println!("   - 强制重建: {}", force_rebuild);

        return room_compute_mode(
            room_keywords,
            db_nums,
            refno_root,
            force_rebuild,
            verbose,
            &db_option_ext,
        )
        .await;
    }

    // ========== 处理 --refresh-transform pe_transform 刷新命令 ==========
    if let Some(dbnums) = matches.get_many::<String>("refresh-transform") {
        let dbnums: Vec<u32> = dbnums.filter_map(|s| s.parse::<u32>().ok()).collect();
        if !dbnums.is_empty() {
            println!("🔄 刷新 pe_transform 缓存: dbnums={:?}", dbnums);
            init_surreal().await?;

            // 使用 DbMetaManager 加载元信息
            use aios_database::data_interface::db_meta;
            if let Err(e) = db_meta().try_load_default() {
                eprintln!("⚠️  {}", e);
                return Ok(());
            }

            let count = aios_core::transform::refresh_pe_transform_for_dbnums(&dbnums).await?;
            println!("✅ pe_transform 刷新完成，共处理 {} 个节点", count);
            return Ok(());
        }
    }

    // 如果指定了启动GRPC服务器
    #[cfg(feature = "grpc")]
    if matches.get_flag("grpc-server") {
        return start_grpc_server_mode(&matches, db_option_ext).await;
    }

    // 否则运行正常的应用程序
    run_app(Some(db_option_ext)).await
}

#[cfg(not(feature = "gui"))]
fn maybe_redirect_stdio_to_log_file() {
    use chrono::{Datelike, Local, Timelike};
    use std::fs::File;

    if std::env::var_os("AIOS_STDIO_REDIRECTED").is_some() {
        return;
    }

    let args: Vec<String> = std::env::args().collect();
    let has_flag = |flag: &str| args.iter().any(|a| a == flag);

    // 显式 verbose / 服务模式：不重定向，便于交互调试/观察运行状态。
    if has_flag("--verbose") || has_flag("--grpc-server") {
        // 允许用户按需设置 AIOS_LOG_TO_CONSOLE=1，把 log::info 也打印到控制台。
        return;
    }

    // 仅在“可能产生海量输出”的路径下默认重定向（debug-model/export/capture 等）。
    let should_redirect = has_flag("--debug-model")
        || has_flag("--export-obj")
        || has_flag("--export-glb")
        || has_flag("--export-gltf")
        || has_flag("--export-obj-refnos")
        || has_flag("--export-glb-refnos")
        || has_flag("--export-gltf-refnos")
        || has_flag("--capture")
        || has_flag("--log-model-error");

    if !should_redirect {
        return;
    }

    // 简易提取一个“标识 refno”用于日志文件命名（仅取第一个）。
    fn first_value_after_flag(args: &[String], flag: &str) -> Option<String> {
        let mut it = args.iter().enumerate();
        while let Some((i, a)) = it.next() {
            if a == flag {
                if let Some(v) = args.get(i + 1) {
                    if !v.starts_with('-') && !v.trim().is_empty() {
                        return Some(v.clone());
                    }
                }
            }
        }
        None
    }

    let ref_tag = first_value_after_flag(&args, "--debug-model")
        .or_else(|| first_value_after_flag(&args, "--export-obj-refnos"))
        .or_else(|| first_value_after_flag(&args, "--export-glb-refnos"))
        .or_else(|| first_value_after_flag(&args, "--export-gltf-refnos"))
        .unwrap_or_else(|| "run".to_string());

    let now = Local::now();
    let ts = format!(
        "{}-{:02}-{:02}_{:02}-{:02}-{:02}",
        now.year(),
        now.month(),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );
    let log_filename = format!("logs/{}_{}.log", ref_tag.replace('/', "_"), ts);

    // 预创建目录/文件；失败则回退到控制台模式。
    if let Some(parent) = std::path::Path::new(&log_filename).parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let Ok(out_file) = File::create(&log_filename) else {
        return;
    };
    let Ok(err_file) = out_file.try_clone() else {
        return;
    };

    // 重新执行自身：把 stdout/stderr 重定向到日志文件；父进程仅输出日志路径。
    // 注意：避免递归重进（AIOS_STDIO_REDIRECTED 标记）。
    let exe = &args[0];
    let child_status = StdCommand::new(exe)
        .args(&args[1..])
        .env("AIOS_STDIO_REDIRECTED", "1")
        .env("AIOS_LOG_FILE", &log_filename)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_file))
        .stderr(Stdio::from(err_file))
        .status();

    match child_status {
        Ok(status) => {
            // 仅打印一行提示，满足“默认不刷控制台”的诉求。
            eprintln!("日志已写入: {}", log_filename);
            std::process::exit(status.code().unwrap_or(1));
        }
        Err(_) => {
            // 启动失败则回退控制台输出
        }
    }
}
