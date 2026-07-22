//! specs/022/024 IncrementRun：一次 sesno 增量的完整管线（采集 → 落库/锚点 → 可选生成）。
//!
//! 深模块：`run_increment(options)` 一个入口，内部各阶段（源文件写前/写后 hash 门、
//! 采集、db_meta 刷新、Version Commit 落库、pe_owner 完整性证据、模型生成、Parquet 导出、
//! summary 汇总）全部私有。CLI（`incremental-sesno` /
//! `watch-incremental`）只是薄参数 adapter；连接策略（端口探测/自启动）留在
//! adapter 侧，经 `ensure_model_store` 闭包传入——与 `commit_version` 的 apply
//! 闭包同一模式。

use std::future::Future;
use std::path::PathBuf;

use anyhow::Context;

use crate::options::DbOptionExt;

#[derive(Debug, Clone, serde::Serialize)]
pub struct DbnumIncrementRange {
    pub dbnum: u32,
    pub from_sesno: u32,
    pub to_sesno: u32,
}

/// 一次增量运行的全部输入。区间起点语义：从 `from_sesno + 1` 收集到
/// `to_sesno`（缺省为文件最新 sesno）。
#[derive(Debug, Clone)]
pub struct IncrementRunOptions {
    pub file: Option<PathBuf>,
    pub dbnums: Vec<u32>,
    /// watch 多库轮次使用的逐库范围；与 `dbnums` 中的库不得重复。
    pub dbnum_ranges: Vec<DbnumIncrementRange>,
    pub from_sesno: u32,
    pub to_sesno: Option<u32>,
    pub rescan_index: bool,
    pub persist_data: bool,
    pub recover_pending: bool,
    pub generate_model: bool,
    /// 为 false 时所有 Modified 都进入模型更新桶，作为属性门控逃生口。
    pub model_impact_filter: bool,
    /// specs/023 M3/T8：增量模型生成前要求 pe_owner 完整性证据就绪
    /// （`pe_owner_version_meta` 存在 + 抽查通过）；不就绪快速失败。
    pub require_pe_owner_ready: bool,
    pub verbose: bool,
}

/// 运行结果。`parquet_export` 序列化为 JSON，公共接口不依赖 `gen_model` feature。
pub struct IncrementRunResult {
    pub summary: serde_json::Value,
    pub outcome: crate::data_interface::sesno_increment::PdmsSesnoIncrementOutcome,
    pub persist_stats: crate::data_interface::sesno_increment::PdmsIncrementPersistStats,
    pub generation_success: Option<bool>,
    pub parquet_export: Option<serde_json::Value>,
    pub failures: Vec<String>,
}

struct SourceHashGate {
    before: std::collections::BTreeMap<PathBuf, String>,
    aggregate_sha256: String,
}

/// specs/023 M3/T8：pe_owner 完整性证据（替代 `.tree` 文件存在性证据）。
///
/// latest 层级查询与增量生成的层级展开已统一走 pe_owner/pe（M1/M2），
/// 生成前的"层级数据可用"判据随之换源：
/// - `pe_owner_version_meta.maintained_since_sesno` 存在（full 重灌 / rebuild-pe-owner 建立）；
/// - 轻量抽查：抽样有子 parent 对比 `count(<-pe_owner)` 与 `len(children)`（口径对齐
///   `db-data/audit_pe_owner_vs_children.surql` [2] 段，样本上限 200/库）。
#[derive(Debug, Clone)]
pub(crate) struct PeOwnerEvidence {
    pub(crate) ready: bool,
    pub(crate) not_ready_dbnums: Vec<u32>,
    pub(crate) summary: serde_json::Value,
}

