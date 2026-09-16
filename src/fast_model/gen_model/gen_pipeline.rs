use crate::generation_read::HierarchySnapshot;
use crate::options::DbOptionExt;
use aios_core::RefnoEnum;
use aios_core::geometry::ShapeInstancesData;

use aios_core::pdms_types::{
    BRAN_COMPONENT_NOUN_NAMES, GNERAL_LOOP_OWNER_NOUN_NAMES, GNERAL_PRIM_NOUN_NAMES,
    USE_CATE_NOUN_NAMES,
};
use aios_core::pe::SPdmsElement;
use aios_core::tool::db_tool::db1_hash;
use dashmap::DashMap;
use glam::Vec3;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

const DATUM_MARKER_NOUN_NAMES: [&str; 2] = ["JLDATU", "PLDATU"];

fn prim_noun_names() -> Vec<&'static str> {
    GNERAL_PRIM_NOUN_NAMES
        .iter()
        .copied()
        .chain(DATUM_MARKER_NOUN_NAMES)
        .collect()
}
use super::cate_processor::process_cate_refno_page;
use super::categorized_refnos::CategorizedRefnos;
use super::config::{GenPipelineConfig, GenerationContract};
use super::context::{GenerationReadContext, NounProcessContext};
use super::errors::{GenPipelineError, Result};
use super::loop_processor::process_loop_refno_page;
use super::model_writer::GenerationArtifacts;
use super::noun_collection::GenerationTargets;
use super::prim_processor::process_prim_refno_page;
use super::utilities::build_cata_hash_map_from_session;
use crate::data_interface::db_meta;
use crate::data_interface::increment_record::IncrGeoUpdateLog;
use crate::fast_model::cata_model;

// Performance profiling support
#[cfg(feature = "profile")]
use tracing::{info, instrument};

/// 验证 SJUS map 是否完整
///
/// 根据配置决定是否警告或报错
pub fn validate_sjus_map(
    sjus_map: &DashMap<RefnoEnum, (Vec3, f32)>,
    config: &GenPipelineConfig,
) -> Result<()> {
    if config.validate_sjus_map && sjus_map.is_empty() {
        let warning = "⚠️ SJUS map 为空，几何体生成可能产生不正确的结果";

        if config.strict_validation {
            log::error!("{}", warning);
            return Err(GenPipelineError::EmptySjusMap);
        } else {
            log::warn!("{}", warning);
            log::warn!("  提示：如果这是预期行为，可以在配置中禁用 validate_sjus_map");
        }
    }
    Ok(())
}

/// GenPipeline下生成所有几何体（优化版本）
///
/// # 主要改进
/// 1. ✅ BRAN/HANG 优先处理：先处理 BRAN/HANG 及其依赖，记录已生成的子节点
/// 2. ✅ 顺序执行：LOOP -> CATE -> PRIM（确保依赖关系正确）
/// 3. ✅ 批量并发：每个类别内部使用批量并发处理
/// 4. ✅ 内存优化：使用 CategorizedRefnos 替代三个 HashSet
/// 5. ✅ 数据验证：检查 SJUS map 完整性
/// 6. ✅ 类型安全：使用 GenPipelineConfig 和错误类型
///
/// # 执行顺序
/// BRAN/HANG 优先 -> LOOP -> CATE -> PRIM（跳过已生成的 refno）
#[cfg_attr(
    feature = "profile",
    instrument(skip(db_option, generation_read, config, sender))
)]
pub async fn gen_pipeline_geos(
    db_option: Arc<DbOptionExt>,
    generation_read: Arc<GenerationReadContext>,
    config: &GenPipelineConfig,
    sender: flume::Sender<ShapeInstancesData>,
    seed_roots: Option<Vec<RefnoEnum>>,
    generation_artifacts: Option<Arc<GenerationArtifacts>>,
) -> Result<CategorizedRefnos> {
    let targets = match seed_roots {
        Some(roots) => resolve_root_generation_targets(&generation_read.hierarchy, config, &roots)?,
        None => {
            resolve_full_generation_targets(&db_option, &generation_read.hierarchy, config).await?
        }
    };
    let generation_contract = Arc::new(GenerationContract::from_db_option(&db_option, config));
    execute_generation_targets(
        db_option,
        generation_read,
        config,
        generation_contract,
        sender,
        &targets,
        generation_artifacts,
    )
    .await
}

