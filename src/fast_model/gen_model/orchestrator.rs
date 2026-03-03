//! 模型生成编排器

//!

//! 负责协调整个模型生成流程：

//! - IndexTree 单管线路由（Full / Manual / Debug / Incremental）

//! - 几何体生成、Mesh 生成、布尔运算的编排

//! - 增量更新、手动 refno、调试模式的处理

//! - 空间索引和截图捕获的触发

use std::collections::{BTreeSet, HashMap, HashSet};

use std::path::{Path, PathBuf};

use std::sync::Arc;

use std::time::Instant;

use crate::fast_model::export_model::export_prepack_lod::export_prepack_lod_for_refnos;

use crate::fast_model::export_model::export_prepack_lod::export_instances_json_for_dbnos;

use crate::fast_model::export_model::export_prepack_lod::export_instances_json_for_refnos_grouped_by_dbno;

use crate::fast_model::unit_converter::LengthUnit;

use aios_core::RefnoEnum;

use crate::data_interface::increment_record::IncrGeoUpdateLog;

use crate::data_interface::sesno_increment::get_changes_at_sesno;

// use crate::fast_model::capture::capture_refnos_if_enabled; // removed on foyer-cache-cleanup

use crate::data_interface::db_meta_manager::db_meta;

use crate::fast_model::mesh_generate::{
    run_boolean_worker,
    MeshTask, MeshResult, MeshWorkerReport, RecentGeoDeduper,
    extract_mesh_tasks, generate_meshes_for_batch, query_existing_inst_geo_ids,
    run_mesh_worker_from_channel,
};
use crate::fast_model::gen_model::boolean_task::{BooleanTask, BooleanTaskAccumulator};
use crate::fast_model::gen_model::manifold_bool::run_bool_worker_from_tasks;

use crate::fast_model::pdms_inst::save_instance_data_optimize;
use crate::fast_model::pdms_inst::{save_instance_data_to_sql_file, InstRelatePrecomputed};

use crate::options::{BooleanPipelineMode, DbOptionExt, MeshFormat};

#[cfg(feature = "parquet-export")]
use crate::fast_model::export_model::ParquetStreamWriter;

#[cfg(feature = "duckdb-feature")]

use crate::fast_model::export_model::{DuckDBStreamWriter, DuckDBWriteMode};

use super::config::IndexTreeConfig;

use super::errors::{IndexTreeError, Result};

use super::cache_miss_report;

use super::index_tree_mode::gen_index_tree_geos_optimized;

use super::models::NounCategory;

use crate::fast_model::gen_model::tree_index_manager::TreeIndexManager;

use aios_core::tool::db_tool::db1_hash;

/// 按 dbnum 拆分一个 batch，保证写入 InstanceCache 时“一个 batch 只落到一个 dbnum 分桶”。
///
/// 说明：
/// - 这里不尝试“从 ref0 推 dbnum”，必须通过 TreeIndexManager 映射。
/// - 若某个 refno 无法映射 dbnum：直接返回 Err（避免悄然写错桶）。
pub(crate) async fn split_shape_instances_by_dbnum(
    shape_insts: &aios_core::geometry::ShapeInstancesData,
) -> anyhow::Result<HashMap<u32, aios_core::geometry::ShapeInstancesData>> {
    use aios_core::geometry::ShapeInstancesData;

    let mut out: HashMap<u32, ShapeInstancesData> = HashMap::new();
    let mut cache: HashMap<RefnoEnum, u32> = HashMap::new();
    let mut missing_by_source: HashMap<&'static str, usize> = HashMap::new();
    let mut missing_refnos: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    async fn get_dbnum_cached(
        refno: RefnoEnum,
        source: &'static str,
        cache: &mut HashMap<RefnoEnum, u32>,
        missing_by_source: &mut HashMap<&'static str, usize>,
        missing_refnos: &mut std::collections::BTreeSet<String>,
    ) -> Option<u32> {
        if let Some(v) = cache.get(&refno) {
            return Some(*v);
        }
        let dbnum = TreeIndexManager::resolve_dbnum_for_refno(refno)
            .ok();
        if let Some(dbnum) = dbnum {
            cache.insert(refno, dbnum);
            return Some(dbnum);
        }
        *missing_by_source.entry(source).or_insert(0) += 1;
        missing_refnos.insert(refno.to_string());
        None
    }

    fn summarize_missing_sources(missing_by_source: &HashMap<&'static str, usize>) -> String {
        let mut parts: Vec<String> = missing_by_source
            .iter()
            .map(|(k, v)| format!("{k}:{v}"))
            .collect();
        parts.sort();
        parts.join(", ")
    }

    fn summarize_missing_samples(
        missing_refnos: &std::collections::BTreeSet<String>,
        max_n: usize,
    ) -> String {
        missing_refnos
            .iter()
            .take(max_n)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    }

    // inst_info
    for (refno, info) in shape_insts.inst_info_map.iter() {
        let refno = *refno;
        let Some(dbnum) = get_dbnum_cached(
            refno,
            "inst_info.key",
            &mut cache,
            &mut missing_by_source,
            &mut missing_refnos,
        )
        .await
        else {
            continue;
        };
        out.entry(dbnum)
            .or_insert_with(ShapeInstancesData::default)
            .inst_info_map
            .insert(refno, info.clone());
    }

    // inst_tubi
    for (refno, tubi) in shape_insts.inst_tubi_map.iter() {
        let refno = *refno;
        let Some(dbnum) = get_dbnum_cached(
            refno,
            "inst_tubi.key",
            &mut cache,
            &mut missing_by_source,
            &mut missing_refnos,
        )
        .await
        else {
            continue;
        };
        out.entry(dbnum)
            .or_insert_with(ShapeInstancesData::default)
            .inst_tubi_map
            .insert(refno, tubi.clone());
    }

    // inst_geos：每条 geos_data 都绑定一个 refno（元素），直接按 geos_data.refno 分桶。
    for (inst_key, geos_data) in shape_insts.inst_geos_map.iter() {
        let inst_key = inst_key.clone();
        let geos_data = geos_data.clone();
        let Some(dbnum) = get_dbnum_cached(
            geos_data.refno,
            "inst_geos.refno",
            &mut cache,
            &mut missing_by_source,
            &mut missing_refnos,
        )
        .await
        else {
            continue;
        };
        out.entry(dbnum)
            .or_insert_with(ShapeInstancesData::default)
            .inst_geos_map
            .insert(inst_key, geos_data);
    }

    // neg_relate / ngmr_neg_relate：按 key(refno) 分桶
    for (refno, v) in &shape_insts.neg_relate_map {
        let Some(dbnum) = get_dbnum_cached(
            *refno,
            "neg_relate.key",
            &mut cache,
            &mut missing_by_source,
            &mut missing_refnos,
        )
        .await
        else {
            continue;
        };
        out.entry(dbnum)
            .or_insert_with(ShapeInstancesData::default)
            .neg_relate_map
            .insert(*refno, v.clone());
    }
    for (refno, v) in &shape_insts.ngmr_neg_relate_map {
        let Some(dbnum) = get_dbnum_cached(
            *refno,
            "ngmr_neg_relate.key",
            &mut cache,
            &mut missing_by_source,
            &mut missing_refnos,
        )
        .await
        else {
            continue;
        };
        out.entry(dbnum)
            .or_insert_with(ShapeInstancesData::default)
            .ngmr_neg_relate_map
            .insert(*refno, v.clone());
    }

    if !missing_refnos.is_empty() {
        let source_summary = summarize_missing_sources(&missing_by_source);
        let sample = summarize_missing_samples(&missing_refnos, 8);
        return Err(anyhow::anyhow!(
            "缺少 ref0->dbnum 映射: unique_refnos={}, sources=[{}], sample=[{}]",
            missing_refnos.len(),
            source_summary,
            sample
        ));
    }

    Ok(out)
}

