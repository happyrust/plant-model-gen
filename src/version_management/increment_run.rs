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

/// 一次增量运行的全部输入。区间起点语义：从 `from_sesno + 1` 收集到
/// `to_sesno`（缺省为文件最新 sesno）。
#[derive(Debug, Clone)]
pub struct IncrementRunOptions {
    pub file: Option<PathBuf>,
    pub dbnums: Vec<u32>,
    pub from_sesno: u32,
    pub to_sesno: Option<u32>,
    pub rescan_index: bool,
    pub persist_data: bool,
    pub recover_pending: bool,
    pub generate_model: bool,
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
struct PeOwnerEvidence {
    ready: bool,
    not_ready_dbnums: Vec<u32>,
    summary: serde_json::Value,
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
        let file_outcome = crate::data_interface::sesno_increment::collect_pdms_increment_for_file_with_operations(
                &db_option_ext.inner.project_name,
                file.clone(),
                options.from_sesno,
                options.to_sesno,
                options.verbose,
            )?;
        crate::perf_metrics::record_generate_progress(
            "incremental_sesno_collected_file",
            Some(&detail),
            metrics_elapsed(),
        );
        collected_outcome.merge(file_outcome);
    }

    if !options.dbnums.is_empty() {
        source_count += options.dbnums.len();
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
            let detail = format!("dbnums={:?}", options.dbnums);
            let _heartbeat = crate::perf_metrics::start_generate_heartbeat(
                "incremental_sesno_collecting_dbnums",
                Some(detail.clone()),
                std::time::Duration::from_secs(15),
            );
            let indexed_outcome = match crate::data_interface::sesno_increment::collect_pdms_increment_for_dbnums_from_index_with_operations(
                &db_option_ext.inner.project_name,
                &index_path,
                &options.dbnums,
                options.from_sesno,
                options.to_sesno,
                options.verbose,
            ) {
                Ok(outcome) => outcome,
                Err(err) if !options.rescan_index => {
                    eprintln!("⚠️  db_index 命中失败，按指纹刷新索引后重试: {}", err);
                    let report =
                        crate::data_interface::db_index::rebuild_from_config(false)
                            .await?;
                    println!(
                        "✅ db_index 已刷新: {} 个库, {} 条 ref0 映射",
                        report.db_files, report.ref0_total
                    );
                    crate::data_interface::sesno_increment::collect_pdms_increment_for_dbnums_from_index_with_operations(
                        &db_option_ext.inner.project_name,
                        &index_path,
                        &options.dbnums,
                        options.from_sesno,
                        options.to_sesno,
                        options.verbose,
                    )?
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
        if !stats.commit_failures.is_empty() {
            let partial = serde_json::json!({
                "committed_anchors": &stats.anchors,
                "failed_commits": &stats.commit_failures,
            });
            anyhow::bail!(
                "one or more per-dbnum version commits failed; no model generation was started: {}",
                serde_json::to_string(&partial)?
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

    let pe_owner_evidence = if options.generate_model {
        let _heartbeat = crate::perf_metrics::start_generate_heartbeat(
            "incremental_sesno_checking_pe_owner",
            Some(format!("dbnums={generation_dbnums:?}")),
            std::time::Duration::from_secs(15),
        );
        let evidence =
            build_pe_owner_evidence(&generation_dbnums, options.require_pe_owner_ready).await;
        crate::perf_metrics::record_generate_progress(
            "incremental_sesno_pe_owner_checked",
            Some(if evidence.ready {
                "pe_owner_ready"
            } else {
                "pe_owner_degraded_or_missing"
            }),
            metrics_elapsed(),
        );
        Some(evidence)
    } else {
        None
    };

    let mut generation_success = None;
    #[cfg(feature = "gen_model")]
    let mut export_report_opt: Option<
        crate::fast_model::export_model::post_gen_export::PostGenerationParquetExportReport,
    > = None;

    #[cfg(feature = "gen_model")]
    if options.generate_model {
        let update_log = outcome.update_log.clone();
        if update_log.count() == 0 {
            println!("ℹ️ 未收集到模型变更，模型生成按成功空操作收尾");
            generation_success = Some(true);
        } else {
            if let Some(evidence) = &pe_owner_evidence
                && options.require_pe_owner_ready
                && !evidence.ready
            {
                anyhow::bail!(
                    "pe_owner_not_ready: --require-pe-owner-ready enabled but pe_owner integrity evidence is not ready for dbnums {:?}; run `model-version rebuild-pe-owner --dbnum <n>` (audit: scripts/smoke/pe_owner_children_audit.ps1); checked evidence: {}",
                    evidence.not_ready_dbnums,
                    serde_json::to_string(&evidence.summary)?
                );
            }
            let mut gen_db_option_ext = db_option_ext.clone();
            if !generation_dbnums.is_empty() {
                gen_db_option_ext.inner.manual_db_nums = Some(generation_dbnums.clone());
                println!(
                    "🔧 增量模型生成限定 manual_db_nums -> {:?}",
                    generation_dbnums
                );
            }
            let generate_started = std::time::Instant::now();
            crate::perf_metrics::record_generate_progress(
                "incremental_sesno_generate_started",
                Some("incremental-sesno"),
                0,
            );
            let gen_result = {
                let _heartbeat = crate::perf_metrics::start_generate_heartbeat(
                    "incremental_sesno_generate_running",
                    Some(format!("dbnums={generation_dbnums:?}")),
                    std::time::Duration::from_secs(15),
                );
                crate::fast_model::gen_all_geos_data(
                    Vec::new(),
                    &gen_db_option_ext,
                    Some(update_log),
                )
                .await
            };
            let generate_ms = generate_started.elapsed().as_millis() as u64;
            match &gen_result {
                Ok(_) => crate::perf_metrics::record_generate_progress(
                    "incremental_sesno_generate_finished",
                    Some("incremental-sesno"),
                    generate_ms,
                ),
                Err(err) => crate::perf_metrics::record_generate_progress(
                    "incremental_sesno_generate_failed",
                    Some(&err.to_string()),
                    generate_ms,
                ),
            }
            crate::perf_metrics::finish_generate_stage_from_db(generate_ms).await;
            let gen_result = gen_result?;
            generation_success = Some(gen_result.success);
            let export_report = {
                let _heartbeat = crate::perf_metrics::start_generate_heartbeat(
                    "incremental_sesno_exporting_parquet",
                    Some(format!("dbnums={generation_dbnums:?}")),
                    std::time::Duration::from_secs(15),
                );
                crate::fast_model::export_model::post_gen_export::export_parquet_after_generation_if_enabled(
                    &gen_db_option_ext,
                    Some(generation_dbnums.clone()),
                )
                .await?
            };
            crate::perf_metrics::record_generate_progress(
                "incremental_sesno_parquet_export_checked",
                Some(if export_report.enabled {
                    "post_generation_export_enabled"
                } else {
                    "post_generation_export_disabled"
                }),
                metrics_elapsed(),
            );
            if export_report.enabled {
                println!(
                    "✅ 生成后 Parquet 导出: dbnums={:?} skipped={:?}",
                    export_report.exported_dbnums, export_report.skipped_reason
                );
            }
            export_report_opt = Some(export_report);
        }
    }

    let source_hash_summary = verify_source_hash_gate(&source_hash_gate)?;

    let mut model_gen_anchors: Vec<crate::versioned_db::version_commit::ModelGenAnchor> =
        Vec::new();
    #[cfg(feature = "gen_model")]
    if generation_success == Some(true)
        && db_option_ext.use_surrealdb
        && db_option_ext.model_writer_mode.writes_to_surreal()
        && !db_option_ext.gen_model_dry_run
    {
        let generation_dbnums = generation_dbnums
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        let mut committed_sesnos = std::collections::BTreeMap::new();
        for anchor in &persist_stats.anchors {
            if generation_dbnums.contains(&anchor.dbnum) {
                committed_sesnos
                    .entry(anchor.dbnum)
                    .and_modify(|sesno: &mut u32| *sesno = (*sesno).max(anchor.sesno))
                    .or_insert(anchor.sesno);
            }
        }
        for (dbnum, sesno) in committed_sesnos {
            let anchor =
                crate::versioned_db::version_commit::write_model_gen_anchor(dbnum, sesno).await?;
            println!(
                "✅ model_gen 锚点已发布: dbnum={} sesno={} anchored_at={}",
                anchor.dbnum, anchor.sesno, anchor.anchored_at
            );
            model_gen_anchors.push(anchor);
        }
    }

    #[cfg(feature = "gen_model")]
    let parquet_export_value: Option<serde_json::Value> = export_report_opt
        .as_ref()
        .map(serde_json::to_value)
        .transpose()?;
    #[cfg(not(feature = "gen_model"))]
    let parquet_export_value: Option<serde_json::Value> = None;

    let summary = serde_json::json!({
        "from_sesno": options.from_sesno,
        "to_sesno": options.to_sesno,
        "source_hash_gate": source_hash_summary,
        "pe_owner_evidence": pe_owner_evidence.as_ref().map(|evidence| evidence.summary.clone()),
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
async fn build_pe_owner_evidence(
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
