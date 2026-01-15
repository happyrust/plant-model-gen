//! 模型生成编排器
//!
//! 负责协调整个模型生成流程：
//! - Full Noun 模式 vs 非 Full Noun 模式的路由
//! - 几何体生成、Mesh 生成、布尔运算的编排
//! - 增量更新、手动 refno、调试模式的处理
//! - 空间索引和截图捕获的触发

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use crate::fast_model::export_model::export_prepack_lod::export_prepack_lod_for_refnos;
use crate::fast_model::export_model::export_prepack_lod::export_instances_json_for_dbnos;
use crate::fast_model::export_model::export_prepack_lod::export_instances_json_for_refnos_grouped_by_dbno;
use crate::fast_model::unit_converter::LengthUnit;

use aios_core::RefnoEnum;

use crate::data_interface::increment_record::IncrGeoUpdateLog;
use crate::data_interface::sesno_increment::get_changes_at_sesno;
use crate::fast_model::capture::capture_refnos_if_enabled;
use crate::fast_model::mesh_generate::run_mesh_worker;
use crate::fast_model::pdms_inst::save_instance_data_optimize;
use crate::options::{DbOptionExt, MeshFormat};
use crate::fast_model::export_model::ParquetStreamWriter;
#[cfg(feature = "duckdb-feature")]
use crate::fast_model::export_model::{DuckDBStreamWriter, DuckDBWriteMode};

use super::config::FullNounConfig;
use super::errors::{FullNounError, Result};
use super::full_noun_mode::gen_full_noun_geos_optimized;
use super::models::NounCategory;

fn parse_dbno_from_refno(refno: RefnoEnum) -> Option<u32> {
    // RefnoEnum 的 to_string 在项目中通常是 "dbno_sesno" 或 "dbno/sesno"；
    // 这里做最小兼容解析，只取 dbno。
    let s = refno.to_string().replace('/', "_");
    s.split('_').next()?.parse::<u32>().ok()
}