#[derive(Debug, Clone)]
enum GenerationScope {
    Full,
    Manual { roots: Vec<RefnoEnum> },
    Debug { roots: Vec<RefnoEnum> },
    Incremental { log: IncrGeoUpdateLog },
}

fn decide_generation_scope(
    manual_refnos: &[RefnoEnum],
    debug_roots: &[RefnoEnum],
    has_incr_log: bool,
    incr_visible_roots: &[RefnoEnum],
    incr_updates: Option<&IncrGeoUpdateLog>,
) -> GenerationScope {
    let has_manual = !manual_refnos.is_empty();
    let has_debug = !debug_roots.is_empty();

    if has_manual && !has_debug && !has_incr_log {
        return GenerationScope::Manual {
            roots: manual_refnos.to_vec(),
        };
    }

    if has_debug && !has_manual && !has_incr_log {
        return GenerationScope::Debug {
            roots: debug_roots.to_vec(),
        };
    }

    if has_incr_log && !has_manual && !has_debug {
        return GenerationScope::Incremental {
            log: incr_updates.cloned().unwrap_or_default(),
        };
    }

    if has_manual || has_debug || has_incr_log {
        let mut merged: HashSet<RefnoEnum> = HashSet::new();
        merged.extend(manual_refnos.iter().copied());
        merged.extend(debug_roots.iter().copied());
        merged.extend(incr_visible_roots.iter().copied());
        return GenerationScope::Manual {
            roots: merged.into_iter().collect(),
        };
    }

    GenerationScope::Full
}

async fn collect_db_write_failures(db_write_handles: Vec<tokio::task::JoinHandle<bool>>) -> usize {
    let mut db_write_failures = 0usize;
    for h in db_write_handles {
        match h.await {
            Ok(true) => {}
            Ok(false) => db_write_failures += 1,
            Err(e) => {
                eprintln!("等待写库任务失败: {}", e);
                db_write_failures += 1;
            }
        }
    }
    db_write_failures
}

fn ensure_no_db_write_failures(db_write_failures: usize) -> anyhow::Result<()> {
    if db_write_failures > 0 {
        return Err(anyhow::anyhow!(
            "SurrealDB 批量写入存在失败任务: {}",
            db_write_failures
        ));
    }
    Ok(())
}

#[derive(Debug, Default)]
struct InsertHandleReport {
    batch_cnt: u64,
    bool_tasks: Vec<BooleanTask>,
}

#[derive(Debug, Clone)]
pub struct GenModelResult {
    pub success: bool,
    pub deferred_sql_path: Option<PathBuf>,
}

/// 主入口函数：生成所有几何体数据

///

/// 这是主要的公共 API，统一收敛到 IndexTree 生成管线：
/// - Full：按 `index_tree_enabled_target_types` 从 TreeIndex 提取入口 roots
/// - Manual / Debug / Incremental：构造 roots 并集后以 seed_roots 直入

///

/// # Arguments

/// * `manual_refnos` - 手动指定的 refno 列表

/// * `db_option` - 数据库配置

/// * `incr_updates` - 增量更新日志

/// * `target_sesno` - 目标 sesno

#[cfg_attr(feature = "profile", tracing::instrument(skip_all, name = "gen_all_geos_data"))]

