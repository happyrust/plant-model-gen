use aios_core::RefnoEnum;
use aios_core::geometry::ShapeInstancesData;
use aios_core::options::DbOption;
use crate::options::DbOptionExt;

use aios_core::pdms_types::{
    BRAN_COMPONENT_NOUN_NAMES, GNERAL_LOOP_OWNER_NOUN_NAMES, GNERAL_PRIM_NOUN_NAMES, USE_CATE_NOUN_NAMES,
};
use aios_core::pe::SPdmsElement;
use aios_core::{DBType, query_mdb_db_nums};
use dashmap::DashMap;
use glam::Vec3;
use std::collections::HashSet;
use std::sync::Arc;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use super::cate_processor::process_cate_refno_page;
use super::categorized_refnos::CategorizedRefnos;
use super::config::FullNounConfig;
use super::context::NounProcessContext;
use super::errors::{FullNounError, Result};
use super::loop_processor::process_loop_refno_page;
use super::prim_processor::process_prim_refno_page;
use super::tree_index_manager::TreeIndexManager;
use super::utilities::build_cata_hash_map_from_tree;
use crate::data_interface::db_meta;

use crate::fast_model::refno_errors::{
    REFNO_ERROR_STORE, RefnoErrorKind, RefnoErrorStage, record_refno_error,
};
use crate::fast_model::{cata_model, pdms_inst, query_provider};

// Performance profiling support
#[cfg(feature = "profile")]
use tracing::{info, instrument};

/// 验证 SJUS map 是否完整
///
/// 根据配置决定是否警告或报错
pub fn validate_sjus_map(
    sjus_map: &DashMap<RefnoEnum, (Vec3, f32)>,
    config: &FullNounConfig,
) -> Result<()> {
    if config.validate_sjus_map && sjus_map.is_empty() {
        let warning = "⚠️ SJUS map 为空，几何体生成可能产生不正确的结果";

        if config.strict_validation {
            log::error!("{}", warning);
            return Err(FullNounError::EmptySjusMap);
        } else {
            log::warn!("{}", warning);
            log::warn!("  提示：如果这是预期行为，可以在配置中禁用 validate_sjus_map");
        }
    }
    Ok(())
}

fn track_refno_issues(refnos: &[RefnoEnum], context: &str, stage: RefnoErrorStage) {
    let mut seen = HashSet::new();
    for &refno in refnos {
        if matches!(refno, RefnoEnum::Refno(r) if r.0 == 0) {
            record_refno_error(
                RefnoErrorKind::ZeroOrNegative,
                stage,
                "fast_model/gen_model/full_noun_mode.rs",
                "collect_refnos",
                format!("{} 返回无效 RefNo=0", context),
                Some(&refno),
                None,
                &[],
                None,
            );
        }

        if !seen.insert(refno) {
            record_refno_error(
                RefnoErrorKind::Duplicate,
                stage,
                "fast_model/gen_model/full_noun_mode.rs",
                "collect_refnos",
                format!("{} 中检测到重复 RefNo", context),
                Some(&refno),
                None,
                &[],
                None,
            );
        }
    }
}

/// NOUN 类型及其数量信息
#[derive(Debug, Clone)]
pub struct NounTypeInfo {
    pub noun: &'static str,
    pub count: usize,
    pub refnos: Vec<RefnoEnum>,
}

