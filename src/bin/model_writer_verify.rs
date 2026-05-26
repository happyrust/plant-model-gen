use std::sync::Arc;

use aios_database::fast_model::gen_model::model_writer::{
    DrainOnlyModelWriterBackend, DrainOnlyStats, ModelWriterBackend, ModelWriterStageReport,
    model_writer_contract_evidence,
};
#[cfg(feature = "model-writer-ducklake")]
use aios_database::fast_model::gen_model::model_writer_ducklake::{
    DuckLakeConfig, DuckLakeModelWriterBackend,
};
use aios_database::options::ModelWriterMode;
use clap::{Arg, ArgAction, Command};
use serde::Serialize;

fn parse_mode(raw: &str) -> ModelWriterMode {
    match raw {
        "drain-only" => ModelWriterMode::DrainOnly,
        "ducklake" | "duck-lake" => ModelWriterMode::DuckLake,
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

/// DuckLake smoke exec: open DuckDB + INSTALL/LOAD ducklake + ATTACH metadata
/// + CREATE 9 raw tables + finalize. Does NOT run a real generation batch;
/// the purpose is to probe runtime DuckLake availability (Slice 6 (b)-lite,
/// see goals/ducklake-model-writer/plan.md and blockers.md Known Blockers).
#[cfg(feature = "model-writer-ducklake")]
#[derive(Serialize)]
struct DuckLakeSmokeEvidence {
    backend: &'static str,
    writes_to_surreal: bool,
    runs_downstream_pipeline: bool,
    elapsed_ms: u128,
    stage_reports: Vec<ModelWriterStageReport>,
    ducklake_root: String,
}

#[cfg(feature = "model-writer-ducklake")]
async fn run_ducklake_smoke_exec() -> anyhow::Result<DuckLakeSmokeEvidence> {
    let cfg = DuckLakeConfig::default();
    let ducklake_root = cfg.root_dir.to_string_lossy().to_string();
    let writer: Arc<dyn ModelWriterBackend> = Arc::new(DuckLakeModelWriterBackend::new(cfg));
    let writes_to_surreal = writer.writes_to_surreal();
    let runs_downstream_pipeline = writer.runs_downstream_pipeline();

    let started = std::time::Instant::now();
    let _cleanup = writer.cleanup().await?;
    let _init = writer.init().await?;
    let finish = writer.finalize().await?;
    let elapsed_ms = started.elapsed().as_millis();

    Ok(DuckLakeSmokeEvidence {
        backend: finish.writer_name,
        writes_to_surreal,
        runs_downstream_pipeline,
        elapsed_ms,
        stage_reports: finish.stage_reports,
        ducklake_root,
    })
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let matches = Command::new("model-writer-verify")
        .about("Emit safe ModelWriter backend lifecycle evidence as JSON")
        .arg(
            Arg::new("mode")
                .long("mode")
                .value_parser(["surreal", "drain-only", "ducklake", "duck-lake"])
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
                    "Actually drive a safe backend lifecycle (cleanup → init → finalize) and emit \
                     runtime stage_reports JSON. Valid for --mode drain-only and --mode ducklake; \
                     never writes to SurrealDB.",
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
        match mode {
            ModelWriterMode::DrainOnly => {
                let evidence = run_drain_only_exec().await?;
                let json = if compact {
                    serde_json::to_string(&evidence)?
                } else {
                    serde_json::to_string_pretty(&evidence)?
                };
                println!("{}", json);
                return Ok(());
            }
            ModelWriterMode::DuckLake => {
                #[cfg(feature = "model-writer-ducklake")]
                {
                    let evidence = run_ducklake_smoke_exec().await?;
                    let json = if compact {
                        serde_json::to_string(&evidence)?
                    } else {
                        serde_json::to_string_pretty(&evidence)?
                    };
                    println!("{}", json);
                    return Ok(());
                }
                #[cfg(not(feature = "model-writer-ducklake"))]
                {
                    anyhow::bail!(
                        "--exec --mode ducklake requires feature `model-writer-ducklake`; \
                         rebuild with --features \"review,model-writer-drain,model-writer-ducklake\" \
                         (see goals/ducklake-model-writer/)"
                    );
                }
            }
            ModelWriterMode::Surreal => {
                anyhow::bail!(
                    "--exec --mode surreal is not supported; surreal exec would touch the live \
                     database which violates the verify CLI safety contract"
                );
            }
        }
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
