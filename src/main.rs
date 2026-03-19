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

#[cfg(not(feature = "gui"))]

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

    // 2. export-parquet / export-dbnum-instances-json
    let want_parquet =
        matches.get_flag("export-parquet") || matches.get_flag("export-dbnum-instances");
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
        println!(
            "\n📦 后置步骤：从 SurrealDB 导出 dbnum={} 实例数据为 Parquet",
            dbnum
        );
        crate::cli_modes::export_dbnum_instances_parquet_mode(
            dbnum,
            verbose,
            output_override.clone(),
            db_option_ext,
            root_refno,
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
    get_output_filename_for_refno, rebuild_room_spatial_index_mode,
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
    // 默认不重定向 stdout/stderr，保证终端有输出，避免控制台刷屏导致“看似死循环”。
    // 默认不重定向；-v/--verbose 始终保留控制台输出；AIOS_REDIRECT_STDIO=1 可启用重定向到 logs/。
    maybe_redirect_stdio_to_log_file();

    let matches = aios_database::cli_args::add_export_instance_args(Command::new("aios-database")
        .version("0.1.3")
        .about("AIOS Database Processing Tool")
        .arg(
            Arg::new("config")
                .long("config")
                .short('c')
                .help("Path to the configuration file (Without extension)")
                .value_name("CONFIG_PATH")
                .default_value(if cfg!(target_family = "unix") {
                    "db_options/DbOption-mac"
                } else {
                    "db_options/DbOption"
                }),
        )
        .arg(
            Arg::new("gen-lod")
                .long("gen-lod")
                .help("Override mesh generation LOD level for this run (L0-L4). Defaults to db_options/DbOption.toml")
                .value_name("LOD")
                .value_parser(["L0", "L1", "L2", "L3", "L4"]),
        )
        .arg(
            Arg::new("debug-model")
                .long("debug-model")
                .help("Enable debug model output with verbose debug logging. Can optionally specify reference numbers (comma-separated)")
                .value_name("REFNOS")
                .value_delimiter(',')
                .num_args(0..)
                .conflicts_with("root-model"),
        )
        .arg(
            Arg::new("root-model")
                .long("root-model")
                .help("Incremental model generation for specified refnos WITHOUT debug logging (quieter alternative to --debug-model)")
                .value_name("REFNOS")
                .value_delimiter(',')
                .num_args(0..)
                .conflicts_with("debug-model"),
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
                .help("Deprecated and ignored: DB writes always stay online during model generation")
                .action(clap::ArgAction::SetTrue),
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
            Arg::new("gen-dry-run")
                .long("gen-dry-run")
                .help("Dry run: only collect refnos and log, skip geometry generation and DB writes. Use to verify refnos are processed (e.g. grep 24381_145019)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-parquet-after-gen")
                .long("export-parquet-after-gen")
                .help("After model generation, automatically export Parquet for each dbnum in manual_db_nums (instances/tubings/transforms/aabb)")
                .action(clap::ArgAction::SetTrue),
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
        ))
        .arg(
            Arg::new("import-spatial-index")
                .long("import-spatial-index")
                .help("Import instances.json to SQLite spatial index")
                .value_name("JSON_PATH"),
        )
        .arg(
            Arg::new("import-rvm")
                .long("import-rvm")
                .help("Import an RVM file into SQLite relation tables")
                .value_name("RVM_PATH"),
        )
        .arg(
            Arg::new("import-att")
                .long("import-att")
                .help("Optional ATT/TXT files paired with --import-rvm (comma-separated or repeated)")
                .value_name("ATT_PATHS")
                .value_delimiter(',')
                .num_args(1..),
        )
        .arg(
            Arg::new("spatial-index-output")
                .long("spatial-index-output")
                .help("Output path for SQLite spatial index (default: output/spatial_index.sqlite)")
                .value_name("SQLITE_PATH"),
        )
        .arg(
            Arg::new("relation-store-output")
                .long("relation-store-output")
                .help("Root directory for SQLite relation store output (default: output/model_relations)")
                .value_name("DIR"),
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
        .subcommand(
            Command::new("spatial")
                .about("SQLite 空间范围查询与回归验证")
                .subcommand(
                    Command::new("query-refno")
                        .about("以 refno 为中心做空间范围查询，并可校验 expect-refnos / verify-json")
                        .arg(
                            Arg::new("refno")
                                .help("查询中心 refno（如 24381/145019）")
                                .required(true),
                        )
                        .arg(
                            Arg::new("distance-mm")
                                .long("distance-mm")
                                .help("查询距离，单位毫米；1m 请传 1000")
                                .default_value("1000"),
                        )
                        .arg(
                            Arg::new("include-self")
                                .long("include-self")
                                .help("结果中包含查询 refno 本身")
                                .action(clap::ArgAction::SetTrue),
                        )
                        .arg(
                            Arg::new("build-spatial")
                                .long("build-spatial")
                                .help("查询前先刷新 output/spatial_index.sqlite")
                                .action(clap::ArgAction::SetTrue),
                        )
                        .arg(
                            Arg::new("expect-refnos")
                                .long("expect-refnos")
                                .help("期望命中的 refno（逗号分隔）")
                                .value_delimiter(',')
                                .num_args(1..),
                        )
                        .arg(
                            Arg::new("verify-json")
                                .long("verify-json")
                                .help("将当前查询结果与给定 JSON 快照做回归校验"),
                        )
                        .arg(
                            Arg::new("write-verify-json")
                                .long("write-verify-json")
                                .help("将当前查询结果写入 JSON 快照文件"),
                        ),
                ),
        )
        // ========== 房间计算子命令 ==========
        .subcommand(
            Command::new("room")
                .about("房间计算相关命令")
                .subcommand(
                    Command::new("compute")
                        .about("执行房间关系计算（构件空间归属判定）")
                        .arg(Arg::new("keywords").long("keywords").short('k')
                            .help("房间名称关键词过滤（逗号分隔）")
                            .value_delimiter(',')
                            .num_args(1..))
                        .arg(Arg::new("db-nums").long("db-nums")
                            .help("限定数据库编号（逗号分隔）")
                            .value_delimiter(',')
                            .num_args(1..))
                        .arg(Arg::new("refno-root").long("refno-root")
                            .help("限定 refno 子树根（如 21491_10000）"))
                        .arg(Arg::new("gen-panels-mesh").long("gen-panels-mesh")
                            .help("预生成缺失面板的几何模型（默认跳过，仅计算空间关系）")
                            .action(clap::ArgAction::SetTrue))
                        .arg(Arg::new("report-json").long("report-json")
                            .help("将房间计算阶段耗时与统计写入 JSON 报告")
                            .value_name("FILE")),
                )
                .subcommand(
                    Command::new("compute-panel")
                        .about("指定单个面板 refno 执行房间计算")
                        .arg(Arg::new("panel-refno")
                            .help("面板参考号（如 24381/35798）")
                            .required(true))
                        .arg(Arg::new("expect-refnos").long("expect-refnos")
                            .help("期望命中的构件 refno（逗号分隔），用于验证计算结果")
                            .value_delimiter(',')
                            .num_args(1..))
                        .arg(Arg::new("rebuild-spatial-index").long("rebuild-spatial-index")
                            .help("显式重建本次 panel 计算使用的局部 SQLite 空间索引；默认直接复用现有索引")
                            .action(clap::ArgAction::SetTrue))
                        .arg(Arg::new("report-json").long("report-json")
                            .help("将单面板计算阶段耗时与统计写入 JSON 报告")
                            .value_name("FILE")),
                )
                .subcommand(
                    Command::new("rebuild-spatial-index")
                        .about("从 inst_relate_aabb 正式重建全量 SQLite 空间索引"),
                )
                .subcommand(
                    Command::new("clean")
                        .about("清理已有的房间关系数据（room_relate + room_panel_relate）"),
                )
                .subcommand(
                    Command::new("verify-json")
                        .about("读取 JSON fixture 校验已持久化的房间计算结果（默认只读）")
                        .arg(
                            Arg::new("input")
                                .long("input")
                                .short('i')
                                .help("验证 fixture JSON 路径（推荐：verification/room_compute_validation.json）")
                                .required(true)
                                .value_name("FILE"),
                        ),
                )
                .subcommand(
                    Command::new("export")
                        .about("导出房间计算结果为 JSON")
                        .arg(Arg::new("output").long("output").short('o')
                            .help("输出目录")),
                ),
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
        .arg(
            Arg::new("force")
                .long("force")
                .help("Force kill processes holding RocksDB LOCK files before connecting (强制终止占用 LOCK 的进程)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("offline")
                .long("offline")
                .help("Use embedded file mode instead of WebSocket. Auto-kills any running SurrealDB server on the configured port")
                .action(clap::ArgAction::SetTrue),
        )
        .get_matches();

    if let Some(relation_store_root) = matches.get_one::<String>("relation-store-output") {
        unsafe {
            std::env::set_var("MODEL_RELATION_STORE_PATH", relation_store_root);
        }
    }

    // 获取配置文件路径
    let config_path = matches
        .get_one::<String>("config")
        .expect("default value ensures this exists");

    // 设置环境变量，让 rs-core 库使用正确的配置文件
    unsafe {
        std::env::set_var("DB_OPTION_FILE", config_path);
    }

    // --offline：在 get_db_option() OnceCell 初始化前设置环境变量，覆盖 surrealdb.mode = file
    let is_offline = matches.get_flag("offline");
    if is_offline {
        println!("🔌 --offline 模式：切换为嵌入式文件连接");
        unsafe {
            std::env::set_var("SURREAL_CONN_MODE", "file");
        }
    }

    // --force：强制清理 RocksDB LOCK 文件（kill 占用进程）
    if matches.get_flag("force") {
        println!("🔧 --force 模式：将强制清理 LOCK 文件");
        unsafe {
            std::env::set_var("AIOS_FORCE_LOCK", "1");
        }
    }

    // 预先初始化 OnceCell，避免后续第一次 get_db_option() 时覆盖 active_precision
    let db_option = aios_core::get_db_option();

    // --offline 时立即关闭占用 ws 端口的 server 进程（RocksDB 排他锁）
    if is_offline {
        crate::cli_modes::kill_process_on_port(db_option.surrealdb.port);
    }

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

    if is_offline {
        db_option_ext.inner.surrealdb.mode = aios_core::options::DbConnMode::File;
        println!(
            "🔧 CLI 覆盖 surrealdb.mode -> {}（db_option_ext）",
            db_option_ext.inner.surrealdb.mode.as_str()
        );
    }

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
    if matches.get_flag("gen-dry-run") {
        db_option_ext.gen_model_dry_run = true;
        println!("🔧 模型生成空跑模式: 仅收集 refno 并记录日志，跳过几何生成与 DB 写入");
    }
    if matches.get_flag("export-parquet-after-gen") {
        db_option_ext.export_parquet_after_gen = true;
        println!("🔧 模型生成完成后将自动导出 Parquet（按 manual_db_nums）");
    }

    // 同步精度配置到 rs-core 全局 active_precision，保证布尔/导出等逻辑使用同一套 LOD
    aios_core::mesh_precision::set_active_precision(db_option_ext.inner.mesh_precision.clone());

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
    let root_model_requested = matches.contains_id("root-model");
    let any_model_requested = debug_model_requested || root_model_requested;
    let debug_model_errors_only = matches.get_flag("debug-model-errors-only") || log_model_error;

    if log_model_error {
        println!("📊 启用模型错误记录模式（自动开启 debug-model + errors-only）");
    }

    if !any_model_requested && db_option_ext.inner.debug_model_refnos.is_some() {
        println!("ℹ️ 未开启调试/根模型模式，本次运行将忽略配置中的 debug_model_refnos");
    }
    if !any_model_requested {
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

    // ========== 首先处理 --debug-model / --root-model 参数（必须在所有导出逻辑之前） ==========
    let debug_model_refnos: Option<Vec<String>> = if any_model_requested {
        // --debug-model 才启用调试打印；--root-model 不启用
        if debug_model_requested {
            aios_core::set_debug_model_enabled(true);
            clear_ploop_debug_cache(); // 清理PLOOP调试文件缓存，允许重新生成
            println!("✅ 已启用 debug_model 调试信息打印");
        } else {
            println!("✅ 已启用 root-model 模式（不打印调试信息）");
        }

        if !db_option_ext.inner.gen_mesh {
            println!("🔄 自动开启 gen_mesh");
            db_option_ext.inner.gen_mesh = true;
        }

        // 确保 gen_model 也被启用，以便 is_gen_mesh_or_model() 返回 true
        if !db_option_ext.inner.gen_model {
            println!("🔄 自动开启 gen_model");
            db_option_ext.inner.gen_model = true;
        }

        // 从 --debug-model 或 --root-model 中取 refnos
        let cli_refnos: Vec<String> = matches
            .get_many::<String>("debug-model")
            .or_else(|| matches.get_many::<String>("root-model"))
            .map(|values| values.map(|s| s.to_string()).collect())
            .unwrap_or_else(Vec::new);

        let mode_label = if debug_model_requested {
            "debug-model"
        } else {
            "root-model"
        };

        if !cli_refnos.is_empty() {
            println!(
                "🔍 使用命令行指定的 {} 参考号: {:?}",
                mode_label, cli_refnos
            );
            db_option_ext.inner.debug_model_refnos = Some(cli_refnos.clone());
            Some(cli_refnos)
        } else if let Some(config_refnos) = config_debug_refnos.as_ref() {
            if config_refnos.is_empty() {
                println!("💡 仅启用 {} 模式，未指定参考号", mode_label);
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
            println!("💡 仅启用 {} 模式，未指定参考号", mode_label);
            db_option_ext.inner.debug_model_refnos = None;
            None
        }
    } else {
        None
    };

    if debug_model_requested {
        // 仅 --debug-model 才启用日志文件写入
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
    let regen_auto_enabled_defer_db_write = false;
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
        // mesh 已改为 insert_handle 内联处理，不再有竞态条件，无需 defer_db_write
    }

    // --defer-db-write：模型生成阶段不写 SurrealDB，SQL 输出到 .surql 文件
    let defer_db_write_explicit = matches.get_flag("defer-db-write");
    if defer_db_write_explicit {
        println!("⚠️ --defer-db-write 已停用，当前版本将忽略该参数并继续在线写库");
    }

    // --debug-model 是增量模式，不应强制 replace_mesh（不清理旧数据）；
    // 只有 --regen-model 才需要 replace_mesh + pre_cleanup_for_regen。
    if any_model_requested && !regen_model_requested && db_option_ext.inner.gen_mesh {
        if db_option_ext.inner.replace_mesh == Some(true) {
            println!("⚠️ 调试模式检测到 replace_mesh=true（配置/--regen-model），保持不变");
        }
    }

    // 模型导出请求：默认只导出不触发生成；--regen-model 或 --debug-model 前置生成。
    let model_export_requested = matches.get_flag("export-obj")
        || matches.get_flag("export-svg")
        || matches.get_flag("export-glb")
        || matches.get_flag("export-gltf")
        || matches.contains_id("export-obj-refnos")
        || matches.contains_id("export-glb-refnos")
        || matches.contains_id("export-gltf-refnos")
        || (any_model_requested && capture_dir.is_some());
    let follow_up_export_requested =
        model_export_requested || matches.get_flag("export-parquet-after-gen");
    let any_export_requested = model_export_requested
        || matches.get_flag("export-all-parquet")
        || matches.get_flag("export-all-relates")
        || matches.get_flag("export-dbnum-instances-json")
        || matches.get_flag("export-parquet")
        || matches.get_flag("export-dbnum-instances")
        || matches.get_flag("export-pdms-tree-parquet")
        || matches.get_flag("export-world-sites-parquet");

    // ========== 执行模型生成 ==========
    // --regen-model: 清理后重新生成（强制 replace_mesh + FORCE_REGEN_MESH）
    // --debug-model: 直接增量生成（不清理，补充缺失的 inst_geo/mesh/布尔结果）
    let should_generate = regen_model_requested || any_model_requested;
    if should_generate {
        // 确定生成的目标 refnos：优先 debug-model 指定的 refnos，其次 CLI 独立 refno 参数，
        // 再次 dbnum（查询所有 SITE），最后全库模式。
        let gen_refnos_vec: Vec<String> = if let Some(ref refnos) = debug_model_refnos {
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
        let gen_config = build_export_config(
            gen_refnos_vec,
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

        if regen_model_requested {
            // --regen-model: 清理 + 强制重新生成
            let regen_result = cli_modes::run_regen_model(&gen_config, &db_option_ext).await?;

            if !any_export_requested {
                println!("✅ --regen-model 单独执行完成（未请求导出，流程到此结束）");
                return Ok(());
            }
        } else {
            // --debug-model: 增量生成（不清理、不强制 FORCE_REPLACE_MESH）
            let _gen_result = cli_modes::run_generate_model(&gen_config, &db_option_ext).await?;
        }
    }

    // 当前策略固定为 SurrealDB 输入，导出流程仅保留该路径。
    if model_export_requested {
        db_option_ext.use_surrealdb = true;
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

        // 解析 --debug-model / --root-model 参数作为 root_refno
        let root_refno: Option<RefnoEnum> = matches
            .get_many::<String>("debug-model")
            .or_else(|| matches.get_many::<String>("root-model"))
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

    // 导出 dbnum 实例数据为 Parquet（显式 --export-parquet）
    // 或默认格式（--export-dbnum-instances，默认 Parquet）
    if matches.get_flag("export-parquet") || matches.get_flag("export-dbnum-instances") {
        use aios_core::pdms_types::RefnoEnum;
        use std::str::FromStr;

        let dbnum_cli = matches.get_one::<u32>("dbnum").copied();
        let root_refno: Option<RefnoEnum> = matches.get_one::<String>("root-refno").and_then(|s| {
            let refno_str = s.replace('_', "/");
            RefnoEnum::from_str(&refno_str).ok()
        });
        let dbnum_from_root = root_refno
            .as_ref()
            .and_then(|r| aios_database::data_interface::db_meta().get_dbnum_by_refno(*r));

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

        let single_dbnum = match (dbnum_cli, dbnum_from_root) {
            (Some(n), _) => Some(n),
            (None, Some(n)) => Some(n),
            (None, None) => None,
        };

        // 未指定 dbnum：扫描 inst_relate 所有 distinct dbnum，逐一导出
        #[cfg(feature = "parquet-export")]
        if single_dbnum.is_none() {
            use aios_database::fast_model::export_model::export_dbnum_instances_parquet::query_distinct_dbnums_from_inst_relate;

            println!("📋 未指定 --dbnum，扫描 inst_relate 所有 dbnum...");
            init_surreal().await?;
            let dbnums = query_distinct_dbnums_from_inst_relate().await?;

            if dbnums.is_empty() {
                eprintln!("❌ 错误: inst_relate 表中未找到任何 dbnum");
                std::process::exit(1);
            }

            println!("📋 扫描到 {} 个 dbnum: {:?}", dbnums.len(), dbnums);

            for (i, dbnum) in dbnums.iter().enumerate() {
                println!(
                    "\n{} [{}/{}] 导出 dbnum={}",
                    "=".repeat(30),
                    i + 1,
                    dbnums.len(),
                    dbnum,
                );
                crate::cli_modes::export_dbnum_instances_parquet_mode(
                    *dbnum,
                    verbose,
                    export_bundle_dir.clone(),
                    &db_option_ext,
                    None,
                )
                .await?;
            }
            println!("\n🎉 所有 dbnum 导出完成！共 {} 个", dbnums.len());
            return Ok(());
        }

        #[cfg(not(feature = "parquet-export"))]
        if single_dbnum.is_none() {
            eprintln!("❌ 错误: parquet-export 特性未启用，请使用 --features parquet-export 编译");
            std::process::exit(1);
        }

        let dbnum = single_dbnum.unwrap();

        println!("🎯 导出 dbnum 实例数据为 Parquet（多表，供前端查询）");
        println!("   - 按 dbnum={} 过滤", dbnum);
        if let Some(ref root) = root_refno {
            println!("   - 根节点: {}（仅导出其 visible 子孙）", root);
        }
        println!("   - 数据源: SurrealDB");
        if let Some(ref dir) = export_bundle_dir {
            println!("   - 输出目录: {}", dir.display());
        }

        #[cfg(feature = "parquet-export")]
        return crate::cli_modes::export_dbnum_instances_parquet_mode(
            dbnum,
            verbose,
            export_bundle_dir,
            &db_option_ext,
            root_refno,
        )
        .await;
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

    if let Some(rvm_path) = matches.get_one::<String>("import-rvm") {
        use crate::cli_modes::import_rvm_mode;

        let dbnum = matches
            .get_one::<u32>("dbnum")
            .copied()
            .ok_or_else(|| anyhow::anyhow!("--import-rvm 需要同时指定 --dbnum"))?;
        let att_paths: Vec<PathBuf> = matches
            .get_many::<String>("import-att")
            .map(|vals| vals.map(PathBuf::from).collect())
            .unwrap_or_default();
        let relation_store_root = matches
            .get_one::<String>("relation-store-output")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("output/model_relations"));

        return import_rvm_mode(
            Path::new(rvm_path),
            &att_paths,
            dbnum,
            &relation_store_root,
            verbose,
        );
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

    // ========== 处理 spatial 子命令 ==========
    if let Some(spatial_matches) = matches.subcommand_matches("spatial") {
        use crate::cli_modes::spatial_query_refno_mode;

        match spatial_matches.subcommand() {
            Some(("query-refno", sub_m)) => {
                let refno = sub_m.get_one::<String>("refno").unwrap();
                let distance_mm = sub_m
                    .get_one::<String>("distance-mm")
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(1000.0);
                let include_self = sub_m.get_flag("include-self");
                let build_spatial = sub_m.get_flag("build-spatial");
                let expect_refnos: Option<Vec<String>> = sub_m
                    .get_many::<String>("expect-refnos")
                    .map(|v| v.map(|s| s.to_string()).collect());
                let verify_json_path = sub_m.get_one::<String>("verify-json").map(PathBuf::from);
                let write_verify_json_path = sub_m
                    .get_one::<String>("write-verify-json")
                    .map(PathBuf::from);

                return spatial_query_refno_mode(
                    refno,
                    distance_mm,
                    include_self,
                    build_spatial,
                    expect_refnos,
                    verify_json_path.as_deref(),
                    write_verify_json_path.as_deref(),
                    verbose,
                )
                .await;
            }
            _ => {
                println!("请指定 spatial 子命令，使用 --help 查看可用命令");
                return Ok(());
            }
        }
    }

    // ========== 处理 room 子命令 ==========
    if let Some(room_matches) = matches.subcommand_matches("room") {
        use crate::cli_modes::{
            export_room_instances_mode, room_clean_mode, room_compute_mode,
            room_compute_panel_mode, room_verify_json_mode,
        };
        use aios_core::RefnoEnum;
        use std::str::FromStr;

        match room_matches.subcommand() {
            Some(("compute", sub_m)) => {
                let keywords: Option<Vec<String>> = sub_m
                    .get_many::<String>("keywords")
                    .map(|kws| kws.map(|s| s.to_string()).collect());

                let db_nums: Option<Vec<u32>> = sub_m
                    .get_many::<String>("db-nums")
                    .map(|nums| nums.filter_map(|s| s.parse::<u32>().ok()).collect());

                let refno_root: Option<RefnoEnum> =
                    sub_m.get_one::<String>("refno-root").and_then(|s| {
                        let refno_str = s.replace('_', "/");
                        RefnoEnum::from_str(&refno_str).ok()
                    });

                let gen_panels_mesh = sub_m.get_flag("gen-panels-mesh");
                let report_json = sub_m.get_one::<String>("report-json").map(PathBuf::from);

                return room_compute_mode(
                    keywords,
                    db_nums,
                    refno_root,
                    gen_panels_mesh,
                    report_json,
                    verbose,
                    &db_option_ext,
                )
                .await;
            }
            Some(("compute-panel", sub_m)) => {
                let panel_refno = sub_m.get_one::<String>("panel-refno").unwrap();
                let expect_refnos: Option<Vec<String>> = sub_m
                    .get_many::<String>("expect-refnos")
                    .map(|v| v.map(|s| s.to_string()).collect());
                let rebuild_spatial_index = sub_m.get_flag("rebuild-spatial-index");
                let report_json = sub_m.get_one::<String>("report-json").map(PathBuf::from);

                return room_compute_panel_mode(
                    panel_refno,
                    expect_refnos,
                    rebuild_spatial_index,
                    report_json,
                    verbose,
                    &db_option_ext,
                )
                .await;
            }
            Some(("rebuild-spatial-index", _)) => {
                return rebuild_room_spatial_index_mode(verbose).await;
            }
            Some(("clean", _)) => {
                return room_clean_mode(&db_option_ext).await;
            }
            Some(("verify-json", sub_m)) => {
                let input = sub_m.get_one::<String>("input").unwrap();
                return room_verify_json_mode(Path::new(input), &db_option_ext).await;
            }
            Some(("export", sub_m)) => {
                let output_dir = sub_m.get_one::<String>("output").map(PathBuf::from);
                return export_room_instances_mode(output_dir, verbose).await;
            }
            _ => {
                println!("请指定 room 子命令，使用 --help 查看可用命令");
                return Ok(());
            }
        }
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
    // 支持 -v 和 --verbose，避免用户加 -v 后仍被重定向导致终端无输出（看似卡住）。
    if has_flag("--verbose") || has_flag("-v") {
        // 允许用户按需设置 AIOS_LOG_TO_CONSOLE=1，把 log::info 也打印到控制台。
        return;
    }

    // 默认不重定向，避免 spawn 子进程后终端无输出导致“卡住”的假象。
    // 需要重定向时设置环境变量 AIOS_REDIRECT_STDIO=1。
    if std::env::var_os("AIOS_REDIRECT_STDIO")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        return;
    }

    // 仅在“可能产生海量输出”的路径下重定向（debug-model/export/capture 等）。
    let should_redirect = has_flag("--debug-model")
        || has_flag("--root-model")
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
        .or_else(|| first_value_after_flag(&args, "--root-model"))
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
