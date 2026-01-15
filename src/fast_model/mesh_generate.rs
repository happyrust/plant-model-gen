//! CSG 几何体网格生成模块
//!
//! 本模块提供基于 CSG（Constructive Solid Geometry）的几何体网格生成功能，包括：
//! - 实例网格生成（使用 Manifold 库）
//! - 包围盒（AABB）更新
//! - 布尔运算处理
//! - SQLite 空间索引优化支持

use crate::fast_model::export_model::export_glb::export_single_mesh_to_glb;
use crate::fast_model::manifold_bool::{
    apply_cata_neg_boolean_manifold, apply_insts_boolean_manifold,
};
use crate::fast_model::{EXIST_MESH_GEO_HASHES, utils};
use crate::fast_model::{debug_model, debug_model_debug, debug_model_warn};
use crate::options::{DbOptionExt, MeshFormat};
use crate::{batch_update_err, db_err, deser_err, log_err, query_err};
use aios_core::accel_tree::acceleration_tree::RStarBoundingBox;
use aios_core::error::{init_deserialize_error, init_query_error, init_save_database_error};
use aios_core::geometry::csg::GeneratedMesh;
use aios_core::mesh_precision::MeshPrecisionSettings;
use aios_core::options::DbOption;
use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
use aios_core::SurrealQueryExt;
use aios_core::shape::pdms_shape::{PlantMesh, RsVec3};
use aios_core::tool::float_tool::{dvec4_round_3, f64_round};
use aios_core::{
    RecordId, RefU64, RefnoEnum, SUL_DB, gen_bytes_hash, get_inst_relate_keys,
    query_deep_neg_inst_refnos, query_deep_visible_inst_refnos, utils::RecordIdExt,
};
use aios_core::{get_db_option, init_test_surreal};
// 导入几何查询相关的结构体和方法
use aios_core::{
    CataNegGroup, GeoAabbTrans, GeoParam, GmGeoData, ManiGeoTransQuery, NegInfo, ParamNegInfo,
    QueryAabbParam, QueryGeoParam, query_aabb_params, query_geo_params, query_inst_geo_ids,
};
// 使用 aios_core 中查询方法的宏
use aios_core::query_db;
use anyhow::anyhow;
use bevy_transform::prelude::Transform;
use chrono;
use dashmap::DashMap;
use glam::DMat4;
use itertools::Itertools;
use log::info;
use parry3d::bounding_volume::*;
use parry3d::math::Isometry;
use parse_pdms_db::parse::round_f32;
use serde_json::Value as JsonValue;
use surrealdb::types as surrealdb_types;
use surrealdb::types::SurrealValue;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use aios_core::geometry::csg::generate_csg_mesh;

/// 在数据库中生成网格模型并更新包围盒
///
/// 该函数按批次处理参考号，依次执行：
/// 1. 生成实例网格文件
/// 2. 更新实例关联的包围盒数据
///
/// # 参数
///
/// * `option` - 数据库选项，包含网格路径、精度设置等配置
/// * `refnos` - 需要处理的参考号数组
///
/// # 返回值
///
/// 返回 `anyhow::Result<()>` 表示执行是否成功
pub async fn gen_meshes_in_db(
    option: Option<Arc<DbOption>>,
    refnos: &[RefnoEnum],
) -> anyhow::Result<()> {
    if refnos.is_empty() {
        return Ok(());
    }
    let replace_exist = option
        .as_ref()
        .map(|x| x.is_replace_mesh())
        .unwrap_or(false);
    // let time = std::time::Instant::now();
    let dir = option
        .as_ref()
        .map(|x| x.get_meshes_path())
        .unwrap_or("assets/meshes".into());

    // Check if the directory exists, if not, create it
    if !std::path::Path::new(&dir).exists() {
        std::fs::create_dir_all(&dir)?;
    }
    let precision = Arc::new(
        option
            .as_ref()
            .map(|opt| opt.mesh_precision().clone())
            .unwrap_or_else(|| get_db_option().mesh_precision().clone()),
    );
    for chunk in refnos.chunks(100) {
        // 生成模型文件
        gen_inst_meshes(&dir, &precision, chunk, replace_exist, &[MeshFormat::PdmsMesh])
            .await
            .unwrap();
        // println!(
        //     "gen_inst_meshes finished: {} ms",
        //     time.elapsed().as_millis()
        // );
        // let time = std::time::Instant::now();
        update_inst_relate_aabbs_by_refnos(chunk, replace_exist)
            .await
            .unwrap();
        // println!(
        //     "update_inst_relate_aabbs finished: {} ms",
        //     time.elapsed().as_millis()
        // );
    }
    Ok(())
}

/// 查询需要执行 catalog 级布尔运算的实例列表
async fn query_pending_cata_boolean(
    limit: usize,
    replace_exist: bool,
) -> anyhow::Result<Vec<RefnoEnum>> {
    let filter_booled = if replace_exist {
        String::new()
    } else {
        // 非覆盖模式下：跳过已成功写入 inst_relate_cata_bool 的实例，避免重复计算
        "AND (SELECT status FROM inst_relate_cata_bool WHERE refno = in AND status = 'Success' LIMIT 1) = NONE"
            .to_string()
    };

    let sql = format!(
        r#"SELECT VALUE in
	FROM inst_relate
	WHERE has_cata_neg = true
	  {filter_booled}
	LIMIT {limit};"#,
    );

    let refnos: Vec<RefnoEnum> = SUL_DB.query_take(&sql, 0).await?;
    Ok(refnos)
}

/// 扫描关系表，提取指向正实体的目标 refno（去重后返回）
async fn query_relation_targets(table: &str) -> anyhow::Result<Vec<RefnoEnum>> {
    let sql = format!(
        r#"SELECT VALUE out
FROM {table}
GROUP BY out;"#
    );
    let refnos: Vec<RefnoEnum> = SUL_DB.query_take(&sql, 0).await?;
    Ok(refnos)
}

/// 聚合 neg_relate 与 ngmr_relate 的目标集合（去重）
async fn query_relation_targets_combined() -> anyhow::Result<HashSet<RefnoEnum>> {
    let neg_targets = query_relation_targets("neg_relate").await?;
    let ngmr_targets = query_relation_targets("ngmr_relate").await?;
    let mut candidates: HashSet<RefnoEnum> = HashSet::new();
    candidates.extend(neg_targets.iter().copied());
    candidates.extend(ngmr_targets.iter().copied());

    println!(
        "[boolean_worker] 关系扫描: neg_targets={} ngmr_targets={} unique_targets={}",
        neg_targets.len(),
        ngmr_targets.len(),
        candidates.len()
    );

    Ok(candidates)
}