pub async fn gen_all_geos_data(

    manual_refnos: Vec<RefnoEnum>,

    db_option: &DbOptionExt,

    incr_updates: Option<IncrGeoUpdateLog>,

    target_sesno: Option<u32>,
) -> Result<GenModelResult> {

    let time = Instant::now();

    let mut perf = crate::perf_timer::PerfTimer::new("gen_all_geos_data");

    perf.mark("init");

    // cache-first 缺失报告：生成过程中按需补充记录，结束时输出到 output/<project>/cache_miss_report.json
    // cache-first 模式已移除（foyer-cache-cleanup），使用 Direct 模式
    cache_miss_report::init_global_cache_miss_report(db_option, "Direct");

    let mut final_incr_updates = incr_updates;

    // 如果指定了 target_sesno，获取该 sesno 的增量数据

    if let Some(sesno) = target_sesno {

        if !db_option.use_surrealdb {

            return Err(IndexTreeError::Other(anyhow::anyhow!(

                "cache-only 模式下不支持 --target-sesno（需要从 SurrealDB 获取 element_changes）：sesno={}",

                sesno

            )));

        }

        if final_incr_updates.is_none() {

            match get_changes_at_sesno(sesno).await {

                Ok(sesno_changes) => {

                    if sesno_changes.count() > 0 {

                        final_incr_updates = Some(sesno_changes);

                    } else {

                        println!("[gen_model] sesno {} 没有发现变更，跳过增量生成", sesno);

                        return Ok(GenModelResult {
                            success: false,
                            deferred_sql_path: None,
                        });

                    }

                }

                Err(e) => {

                    eprintln!("获取 sesno {} 的变更失败: {}", sesno, e);

                    return Err(IndexTreeError::Other(e));

                }

            }

        }

    }

    let incr_count = final_incr_updates

        .as_ref()

        .map(|log| log.count())

        .unwrap_or(0);

    println!(

        "[gen_model] 启动 gen_all_geos_data: manual_refnos={}, incr_updates={}, target_sesno={:?}",

        manual_refnos.len(),

        incr_count,

        target_sesno,

    );

    // 性能剖析：尽量在最上层启用 tracing，覆盖 precheck -> gen_model -> mesh -> room 计算全链路。

    #[cfg(feature = "profile")]

    let _ = crate::profiling::init_chrome_tracing_for_db_option(db_option, "full_flow_room");

    perf.mark("precheck");

    // ✨ 执行预检查：确保 Tree 文件、pe_transform、db_meta_info 就绪

    if db_option.use_surrealdb {

        use crate::fast_model::gen_model::precheck_coordinator::{run_precheck, PrecheckConfig};

        let precheck_config = PrecheckConfig {

            enabled: true,

            check_tree: true,

            check_pe_transform: true,

            check_db_meta: true,

            tree_output_dir: db_option.get_project_output_dir().join("scene_tree").to_string_lossy().to_string(),

        };

        match run_precheck(db_option, Some(precheck_config)).await {

            Ok(stats) => {

                log::info!("[gen_model] 预检查完成: {:?}", stats);

            }

            Err(e) => {

                log::warn!("[gen_model] 预检查部分失败: {}", e);

                // 不阻断流程，继续执行

            }

        }

    } else {

        // cache-only 模式：仅检查 db_meta_info
        let _ = db_meta().ensure_loaded();

    }

    // 调试：打印 IndexTree 模式配置

    println!(
        "[gen_model] IndexTree 默认管线配置: concurrency={}, batch_size={}",
        db_option.get_index_tree_concurrency(),
        db_option.get_index_tree_batch_size()
    );

    // ✅ SurrealDB 写入侧初始化：仅在 use_surrealdb=true 时需要。

    if db_option.use_surrealdb && !db_option.defer_db_write {

        if let Err(e) = aios_core::rs_surreal::inst::init_model_tables().await {

            eprintln!("[gen_model] ❌ 初始化 inst_relate 表结构失败: {}", e);

            // 严重错误，建议直接中断，否则后续写入必挂

            return Err(IndexTreeError::Other(e));

        }

    }

    // =========================

    // LOOP/PRIM 输入缓存初始化（按环境变量启用）

    // =========================

    // geom_input_cache 已移除（foyer-cache-cleanup），跳过缓存初始化
    println!("[gen_model] geom_input_cache: Direct 模式（cache 已移除）");

    // =========================

    // IndexTree 模式：新管线

    // =========================

    // 统一入口：manual/debug/incr/full 全部收敛到 IndexTree 生成管线

    perf.mark("route_decision");
    let debug_roots = db_option.inner.get_all_debug_refnos().await;
    let incr_visible_roots: Vec<RefnoEnum> = final_incr_updates
        .as_ref()
        .map(|log| log.get_all_visible_refnos().into_iter().collect())
        .unwrap_or_default();
    let has_incr_log = final_incr_updates
        .as_ref()
        .map(|log| log.count() > 0)
        .unwrap_or(false);
    let has_incr_visible_roots = !incr_visible_roots.is_empty();

    let scope = decide_generation_scope(
        &manual_refnos,
        &debug_roots,
        has_incr_log,
        &incr_visible_roots,
        final_incr_updates.as_ref(),
    );

    if matches!(scope, GenerationScope::Incremental { .. }) && !has_incr_visible_roots {
        println!(
            "[gen_model] 增量日志存在但未解析到可见 roots，将按 Incremental 空 roots 路径执行（不会回退 Full）"
        );
    }

    let input_source_cnt =
        (!manual_refnos.is_empty() as u8) + (!debug_roots.is_empty() as u8) + (has_incr_log as u8);
    if input_source_cnt >= 2 {
        if let GenerationScope::Manual { roots } = &scope {
            println!(
                "[gen_model] 检测到混合输入(manual/debug/incr)，按 roots 并集执行：{} 个",
                roots.len()
            );
        }
    }

    perf.mark("index_tree_generation");
    let result = process_index_tree_generation(scope, db_option, target_sesno, time).await;

    perf.print_summary();

    // 输出 cache miss 报告（覆盖写）。
    if let Some(report) = cache_miss_report::snapshot_global_report() {
        match report.write_to_default_path(db_option) {
            Ok(path) => {
                println!(
                    "[gen_model] cache_miss_report 已写入: {} (mode={})",
                    path.display(),
                    report.mode
                );
            }
            Err(e) => {
                eprintln!("[gen_model] 写入 cache_miss_report 失败: {}", e);
            }
        }
    } else {
        eprintln!("[gen_model] cache_miss_report 未初始化，跳过写入");
    }

    result

}

async fn filter_bran_hang_refnos(refnos: &[RefnoEnum]) -> Vec<RefnoEnum> {

    let bran_hash = db1_hash("BRAN");

    let hang_hash = db1_hash("HANG");

    let mut out = Vec::new();

    for &r in refnos {

        if !r.is_valid() {

            continue;

        }

        let dbnum = match TreeIndexManager::resolve_dbnum_for_refno(r) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let manager = TreeIndexManager::with_default_dir(vec![dbnum]);

        let Ok(index) = manager.load_index(dbnum) else {

            continue;

        };

        let Some(meta) = index.node_meta(r.refno()) else {

            continue;

        };

        if meta.noun == bran_hash || meta.noun == hang_hash {

            out.push(r);

        }

    }

    out

}

/// 处理 IndexTree 模式的生成流程