pub(crate) async fn resolve_full_generation_targets(
    db_option: &DbOptionExt,
    hierarchy: &HierarchySnapshot,
    config: &GenPipelineConfig,
) -> Result<GenerationTargets> {
    let dbnums: BTreeSet<u32> = get_filtered_dbnums(db_option).await?.into_iter().collect();
    let include_bran_hang = should_include_bran_hang(config);
    let candidates = hierarchy
        .all_refnos()
        .into_iter()
        .filter(|refno| {
            hierarchy
                .node(*refno)
                .is_some_and(|node| dbnums.contains(&node.dbnum))
        })
        .collect();
    targets_from_candidates(hierarchy, config, candidates, include_bran_hang)
}

pub(crate) fn resolve_root_generation_targets(
    hierarchy: &HierarchySnapshot,
    config: &GenPipelineConfig,
    roots: &[RefnoEnum],
) -> Result<GenerationTargets> {
    let include_bran_hang = should_include_bran_hang(config);
    let target_nouns = configured_target_nouns(config, include_bran_hang);
    let roots: Vec<_> = roots.iter().copied().filter(RefnoEnum::is_valid).collect();
    if roots.is_empty() {
        return Ok(GenerationTargets::new([], [], [], [], []));
    }

    let query = crate::generation_read::HierarchyQuery {
        include_self: true,
        nouns: target_nouns,
        max_depth: None,
        prune_on_match: false,
    };
    let candidates = hierarchy
        .descendants(&roots, &query)
        .map_err(anyhow::Error::new)?;
    targets_from_candidates(hierarchy, config, candidates, include_bran_hang)
}

/// 变更根按后代展开，口径必须与 `pre_cleanup_for_regen_versioned` 一致。
///
/// cleanup 会删掉每个目标 refno **及其全部后代**的旧产物；若这里只重算根自身，
/// 被删的子件就不会被重建（典型：只改 EQUI 的 POS，其子 PRIM 的模型产物被清掉
/// 却无人写回），而这一轮仍会以 `Ok` 收尾、推进 model_gen 水位并消费欠账，
/// 下一轮增量不再重试。
///
/// 新切面里已不存在的根（区间内先增后删、或已删除）先过滤掉：`descendants()`
/// 对缺席根返回 `MissingRequiredData`，而它们的旧产物由 `delete_refnos` 负责清理。
pub(crate) fn resolve_incremental_generation_targets(
    hierarchy: &HierarchySnapshot,
    config: &GenPipelineConfig,
    log: &IncrGeoUpdateLog,
) -> Result<GenerationTargets> {
    let present_roots = log
        .get_all_visible_refnos()
        .into_iter()
        .filter(|refno| hierarchy.node(*refno).is_some())
        .collect::<Vec<_>>();
    let generated = resolve_root_generation_targets(hierarchy, config, &present_roots)?;
    Ok(GenerationTargets::new(
        generated.bran_hang_refnos().iter().copied(),
        generated.loop_refnos().iter().copied(),
        generated.cate_refnos().iter().copied(),
        generated.prim_refnos().iter().copied(),
        log.delete_refnos.iter().copied(),
    ))
}

fn should_include_bran_hang(config: &GenPipelineConfig) -> bool {
    config.enabled_categories.is_empty()
        || config
            .enabled_categories
            .iter()
            .any(|value| value.eq_ignore_ascii_case("BRAN") || value.eq_ignore_ascii_case("HANG"))
}

fn configured_target_nouns(
    config: &GenPipelineConfig,
    include_bran_hang: bool,
) -> BTreeSet<String> {
    let mut nouns: BTreeSet<String> = get_entry_nouns(config)
        .into_iter()
        .map(|noun| noun.to_uppercase())
        .collect();
    if include_bran_hang {
        nouns.insert("BRAN".to_string());
        nouns.insert("HANG".to_string());
    }
    nouns
}

