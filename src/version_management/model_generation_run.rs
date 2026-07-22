//! Append-only audit ledger for model-generation attempts.
//!
//! A run owns exactly two possible immutable events:
//! - `started`, written before any model mutation;
//! - `terminal`, written once with either `succeeded` or `failed`.
//!
//! Event record ids are deterministic from `(run_id, event_type)`. Repeating an
//! append with the same canonical payload is idempotent; reusing the identity
//! with a different payload is an explicit conflict. The repository exposes no
//! update/delete operation and the table permissions deny those operations to
//! record-level users.

use std::collections::BTreeSet;

use aios_core::project_primary_db;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;
use thiserror::Error;

const TABLE_NAME: &str = "model_generation_run";
const PAYLOAD_VERSION: &str = "model-generation-run-event:v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelGenerationRunKind {
    Initialization,
    Incremental,
    NoOp,
    CatchUp,
    Repair,
}

impl ModelGenerationRunKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Initialization => "initialization",
            Self::Incremental => "incremental",
            Self::NoOp => "no_op",
            Self::CatchUp => "catch_up",
            Self::Repair => "repair",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelGenerationRunEventType {
    Started,
    Terminal,
}

impl ModelGenerationRunEventType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Terminal => "terminal",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "started" => Some(Self::Started),
            "terminal" => Some(Self::Terminal),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelGenerationRunTerminalResult {
    Succeeded,
    Failed,
}

impl ModelGenerationRunTerminalResult {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }
}

/// Input and pre-run model watermarks observed for one database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
pub struct ModelGenerationRunWatermark {
    pub dbnum: u32,
    pub data_watermark: u32,
    pub model_generation_watermark: u32,
}