async fn process_index_tree_generation(
    scope: GenerationScope,
    db_option: &DbOptionExt,
    _target_sesno: Option<u32>,
    time: Instant,
) -> Result<GenModelResult> {
    let mut perf = crate::perf_timer::PerfTimer::new("index_tree_generation");

    perf.mark("init");

    println!("[gen_model] 进入 IndexTree 生成模式（统一管线）");

    if db_option.manual_db_nums.is_some() || db_option.exclude_db_nums.is_some() {

        println!(

            "[gen_model] 提示: IndexTree 新管线已支持 manual_db_nums / exclude_db_nums 过滤，当前仍按配置执行"

        );

    }

    let seed_roots = match &scope {
        GenerationScope::Full => {
            println!("[gen_model] 当前 scope: Full（按 target_type 入口查询 roots）");
            None
        }
        GenerationScope::Manual { roots } => {
            println!("[gen_model] 当前 scope: Manual roots={}", roots.len());
            Some(roots.clone())
        }
        GenerationScope::Debug { roots } => {
            println!("[gen_model] 当前 scope: Debug roots={}", roots.len());
            Some(roots.clone())
        }
        GenerationScope::Incremental { log } => {
            let roots: Vec<RefnoEnum> = log.get_all_visible_refnos().into_iter().collect();
            println!("[gen_model] 当前 scope: Incremental roots={}", roots.len());
            Some(roots)
        }
    };

    let full_start = Instant::now();

    perf.mark("categorize_and_inst_relate");

    // 1️⃣ 生成/更新 inst_relate，并获取分类后的根 refno

    let config = IndexTreeConfig::from_db_option_ext(db_option)

        .map_err(|e| anyhow::anyhow!("配置错误: {}", e))?;

    let (sender, receiver) = flume::bounded::<aios_core::geometry::ShapeInstancesData>(100);

    let replace_exist = db_option.inner.is_replace_mesh();

    // 🧹 预处理清理：在生成前一次性删除目标 refnos 的旧模型记录，
    // 避免生成过程中 DELETE + INSERT IGNORE 与 mesh worker 的竞态条件。
    // defer_db_write 模式下也需要清理：DELETE 直接写 DB，新 INSERT 走 deferred .surql 文件
    if replace_exist && db_option.use_surrealdb {
        if let Some(ref roots) = seed_roots {
            if !roots.is_empty() {
                perf.mark("pre_cleanup_for_regen");
                if let Err(e) = super::pdms_inst::pre_cleanup_for_regen(roots).await {
                    eprintln!("[gen_model] ⚠️ pre_cleanup_for_regen 失败（继续生成）: {}", e);
                }
            }
        }
    }

    let use_surrealdb = db_option.use_surrealdb;
    let defer_db_write = db_option.defer_db_write;

    // defer_db_write 模式：初始化 SqlFileWriter
    let sql_file_writer: Option<Arc<super::sql_file_writer::SqlFileWriter>> = if defer_db_write {
        let output_dir = db_option.get_project_output_dir();
        let path = super::sql_file_writer::SqlFileWriter::default_path(&output_dir, None);
        match super::sql_file_writer::SqlFileWriter::new(&path) {
            Ok(w) => {
                println!("[gen_model] 🗂️ defer_db_write 模式已启用，SQL 输出到: {}", path.display());
                Some(Arc::new(w))
            }
            Err(e) => {
                eprintln!("[gen_model] ❌ 创建 SqlFileWriter 失败: {}", e);
                return Err(IndexTreeError::Other(e));
            }
        }
    } else {
        None
    };

    // Mesh 生成：内联模式，每批次先生成 mesh 再写 DB，消除 UPDATE 竞态
    let gen_mesh = db_option.inner.gen_mesh;

    // 初始化 Parquet 写入器（默认关闭，通过环境变量显式开启）。
    //
    // 开关：AIOS_ENABLE_PARQUET_STREAM_WRITER=1|true|yes|on
    // 说明：此前该路径固定为 None，容易造成“看似支持但实际未启用”的误解；
    // 这里改为显式开关，默认行为保持不变（关闭）。
    let enable_parquet_stream_writer = std::env::var("AIOS_ENABLE_PARQUET_STREAM_WRITER")
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);

    // ParquetStreamWriter 需要 parquet-export feature
    #[cfg(feature = "parquet-export")]
    let parquet_writer = if enable_parquet_stream_writer {
        let output_dir = db_option.inner.meshes_path.as_deref().unwrap_or("assets/meshes");
        let parquet_dir = std::path::Path::new(output_dir)
            .parent()
            .unwrap_or(std::path::Path::new("output"));

        match ParquetStreamWriter::new(parquet_dir) {
            Ok(writer) => {
                println!(
                    "[Parquet] 已启用流式写入（AIOS_ENABLE_PARQUET_STREAM_WRITER=1），输出目录: {}",
                    parquet_dir.display()
                );
                Some(std::sync::Arc::new(writer))
            }
            Err(e) => {
                eprintln!("[Parquet] 初始化写入器失败: {}, 回退为禁用", e);
                None
            }
        }
    } else {
        println!(
            "[Parquet] 流式写入已禁用（可设置 AIOS_ENABLE_PARQUET_STREAM_WRITER=1 显式开启）"
        );
        None
    };
    #[cfg(not(feature = "parquet-export"))]
    let parquet_writer: Option<std::sync::Arc<()>> = None;

    #[cfg(feature = "duckdb-feature")]

    let duckdb_writer: Option<std::sync::Arc<DuckDBStreamWriter>> = None;

    /*

    #[cfg(feature = "duckdb-feature")]

    let duckdb_writer = {

        let output_dir = db_option.inner.meshes_path.as_deref().unwrap_or("assets/meshes");

        let parquet_dir = std::path::Path::new(output_dir).parent().unwrap_or(std::path::Path::new("output"));

        let duckdb_dir = parquet_dir.join("database_models/_global");

        match DuckDBStreamWriter::new(&duckdb_dir, DuckDBWriteMode::Rebuild) {

            Ok(writer) => Some(std::sync::Arc::new(writer)),

            Err(e) => {

                eprintln!("[DuckDB] 初始化写入器失败: {}, 跳过 DuckDB 导出", e);

                None

            }

        }

    };

    */

    #[allow(unused_variables)]
    let parquet_writer_clone = parquet_writer.clone();

    #[cfg(feature = "duckdb-feature")]

    let duckdb_writer_clone = duckdb_writer.clone();

    // model cache-only 已移除（foyer-cache-cleanup）
    let model_cache_ctx: Option<()> = None;
    #[allow(unused_variables)]
    let cache_manager_for_insert: Option<()> = None;

    let touched_dbnums: Arc<std::sync::Mutex<BTreeSet<u32>>> =

        Arc::new(std::sync::Mutex::new(BTreeSet::new()));

    let touched_dbnums_for_insert = touched_dbnums.clone();

    // IndexTree 下用于 inst_relate_aabb 写入的 refno 集合：只收集“本次生成触达”的实例，
    // 避免通过 pe_transform 全库扫描导致卡死/耗时失真。
    let touched_refnos: Arc<std::sync::Mutex<std::collections::HashSet<RefnoEnum>>> =
        Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));
    let touched_refnos_for_insert = touched_refnos.clone();

    // 当 manual_db_nums 只有一个值时，直接使用该 dbnum，无需从 refno 反推

    let known_dbnum: Option<u32> = db_option.inner.manual_db_nums.as_ref()

        .filter(|nums| nums.len() == 1)

        .and_then(|nums| nums.first().copied());

    let sql_writer_clone = sql_file_writer.clone();
    let db_option_inner = db_option.inner.clone();

    let insert_handle = tokio::spawn(async move {

        #[cfg(feature = "profile")]

        let sink_span = tracing::info_span!("instance_sink");

        let mut batch_cnt: u64 = 0;

        let mut t_save_db = std::time::Duration::ZERO;

        let mut t_cache = std::time::Duration::ZERO;

        let mut t_parquet = std::time::Duration::ZERO;

        #[cfg(feature = "duckdb-feature")]

        let mut t_duckdb = std::time::Duration::ZERO;

        // SurrealDB 写入后台任务句柄：不阻塞 cache 写入和后续 batch 接收
        let mut db_write_handles: Vec<tokio::task::JoinHandle<bool>> = Vec::new();
        let mut db_write_failures: usize = 0;
        // 控制 SurrealDB 后台写入的最大并发数
        let db_write_semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
        let mut bool_accumulator = BooleanTaskAccumulator::default();

        // Mesh 内联生成所需的跨批次共享状态
        let mut mesh_deduper = RecentGeoDeduper::new(200_000);
        let mesh_aabb_map: Arc<dashmap::DashMap<String, parry3d::bounding_volume::Aabb>> = Arc::new(dashmap::DashMap::new());
        let mesh_pts_map: Arc<dashmap::DashMap<u64, String>> = Arc::new(dashmap::DashMap::new());
        let mut t_mesh = std::time::Duration::ZERO;
        let mut mesh_total = 0usize;
        // 预加载 mesh 缓存 + 已有 inst_geo IDs（一次性）
        if gen_mesh {
            if let Err(e) = crate::fast_model::preload_mesh_cache().await {
                eprintln!("[mesh_inline] 预加载 mesh 缓存失败: {}", e);
            }
            match query_existing_inst_geo_ids().await {
                Ok(ids) => {
                    let count = ids.len();
                    mesh_deduper.preload(ids);
                    println!("[mesh_inline] 预加载 {} 个已有 inst_geo ID 到去重器 (capacity={})", count, mesh_deduper.capacity);
                }
                Err(e) => eprintln!("[mesh_inline] 预加载 inst_geo IDs 失败: {}", e),
            }
        }

        loop {

            let Ok(shape_insts) = receiver.recv_async().await else {

                break;

            };

            let shape_insts_arc = std::sync::Arc::new(shape_insts);

            #[cfg(feature = "profile")]
            let _enter = sink_span.enter();

            batch_cnt += 1;

            // 记录本批次触达的实例 refno（用于后续 inst_relate_aabb 写入范围收敛）
            {
                let mut guard = touched_refnos_for_insert.lock().unwrap();
                for r in shape_insts_arc.inst_info_map.keys() {
                    guard.insert(*r);
                }
                for r in shape_insts_arc.inst_tubi_map.keys() {
                    guard.insert(*r);
                }
            }

            // [foyer-removal] cache_manager 已移除，跳过 insert_from_shape
            let _ = &cache_manager_for_insert;

            // 同时写入 Parquet（如果启用）

            // [foyer-removal] parquet_writer 已移除，跳过 write_batch
            let _ = &parquet_writer_clone;

            #[cfg(feature = "duckdb-feature")]

            if let Some(ref writer) = duckdb_writer_clone {

                let t0 = Instant::now();

                if let Err(e) = writer.write_batch(&shape_insts_arc) {

                    eprintln!("[DuckDB] 写入批次失败: {}", e);

                }

                t_duckdb += t0.elapsed();

            }

            // Mesh 内联生成：先生成 mesh，再将结果合并到 inst_geo INSERT 中
            let mesh_results: HashMap<u64, MeshResult> = if gen_mesh {
                let tasks = extract_mesh_tasks(&shape_insts_arc);
                if !tasks.is_empty() {
                    let t0 = Instant::now();
                    let results = generate_meshes_for_batch(
                        &tasks, &db_option_inner, &mut mesh_deduper,
                        &mesh_aabb_map, &mesh_pts_map,
                    ).await;
                    mesh_total += results.len();
                    t_mesh += t0.elapsed();
                    results
                } else {
                    HashMap::new()
                }
            } else {
                HashMap::new()
            };

            // 布尔任务跨批次汇总：统一在 insert_handle 结束后一次性抽取，避免漏任务。
            bool_accumulator.merge_batch(&shape_insts_arc);

            // SurrealDB 写入放到后台，不阻塞 cache 写入和后续 batch 接收
            // 采用 Semaphore 限流，防止瞬发海量并发协程打垮数据库导致事务冲突风暴
            // 此处在 spawn 外侧 acquire，当达到并发上限时直接施加回压（Backpressure），阻塞 recv 接收
            if defer_db_write {
                // defer_db_write 模式：SQL 写入文件，不写 SurrealDB
                if let Some(ref writer) = sql_writer_clone {
                    let t0 = Instant::now();
                    // 收集本批次的 refnos 用于预计算
                    let batch_refnos: Vec<aios_core::RefnoEnum> =
                        shape_insts_arc.inst_info_map.keys().copied().collect();
                    let precomputed = InstRelatePrecomputed::build(&batch_refnos).await;
                    if let Err(e) = save_instance_data_to_sql_file(
                        &shape_insts_arc,
                        replace_exist,
                        writer,
                        &precomputed,
                        &mesh_results,
                    ).await {
                        eprintln!("[defer_db_write] 写入 SQL 文件失败: {}", e);
                    }
                    t_save_db += t0.elapsed();
                }
            } else if use_surrealdb {
                let t0 = Instant::now();
                
                // 在主循环中 acquire 以提供反压
                let permit = match db_write_semaphore.clone().acquire_owned().await {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("获取写库并发锁失败: {}", e);
                        continue;
                    }
                };
                
                let shape_insts_clone = shape_insts_arc.clone();
                db_write_handles.push(tokio::spawn(async move {
                    let _permit_holder = permit; // 离开作用域时自动释放信号量

                    if let Err(e) = save_instance_data_optimize(&shape_insts_clone, replace_exist, &mesh_results).await {
                        eprintln!("保存实例数据失败: {}", e);
                        return false;
                    }
                    true
                }));
                t_save_db += t0.elapsed();
            }

        }

        // 等待所有 SurrealDB 后台写入完成
        if !db_write_handles.is_empty() {
            let t_wait = Instant::now();
            let total = db_write_handles.len();
            db_write_failures += collect_db_write_failures(db_write_handles).await;
            let wait_ms = t_wait.elapsed().as_millis();
            if wait_ms > 100 {
                println!(
                    "[gen_model] SurrealDB 后台写入等待完成: {} 个任务, 额外等待 {} ms",
                    total, wait_ms
                );
            }
        }

        // 保存 mesh 生成的 aabb/pts 数据
        if gen_mesh && (!mesh_aabb_map.is_empty() || !mesh_pts_map.is_empty()) {
            if let Some(ref writer) = sql_writer_clone {
                // defer 模式：写入 .surql 文件
                if !mesh_aabb_map.is_empty() {
                    let keys: Vec<String> = mesh_aabb_map.iter().map(|kv| kv.key().clone()).collect();
                    for chunk in keys.chunks(300) {
                        let mut rows: Vec<String> = Vec::with_capacity(chunk.len());
                        for k in chunk {
                            let v = mesh_aabb_map.get(k).unwrap();
                            let d = serde_json::to_string(v.value()).unwrap();
                            let id_key = if k.starts_with("aabb:") {
                                k.to_string()
                            } else {
                                format!("aabb:⟨{}⟩", k)
                            };
                            rows.push(format!("{{'id':{id_key}, 'd':{d}}}"));
                        }
                        let sql = format!("INSERT IGNORE INTO aabb [{}]", rows.join(","));
                        let _ = writer.write_statement(&sql);
                    }
                }
                if !mesh_pts_map.is_empty() {
                    let keys: Vec<u64> = mesh_pts_map.iter().map(|kv| *kv.key()).collect();
                    for chunk in keys.chunks(100) {
                        let mut rows: Vec<String> = Vec::with_capacity(chunk.len());
                        for &k in chunk {
                            let v = mesh_pts_map.get(&k).unwrap();
                            rows.push(format!("{{'id':vec3:⟨{}⟩, 'd':{}}}", k, v.value()));
                        }
                        let sql = format!("INSERT IGNORE INTO vec3 [{}]", rows.join(","));
                        let _ = writer.write_statement(&sql);
                    }
                }
                println!(
                    "[mesh_inline] deferred: aabb={} pts={} 条写入 .surql",
                    mesh_aabb_map.len(), mesh_pts_map.len()
                );
            } else {
                // 直接模式：写入 SurrealDB
                crate::fast_model::utils::save_pts_to_surreal(&mesh_pts_map).await;
                crate::fast_model::utils::save_aabb_to_surreal(&mesh_aabb_map).await;
            }
        }

        println!(
            "[insert_handle] 汇总: batch_cnt={}, t_save_db={}ms, t_mesh={}ms (mesh={}), t_cache={}ms, t_parquet={}ms",
            batch_cnt,
            t_save_db.as_millis(),
            t_mesh.as_millis(),
            mesh_total,
            t_cache.as_millis(),
            t_parquet.as_millis(),
        );

        #[cfg(feature = "profile")]

        {

            tracing::info!(

                batch_cnt,

                save_db_ms = t_save_db.as_millis() as u64,

                cache_ms = t_cache.as_millis() as u64,

                parquet_ms = t_parquet.as_millis() as u64,

                "instance_sink finished"

            );

            #[cfg(feature = "duckdb-feature")]

            tracing::info!(duckdb_ms = t_duckdb.as_millis() as u64, "instance_sink duckdb finished");

        }

        ensure_no_db_write_failures(db_write_failures)?;
        let bool_tasks = bool_accumulator.build_tasks();
        Ok::<InsertHandleReport, anyhow::Error>(InsertHandleReport { batch_cnt, bool_tasks })

    });

    let categorized = gen_index_tree_geos_optimized(
        Arc::new(db_option.clone()),
        &config,
        sender.clone(),
        seed_roots,
    )

        .await

        .map_err(|e| anyhow::anyhow!("IndexTree 生成失败: {}", e))?;

    // 🔥 显式 drop sender，让 receiver 的循环能够正常结束
    // 否则 insert_handle.await 会永久阻塞
    drop(sender);

    let insert_report = insert_handle
        .await
        .map_err(|e| anyhow::anyhow!("instance sink 任务异常退出: {}", e))?
        .map_err(IndexTreeError::Other)?;
    let mut bool_tasks = insert_report.bool_tasks;

    println!(

        "[gen_model] IndexTree 模式 insts 入库完成，用时 {} ms",

        full_start.elapsed().as_millis()

    );

    perf.mark("mesh_generation");

    // 2️⃣ 可选执行 mesh 生成（已通过 mesh_handle 并行处理，此处等待完成）

    if db_option.inner.gen_mesh {

        let mesh_start = Instant::now();

        // 收集所有 refnos（后续 web bundle / aabb 等步骤仍需使用）
        let cate = categorized.get_by_category(NounCategory::Cate);
        let loops = categorized.get_by_category(NounCategory::LoopOwner);
        let prims = categorized.get_by_category(NounCategory::Prim);
        
        let mut all_refnos = Vec::with_capacity(cate.len() + loops.len() + prims.len());
        all_refnos.extend(cate);
        all_refnos.extend(loops);
        all_refnos.extend(prims);

        let mut ran_primary = false;

        // model_cache mesh worker 已移除（foyer-cache-cleanup）
        let _ = &model_cache_ctx;

        // mesh 已在 insert_handle 内联完成，无需额外等待
        ran_primary = gen_mesh;
        if gen_mesh {
            println!(
                "[gen_model] IndexTree 模式 mesh 内联生成完成，用时 {} ms",
                mesh_start.elapsed().as_millis()
            );
        }

        perf.mark("aabb_write");

        // 3️⃣ 写入 inst_relate_aabb 并导出 Parquet（供房间计算使用）

        if use_surrealdb && !defer_db_write {

            // 性能实验：允许跳过 AABB 写入，便于先定位“生成/mesh/boolean”的主耗时。
            let skip_aabb_write = std::env::var_os("AIOS_SKIP_INST_RELATE_AABB").is_some();
            if skip_aabb_write {
                println!(
                    "[gen_model] IndexTree 模式跳过 inst_relate_aabb 写入（AIOS_SKIP_INST_RELATE_AABB=1）"
                );
            } else {
                let aabb_start = Instant::now();

                println!("[gen_model] IndexTree 模式开始写入 inst_relate_aabb");

                // 只写本次生成触达的 refno，避免 pe_transform 全库扫描导致卡死/耗时失真。
                let mut aabb_refnos: Vec<RefnoEnum> = {
                    let guard = touched_refnos.lock().unwrap();
                    guard.iter().copied().collect()
                };

                // manual_db_nums=单库时，严格按 db_meta 映射过滤，避免混入其他 dbnum 的 refno。
                if let Some(known) = known_dbnum {
                    let _ = db_meta().ensure_loaded();
                    let mut filtered = Vec::with_capacity(aabb_refnos.len());
                    let mut missing = 0usize;
                    for r in aabb_refnos.drain(..) {
                        match db_meta().get_dbnum_by_refno(r) {
                            Some(dbnum) if dbnum == known => filtered.push(r),
                            Some(_) => {}
                            None => missing += 1,
                        }
                    }
                    if missing > 0 {
                        eprintln!(
                            "[gen_model] ⚠️ inst_relate_aabb refno 过滤时发现 {} 个 refno 缺少 ref0->dbnum 映射，已跳过（known_dbnum={}）",
                            missing, known
                        );
                    }
                    aabb_refnos = filtered;
                }

                if aabb_refnos.is_empty() {
                    eprintln!(
                        "[gen_model] IndexTree 模式写入 inst_relate_aabb 被跳过：本次生成未收集到可用 refno"
                    );
                } else {
                    println!(
                        "[gen_model] IndexTree 模式 inst_relate_aabb 写入范围: refnos={}",
                        aabb_refnos.len()
                    );

                    if let Err(e) = crate::fast_model::mesh_generate::update_inst_relate_aabbs_by_refnos(
                        &aabb_refnos,
                        db_option.is_replace_mesh(),
                    )
                    .await
                    {
                        eprintln!("[gen_model] IndexTree 模式写入 inst_relate_aabb 失败: {}", e);
                    } else {
                        println!(
                            "[gen_model] IndexTree 模式完成 inst_relate_aabb 写入，用时 {} ms",
                            aabb_start.elapsed().as_millis()
                        );
                    }
                }
            }

        }

        perf.mark("boolean_operation");

        // 3.5️⃣ 补建跨阶段缺失的 neg_relate（LOOP 阶段发现负实体但 PRIM 阶段才创建 geo_relate）
        if use_surrealdb && !defer_db_write {
            if let Err(e) = crate::fast_model::gen_model::pdms_inst::reconcile_missing_neg_relate(&all_refnos).await {
                eprintln!("[gen_model] reconcile_missing_neg_relate 失败: {}", e);
            }
        }

        // 4️⃣ 可选执行布尔运算

        if db_option.inner.apply_boolean_operation {

            let bool_start = Instant::now();

            println!("[gen_model] IndexTree 模式开始布尔运算（boolean worker）");
            println!(
                "[gen_model] boolean_pipeline_mode={:?}, defer_db_write={}, use_surrealdb={}, enable_db_backfill={}",
                db_option.boolean_pipeline_mode, defer_db_write, use_surrealdb, db_option.enable_db_backfill
            );
            println!(
                "[gen_model] 布尔任务统计: total={} (insert_batch_cnt={})",
                bool_tasks.len(),
                insert_report.batch_cnt
            );

            // model_cache boolean worker 已移除（foyer-cache-cleanup）
            match db_option.boolean_pipeline_mode {
                BooleanPipelineMode::DbLegacy => {
                    if use_surrealdb && !defer_db_write {
                        if let Err(e) = run_boolean_worker(Arc::new(db_option.inner.clone()), 100).await {
                            eprintln!("[gen_model] IndexTree 布尔运算失败（db_legacy）: {}", e);
                        }
                    } else {
                        println!(
                            "[gen_model] boolean_pipeline_mode=db_legacy，当前模式不满足执行条件（use_surrealdb={} defer_db_write={}）",
                            use_surrealdb, defer_db_write
                        );
                    }
                }
                BooleanPipelineMode::MemoryTasks => {
                    // 模式组合合法性守卫：MemoryTasks 至少需要一种写入通道
                    if !defer_db_write && !use_surrealdb {
                        eprintln!(
                            "[gen_model] boolean_pipeline_mode=memory_tasks 非法：defer_db_write=false 且 use_surrealdb=false，无写入通道，跳过布尔"
                        );
                    } else if bool_tasks.is_empty() {
                        println!("[gen_model] boolean_pipeline_mode=memory_tasks，但没有可执行布尔任务");
                    } else {
                        // T7: DB backfill — 补齐内存中缺失的 cata 任务
                        if db_option.enable_db_backfill {
                            match super::boolean_backfill::backfill_cata_tasks_from_db(
                                &mut bool_tasks,
                                use_surrealdb,
                            )
                            .await
                            {
                                Ok(count) if count > 0 => {
                                    println!(
                                        "[gen_model] DB backfill 补齐了 {} 个 cata 布尔任务，当前总数 {}",
                                        count,
                                        bool_tasks.len()
                                    );
                                }
                                Err(e) => {
                                    eprintln!("[gen_model] DB backfill 失败（非致命，继续执行）: {}", e);
                                }
                                _ => {}
                            }
                        }

                        match run_bool_worker_from_tasks(
                            std::mem::take(&mut bool_tasks),
                            Arc::new(db_option.inner.clone()),
                            sql_file_writer.clone(),
                        )
                        .await
                        {
                            Ok(report) => {
                                println!(
                                    "[gen_model] memory bool worker 完成: total={} cata={} inst={} success={} failed={} skipped={} defer={}",
                                    report.total,
                                    report.cata_cnt,
                                    report.inst_cnt,
                                    report.success,
                                    report.failed,
                                    report.skipped,
                                    report.deferred_mode
                                );
                            }
                            Err(e) => {
                                eprintln!("[gen_model] IndexTree 布尔运算失败（memory_tasks）: {}", e);
                            }
                        }
                    }
                }
            }

            println!(

                "[gen_model] IndexTree 模式布尔运算完成，用时 {} ms",

                bool_start.elapsed().as_millis()

            );

        }

        // defer_db_write 模式：布尔阶段后再 flush，确保布尔 SQL 也写入同一文件。
        if let Some(ref writer) = sql_file_writer {
            writer.flush()?;
            println!(
                "[gen_model] 🗂️ defer_db_write 完成: {} 条 SQL 语句已写入 {}",
                writer.statement_count(),
                writer.path().display()
            );
            println!(
                "[gen_model] 提示: 使用 --import-sql {} 导入到 SurrealDB",
                writer.path().display()
            );
        }

        perf.mark("web_bundle_export");

        // 5️⃣ 生成 Web Bundle (GLB + JSON 数据包)

        if db_option.mesh_formats.contains(&MeshFormat::Glb) {

            let web_bundle_start = Instant::now();

            println!("[gen_model] 开始生成 Web Bundle (GLB + JSON 数据包)...");

        let mesh_dir = Path::new(db_option.inner.meshes_path.as_deref().unwrap_or("assets/meshes"));

        // 输出到与 meshes 同级的 web_bundle 目录

        let output_dir = mesh_dir.parent().unwrap_or(mesh_dir).join("web_bundle");

        if let Err(e) = export_prepack_lod_for_refnos(

            &all_refnos,

            &mesh_dir,

            &output_dir,

            Arc::new(db_option.inner.clone()),

            true, // include_descendants

            None, // filter_nouns

            true, // verbose

            None, // name_config

            false, // export_all_lods: 改为 false，遵循 DbOption 中的默认设置

            LengthUnit::Millimeter,

            LengthUnit::Millimeter,

        )

        .await

        {

            eprintln!("[gen_model] 生成 Web Bundle 失败: {}", e);

            } else {

                println!(

                    "[gen_model] Web Bundle 生成完成，输出目录: {}, 用时 {} ms",

                    output_dir.display(),

                    web_bundle_start.elapsed().as_millis()

                );

            }

        }

    }

    perf.mark("sqlite_spatial_index");

    println!(

        "[gen_model] IndexTree 模式全部完成，总用时 {} ms",

        full_start.elapsed().as_millis()

    );

    println!(

        "[gen_model] gen_all_geos_data 总耗时: {} ms",

        time.elapsed().as_millis()

    );

    // 4️⃣ 生成 SQLite 空间索引（从 model cache 批量落库）

    let touched_dbnums_vec: Vec<u32> = touched_dbnums

        .lock()

        .map(|s| s.iter().copied().collect())

        .unwrap_or_default();

    if let Err(e) = update_sqlite_spatial_index_from_cache(db_option, &touched_dbnums_vec).await {

        eprintln!("[gen_model] SQLite 空间索引生成失败: {}", e);

    }

    perf.mark("instances_export");

    // ✅ 模型生成完毕后导出 instances.json（按 dbno）

    if db_option.export_instances {

        let (dbno_source, mut dbnos): (&str, Vec<u32>) =
            if let Some(nums) = db_option.inner.manual_db_nums.clone() {
                ("manual_db_nums", nums)
            } else if !touched_dbnums_vec.is_empty() {
                // 优先导出本次生成实际触达的 dbnum，避免扫描全 MDB 触发无关库的 tree 缺失报错。
                ("touched_dbnums", touched_dbnums_vec.clone())
            } else {
                (
                    "query_mdb_db_nums",
                    aios_core::query_mdb_db_nums(None, aios_core::DBType::DESI).await?,
                )
            };

        if let Some(exclude_nums) = &db_option.inner.exclude_db_nums {

            use std::collections::HashSet;

            let exclude: HashSet<u32> = exclude_nums.iter().copied().collect();

            dbnos.retain(|dbnum| !exclude.contains(dbnum));

        }

        dbnos.sort_unstable();
        dbnos.dedup();

        if dbnos.is_empty() {
            println!("[instances] 跳过导出：未解析到可用 dbnum（source={})", dbno_source);
        } else {
            println!(
                "[instances] 开始导出 instances.json: source={}, dbnums={:?}",
                dbno_source, dbnos
            );
        }

        let mesh_dir =

            Path::new(db_option.inner.meshes_path.as_deref().unwrap_or("assets/meshes"));

        if !dbnos.is_empty() {
            if let Err(e) = export_instances_json_for_dbnos(
                &dbnos,
                mesh_dir,
                &db_option.get_project_output_dir(),
                Arc::new(db_option.inner.clone()),
                true,
            )
            .await
            {
                eprintln!("[instances] IndexTree 导出失败: {}", e);
            }
        }

    }

    // model_cache close 已移除（foyer-cache-cleanup）

    perf.end_current();

    // 输出性能摘要到控制台

    perf.print_summary();

    // 保存性能报告为 JSON 和 CSV

    let project_name = if !db_option.inner.project_name.is_empty() {

        db_option.inner.project_name.clone()

    } else {

        "default".to_string()

    };

    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");

    let profile_dir = std::path::PathBuf::from("output")

        .join(&project_name)

        .join("profile");

    // 收集配置元数据

    let dbnum_tag = db_option.inner.manual_db_nums.as_ref()

        .and_then(|nums| nums.first().copied())

        .map(|n| n.to_string())

        .unwrap_or_else(|| "all".to_string());

    let enabled_nouns = db_option.index_tree_enabled_target_types.clone();

    let metadata = serde_json::json!({

        "mode": "index_tree",

        "project_name": project_name,

        "dbnum": dbnum_tag,

        "enabled_nouns": enabled_nouns,

        "use_surrealdb": db_option.use_surrealdb,

        "model_cache_write": true,

        "apply_boolean": db_option.inner.apply_boolean_operation,

        "gen_mesh": db_option.inner.gen_mesh,

        "concurrency": db_option.get_index_tree_concurrency(),

        "batch_size": db_option.get_index_tree_batch_size(),

    });

    let json_path = profile_dir.join(format!("perf_gen_model_index_tree_dbnum_{}_{}.json", dbnum_tag, timestamp));

    let csv_path = profile_dir.join(format!("perf_gen_model_index_tree_dbnum_{}_{}.csv", dbnum_tag, timestamp));

    if let Err(e) = perf.save_json(&json_path, metadata.clone()) {

        eprintln!("[perf] 保存 JSON 报告失败: {}", e);

    }

    if let Err(e) = perf.save_csv(&csv_path, metadata) {

        eprintln!("[perf] 保存 CSV 报告失败: {}", e);

    }

    let deferred_sql_path = sql_file_writer
        .as_ref()
        .map(|writer| writer.path().to_path_buf());
    Ok(GenModelResult {
        success: true,
        deferred_sql_path,
    })

}

