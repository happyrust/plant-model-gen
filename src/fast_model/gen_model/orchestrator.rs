//! 模型生成编排器

//!

//! 负责协调整个模型生成流程：

//! - Full Noun 模式 vs 非 Full Noun 模式的路由

//! - 几何体生成、Mesh 生成、布尔运算的编排

//! - 增量更新、手动 refno、调试模式的处理

//! - 空间索引和截图捕获的触发



use std::collections::{BTreeSet, HashMap};

use std::path::Path;

use std::sync::Arc;

use std::time::Instant;



use crate::fast_model::export_model::export_prepack_lod::export_prepack_lod_for_refnos;

use crate::fast_model::export_model::export_prepack_lod::export_instances_json_for_dbnos;

use crate::fast_model::export_model::export_prepack_lod::export_instances_json_for_refnos_grouped_by_dbno;

use crate::fast_model::query_provider;

use crate::fast_model::query_compat::{query_deep_neg_inst_refnos, query_deep_visible_inst_refnos};

use crate::fast_model::unit_converter::LengthUnit;



use aios_core::RefnoEnum;



use crate::data_interface::increment_record::IncrGeoUpdateLog;

use crate::data_interface::sesno_increment::get_changes_at_sesno;

use crate::fast_model::capture::capture_refnos_if_enabled;

use crate::data_interface::db_meta_manager::db_meta;

use crate::fast_model::mesh_generate::{

    run_boolean_worker, run_mesh_worker,

};

use crate::fast_model::pdms_inst::save_instance_data_optimize;

use crate::options::{DbOptionExt, MeshFormat};

use crate::fast_model::export_model::ParquetStreamWriter;

#[cfg(feature = "duckdb-feature")]

use crate::fast_model::export_model::{DuckDBStreamWriter, DuckDBWriteMode};



use super::config::FullNounConfig;

use super::errors::{FullNounError, Result};

use super::cache_miss_report;

use super::full_noun_mode::gen_full_noun_geos_optimized;

use super::models::NounCategory;

use std::str::FromStr;

use crate::fast_model::gen_model::tree_index_manager::TreeIndexManager;

use aios_core::tool::db_tool::db1_hash;

use dashmap::{DashMap, DashSet};

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

    async fn get_dbnum_cached(
        refno: RefnoEnum,
        cache: &mut HashMap<RefnoEnum, u32>,
    ) -> anyhow::Result<u32> {
        if let Some(v) = cache.get(&refno) {
            return Ok(*v);
        }
        let dbnum = TreeIndexManager::resolve_dbnum_for_refno(refno)
            .await
            .map_err(|e| anyhow::anyhow!("缺少 ref0->dbnum 映射: refno={refno}, err={e}"))?;
        cache.insert(refno, dbnum);
        Ok(dbnum)
    }

    // inst_info
    for (refno, info) in shape_insts.inst_info_map.iter() {
        let refno = *refno;
        let dbnum = get_dbnum_cached(refno, &mut cache).await?;
        out.entry(dbnum)
            .or_insert_with(ShapeInstancesData::default)
            .inst_info_map
            .insert(refno, info.clone());
    }

    // inst_tubi
    for (refno, tubi) in shape_insts.inst_tubi_map.iter() {
        let refno = *refno;
        let dbnum = get_dbnum_cached(refno, &mut cache).await?;
        out.entry(dbnum)
            .or_insert_with(ShapeInstancesData::default)
            .inst_tubi_map
            .insert(refno, tubi.clone());
    }

    // inst_geos：每条 geos_data 都绑定一个 refno（元素），直接按 geos_data.refno 分桶。
    for (inst_key, geos_data) in shape_insts.inst_geos_map.iter() {
        let inst_key = inst_key.clone();
        let geos_data = geos_data.clone();
        let dbnum = get_dbnum_cached(geos_data.refno, &mut cache).await?;
        out.entry(dbnum)
            .or_insert_with(ShapeInstancesData::default)
            .inst_geos_map
            .insert(inst_key, geos_data);
    }

    // neg_relate / ngmr_neg_relate：按 key(refno) 分桶
    for (refno, v) in &shape_insts.neg_relate_map {
        let dbnum = get_dbnum_cached(*refno, &mut cache).await?;
        out.entry(dbnum)
            .or_insert_with(ShapeInstancesData::default)
            .neg_relate_map
            .insert(*refno, v.clone());
    }
    for (refno, v) in &shape_insts.ngmr_neg_relate_map {
        let dbnum = get_dbnum_cached(*refno, &mut cache).await?;
        out.entry(dbnum)
            .or_insert_with(ShapeInstancesData::default)
            .ngmr_neg_relate_map
            .insert(*refno, v.clone());
    }

    Ok(out)
}



/// 主入口函数：生成所有几何体数据

///

/// 这是主要的公共 API，根据配置路由到不同的生成策略：

/// - Full Noun 模式：使用新优化的 gen_full_noun_geos_optimized 管线

/// - 非 Full Noun 模式：

///   - 增量更新模式（incr_updates 非空）

///   - 手动 refno 模式（manual_refnos 非空）

///   - 调试模式（debug_model_refnos 非空）