/// Snapshot of a model anchor before or after a run.
///
/// `sesno`/`anchored_at` are optional so a started event can explicitly record
/// that a dbnum had no prior model anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
pub struct ModelGenerationAnchorSnapshot {
    pub dbnum: u32,
    pub sesno: Option<u32>,
    pub anchored_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelGenerationRunStarted {
    pub run_id: String,
    pub kind: ModelGenerationRunKind,
    pub actor: String,
    pub reason: String,
    pub dbnums: Vec<u32>,
    #[serde(default)]
    pub input_watermarks: Vec<ModelGenerationRunWatermark>,
    /// As-of time used to inspect the old hierarchy during cleanup.
    #[serde(default)]
    pub cleanup_read_at: Option<String>,
    /// Single as-of time used by all versioned generation reads.
    ///
    /// Initialization may leave this unset because it reads the isolated
    /// staging database's live current state.
    #[serde(default)]
    pub read_at: Option<String>,
    pub contract_hash: String,
    #[serde(default)]
    pub previous_model_anchors: Vec<ModelGenerationAnchorSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelGenerationRunTerminal {
    pub run_id: String,
    pub result: ModelGenerationRunTerminalResult,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub model_anchors: Vec<ModelGenerationAnchorSnapshot>,
    /// Model anchor time observed before this attempt.
    #[serde(default)]
    pub old_model_anchor_at: Option<String>,
    /// Model anchor time published by this attempt, when successful.
    #[serde(default)]
    pub new_model_anchor_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppendModelGenerationRunEventOutcome {
    pub run_id: String,
    pub event_type: ModelGenerationRunEventType,
    pub recorded_at: String,
    pub idempotent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedModelGenerationRunStarted {
    pub event: ModelGenerationRunStarted,
    pub started_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedModelGenerationRunTerminal {
    pub event: ModelGenerationRunTerminal,
    pub finished_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelGenerationRunState {
    /// A started event without a terminal event. At process recovery time this
    /// is an abandoned attempt, not a success.
    Abandoned,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelGenerationRunRecord {
    pub started: RecordedModelGenerationRunStarted,
    pub terminal: Option<RecordedModelGenerationRunTerminal>,
}

impl ModelGenerationRunRecord {
    pub fn state(&self) -> ModelGenerationRunState {
        match self.terminal.as_ref().map(|event| event.event.result) {
            None => ModelGenerationRunState::Abandoned,
            Some(ModelGenerationRunTerminalResult::Succeeded) => ModelGenerationRunState::Succeeded,
            Some(ModelGenerationRunTerminalResult::Failed) => ModelGenerationRunState::Failed,
        }
    }
}

#[derive(Debug, Error)]
pub enum ModelGenerationRunRepositoryError {
    #[error("invalid model_generation_run event: {0}")]
    Invalid(String),
    #[error(
        "immutable model_generation_run conflict: run_id={run_id} event_type={event_type} existing_hash={existing_hash} requested_hash={requested_hash}"
    )]
    Conflict {
        run_id: String,
        event_type: String,
        existing_hash: String,
        requested_hash: String,
    },
    #[error("model_generation_run terminal event has no started event: run_id={run_id}")]
    MissingStarted { run_id: String },
    #[error(
        "corrupt model_generation_run event: run_id={run_id} event_type={event_type}: {detail}"
    )]
    Corrupt {
        run_id: String,
        event_type: String,
        detail: String,
    },
    #[error("model_generation_run storage error: {0}")]
    Storage(#[from] anyhow::Error),
}

pub type ModelGenerationRunRepositoryResult<T> =
    std::result::Result<T, ModelGenerationRunRepositoryError>;

#[derive(Debug, Clone, Deserialize, SurrealValue)]
struct StoredEventRow {
    run_id: String,
    event_type: String,
    payload_hash: String,
    payload_json: String,
    event_at: String,
}

/// Define the immutable event table and its query indexes.
pub async fn ensure_model_generation_run_schema() -> ModelGenerationRunRepositoryResult<()> {
    let sql = r#"
DEFINE TABLE IF NOT EXISTS model_generation_run SCHEMAFULL
    PERMISSIONS
        FOR select FULL
        FOR create FULL
        FOR update NONE
        FOR delete NONE;
DEFINE FIELD IF NOT EXISTS run_id ON TABLE model_generation_run TYPE string;
DEFINE FIELD IF NOT EXISTS event_type ON TABLE model_generation_run TYPE string ASSERT $value IN ['started', 'terminal'];
DEFINE FIELD IF NOT EXISTS kind ON TABLE model_generation_run TYPE option<string>;
DEFINE FIELD IF NOT EXISTS actor ON TABLE model_generation_run TYPE option<string>;
DEFINE FIELD IF NOT EXISTS reason ON TABLE model_generation_run TYPE option<string>;
DEFINE FIELD IF NOT EXISTS dbnums ON TABLE model_generation_run TYPE array<int> DEFAULT [];
DEFINE FIELD IF NOT EXISTS input_watermarks ON TABLE model_generation_run TYPE array<object> DEFAULT [];
DEFINE FIELD IF NOT EXISTS input_watermarks.* ON TABLE model_generation_run TYPE object;
DEFINE FIELD IF NOT EXISTS input_watermarks.*.dbnum ON TABLE model_generation_run TYPE int;
DEFINE FIELD IF NOT EXISTS input_watermarks.*.data_watermark ON TABLE model_generation_run TYPE int;
DEFINE FIELD IF NOT EXISTS input_watermarks.*.model_generation_watermark ON TABLE model_generation_run TYPE int;
DEFINE FIELD IF NOT EXISTS cleanup_read_at ON TABLE model_generation_run TYPE option<string>;
DEFINE FIELD IF NOT EXISTS read_at ON TABLE model_generation_run TYPE option<string>;
DEFINE FIELD IF NOT EXISTS contract_hash ON TABLE model_generation_run TYPE option<string>;
DEFINE FIELD IF NOT EXISTS result ON TABLE model_generation_run TYPE option<string>;
DEFINE FIELD IF NOT EXISTS error ON TABLE model_generation_run TYPE option<string>;
DEFINE FIELD IF NOT EXISTS old_model_anchor_at ON TABLE model_generation_run TYPE option<string>;
DEFINE FIELD IF NOT EXISTS new_model_anchor_at ON TABLE model_generation_run TYPE option<string>;
DEFINE FIELD IF NOT EXISTS previous_model_anchors ON TABLE model_generation_run TYPE array<object> DEFAULT [];
DEFINE FIELD IF NOT EXISTS previous_model_anchors.* ON TABLE model_generation_run TYPE object;
DEFINE FIELD IF NOT EXISTS previous_model_anchors.*.dbnum ON TABLE model_generation_run TYPE int;
DEFINE FIELD IF NOT EXISTS previous_model_anchors.*.sesno ON TABLE model_generation_run TYPE option<int>;
DEFINE FIELD IF NOT EXISTS previous_model_anchors.*.anchored_at ON TABLE model_generation_run TYPE option<string>;
DEFINE FIELD IF NOT EXISTS model_anchors ON TABLE model_generation_run TYPE array<object> DEFAULT [];
DEFINE FIELD IF NOT EXISTS model_anchors.* ON TABLE model_generation_run TYPE object;
DEFINE FIELD IF NOT EXISTS model_anchors.*.dbnum ON TABLE model_generation_run TYPE int;
DEFINE FIELD IF NOT EXISTS model_anchors.*.sesno ON TABLE model_generation_run TYPE option<int>;
DEFINE FIELD IF NOT EXISTS model_anchors.*.anchored_at ON TABLE model_generation_run TYPE option<string>;
DEFINE FIELD IF NOT EXISTS payload_hash ON TABLE model_generation_run TYPE string;
DEFINE FIELD IF NOT EXISTS payload_json ON TABLE model_generation_run TYPE string;
DEFINE FIELD IF NOT EXISTS event_at ON TABLE model_generation_run TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS idx_model_generation_run_identity ON TABLE model_generation_run FIELDS run_id, event_type UNIQUE;
DEFINE INDEX IF NOT EXISTS idx_model_generation_run_event_at ON TABLE model_generation_run FIELDS event_at;
"#;
    project_primary_db()
        .query(sql)
        .await
        .context("define model_generation_run schema")?
        .check()
        .context("check model_generation_run schema statements")?;
    Ok(())
}

/// Append the immutable `started` event for a run.
pub async fn append_started(
    event: ModelGenerationRunStarted,
) -> ModelGenerationRunRepositoryResult<AppendModelGenerationRunEventOutcome> {
    ensure_model_generation_run_schema().await?;
    let event = normalize_started(event)?;
    let event_type = ModelGenerationRunEventType::Started;
    let (payload_json, payload_hash) = canonical_payload(event_type, &event)?;

    if let Some(outcome) =
        resolve_existing(&event.run_id, event_type, &payload_hash, &payload_json).await?
    {
        return Ok(outcome);
    }

    let record_id = event_record_id(&event.run_id, event_type);
    let sql = format!(
        "CREATE ONLY {record_id} SET \
         run_id = $run_id, event_type = 'started', kind = $kind, actor = $actor, \
         reason = $reason, dbnums = $dbnums, input_watermarks = $input_watermarks, \
         cleanup_read_at = $cleanup_read_at, read_at = $read_at, \
         contract_hash = $contract_hash, result = NONE, error = NONE, \
         old_model_anchor_at = NONE, new_model_anchor_at = NONE, \
         previous_model_anchors = $previous_model_anchors, model_anchors = [], \
         payload_hash = $payload_hash, payload_json = $payload_json, event_at = time::now();"
    );
    let create_result: anyhow::Result<()> = async {
        project_primary_db()
            .query(sql)
            .bind(("run_id", event.run_id.clone()))
            .bind(("kind", event.kind.as_str()))
            .bind(("actor", event.actor.clone()))
            .bind(("reason", event.reason.clone()))
            .bind(("dbnums", event.dbnums.clone()))
            .bind(("input_watermarks", event.input_watermarks.clone()))
            .bind(("cleanup_read_at", event.cleanup_read_at.clone()))
            .bind(("read_at", event.read_at.clone()))
            .bind(("contract_hash", event.contract_hash.clone()))
            .bind((
                "previous_model_anchors",
                event.previous_model_anchors.clone(),
            ))
            .bind(("payload_hash", payload_hash.clone()))
            .bind(("payload_json", payload_json.clone()))
            .await
            .context("append model_generation_run started event")?
            .check()
            .context("check model_generation_run started append")?;
        Ok(())
    }
    .await;

    finish_append(
        &event.run_id,
        event_type,
        &payload_hash,
        &payload_json,
        create_result,
    )
    .await
}

/// Append the single immutable terminal event for a run.
///
/// A terminal event is rejected unless its started event already exists.
pub async fn append_terminal(
    event: ModelGenerationRunTerminal,
) -> ModelGenerationRunRepositoryResult<AppendModelGenerationRunEventOutcome> {
    ensure_model_generation_run_schema().await?;
    let event = normalize_terminal(event)?;
    let started_row = load_stored_event(&event.run_id, ModelGenerationRunEventType::Started)
        .await?
        .ok_or_else(|| ModelGenerationRunRepositoryError::MissingStarted {
            run_id: event.run_id.clone(),
        })?;
    let started = decode_started(started_row)?;
    validate_terminal_dbnums(&event, &started.event)?;

    let event_type = ModelGenerationRunEventType::Terminal;
    let (payload_json, payload_hash) = canonical_payload(event_type, &event)?;
    if let Some(outcome) =
        resolve_existing(&event.run_id, event_type, &payload_hash, &payload_json).await?
    {
        return Ok(outcome);
    }

    let record_id = event_record_id(&event.run_id, event_type);
    let sql = format!(
        "CREATE ONLY {record_id} SET \
         run_id = $run_id, event_type = 'terminal', kind = NONE, actor = NONE, \
         reason = NONE, dbnums = [], input_watermarks = [], cleanup_read_at = NONE, \
         read_at = NONE, contract_hash = NONE, result = $result, error = $error, \
         old_model_anchor_at = $old_model_anchor_at, new_model_anchor_at = $new_model_anchor_at, \
         previous_model_anchors = [], model_anchors = $model_anchors, \
         payload_hash = $payload_hash, payload_json = $payload_json, event_at = time::now();"
    );
    let create_result: anyhow::Result<()> = async {
        project_primary_db()
            .query(sql)
            .bind(("run_id", event.run_id.clone()))
            .bind(("result", event.result.as_str()))
            .bind(("error", event.error.clone()))
            .bind(("model_anchors", event.model_anchors.clone()))
            .bind(("old_model_anchor_at", event.old_model_anchor_at.clone()))
            .bind(("new_model_anchor_at", event.new_model_anchor_at.clone()))
            .bind(("payload_hash", payload_hash.clone()))
            .bind(("payload_json", payload_json.clone()))
            .await
            .context("append model_generation_run terminal event")?
            .check()
            .context("check model_generation_run terminal append")?;
        Ok(())
    }
    .await;

    finish_append(
        &event.run_id,
        event_type,
        &payload_hash,
        &payload_json,
        create_result,
    )
    .await
}

/// Load both events for one run.
pub async fn load_model_generation_run(
    run_id: &str,
) -> ModelGenerationRunRepositoryResult<Option<ModelGenerationRunRecord>> {
    ensure_model_generation_run_schema().await?;
    let run_id = normalize_required("run_id", run_id.to_string())?;
    let started = load_stored_event(&run_id, ModelGenerationRunEventType::Started).await?;
    let terminal = load_stored_event(&run_id, ModelGenerationRunEventType::Terminal).await?;

    match (started, terminal) {
        (None, None) => Ok(None),
        (None, Some(_)) => Err(ModelGenerationRunRepositoryError::Corrupt {
            run_id,
            event_type: ModelGenerationRunEventType::Terminal.as_str().to_string(),
            detail: "terminal event exists without a started event".to_string(),
        }),
        (Some(started), terminal) => Ok(Some(ModelGenerationRunRecord {
            started: decode_started(started)?,
            terminal: terminal.map(decode_terminal).transpose()?,
        })),
    }
}

/// Return started events which have no terminal event.
///
/// This is intended for startup/recovery. Such rows represent abandoned
/// attempts and must never be interpreted as successful generation.
pub async fn list_abandoned_runs()
-> ModelGenerationRunRepositoryResult<Vec<RecordedModelGenerationRunStarted>> {
    ensure_model_generation_run_schema().await?;
    let sql = r#"
SELECT run_id, event_type, payload_hash, payload_json, type::string(event_at) AS event_at
    FROM model_generation_run WHERE event_type = 'started' ORDER BY event_at ASC;
SELECT VALUE run_id FROM model_generation_run WHERE event_type = 'terminal';
"#;
    let mut response = project_primary_db()
        .query(sql)
        .await
        .context("query unfinished model_generation_run events")?
        .check()
        .context("check unfinished model_generation_run query")?;
    let started_rows: Vec<StoredEventRow> = response
        .take(0)
        .context("decode model_generation_run started rows")?;
    let terminal_run_ids: BTreeSet<String> = response
        .take::<Vec<String>>(1)
        .context("decode model_generation_run terminal ids")?
        .into_iter()
        .collect();

    started_rows
        .into_iter()
        .filter(|row| !terminal_run_ids.contains(&row.run_id))
        .map(decode_started)
        .collect()
}

async fn finish_append(
    run_id: &str,
    event_type: ModelGenerationRunEventType,
    payload_hash: &str,
    payload_json: &str,
    create_result: anyhow::Result<()>,
) -> ModelGenerationRunRepositoryResult<AppendModelGenerationRunEventOutcome> {
    match create_result {
        Ok(()) => resolve_existing(run_id, event_type, payload_hash, payload_json)
            .await?
            .ok_or_else(|| ModelGenerationRunRepositoryError::Corrupt {
                run_id: run_id.to_string(),
                event_type: event_type.as_str().to_string(),
                detail: "CREATE ONLY succeeded but the event cannot be read back".to_string(),
            })
            .map(|mut outcome| {
                outcome.idempotent = false;
                outcome
            }),
        Err(create_error) => {
            // A concurrent identical writer may have won after our pre-read.
            // Re-read before surfacing the CREATE ONLY error.
            if let Some(outcome) =
                resolve_existing(run_id, event_type, payload_hash, payload_json).await?
            {
                return Ok(outcome);
            }
            Err(ModelGenerationRunRepositoryError::Storage(
                create_error.context(format!(
                    "append {} event for run_id={run_id}",
                    event_type.as_str()
                )),
            ))
        }
    }
}

async fn resolve_existing(
    run_id: &str,
    event_type: ModelGenerationRunEventType,
    requested_hash: &str,
    requested_json: &str,
) -> ModelGenerationRunRepositoryResult<Option<AppendModelGenerationRunEventOutcome>> {
    let Some(existing) = load_stored_event(run_id, event_type).await? else {
        return Ok(None);
    };
    verify_stored_row(&existing, event_type)?;
    if existing.payload_hash == requested_hash && existing.payload_json == requested_json {
        return Ok(Some(AppendModelGenerationRunEventOutcome {
            run_id: run_id.to_string(),
            event_type,
            recorded_at: existing.event_at,
            idempotent: true,
        }));
    }
    Err(ModelGenerationRunRepositoryError::Conflict {
        run_id: run_id.to_string(),
        event_type: event_type.as_str().to_string(),
        existing_hash: existing.payload_hash,
        requested_hash: requested_hash.to_string(),
    })
}

async fn load_stored_event(
    run_id: &str,
    event_type: ModelGenerationRunEventType,
) -> ModelGenerationRunRepositoryResult<Option<StoredEventRow>> {
    let record_id = event_record_id(run_id, event_type);
    let sql = format!(
        "SELECT run_id, event_type, payload_hash, payload_json, \
         type::string(event_at) AS event_at FROM {record_id};"
    );
    let mut response = project_primary_db()
        .query(sql)
        .await
        .with_context(|| {
            format!(
                "query model_generation_run event run_id={run_id} event_type={}",
                event_type.as_str()
            )
        })?
        .check()
        .context("check model_generation_run event query")?;
    let rows: Vec<StoredEventRow> = response
        .take(0)
        .context("decode model_generation_run event")?;
    if rows.len() > 1 {
        return Err(ModelGenerationRunRepositoryError::Corrupt {
            run_id: run_id.to_string(),
            event_type: event_type.as_str().to_string(),
            detail: format!("deterministic event id returned {} rows", rows.len()),
        });
    }
    Ok(rows.into_iter().next())
}

fn decode_started(
    row: StoredEventRow,
) -> ModelGenerationRunRepositoryResult<RecordedModelGenerationRunStarted> {
    verify_stored_row(&row, ModelGenerationRunEventType::Started)?;
    let event: ModelGenerationRunStarted =
        serde_json::from_str(&row.payload_json).context("decode started payload_json")?;
    if event.run_id != row.run_id {
        return Err(ModelGenerationRunRepositoryError::Corrupt {
            run_id: row.run_id,
            event_type: row.event_type,
            detail: "payload run_id does not match indexed run_id".to_string(),
        });
    }
    Ok(RecordedModelGenerationRunStarted {
        event,
        started_at: row.event_at,
    })
}

fn decode_terminal(
    row: StoredEventRow,
) -> ModelGenerationRunRepositoryResult<RecordedModelGenerationRunTerminal> {
    verify_stored_row(&row, ModelGenerationRunEventType::Terminal)?;
    let event: ModelGenerationRunTerminal =
        serde_json::from_str(&row.payload_json).context("decode terminal payload_json")?;
    if event.run_id != row.run_id {
        return Err(ModelGenerationRunRepositoryError::Corrupt {
            run_id: row.run_id,
            event_type: row.event_type,
            detail: "payload run_id does not match indexed run_id".to_string(),
        });
    }
    Ok(RecordedModelGenerationRunTerminal {
        event,
        finished_at: row.event_at,
    })
}

fn verify_stored_row(
    row: &StoredEventRow,
    expected_type: ModelGenerationRunEventType,
) -> ModelGenerationRunRepositoryResult<()> {
    let Some(actual_type) = ModelGenerationRunEventType::parse(&row.event_type) else {
        return Err(ModelGenerationRunRepositoryError::Corrupt {
            run_id: row.run_id.clone(),
            event_type: row.event_type.clone(),
            detail: "unknown event_type".to_string(),
        });
    };
    if actual_type != expected_type {
        return Err(ModelGenerationRunRepositoryError::Corrupt {
            run_id: row.run_id.clone(),
            event_type: row.event_type.clone(),
            detail: format!(
                "event type does not match deterministic id; expected={}",
                expected_type.as_str()
            ),
        });
    }
    let actual_hash = payload_hash(actual_type, &row.payload_json);
    if actual_hash != row.payload_hash {
        return Err(ModelGenerationRunRepositoryError::Corrupt {
            run_id: row.run_id.clone(),
            event_type: row.event_type.clone(),
            detail: format!(
                "payload hash mismatch stored={} actual={actual_hash}",
                row.payload_hash
            ),
        });
    }
    Ok(())
}

fn normalize_started(
    mut event: ModelGenerationRunStarted,
) -> ModelGenerationRunRepositoryResult<ModelGenerationRunStarted> {
    event.run_id = normalize_required("run_id", event.run_id)?;
    event.actor = normalize_required("actor", event.actor)?;
    event.reason = normalize_required("reason", event.reason)?;
    event.contract_hash = normalize_required("contract_hash", event.contract_hash)?;
    event.cleanup_read_at = normalize_optional(event.cleanup_read_at);
    event.read_at = normalize_optional(event.read_at);
    if event.dbnums.iter().any(|dbnum| *dbnum == 0) {
        return Err(ModelGenerationRunRepositoryError::Invalid(
            "dbnums must be non-zero".to_string(),
        ));
    }
    event.dbnums.sort_unstable();
    event.dbnums.dedup();
    if event.dbnums.is_empty() {
        return Err(ModelGenerationRunRepositoryError::Invalid(
            "dbnums must not be empty".to_string(),
        ));
    }
    if event.kind != ModelGenerationRunKind::Initialization && event.read_at.is_none() {
        return Err(ModelGenerationRunRepositoryError::Invalid(format!(
            "read_at is required for {} runs",
            event.kind.as_str()
        )));
    }

    event.input_watermarks = normalize_input_watermarks(event.input_watermarks)?;
    event.previous_model_anchors = normalize_anchors(event.previous_model_anchors)?;
    let dbnums: BTreeSet<u32> = event.dbnums.iter().copied().collect();
    if let Some(dbnum) = event
        .input_watermarks
        .iter()
        .map(|watermark| watermark.dbnum)
        .find(|dbnum| !dbnums.contains(dbnum))
    {
        return Err(ModelGenerationRunRepositoryError::Invalid(format!(
            "watermark dbnum={dbnum} is not present in dbnums"
        )));
    }
    if let Some(dbnum) = event
        .previous_model_anchors
        .iter()
        .map(|anchor| anchor.dbnum)
        .find(|dbnum| !dbnums.contains(dbnum))
    {
        return Err(ModelGenerationRunRepositoryError::Invalid(format!(
            "previous model anchor dbnum={dbnum} is not present in dbnums"
        )));
    }
    Ok(event)
}

fn normalize_terminal(
    mut event: ModelGenerationRunTerminal,
) -> ModelGenerationRunRepositoryResult<ModelGenerationRunTerminal> {
    event.run_id = normalize_required("run_id", event.run_id)?;
    event.error = normalize_optional(event.error);
    match (event.result, event.error.as_ref()) {
        (ModelGenerationRunTerminalResult::Succeeded, Some(_)) => {
            return Err(ModelGenerationRunRepositoryError::Invalid(
                "a succeeded terminal event must not contain error".to_string(),
            ));
        }
        (ModelGenerationRunTerminalResult::Failed, None) => {
            return Err(ModelGenerationRunRepositoryError::Invalid(
                "a failed terminal event requires error".to_string(),
            ));
        }
        _ => {}
    }
    event.model_anchors = normalize_anchors(event.model_anchors)?;
    event.old_model_anchor_at = normalize_optional(event.old_model_anchor_at);
    event.new_model_anchor_at = normalize_optional(event.new_model_anchor_at);
    Ok(event)
}

fn validate_terminal_dbnums(
    terminal: &ModelGenerationRunTerminal,
    started: &ModelGenerationRunStarted,
) -> ModelGenerationRunRepositoryResult<()> {
    let dbnums: BTreeSet<u32> = started.dbnums.iter().copied().collect();
    if let Some(dbnum) = terminal
        .model_anchors
        .iter()
        .map(|anchor| anchor.dbnum)
        .find(|dbnum| !dbnums.contains(dbnum))
    {
        return Err(ModelGenerationRunRepositoryError::Invalid(format!(
            "terminal model anchor dbnum={dbnum} is not present in the started event"
        )));
    }
    Ok(())
}

fn normalize_input_watermarks(
    mut watermarks: Vec<ModelGenerationRunWatermark>,
) -> ModelGenerationRunRepositoryResult<Vec<ModelGenerationRunWatermark>> {
    if watermarks.iter().any(|watermark| watermark.dbnum == 0) {
        return Err(ModelGenerationRunRepositoryError::Invalid(
            "watermark dbnum must be non-zero".to_string(),
        ));
    }
    watermarks.sort_by_key(|watermark| watermark.dbnum);
    let mut normalized: Vec<ModelGenerationRunWatermark> = Vec::with_capacity(watermarks.len());
    for watermark in watermarks {
        if let Some(previous) = normalized.last()
            && previous.dbnum == watermark.dbnum
        {
            if previous == &watermark {
                continue;
            }
            return Err(ModelGenerationRunRepositoryError::Invalid(format!(
                "conflicting watermarks for dbnum={}",
                watermark.dbnum
            )));
        }
        normalized.push(watermark);
    }
    Ok(normalized)
}

fn normalize_anchors(
    mut anchors: Vec<ModelGenerationAnchorSnapshot>,
) -> ModelGenerationRunRepositoryResult<Vec<ModelGenerationAnchorSnapshot>> {
    for anchor in &mut anchors {
        if anchor.dbnum == 0 {
            return Err(ModelGenerationRunRepositoryError::Invalid(
                "model anchor dbnum must be non-zero".to_string(),
            ));
        }
        anchor.anchored_at = normalize_optional(anchor.anchored_at.take());
    }
    anchors.sort_by_key(|anchor| anchor.dbnum);
    let mut normalized: Vec<ModelGenerationAnchorSnapshot> = Vec::with_capacity(anchors.len());
    for anchor in anchors {
        if let Some(previous) = normalized.last()
            && previous.dbnum == anchor.dbnum
        {
            if previous == &anchor {
                continue;
            }
            return Err(ModelGenerationRunRepositoryError::Invalid(format!(
                "conflicting model anchor snapshots for dbnum={}",
                anchor.dbnum
            )));
        }
        normalized.push(anchor);
    }
    Ok(normalized)
}

fn normalize_required(field: &str, value: String) -> ModelGenerationRunRepositoryResult<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        Err(ModelGenerationRunRepositoryError::Invalid(format!(
            "{field} must not be empty"
        )))
    } else {
        Ok(value)
    }
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn canonical_payload(
    event_type: ModelGenerationRunEventType,
    event: &impl Serialize,
) -> ModelGenerationRunRepositoryResult<(String, String)> {
    let payload_json =
        serde_json::to_string(event).context("serialize model_generation_run payload")?;
    let payload_hash = payload_hash(event_type, &payload_json);
    Ok((payload_json, payload_hash))
}

fn payload_hash(event_type: ModelGenerationRunEventType, payload_json: &str) -> String {
    let mut bytes = Vec::with_capacity(
        PAYLOAD_VERSION.len() + event_type.as_str().len() + payload_json.len() + 2,
    );
    bytes.extend_from_slice(PAYLOAD_VERSION.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(event_type.as_str().as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(payload_json.as_bytes());
    super::hashing::sha256_bytes(&bytes)
}

fn event_record_id(run_id: &str, event_type: ModelGenerationRunEventType) -> String {
    let mut bytes =
        Vec::with_capacity(PAYLOAD_VERSION.len() + run_id.len() + event_type.as_str().len() + 2);
    bytes.extend_from_slice(PAYLOAD_VERSION.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(run_id.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(event_type.as_str().as_bytes());
    let hash = super::hashing::sha256_bytes(&bytes);
    format!("{TABLE_NAME}:⟨{hash}⟩")
}