fn targets_from_candidates(
    hierarchy: &HierarchySnapshot,
    config: &GenPipelineConfig,
    candidates: Vec<RefnoEnum>,
    include_bran_hang: bool,
) -> Result<GenerationTargets> {
    let target_nouns = configured_target_nouns(config, include_bran_hang);
    let mut grouped: HashMap<String, Vec<RefnoEnum>> = HashMap::new();
    for refno in candidates.into_iter().filter(RefnoEnum::is_valid) {
        let Some(node) = hierarchy.node(refno) else {
            continue;
        };
        let noun = node.noun.to_uppercase();
        if target_nouns.contains(&noun) {
            grouped.entry(noun).or_default().push(refno);
        }
    }

    let loop_hashes: HashSet<u32> = GNERAL_LOOP_OWNER_NOUN_NAMES
        .iter()
        .map(|noun| db1_hash(noun))
        .collect();
    let prim_hashes: HashSet<u32> = prim_noun_names().into_iter().map(db1_hash).collect();
    let cate_hashes: HashSet<u32> = USE_CATE_NOUN_NAMES
        .iter()
        .map(|noun| db1_hash(noun))
        .collect();

    let mut bran_hang_refnos = Vec::new();
    let mut loop_refnos = Vec::new();
    let mut cate_refnos = Vec::new();
    let mut prim_refnos = Vec::new();
    for (noun, mut refnos) in grouped {
        if let Some(limit) = config.gen_pipeline_debug_limit_per_target_type {
            // 只有截断需要在这里先定序：口径必须与 GenerationTargets::new 的
            // normalize_refnos 一致，否则同一输入会截出不同子集。
            refnos.sort_by_key(ToString::to_string);
            refnos.dedup();
            refnos.truncate(limit);
        }

        if noun == "BRAN" || noun == "HANG" {
            if include_bran_hang && config.should_process_noun(&noun, "cate") {
                bran_hang_refnos.extend(refnos);
            }
            continue;
        }

        let noun_hash = db1_hash(&noun);
        if loop_hashes.contains(&noun_hash) && config.should_process_noun(&noun, "loop") {
            loop_refnos.extend(refnos.iter().copied());
        }
        if cate_hashes.contains(&noun_hash) && config.should_process_noun(&noun, "cate") {
            cate_refnos.extend(refnos.iter().copied());
        }
        if prim_hashes.contains(&noun_hash) && config.should_process_noun(&noun, "prim") {
            prim_refnos.extend(refnos);
        }
    }

    Ok(GenerationTargets::new(
        bran_hang_refnos,
        loop_refnos,
        cate_refnos,
        prim_refnos,
        [],
    ))
}

pub(crate) async fn execute_generation_targets(
    db_option: Arc<DbOptionExt>,
    generation_read: Arc<GenerationReadContext>,
    config: &GenPipelineConfig,
    generation_contract: Arc<GenerationContract>,
    sender: flume::Sender<ShapeInstancesData>,
    targets: &GenerationTargets,
    generation_artifacts: Option<Arc<GenerationArtifacts>>,
) -> Result<CategorizedRefnos> {
    let total_start = Instant::now();
    println!(
        "🚀 启动 GenPipeline executor: bran/hang={} loop={} cate={} prim={} delete={} target_hash={}",
        targets.bran_hang_refnos().len(),
        targets.loop_refnos().len(),
        targets.cate_refnos().len(),
        targets.prim_refnos().len(),
        targets.delete_refnos().len(),
        targets.target_hash(),
    );
    config.print_info();
    if generation_contract.dry_run() {
        println!("  [gen_model] dry-run：已解析并验证 targets，跳过全部几何生成阶段和写入");
        return Ok(CategorizedRefnos::new());
    }

    let loop_sjus_map = Arc::new(DashMap::new());
    validate_sjus_map(&loop_sjus_map, config)?;
    let ctx = NounProcessContext::new(
        db_option,
        generation_read,
        generation_contract,
        config.batch_size.get(),
        config.concurrency.get(),
    );
    let mut categorized = CategorizedRefnos::new();
    let mut bran_generated_refnos = HashSet::new();

    let bran_start = Instant::now();
    process_bran_hang_core_logic(
        &ctx,
        targets.bran_hang_refnos(),
        Arc::clone(&loop_sjus_map),
        sender.clone(),
        &mut bran_generated_refnos,
        generation_artifacts.as_deref(),
    )
    .await?;
    let bran_duration = bran_start.elapsed();
    for refno in targets.bran_hang_refnos() {
        categorized.insert(*refno, super::models::NounCategory::Cate);
    }

    let (loop_refnos, loop_duration) = process_loop_stage(
        &ctx,
        targets.loop_refnos().to_vec(),
        Arc::clone(&loop_sjus_map),
        sender.clone(),
    )
    .await?;
    let (cate_refnos, cate_duration) = process_cate_stage(
        &ctx,
        targets.cate_refnos().to_vec(),
        &bran_generated_refnos,
        loop_sjus_map,
        sender.clone(),
    )
    .await?;
    let (prim_refnos, prim_duration) =
        process_prim_stage(&ctx, targets.prim_refnos().to_vec(), sender).await?;

    for refno in cate_refnos.into_iter().chain(bran_generated_refnos) {
        categorized.insert(refno, super::models::NounCategory::Cate);
    }
    for refno in loop_refnos {
        categorized.insert(refno, super::models::NounCategory::LoopOwner);
    }
    for refno in prim_refnos {
        categorized.insert(refno, super::models::NounCategory::Prim);
    }

    print_final_summary(
        total_start.elapsed(),
        loop_duration,
        cate_duration,
        prim_duration,
        bran_duration,
    );
    categorized.print_statistics();
    Ok(categorized)
}

