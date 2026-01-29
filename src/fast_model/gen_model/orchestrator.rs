//! 模型生成编排器
//!
//! 负责协调整个模型生成流程：
//! - Full Noun 模式 vs 非 Full Noun 模式的路由
//! - 几何体生成、Mesh 生成、布尔运算的编排
//! - 增量更新、手动 refno、调试模式的处理
//! - 空间索引和截图捕获的触发

use std::collections::BTreeSet;
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
use crate::fast_model::instance_cache::InstanceCacheManager;
use crate::fast_model::mesh_generate::{
    run_boolean_worker, run_mesh_worker, run_mesh_worker_from_cache_manager,
};
use crate::fast_model::manifold_bool::run_boolean_worker_from_cache_manager;
use crate::fast_model::pdms_inst::save_instance_data_optimize;
use crate::options::{DbOptionExt, MeshFormat};
use crate::fast_model::export_model::ParquetStreamWriter;
#[cfg(feature = "duckdb-feature")]
use crate::fast_model::export_model::{DuckDBStreamWriter, DuckDBWriteMode};

use super::config::FullNounConfig;
use super::errors::{FullNounError, Result};
use super::full_noun_mode::gen_full_noun_geos_optimized;
use super::models::NounCategory;
use std::str::FromStr;
use crate::fast_model::gen_model::tree_index_manager::TreeIndexManager;

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
pub async fn gen_all_geos_data(
    manual_refnos: Vec<RefnoEnum>,
    db_option: &DbOptionExt,
    incr_updates: Option<IncrGeoUpdateLog>,
    target_sesno: Option<u32>,
) -> Result<bool> {
    let time = Instant::now();
    let mut final_incr_updates = incr_updates;

    // 如果指定了 target_sesno，获取该 sesno 的增量数据
    if let Some(sesno) = target_sesno {
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

    // TreeIndex 文件（output/scene_tree/{dbnum}.tree）在不少流程中是必需的（Full Noun/导出/层级查询）。
    // 这里默认启用“缺失则从 SurrealDB 自动重建”，避免因缺文件导致生成结果为空。
    crate::fast_model::gen_model::tree_index_manager::enable_auto_generate_tree();

    // 调试：打印 Full Noun 模式配置
    println!(
        "[gen_model] Full Noun 模式配置: full_noun_mode={}, concurrency={}, batch_size={}",
        db_option.full_noun_mode,
        db_option.get_full_noun_concurrency(),
        db_option.get_full_noun_batch_size()
    );

    // ✅ 核心修复：确保 inst_relate 表已定义（显式创建 RELATION 表及索引）
    if let Err(e) = aios_core::rs_surreal::inst::init_model_tables().await {
        eprintln!("[gen_model] ❌ 初始化 inst_relate 表结构失败: {}", e);
        // 严重错误，建议直接中断，否则后续写入必挂
        return Err(FullNounError::Other(e));
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
    if has_debug || has_manual_refnos || is_incr_update {
        process_targeted_generation(
            manual_refnos,
            db_option,
            final_incr_updates,
            target_sesno,
            time,
        )
        .await
    } else if db_option.full_noun_mode {
        process_full_noun_mode(db_option, final_incr_updates, time).await
    } else {
        process_full_database_generation(db_option, target_sesno, time).await
    }
}

async fn resolve_dbnum_from_shape(
    shape_insts: &aios_core::geometry::ShapeInstancesData,
) -> Option<u32> {
    let refno = shape_insts
        .inst_info_map
        .keys()
        .next()
        .copied()
        .or_else(|| shape_insts.inst_tubi_map.keys().next().copied());
    match refno {
        Some(r) => {
            if db_meta().ensure_loaded().is_ok() {
                if let Some(dbnum) = db_meta().get_dbnum_by_refno(r) {
                    return Some(dbnum);
                }
            }
            TreeIndexManager::resolve_dbnum_for_refno(r).await.ok()
        }
        None => None,
    }
}

/// 处理 Full Noun 模式的生成流程
async fn process_full_noun_mode(
    db_option: &DbOptionExt,
    incr_updates: Option<IncrGeoUpdateLog>,
    time: Instant,
) -> Result<bool> {
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

    // 1️⃣ 生成/更新 inst_relate，并获取分类后的根 refno
    let config = FullNounConfig::from_db_option_ext(db_option)
        .map_err(|e| anyhow::anyhow!("配置错误: {}", e))?;

    let (sender, receiver) = flume::unbounded();
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

    let cache_manager = if db_option.use_cache {
        Some(Arc::new(
            InstanceCacheManager::new(&db_option.get_foyer_cache_dir()).await?,
        ))
    } else {
        None
    };
    let cache_manager_for_insert = cache_manager.clone();
    let touched_dbnums: Arc<std::sync::Mutex<BTreeSet<u32>>> =
        Arc::new(std::sync::Mutex::new(BTreeSet::new()));
    let touched_dbnums_for_insert = touched_dbnums.clone();

    let insert_handle = tokio::spawn(async move {
        while let Ok(shape_insts) = receiver.recv_async().await {
            // 保存到 SurrealDB
            if use_surrealdb {
                if let Err(e) = save_instance_data_optimize(&shape_insts, replace_exist).await {
                    eprintln!("保存实例数据失败: {}", e);
                }
            }
            if let Some(ref cache_manager) = cache_manager_for_insert {
                if let Some(dbnum) = resolve_dbnum_from_shape(&shape_insts).await {
                    cache_manager.insert_from_shape(dbnum, &shape_insts);
                    let _ = touched_dbnums_for_insert.lock().map(|mut s| s.insert(dbnum));
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

    let categorized =
        gen_full_noun_geos_optimized(Arc::new(db_option.clone()), &config, sender.clone())
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

        if let Some(ref cache_manager) = cache_manager {
            let mesh_dir = db_option.inner.get_meshes_path();
            if let Err(e) = run_mesh_worker_from_cache_manager(
                cache_manager.as_ref(),
                &mesh_dir,
                &db_option.inner.mesh_precision,
                &db_option.mesh_formats,
            )
            .await
            {
                eprintln!("[gen_model] mesh worker 缓存路径失败: {}", e);
            } else {
                ran_primary = true;
            }
        }

        if use_surrealdb {
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

        // 3️⃣ 写入 inst_relate_aabb 并导出 Parquet（供房间计算使用）
        if use_surrealdb {
        let aabb_start = Instant::now();
        println!("[gen_model] Full Noun 模式开始写入 inst_relate_aabb");
        // 使用实际 inst_relate 中的 refno，避免分类结果与 inst 列表不一致导致 0 写入
        let aabb_refnos = match crate::fast_model::mesh_generate::fetch_inst_relate_refnos().await {
            Ok(v) if !v.is_empty() => v,
            Ok(_) => {
                eprintln!("[gen_model] pe_transform 中无可用 refno，跳过 AABB 写入");
                Vec::new()
            }
            Err(e) => {
                eprintln!("[gen_model] 获取 pe_transform refno 失败: {}", e);
                Vec::new()
            }
        };

        if aabb_refnos.is_empty() {
            eprintln!("[gen_model] Full Noun 模式写入 inst_relate_aabb 被跳过：没有可用 refno");
        } else {
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

                // 导出前端模型数据 (db_models_{dbnum}.parquet)
                // let export_path = std::path::Path::new("assets/database_models");
                // if let Err(e) = crate::fast_model::export_model::export_parquet::export_db_models_parquet(
                //     export_path,
                //     None, // 导出所有已生成的 dbnums
                // ).await {
                //     eprintln!("[gen_model] Full Noun 模式导出前端模型 Parquet 失败: {}", e);
                // }
            }
        }
        }

        // 4️⃣ 可选执行布尔运算
        if db_option.inner.apply_boolean_operation {
            let bool_start = Instant::now();
            println!("[gen_model] Full Noun 模式开始布尔运算（boolean worker）");
            if let Some(ref cache_manager) = cache_manager {
                if let Err(e) = run_boolean_worker_from_cache_manager(cache_manager.as_ref()).await
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
            Path::new("output"),
            Arc::new(db_option.inner.clone()),
            true,
        )
        .await
        {
            eprintln!("[instances] Full Noun 导出失败: {}", e);
        }
    }

    if let Some(ref cache_manager) = cache_manager {
        if let Err(e) = cache_manager.close().await {
            eprintln!("[cache] 关闭缓存失败: {}", e);
        }
    }

    Ok(true)
}

/// 处理增量/手动/调试模式的目标生成
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

    // 生成前置预检查（通用）：确保 pe_transform(world_trans) 可用。
    // inst_relate 不再保存 world_transform，因此这里必须确保 pe_transform 侧数据齐备。
    let precheck_refnos: Vec<RefnoEnum> = if let Some(upd) = &incr_updates {
        upd.get_all_visible_refnos().into_iter().collect()
    } else if has_manual_refnos {
        manual_refnos.clone()
    } else {
        db_option
            .inner
            .debug_model_refnos
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|s| s.parse::<RefnoEnum>().ok())
            .collect()
    };
    if !precheck_refnos.is_empty() {
        crate::fast_model::precheck::ensure_pe_transform_for_refnos(&precheck_refnos)
            .await
            .map_err(FullNounError::Other)?;
    }

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

    let (sender, receiver) = flume::unbounded();
    let receiver: flume::Receiver<aios_core::geometry::ShapeInstancesData> = receiver.clone();

    let replace_exist = db_option.inner.is_replace_mesh();
    let use_surrealdb = db_option.use_surrealdb;
    let cache_manager = if db_option.use_cache {
        Some(Arc::new(
            InstanceCacheManager::new(&db_option.get_foyer_cache_dir()).await?,
        ))
    } else {
        None
    };
    let cache_manager_for_insert = cache_manager.clone();
    let touched_dbnums: Arc<std::sync::Mutex<BTreeSet<u32>>> =
        Arc::new(std::sync::Mutex::new(BTreeSet::new()));
    let touched_dbnums_for_insert = touched_dbnums.clone();

    let insert_task = tokio::task::spawn(async move {
        while let Ok(shape_insts) = receiver.recv_async().await {
            if use_surrealdb {
                if let Err(e) = save_instance_data_optimize(&shape_insts, replace_exist).await {
                    eprintln!("保存实例数据失败: {}", e);
                }
            }
            if let Some(ref cache_manager) = cache_manager_for_insert {
                if let Some(dbnum) = resolve_dbnum_from_shape(&shape_insts).await {
                    cache_manager.insert_from_shape(dbnum, &shape_insts);
                    let _ = touched_dbnums_for_insert.lock().map(|mut s| s.insert(dbnum));
                }
            }
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

    println!(
        "[gen_model] {}路径几何体生成完成，共 {} 个根节点",
        mode_label,
        target_root_refnos.len()
    );

    if db_option.inner.gen_mesh {
        let mesh_start = Instant::now();
        println!("[gen_model] 开始 mesh 生成");

        let mut ran_primary = false;
        if let Some(ref cache_manager) = cache_manager {
            let mesh_dir = db_option.inner.get_meshes_path();
            if let Err(e) = run_mesh_worker_from_cache_manager(
                cache_manager.as_ref(),
                &mesh_dir,
                &db_option.inner.mesh_precision,
                &db_option.mesh_formats,
            )
            .await
            {
                eprintln!("[gen_model] mesh worker 缓存路径失败: {}", e);
            } else {
                ran_primary = true;
            }
        }
        if use_surrealdb {
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

            if let Some(ref cache_manager) = cache_manager {
                if let Err(e) = run_boolean_worker_from_cache_manager(cache_manager.as_ref()).await
                {
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

    if let Err(err) = capture_refnos_if_enabled(&target_root_refnos, db_option).await {
        eprintln!("[capture] 捕获截图失败: {}", err);
    }

    if let Some(ref cache_manager) = cache_manager {
        if let Err(e) = cache_manager.close().await {
            eprintln!("[cache] 关闭缓存失败: {}", e);
        }
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
    // 缓存功能已禁用
    if dbnos.is_empty() {
        println!("[gen_model] 未找到需要生成的数据库，直接结束");
    }

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

        let cache_manager = if db_option_arc.use_cache {
            Some(Arc::new(
                InstanceCacheManager::new(&db_option_arc.get_foyer_cache_dir()).await?,
            ))
        } else {
            None
        };
        let cache_manager_for_insert = cache_manager.clone();

        let insert_task = tokio::task::spawn(async move {
            while let Ok(shape_insts) = receiver.recv_async().await {
                if use_surrealdb {
                    if let Err(e) = save_instance_data_optimize(&shape_insts, false).await {
                        eprintln!("保存实例数据失败: {}", e);
                    }
                }
                if let Some(ref cache_manager) = cache_manager_for_insert {
                    cache_manager.insert_from_shape(dbnum, &shape_insts);
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

        if db_option_arc.gen_mesh {
            let mesh_start = Instant::now();
            println!("[gen_model] -> 数据库 {} 开始生成三角网格", dbnum);

            let mut ran_primary = false;
            if let Some(ref cache_manager) = cache_manager {
                let mesh_dir = db_option_arc.inner.get_meshes_path();
                if let Err(e) = run_mesh_worker_from_cache_manager(
                    cache_manager.as_ref(),
                    &mesh_dir,
                    &db_option_arc.inner.mesh_precision,
                    &db_option_arc.mesh_formats,
                )
                .await
                {
                    eprintln!("[gen_model] mesh worker 缓存路径失败: {}", e);
                } else {
                    ran_primary = true;
                }
            }
            if use_surrealdb {
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
            Path::new("output"),
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
    let base_out = PathBuf::from("output/instances_cache_for_index");
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