///   - 全量生成模式（按 dbnum 循环）

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

) -> Result<bool> {

    let time = Instant::now();

    let mut perf = crate::perf_timer::PerfTimer::new("gen_all_geos_data");

    perf.mark("init");

    // cache-first 缺失报告：生成过程中按需补充记录，结束时输出到 output/<project>/cache_miss_report.json
    let cache_run_mode = crate::fast_model::foyer_cache::geom_input_cache::resolve_cache_run_mode();
    cache_miss_report::init_global_cache_miss_report(db_option, cache_run_mode.as_str());

    let mut final_incr_updates = incr_updates;



    // 如果指定了 target_sesno，获取该 sesno 的增量数据

    if let Some(sesno) = target_sesno {

        if !db_option.use_surrealdb {

            return Err(FullNounError::Other(anyhow::anyhow!(

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

                        return Ok(false);

                    }

                }

                Err(e) => {

                    eprintln!("获取 sesno {} 的变更失败: {}", sesno, e);

                    return Err(FullNounError::Other(e));

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



    // TreeIndex 文件（output/scene_tree/{dbnum}.tree）在不少流程中是必需的（Full Noun/导出/层级查询）。

    // cache-only：不允许自动生成/回退 SurrealDB；缺失即报错，避免“看似成功但数据为空”。

    if db_option.use_surrealdb {

        crate::fast_model::gen_model::tree_index_manager::enable_auto_generate_tree();

    } else {

        crate::fast_model::gen_model::tree_index_manager::disable_auto_generate_tree();

    }



    // 调试：打印 Full Noun 模式配置

    println!(

        "[gen_model] Full Noun 模式配置: full_noun_mode={}, concurrency={}, batch_size={}",

        db_option.full_noun_mode,

        db_option.get_full_noun_concurrency(),

        db_option.get_full_noun_batch_size()

    );



    // ✅ SurrealDB 写入侧初始化：仅在 use_surrealdb=true 时需要。

    if db_option.use_surrealdb {

        if let Err(e) = aios_core::rs_surreal::inst::init_model_tables().await {

            eprintln!("[gen_model] ❌ 初始化 inst_relate 表结构失败: {}", e);

            // 严重错误，建议直接中断，否则后续写入必挂

            return Err(FullNounError::Other(e));

        }

    }



    // =========================

    // LOOP/PRIM 输入缓存初始化（按环境变量启用）

    // =========================

    {
        use crate::fast_model::foyer_cache::geom_input_cache;

        let cache_run_mode = geom_input_cache::resolve_cache_run_mode();
        println!(
            "[gen_model] geom_input_cache 运行模式: {}",
            cache_run_mode.as_str()
        );

        if !matches!(cache_run_mode, geom_input_cache::CacheRunMode::Direct) {
            if let Err(e) = geom_input_cache::init_global_geom_input_cache(db_option).await {
                eprintln!("[gen_model] ⚠️  初始化 geom_input_cache 失败: {}", e);
            } else {
                println!(
                    "[gen_model] geom_input_cache 已初始化 (mode={})",
                    cache_run_mode.as_str()
                );
            }
        }
    }



    // =========================

    // Full Noun 模式：新管线

    // =========================

    // 恢复非 Full Noun 入口：debug/manual/incr 走定向生成，避免被 Full Noun 吞掉

    let is_incr_update = final_incr_updates.is_some();

    let has_manual_refnos = !manual_refnos.is_empty();

    let has_debug = db_option

        .inner

        .debug_model_refnos

        .as_ref()

        .map(|v| !v.is_empty())

        .unwrap_or(false);

    perf.mark("route_decision");

    let result = if has_debug || has_manual_refnos || is_incr_update {

        perf.mark("targeted_generation");

        process_targeted_generation(

            manual_refnos,

            db_option,

            final_incr_updates,

            target_sesno,

            time,

        )

        .await

    } else if db_option.full_noun_mode {

        perf.mark("full_noun_pipeline");

        process_full_noun_mode(db_option, final_incr_updates, time).await

    } else {

        perf.mark("full_database_generation");

        process_full_database_generation(db_option, target_sesno, time).await

    };



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

        let dbnum = match TreeIndexManager::resolve_dbnum_for_refno(r).await {

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



/// 处理 Full Noun 模式的生成流程

async fn process_full_noun_mode(

    db_option: &DbOptionExt,

    incr_updates: Option<IncrGeoUpdateLog>,

    time: Instant,

) -> Result<bool> {

    let mut perf = crate::perf_timer::PerfTimer::new("full_noun_mode");

    perf.mark("init");



    println!("[gen_model] 进入 Full Noun 模式（新 gen_model 管线）");



    if db_option.manual_db_nums.is_some() || db_option.exclude_db_nums.is_some() {

        println!(

            "[gen_model] 提示: Full Noun 新管线已支持 manual_db_nums / exclude_db_nums 过滤，当前仍按配置执行"

        );

    }



    if incr_updates.is_some() {

        println!("[gen_model] 警告: Full Noun 模式下增量更新将被忽略，将执行全库重建");

    }



    let full_start = Instant::now();



    perf.mark("categorize_and_inst_relate");



    // 1️⃣ 生成/更新 inst_relate，并获取分类后的根 refno

    let config = FullNounConfig::from_db_option_ext(db_option)

        .map_err(|e| anyhow::anyhow!("配置错误: {}", e))?;



    let (sender, receiver) = flume::bounded::<aios_core::geometry::ShapeInstancesData>(100);

    let replace_exist = db_option.inner.is_replace_mesh();

    let use_surrealdb = db_option.use_surrealdb;



    // 初始化 Parquet 写入器

    let parquet_writer: Option<std::sync::Arc<ParquetStreamWriter>> = None;

    /*

    let parquet_writer = {

        let output_dir = db_option.inner.meshes_path.as_deref().unwrap_or("assets/meshes");

        let parquet_dir = std::path::Path::new(output_dir).parent().unwrap_or(std::path::Path::new("output"));

        match ParquetStreamWriter::new(&parquet_dir) {

            Ok(writer) => Some(std::sync::Arc::new(writer)),

            Err(e) => {

                eprintln!("[Parquet] 初始化写入器失败: {}, 跳过 Parquet 导出", e);

                None

            }

        }

    };

    */

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

    let parquet_writer_clone = parquet_writer.clone();

    #[cfg(feature = "duckdb-feature")]

    let duckdb_writer_clone = duckdb_writer.clone();



    // foyer cache-only 上下文：统一管理 cache_dir 与 InstanceCacheManager（并尽力预初始化 transform_cache）。

    let foyer_cache_ctx = crate::fast_model::foyer_cache::FoyerCacheContext::try_from_db_option(db_option).await?;

    let cache_manager_for_insert = foyer_cache_ctx.as_ref().map(|c| c.cache_arc());

    let touched_dbnums: Arc<std::sync::Mutex<BTreeSet<u32>>> =

        Arc::new(std::sync::Mutex::new(BTreeSet::new()));

    let touched_dbnums_for_insert = touched_dbnums.clone();

    // Full Noun 下用于 inst_relate_aabb 写入的 refno 集合：只收集“本次生成触达”的实例，
    // 避免通过 pe_transform 全库扫描导致卡死/耗时失真。
    let touched_refnos: Arc<DashSet<RefnoEnum>> = Arc::new(DashSet::new());
    let touched_refnos_for_insert = touched_refnos.clone();



    // 当 manual_db_nums 只有一个值时，直接使用该 dbnum，无需从 refno 反推

    let known_dbnum: Option<u32> = db_option.inner.manual_db_nums.as_ref()

        .filter(|nums| nums.len() == 1)

        .and_then(|nums| nums.first().copied());



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
        let mut db_write_handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();
        // 控制 SurrealDB 后台写入的最大并发数
        let db_write_semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(8));

        loop {

            let Ok(shape_insts) = receiver.recv_async().await else {

                break;

            };

            let shape_insts_arc = std::sync::Arc::new(shape_insts);

            #[cfg(feature = "profile")]
            let _enter = sink_span.enter();

            batch_cnt += 1;

            // 记录本批次触达的实例 refno（用于后续 inst_relate_aabb 写入范围收敛）
            for r in shape_insts_arc.inst_info_map.keys() {
                touched_refnos_for_insert.insert(*r);
            }
            for r in shape_insts_arc.inst_tubi_map.keys() {
                touched_refnos_for_insert.insert(*r);
            }

            if let Some(ref cache_manager) = cache_manager_for_insert {
                let t0 = Instant::now();

                let by_dbnum = match split_shape_instances_by_dbnum(&shape_insts_arc).await {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("[cache] batch 拆分 dbnum 失败，跳过写 cache: {}", e);
                        continue;
                    }
                };

                if by_dbnum.len() > 1 {
                    eprintln!(
                        "[cache] ⚠️ batch 同时包含多个 dbnum，将按 dbnum 拆分写入: dbnums={:?}",
                        by_dbnum.keys().collect::<Vec<_>>()
                    );
                }

                if let Some(known) = known_dbnum {
                    if let Some(sub) = by_dbnum.get(&known) {
                        cache_manager.insert_from_shape(known, sub);
                        let _ = touched_dbnums_for_insert.lock().map(|mut s| s.insert(known));
                    }
                    // 其他 dbnum 的数据直接报警（manual_db_nums=单库时出现多库，属于上游 bug）
                    for dbnum in by_dbnum.keys().copied() {
                        if dbnum != known {
                            eprintln!(
                                "[cache] ⚠️ manual_db_nums 指定 dbnum={}，但 batch 内出现 dbnum={}，已忽略",
                                known, dbnum
                            );
                        }
                    }
                } else {
                    for (dbnum, sub) in by_dbnum {
                        cache_manager.insert_from_shape(dbnum, &sub);
                        let _ = touched_dbnums_for_insert.lock().map(|mut s| s.insert(dbnum));
                    }
                }

                t_cache += t0.elapsed();
            }

            // 同时写入 Parquet（如果启用）

            if let Some(ref writer) = parquet_writer_clone {

                let t0 = Instant::now();

                if let Err(e) = writer.write_batch(&shape_insts_arc) {

                    eprintln!("[Parquet] 写入批次失败: {}", e);

                }

                t_parquet += t0.elapsed();

            }

            #[cfg(feature = "duckdb-feature")]

            if let Some(ref writer) = duckdb_writer_clone {

                let t0 = Instant::now();

                if let Err(e) = writer.write_batch(&shape_insts_arc) {

                    eprintln!("[DuckDB] 写入批次失败: {}", e);

                }

                t_duckdb += t0.elapsed();

            }

            // SurrealDB 写入放到后台，不阻塞 cache 写入和后续 batch 接收
            // 采用 Semaphore 限流，防止瞬发海量并发协程打垮数据库导致事务冲突风暴
            // 此处在 receiver 循环外侧初始化，限定最大并行写库数为 8
            if use_surrealdb {
                let t0 = Instant::now();
                let sem = db_write_semaphore.clone();
                let shape_insts_clone = shape_insts_arc.clone();
                db_write_handles.push(tokio::spawn(async move {
                    let _permit = match sem.acquire_owned().await {
                        Ok(p) => p,
                        Err(e) => {
                            eprintln!("获取写库并发锁失败: {}", e);
                            return;
                        }
                    };
                    if let Err(e) = save_instance_data_optimize(&shape_insts_clone, replace_exist).await {
                        eprintln!("保存实例数据失败: {}", e);
                    }
                }));
                t_save_db += t0.elapsed();
            }

        }

        // 等待所有 SurrealDB 后台写入完成
        if !db_write_handles.is_empty() {
            let t_wait = Instant::now();
            let total = db_write_handles.len();
            for h in db_write_handles {
                let _ = h.await;
            }
            let wait_ms = t_wait.elapsed().as_millis();
            if wait_ms > 100 {
                println!(
                    "[gen_model] SurrealDB 后台写入等待完成: {} 个任务, 额外等待 {} ms",
                    total, wait_ms
                );
            }
        }



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

    });



    let categorized = gen_full_noun_geos_optimized(Arc::new(db_option.clone()), &config, sender.clone())

        .await

        .map_err(|e| anyhow::anyhow!("Full Noun 生成失败: {}", e))?;



    // 🔥 显式 drop sender，让 receiver 的循环能够正常结束

    // 否则 insert_handle.await 会永久阻塞

    drop(sender);



    let _ = insert_handle.await;



    // 完成 Parquet 写入并合并文件

    if let Some(ref writer) = parquet_writer {

        if let Err(e) = writer.finalize() {

            eprintln!("[Parquet] 合并文件失败: {}", e);

        }

    }

    #[cfg(feature = "duckdb-feature")]

    if let Some(ref writer) = duckdb_writer {

        if let Err(e) = writer.finalize() {

            eprintln!("[DuckDB] finalize 失败: {}", e);

        }

    }



    println!(

        "[gen_model] Full Noun 模式 insts 入库完成，用时 {} ms",

        full_start.elapsed().as_millis()

    );



    perf.mark("mesh_generation");



    // 2️⃣ 可选执行 mesh 生成

    if db_option.inner.gen_mesh {

        let mesh_start = Instant::now();

        println!("[gen_model] Full Noun 模式开始生成三角网格（深度收集几何节点）");



        // 收集所有 refnos

        let cate = categorized.get_by_category(NounCategory::Cate);

        let loops = categorized.get_by_category(NounCategory::LoopOwner);

        let prims = categorized.get_by_category(NounCategory::Prim);

        let mut all_refnos = Vec::new();

        all_refnos.extend(cate);

        all_refnos.extend(loops);

        all_refnos.extend(prims);



        let mut ran_primary = false;



        if let Some(ref ctx) = foyer_cache_ctx {

            let mesh_dir = db_option.inner.get_meshes_path();

            match crate::fast_model::foyer_cache::mesh::run_mesh_worker(

                ctx,

                &mesh_dir,

                &db_option.inner.mesh_precision,

                &db_option.mesh_formats,

            )

            .await

            {

                Ok(n) if n > 0 => {
                    println!("[gen_model] mesh worker 缓存路径完成: {} 个", n);
                    ran_primary = true;
                }

                Ok(_) => {
                    println!("[gen_model] mesh worker 缓存路径: 0 个 mesh，回退 SurrealDB");
                }

                Err(e) => {
                    eprintln!("[gen_model] mesh worker 缓存路径失败: {}", e);
                }

            }

        }



        if use_surrealdb && !ran_primary {

            if let Err(e) = run_mesh_worker(Arc::new(db_option.inner.clone()), 100).await {

                eprintln!("[gen_model] mesh worker 失败: {}", e);

            } else {

                ran_primary = true;

            }

        }



        if ran_primary {

            println!(

                "[gen_model] Full Noun 模式 mesh 生成完成，用时 {} ms",

                mesh_start.elapsed().as_millis()

            );

        }



        perf.mark("aabb_write");



        // 3️⃣ 写入 inst_relate_aabb 并导出 Parquet（供房间计算使用）

        if use_surrealdb {

            // 性能实验：允许跳过 AABB 写入，便于先定位“生成/mesh/boolean”的主耗时。
            let skip_aabb_write = std::env::var_os("AIOS_SKIP_INST_RELATE_AABB").is_some();
            if skip_aabb_write {
                println!(
                    "[gen_model] Full Noun 模式跳过 inst_relate_aabb 写入（AIOS_SKIP_INST_RELATE_AABB=1）"
                );
            } else {
                let aabb_start = Instant::now();

                println!("[gen_model] Full Noun 模式开始写入 inst_relate_aabb");

                // 只写本次生成触达的 refno，避免 pe_transform 全库扫描导致卡死/耗时失真。
                let mut aabb_refnos: Vec<RefnoEnum> =
                    touched_refnos.iter().map(|r| *r.key()).collect();

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
                        "[gen_model] Full Noun 模式写入 inst_relate_aabb 被跳过：本次生成未收集到可用 refno"
                    );
                } else {
                    println!(
                        "[gen_model] Full Noun 模式 inst_relate_aabb 写入范围: refnos={}",
                        aabb_refnos.len()
                    );

                    if let Err(e) = crate::fast_model::mesh_generate::update_inst_relate_aabbs_by_refnos(
                        &aabb_refnos,
                        db_option.is_replace_mesh(),
                    )
                    .await
                    {
                        eprintln!("[gen_model] Full Noun 模式写入 inst_relate_aabb 失败: {}", e);
                    } else {
                        println!(
                            "[gen_model] Full Noun 模式完成 inst_relate_aabb 写入，用时 {} ms",
                            aabb_start.elapsed().as_millis()
                        );
                    }
                }
            }

        }



        perf.mark("boolean_operation");



        // 4️⃣ 可选执行布尔运算

        if db_option.inner.apply_boolean_operation {

            let bool_start = Instant::now();

            println!("[gen_model] Full Noun 模式开始布尔运算（boolean worker）");

            if let Some(ref ctx) = foyer_cache_ctx {

                if let Err(e) = crate::fast_model::foyer_cache::boolean::run_boolean_worker(ctx).await

                {

                    eprintln!("[gen_model] Full Noun 缓存布尔运算失败: {}", e);

                }

            }

            if use_surrealdb {

                if let Err(e) = run_boolean_worker(Arc::new(db_option.inner.clone()), 100).await {

                    eprintln!("[gen_model] Full Noun 布尔运算失败: {}", e);

                }

            }



            println!(

                "[gen_model] Full Noun 模式布尔运算完成，用时 {} ms",

                bool_start.elapsed().as_millis()

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

        "[gen_model] Full Noun 模式全部完成，总用时 {} ms",

        full_start.elapsed().as_millis()

    );

    println!(

        "[gen_model] gen_all_geos_data 总耗时: {} ms",

        time.elapsed().as_millis()

    );



    // 4️⃣ 生成 SQLite 空间索引（从 foyer cache 批量落库）

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

        let mut dbnos: Vec<u32> = if let Some(nums) = db_option.inner.manual_db_nums.clone() {

            nums

        } else {

            aios_core::query_mdb_db_nums(None, aios_core::DBType::DESI).await?

        };

        if let Some(exclude_nums) = &db_option.inner.exclude_db_nums {

            use std::collections::HashSet;

            let exclude: HashSet<u32> = exclude_nums.iter().copied().collect();

            dbnos.retain(|dbnum| !exclude.contains(dbnum));

        }

        let mesh_dir =

            Path::new(db_option.inner.meshes_path.as_deref().unwrap_or("assets/meshes"));

        if let Err(e) = export_instances_json_for_dbnos(

            &dbnos,

            mesh_dir,

            &db_option.get_project_output_dir(),

            Arc::new(db_option.inner.clone()),

            true,

        )

        .await

        {

            eprintln!("[instances] Full Noun 导出失败: {}", e);

        }

    }



    if let Some(ref ctx) = foyer_cache_ctx {

        if let Err(e) = ctx.cache().close().await {

            eprintln!("[cache] 关闭缓存失败: {}", e);

        }

    }



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



    let enabled_nouns = db_option.full_noun_enabled_categories.clone();



    let metadata = serde_json::json!({

        "mode": "full_noun",

        "project_name": project_name,

        "dbnum": dbnum_tag,

        "enabled_nouns": enabled_nouns,

        "use_surrealdb": db_option.use_surrealdb,

        "use_cache": db_option.use_cache,

        "apply_boolean": db_option.inner.apply_boolean_operation,

        "gen_mesh": db_option.inner.gen_mesh,

        "concurrency": db_option.get_full_noun_concurrency(),

        "batch_size": db_option.get_full_noun_batch_size(),

    });



    let json_path = profile_dir.join(format!("perf_gen_model_full_noun_dbnum_{}_{}.json", dbnum_tag, timestamp));

    let csv_path = profile_dir.join(format!("perf_gen_model_full_noun_dbnum_{}_{}.csv", dbnum_tag, timestamp));



    if let Err(e) = perf.save_json(&json_path, metadata.clone()) {

        eprintln!("[perf] 保存 JSON 报告失败: {}", e);

    }

    if let Err(e) = perf.save_csv(&csv_path, metadata) {

        eprintln!("[perf] 保存 CSV 报告失败: {}", e);

    }

    Ok(true)

}



/// 处理增量/手动/调试模式的目标生成

#[cfg_attr(feature = "profile", tracing::instrument(skip_all, name = "targeted_generation"))]

async fn process_targeted_generation(

    manual_refnos: Vec<RefnoEnum>,

    db_option: &DbOptionExt,

    incr_updates: Option<IncrGeoUpdateLog>,

    target_sesno: Option<u32>,

    time: Instant,

) -> Result<bool> {

    let is_incr_update = incr_updates.is_some();

    let has_manual_refnos = !manual_refnos.is_empty();



    let mode_label = if is_incr_update {

        "增量"

    } else if has_manual_refnos {

        "手动"

    } else {

        "调试"

    };



    // foyer cache-only 上下文：统一管理 cache_dir 与 InstanceCacheManager（并尽力预初始化 transform_cache）。

    let foyer_cache_ctx = crate::fast_model::foyer_cache::FoyerCacheContext::try_from_db_option(db_option).await?;



    let target_count = if is_incr_update {

        incr_updates.as_ref().map(|log| log.count()).unwrap_or(0)

    } else if has_manual_refnos {

        manual_refnos.len()

    } else {

        db_option

            .inner

            .debug_model_refnos

            .as_ref()

            .map(|v| v.len())

            .unwrap_or(0)

    };



    println!(

        "[gen_model] 进入{}生成路径，目标节点数: {}",

        mode_label, target_count

    );

    println!(

        "[gen_model] 缓存开关: use_cache={}, foyer_primary={}, dual_run={}",

        db_option.use_cache,

        db_option.foyer_primary,

        db_option.dual_run_enabled

    );



    let (sender, receiver) = flume::bounded(100);

    let receiver: flume::Receiver<aios_core::geometry::ShapeInstancesData> = receiver.clone();



    let replace_exist = db_option.inner.is_replace_mesh();

    let use_surrealdb = db_option.use_surrealdb;

    let cache_manager_for_insert = foyer_cache_ctx.as_ref().map(|c| c.cache_arc());

    let touched_dbnums: Arc<std::sync::Mutex<BTreeSet<u32>>> =

        Arc::new(std::sync::Mutex::new(BTreeSet::new()));

    let touched_dbnums_for_insert = touched_dbnums.clone();



    // 当 manual_db_nums 只有一个值时，直接使用该 dbnum，无需从 refno 反推

    let known_dbnum: Option<u32> = db_option.inner.manual_db_nums.as_ref()

        .filter(|nums| nums.len() == 1)

        .and_then(|nums| nums.first().copied());



    let mut db_write_handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    let db_write_semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(8));

    let insert_task = tokio::task::spawn(async move {

        while let Ok(shape_insts) = receiver.recv_async().await {
            let shape_insts_arc = std::sync::Arc::new(shape_insts);
            // SurrealDB 写入放到后台，不阻塞 cache 写入和后续 batch 接收
            // 采用 Semaphore 限流，防止瞬发海量并发协程打垮数据库导致事务冲突风暴
            if use_surrealdb {
                let sem = db_write_semaphore.clone();
                let shape_insts_clone = shape_insts_arc.clone();
                db_write_handles.push(tokio::spawn(async move {
                    let _permit = match sem.acquire_owned().await {
                        Ok(p) => p,
                        Err(e) => {
                            eprintln!("获取写库并发锁失败: {}", e);
                            return;
                        }
                    };
                    if let Err(e) = save_instance_data_optimize(&shape_insts_clone, replace_exist).await {
                        eprintln!("保存实例数据失败: {}", e);
                    }
                }));
            }

            if let Some(ref cache_manager) = cache_manager_for_insert {

                let by_dbnum = match split_shape_instances_by_dbnum(&shape_insts_arc).await {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("[cache] batch 拆分 dbnum 失败，跳过写 cache: {}", e);
                        continue;
                    }
                };

                if by_dbnum.len() > 1 {
                    eprintln!(
                        "[cache] ⚠️ batch 同时包含多个 dbnum，将按 dbnum 拆分写入: dbnums={:?}",
                        by_dbnum.keys().collect::<Vec<_>>()
                    );
                }

                if let Some(known) = known_dbnum {
                    if let Some(sub) = by_dbnum.get(&known) {
                        cache_manager.insert_from_shape(known, sub);
                        let _ = touched_dbnums_for_insert.lock().map(|mut s| s.insert(known));
                    }
                    for dbnum in by_dbnum.keys().copied() {
                        if dbnum != known {
                            eprintln!(
                                "[cache] ⚠️ manual_db_nums 指定 dbnum={}，但 batch 内出现 dbnum={}，已忽略",
                                known, dbnum
                            );
                        }
                    }
                } else {
                    for (dbnum, sub) in by_dbnum {
                        cache_manager.insert_from_shape(dbnum, &sub);
                        let _ = touched_dbnums_for_insert.lock().map(|mut s| s.insert(dbnum));
                    }
                }

            }

        }

        // 等待所有的 db 写入句柄完成
        for h in db_write_handles {
            let _ = h.await;
        }
    });



    let target_root_refnos = super::non_full_noun::gen_geos_data(

        None,

        manual_refnos.clone(),

        db_option,

        incr_updates.clone(),

        sender.clone(),

        target_sesno,

        has_manual_refnos, // 手动模式时启用手动布尔运算

    )

    .await?;



    drop(sender);

    let _ = insert_task.await;



    // 方案 B：补齐 BRAN/HANG 的 tubi（从 SurrealDB 的 tubi_relate 读取最小必要信息），

    // 并写入 foyer cache，确保后续 export-dbnum-instances-json / mbd_pipe_api 能从 cache 拿到 inst_tubi_map 数据。

    if let Some(ref ctx) = foyer_cache_ctx {

        let branch_refnos = filter_bran_hang_refnos(&target_root_refnos).await;

        if !branch_refnos.is_empty() {

            println!(

                "[gen_model] cache-only: 开始写入 BRAN/HANG tubi_relate 到 cache（count={}）",

                branch_refnos.len()

            );

            use crate::fast_model::gen_model::tree_index_manager::TreeIndexManager;

            use std::collections::HashMap;



            // 定向生成可能跨 dbnum：按 owner_refno 分组写 cache。

            let mut groups: HashMap<u32, Vec<RefnoEnum>> = HashMap::new();

            for &owner in &branch_refnos {

                let Ok(dbnum) = TreeIndexManager::resolve_dbnum_for_refno(owner).await else {

                    continue;

                };

                if dbnum == 0 {

                    continue;

                }

                groups.entry(dbnum).or_default().push(owner);

            }



            for (dbnum, owners) in groups {

                match crate::fast_model::foyer_cache::geos::write_tubi_relate_into_cache_with_ctx(

                    ctx,

                    dbnum,

                    &owners,

                )

                .await

                {

                    Ok(cnt) => {

                        if cnt > 0 {

                            let _ = touched_dbnums.lock().map(|mut s| s.insert(dbnum));

                        }

                        println!(

                            "[gen_model] cache-only: dbnum={} 写入 tubi_relate -> cache 完成（tubi_cnt={}）",

                            dbnum, cnt

                        );

                    }

                    Err(e) => {

                        eprintln!(

                            "[gen_model] cache-only: dbnum={} 写入 tubi_relate -> cache 失败: {}",

                            dbnum, e

                        );

                    }

                }

            }

        }

    }



    println!(

        "[gen_model] {}路径几何体生成完成，共 {} 个根节点",

        mode_label,

        target_root_refnos.len()

    );



    if db_option.inner.gen_mesh {

        let mesh_start = Instant::now();

        println!("[gen_model] 开始 mesh 生成");



        let mut ran_primary = false;

        if let Some(ref ctx) = foyer_cache_ctx {

            let mesh_dir = db_option.inner.get_meshes_path();

            match crate::fast_model::foyer_cache::mesh::run_mesh_worker(

                ctx,

                &mesh_dir,

                &db_option.inner.mesh_precision,

                &db_option.mesh_formats,

            )

            .await

            {

                Ok(n) if n > 0 => {
                    println!("[gen_model] mesh worker 缓存路径完成: {} 个", n);
                    ran_primary = true;
                }

                Ok(_) => {
                    println!("[gen_model] mesh worker 缓存路径: 0 个 mesh，回退 SurrealDB");
                }

                Err(e) => {
                    eprintln!("[gen_model] mesh worker 缓存路径失败: {}", e);
                }

            }

        }

        if use_surrealdb && !ran_primary {

            if let Err(e) = run_mesh_worker(Arc::new(db_option.inner.clone()), 100).await {

                eprintln!("[gen_model] mesh worker 失败: {}", e);

            } else {

                ran_primary = true;

            }

        }



        if ran_primary {

            println!(

                "[gen_model] 完成 mesh 生成，用时 {} ms",

                mesh_start.elapsed().as_millis()

            );

        }



        if use_surrealdb {

            let aabb_start = Instant::now();

            println!("[gen_model] 开始写入 inst_relate_aabb");

            let mut aabb_refnos = target_root_refnos.clone();

            match query_provider::query_multi_descendants(&target_root_refnos, &[]).await {

                Ok(descendants) if !descendants.is_empty() => {

                    aabb_refnos.extend(descendants);

                }

                Ok(_) => {}

                Err(e) => {

                    eprintln!(

                        "[gen_model] 查询子孙节点失败，仅写根节点 inst_relate_aabb: {}",

                        e

                    );

                }

            };



            if let Err(e) = crate::fast_model::mesh_generate::update_inst_relate_aabbs_by_refnos(

                &aabb_refnos,

                db_option.is_replace_mesh(),

            )

            .await

            {

                eprintln!("[gen_model] 写入 inst_relate_aabb 失败: {}", e);

            } else {

                println!(

                    "[gen_model] 完成 inst_relate_aabb 写入，用时 {} ms",

                    aabb_start.elapsed().as_millis()

                );

            }

        }



        if db_option.inner.apply_boolean_operation {

            let bool_start = Instant::now();

            println!("[gen_model] 开始布尔运算 worker");



            // 构建 debug_model 过滤集合（用于调试模式只处理指定 refno）

            let filter_refnos: Option<std::collections::HashSet<aios_core::RefnoEnum>> = {

                let debug_refnos = db_option.inner.get_all_debug_refnos().await;

                if debug_refnos.is_empty() {

                    None

                } else {

                    Some(debug_refnos.into_iter().collect())

                }

            };



            if let Some(ref ctx) = foyer_cache_ctx {

                if let Err(e) = crate::fast_model::foyer_cache::boolean::run_boolean_worker_with_filter(

                    ctx,

                    filter_refnos.as_ref(),

                ).await {

                    eprintln!("[gen_model] 缓存布尔运算失败: {}", e);

                }

            }

            if use_surrealdb {

                if let Err(e) = run_boolean_worker(Arc::new(db_option.inner.clone()), 100).await {

                    eprintln!("[gen_model] boolean worker 失败: {}", e);

                }

            }



            println!(

                "[gen_model] 完成布尔运算，用时 {} ms",

                bool_start.elapsed().as_millis()

            );

        }

    }

    // ⚠️ 布尔运算完成后、capture 之前必须 close cache 强制刷盘。
    // 原因：布尔 worker 通过 foyer_cache_ctx 的 InstanceCacheManager 写入 inst_relate_bool_map，
    // 但 capture 内部会创建新的 InstanceCacheManager（query.rs），新实例只能从磁盘读取。
    // 若不先 close，布尔结果可能还在内存层，capture 看不到 → 截图中没有布尔孔洞。
    if let Some(ref ctx) = foyer_cache_ctx {

        if let Err(e) = ctx.cache().close().await {

            eprintln!("[cache] 关闭缓存失败: {}", e);

        }

    }



    if let Err(err) = capture_refnos_if_enabled(&target_root_refnos, db_option).await {

        eprintln!("[capture] 捕获截图失败: {}", err);

    }



    // 生成 SQLite 空间索引（从 foyer cache 批量落库）

    let touched_dbnums_vec: Vec<u32> = touched_dbnums

        .lock()

        .map(|s| s.iter().copied().collect())

        .unwrap_or_default();

    if let Err(e) = update_sqlite_spatial_index_from_cache(db_option, &touched_dbnums_vec).await {

        eprintln!("[gen_model] SQLite 空间索引生成失败: {}", e);

    }



    println!(

        "[gen_model] gen_all_geos_data 完成，总耗时 {} ms",

        time.elapsed().as_millis()

    );



    Ok(true)

}