// ============================================================================

// SQLite 空间索引：从 model cache 生成/增量更新 output/spatial_index.sqlite

//

// 目标：模型生成（写 cache）后，将 AABB 批量落库到 SQLite RTree，供房间计算等流程做粗筛。

// ============================================================================

#[cfg(feature = "sqlite-index")]

async fn update_sqlite_spatial_index_from_cache(db_option: &DbOptionExt, dbnums: &[u32]) -> Result<()> {

    use crate::spatial_index::SqliteSpatialIndex;

    use crate::sqlite_index::{ImportConfig, SqliteAabbIndex};

    use std::fs;

    use std::path::PathBuf;

    if dbnums.is_empty() {

        return Ok(());

    }

    if !db_option.inner.enable_sqlite_rtree {

        // 常见误区：已切换到 cache 生成，但忘了开 enable_sqlite_rtree，导致 spatial_index.sqlite 不会更新，
        // 房间计算（SQLite RTree 粗筛）会退化/失效。

        let idx_path = SqliteSpatialIndex::default_path();
        if !idx_path.exists() {

            eprintln!(

                "[gen_model] 警告：enable_sqlite_rtree=false，且未发现 {:?}；模型 AABB 不会落库到 SQLite。\

                 若需房间计算粗筛/诊断，请在 DbOption.toml 开启 enable_sqlite_rtree=true 或使用 CLI 导入 instances.json。",

                idx_path

            );

        }

        return Ok(());

    }

    // 打开/初始化索引（幂等）

    let idx_path = SqliteSpatialIndex::default_path();

    if let Some(parent) = idx_path.parent() {

        fs::create_dir_all(parent).map_err(|e| anyhow::anyhow!(e))?;

    }

    let idx = SqliteAabbIndex::open(&idx_path).map_err(|e| anyhow::anyhow!(e))?;

    idx.init_schema().map_err(|e| anyhow::anyhow!(e))?;

    // 为避免 aabb.json/trans.json（固定文件名）互相覆盖，每个 dbnum 独立输出目录。

    let base_out = db_option.get_project_output_dir().join("instances_cache_for_index");

    fs::create_dir_all(&base_out).map_err(|e| anyhow::anyhow!(e))?;

    // mesh_lod_tag 仅用于导出侧选择 mesh（用于补齐/计算 AABB）

    let cache_dir = db_option.get_model_cache_dir();

    let mesh_dir = db_option.inner.get_meshes_path();

    let mesh_lod_tag = format!("{:?}", db_option.inner.mesh_precision.default_lod);

    // 去重并保证顺序稳定（便于日志与排查）

    let mut uniq: BTreeSet<u32> = BTreeSet::new();

    uniq.extend(dbnums.iter().copied());

    for dbnum in uniq {

        let out_dir = base_out.join(format!("{}", dbnum));

        fs::create_dir_all(&out_dir).map_err(|e| anyhow::anyhow!(e))?;

        // 1) cache -> instances_{dbnum}.json + aabb.json + trans.json

        let _ = crate::fast_model::export_model::export_prepack_lod::export_dbnum_instances_json_from_cache(

            dbnum,

            &out_dir,

            &cache_dir,

            Some(&mesh_dir),

            Some(mesh_lod_tag.as_str()),

            false,

            None,

            false,

        )

        .await?;

        // 2) instances_{dbnum}.json -> spatial_index.sqlite (RTree)

        let instances_path = out_dir.join(format!("instances_{}.json", dbnum));

        if instances_path.exists() {

            let _ = idx.import_from_instances_json(&instances_path, &ImportConfig::default())?;

        }

    }

    Ok(())

}