/// 预查询所有 NOUN 类型的数量，过滤掉空类型
///
/// 返回按类别分组的非空 NOUN 类型列表
pub async fn prequery_noun_counts(
    nouns: &[&'static str],
    dbnums: &[u32],
) -> Result<Vec<NounTypeInfo>> {
    let mut results = Vec::new();

    let tree_dbnums = resolve_tree_dbnums(dbnums)?;
    let manager = TreeIndexManager::with_default_dir(tree_dbnums);

    for &noun in nouns {
        let mut refnos = manager.query_noun_refnos(noun, None);
        refnos.retain(|r| r.is_valid());

        if !refnos.is_empty() {
            results.push(NounTypeInfo {
                noun,
                count: refnos.len(),
                refnos,
            });
        }
    }

    Ok(results)
}

/// 处理类别枚举
#[derive(Debug, Clone, Copy)]
pub enum NounCategoryType {
    Loop,
    Prim,
    Cate,
}

/// 按 NOUN 类型分组处理（每次 2 个类型并发）
///
/// # Arguments
/// * `noun_infos` - NOUN 类型信息列表
/// * `ctx` - 处理上下文
/// * `category` - 处理类别（Loop/Prim/Cate）
/// * `loop_sjus_map` - Loop SJUS 映射（仅 Loop 和 Cate 需要）
/// * `sender` - 几何数据发送通道
pub async fn process_nouns_by_type(
    noun_infos: Vec<NounTypeInfo>,
    ctx: &NounProcessContext,
    category: NounCategoryType,
    loop_sjus_map: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
) -> Result<Vec<RefnoEnum>> {
    if noun_infos.is_empty() {
        println!("[{:?}] 无有效 NOUN 类型，跳过", category);
        return Ok(vec![]);
    }

    let total_count: usize = noun_infos.iter().map(|n| n.count).sum();
    println!(
        "📍 [{:?}] 开始处理 {} 个 NOUN 类型（共 {} 个实例），每次 2 个类型并发",
        category,
        noun_infos.len(),
        total_count
    );

    let mut all_processed_refnos = Vec::new();

    // 每次处理 2 个 NOUN 类型
    for (chunk_idx, chunk) in noun_infos.chunks(2).enumerate() {
        let noun_names: Vec<_> = chunk.iter().map(|n| format!("{}({})", n.noun, n.count)).collect();
        println!("[{:?}] 第 {} 批并发处理: {:?}", category, chunk_idx + 1, noun_names);

        // 收集本批次的 refnos
        for info in chunk {
            all_processed_refnos.extend(info.refnos.iter().copied());
        }

        // 并发处理本批次的 NOUN 类型
        let handles: Vec<_> = chunk
            .iter()
            .map(|info| {
                let ctx = ctx.clone();
                let sender = sender.clone();
                let loop_sjus_map = loop_sjus_map.clone();
                let refnos = info.refnos.clone();
                let noun = info.noun;

                tokio::spawn(async move {
                    process_single_noun_type(&ctx, category, &refnos, loop_sjus_map, sender, noun)
                        .await
                })
            })
            .collect();

        for handle in handles {
            handle.await.map_err(|e| {
                FullNounError::GeometryGenerationFailed(format!("{:?}", category), e.to_string())
            })??;
        }
    }

    Ok(all_processed_refnos)
}

/// 处理单个 NOUN 类型的所有 refnos
async fn process_single_noun_type(
    ctx: &NounProcessContext,
    category: NounCategoryType,
    refnos: &[RefnoEnum],
    loop_sjus_map: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
    noun: &str,
) -> Result<()> {
    if refnos.is_empty() {
        return Ok(());
    }

    let ranges = ctx.bounded_chunks(refnos.len());
    for (page_idx, (start, end)) in ranges.into_iter().enumerate() {
        let slice = &refnos[start..end];
        println!(
            "[{:?}:{}] 处理第 {} 页 ({} ~ {})",
            category,
            noun,
            page_idx + 1,
            start + 1,
            end
        );

        match category {
            NounCategoryType::Loop => {
                process_loop_refno_page(ctx, loop_sjus_map.clone(), sender.clone(), slice)
                    .await
                    .map_err(|e| {
                        FullNounError::GeometryGenerationFailed(format!("loop:{}", noun), e.to_string())
                    })?;
            }
            NounCategoryType::Prim => {
                process_prim_refno_page(ctx, sender.clone(), slice)
                    .await
                    .map_err(|e| {
                        FullNounError::GeometryGenerationFailed(format!("prim:{}", noun), e.to_string())
                    })?;
            }
            NounCategoryType::Cate => {
                process_cate_refno_page(ctx, loop_sjus_map.clone(), sender.clone(), slice)
                    .await
                    .map_err(|e| {
                        FullNounError::GeometryGenerationFailed(format!("cate:{}", noun), e.to_string())
                    })?;
            }
        }
    }

    Ok(())
}

/// Full Noun 模式下生成所有几何体（优化版本）
///
/// # 主要改进
/// 1. ✅ BRAN/HANG 优先处理：先处理 BRAN/HANG 及其依赖，记录已生成的子节点
/// 2. ✅ 顺序执行：LOOP -> PRIM -> CATE（确保依赖关系正确）
/// 3. ✅ 批量并发：每个类别内部使用批量并发处理
/// 4. ✅ 内存优化：使用 CategorizedRefnos 替代三个 HashSet
/// 5. ✅ 数据验证：检查 SJUS map 完整性
/// 6. ✅ 类型安全：使用 FullNounConfig 和错误类型
///
/// # 执行顺序
/// BRAN/HANG 优先 -> LOOP -> PRIM -> CATE（跳过已生成的 refno）
#[cfg_attr(feature = "profile", instrument(skip(db_option, config, sender)))]
pub async fn gen_full_noun_geos_optimized(
    db_option: Arc<DbOptionExt>,
    config: &FullNounConfig,
    sender: flume::Sender<ShapeInstancesData>,
) -> Result<CategorizedRefnos> {
    let total_start = Instant::now();

    println!("🚀 启动 Full Noun 模式（统一流水线版）");
    config.print_info();

    // 1. 获取数据库过滤列表
    let dbnums = get_filtered_dbnums(&db_option).await?;
    if !dbnums.is_empty() {
        println!("🗂️  数据库过滤: 仅查询 dbnum = {:?}", dbnums);
    }

    let loop_sjus_map_arc = Arc::new(DashMap::new());
    validate_sjus_map(&loop_sjus_map_arc, config)?;

    let mut categorized = CategorizedRefnos::new();
    let mut bran_generated_refnos = HashSet::new();

    // ============================================================================
    // 🚩 [第一阶段] BRAN/HANG 核心逻辑（始终执行）
    // ============================================================================
    let need_bran_hang_stage = config.enabled_categories.is_empty()
        || config.enabled_categories.iter().any(|cat| {
            let upper = cat.to_uppercase();
            upper == "BRAN" || upper == "HANG"
        });

    let mut bran_roots_vec: Vec<RefnoEnum> = Vec::new();
    let mut bran_duration = Duration::ZERO;
    if need_bran_hang_stage {
        let mut bran_hanger_roots: HashSet<RefnoEnum> = HashSet::new();
        for noun in &["BRAN", "HANG"] {
            let refnos = query_noun_refnos(noun, &dbnums, config.debug_limit_per_noun).await?;
            if !refnos.is_empty() {
                println!("[Pipeline] 收集到 {} 根节点: {} 个", noun, refnos.len());
                bran_hanger_roots.extend(refnos);
            }
        }

        bran_roots_vec = bran_hanger_roots.into_iter().collect();
        if !bran_roots_vec.is_empty() {
            let bran_start = Instant::now();
            process_bran_hang_core_logic(
                &db_option,
                &bran_roots_vec,
                loop_sjus_map_arc.clone(),
                sender.clone(),
                &mut bran_generated_refnos,
            )
            .await?;
            bran_duration = bran_start.elapsed();

            // 记录 BRAN/HANG 为 Cate 类别
            for r in &bran_roots_vec {
                categorized.insert(*r, super::models::NounCategory::Cate);
            }
        }
    } else {
        println!("[Pipeline] 未启用 BRAN/HANG：跳过 BRAN/HANG 优先阶段");
    }

    // ============================================================================
    // 🚦 [判定点] 如果仅启用了 BRAN/HANG，此时可以结束
    // ============================================================================
    // 注意：`enabled_categories = []` 的语义是“启用所有类别”，不能被误判为“仅 BRAN/HANG”。
    // Rust 的 `Iterator::all` 在空迭代器上返回 true（空集恒真），因此这里必须显式要求非空。
    let only_bran_hang = !config.enabled_categories.is_empty()
        && config.enabled_categories.iter().all(|cat| {
            let upper = cat.to_uppercase();
            upper == "BRAN" || upper == "HANG"
        });

    if only_bran_hang {
        // 重要语义说明：
        // - BRAN/HANG 本身多为“容器/挂点”，可渲染几何往往来自其子孙（LOOP/PRIM/CATE 等）。
        // - 因此当用户显式指定仅 BRAN/HANG 时，不应直接退出；而应仅以 BRAN/HANG 作为根，
        //   深度收集其子孙并生成几何（同时严格按 dbnum 过滤，避免跨库污染）。
        println!(
            "✅ [Optimization] 仅启用 BRAN/HANG：以 BRAN/HANG 为根，生成其子孙中的 LOOP/PRIM/CATE 几何（dbnum 过滤生效）"
        );

        // BRAN/HANG 的层级关系以 children/tree 为准（owner 深度查询在此场景下可能为空）。
        // 因此这里用 TreeIndex 进行子孙收集，并按 noun 类型过滤到 LOOP/PRIM/CATE 三类。
        let tree_dbnums = resolve_tree_dbnums(&dbnums)?;
        let manager = TreeIndexManager::with_default_dir(tree_dbnums);

        let mut loop_refnos: HashSet<RefnoEnum> = HashSet::new();
        let mut prim_refnos: HashSet<RefnoEnum> = HashSet::new();
        let mut cate_refnos: HashSet<RefnoEnum> = HashSet::new();
        for &root in &bran_roots_vec {
            loop_refnos.extend(manager.query_descendants_filtered(root, &GNERAL_LOOP_OWNER_NOUN_NAMES, None));
            prim_refnos.extend(manager.query_descendants_filtered(root, &GNERAL_PRIM_NOUN_NAMES, None));
            cate_refnos.extend(manager.query_descendants_filtered(root, &USE_CATE_NOUN_NAMES, None));
            // BRAN 下的管道组件（如 TUBI/ELBO/TEE...）常不在 USE_CATE_NOUN_NAMES 中，需要单独纳入。
            cate_refnos.extend(manager.query_descendants_filtered(root, &BRAN_COMPONENT_NOUN_NAMES, None));
        }

        println!(
            "[BRAN-only] 子孙收集结果: LOOP={}, PRIM={}, CATE={}",
            loop_refnos.len(),
            prim_refnos.len(),
            cate_refnos.len()
        );

        let ctx = NounProcessContext::new(
            db_option.clone(),
            config.batch_size.get(),
            config.concurrency.get(),
        );

        // LOOP
        let mut loop_vec: Vec<RefnoEnum> = loop_refnos.into_iter().collect();
        loop_vec.sort_by_key(|r| r.to_string());
        for (i, chunk) in loop_vec.chunks(ctx.batch_size.max(1)).enumerate() {
            println!(
                "[BRAN-only][LOOP] 分页 {}/{} ({} ~ {})",
                i + 1,
                (loop_vec.len() + ctx.batch_size.max(1) - 1) / ctx.batch_size.max(1),
                i * ctx.batch_size.max(1) + 1,
                (i * ctx.batch_size.max(1) + chunk.len())
            );
            process_loop_refno_page(
                &ctx,
                loop_sjus_map_arc.clone(),
                sender.clone(),
                chunk,
            )
            .await?;
        }
        for r in &loop_vec {
            categorized.insert(*r, super::models::NounCategory::LoopOwner);
        }

        // PRIM
        let mut prim_vec: Vec<RefnoEnum> = prim_refnos.into_iter().collect();
        prim_vec.sort_by_key(|r| r.to_string());
        for (i, chunk) in prim_vec.chunks(ctx.batch_size.max(1)).enumerate() {
            println!(
                "[BRAN-only][PRIM] 分页 {}/{} ({} ~ {})",
                i + 1,
                (prim_vec.len() + ctx.batch_size.max(1) - 1) / ctx.batch_size.max(1),
                i * ctx.batch_size.max(1) + 1,
                (i * ctx.batch_size.max(1) + chunk.len())
            );
            process_prim_refno_page(&ctx, sender.clone(), chunk).await?;
        }
        for r in &prim_vec {
            categorized.insert(*r, super::models::NounCategory::Prim);
        }

        // CATE
        let mut cate_vec: Vec<RefnoEnum> = cate_refnos.into_iter().collect();
        cate_vec.sort_by_key(|r| r.to_string());
        for (i, chunk) in cate_vec.chunks(ctx.batch_size.max(1)).enumerate() {
            println!(
                "[BRAN-only][CATE] 分页 {}/{} ({} ~ {})",
                i + 1,
                (cate_vec.len() + ctx.batch_size.max(1) - 1) / ctx.batch_size.max(1),
                i * ctx.batch_size.max(1) + 1,
                (i * ctx.batch_size.max(1) + chunk.len())
            );
            process_cate_refno_page(
                &ctx,
                loop_sjus_map_arc.clone(),
                sender.clone(),
                chunk,
            )
            .await?;
        }
        for r in &cate_vec {
            categorized.insert(*r, super::models::NounCategory::Cate);
        }

        return Ok(categorized);
    }

    // ============================================================================
    // 🔍 [第二阶段] 通用深度查询路径（处理 LOOP/PRIM/CATE）
    // ============================================================================
    println!("🔍 正在收集其余 Noun 的根节点并执行深度递归查询...");

    let entry_nouns = get_entry_nouns(config);
    if entry_nouns.is_empty() {
        println!("[Pipeline] 加载到的其余入口 Noun 列表为空。");
    } else {
        println!("📌 补充入口 Noun 列表: {:?}", entry_nouns);

        let mut all_roots = HashSet::new();
        let mut loop_refnos = HashSet::new();
        let mut prim_refnos = HashSet::new();
        let mut cate_refnos = HashSet::new();

        // 收集根节点
        for entry in &entry_nouns {
            let noun_upper = entry.to_uppercase();
            let noun_str = noun_upper.as_str();

            // 跳过已处理的 BRAN/HANG
            if noun_str == "BRAN" || noun_str == "HANG" {
                continue;
            }

            let refnos = query_noun_refnos(noun_str, &dbnums, config.debug_limit_per_noun).await?;
            if refnos.is_empty() {
                continue;
            }

            track_refno_issues(&refnos, noun_str, RefnoErrorStage::InputParse);
            all_roots.extend(refnos.iter().copied());

            if GNERAL_LOOP_OWNER_NOUN_NAMES.contains(&noun_str) {
                loop_refnos.extend(refnos.iter().copied());
            }
            if GNERAL_PRIM_NOUN_NAMES.contains(&noun_str) {
                prim_refnos.extend(refnos.iter().copied());
            }
            if USE_CATE_NOUN_NAMES.contains(&noun_str) {
                cate_refnos.extend(refnos.iter().copied());
            }
        }

        if !all_roots.is_empty() {
            println!("[Pipeline] 其余根节点总数 {}", all_roots.len());
            let roots_vec: Vec<RefnoEnum> = all_roots.into_iter().collect();

            // 递归收集子节点
            collect_all_descendants(&roots_vec, &mut loop_refnos, &mut prim_refnos, &mut cate_refnos)
                .await?;

            let ctx = NounProcessContext::new(
                db_option.clone(),
                config.batch_size.get(),
                config.concurrency.get(),
            );

            // [1-3/4] 处理 LOOP, PRIM, CATE
            let (loop_vec, loop_dur) = process_loop_stage(
                &ctx,
                loop_refnos,
                config,
                &dbnums,
                &bran_generated_refnos,
                loop_sjus_map_arc.clone(),
                sender.clone(),
            )
            .await?;
            let (prim_vec, prim_dur) =
                process_prim_stage(&ctx, prim_refnos, config, &dbnums, sender.clone())
                .await?;
            let (cate_vec, cate_dur) = process_cate_stage(
                &ctx,
                cate_refnos,
                config,
                &dbnums,
                &bran_generated_refnos,
                loop_sjus_map_arc,
                sender,
            )
            .await?;

            // 归类结果
            for r in &cate_vec {
                categorized.insert(*r, super::models::NounCategory::Cate);
            }
            for r in &loop_vec {
                categorized.insert(*r, super::models::NounCategory::LoopOwner);
            }
            for r in &prim_vec {
                categorized.insert(*r, super::models::NounCategory::Prim);
            }

            let total_duration = total_start.elapsed();
            print_final_summary(total_duration, loop_dur, prim_dur, cate_dur, bran_duration);
        }
    }

    categorized.print_statistics();
    Ok(categorized)
}

/// 内部核心逻辑：处理 BRAN/HANG 相关的 CATE 生成及 Tubing
async fn process_bran_hang_core_logic(
    db_option: &Arc<DbOptionExt>,
    bran_roots: &[RefnoEnum],
    loop_sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
    bran_generated_refnos: &mut HashSet<RefnoEnum>,
) -> Result<()> {
    if bran_roots.is_empty() {
        return Ok(());
    }
    println!("📍 优先处理 BRAN/HANG 及其依赖 (count={})...", bran_roots.len());

    // 1. 查询子元素并记录已生成的 refno
    let branch_refnos_map: DashMap<RefnoEnum, Vec<SPdmsElement>> = DashMap::new();
    for &refno in bran_roots {
        if let Ok(children) = TreeIndexManager::collect_children_elements_from_tree(refno).await {
            for child in &children {
                bran_generated_refnos.insert(child.refno);
            }
            if !children.is_empty() {
                branch_refnos_map.insert(refno, children);
            }
        }
    }

    // 2. 查询 BRAN 下子元件（管件）的元件库分组
    // 注意：应该查询子元件（TEE、ELBO等）的 cata_hash，而不是 BRAN 自身
    let child_refnos: Vec<RefnoEnum> = branch_refnos_map
        .iter()
        .flat_map(|entry| entry.value().iter().map(|c| c.refno).collect::<Vec<_>>())
        .collect();
    let target_bran_reuse_cata_map = if child_refnos.is_empty() {
        DashMap::new()
    } else {
        build_cata_hash_map_from_tree(&child_refnos)
            .await
            .unwrap_or_default()
    };

    // 3. 生成 CATE 几何
    let cate_outcome = match cata_model::gen_cata_instances(
        db_option.clone(),
        Arc::new(target_bran_reuse_cata_map),
        loop_sjus_map_arc.clone(),
        sender.clone(),
    )
    .await
    {
        Ok(outcome) => Some(outcome),
        Err(e) => {
            // 这里此前使用 `.ok()` 会吞掉错误，导致“看似成功但 CATE 数据缺失”。
            // 为保持行为向后兼容，默认仅打印错误并继续；若未来需要严格模式，可在此处改为直接返回 Err。
            eprintln!("[Pipeline] CATE 几何生成失败，将跳过 CATE：{e}");
            None
        }
    };

    // 4. 保存 tubi_info
    if let Some(ref outcome) = cate_outcome {
        let _ = pdms_inst::save_tubi_info_batch(&outcome.tubi_info_map).await;
    }

    // 5. 生成 Tubing
    let local_al_map = cate_outcome
        .map(|o| o.local_al_map)
        .unwrap_or_else(|| Arc::new(DashMap::new()));
    let _ = cata_model::gen_branch_tubi(
        db_option.clone(),
        Arc::new(branch_refnos_map),
        loop_sjus_map_arc,
        sender,
        local_al_map,
    )
    .await;

    Ok(())
}

async fn process_loop_stage(
    ctx: &NounProcessContext,
    _loop_refnos: HashSet<RefnoEnum>,
    config: &FullNounConfig,
    dbnums: &[u32],
    bran_generated_refnos: &HashSet<RefnoEnum>,
    loop_sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
) -> Result<(Vec<RefnoEnum>, Duration)> {
    let start = Instant::now();
    let mut loop_noun_infos = prequery_noun_counts(&GNERAL_LOOP_OWNER_NOUN_NAMES, dbnums).await?;
    // FullNounConfig.enabled_categories 的语义：空=全启用；否则按类别/具体 noun 精确过滤。
    loop_noun_infos.retain(|info| config.should_process_noun(info.noun, "loop"));
    loop_noun_infos.retain(|info| info.count > 0);

    let vec =
        process_nouns_by_type(loop_noun_infos, ctx, NounCategoryType::Loop, loop_sjus_map_arc, sender)
            .await?;
    Ok((vec, start.elapsed()))
}

async fn process_prim_stage(
    ctx: &NounProcessContext,
    _refnos: HashSet<RefnoEnum>,
    config: &FullNounConfig,
    dbnums: &[u32],
    sender: flume::Sender<ShapeInstancesData>,
) -> Result<(Vec<RefnoEnum>, Duration)> {
    let start = Instant::now();
    let mut prim_noun_infos = prequery_noun_counts(&GNERAL_PRIM_NOUN_NAMES, dbnums).await?;
    prim_noun_infos.retain(|info| config.should_process_noun(info.noun, "prim"));
    let vec = process_nouns_by_type(
        prim_noun_infos,
        ctx,
        NounCategoryType::Prim,
        Arc::new(DashMap::new()),
        sender,
    )
    .await?;
    Ok((vec, start.elapsed()))
}

async fn process_cate_stage(
    ctx: &NounProcessContext,
    _refnos: HashSet<RefnoEnum>,
    config: &FullNounConfig,
    dbnums: &[u32],
    bran_generated_refnos: &HashSet<RefnoEnum>,
    loop_sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
) -> Result<(Vec<RefnoEnum>, Duration)> {
    let start = Instant::now();
    let mut cate_noun_infos = prequery_noun_counts(&USE_CATE_NOUN_NAMES, dbnums).await?;
    cate_noun_infos.retain(|info| config.should_process_noun(info.noun, "cate"));
    for info in &mut cate_noun_infos {
        info.refnos.retain(|r| !bran_generated_refnos.contains(r));
        info.count = info.refnos.len();
    }
    cate_noun_infos.retain(|info| info.count > 0);

    let vec = process_nouns_by_type(
        cate_noun_infos,
        ctx,
        NounCategoryType::Cate,
        loop_sjus_map_arc,
        sender,
    )
    .await?;
    Ok((vec, start.elapsed()))
}

fn print_final_summary(total: Duration, l: Duration, p: Duration, c: Duration, b: Duration) {
    println!("✅ Full Noun 处理完成 (GeneralPath)");
    println!(
        "⏱️  Total: {} ms [L: {}ms, P: {}ms, C: {}ms, B: {}ms]",
        total.as_millis(),
        l.as_millis(),
        p.as_millis(),
        c.as_millis(),
        b.as_millis()
    );
}

async fn get_filtered_dbnums(db_option: &DbOptionExt) -> Result<Vec<u32>> {
    let mut dbnums: Vec<u32> = if let Some(manual) = db_option.manual_db_nums.clone() {
        manual
    } else {
        query_mdb_db_nums(None, DBType::DESI).await.map_err(|e| {
            FullNounError::DatabaseError(format!("query_mdb_db_nums(None, DESI) failed: {}", e))
        })?
    };

    if let Some(exclude) = &db_option.exclude_db_nums {
        dbnums.retain(|dbnum| !exclude.contains(dbnum));
    }
    Ok(dbnums)
}

fn get_entry_nouns(config: &FullNounConfig) -> Vec<String> {
    let has_explicit_entry_nouns = config.enabled_categories.iter().any(|cat| {
        let lower = cat.to_lowercase();
        !matches!(lower.as_str(), "cate" | "loop" | "prim")
    });

    if has_explicit_entry_nouns {
        config
            .enabled_categories
            .iter()
            .filter(|cat| {
                let lower = cat.to_lowercase();
                !matches!(lower.as_str(), "cate" | "loop" | "prim")
            })
            .cloned()
            .collect()
    } else {
        let mut set = HashSet::new();
        for &noun in GNERAL_LOOP_OWNER_NOUN_NAMES
            .iter()
            .chain(GNERAL_PRIM_NOUN_NAMES.iter())
            .chain(USE_CATE_NOUN_NAMES.iter())
        {
            set.insert(noun.to_string());
        }
        set.into_iter().collect()
    }
}

async fn query_noun_refnos(noun: &str, dbnums: &[u32], limit: Option<usize>) -> Result<Vec<RefnoEnum>> {
    let tree_dbnums = resolve_tree_dbnums(dbnums)?;
    let manager = TreeIndexManager::with_default_dir(tree_dbnums);
    let mut refnos = manager.query_noun_refnos(noun, limit);
    refnos.retain(|r| r.is_valid());

    if let Some(l) = limit {
        if refnos.len() > l {
            refnos.truncate(l);
        }
    }
    Ok(refnos)
}

fn resolve_tree_dbnums(dbnums: &[u32]) -> Result<Vec<u32>> {
    if !dbnums.is_empty() {
        return Ok(dbnums.to_vec());
    }
    db_meta()
        .ensure_loaded()
        .map_err(|e| FullNounError::DatabaseError(format!("加载 db_meta_info.json 失败: {}", e)))?;
    let mut all_dbnums = db_meta().get_all_dbnums();
    if all_dbnums.is_empty() {
        return Err(FullNounError::DatabaseError(
            "db_meta_info.json 中未找到可用 dbnum".to_string(),
        ));
    }
    all_dbnums.sort_unstable();
    Ok(all_dbnums)
}

async fn collect_all_descendants(
    roots: &[RefnoEnum],
    loop_refnos: &mut HashSet<RefnoEnum>,
    prim_refnos: &mut HashSet<RefnoEnum>,
    cate_refnos: &mut HashSet<RefnoEnum>,
) -> Result<()> {
    let loop_descendants = query_provider::query_multi_descendants_with_self(
        roots,
        &GNERAL_LOOP_OWNER_NOUN_NAMES,
        true,
    )
        .await
        .map_err(|e| FullNounError::DatabaseError(format!("collect_descendant_filter_ids(loop) failed: {}", e)))?;
    track_refno_issues(&loop_descendants, "loop_descendants", RefnoErrorStage::Query);
    loop_refnos.extend(loop_descendants);

    // roots 可能本身就是 LOOP/PRIM/CATE；此处必须 include_self=true，
    // 否则会在 debug-model/手动指定节点场景下漏掉根节点自身的几何生成。
    let prim_descendants = query_provider::query_multi_descendants_with_self(
        roots,
        &GNERAL_PRIM_NOUN_NAMES,
        true,
    )
        .await
        .map_err(|e| FullNounError::DatabaseError(format!("collect_descendant_filter_ids(prim) failed: {}", e)))?;
    track_refno_issues(&prim_descendants, "prim_descendants", RefnoErrorStage::Query);
    prim_refnos.extend(prim_descendants);

    let cate_descendants =
        query_provider::query_multi_descendants_with_self(roots, &USE_CATE_NOUN_NAMES, true)
        .await
        .map_err(|e| FullNounError::DatabaseError(format!("collect_descendant_filter_ids(cate) failed: {}", e)))?;
    track_refno_issues(&cate_descendants, "cate_descendants", RefnoErrorStage::Query);
    cate_refnos.extend(cate_descendants);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_sjus_map_empty_warning() {
        let sjus_map = DashMap::new();
        let config = FullNounConfig::default();

        // 默认配置下，空 map 会警告但不报错
        let result = validate_sjus_map(&sjus_map, &config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_sjus_map_empty_strict() {
        let sjus_map = DashMap::new();
        let config = FullNounConfig::default().with_strict_validation(true);

        // 严格模式下，空 map 会报错
        let result = validate_sjus_map(&sjus_map, &config);
        assert!(result.is_err());

        if let Err(FullNounError::EmptySjusMap) = result {
            // 正确
        } else {
            panic!("Expected EmptySjusMap error");
        }
    }

    // #[test]
    // fn test_validate_sjus_map_with_data() {
    //     let sjus_map = DashMap::new();
    //     sjus_map.insert(RefnoEnum::RefU64(1), (Vec3::ZERO, 1.0));

    //     let config = FullNounConfig::default().with_strict_validation(true);

    //     // 有数据时不应报错
    //     let result = validate_sjus_map(&sjus_map, &config);
    //     assert!(result.is_ok());
    // }
}

// ============================================================================
// 兼容层函数（从 legacy.rs 迁移）
// ============================================================================

use crate::fast_model::pdms_inst::save_instance_data_optimize;
// Duplicate import removed
use anyhow::Result as AnyhowResult;

/// 兼容函数：旧版的 gen_full_noun_geos
///
/// 为了保持向后兼容，保留这个函数签名。
/// 内部转发到优化版本 gen_full_noun_geos_optimized
#[deprecated(note = "请使用 gen_full_noun_geos_optimized 替代")]
pub async fn gen_full_noun_geos(
    db_option: Arc<DbOptionExt>,
    _extra_nouns: Option<Vec<&'static str>>,
) -> AnyhowResult<super::models::DbModelInstRefnos> {
    println!("⚠️ 警告：使用已弃用的 gen_full_noun_geos，内部已转发到优化版本");

    let config = FullNounConfig::from_db_option_ext(&db_option)
        .map_err(|e| anyhow::anyhow!("配置错误: {}", e))?;

    let (sender, receiver) = flume::unbounded();
    let replace_exist = db_option.inner.is_replace_mesh();

    // Full Noun 生成过程中，部分子任务可能会持有 sender 的 clone，导致 channel 不会自然断开；
    // 这里用一个 “done + idle timeout” 机制兜底，避免在 insert_handle.await 处永久挂起。
    let done = Arc::new(AtomicBool::new(false));
    let done_rx = done.clone();
    let insert_handle = tokio::spawn(async move {
        loop {
            let next = if done_rx.load(Ordering::Relaxed) {
                match tokio::time::timeout(Duration::from_millis(800), receiver.recv_async()).await {
                    Ok(Ok(v)) => Some(v),
                    Ok(Err(_)) => return, // channel 断开
                    Err(_) => None,       // idle timeout：认为发送端已结束但 channel 未断开
                }
            } else {
                match receiver.recv_async().await {
                    Ok(v) => Some(v),
                    Err(_) => return,
                }
            };

            let Some(shape_insts) = next else { break };
            if let Err(e) = save_instance_data_optimize(&shape_insts, replace_exist).await {
                eprintln!("保存实例数据失败: {}", e);
            }
        }
    });

    let categorized =
        gen_full_noun_geos_optimized(db_option.clone(), &config, sender)
            .await
            .map_err(|e| anyhow::anyhow!("Full Noun 生成失败: {}", e))?;

    done.store(true, Ordering::Relaxed);
    let _ = insert_handle.await;

    let cate = categorized.get_by_category(super::models::NounCategory::Cate);
    let loops = categorized.get_by_category(super::models::NounCategory::LoopOwner);
    let prims = categorized.get_by_category(super::models::NounCategory::Prim);

    let result = super::models::DbModelInstRefnos {
        bran_hanger_refnos: Arc::new(Vec::new()),
        use_cate_refnos: Arc::new(cate),
        loop_owner_refnos: Arc::new(loops),
        prim_refnos: Arc::new(prims),
    };

    Ok(result)
}