/// 主入口函数：生成所有几何体数据
///
/// 这是主要的公共 API，根据配置路由到不同的生成策略：
/// - Full Noun 模式：使用新优化的 gen_full_noun_geos_optimized 管线
/// - 非 Full Noun 模式：
///   - 增量更新模式（incr_updates 非空）
///   - 手动 refno 模式（manual_refnos 非空）
///   - 调试模式（debug_model_refnos 非空）
///   - 全量生成模式（按 dbno 循环）
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

    // 调试：打印 Full Noun 模式配置
    println!(
        "[gen_model] Full Noun 模式配置: full_noun_mode={}, concurrency={}, batch_size={}",
        db_option.full_noun_mode,
        db_option.get_full_noun_concurrency(),
        db_option.get_full_noun_batch_size()
    );

    // =========================
    // Full Noun 模式：新管线
    // =========================
    // =========================
    // 判断是否有调试/手动指定的 refno
    // =========================
    let is_incr_update = final_incr_updates.is_some();
    let has_manual_refnos = !manual_refnos.is_empty();
    let has_debug = db_option.inner.debug_model_refnos.is_some();

    // 如果有调试 refno 或手动 refno，优先走调试路径（即使在 Full Noun 模式下）
    if has_debug || has_manual_refnos || is_incr_update {
        // 增量/手动/调试路径
        process_targeted_generation(
            manual_refnos,
            db_option,
            final_incr_updates,
            target_sesno,
            time,
        )
        .await
    } else if db_option.full_noun_mode {
        // Full Noun 模式：新管线
        process_full_noun_mode(db_option, final_incr_updates, time).await
    } else {
        // 全量生成路径（按 dbno 循环）
        process_full_database_generation(db_option, target_sesno, time).await
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

    let insert_handle = tokio::spawn(async move {
        while let Ok(shape_insts) = receiver.recv_async().await {
            // 保存到 SurrealDB
            if let Err(e) = save_instance_data_optimize(&shape_insts, replace_exist).await {
                eprintln!("保存实例数据失败: {}", e);
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
        gen_full_noun_geos_optimized(Arc::new(db_option.clone()), &config, sender)
            .await
            .map_err(|e| anyhow::anyhow!("Full Noun 生成失败: {}", e))?;

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

        // 使用 mesh worker 后台扫描 inst_geo 表生成 mesh
        if let Err(e) = run_mesh_worker(Arc::new(db_option.inner.clone()), 100).await {
            eprintln!("[gen_model] mesh worker 失败: {}", e);
        } else {
            println!(
                "[gen_model] Full Noun 模式 mesh 生成完成，用时 {} ms",
                mesh_start.elapsed().as_millis()
            );
        }

        // 3️⃣ 写入 inst_relate_aabb 并导出 Parquet（供房间计算使用）
        let aabb_start = Instant::now();
        println!("[gen_model] Full Noun 模式开始写入 inst_relate_aabb");
        // 使用实际 inst_relate 中的 refno，避免分类结果与 inst 列表不一致导致 0 写入
        let aabb_refnos = match crate::fast_model::mesh_generate::fetch_inst_relate_refnos().await {
            Ok(v) if !v.is_empty() => v,
            Ok(_) => {
                eprintln!("[gen_model] inst_relate 中无可用 refno，跳过 AABB 写入");
                Vec::new()
            }
            Err(e) => {
                eprintln!("[gen_model] 获取 inst_relate refno 失败: {}", e);
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

        // 4️⃣ 可选执行布尔运算
        if db_option.inner.apply_boolean_operation {
            let bool_start = Instant::now();
            println!("[gen_model] Full Noun 模式开始布尔运算（boolean worker）");
            if let Err(e) = crate::fast_model::mesh_generate::run_boolean_worker(
                Arc::new(db_option.inner.clone()),
                100,
            )
            .await
            {
                eprintln!("[gen_model] Full Noun 布尔运算失败: {}", e);
            } else {
                println!(
                    "[gen_model] Full Noun 模式布尔运算完成，用时 {} ms",
                    bool_start.elapsed().as_millis()
                );
            }
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
            dbnos.retain(|dbno| !exclude.contains(dbno));
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

    let (sender, receiver) = flume::unbounded();
    let receiver: flume::Receiver<aios_core::geometry::ShapeInstancesData> = receiver.clone();

    let replace_exist = db_option.inner.is_replace_mesh();

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
        match DuckDBStreamWriter::new(&duckdb_dir, DuckDBWriteMode::Append) {
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

    let insert_task = tokio::task::spawn(async move {
        while let Ok(shape_insts) = receiver.recv_async().await {
            if let Err(e) = save_instance_data_optimize(&shape_insts, replace_exist).await {
                eprintln!("保存实例数据失败: {}", e);
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
        println!(
            "[gen_model] 开始后台扫描 inst_geo 生成 mesh"
        );

        // 使用 mesh worker 后台扫描 inst_geo 表生成 mesh
        if let Err(e) = run_mesh_worker(Arc::new(db_option.inner.clone()), 100).await {
            eprintln!("[gen_model] mesh worker 失败: {}", e);
        } else {
            println!(
                "[gen_model] 完成 mesh 生成，用时 {} ms",
                mesh_start.elapsed().as_millis()
            );
        }
        
        // 写入实例 AABB（mesh worker 只更新了 inst_geo.aabb）
        let aabb_start = Instant::now();
        println!("[gen_model] 开始写入 inst_relate_aabb");
        // 注意：export-glb 等导出会查询“根 + 全部子孙”的 inst 列表；
        // 仅写根节点会导致子孙节点缺少 inst_relate_aabb，从而 world_aabb 变 null 并反序列化失败。
        let aabb_refnos = match aios_core::collect_descendant_filter_ids(&target_root_refnos, &[], None).await
        {
            Ok(v) if !v.is_empty() => v,
            Ok(_) => target_root_refnos.clone(),
            Err(e) => {
                eprintln!(
                    "[gen_model] 查询子孙节点失败，回退仅写根节点 inst_relate_aabb: {}",
                    e
                );
                target_root_refnos.clone()
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

        // 运行 boolean worker 处理布尔运算
        let bool_start = Instant::now();
        println!("[gen_model] 开始布尔运算 worker");
        
        if let Err(e) = crate::fast_model::mesh_generate::run_boolean_worker(
            Arc::new(db_option.inner.clone()),
            100
        ).await {
            eprintln!("[gen_model] boolean worker 失败: {}", e);
        } else {
            println!(
                "[gen_model] 完成布尔运算，用时 {} ms",
                bool_start.elapsed().as_millis()
            );
        }
    }

    // ✅ 等待所有 workers 完成后，再合并 Parquet 文件
    if let Some(ref writer) = parquet_writer {
        println!("[Parquet] 所有 workers 已完成，开始合并文件...");
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

    if let Err(err) = capture_refnos_if_enabled(&target_root_refnos, &db_option.inner).await {
        eprintln!("[capture] 捕获截图失败: {}", err);
    }

    // initialize_spatial_index();

    println!(
        "[gen_model] gen_all_geos_data 完成，总耗时 {} ms",
        time.elapsed().as_millis()
    );

    // ✅ 模型生成完毕后导出 instances.json（仅导出本次涉及的 dbno）
    if db_option.export_instances {
        let mesh_dir =
            Path::new(db_option.inner.meshes_path.as_deref().unwrap_or("assets/meshes"));
        if let Err(e) = export_instances_json_for_refnos_grouped_by_dbno(
            &target_root_refnos,
            mesh_dir,
            Path::new("output"),
            Arc::new(db_option.inner.clone()),
            true,
        )
        .await
        {
            eprintln!("[instances] 目标生成导出失败: {}", e);
        }
    }

    Ok(true)
}

/// 处理全量数据库生成（按 dbno 循环）
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
        dbnos.retain(|dbno| !exclude.contains(dbno));
    }

    println!(
        "[gen_model] 进入全量生成路径，共 {} 个数据库待处理",
        dbnos.len()
    );

    let db_option_arc = Arc::new(db_option.clone());
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

    for dbno in dbnos.clone() {
        println!("[gen_model] -> 开始处理数据库 {}", dbno);
        let db_start = Instant::now();

        let (sender, receiver) = flume::unbounded();
        let receiver: flume::Receiver<aios_core::geometry::ShapeInstancesData> = receiver.clone();

        let parquet_writer_clone = parquet_writer.clone();
        #[cfg(feature = "duckdb-feature")]
        let duckdb_writer_clone = duckdb_writer.clone();

        let insert_task = tokio::task::spawn(async move {
            while let Ok(shape_insts) = receiver.recv_async().await {
                if let Err(e) = save_instance_data_optimize(&shape_insts, false).await {
                    eprintln!("保存实例数据失败: {}", e);
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
            dbno,
            db_option_arc.clone(),
            sender.clone(),
            target_sesno,
        )
        .await?;

        drop(sender);
        let _ = insert_task.await;

        println!(
            "[gen_model] -> 数据库 {} insts 入库完成，用时 {} ms",
            dbno,
            db_start.elapsed().as_millis()
        );

        if db_option_arc.gen_mesh {
            let mesh_start = Instant::now();
            println!("[gen_model] -> 数据库 {} 开始生成三角网格", dbno);

            db_refnos
                .execute_gen_inst_meshes(Some(db_option_arc.clone()))
                .await;

            println!(
                "[gen_model] -> 数据库 {} 三角网格生成完成，用时 {} ms",
                dbno,
                mesh_start.elapsed().as_millis()
            );
        }

        println!(
            "[gen_model] -> 数据库 {} 处理完成，总耗时 {} ms",
            dbno,
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

    // initialize_spatial_index();

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
        if let Ok(visible_refnos) = aios_core::query_deep_visible_inst_refnos(root_refno).await {
            boolean_refnos.extend(visible_refnos);
        }
        // 查询深度负实例
        if let Ok(neg_refnos) = aios_core::query_deep_neg_inst_refnos(root_refno).await {
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
