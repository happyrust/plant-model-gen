use std::sync::Arc;

use aios_database::fast_model::gen_model::model_writer::{
    DrainOnlyModelWriterBackend, DrainOnlyStats, ModelWriterBackend, ModelWriterStageReport,
    model_writer_contract_evidence,
};
use aios_database::options::ModelWriterMode;
use clap::{Arg, ArgAction, Command};
use serde::Serialize;

fn parse_mode(raw: &str) -> ModelWriterMode {
    match raw {
        "drain-only" => ModelWriterMode::DrainOnly,
        _ => ModelWriterMode::Surreal,
    }
}

#[derive(Serialize)]
struct DrainOnlyExecEvidence {
    backend: &'static str,
    writes_to_surreal: bool,
    runs_downstream_pipeline: bool,
    batches: usize,
    instances: usize,
    inst_info: usize,
    inst_tubi: usize,
    geo_keys: usize,
    geo_instances: usize,
    neg_relations: usize,
    ngmr_relations: usize,
    skipped_stages: usize,
    elapsed_ms: u128,
    stage_reports: Vec<ModelWriterStageReport>,
}

impl DrainOnlyExecEvidence {
    fn from_stats(
        backend: &'static str,
        writes_to_surreal: bool,
        runs_downstream_pipeline: bool,
        stats: DrainOnlyStats,
        stage_reports: Vec<ModelWriterStageReport>,
    ) -> Self {
        Self {
            backend,
            writes_to_surreal,
            runs_downstream_pipeline,
            batches: stats.batches,
            instances: stats.instances,
            inst_info: stats.inst_info,
            inst_tubi: stats.inst_tubi,
            geo_keys: stats.geo_keys,
            geo_instances: stats.geo_instances,
            neg_relations: stats.neg_relations,
            ngmr_relations: stats.ngmr_relations,
            skipped_stages: stats.skipped_stages,
            elapsed_ms: stats.elapsed.as_millis(),
            stage_reports,
        }
    }
}

async fn run_drain_only_exec() -> anyhow::Result<DrainOnlyExecEvidence> {
    let writer: Arc<dyn ModelWriterBackend> = Arc::new(DrainOnlyModelWriterBackend::new());
    let writes_to_surreal = writer.writes_to_surreal();
    let runs_downstream_pipeline = writer.runs_downstream_pipeline();

    let _cleanup_report = writer.cleanup().await?;
    let _init_report = writer.init().await?;
    let finish = writer.finalize().await?;

    let stats = finish.drain_only_stats.unwrap_or_default();
    Ok(DrainOnlyExecEvidence::from_stats(
        finish.writer_name,
        writes_to_surreal,
        runs_downstream_pipeline,
        stats,
        finish.stage_reports,
    ))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let matches = Command::new("model-writer-verify")
        .about("Emit safe ModelWriter backend lifecycle evidence as JSON")
        .arg(
            Arg::new("mode")
                .long("mode")
                .value_parser(["surreal", "drain-only"])
                .default_value("surreal"),
        )
        .arg(
            Arg::new("json")
                .long("json")
                .action(ArgAction::SetTrue)
                .help("Emit compact JSON evidence"),
        )
        .arg(
            Arg::new("exec")
                .long("exec")
                .action(ArgAction::SetTrue)
                .help(
                    "Actually drive the drain-only backend lifecycle (cleanup → init → finalize) \
                     with zero batches and emit real DrainOnlyStats + stage_reports JSON. \
                     Only valid with --mode drain-only; never writes to SurrealDB.",
                ),
        )
        .get_matches();

    let mode = matches
        .get_one::<String>("mode")
        .map(|value| parse_mode(value))
        .unwrap_or(ModelWriterMode::Surreal);
    let compact = matches.get_flag("json");
    let exec = matches.get_flag("exec");

    if exec {
        if !matches!(mode, ModelWriterMode::DrainOnly) {
            anyhow::bail!(
                "--exec only supports --mode drain-only; surreal exec would touch the database \
                 which violates the verify CLI safety contract"
            );
        }

        let evidence = run_drain_only_exec().await?;
        let json = if compact {
            serde_json::to_string(&evidence)?
        } else {
            serde_json::to_string_pretty(&evidence)?
        };
        println!("{}", json);
        return Ok(());
    }

    let evidence = model_writer_contract_evidence(mode);
    let json = if compact {
        serde_json::to_string(&evidence)?
    } else {
        serde_json::to_string_pretty(&evidence)?
    };
    println!("{}", json);

    Ok(())
}