/// 查询需要执行实例级布尔运算的实例列表（仅限存在负关系的目标）
async fn query_pending_inst_boolean(
    limit: usize,
    replace_exist: bool,
    candidates: &HashSet<RefnoEnum>,
) -> anyhow::Result<Vec<RefnoEnum>> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let filter_booled = if replace_exist {
        String::new()
    } else {
        "AND (SELECT status FROM inst_relate_bool WHERE refno = inst_relate_aabb.refno AND status = 'Success' LIMIT 1) = NONE".to_string()
    };

    const CHUNK_SIZE: usize = 200;
    let candidate_keys: Vec<String> = candidates.iter().map(|r| r.to_pe_key()).collect();
    let mut pending: Vec<RefnoEnum> = Vec::new();
    for chunk in candidate_keys.chunks(CHUNK_SIZE) {
        if pending.len() >= limit {
            break;
        }
        let remaining = limit - pending.len();
        let sql = format!(
            r#"
SELECT VALUE in FROM inst_relate_aabb
WHERE in IN [{}]
{filter_booled}
LIMIT {remaining};
"#,
            chunk.join(","),
            filter_booled = filter_booled
        );

        let mut refnos: Vec<RefnoEnum> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
        pending.append(&mut refnos);
    }

    pending.truncate(limit);
    Ok(pending)
}

/// 查询需要生成 mesh 的 inst_geo 记录的 id
/// 条件：meshed = false, param != NONE, bad != true
/// 返回 inst_geo 的 id 列表（geo_hash）
async fn query_pending_mesh_geo_ids(limit: usize, replace_exist: bool) -> anyhow::Result<Vec<RecordId>> {
    let sql = if replace_exist {
        format!(
            "SELECT value id FROM inst_geo WHERE param != NONE AND bad != true LIMIT {}",
            limit
        )
    } else {
        format!(
            "SELECT value id FROM inst_geo WHERE meshed != true AND param != NONE AND bad != true LIMIT {}",
            limit
        )
    };
    
    let ids: Vec<RecordId> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
    Ok(ids)
}

/// 基于 inst_geo 状态的 Mesh 生成 Worker
///
/// 按批次扫描需要生成 mesh 的 inst_geo 记录，直接基于 geo_id 生成网格。
pub async fn run_mesh_worker(db_option: Arc<DbOption>, batch_size: usize) -> anyhow::Result<()> {
    let batch_size = batch_size.max(1);
    let replace_exist = db_option.is_replace_mesh();
    let mut round = 0usize;
    let mut total_processed = 0usize;
    let mut stalled_rounds = 0usize;
    let mut last_pending: Option<HashSet<String>> = None;
    
    // 获取 mesh 生成所需的配置
    let mesh_dir = db_option.get_meshes_path();
    if !mesh_dir.exists() {
        std::fs::create_dir_all(&mesh_dir)?;
    }
    
    let precision = db_option.mesh_precision().clone();
    let mesh_formats = crate::options::get_db_option_ext().mesh_formats.clone();

    // 性能优化：启动前预加载数据库中已网格化的几何信息到内存，避免后续循环中重复查询。
    crate::fast_model::preload_mesh_cache().await?;

    loop {
        let round_start = std::time::Instant::now();
        let pending_geo_ids = query_pending_mesh_geo_ids(batch_size, replace_exist).await?;
        
        let pending: HashSet<_> = pending_geo_ids.iter().map(|id| id.to_raw()).collect();
        
        if pending.is_empty() {
            println!("[mesh_worker] 没有待处理 mesh 任务，退出");
            break;
        }
        
        // 检测是否卡住（连续多轮处理相同的 geo_ids）
        if let Some(prev) = &last_pending {
            if *prev == pending {
                stalled_rounds += 1;
            } else {
                stalled_rounds = 0;
            }
        }
        last_pending = Some(pending.clone());
        
        round += 1;
        println!(
            "[mesh_worker] 轮次 {}: pending={} batch_size={} replace_exist={}",
            round,
            pending_geo_ids.len(),
            batch_size,
            replace_exist
        );
        
        if !pending_geo_ids.is_empty() {
            let t = std::time::Instant::now();
            // 直接基于 geo_ids 生成 mesh
            gen_inst_meshes_by_geo_ids(
                &mesh_dir,
                &precision,
                &pending_geo_ids,
                &mesh_formats,
            ).await?;
            
            println!(
                "[mesh_worker] 轮次 {} mesh 生成完成: {} 个，用时 {} ms",
                round,
                pending_geo_ids.len(),
                t.elapsed().as_millis()
            );
        }
        
        total_processed += pending_geo_ids.len();
        println!(
            "[mesh_worker] 轮次 {} 结束: 本轮 {} 个，累计 {} 个，用时 {} ms",
            round,
            pending_geo_ids.len(),
            total_processed,
            round_start.elapsed().as_millis()
        );
        
        // 如果连续3轮 pending 集合未变化，可能卡住了
        if stalled_rounds >= 3 {
            let sample: Vec<_> = pending.iter().take(5).cloned().collect();
            if replace_exist {
                // replace_exist=true 时 stall 是预期的（已处理的记录会再次被查询到）
                println!(
                    "[mesh_worker] replace_exist=true 模式下检测到 stall，已完成 {} 个，退出",
                    total_processed
                );
                break;
            } else {
                return Err(anyhow!(
                    "[mesh_worker] 连续 {} 轮 pending 集合未变化，疑似卡住；示例 geo_id: {:?}",
                    stalled_rounds + 1,
                    sample
                ));
            }
        }
    }
    
    println!(
        "[mesh_worker] mesh 生成完成: 共 {} 轮，累计处理 {} 个实例",
        round, total_processed
    );
    
    Ok(())
}