/// 处理全量数据库生成（按 dbnum 循环）

async fn process_full_database_generation(

    db_option: &DbOptionExt,

    target_sesno: Option<u32>,

    time: Instant,

) -> Result<bool> {

    let mut dbnos: Vec<u32> = if let Some(nums) = db_option.inner.manual_db_nums.clone() {

        nums

    } else {

        aios_core::query_mdb_db_nums(None, aios_core::DBType::DESI).await?

    };



    // 过滤掉 exclude_db_nums 中的数据库编号

    if let Some(exclude_nums) = &db_option.inner.exclude_db_nums {

        use std::collections::HashSet;

        let exclude: HashSet<u32> = exclude_nums.iter().copied().collect();

        dbnos.retain(|dbnum| !exclude.contains(dbnum));

    }



    println!(

        "[gen_model] 进入全量生成路径，共 {} 个数据库待处理",

        dbnos.len()

    );



    let db_option_arc = Arc::new(db_option.clone());

    let use_surrealdb = db_option.use_surrealdb;

    let replace_exist = db_option.inner.is_replace_mesh();

    // 缓存功能已禁用

    if dbnos.is_empty() {

        println!("[gen_model] 未找到需要生成的数据库，直接结束");

    }



    // foyer cache-only 上下文：全量模式下也只初始化一次并在各 dbnum 间复用。

    let foyer_cache_ctx = crate::fast_model::foyer_cache::FoyerCacheContext::try_from_db_option(db_option).await?;



    // 初始化 Parquet 写入器

    let parquet_writer: Option<std::sync::Arc<ParquetStreamWriter>> = None;

    /*

    let parquet_writer = {

        let output_dir = db_option.inner.meshes_path.as_deref().unwrap_or("assets/meshes");

        let parquet_dir = std::path::Path::new(output_dir).parent().unwrap_or(std::path::Path::new("output"));

        match ParquetStreamWriter::new(&parquet_dir) {

            Ok(writer) => Some(std::sync::Arc::new(writer)),

            Err(e) => {

                eprintln!("[Parquet] 初始化写入器失败: {}, 跳过 Parquet 导出", e);

                None

            }

        }

    };

    */

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



    for dbnum in dbnos.clone() {

        println!("[gen_model] -> 开始处理数据库 {}", dbnum);

        let db_start = Instant::now();



        let (sender, receiver) = flume::unbounded();

        let receiver: flume::Receiver<aios_core::geometry::ShapeInstancesData> = receiver.clone();



        let parquet_writer_clone = parquet_writer.clone();

        #[cfg(feature = "duckdb-feature")]

        let duckdb_writer_clone = duckdb_writer.clone();



        let cache_manager_for_insert = foyer_cache_ctx.as_ref().map(|c| c.cache_arc());



        let insert_task = tokio::task::spawn(async move {

            while let Ok(shape_insts) = receiver.recv_async().await {

                if use_surrealdb {

                    if let Err(e) = save_instance_data_optimize(&shape_insts, replace_exist).await {

                        eprintln!("保存实例数据失败: {}", e);

                    }

                }

                if let Some(ref cache_manager) = cache_manager_for_insert {

                    let by_dbnum = match split_shape_instances_by_dbnum(&shape_insts).await {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!("[cache] batch 拆分 dbnum 失败，跳过写 cache: {}", e);
                            continue;
                        }
                    };
                    if by_dbnum.len() != 1 || !by_dbnum.contains_key(&dbnum) {
                        eprintln!(
                            "[cache] ⚠️ full-db 生成路径期望单库 dbnum={}，但 batch 出现 dbnums={:?}",
                            dbnum,
                            by_dbnum.keys().collect::<Vec<_>>()
                        );
                    }
                    if let Some(sub) = by_dbnum.get(&dbnum) {
                        cache_manager.insert_from_shape(dbnum, sub);
                    }

                }

                // 同时写入 Parquet（如果启用）

                if let Some(ref writer) = parquet_writer_clone {

                    if let Err(e) = writer.write_batch(&shape_insts) {

                        eprintln!("[Parquet] 写入批次失败: {}", e);

                    }

                }

                #[cfg(feature = "duckdb-feature")]

                if let Some(ref writer) = duckdb_writer_clone {

                    if let Err(e) = writer.write_batch(&shape_insts) {

                        eprintln!("[DuckDB] 写入批次失败: {}", e);

                    }

                }

            }

        });



        let db_refnos = super::non_full_noun::gen_geos_data_by_dbnum(

            dbnum,

            db_option_arc.clone(),

            sender.clone(),

            target_sesno,

        )

        .await?;



        drop(sender);

        let _ = insert_task.await;



        println!(

            "[gen_model] -> 数据库 {} insts 入库完成，用时 {} ms",

            dbnum,

            db_start.elapsed().as_millis()

        );



        // 方案 B：补齐 BRAN/HANG 的 tubi（从 SurrealDB 的 tubi_relate 读取最小必要信息），

        // 并写入 foyer cache，确保 export-dbnum-instances-json 等流程能拿到 inst_tubi_map。

        if let Some(ref ctx) = foyer_cache_ctx {

            let manager = TreeIndexManager::with_default_dir(vec![dbnum]);

            let mut branch_refnos = manager.query_noun_refnos("BRAN", None);

            branch_refnos.extend(manager.query_noun_refnos("HANG", None));

            if !branch_refnos.is_empty() {

                println!(

                    "[gen_model] cache-only: dbnum={} 开始写入 BRAN/HANG tubi_relate 到 cache（count={}）",

                    dbnum,

                    branch_refnos.len()

                );

                match crate::fast_model::foyer_cache::geos::write_tubi_relate_into_cache_with_ctx(

                    ctx,

                    dbnum,

                    &branch_refnos,

                )

                .await

                {

                    Ok(cnt) => println!(

                        "[gen_model] cache-only: dbnum={} 写入 tubi_relate -> cache 完成（tubi_cnt={}）",

                        dbnum, cnt

                    ),

                    Err(e) => eprintln!(

                        "[gen_model] cache-only: dbnum={} 写入 tubi_relate -> cache 失败: {}",

                        dbnum, e

                    ),

                }

            }

        }



            if db_option_arc.gen_mesh {

                let mesh_start = Instant::now();

                println!("[gen_model] -> 数据库 {} 开始生成三角网格", dbnum);



                let mut ran_primary = false;

            if let Some(ref ctx) = foyer_cache_ctx {

                let mesh_dir = db_option_arc.inner.get_meshes_path();

                match crate::fast_model::foyer_cache::mesh::run_mesh_worker(

                    ctx,

                    &mesh_dir,

                    &db_option_arc.inner.mesh_precision,

                    &db_option_arc.mesh_formats,

                )

                .await

                {

                    Ok(n) if n > 0 => {
                        println!("[gen_model] mesh worker 缓存路径完成: {} 个", n);
                        ran_primary = true;
                    }

                    Ok(_) => {
                        println!("[gen_model] mesh worker 缓存路径: 0 个 mesh，回退 SurrealDB");
                    }

                    Err(e) => {
                        eprintln!("[gen_model] mesh worker 缓存路径失败: {}", e);
                    }

                }

            }

            if use_surrealdb && !ran_primary {

                db_refnos

                    .execute_gen_inst_meshes(Some(db_option_arc.clone()))

                    .await;

                ran_primary = true;

            }



            if ran_primary {

                println!(

                    "[gen_model] -> 数据库 {} 三角网格生成完成，用时 {} ms",

                    dbnum,

                    mesh_start.elapsed().as_millis()

                );

            }

        }



        println!(

            "[gen_model] -> 数据库 {} 处理完成，总耗时 {} ms",

            dbnum,

            db_start.elapsed().as_millis()

        );

    }



    if let Some(ref ctx) = foyer_cache_ctx {

        if let Err(e) = ctx.cache().close().await {

            eprintln!("[cache] 关闭缓存失败: {}", e);

        }

    }



    // 完成 Parquet 写入并合并文件

    if let Some(ref writer) = parquet_writer {

        if let Err(e) = writer.finalize() {

            eprintln!("[Parquet] 合并文件失败: {}", e);

        }

    }

    #[cfg(feature = "duckdb-feature")]

    if let Some(ref writer) = duckdb_writer {

        if let Err(e) = writer.finalize() {

            eprintln!("[DuckDB] finalize 失败: {}", e);

        }

    }



    // 生成 SQLite 空间索引（从 foyer cache 批量落库）

    if let Err(e) = update_sqlite_spatial_index_from_cache(db_option, &dbnos).await {

        eprintln!("[gen_model] SQLite 空间索引生成失败: {}", e);

    }



    println!(

        "[gen_model] gen_all_geos_data 完成，总耗时 {} ms",

        time.elapsed().as_millis()

    );



    // ✅ 模型生成完毕后导出 instances.json（按 dbno）

    if db_option.export_instances {

        let mesh_dir =

            Path::new(db_option.inner.meshes_path.as_deref().unwrap_or("assets/meshes"));

        if let Err(e) = export_instances_json_for_dbnos(

            &dbnos,

            mesh_dir,

            &db_option.get_project_output_dir(),

            Arc::new(db_option.inner.clone()),

            true,

        )

        .await

        {

            eprintln!("[instances] 全量生成导出失败: {}", e);

        }

    }



    Ok(true)

}