/// 内部核心逻辑：处理 BRAN/HANG 相关的 CATE 生成及 Tubing
#[cfg_attr(
    feature = "profile",
    tracing::instrument(skip_all, name = "bran_hang_core_logic")
)]
async fn process_bran_hang_core_logic(
    ctx: &NounProcessContext,
    bran_roots: &[RefnoEnum],
    loop_sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
    bran_generated_refnos: &mut HashSet<RefnoEnum>,
    generation_artifacts: Option<&GenerationArtifacts>,
) -> Result<()> {
    if bran_roots.is_empty() {
        return Ok(());
    }
    let db_option = &ctx.db_option;
    let phase_total = Instant::now();
    println!(
        "📍 优先处理 BRAN/HANG 及其依赖 (count={})...",
        bran_roots.len()
    );

    // ── 阶段 1: 收集子元素 ──
    let t1 = Instant::now();
    #[cfg(feature = "profile")]
    let _span1 = tracing::info_span!("bran_collect_children").entered();
    let branch_refnos_map: DashMap<RefnoEnum, Vec<SPdmsElement>> = DashMap::new();
    let mut total_children: usize = 0;
    let generation_read = Arc::clone(&ctx.generation_read);
    for &refno in bran_roots {
        let child_refnos = super::session_query::get_descendants_by_types(
            &generation_read,
            refno,
            &BRAN_COMPONENT_NOUN_NAMES,
            None,
            false,
        )?;
        let children: Vec<SPdmsElement> = child_refnos
            .into_iter()
            .filter_map(|child_refno| {
                generation_read.hierarchy.node(child_refno).map(|node| {
                    let mut element = SPdmsElement::default();
                    element.refno = child_refno;
                    element.owner = node.owner;
                    element.noun = node.noun.clone();
                    element.dbnum = node.dbnum as i32;
                    element
                })
            })
            .collect();
        total_children += children.len();
        for child in &children {
            bran_generated_refnos.insert(child.refno);
        }
        if !children.is_empty() {
            branch_refnos_map.insert(refno, children);
        }
    }
    #[cfg(feature = "profile")]
    drop(_span1);
    let t1_ms = t1.elapsed().as_millis();
    println!(
        "  [BRAN perf] 阶段1 collect_children: {} ms (roots={}, children={})",
        t1_ms,
        bran_roots.len(),
        total_children
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
        // 失败必须上抛：退化成空 map 会静默跳过整批 BRAN 子件的 CATE 几何，
        // 而调用方仍会把这次 run 当成功、推进 model_gen 水位并消费欠账。
        build_cata_hash_map_from_session(&generation_read, &child_refnos)
            .await
            .map_err(|error| {
                GenPipelineError::GeometryGenerationFailed(
                    "bran_cata_hash_map".to_string(),
                    error.to_string(),
                )
            })?
    };
    let unique_cata_cnt = target_bran_reuse_cata_map.len();
    let target_bran_reuse_cata_map = Arc::new(target_bran_reuse_cata_map);
    #[cfg(feature = "profile")]
    drop(_span2);
    let t2_ms = t2.elapsed().as_millis();
    println!(
        "  [BRAN perf] 阶段2 build_cata_hash_map: {} ms (child_refnos={}, unique_cata={})",
        t2_ms,
        child_refnos.len(),
        unique_cata_cnt
    );
    // 逐个 refno 便于 grep 分析（如 24381_145019 是否进入 gen_cata_instances）。
    // 全量生成下这是百万级输出，只在 RUST_LOG=trace 时产生。
    if log::log_enabled!(log::Level::Trace) {
        for r in &child_refnos {
            log::trace!("[gen_model] BRAN child refno={r}");
        }
    }

    // ── 阶段 3: 生成 CATE 几何 ──
    let t4 = Instant::now();
    #[cfg(feature = "profile")]
    let _span4 = tracing::info_span!("bran_generate_cate").entered();

    let mut cate_outcome = None;
    if !child_refnos.is_empty() {
        cate_outcome = Some(
            cata_model::gen_cata_instances_versioned(
                db_option.clone(),
                Arc::clone(&generation_read),
                target_bran_reuse_cata_map.clone(),
                loop_sjus_map_arc.clone(),
                sender.clone(),
                ctx.generation_contract.respect_tufl(),
            )
            .await?,
        );
    }

    #[cfg(feature = "profile")]
    drop(_span4);
    let t4_ms = t4.elapsed().as_millis();
    if let Some(ref outcome) = cate_outcome {
        println!(
            "  [BRAN perf] 阶段3 gen_cata_instances: {} ms (unique_cata={}, elapsed_inner={} ms)",
            t4_ms, outcome.unique_cata_cnt, outcome.elapsed_ms
        );
        for (k, v) in &outcome.time_stats {
            println!("    [BRAN perf]   cata_time.{}: {} ms", k, v);
        }
    } else {
        println!(
            "  [BRAN perf] 阶段3 gen_cata_instances: {} ms (skipped)",
            t4_ms
        );
    }

    // ── 阶段 4: 收集 tubi_info；由 ModelWriter 在 barrier 后统一持久化 ──
    let t5 = Instant::now();
    #[cfg(feature = "profile")]
    let _span5 = tracing::info_span!("bran_save_tubi_info").entered();
    if let (Some(artifacts), Some(outcome)) = (generation_artifacts, cate_outcome.as_ref()) {
        artifacts.record_tubi_info(&outcome.tubi_info_map)?;
    }
    #[cfg(feature = "profile")]
    drop(_span5);
    let t5_ms = t5.elapsed().as_millis();
    println!("  [BRAN perf] 阶段4 collect_tubi_info: {} ms", t5_ms);

    // ── 阶段 5: 生成 Tubing ──
    let t6 = Instant::now();
    #[cfg(feature = "profile")]
    let _span6 = tracing::info_span!("bran_gen_branch_tubi").entered();
    let local_al_map = cate_outcome
        .as_ref()
        .map(|o| o.local_al_map.clone())
        .unwrap_or_else(|| Arc::new(DashMap::new()));

    let tubi_result = cata_model::gen_branch_tubi_from_db_with_prefetch_versioned(
        db_option.clone(),
        Arc::clone(&generation_read),
        Arc::new(branch_refnos_map),
        loop_sjus_map_arc,
        sender,
        local_al_map,
        None,
        None,
    )
    .await;
    #[cfg(feature = "profile")]
    drop(_span6);
    let t6_ms = t6.elapsed().as_millis();
    // 失败必须上抛：吞掉这里的错误会让整条 BRAN 的 tubing 缺失，
    // 而 run 仍以成功收尾并推进 model_gen 水位，下一轮增量也不会重试。
    let tubi_outcome = match tubi_result {
        Ok(outcome) => outcome,
        Err(error) => {
            println!("  [BRAN perf] 阶段5 gen_branch_tubi: {} ms (failed)", t6_ms);
            return Err(GenPipelineError::GeometryGenerationFailed(
                "bran_tubi".to_string(),
                error.to_string(),
            ));
        }
    };
    println!(
        "  [BRAN perf] 阶段5 gen_branch_tubi: {} ms (tubi_count={}, elapsed_inner={} ms)",
        t6_ms, tubi_outcome.tubi_count, tubi_outcome.elapsed_ms
    );
    for (k, v) in &tubi_outcome.time_stats {
        println!("    [BRAN perf]   tubi_time.{}: {} ms", k, v);
    }

    // ── 汇总 ──
    let total_ms = phase_total.elapsed().as_millis();
    println!(
        "  [BRAN perf] 总计: {} ms [collect={}ms, cata_hash={}ms, cata_gen={}ms, tubi_info={}ms, tubi_gen={}ms]",
        total_ms, t1_ms, t2_ms, t4_ms, t5_ms, t6_ms
    );

    Ok(())
}

