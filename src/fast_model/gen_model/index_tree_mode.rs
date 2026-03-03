use aios_core::RefnoEnum;
use aios_core::geometry::ShapeInstancesData;
use aios_core::options::DbOption;
use crate::options::DbOptionExt;

use aios_core::pdms_types::{
    GNERAL_LOOP_OWNER_NOUN_NAMES, GNERAL_PRIM_NOUN_NAMES, USE_CATE_NOUN_NAMES,
};
use aios_core::pe::SPdmsElement;
use dashmap::DashMap;
use glam::Vec3;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use super::cate_processor::process_cate_refno_page;
use super::categorized_refnos::CategorizedRefnos;
use super::config::IndexTreeConfig;
use super::context::{GenStage, NounProcessContext};
use super::errors::{IndexTreeError, Result};
use super::cata_resolve_cache_pipeline;
use super::loop_processor::process_loop_refno_page;
use super::prim_processor::process_prim_refno_page;
use super::tree_index_manager::TreeIndexManager;
use super::utilities::build_cata_hash_map_from_tree;
use crate::data_interface::db_meta;
use crate::fast_model::model_cache::geom_input_cache;
use crate::fast_model::model_cache::cata_resolve_cache;
use crate::fast_model::instance_cache::InstanceCacheManager;
use crate::fast_model::transform_cache;

use crate::fast_model::refno_errors::{
    REFNO_ERROR_STORE, RefnoErrorKind, RefnoErrorStage, record_refno_error,
};
use crate::fast_model::{cata_model, pdms_inst, query_provider};
use aios_core::geometry::EleGeosInfo;
use aios_core::parsed_data::CateAxisParam;
use aios_core::prim_geo::tubing::TubiSize;
use aios_core::shape::pdms_shape::RsVec3;

// Performance profiling support
#[cfg(feature = "profile")]
use tracing::{info, instrument};

/// 验证 SJUS map 是否完整
///
/// 根据配置决定是否警告或报错
pub fn validate_sjus_map(
    sjus_map: &DashMap<RefnoEnum, (Vec3, f32)>,
    config: &IndexTreeConfig,
) -> Result<()> {
    if config.validate_sjus_map && sjus_map.is_empty() {
        let warning = "⚠️ SJUS map 为空，几何体生成可能产生不正确的结果";

        if config.strict_validation {
            log::error!("{}", warning);
            return Err(IndexTreeError::EmptySjusMap);
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
                "fast_model/gen_model/index_tree_mode.rs",
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
                "fast_model/gen_model/index_tree_mode.rs",
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
                IndexTreeError::GeometryGenerationFailed(format!("{:?}", category), e.to_string())
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
                        IndexTreeError::GeometryGenerationFailed(format!("loop:{}", noun), e.to_string())
                    })?;
            }
            NounCategoryType::Prim => {
                process_prim_refno_page(ctx, sender.clone(), slice)
                    .await
                    .map_err(|e| {
                        IndexTreeError::GeometryGenerationFailed(format!("prim:{}", noun), e.to_string())
                    })?;
            }
            NounCategoryType::Cate => {
                process_cate_refno_page(ctx, loop_sjus_map.clone(), sender.clone(), slice)
                    .await
                    .map_err(|e| {
                        IndexTreeError::GeometryGenerationFailed(format!("cate:{}", noun), e.to_string())
                    })?;
            }
        }
    }

    Ok(())
}

