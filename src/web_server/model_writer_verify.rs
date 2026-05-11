use axum::{Json, response::IntoResponse};
use serde::Deserialize;

use crate::fast_model::gen_model::model_writer::model_writer_contract_evidence;
use crate::options::ModelWriterMode;

#[derive(Debug, Deserialize, Default)]
pub struct ModelWriterVerifyRequest {
    #[serde(default)]
    pub mode: Option<ModelWriterMode>,
}

/// POST /api/model/writer-verify
///
/// Returns non-destructive lifecycle evidence for the selected backend. This endpoint is meant for
/// runtime verification of the backend boundary and does not write or clean up SurrealDB data.
pub async fn api_model_writer_verify(
    Json(req): Json<ModelWriterVerifyRequest>,
) -> impl IntoResponse {
    let mode = req.mode.unwrap_or_default();
    let evidence = model_writer_contract_evidence(mode);
    println!(
        "[model-writer:web-verify] backend={} writes_to_surreal={} runs_downstream_pipeline={} stages={}",
        evidence.backend,
        evidence.writes_to_surreal,
        evidence.runs_downstream_pipeline,
        evidence.stages.len()
    );
    Json(evidence)
}