/// 基于 inst_relate 状态的布尔运算 Worker
///
/// 按批次扫描需要布尔运算的实例（catalog & 实例级），并复用现有
/// `booleans_meshes_in_db` 管道完成实际计算。
pub async fn run_boolean_worker(db_option: Arc<DbOption>, batch_size: usize) -> anyhow::Result<()> {
    let batch_size = batch_size.max(1);
    let replace_exist = db_option.is_replace_mesh();
    let mut round = 0usize;
    let mut total_processed = 0usize;
    let mut stalled_rounds = 0usize;
    let mut last_pending: Option<HashSet<RefnoEnum>> = None;
    let relation_targets = query_relation_targets_combined().await?;

    loop {
        let round_start = std::time::Instant::now();
        let cata_refnos = query_pending_cata_boolean(batch_size, replace_exist).await?;
        let inst_refnos =
            query_pending_inst_boolean(batch_size, replace_exist, &relation_targets).await?;

        let pending: HashSet<_> = cata_refnos
            .iter()
            .copied()
            .chain(inst_refnos.iter().copied())
            .collect();

        if pending.is_empty() {
            println!("[boolean_worker] 没有待处理布尔任务，退出");
            break;
        }

        if let Some(prev) = &last_pending {
            if *prev == pending {
                stalled_rounds += 1;
            } else {
                stalled_rounds = 0;
            }
        }
        last_pending = Some(pending.clone());

        round += 1;
        println!(
            "[boolean_worker] 轮次 {}: catalog={} inst={} batch_size={} replace_exist={}",
            round,
            cata_refnos.len(),
            inst_refnos.len(),
            batch_size,
            replace_exist
        );

        if !cata_refnos.is_empty() {
            let t = std::time::Instant::now();
            booleans_meshes_in_db(Some(db_option.clone()), &cata_refnos).await?;
            println!(
                "[boolean_worker] 轮次 {} catalog 布尔完成: {} 个，用时 {} ms",
                round,
                cata_refnos.len(),
                t.elapsed().as_millis()
            );
        }

        if !inst_refnos.is_empty() {
            let t = std::time::Instant::now();
            booleans_meshes_in_db(Some(db_option.clone()), &inst_refnos).await?;
            println!(
                "[boolean_worker] 轮次 {} inst 布尔完成: {} 个，用时 {} ms",
                round,
                inst_refnos.len(),
                t.elapsed().as_millis()
            );
        }

        let round_processed = cata_refnos.len() + inst_refnos.len();
        total_processed += round_processed;
        println!(
            "[boolean_worker] 轮次 {} 结束: 本轮 {} 个，累计 {} 个，用时 {} ms",
            round,
            round_processed,
            total_processed,
            round_start.elapsed().as_millis()
        );

        if stalled_rounds >= 3 {
            let sample: Vec<_> = pending.iter().take(5).cloned().collect();
            if replace_exist {
                println!(
                    "[boolean_worker] replace_exist=true 模式下检测到 stall，已完成 {} 个，退出",
                    total_processed
                );
                break;
            } else {
                return Err(anyhow!(
                    "[boolean_worker] 连续 {} 轮 pending 集合未变化，疑似卡住；示例 refno: {:?}",
                    stalled_rounds + 1,
                    sample
                ));
            }
        }
    }

    println!(
        "[boolean_worker] 布尔运算完成: 共 {} 轮，累计处理 {} 个实例",
        round, total_processed
    );

    Ok(())
}

///执行布尔运算的部分
pub async fn booleans_meshes_in_db(
    option: Option<Arc<DbOption>>,
    refnos: &[RefnoEnum],
) -> anyhow::Result<()> {
    if refnos.is_empty() {
        return Ok(());
    }
    let replace_exist = option
        .as_ref()
        .map(|x| x.is_replace_mesh())
        .unwrap_or(false);

    for chunk in refnos.chunks(100) {
        apply_cata_neg_boolean_manifold(chunk, replace_exist).await?;
        apply_insts_boolean_manifold(chunk, replace_exist).await?;
    }
    Ok(())
}

/// 处理网格并更新数据库
///
/// # 参数
/// * `option` - 数据库选项，包含网格路径和是否替换现有网格等配置
/// * `refnos` - 需要处理的引用号列表
///
/// # 返回值
/// * `anyhow::Result<()>` - 执行结果
pub async fn process_meshes_update_db(
    option: Option<Arc<DbOption>>,
    refnos: &[RefnoEnum],
) -> anyhow::Result<()> {
    if refnos.is_empty() {
        return Ok(());
    }
    let replace_exist = option
        .as_ref()
        .map(|x| x.is_replace_mesh())
        .unwrap_or(false);
    let time = std::time::Instant::now();
    let dir = option
        .as_ref()
        .map(|x| x.get_meshes_path())
        .unwrap_or("assets/meshes".into());
    let precision = Arc::new(
        option
            .as_ref()
            .map(|opt| opt.mesh_precision().clone())
            .unwrap_or_else(|| get_db_option().mesh_precision().clone()),
    );
    // dbg!(&target_refnos);
    // 生成模型文件
    gen_inst_meshes(&dir, &precision, &refnos, replace_exist, &[MeshFormat::PdmsMesh])
        .await
        .unwrap();
    println!(
        "gen_inst_meshes finished: {} ms",
        time.elapsed().as_millis()
    );
    let time = std::time::Instant::now();
    update_inst_relate_aabbs_by_refnos(&refnos, replace_exist)
        .await
        .unwrap();
    println!(
        "update_inst_relate_aabbs finished: {} ms",
        time.elapsed().as_millis()
    );

    apply_cata_neg_boolean_manifold(&refnos, replace_exist).await?;
    apply_insts_boolean_manifold(&refnos, replace_exist).await?;

    Ok(())
}

/// BRAN 专用的网格处理函数
/// 
/// BRAN 类型不需要：
/// - 查找子节点（没有 deep 遍历）
/// - 布尔运算（没有负实体计算）
/// 
/// # 参数
/// * `option` - 数据库选项
/// * `refnos` - BRAN 类型的 refno 列表
pub async fn process_meshes_bran(
    option: Option<Arc<DbOptionExt>>,
    refnos: &[RefnoEnum],
) -> anyhow::Result<()> {
    if refnos.is_empty() {
        return Ok(());
    }
    let replace_exist = option
        .as_ref()
        .map(|x| x.is_replace_mesh())
        .unwrap_or(false);
    let time = std::time::Instant::now();
    let dir = option
        .as_ref()
        .map(|x| {
            Path::new(x.inner.meshes_path.as_deref().unwrap_or("assets/meshes")).to_path_buf()
        })
        .unwrap_or_else(|| "assets/meshes".into());
    let precision = option
        .as_ref()
        .map(|opt| opt.inner.mesh_precision.clone())
        .unwrap_or_else(|| crate::options::get_db_option_ext().inner.mesh_precision.clone());
    let mesh_formats = option
        .as_ref()
        .map(|opt| opt.mesh_formats.clone())
        .unwrap_or_else(|| crate::options::get_db_option_ext().mesh_formats.clone());
    
    // 生成模型文件
    gen_inst_meshes(
        &dir,
        &precision,
        &refnos,
        replace_exist,
        &mesh_formats,
    )
    .await?;
    println!(
        "[BRAN] gen_inst_meshes finished: {} ms",
        time.elapsed().as_millis()
    );
    
    let time = std::time::Instant::now();
    update_inst_relate_aabbs_by_refnos(&refnos, replace_exist)
        .await
        .unwrap();
    println!(
        "[BRAN] update_inst_relate_aabbs finished: {} ms",
        time.elapsed().as_millis()
    );

    // BRAN 不需要布尔运算，直接返回
    Ok(())
}

