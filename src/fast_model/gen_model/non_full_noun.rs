//! 非 Full Noun 模式的几何体生成
//!
//! 本模块包含从 gen_model_old.rs 迁移的非 Full Noun 模式代码：
//! - gen_geos_data_by_dbnum: 按数据库编号生成几何体
//! - gen_geos_data: 主要的几何体数据生成函数
//! - process_gen_geos_data_chunks: 分块处理辅助函数
//!
//! 这些函数用于：
//! - 增量更新模式
//! - 手动 refno 模式
//! - 调试模式（如 `--debug-model 25688_36110`）
//! - 按数据库编号的全量生成
//!
//! ## 重要说明：dbnum 解析
//!
//! 本模块在处理 refno 时，使用 `TreeIndexManager::resolve_dbnum_for_refno()`
//! 来正确解析 dbnum。这确保了像 `25688_36110` 这样的 refno 不会被错误地
//! 解析为 dbnum=25688。
//!
//! 注意：Full Noun 模式已迁移到 full_noun_mode.rs

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use crate::options::DbOptionExt;
use anyhow::Result;
use std::time::Instant;
use dashmap::DashMap;
use futures::stream::{FuturesUnordered, StreamExt};
use glam::Vec3;

use aios_core::geometry::ShapeInstancesData;
use aios_core::options::DbOption;
use aios_core::pdms_types::{
    CataHashRefnoKV, GNERAL_LOOP_OWNER_NOUN_NAMES, GNERAL_PRIM_NOUN_NAMES, USE_CATE_NOUN_NAMES,
};
use aios_core::{RefnoEnum, SUL_DB};

use crate::data_interface::increment_record::IncrGeoUpdateLog;
use crate::fast_model::query_provider::{query_by_type, query_multi_descendants};
use crate::fast_model::{cata_model, debug_model_debug, debug_model_trace, loop_model, prim_model};

use super::utilities::{build_cata_hash_map_from_tree, is_e3d_debug_enabled};

use super::models::DbModelInstRefnos;