/// IndexTree 模式下生成所有几何体（优化版本）
///
/// # 主要改进
/// 1. ✅ BRAN/HANG 优先处理：先处理 BRAN/HANG 及其依赖，记录已生成的子节点
/// 2. ✅ 顺序执行：LOOP -> CATE -> PRIM（确保依赖关系正确）
/// 3. ✅ 批量并发：每个类别内部使用批量并发处理
/// 4. ✅ 内存优化：使用 CategorizedRefnos 替代三个 HashSet
/// 5. ✅ 数据验证：检查 SJUS map 完整性
/// 6. ✅ 类型安全：使用 IndexTreeConfig 和错误类型
///
/// # 执行顺序
/// BRAN/HANG 优先 -> LOOP -> CATE -> PRIM（跳过已生成的 refno）
#[cfg_attr(feature = "profile", instrument(skip(db_option, config, sender)))]
pub async fn gen_index_tree_geos_optimized(
    db_option: Arc<DbOptionExt>,
    config: &IndexTreeConfig,
    sender: flume::Sender<ShapeInstancesData>,
    seed_roots: Option<Vec<RefnoEnum>>,
) -> Result<CategorizedRefnos> {
    let total_start = Instant::now();

    println!("🚀 启动 IndexTree 模式（统一流水线版）");
    config.print_info();
    if let Some(roots) = seed_roots.as_ref() {
        println!("[Pipeline] 使用外部 roots 直入模式: {} 个", roots.len());
    }

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
    let has_seed_roots = seed_roots.is_some();
    let need_bran_hang_stage = has_seed_roots
        || config.enabled_categories.is_empty()
        || config.enabled_categories.iter().any(|cat| {
            let upper = cat.to_uppercase();
            upper == "BRAN" || upper == "HANG"
        });

    let mut bran_roots_vec: Vec<RefnoEnum> = Vec::new();
    let mut bran_duration = Duration::ZERO;
    if need_bran_hang_stage {
        let mut bran_hanger_roots: HashSet<RefnoEnum> = HashSet::new();
        if let Some(roots) = seed_roots.as_ref() {
            for &root in roots {
                if !root.is_valid() {
                    continue;
                }
                let Ok(dbnum) = TreeIndexManager::resolve_dbnum_for_refno(root) else {
                    continue;
                };
                let manager = TreeIndexManager::with_default_dir(vec![dbnum]);
                let Some(noun) = manager.get_noun(root) else {
                    continue;
                };
                let noun_upper = noun.to_uppercase();
                if noun_upper == "BRAN" || noun_upper == "HANG" {
                    bran_hanger_roots.insert(root);
                }
            }
            if !bran_hanger_roots.is_empty() {
                println!(
                    "[Pipeline] seed_roots 中 BRAN/HANG 根节点: {} 个",
                    bran_hanger_roots.len()
                );
            }
        } else {
            for noun in &["BRAN", "HANG"] {
                let refnos = query_noun_refnos(noun, &dbnums, config.index_tree_debug_limit_per_target_type).await?;
                if !refnos.is_empty() {
                    println!("[Pipeline] 收集到 {} 根节点: {} 个", noun, refnos.len());
                    bran_hanger_roots.extend(refnos);
                }
            }
        }

        bran_roots_vec = bran_hanger_roots.into_iter().collect();
        if !bran_roots_vec.is_empty() {
            // BRAN/HANG 阶段也遵循“两阶段（Prefetch -> Generate）”语义：
            // - PrefetchThenGenerate：先填充缓存，再进入离线 Generate
            // - CacheOnly：不预取，直接离线 Generate（若缓存不全应直接失败）
            let ctx_bran_prefetch = NounProcessContext::new(
                db_option.clone(),
                config.batch_size.get(),
                config.concurrency.get(),
            )
            .with_stage(GenStage::Prefetch);
            let ctx_bran_generate = ctx_bran_prefetch.with_stage(GenStage::Generate);

            let bran_start = Instant::now();
            process_bran_hang_core_logic(
                &ctx_bran_prefetch,
                &ctx_bran_generate,
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
    // 🔍 [第二阶段] 通用深度查询路径（处理 LOOP/CATE/PRIM）
    // ============================================================================
    println!("🔍 正在收集其余 Noun 的根节点并执行深度递归查询...");

    let mut all_roots = HashSet::new();
    let mut loop_refnos = HashSet::new();
    let mut prim_refnos = HashSet::new();
    let mut cate_refnos = HashSet::new();

    if let Some(roots) = seed_roots.as_ref() {
        for &root in roots {
            if root.is_valid() {
                all_roots.insert(root);
            }
        }
        if !all_roots.is_empty() {
            let roots_vec: Vec<RefnoEnum> = all_roots.iter().copied().collect();
            track_refno_issues(&roots_vec, "seed_roots", RefnoErrorStage::InputParse);
            println!("[Pipeline] 使用 seed_roots 作为入口 roots，总数 {}", all_roots.len());
        }
    } else {
        let entry_nouns = get_entry_nouns(config);
        if entry_nouns.is_empty() {
            println!("[Pipeline] 加载到的其余入口 Noun 列表为空。");
        } else {
            println!("📌 补充入口 Noun 列表: {:?}", entry_nouns);

            // 收集根节点
            for entry in &entry_nouns {
                let noun_upper = entry.to_uppercase();
                let noun_str = noun_upper.as_str();

                // 跳过已处理的 BRAN/HANG
                if noun_str == "BRAN" || noun_str == "HANG" {
                    continue;
                }

                let refnos = query_noun_refnos(noun_str, &dbnums, config.index_tree_debug_limit_per_target_type).await?;
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
        }
    }

    if !all_roots.is_empty() {
        println!("[Pipeline] 其余根节点总数 {}", all_roots.len());
        let roots_vec: Vec<RefnoEnum> = all_roots.into_iter().collect();

            // 递归收集子节点
            collect_all_descendants(&roots_vec, &mut loop_refnos, &mut prim_refnos, &mut cate_refnos)
                .await?;

            // 两阶段（Prefetch -> Generate）：
            // - PrefetchThenGenerate：先把 LOOP/CATE/PRIM 输入预取到 geom_input_cache，再进入纯离线生成阶段消费缓存
            // - CacheOnly：不做预取，直接进入离线生成阶段（只读 cache，miss 按策略跳过并记录）
            let ctx_prefetch = NounProcessContext::new(
                db_option.clone(),
                config.batch_size.get(),
                config.concurrency.get(),
            )
            .with_stage(GenStage::Prefetch);

            // [foyer-removal] PrefetchThenGenerate 已移除，此分支不再触发
            if false {
                let loop_vec: Vec<RefnoEnum> = loop_refnos.iter().copied().collect();
                let prim_vec: Vec<RefnoEnum> = prim_refnos.iter().copied().collect();
                let cate_vec: Vec<RefnoEnum> = cate_refnos.iter().copied().collect();
                println!(
                    "[Pipeline] PrefetchThenGenerate: 开始预取 LOOP/CATE/PRIM 输入到 geom_input_cache (loop_refnos={}, cate_refnos={}, prim_refnos={})",
                    loop_vec.len(),
                    cate_vec.len(),
                    prim_vec.len()
                );

                // 全局 geom_input_cache 已在 orchestrator 初始化；这里再 init 一次保证 IndexTree 直调也可用。
                geom_input_cache::init_global_geom_input_cache();
                let _ = geom_input_cache::prefetch_all_geom_inputs(
                    ctx_prefetch.db_option.as_ref(),
                    &loop_vec,
                    &prim_vec,
                    &cate_vec,
                )
                .await?;

                // CATE prepared geos/ptset：预热 cata_resolve_cache（按 cata_hash）
                let mut target_cata_map_for_validate: Option<Arc<DashMap<String, aios_core::pdms_types::CataHashRefnoKV>>> = None;
                if !cate_vec.is_empty() {
                    println!(
                        "[Pipeline] PrefetchThenGenerate: 开始预热 cata_resolve_cache (cate_refnos={})",
                        cate_vec.len()
                    );
                    // PrefetchThenGenerate：此处必须严格成功。离线 Generate 不允许回查 DB；miss 视为流程不正确。
                    let target_cata_map = Arc::new(build_cata_hash_map_from_tree(&cate_vec).await?);
                    target_cata_map_for_validate = Some(target_cata_map.clone());
                    if !target_cata_map.is_empty() {
                        let outcome = cata_resolve_cache_pipeline::prefetch_cata_resolve_cache_for_target_map(
                            ctx_prefetch.db_option.clone(),
                            target_cata_map,
                        )
                        .await?;
                        if outcome.failed > 0 {
                            return Err(anyhow::anyhow!(
                                "cata_resolve_cache 预热失败：failed_groups={}（离线生成不允许 miss）",
                                outcome.failed
                            )
                            .into());
                        }
                    }
                }

                // PrefetchThenGenerate：预取完成后进行完整性校验；不通过则不进入离线生成阶段。
                geom_input_cache::ensure_geom_inputs_present_for_refnos_from_global(
                    &loop_vec,
                    &prim_vec,
                    &cate_vec,
                )
                .map_err(IndexTreeError::from)?;

                if let Some(target_cata_map) = target_cata_map_for_validate {
                    if !target_cata_map.is_empty() {
                        let cache_dir = ctx_prefetch.db_option.get_model_cache_dir().join("cata_resolve_cache");
                        cata_resolve_cache::init_global_cata_resolve_cache();
                        let Some(resolve_cache) = cata_resolve_cache::global_cata_resolve_cache() else {
                            return Err(anyhow::anyhow!("global_cata_resolve_cache 未初始化").into());
                        };

                        // 校验每个 cata_hash 是否已命中缓存；缺失直接失败（给出样例 key）。
                        const SAMPLE_LIMIT: usize = 16;
                        let mut missing_keys: Vec<String> = Vec::new();
                        for kv in target_cata_map.iter() {
                            let key = kv.key().clone();
                            drop(kv);
                            if resolve_cache.get(&key).is_none() {
                                missing_keys.push(key);
                            }
                        }
                        if !missing_keys.is_empty() {
                            let sample = missing_keys
                                .iter()
                                .take(SAMPLE_LIMIT)
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(", ");
                            return Err(anyhow::anyhow!(
                                "cata_resolve_cache 不完整：missing_keys={}, sample=[{}]（请先完成 Prefetch 预热）",
                                missing_keys.len(),
                                sample
                            )
                            .into());
                        }
                    }
                }
                println!(
                    "[Pipeline] PrefetchThenGenerate: 预取完成，进入离线生成阶段 (stage={})",
                    GenStage::Generate.as_str()
                );
            }

            let ctx = ctx_prefetch.with_stage(GenStage::Generate);

            // [1-3/4] 处理 LOOP, CATE, PRIM
            let (loop_vec, loop_dur) = process_loop_stage(
                &ctx,
                loop_refnos,
                config,
                &dbnums,
                &bran_generated_refnos,
                loop_sjus_map_arc.clone(),
                sender.clone(),
                has_seed_roots,
            )
            .await?;
            let (cate_vec, cate_dur) = process_cate_stage(
                &ctx,
                cate_refnos,
                config,
                &dbnums,
                &bran_generated_refnos,
                loop_sjus_map_arc,
                sender.clone(),
                has_seed_roots,
            )
            .await?;
            let (prim_vec, prim_dur) =
                process_prim_stage(&ctx, prim_refnos, config, &dbnums, sender.clone(), has_seed_roots)
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
            print_final_summary(total_duration, loop_dur, cate_dur, prim_dur, bran_duration);
    }

    categorized.print_statistics();
    Ok(categorized)
}

/// 内部核心逻辑：处理 BRAN/HANG 相关的 CATE 生成及 Tubing
#[cfg_attr(feature = "profile", tracing::instrument(skip_all, name = "bran_hang_core_logic"))]
async fn process_bran_hang_core_logic(
    ctx_prefetch: &NounProcessContext,
    ctx_generate: &NounProcessContext,
    bran_roots: &[RefnoEnum],
    loop_sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
    bran_generated_refnos: &mut HashSet<RefnoEnum>,
) -> Result<()> {
    if bran_roots.is_empty() {
        return Ok(());
    }
    let db_option = &ctx_prefetch.db_option;
    let phase_total = Instant::now();
    println!("📍 优先处理 BRAN/HANG 及其依赖 (count={})...", bran_roots.len());

    // ── 阶段 1: 收集子元素 ──
    let t1 = Instant::now();
    #[cfg(feature = "profile")]
    let _span1 = tracing::info_span!("bran_collect_children").entered();
    let branch_refnos_map: DashMap<RefnoEnum, Vec<SPdmsElement>> = DashMap::new();
    let mut total_children: usize = 0;
    for &refno in bran_roots {
        if let Ok(children) = TreeIndexManager::collect_children_elements_from_tree(refno).await {
            total_children += children.len();
            for child in &children {
                bran_generated_refnos.insert(child.refno);
            }
            if !children.is_empty() {
                branch_refnos_map.insert(refno, children);
            }
        }
    }
    #[cfg(feature = "profile")]
    drop(_span1);
    let t1_ms = t1.elapsed().as_millis();
    println!(
        "  [BRAN perf] 阶段1 collect_children: {} ms (roots={}, children={})",
        t1_ms, bran_roots.len(), total_children
    );

    // ── 阶段 2: 构建 cata_hash_map ──
    let t2 = Instant::now();
    #[cfg(feature = "profile")]
    let _span2 = tracing::info_span!("bran_build_cata_hash_map").entered();
    let child_refnos: Vec<RefnoEnum> = branch_refnos_map
        .iter()
        .flat_map(|entry| entry.value().iter().map(|c| c.refno).collect::<Vec<_>>())
        .collect();
    let target_bran_reuse_cata_map = if child_refnos.is_empty() {
        DashMap::new()
    } else {
        match build_cata_hash_map_from_tree(&child_refnos).await {
            Ok(m) => m,
            Err(e) => {
                // 离线 Generate 阶段不允许缺失 tree/db_meta（否则无法按 cata_hash 分组消费缓存）。
                // [foyer-removal] PrefetchThenGenerate 已移除
                if ctx_generate.is_offline_generate()
                {
                    return Err(e.into());
                }
                eprintln!(
                    "[BRAN/HANG] build_cata_hash_map_from_tree 失败（Direct 路径将跳过 CATE 生成）: {}",
                    e
                );
                DashMap::new()
            }
        }
    };
    let unique_cata_cnt = target_bran_reuse_cata_map.len();
    let target_bran_reuse_cata_map = Arc::new(target_bran_reuse_cata_map);
    #[cfg(feature = "profile")]
    drop(_span2);
    let t2_ms = t2.elapsed().as_millis();
    println!(
        "  [BRAN perf] 阶段2 build_cata_hash_map: {} ms (child_refnos={}, unique_cata={})",
        t2_ms, child_refnos.len(), unique_cata_cnt
    );

    // ── 阶段 3: Prefetch（仅 PrefetchThenGenerate） ──
    let t3 = Instant::now();
    #[cfg(feature = "profile")]
    let _span3 = tracing::info_span!("bran_prefetch_offline_inputs").entered();
    // [foyer-removal] PrefetchThenGenerate 已移除，此分支不再触发
    if false {
        prefetch_bran_hang_inputs_for_offline_generate(
            ctx_prefetch,
            bran_roots,
            &child_refnos,
            target_bran_reuse_cata_map.clone(),
        )
        .await?;
    }
    #[cfg(feature = "profile")]
    drop(_span3);
    let t3_ms = t3.elapsed().as_millis();
    println!("  [BRAN perf] 阶段3 prefetch_offline_inputs: {} ms", t3_ms);

    // ── 阶段 3.5: tubi_prefetch（已禁用） ──
    // 原逻辑 spawn 独立 task 建立第二个 SurrealDB 连接做逐条预取（1700+ 次查询），
    // 导致连接竞争/死锁。去掉后，阶段 6 内部的 P4 优化会在主连接上完成同样工作。
    let branch_tubi_prefetch_task: Option<
        tokio::task::JoinHandle<anyhow::Result<cata_model::BranchPrefetchResult>>,
    > = None;
    println!("  [BRAN perf] 阶段3.5 spawn_tubi_prefetch: 已跳过（由阶段6内部P4优化替代）");

    // ── 阶段 4: 生成 CATE 几何（Generate 阶段；离线时只读缓存） ──
    let t4 = Instant::now();
    #[cfg(feature = "profile")]
    let _span4 = tracing::info_span!("bran_generate_cate").entered();

    let mut cate_outcome = None;
    if !child_refnos.is_empty() {
        if ctx_generate.is_offline_generate() {
            // 离线 Generate：严格只读缓存（geom_input_cache + cata_resolve_cache）。
            let ranges = ctx_generate.bounded_chunks(child_refnos.len());
            for (i, (s, e)) in ranges.into_iter().enumerate() {
                let slice = &child_refnos[s..e];
                println!(
                    "  [BRAN][CATE][offline] 分页 {}/{} ({} ~ {})",
                    i + 1,
                    (child_refnos.len() + ctx_generate.batch_size.max(1) - 1)
                        / ctx_generate.batch_size.max(1),
                    s + 1,
                    e
                );
                process_cate_refno_page(
                    ctx_generate,
                    loop_sjus_map_arc.clone(),
                    sender.clone(),
                    slice,
                )
                .await?;
            }
        } else {
            // Direct：复用旧逻辑（允许 DB 查询与 local_al_map/tubi_info 收集）
            cate_outcome = Some(
                cata_model::gen_cata_instances(
                    db_option.clone(),
                    target_bran_reuse_cata_map.clone(),
                    loop_sjus_map_arc.clone(),
                    sender.clone(),
                )
                .await?,
            );
        }
    }

    #[cfg(feature = "profile")]
    drop(_span4);
    let t4_ms = t4.elapsed().as_millis();
    if let Some(ref outcome) = cate_outcome {
        println!(
            "  [BRAN perf] 阶段4 gen_cata_instances: {} ms (unique_cata={}, elapsed_inner={} ms)",
            t4_ms, outcome.unique_cata_cnt, outcome.elapsed_ms
        );
        for (k, v) in &outcome.time_stats {
            println!("    [BRAN perf]   cata_time.{}: {} ms", k, v);
        }
    } else {
        println!("  [BRAN perf] 阶段4 gen_cata_instances: {} ms (offline_or_skipped)", t4_ms);
    }

    // ── 阶段 5: 保存 tubi_info（仅 Direct 且 use_surrealdb=true） ──
    let t5 = Instant::now();
    #[cfg(feature = "profile")]
    let _span5 = tracing::info_span!("bran_save_tubi_info").entered();
    if db_option.use_surrealdb {
        if let Some(ref outcome) = cate_outcome {
            let _ = pdms_inst::save_tubi_info_batch(&outcome.tubi_info_map).await;
        }
    }
    #[cfg(feature = "profile")]
    drop(_span5);
    let t5_ms = t5.elapsed().as_millis();
    println!("  [BRAN perf] 阶段5 save_tubi_info: {} ms", t5_ms);

    // ── 阶段 6: 生成 Tubing（Generate 阶段；离线时 cache-only） ──
    let t6 = Instant::now();
    #[cfg(feature = "profile")]
    let _span6 = tracing::info_span!("bran_gen_branch_tubi").entered();
    let local_al_map = cate_outcome
        .as_ref()
        .map(|o| o.local_al_map.clone())
        .unwrap_or_else(|| Arc::new(DashMap::new()));

    let tubi_result = if ctx_generate.is_offline_generate() {
        cata_model::gen_branch_tubi_cache_only(
            db_option.clone(),
            Arc::new(branch_refnos_map),
            loop_sjus_map_arc,
            sender,
            local_al_map,
        )
        .await
    } else {
        let t_prefetch_wait = Instant::now();
        let mut external_prefetch = None;
        let mut external_prefetch_wait_ms = None;
        if let Some(handle) = branch_tubi_prefetch_task {
            match handle.await {
                Ok(Ok(prefetch)) => {
                    let wait_ms = t_prefetch_wait.elapsed().as_millis();
                    external_prefetch_wait_ms = Some(wait_ms);
                    println!(
                        "  [BRAN perf] 阶段6 使用外部 tubi_prefetch: wait={} ms, tubi_cache_hit={}, branch_meta_hit={}",
                        wait_ms,
                        prefetch.tubi_size_cache.len(),
                        prefetch.branch_meta_cache.len()
                    );
                    external_prefetch = Some(prefetch);
                }
                Ok(Err(e)) => {
                    external_prefetch_wait_ms = Some(t_prefetch_wait.elapsed().as_millis());
                    eprintln!(
                        "  [BRAN perf] 阶段6 外部 tubi_prefetch 失败，将回退内部预取: {}",
                        e
                    );
                }
                Err(e) => {
                    external_prefetch_wait_ms = Some(t_prefetch_wait.elapsed().as_millis());
                    eprintln!(
                        "  [BRAN perf] 阶段6 外部 tubi_prefetch 任务异常，将回退内部预取: {}",
                        e
                    );
                }
            }
        }

        cata_model::gen_branch_tubi_from_db_with_prefetch(
            db_option.clone(),
            Arc::new(branch_refnos_map),
            loop_sjus_map_arc,
            sender,
            local_al_map,
            external_prefetch,
            external_prefetch_wait_ms,
        )
        .await
    };
    #[cfg(feature = "profile")]
    drop(_span6);
    let t6_ms = t6.elapsed().as_millis();
    if let Ok(ref tubi_outcome) = tubi_result {
        println!(
            "  [BRAN perf] 阶段6 gen_branch_tubi: {} ms (tubi_count={}, elapsed_inner={} ms)",
            t6_ms, tubi_outcome.tubi_count, tubi_outcome.elapsed_ms
        );
        for (k, v) in &tubi_outcome.time_stats {
            println!("    [BRAN perf]   tubi_time.{}: {} ms", k, v);
        }
    } else {
        println!(
            "  [BRAN perf] 阶段6 gen_branch_tubi: {} ms (result={:?})",
            t6_ms,
            tubi_result.err()
        );
    }

    // ── 汇总 ──
    let total_ms = phase_total.elapsed().as_millis();
    println!(
        "  [BRAN perf] 总计: {} ms [collect={}ms, cata_hash={}ms, prefetch={}ms, cata_gen={}ms, tubi_info={}ms, tubi_gen={}ms]",
        total_ms, t1_ms, t2_ms, t3_ms, t4_ms, t5_ms, t6_ms
    );

    Ok(())
}

fn make_meta_axis_param(
    refno: RefnoEnum,
    number: i32,
    pt: Vec3,
    dir: Option<Vec3>,
    pbore: f32,
    pwidth: f32,
    pheight: f32,
) -> CateAxisParam {
    let dir_flag = if dir.is_some() { 1.0 } else { 0.0 };
    CateAxisParam {
        refno,
        number,
        pt: RsVec3(pt),
        dir: dir.map(RsVec3),
        dir_flag,
        ref_dir: None,
        pbore,
        pwidth,
        pheight,
        pconnect: String::new(),
    }
}

fn tubi_size_to_axis_fields(size: &TubiSize) -> (f32, f32, f32) {
    match size {
        TubiSize::BoreSize(b) => (*b, 0.0, 0.0),
        TubiSize::BoxSize((h, w)) => (0.0, *w, *h),
        _ => (0.0, 0.0, 0.0),
    }
}

async fn insert_inst_info_into_instance_cache(
    db_option: &DbOptionExt,
    inst_infos: HashMap<RefnoEnum, EleGeosInfo>,
) -> Result<()> {
    if inst_infos.is_empty() {
        return Ok(());
    }

    db_meta().ensure_loaded()?;
    let cache_dir = db_option.get_model_cache_dir();
    let cache_manager = InstanceCacheManager::new(&cache_dir).await?;

    // 将 inst_info 按 dbnum 分桶写入 instance_cache（ref0 != dbnum）。
    let mut per_db: HashMap<u32, ShapeInstancesData> = HashMap::new();
    for (refno, info) in inst_infos {
        let Some(dbnum) = db_meta().get_dbnum_by_refno(refno) else {
            return Err(anyhow::anyhow!("缺少 ref0->dbnum 映射: refno={}", refno).into());
        };
        if dbnum == 0 {
            return Err(anyhow::anyhow!("无效 dbnum=0（ref0->dbnum 映射缺失）: refno={}", refno).into());
        }
        per_db
            .entry(dbnum)
            .or_insert_with(ShapeInstancesData::default)
            .insert_info(refno, info);
    }

    for (dbnum, shape) in per_db {
        let _batch_id = cache_manager.insert_from_shape(dbnum, &shape);
    }

    Ok(())
}

/// 严格校验：指定 refnos 的 inst_info 必须已写入 instance_cache。
///
/// 语义：PrefetchThenGenerate 下，Generate 阶段不允许再回查 DB；因此 inst_info miss
/// 必须在 Prefetch 阶段立刻失败，便于定位“哪个 refno 没被写进 cache”。
async fn ensure_inst_info_present_in_instance_cache(
    db_option: &DbOptionExt,
    refnos: &[RefnoEnum],
) -> Result<()> {
    if refnos.is_empty() {
        return Ok(());
    }

    db_meta().ensure_loaded()?;
    let cache_dir = db_option.get_model_cache_dir();
    let cache = InstanceCacheManager::new(&cache_dir).await?;

    // 按 dbnum 分组；ref0 != dbnum，必须走 db_meta 映射。
    let mut groups: HashMap<u32, Vec<RefnoEnum>> = HashMap::new();
    for &r in refnos {
        let Some(dbnum) = db_meta().get_dbnum_by_refno(r) else {
            return Err(anyhow::anyhow!("缺少 ref0->dbnum 映射: refno={}", r).into());
        };
        if dbnum == 0 {
            return Err(anyhow::anyhow!("无效 dbnum=0（ref0->dbnum 映射缺失）: refno={}", r).into());
        }
        groups.entry(dbnum).or_default().push(r);
    }

    for (dbnum, want) in groups {
        let mut missing: Vec<RefnoEnum> = Vec::new();
        for &r in &want {
            if cache.get_inst_info(dbnum, r).await.is_none() {
                missing.push(r);
            }
        }
        if !missing.is_empty() {
            missing.sort_by_key(|r| r.refno());
            const SAMPLE_LIMIT: usize = 16;
            let sample = missing
                .iter()
                .take(SAMPLE_LIMIT)
                .map(|r| r.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(anyhow::anyhow!(
                "instance_cache inst_info 不完整: dbnum={} missing={} sample=[{}] dir={}",
                dbnum,
                missing.len(),
                sample,
                cache_dir.display()
            )
            .into());
        }
    }

    Ok(())
}

/// BRAN/HANG 离线 Generate 的 Prefetch 阶段：将生成热路径需要的输入填满到 model cache。
///
/// 目标：Generate 阶段严格只读 cache（geom_input_cache/cata_resolve_cache/transform_cache/instance_cache）。
async fn prefetch_bran_hang_inputs_for_offline_generate(
    ctx_prefetch: &NounProcessContext,
    bran_roots: &[RefnoEnum],
    child_refnos: &[RefnoEnum],
    target_cata_map: Arc<DashMap<String, aios_core::pdms_types::CataHashRefnoKV>>,
) -> Result<()> {
    if bran_roots.is_empty() && child_refnos.is_empty() {
        return Ok(());
    }

    // 0) 预取 transform（BRAN roots + 子元件）
    let mut transform_targets: Vec<RefnoEnum> = Vec::new();
    transform_targets.extend_from_slice(bran_roots);
    transform_targets.extend_from_slice(child_refnos);
    transform_targets.sort_by_key(|r| r.refno());
    transform_targets.dedup();

    if !transform_targets.is_empty() {
        let _ = transform_cache::get_world_transforms_cache_first_batch(
            Some(ctx_prefetch.db_option.as_ref()),
            &transform_targets,
        )
        .await?;
        // 严格校验：Generate cache-only 阶段不允许 miss
        transform_cache::ensure_world_transforms_present(ctx_prefetch.db_option.as_ref(), &transform_targets)
            .await?;
    }

    // 1) 预取 CATE inputs（child_refnos）
    if !child_refnos.is_empty() {
        geom_input_cache::init_global_geom_input_cache();
        let empty: Vec<RefnoEnum> = Vec::new();
        let _ = geom_input_cache::prefetch_all_geom_inputs(
            ctx_prefetch.db_option.as_ref(),
            &empty,
            &empty,
            child_refnos,
        )
        .await?;
        geom_input_cache::ensure_geom_inputs_present_for_refnos_from_global(&empty, &empty, child_refnos)
            .map_err(IndexTreeError::from)?;
    }

    // 2) 预热 cata_resolve_cache（按 cata_hash）
    if !target_cata_map.is_empty() {
        let outcome = cata_resolve_cache_pipeline::prefetch_cata_resolve_cache_for_target_map(
            ctx_prefetch.db_option.clone(),
            target_cata_map.clone(),
        )
        .await?;
        if outcome.failed > 0 {
            return Err(anyhow::anyhow!(
                "cata_resolve_cache 预热失败：failed_groups={}（离线生成不允许 miss）",
                outcome.failed
            )
            .into());
        }

        let cache_dir = ctx_prefetch.db_option.get_model_cache_dir().join("cata_resolve_cache");
        cata_resolve_cache::init_global_cata_resolve_cache();
        let Some(resolve_cache) = cata_resolve_cache::global_cata_resolve_cache() else {
            return Err(anyhow::anyhow!("global_cata_resolve_cache 未初始化").into());
        };

        // 校验每个 cata_hash 均已命中缓存。
        const SAMPLE_LIMIT: usize = 16;
        let mut missing_keys: Vec<String> = Vec::new();
        for kv in target_cata_map.iter() {
            let key = kv.key().clone();
            drop(kv);
            if resolve_cache.get(&key).is_none() {
                missing_keys.push(key);
            }
        }
        if !missing_keys.is_empty() {
            let sample = missing_keys
                .iter()
                .take(SAMPLE_LIMIT)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            return Err(anyhow::anyhow!(
                "cata_resolve_cache 不完整：missing_keys={}, sample=[{}]",
                missing_keys.len(),
                sample
            )
            .into());
        }

        // 3) 将 BRAN/HANG meta（HPOS/TPOS/尺寸/类型）写入 instance_cache.inst_info_map
        //    以及将子元件 ptset_map 写入 instance_cache（用于 cache-only tubi 生成）。
        db_meta().ensure_loaded()?;
        geom_input_cache::init_global_geom_input_cache();
        let cate_inputs = geom_input_cache::load_cate_inputs_for_refnos_from_global(child_refnos)?;

        let mut inst_infos: HashMap<RefnoEnum, EleGeosInfo> = HashMap::new();

        // 3.1) 子元件 inst_info（ptset_map 来自 cata_resolve_cache；owner/transform 来自 geom_input_cache）
        for kv in target_cata_map.iter() {
            let cata_hash = kv.key().clone();
            let group_refnos = kv.value().group_refnos.clone();
            drop(kv);

            let Some(resolved_comp) = resolve_cache.get(&cata_hash) else {
                return Err(anyhow::anyhow!(
                    "cata_resolve_cache miss（已校验仍缺失）：cata_hash={}",
                    cata_hash
                )
                .into());
            };

            let ptset_map: BTreeMap<i32, CateAxisParam> = resolved_comp.ptset_map();
            let has_solid = resolved_comp.has_solid;

            for &r in &group_refnos {
                let Some(input) = cate_inputs.get(&r) else {
                    return Err(anyhow::anyhow!(
                        "geom_input_cache miss（已 ensure 仍缺失）：refno={}, cata_hash={}",
                        r,
                        cata_hash
                    )
                    .into());
                };
                inst_infos.insert(
                    r,
                    EleGeosInfo {
                        refno: r,
                        sesno: input.attmap.sesno(),
                        owner_refno: input.owner_refno,
                        owner_type: input.owner_type.clone(),
                        cata_hash: Some(cata_hash.clone()),
                        visible: input.visible,
                        ptset_map: ptset_map.clone(),
                        is_solid: has_solid,
                        world_transform: input.world_transform,
                        ..Default::default()
                    },
                );
            }
        }

        // 3.2) BRAN/HANG meta inst_info
        for &branch_refno in bran_roots {
            let att = aios_core::get_named_attmap(branch_refno).await?;
            let is_hang = att.get_type_str() == "HANG";

            let hpos = att
                .get_vec3("HPOS")
                .ok_or_else(|| anyhow::anyhow!("BRAN/HANG 缺少 HPOS: refno={}", branch_refno))?;
            let tpos = att
                .get_vec3("TPOS")
                .ok_or_else(|| anyhow::anyhow!("BRAN/HANG 缺少 TPOS: refno={}", branch_refno))?;
            let hdir = att
                .get_vec3("HDIR")
                .ok_or_else(|| anyhow::anyhow!("BRAN/HANG 缺少 HDIR: refno={}", branch_refno))?;
            let tdir = att
                .get_vec3("TDIR")
                .ok_or_else(|| anyhow::anyhow!("BRAN/HANG 缺少 TDIR: refno={}", branch_refno))?;

            // BRAN/HANG world_transform 必须已在 transform_cache 中命中（上面已 ensure）
            let world_transform =
                transform_cache::get_world_transform_cache_only(ctx_prefetch.db_option.as_ref(), branch_refno)
                    .await?;

            let owner_refno = att.get_owner();
            let owner_type = aios_core::get_type_name(owner_refno)
                .await
                .unwrap_or_default();

            // tubi_size：沿用旧逻辑（HSTU/HREF -> CATR -> query_tubi_size）
            let h_ref = att
                .get_foreign_refno(if is_hang { "HREF" } else { "HSTU" })
                .unwrap_or_default();
            if !h_ref.is_valid() {
                return Err(anyhow::anyhow!(
                    "BRAN/HANG 缺少 HREF/HSTU（无法推导 tubi_size）: refno={}",
                    branch_refno
                )
                .into());
            }
            let tubi_att = aios_core::get_named_attmap(h_ref).await?;
            let catr = tubi_att.get_foreign_refno("CATR").unwrap_or_default();
            if !catr.is_valid() {
                return Err(anyhow::anyhow!(
                    "BRAN/HANG 缺少 CATR（无法推导 tubi_size）: refno={}, h_ref={}",
                    branch_refno,
                    h_ref
                )
                .into());
            }
            let tubi_size = crate::fast_model::query_tubi_size(branch_refno, catr, is_hang).await?;
            if matches!(tubi_size, TubiSize::None) {
                return Err(anyhow::anyhow!(
                    "BRAN/HANG tubi_size 为 None（离线生成不允许 miss）: refno={}",
                    branch_refno
                )
                .into());
            }

            let mut ptset_map: BTreeMap<i32, CateAxisParam> = BTreeMap::new();
            ptset_map.insert(
                crate::fast_model::cata_model::BRANCH_META_HPOS_NO,
                make_meta_axis_param(
                    branch_refno,
                    crate::fast_model::cata_model::BRANCH_META_HPOS_NO,
                    hpos,
                    Some(hdir),
                    0.0,
                    0.0,
                    0.0,
                ),
            );
            ptset_map.insert(
                crate::fast_model::cata_model::BRANCH_META_TPOS_NO,
                make_meta_axis_param(
                    branch_refno,
                    crate::fast_model::cata_model::BRANCH_META_TPOS_NO,
                    tpos,
                    Some(tdir),
                    0.0,
                    0.0,
                    0.0,
                ),
            );
            let (pbore, pwidth, pheight) = tubi_size_to_axis_fields(&tubi_size);
            ptset_map.insert(
                crate::fast_model::cata_model::BRANCH_META_SIZE_NO,
                make_meta_axis_param(
                    branch_refno,
                    crate::fast_model::cata_model::BRANCH_META_SIZE_NO,
                    Vec3::ZERO,
                    None,
                    pbore,
                    pwidth,
                    pheight,
                ),
            );
            ptset_map.insert(
                crate::fast_model::cata_model::BRANCH_META_KIND_NO,
                make_meta_axis_param(
                    branch_refno,
                    crate::fast_model::cata_model::BRANCH_META_KIND_NO,
                    Vec3::ZERO,
                    None,
                    if is_hang { 1.0 } else { 0.0 },
                    0.0,
                    0.0,
                ),
            );

            inst_infos.insert(
                branch_refno,
                EleGeosInfo {
                    refno: branch_refno,
                    sesno: att.sesno(),
                    owner_refno,
                    owner_type,
                    visible: true,
                    ptset_map,
                    world_transform,
                    ..Default::default()
                },
            );
        }

        insert_inst_info_into_instance_cache(ctx_prefetch.db_option.as_ref(), inst_infos).await?;

        // 写入后立即回读校验，保证后续 cache-only tubing/生成不出现 inst_info miss。
        let mut to_check: Vec<RefnoEnum> = Vec::new();
        to_check.extend_from_slice(bran_roots);
        to_check.extend_from_slice(child_refnos);
        ensure_inst_info_present_in_instance_cache(ctx_prefetch.db_option.as_ref(), &to_check).await?;
    }

    Ok(())
}

async fn process_loop_stage(
    ctx: &NounProcessContext,
    loop_refnos: HashSet<RefnoEnum>,
    config: &IndexTreeConfig,
    dbnums: &[u32],
    _bran_generated_refnos: &HashSet<RefnoEnum>,
    loop_sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
    use_input_roots: bool,
) -> Result<(Vec<RefnoEnum>, Duration)> {
    let start = Instant::now();
    if use_input_roots {
        let mut vec: Vec<RefnoEnum> = loop_refnos.into_iter().collect();
        vec.sort_by_key(|r| r.to_string());
        let ranges = ctx.bounded_chunks(vec.len());
        for (page_idx, (s, e)) in ranges.into_iter().enumerate() {
            let slice = &vec[s..e];
            println!(
                "[Loop][seed_roots] 处理第 {} 页 ({} ~ {})",
                page_idx + 1,
                s + 1,
                e
            );
            process_loop_refno_page(ctx, loop_sjus_map_arc.clone(), sender.clone(), slice)
                .await
                .map_err(|e| {
                    IndexTreeError::GeometryGenerationFailed("loop(seed_roots)".to_string(), e.to_string())
                })?;
        }
        return Ok((vec, start.elapsed()));
    }

    let mut loop_noun_infos = prequery_noun_counts(&GNERAL_LOOP_OWNER_NOUN_NAMES, dbnums).await?;
    // IndexTreeConfig.enabled_categories 的语义：空=全启用；否则按类别/具体 noun 精确过滤。
    loop_noun_infos.retain(|info| config.should_process_noun(info.noun, "loop"));
    loop_noun_infos.retain(|info| info.count > 0);

    let vec =
        process_nouns_by_type(loop_noun_infos, ctx, NounCategoryType::Loop, loop_sjus_map_arc, sender)
            .await?;
    Ok((vec, start.elapsed()))
}

async fn process_prim_stage(
    ctx: &NounProcessContext,
    refnos: HashSet<RefnoEnum>,
    config: &IndexTreeConfig,
    dbnums: &[u32],
    sender: flume::Sender<ShapeInstancesData>,
    use_input_roots: bool,
) -> Result<(Vec<RefnoEnum>, Duration)> {
    let start = Instant::now();
    if use_input_roots {
        let mut vec: Vec<RefnoEnum> = refnos.into_iter().collect();
        vec.sort_by_key(|r| r.to_string());
        let ranges = ctx.bounded_chunks(vec.len());
        for (page_idx, (s, e)) in ranges.into_iter().enumerate() {
            let slice = &vec[s..e];
            println!(
                "[Prim][seed_roots] 处理第 {} 页 ({} ~ {})",
                page_idx + 1,
                s + 1,
                e
            );
            process_prim_refno_page(ctx, sender.clone(), slice)
                .await
                .map_err(|e| {
                    IndexTreeError::GeometryGenerationFailed("prim(seed_roots)".to_string(), e.to_string())
                })?;
        }
        return Ok((vec, start.elapsed()));
    }

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
    refnos: HashSet<RefnoEnum>,
    config: &IndexTreeConfig,
    dbnums: &[u32],
    bran_generated_refnos: &HashSet<RefnoEnum>,
    loop_sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
    use_input_roots: bool,
) -> Result<(Vec<RefnoEnum>, Duration)> {
    let start = Instant::now();
    if use_input_roots {
        let mut vec: Vec<RefnoEnum> = refnos
            .into_iter()
            .filter(|r| !bran_generated_refnos.contains(r))
            .collect();
        vec.sort_by_key(|r| r.to_string());
        let ranges = ctx.bounded_chunks(vec.len());
        for (page_idx, (s, e)) in ranges.into_iter().enumerate() {
            let slice = &vec[s..e];
            println!(
                "[Cate][seed_roots] 处理第 {} 页 ({} ~ {})",
                page_idx + 1,
                s + 1,
                e
            );
            process_cate_refno_page(ctx, loop_sjus_map_arc.clone(), sender.clone(), slice)
                .await
                .map_err(|e| {
                    IndexTreeError::GeometryGenerationFailed("cate(seed_roots)".to_string(), e.to_string())
                })?;
        }
        return Ok((vec, start.elapsed()));
    }

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

fn print_final_summary(total: Duration, l: Duration, c: Duration, p: Duration, b: Duration) {
    println!("✅ IndexTree 处理完成 (GeneralPath)");
    println!(
        "⏱️  Total: {} ms [L: {}ms, C: {}ms, P: {}ms, B: {}ms]",
        total.as_millis(),
        l.as_millis(),
        c.as_millis(),
        p.as_millis(),
        b.as_millis()
    );
}

async fn get_filtered_dbnums(db_option: &DbOptionExt) -> Result<Vec<u32>> {
    let mut dbnums: Vec<u32> = if let Some(manual) = db_option.manual_db_nums.clone() {
        manual
    } else {
        // 固定策略：优先走本地 db_meta（由 scene_tree 阶段产出），避免对 MDB 表的依赖。
        let mut from_meta = Vec::new();
        match db_meta().ensure_loaded() {
            Ok(_) => {
                from_meta = db_meta().get_all_dbnums();
            }
            Err(e) => {
                log::warn!(
                    "[IndexTree] 加载 db_meta_info.json 失败，尝试从 SurrealDB(pe) 获取 dbnum: {}",
                    e
                );
            }
        }

        if from_meta.is_empty() {
            return Err(IndexTreeError::DatabaseError(
                "db_meta_info.json 为空且无其他 dbnum 来源，请先运行 --parse-db".to_string(),
            ));
        } else {
            from_meta
        }
    };

    dbnums.sort_unstable();
    dbnums.dedup();

    if let Some(exclude) = &db_option.exclude_db_nums {
        dbnums.retain(|dbnum| !exclude.contains(dbnum));
    }
    Ok(dbnums)
}

fn get_entry_nouns(config: &IndexTreeConfig) -> Vec<String> {
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

#[cfg_attr(feature = "profile", tracing::instrument(skip_all, name = "query_noun_refnos"))]
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
        .map_err(|e| IndexTreeError::DatabaseError(format!("加载 db_meta_info.json 失败: {}", e)))?;
    let mut all_dbnums = db_meta().get_all_dbnums();
    if all_dbnums.is_empty() {
        return Err(IndexTreeError::DatabaseError(
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
        .map_err(|e| IndexTreeError::DatabaseError(format!("collect_descendant_filter_ids(loop) failed: {}", e)))?;
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
        .map_err(|e| IndexTreeError::DatabaseError(format!("collect_descendant_filter_ids(prim) failed: {}", e)))?;
    track_refno_issues(&prim_descendants, "prim_descendants", RefnoErrorStage::Query);
    prim_refnos.extend(prim_descendants);

    let cate_descendants =
        query_provider::query_multi_descendants_with_self(roots, &USE_CATE_NOUN_NAMES, true)
        .await
        .map_err(|e| IndexTreeError::DatabaseError(format!("collect_descendant_filter_ids(cate) failed: {}", e)))?;
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
        let config = IndexTreeConfig::default();

        // 默认配置下，空 map 会警告但不报错
        let result = validate_sjus_map(&sjus_map, &config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_sjus_map_empty_strict() {
        let sjus_map = DashMap::new();
        let config = IndexTreeConfig::default().with_strict_validation(true);

        // 严格模式下，空 map 会报错
        let result = validate_sjus_map(&sjus_map, &config);
        assert!(result.is_err());

        if let Err(IndexTreeError::EmptySjusMap) = result {
            // 正确
        } else {
            panic!("Expected EmptySjusMap error");
        }
    }

    // #[test]
    // fn test_validate_sjus_map_with_data() {
    //     let sjus_map = DashMap::new();
    //     sjus_map.insert(RefnoEnum::RefU64(1), (Vec3::ZERO, 1.0));

    //     let config = IndexTreeConfig::default().with_strict_validation(true);

    //     // 有数据时不应报错
    //     let result = validate_sjus_map(&sjus_map, &config);
    //     assert!(result.is_ok());
    // }
}