/// 使用默认数据库选项更新深层模型网格数据
///
/// # 参数
///
/// * `refnos` - 参考号数组
///
/// # 返回值
///
/// 返回 `anyhow::Result<()>` 表示更新是否成功
pub async fn process_meshes_update_db_deep_default(refnos: &[RefnoEnum]) -> anyhow::Result<()> {
    let dboption = crate::options::get_db_option_ext();
    process_meshes_update_db_deep(&dboption, refnos).await
}

/// 使用指定数据库选项更新深层模型网格数据
///
/// # 参数
///
/// * `dboption` - 数据库选项
/// * `refnos` - 参考号数组
///
/// # 返回值
///
/// 返回 `anyhow::Result<()>` 表示更新是否成功
pub async fn process_meshes_update_db_deep(
    dboption: &DbOptionExt,
    refnos: &[RefnoEnum],
) -> anyhow::Result<()> {
    if !refnos.is_empty() {
        // 确保 mesh根目录存在
        let dir = Path::new(dboption.inner.meshes_path.as_deref().unwrap_or("assets/meshes")).to_path_buf();
        if !dir.exists() {
            std::fs::create_dir_all(&dir)?;
        }

        let precision = &dboption.inner.mesh_precision;
        let replace_exist = dboption.is_replace_mesh();
        let mesh_formats = &dboption.mesh_formats;
        println!("📊 更新模型结点数量: {}", refnos.len());
        let time = std::time::Instant::now();

        for (idx, &refno) in refnos.iter().enumerate() {
            println!(
                "\n🔄 [{}/{}] 处理模型结点: {}",
                idx + 1,
                refnos.len(),
                refno
            );

            // 使用 match 来捕获错误并继续处理其他 refno
            let result: anyhow::Result<()> = async {
                let mut target_visible_refnos = vec![];
                let mut update_refnos =
                    query_deep_visible_inst_refnos(refno).await.map_err(|e| {
                        eprintln!("⚠️  查询可见实例失败 (refno: {}): {}", refno, e);
                        e
                    })?;
                target_visible_refnos.extend(update_refnos.clone());

                let neg_refnos = query_deep_neg_inst_refnos(refno).await.map_err(|e| {
                    eprintln!("⚠️  查询负实例失败 (refno: {}): {}", refno, e);
                    e
                })?;
                update_refnos.extend(neg_refnos.clone());

                if update_refnos.is_empty() {
                    println!("跳过空的 update_refnos for refno: {}", refno);
                    return Ok(());
                }

                println!("  📦 实际需要更新模型结点数量: {}", update_refnos.len());

                if dboption.gen_mesh {
                    // 生成模型文件
                    let mesh_time = std::time::Instant::now();
                    gen_inst_meshes(
                        &dir,
                        precision,
                        &update_refnos,
                        replace_exist,
                        mesh_formats,
                    )
                    .await
                    .map_err(|e| {
                        eprintln!("❌ gen_inst_meshes 失败 (refno: {}): {}", refno, e);
                        anyhow::anyhow!("生成网格失败 for refno {}: {}", refno, e)
                    })?;
                    debug_model!(
                        "  ✅ gen_inst_meshes 完成: {} ms",
                        mesh_time.elapsed().as_millis()
                    );

                    let aabb_time = std::time::Instant::now();
                    // 更新aabb 到inst relate，geo relate
                    update_inst_relate_aabbs_by_refnos(&update_refnos, replace_exist)
                        .await
                        .map_err(|e| {
                            eprintln!(
                                "❌ update_inst_relate_aabbs_by_refnos 失败 (refno: {}): {}",
                                refno, e
                            );
                            anyhow::anyhow!("更新 AABB 失败 for refno {}: {}", refno, e)
                        })?;
                    debug_model!(
                        "  ✅ update_inst_relate_aabbs 完成: {} ms",
                        aabb_time.elapsed().as_millis()
                    );
                }

                if target_visible_refnos.is_empty() {
                    println!("跳过空的 target_visible_refnos for refno: {}", refno);
                    return Ok(());
                }

                if dboption.apply_boolean_operation {
                    let bool_time = std::time::Instant::now();
                    
                    // 过滤掉 BRAN 类型，BRAN 不需要布尔运算
                    let boolean_refnos = {
                        let refno_keys: Vec<String> = target_visible_refnos.iter().map(|r| r.to_pe_key()).collect();
                        if refno_keys.is_empty() {
                            Vec::new()
                        } else {
                            let refno_keys = refno_keys.join(",");
                            let sql = format!(
                                "SELECT value id FROM [{refno_keys}] WHERE noun != 'BRAN'"
                            );
                            SUL_DB.query_take::<Vec<RefnoEnum>>(&sql, 0).await.unwrap_or_else(|e| {
                                eprintln!("SQL error in CSG mesh boolean query: {}", e);
                                Vec::new()
                            })
                        }
                    };
                    
                    if boolean_refnos.is_empty() {
                        debug_model!("  跳过布尔运算：全部为 BRAN 类型");
                    } else {
                        // 生成元件库内部几何体的负实体运算（catalog-level: 同一元件库内的正负几何体布尔）
                        apply_cata_neg_boolean_manifold(&boolean_refnos, replace_exist)
                            .await
                            .map_err(|e| {
                                eprintln!(
                                    "❌ apply_cata_neg_boolean_manifold 失败 (refno: {}): {}",
                                    refno, e
                                );
                                e
                            })?;
                        // 实例级布尔运算（instance-level: 通过 ngmr 关系切割的正实体）
                        // 传入正实体列表，函数内部会查询它们关联的负实体
                        apply_insts_boolean_manifold(&boolean_refnos, replace_exist)
                            .await
                            .map_err(|e| {
                                eprintln!(
                                    "❌ apply_insts_boolean_manifold 失败 (refno: {}): {}",
                                    refno, e
                                );
                                e
                            })?;
                        debug_model!("  ✅ 布尔运算完成: {} ms", bool_time.elapsed().as_millis());
                    }
                }

                Ok(())
            }
            .await;

            // 如果处理失败，打印错误但继续处理下一个 refno
            if let Err(e) = result {
                eprintln!("❌ 处理 refno {} 失败: {}", refno, e);
                eprintln!("   继续处理下一个节点...\n");
            } else {
                println!("✅ 成功处理 refno: {}", refno);
            }
        }
        println!("\n⏱️  总耗时: {} ms", time.elapsed().as_millis());
    }
    Ok(())
}