/// `refnos` 由 `GenerationTargets` 提供，已在 `normalize_refnos` 里排序去重。
async fn process_loop_stage(
    ctx: &NounProcessContext,
    refnos: Vec<RefnoEnum>,
    loop_sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
) -> Result<(Vec<RefnoEnum>, Duration)> {
    let start = Instant::now();
    let chunk_size = ctx.batch_size.max(1);
    for (page_idx, slice) in refnos.chunks(chunk_size).enumerate() {
        let offset = page_idx * chunk_size;
        println!(
            "[Loop] 处理第 {} 页 ({} ~ {})",
            page_idx + 1,
            offset + 1,
            offset + slice.len()
        );
        process_loop_refno_page(ctx, loop_sjus_map_arc.clone(), sender.clone(), slice)
            .await
            .map_err(|e| {
                GenPipelineError::GeometryGenerationFailed("loop".to_string(), e.to_string())
            })?;
    }
    Ok((refnos, start.elapsed()))
}

async fn process_cate_stage(
    ctx: &NounProcessContext,
    mut refnos: Vec<RefnoEnum>,
    bran_generated_refnos: &HashSet<RefnoEnum>,
    loop_sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
) -> Result<(Vec<RefnoEnum>, Duration)> {
    let start = Instant::now();
    // retain 保序，`GenerationTargets` 给的顺序在过滤后依然有效。
    refnos.retain(|r| !bran_generated_refnos.contains(r));
    let chunk_size = ctx.batch_size.max(1);
    for (page_idx, slice) in refnos.chunks(chunk_size).enumerate() {
        let offset = page_idx * chunk_size;
        println!(
            "[Cate] 处理第 {} 页 ({} ~ {})",
            page_idx + 1,
            offset + 1,
            offset + slice.len()
        );
        process_cate_refno_page(ctx, loop_sjus_map_arc.clone(), sender.clone(), slice)
            .await
            .map_err(|e| {
                GenPipelineError::GeometryGenerationFailed("cate".to_string(), e.to_string())
            })?;
    }
    Ok((refnos, start.elapsed()))
}

