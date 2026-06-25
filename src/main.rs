#![feature(let_chains)]
#![feature(duration_constructors)]
// 暂时屏蔽warnings
#![allow(warnings)]
#![recursion_limit = "256"]

#[macro_use]
extern crate clap;
#[macro_use]
extern crate nom;

extern crate strum;

use std::path::{Path, PathBuf};

use chrono::{Datelike, Local, Timelike};

#[cfg(not(feature = "gui"))]
mod cli_modes;

#[cfg(not(feature = "gui"))]
fn parse_lod_level(s: &str) -> Option<aios_core::mesh_precision::LodLevel> {
    use aios_core::mesh_precision::LodLevel;
    match s.trim().to_ascii_uppercase().as_str() {
        "L0" => Some(LodLevel::L0),
        "L1" => Some(LodLevel::L1),
        "L2" => Some(LodLevel::L2),
        "L3" => Some(LodLevel::L3),
        "L4" => Some(LodLevel::L4),
        _ => None,
    }
}

/// 构建导出配置的辅助函数
fn build_export_config(
    refnos_vec: Vec<String>,
    output_path: Option<String>,
    filter_nouns: Option<Vec<String>>,
    include_descendants: bool,
    source_unit: &str,
    target_unit: &str,
    verbose: bool,
    regenerate_plant_mesh: bool,
    dbnum: Option<u32>,
    split_by_site: bool,
    include_negative: bool,
    export_svg: bool,
) -> ExportConfig {
    let run_all_dbnos = refnos_vec.is_empty() && dbnum.is_none();
    ExportConfig {
        refnos_str: refnos_vec,
        output_path,
        filter_nouns,
        include_descendants,
        source_unit: source_unit.to_string(),
        target_unit: target_unit.to_string(),
        verbose,
        regenerate_plant_mesh,
        dbnum,
        use_basic_materials: false,
        run_all_dbnos,
        split_by_site,
        include_negative,
        export_svg,
    }
}

#[cfg(not(feature = "gui"))]
fn parse_cli_refno(refno: &str) -> anyhow::Result<aios_core::pdms_types::RefnoEnum> {
    use aios_core::pdms_types::RefnoEnum;
    use std::str::FromStr;

    let normalized = refno.replace('_', "/");
    RefnoEnum::from_str(&normalized)
        .map_err(|e| anyhow::anyhow!("解析参考号失败: {} ({})", refno, e))
}

#[cfg(not(feature = "gui"))]
fn format_cli_refno(refno: aios_core::pdms_types::RefnoEnum) -> String {
    refno.to_string().replace('/', "_")
}

#[cfg(not(feature = "gui"))]
fn parse_cli_model_record_refno(
    refno: &str,
    sesno: Option<u32>,
) -> anyhow::Result<aios_core::pdms_types::RefnoEnum> {
    let parsed = parse_cli_refno(refno)?;
    Ok(match sesno {
        Some(sesno) => {
            let base = parsed.refno();
            aios_core::pdms_types::RefnoEnum::from((base, sesno))
        }
        None => parsed,
    })
}

#[cfg(not(feature = "gui"))]
async fn promote_generation_refnos_to_bran_hang_roots(
    refnos: &[String],
    verbose: bool,
) -> anyhow::Result<Vec<String>> {
    use aios_database::fast_model::gen_model::query_compat::query_filter_ancestors;
    use aios_database::fast_model::gen_model::tree_index_manager::TreeIndexManager;
    use std::collections::{BTreeSet, HashMap};

    const BRAN_HANG_NOUNS: &[&str] = &["BRAN", "HANG"];

    let mut promoted_refnos = Vec::new();
    let mut seen = BTreeSet::new();
    let mut tree_managers: HashMap<u32, TreeIndexManager> = HashMap::new();

    for input_refno in refnos {
        let parsed_refno = parse_cli_refno(input_refno)?;
        let dbnum = TreeIndexManager::resolve_dbnum_for_refno(parsed_refno)?;
        let manager = tree_managers
            .entry(dbnum)
            .or_insert_with(|| TreeIndexManager::with_default_dir(vec![dbnum]));
        let self_noun = manager
            .get_noun(parsed_refno)
            .unwrap_or_default()
            .trim()
            .to_ascii_uppercase();

        let promoted_refno = if matches!(self_noun.as_str(), "BRAN" | "HANG") {
            parsed_refno
        } else {
            query_filter_ancestors(parsed_refno, BRAN_HANG_NOUNS)
                .await?
                .last()
                .copied()
                .unwrap_or(parsed_refno)
        };

        let promoted_refno_str = format_cli_refno(promoted_refno);
        if promoted_refno != parsed_refno {
            println!(
                "🔧 生成目标提升到最近 BRAN/HANG 根: {} -> {}",
                input_refno, promoted_refno_str
            );
        } else if verbose {
            println!("🔧 生成目标保持原 refno: {}", input_refno);
        }

        if seen.insert(promoted_refno_str.clone()) {
            promoted_refnos.push(promoted_refno_str);
        }
    }

    if promoted_refnos.len() < refnos.len() {
        println!(
            "🔁 生成目标去重完成: 输入 {} 个，提升后 {} 个",
            refnos.len(),
            promoted_refnos.len()
        );
    }

    Ok(promoted_refnos)
}

#[cfg(not(feature = "gui"))]
#[derive(Debug, Clone)]
struct IncrementalSesnoRunOptions {
    file: Option<PathBuf>,
    dbnums: Vec<u32>,
    from_sesno: u32,
    to_sesno: Option<u32>,
    rescan_index: bool,
    persist_data: bool,
    generate_model: bool,
    source_observation_manifest: Option<PathBuf>,
    source_observation_manifest_hash: Option<String>,
    publication_handoff_dir: Option<PathBuf>,
    release_id_prefix: Option<String>,
    require_tree_index: bool,
    verbose: bool,
}

#[cfg(not(feature = "gui"))]
struct IncrementalSesnoRunResult {
    summary: serde_json::Value,
    outcome: aios_database::data_interface::sesno_increment::PdmsSesnoIncrementOutcome,
    persist_stats: aios_database::data_interface::sesno_increment::PdmsIncrementPersistStats,
    generation_success: Option<bool>,
    parquet_export: Option<
        aios_database::fast_model::export_model::post_gen_export::PostGenerationParquetExportReport,
    >,
}

#[cfg(not(feature = "gui"))]
struct IncrementalSourceObservationGate {
    evidence: aios_database::version_management::source_observation::SourceObservationEvidence,
    source_sha256_before: String,
}

#[cfg(not(feature = "gui"))]
#[derive(Debug, Clone)]
struct IncrementalTreeIndexEvidence {
    ready: bool,
    missing_dbnums: Vec<u32>,
    summary: serde_json::Value,
}

#[cfg(all(not(feature = "gui"), feature = "sqlite-index"))]
struct WatchSourceObservationGate {
    manifest_path: PathBuf,
    manifest_hash: String,
}