/// 直接基于 inst_geo id 列表生成网格数据
///
/// 与 `gen_inst_meshes` 不同，此函数直接接收 `inst_geo` 的 RecordId 列表，
/// 无需通过 refno 查询 inst_relate -> geo_relate 链条。
///
/// # 参数
///
/// * `dir` - 模型文件目录路径
/// * `precision` - 网格精度设置
/// * `geo_ids` - inst_geo 的 RecordId 列表
/// * `mesh_formats` - 输出的网格格式
///
/// # 返回值
///
/// 返回 `anyhow::Result<()>` 表示生成是否成功
pub async fn gen_inst_meshes_by_geo_ids(
    dir: &Path,
    precision: &MeshPrecisionSettings,
    geo_ids: &[RecordId],
    mesh_formats: &[MeshFormat],
) -> anyhow::Result<()> {
    if geo_ids.is_empty() {
        return Ok(());
    }
    
    // 创建 LOD 子目录
    let lod_dir = dir.join(format!("lod_{:?}", precision.default_lod));
    if !lod_dir.exists() {
        std::fs::create_dir_all(&lod_dir)?;
    }
    
    // 构建查询的 id 列表
    let ids_str = geo_ids.iter().map(|id| id.to_raw()).join(",");
    
    // 查询 inst_geo 的参数
    let sql = format!(
        "SELECT id, param, unit_flag ?? false as unit_flag FROM [{}] WHERE param != NONE",
        ids_str
    );
    
    let mut response = SUL_DB.query(&sql).await?;
    let geo_params: Vec<QueryGeoParam> = response.take(0).unwrap_or_default();
    
    if geo_params.is_empty() {
        debug_model_debug!("[gen_inst_meshes_by_geo_ids] 没有找到有效的几何参数");
        return Ok(());
    }
    
    let aabb_map: Arc<DashMap<String, Aabb>> = Arc::new(DashMap::new());
    let pts_json_map: Arc<DashMap<u64, String>> = Arc::new(DashMap::new());
    let inst_aabb_map: Arc<DashMap<String, Aabb>> = Arc::new(DashMap::new());
    let mut update_sql = String::new();
    
    for g in geo_params {
        let geo_type_name = g.param.type_name();
        let profile = precision.profile_for_geo(geo_type_name);
        let non_scalable_geo = precision.is_non_scalable_geo(geo_type_name);
        let mesh_id = g.id.to_mesh_id();
        
        // 不需要 refno
        match generate_csg_mesh(&g.param, &profile.csg_settings, non_scalable_geo, None) {
            Some(csg_mesh) => {
                let mesh_filename = format!("{}_{:?}", mesh_id, precision.default_lod);
                
                if let Err(e) = handle_csg_mesh(
                    &lod_dir,
                    &mesh_id,
                    &mesh_filename,
                    csg_mesh,
                    &aabb_map,
                    &pts_json_map,
                    &inst_aabb_map,
                    &mut update_sql,
                    mesh_formats,
                )
                .await
                {
                    debug_model_warn!("CSG mesh 生成失败 for {}: {}", mesh_id, e);
                    // 设置 bad=true 和 meshed=true 避免重复处理
                    update_sql.push_str(&format!("UPDATE inst_geo:⟨{}⟩ SET bad=true, meshed=true;", mesh_id));
                }
            }
            None => {
                debug_model_warn!("CSG mesh 返回 None for {}", mesh_id);
                // 设置 bad=true 和 meshed=true 避免重复处理
                update_sql.push_str(&format!("UPDATE inst_geo:⟨{}⟩ SET bad=true, meshed=true;", mesh_id));
            }
        }
    }
    
    // 执行批量更新
    if !update_sql.is_empty() {
        println!("[gen_inst_meshes_by_geo_ids] 执行 update_sql ({} bytes)", update_sql.len());
        match SUL_DB.query(&update_sql).await {
            Ok(_) => println!("[gen_inst_meshes_by_geo_ids] update_sql 执行成功"),
            Err(e) => eprintln!("[gen_inst_meshes_by_geo_ids] 更新数据库失败: {}", e),
        }
    } else {
        println!("[gen_inst_meshes_by_geo_ids] update_sql 为空，没有需要更新的记录");
    }
    
    // 保存 aabb 和 pts 数据
    utils::save_pts_to_surreal(&pts_json_map).await;
    utils::save_aabb_to_surreal(&aabb_map).await;
    
    Ok(())
}