/// 按数据库编号生成几何体数据
///
/// # 参数
/// * `dbnum` - 数据库编号
/// * `db_option_arc` - 数据库配置（Arc 包装）
/// * `sender` - 几何体数据发送通道
/// * `target_sesno` - 目标会话号（用于历史查询）
///
/// # 返回
/// 返回分类后的模型实例引用号集合
pub async fn gen_geos_data_by_dbnum(
    dbnum: u32,
    db_option_arc: Arc<DbOptionExt>,
    sender: flume::Sender<ShapeInstancesData>,
    target_sesno: Option<u32>,
) -> Result<DbModelInstRefnos> {
    let gen_history = db_option_arc.is_gen_history_model();

    // 判断有空的层级，不用去生成
    let zones = if let Some(sesno) = target_sesno {
        // 使用历史查询
        query_by_type(&["ZONE"], dbnum as i32, Some(true))
            .await
            .unwrap_or_default()
    } else {
        // 使用当前数据查询
        query_by_type(&["ZONE"], dbnum as i32, Some(true))
            .await
            .unwrap_or_default()
    };
    if zones.is_empty() {
        return Ok(Default::default());
    }

    let d_types = db_option_arc.debug_refno_types.clone();
    let mut gen_cata_flag = d_types.iter().any(|x| x == "CATA");
    let mut gen_loop_flag = d_types.iter().any(|x| x == "LOOP");
    let mut gen_prim_flag = d_types.iter().any(|x| x == "PRIM");
    let gen_model = db_option_arc.gen_model;
    let test_refno = db_option_arc.get_test_refno();

    // Step 1、提前缓存 ploo，得到对齐方式的偏移
    let loop_sjus_map = DashMap::new();
    {
        // 查找到子节点的所有 PLOO 类型
        let target_ploo_refnos = query_by_type(&["PLOO"], dbnum as i32, Some(true))
            .await
            .unwrap_or_default();
        #[cfg(debug_assertions)]
        if !target_ploo_refnos.is_empty() {}
        if gen_model {
            for r in target_ploo_refnos.chunks(200) {
                let sql = format!(
                    "select value [OWNER, HEIG, SJUS] from [{}] where SJUS!=0",
                    r.iter()
                        .map(|x| x.to_table_key("PLOO"))
                        .collect::<Vec<_>>()
                        .join(",")
                );
                let mut response = SUL_DB.query(sql).await?;
                let tuples: Vec<(RefnoEnum, f32, String)> = response.take(0)?;
                for (owner, height, sjus) in tuples {
                    let off_z =
                        crate::fast_model::gen_model::cate_helpers::cal_sjus_value(&sjus, height);
                    // 对齐方式的距离，应该存储下来，子节点要与其保持一致的偏移
                    // 插入方向和偏移距离
                    loop_sjus_map.insert(owner, (Vec3::NEG_Z * off_z, height));
                }
            }
        }
    }
    let loop_sjus_map_arc = Arc::new(loop_sjus_map);

    // Step 2、按类目先逐个分好类的参考号集合
    // 2.1 管道或者支吊架的分类
    let target_bran_hanger_refnos =
        Arc::new(query_by_type(&["BRAN", "HANG"], dbnum as i32, None).await?);

    // 打印管道/支吊架的使用数量
    if !target_bran_hanger_refnos.is_empty() && gen_cata_flag && gen_model {
        // 查询出 branch 和 branch 下的子节点
        let mut branch_refnos_map = DashMap::new();
        let mut bran_comp_eles = HashSet::new();
        for &refno in target_bran_hanger_refnos.as_slice() {
            // 使用新的泛型函数接口
            let children = aios_core::collect_children_elements(refno, &[])
                .await
                .unwrap_or_default();
            bran_comp_eles.extend(children.iter().map(|x| x.refno));
            // 求出元件对应的 outside bore
            branch_refnos_map.insert(refno, children);
        }

        let target_bran_reuse_cata_map: DashMap<String, CataHashRefnoKV> = {
            let map = build_cata_hash_map_from_tree(target_bran_hanger_refnos.as_slice())
                .await
                .unwrap_or_default();
            if let Some(t_refno) = test_refno {
                if bran_comp_eles.contains(&t_refno) {
                    for kv in &map {
                        if kv.value().group_refnos.contains(&t_refno) {
                            debug_model_trace!("kv.value(): {:?}", kv.value());
                        }
                    }
                }
            }
            map
        };

        // 元件库的模型计算
        // bran，hanger 下需要重用的模型
        if gen_model && (!target_bran_reuse_cata_map.is_empty() || !branch_refnos_map.is_empty()) {
            let sjus_map_clone = loop_sjus_map_arc.clone();
            let db_option = db_option_arc.clone();
            let sender = sender.clone();
            let start_time = Instant::now();
            cata_model::gen_cata_geos(
                db_option,
                Arc::new(target_bran_reuse_cata_map),
                Arc::new(branch_refnos_map),
                sjus_map_clone,
                sender,
            )
            .await
            .unwrap();
        }
    }
    let mut use_cate_refnos = vec![];
    for cate_names in USE_CATE_NOUN_NAMES.chunks(4) {
        let refnos = query_by_type(cate_names, dbnum as i32, None).await?;
        if refnos.is_empty() {
            continue;
        }
        use_cate_refnos.extend(refnos.clone());
        let cur_cate_refnos = Arc::new(refnos);

        // 查询单个使用元件库的数量
        let target_single_cata_map = {
            // 要过滤掉 owner 是 BRAN 和 HANG 的
            let map = build_cata_hash_map_from_tree(cur_cate_refnos.as_slice())
                .await
                .unwrap_or_default();
            map
        };
        debug_model_trace!(
            "target_single_cata_map.len(): {}",
            target_single_cata_map.len()
        );

        if gen_model && gen_cata_flag && !target_single_cata_map.is_empty() {
            let sjus_map_clone = loop_sjus_map_arc.clone();
            let db_option = db_option_arc.clone();
            let sender = sender.clone();
            let start_time = Instant::now();
            cata_model::gen_cata_geos(
                db_option,
                Arc::new(target_single_cata_map),
                Arc::new(Default::default()),
                sjus_map_clone,
                sender,
            )
            .await
            .unwrap();
        }
    }

    let target_loop_owner_refnos = Arc::new(
        query_by_type(&GNERAL_LOOP_OWNER_NOUN_NAMES, dbnum as i32, Some(true))
            .await
            .unwrap_or_default(),
    );
    if gen_model && gen_loop_flag && !target_loop_owner_refnos.is_empty() {
        let sjus_map_clone = loop_sjus_map_arc.clone();
        let sender = sender.clone();
        let db_option = db_option_arc.clone();
        let target_loop_owner_refnos_arc = target_loop_owner_refnos.clone();
        loop_model::gen_loop_geos(
            db_option,
            &target_loop_owner_refnos_arc,
            sjus_map_clone,
            sender,
        )
        .await
        .unwrap();
    }

    let target_prim_refnos = Arc::new(
        query_by_type(&GNERAL_PRIM_NOUN_NAMES, dbnum as i32, None)
            .await
            .unwrap_or_default(),
    );

    // 基本元件的生成
    if gen_model && gen_prim_flag && !target_prim_refnos.is_empty() {
        // 基本体模型的生成
        let db_option = db_option_arc.clone();
        let sender = sender.clone();
        let target_prim_refnos_arc = target_prim_refnos.clone();
        prim_model::gen_prim_geos(db_option, target_prim_refnos_arc.as_slice(), sender)
            .await
            .unwrap();
    }

    let db_refnos = DbModelInstRefnos {
        bran_hanger_refnos: target_bran_hanger_refnos,
        use_cate_refnos: Arc::new(use_cate_refnos),
        loop_owner_refnos: target_loop_owner_refnos,
        prim_refnos: target_prim_refnos,
    };

    Ok(db_refnos)
}