#[cfg(not(feature = "sqlite-index"))]

async fn update_sqlite_spatial_index_from_cache(_db_option: &DbOptionExt, _dbnums: &[u32]) -> Result<()> {

    Ok(())

}

/// 初始化空间索引（如果启用）

#[cfg(feature = "duckdb-feature")]

fn initialize_spatial_index() {

    // if SqliteSpatialIndex::is_enabled() {

    //     match SqliteSpatialIndex::with_default_path() {

    //         Ok(_index) => println!("SQLite spatial index initialized"),

    //         Err(e) => eprintln!("Failed to initialize SQLite spatial index: {}", e),

    //     }

    // }

}

#[cfg(not(feature = "duckdb-feature"))]

fn initialize_spatial_index() {

    // No-op when feature is disabled

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_route_incremental_when_visible_roots_empty() {
        let manual_refnos: Vec<RefnoEnum> = Vec::new();
        let debug_roots: Vec<RefnoEnum> = Vec::new();
        let mut incr_log = IncrGeoUpdateLog::default();
        incr_log.prim_refnos.insert("17496_171666".into());

        let scope = decide_generation_scope(
            &manual_refnos,
            &debug_roots,
            true,
            &[],
            Some(&incr_log),
        );

        assert!(matches!(scope, GenerationScope::Incremental { .. }));
    }

    #[tokio::test]
    async fn test_db_write_failures_are_not_silenced() {
        let handles = vec![tokio::spawn(async { true }), tokio::spawn(async { false })];
        let failures = collect_db_write_failures(handles).await;
        let result = ensure_no_db_write_failures(failures);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("SurrealDB 批量写入存在失败任务")
        );
    }
}

