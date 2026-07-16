use crate::options::{DbOptionExt, get_db_option_ext, get_db_option_ext_from_path};
use crate::version_management::baseline_state::validate_baseline_state_request;
use crate::version_management::bounded_runner::{
    BoundedCommandRunRequest, BoundedRunCancelResponse, BoundedRunRecord, BoundedRunStatus,
    read_bounded_run_status, request_bounded_run_cancel, run_bounded_command,
};
use crate::version_management::ducklake_store::ModelVersionDuckLakeStore;
use crate::version_management::hashing::sha256_file;
use crate::version_management::history_baseline::{
    HistoryBaselineInspectRequest, HistoryBaselineInspectResponse, inspect_history_baseline,
};
use crate::version_management::model_release::{
    diff_model_release_units, diff_model_releases, get_model_component_unit_impacts,
    get_model_release_events, get_model_release_mesh_assets, get_model_release_scene,
    index_model_release_components, index_model_release_mesh_assets, index_model_release_units,
    list_model_releases, publish_history_model_release, reconcile_model_release,
    register_model_release, validate_model_release_pair_readiness,
};
use crate::version_management::release_package::load_model_package;
use crate::version_management::release_state_machine::{
    ModelReleaseStateMachineAction, ModelReleaseStateMachineReport,
    ModelReleaseStateMachineRequest, run_model_release_state_machine,
};
use crate::version_management::source_observation::{
    SourceObservationBuildRequest, build_source_observation_manifest,
    write_source_observation_manifest,
};
use crate::version_management::types::{
    ModelBaselineStateValidationRequest, ModelBaselineStateValidationResponse,
    ModelComponentDiffResponse, ModelComponentSnapshotStats, ModelComponentUnitImpactResponse,
    ModelHistoryReleasePublishRequest, ModelHistoryReleasePublishResponse,
    ModelHistoryReplayPrepareResponse, ModelReleaseEventsResponse,
    ModelReleaseMeshAssetIndexResponse, ModelReleaseMeshAssetIndexStats,
    ModelReleasePairReadinessResponse, ModelReleaseQuality, ModelReleaseReconcileReport,
    ModelReleaseRecord, ModelReleaseRegisterRequest, ModelReleaseRegistration,
    ModelReleaseSceneResponse, ModelReleaseStatus, ModelSourceObservationManifest,
    ModelUnitDiffResponse, ModelUnitIndexStats, ModelVersionDuckLakeConfig,
};
use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query},
    http::{HeaderValue, StatusCode, header::CONTENT_TYPE},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::thread;
use std::time::Duration;

const DEFAULT_DIFF_LIMIT: usize = 200;
const MAX_DIFF_LIMIT: usize = 5_000;
const DEFAULT_SCENE_LIMIT: usize = 2_000;
const MAX_SCENE_LIMIT: usize = 20_000;
const DEFAULT_HISTORY_BASELINE_SAMPLE_LIMIT: usize = 100;
const MAX_HISTORY_BASELINE_SAMPLE_LIMIT: usize = 1_000;