///
/// # 参数
///
/// * `refnos` - 参考号数组
/// * `replace_exist` - 是否替换已存在的网格数据
/// * `dir` - 模型文件目录路径
///
/// # 返回值
///
/// 返回 `anyhow::Result<()>` 表示生成是否成功
///
/// # 侧效与说明
/// - 并发分批查询 inst_geo 参数并生成网格
/// - 将网格序列化保存到磁盘（dir/*.mesh）
/// - 回写 SurrealDB: inst_geo.meshed/aabb/pts 字段，错误则标记 bad=true
/// - 更新内存缓存 EXIST_MESH_GEO_HASHES；最后批量保存 aabb/pts 到 SurrealDB
pub async fn gen_inst_meshes(
    dir: &Path,
    precision: &MeshPrecisionSettings,
    refnos: &[RefnoEnum],
    replace_exist: bool,
    mesh_formats: &[MeshFormat],
) -> anyhow::Result<()> {
    debug_model_debug!(
        "gen_inst_meshes start: refnos={}, replace_exist={}, dir={}",
        refnos.len(),
        replace_exist,
        dir.display()
    );
    // 每批并发处理的 inst_geo 数量上限，控制单批任务规模
    const PAGE_NUM: usize = 100;
    // 计数/调试用途（目前未外显）
    let mut i = 0;

    // 根据 LOD 级别创建子目录（如果传入的 dir 不是已经包含 lod_ 前缀）
    let dir = if let Some(dir_name) = dir.file_name() {
        let dir_str = dir_name.to_string_lossy();
        // 如果目录名已经是 lod_XX 格式，直接使用
        if dir_str.starts_with("lod_") {
            dir.to_path_buf()
        } else {
            // 否则创建 LOD 子目录
            let lod_dir = dir.join(format!("lod_{:?}", precision.default_lod));
            if !lod_dir.exists() {
                std::fs::create_dir_all(&lod_dir)?;
            }
            lod_dir
        }
    } else {
        // 如果无法获取目录名，创建 LOD 子目录
        let lod_dir = dir.join(format!("lod_{:?}", precision.default_lod));
        if !lod_dir.exists() {
            std::fs::create_dir_all(&lod_dir)?;
        }
        lod_dir
    };

    // 使用结构化的 query_inst_geo_ids API 查询几何 ID
    // 根据 replace_exist 决定是否跳过已生成或异常的几何：
    // - replace_exist=true：不过滤 aabb/meshed，允许覆盖，但仍过滤 bad
    // - replace_exist=false：仅选择 aabb 为空、未网格化且非 bad 的几何
    // 返回包含 geo_id 和 has_neg_relate 字段的结构化结果
    let inst_geo_ids = match query_inst_geo_ids(refnos, replace_exist).await {
        Ok(ids) => ids,
        Err(e) => {
            debug_model_debug!(
                "query_inst_geo_ids failed for refnos={:?}: {}. This is normal for objects without geometry (e.g., FLOOR, or pipe tubing).",
                refnos,
                e
            );
            return Ok(());
        }
    };
    debug_model_debug!(
        "gen_inst_meshes fetched inst_geo_ids: {}",
        inst_geo_ids.len()
    );
    // println!("inst_geo_ids: {:?}", &inst_geo_ids);
    // 无可处理对象则直接返回
    if inst_geo_ids.is_empty() {
        debug_model_debug!(
            "[WARN] gen_inst_meshes: inst_geo_ids empty for refnos={:?}",
            refnos
        );
        return Ok(());
    }
    let mut tasks = vec![];
    // 线程安全缓存：aabb_map 用于累积 aabb；pts_json_map 用于存储端点 JSON（去重）
    let aabb_map = Arc::new(DashMap::new());
    let pts_json_map = Arc::new(DashMap::new());
    let inst_aabb_map = Arc::new(DashMap::new());

    // 分批并发处理 inst_geo
    for (chunk_idx, chunk) in inst_geo_ids.chunks(PAGE_NUM).enumerate() {
        debug_model_debug!(
            "gen_inst_meshes chunk {} processing {} inst_geo ids",
            chunk_idx,
            chunk.len()
        );
        // 将本批次 inst_geo id 合并为 SurrealDB in 子查询集合，并构建 refno 映射
        let chunk_records: Vec<(String, Option<RefnoEnum>)> = chunk
            .iter()
            .map(|result| (result.geo_id.to_raw(), result.refno.clone()))
            .collect();
        let ids = chunk_records.iter().map(|(raw, _)| raw.as_str()).join(",");
        let chunk_refno_map: HashMap<String, Option<RefnoEnum>> =
            chunk_records.into_iter().collect();
        // 克隆所需上下文到异步任务中
        let dir = dir.clone();
        let aabb_map = aabb_map.clone();
        let pts_json_map = pts_json_map.clone();
        let precision = Arc::new(precision.clone()); // Clone Arc<MeshPrecisionSettings>
        let inst_aabb_map = inst_aabb_map.clone();
        let chunk_refno_map = chunk_refno_map.clone();
        let mesh_formats = mesh_formats.to_vec();
        // 每批一个异步任务：查询参数 -> CSG 网格化 -> 回写
        let task = tokio::spawn(async move {
            // 查询本批所有 inst_geo 的参数
            let sql = format!(
                "select id, param, unit_flag ?? false as unit_flag from [{}] where param != NONE",
                ids
            );
            match SUL_DB.query(&sql).await {
                Ok(mut response) => {
                    let result: Vec<QueryGeoParam> = response.take(0).unwrap();
                    i += 1;
                    let mut update_sql = String::new();
                    // 遍历每个几何参数并使用 CSG 生成网格
                    for g in result {
                        debug_model_debug!("gen mesh param: {:?}", &g.param);
                        let geo_type_name = g.param.type_name();
                        let profile = precision.profile_for_geo(geo_type_name);
                        let non_scalable_geo = precision.is_non_scalable_geo(geo_type_name);
                        let mesh_id = g.id.to_mesh_id();
                        let geo_raw = g.id.to_raw();
                        let refno_for_mesh: Option<RefnoEnum> = chunk_refno_map
                            .get(&geo_raw)
                            .cloned()
                            .flatten();

                        // 统一使用 CSG 方式生成网格
                        match generate_csg_mesh(
                            &g.param,
                            &profile.csg_settings,
                            non_scalable_geo,
                            refno_for_mesh,
                        ) {
                            Some(csg_mesh) => {
                                // 构造带 LOD 后缀的文件名，保持所有 LOD 级别命名一致
                                let mesh_filename =
                                    format!("{}_{:?}", mesh_id, precision.default_lod);

                                if let Err(e) = handle_csg_mesh(
                                    &dir,
                                    &mesh_id,
                                    &mesh_filename,
                                    csg_mesh,
                                    &aabb_map,
                                    &pts_json_map,
                                    &inst_aabb_map,
                                    &mut update_sql,
                                    &mesh_formats,
                                )
                                .await
                                {
                                    debug_model_warn!(
                                        "CSG mesh generation failed for {}: {}",
                                        mesh_id,
                                        e
                                    );
                                    // 标记 bad，避免后续重复尝试
                                    update_sql.push_str(&format!(
                                        "update inst_geo:⟨{}⟩ set bad=true;",
                                        mesh_id
                                    ));
                                }
                            }
                            None => {
                                // CSG 生成失败
                                let failed_refnos = aios_core::query_refnos_by_geo_hash(&mesh_id)
                                    .await
                                    .unwrap_or_default();
                                debug_model_warn!(
                                    "{:?} CSG mesh generation not supported for type: {}",
                                    failed_refnos,
                                    geo_type_name
                                );
                                // 标记 bad，避免后续重复尝试
                                update_sql.push_str(&format!(
                                    "update inst_geo:⟨{}⟩ set bad=true;",
                                    mesh_id
                                ));
                            }
                        }
                    }
                    if !update_sql.is_empty() {
                        // 批量回写 SurrealDB（使用一个语句拼接多条 update）
                        println!("准备执行批量更新 SQL，长度: {}", update_sql.len());
                        match SUL_DB.query(&update_sql).await {
                            Ok(_) => {
                                println!("✅ 批量更新成功");
                            }
                            Err(e) => {
                                let ctx = crate::fast_model::error_macros::ErrorContext {
                                    location: format!("{}:{}", file!(), line!()),
                                    error_msg: e.to_string(),
                                    extra_info: vec![(
                                        "📄 SQL (前500字符)".to_string(),
                                        update_sql.chars().take(500).collect::<String>(),
                                    )],
                                };
                                ctx.print("gen_inst_meshes 批量更新失败");
                                init_save_database_error(
                                    &update_sql,
                                    &std::panic::Location::caller().to_string(),
                                );
                            }
                        }
                    }
                }
                // 本批次查询失败：记录错误并继续其他批次
                Err(e) => {
                    init_query_error(&sql, e, &std::panic::Location::caller().to_string());
                }
            }
        });
        tasks.push(task);
    }

    // 等待所有批次任务完成
    match futures::future::try_join_all(tasks).await {
        Ok(_) => {}
        Err(e) => {
            dbg!(e);
        }
    }

    // 用新生成的 aabb 更新内存缓存，避免重复计算
    for result in inst_geo_ids {
        let h = result.geo_id.to_mesh_id();
        if let Some(aabb) = inst_aabb_map.get(&h) {
            EXIST_MESH_GEO_HASHES.insert(h.clone(), *aabb);
        }
    }

    // 批量持久化点集与 aabb 实体
    utils::save_pts_to_surreal(&pts_json_map).await;
    utils::save_aabb_to_surreal(&aabb_map).await;

    Ok(())
}