#[cfg(not(feature = "gui"))]
async fn run_incremental_sesno_once(
    db_option_ext: &aios_database::options::DbOptionExt,
    options: IncrementalSesnoRunOptions,
) -> anyhow::Result<IncrementalSesnoRunResult> {
    let run_started = std::time::Instant::now();
    let metrics_elapsed = || run_started.elapsed().as_millis() as u64;
    if !options.persist_data && options.generate_model {
        anyhow::bail!(
            "--no-persist cannot be combined with --generate-model; incremental model generation requires persisted PE/ATT data"
        );
    }
    aios_database::perf_metrics::record_generate_progress(
        "incremental_sesno_started",
        Some("preparing source observation and collection"),
        metrics_elapsed(),
    );
    let source_observation_gate =
        prepare_incremental_source_observation_gate(db_option_ext, &options)?;
    let mut collected_outcome =
        aios_database::data_interface::sesno_increment::PdmsSesnoCollectedOutcome::default();
    let mut source_count = 0usize;

    if let Some(file) = options.file.as_ref() {
        source_count += 1;
        let detail = format!("file={}", file.display());
        let _heartbeat = aios_database::perf_metrics::start_generate_heartbeat(
            "incremental_sesno_collecting_file",
            Some(detail.clone()),
            std::time::Duration::from_secs(15),
        );
        let file_outcome = aios_database::data_interface::sesno_increment::collect_pdms_increment_for_file_with_operations(
                &db_option_ext.inner.project_name,
                file.clone(),
                options.from_sesno,
                options.to_sesno,
                options.verbose,
            )?;
        aios_database::perf_metrics::record_generate_progress(
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
            let index_path = aios_database::data_interface::db_index::default_index_path(
                &db_option_ext.inner.project_name,
            );
            if options.rescan_index || !index_path.exists() {
                let _heartbeat = aios_database::perf_metrics::start_generate_heartbeat(
                    "incremental_sesno_rebuilding_db_index",
                    Some(format!("index_path={}", index_path.display())),
                    std::time::Duration::from_secs(15),
                );
                let report =
                    aios_database::data_interface::db_index::rebuild_from_config(false).await?;
                println!(
                    "✅ db_index 已刷新: {} 个库, {} 条 ref0 映射",
                    report.db_files, report.ref0_total
                );
            }
            let detail = format!("dbnums={:?}", options.dbnums);
            let _heartbeat = aios_database::perf_metrics::start_generate_heartbeat(
                "incremental_sesno_collecting_dbnums",
                Some(detail.clone()),
                std::time::Duration::from_secs(15),
            );
            let indexed_outcome = match aios_database::data_interface::sesno_increment::collect_pdms_increment_for_dbnums_from_index_with_operations(
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
                        aios_database::data_interface::db_index::rebuild_from_config(false)
                            .await?;
                    println!(
                        "✅ db_index 已刷新: {} 个库, {} 条 ref0 映射",
                        report.db_files, report.ref0_total
                    );
                    aios_database::data_interface::sesno_increment::collect_pdms_increment_for_dbnums_from_index_with_operations(
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
            aios_database::perf_metrics::record_generate_progress(
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

    let aios_database::data_interface::sesno_increment::PdmsSesnoCollectedOutcome {
        outcome,
        files: collected_increment_files,
    } = collected_outcome;

    let db_meta_refreshed_files = if options.persist_data {
        let refreshed = {
            let _heartbeat = aios_database::perf_metrics::start_generate_heartbeat(
                "incremental_sesno_refreshing_db_meta",
                Some(format!("files={}", outcome.files.len())),
                std::time::Duration::from_secs(15),
            );
            aios_database::data_interface::sesno_increment::refresh_db_meta_for_increment_files(
                &db_option_ext.inner.project_name,
                &outcome.files,
            )?
        };
        aios_database::perf_metrics::record_generate_progress(
            "incremental_sesno_db_meta_refreshed",
            Some(&format!("files={refreshed}")),
            metrics_elapsed(),
        );
        if refreshed > 0 {
            println!("✅ 增量 db_meta 已刷新: {} 个 db 文件", refreshed);
        }
        refreshed
    } else {
        aios_database::perf_metrics::record_generate_progress(
            "incremental_sesno_db_meta_refresh_skipped",
            Some("--no-persist requested"),
            metrics_elapsed(),
        );
        0
    };

    let persist_stats = if options.persist_data {
        {
            let _heartbeat = aios_database::perf_metrics::start_generate_heartbeat(
                "incremental_sesno_connecting_model_store",
                Some("ensure_surreal_connected".to_string()),
                std::time::Duration::from_secs(15),
            );
            crate::cli_modes::ensure_surreal_connected(db_option_ext).await?;
        }
        aios_database::perf_metrics::record_generate_progress(
            "incremental_sesno_model_store_connected",
            Some("ensure_surreal_connected"),
            metrics_elapsed(),
        );
        let stats = {
            let _heartbeat = aios_database::perf_metrics::start_generate_heartbeat(
                "incremental_sesno_persisting",
                Some(format!(
                    "files={} reused_collected_operations=true",
                    collected_increment_files.len()
                )),
                std::time::Duration::from_secs(15),
            );
            aios_database::data_interface::sesno_increment::persist_collected_pdms_increment_files(
                &collected_increment_files,
            )
            .await?
        };
        aios_database::perf_metrics::record_generate_progress(
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
        aios_database::perf_metrics::record_generate_progress(
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

    let update_log = outcome.update_log.clone();
    let tree_index_evidence = if options.generate_model {
        let _heartbeat = aios_database::perf_metrics::start_generate_heartbeat(
            "incremental_sesno_checking_tree_index",
            Some(format!("dbnums={generation_dbnums:?}")),
            std::time::Duration::from_secs(15),
        );
        let evidence = build_incremental_tree_index_evidence(
            db_option_ext,
            &generation_dbnums,
            options.require_tree_index,
        )?;
        aios_database::perf_metrics::record_generate_progress(
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
    let mut parquet_export = None;
    if options.generate_model {
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
            aios_database::perf_metrics::record_generate_progress(
                "incremental_sesno_generate_started",
                Some("incremental-sesno"),
                0,
            );
            let gen_result = {
                let _heartbeat = aios_database::perf_metrics::start_generate_heartbeat(
                    "incremental_sesno_generate_running",
                    Some(format!("dbnums={generation_dbnums:?}")),
                    std::time::Duration::from_secs(15),
                );
                aios_database::fast_model::gen_all_geos_data(
                    Vec::new(),
                    &gen_db_option_ext,
                    Some(update_log),
                    None,
                )
                .await
            };
            let generate_ms = generate_started.elapsed().as_millis() as u64;
            match &gen_result {
                Ok(_) => aios_database::perf_metrics::record_generate_progress(
                    "incremental_sesno_generate_finished",
                    Some("incremental-sesno"),
                    generate_ms,
                ),
                Err(err) => aios_database::perf_metrics::record_generate_progress(
                    "incremental_sesno_generate_failed",
                    Some(&err.to_string()),
                    generate_ms,
                ),
            }
            aios_database::perf_metrics::finish_generate_stage_from_model_store(generate_ms).await;
            let gen_result = gen_result?;
            generation_success = Some(gen_result.success);
            let export_report = {
                let _heartbeat = aios_database::perf_metrics::start_generate_heartbeat(
                    "incremental_sesno_exporting_parquet",
                    Some(format!("dbnums={generation_dbnums:?}")),
                    std::time::Duration::from_secs(15),
                );
                aios_database::fast_model::export_model::post_gen_export::export_parquet_after_generation_if_enabled(
                    &gen_db_option_ext,
                    Some(generation_dbnums.clone()),
                )
                .await?
            };
            aios_database::perf_metrics::record_generate_progress(
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
            parquet_export = Some(export_report);
        }
    }

    let source_observation_summary = if let Some(gate) = &source_observation_gate {
        let source_sha256_after =
            aios_database::version_management::source_observation::verify_source_observation_primary_hash(
                &gate.evidence,
                "after incremental-sesno",
            )?;
        Some(serde_json::json!({
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
        }))
    } else {
        None
    };

    let publication_handoff = {
        let _heartbeat = aios_database::perf_metrics::start_generate_heartbeat(
            "incremental_sesno_building_handoff",
            Some(format!("dbnums={generation_dbnums:?}")),
            std::time::Duration::from_secs(15),
        );
        build_incremental_publication_handoff(
            db_option_ext,
            &options,
            &outcome,
            &persist_stats,
            &generation_dbnums,
            generation_success,
            parquet_export.as_ref(),
            source_observation_summary.as_ref(),
            tree_index_evidence
                .as_ref()
                .map(|evidence| &evidence.summary),
        )?
    };
    aios_database::perf_metrics::record_generate_progress(
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
        "data_persist_skipped_reason": if options.persist_data { serde_json::Value::Null } else { serde_json::json!("--no-persist requested") },
        "data_persist": persist_stats,
        "generation_dbnums": generation_dbnums,
        "generation_success": generation_success,
        "parquet_export": parquet_export,
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

    Ok(IncrementalSesnoRunResult {
        summary,
        outcome,
        persist_stats,
        generation_success,
        parquet_export,
    })
}

#[cfg(not(feature = "gui"))]
fn build_incremental_tree_index_evidence(
    db_option_ext: &aios_database::options::DbOptionExt,
    generation_dbnums: &[u32],
    require_tree_index: bool,
) -> anyhow::Result<IncrementalTreeIndexEvidence> {
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

    Ok(IncrementalTreeIndexEvidence {
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

#[cfg(not(feature = "gui"))]
fn prepare_incremental_source_observation_gate(
    db_option_ext: &aios_database::options::DbOptionExt,
    options: &IncrementalSesnoRunOptions,
) -> anyhow::Result<Option<IncrementalSourceObservationGate>> {
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

    let evidence =
        aios_database::version_management::source_observation::load_source_observation_manifest(
            manifest_path,
            options.source_observation_manifest_hash.as_deref(),
        )?;
    let observed_dbnum = evidence.manifest.dbnum;
    aios_database::version_management::source_observation::validate_source_observation_for_increment(
        &evidence,
        &db_option_ext.inner.project_name,
        observed_dbnum,
        options.from_sesno,
        options.to_sesno,
    )?;

    if let Some(file) = &options.file {
        ensure_incremental_observation_file_matches(file, &evidence)?;
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
        aios_database::version_management::source_observation::verify_source_observation_primary_hash(
            &evidence,
            "before incremental-sesno",
        )?;
    Ok(Some(IncrementalSourceObservationGate {
        evidence,
        source_sha256_before,
    }))
}

#[cfg(not(feature = "gui"))]
fn ensure_incremental_observation_file_matches(
    file: &Path,
    evidence: &aios_database::version_management::source_observation::SourceObservationEvidence,
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

#[cfg(all(not(feature = "gui"), feature = "sqlite-index"))]
fn build_watch_source_observation_gate(
    db_option_ext: &aios_database::options::DbOptionExt,
    rec: &aios_database::data_interface::db_index::DbFileRecord,
    from_sesno: u32,
    to_sesno: u32,
    observation_dir: &Path,
    quiescence_window_ms: u64,
) -> anyhow::Result<WatchSourceObservationGate> {
    let observation_id = format!(
        "watch-db{}-{}-to-{}-{}",
        rec.dbnum,
        from_sesno,
        to_sesno,
        chrono::Utc::now().format("%Y%m%dT%H%M%S%3fZ")
    );
    let manifest_path = observation_dir.join(format!("{observation_id}.json"));
    let manifest =
        aios_database::version_management::source_observation::build_source_observation_manifest(
            aios_database::version_management::source_observation::SourceObservationBuildRequest {
                observation_id,
                project_name: db_option_ext.inner.project_name.clone(),
                dbnum: rec.dbnum,
                primary_file: PathBuf::from(&rec.file_path),
                dependency_files: Vec::new(),
                requested_sesno: Some(format!("{}..{}", from_sesno + 1, to_sesno)),
                resolved_sesno: Some(to_sesno),
                quiescence_window_ms,
            },
        )?;
    let manifest_hash =
        aios_database::version_management::source_observation::write_source_observation_manifest(
            &manifest_path,
            &manifest,
        )?;
    Ok(WatchSourceObservationGate {
        manifest_path,
        manifest_hash,
    })
}

#[cfg(not(feature = "gui"))]
fn build_incremental_publication_handoff(
    db_option_ext: &aios_database::options::DbOptionExt,
    options: &IncrementalSesnoRunOptions,
    outcome: &aios_database::data_interface::sesno_increment::PdmsSesnoIncrementOutcome,
    persist_stats: &aios_database::data_interface::sesno_increment::PdmsIncrementPersistStats,
    generation_dbnums: &[u32],
    generation_success: Option<bool>,
    parquet_export: Option<
        &aios_database::fast_model::export_model::post_gen_export::PostGenerationParquetExportReport,
    >,
    source_observation_summary: Option<&serde_json::Value>,
    tree_index_summary: Option<&serde_json::Value>,
) -> anyhow::Result<Option<serde_json::Value>> {
    let disabled = |reason: &str| {
        Ok(Some(serde_json::json!({
            "enabled": false,
            "policy": "explicit_register_required",
            "reason": reason,
            "side_effect": "no release was registered by incremental-sesno",
        })))
    };

    if !options.generate_model {
        return disabled("--generate-model not requested");
    }
    if generation_success != Some(true) {
        return disabled("model generation did not complete successfully");
    }
    let Some(export) = parquet_export else {
        return disabled("post-generation Parquet export did not run");
    };
    if !export.enabled {
        return disabled(
            export
                .skipped_reason
                .as_deref()
                .unwrap_or("post-generation Parquet export is disabled"),
        );
    }
    if let Some(reason) = export.skipped_reason.as_deref() {
        return disabled(reason);
    }
    let Some(output_dir) = export.output_dir.as_ref() else {
        return disabled("post-generation Parquet export did not report output_dir");
    };
    if export.exported_dbnums.is_empty() {
        return disabled("post-generation Parquet export reported no exported dbnums");
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
        let package = aios_database::version_management::release_package::load_model_package(
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
        aios_database::version_management::release_package::validate_release_id_for_path(
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
                "incremental_sesno_does_not_write_ducklake_release_catalog": true,
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

#[cfg(not(feature = "gui"))]
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
    aios_database::version_management::hashing::sha256_file(path)
        .map_err(|err| anyhow::anyhow!("hash handoff manifest failed: {}: {}", path.display(), err))
}

#[cfg(not(feature = "gui"))]
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

#[cfg(not(feature = "gui"))]
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

#[cfg(not(feature = "gui"))]
fn print_incremental_sesno_summary(result: &IncrementalSesnoRunResult) {
    println!(
        "✅ incremental-sesno 完成: files={} sessions={} elements={} total_changes={}",
        result.summary["file_count"],
        result.summary["session_count"],
        result.summary["element_count"],
        result.summary["category_counts"]["total"]
    );
    println!(
        "   prim={} loop_owner={} bran_hanger={} basic_cata={} delete={}",
        result.summary["category_counts"]["prim"],
        result.summary["category_counts"]["loop_owner"],
        result.summary["category_counts"]["bran_hanger"],
        result.summary["category_counts"]["basic_cata"],
        result.summary["category_counts"]["delete"]
    );
    if let Some(success) = result.generation_success {
        println!("   generate_model_success={}", success);
    }
    if let Some(tree_index) = result.summary.get("tree_index")
        && !tree_index.is_null()
    {
        let ready = tree_index
            .get("ready")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let mode = tree_index
            .get("mode")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");
        let missing = tree_index
            .get("missing_dbnums")
            .map(|value| value.to_string())
            .unwrap_or_else(|| "[]".to_string());
        println!(
            "   tree_index: ready={} mode={} missing_dbnums={}",
            ready, mode, missing
        );
    }
    if let Some(export) = &result.parquet_export {
        if export.enabled {
            println!(
                "   parquet_export: dbnums={:?} skipped={:?}",
                export.exported_dbnums, export.skipped_reason
            );
        }
    }
    if let Some(handoff) = result.summary.get("publication_handoff") {
        if handoff
            .get("enabled")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            println!(
                "   publication_handoff: candidates={} manifest={}",
                handoff
                    .get("candidate_count")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0),
                handoff
                    .get("manifest_path")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
            );
        }
    }
    if result
        .summary
        .get("data_persist_enabled")
        .and_then(|value| value.as_bool())
        .unwrap_or(true)
    {
        println!(
            "   data_persist: sessions={} pe={} att={} uda={} deletes={} dbnum_info={}",
            result.persist_stats.session_count,
            result.persist_stats.pe_rows,
            result.persist_stats.att_rows,
            result.persist_stats.uda_rows,
            result.persist_stats.delete_count,
            result.persist_stats.dbnum_info_updates
        );
    } else {
        let reason = result
            .summary
            .get("data_persist_skipped_reason")
            .and_then(|value| value.as_str())
            .unwrap_or("--no-persist requested");
        println!("   data_persist: skipped ({})", reason);
    }
}

#[cfg(not(feature = "gui"))]

/// 模型生成完成后同步缓存数据到 SurrealDB 的辅助函数
///
/// `debug_model_refnos`: 当指定时，仅同步这些 refno 的子孙节点数据（避免同步整个 cache）。
#[cfg(not(feature = "gui"))]
async fn sync_cache_to_db_if_enabled(
    sync_enabled: bool,
    db_option_ext: &aios_database::options::DbOptionExt,
    debug_model_refnos: Option<&[String]>,
) -> anyhow::Result<()> {
    if !sync_enabled {
        return Ok(());
    }

    // 确保数据库已连接
    init_surreal().await?;

    // 如果有 debug_model_refnos，收集子孙节点构建 refno_filter
    let refno_filter = if let Some(refno_strs) = debug_model_refnos {
        if !refno_strs.is_empty() {
            use aios_core::pdms_types::RefnoEnum;
            use std::str::FromStr;

            let roots: Vec<RefnoEnum> = refno_strs
                .iter()
                .filter_map(|s| {
                    let r = s.replace('_', "/");
                    RefnoEnum::from_str(&r).ok()
                })
                .collect();

            if !roots.is_empty() {
                println!(
                    "\n🗄️  --sync-to-db: 仅同步 debug-model 指定节点的子孙数据: {:?}",
                    refno_strs
                );
                // 查询子孙节点（包含自身）
                let descendants =
                    aios_core::collect_descendant_filter_ids(&roots, &[], None).await?;
                let mut filter: std::collections::HashSet<RefnoEnum> =
                    descendants.into_iter().collect();
                // 包含根节点自身
                filter.extend(roots.iter().copied());
                println!("   收集到 {} 个子孙 refno（含根节点）", filter.len());
                Some(filter)
            } else {
                println!("\n🗄️  --sync-to-db: 模型生成完成，开始同步缓存数据到 SurrealDB...");
                None
            }
        } else {
            println!("\n🗄️  --sync-to-db: 模型生成完成，开始同步缓存数据到 SurrealDB...");
            None
        }
    } else {
        println!("\n🗄️  --sync-to-db: 模型生成完成，开始同步缓存数据到 SurrealDB...");
        None
    };

    let cache_dir = db_option_ext.get_model_cache_dir();
    let flushed = aios_database::fast_model::cache_flush::flush_latest_instance_cache_to_surreal(
        &cache_dir,
        None, // 同步所有 dbnums（refno_filter 会在 merge 后精确过滤）
        true, // replace_exist = true，覆盖已有数据
        true, // verbose
        refno_filter.as_ref(),
    )
    .await?;

    println!(
        "✅ 数据同步完成：cache_dir={} flushed_dbnums={}",
        cache_dir.display(),
        flushed
    );

    Ok(())
}

/// debug-model 流程的后置步骤：sync-to-db + export-dbnum-instances（parquet/json）
///
/// 将 sync + 导出合并为一个调用，避免 debug-model 分支中重复编写。
#[cfg(not(feature = "gui"))]
async fn post_export_steps(
    matches: &clap::ArgMatches,
    db_option_ext: &aios_database::options::DbOptionExt,
    debug_model_refnos: Option<&[String]>,
    verbose: bool,
) -> anyhow::Result<()> {
    // 1. sync-to-db
    sync_cache_to_db_if_enabled(
        matches.get_flag("sync-to-db"),
        db_option_ext,
        debug_model_refnos,
    )
    .await?;

    // 2. export-parquet / export-dbnum-instances-json
    let want_parquet =
        matches.get_flag("export-parquet") || matches.get_flag("export-dbnum-instances");
    let want_json = matches.get_flag("export-dbnum-instances-json");

    if !want_parquet && !want_json {
        return Ok(());
    }

    // 从 debug-model refno 推导 dbnum + root_refno
    use aios_core::pdms_types::RefnoEnum;
    use std::str::FromStr;

    let first_refno_str = debug_model_refnos
        .and_then(|v| v.first())
        .map(|s| s.replace('_', "/"));
    let root_refno: Option<RefnoEnum> = first_refno_str
        .as_deref()
        .and_then(|s| RefnoEnum::from_str(s).ok());

    // 自动推导 dbnum：优先 CLI --dbnum，否则从 refno 推导
    let dbnum = matches.get_one::<u32>("dbnum").copied().or_else(|| {
        root_refno.and_then(|r| aios_database::data_interface::db_meta().get_dbnum_by_refno(r))
    });

    let Some(dbnum) = dbnum else {
        eprintln!("⚠️  无法推导 dbnum，跳过 export-dbnum-instances");
        return Ok(());
    };

    let output_override = matches
        .get_one::<String>("output")
        .map(std::path::PathBuf::from);

    // 确保数据库已连接
    init_surreal().await?;

    #[cfg(feature = "parquet-export")]
    if want_parquet {
        println!(
            "\n📦 后置步骤：从 SurrealDB 导出 dbnum={} 实例数据为 Parquet",
            dbnum
        );
        crate::cli_modes::export_dbnum_instances_parquet_mode(
            dbnum,
            verbose,
            output_override.clone(),
            db_option_ext,
            root_refno,
        )
        .await?;
    }

    if want_json {
        let from_cache = matches.get_flag("from-cache");
        let detailed = matches.get_flag("detailed");
        println!("\n📦 后置步骤：导出 dbnum={} 实例数据为 JSON", dbnum);
        crate::cli_modes::export_dbnum_instances_json_mode(
            dbnum,
            verbose,
            output_override,
            db_option_ext,
            true, // autorun
            root_refno,
            from_cache,
            detailed,
        )
        .await?;
    }

    Ok(())
}

#[cfg(all(not(feature = "gui"), feature = "grpc"))]
use crate::cli_modes::start_grpc_server_mode;
#[cfg(not(feature = "gui"))]
use crate::cli_modes::{
    ExportConfig, export_glb_mode, export_gltf_mode, export_model_mode, export_obj_mode,
    get_output_filename_for_refno, rebuild_room_spatial_index_mode,
};
#[cfg(not(feature = "gui"))]
use aios_core::geometry::csg::clear_ploop_debug_cache;
#[cfg(not(feature = "gui"))]
use aios_core::{DBType, init_surreal, query_mdb_db_nums};
#[cfg(feature = "gui")]
use aios_database::gui;
#[cfg(not(feature = "gui"))]
use aios_database::options::{
    MeshFormat, ModelWriterMode, get_db_option_ext_from_path, parse_transform_compare_backends,
    parse_transform_read_backend, parse_transform_write_backend,
};
#[cfg(not(feature = "gui"))]
use aios_database::run_app;
#[cfg(not(feature = "gui"))]
use clap::{Arg, Command};
#[cfg(not(feature = "gui"))]
use std::process::{Command as StdCommand, Stdio};

#[cfg(feature = "gui")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    gui::run_gui();
    Ok(())
}

#[cfg(not(feature = "gui"))]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 默认不重定向 stdout/stderr，保证终端有输出，避免控制台刷屏导致“看似死循环”。
    // 默认不重定向；-v/--verbose 始终保留控制台输出；AIOS_REDIRECT_STDIO=1 可启用重定向到 logs/。
    maybe_redirect_stdio_to_log_file();

    let matches = aios_database::cli_args::add_init_project_subcommand(
        aios_database::cli_args::add_export_instance_args(Command::new("aios-database")
        .version("0.1.3")
        .about("AIOS Database Processing Tool")
        .arg(
            Arg::new("config")
                .long("config")
                .short('c')
                .help("Path to the configuration file (Without extension)")
                .value_name("CONFIG_PATH")
                .default_value(if cfg!(target_family = "unix") {
                    "db_options/DbOption-mac"
                } else {
                    "db_options/DbOption"
                }),
        )
        .arg(
            Arg::new("gen-lod")
                .long("gen-lod")
                .help("Override mesh generation LOD level for this run (L0-L4). Defaults to db_options/DbOption.toml")
                .value_name("LOD")
                .value_parser(["L0", "L1", "L2", "L3", "L4"]),
        )
        .arg(
            Arg::new("debug-model")
                .long("debug-model")
                .help("Enable debug model output with verbose debug logging. Can optionally specify reference numbers (comma-separated)")
                .value_name("REFNOS")
                .value_delimiter(',')
                .num_args(0..)
                .conflicts_with("root-model"),
        )
        .arg(
            Arg::new("root-model")
                .long("root-model")
                .help("Incremental model generation for specified refnos WITHOUT debug logging (quieter alternative to --debug-model)")
                .value_name("REFNOS")
                .value_delimiter(',')
                .num_args(0..)
                .conflicts_with("debug-model"),
        )
        .arg(
            Arg::new("debug-model-errors-only")
                .long("debug-model-errors-only")
                .help("Only log errors during model generation (reduces log verbosity)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("log-model-error")
                .long("log-model-error")
                .help("Record model generation errors for statistical analysis (automatically enables debug-model and errors-only mode)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("gen-indextree")
                .long("gen-indextree")
                .help("生成 indextree 文件。可选指定 dbnum，不指定则生成所有 DESI 类型")
                .value_name("DBNUM")
                .num_args(0..=1),
        )
        .arg(
            Arg::new("gen-all-desi-indextree")
                .long("gen-all-desi-indextree")
                .help("强制生成所有 DESI 类型的 indextree 文件（绕过配置文件中的 manual_db_nums 限制）")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("capture")
                .long("capture")
                .help("After model generation, export OBJ and capture screenshots (optionally provide output directory)")
                .value_name("DIR")
                .num_args(0..=1)
                .default_missing_value("output/screenshots"),
        )
        .arg(
            Arg::new("capture-width")
                .long("capture-width")
                .help("Screenshot width in pixels (default 800)")
                .value_name("PX")
                .value_parser(clap::value_parser!(u32))
                .requires("capture"),
        )
        .arg(
            Arg::new("capture-height")
                .long("capture-height")
                .help("Screenshot height in pixels (default 600)")
                .value_name("PX")
                .value_parser(clap::value_parser!(u32))
                .requires("capture"),
        )
        .arg(
            Arg::new("capture-views")
                .long("capture-views")
                .help("Extra camera views to render (>=1). When >1, saves `{basename}_viewXX.png` alongside `{basename}.png`")
                .value_name("N")
                .value_parser(clap::value_parser!(u8))
                .requires("capture"),
        )
        .arg(
            Arg::new("capture-include-descendants")
                .long("capture-include-descendants")
                .help("Include descendants when exporting OBJ for capture (default: true). You can pass `--capture-include-descendants=false` to disable.")
                // 兼容两种写法：
                // - 旧：`--capture-include-descendants`（无值）=> true
                // - 新：`--capture-include-descendants=true/false`
                .num_args(0..=1)
                .default_missing_value("true")
                .default_value("true")
                .value_parser(clap::value_parser!(bool))
                .requires("capture"),
        )
        .arg(
            Arg::new("capture-baseline")
                .long("capture-baseline")
                .help("Compare captured screenshots with baseline directory (expects same filename .png)")
                .value_name("DIR")
                .requires("capture"),
        )
        .arg(
            Arg::new("capture-diff")
                .long("capture-diff")
                .help("Output directory for diff images (default: <capture-dir>/diff)")
                .value_name("DIR")
                .requires("capture"),
        )
        .arg(
            Arg::new("export-obj")
                .long("export-obj")
                .help("Export OBJ model when using --debug-model")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-svg")
                .long("export-svg")
                .help("Export profile SVG when using --debug-model")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("regen-model")
                .long("regen-model")
                .help("Regenerate model data (forces replace_mesh mode). With export flags: regenerate first then export; without export flags: regenerate only")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("diagnose-surreal")
                .long("diagnose-surreal")
                .help("Print SurrealDB startup diagnostics and copyable test commands without opening embedded RocksDB")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("defer-db-write")
                .long("defer-db-write")
                .help("Deprecated and ignored: DB writes always stay online during model generation")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("flush-cache-to-db")
                .long("flush-cache-to-db")
                .help("Flush model instance_cache to SurrealDB (backup). Requires SurrealDB config in DbOption")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("flush-cache-dbnums")
                .long("flush-cache-dbnums")
                .help("Only flush specified dbnums (comma-separated, e.g. 1112,1113). Default: all dbnums in cache")
                .value_name("DBNUMS")
                .value_delimiter(',')
                .value_parser(clap::value_parser!(u32))
                .num_args(1..)
                .requires("flush-cache-to-db"),
        )
        .arg(
            Arg::new("flush-cache-replace")
                .long("flush-cache-replace")
                .help("When flushing cache to SurrealDB, delete/replace existing instance records (危险：会覆盖 DB 侧数据)")
                .action(clap::ArgAction::SetTrue)
                .requires("flush-cache-to-db"),
        )
        .arg(
            Arg::new("sync-to-db")
                .long("sync-to-db")
                .help("After model generation, sync cache data to SurrealDB (模型生成完成后同步数据到数据库)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-glb")
                .long("export-glb")
                .help("Export GLB model when using --debug-model")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-gltf")
                .long("export-gltf")
                .help("Export glTF model when using --debug-model")
                .action(clap::ArgAction::SetTrue),
        )
        // 新增独立导出命令 - 不启用调试模式
        .arg(
            Arg::new("export-obj-refnos")
                .long("export-obj-refnos")
                .help("Export OBJ model for specified reference numbers (comma-separated, no debug mode)")
                .value_name("REFNOS")
                .value_delimiter(',')
                .num_args(1..),
        )
        .arg(
            Arg::new("export-glb-refnos")
                .long("export-glb-refnos")
                .help("Export GLB model for specified reference numbers (comma-separated, no debug mode)")
                .value_name("REFNOS")
                .value_delimiter(',')
                .num_args(1..),
        )
        .arg(
            Arg::new("export-gltf-refnos")
                .long("export-gltf-refnos")
                .help("Export glTF model for specified reference numbers (comma-separated, no debug mode)")
                .value_name("REFNOS")
                .value_delimiter(',')
                .num_args(1..),
        )
        .arg(
            Arg::new("export-obj-output")
                .long("export-obj-output")
                .help("Output path for exported OBJ file (optional, defaults to PE name)")
                .value_name("OUTPUT_PATH"),
        )
        .arg(
            Arg::new("use-surrealdb")
                .long("use-surrealdb")
                .help("Force enable SurrealDB instances source / model-data writes for export/debug flows (default follows config)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("include-negative")
                .long("include-negative")
                .help("Include negative entities (Neg type geometries) in export")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-filter-nouns")
                .long("export-filter-nouns")
                .help("Filter by noun types (comma-separated, e.g., EQUI,PIPE,VALV)")
                .value_name("NOUNS")
                .value_delimiter(',')
                .num_args(0..),
        )
        .arg(
            Arg::new("export-include-descendants")
                .long("export-include-descendants")
                .help("Include all descendants of specified refnos")
                .value_name("BOOL")
                .default_value("true")
                .value_parser(clap::value_parser!(bool)),
        )
        .arg(
            Arg::new("export-format")
                .long("export-format")
                .help("Export format (obj, glb, gltf)")
                .value_name("FORMAT")
                .default_value("obj"),
        )
        .arg(
            Arg::new("dbnum")
                .long("dbnum")
                .help("Database number for export / model generation. When running gen_model, overrides manual_db_nums")
                .value_name("DBNO")
                .value_parser(clap::value_parser!(u32)),
        )
        .arg(
            Arg::new("root-refno")
                .long("root-refno")
                .help("Root refno for scoped export (e.g. 24381_145018 or 24381/145018)")
                .value_name("REFNO"),
        )
        .arg(
            Arg::new("gen-nouns")
                .long("gen-nouns")
                .help("Only generate specified noun types (comma-separated, e.g. BRAN,PANE). Overrides index_tree_enabled_target_types in DbOption")
                .value_name("NOUNS")
                .value_delimiter(',')
                .num_args(1..),
        )
        .arg(
            Arg::new("gen-limit-per-noun")
                .long("gen-limit-per-noun")
                .help("Limit max instances per noun type during generation (e.g. 50). 0 means unlimited.")
                .value_name("LIMIT")
                .value_parser(clap::value_parser!(usize)),
        )
        .arg(
            Arg::new("gen-dry-run")
                .long("gen-dry-run")
                .help("Dry run: only collect refnos and log, skip geometry generation and DB writes. Use to verify refnos are processed (e.g. grep 24381_145019)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("model-writer")
                .long("model-writer")
                .help("Model writer backend: surreal writes to SurrealDB; drain-only consumes generated batches for throughput testing without persistence; ducklake writes 9 trait-covered Phase 1 raw tables to ducklake-canonical schema via Rust duckdb crate (requires feature `model-writer-ducklake`; see goals/ducklake-model-writer/)")
                .value_name("WRITER")
                .value_parser(["surreal", "drain-only", "ducklake", "duck-lake"]),
        )
        .arg(
            Arg::new("export-parquet-after-gen")
                .long("export-parquet-after-gen")
                .help("After model generation, automatically export Parquet for each dbnum in manual_db_nums (instances/tubings/transforms/aabb)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("verbose")
                .long("verbose")
                .short('v')
                .help("Enable verbose output")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-source-unit")
                .long("export-source-unit")
                .help("Source unit for export (mm, cm, m, in, ft, yd)")
                .value_name("UNIT")
                .default_value("mm"),
        )
        .arg(
            Arg::new("export-target-unit")
                .long("export-target-unit")
                .help("Target unit for export (mm, cm, dm, m, in, ft, yd)")
                .value_name("UNIT")
                .default_value("mm"),
        )
        .arg(
            Arg::new("basic-materials")
                .long("basic-materials")
                .help("Use basic (unlit) materials instead of PBR when exporting GLB/GLTF")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("split-site")
                .long("split-site")
                .help("Split each SITE into separate files (default: merge all SITEs in the same dbnum)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("output")
                .long("output")
                .help("Override the export output directory (defaults vary by subcommand)")
                .value_name("DIR"),
        )
        .arg(
            Arg::new("export-refnos")
                .long("export-refnos")
                .help("Export only specified refnos (comma-separated, e.g., '24381_46959,24381_46960')")
                .value_name("REFNOS"),
        )
        .arg(
            Arg::new("export-all-relates")
                .long("export-all-relates")
                .help("Export all inst_relate entities in Prepack LOD format (按 zone 分组, 默认仅 L1)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-all-parquet")
                .long("export-all-parquet")
                .help("Export all inst_relate entities in Prepack LOD format with additional Parquet manifests (instances.parquet + geometry_manifest.parquet)")
                .action(clap::ArgAction::SetTrue),
        ))
        .arg(
            Arg::new("import-spatial-index")
                .long("import-spatial-index")
                .help("Import instances.json to SQLite spatial index")
                .value_name("JSON_PATH"),
        )
        .arg(
            Arg::new("import-spatial-index-parquet")
                .long("import-spatial-index-parquet")
                .help("Import dbnum Parquet directory (aabb/instances/tubings.parquet) to SQLite spatial index")
                .value_name("PARQUET_DIR"),
        )
        .arg(
            Arg::new("import-rvm")
                .long("import-rvm")
                .help("Import an RVM file into SQLite relation tables")
                .value_name("RVM_PATH"),
        )
        .arg(
            Arg::new("import-att")
                .long("import-att")
                .help("Optional ATT/TXT files paired with --import-rvm (comma-separated or repeated)")
                .value_name("ATT_PATHS")
                .value_delimiter(',')
                .num_args(1..),
        )
        .arg(
            Arg::new("no-resolve-identity")
                .long("no-resolve-identity")
                .help("spec 009: disable resolving RVM group names to real PDMS refnos during --import-rvm")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("compare-rvm")
                .long("compare-rvm")
                .help("spec 009: compare imported RVM baseline against gen-model Parquet export. Requires --dbnum, --root-refno and --parquet-dir")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("parquet-dir")
                .long("parquet-dir")
                .help("Parquet export directory for --compare-rvm (e.g. .../output/<project>/parquet/<dbnum>)")
                .value_name("PARQUET_DIR"),
        )
        .arg(
            Arg::new("tol-aabb-mm")
                .long("tol-aabb-mm")
                .help("AABB per-component tolerance in mm for --compare-rvm (default 1.0)")
                .value_name("MM"),
        )
        .arg(
            Arg::new("spatial-index-output")
                .long("spatial-index-output")
                .help("Output path for SQLite spatial index (default: output/spatial_index.sqlite)")
                .value_name("SQLITE_PATH"),
        )
        .arg(
            Arg::new("relation-store-output")
                .long("relation-store-output")
                .help("Root directory for SQLite relation store output (default: output/model_relations)")
                .value_name("DIR"),
        )
        .arg(
            Arg::new("export-all-lods")
                .long("export-all-lods")
                .help("Export all LOD levels (L1, L2, L3). Without this, only L1 is exported")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("owner-types")
                .long("owner-types")
                .help("Filter by owner_type (comma-separated, e.g., 'BRAN,HANG')")
                .value_name("TYPES"),
        )
        .arg(
            Arg::new("name-config")
                .long("name-config")
                .help("Excel file for name mapping (三维模型节点 -> PID对象)")
                .value_name("EXCEL_PATH"),
        )
        .arg(
            Arg::new("mesh-type")
                .long("mesh-type")
                .alias("mesh_type")
                .help("Mesh format to generate (pdmsmesh, glb, obj). Multiple values allowed.")
                .value_name("TYPE")
                .value_delimiter(',')
                .num_args(1..),
        )
        .subcommand(
            Command::new("spatial")
                .about("SQLite 空间范围查询与回归验证")
                .subcommand(
                    Command::new("query-refno")
                        .about("以 refno 为中心做空间范围查询，并可校验 expect-refnos / verify-json")
                        .arg(
                            Arg::new("refno")
                                .help("查询中心 refno（如 24381/145019）")
                                .required(true),
                        )
                        .arg(
                            Arg::new("distance-mm")
                                .long("distance-mm")
                                .help("查询距离，单位毫米；1m 请传 1000")
                                .default_value("1000"),
                        )
                        .arg(
                            Arg::new("include-self")
                                .long("include-self")
                                .help("结果中包含查询 refno 本身")
                                .action(clap::ArgAction::SetTrue),
                        )
                        .arg(
                            Arg::new("build-spatial")
                                .long("build-spatial")
                                .help("查询前先刷新 output/spatial_index.sqlite")
                                .action(clap::ArgAction::SetTrue),
                        )
                        .arg(
                            Arg::new("expect-refnos")
                                .long("expect-refnos")
                                .help("期望命中的 refno（逗号分隔）")
                                .value_delimiter(',')
                                .num_args(1..),
                        )
                        .arg(
                            Arg::new("verify-json")
                                .long("verify-json")
                                .help("将当前查询结果与给定 JSON 快照做回归校验"),
                        )
                        .arg(
                            Arg::new("write-verify-json")
                                .long("write-verify-json")
                                .help("将当前查询结果写入 JSON 快照文件"),
                        ),
                ),
        )
        // ========== 房间计算子命令 ==========
        .subcommand(
            Command::new("room")
                .about("房间计算相关命令")
                .subcommand(
                    Command::new("compute")
                        .about("执行房间关系计算（构件空间归属判定）")
                        .arg(Arg::new("keywords").long("keywords").short('k')
                            .help("房间名称关键词过滤（逗号分隔）")
                            .value_delimiter(',')
                            .num_args(1..))
                        .arg(Arg::new("db-nums").long("db-nums")
                            .help("限定数据库编号（逗号分隔）")
                            .value_delimiter(',')
                            .num_args(1..))
                        .arg(Arg::new("refno-root").long("refno-root")
                            .help("限定 refno 子树根（如 21491_10000）"))
                        .arg(Arg::new("gen-panels-mesh").long("gen-panels-mesh")
                            .visible_alias("generate-models")
                            .help("显式允许为缺失面板预生成几何模型；默认跳过模型生成，仅计算空间关系")
                            .action(clap::ArgAction::SetTrue))
                        .arg(Arg::new("report-json").long("report-json")
                            .help("将房间计算阶段耗时与统计写入 JSON 报告")
                            .value_name("FILE")),
                )
                .subcommand(
                    Command::new("compute-panel")
                        .about("指定单个面板 refno 执行房间计算")
                        .arg(Arg::new("panel-refno")
                            .help("面板参考号（如 24381/35798）")
                            .required(true))
                        .arg(Arg::new("generate-models").long("generate-models")
                            .visible_alias("gen-panels-mesh")
                            .help("显式允许为 panel 与 expect 对应目标补生成模型；默认跳过模型生成，仅做关系计算")
                            .action(clap::ArgAction::SetTrue))
                        .arg(Arg::new("expect-refnos").long("expect-refnos")
                            .help("期望命中的构件 refno（逗号分隔），用于验证计算结果")
                            .value_delimiter(',')
                            .num_args(1..))
                        .arg(Arg::new("rebuild-spatial-index").long("rebuild-spatial-index")
                            .help("显式重建本次 panel 计算使用的 SQLite 空间索引；默认直接复用现有索引。若局部刷新结果为空，会自动回退为全量重建")
                            .action(clap::ArgAction::SetTrue))
                        .arg(Arg::new("report-json").long("report-json")
                            .help("将单面板计算阶段耗时与统计写入 JSON 报告")
                            .value_name("FILE")),
                )
                .subcommand(
                    Command::new("rebuild-spatial-index")
                        .about("从 inst_relate_aabb 正式重建全量 SQLite 空间索引"),
                )
                .subcommand(
                    Command::new("clean")
                        .about("清理已有的房间关系数据（room_relate + room_panel_relate）"),
                )
                .subcommand(
                    Command::new("verify-json")
                        .about("读取 JSON fixture 校验已持久化的房间计算结果（默认只读）")
                        .arg(
                            Arg::new("input")
                                .long("input")
                                .short('i')
                                .help("验证 fixture JSON 路径（推荐：verification/room_compute_validation.json）")
                                .required(true)
                                .value_name("FILE"),
                        ),
                )
                .subcommand(
                    Command::new("export")
                        .about("导出房间计算结果为 JSON")
                        .arg(Arg::new("output").long("output").short('o')
                            .help("输出目录")),
                ),
        )
        .subcommand(
            Command::new("incremental-sesno")
                .about("使用 pdms-io 指定 sesno 范围收集 E3D/PDMS 增量，并可选择触发增量模型生成")
                .arg(
                    Arg::new("dbnum")
                        .long("dbnum")
                        .help("通过 db_index.sqlite 定位的 dbnum，可逗号分隔/重复传入")
                        .value_name("DBNUM")
                        .value_delimiter(',')
                        .value_parser(clap::value_parser!(u32))
                        .num_args(1..),
                )
                .arg(
                    Arg::new("file")
                        .long("file")
                        .help("直接指定单个 PDMS db 文件路径（不依赖 db_index.sqlite）")
                        .value_name("DB_FILE"),
                )
                .arg(
                    Arg::new("from-sesno")
                        .long("from-sesno")
                        .help("当前已解析/缓存到的 sesno；实际增量从 from+1 开始")
                        .value_name("SESNO")
                        .value_parser(clap::value_parser!(u32))
                        .required(true),
                )
                .arg(
                    Arg::new("to-sesno")
                        .long("to-sesno")
                        .help("目标 sesno；省略则使用文件最新 sesno")
                        .value_name("SESNO")
                        .value_parser(clap::value_parser!(u32)),
                )
                .arg(
                    Arg::new("rescan-index")
                        .long("rescan-index")
                        .help("按指纹增量刷新 db_index.sqlite 后再按 dbnum 定位文件")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("no-persist")
                        .long("no-persist")
                        .help("只收集和分类增量；不刷新 db_meta、不连接 SurrealDB、不写 PE/ATT。不能与 --generate-model 同用")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("generate-model")
                        .long("generate-model")
                        .help("收集增量后调用 gen_all_geos_data(..., Some(update_log), None)")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("require-tree-index")
                        .long("require-tree-index")
                        .help("增量模型生成前要求 scene_tree/<dbnum>.tree 已存在；缺失时快速失败，不进入模型生成")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("source-observation-manifest")
                        .long("source-observation-manifest")
                        .help("model-version observe-source 生成的 source observation manifest；启用后会在增量任务前后校验源 DB hash")
                        .value_name("FILE"),
                )
                .arg(
                    Arg::new("source-observation-manifest-hash")
                        .long("source-observation-manifest-hash")
                        .help("可选的 source observation manifest SHA-256，防止 manifest 路径被替换")
                        .value_name("SHA256"),
                )
                .arg(
                    Arg::new("publication-handoff-dir")
                        .long("publication-handoff-dir")
                        .help("增量生成成功后写入 release publication handoff manifest 的目录；不自动注册 release")
                        .value_name("DIR"),
                )
                .arg(
                    Arg::new("release-id-prefix")
                        .long("release-id-prefix")
                        .help("publication handoff 中建议 release_id 的前缀")
                        .value_name("PREFIX")
                        .default_value("incremental-sesno"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("输出 pretty JSON 摘要")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("watch-incremental")
                .about("轮询监控 E3D/PDMS db 目录，发现 sesno 增长后执行增量解析保存，可选择增量生成模型")
                .arg(
                    Arg::new("dbnum")
                        .long("dbnum")
                        .help("仅监控指定 dbnum；可逗号分隔/重复传入。省略则监控 db_index 中全部 db 文件")
                        .value_name("DBNUM")
                        .value_delimiter(',')
                        .value_parser(clap::value_parser!(u32))
                        .num_args(1..),
                )
                .arg(
                    Arg::new("interval-secs")
                        .long("interval-secs")
                        .help("轮询间隔秒数")
                        .value_name("SECS")
                        .value_parser(clap::value_parser!(u64))
                        .default_value("30"),
                )
                .arg(
                    Arg::new("once")
                        .long("once")
                        .help("只执行一轮扫描，便于验证 watcher 配置")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("force-initial-scan")
                        .long("force-initial-scan")
                        .help("启动时强制全量重建 db_index，并以该结果作为 watcher 基线")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("generate-model")
                        .long("generate-model")
                        .help("发现增量后同步触发模型增量生成")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("require-tree-index")
                        .long("require-tree-index")
                        .help("watcher 触发增量模型生成前要求 scene_tree/<dbnum>.tree 已存在；缺失时快速失败，不进入模型生成")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("observation-quiescence-window-ms")
                        .long("observation-quiescence-window-ms")
                        .help("发现增量后生成 source observation manifest 时，两次源文件 hash/size 检查之间的静默窗口")
                        .value_name("MS")
                        .value_parser(clap::value_parser!(u64))
                        .default_value("1000"),
                )
                .arg(
                    Arg::new("source-observation-dir")
                        .long("source-observation-dir")
                        .help("watch-incremental 自动写入 source observation manifest 的目录")
                        .value_name("DIR"),
                )
                .arg(
                    Arg::new("publication-handoff-dir")
                        .long("publication-handoff-dir")
                        .help("增量生成成功后写入 release publication handoff manifest 的目录；不自动注册 release")
                        .value_name("DIR"),
                )
                .arg(
                    Arg::new("release-id-prefix")
                        .long("release-id-prefix")
                        .help("publication handoff 中建议 release_id 的前缀")
                        .value_name("PREFIX")
                        .default_value("incremental-sesno"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("每轮输出 pretty JSON 摘要")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("scan-db-index")
                .about(
                    "index-only 预扫描所有 db 文件 ref0/dbnum（pdms-io INDEX 直扫）写入 SQLite，并记录设计库精确依赖边（关联库精确解析基础）",
                )
                .arg(
                    Arg::new("no-scan")
                        .long("no-scan")
                        .help("增量模式：仅重扫指纹（mtime/size）变化的库（默认全量重扫）")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("gen-cata-closure")
                .about(
                    "前置 CATA 闭包 pass：扫描工程根下全部 DESI 库 → refno 级引用闭包 → 写 <output>/<项目>/scene_tree/cata_closure.json（解析时配合 AIOS_CATA_CLOSURE_MODE=manifest 启用 CATA 部分解析）",
                )
                .arg(
                    Arg::new("rescan-index")
                        .long("rescan-index")
                        .help("先按指纹（mtime/size）增量刷新 db_index.sqlite（缺失时总会自动全量预扫描）")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("seed-refnos")
                        .long("seed-refnos")
                        .help("按需模式：仅以这些设计元素（如 BRAN refno，形如 24381_145018，逗号分隔）的子树出向引用为种子做闭包；省略则扫描工程根下全部 DESI 库")
                        .value_name("REFNOS")
                        .value_delimiter(',')
                        .num_args(1..),
                )
                .arg(
                    Arg::new("out")
                        .long("out")
                        .help("覆盖 manifest 输出路径（默认 <output>/<项目>/scene_tree/cata_closure.json）")
                        .value_name("PATH"),
                ),
        )
        .subcommand(
            Command::new("verify-cata-closure")
                .about(
                    "T008 离线校验：当前 -c 配置=按需站点，对比基准站点（整库解析）的生成结果（成员/几何指纹/TUBI/裁剪率），写 <output>/<项目>/cata_closure_verify.json，失败以非零码退出",
                )
                .arg(
                    Arg::new("refnos")
                        .long("refnos")
                        .help("校验的设计根 refno（如 BRAN，逗号分隔）")
                        .value_name("REFNOS")
                        .value_delimiter(',')
                        .num_args(1..)
                        .required(true),
                )
                .arg(
                    Arg::new("baseline-endpoint")
                        .long("baseline-endpoint")
                        .help("基准站点 SurrealDB 地址（host:port，可带 ws:// 前缀）")
                        .value_name("ADDR")
                        .required(true),
                )
                .arg(Arg::new("baseline-ns").long("baseline-ns").value_name("NS").required(true))
                .arg(Arg::new("baseline-db").long("baseline-db").value_name("DB").required(true))
                .arg(Arg::new("baseline-user").long("baseline-user").value_name("USER").required(true))
                .arg(Arg::new("baseline-pass").long("baseline-pass").value_name("PASS").required(true))
                .arg(
                    Arg::new("out")
                        .long("out")
                        .help("覆盖报告输出路径（默认 <output>/<项目>/cata_closure_verify.json）")
                        .value_name("PATH"),
                ),
        )
        .subcommand(
            Command::new("model-record-id-verify")
                .about("输出模型产物版本化 array record id 样例，用于 versioned-model-record-id 重构验证")
                .arg(
                    Arg::new("refno")
                        .long("refno")
                        .help("基础 refno，例如 24381/145569 或 24381_145569")
                        .required(true)
                        .value_name("REFNO"),
                )
                .arg(
                    Arg::new("sesno")
                        .long("sesno")
                        .help("模型数据版本号；省略时按 current/latest sesno=0")
                        .value_parser(clap::value_parser!(u32))
                        .value_name("SESNO"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("以 pretty JSON 输出")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(aios_database::version_management::cli::model_version_command())
        .subcommand(
            Command::new("serve")
                .about("启动 aios-database 解析域 sidecar HTTP/WS 服务")
                .arg(
                    Arg::new("site-key")
                        .long("site-key")
                        .help("sidecar 生命周期 key，例如 site:<site_id> 或 preview:<hash>")
                        .required(true)
                        .value_name("SITE_KEY"),
                )
                .arg(
                    Arg::new("bind-host")
                        .long("bind-host")
                        .help("sidecar 监听地址，默认只绑定本机")
                        .default_value("127.0.0.1")
                        .value_name("HOST"),
                )
                .arg(
                    Arg::new("http-port")
                        .long("http-port")
                        .help("sidecar HTTP/WS 监听端口")
                        .required(true)
                        .value_parser(clap::value_parser!(u16))
                        .value_name("PORT"),
                )
                .arg(
                    Arg::new("runtime-dir")
                        .long("runtime-dir")
                        .help("sidecar runtime 目录")
                        .required(true)
                        .value_name("DIR"),
                )
                .arg(
                    Arg::new("token")
                        .long("token")
                        .help("web_server 调用 sidecar 时使用的 Bearer token；为空则不启用内部鉴权")
                        .value_name("TOKEN"),
                )
                .arg(
                    Arg::new("shutdown-after-job")
                        .long("shutdown-after-job")
                        .help("CLI job 进入 terminal 状态后优雅关闭 sidecar")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("shutdown-delay-ms")
                        .long("shutdown-delay-ms")
                        .help("CLI job 结束后延迟多少毫秒再关闭 sidecar")
                        .default_value("1000")
                        .value_parser(clap::value_parser!(u64))
                        .value_name("MS"),
                )
                .arg(
                    Arg::new("idle-timeout-secs")
                        .long("idle-timeout-secs")
                        .help("serve sidecar 空闲多少秒后自动退出（0 表示禁用），默认 1800")
                        .default_value("1800")
                        .value_parser(clap::value_parser!(u64))
                        .value_name("SECS"),
                ),
        )
        // ========== pe_transform 刷新命令 ==========
        .arg(
            Arg::new("refresh-transform")
                .long("refresh-transform")
                .help("Refresh pe_transform cache for specified dbnums (comma-separated, e.g., '1112,1113')")
                .value_name("DB_NUMS")
                .value_delimiter(',')
                .num_args(1..),
        )
        .arg(
            Arg::new("transform-write-backend")
                .long("transform-write-backend")
                .help("pe_transform 写入后端: surreal|parquet|ducklake|dual")
                .value_name("BACKEND"),
        )
        .arg(
            Arg::new("transform-read-backend")
                .long("transform-read-backend")
                .help("pe_transform 读取后端: auto|surreal|parquet|ducklake|rkyv|memory")
                .value_name("BACKEND"),
        )
        .arg(
            Arg::new("transform-compare-backends")
                .long("transform-compare-backends")
                .help("pe_transform 对比读取后端，逗号分隔，例如 surreal,parquet")
                .value_name("BACKENDS"),
        )
        .arg(
            Arg::new("transform-parquet-dir")
                .long("transform-parquet-dir")
                .help("pe_transform Parquet 输出/读取目录")
                .value_name("DIR"),
        )
        .arg(
            Arg::new("transform-ducklake-metadata")
                .long("transform-ducklake-metadata")
                .help("DuckLake metadata.ducklake 路径")
                .value_name("FILE"),
        )
        .arg(
            Arg::new("transform-ducklake-data-path")
                .long("transform-ducklake-data-path")
                .help("DuckLake data path 目录")
                .value_name("DIR"),
        )
        .arg(
            Arg::new("clear-transform-before-refresh")
                .long("clear-transform-before-refresh")
                .help("刷新前清理目标 dbnum 的历史 pe_transform，用于对比实验")
                .action(clap::ArgAction::SetTrue),
        )
        // ========== MBD JSON 预生成 ==========
        .arg(
            Arg::new("export-mbd")
                .long("export-mbd")
                .help("预生成所有 BRAN/HANG 的 MBD 标注 JSON 文件（按 --dbnum 过滤，不传则全量）")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-mbd-refno")
                .long("export-mbd-refno")
                .help("预生成指定 refno 及其子孙 BRAN/HANG 的 MBD 标注 JSON 文件")
                .value_name("REFNO"),
        )
        .arg(
            Arg::new("force")
                .long("force")
                .help("Force kill processes holding RocksDB LOCK files before connecting (强制终止占用 LOCK 的进程)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("offline")
                .long("offline")
                .help("Use embedded file mode instead of WebSocket. Auto-kills any running SurrealDB server on the configured port")
                .action(clap::ArgAction::SetTrue),
        ))
        .get_matches();

    if let Some(relation_store_root) = matches.get_one::<String>("relation-store-output") {
        unsafe {
            std::env::set_var("MODEL_RELATION_STORE_PATH", relation_store_root);
        }
    }

    // 获取配置文件路径
    let config_path = matches
        .get_one::<String>("config")
        .expect("default value ensures this exists");

    // 设置环境变量，让 rs-core 库使用正确的配置文件
    unsafe {
        std::env::set_var("DB_OPTION_FILE", config_path);
    }
    if matches.subcommand_matches("model-version").is_some() {
        unsafe {
            std::env::set_var("AIOS_QUIET_CONFIG", "1");
        }
    }

    // --offline：在 get_db_option() OnceCell 初始化前设置环境变量，覆盖 surrealdb.mode = file
    let is_offline = matches.get_flag("offline");
    if is_offline {
        println!("🔌 --offline 模式：切换为嵌入式文件连接");
        unsafe {
            std::env::set_var("SURREAL_CONN_MODE", "file");
        }
    }

    // --force：强制清理 RocksDB LOCK 文件（kill 占用进程）
    if matches.get_flag("force") {
        println!("🔧 --force 模式：将强制清理 LOCK 文件");
        unsafe {
            std::env::set_var("AIOS_FORCE_LOCK", "1");
        }
    }

    // 预先初始化 OnceCell，避免后续第一次 get_db_option() 时覆盖 active_precision
    let db_option = aios_core::get_db_option();

    // --offline 时立即关闭占用 ws 端口的 server 进程（RocksDB 排他锁）
    if is_offline {
        crate::cli_modes::kill_process_on_port(db_option.surrealdb.port);
    }

    let export_all_lods = matches.get_flag("export-all-lods");
    unsafe {
        if export_all_lods {
            std::env::set_var("EXPORT_ALL_LODS", "true");
        } else {
            std::env::remove_var("EXPORT_ALL_LODS");
        }
    }

    // 创建自定义的 DbOptionExt
    let mut db_option_ext = get_db_option_ext_from_path(config_path)?;

    if is_offline {
        db_option_ext.inner.surrealdb.mode = aios_core::options::DbConnMode::File;
        println!(
            "🔧 CLI 覆盖 surrealdb.mode -> {}（db_option_ext）",
            db_option_ext.inner.surrealdb.mode.as_str()
        );
    }

    if let Some(serve_matches) = matches.subcommand_matches("serve") {
        #[cfg(feature = "web_server")]
        {
            let site_key = serve_matches
                .get_one::<String>("site-key")
                .expect("required by clap")
                .to_string();
            let bind_host = serve_matches
                .get_one::<String>("bind-host")
                .expect("default value ensures this exists")
                .to_string();
            let http_port = serve_matches
                .get_one::<u16>("http-port")
                .copied()
                .expect("required by clap");
            let runtime_dir = serve_matches
                .get_one::<String>("runtime-dir")
                .expect("required by clap");
            let token = serve_matches
                .get_one::<String>("token")
                .cloned()
                .or_else(|| std::env::var("AIOS_SIDECAR_TOKEN").ok())
                .filter(|value| !value.trim().is_empty());
            let shutdown_after_job = serve_matches.get_flag("shutdown-after-job");
            let shutdown_delay_ms = serve_matches
                .get_one::<u64>("shutdown-delay-ms")
                .copied()
                .unwrap_or(1000);
            let idle_timeout_secs = serve_matches
                .get_one::<u64>("idle-timeout-secs")
                .copied()
                .unwrap_or(1800);
            return aios_database::parse_sidecar::run_parse_sidecar(
                aios_database::parse_sidecar::ParseSidecarOptions {
                    site_key,
                    bind_host,
                    http_port,
                    runtime_dir: PathBuf::from(runtime_dir),
                    token,
                    shutdown_after_job,
                    shutdown_delay_ms,
                    idle_timeout_secs,
                },
            )
            .await;
        }
        #[cfg(not(feature = "web_server"))]
        {
            anyhow::bail!("serve sidecar requires the web_server feature");
        }
    }

    if aios_database::version_management::cli::handle_model_version_command(
        &matches,
        &db_option_ext,
    )
    .await?
    {
        return Ok(());
    }

    if let Some(lod_str) = matches.get_one::<String>("gen-lod").map(|s| s.as_str()) {
        if let Some(lod) = parse_lod_level(lod_str) {
            println!(
                "🔧 CLI 覆盖 default_lod: {:?} -> {:?}",
                db_option_ext.inner.mesh_precision.default_lod, lod
            );
            db_option_ext.inner.mesh_precision.default_lod = lod;
        }
    }

    if let Some(mesh_types) = matches.get_many::<String>("mesh-type") {
        let mut formats = Vec::new();
        for mt in mesh_types {
            match mt.to_lowercase().as_str() {
                "pdmsmesh" | "mesh" => formats.push(MeshFormat::PdmsMesh),
                "glb" => formats.push(MeshFormat::Glb),
                "obj" => formats.push(MeshFormat::Obj),
                _ => println!("⚠️ 忽略未知的网格格式: {}", mt),
            }
        }
        if !formats.is_empty() {
            println!(
                "🔧 CLI 覆盖 mesh_formats: {:?} -> {:?}",
                db_option_ext.mesh_formats, formats
            );
            db_option_ext.mesh_formats = formats;
        }
    }

    // CLI 覆盖：模型生成的 dbnum / noun 类型（无需修改 DbOption.toml）
    // 注意：refno 的第一段是 ref0，不是 dbnum。若当前是 refno 定向生成/导出，
    // 则不要把 --dbnum 直接写进 manual_db_nums，真实 dbnum 应从 refno->dbnum 映射推导。
    let has_refno_scoped_generation = matches.contains_id("debug-model")
        || matches.contains_id("root-model")
        || matches.get_many::<String>("export-obj-refnos").is_some()
        || matches.get_many::<String>("export-glb-refnos").is_some()
        || matches.get_many::<String>("export-gltf-refnos").is_some();
    if let Some(dbnum) = matches.get_one::<u32>("dbnum").copied() {
        if has_refno_scoped_generation {
            println!(
                "ℹ️ 检测到 refno 定向生成/导出，暂不将 --dbnum={} 写入 manual_db_nums；后续将按 refno 映射真实 dbnum",
                dbnum
            );
        } else {
            db_option_ext.inner.manual_db_nums = Some(vec![dbnum]);
            println!("🔧 CLI 覆盖 manual_db_nums -> [{}]", dbnum);
        }
    }
    if let Some(nouns) = matches.get_many::<String>("gen-nouns") {
        let v: Vec<String> = nouns.map(|s| s.to_uppercase()).collect();
        if !v.is_empty() {
            println!(
                "🔧 CLI 覆盖 index_tree_enabled_target_types: {:?} -> {:?}",
                db_option_ext.index_tree_enabled_target_types, v
            );
            db_option_ext.index_tree_enabled_target_types = v;
        }
    }
    if let Some(limit) = matches.get_one::<usize>("gen-limit-per-noun").copied() {
        let override_limit = if limit == 0 { None } else { Some(limit) };
        println!(
            "🔧 CLI 覆盖 index_tree_debug_limit_per_target_type: {:?} -> {:?}",
            db_option_ext.index_tree_debug_limit_per_target_type, override_limit
        );
        db_option_ext.index_tree_debug_limit_per_target_type = override_limit;
    }
    if matches.get_flag("gen-dry-run") {
        db_option_ext.gen_model_dry_run = true;
        println!("🔧 模型生成空跑模式: 仅收集 refno 并记录日志，跳过几何生成与 DB 写入");
    }
    if let Some(writer) = matches.get_one::<String>("model-writer") {
        db_option_ext.model_writer_mode = match writer.as_str() {
            "drain-only" => ModelWriterMode::DrainOnly,
            "ducklake" | "duck-lake" => ModelWriterMode::DuckLake,
            _ => ModelWriterMode::Surreal,
        };
        println!(
            "🔧 模型写入后端: {}",
            db_option_ext.model_writer_mode.as_str()
        );
        if db_option_ext.model_writer_mode == ModelWriterMode::DrainOnly {
            println!("🔧 drain-only 压测模式: 生成几何 batch，仅消费统计，不写 SurrealDB");
        }
        if db_option_ext.model_writer_mode == ModelWriterMode::DuckLake {
            println!(
                "🔧 ducklake 模式: 写 9 张 trait 覆盖的 Phase 1 raw 表到 ducklake-canonical schema; \
                 tubi/transforms/refno_assoc 6 项保持 Known Gap (goals/ducklake-model-writer/)"
            );
        }
    }
    db_option_ext.validate_model_writer_features()?;
    if let Some(backend) = matches.get_one::<String>("transform-write-backend") {
        db_option_ext.transform_write_backend = parse_transform_write_backend(Some(backend));
        println!(
            "🔧 pe_transform 写入后端: {}",
            db_option_ext.transform_write_backend.as_str()
        );
    }
    if let Some(backend) = matches.get_one::<String>("transform-read-backend") {
        db_option_ext.transform_read_backend = parse_transform_read_backend(Some(backend));
        println!(
            "🔧 pe_transform 读取后端: {}",
            db_option_ext.transform_read_backend.as_str()
        );
    }
    if let Some(backends) = matches.get_one::<String>("transform-compare-backends") {
        db_option_ext.transform_compare_backends = parse_transform_compare_backends(Some(backends));
        let labels = db_option_ext
            .transform_compare_backends
            .iter()
            .map(|backend| backend.as_str())
            .collect::<Vec<_>>()
            .join(",");
        println!("🔧 pe_transform 对比后端: {}", labels);
    }
    if let Some(dir) = matches.get_one::<String>("transform-parquet-dir") {
        db_option_ext.transform_parquet_dir = Some(dir.clone());
        println!("🔧 pe_transform Parquet 目录: {}", dir);
    }
    if let Some(path) = matches.get_one::<String>("transform-ducklake-metadata") {
        db_option_ext.transform_ducklake_metadata = Some(path.clone());
        println!("🔧 pe_transform DuckLake metadata: {}", path);
    }
    if let Some(path) = matches.get_one::<String>("transform-ducklake-data-path") {
        db_option_ext.transform_ducklake_data_path = Some(path.clone());
        println!("🔧 pe_transform DuckLake data path: {}", path);
    }
    if matches.get_flag("clear-transform-before-refresh") {
        db_option_ext.clear_transform_before_refresh = true;
        println!("🔧 pe_transform 刷新前将清理目标 dbnum 历史数据");
    }
    db_option_ext.validate_transform_store_features()?;
    if matches.get_flag("export-parquet-after-gen") {
        db_option_ext.export_parquet_after_gen = true;
        println!("🔧 模型生成完成后将自动导出 Parquet（按 manual_db_nums）");
    }

    if matches.get_flag("diagnose-surreal") {
        return cli_modes::diagnose_surreal_startup_mode(&db_option_ext).await;
    }

    // 同步精度配置到 rs-core 全局 active_precision，保证布尔/导出等逻辑使用同一套 LOD
    aios_core::mesh_precision::set_active_precision(db_option_ext.inner.mesh_precision.clone());

    // ========== cache -> SurrealDB：一键备份落库 ==========
    if matches.get_flag("flush-cache-to-db") {
        println!("\n🗄️  flush-cache-to-db: 将 model instance_cache 写入 SurrealDB（备份）");
        init_surreal().await?;
        println!("✅ 数据库连接成功");

        let cache_dir = db_option_ext.get_model_cache_dir();
        let dbnums: Option<Vec<u32>> = matches
            .get_many::<u32>("flush-cache-dbnums")
            .map(|v| v.copied().collect());
        let replace_exist = matches.get_flag("flush-cache-replace");

        let flushed =
            aios_database::fast_model::cache_flush::flush_latest_instance_cache_to_surreal(
                &cache_dir,
                dbnums.as_deref(),
                replace_exist,
                true,
                None, // 全量备份，不按 refno 过滤
            )
            .await?;

        println!(
            "✅ flush-cache-to-db 完成：cache_dir={} flushed_dbnums={}",
            cache_dir.display(),
            flushed
        );
        return Ok(());
    }

    // ========== MBD JSON 预生成 ==========
    #[cfg(feature = "mbd-pipe")]
    if matches.get_flag("export-mbd") || matches.get_one::<String>("export-mbd-refno").is_some() {
        use aios_database::web_api::{MbdExportScope, export_mbd_json_batch, get_mbd_output_dir};

        init_surreal().await?;
        let output_dir = get_mbd_output_dir();

        let scope = if let Some(refno_str) = matches.get_one::<String>("export-mbd-refno") {
            use aios_core::pdms_types::RefnoEnum;
            use std::str::FromStr;
            let refno_str = refno_str.replace('_', "/");
            let refno = RefnoEnum::from_str(&refno_str)
                .map_err(|e| anyhow::anyhow!("无效的 refno '{}': {e}", refno_str))?;
            println!("🎯 MBD 预生成：指定 refno={} 及其子孙 BRAN/HANG", refno);
            MbdExportScope::ByRefno(refno)
        } else if let Some(dbnum) = matches.get_one::<u32>("dbnum").copied() {
            println!("🎯 MBD 预生成：dbnum={} 下所有 BRAN/HANG", dbnum);
            MbdExportScope::ByDbnum(dbnum)
        } else {
            println!("🎯 MBD 预生成：全量 BRAN/HANG");
            MbdExportScope::AllDbnums
        };

        let stats = export_mbd_json_batch(&output_dir, scope).await?;
        println!(
            "✅ MBD 预生成完成：{}/{} 成功，输出目录 {}",
            stats.success, stats.total, stats.output_dir
        );
        return Ok(());
    }

    // 调试：显示配置加载结果
    println!("🔧 配置加载完成:");
    println!("   - 配置文件路径: {}", config_path);
    println!(
        "   - index_tree_enabled_target_types: {:?}",
        db_option_ext.index_tree_enabled_target_types
    );
    println!(
        "   - index_tree_excluded_target_types: {:?}",
        db_option_ext.index_tree_excluded_target_types
    );

    println!("✅ IndexTree 默认生成管线已启用（无模式开关）");
    let config_debug_refnos: Option<Vec<String>> = db_option_ext.inner.debug_model_refnos.clone();
    let log_model_error = matches.get_flag("log-model-error");
    let debug_model_requested = matches.contains_id("debug-model") || log_model_error;
    let root_model_requested = matches.contains_id("root-model");
    let any_model_requested = debug_model_requested || root_model_requested;
    let debug_model_errors_only = matches.get_flag("debug-model-errors-only") || log_model_error;

    if log_model_error {
        println!("📊 启用模型错误记录模式（自动开启 debug-model + errors-only）");
    }

    if !any_model_requested && db_option_ext.inner.debug_model_refnos.is_some() {
        println!("ℹ️ 未开启调试/根模型模式，本次运行将忽略配置中的 debug_model_refnos");
    }
    if !any_model_requested {
        aios_core::set_debug_model_enabled(false);
        db_option_ext.inner.debug_model_refnos = None;
    }

    // 设置错误日志模式
    if debug_model_errors_only {
        aios_database::fast_model::set_debug_model_errors_only(true);
        if !log_model_error {
            println!("✅ 启用仅错误日志模式");
        }
    }

    // 获取通用参数
    let output_path = matches.get_one::<String>("export-obj-output").cloned();
    let filter_nouns: Option<Vec<String>> = matches
        .get_many::<String>("export-filter-nouns")
        .map(|nouns| nouns.map(|s| s.to_string()).collect());
    let include_descendants = matches
        .get_one::<bool>("export-include-descendants")
        .copied()
        .unwrap_or(true);
    let verbose = matches.get_flag("verbose");
    let use_basic_materials = matches.get_flag("basic-materials");

    // 获取单位转换参数
    let source_unit = matches
        .get_one::<String>("export-source-unit")
        .unwrap()
        .as_str();
    let target_unit = matches
        .get_one::<String>("export-target-unit")
        .unwrap()
        .as_str();

    // 获取 dbnum 参数（用于按 SITE 导出）
    let dbnum = matches.get_one::<u32>("dbnum").copied();

    // 获取 split-site 参数（默认合并，有此参数才拆分）
    let split_by_site = matches.get_flag("split-site");

    // 获取 include-negative 参数（是否包含负实体）
    let include_negative = matches.get_flag("include-negative");

    let capture_dir = matches.get_one::<String>("capture").cloned();
    let capture_width = matches
        .get_one::<u32>("capture-width")
        .copied()
        .unwrap_or(1200);
    let capture_height = matches
        .get_one::<u32>("capture-height")
        .copied()
        .unwrap_or(900);
    let capture_views = matches.get_one::<u8>("capture-views").copied().unwrap_or(1);
    // 截图链路默认包含子孙节点（与导出默认语义一致），否则像 BRAN/HANG 这类“几何主要在子孙节点/关联表”时
    // 会只截到一小段 TUBI，从而误判“导出管道不对”。
    let capture_include_descendants = matches
        .get_one::<bool>("capture-include-descendants")
        .copied()
        .unwrap_or(true);
    let capture_baseline_dir = matches.get_one::<String>("capture-baseline").cloned();
    let capture_diff_dir = matches.get_one::<String>("capture-diff").cloned();

    if let Some(ref dir) = capture_dir {
        let output_dir = PathBuf::from(dir.clone());
        aios_database::fast_model::set_capture_config(Some(
            aios_database::fast_model::CaptureConfig::new(
                output_dir,
                capture_width,
                capture_height,
                capture_include_descendants,
                capture_views,
                capture_baseline_dir.map(PathBuf::from),
                capture_diff_dir.map(PathBuf::from),
            ),
        ));
    } else {
        aios_database::fast_model::set_capture_config(None);
    }

    // ========== 首先处理 --debug-model / --root-model 参数（必须在所有导出逻辑之前） ==========
    let debug_model_refnos: Option<Vec<String>> = if any_model_requested {
        // --debug-model 才启用调试打印；--root-model 不启用
        if debug_model_requested {
            aios_core::set_debug_model_enabled(true);
            clear_ploop_debug_cache(); // 清理PLOOP调试文件缓存，允许重新生成
            println!("✅ 已启用 debug_model 调试信息打印");
        } else {
            println!("✅ 已启用 root-model 模式（不打印调试信息）");
        }

        if !db_option_ext.inner.gen_mesh {
            println!("🔄 自动开启 gen_mesh");
            db_option_ext.inner.gen_mesh = true;
        }

        // 确保 gen_model 也被启用，以便 is_gen_mesh_or_model() 返回 true
        if !db_option_ext.inner.gen_model {
            println!("🔄 自动开启 gen_model");
            db_option_ext.inner.gen_model = true;
        }

        // 从 --debug-model 或 --root-model 中取 refnos
        let cli_refnos: Vec<String> = matches
            .get_many::<String>("debug-model")
            .or_else(|| matches.get_many::<String>("root-model"))
            .map(|values| values.map(|s| s.to_string()).collect())
            .unwrap_or_else(Vec::new);

        let mode_label = if debug_model_requested {
            "debug-model"
        } else {
            "root-model"
        };

        if !cli_refnos.is_empty() {
            println!(
                "🔍 使用命令行指定的 {} 参考号: {:?}",
                mode_label, cli_refnos
            );
            db_option_ext.inner.debug_model_refnos = Some(cli_refnos.clone());
            Some(cli_refnos)
        } else if let Some(config_refnos) = config_debug_refnos.as_ref() {
            if config_refnos.is_empty() {
                println!("💡 仅启用 {} 模式，未指定参考号", mode_label);
                db_option_ext.inner.debug_model_refnos = Some(Vec::new());
                None
            } else {
                println!(
                    "🗂️ 使用配置文件中的 debug_model_refnos: {:?}",
                    config_refnos
                );
                db_option_ext.inner.debug_model_refnos = Some(config_refnos.clone());
                Some(config_refnos.clone())
            }
        } else {
            println!("💡 仅启用 {} 模式，未指定参考号", mode_label);
            db_option_ext.inner.debug_model_refnos = None;
            None
        }
    } else {
        None
    };

    if db_option_ext.export_parquet_after_gen
        && db_option_ext.inner.debug_model_refnos.is_none()
        && let Some(refnos) = debug_model_refnos
            .as_ref()
            .filter(|values| !values.is_empty())
    {
        db_option_ext.inner.debug_model_refnos = Some(refnos.clone());
    }

    if debug_model_requested {
        // 仅 --debug-model 才启用日志文件写入
        db_option_ext.inner.enable_log = true;
        let now = Local::now();
        let log_refno = debug_model_refnos
            .as_ref()
            .and_then(|refnos| refnos.first().map(|s| s.as_str()))
            .unwrap_or("debug");
        let log_filename = format!(
            "logs/{}_{}-{:02}-{:02}_{:02}-{:02}-{:02}.log",
            log_refno,
            now.year(),
            now.month(),
            now.day(),
            now.hour(),
            now.minute(),
            now.second()
        );
        unsafe {
            std::env::set_var("AIOS_LOG_FILE", log_filename);
        }
        aios_database::init_logging(true);
    }

    // spec 004：任务级指标采集（web_server 派发 sidecar job 时注入产物路径 env；
    // 未注入时为 no-op，不影响普通 CLI 使用）。
    aios_database::perf_metrics::init_task_metrics_from_env();

    if let Some(("init-project", sub_matches)) = matches.subcommand() {
        let cli_dbnums = sub_matches
            .get_many::<u32>("dbnums")
            .map(|values| values.copied().collect());
        return aios_database::init_project::run_init_project_mode(db_option_ext, cli_dbnums).await;
    }

    // ========== 处理 --gen-all-desi-indextree 参数 ==========
    if matches.get_flag("gen-all-desi-indextree") {
        println!("🔄 生成所有 DESI 类型的 indextree (忽略 manual_db_nums)...");
        aios_database::data_interface::db_meta_manager::generate_desi_indextree(true)?;
        println!("✅ indextree 生成完成");
        return Ok(());
    }

    // ========== 处理 --gen-indextree 参数 ==========
    if matches.contains_id("gen-indextree") {
        let dbnum: Option<u32> = matches
            .get_one::<String>("gen-indextree")
            .and_then(|s| s.parse().ok());

        if let Some(dbnum) = dbnum {
            println!("🔄 生成指定 dbnum={} 的 indextree...", dbnum);
            aios_database::data_interface::db_meta_manager::generate_single_indextree(dbnum)?;
        } else {
            println!("🔄 生成所有 DESI 类型的 indextree...");
            aios_database::data_interface::db_meta_manager::generate_desi_indextree(false)?;
        }
        println!("✅ indextree 生成完成");
        return Ok(());
    }

    // ========== 处理 --regen-model 参数 ==========
    let regen_model_requested = matches.get_flag("regen-model");
    let regen_auto_enabled_defer_db_write = false;
    if regen_model_requested {
        println!("🔄 检测到 --regen-model 参数，强制开启 FORCE_REGEN_MESH 模式");
        // 强制 mesh_worker 忽略 mesh_sig 缓存，确保本次能看到最新代码/配置效果。
        unsafe {
            std::env::set_var("FORCE_REGEN_MESH", "1");
        }
        // 元件库(cata_neg)/设计型负实体导出依赖布尔结果（CatePos），因此 regen-model 必须开启布尔运算。
        if !db_option_ext.inner.apply_boolean_operation {
            println!("🔄 --regen-model 自动开启 apply_boolean_operation（生成 CatePos 布尔结果）");
            db_option_ext.inner.apply_boolean_operation = true;
        }
        // mesh 已改为 insert_handle 内联处理，不再有竞态条件，无需 defer_db_write
    }

    // --defer-db-write：模型生成阶段不写 SurrealDB，SQL 输出到 .surql 文件
    let defer_db_write_explicit = matches.get_flag("defer-db-write");
    if defer_db_write_explicit {
        println!("⚠️ --defer-db-write 已停用，当前版本将忽略该参数并继续在线写库");
    }

    // --debug-model 是增量模式，不强制覆盖旧数据；
    // 只有 --regen-model 才需要 pre_cleanup_for_regen。

    // 模型导出请求：默认只导出不触发生成；--regen-model 或 --debug-model 前置生成。
    let model_export_requested = matches.get_flag("export-obj")
        || matches.get_flag("export-svg")
        || matches.get_flag("export-glb")
        || matches.get_flag("export-gltf")
        || matches.contains_id("export-obj-refnos")
        || matches.contains_id("export-glb-refnos")
        || matches.contains_id("export-gltf-refnos")
        || (any_model_requested && capture_dir.is_some());
    let post_gen_export_requested = matches.get_flag("export-parquet-after-gen");
    let non_post_gen_export_requested = model_export_requested
        || matches.get_flag("export-all-parquet")
        || matches.get_flag("export-all-relates")
        || matches.get_flag("export-dbnum-instances-json")
        || matches.get_flag("export-parquet")
        || matches.get_flag("export-dbnum-instances")
        || matches.get_flag("export-dbnum-instances-web")
        || matches.get_flag("export-v3");
    let any_export_requested = non_post_gen_export_requested || post_gen_export_requested;

    // ========== 执行模型生成 ==========
    // --regen-model: 清理后重新生成（强制 FORCE_REGEN_MESH）
    // --debug-model: 直接增量生成（不清理，补充缺失的 inst_geo/mesh/布尔结果）
    let should_generate = regen_model_requested || any_model_requested;
    if should_generate {
        // 确定生成的目标 refnos：优先 debug-model 指定的 refnos，其次 CLI 独立 refno 参数，
        // 再次 dbnum（查询所有 SITE），最后全库模式。
        let gen_refnos_vec: Vec<String> = if let Some(ref refnos) = debug_model_refnos {
            promote_generation_refnos_to_bran_hang_roots(refnos, verbose).await?
        } else if let Some(refnos) = matches.get_many::<String>("export-obj-refnos") {
            refnos.map(|s| s.to_string()).collect()
        } else if let Some(refnos) = matches.get_many::<String>("export-glb-refnos") {
            refnos.map(|s| s.to_string()).collect()
        } else if let Some(refnos) = matches.get_many::<String>("export-gltf-refnos") {
            refnos.map(|s| s.to_string()).collect()
        } else {
            vec![]
        };
        let gen_config = build_export_config(
            gen_refnos_vec,
            None,
            None,
            include_descendants,
            source_unit,
            target_unit,
            verbose,
            false,
            dbnum,
            split_by_site,
            include_negative,
            false,
        );

        if regen_model_requested {
            // --regen-model: 清理 + 强制重新生成
            let regen_result = cli_modes::run_regen_model(&gen_config, &db_option_ext).await;
            if let Err(err) = regen_result {
                aios_database::perf_metrics::finalize_task_metrics(false);
                return Err(err);
            }

            if !any_export_requested {
                aios_database::perf_metrics::finalize_task_metrics(true);
                println!("✅ --regen-model 单独执行完成（未请求导出，流程到此结束）");
                return Ok(());
            }
        } else {
            // --debug-model: 增量生成（不清理、不强制 FORCE_REPLACE_MESH）
            let gen_result = cli_modes::run_generate_model(&gen_config, &db_option_ext).await;
            if let Err(err) = gen_result {
                aios_database::perf_metrics::finalize_task_metrics(false);
                return Err(err);
            }
            if !any_export_requested {
                aios_database::perf_metrics::finalize_task_metrics(true);
                println!("✅ 模型生成单独执行完成（未请求导出，流程到此结束）");
                return Ok(());
            }
        }

        if post_gen_export_requested {
            let dbnums_hint = gen_config.dbnum.map(|value| vec![value]);
            let export_report =
                aios_database::fast_model::export_model::post_gen_export::export_parquet_after_generation_if_enabled(
                    &db_option_ext,
                    dbnums_hint,
                )
                .await;
            let export_report = match export_report {
                Ok(report) => report,
                Err(err) => {
                    aios_database::perf_metrics::finalize_task_metrics(false);
                    return Err(err);
                }
            };
            println!(
                "✅ 生成后 Parquet 导出: dbnums={:?} skipped={:?}",
                export_report.exported_dbnums, export_report.skipped_reason
            );
            if !non_post_gen_export_requested {
                aios_database::perf_metrics::finalize_task_metrics(true);
                println!("✅ --export-parquet-after-gen 执行完成（无其他导出请求，流程到此结束）");
                return Ok(());
            }
        }
    } else if post_gen_export_requested {
        anyhow::bail!(
            "--export-parquet-after-gen 需要与 --regen-model 或调试/导出模型生成请求一起使用"
        );
    }

    // 当前策略固定为 SurrealDB 输入，导出流程仅保留该路径。
    if model_export_requested {
        db_option_ext.use_surrealdb = true;
    }

    // ========== 处理 --debug-model 与导出标志的组合 ==========
    if let Some(refnos_vec) = &debug_model_refnos {
        // 如果用户开启了 --capture 但没有指定任何导出标志，则默认走 OBJ 导出流程。
        // 这样可保证 `--debug-model ... --capture ...` 行为稳定：统一复用导出链路收集几何并触发截图。
        if capture_dir.is_some()
            && !matches.get_flag("export-obj")
            && !matches.get_flag("export-svg")
            && !matches.get_flag("export-glb")
            && !matches.get_flag("export-gltf")
        {
            println!(
                "📸 调试模式 + 截图模式：生成模型并截图（默认走 OBJ 导出流程）: {:?}",
                refnos_vec
            );
            let config = build_export_config(
                refnos_vec.clone(),
                output_path,
                filter_nouns,
                capture_include_descendants,
                source_unit,
                target_unit,
                verbose,
                false, // regen-model 已在导出前集中处理
                None,
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            let result = export_obj_mode(config, &db_option_ext).await;
            post_export_steps(
                &matches,
                &db_option_ext,
                debug_model_refnos.as_deref(),
                verbose,
            )
            .await?;
            return result;
        }

        // 检查是否有导出标志
        if matches.get_flag("export-obj") {
            println!("🎯 导出 OBJ 模型 (调试模式): {:?}", refnos_vec);

            // debug-model + export-obj 时自动启用截图（如果用户没有显式指定 --capture）
            if capture_dir.is_none() {
                let auto_capture_dir = db_option_ext.get_project_output_dir().join("screenshots");
                println!("📸 自动启用截图: {}", auto_capture_dir.display());
                aios_database::fast_model::set_capture_config(Some(
                    aios_database::fast_model::CaptureConfig::new(
                        auto_capture_dir,
                        capture_width,
                        capture_height,
                        include_descendants,
                        capture_views,
                        None,
                        None,
                    ),
                ));
            }

            let config = build_export_config(
                refnos_vec.clone(),
                output_path,
                filter_nouns,
                include_descendants,
                source_unit,
                target_unit,
                verbose,
                false, // regen-model 已在导出前集中处理
                None,
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            let result = export_obj_mode(config, &db_option_ext).await;
            post_export_steps(
                &matches,
                &db_option_ext,
                debug_model_refnos.as_deref(),
                verbose,
            )
            .await?;
            return result;
        }

        if matches.get_flag("export-svg") {
            println!("🎯 导出 SVG 截面 (调试模式): {:?}", refnos_vec);
            let config = build_export_config(
                refnos_vec.clone(),
                output_path,
                filter_nouns,
                include_descendants,
                source_unit,
                target_unit,
                verbose,
                false, // regen-model 已在导出前集中处理
                None,
                split_by_site,
                include_negative,
                true, // export_svg = true
            );
            let result = export_obj_mode(config, &db_option_ext).await;
            post_export_steps(
                &matches,
                &db_option_ext,
                debug_model_refnos.as_deref(),
                verbose,
            )
            .await?;
            return result;
        }

        if matches.get_flag("export-glb") {
            println!("🎯 导出 GLB 模型 (调试模式): {:?}", refnos_vec);
            let mut config = build_export_config(
                refnos_vec.clone(),
                output_path,
                filter_nouns,
                include_descendants,
                source_unit,
                target_unit,
                verbose,
                false, // regen-model 已在导出前集中处理
                None,
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            config.use_basic_materials = use_basic_materials;
            let result = export_glb_mode(config, &db_option_ext).await;
            post_export_steps(
                &matches,
                &db_option_ext,
                debug_model_refnos.as_deref(),
                verbose,
            )
            .await?;
            return result;
        }

        if matches.get_flag("export-gltf") {
            println!("🎯 导出 glTF 模型 (调试模式): {:?}", refnos_vec);
            let mut config = build_export_config(
                refnos_vec.clone(),
                output_path,
                filter_nouns,
                include_descendants,
                source_unit,
                target_unit,
                verbose,
                false, // regen-model 已在导出前集中处理
                None,
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            config.use_basic_materials = use_basic_materials;
            let result = export_gltf_mode(config, &db_option_ext).await;
            post_export_steps(
                &matches,
                &db_option_ext,
                debug_model_refnos.as_deref(),
                verbose,
            )
            .await?;
            return result;
        }
    }

    // ========== 然后处理导出命令 ==========
    // 首先处理带 dbnum 的导出命令（查询所有 SITE 并分别导出）
    if let Some(dbnum) = dbnum {
        if matches.get_flag("export-obj") {
            println!("🎯 导出 OBJ 模型 (按 dbnum={} 的所有 SITE):", dbnum);
            let config = build_export_config(
                vec![], // 不传 refnos，由 dbnum 自动查询 SITE
                output_path,
                filter_nouns,
                include_descendants,
                source_unit,
                target_unit,
                verbose,
                false, // regen-model 已在导出前集中处理
                Some(dbnum),
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            let result = export_obj_mode(config, &db_option_ext).await;
            post_export_steps(
                &matches,
                &db_option_ext,
                debug_model_refnos.as_deref(),
                verbose,
            )
            .await?;
            return result;
        }

        if matches.get_flag("export-glb") {
            println!("🎯 导出 GLB 模型 (按 dbnum={} 的所有 SITE):", dbnum);
            let mut config = build_export_config(
                vec![],
                output_path,
                filter_nouns,
                include_descendants,
                source_unit,
                target_unit,
                verbose,
                false, // regen-model 已在导出前集中处理
                Some(dbnum),
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            config.use_basic_materials = use_basic_materials;
            let result = export_glb_mode(config, &db_option_ext).await;
            post_export_steps(
                &matches,
                &db_option_ext,
                debug_model_refnos.as_deref(),
                verbose,
            )
            .await?;
            return result;
        }

        if matches.get_flag("export-gltf") {
            println!("🎯 导出 glTF 模型 (按 dbnum={} 的所有 SITE):", dbnum);
            let mut config = build_export_config(
                vec![],
                output_path,
                filter_nouns,
                include_descendants,
                source_unit,
                target_unit,
                verbose,
                false, // regen-model 已在导出前集中处理
                Some(dbnum),
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            config.use_basic_materials = use_basic_materials;
            let result = export_gltf_mode(config, &db_option_ext).await;
            post_export_steps(
                &matches,
                &db_option_ext,
                debug_model_refnos.as_deref(),
                verbose,
            )
            .await?;
            return result;
        }
    }

    // no-dbnum 情况的默认"全库导出"由各导出模式内部处理（config.run_all_dbnos）

    // 然后处理独立的导出命令（不启用调试模式）
    if let Some(refnos) = matches.get_many::<String>("export-obj-refnos") {
        let refnos_vec: Vec<String> = refnos.map(|s| s.to_string()).collect();
        if !refnos_vec.is_empty() {
            println!("🎯 导出 OBJ 模型 (非调试模式): {:?}", refnos_vec);
            let config = build_export_config(
                refnos_vec,
                output_path,
                filter_nouns,
                include_descendants,
                source_unit,
                target_unit,
                verbose,
                false, // regen-model 已在导出前集中处理
                None,
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            let result = export_obj_mode(config, &db_option_ext).await;
            post_export_steps(
                &matches,
                &db_option_ext,
                debug_model_refnos.as_deref(),
                verbose,
            )
            .await?;
            return result;
        }
    }

    if let Some(refnos) = matches.get_many::<String>("export-glb-refnos") {
        let refnos_vec: Vec<String> = refnos.map(|s| s.to_string()).collect();
        if !refnos_vec.is_empty() {
            println!("🎯 导出 GLB 模型 (非调试模式): {:?}", refnos_vec);
            let mut config = build_export_config(
                refnos_vec,
                output_path,
                filter_nouns,
                include_descendants,
                source_unit,
                target_unit,
                verbose,
                false, // GLB 不需要 regenerate_plant_mesh
                None,
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            config.use_basic_materials = use_basic_materials;
            let result = export_glb_mode(config, &db_option_ext).await;
            post_export_steps(
                &matches,
                &db_option_ext,
                debug_model_refnos.as_deref(),
                verbose,
            )
            .await?;
            return result;
        }
    }

    if let Some(refnos) = matches.get_many::<String>("export-gltf-refnos") {
        let refnos_vec: Vec<String> = refnos.map(|s| s.to_string()).collect();
        if !refnos_vec.is_empty() {
            println!("🎯 导出 glTF 模型 (非调试模式): {:?}", refnos_vec);
            let mut config = build_export_config(
                refnos_vec,
                output_path,
                filter_nouns,
                include_descendants,
                source_unit,
                target_unit,
                verbose,
                false, // glTF 不需要 regenerate_plant_mesh
                None,
                split_by_site,
                include_negative,
                matches.get_flag("export-svg"),
            );
            config.use_basic_materials = use_basic_materials;
            let result = export_gltf_mode(config, &db_option_ext).await;
            post_export_steps(
                &matches,
                &db_option_ext,
                debug_model_refnos.as_deref(),
                verbose,
            )
            .await?;
            return result;
        }
    }

    // ========== 处理单独的导出标志（无 dbnum、无 refnos 时默认全库导出） ==========
    // 这是兜底逻辑：如果前面的条件都没匹配，说明用户只设置了导出标志

    if matches.get_flag("export-gltf") {
        println!("🎯 导出 glTF 模型 (全库模式 - MDB 所有 dbnum)");
        let config = ExportConfig::build_for_all_dbnos(
            output_path,
            filter_nouns,
            include_descendants,
            source_unit.to_string(),
            target_unit.to_string(),
            verbose,
            false, // regen-model 已在导出前集中处理
            use_basic_materials,
            split_by_site,
            include_negative,
            matches.get_flag("export-svg"),
        );
        let result = export_gltf_mode(config, &db_option_ext).await;
        post_export_steps(
            &matches,
            &db_option_ext,
            debug_model_refnos.as_deref(),
            verbose,
        )
        .await?;
        return result;
    }

    if matches.get_flag("export-glb") {
        println!("🎯 导出 GLB 模型 (全库模式 - MDB 所有 dbnum)");
        let config = ExportConfig::build_for_all_dbnos(
            output_path,
            filter_nouns,
            include_descendants,
            source_unit.to_string(),
            target_unit.to_string(),
            verbose,
            false, // regen-model 已在导出前集中处理
            use_basic_materials,
            split_by_site,
            include_negative,
            matches.get_flag("export-svg"),
        );
        let result = export_glb_mode(config, &db_option_ext).await;
        post_export_steps(
            &matches,
            &db_option_ext,
            debug_model_refnos.as_deref(),
            verbose,
        )
        .await?;
        return result;
    }

    if matches.get_flag("export-obj") {
        println!("🎯 导出 OBJ 模型 (全库模式 - MDB 所有 dbnum)");
        let config = ExportConfig::build_for_all_dbnos(
            output_path,
            filter_nouns,
            include_descendants,
            source_unit.to_string(),
            target_unit.to_string(),
            verbose,
            false, // regen-model 已在导出前集中处理
            use_basic_materials,
            split_by_site,
            include_negative,
            matches.get_flag("export-svg"),
        );
        let result = export_obj_mode(config, &db_option_ext).await;
        post_export_steps(
            &matches,
            &db_option_ext,
            debug_model_refnos.as_deref(),
            verbose,
        )
        .await?;
        return result;
    }

    if matches.get_flag("export-all-parquet") {
        use crate::cli_modes::export_all_parquet_mode;

        let dbnum = matches.get_one::<u32>("dbnum").copied();
        let export_bundle_dir = matches.get_one::<String>("output").map(PathBuf::from);
        let export_all_lods = matches.get_flag("export-all-lods");
        let export_refnos = matches.get_one::<String>("export-refnos").cloned();
        let owner_types: Option<Vec<String>> = matches
            .get_one::<String>("owner-types")
            .map(|s| s.split(',').map(|t| t.trim().to_uppercase()).collect());
        let name_config_path = matches.get_one::<String>("name-config").map(PathBuf::from);

        println!("🎯 导出 inst_relate 实体 (Prepack LOD + Parquet)");
        if let Some(ref refnos) = export_refnos {
            println!("   - 🎯 仅导出指定 refnos={}", refnos);
        } else if let Some(dbnum) = dbnum {
            println!("   - 按 dbnum={} 过滤", dbnum);
        } else {
            println!("   - 全表扫描（所有 dbnum）");
        }
        if let Some(ref types) = owner_types {
            println!("   - 按 owner_type 过滤: {:?}", types);
        }
        if let Some(ref path) = name_config_path {
            println!("   - 名称配置文件: {}", path.display());
        }

        return export_all_parquet_mode(
            dbnum,
            verbose,
            export_bundle_dir,
            owner_types,
            name_config_path,
            export_all_lods,
            export_refnos,
            source_unit.to_string(),
            target_unit.to_string(),
            &db_option_ext,
        )
        .await;
    }

    if matches.get_flag("export-dbnum-instances-json") {
        use crate::cli_modes::export_dbnum_instances_json_mode;
        use aios_core::pdms_types::RefnoEnum;
        use std::str::FromStr;

        let dbnum = matches.get_one::<u32>("dbnum").copied();
        let export_bundle_dir = matches.get_one::<String>("output").map(PathBuf::from);

        // 解析 --debug-model / --root-model 参数作为 root_refno
        let root_refno: Option<RefnoEnum> = matches
            .get_many::<String>("debug-model")
            .or_else(|| matches.get_many::<String>("root-model"))
            .and_then(|values| values.into_iter().next())
            .and_then(|s| {
                let refno_str = s.replace('_', "/");
                RefnoEnum::from_str(&refno_str).ok()
            });

        // 必须提供 dbnum 参数
        let dbnum = match dbnum {
            Some(n) => n,
            None => {
                eprintln!("❌ 错误: --export-dbnum-instances-json 需要提供 --dbnum 参数");
                eprintln!("   例如: cargo run -- --export-dbnum-instances-json --dbnum 1112");
                std::process::exit(1);
            }
        };

        let from_cache = matches.get_flag("from-cache");
        let detailed = matches.get_flag("detailed");

        // 处理 --use-surrealdb 参数
        let cli_use_surrealdb = matches.get_flag("use-surrealdb");
        if cli_use_surrealdb {
            db_option_ext.use_surrealdb = true;
        }

        println!("🎯 导出 dbnum 实例数据为 JSON（含 AABB）");
        println!("   - 按 dbnum={} 过滤", dbnum);
        println!(
            "   - 数据源: {}",
            if from_cache {
                "model cache"
            } else {
                "SurrealDB"
            }
        );
        println!(
            "   - 格式: {}",
            if detailed {
                "详细模式 (version 3)"
            } else {
                "精简模式 (version 4)"
            }
        );
        if let Some(ref refno) = root_refno {
            println!("   - 仅导出 {} 的 visible 子孙", refno);
        }
        if let Some(ref dir) = export_bundle_dir {
            println!("   - 输出目录: {}", dir.display());
        }

        return export_dbnum_instances_json_mode(
            dbnum,
            verbose,
            export_bundle_dir,
            &db_option_ext,
            true, // autorun=true
            root_refno,
            from_cache,
            detailed,
        )
        .await;
    }

    // 导出 delivery-code 兼容的 V2 JSON
    if matches.get_flag("export-dbnum-instances-web") {
        use crate::cli_modes::export_dbnum_instances_web_mode;
        use aios_core::pdms_types::RefnoEnum;
        use std::str::FromStr;

        let dbnum = matches.get_one::<u32>("dbnum").copied();
        let export_bundle_dir = matches.get_one::<String>("output").map(PathBuf::from);

        let root_refno: Option<RefnoEnum> = matches
            .get_many::<String>("debug-model")
            .or_else(|| matches.get_many::<String>("root-model"))
            .and_then(|values| values.into_iter().next())
            .and_then(|s| {
                let refno_str = s.replace('_', "/");
                RefnoEnum::from_str(&refno_str).ok()
            });

        let dbnum = match dbnum {
            Some(n) => n,
            None => {
                eprintln!("❌ 错误: --export-dbnum-instances-web 需要提供 --dbnum 参数");
                eprintln!("   例如: cargo run -- --export-dbnum-instances-web --dbnum 1112");
                std::process::exit(1);
            }
        };

        let cli_use_surrealdb = matches.get_flag("use-surrealdb");
        if cli_use_surrealdb {
            db_option_ext.use_surrealdb = true;
        }

        println!("🎯 导出 delivery-code 兼容的 V2 JSON");
        println!("   - dbnum={}", dbnum);
        if let Some(ref root) = root_refno {
            println!(
                "   - root_refno={}（仅子树，输出 instances_web_root_*.json）",
                root
            );
        }

        return export_dbnum_instances_web_mode(
            dbnum,
            verbose,
            export_bundle_dir,
            &db_option_ext,
            root_refno,
        )
        .await;
    }

    // 导出精简 V3 JSON（矩阵去重）
    if matches.get_flag("export-v3") {
        use crate::cli_modes::{export_dbnum_instances_v3_mode, merge_v3_instances_mode};
        use aios_core::pdms_types::RefnoEnum;
        use std::str::FromStr;

        let dbnum = matches.get_one::<u32>("dbnum").copied();
        let export_bundle_dir = matches.get_one::<String>("output").map(PathBuf::from);
        let target_unit = matches.get_one::<String>("v3-target-unit").cloned();
        let apply_rotation = matches.get_flag("v3-rotate");

        let root_refno: Option<RefnoEnum> = matches
            .get_many::<String>("debug-model")
            .or_else(|| matches.get_many::<String>("root-model"))
            .and_then(|values| values.into_iter().next())
            .and_then(|s| {
                let refno_str = s.replace('_', "/");
                RefnoEnum::from_str(&refno_str).ok()
            });

        let cli_use_surrealdb = matches.get_flag("use-surrealdb");
        if cli_use_surrealdb {
            db_option_ext.use_surrealdb = true;
        }

        if let Some(single_dbnum) = dbnum {
            // 单 dbnum 模式
            return export_dbnum_instances_v3_mode(
                single_dbnum,
                verbose,
                export_bundle_dir,
                &db_option_ext,
                root_refno,
                target_unit,
                apply_rotation,
            )
            .await;
        } else {
            // 全库模式：一次性查 inst_relate 全表直接输出
            use crate::cli_modes::export_all_instances_v3_mode;
            return export_all_instances_v3_mode(
                verbose,
                export_bundle_dir,
                &db_option_ext,
                target_unit,
                apply_rotation,
            )
            .await;
        }
    }

    // 合并所有 per-dbnum V3 JSON → instances_v3.json
    if matches.get_flag("merge-v3") {
        use crate::cli_modes::merge_v3_instances_mode;

        let export_bundle_dir = matches.get_one::<String>("output").map(PathBuf::from);
        return merge_v3_instances_mode(verbose, export_bundle_dir, &db_option_ext);
    }

    // 导出 dbnum 实例数据为 Parquet（显式 --export-parquet）
    // 或默认格式（--export-dbnum-instances，默认 Parquet）
    if matches.get_flag("export-parquet") || matches.get_flag("export-dbnum-instances") {
        use aios_core::pdms_types::RefnoEnum;
        use std::str::FromStr;

        let dbnum_cli = matches.get_one::<u32>("dbnum").copied();
        let root_refno: Option<RefnoEnum> = matches.get_one::<String>("root-refno").and_then(|s| {
            let refno_str = s.replace('_', "/");
            RefnoEnum::from_str(&refno_str).ok()
        });
        let dbnum_from_root = if let Some(root) = root_refno.as_ref() {
            let meta = aios_database::data_interface::db_meta();
            match meta.ensure_loaded() {
                Ok(()) => meta.get_dbnum_by_refno(*root),
                Err(err) if dbnum_cli.is_none() => {
                    eprintln!(
                        "❌ 错误: --root-refno={} 需要从 db_meta_info.json 推导 dbnum，但元数据加载失败: {}。请显式传入 --dbnum。",
                        root, err
                    );
                    std::process::exit(1);
                }
                Err(_) => None,
            }
        } else {
            None
        };

        let export_bundle_dir = matches.get_one::<String>("output").map(PathBuf::from);

        if let (Some(dbnum_cli), Some(dbnum_root), Some(root)) =
            (dbnum_cli, dbnum_from_root, root_refno.as_ref())
        {
            if dbnum_cli != dbnum_root {
                eprintln!(
                    "❌ 错误: --dbnum={} 与 --root-refno={} 推导 dbnum={} 不一致",
                    dbnum_cli, root, dbnum_root
                );
                std::process::exit(1);
            }
        }

        let single_dbnum = match (dbnum_cli, dbnum_from_root) {
            (Some(n), _) => Some(n),
            (None, Some(n)) => Some(n),
            (None, None) if root_refno.is_some() => {
                eprintln!(
                    "❌ 错误: 无法根据 --root-refno 推导 dbnum，请显式传入 --dbnum，避免误导出全量数据"
                );
                std::process::exit(1);
            }
            (None, None) => None,
        };

        // 未指定 dbnum：扫描 inst_relate 所有 distinct dbnum，逐一导出
        #[cfg(feature = "parquet-export")]
        if single_dbnum.is_none() {
            use aios_database::fast_model::export_model::export_dbnum_instances_parquet::query_distinct_dbnums_from_inst_relate;

            println!("📋 未指定 --dbnum，扫描 inst_relate 所有 dbnum...");
            init_surreal().await?;
            let dbnums = query_distinct_dbnums_from_inst_relate().await?;

            if dbnums.is_empty() {
                eprintln!("❌ 错误: inst_relate 表中未找到任何 dbnum");
                std::process::exit(1);
            }

            println!("📋 扫描到 {} 个 dbnum: {:?}", dbnums.len(), dbnums);

            for (i, dbnum) in dbnums.iter().enumerate() {
                println!(
                    "\n{} [{}/{}] 导出 dbnum={}",
                    "=".repeat(30),
                    i + 1,
                    dbnums.len(),
                    dbnum,
                );
                crate::cli_modes::export_dbnum_instances_parquet_mode(
                    *dbnum,
                    verbose,
                    export_bundle_dir.clone(),
                    &db_option_ext,
                    None,
                )
                .await?;
            }
            println!("\n🎉 所有 dbnum 导出完成！共 {} 个", dbnums.len());
            return Ok(());
        }

        #[cfg(not(feature = "parquet-export"))]
        if single_dbnum.is_none() {
            eprintln!("❌ 错误: parquet-export 特性未启用，请使用 --features parquet-export 编译");
            std::process::exit(1);
        }

        let dbnum = single_dbnum.unwrap();

        println!("🎯 导出 dbnum 实例数据为 Parquet（多表，供前端查询）");
        println!("   - 按 dbnum={} 过滤", dbnum);
        if let Some(ref root) = root_refno {
            println!("   - 根节点: {}（仅导出其 visible 子孙）", root);
        }
        println!("   - 数据源: SurrealDB");
        if let Some(ref dir) = export_bundle_dir {
            println!("   - 输出目录: {}", dir.display());
        }

        #[cfg(feature = "parquet-export")]
        return crate::cli_modes::export_dbnum_instances_parquet_mode(
            dbnum,
            verbose,
            export_bundle_dir,
            &db_option_ext,
            root_refno,
        )
        .await;
    }

    // 从 Parquet 目录导入 SQLite 空间索引（新生产路径的独立重建入口）
    if let Some(parquet_dir) = matches.get_one::<String>("import-spatial-index-parquet") {
        use crate::cli_modes::import_spatial_index_parquet_mode;

        let dbnum = matches.get_one::<u32>("dbnum").copied().ok_or_else(|| {
            anyhow::anyhow!("--import-spatial-index-parquet 需要同时指定 --dbnum")
        })?;
        let sqlite_path = matches
            .get_one::<String>("spatial-index-output")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("output/spatial_index.sqlite"));

        return import_spatial_index_parquet_mode(
            Path::new(parquet_dir),
            dbnum,
            &sqlite_path,
            verbose,
        );
    }

    // 导入 instances.json 到 SQLite 空间索引
    if let Some(json_path) = matches.get_one::<String>("import-spatial-index") {
        use crate::cli_modes::import_spatial_index_mode;

        let sqlite_path = matches
            .get_one::<String>("spatial-index-output")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("output/spatial_index.sqlite"));

        return import_spatial_index_mode(Path::new(json_path), &sqlite_path, verbose);
    }

    if let Some(rvm_path) = matches.get_one::<String>("import-rvm") {
        use crate::cli_modes::import_rvm_mode;

        let dbnum = matches
            .get_one::<u32>("dbnum")
            .copied()
            .ok_or_else(|| anyhow::anyhow!("--import-rvm 需要同时指定 --dbnum"))?;
        let mut att_paths: Vec<PathBuf> = matches
            .get_many::<String>("import-att")
            .map(|vals| vals.map(PathBuf::from).collect())
            .unwrap_or_default();
        if att_paths.is_empty() {
            let rvm_path = Path::new(rvm_path);
            let auto_att_path = rvm_path.with_extension("att.txt");
            if auto_att_path.exists() {
                if verbose {
                    println!(
                        "[rvm-import] 自动发现 ATT 文件: {}",
                        auto_att_path.display()
                    );
                }
                att_paths.push(auto_att_path);
            }
        }
        let relation_store_root = matches
            .get_one::<String>("relation-store-output")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("output/model_relations"));
        // spec 009:默认尝试把 RVM 组名解析为真实 PDMS refno(连接不可用自动回退)。
        let resolve_identity = !matches.get_flag("no-resolve-identity");

        return import_rvm_mode(
            Path::new(rvm_path),
            &att_paths,
            dbnum,
            &relation_store_root,
            resolve_identity,
            verbose,
        )
        .await;
    }

    if matches.get_flag("compare-rvm") {
        use crate::cli_modes::compare_rvm_cli_mode;
        use aios_core::pdms_types::RefnoEnum;
        use std::str::FromStr;

        let dbnum = matches
            .get_one::<u32>("dbnum")
            .copied()
            .ok_or_else(|| anyhow::anyhow!("--compare-rvm 需要同时指定 --dbnum"))?;
        let root_refno_str = matches
            .get_one::<String>("root-refno")
            .ok_or_else(|| anyhow::anyhow!("--compare-rvm 需要 --root-refno(如 2013286704/476)"))?;
        let root_refno = RefnoEnum::from_str(root_refno_str)
            .map_err(|e| anyhow::anyhow!("解析 --root-refno 失败: {e}"))?
            .refno()
            .0;
        let parquet_dir = matches
            .get_one::<String>("parquet-dir")
            .map(PathBuf::from)
            .ok_or_else(|| anyhow::anyhow!("--compare-rvm 需要 --parquet-dir"))?;
        let relation_store_root = matches
            .get_one::<String>("relation-store-output")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("output/model_relations"));
        let tol_aabb_mm = matches
            .get_one::<String>("tol-aabb-mm")
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(1.0);

        return compare_rvm_cli_mode(
            dbnum,
            root_refno,
            &relation_store_root,
            &parquet_dir,
            Path::new("runtime/rvm-compare"),
            tol_aabb_mm,
        );
    }

    if matches.get_flag("export-rvm-semantic-debug") {
        use crate::cli_modes::export_rvm_semantic_debug_mode;
        use aios_core::pdms_types::RefnoEnum;
        use std::str::FromStr;

        let dbnum = matches
            .get_one::<u32>("dbnum")
            .copied()
            .ok_or_else(|| anyhow::anyhow!("--export-rvm-semantic-debug 需要同时指定 --dbnum"))?;

        let root_refno = matches
            .get_one::<String>("root-refno")
            .cloned()
            .or_else(|| {
                matches
                    .get_many::<String>("debug-model")
                    .or_else(|| matches.get_many::<String>("root-model"))
                    .and_then(|values| values.into_iter().next().cloned())
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "--export-rvm-semantic-debug 需要通过 --root-refno 或 --debug-model/--root-model 指定作用域 refno"
                )
            })?;

        let normalized_refno = root_refno.replace('_', "/");
        let root_refno = RefnoEnum::from_str(&normalized_refno)
            .map_err(|e| anyhow::anyhow!("解析 root refno 失败: {} ({})", normalized_refno, e))?;
        let output_dir = matches.get_one::<String>("output").map(PathBuf::from);

        return export_rvm_semantic_debug_mode(dbnum, verbose, output_dir, root_refno);
    }

    if matches.get_flag("export-all-relates") {
        use crate::cli_modes::export_all_relates_mode;

        let dbnum = matches.get_one::<u32>("dbnum").copied();
        let export_bundle_dir = matches.get_one::<String>("output").map(PathBuf::from);
        let export_all_lods = matches.get_flag("export-all-lods");
        let export_refnos = matches.get_one::<String>("export-refnos").cloned();

        // 解析 owner-types 参数（逗号分隔）
        let owner_types: Option<Vec<String>> = matches
            .get_one::<String>("owner-types")
            .map(|s| s.split(',').map(|t| t.trim().to_uppercase()).collect());

        // 获取名称配置文件路径
        let name_config_path = matches.get_one::<String>("name-config").map(PathBuf::from);

        println!("🎯 导出 inst_relate 实体 (Prepack LOD 格式)");
        if let Some(ref refnos) = export_refnos {
            println!("   - 🎯 仅导出指定 refnos={}", refnos);
        } else if let Some(dbnum) = dbnum {
            println!("   - 按 dbnum={} 过滤", dbnum);
        } else {
            println!("   - 全表扫描（所有 dbnum）");
        }
        if let Some(ref types) = owner_types {
            println!("   - 按 owner_type 过滤: {:?}", types);
        }
        if let Some(ref path) = name_config_path {
            println!("   - 名称配置文件: {}", path.display());
        }

        return export_all_relates_mode(
            dbnum,
            verbose,
            export_bundle_dir,
            owner_types,
            name_config_path,
            export_all_lods,
            export_refnos,
            source_unit.to_string(),
            target_unit.to_string(),
            &db_option_ext,
        )
        .await;
    }

    // ========== 处理 incremental-sesno 子命令 ==========
    if let Some(incr_matches) = matches.subcommand_matches("incremental-sesno") {
        let from_sesno = incr_matches
            .get_one::<u32>("from-sesno")
            .copied()
            .expect("required by clap");
        let json_output = incr_matches.get_flag("json");
        let options = IncrementalSesnoRunOptions {
            file: incr_matches.get_one::<String>("file").map(PathBuf::from),
            dbnums: incr_matches
                .get_many::<u32>("dbnum")
                .map(|values| values.copied().collect())
                .unwrap_or_default(),
            from_sesno,
            to_sesno: incr_matches.get_one::<u32>("to-sesno").copied(),
            rescan_index: incr_matches.get_flag("rescan-index"),
            persist_data: !incr_matches.get_flag("no-persist"),
            generate_model: incr_matches.get_flag("generate-model"),
            source_observation_manifest: incr_matches
                .get_one::<String>("source-observation-manifest")
                .map(PathBuf::from),
            source_observation_manifest_hash: incr_matches
                .get_one::<String>("source-observation-manifest-hash")
                .cloned(),
            publication_handoff_dir: incr_matches
                .get_one::<String>("publication-handoff-dir")
                .map(PathBuf::from),
            release_id_prefix: incr_matches.get_one::<String>("release-id-prefix").cloned(),
            require_tree_index: incr_matches.get_flag("require-tree-index"),
            verbose,
        };
        let result = match run_incremental_sesno_once(&db_option_ext, options).await {
            Ok(result) => result,
            Err(err) => {
                aios_database::perf_metrics::finalize_task_metrics(false);
                return Err(err);
            }
        };

        if json_output {
            println!("{}", serde_json::to_string_pretty(&result.summary)?);
        } else {
            print_incremental_sesno_summary(&result);
        }

        aios_database::perf_metrics::finalize_task_metrics(true);
        return Ok(());
    }

    // ========== 处理 watch-incremental 子命令 ==========
    if let Some(watch_matches) = matches.subcommand_matches("watch-incremental") {
        let interval_secs = watch_matches
            .get_one::<u64>("interval-secs")
            .copied()
            .unwrap_or(30)
            .max(1);
        let once = watch_matches.get_flag("once");
        let generate_model = watch_matches.get_flag("generate-model");
        let require_tree_index = watch_matches.get_flag("require-tree-index");
        let json_output = watch_matches.get_flag("json");
        let observation_quiescence_window_ms = watch_matches
            .get_one::<u64>("observation-quiescence-window-ms")
            .copied()
            .expect("default value ensures this exists");
        let source_observation_dir = watch_matches
            .get_one::<String>("source-observation-dir")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                db_option_ext
                    .get_project_output_dir()
                    .join("model_versions")
                    .join("source_observations")
                    .join("watch-incremental")
            });
        let publication_handoff_dir = watch_matches
            .get_one::<String>("publication-handoff-dir")
            .map(PathBuf::from);
        let release_id_prefix = watch_matches
            .get_one::<String>("release-id-prefix")
            .cloned();
        let requested_dbnums: Vec<u32> = watch_matches
            .get_many::<u32>("dbnum")
            .map(|values| values.copied().collect())
            .unwrap_or_default();

        #[cfg(feature = "sqlite-index")]
        {
            let index_path = aios_database::data_interface::db_index::default_index_path(
                &db_option_ext.inner.project_name,
            );
            if !index_path.exists() || watch_matches.get_flag("force-initial-scan") {
                let report =
                    aios_database::data_interface::db_index::rebuild_from_config(true).await?;
                println!(
                    "✅ watch-incremental 初始 db_index: {} 个库, {} 条 ref0 映射",
                    report.db_files, report.ref0_total
                );
            }

            let mut baselines = {
                let store =
                    aios_database::data_interface::db_index::DbIndexStore::open(&index_path)?;
                store
                    .all_db_files()
                    .into_iter()
                    .filter(|rec| {
                        requested_dbnums.is_empty() || requested_dbnums.contains(&rec.dbnum)
                    })
                    .map(|rec| (rec.dbnum, rec.latest_sesno))
                    .collect::<std::collections::BTreeMap<_, _>>()
            };
            if baselines.is_empty() {
                anyhow::bail!("watch-incremental 未找到可监控 db 文件，请先检查 DbOption 工程路径");
            }
            println!(
                "👀 watch-incremental 启动: dbnums={} interval={}s generate_model={}",
                baselines.len(),
                interval_secs,
                generate_model
            );

            loop {
                let report =
                    aios_database::data_interface::db_index::rebuild_from_config(false).await?;
                let records = {
                    let store =
                        aios_database::data_interface::db_index::DbIndexStore::open(&index_path)?;
                    store.all_db_files()
                };
                let mut cycle_summaries = Vec::new();
                let mut update_count = 0usize;

                for rec in records {
                    if !requested_dbnums.is_empty() && !requested_dbnums.contains(&rec.dbnum) {
                        continue;
                    }
                    let Some(previous) = baselines.get(&rec.dbnum).copied() else {
                        baselines.insert(rec.dbnum, rec.latest_sesno);
                        continue;
                    };
                    if rec.latest_sesno <= previous {
                        continue;
                    }

                    println!(
                        "🔄 dbnum={} sesno {} -> {}，开始增量更新",
                        rec.dbnum, previous, rec.latest_sesno
                    );
                    let source_observation = build_watch_source_observation_gate(
                        &db_option_ext,
                        &rec,
                        previous,
                        rec.latest_sesno,
                        &source_observation_dir,
                        observation_quiescence_window_ms,
                    )?;
                    println!(
                        "🧾 source observation: {} sha256={}",
                        source_observation.manifest_path.display(),
                        source_observation.manifest_hash
                    );
                    let result = run_incremental_sesno_once(
                        &db_option_ext,
                        IncrementalSesnoRunOptions {
                            file: None,
                            dbnums: vec![rec.dbnum],
                            from_sesno: previous,
                            to_sesno: Some(rec.latest_sesno),
                            rescan_index: false,
                            persist_data: true,
                            generate_model,
                            source_observation_manifest: Some(
                                source_observation.manifest_path.clone(),
                            ),
                            source_observation_manifest_hash: Some(
                                source_observation.manifest_hash.clone(),
                            ),
                            publication_handoff_dir: publication_handoff_dir.clone(),
                            release_id_prefix: release_id_prefix.clone(),
                            require_tree_index,
                            verbose,
                        },
                    )
                    .await?;
                    baselines.insert(rec.dbnum, rec.latest_sesno);
                    update_count += 1;
                    if json_output {
                        cycle_summaries.push(result.summary);
                    } else {
                        print_incremental_sesno_summary(&result);
                    }
                }

                if json_output {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "scanned": report.scanned,
                            "skipped": report.skipped,
                            "db_files": report.db_files,
                            "updates": cycle_summaries,
                        }))?
                    );
                } else if update_count == 0 {
                    println!(
                        "ℹ️ watch-incremental 本轮无 sesno 增长 (scanned={} skipped={})",
                        report.scanned, report.skipped
                    );
                }

                if once {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
            }

            return Ok(());
        }
        #[cfg(not(feature = "sqlite-index"))]
        {
            let _ = (
                interval_secs,
                once,
                generate_model,
                json_output,
                observation_quiescence_window_ms,
                source_observation_dir,
                publication_handoff_dir,
                release_id_prefix,
                require_tree_index,
                requested_dbnums,
            );
            anyhow::bail!("watch-incremental 需要 sqlite-index feature");
        }
    }

    // ========== 处理 scan-db-index 子命令 ==========
    if let Some(scan_matches) = matches.subcommand_matches("scan-db-index") {
        let no_scan = scan_matches.get_flag("no-scan");
        #[cfg(feature = "sqlite-index")]
        {
            // index-only 预扫（pdms-io INDEX 直扫）+ 设计库精确依赖边。
            // 默认全量重扫；--no-scan 走指纹（mtime/size）增量，仅重扫变化的库。
            let report =
                aios_database::data_interface::db_index::rebuild_from_config(!no_scan).await?;
            println!(
                "✅ scan-db-index 完成: {} 个库, {} 条 ref0 映射",
                report.db_files, report.ref0_total
            );
            return Ok(());
        }
        #[cfg(not(feature = "sqlite-index"))]
        {
            let _ = no_scan;
            anyhow::bail!("scan-db-index 需要 sqlite-index feature（默认/web_server 构建已含）");
        }
    }

    // ========== 处理 gen-cata-closure 子命令 ==========
    if let Some(closure_matches) = matches.subcommand_matches("gen-cata-closure") {
        let rescan_index = closure_matches.get_flag("rescan-index");
        let out_override = closure_matches
            .get_one::<String>("out")
            .map(std::path::PathBuf::from);
        let seed_refno_strs: Option<Vec<String>> = closure_matches
            .get_many::<String>("seed-refnos")
            .map(|vals| vals.cloned().collect());
        #[cfg(feature = "sqlite-index")]
        {
            // 前置闭包 pass（spec 002 Q8）：独立于 sync，产出 cata_closure.json 供解析消费。
            let closure_started = std::time::Instant::now();
            let manifest = if let Some(seed_strs) = seed_refno_strs {
                // 按需模式：仅以指定设计元素（如单个 BRAN）子树播种。
                aios_database::data_interface::cata_closure::run_cata_closure_pass_for_refno_strs_from_config(
                    rescan_index,
                    &seed_strs,
                    out_override,
                )
                .await?
            } else {
                aios_database::data_interface::cata_closure::run_cata_closure_pass_from_config(
                    rescan_index,
                    out_override,
                )
                .await?
            };
            println!(
                "✅ gen-cata-closure 完成: {} 个 CATA 库 / seeds={} / visited={} / missing={} / rounds={}",
                manifest.by_dbnum.len(),
                manifest.seed_count,
                manifest.visited_count,
                manifest.missing,
                manifest.rounds
            );
            println!(
                "💡 解析时设置 AIOS_CATA_CLOSURE_MODE=manifest 启用 CATA 部分解析（manifest 缺失/未覆盖时自动整库回退）"
            );
            // spec 004：闭包阶段指标。
            let covered: std::collections::BTreeMap<u32, usize> = manifest
                .by_dbnum
                .iter()
                .map(|(dbnum, refs)| (*dbnum, refs.len()))
                .collect();
            aios_database::perf_metrics::record_closure_stage(
                manifest.seed_count,
                manifest.visited_count,
                manifest.rounds,
                manifest.missing,
                &covered,
                closure_started.elapsed().as_millis() as u64,
            );
            aios_database::perf_metrics::finalize_task_metrics(true);
            return Ok(());
        }
        #[cfg(not(feature = "sqlite-index"))]
        {
            let _ = (rescan_index, out_override, seed_refno_strs);
            anyhow::bail!("gen-cata-closure 需要 sqlite-index feature（默认/web_server 构建已含）");
        }
    }

    if let Some(record_id_matches) = matches.subcommand_matches("model-record-id-verify") {
        let refno = record_id_matches
            .get_one::<String>("refno")
            .expect("required by clap");
        let sesno = record_id_matches.get_one::<u32>("sesno").copied();
        let refno = parse_cli_model_record_refno(refno, sesno)?;
        let evidence =
            aios_database::fast_model::gen_model::model_record_id::build_model_record_id_evidence(
                refno,
            );
        if record_id_matches.get_flag("json") {
            println!("{}", serde_json::to_string_pretty(&evidence)?);
        } else {
            println!("input_refno: {}", evidence.input_refno);
            println!(
                "parts: ref0={} ref1={} sesno={}",
                evidence.parts.ref0, evidence.parts.ref1, evidence.parts.sesno
            );
            println!("inst_relate: {}", evidence.inst_relate);
            println!("inst_relate_aabb: {}", evidence.inst_relate_aabb);
            println!("geo_relate_0: {}", evidence.geo_relate_0);
            println!(
                "neg_relate_target_owned_0_0: {}",
                evidence.neg_relate_target_owned_0_0
            );
            println!("tubi_relate_0: {}", evidence.tubi_relate_0);
        }
        return Ok(());
    }

    // ========== 处理 verify-cata-closure 子命令（T008 离线校验）==========
    if let Some(verify_matches) = matches.subcommand_matches("verify-cata-closure") {
        #[cfg(all(feature = "sqlite-index", feature = "surreal-save"))]
        {
            use aios_database::data_interface::cata_closure_verify::{
                BaselineEndpoint, run_verify_from_cli,
            };
            let refnos: Vec<String> = verify_matches
                .get_many::<String>("refnos")
                .map(|v| v.cloned().collect())
                .unwrap_or_default();
            let baseline_ep = BaselineEndpoint {
                address: verify_matches
                    .get_one::<String>("baseline-endpoint")
                    .cloned()
                    .unwrap_or_default(),
                ns: verify_matches
                    .get_one::<String>("baseline-ns")
                    .cloned()
                    .unwrap_or_default(),
                db: verify_matches
                    .get_one::<String>("baseline-db")
                    .cloned()
                    .unwrap_or_default(),
                user: verify_matches
                    .get_one::<String>("baseline-user")
                    .cloned()
                    .unwrap_or_default(),
                pass: verify_matches
                    .get_one::<String>("baseline-pass")
                    .cloned()
                    .unwrap_or_default(),
            };
            let out_override = verify_matches
                .get_one::<String>("out")
                .map(std::path::PathBuf::from);
            let report = run_verify_from_cli(&refnos, &baseline_ep, out_override).await?;
            println!(
                "📋 verify-cata-closure: members={}（missing={}）/ hash {}:{}/{}（mismatch={}）/ tubi ondemand={} baseline={}{}",
                report.member_total,
                report.member_missing.len(),
                report.hash_baseline_source,
                report.hash_matched,
                report.hash_checked,
                report.hash_mismatched.len(),
                report.tubi_ondemand,
                report.tubi_baseline,
                if report.tubi_baseline_missing {
                    "（基准缺 tubi 数据，该项跳过）"
                } else {
                    ""
                }
            );
            for item in &report.cata_pe_counts {
                println!(
                    "   - CATA dbnum {}: 按需 {} / 基准 {} pe",
                    item.dbnum, item.ondemand, item.baseline
                );
            }
            if report.passed {
                println!("✅ 校验通过");
                return Ok(());
            }
            anyhow::bail!(
                "校验未通过（missing={}, mismatch={}）",
                report.member_missing.len(),
                report.hash_mismatched.len()
            );
        }
        #[cfg(not(all(feature = "sqlite-index", feature = "surreal-save")))]
        {
            let _ = verify_matches;
            anyhow::bail!("verify-cata-closure 需要 sqlite-index + surreal-save feature");
        }
    }

    // ========== 处理 spatial 子命令 ==========
    if let Some(spatial_matches) = matches.subcommand_matches("spatial") {
        use crate::cli_modes::spatial_query_refno_mode;

        match spatial_matches.subcommand() {
            Some(("query-refno", sub_m)) => {
                let refno = sub_m.get_one::<String>("refno").unwrap();
                let distance_mm = sub_m
                    .get_one::<String>("distance-mm")
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(1000.0);
                let include_self = sub_m.get_flag("include-self");
                let build_spatial = sub_m.get_flag("build-spatial");
                let expect_refnos: Option<Vec<String>> = sub_m
                    .get_many::<String>("expect-refnos")
                    .map(|v| v.map(|s| s.to_string()).collect());
                let verify_json_path = sub_m.get_one::<String>("verify-json").map(PathBuf::from);
                let write_verify_json_path = sub_m
                    .get_one::<String>("write-verify-json")
                    .map(PathBuf::from);

                return spatial_query_refno_mode(
                    refno,
                    distance_mm,
                    include_self,
                    build_spatial,
                    expect_refnos,
                    verify_json_path.as_deref(),
                    write_verify_json_path.as_deref(),
                    verbose,
                )
                .await;
            }
            _ => {
                println!("请指定 spatial 子命令，使用 --help 查看可用命令");
                return Ok(());
            }
        }
    }

    // ========== 处理 room 子命令 ==========
    if let Some(room_matches) = matches.subcommand_matches("room") {
        use crate::cli_modes::{
            export_room_instances_mode, room_clean_mode, room_compute_mode,
            room_compute_panel_mode, room_verify_json_mode,
        };
        use aios_core::RefnoEnum;
        use std::str::FromStr;

        match room_matches.subcommand() {
            Some(("compute", sub_m)) => {
                let keywords: Option<Vec<String>> = sub_m
                    .get_many::<String>("keywords")
                    .map(|kws| kws.map(|s| s.to_string()).collect());

                let db_nums: Option<Vec<u32>> = sub_m
                    .get_many::<String>("db-nums")
                    .map(|nums| nums.filter_map(|s| s.parse::<u32>().ok()).collect());

                let refno_root: Option<RefnoEnum> =
                    sub_m.get_one::<String>("refno-root").and_then(|s| {
                        let refno_str = s.replace('_', "/");
                        RefnoEnum::from_str(&refno_str).ok()
                    });

                let gen_panels_mesh = sub_m.get_flag("gen-panels-mesh");
                let report_json = sub_m.get_one::<String>("report-json").map(PathBuf::from);

                return room_compute_mode(
                    keywords,
                    db_nums,
                    refno_root,
                    gen_panels_mesh,
                    report_json,
                    verbose,
                    &db_option_ext,
                )
                .await;
            }
            Some(("compute-panel", sub_m)) => {
                let panel_refno = sub_m.get_one::<String>("panel-refno").unwrap();
                let generate_models = sub_m.get_flag("generate-models");
                let expect_refnos: Option<Vec<String>> = sub_m
                    .get_many::<String>("expect-refnos")
                    .map(|v| v.map(|s| s.to_string()).collect());
                let rebuild_spatial_index = sub_m.get_flag("rebuild-spatial-index");
                let report_json = sub_m.get_one::<String>("report-json").map(PathBuf::from);

                return room_compute_panel_mode(
                    panel_refno,
                    generate_models,
                    expect_refnos,
                    rebuild_spatial_index,
                    report_json,
                    verbose,
                    &db_option_ext,
                )
                .await;
            }
            Some(("rebuild-spatial-index", _)) => {
                return rebuild_room_spatial_index_mode(verbose).await;
            }
            Some(("clean", _)) => {
                return room_clean_mode(&db_option_ext).await;
            }
            Some(("verify-json", sub_m)) => {
                let input = sub_m.get_one::<String>("input").unwrap();
                return room_verify_json_mode(Path::new(input), &db_option_ext).await;
            }
            Some(("export", sub_m)) => {
                let output_dir = sub_m.get_one::<String>("output").map(PathBuf::from);
                return export_room_instances_mode(output_dir, verbose).await;
            }
            _ => {
                println!("请指定 room 子命令，使用 --help 查看可用命令");
                return Ok(());
            }
        }
    }

    // ========== 处理 --refresh-transform pe_transform 刷新命令 ==========
    if let Some(dbnums) = matches.get_many::<String>("refresh-transform") {
        let dbnums: Vec<u32> = dbnums.filter_map(|s| s.parse::<u32>().ok()).collect();
        if !dbnums.is_empty() {
            println!("🔄 刷新 pe_transform 缓存: dbnums={:?}", dbnums);
            init_surreal().await?;

            // 使用 DbMetaManager 加载元信息
            use aios_database::data_interface::db_meta;
            if let Err(e) = db_meta().try_load_default() {
                eprintln!("⚠️  {}", e);
                return Ok(());
            }

            if db_option_ext.clear_transform_before_refresh {
                let cleared =
                    aios_database::pe_transform_store::clear_pe_transform_for_dbnums(&dbnums)
                        .await?;
                println!(
                    "🧹 已清理历史 pe_transform: dbnums={:?}, refnos={}",
                    dbnums, cleared
                );
            }

            let count = aios_database::pe_transform_refresh::refresh_pe_transform_for_dbnums(
                &dbnums,
                &db_option_ext,
            )
            .await?;
            println!("✅ pe_transform 刷新完成，共处理 {} 个节点", count);
            if !db_option_ext.transform_compare_backends.is_empty() {
                let stats = aios_database::pe_transform_store::compare_backends_for_dbnums(
                    &db_option_ext,
                    &dbnums,
                )
                .await?;
                println!("📊 pe_transform backend 对比结果:");
                for stat in stats {
                    println!(
                        "   - {}: loaded={} missing={} mismatched={} max_delta={:.6} elapsed_ms={}",
                        stat.backend.as_str(),
                        stat.loaded,
                        stat.missing,
                        stat.mismatched,
                        stat.max_delta,
                        stat.elapsed_ms
                    );
                }
            }
            return Ok(());
        }
    }

    // 否则运行正常的应用程序
    run_app(Some(db_option_ext)).await
}

#[cfg(not(feature = "gui"))]
fn maybe_redirect_stdio_to_log_file() {
    use chrono::{Datelike, Local, Timelike};
    use std::fs::File;

    if std::env::var_os("AIOS_STDIO_REDIRECTED").is_some() {
        return;
    }

    let args: Vec<String> = std::env::args().collect();
    let has_flag = |flag: &str| args.iter().any(|a| a == flag);

    // 显式 verbose / 服务模式：不重定向，便于交互调试/观察运行状态。
    // 支持 -v 和 --verbose，避免用户加 -v 后仍被重定向导致终端无输出（看似卡住）。
    if has_flag("--verbose") || has_flag("-v") {
        // 允许用户按需设置 AIOS_LOG_TO_CONSOLE=1，把 log::info 也打印到控制台。
        return;
    }

    // 默认不重定向，避免 spawn 子进程后终端无输出导致“卡住”的假象。
    // 需要重定向时设置环境变量 AIOS_REDIRECT_STDIO=1。
    if std::env::var_os("AIOS_REDIRECT_STDIO")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        return;
    }

    // 仅在“可能产生海量输出”的路径下重定向（debug-model/export/capture 等）。
    let should_redirect = has_flag("--debug-model")
        || has_flag("--root-model")
        || has_flag("--export-obj")
        || has_flag("--export-glb")
        || has_flag("--export-gltf")
        || has_flag("--export-obj-refnos")
        || has_flag("--export-glb-refnos")
        || has_flag("--export-gltf-refnos")
        || has_flag("--capture")
        || has_flag("--log-model-error");

    if !should_redirect {
        return;
    }

    // 简易提取一个“标识 refno”用于日志文件命名（仅取第一个）。
    fn first_value_after_flag(args: &[String], flag: &str) -> Option<String> {
        let mut it = args.iter().enumerate();
        while let Some((i, a)) = it.next() {
            if a == flag {
                if let Some(v) = args.get(i + 1) {
                    if !v.starts_with('-') && !v.trim().is_empty() {
                        return Some(v.clone());
                    }
                }
            }
        }
        None
    }

    let ref_tag = first_value_after_flag(&args, "--debug-model")
        .or_else(|| first_value_after_flag(&args, "--root-model"))
        .or_else(|| first_value_after_flag(&args, "--export-obj-refnos"))
        .or_else(|| first_value_after_flag(&args, "--export-glb-refnos"))
        .or_else(|| first_value_after_flag(&args, "--export-gltf-refnos"))
        .unwrap_or_else(|| "run".to_string());

    let now = Local::now();
    let ts = format!(
        "{}-{:02}-{:02}_{:02}-{:02}-{:02}",
        now.year(),
        now.month(),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );
    let log_filename = format!("logs/{}_{}.log", ref_tag.replace('/', "_"), ts);

    // 预创建目录/文件；失败则回退到控制台模式。
    if let Some(parent) = std::path::Path::new(&log_filename).parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let Ok(out_file) = File::create(&log_filename) else {
        return;
    };
    let Ok(err_file) = out_file.try_clone() else {
        return;
    };

    // 重新执行自身：把 stdout/stderr 重定向到日志文件；父进程仅输出日志路径。
    // 注意：避免递归重进（AIOS_STDIO_REDIRECTED 标记）。
    let exe = &args[0];
    let child_status = StdCommand::new(exe)
        .args(&args[1..])
        .env("AIOS_STDIO_REDIRECTED", "1")
        .env("AIOS_LOG_FILE", &log_filename)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_file))
        .stderr(Stdio::from(err_file))
        .status();

    match child_status {
        Ok(status) => {
            // 仅打印一行提示，满足“默认不刷控制台”的诉求。
            eprintln!("日志已写入: {}", log_filename);
            std::process::exit(status.code().unwrap_or(1));
        }
        Err(_) => {
            // 启动失败则回退控制台输出
        }
    }
}