pub fn create_model_version_routes() -> Router {
    Router::new()
        .route("/api/model-version/releases", get(get_releases))
        .route(
            "/api/model-version/releases/register",
            post(post_register_release),
        )
        .route(
            "/api/model-version/releases/publish-history",
            post(post_publish_history_release),
        )
        .route(
            "/api/model-version/incremental/handoff",
            post(post_incremental_handoff),
        )
        .route("/api/model-version/runs", post(post_start_run))
        .route(
            "/api/model-version/runs/prepare-physical-snapshot",
            post(post_prepare_physical_snapshot_run),
        )
        .route(
            "/api/model-version/runs/prepare-history-replay",
            post(post_prepare_history_replay_run),
        )
        .route(
            "/api/model-version/runs/execute-history-replay-plan",
            post(post_execute_history_replay_plan_run),
        )
        .route(
            "/api/model-version/runs/parse-baseline",
            post(post_parse_baseline_run),
        )
        .route(
            "/api/model-version/runs/generate-full-model",
            post(post_generate_full_model_run),
        )
        .route("/api/model-version/runs/{run_id}", get(get_run_status))
        .route(
            "/api/model-version/runs/{run_id}/cancel",
            post(post_cancel_run),
        )
        .route(
            "/api/model-version/releases/{release_id}",
            get(get_release_detail),
        )
        .route(
            "/api/model-version/releases/{release_id}/runtime-scene",
            get(get_release_runtime_scene),
        )
        .route(
            "/api/model-version/releases/{release_id}/events",
            get(get_release_events),
        )
        .route(
            "/api/model-version/releases/{release_id}/reconcile",
            post(post_reconcile_release),
        )
        .route(
            "/api/model-version/releases/{release_id}/state-machine",
            post(post_release_state_machine),
        )
        .route(
            "/api/model-version/releases/{release_id}/index",
            post(post_index_release),
        )
        .route(
            "/api/model-version/releases/{release_id}/index-units",
            post(post_index_release_units),
        )
        .route(
            "/api/model-version/releases/{release_id}/index-assets",
            post(post_index_release_assets),
        )
        .route(
            "/api/model-version/releases/{release_id}/mesh-assets",
            get(get_release_mesh_assets),
        )
        .route(
            "/api/model-version/compare-readiness",
            get(get_compare_readiness),
        )
        .route(
            "/api/model-version/history-baseline-inspect",
            get(get_history_baseline_inspect),
        )
        .route("/api/model-version/diff", get(get_release_diff))
        .route("/api/model-version/unit-diff", get(get_unit_diff))
        .route(
            "/api/model-version/component-impact",
            get(get_component_unit_impact),
        )
        .route("/model-version/compare", get(compare_page))
        .route("/model-version/release-viewer", get(release_viewer_page))
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseListQuery {
    project: Option<String>,
    all_projects: Option<bool>,
    dbnum: Option<u32>,
    quality: Option<String>,
    complete_visual_only: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct RegisterReleaseRequest {
    project: Option<String>,
    release_id: String,
    release_label: Option<String>,
    release_quality: Option<String>,
    release_quality_reason: Option<String>,
    validation_flags: Option<Vec<String>>,
    spec_info_fallback_count: Option<u64>,
    branch_id: Option<String>,
    parent_release_id: Option<String>,
    derivation_type: Option<String>,
    dbnum: u32,
    /// specs/023: export sesno for unit_versions_v2 sync after index-units.
    sesno: Option<u32>,
    parquet_dir: Option<PathBuf>,
    release_root: Option<PathBuf>,
    metadata_json: Option<Value>,
    index_units: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct PublishHistoryReleaseRequest {
    project: Option<String>,
    release_id: String,
    release_label: Option<String>,
    release_quality: Option<String>,
    release_quality_reason: Option<String>,
    validation_flags: Option<Vec<String>>,
    spec_info_fallback_count: Option<u64>,
    branch_id: Option<String>,
    parent_release_id: Option<String>,
    dbnum: u32,
    source_db_file: PathBuf,
    from_sesno: u32,
    to_sesno: u32,
    parquet_dir: PathBuf,
    current_parquet_dir: Option<PathBuf>,
    scene_tree_dir: Option<PathBuf>,
    require_scene_tree: Option<bool>,
    release_root: Option<PathBuf>,
    metadata_json: Option<Value>,
    mesh_root: Option<PathBuf>,
    mesh_base_url: Option<String>,
    materialize_assets: Option<bool>,
    index_units: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct IncrementalHandoffRequest {
    project: Option<String>,
    handoff_manifest_path: PathBuf,
    candidate_index: Option<usize>,
    dbnum: Option<u32>,
    release_id: Option<String>,
    release_label: Option<String>,
    branch_id: Option<String>,
    parent_release_id: Option<String>,
    release_quality: Option<String>,
    release_quality_reason: Option<String>,
    validation_flags: Option<Vec<String>>,
    metadata_json: Option<Value>,
    index_units: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct IndexReleaseQuery {
    project: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseDetailQuery {
    project: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseEventsQuery {
    project: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ReconcileReleaseQuery {
    project: Option<String>,
    publish_if_complete: Option<bool>,
    fail_if_unusable: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseStateMachineRequest {
    project: Option<String>,
    action: Option<String>,
    reason: Option<String>,
    require_generation_job_id: Option<bool>,
    require_baseline_state: Option<bool>,
    require_asset_manifest: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct RuntimeSceneQuery {
    project: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
    component_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct MeshAssetIndexQuery {
    project: Option<String>,
    mesh_base_url: Option<String>,
    materialize: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct MeshAssetListQuery {
    project: Option<String>,
    missing_only: Option<bool>,
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
struct DiffQuery {
    project: Option<String>,
    from_release_id: String,
    to_release_id: String,
    change_type: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
struct UnitDiffQuery {
    project: Option<String>,
    from_release_id: String,
    to_release_id: String,
    unit_noun: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
struct ComponentImpactQuery {
    project: Option<String>,
    from_release_id: String,
    to_release_id: String,
    component_key: Option<String>,
    refno_u64: Option<u64>,
    dbnum: Option<u32>,
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
struct HistoryBaselineInspectQuery {
    project: Option<String>,
    source_db_file: PathBuf,
    target_sesno: u32,
    parse_sample_limit: Option<usize>,
    allow_nearest_sesno: Option<bool>,
    detail: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct RunQuery {
    project: Option<String>,
    state_dir: Option<PathBuf>,
    reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct StartRunRequest {
    project: Option<String>,
    state_dir: Option<PathBuf>,
    run_id: String,
    kind: Option<String>,
    argv: Vec<String>,
    executable: Option<PathBuf>,
    cwd: Option<PathBuf>,
    env: Option<BTreeMap<String, String>>,
    stdout_path: Option<PathBuf>,
    stderr_path: Option<PathBuf>,
    metrics_path: Option<PathBuf>,
    timeout_secs: Option<u64>,
    stale_heartbeat_secs: Option<u64>,
    poll_interval_ms: Option<u64>,
    source_db_file: Option<PathBuf>,
    source_db_sha256: Option<String>,
    force: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct PreparePhysicalSnapshotRunRequest {
    project: Option<String>,
    run_id: String,
    snapshot_id: Option<String>,
    dbnum: u32,
    source_db_file: PathBuf,
    baseline_dbnums: Option<Vec<u32>>,
    base_config_arg: Option<String>,
    dependency_files: Option<Vec<PathBuf>>,
    requested_sesno: Option<String>,
    resolved_sesno: Option<u32>,
    quiescence_window_ms: Option<u64>,
    executable: Option<PathBuf>,
    timeout_secs: Option<u64>,
    stale_heartbeat_secs: Option<u64>,
    poll_interval_ms: Option<u64>,
    copy_files: Option<bool>,
    force: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct PrepareHistoryReplayRunRequest {
    project: Option<String>,
    run_id: String,
    snapshot_id: Option<String>,
    release_id: String,
    release_label: Option<String>,
    baseline_release_id: Option<String>,
    parent_release_id: Option<String>,
    branch_id: Option<String>,
    dbnum: Option<u32>,
    baseline_dbnums: Option<Vec<u32>>,
    source_db_file: Option<PathBuf>,
    base_config_arg: Option<String>,
    from_sesno: u32,
    to_sesno: u32,
    replay_config_out: Option<PathBuf>,
    baseline_config_out: Option<PathBuf>,
    replay_output_root: Option<PathBuf>,
    replay_surreal_ns: Option<String>,
    current_parquet_dir: Option<PathBuf>,
    baseline_source_confirmed_at_from_sesno: Option<bool>,
    dependency_files: Option<Vec<PathBuf>>,
    quiescence_window_ms: Option<u64>,
    executable: Option<PathBuf>,
    timeout_secs: Option<u64>,
    stale_heartbeat_secs: Option<u64>,
    poll_interval_ms: Option<u64>,
    force: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct ExecuteHistoryReplayPlanRunRequest {
    project: Option<String>,
    state_dir: Option<PathBuf>,
    prepare_run_id: String,
    phase: String,
    run_id: Option<String>,
    executable: Option<PathBuf>,
    cwd: Option<PathBuf>,
    env: Option<BTreeMap<String, String>>,
    stdout_path: Option<PathBuf>,
    stderr_path: Option<PathBuf>,
    metrics_path: Option<PathBuf>,
    timeout_secs: Option<u64>,
    stale_heartbeat_secs: Option<u64>,
    poll_interval_ms: Option<u64>,
    force: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct ParseBaselineRunRequest {
    project: Option<String>,
    run_id: String,
    snapshot_id: String,
    dbnum: Option<u32>,
    dependency_files: Option<Vec<PathBuf>>,
    quiescence_window_ms: Option<u64>,
    executable: Option<PathBuf>,
    timeout_secs: Option<u64>,
    stale_heartbeat_secs: Option<u64>,
    poll_interval_ms: Option<u64>,
    force: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct GenerateFullModelRunRequest {
    project: Option<String>,
    run_id: String,
    snapshot_id: String,
    dbnum: Option<u32>,
    parse_run_id: Option<String>,
    allow_incomplete_parse: Option<bool>,
    diagnostic_reason: Option<String>,
    dependency_files: Option<Vec<PathBuf>>,
    quiescence_window_ms: Option<u64>,
    executable: Option<PathBuf>,
    timeout_secs: Option<u64>,
    stale_heartbeat_secs: Option<u64>,
    poll_interval_ms: Option<u64>,
    force: Option<bool>,
}

#[derive(Debug, Clone)]
struct VersionContext {
    project_name: String,
    output_root: PathBuf,
    mesh_root: PathBuf,
    ducklake: ModelVersionDuckLakeConfig,
}

#[derive(Debug, Serialize)]
struct ReleaseListApiData {
    project_name: Option<String>,
    ducklake_metadata_path: PathBuf,
    ducklake_data_path: PathBuf,
    releases: Vec<ModelReleaseView>,
}

#[derive(Debug, Serialize)]
struct RegisterReleaseApiData {
    ducklake_metadata_path: PathBuf,
    ducklake_data_path: PathBuf,
    registration: ModelReleaseRegistration,
}

#[derive(Debug, Serialize)]
struct PublishHistoryReleaseApiData {
    ducklake_metadata_path: PathBuf,
    ducklake_data_path: PathBuf,
    publish: ModelHistoryReleasePublishResponse,
}

#[derive(Debug, Serialize)]
struct IncrementalHandoffApiData {
    ducklake_metadata_path: PathBuf,
    ducklake_data_path: PathBuf,
    handoff_manifest_path: PathBuf,
    handoff_manifest_hash: String,
    handoff_run_id: String,
    selected_candidate: Value,
    registration: ModelReleaseRegistration,
}

#[derive(Debug, Serialize)]
struct ModelReleaseView {
    #[serde(flatten)]
    release: ModelReleaseRecord,
    package_url: Option<String>,
    manifest_url: Option<String>,
    viewer_url: String,
    release_viewer_url: String,
}

#[derive(Debug, Serialize)]
struct ReleaseDetailApiData {
    release: ModelReleaseView,
    manifest: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct ReleaseEventsApiData {
    ducklake_metadata_path: PathBuf,
    ducklake_data_path: PathBuf,
    events: ModelReleaseEventsResponse,
}

#[derive(Debug, Serialize)]
struct ReconcileReleaseApiData {
    ducklake_metadata_path: PathBuf,
    ducklake_data_path: PathBuf,
    reconcile: ModelReleaseReconcileReport,
}

#[derive(Debug, Serialize)]
struct ReleaseStateMachineApiData {
    ducklake_metadata_path: PathBuf,
    ducklake_data_path: PathBuf,
    state_machine: ModelReleaseStateMachineReport,
}

#[derive(Debug, Serialize)]
struct IndexReleaseApiData {
    ducklake_metadata_path: PathBuf,
    ducklake_data_path: PathBuf,
    component_index: ModelComponentSnapshotStats,
}

#[derive(Debug, Serialize)]
struct IndexUnitApiData {
    ducklake_metadata_path: PathBuf,
    ducklake_data_path: PathBuf,
    unit_index: ModelUnitIndexStats,
}

#[derive(Debug, Serialize)]
struct IndexMeshAssetsApiData {
    ducklake_metadata_path: PathBuf,
    ducklake_data_path: PathBuf,
    mesh_asset_index: ModelReleaseMeshAssetIndexStats,
}

#[derive(Debug, Serialize)]
struct MeshAssetsApiData {
    ducklake_metadata_path: PathBuf,
    ducklake_data_path: PathBuf,
    mesh_assets: ModelReleaseMeshAssetIndexResponse,
}

#[derive(Debug, Serialize)]
struct DiffApiData {
    ducklake_metadata_path: PathBuf,
    ducklake_data_path: PathBuf,
    diff: ModelComponentDiffResponse,
}

#[derive(Debug, Serialize)]
struct CompareReadinessApiData {
    ducklake_metadata_path: PathBuf,
    ducklake_data_path: PathBuf,
    readiness: ModelReleasePairReadinessResponse,
}

#[derive(Debug, Serialize)]
struct HistoryBaselineInspectApiData {
    project_name: String,
    source_db_file: PathBuf,
    target_sesno: u32,
    inspect: HistoryBaselineInspectResponse,
}

#[derive(Debug, Serialize)]
struct UnitDiffApiData {
    ducklake_metadata_path: PathBuf,
    ducklake_data_path: PathBuf,
    unit_diff: ModelUnitDiffResponse,
}

#[derive(Debug, Serialize)]
struct ComponentImpactApiData {
    ducklake_metadata_path: PathBuf,
    ducklake_data_path: PathBuf,
    impact: ModelComponentUnitImpactResponse,
}

#[derive(Debug, Serialize)]
struct ModelVersionRunApiData {
    state_dir: PathBuf,
    run_id: String,
    record: Option<BoundedRunRecord>,
    launch_observed: bool,
}

#[derive(Debug, Serialize)]
struct ModelVersionRunStatusApiData {
    state_dir: PathBuf,
    run: BoundedRunRecord,
}

#[derive(Debug, Serialize)]
struct ModelVersionRunCancelApiData {
    state_dir: PathBuf,
    cancel: BoundedRunCancelResponse,
}

#[derive(Debug, Serialize)]
struct ModelVersionPipelineRunApiData {
    state_dir: PathBuf,
    run_id: String,
    kind: String,
    command_argv: Vec<String>,
    snapshot_id: Option<String>,
    source_observation_manifest_path: PathBuf,
    source_observation_manifest_hash: String,
    source_observation: ModelSourceObservationManifest,
    baseline_state_manifest_path: Option<PathBuf>,
    baseline_state_manifest_hash: Option<String>,
    history_replay: Option<PrepareHistoryReplayRunEvidence>,
    parse_run_id: Option<String>,
    parse_run_status: Option<BoundedRunStatus>,
    diagnostic_reason: Option<String>,
    record: Option<BoundedRunRecord>,
    launch_observed: bool,
}

#[derive(Debug, Clone, Serialize)]
struct PrepareHistoryReplayRunEvidence {
    source_mode: String,
    baseline_source_confirmed_at_from_sesno: bool,
    source_db_file: PathBuf,
    base_config_arg: String,
    replay_config_arg: PathBuf,
    baseline_config_arg: PathBuf,
    replay_output_root: PathBuf,
    current_parquet_dir: PathBuf,
}

#[derive(Debug, Serialize)]
struct ExecuteHistoryReplayPlanRunApiData {
    state_dir: PathBuf,
    run_id: String,
    kind: String,
    phase: String,
    prepare_run_id: String,
    prepare_record: BoundedRunRecord,
    prepare_stdout_path: PathBuf,
    prepare_stdout_hash: String,
    plan: HistoryReplayPlanExecutionSummary,
    command_argv: Vec<String>,
    source_db_file: PathBuf,
    expected_source_db_sha256: String,
    record: Option<BoundedRunRecord>,
    launch_observed: bool,
}

#[derive(Debug, Serialize)]
struct HistoryReplayPlanExecutionSummary {
    project_name: String,
    release_id: String,
    baseline_release_id: String,
    dbnum: u32,
    from_sesno: u32,
    to_sesno: u32,
    source_db_file: PathBuf,
    replay_config_arg: PathBuf,
    baseline_config_arg: PathBuf,
    replay_output_root: PathBuf,
    replay_parquet_dir: PathBuf,
    current_parquet_dir: PathBuf,
    safety_checks: Value,
}

struct PreparedHistoryReplayPlanRun {
    run_request: BoundedCommandRunRequest,
    phase: String,
    prepare_run_id: String,
    prepare_record: BoundedRunRecord,
    prepare_stdout_path: PathBuf,
    prepare_stdout_hash: String,
    plan: HistoryReplayPlanExecutionSummary,
    source_db_file: PathBuf,
    expected_source_db_sha256: String,
}

struct PreparedPipelineRun {
    run_request: BoundedCommandRunRequest,
    snapshot_id: Option<String>,
    source_observation_manifest_path: PathBuf,
    source_observation_manifest_hash: String,
    source_observation: ModelSourceObservationManifest,
    baseline_state_manifest_path: Option<PathBuf>,
    baseline_state_manifest_hash: Option<String>,
    history_replay: Option<PrepareHistoryReplayRunEvidence>,
    parse_run_id: Option<String>,
    parse_run_status: Option<BoundedRunStatus>,
    diagnostic_reason: Option<String>,
}

struct IncrementalHandoffRegisterPlan {
    register_request: ModelReleaseRegisterRequest,
    handoff_manifest_path: PathBuf,
    handoff_manifest_hash: String,
    handoff_run_id: String,
    selected_candidate: Value,
}

#[derive(Debug, Serialize)]
struct RuntimeSceneApiData {
    ducklake_metadata_path: PathBuf,
    ducklake_data_path: PathBuf,
    package_url: Option<String>,
    manifest_url: Option<String>,
    mesh_lod_tag: String,
    mesh_base_url: String,
    mesh_url_pattern: String,
    scene: ModelReleaseSceneResponse,
}

async fn get_releases(Query(query): Query<ReleaseListQuery>) -> Response {
    let context = match version_context(query.project.as_deref()) {
        Ok(context) => context,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let filter_project = if query.all_projects.unwrap_or(false) {
        None
    } else {
        Some(context.project_name.clone())
    };
    let ducklake = context.ducklake.clone();
    let result =
        run_blocking(move || list_model_releases(ducklake, filter_project.as_deref())).await;
    let mut response = match result {
        Ok(response) => response,
        Err(message) => return api_error(classify_error(&message), message),
    };

    if let Some(dbnum) = query.dbnum {
        response.releases.retain(|release| release.dbnum == dbnum);
    }
    let quality_filter = match normalize_release_quality_filter(
        query.quality.as_deref(),
        query.complete_visual_only.unwrap_or(false),
    ) {
        Ok(value) => value,
        Err(message) => return api_error(StatusCode::BAD_REQUEST, message),
    };
    if let Some(quality) = quality_filter {
        response
            .releases
            .retain(|release| release.release_quality.as_str() == quality);
    }

    let releases = response
        .releases
        .into_iter()
        .map(|release| release_view(release, &context.output_root))
        .collect();

    api_ok(
        "model releases loaded",
        ReleaseListApiData {
            project_name: response.project_name,
            ducklake_metadata_path: context.ducklake.metadata_path,
            ducklake_data_path: context.ducklake.data_path,
            releases,
        },
    )
}

async fn post_register_release(Json(request): Json<RegisterReleaseRequest>) -> Response {
    let context = match version_context(request.project.as_deref()) {
        Ok(context) => context,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let register_request = match build_register_release_request(request, &context) {
        Ok(request) => request,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let ducklake_metadata_path = context.ducklake.metadata_path.clone();
    let ducklake_data_path = context.ducklake.data_path.clone();
    let result = run_blocking(move || register_model_release(register_request)).await;

    match result {
        Ok(registration) => api_ok(
            "model release registered",
            RegisterReleaseApiData {
                ducklake_metadata_path,
                ducklake_data_path,
                registration,
            },
        ),
        Err(message) => api_error(classify_error(&message), message),
    }
}

async fn post_publish_history_release(
    Json(request): Json<PublishHistoryReleaseRequest>,
) -> Response {
    let context = match version_context(request.project.as_deref()) {
        Ok(context) => context,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let publish_request = match build_publish_history_release_request(request, &context) {
        Ok(request) => request,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let ducklake_metadata_path = context.ducklake.metadata_path.clone();
    let ducklake_data_path = context.ducklake.data_path.clone();
    let result = run_blocking(move || publish_history_model_release(publish_request)).await;

    match result {
        Ok(publish) => api_ok(
            "history model release published",
            PublishHistoryReleaseApiData {
                ducklake_metadata_path,
                ducklake_data_path,
                publish,
            },
        ),
        Err(message) => api_error(classify_error(&message), message),
    }
}

async fn post_incremental_handoff(Json(request): Json<IncrementalHandoffRequest>) -> Response {
    let context = match version_context(request.project.as_deref()) {
        Ok(context) => context,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let plan = match build_incremental_handoff_register_request(request, &context) {
        Ok(plan) => plan,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let IncrementalHandoffRegisterPlan {
        register_request,
        handoff_manifest_path,
        handoff_manifest_hash,
        handoff_run_id,
        selected_candidate,
    } = plan;
    let ducklake_metadata_path = context.ducklake.metadata_path.clone();
    let ducklake_data_path = context.ducklake.data_path.clone();
    let result = run_blocking(move || register_model_release(register_request)).await;

    match result {
        Ok(registration) => api_ok(
            "incremental handoff registered as staged release",
            IncrementalHandoffApiData {
                ducklake_metadata_path,
                ducklake_data_path,
                handoff_manifest_path,
                handoff_manifest_hash,
                handoff_run_id,
                selected_candidate,
                registration,
            },
        ),
        Err(message) => api_error(classify_error(&message), message),
    }
}

async fn post_start_run(Json(request): Json<StartRunRequest>) -> Response {
    let context = match version_context(request.project.as_deref()) {
        Ok(context) => context,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let run_request = match build_http_bounded_run_request(request, &context) {
        Ok(request) => request,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let run_id = run_request.run_id.clone();
    let state_dir = run_request.state_dir.clone();
    let thread_name = bounded_run_thread_name(&run_id);
    let spawn_result = thread::Builder::new().name(thread_name).spawn(move || {
        if let Err(error) = run_bounded_command(run_request) {
            eprintln!("[model-version-run] bounded run failed to start or complete: {error}");
        }
    });
    if let Err(error) = spawn_result {
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("spawn model-version runner thread failed: {error}"),
        );
    }

    let record = wait_for_bounded_run_record(&state_dir, &run_id).await;
    let launch_observed = record.is_some();
    api_ok(
        if launch_observed {
            "model-version run started"
        } else {
            "model-version run accepted; status file not observed yet"
        },
        ModelVersionRunApiData {
            state_dir,
            run_id,
            record,
            launch_observed,
        },
    )
}

async fn post_prepare_physical_snapshot_run(
    Json(request): Json<PreparePhysicalSnapshotRunRequest>,
) -> Response {
    let context = match version_context(request.project.as_deref()) {
        Ok(context) => context,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let prepared = match build_prepare_physical_snapshot_pipeline_run(request, &context) {
        Ok(value) => value,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };

    let run_id = prepared.run_request.run_id.clone();
    let state_dir = prepared.run_request.state_dir.clone();
    let kind = prepared.run_request.kind.clone();
    let command_argv = prepared.run_request.argv.clone();
    let snapshot_id = prepared.snapshot_id;
    let source_observation = prepared.source_observation;
    let source_observation_manifest_path = prepared.source_observation_manifest_path;
    let source_observation_manifest_hash = prepared.source_observation_manifest_hash;
    let baseline_state_manifest_path = prepared.baseline_state_manifest_path;
    let baseline_state_manifest_hash = prepared.baseline_state_manifest_hash;
    let history_replay = prepared.history_replay;
    let parse_run_id = prepared.parse_run_id;
    let parse_run_status = prepared.parse_run_status;
    let diagnostic_reason = prepared.diagnostic_reason;
    let thread_name = bounded_run_thread_name(&run_id);
    let run_request = prepared.run_request;
    let spawn_result = thread::Builder::new().name(thread_name).spawn(move || {
        if let Err(error) = run_bounded_command(run_request) {
            eprintln!("[model-version-run] prepare physical snapshot run failed: {error}");
        }
    });
    if let Err(error) = spawn_result {
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("spawn prepare physical snapshot runner thread failed: {error}"),
        );
    }

    let record = wait_for_bounded_run_record(&state_dir, &run_id).await;
    let launch_observed = record.is_some();
    api_ok(
        if launch_observed {
            "prepare physical snapshot run started"
        } else {
            "prepare physical snapshot run accepted; status file not observed yet"
        },
        ModelVersionPipelineRunApiData {
            state_dir,
            run_id,
            kind,
            command_argv,
            snapshot_id,
            source_observation_manifest_path,
            source_observation_manifest_hash,
            source_observation,
            baseline_state_manifest_path,
            baseline_state_manifest_hash,
            history_replay,
            parse_run_id,
            parse_run_status,
            diagnostic_reason,
            record,
            launch_observed,
        },
    )
}

async fn post_prepare_history_replay_run(
    Json(request): Json<PrepareHistoryReplayRunRequest>,
) -> Response {
    let context = match version_context(request.project.as_deref()) {
        Ok(context) => context,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let prepared = match build_prepare_history_replay_pipeline_run(request, &context) {
        Ok(value) => value,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };

    let run_id = prepared.run_request.run_id.clone();
    let state_dir = prepared.run_request.state_dir.clone();
    let kind = prepared.run_request.kind.clone();
    let command_argv = prepared.run_request.argv.clone();
    let snapshot_id = prepared.snapshot_id;
    let source_observation = prepared.source_observation;
    let source_observation_manifest_path = prepared.source_observation_manifest_path;
    let source_observation_manifest_hash = prepared.source_observation_manifest_hash;
    let baseline_state_manifest_path = prepared.baseline_state_manifest_path;
    let baseline_state_manifest_hash = prepared.baseline_state_manifest_hash;
    let history_replay = prepared.history_replay;
    let parse_run_id = prepared.parse_run_id;
    let parse_run_status = prepared.parse_run_status;
    let diagnostic_reason = prepared.diagnostic_reason;
    let thread_name = bounded_run_thread_name(&run_id);
    let run_request = prepared.run_request;
    let spawn_result = thread::Builder::new().name(thread_name).spawn(move || {
        if let Err(error) = run_bounded_command(run_request) {
            eprintln!("[model-version-run] prepare history replay run failed: {error}");
        }
    });
    if let Err(error) = spawn_result {
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("spawn prepare history replay runner thread failed: {error}"),
        );
    }

    let record = wait_for_bounded_run_record(&state_dir, &run_id).await;
    let launch_observed = record.is_some();
    api_ok(
        if launch_observed {
            "prepare history replay run started"
        } else {
            "prepare history replay run accepted; status file not observed yet"
        },
        ModelVersionPipelineRunApiData {
            state_dir,
            run_id,
            kind,
            command_argv,
            snapshot_id,
            source_observation_manifest_path,
            source_observation_manifest_hash,
            source_observation,
            baseline_state_manifest_path,
            baseline_state_manifest_hash,
            history_replay,
            parse_run_id,
            parse_run_status,
            diagnostic_reason,
            record,
            launch_observed,
        },
    )
}

async fn post_execute_history_replay_plan_run(
    Json(request): Json<ExecuteHistoryReplayPlanRunRequest>,
) -> Response {
    let context = match version_context(request.project.as_deref()) {
        Ok(context) => context,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let prepared = match build_execute_history_replay_plan_run(request, &context) {
        Ok(value) => value,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };

    let run_id = prepared.run_request.run_id.clone();
    let state_dir = prepared.run_request.state_dir.clone();
    let kind = prepared.run_request.kind.clone();
    let phase = prepared.phase;
    let prepare_run_id = prepared.prepare_run_id;
    let prepare_record = prepared.prepare_record;
    let prepare_stdout_path = prepared.prepare_stdout_path;
    let prepare_stdout_hash = prepared.prepare_stdout_hash;
    let plan = prepared.plan;
    let command_argv = prepared.run_request.argv.clone();
    let source_db_file = prepared.source_db_file;
    let expected_source_db_sha256 = prepared.expected_source_db_sha256;
    let thread_name = bounded_run_thread_name(&run_id);
    let run_request = prepared.run_request;
    let spawn_result = thread::Builder::new().name(thread_name).spawn(move || {
        if let Err(error) = run_bounded_command(run_request) {
            eprintln!("[model-version-run] execute history replay plan run failed: {error}");
        }
    });
    if let Err(error) = spawn_result {
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("spawn execute history replay plan runner thread failed: {error}"),
        );
    }

    let record = wait_for_bounded_run_record(&state_dir, &run_id).await;
    let launch_observed = record.is_some();
    api_ok(
        if launch_observed {
            "history replay plan run started"
        } else {
            "history replay plan run accepted; status file not observed yet"
        },
        ExecuteHistoryReplayPlanRunApiData {
            state_dir,
            run_id,
            kind,
            phase,
            prepare_run_id,
            prepare_record,
            prepare_stdout_path,
            prepare_stdout_hash,
            plan,
            command_argv,
            source_db_file,
            expected_source_db_sha256,
            record,
            launch_observed,
        },
    )
}

async fn post_parse_baseline_run(Json(request): Json<ParseBaselineRunRequest>) -> Response {
    let context = match version_context(request.project.as_deref()) {
        Ok(context) => context,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let prepared = match build_parse_baseline_pipeline_run(request, &context) {
        Ok(value) => value,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };

    let run_id = prepared.run_request.run_id.clone();
    let state_dir = prepared.run_request.state_dir.clone();
    let kind = prepared.run_request.kind.clone();
    let command_argv = prepared.run_request.argv.clone();
    let snapshot_id = prepared.snapshot_id;
    let source_observation = prepared.source_observation;
    let source_observation_manifest_path = prepared.source_observation_manifest_path;
    let source_observation_manifest_hash = prepared.source_observation_manifest_hash;
    let baseline_state_manifest_path = prepared.baseline_state_manifest_path;
    let baseline_state_manifest_hash = prepared.baseline_state_manifest_hash;
    let history_replay = prepared.history_replay;
    let parse_run_id = prepared.parse_run_id;
    let parse_run_status = prepared.parse_run_status;
    let diagnostic_reason = prepared.diagnostic_reason;
    let thread_name = bounded_run_thread_name(&run_id);
    let run_request = prepared.run_request;
    let spawn_result = thread::Builder::new().name(thread_name).spawn(move || {
        if let Err(error) = run_bounded_command(run_request) {
            eprintln!("[model-version-run] parse baseline run failed: {error}");
        }
    });
    if let Err(error) = spawn_result {
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("spawn parse baseline runner thread failed: {error}"),
        );
    }

    let record = wait_for_bounded_run_record(&state_dir, &run_id).await;
    let launch_observed = record.is_some();
    api_ok(
        if launch_observed {
            "parse baseline run started"
        } else {
            "parse baseline run accepted; status file not observed yet"
        },
        ModelVersionPipelineRunApiData {
            state_dir,
            run_id,
            kind,
            command_argv,
            snapshot_id,
            source_observation_manifest_path,
            source_observation_manifest_hash,
            source_observation,
            baseline_state_manifest_path,
            baseline_state_manifest_hash,
            history_replay,
            parse_run_id,
            parse_run_status,
            diagnostic_reason,
            record,
            launch_observed,
        },
    )
}

async fn post_generate_full_model_run(
    Json(request): Json<GenerateFullModelRunRequest>,
) -> Response {
    let context = match version_context(request.project.as_deref()) {
        Ok(context) => context,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let prepared = match build_generate_full_model_pipeline_run(request, &context) {
        Ok(value) => value,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };

    let run_id = prepared.run_request.run_id.clone();
    let state_dir = prepared.run_request.state_dir.clone();
    let kind = prepared.run_request.kind.clone();
    let command_argv = prepared.run_request.argv.clone();
    let snapshot_id = prepared.snapshot_id;
    let source_observation = prepared.source_observation;
    let source_observation_manifest_path = prepared.source_observation_manifest_path;
    let source_observation_manifest_hash = prepared.source_observation_manifest_hash;
    let baseline_state_manifest_path = prepared.baseline_state_manifest_path;
    let baseline_state_manifest_hash = prepared.baseline_state_manifest_hash;
    let history_replay = prepared.history_replay;
    let parse_run_id = prepared.parse_run_id;
    let parse_run_status = prepared.parse_run_status;
    let diagnostic_reason = prepared.diagnostic_reason;
    let thread_name = bounded_run_thread_name(&run_id);
    let run_request = prepared.run_request;
    let spawn_result = thread::Builder::new().name(thread_name).spawn(move || {
        if let Err(error) = run_bounded_command(run_request) {
            eprintln!("[model-version-run] generate full model run failed: {error}");
        }
    });
    if let Err(error) = spawn_result {
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("spawn generate full model runner thread failed: {error}"),
        );
    }

    let record = wait_for_bounded_run_record(&state_dir, &run_id).await;
    let launch_observed = record.is_some();
    api_ok(
        if launch_observed {
            "generate full model run started"
        } else {
            "generate full model run accepted; status file not observed yet"
        },
        ModelVersionPipelineRunApiData {
            state_dir,
            run_id,
            kind,
            command_argv,
            snapshot_id,
            source_observation_manifest_path,
            source_observation_manifest_hash,
            source_observation,
            baseline_state_manifest_path,
            baseline_state_manifest_hash,
            history_replay,
            parse_run_id,
            parse_run_status,
            diagnostic_reason,
            record,
            launch_observed,
        },
    )
}

async fn get_run_status(
    AxumPath(run_id): AxumPath<String>,
    Query(query): Query<RunQuery>,
) -> Response {
    let state_dir = match runner_state_dir_from_query(query.project.as_deref(), query.state_dir) {
        Ok(state_dir) => state_dir,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    match read_bounded_run_status(&state_dir, run_id.trim()) {
        Ok(run) => api_ok(
            "model-version run status loaded",
            ModelVersionRunStatusApiData { state_dir, run },
        ),
        Err(error) => api_error(classify_error(&error.to_string()), error.to_string()),
    }
}

async fn post_cancel_run(
    AxumPath(run_id): AxumPath<String>,
    Query(query): Query<RunQuery>,
) -> Response {
    let state_dir =
        match runner_state_dir_from_query(query.project.as_deref(), query.state_dir.clone()) {
            Ok(state_dir) => state_dir,
            Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
        };
    match request_bounded_run_cancel(&state_dir, run_id.trim(), query.reason) {
        Ok(cancel) => api_ok(
            "model-version run cancellation requested",
            ModelVersionRunCancelApiData { state_dir, cancel },
        ),
        Err(error) => api_error(classify_error(&error.to_string()), error.to_string()),
    }
}

async fn get_release_detail(
    AxumPath(release_id): AxumPath<String>,
    Query(query): Query<ReleaseDetailQuery>,
) -> Response {
    let release_id = release_id.trim().to_string();
    if release_id.is_empty() {
        return api_error(StatusCode::BAD_REQUEST, "release_id is required");
    }

    let context = match version_context(query.project.as_deref()) {
        Ok(context) => context,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let ducklake = context.ducklake.clone();
    let result = run_blocking(move || {
        ModelVersionDuckLakeStore::open_readonly(ducklake)?.get_release(&release_id)
    })
    .await;
    let release = match result {
        Ok(release) => release,
        Err(message) => return api_error(classify_error(&message), message),
    };
    let manifest = match read_release_manifest(&release) {
        Ok(manifest) => manifest,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };

    api_ok(
        "model release loaded",
        ReleaseDetailApiData {
            release: release_view(release, &context.output_root),
            manifest,
        },
    )
}

async fn get_release_events(
    AxumPath(release_id): AxumPath<String>,
    Query(query): Query<ReleaseEventsQuery>,
) -> Response {
    let release_id = release_id.trim().to_string();
    if release_id.is_empty() {
        return api_error(StatusCode::BAD_REQUEST, "release_id is required");
    }

    let context = match version_context(query.project.as_deref()) {
        Ok(context) => context,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let ducklake = context.ducklake.clone();
    let result = run_blocking(move || get_model_release_events(ducklake, &release_id)).await;
    match result {
        Ok(events) => api_ok(
            "model release events loaded",
            ReleaseEventsApiData {
                ducklake_metadata_path: context.ducklake.metadata_path,
                ducklake_data_path: context.ducklake.data_path,
                events,
            },
        ),
        Err(message) => api_error(classify_error(&message), message),
    }
}

async fn post_reconcile_release(
    AxumPath(release_id): AxumPath<String>,
    Query(query): Query<ReconcileReleaseQuery>,
) -> Response {
    let release_id = release_id.trim().to_string();
    if release_id.is_empty() {
        return api_error(StatusCode::BAD_REQUEST, "release_id is required");
    }

    let context = match version_context(query.project.as_deref()) {
        Ok(context) => context,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let ducklake = context.ducklake.clone();
    let publish_if_complete = query.publish_if_complete.unwrap_or(false);
    let fail_if_unusable = query.fail_if_unusable.unwrap_or(false);
    let result = run_blocking(move || {
        reconcile_model_release(ducklake, &release_id, publish_if_complete, fail_if_unusable)
    })
    .await;
    match result {
        Ok(reconcile) => api_ok(
            "model release reconciled",
            ReconcileReleaseApiData {
                ducklake_metadata_path: context.ducklake.metadata_path,
                ducklake_data_path: context.ducklake.data_path,
                reconcile,
            },
        ),
        Err(message) => api_error(classify_error(&message), message),
    }
}

async fn post_release_state_machine(
    AxumPath(release_id): AxumPath<String>,
    Json(request): Json<ReleaseStateMachineRequest>,
) -> Response {
    let release_id = release_id.trim().to_string();
    if release_id.is_empty() {
        return api_error(StatusCode::BAD_REQUEST, "release_id is required");
    }
    let action = match ModelReleaseStateMachineAction::from_str(
        request.action.as_deref().unwrap_or("review"),
    ) {
        Ok(action) => action,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };

    let context = match version_context(request.project.as_deref()) {
        Ok(context) => context,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let state_request = ModelReleaseStateMachineRequest {
        ducklake: context.ducklake.clone(),
        release_id,
        action,
        reason: non_empty_string(request.reason),
        require_generation_job_id: request.require_generation_job_id.unwrap_or(true),
        require_baseline_state: request.require_baseline_state.unwrap_or(true),
        require_asset_manifest: request.require_asset_manifest.unwrap_or(true),
    };
    let result = run_blocking(move || run_model_release_state_machine(state_request)).await;
    match result {
        Ok(state_machine) => api_ok(
            "model release state-machine evaluated",
            ReleaseStateMachineApiData {
                ducklake_metadata_path: context.ducklake.metadata_path,
                ducklake_data_path: context.ducklake.data_path,
                state_machine,
            },
        ),
        Err(message) => api_error(classify_error(&message), message),
    }
}

async fn post_index_release(
    AxumPath(release_id): AxumPath<String>,
    Query(query): Query<IndexReleaseQuery>,
) -> Response {
    let release_id = release_id.trim().to_string();
    if release_id.is_empty() {
        return api_error(StatusCode::BAD_REQUEST, "release_id is required");
    }

    let context = match version_context(query.project.as_deref()) {
        Ok(context) => context,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let ducklake = context.ducklake.clone();
    let result = run_blocking(move || index_model_release_components(ducklake, &release_id)).await;
    match result {
        Ok(component_index) => api_ok(
            "release component index rebuilt",
            IndexReleaseApiData {
                ducklake_metadata_path: context.ducklake.metadata_path,
                ducklake_data_path: context.ducklake.data_path,
                component_index,
            },
        ),
        Err(message) => api_error(classify_error(&message), message),
    }
}

async fn post_index_release_units(
    AxumPath(release_id): AxumPath<String>,
    Query(query): Query<IndexReleaseQuery>,
) -> Response {
    let release_id = release_id.trim().to_string();
    if release_id.is_empty() {
        return api_error(StatusCode::BAD_REQUEST, "release_id is required");
    }

    let context = match version_context(query.project.as_deref()) {
        Ok(context) => context,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let ducklake = context.ducklake.clone();
    let result = run_blocking(move || index_model_release_units(ducklake, &release_id)).await;
    match result {
        Ok(unit_index) => api_ok(
            "release unit index rebuilt",
            IndexUnitApiData {
                ducklake_metadata_path: context.ducklake.metadata_path,
                ducklake_data_path: context.ducklake.data_path,
                unit_index,
            },
        ),
        Err(message) => api_error(classify_error(&message), message),
    }
}

async fn post_index_release_assets(
    AxumPath(release_id): AxumPath<String>,
    Query(query): Query<MeshAssetIndexQuery>,
) -> Response {
    let release_id = release_id.trim().to_string();
    if release_id.is_empty() {
        return api_error(StatusCode::BAD_REQUEST, "release_id is required");
    }

    let context = match version_context(query.project.as_deref()) {
        Ok(context) => context,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let ducklake = context.ducklake.clone();
    let mesh_root = context.mesh_root.clone();
    let mesh_base_url = query
        .mesh_base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let materialize = query.materialize.unwrap_or(false);
    let result = run_blocking(move || {
        index_model_release_mesh_assets(
            ducklake,
            &release_id,
            &mesh_root,
            mesh_base_url.as_deref(),
            materialize,
        )
    })
    .await;
    match result {
        Ok(mesh_asset_index) => api_ok(
            "release mesh asset index rebuilt",
            IndexMeshAssetsApiData {
                ducklake_metadata_path: context.ducklake.metadata_path,
                ducklake_data_path: context.ducklake.data_path,
                mesh_asset_index,
            },
        ),
        Err(message) => api_error(classify_error(&message), message),
    }
}

async fn get_release_mesh_assets(
    AxumPath(release_id): AxumPath<String>,
    Query(query): Query<MeshAssetListQuery>,
) -> Response {
    let release_id = release_id.trim().to_string();
    if release_id.is_empty() {
        return api_error(StatusCode::BAD_REQUEST, "release_id is required");
    }

    let context = match version_context(query.project.as_deref()) {
        Ok(context) => context,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let ducklake = context.ducklake.clone();
    let limit = query
        .limit
        .unwrap_or(DEFAULT_DIFF_LIMIT)
        .clamp(1, MAX_DIFF_LIMIT);
    let missing_only = query.missing_only.unwrap_or(false);
    let result = run_blocking(move || {
        get_model_release_mesh_assets(ducklake, &release_id, limit, missing_only)
    })
    .await;
    match result {
        Ok(mesh_assets) => api_ok(
            "release mesh assets loaded",
            MeshAssetsApiData {
                ducklake_metadata_path: context.ducklake.metadata_path,
                ducklake_data_path: context.ducklake.data_path,
                mesh_assets,
            },
        ),
        Err(message) => api_error(classify_error(&message), message),
    }
}

async fn get_release_runtime_scene(
    AxumPath(release_id): AxumPath<String>,
    Query(query): Query<RuntimeSceneQuery>,
) -> Response {
    let release_id = release_id.trim().to_string();
    if release_id.is_empty() {
        return api_error(StatusCode::BAD_REQUEST, "release_id is required");
    }
    let limit = query
        .limit
        .unwrap_or(DEFAULT_SCENE_LIMIT)
        .clamp(1, MAX_SCENE_LIMIT);
    let offset = query.offset.unwrap_or(0);
    let component_key = query
        .component_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let context = match version_context(query.project.as_deref()) {
        Ok(context) => context,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let ducklake = context.ducklake.clone();
    let result = run_blocking(move || {
        get_model_release_scene(
            ducklake,
            &release_id,
            limit,
            offset,
            component_key.as_deref(),
        )
    })
    .await;
    let scene = match result {
        Ok(scene) => scene,
        Err(message) => return api_error(classify_error(&message), message),
    };

    let manifest = match read_release_manifest(&scene.release) {
        Ok(manifest) => manifest,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let package_url = output_file_url(&scene.release.immutable_package_dir, &context.output_root);
    let manifest_url = package_url
        .as_ref()
        .map(|base| format!("{}/manifest.json", base.trim_end_matches('/')));
    let lod_tag = manifest_mesh_lod_tag(&manifest);
    let mesh_base_url = match release_local_mesh_base_url(
        &scene.release,
        &lod_tag,
        &context.output_root,
    ) {
        Some(value) => value,
        None => {
            return api_error(
                StatusCode::FAILED_DEPENDENCY,
                format!(
                    "missing dependency: release-local mesh directory is missing for published release '{}' lod {}; rerun index-assets with materialize=true",
                    scene.release.release_id, lod_tag
                ),
            );
        }
    };

    api_ok(
        "model release runtime scene loaded",
        RuntimeSceneApiData {
            ducklake_metadata_path: context.ducklake.metadata_path,
            ducklake_data_path: context.ducklake.data_path,
            package_url,
            manifest_url,
            mesh_lod_tag: lod_tag.clone(),
            mesh_base_url: mesh_base_url.clone(),
            mesh_url_pattern: format!("{mesh_base_url}/{{geo_hash}}_{lod_tag}.glb"),
            scene,
        },
    )
}

async fn get_compare_readiness(Query(query): Query<DiffQuery>) -> Response {
    let from_release_id = query.from_release_id.trim().to_string();
    let to_release_id = query.to_release_id.trim().to_string();
    if from_release_id.is_empty() || to_release_id.is_empty() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "from_release_id and to_release_id are required",
        );
    }

    let context = match version_context(query.project.as_deref()) {
        Ok(context) => context,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let ducklake = context.ducklake.clone();
    let result = run_blocking(move || {
        validate_model_release_pair_readiness(ducklake, &from_release_id, &to_release_id)
    })
    .await;

    match result {
        Ok(readiness) => api_ok(
            "model release compare readiness loaded",
            CompareReadinessApiData {
                ducklake_metadata_path: context.ducklake.metadata_path,
                ducklake_data_path: context.ducklake.data_path,
                readiness,
            },
        ),
        Err(message) => api_error(classify_error(&message), message),
    }
}

async fn get_history_baseline_inspect(
    Query(query): Query<HistoryBaselineInspectQuery>,
) -> Response {
    let project_name = match version_context(query.project.as_deref()) {
        Ok(context) => context.project_name,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let source_db_file = query.source_db_file;
    if !source_db_file.is_file() {
        return api_error(
            StatusCode::NOT_FOUND,
            format!(
                "source DB file is missing or not a file: {}",
                source_db_file.display()
            ),
        );
    }
    let parse_sample_limit = query
        .parse_sample_limit
        .unwrap_or(DEFAULT_HISTORY_BASELINE_SAMPLE_LIMIT)
        .min(MAX_HISTORY_BASELINE_SAMPLE_LIMIT);
    let target_sesno = query.target_sesno;
    let request = HistoryBaselineInspectRequest {
        project_name: project_name.clone(),
        source_db_file: source_db_file.clone(),
        target_sesno,
        parse_sample_limit,
        require_exact_sesno: !query.allow_nearest_sesno.unwrap_or(false),
        detail: query.detail.unwrap_or(false),
    };
    let result = run_history_baseline_inspect_blocking(request).await;

    match result {
        Ok(inspect) => api_ok(
            "history baseline inspection loaded",
            HistoryBaselineInspectApiData {
                project_name,
                source_db_file,
                target_sesno,
                inspect,
            },
        ),
        Err(message) => api_error(classify_error(&message), message),
    }
}

async fn get_release_diff(Query(query): Query<DiffQuery>) -> Response {
    let from_release_id = query.from_release_id.trim().to_string();
    let to_release_id = query.to_release_id.trim().to_string();
    if from_release_id.is_empty() || to_release_id.is_empty() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "from_release_id and to_release_id are required",
        );
    }
    let change_type = match normalize_change_type(query.change_type.as_deref()) {
        Ok(value) => value,
        Err(message) => return api_error(StatusCode::BAD_REQUEST, message),
    };
    let limit = query
        .limit
        .unwrap_or(DEFAULT_DIFF_LIMIT)
        .clamp(1, MAX_DIFF_LIMIT);

    let context = match version_context(query.project.as_deref()) {
        Ok(context) => context,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let ducklake = context.ducklake.clone();
    let result = run_blocking(move || {
        diff_model_releases(
            ducklake,
            &from_release_id,
            &to_release_id,
            limit,
            change_type.as_deref(),
        )
    })
    .await;

    match result {
        Ok(diff) => api_ok(
            "model release diff loaded",
            DiffApiData {
                ducklake_metadata_path: context.ducklake.metadata_path,
                ducklake_data_path: context.ducklake.data_path,
                diff,
            },
        ),
        Err(message) => api_error(classify_error(&message), message),
    }
}

async fn get_unit_diff(Query(query): Query<UnitDiffQuery>) -> Response {
    let from_release_id = query.from_release_id.trim().to_string();
    let to_release_id = query.to_release_id.trim().to_string();
    if from_release_id.is_empty() || to_release_id.is_empty() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "from_release_id and to_release_id are required",
        );
    }
    let limit = query
        .limit
        .unwrap_or(DEFAULT_DIFF_LIMIT)
        .clamp(1, MAX_DIFF_LIMIT);

    let context = match version_context(query.project.as_deref()) {
        Ok(context) => context,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let ducklake = context.ducklake.clone();
    let unit_noun = query.unit_noun.clone();
    let result = run_blocking(move || {
        diff_model_release_units(
            ducklake,
            &from_release_id,
            &to_release_id,
            limit,
            unit_noun.as_deref(),
        )
    })
    .await;

    match result {
        Ok(unit_diff) => api_ok(
            "model release unit diff loaded",
            UnitDiffApiData {
                ducklake_metadata_path: context.ducklake.metadata_path,
                ducklake_data_path: context.ducklake.data_path,
                unit_diff,
            },
        ),
        Err(message) => api_error(classify_error(&message), message),
    }
}

async fn get_component_unit_impact(Query(query): Query<ComponentImpactQuery>) -> Response {
    let from_release_id = query.from_release_id.trim().to_string();
    let to_release_id = query.to_release_id.trim().to_string();
    if from_release_id.is_empty() || to_release_id.is_empty() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "from_release_id and to_release_id are required",
        );
    }
    let limit = query
        .limit
        .unwrap_or(DEFAULT_DIFF_LIMIT)
        .clamp(1, MAX_DIFF_LIMIT);
    let component_key =
        match component_key_filter(query.component_key.as_deref(), query.refno_u64, query.dbnum) {
            Ok(value) => value,
            Err(message) => return api_error(StatusCode::BAD_REQUEST, message),
        };

    let context = match version_context(query.project.as_deref()) {
        Ok(context) => context,
        Err(error) => return api_error(classify_error(&error.to_string()), error.to_string()),
    };
    let ducklake = context.ducklake.clone();
    let result = run_blocking(move || {
        get_model_component_unit_impacts(
            ducklake,
            &from_release_id,
            &to_release_id,
            limit,
            component_key.as_deref(),
        )
    })
    .await;

    match result {
        Ok(impact) => api_ok(
            "model release component unit impact loaded",
            ComponentImpactApiData {
                ducklake_metadata_path: context.ducklake.metadata_path,
                ducklake_data_path: context.ducklake.data_path,
                impact,
            },
        ),
        Err(message) => api_error(classify_error(&message), message),
    }
}

async fn compare_page() -> Html<String> {
    Html(COMPARE_PAGE_HTML.to_string())
}

async fn release_viewer_page() -> Html<String> {
    Html(RELEASE_VIEWER_PAGE_HTML.to_string())
}

async fn run_blocking<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| format!("model-version worker join failed: {error}"))?
        .map_err(|error| error.to_string())
}

async fn run_history_baseline_inspect_blocking(
    request: HistoryBaselineInspectRequest,
) -> Result<HistoryBaselineInspectResponse, String> {
    tokio::task::spawn_blocking(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("create history baseline inspect runtime")?;
        runtime.block_on(inspect_history_baseline(request))
    })
    .await
    .map_err(|error| format!("model-version worker join failed: {error}"))?
    .map_err(|error| error.to_string())
}

fn build_register_release_request(
    request: RegisterReleaseRequest,
    context: &VersionContext,
) -> anyhow::Result<ModelReleaseRegisterRequest> {
    let release_id = request.release_id.trim().to_string();
    validate_http_run_id(&release_id)?;
    let project_output_dir = context.output_root.join(&context.project_name);
    let project_model_versions_root = project_output_dir.join("model_versions");
    let source_parquet_dir = request.parquet_dir.unwrap_or_else(|| {
        project_output_dir
            .join("parquet")
            .join(request.dbnum.to_string())
    });
    let release_root = request
        .release_root
        .unwrap_or_else(|| project_model_versions_root.join("releases"));
    ensure_path_under(&context.output_root, &source_parquet_dir, "parquet_dir")?;
    ensure_path_under(&project_model_versions_root, &release_root, "release_root")?;

    Ok(ModelReleaseRegisterRequest {
        project_name: context.project_name.clone(),
        release_id,
        release_label: non_empty_string(request.release_label),
        release_quality: parse_optional_release_quality(request.release_quality.as_deref())?,
        release_quality_reason: non_empty_string(request.release_quality_reason),
        validation_flags: normalize_string_list(request.validation_flags.unwrap_or_default()),
        spec_info_fallback_count: request.spec_info_fallback_count,
        branch_id: non_empty_string(request.branch_id).unwrap_or_else(|| "main".to_string()),
        parent_release_id: non_empty_string(request.parent_release_id),
        derivation_type: non_empty_string(request.derivation_type)
            .unwrap_or_else(|| "manual-http-register".to_string()),
        dbnum: request.dbnum,
        source_parquet_dir,
        release_root,
        ducklake: context.ducklake.clone(),
        extra_metadata: metadata_json_object(request.metadata_json)?,
        initial_status: ModelReleaseStatus::Staged,
        index_units: request.index_units.unwrap_or(false),
        export_sesno: request.sesno,
    })
}

fn build_publish_history_release_request(
    request: PublishHistoryReleaseRequest,
    context: &VersionContext,
) -> anyhow::Result<ModelHistoryReleasePublishRequest> {
    let release_id = request.release_id.trim().to_string();
    validate_http_run_id(&release_id)?;
    if request.from_sesno >= request.to_sesno {
        anyhow::bail!(
            "invalid sesno range for publish-history: from_sesno={} must be less than to_sesno={}",
            request.from_sesno,
            request.to_sesno
        );
    }

    let project_output_dir = context.output_root.join(&context.project_name);
    let project_model_versions_root = project_output_dir.join("model_versions");
    let current_parquet_dir = request.current_parquet_dir.unwrap_or_else(|| {
        project_output_dir
            .join("parquet")
            .join(request.dbnum.to_string())
    });
    let release_root = request
        .release_root
        .unwrap_or_else(|| project_model_versions_root.join("releases"));
    ensure_path_under(&context.output_root, &request.parquet_dir, "parquet_dir")?;
    ensure_path_under(
        &context.output_root,
        &current_parquet_dir,
        "current_parquet_dir",
    )?;
    ensure_path_under(&project_model_versions_root, &release_root, "release_root")?;
    if let Some(scene_tree_dir) = request.scene_tree_dir.as_ref() {
        ensure_path_under(&context.output_root, scene_tree_dir, "scene_tree_dir")?;
    }

    let materialize_assets = request.materialize_assets.unwrap_or(false);
    let mesh_root = if materialize_assets || request.mesh_root.is_some() {
        Some(
            request
                .mesh_root
                .unwrap_or_else(|| context.mesh_root.clone()),
        )
    } else {
        None
    };

    Ok(ModelHistoryReleasePublishRequest {
        project_name: context.project_name.clone(),
        release_id,
        release_label: non_empty_string(request.release_label),
        release_quality: parse_optional_release_quality(request.release_quality.as_deref())?,
        release_quality_reason: non_empty_string(request.release_quality_reason),
        validation_flags: normalize_string_list(request.validation_flags.unwrap_or_default()),
        spec_info_fallback_count: request.spec_info_fallback_count,
        branch_id: non_empty_string(request.branch_id).unwrap_or_else(|| "main".to_string()),
        parent_release_id: non_empty_string(request.parent_release_id),
        dbnum: request.dbnum,
        source_db_file: request.source_db_file,
        from_sesno: request.from_sesno,
        to_sesno: request.to_sesno,
        source_parquet_dir: request.parquet_dir,
        current_parquet_dir,
        scene_tree_dir: request.scene_tree_dir,
        require_scene_tree: request.require_scene_tree.unwrap_or(false),
        release_root,
        ducklake: context.ducklake.clone(),
        extra_metadata: metadata_json_object(request.metadata_json)?,
        mesh_root,
        mesh_base_url: non_empty_string(request.mesh_base_url),
        materialize_assets,
        index_units: request.index_units.unwrap_or(false),
    })
}

fn build_incremental_handoff_register_request(
    request: IncrementalHandoffRequest,
    context: &VersionContext,
) -> anyhow::Result<IncrementalHandoffRegisterPlan> {
    let cwd = std::env::current_dir().context("resolve current directory failed")?;
    let handoff_manifest_path = absolute_lexical_path(&request.handoff_manifest_path)?;
    ensure_path_under(&cwd, &handoff_manifest_path, "handoff_manifest_path")?;
    if !handoff_manifest_path.is_file() {
        anyhow::bail!(
            "handoff_manifest_path is missing or not a file: {}",
            handoff_manifest_path.display()
        );
    }
    let handoff_manifest_hash = sha256_file(&handoff_manifest_path)?;
    let manifest_bytes = fs::read(&handoff_manifest_path).with_context(|| {
        format!(
            "read incremental handoff manifest failed: {}",
            handoff_manifest_path.display()
        )
    })?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes).with_context(|| {
        format!(
            "parse incremental handoff manifest JSON failed: {}",
            handoff_manifest_path.display()
        )
    })?;

    let manifest_version = required_json_string_at(&manifest, &["manifest_version"])?;
    if manifest_version != "incremental_publication_handoff:v1" {
        anyhow::bail!(
            "unsupported incremental handoff manifest_version '{}'; expected incremental_publication_handoff:v1",
            manifest_version
        );
    }
    let manifest_project_name = required_json_string_at(&manifest, &["project_name"])?;
    if manifest_project_name != context.project_name {
        anyhow::bail!(
            "incremental handoff project_name mismatch: manifest={} request_context={}",
            manifest_project_name,
            context.project_name
        );
    }
    let policy = required_json_string_at(&manifest, &["policy"])?;
    if policy != "explicit_register_required" {
        anyhow::bail!(
            "incremental handoff policy must be explicit_register_required, got '{}'",
            policy
        );
    }
    if !required_json_bool_at(&manifest, &["generation_success"])? {
        anyhow::bail!("incremental handoff generation_success must be true");
    }
    let from_sesno = required_json_u32_at(&manifest, &["from_sesno"])?;
    let to_sesno = required_json_u32_at(&manifest, &["to_sesno"])?;
    if from_sesno >= to_sesno {
        anyhow::bail!(
            "invalid incremental handoff sesno range: from_sesno={} must be less than to_sesno={}",
            from_sesno,
            to_sesno
        );
    }
    let handoff_run_id = required_json_string_at(&manifest, &["run_id"])?.to_string();
    let candidates = manifest
        .get("candidates")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("incremental handoff manifest missing candidates array"))?;
    let selected_index =
        select_handoff_candidate_index(candidates, request.candidate_index, request.dbnum)?;
    let selected_candidate = candidates
        .get(selected_index)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("selected candidate index disappeared"))?;

    let dbnum = required_json_u32_at(&selected_candidate, &["dbnum"])?;
    if let Some(requested_dbnum) = request.dbnum {
        if requested_dbnum != dbnum {
            anyhow::bail!(
                "incremental handoff dbnum mismatch: request={} candidate={}",
                requested_dbnum,
                dbnum
            );
        }
    }
    let source_parquet_dir_raw =
        required_json_string_at(&selected_candidate, &["source_parquet_dir"])?;
    let source_parquet_dir = PathBuf::from(source_parquet_dir_raw);
    ensure_path_under(
        &context.output_root,
        &source_parquet_dir,
        "candidate.source_parquet_dir",
    )?;
    let package = load_model_package(&source_parquet_dir, dbnum).with_context(|| {
        format!(
            "load incremental handoff package failed: {}",
            source_parquet_dir.display()
        )
    })?;
    let candidate_package_hash = required_json_string_at(&selected_candidate, &["package_hash"])?;
    if !package
        .package_hash
        .eq_ignore_ascii_case(candidate_package_hash)
    {
        anyhow::bail!(
            "incremental handoff candidate package_hash mismatch: manifest={} actual={}",
            candidate_package_hash,
            package.package_hash
        );
    }
    validate_candidate_rows_by_table(&selected_candidate, &package.rows_by_table)?;

    let release_id = non_empty_string(request.release_id)
        .or_else(|| {
            json_string_at(&selected_candidate, &["suggested_release_id"]).map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| {
            suggested_incremental_handoff_release_id(dbnum, to_sesno, &package.package_hash)
        });
    validate_http_run_id(&release_id)?;

    let release_quality_raw = non_empty_string(request.release_quality)
        .or_else(|| {
            json_string_at(&selected_candidate, &["suggested_release_quality"])
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "patch_only".to_string());
    let release_quality = parse_optional_release_quality(Some(&release_quality_raw))?
        .unwrap_or(ModelReleaseQuality::PatchOnly);
    if matches!(release_quality, ModelReleaseQuality::CompleteVisual) {
        anyhow::bail!(
            "incremental handoff cannot register complete_visual; use patch_only, quarantined_visual, degraded_visual, or non_visual until full baseline evidence is proven"
        );
    }

    let release_quality_reason = non_empty_string(request.release_quality_reason)
        .or_else(|| json_string_at(&selected_candidate, &["next_step"]).map(ToOwned::to_owned))
        .or_else(|| {
            Some(
                "incremental handoff contains an affected-scope package; verify or hydrate a full baseline package before complete visual publication"
                    .to_string(),
            )
        });

    let mut validation_flags = normalize_string_list(request.validation_flags.unwrap_or_default());
    push_unique_validation_flag(&mut validation_flags, "incremental_handoff_affected_scope");
    push_unique_validation_flag(
        &mut validation_flags,
        "explicit_release_registration_required",
    );
    push_unique_validation_flag(&mut validation_flags, "http_incremental_handoff_reviewed");

    let user_metadata = metadata_json_object(request.metadata_json)?;
    let project_output_dir = context.output_root.join(&context.project_name);
    let project_model_versions_root = project_output_dir.join("model_versions");
    let release_root = project_model_versions_root.join("releases");
    ensure_path_under(&project_model_versions_root, &release_root, "release_root")?;

    let extra_metadata = json!({
        "source": "http incremental handoff",
        "project_name": context.project_name.clone(),
        "dbnum": dbnum,
        "from_sesno": from_sesno,
        "to_sesno": to_sesno,
        "release_quality": release_quality.as_str(),
        "release_quality_reason": release_quality_reason.clone(),
        "validation_flags": validation_flags.clone(),
        "generation_job_id": handoff_run_id.clone(),
        "candidate_package": {
            "source_parquet_dir": source_parquet_dir.clone(),
            "package_hash": package.package_hash.clone(),
            "rows_by_table": package.rows_by_table.clone(),
        },
        "incremental_handoff": {
            "manifest_version": manifest_version,
            "manifest_path": handoff_manifest_path.clone(),
            "manifest_hash": handoff_manifest_hash.clone(),
            "run_id": handoff_run_id.clone(),
            "policy": policy,
            "candidate_index": selected_index,
            "candidate": selected_candidate.clone(),
            "source_observation": manifest.get("source_observation").cloned().unwrap_or(Value::Null),
            "tree_index": manifest.get("tree_index").cloned().unwrap_or(Value::Null),
            "parquet_export": manifest.get("parquet_export").cloned().unwrap_or(Value::Null),
        },
        "publication_policy": {
            "release_registration_is_explicit": true,
            "incremental_sesno_does_not_write_ducklake_release_catalog": true,
            "register_copies_mutable_parquet_to_immutable_release_package": true,
            "suggested_release_quality": release_quality.as_str(),
        },
        "user_metadata": user_metadata,
    });

    Ok(IncrementalHandoffRegisterPlan {
        register_request: ModelReleaseRegisterRequest {
            project_name: context.project_name.clone(),
            release_id,
            release_label: non_empty_string(request.release_label),
            release_quality: Some(release_quality),
            release_quality_reason,
            validation_flags,
            spec_info_fallback_count: None,
            branch_id: non_empty_string(request.branch_id).unwrap_or_else(|| "main".to_string()),
            parent_release_id: non_empty_string(request.parent_release_id),
            derivation_type: "incremental-sesno-handoff".to_string(),
            dbnum,
            source_parquet_dir,
            release_root,
            ducklake: context.ducklake.clone(),
            extra_metadata,
            initial_status: ModelReleaseStatus::Staged,
            index_units: request.index_units.unwrap_or(true),
            export_sesno: Some(to_sesno),
        },
        handoff_manifest_path,
        handoff_manifest_hash,
        handoff_run_id,
        selected_candidate,
    })
}

fn build_http_bounded_run_request(
    request: StartRunRequest,
    context: &VersionContext,
) -> anyhow::Result<BoundedCommandRunRequest> {
    let run_id = request.run_id.trim().to_string();
    validate_http_run_id(&run_id)?;
    if request.argv.is_empty() {
        anyhow::bail!("argv must not be empty");
    }
    if request.argv.iter().any(|item| item.trim().is_empty()) {
        anyhow::bail!("argv must not contain empty arguments");
    }
    let timeout_secs = request.timeout_secs.unwrap_or(14_400);
    if timeout_secs == 0 {
        anyhow::bail!("timeout_secs must be greater than 0");
    }
    let poll_interval_ms = request.poll_interval_ms.unwrap_or(1_000);
    if poll_interval_ms == 0 {
        anyhow::bail!("poll_interval_ms must be greater than 0");
    }

    let cwd = request
        .cwd
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    if !cwd.is_dir() {
        anyhow::bail!("cwd is not a directory: {}", cwd.display());
    }
    let executable = resolve_http_aios_database_executable(request.executable.as_ref())?;
    let state_dir = request
        .state_dir
        .unwrap_or_else(|| default_runner_state_dir(context));

    let mut env = request.env.unwrap_or_default();
    if env.keys().any(|key| key.trim().is_empty()) {
        anyhow::bail!("env keys must not be empty");
    }
    if let Some(metrics_path) = &request.metrics_path {
        env.entry("AIOS_TASK_METRICS_PATH".to_string())
            .or_insert_with(|| metrics_path.to_string_lossy().to_string());
    }
    if let Some(kind) = request
        .kind
        .as_deref()
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
    {
        env.entry("AIOS_TASK_METRICS_KIND".to_string())
            .or_insert_with(|| kind.to_string());
    }

    Ok(BoundedCommandRunRequest {
        run_id,
        kind: request
            .kind
            .map(|kind| kind.trim().to_string())
            .filter(|kind| !kind.is_empty())
            .unwrap_or_else(|| "generic".to_string()),
        state_dir,
        executable: Some(executable),
        argv: request.argv,
        cwd,
        env,
        stdout_path: request.stdout_path,
        stderr_path: request.stderr_path,
        metrics_path: request.metrics_path,
        timeout_secs,
        stale_heartbeat_secs: request.stale_heartbeat_secs,
        source_db_file: request.source_db_file,
        expected_source_db_sha256: request
            .source_db_sha256
            .map(|hash| hash.trim().to_string())
            .filter(|hash| !hash.is_empty()),
        poll_interval_ms,
        force: request.force.unwrap_or(false),
    })
}

fn build_prepare_physical_snapshot_pipeline_run(
    request: PreparePhysicalSnapshotRunRequest,
    context: &VersionContext,
) -> anyhow::Result<PreparedPipelineRun> {
    let run_id = request.run_id.trim().to_string();
    validate_http_run_id(&run_id)?;
    let snapshot_id = request
        .snapshot_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| run_id.clone());
    validate_http_run_id(&snapshot_id)?;

    let timeout_secs = request.timeout_secs.unwrap_or(3_600);
    if timeout_secs == 0 {
        anyhow::bail!("timeout_secs must be greater than 0");
    }
    let poll_interval_ms = request.poll_interval_ms.unwrap_or(1_000);
    if poll_interval_ms == 0 {
        anyhow::bail!("poll_interval_ms must be greater than 0");
    }
    let force = request.force.unwrap_or(false);
    let state_dir = default_runner_state_dir(context);
    let project_model_version_root = context
        .output_root
        .join(&context.project_name)
        .join("model_versions");
    let physical_snapshot_root = project_model_version_root
        .join("physical_baselines")
        .join(&snapshot_id);
    let config_arg = physical_snapshot_root.join("DbOption-physical-baseline");
    let output_root = physical_snapshot_root.join("output");
    let run_dir = state_dir.join(&run_id);
    let metrics_path = run_dir.join("task-metrics.json");
    let source_observation_manifest_path = state_dir
        .join("_source_observations")
        .join(&run_id)
        .join("source_observation_manifest.json");

    ensure_path_under(
        &project_model_version_root,
        &physical_snapshot_root,
        "physical_snapshot_root",
    )?;
    ensure_path_under(&project_model_version_root, &config_arg, "config_arg")?;
    ensure_path_under(&project_model_version_root, &output_root, "output_root")?;
    ensure_path_under(&state_dir, &run_dir, "run_dir")?;
    ensure_path_under(&run_dir, &metrics_path, "metrics_path")?;
    ensure_path_under(
        &state_dir,
        &source_observation_manifest_path,
        "source_observation_manifest_path",
    )?;

    if run_dir.exists() && !force {
        anyhow::bail!(
            "run directory already exists for '{}'; pass force=true to overwrite: {}",
            run_id,
            run_dir.display()
        );
    }
    if source_observation_manifest_path.exists() && !force {
        anyhow::bail!(
            "source observation manifest already exists for '{}'; pass force=true to overwrite: {}",
            run_id,
            source_observation_manifest_path.display()
        );
    }

    let executable = resolve_http_aios_database_executable(request.executable.as_ref())?;
    let source_observation = build_source_observation_manifest(SourceObservationBuildRequest {
        observation_id: run_id.clone(),
        project_name: context.project_name.clone(),
        dbnum: request.dbnum,
        primary_file: request.source_db_file.clone(),
        dependency_files: request.dependency_files.unwrap_or_default(),
        requested_sesno: request
            .requested_sesno
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| Some("physical-current".to_string())),
        resolved_sesno: request.resolved_sesno,
        quiescence_window_ms: request.quiescence_window_ms.unwrap_or(0),
    })?;
    if !source_observation.quiescence.stable {
        anyhow::bail!(
            "source DB file changed during observation window; retry after the E3D database is stable: {}",
            source_observation.primary.path.display()
        );
    }
    let source_observation_manifest_hash =
        write_source_observation_manifest(&source_observation_manifest_path, &source_observation)?;

    let base_config_arg = request
        .base_config_arg
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(strip_toml_suffix)
        .unwrap_or_else(default_db_option_file_arg);
    let baseline_dbnums = normalize_dbnum_list(request.baseline_dbnums.unwrap_or_default());
    let mut argv = vec![
        "aios-database".to_string(),
        "-c".to_string(),
        base_config_arg.clone(),
        "model-version".to_string(),
        "prepare-physical-baseline-snapshot".to_string(),
        "--snapshot-id".to_string(),
        snapshot_id.clone(),
        "--project".to_string(),
        context.project_name.clone(),
        "--dbnum".to_string(),
        request.dbnum.to_string(),
        "--source-db-file".to_string(),
        source_observation
            .primary
            .path
            .to_string_lossy()
            .to_string(),
        "--base-config".to_string(),
        base_config_arg,
        "--config-out".to_string(),
        config_arg.to_string_lossy().to_string(),
        "--snapshot-root".to_string(),
        physical_snapshot_root.to_string_lossy().to_string(),
        "--output-root".to_string(),
        output_root.to_string_lossy().to_string(),
        "--json".to_string(),
    ];
    for dbnum in baseline_dbnums {
        argv.push("--baseline-dbnum".to_string());
        argv.push(dbnum.to_string());
    }
    if request.copy_files.unwrap_or(false) {
        argv.push("--copy-files".to_string());
    }
    if force {
        argv.push("--force".to_string());
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut env = BTreeMap::new();
    env.insert(
        "AIOS_TASK_METRICS_KIND".to_string(),
        "prepare_physical_snapshot".to_string(),
    );
    env.insert(
        "AIOS_TASK_METRICS_PATH".to_string(),
        metrics_path.to_string_lossy().to_string(),
    );
    env.insert(
        "AIOS_SOURCE_OBSERVATION_MANIFEST".to_string(),
        source_observation_manifest_path
            .to_string_lossy()
            .to_string(),
    );
    env.insert(
        "AIOS_SOURCE_OBSERVATION_MANIFEST_SHA256".to_string(),
        source_observation_manifest_hash.clone(),
    );

    Ok(PreparedPipelineRun {
        run_request: BoundedCommandRunRequest {
            run_id,
            kind: "prepare_physical_snapshot".to_string(),
            state_dir,
            executable: Some(executable),
            argv,
            cwd,
            env,
            stdout_path: None,
            stderr_path: None,
            metrics_path: Some(metrics_path),
            timeout_secs,
            stale_heartbeat_secs: request.stale_heartbeat_secs,
            source_db_file: Some(source_observation.primary.path.clone()),
            expected_source_db_sha256: Some(source_observation.primary.sha256.clone()),
            poll_interval_ms,
            force,
        },
        snapshot_id: Some(snapshot_id),
        source_observation_manifest_path,
        source_observation_manifest_hash,
        source_observation,
        baseline_state_manifest_path: None,
        baseline_state_manifest_hash: None,
        history_replay: None,
        parse_run_id: None,
        parse_run_status: None,
        diagnostic_reason: None,
    })
}

fn build_prepare_history_replay_pipeline_run(
    request: PrepareHistoryReplayRunRequest,
    context: &VersionContext,
) -> anyhow::Result<PreparedPipelineRun> {
    let run_id = request.run_id.trim().to_string();
    validate_http_run_id(&run_id)?;
    let release_id = request.release_id.trim().to_string();
    validate_http_run_id(&release_id)?;
    if request.from_sesno >= request.to_sesno {
        anyhow::bail!(
            "invalid sesno range for prepare-history-replay: from_sesno={} must be less than to_sesno={}",
            request.from_sesno,
            request.to_sesno
        );
    }

    let timeout_secs = request.timeout_secs.unwrap_or(600);
    if timeout_secs == 0 {
        anyhow::bail!("timeout_secs must be greater than 0");
    }
    let poll_interval_ms = request.poll_interval_ms.unwrap_or(1_000);
    if poll_interval_ms == 0 {
        anyhow::bail!("poll_interval_ms must be greater than 0");
    }
    let force = request.force.unwrap_or(false);
    let state_dir = default_runner_state_dir(context);
    let project_model_version_root = context
        .output_root
        .join(&context.project_name)
        .join("model_versions");
    let run_dir = state_dir.join(&run_id);
    let metrics_path = run_dir.join("task-metrics.json");
    let source_observation_manifest_path = state_dir
        .join("_source_observations")
        .join(&run_id)
        .join("source_observation_manifest.json");
    let replay_config_arg = request.replay_config_out.unwrap_or_else(|| {
        project_model_version_root
            .join("replay_configs")
            .join(&release_id)
            .join("DbOption-replay")
    });
    let baseline_config_arg = request.baseline_config_out.unwrap_or_else(|| {
        project_model_version_root
            .join("replay_configs")
            .join(&release_id)
            .join("DbOption-baseline")
    });
    let replay_output_root = request.replay_output_root.unwrap_or_else(|| {
        project_model_version_root
            .join("replay_work")
            .join(&release_id)
            .join("output")
    });

    ensure_path_under(&state_dir, &run_dir, "run_dir")?;
    ensure_path_under(&run_dir, &metrics_path, "metrics_path")?;
    ensure_path_under(
        &state_dir,
        &source_observation_manifest_path,
        "source_observation_manifest_path",
    )?;
    ensure_path_under(
        &project_model_version_root,
        &replay_config_arg,
        "replay_config_out",
    )?;
    ensure_path_under(
        &project_model_version_root,
        &baseline_config_arg,
        "baseline_config_out",
    )?;
    ensure_path_under(
        &project_model_version_root,
        &replay_output_root,
        "replay_output_root",
    )?;

    if run_dir.exists() && !force {
        anyhow::bail!(
            "run directory already exists for '{}'; pass force=true to overwrite: {}",
            run_id,
            run_dir.display()
        );
    }
    if source_observation_manifest_path.exists() && !force {
        anyhow::bail!(
            "source observation manifest already exists for '{}'; pass force=true to overwrite: {}",
            run_id,
            source_observation_manifest_path.display()
        );
    }

    let executable = resolve_http_aios_database_executable(request.executable.as_ref())?;
    let mut dependency_files = request.dependency_files.unwrap_or_default();
    let mut snapshot_id = None;
    let mut baseline_state_manifest_path = None;
    let mut baseline_state_manifest_hash = None;
    let dbnum;
    let source_db_file;
    let base_config_arg;
    let baseline_source_confirmed_at_from_sesno;
    let source_mode;
    let expected_source_db_sha256;

    if let Some(raw_snapshot_id) = request
        .snapshot_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        validate_http_run_id(raw_snapshot_id)?;
        let physical_snapshot_root = project_model_version_root
            .join("physical_baselines")
            .join(raw_snapshot_id);
        let manifest_path = physical_snapshot_root.join("baseline_state_manifest.json");
        ensure_path_under(
            &project_model_version_root,
            &physical_snapshot_root,
            "physical_snapshot_root",
        )?;
        ensure_path_under(
            &physical_snapshot_root,
            &manifest_path,
            "baseline_state_manifest_path",
        )?;
        let baseline_state = validate_http_baseline_state(
            context,
            raw_snapshot_id,
            request.dbnum,
            &physical_snapshot_root,
            &manifest_path,
        )?;
        if request
            .base_config_arg
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some()
        {
            anyhow::bail!(
                "snapshot_id mode derives base_config_arg from baseline_state_manifest; omit base_config_arg"
            );
        }
        dbnum = baseline_state.dbnum;
        source_db_file = request
            .source_db_file
            .clone()
            .unwrap_or_else(|| baseline_state.replacement_db_file.clone());
        if !source_db_file.is_file() {
            anyhow::bail!(
                "history replay source_db_file is missing or not a file: {}",
                source_db_file.display()
            );
        }
        base_config_arg = strip_toml_suffix(&baseline_state.config_path.to_string_lossy());
        baseline_source_confirmed_at_from_sesno = true;
        if absolute_lexical_path(&source_db_file)?
            == absolute_lexical_path(&baseline_state.replacement_db_file)?
        {
            source_mode = "physical_snapshot_replacement_source".to_string();
            expected_source_db_sha256 = Some(baseline_state.replacement_db_sha256.clone());
        } else {
            source_mode = "physical_snapshot_with_history_source".to_string();
            expected_source_db_sha256 = None;
        }
        dependency_files.push(manifest_path.clone());
        dependency_files.push(baseline_state.replacement_db_file.clone());
        snapshot_id = Some(raw_snapshot_id.to_string());
        baseline_state_manifest_hash = Some(baseline_state.baseline_state_manifest_hash.clone());
        baseline_state_manifest_path = Some(manifest_path);
    } else {
        baseline_source_confirmed_at_from_sesno = request
            .baseline_source_confirmed_at_from_sesno
            .unwrap_or(false);
        if !baseline_source_confirmed_at_from_sesno {
            anyhow::bail!(
                "direct prepare-history-replay requires baseline_source_confirmed_at_from_sesno=true; use snapshot_id for a validated physical baseline or explicitly confirm source_db_file is already the from_sesno baseline"
            );
        }
        dbnum = request
            .dbnum
            .ok_or_else(|| anyhow::anyhow!("dbnum is required without snapshot_id"))?;
        source_db_file = request
            .source_db_file
            .clone()
            .ok_or_else(|| anyhow::anyhow!("source_db_file is required without snapshot_id"))?;
        base_config_arg = request
            .base_config_arg
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(strip_toml_suffix)
            .unwrap_or_else(default_db_option_file_arg);
        source_mode = "direct_confirmed_source".to_string();
        expected_source_db_sha256 = None;
    }

    let current_parquet_dir = request.current_parquet_dir.unwrap_or_else(|| {
        context
            .output_root
            .join(&context.project_name)
            .join("parquet")
            .join(dbnum.to_string())
    });
    let baseline_dbnums = normalize_dbnum_list(request.baseline_dbnums.unwrap_or_default());
    let source_observation = build_source_observation_manifest(SourceObservationBuildRequest {
        observation_id: run_id.clone(),
        project_name: context.project_name.clone(),
        dbnum,
        primary_file: source_db_file.clone(),
        dependency_files,
        requested_sesno: Some(format!(
            "history-replay:{}-{}",
            request.from_sesno, request.to_sesno
        )),
        resolved_sesno: Some(request.to_sesno),
        quiescence_window_ms: request.quiescence_window_ms.unwrap_or(0),
    })?;
    if !source_observation.quiescence.stable {
        anyhow::bail!(
            "history replay source DB file changed during observation window; retry after the snapshot/source is stable: {}",
            source_observation.primary.path.display()
        );
    }
    if let Some(expected_hash) = &expected_source_db_sha256 {
        if !source_observation
            .primary
            .sha256
            .eq_ignore_ascii_case(expected_hash)
        {
            anyhow::bail!(
                "baseline replacement DB hash mismatch: manifest={}, observed={}",
                expected_hash,
                source_observation.primary.sha256
            );
        }
    }
    let source_observation_manifest_hash =
        write_source_observation_manifest(&source_observation_manifest_path, &source_observation)?;

    let mut argv = vec![
        "aios-database".to_string(),
        "-c".to_string(),
        base_config_arg.clone(),
        "model-version".to_string(),
        "prepare-history-replay".to_string(),
        "--release-id".to_string(),
        release_id.clone(),
        "--branch-id".to_string(),
        request
            .branch_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("main")
            .to_string(),
        "--project".to_string(),
        context.project_name.clone(),
        "--dbnum".to_string(),
        dbnum.to_string(),
        "--source-db-file".to_string(),
        source_observation
            .primary
            .path
            .to_string_lossy()
            .to_string(),
        "--from-sesno".to_string(),
        request.from_sesno.to_string(),
        "--to-sesno".to_string(),
        request.to_sesno.to_string(),
        "--base-config".to_string(),
        base_config_arg.clone(),
        "--replay-config-out".to_string(),
        replay_config_arg.to_string_lossy().to_string(),
        "--baseline-config-out".to_string(),
        baseline_config_arg.to_string_lossy().to_string(),
        "--replay-output-root".to_string(),
        replay_output_root.to_string_lossy().to_string(),
        "--current-parquet-dir".to_string(),
        current_parquet_dir.to_string_lossy().to_string(),
        "--baseline-source-confirmed-at-from-sesno".to_string(),
        "--json".to_string(),
    ];
    if let Some(value) = request
        .release_label
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        argv.push("--release-label".to_string());
        argv.push(value.to_string());
    }
    if let Some(value) = request
        .baseline_release_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        argv.push("--baseline-release-id".to_string());
        argv.push(value.to_string());
    }
    if let Some(value) = request
        .parent_release_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        argv.push("--parent-release-id".to_string());
        argv.push(value.to_string());
    }
    if let Some(value) = request
        .replay_surreal_ns
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        argv.push("--replay-surreal-ns".to_string());
        argv.push(value.to_string());
    }
    for baseline_dbnum in baseline_dbnums {
        argv.push("--baseline-dbnum".to_string());
        argv.push(baseline_dbnum.to_string());
    }
    if force {
        argv.push("--force".to_string());
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut env = BTreeMap::new();
    env.insert(
        "AIOS_TASK_METRICS_KIND".to_string(),
        "prepare_history_replay".to_string(),
    );
    env.insert(
        "AIOS_TASK_METRICS_PATH".to_string(),
        metrics_path.to_string_lossy().to_string(),
    );
    env.insert(
        "AIOS_SOURCE_OBSERVATION_MANIFEST".to_string(),
        source_observation_manifest_path
            .to_string_lossy()
            .to_string(),
    );
    env.insert(
        "AIOS_SOURCE_OBSERVATION_MANIFEST_SHA256".to_string(),
        source_observation_manifest_hash.clone(),
    );
    if let Some(path) = &baseline_state_manifest_path {
        env.insert(
            "AIOS_BASELINE_STATE_MANIFEST".to_string(),
            path.to_string_lossy().to_string(),
        );
    }
    if let Some(hash) = &baseline_state_manifest_hash {
        env.insert(
            "AIOS_BASELINE_STATE_MANIFEST_SHA256".to_string(),
            hash.clone(),
        );
    }

    Ok(PreparedPipelineRun {
        run_request: BoundedCommandRunRequest {
            run_id,
            kind: "prepare_history_replay".to_string(),
            state_dir,
            executable: Some(executable),
            argv,
            cwd,
            env,
            stdout_path: None,
            stderr_path: None,
            metrics_path: Some(metrics_path),
            timeout_secs,
            stale_heartbeat_secs: request.stale_heartbeat_secs,
            source_db_file: Some(source_observation.primary.path.clone()),
            expected_source_db_sha256: Some(source_observation.primary.sha256.clone()),
            poll_interval_ms,
            force,
        },
        snapshot_id,
        source_observation_manifest_path,
        source_observation_manifest_hash,
        source_observation,
        baseline_state_manifest_path,
        baseline_state_manifest_hash,
        history_replay: Some(PrepareHistoryReplayRunEvidence {
            source_mode,
            baseline_source_confirmed_at_from_sesno,
            source_db_file,
            base_config_arg,
            replay_config_arg,
            baseline_config_arg,
            replay_output_root,
            current_parquet_dir,
        }),
        parse_run_id: None,
        parse_run_status: None,
        diagnostic_reason: None,
    })
}

fn build_execute_history_replay_plan_run(
    request: ExecuteHistoryReplayPlanRunRequest,
    context: &VersionContext,
) -> anyhow::Result<PreparedHistoryReplayPlanRun> {
    let prepare_run_id = request.prepare_run_id.trim().to_string();
    validate_http_run_id(&prepare_run_id)?;
    let phase = normalize_history_replay_plan_phase(&request.phase)?;
    let run_id = request
        .run_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("{prepare_run_id}-{phase}"));
    validate_http_run_id(&run_id)?;
    if run_id == prepare_run_id {
        anyhow::bail!("run_id must differ from prepare_run_id");
    }

    let state_dir = request
        .state_dir
        .clone()
        .unwrap_or_else(|| default_runner_state_dir(context));
    let prepare_record = read_bounded_run_status(&state_dir, &prepare_run_id)
        .with_context(|| format!("prepare run not found or unreadable: {prepare_run_id}"))?;
    if prepare_record.kind != "prepare_history_replay" {
        anyhow::bail!(
            "prepare_run_id '{}' is kind '{}', expected prepare_history_replay",
            prepare_run_id,
            prepare_record.kind
        );
    }
    if prepare_record.status != BoundedRunStatus::Succeeded || prepare_record.exit_code != Some(0) {
        anyhow::bail!(
            "prepare_run_id '{}' must be succeeded with exit_code=0 before execution; status={:?} exit_code={:?}",
            prepare_run_id,
            prepare_record.status,
            prepare_record.exit_code
        );
    }
    if prepare_record.source_db_hash_unchanged != Some(true) {
        anyhow::bail!(
            "prepare run source DB hash changed or was not verified; refusing to execute plan from '{}'",
            prepare_run_id
        );
    }
    let expected_source_db_sha256 = prepare_record
        .source_db_sha256_after
        .clone()
        .or_else(|| prepare_record.source_db_sha256_before.clone())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "prepare run '{}' has no source DB hash evidence",
                prepare_run_id
            )
        })?;
    let prepare_source_db_file = prepare_record.source_db_file.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "prepare run '{}' has no source_db_file evidence",
            prepare_run_id
        )
    })?;
    if !prepare_record.stdout_path.is_file() {
        anyhow::bail!(
            "prepare run stdout is missing or not a file: {}",
            prepare_record.stdout_path.display()
        );
    }
    let prepare_stdout_hash = sha256_file(&prepare_record.stdout_path)?;
    let prepare_stdout = fs::read_to_string(&prepare_record.stdout_path).with_context(|| {
        format!(
            "read prepare history replay stdout failed: {}",
            prepare_record.stdout_path.display()
        )
    })?;
    let replay_plan: ModelHistoryReplayPrepareResponse = serde_json::from_str(&prepare_stdout)
        .with_context(|| {
            format!(
                "parse prepare history replay stdout JSON failed: {}",
                prepare_record.stdout_path.display()
            )
        })?;
    if replay_plan.project_name != context.project_name {
        anyhow::bail!(
            "history replay plan project mismatch: plan={} request_context={}",
            replay_plan.project_name,
            context.project_name
        );
    }
    if absolute_lexical_path(&replay_plan.source_db_file)?
        != absolute_lexical_path(&prepare_source_db_file)?
    {
        anyhow::bail!(
            "history replay plan source_db_file differs from prepare run record: plan={} record={}",
            replay_plan.source_db_file.display(),
            prepare_source_db_file.display()
        );
    }
    if !replay_plan
        .safety_checks
        .baseline_source_confirmed_at_from_sesno
    {
        anyhow::bail!(
            "history replay plan is not baseline-source confirmed at from_sesno; refusing execution"
        );
    }
    if !replay_plan.safety_checks.generation_is_external_process {
        anyhow::bail!("history replay plan must require external generation process execution");
    }

    let selected_argv = select_history_replay_phase_argv(&replay_plan, &phase)?;
    validate_history_replay_phase_argv(&replay_plan, &phase, selected_argv)?;
    let prepare_stdout_path = prepare_record.stdout_path.clone();

    let timeout_secs = request
        .timeout_secs
        .unwrap_or_else(|| default_history_replay_phase_timeout_secs(&phase));
    if timeout_secs == 0 {
        anyhow::bail!("timeout_secs must be greater than 0");
    }
    let poll_interval_ms = request.poll_interval_ms.unwrap_or(1_000);
    if poll_interval_ms == 0 {
        anyhow::bail!("poll_interval_ms must be greater than 0");
    }
    let cwd = request
        .cwd
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    if !cwd.is_dir() {
        anyhow::bail!("cwd is not a directory: {}", cwd.display());
    }
    let executable = resolve_http_aios_database_executable(request.executable.as_ref())?;
    let run_dir = state_dir.join(&run_id);
    let metrics_path = request
        .metrics_path
        .unwrap_or_else(|| run_dir.join("task-metrics.json"));
    ensure_path_under(&state_dir, &run_dir, "run_dir")?;
    ensure_path_under(&run_dir, &metrics_path, "metrics_path")?;

    let mut env = request.env.unwrap_or_default();
    if env.keys().any(|key| key.trim().is_empty()) {
        anyhow::bail!("env keys must not be empty");
    }
    env.entry("AIOS_TASK_METRICS_KIND".to_string())
        .or_insert_with(|| format!("history_replay_plan_{phase}"));
    env.entry("AIOS_TASK_METRICS_PATH".to_string())
        .or_insert_with(|| metrics_path.to_string_lossy().to_string());
    env.insert(
        "AIOS_HISTORY_REPLAY_PREPARE_RUN_ID".to_string(),
        prepare_run_id.clone(),
    );
    env.insert(
        "AIOS_HISTORY_REPLAY_PREPARE_STDOUT_SHA256".to_string(),
        prepare_stdout_hash.clone(),
    );

    let safety_checks = serde_json::to_value(&replay_plan.safety_checks)
        .context("serialize history replay safety checks")?;
    let plan_summary = HistoryReplayPlanExecutionSummary {
        project_name: replay_plan.project_name.clone(),
        release_id: replay_plan.release_id.clone(),
        baseline_release_id: replay_plan.baseline_release_id.clone(),
        dbnum: replay_plan.dbnum,
        from_sesno: replay_plan.from_sesno,
        to_sesno: replay_plan.to_sesno,
        source_db_file: replay_plan.source_db_file.clone(),
        replay_config_arg: replay_plan.replay_config_arg.clone(),
        baseline_config_arg: replay_plan.baseline_config_arg.clone(),
        replay_output_root: replay_plan.replay_output_root.clone(),
        replay_parquet_dir: replay_plan.replay_parquet_dir.clone(),
        current_parquet_dir: replay_plan.current_parquet_dir.clone(),
        safety_checks,
    };

    Ok(PreparedHistoryReplayPlanRun {
        run_request: BoundedCommandRunRequest {
            run_id,
            kind: format!("history_replay_plan_{phase}"),
            state_dir,
            executable: Some(executable),
            argv: selected_argv.to_vec(),
            cwd,
            env,
            stdout_path: request.stdout_path,
            stderr_path: request.stderr_path,
            metrics_path: Some(metrics_path),
            timeout_secs,
            stale_heartbeat_secs: request.stale_heartbeat_secs,
            source_db_file: Some(replay_plan.source_db_file.clone()),
            expected_source_db_sha256: Some(expected_source_db_sha256.clone()),
            poll_interval_ms,
            force: request.force.unwrap_or(false),
        },
        phase,
        prepare_run_id,
        prepare_record,
        prepare_stdout_path,
        prepare_stdout_hash,
        plan: plan_summary,
        source_db_file: replay_plan.source_db_file,
        expected_source_db_sha256,
    })
}

fn normalize_history_replay_plan_phase(raw: &str) -> anyhow::Result<String> {
    let normalized = raw.trim().replace('-', "_").to_ascii_lowercase();
    match normalized.as_str() {
        "baseline_parse" | "baseline_generate" | "baseline_register" | "generate" | "publish" => {
            Ok(normalized)
        }
        _ => anyhow::bail!(
            "unsupported history replay plan phase '{}'; expected baseline_parse, baseline_generate, baseline_register, generate, or publish",
            raw
        ),
    }
}

fn default_history_replay_phase_timeout_secs(phase: &str) -> u64 {
    match phase {
        "baseline_register" => 1_800,
        "publish" => 7_200,
        "baseline_parse" | "baseline_generate" | "generate" => 14_400,
        _ => 3_600,
    }
}

fn select_history_replay_phase_argv<'a>(
    plan: &'a ModelHistoryReplayPrepareResponse,
    phase: &str,
) -> anyhow::Result<&'a [String]> {
    let argv = match phase {
        "baseline_parse" => &plan.commands.baseline_parse_argv,
        "baseline_generate" => &plan.commands.baseline_generate_argv,
        "baseline_register" => &plan.commands.baseline_register_argv,
        "generate" => &plan.commands.generate_argv,
        "publish" => &plan.commands.publish_argv,
        _ => anyhow::bail!("unsupported history replay plan phase '{}'", phase),
    };
    if argv.is_empty() {
        anyhow::bail!("history replay plan phase '{}' has empty argv", phase);
    }
    if argv.iter().any(|item| item.trim().is_empty()) {
        anyhow::bail!(
            "history replay plan phase '{}' argv contains empty arguments",
            phase
        );
    }
    Ok(argv)
}

fn validate_history_replay_phase_argv(
    plan: &ModelHistoryReplayPrepareResponse,
    phase: &str,
    argv: &[String],
) -> anyhow::Result<()> {
    require_aios_database_argv(argv, phase)?;
    require_argv_token(argv, "-c", phase)?;
    match phase {
        "baseline_parse" | "baseline_generate" => {
            if argv.len() != 3 {
                anyhow::bail!(
                    "history replay phase '{}' must be the prepared aios-database -c <config> argv only",
                    phase
                );
            }
        }
        "baseline_register" => {
            require_argv_sequence(argv, &["model-version", "register"], phase)?;
            require_argv_flag_value(argv, "--release-id", &plan.baseline_release_id, phase)?;
            require_argv_flag_value(argv, "--dbnum", &plan.dbnum.to_string(), phase)?;
            require_argv_flag_value(
                argv,
                "--parquet-dir",
                &plan.replay_parquet_dir.to_string_lossy(),
                phase,
            )?;
            require_argv_flag_value(argv, "--derivation-type", "historical-baseline", phase)?;
            require_argv_token(argv, "--metadata-json", phase)?;
            require_argv_token(argv, "--json", phase)?;
        }
        "generate" => {
            require_argv_token(argv, "incremental-sesno", phase)?;
            require_argv_flag_value(
                argv,
                "--file",
                &plan.source_db_file.to_string_lossy(),
                phase,
            )?;
            require_argv_flag_value(argv, "--from-sesno", &plan.from_sesno.to_string(), phase)?;
            require_argv_flag_value(argv, "--to-sesno", &plan.to_sesno.to_string(), phase)?;
            require_argv_token(argv, "--generate-model", phase)?;
            require_argv_token(argv, "--json", phase)?;
        }
        "publish" => {
            require_argv_sequence(argv, &["model-version", "publish-history"], phase)?;
            require_argv_flag_value(argv, "--release-id", &plan.release_id, phase)?;
            require_argv_flag_value(argv, "--dbnum", &plan.dbnum.to_string(), phase)?;
            require_argv_flag_value(
                argv,
                "--source-db-file",
                &plan.source_db_file.to_string_lossy(),
                phase,
            )?;
            require_argv_flag_value(argv, "--from-sesno", &plan.from_sesno.to_string(), phase)?;
            require_argv_flag_value(argv, "--to-sesno", &plan.to_sesno.to_string(), phase)?;
            require_argv_flag_value(
                argv,
                "--parquet-dir",
                &plan.replay_parquet_dir.to_string_lossy(),
                phase,
            )?;
            require_argv_flag_value(
                argv,
                "--parent-release-id",
                &plan.baseline_release_id,
                phase,
            )?;
            require_argv_token(argv, "--materialize-assets", phase)?;
            require_argv_token(argv, "--json", phase)?;
            if !plan.safety_checks.materialize_assets_in_publish_command {
                anyhow::bail!(
                    "history replay publish plan lacks materialize-assets safety evidence"
                );
            }
        }
        _ => anyhow::bail!("unsupported history replay plan phase '{}'", phase),
    }
    Ok(())
}

fn require_aios_database_argv(argv: &[String], phase: &str) -> anyhow::Result<()> {
    let Some(first) = argv.first() else {
        anyhow::bail!("history replay phase '{}' argv is empty", phase);
    };
    if !is_aios_database_executable(Path::new(first)) {
        anyhow::bail!(
            "history replay phase '{}' argv must start with aios-database, got '{}'",
            phase,
            first
        );
    }
    Ok(())
}

fn require_argv_token(argv: &[String], token: &str, phase: &str) -> anyhow::Result<()> {
    if !argv.iter().any(|item| item == token) {
        anyhow::bail!(
            "history replay phase '{}' argv must include token '{}'",
            phase,
            token
        );
    }
    Ok(())
}

fn require_argv_sequence(argv: &[String], sequence: &[&str], phase: &str) -> anyhow::Result<()> {
    if sequence.is_empty() {
        return Ok(());
    }
    if argv.windows(sequence.len()).any(|window| {
        window
            .iter()
            .map(String::as_str)
            .eq(sequence.iter().copied())
    }) {
        return Ok(());
    }
    anyhow::bail!(
        "history replay phase '{}' argv must include sequence '{}'",
        phase,
        sequence.join(" ")
    )
}

fn require_argv_flag_value(
    argv: &[String],
    flag: &str,
    expected: &str,
    phase: &str,
) -> anyhow::Result<()> {
    let expected = expected.trim();
    let Some(actual) = argv
        .windows(2)
        .find_map(|window| (window[0] == flag).then(|| window[1].trim()))
    else {
        anyhow::bail!(
            "history replay phase '{}' argv must include flag '{}'",
            phase,
            flag
        );
    };
    if actual != expected {
        anyhow::bail!(
            "history replay phase '{}' argv flag '{}' mismatch: expected '{}', got '{}'",
            phase,
            flag,
            expected,
            actual
        );
    }
    Ok(())
}

fn validate_http_baseline_state(
    context: &VersionContext,
    snapshot_id: &str,
    expected_dbnum: Option<u32>,
    physical_snapshot_root: &Path,
    baseline_state_manifest_path: &Path,
) -> anyhow::Result<ModelBaselineStateValidationResponse> {
    if !baseline_state_manifest_path.is_file() {
        anyhow::bail!(
            "baseline state manifest is missing; prepare the physical snapshot first: {}",
            baseline_state_manifest_path.display()
        );
    }

    let baseline_state = validate_baseline_state_request(ModelBaselineStateValidationRequest {
        project_name: context.project_name.clone(),
        dbnum: expected_dbnum,
        from_sesno: None,
        baseline_state_manifest_path: baseline_state_manifest_path.to_path_buf(),
        baseline_state_manifest_hash: None,
        scene_tree_dir: None,
        require_scene_tree: false,
    })?;

    if baseline_state.snapshot_id != snapshot_id {
        anyhow::bail!(
            "baseline state manifest snapshot_id mismatch: expected {}, got {}",
            snapshot_id,
            baseline_state.snapshot_id
        );
    }

    ensure_path_under(
        physical_snapshot_root,
        &baseline_state.snapshot_root,
        "manifest.snapshot_root",
    )?;
    ensure_path_under(
        physical_snapshot_root,
        &baseline_state.config_path,
        "manifest.config_path",
    )?;
    ensure_path_under(
        physical_snapshot_root,
        &baseline_state.output_root,
        "manifest.output_root",
    )?;
    ensure_path_under(
        physical_snapshot_root,
        &baseline_state.replacement_db_file,
        "manifest.replacement_db_file",
    )?;
    if !baseline_state.config_path.is_file() {
        anyhow::bail!(
            "baseline config file is missing: {}",
            baseline_state.config_path.display()
        );
    }
    if !baseline_state.replacement_db_file.is_file() {
        anyhow::bail!(
            "baseline replacement DB file is missing: {}",
            baseline_state.replacement_db_file.display()
        );
    }

    Ok(baseline_state)
}

fn build_parse_baseline_pipeline_run(
    request: ParseBaselineRunRequest,
    context: &VersionContext,
) -> anyhow::Result<PreparedPipelineRun> {
    let run_id = request.run_id.trim().to_string();
    validate_http_run_id(&run_id)?;
    let snapshot_id = request.snapshot_id.trim().to_string();
    validate_http_run_id(&snapshot_id)?;

    let timeout_secs = request.timeout_secs.unwrap_or(7_200);
    if timeout_secs == 0 {
        anyhow::bail!("timeout_secs must be greater than 0");
    }
    let poll_interval_ms = request.poll_interval_ms.unwrap_or(1_000);
    if poll_interval_ms == 0 {
        anyhow::bail!("poll_interval_ms must be greater than 0");
    }
    let force = request.force.unwrap_or(false);
    let state_dir = default_runner_state_dir(context);
    let project_model_version_root = context
        .output_root
        .join(&context.project_name)
        .join("model_versions");
    let physical_snapshot_root = project_model_version_root
        .join("physical_baselines")
        .join(&snapshot_id);
    let baseline_state_manifest_path = physical_snapshot_root.join("baseline_state_manifest.json");
    let run_dir = state_dir.join(&run_id);
    let metrics_path = run_dir.join("task-metrics.json");
    let source_observation_manifest_path = state_dir
        .join("_source_observations")
        .join(&run_id)
        .join("source_observation_manifest.json");

    ensure_path_under(
        &project_model_version_root,
        &physical_snapshot_root,
        "physical_snapshot_root",
    )?;
    ensure_path_under(
        &physical_snapshot_root,
        &baseline_state_manifest_path,
        "baseline_state_manifest_path",
    )?;
    ensure_path_under(&state_dir, &run_dir, "run_dir")?;
    ensure_path_under(&run_dir, &metrics_path, "metrics_path")?;
    ensure_path_under(
        &state_dir,
        &source_observation_manifest_path,
        "source_observation_manifest_path",
    )?;

    if run_dir.exists() && !force {
        anyhow::bail!(
            "run directory already exists for '{}'; pass force=true to overwrite: {}",
            run_id,
            run_dir.display()
        );
    }
    if source_observation_manifest_path.exists() && !force {
        anyhow::bail!(
            "source observation manifest already exists for '{}'; pass force=true to overwrite: {}",
            run_id,
            source_observation_manifest_path.display()
        );
    }
    let executable = resolve_http_aios_database_executable(request.executable.as_ref())?;
    let baseline_state = validate_http_baseline_state(
        context,
        &snapshot_id,
        request.dbnum,
        &physical_snapshot_root,
        &baseline_state_manifest_path,
    )?;
    let baseline_state_manifest_hash = baseline_state.baseline_state_manifest_hash.clone();

    let mut dependency_files = request.dependency_files.unwrap_or_default();
    dependency_files.push(baseline_state_manifest_path.clone());
    let source_observation = build_source_observation_manifest(SourceObservationBuildRequest {
        observation_id: run_id.clone(),
        project_name: context.project_name.clone(),
        dbnum: baseline_state.dbnum,
        primary_file: baseline_state.replacement_db_file.clone(),
        dependency_files,
        requested_sesno: Some(format!(
            "physical-snapshot:{}",
            baseline_state.source_db_latest_sesno
        )),
        resolved_sesno: Some(baseline_state.source_db_latest_sesno),
        quiescence_window_ms: request.quiescence_window_ms.unwrap_or(0),
    })?;
    if !source_observation.quiescence.stable {
        anyhow::bail!(
            "baseline replacement DB file changed during observation window; retry after the snapshot is stable: {}",
            source_observation.primary.path.display()
        );
    }
    if !source_observation
        .primary
        .sha256
        .eq_ignore_ascii_case(&baseline_state.replacement_db_sha256)
    {
        anyhow::bail!(
            "baseline replacement DB hash mismatch: manifest={}, observed={}",
            baseline_state.replacement_db_sha256,
            source_observation.primary.sha256
        );
    }
    let source_observation_manifest_hash =
        write_source_observation_manifest(&source_observation_manifest_path, &source_observation)?;

    let config_arg = strip_toml_suffix(&baseline_state.config_path.to_string_lossy());
    let argv = vec!["aios-database".to_string(), "-c".to_string(), config_arg];
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut env = BTreeMap::new();
    env.insert(
        "AIOS_TASK_METRICS_KIND".to_string(),
        "parse_baseline".to_string(),
    );
    env.insert(
        "AIOS_TASK_METRICS_PATH".to_string(),
        metrics_path.to_string_lossy().to_string(),
    );
    env.insert(
        "AIOS_SOURCE_OBSERVATION_MANIFEST".to_string(),
        source_observation_manifest_path
            .to_string_lossy()
            .to_string(),
    );
    env.insert(
        "AIOS_SOURCE_OBSERVATION_MANIFEST_SHA256".to_string(),
        source_observation_manifest_hash.clone(),
    );
    env.insert(
        "AIOS_BASELINE_STATE_MANIFEST".to_string(),
        baseline_state_manifest_path.to_string_lossy().to_string(),
    );
    env.insert(
        "AIOS_BASELINE_STATE_MANIFEST_SHA256".to_string(),
        baseline_state_manifest_hash.clone(),
    );

    Ok(PreparedPipelineRun {
        run_request: BoundedCommandRunRequest {
            run_id,
            kind: "parse_baseline".to_string(),
            state_dir,
            executable: Some(executable),
            argv,
            cwd,
            env,
            stdout_path: None,
            stderr_path: None,
            metrics_path: Some(metrics_path),
            timeout_secs,
            stale_heartbeat_secs: request.stale_heartbeat_secs,
            source_db_file: Some(source_observation.primary.path.clone()),
            expected_source_db_sha256: Some(source_observation.primary.sha256.clone()),
            poll_interval_ms,
            force,
        },
        snapshot_id: Some(snapshot_id),
        source_observation_manifest_path,
        source_observation_manifest_hash,
        source_observation,
        baseline_state_manifest_path: Some(baseline_state_manifest_path),
        baseline_state_manifest_hash: Some(baseline_state_manifest_hash),
        history_replay: None,
        parse_run_id: None,
        parse_run_status: None,
        diagnostic_reason: None,
    })
}

fn build_generate_full_model_pipeline_run(
    request: GenerateFullModelRunRequest,
    context: &VersionContext,
) -> anyhow::Result<PreparedPipelineRun> {
    let run_id = request.run_id.trim().to_string();
    validate_http_run_id(&run_id)?;
    let snapshot_id = request.snapshot_id.trim().to_string();
    validate_http_run_id(&snapshot_id)?;

    let timeout_secs = request.timeout_secs.unwrap_or(14_400);
    if timeout_secs == 0 {
        anyhow::bail!("timeout_secs must be greater than 0");
    }
    let poll_interval_ms = request.poll_interval_ms.unwrap_or(1_000);
    if poll_interval_ms == 0 {
        anyhow::bail!("poll_interval_ms must be greater than 0");
    }
    let force = request.force.unwrap_or(false);
    let state_dir = default_runner_state_dir(context);
    let project_model_version_root = context
        .output_root
        .join(&context.project_name)
        .join("model_versions");
    let physical_snapshot_root = project_model_version_root
        .join("physical_baselines")
        .join(&snapshot_id);
    let baseline_state_manifest_path = physical_snapshot_root.join("baseline_state_manifest.json");
    let run_dir = state_dir.join(&run_id);
    let metrics_path = run_dir.join("task-metrics.json");
    let source_observation_manifest_path = state_dir
        .join("_source_observations")
        .join(&run_id)
        .join("source_observation_manifest.json");

    ensure_path_under(
        &project_model_version_root,
        &physical_snapshot_root,
        "physical_snapshot_root",
    )?;
    ensure_path_under(
        &physical_snapshot_root,
        &baseline_state_manifest_path,
        "baseline_state_manifest_path",
    )?;
    ensure_path_under(&state_dir, &run_dir, "run_dir")?;
    ensure_path_under(&run_dir, &metrics_path, "metrics_path")?;
    ensure_path_under(
        &state_dir,
        &source_observation_manifest_path,
        "source_observation_manifest_path",
    )?;

    if run_dir.exists() && !force {
        anyhow::bail!(
            "run directory already exists for '{}'; pass force=true to overwrite: {}",
            run_id,
            run_dir.display()
        );
    }
    if source_observation_manifest_path.exists() && !force {
        anyhow::bail!(
            "source observation manifest already exists for '{}'; pass force=true to overwrite: {}",
            run_id,
            source_observation_manifest_path.display()
        );
    }
    let executable = resolve_http_aios_database_executable(request.executable.as_ref())?;
    let baseline_state = validate_http_baseline_state(
        context,
        &snapshot_id,
        request.dbnum,
        &physical_snapshot_root,
        &baseline_state_manifest_path,
    )?;
    let baseline_state_manifest_hash = baseline_state.baseline_state_manifest_hash.clone();

    let allow_incomplete_parse = request.allow_incomplete_parse.unwrap_or(false);
    let diagnostic_reason = request
        .diagnostic_reason
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    if allow_incomplete_parse && diagnostic_reason.is_none() {
        anyhow::bail!(
            "diagnostic_reason is required when allow_incomplete_parse=true; production generate-full-model requires a successful parse_run_id"
        );
    }

    let mut parse_run_id = None;
    let mut parse_run_status = None;
    if allow_incomplete_parse {
        if let Some(raw_parse_run_id) = request
            .parse_run_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            validate_http_run_id(raw_parse_run_id)?;
            let parse_record =
                read_bounded_run_status(&state_dir, raw_parse_run_id).with_context(|| {
                    format!(
                        "read diagnostic parse run status failed for '{}'",
                        raw_parse_run_id
                    )
                })?;
            parse_run_status = Some(parse_record.status);
            parse_run_id = Some(raw_parse_run_id.to_string());
        }
    } else {
        let raw_parse_run_id = request
            .parse_run_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "parse_run_id is required unless allow_incomplete_parse=true with a diagnostic_reason"
                )
            })?;
        validate_http_run_id(raw_parse_run_id)?;
        let parse_record =
            read_bounded_run_status(&state_dir, raw_parse_run_id).with_context(|| {
                format!(
                    "missing dependency: read parse run status failed for '{}'",
                    raw_parse_run_id
                )
            })?;
        if parse_record.kind != "parse_baseline" {
            anyhow::bail!(
                "missing dependency: parse_run_id '{}' must reference kind parse_baseline, got {}",
                raw_parse_run_id,
                parse_record.kind
            );
        }
        if parse_record.status != BoundedRunStatus::Succeeded {
            anyhow::bail!(
                "missing dependency: parse_run_id '{}' must have status succeeded before generate-full-model, got {:?}",
                raw_parse_run_id,
                parse_record.status
            );
        }
        if parse_record.source_db_hash_unchanged != Some(true) {
            anyhow::bail!(
                "missing dependency: parse_run_id '{}' did not prove the baseline replacement DB hash stayed unchanged",
                raw_parse_run_id
            );
        }
        let parse_source = parse_record.source_db_file.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "missing dependency: parse_run_id '{}' is missing source_db_file evidence",
                raw_parse_run_id
            )
        })?;
        if absolute_lexical_path(parse_source)?
            != absolute_lexical_path(&baseline_state.replacement_db_file)?
        {
            anyhow::bail!(
                "missing dependency: parse_run_id '{}' used a different source DB file: expected {}, got {}",
                raw_parse_run_id,
                baseline_state.replacement_db_file.display(),
                parse_source.display()
            );
        }
        let expected_hash = baseline_state.replacement_db_sha256.as_str();
        if !parse_record
            .source_db_sha256_before
            .as_deref()
            .map(|hash| hash.eq_ignore_ascii_case(expected_hash))
            .unwrap_or(false)
        {
            anyhow::bail!(
                "missing dependency: parse_run_id '{}' source hash before run does not match baseline manifest",
                raw_parse_run_id
            );
        }
        if !parse_record
            .source_db_sha256_after
            .as_deref()
            .map(|hash| hash.eq_ignore_ascii_case(expected_hash))
            .unwrap_or(false)
        {
            anyhow::bail!(
                "missing dependency: parse_run_id '{}' source hash after run does not match baseline manifest",
                raw_parse_run_id
            );
        }
        parse_run_status = Some(parse_record.status);
        parse_run_id = Some(raw_parse_run_id.to_string());
    }

    let mut dependency_files = request.dependency_files.unwrap_or_default();
    dependency_files.push(baseline_state_manifest_path.clone());
    let source_observation = build_source_observation_manifest(SourceObservationBuildRequest {
        observation_id: run_id.clone(),
        project_name: context.project_name.clone(),
        dbnum: baseline_state.dbnum,
        primary_file: baseline_state.replacement_db_file.clone(),
        dependency_files,
        requested_sesno: Some(format!(
            "physical-snapshot:{}",
            baseline_state.source_db_latest_sesno
        )),
        resolved_sesno: Some(baseline_state.source_db_latest_sesno),
        quiescence_window_ms: request.quiescence_window_ms.unwrap_or(0),
    })?;
    if !source_observation.quiescence.stable {
        anyhow::bail!(
            "baseline replacement DB file changed during observation window; retry after the snapshot is stable: {}",
            source_observation.primary.path.display()
        );
    }
    if !source_observation
        .primary
        .sha256
        .eq_ignore_ascii_case(&baseline_state.replacement_db_sha256)
    {
        anyhow::bail!(
            "baseline replacement DB hash mismatch: manifest={}, observed={}",
            baseline_state.replacement_db_sha256,
            source_observation.primary.sha256
        );
    }
    let source_observation_manifest_hash =
        write_source_observation_manifest(&source_observation_manifest_path, &source_observation)?;

    let config_arg = strip_toml_suffix(&baseline_state.config_path.to_string_lossy());
    let argv = vec![
        "aios-database".to_string(),
        "-c".to_string(),
        config_arg,
        "--regen-model".to_string(),
        "--dbnum".to_string(),
        baseline_state.dbnum.to_string(),
        "--export-parquet-after-gen".to_string(),
    ];
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut env = BTreeMap::new();
    env.insert(
        "AIOS_TASK_METRICS_KIND".to_string(),
        "generate_full_model".to_string(),
    );
    env.insert(
        "AIOS_TASK_METRICS_PATH".to_string(),
        metrics_path.to_string_lossy().to_string(),
    );
    env.insert(
        "AIOS_SOURCE_OBSERVATION_MANIFEST".to_string(),
        source_observation_manifest_path
            .to_string_lossy()
            .to_string(),
    );
    env.insert(
        "AIOS_SOURCE_OBSERVATION_MANIFEST_SHA256".to_string(),
        source_observation_manifest_hash.clone(),
    );
    env.insert(
        "AIOS_BASELINE_STATE_MANIFEST".to_string(),
        baseline_state_manifest_path.to_string_lossy().to_string(),
    );
    env.insert(
        "AIOS_BASELINE_STATE_MANIFEST_SHA256".to_string(),
        baseline_state_manifest_hash.clone(),
    );
    if let Some(parse_run_id) = &parse_run_id {
        env.insert("AIOS_PARSE_RUN_ID".to_string(), parse_run_id.clone());
    }
    if let Some(reason) = &diagnostic_reason {
        env.insert(
            "AIOS_MODEL_VERSION_DIAGNOSTIC_REASON".to_string(),
            reason.clone(),
        );
    }

    Ok(PreparedPipelineRun {
        run_request: BoundedCommandRunRequest {
            run_id,
            kind: "generate_full_model".to_string(),
            state_dir,
            executable: Some(executable),
            argv,
            cwd,
            env,
            stdout_path: None,
            stderr_path: None,
            metrics_path: Some(metrics_path),
            timeout_secs,
            stale_heartbeat_secs: request.stale_heartbeat_secs,
            source_db_file: Some(source_observation.primary.path.clone()),
            expected_source_db_sha256: Some(source_observation.primary.sha256.clone()),
            poll_interval_ms,
            force,
        },
        snapshot_id: Some(snapshot_id),
        source_observation_manifest_path,
        source_observation_manifest_hash,
        source_observation,
        baseline_state_manifest_path: Some(baseline_state_manifest_path),
        baseline_state_manifest_hash: Some(baseline_state_manifest_hash),
        history_replay: None,
        parse_run_id,
        parse_run_status,
        diagnostic_reason,
    })
}

fn runner_state_dir_from_query(
    project_override: Option<&str>,
    state_dir_override: Option<PathBuf>,
) -> anyhow::Result<PathBuf> {
    if let Some(state_dir) = state_dir_override {
        return Ok(state_dir);
    }
    let context = version_context(project_override)?;
    Ok(default_runner_state_dir(&context))
}

fn default_runner_state_dir(context: &VersionContext) -> PathBuf {
    context
        .output_root
        .join(&context.project_name)
        .join("model_versions")
        .join("runs")
}

fn ensure_path_under(base: &Path, path: &Path, label: &str) -> anyhow::Result<()> {
    let base = absolute_lexical_path(base)?;
    let path = absolute_lexical_path(path)?;
    if !path.starts_with(&base) {
        anyhow::bail!(
            "{label} must stay under {}; got {}",
            base.display(),
            path.display()
        );
    }
    Ok(())
}

fn absolute_lexical_path(path: &Path) -> anyhow::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("resolve current directory failed")?
            .join(path)
    };
    Ok(normalize_lexical(&absolute))
}

fn normalize_lexical(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

fn strip_toml_suffix(value: &str) -> String {
    value
        .trim()
        .strip_suffix(".toml")
        .unwrap_or_else(|| value.trim())
        .to_string()
}

fn default_db_option_file_arg() -> String {
    std::env::var("DB_OPTION_FILE")
        .ok()
        .map(|value| strip_toml_suffix(&value))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "db_options/DbOption".to_string())
}

fn normalize_dbnum_list(mut values: Vec<u32>) -> Vec<u32> {
    values.sort_unstable();
    values.dedup();
    values
}

fn resolve_http_aios_database_executable(requested: Option<&PathBuf>) -> anyhow::Result<PathBuf> {
    if let Some(path) = requested {
        return validate_aios_database_executable(path);
    }

    let mut candidates = Vec::new();
    if let Ok(current_exe) = std::env::current_exe() {
        if is_aios_database_executable(&current_exe) && current_exe.is_file() {
            return Ok(current_exe);
        }
        if let Some(parent) = current_exe.parent() {
            candidates.push(parent.join("aios-database.exe"));
            candidates.push(parent.join("aios-database"));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("target").join("debug").join("aios-database.exe"));
        candidates.push(cwd.join("target").join("debug").join("aios-database"));
        candidates.push(
            cwd.join("target")
                .join("codex-cli-validate-build")
                .join("debug")
                .join("aios-database.exe"),
        );
        candidates.push(
            cwd.join("target")
                .join("codex-cli-validate-build")
                .join("debug")
                .join("aios-database"),
        );
    }

    for candidate in candidates {
        if candidate.is_file() && is_aios_database_executable(&candidate) {
            return Ok(candidate);
        }
    }

    anyhow::bail!(
        "aios-database executable was not found; pass executable pointing to aios-database"
    )
}

fn validate_aios_database_executable(path: &Path) -> anyhow::Result<PathBuf> {
    if !path.is_file() {
        anyhow::bail!("executable is missing or not a file: {}", path.display());
    }
    if !is_aios_database_executable(path) {
        anyhow::bail!(
            "HTTP model-version runs only allow the aios-database executable, got {}",
            path.display()
        );
    }
    Ok(std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
}

fn is_aios_database_executable(path: &Path) -> bool {
    path.file_stem()
        .and_then(|value| value.to_str())
        .map(|stem| stem.eq_ignore_ascii_case("aios-database"))
        .unwrap_or(false)
}

fn validate_http_run_id(run_id: &str) -> anyhow::Result<()> {
    if run_id.is_empty() {
        anyhow::bail!("run_id must not be empty");
    }
    if run_id.len() > 128 {
        anyhow::bail!("run_id must be <= 128 characters");
    }
    if run_id.contains("..")
        || run_id.starts_with('.')
        || run_id.ends_with('.')
        || !run_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        anyhow::bail!(
            "run_id must be path-safe ASCII using only letters, numbers, dash, underscore, or dot"
        );
    }
    Ok(())
}

fn bounded_run_thread_name(run_id: &str) -> String {
    let mut name = String::from("model-version-run-");
    name.extend(run_id.chars().take(40));
    name
}

async fn wait_for_bounded_run_record(state_dir: &Path, run_id: &str) -> Option<BoundedRunRecord> {
    for _ in 0..50 {
        if let Ok(record) = read_bounded_run_status(state_dir, run_id) {
            return Some(record);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    None
}

fn version_context(project_override: Option<&str>) -> anyhow::Result<VersionContext> {
    let db_option_ext = load_runtime_db_option_ext()?;
    let project_name = project_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| db_option_ext.inner.project_name.clone());
    if project_name.trim().is_empty() {
        anyhow::bail!("project name is required");
    }

    let output_root = db_option_ext.get_output_root();
    let project_output_dir = output_root.join(&project_name);
    let ducklake =
        ModelVersionDuckLakeConfig::for_project_output_dir(&project_output_dir, &project_name);

    Ok(VersionContext {
        project_name,
        output_root,
        mesh_root: db_option_ext.inner.get_meshes_path(),
        ducklake,
    })
}

fn load_runtime_db_option_ext() -> anyhow::Result<DbOptionExt> {
    let raw_config =
        std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption".into());
    let config_path = strip_toml_extension(&raw_config);
    get_db_option_ext_from_path(&config_path)
        .with_context(|| format!("load runtime DbOptionExt from {config_path}.toml"))
        .or_else(|_| Ok(get_db_option_ext()))
}

fn strip_toml_extension(raw: &str) -> String {
    let path = PathBuf::from(raw);
    if path.extension().and_then(|ext| ext.to_str()) == Some("toml") {
        path.with_extension("").to_string_lossy().to_string()
    } else {
        raw.to_string()
    }
}

fn normalize_change_type(raw: Option<&str>) -> Result<Option<String>, String> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let value = raw.trim().to_ascii_lowercase();
    if value.is_empty() {
        return Ok(None);
    }
    match value.as_str() {
        "added" | "deleted" | "changed" => Ok(Some(value)),
        _ => Err("change_type must be one of added, deleted, changed".to_string()),
    }
}

fn normalize_release_quality_filter(
    raw: Option<&str>,
    complete_visual_only: bool,
) -> Result<Option<&'static str>, String> {
    if complete_visual_only {
        return Ok(Some("complete_visual"));
    }
    let Some(raw) = raw else {
        return Ok(None);
    };
    let value = raw.trim().to_ascii_lowercase();
    if value.is_empty() || value == "all" || value == "*" {
        return Ok(None);
    }
    match value.as_str() {
        "complete_visual" | "complete" => Ok(Some("complete_visual")),
        "quarantined_visual" | "quarantined" | "quarantine" => {
            Ok(Some("quarantined_visual"))
        }
        "degraded_visual" | "degraded" => Ok(Some("degraded_visual")),
        "patch_only" | "patch" => Ok(Some("patch_only")),
        "non_visual" | "nonvisual" => Ok(Some("non_visual")),
        _ => Err(
            "quality must be one of all, complete_visual, quarantined_visual, degraded_visual, patch_only, non_visual"
                .to_string(),
        ),
    }
}

fn parse_optional_release_quality(
    raw: Option<&str>,
) -> anyhow::Result<Option<ModelReleaseQuality>> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let quality = match raw.to_ascii_lowercase().as_str() {
        "complete_visual" | "complete" => ModelReleaseQuality::CompleteVisual,
        "quarantined_visual" | "quarantined" | "quarantine" => {
            ModelReleaseQuality::QuarantinedVisual
        }
        "degraded_visual" | "degraded" | "partial" => ModelReleaseQuality::DegradedVisual,
        "patch_only" | "patch-only" | "patch" => ModelReleaseQuality::PatchOnly,
        "non_visual" | "non-visual" | "nonvisual" => ModelReleaseQuality::NonVisual,
        _ => anyhow::bail!(
            "invalid release quality '{}'; expected complete_visual, quarantined_visual, degraded_visual, patch_only, or non_visual",
            raw
        ),
    };
    Ok(Some(quality))
}

fn non_empty_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_string_list(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .flat_map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn push_unique_validation_flag(flags: &mut Vec<String>, flag: &str) {
    if !flags.iter().any(|existing| existing == flag) {
        flags.push(flag.to_string());
    }
}

fn metadata_json_object(value: Option<Value>) -> anyhow::Result<Value> {
    match value {
        Some(value) if value.is_object() => Ok(value),
        Some(_) => anyhow::bail!("metadata_json must be a JSON object"),
        None => Ok(json!({})),
    }
}

fn json_value_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

fn json_string_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    json_value_at(value, path)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn required_json_string_at<'a>(value: &'a Value, path: &[&str]) -> anyhow::Result<&'a str> {
    json_string_at(value, path).ok_or_else(|| {
        anyhow::anyhow!("JSON field '{}' must be a non-empty string", path.join("."))
    })
}

fn required_json_bool_at(value: &Value, path: &[&str]) -> anyhow::Result<bool> {
    json_value_at(value, path)
        .and_then(Value::as_bool)
        .ok_or_else(|| anyhow::anyhow!("JSON field '{}' must be a boolean", path.join(".")))
}

fn required_json_u32_at(value: &Value, path: &[&str]) -> anyhow::Result<u32> {
    let raw = json_value_at(value, path)
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "JSON field '{}' must be an unsigned integer",
                path.join(".")
            )
        })?;
    u32::try_from(raw)
        .map_err(|_| anyhow::anyhow!("JSON field '{}' is out of u32 range", path.join(".")))
}

fn select_handoff_candidate_index(
    candidates: &[Value],
    candidate_index: Option<usize>,
    dbnum: Option<u32>,
) -> anyhow::Result<usize> {
    if candidates.is_empty() {
        anyhow::bail!("incremental handoff manifest has no candidates");
    }
    if let Some(index) = candidate_index {
        let candidate = candidates.get(index).ok_or_else(|| {
            anyhow::anyhow!(
                "candidate_index {} out of range for {} candidates",
                index,
                candidates.len()
            )
        })?;
        if let Some(expected_dbnum) = dbnum {
            let actual_dbnum = required_json_u32_at(candidate, &["dbnum"])?;
            if actual_dbnum != expected_dbnum {
                anyhow::bail!(
                    "candidate_index {} points to dbnum {}, not requested dbnum {}",
                    index,
                    actual_dbnum,
                    expected_dbnum
                );
            }
        }
        return Ok(index);
    }
    if let Some(expected_dbnum) = dbnum {
        let mut matches = Vec::new();
        for (index, candidate) in candidates.iter().enumerate() {
            if required_json_u32_at(candidate, &["dbnum"])? == expected_dbnum {
                matches.push(index);
            }
        }
        return match matches.as_slice() {
            [index] => Ok(*index),
            [] => anyhow::bail!(
                "no incremental handoff candidate found for dbnum {}",
                expected_dbnum
            ),
            _ => anyhow::bail!(
                "multiple incremental handoff candidates found for dbnum {}; pass candidate_index",
                expected_dbnum
            ),
        };
    }
    if candidates.len() == 1 {
        Ok(0)
    } else {
        anyhow::bail!(
            "incremental handoff has {} candidates; pass candidate_index or dbnum",
            candidates.len()
        )
    }
}

fn validate_candidate_rows_by_table(
    candidate: &Value,
    package_rows: &BTreeMap<String, u64>,
) -> anyhow::Result<()> {
    let Some(rows) = candidate.get("rows_by_table").and_then(Value::as_object) else {
        return Ok(());
    };
    for (table, value) in rows {
        let expected = value.as_u64().ok_or_else(|| {
            anyhow::anyhow!(
                "candidate.rows_by_table.{} must be an unsigned integer",
                table
            )
        })?;
        let actual = package_rows
            .get(table)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("loaded package missing candidate table '{}'", table))?;
        if actual != expected {
            anyhow::bail!(
                "candidate rows_by_table mismatch for '{}': manifest={} actual={}",
                table,
                expected,
                actual
            );
        }
    }
    Ok(())
}

fn suggested_incremental_handoff_release_id(
    dbnum: u32,
    to_sesno: u32,
    package_hash: &str,
) -> String {
    let hash_prefix: String = package_hash.chars().take(12).collect();
    format!("http-incr-db{dbnum}-sesno{to_sesno}-pkg{hash_prefix}")
}

fn component_key_filter(
    component_key: Option<&str>,
    refno_u64: Option<u64>,
    dbnum: Option<u32>,
) -> Result<Option<String>, String> {
    if let Some(component_key) = component_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(Some(component_key.to_string()));
    }
    let Some(refno_u64) = refno_u64 else {
        return Ok(None);
    };
    let Some(dbnum) = dbnum else {
        return Err("dbnum is required when refno_u64 is used".to_string());
    };
    Ok(Some(format!("{}:{}", dbnum, refno_u64)))
}

fn release_view(release: ModelReleaseRecord, output_root: &Path) -> ModelReleaseView {
    let package_url = output_file_url(&release.immutable_package_dir, output_root);
    let manifest_url = package_url
        .as_ref()
        .map(|base| format!("{}/manifest.json", base.trim_end_matches('/')));
    let viewer_url = viewer_url_for_release(&release, package_url.as_deref());
    let release_viewer_url = release_viewer_url_for_release(&release);
    ModelReleaseView {
        release,
        package_url,
        manifest_url,
        viewer_url,
        release_viewer_url,
    }
}

fn viewer_url_for_release(release: &ModelReleaseRecord, package_url: Option<&str>) -> String {
    let mut parts = vec![
        format!(
            "output_project={}",
            urlencoding::encode(&release.project_name)
        ),
        format!("show_dbnum={}", release.dbnum),
        format!(
            "model_release_id={}",
            urlencoding::encode(&release.release_id)
        ),
    ];
    if let Some(package_url) = package_url {
        parts.push(format!(
            "parquet_base_url={}",
            urlencoding::encode(package_url)
        ));
    }
    format!("/viewer/?{}", parts.join("&"))
}

fn release_viewer_url_for_release(release: &ModelReleaseRecord) -> String {
    format!(
        "/model-version/release-viewer?project={}&release_id={}",
        urlencoding::encode(&release.project_name),
        urlencoding::encode(&release.release_id)
    )
}

fn read_release_manifest(release: &ModelReleaseRecord) -> anyhow::Result<serde_json::Value> {
    let manifest_path = release.immutable_package_dir.join("manifest.json");
    let raw = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("read release manifest failed: {}", manifest_path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("parse release manifest failed: {}", manifest_path.display()))
}

fn manifest_mesh_lod_tag(manifest: &serde_json::Value) -> String {
    manifest
        .get("mesh_validation")
        .and_then(|value| value.get("lod_tag"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("L1")
        .to_string()
}

fn release_local_mesh_base_url(
    release: &ModelReleaseRecord,
    lod_tag: &str,
    output_root: &Path,
) -> Option<String> {
    let release_root = release_root_dir(release).ok()?;
    let mesh_dir = release_root.join("meshes").join(format!("lod_{lod_tag}"));
    if mesh_dir.is_dir() {
        output_file_url(&mesh_dir, output_root)
    } else {
        None
    }
}

fn release_root_dir(release: &ModelReleaseRecord) -> anyhow::Result<PathBuf> {
    let parquet_dir = release.immutable_package_dir.parent().with_context(|| {
        format!(
            "release immutable package dir has no parquet parent: {}",
            release.immutable_package_dir.display()
        )
    })?;
    let release_root = parquet_dir.parent().with_context(|| {
        format!(
            "release immutable package dir has no release root: {}",
            release.immutable_package_dir.display()
        )
    })?;
    Ok(release_root.to_path_buf())
}

fn output_file_url(path: &Path, output_root: &Path) -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let abs_root = absolute_path(output_root, &cwd);
    let abs_path = absolute_path(path, &cwd);
    let canonical_root = abs_root.canonicalize().unwrap_or(abs_root);
    let canonical_path = abs_path.canonicalize().unwrap_or(abs_path);
    let relative = canonical_path.strip_prefix(&canonical_root).ok()?;
    let mut segments = Vec::new();
    for component in relative.components() {
        let Component::Normal(value) = component else {
            return None;
        };
        segments.push(urlencoding::encode(&value.to_string_lossy()).to_string());
    }
    if segments.is_empty() {
        return Some("/files/output".to_string());
    }
    Some(format!("/files/output/{}", segments.join("/")))
}

fn absolute_path(path: &Path, cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn api_ok<T: Serialize>(message: impl Into<String>, data: T) -> Response {
    json_response(
        StatusCode::OK,
        json!({
            "success": true,
            "message": message.into(),
            "data": data,
        }),
    )
}

fn api_error(message_status: StatusCode, message: impl Into<String>) -> Response {
    json_response(
        message_status,
        json!({
            "success": false,
            "message": message.into(),
            "data": null,
        }),
    )
}

fn json_response(status: StatusCode, value: serde_json::Value) -> Response {
    let mut response = (status, Json(value)).into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    response
}

fn classify_error(message: &str) -> StatusCode {
    let lower = message.to_ascii_lowercase();
    if lower.contains("not found")
        || lower.contains("does not exist")
        || (!lower.contains("missing dependency")
            && (lower.contains(" is missing") || lower.contains("missing or not a file")))
        || message.contains("不存在")
    {
        StatusCode::NOT_FOUND
    } else if lower.contains("missing dependency") || lower.contains("index is missing") {
        StatusCode::FAILED_DEPENDENCY
    } else if lower.contains("required")
        || lower.contains("requires")
        || lower.contains("invalid")
        || lower.contains("must be")
        || lower.contains("unsupported")
        || lower.contains("expected")
        || lower.contains("only allow")
        || lower.contains("cannot register")
        || lower.contains("project name")
        || message.contains("无效")
    {
        StatusCode::BAD_REQUEST
    } else if lower.contains("different project")
        || lower.contains("different dbnum")
        || lower.contains("already exists")
        || lower.contains("lock")
        || lower.contains("another program")
    {
        StatusCode::CONFLICT
    } else if lower.contains("feature `model-version-ducklake`")
        || lower.contains("ducklake extension")
    {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

const RELEASE_VIEWER_PAGE_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Model Release Viewer</title>
  <style>
    :root {
      color-scheme: dark;
      --bg: #0c1118;
      --panel: rgba(15, 23, 42, .86);
      --line: rgba(148, 163, 184, .35);
      --text: #e5edf7;
      --muted: #9aa8ba;
      --accent: #2dd4bf;
      --danger: #fb7185;
    }
    * { box-sizing: border-box; }
    html, body { width: 100%; height: 100%; margin: 0; overflow: hidden; background: var(--bg); color: var(--text); font-family: Inter, Segoe UI, Arial, sans-serif; }
    #viewerCanvas { width: 100%; height: 100%; display: block; background: #0a0f16; }
    #navCubeCanvas { position: absolute; right: 12px; bottom: 12px; width: 120px; height: 120px; }
    .hud { position: absolute; top: 10px; left: 10px; right: 10px; display: flex; gap: 8px; align-items: center; min-width: 0; pointer-events: none; }
    .pill { max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; border: 1px solid var(--line); border-radius: 999px; background: var(--panel); padding: 7px 10px; font-size: 12px; color: var(--muted); }
    .pill strong { color: var(--text); font-weight: 650; }
    .status { position: absolute; left: 10px; bottom: 10px; right: 144px; border: 1px solid var(--line); background: var(--panel); border-radius: 8px; padding: 8px 10px; font-size: 12px; color: var(--muted); overflow-wrap: anywhere; }
    .load-more { position: absolute; right: 12px; top: 52px; min-width: 98px; height: 32px; border: 1px solid var(--line); border-radius: 7px; background: var(--panel); color: var(--text); font: inherit; font-size: 12px; cursor: pointer; }
    .load-more:disabled { cursor: default; color: var(--muted); opacity: .72; }
    .absence-notice { position: absolute; left: 50%; top: 50%; transform: translate(-50%, -50%); max-width: min(420px, calc(100% - 48px)); border: 1px solid rgba(251, 113, 133, .7); border-radius: 8px; background: rgba(15, 23, 42, .92); color: #ffe4e6; box-shadow: 0 12px 32px rgba(0, 0, 0, .32); padding: 14px 16px; text-align: center; pointer-events: none; }
    .absence-notice strong { display: block; color: #fff; font-size: 14px; margin-bottom: 4px; }
    .absence-notice span { display: block; color: #fecdd3; font-size: 12px; overflow-wrap: anywhere; }
    .error { color: var(--danger); }
    a { color: var(--accent); }
  </style>
</head>
<body>
  <canvas id="viewerCanvas"></canvas>
  <canvas id="navCubeCanvas"></canvas>
  <div class="hud">
    <div class="pill"><strong id="releaseLabel">release</strong></div>
    <div class="pill"><span id="counts">loading</span></div>
  </div>
  <button id="loadMoreBtn" class="load-more" type="button" hidden>Load more</button>
  <div class="absence-notice" id="absenceNotice" hidden><strong>Absent</strong><span></span></div>
  <div class="status" id="status">Preparing viewer...</div>
  <script type="module">
    import {
      Viewer,
      GLTFLoaderPlugin,
      NavCubePlugin,
      Mesh,
      ReadableGeometry,
      PhongMaterial,
      buildBoxGeometry,
    } from '/static/xeokit-sdk.es.js';

    const qs = new URLSearchParams(location.search);
    const releaseId = qs.get('release_id') || '';
    const project = qs.get('project') || '';
    const limit = qs.get('limit') || '2000';
    const statusEl = document.getElementById('status');
    const countsEl = document.getElementById('counts');
    const releaseLabelEl = document.getElementById('releaseLabel');
    const loadMoreBtn = document.getElementById('loadMoreBtn');
    const absenceNoticeEl = document.getElementById('absenceNotice');
    const identity = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];

    let viewer = null;
    let loader = null;
    let expectedGeometries = 0;
    let loadedGeometries = 0;
    let failedGeometries = 0;
    let loadedComponents = 0;
    let totalComponents = 0;
    let nextOffset = 0;
    let hasMore = false;
    let pageLoading = false;
    let proxyMeshes = [];
    let proxyMaterial = null;
    const seenComponentKeys = new Set();
    const componentRenderIndex = new Map();
    const selectedModelIds = new Set();
    let selectedComponentKey = null;
    let cameraSyncSeq = 0;

    function setStatus(message, isError = false) {
      statusEl.textContent = message;
      statusEl.className = isError ? 'status error' : 'status';
    }

    function setAbsenceNotice(title, detail, reason) {
      absenceNoticeEl.hidden = false;
      absenceNoticeEl.querySelector('strong').textContent = title;
      absenceNoticeEl.querySelector('span').textContent = detail || '';
      document.body.dataset.absenceVisible = 'true';
      document.body.dataset.absenceReason = reason || '';
      document.body.dataset.absenceTitle = title || '';
      document.body.dataset.absenceDetail = detail || '';
    }

    function clearAbsenceNotice() {
      absenceNoticeEl.hidden = true;
      absenceNoticeEl.querySelector('strong').textContent = '';
      absenceNoticeEl.querySelector('span').textContent = '';
      document.body.dataset.absenceVisible = 'false';
      document.body.dataset.absenceReason = '';
      document.body.dataset.absenceTitle = '';
      document.body.dataset.absenceDetail = '';
    }

    function updateCounts() {
      countsEl.textContent = `components ${loadedComponents}/${totalComponents || loadedComponents} | geometries ${loadedGeometries}/${expectedGeometries} | failed ${failedGeometries}`;
      document.body.dataset.loadedComponents = String(loadedComponents);
      document.body.dataset.totalComponents = String(totalComponents || loadedComponents);
      document.body.dataset.loadedGeometries = String(loadedGeometries);
      document.body.dataset.expectedGeometries = String(expectedGeometries);
      document.body.dataset.failedGeometries = String(failedGeometries);
      document.body.dataset.nextOffset = nextOffset == null ? '' : String(nextOffset);
      document.body.dataset.hasMore = String(Boolean(hasMore));
      document.body.dataset.pageLoading = String(Boolean(pageLoading));
    }

    function updateLoadMoreButton() {
      loadMoreBtn.hidden = !hasMore;
      loadMoreBtn.disabled = pageLoading || !hasMore;
      loadMoreBtn.textContent = pageLoading ? 'Loading...' : 'Load more';
      updateCounts();
    }

    function multiplyMat4(a, b) {
      const out = new Array(16).fill(0);
      for (let col = 0; col < 4; col++) {
        for (let row = 0; row < 4; row++) {
          out[col * 4 + row] =
            a[0 * 4 + row] * b[col * 4 + 0] +
            a[1 * 4 + row] * b[col * 4 + 1] +
            a[2 * 4 + row] * b[col * 4 + 2] +
            a[3 * 4 + row] * b[col * 4 + 3];
        }
      }
      return out;
    }

    function meshUrl(base, lodTag, geoHash) {
      return `${base.replace(/\/$/, '')}/${encodeURIComponent(geoHash)}_${encodeURIComponent(lodTag)}.glb`;
    }

    function unionAabb(components) {
      let min = [Infinity, Infinity, Infinity];
      let max = [-Infinity, -Infinity, -Infinity];
      for (const component of components) {
        if (!component.aabb) continue;
        for (let i = 0; i < 3; i++) {
          min[i] = Math.min(min[i], component.aabb.min[i]);
          max[i] = Math.max(max[i], component.aabb.max[i]);
        }
      }
      if (!Number.isFinite(min[0]) || !Number.isFinite(max[0])) return null;
      return { min, max };
    }

    function fitCamera(components) {
      const aabb = unionAabb(components);
      if (!aabb || !viewer) return;
      const center = [
        (aabb.min[0] + aabb.max[0]) / 2,
        (aabb.min[1] + aabb.max[1]) / 2,
        (aabb.min[2] + aabb.max[2]) / 2,
      ];
      const size = Math.max(
        aabb.max[0] - aabb.min[0],
        aabb.max[1] - aabb.min[1],
        aabb.max[2] - aabb.min[2],
        1000,
      );
      viewer.camera.eye = [center[0] + size * 1.25, center[1] - size * 1.55, center[2] + size * .85];
      viewer.camera.look = center;
      viewer.camera.up = [0, 0, 1];
    }

    function fitLoadedScene() {
      if (!viewer || !viewer.scene || !viewer.scene.aabb) return;
      const aabb = Array.from(viewer.scene.aabb);
      if (aabb.length !== 6 || aabb.some(value => !Number.isFinite(value))) return;
      if (aabb[3] <= aabb[0] || aabb[4] <= aabb[1] || aabb[5] <= aabb[2]) return;
      viewer.cameraFlight.jumpTo({ aabb, fit: true, fitFOV: 65 });
      viewer.camera.up = [0, 0, 1];
    }

    function componentAabbArray(componentOrEntry) {
      const aabb = componentOrEntry && componentOrEntry.aabb;
      if (!aabb || !aabb.min || !aabb.max) return null;
      const values = [
        aabb.min[0], aabb.min[1], aabb.min[2],
        aabb.max[0], aabb.max[1], aabb.max[2],
      ];
      if (values.some(value => !Number.isFinite(value))) return null;
      if (values[3] <= values[0] || values[4] <= values[1] || values[5] < values[2]) return null;
      return values;
    }

    function cameraVec3(value) {
      const values = Array.from(value || []).slice(0, 3).map(Number);
      if (values.length !== 3 || values.some(item => !Number.isFinite(item))) return null;
      return values;
    }

    function roundedCameraVec3(value) {
      const values = cameraVec3(value);
      if (!values) return null;
      return values.map(item => Math.round(item * 1000) / 1000);
    }

    function cameraSnapshot() {
      if (!viewer || !viewer.camera) return null;
      const eye = cameraVec3(viewer.camera.eye);
      const look = cameraVec3(viewer.camera.look);
      const up = cameraVec3(viewer.camera.up);
      if (!eye || !look || !up) return null;
      return {
        eye,
        look,
        up,
        projection: viewer.camera.projection || '',
      };
    }

    function cameraSignature(snapshot = cameraSnapshot()) {
      if (!snapshot) return '';
      const eye = roundedCameraVec3(snapshot.eye);
      const look = roundedCameraVec3(snapshot.look);
      const up = roundedCameraVec3(snapshot.up);
      if (!eye || !look || !up) return '';
      return [eye, look, up].map(values => values.join(',')).join('|');
    }

    function refreshCameraDataset() {
      const signature = cameraSignature();
      if (!signature) return null;
      document.body.dataset.cameraSignature = signature;
      document.body.dataset.cameraSyncSeq = String(cameraSyncSeq);
      return signature;
    }

    function applyCameraSnapshot(snapshot, options = {}) {
      if (!viewer || !viewer.camera || !snapshot) {
        return { applied: false, reason: 'viewer_not_ready' };
      }
      const eye = cameraVec3(snapshot.eye);
      const look = cameraVec3(snapshot.look);
      const up = cameraVec3(snapshot.up);
      if (!eye || !look || !up) {
        return { applied: false, reason: 'invalid_camera_snapshot' };
      }
      viewer.camera.eye = eye;
      viewer.camera.look = look;
      viewer.camera.up = up;
      if (snapshot.projection === 'ortho' || snapshot.projection === 'perspective') {
        try {
          viewer.camera.projection = snapshot.projection;
        } catch (_) {
          // Some xeokit builds expose projection as read-only; eye/look/up are enough for sync.
        }
      }
      cameraSyncSeq += 1;
      document.body.dataset.cameraLastSource = options.source || 'external';
      const signature = refreshCameraDataset();
      return { applied: true, signature, snapshot: cameraSnapshot() };
    }

    function ensureComponentEntry(component) {
      if (!component || !component.component_key) return null;
      let entry = componentRenderIndex.get(component.component_key);
      if (!entry) {
        entry = {
          componentKey: component.component_key,
          refnoStr: component.refno_str || '',
          noun: component.noun || '',
          aabb: component.aabb || null,
          modelIds: [],
          assetLineage: [],
          assetKeys: new Set(),
        };
        componentRenderIndex.set(component.component_key, entry);
      } else if (!entry.aabb && component.aabb) {
        entry.aabb = component.aabb;
      }
      return entry;
    }

    function recordGeometryAsset(entry, geo) {
      if (!entry || !geo) return;
      const asset = geo.mesh_asset || null;
      const key = `${geo.geo_hash || ''}|${geo.geo_index || 0}|${asset?.sha256 || ''}|${asset?.mesh_url || ''}`;
      if (entry.assetKeys.has(key)) return;
      entry.assetKeys.add(key);
      entry.assetLineage.push({
        geo_index: geo.geo_index,
        geo_hash: geo.geo_hash || '',
        mesh_url: asset?.mesh_url || meshUrl(window.__MODEL_VERSION_MESH_BASE_URL || '', window.__MODEL_VERSION_MESH_LOD_TAG || '', geo.geo_hash || ''),
        mesh_relative_path: asset?.mesh_relative_path || null,
        bytes: asset?.bytes ?? null,
        sha256: asset?.sha256 || null,
        exists: asset?.exists ?? null,
        builtin: asset?.builtin ?? null,
        glb_readable: asset?.glb_readable ?? null,
        glb_validation_error: asset?.glb_validation_error || null,
      });
    }

    function applyBaseModelStyle(model) {
      if (!model) return;
      model.visible = true;
      model.opacity = 1.0;
      model.edges = true;
      model.colorize = [0.2, 0.75, 0.95];
      model.selected = false;
      model.highlighted = false;
      for (const objectId of model.objectIds || model.entityIds || []) {
        const object = viewer.scene.objects[objectId];
        if (!object) continue;
        object.visible = true;
        object.opacity = 1.0;
        object.edges = true;
        object.colorize = [0.2, 0.75, 0.95];
        object.selected = false;
        object.highlighted = false;
      }
    }

    function applySelectedModelStyle(model) {
      if (!model) return;
      model.visible = true;
      model.opacity = 1.0;
      model.edges = true;
      model.colorize = [1.0, 0.78, 0.12];
      model.selected = true;
      model.highlighted = true;
      for (const objectId of model.objectIds || model.entityIds || []) {
        const object = viewer.scene.objects[objectId];
        if (!object) continue;
        object.visible = true;
        object.opacity = 1.0;
        object.edges = true;
        object.colorize = [1.0, 0.78, 0.12];
        object.selected = true;
        object.highlighted = true;
      }
    }

    function clearSelection(resetKey = true) {
      if (viewer && viewer.scene) {
        for (const modelId of selectedModelIds) {
          applyBaseModelStyle(viewer.scene.models[modelId]);
        }
      }
      selectedModelIds.clear();
      clearAbsenceNotice();
      if (resetKey) {
        selectedComponentKey = null;
        document.body.dataset.selectedComponentKey = '';
        document.body.dataset.selectionFound = 'false';
        document.body.dataset.selectedModelCount = '0';
        document.body.dataset.selectedAssetCount = '0';
        document.body.dataset.selectedAssetHashes = '';
        document.body.dataset.selectedReadableAssetCount = '0';
      }
    }

    function focusComponentEntry(entry) {
      if (!viewer || !viewer.cameraFlight) return false;
      const aabb = componentAabbArray(entry);
      if (!aabb) return false;
      viewer.cameraFlight.jumpTo({ aabb, fit: true, fitFOV: 55 });
      viewer.camera.up = [0, 0, 1];
      return true;
    }

    async function selectComponent(componentKey, options = {}) {
      clearSelection(false);
      selectedComponentKey = componentKey || null;
      document.body.dataset.selectedComponentKey = selectedComponentKey || '';
      if (!selectedComponentKey) {
        document.body.dataset.selectionFound = 'false';
        document.body.dataset.selectedModelCount = '0';
        document.body.dataset.selectedAssetCount = '0';
        document.body.dataset.selectedAssetHashes = '';
        document.body.dataset.selectedReadableAssetCount = '0';
        return { found: false, component_key: '', model_count: 0, reason: 'missing_component_key', asset_lineage: [], asset_count: 0 };
      }

      let entry = componentRenderIndex.get(selectedComponentKey);
      if (!entry && options.loadIfMissing !== false) {
        document.body.dataset.selectionFound = 'false';
        document.body.dataset.selectedModelCount = '0';
        document.body.dataset.selectedAssetCount = '0';
        document.body.dataset.selectedAssetHashes = '';
        document.body.dataset.selectedReadableAssetCount = '0';
        setStatus(`Loading component ${selectedComponentKey} from immutable release package...`);
        const data = await loadScene(0, selectedComponentKey);
        window.__MODEL_VERSION_SCENE_PAGES = window.__MODEL_VERSION_SCENE_PAGES || [];
        window.__MODEL_VERSION_SCENE_PAGES.push(data.scene);
        await loadGeometryModels(data, true);
        entry = componentRenderIndex.get(selectedComponentKey);
      }

      if (!entry) {
        const expectedAbsent = options.expectedPresence === false;
        const reason = expectedAbsent ? 'component_absent_expected' : 'component_absent_unexpected';
        const title = expectedAbsent ? 'Absent in this release' : 'Component not found in this release';
        const detail = `${options.changeType || 'selection'} ${selectedComponentKey}`;
        document.body.dataset.selectionFound = 'false';
        document.body.dataset.selectedModelCount = '0';
        document.body.dataset.selectionReason = reason;
        document.body.dataset.selectedAssetCount = '0';
        document.body.dataset.selectedAssetHashes = '';
        document.body.dataset.selectedReadableAssetCount = '0';
        setAbsenceNotice(title, detail, reason);
        setStatus(`${title}: ${selectedComponentKey}`);
        return {
          found: false,
          component_key: selectedComponentKey,
          model_count: 0,
          loaded_components: loadedComponents,
          total_components: totalComponents,
          expected_presence: options.expectedPresence ?? null,
          reason,
          asset_lineage: [],
          asset_count: 0,
          readable_asset_count: 0,
        };
      }

      let modelCount = 0;
      for (const modelId of entry.modelIds) {
        const model = viewer?.scene?.models?.[modelId];
        if (!model) continue;
        applySelectedModelStyle(model);
        selectedModelIds.add(modelId);
        modelCount += 1;
      }
      const focused = modelCount > 0 && options.focus !== false ? focusComponentEntry(entry) : false;
      const found = modelCount > 0;
      const assetLineage = entry.assetLineage || [];
      const readableCount = assetLineage.filter(asset => asset.glb_readable === true).length;
      document.body.dataset.selectionFound = String(found);
      document.body.dataset.selectedModelCount = String(modelCount);
      document.body.dataset.selectionReason = found ? '' : 'component_has_no_loaded_geometry';
      document.body.dataset.selectedAssetCount = String(assetLineage.length);
      document.body.dataset.selectedAssetHashes = assetLineage.map(asset => asset.geo_hash).join(',');
      document.body.dataset.selectedReadableAssetCount = String(readableCount);
      if (found) {
        clearAbsenceNotice();
        setStatus(`Selected ${entry.refnoStr || selectedComponentKey} (${modelCount} geometries)`);
      } else {
        const expectedAbsent = options.expectedPresence === false;
        const title = expectedAbsent ? 'Absent visual geometry' : 'No renderable geometry';
        const detail = `${entry.refnoStr || selectedComponentKey} exists but has no loaded mesh geometry`;
        setAbsenceNotice(title, detail, 'component_has_no_loaded_geometry');
        setStatus(`Component ${selectedComponentKey} has no loaded geometry in this release page`);
      }
      return {
        found,
        component_key: selectedComponentKey,
        refno_str: entry.refnoStr,
        noun: entry.noun,
        model_count: modelCount,
        focused,
        loaded_components: loadedComponents,
        total_components: totalComponents,
        expected_presence: options.expectedPresence ?? null,
        reason: found ? null : 'component_has_no_loaded_geometry',
        asset_lineage: assetLineage,
        asset_count: assetLineage.length,
        readable_asset_count: readableCount,
      };
    }

    function reapplySelectedComponent() {
      if (!selectedComponentKey) return;
      void selectComponent(selectedComponentKey, { focus: false, loadIfMissing: false });
    }

    function emphasizeLoadedObjects() {
      if (!viewer || !viewer.scene) return;
      const modelIds = Object.keys(viewer.scene.models || {});
      for (const modelId of modelIds) {
        applyBaseModelStyle(viewer.scene.models[modelId]);
      }
      const objectIds = viewer.scene.objectIds || [];
      for (const objectId of objectIds) {
        const object = viewer.scene.objects[objectId];
        if (!object) continue;
        object.visible = true;
        object.opacity = 1.0;
        object.edges = true;
        object.colorize = [0.2, 0.75, 0.95];
      }
    }

    function buildProxyModels(components, append = false) {
      if (!viewer || !components.length) return;
      if (!append) {
        proxyMeshes.forEach(mesh => mesh.destroy());
        proxyMeshes = [];
      }
      proxyMaterial = proxyMaterial || new PhongMaterial(viewer.scene, {
        diffuse: [0.0, 0.78, 0.95],
        ambient: [0.0, 0.25, 0.35],
        alpha: 0.22,
        alphaMode: 'blend',
      });

      const maxProxyBoxes = Math.min(components.length, 1200);
      let emitted = 0;
      for (const component of components) {
        if (emitted >= maxProxyBoxes) break;
        const aabb = component.aabb;
        if (!aabb || !aabb.min || !aabb.max) continue;
        const min = aabb.min;
        const max = aabb.max;
        const dx = max[0] - min[0];
        const dy = max[1] - min[1];
        const dz = max[2] - min[2];
        if (![dx, dy, dz].every(Number.isFinite)) continue;
        if (dx <= 0 || dy <= 0 || dz < 0) continue;
        const center = [
          (min[0] + max[0]) / 2,
          (min[1] + max[1]) / 2,
          (min[2] + max[2]) / 2,
        ];
        const minHalfSize = 10;
        const geometry = new ReadableGeometry(viewer.scene, buildBoxGeometry({
          center,
          xSize: Math.max(dx / 2, minHalfSize),
          ySize: Math.max(dy / 2, minHalfSize),
          zSize: Math.max(dz / 2, minHalfSize),
        }));
        const mesh = new Mesh(viewer.scene, {
          id: `proxy_${component.refno_u64 || emitted}`,
          geometry,
          material: proxyMaterial,
          edges: true,
          pickable: false,
          clippable: false,
        });
        proxyMeshes.push(mesh);
        emitted += 1;
      }
      document.body.dataset.proxyGeometries = String(proxyMeshes.length);
      if (proxyMeshes.length > 0) {
        statusEl.dataset.proxy = String(proxyMeshes.length);
      }
    }

    function initViewer() {
      viewer = new Viewer({
        canvasId: 'viewerCanvas',
        transparent: false,
        logarithmicDepthBufferEnabled: true,
      });
      viewer.camera.perspective.near = 1;
      viewer.camera.perspective.far = 1000000;
      viewer.camera.ortho.near = 1;
      viewer.camera.ortho.far = 1000000;
      loader = new GLTFLoaderPlugin(viewer);
      new NavCubePlugin(viewer, {
        canvasId: 'navCubeCanvas',
        visible: true,
        size: 120,
        alignment: 'bottomRight',
        bottomMargin: 12,
        rightMargin: 12,
      });
      window.__MODEL_VERSION_VIEWER = viewer;
      window.__MODEL_VERSION_COMPONENT_INDEX = componentRenderIndex;
      window.__MODEL_VERSION_CLEAR_SELECTION = clearSelection;
      window.__MODEL_VERSION_SELECT_COMPONENT = selectComponent;
      window.__MODEL_VERSION_GET_CAMERA = cameraSnapshot;
      window.__MODEL_VERSION_SET_CAMERA = applyCameraSnapshot;
      window.__MODEL_VERSION_GET_CAMERA_SIGNATURE = () => cameraSignature();
      window.setInterval(refreshCameraDataset, 250);
    }

    async function loadScene(offset = 0, componentKey = null) {
      if (!releaseId) {
        throw new Error('release_id query parameter is required');
      }
      const params = new URLSearchParams({ limit, offset: String(offset) });
      if (componentKey) {
        params.set('component_key', componentKey);
        params.set('limit', '1');
        params.set('offset', '0');
      }
      if (project) params.set('project', project);
      const response = await fetch(`/api/model-version/releases/${encodeURIComponent(releaseId)}/runtime-scene?${params}`);
      const body = await response.json();
      if (!response.ok || !body.success) {
        throw new Error(body.message || 'runtime scene request failed');
      }
      return body.data;
    }

    function markPageComplete(scene, message) {
      pageLoading = false;
      updateLoadMoreButton();
      setStatus(message || `Loaded ${loadedGeometries} geometries from ${scene.release.release_id}`);
      window.__MODEL_VERSION_VIEWER_READY = true;
    }

    function loadGeometryModels(data, append = false) {
      const scene = data.scene;
      window.__MODEL_VERSION_MESH_BASE_URL = data.mesh_base_url || '';
      window.__MODEL_VERSION_MESH_LOD_TAG = data.mesh_lod_tag || '';
      const incomingComponents = scene.components || [];
      const components = incomingComponents.filter(component => {
        if (!component.component_key || seenComponentKeys.has(component.component_key)) {
          return false;
        }
        seenComponentKeys.add(component.component_key);
        return true;
      });
      const pageGeometryCount = components.reduce((count, component) => count + ((component.geometries || []).length), 0);
      pageLoading = true;
      window.__MODEL_VERSION_COMPONENTS = loadedComponents + components.length;
      loadedComponents += components.length;
      totalComponents = scene.total_components || Math.max(totalComponents, loadedComponents);
      hasMore = Boolean(scene.has_more);
      nextOffset = scene.next_offset == null ? null : scene.next_offset;
      expectedGeometries += pageGeometryCount;
      releaseLabelEl.textContent = `${scene.release.release_id} | db ${scene.release.dbnum}`;
      updateCounts();
      updateLoadMoreButton();
      buildProxyModels(components, append);
      if (!append) {
        fitCamera(components);
      }

      return new Promise(resolve => {
        let settled = false;
        const finish = (message, isError = false) => {
          if (settled) return;
          settled = true;
          if (isError) {
            pageLoading = false;
            updateLoadMoreButton();
            window.__MODEL_VERSION_VIEWER_READY = true;
            setStatus(message, true);
          } else {
            markPageComplete(scene, message);
          }
          resolve({
            release_id: scene.release.release_id,
            component_count: components.length,
            geometry_count: pageGeometryCount,
            failed_geometries: failedGeometries,
          });
        };
        const maybeFinish = () => {
          if (loadedGeometries + failedGeometries < expectedGeometries) return;
          emphasizeLoadedObjects();
          reapplySelectedComponent();
          fitLoadedScene();
          finish();
        };

        for (const component of components) {
          const componentEntry = ensureComponentEntry(component);
          const instanceMatrix = component.instance_matrix || identity;
          for (const geo of component.geometries || []) {
            const matrix = multiplyMat4(instanceMatrix, geo.geo_matrix || identity);
            const modelId = `${component.refno_u64}_${geo.geo_index}_${geo.geo_hash}`;
            if (componentEntry && !componentEntry.modelIds.includes(modelId)) {
              componentEntry.modelIds.push(modelId);
            }
            recordGeometryAsset(componentEntry, geo);
            const assetUrl = geo.mesh_asset && geo.mesh_asset.mesh_url ? geo.mesh_asset.mesh_url : meshUrl(data.mesh_base_url, data.mesh_lod_tag, geo.geo_hash);
            const model = loader.load({
              id: modelId,
              src: assetUrl,
              matrix,
              edges: true,
              backfaces: true,
              dtxEnabled: false,
              autoMetaModel: true,
            });
            model.once('loaded', () => {
              loadedGeometries += 1;
              updateCounts();
              maybeFinish();
            });
            model.once('error', (message) => {
              failedGeometries += 1;
              updateCounts();
              if (loadedGeometries + failedGeometries >= expectedGeometries) {
                finish(`Mesh load failed: ${message}`, true);
              } else {
                setStatus(`Mesh load failed: ${message}`, true);
              }
            });
          }
        }

        if (pageGeometryCount === 0) {
          finish(`Release ${scene.release.release_id} page has no geometries to display`);
        } else {
          setStatus(`Loading ${pageGeometryCount} geometries from immutable release package...`);
        }
      });
    }

    async function loadNextPage() {
      if (!hasMore || pageLoading || nextOffset == null) return;
      window.__MODEL_VERSION_VIEWER_READY = false;
      pageLoading = true;
      updateLoadMoreButton();
      try {
        const data = await loadScene(nextOffset);
        window.__MODEL_VERSION_SCENE_PAGES = window.__MODEL_VERSION_SCENE_PAGES || [];
        window.__MODEL_VERSION_SCENE_PAGES.push(data.scene);
        await loadGeometryModels(data, true);
      } catch (error) {
        pageLoading = false;
        updateLoadMoreButton();
        setStatus(error.message, true);
      }
    }

    loadMoreBtn.addEventListener('click', loadNextPage);

    initViewer();
    loadScene()
      .then(data => {
        window.__MODEL_VERSION_SCENE = data.scene;
        window.__MODEL_VERSION_SCENE_PAGES = [data.scene];
        loadGeometryModels(data);
      })
      .catch(error => {
        setStatus(error.message, true);
        window.__MODEL_VERSION_VIEWER_READY = false;
      });
  </script>
</body>
</html>"#;

const COMPARE_PAGE_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Model Version Compare</title>
  <style>
    :root {
      color-scheme: light;
      --bg: #f7f9fb;
      --panel: #ffffff;
      --line: #d7dee8;
      --text: #172033;
      --muted: #5e6b7c;
      --accent: #0f766e;
      --accent-2: #b45309;
      --danger: #b91c1c;
    }
    * { box-sizing: border-box; }
    body { margin: 0; font-family: Inter, Segoe UI, Arial, sans-serif; background: var(--bg); color: var(--text); }
    header { display: flex; align-items: center; justify-content: space-between; gap: 16px; padding: 16px 20px; border-bottom: 1px solid var(--line); background: var(--panel); }
    h1 { margin: 0; font-size: 18px; font-weight: 650; letter-spacing: 0; }
    main { display: grid; grid-template-rows: auto auto minmax(360px, 1fr) auto auto auto auto; gap: 12px; padding: 12px; height: calc(100vh - 58px); }
    .toolbar { display: grid; grid-template-columns: minmax(180px, 1fr) minmax(180px, 1fr) minmax(120px, auto) minmax(170px, auto) auto; gap: 10px; align-items: end; }
    label { display: grid; gap: 4px; color: var(--muted); font-size: 12px; }
    select, button { height: 36px; border: 1px solid var(--line); border-radius: 6px; background: #fff; color: var(--text); font: inherit; }
    select { padding: 0 10px; min-width: 0; }
    button { padding: 0 14px; cursor: pointer; }
    button.primary { background: var(--accent); color: #fff; border-color: var(--accent); }
    button:disabled { opacity: .55; cursor: default; }
    .toggle-label { display: flex; align-items: center; gap: 8px; height: 36px; padding: 0 10px; border: 1px solid var(--line); border-radius: 6px; background: #fff; color: var(--text); white-space: nowrap; }
    .toggle-label input { margin: 0; accent-color: var(--accent); }
    .sync-state { color: var(--muted); font-size: 11px; font-weight: 500; }
    .sync-state.active { color: var(--accent); }
    .sync-state.error { color: var(--danger); }
    .viewer-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; min-height: 0; }
    .pane { display: grid; grid-template-rows: auto 1fr auto; min-height: 0; border: 1px solid var(--line); background: var(--panel); border-radius: 8px; overflow: hidden; }
    .pane-title { display: flex; align-items: center; justify-content: space-between; gap: 10px; padding: 10px 12px; border-bottom: 1px solid var(--line); font-size: 13px; }
    .pane-links { display: flex; gap: 8px; flex: none; }
    .pane-title strong { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .pane-heading { display: flex; align-items: center; gap: 8px; min-width: 0; }
    .quality-badge { display: inline-flex; align-items: center; height: 22px; padding: 0 8px; border-radius: 999px; border: 1px solid #cbd5e1; color: #475569; background: #f8fafc; font-size: 11px; font-weight: 650; white-space: nowrap; flex: none; }
    .quality-badge.complete { border-color: #99f6e4; color: #0f766e; background: #f0fdfa; }
    .quality-badge.quarantined { border-color: #fed7aa; color: #b45309; background: #fff7ed; }
    .quality-badge.degraded { border-color: #fecaca; color: #b91c1c; background: #fef2f2; }
    .quality-badge.patch, .quality-badge.nonvisual { border-color: #d8b4fe; color: #7e22ce; background: #faf5ff; }
    iframe { width: 100%; height: 100%; min-height: 260px; border: 0; background: #eef2f7; }
    .meta { padding: 8px 12px; border-top: 1px solid var(--line); font-size: 12px; color: var(--muted); overflow-wrap: anywhere; }
    .meta-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 4px 12px; }
    .meta-line { min-width: 0; }
    .meta-line span { color: #334155; font-weight: 600; }
    .summary { display: grid; grid-template-columns: repeat(5, minmax(90px, 1fr)); gap: 8px; }
    .metric { border: 1px solid var(--line); border-radius: 8px; background: var(--panel); padding: 10px 12px; }
    .metric span { display: block; color: var(--muted); font-size: 12px; }
    .metric strong { display: block; margin-top: 4px; font-size: 20px; font-weight: 650; }
    .rows { min-height: 140px; max-height: 240px; overflow: auto; border: 1px solid var(--line); border-radius: 8px; background: var(--panel); }
    table { width: 100%; border-collapse: collapse; font-size: 12px; }
    th, td { padding: 8px 10px; border-bottom: 1px solid var(--line); text-align: left; white-space: nowrap; }
    th { position: sticky; top: 0; background: #fbfcfe; color: var(--muted); font-weight: 600; }
    tbody tr.selectable { cursor: pointer; }
    tbody tr.selectable:hover { background: #f1f5f9; }
    tbody tr.selected { background: #ccfbf1; outline: 2px solid #14b8a6; outline-offset: -2px; }
    .changed { color: var(--accent-2); font-weight: 650; }
    .added { color: var(--accent); font-weight: 650; }
    .deleted { color: var(--danger); font-weight: 650; }
    .selection-status { border: 1px solid var(--line); border-radius: 8px; background: var(--panel); padding: 8px 12px; color: var(--muted); font-size: 12px; display: flex; flex-wrap: wrap; gap: 10px; align-items: center; }
    .selection-status strong { color: var(--text); }
    .selection-status .found { color: var(--accent); font-weight: 650; }
    .selection-status .missing { color: var(--danger); font-weight: 650; }
    .selection-status .asset-lineage { flex-basis: 100%; color: var(--muted); overflow-wrap: anywhere; }
    .readiness-status { border: 1px solid var(--line); border-radius: 8px; background: var(--panel); padding: 8px 12px; color: var(--muted); font-size: 12px; display: flex; flex-wrap: wrap; gap: 10px; align-items: center; }
    .readiness-status strong { color: var(--text); }
    .readiness-status.production_ready { border-color: #99f6e4; background: #f0fdfa; }
    .readiness-status.quarantined_visual { border-color: #fed7aa; background: #fff7ed; }
    .readiness-status.incomplete_indexes, .readiness-status.missing_release, .readiness-status.not_production_ready { border-color: #fecaca; background: #fef2f2; }
    .readiness-status .ready { color: var(--accent); font-weight: 650; }
    .readiness-status .not-ready { color: var(--danger); font-weight: 650; }
    .readiness-status .detail { flex-basis: 100%; overflow-wrap: anywhere; }
    .error { color: var(--danger); font-size: 13px; }
    a { color: var(--accent); text-decoration: none; }
    @media (max-width: 900px) {
      main { height: auto; }
      .toolbar, .viewer-grid, .summary, .meta-grid { grid-template-columns: 1fr; }
      iframe { min-height: 360px; }
    }
  </style>
</head>
<body>
  <header>
    <h1>Model Version Compare</h1>
    <a href="/api/model-version/releases" target="_blank">JSON</a>
  </header>
  <main>
    <section class="toolbar">
      <label>From release<select id="fromRelease"></select></label>
      <label>To release<select id="toRelease"></select></label>
      <label>Change<select id="changeType"><option value="">All</option><option>changed</option><option>added</option><option>deleted</option></select></label>
      <label class="toggle-label"><input type="checkbox" id="cameraSync"> Camera sync <span id="cameraSyncStatus" class="sync-state">off</span></label>
      <button id="compareButton" class="primary">Compare</button>
    </section>
    <section class="readiness-status" id="readinessStatus" hidden></section>
    <section class="viewer-grid">
      <div class="pane">
        <div class="pane-title"><span class="pane-heading"><strong id="fromTitle">From</strong><span id="fromQuality" class="quality-badge"></span></span><span class="pane-links"><a id="fromOpen" target="_blank">Open</a><a id="fromPlantViewer" target="_blank">Plant</a></span></div>
        <iframe id="fromFrame" title="from model"></iframe>
        <div class="meta" id="fromMeta"></div>
      </div>
      <div class="pane">
        <div class="pane-title"><span class="pane-heading"><strong id="toTitle">To</strong><span id="toQuality" class="quality-badge"></span></span><span class="pane-links"><a id="toOpen" target="_blank">Open</a><a id="toPlantViewer" target="_blank">Plant</a></span></div>
        <iframe id="toFrame" title="to model"></iframe>
        <div class="meta" id="toMeta"></div>
      </div>
    </section>
    <section class="summary" id="summary"></section>
    <section class="selection-status" id="selectionStatus" hidden></section>
    <section class="rows">
      <table>
        <thead><tr><th>Change</th><th>Refno</th><th>Noun</th><th>Old hash</th><th>New hash</th></tr></thead>
        <tbody id="rows"></tbody>
      </table>
    </section>
    <div class="error" id="error"></div>
  </main>
  <script>
    const state = {
      releases: [],
      diffRows: [],
      readiness: null,
      selectedRowIndex: null,
      cameraSync: {
        enabled: false,
        lastFrom: '',
        lastTo: '',
        lastSource: '',
        lastMessage: 'off',
        ticks: 0,
      },
    };
    const qs = new URLSearchParams(location.search);
    const viewerLimit = qs.get('viewer_limit') || '2000';
    const diffLimit = qs.get('diff_limit') || '200';
    const byId = id => document.getElementById(id);
    const text = value => value == null || value === '' ? '-' : String(value);
    const queryReleaseId = (canonical, legacy) => qs.get(canonical) || qs.get(legacy);

    function releaseLabel(release) {
      return `${release.release_id} | db ${release.dbnum}`;
    }

    function shortHash(value) {
      return value ? String(value).slice(0, 12) : '-';
    }

    function escapeHtml(value) {
      return text(value)
        .replaceAll('&', '&amp;')
        .replaceAll('<', '&lt;')
        .replaceAll('>', '&gt;')
        .replaceAll('"', '&quot;')
        .replaceAll("'", '&#39;');
    }

    function qualityClass(release) {
      const quality = String(release?.release_quality || release?.release_status || '').toLowerCase();
      if (quality.includes('quarantined')) return 'quarantined';
      if (quality.includes('degraded') || quality.includes('failed')) return 'degraded';
      if (quality.includes('patch')) return 'patch';
      if (quality.includes('non_visual')) return 'nonvisual';
      if (quality.includes('complete') || quality.includes('published')) return 'complete';
      return '';
    }

    function renderQuality(prefix, release) {
      const badge = byId(`${prefix}Quality`);
      if (!badge) return;
      const label = release ? (release.release_quality || release.release_status || '-') : '-';
      badge.textContent = label;
      badge.className = `quality-badge ${release ? qualityClass(release) : ''}`;
      badge.hidden = !release;
    }

    function renderReleaseMeta(release) {
      if (!release) return '';
      const items = [
        ['lifecycle', release.release_lifecycle || release.release_status],
        ['quality', release.release_quality],
        ['quality reason', release.release_quality_reason || '-'],
        ['flags', Array.isArray(release.validation_flags) && release.validation_flags.length ? release.validation_flags.join(', ') : '-'],
        ['spec fallback', release.spec_info_fallback_count ?? '-'],
        ['package', shortHash(release.package_hash)],
        ['asset', shortHash(release.asset_manifest_hash)],
        ['baseline', shortHash(release.baseline_state_manifest_hash)],
        ['job', release.generation_job_id || '-'],
        ['manifest', release.manifest_url || '-'],
        ['package url', release.package_url || '-'],
      ];
      return `<div class="meta-grid">${items.map(([label, value]) =>
        `<div class="meta-line"><span>${escapeHtml(label)}</span> ${escapeHtml(value)}</div>`
      ).join('')}</div>`;
    }

    function renderReadiness(readiness) {
      const element = byId('readinessStatus');
      if (!element) return;
      if (!readiness) {
        element.hidden = true;
        element.dataset.classification = '';
        element.dataset.productionReady = 'false';
        return;
      }
      const classification = readiness.classification || 'not_production_ready';
      const ready = readiness.production_ready === true;
      const problems = Array.isArray(readiness.problems) && readiness.problems.length
        ? readiness.problems.slice(0, 3).join('; ')
        : '';
      const warnings = Array.isArray(readiness.warnings) && readiness.warnings.length
        ? readiness.warnings.slice(0, 3).join('; ')
        : '';
      const detail = problems || warnings || readiness.recommended_action || '';
      element.hidden = false;
      element.className = `readiness-status ${classification}`;
      element.dataset.classification = classification;
      element.dataset.productionReady = String(ready);
      element.dataset.componentIndexesReady = String(readiness.component_indexes_ready === true);
      element.dataset.meshAssetsReady = String(readiness.mesh_assets_ready === true);
      element.innerHTML = [
        `<span><strong>readiness</strong> ${escapeHtml(classification)}</span>`,
        `<span class="${ready ? 'ready' : 'not-ready'}">${ready ? 'production ready' : 'not production ready'}</span>`,
        `<span>${escapeHtml(readiness.recommended_action || '')}</span>`,
        detail ? `<span class="detail">${escapeHtml(detail)}</span>` : '',
      ].filter(Boolean).join('');
    }

    function selectedRelease(id) {
      const value = byId(id).value;
      return state.releases.find(item => item.release_id === value) || null;
    }

    function fillSelect(select, selected) {
      select.innerHTML = '';
      for (const release of state.releases) {
        const option = document.createElement('option');
        option.value = release.release_id;
        option.textContent = releaseLabel(release);
        select.appendChild(option);
      }
      if (selected && state.releases.some(item => item.release_id === selected)) {
        select.value = selected;
      }
    }

    function renderPane(prefix, release) {
      byId(`${prefix}Title`).textContent = release ? releaseLabel(release) : prefix;
      renderQuality(prefix, release);
      byId(`${prefix}Meta`).innerHTML = renderReleaseMeta(release);
      const releaseViewerUrl = release ? withViewerLimit(release.release_viewer_url || release.viewer_url) : '#';
      byId(`${prefix}Open`).href = releaseViewerUrl;
      byId(`${prefix}PlantViewer`).href = release ? release.viewer_url : '#';
      byId(`${prefix}Frame`).src = release ? releaseViewerUrl : 'about:blank';
    }

    function withViewerLimit(url) {
      const parsed = new URL(url, location.origin);
      parsed.searchParams.set('limit', viewerLimit);
      return `${parsed.pathname}${parsed.search}`;
    }

    function resetCameraSyncState() {
      state.cameraSync.lastFrom = '';
      state.cameraSync.lastTo = '';
      state.cameraSync.lastSource = '';
      state.cameraSync.ticks = 0;
      document.body.dataset.cameraSyncFromSignature = '';
      document.body.dataset.cameraSyncToSignature = '';
      document.body.dataset.cameraSyncLastSource = '';
    }

    function updateCameraSyncStatus(message, isError = false) {
      const status = byId('cameraSyncStatus');
      state.cameraSync.lastMessage = message;
      status.textContent = message;
      status.className = `sync-state ${isError ? 'error' : (state.cameraSync.enabled ? 'active' : '')}`;
      document.body.dataset.cameraSyncEnabled = String(Boolean(state.cameraSync.enabled));
      document.body.dataset.cameraSyncStatus = message;
    }

    function cameraVec3(value) {
      const values = Array.from(value || []).slice(0, 3).map(Number);
      if (values.length !== 3 || values.some(item => !Number.isFinite(item))) return null;
      return values;
    }

    function roundedCameraVec3(value) {
      const values = cameraVec3(value);
      if (!values) return null;
      return values.map(item => Math.round(item * 1000) / 1000);
    }

    function cameraSignature(snapshot) {
      if (!snapshot) return '';
      const eye = roundedCameraVec3(snapshot.eye);
      const look = roundedCameraVec3(snapshot.look);
      const up = roundedCameraVec3(snapshot.up);
      if (!eye || !look || !up) return '';
      return [eye, look, up].map(values => values.join(',')).join('|');
    }

    function getPaneCamera(prefix) {
      const frame = byId(`${prefix}Frame`);
      const api = frame.contentWindow && frame.contentWindow.__MODEL_VERSION_GET_CAMERA;
      return typeof api === 'function' ? api() : null;
    }

    function getPaneCameraSignature(prefix, snapshot) {
      const frame = byId(`${prefix}Frame`);
      const api = frame.contentWindow && frame.contentWindow.__MODEL_VERSION_GET_CAMERA_SIGNATURE;
      const signature = typeof api === 'function' ? api() : '';
      return signature || cameraSignature(snapshot);
    }

    function setPaneCamera(prefix, snapshot, source) {
      const frame = byId(`${prefix}Frame`);
      const api = frame.contentWindow && frame.contentWindow.__MODEL_VERSION_SET_CAMERA;
      if (typeof api !== 'function') return false;
      const result = api(snapshot, { source });
      return Boolean(result && result.applied);
    }

    function setCameraSyncEnabled(enabled) {
      state.cameraSync.enabled = Boolean(enabled);
      resetCameraSyncState();
      updateCameraSyncStatus(enabled ? 'waiting' : 'off');
      window.__MODEL_VERSION_CAMERA_SYNC_STATE = state.cameraSync;
    }

    function cameraSyncTick() {
      if (!state.cameraSync.enabled) return;
      const fromSnapshot = getPaneCamera('from');
      const toSnapshot = getPaneCamera('to');
      const fromSignature = getPaneCameraSignature('from', fromSnapshot);
      const toSignature = getPaneCameraSignature('to', toSnapshot);
      if (!fromSignature || !toSignature || !fromSnapshot || !toSnapshot) {
        updateCameraSyncStatus('waiting');
        return;
      }

      document.body.dataset.cameraSyncFromSignature = fromSignature;
      document.body.dataset.cameraSyncToSignature = toSignature;
      state.cameraSync.ticks += 1;

      if (!state.cameraSync.lastFrom || !state.cameraSync.lastTo) {
        state.cameraSync.lastFrom = fromSignature;
        state.cameraSync.lastTo = toSignature;
        updateCameraSyncStatus('active');
        return;
      }

      const fromChanged = fromSignature !== state.cameraSync.lastFrom;
      const toChanged = toSignature !== state.cameraSync.lastTo;
      if (!fromChanged && !toChanged) return;

      let source = null;
      let target = null;
      let snapshot = null;
      let signature = null;
      if (fromChanged && !toChanged) {
        source = 'from';
        target = 'to';
        snapshot = fromSnapshot;
        signature = fromSignature;
      } else if (toChanged && !fromChanged) {
        source = 'to';
        target = 'from';
        snapshot = toSnapshot;
        signature = toSignature;
      } else if (state.cameraSync.lastSource === 'to') {
        source = 'to';
        target = 'from';
        snapshot = toSnapshot;
        signature = toSignature;
      } else {
        source = 'from';
        target = 'to';
        snapshot = fromSnapshot;
        signature = fromSignature;
      }

      if (setPaneCamera(target, snapshot, `compare-${source}`)) {
        state.cameraSync.lastSource = source;
        state.cameraSync.lastFrom = signature;
        state.cameraSync.lastTo = signature;
        document.body.dataset.cameraSyncLastSource = source;
        updateCameraSyncStatus(`${source} -> ${target}`);
      } else {
        updateCameraSyncStatus('blocked', true);
      }
    }

    function renderSummary(summary) {
      const items = [
        ['Added', summary?.added ?? 0],
        ['Deleted', summary?.deleted ?? 0],
        ['Changed', summary?.changed ?? 0],
        ['Unchanged', summary?.unchanged ?? 0],
        ['Emitted', summary?.emitted ?? 0],
      ];
      byId('summary').innerHTML = items.map(([label, value]) =>
        `<div class="metric"><span>${label}</span><strong>${value}</strong></div>`
      ).join('');
    }

    function renderRows(rows) {
      state.diffRows = rows || [];
      state.selectedRowIndex = null;
      byId('selectionStatus').hidden = true;
      byId('rows').innerHTML = state.diffRows.map((row, index) => `
        <tr class="selectable" tabindex="0" data-index="${index}" data-component-key="${escapeHtml(row.component_key)}">
          <td class="${escapeHtml(row.change_type)}">${escapeHtml(row.change_type)}</td>
          <td>${escapeHtml(row.refno_str || row.component_key)}</td>
          <td>${escapeHtml(row.noun)}</td>
          <td>${escapeHtml(shortHash(row.old_component_hash))}</td>
          <td>${escapeHtml(shortHash(row.new_component_hash))}</td>
        </tr>
      `).join('');
    }

    function selectRowElement(rowElement) {
      for (const element of byId('rows').querySelectorAll('tr.selected')) {
        element.classList.remove('selected');
      }
      if (rowElement) rowElement.classList.add('selected');
    }

    function expectedPresenceForPane(row, prefix) {
      if (!row) return null;
      if (row.change_type === 'added') return prefix === 'to';
      if (row.change_type === 'deleted') return prefix === 'from';
      if (row.change_type === 'changed') return true;
      return null;
    }

    function selectionReasonText(result) {
      const reason = result?.reason || '';
      if (reason === 'component_absent_expected') return 'absent in this release';
      if (reason === 'component_absent_unexpected') return 'not found in this release';
      if (reason === 'component_has_no_loaded_geometry') return 'no renderable geometry';
      if (reason === 'viewer_not_ready') return 'viewer not ready';
      if (reason === 'selection_failed') return 'selection failed';
      if (reason === 'missing_component_key') return 'missing component key';
      if (reason === 'component_not_loaded') return 'not loaded';
      return reason || 'not loaded';
    }

    function assetLineageText(result) {
      const assets = Array.isArray(result?.asset_lineage) ? result.asset_lineage : [];
      if (!assets.length) return 'assets 0';
      const readable = assets.filter(asset => asset.glb_readable === true).length;
      const hashes = assets.slice(0, 3).map(asset => shortHash(asset.geo_hash)).join(', ');
      const more = assets.length > 3 ? ` +${assets.length - 3}` : '';
      return `assets ${assets.length}, readable ${readable}, geo ${hashes}${more}`;
    }

    function assetLineageDataset(result) {
      const assets = Array.isArray(result?.asset_lineage) ? result.asset_lineage : [];
      return {
        count: String(assets.length),
        readable: String(assets.filter(asset => asset.glb_readable === true).length),
        hashes: assets.map(asset => asset.geo_hash || '').filter(Boolean).join(','),
        urls: assets.map(asset => asset.mesh_url || '').filter(Boolean).join('|'),
        sha256: assets.map(asset => asset.sha256 || '').filter(Boolean).join(','),
      };
    }

    async function callPaneSelection(prefix, row) {
      const componentKey = row.component_key;
      const expectedPresence = expectedPresenceForPane(row, prefix);
      const frame = byId(`${prefix}Frame`);
      try {
        const api = frame.contentWindow && frame.contentWindow.__MODEL_VERSION_SELECT_COMPONENT;
        if (typeof api !== 'function') {
          return { found: false, component_key: componentKey, model_count: 0, expected_presence: expectedPresence, reason: 'viewer_not_ready' };
        }
        return await api(componentKey, {
          focus: true,
          loadIfMissing: true,
          expectedPresence,
          changeType: row.change_type || '',
        });
      } catch (error) {
        return { found: false, component_key: componentKey, model_count: 0, expected_presence: expectedPresence, reason: error.message || 'selection_failed' };
      }
    }

    function renderSelectionStatus(row, fromResult, toResult) {
      const element = byId('selectionStatus');
      const paneText = (label, result) => {
        const found = Boolean(result && result.found);
        const cls = found ? 'found' : 'missing';
        const detail = found ? `${result.model_count || 0} geometries` : selectionReasonText(result);
        return `<span><strong>${escapeHtml(label)}</strong> <span class="${cls}">${found ? 'found' : 'missing'}</span> ${escapeHtml(detail)}</span>`;
      };
      element.hidden = false;
      element.dataset.componentKey = row.component_key || '';
      element.dataset.changeType = row.change_type || '';
      element.dataset.fromFound = String(Boolean(fromResult && fromResult.found));
      element.dataset.toFound = String(Boolean(toResult && toResult.found));
      element.dataset.fromReason = fromResult?.reason || '';
      element.dataset.toReason = toResult?.reason || '';
      element.dataset.fromExpectedPresence = String(fromResult?.expected_presence ?? '');
      element.dataset.toExpectedPresence = String(toResult?.expected_presence ?? '');
      const fromAssets = assetLineageDataset(fromResult);
      const toAssets = assetLineageDataset(toResult);
      element.dataset.fromAssetCount = fromAssets.count;
      element.dataset.toAssetCount = toAssets.count;
      element.dataset.fromReadableAssetCount = fromAssets.readable;
      element.dataset.toReadableAssetCount = toAssets.readable;
      element.dataset.fromAssetHashes = fromAssets.hashes;
      element.dataset.toAssetHashes = toAssets.hashes;
      element.dataset.fromAssetUrls = fromAssets.urls;
      element.dataset.toAssetUrls = toAssets.urls;
      element.dataset.fromAssetSha256 = fromAssets.sha256;
      element.dataset.toAssetSha256 = toAssets.sha256;
      element.innerHTML = [
        `<span><strong>${escapeHtml(row.change_type)}</strong> ${escapeHtml(row.refno_str || row.component_key)}</span>`,
        paneText('from', fromResult),
        paneText('to', toResult),
        `<span class="asset-lineage"><strong>from assets</strong> ${escapeHtml(assetLineageText(fromResult))}</span>`,
        `<span class="asset-lineage"><strong>to assets</strong> ${escapeHtml(assetLineageText(toResult))}</span>`,
      ].join('');
    }

    async function selectDiffRow(index, rowElement) {
      const row = state.diffRows[index];
      if (!row || !row.component_key) return;
      state.selectedRowIndex = index;
      selectRowElement(rowElement);
      const status = byId('selectionStatus');
      status.hidden = false;
      status.dataset.componentKey = row.component_key || '';
      status.dataset.changeType = row.change_type || '';
      status.dataset.fromFound = 'pending';
      status.dataset.toFound = 'pending';
      status.innerHTML = `<span><strong>${escapeHtml(row.change_type)}</strong> ${escapeHtml(row.refno_str || row.component_key)}</span><span>loading selection...</span>`;
      const [fromResult, toResult] = await Promise.all([
        callPaneSelection('from', row),
        callPaneSelection('to', row),
      ]);
      renderSelectionStatus(row, fromResult, toResult);
      window.__MODEL_VERSION_SELECTED_DIFF_ROW = {
        index,
        row,
        from: fromResult,
        to: toResult,
      };
    }

    async function loadReleases() {
      const response = await fetch('/api/model-version/releases');
      const body = await response.json();
      if (!response.ok || !body.success) throw new Error(body.message || 'release list failed');
      state.releases = body.data.releases || [];
      fillSelect(byId('fromRelease'), queryReleaseId('from_release_id', 'from'));
      fillSelect(byId('toRelease'), queryReleaseId('to_release_id', 'to'));
      if (!byId('toRelease').value && state.releases.length > 1) {
        byId('toRelease').selectedIndex = 1;
      }
      renderPane('from', selectedRelease('fromRelease'));
      renderPane('to', selectedRelease('toRelease'));
      renderSummary({});
    }

    async function compare() {
      const fromRelease = selectedRelease('fromRelease');
      const toRelease = selectedRelease('toRelease');
      renderPane('from', fromRelease);
      renderPane('to', toRelease);
      resetCameraSyncState();
      if (state.cameraSync.enabled) updateCameraSyncStatus('waiting');
      if (!fromRelease || !toRelease) return;
      byId('error').textContent = '';
      renderReadiness(null);
      const params = new URLSearchParams({
        from_release_id: fromRelease.release_id,
        to_release_id: toRelease.release_id,
        limit: diffLimit,
      });
      const changeType = byId('changeType').value;
      if (changeType) params.set('change_type', changeType);
      const readinessParams = new URLSearchParams({
        from_release_id: fromRelease.release_id,
        to_release_id: toRelease.release_id,
      });
      const readinessResponse = await fetch(`/api/model-version/compare-readiness?${readinessParams}`);
      const readinessBody = await readinessResponse.json();
      if (!readinessResponse.ok || !readinessBody.success) throw new Error(readinessBody.message || 'compare readiness failed');
      state.readiness = readinessBody.data.readiness;
      renderReadiness(state.readiness);
      const response = await fetch(`/api/model-version/diff?${params}`);
      const body = await response.json();
      if (!response.ok || !body.success) throw new Error(body.message || 'diff failed');
      renderSummary(body.data.diff.summary);
      renderRows(body.data.diff.rows);
    }

    byId('compareButton').addEventListener('click', () => compare().catch(error => {
      byId('error').textContent = error.message;
    }));
    byId('cameraSync').addEventListener('change', event => {
      setCameraSyncEnabled(event.target.checked);
    });
    byId('rows').addEventListener('click', (event) => {
      const rowElement = event.target.closest('tr[data-index]');
      if (!rowElement) return;
      selectDiffRow(Number(rowElement.dataset.index), rowElement).catch(error => {
        byId('error').textContent = error.message;
      });
    });
    byId('rows').addEventListener('keydown', (event) => {
      if (event.key !== 'Enter' && event.key !== ' ') return;
      const rowElement = event.target.closest('tr[data-index]');
      if (!rowElement) return;
      event.preventDefault();
      selectDiffRow(Number(rowElement.dataset.index), rowElement).catch(error => {
        byId('error').textContent = error.message;
      });
    });
    byId('fromRelease').addEventListener('change', () => renderPane('from', selectedRelease('fromRelease')));
    byId('toRelease').addEventListener('change', () => renderPane('to', selectedRelease('toRelease')));
    window.setInterval(cameraSyncTick, 180);

    loadReleases()
      .then(compare)
      .catch(error => { byId('error').textContent = error.message; });
  </script>
</body>
</html>"#;
