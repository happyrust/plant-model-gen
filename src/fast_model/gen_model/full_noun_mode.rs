use aios_core::RefnoEnum;
use aios_core::geometry::ShapeInstancesData;
use aios_core::options::DbOption;
use crate::options::DbOptionExt;
use aios_core::pdms_types::{
    GNERAL_LOOP_OWNER_NOUN_NAMES, GNERAL_PRIM_NOUN_NAMES, USE_CATE_NOUN_NAMES,
};
use aios_core::pe::SPdmsElement;
use aios_core::{DBType, query_mdb_db_nums};
use aios_core::{RecordId, SUL_DB, SurrealQueryExt};
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

    println!("🚀 启动 Full Noun 模式（入口 Noun 深度查询版本）");
    config.print_info();

    // 🔥 读取数据库过滤配置：优先 manual_db_nums，否则按当前 MDB 的 DB 列表，并应用 exclude_db_nums
    let mut dbnums: Vec<u32> = if let Some(manual) = db_option.manual_db_nums.clone() {
        manual
    } else {
        // 从 MDB 获取当前项目允许的 DB 列表（DESI）
        query_mdb_db_nums(None, DBType::DESI).await.map_err(|e| {
            FullNounError::DatabaseError(format!("query_mdb_db_nums(None, DESI) failed: {}", e))
        })?
    };

    // 应用排除列表
    if let Some(exclude) = &db_option.exclude_db_nums {
        dbnums.retain(|dbno| !exclude.contains(dbno));
    }

    if !dbnums.is_empty() {
        println!("🗂️  数据库过滤: 仅查询 dbnum = {:?}", dbnums);
    } else {
        println!("🗂️  数据库过滤: 查询所有数据库（未设置 manual_db_nums），或过滤后为空");
    }

    let has_explicit_entry_nouns = config.enabled_categories.iter().any(|cat| {
        let lower = cat.to_lowercase();
        !matches!(lower.as_str(), "cate" | "loop" | "prim")
    });

    let entry_nouns: Vec<String> = if has_explicit_entry_nouns {
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
    };

    if entry_nouns.is_empty() {
        println!("[gen_full_noun_geos] 入口 Noun 列表为空，直接返回");
        return Ok(CategorizedRefnos::new());
    }

    println!("📌 入口 Noun 列表: {:?}", entry_nouns);

    let mut all_roots: HashSet<RefnoEnum> = HashSet::new();
    let mut loop_refnos: HashSet<RefnoEnum> = HashSet::new();
    let mut prim_refnos: HashSet<RefnoEnum> = HashSet::new();
    let mut cate_refnos: HashSet<RefnoEnum> = HashSet::new();
    // BRAN/HANG 根节点单独收集，避免混入普通 CATE 流程
    let mut bran_hanger_roots: HashSet<RefnoEnum> = HashSet::new();
    // 🔥 记录 BRAN/HANG 处理过程中已生成的子节点 refno，后续处理时跳过
    let mut bran_generated_refnos: HashSet<RefnoEnum> = HashSet::new();

    for entry in &entry_nouns {
        let noun_upper = entry.to_uppercase();
        let noun_str = noun_upper.as_str();

        let db_filter = if dbnums.is_empty() {
            "true".to_string()
        } else {
            let nums = dbnums
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(",");
            format!(
                "dbnum IN [{}]",
                nums
            )
        };
        let sql = format!("SELECT VALUE id FROM {} WHERE {}", noun_str, db_filter);
        let record_ids: Vec<RecordId> = SUL_DB.query_take(&sql, 0).await.map_err(|e| {
            FullNounError::DatabaseError(format!("query noun={} failed: {}", noun_str, e))
        })?;
        let mut refnos: Vec<RefnoEnum> = record_ids
            .into_iter()
            .map(RefnoEnum::from)
            .filter(|r| r.is_valid())
            .collect();

        if let Some(limit) = config.debug_limit_per_noun {
            if refnos.len() > limit {
                println!(
                    "[gen_full_noun_geos] 入口 noun {}: 调试模式限制实例数量从 {} 到 {}",
                    noun_str,
                    refnos.len(),
                    limit
                );
                refnos.truncate(limit);
            }
        }

        track_refno_issues(&refnos, noun_str, RefnoErrorStage::InputParse);

        if refnos.is_empty() {
            println!(
                "[gen_full_noun_geos] 入口 noun {}: 未找到实例，跳过",
                noun_str
            );
            continue;
        }

        all_roots.extend(refnos.iter().copied());

        // 🔥 BRAN/HANG 作为特殊类型单独处理（用于 Tubing 生成）
        if noun_str == "BRAN" || noun_str == "HANG" {
            bran_hanger_roots.extend(refnos.iter().copied());
            println!(
                "[gen_full_noun_geos] {} 作为 BRAN/HANG 特殊处理（Tubing）",
                noun_str
            );
        }

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

    // 🔥 直接查询数据库中的 BRAN/HANG 表（不依赖入口 Noun）
    for noun in &["BRAN", "HANG"] {
        let db_filter = if dbnums.is_empty() {
            "true".to_string()
        } else {
            let nums = dbnums
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(",");
            format!("dbnum IN [{}]", nums)
        };
        let sql = format!("SELECT VALUE id FROM {} WHERE {}", noun, db_filter);
        let record_ids: Vec<RecordId> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
        let refnos: Vec<RefnoEnum> = record_ids
            .into_iter()
            .map(RefnoEnum::from)
            .filter(|r| r.is_valid())
            .collect();
        if !refnos.is_empty() {
            println!(
                "[gen_full_noun_geos] 直接查询 {} 表: {} 个实例",
                noun,
                refnos.len()
            );
            bran_hanger_roots.extend(refnos.iter().copied());
            // 同时加入 all_roots，确保后续深度查询能覆盖
            all_roots.extend(refnos.iter().copied());
        }
    }

    if all_roots.is_empty() {
        println!(
            "[gen_full_noun_geos] 入口 Noun {:?}: 未找到任何实例，直接返回",
            entry_nouns
        );
        return Ok(CategorizedRefnos::new());
    }

    println!(
        "[gen_full_noun_geos] 入口 Noun {:?}: 根节点总数 {}",
        entry_nouns,
        all_roots.len()
    );

    let roots_vec: Vec<RefnoEnum> = all_roots.iter().copied().collect();

    let loop_descendants =
        aios_core::collect_descendant_filter_ids(&roots_vec, &GNERAL_LOOP_OWNER_NOUN_NAMES, None)
            .await
            .map_err(|e| {
                FullNounError::DatabaseError(format!(
                    "collect_descendant_filter_ids(loop) failed: {}",
                    e
                ))
            })?;
    track_refno_issues(
        &loop_descendants,
        "loop_descendants",
        RefnoErrorStage::Query,
    );
    loop_refnos.extend(loop_descendants);

    let prim_descendants =
        aios_core::collect_descendant_filter_ids(&roots_vec, &GNERAL_PRIM_NOUN_NAMES, None)
            .await
            .map_err(|e| {
                FullNounError::DatabaseError(format!(
                    "collect_descendant_filter_ids(prim) failed: {}",
                    e
                ))
            })?;
    track_refno_issues(
        &prim_descendants,
        "prim_descendants",
        RefnoErrorStage::Query,
    );
    prim_refnos.extend(prim_descendants);

    let cate_descendants =
        aios_core::collect_descendant_filter_ids(&roots_vec, &USE_CATE_NOUN_NAMES, None)
            .await
            .map_err(|e| {
                FullNounError::DatabaseError(format!(
                    "collect_descendant_filter_ids(cate) failed: {}",
                    e
                ))
            })?;
    track_refno_issues(
        &cate_descendants,
        "cate_descendants",
        RefnoErrorStage::Query,
    );
    cate_refnos.extend(cate_descendants);

    println!(
        " 深度查询结果：Loop={}，Prim={}，Cate={}，BRAN/HANG={}",
        loop_refnos.len(),
        prim_refnos.len(),
        cate_refnos.len(),
        bran_hanger_roots.len()
    );

    let loop_sjus_map_arc = Arc::new(DashMap::new());
    validate_sjus_map(&loop_sjus_map_arc, config)?;

    let ctx = NounProcessContext::new(
        db_option.clone(),
        config.batch_size.get(),
        config.concurrency.get(),
    );

    let mut categorized = CategorizedRefnos::new();

    // ============================================================================
    // 🔥 [0/4] BRAN/HANG 优先处理阶段
    // ============================================================================
    let mut bran_duration = std::time::Duration::ZERO;
    let bran_roots: Vec<RefnoEnum> = bran_hanger_roots.iter().copied().collect();
    if !bran_roots.is_empty() {
        println!("📍 [0/4] 优先处理 BRAN/HANG 及其依赖 (count={})...", bran_roots.len());
        let bran_start = Instant::now();

        // 1. 收集 BRAN/HANG 相关的 LOOP refno（用于生成 SJUS 数据）
        let bran_loop_descendants = aios_core::collect_descendant_filter_ids(
            &bran_roots,
            &GNERAL_LOOP_OWNER_NOUN_NAMES,
            None,
        )
        .await
        .unwrap_or_default();

        if !bran_loop_descendants.is_empty() {
            println!(
                "[gen_full_noun_geos] BRAN/HANG 相关 LOOP: {} 个",
                bran_loop_descendants.len()
            );
            // 处理 BRAN/HANG 相关的 LOOP
            let ranges = ctx.bounded_chunks(bran_loop_descendants.len());
            for (page_index, (start, end)) in ranges.into_iter().enumerate() {
                let slice = &bran_loop_descendants[start..end];
                println!(
                    "[gen_full_noun_geos] bran-loop: 处理第 {} 页 ({} ~ {})",
                    page_index + 1,
                    start + 1,
                    end
                );
                process_loop_refno_page(&ctx, loop_sjus_map_arc.clone(), sender.clone(), slice)
                    .await
                    .map_err(|e| {
                        FullNounError::GeometryGenerationFailed("bran-loop".to_string(), e.to_string())
                    })?;
            }
            // 记录已生成的 LOOP refno
            bran_generated_refnos.extend(bran_loop_descendants.iter().copied());
        }

        // 2. 查询 BRAN/HANG 的子元素
        let branch_refnos_map: DashMap<RefnoEnum, Vec<SPdmsElement>> = DashMap::new();
        for &refno in &bran_roots {
            match aios_core::collect_children_elements(refno, &[]).await {
                Ok(children) => {
                    if !children.is_empty() {
                        // 记录子元素的 refno
                        for child in &children {
                            bran_generated_refnos.insert(child.refno);
                        }
                        branch_refnos_map.insert(refno, children);
                    }
                }
                Err(e) => {
                    println!(
                        "[gen_full_noun_geos] 查询 BRAN/HANG 子元素失败 (refno={}): {}",
                        refno, e
                    );
                }
            }
        }

        // 3. 查询 BRAN/HANG 的元件库分组
        let target_bran_reuse_cata_map = match aios_core::query_group_by_cata_hash(&bran_roots).await {
            Ok(map) => map,
            Err(e) => {
                println!("[gen_full_noun_geos] 查询 BRAN/HANG 元件库分组失败: {}", e);
                DashMap::new()
            }
        };

        // 4. 生成 BRAN/HANG 相关的 CATE 几何
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
                println!("[gen_full_noun_geos] BRAN/HANG 关联 CATE 生成失败: {}", e);
                None
            }
        };

        // 5. 保存 tubi_info
        if let Some(ref outcome) = cate_outcome {
            println!(
                "[gen_full_noun_geos] BRAN/HANG tubi_info_map: {}, local_al_map: {}",
                outcome.tubi_info_map.len(),
                outcome.local_al_map.len()
            );
            if let Err(e) = pdms_inst::save_tubi_info_batch(&outcome.tubi_info_map).await {
                println!("[gen_full_noun_geos] 保存 tubi_info 失败: {}", e);
            }
        }

        // 6. 生成 BRAN/HANG Tubing
        let local_al_map = cate_outcome
            .as_ref()
            .map(|o| o.local_al_map.clone())
            .unwrap_or_else(|| Arc::new(DashMap::new()));

        if let Err(e) = cata_model::gen_branch_tubi(
            db_option.clone(),
            Arc::new(branch_refnos_map),
            loop_sjus_map_arc.clone(),
            sender.clone(),
            local_al_map,
        )
        .await
        {
            println!("[gen_full_noun_geos] BRAN/HANG Tubing 生成失败: {}", e);
        }

        bran_duration = bran_start.elapsed();
        println!(
            "⏱️  BRAN/HANG 优先处理完成: {} ms, 已生成子节点: {} 个",
            bran_duration.as_millis(),
            bran_generated_refnos.len()
        );
    }

    // ============================================================================
    // [1/4] 处理剩余的 LOOP（跳过已生成的）
    // ============================================================================
    println!("📍 [1/4] 处理 LOOP Refno 集合（跳过已生成）...");
    let loop_start = Instant::now();
    // 过滤掉已生成的 refno
    let loop_vec: Vec<RefnoEnum> = loop_refnos
        .iter()
        .copied()
        .filter(|r| !bran_generated_refnos.contains(r))
        .collect();
    let loop_skipped = loop_refnos.len() - loop_vec.len();
    if loop_skipped > 0 {
        println!("[gen_full_noun_geos] LOOP 跳过已生成: {} 个", loop_skipped);
    }
    {
        let ranges = ctx.bounded_chunks(loop_vec.len());
        for (page_index, (start, end)) in ranges.into_iter().enumerate() {
            let slice = &loop_vec[start..end];
            println!(
                "[gen_full_noun_geos] loop: 处理第 {} 页 ({} ~ {})",
                page_index + 1,
                start + 1,
                end
            );
            process_loop_refno_page(&ctx, loop_sjus_map_arc.clone(), sender.clone(), slice)
                .await
                .map_err(|e| {
                    FullNounError::GeometryGenerationFailed("loop".to_string(), e.to_string())
                })?;
        }
    }
    let loop_duration = loop_start.elapsed();
    println!("⏱️  LOOP processing took {} ms", loop_duration.as_millis());

    #[cfg(feature = "profile")]
    info!(
        loop_count = loop_vec.len(),
        duration_ms = loop_duration.as_millis() as u64,
        "LOOP Noun processing completed (entry-based)"
    );

    println!("📍 [2/4] 处理 PRIM Refno 集合...");
    let prim_start = Instant::now();
    let prim_vec: Vec<RefnoEnum> = prim_refnos.iter().copied().collect();
    {
        let ranges = ctx.bounded_chunks(prim_vec.len());
        for (page_index, (start, end)) in ranges.into_iter().enumerate() {
            let slice = &prim_vec[start..end];
            println!(
                "[gen_full_noun_geos] prim: 处理第 {} 页 ({} ~ {})",
                page_index + 1,
                start + 1,
                end
            );
            process_prim_refno_page(&ctx, sender.clone(), slice)
                .await
                .map_err(|e| {
                    FullNounError::GeometryGenerationFailed("prim".to_string(), e.to_string())
                })?;
        }
    }
    let prim_duration = prim_start.elapsed();
    println!("⏱️  PRIM processing took {} ms", prim_duration.as_millis());

    #[cfg(feature = "profile")]
    info!(
        prim_count = prim_vec.len(),
        duration_ms = prim_duration.as_millis() as u64,
        "PRIM Noun processing completed (entry-based)"
    );

    println!("📍 [3/4] 处理 CATE Refno 集合（跳过已生成）...");
    let cate_start = Instant::now();
    // 过滤掉已生成的 refno
    let cate_vec: Vec<RefnoEnum> = cate_refnos
        .iter()
        .copied()
        .filter(|r| !bran_generated_refnos.contains(r))
        .collect();
    let cate_skipped = cate_refnos.len() - cate_vec.len();
    if cate_skipped > 0 {
        println!("[gen_full_noun_geos] CATE 跳过已生成: {} 个", cate_skipped);
    }
    {
        let ranges = ctx.bounded_chunks(cate_vec.len());
        for (page_index, (start, end)) in ranges.into_iter().enumerate() {
            let slice = &cate_vec[start..end];
            println!(
                "[gen_full_noun_geos] cate: 处理第 {} 页 ({} ~ {})",
                page_index + 1,
                start + 1,
                end
            );
            process_cate_refno_page(&ctx, loop_sjus_map_arc.clone(), sender.clone(), slice)
                .await
                .map_err(|e| {
                    FullNounError::GeometryGenerationFailed("cate".to_string(), e.to_string())
                })?;
        }
    }
    let cate_duration = cate_start.elapsed();
    println!("⏱️  CATE processing took {} ms", cate_duration.as_millis());

    #[cfg(feature = "profile")]
    info!(
        cate_count = cate_vec.len(),
        duration_ms = cate_duration.as_millis() as u64,
        "CATE Noun processing completed (entry-based)"
    );

    // 将 BRAN/HANG 根节点也归类为 Cate，便于后续 mesh 深度遍历
    let bran_vec: Vec<RefnoEnum> = bran_hanger_roots.iter().copied().collect();

    for r in &cate_vec {
        categorized.insert(*r, super::models::NounCategory::Cate);
    }
    for r in &bran_vec {
        categorized.insert(*r, super::models::NounCategory::Cate);
    }
    for r in &loop_vec {
        categorized.insert(*r, super::models::NounCategory::LoopOwner);
    }
    for r in &prim_vec {
        categorized.insert(*r, super::models::NounCategory::Prim);
    }

    let total_duration = total_start.elapsed();
    println!("✅ Full Noun 处理完成（入口 Noun 深度查询版本）");
    println!(
        "⏱️  Total Full Noun processing: {} ms",
        total_duration.as_millis()
    );
    println!(
        "   ├─ LOOP: {} ms ({:.1}%)",
        loop_duration.as_millis(),
        loop_duration.as_secs_f64() / total_duration.as_secs_f64() * 100.0
    );
    println!(
        "   ├─ PRIM: {} ms ({:.1}%)",
        prim_duration.as_millis(),
        prim_duration.as_secs_f64() / total_duration.as_secs_f64() * 100.0
    );
    println!(
        "   ├─ CATE: {} ms ({:.1}%)",
        cate_duration.as_millis(),
        cate_duration.as_secs_f64() / total_duration.as_secs_f64() * 100.0
    );
    println!(
        "   └─ BRAN/HANG Tubing: {} ms ({:.1}%)",
        bran_duration.as_millis(),
        bran_duration.as_secs_f64() / total_duration.as_secs_f64() * 100.0
    );

    categorized.print_statistics();

    let error_summary = REFNO_ERROR_STORE.summary();
    if error_summary.total > 0 {
        println!("📊 RefNo 错误统计: 总计 {}", error_summary.total);
        for (kind, count) in error_summary.by_kind.iter() {
            println!("   - {:?}: {}", kind, count);
        }
        for (stage, count) in error_summary.by_stage.iter() {
            println!("   - 阶段 {:?}: {}", stage, count);
        }
    }

    #[cfg(feature = "profile")]
    info!(
        total_duration_ms = total_duration.as_millis() as u64,
        loop_ms = loop_duration.as_millis() as u64,
        prim_ms = prim_duration.as_millis() as u64,
        cate_ms = cate_duration.as_millis() as u64,
        "Full Noun generation completed with performance metrics (entry-based)"
    );

    Ok(categorized)
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