/// 分块处理几何体生成的内部函数
///
/// # 参数
/// * `origin_root_refnos` - 原始根引用号列表
/// * `db_option_arc` - 数据库配置
/// * `incr_updates` - 增量更新日志
/// * `is_incr_update` - 是否增量更新
/// * `has_manual_refnos` - 是否有手动指定的引用号
/// * `skip_exist` - 是否跳过已存在的
/// * `chunk_size` - 分块大小
/// * `sender` - 几何体数据发送通道
async fn process_gen_geos_data_chunks(
    origin_root_refnos: &[RefnoEnum],
    db_option_arc: Arc<DbOptionExt>,
    incr_updates: Option<IncrGeoUpdateLog>,
    is_incr_update: bool,
    has_manual_refnos: bool,
    skip_exist: bool,
    chunk_size: usize,
    sender: flume::Sender<ShapeInstancesData>,
) -> Result<()> {
    let mut all_handles = FuturesUnordered::new();

    let d_types = db_option_arc.debug_refno_types.clone();
    let mut gen_cata_flag =
        d_types.iter().any(|x| x == "CATA") || is_incr_update || has_manual_refnos;
    let mut gen_loop_flag =
        d_types.iter().any(|x| x == "LOOP") || is_incr_update || has_manual_refnos;
    let mut gen_prim_flag =
        d_types.iter().any(|x| x == "PRIM") || is_incr_update || has_manual_refnos;

    let incr_updates_log_arc = Arc::new(incr_updates.clone().unwrap_or_default());
    let mut chunked_root_refnos = origin_root_refnos.chunks(chunk_size);
    let gen_model = db_option_arc.gen_model || is_incr_update || has_manual_refnos;

    println!("╔════════════════════════════════════════════════════════════════╗");
    println!("║         gen_geos_data 模型生成配置检查                          ║");
    println!("╠════════════════════════════════════════════════════════════════╣");
    println!("║ db_option_arc.gen_model: {:<40} ║", db_option_arc.gen_model);
    println!("║ is_incr_update: {:<48} ║", is_incr_update);
    println!("║ has_manual_refnos: {:<45} ║", has_manual_refnos);
    println!("║ gen_model (最终值): {:<44} ║", gen_model);
    println!("╟────────────────────────────────────────────────────────────────╢");
    println!("║ debug_refno_types: {:<44} ║", format!("{:?}", d_types));
    println!("║ gen_cata_flag: {:<49} ║", gen_cata_flag);
    println!("║ gen_loop_flag: {:<49} ║", gen_loop_flag);
    println!("║ gen_prim_flag: {:<49} ║", gen_prim_flag);
    println!("╟────────────────────────────────────────────────────────────────╢");
    println!("║ origin_root_refnos 数量: {:<39} ║", origin_root_refnos.len());
    if !origin_root_refnos.is_empty() {
        println!("║ origin_root_refnos: {:<44} ║", format!("{:?}", origin_root_refnos.iter().take(3).collect::<Vec<_>>()));
    }
    println!("╚════════════════════════════════════════════════════════════════╝");

    debug_model_debug!("========== 开始遍历 root_refnos 小块 ==========");
    debug_model_debug!("准备进入 while 循环");

    while gen_model && let Some(target_refnos) = chunked_root_refnos.next() {
        debug_model_debug!(
            "========== 处理一个小块，包含 {} 个节点 ==========",
            target_refnos.len()
        );
        debug_model_debug!("target_refnos: {:?}", target_refnos);

        // Step 1、提前缓存 ploo，得到对齐方式的偏移
        let loop_sjus_map = DashMap::new();
        {
            let Ok(target_ploo_refnos) = query_multi_descendants(target_refnos, &["PLOO"]).await
            else {
                continue;
            };
            #[cfg(debug_assertions)]
            if !target_ploo_refnos.is_empty() && is_e3d_debug_enabled() {
                debug_model_debug!("target_ploo_refnos: {:?}", target_ploo_refnos.len());
            }
            for r in target_ploo_refnos {
                let Ok(loop_att) = aios_core::get_named_attmap(r).await else {
                    continue;
                };
                let owner = loop_att.get_owner();
                let mut height = loop_att.get_f32("HEIG").unwrap_or_default();
                let sjus = loop_att.get_str("SJUS").unwrap_or_default();
                let off_z =
                    crate::fast_model::gen_model::cate_helpers::cal_sjus_value(sjus, height);
                // 对齐方式的距离，应该存储下来，子节点要与其保持一致的偏移
                // 插入方向和偏移距离
                loop_sjus_map.insert(owner, (Vec3::NEG_Z * off_z, height));
            }
        }
        let loop_sjus_map_arc = Arc::new(loop_sjus_map);

        // Step 2、按类目先逐个分好类的参考号集合
        // 2.1 管道或者支吊架的分类
        let target_bran_hanger_refnos: Vec<RefnoEnum> = if is_incr_update {
            incr_updates_log_arc
                .bran_hanger_refnos
                .iter()
                .cloned()
                .collect()
        } else {
            // 查找后代中的 BRAN/HANG
            let mut r = query_multi_descendants(target_refnos, &["BRAN", "HANG"])
                .await
                .unwrap();
            
            // 🔧 修复：同时检查 target_refnos 本身是否为 BRAN/HANG 类型
            // 这对于用户直接传入 BRAN refno 的场景是必需的
            for refno in target_refnos {
                if let Ok(Some(pe)) = aios_core::get_pe(*refno).await {
                    let noun = pe.noun.to_uppercase();
                    if (noun == "BRAN" || noun == "HANG") && !r.contains(refno) {
                        debug_model_debug!("[BRAN_FIX] 添加 target_refno 本身作为 BRAN/HANG: {}", refno);
                        r.push(*refno);
                    }
                }
            }
            
            r.into_iter().collect()
        };
        // 🔧 修复：先收集 BRAN 的子元件，再用子元件的 refno 查询 cata_hash
        // 之前错误地使用 BRAN 本身的 refno，但 BRAN 不是元件库元素，其 cata_hash 为 "0"
        let mut branch_refnos_map: DashMap<RefnoEnum, Vec<aios_core::pe::SPdmsElement>> = DashMap::new();
        let mut bran_comp_eles: Vec<RefnoEnum> = vec![];
        for &refno in &target_bran_hanger_refnos {
            let children = aios_core::collect_children_elements(refno, &[])
                .await
                .unwrap_or_default();
            bran_comp_eles.extend(children.iter().map(|x| x.refno));
            branch_refnos_map.insert(refno, children);
        }
        debug_model_debug!(
            "[BRAN_FIX] 收集 BRAN 子元件完成: bran_count={}, child_count={}",
            target_bran_hanger_refnos.len(),
            bran_comp_eles.len()
        );

        // 使用子元件的 refno 查询 cata_hash（而非 BRAN 本身）
        let target_bran_reuse_cata_map: DashMap<String, CataHashRefnoKV> = if bran_comp_eles.is_empty() {
            DashMap::new()
        } else {
            let map = build_cata_hash_map_from_tree(&bran_comp_eles)
                .await
                .unwrap_or_default();
            debug_model_debug!(
                "[BRAN_FIX] 子元件 cata_hash 分组完成: unique_cata_count={}",
                map.len()
            );
            for kv in map.iter() {
                debug_model_debug!(
                    "  cata_hash: {}, group_refnos: {:?}",
                    kv.key(),
                    kv.value().group_refnos
                );
            }
            map
        };
        let mut use_cata_refnos = HashSet::new();
        // 查询单个使用元件库的数量
        let target_single_cata_map = if is_incr_update {
            let cata_refnos = &incr_updates_log_arc.basic_cata_refnos;
            if cata_refnos.is_empty() {
                DashMap::new()
            } else {
                let cata_refnos_vec: Vec<RefnoEnum> =
                    cata_refnos.iter().copied().collect();
                build_cata_hash_map_from_tree(&cata_refnos_vec)
                    .await
                    .unwrap_or_default()
            }
        } else {
            // 查询是否是单个使用元件库，父节点是 BRAN HANG
            let sql = format!(
                "select value refno from [{}] where owner.noun in ['BRAN', 'HANG']",
                target_refnos
                    .iter()
                    .map(|x| x.to_pe_key())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            let mut response = SUL_DB.query(sql).await.unwrap();

            let Ok(bran_children_refnos) = response.take::<Vec<RefnoEnum>>(0) else {
                debug_model_debug!("[WARN] 查询BRAN, HANG出错");
                continue;
            };
            let single_refnos = target_refnos
                .iter()
                .filter(|x| !target_bran_hanger_refnos.contains(x))
                .map(|x| *x)
                .collect::<Vec<_>>();

            debug_model_debug!("========== 调试模式：查询子孙节点 ==========");
            debug_model_debug!("target_refnos: {:?}", target_refnos);
            debug_model_debug!(
                "target_bran_hanger_refnos: {:?}",
                &target_bran_hanger_refnos
            );
            debug_model_debug!("single_refnos: {:?}", &single_refnos);
            debug_model_debug!("single_refnos 数量: {}", single_refnos.len());

            use_cata_refnos =
                aios_core::query_deep_children_refnos_filter_spre(&single_refnos, skip_exist)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .collect::<HashSet<_>>();

            debug_model_debug!(
                "查询子孙节点后 use_cata_refnos 数量: {}",
                use_cata_refnos.len()
            );
            debug_model_debug!("use_cata_refnos: {:?}", &use_cata_refnos);

            // 🔧 修复：不再将 bran_children_refnos 添加到 use_cata_refnos
            // BRAN 子元件已在 target_bran_reuse_cata_map 中处理，避免重复
            // use_cata_refnos.extend(bran_children_refnos);
            debug_model_debug!(
                "[BRAN_FIX] 跳过 bran_children_refnos 扩展，避免与 target_bran_reuse_cata_map 重复"
            );

            debug_model_debug!(
                "use_cata_refnos 数量 (排除 BRAN 子元件): {}",
                use_cata_refnos.len()
            );

            let use_cata_vec: Vec<RefnoEnum> = use_cata_refnos.iter().copied().collect();
            let map = build_cata_hash_map_from_tree(&use_cata_vec)
                .await
                .unwrap_or_default();

            debug_model_debug!("tree cata_hash 分组 map 数量: {}", map.len());
            for kv in map.iter() {
                debug_model_debug!(
                    "  cata_hash: {}, group_refnos: {:?}",
                    kv.key(),
                    kv.value().group_refnos
                );
            }
            map
        };
        // 元件库的模型计算（BRAN/HANG 子元件已在前面收集）
        if !target_bran_hanger_refnos.is_empty() && gen_cata_flag {
            if !target_bran_reuse_cata_map.is_empty() || !branch_refnos_map.is_empty() {
                let sjus_map_clone = loop_sjus_map_arc.clone();
                let db_option = db_option_arc.clone();
                let sender = sender.clone();
                let handle = tokio::spawn(async move {
                    let start_time = Instant::now();
                    cata_model::gen_cata_geos(
                        db_option,
                        Arc::new(target_bran_reuse_cata_map),
                        Arc::new(branch_refnos_map),
                        sjus_map_clone,
                        sender,
                    )
                    .await
                    .unwrap();
                });
                all_handles.push(handle);
            }
        }

        if gen_cata_flag && !target_single_cata_map.is_empty() {
            let sjus_map_clone = loop_sjus_map_arc.clone();
            let db_option = db_option_arc.clone();
            let sender = sender.clone();
            let handle = tokio::spawn(async move {
                let start_time = Instant::now();
                cata_model::gen_cata_geos(
                    db_option,
                    Arc::new(target_single_cata_map),
                    Arc::new(Default::default()),
                    sjus_map_clone,
                    sender,
                )
                .await
                .unwrap();
            });
            all_handles.push(handle);
        }

        // ============================================================
        // 🔍 LOOP 节点查询 - 添加详细调试日志
        // ============================================================
        let target_loop_owner_refnos: Vec<RefnoEnum> = if is_incr_update {
            incr_updates_log_arc
                .loop_owner_refnos
                .iter()
                .cloned()
                .collect()
        } else {
            println!("========== 🔍 开始查询 LOOP 节点 ==========");
            println!("target_refnos 数量: {}", target_refnos.len());
            println!("target_refnos: {:?}", target_refnos);
            println!("查询的 LOOP 类型: {:?}", GNERAL_LOOP_OWNER_NOUN_NAMES);

            // 🔧 修复：先检查 target_refnos 本身是否为 LOOP 类型
            let mut loop_owner_refnos = Vec::new();
            for refno in target_refnos {
                if let Ok(Some(pe)) = aios_core::get_pe(*refno).await {
                    let noun_upper = pe.noun.to_uppercase();
                    if GNERAL_LOOP_OWNER_NOUN_NAMES.contains(&noun_upper.as_str()) {
                        println!("✅ target_refno 本身是 LOOP 类型: {} (noun={})", refno, pe.noun);
                        loop_owner_refnos.push(*refno);
                    }
                }
            }

            // 再查询子孙节点中的 LOOP 类型
            let mut descendants_loop =
                query_multi_descendants(target_refnos, &GNERAL_LOOP_OWNER_NOUN_NAMES)
                    .await
                    .unwrap_or_default();

            println!("✅ query_multi_descendants 查询结果: {} 个 LOOP 节点", descendants_loop.len());
            loop_owner_refnos.append(&mut descendants_loop);

            println!("✅ 总共找到 {} 个 LOOP 节点（包含 target_refnos 本身）", loop_owner_refnos.len());

            if !loop_owner_refnos.is_empty() {
                println!("   LOOP 节点列表（前10个）: {:?}", loop_owner_refnos.iter().take(10).collect::<Vec<_>>());
            } else {
                println!("   ⚠️  未找到任何 LOOP 节点");

                // 🔧 使用 TreeIndexManager 进行二次验证
                println!("\n========== 🔍 使用 TreeIndexManager 进行二次验证 ==========");
                use crate::fast_model::gen_model::tree_index_manager::TreeIndexManager;

                // 获取 dbnum - 使用 TreeIndexManager::resolve_dbnum_for_refno 正确解析
                let dbnums = if let Some(first_refno) = target_refnos.first() {
                    match TreeIndexManager::resolve_dbnum_for_refno(*first_refno).await {
                        Ok(dbnum) => {
                            println!("📌 使用 dbnum: {} (从 refno {} 解析)", dbnum, first_refno);
                            vec![dbnum]
                        }
                        Err(e) => {
                            println!("⚠️  无法从 refno {} 解析 dbnum: {}", first_refno, e);
                            vec![]
                        }
                    }
                } else {
                    println!("⚠️  target_refnos 为空");
                    vec![]
                };

                if !dbnums.is_empty() {
                    let manager = TreeIndexManager::with_default_dir(dbnums.clone());
                    println!("✅ TreeIndexManager 已创建，管理的 dbnum: {:?}", manager.dbnums());

                    for refno in target_refnos {
                        println!("\n🔍 检查节点: {}", refno);

                        // 1. 检查节点本身的类型
                        if let Ok(Some(pe)) = aios_core::get_pe(*refno).await {
                            println!("   节点类型: noun={}, name={}", pe.noun, pe.name);
                        }

                        // 2. 使用 TreeIndexManager 查询子孙节点
                        let descendants = manager.query_descendants_filtered(
                            *refno,
                            &GNERAL_LOOP_OWNER_NOUN_NAMES,
                            None, // 无深度限制
                        );

                        println!("   TreeIndexManager 查询结果: {} 个 LOOP 节点", descendants.len());
                        if !descendants.is_empty() {
                            println!("   ✅ 找到的 LOOP 节点（前5个）:");
                            for (i, desc) in descendants.iter().take(5).enumerate() {
                                if let Ok(Some(pe)) = aios_core::get_pe(*desc).await {
                                    println!("      {}. {} (noun={}, name={})", i+1, desc, pe.noun, pe.name);
                                }
                            }

                            // 如果 TreeIndexManager 找到了但 query_multi_descendants 没找到
                            println!("\n   ⚠️  警告: TreeIndexManager 找到了 {} 个 LOOP 节点，但 query_multi_descendants 未找到！", descendants.len());
                            println!("   可能原因: tree_index 文件存在但 query_multi_descendants 使用了 SurrealDB 查询");
                        } else {
                            println!("   TreeIndexManager 也未找到 LOOP 节点");
                        }

                        // 3. 查询所有子孙节点类型分布
                        let all_descendants = manager.query_descendants(*refno, None);
                        println!("   总共 {} 个子孙节点", all_descendants.len());

                        if !all_descendants.is_empty() {
                            // 统计类型分布
                            use std::collections::HashMap;
                            let mut noun_counts: HashMap<String, usize> = HashMap::new();
                            for desc in all_descendants.iter().take(100) { // 只统计前100个
                                if let Ok(Some(pe)) = aios_core::get_pe(*desc).await {
                                    *noun_counts.entry(pe.noun).or_insert(0) += 1;
                                }
                            }
                            println!("   子孙节点类型分布（前10种，最多统计100个节点）:");
                            let mut sorted: Vec<_> = noun_counts.iter().collect();
                            sorted.sort_by(|a, b| b.1.cmp(a.1));
                            for (noun, count) in sorted.iter().take(10) {
                                println!("      {}: {}", noun, count);
                            }
                        }
                    }
                    println!("========== TreeIndexManager 验证完成 ==========\n");
                } else {
                    println!("⚠️  无法获取 dbnum，跳过 TreeIndexManager 验证");
                }
            }
            println!("========== LOOP 节点查询完成 ==========\n");

            loop_owner_refnos
        };

        if gen_loop_flag && !target_loop_owner_refnos.is_empty() {
            println!("✅ 开始生成 {} 个 LOOP 模型", target_loop_owner_refnos.len());
            let sjus_map_clone = loop_sjus_map_arc.clone();
            let sender = sender.clone();
            let db_option = db_option_arc.clone();
            let handle = tokio::spawn(async move {
                loop_model::gen_loop_geos(
                    db_option,
                    &target_loop_owner_refnos,
                    sjus_map_clone,
                    sender,
                )
                .await
                .unwrap();
            });
            all_handles.push(handle);
        } else if gen_loop_flag {
            println!("⚠️  gen_loop_flag=true 但 target_loop_owner_refnos 为空，跳过 LOOP 生成");
        } else {
            println!("ℹ️  gen_loop_flag=false，跳过 LOOP 生成");
        }

        // ============================================================
        // 🔍 PRIM 节点查询 - 添加调试日志
        // ============================================================
        let target_prim_refnos: Vec<RefnoEnum> = if is_incr_update {
            incr_updates_log_arc.prim_refnos.iter().cloned().collect()
        } else {
            println!("========== 🔍 开始查询 PRIM 节点 ==========");
            println!("查询的 PRIM 类型: {:?}", GNERAL_PRIM_NOUN_NAMES);

            let mut prim_refnos = query_multi_descendants(target_refnos, &GNERAL_PRIM_NOUN_NAMES)
                .await
                .unwrap_or_default();

            println!("✅ query_multi_descendants 查询结果: {} 个 PRIM 节点", prim_refnos.len());
            if !prim_refnos.is_empty() {
                println!("   PRIM 节点列表（前10个）: {:?}", prim_refnos.iter().take(10).collect::<Vec<_>>());
            }
            debug_model_trace!("prim_refnos: {:?}", &prim_refnos);
            println!("========== PRIM 节点查询完成 ==========\n");

            prim_refnos.into_iter().collect()
        };

        // 基本元件的生成
        if gen_prim_flag && !target_prim_refnos.is_empty() {
            println!("✅ 开始生成 {} 个 PRIM 模型", target_prim_refnos.len());
            // 基本体模型的生成
            let db_option = db_option_arc.clone();
            let sender = sender.clone();
            let handle = tokio::spawn(async move {
                prim_model::gen_prim_geos(db_option, target_prim_refnos.as_slice(), sender)
                    .await
                    .unwrap();
            });
            all_handles.push(handle);
        } else if gen_prim_flag {
            println!("⚠️  gen_prim_flag=true 但 target_prim_refnos 为空，跳过 PRIM 生成");
        } else {
            println!("ℹ️  gen_prim_flag=false，跳过 PRIM 生成");
        }
        if is_incr_update {
            break;
        }
    }

    while let Some(_result) = all_handles.next().await {
        // 处理每个完成的 future 的结果（当前忽略具体结果）
    }

    Ok(())
}

/// 生成几何体数据（非 Full Noun 模式）
///
/// # 参数
/// * `dbnum` - 可选的数据库编号
/// * `manual_refnos` - 手动指定的引用号列表
/// * `db_option` - 数据库选项
/// * `incr_updates` - 增量更新日志
/// * `sender` - 数据发送通道
/// * `target_sesno` - 目标会话号，用于历史模型生成
/// * `manual_boolean_mode` - 手动布尔运算模式（保留参数，暂未使用）
///
/// # 返回
/// 返回目标根引用号列表
///
/// # 适用场景
/// - 增量更新模式（`incr_updates` 不为空）
/// - 手动 refno 模式（`manual_refnos` 不为空）
/// - 调试模式（`db_option.debug_model_refnos` 有值）
/// - 按数据库编号的全量生成（`dbnum` 有值）
pub async fn gen_geos_data(
    dbnum: Option<u32>,
    manual_refnos: Vec<RefnoEnum>,
    db_option: &DbOptionExt,
    incr_updates: Option<IncrGeoUpdateLog>,
    sender: flume::Sender<ShapeInstancesData>,
    target_sesno: Option<u32>,
    manual_boolean_mode: bool,
) -> Result<Vec<RefnoEnum>> {
    const CHUNK_SIZE: usize = 100;

    // 根据需要拉入数据到本地数据库也可以
    let is_incr_update = incr_updates.is_some();
    let has_manual_refnos = !manual_refnos.is_empty();

    // 排除增量更新的情况，如果 debug_model_refnos 为空，即没有模型需要生成
    let debug_model_refnos = db_option.get_all_debug_refnos().await;
    let has_debug = !debug_model_refnos.is_empty();
    let skip_exist = !db_option.is_replace_mesh();

    println!("========== DEBUG: gen_geos_data ==========");
    println!(
        "debug_model_refnos 配置: {:?}",
        db_option.debug_model_refnos
    );
    println!("解析后的 debug_model_refnos: {:?}", debug_model_refnos);
    println!("debug_model_refnos 数量: {}", debug_model_refnos.len());
    println!(
        "is_incr_update: {}, has_manual_refnos: {}",
        is_incr_update, has_manual_refnos
    );
    debug_model_trace!("debug_model_refnos: {:?}", &debug_model_refnos);

    if !is_incr_update
        // debug_model_refnos = [] 时表示不生成模型，如果没有这个属性表示生成所有
        && (db_option.debug_model_refnos.is_some() && debug_model_refnos.is_empty())
        && (!has_manual_refnos)
    {
        println!("DEBUG: 没有模型需要生成，提前返回");
        return Ok(vec![]);
    }
    if is_incr_update && incr_updates.as_ref().unwrap().count() == 0 {
        return Ok(vec![]);
    }

    let db_option_arc = Arc::new(db_option.clone());
    let is_debug = debug_model_refnos.len() > 0;

    let include_history = db_option_arc.is_gen_history_model();
    let is_replace_mesh = db_option_arc.is_replace_mesh();
    let incr_count = if is_incr_update {
        incr_updates.as_ref().unwrap().count()
    } else {
        0
    };

    let mut target_root_refnos = vec![];
    if is_incr_update {
        // root_refnos 为 incr_update_log 里的 loop_refnos，basic_cata_refnos，prim_refnos 的合集
        target_root_refnos = incr_updates
            .as_ref()
            .unwrap()
            .get_all_visible_refnos()
            .into_iter()
            .collect();
    } else if is_debug || has_manual_refnos {
        target_root_refnos = if has_manual_refnos {
            manual_refnos.clone()
        } else {
            debug_model_refnos.clone()
        };
        debug_model_debug!(
            "DEBUG: 使用调试模式，target_root_refnos: {:?}",
            target_root_refnos
        );

        // 查询目标节点的基本信息
        for refno in &target_root_refnos {
            match aios_core::get_pe(*refno).await {
                Ok(Some(pe)) => {
                    // 查询元件库关系
                    match aios_core::get_named_attmap(*refno).await {
                        Ok(att_map) => {
                            // 先检查是否有直接的 CATR 关系（如 NOZZ）
                            if let Some(catr_refno) = att_map.get_foreign_refno("CATR") {
                                debug_model_debug!("✅ 直接 CATR 关系: {}", catr_refno);
                                if let Some(catr_attr) = att_map.get_as_string("CATR") {
                                    debug_model_debug!("   CATR 属性原始值: {}", catr_attr);
                                }

                                // 查询 CATR 的详细信息
                                match aios_core::get_pe(catr_refno).await {
                                    Ok(Some(catr_pe)) => {
                                        debug_model_debug!(
                                            "   CATR noun: {}, name: {}",
                                            catr_pe.noun,
                                            catr_pe.name
                                        );
                                    }
                                    Ok(None) => {
                                        debug_model_debug!(
                                            "   ⚠️ 未找到 CATR 元素: {}",
                                            catr_refno
                                        );
                                    }
                                    Err(err) => {
                                        debug_model_debug!(
                                            "   ❌ 查询 CATR 元素失败 {}: {}",
                                            catr_refno,
                                            err
                                        );
                                    }
                                }
                            }
                            // 再检查是否有 SPRE 关系
                            else if let Some(spre_refno) = att_map.get_foreign_refno("SPRE") {
                                debug_model_debug!("SPRE refno: {}", spre_refno);

                                // 查询 SPRE 指向的 CATR
                                match aios_core::get_named_attmap(spre_refno).await {
                                    Ok(spre_att) => {
                                        if let Some(catr_refno) = spre_att.get_foreign_refno("CATR")
                                        {
                                            debug_model_debug!(
                                                "   通过 SPRE 的 CATR: {}",
                                                catr_refno
                                            );
                                        } else {
                                            debug_model_debug!("   ⚠️ SPRE 没有 CATR 关系");
                                        }
                                    }
                                    Err(err) => {
                                        debug_model_debug!(
                                            "   ❌ 查询 SPRE 属性失败 {}: {}",
                                            spre_refno,
                                            err
                                        );
                                    }
                                }
                            } else {
                                debug_model_debug!("⚠️ 没有 CATR 或 SPRE 关系");
                            }
                        }
                        Err(err) => {
                            debug_model_debug!("❌ 查询 attmap 失败 {}: {}", refno, err);
                        }
                    }
                }
                Ok(None) => {
                    debug_model_debug!("⚠️ 找不到元素 {}", refno);
                }
                Err(err) => {
                    debug_model_debug!("❌ 查询元素失败 {}: {}", refno, err);
                }
            }
        }
    } else if dbnum.is_some() {
        // 检查是否需要进行历史查询
        if let Some(sesno) = target_sesno {
            println!(
                "使用历史查询，目标会话号: {} (注意：当前使用当前数据替代)",
                sesno
            );
            target_root_refnos = query_by_type(&["SITE"], dbnum.unwrap() as i32, Some(true))
                .await?
                .into_iter()
                .collect();
        } else {
            // 使用当前数据查询
            target_root_refnos = query_by_type(&["SITE"], dbnum.unwrap() as i32, Some(true))
                .await?
                .into_iter()
                .collect();
        }
    }

    let origin_root_refnos = target_root_refnos.clone();

    process_gen_geos_data_chunks(
        &origin_root_refnos,
        db_option_arc.clone(),
        incr_updates.clone(),
        is_incr_update,
        has_manual_refnos,
        skip_exist,
        CHUNK_SIZE,
        sender.clone(),
    )
    .await?;

    if dbnum.is_some() {
        println!("数据库号： {} 生成instances完毕。", dbnum.unwrap());
    }

    Ok(target_root_refnos)
}
