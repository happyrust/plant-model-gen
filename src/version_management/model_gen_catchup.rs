use serde::{Deserialize, Serialize};

use crate::options::DbOptionExt;
use crate::versioned_db::model_gen_debt::{ModelGenDebtCoverage, analyze_model_gen_debt};

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
    pub generation_success: Option<bool>,
    pub full_regen: bool,
    pub pe_owner_evidence: Option<serde_json::Value>,
    pub model_gen_anchor: Option<crate::versioned_db::version_commit::ModelGenAnchor>,
    pub parquet_export: Option<serde_json::Value>,
}

pub async fn catch_up_model_generation(
    db_option_ext: &DbOptionExt,
    dbnum: u32,
    options: ModelGenCatchUpOptions,
) -> anyhow::Result<ModelGenCatchUpResult> {
    let coverage = analyze_model_gen_debt(dbnum).await?;
    let mut result = ModelGenCatchUpResult {
        dbnum,
        coverage,
        generation_success: None,
        full_regen: false,
        pe_owner_evidence: None,
        model_gen_anchor: None,
        parquet_export: None,
    };
    if result.coverage.data_watermark == 0
        || result.coverage.model_generation_watermark >= result.coverage.data_watermark
        || options.dry_run
    {
        return Ok(result);
    }
    if result.coverage.needs_full_regen && !options.allow_full_regen {
        return Ok(result);
    }

    #[cfg(not(feature = "gen_model"))]
    anyhow::bail!("model generation catch-up requires the gen_model feature");

    #[cfg(feature = "gen_model")]
    {
        let evidence = super::increment_run::build_pe_owner_evidence(
            &[dbnum],
            options.require_pe_owner_ready,
        )
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
        let full_regen = result.coverage.needs_full_regen && options.allow_full_regen;
        result.full_regen = full_regen;
        let update_log = result.coverage.merged_update_log.clone();
        let generation_success = if !full_regen && update_log.count() == 0 {
            true
        } else {
            let generation = crate::fast_model::gen_all_geos_data(
                Vec::new(),
                &generation_options,
                (!full_regen).then_some(update_log),
            )
            .await?;
            if !generation.success {
                false
            } else {
                let export = crate::fast_model::export_model::post_gen_export::export_parquet_after_generation_if_enabled(
                    &generation_options,
                    Some(vec![dbnum]),
                )
                .await?;
                result.parquet_export = Some(serde_json::to_value(export)?);
                true
            }
        };
        result.generation_success = Some(generation_success);
        if generation_success
            && generation_options.use_surrealdb
            && generation_options.model_writer_mode.writes_to_surreal()
            && !generation_options.gen_model_dry_run
        {
            result.model_gen_anchor = Some(
                crate::versioned_db::model_gen_debt::finalize_model_generation(
                    dbnum,
                    result.coverage.data_watermark,
                )
                .await?,
            );
        }
    }
    Ok(result)
}
