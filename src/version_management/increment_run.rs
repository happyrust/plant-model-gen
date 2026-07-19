//! specs/022 IncrementRun：一次 sesno 增量的完整管线（采集 → 落库/锚点 → 可选生成 → handoff）。
//!
//! 深模块：`run_increment(options)` 一个入口，内部各阶段（source-observation 门、
//! 采集、db_meta 刷新、Version Commit 落库、tree-index 证据、模型生成、Parquet 导出、
//! publication handoff、summary 汇总）全部私有。CLI（`incremental-sesno` /
//! `watch-incremental`）只是薄参数 adapter；连接策略（端口探测/自启动）留在
//! adapter 侧，经 `ensure_model_store` 闭包传入——与 `commit_version` 的 apply
//! 闭包同一模式。

use std::future::Future;
use std::path::{Path, PathBuf};

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
    pub source_observation_manifest: Option<PathBuf>,
    pub source_observation_manifest_hash: Option<String>,
    pub publication_handoff_dir: Option<PathBuf>,
    pub release_id_prefix: Option<String>,
    pub require_tree_index: bool,
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

struct SourceObservationGate {
    evidence: crate::version_management::source_observation::SourceObservationEvidence,
    source_sha256_before: String,
}

#[derive(Debug, Clone)]
struct TreeIndexEvidence {
    ready: bool,
    missing_dbnums: Vec<u32>,
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
        Some("preparing source observation and collection"),
        metrics_elapsed(),
    );
    let source_observation_gate = prepare_source_observation_gate(db_option_ext, &options)?;
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
                source_observation_gate
                    .as_ref()
                    .map(|gate| gate.evidence.manifest_hash.as_str()),
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

    let tree_index_evidence = if options.generate_model {
        let _heartbeat = crate::perf_metrics::start_generate_heartbeat(
            "incremental_sesno_checking_tree_index",
            Some(format!("dbnums={generation_dbnums:?}")),
            std::time::Duration::from_secs(15),
        );
        let evidence = build_tree_index_evidence(
            db_option_ext,
            &generation_dbnums,
            options.require_tree_index,
        )?;
        crate::perf_metrics::record_generate_progress(
            "incremental_sesno_tree_index_checked",
            Some(if evidence.ready {
                "tree_index_ready"
            } else {
                "tree_index_degraded_or_missing"
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
            println!("ℹ️ 未收集到增量元素，跳过模型生成");
            generation_success = Some(false);
        } else {
            if let Some(evidence) = &tree_index_evidence
                && options.require_tree_index
                && !evidence.ready
            {
                anyhow::bail!(
                    "tree_index_missing: --require-tree-index enabled but scene_tree files are missing for dbnums {:?}; checked evidence: {}",
                    evidence.missing_dbnums,
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
                crate::fast_model::gen_all_geos_data(Vec::new(), &gen_db_option_ext, Some(update_log))
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

    let source_observation_summary =
        build_source_observation_summary(&source_observation_gate)?;

    #[cfg(feature = "gen_model")]
    let publication_handoff = {
        let _heartbeat = crate::perf_metrics::start_generate_heartbeat(
            "incremental_sesno_building_handoff",
            Some(format!("dbnums={generation_dbnums:?}")),
            std::time::Duration::from_secs(15),
        );
        build_publication_handoff(
            db_option_ext,
            &options,
            &outcome,
            &persist_stats,
            &generation_dbnums,
            generation_success,
            export_report_opt.as_ref(),
            source_observation_summary.as_ref(),
            tree_index_evidence
                .as_ref()
                .map(|evidence| &evidence.summary),
        )?
    };
    #[cfg(not(feature = "gen_model"))]
    let publication_handoff = disabled_handoff("--generate-model not requested");

    #[cfg(feature = "gen_model")]
    let parquet_export_value: Option<serde_json::Value> = export_report_opt
        .as_ref()
        .map(serde_json::to_value)
        .transpose()?;
    #[cfg(not(feature = "gen_model"))]
    let parquet_export_value: Option<serde_json::Value> = None;

    crate::perf_metrics::record_generate_progress(
        "incremental_sesno_handoff_built",
        publication_handoff
            .as_ref()
            .and_then(|value| value.get("manifest_path"))
            .and_then(|value| value.as_str()),
        metrics_elapsed(),
    );

    let summary = serde_json::json!({
        "from_sesno": options.from_sesno,
        "to_sesno": options.to_sesno,
        "source_observation": source_observation_summary,
        "tree_index": tree_index_evidence.as_ref().map(|evidence| evidence.summary.clone()),
        "publication_handoff": publication_handoff,
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

/// source observation 摘要；重验源文件 hash（"after incremental-sesno"）。
fn build_source_observation_summary(
    gate: &Option<SourceObservationGate>,
) -> anyhow::Result<Option<serde_json::Value>> {
    let Some(gate) = gate else {
        return Ok(None);
    };
    let source_sha256_after =
        crate::version_management::source_observation::verify_source_observation_primary_hash(
            &gate.evidence,
            "after incremental-sesno",
        )?;
    Ok(Some(serde_json::json!({
        "manifest_path": gate.evidence.manifest_path,
        "manifest_hash": gate.evidence.manifest_hash,
        "observation_id": gate.evidence.manifest.observation_id,
        "dbnum": gate.evidence.manifest.dbnum,
        "source_db_file": gate.evidence.manifest.primary.path,
        "resolved_sesno": gate.evidence.manifest.resolved_sesno,
        "quiescence_stable": gate.evidence.manifest.quiescence.stable,
        "primary_sha256": gate.evidence.manifest.primary.sha256,
        "source_sha256_before": gate.source_sha256_before,
        "source_sha256_after": source_sha256_after,
        "source_hash_unchanged": gate.source_sha256_before.eq_ignore_ascii_case(&source_sha256_after),
    })))
}

fn disabled_handoff(reason: &str) -> Option<serde_json::Value> {
    Some(serde_json::json!({
        "enabled": false,
        "policy": "explicit_register_required",
        "reason": reason,
        "side_effect": "no release was registered by incremental-sesno",
    }))
}

fn build_tree_index_evidence(
    db_option_ext: &DbOptionExt,
    generation_dbnums: &[u32],
    require_tree_index: bool,
) -> anyhow::Result<TreeIndexEvidence> {
    let scene_tree_dir = db_option_ext.get_scene_tree_dir();
    let db_meta_info_file = scene_tree_dir.join("db_meta_info.json");
    let db_meta_info_exists = db_meta_info_file.is_file();
    let mut checked_dbnums: Vec<u32> = generation_dbnums.to_vec();
    checked_dbnums.sort_unstable();
    checked_dbnums.dedup();

    let mut files = Vec::new();
    let mut missing_dbnums = Vec::new();
    for dbnum in &checked_dbnums {
        let tree_file = scene_tree_dir.join(format!("{dbnum}.tree"));
        let metadata = std::fs::metadata(&tree_file).ok();
        let exists = metadata.as_ref().map(|m| m.is_file()).unwrap_or(false);
        if !exists {
            missing_dbnums.push(*dbnum);
        }
        let modified_unix_ms = metadata
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64);
        files.push(serde_json::json!({
            "dbnum": dbnum,
            "path": tree_file,
            "exists": exists,
            "bytes": metadata.as_ref().map(|m| m.len()),
            "modified_unix_ms": modified_unix_ms,
        }));
    }

    let ready = missing_dbnums.is_empty();
    let mode = if require_tree_index {
        "strict_required"
    } else if ready {
        "ready"
    } else {
        "degraded_allowed"
    };
    let recommendation = if ready {
        "Tree index files are present for the incremental generation dbnums.".to_string()
    } else if require_tree_index {
        format!(
            "Build or restore scene_tree files for dbnums {:?} before model generation, or rerun without --require-tree-index only for patch_only/quarantined handoff validation.",
            missing_dbnums
        )
    } else {
        format!(
            "Generation may continue in degraded mode, but publication must remain patch_only/quarantined until scene_tree files are built or restored for dbnums {:?}. Do not auto-run long --gen-indextree work from watcher/default incremental paths.",
            missing_dbnums
        )
    };

    Ok(TreeIndexEvidence {
        ready,
        missing_dbnums: missing_dbnums.clone(),
        summary: serde_json::json!({
            "manifest_version": "incremental_tree_index_evidence:v1",
            "ready": ready,
            "mode": mode,
            "required": require_tree_index,
            "scene_tree_dir": scene_tree_dir,
            "db_meta_info_file": db_meta_info_file,
            "db_meta_info_exists": db_meta_info_exists,
            "checked_dbnums": checked_dbnums,
            "missing_dbnums": missing_dbnums,
            "files": files,
            "recommendation": recommendation,
        }),
    })
}

fn prepare_source_observation_gate(
    db_option_ext: &DbOptionExt,
    options: &IncrementRunOptions,
) -> anyhow::Result<Option<SourceObservationGate>> {
    let Some(manifest_path) = &options.source_observation_manifest else {
        return Ok(None);
    };
    let requested_source_count = usize::from(options.file.is_some()) + options.dbnums.len();
    if requested_source_count != 1 {
        anyhow::bail!(
            "--source-observation-manifest currently gates exactly one incremental source; pass one --file or one --dbnum, got {} sources",
            requested_source_count
        );
    }

    let evidence = crate::version_management::source_observation::load_source_observation_manifest(
        manifest_path,
        options.source_observation_manifest_hash.as_deref(),
    )?;
    let observed_dbnum = evidence.manifest.dbnum;
    crate::version_management::source_observation::validate_source_observation_for_increment(
        &evidence,
        &db_option_ext.inner.project_name,
        observed_dbnum,
        options.from_sesno,
        options.to_sesno,
    )?;

    if let Some(file) = &options.file {
        ensure_observation_file_matches(file, &evidence)?;
    }
    if !options.dbnums.is_empty() {
        let requested_dbnum = options.dbnums[0];
        if requested_dbnum != observed_dbnum {
            anyhow::bail!(
                "source observation dbnum {} does not match incremental-sesno --dbnum {}",
                observed_dbnum,
                requested_dbnum
            );
        }
    }

    let source_sha256_before =
        crate::version_management::source_observation::verify_source_observation_primary_hash(
            &evidence,
            "before incremental-sesno",
        )?;
    Ok(Some(SourceObservationGate {
        evidence,
        source_sha256_before,
    }))
}

fn ensure_observation_file_matches(
    file: &Path,
    evidence: &crate::version_management::source_observation::SourceObservationEvidence,
) -> anyhow::Result<()> {
    let requested = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    let observed = std::fs::canonicalize(&evidence.manifest.primary.path)
        .unwrap_or_else(|_| evidence.manifest.primary.path.clone());
    if requested != observed {
        anyhow::bail!(
            "source observation primary file does not match incremental-sesno --file: observed={}, requested={}",
            observed.display(),
            requested.display()
        );
    }
    Ok(())
}

#[cfg(feature = "gen_model")]
fn build_publication_handoff(
    db_option_ext: &DbOptionExt,
    options: &IncrementRunOptions,
    outcome: &crate::data_interface::sesno_increment::PdmsSesnoIncrementOutcome,
    persist_stats: &crate::data_interface::sesno_increment::PdmsIncrementPersistStats,
    generation_dbnums: &[u32],
    generation_success: Option<bool>,
    parquet_export: Option<
        &crate::fast_model::export_model::post_gen_export::PostGenerationParquetExportReport,
    >,
    source_observation_summary: Option<&serde_json::Value>,
    tree_index_summary: Option<&serde_json::Value>,
) -> anyhow::Result<Option<serde_json::Value>> {
    if !options.generate_model {
        return Ok(disabled_handoff("--generate-model not requested"));
    }
    if generation_success != Some(true) {
        return Ok(disabled_handoff("model generation did not complete successfully"));
    }
    let Some(export) = parquet_export else {
        return Ok(disabled_handoff("post-generation Parquet export did not run"));
    };
    if !export.enabled {
        return Ok(disabled_handoff(
            export
                .skipped_reason
                .as_deref()
                .unwrap_or("post-generation Parquet export is disabled"),
        ));
    }
    if let Some(reason) = export.skipped_reason.as_deref() {
        return Ok(disabled_handoff(reason));
    }
    let Some(output_dir) = export.output_dir.as_ref() else {
        return Ok(disabled_handoff(
            "post-generation Parquet export did not report output_dir",
        ));
    };
    if export.exported_dbnums.is_empty() {
        return Ok(disabled_handoff(
            "post-generation Parquet export reported no exported dbnums",
        ));
    }

    let actual_to_sesno = outcome
        .files
        .iter()
        .map(|file| file.actual_end_sesno)
        .max()
        .or(options.to_sesno)
        .unwrap_or(options.from_sesno);
    let handoff_dir = options.publication_handoff_dir.clone().unwrap_or_else(|| {
        db_option_ext
            .get_project_output_dir()
            .join("model_versions")
            .join("runs")
            .join("incremental_publication_handoffs")
    });
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%3fZ").to_string();
    let dbnum_tag = if export.exported_dbnums.len() == 1 {
        export.exported_dbnums[0].to_string()
    } else {
        export
            .exported_dbnums
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join("-")
    };
    let run_id = format!(
        "incremental-db{}-{}-to-{}-{}",
        dbnum_tag, options.from_sesno, actual_to_sesno, timestamp
    );
    let manifest_path = handoff_dir.join(format!("{run_id}.json"));

    let config_arg =
        std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption".to_string());
    let executable = std::env::current_exe()
        .ok()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "aios-database".to_string());
    let release_id_prefix = sanitize_release_id_fragment(
        options
            .release_id_prefix
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("incremental-sesno"),
    );
    if release_id_prefix.is_empty() {
        anyhow::bail!("release-id-prefix produces an empty path-safe fragment");
    }

    let mut candidates = Vec::new();
    for dbnum in &export.exported_dbnums {
        let parquet_dir = output_dir.join(dbnum.to_string());
        let package = crate::version_management::release_package::load_model_package(
            &parquet_dir,
            *dbnum,
        )
        .map_err(|err| {
            anyhow::anyhow!(
                "post-generation handoff cannot load candidate package for dbnum {} at {}: {}",
                dbnum,
                parquet_dir.display(),
                err
            )
        })?;
        let package_hash_short = package.package_hash.chars().take(12).collect::<String>();
        let suggested_release_id = format!(
            "{}-db{}-sesno{}-pkg{}",
            release_id_prefix, dbnum, actual_to_sesno, package_hash_short
        );
        crate::version_management::release_package::validate_release_id_for_path(
            &suggested_release_id,
        )?;

        let metadata = serde_json::json!({
            "source": "incremental-sesno publication handoff",
            "project_name": db_option_ext.inner.project_name,
            "dbnum": dbnum,
            "from_sesno": options.from_sesno,
            "to_sesno": actual_to_sesno,
            "source_observation": source_observation_summary,
            "incremental": {
                "file_count": outcome.files.len(),
                "session_count": outcome.total_session_count(),
                "element_count": outcome.total_element_count(),
                "data_persist": persist_stats,
                "category_counts": {
                    "prim": outcome.update_log.prim_refnos.len(),
                    "loop_owner": outcome.update_log.loop_owner_refnos.len(),
                    "bran_hanger": outcome.update_log.bran_hanger_refnos.len(),
                    "basic_cata": outcome.update_log.basic_cata_refnos.len(),
                    "delete": outcome.update_log.delete_refnos.len(),
                    "total": outcome.update_log.count(),
                },
            },
            "generation": {
                "success": generation_success,
                "generation_dbnums": generation_dbnums,
                "parquet_export": export,
                "tree_index": tree_index_summary,
            },
            "candidate_package": {
                "source_parquet_dir": parquet_dir,
                "package_hash": package.package_hash,
                "rows_by_table": package.rows_by_table,
            },
            "publication_policy": {
                "release_registration_is_explicit": true,
                "register_copies_mutable_parquet_to_immutable_release_package": true,
                "incremental_sesno_does_not_write_model_release_catalog": true,
                "suggested_release_quality": "patch_only",
                "reason": "incremental-sesno generates the affected scope, not a proven full visual baseline",
            }
        });
        let metadata_json = serde_json::to_string(&metadata)?;
        let register_argv = vec![
            executable.clone(),
            "-c".to_string(),
            config_arg.clone(),
            "model-version".to_string(),
            "register".to_string(),
            "--release-id".to_string(),
            suggested_release_id.clone(),
            "--dbnum".to_string(),
            dbnum.to_string(),
            "--parquet-dir".to_string(),
            parquet_dir.display().to_string(),
            "--derivation-type".to_string(),
            "incremental-sesno".to_string(),
            "--release-quality".to_string(),
            "patch_only".to_string(),
            "--release-quality-reason".to_string(),
            "incremental-sesno handoff contains the generated affected scope; verify or hydrate a full baseline package before publishing as a complete visual release".to_string(),
            "--validation-flag".to_string(),
            "incremental_handoff_affected_scope".to_string(),
            "--validation-flag".to_string(),
            "explicit_release_registration_required".to_string(),
            "--metadata-json".to_string(),
            metadata_json,
            "--json".to_string(),
        ];

        candidates.push(serde_json::json!({
            "dbnum": dbnum,
            "source_parquet_dir": parquet_dir,
            "package_hash": package.package_hash,
            "rows_by_table": package.rows_by_table,
            "suggested_release_id": suggested_release_id,
            "register_argv": register_argv,
            "register_command": command_to_shell_string_for_handoff(&register_argv),
            "suggested_release_quality": "patch_only",
            "next_step": "review the affected-scope package, then run register_argv to copy it into an immutable patch-only release; hydrate or validate a full baseline before complete_visual publication",
        }));
    }

    let handoff = serde_json::json!({
        "manifest_version": "incremental_publication_handoff:v1",
        "run_id": run_id,
        "project_name": db_option_ext.inner.project_name,
        "from_sesno": options.from_sesno,
        "to_sesno": actual_to_sesno,
        "policy": "explicit_register_required",
        "source_observation": source_observation_summary,
        "tree_index": tree_index_summary,
        "generation_success": generation_success,
        "parquet_export": export,
        "candidates": candidates,
    });
    let manifest_hash = write_json_manifest_atomic(&manifest_path, &handoff)?;

    Ok(Some(serde_json::json!({
        "enabled": true,
        "policy": "explicit_register_required",
        "manifest_path": manifest_path,
        "manifest_hash": manifest_hash,
        "candidate_count": candidates.len(),
        "candidates": candidates,
        "side_effect": "handoff manifest written; no release was registered by incremental-sesno",
    })))
}

#[cfg(feature = "gen_model")]
fn write_json_manifest_atomic(path: &Path, value: &serde_json::Value) -> anyhow::Result<String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            anyhow::anyhow!(
                "create handoff manifest dir failed: {}: {}",
                parent.display(),
                err
            )
        })?;
    }
    let tmp = path.with_extension(format!("json.tmp-{}", std::process::id()));
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?).map_err(|err| {
        anyhow::anyhow!(
            "write temporary handoff manifest failed: {}: {}",
            tmp.display(),
            err
        )
    })?;
    if path.exists() {
        std::fs::remove_file(path).map_err(|err| {
            anyhow::anyhow!(
                "remove previous handoff manifest failed: {}: {}",
                path.display(),
                err
            )
        })?;
    }
    std::fs::rename(&tmp, path).map_err(|err| {
        anyhow::anyhow!(
            "replace handoff manifest failed: {}: {}",
            path.display(),
            err
        )
    })?;
    crate::version_management::hashing::sha256_file(path)
        .map_err(|err| anyhow::anyhow!("hash handoff manifest failed: {}: {}", path.display(), err))
}

#[cfg(feature = "gen_model")]
fn sanitize_release_id_fragment(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

#[cfg(feature = "gen_model")]
fn command_to_shell_string_for_handoff(argv: &[String]) -> String {
    argv.iter()
        .map(|arg| {
            if arg.chars().all(|ch| {
                ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '/' | '\\' | ':')
            }) {
                arg.clone()
            } else {
                format!("\"{}\"", arg.replace('"', "\\\""))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