/// 执行一次增量运行。
///
/// `ensure_model_store`：persist 开启时、写入前被调用一次，负责保证模型库
/// 连接可用（CLI 传 `ensure_surreal_connected`；测试可传 no-op）。
pub async fn run_increment<C, Fut>(
    db_option_ext: &DbOptionExt,
    options: IncrementRunOptions,
    ensure_model_store: C,
) -> anyhow::Result<IncrementRunResult>
where
    C: FnOnce() -> Fut,
    Fut: Future<Output = anyhow::Result<()>>,
{
    let mutation_lock =
        super::project_mutation_lock::ProjectMutationLock::acquire_for_current_command(
            db_option_ext,
        )?;
    run_increment_with_lock(
        db_option_ext,
        options,
        ensure_model_store,
        mutation_lock.held(),
    )
    .await
}

pub(crate) async fn run_increment_with_lock<C, Fut>(
    db_option_ext: &DbOptionExt,
    options: IncrementRunOptions,
    ensure_model_store: C,
    mutation_lock: super::project_mutation_lock::HeldProjectMutationLock<'_>,
) -> anyhow::Result<IncrementRunResult>
where
    C: FnOnce() -> Fut,
    Fut: Future<Output = anyhow::Result<()>>,
{
    let run_started = std::time::Instant::now();
    let metrics_elapsed = || run_started.elapsed().as_millis() as u64;
    if !options.persist_data && options.generate_model {
        anyhow::bail!(
            "--no-persist cannot be combined with --generate-model; incremental model generation requires persisted PE/ATT data"
        );
    }
    if !options.persist_data && options.recover_pending {
        anyhow::bail!("--recover-pending cannot be combined with --no-persist");
    }
    #[cfg(not(feature = "gen_model"))]
    if options.generate_model {
        anyhow::bail!("--generate-model requires the gen_model feature");
    }
    crate::perf_metrics::record_generate_progress(
        "incremental_sesno_started",
        Some("collecting source increments"),
        metrics_elapsed(),
    );
    let mut collected_outcome =
        crate::data_interface::sesno_increment::PdmsSesnoCollectedOutcome::default();
    let mut source_count = 0usize;

    if let Some(file) = options.file.as_ref() {
        source_count += 1;
        let detail = format!("file={}", file.display());
        let _heartbeat = crate::perf_metrics::start_generate_heartbeat(
            "incremental_sesno_collecting_file",
            Some(detail.clone()),
            std::time::Duration::from_secs(15),
        );
        let file_outcome = crate::data_interface::sesno_increment::collect_pdms_increment_for_file_with_operations_options(
                &db_option_ext.inner.project_name,
                file.clone(),
                options.from_sesno,
                options.to_sesno,
                options.verbose,
                options.model_impact_filter,
            )?;
        crate::perf_metrics::record_generate_progress(
            "incremental_sesno_collected_file",
            Some(&detail),
            metrics_elapsed(),
        );
        collected_outcome.merge(file_outcome);
    }

    let dbnum_requests = {
        let mut seen = std::collections::BTreeSet::new();
        let mut requests = Vec::new();
        if !options.dbnums.is_empty() {
            for dbnum in &options.dbnums {
                anyhow::ensure!(
                    seen.insert(*dbnum),
                    "duplicate dbnum in incremental request: {dbnum}"
                );
            }
            requests.push((options.dbnums.clone(), options.from_sesno, options.to_sesno));
        }
        for range in &options.dbnum_ranges {
            anyhow::ensure!(range.dbnum > 0, "incremental dbnum must be non-zero");
            anyhow::ensure!(
                range.to_sesno >= range.from_sesno,
                "invalid incremental range for dbnum={}: {}..={}",
                range.dbnum,
                range.from_sesno,
                range.to_sesno
            );
            anyhow::ensure!(
                seen.insert(range.dbnum),
                "duplicate dbnum in incremental request: {}",
                range.dbnum
            );
            requests.push((vec![range.dbnum], range.from_sesno, Some(range.to_sesno)));
        }
        requests
    };

    if !dbnum_requests.is_empty() {
        source_count += dbnum_requests
            .iter()
            .map(|(dbnums, _, _)| dbnums.len())
            .sum::<usize>();
        #[cfg(feature = "sqlite-index")]
        {
            let index_path = crate::data_interface::db_index::default_index_path(
                &db_option_ext.inner.project_name,
            );
            if options.rescan_index || !index_path.exists() {
                let _heartbeat = crate::perf_metrics::start_generate_heartbeat(
                    "incremental_sesno_rebuilding_db_index",
                    Some(format!("index_path={}", index_path.display())),
                    std::time::Duration::from_secs(15),
                );
                let report = crate::data_interface::db_index::rebuild_from_config(false).await?;
                println!(
                    "✅ db_index 已刷新: {} 个库, {} 条 ref0 映射",
                    report.db_files, report.ref0_total
                );
            }
            let mut refreshed_after_miss = false;
            for (dbnums, from_sesno, to_sesno) in dbnum_requests {
                let detail = format!("dbnums={dbnums:?} from={from_sesno} to={to_sesno:?}");
                let _heartbeat = crate::perf_metrics::start_generate_heartbeat(
                    "incremental_sesno_collecting_dbnums",
                    Some(detail.clone()),
                    std::time::Duration::from_secs(15),
                );
                let collect = || {
                    crate::data_interface::sesno_increment::collect_pdms_increment_for_dbnums_from_index_with_operations_options(
                        &db_option_ext.inner.project_name,
                        &index_path,
                        &dbnums,
                        from_sesno,
                        to_sesno,
                        options.verbose,
                        options.model_impact_filter,
                    )
                };
                let indexed_outcome = match collect() {
                    Ok(outcome) => outcome,
                    Err(err) if !options.rescan_index && !refreshed_after_miss => {
                        eprintln!("⚠️  db_index 命中失败，按指纹刷新索引后重试: {}", err);
                        let report =
                            crate::data_interface::db_index::rebuild_from_config(false).await?;
                        refreshed_after_miss = true;
                        println!(
                            "✅ db_index 已刷新: {} 个库, {} 条 ref0 映射",
                            report.db_files, report.ref0_total
                        );
                        collect()?
                    }
                    Err(err) => return Err(err),
                };
                crate::perf_metrics::record_generate_progress(
                    "incremental_sesno_collected_dbnums",
                    Some(&detail),
                    metrics_elapsed(),
                );
                collected_outcome.merge(indexed_outcome);
            }
        }
        #[cfg(not(feature = "sqlite-index"))]
        {
            anyhow::bail!(
                "incremental-sesno --dbnum 需要 sqlite-index feature；可改用 --file 直接指定 db 文件"
            );
        }
    }

    if source_count == 0 {
        anyhow::bail!("incremental-sesno 需要指定 --file 或 --dbnum");
    }

    let crate::data_interface::sesno_increment::PdmsSesnoCollectedOutcome {
        outcome,
        files: collected_increment_files,
    } = collected_outcome;
    let source_hash_gate = prepare_source_hash_gate(&outcome)?;
    verify_source_hash_gate(&source_hash_gate)
        .context("source hash gate failed after collection; no data commit was started")?;

    let db_meta_refreshed_files = if options.persist_data {
        let refreshed = {
            let _heartbeat = crate::perf_metrics::start_generate_heartbeat(
                "incremental_sesno_refreshing_db_meta",
                Some(format!("files={}", outcome.files.len())),
                std::time::Duration::from_secs(15),
            );
            crate::data_interface::sesno_increment::refresh_db_meta_for_increment_files(
                &db_option_ext.inner.project_name,
                &outcome.files,
            )?
        };
        crate::perf_metrics::record_generate_progress(
            "incremental_sesno_db_meta_refreshed",
            Some(&format!("files={refreshed}")),
            metrics_elapsed(),
        );
        if refreshed > 0 {
            println!("✅ 增量 db_meta 已刷新: {} 个 db 文件", refreshed);
        }
        refreshed
    } else {
        crate::perf_metrics::record_generate_progress(
            "incremental_sesno_db_meta_refresh_skipped",
            Some("--no-persist requested"),
            metrics_elapsed(),
        );
        0
    };

    let persist_stats = if options.persist_data {
        {
            let _heartbeat = crate::perf_metrics::start_generate_heartbeat(
                "incremental_sesno_connecting_model_store",
                Some("ensure_model_store".to_string()),
                std::time::Duration::from_secs(15),
            );
            ensure_model_store().await?;
        }
        crate::perf_metrics::record_generate_progress(
            "incremental_sesno_model_store_connected",
            Some("ensure_model_store"),
            metrics_elapsed(),
        );
        let stats = {
            let _heartbeat = crate::perf_metrics::start_generate_heartbeat(
                "incremental_sesno_persisting",
                Some(format!(
                    "files={} reused_collected_operations=true",
                    collected_increment_files.len()
                )),
                std::time::Duration::from_secs(15),
            );
            crate::data_interface::sesno_increment::persist_collected_pdms_increment_files(
                &collected_increment_files,
                Some(source_hash_gate.aggregate_sha256.as_str()),
                options.recover_pending,
            )
            .await?
        };
        crate::perf_metrics::record_generate_progress(
            "incremental_sesno_persisted",
            Some(&format!(
                "sessions={} pe={} att={} deletes={}",
                stats.session_count, stats.pe_rows, stats.att_rows, stats.delete_count
            )),
            metrics_elapsed(),
        );
        if stats.session_count > 0 || stats.upsert_count > 0 {
            println!(
                "✅ 增量数据已保存: sessions={} pe={} att={} uda={} deletes={} dbnum_info={}",
                stats.session_count,
                stats.pe_rows,
                stats.att_rows,
                stats.uda_rows,
                stats.delete_count,
                stats.dbnum_info_updates
            );
        }
        stats
    } else {
        crate::perf_metrics::record_generate_progress(
            "incremental_sesno_persist_skipped",
            Some("--no-persist requested"),
            metrics_elapsed(),
        );
        println!("ℹ️ --no-persist 已启用：跳过 db_meta 刷新、SurrealDB 连接和 PE/ATT 写入");
        Default::default()
    };

    let generation_dbnums: Vec<u32> = {
        let mut dbnums = std::collections::BTreeSet::new();
        for file in &outcome.files {
            if file.dbnum > 0 {
                dbnums.insert(file.dbnum);
            }
        }
        dbnums.into_iter().collect()
    };
    let mut failures = persist_stats
        .commit_failures
        .iter()
        .map(|failure| {
            format!(
                "dbnum={} data commit {}..={} failed: {}",
                failure.dbnum, failure.from_sesno, failure.to_sesno, failure.error
            )
        })
        .collect::<Vec<_>>();
    let mut debt_written = Vec::new();
    let mut debt_write_failures = Vec::new();
    let mut debt_blocked_dbnums = std::collections::BTreeSet::new();
    if options.persist_data {
        for anchor in &persist_stats.anchors {
            let file = collected_increment_files.iter().find(|file| {
                file.report.dbnum == anchor.dbnum && file.report.actual_end_sesno == anchor.sesno
            });
            let Some(file) = file else {
                let message = format!(
                    "dbnum={} sesno={} committed without matching collected update log",
                    anchor.dbnum, anchor.sesno
                );
                debt_blocked_dbnums.insert(anchor.dbnum);
                failures.push(message.clone());
                debt_write_failures.push(serde_json::json!({
                    "dbnum": anchor.dbnum,
                    "to_sesno": anchor.sesno,
                    "error": message,
                }));
                continue;
            };
            let fingerprint = anchor.fingerprint.as_deref().unwrap_or_default();
            match crate::versioned_db::model_gen_debt::write_model_gen_debt(
                anchor.dbnum,
                file.report.actual_start_sesno,
                anchor.sesno,
                fingerprint,
                &file.update_log,
            )
            .await
            {
                Ok(written) => debt_written.push(written),
                Err(error) => {
                    let message = format!(
                        "dbnum={} model_gen_debt write failed: {error:#}",
                        anchor.dbnum
                    );
                    debt_blocked_dbnums.insert(anchor.dbnum);
                    failures.push(message.clone());
                    debt_write_failures.push(serde_json::json!({
                        "dbnum": anchor.dbnum,
                        "from_sesno": file.report.actual_start_sesno,
                        "to_sesno": anchor.sesno,
                        "error": message,
                    }));
                }
            }
        }
    }

    let commit_failure_dbnums = persist_stats
        .commit_failures
        .iter()
        .map(|failure| failure.dbnum)
        .collect::<std::collections::BTreeSet<_>>();
    let generation_barrier_blocked =
        !commit_failure_dbnums.is_empty() || !debt_blocked_dbnums.is_empty();
    let generation_barrier_status = if !options.generate_model {
        "disabled"
    } else if generation_barrier_blocked {
        "skipped_due_to_data_barrier"
    } else {
        "passed"
    };

    verify_source_hash_gate(&source_hash_gate)
        .context("source hash gate failed before model generation")?;
    let committed_dbnums = persist_stats
        .anchors
        .iter()
        .map(|anchor| anchor.dbnum)
        .collect::<std::collections::BTreeSet<_>>();
    let mut catch_up_results = Vec::new();
    let mut coverages = Vec::new();
    if options.persist_data {
        for dbnum in committed_dbnums {
            if options.generate_model && !generation_barrier_blocked {
                let catch_up =
                    crate::version_management::model_gen_catchup::catch_up_model_generation_with_lock(
                        db_option_ext,
                        dbnum,
                        crate::version_management::model_gen_catchup::ModelGenCatchUpOptions {
                            require_pe_owner_ready: options.require_pe_owner_ready,
                            allow_full_regen: false,
                            dry_run: db_option_ext.gen_model_dry_run,
                        },
                        mutation_lock,
                        false,
                    )
                    .await;
                match catch_up {
                    Ok(result) => {
                        coverages.push(result.coverage.clone());
                        if result.coverage.needs_full_regen {
                            failures.push(format!(
                                "dbnum={dbnum} model debt has a gap; controlled repair is required"
                            ));
                        } else if result.generation_success == Some(false) {
                            failures.push(format!("dbnum={dbnum} model generation failed"));
                        }
                        catch_up_results.push(result);
                    }
                    Err(error) => failures.push(format!(
                        "dbnum={dbnum} model generation catch-up failed: {error:#}"
                    )),
                }
            } else {
                match crate::versioned_db::model_gen_debt::analyze_model_gen_debt(dbnum).await {
                    Ok(coverage) => coverages.push(coverage),
                    Err(error) => failures.push(format!(
                        "dbnum={dbnum} model debt analysis failed: {error:#}"
                    )),
                }
            }
        }
    }
    let source_hash_summary = verify_source_hash_gate(&source_hash_gate).context(
        "source hash gate failed after model generation; no deferred model_gen anchors were published",
    )?;
    if db_option_ext.use_surrealdb
        && db_option_ext.model_writer_mode.writes_to_surreal()
        && !db_option_ext.gen_model_dry_run
    {
        for result in &mut catch_up_results {
            if result.generation_success == Some(true) && result.model_gen_anchor.is_none() {
                result.model_gen_anchor = Some(
                    crate::versioned_db::model_gen_debt::finalize_model_generation(
                        result.dbnum,
                        result.coverage.data_watermark,
                    )
                    .await?,
                );
            }
        }
    }
    let model_gen_anchors = catch_up_results
        .iter()
        .filter_map(|result| result.model_gen_anchor.clone())
        .collect::<Vec<_>>();
    let pe_owner_evidence = catch_up_results
        .iter()
        .filter_map(|result| result.pe_owner_evidence.clone())
        .collect::<Vec<_>>();
    let parquet_exports = catch_up_results
        .iter()
        .filter_map(|result| result.parquet_export.clone())
        .collect::<Vec<_>>();
    let parquet_export_value =
        (!parquet_exports.is_empty()).then_some(serde_json::json!(parquet_exports));
    let generation_success = options.generate_model.then_some(failures.is_empty());
    let model_neutral_changes = outcome
        .element_changes
        .iter()
        .filter(|change| {
            change.impact_decision == "neutral" && change.impact_reason == "known_neutral"
        })
        .cloned()
        .collect::<Vec<_>>();
    let data_watermarks = coverages
        .iter()
        .map(|coverage| (coverage.dbnum, coverage.data_watermark))
        .collect::<std::collections::BTreeMap<_, _>>();
    let model_generation_watermarks = coverages
        .iter()
        .map(|coverage| (coverage.dbnum, coverage.model_generation_watermark))
        .collect::<std::collections::BTreeMap<_, _>>();
    let debt_ranges = coverages
        .iter()
        .map(|coverage| (coverage.dbnum, coverage.debt_ranges.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let consumable_debt_ranges = coverages
        .iter()
        .map(|coverage| (coverage.dbnum, coverage.consumable_debt_ranges.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let stale_debt_ranges = coverages
        .iter()
        .map(|coverage| (coverage.dbnum, coverage.stale_debt_ranges.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let debt_gap_ranges = coverages
        .iter()
        .map(|coverage| (coverage.dbnum, coverage.gap_ranges.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let debt_bucket_counts = coverages
        .iter()
        .map(|coverage| (coverage.dbnum, coverage.debt_bucket_counts.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let consumable_debt_bucket_counts = coverages
        .iter()
        .map(|coverage| (coverage.dbnum, coverage.consumable_bucket_counts.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let coverage_complete = coverages.iter().all(|coverage| coverage.coverage_complete);
    let needs_full_regen = coverages.iter().any(|coverage| coverage.needs_full_regen);

    let summary = serde_json::json!({
        "from_sesno": options.from_sesno,
        "to_sesno": options.to_sesno,
        "dbnum_ranges": options.dbnum_ranges,
        "source_hash_gate": source_hash_summary,
        "pe_owner_evidence": pe_owner_evidence,
        "source_count": source_count,
        "file_count": outcome.files.len(),
        "session_count": outcome.total_session_count(),
        "element_count": outcome.total_element_count(),
        "db_meta_refreshed_files": db_meta_refreshed_files,
        "data_persist_enabled": options.persist_data,
        "recover_pending": options.recover_pending,
        "data_persist_skipped_reason": if options.persist_data { serde_json::Value::Null } else { serde_json::json!("--no-persist requested") },
        "data_persist": persist_stats,
        "version_anchor": persist_stats.anchors,
        "model_gen_anchor": model_gen_anchors,
        "model_neutral_changes": model_neutral_changes,
        "debt_written": debt_written,
        "debt_write_failures": debt_write_failures,
        "generation_barrier": {
            "status": generation_barrier_status,
            "blocked": generation_barrier_blocked,
            "commit_failure_dbnums": commit_failure_dbnums,
            "debt_failure_dbnums": debt_blocked_dbnums,
        },
        "data_watermark": data_watermarks,
        "model_generation_watermark": model_generation_watermarks,
        "debt_range_semantics": crate::versioned_db::model_gen_debt::MODEL_GEN_DEBT_RANGE_SEMANTICS,
        "debt_ranges": debt_ranges,
        "consumable_debt_ranges": consumable_debt_ranges,
        "stale_debt_ranges": stale_debt_ranges,
        "debt_gap_ranges": debt_gap_ranges,
        "debt_bucket_counts": debt_bucket_counts,
        "consumable_debt_bucket_counts": consumable_debt_bucket_counts,
        "coverage_complete": coverage_complete,
        "needs_full_regen": needs_full_regen,
        "generation_failures": failures,
        "generation_dbnums": generation_dbnums,
        "generation_success": generation_success,
        "parquet_export": parquet_export_value,
        "category_counts": {
            "prim": outcome.update_log.prim_refnos.len(),
            "loop_owner": outcome.update_log.loop_owner_refnos.len(),
            "bran_hanger": outcome.update_log.bran_hanger_refnos.len(),
            "basic_cata": outcome.update_log.basic_cata_refnos.len(),
            "delete": outcome.update_log.delete_refnos.len(),
            "total": outcome.update_log.count(),
        },
        "files": outcome.files,
        "element_changes": outcome.element_changes,
        "update_log": outcome.update_log,
    });

    Ok(IncrementRunResult {
        summary,
        outcome,
        persist_stats,
        generation_success,
        parquet_export: parquet_export_value,
        failures,
    })
}

fn prepare_source_hash_gate(
    outcome: &crate::data_interface::sesno_increment::PdmsSesnoIncrementOutcome,
) -> anyhow::Result<SourceHashGate> {
    let mut before = std::collections::BTreeMap::new();
    for report in &outcome.files {
        if let Some(existing) = before.insert(
            report.file_path.clone(),
            report.source_sha256_before.clone(),
        ) && !existing.eq_ignore_ascii_case(&report.source_sha256_before)
        {
            anyhow::bail!(
                "same source file was collected with different pre-read hashes: {}",
                report.file_path.display()
            );
        }
    }
    let aggregate_sha256 = aggregate_source_hashes(&before)?;
    Ok(SourceHashGate {
        before,
        aggregate_sha256,
    })
}

fn verify_source_hash_gate(gate: &SourceHashGate) -> anyhow::Result<serde_json::Value> {
    let mut after = std::collections::BTreeMap::new();
    let mut changed = Vec::new();
    for (path, before_hash) in &gate.before {
        let after_hash = crate::version_management::hashing::sha256_file(path)?;
        if !before_hash.eq_ignore_ascii_case(&after_hash) {
            changed.push(serde_json::json!({
                "path": path,
                "before": before_hash,
                "after": after_hash,
            }));
        }
        after.insert(path.clone(), after_hash);
    }
    if !changed.is_empty() {
        anyhow::bail!(
            "source files changed while incremental write/model generation was running; model_gen anchor was not published: {}",
            serde_json::to_string(&changed)?
        );
    }
    let after_aggregate_sha256 = aggregate_source_hashes(&after)?;
    Ok(serde_json::json!({
        "policy": "inline_pre_post_sha256",
        "file_count": gate.before.len(),
        "aggregate_sha256_before": gate.aggregate_sha256,
        "aggregate_sha256_after": after_aggregate_sha256,
        "unchanged": gate.aggregate_sha256.eq_ignore_ascii_case(&after_aggregate_sha256),
        "files": after
            .iter()
            .map(|(path, sha256)| serde_json::json!({"path": path, "sha256": sha256}))
            .collect::<Vec<_>>(),
    }))
}

fn aggregate_source_hashes(
    hashes: &std::collections::BTreeMap<PathBuf, String>,
) -> anyhow::Result<String> {
    let stable_rows = hashes
        .iter()
        .map(|(path, sha256)| {
            serde_json::json!({
                "path": path.to_string_lossy(),
                "sha256": sha256,
            })
        })
        .collect::<Vec<_>>();
    Ok(crate::version_management::hashing::sha256_bytes(
        &serde_json::to_vec(&stable_rows)?,
    ))
}

/// specs/023 M3/T8：构建 pe_owner 完整性证据（替代 `.tree` 存在性证据）。
///
/// 证据本身是**咨询性**的（degraded_allowed 语义保留）：单库探测失败不终止增量运行，
/// 记为该 dbnum not_ready 并把错误写进 summary；只有 `--require-pe-owner-ready`（strict）
/// 才把 not_ready 升级为快速失败（调用方判定）。
pub(crate) async fn build_pe_owner_evidence(
    generation_dbnums: &[u32],
    require_pe_owner_ready: bool,
) -> PeOwnerEvidence {
    use aios_core::{SurrealQueryExt, project_primary_db};
    use serde::Deserialize;
    use surrealdb::types::SurrealValue;

    /// 抽查样本上限（对齐 audit_pe_owner_vs_children.surql [2] 的口径，收敛为轻量在线探测）
    const SAMPLE_LIMIT: usize = 200;

    #[derive(Debug, Deserialize, SurrealValue)]
    struct SampleProbe {
        sampled: i64,
        mismatched: i64,
    }

    let mut checked_dbnums: Vec<u32> = generation_dbnums.to_vec();
    checked_dbnums.sort_unstable();
    checked_dbnums.dedup();

    let mut per_dbnum = Vec::new();
    let mut not_ready_dbnums = Vec::new();
    for dbnum in &checked_dbnums {
        let mut maintained_since: Option<u32> = None;
        let mut sampled = 0i64;
        let mut mismatched = 0i64;
        let mut probe_error: Option<String> = None;

        match crate::versioned_db::pe_owner_meta::get_maintained_since(*dbnum).await {
            Ok(value) => maintained_since = value,
            Err(error) => probe_error = Some(format!("meta query failed: {error:#}")),
        }
        if probe_error.is_none() {
            // 抽样有子 parent，对比边计数与 children 长度（一次请求两条语句）
            let sql = format!(
                "LET $s = (SELECT id, array::len(children ?? []) AS child_cnt, count(<-pe_owner) AS edge_cnt FROM pe WHERE dbnum = {dbnum} AND array::len(children ?? []) > 0 LIMIT {SAMPLE_LIMIT});\n\
                 RETURN {{ sampled: $s.len(), mismatched: $s.filter(|$r| $r.child_cnt != $r.edge_cnt).len() }};"
            );
            match project_primary_db()
                .query_take::<Option<SampleProbe>>(&sql, 1)
                .await
            {
                Ok(Some(probe)) => {
                    sampled = probe.sampled;
                    mismatched = probe.mismatched;
                }
                Ok(None) => probe_error = Some("sample probe returned no row".to_string()),
                Err(error) => probe_error = Some(format!("sample probe failed: {error:#}")),
            }
        }

        let dbnum_ready = probe_error.is_none() && maintained_since.is_some() && mismatched == 0;
        if !dbnum_ready {
            not_ready_dbnums.push(*dbnum);
        }
        per_dbnum.push(serde_json::json!({
            "dbnum": dbnum,
            "maintained_since_sesno": maintained_since,
            "sampled": sampled,
            "mismatched": mismatched,
            "ready": dbnum_ready,
            "error": probe_error,
        }));
    }

    let ready = not_ready_dbnums.is_empty();
    let mode = if require_pe_owner_ready {
        "strict_required"
    } else if ready {
        "ready"
    } else {
        "degraded_allowed"
    };
    let recommendation = if ready {
        "pe_owner edges are maintained and sample-consistent for the incremental generation dbnums."
            .to_string()
    } else {
        format!(
            "Run `model-version rebuild-pe-owner --dbnum <n>` for dbnums {:?} (full audit: scripts/smoke/pe_owner_children_audit.ps1). Generation may continue in degraded mode only when stale hierarchy output is acceptable.",
            not_ready_dbnums
        )
    };

    PeOwnerEvidence {
        ready,
        not_ready_dbnums: not_ready_dbnums.clone(),
        summary: serde_json::json!({
            "manifest_version": "incremental_pe_owner_evidence:v1",
            "ready": ready,
            "mode": mode,
            "required": require_pe_owner_ready,
            "sample_limit": SAMPLE_LIMIT,
            "checked_dbnums": checked_dbnums,
            "not_ready_dbnums": not_ready_dbnums,
            "dbnums": per_dbnum,
            "recommendation": recommendation,
        }),
    }
}