async fn handle_csg_mesh(
    dir: &Path,
    inst_key: &str,
    mesh_id: &str,
    mut generated: GeneratedMesh,
    aabb_map: &Arc<DashMap<String, Aabb>>,
    pts_json_map: &Arc<DashMap<u64, String>>,
    inst_aabb_map: &Arc<DashMap<String, Aabb>>,
    update_sql: &mut String,
    mesh_formats: &[MeshFormat],
) -> anyhow::Result<()> {
    if generated.mesh.aabb.is_none() {
        generated.mesh.aabb = generated.aabb;
    }
    let mesh_aabb = generated
        .mesh
        .aabb
        .ok_or_else(|| anyhow!("CSG mesh 缺少有效的 AABB"))?;

    let pt_refs = derive_csg_points(&generated.mesh, pts_json_map);

    let mesh_base_path = dir.join(mesh_id);
    
    // 强制生成 GLB
    let glb_path = mesh_base_path.with_extension("glb");
    if let Err(e) = export_single_mesh_to_glb(&generated.mesh, &glb_path) {
        debug_model_warn!("   ⚠️ 生成 GLB 失败: {} - {}", mesh_id, e);
    }

    if mesh_formats.contains(&MeshFormat::Obj) {
        let obj_path = mesh_base_path.with_extension("obj");
        if let Err(e) = generated.mesh.export_obj(false, obj_path.to_str().unwrap()) {
            debug_model_warn!("   ⚠️ 生成 OBJ 失败: {} - {}", mesh_id, e);
        }
    }

    let aabb_hash = gen_bytes_hash(&mesh_aabb);
    aabb_map.entry(aabb_hash.to_string()).or_insert(mesh_aabb);
    if !EXIST_MESH_GEO_HASHES.contains_key(mesh_id) {
        EXIST_MESH_GEO_HASHES.insert(mesh_id.to_string(), mesh_aabb);
    }
    inst_aabb_map.insert(mesh_id.to_string(), mesh_aabb);

    update_sql.push_str(&format!(
        "update inst_geo:⟨{}⟩ set meshed = true, aabb = aabb:⟨{}⟩, pts=[{}];",
        inst_key,
        aabb_hash,
        pt_refs.join(","),
    ));

    Ok(())
}

fn derive_csg_points(mesh: &PlantMesh, pts_json_map: &Arc<DashMap<u64, String>>) -> Vec<String> {
    let mut hashes = HashSet::new();
    for vertex in &mesh.vertices {
        let rs_vec = RsVec3(*vertex);
        let hash = rs_vec.gen_hash();
        if hashes.insert(hash) && !pts_json_map.contains_key(&hash) {
            if let Ok(serialized) = serde_json::to_string(&rs_vec) {
                pts_json_map.insert(hash, serialized);
            }
        }
    }
    hashes
        .into_iter()
        .map(|hash| format!("vec3:⟨{}⟩", hash))
        .collect()
}