/// 执行手动布尔运算

async fn execute_manual_boolean_operations(

    target_root_refnos: &[RefnoEnum],

    db_option: &DbOptionExt,

) {

    use crate::fast_model::manifold_bool::{

        apply_cata_neg_boolean_manifold, apply_insts_boolean_manifold,

    };

    use std::collections::HashSet;



    println!("[gen_model] 手动布尔运算模式：开始执行布尔运算");



    // 查询需要布尔运算的实例（基于 target_root_refnos 的子孙节点）

    let mut boolean_refnos = vec![];

    for &root_refno in target_root_refnos {

        // 查询深度可见实例

        if let Ok(visible_refnos) = query_deep_visible_inst_refnos(root_refno).await {

            boolean_refnos.extend(visible_refnos);

        }

        // 查询深度负实例

        if let Ok(neg_refnos) = query_deep_neg_inst_refnos(root_refno).await {

            boolean_refnos.extend(neg_refnos);

        }

    }



    // 去重

    let boolean_refnos: Vec<aios_core::RefnoEnum> = boolean_refnos

        .into_iter()

        .collect::<HashSet<_>>()

        .into_iter()

        .collect();



    if !boolean_refnos.is_empty() {

        let replace_exist = db_option.inner.is_replace_mesh();

        println!(

            "[gen_model] 手动布尔运算模式：找到 {} 个需要布尔运算的实例",

            boolean_refnos.len()

        );



        let boolean_start = Instant::now();



        // 执行元件库级布尔运算

        if let Err(e) = apply_cata_neg_boolean_manifold(&boolean_refnos, replace_exist).await {

            eprintln!("[gen_model] 手动布尔运算模式：元件库级布尔运算失败: {}", e);

        }



        // 执行实例级布尔运算

        if let Err(e) = apply_insts_boolean_manifold(&boolean_refnos, replace_exist).await {

            eprintln!("[gen_model] 手动布尔运算模式：实例级布尔运算失败: {}", e);

        } else {

            println!(

                "[gen_model] 手动布尔运算模式：布尔运算完成，用时 {} ms",

                boolean_start.elapsed().as_millis()

            );

        }

    } else {

        println!("[gen_model] 手动布尔运算模式：没有需要布尔运算的实例");

    }

}



// ============================================================================

// SQLite 空间索引：从 foyer cache 生成/增量更新 output/spatial_index.sqlite

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

    if !db_option.use_cache {

        return Ok(());

    }



    if !db_option.inner.enable_sqlite_rtree {

        // 常见误区：已切换到 cache 生成，但忘了开 enable_sqlite_rtree，导致 spatial_index.sqlite 不会更新，

        // 房间计算（SQLite RTree 粗筛）会退化/失效。

        let idx_path = SqliteSpatialIndex::default_path();

        if !idx_path.exists() {

            eprintln!(

                "[gen_model] 警告：use_cache=true 但 enable_sqlite_rtree=false，且未发现 {:?}；模型 AABB 不会落库到 SQLite。\

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

    let cache_dir = db_option.get_foyer_cache_dir();

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

