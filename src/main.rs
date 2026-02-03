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

    let matches = Command::new("aios-database")
        .version("0.1.3")
        .about("AIOS Database Processing Tool")
        .arg(
            Arg::new("config")
                .long("config")
                .short('c')
                .help("Path to the configuration file (Without extension)")
                .value_name("CONFIG_PATH")
                .default_value("DbOption"),
        )
        .arg(
            Arg::new("gen-lod")
                .long("gen-lod")
                .help("Override mesh generation LOD level for this run (L0-L4). Defaults to DbOption.toml")
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
                .help("Regenerate model data before export (forces replace_mesh mode)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("flush-cache-to-db")
                .long("flush-cache-to-db")
                .help("Flush foyer instance_cache to SurrealDB (backup). Requires SurrealDB config in DbOption")
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
                .help("Enable SurrealDB instances source / model-data writes for export/debug flows (default: cache-only instances, no fallback)")
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
                .help("Database number for exporting all SITE models")
                .value_name("DBNO")
                .value_parser(clap::value_parser!(u32)),
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
                .help("Export dbnum instances as simplified JSON with AABB data (use_cache=true 时仅走缓存，不回退 SurrealDB)")
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
        // ========== pe_transform 刷新命令 ==========
        .arg(
            Arg::new("refresh-transform")
                .long("refresh-transform")
                .help("Refresh pe_transform cache for specified dbnums (comma-separated, e.g., '1112,1113')")
                .value_name("DB_NUMS")
                .value_delimiter(',')
                .num_args(1..),
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

    // 同步精度配置到 rs-core 全局 active_precision，保证布尔/导出等逻辑使用同一套 LOD
    aios_core::mesh_precision::set_active_precision(db_option_ext.inner.mesh_precision.clone());

    // ========== cache -> SurrealDB：一键备份落库 ==========
    if matches.get_flag("flush-cache-to-db") {
        println!("\n🗄️  flush-cache-to-db: 将 foyer instance_cache 写入 SurrealDB（备份）");
        init_surreal().await?;
        println!("✅ 数据库连接成功");

        let cache_dir = db_option_ext.get_foyer_cache_dir();
        let dbnums: Option<Vec<u32>> = matches
            .get_many::<u32>("flush-cache-dbnums")
            .map(|v| v.copied().collect());
        let replace_exist = matches.get_flag("flush-cache-replace");

        let flushed = aios_database::fast_model::cache_flush::flush_latest_instance_cache_to_surreal(
            &cache_dir,
            dbnums.as_deref(),
            replace_exist,
            true,
        )
        .await?;

        println!(
            "✅ flush-cache-to-db 完成：cache_dir={} flushed_dbnums={}",
            cache_dir.display(),
            flushed
        );
        return Ok(());
    }

    // 调试：显示配置加载结果
    println!("🔧 配置加载完成:");
    println!("   - 配置文件路径: {}", config_path);
    println!(
        "   - full_noun_enabled_categories: {:?}",
        db_option_ext.full_noun_enabled_categories
    );
    println!(
        "   - full_noun_excluded_nouns: {:?}",
        db_option_ext.full_noun_excluded_nouns
    );

    // 设置 Full Noun 模式环境变量
    if db_option_ext.full_noun_mode {
        unsafe {
            std::env::set_var("FULL_NOUN_MODE", "true");
        }
        println!("✅ Full Noun 模式已启用");
    }
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
            aios_database::data_interface::db_meta_manager::generate_desi_indextree()?;
        }
        println!("✅ indextree 生成完成");
        return Ok(());
    }

    // ========== 处理 --regen-model 参数（影响纯模型生成） ==========
    if matches.get_flag("regen-model") {
        println!("🔄 检测到 --regen-model 参数，强制开启 replace_mesh 模式");
        // 与 replace_mesh 配合：强制 mesh_worker 忽略 mesh_sig 缓存，确保本次能看到最新代码/配置效果。
        unsafe {
            std::env::set_var("FORCE_REGEN_MESH", "1");
            // 兼容 cache-only mesh_worker 的替换语义（run_mesh_worker_from_cache_manager 读取该变量）。
            std::env::set_var("FORCE_REPLACE_MESH", "true");
        }
        db_option_ext.inner.replace_mesh = Some(true);
        // 元件库(cata_neg)/设计型负实体导出依赖布尔结果（CatePos），因此 regen-model 必须开启布尔运算。
        if !db_option_ext.inner.apply_boolean_operation {
            println!("🔄 --regen-model 自动开启 apply_boolean_operation（生成 CatePos 布尔结果）");
            db_option_ext.inner.apply_boolean_operation = true;
        }
    }

    // 调试模式下，如果配置开启了 gen_mesh，默认也应强制重新生成 mesh
    if debug_model_requested && db_option_ext.inner.gen_mesh {
        if db_option_ext.inner.replace_mesh != Some(true) {
            println!("🔄 调试模式启用 gen_mesh，默认开启 replace_mesh 以重新生成模型数据");
        }
        db_option_ext.inner.replace_mesh = Some(true);
    }

    // ========== OBJ 导出：默认走缓存，SurrealDB 需显式开关 ==========
    //
    // 约定：
    // - 默认（不传 --use-surrealdb）时，OBJ 导出/截图相关路径不依赖 SurrealDB；
    // - 若需走 SurrealDB（用于对照/迁移验证），必须显式添加 --use-surrealdb；
    // - 这不是 fallback：cache 与 surrealdb 两条路径同时存在仅用于验证准确性。
    let obj_export_flow = matches.get_flag("export-obj")
        || matches.get_flag("export-svg")
        || matches.contains_id("export-obj-refnos")
        || (debug_model_requested && capture_dir.is_some());

    if obj_export_flow {
        let cli_use_surrealdb = matches.get_flag("use-surrealdb");
        db_option_ext.use_surrealdb = cli_use_surrealdb;
        if cli_use_surrealdb {
            // SurrealDB-only：用于对照验证，避免与 cache 混用
            db_option_ext.use_cache = false;
        }

        if !db_option_ext.use_cache && !db_option_ext.use_surrealdb {
            anyhow::bail!(
                "OBJ 导出默认关闭 SurrealDB，但当前配置未启用 use_cache；请开启 use_cache 或添加 --use-surrealdb"
            );
        }

        if !db_option_ext.use_surrealdb {
            println!(
                "📦 cache-only：OBJ 导出默认使用 indextree + foyer 缓存（instances 从缓存读，不写入 inst_*）。SurrealDB 仍作为输入数据源连接（PE/属性/世界矩阵等）。如需写入/对照验证，请添加 --use-surrealdb。"
            );
        }
    }

    // ========== 处理 --debug-model 与导出标志的组合 ==========
    if let Some(refnos_vec) = &debug_model_refnos {
        // 如果用户开启了 --capture 但没有指定任何导出标志，则默认走 OBJ 导出流程：
        // - export_obj_mode 内部会在需要时触发模型生成（配合 --regen-model 可强制重建）
        // - gen_all_geos_data 末尾会调用 capture_refnos_if_enabled 生成截图
        //
        // 这样可以保证 `--debug-model ... --capture ...` 的行为稳定且符合“生成模型并截图”的预期。
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
                matches.get_flag("regen-model"),
                None,
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            return export_obj_mode(config, &db_option_ext).await;
        }

        // 检查是否有导出标志
        if matches.get_flag("export-obj") {
            println!("🎯 导出 OBJ 模型 (调试模式): {:?}", refnos_vec);
            let config = build_export_config(
                refnos_vec.clone(),
                output_path,
                filter_nouns,
                include_descendants,
                source_unit,
                target_unit,
                verbose,
                matches.get_flag("regen-model"),
                None,
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            return export_obj_mode(config, &db_option_ext).await;
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
                matches.get_flag("regen-model"),
                None,
                split_by_site,
                include_negative,
                true, // export_svg = true
            );
            return export_obj_mode(config, &db_option_ext).await;
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
                matches.get_flag("regen-model"),
                None,
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            config.use_basic_materials = use_basic_materials;
            return export_glb_mode(config, &db_option_ext).await;
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
                matches.get_flag("regen-model"),
                None,
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            config.use_basic_materials = use_basic_materials;
            return export_gltf_mode(config, &db_option_ext).await;
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
                matches.get_flag("regen-model"),
                Some(dbnum),
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            return export_obj_mode(config, &db_option_ext).await;
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
                matches.get_flag("regen-model"),
                Some(dbnum),
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            config.use_basic_materials = use_basic_materials;
            return export_glb_mode(config, &db_option_ext).await;
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
                matches.get_flag("regen-model"),
                Some(dbnum),
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            config.use_basic_materials = use_basic_materials;
            return export_gltf_mode(config, &db_option_ext).await;
        }
    }

    // no-dbnum 情况的默认“全库导出”由各导出模式内部处理（config.run_all_dbnos）

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
                matches.get_flag("regen-model"),
                None,
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            return export_obj_mode(config, &db_option_ext).await;
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
            return export_glb_mode(config, &db_option_ext).await;
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
            return export_gltf_mode(config, &db_option_ext).await;
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
            matches.get_flag("regen-model"),
            use_basic_materials,
            split_by_site,
            include_negative,
            matches.get_flag("export-svg"),
        );
        return export_gltf_mode(config, &db_option_ext).await;
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
            matches.get_flag("regen-model"),
            use_basic_materials,
            split_by_site,
            include_negative,
            matches.get_flag("export-svg"),
        );
        return export_glb_mode(config, &db_option_ext).await;
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
            matches.get_flag("regen-model"),
            use_basic_materials,
            split_by_site,
            include_negative,
            matches.get_flag("export-svg"),
        );
        return export_obj_mode(config, &db_option_ext).await;
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

        println!("🎯 导出 dbnum 实例数据为 JSON（含 AABB）");
        println!("   - 按 dbnum={} 过滤", dbnum);
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

        let room_keywords: Option<Vec<String>> = matches
            .get_many::<String>("room-keywords")
            .map(|kws| kws.map(|s| s.to_string()).collect());

        let force_rebuild = matches.get_flag("room-force-rebuild");

        let db_nums: Option<Vec<u32>> = matches
            .get_many::<String>("room-db-nums")
            .map(|nums| nums.filter_map(|s| s.parse::<u32>().ok()).collect());

        println!("🏠 启动房间计算模式");
        if let Some(ref kws) = room_keywords {
            println!("   - 房间关键词: {:?}", kws);
        }
        if let Some(ref nums) = db_nums {
            println!("   - 数据库编号: {:?}", nums);
        }
        println!("   - 强制重建: {}", force_rebuild);

        return room_compute_mode(
            room_keywords,
            db_nums,
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

    // ========== 处理 --debug-model + --capture 但无导出标志的情况 ==========
    // 如果使用了 --debug-model 和 --capture 但没有指定导出标志，直接触发模型生成和 capture
    if debug_model_requested && capture_dir.is_some() {
        if let Some(refnos_vec) = &debug_model_refnos {
            if !refnos_vec.is_empty() {
                println!(
                    "🎯 调试模式 + 截图模式：生成模型并截图 (无导出标志): {:?}",
                    refnos_vec
                );
                // 确保 gen_mesh 已启用（已在前面设置）
                // 直接调用 run_app，它会通过 run_cli -> gen_all_geos_data 生成模型
                // gen_all_geos_data 会检查 debug_model_refnos 并生成模型，然后触发 capture
                return run_app(Some(db_option_ext)).await;
            }
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