/// 更新实例关联的包围盒数据（带 SQLite 空间索引优化）
///
/// 这是 aios_core::update_inst_relate_aabbs_by_refnos 的增强版本，
/// 集成了 SQLite 空间索引支持，用于读取和写入 AABB 缓存。
///
/// # 参数
///
/// * `refnos` - 参考号数组
/// * `replace_exist` - 是否替换已存在的包围盒数据
///
/// # 返回值
///
/// 返回 `anyhow::Result<()>` 表示更新是否成功
///
/// # 优化说明
///
/// 如果启用了 `sqlite-index` feature：
/// - 优先从 SQLite 空间索引读取已缓存的 AABB
/// - 计算新的 AABB 后写入 SQLite 空间索引
/// - 减少重复计算，提升性能
pub async fn update_inst_relate_aabbs_by_refnos(
    refnos: &[RefnoEnum],
    replace_exist: bool,
) -> anyhow::Result<()> {
    const CHUNK: usize = 100;

    let aabb_map = DashMap::new();
    let inst_aabb_map = DashMap::new();

    for chunk in refnos.chunks(CHUNK) {
        if chunk.is_empty() {
            continue;
        }

        // 非替换模式下，跳过已经存在新表记录的 refno
        let target_refnos = if replace_exist {
            chunk.to_vec()
        } else {
            filter_missing_inst_aabb(chunk).await?
        };

        if target_refnos.is_empty() {
            continue;
        }

        // inst_relate.aabb 不再作为过滤条件，因此查询时强制 replace_exist=true
        let inst_keys = get_inst_relate_keys(&target_refnos);
        let result = query_aabb_params(&inst_keys, true).await?;
        println!(
            "[aabb] 查询 inst_relate keys {} -> {} 个候选",
            target_refnos.len(),
            result.len()
        );

        for r in result {
            // 计算合并后的 AABB
            let mut aabb = Aabb::new_invalid();
            for g in &r.geo_aabbs {
                let t = r.world_trans * &g.trans;
                let tmp_aabb = g.aabb.scaled(&t.scale.into());
                let tmp_aabb = tmp_aabb.transform_by(&Isometry {
                    rotation: t.rotation.into(),
                    translation: t.translation.into(),
                });
                aabb.merge(&tmp_aabb);
            }

            // 过滤无效 AABB
            let extent = aabb.extents().magnitude();
            if extent.is_nan() || extent.is_infinite() {
                continue;
            }

            let aabb_hash = gen_bytes_hash(&aabb).to_string();
            aabb_map.entry(aabb_hash.clone()).or_insert(aabb);

            let refno = r.refno();
            inst_aabb_map.insert(refno.clone(), aabb_hash.clone());
        }
    }

    // 批量保存 AABB 到 aabb 表
    utils::save_aabb_to_surreal(&aabb_map).await;

    if inst_aabb_map.is_empty() {
        println!(
            "[aabb] 未生成任何实例级 AABB（refnos={}，replace_exist={}），跳过 inst_relate_aabb 写入",
            refnos.len(),
            replace_exist
        );
        return Ok(());
    }

    // 将实例级 AABB 写入关系表 inst_relate_aabb（in=pe, out=aabb）
    utils::save_inst_relate_aabb(&inst_aabb_map, "gen_mesh").await;
    println!(
        "[aabb] inst_relate_aabb 写入 {} 条记录 (replace_exist={})",
        inst_aabb_map.len(),
        replace_exist
    );

    Ok(())
}

/// 查询所有 inst_relate 的 refno（仅 world_trans 存在的实例）
pub async fn fetch_inst_relate_refnos() -> anyhow::Result<Vec<RefnoEnum>> {
    let sql = "SELECT value in.id FROM inst_relate WHERE world_trans.d != none";
    let mut resp = SUL_DB.query(sql).await?;
    let refnos: Vec<RefnoEnum> = resp.take(0)?;
    Ok(refnos)
}

async fn filter_missing_inst_aabb(refnos: &[RefnoEnum]) -> anyhow::Result<Vec<RefnoEnum>> {
    if refnos.is_empty() {
        return Ok(Vec::new());
    }

    let pe_keys: Vec<String> = refnos.iter().map(|r| r.to_pe_key()).collect();
    if pe_keys.is_empty() {
        return Ok(Vec::new());
    }

    let sql = format!(
        "SELECT value in FROM inst_relate_aabb WHERE in IN [{}]",
        pe_keys.join(",")
    );

    let existing: Vec<RefnoEnum> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
    let existing: HashSet<RefnoEnum> = existing.into_iter().collect();

    let missing = refnos
        .iter()
        .cloned()
        .filter(|r| !existing.contains(r))
        .collect();

    Ok(missing)
}

// Database query structures are now imported from aios_core::query_structs

// ========================
// Scene Tree 集成（替代 inst_relate_aabb）
// ========================

/// 过滤未在 scene_tree 中标记为已生成的节点
///
/// 替代 `filter_missing_inst_aabb`，使用 scene_tree 的 `generated` 字段
#[cfg(feature = "gen_model")]
pub async fn filter_missing_scene_node(refnos: &[RefnoEnum]) -> anyhow::Result<Vec<RefnoEnum>> {
    if refnos.is_empty() {
        return Ok(Vec::new());
    }

    // 查询已生成的节点
    let existing = crate::scene_tree::query_generated_refnos(refnos).await?;
    let existing: HashSet<RefnoEnum> = existing.into_iter().collect();

    // 返回未生成的节点
    let missing = refnos
        .iter()
        .cloned()
        .filter(|r| !existing.contains(r))
        .collect();

    Ok(missing)
}

/// 使用 scene_tree 更新 AABB 数据
///
/// 替代 `update_inst_relate_aabbs_by_refnos`，写入 scene_node 表
#[cfg(feature = "gen_model")]
pub async fn update_scene_node_aabbs_by_refnos(
    refnos: &[RefnoEnum],
    _replace_exist: bool,
) -> anyhow::Result<()> {
    if refnos.is_empty() {
        return Ok(());
    }

    const CHUNK: usize = 100;

    #[derive(serde::Deserialize)]
    struct InstAabbRow {
        #[serde(rename = "in")]
        refno: String,
        aabb: String,
    }

    let inst_aabb_map = DashMap::new();

    for chunk in refnos.chunks(CHUNK) {
        if chunk.is_empty() {
            continue;
        }

        // 获取 inst_relate.aabb 数据
        let inst_keys = get_inst_relate_keys(chunk);
        let result = query_aabb_params(&inst_keys, true).await?;

        for r in result {
            // 计算合并后的 AABB
            let mut aabb = Aabb::new_invalid();
            for g in &r.geo_aabbs {
                let t = r.world_trans * &g.trans;
                let tmp_aabb = g.aabb.scaled(&t.scale.into());
                let tmp_aabb = tmp_aabb.transform_by(&Isometry {
                    rotation: t.rotation.into(),
                    translation: t.translation.into(),
                });
                aabb.merge(&tmp_aabb);
            }

            // 过滤无效 AABB
            let extent = aabb.extents().magnitude();
            if extent.is_nan() || extent.is_infinite() {
                continue;
            }

            let aabb_hash = gen_bytes_hash(&aabb);
            inst_aabb_map.insert(r.refno, aabb_hash.to_string());
        }
    }

    // 使用 scene_tree 模块更新
    crate::scene_tree::update_scene_node_aabb(&inst_aabb_map).await?;

    println!(
        "[scene_tree] 更新 {} 个节点的 AABB",
        inst_aabb_map.len()
    );

    Ok(())
}