/// `refnos` 由 `GenerationTargets` 提供，已在 `normalize_refnos` 里排序去重。
async fn process_prim_stage(
    ctx: &NounProcessContext,
    refnos: Vec<RefnoEnum>,
    sender: flume::Sender<ShapeInstancesData>,
) -> Result<(Vec<RefnoEnum>, Duration)> {
    let start = Instant::now();
    let chunk_size = ctx.batch_size.max(1);
    for (page_idx, slice) in refnos.chunks(chunk_size).enumerate() {
        let offset = page_idx * chunk_size;
        println!(
            "[Prim] 处理第 {} 页 ({} ~ {})",
            page_idx + 1,
            offset + 1,
            offset + slice.len()
        );
        process_prim_refno_page(ctx, sender.clone(), slice)
            .await
            .map_err(|e| {
                GenPipelineError::GeometryGenerationFailed("prim".to_string(), e.to_string())
            })?;
    }
    Ok((refnos, start.elapsed()))
}

fn print_final_summary(total: Duration, l: Duration, c: Duration, p: Duration, b: Duration) {
    println!("✅ GenPipeline 处理完成 (GeneralPath)");
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
                from_meta = db_meta().get_dbnums_by_type(&db_option.inner.module);
                if from_meta.is_empty() && db_option.inner.module.eq_ignore_ascii_case("DESI") {
                    log::warn!(
                        "[gen_pipeline] db_meta_info.json 中未发现 DESI 数据库，回退到所有 dbnum"
                    );
                    from_meta = db_meta().get_all_dbnums();
                }
            }
            Err(e) => {
                log::warn!(
                    "[gen_pipeline] 加载 db_meta_info.json 失败，尝试从 SurrealDB(pe) 获取 dbnum: {}",
                    e
                );
            }
        }

        if from_meta.is_empty() {
            return Err(GenPipelineError::DatabaseError(format!(
                "db_meta_info.json 中未找到 module={} 对应的 dbnum，请先完成解析或指定 manual_db_nums",
                db_option.inner.module
            )));
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

fn get_entry_nouns(config: &GenPipelineConfig) -> Vec<String> {
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
            .chain(DATUM_MARKER_NOUN_NAMES.iter())
            .chain(USE_CATE_NOUN_NAMES.iter())
        {
            set.insert(noun.to_string());
        }
        set.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation_read::{ElementSnapshot, HierarchyRow};

    #[test]
    fn prim_noun_names_include_datum_marker_nouns() {
        let nouns = prim_noun_names();

        assert!(nouns.contains(&"JLDATU"));
        assert!(nouns.contains(&"PLDATU"));
    }

    #[test]
    fn bran_hang_scope_policy_is_config_driven() {
        assert!(should_include_bran_hang(&GenPipelineConfig::default()));
        assert!(should_include_bran_hang(
            &GenPipelineConfig::default().with_enabled_categories(vec!["bran".into()])
        ));
        assert!(!should_include_bran_hang(
            &GenPipelineConfig::default().with_enabled_categories(vec!["cate".into()])
        ));
    }

    #[tokio::test]
    async fn full_root_and_incremental_resolvers_normalize_to_same_targets() {
        let world = RefnoEnum::from("1/1");
        let bran = RefnoEnum::from("1/2");
        let cate = RefnoEnum::from("1/3");
        let prim = RefnoEnum::from("1/4");
        let elements = vec![
            ElementSnapshot {
                refno: world,
                dbnum: 1,
                owner: RefnoEnum::from("0/0"),
                noun: "WORLD".into(),
                name: "world".into(),
                children: vec![bran, cate, prim],
                has_children: true,
            },
            ElementSnapshot {
                refno: bran,
                dbnum: 1,
                owner: world,
                noun: "BRAN".into(),
                name: "bran".into(),
                children: Vec::new(),
                has_children: false,
            },
            ElementSnapshot {
                refno: cate,
                dbnum: 1,
                owner: world,
                noun: "EQUI".into(),
                name: "cate".into(),
                children: Vec::new(),
                has_children: false,
            },
            ElementSnapshot {
                refno: prim,
                dbnum: 1,
                owner: world,
                noun: "BOX".into(),
                name: "prim".into(),
                children: Vec::new(),
                has_children: false,
            },
        ];
        let rows = [bran, cate, prim]
            .into_iter()
            .enumerate()
            .map(|(ordinal, child)| HierarchyRow {
                dbnum: 1,
                parent: world,
                child,
                ordinal: ordinal as u32,
            })
            .collect();
        let hierarchy = HierarchySnapshot::from_parts(42, elements, rows).expect("hierarchy");
        let config = GenPipelineConfig::default();
        let mut option = DbOptionExt::from(aios_core::options::DbOption::default());
        option.manual_db_nums = Some(vec![1]);

        let full = resolve_full_generation_targets(&option, &hierarchy, &config)
            .await
            .expect("full targets");
        let root =
            resolve_root_generation_targets(&hierarchy, &config, &[world]).expect("root targets");
        let mut log = IncrGeoUpdateLog::default();
        log.bran_hanger_refnos.insert(bran);
        log.basic_cata_refnos.insert(cate);
        log.prim_refnos.insert(prim);
        let incremental = resolve_incremental_generation_targets(&hierarchy, &config, &log)
            .expect("incremental targets");

        assert_eq!(full.target_hash(), root.target_hash());
        assert_eq!(full.target_hash(), incremental.target_hash());
        assert_eq!(full.bran_hang_refnos(), [bran]);
        assert_eq!(full.cate_refnos(), [cate]);
        assert_eq!(full.prim_refnos(), [prim]);
    }

    #[test]
    fn test_validate_sjus_map_empty_warning() {
        let sjus_map = DashMap::new();
        let config = GenPipelineConfig::default();

        // 默认配置下，空 map 会警告但不报错
        let result = validate_sjus_map(&sjus_map, &config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_sjus_map_empty_strict() {
        let sjus_map = DashMap::new();
        let config = GenPipelineConfig::default().with_strict_validation(true);

        // 严格模式下，空 map 会报错
        let result = validate_sjus_map(&sjus_map, &config);
        assert!(result.is_err());

        if let Err(GenPipelineError::EmptySjusMap) = result {
            // 正确
        } else {
            panic!("Expected EmptySjusMap error");
        }
    }

    // #[test]
    // fn test_validate_sjus_map_with_data() {
    //     let sjus_map = DashMap::new();
    //     sjus_map.insert(RefnoEnum::RefU64(1), (Vec3::ZERO, 1.0));

    //     let config = GenPipelineConfig::default().with_strict_validation(true);

    //     // 有数据时不应报错
    //     let result = validate_sjus_map(&sjus_map, &config);
    //     assert!(result.is_ok());
    // }
}
