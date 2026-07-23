use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::model_generation_run::{
    ModelGenerationAnchorSnapshot, ModelGenerationRunKind, ModelGenerationRunStarted,
    ModelGenerationRunTerminal, ModelGenerationRunTerminalResult, ModelGenerationRunWatermark,
};
use crate::options::DbOptionExt;
use crate::versioned_db::model_gen_debt::{ModelGenDebtCoverage, analyze_model_gen_debt};

static RUN_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy)]
pub struct ModelGenCatchUpOptions {
    pub require_pe_owner_ready: bool,
    pub allow_full_regen: bool,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelGenCatchUpResult {
    pub dbnum: u32,
    pub coverage: ModelGenDebtCoverage,
    pub stale_debt_reconciled: usize,
    pub generation_success: Option<bool>,
    pub full_regen: bool,
    pub pe_owner_evidence: Option<serde_json::Value>,
    pub model_gen_anchor: Option<crate::versioned_db::version_commit::ModelGenAnchor>,
    pub parquet_export: Option<serde_json::Value>,
    pub read_at: Option<String>,
    pub cleanup_read_at: Option<String>,
    pub model_generation_run_id: Option<String>,
    pub model_gen_note: Option<String>,
}

pub async fn catch_up_model_generation(
    db_option_ext: &DbOptionExt,
    dbnum: u32,
    options: ModelGenCatchUpOptions,
) -> anyhow::Result<ModelGenCatchUpResult> {
    let mutation_lock =
        super::project_mutation_lock::ProjectMutationLock::acquire_for_current_command(
            db_option_ext,
        )?;
    catch_up_model_generation_with_lock(db_option_ext, dbnum, options, mutation_lock.held(), true)
        .await
}

pub(crate) async fn catch_up_model_generation_with_lock(
    db_option_ext: &DbOptionExt,
    dbnum: u32,
    options: ModelGenCatchUpOptions,
    _mutation_lock: super::project_mutation_lock::HeldProjectMutationLock<'_>,
    finalize: bool,
) -> anyhow::Result<ModelGenCatchUpResult> {
    let mut coverage = analyze_model_gen_debt(dbnum).await?;
    let stale_debt_reconciled = if !options.dry_run
        && db_option_ext.use_surrealdb
        && db_option_ext.model_writer_mode.writes_to_surreal()
        && !db_option_ext.gen_model_dry_run
        && !coverage.stale_debt_ranges.is_empty()
    {
        let count = coverage.stale_debt_ranges.len();
        crate::versioned_db::model_gen_debt::reconcile_model_gen_debt_covered_by_watermark(
            dbnum,
            coverage.model_generation_watermark,
        )
        .await?;
        coverage = analyze_model_gen_debt(dbnum).await?;
        count
    } else {
        0
    };
    let mut result = ModelGenCatchUpResult {
        dbnum,
        coverage,
        stale_debt_reconciled,
        generation_success: None,
        full_regen: false,
        pe_owner_evidence: None,
        model_gen_anchor: None,
        parquet_export: None,
        read_at: None,
        cleanup_read_at: None,
        model_generation_run_id: None,
        model_gen_note: None,
    };
    if result.coverage.data_watermark == 0
        || result.coverage.model_generation_watermark >= result.coverage.data_watermark
        || options.dry_run
    {
        return Ok(result);
    }
    if result.coverage.needs_full_regen {
        if !options.allow_full_regen {
            return Ok(result);
        }
        result.full_regen = true;
    }

    #[cfg(not(feature = "gen_model"))]
    anyhow::bail!("model generation catch-up requires the gen_model feature");

    #[cfg(feature = "gen_model")]
    {
        let evidence =
            super::increment_run::build_pe_owner_evidence(&[dbnum], options.require_pe_owner_ready)
                .await;
        result.pe_owner_evidence = Some(evidence.summary.clone());
        if options.require_pe_owner_ready && !evidence.ready {
            anyhow::bail!(
                "pe_owner_not_ready for dbnum={dbnum}: {:?}",
                evidence.not_ready_dbnums
            );
        }

        let mut generation_options = db_option_ext.clone();
        generation_options.inner.manual_db_nums = Some(vec![dbnum]);
        let update_log = result.coverage.merged_update_log.clone();
        let generation_read_spec =
            crate::generation_read::resolve_anchored_generation_read_spec(db_option_ext).await?;
        let cleanup_read_spec = crate::generation_read::resolve_cleanup_read_spec(
            dbnum,
            result.coverage.model_generation_watermark,
        )
        .await?;
        result.read_at = generation_read_spec.read_at().map(str::to_string);
        result.cleanup_read_at = cleanup_read_spec
            .as_ref()
            .and_then(|spec| spec.read_at())
            .map(str::to_string);

        let run_id = next_run_id(dbnum, result.coverage.data_watermark);
        result.model_generation_run_id = Some(run_id.clone());
        let run_kind = if result.full_regen {
            ModelGenerationRunKind::Repair
        } else if update_log.count() == 0 {
            ModelGenerationRunKind::NoOp
        } else if finalize {
            ModelGenerationRunKind::CatchUp
        } else {
            ModelGenerationRunKind::Incremental
        };
        result.model_gen_note = Some(match run_kind {
            ModelGenerationRunKind::NoOp => {
                "no-op: no model-impacting debt; data watermark advanced".to_string()
            }
            ModelGenerationRunKind::Repair => {
                "model generation completed by controlled repair".to_string()
            }
            ModelGenerationRunKind::CatchUp => {
                "model generation completed by debt catch-up".to_string()
            }
            _ => "model generation completed".to_string(),
        });
        super::model_generation_run::append_started(ModelGenerationRunStarted {
            run_id: run_id.clone(),
            kind: run_kind,
            actor: "model-gen-catch-up".to_string(),
            reason: if result.full_regen {
                "explicit controlled repair for debt coverage gap".to_string()
            } else {
                "consume continuous model_gen_debt".to_string()
            },
            dbnums: vec![dbnum],
            input_watermarks: vec![ModelGenerationRunWatermark {
                dbnum,
                data_watermark: result.coverage.data_watermark,
                model_generation_watermark: result.coverage.model_generation_watermark,
            }],
            cleanup_read_at: result.cleanup_read_at.clone(),
            read_at: result.read_at.clone(),
            contract_hash: crate::generation_read::hash_serializable(&(
                run_kind.as_str(),
                dbnum,
                result.coverage.model_generation_watermark,
                result.coverage.data_watermark,
                update_log.count(),
            )),
            previous_model_anchors: vec![ModelGenerationAnchorSnapshot {
                dbnum,
                sesno: (result.coverage.model_generation_watermark > 0)
                    .then_some(result.coverage.model_generation_watermark),
                anchored_at: result.cleanup_read_at.clone(),
            }],
        })
        .await?;

        // Once started is durable, every handled exit below is captured by this
        // execution boundary so cleanup/export/finalization failures also append
        // a terminal event instead of leaving an ambiguous started-only run.
        let execution: anyhow::Result<bool> = async {
            if result.full_regen {
                if let Some(cleanup_spec) = cleanup_read_spec.as_ref() {
                    let cleanup_session =
                        crate::generation_read::open_generation_read_session_with_spec(
                            &generation_options,
                            cleanup_spec,
                        )
                        .await?;
                    let cleanup_hierarchy = crate::generation_read::HierarchySnapshot::load(
                        cleanup_session.clone(),
                        &cleanup_session.manifest().dbnums(),
                    )
                    .await?;
                    crate::fast_model::gen_model::pdms_inst::pre_cleanup_for_regen_versioned(
                        &cleanup_hierarchy.all_refnos(),
                        &cleanup_hierarchy,
                    )
                    .await?;
                }
            }

            let generation_success = if update_log.count() == 0 && !result.full_regen {
                true
            } else {
                let incremental_log = (!result.full_regen).then_some(update_log);
                match crate::fast_model::gen_all_geos_data_with_read_specs(
                    Vec::new(),
                    &generation_options,
                    incremental_log,
                    generation_read_spec,
                    cleanup_read_spec,
                )
                .await
                {
                    Ok(generation) if generation.success => {
                        let export = crate::fast_model::export_model::post_gen_export::export_parquet_after_generation_if_enabled(
                            &generation_options,
                            Some(vec![dbnum]),
                        )
                        .await?;
                        result.parquet_export = Some(serde_json::to_value(export)?);
                        true
                    }
                    Ok(_) => false,
                    Err(error) => return Err(error.into()),
                }
            };
            result.generation_success = Some(generation_success);
            if generation_success
                && finalize
                && generation_options.use_surrealdb
                && generation_options.model_writer_mode.writes_to_surreal()
                && !generation_options.gen_model_dry_run
            {
                result.model_gen_anchor = Some(
                    crate::versioned_db::model_gen_debt::finalize_model_generation(
                        dbnum,
                        result.coverage.data_watermark,
                        result
                            .model_gen_note
                            .as_deref()
                            .unwrap_or("model generation completed"),
                    )
                    .await?,
                );
            }
            Ok(generation_success)
        }
        .await;
        let generation_success = match execution {
            Ok(success) => success,
            Err(error) => {
                append_terminal(&result, false, Some(format!("{error:#}"))).await?;
                return Err(error);
            }
        };
        if finalize {
            append_terminal(
                &result,
                generation_success,
                (!generation_success)
                    .then_some("model generation returned success=false".to_string()),
            )
            .await?;
        } else if !generation_success {
            append_terminal(
                &result,
                false,
                Some("model generation returned success=false".to_string()),
            )
            .await?;
        }
    }
    Ok(result)
}

pub(crate) async fn append_deferred_terminal(
    result: &ModelGenCatchUpResult,
    success: bool,
    error: Option<String>,
) -> anyhow::Result<()> {
    append_terminal(result, success, error).await
}

async fn append_terminal(
    result: &ModelGenCatchUpResult,
    success: bool,
    error: Option<String>,
) -> anyhow::Result<()> {
    let Some(run_id) = result.model_generation_run_id.clone() else {
        return Ok(());
    };
    let anchor = result
        .model_gen_anchor
        .as_ref()
        .map(|anchor| ModelGenerationAnchorSnapshot {
            dbnum: anchor.dbnum,
            sesno: Some(anchor.sesno),
            anchored_at: Some(anchor.anchored_at.clone()),
        });
    super::model_generation_run::append_terminal(ModelGenerationRunTerminal {
        run_id,
        result: if success {
            ModelGenerationRunTerminalResult::Succeeded
        } else {
            ModelGenerationRunTerminalResult::Failed
        },
        error,
        model_anchors: anchor.clone().into_iter().collect(),
        old_model_anchor_at: result.cleanup_read_at.clone(),
        new_model_anchor_at: anchor.and_then(|anchor| anchor.anchored_at),
    })
    .await?;
    Ok(())
}

fn next_run_id(dbnum: u32, target_sesno: u32) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let sequence = RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!(
        "model-gen-{}-{dbnum}-{target_sesno}-{millis}-{sequence}",
        std::process::id()
    )
}
